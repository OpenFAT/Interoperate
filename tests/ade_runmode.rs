//! Maestro's run modes through Schemify and back (spec `interop.md` §9.9, lane L5): the
//! Monte Carlo and optimizer options of the stored setup become `montecarlo`/`optimize`
//! records, the specs goals, a range var `vary` with `min`/`max`/`step`; an edit in
//! Schemify goes back into the stored setup XML and nothing else changes.
//!
//! The tests read the real probe setup in `tests/fixtures/cadence/maestro/` (local-only:
//! they skip where it is absent) and run it through the mock maestro.

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
const BENCH: &str = "sfyprobe_lib/rc.shbench";

fn fixture(rel: &str) -> Option<String> {
    Some(std::fs::read_to_string(cadence_fixture(rel)?).unwrap())
}

/// Setup XML after `normalize`, without the current run mode the dumper adds.
fn normalized(xml: &str) -> String {
    let xml: String = xml
        .lines()
        .filter(|l| !l.trim().starts_with("<currentmode>"))
        .map(|l| format!("{l}\n"))
        .collect();
    let mut n = mae_xml::parse(&xml).unwrap_or_else(|e| panic!("{e}\n{xml}"));
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

/// The setup XML a `.sfyx` file sends.
fn sent_xml(text: &str) -> String {
    records(text, "maesdb")[0].text(0).unwrap()
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

/// The probe setup `xml` (with the fixture test states) in a mock view, dumped and
/// imported into a fresh project: (dumped XML, project files).
fn through_schemify(xml: &str) -> Option<(String, Files)> {
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
    i.files.insert("/fix/setup.sdb".into(), xml.to_owned());
    let ok = run(
        &mut i,
        &format!(
            r#"(let ((s (maeOpenSetup "{LIB}" "rc" "maestro" ?mode "a")))
                 (axlImportSetup s "/fix/setup.sdb" (list "vars" "tests" "parameters" "corners" "runoptions" "specs") "" 'overwrite)
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
    let dump = i.files["/t/a.sfyx"].clone();
    let mut files = Files::new();
    files.insert("schemify.project".into(), b"schemify 1 project\n".to_vec());
    let imp = virtuoso::import(
        &dump,
        &files,
        &Mapping::default(),
        &ImportOptions::default(),
    )
    .unwrap_or_else(|e| panic!("{e:?}"));
    files.extend(imp.files);
    Some((sent_xml(&dump), files))
}

fn bench(files: &Files) -> String {
    String::from_utf8(files[BENCH].clone()).unwrap()
}

/// The lines of `text`'s record that starts with `head` (`montecarlo mc`), its
/// attributes included.
fn block(text: &str, head: &str) -> Vec<String> {
    let mut out = Vec::new();
    for l in text.lines() {
        if out.is_empty() {
            if l == head {
                out.push(l.to_owned());
            }
        } else if l.starts_with("  ") {
            out.push(l.to_owned());
        } else {
            break;
        }
    }
    out
}

/// `files` with the bench text edited by `f`.
fn edited(files: &Files, f: impl Fn(String) -> String) -> Files {
    let mut out = files.clone();
    out.insert(BENCH.into(), f(bench(files)).into_bytes());
    out
}

/// The lines of `a` and `b` that differ, as `(a line, b line)` (same line count).
fn diff(a: &str, b: &str) -> Vec<(String, String)> {
    assert_eq!(a.lines().count(), b.lines().count(), "{a}\n----\n{b}");
    a.lines()
        .zip(b.lines())
        .filter(|(x, y)| x != y)
        .map(|(x, y)| (x.to_owned(), y.to_owned()))
        .collect()
}

#[test]
#[cfg_attr(
    not(feature = "cadence-records"),
    ignore = "needs the bench test/param records of Schemify's cadence-records format"
)]
fn the_probe_run_modes_become_montecarlo_and_optimize_records() {
    let Some(xml) = fixture("maestro/mae_export.sdb") else {
        return;
    };
    let Some((_, files)) = through_schemify(&xml) else {
        return;
    };
    let b = bench(&files);
    assert_eq!(
        block(&b, "montecarlo mc"),
        [
            "montecarlo mc",
            "  samples 7",
            "  seed 4242",
            "  variation process",
            "  sampling lhs",
            "  vmc mcnumbins 5",
            "  vmc saveallplots 1",
        ],
        "{b}"
    );
    // T1 is the mapped test: its specs are the goals (gt, and two targets with their
    // tolerances); T2's stay Maestro's.
    let goals = [
        "  goal vmax gt 0.5",
        "  goal vmin target 0.1",
        "  goal vavg target 0.5",
    ];
    let tols = ["  vtol vmin 10%", "  vtol vavg 0.05"];
    let want = |head: &str, algo: &str, budget: &str, extra: &[&str]| -> Vec<String> {
        let mut v = vec![head.to_owned(), format!("  algo {algo}")];
        v.extend(goals.iter().map(|s| (*s).to_owned()));
        v.push(format!("  budget {budget}"));
        v.extend(extra.iter().map(|s| (*s).to_owned()));
        v.extend(tols.iter().map(|s| (*s).to_owned()));
        v
    };
    assert_eq!(
        block(&b, "optimize global"),
        want("optimize global", "global", "50", &["  vopt timelimit 7"]),
        "{b}"
    );
    assert_eq!(
        block(&b, "optimize local"),
        want("optimize local", "hooke-jeeves", "10", &[]),
        "{b}"
    );
}

#[test]
#[cfg_attr(
    not(feature = "cadence-records"),
    ignore = "needs the bench test/param records of Schemify's cadence-records format"
)]
fn an_unchanged_bench_sends_the_stored_setup_byte_for_byte() {
    let Some(xml) = fixture("maestro/mae_export.sdb") else {
        return;
    };
    let Some((dumped, files)) = through_schemify(&xml) else {
        return;
    };
    // A bench that holds the view's files goes back as those alone (tests/mae_files.rs).
    let files = common::without_view_files(&files, BENCH);
    let (text, notes) = export(&files, BENCH);
    assert_eq!(sent_xml(&text), dumped, "{}", notes.text());
    // The re-import maps the same records.
    let again = virtuoso::import(
        &text,
        &files,
        &Mapping::default(),
        &ImportOptions::default(),
    )
    .unwrap();
    let b2 = again.files.iter().find(|f| f.0 == BENCH).unwrap();
    let b2 = String::from_utf8(b2.1.clone()).unwrap();
    for head in ["montecarlo mc", "optimize global", "optimize local"] {
        assert_eq!(block(&b2, head), block(&bench(&files), head), "{head}");
    }
}

/// XML → records → XML: the run modes written from the records alone (into the setup
/// without its `<runoptions>`) are the probe's, modulo `normalize`.
#[test]
#[cfg_attr(
    not(feature = "cadence-records"),
    ignore = "needs the bench test/param records of Schemify's cadence-records format"
)]
fn run_modes_written_from_the_records_are_the_probe_setup() {
    let Some(xml) = fixture("maestro/mae_export.sdb") else {
        return;
    };
    let Some((_, full)) = through_schemify(&xml) else {
        return;
    };
    let mut root = mae_xml::parse(&xml).unwrap();
    root.child_mut("active")
        .unwrap()
        .children
        .retain(|c| c.name != "runoptions");
    let Some((bare_xml, bare)) = through_schemify(&mae_xml::write(&root)) else {
        return;
    };
    assert!(!bench(&bare).contains("montecarlo"), "{}", bench(&bare));
    // The bare setup's bench, given the full one's run-mode records.
    let full_b = bench(&full);
    let mut recs = String::new();
    for head in ["montecarlo mc", "optimize global", "optimize local"] {
        recs.push('\n');
        for l in block(&full_b, head) {
            recs.push_str(&l);
            recs.push('\n');
        }
    }
    // (Their var bounds have nothing to add: the probe has no range var.)
    let files = edited(&bare, |b| b + &recs);
    let (text, notes) = export(&files, BENCH);
    let sent = sent_xml(&text);
    assert_ne!(sent, bare_xml);
    assert_eq!(normalized(&sent), normalized(&xml), "{}", notes.text());
}

#[test]
#[cfg_attr(
    not(feature = "cadence-records"),
    ignore = "needs the bench test/param records of Schemify's cadence-records format"
)]
fn one_monte_carlo_option_changed_in_schemify_changes_that_option_only() {
    let Some(xml) = fixture("maestro/mae_export.sdb") else {
        return;
    };
    let Some((dumped, files)) = through_schemify(&xml) else {
        return;
    };
    let files = edited(&files, |b| b.replace("  samples 7\n", "  samples 9\n"));
    let (text, _) = export(&files, BENCH);
    let sent = sent_xml(&text);
    assert_eq!(
        diff(&dumped, &sent),
        [(
            "\t\t\t\t\t<value>7</value>".to_owned(),
            "\t\t\t\t\t<value>9</value>".to_owned()
        )]
    );
    assert!(sent.contains("<runoption>mcnumpoints\n\t\t\t\t\t<value>9</value>"));

    // A new option goes last in its mode; a removed extra leaves it.
    let files = edited(&files, |b| {
        b.replace("  vmc saveallplots 1\n", "")
            .replace("  sampling lhs\n", "  sampling lhs\n  nominal true\n")
    });
    let (text, _) = export(&files, BENCH);
    let s = mae_xml::Setup::from_xml(&sent_xml(&text)).unwrap();
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
            "donominal"
        ]
    );
    assert_eq!(mc.option("donominal"), Some("1"));
}

#[test]
#[cfg_attr(
    not(feature = "cadence-records"),
    ignore = "needs the bench test/param records of Schemify's cadence-records format"
)]
fn optimizer_edits_go_into_their_mode_and_specs() {
    let Some(xml) = fixture("maestro/mae_export.sdb") else {
        return;
    };
    let Some((dumped, files)) = through_schemify(&xml) else {
        return;
    };
    // A local algorithm, a budget, and a goal's bound and weight (in both records: Maestro
    // has one spec per output).
    let files = edited(&files, |b| {
        b.replace("  algo hooke-jeeves\n", "  algo brent-powell\n")
            .replace("  budget 50\n", "  budget 80\n")
            .replace("  goal vmax gt 0.5\n", "  goal vmax gt 0.6 weight 4\n")
    });
    let (text, notes) = export(&files, BENCH);
    let s = mae_xml::Setup::from_xml(&sent_xml(&text)).unwrap();
    let base = mae_xml::Setup::from_xml(&dumped).unwrap();
    assert_eq!(
        s.run_mode("Local Optimization").unwrap().options,
        [
            ("effort".to_owned(), "fine".to_owned()),
            ("numpoints".to_owned(), "10".to_owned())
        ]
    );
    assert_eq!(
        s.run_mode("Global Optimization")
            .unwrap()
            .option("numpoints"),
        Some("80")
    );
    let vmax = s.spec("T1", "vmax").unwrap();
    assert_eq!(
        (
            vmax.kind.as_str(),
            vmax.min.as_str(),
            vmax.weight.as_deref()
        ),
        ("gt", "0.6", Some("4")),
        "{}",
        notes.text()
    );
    // Everything else as Maestro had it.
    for sp in &base.specs {
        if sp.name != "T1.vmax" {
            assert_eq!(s.specs.iter().find(|x| x.name == sp.name), Some(sp));
        }
    }
    assert_eq!(
        s.run_mode("Monte Carlo Sampling"),
        base.run_mode("Monte Carlo Sampling")
    );
    assert_eq!(
        (&s.vars, &s.corners, &s.tests),
        (&base.vars, &base.corners, &base.tests)
    );

    // An algorithm Maestro does not run is reported and its mode left alone.
    let files = edited(&files, |b| {
        b.replace("  algo brent-powell\n", "  algo nelder-mead\n")
    });
    let (text, notes) = export(&files, BENCH);
    let s = mae_xml::Setup::from_xml(&sent_xml(&text)).unwrap();
    assert_eq!(
        s.run_mode("Local Optimization"),
        base.run_mode("Local Optimization")
    );
    assert!(
        notes
            .text()
            .contains("algo nelder-mead has no Maestro run mode"),
        "{}",
        notes.text()
    );
}

#[test]
#[cfg_attr(
    not(feature = "cadence-records"),
    ignore = "needs the bench test/param records of Schemify's cadence-records format"
)]
fn a_range_var_is_varied_and_its_step_goes_back_as_its_value() {
    let Some(xml) = fixture("maestro/mae_export.sdb") else {
        return;
    };
    let xml = xml.replace(
        "<value>7</value>\n\t\t\t</var>",
        "<value>1:1:9</value>\n\t\t\t</var>",
    );
    assert!(xml.contains("<var>K\n\t\t\t\t<value>1:1:9</value>"));
    let Some((dumped, files)) = through_schemify(&xml) else {
        return;
    };
    let b = bench(&files);
    assert!(b.contains("var K 1\n  min 1\n  max 9\n  step 1\n"), "{b}");
    assert!(
        block(&b, "optimize local").contains(&"  vary K".to_owned()),
        "{b}"
    );
    let (text, _) = export(&common::without_view_files(&files, BENCH), BENCH);
    assert_eq!(sent_xml(&text), dumped);

    let files = edited(&files, |b| b.replace("  step 1\n", "  step 2\n"));
    let (text, notes) = export(&files, BENCH);
    assert_eq!(
        diff(&dumped, &sent_xml(&text)),
        [(
            "\t\t\t\t<value>1:1:9</value>".to_owned(),
            "\t\t\t\t<value>1:2:9</value>".to_owned()
        )],
        "{}",
        notes.text()
    );
}

#[test]
fn run_modes_without_a_stored_setup_are_reported() {
    let mut files = Files::new();
    files.insert("schemify.project".into(), b"schemify 1 project\n".to_vec());
    let d = schemify_format::parse(
        b"schemify 1 shbench\n\nvar R 1k\n\nmontecarlo mc\n  samples 5\n\noptimize o\n  algo global\n\ncode sim spice\n  | .tran 1n 1u\n",
    )
    .unwrap();
    files.insert("tb.shbench".into(), schemify_format::write(&d).into_bytes());
    let (_, notes) = export(&files, "tb.shbench");
    let t = notes.text();
    assert!(t.contains("montecarlo mc: no stored Maestro setup"), "{t}");
    assert!(t.contains("optimize o: no stored Maestro setup"), "{t}");
}
