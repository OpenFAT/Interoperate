//! `maestro-results` (spec `interop.md` §9.10): a maestro history, as `SfyAdeResults`
//! dumped it, → one imported Schemify run of a bench (run.md §4, waves.md §9).

use super::*;
use crate::virtuoso::results::{self, History, wave};
use schemify_format::{RecordKind, parse};

const USAGE: &str = "usage: schemify-interop maestro-results <project-dir> <dump.sfyx> <bench.shbench>   (paths relative to <project-dir>)";

/// `maestro-results <project-dir> <dump.sfyx> <bench>`: writes
/// `.schemify/runs/v<hash16>/` under the project, the history's run of `bench`: `results`
/// (with `source virtuoso …`), the `bench` sidecar, `<job>/out.raw` per point whose
/// waveforms were exported, and the history's `.rdb` and setup beside them. 0 written,
/// 1 refused (the design does not elaborate, or another history holds that directory),
/// 2 usage or I/O error.
pub(super) fn maestro_results(args: &[String]) -> u8 {
    let [project, dump, bench] = args else {
        eprintln!("{USAGE}");
        return 2;
    };
    let root = Path::new(project);
    let dump_path = root.join(dump);
    let base = dump_path.parent().unwrap_or(root).to_path_buf();
    let read = |p: &Path| std::fs::read(p).map_err(|e| io_err(p, e));
    let text = match read(&dump_path) {
        Ok(b) => String::from_utf8_lossy(&b).into_owned(),
        Err(c) => return c,
    };
    let d = match results::parse(&text) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("schemify-interop: {}: {e}", dump_path.display());
            return 2;
        }
    };
    let rdb_path = base.join(&d.rdb);
    let rdb = match read(&rdb_path) {
        Ok(b) => b,
        Err(c) => return c,
    };
    let h = match results::convert(&d, &rdb) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("schemify-interop: {}: {e}", rdb_path.display());
            return 2;
        }
    };
    match import(root, bench, &base, &h) {
        Ok(dir) => {
            for n in &h.notes {
                println!("note: {n}");
            }
            println!(
                "{} job(s) of test {} ({}) → {}",
                h.jobs.len(),
                h.test,
                h.dump.name,
                slash(&dir)
            );
            0
        }
        Err((code, msg)) => {
            eprintln!("schemify-interop: {msg}");
            code
        }
    }
}

/// Writes the run of `h` for `bench` and returns its directory.
fn import(root: &Path, bench: &str, base: &Path, h: &History) -> Result<PathBuf, (u8, String)> {
    let path = root.join(bench);
    let bytes = std::fs::read(&path).map_err(|e| (2, format!("{}: {e}", path.display())))?;
    let doc = parse(&bytes).map_err(|e| (1, format!("{bench}: {e}")))?;
    let name = bench_name(root, bench)?;
    // run.md §4 job key lines: `<config|-> <corner|-> <sample|-> var=value …`.
    let keys: Vec<String> = h
        .jobs
        .iter()
        .map(|j| {
            let mut k = format!("- {} -", j.corner.as_deref().unwrap_or("-"));
            for (n, v) in &j.sets {
                k.push_str(&format!(" {n}={v}"));
            }
            k
        })
        .collect();
    let hex = design_hash(root, bench, &keys)?;
    let dir = root.join(".schemify/runs").join(format!("v{}", &hex[..16]));
    let source = format!(
        "  source virtuoso {}/{}/{} ",
        h.dump.view.0, h.dump.view.1, h.dump.view.2
    );
    if let Ok(old) = std::fs::read_to_string(dir.join("results"))
        && let Some(line) = old.lines().find(|l| l.starts_with(&source))
        && line != format!("{source}{}", h.dump.name)
    {
        return Err((
            1,
            format!(
                "{}: holds another history of the same design ({line}); remove it to import {}",
                slash(&dir),
                h.dump.name
            ),
        ));
    }
    let names = |kind: RecordKind| -> Vec<String> {
        (1..doc.record_count())
            .filter(|&r| doc.record_kind(r) == kind)
            .map(|r| doc.field(r, 0).unwrap_or("").to_owned())
            .collect()
    };
    let (outputs, specs, vars) = (
        names(RecordKind::Output),
        names(RecordKind::Spec),
        names(RecordKind::Var),
    );
    let text = results::results_file(h, &hex, &outputs, &specs, &vars);
    let io = |p: &Path, e: std::io::Error| (2, format!("{}: {e}", p.display()));
    let write = |p: &Path, b: &[u8]| {
        std::fs::create_dir_all(p.parent().expect("under the run dir")).map_err(|e| io(p, e))?;
        std::fs::write(p, b).map_err(|e| io(p, e))
    };
    for (j, job) in h.jobs.iter().enumerate() {
        if job.waves.is_empty() {
            continue;
        }
        let src = base.join(&job.waves);
        let t = std::fs::read_to_string(&src).map_err(|e| io(&src, e))?;
        let a = wave::parse(&t).map_err(|e| (2, format!("{}: {e}", src.display())))?;
        write(&dir.join(j.to_string()).join("out.raw"), &wave::raw(&a))?;
        let skipped = wave::skipped(&t);
        if !skipped.is_empty() {
            println!(
                "note: point {j}: {} result value(s) are no waveform and are not mapped (the \
                 operating point and noise summaries come from Schemify's own run): {}",
                skipped.len(),
                skipped.join(" ")
            );
        }
    }
    for (from, to) in [(&h.dump.rdb, "maestro.rdb"), (&h.dump.setup, "history.zip")] {
        if !from.is_empty() {
            let src = base.join(from);
            write(
                &dir.join(to),
                &std::fs::read(&src).map_err(|e| io(&src, e))?,
            )?;
        }
    }
    write(&dir.join("bench"), format!("{name}\n").as_bytes())?;
    write(&dir.join("results"), text.as_bytes())?;
    Ok(dir)
}

/// The design hash Schemify's run gives `bench` with these job key lines (run.md §4).
///
/// # Errors
///
/// Always, for now: the hash needs elaboration (the cells each config reaches), and neither
/// Schemify's public SDK nor its shipped command line offers it.
// ponytail: refuses until the plugin ABI can import a run (the host hashes the design and
// writes `.schemify/runs/v<hash>/`, as Simulate does); every other step below is in place.
fn design_hash(_root: &Path, bench: &str, _keys: &[String]) -> Result<String, (u8, String)> {
    Err((
        2,
        format!(
            "{bench}: maestro-results needs the design hash of Schemify's run (run.md §4), \
             which Schemify's public interface does not offer yet"
        ),
    ))
}

/// The bench's display name for the run's `bench` sidecar: `lib:dir/stem`, or `dir/stem` in
/// the project library (a `lib … path` directory of the manifest holds it, else the project).
fn bench_name(root: &Path, bench: &str) -> Result<String, (u8, String)> {
    let manifest = std::fs::read(root.join("schemify.project"))
        .map_err(|e| (2, format!("schemify.project: {e}")))?;
    let libs = crate::design::manifest_libs(&manifest).unwrap_or_default();
    let bench = bench.replace('\\', "/");
    let stem = bench.rsplit_once('.').map_or(bench.as_str(), |(s, _)| s);
    for (lib, dir) in &libs {
        if let Some(rel) = stem
            .strip_prefix(dir.as_str())
            .and_then(|r| r.strip_prefix('/'))
        {
            return Ok(format!("{lib}:{rel}"));
        }
    }
    Ok(stem.to_owned())
}
