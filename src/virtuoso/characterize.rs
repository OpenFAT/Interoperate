//! PDK callback-derived parameters (spec `interop.md` §8.3). A PDK device's CDF callbacks
//! (SKILL, only Virtuoso runs them) derive netlisted parameters (`ad`, `pd`, `nrd`, …) from
//! the ones a user sets. `SfyCharacterize` finds which, by experiment ([`Device::derived`]);
//! the mirror cell marks them `vderived` with the inputs they depend on
//! ([`mark_derived`]); each instance carries Virtuoso's values as `cached` lines with the
//! digest of its inputs ([`inputs_digest`]), stale once an input changes (E039). A refresh
//! asks Virtuoso for the values at exactly the stale instances' inputs
//! ([`refresh_wants`], [`refresh_skill`]) and writes them back ([`refresh_apply`]).
//! Nothing here fits or predicts a callback: every value is Virtuoso's.
//!
//! A characterization file names a PDK's parameters and values, so it stays on the machine
//! it was made on.

use std::collections::BTreeMap;

use schemify_format::{Document, RecordKind, parse, write};

use crate::design::{Files, copy_attr, copy_header, manifest_libs, push_attr};
use crate::sfyx::{self, Record, Value};

/// Most points one device may have.
pub const POINTS_MAX: usize = 10_000;

/// What an input is, as measured (its CDF type and default), never its name.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    /// A continuous number.
    #[default]
    Num,
    /// A whole number (a finger or segment count).
    Count,
    /// A boolean (`"t"` or `"nil"`).
    Bool,
    /// One of the CDF's choices.
    Choice,
}

impl Kind {
    fn parse(s: &str) -> Option<Kind> {
        Some(match s {
            "num" => Kind::Num,
            "count" => Kind::Count,
            "bool" => Kind::Bool,
            "choice" => Kind::Choice,
            _ => return None,
        })
    }
}

/// One characterized device.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Device {
    /// Library and cell.
    pub lib: String,
    /// See [`Device::lib`].
    pub cell: String,
    /// The inputs, in CDF order: the candidates whose nudges changed another netlisted
    /// param, or those `?inputs` named.
    pub inputs: Vec<String>,
    /// Kind per input, parallel to [`Device::inputs`].
    pub kinds: Vec<Kind>,
    /// Default value text per input, parallel to [`Device::inputs`].
    pub defaults: Vec<String>,
    /// The netlisted parameters (spectre `instParameters`), in CDF order.
    pub outputs: Vec<String>,
    /// The discovery probes, one per candidate, in CDF order (none with `?inputs`).
    pub probes: Vec<Probe>,
    /// The points, in file order.
    pub points: Vec<Point>,
}

/// One discovery probe: a candidate nudged from the defaults.
#[derive(Clone, Debug, PartialEq)]
pub struct Probe {
    /// The candidate.
    pub name: String,
    /// Its kind.
    pub kind: Kind,
    /// The value its first nudge set.
    pub set: String,
    /// Whether every callback ran (for one nudge at least).
    pub status: Result<(), String>,
    /// The netlisted params (other than itself) its nudges changed.
    pub changed: Vec<String>,
}

/// One measured point (`?points`: a refresh's inputs).
#[derive(Clone, Debug, PartialEq)]
pub struct Point {
    /// The value set per input (`""`: not set at this point), parallel to [`Device::inputs`].
    pub set: Vec<String>,
    /// Whether every callback ran; else the reason, as SfySetParams gave it.
    pub status: Result<(), String>,
    /// The value read back per netlisted parameter, parallel to [`Device::outputs`].
    pub values: Vec<String>,
    /// The value read back per input, parallel to [`Device::inputs`] (a callback may
    /// rewrite what was set).
    pub back: Vec<String>,
}

fn status_of(s: &str) -> Result<(), String> {
    match s {
        "ok" => Ok(()),
        why => Err(why.to_owned()),
    }
}

fn strs(r: &Record, v: &[Value], what: &str) -> Result<Vec<String>, String> {
    v.iter()
        .map(|x| match x {
            Value::Str(s) => Ok(s.clone()),
            _ => Err(format!("line {}: {what}: not a string", r.line)),
        })
        .collect()
}

/// Reads a characterization file.
///
/// # Errors
///
/// A record that is not `char`/`probe`/`pt` between the `sfyx` header and `end`, a `probe`
/// or `pt` before any `char`, lists that do not match the `char`'s, an unknown kind, or
/// more than [`POINTS_MAX`] points for one device.
pub fn read(text: &str) -> Result<Vec<Device>, String> {
    let recs = sfyx::read(text).map_err(|e| format!("line {}: {}", e.line, e.msg))?;
    let mut out: Vec<Device> = Vec::new();
    for r in &recs {
        match r.tag.as_str() {
            "sfyx" | "end" => {}
            "char" => {
                r.arity(6).map_err(|e| e.msg)?;
                let inputs = strs(r, r.list(2).map_err(|e| e.msg)?, "inputs")?;
                let kinds = strs(r, r.list(4).map_err(|e| e.msg)?, "kinds")?
                    .iter()
                    .map(|k| Kind::parse(k).ok_or_else(|| format!("line {}: kind `{k}`", r.line)))
                    .collect::<Result<Vec<_>, _>>()?;
                let defaults = strs(r, r.list(5).map_err(|e| e.msg)?, "defaults")?;
                if kinds.len() != inputs.len() || defaults.len() != inputs.len() {
                    return Err(format!("line {}: kinds and defaults per input", r.line));
                }
                out.push(Device {
                    lib: r.str(0).map_err(|e| e.msg)?.to_owned(),
                    cell: r.str(1).map_err(|e| e.msg)?.to_owned(),
                    inputs,
                    kinds,
                    defaults,
                    outputs: strs(r, r.list(3).map_err(|e| e.msg)?, "netlisted")?,
                    probes: Vec::new(),
                    points: Vec::new(),
                });
            }
            "probe" => {
                r.arity(5).map_err(|e| e.msg)?;
                let d = out
                    .last_mut()
                    .ok_or_else(|| format!("line {}: a probe before any `char`", r.line))?;
                let kind = r.str(1).map_err(|e| e.msg)?;
                d.probes.push(Probe {
                    name: r.str(0).map_err(|e| e.msg)?.to_owned(),
                    kind: Kind::parse(kind)
                        .ok_or_else(|| format!("line {}: kind `{kind}`", r.line))?,
                    set: r.str(2).map_err(|e| e.msg)?.to_owned(),
                    status: status_of(r.str(3).map_err(|e| e.msg)?),
                    changed: strs(r, r.list(4).map_err(|e| e.msg)?, "changed")?,
                });
            }
            "pt" => {
                r.arity(4).map_err(|e| e.msg)?;
                let d = out
                    .last_mut()
                    .ok_or_else(|| format!("line {}: a point before any `char`", r.line))?;
                let set = strs(r, r.list(0).map_err(|e| e.msg)?, "set")?;
                let values = strs(r, r.list(2).map_err(|e| e.msg)?, "values")?;
                let back = strs(r, r.list(3).map_err(|e| e.msg)?, "read back")?;
                if set.len() != d.inputs.len()
                    || back.len() != d.inputs.len()
                    || values.len() != d.outputs.len()
                {
                    return Err(format!(
                        "line {}: {} set and {} values for {} inputs and {} parameters",
                        r.line,
                        set.len(),
                        values.len(),
                        d.inputs.len(),
                        d.outputs.len()
                    ));
                }
                if d.points.len() == POINTS_MAX {
                    return Err(format!("line {}: more than {POINTS_MAX} points", r.line));
                }
                let status = status_of(r.str(1).map_err(|e| e.msg)?);
                d.points.push(Point {
                    set,
                    status,
                    values,
                    back,
                });
            }
            t => return Err(format!("line {}: unknown record `{t}`", r.line)),
        }
    }
    debug_assert!(out.iter().all(|d| d.points.len() <= POINTS_MAX));
    Ok(out)
}

impl Device {
    /// The value input `k` was set to at `p`: its default where the point left it.
    pub fn set_value<'a>(&'a self, p: &'a Point, k: usize) -> &'a str {
        if p.set[k].is_empty() {
            self.defaults.get(k).map_or("", String::as_str)
        } else {
            &p.set[k]
        }
    }

    /// The netlisted params the callbacks derive, in netlisted order: those the probe of
    /// an input moved, other than the inputs themselves (a param an input moves is
    /// derived even when it is editable).
    pub fn derived(&self) -> Vec<String> {
        self.outputs
            .iter()
            .filter(|o| {
                !self.inputs.contains(o)
                    && self.probes.iter().any(|p| {
                        p.status.is_ok() && self.inputs.contains(&p.name) && p.changed.contains(o)
                    })
            })
            .cloned()
            .collect()
    }
}

/// A CDF number as Virtuoso prints it (`6.72f`, `80n`, `1.2u`, `2`): AEL scale factors (`M`
/// is mega), `None` for anything else (`iPar("m")`, `""`).
pub fn ael_num(s: &str) -> Option<f64> {
    let s = s.trim();
    let (body, scale) = match s.chars().last()? {
        c if c.is_ascii_digit() || c == '.' => (s, 1.0),
        c => (
            &s[..s.len() - c.len_utf8()],
            match c {
                'T' => 1e12,
                'G' => 1e9,
                'M' => 1e6,
                'K' | 'k' => 1e3,
                'm' => 1e-3,
                'u' => 1e-6,
                'n' => 1e-9,
                'p' => 1e-12,
                'f' => 1e-15,
                'a' => 1e-18,
                _ => return None,
            },
        ),
    };
    let x: f64 = body.parse().ok()?;
    (x * scale).is_finite().then_some(x * scale)
}

/// Appends record `r` of `src` to `out` without the attributes `skip` names.
fn copy_without(out: &mut Document<'static>, src: &Document<'_>, r: u32, skip: &[&str]) {
    out.begin_record(src.record_kind(r));
    for f in src.fields(r) {
        out.push_field(f);
    }
    for a in src.attrs(r) {
        if !skip.contains(&src.attr_key(a)) {
            copy_attr(out, src, a);
        }
    }
    for v in src.verbatim(r) {
        out.push_verbatim(v);
    }
}

/// A mirror cell file with `dev`'s derived params marked (spec §8.3): each `vderived true`
/// (its value is Virtuoso's, cached per instance, stale when an input changes; D20 as
/// amended), the header's `vinputs` naming what they depend on. A param the project
/// computes itself (`computed`) is left to its rule; a param no longer derived loses the
/// mark.
///
/// # Errors
///
/// The text is not a Schemify file.
pub fn mark_derived(text: &[u8], dev: &Device) -> Result<String, String> {
    let src = parse(text).map_err(|e| e.to_string())?;
    let derived = dev.derived();
    let marked = |r: u32| {
        src.record_kind(r) == RecordKind::Param
            && derived.iter().any(|d| Some(d.as_str()) == src.field(r, 0))
            && src.find_attr(r, "computed").is_none()
    };
    let mut out = Document::new(src.kind());
    copy_header(&mut out, &src, &["vinputs"]);
    if (1..src.record_count()).any(marked) {
        let v: Vec<&str> = dev.inputs.iter().map(String::as_str).collect();
        push_attr(&mut out, "vinputs", &v);
    }
    for r in 1..src.record_count() {
        if src.record_kind(r) != RecordKind::Param {
            out.copy_record(&src, r);
            continue;
        }
        copy_without(&mut out, &src, r, &["vderived"]);
        if marked(r) {
            push_attr(&mut out, "vderived", &["true"]);
        }
    }
    debug_assert_eq!(out.record_count(), src.record_count());
    Ok(write(&out))
}

/// A param value's canonical text (expr spec §7; a string quoted as format §2 escapes it):
/// what an instance's `cached` line holds and what [`inputs_digest`] hashes, the same text
/// elab writes for the value. `template`: the param is a string or a choice (its value is
/// the text itself). `None` when the value is not a constant.
pub fn canonical_value(text: &str, template: bool) -> Option<String> {
    use crate::expr::{Mode, Program, Scratch, Value};
    let mut out = String::new();
    let quote = |s: &str, out: &mut String| {
        out.push('"');
        for c in s.chars() {
            match c {
                '"' => out.push_str(r#"\""#),
                '\\' => out.push_str(r"\\"),
                '\n' => out.push_str(r"\n"),
                '\t' => out.push_str(r"\t"),
                c => out.push(c),
            }
        }
        out.push('"');
    };
    if template {
        if text.contains('{') {
            return None;
        }
        quote(text, &mut out);
        return Some(out);
    }
    let prog = Program::compile(text, Mode::Scalar).ok()?;
    if prog.idents().len() > 0 {
        return None;
    }
    match prog.eval(&[], &mut Scratch::default()).ok()? {
        Value::Num(x) => crate::expr::write_num(&mut out, x),
        Value::Int(x) => crate::expr::write_int(&mut out, x),
        Value::Bool(b) => out.push_str(if b { "true" } else { "false" }),
        Value::Str(s) => quote(&s, &mut out),
        _ => return None,
    }
    Some(out)
}

/// The digest of an instance's input values (D20 as amended 2026-10-15; elab computes the
/// same to tell a `cached` value fresh from stale): FNV-1a 64 of one `<name>=<canonical
/// value>` line per name of `vinputs`, in order, as 16 lowercase hex digits. `value(name)`
/// gives the instance's value text (its override, else the cell's default) and whether the
/// param is a string or a choice; `None` when one is missing or not a constant.
pub fn inputs_digest<'a>(
    vinputs: &[String],
    mut value: impl FnMut(&str) -> Option<(&'a str, bool)>,
) -> Option<String> {
    let mut text = String::new();
    for n in vinputs {
        let (v, template) = value(n)?;
        text.push_str(n);
        text.push('=');
        text.push_str(&canonical_value(v, template)?);
        text.push('\n');
    }
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    Some(format!("{h:016x}"))
}

/// One PDK cell a refresh measures: its inputs (the mirror's `vinputs`) and one point per
/// distinct set of input values among the stale instances: each input the instance
/// overrides as the CDF text the export sends, `None` for one it leaves at the CDF default
/// (Virtuoso's own default text, which a callback may read differently from a respelling).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Want {
    /// Library and cell.
    pub lib: String,
    /// See [`Want::lib`].
    pub cell: String,
    /// Input names, in `vinputs` order.
    pub inputs: Vec<String>,
    /// Per point, per input: the CDF text it is set to, `None` at its default.
    pub points: Vec<Vec<Option<String>>>,
}

/// A mirror cell's derived-param interface: `vinputs`, the `vderived` params, and per
/// param its kind and default.
struct Mirror {
    vinputs: Vec<String>,
    vderived: Vec<String>,
    params: Vec<(String, String, String)>,
}

impl Mirror {
    fn of(bytes: &[u8]) -> Option<Mirror> {
        let d = parse(bytes).ok()?;
        let vinputs: Vec<String> = d
            .find_attr(0, "vinputs")
            .map(|a| {
                (0..)
                    .map_while(|i| d.attr_value(a, i))
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        let mut m = Mirror {
            vinputs,
            vderived: Vec::new(),
            params: Vec::new(),
        };
        for r in (1..d.record_count()).filter(|&r| d.record_kind(r) == RecordKind::Param) {
            let name = d.field(r, 0)?.to_owned();
            if d.find_attr(r, "vderived").and_then(|a| d.attr_value(a, 0)) == Some("true") {
                m.vderived.push(name.clone());
            }
            let kind = d.find_attr(r, "kind").and_then(|a| d.attr_value(a, 0));
            m.params.push((
                name,
                kind.unwrap_or("number").to_owned(),
                d.field(r, 1)?.to_owned(),
            ));
        }
        (!m.vinputs.is_empty() && !m.vderived.is_empty()).then_some(m)
    }

    fn param(&self, n: &str) -> Option<&(String, String, String)> {
        self.params.iter().find(|p| p.0 == n)
    }

    fn template(&self, n: &str) -> bool {
        self.param(n)
            .is_some_and(|p| matches!(p.1.as_str(), "string" | "choice"))
    }
}

/// An instance of a mirror cell, read for a refresh.
struct Inst {
    /// Its input values: the override, else the default.
    values: Vec<String>,
    /// Per input, whether the instance overrides it.
    set: Vec<bool>,
    /// The digest of [`Inst::values`]; `None` when one is not a constant.
    digest: Option<String>,
    /// Whether every derived param has a `cached` line of that digest, or none has one
    /// and no input is overridden (elab's E039, the other way round).
    fresh: bool,
}

fn inst_of(d: &Document<'_>, r: u32, m: &Mirror) -> Option<Inst> {
    let over = |n: &str| d.find_attr(r, n).and_then(|a| d.attr_value(a, 0));
    let values: Vec<String> = m
        .vinputs
        .iter()
        .map(|n| {
            over(n)
                .or_else(|| m.param(n).map(|p| p.2.as_str()))
                .map(str::to_owned)
        })
        .collect::<Option<_>>()?;
    let set: Vec<bool> = m.vinputs.iter().map(|n| over(n).is_some()).collect();
    let digest = inputs_digest(&m.vinputs, |n| {
        let k = m.vinputs.iter().position(|v| v == n)?;
        Some((values[k].as_str(), m.template(n)))
    });
    let cached: Vec<(&str, &str)> = d
        .attrs(r)
        .filter(|&a| d.attr_key(a) == "cached")
        .filter_map(|a| Some((d.attr_value(a, 0)?, d.attr_value(a, 2)?)))
        .collect();
    let fresh = match &digest {
        None => true,
        Some(_) if cached.is_empty() => !m.vinputs.iter().any(|n| over(n).is_some()),
        Some(h) => m
            .vderived
            .iter()
            .all(|p| cached.iter().any(|c| c.0 == p && c.1 == h)),
    };
    Some(Inst {
        values,
        set,
        digest,
        fresh,
    })
}

/// An instance of a PDK library cell: (record, lib, cell).
type LibInst = (u32, String, String);

/// The design files of `files` (a project) with their instances of library cells.
fn mirror_instances(files: &Files) -> Vec<(&String, Vec<LibInst>)> {
    let libs = files
        .get("schemify.project")
        .and_then(|m| manifest_libs(m).ok())
        .unwrap_or_default();
    let mut out = Vec::new();
    for (path, bytes) in files {
        if !(path.ends_with(".shcomp") || path.ends_with(".shbench")) {
            continue;
        }
        let Ok(d) = parse(bytes) else { continue };
        let insts: Vec<LibInst> = (1..d.record_count())
            .filter(|&r| d.record_kind(r) == RecordKind::Inst)
            .filter_map(|r| {
                let (lib, cell) = d.field(r, 1)?.split_once(':')?;
                libs.contains_key(lib)
                    .then(|| (r, lib.to_owned(), cell.to_owned()))
            })
            .collect();
        if !insts.is_empty() {
            out.push((path, insts));
        }
    }
    out
}

/// The mirror cell `lib:cell` of project `files`.
fn mirror_of(files: &Files, lib: &str, cell: &str) -> Option<Mirror> {
    let libs = manifest_libs(files.get("schemify.project")?).ok()?;
    let dir = libs.get(lib)?;
    let path = if dir.is_empty() || dir == "." {
        format!("{cell}.shprim")
    } else {
        format!("{}/{cell}.shprim", dir.trim_end_matches('/'))
    };
    Mirror::of(files.get(&path)?)
}

/// An instance's point: per input, the CDF text of its override (`None` at its default);
/// `None` when an override is not a constant.
fn point_texts(m: &Mirror, i: &Inst) -> Option<Vec<Option<String>>> {
    m.vinputs
        .iter()
        .zip(i.values.iter().zip(&i.set))
        .map(|(n, (v, &set))| match set {
            true => cdf_text(&m.param(n).expect("a vinput param").1, v).map(Some),
            false => Some(None),
        })
        .collect()
}

/// A value as the CDF text the export sends for a param of `kind` (§3.4); `None` when it
/// is not a constant.
fn cdf_text(kind: &str, value: &str) -> Option<String> {
    canonical_value(value, matches!(kind, "string" | "choice"))?;
    super::write::constant_cdf_text(kind, value)
}

/// What a refresh asks Virtuoso to measure: per mirror cell, one point per distinct set of
/// input values among the instances whose cached derived values are stale or missing
/// (E039), and a note per stale instance it cannot ask for (an input that is not a
/// constant). Sorted by lib and cell; points in first-seen order.
pub fn refresh_wants(files: &Files) -> (Vec<Want>, Vec<String>) {
    let mut wants: BTreeMap<(String, String), Want> = BTreeMap::new();
    let mut notes = Vec::new();
    for (path, insts) in mirror_instances(files) {
        let d = parse(&files[path]).expect("parsed above");
        for (r, lib, cell) in insts {
            let Some(m) = mirror_of(files, &lib, &cell) else {
                continue;
            };
            let Some(i) = inst_of(&d, r, &m) else {
                continue;
            };
            if i.fresh {
                continue;
            }
            let name = d
                .find_attr(r, "name")
                .and_then(|a| d.attr_value(a, 0))
                .unwrap_or("?");
            let texts = point_texts(&m, &i);
            match (i.digest, texts) {
                (Some(_), Some(t)) => {
                    let w = wants.entry((lib.clone(), cell.clone())).or_insert_with(|| Want {
                        lib,
                        cell,
                        inputs: m.vinputs.clone(),
                        points: Vec::new(),
                    });
                    if !w.points.contains(&t) {
                        w.points.push(t);
                    }
                }
                _ => notes.push(format!(
                    "{path}: {name}: an input of {lib}:{cell} is not a constant; its derived values stay stale"
                )),
            }
        }
    }
    (wants.into_values().collect(), notes)
}

/// A SKILL string literal.
fn skill_str(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The SKILL that measures `wants` in Virtuoso (after `schemify.il` is loaded): one
/// `SfyCharacterize ?inputs ?points` per cell into `<dir>/refresh.<lib>.<cell>.sfyx`, on a
/// throwaway schematic in library `scratch`. Your designs are not touched.
pub fn refresh_skill(wants: &[Want], dir: &str, scratch: &str) -> String {
    let mut out = String::from(
        ";; schemify-interop virtuoso-refresh: the derived values of stale instances.\n",
    );
    for w in wants {
        let file = format!(
            "{}/refresh.{}.{}.sfyx",
            dir.trim_end_matches('/'),
            w.lib,
            w.cell
        );
        let inputs: Vec<String> = w.inputs.iter().map(|n| skill_str(n)).collect();
        let points: Vec<String> = w
            .points
            .iter()
            .map(|p| {
                let pairs: Vec<String> = w
                    .inputs
                    .iter()
                    .zip(p)
                    .filter_map(|(n, v)| {
                        Some(format!(
                            "(list {} {})",
                            skill_str(n),
                            skill_str(v.as_ref()?)
                        ))
                    })
                    .collect();
                format!("(list {})", pairs.join(" "))
            })
            .collect();
        out.push_str(&format!(
            "(SfyCharacterize {} {} (list {}) {} ?inputs (list {}) ?points (list {}))\n",
            skill_str(&file),
            skill_str(&w.lib),
            skill_str(&w.cell),
            skill_str(scratch),
            inputs.join(" "),
            points.join(" ")
        ));
    }
    out
}

/// What [`refresh_apply`] did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Refreshed {
    /// The changed design files, `(path, bytes)`.
    pub files: Vec<(String, Vec<u8>)>,
    /// One line per stale instance left stale (no point, a failed callback).
    pub stale: Vec<String>,
    /// One line per input the callbacks rewrote (the instance now holds Virtuoso's value,
    /// as an import would bring it back).
    pub rewritten: Vec<String>,
}

/// `files` with every stale instance a point of `measured` matches (its overrides set
/// exactly, every callback run) brought to what Virtuoso holds: the inputs the callbacks
/// rewrote as overrides (`wf` after a `w` edit), and the derived values as fresh `cached`
/// lines with the digest of those inputs.
pub fn refresh_apply(files: &Files, measured: &[Device]) -> Refreshed {
    let mut out_files = Vec::new();
    let mut notes = Vec::new();
    let mut rewritten = Vec::new();
    for (path, insts) in mirror_instances(files) {
        let d = parse(&files[path]).expect("parsed above");
        // Per record: the rewritten inputs and the cached lines.
        type Update = (Vec<(String, String)>, Vec<[String; 3]>);
        let mut lines: BTreeMap<u32, Update> = BTreeMap::new();
        for (r, lib, cell) in insts {
            let Some(m) = mirror_of(files, &lib, &cell) else {
                continue;
            };
            let Some(i) = inst_of(&d, r, &m) else {
                continue;
            };
            if i.digest.is_none() || i.fresh {
                continue;
            }
            let name = d
                .find_attr(r, "name")
                .and_then(|a| d.attr_value(a, 0))
                .unwrap_or("?");
            let texts = point_texts(&m, &i);
            let found = texts.as_ref().and_then(|t| {
                let dev = measured.iter().find(|x| x.lib == lib && x.cell == cell)?;
                let p = dev.points.iter().find(|p| {
                    m.vinputs.iter().zip(t).all(|(n, v)| {
                        dev.inputs
                            .iter()
                            .position(|x| x == n)
                            .is_some_and(|k| p.set[k] == v.as_deref().unwrap_or(""))
                    })
                })?;
                Some((dev, p))
            });
            let Some((dev, p)) = found else {
                notes.push(format!(
                    "{path}: {name}: no measurement of {lib}:{cell} at its inputs; still stale"
                ));
                continue;
            };
            if let Err(why) = &p.status {
                notes.push(format!(
                    "{path}: {name}: a callback of {lib}:{cell} failed ({why}); still stale"
                ));
                continue;
            }
            // The inputs as Virtuoso holds them after the callbacks.
            let mut values = i.values.clone();
            let mut rewrites = Vec::new();
            for (j, n) in m.vinputs.iter().enumerate() {
                let Some(k) = dev.inputs.iter().position(|x| x == n) else {
                    continue;
                };
                let kind = &m.param(n).expect("a vinput param").1;
                let Some(back) = super::read::constant_from_cdf(kind, &p.back[k]) else {
                    continue;
                };
                let canon = |v: &str| canonical_value(v, m.template(n));
                if canon(&back).is_some() && canon(&back) != canon(&values[j]) {
                    rewritten.push(format!(
                        "{path}: {name}: Virtuoso's callbacks set {n} to {back} (was {})",
                        values[j]
                    ));
                    rewrites.push((n.clone(), back.clone()));
                    values[j] = back;
                }
            }
            let digest = inputs_digest(&m.vinputs, |n| {
                let j = m.vinputs.iter().position(|v| v == n)?;
                Some((values[j].as_str(), m.template(n)))
            });
            let cached: Option<Vec<[String; 3]>> = m
                .vderived
                .iter()
                .map(|q| {
                    let k = dev.outputs.iter().position(|o| o == q)?;
                    let kind = &m.param(q)?.1;
                    let v = super::read::constant_from_cdf(kind, &p.values[k])?;
                    Some([
                        q.clone(),
                        canonical_value(&v, m.template(q))?,
                        digest.clone()?,
                    ])
                })
                .collect();
            match cached {
                Some(v) => {
                    lines.insert(r, (rewrites, v));
                }
                None => notes.push(format!(
                    "{path}: {name}: a derived value of {lib}:{cell} was not measured as a constant; still stale"
                )),
            }
        }
        if lines.is_empty() {
            continue;
        }
        let mut out = Document::new(d.kind());
        copy_header(&mut out, &d, &[]);
        for r in 1..d.record_count() {
            let Some((rewrites, cached)) = lines.get(&r) else {
                out.copy_record(&d, r);
                continue;
            };
            // Overrides in place, a new one after them, then the cached lines.
            out.begin_record(d.record_kind(r));
            for f in d.fields(r) {
                out.push_field(f);
            }
            for a in d.attrs(r) {
                match (
                    d.attr_key(a),
                    rewrites.iter().find(|w| w.0 == d.attr_key(a)),
                ) {
                    ("cached", _) => {}
                    (_, Some((n, v))) => push_attr(&mut out, n, &[v]),
                    _ => copy_attr(&mut out, &d, a),
                }
            }
            for (n, v) in rewrites {
                if d.find_attr(r, n).is_none() {
                    push_attr(&mut out, n, &[v]);
                }
            }
            for [p, val, h] in cached {
                push_attr(&mut out, "cached", &[p, val, h]);
            }
        }
        out_files.push((path.clone(), write(&out).into_bytes()));
    }
    Refreshed {
        files: out_files,
        stale: notes,
        rewritten,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = r#"("sfyx" 1 "schemify.il 1")
("char" "pdk" "nch" ("w" "nf") ("w" "nf" "ad" "m") ("num" "count") ("1e-06" "1"))
("probe" "w" "num" "2e-06" "ok" ("ad"))
("probe" "flip" "bool" "t" "ok" ())
("pt" ("1e-06" "1") "ok" ("1u" "1" "1f" "1") ("1u" "1"))
("pt" ("1e-06" "2") "ok" ("1u" "2" "0.5f" "1") ("1u" "2"))
("pt" ("4e-06" "1") "\"callback: w failed\"" ("4u" "1" "?" "1") ("4u" "1"))
("end")
"#;

    #[test]
    fn a_file_reads_and_its_probes_name_the_derived_params() {
        let d = &read(FILE).unwrap()[0];
        assert_eq!((d.lib.as_str(), d.cell.as_str()), ("pdk", "nch"));
        assert_eq!(d.points.len(), 3);
        assert_eq!(d.kinds, [Kind::Num, Kind::Count]);
        assert_eq!(d.probes.len(), 2);
        assert_eq!(d.probes[0].changed, ["ad"]);
        assert!(d.probes[1].changed.is_empty() && d.probes[1].kind == Kind::Bool);
        assert!(d.points[2].status.is_err());
        // ad moves with w; the inputs and the params nothing moves are not derived.
        assert_eq!(d.derived(), ["ad"]);
    }

    /// The digest elab compares a `cached` line against: FNV-1a 64 of the canonical
    /// `name=value` lines, written here independently of the implementation.
    #[test]
    fn inputs_digest_hashes_canonical_values() {
        let fnv = |t: &str| {
            let h = t.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
                (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3)
            });
            format!("{h:016x}")
        };
        let vin = ["w".to_owned(), "nf".to_owned(), "mode".to_owned()];
        let vals = [("w", "2u"), ("nf", "1"), ("mode", "lv\"t")];
        let d = inputs_digest(&vin, |n| {
            let v = vals.iter().find(|x| x.0 == n)?.1;
            Some((v, n == "mode"))
        });
        assert_eq!(
            d.as_deref(),
            Some(fnv("w=2e-6\nnf=1\nmode=\"lv\\\"t\"\n").as_str())
        );
        assert_eq!(canonical_value("2*3", false).as_deref(), Some("6"));
        assert_eq!(canonical_value("1.0", false).as_deref(), Some("1"));
        assert_eq!(canonical_value("true", false).as_deref(), Some("true"));
        // Not a constant: no digest.
        assert!(canonical_value("w*2", false).is_none());
        assert!(
            inputs_digest(&vin, |n| Some((if n == "w" { "wn" } else { "1" }, false))).is_none()
        );
    }

    #[test]
    fn derived_params_are_marked_on_the_mirror() {
        let d = &read(FILE).unwrap()[0];
        let cell = b"schemify 1 shprim\n  vinputs old\n\npin d inout 0 0\n\nparam w 100n\nparam nf 1\n  vderived true\nparam ad 5f\n  unit m\nparam m 1\n\nimpl spice spice\n  format spice \"M{name} {d} x w={w} ad={ad}\"\n";
        let out = mark_derived(cell, d).unwrap();
        assert!(
            out.starts_with("schemify 1 shprim\n  vinputs w nf\n"),
            "{out}"
        );
        assert!(
            out.contains("param ad 5f\n  unit m\n  vderived true\n"),
            "{out}"
        );
        // nf is an input now: its stale mark goes; m is not derived.
        assert!(out.contains("param nf 1\nparam ad"), "{out}");
        assert!(out.contains("param m 1\n\n"), "{out}");
        // A param the project computes keeps its rule and gets no mark (nor a vinputs).
        let own = String::from_utf8_lossy(cell).replace("  unit m\n", "  computed w*2\n");
        let out = mark_derived(own.as_bytes(), d).unwrap();
        assert!(
            !out.contains("vderived") && !out.contains("vinputs"),
            "{out}"
        );
    }

    /// The live PDK case: `w` edited, the stored finger width `wf` stale; the callbacks
    /// set `wf` from `w`. The refresh brings both back as Virtuoso holds them, the cached
    /// value with the digest of those inputs.
    #[test]
    fn a_refresh_takes_the_inputs_the_callbacks_rewrote() {
        let mirror = b"schemify 1 shprim\n  vinputs w wf\n\npin d inout 0 0\n\nparam w 80n\nparam wf 80n\nparam ad 1f\n  vderived true\n\nimpl spice spice\n  format spice \"M{name} {d} x w={w}\"\n";
        let design = b"schemify 1 shcomp\n\nimpl schematic schematic\n\ninst aaaaaa pdk:mos\n  at 0 0 R0\n  name M0\n  w 4e-7\n  wf 200n\n  cached ad 1.68e-14 0000000000000000\n";
        let files: Files = [
            (
                "schemify.project".to_owned(),
                b"schemify 1 project\n\nlib pdk path pdk\n".to_vec(),
            ),
            ("pdk/mos.shprim".to_owned(), mirror.to_vec()),
            ("top.shcomp".to_owned(), design.to_vec()),
        ]
        .into_iter()
        .collect();
        let (wants, _) = refresh_wants(&files);
        assert_eq!(
            wants[0].points,
            [[Some("4e-7".to_owned()), Some("200n".to_owned())]]
        );
        let dev = read(concat!(
            "(\"sfyx\" 1 \"t\")\n",
            "(\"char\" \"pdk\" \"mos\" (\"w\" \"wf\") (\"w\" \"wf\" \"ad\") (\"num\" \"num\") (\"8e-08\" \"8e-08\"))\n",
            "(\"pt\" (\"4e-7\" \"200n\") \"ok\" (\"400n\" \"400n\" \"33.6f\") (\"400n\" \"400n\"))\n",
            "(\"end\")\n"
        ))
        .unwrap();
        let done = refresh_apply(&files, &dev);
        assert!(done.stale.is_empty(), "{done:?}");
        assert_eq!(done.rewritten.len(), 1, "{done:?}");
        let text = String::from_utf8(done.files[0].1.clone()).unwrap();
        let digest = inputs_digest(&["w".into(), "wf".into()], |_| Some(("4e-7", false))).unwrap();
        assert!(text.contains("  w 4e-7\n  wf 400n\n"), "{text}");
        assert!(
            text.contains(&format!("  cached ad 3.36e-14 {digest}\n")),
            "{text}"
        );
        // Fresh now: nothing left to ask.
        let mut after = files.clone();
        after.extend(done.files);
        assert!(refresh_wants(&after).0.is_empty());
    }

    #[test]
    fn malformed_files_are_refused() {
        let bad = [
            FILE.replace(
                "(\"char\" \"pdk\" \"nch\" (\"w\" \"nf\") (\"w\" \"nf\" \"ad\" \"m\") (\"num\" \"count\") (\"1e-06\" \"1\"))\n",
                "",
            ),
            FILE.replace(
                "(\"1e-06\" \"1\") \"ok\" (\"1u\" \"1\" \"1f\" \"1\")",
                "(\"1e-06\") \"ok\" (\"1u\" \"1\" \"1f\" \"1\")",
            ),
            FILE.replace("(\"end\")", "(\"what\")\n(\"end\")"),
            FILE.replace("(\"num\" \"count\")", "(\"num\" \"wat\")"),
        ];
        for b in &bad {
            assert!(read(b).is_err(), "{b}");
        }
    }
}
