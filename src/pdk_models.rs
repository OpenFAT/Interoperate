//! `schemify-interop pdk-models` (feature `cli`): a local kit's model files converted from one
//! simulator dialect to another through SpiceRack's IR, which is also the store form
//! (`--to rack`: each file's `CircuitIR` as JSON). PDK-Manager keeps a local install as
//! `rack` and writes the dialect a simulator wants from it on demand.
//!
//! ```text
//! pdk-models --root <dir> --entry <relpath> --from <d> --to <d> -o <outdir>
//!   d: spice (ngspice) | hspice | spectre | vacask | xyce | rack
//! ```
//!
//! Every file the entry reaches (includes, `.lib` calls, and those inside library sections)
//! is converted on its own and written at the same relative path under `<outdir>`, so the
//! references between files still resolve. Prints `converted <n> file(s)`, then one
//! `lost <relpath> <n>` line per file whose cards SpiceRack does not read (never the cards)
//! and one `missing <relpath>` per referenced file the kit lacks, then `end`. Exit 0:
//! lossless; 1: written, with losses or missing files; 2: nothing written.
//!
//! Remote kits never come here: they stay where they are (pdk-scan serves their interface).

use std::collections::{BTreeSet, VecDeque};
use std::path::{Path, PathBuf};

use spicerack::ir::{CircuitIR, Dialect};

use crate::spice::{self, MAX_FILES, MAX_INCLUDE_DEPTH};

const USAGE: &str = "usage: schemify-interop pdk-models --root <dir> --entry <relpath> \
     --from <d> --to <d> -o <outdir>   (d: spice | hspice | spectre | vacask | xyce | rack)";

/// A model file form: a SpiceRack dialect, or `None` for the IR itself (`rack`).
fn form(name: &str) -> Result<Option<Dialect>, String> {
    Ok(Some(match name {
        "spice" | "ngspice" => Dialect::Ngspice,
        "hspice" => Dialect::Hspice,
        "spectre" => Dialect::Spectre,
        "vacask" => Dialect::Vacask,
        "xyce" => Dialect::Xyce,
        "rack" => return Ok(None),
        d => return Err(format!("unknown dialect {d}\n{USAGE}")),
    }))
}

/// One file read into IR, and how many of its lines did not make it.
fn read(text: &str, from: Option<Dialect>) -> Result<(CircuitIR, usize), String> {
    match from {
        None => serde_json::from_str(text)
            .map(|ir| (ir, 0))
            .map_err(|e| e.to_string()),
        // Only a top-level netlist has a title line: an included file gets an empty one.
        Some(d) => spicerack::import::parse(&format!("\n{text}"), "pdk", d)
            .map(|(ir, r)| (ir, r.unparsed.len())),
    }
}

fn write(ir: &CircuitIR, to: Option<Dialect>) -> Result<String, String> {
    match to {
        None => serde_json::to_string_pretty(ir).map_err(|e| e.to_string()),
        Some(d) => spicerack::codegen::netlist(ir, d).map_err(|e| e.to_string()),
    }
}

/// The files `ir` names: includes and `.lib` calls, at the top and in every section.
fn references(ir: &CircuitIR) -> Vec<&str> {
    let bodies = std::iter::once(&ir.top).chain(ir.sections.iter().map(|s| &s.body));
    bodies
        .flat_map(|b| {
            let libs = b.libs.iter().map(|(f, _)| f.as_str());
            b.includes.iter().map(String::as_str).chain(libs)
        })
        .map(|f| f.trim_matches(['"', '\'']))
        .collect()
}

/// Runs `pdk-models` on `args` (after the subcommand): stdout's text and the exit status.
pub fn run(args: &[String]) -> (String, u8) {
    let mut opt = std::collections::BTreeMap::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match (a.as_str(), it.next()) {
            (k @ ("--root" | "--entry" | "--from" | "--to" | "-o"), Some(v)) => {
                opt.insert(k, v.as_str());
            }
            _ => return (format!("{USAGE}\n"), 2),
        }
    }
    let [Some(root), Some(entry), Some(from), Some(to), Some(out)] =
        ["--root", "--entry", "--from", "--to", "-o"].map(|k| opt.get(k).copied())
    else {
        return (format!("{USAGE}\n"), 2);
    };
    let (from, to) = match (form(from), form(to)) {
        (Ok(f), Ok(t)) => (f, t),
        (Err(e), _) | (_, Err(e)) => return (format!("{e}\n"), 2),
    };
    let Ok(root) = std::fs::canonicalize(root).map(|r| spice::normalize(&r)) else {
        return (format!("cannot open root {root}\n"), 2);
    };
    match convert(&root, &root.join(entry), from, to, Path::new(out)) {
        Ok((n, lost, missing)) => {
            let mut s = format!("converted {n} file(s)\n");
            for (p, k) in &lost {
                s += &format!("lost {p} {k}\n");
            }
            for p in &missing {
                s += &format!("missing {p}\n");
            }
            s += "end\n";
            (s, u8::from(!lost.is_empty() || !missing.is_empty()))
        }
        Err(e) => (format!("{e}\n"), 2),
    }
}

/// Files written, files with lost lines (relative path, count), missing files.
type Converted = (usize, Vec<(String, usize)>, Vec<String>);

/// Converts every file `entry` reaches under `root` into `out`. All files are read and
/// converted before any is written, so a failure leaves `out` untouched. Returns the count
/// written, the files with lost lines (relative path, count) and the missing ones.
fn convert(
    root: &Path,
    entry: &Path,
    from: Option<Dialect>,
    to: Option<Dialect>,
    out: &Path,
) -> Result<Converted, String> {
    let rel = |p: &Path| p.strip_prefix(root).unwrap_or(p).to_path_buf();
    let mut todo = VecDeque::from([(spice::normalize(entry), 0u8)]);
    let mut seen = BTreeSet::new();
    let mut done: Vec<(PathBuf, String)> = Vec::new();
    let (mut lost, mut missing) = (Vec::new(), Vec::new());
    while let Some((p, depth)) = todo.pop_front() {
        if !seen.insert(p.clone()) {
            continue;
        }
        if seen.len() > MAX_FILES {
            return Err("too many files".into());
        }
        let shown = rel(&p).to_string_lossy().replace('\\', "/");
        if !p.starts_with(root) {
            return Err(format!("{shown}: outside the root, refused"));
        }
        let Ok(c) = std::fs::canonicalize(&p) else {
            missing.push(shown);
            continue;
        };
        // The root is canonical; a symlink may not lead out of it.
        if !c.starts_with(root) {
            return Err(format!("{shown}: outside the root, refused"));
        }
        let bytes = std::fs::read(&c).map_err(|e| format!("{shown}: {e}"))?;
        let (ir, n) = read(&spicerack::import::decode(&bytes), from)
            .map_err(|e| format!("{shown}: not read ({e})"))?;
        if n > 0 {
            lost.push((shown.clone(), n));
        }
        let dir = p.parent().unwrap_or(Path::new(""));
        for f in references(&ir) {
            if depth >= MAX_INCLUDE_DEPTH {
                return Err(format!("{shown}: include chain too deep"));
            }
            todo.push_back((spice::normalize(&dir.join(f)), depth + 1));
        }
        let text = write(&ir, to).map_err(|e| format!("{shown}: not written ({e})"))?;
        done.push((rel(&p), text));
    }
    for (p, text) in &done {
        let to = out.join(p);
        if let Some(d) = to.parent() {
            std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
        }
        std::fs::write(&to, text).map_err(|e| format!("{}: {e}", to.display()))?;
    }
    Ok((done.len(), lost, missing))
}
