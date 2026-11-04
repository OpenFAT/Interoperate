//! DD, database and schematic-editor functions (`dd*`, `db*`, `sch*`; skdfref "Design
//! Management Functions", "Database Access Functions"; skcompref "PI Functions"): the
//! arena, cellviews, figures and properties, the Rust-side API of [`MockDb`], and
//! `dispatch`, which runs every function of [`SIGNATURES`] that no other area delegates.

use super::*;

/// The DD, database and schematic-editor rows of [`SIGNATURES`].
pub(super) const ROWS: &[(&str, &str)] = &[
    // DD (skdfref "Design Management Functions")
    ("ddGetObj", "T [T T T D t]"),
    ("ddGetObjWritePath", "d"),
    // Database (skdfref "Database Access Functions")
    ("dbOpenCellViewByType", "g t g [T t D]"),
    ("dbSave", "d [g t t]"),
    ("dbClose", "d"),
    ("dbReopen", "d t"),
    ("dbPurge", "d"),
    ("dbGetOpenCellViews", ""),
    ("dbDeleteObject", "d"),
    ("dbFindTermByName", "d t"),
    ("dbTransformPoint", "p l"),
    ("dbTransformBBox", "b l"),
    ("dbGetTermNetExpr", "d"),
    ("dbMakeNet", "d t [D]"),
    ("dbCreateTerm", "d t t [x g]"),
    ("dbCreatePin", "d D [T D]"),
    ("dbCreateTermNetExpr", "d t"),
    ("dbCreateRect", "d L b"),
    ("dbCreateLine", "d L l"),
    ("dbCreatePath", "d L l n [t]"),
    ("dbCreatePolygon", "d L l"),
    ("dbCreateEllipse", "d L b"),
    ("dbCreateArc", "d L b b"),
    ("dbCreateLabel", "d L p t t t t n"),
    ("dbReplaceProp", "d t t g"),
    ("dbCreateProp", "d t t g"),
    ("dbProduceMemName", "t"),
    // Schematic editor (skcompref "PI Functions")
    ("schCreateInst", "d d T p t [n]"),
    ("dbCreateInst", "d d T p t [n]"),
    ("schCreateWire", "d t t l n n n [t t]"),
    ("schCreateWireLabel", "d d p t t t t n g"),
    ("schCreateNetExpression", "d t d p t t t n"),
    ("schCreatePin", "d d t t g p t [g g g]"),
    ("schCreateNoteLabel", "d p t t t t n t"),
    ("schGlueLabel", "d d"),
    ("schCheck", "d"),
    ("schSolder", "d p"),
];

impl MockDb {
    // ------------------------------------------------------------------ Rust API

    /// Adds library `name` to the cds.lib list; returns its DD object (the existing one when
    /// already defined).
    pub fn add_lib(&mut self, name: &str) -> u32 {
        if let Some(l) = self.lib(name) {
            return l;
        }
        let l = self.new_obj(
            "ddLib",
            vec![
                ("name", V::str(name)),
                ("readPath", V::str(&format!("/mock/{name}"))),
            ],
        );
        self.libs.push(l);
        l
    }

    /// The DD object of library `name`, if defined.
    pub fn lib(&self, name: &str) -> Option<u32> {
        self.libs
            .iter()
            .copied()
            .find(|&l| s(&self.a(l, "name")) == Some(name))
    }

    /// Removes cell `lib/cell` from the library (as when its library leaves cds.lib, or the
    /// cell is deleted on disk): `ddGetObj` no longer finds it, its cellviews are gone, and
    /// instances drawn of it stay behind unbound.
    pub fn forget_cell(&mut self, lib: &str, cell: &str) {
        let l = self.lib(lib).expect("library defined");
        let c = self
            .dd_find(lib, Some(cell), None, None)
            .expect("cell exists");
        let kept: Vec<V> = self
            .a(l, "cells")
            .items()
            .iter()
            .filter(|x| Self::obj(x) != Some(c))
            .cloned()
            .collect();
        self.put(l, "cells", V::list(kept));
        self.cvs.retain(|k, _| !(k.0 == lib && k.1 == cell));
    }

    /// The cellview `lib/cell/view` if it exists (saved, or open and not yet saved).
    pub fn cellview(&self, lib: &str, cell: &str, view: &str) -> Option<u32> {
        self.cvs
            .get(&(lib.into(), cell.into(), view.into()))
            .copied()
    }

    /// True when cellview `cv` has been saved at least once and has no unsaved edits.
    pub fn is_saved(&self, cv: u32) -> bool {
        self.a(cv, "sfySaved").truthy() && !self.a(cv, "modifiedButNotSaved").truthy()
    }

    /// `obj~>attr`, exactly as SKILL sees it (attributes, then properties; nil otherwise).
    pub fn attr(&self, obj: u32, attr: &str) -> V {
        self.get_attr(obj, attr)
    }

    /// Calls Cadence function `name` as SKILL would (checked against [`SIGNATURES`]).
    ///
    /// # Errors
    ///
    /// What the function signals: a bad argument, or an unknown function.
    pub fn call_fn(&mut self, name: &str, args: &[V]) -> Result<V, String> {
        self.call_db(name, args)
            .unwrap_or_else(|| Err(format!("undefined function {name}")))
    }

    /// Creates the DD file `lib/cell/view/file` (a text view's master) and returns the path
    /// its contents live at in the interpreter's files.
    pub fn add_text_file(&mut self, lib: &str, cell: &str, view: &str, file: &str) -> String {
        let f = self
            .dd_get(lib, Some(cell), Some(view), Some(file), "a")
            .expect("library defined");
        s(&self.a(f, "readPath")).unwrap_or_default().to_owned()
    }

    /// The base cell CDF of `lib/cell`, if any.
    pub fn base_cdf(&self, lib: &str, cell: &str) -> Option<u32> {
        let c = self.dd_find(lib, Some(cell), None, None)?;
        match self.a(c, "sfyBaseCdf") {
            V::Obj(o) => Some(o),
            _ => None,
        }
    }

    // ------------------------------------------------------------------ arena

    pub(super) fn new_obj(&mut self, ty: &str, attrs: Vec<(&str, V)>) -> u32 {
        let mut o = Obj::default();
        o.attrs.insert("objType".into(), V::str(ty));
        for (k, v) in attrs {
            o.attrs.insert(k.into(), v);
        }
        self.objs.push(o);
        (self.objs.len() - 1) as u32
    }

    pub(super) fn a(&self, id: u32, k: &str) -> V {
        self.objs
            .get(id as usize)
            .and_then(|o| o.attrs.get(k))
            .cloned()
            .unwrap_or(V::Nil)
    }

    pub(super) fn put(&mut self, id: u32, k: &str, v: V) {
        self.objs[id as usize].attrs.insert(k.into(), v);
    }

    pub(super) fn push(&mut self, id: u32, k: &str, v: V) {
        let mut l = self.a(id, k).items().to_vec();
        l.push(v);
        self.put(id, k, V::list(l));
    }

    fn unlink(&mut self, id: u32, k: &str, x: u32) {
        let l: Vec<V> = self
            .a(id, k)
            .items()
            .iter()
            .filter(|v| !matches!(v, V::Obj(o) if *o == x))
            .cloned()
            .collect();
        self.put(id, k, V::list(l));
    }

    pub(super) fn ty(&self, id: u32) -> String {
        s(&self.a(id, "objType")).unwrap_or("").to_owned()
    }

    pub(super) fn obj(v: &V) -> Option<u32> {
        match v {
            V::Obj(o) => Some(*o),
            _ => None,
        }
    }

    // ------------------------------------------------------------------ DD

    fn dd_child(&self, parent: u32, rel: &str, name: &str) -> Option<u32> {
        self.a(parent, rel)
            .items()
            .iter()
            .filter_map(Self::obj)
            .find(|&c| s(&self.a(c, "name")) == Some(name))
    }

    pub(super) fn dd_find(
        &self,
        lib: &str,
        cell: Option<&str>,
        view: Option<&str>,
        file: Option<&str>,
    ) -> Option<u32> {
        let mut o = self.lib(lib)?;
        for (rel, n) in [("cells", cell), ("views", view), ("files", file)] {
            match n {
                Some(n) => o = self.dd_child(o, rel, n)?,
                None => return Some(o),
            }
        }
        Some(o)
    }

    /// `ddGetObj` with mode `r`, `a` or `w` (creating cells, views and files on `a`/`w`).
    pub(super) fn dd_get(
        &mut self,
        lib: &str,
        cell: Option<&str>,
        view: Option<&str>,
        file: Option<&str>,
        mode: &str,
    ) -> Option<u32> {
        let l = self.lib(lib)?;
        let create = mode == "a" || mode == "w";
        let mut o = l;
        let mut path = format!("/mock/{lib}");
        for (rel, ty, n) in [
            ("cells", "ddCell", cell),
            ("views", "ddView", view),
            ("files", "ddFile", file),
        ] {
            let Some(n) = n else { return Some(o) };
            path = format!("{path}/{n}");
            match self.dd_child(o, rel, n) {
                Some(c) => {
                    if ty == "ddFile" && mode == "w" {
                        return None; // "fails if a master object already exists"
                    }
                    o = c;
                }
                None if create => {
                    let up = match ty {
                        "ddCell" => "lib",
                        "ddView" => "cell",
                        _ => "view",
                    };
                    let c = self.new_obj(ty, vec![("name", V::str(n)), (up, V::Obj(o))]);
                    if ty == "ddFile" {
                        self.put(c, "readPath", V::str(&path));
                        self.put(c, "writePath", V::str(&path));
                    }
                    self.push(o, rel, V::Obj(c));
                    o = c;
                }
                None => return None,
            }
        }
        Some(o)
    }

    // ------------------------------------------------------------------ cellviews

    pub(super) fn editable(&self, cv: u32) -> bool {
        matches!(s(&self.a(cv, "mode")), Some("w" | "a"))
    }

    fn touch(&mut self, cv: u32) {
        self.put(cv, "modifiedButNotSaved", V::T);
    }

    fn cv_of(&self, o: u32) -> Option<u32> {
        if self.ty(o) == "cellView" {
            return Some(o);
        }
        Self::obj(&self.a(o, "cellView"))
    }

    fn open_cv(
        &mut self,
        lib: &str,
        cell: &str,
        views: &[String],
        vtype: Option<&str>,
        mode: &str,
    ) -> Result<V, String> {
        if !matches!(mode, "r" | "a" | "w" | "s") {
            return Err(format!(
                "*Error* dbOpenCellViewByType: mode {mode} is not r, a, w or s"
            ));
        }
        if self.lib(lib).is_none() {
            return Ok(V::Nil);
        }
        for view in views {
            let key = (lib.to_owned(), cell.to_owned(), view.clone());
            if let Some(&cv) = self.cvs.get(&key) {
                let n = num(&self.a(cv, "sfyOpen")).unwrap_or(0.0) as i64;
                self.put(cv, "sfyOpen", V::Int(n + 1));
                match mode {
                    "w" => {
                        let keep = self.objs[cv as usize].attrs.clone();
                        self.backups.entry(cv).or_insert(keep);
                        for k in ["instances", "shapes", "terminals", "nets", "prop"] {
                            self.put(cv, k, V::Nil);
                        }
                        self.put(cv, "mode", V::str("w"));
                        self.touch(cv);
                    }
                    "a" | "s" if !self.editable(cv) => {
                        let keep = self.objs[cv as usize].attrs.clone();
                        self.backups.entry(cv).or_insert(keep);
                        self.put(cv, "mode", V::str(mode));
                    }
                    _ => {}
                }
                return Ok(V::Obj(cv));
            }
        }
        // Not found: only a/w/s with a view type create, in the first view named.
        let (Some(vt), Some(view)) = (vtype.filter(|v| !v.is_empty()), views.first()) else {
            return Ok(V::Nil);
        };
        if mode == "r" {
            return Ok(V::Nil);
        }
        let ddc = self
            .dd_get(lib, Some(cell), None, None, "a")
            .expect("lib exists");
        let ddv = self
            .dd_get(lib, Some(cell), Some(view), None, "a")
            .expect("lib exists");
        let ddl = self.lib(lib).expect("lib exists");
        let cv = self.new_obj(
            "cellView",
            vec![
                ("libName", V::str(lib)),
                ("cellName", V::str(cell)),
                ("viewName", V::str(view)),
                ("cellViewType", V::str(vt)),
                // GUESS(G2): schematic and schematicSymbol cellviews: 160 DBU per inch.
                ("DBUPerUU", V::Float(160.0)),
                ("userUnits", V::str("inch")),
                ("mode", V::str(mode)),
                ("lib", V::Obj(ddl)),
                ("cell", V::Obj(ddc)),
                ("sfyView", V::Obj(ddv)),
                ("sfyOpen", V::Int(1)),
                ("modifiedButNotSaved", V::T),
            ],
        );
        self.cvs.insert((lib.into(), cell.into(), view.clone()), cv);
        Ok(V::Obj(cv))
    }

    /// Drops unsaved edits: restores the content saved before the cellview was opened for
    /// edit, or removes a never-saved cellview (and its DD view when it holds no files).
    fn revert(&mut self, cv: u32) {
        if !self.a(cv, "modifiedButNotSaved").truthy() {
            self.backups.remove(&cv);
            return;
        }
        if let Some(b) = self.backups.remove(&cv) {
            self.objs[cv as usize].attrs = b;
            self.put(cv, "mode", V::str("r"));
            return;
        }
        if !self.a(cv, "sfySaved").truthy() {
            let key = (
                s(&self.a(cv, "libName")).unwrap_or("").to_owned(),
                s(&self.a(cv, "cellName")).unwrap_or("").to_owned(),
                s(&self.a(cv, "viewName")).unwrap_or("").to_owned(),
            );
            self.cvs.remove(&key);
            if let (Some(ddc), Some(ddv)) = (
                Self::obj(&self.a(cv, "cell")),
                Self::obj(&self.a(cv, "sfyView")),
            ) && !self.a(ddv, "files").truthy()
            {
                self.unlink(ddc, "views", ddv);
            }
        }
    }

    // ------------------------------------------------------------------ figures

    pub(super) fn lpp(v: &V) -> (String, String) {
        match v {
            V::Str(l) => (l.to_string(), "drawing".into()),
            _ => {
                let i = v.items();
                (
                    s(&i[0]).unwrap_or("").to_owned(),
                    s(&i[1]).unwrap_or("").to_owned(),
                )
            }
        }
    }

    pub(super) fn new_shape(
        &mut self,
        cv: u32,
        ty: &str,
        lpp: &(String, String),
        extra: Vec<(&str, V)>,
    ) -> u32 {
        let mut attrs = vec![
            ("cellView", V::Obj(cv)),
            ("layerName", V::str(&lpp.0)),
            ("purpose", V::str(&lpp.1)),
            ("lpp", V::list(vec![V::str(&lpp.0), V::str(&lpp.1)])),
        ];
        attrs.extend(extra);
        let id = self.new_obj(ty, attrs);
        self.push(cv, "shapes", V::Obj(id));
        self.touch(cv);
        id
    }

    fn points(v: &V, min: usize) -> Option<V> {
        let pts: Option<Vec<(f64, f64)>> = v.items().iter().map(pt).collect();
        let pts = pts?;
        if pts.len() < min {
            return None;
        }
        Some(V::list(pts.into_iter().map(vpt).collect()))
    }

    fn new_label(
        &mut self,
        cv: u32,
        lpp: &(String, String),
        a: &[V],
        ltype: &str,
    ) -> Result<u32, String> {
        // a = (point text justify orient font height)
        let just = s(&a[2]).unwrap_or("");
        let orient = s(&a[3]).unwrap_or("");
        let font = s(&a[4]).unwrap_or("");
        if !JUSTIFY.contains(&just) {
            return Err(format!("*Error* label: justification {just}"));
        }
        if !ORIENTS.contains(&orient) {
            return Err(format!("*Error* label: orientation {orient}"));
        }
        if !FONTS.contains(&font) {
            return Err(format!("*Error* label: font {font}"));
        }
        let h = num(&a[5]).unwrap_or(0.0);
        if h <= 0.0 {
            return Err("*Error* label: the height must be a positive number".into());
        }
        Ok(self.new_shape(
            cv,
            "label",
            lpp,
            vec![
                ("xy", vpt(pt(&a[0]).expect("checked"))),
                ("theLabel", a[1].clone()),
                ("justify", V::str(just)),
                ("orient", V::str(orient)),
                ("font", V::str(font)),
                ("height", V::Float(h)),
                ("labelType", V::str(ltype)),
                // Drafted by default (live ICADVM20.1: every label dumps isDrafted t).
                ("isDrafted", V::T),
                ("isVisible", V::T),
            ],
        ))
    }

    fn glue(&mut self, label: u32, fig: Option<u32>) {
        if let Some(old) = Self::obj(&self.a(label, "parent")) {
            self.unlink(old, "children", label);
        }
        match fig {
            Some(f) => {
                self.put(label, "parent", V::Obj(f));
                self.push(f, "children", V::Obj(label));
            }
            None => self.put(label, "parent", V::Nil),
        }
    }

    fn is_wire(&self, o: u32) -> bool {
        matches!(self.ty(o).as_str(), "line" | "path") && s(&self.a(o, "layerName")) == Some("wire")
    }

    fn is_pin_fig(&self, o: u32) -> bool {
        (self.ty(o) == "inst" && s(&self.a(o, "purpose")) == Some("pin")) || self.ty(o) == "pin"
    }

    fn new_inst(
        &mut self,
        cv: u32,
        master: u32,
        name: &str,
        xy: (f64, f64),
        orient: &str,
        purpose: &str,
    ) -> u32 {
        let (base, n) = match name.split_once('<') {
            Some((b, r)) => {
                let r = r.trim_end_matches('>');
                let n = match r.split_once(':') {
                    Some((x, y)) => {
                        x.parse::<i64>()
                            .unwrap_or(0)
                            .abs_diff(y.parse::<i64>().unwrap_or(0))
                            + 1
                    }
                    None => 1,
                };
                (b.to_owned(), n as i64)
            }
            None => (name.to_owned(), 1),
        };
        let inst = self.new_obj(
            "inst",
            vec![
                ("cellView", V::Obj(cv)),
                ("name", V::str(name)),
                ("baseName", V::str(&base)),
                ("numInst", V::Int(n)),
                ("libName", self.a(master, "libName")),
                ("cellName", self.a(master, "cellName")),
                ("viewName", self.a(master, "viewName")),
                ("master", V::Obj(master)),
                ("xy", vpt(xy)),
                ("orient", V::str(orient)),
                ("mag", V::Float(1.0)),
                ("purpose", V::str(purpose)),
            ],
        );
        for t in self.a(master, "terminals").items().to_vec() {
            let Some(t) = Self::obj(&t) else { continue };
            let it = self.new_obj(
                "instTerm",
                vec![
                    ("name", self.a(t, "name")),
                    ("term", V::Obj(t)),
                    ("inst", V::Obj(inst)),
                    ("direction", self.a(t, "direction")),
                    ("cellView", V::Obj(cv)),
                ],
            );
            self.push(inst, "instTerms", V::Obj(it));
        }
        self.push(cv, "instances", V::Obj(inst));
        // Leaked cell CDF values (leak_to_cell) become the new instance's own properties.
        let leaked: Vec<(String, V)> = s(&self.a(master, "libName"))
            .zip(s(&self.a(master, "cellName")))
            .and_then(|(l, c)| self.dd_find(l, Some(c), None, None))
            .and_then(|c| Self::obj(&self.a(c, "sfyCellCdf")))
            .map(|cdf| {
                self.a(cdf, "parameters")
                    .items()
                    .iter()
                    .filter_map(Self::obj)
                    .filter(|&p| !matches!(self.a(p, "sfyLive"), V::Nil))
                    .map(|p| {
                        (
                            s(&self.a(p, "name")).unwrap_or("").to_owned(),
                            self.a(p, "sfyLive"),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        for (n, v) in leaked {
            let ty = if matches!(v, V::Int(_)) {
                "int"
            } else {
                "string"
            };
            self.call_fn("dbReplaceProp", &[V::Obj(inst), V::str(&n), V::str(ty), v])
                .ok();
        }
        self.touch(cv);
        inst
    }

    fn inst_name_ok(name: &str) -> bool {
        // base[<N[:M]>] only (skdfref "Instance Name Syntax").
        let (base, rest) = match name.split_once('<') {
            Some((b, r)) => (b, Some(r)),
            None => (name, None),
        };
        let base_ok = !base.is_empty()
            && base
                .chars()
                .all(|c| c.is_ascii_graphic() && !"<>(),/".contains(c));
        let rest_ok = match rest {
            None => true,
            Some(r) => r.strip_suffix('>').is_some_and(|r| {
                let parts: Vec<&str> = r.split(':').collect();
                parts.len() <= 2
                    && parts
                        .iter()
                        .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
            }),
        };
        base_ok && rest_ok
    }

    fn bbox(&self, o: u32) -> Option<((f64, f64), (f64, f64))> {
        let grow = |acc: Option<((f64, f64), (f64, f64))>, p: (f64, f64)| {
            Some(match acc {
                None => (p, p),
                Some((a, b)) => ((a.0.min(p.0), a.1.min(p.1)), (b.0.max(p.0), b.1.max(p.1))),
            })
        };
        match self.ty(o).as_str() {
            "line" | "path" | "polygon" => self
                .a(o, "points")
                .items()
                .iter()
                .filter_map(pt)
                .fold(None, grow),
            "rect" | "ellipse" => bb(&self.a(o, "bBox")),
            "arc" => bb(&self.a(o, "ellipseBBox")),
            "label" | "dot" => pt(&self.a(o, "xy")).map(|p| (p, p)),
            "cellView" => {
                let mut acc = None;
                for f in self.a(o, "shapes").items().iter().filter_map(Self::obj) {
                    if let Some((a, b)) = self.bbox(f) {
                        acc = grow(grow(acc, a), b);
                    }
                }
                acc
            }
            "inst" => {
                let m = Self::obj(&self.a(o, "master"))?;
                let (a, b) = self.bbox(m)?;
                let t = self.inst_transform(o);
                let pa = transform(a, &t).ok()?;
                let pb = transform(b, &t).ok()?;
                let r = vbox(pa, pb);
                bb(&r)
            }
            _ => None,
        }
    }

    fn inst_transform(&self, o: u32) -> V {
        V::list(vec![self.a(o, "xy"), self.a(o, "orient"), self.a(o, "mag")])
    }

    // ------------------------------------------------------------------ props

    fn prop_value_ok(ty: &str, v: &V) -> bool {
        match ty {
            "string" | "time" | "fileName" | "ILExpr" | "NLPExpr" | "netSet" => {
                matches!(v, V::Str(_))
            }
            "int" => matches!(v, V::Int(_)),
            "float" => matches!(v, V::Float(_)),
            "boolean" => matches!(v, V::T | V::Nil),
            "ILList" => matches!(v, V::List(_) | V::Nil),
            _ => true,
        }
    }

    fn find_prop(&self, o: u32, name: &str) -> Option<u32> {
        self.a(o, "prop")
            .items()
            .iter()
            .filter_map(Self::obj)
            .find(|&p| s(&self.a(p, "name")) == Some(name))
    }

    pub(super) fn set_prop(&mut self, o: u32, name: &str, ty: &str, v: V) -> u32 {
        let p = match self.find_prop(o, name) {
            Some(p) => p,
            None => {
                let p = self.new_obj("prop", vec![("name", V::str(name)), ("object", V::Obj(o))]);
                self.push(o, "prop", V::Obj(p));
                p
            }
        };
        self.put(p, "valueType", V::str(ty));
        self.put(p, "value", v);
        if let Some(cv) = self.cv_of(o) {
            self.touch(cv);
        }
        p
    }

    pub(super) fn remove_prop(&mut self, o: u32, name: &str) {
        if let Some(p) = self.find_prop(o, name) {
            self.unlink(o, "prop", p);
        }
    }

    /// A property's value as `~>` reads it: booleans as "TRUE"/"FALSE" (skdfref dbGet).
    fn prop_read(&self, p: u32) -> V {
        let v = self.a(p, "value");
        if s(&self.a(p, "valueType")) == Some("boolean") {
            return V::str(if v.truthy() { "TRUE" } else { "FALSE" });
        }
        v
    }

    // ------------------------------------------------------------------ dispatch

    pub(super) fn call_db(&mut self, name: &str, args: &[V]) -> Option<Result<V, String>> {
        let sig = SIGNATURES.iter().find(|(n, _)| *n == name)?.1;
        if DELEGATED.iter().any(|t| t.iter().any(|(n, _)| *n == name)) {
            // Signature checked here; the body lives in the area's module (`Host::call`).
            return check(name, sig, args).err().map(Err);
        }
        Some(check(name, sig, args).and_then(|(a, kw)| self.dispatch(name, &a, &kw)))
    }

    fn dispatch(&mut self, name: &str, a: &[V], kw: &[(String, V)]) -> Result<V, String> {
        let o = |i: usize| -> u32 { Self::obj(&a[i]).unwrap_or(0) };
        let st = |i: usize| -> String { a.get(i).and_then(s).unwrap_or("").to_owned() };
        Ok(match name {
            "ddGetObj" => {
                let lib = st(0);
                let opt = |i: usize| {
                    a.get(i)
                        .and_then(s)
                        .filter(|x| !x.is_empty())
                        .map(str::to_owned)
                };
                let mode = a.get(5).and_then(s).unwrap_or("r").to_owned();
                if a.get(4).is_some_and(V::truthy) {
                    return Err("*Error* ddGetObj: context objects are not modelled".into());
                }
                let (c, v, f) = (opt(1), opt(2), opt(3));
                match self.dd_get(&lib, c.as_deref(), v.as_deref(), f.as_deref(), &mode) {
                    Some(x) => V::Obj(x),
                    None => V::Nil,
                }
            }
            "ddGetObjWritePath" => self.a(o(0), "writePath"),
            "dbOpenCellViewByType" => {
                let lib = match &a[0] {
                    V::Str(l) => l.to_string(),
                    V::Obj(l) => s(&self.a(*l, "name")).unwrap_or("").to_owned(),
                    _ => return Err(bad(name, 1, "a library name or ddId")),
                };
                let views: Vec<String> = match &a[2] {
                    V::Str(v) => v.split_whitespace().map(str::to_owned).collect(),
                    V::List(l) => l.iter().filter_map(s).map(str::to_owned).collect(),
                    _ => return Err(bad(name, 3, "a view name or list")),
                };
                let vt = a.get(3).and_then(s).map(str::to_owned);
                let mode = a.get(4).and_then(s).unwrap_or("r").to_owned();
                self.open_cv(&lib, &st(1), &views, vt.as_deref(), &mode)?
            }
            "dbSave" => {
                if a.len() > 1 {
                    return Err("*Error* dbSave: saving to another cellview is not modelled".into());
                }
                let cv = o(0);
                if self.ty(cv) != "cellView" || !self.editable(cv) {
                    return Ok(V::Nil);
                }
                self.backups.remove(&cv);
                self.put(cv, "sfySaved", V::T);
                self.put(cv, "modifiedButNotSaved", V::Nil);
                V::T
            }
            // The open cellview in another mode (the same object, its references kept).
            "dbReopen" => {
                let cv = o(0);
                let mode = st(1);
                if self.ty(cv) != "cellView" || !["r", "a", "w"].contains(&mode.as_str()) {
                    return Ok(V::Nil);
                }
                self.put(cv, "mode", V::str(&mode));
                V::T
            }
            "dbClose" | "dbPurge" => {
                let cv = o(0);
                if self.ty(cv) != "cellView" {
                    return Ok(V::Nil);
                }
                let n = num(&self.a(cv, "sfyOpen")).unwrap_or(0.0) as i64 - 1;
                let n = if name == "dbPurge" { 0 } else { n.max(0) };
                self.put(cv, "sfyOpen", V::Int(n));
                if n == 0 {
                    self.revert(cv);
                    self.put(cv, "mode", V::str("r"));
                }
                V::T
            }
            "dbFindTermByName" => self
                .a(o(0), "terminals")
                .items()
                .iter()
                .find(|t| Self::obj(t).is_some_and(|t| s(&self.a(t, "name")) == Some(&st(1))))
                .cloned()
                .unwrap_or(V::Nil),
            "dbTransformPoint" => vpt(transform(pt(&a[0]).expect("checked"), &a[1])?),
            "dbTransformBBox" => {
                let (p, q) = bb(&a[0]).expect("checked");
                vbox(transform(p, &a[1])?, transform(q, &a[1])?)
            }
            "dbGetTermNetExpr" => self.a(o(0), "sfyNetExpr"),
            "dbMakeNet" => {
                let cv = o(0);
                if !self.editable(cv) {
                    return Ok(V::Nil);
                }
                let existing = self
                    .a(cv, "nets")
                    .items()
                    .iter()
                    .find(|n| Self::obj(n).is_some_and(|n| s(&self.a(n, "name")) == Some(&st(1))))
                    .cloned();
                match existing {
                    Some(n) => n,
                    None => {
                        let n = self.new_obj(
                            "net",
                            vec![("name", V::str(&st(1))), ("cellView", V::Obj(cv))],
                        );
                        self.push(cv, "nets", V::Obj(n));
                        self.touch(cv);
                        V::Obj(n)
                    }
                }
            }
            "dbCreateTerm" => {
                let net = o(0);
                let dir = st(2);
                let dirs = [
                    "input",
                    "output",
                    "inputOutput",
                    "jumper",
                    "switch",
                    "unused",
                    "powerInput",
                    "powerOutput",
                    "bidir",
                    "tristate",
                ];
                if !dirs.contains(&dir.as_str()) {
                    return Err(bad(name, 3, "a terminal direction"));
                }
                let Some(cv) = self.cv_of(net) else {
                    return Ok(V::Nil);
                };
                if self.a(net, "term").truthy()
                    || Self::obj(&self.call_fn("dbFindTermByName", &[V::Obj(cv), a[1].clone()])?)
                        .is_some()
                {
                    return Ok(V::Nil);
                }
                let t = self.new_obj(
                    "term",
                    vec![
                        ("name", a[1].clone()),
                        ("direction", V::str(&dir)),
                        ("net", V::Obj(net)),
                        ("cellView", V::Obj(cv)),
                    ],
                );
                self.put(net, "term", V::Obj(t));
                self.push(cv, "terminals", V::Obj(t));
                self.touch(cv);
                V::Obj(t)
            }
            "dbCreatePin" => {
                let net = o(0);
                let Some(cv) = self.cv_of(net) else {
                    return Ok(V::Nil);
                };
                let term = match a.get(3).and_then(Self::obj) {
                    Some(t) => t,
                    None => match Self::obj(&self.a(net, "term")) {
                        Some(t) => t,
                        None => {
                            let n = self.a(net, "name");
                            Self::obj(&self.call_fn(
                                "dbCreateTerm",
                                &[V::Obj(net), n, V::str("inputOutput")],
                            )?)
                            .unwrap_or(0)
                        }
                    },
                };
                let k = self.a(term, "pins").items().len();
                let pname = match a.get(2).and_then(s).filter(|x| !x.is_empty()) {
                    Some(n) => n.to_owned(),
                    None => format!("{}__{k}", s(&self.a(term, "name")).unwrap_or("P")),
                };
                let pin = self.new_obj(
                    "pin",
                    vec![
                        ("name", V::str(&pname)),
                        ("term", V::Obj(term)),
                        ("net", V::Obj(net)),
                        ("cellView", V::Obj(cv)),
                    ],
                );
                if let Some(f) = a.get(1).and_then(Self::obj) {
                    // An instance that becomes a pin figure takes purpose "pin" (live
                    // ICADVM20.1).
                    if self.ty(f) == "inst" {
                        self.put(f, "purpose", V::str("pin"));
                    }
                    self.put(pin, "fig", V::Obj(f));
                    self.put(pin, "figs", V::list(vec![V::Obj(f)]));
                    self.put(f, "pin", V::Obj(pin));
                    self.put(f, "net", V::Obj(net));
                }
                self.push(term, "pins", V::Obj(pin));
                self.touch(cv);
                V::Obj(pin)
            }
            "dbCreateTermNetExpr" => {
                let t = o(0);
                if self.ty(t) != "term" || self.a(t, "sfyNetExpr").truthy() {
                    return Ok(V::Nil);
                }
                self.put(t, "sfyNetExpr", a[1].clone());
                V::T
            }
            "dbCreateRect" | "dbCreateEllipse" => {
                let cv = o(0);
                if !self.editable(cv) {
                    return Ok(V::Nil);
                }
                let (mut p, mut q) = bb(&a[2]).expect("checked");
                let ty = if name == "dbCreateRect" {
                    "rect"
                } else {
                    // The ellipse box is stored in whole DBUs, rounded outward (live
                    // ICADVM20.1: a radius-2.5 bubble at y 0 came back with box y -3..3).
                    let dbu = num(&self.a(cv, "DBUPerUU")).unwrap_or(160.0);
                    let (lo, hi) = (p.0.min(q.0), p.0.max(q.0));
                    (p.0, q.0) = ((lo * dbu).floor() / dbu, (hi * dbu).ceil() / dbu);
                    let (lo, hi) = (p.1.min(q.1), p.1.max(q.1));
                    (p.1, q.1) = ((lo * dbu).floor() / dbu, (hi * dbu).ceil() / dbu);
                    "ellipse"
                };
                V::Obj(self.new_shape(cv, ty, &Self::lpp(&a[1]), vec![("bBox", vbox(p, q))]))
            }
            "dbCreateLine" | "dbCreatePolygon" | "dbCreatePath" => {
                let cv = o(0);
                if !self.editable(cv) {
                    return Ok(V::Nil);
                }
                let min = if name == "dbCreatePolygon" { 3 } else { 2 };
                let Some(pts) = Self::points(&a[2], min) else {
                    return Err(bad(name, 3, &format!("a list of at least {min} points")));
                };
                let n = pts.items().len() as i64;
                let mut extra = vec![("points", pts), ("nPoints", V::Int(n))];
                let ty = match name {
                    "dbCreateLine" => "line",
                    "dbCreatePolygon" => "polygon",
                    _ => {
                        let dbu = num(&self.a(cv, "DBUPerUU")).unwrap_or(160.0);
                        let w = Self::width_on_grid(num(&a[3]).expect("checked"), dbu);
                        extra.push(("width", V::Float(w)));
                        extra.push((
                            "pathStyle",
                            a.get(4).cloned().unwrap_or(V::str("truncateExtend")),
                        ));
                        "path"
                    }
                };
                V::Obj(self.new_shape(cv, ty, &Self::lpp(&a[1]), extra))
            }
            "dbCreateArc" => {
                let cv = o(0);
                if !self.editable(cv) {
                    return Ok(V::Nil);
                }
                let (p, q) = bb(&a[2]).expect("checked");
                // The arc is the part of the ellipse inside the arc box (live ICADVM20.1: an
                // arc box that is the ellipse box is refused, DB-270000).
                let Some((s0, s1)) = bb(&a[3]).and_then(|arc| Self::arc_angles((p, q), arc)) else {
                    return Ok(V::Nil);
                };
                V::Obj(self.new_shape(
                    cv,
                    "arc",
                    &Self::lpp(&a[1]),
                    vec![
                        ("ellipseBBox", vbox(p, q)),
                        ("startAngle", V::Float(s0)),
                        ("stopAngle", V::Float(s1)),
                    ],
                ))
            }
            "dbCreateLabel" => {
                let cv = o(0);
                if !self.editable(cv) {
                    return Ok(V::Nil);
                }
                V::Obj(self.new_label(cv, &Self::lpp(&a[1]), &a[2..8], "normalLabel")?)
            }
            "dbReplaceProp" | "dbCreateProp" => {
                let obj = o(0);
                let ty = st(2);
                if !PROP_TYPES.contains(&ty.as_str()) {
                    return Err(bad(name, 3, "a property type"));
                }
                if !Self::prop_value_ok(&ty, &a[3]) {
                    return Err(bad(name, 4, &format!("a value of type {ty}")));
                }
                if self.cv_of(obj).is_some_and(|cv| !self.editable(cv)) {
                    return Ok(V::Nil);
                }
                if name == "dbCreateProp" && self.find_prop(obj, &st(1)).is_some() {
                    return Ok(V::Nil);
                }
                V::Obj(self.set_prop(obj, &st(1), &ty, a[3].clone()))
            }
            "dbProduceMemName" => {
                let mut out = Vec::new();
                for part in st(0).split(',') {
                    match part.split_once('<') {
                        Some((b, r)) => {
                            let r = r.trim_end_matches('>');
                            let (x, y) = r.split_once(':').unwrap_or((r, r));
                            let (x, y): (i64, i64) =
                                (x.parse().unwrap_or(0), y.parse().unwrap_or(0));
                            let step = if y >= x { 1 } else { -1 };
                            let mut i = x;
                            loop {
                                out.push(V::str(&format!("{b}<{i}>")));
                                if i == y {
                                    break;
                                }
                                i += step;
                            }
                        }
                        None => out.push(V::str(part)),
                    }
                }
                V::list(out)
            }
            "schCreateInst" => {
                let (cv, master) = (o(0), o(1));
                if !self.editable(cv)
                    || s(&self.a(cv, "cellViewType")) != Some("schematic")
                    || self.ty(master) != "cellView"
                    || cv == master
                {
                    return Ok(V::Nil);
                }
                let orient = st(4);
                if !ORIENTS.contains(&orient.as_str()) {
                    return Err(bad(name, 5, "an orientation"));
                }
                let iname = match &a[2] {
                    V::Str(n) => n.to_string(),
                    _ => self.auto_name(cv, "I"),
                };
                if !Self::inst_name_ok(&iname) || self.inst_named(cv, &iname) {
                    return Ok(V::Nil);
                }
                V::Obj(self.new_inst(
                    cv,
                    master,
                    &iname,
                    pt(&a[3]).expect("checked"),
                    &orient,
                    "cell",
                ))
            }
            // An instance in any cellview (live ICADVM20.1: a symbol's pin figure, an
            // instance of basic/sympin, named I0, I1, … when the name is nil).
            "dbCreateInst" => {
                let (cv, master) = (o(0), o(1));
                if !self.editable(cv) || self.ty(master) != "cellView" || cv == master {
                    return Ok(V::Nil);
                }
                let orient = st(4);
                if !ORIENTS.contains(&orient.as_str()) {
                    return Err(bad(name, 5, "an orientation"));
                }
                let iname = match &a[2] {
                    V::Str(n) => n.to_string(),
                    _ => self.auto_name(cv, "I"),
                };
                if !Self::inst_name_ok(&iname) || self.inst_named(cv, &iname) {
                    return Ok(V::Nil);
                }
                V::Obj(self.new_inst(
                    cv,
                    master,
                    &iname,
                    pt(&a[3]).expect("checked"),
                    &orient,
                    "cell",
                ))
            }
            "schCreatePin" => {
                let (cv, master) = (o(0), o(1));
                let dir = st(3);
                if !["input", "output", "inputOutput", "switch"].contains(&dir.as_str()) {
                    return Err(bad(name, 4, "input, output, inputOutput or switch"));
                }
                let orient = st(6);
                if !ORIENTS.contains(&orient.as_str()) {
                    return Err(bad(name, 7, "an orientation"));
                }
                if !self.editable(cv)
                    || s(&self.a(cv, "cellViewType")) != Some("schematic")
                    || self.ty(master) != "cellView"
                {
                    return Ok(V::Nil);
                }
                let tname = st(2);
                if Self::obj(&self.call_fn("dbFindTermByName", &[V::Obj(cv), V::str(&tname)])?)
                    .is_some()
                {
                    return Ok(V::Nil);
                }
                // GUESS(G11): the pin instance is named PIN<n> and is what schCreatePin returns.
                self.pins_made += 1;
                let iname = format!("PIN{}", self.pins_made);
                let inst = self.new_inst(
                    cv,
                    master,
                    &iname,
                    pt(&a[5]).expect("checked"),
                    &orient,
                    "pin",
                );
                let net = Self::obj(&self.call_fn("dbMakeNet", &[V::Obj(cv), V::str(&tname)])?)
                    .unwrap_or(0);
                let term = Self::obj(
                    &self.call_fn("dbCreateTerm", &[V::Obj(net), V::str(&tname), V::str(&dir)])?,
                )
                .unwrap_or(0);
                let pin = self.new_obj(
                    "pin",
                    vec![
                        ("name", V::str(&tname)),
                        ("term", V::Obj(term)),
                        ("net", V::Obj(net)),
                        ("fig", V::Obj(inst)),
                        ("figs", V::list(vec![V::Obj(inst)])),
                        ("cellView", V::Obj(cv)),
                    ],
                );
                self.push(term, "pins", V::Obj(pin));
                self.put(inst, "pin", V::Obj(pin));
                self.put(inst, "net", V::Obj(net));
                V::Obj(inst)
            }
            "schCreateWire" => {
                let cv = o(0);
                let entry = st(1);
                if entry != "draw" && entry != "route" {
                    return Err(bad(name, 2, "\"draw\" or \"route\""));
                }
                if !["full", "direct", "flight"].contains(&st(2).as_str()) {
                    return Err(bad(name, 3, "\"full\", \"direct\" or \"flight\""));
                }
                if !self.editable(cv) || s(&self.a(cv, "cellViewType")) != Some("schematic") {
                    return Ok(V::Nil);
                }
                let Some(pts) = Self::points(&a[3], 2) else {
                    return Err(bad(name, 4, "a list of at least 2 points"));
                };
                let (xs, ys) = (num(&a[4]).unwrap_or(0.0), num(&a[5]).unwrap_or(0.0));
                let snap = |v: f64, g: f64| if g > 0.0 { (v / g).round() * g } else { v };
                // The database stores every point on the cellview's DBU grid,
                // so a segment shorter than half a DBU vanishes.
                let dbu = num(&self.a(cv, "DBUPerUU")).unwrap_or(160.0);
                let pts: Vec<(f64, f64)> = pts
                    .items()
                    .iter()
                    .filter_map(pt)
                    .map(|(x, y)| (snap(snap(x, xs), 1.0 / dbu), snap(snap(y, ys), 1.0 / dbu)))
                    .collect();
                let width = Self::width_on_grid(num(&a[6]).unwrap_or(0.0), dbu);
                // A segment that already exists makes the call return nil (VCLI).
                for w in pts.windows(2) {
                    if self.segment_exists(cv, w[0], w[1]) {
                        return Ok(V::Nil);
                    }
                }
                // "draw" adds one figure per segment, no solder dots, and splits a wire where
                // another wire's end lands inside it (a T), the new piece without properties
                // (live ICADVM20.1 run 2026-10-15; GUESS G5 had no split).
                let mut out = Vec::new();
                for w in pts.windows(2) {
                    if w[0] == w[1] {
                        continue;
                    }
                    let lpp = ("wire".to_owned(), "drawing".to_owned());
                    let mut extra = vec![
                        ("points", V::list(vec![vpt(w[0]), vpt(w[1])])),
                        ("nPoints", V::Int(2)),
                    ];
                    let ty = if width > 0.0 {
                        extra.push(("width", V::Float(width)));
                        "path"
                    } else {
                        "line"
                    };
                    out.push(V::Obj(self.new_shape(cv, ty, &lpp, extra)));
                }
                if entry == "draw" {
                    self.split_at_tees(cv);
                }
                V::list(out)
            }
            "schCreateWireLabel" => {
                let (cv, glue) = (o(0), o(1));
                if !self.editable(cv)
                    || !(self.is_wire(glue) || self.is_pin_fig(glue))
                    || st(3).is_empty()
                {
                    return Ok(V::Nil);
                }
                // An alias label too lands on wire/label (live ICADVM20.1 run).
                let lpp = ("wire", "label");
                let l = self.new_label(
                    cv,
                    &(lpp.0.into(), lpp.1.into()),
                    &[
                        a[2].clone(),
                        a[3].clone(),
                        a[4].clone(),
                        a[5].clone(),
                        a[6].clone(),
                        a[7].clone(),
                    ],
                    "normalLabel",
                )?;
                self.glue(l, Some(glue));
                V::Obj(l)
            }
            "schCreateNetExpression" => {
                let (cv, glue) = (o(0), o(2));
                let e = st(1);
                let expr_ok = e.starts_with("[@") && e.contains(":%:") && e.ends_with(']');
                if !self.editable(cv) || !expr_ok {
                    return Ok(V::Nil);
                }
                let lpp = if self.is_wire(glue) {
                    ("wire", "label")
                } else if self.is_pin_fig(glue) {
                    ("pin", "annotate")
                } else {
                    return Ok(V::Nil);
                };
                let l = self.new_label(
                    cv,
                    &(lpp.0.into(), lpp.1.into()),
                    &[
                        a[3].clone(),
                        // Virtuoso keeps the expression and labels it cdsNetExpr(), an IL
                        // label (live ICADVM20.1 run).
                        V::str("cdsNetExpr()"),
                        a[4].clone(),
                        a[5].clone(),
                        a[6].clone(),
                        a[7].clone(),
                    ],
                    "ILLabel",
                )?;
                // The expression itself is the label's netExpression property (live
                // ICADVM20.1 probe).
                self.set_prop(l, "netExpression", "string", V::str(&e));
                self.glue(l, Some(glue));
                V::Obj(l)
            }
            "schCreateNoteLabel" => {
                let cv = o(0);
                let ty = st(7);
                if !LABEL_TYPES.contains(&ty.as_str()) {
                    return Err(bad(name, 8, "normalLabel, NLPLabel or ILLabel"));
                }
                if !self.editable(cv) {
                    return Ok(V::Nil);
                }
                V::Obj(self.new_label(cv, &("text".into(), "drawing".into()), &a[1..7], &ty)?)
            }
            "schGlueLabel" => {
                let (l, f) = (o(0), o(1));
                if self.ty(l) != "label" || l == f {
                    return Ok(V::Nil);
                }
                self.glue(l, Some(f));
                V::T
            }
            "schCheck" => {
                let cv = o(0);
                if s(&self.a(cv, "cellViewType")) != Some("schematic") {
                    return Ok(V::Nil);
                }
                // The mock's checker: a wire label glued to nothing is an error.
                let errors = self
                    .a(cv, "shapes")
                    .items()
                    .iter()
                    .filter_map(Self::obj)
                    .filter(|&f| {
                        self.ty(f) == "label"
                            && s(&self.a(f, "layerName")) == Some("wire")
                            && !self.a(f, "parent").truthy()
                    })
                    .count() as i64;
                if self.editable(cv) {
                    // GUESS(G14): extraction stamps bookkeeping properties on the cellview.
                    let n = self.a(cv, "instances").items().len() as i64;
                    self.set_prop(
                        cv,
                        "lastSchematicExtraction",
                        "time",
                        V::str("Oct 12 12:00:00 2026"),
                    );
                    self.set_prop(cv, "instance#", "int", V::Int(n));
                }
                V::list(vec![V::Int(errors), V::Int(0)])
            }
            "schSolder" => {
                let cv = o(0);
                if !self.editable(cv) {
                    return Ok(V::Nil);
                }
                // GUESS(G15): a solder dot is a "dot" on wire drawing.
                let p = pt(&a[1]).expect("checked");
                V::Obj(self.new_shape(
                    cv,
                    "dot",
                    &("wire".into(), "drawing".into()),
                    vec![
                        ("xy", vpt(p)),
                        ("width", V::Float(0.025)),
                        ("height", V::Float(0.025)),
                    ],
                ))
            }
            // Instances only: unlinked from their cellview.
            "dbDeleteObject" => {
                let x = o(0);
                if self.ty(x) != "inst" {
                    return Err("*Error* dbDeleteObject: the mock deletes instances only".into());
                }
                let Some(cv) = Self::obj(&self.a(x, "cellView")) else {
                    return Ok(V::Nil);
                };
                if !self.editable(cv) {
                    return Ok(V::Nil);
                }
                let kept: Vec<V> = self
                    .a(cv, "instances")
                    .items()
                    .iter()
                    .filter(|i| Self::obj(i) != Some(x))
                    .cloned()
                    .collect();
                self.put(cv, "instances", V::list(kept));
                self.touch(cv);
                V::T
            }
            "dbGetOpenCellViews" => V::list(
                self.cvs
                    .values()
                    .filter(|&&cv| num(&self.a(cv, "sfyOpen")).unwrap_or(0.0) > 0.0)
                    .map(|&cv| V::Obj(cv))
                    .collect(),
            ),
            _ => return self.dispatch_cdf(name, a, kw),
        })
    }

    fn inst_named(&self, cv: u32, name: &str) -> bool {
        self.a(cv, "instances")
            .items()
            .iter()
            .filter_map(Self::obj)
            .any(|i| s(&self.a(i, "name")) == Some(name))
    }

    fn auto_name(&self, cv: u32, prefix: &str) -> String {
        (0..)
            .map(|k| format!("{prefix}{k}"))
            .find(|n| !self.inst_named(cv, n))
            .expect("a free name")
    }

    /// The angles (counter-clockwise from +x, y up) of the part of the ellipse in box `e`
    /// that lies inside box `arc`; `None` when that is all of it or nothing.
    fn arc_angles(
        e: ((f64, f64), (f64, f64)),
        arc: ((f64, f64), (f64, f64)),
    ) -> Option<(f64, f64)> {
        use std::f64::consts::{FRAC_PI_2, PI, TAU};
        let (cx, cy) = ((e.0.0 + e.1.0) / 2.0, (e.0.1 + e.1.1) / 2.0);
        let (rx, ry) = ((e.1.0 - e.0.0) / 2.0, (e.1.1 - e.0.1) / 2.0);
        let inside = |t: f64| {
            let (x, y) = (cx + rx * t.cos(), cy + ry * t.sin());
            x >= arc.0.0 - 1e-9 && x <= arc.1.0 + 1e-9 && y >= arc.0.1 - 1e-9 && y <= arc.1.1 + 1e-9
        };
        let mut cut = vec![0.0, FRAC_PI_2, PI, 3.0 * FRAC_PI_2];
        for x in [arc.0.0, arc.1.0] {
            let a = ((x - cx) / rx).clamp(-1.0, 1.0).acos();
            cut.extend([a, TAU - a]);
        }
        for y in [arc.0.1, arc.1.1] {
            let a = ((y - cy) / ry).clamp(-1.0, 1.0).asin();
            cut.extend([a.rem_euclid(TAU), (PI - a).rem_euclid(TAU)]);
        }
        for c in &mut cut {
            *c = c.rem_euclid(TAU);
            if TAU - *c < 1e-12 {
                *c = 0.0;
            }
        }
        cut.sort_by(f64::total_cmp);
        cut.dedup_by(|a, b| (*a - *b).abs() < 1e-12);
        let n = cut.len();
        let span = |k: usize| (cut[k], if k + 1 < n { cut[k + 1] } else { cut[0] + TAU });
        let ins: Vec<bool> = (0..n)
            .map(|k| {
                let (a, b) = span(k);
                inside((a + b) / 2.0)
            })
            .collect();
        // The arc starts after an outside span and runs over the inside ones that follow.
        let first = (0..n).find(|&k| ins[k] && !ins[(k + n - 1) % n])?;
        let mut last = first;
        while ins[(last + 1) % n] && (last + 1) % n != first {
            last = (last + 1) % n;
        }
        let (s0, _) = span(first);
        let (_, s1) = span(last);
        let s1 = if s1 > TAU + 1e-12 { s1 - TAU } else { s1 };
        Some((s0, s1))
    }

    /// A path or wide-wire width as Virtuoso stores it: rounded up to a multiple of two
    /// database units (0.0125 inch at 160; DB-270376 on a live ICADVM20.1 run).
    fn width_on_grid(w: f64, dbu: f64) -> f64 {
        if w <= 0.0 {
            return w;
        }
        let step = 2.0 / dbu;
        (w / step - 1e-9).ceil() * step
    }

    /// Splits every two-point wire of `cv` at each wire end that lies strictly inside it
    /// (collinear, between its ends), as Virtuoso's schematic editor does: the wire keeps its
    /// first part and properties, a new figure takes the rest.
    fn split_at_tees(&mut self, cv: u32) {
        loop {
            let wires: Vec<(u32, Vec<(f64, f64)>)> = self
                .a(cv, "shapes")
                .items()
                .iter()
                .filter_map(Self::obj)
                .filter(|&f| self.is_wire(f))
                .map(|f| {
                    (
                        f,
                        self.a(f, "points").items().iter().filter_map(pt).collect(),
                    )
                })
                .collect();
            let ends: Vec<(f64, f64)> = wires
                .iter()
                .flat_map(|(_, p)| [p[0], p[p.len() - 1]])
                .collect();
            let inside = |a: (f64, f64), b: (f64, f64), e: (f64, f64)| {
                let cross = (b.0 - a.0) * (e.1 - a.1) - (b.1 - a.1) * (e.0 - a.0);
                let dot = (e.0 - a.0) * (b.0 - a.0) + (e.1 - a.1) * (b.1 - a.1);
                let len = (b.0 - a.0).powi(2) + (b.1 - a.1).powi(2);
                cross.abs() < 1e-9 && dot > 1e-12 && dot < len - 1e-12
            };
            let hit = wires.iter().find_map(|(f, p)| {
                (p.len() == 2)
                    .then(|| {
                        ends.iter()
                            .find(|&&e| inside(p[0], p[1], e))
                            .map(|&e| (*f, p[0], p[1], e))
                    })
                    .flatten()
            });
            let Some((f, a, b, e)) = hit else { return };
            self.put(f, "points", V::list(vec![vpt(a), vpt(e)]));
            let lpp = ("wire".to_owned(), "drawing".to_owned());
            let ty = self.ty(f);
            let mut extra = vec![
                ("points", V::list(vec![vpt(e), vpt(b)])),
                ("nPoints", V::Int(2)),
            ];
            let w = self.a(f, "width");
            if w.truthy() {
                extra.push(("width", w));
            }
            self.new_shape(cv, &ty, &lpp, extra);
        }
    }

    fn segment_exists(&self, cv: u32, p: (f64, f64), q: (f64, f64)) -> bool {
        self.a(cv, "shapes")
            .items()
            .iter()
            .filter_map(Self::obj)
            .any(|f| {
                if !self.is_wire(f) {
                    return false;
                }
                let pts: Vec<(f64, f64)> =
                    self.a(f, "points").items().iter().filter_map(pt).collect();
                pts.windows(2)
                    .any(|w| (w[0] == p && w[1] == q) || (w[0] == q && w[1] == p))
            })
    }

    pub(super) fn get_attr(&self, id: u32, attr: &str) -> V {
        let ty = self.ty(id);
        match (ty.as_str(), attr) {
            (_, "bBox") if ty != "rect" && ty != "ellipse" => {
                return self.bbox(id).map_or(V::Nil, |(p, q)| vbox(p, q));
            }
            ("inst", "transform") => return self.inst_transform(id),
            ("prop", "value") => return self.prop_read(id),
            ("cdfParam", "value") => {
                if let Some(inst) = Self::obj(&self.a(id, "sfyInst")) {
                    let n = s(&self.a(id, "name")).unwrap_or("").to_owned();
                    return match self.find_prop(inst, &n) {
                        Some(p) => self.a(p, "value"),
                        None => self.a(id, "defValue"),
                    };
                }
                if !matches!(self.a(id, "sfyLive"), V::Nil) {
                    return self.a(id, "sfyLive");
                }
                return self.a(id, "defValue");
            }
            _ => {}
        }
        if let Some(v) = self.objs.get(id as usize).and_then(|o| o.attrs.get(attr)) {
            if DB_ORDER.contains(&attr) {
                let mut l = v.items().to_vec();
                l.reverse();
                return V::list(l);
            }
            return v.clone();
        }
        if ty == "cdf"
            && let Some(p) = self.cdf_param(id, attr)
        {
            return V::Obj(p);
        }
        match self.find_prop(id, attr) {
            Some(p) => self.prop_read(p),
            None => V::Nil,
        }
    }

    pub(super) fn set_attr(&mut self, id: u32, attr: &str, v: V) -> Result<V, String> {
        let ty = self.ty(id);
        if id as usize >= self.objs.len() || id == 0 {
            return Err(format!("db:{id}~>{attr}: no such object"));
        }
        match (ty.as_str(), attr) {
            ("label", "parent") => {
                let f = Self::obj(&v);
                self.glue(id, f);
            }
            ("label", "labelType") => {
                if !s(&v).is_some_and(|t| LABEL_TYPES.contains(&t)) {
                    return Err(format!("*Error* labelType: {v:?}"));
                }
                self.put(id, attr, v.clone());
            }
            (
                "label",
                "theLabel" | "justify" | "orient" | "font" | "height" | "xy" | "isVisible"
                | "isDrafted" | "isOverbar",
            )
            | ("arc", "startAngle" | "stopAngle" | "ellipseBBox")
            | ("inst", "orient" | "xy" | "name")
            | ("textDisplay", "orient" | "xy")
            | ("cellView", "DBUPerUU" | "userUnits")
            // An output's Plot check box (live: maeGetTestOutputs answers the session's struct).
            | ("sevOutputStruct", "plot")
            | ("cdf", _) => {
                if matches!(attr, "startAngle" | "stopAngle") && num(&v).is_none() {
                    return Err(format!("*Error* {attr}: expected a number"));
                }
                if ty == "cdf" && self.cdf_param(id, attr).is_some() {
                    return Err(format!(
                        "*Error* cdf~>{attr}: a parameter cannot be replaced"
                    ));
                }
                self.put(id, attr, v.clone());
            }
            ("cdfParam", "value") => {
                if let Some(inst) = Self::obj(&self.a(id, "sfyInst")) {
                    let r = self.set_inst_param(id, inst, v.clone())?;
                    self.leak_to_cell(inst, id, v);
                    return Ok(r);
                }
                // A cell CDF (cdfGetCellCDF) holds a live value beside its default; a base
                // CDF's value is its default.
                if s(&self.a(id, "sfyCellOf")).is_some() {
                    self.put(
                        id,
                        "sfyLive",
                        if super::super::equal(&v, &self.a(id, "defValue")) {
                            V::Nil
                        } else {
                            v.clone()
                        },
                    );
                } else {
                    self.put(id, "defValue", v.clone());
                }
            }
            ("cdfParam", _) => {
                self.put(id, attr, v.clone());
            }
            (_, _) if self.objs[id as usize].attrs.contains_key(attr) => {
                return Err(format!("*Error* {ty}~>{attr} is not settable"));
            }
            _ => {
                // An unknown name on a database object sets a property of that name.
                let pty = match &v {
                    V::Str(_) => "string",
                    V::Int(_) => "int",
                    V::Float(_) => "float",
                    V::T | V::Nil => "boolean",
                    V::List(_) => "ILList",
                    _ => return Err(format!("*Error* ~>{attr}: unsupported value")),
                };
                self.set_prop(id, attr, pty, v.clone());
            }
        }
        if let Some(cv) = self.cv_of(id) {
            self.touch(cv);
        }
        Ok(v)
    }
}
