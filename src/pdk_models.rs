//! `schemify-interop pdk-models` (feature `cli`): a local kit's model files converted from one
//! simulator dialect to another through SpiceRack's transpiler (`import::kit` reads the kit,
//! `render::render_kit` writes each file). The store is the kit's own source files and their
//! read order; PDK-Manager writes the dialect a simulator wants from them on demand.
//!
//! ```text
//! pdk-models --root <dir> --entry <relpath>... --from <d> --to <d> -o <outdir>
//!   d: spice (ngspice) | hspice | spectre | vacask | xyce
//! ```
//!
//! Every file the entries reach (`--entry` repeats: a kit with a file per corner; includes,
//! `.lib` calls and `include … section=`, at the top, in sections, subcircuits and branches)
//! is converted on its own and written at the same relative path under `<outdir>`, so the
//! references between files still resolve; Verilog-A sources (`ahdl_include`) are copied.
//! Output, files in the kit's read order:
//!
//! ```text
//! converted <n> file(s)           exit 0 or 1; on exit 2 `not written` instead
//! diag <relpath> <code> <count>   per file, per SpiceRack diagnostic code (never the cards)
//! end
//! ```
//!
//! A file the kit lacks is `diag <its relpath> R0006 1` (not repeated on its includer).
//! `diag::info(code)` in SpiceRack is a code's title and explanation. Exit 0: lossless (no
//! `diag` line); 1: written, with notes or missing files; 2: nothing written, either a file
//! the target refuses (its `W` codes among its `diag` lines) or a path outside the root
//! (`R0007`, also printed as `<relpath>: outside the root, refused`). Usage errors print the
//! usage and exit 2.
//!
//! Remote kits never come here: they stay where they are (pdk-scan serves their interface).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use spicerack::syntax::Dialect;
use spicerack::syntax::diag::Code;
use spicerack::syntax::render::{self, Diags};
use spicerack::syntax::resolve::{FileId, Form, Kit, LoadError, Role};

use crate::spice;

const USAGE: &str = "usage: schemify-interop pdk-models --root <dir> --entry <relpath>... \
     --from <d> --to <d> -o <outdir>   (d: spice | hspice | spectre | vacask | xyce)";

fn dialect(name: &str) -> Result<Dialect, String> {
    Ok(match name {
        "spice" | "ngspice" => Dialect::Ngspice,
        "hspice" => Dialect::Hspice,
        "spectre" => Dialect::Spectre,
        "vacask" => Dialect::Vacask,
        "xyce" => Dialect::Xyce,
        d => return Err(format!("unknown dialect {d}\n{USAGE}")),
    })
}

/// Runs `pdk-models` on `args` (after the subcommand): stdout's text and the exit status.
pub fn run(args: &[String]) -> (String, u8) {
    let mut opt = BTreeMap::new();
    let mut entries = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match (a.as_str(), it.next()) {
            ("--entry", Some(v)) => entries.push(v.as_str()),
            (k @ ("--root" | "--from" | "--to" | "-o"), Some(v)) => {
                opt.insert(k, v.as_str());
            }
            _ => return (format!("{USAGE}\n"), 2),
        }
    }
    let [Some(root), Some(from), Some(to), Some(out)] =
        ["--root", "--from", "--to", "-o"].map(|k| opt.get(k).copied())
    else {
        return (format!("{USAGE}\n"), 2);
    };
    if entries.is_empty() {
        return (format!("{USAGE}\n"), 2);
    }
    let (from, to) = match (dialect(from), dialect(to)) {
        (Ok(f), Ok(t)) => (f, t),
        (Err(e), _) | (_, Err(e)) => return (format!("{e}\n"), 2),
    };
    let Ok(root) = std::fs::canonicalize(root).map(|r| spice::normalize(&r)) else {
        return (format!("cannot open root {root}\n"), 2);
    };
    let entries: Vec<PathBuf> = entries.iter().map(|e| root.join(e)).collect();
    convert(&root, &entries, from, to, Path::new(out))
}

/// Converts every file the `entries` reach under `root` into `out`: the report and the exit
/// status. Every file is read and written to text before any is stored, so a refusal leaves
/// `out` untouched.
fn convert(
    root: &Path,
    entries: &[PathBuf],
    from: Dialect,
    to: Dialect,
    out: &Path,
) -> (String, u8) {
    // A lexical path under the root whose real path (symlinks resolved) is also under it.
    let mut load = |p: &Path| {
        if !p.starts_with(root) {
            return Err(LoadError::Refused);
        }
        let real = std::fs::canonicalize(p).map_err(|_| LoadError::NotFound)?;
        if !real.starts_with(root) {
            return Err(LoadError::Refused);
        }
        let bytes = std::fs::read(&real).map_err(|_| LoadError::NotFound)?;
        Ok(spicerack::import::decode(&bytes))
    };
    let kit = Kit::load(entries, Form::Library, from, &mut load);
    let rel = |p: &Path| {
        let p = p.strip_prefix(root).unwrap_or(p);
        p.to_string_lossy().replace('\\', "/")
    };
    let (mut report, mut done) = (String::new(), Vec::new());
    let (mut refused, mut outside) = (false, String::new());
    for (i, file) in kit.files().iter().enumerate() {
        let shown = rel(&file.path);
        let codes = match file.role {
            Role::Unread(LoadError::Refused) => {
                refused = true;
                outside += &format!("{shown}: outside the root, refused\n");
                vec![Code::R0007]
            }
            Role::Unread(LoadError::NotFound) => vec![Code::R0006],
            Role::VerilogA => {
                done.push((&file.path, file.text.clone()));
                Vec::new()
            }
            Role::Deck | Role::Library => {
                match render::render_kit(&kit, FileId(i as u32), &shown, to) {
                    Ok(w) => {
                        done.push((&file.path, w.text));
                        codes(&w.notes)
                    }
                    Err(d) => {
                        refused = true;
                        codes(&d)
                    }
                }
            }
        };
        let mut counts: BTreeMap<Code, usize> = BTreeMap::new();
        // An unread include is reported at its own path, not again on its includer's statement.
        let own = matches!(file.role, Role::Unread(_));
        for c in codes {
            if own || !matches!(c, Code::R0006 | Code::R0007) {
                *counts.entry(c).or_default() += 1;
            }
        }
        for (c, n) in counts {
            report += &format!("diag {shown} {c:?} {n}\n");
        }
    }
    if refused {
        return (format!("not written\n{outside}{report}end\n"), 2);
    }
    for (p, text) in &done {
        let to = out.join(p.strip_prefix(root).unwrap_or(p));
        let wrote = to
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&to, text));
        if let Err(e) = wrote {
            return (format!("{}: {e}\n", to.display()), 2);
        }
    }
    let lossy = !report.is_empty();
    (
        format!("converted {} file(s)\n{report}end\n", done.len()),
        u8::from(lossy),
    )
}

fn codes(d: &Diags) -> Vec<Code> {
    d.diags.iter().map(|d| d.code).collect()
}
