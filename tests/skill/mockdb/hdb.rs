//! Mock config views: the hierarchy editor (`hdb*`) functions `sfy_cfg.il` calls (spec
//! `interop.md` §5.4, §9.3; *Virtuoso Hierarchy Editor SKILL Reference*, `hdbOpen` …).
//!
//! Model: a handle is an `hdbConfig` object naming its view and mode; `hdbOpen "w"` creates
//! the DD view, `hdbSave` creates its `expand.cfg` and `master.tag` DD files (their text is
//! the interpreter's, which the mock cannot write: the loader writes `expand.cfg` itself,
//! and nothing reads `master.tag`), `hdbOpen "r"`/`"a"` needs a saved view, and a closed
//! handle signals. hdb's own rule printing is not modelled: the loader sends text.

use super::super::V;
use super::{MockDb, s};

/// The rows of this area in the assembled `mockdb::SIGNATURES`.
pub const ROWS: &[(&str, &str)] = &[
    ("hdbOpen", "t t t t"),
    ("hdbSetTopCellViewName", "d t t t"),
    ("hdbSave", "d"),
    ("hdbClose", "d"),
];

/// Handles a function of this area; `None` when `name` is not one.
pub fn call(db: &mut MockDb, name: &str, args: &[V]) -> Option<Result<V, String>> {
    let st = |i: usize| s(&args[i]).unwrap_or("").to_owned();
    // An open handle: (id, lib, cell, view, mode).
    let handle = |db: &MockDb| -> Result<(u32, String, String, String, String), String> {
        let h = MockDb::obj(&args[0])
            .filter(|&h| db.ty(h) == "hdbConfig")
            .ok_or_else(|| format!("*Error* {name}: argument #1 is not a config handle"))?;
        if db.a(h, "closed").truthy() {
            return Err(format!("*Error* {name}: the config handle is closed"));
        }
        let g = |k: &str| s(&db.a(h, k)).unwrap_or("").to_owned();
        Ok((h, g("lib"), g("cell"), g("view"), g("mode")))
    };
    let r = match name {
        "hdbOpen" => {
            let (lib, cell, view, mode) = (st(0), st(1), st(2), st(3));
            let found = match mode.as_str() {
                "w" => db
                    .dd_get(&lib, Some(&cell), Some(&view), None, "a")
                    .is_some(),
                "r" | "a" => db
                    .dd_find(&lib, Some(&cell), Some(&view), Some("expand.cfg"))
                    .is_some(),
                m => return Some(Err(format!("*Error* hdbOpen: mode {m} (r, w or a)"))),
            };
            Ok(if found {
                let attrs = [("lib", lib), ("cell", cell), ("view", view), ("mode", mode)];
                V::Obj(db.new_obj("hdbConfig", attrs.map(|(k, v)| (k, V::str(&v))).to_vec()))
            } else {
                V::Nil
            })
        }
        "hdbSetTopCellViewName" => handle(db).map(|(h, ..)| {
            let (lib, cell, view) = (st(1), st(2), st(3));
            if db.dd_find(&lib, Some(&cell), Some(&view), None).is_none() {
                return V::Nil;
            }
            db.put(h, "top", V::str(&format!("{lib}/{cell}/{view}")));
            V::T
        }),
        "hdbSave" => handle(db).map(|(h, lib, cell, view, mode)| {
            if mode == "r" || !db.a(h, "top").truthy() {
                return V::Nil;
            }
            for f in ["expand.cfg", "master.tag"] {
                db.dd_get(&lib, Some(&cell), Some(&view), Some(f), "a")
                    .expect("hdbOpen made the view");
            }
            V::T
        }),
        "hdbClose" => handle(db).map(|(h, ..)| {
            db.put(h, "closed", V::T);
            V::T
        }),
        _ => return None,
    };
    Some(r)
}
