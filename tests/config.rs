//! Config views (`virtuoso::config`, spec `interop.md` §4.2 `config`, §9.3): the probe's
//! `expand.cfg` (every rule kind) → bench `config` record → `expand.cfg`, byte for byte;
//! an edited record regenerates the text and reads back to itself; a Schemify-made config
//! and its maestro test binding cross; and the same through the SKILL dumper and loader on
//! the mock hdb.

mod common;
mod skill;

use common::{cadence_fixture, import_opts, load_all};
use interop::design::Files;
use interop::map::Mapping;
use interop::sfyx::{self, Record, s};
use interop::virtuoso::{self, ExportOptions};
use skill::mockdb::MockDb;
use skill::{Interp, V};

/// What probe `config` saved (the committed copy of `config/hier_config/expand.cfg`, which
/// [`the_real_fixture_crosses_byte_for_byte`] checks against the local fixture).
const PROBE: &str = "//Revision 1\nconfig hier;\ndesign sfyprobe_lib.hier:schematic;\nliblist sfyprobe_lib, analogLib, basic;\n\nviewlist spectre, cmos_sch, schematic, veriloga;\nstoplist spectre;\n\n\ncell sfyprobe_lib.sub viewlist schematic, symbol;\ninst (sfyprobe_lib.hier:schematic).I1 liblist sfyprobe_lib;\ninst (sfyprobe_lib.hier:schematic).I1 binding :schematic;\ninst (sfyprobe_lib.hier:schematic).I0 stop;\n\ncell analogLib.res binding :spectre;\n\npath (sfyprobe_lib.hier:schematic).(sfyprobe_lib.sub:schematic)I1.RS0\n    binding analogLib.res:spectre;\n\nendconfig\n";

/// The probe's library: `hier` (I0, I1 of `sub`), `sub` (RS0 of analogLib `res`), with
/// `hier/config` holding `cfg`.
fn dump(cfg: &str, maestro_view: Option<&str>) -> String {
    let base = r#"("sfyx" 1 "t")
("lib" "analogLib")
("cell" "res" "ref")
("cv" "symbol" "schematicSymbol" 160)
("term" "MINUS" "inputOutput" "")
("term" "PLUS" "inputOutput" "")
("pinfig" 1 "MINUS" -2 -42 2 -38)
("pinfig" 2 "PLUS" -2 38 2 42)
("lib" "sfyprobe_lib")
("cell" "hier" "design")
("cv" "schematic" "schematic" 160)
("inst" 1 "I0" "sfyprobe_lib" "sub" "symbol" 0 0 "R0")
("inst" 2 "I1" "sfyprobe_lib" "sub" "symbol" 160 0 "R0")
("cell" "sub" "design")
("cv" "schematic" "schematic" 160)
("inst" 1 "RS0" "analogLib" "res" "symbol" 0 0 "R0")
("cv" "symbol" "schematicSymbol" 160)
("shape" 1 "rect" "device" "drawing" (-20 -20 20 20) 0)
("end")
"#;
    let mut recs = sfyx::read(base).expect("base dump");
    let at = recs
        .iter()
        .position(|r| r.tag == "cell" && r.str(0).ok() == Some("sub"));
    recs.insert(
        at.expect("sub"),
        Record::new("config", vec![s("config"), sfyx::text(cfg)]),
    );
    if let Some(v) = maestro_view {
        let end = recs.len() - 1;
        let test = vec![s("T1"), s("sfyprobe_lib"), s("hier"), s(v), s("spectre")];
        recs.insert(end, Record::new("var", vec![s("R"), s("1k"), s("")]));
        recs.insert(end, Record::new("maestro", test));
    }
    sfyx::write(&recs)
}

fn import(text: &str) -> Files {
    let imp = virtuoso::import(text, &Files::new(), &Mapping::default(), &import_opts())
        .unwrap_or_else(|e| panic!("{e:?}"));
    imp.files.into_iter().collect()
}

fn text(files: &Files, path: &str) -> String {
    String::from_utf8(files[path].clone()).unwrap()
}

/// The config views of `files` exported, `(view, text)`, and the maestro tests' views.
fn export(files: &Files) -> (Vec<(String, String)>, Vec<String>) {
    let d = load_all(files);
    let tops: Vec<usize> = (0..d.cells.len())
        .filter(|&k| d.cells[k].lib != "core")
        .collect();
    let opts = ExportOptions {
        producer: "t".into(),
        ..ExportOptions::default()
    };
    let e =
        virtuoso::export(&d, &tops, &Mapping::default(), &opts).unwrap_or_else(|e| panic!("{e:?}"));
    let recs = sfyx::read(&e.sfyx).unwrap();
    let cfgs = recs
        .iter()
        .filter(|r| r.tag == "config")
        .map(|r| (r.str(0).unwrap().to_owned(), r.text(1).unwrap()))
        .collect();
    let views = recs
        .iter()
        .filter(|r| r.tag == "maestro")
        .map(|r| r.str(3).unwrap().to_owned())
        .collect();
    (cfgs, views)
}

/// The mapped lines of the `config` record (its attributes but `vcfg`/`vcfgsum`).
fn mapped(bench: &str) -> Vec<String> {
    let at = bench.find("\nconfig ").expect("a config record");
    bench[at + 1..]
        .lines()
        .skip(1)
        .take_while(|l| l.starts_with("  "))
        .filter(|l| !l.starts_with("  vcfg"))
        .map(str::to_owned)
        .collect()
}

const BENCH: &str = "sfyprobe_lib/hier.shbench";

#[test]
fn every_rule_kind_maps_and_comes_back_byte_for_byte() {
    let files = import(&dump(PROBE, None));
    let bench = text(&files, BENCH);
    assert_eq!(
        mapped(&bench),
        [
            "  order spice schematic veriloga",
            "  cell analogLib:res spice",
            "  inst /I1/RS0 spice",
            "  cellinst ./hier I1 schematic",
            "  stop spice",
        ],
        "{bench}"
    );
    // The statements with no Schemify form are kept for Virtuoso, the rest regenerable.
    for keep in [
        "liblist sfyprobe_lib, analogLib, basic;",
        "cell sfyprobe_lib.sub viewlist schematic, symbol;",
        "inst (sfyprobe_lib.hier:schematic).I1 liblist sfyprobe_lib;",
        "inst (sfyprobe_lib.hier:schematic).I0 stop;",
    ] {
        let kept = |l: &str| l.starts_with("  vcfg keep \"") && l.ends_with(&format!("n{keep}\""));
        assert!(bench.lines().any(kept), "{keep}\n{bench}");
    }
    let (cfgs, _) = export(&files);
    assert_eq!(cfgs, [("config".to_owned(), PROBE.to_owned())]);
}

#[test]
fn the_real_fixture_crosses_byte_for_byte() {
    let Some(p) = cadence_fixture("config/hier_config/expand.cfg") else {
        return;
    };
    let real = std::fs::read_to_string(p).unwrap();
    assert_eq!(real, PROBE, "the committed copy is the probe's file");
    let (cfgs, _) = export(&import(&dump(&real, None)));
    assert_eq!(cfgs[0].1, real);
    let tag = cadence_fixture("config/hier_config/master.tag").unwrap();
    assert_eq!(
        std::fs::read_to_string(tag).unwrap(),
        "-- Master.tag File, Rev:1.0\nexpand.cfg\n",
        "hdbSave writes master.tag; the loader never does"
    );
}

#[test]
fn an_edited_config_regenerates_and_reads_back_to_itself() {
    let mut files = import(&dump(PROBE, None));
    let edited = text(&files, BENCH)
        .replace(
            "  order spice schematic veriloga\n",
            "  order schematic spice\n",
        )
        .replace(
            "  cellinst ./hier I1 schematic\n",
            "  cellinst ./hier I1 spice\n  cellinst ./sub RS0 spice\n",
        )
        .replace("  stop spice\n", "  stop spice ./sub\n");
    files.insert(BENCH.into(), edited.clone().into_bytes());
    let (cfgs, _) = export(&files);
    let out = &cfgs[0].1;
    assert_eq!(
        out,
        "config hier;\ndesign sfyprobe_lib.hier:schematic;\nliblist sfyprobe_lib, analogLib, basic;\ncell sfyprobe_lib.sub viewlist schematic, symbol;\ninst (sfyprobe_lib.hier:schematic).I1 liblist sfyprobe_lib;\ninst (sfyprobe_lib.hier:schematic).I0 stop;\nviewlist schematic, spectre;\nstoplist spectre;\ncell analogLib.res binding :spectre;\npath (sfyprobe_lib.hier:schematic).(sfyprobe_lib.sub:schematic)I1.RS0 binding analogLib.res:spectre;\ninst (sfyprobe_lib.hier:schematic).I1 binding :spectre;\ninst (sfyprobe_lib.sub:schematic).RS0 binding :spectre;\ncell sfyprobe_lib.sub stop;\nendconfig\n"
    );
    let back = text(&import(&dump(out, None)), BENCH);
    let mut want = mapped(&edited);
    let mut got = mapped(&back);
    want.sort();
    got.sort();
    assert_eq!(got, want, "{back}");
}

#[test]
fn a_schemify_config_and_its_test_binding_cross() {
    // Import binds the bench to the config its maestro test runs (G31).
    let files = import(&dump(PROBE, Some("config")));
    let bench = text(&files, BENCH);
    let header = bench.split("\n\n").next().unwrap();
    assert!(header.contains("\n  config config"), "{bench}");
    let (_, views) = export(&files);
    assert_eq!(views, ["config"]);
    // A config made in Schemify, the bench header naming it.
    let mut files = import(&dump(PROBE, Some("schematic")));
    let b = text(&files, BENCH);
    let at = b.find("\nconfig config").unwrap();
    let mut mine = b[..at].to_owned();
    mine = mine.replacen("\n", "\n  config fast\n", 1);
    mine.push_str("\nconfig fast\n  order schematic spice\n  cell analogLib:res spice\n  inst /I1/RS0 veriloga\n");
    files.insert(BENCH.into(), mine.into_bytes());
    let (cfgs, views) = export(&files);
    assert_eq!(views, ["fast"]);
    assert_eq!(
        cfgs,
        [(
            "fast".to_owned(),
            "config hier;\ndesign sfyprobe_lib.hier:schematic;\nliblist sfyprobe_lib, analogLib;\nviewlist schematic, spectre;\ncell analogLib.res binding :spectre;\npath (sfyprobe_lib.hier:schematic).(sfyprobe_lib.sub:schematic)I1.RS0 binding analogLib.res:veriloga;\nendconfig\n".to_owned()
        )]
    );
}

// ---------------------------------------------------------------------- SKILL and mock hdb

const BUILD: &str = include_str!("fixtures/skill/build.il");

/// A mock Virtuoso with analogLib (`SfyFixStdlib`) and an empty `sfyprobe_lib`.
fn virtuoso() -> Interp<MockDb> {
    let mut i = skill::virtuoso(&["analogLib", "basic", "sfyprobe_lib"]);
    i.load(BUILD).expect("build.il loads");
    i.load("(SfyFixStdlib)").expect("stdlib");
    i
}

fn run(i: &mut Interp<MockDb>, src: &str) -> V {
    i.load(src)
        .unwrap_or_else(|e| panic!("{src}: {e}\nlog: {}", i.log))
}

/// The `config` records of the dump at `path`.
fn configs(i: &Interp<MockDb>, path: &str) -> Vec<(String, String)> {
    sfyx::read(&i.files[path])
        .unwrap()
        .iter()
        .filter(|r| r.tag == "config")
        .map(|r| (r.str(0).unwrap().to_owned(), r.text(1).unwrap()))
        .collect()
}

#[test]
fn the_loader_writes_the_view_through_hdb_and_the_dumper_reads_it_back() {
    let mut i = virtuoso();
    i.files.insert("/t/in.sfyx".into(), dump(PROBE, None));
    let ok = run(&mut i, r#"(SfyImport "/t/in.sfyx")"#);
    let log = i.files["/t/in.sfyx.log"].clone();
    assert!(ok.truthy() && log.contains("count config 1\n"), "{log}");
    // hdb made the view (expand.cfg, master.tag); the loader wrote the text.
    for f in ["expand.cfg", "master.tag"] {
        let found = run(
            &mut i,
            &format!(r#"(ddGetObj "sfyprobe_lib" "hier" "config" "{f}")"#),
        );
        assert!(found.truthy(), "{f}");
    }
    run(
        &mut i,
        r#"(SfyExport "/t/out.sfyx" "sfyprobe_lib" (list "hier"))"#,
    );
    assert_eq!(
        configs(&i, "/t/out.sfyx"),
        [("config".to_owned(), PROBE.to_owned())]
    );
    // An existing view is refused without ?overwrite and replaced with it.
    assert!(!run(&mut i, r#"(SfyImport "/t/in.sfyx")"#).truthy());
    assert!(i.files["/t/in.sfyx.log"].contains("hier/config exists; pass ?overwrite t"));
    assert!(run(&mut i, r#"(SfyImport "/t/in.sfyx" ?overwrite t)"#).truthy());
}

#[test]
fn the_dumper_reads_the_real_fixture_verbatim() {
    let Some(p) = cadence_fixture("config/hier_config/expand.cfg") else {
        return;
    };
    let real = std::fs::read_to_string(p).unwrap();
    let mut i = virtuoso();
    i.files
        .insert("/t/in.sfyx".into(), dump("config hier;\nendconfig\n", None));
    assert!(run(&mut i, r#"(SfyImport "/t/in.sfyx")"#).truthy());
    let f = i
        .host
        .add_text_file("sfyprobe_lib", "hier", "config", "expand.cfg");
    i.files.insert(f, real.clone());
    run(
        &mut i,
        r#"(SfyExport "/t/out.sfyx" "sfyprobe_lib" (list "hier"))"#,
    );
    assert_eq!(configs(&i, "/t/out.sfyx"), [("config".to_owned(), real)]);
}

#[test]
fn mock_hdb_refuses_what_hdb_refuses() {
    let mut i = virtuoso();
    // No config view to read; a closed handle signals; a top that does not exist is nil.
    assert!(!run(&mut i, r#"(hdbOpen "sfyprobe_lib" "x" "config" "r")"#).truthy());
    run(
        &mut i,
        r#"(setq sfyH (hdbOpen "sfyprobe_lib" "x" "config" "w"))"#,
    );
    assert!(
        !run(
            &mut i,
            r#"(hdbSetTopCellViewName sfyH "sfyprobe_lib" "x" "schematic")"#
        )
        .truthy()
    );
    assert!(!run(&mut i, "(hdbSave sfyH)").truthy(), "no top, no save");
    run(&mut i, "(hdbClose sfyH)");
    assert!(i.load("(hdbSave sfyH)").is_err());
    assert!(
        i.load(r#"(hdbOpen "sfyprobe_lib" "x" "config" "q")"#)
            .is_err()
    );
}
