//! A mock Virtuoso database for the SKILL test interpreter (spec `interop.md` §5.4).
//!
//! Each Cadence function checks its arity and argument types against [`SIGNATURES`] (taken
//! from the IC20.1 reference manuals) and fails as documented: a bad argument signals (`Err`), a refused
//! operation returns `nil`. Behaviour the manuals leave open is marked `GUESS(Gn)`, matching
//! the spec's §13 table; the probe (`skill/sfy_probe.il`) settles each one on a real
//! Virtuoso. The maestro part lives in `mae.rs`.
//!
//! Model: one arena of objects ([`Obj`], an attribute bag with `objType`), addressed by
//! `V::Obj(id)`. Libraries are the cds.lib list ([`MockDb::add_lib`]); cells, views and files
//! are DD objects; a cellview is open-able once created and keeps a backup of its content
//! while opened `"w"`, so a purge or a last close without `dbSave` restores what was saved
//! (or removes a never-saved view). Database order is unspecified in Virtuoso, so list
//! attributes (`shapes`, `instances`, `terminals`, `nets`, `cells`, `views`) come back in
//! *reverse* creation order: a dumper that forgets to sort fails the goldens.
//!
//! ponytail: one CDF level (the base cell CDF is the effective CDF; no library or user
//! levels); objects of a purged cellview stay in the arena (Virtuoso invalidates them).
//!
//! Submodules, one area of Cadence functions each, with its rows of [`SIGNATURES`]: `core`
//! (DD, database, schematic editor), `cdf`, `mae` (maestro, `mae*`/`axl*`), and the stubs
//! `hdb` (config views), `asi` (ADE L sessions) and `layout`, empty until their lanes fill
//! them (README "Development"), and `results` (maestro histories and OCEAN, lane L4).

use super::{Host, V};
use std::collections::BTreeMap;
use std::sync::LazyLock;

mod asi;
mod cdf;
mod core;
mod hdb;
mod layout;
pub mod mae;
pub mod results;

/// Signatures of every Cadence function the mock implements: `(name, args)`. `args` is a
/// space-separated list of argument kinds, `[` starting the optional ones, `?key` naming an
/// accepted keyword: `d` db/DD/CDF object, `D` object or nil, `t` string, `T` string or nil,
/// `S` string or symbol, `x` integer, `n` number, `l` list, `p` point, `b` box, `L` layer
/// (`"name"` or `("layer" "purpose")`), `g` anything. `tests/skill_lint.rs` checks every call
/// in the shipped scripts against this table. Assembled from the areas' tables, in order.
pub static SIGNATURES: LazyLock<Vec<(&str, &str)>> = LazyLock::new(|| {
    DISPATCHED
        .iter()
        .chain(&DELEGATED)
        .flat_map(|t| t.iter().copied())
        .collect()
});

/// Areas whose bodies are arms of `MockDb::dispatch`.
const DISPATCHED: [&[(&str, &str)]; 2] = [core::ROWS, cdf::ROWS];

/// Areas whose bodies live in their module's `call`: the signature is checked before the
/// call reaches it. A lane adds its area here and to `Host::call`.
const DELEGATED: [&[(&str, &str)]; 5] =
    [mae::ROWS, hdb::ROWS, asi::ROWS, layout::ROWS, results::ROWS];

/// The arity a signature allows: (required positional, max positional, keywords).
pub fn arity(sig: &str) -> (usize, usize, Vec<&str>) {
    let mut req = 0;
    let mut max = 0;
    let mut opt = false;
    let mut keys = Vec::new();
    for tok in sig.split_whitespace() {
        if tok == "..." {
            max = usize::MAX;
            continue;
        }
        if let Some(k) = tok.strip_prefix('?') {
            keys.push(k);
            continue;
        }
        let t = tok.trim_matches(|c| c == '[' || c == ']');
        if tok.starts_with('[') {
            opt = true;
        }
        if t.is_empty() {
            continue;
        }
        max = max.saturating_add(1);
        if !opt {
            req += 1;
        }
    }
    (req, max, keys)
}

/// One object: an attribute bag. `objType` names its kind.
#[derive(Clone, Debug, Default)]
pub struct Obj {
    /// Attributes by name (relations hold `V::Obj` or lists of them).
    pub attrs: BTreeMap<String, V>,
}

/// The mock database: libraries, cells, cellviews and their objects, CDF, maestro setups.
pub struct MockDb {
    /// Maestro state.
    pub mae: mae::Mae,
    objs: Vec<Obj>,
    libs: Vec<u32>,
    cvs: BTreeMap<(String, String, String), u32>,
    backups: BTreeMap<u32, BTreeMap<String, V>>,
    pins_made: u32,
}

impl Default for MockDb {
    fn default() -> Self {
        MockDb {
            mae: mae::Mae::default(),
            objs: vec![Obj::default()],
            libs: Vec::new(),
            cvs: BTreeMap::new(),
            backups: BTreeMap::new(),
            pins_made: 0,
        }
    }
}

const ORIENTS: &[&str] = &["R0", "R90", "R180", "R270", "MY", "MYR90", "MX", "MXR90"];
const JUSTIFY: &[&str] = &[
    "lowerLeft",
    "lowerCenter",
    "lowerRight",
    "centerLeft",
    "centerCenter",
    "centerRight",
    "upperLeft",
    "upperCenter",
    "upperRight",
];
const FONTS: &[&str] = &[
    "euroStyle",
    "gothic",
    "math",
    "roman",
    "script",
    "stick",
    "swedish",
    "fixed",
    "milSpec",
];
const LABEL_TYPES: &[&str] = &["normalLabel", "NLPLabel", "ILLabel"];
const PROP_TYPES: &[&str] = &[
    "boolean", "fileName", "float", "hierProp", "ILExpr", "ILList", "int", "NLPExpr", "netSet",
    "string", "time",
];
const CDF_TYPES: &[&str] = &[
    "string", "int", "float", "radio", "cyclic", "boolean", "button", "netSet",
];
/// Attributes returned in reverse creation order ("database order").
const DB_ORDER: &[&str] = &["shapes", "instances", "terminals", "nets", "cells", "views"];

fn num(v: &V) -> Option<f64> {
    match v {
        V::Int(i) => Some(*i as f64),
        V::Float(f) => Some(*f),
        _ => None,
    }
}

fn pt(v: &V) -> Option<(f64, f64)> {
    match v.items() {
        [x, y] => Some((num(x)?, num(y)?)),
        _ => None,
    }
}

fn bb(v: &V) -> Option<((f64, f64), (f64, f64))> {
    match v.items() {
        [a, b] => Some((pt(a)?, pt(b)?)),
        _ => None,
    }
}

fn vpt(p: (f64, f64)) -> V {
    V::list(vec![V::Float(p.0), V::Float(p.1)])
}

fn vbox(a: (f64, f64), b: (f64, f64)) -> V {
    let (x0, x1) = (a.0.min(b.0), a.0.max(b.0));
    let (y0, y1) = (a.1.min(b.1), a.1.max(b.1));
    V::list(vec![vpt((x0, y0)), vpt((x1, y1))])
}

fn s(v: &V) -> Option<&str> {
    match v {
        V::Str(x) => Some(x),
        _ => None,
    }
}

/// Orientation point maps in Virtuoso's y-up frame: mirror first, then turn
/// counter-clockwise.
// GUESS(G1): MYR90 = (-y, -x) and MXR90 = (y, x) (derived; the other six are definitions).
fn orient_map(o: &str, (x, y): (f64, f64)) -> Option<(f64, f64)> {
    Some(match o {
        "R0" => (x, y),
        "R90" => (-y, x),
        "R180" => (-x, -y),
        "R270" => (y, -x),
        "MY" => (-x, y),
        "MYR90" => (-y, -x),
        "MX" => (x, -y),
        "MXR90" => (y, x),
        _ => return None,
    })
}

/// Applies a transform list `(offset orient [mag])`: orient, then magnify, then translate.
fn transform(p: (f64, f64), t: &V) -> Result<(f64, f64), String> {
    let items = t.items();
    let off = items
        .first()
        .and_then(pt)
        .ok_or("transform: needs (offset orient [mag])")?;
    let o = items.get(1).and_then(s).ok_or("transform: orient")?;
    let mag = items.get(2).and_then(num).unwrap_or(1.0);
    let (x, y) = orient_map(o, p).ok_or_else(|| format!("transform: orient {o}"))?;
    Ok((x * mag + off.0, y * mag + off.1))
}

fn bad(name: &str, k: usize, what: &str) -> String {
    format!("*Error* {name}: argument #{k} should be {what}")
}

/// Checks `args` against `sig`; returns positional arguments and keyword arguments.
/// Positional arguments and `(keyword, value)` pairs of a checked call.
type Args = (Vec<V>, Vec<(String, V)>);

fn check(name: &str, sig: &str, args: &[V]) -> Result<Args, String> {
    let (req, max, keys) = arity(sig);
    let kinds: Vec<&str> = sig
        .split_whitespace()
        .filter(|t| !t.starts_with('?'))
        .map(|t| t.trim_matches(|c| c == '[' || c == ']'))
        .filter(|t| !t.is_empty())
        .collect();
    let mut pos = Vec::new();
    let mut kw = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a {
            V::Sym(k) if k.starts_with('?') => {
                let k = &k[1..];
                if !keys.contains(&k) {
                    return Err(format!("*Error* {name}: unknown keyword ?{k}"));
                }
                let v = it
                    .next()
                    .ok_or_else(|| format!("*Error* {name}: ?{k} without a value"))?;
                kw.push((k.to_owned(), v.clone()));
            }
            _ => pos.push(a.clone()),
        }
    }
    if pos.len() < req || pos.len() > max {
        return Err(format!(
            "*Error* {name}: called with {} arguments, takes {req}..{max}",
            pos.len()
        ));
    }
    for (k, (v, kind)) in pos.iter().zip(&kinds).enumerate() {
        let ok = match *kind {
            "d" => matches!(v, V::Obj(_)),
            "D" => matches!(v, V::Obj(_) | V::Nil),
            "t" => matches!(v, V::Str(_)),
            "T" => matches!(v, V::Str(_) | V::Nil),
            "S" => matches!(v, V::Str(_) | V::Sym(_)),
            "x" => matches!(v, V::Int(_)),
            "n" => num(v).is_some(),
            "l" => matches!(v, V::List(_) | V::Nil),
            "p" => pt(v).is_some(),
            "b" => bb(v).is_some(),
            "L" => matches!(v, V::Str(_)) || matches!(v.items(), [V::Str(_), V::Str(_)]),
            _ => true,
        };
        if !ok {
            let what = match *kind {
                "d" => "a database object",
                "D" => "a database object or nil",
                "t" => "a string",
                "T" => "a string or nil",
                "S" => "a string or symbol",
                "x" => "an integer",
                "n" => "a number",
                "l" => "a list",
                "p" => "a point (x y)",
                "b" => "a box ((x0 y0) (x1 y1))",
                _ => "a layer name or (layer purpose)",
            };
            return Err(bad(name, k + 1, what));
        }
    }
    Ok((pos, kw))
}

impl Host for MockDb {
    fn call(&mut self, name: &str, args: &[V]) -> Option<Result<V, String>> {
        if let Some(r) = self.call_db(name, args) {
            return Some(r);
        }
        mae::call(self, name, args)
            .or_else(|| hdb::call(self, name, args))
            .or_else(|| asi::call(self, name, args))
            .or_else(|| layout::call(self, name, args))
            .or_else(|| results::call(self, name, args))
    }

    fn get(&mut self, obj: u32, attr: &str) -> Result<V, String> {
        if obj == 0 || obj as usize >= self.objs.len() {
            return Err(format!("db:{obj}~>{attr}: no such object"));
        }
        Ok(self.get_attr(obj, attr))
    }

    fn set(&mut self, obj: u32, attr: &str, value: V) -> Result<V, String> {
        self.set_attr(obj, attr, value)
    }

    fn bound(&self, name: &str) -> bool {
        SIGNATURES.iter().any(|(n, _)| *n == name)
    }

    fn call_files(
        &mut self,
        name: &str,
        args: &[V],
        files: &mut BTreeMap<String, String>,
    ) -> Option<Result<V, String>> {
        match mae::call_files(self, name, args, files) {
            Some(r) => Some(r),
            None if results::active() && matches!(name, "isDir" | "system") => {
                self.call(name, args)
            }
            None => asi::call_files(self, name, args, files).or_else(|| self.call(name, args)),
        }
    }
}
