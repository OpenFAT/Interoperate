//! Derived params (spec `interop.md` §8.3) through the SKILL bridge on the mock Virtuoso,
//! against a made-up PDK whose parameter names mean nothing (`tests/fixtures/skill/
//! charpdk.il`): inputs and derived params are found by experiment, never by name; their
//! values are Virtuoso's, cached per instance, stale on an edit, refreshed by measuring.

mod common;
mod skill;

use interop::design::Files;
use interop::map::Mapping;
use interop::virtuoso::characterize::{self, Device, Kind};
use interop::virtuoso::{self, ExportOptions, ImportOptions};

const BUILD: &str = include_str!("fixtures/skill/build.il");
const CHARPDK: &str = include_str!("fixtures/skill/charpdk.il");

/// Characterizes `cells` of the made-up PDK with the extra SfyCharacterize keys `keys`.
fn characterize(cells: &[&str], keys: &str) -> Vec<Device> {
    let mut i = skill::virtuoso(&["zpdk", "scratch"]);
    i.load(BUILD).unwrap();
    i.load(CHARPDK).unwrap();
    i.load("(SfyFixCharPdk)").unwrap();
    let list: Vec<String> = cells.iter().map(|c| format!("\"{c}\"")).collect();
    let ok = i
        .load(&format!(
            "(SfyCharacterize \"/t/char.sfyx\" \"zpdk\" (list {}) \"scratch\" {keys})",
            list.join(" ")
        ))
        .unwrap_or_else(|e| panic!("{e}\n{}", i.log));
    let log = i.files.get("/t/char.sfyx.log").cloned().unwrap_or_default();
    assert!(ok.truthy(), "{log}");
    characterize::read(&i.files["/t/char.sfyx"]).unwrap()
}

fn names(v: &[String]) -> Vec<&str> {
    v.iter().map(String::as_str).collect()
}

#[test]
fn inputs_and_derived_params_are_found_by_experiment() {
    let devs = characterize(&["zeta", "tab", "chaos"], "");
    let zeta = &devs[0];
    assert_eq!(names(&zeta.inputs), ["zq", "kappa", "ct"]);
    assert_eq!(zeta.kinds, [Kind::Num, Kind::Num, Kind::Count]);
    assert_eq!(names(&zeta.defaults), ["5e-08", "1e-08", "1"]);
    // Negative space: a boolean and a number whose callbacks change nothing were probed
    // and are not inputs; a param without a callback that is not netlisted is no candidate.
    let probed: Vec<&str> = zeta.probes.iter().map(|p| p.name.as_str()).collect();
    assert!(
        probed.contains(&"flip") && probed.contains(&"knob"),
        "{probed:?}"
    );
    assert!(!probed.contains(&"memo"));
    for p in &zeta.probes {
        let input = zeta.inputs.contains(&p.name);
        assert_eq!(!p.changed.is_empty() && p.status.is_ok(), input, "{p:?}");
    }
    assert_eq!(
        zeta.probes.iter().find(|p| p.name == "flip").unwrap().kind,
        Kind::Bool
    );
    assert_eq!(zeta.derived(), ["rho", "sig"]);
    let tab = &devs[1];
    assert_eq!(names(&tab.inputs), ["mu"]);
    assert_eq!(tab.derived(), ["nu"]);
    let chaos = &devs[2];
    assert_eq!(names(&chaos.inputs), ["pp", "qq"]);
    assert_eq!(chaos.kinds, [Kind::Num, Kind::Count]);
    assert_eq!(chaos.derived(), ["zz"]);
    // A count every nudge of which the callbacks put back at the default width is probed
    // again at a wider one (gaps2 P2: GF22FDX nfet's nf at w 80n).
    let fing = &characterize(&["fing"], "")[0];
    assert_eq!(names(&fing.inputs), ["w", "nf"]);
    assert_eq!(fing.derived(), ["ad"]);
    // ?inputs overrides discovery.
    let named = characterize(&["zeta"], "?inputs (list \"kappa\")");
    assert_eq!(names(&named[0].inputs), ["kappa"]);
    // Discovery alone measures nothing else.
    assert!(devs.iter().all(|d| d.points.is_empty()));
}

/// The blocking findings and I005s of `cell` in project `files` (`code param` each).
fn findings(files: &Files, cell: &str) -> Vec<String> {
    use schemify_doc::{Project, core_library};
    use schemify_elab::{CellState, no_funcs};
    let manifest = files.get("schemify.project").map(Vec::as_slice);
    let mut p = Project::new(manifest, None, 7).unwrap();
    let dirs = interop::design::manifest_libs(manifest.unwrap()).unwrap();
    let mut libs: std::collections::BTreeMap<String, Vec<(&str, &[u8])>> = Default::default();
    libs.insert("core".into(), core_library().to_vec());
    for (path, bytes) in files.iter().filter(|f| common::is_cell(f.0)) {
        let (dir, rel) = path.rsplit_once('/').unwrap_or(("", path));
        let lib = dirs
            .iter()
            .find(|(_, d)| d.as_str() == dir)
            .map_or(String::new(), |(l, _)| l.clone());
        libs.entry(lib).or_default().push((rel, bytes));
    }
    for (lib, listing) in &libs {
        p.index_library(lib, "t", listing).unwrap();
    }
    let h = p.open("", cell, &files[cell], None).unwrap().handle;
    let st = CellState::build(&p, h, &mut no_funcs).unwrap();
    st.violations
        .rows
        .iter()
        .filter(|v| v.code.as_str().starts_with('E') || v.code.as_str() == "I005")
        .map(|v| format!("{} {}", v.code.as_str(), st.names.get(v.param)))
        .collect()
}

/// The `cached zz` line of the design's instance: (value, digest).
fn cached_zz(files: &Files) -> Option<(String, String)> {
    let d = schemify_format::parse(&files["top.shcomp"]).unwrap();
    let r =
        (1..d.record_count()).find(|&r| d.record_kind(r) == schemify_format::RecordKind::Inst)?;
    let a = d
        .attrs(r)
        .find(|&a| d.attr_key(a) == "cached" && d.attr_value(a, 0) == Some("zz"))?;
    Some((
        d.attr_value(a, 1)?.to_owned(),
        d.attr_value(a, 2)?.to_owned(),
    ))
}

/// P2 end to end (D20 as amended 2026-10-15): a derived value (`zz`, a hash of `pp` and
/// `qq`) comes into Schemify from Virtuoso cached with the digest of its inputs, fresh; an edit of an input in Schemify makes it stale (E039) and the export
/// leaves it out; Virtuoso's callbacks (`SfyImport ?callbacks t`) derive it again and the
/// next import brings it back fresh, with the callbacks' value.
#[test]
fn a_derived_value_goes_stale_in_schemify_and_comes_back_fresh() {
    let mut i = skill::virtuoso(&["zpdk", "scratch", "work", "analogLib", "basic"]);
    i.load(BUILD).unwrap();
    i.load("(SfyFixStdlib)").unwrap();
    i.load(CHARPDK).unwrap();
    i.load("(SfyFixCharPdk)").unwrap();
    // The PDK mirror, its derived params marked.
    assert!(
        i.load("(SfyCharacterize \"/t/char.sfyx\" \"zpdk\" (list \"chaos\") \"scratch\")")
            .unwrap()
            .truthy()
    );
    let dev = characterize::read(&i.files["/t/char.sfyx"])
        .unwrap()
        .remove(0);
    assert_eq!(dev.derived(), ["zz"]);
    assert!(
        i.load("(SfyExport \"/t/pdk.sfyx\" \"zpdk\" nil)")
            .unwrap()
            .truthy()
    );
    let pdk = virtuoso::import(
        &i.files["/t/pdk.sfyx"],
        &Files::new(),
        &Mapping::default(),
        &ImportOptions::default(),
    )
    .unwrap();
    let mut files: Files = pdk.files.into_iter().collect();
    let mirror = files
        .keys()
        .find(|k| k.ends_with("chaos.shprim"))
        .unwrap()
        .clone();
    let applied = characterize::mark_derived(&files[&mirror], &dev).unwrap();
    assert!(
        applied.contains("  vinputs pp qq\n") && applied.contains("  vderived true"),
        "{applied}"
    );
    files.insert(mirror, applied.into_bytes());
    // A design instantiating it, sent to Virtuoso with its callbacks.
    let design = |pp: &str| {
        format!(
            "schemify 1 shcomp\n\npin A inout 0 0\n\nimpl schematic schematic\n\ninst 0x0001 zpdk:chaos\n  at 0 0 R0\n  name X1\n  pp {pp}\n\nport 0p0001 0 0 R0 A\n"
        )
    };
    files.insert("top.shcomp".into(), design("2.5").into_bytes());
    let mut io = ImportOptions::default();
    io.lib_map.insert("work".into(), String::new());
    let mut eo = ExportOptions {
        producer: "t".into(),
        ..ExportOptions::default()
    };
    eo.lib_map.insert(String::new(), "work".into());
    let trip = |i: &mut skill::Interp<skill::mockdb::MockDb>,
                files: &mut Files,
                overwrite: bool|
     -> (String, String) {
        let d = common::load_tops(files, &["top.shcomp"]);
        let out = virtuoso::export(&d, &[0], &Mapping::default(), &eo)
            .unwrap()
            .sfyx;
        i.files.insert("/t/d.sfyx".into(), out.clone());
        let keys = if overwrite { " ?overwrite t" } else { "" };
        let ok = i
            .load(&format!("(SfyImport \"/t/d.sfyx\" ?callbacks t{keys})"))
            .unwrap();
        assert!(ok.truthy(), "{}", i.files["/t/d.sfyx.log"]);
        assert!(
            i.load("(SfyExport \"/t/back.sfyx\" \"work\" (list \"top\"))")
                .unwrap()
                .truthy()
        );
        let back =
            virtuoso::import(&i.files["/t/back.sfyx"], files, &Mapping::default(), &io).unwrap();
        for (p, b) in back.files {
            files.insert(p, b);
        }
        (out, i.files["/t/back.sfyx"].clone())
    };
    // zz is a string param (no parseAsNumber): its canonical value is quoted.
    let zz = |pp: i64, qq: i64| format!("\"{}\"", (7 * pp + 13 * qq) % 11);
    let (_, back) = trip(&mut i, &mut files, false);
    let top = String::from_utf8_lossy(&files["top.shcomp"]).into_owned();
    let (v, d1) = cached_zz(&files).unwrap_or_else(|| panic!("{top}\n{back}"));
    assert_eq!(v, zz(2500, 1));
    assert!(
        findings(&files, "top.shcomp").is_empty(),
        "{:?}",
        findings(&files, "top.shcomp")
    );
    // The edit: pp 2.5 -> 3.5. The cached value is stale.
    let edited = String::from_utf8(files["top.shcomp"].clone())
        .unwrap()
        .replace("  pp 2.5\n", "  pp 3.5\n");
    files.insert("top.shcomp".into(), edited.into_bytes());
    assert_eq!(findings(&files, "top.shcomp"), ["E039 zz"]);
    // The refresh: Virtuoso measures the stale instance's inputs on a scratch schematic,
    // and the measured value comes back as a fresh cached line, without a round trip.
    let mut refreshed = files.clone();
    let (wants, notes) = characterize::refresh_wants(&refreshed);
    assert!(notes.is_empty(), "{notes:?}");
    assert_eq!(wants.len(), 1);
    assert_eq!(
        (wants[0].lib.as_str(), wants[0].cell.as_str()),
        ("zpdk", "chaos")
    );
    assert_eq!(wants[0].inputs, ["pp", "qq"]);
    // pp is overridden; qq stays at Virtuoso's own default.
    assert_eq!(wants[0].points, [[Some("3.5".to_owned()), None]]);
    let ask = characterize::refresh_skill(&wants, "/t", "scratch");
    assert!(i.load(&ask).unwrap().truthy(), "{ask}");
    let measured = characterize::read(&i.files["/t/refresh.zpdk.chaos.sfyx"]).unwrap();
    assert!(measured[0].probes.is_empty(), "named inputs are not probed");
    let done = characterize::refresh_apply(&refreshed, &measured);
    assert!(
        done.stale.is_empty() && done.rewritten.is_empty(),
        "{done:?}"
    );
    assert_eq!(done.files.len(), 1);
    refreshed.extend(done.files);
    let (rv, rd) = cached_zz(&refreshed).expect("a refreshed cached zz");
    assert_eq!(rv, zz(3500, 1));
    assert!(findings(&refreshed, "top.shcomp").is_empty());
    assert!(
        characterize::refresh_wants(&refreshed).0.is_empty(),
        "nothing left to refresh"
    );
    // The export leaves the stale value to the callbacks; they derive it again.
    let (sent, _) = trip(&mut i, &mut files, true);
    assert!(!sent.contains("\"zz\""), "a stale value was sent:\n{sent}");
    let (v, d2) = cached_zz(&files).expect("a cached zz line");
    assert_eq!(v, zz(3500, 1));
    assert_ne!(d1, d2);
    // The round trip and the refresh agree, digest included.
    assert_eq!((rv, rd), (v, d2.clone()));
    assert!(
        findings(&files, "top.shcomp").is_empty(),
        "{:?}",
        findings(&files, "top.shcomp")
    );
}

/// The trap a live PDK set: a param editable with a callback (zq_area sets zq back from
/// the area) that the declared sizing param zq moves is derived, not an input; inputs are
/// read from the probes as an influence graph seeded by paramLabelSet, and a cell without
/// one falls back to the graph's sources (reported as info).
#[test]
fn a_param_a_sizing_input_moves_is_derived_even_when_editable() {
    let devs = characterize(&["ar", "tab"], "");
    let ar = &devs[0];
    assert_eq!(names(&ar.inputs), ["zq", "nn"]);
    let probe = ar.probes.iter().find(|p| p.name == "zq_area").unwrap();
    assert!(
        !probe.changed.is_empty(),
        "zq_area's own nudge moves zq: {probe:?}"
    );
    assert_eq!(ar.derived(), ["zq_area", "qa"]);
    assert_eq!(names(&devs[1].inputs), ["mu"]);
    let mut i = skill::virtuoso(&["zpdk", "scratch"]);
    i.load(BUILD).unwrap();
    i.load(CHARPDK).unwrap();
    i.load("(SfyFixCharPdk)").unwrap();
    i.load(r#"(SfyCharacterize "/t/c.sfyx" "zpdk" (list "tab" "zeta") "scratch")"#)
        .unwrap();
    let log = &i.files["/t/c.sfyx.log"];
    assert!(
        log.contains("info zpdk/tab: no paramLabelSet param moves another"),
        "{log}"
    );
    assert!(!log.contains("info zpdk/zeta"), "{log}");
}
