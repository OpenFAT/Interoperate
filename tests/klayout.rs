//! Layout through KLayout (gap H, `interop::klayout`): the `.lyp`/`.lyt` generator, the
//! export into a project and the import's restore-or-strmin choice, over the probe's real
//! files (`tests/fixtures/cadence/layout/`, local-only: those tests skip without them) and
//! a display-resource dump written here. KLayout itself checks the generated files when it
//! is installed (`KLAYOUT`, else `klayout` on PATH).

mod common;

use std::collections::BTreeMap;
use std::path::Path;

use interop::klayout::{self, Files, Mode};

/// A `drDumpDrf` file in the shape of the Virtuoso host's (probe `layout`), with values of its
/// own: a 4×4 stipple and a dashed line style for y1.
const DRF: &str = "\
drDefineDisplay(
;( DisplayName )
 ( display       )
)
drDefineColor(
;( DisplayName   ColorsName   Red      Green    Blue     )
 ( display       yellow       255      255      0        )
 ( display       red          255      0        0        )
 ( display       blinkRed     255      0        0        t )
 ( display       navy         51       51       153      )
)
drDefineStipple(
;( DisplayName   StippleName  Bitmap     )
 ( display       blank        ( ( 0 0 0 0 )
                                ( 0 0 0 0 ) ) )
 ( display       diag4        ( ( 1 0 0 0  )
                                ( 0 1 0 0  )
                                ( 0 0 1 0  )
                                ( 0 0 0 1  ) ) )
)
drDefineLineStyle(
;( DisplayName   LineStyle    Size       Pattern   )
 ( display       solid        1          (1 1 1 ) )
 ( display       dashed       2          (1 1 1 1 0 0 ) )
)
drDefinePacket(
;( DisplayName  PacketName           Stipple    LineStyle  Fill       Outline    [FillStyle])
 ( display      y0                   blank      solid      yellow     yellow     outline   )
 ( display      y1                   diag4      dashed     red        navy       stipple   )
)
";

/// `layout.sfyx` of the probe's `sfyprobe_lib/lay/layout` (probe `layout`: the LPPs of y0
/// and y1 with their packets and priorities, the PCell instance, the object counts).
const INFO: &str = "\
(\"sfyx\" 1 \"schemify.il SfyLayoutExport\")
(\"layout\" \"sfyprobe_lib\" \"lay\" \"layout\" \"cdsDefTechLib\")
(\"dbu\" \"1000.0\")
(\"lp\" \"y1\" \"drawing\" \"y1\" 43 \"\")
(\"lp\" \"y0\" \"drawing\" \"y0\" 42 \"t\")
(\"lp\" \"y0\" \"flight\" \"y0Flt\" 111 \"t\")
(\"pcell\" \"PC0\" \"sfyprobe_lib\" \"pc\" \"w=2.0\")
(\"oa\" \"net\" 1)
(\"oa\" \"label\" 1)
(\"oa\" \"via\" 0)
(\"end\")
";

const MAP: &str = " y0  drawing  207  0 \n y1  drawing  208  0 \n";

fn info() -> klayout::Info {
    klayout::read_info(INFO).unwrap()
}

#[test]
fn the_lyp_draws_each_mapped_lpp_as_its_packet() {
    let drf = klayout::read_drf(DRF).unwrap();
    assert_eq!(drf.colors["navy"], [51, 51, 153]);
    assert_eq!(drf.stipples["diag4"][1], [false, true, false, false]);
    assert_eq!(drf.line_styles["dashed"].0, 2);
    let map = klayout::read_layer_map(MAP);
    let lyp = klayout::lyp(&map, &info(), &drf);
    let props: Vec<&str> = lyp.split("<properties>").skip(1).collect();
    // Priority order: y0 (42) before y1 (43), although the map lists y1 second anyway.
    assert!(
        props[0].contains("<name>y0.drawing</name><source>207/0@1</source>"),
        "{lyp}"
    );
    assert!(
        props[0].contains("<frame-color>#ffff00</frame-color><fill-color>#ffff00</fill-color>")
    );
    assert!(props[0].contains("<dither-pattern>I1</dither-pattern><line-style>I0</line-style>"));
    assert!(props[0].contains("<visible>true</visible>"));
    assert!(
        props[1].contains("<frame-color>#333399</frame-color><fill-color>#ff0000</fill-color>")
    );
    assert!(props[1].contains("<dither-pattern>C0</dither-pattern><line-style>C0</line-style>"));
    assert!(props[1].contains("<visible>false</visible>") && props[1].contains("<width>2</width>"));
    assert!(lyp.contains(
        "<custom-dither-pattern><pattern><line>*...</line><line>.*..</line><line>..*.</line>\
         <line>...*</line></pattern><order>1</order><name>diag4</name>"
    ));
    assert!(lyp.contains(
        "<custom-line-style><pattern>****..</pattern><order>1</order><name>dashed</name>"
    ));
    let lyt = klayout::lyt(&map, &info(), "cdsDefTechLib.lyp").unwrap();
    assert!(lyt.contains("<dbu>0.001</dbu>"));
    assert!(lyt.contains("layer_map('207/0 : y0.drawing (207/0)';'208/0 : y1.drawing (208/0)')"));
}

/// KLayout, when installed: `KLAYOUT`, else `klayout` on PATH.
fn klayout_bin() -> Option<String> {
    let k = std::env::var("KLAYOUT").unwrap_or_else(|_| "klayout".into());
    let ok = std::process::Command::new(&k)
        .arg("-v")
        .output()
        .is_ok_and(|o| o.status.success());
    if !ok {
        eprintln!("{k}: not runnable; KLayout check skipped");
    }
    ok.then_some(k)
}

#[test]
fn klayout_loads_the_generated_lyt_and_lyp_over_the_probe_gds() {
    let (Some(gds), Some(k)) = (common::cadence_fixture("layout/det1.gds"), klayout_bin()) else {
        return;
    };
    let dir = std::env::temp_dir().join(format!("sfy-klayout-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let map = klayout::read_layer_map(MAP);
    let drf = klayout::read_drf(DRF).unwrap();
    std::fs::write(dir.join("t.lyp"), klayout::lyp(&map, &info(), &drf)).unwrap();
    std::fs::write(
        dir.join("t.lyt"),
        klayout::lyt(&map, &info(), "t.lyp").unwrap(),
    )
    .unwrap();
    std::fs::write(
        dir.join("t.py"),
        "import pya\n\
         t = pya.Technology.technology_from_xml(open(tech).read())\n\
         o = t.load_layout_options\n\
         v = pya.LayoutView()\n\
         v.load_layout(gds, o, True)\n\
         v.load_layer_props(lyp)\n\
         ly = v.cellview(0).layout()\n\
         print('tech', t.name, t.dbu, t.layer_properties_file)\n\
         print('layers', sorted(str(ly.get_info(i)) for i in ly.layer_indexes()))\n\
         print('arrays', sum(1 for i in ly.top_cell().each_inst() if i.is_regular_array()))\n\
         it = v.begin_layers()\n\
         while not it.at_end():\n\
         \x20   q = it.current()\n\
         \x20   print('layer', q.name, q.source, '%06x' % (q.frame_color & 0xffffff), '%06x' % (q.fill_color & 0xffffff), q.width, q.visible, q.xfill, repr(v.get_stipple(q.dither_pattern)), repr(v.get_line_style(q.line_style)))\n\
         \x20   it.next()\n",
    )
    .unwrap();
    let out = std::process::Command::new(&k)
        .env("QT_QPA_PLATFORM", "offscreen")
        .args(["-b", "-r"])
        .arg(dir.join("t.py"))
        .arg("-rd")
        .arg(format!("tech={}", dir.join("t.lyt").display()))
        .arg("-rd")
        .arg(format!("lyp={}", dir.join("t.lyp").display()))
        .arg("-rd")
        .arg(format!("gds={}", gds.display()))
        .output()
        .unwrap();
    let text =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(text.contains("tech cdsDefTechLib 0.001 t.lyp"), "{text}");
    assert!(
        text.contains("layers ['y0.drawing (207/0)', 'y1.drawing (208/0)']"),
        "{text}"
    );
    // The 2×3 mosaic streams as one aref (probe `stream`).
    assert!(text.contains("arrays 1"), "{text}");
    assert!(
        text.contains("layer y0.drawing 207/0@1 ffff00 ffff00 1 True False"),
        "{text}"
    );
    assert!(
        text.contains("layer y1.drawing 208/0@1 333399 ff0000 2 False False '*...\\n.*..\\n..*.\\n...*\\n' '****..'"),
        "{text}"
    );
}

/// The export bundle of `lay` from the probe's files: its GDS, layer map and OA files.
fn probe_bundle() -> Option<Files> {
    let d = common::cadence_fixture("layout")?;
    let read = |p: &str| std::fs::read(d.join(p)).unwrap();
    let mut b = Files::new();
    b.insert("layout.sfyx".into(), INFO.as_bytes().to_vec());
    b.insert("display.drf".into(), DRF.as_bytes().to_vec());
    b.insert("layermap".into(), read("xStrmOut_layerMap.txt"));
    b.insert("layout.gds".into(), read("det1.gds"));
    for f in ["layout.oa", "master.tag", "thumbnail_128x128.png"] {
        b.insert(format!("oa/{f}"), read(&format!("oa_lay/{f}")));
    }
    Some(b)
}

const STASH: &str = ".schemify/virtuoso/sfyprobe_lib/lay/layout";

/// The stash part of an export's files, relative to the stash directory.
fn stash_of(out: &Files) -> Files {
    out.iter()
        .filter_map(|(p, b)| {
            Some((
                p.strip_prefix(STASH)?.strip_prefix('/')?.to_owned(),
                b.clone(),
            ))
        })
        .collect()
}

#[test]
fn an_export_lays_the_probe_view_out_in_the_project() {
    let Some(b) = probe_bundle() else { return };
    let out = klayout::export(&b, "probe", None).unwrap();
    let names: Vec<&str> = out.keys().map(String::as_str).collect();
    assert_eq!(
        names,
        [
            ".schemify/virtuoso/sfyprobe_lib/lay/layout/SHA256SUMS",
            ".schemify/virtuoso/sfyprobe_lib/lay/layout/lay.layout.gds",
            ".schemify/virtuoso/sfyprobe_lib/lay/layout/layermap",
            ".schemify/virtuoso/sfyprobe_lib/lay/layout/layout.sfyx",
            ".schemify/virtuoso/sfyprobe_lib/lay/layout/oa/layout.oa",
            ".schemify/virtuoso/sfyprobe_lib/lay/layout/oa/master.tag",
            ".schemify/virtuoso/sfyprobe_lib/lay/layout/oa/thumbnail_128x128.png",
            "probe/cdsDefTechLib.lyp",
            "probe/cdsDefTechLib.lyt",
            "probe/lay.layout.gds",
        ]
    );
    assert_eq!(out["probe/lay.layout.gds"], b["layout.gds"]);
    // The digests are sha256sum's (computed by it over the fixture files).
    let sums = String::from_utf8(out[&format!("{STASH}/SHA256SUMS")].clone()).unwrap();
    assert_eq!(
        sums,
        "13fcb620406d760550c30d32a42e0bf011f9a0be9a3383aa2ea012bd2cbb942d  lay.layout.gds\n\
         ecbea8863539a54ff32fb3b39700cad804dbc3bc57f570006a847f2e8ea2b714  oa/layout.oa\n\
         0547da475c6495aab98e43894e8e330ed09303a205699e8413c35388b6906a05  oa/master.tag\n\
         7e2f34e4a4ed5b73d496e389e0315eb9050b81bf64265fccf5d647d1858c5689  oa/thumbnail_128x128.png\n"
    );
}

#[test]
fn deterministic_streams_keep_the_digest_and_timestamped_ones_do_not() {
    let Some(d) = common::cadence_fixture("layout") else {
        return;
    };
    let h = |p: &str| klayout::sha256(&std::fs::read(d.join(p)).unwrap());
    // -ignoreCurrentTimestamp: two runs, one digest, so an unedited GDS restores.
    assert_eq!(h("det1.gds"), h("det2.gds"));
    // Without it every export would read as an edit.
    assert_ne!(h("ts1.gds"), h("ts2.gds"));
    // strmin rebuilds the view: its layout.oa is not the original even for the same
    // geometry, so only the stash gives the original back.
    assert_ne!(h("strmin_lay/layout.oa"), h("oa_lay/layout.oa"));
}

#[test]
fn an_unchanged_gds_restores_the_oa_files_byte_for_byte() {
    let Some(b) = probe_bundle() else { return };
    let stash = stash_of(&klayout::export(&b, "probe", None).unwrap());
    let imp = klayout::import(&stash, &b["layout.gds"]).unwrap();
    assert_eq!(imp.mode, Mode::Restore);
    assert!(imp.notes.rows.is_empty());
    for f in ["layout.oa", "master.tag", "thumbnail_128x128.png"] {
        assert_eq!(imp.files[&format!("oa/{f}")], b[&format!("oa/{f}")], "{f}");
    }
    assert!(!imp.files.contains_key("layout.gds"));
    let plan = String::from_utf8(imp.files["layout.sfyx"].clone()).unwrap();
    assert!(plan.contains(
        "(\"layout\" \"sfyprobe_lib\" \"lay\" \"layout\" \"cdsDefTechLib\")\n(\"mode\" \"restore\")"
    ));
    // oa.sha256 checks the view directory itself (`cd <view> && sha256sum -c`).
    let sums = String::from_utf8(imp.files["oa.sha256"].clone()).unwrap();
    assert!(sums.starts_with(
        "ecbea8863539a54ff32fb3b39700cad804dbc3bc57f570006a847f2e8ea2b714  layout.oa\n"
    ));
}

#[test]
fn an_edited_gds_goes_through_strmin_and_reports_what_gds_loses() {
    let Some(b) = probe_bundle() else { return };
    let d = common::cadence_fixture("layout").unwrap();
    let stash = stash_of(&klayout::export(&b, "probe", None).unwrap());
    // The probe's strmin of det1, streamed out again: other bytes, same geometry.
    let edited = std::fs::read(d.join("back_sfyprobe_in.gds")).unwrap();
    let imp = klayout::import(&stash, &edited).unwrap();
    assert_eq!(imp.mode, Mode::Strmin);
    assert_eq!(imp.files["layout.gds"], edited);
    assert_eq!(
        imp.files["layermap"], b["layermap"],
        "the export's own map (XSTRM-74)"
    );
    assert!(!imp.files.keys().any(|p| p.starts_with("oa/")));
    let report = imp.notes.text();
    for want in [
        "PCell instance PC0 of sfyprobe_lib/pc (w=2.0) becomes an instance of a plain cell",
        "1 net: nets: only pins come back",
        "1 label: labels: font and height",
        "constraints of the view",
    ] {
        assert!(report.contains(want), "{want}\n{report}");
    }
    assert!(!report.contains("via"), "a zero count is no loss: {report}");
}

#[test]
fn a_changed_stash_is_refused() {
    let Some(b) = probe_bundle() else { return };
    let mut stash = stash_of(&klayout::export(&b, "probe", None).unwrap());
    stash.get_mut("oa/layout.oa").unwrap()[100] ^= 1;
    let e = klayout::import(&stash, &b["layout.gds"]).unwrap_err();
    assert!(e.contains("oa/layout.oa is not the file exported"), "{e}");
}

#[test]
fn vlayout_names_the_gds_of_each_view_once() {
    let cell = b"schemify 1 shbench\n  vsch 0123\n  vlayout layout old\n  vlayout lay2 keep\n";
    let out = klayout::with_vlayout(cell, "layout", "new").unwrap();
    let text = String::from_utf8(out).unwrap();
    assert_eq!(
        text,
        "schemify 1 shbench\n  vsch 0123\n  vlayout lay2 keep\n  vlayout layout new\n"
    );
}

#[test]
fn an_extracted_netlist_becomes_a_code_block_of_the_component() {
    let inv = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/virtuoso/inverter/demo/inv.shcomp");
    let cell = std::fs::read(inv).unwrap();
    let netlist = "// av_extracted\nsimulator lang=spectre\nsubckt inv in out vdd vss\n\
                   M0 (out in vss vss) nch w=1u l=100n\nc1 (out vss) capacitor c=1.2f\nends inv\n";
    let once = klayout::with_code(&cell, "av_extracted", netlist, "inv").unwrap();
    let twice = klayout::with_code(&once, "av_extracted", netlist, "inv").unwrap();
    assert_eq!(
        once, twice,
        "the block of the view is replaced, not added again"
    );
    let text = String::from_utf8(once).unwrap();
    assert!(
        text.ends_with(
            "code av_extracted spectre\n  | M0 (out in vss vss) nch w=1u l=100n\n  | c1 (out vss) capacitor c=1.2f\n"
        ),
        "{text}"
    );
    // The file still loads and round-trips as the original did.
    let check = |b: Vec<u8>| interop::check_output(&[("lib/inv.shcomp".into(), b)]);
    assert_eq!(check(text.into_bytes()), check(cell));
}

#[test]
fn read_sha256sums_reads_what_sha256sums_writes() {
    let t = klayout::sha256sums([("b", &b"2"[..]), ("a", &b"1"[..])]);
    let m: BTreeMap<String, String> = klayout::read_sha256sums(&t);
    assert_eq!(m.keys().collect::<Vec<_>>(), ["a", "b"]);
    assert_eq!(m["a"], klayout::sha256(b"1"));
}
