//! One maestro test's records → bench records ([`import`], spec `interop.md` §9.2): the
//! records parsed and checked ([`check`], the reader's boundary), then vars and sweeps,
//! corners, the analysis block (SPICE when every analysis has a SPICE form, else Spectre),
//! outputs and specs, and definition files.

use super::*;

/// A Spectre word for `v`: quoted when empty or holding white space, `"`, `\` or `=`.
pub(super) fn spectre_word(v: &str) -> String {
    if !v.is_empty()
        && !v
            .chars()
            .any(|c| c.is_whitespace() || matches!(c, '"' | '\\' | '='))
    {
        return v.to_owned();
    }
    let mut out = String::from("\"");
    for ch in v.chars() {
        if matches!(ch, '"' | '\\') {
            out.push('\\');
        }
        out.push(ch);
    }
    out.push('"');
    out
}

/// `v` as a quoted Spectre string, `"` and `\` escaped.
fn spectre_quoted(v: &str) -> String {
    let w = spectre_word(v);
    if w.starts_with('"') {
        w
    } else {
        format!("\"{w}\"")
    }
}

fn flag(r: &Record, k: usize) -> Result<bool, Error> {
    match r.int(k)? {
        0 => Ok(false),
        1 => Ok(true),
        v => Err(r.err(format!("enable {v} (0 or 1)"))),
    }
}

/// Parses one maestro test (its `maestro` record first): every record's arity and element
/// types, `enable` 0/1, options as `(key value)` strings, a `var`/`model` scope naming a
/// corner declared before it, a `spec` naming an output before it with op `lt`/`gt`, output
/// kinds `point`/`net`, no name twice per kind, at most [`ROWS_MAX`] rows per kind.
pub(super) fn parse(records: &[Record]) -> Result<Test, Error> {
    let m = records.first().ok_or(Error {
        line: 0,
        msg: "maestro: an empty test".into(),
    })?;
    if m.tag != "maestro" {
        return Err(m.err("a test starts with its maestro record"));
    }
    m.arity(5)?;
    let mut t = Test {
        name: m.str(0)?.to_owned(),
        lib: m.str(1)?.to_owned(),
        cell: m.str(2)?.to_owned(),
        view: m.str(3)?.to_owned(),
        sim: m.str(4)?.to_owned(),
        ..Test::default()
    };
    if t.name.is_empty() || t.cell.is_empty() {
        return Err(m.err("empty test or cell name"));
    }
    for r in &records[1..] {
        let scope_ok = |t: &Test, sc: &str| sc.is_empty() || t.corners.iter().any(|c| c.0 == sc);
        match r.tag.as_str() {
            "var" => {
                r.arity(3)?;
                let (n, v, sc) = (r.str(0)?, r.str(1)?, r.str(2)?);
                if n.is_empty() || !scope_ok(&t, sc) {
                    return Err(r.err(format!(
                        "var `{n}` in scope `{sc}`: no such corner before it"
                    )));
                }
                if t.vars.iter().any(|x| x.0 == n && x.2 == sc) {
                    return Err(r.err(format!("var {n} twice in scope `{sc}`")));
                }
                t.vars.push((n.into(), v.into(), sc.into()));
            }
            "analysis" => {
                r.arity(3)?;
                let n = r.str(0)?;
                let en = flag(r, 1)?;
                let mut opts = Vec::new();
                for o in r.list(2)? {
                    match o.items() {
                        [Value::Str(k), Value::Str(v)] => opts.push((k.clone(), v.clone())),
                        _ => {
                            return Err(r.err(format!(
                                "analysis {n}: an option is not (key value) strings"
                            )));
                        }
                    }
                }
                if n.is_empty() || t.analyses.iter().any(|a| a.0 == n) {
                    return Err(r.err(format!("analysis `{n}` empty or twice")));
                }
                // maeGetAnalysis answers form fields nothing set: `anaName` (the analysis
                // name) and the ac/noise sweep chooser `sweep "Frequency"`. They carry no
                // meaning, so a dump of a setup we wrote reads as what we wrote.
                opts.retain(|(k, v)| {
                    !((k == "anaName" && v == n)
                        || (k == "sweep" && v == "Frequency" && matches!(n, "ac" | "noise"))
                        || is_form_default(n, k, v))
                });
                t.analyses.push((n.into(), en, opts));
            }
            "corner" => {
                r.arity(2)?;
                let n = r.str(0)?;
                let en = flag(r, 1)?;
                if n.is_empty() || t.corners.iter().any(|c| c.0 == n) {
                    return Err(r.err(format!("corner `{n}` empty or twice")));
                }
                t.corners.push((n.into(), en));
            }
            "model" => {
                r.arity(3)?;
                let (sc, f, sec) = (r.str(0)?, r.str(1)?, r.str(2)?);
                if f.is_empty() || !scope_ok(&t, sc) {
                    return Err(r.err(format!(
                        "model `{f}` of `{sc}`: empty file or no such corner before it"
                    )));
                }
                t.models.push((sc.into(), f.into(), sec.into()));
            }
            "output" => {
                r.arity(3)?;
                let (n, k, e) = (r.str(0)?, r.str(1)?, r.str(2)?);
                if !matches!(k, "point" | "net") {
                    return Err(r.err(format!("output {n}: kind `{k}` (point or net)")));
                }
                if n.is_empty() || t.outputs.iter().any(|o| o.0 == n) {
                    return Err(r.err(format!("output `{n}` empty or twice")));
                }
                t.outputs.push((n.into(), k.into(), e.into()));
            }
            "spec" => {
                r.arity(3)?;
                let (o, op, v) = (r.str(0)?, r.str(1)?, r.str(2)?);
                if !matches!(op, "lt" | "gt") {
                    return Err(r.err(format!("spec {o}: op `{op}` (lt or gt)")));
                }
                if !t.outputs.iter().any(|x| x.0 == o) {
                    return Err(r.err(format!("spec names output `{o}`, not declared before it")));
                }
                if t.specs.iter().any(|x| x.0 == o && x.1 == op) {
                    return Err(r.err(format!("spec {o} {op} twice")));
                }
                t.specs.push((o.into(), op.into(), v.into()));
            }
            "defn" => {
                r.arity(2)?;
                let f = r.str(0)?;
                let body = r.text(1)?;
                if f.is_empty() || f.contains(['/', '\\']) || t.defns.iter().any(|d| d.0 == f) {
                    return Err(r.err(format!("defn `{f}`: empty, a path, or twice")));
                }
                t.defns.push((f.into(), body));
            }
            "maestro" => return Err(r.err("a second test in one test's records")),
            o => return Err(r.err(format!("`{o}` is not a maestro record"))),
        }
        let rows = [
            t.vars.len(),
            t.analyses.len(),
            t.corners.len(),
            t.models.len(),
        ];
        if rows
            .iter()
            .chain(&[t.outputs.len(), t.specs.len(), t.defns.len()])
            .any(|&n| n > ROWS_MAX)
        {
            return Err(r.err("more than 2^16 rows of one kind"));
        }
    }
    Ok(t)
}

/// Checks one maestro test's records (its `maestro` record first) as [`import`] reads them.
///
/// # Errors
///
/// The first malformed record, with its line: wrong arity or element types, `enable` other
/// than 0/1, an option that is not `(key value)` strings, a var/model naming a corner not
/// declared before it, a spec naming an output not declared before it or an op other than
/// `lt`/`gt`, an output kind other than `point`/`net`, a name twice in its kind, a
/// definition file that is a path.
pub fn check(records: &[Record]) -> Result<(), Error> {
    parse(records).map(|_| ())
}

/// Kinds that mean a test was already imported into the bench being built.
const BENCH_KINDS: [RecordKind; 6] = [
    RecordKind::Var,
    RecordKind::Corner,
    RecordKind::Sweep,
    RecordKind::Output,
    RecordKind::Spec,
    RecordKind::Code,
];

/// Appends the bench records (`var`, `sweep`, `corner`, `output`, `spec`, `code`) that
/// `records` (one maestro test and its records, in file order) describe to `bench` (§9.2).
/// What cannot come back is reported in `notes` under `cell`: an analysis without a SPICE
/// form (the block is then Spectre), an output whose ADE expression does not translate (a
/// comment in the analysis block), a disabled analysis, a value without a
/// Schemify form, a second test of the same view (only the first maps).
///
/// Records [`check`] refuses are reported (`needs_review:code`) and nothing is added; the
/// reader checks them first, so this happens only for records built in memory.
pub fn import(records: &[Record], bench: &mut Document<'static>, cell: &str, notes: &mut Notes) {
    let t = match parse(records) {
        Ok(t) => t,
        Err(e) => {
            notes.review("code", cell, format!("maestro refused: {e}"));
            return;
        }
    };
    if (1..bench.record_count()).any(|r| BENCH_KINDS.contains(&bench.record_kind(r))) {
        notes.review(
            "config",
            cell,
            format!(
                "maestro test {}: only the first test of a view maps",
                t.name
            ),
        );
        return;
    }
    let before = bench.record_count();
    let globals: Vec<String> = t
        .vars
        .iter()
        .filter(|v| v.2.is_empty() && v.0 != "temperature")
        .map(|v| v.0.clone())
        .collect();
    let from = |v: &str, notes: &mut Notes| -> Option<String> {
        let r = ael::from_ael(
            v,
            |n| {
                if globals.iter().any(|g| g == n) {
                    AelName::BenchVar
                } else {
                    AelName::Unknown
                }
            },
            |_| None,
        );
        match r {
            Some(tr) if Program::compile(&tr.text, Mode::Scalar).is_ok() => {
                if let Some(why) = tr.review {
                    notes.review(why, cell, format!("`{v}` → `{}`", tr.text));
                }
                Some(tr.text)
            }
            _ => {
                notes.review("expression", cell, format!("`{v}` has no Schemify form"));
                None
            }
        }
    };
    // Vars and sweeps.
    for (n, v, sc) in t
        .vars
        .iter()
        .filter(|v| v.2.is_empty() && v.0 != "temperature")
    {
        debug_assert!(sc.is_empty());
        let colon = colon_points(v);
        let parts: Vec<&str> = match &colon {
            Some(p) => p.iter().map(String::as_str).collect(),
            None => top_level_split(v),
        };
        let mut pts = Vec::new();
        for p in &parts {
            match from(p, notes) {
                Some(x) => pts.push(x),
                None => break,
            }
        }
        if pts.len() != parts.len() {
            continue;
        }
        push_record(bench, RecordKind::Var, &[n, &pts[0]]);
        if pts.len() > 1 {
            push_record(bench, RecordKind::Sweep, &[n]);
            push_attr(bench, "var", &[n]);
            let (key, vals) = sweep_shape(&pts);
            push_attr(
                bench,
                key,
                &vals.iter().map(String::as_str).collect::<Vec<_>>(),
            );
        }
    }
    // Corners.
    for (c, en) in &t.corners {
        push_record(bench, RecordKind::Corner, &[c]);
        for (_, f, sec) in t.models.iter().filter(|m| &m.0 == c) {
            let sec = sec
                .strip_prefix('"')
                .and_then(|x| x.strip_suffix('"'))
                .unwrap_or(sec);
            if sec.is_empty() {
                push_attr(bench, "include", &[&path_from_ade(f)]);
            } else {
                push_attr(bench, "lib", &[&path_from_ade(f), sec]);
            }
        }
        let mut temp = None;
        for (n, v, _) in t.vars.iter().filter(|v| &v.2 == c) {
            if top_level_split(v).len() > 1 {
                notes.review(
                    "expression",
                    cell,
                    format!("corner {c} var {n}: a sweep list"),
                );
                continue;
            }
            let Some(x) = from(v, notes) else { continue };
            if n == "temperature" {
                temp = Some(x);
            } else {
                push_attr(bench, "set", &[n, &x]);
            }
        }
        if let Some(x) = temp {
            push_attr(bench, "temp", &[&x]);
        }
        if !en {
            push_attr(bench, "enable", &["false"]);
        }
    }
    // The analysis block: SPICE when every analysis has a SPICE form, else Spectre.
    // ADE answers the test temperature 27 when nothing set it (live run): no `.temp` line
    // then (a bench that wrote `.temp 27` loses only that spelling of the default).
    let temp = t
        .vars
        .iter()
        .find(|v| v.0 == "temperature" && v.2.is_empty())
        .map(|v| &v.1)
        .filter(|v| v.parse::<f64>().is_ok_and(|x| x != 27.0) || v.parse::<f64>().is_err());
    let enabled: Vec<&Analysis> = t
        .analyses
        .iter()
        .filter(|a| {
            if !a.1 {
                notes.review(
                    "code",
                    cell,
                    format!("analysis {} is disabled in maestro: dropped", a.0),
                );
            }
            a.1
        })
        .collect();
    let spice: Option<Vec<String>> = enabled
        .iter()
        .map(|a| spice_form(&a.0, &a.2))
        .collect::<Option<Vec<Vec<String>>>>()
        .map(|v| v.concat());
    let spice_temp = match temp {
        Some(v) => from(v, notes).map(|x| format!(".temp {x}")),
        None => Some(String::new()),
    };
    let mut lines: Vec<String> = Vec::new();
    let as_spice = spice.is_some() && spice_temp.is_some();
    if as_spice {
        for (_, f, sec) in t.models.iter().filter(|m| m.0.is_empty()) {
            let f = path_from_ade(f);
            let f = if f.contains(char::is_whitespace) {
                format!("\"{f}\"")
            } else {
                f
            };
            lines.push(if sec.is_empty() {
                format!(".include {f}")
            } else {
                format!(".lib {f} {sec}")
            });
        }
        lines.extend(spice_temp.filter(|x| !x.is_empty()));
        lines.extend(spice.unwrap_or_default());
    } else {
        for a in &enabled {
            if spice_form(&a.0, &a.2).is_none() {
                notes.review(
                    "code",
                    cell,
                    format!(
                        "analysis {} has no SPICE form: kept in the spectre block",
                        a.0
                    ),
                );
            }
        }
        for (_, f, sec) in t.models.iter().filter(|m| m.0.is_empty()) {
            // Always quoted: a bare `/pdk/m.scs` reads as an expression (`/` divides);
            // ADE writes `include "…"`.
            let mut l = format!("include {}", spectre_quoted(f));
            if !sec.is_empty() {
                l.push_str(&format!(" section={}", spectre_word(sec)));
            }
            lines.push(l);
        }
        if let Some(v) = temp {
            lines.push(format!("temp options temp={}", spectre_word(v)));
        }
        for (n, _, opts) in &enabled {
            let mut l = format!("{n} {n}");
            for (k, v) in opts.iter().filter(|o| !FORM_ONLY.contains(&o.0.as_str())) {
                l.push_str(&format!(" {k}={}", spectre_word(v)));
            }
            lines.push(l);
        }
    }
    // Outputs and specs.
    let comment = if as_spice { "*" } else { "//" };
    let mut kept: Vec<&str> = Vec::new();
    for (n, k, e) in &t.outputs {
        match (k.as_str(), out_from_ade(e)) {
            ("point", Some(x)) => {
                push_record(bench, RecordKind::Output, &[n, &x]);
                kept.push(n);
            }
            _ => {
                notes.review(
                    "output",
                    cell,
                    format!("output {n} ({k}) `{e}` has no calculator form"),
                );
                lines.push(format!("{comment} ade output {n} {k}: {e}").replace(['\n', '\r'], " "));
            }
        }
    }
    for o in &kept {
        let mut bounds: Vec<(&str, String)> = Vec::new();
        for (key, op) in [("min", "gt"), ("max", "lt")] {
            if let Some((_, _, v)) = t.specs.iter().find(|x| x.0 == *o && x.1 == op)
                && let Some(x) = from(v, notes)
            {
                bounds.push((key, x));
            }
        }
        if !bounds.is_empty() {
            push_record(bench, RecordKind::Spec, &[o]);
            for (key, x) in bounds {
                push_attr(bench, key, &[&x]);
            }
        }
    }
    for (o, op, _) in &t.specs {
        if !kept.contains(&o.as_str()) {
            notes.review(
                "spec",
                cell,
                format!("spec {o} {op}: its output did not come back"),
            );
        }
    }
    if !lines.is_empty() {
        push_record(
            bench,
            RecordKind::Code,
            &[&t.name, if as_spice { "spice" } else { "spectre" }],
        );
        for l in &lines {
            bench.push_verbatim(l);
        }
    }
    // Definition files.
    for (f, body) in &t.defns {
        let stem = f.strip_suffix(".scs").unwrap_or(f);
        if stem == t.name {
            notes.review(
                "code",
                cell,
                format!("defn {f}: its name is the analysis block's"),
            );
            continue;
        }
        push_record(bench, RecordKind::Code, &[stem, "spectre"]);
        for l in body.lines() {
            bench.push_verbatim(&l.replace('\r', ""));
        }
    }
    assert!(bench.record_count() >= before, "the import only appends");
}

/// The points of ADE's `start:step:stop` sweep form (`"0.81:0.09:0.99"`), `None` for any
/// other text or a step that does not land on `stop` within 1e-9 of a step. The points are
/// [`grid`]'s, so the sweep reads back as `lin start stop n`.
fn colon_points(v: &str) -> Option<Vec<String>> {
    let f: Vec<f64> = v
        .split(':')
        .map(|x| ael::fold_numeric(x.trim())?.parse().ok())
        .collect::<Option<_>>()?;
    let [a, step, b] = f[..] else { return None };
    if step == 0.0 || !(a.is_finite() && step.is_finite() && b.is_finite()) {
        return None;
    }
    let k = (b - a) / step;
    let n = k.round();
    if (k - n).abs() > 1e-9 || n < 1.0 || n + 1.0 > f64::from(SWEEP_POINTS_MAX) {
        return None;
    }
    let pts = grid(a, b, n as u32 + 1, false)?;
    debug_assert!(pts.len() >= 2);
    Some(pts)
}

/// The sweep attribute that reproduces `pts` exactly: `lin a b n`, else `log a b n`, else
/// `values …` (lin/log only when every point is canonical number text).
pub(super) fn sweep_shape(pts: &[String]) -> (&'static str, Vec<String>) {
    assert!(pts.len() >= 2);
    let canon = |p: &String| {
        p.parse::<f64>().ok().filter(|x| {
            let mut s = String::new();
            write_num(&mut s, *x);
            &s == p
        })
    };
    let n = pts.len() as u32;
    if let (Some(a), Some(b), true) = (
        canon(&pts[0]),
        canon(&pts[pts.len() - 1]),
        n <= SWEEP_POINTS_MAX,
    ) {
        for (key, log) in [("lin", false), ("log", true)] {
            if grid(a, b, n, log).as_deref() == Some(pts) {
                return (
                    key,
                    vec![pts[0].clone(), pts[pts.len() - 1].clone(), n.to_string()],
                );
            }
        }
    }
    ("values", pts.to_vec())
}

/// The SPICE lines of one ADE analysis (§9.2), `None` when it has none: options exactly
/// those §9.1 writes for it, numbers only.
fn spice_form(name: &str, opts: &[(String, String)]) -> Option<Vec<String>> {
    let get = |k: &str| opts.iter().find(|o| o.0 == k).map(|o| o.1.as_str());
    let num = |k: &str| -> Option<String> {
        let t = ael::from_ael(get(k)?, |_| AelName::Unknown, |_| None)?.text;
        ael::number_to_ael(&t).map(|_| t)
    };
    let only = |keys: &[&str]| opts.iter().all(|o| keys.contains(&o.0.as_str()));
    let freq = || -> Option<String> {
        let (f1, f2) = (num("start")?, num("stop")?);
        match (get("incrType")?, get("stepTypeLog"), get("stepTypeLin")) {
            ("Logarithmic", Some("Points Per Decade"), None) => {
                Some(format!("dec {} {f1} {f2}", num("dec")?))
            }
            ("Linear", None, Some("Number of Steps")) => {
                Some(format!("lin {} {f1} {f2}", num("lin")?))
            }
            _ => None,
        }
    };
    const FREQ: [&str; 7] = [
        "start",
        "stop",
        "incrType",
        "stepTypeLog",
        "stepTypeLin",
        "dec",
        "lin",
    ];
    match name {
        "tran" if only(&["stop", "start", "maxstep", "errpreset"]) => {
            let stop = num("stop")?;
            let x: f64 = ael::fold_numeric(&stop)?.parse().ok()?;
            let mut step = String::new();
            write_num(&mut step, x / 1000.0);
            let mut l = format!(".tran {step} {stop}");
            match (
                get("start").map(|_| num("start")),
                get("maxstep").map(|_| num("maxstep")),
            ) {
                (None, None) => {}
                (Some(st), None) => l.push_str(&format!(" {}", st?)),
                (st, Some(m)) => {
                    l.push_str(&format!(" {} {}", st.unwrap_or(Some("0".into()))?, m?))
                }
            }
            match get("errpreset") {
                Some(p) if ERRPRESETS.contains(&p) => {
                    Some(vec![format!(".options errpreset={p}"), l])
                }
                Some(_) => None,
                None => Some(vec![l]),
            }
        }
        "ac" if only(&FREQ) => Some(vec![format!(".ac {}", freq()?)]),
        "dc" => {
            let op = get("saveOppoint");
            let sweep = ["sweep", "dev", "param", "start", "stop", "step"];
            if !only(&[&sweep[..], &["saveOppoint"]].concat()) || op.is_some_and(|v| v != "t") {
                return None;
            }
            let mut out = Vec::new();
            if op.is_some() {
                out.push(".op".to_owned());
            }
            if opts.len() > usize::from(op.is_some()) {
                let dev = get("dev")?.strip_prefix('/').filter(|d| is_ident(d))?;
                if get("sweep")? != "Component Parameter" || get("param")? != "dc" {
                    return None;
                }
                out.push(format!(
                    ".dc {dev} {} {} {}",
                    num("start")?,
                    num("stop")?,
                    num("step")?
                ));
            }
            (!out.is_empty()).then_some(out)
        }
        "noise" if only(&[&FREQ[..], &["oprobe", "iprobe"]].concat()) => {
            let o = get("oprobe")?.strip_prefix('/').filter(|d| is_ident(d))?;
            let src = get("iprobe")?.strip_prefix('/').filter(|d| is_ident(d))?;
            Some(vec![format!(".noise v({o}) {src} {}", freq()?)])
        }
        _ => None,
    }
}
