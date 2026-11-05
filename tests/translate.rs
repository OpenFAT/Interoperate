//! Translation oracles on a small hand-written xschem library (docs/dev/testing.md `import`
//! row): every output loads in `doc` and round-trips, every expression compiles
//! (`check_output`), count conservation (asserted inside `translate_library`),
//! byte-identical re-import, and no panic on mutated inputs.

use interop::report::{Class, Drop};
use interop::template::Reason;
use interop::translate::{ORIENT_TABLE, orient, place};
use interop::{Options, check_output, translate_library};
use proptest::prelude::*;
use schemify_format::{Orient, parse};

/// A minimal library in xschem's file format, written for these tests: a `devices`
/// directory (mapped to `core` by name and pin geometry) and a design library.
fn fixture() -> Vec<(String, String)> {
    let f = |p: &str, s: &str| (p.to_owned(), s.to_owned());
    vec![
        f(
            "x/devices/res.sym",
            "v {xschem version=3.4.7 file_version=1.2}\nK {type=resistor\nformat=\"@name @pinlist @value m=@m\"\ntemplate=\"name=R1 value=1k m=1 footprint=0603\"}\nB 5 -2.5 -32.5 2.5 -27.5 {name=P dir=inout}\nB 5 -2.5 27.5 2.5 32.5 {name=M dir=inout}\nL 4 0 -30 0 30 {}\n",
        ),
        f(
            "x/devices/nmos4.sym",
            "v {xschem version=3.4.7 file_version=1.2}\nK {type=nmos\nformat=\"@spiceprefix@name @pinlist @model w=@w l=@l @extra m=@m\"\ntemplate=\"name=M1 model=nmos w=5u l=0.18u m=1\"}\nB 5 17.5 -32.5 22.5 -27.5 {name=d dir=inout}\nB 5 -22.5 -2.5 -17.5 2.5 {name=g dir=in}\nB 5 17.5 27.5 22.5 32.5 {name=s dir=inout}\nB 5 17.5 -2.5 22.5 2.5 {name=b dir=in}\n",
        ),
        f(
            "x/devices/lab_pin.sym",
            "v {xschem version=3.4.7 file_version=1.2}\nK {type=label\nformat=\"*.alias @lab\"\ntemplate=\"name=p1 lab=xxx\"}\nB 5 -1.25 -1.25 1.25 1.25 {name=p dir=in}\nT {@lab} -2.5 -12.5 0 0 0.33 0.33 {}\n",
        ),
        f(
            "x/devices/gnd.sym",
            "v {xschem version=3.4.7 file_version=1.2}\nK {type=label\nglobal=true\nformat=\"*.alias @lab\"\ntemplate=\"name=l1 lab=GND\"}\nB 5 -2.5 -2.5 2.5 2.5 {name=p dir=inout}\n",
        ),
        f(
            "x/devices/ipin.sym",
            "v {xschem version=3.4.7 file_version=1.2}\nK {type=ipin\nformat=\"*.ipin @lab\"\ntemplate=\"name=p1 lab=xxx\"}\nB 5 -1.25 -1.25 1.25 1.25 {name=p dir=out}\n",
        ),
        f(
            "x/devices/opin.sym",
            "v {xschem version=3.4.7 file_version=1.2}\nK {type=opin\nformat=\"*.opin @lab\"\ntemplate=\"name=p1 lab=xxx\"}\nB 5 -1.25 -1.25 1.25 1.25 {name=p dir=in}\n",
        ),
        f(
            "x/devices/code.sym",
            "v {xschem version=3.4.7 file_version=1.2}\nK {type=netlist_commands\ntemplate=\"name=s1 value=blabla\"\nformat=\"\n@value\n\"}\n",
        ),
        f(
            "x/devices/launcher.sym",
            "v {xschem version=3.4.7 file_version=1.2}\nK {type=launcher\ntemplate=\"name=h1 url=x\"}\n",
        ),
        f(
            "x/lib/inv.sym",
            "v {xschem version=3.4.7 file_version=1.2}\nK {type=subcircuit\nformat=\"@name @pinlist @symname WN=@WN\"\ntemplate=\"name=x1 WN=1u\"}\nB 5 -42.5 -2.5 -37.5 2.5 {name=A dir=in}\nB 5 37.5 -2.5 42.5 2.5 {name=Z dir=out}\nL 4 -40 0 40 0 {}\nT {@name} 0 -20 0 0 0.2 0.2 {}\nT {@spice_get_voltage} 0 20 0 0 0.2 0.2 {layer=15}\n",
        ),
        f(
            "x/lib/inv.sch",
            "v {xschem version=3.4.7 file_version=1.2}\nN 0 -30 0 -50 {lab=Z}\nC {ipin.sym} -40 0 0 0 {name=p1 lab=A}\nC {opin.sym} 0 -50 0 0 {name=p2 lab=Z}\nC {devices/nmos4.sym} -20 0 0 0 {name=M1 w=WN l=0.2u}\nC {lab_pin.sym} 0 30 0 0 {name=l1 lab=0}\nC {lab_pin.sym} 0 0 0 0 {name=l2 lab=0}\nC {code.sym} 0 -200 0 0 {name=MOD value=\".param KP=2\n.model nch nmos kp=KP\"}\nC {code.sym} 0 -300 0 0 {name=STIM only_toplevel=true value=\"Vin A 0 1\"}\nL 4 0 0 10 10 {}\n",
        ),
        f(
            "x/lib/nand.sym",
            "v {xschem version=3.4.7 file_version=1.2}\nK {type=primitive\nformat=\"@name @@A @@B @VDD @VSS @@Y @prefix\\\\\\\\nand2\"\ntemplate=\"name=x1 VDD=VDD VSS=VSS prefix=pdk_\"\nextra=\"VDD VSS prefix\"}\nB 5 -42.5 -12.5 -37.5 -7.5 {name=A dir=in}\nB 5 -42.5 7.5 -37.5 12.5 {name=B dir=in}\nB 5 37.5 -2.5 42.5 2.5 {name=Y dir=out}\n",
        ),
        f(
            "x/lib/inc.sym",
            "v {xschem version=3.4.7 file_version=1.2}\nK {type=subcircuit\nformat=\"@name @pinlist @symname\"\ntemplate=\"name=x1\"\nspice_sym_def=\".include inc.cir\"}\nB 5 -42.5 -2.5 -37.5 2.5 {name=A dir=in}\nB 5 37.5 -2.5 42.5 2.5 {name=Z dir=out}\n",
        ),
        // A side input: the file `inc.sym`'s spice_sym_def includes.
        f("x/lib/inc.cir", "* a model\n.subckt inc Z\n+ A\n.ends\n"),
        f(
            "x/lib/tb.sch",
            "v {xschem version=3.4.7 file_version=1.2}\nN 100 -30 100 -60 {lab=OUT}\nN 100 30 100 60 {}\nN 100 60 100 60 {}\nC {inv.sym} 60 0 1 1 {name=X1 WN=2u}\nC {nand.sym} 300 0 0 0 {name=x2 VSS=GND}\nC {res.sym} 100 0 0 0 {name=R1 value='VCC/2' footprint=0805}\nC {res.sym} 200 0 2 0 {name=R2 value=1k spice_ignore=short}\nC {gnd.sym} 100 60 0 0 {name=l1 lab=GND}\nC {lab_pin.sym} 100 -60 0 0 {name=l2 lab=OUT}\nC {code.sym} 0 -200 0 0 {name=SIM value=\".param VCC=5 HALF='VCC/2'\n.tran 1n 10n\n.include $::MODELS/x.lib\"}\nC {code.sym} 0 -300 0 0 {name=TCL format=\"tcleval(@value )\" value=\".lib $::MODELS/m.lib tt\"}\nC {launcher.sym} 0 -400 0 0 {name=h1}\nC {missing.sym} 0 -500 0 0 {name=U1}\nC {res.sym} 400 0 0 0 {name=R3 value=2k format=\"@name @pinlist @value\"}\nC {nand.sym} 500 0 0 0 {name=x3 lvs_format=\" \"}\nC {nand.sym} 500 100 0 0 {name=x4 lvs_format=\" \"}\nC {inc.sym} 600 0 0 0 {name=x5}\nT {a note} 0 100 0 0 0.4 0.4 {}\nB 2 0 0 100 100 {flags=graph}\n",
        ),
    ]
}

fn opts<'a>(roots: &'a [String], vars: &'a [(String, String)]) -> Options<'a> {
    Options {
        roots,
        pdk_vars: vars,
        ..Options::default()
    }
}

fn run(files: &[(String, String)]) -> (Vec<(String, Vec<u8>)>, interop::Report) {
    let roots = vec!["x/devices".to_owned(), "x".to_owned()];
    let vars = vec![("MODELS".to_owned(), "${PDK_ROOT}/models".to_owned())];
    let inputs: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(p, s)| (p.as_str(), s.as_bytes()))
        .collect();
    translate_library(&inputs, &opts(&roots, &vars))
}

fn file<'a>(out: &'a [(String, Vec<u8>)], path: &str) -> &'a str {
    let f = out
        .iter()
        .find(|f| f.0 == path)
        .unwrap_or_else(|| panic!("no {path}"));
    std::str::from_utf8(&f.1).expect("utf-8")
}

#[test]
fn fixture_translates_and_every_output_checks() {
    let (out, report) = run(&fixture());
    let problems = check_output(&out);
    assert!(problems.is_empty(), "{problems:#?}");
    let paths: Vec<&str> = out.iter().map(|f| f.0.as_str()).collect();
    for p in [
        "lib/inv.shcomp",
        "lib/nand.shprim",
        "lib/tb.shbench",
        "schemify.project",
        "import-report.md",
        "import-report.txt",
    ] {
        assert!(paths.contains(&p), "{p} missing from {paths:?}");
    }
    // Mapped devices and non-cells produce no files of their own.
    assert!(
        !paths.iter().any(|p| p.starts_with("devices/")),
        "{paths:?}"
    );
    let tb = file(&out, "lib/tb.shbench");
    // Core mapping, D40 (footprint/default dropped, value kept verbatim), ignore.
    assert!(
        tb.contains("core:res\n  at 100 0 R0\n  name R1\n  value 'VCC/2'\n"),
        "{tb}"
    );
    assert!(tb.contains("  name R2\n  ignore short\n"), "{tb}");
    assert!(!tb.contains("footprint"), "{tb}");
    // Orientation (rot 1, flip 1) and an override of a subcircuit parameter.
    assert!(
        tb.contains("./inv\n  at 60 0 MR90\n  name X1\n  WN 2u\n"),
        "{tb}"
    );
    // Inherited pins: explicit nets on every instance (override or template default).
    assert!(
        tb.contains("  inherit VDD VDD\n  inherit VSS GND\n"),
        "{tb}"
    );
    // Labels; gnd.sym is a global label keeping xschem's name.
    assert!(tb.contains(" GND\n  global true\n"), "{tb}");
    assert!(tb.contains(" OUT\n"), "{tb}");
    // Code: `.param` became bench vars; PDK paths became ${VAR}; Tcl format honoured.
    assert!(
        tb.contains("var VCC 5\n") && tb.contains("var HALF VCC/2\n"),
        "{tb}"
    );
    assert!(
        tb.contains("  | .include $$::MODELS/x.lib\n"),
        "plain text keeps a literal $: {tb}"
    );
    assert!(tb.contains("  | .lib ${MODELS}/m.lib tt"), "{tb}");
    assert!(file(&out, "schemify.project").contains("set MODELS ${PDK_ROOT}/models\n"));
    // Component: ports bound to the symbol pins, parameter passed down as an expression.
    let inv = file(&out, "lib/inv.shcomp");
    assert!(
        inv.contains("impl schematic schematic\n  format spice \"{name} {pins} {cell}\"\n"),
        "{inv}"
    );
    assert!(
        inv.contains(" A\n") && inv.contains("port ") && inv.contains("  w WN\n"),
        "{inv}"
    );
    assert!(inv.contains("stext 0 -20 R0 {name}\n"), "{inv}");
    // Primitive with extra pins: inherited pins and a terminated token.
    let nand = file(&out, "lib/nand.shprim");
    assert!(
        nand.contains("pin VDD inout 0 0\n  inherit VDD\n"),
        "{nand}"
    );
    assert!(
        nand.contains("format spice \"{name} {A} {B} {VDD} {VSS} {Y} {prefix}nand2\""),
        "{nand}"
    );
    // Report: needs_review for the missing symbol, drops for the rest.
    let has = |c: Class| report.items.iter().any(|i| i.class == c);
    assert!(has(Class::Review(Reason::MissingSymbol)));
    assert!(has(Class::Dropped(Drop::NonElectrical)) && has(Class::Dropped(Drop::Graphic)));
    assert!(has(Class::Dropped(Drop::ZeroWire)) && has(Class::Dropped(Drop::Display)));
    // R3 overrides its format: a variant cell, not core:res.
    assert_eq!((report.core_instances, report.device_instances), (3, 4));
    // Instance netlist text (spec §3): variant cells, one per distinct override set.
    assert!(
        tb.contains("./res__R3\n  at 400 0 R0\n  name R3\n  value 2k\n"),
        "{tb}"
    );
    let r3 = file(&out, "lib/res__R3.shprim");
    assert!(
        r3.contains("format spice \"{name} {pins} {value}\"\n"),
        "{r3}"
    );
    assert!(
        tb.contains("./nand__x3\n") && !tb.contains("nand__x4"),
        "{tb}"
    );
    let x3 = file(&out, "lib/nand__x3.shprim");
    assert!(
        x3.contains("  format cdl \"\"\n"),
        "a blank lvs_format: {x3}"
    );
    assert!(
        !report
            .items
            .iter()
            .any(|i| i.class == Class::Review(Reason::InstanceText)),
        "overrides are honoured, not reviewed"
    );
    // spice_sym_def: netlisted by its text, pins in the included `.subckt` order.
    let inc = file(&out, "lib/inc.shprim");
    assert!(
        inc.contains("format spice \"{name} {Z} {A} {cell}\"\n"),
        "{inc}"
    );
    assert!(inc.contains("body spice\n  | .include inc.cir\n"), "{inc}");
    assert!(
        !paths.iter().any(|p| p.ends_with(".cir")),
        "side inputs are read, not output"
    );
    // D17 as amended: a component's netlist text is its own `code` (its `.param` lines
    // stay in it, xschem writes them inside the subckt); `only_toplevel` text belongs to
    // the bench copy alone.
    assert!(
        inv.contains("code MOD spice\n  | .param KP=2\n  | .model nch nmos kp=KP\n"),
        "{inv}"
    );
    assert!(!inv.contains("STIM") && !inv.contains("var KP"), "{inv}");
    let copy = file(&out, "lib/inv__top.shbench");
    assert!(copy.contains("code STIM spice\n  | Vin A 0 1\n"), "{copy}");
    assert!(
        copy.contains("var KP 2\n"),
        "the bench copy's .param: {copy}"
    );
    assert!(has(Class::Dropped(Drop::BenchCopy)));
    assert!(
        !report
            .items
            .iter()
            .any(|i| matches!(i.class, Class::Review(_)) && i.detail.contains("MOD")),
        "component code is translated, not reviewed"
    );
    report.assert_conservation();
}

#[test]
fn reimport_is_byte_identical_whatever_the_input_order() {
    let files = fixture();
    let (a, _) = run(&files);
    let mut rev = files.clone();
    rev.reverse();
    let (b, _) = run(&rev);
    assert_eq!(a, b);
}

/// The table matches xschem's documented placement (feature inventory §5.6: flip x
/// first, then rot 1 maps (x, y) → (−y, x), clockwise on screen with y down).
#[test]
fn orientation_table_matches_documented_xschem_transform() {
    let xschem = |rot: i16, flip: i16, p: [i32; 2]| {
        let (mut x, mut y) = (p[0], p[1]);
        if flip != 0 {
            x = -x;
        }
        for _ in 0..rot {
            (x, y) = (-y, x);
        }
        [x, y]
    };
    for rot in 0..4 {
        for flip in 0..2 {
            for p in [[20, -30], [-20, 0], [7, 13]] {
                assert_eq!(
                    place(p, orient(rot, flip), [0, 0]),
                    Some(xschem(rot, flip, p)),
                    "rot {rot} flip {flip}"
                );
            }
        }
    }
    assert_eq!(ORIENT_TABLE[1][1], Orient::MR90);
    assert_eq!(orient(-1, 5), Orient::MR270);
}

/// A byte-level mutation of one fixture file.
fn mutate(s: &str, ops: &[(u16, u8, u8)]) -> String {
    let mut b = s.as_bytes().to_vec();
    for &(at, op, byte) in ops {
        let i = usize::from(at) % (b.len() + 1);
        let table = b"{}\"\\@ =\n.;[]$0aZ-5";
        let byte = table[usize::from(byte) % table.len()];
        match op % 3 {
            0 if i < b.len() => b[i] = byte,
            1 => b.insert(i, byte),
            _ if i < b.len() => {
                b.remove(i);
            }
            _ => {}
        }
    }
    String::from_utf8_lossy(&b).into_owned()
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

    /// Mutated real-shaped files: the whole translate path never panics, conserves
    /// records (asserted inside) and still produces only valid files.
    #[test]
    fn mutated_inputs_translate_to_valid_outputs(
        which in 0usize..12,
        ops in prop::collection::vec((any::<u16>(), any::<u8>(), any::<u8>()), 1..12),
    ) {
        let mut files = fixture();
        let k = which % files.len();
        files[k].1 = mutate(&files[k].1, &ops);
        let (out, report) = run(&files);
        report.assert_conservation();
        let problems = check_output(&out);
        prop_assert!(problems.is_empty(), "{problems:#?}\n{}", files[k].1);
    }
}

#[test]
fn parse_of_every_output_cell_is_canonical() {
    let (out, _) = run(&fixture());
    for (p, b) in out.iter().filter(|f| {
        f.0.ends_with(".shbench") || f.0.ends_with(".shcomp") || f.0.ends_with(".shprim")
    }) {
        let d = parse(b).unwrap_or_else(|e| panic!("{p}: {e:?}"));
        assert_eq!(schemify_format::write(&d).as_bytes(), b.as_slice(), "{p}");
    }
}

/// Regression (proptest `mutated_inputs_translate_to_valid_outputs`, CI 2026-10-11): a
/// mutated `nand.sym` whose `K` block swallows its pin boxes and whose `template` fused
/// into `format` has no pin, yet its `extra` net `VDD` is a standalone `@VDD` in the
/// format. It was classified a cell with only inherited pins, but no template value
/// backs `VDD`, so it got none: a `.shprim` with zero pins (`E025`). The classification
/// now uses the pin rule itself, so the symbol is netlist text, and no file is invalid.
#[test]
fn pinless_symbol_without_inherited_pins_is_no_cell() {
    let mutated = r#"v {schem version=3.4.7 file_version=1.2}
K {type=primitive
format="@name @@A @@B @VDD VSS @@Y @prefix\\\\nand2"template="name=x1 VDD=VDD VSS=VSS prefix=pdk_"
extra="VDD "SS prefix"
B 5 -42.5 -12.5 -37.5 -7.5 {name=A dir=in
B 5 -42.5 7.5 -37.5 12.5 {name=B dir=in}
B 5 37.5 -2.542.5 2.5 {name=Y dir=out}
"#;
    let mut files = fixture();
    files
        .iter_mut()
        .find(|f| f.0 == "x/lib/nand.sym")
        .expect("fixture")
        .1 = mutated.to_owned();
    let (out, report) = run(&files);
    report.assert_conservation();
    let problems = check_output(&out);
    assert!(problems.is_empty(), "{problems:#?}");
    assert!(
        !out.iter().any(|f| f.0.starts_with("lib/nand")),
        "no cell (and no variant) for a pinless symbol without inherited pins"
    );
    // Positive space: the unmutated nand is still a cell.
    let (out, _) = run(&fixture());
    assert!(out.iter().any(|f| f.0 == "lib/nand.shprim"));
}

/// inv_xschem rows 6 and 17: a per-element value list on an iterated instance and
/// `lvs_ignore` (instance and symbol) cross as `needs_review` rows, never verbatim or silent.
#[test]
fn value_lists_and_lvs_ignore_are_reviewed() {
    let mut files = fixture();
    let v = "v {xschem version=3.4.7 file_version=1.2}\n";
    files.push((
        "x/lib/ign.sym".into(),
        format!("{v}K {{type=primitive\nformat=\"@name @pinlist ign\"\ntemplate=\"name=x1 lvs_ignore=true\"\ntedax_format=\"device @name ign\"}}\nB 5 -2.5 -2.5 2.5 2.5 {{name=a dir=inout}}\n"),
    ));
    files.push((
        "x/lib/vec.sch".into(),
        format!("{v}C {{res.sym}} 0 0 0 0 {{name=R[2..1] value=\"?1 1k,'1k/2'\" lvs_ignore=short}}\nC {{ign.sym}} 100 0 0 0 {{name=x9}}\n"),
    ));
    let (out, report) = run(&files);
    report.assert_conservation();
    let t = report.text();
    let row = |class: &str, file: &str, detail: &str| {
        t.lines().any(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            f.len() == 5 && f[0] == class && f[1].ends_with(file) && f[4].contains(detail)
        })
    };
    assert!(
        row(
            "needs_review:expression",
            "vec.sch",
            "per-element value list"
        ),
        "{t}"
    );
    assert!(
        row("needs_review:unsupported", "vec.sch", "lvs_ignore=short"),
        "{t}"
    );
    assert!(
        row("needs_review:unsupported", "ign.sym", "lvs_ignore=true"),
        "{t}"
    );
    // Row 15: a dialect Schemify has no impl for is counted.
    assert!(t.contains("unused-attr\tsymbol tedax_format\t1\n"), "{t}");
    let vec = file(&out, "lib/vec.shbench");
    // The elements keep the default; the list rides in `xsprop` and goes back to xschem.
    assert!(
        vec.contains("name R2,R1\n  xsprop value \"?1 1k,'1k/2'\"\n") && !vec.contains("  value "),
        "{vec}"
    );
    let files: interop::design::Files = out.into_iter().collect();
    let d = interop::design::load_closed(&files, &["lib/vec.shbench"]).unwrap();
    let x = interop::xschem_out::export(&d, &[d.find("lib/vec.shbench").unwrap()]).unwrap();
    let sch = String::from_utf8_lossy(&x.files.iter().find(|f| f.0 == "lib/vec.sch").unwrap().1)
        .into_owned();
    assert!(sch.contains("{name=R2,R1 value=\"?1 1k,'1k/2'\""), "{sch}");
}

/// inv_xschem row 10: xschem netlists a repeated instance name twice (golden nand2), elab
/// refuses it (E009): the second is renamed, with a `needs_review:name` row.
#[test]
fn a_repeated_instance_name_is_renamed() {
    let mut files = fixture();
    files.push((
        "x/lib/dup.sch".into(),
        "v {xschem version=3.4.7 file_version=1.2}\nC {res.sym} 0 0 0 0 {name=R5}\nC {res.sym} 100 0 0 0 {name=r5}\nC {res.sym} 200 0 0 0 {name=R5_2}\n".into(),
    ));
    let (out, report) = run(&files);
    report.assert_conservation();
    let dup = file(&out, "lib/dup.shbench");
    for n in ["name R5\n", "name r5_2\n", "name R5_2_2\n"] {
        assert!(dup.contains(n), "{n} in {dup}");
    }
    assert!(
        report
            .text()
            .contains("instance name r5 is used twice: renamed r5_2"),
        "{}",
        report.text()
    );
}

/// inv_xschem row 14: schematic drawings become rank-3 `shape` records (and go back out
/// through the xschem export); a graph box stays a `dropped:schematic-graphic` row.
#[test]
fn schematic_drawings_become_shapes() {
    let (out, report) = run(&fixture());
    let inv = file(&out, "lib/inv.shcomp");
    assert!(
        inv.lines()
            .any(|l| l.starts_with("shape ") && l.ends_with(" line 0 0 10 10")),
        "{inv}"
    );
    let t = report.text();
    assert!(
        t.lines().any(|l| l
            .starts_with("dropped:schematic-graphic\tlib/tb.sch\t0\t1\t1 L/B/P/A records (graphs")),
        "{t}"
    );
    let files: interop::design::Files = out.into_iter().collect();
    let d = interop::design::load_closed(&files, &["lib/inv.shcomp"]).unwrap();
    let x = interop::xschem_out::export(&d, &[d.find("lib/inv.shcomp").unwrap()]).unwrap();
    let sch = String::from_utf8_lossy(&x.files.iter().find(|f| f.0 == "lib/inv.sch").unwrap().1)
        .into_owned();
    assert!(sch.contains("\nL 4 0 0 10 10 {}"), "{sch}");
}

/// inv_xschem row X22: `schematic=tcleval([hierarchy_config @symname])` (IHP std cells)
/// resolves to the proc's default, the same-named schematic in the sibling `sch` tree; with
/// no such file, or with another proc, it is a `needs_review:tcl` row and the cell a leaf.
#[test]
fn hierarchy_config_takes_its_default_view_or_is_reviewed() {
    let v = "v {xschem version=3.4.7 file_version=1.2}\n";
    let sym = |cfg: &str| {
        format!(
            "{v}K {{type=subcircuit\nformat=\"@name @pinlist @symname\"\ntemplate=\"name=x1\"\nschematic=\"tcleval([{cfg} @symname])\"}}\nB 5 -42.5 -2.5 -37.5 2.5 {{name=A dir=in}}\nB 5 37.5 -2.5 42.5 2.5 {{name=Z dir=out}}\n"
        )
    };
    let sch = format!(
        "{v}C {{res.sym}} 0 0 0 0 {{name=R1}}\nC {{ipin.sym}} 0 -30 0 0 {{name=p1 lab=A}}\nC {{opin.sym}} 0 30 0 0 {{name=p2 lab=Z}}\n"
    );
    let mut files = fixture();
    files.extend([
        (
            "x/cells/sym/xschem/c1.sym".to_owned(),
            sym("hierarchy_config"),
        ),
        ("x/cells/sch/xschem/c1.sch".to_owned(), sch),
        (
            "x/cells/sym/xschem/c2.sym".to_owned(),
            sym("hierarchy_config"),
        ),
        ("x/cells/sym/xschem/c3.sym".to_owned(), sym("my_config")),
    ]);
    let (out, report) = run(&files);
    report.assert_conservation();
    let paths: Vec<&str> = out.iter().map(|f| f.0.as_str()).collect();
    assert!(paths.contains(&"cells/c1.shcomp"), "{paths:?}");
    assert!(file(&out, "cells/c1.shcomp").contains("core:res"));
    let t = report.text();
    assert!(
        t.lines().any(
            |l| l.starts_with("needs_review:tcl\tcells/sym/xschem/c2.sym")
                && l.contains("no ")
                && l.contains("c2.sch (the proc's default)")
        ),
        "{t}"
    );
    assert!(
        t.lines().any(
            |l| l.starts_with("needs_review:tcl\tcells/sym/xschem/c3.sym")
                && l.contains("my_config")
        ),
        "{t}"
    );
}

/// inv_xschem row X15: a schematic's `V {}` Verilog text is the cell's `code … verilog`
/// (xschem writes it in the module body); a `devices/` symbol mapped to core loses its
/// `verilog_format` with a review row (core cells have no Verilog format).
#[test]
fn verilog_text_is_code_and_a_core_device_verilog_format_is_reviewed() {
    let mut files = fixture();
    let res = files
        .iter_mut()
        .find(|f| f.0 == "x/devices/res.sym")
        .unwrap();
    res.1 = res.1.replace(
        "template=",
        "verilog_format=\"tran @name (@@P\\\\\\\\, @@M\\\\\\\\);\"\ntemplate=",
    );
    files.push((
        "x/lib/vl.sch".into(),
        "v {xschem version=3.4.7 file_version=1.2}\nV {assign #del Y=~A;\n$display(\"x\");}\nC {ipin.sym} 0 0 0 0 {name=p1 lab=A}\nC {opin.sym} 100 0 0 0 {name=p2 lab=Y}\n".into(),
    ));
    let (out, report) = run(&files);
    report.assert_conservation();
    let vl = file(&out, "lib/vl.shcomp");
    assert!(
        vl.contains(
            "code schematic_verilog verilog\n  | assign #del Y=~A;\n  | $$display(\"x\");\n"
        ),
        "{vl}"
    );
    let t = report.text();
    assert!(
        t.lines().any(
            |l| l.starts_with("needs_review:unsupported\tdevices/res.sym")
                && l.contains("core:res has no Verilog format")
        ),
        "{t}"
    );
    assert!(!t.contains("other-netlister-text\tlib/vl.sch"), "{t}");
}
