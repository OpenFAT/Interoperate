//! Maestro results → Schemify waves (`virtuoso::results`, `maestro-results`) on the real
//! probe captures (`tests/fixtures/cadence/results/`, local-only: each test skips without
//! them): the `.rdb` read exactly, the OCEAN export against Spectre's own nutbin of the
//! same netlist bit for bit, and the imported run the CLI writes.

mod common;
mod skill;

#[cfg(feature = "cli")]
use std::path::Path;

use interop::virtuoso::results::{self, rdb, wave};

fn fixture(rel: &str) -> Option<Vec<u8>> {
    common::cadence_fixture(&format!("results/{rel}")).map(|p| std::fs::read(p).unwrap())
}

/// One plot of a Spectre nutbin file: (complex, variable names, columns of (re, im)).
type NutPlot = (bool, Vec<String>, Vec<Vec<(f64, f64)>>);

/// Every plot of a Spectre nutbin file (big-endian doubles, every column complex when the
/// plot is), in file order.
fn nutbin(b: &[u8]) -> Vec<NutPlot> {
    let mut plots = Vec::new();
    let mut from = 0;
    while let Some(k) = b[from..].windows(8).position(|w| w == b"Binary:\n") {
        let at = from + k + 8;
        let head = String::from_utf8_lossy(&b[from..at]).into_owned();
        let field = |k: &str| {
            let l = head.lines().find(|l| l.starts_with(k)).unwrap();
            l[k.len()..].trim().to_owned()
        };
        let nv: usize = field("No. Variables:").parse().unwrap();
        let np: usize = field("No. Points:").parse().unwrap();
        let complex = field("Flags:") == "complex";
        let vars_at = head.find("Variables:\t").unwrap();
        let names: Vec<String> = head[vars_at + 10..]
            .lines()
            .take(nv)
            .map(|l| {
                l.split('\t')
                    .filter(|f| !f.is_empty())
                    .nth(1)
                    .unwrap()
                    .to_owned()
            })
            .collect();
        let w = if complex { 16 } else { 8 };
        let f = |k: usize| f64::from_be_bytes(b[k..k + 8].try_into().unwrap());
        let mut cols = vec![Vec::new(); nv];
        for p in 0..np {
            for (v, c) in cols.iter_mut().enumerate() {
                let k = at + (p * nv + v) * w;
                c.push((f(k), if complex { f(k + 8) } else { 0.0 }));
            }
        }
        plots.push((complex, names, cols));
        from = at + nv * np * w;
    }
    plots
}

/// One plot of an ngspice binary raw file: (complex, variable names, columns of re and im).
struct RawPlot {
    complex: bool,
    names: Vec<String>,
    re: Vec<Vec<f64>>,
    im: Vec<Vec<f64>>,
}

impl RawPlot {
    fn is_complex(&self) -> bool {
        self.complex
    }
    fn find_var(&self, name: &str) -> Option<usize> {
        self.names.iter().position(|n| n.eq_ignore_ascii_case(name))
    }
    fn col(&self, k: usize) -> &[f64] {
        &self.re[k]
    }
    fn col_im(&self, k: usize) -> &[f64] {
        &self.im[k]
    }
    fn len(&self) -> usize {
        self.re[0].len()
    }
}

/// Every plot of an ngspice binary raw file (little-endian doubles, a complex plot's every
/// column re/im pairs), as `wave::raw` writes it. Schemify's own reader is not in its
/// public SDK, so this one reads only that layout.
fn parse_raw(b: &[u8]) -> Result<Vec<RawPlot>, String> {
    let mut plots = Vec::new();
    let mut from = 0;
    while from < b.len() {
        let k = b[from..]
            .windows(8)
            .position(|w| w == b"Binary:\n")
            .ok_or("no Binary: line")?;
        let at = from + k + 8;
        let head = String::from_utf8_lossy(&b[from..at]).into_owned();
        let field = |k: &str| -> Result<String, String> {
            let l = head
                .lines()
                .find(|l| l.starts_with(k))
                .ok_or(k.to_owned())?;
            Ok(l[k.len()..].trim().to_owned())
        };
        let nv: usize = field("No. Variables:")?.parse().map_err(|_| "variables")?;
        let np: usize = field("No. Points:")?.parse().map_err(|_| "points")?;
        let complex = field("Flags:")? == "complex";
        let vars = head.find("Variables:\n").ok_or("no Variables:")? + 11;
        let names: Vec<String> = head[vars..]
            .lines()
            .take(nv)
            .map(|l| l.split('\t').nth(2).unwrap_or("").to_owned())
            .collect();
        let w = if complex { 16 } else { 8 };
        if b.len() < at + nv * np * w {
            return Err("short data".into());
        }
        let f = |k: usize| f64::from_le_bytes(b[k..k + 8].try_into().expect("8 bytes"));
        let (mut re, mut im) = (vec![Vec::new(); nv], vec![Vec::new(); nv]);
        for p in 0..np {
            for v in 0..nv {
                let k = at + (p * nv + v) * w;
                re[v].push(f(k));
                im[v].push(if complex { f(k + 8) } else { 0.0 });
            }
        }
        plots.push(RawPlot {
            complex,
            names,
            re,
            im,
        });
        from = at + nv * np * w;
    }
    Ok(plots)
}

/// The values of job `j`'s `key` lines in a results file (`  key a b` → `a b`).
#[cfg(feature = "cli")]
fn job_attr(text: &str, j: usize, key: &str) -> Vec<String> {
    let head = format!("job {j} ");
    text.lines()
        .skip_while(|l| !l.starts_with(&head))
        .skip(1)
        .take_while(|l| l.starts_with("  "))
        .filter_map(|l| l.trim().strip_prefix(&format!("{key} ")).map(str::to_owned))
        .collect()
}

/// Job `j`'s value of output `o`.
#[cfg(feature = "cli")]
fn job_value(text: &str, j: usize, o: &str) -> Option<f64> {
    job_attr(text, j, "value")
        .iter()
        .find_map(|v| v.strip_prefix(&format!("{o} "))?.parse().ok())
}

/// Requires plot `p` (Interop's raw) to hold `/out` and `/in` of nutbin plot `n` and its
/// x axis, bit for bit.
fn same_columns(p: &RawPlot, n: &NutPlot, what: &str) {
    let (complex, names, cols) = n;
    assert_eq!(p.is_complex(), *complex, "{what}");
    for (ours, net) in [
        (Some(0), None),
        (p.find_var("v(out)"), Some("out")),
        (p.find_var("v(in)"), Some("in")),
    ] {
        let ours = ours.unwrap_or_else(|| panic!("{what}: no {net:?}"));
        let theirs = net.map_or(0, |net| names.iter().position(|x| x == net).unwrap());
        let re = p.col(ours);
        assert_eq!(re.len(), cols[theirs].len(), "{what}");
        for (k, (x, &(r, i))) in re.iter().zip(&cols[theirs]).enumerate() {
            assert_eq!(
                x.to_bits(),
                r.to_bits(),
                "{what} {} point {k}",
                names[theirs]
            );
            if *complex && ours > 0 {
                assert_eq!(
                    p.col_im(ours)[k].to_bits(),
                    i.to_bits(),
                    "{what} im point {k}"
                );
            }
        }
    }
}

#[test]
fn the_rdb_reads_bit_for_bit() {
    let Some(b) = fixture("Interactive.0.rdb") else {
        return;
    };
    let db = rdb::open(&b).unwrap();
    let tests = db.table("test").unwrap();
    assert_eq!(tests[0][1], rdb::Val::Text("T1".into()));
    assert_eq!(
        tests[0][0],
        rdb::Val::Int(1),
        "INTEGER PRIMARY KEY is the rowid"
    );
    let rv = db.table("resultValue").unwrap();
    assert_eq!(rv.len(), 14);
    // (pointID 2, resultID 1) is vmax at the hot point: Python's sqlite3 repr of the value.
    let vmax_hot = rv
        .iter()
        .find(|r| r[0] == rdb::Val::Int(2) && r[1] == rdb::Val::Int(1))
        .unwrap();
    assert_eq!(vmax_hot[2], rdb::Val::Real(1.0927514864610322));
    // Every table of the schema reads.
    for t in [
        "point",
        "corner",
        "parameterValue",
        "specValue",
        "testStatus",
        "error",
        "psfTable",
    ] {
        db.table(t).unwrap_or_else(|e| panic!("{t}: {e}"));
    }
}

#[test]
fn a_history_converts_to_jobs_with_exact_values_and_verdicts() {
    let Some(b) = fixture("Interactive.0.rdb") else {
        return;
    };
    let dump = results::Dump {
        view: ("sfyprobe_lib".into(), "rc".into(), "maestro".into()),
        name: "Interactive.0".into(),
        rdb: "Interactive.0.rdb".into(),
        ..results::Dump::default()
    };
    let h = results::convert(&dump, &b).unwrap();
    assert_eq!(h.test, "T1");
    assert_eq!(h.notes[0], "test T2: only the first test of a history maps");
    assert_eq!(h.jobs.len(), 2);
    let (nom, hot) = (&h.jobs[0], &h.jobs[1]);
    assert_eq!((nom.corner.as_deref(), nom.point), (None, 1));
    assert_eq!((hot.corner.as_deref(), hot.point), (Some("hot"), 1));
    assert!(nom.sets.is_empty(), "nothing is swept across design points");
    assert_eq!(nom.status, "done");
    assert_eq!(nom.values["vmax"], 0.9934104423801275);
    assert_eq!(hot.values["vmax"], 1.0927514864610322);
    assert_eq!(hot.values["vavg"], 0.5497347733415893);
    assert_eq!(nom.vars["C"], 1e-9);
    assert_eq!(nom.vars["K"], 7.0);
    assert!(!nom.vars.contains_key("rmc"), "an expression has no number");
    assert!(!nom.values.contains_key("out"), "a net output has no value");
    assert_eq!(
        (nom.pass["vmax"], nom.pass["vmin"], nom.pass["vavg"]),
        (true, false, true)
    );
    assert!(!hot.pass["vavg"], "near is outside the spec");
    assert!(h.notes.iter().any(|n| n.starts_with("1 verdict(s) `near`")));
}

/// The OCEAN export of hot point 1 against Spectre's nutbin of that point's netlist: the
/// raw file Interop writes holds the very same doubles (G35).
fn exact(ocean: &str, analysis: &str, oracle: &str, complex: bool) {
    let (Some(text), Some(nut)) = (fixture(ocean), fixture(oracle)) else {
        return;
    };
    let text = format!(
        "# analysis {analysis}\n{}",
        String::from_utf8(text).unwrap()
    );
    let a = wave::parse(&text).unwrap();
    let raw = wave::raw(&a);
    let plots = parse_raw(&raw).unwrap();
    assert_eq!(plots.len(), 1, "/out and /in share their x values");
    let p = &plots[0];
    assert_eq!(p.is_complex(), complex);
    let n = nutbin(&nut);
    assert_eq!(n.len(), 1);
    same_columns(p, &n[0], oracle);
}

#[test]
fn the_ocean_tran_export_equals_spectres_nutbin_bit_for_bit() {
    exact(
        "ocean_psfxl_tran.txt",
        "tran",
        "direct_T1_hot.nutbin",
        false,
    );
}

#[test]
fn the_ocean_ac_export_equals_spectres_nutbin_bit_for_bit() {
    exact("ocean_psfxl_ac.txt", "ac", "direct_T2_hot.nutbin", true);
}

#[cfg(feature = "cli")]
const BENCH_EXTRA: &str = "\noutput vmax \"ymax(v(out))\"\noutput vavg \"avg(v(out))\"\n\nspec vmax\n  min 0.5\n\ncorner hot\n  set R 1k\n";

#[cfg(feature = "cli")]
/// A project holding the RC bench of the live kit with the history's output names, and a
/// results dump of `Interactive.0` whose hot point carries the OCEAN tran export.
fn project(dir: &Path, history: &str) -> Option<()> {
    let (rdb_bytes, tran) = (
        fixture("Interactive.0.rdb")?,
        fixture("ocean_psfxl_tran.txt")?,
    );
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir.join("dump")).unwrap();
    let live = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/live/rclive");
    for f in ["schemify.project", "rcfilter.shcomp"] {
        std::fs::copy(live.join(f), dir.join(f)).unwrap();
    }
    let bench = std::fs::read_to_string(live.join("rclive_tb.shbench")).unwrap();
    std::fs::write(dir.join("rc_tb.shbench"), bench + BENCH_EXTRA).unwrap();
    std::fs::write(dir.join("dump/Interactive.0.rdb"), rdb_bytes).unwrap();
    let mut w = b"# analysis tran\n".to_vec();
    w.extend(tran);
    std::fs::write(dir.join("dump/hot.1.txt"), w).unwrap();
    std::fs::write(
        dir.join("dump/h.sfyx"),
        format!(
            "(\"sfyx\" 1 \"test\")\n(\"history\" \"sfyprobe_lib\" \"rc\" \"maestro\" \"{history}\" \"Interactive.0.rdb\" \"\")\n(\"job\" \"T1\" \"\" 1 \"/sim/1/T1/psf\" \"\")\n(\"job\" \"T1\" \"hot\" 1 \"/sim/2/T1/psf\" \"hot.1.txt\")\n(\"end\")\n"
        ),
    )
    .unwrap();
    Some(())
}

#[cfg(feature = "cli")]
#[test]
#[ignore = "maestro-results needs the design hash, not in Schemify's public interface yet"]
fn maestro_results_writes_one_imported_run_per_history() {
    let dir = std::env::temp_dir().join(format!("sfy-mres-{}", std::process::id()));
    if project(&dir, "Interactive.0").is_none() {
        return;
    }
    let args = |h: &str| -> Vec<String> {
        [dir.to_str().unwrap(), &format!("dump/{h}"), "rc_tb.shbench"]
            .map(String::from)
            .to_vec()
    };
    let mut cmd = vec!["maestro-results".to_owned()];
    cmd.extend(args("h.sfyx"));
    assert_eq!(interop::cli::run(&cmd), 0);
    let runs: Vec<_> = std::fs::read_dir(dir.join(".schemify/runs"))
        .unwrap()
        .flatten()
        .collect();
    assert_eq!(runs.len(), 1);
    let run = runs[0].path();
    let name = run.file_name().unwrap().to_str().unwrap().to_owned();
    assert!(name.starts_with('v') && name.len() == 17, "{name}");
    let text = std::fs::read_to_string(run.join("results")).unwrap();
    assert!(
        text.contains("  source virtuoso sfyprobe_lib/rc/maestro Interactive.0\n"),
        "{text}"
    );
    assert!(text.contains("\njob 1 done\n  corner hot\n"), "{text}");
    assert!(text.contains("  value vmax 1.0927514864610322\n"), "{text}");
    assert!(text.contains("  pass vmax pass\n"), "{text}");
    // Values bit for bit (Schemify's results reader is not in its public SDK).
    assert_eq!(text.lines().filter(|l| l.starts_with("job ")).count(), 2);
    assert_eq!(job_attr(&text, 1, "corner"), ["hot"]);
    assert_eq!(job_value(&text, 0, "vmax"), Some(0.9934104423801275));
    assert_eq!(
        std::fs::read_to_string(run.join("bench")).unwrap(),
        "rc_tb\n"
    );
    assert!(!run.join("0/out.raw").exists(), "Nominal had no export");
    let plots = parse_raw(&std::fs::read(run.join("1/out.raw")).unwrap()).unwrap();
    assert_eq!(plots[0].len(), 101);
    // Deterministic, and another history of the same design is refused, not overwritten.
    assert_eq!(interop::cli::run(&cmd), 0);
    assert_eq!(std::fs::read_to_string(run.join("results")).unwrap(), text);
    project(&dir, "Interactive.9").unwrap();
    std::fs::create_dir_all(&run).unwrap();
    std::fs::write(run.join("results"), &text).unwrap();
    assert_eq!(interop::cli::run(&cmd), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_dumper_names_points_by_their_directory_and_prints_waves_exactly() {
    use skill::mockdb::{mae, results as mock};
    let mut i = skill::virtuoso(&["lib"]);
    i.host.mae.saved.insert(
        ("lib".into(), "rc".into()),
        mae::Setup {
            tests: vec![mae::Test {
                name: "T1".into(),
                ..mae::Test::default()
            }],
            corners: vec![mae::Corner {
                name: "hot".into(),
                enabled: true,
                ..mae::Corner::default()
            }],
            ..mae::Setup::default()
        },
    );
    // Point directories numbered unlike their design points (dir 2 is hot point 1).
    let tran = |k: f64| {
        let t = vec![0.0, 1e-7, 0.1 + 0.2];
        vec![
            (
                "/out".to_owned(),
                false,
                t.clone(),
                t.iter().map(|x| (x * k, None)).collect(),
            ),
            (
                "/V0/PLUS".to_owned(),
                true,
                t.clone(),
                t.iter().map(|x| (-x, None)).collect(),
            ),
        ]
    };
    let ac = vec![(
        "/out".to_owned(),
        false,
        vec![1.0, 10.0],
        vec![(1.0, Some(-1e-17)), (0.5, Some(-0.5))],
    )];
    let dir = |k: u32| format!("/sim/Interactive.0/{k}/T1/psf");
    mock::set(mock::History {
        name: "Interactive.0".into(),
        rdb: "/v/lib/rc/maestro/results/maestro/Interactive.0.rdb".into(),
        points: [
            (("T1".into(), String::new(), 1), dir(1)),
            (("T1".into(), String::new(), 2), dir(3)),
            (("T1".into(), "hot".into(), 1), dir(2)),
        ]
        .into(),
        dirs: [
            (
                dir(1),
                vec![("tran".into(), tran(1.0)), ("ac".into(), ac.clone())],
            ),
            (dir(2), vec![("tran".into(), tran(2.0))]),
            (dir(3), vec![("tran".into(), tran(3.0))]),
        ]
        .into(),
    });
    let ok = i
        .load(r#"(SfyAdeResults "/t" "lib" "rc" "Interactive.0")"#)
        .unwrap();
    let log = i.files["/t/Interactive.0.sfyx.log"].clone();
    assert!(ok.truthy(), "{log}");
    let dump = &i.files["/t/Interactive.0.sfyx"];
    assert_eq!(
        dump,
        "(\"sfyx\" 1 \"schemify.il 1\")\n(\"history\" \"lib\" \"rc\" \"maestro\" \"Interactive.0\" \"Interactive.0.rdb\" \"\")\n\
         (\"job\" \"T1\" \"\" 1 \"/sim/Interactive.0/1/T1/psf\" \"Interactive.0.1.txt\")\n\
         (\"job\" \"T1\" \"\" 2 \"/sim/Interactive.0/3/T1/psf\" \"Interactive.0.2.txt\")\n\
         (\"job\" \"T1\" \"hot\" 1 \"/sim/Interactive.0/2/T1/psf\" \"Interactive.0.3.txt\")\n(\"end\")\n"
    );
    let d = results::parse(dump).unwrap();
    assert_eq!(
        d.points[&("T1".into(), "hot".into(), 1)].1,
        "Interactive.0.3.txt"
    );
    assert_eq!(
        mock::commands(),
        ["cp '/v/lib/rc/maestro/results/maestro/Interactive.0.rdb' '/t/Interactive.0.rdb'"]
    );
    // Nominal point 1: tran (a voltage and a terminal current) and ac (complex), %.17g.
    let a = wave::parse(&i.files["/t/Interactive.0.1.txt"]).unwrap();
    assert_eq!(
        a.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(),
        ["ac", "tran"]
    );
    assert_eq!(a[0].signals[0].im, [-1e-17, -0.5]);
    let tr = &a[1].signals;
    assert_eq!((tr[0].name.as_str(), tr[0].current), ("/V0/PLUS", true));
    assert_eq!((tr[1].name.as_str(), tr[1].current), ("/out", false));
    assert_eq!(tr[1].re[2].to_bits(), (0.1f64 + 0.2).to_bits());
    let hot = wave::parse(&i.files["/t/Interactive.0.3.txt"]).unwrap();
    assert_eq!(hot[0].signals[1].re[1], 2e-7);
}

/// The live check's history (`live/checks/l4_results.il`, fetched from
/// `$SFYCHECK_WORK/out/l4` into `$SFY_L4_LIVE`): imported into a project whose bench sweeps
/// C over 1n, 2n with corner hot, every point's `out.raw` equals Spectre's nutbin of that
/// point's netlist bit for bit, and vmax/vmin, from the `.rdb`, are exactly the max/min of
/// the nutbin's `out`.
#[cfg(feature = "cli")]
#[test]
#[ignore = "needs the live check's output: SFY_L4_LIVE=<dir> (live/checks/run.sh l4_results), \
            and maestro-results the design hash (not in Schemify's public interface yet)"]
fn live_history_against_nutbin() {
    let live = std::path::PathBuf::from(std::env::var("SFY_L4_LIVE").expect("SFY_L4_LIVE"));
    let mut names: Vec<_> = std::fs::read_dir(&live)
        .unwrap()
        .flatten()
        // Not Spectre's <stem>.nutbin.psf directories, its info files beside each nutbin.
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .map(|e| e.file_name().into_string().unwrap())
        .collect();
    names.sort();
    let dump = names
        .iter()
        .find(|n| n.ends_with(".sfyx"))
        .expect("a dump")
        .clone();
    let dir = std::env::temp_dir().join(format!("sfy-mres-live-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("dump")).unwrap();
    for n in &names {
        std::fs::copy(live.join(n), dir.join("dump").join(n)).unwrap();
    }
    let rc = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/live/rclive");
    for f in ["schemify.project", "rcfilter.shcomp"] {
        std::fs::copy(rc.join(f), dir.join(f)).unwrap();
    }
    let bench = std::fs::read_to_string(rc.join("rclive_tb.shbench")).unwrap();
    std::fs::write(
        dir.join("rc_tb.shbench"),
        bench + "\noutput vmax \"ymax(v(out))\"\noutput vmin \"ymin(v(out))\"\n\nspec vmax\n  min 0.5\n\ncorner hot\n  set R 1k\n\nsweep C\n  var C\n  values 1n 2n\n",
    )
    .unwrap();
    let cmd: Vec<String> = [
        "maestro-results",
        dir.to_str().unwrap(),
        &format!("dump/{dump}"),
        "rc_tb.shbench",
    ]
    .map(String::from)
    .to_vec();
    assert_eq!(interop::cli::run(&cmd), 0);
    let run = std::fs::read_dir(dir.join(".schemify/runs"))
        .unwrap()
        .flatten()
        .next()
        .unwrap()
        .path();
    let text = std::fs::read_to_string(run.join("results")).unwrap();
    assert_eq!(
        text.lines().filter(|l| l.starts_with("job ")).count(),
        4,
        "{text}"
    );
    let d = results::parse(&std::fs::read_to_string(live.join(&dump)).unwrap()).unwrap();
    let h = results::convert(&d, &std::fs::read(live.join(&d.rdb)).unwrap()).unwrap();
    for (j, job) in h.jobs.iter().enumerate() {
        let what = format!("job {j} {:?} point {}", job.corner, job.point);
        assert_eq!(
            job_attr(&text, j, "corner").first().map(String::as_str),
            job.corner.as_deref(),
            "{what}"
        );
        assert_eq!(
            job_attr(&text, j, "set"),
            [format!("C {}", ["1n", "2n"][job.point as usize - 1])],
            "{what}"
        );
        assert_eq!(job.status, "done", "{what}");
        let stem = job.waves.strip_suffix(".txt").expect("exported");
        let nut = nutbin(&std::fs::read(live.join(format!("{stem}.nutbin"))).unwrap());
        let raw =
            parse_raw(&std::fs::read(run.join(j.to_string()).join("out.raw")).unwrap()).unwrap();
        let tran = raw.iter().find(|p| !p.is_complex()).expect("a tran plot");
        let ac = raw.iter().find(|p| p.is_complex()).expect("an ac plot");
        let ntran = nut.iter().find(|p| !p.0).expect("a tran nutbin plot");
        let nac = nut.iter().find(|p| p.0).expect("an ac nutbin plot");
        same_columns(tran, ntran, &format!("{what} tran"));
        same_columns(ac, nac, &format!("{what} ac"));
        let o = ntran.1.iter().position(|x| x == "out").unwrap();
        let col = ntran.2[o].iter().map(|c| c.0);
        let (max, min) = col.fold((f64::NEG_INFINITY, f64::INFINITY), |(a, b), x| {
            (a.max(x), b.min(x))
        });
        assert_eq!(
            job_value(&text, j, "vmax").map(f64::to_bits),
            Some(max.to_bits()),
            "{what} vmax"
        );
        assert_eq!(
            job_value(&text, j, "vmin").map(f64::to_bits),
            Some(min.to_bits()),
            "{what} vmin"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}
