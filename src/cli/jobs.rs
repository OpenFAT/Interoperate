//! The job subcommands (`virtuoso-export`, `virtuoso-import`, `xschem-export`,
//! `install-skill`): arguments → a `job::Command`, run against the file system.

use super::*;

/// Runs job subcommand `cmd` on `args`: 0 written, 1 refused, 2 a usage or I/O error.
pub(super) fn job(cmd: &str, args: &[String]) -> u8 {
    match job_command(cmd, args) {
        Some((project, c)) => drive(&project, c),
        None => {
            eprintln!("{JOB_USAGE}");
            2
        }
    }
}

pub(super) const JOB_USAGE: &str = "\
usage: schemify-interop virtuoso-export <project-dir> <cell-file|dir>... -o <file.sfyx> [--core-cells] [--lib L=V]...
       schemify-interop virtuoso-import <project-dir> <file.sfyx> [--analoglib] [--lib L=V]...
       schemify-interop xschem-export <project-dir> <cell-file|dir>... -o <out-dir>
       schemify-interop install-skill <dir>
       schemify-interop pdk-derived <project-dir> <char.sfyx>
       schemify-interop virtuoso-refresh <project-dir> --scratch <lib> -o <dir> | --apply <dir>
       schemify-interop pdk-scan --protocol | --remote-script | [--stdin] --root <dir> --entry <path> --dialect spice|hspice|spectre [--section <s>] [--lib <name>]
       schemify-interop pdk-models --root <dir> --entry <relpath>... --from <d> --to <d> -o <outdir>   (d: spice|hspice|spectre|vacask|xyce)
(cell files, .sfyx paths and output paths are relative to <project-dir>; a directory names every
cell file under it)";

/// Parses the arguments of a job subcommand into (project directory, command).
pub(super) fn job_command(cmd: &str, args: &[String]) -> Option<(PathBuf, crate::job::Command)> {
    use crate::job::Command;
    use crate::virtuoso::{ExportOptions, ImportOptions};
    if cmd == "install-skill" {
        let [dir] = args else { return None };
        return Some((
            PathBuf::from(dir),
            Command::InstallSkill { dir: String::new() },
        ));
    }
    let (project, rest) = args.split_first()?;
    let mut paths = Vec::new();
    let mut out = None;
    let mut iopts = ImportOptions::default();
    // Export defaults to analogLib masters (§6.5): what ADE users place; `--core-cells`
    // keeps the `schemify_core` mirror cells.
    let mut opts = ExportOptions {
        producer: format!("schemify-interop {}", env!("CARGO_PKG_VERSION")),
        analoglib: true,
        ..ExportOptions::default()
    };
    let mut it = rest.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-o" => out = Some(it.next()?.clone()),
            "--analoglib" => {
                opts.analoglib = true;
                iopts.analoglib = true;
            }
            "--core-cells" => opts.analoglib = false,
            "--lib" => {
                // Export: Schemify library L → Virtuoso library V. Import: Virtuoso V → Schemify L.
                let (l, v) = it.next()?.split_once('=')?;
                opts.lib_map.insert(l.to_owned(), v.to_owned());
                iopts.lib_map.insert(v.to_owned(), l.to_owned());
            }
            f if f.starts_with("--") => return None,
            p => paths.push(p.replace('\\', "/")),
        }
    }
    // A directory names every cell file under it (a whole library: PDK-Manager exports an
    // install whose cells it cannot list).
    if cmd.ends_with("-export") {
        let root = Path::new(project);
        let mut all = Vec::new();
        for p in paths {
            let dir = root.join(&p);
            if dir.is_dir() {
                let mut found = Vec::new();
                walk_cells(&dir, &mut found);
                all.extend(
                    found
                        .iter()
                        .map(|f| slash(f.strip_prefix(root).unwrap_or(f))),
                );
            } else {
                all.push(p);
            }
        }
        paths = all;
    }
    let c = match cmd {
        "virtuoso-export" if !paths.is_empty() => Command::VirtuosoExport {
            tops: paths,
            out: out?,
            opts,
        },
        "virtuoso-import" if paths.len() == 1 && out.is_none() => Command::VirtuosoImport {
            sfyx: paths.remove(0),
            opts: iopts,
        },
        "xschem-export" if !paths.is_empty() => Command::XschemExport {
            tops: paths,
            out_dir: out?,
        },
        _ => return None,
    };
    Some((PathBuf::from(project), c))
}

/// Runs a job against the file system under `project`: 0 written, 1 refused (problems
/// printed), 2 an I/O error.
fn drive(project: &Path, command: crate::job::Command) -> u8 {
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
                return 0;
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
