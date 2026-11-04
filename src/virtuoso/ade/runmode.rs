//! Maestro's run modes ⇄ bench `montecarlo`/`optimize` records (spec `interop.md` §9.9,
//! lane L5). Maestro keeps them in the setup database the view's own setup carries
//! (`maesdb`, `restore`): `<runoptions>` holds each mode's options, `<specs>` the goals,
//! a global var's `min:step:max` value the optimizer's range.
//!
//! - [`import`]: the stored setup → `montecarlo mc` (mode `Monte Carlo Sampling`),
//!   `optimize global` and `optimize local` (the optimization modes), with the specs of
//!   the mapped test as goals and the range vars as `vary` (the var gains `min`, `max`,
//!   `step`). An option without a Schemify attribute rides as `vmc`/`vopt <option>
//!   <value>`, a target spec's tolerance as `vtol <output> <tol>`.
//! - [`export_run_modes`]: the bench's records → the stored setup's XML, which the loader
//!   restores (`axlImportSetup … 'overwrite`). The stored XML is the baseline: what the
//!   records say differently from what it imports as is written into it, element by
//!   element (a mode's options in Maestro's order, a spec, a var's value), and the rest
//!   goes back exactly as Virtuoso wrote it.
//!
//! What Maestro holds and Schemify cannot express stays in the stored XML and is reported.

use super::mae_xml::{self, Node, Setup};
use super::*;

/// Maestro's Monte Carlo run mode.
pub const MC: &str = "Monte Carlo Sampling";
/// Maestro's global optimizer.
pub const GLOBAL: &str = "Global Optimization";
/// Maestro's local optimizer.
pub const LOCAL: &str = "Local Optimization";

/// One mapped option: (Maestro option, Schemify attribute, (Maestro, Schemify) value
/// words; empty: a decimal integer, written alike in both).
type OptMap = (
    &'static str,
    &'static str,
    &'static [(&'static str, &'static str)],
);

/// Monte Carlo options with a `montecarlo` attribute. A mode written from a record lists
/// its mapped options in table order (Maestro's own creation order in the probes), then
/// the extras in record order.
/// G49 (settled, live probe `l11_probe`): `lds` is `samplingmode`'s word for low-discrepancy
/// sampling; the netlist says `sampling=lds`, the run log `Low-Discrepancy Sequence`.
const MC_OPTS: [OptMap; 6] = [
    ("mcnumpoints", "samples", &[]),
    (
        "mcmethod",
        "variation",
        &[
            ("all", "all"),
            ("process", "process"),
            ("mismatch", "mismatch"),
        ],
    ),
    (
        "samplingmode",
        "sampling",
        &[("standard", "random"), ("lhs", "lhs"), ("lds", "lds")],
    ),
    ("montecarloseed", "seed", &[]),
    ("mcstartingrunnumber", "first", &[]),
    ("donominal", "nominal", &[("1", "true"), ("0", "false")]),
];

/// Optimizer options with an `optimize` attribute. The local optimizer's algorithm is its
/// `effort` (adexl's form: Brent-Powell `fine`, Hooke-Jeeves `coarse`). G50 (settled, live
/// probe `l11_probe`): on a two-variable bowl from one start, `fine` searches one variable
/// at a time (line minimization), `coarse` takes steps of 0.5 in both (a pattern search).
const OPT_OPTS: [OptMap; 2] = [
    (
        "effort",
        "algo",
        &[("fine", "brent-powell"), ("coarse", "hooke-jeeves")],
    ),
    ("numpoints", "budget", &[]),
];

/// The options table of `mode`'s mapped options (the global optimizer has no `effort`).
fn table(mode: &str) -> &'static [OptMap] {
    match mode {
        MC => &MC_OPTS,
        LOCAL => &OPT_OPTS,
        _ => &OPT_OPTS[1..],
    }
}

/// The `algo` of a mode with no algorithm option.
fn plain_algo(mode: &str) -> &'static str {
    if mode == GLOBAL { "global" } else { "local" }
}

/// The Maestro mode an `optimize` record's `algo` runs as.
fn algo_mode(algo: &str) -> Option<&'static str> {
    match algo {
        "global" => Some(GLOBAL),
        "local" | "brent-powell" | "hooke-jeeves" => Some(LOCAL),
        _ => None,
    }
}

/// The unknown attribute of a mode's unmapped options.
fn extra_key(mode: &str) -> &'static str {
    if mode == MC { "vmc" } else { "vopt" }
}

/// Header-free view of one record: kind, name, attributes in order.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Rec {
    kind: RecordKind,
    name: String,
    attrs: Vec<(String, Vec<String>)>,
}

/// A goal in Schemify words: the `goal` tail after the output, and the tolerance.
type Goal = (Vec<String>, Option<String>);

/// What a setup maps to.
#[derive(Debug, Default)]
struct Mapped {
    /// `montecarlo` and `optimize` records, in mode order.
    recs: Vec<Rec>,
    /// Goals by output (the mapped test's specs), in spec order.
    goals: Vec<(String, Goal)>,
    /// Optimizer ranges `(var, min, max, step)`, Schemify expressions.
    ranges: Vec<(String, String, String, String)>,
}

/// Whether `v` is a decimal integer in canonical form (`7`, not `07` or `+7`).
fn canonical_int(v: &str) -> bool {
    v.parse::<u64>().is_ok_and(|n| n.to_string() == v)
}

/// The Schemify attribute of Maestro option `(k, v)` under `table`, `None` when it has none.
fn opt_attr(table: &[OptMap], k: &str, v: &str) -> Option<(&'static str, String)> {
    let (_, attr, words) = table.iter().find(|o| o.0 == k)?;
    if words.is_empty() {
        return canonical_int(v).then(|| (*attr, v.to_owned()));
    }
    let (_, w) = words.iter().find(|w| w.0 == v)?;
    Some((*attr, (*w).to_owned()))
}

/// The Maestro option of Schemify attribute `(attr, v)` under `table`.
fn attr_opt(table: &[OptMap], attr: &str, v: &str) -> Option<(&'static str, String)> {
    let (k, _, words) = table.iter().find(|o| o.1 == attr)?;
    if words.is_empty() {
        return canonical_int(v).then(|| (*k, v.to_owned()));
    }
    let (m, _) = words.iter().find(|w| w.1 == v)?;
    Some((*k, (*m).to_owned()))
}

/// A Maestro value as a Schemify expression (the setup's globals bare), `None` without one.
fn from_maestro(v: &str, globals: &[String]) -> Option<String> {
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
    )?;
    Program::compile(&r.text, Mode::Scalar).ok()?;
    Some(r.text)
}

/// A Schemify expression as a Maestro value, `None` without one.
fn to_maestro(e: &str, vars: &[String]) -> Option<String> {
    ael::to_ael(e, |n| {
        if vars.iter().any(|v| v == n) {
            Name::BenchVar
        } else {
            Name::Unknown
        }
    })
    .ok()
    .map(|t| t.text)
}

/// The `optimize`/`montecarlo` record of one mode's options.
fn mode_rec(mode: &str, opts: &[(String, String)], cell: &str, notes: &mut Notes) -> Option<Rec> {
    let (kind, name) = match mode {
        MC => (RecordKind::Montecarlo, "mc"),
        GLOBAL => (RecordKind::Optimize, "global"),
        LOCAL => (RecordKind::Optimize, "local"),
        _ => {
            notes.review(
                "optimize",
                cell,
                format!("maestro run mode {mode}: no Schemify form (Maestro keeps it)"),
            );
            return None;
        }
    };
    let mut attrs: Vec<(String, Vec<String>)> = Vec::new();
    let mut extra = Vec::new();
    for (k, v) in opts {
        match opt_attr(table(mode), k, v) {
            Some((a, x)) if !attrs.iter().any(|y| y.0 == a) => attrs.push((a.into(), vec![x])),
            _ => extra.push((extra_key(mode).to_owned(), vec![k.clone(), v.clone()])),
        }
    }
    if kind == RecordKind::Optimize && !attrs.iter().any(|a| a.0 == "algo") {
        attrs.insert(0, ("algo".into(), vec![plain_algo(mode).into()]));
    }
    if kind == RecordKind::Montecarlo && !attrs.iter().any(|a| a.0 == "samples") {
        notes.review(
            "montecarlo",
            cell,
            format!("maestro {mode}: no mcnumpoints, so no montecarlo record (Maestro keeps the options)"),
        );
        return None;
    }
    attrs.extend(extra);
    Some(Rec {
        kind,
        name: name.into(),
        attrs,
    })
}

/// Maps the run modes of `setup` whose mapped test is `test`.
fn map(setup: &Setup, test: &str, cell: &str, notes: &mut Notes) -> Mapped {
    let globals: Vec<String> = setup.vars.iter().map(|v| v.name.clone()).collect();
    let mut m = Mapped::default();
    for rm in &setup.run_modes {
        if let Some(r) = mode_rec(&rm.mode, &rm.options, cell, notes) {
            m.recs.push(r);
        }
    }
    if !m.recs.iter().any(|r| r.kind == RecordKind::Optimize) {
        return m;
    }
    for sp in setup.specs.iter().filter(|s| s.test == test) {
        let x = |v: &str| from_maestro(v, &globals);
        let tail: Option<Vec<Option<String>>> = match sp.kind.as_str() {
            "gt" => Some(vec![Some("gt".into()), x(&sp.min)]),
            "lt" => Some(vec![Some("lt".into()), x(&sp.max)]),
            "range" => Some(vec![Some("range".into()), x(&sp.min), x(&sp.max)]),
            "min" | "max" => Some(vec![Some(sp.kind.clone())]),
            "tol" => Some(vec![Some("target".into()), x(&sp.target)]),
            _ => None, // info: a spec, not a goal
        };
        let Some(tail) = tail else { continue };
        let w = sp.weight.as_deref().map(x);
        match (tail.into_iter().collect::<Option<Vec<String>>>(), w) {
            (Some(mut t), w @ (None | Some(Some(_)))) => {
                if let Some(Some(w)) = w {
                    t.extend(["weight".to_owned(), w]);
                }
                m.goals.push((sp.output.clone(), (t, tol(sp))));
            }
            _ => notes.review(
                "spec",
                cell,
                format!("maestro spec {}: a bound or weight has no Schemify form, so no goal (Maestro keeps it)", sp.name),
            ),
        }
    }
    for v in &setup.vars {
        let parts: Vec<&str> = v.value.split(':').collect();
        if let [a, s, b] = parts[..]
            && let (Some(a), Some(s), Some(b)) = (
                from_maestro(a, &globals),
                from_maestro(s, &globals),
                from_maestro(b, &globals),
            )
        {
            m.ranges.push((v.name.clone(), a, b, s));
        } else if top_level_split(&v.value).len() > 1 {
            notes.review(
                "optimize",
                cell,
                format!("maestro var {} = {}: a value list as an optimizer domain has no Schemify bounds (Maestro keeps it)", v.name, v.value),
            );
        }
    }
    for p in &setup.params {
        if p.value.contains(':') || top_level_split(&p.value).len() > 1 {
            notes.review(
                "optimize",
                cell,
                format!("maestro parameter {}/{} = {}: varied as vary param; its range has no Schemify form (Maestro keeps it)", p.location, p.name, p.value),
            );
        }
    }
    let params: Vec<String> = setup
        .params
        .iter()
        .filter(|p| p.value.contains(':') || top_level_split(&p.value).len() > 1)
        .map(|p| {
            let inst = p.location.splitn(4, '/').nth(3).unwrap_or(&p.location);
            format!("{inst}/{}", p.name)
        })
        .collect();
    for r in m.recs.iter_mut().filter(|r| r.kind == RecordKind::Optimize) {
        let lines = &mut r.attrs;
        for (o, (tail, _)) in &m.goals {
            lines.push((
                "goal".into(),
                std::iter::once(o.clone()).chain(tail.clone()).collect(),
            ));
        }
        for (v, ..) in &m.ranges {
            lines.push(("vary".into(), vec![v.clone()]));
        }
        for p in &params {
            lines.push(("vary".into(), vec!["param".into(), p.clone()]));
        }
        for (o, (_, t)) in &m.goals {
            if let Some(t) = t {
                lines.push(("vtol".into(), vec![o.clone(), t.clone()]));
            }
        }
    }
    m
}

/// The tolerance of a target spec.
fn tol(sp: &mae_xml::SetupSpec) -> Option<String> {
    (sp.kind == "tol").then(|| sp.tol.clone())
}

/// Import side (spec §9.9): the run modes of the stored setup in `records` (one test's,
/// `maestro` first; `test` the mapped test) become `montecarlo`/`optimize` records of
/// `bench`, and each optimizer range var gains `min`, `max` and `step`. Nothing happens
/// without a `maesdb` record.
pub(super) fn import(
    records: &[Record],
    test: &str,
    bench: &mut Document<'static>,
    cell: &str,
    notes: &mut Notes,
) {
    let Some(xml) = records
        .iter()
        .find(|r| r.tag == "maesdb")
        .and_then(|r| r.text(0).ok())
    else {
        return;
    };
    let setup = match Setup::from_xml(&xml) {
        Ok(s) => s,
        Err(e) => {
            notes.review("code", cell, format!("maesdb: {e}: run modes not mapped"));
            return;
        }
    };
    let m = map(&setup, test, cell, notes);
    if m.recs.iter().any(|r| r.kind == RecordKind::Montecarlo) {
        notes.review(
            "montecarlo",
            cell,
            "montecarlo mc: the options cross, the draws do not (Spectre's sample k is not Schemify's)",
        );
    }
    if !m.ranges.is_empty() {
        let mut out = Document::new(bench.kind());
        crate::design::copy_header(&mut out, bench, &[]);
        for r in 1..bench.record_count() {
            out.copy_record(bench, r);
            if bench.record_kind(r) != RecordKind::Var {
                continue;
            }
            let name = bench.field(r, 0).unwrap_or("");
            if let Some((_, a, b, s)) = m.ranges.iter().find(|x| x.0 == name) {
                for (k, v) in [("min", a), ("max", b), ("step", s)] {
                    if bench.find_attr(r, k).is_none() {
                        push_attr(&mut out, k, &[v]);
                    }
                }
            }
        }
        assert_eq!(out.record_count(), bench.record_count());
        *bench = out;
    }
    let vars: Vec<String> = (1..bench.record_count())
        .filter(|&r| bench.record_kind(r) == RecordKind::Var)
        .filter_map(|r| bench.field(r, 0).map(str::to_owned))
        .collect();
    let has_var = |v: &str| vars.iter().any(|x| x == v);
    for (v, ..) in m.ranges.iter().filter(|x| !has_var(&x.0)) {
        notes.review(
            "optimize",
            cell,
            format!(
                "maestro var {v}: an optimizer range, but the var did not come back: not varied"
            ),
        );
    }
    for r in &m.recs {
        // The format's attribute order (format.md rank 6), the unknown ones last.
        let mut attrs = r.attrs.clone();
        attrs.retain(|a| !(a.0 == "vary" && a.1.len() == 1 && !has_var(&a.1[0])));
        attrs.sort_by_key(|a| {
            ATTR_ORDER
                .iter()
                .position(|k| *k == a.0)
                .unwrap_or(usize::MAX)
        });
        push_record(bench, r.kind, &[&r.name]);
        for (k, v) in &attrs {
            push_attr(bench, k, v);
        }
    }
}

/// The known attributes of `montecarlo` and `optimize`, in the format's order; an
/// unknown one sorts after them.
const ATTR_ORDER: [&str; 10] = [
    "samples",
    "seed",
    "first",
    "variation",
    "sampling",
    "nominal",
    "algo",
    "goal",
    "vary",
    "budget",
];

/// The record of bench cell `c` at `r`, in [`Rec`] form.
fn rec_of(c: &Cell, r: u32) -> Rec {
    Rec {
        kind: c.doc.record_kind(r),
        name: c.field(r, 0).to_owned(),
        attrs: c
            .doc
            .attrs(r)
            .map(|a| {
                (
                    c.doc.attr_key(a).to_owned(),
                    c.doc.attr_values(a).map(str::to_owned).collect(),
                )
            })
            .collect(),
    }
}

/// The Maestro mode and options of a bench record, `None` (reported) when it has none.
fn rec_mode(
    r: &Rec,
    cell: &str,
    notes: &mut Notes,
) -> Option<(&'static str, Vec<(String, String)>)> {
    let mode = if r.kind == RecordKind::Montecarlo {
        MC
    } else {
        let algo = r
            .attrs
            .iter()
            .find(|a| a.0 == "algo")
            .map_or("nelder-mead", |a| a.1[0].as_str());
        let Some(mode) = algo_mode(algo) else {
            notes.review(
                "optimize",
                cell,
                format!("optimize {}: algo {algo} has no Maestro run mode (maestro runs global, local, brent-powell, hooke-jeeves)", r.name),
            );
            return None;
        };
        mode
    };
    let table = table(mode);
    let mut opts: Vec<(String, String)> = Vec::new();
    let mut extra: Vec<(String, String)> = Vec::new();
    for (k, v) in &r.attrs {
        match (k.as_str(), &v[..]) {
            ("goal" | "vary" | "vtol", _) => {}
            ("algo", [a]) if a == plain_algo(mode) => {}
            (x, [a, b]) if x == extra_key(mode) => extra.push((a.clone(), b.clone())),
            (attr, [v]) => match attr_opt(table, attr, v) {
                Some((o, x)) => opts.push((o.into(), x)),
                None => notes.review(
                    if mode == MC { "montecarlo" } else { "optimize" },
                    cell,
                    format!(
                        "{} {} {attr} {v}: no Maestro form (stays in Schemify)",
                        if mode == MC { "montecarlo" } else { "optimize" },
                        r.name
                    ),
                ),
            },
            (attr, _) => notes.review(
                "optimize",
                cell,
                format!(
                    "optimize {} {attr}: no Maestro form (stays in Schemify)",
                    r.name
                ),
            ),
        }
    }
    opts.sort_by_key(|o| table.iter().position(|t| t.0 == o.0));
    opts.extend(
        extra
            .into_iter()
            .filter(|e| !opts.iter().any(|o| o.0 == e.0))
            .collect::<Vec<_>>(),
    );
    Some((mode, opts))
}

/// The options of `mode` set to `opts`: options `mode` already holds keep their place
/// (and their element when the value is the same), the others follow in `opts` order.
fn put_mode(root: &mut Node, mode: &str, opts: &[(String, String)]) {
    let active = root
        .child_mut("active")
        .expect("Setup::from_xml checked <active>");
    if active.child("runoptions").is_none() {
        active.children.push(Node::leaf("runoptions", ""));
    }
    let ro = active.child_mut("runoptions").expect("just made");
    if !ro
        .children
        .iter()
        .any(|m| m.name == "mode" && m.text == mode)
    {
        ro.children.push(Node::leaf("mode", mode));
    }
    let m = ro
        .children
        .iter_mut()
        .find(|m| m.name == "mode" && m.text == mode)
        .expect("just made");
    m.children
        .retain(|o| o.name != "runoption" || opts.iter().any(|x| x.0 == o.text));
    for (k, v) in opts {
        match m
            .children
            .iter_mut()
            .find(|o| o.name == "runoption" && o.text == *k)
        {
            Some(o) => set_child(o, "value", v),
            None => {
                let mut o = Node::leaf("runoption", k);
                o.children.push(Node::leaf("value", v));
                m.children.push(o);
            }
        }
    }
}

/// Sets child `name` of `n` to `text`, appending it when missing.
fn set_child(n: &mut Node, name: &str, text: &str) {
    match n.child_mut(name) {
        Some(c) => text.clone_into(&mut c.text),
        None => n.children.push(Node::leaf(name, text)),
    }
}

/// The spec of `output` in test `test` set to `goal`; Maestro's spelling of a bound is kept
/// when it still means the goal's.
fn put_spec(
    root: &mut Node,
    test: &str,
    output: &str,
    goal: &Goal,
    vars: &[String],
    cell: &str,
    notes: &mut Notes,
) {
    let (tail, tol) = goal;
    let (kind, rest) = tail.split_first().expect("a goal has a kind");
    let wpos = rest.iter().position(|x| x == "weight");
    let (bounds, weight) = match wpos {
        Some(k) => (&rest[..k], rest.get(k + 1)),
        None => (rest, None),
    };
    let ael = |e: &String, notes: &mut Notes| {
        let v = to_maestro(e, vars);
        if v.is_none() {
            notes.review(
                "spec",
                cell,
                format!("goal {output}: `{e}` has no Maestro form"),
            );
        }
        v
    };
    let Some(bounds) = bounds
        .iter()
        .map(|b| ael(b, notes))
        .collect::<Option<Vec<String>>>()
    else {
        return;
    };
    let weight = match weight.map(|w| ael(w, notes)) {
        Some(None) => return,
        w => w.flatten(),
    };
    // (specType, min, max, target); `None` leaves the field as Maestro has it.
    let fields: [Option<&str>; 4] = match (kind.as_str(), &bounds[..]) {
        ("gt", [b]) => [Some("gt"), Some(b), Some(""), Some("")],
        ("lt", [b]) => [Some("lt"), Some(""), Some(b), Some("")],
        ("range", [lo, hi]) => [Some("range"), Some(lo), Some(hi), Some("")],
        ("target", [t]) => [Some("tol"), Some(""), Some(""), Some(t)],
        ("min" | "max", []) => [Some(kind), None, None, None],
        _ => {
            notes.review(
                "spec",
                cell,
                format!("goal {output} {}: no Maestro spec form", tail.join(" ")),
            );
            return;
        }
    };
    let active = root
        .child_mut("active")
        .expect("Setup::from_xml checked <active>");
    if active.child("specs").is_none() {
        active.children.push(Node::leaf("specs", ""));
    }
    let specs = active.child_mut("specs").expect("just made");
    let name = format!("{test}.{output}");
    if !specs
        .children
        .iter()
        .any(|s| s.name == "spec" && s.text == name)
    {
        let mut s = Node::leaf("spec", &name);
        for (k, v) in [
            ("testname", test),
            ("resname", output),
            ("specType", ""),
            ("min", ""),
            ("max", ""),
            ("target", ""),
            ("tol", ""),
        ] {
            s.children.push(Node::leaf(k, v));
        }
        specs.children.push(s);
    }
    let s = specs
        .children
        .iter_mut()
        .find(|s| s.name == "spec" && s.text == name)
        .expect("just made");
    // Maestro's spelling (`1G`) of a value that still means the same is kept.
    let same = |old: &str, v: &str| {
        old == v || (!v.is_empty() && from_maestro(old, vars) == from_maestro(v, vars))
    };
    for (k, v) in ["specType", "min", "max", "target"].into_iter().zip(fields) {
        let Some(v) = v else { continue };
        if !s.get(k).is_some_and(|old| same(old, v)) {
            set_child(s, k, v);
        }
    }
    if kind != "min" && kind != "max" {
        let t = if kind == "target" {
            tol.clone()
                .or_else(|| s.get("tol").map(str::to_owned))
                .unwrap_or_default()
        } else {
            String::new()
        };
        set_child(s, "tol", &t);
    }
    match weight {
        Some(w) => {
            if !s.get("specWeight").is_some_and(|old| same(old, &w)) {
                set_child(s, "specWeight", &w);
            }
        }
        None => s.children.retain(|c| c.name != "specWeight"),
    }
}

/// Global var `name` set to `value`.
fn put_var(root: &mut Node, name: &str, value: &str) {
    let active = root
        .child_mut("active")
        .expect("Setup::from_xml checked <active>");
    if active.child("vars").is_none() {
        active.children.push(Node::leaf("vars", ""));
    }
    let vars = active.child_mut("vars").expect("just made");
    if !vars
        .children
        .iter()
        .any(|v| v.name == "var" && v.text == name)
    {
        vars.children.push(Node::leaf("var", name));
    }
    let v = vars
        .children
        .iter_mut()
        .find(|v| v.name == "var" && v.text == name)
        .expect("just made");
    set_child(v, "value", value);
}

/// Export side, after `with_stored` (spec §9.9): bench `c`'s `montecarlo` and `optimize`
/// records go into the stored setup's XML in `records`' `maesdb` record, where they
/// differ from what that XML imports as. Without a stored setup they are reported
/// (`needs_review: montecarlo`/`optimize`): only a view from Virtuoso carries the setup
/// database they are written into.
pub fn export_run_modes(c: &Cell, records: &mut [Record], notes: &mut Notes) {
    let cell = c.path.as_str();
    let mut recs: Vec<Rec> = c
        .records(RecordKind::Montecarlo)
        .chain(c.records(RecordKind::Optimize))
        .map(|r| rec_of(c, r))
        .collect();
    if recs.is_empty() {
        return;
    }
    let (Some(k), Some(test)) = (
        records.iter().position(|r| r.tag == "maesdb"),
        records
            .first()
            .and_then(|r| r.str(0).ok())
            .map(str::to_owned),
    ) else {
        for r in &recs {
            let what = if r.kind == RecordKind::Montecarlo {
                "montecarlo"
            } else {
                "optimize"
            };
            notes.review(
                what,
                cell,
                format!(
                    "{what} {}: no stored Maestro setup to carry it (§9.9)",
                    r.name
                ),
            );
        }
        return;
    };
    let xml = records[k].text(0).expect("parse checked maesdb");
    let (Ok(mut root), Ok(setup)) = (mae_xml::parse(&xml), Setup::from_xml(&xml)) else {
        notes.review(
            "code",
            cell,
            "maesdb: unreadable stored setup: run modes not sent",
        );
        return;
    };
    let base = map(&setup, &test, cell, &mut Notes::default());
    let vars: Vec<String> = c
        .records(RecordKind::Var)
        .map(|r| c.field(r, 0).to_owned())
        .collect();
    recs.sort_by_key(|r| r.kind != RecordKind::Montecarlo);
    // Modes.
    let mut done: Vec<&str> = Vec::new();
    let mut opt_recs: Vec<&Rec> = Vec::new();
    for r in &recs {
        let Some((mode, opts)) = rec_mode(r, cell, notes) else {
            continue;
        };
        if done.contains(&mode) {
            notes.review(
                if mode == MC { "montecarlo" } else { "optimize" },
                cell,
                format!(
                    "{} {}: a second record for {mode}: only the first maps",
                    if mode == MC { "montecarlo" } else { "optimize" },
                    r.name
                ),
            );
            continue;
        }
        done.push(mode);
        if r.kind == RecordKind::Optimize {
            opt_recs.push(r);
        }
        let mut have: Vec<(String, String)> = setup
            .run_mode(mode)
            .map_or(Vec::new(), |m| m.options.clone());
        let mut want = opts.clone();
        have.sort();
        want.sort();
        if have != want {
            put_mode(&mut root, mode, &opts);
        }
    }
    for b in &base.recs {
        let mode = match b.name.as_str() {
            "mc" => MC,
            "global" => GLOBAL,
            _ => LOCAL,
        };
        if !done.contains(&mode) {
            notes.review(
                if mode == MC { "montecarlo" } else { "optimize" },
                cell,
                format!("maestro {mode} has no record in the bench (removed?): Maestro keeps its options"),
            );
        }
    }
    // Goals, from the mapped optimize records (the first record holding an output wins).
    let mut goals: Vec<(String, Goal)> = Vec::new();
    let mut varied: Vec<String> = Vec::new();
    for r in &opt_recs {
        let tols: Vec<(&str, &str)> = r
            .attrs
            .iter()
            .filter(|a| a.0 == "vtol" && a.1.len() == 2)
            .map(|a| (a.1[0].as_str(), a.1[1].as_str()))
            .collect();
        for (k, v) in &r.attrs {
            match (k.as_str(), &v[..]) {
                ("goal", [o, tail @ ..]) if !tail.is_empty() => {
                    let t = tols.iter().find(|x| x.0 == o).map(|x| x.1.to_owned());
                    let g = (tail.to_vec(), t);
                    match goals.iter().find(|x| x.0 == *o) {
                        Some(x) if x.1 != g => notes.review(
                            "optimize",
                            cell,
                            format!("optimize {}: goal {o} differs from another record's; Maestro has one spec per output: the first is sent", r.name),
                        ),
                        Some(_) => {}
                        None => goals.push((o.clone(), g)),
                    }
                }
                ("vary", [v]) if !varied.contains(v) => varied.push(v.clone()),
                _ => {}
            }
        }
    }
    if !opt_recs.is_empty() {
        for (o, g) in &goals {
            if base.goals.iter().any(|b| &b.0 == o && &b.1 == g) {
                continue;
            }
            put_spec(&mut root, &test, o, g, &vars, cell, notes);
        }
        for (o, _) in &base.goals {
            if !goals.iter().any(|g| &g.0 == o) {
                notes.review(
                    "spec",
                    cell,
                    format!("maestro spec {test}.{o} is no goal of the bench: Maestro keeps it"),
                );
            }
        }
    }
    // Ranges of the varied vars.
    for v in &varied {
        let Some(r) = c.records(RecordKind::Var).find(|&r| c.field(r, 0) == v) else {
            continue; // the plan reports a vary of no var
        };
        let (Some(a), Some(b), Some(s)) = (c.attr(r, "min"), c.attr(r, "max"), c.attr(r, "step"))
        else {
            notes.review(
                "optimize",
                cell,
                format!("vary {v}: Maestro varies a var over min:step:max; it lacks one of them"),
            );
            continue;
        };
        if base
            .ranges
            .iter()
            .any(|x| &x.0 == v && (x.1.as_str(), x.2.as_str(), x.3.as_str()) == (a, b, s))
        {
            continue;
        }
        let parts: Option<Vec<String>> = [a, s, b].iter().map(|e| to_maestro(e, &vars)).collect();
        match parts {
            Some(p) => {
                if setup.var(v).is_none() {
                    notes.review("optimize", cell, format!("vary {v}: a var new in Schemify; its own value, sent after the setup, replaces the range"));
                }
                put_var(&mut root, v, &p.join(":"));
            }
            None => notes.review(
                "optimize",
                cell,
                format!("vary {v}: a bound has no Maestro form"),
            ),
        }
    }
    let patched = mae_xml::write(&root);
    if patched != xml {
        records[k].fields[0] = text(&patched);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_map_both_ways_or_ride_as_extras() {
        for (k, v) in [
            ("mcnumpoints", "7"),
            ("samplingmode", "standard"),
            ("donominal", "0"),
        ] {
            let (a, x) = opt_attr(&MC_OPTS, k, v).unwrap();
            assert_eq!(attr_opt(&MC_OPTS, a, &x), Some((k, v.to_owned())));
        }
        assert_eq!(opt_attr(&MC_OPTS, "mcnumpoints", "07"), None);
        assert_eq!(opt_attr(&MC_OPTS, "mcmethod", "bogus"), None);
        assert_eq!(opt_attr(&MC_OPTS, "saveallplots", "1"), None);
        let r = mode_rec(
            LOCAL,
            &[("effort".into(), "fine".into())],
            "c",
            &mut Notes::default(),
        )
        .unwrap();
        assert_eq!(
            r.attrs,
            [("algo".to_owned(), vec!["brent-powell".to_owned()])]
        );
    }
}
