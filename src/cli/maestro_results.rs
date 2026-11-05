//! `maestro-results` (spec `interop.md` §9.10): a maestro history, as `SfyAdeResults`
//! dumped it, → one imported Schemify run of a bench (run.md §4, waves.md §9).

use super::*;
use crate::virtuoso::results::{self, History, wave};
use schemify_doc::{Assignment, DocHandle, LibSource, Project};

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
    let (mut project, top, name) = open_project(root, bench).map_err(|e| (2, e))?;
    let plan = schemify_run::Plan::read(project.doc(top).expect("open"))
        .map_err(|e| (1, format!("{bench}: {e}")))?;
    let mut jobs = schemify_run::Jobs::new();
    for j in &h.jobs {
        let sets: Vec<Assignment<&str>> = j
            .sets
            .iter()
            .map(|(n, v)| Assignment {
                name: n.as_str(),
                value: v.as_str(),
            })
            .collect();
        jobs.push(None, j.corner.as_deref(), schemify_run::NO_SAMPLE, &sets)
            .map_err(|e| (1, e.to_string()))?;
    }
    // run.md §4: the environment is part of the hash, as `schemify run` passes it.
    let mut env: Vec<(String, String)> = std::env::vars().collect();
    env.sort_unstable();
    let env: Vec<Assignment<&str>> = env
        .iter()
        .map(|(n, v)| Assignment {
            name: n.as_str(),
            value: v.as_str(),
        })
        .collect();
    // ponytail: open what elaboration asks for, at most once per row.
    let digest = loop {
        match schemify_run::design(
            &project,
            top,
            &jobs,
            &env,
            schemify_elab::Dialect::Spectre,
            &mut schemify_elab::no_funcs,
        ) {
            Ok(d) => break d,
            Err(schemify_run::Blocked::NeedsOpen(rows)) => {
                for r in rows {
                    open_row(&mut project, root, r as usize).map_err(|e| (1, e))?;
                }
            }
            Err(schemify_run::Blocked::Violations(v)) => {
                return Err((1, format!("{bench} does not elaborate: {}", v.join("; "))));
            }
        }
    };
    let hex = schemify_doc::hex(&digest);
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
    let names = |v: Vec<&str>| v.into_iter().map(str::to_owned).collect::<Vec<_>>();
    let outputs = names(plan.outputs.iter().map(|o| o.name.as_str()).collect());
    let specs = names(
        plan.specs
            .iter()
            .map(|s| plan.outputs[usize::from(s.output)].name.as_str())
            .collect(),
    );
    let vars = names(plan.vars.iter().map(|v| v.name.as_str()).collect());
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

/// The project at `root` loaded as `schemify run` loads it (manifest, lockfile, every
/// local library indexed from its directory), with `bench` open: (project, bench handle,
/// the bench's name for the run's `bench` sidecar, `lib:stem` or `stem`).
fn open_project(root: &Path, bench: &str) -> Result<(Project, DocHandle, String), String> {
    let manifest = std::fs::read(root.join("schemify.project")).ok();
    let lock = std::fs::read(root.join("schemify.lock")).ok();
    let mut p = Project::new(manifest.as_deref(), lock.as_deref(), 0)
        .map_err(|e| format!("schemify.project: {e:?}"))?;
    let dirs = lib_dirs(&p, root);
    let nested: Vec<&PathBuf> = dirs.iter().skip(1).flatten().collect();
    let want = root.join(bench);
    let mut top = None;
    for (row, dir) in dirs.iter().enumerate() {
        let Some(dir) = dir else { continue };
        let mut files = Vec::new();
        list(dir, 0, &nested, dir, &mut files);
        let mut listed = Vec::new();
        for (rel, path) in &files {
            listed.push((
                rel.as_str(),
                std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?,
            ));
        }
        let lib = p.libs().name[row].clone();
        let refs: Vec<(&str, &[u8])> = listed.iter().map(|(r, b)| (*r, b.as_slice())).collect();
        p.index_library(&lib, &dir.display().to_string(), &refs)
            .map_err(|c| format!("lib {lib}: {c}"))?;
        if let Some((rel, b)) = files
            .iter()
            .zip(&listed)
            .find(|(f, _)| f.1 == want)
            .map(|(f, l)| (&f.0, &l.1))
        {
            let o = p
                .open(&lib, rel, b, None)
                .map_err(|e| format!("{bench}: {e:?}"))?;
            let stem = rel.rsplit_once('.').map_or(rel.as_str(), |(s, _)| s);
            let name = if lib.is_empty() {
                stem.to_owned()
            } else {
                format!("{lib}:{stem}")
            };
            top = Some((o.handle, name));
        }
    }
    // D10: the implicit `lib core builtin`, indexed from the binary under the name the
    // Schemify CLI gives it (`libs::BUILTIN_DIR`).
    if p.libs()
        .find("core")
        .is_some_and(|r| p.libs().source[r] == LibSource::Builtin)
    {
        p.index_library("core", "<builtin>/core", schemify_doc::core_library())
            .map_err(|c| format!("lib core: {c}"))?;
    }
    let (h, name) = top.ok_or_else(|| format!("{bench}: no library of the project holds it"))?;
    Ok((p, h, name))
}

/// Each library row's directory (`None`: builtin, or a source not on disk).
fn lib_dirs(p: &Project, root: &Path) -> Vec<Option<PathBuf>> {
    let l = p.libs();
    (0..l.name.len())
        .map(|r| match &l.source[r] {
            LibSource::Project => Some(root.to_path_buf()),
            LibSource::Path { path } => {
                let d = root.join(path);
                Some(l.subdir[r].as_ref().map_or(d.clone(), |s| d.join(s)))
            }
            _ => None,
        })
        .collect()
}

/// Cell files under `dir` (relative to `lib`), sorted, skipping hidden entries, `target`
/// and other libraries' directories, as the Schemify CLI lists a library.
fn list(dir: &Path, depth: u32, skip: &[&PathBuf], lib: &Path, out: &mut Vec<(String, PathBuf)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();
    for p in paths {
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or(".");
        if name.starts_with('.') || name == "target" || skip.iter().any(|s| **s == p) {
            continue;
        }
        if p.is_dir() {
            if depth < 64 {
                list(&p, depth + 1, skip, lib, out);
            }
        } else if [".shprim", ".shcomp", ".shbench"]
            .iter()
            .any(|e| name.ends_with(e))
            && let Ok(rel) = p.strip_prefix(lib)
        {
            out.push((slash(rel), p.clone()));
        }
    }
}

/// Opens indexed cell `row` (its file, or the bundled core cell).
fn open_row(p: &mut Project, root: &Path, row: usize) -> Result<(), String> {
    let c = p.cells();
    let s = p.strings();
    let (dir, stem) = (s.get(c.dir[row]).to_owned(), s.get(c.stem[row]).to_owned());
    let rel = if dir.is_empty() {
        format!("{stem}.{}", c.kind[row].as_str())
    } else {
        format!("{dir}/{stem}.{}", c.kind[row].as_str())
    };
    let lib = usize::from(c.lib[row]);
    let name = p.libs().name[lib].clone();
    let bytes = match &lib_dirs(p, root)[lib] {
        Some(d) => std::fs::read(d.join(&rel)).map_err(|e| format!("{name}/{rel}: {e}"))?,
        None => schemify_doc::core_file(&stem)
            .ok_or_else(|| format!("{name}:{stem}: not on disk"))?
            .to_vec(),
    };
    p.open(&name, &rel, &bytes, None)
        .map(|_| ())
        .map_err(|e| format!("{name}/{rel}: {e:?}"))
}
