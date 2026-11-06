//! `schemify-interop pdk-scan` on a made-up HSPICE-style kit (`tests/fixtures/pdk_scan/kit`):
//! the cells, no card data, determinism, `--stdin` against the filesystem, `need` lines, the
//! remote script through a real `sh`, and the root confinement.
#![cfg(feature = "cli")]

use std::path::PathBuf;

use interop::pdk_scan;

fn root() -> PathBuf {
    std::fs::canonicalize(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/pdk_scan/kit"
    ))
    .expect("fixture kit")
}

fn scan(args: &[&str], stdin: &[u8]) -> (String, u8) {
    let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    let (out, code) = pdk_scan::run(&args, stdin);
    (String::from_utf8(out).expect("utf-8 output"), code)
}

fn scan_fs(section: &str) -> (String, u8) {
    let root = root();
    let args = [
        "--root",
        root.to_str().unwrap(),
        "--entry",
        "models/top.lib",
    ];
    scan(
        &[&args[..], &["--dialect", "hspice", "--section", section]].concat(),
        b"",
    )
}

/// `(files, other lines)` of protocol 1 output, checking its frame.
fn parse(out: &str) -> (Vec<(String, String)>, Vec<String>) {
    let mut rest = out.strip_prefix("pdkscan 1\n").expect("header");
    let (mut files, mut lines) = (Vec::new(), Vec::new());
    while let Some((line, tail)) = rest.split_once('\n') {
        rest = tail;
        if let Some(f) = line.strip_prefix("file ") {
            let (path, len) = f.rsplit_once(' ').unwrap();
            let len: usize = len.parse().unwrap();
            files.push((path.to_owned(), rest[..len].to_owned()));
            rest = &rest[len..];
        } else {
            lines.push(line.to_owned());
        }
    }
    assert!(rest.is_empty() && lines.last().map(String::as_str) == Some("end"));
    (files, lines)
}

/// Frames of `rels` under the kit, as the remote script would print them unfiltered.
fn frames(rels: &[&str]) -> Vec<u8> {
    let mut out = b"sfyframe 1\n".to_vec();
    for r in rels {
        let p = root().join(r);
        let b = std::fs::read(&p).unwrap();
        out.extend(format!("f {} {}\n", b.len(), p.display()).as_bytes());
        out.extend(b);
    }
    out.extend(b"end\n");
    out
}

const KIT_FILES: [&str; 3] = ["models/top.lib", "models/mos.inc", "models/passive.inc"];

#[test]
fn cells_are_the_kit_interface() {
    let (out, code) = scan_fs("tt");
    assert_eq!(code, 0, "{out}");
    let (files, lines) = parse(&out);
    let names: Vec<&str> = files.iter().map(|f| f.0.as_str()).collect();
    // Binned nch.1/nch.2 collapse to nch; the subckt-local nmos_lvt_core is not served.
    assert_eq!(
        names,
        [
            "nch.shprim",
            "nmos_lvt.shprim",
            "pch.shprim",
            "rppoly.shprim"
        ]
    );
    let cell = |n: &str| files.iter().find(|f| f.0 == n).unwrap().1.as_str();
    let lvt = cell("nmos_lvt.shprim");
    let pins: Vec<&str> = lvt.lines().filter_map(|l| l.strip_prefix("pin ")).collect();
    assert_eq!(
        pins,
        [
            "d inout 40 0",
            "g inout 40 20",
            "s inout 40 40",
            "b inout 40 60"
        ]
    );
    let params: Vec<&str> = lvt
        .lines()
        .filter_map(|l| l.strip_prefix("param "))
        .collect();
    assert_eq!(params, ["w \"\"", "l \"\"", "nf \"\""]);
    assert!(lvt.contains(
        r#"format spice "X{name} {d} {g} {s} {b} nmos_lvt{w != \"\" ? \" w=\" + w : \"\"}{l != \"\" ? \" l=\" + l : \"\"}{nf != \"\" ? \" nf=\" + nf : \"\"}""#
    ));
    assert!(lvt.contains(r#"format spectre "{name} ({d} {g} {s} {b}) nmos_lvt{w != \"\" ? "#));
    let nch = cell("nch.shprim");
    assert!(nch.contains(r#"format spice "M{name} {d} {g} {s} {b} nch{w != \"\" ? "#));
    assert!(nch.contains(r#"{m != \"\" ? \" m=\" + m : \"\"}""#));
    assert!(cell("rppoly.shprim").contains(r#"format spice "X{name} {p} {n} rppoly{l "#));
    assert_eq!(
        lines[1..],
        [
            "issue 3 protected (.prot) lines not read",
            "issue models not served (no cell kind): 1 d, 1 r",
            "end"
        ]
    );
    assert!(lines[0].starts_with("version ") && lines[0].len() == 24);
    // The cells load in Schemify, as a library named after the root.
    let mut lib: Vec<(String, Vec<u8>)> = files
        .iter()
        .map(|(p, b)| (format!("kit/{p}"), b.clone().into_bytes()))
        .collect();
    lib.push((
        "schemify.project".into(),
        b"schemify 1 project\n\nlib kit path kit\n".to_vec(),
    ));
    assert_eq!(interop::check_output(&lib), Vec::<String>::new());
}

#[test]
fn no_card_data_in_the_output_and_it_is_deterministic() {
    let (a, _) = scan_fs("tt");
    let (b, _) = scan_fs("ff");
    for out in [&a, &b] {
        for card in [
            "level", "lmin", "vth0", "toxe", "rsh", "0.4123", "0.4321", "1.11e-7", "9.99e",
            "2.22e-7", "8.88e-6", "0.3789", "123.45", "1.23e-14", "-0.0371", "4.44e-7", "1u",
            "100n", "2u", "0.5u", "zz9081", "scale",
        ] {
            assert!(!out.to_lowercase().contains(card), "{card} leaked");
        }
    }
    // Same kit, same bytes: the corners differ only in card values.
    assert_eq!(a, scan_fs("tt").0);
    assert_eq!(a, b);
}

#[test]
fn stdin_frames_give_the_filesystem_output() {
    let root = root();
    let r = root.to_str().unwrap();
    let args = [
        "--stdin",
        "--root",
        r,
        "--entry",
        "models/top.lib",
        "--dialect",
        "hspice",
    ];
    let (out, code) = scan(
        &[&args[..], &["--section", "tt"]].concat(),
        &frames(&KIT_FILES),
    );
    assert_eq!(code, 0);
    assert_eq!(out, scan_fs("tt").0);
    // An include the frames lack: a `need` line, the rest still served.
    let (out, _) = scan(
        &[&args[..], &["--section", "tt"]].concat(),
        &frames(&KIT_FILES[..2]),
    );
    let (files, lines) = parse(&out);
    assert_eq!(files.len(), 4);
    let need = format!("need {}", root.join("models/passive.inc").display());
    assert!(lines.contains(&need), "{lines:?}");
    assert!(lines.contains(&"issue models/top.lib:5: include not found: passive.inc".into()));
    // Without --root the entry is the framed path and nothing is confined.
    let entry = root.join("models/top.lib");
    let bare = [
        "--stdin",
        "--entry",
        entry.to_str().unwrap(),
        "--dialect",
        "spice",
        "--lib",
        "kit",
    ];
    let (out, _) = scan(
        &[&bare[..], &["--section", "tt"]].concat(),
        &frames(&KIT_FILES),
    );
    assert_eq!(parse(&out).0, parse(&scan_fs("tt").0).0);
}

#[test]
#[cfg_attr(
    windows,
    ignore = "the remote script runs in a Unix host's sh; Git Bash mangles verbatim Windows paths"
)]
fn the_remote_script_frames_only_structure() {
    let (script, code) = scan(&["--remote-script"], b"");
    assert_eq!(code, 0);
    let mut paths: Vec<String> = KIT_FILES
        .iter()
        .map(|r| root().join(r).display().to_string())
        .collect();
    paths.push(root().join("models/absent.inc").display().to_string());
    let mut child = std::process::Command::new("sh")
        .args(["-s", "--"])
        .args(&paths)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("sh");
    use std::io::Write;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(script.as_bytes())
        .unwrap();
    let framed = child.wait_with_output().unwrap();
    assert!(framed.status.success());
    let text = String::from_utf8(framed.stdout.clone()).unwrap();
    for card in [
        "level", "lmin", "vth0", "0.4123", "123.45", "1u", "100n", "zz9081", "m0 ",
    ] {
        assert!(
            !text.to_lowercase().contains(card),
            "{card} left the host:\n{text}"
        );
    }
    assert!(
        text.contains(".subckt nmos_lvt d g s b w= l=\n+ nf=\n"),
        "{text}"
    );
    assert!(
        text.contains(".model nch.2 nmos\n.MODEL pch.1 PMOS\n"),
        "{text}"
    );
    assert!(
        text.contains(&format!("f 0 ! {}\nend\n", paths[3])),
        "{text}"
    );
    let r = root();
    let args = [
        "--stdin",
        "--root",
        r.to_str().unwrap(),
        "--entry",
        "models/top.lib",
    ];
    let (out, code) = scan(
        &[&args[..], &["--dialect", "hspice", "--section", "tt"]].concat(),
        &framed.stdout,
    );
    assert_eq!(code, 0, "{out}");
    let (direct, _) = scan_fs("tt");
    assert_eq!(parse(&out).0, parse(&direct).0);
    assert_eq!(out.lines().nth(1), direct.lines().nth(1), "same version");
}

#[test]
fn includes_outside_the_root_are_refused() {
    let (out, code) = scan_fs("escape");
    assert_eq!(code, 0);
    let (files, lines) = parse(&out);
    assert!(files.is_empty() && !out.contains("sneaky"));
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with("issue include outside the root refused: "))
    );
    // The same with frames: lexical confinement under --root.
    let r = root();
    let mut f = b"sfyframe 1\n".to_vec();
    let outside = r.parent().unwrap().join("outside.inc");
    let text = std::fs::read(&outside).unwrap();
    f.extend(format!("f {} {}\n", text.len(), outside.display()).as_bytes());
    f.extend(&text);
    let top = std::fs::read(r.join("models/top.lib")).unwrap();
    f.extend(format!("f {} {}\n", top.len(), r.join("models/top.lib").display()).as_bytes());
    f.extend(&top);
    f.extend(b"end\n");
    let args = [
        "--stdin",
        "--root",
        r.to_str().unwrap(),
        "--entry",
        "models/top.lib",
    ];
    let (out, _) = scan(
        &[&args[..], &["--dialect", "spice", "--section", "escape"]].concat(),
        &f,
    );
    assert!(!out.contains("sneaky") && out.contains("issue include outside the root refused: "));
    // The entry itself must lie under the root.
    let (out, code) = scan(
        &[
            "--root",
            r.to_str().unwrap(),
            "--entry",
            "../outside.inc",
            "--dialect",
            "spice",
        ],
        b"",
    );
    assert_eq!(code, 2);
    assert!(
        out.starts_with("pdkscan 1\nerror \"cannot read entry"),
        "{out}"
    );
}

#[test]
fn protocol_and_errors() {
    assert_eq!(scan(&["--protocol"], b""), ("pdkscan 1\n".into(), 0));
    let r = root();
    assert_eq!(
        scan(
            &[
                "--root",
                r.to_str().unwrap(),
                "--entry",
                "models/top.lib",
                "--dialect",
                "eldo"
            ],
            b""
        ),
        (
            "pdkscan 1\nerror \"unknown dialect eldo: spice, hspice or spectre\"\nend\n".into(),
            2
        )
    );
    let (out, code) = scan(
        &["--entry", "x", "--dialect", "spice", "--stdin"],
        b"garbage",
    );
    assert_eq!(
        (out.as_str(), code),
        (
            "pdkscan 1\nerror \"stdin is not sfyframe 1 input\"\nend\n",
            2
        )
    );
    let (out, code) = scan(&["--bogus"], b"");
    assert!(code == 2 && out.starts_with("pdkscan 1\nerror \"usage: "));
}

// ---------------------------------------------------------------------------- Spectre

fn spectre_root() -> PathBuf {
    std::fs::canonicalize(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/pdk_scan/spectre"
    ))
    .expect("fixture kit")
}

fn scan_spectre() -> (String, u8) {
    let r = spectre_root();
    let args = [
        "--root",
        r.to_str().unwrap(),
        "--entry",
        "top.scs",
        "--dialect",
        "spectre",
    ];
    scan(&args, b"")
}

/// The card values of the Spectre fixture: none may leave the kit.
const SPECTRE_CARDS: [&str; 14] = [
    "0.4123", "1.17e-9", "-0.3876", "0.0123", "-0.0456", "0.031", "1k", "2k", "4.56e-12",
    "7.89e-15", "0.654", "1.23e-3", "0.111", "hidden",
];

/// A Spectre kit read with SpiceRack: subckt wrappers (an `inline subckt` and a `\`
/// continuation included) and the top-level compact models are cells, names in their own
/// case; corner sections are not cells, and only the selected one's include gives cells; no card
/// value appears.
#[test]
fn a_spectre_kit_reads_through_spicerack() {
    let (out, code) = scan_spectre();
    assert_eq!(code, 0, "{out}");
    let (files, lines) = parse(&out);
    let names: Vec<&str> = files.iter().map(|f| f.0.as_str()).collect();
    assert_eq!(
        names,
        [
            "nch_core.shprim",
            "nfet_LVT.shprim",
            "pch_core.shprim",
            "pfet_lvt.shprim",
            "rhigh.shprim",
            "tt_only.shprim",
            "twoport.shprim",
            "varactor_va.shprim"
        ],
        "{out}"
    );
    let cell = |n: &str| &files.iter().find(|f| f.0 == n).unwrap().1;
    // Verilog-A modules: header ports in order, `parameter`s but no `localparam`.
    let va = cell("varactor_va.shprim");
    assert!(
        va.contains("pin p inout") && va.contains("pin n inout"),
        "{va}"
    );
    for p in [
        "param area \"\"",
        "param c0 \"\"",
        "param vbi \"\"",
        "param lvl \"\"",
    ] {
        assert!(va.contains(p), "{p}: {va}");
    }
    assert!(
        va.contains("format spice \"N{name} {p} {n} varactor_va"),
        "{va}"
    );
    assert!(
        va.contains("format spectre \"{name} ({p} {n}) varactor_va"),
        "{va}"
    );
    assert!(!out.contains("fake"), "a commented-out module: {out}");
    let two = cell("twoport.shprim");
    assert!(
        two.contains("{name} ({a} {b} {c}) twoport") && two.contains("param g \"\""),
        "{two}"
    );
    assert!(cell("nfet_LVT.shprim").contains("pin d inout"), "{out}");
    for p in ["param w \"\"", "param l \"\"", "param nf \"\""] {
        assert!(cell("nfet_LVT.shprim").contains(p), "{p}: {out}");
    }
    assert!(
        cell("rhigh.shprim").contains("param r \"\"")
            && cell("rhigh.shprim").contains("param l \"\""),
        "{out}"
    );
    assert!(
        cell("nfet_LVT.shprim").contains("format spectre \"{name} ({d} {g} {s} {b}) nfet_LVT"),
        "{out}"
    );
    for card in SPECTRE_CARDS {
        assert!(!out.contains(card), "{card} left the kit:\n{out}");
    }
    // The kit reads every include, an unselected corner's too: the ff corner's missing file is
    // reported, though nothing in that section can be a cell.
    assert_eq!(
        lines
            .iter()
            .filter(|l| l.starts_with("issue "))
            .collect::<Vec<_>>(),
        ["issue ff_missing.scs: include not found"],
        "{lines:?}"
    );
    // The `simulator lang=spice` region is read as SPICE now: nothing is left unread.
    assert!(
        !lines.iter().any(|l| l.contains("does not read")),
        "{lines:?}"
    );
    // Deterministic.
    assert_eq!(scan_spectre().0, out);
}

/// The remote script on the Spectre kit through a real `sh`: only structure crosses (no card
/// value), and `--stdin` on its frames, fetching includes round by round as PDK-Manager
/// does, gives the same cells and version as the direct scan.
#[test]
#[cfg_attr(
    windows,
    ignore = "the remote script runs in a Unix host's sh; Git Bash mangles verbatim Windows paths"
)]
fn the_remote_script_serves_a_spectre_kit() {
    let (script, _) = scan(&["--remote-script"], b"");
    let r = spectre_root();
    let run_sh = |paths: &[String]| -> Vec<u8> {
        use std::io::Write;
        let mut child = std::process::Command::new("sh")
            .args(["-s", "--"])
            .args(paths)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("sh");
        child
            .stdin
            .take()
            .unwrap()
            .write_all(script.as_bytes())
            .unwrap();
        let o = child.wait_with_output().unwrap();
        assert!(o.status.success());
        o.stdout
    };
    let entry = r.join("top.scs").display().to_string();
    let mut body = Vec::new();
    let mut want = vec![entry.clone()];
    let mut out = String::new();
    for _ in 0..16 {
        let framed = run_sh(&want);
        let text = String::from_utf8(framed.clone()).unwrap();
        for card in SPECTRE_CARDS {
            assert!(!text.contains(card), "{card} left the host:\n{text}");
        }
        body.extend_from_slice(
            text.strip_prefix("sfyframe 1\n")
                .unwrap()
                .strip_suffix("end\n")
                .unwrap()
                .as_bytes(),
        );
        let frames = [b"sfyframe 1\n".as_slice(), &body, b"end\n"].concat();
        let args = [
            "--stdin",
            "--root",
            r.to_str().unwrap(),
            "--entry",
            "top.scs",
            "--dialect",
            "spectre",
        ];
        out = scan(&args, &frames).0;
        want = out
            .lines()
            .filter_map(|l| l.strip_prefix("need "))
            .map(str::to_owned)
            .collect();
        if want.is_empty() {
            break;
        }
    }
    let (direct, _) = scan_spectre();
    assert_eq!(parse(&out).0, parse(&direct).0, "{out}");
    assert_eq!(out.lines().nth(1), direct.lines().nth(1), "same version");
}

/// The issue lines name what a kit does not declare (gaps2 P16, shaped like a commercial
/// kit's ESD part): an instance of a Verilog-A module the kit loads is declared (SpiceRack
/// binds none), any other undeclared master is named with its count; a parameter declared
/// twice is served once.
#[test]
fn undeclared_masters_are_named_and_verilog_a_ones_are_not_undeclared() {
    let r = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pdk_scan/spectre2");
    let (out, code) = scan(
        &[
            "--root",
            r.to_str().unwrap(),
            "--entry",
            "top.scs",
            "--dialect",
            "spectre",
        ],
        b"",
    );
    assert_eq!(code, 0, "{out}");
    let (files, lines) = parse(&out);
    let prot = &files.iter().find(|f| f.0 == "prot.shprim").unwrap().1;
    assert_eq!(prot.matches("param w \"\"").count(), 1, "{prot}");
    assert!(files.iter().any(|f| f.0 == "mon_va.shprim"), "{out}");
    let issues: Vec<&String> = lines.iter().filter(|l| l.starts_with("issue ")).collect();
    assert_eq!(
        issues,
        ["issue 2 instance(s) whose master the kit does not declare: parasitic_rc (2)"],
        "{out}"
    );
}
