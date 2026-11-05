//! A maestro point's waveforms as `SfyAdeResults` exports them (OCEAN `drGetElem` printed
//! `%.17g`, the only exact way out of psfxl; spec `interop.md` §13 G35) → an ngspice
//! binary raw file (Schemify waves.md §9, run.md §4: a job's `out.raw`).
//!
//! The text, per point:
//!
//! ```text
//! # analysis tran
//! # /out 101 points V
//! 0 0
//! 1.9999999999999999e-07 0.19908934923846852
//! # analysis ac
//! # /out 46 points V
//! 1 0.99999999996052158 -6.2831853069315364e-06
//! ```
//!
//! One `# analysis <name>` line per OCEAN result, then per waveform `# <signal> <n> points
//! <V|I>` (the kind word absent means `V`) and `n` lines `x y` (real) or `x re im` (complex).
//! `%.17g` text parses back to the same double, so the raw holds the simulator's bits.
//! A `# skipped <result> <output>` line names a result value that is no waveform (a dcOp
//! value, a noise summary): [`skipped`] lists them for the report.

/// One waveform of one analysis.
#[derive(Clone, Debug, PartialEq)]
pub struct Signal {
    /// The OCEAN name (`/out`, `/V0/PLUS`).
    pub name: String,
    /// A current (`i(…)`), else a voltage.
    pub current: bool,
    /// The x values.
    pub x: Vec<f64>,
    /// The real parts.
    pub re: Vec<f64>,
    /// The imaginary parts; empty for a real waveform.
    pub im: Vec<f64>,
}

/// One OCEAN result (`tran`, `ac`, …) and its waveforms in file order.
#[derive(Clone, Debug, PartialEq)]
pub struct Analysis {
    /// The result name.
    pub name: String,
    /// Its waveforms.
    pub signals: Vec<Signal>,
}

/// Points per waveform (run.md's bound on a job: a raw file is at most 2^32 points).
pub const POINTS_MAX: usize = 1 << 26;

/// Parses an export.
///
/// # Errors
///
/// The first malformed line, with its 1-based number: a waveform before any analysis, a
/// header that is not `# <name> <n> points [V|I]`, a value that is not a number, a line
/// with another column count than the waveform's first, fewer lines than announced.
pub fn parse(text: &str) -> Result<Vec<Analysis>, String> {
    let mut out: Vec<Analysis> = Vec::new();
    let mut lines = text.lines().enumerate();
    while let Some((k, line)) = lines.next() {
        let bad = |why: &str| format!("line {}: {why}: `{line}`", k + 1);
        if line.trim().is_empty() {
            continue;
        }
        let Some(head) = line.strip_prefix("# ") else {
            return Err(bad("a value outside a waveform"));
        };
        let w: Vec<&str> = head.split_whitespace().collect();
        if let ["skipped", _, _] = w[..] {
            continue;
        }
        if let ["analysis", name] = w[..] {
            out.push(Analysis {
                name: name.to_owned(),
                signals: Vec::new(),
            });
            continue;
        }
        let (name, n, current) = match w[..] {
            [name, n, "points"] => (name, n, false),
            [name, n, "points", kind @ ("V" | "I")] => (name, n, kind == "I"),
            _ => return Err(bad("not `# <signal> <n> points [V|I]`")),
        };
        let n: usize = n.parse().map_err(|_| bad("a point count"))?;
        if n == 0 || n > POINTS_MAX {
            return Err(bad("a point count outside 1..2^26"));
        }
        let a = out
            .last_mut()
            .ok_or_else(|| bad("a waveform before any `# analysis`"))?;
        let mut s = Signal {
            name: name.to_owned(),
            current,
            x: Vec::with_capacity(n),
            re: Vec::with_capacity(n),
            im: Vec::new(),
        };
        for _ in 0..n {
            let Some((k, l)) = lines.next() else {
                return Err(format!("{name}: {n} points announced, fewer given"));
            };
            let bad = |why: &str| format!("line {}: {why}: `{l}`", k + 1);
            let v = l
                .split_whitespace()
                .map(str::parse::<f64>)
                .collect::<Result<Vec<f64>, _>>()
                .map_err(|_| bad("not a number"))?;
            let complex = (s.x.is_empty() && v.len() == 3) || !s.im.is_empty();
            match (v.len(), complex) {
                (2, false) => {}
                (3, true) => s.im.push(v[2]),
                _ => return Err(bad("another column count than the waveform's first line")),
            }
            s.x.push(v[0]);
            s.re.push(v[1]);
        }
        a.signals.push(s);
    }
    Ok(out)
}

/// The `<result>/<output>` of each `# skipped` line of an export, in file order.
pub fn skipped(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| {
            match l
                .strip_prefix("# skipped ")?
                .split_whitespace()
                .collect::<Vec<_>>()[..]
            {
                [r, o] => Some(format!("{r}/{o}")),
                _ => None,
            }
        })
        .collect()
}

/// Schemify's name of OCEAN signal `s`, by the rule outputs use (`interop.md` §9.2): a
/// voltage `/a/b` is `v(a.b)`, a terminal current `/V0/PLUS` (or `/V0/p`) is `i(V0)`;
/// `<i>` bus bits become `[i]`.
pub fn raw_name(s: &Signal) -> String {
    let p = s.name.strip_prefix('/').unwrap_or(&s.name);
    let p = if s.current {
        p.strip_suffix("/PLUS")
            .or_else(|| p.strip_suffix("/p"))
            .unwrap_or(p)
    } else {
        p
    };
    let n = p.replace('/', ".").replace('<', "[").replace('>', "]");
    format!("{}({n})", if s.current { 'i' } else { 'v' })
}

/// The ngspice binary raw file of `analyses`: one plot per analysis and x axis (waveforms
/// of one analysis whose x values are bit-identical share a plot, in file order), complex
/// when the analysis' waveforms are. Nothing is interpolated: a waveform on its own time
/// points gets its own plot.
pub fn raw(analyses: &[Analysis]) -> Vec<u8> {
    let mut out = Vec::new();
    for a in analyses {
        let mut groups: Vec<Vec<&Signal>> = Vec::new();
        for s in &a.signals {
            let same = |g: &&mut Vec<&Signal>| {
                let x = &g[0].x;
                x.len() == s.x.len()
                    && x.iter().zip(&s.x).all(|(p, q)| p.to_bits() == q.to_bits())
                    && g[0].im.is_empty() == s.im.is_empty()
            };
            match groups.iter_mut().find(same) {
                Some(g) => g.push(s),
                None => groups.push(vec![s]),
            }
        }
        for g in &groups {
            plot(&mut out, &a.name, g);
        }
    }
    out
}

fn plot(out: &mut Vec<u8>, analysis: &str, g: &[&Signal]) {
    let complex = !g[0].im.is_empty();
    // OCEAN may name a result `tran-tran` (its logFile entry): the analysis is the first word.
    let (title, scale, kind) = match analysis.split('-').next().unwrap_or("") {
        "tran" => ("Transient Analysis", "time", "time"),
        "ac" | "noise" | "xf" | "sp" | "stb" => ("AC Analysis", "frequency", "frequency"),
        "dc" => ("DC transfer characteristic", "sweep", "voltage"),
        // The operating point (`dcOp`, one point per node voltage and terminal current):
        // the plot run.md §4 reads `op` data and the OP annotation from.
        "dcOp" => ("Operating Point", "sweep", "notype"),
        _ => ("", "sweep", "notype"),
    };
    let title = if title.is_empty() {
        format!("{analysis} Analysis")
    } else {
        title.to_owned()
    };
    let n = g[0].x.len();
    let mut h = format!(
        "Title: maestro {analysis}\nDate: \nPlotname: {title}\nFlags: {}\nNo. Variables: {}\nNo. Points: {n}\nVariables:\n\t0\t{scale}\t{kind}\n",
        if complex { "complex" } else { "real" },
        g.len() + 1
    );
    for (k, s) in g.iter().enumerate() {
        let ty = if s.current { "current" } else { "voltage" };
        h.push_str(&format!("\t{}\t{}\t{ty}\n", k + 1, raw_name(s)));
    }
    h.push_str("Binary:\n");
    out.extend_from_slice(h.as_bytes());
    for p in 0..n {
        out.extend_from_slice(&g[0].x[p].to_le_bytes());
        if complex {
            out.extend_from_slice(&0f64.to_le_bytes());
        }
        for s in g {
            out.extend_from_slice(&s.re[p].to_le_bytes());
            if complex {
                out.extend_from_slice(&s.im[p].to_le_bytes());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_export_parses_and_names_map() {
        let t = "# analysis ac\n# /out 2 points\n1 0.5 -0.25\n10 0.125 -0.5\n# /V0/PLUS 2 points I\n1 1e-3 0\n10 2e-3 0\n";
        let a = parse(t).unwrap();
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].signals[0].im, [-0.25, -0.5]);
        assert_eq!(raw_name(&a[0].signals[0]), "v(out)");
        assert_eq!(raw_name(&a[0].signals[1]), "i(V0)");
        for bad in [
            "# /out 1 points\n1 2\n",
            "# analysis tran\n# /out 2 points\n1 2\n",
            "# analysis tran\n# /out 1 points\n1 x\n",
            "# analysis tran\n# /out 2 points\n1 2\n3 4 5\n",
            "# skipped dcOp\n",
            "1 2\n",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
        let t = "# analysis dcOp\n# skipped dcOp /out\n# analysis tran\n# /out 1 points\n0 1\n";
        assert_eq!(parse(t).unwrap()[1].signals.len(), 1);
        assert_eq!(skipped(t), ["dcOp//out"]);
        // The operating point (live a_res: v("/out") 0.5) is ngspice's "Operating Point" plot.
        let op = parse("# analysis dcOp\n# /out 1 points V\n0 0.5\n").unwrap();
        let r = String::from_utf8_lossy(&raw(&op)).into_owned();
        assert!(r.contains("Plotname: Operating Point\n"), "{r}");
    }

    #[test]
    fn waveforms_on_other_time_points_get_their_own_plot() {
        let t = "# analysis tran\n# /a 2 points\n0 1\n1 2\n# /b 2 points\n0 3\n1 4\n# /c 3 points\n0 5\n0.5 6\n1 7\n";
        let raw = raw(&parse(t).unwrap());
        let text = String::from_utf8_lossy(&raw);
        assert_eq!(text.matches("Plotname: Transient Analysis").count(), 2);
        assert!(text.contains("No. Variables: 3\nNo. Points: 2"));
        assert!(text.contains("No. Variables: 2\nNo. Points: 3"));
    }

    /// A live point (a_res: dc op, noise, stb, pss, pac) parses; its operating point is a
    /// plot of its own; what has no waveform is listed.
    #[test]
    fn a_live_point_of_every_kind_parses() {
        let t = include_str!("../../../tests/fixtures/virtuoso/a_res_point.txt");
        let a = parse(t).unwrap();
        let op = a.iter().find(|x| x.name == "dcOp").expect("dcOp");
        let out = op.signals.iter().find(|s| s.name == "/out").expect("/out");
        assert_eq!(out.re, [0.5], "the divider: ground is ground");
        for k in ["stb", "pss_td", "pss_fd", "noise"] {
            assert!(
                a.iter().any(|x| x.name == k && !x.signals.is_empty()),
                "{k}"
            );
        }
        let sk = skipped(t);
        assert!(
            sk.iter().any(|x| x.starts_with("pac/"))
                && sk.iter().any(|x| x.starts_with("dcOpInfo/"))
        );
        assert!(String::from_utf8_lossy(&raw(&a)).contains("Plotname: Operating Point"));
    }
}
