//! `pdk-derived` (spec `interop.md` §8.3): a characterization's derived params → the
//! project's mirror cells.

use super::*;

/// `pdk-derived <project-dir> <char.sfyx>` (spec `interop.md` §8.3): marks the params a
/// characterization found derived on the project's mirror cells (`vderived`, header
/// `vinputs`). Prints what it did; 0 done, 1 nothing marked, 2 usage or I/O error.
pub(super) fn pdk_derived(args: &[String]) -> u8 {
    use crate::virtuoso::characterize;
    let [project, char_file] = args else {
        eprintln!("usage: schemify-interop pdk-derived <project-dir> <char.sfyx>");
        return 2;
    };
    let project = Path::new(project);
    let text = match std::fs::read_to_string(char_file) {
        Ok(t) => t,
        Err(e) => return io_err(Path::new(char_file), e),
    };
    let devs = match characterize::read(&text) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("schemify-interop: {char_file}: {e}");
            return 2;
        }
    };
    let manifest = project.join("schemify.project");
    let libs = match std::fs::read(&manifest) {
        Ok(b) => crate::design::manifest_libs(&b).unwrap_or_default(),
        Err(e) => return io_err(&manifest, e),
    };
    let mut marked = 0;
    for d in &devs {
        let dir = libs.get(&d.lib).map_or(d.lib.as_str(), String::as_str);
        let cell = project.join(dir).join(format!("{}.shprim", d.cell));
        let Ok(bytes) = std::fs::read(&cell) else {
            println!(
                "{}/{}: no mirror cell at {}: import the PDK first",
                d.lib,
                d.cell,
                cell.display()
            );
            continue;
        };
        let out = match characterize::mark_derived(&bytes, d) {
            Ok(o) => o,
            Err(e) => {
                eprintln!("schemify-interop: {}: {e}", cell.display());
                return 2;
            }
        };
        if let Err(e) = std::fs::write(&cell, out) {
            return io_err(&cell, e);
        }
        marked += 1;
        println!(
            "{}/{}: derived ({}) from inputs ({})",
            d.lib,
            d.cell,
            d.derived().join(" "),
            d.inputs.join(" ")
        );
    }
    if marked == 0 { 1 } else { 0 }
}
