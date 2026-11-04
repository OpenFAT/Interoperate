//! `schemify-interop ciel <args>…`: ciel (the open_pdks version manager PDK-Manager
//! installs kits with), run with `<args>` and its exit status passed back. A `ciel` on
//! `PATH` wins; otherwise the first call creates a private virtual environment pinned to
//! [`CIEL_VERSION`] under Schemify's data directory (`python3 -m venv`, then `pip install`)
//! and every call runs ciel from there. The install's output goes to stderr, so ciel's
//! stdout (`ls-remote`'s version list) is all a caller reads.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The ciel release the private environment installs.
pub(super) const CIEL_VERSION: &str = "3.2.0";

/// Runs ciel with `args`; returns its exit status (2 when it cannot be found or installed).
pub(super) fn ciel(args: &[String]) -> u8 {
    let program = match locate(std::env::var_os("PATH").as_deref(), tools_dir().as_deref()) {
        Located::Ready(p) => p,
        Located::Install(venv) => match install(&venv) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("schemify-interop ciel: {e}");
                return 2;
            }
        },
        Located::Nowhere(e) => {
            eprintln!("schemify-interop ciel: {e}");
            return 2;
        }
    };
    match Command::new(&program).args(args).status() {
        Ok(s) => s.code().map_or(2, |c| u8::try_from(c).unwrap_or(2)),
        Err(e) => {
            eprintln!("schemify-interop ciel: {}: {e}", program.display());
            2
        }
    }
}

/// Where ciel comes from.
#[derive(Debug, PartialEq)]
enum Located {
    /// A runnable ciel: on `PATH`, or in an environment an earlier call installed.
    Ready(PathBuf),
    /// No ciel yet: install the private environment at this directory.
    Install(PathBuf),
    /// No ciel and no data directory to install one in.
    Nowhere(String),
}

/// `ciel` on `path` (the `PATH` value), else the private environment under `tools`.
fn locate(path: Option<&std::ffi::OsStr>, tools: Option<&Path>) -> Located {
    let exe = if cfg!(windows) { "ciel.exe" } else { "ciel" };
    if let Some(found) = path
        .into_iter()
        .flat_map(std::env::split_paths)
        .map(|d| d.join(exe))
        .find(|p| p.is_file())
    {
        return Located::Ready(found);
    }
    let Some(tools) = tools else {
        return Located::Nowhere(
            "ciel is not on PATH, and neither XDG_DATA_HOME nor HOME (LOCALAPPDATA on Windows) \
             names a directory to install it in"
                .to_owned(),
        );
    };
    let venv = tools.join(format!("ciel-{CIEL_VERSION}"));
    let bin = venv_bin(&venv, exe);
    if bin.is_file() && venv.join(DONE).is_file() {
        Located::Ready(bin)
    } else {
        Located::Install(venv)
    }
}

/// Creates the environment at `venv` and installs ciel in it; returns the ciel program.
/// A virtual environment cannot move (its scripts name their own path), so it is built in
/// place and counts as installed only once [`DONE`] is written, last: an interrupted
/// install is rebuilt by the next call.
// ponytail: two first calls at once both build into `venv`; add a lock file if PDK-Manager
// ever runs ciel steps in parallel.
fn install(venv: &Path) -> Result<PathBuf, String> {
    let exe = if cfg!(windows) { "ciel.exe" } else { "ciel" };
    let _ = std::fs::remove_dir_all(venv);
    std::fs::create_dir_all(venv).map_err(|e| format!("{}: {e}", venv.display()))?;
    eprintln!(
        "schemify-interop: installing ciel {CIEL_VERSION} into {} (first use)",
        venv.display()
    );
    let python = ["python3", "python"]
        .into_iter()
        .find(|p| run(Command::new(p).arg("--version")).is_ok())
        .ok_or("ciel is not on PATH and no python3 is there to install it with")?;
    run(Command::new(python).args(["-m", "venv"]).arg(venv))?;
    let py = venv_bin(
        venv,
        if cfg!(windows) {
            "python.exe"
        } else {
            "python"
        },
    );
    let pin = format!("ciel=={CIEL_VERSION}");
    let pip = [
        "-m",
        "pip",
        "install",
        "--quiet",
        "--disable-pip-version-check",
        &pin,
    ];
    run(Command::new(&py).args(pip)).map_err(|e| format!("pip install {pin}: {e}"))?;
    let bin = venv_bin(venv, exe);
    if !bin.is_file() {
        return Err(format!("{}: no ciel after the install", bin.display()));
    }
    std::fs::write(venv.join(DONE), CIEL_VERSION)
        .map_err(|e| format!("{}: {e}", venv.display()))?;
    Ok(bin)
}

/// Written into the environment once ciel is installed in it.
const DONE: &str = "schemify-installed";

/// Runs `c` with its output on stderr; an error unless it exits 0.
fn run(c: &mut Command) -> Result<(), String> {
    let status = c
        .stdin(Stdio::null())
        .stdout(std::io::stderr())
        .status()
        .map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("exit {}", status.code().unwrap_or(-1)))
    }
}

/// Program `exe` of the virtual environment at `venv`.
fn venv_bin(venv: &Path, exe: &str) -> PathBuf {
    venv.join(if cfg!(windows) { "Scripts" } else { "bin" })
        .join(exe)
}

/// `<Schemify data directory>/tools`: `$XDG_DATA_HOME/schemify`, else
/// `$HOME/.local/share/schemify`, on Windows `%LOCALAPPDATA%\schemify-data` (Schemify's
/// config spec §1; `SCHEMIFY_APPNAME` is not followed).
fn tools_dir() -> Option<PathBuf> {
    let var = |k: &str| {
        std::env::var_os(k)
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
    };
    let data = if cfg!(windows) {
        var("LOCALAPPDATA").map(|d| d.join("schemify-data"))
    } else {
        var("XDG_DATA_HOME")
            .map(|d| d.join("schemify"))
            .or_else(|| var("HOME").map(|h| h.join(".local/share/schemify")))
    };
    data.map(|d| d.join("tools"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("sfy-ciel-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A ciel on PATH wins over the private environment; with neither, the environment is
    /// installed at `<tools>/ciel-<pin>`; one that exists is used as is.
    #[test]
    fn path_first_then_the_pinned_environment() {
        let root = scratch("locate");
        let exe = if cfg!(windows) { "ciel.exe" } else { "ciel" };
        let (on_path, tools) = (root.join("bin"), root.join("tools"));
        std::fs::create_dir_all(&on_path).unwrap();
        let venv = tools.join(format!("ciel-{CIEL_VERSION}"));
        let empty = std::env::join_paths([root.join("nothing")]).unwrap();
        assert_eq!(
            locate(Some(&empty), Some(&tools)),
            Located::Install(venv.clone())
        );
        assert!(matches!(locate(Some(&empty), None), Located::Nowhere(_)));
        let installed = venv_bin(&venv, exe);
        std::fs::create_dir_all(installed.parent().unwrap()).unwrap();
        std::fs::write(&installed, "").unwrap();
        assert_eq!(
            locate(Some(&empty), Some(&tools)),
            Located::Install(venv.clone()),
            "an install that never finished is redone"
        );
        std::fs::write(venv.join(DONE), CIEL_VERSION).unwrap();
        assert_eq!(
            locate(Some(&empty), Some(&tools)),
            Located::Ready(installed)
        );
        std::fs::write(on_path.join(exe), "").unwrap();
        let path = std::env::join_paths([root.join("nothing"), on_path.clone()]).unwrap();
        assert_eq!(
            locate(Some(&path), Some(&tools)),
            Located::Ready(on_path.join(exe))
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The real install (network, python3): ciel runs from the new environment. Run with
    /// `cargo test --features cli -- --ignored ciel_installs`.
    #[test]
    #[ignore = "downloads ciel from PyPI"]
    fn ciel_installs_and_runs() {
        let root = scratch("install");
        let venv = root.join(format!("ciel-{CIEL_VERSION}"));
        let bin = install(&venv).unwrap();
        let out = Command::new(bin).arg("--version").output().unwrap();
        assert!(out.status.success());
        let _ = std::fs::remove_dir_all(&root);
    }
}
