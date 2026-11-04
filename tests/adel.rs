//! ADE L states through Schemify and back (spec `interop.md` §9.8, lane L7): a state's files
//! dumped verbatim (`SfyAdeLExport`), read as data into a bench like a Maestro test, kept
//! in the bench header, and written back byte for byte (`SfyAdeLImport`) while the bench
//! is unchanged; a changed bench goes to Maestro with its variables.
//!
//! The fixture tests read the probe's real states in `tests/fixtures/cadence/adel/`
//! (local-only: they skip where it is absent); the rest run on hand-written states.

mod common;
mod skill;

use std::collections::BTreeMap;
use std::path::Path;

use common::cadence_fixture;
use interop::design::Files;
use interop::map::Mapping;
use interop::sfyx::{self, Record};
use interop::virtuoso::{self, ExportOptions, ImportOptions};
use skill::mockdb::MockDb;
use skill::{Interp, V};

const LIB: &str = "sfyprobe_lib";

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

/// The files of `dir`, by name.
fn dir_files(dir: &Path) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        if e.path().is_file() {
            let n = e.file_name().to_string_lossy().into_owned();
            out.insert(n, std::fs::read_to_string(e.path()).unwrap());
        }
    }
    out
}

/// A mock Virtuoso with an empty `LIB/rc` schematic (the mock puts the library at
/// `/mock/LIB`).
fn virtuoso() -> Interp<MockDb> {
    let mut i = skill::virtuoso(&[LIB, "analogLib", "basic"]);
    run(
        &mut i,
        &format!(
            r#"(let ((cv (dbOpenCellViewByType "{LIB}" "rc" "schematic" "schematic" "w"))) (dbSave cv) (dbClose cv))"#
        ),
    );
    i
}

/// `files` put at `dir` in the mock's files.
fn put(i: &mut Interp<MockDb>, dir: &str, files: &BTreeMap<String, String>) {
    for (n, t) in files {
        i.files.insert(format!("{dir}/{n}"), t.clone());
    }
}

/// What the mock holds under `dir`, by name.
fn under(i: &Interp<MockDb>, dir: &str) -> BTreeMap<String, String> {
    i.files
        .iter()
        .filter_map(|(k, v)| Some((k.strip_prefix(&format!("{dir}/"))?.to_owned(), v.clone())))
        .filter(|(k, _)| !k.contains('/'))
        .collect()
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

fn project() -> Files {
    let mut f = Files::new();
    f.insert("schemify.project".into(), b"schemify 1 project\n".to_vec());
    f
}

const BENCH: &str = "sfyprobe_lib/rc.shbench";

/// Dumps state `state` of `LIB/rc` from the mock (`from` "dir" under `/states`, else
/// "cellview"), checks the log, and returns the `.sfyx`.
fn dump(i: &mut Interp<MockDb>, state: &str, from: &str) -> String {
    let ok = run(
        i,
        &format!(
            r#"(SfyAdeLExport "/t/a.sfyx" "{LIB}" "rc" "{state}" ?from "{from}" ?stateDir "/states")"#
        ),
    );
    assert!(ok.truthy(), "{}", i.files["/t/a.sfyx.log"]);
    i.files["/t/a.sfyx"].clone()
}

/// Loads `text` with `SfyAdeLImport` (overwrite), checks the log and returns it.
fn load(i: &mut Interp<MockDb>, text: &str) -> String {
    i.files.insert("/t/b.sfyx".into(), text.to_owned());
    let ok = run(i, r#"(SfyAdeLImport "/t/b.sfyx" ?overwrite t)"#);
    let log = i.files["/t/b.sfyx.log"].clone();
    assert!(
        ok.truthy(),
        "{log}\n{}",
        i.files["/t/b.sfyx.cells.sfyx.log"]
    );
    log
}

// ---------------------------------------------------------------------------------------
// A hand-written state: every mapped component
// ---------------------------------------------------------------------------------------

fn state(vars: &[(&str, &str)]) -> BTreeMap<String, String> {
    let mut v = String::from("savedComponent = (let\n(tmp1 tmp2)\n");
    for (k, (n, e)) in vars.iter().enumerate() {
        let t = format!("tmp{}", k + 1);
        v.push_str(&format!(
            "(unless {t}\n{t} = make_sevVariableStruct()\n)\n{t}->name = \"{n}\"\n{t}->expression = \"{e}\"\n{t}->index = {}\n",
            k + 1
        ));
    }
    v.push_str("list(tmp1 tmp2)\n)\n");
    let out = "_outputList = nil\nsavedComponent = (let\n(tmp1)\n(unless tmp1\ntmp1 = make_sevOutputStruct()\n)\ntmp1->name = \"vmax\"\ntmp1->expression = (let\n(tmp1 tmp2)\ntmp1 = 'ymax\ntmp2 = (let\n(tmp1 tmp2)\ntmp1 = 'VT\ntmp2 = \"/out\"\nlist(tmp1 tmp2)\n)\nlist(tmp1 tmp2)\n)\ntmp1->signal = nil\nlist(tmp1)\n)\n_outputList = nconc( _outputList savedComponent) \nsavedComponent = list( _outputList nil ) \n_outputList = 'unbound\n";
    [
        ("ADE_state.info", "projectDir = '\"/sim\"\ndesignInfo = '(\"sfyprobe_lib\" \"rc\" \"schematic\" \"spectre\")\ndescriptionText = \"hand\"\n"),
        (".sevSaveDir", ""),
        ("variables", &v),
        ("analyses", "analysis(tran fields enable) (t)\nanalysis(tran fields stop) \"20u\"\nanalysis(tran fields binnum) \"16\"\nanalysis(tran options errpreset) \"\"\nanalysis(tran options finalTimeOp) t\n"),
        ("outputs", out),
        ("modelSetup", "((\"/pdk/m.scs\" \"tt\"))\n"),
        ("simulatorOptions", "(opts temp) \"85\"\n(opts gmin) \"\"\n"),
        ("simulationFiles", "definitionFiles nil\nincludePath \"\"\n"),
        ("cosimOptions", "(cosim enable) nil\n"),
        ("graphicalStimuli", "\"#INPUTS\"\n\"#GLOBALS\"\n"),
        ("opPoints", "_opPointKeepList = nil\nsavedComponent = _opPointKeepList \n_opPointKeepList = 'unbound\n"),
    ]
    .into_iter()
    .map(|(a, b)| (a.to_owned(), b.to_owned()))
    .collect()
}

const DIR: &str = "/states/sfyprobe_lib/rc/spectre/st1";

#[test]
fn a_state_maps_onto_the_bench_like_a_maestro_test() {
    let mut i = virtuoso();
    let st = state(&[("C", "1n"), ("R", "2k")]);
    put(&mut i, DIR, &st);
    let text = dump(&mut i, "st1", "dir");
    let head = records(&text, "adel");
    assert_eq!(head.len(), 1);
    assert_eq!(
        head[0].fields,
        ["st1", LIB, "rc", "spectre", "dir", "/states"]
            .map(sfyx::s)
            .to_vec()
    );
    assert_eq!(records(&text, "adelfile").len(), st.len());

    let (files, notes) = import(&text, &project());
    let bench = String::from_utf8(files[BENCH].clone()).unwrap();
    for want in [
        "var C 1n",
        "var R 2k",
        "output vmax",
        "code st1 spice",
        "  | .lib /pdk/m.scs tt",
        "  | .temp 85",
        "  | .tran 2e-8 20u",
        "  vadel st1 sfyprobe_lib rc spectre dir /states ",
    ] {
        assert!(bench.contains(want), "{want}\n{bench}");
    }
    assert_eq!(bench.matches("  vadelfile ").count(), st.len(), "{bench}");
    let n = notes.text();
    assert!(
        n.contains("not mapped, kept verbatim for the way back while the bench is unchanged: cosimOptions graphicalStimuli opPoints"),
        "{n}"
    );
}

#[test]
fn an_unchanged_bench_gives_the_state_back_byte_for_byte() {
    for from in ["dir", "cellview"] {
        let dir = if from == "dir" {
            DIR.to_owned()
        } else {
            format!("/mock/{LIB}/rc/st1")
        };
        let mut i = virtuoso();
        let mut st = state(&[("C", "1n")]);
        if from == "cellview" {
            st = st
                .into_iter()
                .map(|(k, v)| match k.as_str() {
                    "ADE_state.info" | ".sevSaveDir" => (k, v),
                    _ => (format!("{k}.state"), v),
                })
                .collect();
            st.remove(".sevSaveDir");
            st.insert(
                "master.tag".into(),
                "-- Master.tag File, Rev:1.0\nADE_state.info\n".into(),
            );
        }
        put(&mut i, &dir, &st);
        let text = dump(&mut i, "st1", from);
        let (files, _) = import(&text, &project());

        let (back, notes) = export(&files, BENCH);
        assert!(records(&back, "maestro").is_empty(), "{back}");
        assert_eq!(records(&back, "adel")[0].str(4).unwrap(), from);
        assert!(!notes.text().contains("ADE L"), "{}", notes.text());

        // Into a Virtuoso that lost the state: every file comes back, byte for byte.
        for f in under(&i, &dir).keys() {
            i.files.remove(&format!("{dir}/{f}"));
        }
        i.files.insert(format!("{dir}/stale"), "x".into());
        let log = load(&mut i, &back);
        assert!(log.contains("count adel state 1"), "{log}");
        assert_eq!(under(&i, &dir), st, "{from}");
        if from == "dir" {
            assert!(
                i.files
                    .contains_key("/states/sfyprobe_lib/rc/spectre/.sevSaveDir")
            );
        }
        // And the dump of what came back is the first dump.
        assert_eq!(dump(&mut i, "st1", from), text);
    }
}

#[test]
fn a_changed_bench_becomes_maestro_with_its_variables() {
    let mut i = virtuoso();
    put(&mut i, DIR, &state(&[("C", "1n"), ("R", "2k")]));
    let text = dump(&mut i, "st1", "dir");
    let (mut files, _) = import(&text, &project());
    let bench = String::from_utf8(files[BENCH].clone()).unwrap();
    files.insert(
        BENCH.into(),
        bench.replace("var C 1n", "var C 2n").into_bytes(),
    );

    let (back, notes) = export(&files, BENCH);
    assert!(records(&back, "adel").is_empty());
    let m = records(&back, "maestro");
    assert_eq!(m[0].str(0).unwrap(), "st1");
    let n = notes.text();
    assert!(
        n.contains("ADE L state st1 was changed in Schemify: it goes back as Maestro test st1 of sfyprobe_lib/rc/maestro"),
        "{n}"
    );
    for f in under(&i, DIR).keys() {
        i.files.remove(&format!("{DIR}/{f}"));
    }
    load(&mut i, &back);
    let su = i.host.mae.saved[&(LIB.to_owned(), "rc".to_owned())].clone();
    let var = |n: &str| su.vars.iter().find(|v| v.0 == n).map(|v| v.1.as_str());
    assert_eq!((var("C"), var("R")), (Some("2n"), Some("2k")));
    assert!(su.tests.iter().any(|t| t.name == "st1"));
    assert!(
        under(&i, DIR).is_empty(),
        "the state files are not rewritten"
    );
}

#[test]
fn an_existing_state_is_refused_without_overwrite() {
    let mut i = virtuoso();
    put(&mut i, DIR, &state(&[("C", "1n")]));
    let text = dump(&mut i, "st1", "dir");
    let (files, _) = import(&text, &project());
    let (back, _) = export(&files, BENCH);
    i.files.insert("/t/b.sfyx".into(), back);
    let ok = run(&mut i, r#"(SfyAdeLImport "/t/b.sfyx")"#);
    assert!(!ok.truthy());
    assert!(
        i.files["/t/b.sfyx.log"]
            .contains("exists at /states/sfyprobe_lib/rc/spectre/st1; pass ?overwrite t")
    );
}

#[test]
fn malformed_state_records_are_refused_with_their_line() {
    let head = "(\"sfyx\" 1 \"t\")\n";
    for (body, want) in [
        (
            "(\"adelfile\" \"a\" \"\")\n",
            "line 2: adelfile: outside an adel state",
        ),
        (
            "(\"adel\" \"s\" \"l\" \"c\" \"spectre\" \"dir\" \"rel\")\n",
            "root is an absolute path",
        ),
        (
            "(\"adel\" \"s\" \"l\" \"c\" \"spectre\" \"x\" \"/r\")\n",
            "from `x`",
        ),
        (
            "(\"adel\" \"s\" \"l\" \"c\" \"spectre\" \"dir\" \"/r\")\n(\"adelfile\" \"../a\" \"\")\n",
            "not a file name",
        ),
        (
            "(\"adel\" \"s\" \"l\" \"c\" \"spectre\" \"dir\" \"/r\")\n(\"adelfile\" \"a\" \"*\")\n",
            "a: not base64",
        ),
    ] {
        let text = format!("{head}{body}(\"end\")\n");
        let e = virtuoso::import(
            &text,
            &Files::new(),
            &Mapping::default(),
            &ImportOptions::default(),
        )
        .err()
        .unwrap_or_else(|| panic!("accepted: {body}"));
        assert!(e.join("\n").contains(want), "{e:?} for {body}");
    }
}

#[test]
fn a_component_outside_the_grammar_is_reported_and_kept() {
    let mut i = virtuoso();
    let mut st = state(&[("C", "1n")]);
    st.insert(
        "variables".into(),
        "savedComponent = system(\"rm -rf ~\")\n".into(),
    );
    put(&mut i, DIR, &st);
    let text = dump(&mut i, "st1", "dir");
    let (files, notes) = import(&text, &project());
    assert!(
        notes.text().contains(
            "component variables is not data (calls `system`): kept verbatim, not mapped"
        ),
        "{}",
        notes.text()
    );
    let bench = String::from_utf8(files[BENCH].clone()).unwrap();
    assert!(!bench.contains("var C"), "{bench}");
    assert!(bench.contains("  vadelfile variables text "), "{bench}");
}

// ---------------------------------------------------------------------------------------
// The probe's real states
// ---------------------------------------------------------------------------------------

#[test]
fn the_probe_states_round_trip_byte_for_byte() {
    let (Some(d), Some(c)) = (
        cadence_fixture("adel/states_dir/sfyprobe_lib/rc/spectre/st1"),
        cadence_fixture("adel/cellview_state_st2"),
    ) else {
        return;
    };
    for (from, state, src, dir) in [
        ("dir", "st1", d, DIR.to_owned()),
        ("cellview", "st2", c, format!("/mock/{LIB}/rc/st2")),
    ] {
        let st = dir_files(&src);
        let mut i = virtuoso();
        put(&mut i, &dir, &st);
        if from == "dir" {
            for lvl in [
                "/states/sfyprobe_lib",
                "/states/sfyprobe_lib/rc",
                "/states/sfyprobe_lib/rc/spectre",
            ] {
                i.files.insert(format!("{lvl}/.sevSaveDir"), String::new());
            }
        }
        let text = dump(&mut i, state, from);
        assert_eq!(
            records(&text, "adelfile").len(),
            st.len(),
            "{from}: every file"
        );

        let (files, notes) = import(&text, &project());
        let bench = String::from_utf8(files[BENCH].clone()).unwrap();
        // The test's own setup, as Maestro has it: tran, the outputs, the model file.
        for want in [
            format!("code {state} spectre"),
            "  | tran tran stop=20u strobeoutput=strobeonly strobeperiod=0.2u".to_owned(),
            "output vmax".to_owned(),
            "output vavg".to_owned(),
        ] {
            assert!(bench.contains(&want), "{from}: {want}\n{bench}");
        }
        // The model file sits in the probing account's home: only the path's tail is checked.
        assert!(
            bench
                .lines()
                .any(|l| l.starts_with("  | include \"/")
                    && l.ends_with("/sfytest/probes/stats.scs\"")),
            "{from}: the model file\n{bench}"
        );
        let n = notes.text();
        assert!(
            n.contains("component convergence is a tar archive of sigList (calls `asiAddToRestorePool`, which only evaluating applies): kept verbatim, not mapped"),
            "{from}: {n}"
        );
        assert!(
            n.contains("`` has no Schemify form"),
            "{from}: the empty C and rmc: {n}"
        );
        assert!(
            n.contains("not mapped, kept verbatim for the way back while the bench is unchanged: cosimOptions devCheckingSetup emirOptions environmentOptions"),
            "{from}: {n}"
        );

        let (back, _) = export(&files, BENCH);
        assert_eq!(records(&back, "adelfile").len(), st.len());
        for f in st.keys() {
            i.files.remove(&format!("{dir}/{f}"));
        }
        load(&mut i, &back);
        assert_eq!(under(&i, &dir), st, "{from}: byte for byte");
    }
}
