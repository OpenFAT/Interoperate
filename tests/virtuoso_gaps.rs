//! Lane S of the second gap round (notes/interop-research/gaps2/inv_schematic.md): a live
//! dump graded on the Schemify side. `fixtures/virtuoso/gaps.sfyx` is what
//! `live/checks/s_schematic.il` dumped on ICADVM20.1 (`s_drive.sh`), `gaps/si/` the si
//! netlists of the same cells: the import must connect every pin as Virtuoso does (its own
//! `tnet` table and si's netlists), write no empty instance parameter, and keep the view an
//! instance is placed through.

mod common;

use std::collections::BTreeMap;

use interop::map::Mapping;
use interop::virtuoso::{self, ExportOptions, ImportOptions};

type Files = BTreeMap<String, Vec<u8>>;

fn fixture(rel: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/tests/fixtures/virtuoso/{rel}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap_or_else(|e| panic!("{rel}: {e}"))
}

fn import() -> (Files, String) {
    let imp = virtuoso::import(
        &fixture("gaps.sfyx"),
        &Files::new(),
        &Mapping::default(),
        &ImportOptions::default(),
    )
    .expect("the live dump imports");
    (imp.files.into_iter().collect(), imp.notes.text())
}

/// The `dialect` netlist's input of `cell` (a project path) against its snapshot
/// (`common::frozen_input`), taken when the netlist agreed with si's (`gaps/si/`).
fn netlist(files: &Files, cell: &str, dialect: &str) {
    common::frozen_input(files, cell, dialect);
}

#[test]
fn the_live_dump_connects_as_virtuoso_does() {
    let (files, report) = import();
    let problems = interop::check_output(&files.clone().into_iter().collect::<Vec<_>>());
    assert!(problems.is_empty(), "{problems:#?}");
    // Virtuoso's own instTerm table (tnet) agrees with the elaborated nets: no row.
    assert!(!report.contains("needs_review:connectivity"), "{report}");
    // A wire ending on a pin's edge, inside it, on its corner, or through it off centre
    // and two overlapping pins are joined by stubs (S1).
    assert!(
        report.contains("joins on a pin figure off its centre"),
        "{report}"
    );
    // Their Spectre netlists connected as si's (`gaps/si/<cell>.spectre.net`), the supply
    // symbol a global label (S11), not an instance.
    for cell in ["conn", "alt"] {
        netlist(&files, &format!("sfyprobe_s/{cell}.shbench"), "Spectre");
    }
}

#[test]
fn the_joining_stubs_and_labels_survive_a_round_trip_unchanged() {
    // Export leaves the import's own `vjoin` records out; importing that export onto the
    // project gives the same files back (no stub or label piles up).
    let (files, _) = import();
    let d = common::load_all(&files);
    let tops: Vec<usize> = ["sfyprobe_s/conn.shbench", "sfyprobe_s/alt.shbench"]
        .iter()
        .map(|p| d.find(p).expect("cell"))
        .collect();
    let out = virtuoso::export(&d, &tops, &Mapping::default(), &ExportOptions::default())
        .expect("exports");
    // Only the stash (Schemify's own records, §7.3) names them; Virtuoso gets no such object.
    assert!(
        out.sfyx
            .lines()
            .filter(|l| l.contains("vjoin"))
            .all(|l| l.starts_with("(\"prop\" 0 \"schemifyRecords")),
        "{}",
        out.sfyx
    );
    let back = virtuoso::import(
        &out.sfyx,
        &files,
        &Mapping::default(),
        &ImportOptions::default(),
    )
    .expect("the export imports");
    for (p, b) in back.files.iter().filter(|f| f.0.ends_with(".shbench")) {
        assert_eq!(
            common::strip_fp(&String::from_utf8_lossy(b)),
            common::strip_fp(&String::from_utf8_lossy(&files[p])),
            "{p}"
        );
    }
}

#[test]
fn an_instance_through_another_view_keeps_its_pins_and_view() {
    let (files, report) = import();
    let alt = String::from_utf8(files["sfyprobe_s/alt.shbench"].clone()).unwrap();
    assert!(alt.contains("vview symbol_alt"), "{alt}");
    assert!(
        report.contains("S1: sfyprobe_s/sub/symbol_alt placed as"),
        "{report}"
    );
    // Export draws the instance through that view again, without the joining labels.
    let d = common::load_all(&files);
    let top = d.find("sfyprobe_s/alt.shbench").expect("alt");
    let out = virtuoso::export(&d, &[top], &Mapping::default(), &ExportOptions::default())
        .expect("alt exports");
    let s1 = out
        .sfyx
        .lines()
        .find(|l| l.starts_with("(\"inst\"") && l.contains("\"S1\""))
        .expect("S1 exported");
    assert!(s1.contains("\"symbol_alt\""), "{s1}");
    // The labels joining symbol_alt's pins to sub's are import's own: not drawn again.
    let labels = out
        .sfyx
        .lines()
        .skip_while(|l| !l.starts_with("(\"cell\" \"alt\""))
        .take_while(|l| !l.starts_with("(\"cell\" \"sub\""))
        .filter(|l| l.starts_with("(\"label\""))
        .count();
    assert_eq!(labels, 6, "{}", out.sfyx);
}

#[test]
fn empty_cdf_values_write_no_parameter() {
    let (files, _) = import();
    let cell = "sfyprobe_s/e1.shbench";
    // si writes `M0 (nD nG nS 0) nmos` and `R0 (nD nS) resistor r=1K`, auCdl `RR0 nD nS 1K
    // $[RP]`: no `k=` for an empty CDF value (`m`, `region`, `isnoisy`). The netlists had
    // none, and ngspice ran the resistor line (`RR0 nD nS 1000 m=` before, which it refuses).
    for d in ["Spectre", "Cdl", "Ngspice", "Ngspice"] {
        netlist(&files, cell, d);
    }
}

#[test]
fn cds_thru_is_a_short_in_every_dialect() {
    // si: `T0 (x y) iprobe` (S12).
    let (files, _) = import();
    let cell = "sfyprobe_s/pc.shbench";
    // Spectre `T0 (x y) iprobe`, ngspice `VT0 x y 0`.
    netlist(&files, cell, "Spectre");
    netlist(&files, cell, "Ngspice");
}

#[test]
fn port_order_without_term_order_is_virtuosos() {
    // si writes `subckt ord a m z` and `X0 (na nm nz) ord`: terminals sorted by name, not
    // in the symbol's creation order z a m (G8).
    let (files, _) = import();
    // Ours had `subckt ord a m z` and `(na nm nz)` too.
    let want = fixture("gaps/si/ordtop.spectre.net");
    assert!(want.contains("subckt ord a m z"), "{want}");
    netlist(&files, "sfyprobe_s/ordtop.shbench", "Spectre");
}

/// The live dump with `edits` applied (`(after, inserted)` line pairs), imported.
fn import_edited(edits: &[(&str, &str)], existing: &Files, analoglib: bool) -> String {
    let mut text = fixture("gaps.sfyx");
    for (after, ins) in edits {
        let k = text.find(after).unwrap_or_else(|| panic!("{after}")) + after.len();
        text.insert_str(k, &format!("\n{ins}"));
    }
    let opts = ImportOptions {
        analoglib,
        ..ImportOptions::default()
    };
    virtuoso::import(&text, existing, &Mapping::default(), &opts)
        .expect("imports")
        .notes
        .text()
}

#[test]
fn what_the_import_cannot_carry_is_a_report_row() {
    // S17: a CDF button and an int whose default is an expression; S10: a callback that
    // runs libInit.il code; S15: nets differing only in case.
    let r = import_edited(
        &[
            (
                r#"("param" "model" "string" "" "Model name" "" ("parseAsCEL") () "")"#,
                r#"("param" "btn" "button" "" "Press" "" () () "")
("param" "nf2" "int" "iPar(\"m\")*2" "nf2" "" () () "")"#,
            ),
            (
                r#"("cell" "sub" "design")"#,
                r#"("cdf" "")
("param" "k" "string" "1" "k" "" () () "G_cb()" "libInit.il" "(procedure (G_cb) t)")"#,
            ),
            (
                r#"("label" 12 7 "nD" 360000 0 "R0" "lowerLeft" "stick" 10000 "normal" "wire" "label")"#,
                r#"("label" 13 0 "ND" 400000 0 "R0" "lowerLeft" "stick" 10000 "normal" "wire" "label")"#,
            ),
        ],
        &Files::new(),
        false,
    );
    for want in [
        "dropped:display-only\tanalogLib/nmos\tparam btn: a CDF button",
        "needs_review:expression\tanalogLib/nmos\tparam nf2: default `iPar(\"m\")*2` imported as 0",
        "needs_review:callback\tsfyprobe_s/sub\tparam k: its callback runs libInit.il code",
        "needs_review:name\tsfyprobe_s/e1\tnets ND nD differ only in case",
        // S6: what the dumper skipped, from its own records.
        "dropped:display-only\tsfyprobe_s/skip\tcvprop teamNote ×1 (schematic view)",
        "dropped:display-only\tsfyprobe_s/skip\tdonut ×1 (symbol view)",
        // S5: a multi-sheet index.
        "needs_review:name\tsfyprobe_s/ms\tSH1: sheet sfyprobe_s/ms@sheet001",
        // A sheet border has no pins: not imported.
        "dropped:display-only\tUS_8ths/Asize\ta symbol without pins",
    ] {
        assert!(r.contains(want), "{want}\n{r}");
    }
    // S13: an analogLib property the core cell has no param for.
    let r = import_edited(
        &[(
            r#"("label" 12 7 "nD" 360000 0 "R0" "lowerLeft" "stick" 10000 "normal" "wire" "label")"#,
            r#"("prop" 2 "tc1" "string" "1m")"#,
        )],
        &Files::new(),
        true,
    );
    assert!(
        r.contains("dropped:unmapped\tsfyprobe_s/e1\tR0: tc1 = 1m has no param"),
        "{r}"
    );
    // S2: a cell of another library the project holds as a component already.
    let mut existing = Files::new();
    existing.insert(
        "schemify.project".into(),
        b"schemify 1 project\n\nlib analogLib path analogLib\n".to_vec(),
    );
    existing.insert(
        "analogLib/res.shcomp".into(),
        b"schemify 1 shcomp\n\npin PLUS inout 0 0\npin MINUS inout 0 -60\n\nimpl schematic schematic\n"
            .to_vec(),
    );
    let r = import_edited(&[], &existing, false);
    assert!(
        r.contains("remapped\tanalogLib/res\tinstances use analogLib/res.shcomp"),
        "{r}"
    );
}

/// `fixtures/virtuoso/gaps_more.sfyx` (live `s_more`) imported into an empty project.
fn import_more() -> (Files, String) {
    let imp = virtuoso::import(
        &fixture("gaps_more.sfyx"),
        &Files::new(),
        &Mapping::default(),
        &ImportOptions::default(),
    )
    .expect("the live dump imports");
    (imp.files.into_iter().collect(), imp.notes.text())
}

#[test]
fn views_libraries_pcells_constraints_and_unsaved_edits_are_report_rows() {
    // S4 S10 S14 S19 S20, each a named row of the live dump.
    let (files, r) = import_more();
    let problems = interop::check_output(&files.into_iter().collect::<Vec<_>>());
    assert!(problems.is_empty(), "{problems:#?}");
    for want in [
        "needs_review:format\tsfyprobe_s/old\tview cmos_sch (sch.oa) is not in the dump",
        "needs_review:format\tsfyprobe_s/cg\tview constraint (Constraint Manager constraints)",
        "needs_review:format\tsfyprobe_s/beh\ttext view spectre (spectre.scs) not imported: a new library gets the symbol",
        "needs_review:format\tsfyprobe_s/beh\ttext view verilogams (verilog.vams) not imported",
        "needs_review:callback\tsfyprobe_s\tthe library CDF (1 params) is not carried",
        "needs_review:callback\tsfyprobe_s\tlibInit.il is not carried",
        "dropped:display-only\tsfyprobe_s/pcs\tpcell code (dumped: its default variant)",
        "dropped:display-only\tsfyprobe_s/pctop\tpcell variant pins of P0",
        "dropped:display-only\tsfyprobe_s/dirty\tunsaved edits (dumped as open)",
    ] {
        assert!(r.contains(want), "{want}\n{r}");
    }
}

#[test]
fn cdf_attributes_travel_back_on_an_unchanged_cell_and_are_reported_on_an_edited_one() {
    // S7: doneProc, formInitProc, paramEvaluate, the display modes and w's display,
    // editable and use (live s_more blk) ride in vcdfrec.
    let (files, _) = import_more();
    let blk = String::from_utf8(files["sfyprobe_s/blk.shcomp"].clone()).unwrap();
    assert!(
        blk.contains("cdfattr") && blk.contains("paramattr"),
        "{blk}"
    );
    let export = |files: &Files| {
        let d = common::load_all(files);
        let top = d.find("sfyprobe_s/blk.shcomp").expect("blk");
        virtuoso::export(&d, &[top], &Mapping::default(), &ExportOptions::default())
            .expect("exports")
    };
    let out = export(&files);
    for want in [
        r#"("cdfattr" "doneProc" "G_done")"#,
        r#"("cdfattr" "paramDisplayMode" "value")"#,
        r#"("paramattr" "w" "display" "cdfgData->mode->value==\"a\"")"#,
        r#"("paramattr" "w" "editable" "nil")"#,
    ] {
        assert!(out.sfyx.contains(want), "{want}\n{}", out.sfyx);
    }
    // An edited interface (a new param): the CDF is derived, and the loss is a row.
    let mut edited = files.clone();
    let text = blk.replacen("\nparam ", "\nparam extra 1\nparam ", 1);
    edited.insert("sfyprobe_s/blk.shcomp".into(), text.into_bytes());
    let out = export(&edited);
    assert!(
        !out.sfyx.lines().any(|l| l.starts_with("(\"cdfattr\"")),
        "{}",
        out.sfyx
    );
    let r = out.notes.text();
    assert!(
        r.contains("interface edited: CDF doneProc formInitProc")
            && r.contains("w.display w.editable w.use"),
        "{r}"
    );
}

#[test]
fn several_text_displays_of_one_instance_are_one_vtd_line() {
    // Two textDisplays on R1 used to be two `vtd` lines, E024 on load.
    let mut text = fixture("gaps.sfyx");
    let at = r#"("tnet" 1 "MINUS" "n_in")"#;
    let k = text.find(at).expect("conn's tnet");
    text.insert_str(
        k,
        "(\"td\" 1 \"prop\" \"r\" \"annotate\" \"drawing\" 10000 0 \"R0\" \"lowerLeft\" \"stick\" 10000 1 0 1 1 1)\n(\"td\" 1 \"prop\" \"m\" \"annotate\" \"drawing\" 10000 -10000 \"R0\" \"lowerLeft\" \"stick\" 10000 1 0 1 1 1)\n",
    );
    let imp = virtuoso::import(
        &text,
        &Files::new(),
        &Mapping::default(),
        &ImportOptions::default(),
    )
    .expect("imports");
    let files: Files = imp.files.into_iter().collect();
    let problems = interop::check_output(&files.clone().into_iter().collect::<Vec<_>>());
    assert!(problems.is_empty(), "{problems:#?}");
    let conn = String::from_utf8(files["sfyprobe_s/conn.shbench"].clone()).unwrap();
    assert_eq!(conn.matches("  vtd ").count(), 1, "{conn}");
    let d = common::load_all(&files);
    let top = d.find("sfyprobe_s/conn.shbench").expect("conn");
    let out = virtuoso::export(&d, &[top], &Mapping::default(), &ExportOptions::default())
        .expect("exports");
    assert_eq!(out.sfyx.matches("(\"td\" ").count(), 2, "{}", out.sfyx);
}

#[test]
fn cds_param_counts_value_only_words() {
    // cdfuser ch. 5: a `-` word of paramLabelSet shows its value only, and is still a word.
    let (_, r) = import();
    assert!(!r.contains("symbol label `cdsParam(3)` dropped"), "{r}");
}

#[test]
fn a_two_group_nlp_label_and_a_radio_param_cross_both_ways() {
    // Live s_nlp (fixtures/virtuoso/gaps_nlp.sfyx): G23, a label `[@w:w=%:w?][@l: l=%]`;
    // S17, a radio param.
    let imp = virtuoso::import(
        &fixture("gaps_nlp.sfyx"),
        &Files::new(),
        &Mapping::default(),
        &ImportOptions::default(),
    )
    .expect("imports");
    let r = imp.notes.text();
    let files: Files = imp.files.into_iter().collect();
    let two = String::from_utf8(files["sfyprobe_s/two.shprim"].clone()).unwrap();
    assert!(
        two.contains("R0 {w={w} l={l}}") || two.contains("w={w} l={l}"),
        "{two}"
    );
    assert!(two.contains("vradio true"), "{two}");
    assert!(
        r.contains("the fallback(s) `w?` for an unset value not shown"),
        "{r}"
    );
    // `two` is a mirror (no schematic): it never goes back. The same label and a radio
    // param on blk (a component) do, also once its interface is edited.
    let mut text = fixture("gaps_more.sfyx");
    let p = r#"("param" "mode" "cyclic" "a" "mode" "" () ("a" "b") "")"#;
    assert!(text.contains(p));
    text = text.replace(p, &p.replace("cyclic", "radio"));
    let sh = r#"("shape" 3 "rect" "device" "drawing" (-80000 -40000 80000 40000) 0)
("cell" "cg" "design")"#;
    assert!(text.contains(sh));
    text = text.replace(
        sh,
        &format!(
            "(\"label\" 4 0 \"[@w:w=%:w?][@l: l=%]\" 0 48000 \"R0\" \"lowerLeft\" \"stick\" 10000 \"NLP\" \"annotate\" \"drawing\")\n{sh}"
        ),
    );
    let files: Files = virtuoso::import(
        &text,
        &Files::new(),
        &Mapping::default(),
        &ImportOptions::default(),
    )
    .expect("imports")
    .files
    .into_iter()
    .collect();
    let blk = String::from_utf8(files["sfyprobe_s/blk.shcomp"].clone()).unwrap();
    assert!(blk.contains("vradio true"), "{blk}");
    let export = |files: &Files| {
        let d = common::load_all(files);
        let top = d.find("sfyprobe_s/blk.shcomp").expect("blk");
        virtuoso::export(&d, &[top], &Mapping::default(), &ExportOptions::default())
            .expect("exports")
            .sfyx
    };
    let out = export(&files);
    assert!(out.contains(r#""[@w:w=%:w?][@l: l=%]""#), "{out}");
    assert!(out.contains(r#"("param" "mode" "radio""#), "{out}");
    let mut edited = files.clone();
    edited.insert(
        "sfyprobe_s/blk.shcomp".into(),
        blk.replacen("\nparam ", "\nparam extra 1\nparam ", 1)
            .into_bytes(),
    );
    let out = export(&edited);
    assert!(out.contains(r#"("param" "mode" "radio""#), "{out}");
    assert!(out.contains(r#""[@w:w=%:w?][@l: l=%]""#), "{out}");
}
