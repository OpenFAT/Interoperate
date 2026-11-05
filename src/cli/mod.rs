//! The plugin's native command line (feature `cli`, binary `schemify-interop`): the
//! converters over the file system, for scripts, CI and corpus runs.
//!
//! One module per subcommand; [`run`] dispatches on its name:
//!
//! | Subcommand | Module |
//! |---|---|
//! | `xschem-import` | `xschem_import` |
//! | `virtuoso-export`, `virtuoso-import`, `xschem-export`, `install-skill` | `jobs` |
//! | `pdk-derived` | `pdk_derived` |
//! | `virtuoso-refresh` | `virtuoso_refresh` |
//! | `pdk-models` | `pdk_models` |
//! | `pdk-scan` | `pdk_scan` |
//! | `pdk-map` | `pdk_map` |
//! | `maestro-results` | `maestro_results` |
//! | `layout-export`, `layout-import` | `layout` |
//! | `ciel` | `ciel` |

use std::path::{Path, PathBuf};

use crate::report::Class;
use crate::{spice, tech};

mod ciel;
mod jobs;
mod layout;
mod maestro_results;
mod pdk_derived;
mod pdk_map;
mod pdk_models;
mod pdk_scan;
mod virtuoso_refresh;
mod xschem_import;

use ciel::*;
use jobs::*;
use layout::*;
use maestro_results::*;
use pdk_derived::*;
use pdk_map::*;
use pdk_models::*;
use pdk_scan::*;
use virtuoso_refresh::*;
use xschem_import::*;

fn slash(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

/// Runs the subcommand `args[0]` on the rest of `args` (the command line without the
/// program name) and returns the exit status: 0 ok, 1 an output failed its check, 2 an
/// unknown subcommand, a usage error or an I/O error.
pub fn run(args: &[String]) -> u8 {
    match args.split_first() {
        Some((cmd, rest)) if cmd == "xschem-import" => xschem_import(rest),
        Some((cmd, rest)) if cmd == "pdk-derived" => pdk_derived(rest),
        Some((cmd, rest)) if cmd == "virtuoso-refresh" => virtuoso_refresh(rest),
        Some((cmd, rest)) if cmd == "pdk-models" => pdk_models(rest),
        Some((cmd, rest)) if cmd == "pdk-scan" => pdk_scan(rest),
        Some((cmd, rest)) if cmd == "pdk-map" => pdk_map(rest),
        Some((cmd, rest))
            if cmd == "virtuoso-export"
                || cmd == "virtuoso-import"
                || cmd == "xschem-export"
                || cmd == "install-skill" =>
        {
            job(cmd, rest)
        }
        // Lane L4: results to waves.
        Some((cmd, rest)) if cmd == "maestro-results" => maestro_results(rest),
        // Lane L8: layout through KLayout.
        Some((cmd, rest)) if cmd == "layout-export" || cmd == "layout-import" => layout(cmd, rest),
        // PDK-Manager's kit installs: ciel from PATH, else a pinned private copy.
        Some((cmd, rest)) if cmd == "ciel" => ciel(rest),
        _ => {
            eprintln!("{JOB_USAGE}");
            2
        }
    }
}

fn io_err(p: &Path, e: std::io::Error) -> u8 {
    eprintln!("schemify-interop: {}: {e}", p.display());
    2
}

/// Every Schemify cell file under `dir`, recursively, sorted (`.schemify/` skipped).
fn walk_cells(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut v: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    v.sort();
    for p in v {
        if p.is_dir() {
            if p.file_name().is_some_and(|n| n != ".schemify") {
                walk_cells(&p, out);
            }
        } else if p
            .extension()
            .is_some_and(|x| x == "shprim" || x == "shcomp" || x == "shbench")
        {
            out.push(p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_export_directory_names_every_cell_under_it() {
        let root = std::env::temp_dir().join(format!("sfy-cli-dir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for f in [
            "lib/b.shprim",
            "lib/a.shprim",
            "lib/sub/c.shcomp",
            "lib/notes.txt",
            "top.shbench",
        ] {
            let p = root.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, "").unwrap();
        }
        let args: Vec<String> = [root.to_str().unwrap(), "lib", "top.shbench", "-o", "x"]
            .map(String::from)
            .to_vec();
        let Some((_, crate::job::Command::XschemExport { tops, .. })) =
            job_command("xschem-export", &args)
        else {
            panic!("not an xschem export");
        };
        assert_eq!(
            tops,
            [
                "lib/a.shprim",
                "lib/b.shprim",
                "lib/sub/c.shcomp",
                "top.shbench"
            ]
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
