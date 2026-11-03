//! `schemify-interop pdk-models` on the made-up pdk-scan kits: every reached file converted,
//! the SpiceRack store (`rack`) a lossless stop between dialects, and root confinement.
#![cfg(feature = "cli")]

use std::path::{Path, PathBuf};

use interop::pdk_models;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/pdk_scan");

fn run(root: &Path, entry: &str, from: &str, to: &str, out: &Path) -> (String, u8) {
    let args = [
        "--root",
        root.to_str().unwrap(),
        "--entry",
        entry,
        "--from",
        from,
        "--to",
        to,
        "-o",
        out.to_str().unwrap(),
    ];
    pdk_models::run(&args.map(String::from))
}

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("pdk_models-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn read(dir: &Path, rel: &str) -> String {
    std::fs::read_to_string(dir.join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

#[test]
fn a_spectre_kit_goes_through_the_store_unchanged() {
    let kit = PathBuf::from(FIXTURES).join("spectre");
    let (direct, store, back) = (scratch("direct"), scratch("store"), scratch("back"));
    let files = [
        "top.scs",
        "devices.scs",
        "corners.lib.scs",
        "tt_devices.scs",
        "va/varactor.va",
    ];
    // Every section is followed: ff's include is not in the kit, and is reported.
    let want = "converted 5 file(s)\nmissing ff_missing.scs\nend\n";

    let (out, code) = run(&kit, "top.scs", "spectre", "spectre", &direct);
    assert_eq!((out.as_str(), code), (want, 1));
    let (out, code) = run(&kit, "top.scs", "spectre", "rack", &store);
    assert_eq!((out.as_str(), code), (want, 1));
    // The store is IR, and following it reaches the same files.
    let ir: serde_json::Value = serde_json::from_str(&read(&store, "corners.lib.scs")).unwrap();
    assert_eq!(ir["sections"].as_array().map(Vec::len), Some(2));
    assert_eq!(
        run(&store, "top.scs", "rack", "spectre", &back),
        (want.into(), 1)
    );
    for f in files {
        assert_eq!(read(&direct, f), read(&back, f), "{f}");
    }
    // Two entries reaching the same files convert each once.
    let twice = scratch("twice");
    let args = [
        "--root",
        store.to_str().unwrap(),
        "--entry",
        "top.scs",
        "--entry",
        "devices.scs",
        "--from",
        "rack",
        "--to",
        "spectre",
        "-o",
        twice.to_str().unwrap(),
    ];
    assert_eq!(pdk_models::run(&args.map(String::from)), (want.into(), 1));
    let _ = std::fs::remove_dir_all(twice);

    // And to SPICE: the devices survive.
    let spice = scratch("spice");
    assert_eq!(run(&store, "devices.scs", "rack", "spice", &spice).1, 0);
    let dev = read(&spice, "devices.scs").to_ascii_lowercase();
    for want in [
        ".subckt nfet_lvt d g s b",
        ".subckt rhigh p n",
        ".model nch_core",
    ] {
        assert!(dev.contains(want), "{want} in\n{dev}");
    }
    // ponytail: the whole kit does not leave Spectre yet, and says why (never a silent drop):
    // its `ahdl_include` needs OpenVAF for ngspice (SpiceRack's `backend` feature), and a
    // `simulator lang=spice` region is still raw Spectre text. Flip when both are handled.
    let (out, code) = run(&store, "top.scs", "rack", "spice", &spice);
    assert_eq!(code, 2, "{out}");
    assert!(out.contains("not written"), "{out}");
    assert!(
        !spice.join("top.scs").exists(),
        "nothing written on failure"
    );
    for d in [direct, store, back, spice] {
        let _ = std::fs::remove_dir_all(d);
    }
}

#[test]
fn an_include_leaving_the_root_writes_nothing() {
    // `.lib escape` in top.lib includes ../../outside.inc; every section is followed.
    let kit = PathBuf::from(FIXTURES).join("kit");
    let out_dir = scratch("escape");
    let (out, code) = run(&kit, "models/top.lib", "hspice", "rack", &out_dir);
    assert_eq!(code, 2, "{out}");
    assert!(out.contains("outside the root, refused"), "{out}");
    assert!(!out_dir.exists());
}

#[test]
fn bad_arguments_are_refused() {
    let kit = PathBuf::from(FIXTURES).join("spectre");
    let out_dir = scratch("bad");
    assert_eq!(run(&kit, "top.scs", "spectre", "eldo", &out_dir).1, 2);
    assert_eq!(pdk_models::run(&["--root".into()]).1, 2);
    assert!(!out_dir.exists());
}
