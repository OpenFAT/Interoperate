//! A Maestro view's own setup through Schemify and back (spec `interop.md` §9.7, lane L1):
//! the setup XML (`axlExportSetup`) and the test states (`active.state`) are dumped as
//! `maesdb`/`maestate`, kept in the bench header, sent back, and restored by
//! `axlImportSetup … 'overwrite` with only Schemify's changes applied on top.
//!
//! The fixture tests read the real captured files in `tests/fixtures/cadence/maestro/`
//! (local-only: they skip where it is absent); the rest run on the mock maestro.

mod common;
mod skill;

use common::cadence_fixture;
use interop::design::Files;
use interop::map::Mapping;
use interop::sfyx::{self, Record};
use interop::virtuoso::ade::mae_xml::{self, Setup};
use interop::virtuoso::{self, ExportOptions, ImportOptions};
use skill::mockdb::MockDb;
use skill::{Interp, V};

fn fixture(rel: &str) -> Option<String> {
    let p = cadence_fixture(rel)?;
    Some(std::fs::read_to_string(p).unwrap())
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

fn records(text: &str, tag: &str) -> Vec<Record> {
    sfyx::read(text)
        .unwrap()
        .into_iter()
        .filter(|r| r.tag == tag)
        .collect()
}

/// The `.sfyx` export of the bench at `path` in `files`.
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

fn import(text: &str, existing: &Files) -> Files {
    let imp = virtuoso::import(
        text,
        existing,
        &Mapping::default(),
        &ImportOptions::default(),
    )
    .unwrap_or_else(|e| panic!("{e:?}"));
    let mut out = existing.clone();
    out.extend(imp.files);
    out
}

// ---------------------------------------------------------------------------------------
// The XML reader on Maestro's own files
// ---------------------------------------------------------------------------------------

#[test]
fn maestro_files_print_back_byte_for_byte() {
    for rel in [
        "maestro/mae_export.sdb",
        "maestro/mae_import_full_overwrite.sdb",
        "maestro/mae_import_empty_overwrite.sdb",
        "maestro/setup_before_runs/maestro.sdb",
        "maestro/setup_before_runs/active.state",
        "maestro/view_after/active.state",
    ] {
        let Some(x) = fixture(rel) else { return };
        let n = mae_xml::parse(&x).unwrap_or_else(|e| panic!("{rel}: {e}"));
        assert!(mae_xml::write(&n) == x, "{rel}: printed back differs");
    }
}

#[test]
fn the_typed_setup_reads_every_part_of_the_probe_setup() {
    let Some(x) = fixture("maestro/mae_export.sdb") else {
        return;
    };
    let s = Setup::from_xml(&x).unwrap();
    let names: Vec<&str> = s.tests.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["T1", "T2", "T3"]);
    assert!(!s.test("T3").unwrap().enabled && s.test("T1").unwrap().enabled);
    assert_eq!(s.test("T1").unwrap().option("lib"), Some("sfyprobe_lib"));
    assert_eq!(s.test("T2").unwrap().vars, [("C".into(), "5n".into())]);
    assert_eq!(s.var("C").unwrap().dependents, [("T2".into(), false)]);
    assert_eq!(s.var("rmc").unwrap().value, "1k*(1+0.05*dmc)");
    assert_eq!(s.params[0].location, "sfyprobe_lib/rc/schematic/V0");
    assert_eq!(
        (s.params[0].name.as_str(), s.params[0].value.as_str()),
        ("v2", "1")
    );
    let hot = s.corner("hot").unwrap();
    assert!(hot.enabled && !s.corner("cold").unwrap().enabled);
    assert_eq!(hot.vars, [("temperature".into(), "85".into())]);
    assert_eq!(hot.params[0].value, "1.1");
    let vmin = s.spec("T1", "vmin").unwrap();
    assert_eq!(
        (vmin.kind.as_str(), vmin.target.as_str(), vmin.tol.as_str()),
        ("tol", "0.1", "10%")
    );
    assert_eq!(s.spec("T2", "gain").unwrap().weight.as_deref(), Some("2"));
    assert_eq!(s.spec("T1", "vmax").unwrap().weight, None);
    let mc = s.run_mode("Monte Carlo Sampling").unwrap();
    let names: Vec<&str> = mc.options.iter().map(|o| o.0.as_str()).collect();
    assert_eq!(
        names,
        [
            "mcnumpoints",
            "mcmethod",
            "samplingmode",
            "montecarloseed",
            "mcnumbins",
            "saveallplots"
        ]
    );
    assert_eq!(mc.option("samplingmode"), Some("lhs"));
    assert_eq!(
        s.run_mode("Local Optimization").unwrap().option("effort"),
        Some("coarse")
    );
    let sdb = fixture("maestro/setup_before_runs/maestro.sdb").unwrap();
    assert_eq!(
        Setup::from_xml(&sdb).unwrap().current_mode.as_deref(),
        Some("Monte Carlo Sampling")
    );
}

/// G32: what an `axlImportSetup … 'overwrite` changes, into a full view or an empty one,
/// is only what `normalize` removes.
#[test]
fn an_overwrite_import_reexports_the_same_setup_modulo_normalisation() {
    let Some(x) = fixture("maestro/mae_export.sdb") else {
        return;
    };
    let want = normalized(&x);
    for rel in [
        "maestro/mae_import_full_overwrite.sdb",
        "maestro/mae_import_empty_overwrite.sdb",
    ] {
        assert_eq!(normalized(&fixture(rel).unwrap()), want, "{rel}");
    }
    // And normalisation keeps what is meaning: T2's own C and its dependent line.
    assert!(want.contains("<dependentTest omitted=\"0\">T2</dependentTest>"));
    assert!(want.contains("<value>5n</value>"));
}

#[test]
fn active_state_cuts_into_its_tests() {
    let Some(st) = fixture("maestro/setup_before_runs/active.state") else {
        return;
    };
    let t = mae_xml::state_tests(&st).unwrap();
    let names: Vec<&str> = t.iter().map(|x| x.0.as_str()).collect();
    assert_eq!(names, ["T1", "T2", "T3"]);
    for (n, el) in &t {
        assert!(el.starts_with(&format!("\t<Test Name=\"{n}\">\n")) && el.ends_with("\t</Test>\n"));
        assert!(st.contains(el.as_str()));
    }
}

// ---------------------------------------------------------------------------------------
// The real probe setup through the mock: dump, Schemify, load
// ---------------------------------------------------------------------------------------

const LIB: &str = "sfyprobe_lib";

/// A mock Virtuoso whose `sfyprobe_lib/rc/maestro` holds the probe's setup: the fixture
/// `active.state` in the view, then the fixture XML imported into it.
fn probe_view(xml: &str, state: &str) -> Interp<MockDb> {
    let mut i = skill::virtuoso(&[LIB, "analogLib", "basic"]);
    run(
        &mut i,
        &format!(
            r#"(let ((cv (dbOpenCellViewByType "{LIB}" "rc" "schematic" "schematic" "w"))) (dbSave cv) (dbClose cv))"#
        ),
    );
    let path = i.host.add_text_file(LIB, "rc", "maestro", "active.state");
    i.files.insert(path, state.to_owned());
    i.files.insert("/fix/setup.sdb".into(), xml.to_owned());
    let ok = run(
        &mut i,
        &format!(
            r#"(let ((s (maeOpenSetup "{LIB}" "rc" "maestro" ?mode "a")))
                 (axlImportSetup s "/fix/setup.sdb" (list "vars" "tests" "parameters" "corners" "runoptions" "specs") "" 'overwrite)
                 (maeSetCurrentRunMode ?runMode "Monte Carlo Sampling" ?session s)
                 (maeSaveSetup ?lib "{LIB}" ?cell "rc" ?view "maestro" ?session s)
                 (maeCloseSession ?session s ?forceClose t))"#
        ),
    );
    assert!(ok.truthy());
    i
}

/// The dump's setup XML without the run mode the dumper adds (an export leaves it out).
fn without_mode(xml: &str) -> String {
    xml.lines()
        .filter(|l| l.trim() != "<currentmode>Monte Carlo Sampling</currentmode>")
        .map(|l| format!("{l}\n"))
        .collect()
}

/// The view's current setup XML and `active.state`, as `axlExportSetup` and the file give
/// them.
fn view_now(i: &mut Interp<MockDb>) -> (String, String) {
    run(
        i,
        &format!(
            r#"(let ((s (maeOpenSetup "{LIB}" "rc" "maestro" ?mode "a")))
                 (axlExportSetup s (axlGetMainSetupDB s) "/fix/now.sdb" (list "vars" "tests" "parameters" "corners" "runoptions" "specs"))
                 (maeCloseSession ?session s ?forceClose t))"#
        ),
    );
    let st = i.files[&format!("/mock/{LIB}/rc/maestro/active.state")].clone();
    (i.files["/fix/now.sdb"].clone(), st)
}

#[test]
#[cfg_attr(
    not(feature = "cadence-records"),
    ignore = "needs the bench test/param records of Schemify's cadence-records format"
)]
fn the_probe_setup_survives_dump_schemify_and_load() {
    let (Some(xml), Some(state)) = (
        fixture("maestro/mae_export.sdb"),
        fixture("maestro/setup_before_runs/active.state"),
    ) else {
        return;
    };
    let mut i = probe_view(&xml, &state);

    // The dump carries the setup XML (root text fixed) and every test's state.
    let ok = run(
        &mut i,
        &format!(r#"(SfyExport "/t/a.sfyx" "{LIB}" (list "rc"))"#),
    );
    assert!(ok.truthy(), "{}", i.files["/t/a.sfyx.log"]);
    let first = i.files["/t/a.sfyx"].clone();
    let sdb = records(&first, "maesdb");
    assert_eq!(sdb.len(), 1);
    let dumped = sdb[0].text(0).unwrap();
    assert!(dumped.contains("<setupdb version=\"6\">maesdb\n"));
    assert!(dumped.contains("\t\t<currentmode>Monte Carlo Sampling</currentmode>\n"));
    assert_eq!(normalized(&without_mode(&dumped)), normalized(&xml));
    let states: Vec<(String, String)> = records(&first, "maestate")
        .iter()
        .map(|r| (r.str(0).unwrap().to_owned(), r.text(1).unwrap()))
        .collect();
    assert_eq!(states, mae_xml::state_tests(&state).unwrap());

    // Schemify keeps them in the bench header.
    let mut man = Files::new();
    man.insert("schemify.project".into(), b"schemify 1 project\n".to_vec());
    let files = import(&first, &man);
    let bench_path = format!("{LIB}/rc.shbench");
    let bench = String::from_utf8(files[&bench_path].clone()).unwrap();
    assert!(
        bench.contains("  vmaesdb ") && bench.matches("  vmaestate ").count() == 3,
        "{bench}"
    );

    // Sent back unchanged: the setup and the states, and no mapped record (nothing changed).
    // (A bench that holds the view's files goes back as those alone: tests/mae_files.rs.)
    let files = common::without_view_files(&files, &bench_path);
    let (second, notes) = export(&files, &bench_path);
    assert_eq!(records(&second, "maesdb")[0].text(0).unwrap(), dumped);
    assert_eq!(records(&second, "maestate").len(), 3);
    let mapped: Vec<String> = sfyx::read(&second)
        .unwrap()
        .iter()
        .skip_while(|r| r.tag != "maestro")
        .filter(|r| {
            !matches!(
                r.tag.as_str(),
                "maestro" | "maesdb" | "maestate" | "maefile" | "end"
            )
        })
        .map(|r| r.tag.clone())
        .collect();
    assert!(
        mapped.is_empty(),
        "unchanged records sent: {mapped:?}\n{}",
        notes.text()
    );

    // Loaded over the view: restored in place, no test deleted or created again.
    i.host.mae.calls.clear();
    i.files.insert("/t/b.sfyx".into(), second);
    let ok = run(&mut i, r#"(SfyImport "/t/b.sfyx" ?overwrite t)"#);
    let log = i.files["/t/b.sfyx.log"].clone();
    assert!(ok.truthy(), "{log}");
    // Every test of the view, in place (lane L6: each is a test of the bench).
    assert!(log.contains("count maestro restored 3"), "{log}");
    let calls = &i.host.mae.calls;
    assert!(calls.iter().any(|c| c == "axlImportSetup"));
    assert!(
        !calls
            .iter()
            .any(|c| c == "maeDeleteTest" || c == "maeCreateTest"),
        "{calls:?}"
    );
    let (after, st) = view_now(&mut i);
    assert_eq!(normalized(&after), normalized(&xml));
    assert_eq!(
        mae_xml::state_tests(&st).unwrap(),
        mae_xml::state_tests(&state).unwrap()
    );

    // Into a library without the view: the states and the setup are restored from nothing.
    let mut j = skill::virtuoso(&[LIB, "analogLib", "basic"]);
    j.files
        .insert("/t/b.sfyx".into(), i.files["/t/b.sfyx"].clone());
    let ok = run(&mut j, r#"(SfyImport "/t/b.sfyx")"#);
    assert!(ok.truthy(), "{}", j.files["/t/b.sfyx.log"]);
    let (after, st) = view_now(&mut j);
    assert_eq!(normalized(&after), normalized(&xml));
    assert_eq!(
        mae_xml::state_tests(&st).unwrap(),
        mae_xml::state_tests(&state).unwrap()
    );
    let su = &j.host.mae.saved[&(LIB.to_owned(), "rc".to_owned())];
    assert_eq!(su.run_mode.as_deref(), Some("Monte Carlo Sampling"));
}

#[test]
fn a_load_into_another_library_names_it_in_the_restored_setup() {
    let (Some(xml), Some(state)) = (
        fixture("maestro/mae_export.sdb"),
        fixture("maestro/setup_before_runs/active.state"),
    ) else {
        return;
    };
    let mut i = probe_view(&xml, &state);
    run(
        &mut i,
        &format!(r#"(SfyExport "/t/a.sfyx" "{LIB}" (list "rc"))"#),
    );
    let mut j = skill::virtuoso(&["other", "analogLib", "basic"]);
    j.files
        .insert("/t/a.sfyx".into(), i.files["/t/a.sfyx"].clone());
    let ok = run(&mut j, r#"(SfyImport "/t/a.sfyx" ?lib "other")"#);
    assert!(ok.truthy(), "{}", j.files["/t/a.sfyx.log"]);
    let st = j.files["/mock/other/rc/maestro/active.state"].clone();
    assert!(st.contains("(\"other\" \"rc\" \"schematic\" \"spectre\")"));
    run(
        &mut j,
        r#"(let ((s (maeOpenSetup "other" "rc" "maestro" ?mode "a")))
             (axlExportSetup s (axlGetMainSetupDB s) "/t/o.sdb" (list "vars"))
             (maeCloseSession ?session s ?forceClose t))"#,
    );
    let s = Setup::from_xml(&j.files["/t/o.sdb"]).unwrap();
    assert!(s.tests.iter().all(|t| t.option("lib") == Some("other")));
    assert_eq!(s.params[0].location, "other/rc/schematic/V0");
}

// ---------------------------------------------------------------------------------------
// A Schemify bench through Virtuoso and back, edited on the way
// ---------------------------------------------------------------------------------------

const BENCH: &str = "schemify 1 shbench

var vdd 1.8
var R 1k

corner ff
  set vdd 1.95
  temp -40

output vmax \"ymax(v(out))\"

spec vmax
  min 1
  max 1.7

code sim spice
  | .include /pdk/top.inc
  | .tran 1n 1u
";

fn bench_project(text: &str) -> Files {
    let d = schemify_format::parse(text.as_bytes()).unwrap();
    let mut f = Files::new();
    f.insert("schemify.project".into(), b"schemify 1 project\n".to_vec());
    f.insert("tb.shbench".into(), schemify_format::write(&d).into_bytes());
    f
}

/// The bench loaded into the mock, the view then given what Schemify does not map (a spec
/// weight, a test the bench does not have), dumped, and imported back: (Virtuoso, files).
fn through_virtuoso() -> (Interp<MockDb>, Files) {
    let files = bench_project(BENCH);
    let (text, _) = export(&files, "tb.shbench");
    let mut i = skill::virtuoso(&["schemify"]);
    i.files.insert("/t/in.sfyx".into(), text);
    let ok = run(&mut i, r#"(SfyImport "/t/in.sfyx")"#);
    assert!(ok.truthy(), "{}", i.files["/t/in.sfyx.log"]);
    let ok = run(
        &mut i,
        r#"(let ((s (maeOpenSetup "schemify" "tb" "maestro" ?mode "a")))
             (maeSetSpec "vmax" "sim" ?range (list "1" "1.7") ?weight "4" ?session s)
             (maeCreateTest "zz" ?lib "schemify" ?cell "tb" ?view "schematic" ?simulator "spectre" ?session s)
             (maeSetAnalysis "zz" "ac" ?enable t ?options (list (list "start" "1")) ?session s)
             (maeSaveSetup ?lib "schemify" ?cell "tb" ?view "maestro" ?session s)
             (maeCloseSession ?session s ?forceClose t))"#,
    );
    assert!(ok.truthy());
    run(&mut i, r#"(SfyExport "/t/d.sfyx" "schemify" (list "tb"))"#);
    let files = import(&i.files["/t/d.sfyx"].clone(), &files);
    (i, files)
}

#[test]
#[cfg_attr(
    not(feature = "cadence-records"),
    ignore = "needs the bench test/param records of Schemify's cadence-records format"
)]
fn only_what_schemify_changed_is_applied_and_the_rest_is_restored() {
    let (mut i, mut files) = through_virtuoso();
    // Edit one var in Schemify.
    let bench = String::from_utf8(files["tb.shbench"].clone()).unwrap();
    assert!(bench.contains("var R 1k"), "{bench}");
    files.insert(
        "tb.shbench".into(),
        bench.replace("var R 1k", "var R 2.2k").into_bytes(),
    );
    let (text, notes) = export(&files, "tb.shbench");
    let sent: Vec<String> = sfyx::read(&text)
        .unwrap()
        .iter()
        .skip_while(|r| r.tag != "maestro")
        .filter(|r| !matches!(r.tag.as_str(), "maesdb" | "maestate" | "maefile" | "end"))
        .map(|r| format!("{} {}", r.tag, r.str(0).unwrap_or("")))
        .collect();
    // Both tests see the global (lane L6: zz is a test of the bench too).
    assert_eq!(
        sent,
        ["maestro sim", "var R", "maestro zz", "var R"],
        "{}",
        notes.text()
    );

    i.host.mae.calls.clear();
    i.files.insert("/t/e.sfyx".into(), text);
    let ok = run(&mut i, r#"(SfyImport "/t/e.sfyx" ?overwrite t)"#);
    let log = i.files["/t/e.sfyx.log"].clone();
    assert!(ok.truthy(), "{log}");
    let calls = &i.host.mae.calls;
    assert!(
        !calls
            .iter()
            .any(|c| c == "maeDeleteTest" || c == "maeCreateTest"),
        "{calls:?}"
    );
    let su = i.host.mae.saved[&("schemify".to_owned(), "tb".to_owned())].clone();
    let var = |n: &str| su.vars.iter().find(|v| v.0 == n).map(|v| v.1.as_str());
    assert_eq!((var("R"), var("vdd")), (Some("2.2k"), Some("1.8")));
    // What the bench does not carry is still there: the other test with its analysis, the
    // spec's weight, the analyses and outputs of the mapped test.
    let other = su
        .tests
        .iter()
        .find(|t| t.name == "zz")
        .expect("the other test kept");
    assert_eq!(other.analyses[0].0, "ac");
    assert_eq!(su.weights.get("sim.vmax").map(String::as_str), Some("4"));
    let sim = su.tests.iter().find(|t| t.name == "sim").unwrap();
    assert_eq!(sim.outputs.len(), 1);
    assert!(sim.analyses.iter().any(|a| a.0 == "tran"));
}

#[test]
fn a_changed_spec_keeps_its_weight_and_a_removed_var_is_reported_not_deleted() {
    let (mut i, mut files) = through_virtuoso();
    let bench = String::from_utf8(files["tb.shbench"].clone()).unwrap();
    let edited = bench
        .replace("  max 1.7", "  max 1.6")
        .replace("var R 1k\n", "");
    assert_ne!(edited, bench);
    files.insert("tb.shbench".into(), edited.into_bytes());
    let (text, notes) = export(&files, "tb.shbench");
    assert!(
        notes.text().contains("maestro var R  is not in the bench"),
        "{}",
        notes.text()
    );
    i.files.insert("/t/e.sfyx".into(), text);
    let ok = run(&mut i, r#"(SfyImport "/t/e.sfyx" ?overwrite t)"#);
    assert!(ok.truthy(), "{}", i.files["/t/e.sfyx.log"]);
    let su = i.host.mae.saved[&("schemify".to_owned(), "tb".to_owned())].clone();
    let sim = su.tests.iter().find(|t| t.name == "sim").unwrap();
    let vmax = sim.outputs.iter().find(|o| o.name == "vmax").unwrap();
    assert_eq!(vmax.spec[0].0, "range");
    assert_eq!(vmax.spec[0].1.items()[1].as_str(), Some("1.6"));
    assert_eq!(su.weights.get("sim.vmax").map(String::as_str), Some("4"));
    assert!(su.vars.iter().any(|v| v.0 == "R"), "Maestro keeps R");
}

/// §0: a design variable that a model file of the test also defines as a parameter fails
/// every Spectre point; the loader refuses it.
#[test]
fn a_design_variable_named_like_a_model_parameter_is_refused() {
    let files = bench_project(BENCH);
    let (text, _) = export(&files, "tb.shbench");
    let mut i = skill::virtuoso(&["schemify"]);
    i.files.insert(
        "/pdk/top.inc".into(),
        "simulator lang=spectre\nparameters vdd=1.8 \\\n  k = 2\n".into(),
    );
    i.files.insert("/t/in.sfyx".into(), text);
    let ok = run(&mut i, r#"(SfyImport "/t/in.sfyx")"#);
    let log = i.files["/t/in.sfyx.log"].clone();
    assert!(!ok.truthy(), "{log}");
    assert!(
        log.contains("design variable vdd is also a parameter"),
        "{log}"
    );
    assert!(!log.contains("design variable R "), "{log}");
    let su = i.host.mae.saved[&("schemify".to_owned(), "tb".to_owned())].clone();
    assert!(su.vars.iter().all(|v| v.0 != "vdd") && su.vars.iter().any(|v| v.0 == "R"));
}

#[test]
fn the_mock_getters_take_live_argument_shapes() {
    let mut i = skill::virtuoso(&["schemify"]);
    run(
        &mut i,
        r#"(let ((s (maeOpenSetup "schemify" "tb" "maestro" ?mode "a")))
             (maeCreateTest "t" ?lib "schemify" ?cell "tb" ?view "schematic" ?simulator "spectre" ?session s)
             (maeAddOutput "o" "t" ?outputType "point" ?expr "1" ?session s)
             (maeSetCorner "c" ?session s)
             (maeSetVar "x" "1" ?typeName "corner" ?typeValue (list "c") ?session s))"#,
    );
    assert!(
        i.load(r#"(maeGetVar "x" ?typeName "corner" ?typeValue (list "c"))"#)
            .is_err()
    );
    assert_eq!(
        run(
            &mut i,
            r#"(maeGetVar "x" ?typeName "corner" ?typeValue "c")"#
        )
        .as_str(),
        Some("1")
    );
    assert!(i.load(r#"(maeSetSpec "o" "t" ?tolerance "0.5")"#).is_err());
    run(
        &mut i,
        r#"(maeSetSpec "o" "t" ?tolerance (list "0.5" "10%"))"#,
    );
    let d = run(
        &mut i,
        r#"(axlGetSpecData (axlGetMainSetupDB "fnxSession0") "o" "t")"#,
    );
    let d: Vec<&str> = d.items().iter().filter_map(V::as_str).collect();
    assert_eq!(d, ["tol", "0.5", "10%"]);
    assert!(
        i.load(r#"(axlImportSetup "fnxSession0" "/x.sdb" (list "vars") "" 'append)"#)
            .is_err()
    );
}

/// The live check's files (`live/checks/l1_mae_xml.il` copies them to its out directory):
/// `SFY_L1_LIVE=<dir> cargo test --test mae_restore live_outputs -- --ignored`. The setup
/// XML after each load equals the one before modulo `normalize`. The test states reach a
/// fixed point after the first load: Maestro fills in defaults of a state it reads once
/// (severities `"None"`, an output's `uniqueName`, a `devCheckingSetup` component, an
/// `allcap` info option, live ICADVM20.1), and every later restore gives the same states.
#[test]
#[ignore]
fn live_outputs_compare_modulo_normalisation() {
    let dir = std::path::PathBuf::from(std::env::var("SFY_L1_LIVE").expect("SFY_L1_LIVE"));
    let read = |f: &str| std::fs::read_to_string(dir.join(f)).unwrap();
    let before = normalized(&read("l1_before.sdb"));
    let first = mae_xml::state_tests(&read("l1_after1.state")).unwrap();
    let names = |v: &[(String, String)]| v.iter().map(|x| x.0.clone()).collect::<Vec<_>>();
    assert_eq!(
        names(&first),
        names(&mae_xml::state_tests(&read("l1_before.state")).unwrap())
    );
    for k in 1..=3 {
        let mut after = normalized(&read(&format!("l1_after{k}.sdb")));
        if k == 3 {
            after = after.replace("<var>K\n\t\t\t\t<value>8<", "<var>K\n\t\t\t\t<value>7<");
        }
        assert_eq!(after, before, "after{k}.sdb");
        let now = mae_xml::state_tests(&read(&format!("l1_after{k}.state"))).unwrap();
        assert_eq!(now, first, "after{k}.state");
    }
}
