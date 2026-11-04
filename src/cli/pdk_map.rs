//! `pdk-map`: `interop.map` rows for a kit's scanned cells (`crate::pdk_map`), written with
//! their report.

use super::*;

const USAGE: &str = "usage: schemify-interop pdk-map --scan <dir> --dump <cdfnames.sfyx> [--lib <name>] -o <interop.map>";

/// `pdk-map --scan <dir> --dump <file> [--lib <name>] -o <file>`: maps every `.shprim` under
/// `<dir>` (Schemify library `<name>`, default the directory's name) onto the dump's cells;
/// writes the map and `<stem>.report.md|txt` beside it. 0 written (misses are report rows),
/// 1 the map failed its own check, 2 usage, I/O or a malformed input.
pub(super) fn pdk_map(args: &[String]) -> u8 {
    use crate::pdk_map;
    let mut opt = std::collections::BTreeMap::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match (a.as_str(), it.next()) {
            (k @ ("--scan" | "--dump" | "--lib" | "-o"), Some(v)) => opt.insert(k, v.as_str()),
            _ => {
                eprintln!("{USAGE}");
                return 2;
            }
        };
    }
    let (Some(scan), Some(dump), Some(out)) = (opt.get("--scan"), opt.get("--dump"), opt.get("-o"))
    else {
        eprintln!("{USAGE}");
        return 2;
    };
    let (scan, dump, out) = (Path::new(scan), Path::new(dump), Path::new(out));
    if !scan.is_dir() {
        eprintln!("schemify-interop: {}: not a directory", scan.display());
        return 2;
    }
    let lib = opt.get("--lib").map_or_else(
        || {
            std::fs::canonicalize(scan)
                .ok()
                .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
                .unwrap_or_default()
        },
        |l| (*l).to_owned(),
    );
    let mut paths = Vec::new();
    walk_cells(scan, &mut paths);
    let mut cells = Vec::new();
    for p in paths
        .iter()
        .filter(|p| p.extension().is_some_and(|x| x == "shprim"))
    {
        let name = p.file_stem().unwrap_or_default().to_string_lossy();
        let bytes = match std::fs::read(p) {
            Ok(b) => b,
            Err(e) => return io_err(p, e),
        };
        match pdk_map::scanned(&name, &bytes) {
            Ok(c) => cells.push(c),
            Err(e) => {
                eprintln!("schemify-interop: {}: {e}", p.display());
                return 2;
            }
        }
    }
    let kit = match std::fs::read_to_string(dump) {
        Ok(t) => match pdk_map::read_dump(&t) {
            Ok(k) => k,
            Err(e) => {
                eprintln!("schemify-interop: {}: {e}", dump.display());
                return 2;
            }
        },
        Err(e) => return io_err(dump, e),
    };
    let (text, notes) = match pdk_map::map(&lib, &cells, &kit) {
        Ok(m) => m,
        Err(errs) => {
            for e in errs {
                eprintln!("schemify-interop: {}: {e}", out.display());
            }
            return 1;
        }
    };
    let base = out.with_extension("");
    let base = base.to_string_lossy();
    for (p, b) in [
        (out.to_path_buf(), text),
        (
            PathBuf::from(format!("{base}.report.md")),
            notes.markdown("PDK map"),
        ),
        (PathBuf::from(format!("{base}.report.txt")), notes.text()),
    ] {
        if let Err(e) = std::fs::write(&p, b) {
            return io_err(&p, e);
        }
    }
    let n = notes.counts();
    println!(
        "pdk-map: {} of {} cells mapped, {} unmapped ({base}.report.md)",
        n.get("mapped").unwrap_or(&0),
        cells.len(),
        n.get("needs_review:unmapped").unwrap_or(&0)
    );
    0
}
