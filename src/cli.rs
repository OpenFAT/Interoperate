//! The plugin's native command line (feature `cli`, binary `schemify-interop`): the
//! converters over the file system, for scripts, CI and corpus runs.
//!
//! ```text
//! schemify-interop virtuoso-export <project-dir> <cell-file>... -o <file.sfyx> [--core-cells] [--lib L=V]...
//! schemify-interop virtuoso-import <project-dir> <file.sfyx> [--analoglib] [--lib L=V]...
//! schemify-interop xschem-export   <project-dir> <cell-file>... -o <out-dir>
//! schemify-interop xschem-import   <lib-dir>... [--root <dir>]... [--xschemrc <file>]... [--no-core]
//!     [--models <file>[:<section>]]... [--spiceinit <file>] [--tech <file>] [--cdf <dump>] -o <out-dir>
//! schemify-interop install-skill   [<dir>]
//! schemify-interop pdk-rules       <project-dir> <char.sfyx>
//! ```
//!
//! The grammar is the editor's ([`crate::job::parse`]); the command line adds the project
//! directory the paths are relative to, and for `xschem-import` the D14 side inputs it reads
//! from the host before the job runs. `xschem-import` paths are host paths: each `<lib-dir>`
//! is one element of the xschem library search path, in order, converted wholesale; `--root`
//! adds an element that contributes only what the inputs reference from it, transitively, so
//! `--root <xschem_library> --root <pdk xschem dir>` converts the devices a design places and
//! no demo benches. `--xschemrc` (repeatable: pass `xschem-drc` too) supplies PDK path
//! variables and `drc` procs. `--spiceinit` reads an ngspice `.spiceinit`: its `sourcepath`
//! (`$VAR`s from the environment) is where model includes are looked up. A Magic `--tech`
//! file's `include`s are read beside it. Exit status: 0 ok, 1 refused or an output failed its
//! check, 2 usage or I/O error.

use std::path::{Path, PathBuf};

use crate::job::{Command, USAGE, parse};
use crate::{spice, tech};

fn usage() -> u8 {
    eprintln!(
        "usage: schemify-interop <command> ...\n{USAGE}\n(after the command, virtuoso-*/xschem-export take <project-dir>; xschem-import also takes --models <file>[:<section>] --spiceinit <file> --tech <file> --cdf <dump>; pdk-rules <project-dir> <char.sfyx>)"
    );
    2
}

/// Every `.sym`/`.sch` under `dir`, recursively, sorted, with the `.cir` files netlist
/// text may include (side inputs, import spec §1).
fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut v: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    v.sort();
    for p in v {
        if p.is_dir() {
            walk(&p, out);
        } else if p
            .extension()
            .is_some_and(|x| x == "sym" || x == "sch" || x == "cir")
        {
            out.push(p);
        }
    }
}

/// `text` with every `$NAME` / `${NAME}` replaced from the environment; `None` when one
/// is undefined.
fn expand_env(text: &str) -> Option<String> {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find('$') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        let (name, tail) = match after.strip_prefix('{') {
            Some(b) => {
                let end = b.find('}')?;
                (&b[..end], &b[end + 1..])
            }
            None => {
                let end = after
                    .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                    .unwrap_or(after.len());
                (&after[..end], &after[end..])
            }
        };
        out.push_str(&std::env::var(name).ok()?);
        rest = tail;
    }
    out.push_str(rest);
    Some(out)
}

fn slash(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

/// Runs the subcommand `args[0]` on the rest of `args` (the command line without the
/// program name) and returns the exit status: 0 ok, 1 refused or an output failed its
/// check, 2 an unknown subcommand, a usage error or an I/O error.
pub fn run(args: &[String]) -> u8 {
    let Some((cmd, rest)) = args.split_first() else {
        return usage();
    };
    match cmd.as_str() {
        "pdk-rules" => pdk_rules(rest),
        "xschem-import" => xschem_import(rest),
        "install-skill" => match parse(cmd, rest) {
            Some(c) => drive(Path::new(""), c),
            None => usage(),
        },
        _ => match rest.split_first() {
            Some((project, rest)) => match parse(cmd, rest) {
                Some(c) => drive(Path::new(project), c),
                None => usage(),
            },
            None => usage(),
        },
    }
}

/// Runs a job against the file system under `project`: 0 written, 1 refused (problems
/// printed), 2 an I/O error.
fn drive(project: &Path, command: Command) -> u8 {
    use crate::job::{Job, Step};
    let mut job = Job::new(command);
    loop {
        match job.step() {
            Step::Read(paths) => {
                for p in paths {
                    let bytes = std::fs::read(project.join(&p)).ok();
                    job.supply(&p, bytes);
                }
            }
            Step::List(dirs) => {
                for d in dirs {
                    let root = project.join(&d);
                    let listing = root.is_dir().then(|| {
                        let mut paths = Vec::new();
                        walk(&root, &mut paths);
                        paths
                            .iter()
                            .filter_map(|q| q.strip_prefix(&root).ok().map(slash))
                            .collect()
                    });
                    job.supply_list(&d, listing);
                }
            }
            Step::Done { writes, summary } => {
                for (p, b) in &writes {
                    let path = project.join(p);
                    if let Some(dir) = path.parent()
                        && let Err(e) = std::fs::create_dir_all(dir)
                    {
                        eprintln!("schemify-interop: {}: {e}", dir.display());
                        return 2;
                    }
                    if let Err(e) = std::fs::write(&path, b) {
                        eprintln!("schemify-interop: {}: {e}", path.display());
                        return 2;
                    }
                }
                println!("{summary}");
                return u8::from(job.problems() > 0);
            }
            Step::Failed(problems) => {
                for p in problems {
                    eprintln!("schemify-interop: {p}");
                }
                return 1;
            }
        }
    }
}

/// `pdk-rules <project-dir> <char.sfyx>` (spec `interop.md` §8.3): applies the rules a
/// characterization explains to the project's mirror cells (`computed`, `vderived`,
/// `valid`) and drops the overrides of the now-computed params from every design instance
/// of those cells. Prints what it did; 0 done, 1 nothing applied, 2 usage or I/O error.
fn pdk_rules(args: &[String]) -> u8 {
    use crate::virtuoso::characterize;
    let [project, char_file] = args else {
        eprintln!("usage: schemify-interop pdk-rules <project-dir> <char.sfyx>");
        return 2;
    };
    let project = Path::new(project);
    let io = |p: &Path, e: std::io::Error| {
        eprintln!("schemify-interop: {}: {e}", p.display());
        2
    };
    let text = match std::fs::read_to_string(char_file) {
        Ok(t) => t,
        Err(e) => return io(Path::new(char_file), e),
    };
    let devs = match characterize::read(&text) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("schemify-interop: {char_file}: {e}");
            return 2;
        }
    };
    let manifest = project.join("schemify.project");
    let libs = match std::fs::read(&manifest) {
        Ok(b) => crate::design::manifest_libs(&b).unwrap_or_default(),
        Err(e) => return io(&manifest, e),
    };
    let mut designs = Vec::new();
    walk_designs(project, &mut designs);
    let mut applied = 0;
    for d in &devs {
        let rules = d.rules();
        let dir = libs.get(&d.lib).map_or(d.lib.as_str(), String::as_str);
        let cell = project.join(dir).join(format!("{}.shprim", d.cell));
        let Ok(bytes) = std::fs::read(&cell) else {
            println!(
                "{}/{}: no mirror cell at {}: import the PDK first",
                d.lib,
                d.cell,
                cell.display()
            );
            continue;
        };
        let out = match characterize::apply_rules(&bytes, &rules) {
            Ok(o) => o,
            Err(e) => {
                eprintln!("schemify-interop: {}: {e}", cell.display());
                return 2;
            }
        };
        if let Err(e) = std::fs::write(&cell, out) {
            return io(&cell, e);
        }
        applied += 1;
        let names: Vec<&str> = rules.computed.iter().map(|c| c.0.as_str()).collect();
        let cellref = format!("{}:{}", d.lib, d.cell);
        let mut dropped = 0;
        for f in &designs {
            let Ok(b) = std::fs::read(f) else { continue };
            if let Some((t, n)) = characterize::strip_overrides(&b, &cellref, &names) {
                if let Err(e) = std::fs::write(f, t) {
                    return io(f, e);
                }
                dropped += n;
            }
        }
        println!(
            "{}/{}: computed {} ({}), unexplained {}, valid on ({}), unruled ({}), checked at {} points; {} instance override(s) dropped",
            d.lib,
            d.cell,
            names.len(),
            names.join(" "),
            if rules.unexplained.is_empty() {
                "none".into()
            } else {
                rules.unexplained.join(" ")
            },
            rules
                .legal
                .iter()
                .map(|l| l.0.as_str())
                .collect::<Vec<_>>()
                .join(" "),
            rules.unruled.join(" "),
            rules.points,
            dropped
        );
    }
    if applied == 0 { 1 } else { 0 }
}

/// Every `.shcomp`/`.shbench` under `dir`, recursively, sorted (`.schemify/` skipped).
fn walk_designs(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut v: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    v.sort();
    for p in v {
        if p.is_dir() {
            if p.file_name().is_some_and(|n| n != ".schemify") {
                walk_designs(&p, out);
            }
        } else if p
            .extension()
            .is_some_and(|x| x == "shcomp" || x == "shbench")
        {
            out.push(p);
        }
    }
}

/// `xschem-import` (module docs): `args` are the subcommand's arguments.
fn xschem_import(args: &[String]) -> u8 {
    // The host-only side inputs; the rest is the shared grammar.
    let mut models: Vec<(PathBuf, Option<String>)> = Vec::new();
    let (mut tech, mut spiceinit, mut cdf) = (None, None, None);
    let mut words = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut value = || it.next().map(PathBuf::from);
        match a.as_str() {
            "--models" => {
                let Some(v) = value() else { return usage() };
                let v = v.to_string_lossy().into_owned();
                models.push(match v.rsplit_once(':') {
                    Some((p, s)) if !s.contains('/') && !s.is_empty() => {
                        (PathBuf::from(p), Some(s.to_owned()))
                    }
                    _ => (PathBuf::from(v), None),
                });
            }
            "--tech" => tech = value(),
            "--spiceinit" => spiceinit = value(),
            "--cdf" => cdf = value(),
            _ => words.push(a.clone()),
        }
    }
    let Some(Command::XschemImport(mut x)) = parse("xschem-import", &words) else {
        return usage();
    };
    for d in x.convert.iter_mut().chain(x.roots.iter_mut()) {
        let Ok(c) = std::fs::canonicalize(&*d) else {
            eprintln!("import: cannot open {d}");
            return 2;
        };
        *d = slash(&c);
    }
    for rc in &x.rcs {
        if !Path::new(rc).is_file() {
            eprintln!("import: cannot read {rc}");
            return 2;
        }
    }
    let search_dirs: Vec<PathBuf> = spiceinit
        .as_ref()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|t| spice::spiceinit_sourcepath(&t))
        .unwrap_or_default()
        .iter()
        .filter_map(|d| match expand_env(d) {
            Some(d) => Some(PathBuf::from(d)),
            None => {
                eprintln!("import: sourcepath {d}: undefined variable, skipped");
                None
            }
        })
        .collect();
    let scans: Vec<spice::ModelScan> = models
        .iter()
        .map(|(p, section)| {
            let opts = spice::ScanOptions {
                section: section.as_deref(),
                search_dirs: &search_dirs,
            };
            let s = spice::scan(p, &opts, |q| std::fs::read_to_string(q).ok());
            eprintln!(
                "models {} [{}]: {} files, {} models, {} bins, {} issues",
                p.display(),
                section.as_deref().unwrap_or("-"),
                s.files.len(),
                s.models.name.len(),
                s.bins.model.len(),
                s.issues.len()
            );
            s
        })
        .collect();
    let grid_pm = tech.as_ref().and_then(|t| {
        let text = std::fs::read_to_string(t).ok()?;
        if t.extension().is_some_and(|x| x == "lyt") {
            tech::read_lyt(&text).ok()?.default_grid_pm
        } else {
            // Magic splices `include`d files in; they sit beside the tech file.
            let dir = t.parent().unwrap_or(Path::new(""));
            let read = |n: &str| {
                std::fs::read_to_string(dir.join(n))
                    .or_else(|_| std::fs::read_to_string(dir.join(format!("{n}.tech"))))
                    .ok()
            };
            let magic = tech::read_magic_with(&text, read);
            for n in &magic.unresolved {
                eprintln!("import: tech include {n}: not found beside {}", t.display());
            }
            magic.manufacturing_grid_pm()
        }
    });
    let cdf = cdf
        .as_ref()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|t| crate::cdf::read(&t));
    x.models = scans;
    x.grid_pm = grid_pm;
    x.cdf = cdf;
    // The same job the plugin runs (`interop.xschem_import`), over host paths.
    drive(Path::new(""), Command::XschemImport(x))
}
