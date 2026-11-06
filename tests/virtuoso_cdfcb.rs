//! Lane L9: design-library CDF callbacks become Schemify `computed` params (spec
//! `interop.md` §6.3), PDK callbacks are never read, and an instance's CDF values,
//! properties and textDisplays survive a redraw (§7.5). The kit is
//! `tests/fixtures/skill/cdfcb_kit.il`.

mod common;
mod skill;

use common::{is_cell, load_tops, located};
use interop::design::Files;
use interop::map::Mapping;
use interop::sfyx;
use interop::virtuoso::{self, ExportOptions, ImportOptions};
use skill::mockdb::MockDb;
use skill::{Interp, V};

const BUILD: &str = include_str!("fixtures/skill/build.il");
const KIT: &str = include_str!("fixtures/skill/cdfcb_kit.il");

fn run(i: &mut Interp<MockDb>, src: &str) -> V {
    i.load(src)
        .unwrap_or_else(|e| panic!("{src}: {e}\n{}", i.log))
}

/// A mock with the kit in `work` and a PDK library `pdk` whose cell `dev` has a plain
/// callback defined in pdk's own libInit.il, instantiated in `work/top`.
fn mock() -> Interp<MockDb> {
    let mut i = skill::virtuoso(&["analogLib", "basic", "pdk", "work"]);
    i.load(BUILD).unwrap();
    i.load(KIT).unwrap();
    run(&mut i, "(SfyFixStdlib)");
    run(&mut i, "(SfyFixCbKit)");
    i.files.insert(
        "/mock/pdk/libInit.il".into(),
        "(procedure (pdkCb) (putpropq (getq cdfgData m) (getq (getq cdfgData w) value) value))\n"
            .into(),
    );
    run(
        &mut i,
        r#"(let ((cdf nil) (cv nil))
             (SfyFixSymbol "pdk" "dev" (list (list "D" "inputOutput" 0 40)))
             (setq cdf (SfyFixCdf "pdk" "dev"))
             (cdfCreateParam cdf ?name "w" ?type "float" ?defValue 1.0 ?prompt "W" ?callback "(pdkCb)")
             (cdfCreateParam cdf ?name "m" ?type "float" ?defValue 1.0 ?prompt "M")
             (cdfSaveCDF cdf)
             (setq cv (dbOpenCellViewByType "work" "top" "schematic" "schematic" "a"))
             (schCreateInst cv (dbOpenCellViewByType "pdk" "dev" "symbol" nil "r") "P0"
               (SfyFixP -200 0) "R0")
             ;; An ILList property: the loader must take linereadstring's first form.
             (dbReplaceProp (car (setof x (getSGq cv instances) (equal (getSGq x name) "I0")))
               "pts" "ILList" (list 1 "a"))
             (dbSave cv) (dbClose cv))"#,
    );
    i
}

fn dump(i: &mut Interp<MockDb>, path: &str) -> String {
    let ok = run(
        i,
        &format!(r#"(SfyExport "{path}" "work" (list "amp" "top"))"#),
    );
    assert!(ok.truthy(), "{}", i.files[&format!("{path}.log")]);
    i.files[path].clone()
}

fn import(text: &str) -> (Files, String) {
    let imp = virtuoso::import(
        text,
        &Files::new(),
        &Mapping::default(),
        &ImportOptions::default(),
    )
    .unwrap_or_else(|e| panic!("{e:?}"));
    let notes = imp.notes.text();
    (imp.files.into_iter().collect(), notes)
}

fn cell_text(files: &Files, lib: &str, cell: &str) -> (String, String) {
    let path = files
        .keys()
        .find(|p| is_cell(p) && p.starts_with(&format!("{lib}/{cell}.")))
        .unwrap_or_else(|| panic!("{lib}/{cell} imported: {:?}", files.keys()))
        .clone();
    let text = String::from_utf8(files[&path].clone()).unwrap();
    (path, text)
}

/// The `param` records of cell `cell` of a dump.
fn params<'a>(text: &'a str, cell: &str) -> Vec<&'a str> {
    let start = text
        .find(&format!("(\"cell\" \"{cell}\" "))
        .unwrap_or_else(|| panic!("{cell} dumped"));
    text[start..]
        .lines()
        .skip(1)
        .take_while(|l| !l.starts_with("(\"cell\"") && !l.starts_with("(\"cv\""))
        .filter(|l| l.starts_with("(\"param\""))
        .collect()
}

#[test]
fn a_float_cdf_value_reads_its_scale_factor() {
    // atof("500n") is 500.0 (live ICADVM20.1); cdfParseFloatString reads 5e-07.
    let mut i = mock();
    let v = run(&mut i, r#"(SfyCdfValue "float" "500n")"#);
    assert!(
        matches!(v, V::Float(f) if (f - 5e-7).abs() < 1e-18),
        "{v:?}"
    );
}

#[test]
fn design_callbacks_dump_with_their_code_and_pdk_ones_never() {
    let mut i = mock();
    let t = dump(&mut i, "/c/first.sfyx");
    let amp = params(&t, "amp");
    let w = amp
        .iter()
        .find(|l| l.starts_with("(\"param\" \"w\""))
        .unwrap();
    assert!(
        w.ends_with(
            "\"(sfyKitWf)\" \"libInit.il\" \"(procedure (sfyKitWf) (putpropq (getq cdfgData wf) (quotient (getq (getq cdfgData w) value) (getq (getq cdfgData nf) value)) value))\")"
        ),
        "{w}"
    );
    // The PDK cell is a reference: its callback is never read, though its library's
    // libInit.il defines it plainly.
    let dev = params(&t, "dev");
    assert!(dev.iter().all(|l| !l.contains("libInit.il")), "{dev:?}");
    assert!(dev[0].ends_with("\"(pdkCb)\")"), "{dev:?}");
}

#[test]
fn the_plain_callback_becomes_computed_and_the_branching_one_stays_virtuosos() {
    let mut i = mock();
    let t = dump(&mut i, "/c/first.sfyx");
    let (files, notes) = import(&t);
    let (_, amp) = cell_text(&files, "work", "amp");
    assert!(amp.contains("computed \"w / nf\""), "{amp}");
    assert_eq!(amp.matches("computed").count(), 1, "{amp}");
    for want in [
        "param w: callback becomes wf computed `w / nf`",
        "param nf: callback becomes wf computed `w / nf`",
        "param l: callback not carried: it branches (`if`)",
        "pdkCb: callbacks of 1 params in 1 cells not carried",
    ] {
        assert!(notes.contains(want), "{want}\n{notes}");
    }
    // The instance's wf is the rule's, not an override (E030).
    let (_, top) = cell_text(&files, "work", "top");
    let i0 = top
        .lines()
        .skip_while(|l| !l.contains("name I0"))
        .take(20)
        .collect::<Vec<_>>();
    assert!(
        !i0.iter().any(|l| l.trim_start().starts_with("wf ")),
        "{top}"
    );
    assert!(
        i0.iter().any(|l| l.trim_start().starts_with("w 3u")),
        "{top}"
    );
}

#[test]
fn export_writes_the_rule_as_a_cel_default_that_evaluates_to_the_callbacks_value() {
    let mut i = mock();
    let t = dump(&mut i, "/c/first.sfyx");
    let (mut files, _) = import(&t);
    // An interface edit: the CDF is derived again (§6.3), not sent back as imported.
    let (path, amp) = cell_text(&files, "work", "amp");
    let edited = amp.replace("doc Width", "doc \"Gate width\"");
    assert_ne!(edited, amp, "{amp}");
    files.insert(path.clone(), edited.into_bytes());
    let d = load_tops(&files, &[path.as_str()]);
    let out = virtuoso::export(&d, &[0], &Mapping::default(), &ExportOptions::default())
        .unwrap_or_else(|e| panic!("{e:?}"))
        .sfyx;
    let wf = params(&out, "amp")
        .into_iter()
        .find(|l| l.starts_with("(\"param\" \"wf\""))
        .unwrap()
        .to_owned();
    let recs = sfyx::read(&format!("(\"sfyx\" 1 \"t\")\n{wf}\n(\"end\")\n")).unwrap();
    let r = &recs[1];
    assert_eq!(r.str(1).unwrap(), "string", "{wf}");
    let cel = r.str(2).unwrap();
    assert_eq!(cel, "iPar(\"w\") / iPar(\"nf\")", "{wf}");
    // The callback's value for three input sets (w / nf, as sfyKitWf computes it).
    for (w, nf) in [(1e-6, 1), (3e-6, 2), (2.5e-6, 7)] {
        let e = cel
            .replace("iPar(\"w\")", &format!("({w:e})"))
            .replace("iPar(\"nf\")", &format!("({nf})"));
        let p = interop::expr::Program::compile(&e, interop::expr::Mode::Scalar).unwrap();
        let got = match p.eval(&[], &mut interop::expr::Scratch::default()).unwrap() {
            interop::expr::Value::Num(x) => x,
            v => panic!("{v:?}"),
        };
        assert_eq!(got, w / f64::from(nf), "{e}");
    }
}

/// The design cells' records of a dump without the `schemify*` properties export adds.
fn design_records(text: &str) -> Vec<String> {
    let recs = sfyx::read(text).unwrap();
    located(&recs)
        .into_iter()
        .filter(|(at, _)| at.design)
        .map(|(_, r)| r.clone())
        .filter(|r| !(r.tag == "prop" && r.str(1).is_ok_and(|n| n.starts_with("schemify"))))
        .map(|r| {
            let mut s = String::new();
            sfyx::write_record(&r, &mut s);
            s.trim_end().to_owned()
        })
        .collect()
}

#[test]
fn a_redraw_keeps_every_instance_value_property_and_textdisplay() {
    let mut i = mock();
    let first = dump(&mut i, "/c/first.sfyx");
    let (mut files, _) = import(&first);
    let (path, top) = cell_text(&files, "work", "top");
    // Move I1 in Schemify: the cellview is redrawn; I0 must come back as it was.
    let edited = top.replacen("at 200 0 R270", "at 240 0 R270", 1);
    assert_ne!(edited, top, "{top}");
    files.insert(path.clone(), edited.into_bytes());
    let d = load_tops(&files, &[path.as_str()]);
    let second = virtuoso::export(
        &d,
        &[0],
        &Mapping::default(),
        &ExportOptions {
            producer: "cdfcb".into(),
            ..ExportOptions::default()
        },
    )
    .unwrap_or_else(|e| panic!("{e:?}"))
    .sfyx;
    i.files.insert("/c/second.sfyx".into(), second.clone());
    let ok = run(&mut i, r#"(SfyImport "/c/second.sfyx" ?overwrite t)"#);
    let log = i.files["/c/second.sfyx.log"].clone();
    assert!(ok.truthy(), "{log}\n{second}");
    assert!(log.contains("count saved "), "{log}");
    let third = dump(&mut i, "/c/third.sfyx");
    let (a, b) = (design_records(&first), design_records(&third));
    let gone: Vec<&String> = a.iter().filter(|l| !b.contains(l)).collect();
    let new: Vec<&String> = b.iter().filter(|l| !a.contains(l)).collect();
    // Only I1's own record moved (its ref and everything else unchanged).
    assert!(
        gone.len() == 1
            && new.len() == 1
            && gone[0].contains("\"I1\"")
            && new[0].contains("\"I1\""),
        "gone {gone:#?}\nnew {new:#?}\nsecond:\n{second}"
    );
}
