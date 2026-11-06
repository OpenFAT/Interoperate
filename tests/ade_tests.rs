//! Maestro views of several tests ⇄ one bench with `test` records (spec `interop.md` §9,
//! lane L6): the probe setup's tests, global and test variables, device parameters,
//! disabled tests and corners, cross-test outputs; a Schemify edit of one test goes back
//! as that test's change only.
//!
//! The fixture tests read the real captured files in `tests/fixtures/cadence/maestro/`
//! (local-only: they skip where it is absent) through the mock maestro.

mod common;
mod skill;

use common::cadence_fixture;
use interop::design::Files;
use interop::map::Mapping;
use interop::sfyx::{self, Record};
use interop::virtuoso::ade::mae_xml;
use interop::virtuoso::{self, ExportOptions, ImportOptions};
use skill::mockdb::MockDb;
use skill::{Interp, V};

const LIB: &str = "sfyprobe_lib";
const TAGS: &str = r#"(list "vars" "tests" "parameters" "corners" "runoptions" "specs")"#;

fn fixture(rel: &str) -> Option<String> {
    Some(std::fs::read_to_string(cadence_fixture(rel)?).unwrap())
}

fn normalized(xml: &str) -> String {
    let mut n = mae_xml::parse(xml).unwrap_or_else(|e| panic!("{e}\n{xml}"));
    mae_xml::normalize(&mut n);
    mae_xml::write(&n)
}

fn run(i: &mut Interp<MockDb>, src: &str) -> V {
    i.load(src)
        .unwrap_or_else(|e| panic!("{src}: {e}\nlog: {}", i.log))
}

fn export(files: &Files, path: &str) -> (String, interop::notes::Notes) {
    let d = common::load_tops(files, &[path]);
    let ci = d.find(path).expect("loaded");
    let opts = ExportOptions {
        producer: "test".into(),
        ..ExportOptions::default()
    };
    let e =
        virtuoso::export(&d, &[ci], &Mapping::default(), &opts).unwrap_or_else(|e| panic!("{e:?}"));
    (e.sfyx, e.notes)
}

fn import(text: &str, existing: &Files) -> (Files, interop::notes::Notes) {
    let imp = virtuoso::import(
        text,
        existing,
        &Mapping::default(),
        &ImportOptions::default(),
    )
    .unwrap_or_else(|e| panic!("{e:?}"));
    let mut out = existing.clone();
    out.extend(imp.files);
    (out, imp.notes)
}

fn manifest() -> Files {
    let mut f = Files::new();
    f.insert("schemify.project".into(), b"schemify 1 project\n".to_vec());
    f
}

/// The records after the first `maestro` but `maesdb`/`maestate`, as `test tag name`.
fn sent(text: &str) -> Vec<String> {
    let mut test = String::new();
    let mut out = Vec::new();
    for r in sfyx::read(text).unwrap() {
        match r.tag.as_str() {
            "maestro" => test = r.str(0).unwrap().to_owned(),
            "maesdb" | "maestate" | "maefile" | "end" => {}
            _ if test.is_empty() => {}
            t => out.push(format!(
                "{test} {t} {}",
                r.str(0).map_or(String::new(), |x| x.to_owned())
            )),
        }
    }
    out
}

/// A mock Virtuoso whose `sfyprobe_lib/rc/maestro` holds the probe's setup (the fixture
/// `active.state` in the view, then the fixture XML imported into it), or `None` without
/// the fixtures.
fn probe() -> Option<(Interp<MockDb>, String)> {
    let xml = fixture("maestro/mae_export.sdb")?;
    let state = fixture("maestro/setup_before_runs/active.state")?;
    let mut i = skill::virtuoso(&[LIB, "analogLib", "basic"]);
    run(
        &mut i,
        &format!(
            r#"(let ((cv (dbOpenCellViewByType "{LIB}" "rc" "schematic" "schematic" "w"))) (dbSave cv) (dbClose cv))"#
        ),
    );
    let path = i.host.add_text_file(LIB, "rc", "maestro", "active.state");
    i.files.insert(path, state);
    i.files.insert("/fix/setup.sdb".into(), xml.clone());
    let ok = run(
        &mut i,
        &format!(
            r#"(let ((s (maeOpenSetup "{LIB}" "rc" "maestro" ?mode "a")))
                 (axlImportSetup s "/fix/setup.sdb" {TAGS} "" 'overwrite)
                 (maeSaveSetup ?lib "{LIB}" ?cell "rc" ?view "maestro" ?session s)
                 (maeCloseSession ?session s ?forceClose t))"#
        ),
    );
    assert!(ok.truthy());
    Some((i, xml))
}

/// The view's setup XML now, as `axlExportSetup` writes it.
fn view_xml(i: &mut Interp<MockDb>) -> String {
    run(
        i,
        &format!(
            r#"(let ((s (maeOpenSetup "{LIB}" "rc" "maestro" ?mode "a")))
                 (axlExportSetup s (axlGetMainSetupDB s) "/fix/now.sdb" {TAGS})
                 (maeCloseSession ?session s ?forceClose t))"#
        ),
    );
    i.files["/fix/now.sdb"].clone()
}

/// The probe view dumped and imported: (Virtuoso, files, the bench's path).
fn through_schemify() -> Option<(Interp<MockDb>, String, Files, String)> {
    let (mut i, xml) = probe()?;
    let ok = run(
        &mut i,
        &format!(r#"(SfyExport "/t/a.sfyx" "{LIB}" (list "rc"))"#),
    );
    assert!(ok.truthy(), "{}", i.files["/t/a.sfyx.log"]);
    let (files, _) = import(&i.files["/t/a.sfyx"].clone(), &manifest());
    Some((i, xml, files, format!("{LIB}/rc.shbench")))
}

/// The probe's three tests come in as `test` records of one bench: T2's own `C`, T3
/// disabled, the parameter globally and in corner hot, corner cold disabled.
#[test]
#[cfg_attr(
    not(feature = "cadence-records"),
    ignore = "needs the bench test/param records of Schemify's cadence-records format"
)]
fn the_probe_tests_become_test_records() {
    let Some((i, _, files, path)) = through_schemify() else {
        return;
    };
    let dump = i.files["/t/a.sfyx"].clone();
    let own: Vec<Record> = sfyx::read(&dump)
        .unwrap()
        .into_iter()
        .filter(|r| r.tag == "testvar")
        .collect();
    assert_eq!(own.len(), 1, "only T2 has a var of its own");
    assert_eq!((own[0].str(0), own[0].str(1)), (Ok("C"), Ok("5n")));
    let bench = String::from_utf8(files[&path].clone()).unwrap();
    for want in [
        "var C 1n\n",
        "var K 7\n",
        "corner cold\n  temp -40\n  enable false\n",
        "corner hot\n  temp 85\n  param V0/v2 1.1\n",
        "param V0/v2 1\n",
        "test T1\n  vnone code output\n",
        "test T2\n  set C 5n\n  vnone code output\n",
        "test T3\n  enable false\n  vnone code output\n",
    ] {
        let want = want.replace("vnone code output", "vnone code\n  vnone output");
        assert!(bench.contains(&want), "{want}\n--- in\n{bench}");
    }
}

/// Unchanged, the bench goes back as the tests' `maestro` records only, and the load
/// leaves the setup as it was (weights, the tolerance spec, run options and all).
#[test]
#[cfg_attr(
    not(feature = "cadence-records"),
    ignore = "needs the bench test/param records of Schemify's cadence-records format"
)]
fn an_unchanged_probe_bench_round_trips_equal_after_normalize() {
    let Some((mut i, xml, files, path)) = through_schemify() else {
        return;
    };
    // A bench that holds the view's files goes back as those alone (tests/mae_files.rs).
    let files = common::without_view_files(&files, &path);
    let (text, notes) = export(&files, &path);
    assert!(
        sent(&text).is_empty(),
        "{:?}\n{}",
        sent(&text),
        notes.text()
    );
    i.files.insert("/t/b.sfyx".into(), text);
    let ok = run(&mut i, r#"(SfyImport "/t/b.sfyx" ?overwrite t)"#);
    let log = i.files["/t/b.sfyx.log"].clone();
    assert!(
        ok.truthy() && log.contains("count maestro restored 3"),
        "{log}"
    );
    assert!(!i.host.mae.calls.iter().any(|c| c == "maeDeleteTest"));
    assert_eq!(normalized(&view_xml(&mut i)), normalized(&xml));
}

/// A Schemify edit of T2's own `C` goes back as that one record, lands as T2's test
/// variable, and changes nothing else of the setup: the global `C` and the other tests
/// stay.
#[test]
#[cfg_attr(
    not(feature = "cadence-records"),
    ignore = "needs the bench test/param records of Schemify's cadence-records format"
)]
fn an_edit_of_one_test_changes_that_test_only() {
    let Some((mut i, xml, mut files, path)) = through_schemify() else {
        return;
    };
    let bench = String::from_utf8(files[&path].clone()).unwrap();
    let edited = bench.replace("  set C 5n\n", "  set C 7n\n");
    assert_ne!(edited, bench);
    files.insert(path.clone(), edited.into_bytes());
    let (text, notes) = export(&files, &path);
    assert_eq!(sent(&text), ["T2 testvar C"], "{}", notes.text());
    i.files.insert("/t/e.sfyx".into(), text);
    let ok = run(&mut i, r#"(SfyImport "/t/e.sfyx" ?overwrite t)"#);
    assert!(ok.truthy(), "{}", i.files["/t/e.sfyx.log"]);
    let want = normalized(&xml).replace("<value>5n</value>", "<value>7n</value>");
    assert_ne!(want, normalized(&xml));
    assert_eq!(normalized(&view_xml(&mut i)), want);
}

/// Enabling T3 and changing hot's parameter in Schemify: a `testenable` and one corner
/// `parameter` record, applied in place.
#[test]
#[cfg_attr(
    not(feature = "cadence-records"),
    ignore = "needs the bench test/param records of Schemify's cadence-records format"
)]
fn enable_and_parameter_edits_go_back_as_their_records() {
    let Some((mut i, xml, mut files, path)) = through_schemify() else {
        return;
    };
    let bench = String::from_utf8(files[&path].clone()).unwrap();
    let edited = bench
        .replace("test T3\n  enable false\n", "test T3\n")
        .replace("  param V0/v2 1.1\n", "  param V0/v2 1.2\n");
    files.insert(path.clone(), edited.into_bytes());
    let (text, notes) = export(&files, &path);
    assert_eq!(
        sent(&text),
        ["T1 parameter V0/v2", "T3 testenable "],
        "{}",
        notes.text()
    );
    i.files.insert("/t/e.sfyx".into(), text);
    let ok = run(&mut i, r#"(SfyImport "/t/e.sfyx" ?overwrite t)"#);
    assert!(ok.truthy(), "{}", i.files["/t/e.sfyx.log"]);
    let now = normalized(&view_xml(&mut i));
    let s = mae_xml::Setup::from_xml(&now).unwrap();
    assert!(s.tests.iter().all(|t| t.enabled), "{now}");
    assert_eq!(s.corner("hot").unwrap().params[0].value, "1.2");
    assert_eq!(s.params[0].value, "1", "the global parameter stays");
    // Everything else is as it was.
    let mut want = mae_xml::parse(&normalized(&xml)).unwrap();
    let active = want.child_mut("active").unwrap();
    active.children.retain(|c| c.name != "disabledtests");
    let w = mae_xml::write(&want).replace("<value>1.1</value>", "<value>1.2</value>");
    assert_eq!(now, w);
}

/// A global a test omits (`dependentTest omitted="1"`) becomes the test's `omit`, and is
/// reported when Schemify changes it (it has no record).
#[test]
#[cfg_attr(
    not(feature = "cadence-records"),
    ignore = "needs the bench test/param records of Schemify's cadence-records format"
)]
fn an_omitted_global_becomes_omit() {
    let Some(xml) = fixture("maestro/mae_export.sdb") else {
        return;
    };
    let omits = xml.replace(
        "<dependentTest omitted=\"0\">T2</dependentTest>",
        "<dependentTest omitted=\"1\">T1</dependentTest>",
    );
    assert_ne!(omits, xml);
    let text = format!(
        "(\"sfyx\" 1 \"t\")\n(\"lib\" \"{LIB}\")\n(\"cell\" \"rc\" \"design\")\n\
         (\"cv\" \"schematic\" \"schematic\" 160)\n(\"maestro\" \"T1\" \"{LIB}\" \"rc\" \"schematic\" \"spectre\")\n\
         (\"maesdb\" {} ())\n(\"var\" \"C\" \"1n\" \"\")\n\
         (\"maestro\" \"T2\" \"{LIB}\" \"rc\" \"schematic\" \"spectre\")\n(\"var\" \"C\" \"1n\" \"\")\n(\"end\")\n",
        sfyx_text(&omits)
    );
    let (files, _) = import(&text, &manifest());
    let path = format!("{LIB}/rc.shbench");
    let bench = String::from_utf8(files[&path].clone()).unwrap();
    assert!(bench.contains("test T1\n  omit C\n"), "{bench}");
    let edited = bench.replace("  omit C\n", "");
    let mut files = files;
    files.insert(path.clone(), edited.into_bytes());
    let (_, notes) = export(&files, &path);
    assert!(
        notes
            .text()
            .contains("test T1 omit : not sent (maestro keeps C)"),
        "{}",
        notes.text()
    );
}

/// `s` as a `.sfyx` text field.
fn sfyx_text(s: &str) -> String {
    let mut out = String::new();
    sfyx::write_record(&Record::new("x", vec![sfyx::text(s)]), &mut out);
    out.trim_start_matches("(\"x\" ")
        .trim_end_matches(")\n")
        .to_owned()
}

/// A cross-test output (`calcVal`) and a dependent one come in as `result()` (reported:
/// runs refuse it until they schedule tests) and go back as Maestro wrote them.
#[test]
#[cfg_attr(
    not(feature = "cadence-records"),
    ignore = "needs the bench test/param records of Schemify's cadence-records format"
)]
fn cross_test_outputs_cross_as_result() {
    let head = "(\"sfyx\" 1 \"t\")\n(\"lib\" \"schemify\")\n(\"cell\" \"tb\" \"design\")\n(\"cv\" \"schematic\" \"schematic\" 160)\n";
    let text = format!(
        "{head}(\"maestro\" \"T1\" \"schemify\" \"tb\" \"schematic\" \"spectre\")\n\
         (\"analysis\" \"tran\" 1 ((\"stop\" \"1u\")))\n(\"output\" \"vmax\" \"point\" \"ymax(VT(\\\"/out\\\"))\")\n\
         (\"output\" \"v2\" \"point\" \"(vmax * 2)\")\n\
         (\"maestro\" \"T2\" \"schemify\" \"tb\" \"schematic\" \"spectre\")\n\
         (\"analysis\" \"ac\" 1 ((\"start\" \"1\") (\"stop\" \"1G\") (\"incrType\" \"Logarithmic\") (\"stepTypeLog\" \"Points Per Decade\") (\"dec\" \"10\")))\n\
         (\"output\" \"x2\" \"point\" \"(calcVal(\\\"vmax\\\" \\\"T1\\\") * 2)\")\n(\"end\")\n"
    );
    let (files, notes) = import(&text, &manifest());
    let bench = String::from_utf8(files["tb.shbench"].clone()).unwrap();
    assert!(
        bench.contains("output v2 \"(result(\\\"vmax\\\", \\\"T1\\\") * 2)\""),
        "{bench}"
    );
    assert!(
        bench.contains("output x2 \"(result(\\\"vmax\\\", \\\"T1\\\") * 2)\""),
        "{bench}"
    );
    assert!(
        bench.contains("test T2\n  code T2\n  output x2\n"),
        "{bench}"
    );
    assert!(notes.text().contains("(X006)"), "{}", notes.text());
    let (back, _) = export(&files, "tb.shbench");
    let outs: Vec<(String, String)> = sfyx::read(&back)
        .unwrap()
        .into_iter()
        .filter(|r| r.tag == "output")
        .map(|r| (r.str(0).unwrap().to_owned(), r.str(2).unwrap().to_owned()))
        .collect();
    let o = |n: &str, e: &str| (n.to_owned(), e.to_owned());
    assert_eq!(
        outs,
        [
            o("v2", "(vmax * 2)"),
            o("vmax", "ymax(VT(\"/out\"))"),
            o("x2", "(calcVal(\"vmax\" \"T1\") * 2)")
        ]
    );
}

/// A bench of two tests made in Schemify loads as two Maestro tests: the second disabled
/// with its own `vdd`, the parameter globally and in corner tt (no fixtures needed).
#[test]
#[cfg_attr(
    not(feature = "cadence-records"),
    ignore = "needs the bench test/param records of Schemify's cadence-records format"
)]
fn a_schemify_bench_of_two_tests_loads_as_two_maestro_tests() {
    let bench = "schemify 1 shbench\n\nvar vdd 1.8\n\ncorner tt\n  temp 27\n  param X1/w 3u\n\n\
                 output vmax ymax(v(out))\n\ntest sim\n  code sim\n  output vmax\n\
                 test t2\n  code sim\n  set vdd 2\n  enable false\n  vnone output\n\n\
                 param X1/w 2u\n\ncode sim spice\n  | .tran 1n 1u\n";
    let d = schemify_format::parse(bench.as_bytes()).unwrap();
    let mut files = manifest();
    files.insert("tb.shbench".into(), schemify_format::write(&d).into_bytes());
    let (text, notes) = export(&files, "tb.shbench");
    let mut i = skill::virtuoso(&["schemify"]);
    i.files.insert("/t/in.sfyx".into(), text);
    let ok = run(&mut i, r#"(SfyImport "/t/in.sfyx")"#);
    assert!(
        ok.truthy(),
        "{}\n{}",
        i.files["/t/in.sfyx.log"],
        notes.text()
    );
    let su = &i.host.mae.saved[&("schemify".to_owned(), "tb".to_owned())];
    let t = |n: &str| su.tests.iter().find(|t| t.name == n).unwrap();
    assert!(!t("sim").disabled && t("t2").disabled);
    assert_eq!(t("t2").vars, [("vdd".to_owned(), "2".to_owned())]);
    assert!(t("sim").vars.is_empty());
    assert_eq!(t("sim").outputs.len(), 1);
    assert!(t("t2").outputs.is_empty());
    let p = |c: &str| {
        su.params
            .iter()
            .find(|p| p.3 == c)
            .map(|p| (p.0.as_str(), p.1.as_str(), p.2.as_str()))
    };
    assert_eq!(p(""), Some(("schemify/tb/schematic/X1", "w", "2u")));
    assert_eq!(p("tt"), Some(("schemify/tb/schematic/X1", "w", "3u")));
}

/// After a run Maestro gives a test an empty placeholder of a global var (found by lane L5):
/// the dumper skips it, so the global's value is what the test sees, not "".
#[test]
fn an_empty_test_var_leaves_the_global() {
    let mut i = skill::virtuoso(&[LIB, "analogLib", "basic"]);
    let ok = run(
        &mut i,
        &format!(
            r#"(let ((cv (dbOpenCellViewByType "{LIB}" "rc" "schematic" "schematic" "w"))
                     (s (maeOpenSetup "{LIB}" "rc" "maestro" ?mode "a")))
                 (dbSave cv) (dbClose cv)
                 (maeCreateTest "T1" ?lib "{LIB}" ?cell "rc" ?view "schematic" ?simulator "spectre" ?session s)
                 (maeSetVar "VDD" "1.8" ?session s)
                 (maeSetVar "VDD" "" ?typeName "test" ?typeValue (list "T1") ?session s)
                 (maeSetVar "C" "5n" ?typeName "test" ?typeValue (list "T1") ?session s)
                 (maeSaveSetup ?lib "{LIB}" ?cell "rc" ?view "maestro" ?session s)
                 (maeCloseSession ?session s ?forceClose t))"#
        ),
    );
    assert!(ok.truthy());
    let ok = run(
        &mut i,
        &format!(r#"(SfyExport "/t/a.sfyx" "{LIB}" (list "rc"))"#),
    );
    assert!(ok.truthy(), "{}", i.files["/t/a.sfyx.log"]);
    let recs = sfyx::read(&i.files["/t/a.sfyx"]).unwrap();
    let of = |tag: &str| {
        recs.iter()
            .filter(|r| r.tag == tag)
            .map(|r| (r.str(0).unwrap().to_owned(), r.str(1).unwrap().to_owned()))
            .collect::<Vec<_>>()
    };
    assert_eq!(of("var"), [("VDD".to_owned(), "1.8".to_owned())]);
    assert_eq!(of("testvar"), [("C".to_owned(), "5n".to_owned())]);
}
