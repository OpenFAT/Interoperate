//! xschem → Schemify → Virtuoso and back (spec `interop.md` §8, §11; inv_xschem rows 1-3, 16):
//! an xschem library with a sky130-shaped PDK device stand-in (`format` + `lvs_format`, no
//! `spectre_format`, W/L in micrometres), a device with a `spectre_format` and a `devices/`
//! resistor (`core:res`) is imported; `pdk-map` maps the PDK device onto a Virtuoso
//! sky130-like kit in the mock; the bench goes to Virtuoso through the shipped SKILL, comes
//! back through the reverse lookup, and goes out to xschem again on the original symbol.
#![cfg(feature = "cli")]

mod skill;

use std::path::Path;

use interop::design::{Files, load_closed};
use interop::map::Mapping;
use interop::virtuoso::{self, ExportOptions, ImportOptions};
use interop::{Options, pdk_map, translate_library, xschem_out};
use skill::{Interp, V};

const BUILD: &str = include_str!("fixtures/skill/build.il");
const KIT: &str = include_str!("fixtures/skill/pdkmap_kit.il");

const V12: &str = "v {xschem version=3.4.7 file_version=1.2}\n";

/// The xschem inputs: `sys/devices/` (xschem's devices, the test stand-ins), `x/sky130_fd_pr`
/// (`nfet_01v8` shaped like the sky130 library's) and `x/mylib` (`rspec` with a
/// `spectre_format`, bench `tb`).
fn inputs() -> Vec<(String, Vec<u8>)> {
    let dev = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/xschem_export/devices");
    let mut v: Vec<(String, Vec<u8>)> = std::fs::read_dir(&dev)
        .unwrap()
        .flatten()
        .map(|e| {
            (
                format!("sys/devices/{}", e.file_name().to_string_lossy()),
                std::fs::read(e.path()).unwrap(),
            )
        })
        .collect();
    for (p, t) in [
        (
            "x/sky130_fd_pr/nfet_01v8.sym",
            "K {type=nmos\nformat=\"@spiceprefix@name @pinlist sky130_fd_pr__@model L=@L W=@W nf=@nf m=@mult\"\nlvs_format=\"@name @pinlist sky130_fd_pr__@model L=@L W=@W nf=@nf m=@mult\"\ntemplate=\"name=M1 L=0.15 W=1 nf=1 mult=1 model=nfet_01v8 spiceprefix=X\"}\nB 5 17.5 -32.5 22.5 -27.5 {name=D dir=inout}\nB 5 -22.5 -2.5 -17.5 2.5 {name=G dir=in}\nB 5 17.5 27.5 22.5 32.5 {name=S dir=inout}\nB 5 17.5 -2.5 22.5 2.5 {name=B dir=in}\nL 4 0 -30 0 30 {}\n",
        ),
        (
            "x/mylib/rspec.sym",
            "K {type=primitive\nformat=\"@name @pinlist rmod r=@r\"\nspectre_format=\"@name ( @pinlist ) rmod r=@r\"\ntemplate=\"name=R1 r=1k\"}\nB 5 -2.5 -32.5 2.5 -27.5 {name=P dir=inout}\nB 5 -2.5 27.5 2.5 32.5 {name=M dir=inout}\nL 4 0 -30 0 30 {}\n",
        ),
        (
            "x/mylib/tb.sch",
            "C {sky130_fd_pr/nfet_01v8.sym} 0 0 0 0 {name=M1 W=2 L=0.5}\nC {lab_pin.sym} 20 -30 0 0 {name=l1 lab=D}\nC {lab_pin.sym} -20 0 0 0 {name=l2 lab=G}\nC {gnd.sym} 20 30 0 0 {name=l3 lab=GND}\nC {gnd.sym} 20 0 0 0 {name=l4 lab=GND}\nC {rspec.sym} 200 0 0 0 {name=R1 r=2k}\nC {lab_pin.sym} 200 -30 0 0 {name=l5 lab=D}\nC {lab_pin.sym} 200 30 0 0 {name=l6 lab=G}\nC {res.sym} 300 0 0 0 {name=R2 value=5k}\nC {lab_pin.sym} 300 -30 0 0 {name=l7 lab=G}\nC {gnd.sym} 300 30 0 0 {name=l8 lab=GND}\n",
        ),
    ] {
        v.push((p.to_owned(), format!("{V12}{t}").into_bytes()));
    }
    v.sort();
    v
}

fn import_xschem() -> Files {
    let inputs = inputs();
    let refs: Vec<(&str, &[u8])> = inputs
        .iter()
        .map(|(p, b)| (p.as_str(), b.as_slice()))
        .collect();
    let roots = ["x".to_owned(), "sys/devices".to_owned()];
    let opts = Options {
        roots: &roots,
        ..Options::default()
    };
    let (out, report) = translate_library(&refs, &opts);
    report.assert_conservation();
    out.into_iter().collect()
}

fn text(files: &Files, p: &str) -> String {
    String::from_utf8(
        files
            .get(p)
            .unwrap_or_else(|| panic!("no {p}: {:?}", files.keys()))
            .clone(),
    )
    .unwrap()
}

const TB: &str = "mylib/tb.shbench";

/// The mock with the sky130-like Virtuoso kit: `sky130_fd_pr/nfet_01v8`, CDF model
/// `sky130_fd_pr__nfet_01v8`, termOrder (D G S B), CDF params `w l nf m` (metres).
fn kit(libs: &[&str]) -> Interp<skill::mockdb::MockDb> {
    let mut i = skill::virtuoso(libs);
    i.load(BUILD).unwrap();
    i.load(KIT).unwrap();
    i.load("(SfyFixStdlib)").unwrap();
    let ok = i
        .load(
            r#"(SfyFixMapDev "sky130_fd_pr" "nfet_01v8" "sky130_fd_pr__nfet_01v8"
                 (list "D" "G" "S" "B")
                 (list (list "D" "inputOutput" 0 40) (list "G" "input" -40 0)
                       (list "S" "inputOutput" 0 -40) (list "B" "inputOutput" 40 0))
                 (list "w" "l" "nf" "m"))"#,
        )
        .unwrap_or_else(|e| panic!("{e}\n{}", i.log));
    assert!(ok.truthy() || matches!(ok, V::Nil), "{}", i.log);
    i
}

/// Row 1 and 16: without a map the PDK device goes out as a user cell whose missing spectre
/// format is a review row; the device with a `spectre_format` gets its componentName.
#[test]
fn without_a_map_a_cell_with_no_spectre_format_is_reviewed() {
    let files = import_xschem();
    let rspec = text(&files, "mylib/rspec.shprim");
    assert!(
        rspec.contains("format spectre \"{name} ( {pins} ) rmod r={r}\""),
        "{rspec}"
    );
    let d = load_closed(&files, &[TB]).unwrap();
    let x = virtuoso::export(
        &d,
        &[d.find(TB).unwrap()],
        &Mapping::default(),
        &ExportOptions::default(),
    )
    .unwrap();
    let notes = x.notes.text();
    assert!(
        notes.contains(
            "needs_review:format\tsky130_fd_pr/nfet_01v8.shprim\tno format spectre: Spectre netlists it with no master"
        ),
        "{notes}"
    );
    assert!(
        !notes.contains("rspec.shprim\tno format spectre"),
        "{notes}"
    );
    assert!(
        !notes.contains("rspec.shprim\tformat spectre is not"),
        "{notes}"
    );
    assert!(x.sfyx.contains("\"componentName\" \"rmod\""), "{}", x.sfyx);
}

/// Rows 2 and 3: pdk-map over the xschem-imported library and the kit's SfyCdfNames dump
/// gives scaled rows; export re-masters onto the kit cell with W/L in metres; the shipped
/// SKILL loads and dumps it; import's reverse lookup gives the xschem cell back with W/L
/// divided back; and xschem export points at the original symbol again.
#[test]
fn sky130_from_xschem_maps_to_a_virtuoso_kit_and_back() {
    let files = import_xschem();
    // pdk-map: the imported library's cells against the kit's names.
    let mut i = kit(&[
        "sky130_fd_pr",
        "mylib",
        "analogLib",
        "basic",
        "schemify_core",
    ]);
    let ok = i
        .load(r#"(SfyCdfNames "/c/cdf.sfyx" (list "sky130_fd_pr"))"#)
        .unwrap();
    assert!(ok.truthy(), "{}", i.files["/c/cdf.sfyx.log"]);
    let dump = pdk_map::read_dump(&i.files["/c/cdf.sfyx"]).unwrap();
    let scan: Vec<pdk_map::Scanned> = files
        .iter()
        .filter(|(p, _)| p.starts_with("sky130_fd_pr/") && p.ends_with(".shprim"))
        .map(|(p, b)| {
            let stem = p
                .rsplit('/')
                .next()
                .unwrap()
                .strip_suffix(".shprim")
                .unwrap();
            pdk_map::scanned(stem, b).unwrap()
        })
        .collect();
    let (map_text, map_notes) = pdk_map::map("sky130_fd_pr", &scan, &dump).unwrap();
    assert_eq!(
        map_text,
        "map sky130_fd_pr:nfet_01v8 virtuoso sky130_fd_pr nfet_01v8\n  pin D D\n  pin G G\n  pin S S\n  pin B B\n  param L l scale 1e-6\n  param W w scale 1e-6\n  param nf nf\n  drop model\n",
        "{}",
        map_notes.text()
    );
    let map = Mapping::parse(&map_text).unwrap();

    // Export: re-mastered, scaled, no review of the mapped cell.
    let d = load_closed(&files, &[TB]).unwrap();
    let x = virtuoso::export(&d, &[d.find(TB).unwrap()], &map, &ExportOptions::default()).unwrap();
    let notes = x.notes.text();
    assert!(
        !notes.contains("nfet_01v8.shprim\tno format spectre"),
        "{notes}"
    );
    assert!(
        notes.contains("remapped\tmylib/tb.shbench\tM1: nfet_01v8 → sky130_fd_pr/nfet_01v8"),
        "{notes}"
    );
    assert!(
        x.sfyx
            .contains("\"M1\" \"sky130_fd_pr\" \"nfet_01v8\" \"symbol\""),
        "{}",
        x.sfyx
    );
    for v in ["\"w\" \"string\" \"2e-6\"", "\"l\" \"string\" \"5e-7\""] {
        assert!(x.sfyx.contains(v), "{v} in\n{}", x.sfyx);
    }

    // The shipped SKILL loads it into the mock and dumps the design library back.
    i.files.insert("/e2e/in.sfyx".into(), x.sfyx.clone());
    let ok = i
        .load(r#"(SfyImport "/e2e/in.sfyx")"#)
        .unwrap_or_else(|e| panic!("SfyImport: {e}\n{}", i.log));
    let log = i.files.get("/e2e/in.sfyx.log").cloned().unwrap_or_default();
    assert!(matches!(ok, V::T), "SfyImport:\n{log}\n{}", i.log);
    let ok = i
        .load(r#"(SfyExport "/e2e/mylib.sfyx" "mylib" nil)"#)
        .unwrap_or_else(|e| panic!("SfyExport: {e}\n{}", i.log));
    assert!(matches!(ok, V::T), "{}", i.log);
    let back = i.files["/e2e/mylib.sfyx"].clone();
    let imp = virtuoso::import(&back, &files, &map, &ImportOptions::default())
        .unwrap_or_else(|e| panic!("{e:?}\n{back}"));
    let mut files2 = files.clone();
    files2.extend(imp.files);
    let tb = text(&files2, TB);
    assert!(
        tb.contains("sky130_fd_pr:nfet_01v8\n  at 0 0 R0\n  name M1\n  L 0.5\n  W 2\n"),
        "{tb}"
    );

    // And out to xschem on the original symbol, with xschem's units.
    let d = load_closed(&files2, &[TB]).unwrap();
    let xs = xschem_out::export(&d, &[d.find(TB).unwrap()]).unwrap();
    let sch = xs
        .files
        .iter()
        .find(|f| f.0 == "mylib/tb.sch")
        .map(|f| String::from_utf8_lossy(&f.1).into_owned())
        .unwrap();
    let line = sch
        .lines()
        .find(|l| l.contains("name=M1"))
        .unwrap_or_else(|| panic!("{sch}"));
    assert!(
        line.starts_with("C {sky130_fd_pr/nfet_01v8.sym}")
            && line.contains("W=2")
            && line.contains("L=0.5"),
        "{sch}\n{}\n{}",
        xs.notes.text(),
        text(&files2, "sky130_fd_pr/nfet_01v8.shprim")
    );
    // Unedited through Virtuoso: rspec is still the original xschem symbol.
    assert!(
        !xs.notes.text().contains("edited since"),
        "{}",
        xs.notes.text()
    );
    assert!(!xs.files.iter().any(|f| f.0.starts_with("sky130_fd_pr/")));

    // Virtuoso → Schemify → xschem without the map: the kit cell is a mirror with no spice
    // format, and the xschem export says it will not netlist it.
    let manifest: Files = files
        .iter()
        .filter(|(p, _)| *p == "schemify.project")
        .map(|(p, b)| (p.clone(), b.clone()))
        .collect();
    let imp = virtuoso::import(
        &back,
        &manifest,
        &Mapping::default(),
        &ImportOptions::default(),
    )
    .unwrap();
    let mut files3 = manifest.clone();
    files3.extend(imp.files);
    let mirror = text(&files3, "sky130_fd_pr/nfet_01v8.shprim");
    assert!(
        mirror.contains("provenance \"virtuoso sky130_fd_pr/nfet_01v8\"")
            && !mirror.contains("format spice"),
        "{mirror}"
    );
    let d = load_closed(&files3, &[TB]).unwrap();
    let xs = xschem_out::export(&d, &[d.find(TB).unwrap()]).unwrap();
    let sym = xs
        .files
        .iter()
        .find(|f| f.0 == "sky130_fd_pr/nfet_01v8.sym")
        .map(|f| String::from_utf8_lossy(&f.1).into_owned())
        .unwrap_or_else(|| panic!("{:?}", xs.files.iter().map(|f| &f.0).collect::<Vec<_>>()));
    // The mirror netlists as Schemify's default call of the kit cell (netlist §7.6.5), a
    // subckt ngspice must be given; the X letter is a review row.
    assert!(sym.contains("format=\"@name @pinlist @symname\""), "{sym}");
    assert!(
        xs.notes
            .text()
            .contains("needs_review:name\tmylib/tb.shbench\tM1 written XM1"),
        "{}",
        xs.notes.text()
    );
}
