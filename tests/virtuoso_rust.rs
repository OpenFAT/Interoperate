//! The Rust side of the Virtuoso bridge (spec `interop.md` §3.4, §6, §7, §8.2, §12):
//! export → import round trips, hand-written dumps → golden files and netlists, merge,
//! re-mastering, and malformed dumps.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use common::{
    canonical, export_cells, import_opts, is_cell, load_all, project, read_tree, root, strip_fp,
    strip_stash,
};
use interop::design::{Design, Files};
use interop::map::Mapping;
use interop::sfyx;
use interop::virtuoso::{self, ExportOptions, ImportOptions};

/// Loads every cell file of `files`.
fn design(files: &Files) -> Design {
    load_all(files)
}

/// Exports every cell of `files`, the root library as `top`.
fn export_all(files: &Files) -> (String, Design) {
    let d = design(files);
    (export_cells(&d, "test"), d)
}

fn fixture_projects() -> Vec<PathBuf> {
    let r = root();
    vec![
        r.join("crates/app/tests/fixtures/rc"),
        r.join("crates/app/tests/fixtures/inv"),
        r.join("crates/app/tests/fixtures/ota"),
        r.join("crates/elab/tests/fixtures"),
    ]
}

/// Cell files of a project, canonical text by path.
fn cells(files: &Files) -> BTreeMap<String, String> {
    files
        .iter()
        .filter(|(p, _)| is_cell(p))
        .map(|(p, b)| (p.clone(), canonical(b)))
        .collect()
}

#[test]
fn export_import_round_trip_is_exact_with_the_stash() {
    for dir in fixture_projects() {
        let files = project(&dir);
        let (text, _) = export_all(&files);
        // Deterministic.
        assert_eq!(text, export_all(&files).0, "{}", dir.display());
        let man: Files = files
            .iter()
            .filter(|(p, _)| *p == "schemify.project")
            .map(|(p, b)| (p.clone(), b.clone()))
            .collect();
        let imp = virtuoso::import(&text, &man, &Mapping::default(), &import_opts())
            .unwrap_or_else(|e| panic!("{e:?}"));
        let got: BTreeMap<String, String> = imp
            .files
            .iter()
            .filter(|(p, _)| is_cell(p))
            .map(|(p, b)| (p.clone(), strip_fp(&String::from_utf8(b.clone()).unwrap())))
            .collect();
        let want = cells(&files);
        assert_eq!(
            got.keys().collect::<Vec<_>>(),
            want.keys().collect::<Vec<_>>(),
            "{}",
            dir.display()
        );
        for (p, w) in &want {
            assert_eq!(&got[p], w, "{}: {p}", dir.display());
        }
    }
}

// ---------------------------------------------------------------------------------------
// Hand-written dumps → golden files and netlists
// ---------------------------------------------------------------------------------------

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/virtuoso")
}

fn bless() -> bool {
    std::env::var_os("SCHEMIFY_BLESS").is_some()
}

/// Imports `<name>.sfyx` into an empty project.
fn import_fixture(name: &str) -> virtuoso::Import {
    let text = std::fs::read_to_string(fixtures().join(format!("{name}.sfyx"))).unwrap();
    virtuoso::import(
        &text,
        &Files::new(),
        &Mapping::default(),
        &ImportOptions::default(),
    )
    .unwrap_or_else(|e| panic!("{name}: {e:?}"))
}

/// Compares (or with `SCHEMIFY_BLESS`, writes) the golden files of fixture `name`.
fn golden(name: &str, files: &[(String, Vec<u8>)], report: &str) {
    let dir = fixtures().join(name);
    let mut all: Vec<(String, Vec<u8>)> = files.to_vec();
    all.push(("report.txt".into(), report.as_bytes().to_vec()));
    if bless() {
        let _ = std::fs::remove_dir_all(&dir);
        for (p, b) in &all {
            let path = dir.join(p);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, b).unwrap();
        }
        return;
    }
    let want = read_tree(&dir);
    let got: Files = all.into_iter().collect();
    assert_eq!(
        got.keys().collect::<Vec<_>>(),
        want.keys().collect::<Vec<_>>(),
        "{name}: file set (re-bless with SCHEMIFY_BLESS=1)"
    );
    for (p, w) in &want {
        assert_eq!(
            String::from_utf8_lossy(&got[p]),
            String::from_utf8_lossy(w),
            "{name}/{p} (re-bless with SCHEMIFY_BLESS=1)"
        );
    }
}

/// Elaborates `bench` of the imported files (plus the core library) and emits ngspice.
fn netlist(files: &[(String, Vec<u8>)], bench: &str) -> String {
    netlist_in(files, bench, schemify_elab::Dialect::Ngspice)
}

/// The same in `dialect`.
fn netlist_in(files: &[(String, Vec<u8>)], bench: &str, dialect: schemify_elab::Dialect) -> String {
    use schemify_doc::{Project, core_library};
    use schemify_elab::{Request, elaborate, no_funcs};
    let manifest = files
        .iter()
        .find(|f| f.0 == "schemify.project")
        .map(|f| f.1.as_slice());
    let mut p = Project::new(manifest, None, 7).expect("manifest parses");
    let mut libs: BTreeMap<String, Vec<(&str, &[u8])>> = BTreeMap::new();
    libs.insert("core".into(), core_library().to_vec());
    let dirs = manifest
        .map(|m| interop::design::manifest_libs(m).unwrap())
        .unwrap_or_default();
    for (path, bytes) in files.iter().filter(|f| is_cell(&f.0)) {
        let (dir, rel) = path.rsplit_once('/').unwrap_or(("", path));
        let lib = dirs
            .iter()
            .find(|(_, d)| d.as_str() == dir)
            .map_or(String::new(), |(l, _)| l.clone());
        libs.entry(lib).or_default().push((rel, bytes));
    }
    let mut handles = BTreeMap::new();
    for (lib, listing) in &libs {
        p.index_library(lib, "import", listing).unwrap();
    }
    for (lib, listing) in &libs {
        if lib == "core" {
            continue;
        }
        for (rel, bytes) in listing {
            let o = p
                .open(lib, rel, bytes, None)
                .unwrap_or_else(|e| panic!("{lib}/{rel}: {e:?}"));
            let errors: Vec<String> = o
                .diags
                .iter()
                .filter(|d| !d.code.is_warning())
                .map(|d| format!("{d:?}"))
                .collect();
            assert!(errors.is_empty(), "{lib}/{rel}: {errors:?}");
            handles.insert(
                if lib.is_empty() {
                    (*rel).to_owned()
                } else {
                    format!("{}/{rel}", dirs[lib])
                },
                o.handle,
            );
        }
    }
    let req = Request {
        config: None,
        corner: None,
        overrides: &[],
        env: &[],
        dialect,
    };
    let e = elaborate(&p, handles[bench], &req, &mut no_funcs);
    let blocking: Vec<String> = e
        .violations
        .rows
        .iter()
        .filter(|v| v.code.severity() == schemify_elab::Severity::Error)
        .map(|v| format!("{} {}", v.code.as_str(), v.detail))
        .collect();
    assert!(blocking.is_empty(), "{bench}: {blocking:?}");
    schemify_netlist::emit(&e, dialect).text
}

fn same_netlist(name: &str, ours: &str) {
    let want = std::fs::read_to_string(fixtures().join(format!("{name}.ngspice"))).unwrap();
    let r = schemify_spicecmp::compare(&want, ours, &schemify_spicecmp::Options::default());
    assert!(
        r.is_equal(),
        "{name}: netlists differ\n--- ours\n{ours}\n--- diffs\n{:#?}",
        r.diffs
    );
}

/// Every imported cell file loads in `doc` without errors and round-trips (the import
/// oracle of the xschem importer, reused).
fn loads(files: &[(String, Vec<u8>)]) {
    let problems = interop::check_output(files);
    assert!(problems.is_empty(), "{problems:#?}");
}

#[test]
fn dump_fixtures_import_to_the_golden_files() {
    for (name, bench) in [
        ("inverter", Some("demo/tb_inv.shbench")),
        ("bus", Some("demo/tb_buf4.shbench")),
        ("symbol", None),
        ("pdk", None),
        ("netexpr", Some("demo/tb_ne.shbench")),
    ] {
        let imp = import_fixture(name);
        golden(name, &imp.files, &imp.notes.text());
        loads(&imp.files);
        if let Some(b) = bench {
            let n = netlist(&imp.files, b);
            same_netlist(name, &n);
        }
    }
}

// ---------------------------------------------------------------------------------------
// The round trip without the stash: Virtuoso-carried data exact, losses documented
// ---------------------------------------------------------------------------------------

/// Header attributes, param attributes and record kinds the Virtuoso side cannot hold:
/// the documented losses of a round trip without the stash (spec §7.1, §7.4; the list the
/// stash exists for). Everything else must come back exactly.
const LOST_PARAM_ATTRS: [&str; 8] = [
    "min", "max", "step", "grid", "show", "computed", "cached", "unit",
];
const LOST_KINDS: [&str; 13] = [
    "prose",
    "var",
    "body",
    "waive",
    "constraint",
    "corner",
    "config",
    "sweep",
    "montecarlo",
    "optimize",
    "output",
    "spec",
    "code",
];

fn recs(text: &str) -> schemify_format::Document<'_> {
    schemify_format::parse(text.as_bytes()).unwrap_or_else(|e| panic!("{e:?}\n{text}"))
}

fn attr<'a>(d: &'a schemify_format::Document<'_>, r: u32, k: &str) -> Option<&'a str> {
    d.find_attr(r, k).and_then(|a| d.attr_value(a, 0))
}

/// The text of record `r` without the attributes `skip`.
fn rec_text(d: &schemify_format::Document<'_>, r: u32, skip: &[&str]) -> String {
    let mut out: Vec<String> = d.fields(r).map(str::to_owned).collect();
    for a in d.attrs(r) {
        if skip.contains(&d.attr_key(a)) {
            continue;
        }
        out.push(format!(
            "{}={}",
            d.attr_key(a),
            d.attr_values(a).collect::<Vec<_>>().join(",")
        ));
    }
    out.join(" ")
}

/// Differences between `orig` and its stash-less round trip `got` that are not documented
/// losses.
fn undocumented_losses(path: &str, orig: &str, got: &str) -> Vec<String> {
    use schemify_format::RecordKind as K;
    let (o, g) = (recs(orig), recs(got));
    let mut bad = Vec::new();
    let by = |d: &schemify_format::Document<'_>, k: K| -> BTreeMap<String, u32> {
        (1..d.record_count())
            .filter(|&r| d.record_kind(r) == k)
            .map(|r| (d.field(r, 0).unwrap_or("").to_owned(), r))
            .collect()
    };
    // Pins: the same set, positions and connection attributes (order may differ).
    // Ground spellings `0` and `gnd` come back as `GND` (§3.3).
    let ground = |t: String| {
        t.replace("inherit=0", "inherit=GND")
            .replace("inherit=gnd", "inherit=GND")
    };
    let pins = |d: &schemify_format::Document<'_>| -> BTreeSet<String> {
        by(d, K::Pin)
            .values()
            .map(|&r| ground(rec_text(d, r, &["domain", "number"])))
            .collect()
    };
    if pins(&o) != pins(&g) {
        bad.push(format!("{path}: pins {:?} → {:?}", pins(&o), pins(&g)));
    }
    // Params: the same names; kinds equal (a computed param comes back a number).
    let (op, gp) = (by(&o, K::Param), by(&g, K::Param));
    if op.keys().collect::<Vec<_>>() != gp.keys().collect::<Vec<_>>() {
        bad.push(format!("{path}: params {:?} → {:?}", op.keys(), gp.keys()));
    }
    for (n, &r) in &op {
        let Some(&gr) = gp.get(n) else { continue };
        let ok = attr(&o, r, "kind").unwrap_or("number");
        let gk = attr(&g, gr, "kind").unwrap_or("number");
        let ok = if attr(&o, r, "computed").is_some() {
            "number"
        } else {
            ok
        };
        if ok != gk {
            bad.push(format!("{path}: param {n} kind {ok} → {gk}"));
        }
        for a in o.attrs(r) {
            let k = o.attr_key(a);
            if !LOST_PARAM_ATTRS.contains(&k)
                && !["kind", "doc", "choices"].contains(&k)
                && attr(&g, gr, k).is_none()
            {
                bad.push(format!("{path}: param {n} lost `{k}`"));
            }
        }
    }
    // Symbol graphics: the same count per form (order, attributes, closing points and
    // label templates outside §3.5 are documented losses; so is a circle off the DBU grid,
    // which Virtuoso stores elliptic and reads back as a polygon).
    let forms = |d: &schemify_format::Document<'_>| -> BTreeMap<K, usize> {
        let mut m = BTreeMap::new();
        for r in 1..d.record_count() {
            let k = match d.record_kind(r) {
                K::Circle => K::Poly,
                k => k,
            };
            if k.rank() == 2 && k != K::Stext {
                *m.entry(k).or_insert(0) += 1;
            }
        }
        m
    };
    if forms(&o) != forms(&g) {
        bad.push(format!(
            "{path}: graphics {:?} → {:?}",
            forms(&o),
            forms(&g)
        ));
    }
    // Rank 3: wires, labels, ports, noconns exactly; texts but their size; instances
    // their placement, name, ignore, inherits and the set of overrides; shapes their form.
    for k in [
        K::Wire,
        K::Label,
        K::Port,
        K::Noconn,
        K::Text,
        K::Shape,
        K::Inst,
    ] {
        let (om, gm) = (by(&o, k), by(&g, k));
        for (id, &r) in &om {
            let Some(&gr) = gm.get(id) else {
                bad.push(format!("{path}: {} {id} missing", k.as_str()));
                continue;
            };
            let same = match k {
                K::Text => rec_text(&o, r, &["size"]) == rec_text(&g, gr, &["size"]),
                K::Shape => o.field(r, 1) == g.field(gr, 1),
                K::Inst => {
                    let keys = |d: &schemify_format::Document<'_>, r: u32| -> BTreeSet<String> {
                        d.attrs(r)
                            .map(|a| d.attr_key(a).to_owned())
                            .filter(|k| k != "ignore")
                            .collect()
                    };
                    attr(&o, r, "name") == attr(&g, gr, "name")
                        && o.find_attr(r, "at")
                            .map(|a| o.attr_values(a).collect::<Vec<_>>())
                            == g.find_attr(gr, "at")
                                .map(|a| g.attr_values(a).collect::<Vec<_>>())
                        && keys(&g, gr).is_subset(&keys(&o, r))
                        && (attr(&o, r, "ignore") == Some("true"))
                            == (attr(&g, gr, "ignore") == Some("true"))
                }
                _ => rec_text(&o, r, &[]) == rec_text(&g, gr, &[]),
            };
            if !same {
                bad.push(format!(
                    "{path}: {} {id}: {} → {}",
                    k.as_str(),
                    rec_text(&o, r, &[]),
                    rec_text(&g, gr, &[])
                ));
            }
        }
        if gm.len() > om.len() {
            bad.push(format!(
                "{path}: {} extra {}",
                gm.len() - om.len(),
                k.as_str()
            ));
        }
    }
    // Rank 4–6 may be lost; nothing may appear from nowhere.
    for r in 1..g.record_count() {
        let w = g.record_word(r);
        if LOST_KINDS.contains(&w) && !(1..o.record_count()).any(|x| o.record_word(x) == w) {
            bad.push(format!("{path}: {w} appeared"));
        }
    }
    bad
}

/// Round trip of `files` both ways: exact with the stash; without it, every difference a
/// documented loss.
fn round_trip(files: &Files, what: &str) {
    let (text, _) = export_all(files);
    let man: Files = files
        .iter()
        .filter(|(p, _)| *p == "schemify.project")
        .map(|(p, b)| (p.clone(), b.clone()))
        .collect();
    let want = cells(files);
    for (stash, t) in [(true, text.clone()), (false, strip_stash(&text))] {
        let imp = virtuoso::import(&t, &man, &Mapping::default(), &import_opts())
            .unwrap_or_else(|e| panic!("{what}: {e:?}"));
        let got: BTreeMap<String, String> = imp
            .files
            .iter()
            .filter(|(p, _)| is_cell(p))
            .map(|(p, b)| (p.clone(), strip_fp(&String::from_utf8(b.clone()).unwrap())))
            .collect();
        assert_eq!(
            got.keys().collect::<Vec<_>>(),
            want.keys().collect::<Vec<_>>(),
            "{what}"
        );
        for (p, w) in &want {
            if stash {
                assert_eq!(&got[p], w, "{what}: {p} (with the stash)\n{t}");
            } else {
                let bad = undocumented_losses(p, w, &got[p]);
                assert!(
                    bad.is_empty(),
                    "{what}: {bad:#?}\n--- want\n{w}\n--- got\n{}",
                    got[p]
                );
            }
        }
    }
}

#[test]
fn round_trip_without_the_stash_loses_only_what_is_documented() {
    for dir in fixture_projects() {
        round_trip(&project(&dir), &dir.display().to_string());
    }
}

/// A bench instantiating every core cell (with overrides), so the core cells export as
/// `schemify_core` cells (§6.5) — here renamed `mycore` so the import writes them back.
fn core_project() -> Files {
    let mut t = String::from("schemify 1 shbench\n\n");
    let mut x = 0;
    for (k, (name, _)) in schemify_doc::core_library().iter().enumerate() {
        let stem = name.trim_end_matches(".shprim");
        t.push_str(&format!(
            "inst a{k:05} core:{stem}\n  at {x} 0 R0\n  name X{k}\n"
        ));
        match stem {
            "vsource" => t.push_str("  value \"dc 1 ac 1 pulse(0 1.8 0 10p 10p 5n 10n)\"\n"),
            "isource" => t.push_str("  value \"sin(0 1m 1meg)\"\n"),
            "res" => t.push_str("  value 2k\n  m 2\n"),
            "nmos4" => t.push_str("  w 2u\n  l 0.5u\n  model nch\n"),
            _ => {}
        }
        x += 200;
    }
    [("tb.shbench".to_owned(), t.into_bytes())]
        .into_iter()
        .collect()
}

#[test]
fn the_core_library_round_trips() {
    let files = core_project();
    let d = design(&files);
    let tops: Vec<usize> = (0..d.cells.len()).collect();
    let mut opts = ExportOptions::default();
    opts.lib_map.insert(String::new(), "top".into());
    opts.lib_map.insert("core".into(), "mycore".into());
    let e = virtuoso::export(&d, &tops, &Mapping::default(), &opts).unwrap();
    assert!(e.sfyx.contains("(\"cell\" \"res\" \"design\")"));
    assert!(
        e.sfyx.contains("(\"componentName\" \"resistor\")"),
        "§6.5 simInfo"
    );
    assert!(
        e.sfyx
            .contains("(\"prop\" 1 \"srcType\" \"string\" \"pulse\")")
            || e.sfyx.contains("\"srcType\" \"string\" \"pulse\"")
    );
    let imp =
        virtuoso::import(&e.sfyx, &Files::new(), &Mapping::default(), &import_opts()).unwrap();
    for (name, bytes) in schemify_doc::core_library() {
        let path = format!("mycore/{name}");
        let got = imp
            .files
            .iter()
            .find(|f| f.0 == path)
            .unwrap_or_else(|| panic!("{path} missing"));
        assert_eq!(
            strip_fp(&String::from_utf8_lossy(&got.1)),
            canonical(bytes),
            "{path}"
        );
    }
    // Without the stash the core cells keep pins, params and graphics (§7.4 losses only).
    let bare = virtuoso::import(
        &strip_stash(&e.sfyx),
        &Files::new(),
        &Mapping::default(),
        &import_opts(),
    )
    .unwrap();
    for (name, bytes) in schemify_doc::core_library() {
        let path = format!("mycore/{name}");
        let got = bare.files.iter().find(|f| f.0 == path).unwrap();
        let mut bad =
            undocumented_losses(&path, &canonical(bytes), &String::from_utf8_lossy(&got.1));
        // A core source's `value` is SPICE text in Schemify and waveform params in its
        // CDF (§6.6): documented.
        bad.retain(|b| {
            !(name.starts_with("vsource") || name.starts_with("isource")) || !b.contains("params")
        });
        assert!(bad.is_empty(), "{bad:#?}");
    }
}

// ---------------------------------------------------------------------------------------
// Generated projects (property)
// ---------------------------------------------------------------------------------------

/// A small deterministic generator driven by one proptest seed.
struct Gen {
    s: u64,
    ids: BTreeSet<String>,
}

impl Gen {
    fn next(&mut self) -> u64 {
        self.s ^= self.s << 13;
        self.s ^= self.s >> 7;
        self.s ^= self.s << 17;
        self.s
    }
    fn n(&mut self, k: u64) -> u64 {
        self.next() % k
    }
    fn coin(&mut self) -> bool {
        self.n(2) == 0
    }
    fn pick<'a>(&mut self, v: &[&'a str]) -> &'a str {
        v[self.n(v.len() as u64) as usize]
    }
    /// A grid coordinate (xschem units, 10-unit grid).
    fn c(&mut self) -> i64 {
        (self.n(40) as i64 - 20) * 10
    }
    fn id(&mut self) -> String {
        const A: &[u8] = b"0123456789abcdefghjkmnpqrstvwxyz";
        loop {
            let s: String = (0..6).map(|_| A[self.n(32) as usize] as char).collect();
            if self.ids.insert(s.clone()) {
                return s;
            }
        }
    }
}

fn gen_symbol(g: &mut Gen, t: &mut String, params: &[&str]) {
    for _ in 0..1 + g.n(4) {
        match g.n(5) {
            0 => t.push_str(&format!(
                "line {} {} {} {}\n",
                g.c(),
                g.c(),
                g.c() + 10,
                g.c()
            )),
            1 => t.push_str(&format!(
                "rect {} {} {} {}\n  fill true\n",
                g.c(),
                g.c(),
                g.c() + 10,
                g.c() + 10
            )),
            2 => t.push_str(&format!("circle {} {} {}\n", g.c(), g.c(), 5 + 5 * g.n(3))),
            3 => t.push_str(&format!(
                "arc {} {} 10 {} {}\n",
                g.c(),
                g.c(),
                90 * g.n(4),
                90 * (1 + g.n(3))
            )),
            _ => {
                // A real triangle: collinear points are no polygon (Virtuoso needs three
                // distinct corners; the exporter sends a degenerate one as a line).
                let (x, y) = (g.c(), g.c());
                t.push_str(&format!(
                    "poly {x} {y} {} {y} {x} {}\n  closed true\n",
                    x + 20,
                    y + 20
                ))
            }
        }
    }
    t.push_str(&format!(
        "stext {} {} R0 {{name}}\n  size 0.2\n",
        g.c(),
        g.c()
    ));
    if let Some(p) = params.first() {
        t.push_str(&format!(
            "stext {} {} R90 {p}={{{p}}}\n  size 0.15\n  hcenter\n",
            g.c(),
            g.c()
        ));
    }
}

/// Rank-3 objects: wires, labels (on and off wires, globals, buses), ports of `pins`,
/// noconns, texts, shapes.
fn gen_schematic(g: &mut Gen, t: &mut String, pins: &[&str], bus_ok: bool) -> Vec<String> {
    let mut wires = Vec::new();
    for _ in 0..g.n(6) {
        let (x, y) = (g.c(), g.c());
        let (x2, y2) = if g.coin() {
            (x + 10 * (1 + g.n(5) as i64), y)
        } else {
            (x, y + 10 * (1 + g.n(5) as i64))
        };
        let id = g.id();
        t.push_str(&format!("wire {id} {x} {y} {x2} {y2}\n"));
        if bus_ok && g.n(4) == 0 {
            t.push_str("  bus true\n");
        }
        wires.push((x, y, x2, y2));
    }
    for _ in 0..g.n(5) {
        let (x, y) = match wires.get(g.n(wires.len() as u64 + 1) as usize) {
            Some(w) if g.coin() => (w.0, w.1),
            _ => (g.c(), g.c()),
        };
        let net = g.pick(&[
            "a", "net1", "VDD", "0", "d[3:0]", "2*b", "q[1]", "x!", "n_2",
        ]);
        let id = g.id();
        let o = g.pick(&["R0", "R90", "R180", "MR0"]);
        t.push_str(&format!("label {id} {x} {y} {o} {net}\n"));
        if net == "VDD" && g.coin() {
            t.push_str("  global true\n");
        }
    }
    for p in pins {
        let id = g.id();
        t.push_str(&format!(
            "port {id} {} {} {} {p}\n",
            g.c(),
            g.c(),
            g.pick(&["R0", "R180", "R270"])
        ));
    }
    for _ in 0..g.n(2) {
        let id = g.id();
        t.push_str(&format!("noconn {id} {} {}\n", g.c(), g.c()));
    }
    for _ in 0..g.n(2) {
        let id = g.id();
        t.push_str(&format!(
            "text {id} {} {} R0 \"note {}\"\n  size 0.3\n",
            g.c(),
            g.c(),
            g.n(100)
        ));
    }
    if g.coin() {
        let id = g.id();
        t.push_str(&format!(
            "shape {id} rect {} {} {} {}\n  dash 1\n",
            g.c(),
            g.c(),
            g.c() + 30,
            g.c() + 30
        ));
    }
    if g.coin() {
        let id = g.id();
        t.push_str(&format!("shape {id} circle {} {} 15\n", g.c(), g.c()));
    }
    Vec::new()
}

/// A project: `dev/dev.shprim` (a device with spectre and spice formats), `sub.shcomp`
/// (bus, inherited and alias pins, params of every kind, docs), `top.shcomp` (iterated
/// instances, overrides of every kind, waivers, constraints) and `tb.shbench`.
fn gen_project(seed: u64) -> Files {
    let mut g = Gen {
        s: seed | 1,
        ids: BTreeSet::new(),
    };
    let mut f = Files::new();
    f.insert(
        "schemify.project".into(),
        b"schemify 1 project\n\nlib dev path dev\n".to_vec(),
    );
    // The device.
    let mut dev = String::from(
        "schemify 1 shprim\n  doc \"generated device\"\n  prefix M\n\npin d inout 20 -30\npin g in -20 0\npin s inout 20 30\npin b in 20 0\n  inherit VSS\n\n",
    );
    dev.push_str("param w 1u\n  unit m\n  min 0\nparam l 180n\n  unit m\nparam nf 1\n  kind int\nparam model nch\n  kind string\nparam flavor lvt\n  kind choice\n  choices lvt hvt\n\n");
    dev.push_str("impl spice spice\n  format spice \"M{name} {d} {g} {s} {b} {model} w={w} l={l} nf={nf}\"\n  format spectre \"{name} ({d} {g} {s} {b}) {model} w={w} l={l} nf={nf}\"\n\n");
    gen_symbol(&mut g, &mut dev, &["w"]);
    f.insert("dev/dev.shprim".into(), dev.into_bytes());
    // The component.
    let mut sub = String::from(
        "schemify 1 shcomp\n  doc \"generated component\"\n\npin A in -40 0\npin Y out 40 0\npin D[3:0] inout 0 -40\npin VDD inout 0 40\n  inherit VDD\npin Y2 out 40 20\n  alias Y\n\n",
    );
    sub.push_str("param k 2*n\n  unit Ohm\n  max 10k\nparam n 3\n  kind int\nparam s abc\n  kind string\nparam c x\n  kind choice\n  choices x y\nparam on true\n  kind bool\nparam kk 1\n  computed k*2\n\nimpl sch schematic\n\n");
    gen_symbol(&mut g, &mut sub, &["k"]);
    let di = g.id();
    sub.push_str(&format!(
        "inst {di} dev:dev\n  at 100 0 R0\n  name M1\n  w k*1u\n  nf 2\n"
    ));
    gen_schematic(&mut g, &mut sub, &["A", "Y", "D[3:0]"], true);
    sub.push_str(&format!(
        "\nprose {}\n  | # Sub\n  |\n  | $x^2$\nvar wq 2*k\n  unit m\n",
        g.id()
    ));
    f.insert("sub.shcomp".into(), sub.into_bytes());
    // The top.
    let mut top = String::from(
        "schemify 1 shcomp\n\npin IN in -40 0\npin OUT out 40 0\n\nparam p 1k\n\nimpl sch schematic\n\n",
    );
    gen_symbol(&mut g, &mut top, &["p"]);
    let x1 = g.id();
    let name = g.pick(&["X1", "X1[1:0]"]);
    top.push_str(&format!(
        "inst {x1} ./sub\n  at 0 0 {}\n  name {name}\n",
        g.pick(&["R0", "R90", "MR0", "MR270"])
    ));
    if g.coin() {
        top.push_str("  k 2*p\n");
    }
    if g.coin() {
        top.push_str("  n 5\n  s hello\n  c y\n  on false\n");
    }
    if g.coin() {
        top.push_str("  inherit VDD vdda\n");
    }
    let r1 = g.id();
    top.push_str(&format!(
        "inst {r1} core:res\n  at 200 0 R0\n  name R1\n  value {}\n",
        g.pick(&["2k", "1meg", "4.7k"])
    ));
    let v1 = g.id();
    top.push_str(&format!(
        "inst {v1} core:vsource\n  at 300 0 R0\n  name V1\n  value \"{}\"\n",
        g.pick(&["dc 1", "DC 1 AC 1", "pulse(0 1 0 1n 1n 5n 10n)"])
    ));
    if g.coin() {
        top.push_str("  ignore true\n");
    }
    gen_schematic(&mut g, &mut top, &["IN", "OUT"], true);
    top.push_str(&format!(
        "\nconstraint {} match\n  member {x1}\n  member {r1}\n",
        g.id()
    ));
    top.push_str(&format!(
        "waive {} {r1} value\n  reason \"generated\"\n",
        g.id()
    ));
    f.insert("top.shcomp".into(), top.into_bytes());
    // The bench.
    let mut tb = String::from("schemify 1 shbench\n  corner tt\n\n");
    let t1 = g.id();
    tb.push_str(&format!(
        "inst {t1} ./top\n  at 0 0 R0\n  name X0\n  p vbias*2\n"
    ));
    gen_schematic(&mut g, &mut tb, &[], false);
    tb.push_str("\nvar vbias 1k\n\ncorner tt\n  temp 27\n\noutput vout \"at(v(OUT), 1n)\"\n\nspec vout\n  min 0.1\n\ncode tran spice\n  | .tran 1n 10n\n");
    f.insert("tb.shbench".into(), tb.into_bytes());
    f
}

proptest::proptest! {
    #![proptest_config(proptest::prelude::ProptestConfig { cases: 48, failure_persistence: None, ..Default::default() })]
    #[test]
    fn generated_projects_round_trip(seed in proptest::prelude::any::<u64>()) {
        let files = gen_project(seed);
        round_trip(&files, &format!("seed {seed}"));
    }
}

// ---------------------------------------------------------------------------------------
// Merge with existing files (§7.3)
// ---------------------------------------------------------------------------------------

fn file<'a>(files: &'a [(String, Vec<u8>)], path: &str) -> &'a str {
    std::str::from_utf8(
        &files
            .iter()
            .find(|f| f.0 == path)
            .unwrap_or_else(|| panic!("{path} missing"))
            .1,
    )
    .unwrap()
}

/// Inserts `extra` after the first line matching `after` (a crude local edit).
fn edit(text: &str, after: &str, extra: &str) -> String {
    let mut out = String::new();
    let mut done = false;
    for l in text.lines() {
        out.push_str(l);
        out.push('\n');
        if !done && l == after {
            out.push_str(extra);
            done = true;
        }
    }
    assert!(done, "no line `{after}` in\n{text}");
    out
}

#[test]
fn import_onto_existing_files_keeps_schemify_only_data() {
    let text = std::fs::read_to_string(fixtures().join("pdk.sfyx")).unwrap();
    let first = virtuoso::import(
        &text,
        &Files::new(),
        &Mapping::default(),
        &ImportOptions::default(),
    )
    .unwrap();
    let dev = file(&first.files, "demopdk/nmos_demo.shprim");
    // A teammate documents the device in Schemify: header doc, a bound, a spice format.
    let mut local = edit(
        dev,
        "  provenance \"virtuoso demopdk/nmos_demo\"",
        "  doc \"thin-oxide nmos\"\n",
    );
    local = edit(&local, "  doc \"Gate width\"", "  min 120n\n  max 100u\n");
    local = edit(
        &local,
        "impl spice spice",
        "  format spice \"X{name} {D} {G} {S} {B} nmos_demo w={w} l={l}\"\n",
    );
    let local = canonical(local.as_bytes());
    let mut existing: Files = first.files.iter().cloned().collect();
    existing.insert(
        "demopdk/nmos_demo.shprim".into(),
        local.clone().into_bytes(),
    );
    // Unchanged in Virtuoso: the local file stays as it is.
    let again = virtuoso::import(
        &text,
        &existing,
        &Mapping::default(),
        &ImportOptions::default(),
    )
    .unwrap();
    assert_eq!(
        strip_fp(file(&again.files, "demopdk/nmos_demo.shprim")),
        strip_fp(&local)
    );
    // Changed in Virtuoso (a new default for w): the CDF wins, the bound, doc and spice
    // format stay.
    let changed = text.replace("\"w\" \"string\" \"300n\"", "\"w\" \"string\" \"240n\"");
    let m = virtuoso::import(
        &changed,
        &existing,
        &Mapping::default(),
        &ImportOptions::default(),
    )
    .unwrap();
    let got = file(&m.files, "demopdk/nmos_demo.shprim");
    assert!(got.contains("param w 240n"), "{got}");
    for kept in [
        "doc \"thin-oxide nmos\"",
        "  min 120n",
        "  max 100u",
        "format spice \"X{name}",
    ] {
        assert!(got.contains(kept), "{kept} lost:\n{got}");
    }
    assert!(
        m.notes
            .rows
            .iter()
            .any(|r| r.class == "updated" && r.cell == "demopdk/nmos_demo.shprim")
    );
}

#[test]
fn stale_schemify_data_is_dropped_with_a_note() {
    let text = std::fs::read_to_string(fixtures().join("inverter.sfyx")).unwrap();
    let first = virtuoso::import(
        &text,
        &Files::new(),
        &Mapping::default(),
        &ImportOptions::default(),
    )
    .unwrap();
    let tb = file(&first.files, "demo/tb_inv.shbench").to_owned();
    // Docs, a bench variable and a waiver on V0 survive; a waiver on a deleted instance
    // goes.
    let v0 = tb
        .lines()
        .find(|l| l.starts_with("inst ") && tb.contains("name V0"))
        .unwrap();
    let v0_id = tb
        .split("\ninst ")
        .find(|b| b.contains("name V0"))
        .and_then(|b| b.split_whitespace().next())
        .unwrap()
        .to_owned();
    let _ = v0;
    let local = format!(
        "{tb}\nwaive 0zzzzz {v0_id} vdc\n  reason \"supply is fixed\"\nwaive 1zzzzz 9zzzzz vdc\n  reason \"old instance\"\n\nprose 2zzzzz\n  | # The bench\n\nvar vdd 1.8\n"
    );
    let local = canonical(local.as_bytes());
    let mut existing: Files = first.files.iter().cloned().collect();
    existing.insert("demo/tb_inv.shbench".into(), local.into_bytes());
    let m = virtuoso::import(
        &text,
        &existing,
        &Mapping::default(),
        &ImportOptions::default(),
    )
    .unwrap();
    let got = file(&m.files, "demo/tb_inv.shbench");
    assert!(got.contains("waive 0zzzzz"), "{got}");
    assert!(!got.contains("waive 1zzzzz"), "{got}");
    assert!(
        got.contains("prose 2zzzzz") && got.contains("var vdd 1.8"),
        "{got}"
    );
    let stale: Vec<_> = m
        .notes
        .rows
        .iter()
        .filter(|r| r.class == "dropped:stale-schemify-data")
        .collect();
    assert_eq!(stale.len(), 1, "{:?}", m.notes.rows);
    assert!(stale[0].detail.contains("1zzzzz"));
}

#[test]
fn reimporting_an_unchanged_dump_is_a_no_op() {
    for name in ["inverter", "bus", "symbol", "pdk", "netexpr"] {
        let text = std::fs::read_to_string(fixtures().join(format!("{name}.sfyx"))).unwrap();
        let first = virtuoso::import(
            &text,
            &Files::new(),
            &Mapping::default(),
            &ImportOptions::default(),
        )
        .unwrap();
        let existing: Files = first.files.iter().cloned().collect();
        let again = virtuoso::import(
            &text,
            &existing,
            &Mapping::default(),
            &ImportOptions::default(),
        )
        .unwrap();
        for (p, b) in &again.files {
            assert_eq!(
                String::from_utf8_lossy(b),
                String::from_utf8_lossy(&existing[p]),
                "{name}: {p}"
            );
        }
        assert!(
            !again.notes.rows.iter().any(|r| r.class == "updated"),
            "{name}: {:?}",
            again.notes.rows
        );
    }
    // Also for a project exported from Schemify and imported twice.
    for dir in fixture_projects() {
        let files = project(&dir);
        let (text, _) = export_all(&files);
        let man: Files = files
            .iter()
            .filter(|(p, _)| *p == "schemify.project")
            .map(|(p, b)| (p.clone(), b.clone()))
            .collect();
        let first = virtuoso::import(&text, &man, &Mapping::default(), &import_opts()).unwrap();
        let mut existing = man.clone();
        existing.extend(first.files.iter().cloned());
        let again =
            virtuoso::import(&text, &existing, &Mapping::default(), &import_opts()).unwrap();
        for (p, b) in &again.files {
            assert_eq!(
                String::from_utf8_lossy(b),
                String::from_utf8_lossy(&existing[p]),
                "{}: {p}",
                dir.display()
            );
        }
    }
}

// ---------------------------------------------------------------------------------------
// Re-mastering keeps every net (§6.4)
// ---------------------------------------------------------------------------------------

/// Per instance id, per pin index, the set of (instance id, pin) it shares a net with:
/// the net partition over instance pins, independent of net names.
fn partition(
    d: &Design,
    ci: usize,
    pin_names: &dyn Fn(&str, usize) -> String,
) -> BTreeSet<BTreeSet<String>> {
    let g = interop::design::geom(d, ci);
    let n = interop::nets::extract(d, ci, &g);
    let mut by_net: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (k, pins) in n.inst_pin.iter().enumerate() {
        for (p, node) in pins.iter().enumerate() {
            if *node == interop::nets::NO_NODE {
                continue;
            }
            // Same-named nets are one net (elab connects labels by name).
            let net = &n.net_name[n.node_net[*node as usize] as usize];
            by_net.entry(net.clone()).or_default().insert(format!(
                "{}.{}",
                g.inst_id[k],
                pin_names(&g.inst_id[k], p)
            ));
        }
    }
    by_net.into_values().collect()
}

#[test]
fn remastered_devices_keep_every_net() {
    // A bench of core resistors and a source; `dev:res2` has the same pins elsewhere.
    let mut f = Files::new();
    f.insert(
        "schemify.project".into(),
        b"schemify 1 project\n\nlib dev path dev\n".to_vec(),
    );
    f.insert(
        "dev/res2.shprim".into(),
        b"schemify 1 shprim\n\npin a inout 0 -50\npin b inout 0 50\n\nparam r 1k\n\nimpl spice spice\n  format spice \"R{name} {a} {b} {r}\"\n\nrect -10 -40 10 40\n".to_vec(),
    );
    f.insert(
        "tb.shbench".into(),
        b"schemify 1 shbench\n\ninst aaaaa1 core:res\n  at 0 0 R0\n  name R1\ninst aaaaa2 core:res\n  at 0 60 R0\n  name R2\ninst aaaaa3 core:res\n  at 100 0 R90\n  name R3\ninst aaaaa4 core:vsource\n  at -100 0 R0\n  name V1\n\nwire bbbbb1 0 30 0 30.5\nwire bbbbb2 -100 -30 0 -30\nwire bbbbb3 -100 30 -100 120\nwire bbbbb4 -100 120 0 120\nwire bbbbb5 0 90 0 120\nwire bbbbb6 70 0 0 0\n\nlabel ccccc1 130 0 R0 out\nlabel ccccc2 -100 120 R0 0\n".to_vec(),
    );
    let d = design(&f);
    let ci = d.find("tb.shbench").unwrap();
    let g = interop::design::geom(&d, ci);
    let n = interop::nets::extract(&d, ci, &g);
    let res2 = interop::design::sym_pins(&d.cells[d.find("dev/res2.shprim").unwrap()]);
    let todo: Vec<interop::remap::Remaster> = (0..g.inst_id.len())
        .filter(|&k| d.cells[g.inst_master[k] as usize].name == "res")
        .map(|k| interop::remap::Remaster {
            inst: k,
            pins: vec![(0, "a".into()), (1, "b".into())],
            new_pins: Some(
                res2.name
                    .iter()
                    .cloned()
                    .zip(res2.at.iter().copied())
                    .collect(),
            ),
            at: None,
        })
        .collect();
    let e = interop::remap::apply(&g, &n, &todo);
    assert!(!e.stubs.is_empty() && !e.labels.is_empty());
    let masters: Vec<interop::remap::NewMaster> = todo
        .iter()
        .map(|t| (t.inst, "dev:res2".to_owned(), None))
        .collect();
    let mut k = 0;
    let doc = interop::remap::apply_to_doc(&d.cells[ci].doc, &g, &e, &masters, || {
        k += 1;
        format!("zz{k:04}")
    });
    let mut f2 = f.clone();
    f2.insert(
        "tb.shbench".into(),
        schemify_format::write(&doc).into_bytes(),
    );
    let d2 = design(&f2);
    let ci2 = d2.find("tb.shbench").unwrap();
    // Old pin p of a re-mastered res is new pin p of res2 (same index).
    let names = |_: &str, p: usize| p.to_string();
    assert_eq!(
        partition(&d, ci, &names),
        partition(&d2, ci2, &names),
        "\n{}",
        schemify_format::write(&doc)
    );
    // And the edits are what §6.4 says: every new pin got a stub, nothing was left
    // dangling at an old pin point.
    assert_eq!(e.stubs.len(), 2 * todo.len());
}

#[test]
fn analoglib_import_remasters_onto_core_cells_keeping_nets() {
    let text = std::fs::read_to_string(fixtures().join("inverter.sfyx")).unwrap();
    let plain = virtuoso::import(
        &text,
        &Files::new(),
        &Mapping::default(),
        &ImportOptions::default(),
    )
    .unwrap();
    let opts = ImportOptions {
        analoglib: true,
        ..ImportOptions::default()
    };
    let core = virtuoso::import(&text, &Files::new(), &Mapping::default(), &opts).unwrap();
    let tb = file(&core.files, "demo/inv.shcomp");
    assert!(
        tb.contains("core:nmos4") && tb.contains("core:pmos4"),
        "{tb}"
    );
    assert!(core.notes.rows.iter().any(|r| r.class == "remapped"));
    let bench = file(&core.files, "demo/tb_inv.shbench");
    assert!(
        bench.contains("core:vsource") && bench.contains("value \"DC 1.8\""),
        "{bench}"
    );
    let pf: Files = plain.files.iter().cloned().collect();
    let cf: Files = core.files.iter().cloned().collect();
    let (dp, dc) = (design(&pf), design(&cf));
    for cell in ["demo/inv.shcomp", "demo/tb_inv.shbench"] {
        let names = |_: &str, p: usize| p.to_string();
        assert_eq!(
            partition(&dp, dp.find(cell).unwrap(), &names),
            partition(&dc, dc.find(cell).unwrap(), &names),
            "{cell}\n{}",
            file(&core.files, cell)
        );
    }
}

// ---------------------------------------------------------------------------------------
// Negative space: malformed dumps, unknown records, export refusals (§4, §8.2, §12)
// ---------------------------------------------------------------------------------------

#[test]
fn malformed_or_truncated_dumps_are_refused_with_the_line() {
    let head = "(\"sfyx\" 1 \"t\")\n(\"lib\" \"L\")\n(\"cell\" \"c\" \"design\")\n(\"cv\" \"schematic\" \"schematic\" 160)\n";
    for (body, line, why) in [
        (
            "(\"inst\" 1 \"I0\" \"a\" \"b\" \"symbol\" 0 0)\n",
            5,
            "fields",
        ),
        (
            "(\"inst\" 1 \"I0\" \"a\" \"b\" \"symbol\" 0 0 \"R45\")\n",
            5,
            "orientation",
        ),
        ("(\"wire\" 1 (0 0 10) 0)\n", 5, "even"),
        ("(\"wire\" 0 (0 0 10 0) 0)\n", 5, "positive"),
        (
            "(\"label\" 1 7 \"n\" 0 0 \"R0\" \"lowerLeft\" \"stick\" 10 \"normal\" \"wire\" \"label\")\n",
            3,
            "unknown ref",
        ),
        ("(\"prop\" 9 \"x\" \"string\" \"1\")\n", 3, "unknown ref"),
        ("(\"prop\" 0 \"x\" \"blob\" \"1\")\n", 5, "type"),
        (
            "(\"shape\" 1 \"blob\" \"text\" \"drawing\" (0 0 1 1) 0)\n",
            5,
            "kind",
        ),
        (
            "(\"wire\" 1 (0 0 10 0) 0)\n(\"wire\" 1 (0 0 0 10) 0)\n",
            6,
            "twice",
        ),
        (
            "(\"param\" \"w\" \"string\" \"1\" \"w\" \"\" () () \"\")\n",
            5,
            "cdf",
        ),
        ("(\"var\" \"x\" \"1\" \"\")\n", 5, "maestro"),
        ("(\"wire\" 1 (0 0 1.5 0) 0)\n", 5, "float"),
        ("(\"wire\" 1 (0 0 010 0) 0)\n", 5, "leading zero"),
    ] {
        let t = format!("{head}{body}(\"end\")\n");
        let e = virtuoso::import(
            &t,
            &Files::new(),
            &Mapping::default(),
            &ImportOptions::default(),
        )
        .expect_err(body);
        assert!(
            e.iter()
                .any(|m| m.contains(&format!("line {line}")) && m.to_lowercase().contains(why)),
            "{body}: {e:?}"
        );
    }
    let e = virtuoso::import(
        head,
        &Files::new(),
        &Mapping::default(),
        &ImportOptions::default(),
    )
    .expect_err("truncated");
    assert!(e[0].contains("truncated"), "{e:?}");
    let e = virtuoso::import(
        "(\"inst\" 1)\n",
        &Files::new(),
        &Mapping::default(),
        &ImportOptions::default(),
    )
    .expect_err("no header");
    assert!(e[0].contains("sfyx v1"), "{e:?}");
    assert!(virtuoso::import_targets(head).is_err());
    // An object outside any cellview.
    let t = "(\"sfyx\" 1 \"t\")\n(\"wire\" 1 (0 0 1 0) 0)\n(\"end\")\n";
    let e = virtuoso::import(
        t,
        &Files::new(),
        &Mapping::default(),
        &ImportOptions::default(),
    )
    .unwrap_err();
    assert!(
        e[0].contains("line 2") && e[0].contains("outside a cellview"),
        "{e:?}"
    );
}

#[test]
fn unknown_records_are_reported_never_dropped_silently() {
    let t = "(\"sfyx\" 1 \"t\")\n(\"frob\" 1)\n(\"lib\" \"L\")\n(\"cell\" \"c\" \"design\")\n(\"cv\" \"schematic\" \"schematic\" 160)\n(\"wire\" 1 (0 0 10 0) 0)\n(\"inst\" 2 \"I0\" \"L\" \"x\" \"symbol\" 0 0 \"R0\")\n(\"blink\" 3 \"x\")\n(\"conn\" 2 \"A\" \"n1\")\n(\"intent\" 2 \"nmos\" ((\"gmid\" \"10\")))\n(\"shape\" 4 \"ellipse\" \"wire\" \"drawing\" (0 0 2 2) 0)\n(\"label\" 5 0 \"x\" 0 0 \"R0\" \"lowerLeft\" \"stick\" 10 \"normal\" \"instance\" \"label\")\n(\"prop\" 0 \"lastSchematicExtraction\" \"string\" \"t\")\n(\"end\")\n";
    let imp = virtuoso::import(
        t,
        &Files::new(),
        &Mapping::default(),
        &ImportOptions::default(),
    )
    .unwrap();
    let c = imp.notes.counts();
    assert_eq!(
        c.get("dropped:unknown-record"),
        Some(&3),
        "{}",
        imp.notes.text()
    ); // frob, blink, intent
    assert!(
        imp.notes.text().contains("`blink` on line 8"),
        "{}",
        imp.notes.text()
    );
    assert!(imp.notes.text().contains("solder-dot"));
    assert!(imp.notes.text().contains("lastSchematicExtraction"));
    assert!(imp.notes.text().contains("conn A n1"));
    // The count conservation asserts inside the import held (it returned at all); the
    // wire and the instance landed.
    let f = file(&imp.files, "L/c.shbench");
    assert!(f.contains("wire ") && f.contains("inst "), "{f}");
}

#[test]
fn export_refuses_a_mapping_that_leaves_a_pin_unmapped() {
    let mut f = Files::new();
    f.insert(
        "schemify.project".into(),
        b"schemify 1 project\n\nlib dev path dev\n".to_vec(),
    );
    f.insert(
        "dev/nfet.shprim".into(),
        b"schemify 1 shprim\n\npin d inout 0 -30\npin g in -20 0\npin s inout 0 30\n\nparam W 1\n\nimpl spice spice\n\nline 0 -30 0 30\n".to_vec(),
    );
    f.insert(
        "tb.shbench".into(),
        b"schemify 1 shbench\n\ninst aaaaaa dev:nfet\n  at 0 0 R0\n  name M1\n  W 2\n".to_vec(),
    );
    let d = design(&f);
    let tops = [d.find("tb.shbench").unwrap()];
    let bad = Mapping::parse("map dev:nfet virtuoso pdk nch\n  pin d D\n  pin g G\n  param L l\n")
        .unwrap();
    let e = virtuoso::export(&d, &tops, &bad, &ExportOptions::default()).unwrap_err();
    assert!(
        e.iter().any(|m| m.contains("pin s has no `pin` row")),
        "{e:?}"
    );
    assert!(e.iter().any(|m| m.contains("no param L")), "{e:?}");
    // A complete row re-masters with `conn` records (the target's geometry is unknown)
    // and scales the width.
    let good = Mapping::parse("map dev:nfet virtuoso pdk nch\n  pin d D\n  pin g G\n  pin s S\n  param W w scale 1e-6\n  set mult 1\n").unwrap();
    let x = virtuoso::export(&d, &tops, &good, &ExportOptions::default()).unwrap();
    assert!(
        x.sfyx
            .contains("(\"inst\" 1 \"M1\" \"pdk\" \"nch\" \"symbol\" 0 0 \"R0\")"),
        "{}",
        x.sfyx
    );
    assert!(
        x.sfyx.contains("\"w\" \"string\" \"0.000002\"")
            || x.sfyx.contains("\"w\" \"string\" \"2e-6\""),
        "{}",
        x.sfyx
    );
    assert!(
        x.sfyx.contains("(\"prop\" 1 \"mult\" \"string\" \"1\")"),
        "{}",
        x.sfyx
    );
    assert_eq!(x.sfyx.matches("(\"conn\" 1 ").count(), 3, "{}", x.sfyx);
    assert!(x.notes.rows.iter().any(|r| r.class == "remapped"));
}

// ---------------------------------------------------------------------------------------
// §3.4 property: AEL translation preserves the value
// ---------------------------------------------------------------------------------------

fn eval(src: &str, w: f64) -> Option<f64> {
    use schemify_expr::{Mode, Program, Scratch, Value};
    let p = Program::compile(src, Mode::Scalar).ok()?;
    let mut inputs = Vec::new();
    p.bind(&mut inputs, |n| (n == "w").then_some(Value::Num(w)))
        .ok()?;
    match p.eval(&inputs, &mut Scratch::default()).ok()? {
        Value::Num(x) => Some(x),
        Value::Int(i) => Some(i as f64),
        _ => None,
    }
}

fn arb_expr() -> impl proptest::strategy::Strategy<Value = String> {
    use proptest::prelude::*;
    let num = (
        1u32..999,
        0u32..3,
        proptest::sample::select(vec![
            "", "f", "p", "n", "u", "m", "M", "k", "K", "meg", "MEG", "g", "t", "mil",
        ]),
    )
        .prop_map(|(m, d, s)| {
            if d == 0 {
                format!("{m}{s}")
            } else {
                format!("{}.{}{s}", m / 10 + 1, m % 10)
            }
        });
    let leaf = prop_oneof![num, Just("w".to_owned()), Just("pi".to_owned())];
    leaf.prop_recursive(4, 24, 3, |inner| {
        prop_oneof![
            (
                inner.clone(),
                proptest::sample::select(vec!["+", "-", "*", "/"]),
                inner.clone()
            )
                .prop_map(|(a, o, b)| format!("{a} {o} {b}")),
            inner.clone().prop_map(|a| format!("-({a})")),
            inner.clone().prop_map(|a| format!("({a})")),
            (inner.clone(), 1u32..3).prop_map(|(a, k)| format!("({a})^{k}")),
            (
                inner.clone(),
                proptest::sample::select(vec![
                    "abs", "sqrt", "exp", "ln", "sin", "cos", "atan", "floor", "ceil"
                ])
            )
                .prop_map(|(a, f)| format!("{f}(abs({a}) + 1)")),
            (inner.clone(), inner).prop_map(|(a, b)| format!("max({a}, {b})")),
        ]
    })
}

proptest::proptest! {
    #![proptest_config(proptest::prelude::ProptestConfig { cases: 512, failure_persistence: None, ..Default::default() })]
    #[test]
    fn ael_translation_preserves_values(e in arb_expr(), w in 1e-9f64..1e3) {
        use virtuoso::ael::{AelName, Name, from_ael, to_ael};
        let Some(want) = eval(&e, w) else { return Ok(()) };
        let a = to_ael(&e, |n| if n == "w" { Name::ParentParam } else { Name::Unknown }).unwrap();
        let back = from_ael(&a.text, |n| if n == "w" { AelName::Param } else { AelName::Unknown }, |_| None).unwrap();
        let got = eval(&back.text, w).unwrap_or_else(|| panic!("{e} → {} → {} does not evaluate", a.text, back.text));
        let tol = 1e-12 * want.abs().max(1e-300);
        proptest::prop_assert!((got - want).abs() <= tol, "{} → {} → {}: {} vs {}", e, a.text, back.text, want, got);
    }
}

/// A seed the generator once failed on: a closed `poly` whose points collapse (spec §6.2).
#[test]
fn generated_project_regression_seeds() {
    let seed = 14_947_292_216_081_295_580_u64;
    round_trip(&gen_project(seed), &format!("seed {seed}"));
}

/// A text size whose height leaves `i32` exports at the largest whole-unit height: the
/// height computation overflowed (a panic in debug builds) before.
#[test]
fn a_huge_text_size_exports_saturated() {
    let mut files = Files::new();
    files.insert("schemify.project".into(), b"schemify 1 project\n".to_vec());
    let sym = "schemify 1 shprim\n\npin a inout 0 0\n\nimpl spice spice\n  format spice \"{name} {a} big\"\n\nstext 10 0 R0 hello\n  size 300000\n";
    files.insert("big.shprim".into(), sym.as_bytes().to_vec());
    let (text, _) = export_all(&files);
    let label = sfyx::read(&text)
        .unwrap()
        .into_iter()
        .find(|r| r.tag == "label" && r.str(2) == Ok("hello"))
        .expect("the stext goes out as a label");
    assert_eq!(label.int(8), Ok(i32::MAX / 1000 * 1000));
}

/// A pass-through of the parent's params (§3.4): a string-kind override that is one hole
/// exports as its `pPar` expression, the parent's CDF carries the params' defaults, and
/// import gives the holes back.
#[test]
fn a_parent_param_hole_crosses_as_ppar() {
    let mut f = Files::new();
    f.insert(
        "schemify.project".into(),
        b"schemify 1 project\n\nlib dev path dev\n".to_vec(),
    );
    f.insert(
        "dev/dev.shprim".into(),
        b"schemify 1 shprim\n  prefix M\n\npin d inout 0 -20\npin g in -20 0\npin s inout 0 20\n\n\
          param w 1u\n  kind string\nparam l 1u\n  kind string\n\n\
          impl spice spice\n  format spice \"M{name} {d} {g} {s} nch w={w} l={l}\"\n\n\
          stext 0 0 R0 {name}\n  size 0.2\n"
            .to_vec(),
    );
    f.insert(
        "amp.shcomp".into(),
        b"schemify 1 shcomp\n\npin A in -40 0\n\nparam w 80n\nparam l 20n\n\n\
          impl sch schematic\n\nstext 0 0 R0 {name}\n  size 0.2\n\n\
          inst a0m0 dev:dev\n  at 100 0 R0\n  name m0\n  w {w}\n  l {l*2}\n"
            .to_vec(),
    );
    let d = design(&f);
    let tops: Vec<usize> = (0..d.cells.len()).collect();
    let mut opts = ExportOptions::default();
    opts.lib_map.insert(String::new(), "top".into());
    let e = virtuoso::export(&d, &tops, &Mapping::default(), &opts).unwrap();
    assert!(!e.notes.text().contains("template"), "{}", e.notes.text());
    for want in [
        r#""w" "string" "pPar(\"w\")")"#,
        r#""l" "string" "pPar(\"l\")*2")"#,
    ] {
        assert!(e.sfyx.contains(want), "{want}\n{}", e.sfyx);
    }
    let amp = e.sfyx.split("(\"cell\" \"amp\"").nth(1).unwrap();
    let cdf = amp.split("(\"cv\"").next().unwrap();
    for (p, v) in [("w", "80n"), ("l", "20n")] {
        assert!(
            cdf.lines()
                .any(|l| l.starts_with(&format!("(\"param\" \"{p}\" \"string\" \"{v}\""))),
            "{p}:\n{cdf}"
        );
    }
    let imp = virtuoso::import(
        &strip_stash(&e.sfyx),
        &Files::new(),
        &Mapping::default(),
        &import_opts(),
    )
    .unwrap();
    let got = String::from_utf8_lossy(&imp.files.iter().find(|x| x.0 == "amp.shcomp").unwrap().1)
        .into_owned();
    assert!(
        got.contains("  w {w}\n") && got.contains("  l {l*2}\n"),
        "{got}"
    );
}

/// A made-up kit whose MOS has three symbol pins and its bulk only in the stopping views:
/// spectre's termOrder names `B`, a pinless term `[@bsub:%:vss!]` (as SfyStopTerms dumps a
/// stopping view's terminal), auCdl's `progn(bn)`; its resistor's `sx` is in spectre's
/// termOrder only (gaps2 S8, P1; the shapes of analogLib nmos and of a commercial kit's
/// nfet and poly resistor, live ICADVM20.1 p_probe).
const IMPLICIT_DUMP: &str = r#"("sfyx" 1 "test")
("lib" "demopdk")
("cell" "nch3_demo" "ref")
("cdf" "")
("param" "model" "string" "demo_nch" "Model" "" ("storeDefault") () "")
("param" "w" "string" "300n" "Width" "lengthMetric" ("parseAsNumber" "parseAsCEL") () "demoCB('w)")
("param" "l" "string" "30n" "Length" "lengthMetric" ("parseAsNumber" "parseAsCEL") () "demoCB('l)")
("param" "bn" "string" "S" "Bulk node" "" ("parseAsCEL") () "")
("siminfo" "auCdl" (("instParameters" ("w")) ("modelName" "NDEMO") ("namePrefix" "M") ("netlistProcedure" "ansCdlCompPrim") ("termOrder" ("D" "G" "S" "progn(bn)"))))
("siminfo" "hspiceD" (("instParameters" ("w")) ("termOrder" ("D" "G" "S" "B"))))
("siminfo" "spectre" (("instParameters" ("w")) ("opParamExprList" ("(\"id\" \"OP(mappedRoot(\\\".m0\\\") \\\"id\\\")\")")) ("otherParameters" ("model")) ("termMapping" (("D" ":1") ("G" ":2"))) ("termOrder" ("D" "G" "S" "B"))))
("cv" "symbol" "schematicSymbol" 160)
("term" "B" "inputOutput" "[@bsub:%:vss!]")
("term" "D" "inputOutput" "")
("term" "G" "input" "")
("term" "S" "inputOutput" "")
("pinfig" 1 "D" 8 38 12 42)
("pinfig" 2 "G" -32 -2 -28 2)
("pinfig" 3 "S" 8 -42 12 -38)
("cell" "res3_demo" "ref")
("cdf" "")
("param" "r" "string" "2k" "R" "resistance" ("parseAsNumber" "parseAsCEL") () "demoCB('r)")
("param" "rsub" "netSet" "[@lsup:%:vss!]" "Bulk" "" ("storeDefault") () "")
("siminfo" "auCdl" (("instParameters" ("r")) ("modelName" "RDEMO") ("namePrefix" "R") ("netlistProcedure" "ansCdlCompPrim") ("termOrder" ("in" "out"))))
("siminfo" "spectre" (("componentName" "resistor") ("instParameters" ("r")) ("termOrder" ("in" "out" "sx"))))
("cv" "symbol" "schematicSymbol" 160)
("term" "in" "inputOutput" "")
("term" "out" "inputOutput" "")
("term" "sx" "inputOutput" "[@rsub:%:NOTSET!]")
("pinfig" 1 "in" -2 28 2 32)
("pinfig" 2 "out" -2 -32 2 -28)
("lib" "demo")
("cell" "amp" "design")
("cv" "schematic" "schematic" 160)
("inst" 1 "M0" "demopdk" "nch3_demo" "symbol" 0 0 "R0")
("inst" 2 "R0" "demopdk" "res3_demo" "symbol" 100 0 "R0")
("wire" 3 (10 40 10 60) 0)
("wire" 4 (-30 0 -50 0) 0)
("wire" 5 (10 -40 10 -60) 0)
("wire" 6 (100 30 100 50) 0)
("wire" 7 (100 -30 100 -50) 0)
("label" 8 3 "d" 10 60 "R0" "lowerLeft" "stick" 10 "normal" "wire" "label")
("label" 9 4 "g" -50 0 "R0" "lowerLeft" "stick" 10 "normal" "wire" "label")
("label" 10 5 "s" 10 -60 "R0" "lowerLeft" "stick" 10 "normal" "wire" "label")
("label" 11 6 "d" 100 50 "R0" "lowerLeft" "stick" 10 "normal" "wire" "label")
("label" 12 7 "s" 100 -50 "R0" "lowerLeft" "stick" 10 "normal" "wire" "label")
("inst" 13 "M1" "demopdk" "nch3_demo" "symbol" 200 0 "R0")
("wire" 14 (210 40 210 60) 0)
("wire" 15 (170 0 150 0) 0)
("wire" 16 (210 -40 210 -60) 0)
("label" 17 14 "d" 210 60 "R0" "lowerLeft" "stick" 10 "normal" "wire" "label")
("label" 18 15 "g" 150 0 "R0" "lowerLeft" "stick" 10 "normal" "wire" "label")
("label" 19 16 "x" 210 -60 "R0" "lowerLeft" "stick" 10 "normal" "wire" "label")
("prop" 13 "bn" "string" "D")
("end")
"#;

#[test]
fn stopping_view_terminals_become_inherited_pins_in_both_formats() {
    let imp = virtuoso::import(
        IMPLICIT_DUMP,
        &Files::new(),
        &Mapping::default(),
        &ImportOptions::default(),
    )
    .unwrap();
    let cell = |stem: &str| {
        let f = imp.files.iter().find(|f| f.0.ends_with(stem)).unwrap();
        String::from_utf8_lossy(&f.1).into_owned()
    };
    let nch = cell("nch3_demo.shprim");
    for want in [
        "pin bsub inout 0 0\n  inherit vss\n",
        r#"format spectre "{name} ({D} {G} {S} {bsub}) {model} w={w}""#,
        r#"format cdl "M{name} {D} {G} {S} {bn == \"B\" ? bsub : bn == \"D\" ? D : bn == \"G\" ? G : bn == \"S\" ? S : bn} NDEMO w={w}""#,
    ] {
        assert!(nch.contains(want), "{want}\n{nch}");
    }
    let res = cell("res3_demo.shprim");
    for want in [
        "pin rsub inout 0 0\n  inherit vss\n",
        r#"format spectre "{name} ({in} {out} {rsub}) resistor r={r}""#,
        r#"format cdl "R{name} {in} {out} r={r} $[RDEMO]""#,
    ] {
        assert!(res.contains(want), "{want}\n{res}");
    }
    let text = imp.notes.text();
    assert!(!text.contains("termOrder"), "{text}");
    assert!(
        text.contains(
            "needs_review:inherit\tdemopdk/res3_demo\trsub defaults to the inherited lsup"
        ),
        "{text}"
    );
    // What simInfo the formats leave, one row per cell; the kit's callbacks one row per
    // (library, function) (gaps2 S9, P14, P13).
    for want in [
        "needs_review:format\tdemopdk/nch3_demo\tsimInfo not carried: hspiceD (all); spectre opParamExprList termMapping\n",
        "needs_review:callback\tdemopdk\tdemoCB: callbacks of 3 params in 2 cells not carried (e.g. nch3_demo param w: demoCB('w))\n",
    ] {
        assert!(text.contains(want), "{want}\n{text}");
    }
    assert_eq!(text.matches("needs_review:callback").count(), 1, "{text}");
    assert!(!text.contains("res3_demo\tsimInfo"), "{text}");
    loads(&imp.files);
    let spectre = netlist_in(
        &imp.files,
        "demo/amp.shbench",
        schemify_elab::Dialect::Spectre,
    );
    assert!(
        spectre.contains("M0 (d g s vss) demo_nch w=3e-7"),
        "{spectre}"
    );
    assert!(
        spectre.contains("R0 (d s vss) resistor r=2000"),
        "{spectre}"
    );
    let cdl = netlist_in(&imp.files, "demo/amp.shbench", schemify_elab::Dialect::Cdl);
    // bn names a terminal: its net (S's by default, D's on M1), as auCdl does.
    assert!(cdl.contains("MM0 d g s s NDEMO w=3e-7"), "{cdl}");
    assert!(cdl.contains("MM1 d g x d NDEMO w=3e-7"), "{cdl}");
}

/// The device lines of a Spectre netlist (`\` continuations joined): (name, nodes, master).
fn spectre_devices(text: &str) -> Vec<(String, Vec<String>, String)> {
    let joined = text.replace("\\\n", " ");
    joined
        .lines()
        .filter_map(|l| {
            let (name, rest) = l.trim().split_once(" (")?;
            let (nodes, tail) = rest.split_once(')')?;
            let master = tail.split_whitespace().next()?.to_owned();
            Some((
                name.to_owned(),
                nodes.split_whitespace().map(str::to_owned).collect(),
                master,
            ))
        })
        .filter(|d| !d.0.contains(' ') && d.0 != "subckt")
        .collect()
}

/// Lane P's live comparison (live/checks/p_drive.sh): `$SFY_P_DIR/p_terms.sfyx` imported;
/// per cell, Schemify's spectre netlist against si's (every device with its master and as
/// many nodes) and its CDL against auCdl (spicecmp, auCdl's `$[model]` names left out).
/// One PASS/FAIL line each; the netlists and the report are written beside them.
#[test]
#[ignore]
fn live_p_terms() {
    let Some(dir) = std::env::var_os("SFY_P_DIR").map(PathBuf::from) else {
        return;
    };
    let dump = std::fs::read_to_string(dir.join("p_terms.sfyx")).unwrap();
    let imp = virtuoso::import(
        &dump,
        &Files::new(),
        &Mapping::default(),
        &ImportOptions::default(),
    )
    .unwrap();
    std::fs::write(dir.join("p_terms.report.txt"), imp.notes.text()).unwrap();
    for (p, b) in &imp.files {
        let path = dir.join("project").join(p);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b).unwrap();
    }
    let mut fails = 0;
    for cell in ["inv_tb", "bulk", "res", "dev"] {
        let Some(path) = imp.files.iter().map(|f| &f.0).find(|p| {
            p.ends_with(&format!("/{cell}.shbench")) || p.ends_with(&format!("/{cell}.shcomp"))
        }) else {
            println!("FAIL {cell}: not imported");
            fails += 1;
            continue;
        };
        let read = |sim: &str| std::fs::read_to_string(dir.join(format!("{cell}.{sim}.net")));
        let ours = netlist_in(&imp.files, path, schemify_elab::Dialect::Spectre);
        std::fs::write(dir.join(format!("{cell}.spectre.ours")), &ours).unwrap();
        if let Ok(want) = read("spectre") {
            let (a, b) = (spectre_devices(&ours), spectre_devices(&want));
            let same = a.len() == b.len()
                && b.iter().all(|w| {
                    a.iter()
                        .any(|o| o.0 == w.0 && o.2 == w.2 && o.1.len() == w.1.len())
                });
            if !same {
                fails += 1;
            }
            println!(
                "{} netlist.spectre.{cell}: ours {a:?} si {b:?}",
                if same { "PASS" } else { "FAIL" }
            );
        } else {
            println!("SKIP netlist.spectre.{cell}: no si netlist");
        }
        let ours = netlist_in(&imp.files, path, schemify_elab::Dialect::Cdl);
        std::fs::write(dir.join(format!("{cell}.auCdl.ours")), &ours).unwrap();
        match read("auCdl") {
            Ok(want) => {
                // auCdl's `$[model]` names out, and the `+` lines continuing its `*.GLOBAL`
                // and `*.PIN` comments (spicecmp would join them to `.PARAM`).
                let mut comment = false;
                let strip = |t: &str| -> String {
                    t.lines()
                        .map(|l| match l.find(" $[") {
                            Some(k) => {
                                let end = l[k..].find(']').map_or(l.len(), |e| k + e + 1);
                                format!("{}{}\n", &l[..k], &l[end..])
                            }
                            None => format!("{l}\n"),
                        })
                        .collect()
                };
                let want: String = want
                    .lines()
                    .filter(|l| {
                        let cont = l.starts_with('+');
                        if !cont {
                            comment = l.starts_with('*');
                        }
                        !(cont && comment)
                    })
                    .map(|l| format!("{l}\n"))
                    .collect();
                // Globals by Schemify's spelling (§3.3: `gnd!` is GND, an inherited default
                // `x!` is the global `x`).
                let mut bangs: Vec<(String, String)> = vec![("gnd!".into(), "0".into())];
                for w in want.split(|ch: char| ch.is_whitespace()) {
                    if let Some(x) = w.strip_suffix('!')
                        && w != "gnd!"
                        && !bangs.iter().any(|b| b.0 == w)
                    {
                        bangs.push((w.to_owned(), x.to_owned()));
                    }
                }
                let c = schemify_spicecmp::compare(
                    &strip(&ours),
                    &strip(&want),
                    &schemify_spicecmp::Options {
                        path_aliases: bangs,
                        ..schemify_spicecmp::Options::default()
                    },
                );
                println!(
                    "{} netlist.cdl.{cell}: {:?}",
                    if c.is_equal() { "PASS" } else { "WARN" },
                    c.diffs
                );
            }
            Err(_) => println!("SKIP netlist.cdl.{cell}: no auCdl netlist"),
        }
    }
    assert_eq!(fails, 0, "see the FAIL lines");
}

/// gaps2 P12: a 4-pin cell mapped onto a kit device whose bulk is an inherited pin of its
/// mirror (`pin b bsub`): the drawn pins are placed by the mirror's geometry, the bulk is
/// a `conn` request the loader turns into the instance's netSet override.
#[test]
fn a_pin_mapped_onto_an_inherited_terminal_is_a_loader_connection() {
    let mut f = Files::new();
    f.insert(
        "schemify.project".into(),
        b"schemify 1 project\n\nlib dev path dev\nlib demopdk path demopdk\n".to_vec(),
    );
    f.insert(
        "dev/n4.shprim".into(),
        b"schemify 1 shprim\n\npin d inout 10 40\npin g in -30 0\npin s inout 10 -40\npin b inout 30 0\n\nparam W 1\n\nimpl spice spice\n\nline 0 -30 0 30\n".to_vec(),
    );
    f.insert(
        "demopdk/nch3_demo.shprim".into(),
        b"schemify 1 shprim\n  provenance \"virtuoso demopdk/nch3_demo\"\n\npin D inout 10 40\npin G in -30 0\npin S inout 10 -40\npin bsub inout 0 0\n  inherit vss\n\nparam w 1\n\nimpl spice spice\n\nline 0 -30 0 30\n".to_vec(),
    );
    f.insert(
        "tb.shbench".into(),
        b"schemify 1 shbench\n\ninst aaaaaa dev:n4\n  at 0 0 R0\n  name M1\n\nwire bbbbbb 30 0 50 0\nlabel cccccc 50 0 R0 VBB\n".to_vec(),
    );
    let d = design(&f);
    let tops = [d.find("tb.shbench").unwrap()];
    let m = Mapping::parse(
        "map dev:n4 virtuoso demopdk nch3_demo\n  pin d D\n  pin g G\n  pin s S\n  pin b bsub\n  param W w\n",
    )
    .unwrap();
    let x = virtuoso::export(&d, &tops, &m, &ExportOptions::default()).unwrap();
    // The three drawn pins land where n4's were (no request); the bulk is one.
    assert_eq!(x.sfyx.matches("(\"conn\" ").count(), 1, "{}", x.sfyx);
    assert!(x.sfyx.contains("\"bsub\" \"VBB\")"), "{}", x.sfyx);
}

/// A drawn inherited terminal whose net expression names another property (`B`,
/// `[@bsub:%:vss!]`): the pin keeps the terminal's name and records the property
/// (`vinherit`), so an instance's netSet `bsub` imports as `inherit B` and exports back as
/// `bsub`, not `B` (gaps2 P1 follow-up).
#[test]
fn an_inherited_terminal_named_apart_from_its_property_keeps_the_property() {
    let dump = r#"("sfyx" 1 "test")
("lib" "demopdk")
("cell" "n4b" "ref")
("cdf" "")
("param" "w" "string" "1u" "W" "" ("parseAsNumber" "parseAsCEL") () "")
("siminfo" "spectre" (("componentName" "nch") ("instParameters" ("w")) ("termOrder" ("D" "G" "S" "B"))))
("cv" "symbol" "schematicSymbol" 160)
("term" "B" "inputOutput" "[@bsub:%:vss!]")
("term" "D" "inputOutput" "")
("term" "G" "input" "")
("term" "S" "inputOutput" "")
("pinfig" 1 "B" 28 -2 32 2)
("pinfig" 2 "D" 8 38 12 42)
("pinfig" 3 "G" -32 -2 -28 2)
("pinfig" 4 "S" 8 -42 12 -38)
("lib" "demo")
("cell" "t" "design")
("cv" "schematic" "schematic" 160)
("inst" 1 "M0" "demopdk" "n4b" "symbol" 0 0 "R0")
("prop" 1 "bsub" "netSet" "VDDA")
("end")
"#;
    let imp = virtuoso::import(dump, &Files::new(), &Mapping::default(), &import_opts()).unwrap();
    let file = |stem: &str| {
        let f = imp.files.iter().find(|f| f.0.ends_with(stem)).unwrap();
        String::from_utf8_lossy(&f.1).into_owned()
    };
    let m = file("n4b.shprim");
    assert!(
        m.contains("pin B inout 30 0\n  inherit vss\n  vinherit bsub\n"),
        "{m}"
    );
    let t = file("t.shbench");
    assert!(t.contains("  inherit B VDDA\n"), "{t}");
    assert!(
        !imp.notes.text().contains("netSet bsub"),
        "{}",
        imp.notes.text()
    );
    let files: Files = imp.files.iter().cloned().collect();
    let d = design(&files);
    let tops = [d.find("demo/t.shbench").unwrap()];
    let x = virtuoso::export(&d, &tops, &Mapping::default(), &ExportOptions::default()).unwrap();
    assert!(
        x.sfyx.contains("\"bsub\" \"netSet\" \"VDDA\")"),
        "{}",
        x.sfyx
    );
}

/// Lane P's live refresh comparison (live/checks/p_drive.sh): in `$SFY_P_DIR/project`
/// refreshed by `virtuoso-refresh --apply`, every `cached` value of the resized kit nfet MN
/// (inv) and resistor R0 (res) against what the callbacks derive for the same sizes
/// (`p_truth.txt`, `<inst> <param> <value>` lines). One PASS/FAIL line each.
#[test]
#[ignore]
fn live_p_refresh() {
    let Some(dir) = std::env::var_os("SFY_P_DIR").map(PathBuf::from) else {
        return;
    };
    let truth = std::fs::read_to_string(dir.join("p_truth.txt")).unwrap();
    let num = |s: &str| schemify_expr::parse_num(s.trim()).ok();
    let mut fails = 0;
    for (file, inst) in [
        ("sfyprobe_p/inv.shcomp", "MN"),
        ("sfyprobe_p/res.shbench", "R0"),
    ] {
        let text = std::fs::read_to_string(dir.join("project").join(file)).unwrap();
        let block: Vec<&str> = text
            .split("\n\n")
            .find(|b| b.lines().any(|l| l == format!("  name {inst}")))
            .unwrap_or_default()
            .lines()
            .collect();
        let cached: Vec<(&str, &str)> = block
            .iter()
            .filter_map(|l| {
                let mut w = l.split_whitespace();
                (w.next() == Some("cached"))
                    .then(|| (w.next().unwrap_or(""), w.next().unwrap_or("")))
            })
            .collect();
        let mut bad = Vec::new();
        for (p, v) in &cached {
            let want = truth.lines().find_map(|l| {
                let mut w = l.splitn(3, ' ');
                (w.next() == Some(inst) && w.next() == Some(p)).then(|| w.next().unwrap_or(""))
            });
            // A string value is written quoted and escaped (`"\"\""`), the truth bare.
            let v = &v.trim_matches(['"', '\\']);
            let same = match (want, num(v), want.and_then(num)) {
                (_, Some(a), Some(b)) => (a - b).abs() <= 1e-6 * a.abs().max(b.abs()),
                (Some(w), _, _) => w == *v,
                _ => false,
            };
            if !same {
                bad.push(format!("{p}: ours {v} callbacks {want:?}"));
            }
        }
        let ok = !cached.is_empty() && bad.is_empty();
        if !ok {
            fails += 1;
        }
        println!(
            "{} refresh.{inst}: {} cached value(s) {}",
            if ok { "PASS" } else { "FAIL" },
            cached.len(),
            if bad.is_empty() {
                "equal the callbacks'".to_owned()
            } else {
                bad.join("; ")
            }
        );
    }
    assert_eq!(fails, 0, "see the FAIL lines");
}
