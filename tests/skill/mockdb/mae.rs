//! Mock maestro (`mae*`, `axl*`) functions for `sfy_ade.il` (spec `interop.md` §5.4, §9).
//!
//! Signatures (rows of `mockdb::SIGNATURES`, checked before a call reaches here) are the
//! IC23.1 *ADE SKILL Reference* syntax lines, as collected (MIT) by virtuoso-bridge-lite's
//! function database; the behaviour follows their descriptions. What the reference leaves open is marked `GUESS(Gn)`
//! (spec §13).
//!
//! Model: a setup per maestro view (`saved`, written by `maeSaveSetup`); `maeOpenSetup`
//! gives a session (`fnxSession<k>`) holding a working copy, dropped by `maeCloseSession`.
//! A read-only session (`?mode "r"`) accepts edits in memory but never saves (as observed
//! on IC23.1). `axl*` handles are integers naming a session's setup database, a corner, a
//! model or a corner variable. Lists of tests, corners, variables and outputs come back in
//! reverse creation order (a dumper that forgets to sort fails); a corner's models and a
//! test's analysis options keep their order (they are ordered in the setup database).
//!
//! The view's files (`call_files`): `maeSaveSetup` writes the view's `active.state` (one
//! `<Test>` element per test: a real one as it was read, else the mock's own, which holds
//! the test's analyses, outputs and options), and `maeOpenSetup` reads it back, so a state
//! written before a session opens is the state its tests get (GUESS(G40)).
//! `axlExportSetup` writes the setup database as Maestro's XML: the tree the last
//! `axlImportSetup` read, updated from what the mock models (tests, global, test and corner
//! variables, corners, specs and their weights); `axlImportSetup … 'overwrite` replaces
//! those in place, creating missing tests and duplicating none (probe G32), and keeps the
//! tree for what the mock does not model (parameters, run options, disabled tests).
//! `maeSaveSetup` also writes the view's `maestro.sdb` (that XML with the run mode), and a
//! `maestro.sdb` the mock did not write (a loader's restored file) is the setup the next
//! `maeOpenSetup` reads, as an `axlImportSetup` into an empty setup (lane L11). An import
//! keeps the tree's parts outside `<active>` it does not hold (live: a run plan survives
//! `axlImportSetup … 'overwrite`). A global variable can be disabled (`axlSetEnabled`,
//! `<var enabled="0">`), an output added with its Plot off (`maeAddOutput ?plot nil`, its
//! `plot` field in the mock's state).

use std::collections::BTreeMap;

use interop::virtuoso::ade::mae_xml::{self, Node};

use super::super::{V, print, read_all};
use super::MockDb;

/// The maestro rows of `mockdb::SIGNATURES` (IC23.1 ADE SKILL Reference syntax lines, via
/// virtuoso-bridge-lite's function database, MIT).
pub const ROWS: &[(&str, &str)] = &[
    ("maeOpenSetup", "t t t ?application ?histName ?mode"),
    ("maeCloseSession", "?session ?forceClose"),
    ("maeSaveSetup", "?lib ?cell ?view ?session"),
    ("maeGetSetup", "?typeName ?enabled ?run ?session"),
    (
        "maeCreateTest",
        "t ?sourceTest ?lib ?cell ?view ?simulator ?session",
    ),
    ("maeDeleteTest", "t ?run ?session"),
    ("maeSetAnalysis", "t t ?enable ?options ?session"),
    ("maeGetEnabledAnalysis", "t ?session"),
    ("maeGetAnalysis", "t t ?includeEmpty ?option ?session"),
    ("maeSetVar", "t g ?typeName ?typeValue ?session"),
    ("maeGetVar", "t ?typeName ?typeValue ?session"),
    (
        "maeSetCorner",
        "t ?enabled ?enableTests ?disableTests ?session",
    ),
    (
        "maeAddOutput",
        "t t ?outputType ?signalName ?expr ?plot ?save ?session",
    ),
    ("maeGetTestOutputs", "t ?session"),
    (
        "maeSetSpec",
        "t t ?minimum ?maximum ?gt ?lt ?range ?tolerance ?info ?weight ?corner ?session",
    ),
    ("maeSetEnvOption", "t ?options ?session"),
    ("maeGetEnvOption", "t ?includeEmpty ?option ?session"),
    ("maeSetSimOption", "t ?options ?session"),
    ("maeGetSimOption", "t ?includeEmpty ?option ?session"),
    ("axlGetTest", "x t"),
    ("axlGetTestToolArgs", "x"),
    ("axlGetMainSetupDB", "t"),
    ("axlGetCorner", "x t"),
    ("axlGetEnabled", "x"),
    ("axlSetEnabled", "x g"),
    ("axlGetModels", "x"),
    ("axlGetCornerDisabledTests", "x"),
    ("axlGetModelGroups", "x"),
    ("axlGetModel", "x t"),
    ("axlPutModel", "x t ?testName ?blockName"),
    ("axlGetModelFile", "x"),
    ("axlGetModelSection", "x"),
    ("axlSetModelFile", "x t"),
    ("axlSetModelSection", "x t"),
    ("axlGetVars", "x"),
    ("axlGetVar", "x t"),
    ("axlGetVarValue", "x"),
    ("axlGetSpecData", "x t t [t]"),
    // The setup database as XML (probe G32: axlImportSetup's template is "ttlts").
    ("axlExportSetup", "t x t l"),
    ("axlImportSetup", "t t l t S"),
    ("axlGetSpec", "x t"),
    ("axlGetSpecWeight", "x"),
    ("maeGetCurrentRunMode", "?session"),
    ("maeSetCurrentRunMode", "?runMode ?session"),
    ("maeSetSetup", "?tests ?corners ?enabled ?session"),
    ("maeSetParameter", "t t ?typeName ?typeValue ?session"),
];

/// First `axl*` handle number (handles look like the reference's `7918`).
const HANDLE_BASE: i64 = 7000;

/// Sessions one run opens at most (a bound on the session table).
const SESSIONS_MAX: usize = 1 << 12;

/// One maestro setup: tests, global variables, corners.
#[derive(Clone, Debug, Default)]
pub struct Setup {
    /// Tests in creation order.
    pub tests: Vec<Test>,
    /// Global variables `(name, value)` in creation order.
    pub vars: Vec<(String, String)>,
    /// The disabled global variables (`axlSetEnabled`, `<var enabled="0">`).
    pub vars_off: Vec<String>,
    /// Corners in creation order.
    pub corners: Vec<Corner>,
    /// The XML tree of the last `axlImportSetup`: what the mock does not model.
    pub sdb: Option<mae_xml::Node>,
    /// Spec weights by `<test>.<output>` (`maeSetSpec ?weight`, `<specWeight>`).
    pub weights: BTreeMap<String, String>,
    /// The current run mode, `None` for the default (`maeSetCurrentRunMode`); no export
    /// holds it (live: maestro.sdb's `<currentmode>` only).
    pub run_mode: Option<String>,
    /// Device parameters `(location, property, value, corner)` in creation order, corner
    /// `""` global (`maeSetParameter`, `<parameters>`).
    pub params: Vec<(String, String, String, String)>,
}

/// An analysis `(name, enabled, options)`, options in the order set.
pub type MockAnalysis = (String, bool, Vec<(String, V)>);

/// One test.
#[derive(Clone, Debug, Default)]
pub struct Test {
    /// Test name.
    pub name: String,
    /// Design `(lib, cell, view)` and simulator.
    pub design: (String, String, String),
    /// Simulator name.
    pub simulator: String,
    /// Analyses `(name, enabled, options)`, options in the order set.
    pub analyses: Vec<MockAnalysis>,
    /// Outputs in creation order.
    pub outputs: Vec<Output>,
    /// Environment options `(name, value)`.
    pub env: Vec<(String, V)>,
    /// Test-level variables `(name, value)` in creation order: they override a global of
    /// the same name in this test.
    pub vars: Vec<(String, String)>,
    /// Simulator options `(name, value)` (`temp`, `reltol`, …).
    pub sim: Vec<(String, V)>,
    /// The `<Test>` element of `active.state` read for this test when it is not the mock's
    /// own (a real Maestro state, kept as it is); `None`: the mock writes its own.
    pub state: Option<String>,
    /// Disabled (`maeSetSetup ?tests ?enabled nil`, `<disabledtests>`).
    pub disabled: bool,
}

/// One output of a test.
#[derive(Clone, Debug, Default)]
pub struct Output {
    /// Output name.
    pub name: String,
    /// `?outputType`.
    pub kind: String,
    /// `?signalName`, `?expr`.
    pub signal: Option<String>,
    /// See `signal`.
    pub expr: Option<String>,
    /// The spec: `(op, value)` pairs from the last `maeSetSpec`.
    pub spec: Vec<(String, V)>,
    /// Its Plot check box is clear (`?plot nil`): Maestro's disabled output.
    pub off: bool,
}

/// One corner.
#[derive(Clone, Debug, Default)]
pub struct Corner {
    /// Corner name.
    pub name: String,
    /// `?enabled`.
    pub enabled: bool,
    /// Corner variables `(name, value)` in creation order.
    pub vars: Vec<(String, String)>,
    /// Models `(name, file, section)` in creation order.
    pub models: Vec<(String, String, String)>,
    /// The tests that do not run the corner (`maeSetCorner ?disableTests`).
    pub disabled_tests: Vec<String>,
}

/// One open session.
#[derive(Clone, Debug)]
pub struct Session {
    /// The maestro view's library and cell.
    pub lib: String,
    /// See `lib`.
    pub cell: String,
    /// `"r"` or `"a"`.
    pub mode: String,
    /// The working copy.
    pub setup: Setup,
    /// The `<Test>` elements of the view's `active.state` when the session opened.
    pub states: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Handle {
    Sdb(usize),
    Corner(usize, String),
    Model(usize, String, String),
    Var(usize, String, String),
    Test(usize, String),
    TestVar(usize, String, String),
    Spec(usize, String),
    Global(usize, String),
}

/// Maestro state: saved setups by (lib, cell), sessions, `axl*` handles, and the log of
/// every maestro call (`name` per call, for tests to check what the loader did).
#[derive(Default)]
pub struct Mae {
    /// Saved setups by (lib, cell).
    pub saved: BTreeMap<(String, String), Setup>,
    /// Sessions by number; `None` once closed.
    pub sessions: Vec<Option<Session>>,
    /// Names of the maestro functions called, in order.
    pub calls: Vec<String>,
    /// Environment options every new test starts with, as a PDK's `libInit.il` sets them
    /// (live run: a PDK's default model files).
    pub default_env: Vec<(String, V)>,
    /// The `active.state` text `maeSaveSetup` last wrote, by path: a file that still holds
    /// it gives the session nothing the saved setup lacks.
    written: BTreeMap<String, String>,
    handles: Vec<Handle>,
    /// `(object, session, test, output)` of each struct `maeGetTestOutputs` answered: a
    /// `plot` set on one is the output's Plot (live: the session's own struct).
    out_objs: Vec<(u32, usize, String, String)>,
}

type Kw = Vec<(String, V)>;

/// Positional arguments and `?key value` pairs (the signature check already ran).
fn split(args: &[V]) -> (Vec<V>, Kw) {
    let mut pos = Vec::new();
    let mut kw = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a {
            V::Sym(k) if k.starts_with('?') => {
                kw.push((k[1..].to_owned(), it.next().cloned().unwrap_or(V::Nil)));
            }
            _ => pos.push(a.clone()),
        }
    }
    (pos, kw)
}

fn key<'a>(kw: &'a Kw, k: &str) -> Option<&'a V> {
    kw.iter().find(|(n, _)| n == k).map(|(_, v)| v)
}

fn st(v: Option<&V>) -> Option<String> {
    match v {
        Some(V::Str(s)) => Some(s.to_string()),
        _ => None,
    }
}

fn strs(v: &V) -> Option<Vec<String>> {
    v.items().iter().map(|x| st(Some(x))).collect()
}

fn names<'a>(it: impl DoubleEndedIterator<Item = &'a String>) -> V {
    V::list(it.rev().map(|s| V::str(s)).collect())
}

fn tf(b: bool) -> V {
    if b { V::T } else { V::Nil }
}

impl Mae {
    /// The session `?session` names, else the latest open one (the "current" session).
    fn session(&self, kw: &Kw) -> Option<usize> {
        match st(key(kw, "session")) {
            Some(s) => {
                let k: usize = s.strip_prefix("fnxSession")?.parse().ok()?;
                self.sessions.get(k)?.as_ref().map(|_| k)
            }
            None => self.sessions.iter().rposition(Option::is_some),
        }
    }

    fn setup(&mut self, k: usize) -> &mut Setup {
        &mut self.sessions[k].as_mut().expect("open session").setup
    }

    fn handle(&mut self, h: Handle) -> V {
        let k = match self.handles.iter().position(|x| *x == h) {
            Some(k) => k,
            None => {
                self.handles.push(h);
                self.handles.len() - 1
            }
        };
        V::Int(HANDLE_BASE + k as i64)
    }

    fn resolve(&self, v: &V) -> Option<Handle> {
        let k = usize::try_from(v.as_int()? - HANDLE_BASE).ok()?;
        let h = self.handles.get(k)?.clone();
        let s = match &h {
            Handle::Sdb(s)
            | Handle::Corner(s, _)
            | Handle::Model(s, ..)
            | Handle::Var(s, ..)
            | Handle::Test(s, _)
            | Handle::TestVar(s, ..)
            | Handle::Spec(s, _)
            | Handle::Global(s, _) => *s,
        };
        self.sessions.get(s)?.as_ref().map(|_| h)
    }

    fn corner(&mut self, s: usize, name: &str) -> Option<&mut Corner> {
        self.setup(s).corners.iter_mut().find(|c| c.name == name)
    }

    fn test(&mut self, s: usize, name: &str) -> Option<&mut Test> {
        self.setup(s).tests.iter_mut().find(|t| t.name == name)
    }
}

/// Handles a maestro function; `None` when `name` is not one.
pub fn call(db: &mut MockDb, name: &str, args: &[V]) -> Option<Result<V, String>> {
    if !(name.starts_with("mae") || name.starts_with("axl")) {
        return None;
    }
    db.mae.calls.push(name.to_owned());
    for (id, k, t, o) in db.mae.out_objs.clone() {
        let on = db.get_attr(id, "plot").truthy();
        if let Some(x) = db.mae.sessions.get_mut(k).and_then(Option::as_mut)
            && let Some(out) = x
                .setup
                .tests
                .iter_mut()
                .find(|x| x.name == t)
                .and_then(|x| x.outputs.iter_mut().find(|x| x.name == o))
        {
            out.off = !on;
        }
    }
    let (a, kw) = split(args);
    let s0 = |i: usize| st(a.get(i)).unwrap_or_default();
    let m = &mut db.mae;
    let r: Result<V, String> = match name {
        "maeOpenSetup" => {
            let (lib, cell, view) = (s0(0), s0(1), s0(2));
            let mode = st(key(&kw, "mode")).unwrap_or_else(|| "a".into());
            if !matches!(mode.as_str(), "r" | "a" | "w") {
                return Some(Err(format!("*Error* maeOpenSetup: mode {mode}")));
            }
            let locked = m
                .sessions
                .iter()
                .flatten()
                .any(|x| x.lib == lib && x.cell == cell && x.mode != "r");
            let full = m.sessions.len() >= SESSIONS_MAX;
            if db.lib(&lib).is_none() || (locked && mode != "r") || full {
                return Some(Ok(V::Nil));
            }
            // "If the given cellview does not exist, the function creates a new cellview."
            let v = db
                .dd_get(&lib, Some(&cell), Some(&view), None, "a")
                .expect("library exists");
            // The view's directory (live: ddGetObj(lib cell "maestro")~>readPath).
            db.put(v, "readPath", V::str(&format!("/mock/{lib}/{cell}/{view}")));
            let m = &mut db.mae;
            let setup = m
                .saved
                .get(&(lib.clone(), cell.clone()))
                .cloned()
                .unwrap_or_default();
            m.sessions.push(Some(Session {
                lib,
                cell,
                mode,
                setup,
                states: BTreeMap::new(),
            }));
            Ok(V::str(&format!("fnxSession{}", m.sessions.len() - 1)))
        }
        "maeCloseSession" => Ok(match m.session(&kw) {
            Some(k) => {
                m.sessions[k] = None;
                V::T
            }
            None => V::Nil,
        }),
        "maeSaveSetup" => {
            let Some(k) = m.session(&kw) else {
                return Some(Ok(V::Nil));
            };
            let sess = m.sessions[k].as_ref().expect("open").clone();
            for (kk, want) in [("lib", &sess.lib), ("cell", &sess.cell)] {
                if st(key(&kw, kk)).is_some_and(|v| &v != want) {
                    return Some(Err(
                        "*Error* maeSaveSetup: saving to another view is not modelled".into(),
                    ));
                }
            }
            if sess.mode == "r" {
                Ok(V::Nil)
            } else {
                m.saved.insert((sess.lib, sess.cell), sess.setup);
                Ok(V::T)
            }
        }
        "maeGetSetup" => {
            let Some(k) = m.session(&kw) else {
                return Some(Ok(V::Nil));
            };
            let su = m.setup(k);
            Ok(match st(key(&kw, "typeName")).as_deref() {
                None | Some("tests") => names(su.tests.iter().map(|t| &t.name)),
                // ADE lists its built-in nominal corner, whose handle is 0 (live run).
                Some("corners") => {
                    let nominal = "Nominal".to_owned();
                    names(std::iter::once(&nominal).chain(su.corners.iter().map(|c| &c.name)))
                }
                // ?enabled t: the enabled ones, nil: the disabled ones, else all (live).
                Some("variables") => {
                    let want = match key(&kw, "enabled") {
                        Some(V::T) => Some(true),
                        Some(V::Nil) => Some(false),
                        _ => None,
                    };
                    let off = su.vars_off.clone();
                    names(
                        su.vars
                            .iter()
                            .filter(|v| want.is_none_or(|w| w != off.contains(&v.0)))
                            .map(|v| &v.0),
                    )
                }
                Some(_) => V::Nil,
            })
        }
        "maeCreateTest" => {
            if key(&kw, "sourceTest").is_some() {
                return Some(Err(
                    "*Error* maeCreateTest: ?sourceTest is not modelled".into()
                ));
            }
            let Some(k) = m.session(&kw) else {
                return Some(Ok(V::Nil));
            };
            let n = s0(0);
            if m.test(k, &n).is_some() {
                return Some(Ok(V::Nil));
            }
            let get = |kk: &str| st(key(&kw, kk)).unwrap_or_default();
            let t = Test {
                name: n,
                design: (get("lib"), get("cell"), get("view")),
                simulator: get("simulator"),
                env: m.default_env.clone(),
                ..Test::default()
            };
            m.setup(k).tests.push(t);
            Ok(V::T)
        }
        "maeDeleteTest" => {
            let Some(k) = m.session(&kw) else {
                return Some(Ok(V::Nil));
            };
            let n = s0(0);
            let su = m.setup(k);
            let before = su.tests.len();
            su.tests.retain(|t| t.name != n);
            Ok(tf(su.tests.len() < before))
        }
        // GUESS(G20): option values are kept as given (strings from the loader).
        "maeSetAnalysis" => {
            let Some(k) = m.session(&kw) else {
                return Some(Ok(V::Nil));
            };
            let mut opts = Vec::new();
            for o in key(&kw, "options").map_or(&[][..], V::items) {
                match o.items() {
                    [V::Str(kk), v] => opts.push((kk.to_string(), v.clone())),
                    _ => {
                        return Some(Err(
                            "*Error* maeSetAnalysis: ?options is a list of (name value)".into(),
                        ));
                    }
                }
            }
            let (tn, an) = (s0(0), s0(1));
            let enable = key(&kw, "enable").map(V::truthy);
            let Some(t) = m.test(k, &tn) else {
                return Some(Ok(V::Nil));
            };
            match t.analyses.iter_mut().find(|x| x.0 == an) {
                Some(x) => {
                    if let Some(e) = enable {
                        x.1 = e;
                    }
                    for (kk, v) in opts {
                        match x.2.iter_mut().find(|o| o.0 == kk) {
                            Some(o) => o.1 = v,
                            None => x.2.push((kk, v)),
                        }
                    }
                }
                None => t.analyses.push((an, enable.unwrap_or(true), opts)),
            }
            Ok(V::T)
        }
        "maeGetEnabledAnalysis" => {
            let Some(k) = m.session(&kw) else {
                return Some(Ok(V::Nil));
            };
            Ok(match m.test(k, &s0(0)) {
                Some(t) => names(t.analyses.iter().filter(|x| x.1).map(|x| &x.0)),
                None => V::Nil,
            })
        }
        "maeGetAnalysis" => {
            let Some(k) = m.session(&kw) else {
                return Some(Ok(V::Nil));
            };
            let an = s0(1);
            // IC23.1 answers form fields it was never given: `anaName` first, and the ac
            // sweep chooser (virtuoso-bridge-lite maestro-skill-api.md: `(("anaName" "ac")
            // ("sweep" "Frequency") ("start" "1") …)`).
            Ok(m.test(k, &s0(0))
                .and_then(|t| t.analyses.iter().find(|x| x.0 == an))
                .map_or(V::Nil, |x| {
                    let mut l = vec![V::list(vec![V::str("anaName"), V::str(&an)])];
                    if an == "ac" && !x.2.iter().any(|o| o.0 == "sweep") {
                        l.push(V::list(vec![V::str("sweep"), V::str("Frequency")]));
                    }
                    l.extend(
                        x.2.iter()
                            .filter(|o| o.0 != "anaName")
                            .map(|(kk, v)| V::list(vec![V::str(kk), v.clone()])),
                    );
                    V::list(l)
                }))
        }
        "maeSetVar" => {
            let Some(k) = m.session(&kw) else {
                return Some(Ok(V::Nil));
            };
            let (n, v) = (s0(0), s0(1));
            if st(a.get(1)).is_none() {
                return Some(Err("*Error* maeSetVar: the mock takes string values".into()));
            }
            let upsert = |vars: &mut Vec<(String, String)>| match vars.iter_mut().find(|x| x.0 == n)
            {
                Some(x) => x.1 = v.clone(),
                None => vars.push((n.clone(), v.clone())),
            };
            match st(key(&kw, "typeName")).as_deref() {
                None => {
                    upsert(&mut m.setup(k).vars);
                    Ok(V::T)
                }
                Some("corner") => {
                    let Some(cs) = key(&kw, "typeValue").and_then(strs) else {
                        return Some(Err(
                            "*Error* maeSetVar: ?typeValue is a list of names".into()
                        ));
                    };
                    if cs.is_empty()
                        || cs
                            .iter()
                            .any(|c| !m.setup(k).corners.iter().any(|x| &x.name == c))
                    {
                        return Some(Ok(V::Nil));
                    }
                    for c in cs {
                        upsert(&mut m.corner(k, &c).expect("checked").vars);
                    }
                    Ok(V::T)
                }
                Some("test") => {
                    let Some(ts) = key(&kw, "typeValue").and_then(strs) else {
                        return Some(Err(
                            "*Error* maeSetVar: ?typeValue is a list of names".into()
                        ));
                    };
                    if ts.is_empty() || ts.iter().any(|t| m.test(k, t).is_none()) {
                        return Some(Ok(V::Nil));
                    }
                    for t in ts {
                        upsert(&mut m.test(k, &t).expect("checked").vars);
                    }
                    Ok(V::T)
                }
                Some(t) => Err(format!("*Error* maeSetVar: ?typeName {t} is not modelled")),
            }
        }
        "maeGetVar" => {
            let Some(k) = m.session(&kw) else {
                return Some(Ok(V::Nil));
            };
            // Live ICADVM20.1: the getter's ?typeValue is a string (a list signals), unlike
            // the setters'.
            if key(&kw, "typeValue").is_some_and(|v| !matches!(v, V::Str(_))) {
                return Some(Err(
                    "*Error* maeGetVar: argument for keyword ?typeValue should be a string".into(),
                ));
            }
            let n = s0(0);
            let find =
                |vars: &[(String, String)]| vars.iter().find(|x| x.0 == n).map(|x| V::str(&x.1));
            Ok(match (
                st(key(&kw, "typeName")).as_deref(),
                st(key(&kw, "typeValue")),
            ) {
                (None, _) => find(&m.setup(k).vars),
                (Some("corner"), Some(c)) => m.corner(k, &c).and_then(|x| find(&x.vars)),
                _ => None,
            }
            .unwrap_or(V::Nil))
        }
        "maeSetCorner" => {
            let Some(k) = m.session(&kw) else {
                return Some(Ok(V::Nil));
            };
            let n = s0(0);
            let en = key(&kw, "enabled").map(V::truthy);
            if m.corner(k, &n).is_none() {
                m.setup(k).corners.push(Corner {
                    name: n.clone(),
                    enabled: true,
                    ..Corner::default()
                });
            }
            // ?disableTests adds tests that skip the corner, ?enableTests removes them.
            let off = key(&kw, "disableTests").and_then(strs).unwrap_or_default();
            let on = key(&kw, "enableTests").and_then(strs).unwrap_or_default();
            let c = m.corner(k, &n).expect("made above");
            c.enabled = en.unwrap_or(c.enabled);
            c.disabled_tests.retain(|t| !on.contains(t));
            for t in off {
                if !c.disabled_tests.contains(&t) {
                    c.disabled_tests.push(t);
                }
            }
            Ok(V::T)
        }
        // ?tests with ?enabled sets those tests' enable (the probe's call).
        "maeSetSetup" => {
            let Some(k) = m.session(&kw) else {
                return Some(Ok(V::Nil));
            };
            let (Some(ts), Some(en)) = (key(&kw, "tests").and_then(strs), key(&kw, "enabled"))
            else {
                return Some(Err(
                    "*Error* maeSetSetup: the mock takes ?tests and ?enabled".into(),
                ));
            };
            let en = en.truthy();
            if ts.iter().any(|t| m.test(k, t).is_none()) {
                return Some(Ok(V::Nil));
            }
            for t in ts {
                m.test(k, &t).expect("checked").disabled = !en;
            }
            Ok(V::T)
        }
        // "lib/cell/view/inst[/inst…]/prop" and a string value, globally or per corner.
        "maeSetParameter" => {
            let Some(k) = m.session(&kw) else {
                return Some(Ok(V::Nil));
            };
            let (path, v) = (s0(0), s0(1));
            let Some((loc, prop)) = path
                .rsplit_once('/')
                .filter(|x| x.0.split('/').count() >= 4)
            else {
                return Some(Err(format!("*Error* maeSetParameter: path {path}")));
            };
            let corners = match st(key(&kw, "typeName")).as_deref() {
                None => vec![String::new()],
                Some("corner") => match key(&kw, "typeValue").and_then(strs) {
                    Some(cs)
                        if cs
                            .iter()
                            .all(|c| m.setup(k).corners.iter().any(|x| &x.name == c)) =>
                    {
                        cs
                    }
                    _ => return Some(Ok(V::Nil)),
                },
                Some(t) => return Some(Err(format!("*Error* maeSetParameter: ?typeName {t}"))),
            };
            let ps = &mut m.setup(k).params;
            for c in corners {
                match ps
                    .iter_mut()
                    .find(|p| p.0 == loc && p.1 == prop && p.3 == c)
                {
                    Some(p) => p.2 = v.clone(),
                    None => ps.push((loc.to_owned(), prop.to_owned(), v.clone(), c)),
                }
            }
            Ok(V::T)
        }
        "maeAddOutput" => {
            let Some(k) = m.session(&kw) else {
                return Some(Ok(V::Nil));
            };
            let kind = st(key(&kw, "outputType")).unwrap_or_default();
            let (sig, ex) = (st(key(&kw, "signalName")), st(key(&kw, "expr")));
            let ok = match kind.as_str() {
                "point" => ex.is_some() && sig.is_none(),
                "net" => sig.is_some() && ex.is_none(),
                _ => false,
            };
            if !ok {
                return Some(Err(
                    "*Error* maeAddOutput: point takes ?expr, net ?signalName".into(),
                ));
            }
            let n = s0(0);
            let Some(t) = m.test(k, &s0(1)) else {
                return Some(Ok(V::Nil));
            };
            if t.outputs.iter().any(|o| o.name == n) {
                return Some(Ok(V::Nil));
            }
            t.outputs.push(Output {
                name: n,
                kind,
                signal: sig,
                expr: ex,
                spec: Vec::new(),
                off: matches!(key(&kw, "plot"), Some(V::Nil)),
            });
            Ok(V::T)
        }
        "maeGetTestOutputs" => {
            let Some(k) = m.session(&kw) else {
                return Some(Ok(V::Nil));
            };
            let outs = m
                .test(k, &s0(0))
                .map(|t| t.outputs.clone())
                .unwrap_or_default();
            let opt = |x: &Option<String>| x.as_deref().map_or(V::Nil, V::str);
            let mut objs = Vec::new();
            for o in outs.iter().rev() {
                let id = db.new_obj(
                    "sevOutputStruct",
                    vec![
                        ("name", V::str(&o.name)),
                        ("outputType", V::str(&o.kind)),
                        // The names IC23.1 answers (virtuoso-bridge-lite reads
                        // `o~>signal o~>expression` live); `~>expr`/`~>signalName` are nil.
                        ("signal", opt(&o.signal)),
                        ("expression", opt(&o.expr)),
                        ("plot", tf(!o.off)),
                    ],
                );
                db.mae.out_objs.push((id, k, s0(0), o.name.clone()));
                objs.push(V::Obj(id));
            }
            Ok(V::list(objs))
        }
        "maeSetSpec" => {
            let Some(k) = m.session(&kw) else {
                return Some(Ok(V::Nil));
            };
            let mut spec = Vec::new();
            for op in ["gt", "lt", "range", "minimum", "maximum", "tolerance"] {
                if let Some(v) = key(&kw, op) {
                    let ok = match op {
                        // GUESS(G21): ?range takes (lo hi). G37 (live): ?tolerance takes
                        // (target tol), a bare string fails.
                        "range" | "tolerance" => strs(v).is_some_and(|x| x.len() == 2),
                        _ => matches!(v, V::Str(_)),
                    };
                    if !ok {
                        return Some(Err(format!("*Error* maeSetSpec: bad ?{op}")));
                    }
                    spec.push((op.to_owned(), v.clone()));
                }
            }
            if spec.len() != 1 {
                return Some(Err(
                    "*Error* maeSetSpec: one of ?gt ?lt ?range ?minimum ?maximum ?tolerance".into(),
                ));
            }
            let o = s0(0);
            let weight = st(key(&kw, "weight"));
            let id = format!("{}.{o}", s0(1));
            match m
                .test(k, &s0(1))
                .and_then(|t| t.outputs.iter_mut().find(|x| x.name == o))
            {
                Some(x) => {
                    x.spec = spec;
                    // GUESS: a spec set without ?weight has none.
                    match weight {
                        Some(w) => m.setup(k).weights.insert(id, w),
                        None => m.setup(k).weights.remove(&id),
                    };
                    Ok(V::T)
                }
                None => Ok(V::Nil),
            }
        }
        "maeSetEnvOption" => {
            let Some(k) = m.session(&kw) else {
                return Some(Ok(V::Nil));
            };
            let mut set = Vec::new();
            for o in key(&kw, "options").map_or(&[][..], V::items) {
                let [V::Str(n), v] = o.items() else {
                    return Some(Err(
                        "*Error* maeSetEnvOption: ?options is a list of (name value)".into(),
                    ));
                };
                let ok = match &**n {
                    "modelFiles" => v
                        .items()
                        .iter()
                        .all(|e| strs(e).is_some_and(|x| (1..=2).contains(&x.len()))),
                    // GUESS(G22): a list of paths.
                    "definitionFiles" => strs(v).is_some(),
                    _ => true,
                };
                if !ok {
                    return Some(Err(format!("*Error* maeSetEnvOption: bad {n}")));
                }
                set.push((n.to_string(), v.clone()));
            }
            let Some(t) = m.test(k, &s0(0)) else {
                return Some(Ok(V::Nil));
            };
            for (n, v) in set {
                t.env.retain(|x| x.0 != n);
                t.env.push((n, v));
            }
            Ok(V::T)
        }
        "maeGetEnvOption" => {
            let Some(k) = m.session(&kw) else {
                return Some(Ok(V::Nil));
            };
            let opt = st(key(&kw, "option"));
            let Some(t) = m.test(k, &s0(0)) else {
                return Some(Ok(V::Nil));
            };
            Ok(match opt {
                Some(o) => t
                    .env
                    .iter()
                    .find(|x| x.0 == o)
                    .map_or(V::Nil, |x| x.1.clone()),
                None => V::list(
                    t.env
                        .iter()
                        .map(|(n, v)| V::list(vec![V::str(n), v.clone()]))
                        .collect(),
                ),
            })
        }
        // GUESS(G28): maeGetSimOption ?option names one option and answers its value as
        // maeGetEnvOption does; maeSetSimOption takes (name value) strings.
        "maeSetSimOption" => {
            let Some(k) = m.session(&kw) else {
                return Some(Ok(V::Nil));
            };
            let mut set = Vec::new();
            for o in key(&kw, "options").map_or(&[][..], V::items) {
                let [V::Str(n), v @ V::Str(_)] = o.items() else {
                    return Some(Err(
                        "*Error* maeSetSimOption: ?options is a list of (name value) strings"
                            .into(),
                    ));
                };
                set.push((n.to_string(), v.clone()));
            }
            let Some(t) = m.test(k, &s0(0)) else {
                return Some(Ok(V::Nil));
            };
            for (n, v) in set {
                t.sim.retain(|x| x.0 != n);
                t.sim.push((n, v));
            }
            Ok(V::T)
        }
        "maeGetSimOption" => {
            let Some(k) = m.session(&kw) else {
                return Some(Ok(V::Nil));
            };
            let opt = st(key(&kw, "option"));
            let Some(t) = m.test(k, &s0(0)) else {
                return Some(Ok(V::Nil));
            };
            Ok(match opt {
                Some(o) => t
                    .sim
                    .iter()
                    .find(|x| x.0 == o)
                    .map_or(V::Nil, |x| x.1.clone()),
                None => V::list(
                    t.sim
                        .iter()
                        .map(|(n, v)| V::list(vec![V::str(n), v.clone()]))
                        .collect(),
                ),
            })
        }
        "axlGetTest" => match m.resolve(&a[0]) {
            Some(Handle::Sdb(k)) => {
                let n = s0(1);
                Ok(if m.test(k, &n).is_some() {
                    m.handle(Handle::Test(k, n))
                } else {
                    V::Nil
                })
            }
            _ => Err("*Error* axlGetTest: argument #1 should be a setup database handle".into()),
        },
        // G31 settled: the design binding as pairs; a test on a config has no state or path.
        "axlGetTestToolArgs" => match m.resolve(&a[0]) {
            Some(Handle::Test(k, n)) => Ok(m.test(k, &n).map_or(V::Nil, |t| {
                let (lib, cell, view) = &t.design;
                let mut args = vec![("view", view.as_str()), ("sim", t.simulator.as_str())];
                if view != "config" {
                    args.extend([("state", "active"), ("path", "$AXL_SETUPDB_DIR")]);
                }
                args.extend([("lib", lib.as_str()), ("cell", cell.as_str())]);
                V::list(
                    args.iter()
                        .map(|(k, v)| V::list(vec![V::str(k), V::str(v)]))
                        .collect(),
                )
            })),
            _ => Err("*Error* axlGetTestToolArgs: argument #1 should be a test handle".into()),
        },
        "axlGetMainSetupDB" => {
            let kw2 = vec![("session".to_owned(), V::str(&s0(0)))];
            Ok(match m.session(&kw2) {
                Some(k) => m.handle(Handle::Sdb(k)),
                None => V::Nil,
            })
        }
        "axlGetCorner" => match m.resolve(&a[0]) {
            Some(Handle::Sdb(k)) => {
                let n = s0(1);
                Ok(if n == "Nominal" {
                    V::Int(0)
                } else if m.corner(k, &n).is_some() {
                    m.handle(Handle::Corner(k, n))
                } else {
                    V::Nil
                })
            }
            _ => Err("*Error* axlGetCorner: argument #1 should be a setup database handle".into()),
        },
        "axlGetEnabled" => match m.resolve(&a[0]) {
            Some(Handle::Corner(k, c)) => Ok(tf(m.corner(k, &c).is_some_and(|x| x.enabled))),
            Some(Handle::Test(k, t)) => Ok(tf(m.test(k, &t).is_some_and(|x| !x.disabled))),
            Some(Handle::Global(k, n)) => Ok(tf(!m.setup(k).vars_off.contains(&n))),
            _ => Err(
                "*Error* axlGetEnabled: the mock models corner, test and var handles only".into(),
            ),
        },
        // Live ICADVM20.1 (probe l11): a global variable's handle, answering 1.
        "axlSetEnabled" => match m.resolve(&a[0]) {
            Some(Handle::Global(k, n)) => {
                let off = &mut m.setup(k).vars_off;
                off.retain(|x| x != &n);
                if !a[1].truthy() {
                    off.push(n);
                }
                Ok(V::Int(1))
            }
            _ => Err("*Error* axlSetEnabled: the mock models global variable handles only".into()),
        },
        "axlGetModels" => match m.resolve(&a[0]) {
            // (handle names), the names in setup order, as axlGetVars (live ICADVM20.1
            // probe: (1053 ("m.scs"))).
            Some(Handle::Corner(k, c)) => {
                let names = m.corner(k, &c).map_or(V::Nil, |x| {
                    V::list(x.models.iter().map(|md| V::str(&md.0)).collect())
                });
                Ok(V::list(vec![V::Int(1053), names]))
            }
            _ => Err("*Error* axlGetModels: argument #1 should be a corner handle".into()),
        },
        // (handle names), live: (1331 ("N1" "N2")), (0 nil) for none.
        "axlGetCornerDisabledTests" => match m.resolve(&a[0]) {
            Some(Handle::Corner(k, c)) => {
                let t = m
                    .corner(k, &c)
                    .map_or(Vec::new(), |x| x.disabled_tests.clone());
                Ok(if t.is_empty() {
                    V::list(vec![V::Int(0), V::Nil])
                } else {
                    V::list(vec![
                        V::Int(1331),
                        V::list(t.iter().map(|x| V::str(x)).collect()),
                    ])
                })
            }
            _ => Err(
                "*Error* axlGetCornerDisabledTests: argument #1 should be a corner handle".into(),
            ),
        },
        // No model groups are modelled: (0 nil), as live without any.
        "axlGetModelGroups" => Ok(V::list(vec![V::Int(0), V::Nil])),
        "axlGetModel" | "axlPutModel" => match m.resolve(&a[0]) {
            Some(Handle::Corner(k, c)) => {
                let n = s0(1);
                let x = m.corner(k, &c).expect("live handle");
                let has = x.models.iter().any(|md| md.0 == n);
                if !has && name == "axlGetModel" {
                    Ok(V::Int(0)) // "=> x_modelFile / 0"
                } else {
                    if !has {
                        x.models.push((n.clone(), String::new(), String::new()));
                    }
                    Ok(m.handle(Handle::Model(k, c, n)))
                }
            }
            _ => Err(format!(
                "*Error* {name}: argument #1 should be a corner handle"
            )),
        },
        "axlGetModelFile" | "axlGetModelSection" | "axlSetModelFile" | "axlSetModelSection" => {
            match m.resolve(&a[0]) {
                Some(Handle::Model(k, c, n)) => {
                    let h = a[0].clone();
                    let md = m
                        .corner(k, &c)
                        .and_then(|x| x.models.iter_mut().find(|md| md.0 == n))
                        .expect("live handle");
                    let field = if name.ends_with("File") {
                        &mut md.1
                    } else {
                        &mut md.2
                    };
                    if name.starts_with("axlSet") {
                        *field = s0(1);
                        Ok(if name.ends_with("File") {
                            V::str(field)
                        } else {
                            h
                        })
                    } else if field.is_empty() {
                        Ok(V::Nil)
                    } else {
                        Ok(V::str(field))
                    }
                }
                _ => Err(format!(
                    "*Error* {name}: argument #1 should be a model handle"
                )),
            }
        }
        "axlGetVars" => match m.resolve(&a[0]) {
            // "a list containing a handle to all variables … and a list of all variable names".
            Some(Handle::Corner(k, c)) => {
                let ns = m
                    .corner(k, &c)
                    .map_or(V::Nil, |x| names(x.vars.iter().map(|v| &v.0)));
                let h = m.handle(Handle::Corner(k, c));
                Ok(V::list(vec![h, ns]))
            }
            Some(Handle::Test(k, t)) => {
                let ns = m
                    .test(k, &t)
                    .map_or(V::Nil, |x| names(x.vars.iter().map(|v| &v.0)));
                let h = m.handle(Handle::Test(k, t));
                Ok(V::list(vec![h, ns]))
            }
            _ => Err("*Error* axlGetVars: the mock models corner and test handles only".into()),
        },
        "axlGetVar" => match m.resolve(&a[0]) {
            Some(Handle::Corner(k, c)) => {
                let n = s0(1);
                let has = m
                    .corner(k, &c)
                    .is_some_and(|x| x.vars.iter().any(|v| v.0 == n));
                Ok(if has {
                    m.handle(Handle::Var(k, c, n))
                } else {
                    V::Nil
                })
            }
            Some(Handle::Test(k, t)) => {
                let n = s0(1);
                let has = m
                    .test(k, &t)
                    .is_some_and(|x| x.vars.iter().any(|v| v.0 == n));
                Ok(if has {
                    m.handle(Handle::TestVar(k, t, n))
                } else {
                    V::Nil
                })
            }
            Some(Handle::Sdb(k)) => {
                let n = s0(1);
                let has = m.setup(k).vars.iter().any(|v| v.0 == n);
                Ok(if has {
                    m.handle(Handle::Global(k, n))
                } else {
                    V::Nil
                })
            }
            _ => Err("*Error* axlGetVar: the mock models sdb, corner and test handles only".into()),
        },
        "axlGetVarValue" => match m.resolve(&a[0]) {
            Some(Handle::Var(k, c, n)) => Ok(m
                .corner(k, &c)
                .and_then(|x| x.vars.iter().find(|v| v.0 == n))
                .map_or(V::Nil, |v| V::str(&v.1))),
            Some(Handle::TestVar(k, t, n)) => Ok(m
                .test(k, &t)
                .and_then(|x| x.vars.iter().find(|v| v.0 == n))
                .map_or(V::Nil, |v| V::str(&v.1))),
            _ => Err("*Error* axlGetVarValue: argument #1 should be a variable handle".into()),
        },
        "axlGetSpecData" => match m.resolve(&a[0]) {
            // One flat (op value...) list, the last spec set: ("gt" "1"), ("range" "1" "2")
            // (live ICADVM20.1 probe; G21 had a list of (op value) pairs).
            Some(Handle::Sdb(k)) => {
                let o = s0(1);
                Ok(m.test(k, &s0(2))
                    .and_then(|t| t.outputs.iter().find(|x| x.name == o))
                    .and_then(|x| x.spec.last())
                    .map_or(V::Nil, |(op, v)| {
                        // Live: ("min" v), ("max" v), ("tol" target tol) (probe G37).
                        let op = match op.as_str() {
                            "minimum" => "min",
                            "maximum" => "max",
                            "tolerance" => "tol",
                            o => o,
                        };
                        let mut items = vec![V::str(op)];
                        match v {
                            V::List(l) => items.extend(l.iter().cloned()),
                            other => items.push(other.clone()),
                        }
                        V::list(items)
                    }))
            }
            _ => {
                Err("*Error* axlGetSpecData: argument #1 should be a setup database handle".into())
            }
        },
        "maeGetCurrentRunMode" => {
            let Some(k) = m.session(&kw) else {
                return Some(Ok(V::Nil));
            };
            let mode = m.setup(k).run_mode.clone();
            Ok(V::str(
                mode.as_deref().unwrap_or("Single Run, Sweeps and Corners"),
            ))
        }
        "maeSetCurrentRunMode" => {
            let Some(k) = m.session(&kw) else {
                return Some(Ok(V::Nil));
            };
            let Some(mode) = st(key(&kw, "runMode")) else {
                return Some(Err(
                    "*Error* maeSetCurrentRunMode: ?runMode is a string".into()
                ));
            };
            m.setup(k).run_mode = (mode != "Single Run, Sweeps and Corners").then_some(mode);
            Ok(V::T)
        }
        "axlGetSpec" => match m.resolve(&a[0]) {
            Some(Handle::Sdb(k)) => {
                let id = s0(1);
                let known = id.split_once('.').is_some_and(|(t, o)| {
                    m.test(k, t).is_some_and(|t| {
                        t.outputs.iter().any(|x| x.name == o && !x.spec.is_empty())
                    })
                }) || m.setup(k).weights.contains_key(&id);
                Ok(if known {
                    m.handle(Handle::Spec(k, id))
                } else {
                    V::Nil
                })
            }
            _ => Err("*Error* axlGetSpec: argument #1 should be a setup database handle".into()),
        },
        // Live: "" when no weight is set.
        "axlGetSpecWeight" => match m.resolve(&a[0]) {
            Some(Handle::Spec(k, id)) => Ok(V::str(
                m.setup(k).weights.get(&id).map_or("", String::as_str),
            )),
            _ => Err("*Error* axlGetSpecWeight: argument #1 should be a spec handle".into()),
        },
        _ => return None,
    };
    Some(r)
}

/// The `<Test>` element the mock writes for a test it holds the state of: its analyses,
/// outputs and options as one printed list, read back by [`from_state`].
fn mock_state(t: &Test) -> String {
    let b = |x: bool| V::str(if x { "1" } else { "0" });
    let opt = |x: &Option<String>| x.as_deref().map_or(V::Nil, V::str);
    let pairs = |v: &[(String, V)]| {
        V::list(
            v.iter()
                .map(|(k, x)| V::list(vec![V::str(k), x.clone()]))
                .collect(),
        )
    };
    let data = V::list(vec![
        V::list(vec![
            V::str(&t.design.0),
            V::str(&t.design.1),
            V::str(&t.design.2),
            V::str(&t.simulator),
        ]),
        // The form fields maeGetAnalysis echoes (anaName, the ac/noise sweep chooser) are
        // the state's whatever sets them to their echoed value: left out.
        V::list(
            t.analyses
                .iter()
                .map(|(n, en, o)| {
                    let o: Vec<(String, V)> = o
                        .iter()
                        .filter(|(k, v)| {
                            !((k == "anaName" && v.as_str() == Some(n))
                                || (k == "sweep" && v.as_str() == Some("Frequency")))
                        })
                        .cloned()
                        .collect();
                    V::list(vec![V::str(n), b(*en), pairs(&o)])
                })
                .collect(),
        ),
        V::list(
            t.outputs
                .iter()
                .map(|o| {
                    V::list(vec![
                        V::str(&o.name),
                        V::str(&o.kind),
                        opt(&o.signal),
                        opt(&o.expr),
                    ])
                })
                .collect(),
        ),
        pairs(&t.env),
        pairs(&t.sim),
    ]);
    let mut text = String::new();
    print(&data, &mut text);
    let leaf = mae_xml::Node::leaf("mock", &text);
    // The outputs' Plot as Maestro's state holds it (`sevOutputStruct` fields), where the
    // export's state patch finds it (lane L11).
    let field = |n: &str, ty: &str, v: &str| Node {
        attrs: vec![("Name".into(), n.into()), ("Type".into(), ty.into())],
        ..Node::leaf("field", v)
    };
    let structs: Vec<Node> = t
        .outputs
        .iter()
        .enumerate()
        .map(|(k, o)| Node {
            children: vec![
                field("name", "string", &format!("\"{}\"", o.name)),
                if o.off {
                    field("plot", "list", "nil")
                } else {
                    field("plot", "symbol", "t")
                },
            ],
            ..field(&format!("outputList_{k}"), "defstruct", "sevOutputStruct")
        })
        .collect();
    let outputs = Node {
        children: structs,
        ..field("outputList", "skillList", "")
    };
    let el = Node {
        name: "Test".into(),
        attrs: vec![("Name".into(), t.name.clone())],
        children: vec![leaf, outputs],
        ..Node::default()
    };
    // The element as active.state holds it: one tab in, under <statedb>.
    let w = mae_xml::write(&Node {
        name: "statedb".into(),
        children: vec![el],
        ..Node::default()
    });
    let body = w.split_once('\n').map_or("", |x| x.1);
    let body = body.split_once('\n').map_or("", |x| x.1);
    body.strip_suffix("</statedb>\n").unwrap_or(body).to_owned()
}

/// The test `name` a state element describes: the mock's own rebuilt, a real one kept as
/// its text (with no analyses or outputs: the mock does not read Maestro's state).
fn from_state(name: &str, el: &str) -> Test {
    let mut t = Test {
        name: name.to_owned(),
        ..Test::default()
    };
    let wrapped = format!("<s>\n{el}</s>\n");
    let parsed = mae_xml::parse(&wrapped).ok();
    // (output, Plot off) from the state's `sevOutputStruct` fields.
    let off: Vec<(String, bool)> = parsed
        .iter()
        .flat_map(|n| n.children.first())
        .flat_map(|t| t.all("field"))
        .filter(|f| f.attr("Name") == Some("outputList"))
        .flat_map(|f| f.all("field"))
        .filter_map(|s| {
            let get = |k: &str| s.all("field").find(|x| x.attr("Name") == Some(k));
            let n = get("name")?.text.trim_matches('"').to_owned();
            Some((n, get("plot")?.text == "nil"))
        })
        .collect();
    let data = parsed
        .and_then(|n| {
            n.children
                .first()
                .and_then(|t| t.get("mock").map(str::to_owned))
        })
        .and_then(|txt| read_all(&txt).ok())
        .and_then(|f| f.into_iter().next());
    let Some(data) = data else {
        t.state = Some(el.to_owned());
        return t;
    };
    let s = |v: &V| v.as_str().unwrap_or_default().to_owned();
    let pairs = |v: &V| -> Vec<(String, V)> {
        v.items()
            .iter()
            .map(|p| (s(&p.items()[0]), p.items()[1].clone()))
            .collect()
    };
    let it = data.items();
    let d = it[0].items();
    t.design = (s(&d[0]), s(&d[1]), s(&d[2]));
    t.simulator = s(&d[3]);
    t.analyses = it[1]
        .items()
        .iter()
        .map(|a| {
            (
                s(&a.items()[0]),
                s(&a.items()[1]) == "1",
                pairs(&a.items()[2]),
            )
        })
        .collect();
    let opt = |v: &V| v.as_str().map(str::to_owned);
    t.outputs = it[2]
        .items()
        .iter()
        .map(|o| Output {
            name: s(&o.items()[0]),
            kind: s(&o.items()[1]),
            signal: opt(&o.items()[2]),
            expr: opt(&o.items()[3]),
            spec: Vec::new(),
            off: off.iter().any(|x| x.0 == s(&o.items()[0]) && x.1),
        })
        .collect();
    t.env = pairs(&it[3]);
    t.sim = pairs(&it[4]);
    t
}

/// The DD path of a maestro view's file.
fn view_file(db: &mut MockDb, lib: &str, cell: &str, file: &str) -> Option<String> {
    let f = db.dd_get(lib, Some(cell), Some("maestro"), Some(file), "a")?;
    super::s(&db.get_attr(f, "readPath")).map(str::to_owned)
}

/// Maestro functions that read or write files: `maeOpenSetup` and `maeSaveSetup` (the
/// view's `active.state`), `axlExportSetup` and `axlImportSetup`. `None` for any other.
pub fn call_files(
    db: &mut MockDb,
    name: &str,
    args: &[V],
    files: &mut BTreeMap<String, String>,
) -> Option<Result<V, String>> {
    if !matches!(
        name,
        "maeOpenSetup" | "maeSaveSetup" | "axlExportSetup" | "axlImportSetup"
    ) {
        return None;
    }
    if let Some(Err(e)) = db.call_db(name, args) {
        return Some(Err(e));
    }
    let (a, kw) = split(args);
    let s0 = |i: usize| st(a.get(i)).unwrap_or_default();
    match name {
        "maeOpenSetup" => {
            let r = call(db, name, args)?;
            let Ok(V::Str(sess)) = &r else { return Some(r) };
            let k: usize = sess["fnxSession".len()..].parse().expect("own name");
            let (lib, cell) = {
                let x = db.mae.sessions[k].as_ref().expect("open");
                (x.lib.clone(), x.cell.clone())
            };
            let path = view_file(db, &lib, &cell, "active.state")?;
            let own = files
                .get(&path)
                .is_some_and(|t| db.mae.written.get(&path) == Some(t));
            let states: BTreeMap<String, String> = files
                .get(&path)
                .and_then(|t| mae_xml::state_tests(t).ok())
                .unwrap_or_default()
                .into_iter()
                .collect();
            // A maestro.sdb the mock did not write is the view's setup (lane L11).
            let sdb_path = view_file(db, &lib, &cell, "maestro.sdb")?;
            let restored = files
                .get(&sdb_path)
                .filter(|t| db.mae.written.get(&sdb_path) != Some(*t))
                .and_then(|t| mae_xml::parse(t).ok())
                .and_then(|root| Some((mae_xml::Setup::from_root(&root).ok()?, root)));
            let x = db.mae.sessions[k].as_mut().expect("open");
            if let Some((xs, root)) = restored {
                let mut su = Setup::default();
                apply_xml(&mut su, root, &xs, &states);
                su.run_mode = xs
                    .current_mode
                    .filter(|m| m != "Single Run, Sweeps and Corners");
                x.setup = su;
            }
            for t in x.setup.tests.iter_mut().filter(|_| !own) {
                if let Some(el) = states.get(&t.name) {
                    let mut n = from_state(&t.name, el);
                    if n.state.is_some() {
                        // A real state: the mock keeps the test as saved, with that text.
                        t.state = n.state;
                    } else {
                        n.vars = std::mem::take(&mut t.vars);
                        *t = n;
                    }
                }
            }
            x.states = states;
            Some(r)
        }
        "maeSaveSetup" => {
            let r = call(db, name, args)?;
            if !matches!(r, Ok(V::T)) {
                return Some(r);
            }
            let k = db.mae.session(&kw).expect("saved");
            let x = db.mae.sessions[k].as_ref().expect("open").clone();
            let path = view_file(db, &x.lib, &x.cell, "active.state")?;
            let mut text = String::from("<?xml version=\"1.0\"?>\n<statedb version=\"5\">active\n");
            for t in &x.setup.tests {
                text.push_str(&t.state.clone().unwrap_or_else(|| mock_state(t)));
            }
            text.push_str("</statedb>\n");
            db.mae.written.insert(path.clone(), text.clone());
            files.insert(path, text);
            // maestro.sdb: the setup's XML with its run mode (live: `<currentmode>`).
            let path = view_file(db, &x.lib, &x.cell, "maestro.sdb")?;
            let mut tree = export_tree(&x.setup, "maestro");
            if let (Some(mode), Some(a)) = (&x.setup.run_mode, tree.child_mut("active")) {
                a.children.insert(0, Node::leaf("currentmode", mode));
            }
            let text = mae_xml::write(&tree);
            db.mae.written.insert(path.clone(), text.clone());
            files.insert(path, text);
            Some(r)
        }
        "axlExportSetup" => {
            db.mae.calls.push(name.to_owned());
            let k = match db.mae.resolve(&a[1]) {
                Some(Handle::Sdb(k)) => k,
                _ => {
                    return Some(Err(
                        "*Error* axlExportSetup: argument #2 should be a setup database handle"
                            .into(),
                    ));
                }
            };
            let path = s0(2);
            let stem = path.rsplit('/').next().unwrap_or("");
            let stem = stem.rsplit_once('.').map_or(stem, |x| x.0);
            let su = db.mae.setup(k).clone();
            files.insert(path.clone(), mae_xml::write(&export_tree(&su, stem)));
            // Live (probe l11): the tests' states go beside the file as active.state.
            if let Some((dir, _)) = path.rsplit_once('/') {
                let mut st =
                    String::from("<?xml version=\"1.0\"?>\n<statedb version=\"5\">active\n");
                for t in &su.tests {
                    st.push_str(&t.state.clone().unwrap_or_else(|| mock_state(t)));
                }
                st.push_str("</statedb>\n");
                files.insert(format!("{dir}/active.state"), st);
            }
            Some(Ok(V::T))
        }
        "axlImportSetup" => {
            db.mae.calls.push(name.to_owned());
            // Live: 'append and 'replace signal "Unknown operation" (probe G32).
            if a.get(4).and_then(V::as_str) != Some("overwrite") {
                return Some(Err(format!(
                    "*Error* error: Unknown operation {:?}",
                    a.get(4).and_then(V::as_str).unwrap_or("")
                )));
            }
            let kw2 = vec![("session".to_owned(), V::str(&s0(0)))];
            let Some(k) = db.mae.session(&kw2) else {
                return Some(Ok(V::Nil));
            };
            let Some(root) = files.get(&s0(1)).and_then(|t| mae_xml::parse(t).ok()) else {
                // Live: the error goes to the CIW, the call does not signal.
                return Some(Ok(V::Nil));
            };
            let Ok(x) = mae_xml::Setup::from_root(&root) else {
                return Some(Ok(V::Nil));
            };
            let mut states = db.mae.sessions[k].as_ref().expect("open").states.clone();
            // Live (probe l11): an active.state beside the file restores its tests over
            // the session's.
            let beside: BTreeMap<String, String> = s0(1)
                .rsplit_once('/')
                .and_then(|(dir, _)| files.get(&format!("{dir}/active.state")))
                .and_then(|t| mae_xml::state_tests(t).ok())
                .unwrap_or_default()
                .into_iter()
                .collect();
            let su = db.mae.setup(k);
            for t in su.tests.iter_mut() {
                if let Some(el) = beside.get(&t.name) {
                    let mut n = from_state(&t.name, el);
                    n.vars = std::mem::take(&mut t.vars);
                    n.disabled = t.disabled;
                    *t = n;
                }
            }
            states.extend(beside);
            apply_xml(su, root, &x, &states);
            Some(Ok(V::T))
        }
        _ => None,
    }
}

/// What `axlImportSetup … 'overwrite` does to setup `su` with the XML `root` (`x` its typed
/// view): tests (made from `states` when missing), vars, corners, parameters and specs in
/// place; the tree kept for the rest, with what the old tree held outside `<active>` that
/// `root` does not (live: a run plan survives).
fn apply_xml(
    su: &mut Setup,
    mut root: Node,
    x: &mae_xml::Setup,
    states: &BTreeMap<String, String>,
) {
    for xt in &x.tests {
        if !su.tests.iter().any(|t| t.name == xt.name) {
            // GUESS(G40): a test the import creates takes the view's state for it.
            let mut t = match states.get(&xt.name) {
                Some(el) => from_state(&xt.name, el),
                None => Test {
                    name: xt.name.clone(),
                    ..Test::default()
                },
            };
            let o = |n: &str| xt.option(n).unwrap_or("").to_owned();
            t.design = (o("lib"), o("cell"), o("view"));
            t.simulator = o("sim");
            su.tests.push(t);
        }
        let t = su
            .tests
            .iter_mut()
            .find(|t| t.name == xt.name)
            .expect("made");
        t.disabled = !xt.enabled;
        for (n, v) in xt.vars.iter().filter(|v| !v.1.is_empty()) {
            match t.vars.iter_mut().find(|x| &x.0 == n) {
                Some(x) => x.1 = v.clone(),
                None => t.vars.push((n.clone(), v.clone())),
            }
        }
    }
    for v in &x.vars {
        match su.vars.iter_mut().find(|y| y.0 == v.name) {
            Some(y) => y.1 = v.value.clone(),
            None => su.vars.push((v.name.clone(), v.value.clone())),
        }
    }
    for c in x.corners.iter().filter(|c| c.name != "_default") {
        if !su.corners.iter().any(|y| y.name == c.name) {
            su.corners.push(Corner {
                name: c.name.clone(),
                ..Corner::default()
            });
        }
        let y = su
            .corners
            .iter_mut()
            .find(|y| y.name == c.name)
            .expect("made");
        y.enabled = c.enabled;
        for (n, v) in &c.vars {
            match y.vars.iter_mut().find(|z| &z.0 == n) {
                Some(z) => z.1 = v.clone(),
                None => y.vars.push((n.clone(), v.clone())),
            }
        }
    }
    su.params = x
        .params
        .iter()
        .map(|p| (p, ""))
        .chain(
            x.corners
                .iter()
                .flat_map(|c| c.params.iter().map(move |p| (p, c.name.as_str()))),
        )
        .map(|(p, c)| {
            (
                p.location.clone(),
                p.name.clone(),
                p.value.clone(),
                c.to_owned(),
            )
        })
        .collect();
    for sp in &x.specs {
        if let Some(w) = &sp.weight {
            su.weights.insert(sp.name.clone(), w.clone());
        }
        let spec = match sp.kind.as_str() {
            "gt" => ("gt", V::str(&sp.min)),
            "lt" => ("lt", V::str(&sp.max)),
            "range" => ("range", V::list(vec![V::str(&sp.min), V::str(&sp.max)])),
            "tol" => (
                "tolerance",
                V::list(vec![V::str(&sp.target), V::str(&sp.tol)]),
            ),
            _ => continue,
        };
        if let Some(o) = su
            .tests
            .iter_mut()
            .find(|t| t.name == sp.test)
            .and_then(|t| t.outputs.iter_mut().find(|o| o.name == sp.output))
        {
            o.spec = vec![(spec.0.to_owned(), spec.1)];
        }
    }
    su.vars_off = x
        .vars
        .iter()
        .filter(|v| !v.enabled)
        .map(|v| v.name.clone())
        .collect();
    if let Some(old) = &su.sdb {
        for c in &old.children {
            if c.name != "active" && !root.children.iter().any(|r| r.name == c.name) {
                root.children.push(c.clone());
            }
        }
    }
    su.sdb = Some(root);
}

/// The setup database of `su` as Maestro's XML, root text `stem`: the tree the last import
/// read (else an empty `<active>`), its tests, variables, corners and specs updated from
/// what the mock models.
fn export_tree(su: &Setup, stem: &str) -> Node {
    let mut root = su.sdb.clone().unwrap_or_else(|| Node {
        name: "setupdb".into(),
        attrs: vec![("version".into(), "6".into())],
        children: vec![Node::leaf("active", "Active Setup")],
        ..Node::default()
    });
    root.text = stem.to_owned();
    let active = root
        .child_mut("active")
        .expect("a setup database has <active>");
    // No export holds the run mode (maestro.sdb's <currentmode> only).
    active.children.retain(|c| c.name != "currentmode");
    let section = |active: &mut Node, name: &str| -> usize {
        if let Some(k) = active.children.iter().position(|c| c.name == name) {
            return k;
        }
        active.children.push(Node::leaf(name, ""));
        active.children.len() - 1
    };
    let var = |n: &str, v: &str| Node {
        name: "var".into(),
        text: n.into(),
        children: vec![Node::leaf("value", v)],
        ..Node::default()
    };
    let set_value = |el: &mut Node, v: &str| match el.child_mut("value") {
        Some(x) => x.text = v.to_owned(),
        None => el.children.insert(0, Node::leaf("value", v)),
    };
    // Corners: Nominal (`_default`) first, then the mock's, each keeping what it holds
    // beyond enable and vars (parameters).
    let k = section(active, "corners");
    let old = std::mem::take(&mut active.children[k].children);
    let mut corners = vec![
        old.iter()
            .find(|c| c.text == "_default")
            .cloned()
            .unwrap_or_else(|| Node {
                attrs: vec![("enabled".into(), "1".into())],
                ..Node::leaf("corner", "_default")
            }),
    ];
    for c in &su.corners {
        let mut el = old
            .iter()
            .find(|o| o.text == c.name)
            .cloned()
            .unwrap_or_else(|| Node::leaf("corner", &c.name));
        el.attrs = vec![("enabled".into(), if c.enabled { "1" } else { "0" }.into())];
        sync_vars(&mut el, &c.vars, &var, &set_value);
        sync_params(&mut el, &su.params, &c.name);
        corners.push(el);
    }
    active.children[k].children = corners;
    // Tests.
    let k = section(active, "tests");
    let old = std::mem::take(&mut active.children[k].children);
    for t in &su.tests {
        let mut el = old
            .iter()
            .find(|o| o.text == t.name)
            .cloned()
            .unwrap_or_else(|| {
                let opts = |v: &[(&str, &str)]| -> Vec<Node> {
                    v.iter()
                        .map(|(n, x)| Node {
                            name: "option".into(),
                            text: (*n).into(),
                            children: vec![Node::leaf("value", x)],
                            ..Node::default()
                        })
                        .collect()
                };
                let d = [
                    ("cell", t.design.1.as_str()),
                    ("lib", t.design.0.as_str()),
                    ("sim", t.simulator.as_str()),
                    ("view", t.design.2.as_str()),
                ];
                let mut tool = d.to_vec();
                tool.extend([("path", "$AXL_SETUPDB_DIR"), ("state", "active")]);
                Node {
                    name: "test".into(),
                    text: t.name.clone(),
                    children: vec![
                        Node::leaf("tool", "ADE"),
                        Node {
                            children: opts(&tool),
                            ..Node::leaf("tooloptions", "")
                        },
                        Node {
                            children: opts(&d),
                            ..Node::leaf("origoptions", "")
                        },
                    ],
                    ..Node::default()
                }
            });
        if !t.vars.is_empty() || el.child("vars").is_some() {
            sync_vars(&mut el, &t.vars, &var, &set_value);
        }
        active.children[k].children.push(el);
    }
    // Global variables, each keeping its <dependentTests>.
    let k = section(active, "vars");
    let old = std::mem::take(&mut active.children[k].children);
    for (n, v) in &su.vars {
        let mut el = old
            .iter()
            .find(|o| &o.text == n)
            .cloned()
            .unwrap_or_else(|| var(n, ""));
        set_value(&mut el, v);
        el.attrs.retain(|a| a.0 != "enabled");
        if su.vars_off.contains(n) {
            el.attrs.push(("enabled".into(), "0".into()));
        }
        active.children[k].children.push(el);
    }
    sync_params(active, &su.params, "");
    let disabled: Vec<Node> = su
        .tests
        .iter()
        .filter(|t| t.disabled)
        .map(|t| Node::leaf("test", &t.name))
        .collect();
    match active.child_mut("disabledtests") {
        Some(d) => d.children = disabled,
        None if !disabled.is_empty() => active.children.push(Node {
            children: disabled,
            ..Node::leaf("disabledtests", "")
        }),
        None => {}
    }
    // Specs: those of the mock's outputs, in place where the tree has them.
    let k = section(active, "specs");
    for t in &su.tests {
        for o in t.outputs.iter().filter(|o| !o.spec.is_empty()) {
            let id = format!("{}.{}", t.name, o.name);
            let (op, v) = &o.spec[0];
            let items: Vec<String> = match v {
                V::List(l) => l
                    .iter()
                    .map(|x| x.as_str().unwrap_or("").to_owned())
                    .collect(),
                x => vec![x.as_str().unwrap_or("").to_owned()],
            };
            let at = |i: usize| items.get(i).cloned().unwrap_or_default();
            let (kind, min, max, target, tol) = match op.as_str() {
                "gt" | "minimum" => (
                    if op == "gt" { "gt" } else { "min" },
                    at(0),
                    String::new(),
                    String::new(),
                    String::new(),
                ),
                "lt" | "maximum" => (
                    if op == "lt" { "lt" } else { "max" },
                    String::new(),
                    at(0),
                    String::new(),
                    String::new(),
                ),
                "range" => ("range", at(0), at(1), String::new(), String::new()),
                _ => ("tol", String::new(), String::new(), at(0), at(1)),
            };
            let mut el = Node {
                name: "spec".into(),
                text: id.clone(),
                children: vec![
                    Node::leaf("testname", &t.name),
                    Node::leaf("resname", &o.name),
                    Node::leaf("specType", kind),
                    Node::leaf("min", &min),
                    Node::leaf("max", &max),
                    Node::leaf("target", &target),
                    Node::leaf("tol", &tol),
                ],
                ..Node::default()
            };
            let specs = &mut active.children[k].children;
            let old = specs.iter().position(|x| x.text == id);
            if let Some(p) = old {
                // What the mock does not model of the spec (tolType) stays.
                for c in &specs[p].children {
                    if c.name == "tolType" {
                        el.children.push(c.clone());
                    }
                }
            }
            if let Some(w) = su.weights.get(&id) {
                el.children.push(Node::leaf("specWeight", w));
            }
            match old {
                Some(p) => specs[p] = el,
                None => specs.push(el),
            }
        }
    }
    for sp in &mut active.children[k].children {
        if let Some(w) = su.weights.get(&sp.text) {
            match sp.child_mut("specWeight") {
                Some(x) => x.text = w.clone(),
                None => sp.children.push(Node::leaf("specWeight", w)),
            }
        } else {
            sp.children.retain(|c| c.name != "specWeight");
        }
    }
    active.children.retain(|c| {
        !(c.text.is_empty()
            && c.children.is_empty()
            && matches!(
                c.name.as_str(),
                "vars" | "specs" | "tests" | "disabledtests"
            ))
    });
    root
}

/// Sets the `<parameters>` of `el` (a corner, or `<active>` for `corner` `""`) to the
/// parameters of `params` in that scope, grouped by location as Maestro writes them, in
/// place where it has them (after `<vars>`, else last).
fn sync_params(el: &mut Node, params: &[(String, String, String, String)], corner: &str) {
    let mut locs: Vec<Node> = Vec::new();
    for (loc, prop, v, _) in params.iter().filter(|p| p.3 == corner) {
        let p = Node {
            name: "parameter".into(),
            text: prop.clone(),
            children: vec![Node::leaf("value", v)],
            ..Node::default()
        };
        match locs.iter_mut().find(|l| &l.text == loc) {
            Some(l) => l.children.push(p),
            None => locs.push(Node {
                children: vec![p],
                ..Node::leaf("location", loc)
            }),
        }
    }
    if locs.is_empty() {
        el.children.retain(|c| c.name != "parameters");
        return;
    }
    if el.child("parameters").is_none() {
        let at = el
            .children
            .iter()
            .position(|c| c.name == "vars")
            .map_or(el.children.len(), |p| p + 1);
        el.children.insert(at, Node::leaf("parameters", ""));
    }
    el.child_mut("parameters").expect("made").children = locs;
}

/// Sets the `<vars>` of `el` to `vars`, in place where it has them.
fn sync_vars(
    el: &mut Node,
    vars: &[(String, String)],
    var: &dyn Fn(&str, &str) -> Node,
    set_value: &dyn Fn(&mut Node, &str),
) {
    if vars.is_empty() {
        el.children.retain(|c| c.name != "vars");
        return;
    }
    if el.child("vars").is_none() {
        // After the test's <tooloptions> (Maestro's place), else last.
        let at = el
            .children
            .iter()
            .position(|c| c.name == "tooloptions")
            .map_or(el.children.len(), |p| p + 1);
        el.children.insert(at, Node::leaf("vars", ""));
    }
    let vs = el.child_mut("vars").expect("made");
    let old = std::mem::take(&mut vs.children);
    for (n, v) in vars {
        let mut x = old
            .iter()
            .find(|o| &o.text == n)
            .cloned()
            .unwrap_or_else(|| var(n, ""));
        set_value(&mut x, v);
        vs.children.push(x);
    }
}
