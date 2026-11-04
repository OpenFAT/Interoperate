//! A Maestro view's setup files and its disabled items through Schemify and back (spec
//! `interop.md` §9.11, lane L11), on the mock maestro: the files (run plans and
//! reliability in `maestro.sdb`, optimizer starting states in `states/`) ride in the bench
//! header and come back byte for byte while the bench is unchanged, first under §9.7's
//! in-place restore once it changed; a disabled var (`<var enabled="0">`) and an output
//! with its Plot off are `enable false`, both ways.

mod common;
mod skill;

use interop::design::Files;
use interop::map::Mapping;
use interop::sfyx::{self, Record};
use interop::virtuoso::{self, ExportOptions, ImportOptions};
use skill::mockdb::MockDb;
use skill::{Interp, V};

const LIB: &str = "mlib";
const VIEW: &str = "/mock/mlib/rc/maestro";
const BENCH: &str = "mlib/rc.shbench";

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

/// The tags of `text`'s maestro records, `maestro` first.
fn maestro_tags(text: &str) -> Vec<String> {
    sfyx::read(text)
        .unwrap()
        .into_iter()
        .skip_while(|r| r.tag != "maestro")
        .filter(|r| r.tag != "end")
        .map(|r| r.tag)
        .collect()
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

fn bench(files: &Files) -> String {
    String::from_utf8(files[BENCH].clone()).unwrap()
}

/// The view's setup files the dumper copies, `(path under the view, text)`.
fn setup_files(i: &Interp<MockDb>) -> Vec<(String, String)> {
    ["active.state", "maestro.sdb", "states/start.sdb"]
        .iter()
        .map(|f| ((*f).to_owned(), i.files[&format!("{VIEW}/{f}")].clone()))
        .collect()
}

/// A mock Virtuoso whose `mlib/rc/maestro` holds test T1 with vars A and K (disabled),
/// outputs vmax and dis (Plot off), a run plan in `maestro.sdb`, a setup state in
/// `states/`, and what the dumper leaves out (a history, results, a session's lock).
fn view() -> Interp<MockDb> {
    let mut i = skill::virtuoso(&[LIB, "analogLib", "basic"]);
    let ok = run(
        &mut i,
        &format!(
            r#"(let ((cv (dbOpenCellViewByType "{LIB}" "rc" "schematic" "schematic" "w")) (s nil))
                 (dbSave cv) (dbClose cv)
                 (setq s (maeOpenSetup "{LIB}" "rc" "maestro" ?mode "a"))
                 (maeCreateTest "T1" ?lib "{LIB}" ?cell "rc" ?view "schematic" ?simulator "spectre" ?session s)
                 (maeSetAnalysis "T1" "tran" ?enable t ?options (list (list "stop" "1u")) ?session s)
                 (maeSetVar "A" "1" ?session s)
                 (maeSetVar "K" "7" ?session s)
                 (axlSetEnabled (axlGetVar (axlGetMainSetupDB s) "K") nil)
                 (maeAddOutput "vmax" "T1" ?outputType "point" ?expr "ymax(VT(\"/out\"))" ?session s)
                 (maeAddOutput "dis" "T1" ?outputType "point" ?expr "ymin(VT(\"/out\"))" ?plot nil ?session s)
                 (maeSaveSetup ?lib "{LIB}" ?cell "rc" ?view "maestro" ?session s)
                 (maeCloseSession ?session s ?forceClose t))"#
        ),
    );
    assert!(ok.truthy(), "{}", i.log);
    let sdb = format!("{VIEW}/maestro.sdb");
    let text = i.files[&sdb].replace(
        "</setupdb>\n",
        "\t<runplan>\n\t\t<run version=\"2.0\">R1\n\t\t\t<currentmode>Single Run, Sweeps and Corners</currentmode>\n\t\t</run>\n\t</runplan>\n</setupdb>\n",
    );
    assert!(text.contains("<var enabled=\"0\">K"), "{text}");
    i.files.insert(sdb, text);
    for (f, t) in [
        (
            "states/start.sdb",
            "<?xml version=\"1.0\"?>\n<setupdb version=\"6\">start\n\t<active>Active Setup\n\t\t<vars>\n\t\t\t<var>A\n\t\t\t\t<value>1</value>\n\t\t\t</var>\n\t\t</vars>\n\t</active>\n</setupdb>\n",
        ),
        ("maestro.sdb.cdslck", "lock\n"),
        ("history/history.sdb", "<history/>\n"),
        ("results/maestro/Interactive.0.log", "run\n"),
    ] {
        i.files.insert(format!("{VIEW}/{f}"), t.to_owned());
    }
    i
}

/// `view()` dumped and imported into a fresh project: (Virtuoso, the dump, project files).
fn through_schemify() -> (Interp<MockDb>, String, Files) {
    let mut i = view();
    let ok = run(
        &mut i,
        &format!(r#"(SfyExport "/t/a.sfyx" "{LIB}" (list "rc"))"#),
    );
    assert!(ok.truthy(), "{}", i.files["/t/a.sfyx.log"]);
    let dump = i.files["/t/a.sfyx"].clone();
    let mut man = Files::new();
    man.insert("schemify.project".into(), b"schemify 1 project\n".to_vec());
    let files = import(&dump, &man);
    (i, dump, files)
}

#[test]
fn the_dump_carries_the_setup_files_and_not_the_runs() {
    let (i, dump, files) = through_schemify();
    let got: Vec<(String, String)> = records(&dump, "maefile")
        .iter()
        .map(|r| (r.str(0).unwrap().to_owned(), r.text(1).unwrap()))
        .map(|(p, b64)| (p, String::from_utf8(skill_b64(&b64)).unwrap()))
        .collect();
    assert_eq!(got, setup_files(&i));
    let b = bench(&files);
    assert_eq!(b.matches("  vmaefile ").count(), 3, "{b}");
    assert_eq!(b.matches("  vmaefiles ").count(), 1, "{b}");
    // The disabled items: K and dis are `enable false`, A and vmax are not.
    assert!(b.contains("var K 7\n  enable false\n"), "{b}");
    assert!(
        b.contains("var A 1\n") && !b.contains("var A 1\n  enable"),
        "{b}"
    );
    assert!(b.contains("\n  enable false\n"), "{b}");
    let dis = b.split("output dis ").nth(1).expect("output dis");
    assert!(dis.lines().nth(1) == Some("  enable false"), "{b}");
    let vmax = b.split("output vmax ").nth(1).expect("output vmax");
    assert!(vmax.lines().nth(1) != Some("  enable false"), "{b}");
}

/// Standard base64, as the dump holds it.
fn skill_b64(t: &str) -> Vec<u8> {
    const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in t.bytes().filter(|c| *c != b'=') {
        acc = (acc << 6) | B64.iter().position(|&x| x == c).unwrap() as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    out
}

#[test]
fn an_unchanged_bench_restores_the_view_byte_for_byte_without_a_session() {
    let (mut i, _, files) = through_schemify();
    let want = setup_files(&i);
    let (second, notes) = export(&files, BENCH);
    assert_eq!(
        maestro_tags(&second),
        ["maestro", "maefile", "maefile", "maefile"],
        "{}",
        notes.text()
    );
    assert!(!notes.text().contains("runplan"), "{}", notes.text());

    // Over the view, the files edited in Virtuoso since the dump: put back, and no setup
    // restore or test call ran.
    for (f, _) in &want {
        i.files.insert(format!("{VIEW}/{f}"), "edited\n".into());
    }
    i.host.mae.calls.clear();
    i.files.insert("/t/b.sfyx".into(), second.clone());
    let ok = run(&mut i, r#"(SfyImport "/t/b.sfyx" ?overwrite t)"#);
    let log = i.files["/t/b.sfyx.log"].clone();
    assert!(ok.truthy(), "{log}");
    assert!(
        log.contains("count maestro view restored from its files 1"),
        "{log}"
    );
    assert_eq!(setup_files(&i), want);
    assert!(
        !i.host
            .mae
            .calls
            .iter()
            .any(|c| c == "axlImportSetup" || c == "maeOpenSetup" || c == "maeCreateTest"),
        "{:?}",
        i.host.mae.calls
    );

    // Into a library without the view: made, then filled with the same bytes; a session
    // then reads the setup from them (K still disabled).
    let mut j = skill::virtuoso(&[LIB, "analogLib", "basic"]);
    j.files.insert("/t/b.sfyx".into(), second);
    let ok = run(&mut j, r#"(SfyImport "/t/b.sfyx")"#);
    assert!(ok.truthy(), "{}", j.files["/t/b.sfyx.log"]);
    let got: Vec<(String, String)> = want
        .iter()
        .map(|(f, _)| (f.clone(), j.files[&format!("{VIEW}/{f}")].clone()))
        .collect();
    assert_eq!(got, want);
    let off = run(
        &mut j,
        &format!(
            r#"(let ((s (maeOpenSetup "{LIB}" "rc" "maestro" ?mode "a")) (r nil))
                 (setq r (maeGetSetup ?typeName "variables" ?enabled nil ?session s))
                 (maeCloseSession ?session s ?forceClose t)
                 r)"#
        ),
    );
    assert_eq!(
        format!("{off:?}"),
        format!("{:?}", V::list(vec![V::str("K")]))
    );
}

#[test]
fn a_changed_bench_writes_the_files_first_then_restores_in_place() {
    let (mut i, _, files) = through_schemify();
    let want = setup_files(&i);
    let mut files = files;
    let b = bench(&files).replace("var A 1\n", "var A 2\n");
    files.insert(BENCH.into(), b.into_bytes());
    let (text, notes) = export(&files, BENCH);
    let tags = maestro_tags(&text);
    assert_eq!(&tags[..2], ["maestro", "maesdb"], "{tags:?}");
    assert_eq!(
        tags.iter().filter(|t| *t == "maefile").count(),
        3,
        "{tags:?}"
    );
    assert!(tags.contains(&"var".to_owned()), "{tags:?}");
    assert!(
        notes.text().contains("needs_review:runplan"),
        "{}",
        notes.text()
    );
    i.files.insert("/t/c.sfyx".into(), text);
    let ok = run(&mut i, r#"(SfyImport "/t/c.sfyx" ?overwrite t)"#);
    let log = i.files["/t/c.sfyx.log"].clone();
    assert!(
        ok.truthy() && log.contains("count maestro restored 1"),
        "{log}"
    );
    // Schemify's change landed, the run plan (no other carrier) is still there, the
    // starting state came back as it was, and K stays disabled.
    let sdb = i.files[&format!("{VIEW}/maestro.sdb")].clone();
    assert!(sdb.contains("<var>A\n\t\t\t\t<value>2</value>"), "{sdb}");
    assert!(sdb.contains("<run version=\"2.0\">R1"), "{sdb}");
    assert!(sdb.contains("<var enabled=\"0\">K"), "{sdb}");
    assert_eq!(i.files[&format!("{VIEW}/states/start.sdb")], want[2].1);
}

#[test]
fn a_load_into_another_library_renames_it_in_the_restored_files() {
    let (_, _, files) = through_schemify();
    let (second, _) = export(&files, BENCH);
    let mut j = skill::virtuoso(&["other", "analogLib", "basic"]);
    j.files.insert("/t/b.sfyx".into(), second);
    let ok = run(&mut j, r#"(SfyImport "/t/b.sfyx" ?lib "other")"#);
    assert!(ok.truthy(), "{}", j.files["/t/b.sfyx.log"]);
    let sdb = j.files["/mock/other/rc/maestro/maestro.sdb"].clone();
    assert!(sdb.contains("<value>other</value>"), "{sdb}");
    assert!(!sdb.contains(&format!("<value>{LIB}</value>")), "{sdb}");
}

const SCHEMIFY_BENCH: &str = "schemify 1 shbench

var A 1
var K 7
  enable false

output vmax \"ymax(v(out))\"

output dis \"ymin(v(out))\"
  enable false

code sim spice
  | .tran 1n 1u
";

/// A Schemify bench with a disabled var and output, loaded into the mock.
fn schemify_bench_in_virtuoso() -> (Interp<MockDb>, Files) {
    let d = schemify_format::parse(SCHEMIFY_BENCH.as_bytes()).unwrap();
    let mut files = Files::new();
    files.insert("schemify.project".into(), b"schemify 1 project\n".to_vec());
    files.insert("tb.shbench".into(), schemify_format::write(&d).into_bytes());
    let (text, _) = export(&files, "tb.shbench");
    let flags: Vec<(String, String, i64)> = ["varenable", "outenable"]
        .iter()
        .flat_map(|t| records(&text, t))
        .map(|r| {
            (
                r.tag.clone(),
                r.str(0).unwrap().to_owned(),
                r.int(1).unwrap().into(),
            )
        })
        .collect();
    assert_eq!(
        flags,
        [
            ("varenable".to_owned(), "K".to_owned(), 0),
            ("outenable".to_owned(), "dis".to_owned(), 0)
        ]
    );
    let mut i = skill::virtuoso(&["schemify"]);
    i.files.insert("/t/in.sfyx".into(), text);
    let ok = run(&mut i, r#"(SfyImport "/t/in.sfyx")"#);
    assert!(ok.truthy(), "{}", i.files["/t/in.sfyx.log"]);
    (i, files)
}

#[test]
fn disabled_vars_and_outputs_go_to_maestro_and_come_back() {
    let (mut i, files) = schemify_bench_in_virtuoso();
    let su = &i.host.mae.saved[&("schemify".to_owned(), "tb".to_owned())];
    assert_eq!(su.vars_off, ["K"]);
    let mut off: Vec<(&str, bool)> = su.tests[0]
        .outputs
        .iter()
        .map(|o| (o.name.as_str(), o.off))
        .collect();
    off.sort();
    assert_eq!(off, [("dis", true), ("vmax", false)]);

    // Dumped back: the same bench, the flags read from the setup XML and the state.
    run(&mut i, r#"(SfyExport "/t/d.sfyx" "schemify" (list "tb"))"#);
    let files = import(&i.files["/t/d.sfyx"].clone(), &files);
    let b = String::from_utf8(files["tb.shbench"].clone()).unwrap();
    assert!(b.contains("var K 7\n  enable false\n"), "{b}");
    let dis = b.split("output dis ").nth(1).expect("output dis");
    assert_eq!(dis.lines().nth(1), Some("  enable false"), "{b}");

    // Both enabled again in Schemify: K by `varenable 1` over the stored disabled var, dis
    // by `outenable 1` over its stored Plot off.
    let mut files = files;
    let edited = b.replace("var K 7\n  enable false\n", "var K 7\n");
    let (head, tail) = edited.split_once("output dis ").unwrap();
    let (first, rest) = tail.split_once("\n  enable false\n").unwrap();
    let edited = format!("{head}output dis {first}\n{rest}");
    assert!(!edited.contains("enable false"), "{edited}");
    files.insert("tb.shbench".into(), edited.into_bytes());
    let (text, notes) = export(&files, "tb.shbench");
    let ve = records(&text, "varenable");
    assert_eq!((ve.len(), ve[0].str(0), ve[0].int(1)), (1, Ok("K"), Ok(1)));
    let oe = records(&text, "outenable");
    assert_eq!(
        (oe.len(), oe[0].str(0), oe[0].int(1)),
        (1, Ok("dis"), Ok(1)),
        "{}",
        notes.text()
    );
    i.files.insert("/t/e.sfyx".into(), text);
    let ok = run(&mut i, r#"(SfyImport "/t/e.sfyx" ?overwrite t)"#);
    assert!(ok.truthy(), "{}", i.files["/t/e.sfyx.log"]);
    let su = &i.host.mae.saved[&("schemify".to_owned(), "tb".to_owned())];
    assert!(su.vars_off.is_empty());
    assert!(su.tests[0].outputs.iter().all(|o| !o.off));
}
