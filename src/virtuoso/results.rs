//! A maestro history → one imported Schemify run (spec `interop.md` §4.2 `history`/`job`,
//! §9.10; Schemify run.md §4 imported runs, waves.md §9).
//!
//! Spine: the dump `SfyAdeResults` wrote ([`parse`]: the `history` record and one `job`
//! per simulated point, naming its waveform export) → the history's `.rdb` beside it
//! ([`convert`], read by `rdb`: tests, corners, points, var values, output values, spec
//! verdicts and statuses, every double bit for bit) → [`History`], the jobs in run order →
//! [`results_file`] (the run's `results` text) and per job its waveforms → `out.raw`
//! (`wave::raw`). The CLI (`maestro-results`) adds the design hash and writes the files.
//!
//! What comes across: the first test of the history (by name, as `ade::import` maps the
//! first test of a view), each (corner, design point) a job — Nominal as the bench
//! default, a named corner as `corner <name>`, a var that takes more than one value across
//! a corner's design points as a `set`. Reported, not mapped: the other tests, outputs
//! reduced across corners or sweeps, Monte Carlo statistics, and Maestro's `near` verdict
//! (outside the spec by at most 10%), which is a `fail` here.

use std::collections::BTreeMap;

use crate::sfyx::{self, Record};

pub mod rdb;
pub mod wave;

use rdb::Val;

/// Jobs per history (run.md's job bound).
pub const JOBS_MAX: usize = 1 << 20;

/// The dump's `history` record and its `job` records.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Dump {
    /// Maestro view (lib, cell, view).
    pub view: (String, String, String),
    /// The history name (`Interactive.0`).
    pub name: String,
    /// The `.rdb` copy, relative to the dump.
    pub rdb: String,
    /// The history's own setup (`history/<name>.zip`) copy, relative to the dump; `""`
    /// when there is none.
    pub setup: String,
    /// `(test, corner, design point) → (psfdir, waves file relative to the dump or "")`.
    pub points: BTreeMap<(String, String, i64), (String, String)>,
}

/// One job of the imported run.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Job {
    /// The corner; `None` for Nominal (the bench default).
    pub corner: Option<String>,
    /// The `.rdb` design point number.
    pub point: i64,
    /// `(var, value text)`: the vars swept across design points, by name.
    pub sets: Vec<(String, String)>,
    /// Maestro's status message for the point (`done` when it simulated).
    pub status: String,
    /// Var values that are numbers, by name.
    pub vars: BTreeMap<String, f64>,
    /// Output values, by output name.
    pub values: BTreeMap<String, f64>,
    /// Output errors, by output name.
    pub errors: BTreeMap<String, String>,
    /// Spec verdicts (`true` pass), by output name.
    pub pass: BTreeMap<String, bool>,
    /// The waveform export, relative to the dump; `""` when the point has none.
    pub waves: String,
}

/// A converted history.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct History {
    /// The dump's records.
    pub dump: Dump,
    /// The test that maps.
    pub test: String,
    /// Jobs in run order: Nominal first, then corners by name; design points ascending.
    pub jobs: Vec<Job>,
    /// What did not come across, one line each.
    pub notes: Vec<String>,
}

/// Reads the dump `SfyAdeResults` wrote: `sfyx`, one `history`, `job`s, `end`.
///
/// # Errors
///
/// A malformed record (with its line), no `history`, two of them, or a point twice.
pub fn parse(text: &str) -> Result<Dump, String> {
    let recs = sfyx::read(text).map_err(|e| e.to_string())?;
    sfyx::require_complete(&recs).map_err(|e| e.to_string())?;
    let mut d: Option<Dump> = None;
    let err = |r: &Record, m: &str| r.err(m).to_string();
    for r in &recs[1..recs.len() - 1] {
        match r.tag.as_str() {
            "history" => {
                r.arity(6).map_err(|e| e.to_string())?;
                if d.is_some() {
                    return Err(err(r, "a second history"));
                }
                let s = |k| r.str(k).map(str::to_owned).map_err(|e| e.to_string());
                d = Some(Dump {
                    view: (s(0)?, s(1)?, s(2)?),
                    name: s(3)?,
                    rdb: s(4)?,
                    setup: s(5)?,
                    points: BTreeMap::new(),
                });
            }
            "job" => {
                r.arity(5).map_err(|e| e.to_string())?;
                let d = d
                    .as_mut()
                    .ok_or_else(|| err(r, "a job before the history"))?;
                let s = |k| r.str(k).map(str::to_owned).map_err(|e| e.to_string());
                let key = (
                    s(0)?,
                    s(1)?,
                    i64::from(r.int(2).map_err(|e| e.to_string())?),
                );
                if d.points.len() >= JOBS_MAX || d.points.insert(key, (s(3)?, s(4)?)).is_some() {
                    return Err(err(r, "a point twice, or more than 2^20"));
                }
            }
            t => return Err(err(r, &format!("`{t}` in a results dump"))),
        }
    }
    d.ok_or_else(|| "no history record".into())
}

/// The history in `rdb` (the `.rdb` bytes) as jobs of its first test, with the dump's
/// waveform exports attached.
///
/// # Errors
///
/// The `.rdb` does not read, or holds no test or no point.
pub fn convert(dump: &Dump, rdb: &[u8]) -> Result<History, String> {
    let db = rdb::open(rdb)?;
    let t = |n: &str| db.table(n);
    let id = |v: &Val| v.int().unwrap_or(-1);
    let tests: BTreeMap<String, i64> = t("test")?
        .iter()
        .map(|r| (r[1].text().to_owned(), id(&r[0])))
        .collect();
    let (test, &tid) = tests.iter().next().ok_or("the .rdb holds no test")?;
    let mut notes: Vec<String> = tests
        .keys()
        .skip(1)
        .map(|n| format!("test {n}: only the first test of a history maps"))
        .collect();
    let corners: BTreeMap<i64, String> = t("corner")?
        .iter()
        .map(|r| (id(&r[0]), r[1].text().to_owned()))
        .collect();
    // pointID → (corner name, design point)
    let points: BTreeMap<i64, (String, i64)> = t("point")?
        .iter()
        .map(|r| {
            let c = corners.get(&id(&r[3])).cloned().unwrap_or_default();
            (id(&r[0]), (c, id(&r[1])))
        })
        .collect();
    let messages: BTreeMap<i64, String> = t("error")?
        .iter()
        .map(|r| (id(&r[0]), r[1].text().to_owned()))
        .collect();
    let message = |v: &Val| {
        v.int()
            .map(|e| messages.get(&e).cloned().unwrap_or_default())
    };

    // Var values: the point's, unless the test overrides the var (a non-empty default).
    let params = t("parameter")?;
    let desvars: BTreeMap<i64, String> = params
        .iter()
        .filter(|r| r[9].text() == "desVar" && r[1].text() != "temperature")
        .map(|r| (id(&r[0]), r[1].text().to_owned()))
        .collect();
    let overrides: BTreeMap<i64, Val> = t("testDefaultParameterValue")?
        .into_iter()
        .filter(|r| id(&r[1]) == tid && !matches!(&r[2], Val::Text(s) if s.is_empty()))
        .map(|r| (id(&r[0]), r[2].clone()))
        .collect();
    let mut var_text: BTreeMap<i64, BTreeMap<String, String>> = BTreeMap::new();
    for r in t("parameterValue")? {
        let Some(name) = desvars.get(&id(&r[1])) else {
            continue;
        };
        let v = overrides.get(&id(&r[1])).unwrap_or(&r[2]);
        let text = match v {
            Val::Int(i) => i.to_string(),
            Val::Real(x) => num(*x),
            Val::Text(s) => s.clone(),
            Val::Null | Val::Blob(_) => continue,
        };
        var_text
            .entry(id(&r[0]))
            .or_default()
            .insert(name.clone(), text);
    }
    // A var is swept when, within one corner, two design points give it different text.
    let mut seen: BTreeMap<(&str, &str), &str> = BTreeMap::new();
    let mut swept: Vec<&str> = Vec::new();
    for (pid, vars) in &var_text {
        let corner = points.get(pid).map_or("", |p| p.0.as_str());
        for (n, v) in vars {
            match seen.insert((corner, n), v) {
                Some(old) if old != v && !swept.contains(&n.as_str()) => swept.push(n),
                _ => {}
            }
        }
    }

    let results: BTreeMap<i64, String> = t("result")?
        .iter()
        .filter(|r| id(&r[2]) == tid && r[3].text() == "0")
        .map(|r| (id(&r[0]), r[1].text().to_owned()))
        .collect();
    let specs: BTreeMap<i64, String> = t("spec")?
        .iter()
        .filter_map(|r| Some((id(&r[0]), results.get(&id(&r[1]))?.clone())))
        .collect();

    let mut jobs: BTreeMap<(Option<String>, i64), (i64, Job)> = BTreeMap::new();
    for r in t("testStatus")? {
        if id(&r[1]) != tid {
            continue;
        }
        let pid = id(&r[0]);
        let Some((corner, dp)) = points.get(&pid) else {
            continue;
        };
        let vars = var_text.get(&pid).cloned().unwrap_or_default();
        let mut sets: Vec<(String, String)> = swept
            .iter()
            .filter_map(|n| Some((n.to_string(), schemify_text(vars.get(*n)?))))
            .collect();
        sets.sort();
        let key = (dump.points.get(&(test.clone(), corner.clone(), *dp)))
            .map(|p| p.1.clone())
            .unwrap_or_default();
        let job = Job {
            corner: (!corner.is_empty()).then(|| corner.clone()),
            point: *dp,
            sets,
            status: message(&r[6]).unwrap_or_else(|| "no status".into()),
            vars: vars
                .iter()
                .filter_map(|(n, v)| Some((n.clone(), number(v)?)))
                .collect(),
            waves: key,
            ..Job::default()
        };
        if jobs.len() >= JOBS_MAX {
            return Err("more than 2^20 points".into());
        }
        jobs.insert((job.corner.clone(), *dp), (pid, job));
    }
    let by_point: BTreeMap<i64, (Option<String>, i64)> =
        jobs.iter().map(|(k, (pid, _))| (*pid, k.clone())).collect();
    for r in t("resultValue")? {
        let (Some(k), Some(out)) = (by_point.get(&id(&r[0])), results.get(&id(&r[1]))) else {
            continue;
        };
        let job = &mut jobs.get_mut(k).expect("by_point names jobs").1;
        if let Some(m) = message(&r[3]) {
            job.errors.insert(out.clone(), m);
        }
        match r[2] {
            Val::Real(x) if x.is_finite() => job.values.insert(out.clone(), x),
            Val::Int(i) => job.values.insert(out.clone(), i as f64),
            _ => None,
        };
    }
    let mut near = 0;
    for r in t("specValue")? {
        let (Some(k), Some(out)) = (by_point.get(&id(&r[0])), specs.get(&id(&r[1]))) else {
            continue;
        };
        let job = &mut jobs.get_mut(k).expect("by_point names jobs").1;
        // specStatus: 0 pass, 1 near (outside by at most 10%), 2 fail; else unknown.
        match r[6].int() {
            Some(0) => job.pass.insert(out.clone(), true),
            Some(1) => {
                near += 1;
                job.pass.insert(out.clone(), false)
            }
            Some(2) => job.pass.insert(out.clone(), false),
            _ => None,
        };
    }
    if near > 0 {
        notes.push(format!(
            "{near} verdict(s) `near` (outside the spec by at most 10%) came across as fail"
        ));
    }
    for (name, n) in [
        ("cornerResultValue", "output(s) reduced across corners"),
        ("sweepResultValue", "output(s) reduced across sweeps"),
        ("statVarValue", "Monte Carlo statistic(s)"),
    ] {
        let rows = t(name).map_or(0, |r| r.len());
        if rows > 0 {
            notes.push(format!("{rows} {n}: not mapped"));
        }
    }
    let jobs: Vec<Job> = jobs.into_values().map(|(_, j)| j).collect();
    if jobs.is_empty() {
        return Err(format!("test {test}: no simulated point in the .rdb"));
    }
    Ok(History {
        dump: dump.clone(),
        test: test.clone(),
        jobs,
        notes,
    })
}

/// The run's `results` file (run.md §4) for `h`: `design` the 64 hex digits of the design
/// hash; the columns are the bench's (`outputs`, the outputs with a `spec`, `vars`), as a
/// native run's, so a value of an output the bench lacks is left out.
pub fn results_file(
    h: &History,
    design: &str,
    outputs: &[String],
    specs: &[String],
    vars: &[String],
) -> String {
    let (lib, cell, view) = &h.dump.view;
    let mut out = String::from("schemify 1 results\n");
    attr(&mut out, "design", &[design]);
    attr(
        &mut out,
        "source",
        &["virtuoso", &format!("{lib}/{cell}/{view}"), &h.dump.name],
    );
    for (word, names) in [("output", outputs), ("spec", specs), ("var", vars)] {
        if !names.is_empty() {
            out.push('\n');
        }
        for n in names {
            out.push_str(word);
            out.push(' ');
            schemify_abi::text::push_value(&mut out, n);
            out.push('\n');
        }
    }
    for (j, job) in h.jobs.iter().enumerate() {
        let done = job.status == "done";
        out.push_str(&format!(
            "\njob {j} {}\n",
            if done { "done" } else { "failed" }
        ));
        if let Some(c) = &job.corner {
            attr(&mut out, "corner", &[c]);
        }
        for (n, v) in &job.sets {
            attr(&mut out, "set", &[n, v]);
        }
        let nums = |out: &mut String, key: &str, names: &[String], m: &BTreeMap<String, f64>| {
            for n in names {
                if let Some(x) = m.get(n) {
                    attr(out, key, &[n, &num(*x)]);
                }
            }
        };
        nums(&mut out, "var", vars, &job.vars);
        nums(&mut out, "value", outputs, &job.values);
        for n in specs {
            if let Some(&p) = job.pass.get(n) {
                attr(&mut out, "pass", &[n, if p { "pass" } else { "fail" }]);
            }
        }
        // Error rows by output, the job's own last (run crate: `JOB_LEVEL` sorts last).
        for n in outputs {
            if let Some(m) = job.errors.get(n) {
                attr(&mut out, "error", &[n, m]);
            }
        }
        if !done {
            attr(
                &mut out,
                "error",
                &["-", &format!("maestro: {}", job.status)],
            );
        }
    }
    out
}

fn attr(out: &mut String, key: &str, vals: &[&str]) {
    out.push_str("  ");
    out.push_str(key);
    for v in vals {
        out.push(' ');
        schemify_abi::text::push_value(out, v);
    }
    out.push('\n');
}

/// Canonical number text (expr.md §7), exact.
fn num(x: f64) -> String {
    let mut s = String::new();
    crate::expr::write_num(&mut s, x);
    s
}

/// A maestro value as Schemify writes it (`ael`, as `ade::import` translates var values);
/// the text itself when it has no translation.
fn schemify_text(v: &str) -> String {
    super::ael::from_ael(v, |_| super::ael::AelName::Unknown, |_| None)
        .map_or_else(|| v.to_owned(), |t| t.text)
}

/// A var value's number, when it is one (`1n`, `7`, `1.1`).
fn number(v: &str) -> Option<f64> {
    crate::expr::parse_num(&schemify_text(v)).ok()
}
