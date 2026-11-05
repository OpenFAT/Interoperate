//! A maestro view's tests' records → bench records ([`import`], spec `interop.md` §9.2):
//! each test's records parsed and checked ([`check`], the reader's boundary), then the
//! view's vars and sweeps, device parameters and corners, then per test its analysis block
//! (SPICE when every analysis has a SPICE form, else Spectre), outputs and specs,
//! definition files and, for a view of several tests, its `test` record.

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
/// kinds `point`/`net`, no name twice per kind, at most [`ROWS_MAX`] rows per kind; the
/// view's `maesdb` (text, digest rows) at most once and a `maestate` (test, text) per test;
/// `testvar` (name, value) once per name, `testenable` 0/1 once, `parameter` (path
/// `inst[/inst…]/prop`, value, scope) once per path and scope, `varenable` and
/// `outenable` (name, 0/1) once per name.
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
    let mut sdb_seen = false;
    let mut states: Vec<&str> = Vec::new();
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
                // maeGetAnalysis answers every form field nothing set (`anaName`, the
                // choosers, the options form; forms.tsv): they carry no meaning, so a dump
                // of a setup we wrote reads as what we wrote. An empty field is unset.
                opts.retain(|(k, v)| !v.is_empty() && !is_form_default(n, k, v));
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
            // The view's setup XML and test states (§9.7): checked here, kept by
            // `restore` (they are no row of the test).
            "maesdb" => {
                r.arity(2)?;
                r.text(0)?;
                for d in r.list(1)? {
                    if d.items().len() < 3 || d.items().iter().any(|x| x.as_str().is_none()) {
                        return Err(r.err("a record digest is not (digest tag key…) strings"));
                    }
                }
                if std::mem::replace(&mut sdb_seen, true) {
                    return Err(r.err("maesdb twice in one test"));
                }
            }
            "maestate" => {
                r.arity(2)?;
                let n = r.str(0)?;
                r.text(1)?;
                if n.is_empty() || states.contains(&n) {
                    return Err(r.err(format!("maestate `{n}` empty or twice")));
                }
                states.push(n);
            }
            "testvar" => {
                r.arity(2)?;
                let (n, v) = (r.str(0)?, r.str(1)?);
                if n.is_empty() || t.own.iter().any(|x| x.0 == n) {
                    return Err(r.err(format!("testvar `{n}` empty or twice")));
                }
                t.own.push((n.into(), v.into()));
            }
            "testenable" => {
                r.arity(1)?;
                if t.enable.replace(flag(r, 0)?).is_some() {
                    return Err(r.err("testenable twice"));
                }
            }
            "varenable" | "outenable" => {
                r.arity(2)?;
                let (n, en) = (r.str(0)?, flag(r, 1)?);
                let rows = if r.tag == "varenable" {
                    &mut t.var_enable
                } else {
                    &mut t.out_enable
                };
                if n.is_empty() || rows.iter().any(|x| x.0 == n) {
                    return Err(r.err(format!("{} `{n}` empty or twice", r.tag)));
                }
                rows.push((n.into(), en));
            }
            "simoption" | "testdesign" | "maenote" => {
                r.arity(2)?;
                let (a, b) = (r.str(0)?, r.str(1)?);
                match r.tag.as_str() {
                    "simoption" if a.is_empty() || t.sim_opts.iter().any(|x| x.0 == a) => {
                        return Err(r.err(format!("simoption `{a}` empty or twice")));
                    }
                    "simoption" => t.sim_opts.push((a.into(), b.into())),
                    "testdesign" if t.design.is_some() || a.is_empty() || b.is_empty() => {
                        return Err(r.err("testdesign twice, or an empty lib or cell"));
                    }
                    "testdesign" => t.design = Some((a.into(), b.into())),
                    _ => t.notes.push((a.into(), b.into())),
                }
            }
            "cornerrun" => {
                r.arity(2)?;
                let (c, on) = (r.str(0)?, flag(r, 1)?);
                if !t.corners.iter().any(|x| x.0 == c) || t.corner_runs.iter().any(|x| x.0 == c) {
                    return Err(r.err(format!(
                        "cornerrun `{c}`: twice, or no such corner before it"
                    )));
                }
                t.corner_runs.push((c.into(), on));
            }
            "parameter" => {
                r.arity(3)?;
                let (p, v, sc) = (r.str(0)?, r.str(1)?, r.str(2)?);
                if p.split('/').count() < 2 || p.split('/').any(str::is_empty) {
                    return Err(r.err(format!("parameter `{p}`: not inst[/inst…]/prop")));
                }
                if !scope_ok(&t, sc) || t.params.iter().any(|x| x.0 == p && x.2 == sc) {
                    return Err(r.err(format!(
                        "parameter {p} in scope `{sc}`: twice, or no such corner before it"
                    )));
                }
                t.params.push((p.into(), v.into(), sc.into()));
            }
            "maestro" => return Err(r.err("a second test in one test's records")),
            o => return Err(r.err(format!("`{o}` is not a maestro record"))),
        }
        let rows = [
            t.vars.len(),
            t.analyses.len(),
            t.corners.len(),
            t.models.len(),
            t.own.len(),
            t.params.len(),
            t.var_enable.len(),
            t.out_enable.len(),
            t.sim_opts.len(),
            t.corner_runs.len(),
            t.notes.len(),
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
/// definition file that is a path, a second `maesdb` or a test's second `maestate`.
pub fn check(records: &[Record]) -> Result<(), Error> {
    parse(records).map(|_| ())
}

/// Test record attribute: `vnone code|output`, the test runs no code block (no output),
/// which a missing `code` (`output`) attribute cannot say.
pub(super) const VNONE: &str = "vnone";

/// Kinds that mean a test was already imported into the bench being built.
const BENCH_KINDS: [RecordKind; 8] = [
    RecordKind::Var,
    RecordKind::Corner,
    RecordKind::Sweep,
    RecordKind::Output,
    RecordKind::Spec,
    RecordKind::Code,
    RecordKind::Test,
    RecordKind::BenchParam,
];

/// `v` (AEL) as a Schemify expression, `globals` the bench vars; `None` (noted
/// `expression`) when it has none.
fn from_ael(v: &str, globals: &[String], cell: &str, notes: &mut Notes) -> Option<String> {
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
}

/// A device parameter's Maestro location (`lib/cell/view/inst[/inst…]`) and property as the
/// `parameter` record's path (`inst[/inst…]/prop`), when it is an instance of test `t`'s
/// design cell.
fn param_path(location: &str, prop: &str, t: &Test) -> Option<String> {
    let mut seg = location.split('/');
    let (lib, cell, _view) = (seg.next()?, seg.next()?, seg.next()?);
    let insts: Vec<&str> = seg.collect();
    (lib == t.lib && cell == t.cell && !insts.is_empty() && !prop.is_empty())
        .then(|| format!("{}/{prop}", insts.join("/")))
}

/// A `parameter` path (Virtuoso instance names) as the bench's (`inst[/inst…]/prop`,
/// format.md rank 6).
fn param_from_virtuoso(p: &str) -> String {
    let (insts, prop) = p.rsplit_once('/').expect("parse checked inst/prop");
    let insts: Vec<String> = insts
        .split('/')
        .map(crate::profile::inst_from_virtuoso)
        .collect();
    format!("{}/{prop}", insts.join("/"))
}

/// The device parameters of a view, `(path, value, scope)`: the setup XML's (global, and
/// per corner) as `parameter` paths, overridden by the first test's `parameter` records (a
/// Rust export read back without Virtuoso). A parameter on another cell, or of Nominal,
/// is reported.
fn view_params(
    setup: Option<&Setup>,
    first: &Test,
    cell: &str,
    notes: &mut Notes,
) -> Vec<(String, String, String)> {
    let mut out: Vec<(String, String, String)> = Vec::new();
    if let Some(su) = setup {
        let scoped = su.params.iter().map(|p| (p, "")).chain(
            su.corners
                .iter()
                .flat_map(|c| c.params.iter().map(move |p| (p, c.name.as_str()))),
        );
        for (p, sc) in scoped {
            match param_path(&p.location, &p.name, first) {
                Some(path) if sc != "_default" => out.push((path, p.value.clone(), sc.into())),
                _ => notes.review(
                    "config",
                    cell,
                    format!(
                        "maestro parameter {}/{} ({}): not an instance of the bench, or of \
                         Nominal; kept in the setup XML",
                        p.location,
                        p.name,
                        if sc.is_empty() { "global" } else { sc }
                    ),
                ),
            }
        }
    }
    for p in &first.params {
        match out.iter_mut().find(|x| x.0 == p.0 && x.2 == p.2) {
            Some(x) => x.1 = p.1.clone(),
            None => out.push(p.clone()),
        }
    }
    out
}

/// Appends the bench records that `tests` (the maestro tests of one view, each its
/// `maestro` record and the records after it, in file order) describe to `bench` (§9.2):
/// from the first test the view's `var`s, `sweep`s and `corner`s (the setup every test
/// sees), the view's device parameters as `param` records and corner `param` lines; from
/// every test its analysis block (`code <test>`), outputs, specs and definition files.
/// A view of several tests, or of one that is disabled, has vars of its own or omits a
/// global, also gets a `test` record per test (format.md rank 6): `code`, `output`, `set`
/// (`testvar`), `omit` and `enable false` (from the setup XML), `config` (its view when not
/// `schematic`). A test's dependent outputs (another of its outputs by name) and
/// cross-test outputs (`calcVal`) become `result(output, test)`.
///
/// What cannot come back is reported in `notes` under `cell`: an analysis without a SPICE
/// form (the block is then Spectre), an output whose ADE expression does not translate (a
/// comment in the analysis block), an output or definition file of a name another test
/// already took, a disabled analysis, a value without a Schemify form. The view's own
/// setup (`maesdb`, `maestate`) goes into the bench header (`restore`, §9.7).
///
/// Records [`check`] refuses are reported (`needs_review:code`) and that test is skipped;
/// the reader checks them first, so this happens only for records built in memory.
pub fn import(tests: &[&[Record]], bench: &mut Document<'static>, cell: &str, notes: &mut Notes) {
    let mut ts: Vec<Test> = Vec::new();
    let mut ok: Vec<&[Record]> = Vec::new();
    for recs in tests {
        match parse(recs) {
            Ok(t) => {
                ts.push(t);
                ok.push(recs);
            }
            Err(e) => notes.review("code", cell, format!("maestro refused: {e}")),
        }
    }
    let Some(first) = ts.first() else { return };
    if (1..bench.record_count()).any(|r| BENCH_KINDS.contains(&bench.record_kind(r))) {
        notes.review(
            "config",
            cell,
            format!(
                "maestro test {}: the bench already holds a maestro view",
                first.name
            ),
        );
        return;
    }
    restore::attach(&ok, &ts, bench);
    let before = bench.record_count();
    let setup = ok[0]
        .iter()
        .find(|r| r.tag == "maesdb")
        .and_then(|r| r.text(0).ok())
        .and_then(|x| Setup::from_xml(&x).ok());
    let enabled = |t: &Test| {
        t.enable
            .or_else(|| setup.as_ref()?.test(&t.name).map(|x| x.enabled))
            .unwrap_or(true)
    };
    let omitted = |t: &Test| -> Vec<String> {
        setup.as_ref().map_or(Vec::new(), |su| {
            su.vars
                .iter()
                .filter(|v| v.dependents.iter().any(|d| d.0 == t.name && d.1))
                .map(|v| v.name.clone())
                .collect()
        })
    };
    let several = ts.len() > 1
        || !first.own.is_empty()
        || !enabled(first)
        || !omitted(first).is_empty()
        || first.design.is_some()
        || first.corner_runs.iter().any(|x| !x.1);
    // What the dumper read and no record carries, and a test Schemify cannot run as Maestro
    // does (another simulator).
    for t in &ts {
        for (class, x) in &t.notes {
            notes.review(class, cell, format!("maestro test {}: {x}", t.name));
        }
        if t.sim != "spectre" {
            notes.review(
                "config",
                cell,
                format!(
                    "maestro test {} runs simulator {}: a Schemify run uses its own; the test \
                     goes back to Maestro as spectre unless the setup XML restores it",
                    t.name, t.sim
                ),
            );
        }
    }
    let globals: Vec<String> = first
        .vars
        .iter()
        .filter(|v| v.2.is_empty() && v.0 != "temperature")
        .map(|v| v.0.clone())
        .collect();
    let from = |v: &str, notes: &mut Notes| from_ael(v, &globals, cell, notes);
    // Vars and sweeps.
    for (n, v, sc) in first
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
        // §9.11: a disabled var (`<var enabled="0">`, or `varenable`) is not netlisted.
        let on = first.var_enable.iter().find(|x| &x.0 == n).map(|x| x.1);
        if !on
            .or_else(|| Some(setup.as_ref()?.var(n)?.enabled))
            .unwrap_or(true)
        {
            push_attr(bench, "enable", &["false"]);
        }
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
    // Device parameters.
    let params = view_params(setup.as_ref(), first, cell, notes);
    for (p, v, _) in params.iter().filter(|x| x.2.is_empty()) {
        if let Some(x) = from(v, notes) {
            push_record(
                bench,
                RecordKind::BenchParam,
                &[&param_from_virtuoso(p), &x],
            );
        }
    }
    // Corners.
    for (c, en) in &first.corners {
        push_record(bench, RecordKind::Corner, &[c]);
        for (_, f, sec) in first.models.iter().filter(|m| &m.0 == c) {
            // Several quoted sections (`"tt" "ss"`, live) make the corner a group over them:
            // Schemify has none (core_A.md), the first comes back.
            let secs: Vec<&str> = sec.split('"').filter(|x| !x.trim().is_empty()).collect();
            if secs.len() > 1 {
                notes.review(
                    "corner",
                    cell,
                    format!(
                        "corner {c} model {f} sections {}: a corner group; only {} comes back \
                         (Schemify has no corner group)",
                        secs.join(" "),
                        secs[0]
                    ),
                );
            }
            let sec = secs.first().copied().unwrap_or("");
            if sec.is_empty() {
                push_attr(bench, "include", &[&path_from_ade(f)]);
            } else {
                push_attr(bench, "lib", &[&path_from_ade(f), sec]);
            }
        }
        let mut temp = None;
        for (n, v, _) in first.vars.iter().filter(|v| &v.2 == c) {
            // A list makes the corner a group (a corner per point, Maestro's `C_0`, `C_1`…;
            // the netlist of the corner itself takes the first point: live). Schemify has
            // no corner group (core_A.md): the first point.
            let pts = top_level_split(v);
            if pts.len() > 1 {
                notes.review(
                    "corner",
                    cell,
                    format!(
                        "corner {c} var {n} {v}: a corner group; only its first point {} \
                         comes back (Schemify has no corner group)",
                        pts[0]
                    ),
                );
            }
            let Some(x) = from(pts[0], notes) else {
                continue;
            };
            if n == "temperature" {
                temp = Some(x);
            } else {
                push_attr(bench, "set", &[n, &x]);
            }
        }
        if let Some(x) = temp {
            push_attr(bench, "temp", &[&x]);
        }
        for (p, v, _) in params.iter().filter(|x| &x.2 == c) {
            if let Some(x) = from(v, notes) {
                push_attr(bench, "param", &[&param_from_virtuoso(p), &x]);
            }
        }
        if !en {
            push_attr(bench, "enable", &["false"]);
        }
    }
    // Each test: its analysis block, outputs, specs and definition files, then its record.
    // §9.11: an output whose Plot is off in its test's state (or `outenable 0`) is
    // disabled.
    let states: Vec<(String, String)> = ok[0]
        .iter()
        .filter(|r| r.tag == "maestate")
        .filter_map(|r| Some((r.str(0).ok()?.to_owned(), r.text(1).ok()?)))
        .collect();
    let off: Vec<(String, String)> = ts
        .iter()
        .flat_map(|t| {
            let st = states.iter().find(|s| s.0 == t.name);
            let from_state = st.map_or(Vec::new(), |s| disabled::plot_off(&s.1));
            t.outputs
                .iter()
                .filter(move |o| match t.out_enable.iter().find(|x| x.0 == o.0) {
                    Some(x) => !x.1,
                    None => from_state.contains(&o.0),
                })
                .map(|o| (t.name.clone(), o.0.clone()))
        })
        .collect();
    let mut taken = Taken {
        names: ts.iter().map(|t| t.name.clone()).collect(),
        off,
        ..Taken::default()
    };
    for t in &ts {
        let (codes, outs) = import_test(t, &globals, bench, cell, &mut taken, notes);
        if !several {
            continue;
        }
        push_record(bench, RecordKind::Test, &[&t.name]);
        // An empty list has no attribute form (a test record without one runs them all):
        // `vnone <key>` keeps it for the way back.
        for (key, list) in [("code", &codes), ("output", &outs)] {
            if list.is_empty() {
                notes.review(
                    "config",
                    cell,
                    format!(
                        "maestro test {} has no {key}: a Schemify run gives it every one",
                        t.name
                    ),
                );
                push_attr(bench, VNONE, &[key]);
            } else {
                push_attr(bench, key, list);
            }
        }
        for (n, v) in &t.own {
            if top_level_split(v).len() > 1 {
                notes.review(
                    "expression",
                    cell,
                    format!("test {} var {n}: a sweep list", t.name),
                );
            } else if let Some(x) = from(v, notes) {
                push_attr(bench, "set", &[n, &x]);
            }
        }
        for o in omitted(t) {
            push_attr(bench, "omit", &[&o]);
        }
        if t.view != "schematic" {
            push_attr(bench, "config", &[&t.view]);
        }
        // The corners the test runs: Maestro's per-corner disabled tests.
        if t.corner_runs.iter().any(|x| !x.1) {
            let runs: Vec<&str> = first
                .corners
                .iter()
                .filter(|c| c.1 && !t.corner_runs.iter().any(|x| x.0 == c.0 && !x.1))
                .map(|c| c.0.as_str())
                .collect();
            if runs.is_empty() {
                notes.review(
                    "corner",
                    cell,
                    format!(
                        "maestro test {} runs no corner: a Schemify run gives it all",
                        t.name
                    ),
                );
            } else {
                push_attr(bench, "corner", &runs);
            }
        }
        if let Some((l, c)) = &t.design {
            let r = if *l == t.lib {
                format!("./{c}")
            } else {
                format!("{l}:{c}")
            };
            push_attr(bench, "design", &[&r]);
        }
        if !enabled(t) {
            push_attr(bench, "enable", &["false"]);
        }
    }
    // §9.9: the stored setup's run modes, with the first test's specs as goals (lane L5).
    runmode::import(ok[0], &first.name, bench, cell, notes);
    assert!(bench.record_count() >= before, "the import only appends");
}

/// One output's spec bounds `(op, value)`.
type Specs = Vec<(String, String)>;

/// What the tests of one view put in the bench so far (`import_test`).
#[derive(Default)]
struct Taken {
    /// The view's test names.
    names: Vec<String>,
    /// Outputs `(name, expression, specs)`, specs `(op, value)` as Maestro holds them: a
    /// test whose output is another's runs that.
    outs: Vec<(String, String, Specs)>,
    /// Analysis blocks `(name, lang, lines)`: a test whose block is another's runs that.
    blocks: Vec<(String, String, Vec<String>)>,
    /// Definition files `(stem, text)`.
    defns: Vec<(String, String)>,
    /// `(test, output)`: the disabled outputs.
    off: Vec<(String, String)>,
}

/// Appends test `t`'s analysis block (`code <test>`, or the earlier test's block of the
/// same text), outputs, specs and definition files to `bench`; `taken` is what earlier
/// tests of the view put there. Returns the code blocks and outputs the test runs, in
/// bench names.
fn import_test(
    t: &Test,
    globals: &[String],
    bench: &mut Document<'static>,
    cell: &str,
    taken: &mut Taken,
    notes: &mut Notes,
) -> (Vec<String>, Vec<String>) {
    let from = |v: &str, notes: &mut Notes| from_ael(v, globals, cell, notes);
    // The analysis block: SPICE when every analysis has a SPICE form, else Spectre.
    // ADE answers the test temperature 27 when nothing set it (live run): no `.temp` line
    // then (a bench that wrote `.temp 27` loses only that spelling of the default).
    let temp = t
        .vars
        .iter()
        .find(|v| v.0 == "temperature" && v.2.is_empty())
        .map(|v| {
            // A temperature list is Maestro's temperature sweep (a global var `temperature`,
            // which wins over the test's temp option in the netlist: live). Schemify has no
            // swept temperature (core_A.md): its first point, the one ADE's own netlist runs.
            let pts = top_level_split(&v.1);
            if pts.len() > 1 {
                notes.review(
                    "sweep",
                    cell,
                    format!(
                        "maestro test {}: temperature sweep {}: only its first point {} comes \
                         back (Schemify has no temperature sweep)",
                        t.name, v.1, pts[0]
                    ),
                );
            }
            pts[0]
        })
        .filter(|v| v.parse::<f64>().is_ok_and(|x| x != 27.0) || v.parse::<f64>().is_err());
    // The test's simulator options ADE would netlist: those not at their default (temp is
    // the temperature above).
    let sim_opts: Vec<&(String, String)> = t
        .sim_opts
        .iter()
        .filter(|(k, v)| {
            k != "temp" && !v.is_empty() && form_default("options", k) != Some(v.as_str())
        })
        .collect();
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
        let mut kvs = Vec::new();
        for (k, v) in &sim_opts {
            match NGSPICE_OPTIONS.iter().find(|o| o.0 == *k) {
                Some((_, n)) if ael::number_to_ael(v).is_some() => kvs.push(format!("{n}={v}")),
                _ => notes.review(
                    "code",
                    cell,
                    format!(
                        "maestro test {}: simulator option {k}={v} has no ngspice form: kept \
                         in the setup XML only",
                        t.name
                    ),
                ),
            }
        }
        if !kvs.is_empty() {
            lines.push(format!(".options {}", kvs.join(" ")));
        }
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
        let mut l = String::from("simopts options");
        for (k, v) in &sim_opts {
            if is_spectre_param("options", k) {
                l.push_str(&format!(" {k}={}", spectre_word(v)));
            } else {
                notes.review(
                    "code",
                    cell,
                    format!(
                        "maestro test {}: simulator option {k}={v} is not a Spectre option: \
                         kept in the setup XML only",
                        t.name
                    ),
                );
            }
        }
        if l.contains('=') {
            lines.push(l);
        }
        // Each analysis as the statement ADE netlists (forms.rs): valid Spectre, not ADE's
        // form fields.
        // The large-signal steady states first: Spectre's pac/pnoise/pxf/hbac read the
        // pss/hb before them (ADE's netlist order).
        let mut order = enabled.clone();
        order.sort_by_key(|a| !STEADY.contains(&a.0.as_str()));
        for (n, _, opts) in order {
            let (l, left) = to_spectre(n, opts);
            for why in left {
                notes.review("code", cell, format!("maestro test {}: {why}", t.name));
            }
            lines.push(l);
        }
    }
    // Outputs and specs.
    let comment = if as_spice { "*" } else { "//" };
    let mut kept: Vec<&str> = Vec::new();
    let mut shared: Vec<&str> = Vec::new();
    let own_outs: Vec<&str> = t.outputs.iter().map(|o| o.0.as_str()).collect();
    for (n, k, e) in &t.outputs {
        let specs: Specs = t
            .specs
            .iter()
            .filter(|x| &x.0 == n)
            .map(|x| (x.1.clone(), x.2.clone()))
            .collect();
        let x = out_from_ade_in(e, &t.name, &own_outs);
        if let Some(o) = taken.outs.iter().find(|o| &o.0 == n) {
            if k == "point" && x.as_ref() == Some(&o.1) && o.2 == specs {
                kept.push(n);
                shared.push(n);
            } else {
                notes.review(
                    "output",
                    cell,
                    format!(
                        "maestro test {} output {n}: another test's output of that name \
                         differs; kept in the setup only",
                        t.name
                    ),
                );
            }
            continue;
        }
        match (k.as_str(), x) {
            ("point", Some(x)) => {
                if x.contains("result(") {
                    notes.review(
                        "output",
                        cell,
                        format!(
                            "output {n} `{x}` reads a test's output: a Schemify run refuses \
                             it (X006) until runs schedule tests"
                        ),
                    );
                }
                push_record(bench, RecordKind::Output, &[n, &x]);
                if taken.off.iter().any(|o| o.0 == t.name && &o.1 == n) {
                    push_attr(bench, "enable", &["false"]);
                }
                taken.outs.push((n.clone(), x, specs));
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
    for o in kept.iter().filter(|o| !shared.contains(o)) {
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
    let mut codes: Vec<String> = Vec::new();
    let lang = if as_spice { "spice" } else { "spectre" };
    if let Some(b) = taken.blocks.iter().find(|b| b.1 == lang && b.2 == lines) {
        codes.push(b.0.clone());
    } else if !lines.is_empty() {
        push_record(bench, RecordKind::Code, &[&t.name, lang]);
        for l in &lines {
            bench.push_verbatim(l);
        }
        codes.push(t.name.clone());
        taken.blocks.push((t.name.clone(), lang.to_owned(), lines));
    }
    // Definition files: one block per name, shared by the tests whose file is the same.
    for (f, body) in &t.defns {
        let stem = f.strip_suffix(".scs").unwrap_or(f);
        // A relative include resolves against the netlist's directory in Maestro, the
        // bench's in Schemify (G22).
        for l in body.lines() {
            let w: Vec<&str> = l.split_whitespace().collect();
            if let [inc, p, ..] = w[..]
                && matches!(inc, "include" | "ahdl_include")
                && !p.trim_matches('"').starts_with(['/', '$'])
            {
                notes.review(
                    "code",
                    cell,
                    format!(
                        "defn {f}: `{}` is relative: Maestro reads it beside the netlist, \
                         Schemify beside the bench",
                        l.trim()
                    ),
                );
            }
        }
        if taken.names.iter().any(|n| n == stem) {
            notes.review(
                "code",
                cell,
                format!("defn {f}: its name is a test's analysis block"),
            );
            continue;
        }
        match taken.defns.iter().find(|d| d.0 == stem) {
            Some(d) if &d.1 == body => {}
            Some(_) => {
                notes.review(
                    "code",
                    cell,
                    format!(
                        "maestro test {} defn {f}: another test's file of that name differs; \
                         kept in the setup only",
                        t.name
                    ),
                );
                continue;
            }
            None => {
                push_record(bench, RecordKind::Code, &[stem, "spectre"]);
                for l in body.lines() {
                    bench.push_verbatim(&l.replace('\r', ""));
                }
                taken.defns.push((stem.to_owned(), body.clone()));
            }
        }
        codes.push(stem.to_owned());
    }
    (codes, kept.iter().map(|x| (*x).to_owned()).collect())
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
/// those §9.1 writes for it (a chooser at its form default may be absent), numbers only.
/// Spectre's `lin=n` is n steps, n+1 points; ngspice's `.ac lin n` is n points.
fn spice_form(name: &str, opts: &[(String, String)]) -> Option<Vec<String>> {
    let set = |k: &str| opts.iter().find(|o| o.0 == k).map(|o| o.1.as_str());
    let get = |k: &str| set(k).or_else(|| form_default(name, k));
    let num = |k: &str| -> Option<String> {
        let t = ael::from_ael(get(k)?, |_| AelName::Unknown, |_| None)?.text;
        ael::number_to_ael(&t).map(|_| t)
    };
    // A dc sweep's bounds and step may be negative (`.dc V1 -0.2 0.2 0.1m`).
    let snum = |k: &str| -> Option<String> {
        let t = ael::from_ael(get(k)?, |_| AelName::Unknown, |_| None)?.text;
        ael::signed_number(&t).map(|_| t)
    };
    let only = |keys: &[&str]| opts.iter().all(|o| keys.contains(&o.0.as_str()));
    let freq = || -> Option<String> {
        let (f1, f2) = (num("start")?, num("stop")?);
        match (get("incrType")?, get("stepTypeLog"), get("stepTypeLin")) {
            ("Logarithmic", Some("Points Per Decade"), _) => {
                Some(format!("dec {} {f1} {f2}", num("dec")?))
            }
            ("Linear", _, Some("Number of Steps")) => {
                let steps: u32 = get("lin")?.parse().ok()?;
                Some(format!("lin {} {f1} {f2}", steps.checked_add(1)?))
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
            // 15 significant digits, as sweep points: 1u/1000 is 1e-9, not 9.999…e-10.
            let mut step = String::new();
            write_num(&mut step, format!("{:.14e}", x / 1000.0).parse().ok()?);
            let mut l = format!(".tran {step} {stop}");
            match (
                set("start").map(|_| num("start")),
                set("maxstep").map(|_| num("maxstep")),
            ) {
                (None, None) => {}
                (Some(st), None) => l.push_str(&format!(" {}", st?)),
                (st, Some(m)) => {
                    l.push_str(&format!(" {} {}", st.unwrap_or(Some("0".into()))?, m?))
                }
            }
            match set("errpreset") {
                Some(p) if ERRPRESETS.contains(&p) => {
                    Some(vec![format!(".options errpreset={p}"), l])
                }
                Some(_) => None,
                None => Some(vec![l]),
            }
        }
        "ac" if only(&FREQ) => Some(vec![format!(".ac {}", freq()?)]),
        "dc" => {
            let op = set("saveOppoint");
            let sweep = [
                "sweep",
                "dev",
                "param",
                "start",
                "stop",
                "step",
                "incrType",
                "stepTypeLin",
            ];
            if !only(&[&sweep[..], &["saveOppoint"]].concat()) || op.is_some_and(|v| v != "t") {
                return None;
            }
            let mut out = Vec::new();
            if op.is_some() {
                out.push(".op".to_owned());
            }
            if opts.len() > usize::from(op.is_some()) {
                let dev = get("dev")?.strip_prefix('/').filter(|d| is_ident(d))?;
                // ADE netlists the step of a Linear sweep by step size only (live: an
                // Automatic one has no `step=`).
                if get("sweep")? != "Component Parameter"
                    || get("param")? != "dc"
                    || get("incrType")? != "Linear"
                    || get("stepTypeLin")? != "Step Size"
                {
                    return None;
                }
                out.push(format!(
                    ".dc {dev} {} {} {}",
                    snum("start")?,
                    snum("stop")?,
                    snum("step")?
                ));
            }
            (!out.is_empty()).then_some(out)
        }
        "noise" if only(&[&FREQ[..], &["outType", "p", "n", "iprobe"]].concat()) => {
            if !get("outType")?.eq_ignore_ascii_case("voltage") {
                return None;
            }
            let node = |k: &str| get(k)?.strip_prefix('/').filter(|d| is_ident(d));
            let out = match get("n") {
                None | Some("/gnd!") => format!("v({})", node("p")?),
                Some(_) => format!("v({},{})", node("p")?, node("n")?),
            };
            Some(vec![format!(
                ".noise {out} {} {}",
                node("iprobe")?,
                freq()?
            )])
        }
        _ => None,
    }
}
