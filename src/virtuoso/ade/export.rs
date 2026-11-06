//! A bench → its maestro tests' records ([`export`], spec `interop.md` §9.1): one test, or
//! one per `test` record; per test its analysis block's lines (SPICE or Spectre) →
//! analyses, model files, temperature and params; vars and sweeps, the test's own values;
//! corners; device parameters; outputs and specs.

use super::*;

/// Code languages whose text is SPICE (format.md rank 6).
const SPICE_LANGS: [&str; 3] = ["spice", "ngspice", "xyce"];

/// One bench's export: its maestro records and its OCEAN script (§9.6).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BenchExport {
    /// The maestro records in §9 order, test after test (each from its `maestro` record);
    /// empty for a bench with no simulation setup.
    pub records: Vec<Record>,
    /// The bench cell's name: the script's name and results directory.
    pub cell: String,
    /// The OCEAN script text; empty when `records` is.
    pub ocean: String,
    /// Definition files the script loads (`definitionFile`), `(file, text)`: the same
    /// files as the `defn` records, written beside the script under `<cell>/`.
    pub defns: Vec<(String, String)>,
}

/// One test's export: its table, its sweeps, outputs and plots as the OCEAN script takes
/// them.
struct TestExport {
    t: Test,
    swept: Vec<(String, String)>,
    ocean_outs: Vec<(String, String)>,
    plots: Vec<(String, String)>,
}

/// The maestro records and the OCEAN script of bench cell `ci` (§9.1, §9.6), each
/// `maestro` record naming Virtuoso library `vlib` (the plan's library of the cell) and the
/// cell's name. A bench without `test` records is one test; with them, one test per
/// record (by name), each running its `code` blocks and outputs, with its `set` lines as
/// its own var values (`testvar`), `enable false` as `testenable`, and `config` as its
/// design view. The bench's device parameters (`param`, corner `param`) go with the first
/// test. A bench whose header keeps the view's setup XML (§9.7) sends a test's enable and
/// a parameter only when they differ from it. `inst` answers the output translator's
/// instance questions ([`InstInfo`]). Bench content §9.1 does not map is reported in
/// `notes` (`needs_review:` `code sweep output spec plot expression design-variable
/// montecarlo optimize config corner`); the caller stashes it (§7.3).
///
/// # Panics
///
/// When `ci` is not a `.shbench` of `d`.
pub fn export<F: Fn(&str) -> Option<InstInfo>>(
    d: &Design,
    ci: usize,
    vlib: &str,
    inst: F,
    notes: &mut Notes,
) -> BenchExport {
    let c = &d.cells[ci];
    assert_eq!(
        c.kind(),
        FileKind::Shbench,
        "{}: maestro is a bench's",
        c.path
    );
    // A bench with no simulation setup (a schematic with no symbol, as a Virtuoso
    // testbench without maestro imports) gets no maestro view: none is created in Virtuoso.
    let setup = [
        RecordKind::Var,
        RecordKind::Corner,
        RecordKind::Sweep,
        RecordKind::Output,
        RecordKind::Spec,
        RecordKind::Code,
        RecordKind::Montecarlo,
        RecordKind::Optimize,
        RecordKind::Config,
        #[cfg(feature = "cadence-records")]
        RecordKind::Test,
        #[cfg(feature = "cadence-records")]
        RecordKind::BenchParam,
    ];
    if !setup.iter().any(|k| c.records(*k).next().is_some()) {
        return BenchExport::default();
    }
    let cell = c.path.as_str();
    let stored = c
        .attr_values(0, VMAESDB)
        .get(1)
        .and_then(|x| mae_xml::Setup::from_xml(x).ok());
    #[cfg(feature = "cadence-records")]
    let mut tests: Vec<u32> = c.records(RecordKind::Test).collect();
    #[cfg(not(feature = "cadence-records"))]
    let mut tests: Vec<u32> = Vec::new();
    tests.sort_by(|a, b| c.field(*a, 0).cmp(c.field(*b, 0)));
    let mut done: Vec<TestExport> = Vec::new();
    if tests.is_empty() {
        done.push(export_test(
            d,
            ci,
            vlib,
            &inst,
            None,
            stored.as_ref(),
            notes,
        ));
    }
    for (k, &r) in tests.iter().enumerate() {
        // Each test sees the bench's shared setup: a note about it is made once.
        let mut own = Notes::default();
        let mut te = export_test(d, ci, vlib, &inst, Some(r), stored.as_ref(), &mut own);
        if k > 0 {
            te.t.params.clear();
        }
        for n in own.rows {
            if !notes.rows.contains(&n) {
                notes.add(&n.class, &n.cell, n.detail);
            }
        }
        if done.iter().any(|x| x.t.name == te.t.name) {
            notes.review(
                "config",
                cell,
                format!(
                    "test {}: a second test of that maestro name; not sent",
                    te.t.name
                ),
            );
            continue;
        }
        done.push(te);
    }
    // With tests, a block or output none of them lists is no test's (format.md rank 6).
    for r in c.records(RecordKind::Code) {
        let b = c.field(r, 0);
        let listed = |t: u32| {
            let l = attr_words(c, t, "code");
            l.contains(&b) || (l.is_empty() && !attr_words(c, t, VNONE).contains(&"code"))
        };
        if !tests.is_empty() && !tests.iter().any(|&t| listed(t)) {
            notes.review(
                "code",
                cell,
                format!("code {b} {}: no test runs it; not sent", c.field(r, 1)),
            );
        }
    }
    for r in c.records(RecordKind::Output) {
        let o = c.field(r, 0);
        let listed = |t: u32| {
            let l = attr_words(c, t, "output");
            l.contains(&o) || (l.is_empty() && !attr_words(c, t, VNONE).contains(&"output"))
        };
        if !tests.is_empty() && !tests.iter().any(|&t| listed(t)) {
            notes.review(
                "output",
                cell,
                format!("output {o}: no test runs it; not sent"),
            );
            if c.records(RecordKind::Spec).any(|x| c.field(x, 0) == o) {
                notes.review("spec", cell, format!("spec {o}: no test runs its output"));
            }
        }
    }
    // The OCEAN script runs one test: the first enabled one.
    let pick = done
        .iter()
        .position(|x| x.t.enable != Some(false))
        .unwrap_or(0);
    if done.len() > 1 {
        notes.review(
            "config",
            cell,
            format!(
                "the OCEAN script runs test {} only (maestro has all {})",
                done[pick].t.name,
                done.len()
            ),
        );
    }
    let te = &done[pick];
    let default_corner = c
        .header("corner")
        .map(str::to_owned)
        .or_else(|| te.t.corners.first().map(|x| x.0.clone()));
    if te.t.corners.len() > 1 {
        notes.review(
            "corner",
            cell,
            format!(
                "the OCEAN script runs corner {} only (maestro has all {})",
                default_corner.as_deref().unwrap_or(""),
                te.t.corners.len()
            ),
        );
    }
    let ocean = ocean_script(
        &te.t,
        cell,
        &te.swept,
        &te.ocean_outs,
        &te.plots,
        default_corner.as_deref(),
        notes,
    );
    BenchExport {
        records: done.iter().flat_map(|x| x.t.records()).collect(),
        cell: te.t.cell.clone(),
        ocean,
        defns: te.t.defns.clone(),
    }
}

/// The words of every attribute `key` of record `r`, in order (`output a b`, `output c`).
fn attr_words<'c>(c: &'c Cell, r: u32, key: &str) -> Vec<&'c str> {
    c.doc
        .attrs(r)
        .filter(|&a| c.doc.attr_key(a) == key)
        .flat_map(|a| c.doc.attr_values(a))
        .collect()
}

/// One maestro test of bench `ci` (§9.1): the implicit one (`rec` `None`), else that of
/// `test` record `rec`; `stored` is the view's setup the bench header keeps (§9.7).
fn export_test<F: Fn(&str) -> Option<InstInfo>>(
    d: &Design,
    ci: usize,
    vlib: &str,
    inst: &F,
    rec: Option<u32>,
    stored: Option<&mae_xml::Setup>,
    notes: &mut Notes,
) -> TestExport {
    let c = &d.cells[ci];
    let cell = c.path.as_str();
    let vars: Vec<String> = c
        .records(RecordKind::Var)
        .map(|r| c.field(r, 0).to_owned())
        .collect();
    // The test's code blocks and outputs (default: every one of the bench).
    let none = |r: u32, key: &str| attr_words(c, r, VNONE).contains(&key);
    let codes_of = rec.map(|r| (attr_words(c, r, "code"), none(r, "code")));
    let outs_of = rec.map(|r| (attr_words(c, r, "output"), none(r, "output")));
    let listed = |of: &Option<(Vec<&str>, bool)>, name: &str| {
        of.as_ref()
            .is_none_or(|(l, none)| l.contains(&name) || (l.is_empty() && !none))
    };
    let runs = |name: &str| listed(&codes_of, name);
    let src = analysis_source(c, &runs);
    let name = match rec {
        Some(r) => safe_name(c.field(r, 0)),
        None => safe_name(src.map_or(c.name.as_str(), |r| c.field(r, 0))),
    };
    let mut t = Test {
        name,
        lib: vlib.to_owned(),
        cell: c.name.clone(),
        view: "schematic".into(),
        sim: "spectre".into(),
        ..Test::default()
    };
    if let Some(r) = rec {
        test_attrs(c, r, &vars, stored, &mut t, notes);
    }
    // Code blocks: the analysis block's lines, spectre text as definition files.
    let mut codes: Vec<u32> = c
        .records(RecordKind::Code)
        .filter(|&r| runs(c.field(r, 0)))
        .collect();
    codes.sort_by(|a, b| c.field(*a, 0).cmp(c.field(*b, 0)));
    for r in codes {
        let (name, lang) = (c.field(r, 0), c.field(r, 1));
        let lines: Vec<&str> = c.doc.verbatim(r).collect();
        if Some(r) == src {
            let spice = SPICE_LANGS.contains(&lang);
            // `.options errpreset=…` sets an option of the tran analysis: after the analyses.
            let mut order: Vec<usize> = (0..lines.len()).collect();
            order.sort_by_key(|&k| {
                spice && matches!(spice_line(lines[k]), Some(SpiceLine::ErrPreset(_)))
            });
            for k in order {
                let line = &lines[k];
                if source_line(line, spice, &vars, &mut t, cell, notes) || line.trim().is_empty() {
                    continue;
                }
                let at = format!("code {name} line {}: `{line}`", k + 1);
                let first = line.split_whitespace().next().unwrap_or("");
                if spice && first.eq_ignore_ascii_case(".title") {
                    // The netlist's title line: Spectre names the netlist after the design.
                    notes.dropped("code", cell, format!("{at}: Maestro has no title"));
                } else if spice && line.split_whitespace().any(|x| x.starts_with("monte=")) {
                    // HSPICE's `sweep monte=N`: Maestro's Monte Carlo is a run mode of the
                    // stored setup (a `montecarlo` record, §9.9) drawing Spectre's
                    // statistics, not HSPICE's.
                    notes.review(
                        "montecarlo",
                        cell,
                        format!(
                            "{at}: an HSPICE Monte Carlo sweep; a `montecarlo` record carries it"
                        ),
                    );
                } else {
                    notes.review("code", cell, format!("{at} has no maestro form"));
                }
            }
        } else if lang == "spectre" && !name.contains(['/', '\\']) {
            let mut body = lines.join("\n");
            body.push('\n');
            t.defns.push((format!("{name}.scs"), body));
        } else {
            let why = if src.is_some() && SPICE_LANGS.contains(&lang) {
                "only the analyses of the first analysis block map"
            } else {
                "no maestro form"
            };
            notes.review("code", cell, format!("code {name} {lang}: {why}"));
        }
    }
    // Vars and their sweeps.
    let mut swept: Vec<(String, String)> = Vec::new();
    let mut sweeps: Vec<u32> = c.records(RecordKind::Sweep).collect();
    sweeps.sort_by(|a, b| c.field(*a, 0).cmp(c.field(*b, 0)));
    for r in sweeps {
        let name = c.field(r, 0);
        let var = c.attr(r, "var").unwrap_or("");
        let points = if name != var || !vars.iter().any(|v| v == var) {
            Err("a sweep maps only when named after its bench var".to_owned())
        } else if swept.iter().any(|x| x.0 == var) {
            Err("a second sweep of the var".to_owned())
        } else {
            sweep_list(c, r, &vars)
        };
        match points {
            Ok(list) => swept.push((var.to_owned(), list)),
            Err(why) => notes.review("sweep", cell, format!("sweep {name}: {why}")),
        }
    }
    for r in c.records(RecordKind::Var) {
        let name = c.field(r, 0);
        // §9.11: `varenable` when disabled, or enabled again over a stored disabled var.
        let on = c.attr(r, "enable") != Some("false");
        if !on
            || stored
                .and_then(|su| su.var(name))
                .is_some_and(|v| !v.enabled)
        {
            t.var_enable.push((name.to_owned(), on));
        }
        if t.vars.iter().any(|v| v.0 == name && v.2.is_empty()) {
            notes.review(
                "code",
                cell,
                format!("var {name}: also set by the analysis block"),
            );
            continue;
        }
        let value = match swept.iter().find(|x| x.0 == name) {
            Some((_, list)) => Some(list.clone()),
            None => to_ael(c.field(r, 1), &vars, cell, notes),
        };
        if let Some(v) = value {
            t.vars.push((name.to_owned(), v, String::new()));
        }
    }
    // Corners.
    for r in c.records(RecordKind::Corner) {
        let corner = c.field(r, 0).to_owned();
        t.corners
            .push((corner.clone(), c.attr(r, "enable") != Some("false")));
        for a in c.doc.attrs(r) {
            let vals: Vec<&str> = c.doc.attr_values(a).collect();
            match (c.doc.attr_key(a), vals.as_slice()) {
                ("lib", [p, sec]) => t
                    .models
                    .push((corner.clone(), path_to_ade(p), (*sec).into())),
                ("include", [p]) => t
                    .models
                    .push((corner.clone(), path_to_ade(p), String::new())),
                ("set", [var, e]) => {
                    if let Some(v) = to_ael(e, &vars, cell, notes) {
                        t.vars.push(((*var).into(), v, corner.clone()));
                    }
                }
                ("temp", [e]) => {
                    if let Some(v) = to_ael(e, &vars, cell, notes) {
                        t.vars.push(("temperature".into(), v, corner.clone()));
                    }
                }
                ("param", [p, e]) => match (param_to_virtuoso(p), to_ael(e, &vars, cell, notes)) {
                    (Some(p), Some(v)) => t.params.push((p, v, corner.clone())),
                    (None, _) => notes.review(
                        "config",
                        cell,
                        format!("corner {corner} param {p}: no Virtuoso instance path"),
                    ),
                    _ => {}
                },
                ("enable", _) => {}
                (k, _) => notes.review(
                    "code",
                    cell,
                    format!("corner {corner}: `{k}` has no maestro form"),
                ),
            }
        }
    }
    // Outputs (maestro and OCEAN spellings) and their plots.
    let letter = signal_letter(&t.analyses);
    let mut scope = OutScope {
        vars: &vars,
        letter,
        inst,
        test: &t.name.clone(),
    };
    let mut ocean_outs: Vec<(String, String)> = Vec::new();
    let outputs: Vec<u32> = c
        .records(RecordKind::Output)
        .filter(|&r| listed(&outs_of, c.field(r, 0)))
        .collect();
    for &r in &outputs {
        let (name, e) = (c.field(r, 0), c.field(r, 1));
        // §9.11: Plot off (`out_flags` compares it with a stored state).
        if c.attr(r, "enable") == Some("false") {
            t.out_enable.push((name.to_owned(), false));
        }
        // `analysis <a>` picks the dataset (format.md rank 6), else the test's default.
        scope.letter = match c.attr(r, "analysis") {
            None => letter,
            Some(a) => match analysis_letter(a, &t.analyses) {
                Some(l) => l,
                None => {
                    notes.review(
                        "output",
                        cell,
                        format!("output {name}: analysis `{a}` is not one the test runs"),
                    );
                    continue;
                }
            },
        };
        // One spelling serves both: OCEAN evaluates ADE's expression functions.
        match out_expr(e, &scope) {
            Some(x) => {
                t.outputs.push((name.to_owned(), "point".into(), x.clone()));
                ocean_outs.push((name.to_owned(), x));
            }
            _ => notes.review(
                "output",
                cell,
                format!("output {name}: `{e}` has no ADE form"),
            ),
        }
    }
    let mut plots: Vec<(String, String)> = Vec::new();
    for &r in &outputs {
        let (name, Some(x)) = (c.field(r, 0), c.attr(r, "vs")) else {
            continue;
        };
        if !ocean_outs.iter().any(|o| o.0 == name) || !ocean_outs.iter().any(|o| o.0 == x) {
            notes.review(
                "plot",
                cell,
                format!("output {name} vs {x}: an output of the plot does not map"),
            );
            continue;
        }
        plots.push((name.to_owned(), x.to_owned()));
        notes.review(
            "plot",
            cell,
            format!(
                "output {name} vs {x}: plotted by the OCEAN script only (a maestro output is \
                 one value per point; the plot spans the sweep)"
            ),
        );
    }
    for r in c.records(RecordKind::Spec) {
        let out = c.field(r, 0);
        if !outputs.iter().any(|&o| c.field(o, 0) == out) {
            continue; // another test's
        }
        if !t.outputs.iter().any(|o| o.0 == out) {
            notes.review("spec", cell, format!("spec {out}: its output does not map"));
            continue;
        }
        for (key, op) in [("min", "gt"), ("max", "lt")] {
            let Some(e) = c.attr(r, key) else { continue };
            // A bound over bench vars (`min A0_spec/(1+A0_spec)`) is folded to its number
            // (an ADE spec target is a value); one over a swept var varies per point and has
            // no single target.
            let idents: Vec<String> = Program::compile(e, Mode::Scalar)
                .map(|p| p.idents().map(str::to_owned).collect())
                .unwrap_or_default();
            if idents.is_empty() {
                if let Some(v) = to_ael(e, &vars, cell, notes) {
                    t.specs.push((out.to_owned(), op.into(), v));
                }
            } else if let Some(s) = idents.iter().find(|n| swept.iter().any(|x| &x.0 == *n)) {
                notes.review(
                    "spec",
                    cell,
                    format!("spec {out} {key} `{e}`: depends on swept var {s}; not sent"),
                );
            } else if let Some(x) = cell_value(d, ci, e) {
                let mut v = String::new();
                write_num(&mut v, x);
                t.specs.push((out.to_owned(), op.into(), v));
            } else {
                notes.review(
                    "spec",
                    cell,
                    format!("spec {out} {key} `{e}`: does not evaluate over the bench vars"),
                );
            }
        }
    }
    // Device parameters: the bench's `param` records.
    #[cfg(feature = "cadence-records")]
    for r in c.records(RecordKind::BenchParam) {
        let (p, e) = (c.field(r, 0), c.field(r, 1));
        match (param_to_virtuoso(p), to_ael(e, &vars, cell, notes)) {
            (Some(p), Some(v)) => t.params.push((p, v, String::new())),
            (None, _) => notes.review(
                "config",
                cell,
                format!("param {p}: no Virtuoso instance path"),
            ),
            _ => {}
        }
    }
    // A view from Virtuoso keeps its parameters in the setup XML: only a changed or new one
    // is sent, and one the bench no longer has stays.
    if let Some(su) = stored {
        let mut old: Vec<(String, String, String)> = Vec::new();
        let scoped = su.params.iter().map(|p| (p, "")).chain(
            su.corners
                .iter()
                .flat_map(|k| k.params.iter().map(move |p| (p, k.name.as_str()))),
        );
        for (p, sc) in scoped {
            let loc = p.location.splitn(4, '/').nth(3).unwrap_or("");
            old.push((format!("{loc}/{}", p.name), p.value.clone(), sc.to_owned()));
        }
        for (p, _, sc) in &old {
            if !t.params.iter().any(|x| &x.0 == p && &x.2 == sc) {
                notes.review(
                    "config",
                    cell,
                    format!("maestro parameter {p} ({sc}) is not in the bench: Maestro keeps it"),
                );
            }
        }
        t.params.retain(|x| !old.contains(x));
    }
    for rows in [
        t.vars.len(),
        t.models.len(),
        t.outputs.len(),
        t.specs.len(),
        t.defns.len(),
        t.own.len(),
        t.params.len(),
    ] {
        assert!(
            rows <= ROWS_MAX,
            "{cell}: a bench holds at most 2^22 records"
        );
    }
    TestExport {
        t,
        swept,
        ocean_outs,
        plots,
    }
}

/// A bench `param` path (`inst[/inst…]/prop`, Schemify instance names) as Virtuoso's; `None`
/// when an instance name has no Virtuoso form.
fn param_to_virtuoso(p: &str) -> Option<String> {
    let (insts, prop) = p.rsplit_once('/')?;
    let insts: Vec<String> = insts
        .split('/')
        .map(crate::profile::inst_to_virtuoso)
        .collect::<Option<_>>()?;
    (!prop.is_empty()).then(|| format!("{}/{prop}", insts.join("/")))
}

/// The attributes of `test` record `r` onto `t` (format.md rank 6): `set` → its own var
/// values, `enable false` → `testenable` (with a stored setup, only when it differs),
/// `config` → its design view, `corner` → the corners it runs (`cornerrun`), `design` →
/// `testdesign`; `omit` has no record
/// and is reported unless the stored setup already holds it.
fn test_attrs(
    c: &Cell,
    r: u32,
    vars: &[String],
    stored: Option<&mae_xml::Setup>,
    t: &mut Test,
    notes: &mut Notes,
) {
    let cell = c.path.as_str();
    let name = t.name.clone();
    let mut omit: Vec<&str> = Vec::new();
    let mut enable = true;
    for a in c.doc.attrs(r) {
        let vals: Vec<&str> = c.doc.attr_values(a).collect();
        match (c.doc.attr_key(a), vals.as_slice()) {
            ("set", [v, e]) => {
                if let Some(x) = to_ael(e, vars, cell, notes) {
                    t.own.push(((*v).to_owned(), x));
                }
            }
            ("omit", [v]) => omit.push(v),
            ("enable", [e]) => enable = *e != "false",
            ("config", [k]) => {
                if c.records(RecordKind::Config).any(|x| c.field(x, 0) == *k) {
                    t.view = (*k).to_owned();
                } else {
                    notes.review(
                        "config",
                        cell,
                        format!("test {name} config {k}: no such config; runs the schematic"),
                    );
                }
            }
            ("code" | "output", _) => {}
            (VNONE, _) => {}
            // The corners it runs: the others disable the test (`maeSetCorner
            // ?disableTests`).
            ("corner", list) => {
                for k in c.records(RecordKind::Corner) {
                    let k = c.field(k, 0);
                    if !t.corner_runs.iter().any(|x| x.0 == k) {
                        t.corner_runs.push((k.to_owned(), list.contains(&k)));
                    }
                }
            }
            // Another cell as its design: `./cell` in the bench's library, else `lib:cell`.
            ("design", [r]) => match r
                .strip_prefix("./")
                .map(|x| (t.lib.as_str(), x))
                .or_else(|| r.split_once(':'))
            {
                Some((l, x)) if !l.is_empty() && !x.is_empty() && !x.contains('/') => {
                    t.design = Some((l.to_owned(), x.to_owned()));
                }
                _ => notes.review(
                    "config",
                    cell,
                    format!("test {name} design {r}: not a cell reference"),
                ),
            },
            (k, _) => notes.review(
                "config",
                cell,
                format!("test {name}: `{k}` has no maestro form"),
            ),
        }
    }
    // Without the test's own config, the bench's (header `config`) is its design.
    if !c.doc.attrs(r).any(|a| c.doc.attr_key(a) == "config")
        && let Some(k) = c.header("config")
        && c.records(RecordKind::Config).any(|x| c.field(x, 0) == k)
    {
        t.view = k.to_owned();
    }
    let was = stored.and_then(|su| su.test(&name));
    t.enable = match was {
        Some(x) if x.enabled == enable => None,
        None if enable => None,
        _ => Some(enable),
    };
    let mut before: Vec<String> = stored.map_or(Vec::new(), |su| {
        su.vars
            .iter()
            .filter(|v| v.dependents.iter().any(|d| d.0 == name && d.1))
            .map(|v| v.name.clone())
            .collect()
    });
    before.sort();
    omit.sort_unstable();
    if before != omit {
        notes.review(
            "design-variable",
            cell,
            format!(
                "test {name} omit {}: not sent (maestro keeps {})",
                omit.join(" "),
                if before.is_empty() {
                    "none omitted".to_owned()
                } else {
                    before.join(" ")
                }
            ),
        );
    }
}

/// The code block whose analyses map: the first (by name) SPICE block with a line that is
/// an analysis, or spectre block whose every line but comments has a maestro form and one
/// is an analysis (the import's own Spectre form, §9.2), among the blocks `runs` takes.
fn analysis_source(c: &Cell, runs: &dyn Fn(&str) -> bool) -> Option<u32> {
    let mut codes: Vec<u32> = c
        .records(RecordKind::Code)
        .filter(|&r| runs(c.field(r, 0)))
        .collect();
    codes.sort_by(|a, b| c.field(*a, 0).cmp(c.field(*b, 0)));
    codes.into_iter().find(|&r| {
        let lines: Vec<&str> = c.doc.verbatim(r).collect();
        match c.field(r, 1) {
            l if SPICE_LANGS.contains(&l) => lines
                .iter()
                .any(|x| matches!(spice_line(x), Some(SpiceLine::Analysis(..)))),
            "spectre" => {
                lines
                    .iter()
                    .any(|x| matches!(spectre_line(x), Some(SpectreLine::Analysis(..))))
                    && lines.iter().all(|x| {
                        let t = x.trim();
                        t.is_empty() || t.starts_with("//") || spectre_line(x).is_some()
                    })
            }
            _ => false,
        }
    })
}

/// A Virtuoso-safe test name: `[A-Za-z0-9_]`, others `_`.
fn safe_name(n: &str) -> String {
    let out: String = n
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect();
    if out.is_empty() { "test".into() } else { out }
}

/// Adds the maestro form of one line of the analysis block to `t`; false when it has none.
fn source_line(
    line: &str,
    spice: bool,
    vars: &[String],
    t: &mut Test,
    cell: &str,
    notes: &mut Notes,
) -> bool {
    let parsed = if spice {
        spice_line(line)
    } else {
        spectre_line(line).map(|l| match l {
            SpectreLine::Analysis(n, o) => SpiceLine::Analysis(n, o),
            SpectreLine::Model(f, sec) => SpiceLine::Model(f, sec),
            SpectreLine::Temp(v) => SpiceLine::Temp(v),
            SpectreLine::SimOpts(o) => SpiceLine::SimOpts(o),
        })
    };
    let has_global = |t: &Test, n: &str| t.vars.iter().any(|v| v.0 == n && v.2.is_empty());
    match parsed {
        None => false,
        Some(SpiceLine::ErrPreset(p)) => match t.analyses.iter_mut().find(|a| a.0 == "tran") {
            Some(a) if !a.2.iter().any(|o| o.0 == "errpreset") => {
                a.2.push(kv("errpreset", &p));
                true
            }
            _ => false,
        },
        Some(SpiceLine::Analysis(name, opts)) => {
            let op_only = |o: &[(String, String)]| o.len() == 1 && o[0].0 == "saveOppoint";
            match t.analyses.iter_mut().find(|a| a.0 == name) {
                None => {
                    t.analyses.push((name, true, opts));
                    true
                }
                // `.op` and `.dc` share ADE's one dc analysis.
                Some(a) if name == "dc" && (op_only(&a.2) != op_only(&opts)) => {
                    if op_only(&a.2) {
                        let keep = std::mem::take(&mut a.2);
                        a.2 = opts;
                        a.2.extend(keep);
                    } else {
                        a.2.extend(opts);
                    }
                    true
                }
                Some(_) => false,
            }
        }
        Some(SpiceLine::Model(f, sec)) => {
            t.models.push((String::new(), path_to_ade(&f), sec));
            true
        }
        Some(SpiceLine::Temp(v))
            if !has_global(t, "temperature") && !vars.iter().any(|x| x == "temperature") =>
        {
            let v = if spice {
                to_ael(&v, vars, cell, notes)
            } else {
                Some(v)
            };
            v.map(|v| t.vars.push(("temperature".into(), v, String::new())))
                .is_some()
        }
        Some(SpiceLine::Temp(_)) => false,
        Some(SpiceLine::SimOpts(o)) => {
            if o.iter().any(|(k, _)| t.sim_opts.iter().any(|x| &x.0 == k)) {
                return false;
            }
            t.sim_opts.extend(o);
            true
        }
        Some(SpiceLine::Param(kvs)) => {
            if kvs
                .iter()
                .any(|(k, _)| has_global(t, k) || vars.iter().any(|x| x == k))
            {
                return false;
            }
            let mut rows = Vec::new();
            for (k, e) in kvs {
                match to_ael(&e, vars, cell, notes) {
                    Some(v) => rows.push((k, v, String::new())),
                    None => return false,
                }
            }
            t.vars.extend(rows);
            true
        }
    }
}

/// What one line of an analysis block means.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum SpiceLine {
    /// `.options errpreset=<preset>`: the transient analysis's accuracy preset.
    ErrPreset(String),
    /// An ADE analysis and its options (values already AEL).
    Analysis(String, Vec<(String, String)>),
    /// A model file and section (`""` for `.include`).
    Model(String, String),
    /// The temperature (Schemify expression for SPICE, AEL for Spectre lines).
    Temp(String),
    /// `.param k=v …` (Schemify expressions).
    Param(Vec<(String, String)>),
    /// Simulator options `(Spectre name, value)`: `.options reltol=…`, Spectre
    /// `<name> options k=v …`.
    SimOpts(Vec<(String, String)>),
}

/// AC/noise sweep options of `dec|lin n f1 f2` (values SPICE numbers). ngspice's `lin n` is
/// n points, Spectre's `lin=n` n steps (n+1 points, live): `lin n` is `lin=n-1` (`lin=0` the
/// one point of start = stop; `lin=1` there is Spectre's SPECTRE-16108, live).
fn freq_opts(ty: &str, n: &str, f1: &str, f2: &str) -> Option<Vec<(String, String)>> {
    let (f1, f2) = (ael::number_to_ael(f1)?, ael::number_to_ael(f2)?);
    let n = match ty.to_ascii_lowercase().as_str() {
        "lin" => (n.parse::<u32>().ok().filter(|n| *n >= 1)? - 1).to_string(),
        _ => ael::number_to_ael(n)?,
    };
    let mut o = vec![kv("start", &f1), kv("stop", &f2)];
    match ty.to_ascii_lowercase().as_str() {
        "dec" => o.extend([
            kv("incrType", "Logarithmic"),
            kv("stepTypeLog", "Points Per Decade"),
            kv("dec", &n),
        ]),
        "lin" => o.extend([
            kv("incrType", "Linear"),
            kv("stepTypeLin", "Number of Steps"),
            kv("lin", &n),
        ]),
        _ => return None,
    }
    Some(o)
}

/// The maestro meaning of one SPICE line, `None` for lines §9.1 does not map (anything
/// that does not parse exactly: extra words such as `uic`, expressions, two-source `.dc`).
fn spice_line(line: &str) -> Option<SpiceLine> {
    let w: Vec<&str> = line.split_whitespace().collect();
    let num = |x: &str| ael::number_to_ael(x);
    let dir = w.first()?.to_ascii_lowercase();
    Some(match (dir.as_str(), &w[1..]) {
        (".tran", [_, stop, rest @ ..]) if rest.len() <= 2 => {
            let mut o = vec![kv("stop", &num(stop)?)];
            if let Some(st) = rest.first() {
                o.push(kv("start", &num(st)?));
            }
            if let Some(m) = rest.get(1) {
                o.push(kv("maxstep", &num(m)?));
            }
            num(w[1])?;
            SpiceLine::Analysis("tran".into(), o)
        }
        (".ac", [ty, n, f1, f2]) => SpiceLine::Analysis("ac".into(), freq_opts(ty, n, f1, f2)?),
        // ICADVM20.1's dc incrType default is Automatic, which netlists no step (live):
        // the step needs Linear by step size.
        (".dc", [src, a, b, st]) if is_ident(src) => SpiceLine::Analysis(
            "dc".into(),
            vec![
                kv("sweep", "Component Parameter"),
                kv("dev", &format!("/{src}")),
                kv("param", "dc"),
                kv("start", &ael::signed_number(a)?),
                kv("stop", &ael::signed_number(b)?),
                kv("incrType", "Linear"),
                kv("stepTypeLin", "Step Size"),
                kv("step", &ael::signed_number(st)?),
            ],
        ),
        (".op", []) => SpiceLine::Analysis("dc".into(), vec![kv("saveOppoint", "t")]),
        // A voltage output is ADE's `outType voltage` with nodes `p`/`n` (`oprobe` names a
        // probe instance: Spectre SFE-1997 on a net, live); ground is `/gnd!`.
        (".noise", [out, src, ty, n, f1, f2]) if is_ident(src) => {
            let nodes = out
                .strip_prefix("v(")
                .or_else(|| out.strip_prefix("V("))
                .and_then(|x| x.strip_suffix(')'))?;
            let (p, r) = nodes.split_once(',').unwrap_or((nodes, "gnd!"));
            let r = if r == "0" { "gnd!" } else { r };
            if !is_ident(p) || !(r == "gnd!" || is_ident(r)) {
                return None;
            }
            let mut o = freq_opts(ty, n, f1, f2)?;
            o.push(kv("outType", "voltage"));
            o.push(kv("p", &format!("/{p}")));
            o.push(kv("n", &format!("/{r}")));
            o.push(kv("iprobe", &format!("/{src}")));
            SpiceLine::Analysis("noise".into(), o)
        }
        (".temp", [t]) => SpiceLine::Temp((*t).to_owned()),
        (".param", kvs) if !kvs.is_empty() => {
            let mut out = Vec::new();
            for p in kvs {
                let (k, v) = p.split_once('=')?;
                if !is_ident(k) || v.is_empty() || Program::compile(v, Mode::Scalar).is_err() {
                    return None;
                }
                out.push((k.to_owned(), v.to_owned()));
            }
            SpiceLine::Param(out)
        }
        (".options" | ".option", [one]) if one.starts_with("errpreset=") => {
            let p = one.strip_prefix("errpreset=")?;
            SpiceLine::ErrPreset(
                ERRPRESETS
                    .iter()
                    .find(|x| x.eq_ignore_ascii_case(p))?
                    .to_string(),
            )
        }
        // Simulator options: those with a Spectre option of the same meaning.
        (".options" | ".option", kvs) if !kvs.is_empty() => {
            let mut out = Vec::new();
            for p in kvs {
                let (k, v) = p.split_once('=')?;
                let o = NGSPICE_OPTIONS
                    .iter()
                    .find(|o| o.1.eq_ignore_ascii_case(k))?;
                out.push(kv(o.0, &ael::number_to_ael(v)?));
            }
            SpiceLine::SimOpts(out)
        }
        (".include" | ".inc", [f]) => SpiceLine::Model(unquote_path(f), String::new()),
        (".lib", [f, sec]) => SpiceLine::Model(unquote_path(f), (*sec).to_owned()),
        _ => return None,
    })
}

fn unquote_path(f: &str) -> String {
    f.strip_prefix('"')
        .and_then(|x| x.strip_suffix('"'))
        .unwrap_or(f)
        .to_owned()
}

/// What one line of a Spectre analysis block (the import's own form, §9.2) means.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum SpectreLine {
    /// `<kind> <kind> k=v …`.
    Analysis(String, Vec<(String, String)>),
    /// `include "<file>" [section=<s>]`.
    Model(String, String),
    /// `temp options temp=<t>`.
    Temp(String),
    /// `<name> options k=v …`: simulator options.
    SimOpts(Vec<(String, String)>),
}

/// Splits a Spectre line into words; a `"…"` (with `\"` and `\\`) may sit inside a word.
pub(super) fn spectre_words(line: &str) -> Option<Vec<String>> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut started = false;
    let mut it = line.chars();
    while let Some(ch) = it.next() {
        match ch {
            '"' => {
                started = true;
                loop {
                    match it.next()? {
                        '"' => break,
                        '\\' => cur.push(it.next()?),
                        o => cur.push(o),
                    }
                }
            }
            ch if ch.is_whitespace() => {
                if started {
                    out.push(std::mem::take(&mut cur));
                    started = false;
                }
            }
            ch => {
                started = true;
                cur.push(ch);
            }
        }
    }
    if started {
        out.push(cur);
    }
    Some(out)
}

fn spectre_line(line: &str) -> Option<SpectreLine> {
    let w = spectre_words(line)?;
    let opts = |rest: &[String]| -> Option<Vec<(String, String)>> {
        rest.iter()
            .map(|p| {
                p.split_once('=')
                    .map(|(k, v)| kv(k, v))
                    .filter(|(k, _)| is_ident(k))
            })
            .collect()
    };
    match w.as_slice() {
        [inc, f, rest @ ..] if inc == "include" && rest.len() <= 1 => {
            let sec = match rest.first() {
                Some(x) => x.strip_prefix("section=")?.to_owned(),
                None => String::new(),
            };
            Some(SpectreLine::Model(f.clone(), sec))
        }
        [a, b, t] if a == "temp" && b == "options" => {
            Some(SpectreLine::Temp(t.strip_prefix("temp=")?.to_owned()))
        }
        [a, b, rest @ ..] if b == "options" && a != "temp" && is_ident(a) && !rest.is_empty() => {
            let o = opts(rest)?;
            o.iter()
                .all(|(k, _)| is_spectre_param("options", k))
                .then_some(SpectreLine::SimOpts(o))
        }
        // `<kind> [(<p> <n>)] <kind> k=v …`, the import's form (forms.rs).
        [a, rest @ ..] if is_ident(a) => {
            let (nodes, rest) = match rest.first() {
                Some(w) if w.starts_with('(') => {
                    let end = rest.iter().position(|w| w.ends_with(')'))?;
                    let inner = rest[..=end].join(" ");
                    let nodes: Vec<String> = inner[1..inner.len() - 1]
                        .split_whitespace()
                        .map(str::to_owned)
                        .collect();
                    (nodes, &rest[end + 1..])
                }
                _ => (Vec::new(), rest),
            };
            match rest {
                [b, params @ ..] if b == a => Some(SpectreLine::Analysis(
                    a.clone(),
                    from_spectre(a, &nodes, &opts(params)?),
                )),
                _ => None,
            }
        }
        _ => None,
    }
}

/// A bench value as AEL (§3.4: bench vars bare), its review reason noted; `None` (noted
/// `expression`) when it has no AEL form.
fn to_ael(e: &str, vars: &[String], cell: &str, notes: &mut Notes) -> Option<String> {
    let r = ael::to_ael(e, |n| {
        if vars.iter().any(|v| v == n) {
            Name::BenchVar
        } else {
            Name::Unknown
        }
    });
    match r {
        Ok(t) => {
            if let Some(why) = t.review {
                notes.review(why, cell, format!("`{e}` → `{}`", t.text));
            }
            Some(t.text)
        }
        Err(why) => {
            notes.review(why, cell, format!("`{e}` has no AEL form"));
            None
        }
    }
}

/// The comma list of a sweep's points (run.md §2): `values` entries through §3.4, `lin`/
/// `log` points as canonical number text.
fn sweep_list(c: &Cell, r: u32, vars: &[String]) -> Result<String, String> {
    let shape = |k: &str| {
        c.doc
            .find_attr(r, k)
            .map(|a| c.doc.attr_values(a).collect::<Vec<_>>())
    };
    let points: Vec<String> = match (shape("values"), shape("lin"), shape("log")) {
        (Some(v), None, None) => {
            let mut out = Vec::new();
            for e in v {
                let t = ael::to_ael(e, |n| {
                    if vars.iter().any(|x| x == n) {
                        Name::BenchVar
                    } else {
                        Name::Unknown
                    }
                })
                .map_err(|_| format!("`{e}` has no AEL form"))?;
                if t.review.is_some() || top_level_split(&t.text).len() != 1 {
                    return Err(format!("`{e}` has no comma-list form"));
                }
                out.push(t.text);
            }
            out
        }
        (None, Some(a), None) | (None, None, Some(a)) if a.len() == 3 => {
            let bound = |x: &str| {
                ael::fold_numeric(x)
                    .and_then(|t| t.parse::<f64>().ok())
                    .ok_or(format!("bound `{x}` is not a constant"))
            };
            let n: u32 = a[2]
                .parse()
                .ok()
                .filter(|n| (1..=SWEEP_POINTS_MAX).contains(n))
                .ok_or("count out of range")?;
            grid(bound(a[0])?, bound(a[1])?, n, shape("log").is_some())
                .ok_or("log bounds must be nonzero with one sign")?
        }
        _ => return Err("needs exactly one of values, lin, log".into()),
    };
    if points.len() < 2 {
        return Err("one point is the var's value, not a sweep".into());
    }
    Ok(points.join(","))
}
