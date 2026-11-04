//! `layout-export`, `layout-import`: layout through KLayout (`crate::klayout`, gap H).
//!
//! - `layout-export <project-dir> <bundle-dir>`: the bundle `SfyLayoutExport` wrote →
//!   the project's `<lib dir>/<cell>.<view>.gds`, `<tech>.lyp`, `<tech>.lyt`, the stash and
//!   the cell file's `vlayout` header attribute.
//! - `layout-import <project-dir> <lib>/<cell>[/<view>] -o <bundle-dir>`: the project's
//!   GDS and stash → an import bundle for `SfyLayoutImport` (restore or strmin) with its
//!   report (`interop.report.md`, `.txt`). For a strmin, KLayout (`KLAYOUT`, else `klayout`
//!   on PATH) compares the GDS with the exported one, and the report lists what changed.
//!
//! The library directory is the manifest's `lib <lib> path <dir>`, else `<lib>`. Bundle
//! paths are taken as given, not relative to the project.

use super::*;
use crate::klayout::{self, Files, Mode};

const USAGE: &str = "\
usage: schemify-interop layout-export <project-dir> <bundle-dir>
       schemify-interop layout-import <project-dir> <lib>/<cell>[/<view>] -o <bundle-dir>";

/// Runs `cmd`: 0 written, 1 refused (problems printed), 2 a usage or I/O error.
pub(super) fn layout(cmd: &str, args: &[String]) -> u8 {
    let r = match (cmd, args) {
        ("layout-export", [project, bundle]) => export(Path::new(project), Path::new(bundle)),
        ("layout-import", [project, view, o, out]) if o == "-o" => {
            import(Path::new(project), view, Path::new(out))
        }
        _ => {
            eprintln!("{USAGE}");
            return 2;
        }
    };
    match r {
        Ok(summary) => {
            println!("{summary}");
            0
        }
        Err((code, msg)) => {
            eprintln!("schemify-interop: {cmd}: {msg}");
            code
        }
    }
}

type Failed = (u8, String);

fn io(p: &Path, e: std::io::Error) -> Failed {
    (2, format!("{}: {e}", p.display()))
}

/// Every file under `dir`, by path relative to it (`/`-separated).
fn read_tree(dir: &Path) -> Result<Files, Failed> {
    fn walk(root: &Path, dir: &Path, out: &mut Files) -> Result<(), Failed> {
        for e in std::fs::read_dir(dir).map_err(|e| io(dir, e))? {
            let p = e.map_err(|e| io(dir, e))?.path();
            if p.is_dir() {
                walk(root, &p, out)?;
            } else {
                let b = std::fs::read(&p).map_err(|e| io(&p, e))?;
                out.insert(slash(p.strip_prefix(root).unwrap_or(&p)), b);
            }
        }
        Ok(())
    }
    let mut out = Files::new();
    walk(dir, dir, &mut out)?;
    Ok(out)
}

fn write_tree(dir: &Path, files: &Files) -> Result<(), Failed> {
    for (p, b) in files {
        let path = dir.join(p);
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d).map_err(|e| io(d, e))?;
        }
        std::fs::write(&path, b).map_err(|e| io(&path, e))?;
    }
    Ok(())
}

/// The project directory of Virtuoso library `lib`.
fn lib_dir(project: &Path, lib: &str) -> Result<String, Failed> {
    let m = project.join("schemify.project");
    let Ok(text) = std::fs::read(&m) else {
        return Ok(lib.to_owned());
    };
    let libs = crate::design::manifest_libs(&text).map_err(|e| (1, e.join("; ")))?;
    Ok(libs.get(lib).cloned().unwrap_or_else(|| lib.to_owned()))
}

fn export(project: &Path, bundle: &Path) -> Result<String, Failed> {
    let files = read_tree(bundle)?;
    let info = files
        .get("layout.sfyx")
        .ok_or_else(|| (1, format!("{}: no layout.sfyx", bundle.display())))
        .and_then(|b| klayout::read_info(&String::from_utf8_lossy(b)).map_err(|e| (1, e)))?;
    let dir = lib_dir(project, &info.lib)?;
    let cell = ["shcomp", "shprim", "shbench"]
        .iter()
        .map(|x| format!("{dir}/{}.{x}", info.cell))
        .find(|p| project.join(p).is_file());
    let bytes = match &cell {
        Some(p) => Some(std::fs::read(project.join(p)).map_err(|e| io(&project.join(p), e))?),
        None => None,
    };
    let cf = cell.as_deref().zip(bytes.as_deref());
    let out = klayout::export(&files, &dir, cf).map_err(|e| (1, e))?;
    write_tree(project, &out)?;
    Ok(format!(
        "layout-export: {}/{}/{} -> {dir}/{}.{}.gds ({} files{})",
        info.lib,
        info.cell,
        info.view,
        info.cell,
        info.view,
        out.len(),
        if cf.is_some() {
            ", vlayout on the cell file"
        } else {
            ", no cell file"
        }
    ))
}

fn import(project: &Path, view: &str, out: &Path) -> Result<String, Failed> {
    let (lib, cell, view) = match view.split('/').collect::<Vec<_>>()[..] {
        [l, c] => (l, c, "layout"),
        [l, c, v] => (l, c, v),
        _ => return Err((2, format!("{view}: not <lib>/<cell>[/<view>]\n{USAGE}"))),
    };
    let stash = read_tree(&project.join(klayout::stash_dir(lib, cell, view)))?;
    let gds_path = project.join(format!("{}/{cell}.{view}.gds", lib_dir(project, lib)?));
    let gds = std::fs::read(&gds_path).map_err(|e| io(&gds_path, e))?;
    let mut imp = klayout::import(&stash, &gds).map_err(|e| (1, e))?;
    let exported = project.join(format!(
        "{}/{cell}.{view}.gds",
        klayout::stash_dir(lib, cell, view)
    ));
    if imp.mode == Mode::Strmin && exported.is_file() {
        let what = format!("{lib}/{cell}");
        match diff(&exported, &gds_path) {
            Ok(rows) => rows
                .iter()
                .for_each(|r| imp.notes.add("updated", &what, format!("in the GDS: {r}"))),
            Err(e) => imp.notes.review(
                "layout",
                &what,
                format!("the GDS was not compared with the exported one: {e}"),
            ),
        }
    }
    // A bundle is written whole: a stale file of an earlier one must not be applied.
    if out.join("layout.sfyx").is_file() {
        std::fs::remove_dir_all(out).map_err(|e| io(out, e))?;
    } else if std::fs::read_dir(out).is_ok_and(|mut d| d.next().is_some()) {
        return Err((
            2,
            format!("{}: not empty and not a layout bundle", out.display()),
        ));
    }
    let mut files = imp.files;
    files.insert(
        "interop.report.md".into(),
        imp.notes.markdown("layout-import").into_bytes(),
    );
    files.insert("interop.report.txt".into(), imp.notes.text().into_bytes());
    write_tree(out, &files)?;
    Ok(match imp.mode {
        Mode::Restore => {
            format!("layout-import: {lib}/{cell}/{view}: GDS unchanged, restore the OA files")
        }
        Mode::Strmin => format!(
            "layout-import: {lib}/{cell}/{view}: GDS edited, strmin ({} report rows)\n{}",
            imp.notes.rows.len(),
            imp.notes.text().trim_end()
        ),
    })
}

/// KLayout's `LayoutDiff` of two GDS files, one row per cell, layer and shape kind.
const DIFF_PY: &str = r#"import pya
la = pya.Layout(); la.read(a)
lb = pya.Layout(); lb.read(b)
d = pya.LayoutDiff()
n = {}
at = ["", ""]
def count(side, kind):
    n.setdefault((at[0], at[1], kind), [0, 0])[side] += 1
def cell(ca, cb):
    at[0] = (ca or cb).name
    at[1] = ""
def layer(info, ia, ib):
    at[1] = str(info)
d.on_begin_cell = cell
d.on_begin_layer = layer
for kind in ["polygon", "path", "box", "edge", "text", "instance"]:
    setattr(d, "on_%s_in_a_only" % kind, lambda *x, kind=kind: count(0, kind))
    setattr(d, "on_%s_in_b_only" % kind, lambda *x, kind=kind: count(1, kind))
d.on_cell_in_a_only = lambda c: print("row cell %s removed" % c.name)
d.on_cell_in_b_only = lambda c: print("row cell %s added" % c.name)
d.compare(la, lb, pya.LayoutDiff.Verbose | pya.LayoutDiff.NoTextOrientation)
for (c, l, kind), (rm, add) in sorted(n.items()):
    print("row cell %s%s: %s %d removed, %d added" % (c, " layer " + l if l else "", kind, rm, add))
print("done")
"#;

/// What differs between GDS files `a` and `b`, by KLayout.
fn diff(a: &Path, b: &Path) -> Result<Vec<String>, String> {
    let k = std::env::var("KLAYOUT").unwrap_or_else(|_| "klayout".into());
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let py = std::env::temp_dir().join(format!("sfy-layout-diff-{}-{n}.py", std::process::id()));
    std::fs::write(&py, DIFF_PY).map_err(|e| format!("{}: {e}", py.display()))?;
    let out = std::process::Command::new(&k)
        .env("QT_QPA_PLATFORM", "offscreen")
        .arg("-b")
        .arg("-r")
        .arg(&py)
        .args([
            "-rd",
            &format!("a={}", a.display()),
            "-rd",
            &format!("b={}", b.display()),
        ])
        .output();
    let _ = std::fs::remove_file(&py);
    let out = out.map_err(|e| format!("{k}: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout);
    if !out.status.success() || !text.lines().any(|l| l == "done") {
        return Err(format!(
            "{k} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(text
        .lines()
        .filter_map(|l| l.strip_prefix("row "))
        .map(str::to_owned)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_then_import_round_trips_through_the_file_system() {
        let root = std::env::temp_dir().join(format!("sfy-cli-layout-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (proj, bundle, back) = (root.join("p"), root.join("b"), root.join("o"));
        let mut b = Files::new();
        b.insert(
            "layout.sfyx".into(),
            b"(\"sfyx\" 1 \"t\")\n(\"layout\" \"L\" \"c\" \"layout\" \"T\")\n(\"dbu\" \"1000.0\")\n(\"end\")\n".to_vec(),
        );
        b.insert("layermap".into(), b"y0 drawing 207 0\n".to_vec());
        b.insert(
            "display.drf".into(),
            b"drDefineDisplay(\n ( display )\n)\n".to_vec(),
        );
        b.insert("layout.gds".into(), b"gds".to_vec());
        b.insert("oa/layout.oa".into(), b"oa".to_vec());
        write_tree(&bundle, &b).unwrap();
        std::fs::create_dir_all(proj.join("L")).unwrap();
        std::fs::write(proj.join("L/c.shbench"), "schemify 1 shbench\n").unwrap();
        assert_eq!(layout("layout-export", &[slash(&proj), slash(&bundle)]), 0);
        let cell = std::fs::read_to_string(proj.join("L/c.shbench")).unwrap();
        assert!(
            cell.contains(&format!("vlayout layout {}", klayout::sha256(b"gds"))),
            "{cell}"
        );
        let args = |v: &str| [slash(&proj), v.to_owned(), "-o".into(), slash(&back)];
        assert_eq!(layout("layout-import", &args("L/c")), 0);
        assert_eq!(std::fs::read(back.join("oa/layout.oa")).unwrap(), b"oa");
        std::fs::write(proj.join("L/c.layout.gds"), b"edited").unwrap();
        assert_eq!(layout("layout-import", &args("L/c/layout")), 0);
        assert!(
            !back.join("oa").exists(),
            "the earlier restore bundle is gone"
        );
        assert_eq!(std::fs::read(back.join("layout.gds")).unwrap(), b"edited");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn klayout_lists_what_differs_between_two_streams() {
        let d = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cadence/layout");
        if !d.join("ts1.gds").is_file() {
            return; // local-only Cadence fixtures
        }
        let Ok(same) = diff(&d.join("det1.gds"), &d.join("back_sfyprobe_in.gds")) else {
            return; // no KLayout
        };
        // strmin then strmout: other bytes, the same geometry (probe `stream`).
        assert_eq!(same, Vec::<String>::new());
        // Without -ignoreCurrentTimestamp the PCell sub-master gets another name.
        let ts = diff(&d.join("det1.gds"), &d.join("ts1.gds")).unwrap();
        assert_eq!(ts[0], "cell pc_CDNS_0 removed");
        assert!(
            ts.contains(&"cell lay: instance 1 removed, 1 added".to_owned()),
            "{ts:?}"
        );
        assert!(diff(&d.join("cmp.txt"), &d.join("ts1.gds")).is_err());
    }
}
