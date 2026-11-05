//! `schemify-interop pdk-scan` (feature `cli`): a PDK's device interface as Schemify cells,
//! for PDK-Manager's plugin-served library (its `docs/remote-pdks.md` §4.3, §4.5). It runs
//! next to the kit (`ssh host schemify-interop pdk-scan …`) or locally on frames the kit
//! host's [`REMOTE_SCRIPT`] printed (`--stdin`), and never prints card data: no defaults, bins
//! or model parameter values.
//!
//! ```text
//! pdk-scan --protocol
//! pdk-scan --remote-script
//! pdk-scan --root <dir> --entry <relpath> --dialect spice|hspice|spectre [--section <s>] [--lib <name>]
//! pdk-scan --stdin [--root <dir>] --entry <path> --dialect spice|hspice|spectre [--section <s>] [--lib <name>]
//! ```
//!
//! Output, protocol 1 (PDK-Manager parses it; keep it exact), files sorted by path:
//!
//! ```text
//! pdkscan 1
//! version <16 hex>            FNV-1a 64 over the files: path, NUL, bytes, NUL each
//! file <relpath> <byte-length>
//! <the cell's bytes>
//! issue <one line>            never model data
//! need <path>                 --stdin only: an include the frames lack
//! end
//! ```
//!
//! On failure `pdkscan 1`, `error "<text>"`, `end`, exit 2. `--stdin` reads `sfyframe 1`,
//! then per file `f <byte-length> <path>` and its bytes (`f 0 ! <path>`: unreadable), then
//! `end`. The filesystem mode reads only under `--root` (symlinks resolved); with `--stdin`,
//! `--root` is optional, joins `--entry` and confines includes lexically.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use schemify_format::{Document, FileKind, RecordKind};

use crate::design::push_attr;
use crate::spice::{self, ScanIssueKind, ScanOptions, TOP_LEVEL, base_name};
use crate::virtuoso::read::{box_symbol, pins_template};
use crate::virtuoso::write::optional_param;

/// The POSIX `sh` + `awk` script the fallback runs on the kit host
/// (`ssh … -- host sh -s -- '<abs path>'…`): one frame per path of its structural lines only.
pub const REMOTE_SCRIPT: &str = include_str!("pdk_frames.sh");

/// Instance parameters a MOS `.model` cell offers (BSIM's geometry and multipliers).
const MOS_PARAMS: [&str; 4] = ["w", "l", "nf", "m"];

const USAGE: &str = "usage: schemify-interop pdk-scan --protocol | --remote-script | \
     [--stdin] --root <dir> --entry <path> --dialect spice|hspice|spectre [--section <s>] [--lib <name>]";

/// What a device is instantiated as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// A `.subckt`: `X` lines, the ports in order.
    Subckt,
    /// A MOS `.model` (bins collapsed): `M` lines, `d g s b`.
    Mos,
    /// A Verilog-A module (`ahdl_include`): `N` lines (ngspice OSDI), the ports in order.
    Va,
}

/// One served device: what its cell needs, and nothing from the model cards.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Device {
    /// Netlist target (subckt or model base name), lowercased; also the cell name.
    target: String,
    kind: Kind,
    /// Terminals in netlist order.
    ports: Vec<String>,
    /// Instance parameter names.
    params: Vec<String>,
}

/// Runs `pdk-scan` on `args` (after the subcommand) with `stdin` the frames (read only with
/// `--stdin`). Returns stdout's bytes and the exit status (0, or 2 with an `error` record).
pub fn run(args: &[String], stdin: &[u8]) -> (Vec<u8>, u8) {
    match args {
        [a] if a == "--protocol" => return (b"pdkscan 1\n".to_vec(), 0),
        [a] if a == "--remote-script" => return (REMOTE_SCRIPT.as_bytes().to_vec(), 0),
        _ => {}
    }
    let mut opt: BTreeMap<&str, &str> = BTreeMap::new();
    let mut from_stdin = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--stdin" => from_stdin = true,
            k @ ("--root" | "--entry" | "--dialect" | "--section" | "--lib") => {
                let Some(v) = it.next() else {
                    return fail(USAGE);
                };
                opt.insert(k, v);
            }
            _ => return fail(USAGE),
        }
    }
    let (Some(entry), Some(dialect)) = (opt.get("--entry"), opt.get("--dialect")) else {
        return fail(USAGE);
    };
    let spectre = match *dialect {
        "spice" | "hspice" => false,
        "spectre" => true,
        d => return fail(&format!("unknown dialect {d}: spice, hspice or spectre")),
    };
    let section = opt.get("--section").copied();
    let mut issues = Vec::new();
    let mut needs: Vec<String> = Vec::new();
    let (root, devs) = if from_stdin {
        let Some(frames) = frames(stdin) else {
            return fail("stdin is not sfyframe 1 input");
        };
        let root = opt.get("--root").map(|r| spice::normalize(Path::new(r)));
        let entry = spice::normalize(&root.as_deref().unwrap_or(Path::new("")).join(entry));
        let devs = devices(
            &entry,
            spectre,
            section,
            root.as_deref(),
            &mut issues,
            |p: &Path| {
                if root.as_ref().is_some_and(|r| !p.starts_with(r)) {
                    return Err(true);
                }
                match frames.get(p) {
                    Some(text) => text.clone().ok_or(false),
                    None => {
                        let p = p.to_string_lossy().into_owned();
                        if !needs.contains(&p) {
                            needs.push(p);
                        }
                        Err(false)
                    }
                }
            },
        );
        let base = root.or_else(|| entry.parent().map(Path::to_path_buf));
        (base, devs)
    } else {
        let Some(root) = opt.get("--root") else {
            return fail(USAGE);
        };
        let Ok(root) = std::fs::canonicalize(root).map(|r| spice::normalize(&r)) else {
            return fail(&format!("cannot open root {root}"));
        };
        let path = root.join(entry);
        if !std::fs::canonicalize(&path).is_ok_and(|p| p.starts_with(&root) && p.is_file()) {
            return fail(&format!("cannot read entry {entry} under the root"));
        }
        let devs = devices(
            &path,
            spectre,
            section,
            Some(&root),
            &mut issues,
            |p: &Path| {
                let c = std::fs::canonicalize(p).map_err(|_| false)?;
                if !c.starts_with(&root) {
                    return Err(true);
                }
                let bytes = std::fs::read(&c).map_err(|_| false)?;
                Ok(String::from_utf8_lossy(&bytes).into_owned())
            },
        );
        (Some(root), devs)
    };
    let lib = opt.get("--lib").map_or_else(
        || {
            root.as_deref()
                .and_then(Path::file_name)
                .map_or("pdk".into(), |n| n.to_string_lossy().into_owned())
        },
        |l| (*l).to_owned(),
    );
    let mut files: Vec<(String, String)> = devs
        .iter()
        .map(|d| (format!("{}.shprim", d.target), cell(&lib, d)))
        .collect();
    files.sort();
    (render(&files, &issues, &needs), 0)
}

/// The devices `entry` defines (`section` read as `.lib entry section`), read through `load`
/// (`Err(true)`: refused, outside the root; `Err(false)`: not there). Paths in `issues` are
/// relative to `base`.
// ponytail: the one reader seam; swap `spice::scan` for SpiceRack's IR here, nothing else changes.
fn devices(
    entry: &Path,
    spectre: bool,
    section: Option<&str>,
    base: Option<&Path>,
    issues: &mut Vec<String>,
    mut load: impl FnMut(&Path) -> Result<String, bool>,
) -> Vec<Device> {
    if spectre {
        return rack_devices(entry, section, base, issues, load);
    }
    let shown = |p: &Path| {
        base.and_then(|b| p.strip_prefix(b).ok())
            .unwrap_or(p)
            .to_string_lossy()
            .replace('\\', "/")
    };
    let mut refused = Vec::new();
    let opts = ScanOptions {
        section,
        search_dirs: &[],
    };
    let s = spice::scan(entry, &opts, |p| {
        load(p)
            .map_err(|outside| {
                if outside {
                    refused.push(shown(p));
                }
            })
            .ok()
    });
    for i in &s.issues {
        let what = match i.kind {
            ScanIssueKind::Unresolved => "include not found",
            ScanIssueKind::Cycle => "include cycle cut",
            ScanIssueKind::TooDeep => "include chain too deep",
            ScanIssueKind::TooManyFiles => "too many files",
            ScanIssueKind::SectionNotFound => "section not found",
            ScanIssueKind::UnbalancedEnds => ".ends without .subckt",
            ScanIssueKind::MalformedModel => ".model without name and type",
            ScanIssueKind::NonNumericBin => continue, // its detail is card data
        };
        let at = s
            .files
            .get(i.file as usize)
            .map_or(String::new(), |f| format!("{}:{}: ", shown(f), i.line));
        let detail = s.str(i.detail);
        let sep = if detail.is_empty() { "" } else { ": " };
        issues.push(format!("{at}{what}{sep}{detail}"));
    }
    issues.extend(
        refused
            .iter()
            .map(|p| format!("include outside the root refused: {p}")),
    );
    let protected = s.line_counts[spice::LineKind::Protected as usize];
    if protected > 0 {
        issues.push(format!("{protected} protected (.prot) lines not read"));
    }
    let mut out: Vec<Device> = Vec::new();
    for r in (0..s.subckts.name.len()).filter(|&r| s.subckts.subckt[r] == TOP_LEVEL) {
        let d = Device {
            target: s.str(s.subckts.name[r]).to_owned(),
            kind: Kind::Subckt,
            ports: s.ports(r).map(str::to_owned).collect(),
            params: s.subckt_params(r).map(|p| p.0.to_owned()).collect(),
        };
        add(&mut out, issues, d);
    }
    let mut mos = BTreeSet::new();
    let mut skipped: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for m in (0..s.models.name.len()).filter(|&m| s.models.subckt[m] == TOP_LEVEL) {
        let (name, kind) = (base_name(s.str(s.models.name[m])), s.str(s.models.kind[m]));
        if !matches!(kind, "nmos" | "pmos") {
            skipped.entry(kind).or_default().insert(name);
        } else if mos.insert(name) && !out.iter().any(|d| d.target == name) {
            let d = Device {
                target: name.to_owned(),
                kind: Kind::Mos,
                ports: ["d", "g", "s", "b"].map(String::from).to_vec(),
                params: MOS_PARAMS.map(String::from).to_vec(),
            };
            add(&mut out, issues, d);
        }
    }
    if !skipped.is_empty() {
        let counts: Vec<String> = skipped
            .iter()
            .map(|(k, names)| format!("{} {k}", names.len()))
            .collect();
        issues.push(format!(
            "models not served (no cell kind): {}",
            counts.join(", ")
        ));
    }
    out
}

/// Whether a model of type `kind` (lowercased) is a MOS transistor: SPICE `nmos`/`pmos`, or
/// a Spectre compact-model family.
fn mos_kind(kind: &str) -> bool {
    matches!(kind, "nmos" | "pmos")
        || ["bsim", "psp", "hisim", "utsoi", "mos"]
            .iter()
            .any(|f| kind.starts_with(f))
}

/// The devices of a Spectre kit, read through SpiceRack's [`Kit`]: every subcircuit visible at
/// the top of a file the entry reaches (`include`, and `include … section=` for the section it
/// selects; `section` read as that section of the entry), every MOS model not already a
/// subcircuit, and every Verilog-A module (`ahdl_include`). Names keep their case (Spectre is
/// case-sensitive). Lines SpiceRack does not read are counted by keyword, never shown: they
/// may be card data.
///
/// [`Kit`]: spicerack::syntax::resolve::Kit
fn rack_devices(
    entry: &Path,
    section: Option<&str>,
    base: Option<&Path>,
    issues: &mut Vec<String>,
    mut load: impl FnMut(&Path) -> Result<String, bool>,
) -> Vec<Device> {
    use spicerack::syntax::Dialect;
    use spicerack::syntax::ast::Node;
    use spicerack::syntax::device::Primitive;
    use spicerack::syntax::diag::Code;
    use spicerack::syntax::resolve::{DefKind, FileId, Form, Kit, LoadError, Role, Scope};
    use spicerack::syntax::{lexicon, tokenize};
    let shown = |p: &Path| {
        base.and_then(|b| p.strip_prefix(b).ok())
            .unwrap_or(p)
            .to_string_lossy()
            .replace('\\', "/")
    };
    let mut loader = |p: &Path| {
        load(p).map_err(|refused| {
            if refused {
                LoadError::Refused
            } else {
                LoadError::NotFound
            }
        })
    };
    let kit = Kit::load(
        &[entry.to_path_buf()],
        Form::Library,
        Dialect::Spectre,
        &mut loader,
    );
    // The sections something selects: every `include … section=`, plus `section` on the entry.
    let mut selected: BTreeSet<(FileId, u32)> = BTreeSet::new();
    for (f, _) in kit.files().iter().enumerate() {
        let f = FileId(f as u32);
        for n in 0..kit.node_count(f) {
            if let Some(t) = kit.target(f, Node(n as u32)) {
                selected.extend(t.section.map(|s| (t.file, s.0)));
            }
        }
    }
    if let Some(sec) = section {
        let entry_section = kit.definitions().iter().find(|d| {
            d.file == FileId(0) && d.kind == DefKind::Section && d.name.eq_ignore_ascii_case(sec)
        });
        match entry_section {
            Some(d) => {
                selected.insert((d.file, d.node.0));
            }
            None => issues.push(format!("{}: no section {sec}", shown(entry))),
        }
    }
    // A declaration is a cell when it is visible at a file's top: not inside a subcircuit, and
    // inside a section only when the section is selected. `if` branches are looked through.
    let visible = |file: FileId, mut scope: Scope| loop {
        match scope {
            Scope::Top => return true,
            Scope::Subckt(_) => return false,
            Scope::Section(s) => return selected.contains(&(file, s.0)),
            Scope::Branch(b) => match kit.scope(file, b) {
                Some(outer) => scope = outer,
                None => return true,
            },
        }
    };
    let mut out: Vec<Device> = Vec::new();
    let mut unread = BTreeMap::new();
    // Undeclared masters by name, with how many instances name each (gaps2 P16).
    let mut unknown: BTreeMap<String, usize> = BTreeMap::new();
    for (i, file) in kit.files().iter().enumerate() {
        let id = FileId(i as u32);
        match file.role {
            Role::Unread(LoadError::Refused) => {
                issues.push(format!(
                    "include outside the root refused: {}",
                    shown(&file.path)
                ));
            }
            Role::Unread(LoadError::NotFound) => {
                issues.push(format!("{}: include not found", shown(&file.path)));
            }
            Role::VerilogA => va_modules(&file.text)
                .into_iter()
                .for_each(|d| add(&mut out, issues, d)),
            Role::Deck | Role::Library => {}
        }
        // The statement each `P` diagnostic sits on, counted by its first word's keyword.
        let lex = lexicon::of(kit.dialect()).expect("Spectre has a reader");
        let toks = tokenize(&file.text, lex);
        let mut lines = Vec::new();
        for d in &file.diags {
            match d.code {
                Code::R0001 => {
                    let at = toks.start(d.token.min(toks.len().saturating_sub(1) as u32)) as usize;
                    let name: String = file.text[at..]
                        .chars()
                        .take_while(|c| !c.is_whitespace() && !"()=".contains(*c))
                        .collect();
                    *unknown.entry(name).or_default() += 1;
                }
                Code::R0008 => {
                    issues.push(format!("{}: include chain too deep", shown(&file.path)))
                }
                Code::R0009 => issues.push("too many files".into()),
                // ponytail: the parse codes by name until SpiceRack's catalogue gives a stage class.
                Code::P0001
                | Code::P0002
                | Code::P0003
                | Code::P0004
                | Code::P0005
                | Code::P0006
                | Code::P0007
                | Code::P0008
                | Code::P0009 => {
                    let at = toks.start(d.token.min(toks.len().saturating_sub(1) as u32)) as usize;
                    let line = file.text[at..].lines().next().unwrap_or("");
                    lines.push(line.to_owned());
                }
                _ => {}
            }
        }
        tally(&mut unread, &lines);
        let defs = kit
            .definitions()
            .iter()
            .filter(|d| d.file == id && visible(id, d.scope));
        let (subckts, models): (Vec<_>, Vec<_>) =
            defs.partition(|d| matches!(d.kind, DefKind::Subckt { .. }));
        for d in subckts {
            if let DefKind::Subckt { ports, params } = &d.kind {
                let dev = Device {
                    target: d.name.clone(),
                    kind: Kind::Subckt,
                    ports: ports.clone(),
                    params: params.clone(),
                };
                add(&mut out, issues, dev);
            }
        }
        for d in models {
            let DefKind::Model { word, kind, .. } = &d.kind else {
                continue;
            };
            let mos = *kind == Some(Primitive::Mosfet) || mos_kind(&word.to_ascii_lowercase());
            // A binned model is one cell, by its base name (`nch` for `nch.1`).
            let name = d.base().to_owned();
            if mos && !out.iter().any(|x| x.target == name) {
                let dev = Device {
                    target: name,
                    kind: Kind::Mos,
                    ports: ["d", "g", "s", "b"].map(String::from).to_vec(),
                    params: MOS_PARAMS.map(String::from).to_vec(),
                };
                add(&mut out, issues, dev);
            }
        }
    }
    let n: usize = unread.values().sum();
    if n > 0 {
        let by: Vec<String> = unread.iter().map(|(k, c)| format!("{c} {k}")).collect();
        issues.push(format!(
            "{n} line(s) SpiceRack does not read (not shown): {}",
            by.join(", ")
        ));
    }
    if !unknown.is_empty() {
        let n: usize = unknown.values().sum();
        let by: Vec<String> = unknown.iter().map(|(k, c)| format!("{k} ({c})")).collect();
        issues.push(format!(
            "{n} instance(s) whose master the kit does not declare: {}",
            by.join(", ")
        ));
    }
    out
}

/// The modules a Verilog-A source defines: name, ports in header order, and the names of
/// its `parameter`s (instance-overridable; `localparam`s are not), never their defaults.
/// Comments, attributes `(* … *)`, strings and compiler directives are skipped.
// ponytail: a token scan of module headers and parameter declarations, not a Verilog-A
// parser; `include`d files are not followed.
fn va_modules(text: &str) -> Vec<Device> {
    let toks = va_tokens(text);
    let at = |i: usize| toks.get(i).map_or("", String::as_str);
    let mut out = Vec::new();
    let mut cur: Option<Device> = None;
    let mut i = 0;
    while i < toks.len() {
        match at(i) {
            "module" | "macromodule" if is_ident(at(i + 1)) => {
                let mut d = Device {
                    target: toks[i + 1].clone(),
                    kind: Kind::Va,
                    ports: Vec::new(),
                    params: Vec::new(),
                };
                i += 2;
                if at(i) == "(" {
                    // ANSI or not, a port is the last word before each `,` and the `)`.
                    let mut last = None;
                    i += 1;
                    while i < toks.len() && at(i) != ")" {
                        match at(i) {
                            "," => d.ports.extend(last.take()),
                            t if is_ident(t) => last = Some(t.to_owned()),
                            _ => {}
                        }
                        i += 1;
                    }
                    d.ports.extend(last);
                }
                cur = Some(d);
            }
            // `parameter [type] name = value [from …] {, name = value} ;`
            "parameter" => {
                let (mut depth, mut want) = (0i32, true);
                i += 1;
                while i < toks.len() && !(depth == 0 && at(i) == ";") {
                    match at(i) {
                        "(" | "[" | "{" => depth += 1,
                        ")" | "]" | "}" => depth -= 1,
                        "," if depth == 0 => want = true,
                        t if want && depth == 0 && is_ident(t) && at(i + 1) == "=" => {
                            if let Some(d) = &mut cur {
                                d.params.push(t.to_owned());
                            }
                            want = false;
                        }
                        _ => {}
                    }
                    i += 1;
                }
            }
            "endmodule" => out.extend(cur.take()),
            _ => {}
        }
        i += 1;
    }
    out
}

fn is_ident(t: &str) -> bool {
    t.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_' || c == '\\')
}

/// Verilog-A text as words and one-character punctuation, without comments, attributes,
/// strings or compiler directives (a `` ` `` line).
fn va_tokens(text: &str) -> Vec<String> {
    let skip_to = |from: usize, end: &str| {
        text.get(from..)
            .and_then(|t| t.find(end))
            .map_or(text.len(), |k| from + k + end.len())
    };
    let word = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '$';
    let (mut out, mut i) = (Vec::new(), 0);
    while let Some(c) = text[i..].chars().next() {
        let rest = &text[i..];
        if rest.starts_with("//") || c == '`' {
            i = skip_to(i, "\n");
        } else if rest.starts_with("/*") {
            i = skip_to(i + 2, "*/");
        } else if rest.starts_with("(*") && !rest.starts_with("(*)") {
            i = skip_to(i + 2, "*)");
        } else if c == '"' {
            i = skip_to(i + 1, "\"");
        } else if word(c) || c == '\\' {
            // An escaped identifier runs to white space.
            let end = rest[1..]
                .find(|d: char| {
                    if c == '\\' {
                        d.is_whitespace()
                    } else {
                        !word(d)
                    }
                })
                .map_or(rest.len(), |k| k + 1);
            out.push(rest[..end].to_owned());
            i += end;
        } else {
            if !c.is_whitespace() {
                out.push(c.to_string());
            }
            i += c.len_utf8();
        }
    }
    out
}

/// Spectre and SPICE statement keywords: the only part of an unread line a scan may show.
const KEYWORDS: [&str; 26] = [
    "statistics",
    "process",
    "mismatch",
    "vary",
    "correlate",
    "truncate",
    "simulator",
    "ahdl_include",
    "model",
    "subckt",
    "inline",
    "parameters",
    "include",
    "section",
    "library",
    "if",
    "else",
    "real",
    "global",
    "options",
    ".param",
    ".model",
    ".subckt",
    ".lib",
    ".include",
    ".option",
];

/// Spectre's built-in device masters: language keywords, so a scan may name them.
const PRIMITIVES: [&str; 20] = [
    "resistor",
    "capacitor",
    "inductor",
    "vsource",
    "isource",
    "vcvs",
    "vccs",
    "ccvs",
    "cccs",
    "bsource",
    "mutual_inductor",
    "diode",
    "bjt",
    "mosfet",
    "switch",
    "port",
    "tline",
    "pvcvs",
    "pvccs",
    "relay",
];

fn primitive(w: Option<&str>) -> Option<&'static str> {
    let w = w?.to_ascii_lowercase();
    PRIMITIVES.iter().find(|p| **p == w).copied()
}

/// Counts `lines` by their first word when it is a [`KEYWORDS`] entry, else by shape alone.
fn tally(into: &mut BTreeMap<&'static str, usize>, lines: &[String]) {
    for l in lines {
        let t = l.trim_start();
        let w = t
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        let k = KEYWORDS
            .iter()
            .find(|k| **k == w)
            .copied()
            .unwrap_or_else(|| {
                let (eq, paren) = (t.find('='), t.find('('));
                let words: Vec<&str> = t.split_whitespace().collect();
                match t.chars().next() {
                    Some('+') => "continuation",
                    Some('{' | '}') => "brace",
                    Some('.') => "dotcard",
                    _ if paren.is_some_and(|p| eq.is_none_or(|e| p < e)) => {
                        // `name (nodes) master …`: the master, when it is a Spectre primitive.
                        let after = t.find(')').map_or("", |c| &t[c + 1..]);
                        primitive(after.split_whitespace().next()).unwrap_or("subckt_instance")
                    }
                    _ if words.first().is_some_and(|w| w.contains('='))
                        || words.get(1) == Some(&"=") =>
                    {
                        "assignment"
                    }
                    // `name n1 n2 master k=v …`: the word before the first `k=v`.
                    _ if eq.is_some() => {
                        let at = words.iter().position(|w| w.contains('=')).unwrap_or(0);
                        let master = at.checked_sub(1).map(|i| words[i]);
                        primitive(master)
                            .map_or("unparenthesized_instance", |_| "unparenthesized_primitive")
                    }
                    _ => "other",
                }
            });
        *into.entry(k).or_default() += 1;
    }
}

/// Appends `d` to `out` unless its name cannot be a cell file, a port repeats, or a device of
/// that name came first (each reported).
fn add(out: &mut Vec<Device>, issues: &mut Vec<String>, d: Device) {
    let safe = d
        .target
        .bytes()
        .next()
        .is_some_and(|b| b != b'.' && b != b'-')
        && d.target
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b));
    let ports: BTreeSet<&String> = d.ports.iter().collect();
    if !safe || ports.len() != d.ports.len() {
        issues.push(format!(
            "{} not served: unusable cell name or repeated port",
            d.target
        ));
    } else if out.iter().any(|o| o.target == d.target) {
        issues.push(format!("{} defined again: the first kept", d.target));
    } else {
        out.push(d);
    }
}

/// The canonical `.shprim` of `d`: pins in order on a box symbol, every parameter an empty
/// string written only when set (`interop.md`'s string-param rule: no PDK default is ever
/// written), `format spice` and `format spectre`.
fn cell(lib: &str, d: &Device) -> String {
    let mut doc = Document::new(FileKind::Shprim);
    push_attr(
        &mut doc,
        "provenance",
        &[format!("pdk-scan {lib}/{}", d.target)],
    );
    box_symbol(
        &mut doc,
        d.ports
            .iter()
            .map(|p| (p.clone(), "inout".into()))
            .collect(),
    );
    let mut tail = String::new();
    for p in &d.params {
        doc.begin_record(RecordKind::Param);
        doc.push_field(p);
        doc.push_field("");
        push_attr(&mut doc, "kind", &["string"]);
        tail.push_str(&optional_param(p, p));
    }
    let letter = match d.kind {
        Kind::Subckt => "X",
        Kind::Mos => "M",
        Kind::Va => "N",
    };
    let (pins, target) = (pins_template(&d.ports), &d.target);
    doc.begin_record(RecordKind::Impl);
    doc.push_field("spice");
    doc.push_field("spice");
    let spice = format!("{letter}{{name}} {pins} {target}{tail}");
    push_attr(&mut doc, "format", &["spice", &spice]);
    let spectre = format!("{{name}} ({pins}) {target}{tail}");
    push_attr(&mut doc, "format", &["spectre", &spectre]);
    schemify_format::write(&doc)
}

/// Protocol 1 for `files` (sorted), `issues` and `needs`.
fn render(files: &[(String, String)], issues: &[String], needs: &[String]) -> Vec<u8> {
    assert!(files.windows(2).all(|w| w[0].0 < w[1].0));
    let mut hashed = String::new();
    for (p, b) in files {
        hashed.extend([p.as_str(), "\0", b, "\0"]);
    }
    let mut out = format!("pdkscan 1\nversion {}\n", crate::merge::digest(&hashed));
    for (p, b) in files {
        out.push_str(&format!("file {p} {}\n{b}", b.len()));
    }
    for i in issues {
        out.push_str(&format!("issue {}\n", i.replace(['\n', '\r'], " ")));
    }
    for n in needs {
        out.push_str(&format!("need {n}\n"));
    }
    out.push_str("end\n");
    out.into_bytes()
}

fn fail(text: &str) -> (Vec<u8>, u8) {
    let t = text.replace('\\', "\\\\").replace('"', "\\\"");
    (format!("pdkscan 1\nerror \"{t}\"\nend\n").into_bytes(), 2)
}

/// The files of `sfyframe 1` input by normalised path (`None`: unreadable on the host), or
/// `None` when the input is not well formed.
fn frames(input: &[u8]) -> Option<BTreeMap<PathBuf, Option<String>>> {
    let mut rest = input.strip_prefix(b"sfyframe 1\n")?;
    let mut out = BTreeMap::new();
    loop {
        let nl = rest.iter().position(|&b| b == b'\n')?;
        let line = std::str::from_utf8(&rest[..nl]).ok()?;
        rest = &rest[nl + 1..];
        if line == "end" {
            return Some(out);
        }
        let (len, path) = line.strip_prefix("f ")?.split_once(' ')?;
        let len: usize = len.parse().ok()?;
        let text = match path.strip_prefix("! ") {
            Some(p) if len == 0 => (p, None),
            _ => {
                let body = rest.get(..len)?;
                rest = &rest[len..];
                (path, Some(String::from_utf8_lossy(body).into_owned()))
            }
        };
        out.insert(spice::normalize(Path::new(text.0)), text.1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_parse_exact_lengths_and_unreadable_files() {
        let input = b"sfyframe 1\nf 4 /k/a.lib\nab\ncf 0 ! /k/b.lib\nf 3 /k/./c\nxyzend\n";
        let f = frames(input).expect("well formed");
        assert_eq!(f[Path::new("/k/a.lib")].as_deref(), Some("ab\nc"));
        assert_eq!(f[Path::new("/k/b.lib")], None);
        assert_eq!(f[Path::new("/k/c")].as_deref(), Some("xyz"));
        assert!(frames(b"sfyframe 1\nf 9 /k/a\nab\nend\n").is_none());
        assert!(frames(b"sfyframe 2\nend\n").is_none());
    }

    #[test]
    fn cells_carry_pins_params_and_both_formats() {
        let d = Device {
            target: "nch".into(),
            kind: Kind::Mos,
            ports: ["d", "g", "s", "b"].map(String::from).to_vec(),
            params: vec!["w".into()],
        };
        let c = cell("kit", &d);
        assert!(c.starts_with("schemify 1 shprim\n  provenance \"pdk-scan kit/nch\"\n"));
        assert!(c.contains("\nparam w \"\"\n  kind string\n"));
        assert!(c.contains(
            r#"format spice "M{name} {d} {g} {s} {b} nch{w != \"\" ? \" w=\" + w : \"\"}""#
        ));
        assert!(c.contains(
            r#"format spectre "{name} ({d} {g} {s} {b}) nch{w != \"\" ? \" w=\" + w : \"\"}""#
        ));
        assert!(c.contains("\npin d inout ") && c.contains("\nrect "));
    }

    #[test]
    fn render_hashes_the_files_only() {
        let a = render(&[("x.shprim".into(), "1\n".into())], &[], &[]);
        let b = render(
            &[("x.shprim".into(), "1\n".into())],
            &["i".into()],
            &["n".into()],
        );
        let c = render(&[("x.shprim".into(), "2\n".into())], &[], &[]);
        let version = |o: &[u8]| String::from_utf8_lossy(o).lines().nth(1).map(str::to_owned);
        assert_eq!(version(&a), version(&b));
        assert_ne!(version(&a), version(&c));
        assert_eq!(
            String::from_utf8_lossy(&b)
                .lines()
                .skip(2)
                .collect::<Vec<_>>(),
            ["file x.shprim 2", "1", "issue i", "need n", "end"]
        );
        assert_eq!(
            fail("a \"b\"").0,
            b"pdkscan 1\nerror \"a \\\"b\\\"\"\nend\n"
        );
    }
}
