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

/// The devices of a Spectre kit, read with SpiceRack (its IR is the PDK store): every
/// top-level `subckt` of the files `entry` reaches through `include` (`section=`: that
/// section of the file), and every MOS `model` not already a subckt, names in their own
/// case (Spectre is case-sensitive). Lines SpiceRack does not read are counted, never shown:
/// they may be card data. A file read with a section (`.lib f s`, `include "f" section=s`) is that section only.
fn rack_devices(
    entry: &Path,
    section: Option<&str>,
    base: Option<&Path>,
    issues: &mut Vec<String>,
    mut load: impl FnMut(&Path) -> Result<String, bool>,
) -> Vec<Device> {
    use spicerack::{import, ir::Dialect};
    let shown = |p: &Path| {
        base.and_then(|b| p.strip_prefix(b).ok())
            .unwrap_or(p)
            .to_string_lossy()
            .replace('\\', "/")
    };
    let mut todo =
        std::collections::VecDeque::from([(entry.to_path_buf(), section.map(str::to_owned), 0u8)]);
    let mut seen = BTreeSet::new();
    let mut out: Vec<Device> = Vec::new();
    // Lines SpiceRack refused, and lines it kept as text, not IR, by keyword (never shown).
    let (mut unread, mut verbatim) = (BTreeMap::new(), BTreeMap::new());
    let mut ahdl = 0usize;
    let mut parsed = std::collections::BTreeMap::new();
    while let Some((p, sec, depth)) = todo.pop_front() {
        if !seen.insert((p.clone(), sec.clone())) {
            continue;
        }
        if seen.len() > spice::MAX_FILES {
            issues.push("too many files".into());
            break;
        }
        // A corner file is selected section by section: it is read once.
        if !parsed.contains_key(&p) {
            let text = match load(&p) {
                Ok(t) => t,
                Err(true) => {
                    issues.push(format!("include outside the root refused: {}", shown(&p)));
                    continue;
                }
                Err(false) => {
                    issues.push(format!("{}: include not found", shown(&p)));
                    continue;
                }
            };
            // Only the top-level netlist has a title line (Spectre UG §1.2): an included file
            // gets an empty one.
            let (ir, report) = match import::parse(&format!("\n{text}"), "pdk", Dialect::Spectre) {
                Ok(x) => x,
                Err(e) => {
                    issues.push(format!("{}: not read ({e})", shown(&p)));
                    continue;
                }
            };
            tally(&mut unread, &report.unparsed);
            let bodies = std::iter::once(&ir.top).chain(&ir.subcircuit_defs).chain(
                ir.sections
                    .iter()
                    .flat_map(|x| std::iter::once(&x.body).chain(&x.subcircuit_defs)),
            );
            for b in bodies {
                tally(&mut verbatim, &b.raw_spice);
                ahdl += b.osdi_loads.len();
            }
            parsed.insert(p.clone(), ir);
        }
        let ir = &parsed[&p];
        let (body, defs) = match &sec {
            None => (&ir.top, &ir.subcircuit_defs),
            Some(s) => match ir
                .sections
                .iter()
                .find(|x| x.body.name.eq_ignore_ascii_case(s))
            {
                Some(x) => (&x.body, &x.subcircuit_defs),
                None => {
                    issues.push(format!("{}: no section {s}", shown(&p)));
                    continue;
                }
            },
        };
        for sc in defs {
            let d = Device {
                target: sc.name.clone(),
                kind: Kind::Subckt,
                ports: sc.ports.iter().map(|q| q.name.clone()).collect(),
                params: sc.parameters.iter().map(|q| q.name.clone()).collect(),
            };
            add(&mut out, issues, d);
        }
        for m in &body.models {
            if mos_kind(&m.kind.to_ascii_lowercase()) && !out.iter().any(|d| d.target == m.name) {
                let d = Device {
                    target: m.name.clone(),
                    kind: Kind::Mos,
                    ports: ["d", "g", "s", "b"].map(String::from).to_vec(),
                    params: MOS_PARAMS.map(String::from).to_vec(),
                };
                add(&mut out, issues, d);
            }
        }
        let refs = body.includes.iter().map(|f| (f, None));
        let refs = refs.chain(body.libs.iter().map(|(f, s)| (f, Some(s.clone()))));
        let dir = p.parent().unwrap_or(Path::new(""));
        for (f, s) in refs {
            if depth >= spice::MAX_INCLUDE_DEPTH {
                issues.push(format!("{}: include chain too deep", shown(&p)));
                break;
            }
            todo.push_back((
                spice::normalize(&dir.join(f.trim_matches('"'))),
                s,
                depth + 1,
            ));
        }
    }
    for (what, kinds) in [
        ("SpiceRack does not read", &unread),
        ("kept as text, not read as devices", &verbatim),
    ] {
        let n: usize = kinds.values().sum();
        if n > 0 {
            let by: Vec<String> = kinds.iter().map(|(k, c)| format!("{c} {k}")).collect();
            issues.push(format!("{n} line(s) {what} (not shown): {}", by.join(", ")));
        }
    }
    if ahdl > 0 {
        issues.push(format!(
            "{ahdl} ahdl_include(s): Verilog-A devices are not scanned"
        ));
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

/// Counts `lines` by their first word when it is a [`KEYWORDS`] entry, else as `other`.
fn tally(into: &mut BTreeMap<&'static str, usize>, lines: &[String]) {
    for l in lines {
        let w = l
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        let k = KEYWORDS
            .iter()
            .find(|k| **k == w)
            .copied()
            .unwrap_or("other");
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
