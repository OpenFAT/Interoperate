//! A maestro test → its OCEAN script (spec `interop.md` §9.6): the analyses with their
//! form defaults, the design variables and the saved signals of the outputs.

use super::*;

/// Maestro analysis options that are ADE form fields (the sweep and step choosers) with no
/// OCEAN `analysis` keyword or Spectre parameter: OCEAN and Spectre infer them from the
/// keywords present.
pub(super) const FORM_ONLY: [&str; 4] = ["sweep", "incrType", "stepTypeLin", "stepTypeLog"];

/// `s` as a SKILL string literal (`\` and `"` escaped).
fn skill_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' | '\\' => {
                out.push('\\');
                out.push(ch);
            }
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Whether AEL text `v` is one number SKILL reads as such (`20n`, `-0.35`, `1e-6`).
fn skill_number(v: &str) -> bool {
    let body = v.strip_prefix('-').unwrap_or(v);
    ael::from_ael(body, |_| AelName::Unknown, |_| None)
        .and_then(|t| ael::number_to_ael(&t.text))
        .is_some()
}

/// The signals an OCEAN expression reads, `(kind, path)` with kind `'v` or `'i`, in first
/// appearance order without repeats: what the script `save`s.
fn signals_of(expr: &str, out: &mut Vec<(char, String)>) {
    const FNS: [&str; 8] = [
        "VT(\"", "VF(\"", "VS(\"", "VDC(\"", "IT(\"", "IF(\"", "IS(\"", "IDC(\"",
    ];
    let b = expr.as_bytes();
    for start in 0..b.len() {
        let rest = &expr[start..];
        let Some(f) = FNS.iter().find(|f| rest.starts_with(**f)) else {
            continue;
        };
        if start > 0 && (b[start - 1].is_ascii_alphanumeric() || b[start - 1] == b'_') {
            continue; // `myVT("…`: another function
        }
        let tail = &rest[f.len()..];
        let Some(end) = tail.find('"') else { continue };
        let row = (
            if f.starts_with('V') { 'v' } else { 'i' },
            tail[..end].to_owned(),
        );
        if !out.contains(&row) {
            out.push(row);
        }
    }
}

/// Evaluates `src` in cell `ci`'s own scope (a spec target over bench vars): identifiers
/// resolve to the cell's vars, then its parameter defaults, recursively (at most 16 levels).
/// A var with a D20 `cached` value stands for that value, as it does wherever the plugin is
/// not loaded; any other plugin call or anything that does not evaluate gives `None`.
pub(super) fn cell_value(d: &Design, ci: usize, src: &str) -> Option<f64> {
    fn go(c: &crate::design::Cell, src: &str, depth: u32) -> Option<f64> {
        if depth >= 16 {
            return None;
        }
        let p = Program::compile(src, Mode::Scalar).ok()?;
        if p.plugin_fns().len() > 0 {
            return None;
        }
        let lookup = |n: &str| -> Option<String> {
            c.records(RecordKind::Var)
                .find(|r| c.field(*r, 0) == n)
                .or_else(|| c.records(RecordKind::Param).find(|r| c.field(*r, 0) == n))
                .map(|r| c.field(r, 1).to_owned())
        };
        let cached = |n: &str| -> Option<f64> {
            let r = c.records(RecordKind::Var).find(|r| c.field(*r, 0) == n)?;
            c.attr(r, "cached")?
                .parse::<f64>()
                .ok()
                .filter(|x| x.is_finite())
        };
        let vals: Vec<Option<f64>> = p
            .idents()
            .map(|n| cached(n).or_else(|| lookup(n).and_then(|e| go(c, &e, depth + 1))))
            .collect();
        let mut inputs = Vec::new();
        let mut k = 0;
        p.bind(&mut inputs, |_| {
            let v = vals[k].map(crate::expr::Value::Num);
            k += 1;
            v
        })
        .ok()?;
        match p.eval(&inputs, &mut crate::expr::Scratch::default()).ok()? {
            crate::expr::Value::Num(x) => Some(x),
            crate::expr::Value::Int(i) => Some(i as f64),
            _ => None,
        }
    }
    assert!(ci < d.cells.len(), "cell {ci} of the design");
    go(&d.cells[ci], src, 0)
}

/// The OCEAN script of test `t` (§9.6): runnable headless (`ocean -nograph < <cell>.ocn`)
/// after `SfyImport` created the bench schematic. Corner `corner` (the
/// bench's default, else its first) supplies the model files, var overrides and
/// temperature; every swept var is a nested `paramAnalysis`; each output is one
/// `ocnPrint` table `<cell>/<output>.txt`, each plot one `<cell>/<y>_vs_<x>.txt` of
/// `waveVsWave`. A swept point that is not a number literal is reported (`sweep`) and the
/// sweep runs without it.
pub(super) fn ocean_script(
    t: &Test,
    cell: &str,
    swept: &[(String, String)],
    outs: &[(String, String)],
    plots: &[(String, String)],
    corner: Option<&str>,
    notes: &mut Notes,
) -> String {
    use std::fmt::Write as _;
    assert!(!t.cell.is_empty(), "a test names its bench cell");
    // Named after the bench cell: tests of different benches may share a name.
    let dir = &t.cell;
    let mut o = String::new();
    let _ = writeln!(
        o,
        "; {dir}.ocn: OCEAN script of bench {cell}, written by schemify-interop (interop.md §9.6).\n\
         ; Run after SfyImport of the same .sfyx, in a directory whose cds.lib\n\
         ; defines library {lib} (OCEAN finds libraries through ./cds.lib) and where {dir}/ may\n\
         ; be created; if SfyImport had ?lib, put that library in design() below:\n\
         ;   ocean -nograph < {dir}.ocn\n\
         ; Writes {dir}/psf (results) and {dir}/<output>.txt (one ocnPrint table per output).",
        lib = t.lib
    );
    let _ = writeln!(o, "createDir({})", skill_str(dir));
    let _ = writeln!(o, "simulator('spectre)");
    let _ = writeln!(
        o,
        "design({} {} {})",
        skill_str(&t.lib),
        skill_str(&t.cell),
        skill_str(&t.view)
    );
    let _ = writeln!(o, "resultsDir({})", skill_str(&format!("{dir}/psf")));
    let in_scope = |sc: &str| sc.is_empty() || Some(sc) == corner;
    let models: Vec<String> = t
        .models
        .iter()
        .filter(|m| in_scope(&m.0))
        .map(|m| format!("'({} {})", skill_str(&m.1), skill_str(&m.2)))
        .collect();
    if !models.is_empty() {
        let _ = writeln!(o, "modelFile({})", models.join(" "));
    }
    let mut defns = t.defns.clone();
    defns.sort();
    for (f, _) in &defns {
        let _ = writeln!(o, "definitionFile({})", skill_str(&format!("{dir}/{f}")));
    }
    // Design variables: globals, then the corner's overrides; a swept var starts at its
    // first point (paramAnalysis sets the rest).
    let mut vars: Vec<(String, String)> = Vec::new();
    for (n, v, sc) in &t.vars {
        if !in_scope(sc) {
            continue;
        }
        let v = match swept.iter().find(|s| &s.0 == n) {
            Some((_, list)) => top_level_split(list)[0].to_owned(),
            None => v.clone(),
        };
        match vars.iter_mut().find(|x| &x.0 == n) {
            Some(x) if !sc.is_empty() => x.1 = v,
            Some(_) => {}
            None => vars.push((n.clone(), v)),
        }
    }
    vars.sort();
    for (n, v) in &vars {
        if n == "temperature" {
            let _ = writeln!(o, "temp({v})");
        } else if skill_number(v) {
            let _ = writeln!(o, "desVar({} {v})", skill_str(n));
        } else {
            let _ = writeln!(o, "desVar({} {})", skill_str(n), skill_str(v));
        }
    }
    // ponytail: OCEAN's option() is not probed; the options ride in maestro only.
    if !t.sim_opts.is_empty() {
        let o: Vec<String> = t.sim_opts.iter().map(|(k, v)| format!("{k}={v}")).collect();
        notes.review(
            "code",
            cell,
            format!(
                "the OCEAN script sets no simulator options ({}): maestro has them",
                o.join(" ")
            ),
        );
    }
    let mut analyses = t.analyses.clone();
    analyses.sort_by(|a, b| a.0.cmp(&b.0));
    for (name, en, opts) in &analyses {
        if !en {
            continue;
        }
        let mut line = format!("analysis('{name}");
        for (k, v) in opts {
            if FORM_ONLY.contains(&k.as_str()) {
                continue;
            }
            if k == "saveOppoint" {
                line.push_str(" ?saveOppoint t");
            } else {
                let _ = write!(line, " ?{k} {}", skill_str(v));
            }
        }
        line.push(')');
        let _ = writeln!(o, "{line}");
    }
    let mut sigs = Vec::new();
    for (_, e) in outs {
        signals_of(e, &mut sigs);
    }
    for kind in ['v', 'i'] {
        let paths: Vec<String> = sigs
            .iter()
            .filter(|s| s.0 == kind)
            .map(|s| skill_str(&s.1))
            .collect();
        if !paths.is_empty() {
            let _ = writeln!(o, "save('{kind} {})", paths.join(" "));
        }
    }
    // Sweeps: one nested paramAnalysis per swept var, by var name.
    let mut sweeps: Vec<(String, Vec<String>)> = Vec::new();
    for (var, list) in swept {
        let pts: Vec<String> = top_level_split(list)
            .into_iter()
            .filter(|p| {
                let ok = skill_number(p);
                if !ok {
                    notes.review(
                        "sweep",
                        cell,
                        format!("sweep {var}: OCEAN point `{p}` is not a number; left out"),
                    );
                }
                ok
            })
            .map(str::to_owned)
            .collect();
        if !pts.is_empty() {
            sweeps.push((var.clone(), pts));
        }
    }
    sweeps.sort();
    if sweeps.is_empty() {
        let _ = writeln!(o, "run()");
    } else {
        let mut call = String::new();
        for (var, pts) in sweeps.iter().rev() {
            let inner = if call.is_empty() {
                String::new()
            } else {
                format!("\n  {call}")
            };
            call = format!(
                "paramAnalysis({} ?values '({}){inner})",
                skill_str(var),
                pts.join(" ")
            );
        }
        let _ = writeln!(o, "{call}");
        let _ = writeln!(o, "paramRun()");
    }
    for (name, e) in outs {
        let _ = writeln!(
            o,
            "ocnPrint(?output {} ?numberNotation 'scientific ?precision 8 {e})",
            skill_str(&format!("{dir}/{name}.txt"))
        );
    }
    for (y, x) in plots {
        let ye = &outs.iter().find(|q| &q.0 == y).expect("plots map").1;
        let xe = &outs.iter().find(|q| &q.0 == x).expect("plots map").1;
        let _ = writeln!(
            o,
            "ocnPrint(?output {} ?numberNotation 'scientific ?precision 8 waveVsWave(?x {xe} ?y {ye}))",
            skill_str(&format!("{dir}/{y}_vs_{x}.txt"))
        );
    }
    let _ = writeln!(o, "exit()");
    debug_assert!(o.ends_with("exit()\n") && o.contains("design("));
    o
}
