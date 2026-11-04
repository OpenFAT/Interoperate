//! Gap D: on a redrawn cellview, every object that survives keeps its own Virtuoso
//! cosmetics (spec `interop.md` §6.2, §7.5). A dump goes through Rust import, an edit of
//! one object in Schemify, Rust export, a load into the mock Virtuoso that already holds
//! the original, and a dump again; only the edited object may differ.

mod common;
mod skill;

use common::{cadence_fixture, is_cell, load_tops, located};
use interop::design::Files;
use interop::map::Mapping;
use interop::sfyx;
use interop::virtuoso::{self, ExportOptions, ImportOptions};
use skill::mockdb::MockDb;
use skill::{Interp, V};

const BUILD: &str = include_str!("fixtures/skill/build.il");

fn run(i: &mut Interp<MockDb>, src: &str) {
    match i.load(src) {
        Ok(V::T) => {}
        r => panic!("{src}: {r:?}\n{}", i.log),
    }
}

/// The design cells' records of a dump without what the round trip adds by design: the
/// `schemify*` properties (ids, stash) the Rust exporter writes. (The reference cells are
/// the mock's own analogLib and basic.)
fn design_records(text: &str) -> String {
    let recs = sfyx::read(text).unwrap();
    let kept: Vec<sfyx::Record> = located(&recs)
        .into_iter()
        .filter(|(at, _)| at.design)
        .map(|(_, r)| r.clone())
        .filter(|r| !(r.tag == "prop" && r.str(1).is_ok_and(|n| n.starts_with("schemify"))))
        .collect();
    sfyx::write(&kept)
}

/// `first` (a SKILL dump of `lib`/`cell`) through Rust import, `edit` of the cell file,
/// Rust export, `SfyImport ?overwrite t` into a mock that holds `first`, and `SfyExport`:
/// the new dump, and the cell file as imported.
fn redraw(first: &str, lib: &str, cell: &str, edit: impl Fn(&str) -> String) -> (String, String) {
    let imp = virtuoso::import(
        first,
        &Files::new(),
        &Mapping::default(),
        &ImportOptions::default(),
    )
    .unwrap_or_else(|e| panic!("{e:?}"));
    let mut files: Files = imp.files.into_iter().collect();
    let path = files
        .keys()
        .find(|p| is_cell(p) && p.starts_with(&format!("{lib}/{cell}.")))
        .expect("the cell was imported")
        .clone();
    let text = String::from_utf8(files[&path].clone()).unwrap();
    let edited = edit(&text);
    assert_ne!(edited, text, "the edit changed nothing:\n{text}");
    files.insert(path.clone(), edited.into_bytes());
    let d = load_tops(&files, &[path.as_str()]);
    let eo = ExportOptions {
        producer: "cosm".into(),
        ..ExportOptions::default()
    };
    let second = virtuoso::export(&d, &[0], &Mapping::default(), &eo)
        .unwrap_or_else(|e| panic!("{e:?}"))
        .sfyx;
    let mut i = skill::virtuoso(&["analogLib", "basic", lib]);
    i.load(BUILD).unwrap();
    run(&mut i, "(SfyFixStdlib)");
    i.files.insert("/c/first.sfyx".into(), first.to_owned());
    if !matches!(i.load("(SfyImport \"/c/first.sfyx\")"), Ok(V::T)) {
        panic!("{}", i.files["/c/first.sfyx.log"]);
    }
    i.files.insert("/c/second.sfyx".into(), second.clone());
    if !matches!(
        i.load("(SfyImport \"/c/second.sfyx\" ?overwrite t)"),
        Ok(V::T)
    ) {
        panic!("{}\n{second}", i.files["/c/second.sfyx.log"]);
    }
    let log = &i.files["/c/second.sfyx.log"];
    assert!(
        log.contains("count saved ") && !log.contains("count kept unchanged 2"),
        "the edited cellview was kept, not redrawn:\n{log}"
    );
    run(
        &mut i,
        &format!("(SfyExport \"/c/third.sfyx\" \"{lib}\" (list \"{cell}\"))"),
    );
    (i.files["/c/third.sfyx"].clone(), text)
}

/// Lines of `want` and `got` that differ, for the panic message.
fn diff(want: &str, got: &str) -> String {
    let (a, b) = (design_records(want), design_records(got));
    let mut out = String::new();
    for l in a.lines().filter(|l| !b.lines().any(|m| m == *l)) {
        out.push_str(&format!("- {l}\n"));
    }
    for l in b.lines().filter(|l| !a.lines().any(|m| m == *l)) {
        out.push_str(&format!("+ {l}\n"));
    }
    out
}

/// The captured Virtuoso fixture (`cadence/cosmetics/`): `cosm.sfyx` with label 11 moved in
/// Schemify redraws as `cosm_moved.sfyx`, the dump of the same move made in Virtuoso.
#[test]
fn a_moved_label_redraws_and_every_other_cosmetic_stays() {
    let Some(dir) = cadence_fixture("cosmetics") else {
        return;
    };
    let read = |f: &str| std::fs::read_to_string(dir.join(f)).unwrap();
    let (first, moved) = (read("cosm.sfyx"), read("cosm_moved.sfyx"));
    let (third, imported) = redraw(&first, "sfyprobe_lib", "cosm", |t| {
        t.replace("voff 80 -40", "voff 40 -80")
    });
    let d = diff(&moved, &third);
    assert!(d.is_empty(), "{d}\nimported:\n{imported}");
}

const KIT: &str = include_str!("fixtures/skill/cosm_kit.il");
const GOLDEN: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/skill/cosm_kit.sfyx"
);

/// The mock dump of `work/kit` (every cosmetic kind), checked against its golden (blessed
/// with `SCHEMIFY_BLESS=1`).
fn kit() -> String {
    let mut i = skill::virtuoso(&["analogLib", "basic", "work"]);
    i.load(BUILD).unwrap();
    i.load(KIT).unwrap();
    run(&mut i, "(SfyFixStdlib)");
    i.load("(SfyFixKit)")
        .unwrap_or_else(|e| panic!("{e}\n{}", i.log));
    run(
        &mut i,
        "(SfyExport \"/c/kit.sfyx\" \"work\" (list \"kit\"))",
    );
    let text = i.files["/c/kit.sfyx"].clone();
    if std::env::var_os("SCHEMIFY_BLESS").is_some() {
        std::fs::write(GOLDEN, &text).unwrap();
    }
    let want = std::fs::read_to_string(GOLDEN).expect("run with SCHEMIFY_BLESS=1");
    assert_eq!(text, want, "cosm_kit.sfyx differs from its golden");
    text
}

/// Every cosmetic kind survives a redraw of its cellview: a wire label moved in Schemify
/// moves in Virtuoso by as much, and nothing else of the schematic changes.
#[test]
fn a_moved_wire_label_leaves_every_other_schematic_cosmetic_as_drawn() {
    let first = kit();
    for kept in [
        "\"OVR\" 0 -200000 \"R0\" \"lowerLeft\" \"stick\" 10000 \"normal\" \"text\" \"drawing\" 1 1 1)",
        "\"DRF\" 0 -240000 \"R0\" \"lowerLeft\" \"stick\" 10000 \"normal\" \"text\" \"drawing\" 0 0 1)",
        "\"INV\" 0 -280000 \"R0\" \"lowerLeft\" \"stick\" 10000 \"normal\" \"text\" \"drawing\" 0 1 0)",
        "\"nc1\" \"basic\" \"noConn\" \"symbol\" 200000 -100000 \"R90\")",
        "(-100000 200000 100000 200000) 4000)",
        " 0 \"Y\" 210000 -50000 \"R90\" \"centerLeft\" \"stick\" 8000 \"normal\" \"pin\" \"annotate\")",
        "\"path\" \"text\" \"drawing\" (300000 0 340000 0 340000 40000) 2000)",
        "\"ellipse\" \"text\" \"drawing\" (300000 100000 380000 140000) 0)",
    ] {
        assert!(first.contains(kept), "{kept}\n{first}");
    }
    let (third, imported) = redraw(&first, "work", "kit", |t| {
        t.replace(" 40 -200 R0 d[1:0]\n", " 60 -200 R0 d[1:0]\n")
    });
    let want = design_records(&first).replace("\"d<1:0>\" 40000 220000", "\"d<1:0>\" 60000 220000");
    assert_ne!(want, design_records(&first));
    let d = diff(&want, &third);
    assert!(d.is_empty(), "{d}\nimported:\n{imported}");
}

/// A symbol label whose template is edited in Schemify keeps its layer, font and flags; the
/// symbol's other labels (an overbar pin label among them) stay as drawn.
#[test]
fn a_template_edit_keeps_the_symbol_labels_drawn_style() {
    let first = kit();
    let (third, imported) = redraw(&first, "work", "kit", |t| {
        t.replace(" R0 note\n", " R0 note2\n")
    });
    let want = design_records(&first).replace("\"note\"", "\"note2\"");
    assert_ne!(want, design_records(&first));
    let d = diff(&want, &third);
    assert!(d.is_empty(), "{d}\nimported:\n{imported}");
}

/// Virtuoso keeps text heights on its 1/160-inch grid, and `size` gives every such height
/// back, so a note's height needs no attribute of its own (gap D's `vheight`).
#[test]
fn size_reproduces_every_grid_height() {
    use interop::profile::{height_from_virtuoso, height_of_size, size_of_height};
    for dbu in 1..=2000 {
        let h = dbu * 1000;
        let milli = height_from_virtuoso(h, 160_000);
        assert_eq!(
            i64::from(height_of_size(size_of_height(milli))),
            milli,
            "{dbu} DBU"
        );
    }
}
