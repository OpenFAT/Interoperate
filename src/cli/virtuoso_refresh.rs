//! `virtuoso-refresh` (spec `interop.md` §8.3): the SKILL that measures stale derived
//! values, and the measured values back as `cached` lines.

use super::*;

/// `virtuoso-refresh <project-dir> --scratch <lib> -o <dir>` writes `<dir>/refresh.il`, the
/// SKILL that measures the derived values of every instance whose cached ones are stale
/// (spec §8.3); `virtuoso-refresh <project-dir> --apply <dir>` writes the measured values
/// (`<dir>/refresh.*.sfyx`) back as fresh `cached` lines. 0 done, 1 something stays stale,
/// 2 usage or I/O error.
pub(super) fn virtuoso_refresh(args: &[String]) -> u8 {
    use crate::virtuoso::characterize;
    const USAGE: &str = "usage: schemify-interop virtuoso-refresh <project-dir> --scratch <lib> -o <dir>\n       schemify-interop virtuoso-refresh <project-dir> --apply <dir>";
    // scratch: Some to write the request, None to apply the measurements.
    let (project, dir, scratch) = match args {
        [p, s, lib, o, d] | [p, o, d, s, lib] if s == "--scratch" && o == "-o" => (p, d, Some(lib)),
        [p, a, d] if a == "--apply" => (p, d, None),
        _ => {
            eprintln!("{USAGE}");
            return 2;
        }
    };
    let project = Path::new(project);
    let mut paths = Vec::new();
    walk_cells(project, &mut paths);
    let mut files = crate::design::Files::new();
    for p in paths.iter().chain([&project.join("schemify.project")]) {
        match std::fs::read(p) {
            Ok(b) => {
                let rel = slash(p.strip_prefix(project).unwrap_or(p));
                files.insert(rel, b);
            }
            Err(e) => return io_err(p, e),
        }
    }
    let dir = Path::new(dir);
    if let Some(scratch) = scratch {
        let (wants, notes) = characterize::refresh_wants(&files);
        for n in &notes {
            println!("{n}");
        }
        if wants.is_empty() {
            println!("nothing to refresh: every cached derived value is fresh");
            return u8::from(!notes.is_empty());
        }
        if let Err(e) = std::fs::create_dir_all(dir) {
            return io_err(dir, e);
        }
        let abs = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
        let il = dir.join("refresh.il");
        let text = characterize::refresh_skill(&wants, &slash(&abs), scratch);
        if let Err(e) = std::fs::write(&il, text) {
            return io_err(&il, e);
        }
        let n: usize = wants.iter().map(|w| w.points.len()).sum();
        println!(
            "{n} point(s) of {} cell(s) to measure: in Virtuoso, load(\"schemify.il\") then load(\"{}\"); then virtuoso-refresh {} --apply {}",
            wants.len(),
            slash(&abs.join("refresh.il")),
            slash(project),
            slash(dir)
        );
        return 0;
    }
    let mut measured = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        eprintln!("schemify-interop: cannot read {}", dir.display());
        return 2;
    };
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("refresh.") && n.ends_with(".sfyx"))
        })
        .collect();
    found.sort();
    for f in &found {
        match std::fs::read_to_string(f).map(|t| characterize::read(&t)) {
            Ok(Ok(d)) => measured.extend(d),
            Ok(Err(e)) => {
                eprintln!("schemify-interop: {}: {e}", f.display());
                return 2;
            }
            Err(e) => return io_err(f, e),
        }
    }
    let done = characterize::refresh_apply(&files, &measured);
    for (rel, bytes) in &done.files {
        let p = project.join(rel);
        if let Err(e) = std::fs::write(&p, bytes) {
            return io_err(&p, e);
        }
    }
    for n in done.rewritten.iter().chain(&done.stale) {
        println!("{n}");
    }
    println!("{} file(s) refreshed", done.files.len());
    u8::from(!done.stale.is_empty())
}
