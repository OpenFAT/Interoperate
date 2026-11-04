//! `schemify-interop pdk-map` on a made-up kit: `SfyCdfNames` dumps the kit's Virtuoso cells
//! (`tests/fixtures/skill/pdkmap_kit.il`) through the mock, `pdk-map` maps the kit's scanned
//! cells (`tests/fixtures/pdk_map/scan`, `pdk-scan`'s cells of `kit.sp`) onto them, and the
//! map passes Interop's own check and re-masters an instance in a `virtuoso-export`.
#![cfg(feature = "cli")]

mod skill;

use std::path::{Path, PathBuf};

use interop::map::Mapping;
use skill::mockdb::MockDb;
use skill::{Interp, V};

const BUILD: &str = include_str!("fixtures/skill/build.il");
const KIT: &str = include_str!("fixtures/skill/pdkmap_kit.il");

fn fixture(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/pdk_map")
        .join(rel)
}

fn run(i: &mut Interp<MockDb>, src: &str) -> V {
    i.load(src)
        .unwrap_or_else(|e| panic!("{src}: {e}\n{}", i.log))
}

/// The kit in `vkit`, and `rbaz` again in `vkit2`.
fn mock() -> Interp<MockDb> {
    let mut i = skill::virtuoso(&["vkit", "vkit2"]);
    i.load(BUILD).unwrap();
    i.load(KIT).unwrap();
    run(&mut i, r#"(SfyFixPdkMapKit "vkit")"#);
    run(&mut i, r#"(SfyFixRbaz "vkit2")"#);
    i
}

/// `SfyCdfNames` of `libs` (a SKILL list), checked PASS.
fn names(i: &mut Interp<MockDb>, libs: &str) -> String {
    let ok = run(i, &format!(r#"(SfyCdfNames "/c/cdf.sfyx" {libs})"#));
    assert!(ok.truthy(), "{}", i.files["/c/cdf.sfyx.log"]);
    i.files["/c/cdf.sfyx"].clone()
}

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("sfy-pdkmap-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn cli(args: &[&Path]) -> u8 {
    let args: Vec<String> = args
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    interop::cli::run(&args)
}

/// `pdk-map` of the scan fixture as library `fkit` against `dump`: (map, report rows).
fn pdk_map(dir: &Path, dump: &str) -> (String, String) {
    std::fs::write(dir.join("cdf.sfyx"), dump).unwrap();
    let code = cli(&[
        Path::new("pdk-map"),
        Path::new("--scan"),
        &fixture("scan"),
        Path::new("--dump"),
        &dir.join("cdf.sfyx"),
        Path::new("--lib"),
        Path::new("fkit"),
        Path::new("-o"),
        &dir.join("interop.map"),
    ]);
    assert_eq!(code, 0);
    let read = |f: &str| std::fs::read_to_string(dir.join(f)).unwrap();
    assert!(read("interop.report.md").starts_with("# PDK map\n"));
    (read("interop.map"), read("interop.report.txt"))
}

#[test]
fn the_scan_fixture_is_pdk_scans_output() {
    let root = fixture("");
    let args = [
        "--root",
        root.to_str().unwrap(),
        "--entry",
        "kit.sp",
        "--dialect",
        "spice",
        "--lib",
        "fkit",
    ];
    let (out, code) = interop::pdk_scan::run(&args.map(String::from), b"");
    assert_eq!(code, 0);
    let out = String::from_utf8(out).unwrap();
    let mut n = 0;
    for f in std::fs::read_dir(fixture("scan")).unwrap() {
        let p = f.unwrap().path();
        let name = p.file_name().unwrap().to_str().unwrap();
        let body = std::fs::read_to_string(&p).unwrap();
        assert!(
            out.contains(&format!("file {name} {}\n{body}", body.len())),
            "{name} is stale: {out}"
        );
        n += 1;
    }
    assert_eq!(n, out.matches("\nfile ").count());
}

#[test]
fn sfy_cdf_names_dumps_names_only() {
    let mut i = mock();
    let t = names(&mut i, r#"(list "vkit" "vkit2")"#);
    assert_eq!(
        t,
        r#"("sfyx" 1 "schemify.il 1")
("cdfnames" "vkit" "nfoo" "nfoo" ("D" "G" "S" "B") ("model" "w" "l" "nf" "m") ("D" "G" "S"))
("cdfnames" "vkit" "nfoo_b" "nfoo" ("D" "G" "S" "B") ("model" "w" "l" "nf" "m") ("B" "D" "G" "S"))
("cdfnames" "vkit" "nocdf" "" () () ("A" "B"))
("cdfnames" "vkit" "pbar" "pbar" ("D" "G" "S") ("model" "w" "l") ("D" "G" "S"))
("cdfnames" "vkit" "rbaz" "rbaz" ("PLUS" "MINUS") ("model" "r" "m") ("MINUS" "PLUS"))
("cdfnames" "vkit2" "rbaz" "rbaz" ("PLUS" "MINUS") ("model" "r" "m") ("MINUS" "PLUS"))
("end")
"#
    );
    // A library missing from cds.lib fails the run; the rest still dumps.
    let ok = run(
        &mut i,
        r#"(SfyCdfNames "/c/x.sfyx" (list "nolib" "vkit2"))"#,
    );
    assert!(!ok.truthy());
    assert!(i.files["/c/x.sfyx.log"].contains("library nolib is not in cds.lib"));
    assert!(i.files["/c/x.sfyx"].contains(r#"("cdfnames" "vkit2" "rbaz""#));
}

#[test]
fn the_map_picks_by_the_rules_and_reports_every_cell() {
    let mut i = mock();
    let dir = tmp("rules");
    let (map, report) = pdk_map(&dir, &names(&mut i, r#"(list "vkit" "vkit2")"#));
    assert_eq!(
        map,
        "map fkit:nfoo virtuoso vkit nfoo_b\n  pin d D\n  pin g G\n  pin s S\n  pin b B\n  param w w\n  param l l\n  param nf nf\n  param m m\n\
         map fkit:rbaz virtuoso vkit rbaz\n  pin p PLUS\n  pin n MINUS\n  param r r\n  param m m\n"
    );
    assert_eq!(Mapping::parse(&map).unwrap().maps.len(), 2);
    assert_eq!(
        report,
        "mapped\tfkit:nfoo\tvkit/nfoo_b: over vkit/nfoo by its symbol's terminals are its termOrder\n\
         mapped\tfkit:rbaz\tvkit/rbaz: over vkit2/rbaz by library order\n\
         needs_review:unmapped\tfkit:nocdf\tno candidate: no Virtuoso cell's CDF model is nocdf (vkit/nocdf has no CDF model)\n\
         needs_review:unmapped\tfkit:pbar\tpin-count mismatch: 4 pins; termOrder: vkit/pbar has 3\n\
         needs_review:unmapped\tfkit:qux\tno candidate: no Virtuoso cell's CDF model is qux\n"
    );
    // The libraries in the other order: rbaz goes to vkit2's.
    let (map, _) = pdk_map(&dir, &names(&mut i, r#"(list "vkit2" "vkit")"#));
    assert!(map.contains("map fkit:rbaz virtuoso vkit2 rbaz\n"), "{map}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_map_remasters_in_a_virtuoso_export() {
    let mut i = mock();
    let dir = tmp("export");
    let (map, _) = pdk_map(&dir, &names(&mut i, r#"(list "vkit" "vkit2")"#));
    let proj = dir.join("proj");
    std::fs::create_dir_all(proj.join("fkit")).unwrap();
    std::fs::write(proj.join("interop.map"), map).unwrap();
    std::fs::write(
        proj.join("schemify.project"),
        "schemify 1 project\n\nlib fkit path fkit\n",
    )
    .unwrap();
    for c in ["nfoo", "rbaz"] {
        std::fs::copy(
            fixture(&format!("scan/{c}.shprim")),
            proj.join(format!("fkit/{c}.shprim")),
        )
        .unwrap();
    }
    std::fs::write(
        proj.join("tb.shbench"),
        "schemify 1 shbench\n\ninst aaaaaa fkit:nfoo\n  at 0 0 R0\n  name M1\n  w 2u\n",
    )
    .unwrap();
    let code = cli(&[
        Path::new("virtuoso-export"),
        &proj,
        Path::new("tb.shbench"),
        Path::new("-o"),
        Path::new("tb.sfyx"),
    ]);
    assert_eq!(code, 0);
    let sfyx = std::fs::read_to_string(proj.join("tb.sfyx")).unwrap();
    assert!(
        sfyx.contains(r#"("inst" 1 "M1" "vkit" "nfoo_b" "symbol" 0 0 "R0")"#),
        "{sfyx}"
    );
    assert_eq!(sfyx.matches(r#"("conn" 1 "#).count(), 4, "{sfyx}");
    for t in ["D", "G", "S", "B"] {
        assert!(
            sfyx.contains(&format!(r#"("conn" 1 "{t}" "#)),
            "{t}: {sfyx}"
        );
    }
    let report = std::fs::read_to_string(proj.join("tb.report.txt")).unwrap();
    assert!(report.contains("remapped"), "{report}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A fake `ssh` (runs the remote command here) and a fake `virtuoso` (checks its
/// arguments and the batch file, notes its environment in `seen`, writes `dump` as
/// SfyCdfNames would) in `dir/bin`; returns that directory.
#[cfg(unix)]
fn fakes(dir: &Path, dump: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::write(dir.join("dump.sfyx"), dump).unwrap();
    let virtuoso = format!(
        "#!/bin/sh\n[ \"$1\" = -nograph ] && [ \"$2\" = -restore ] || exit 9\n\
         grep -q '(SfyCdfNames .* (list \"vkit\" \"vkit2\"))' \"$3\" || exit 9\n\
         echo \"lic=$CDS_LIC_FILE only=${{CDS_LIC_ONLY:-}} setup=${{FAKE_SETUP:-}} pwd=$(pwd)\" >{seen}\n\
         cp {dump} \"$SFY_PDKMAP_DIR/cdf.sfyx\"\necho PASS >\"$SFY_PDKMAP_DIR/cdf.sfyx.log\"\n",
        seen = dir.join("seen").display(),
        dump = dir.join("dump.sfyx").display(),
    );
    for (name, body) in [
        ("ssh", "#!/bin/sh\nshift\nexec \"$@\"\n"),
        ("virtuoso", &virtuoso),
    ] {
        let p = bin.join(name);
        std::fs::write(&p, body).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    bin
}

/// `pdk-map --host` through the fakes, with `env` set and the other settings unset.
#[cfg(unix)]
fn host_map(dir: &Path, bin: &Path, env: &[(&str, String)]) -> std::process::Output {
    let mut c = std::process::Command::new(env!("CARGO_BIN_EXE_schemify-interop"));
    c.args(["pdk-map", "--scan"])
        .arg(fixture("scan"))
        .args([
            "--host", "kithost", "--vlib", "vkit", "--vlib", "vkit2", "--lib", "fkit", "-o",
        ])
        .arg(dir.join("interop.map"))
        .env(
            "PATH",
            format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
        );
    for (k, _) in interop::pdk_map::ENV {
        c.env_remove(k);
    }
    c.envs(env.iter().map(|(k, v)| (k, v))).output().unwrap()
}

#[test]
#[cfg(unix)] // the fakes are sh scripts
fn host_runs_the_dump_with_this_environment_and_maps_it() {
    let mut i = mock();
    let dir = tmp("host");
    let bin = fakes(&dir, &names(&mut i, r#"(list "vkit" "vkit2")"#));
    let cds = dir.join("cds");
    std::fs::create_dir_all(&cds).unwrap();
    std::fs::write(
        dir.join("setup.sh"),
        "CDS_LIC_FILE=from-setup\nFAKE_SETUP=yes\nexport CDS_LIC_FILE FAKE_SETUP\n",
    )
    .unwrap();
    let out = host_map(
        &dir,
        &bin,
        &[
            ("VIRTUOSO", bin.join("virtuoso").display().to_string()),
            (
                "SFY_CADENCE_ENV",
                dir.join("setup.sh").display().to_string(),
            ),
            ("SFY_CDS_DIR", cds.display().to_string()),
            ("CDS_LIC_FILE", "it's given".into()),
        ],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    // The setup ran, the licence given here won over the setup's, Virtuoso ran in the
    // cds.lib directory.
    assert_eq!(
        std::fs::read_to_string(dir.join("seen")).unwrap(),
        format!("lic=it's given only= setup=yes pwd={}\n", cds.display())
    );
    let map = std::fs::read_to_string(dir.join("interop.map")).unwrap();
    assert!(
        map.starts_with("map fkit:nfoo virtuoso vkit nfoo_b\n"),
        "{map}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[cfg(unix)] // the fakes are sh scripts
fn host_without_virtuoso_names_the_variables_to_set() {
    let dir = tmp("novirt");
    let bin = fakes(&dir, "");
    let out = host_map(
        &dir,
        &bin,
        &[("VIRTUOSO", "/nonexistent/virtuoso -64".into())],
    );
    assert_eq!(out.status.code(), Some(2));
    let err = String::from_utf8_lossy(&out.stderr);
    for want in [
        "no Virtuoso command `/nonexistent/virtuoso` on PATH",
        "VIRTUOSO (the Virtuoso command)",
        "SFY_CADENCE_ENV",
        "CDS_LIC_FILE or CDS_LIC_ONLY",
        "SFY_CDS_DIR",
        "the dump on kithost failed",
    ] {
        assert!(err.contains(want), "{want}: {err}");
    }
    assert!(!dir.join("interop.map").exists());
    let _ = std::fs::remove_dir_all(&dir);
}
