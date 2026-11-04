//! Mock maestro history and OCEAN results for `sfy_results.il` (lane L4): the history
//! functions (`axlGetHistoryEntry`, `axlGetHistoryResults`, `axlGetPointPsfDir`), OCEAN's
//! data access (`openResults` … `drGetElem`) and the SKILL builtins the dumper calls that
//! the interpreter lacks (`isDir`, `complexp`, `real`, `imag`, `system`).
//!
//! Model: one history at a time, set by [`set`] (thread-local, so the shared `MockDb` stays
//! as it is): its name, `.rdb` path, and point directories by (test, corner, design point),
//! each holding OCEAN results of waveforms. A point directory's number is not its design
//! point (as on ICADVM20.1: `axlGetPointPsfDir` answers dir `2` for hot point 1). A
//! waveform is the list `("wave" k)`, a vector `("vec" k axis)`, a complex number
//! `("complex" re im)`; `v()` of a current answers nil, as OCEAN's does for a terminal.
//! `system` only records its command ([`commands`]).

use std::cell::RefCell;
use std::collections::BTreeMap;

use super::super::V;
use super::MockDb;

/// The rows of this area in the assembled `mockdb::SIGNATURES` (ICADVM20.1 *ADE SKILL
/// Reference* / *OCEAN Reference* syntax lines).
pub const ROWS: &[(&str, &str)] = &[
    ("axlGetHistoryEntry", "x t"),
    ("axlGetHistoryResults", "x"),
    ("axlGetPointPsfDir", "x t ?cornerName ?designPointId"),
    ("openResults", "t"),
    ("results", ""),
    ("selectResult", "S"),
    ("outputs", ""),
    ("v", "t"),
    ("i", "t"),
    ("drGetWaveformXVec", "g"),
    ("drGetWaveformYVec", "g"),
    ("drVectorLength", "g"),
    ("drGetElem", "g x"),
    ("complexp", "g"),
    ("real", "g"),
    ("imag", "g"),
    ("isDir", "t"),
    ("system", "t"),
];

/// One waveform: OCEAN name, a current, x values, y values `(re, im)` (`im` `None` for a
/// real waveform).
pub type Wave = (String, bool, Vec<f64>, Vec<(f64, Option<f64>)>);

/// The mock history.
#[derive(Clone, Debug, Default)]
pub struct History {
    /// Its name (`Interactive.0`).
    pub name: String,
    /// The `.rdb` path `axlGetHistoryResults` answers.
    pub rdb: String,
    /// `(test, corner, design point) → point directory`.
    pub points: BTreeMap<(String, String, i64), String>,
    /// Point directory → OCEAN results `(name, waveforms)`.
    pub dirs: BTreeMap<String, Vec<(String, Vec<Wave>)>>,
}

#[derive(Default)]
struct State {
    history: History,
    open: Option<String>,
    selected: Option<usize>,
    commands: Vec<String>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

/// Sets the history the mock answers for (and forgets any open results).
pub fn set(h: History) {
    STATE.with(|s| {
        *s.borrow_mut() = State {
            history: h,
            ..State::default()
        }
    });
}

/// Whether a history is set ([`set`]): `isDir` and `system`, rows of `asi` too, are then this
/// area's.
pub fn active() -> bool {
    STATE.with(|s| !s.borrow().history.name.is_empty())
}

/// The commands `system` was given, in order.
pub fn commands() -> Vec<String> {
    STATE.with(|s| s.borrow().commands.clone())
}

const HISTORY_HANDLE: i64 = 8100;

fn tagged<'a>(v: &'a V, tag: &str) -> Option<&'a [V]> {
    let it = v.items();
    (it.first().and_then(V::as_str) == Some(tag)).then(|| &it[1..])
}

fn kw<'a>(args: &'a [V], k: &str) -> Option<&'a V> {
    args.windows(2)
        .find(|w| matches!(&w[0], V::Sym(s) if s.strip_prefix('?') == Some(k)))
        .map(|w| &w[1])
}

/// Handles a function of this area; `None` when `name` is not one.
pub fn call(_db: &mut MockDb, name: &str, args: &[V]) -> Option<Result<V, String>> {
    if !ROWS.iter().any(|(n, _)| *n == name) {
        return None;
    }
    let s0 = |k: usize| args.get(k).and_then(V::as_str).unwrap_or("").to_owned();
    Some(STATE.with(|st| {
        let mut st = st.borrow_mut();
        let h = st.history.clone();
        let waves = |st: &State| -> Vec<Wave> {
            match (&st.open, st.selected) {
                (Some(d), Some(r)) => h.dirs[d][r].1.clone(),
                _ => Vec::new(),
            }
        };
        let wave = |st: &State, k: &V| -> Option<Wave> {
            let k = tagged(k, "wave")?.first()?.as_int()? as usize;
            waves(st).get(k).cloned()
        };
        Ok(match name {
            "axlGetHistoryEntry" => {
                if s0(1) == h.name {
                    V::Int(HISTORY_HANDLE)
                } else {
                    V::Nil
                }
            }
            "axlGetHistoryResults" => V::str(&h.rdb),
            "axlGetPointPsfDir" => {
                let c = kw(args, "cornerName").and_then(V::as_str);
                let p = kw(args, "designPointId").and_then(V::as_int);
                match (args[0].as_int(), c, p) {
                    (Some(HISTORY_HANDLE), Some(c), Some(p)) => h
                        .points
                        .get(&(s0(1), c.to_owned(), p))
                        .map_or(V::Nil, |d| V::str(d)),
                    _ => V::Nil,
                }
            }
            "isDir" => {
                if h.points.values().any(|d| *d == s0(0)) {
                    V::T
                } else {
                    V::Nil
                }
            }
            "openResults" => {
                let d = s0(0);
                if h.dirs.contains_key(&d) {
                    st.open = Some(d);
                    st.selected = None;
                    V::T
                } else {
                    V::Nil
                }
            }
            "results" => match &st.open {
                Some(d) => V::list(h.dirs[d].iter().rev().map(|r| V::sym(&r.0)).collect()),
                None => V::Nil,
            },
            "selectResult" => {
                let r = args[0].as_str().unwrap_or("");
                let k = st
                    .open
                    .as_ref()
                    .and_then(|d| h.dirs[d].iter().position(|x| x.0 == r));
                st.selected = k;
                if k.is_some() { V::T } else { V::Nil }
            }
            "outputs" => V::list(waves(&st).iter().rev().map(|w| V::str(&w.0)).collect()),
            "v" | "i" => {
                let want = name == "i";
                let k = waves(&st).iter().position(|w| w.0 == s0(0) && w.1 == want);
                k.map_or(V::Nil, |k| V::list(vec![V::str("wave"), V::Int(k as i64)]))
            }
            "drGetWaveformXVec" | "drGetWaveformYVec" => match wave(&st, &args[0]) {
                Some(_) => {
                    let mut v = args[0].items().to_vec();
                    v[0] = V::str("vec");
                    v.push(V::str(if name.contains('X') { "x" } else { "y" }));
                    V::list(v)
                }
                None => return Err(format!("*Error* {name}: not a waveform")),
            },
            "drVectorLength" | "drGetElem" => {
                let Some(t) = tagged(&args[0], "vec") else {
                    return Err(format!("*Error* {name}: not a vector"));
                };
                let w = wave(&st, &V::list(vec![V::str("wave"), t[0].clone()]))
                    .ok_or("*Error* stale vector")?;
                let x = t[1].as_str() == Some("x");
                if name == "drVectorLength" {
                    V::Int(w.2.len() as i64)
                } else {
                    let k = args[1].as_int().unwrap_or(-1) as usize;
                    match (x, w.3.get(k)) {
                        (true, _) => V::Float(*w.2.get(k).ok_or("*Error* index")?),
                        (false, Some((re, None))) => V::Float(*re),
                        (false, Some((re, Some(im)))) => {
                            V::list(vec![V::str("complex"), V::Float(*re), V::Float(*im)])
                        }
                        (false, None) => return Err("*Error* drGetElem: index".into()),
                    }
                }
            }
            "complexp" => {
                if tagged(&args[0], "complex").is_some() {
                    V::T
                } else {
                    V::Nil
                }
            }
            "real" | "imag" => {
                let c = tagged(&args[0], "complex").ok_or("*Error* not complex")?;
                c[usize::from(name == "imag")].clone()
            }
            "system" => {
                st.commands.push(s0(0));
                V::Int(0)
            }
            _ => unreachable!("a row of ROWS"),
        })
    }))
}
