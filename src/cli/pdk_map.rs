//! `pdk-map`: `interop.map` rows for a kit's scanned cells (`crate::pdk_map`), written with
//! their report.

use super::*;

const USAGE: &str = "usage: schemify-interop pdk-map --scan <dir> (--dump <cdfnames.sfyx> | --host <ssh name> --vlib <lib>...) [--lib <name>] -o <interop.map>
       schemify-interop pdk-map --remote-script --vlib <lib>...";

/// `pdk-map --scan <dir> (--dump <file> | --host <h> --vlib <l>...) [--lib <name>] -o <file>`:
/// maps every `.shprim` under `<dir>` (Schemify library `<name>`, default the directory's
/// name) onto the cells of a `SfyCdfNames` dump, read from `<file>` or made on host `<h>`
/// (`ssh <h> sh -s` on [`crate::pdk_map::remote_script`], with this environment's
/// [`crate::pdk_map::ENV`]); writes the map and `<stem>.report.md|txt` beside it.
/// `--remote-script` prints that script instead. 0 written (misses are report rows), 1 the
/// map failed its own check, 2 usage, I/O, a malformed input or a failed remote dump.
pub(super) fn pdk_map(args: &[String]) -> u8 {
    use crate::pdk_map;
    let mut opt = std::collections::BTreeMap::new();
    let (mut vlibs, mut script) = (Vec::new(), false);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--remote-script" {
            script = true;
            continue;
        }
        match (a.as_str(), it.next()) {
            ("--vlib", Some(v)) => vlibs.push(v.clone()),
            (k @ ("--scan" | "--dump" | "--host" | "--lib" | "-o"), Some(v)) => {
                opt.insert(k, v.as_str());
            }
            _ => {
                eprintln!("{USAGE}");
                return 2;
            }
        }
    }
    let env = |k: &str| std::env::var(k).ok();
    if script {
        if vlibs.is_empty() || !opt.is_empty() {
            eprintln!("{USAGE}");
            return 2;
        }
        print!("{}", pdk_map::remote_script(&vlibs, env));
        return 0;
    }
    let (Some(scan), Some(out)) = (opt.get("--scan"), opt.get("-o")) else {
        eprintln!("{USAGE}");
        return 2;
    };
    let (scan, out) = (Path::new(scan), Path::new(out));
    if !scan.is_dir() {
        eprintln!("schemify-interop: {}: not a directory", scan.display());
        return 2;
    }
    let (dump, text) = match (opt.get("--dump"), opt.get("--host")) {
        (Some(d), None) if vlibs.is_empty() => match std::fs::read_to_string(d) {
            Ok(t) => (d.to_string(), t),
            Err(e) => return io_err(Path::new(d), e),
        },
        (None, Some(h)) if !vlibs.is_empty() => {
            match remote_dump(h, &pdk_map::remote_script(&vlibs, env)) {
                Ok(t) => (format!("{h}: SfyCdfNames"), t),
                Err(code) => return code,
            }
        }
        _ => {
            eprintln!("{USAGE}");
            return 2;
        }
    };
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
    let kit = match pdk_map::read_dump(&text) {
        Ok(k) => k,
        Err(e) => {
            eprintln!("schemify-interop: {dump}: {e}");
            return 2;
        }
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

/// The stdout of `ssh <host> sh -s` fed `script` (its stderr passes through); exit 2 when
/// ssh cannot start or the script fails.
fn remote_dump(host: &str, script: &str) -> Result<String, u8> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut child = Command::new("ssh")
        .args([host, "sh", "-s"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| {
            eprintln!("schemify-interop: ssh: {e}");
            2
        })?;
    let mut stdin = child.stdin.take().expect("piped stdin");
    let script = script.to_owned();
    let feed = std::thread::spawn(move || stdin.write_all(script.as_bytes()));
    let out = child.wait_with_output().map_err(|e| {
        eprintln!("schemify-interop: ssh: {e}");
        2
    })?;
    let _ = feed.join();
    if !out.status.success() {
        eprintln!(
            "schemify-interop: pdk-map: the dump on {host} failed ({})",
            out.status
        );
        return Err(2);
    }
    String::from_utf8(out.stdout).map_err(|_| {
        eprintln!("schemify-interop: pdk-map: the dump on {host} is not UTF-8");
        2
    })
}
