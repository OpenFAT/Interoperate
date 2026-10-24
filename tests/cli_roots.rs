//! `schemify-interop xschem-import --root`: a search-path element contributes only what the
//! converted inputs reference from it, transitively (`cli::pull_referenced`), so a design
//! imported against xschem's `xschem_library` and a PDK's `libs.tech/xschem` gets the cells
//! it places and none of the libraries' demo benches.
#![cfg(feature = "cli")]

use std::path::Path;

fn write(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

const SUBCKT: &str = "v {xschem version=3.4.7 file_version=1.2}\nK {type=subcircuit\nformat=\"@name @pinlist @symname\"\ntemplate=\"name=x1\"}\nB 5 -42.5 -2.5 -37.5 2.5 {name=A dir=in}\nB 5 37.5 -2.5 42.5 2.5 {name=Z dir=out}\n";
const PRIM: &str = "v {xschem version=3.4.7 file_version=1.2}\nK {type=primitive\nformat=\"@name @pinlist leafmodel\"\ntemplate=\"name=x1\"}\nB 5 -42.5 -2.5 -37.5 2.5 {name=A dir=in}\nB 5 37.5 -2.5 42.5 2.5 {name=Z dir=out}\n";

#[test]
fn root_libraries_contribute_only_what_the_design_references() {
    let dir = std::env::temp_dir().join(format!("interop-cli-roots-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let (pdk, design, out) = (dir.join("pdk"), dir.join("design"), dir.join("out"));
    // The "PDK": a cell with a schematic that places a leaf, an unused cell, a demo bench.
    write(&pdk, "lib/cell.sym", SUBCKT);
    write(
        &pdk,
        "lib/cell.sch",
        "v {xschem version=3.4.7 file_version=1.2}\nC {lib/leaf.sym} 0 0 0 0 {name=x1}\n",
    );
    write(&pdk, "lib/leaf.sym", PRIM);
    write(&pdk, "lib/unused.sym", PRIM);
    write(
        &pdk,
        "lib/demo_tb.sch",
        "v {xschem version=3.4.7 file_version=1.2}\nC {lib/unused.sym} 0 0 0 0 {name=x1}\n",
    );
    write(
        &design,
        "tb.sch",
        "v {xschem version=3.4.7 file_version=1.2}\nC {lib/cell.sym} 0 0 0 0 {name=x1}\nC {lib/missing.sym} 0 100 0 0 {name=x2}\n",
    );
    let s = |p: &Path| p.to_string_lossy().into_owned();
    let args: Vec<String> = [
        "xschem-import",
        &s(&design),
        "--root",
        &s(&pdk),
        "-o",
        &s(&out),
    ]
    .iter()
    .map(|a| (*a).to_owned())
    .collect();
    let code = interop::cli::run(&args);
    assert!(code <= 1, "exit {code}");
    let exists = |rel: &str| out.join(rel).is_file();
    assert!(exists("design/tb.shbench"));
    assert!(
        exists("lib/cell.shcomp"),
        "the placed cell, with its schematic"
    );
    assert!(
        exists("lib/leaf.shprim"),
        "what that schematic places, transitively"
    );
    assert!(
        !exists("lib/unused.shprim"),
        "an unreferenced cell stays out"
    );
    assert!(
        !exists("lib/demo_tb.shbench"),
        "a library's bench stays out"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
