//! `xschem-import` converts xschem libraries — a PDK's device library or whole design
//! trees — into Schemify libraries, with device limits from model cards, `drc=` procs
//! and tech files (D14), then checks every output with the import oracle.
//!
//! ```text
//! schemify-interop xschem-import <lib-dir>... [--root <dir>]... [--models <file>[:<section>]]...
//!     [--spiceinit <file>] [--tech <file>] [--cdf <dump>] [--xschemrc <file>]...
//!     [--no-core] -o <out-dir>
//! ```
//!
//! Each `<lib-dir>` is one element of the xschem library search path, in the order
//! given, and every `.sym`/`.sch` under it is converted. `--root` adds a search-path
//! element whose files are only referenced (xschem's `xschem_library` parent, so that
//! `devices/res.sym` resolves) — they must also be under some `<lib-dir>`. `--xschemrc` (repeatable: pass
//! `xschem-drc` too) supplies PDK path variables and `drc` procs. `--spiceinit` reads an
//! ngspice `.spiceinit`: its `sourcepath` (`$VAR`s from the environment) is where model
//! includes are looked up. A Magic `--tech` file's `include`s are read beside it. Writes the outputs
//! (`<lib>/<cell>.<ext>`, `schemify.project`, `import-report.{md,txt}`) and prints a
//! summary. The reading and translating is `job::XschemImport`, the job the guest's
//! `interop.xschem_import` runs; the side inputs and the check are this command's. Exit
//! status: 0 ok, 1 an input could not be read or an output failed its check, 2 usage or I/O
//! error.

use super::*;

pub(super) struct Args {
    /// Search path in order: (dir, convert its files).
    dirs: Vec<(PathBuf, bool)>,
    models: Vec<(PathBuf, Option<String>)>,
    tech: Option<PathBuf>,
    spiceinit: Option<PathBuf>,
    cdf: Option<PathBuf>,
    rcs: Vec<PathBuf>,
    core: bool,
    out: PathBuf,
}

pub(super) fn usage() -> u8 {
    eprintln!(
        "usage: schemify-interop xschem-import <lib-dir>... [--root <dir>]... [--models <file>[:<section>]]... \
         [--spiceinit <file>] [--tech <file>] [--cdf <dump>] [--xschemrc <file>]... \
         [--no-core] -o <out-dir>"
    );
    2
}

fn parse_args(args: &[String]) -> Option<Args> {
    let mut a = Args {
        dirs: Vec::new(),
        models: Vec::new(),
        tech: None,
        spiceinit: None,
        cdf: None,
        rcs: Vec::new(),
        core: true,
        out: PathBuf::new(),
    };
    let mut it = args.iter().cloned();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--models" => {
                let (path, section) = crate::job::models_arg(&it.next()?);
                a.models.push((PathBuf::from(path), section));
            }
            "--root" => a.dirs.push((PathBuf::from(it.next()?), false)),
            "--tech" => a.tech = Some(PathBuf::from(it.next()?)),
            "--spiceinit" => a.spiceinit = Some(PathBuf::from(it.next()?)),
            "--cdf" => a.cdf = Some(PathBuf::from(it.next()?)),
            "--xschemrc" => a.rcs.push(PathBuf::from(it.next()?)),
            "--no-core" => a.core = false,
            "-o" => a.out = PathBuf::from(it.next()?),
            s if s.starts_with('-') => return None,
            _ => a.dirs.push((PathBuf::from(arg), true)),
        }
    }
    (a.dirs.iter().any(|d| d.1) && !a.out.as_os_str().is_empty()).then_some(a)
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

/// `xschem-import` (module docs): `args` are the subcommand's arguments. The host-only side
/// inputs (`--spiceinit --tech --cdf`) are read here; everything else is the job the guest's
/// `interop.xschem_import` runs (`job::XschemImport`).
pub(super) fn xschem_import(args: &[String]) -> u8 {
    let Some(args) = parse_args(args) else {
        return usage();
    };
    let mut dirs = Vec::new();
    for (d, convert) in &args.dirs {
        let Ok(d) = std::fs::canonicalize(d) else {
            eprintln!("import: cannot open {}", d.display());
            return 2;
        };
        dirs.push((slash(&d), *convert));
    }
    let search_dirs: Vec<PathBuf> = args
        .spiceinit
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
    let grid_pm = args.tech.as_ref().and_then(|t| {
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
    let cdf = args
        .cdf
        .as_ref()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|t| crate::cdf::read(&t));
    let mut job = crate::job::Job::new(crate::job::Command::XschemImport(Box::new(
        crate::job::XschemImport {
            dirs,
            rcs: args.rcs.iter().map(|p| slash(p)).collect(),
            models: args
                .models
                .iter()
                .map(|(p, s)| (slash(p), s.clone()))
                .collect(),
            search_dirs,
            grid_pm,
            cdf,
            core: args.core,
            out_dir: slash(&args.out),
        },
    )));
    let out = format!("{}/", slash(&args.out).trim_end_matches('/'));
    let outputs: Vec<(String, Vec<u8>)> = match drive(Path::new(""), &mut job) {
        Ok(w) => w
            .into_iter()
            .map(|(p, b)| (p.strip_prefix(&out).unwrap_or(&p).to_owned(), b))
            .collect(),
        Err(code) => return code,
    };
    let imp = job.imported().expect("a finished import");
    for p in &imp.unread {
        eprintln!("import: {p}: cannot read");
    }
    for ((p, section), s) in args.models.iter().zip(&imp.scans) {
        eprintln!(
            "models {} [{}]: {} files, {} models, {} bins, {} issues",
            p.display(),
            section.as_deref().unwrap_or("-"),
            s.files.len(),
            s.models.name.len(),
            s.bins.model.len(),
            s.issues.len()
        );
    }
    summarize(&outputs, &imp.report, imp.drc_procs, grid_pm);
    let problems = crate::check_output(&outputs);
    for p in problems.iter().take(60) {
        eprintln!("check: {p}");
    }
    println!("check: {} problems", problems.len());
    if problems.is_empty() { 0 } else { 1 }
}

fn summarize(
    outputs: &[(String, Vec<u8>)],
    report: &crate::Report,
    procs: usize,
    grid_pm: Option<u32>,
) {
    let count = |ext: &str| outputs.iter().filter(|o| o.0.ends_with(ext)).count();
    println!(
        "inputs {} files; outputs {} shprim, {} shcomp, {} shbench; {} drc procs; grid {:?} pm",
        report.files.len(),
        count(".shprim"),
        count(".shcomp"),
        count(".shbench"),
        procs,
        grid_pm
    );
    println!(
        "devices instances mapped to core: {} / {}",
        report.core_instances, report.device_instances
    );
    let mut by_class = std::collections::BTreeMap::new();
    for it in &report.items {
        *by_class.entry(it.class.word()).or_insert(0u32) += 1;
    }
    for (c, n) in &by_class {
        if matches!(
            c.split(':').next(),
            Some("needs_review" | "dropped" | "alt-cell" | "unknown-token" | "rounding")
        ) {
            println!("  {c}: {n}");
        }
    }
    let reviewed: std::collections::BTreeSet<&str> = report
        .items
        .iter()
        .filter(|i| matches!(i.class, Class::Review(_)))
        .map(|i| report.files[i.file as usize].as_str())
        .collect();
    println!("  files with needs_review items: {}", reviewed.len());
    // Limits extracted: params carrying min/max/grid.
    let mut limits = 0;
    for (path, bytes) in outputs.iter().filter(|o| o.0.ends_with(".shprim")) {
        let Ok(d) = schemify_format::parse(bytes) else {
            continue;
        };
        for r in 1..d.record_count() {
            if d.record_kind(r) != schemify_format::RecordKind::Param {
                continue;
            }
            let get = |k| d.find_attr(r, k).and_then(|a| d.attr_value(a, 0));
            let (min, max, grid) = (get("min"), get("max"), get("grid"));
            if min.is_some() || max.is_some() {
                limits += 1;
                println!(
                    "  limit {path} {}: min {} max {} grid {}",
                    d.field(r, 0).unwrap_or(""),
                    min.unwrap_or("-"),
                    max.unwrap_or("-"),
                    grid.unwrap_or("-")
                );
            }
        }
    }
    println!("  params with min/max: {limits}");
}
