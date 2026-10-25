//! The `.sfyx` dump model (spec `interop.md` §4.2): one table per record kind per cellview,
//! shared by both directions. [`dump`] parses records into it (every record checked, with
//! its line), [`records`] serializes it (§4.2 order, refs assigned after sorting).

use std::collections::{BTreeMap, BTreeSet};

use super::ade;
use crate::virtuoso::profile;
use crate::virtuoso::sfyx::{self, Error, Record, Value, i, pairs, s};

// Data design (six questions): in = one `.sfyx` text (≤ 2^30 bytes, ≤ 2^24 records); out
// = one Schemify document per design/ref cell. Rows per kind live in `Vec`s per
// cellview (existence-based: a kind with no records has an empty table); refs are the
// dump's integers, unique per cellview, looked up through a sorted index. Lifetime: one
// import call. Access: each kind is walked once in dump order by the builder.

/// Objects per cellview the reader accepts (format.md's rank-3 bound).
pub const OBJECTS_MAX: usize = 1 << 22;

/// A terminal (`term`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Term {
    /// Terminal name, Virtuoso syntax.
    pub name: String,
    /// `input output inputOutput switch …`.
    pub dir: String,
    /// Inherited-connection expression or `""`.
    pub netexpr: String,
}

/// A pin rectangle (`pinfig`): a symbol pin, or a schematic pin drawn as a bare shape.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PinFig {
    /// Ref.
    pub r: u32,
    /// Terminal name.
    pub term: String,
    /// `x1 y1 x2 y2`, 1/upi inch, y up.
    pub b: [i32; 4],
    /// In a schematic, the terminal's direction (a schematic dumps no `term`); empty in a
    /// symbol.
    pub dir: String,
}

/// An instance (`inst`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Inst {
    /// Ref.
    pub r: u32,
    /// Instance name, Virtuoso syntax.
    pub name: String,
    /// Master library.
    pub lib: String,
    /// Master cell.
    pub cell: String,
    /// Master view.
    pub view: String,
    /// Origin.
    pub xy: [i32; 2],
    /// Virtuoso orientation.
    pub orient: String,
}

/// A schematic pin (`spin`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Spin {
    /// Ref.
    pub r: u32,
    /// Terminal name.
    pub term: String,
    /// Direction.
    pub dir: String,
    /// Pin master library.
    pub lib: String,
    /// Pin master cell.
    pub cell: String,
    /// Pin master view.
    pub view: String,
    /// Origin.
    pub xy: [i32; 2],
    /// Orientation.
    pub orient: String,
}

/// A wire figure (`wire`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Wire {
    /// Ref.
    pub r: u32,
    /// Points (≥ 2).
    pub pts: Vec<[i32; 2]>,
    /// Width, 0 = narrow.
    pub width: i32,
}

/// A label (`label`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Label {
    /// Ref.
    pub r: u32,
    /// Ref of the glued figure, 0 = none.
    pub parent: u32,
    /// Text.
    pub text: String,
    /// Origin.
    pub xy: [i32; 2],
    /// Orientation.
    pub orient: String,
    /// Justification.
    pub justify: String,
    /// Font.
    pub font: String,
    /// Height, 1/upi inch.
    pub height: i32,
    /// `normal`, `NLP` or `IL`.
    pub kind: String,
    /// Layer.
    pub layer: String,
    /// Purpose.
    pub purpose: String,
}

/// Any other figure (`shape`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Shape {
    /// Ref.
    pub r: u32,
    /// `line rect polygon ellipse arc path`.
    pub kind: String,
    /// Layer.
    pub layer: String,
    /// Purpose.
    pub purpose: String,
    /// Points, box, or box plus angles (micro-radians).
    pub coords: Vec<i32>,
    /// Width.
    pub width: i32,
}

/// A property (`prop`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Prop {
    /// Ref of the owner, 0 = the cellview.
    pub r: u32,
    /// Name.
    pub name: String,
    /// `string int float boolean ILList`.
    pub ty: String,
    /// Value text.
    pub value: String,
}

impl Prop {
    /// A property of object `r` (0: the cellview) of SKILL type `ty`.
    pub fn new(r: u32, name: &str, ty: &str, value: impl Into<String>) -> Prop {
        Prop {
            r,
            name: name.to_owned(),
            ty: ty.to_owned(),
            value: value.into(),
        }
    }

    /// A `string` property, the type of every `schemify*` marker and of override text.
    pub fn string(r: u32, name: &str, value: impl Into<String>) -> Prop {
        Prop::new(r, name, "string", value)
    }
}

/// A loader-only connection request (`conn`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Conn {
    /// Instance ref.
    pub r: u32,
    /// Terminal.
    pub term: String,
    /// Net, Virtuoso syntax.
    pub net: String,
}

/// One cellview and its objects, one table per record kind.
#[derive(Clone, Debug, Default)]
pub struct Cv {
    /// View name.
    pub view: String,
    /// `schematic` or `schematicSymbol`.
    pub vtype: String,
    /// Units per inch.
    pub upi: i32,
    /// Terminals.
    pub terms: Vec<Term>,
    /// Symbol pin figures.
    pub pinfigs: Vec<PinFig>,
    /// Instances.
    pub insts: Vec<Inst>,
    /// Schematic pins.
    pub spins: Vec<Spin>,
    /// Wires.
    pub wires: Vec<Wire>,
    /// Labels.
    pub labels: Vec<Label>,
    /// Shapes.
    pub shapes: Vec<Shape>,
    /// Properties.
    pub props: Vec<Prop>,
    /// Every ref an object record of this cellview defined (§4.2: unique per cellview).
    pub refs: BTreeSet<u32>,
    /// Connection requests.
    pub conns: Vec<Conn>,
    /// Records with a tag this reader does not know, kept for the report.
    pub unknown: Vec<Record>,
}

impl Cv {
    /// Object records in this cellview (every row of every kind).
    pub fn objects(&self) -> usize {
        self.terms.len()
            + self.pinfigs.len()
            + self.insts.len()
            + self.spins.len()
            + self.wires.len()
            + self.labels.len()
            + self.shapes.len()
            + self.props.len()
            + self.conns.len()
            + self.unknown.len()
    }

    /// Value of property `name` of object `r`.
    pub fn prop(&self, r: u32, name: &str) -> Option<&str> {
        self.props
            .iter()
            .find(|p| p.r == r && p.name == name)
            .map(|p| p.value.as_str())
    }
}

/// One CDF parameter (`param`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CdfParam {
    /// Name.
    pub name: String,
    /// `string int float boolean radio cyclic button netSet`.
    pub ty: String,
    /// Default value text.
    pub def: String,
    /// Prompt.
    pub prompt: String,
    /// Units name or `""`.
    pub units: String,
    /// Flags (`parseAsNumber parseAsCEL storeDefault dontSave hidden`).
    pub flags: Vec<String>,
    /// Choices.
    pub choices: Vec<String>,
    /// Callback text.
    pub callback: String,
}

/// A simInfo value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SimVal {
    /// A string or symbol name.
    Str(String),
    /// A list of names (`termOrder`, `instParameters`, `otherParameters`).
    List(Vec<String>),
    /// Pairs (`propMapping`, `termMapping`).
    Pairs(Vec<(String, String)>),
}

/// One simulator's simInfo.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SimInfo {
    /// Simulator (`spectre`, `auCdl`, …).
    pub sim: String,
    /// `(key, value)` in record order.
    pub fields: Vec<(String, SimVal)>,
}

impl SimInfo {
    /// Field `key`.
    pub fn get(&self, key: &str) -> Option<&SimVal> {
        self.fields.iter().find(|f| f.0 == key).map(|f| &f.1)
    }

    /// Field `key` as a string.
    pub fn str(&self, key: &str) -> Option<&str> {
        match self.get(key)? {
            SimVal::Str(s) => Some(s),
            _ => None,
        }
    }

    /// Field `key` as a list (a string field is a one-element list).
    pub fn list(&self, key: &str) -> Vec<String> {
        match self.get(key) {
            Some(SimVal::List(v)) => v.clone(),
            Some(SimVal::Str(s)) if !s.is_empty() && s != "nil" => vec![s.clone()],
            _ => Vec::new(),
        }
    }

    /// Field `key` as pairs.
    pub fn pairs(&self, key: &str) -> Vec<(String, String)> {
        match self.get(key) {
            Some(SimVal::Pairs(v)) => v.clone(),
            _ => Vec::new(),
        }
    }
}

/// A cell's base CDF.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cdf {
    /// `paramLabelSet`.
    pub label_set: String,
    /// Parameters, CDF order.
    pub params: Vec<CdfParam>,
    /// simInfo per simulator.
    pub siminfo: Vec<SimInfo>,
}

impl Cdf {
    /// simInfo of `sim`.
    pub fn sim(&self, sim: &str) -> Option<&SimInfo> {
        self.siminfo.iter().find(|s| s.sim == sim)
    }
}

/// A text view (`textview`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextView {
    /// View name.
    pub view: String,
    /// Master file name.
    pub file: String,
    /// Content.
    pub text: String,
}

/// One cell of a dump.
#[derive(Clone, Debug, Default)]
pub struct DCell {
    /// Library.
    pub lib: String,
    /// Cell.
    pub name: String,
    /// Role `"design"` (true) or `"ref"`.
    pub design: bool,
    /// Base CDF.
    pub cdf: Option<Cdf>,
    /// Cellviews in dump order.
    pub cvs: Vec<Cv>,
    /// Text views.
    pub textviews: Vec<TextView>,
    /// Line of the `cell` record.
    pub line: u32,
}

impl DCell {
    /// The cellview named `view`.
    pub fn cv(&self, view: &str) -> Option<&Cv> {
        self.cvs.iter().find(|c| c.view == view)
    }
}

/// A parsed `.sfyx` file.
#[derive(Clone, Debug, Default)]
pub struct Dump {
    /// The producer string of the header.
    pub producer: String,
    /// Cells in dump order.
    pub cells: Vec<DCell>,
    /// Maestro tests: each starts with its `maestro` record.
    pub maestro: Vec<Vec<Record>>,
    /// Records outside any cellview with a tag this reader does not know.
    pub unknown: Vec<Record>,
}

/// The CDF parameter types a `param` record may carry.
pub const CDF_TYPES: [&str; 8] = [
    "string", "int", "float", "boolean", "radio", "cyclic", "button", "netSet",
];

/// The maestro-context record tags (§4.2, owned by `ade`).
pub(crate) const ADE_TAGS: [&str; 8] = [
    "maestro", "var", "analysis", "corner", "model", "output", "spec", "defn",
];

pub(crate) fn ints(r: &Record, v: &[Value], what: &str) -> Result<Vec<i32>, Error> {
    v.iter()
        .map(|x| {
            x.as_int()
                .ok_or_else(|| r.err(format!("{what} holds a non-integer")))
        })
        .collect()
}

pub(crate) fn strs(r: &Record, v: &[Value], what: &str) -> Result<Vec<String>, Error> {
    v.iter()
        .map(|x| {
            x.as_str()
                .map(str::to_owned)
                .ok_or_else(|| r.err(format!("{what} holds a non-string")))
        })
        .collect()
}

pub(crate) fn orient(r: &Record, i: usize) -> Result<String, Error> {
    let o = r.str(i)?;
    profile::orient_from_virtuoso(o)
        .map(|_| o.to_owned())
        .ok_or_else(|| r.err(format!("unknown orientation `{o}`")))
}

/// Parses records into a [`Dump`]: every record's arity and element types, its context
/// (an object needs a cellview, a cellview a cell, a cell a library, `param`/`siminfo` a
/// `cdf`), positive refs unique per cellview, glued parents and property owners that
/// exist. Unknown tags are kept (in their cellview, else file-level) for the report.
///
/// # Errors
///
/// The first malformed record, with its line; a missing header or `("end")`.
pub fn dump(records: &[Record]) -> Result<Dump, Error> {
    sfyx::require_complete(records)?;
    let mut d = Dump {
        producer: records[0].str(1).unwrap_or("").to_owned(),
        ..Dump::default()
    };
    let mut lib: Option<String> = None;
    let mut in_cell = false;
    let mut in_cdf = false;
    let mut in_maestro = false;
    let mut cells_seen: BTreeSet<(String, String)> = BTreeSet::new();
    for r in &records[1..records.len() - 1] {
        let tag = r.tag.as_str();
        if ADE_TAGS.contains(&tag) {
            if tag == "maestro" {
                in_maestro = true;
                d.maestro.push(vec![r.clone()]);
            } else if in_maestro {
                d.maestro.last_mut().expect("in maestro").push(r.clone());
            } else {
                return Err(r.err("outside a maestro context"));
            }
            continue;
        }
        if in_maestro
            && (is_object(tag)
                || matches!(
                    tag,
                    "lib" | "cell" | "cv" | "cdf" | "param" | "siminfo" | "textview"
                ))
        {
            return Err(r.err("cell records after the maestro records"));
        }
        match tag {
            "lib" => {
                r.arity(1)?;
                lib = Some(r.str(0)?.to_owned());
                in_cell = false;
                in_cdf = false;
            }
            "cell" => {
                r.arity(2)?;
                let l = lib.clone().ok_or_else(|| r.err("before any `lib`"))?;
                let design = match r.str(1)? {
                    "design" => true,
                    "ref" => false,
                    o => return Err(r.err(format!("role `{o}` (design or ref)"))),
                };
                let name = r.str(0)?.to_owned();
                if !cells_seen.insert((l.clone(), name.clone())) {
                    return Err(r.err(format!("{l}/{name} appears twice")));
                }
                d.cells.push(DCell {
                    lib: l,
                    name,
                    design,
                    line: r.line,
                    ..DCell::default()
                });
                in_cell = true;
                in_cdf = false;
            }
            "cdf" | "param" | "siminfo" | "textview" | "cv" if !in_cell => {
                return Err(r.err("outside a cell"));
            }
            "cdf" => {
                r.arity(1)?;
                let c = d.cells.last_mut().expect("in cell");
                if c.cdf.is_some() {
                    return Err(r.err("second cdf"));
                }
                c.cdf = Some(Cdf {
                    label_set: r.text(0)?,
                    ..Cdf::default()
                });
                in_cdf = true;
            }
            "param" | "siminfo" if !in_cdf => return Err(r.err("before the cell's `cdf`")),
            "param" => {
                r.arity(8)?;
                let ty = r.str(1)?;
                if !CDF_TYPES.contains(&ty) {
                    return Err(r.err(format!("type `{ty}`")));
                }
                let p = CdfParam {
                    name: r.str(0)?.to_owned(),
                    ty: ty.to_owned(),
                    def: r.str(2)?.to_owned(),
                    prompt: r.str(3)?.to_owned(),
                    units: r.str(4)?.to_owned(),
                    flags: strs(r, r.list(5)?, "flags")?,
                    choices: strs(r, r.list(6)?, "choices")?,
                    callback: r.text(7)?,
                };
                let cdf = d.cells.last_mut().unwrap().cdf.as_mut().unwrap();
                if cdf.params.iter().any(|x| x.name == p.name) {
                    return Err(r.err(format!("param {} twice", p.name)));
                }
                cdf.params.push(p);
            }
            "siminfo" => {
                r.arity(2)?;
                let mut s = SimInfo {
                    sim: r.str(0)?.to_owned(),
                    fields: Vec::new(),
                };
                for f in r.list(1)? {
                    let kv = f.items();
                    let (Some(Value::Str(k)), Some(v), 2) = (kv.first(), kv.get(1), kv.len())
                    else {
                        return Err(r.err("a field is not (key value)"));
                    };
                    let val = match v {
                        Value::Str(s) => SimVal::Str(s.clone()),
                        Value::List(items)
                            if items.iter().all(|x| matches!(x, Value::List(_)))
                                && !items.is_empty() =>
                        {
                            let mut pairs = Vec::new();
                            for p in items {
                                let pv = p.items();
                                match (pv.first(), pv.get(1), pv.len()) {
                                    (Some(Value::Str(a)), Some(Value::Str(b)), 2) => {
                                        pairs.push((a.clone(), b.clone()))
                                    }
                                    _ => {
                                        return Err(
                                            r.err(format!("{k}: a pair is not (name name)"))
                                        );
                                    }
                                }
                            }
                            SimVal::Pairs(pairs)
                        }
                        Value::List(items) => SimVal::List(strs(r, items, k)?),
                        Value::Int(_) => return Err(r.err(format!("{k}: an integer value"))),
                    };
                    s.fields.push((k.clone(), val));
                }
                let cdf = d.cells.last_mut().unwrap().cdf.as_mut().unwrap();
                if cdf.sim(&s.sim).is_some() {
                    return Err(r.err(format!("second {} simInfo", s.sim)));
                }
                cdf.siminfo.push(s);
            }
            "textview" => {
                r.arity(3)?;
                d.cells.last_mut().unwrap().textviews.push(TextView {
                    view: r.str(0)?.to_owned(),
                    file: r.str(1)?.to_owned(),
                    text: r.text(2)?,
                });
            }
            "cv" => {
                r.arity(3)?;
                let upi = r.int(2)?;
                if upi <= 0 {
                    return Err(r.err("units per inch must be positive"));
                }
                let c = d.cells.last_mut().unwrap();
                let view = r.str(0)?.to_owned();
                if c.cv(&view).is_some() {
                    return Err(r.err(format!("view {view} twice")));
                }
                let vtype = r.str(1)?;
                if !c.design && vtype == "schematic" {
                    return Err(r.err("a ref cell carries its symbol and CDF only (§4.2)"));
                }
                if vtype != "schematic" && vtype != "schematicSymbol" {
                    return Err(r.err(format!("view type `{vtype}`")));
                }
                c.cvs.push(Cv {
                    view,
                    vtype: vtype.to_owned(),
                    upi,
                    ..Cv::default()
                });
                in_cdf = false;
            }
            _ => {
                let Some(cv) = (if in_cell {
                    d.cells.last_mut().and_then(|c| c.cvs.last_mut())
                } else {
                    None
                }) else {
                    if is_object(tag) {
                        return Err(r.err("outside a cellview"));
                    }
                    d.unknown.push(r.clone());
                    continue;
                };
                if cv.objects() >= OBJECTS_MAX {
                    return Err(r.err(format!("more than {OBJECTS_MAX} objects in a cellview")));
                }
                object(cv, r)?;
            }
        }
    }
    for c in &d.cells {
        for cv in &c.cvs {
            check_refs(cv).map_err(|m| Error {
                line: c.line,
                msg: format!("{}/{} {}: {m}", c.lib, c.name, cv.view),
            })?;
        }
    }
    for t in &d.maestro {
        ade::check(t)?;
    }
    Ok(d)
}

pub(crate) fn is_object(tag: &str) -> bool {
    matches!(
        tag,
        "term" | "pinfig" | "inst" | "spin" | "wire" | "label" | "shape" | "prop" | "conn"
    )
}

pub(crate) fn object(cv: &mut Cv, r: &Record) -> Result<(), Error> {
    let pos = |r: &Record, i: usize| -> Result<u32, Error> {
        let v = r.int(i)?;
        u32::try_from(v)
            .ok()
            .filter(|v| *v > 0)
            .ok_or_else(|| r.err("refs are positive"))
    };
    // Objects belong to one kind of cellview (§4.2, §6.2): terminals and pin figures to a
    // symbol (a schematic's terminals travel as `spin`s), wires, schematic pins and loader
    // records to a schematic.
    let schematic_only = r.tag == "conn"
        || (matches!(r.tag.as_str(), "spin" | "wire") && cv.vtype != "schematicSymbol");
    let symbol_only = r.tag == "term";
    // A pin figure in a schematic (Cadence's sample library has them) from a dumper that
    // did not write its direction is reported below.
    if r.tag == "pinfig" && cv.vtype != "schematicSymbol" && r.fields.len() != 7 {
        cv.unknown.push(r.clone());
        return Ok(());
    }
    if (schematic_only && cv.vtype != "schematic") || (symbol_only && cv.vtype != "schematicSymbol")
    {
        return Err(r.err(format!(
            "`{}` does not belong in a {} cellview",
            r.tag, cv.vtype
        )));
    }
    // An instance inside a symbol (a pin master placed without flattening; the dumper writes
    // one of a pin cell as `spin`, a live run found such symbols in Cadence's `sample`
    // library) or a wire there has no Schemify form: reported, not refused.
    if matches!(r.tag.as_str(), "inst" | "spin" | "wire") && cv.vtype == "schematicSymbol" {
        cv.unknown.push(r.clone());
        return Ok(());
    }
    // A record that defines an object must not reuse a ref (a duplicated line would
    // otherwise become two objects of one identity).
    if matches!(
        r.tag.as_str(),
        "pinfig" | "inst" | "spin" | "wire" | "label" | "shape"
    ) {
        let ref_ = pos(r, 0)?;
        if !cv.refs.insert(ref_) {
            return Err(r.err(format!("ref {ref_} defined twice in this cellview")));
        }
    }
    match r.tag.as_str() {
        "term" => {
            r.arity(3)?;
            let name = r.str(0)?.to_owned();
            if cv.terms.iter().any(|t| t.name == name) {
                return Err(r.err(format!("terminal {name} twice")));
            }
            cv.terms.push(Term {
                name,
                dir: r.str(1)?.to_owned(),
                netexpr: r.str(2)?.to_owned(),
            });
        }
        "pinfig" => {
            r.arity(if cv.vtype == "schematicSymbol" { 6 } else { 7 })?;
            cv.pinfigs.push(PinFig {
                r: pos(r, 0)?,
                term: r.str(1)?.to_owned(),
                b: [r.int(2)?, r.int(3)?, r.int(4)?, r.int(5)?],
                dir: if r.fields.len() == 7 {
                    r.str(6)?.to_owned()
                } else {
                    String::new()
                },
            });
        }
        "inst" => {
            r.arity(8)?;
            cv.insts.push(Inst {
                r: pos(r, 0)?,
                name: r.str(1)?.to_owned(),
                lib: r.str(2)?.to_owned(),
                cell: r.str(3)?.to_owned(),
                view: r.str(4)?.to_owned(),
                xy: [r.int(5)?, r.int(6)?],
                orient: orient(r, 7)?,
            });
        }
        "spin" => {
            r.arity(9)?;
            cv.spins.push(Spin {
                r: pos(r, 0)?,
                term: r.str(1)?.to_owned(),
                dir: r.str(2)?.to_owned(),
                lib: r.str(3)?.to_owned(),
                cell: r.str(4)?.to_owned(),
                view: r.str(5)?.to_owned(),
                xy: [r.int(6)?, r.int(7)?],
                orient: orient(r, 8)?,
            });
        }
        "wire" => {
            r.arity(3)?;
            let p = ints(r, r.list(1)?, "points")?;
            if p.len() < 4 || p.len() % 2 != 0 {
                return Err(r.err("points: an even count of at least 4"));
            }
            cv.wires.push(Wire {
                r: pos(r, 0)?,
                pts: p.chunks(2).map(|c| [c[0], c[1]]).collect(),
                width: r.int(2)?,
            });
        }
        "label" => {
            r.arity(12)?;
            let kind = r.str(9)?;
            if !matches!(kind, "normal" | "NLP" | "IL") {
                return Err(r.err(format!("kind `{kind}`")));
            }
            let parent = r.int(1)?;
            cv.labels.push(Label {
                r: pos(r, 0)?,
                parent: u32::try_from(parent).map_err(|_| r.err("negative parent"))?,
                text: r.str(2)?.to_owned(),
                xy: [r.int(3)?, r.int(4)?],
                orient: orient(r, 5)?,
                justify: r.str(6)?.to_owned(),
                font: r.str(7)?.to_owned(),
                height: r.int(8)?,
                kind: kind.to_owned(),
                layer: r.str(10)?.to_owned(),
                purpose: r.str(11)?.to_owned(),
            });
        }
        "shape" => {
            r.arity(6)?;
            let kind = r.str(1)?;
            let coords = ints(r, r.list(4)?, "coords")?;
            let ok = match kind {
                "line" | "path" => coords.len() >= 4 && coords.len() % 2 == 0,
                "polygon" => coords.len() >= 6 && coords.len() % 2 == 0,
                "rect" | "ellipse" => coords.len() == 4,
                "arc" => coords.len() == 6,
                _ => return Err(r.err(format!("kind `{kind}`"))),
            };
            if !ok {
                return Err(r.err(format!("{} coordinates for a {kind}", coords.len())));
            }
            cv.shapes.push(Shape {
                r: pos(r, 0)?,
                kind: kind.to_owned(),
                layer: r.str(2)?.to_owned(),
                purpose: r.str(3)?.to_owned(),
                coords,
                width: r.int(5)?,
            });
        }
        "prop" => {
            r.arity(4)?;
            let ty = r.str(2)?;
            if !matches!(
                ty,
                "string" | "int" | "float" | "boolean" | "ILList" | "netSet"
            ) {
                return Err(r.err(format!("type `{ty}`")));
            }
            let owner = r.int(0)?;
            cv.props.push(Prop {
                r: u32::try_from(owner).map_err(|_| r.err("negative owner"))?,
                name: r.str(1)?.to_owned(),
                ty: ty.to_owned(),
                value: r.str(3)?.to_owned(),
            });
        }
        "conn" => {
            r.arity(3)?;
            cv.conns.push(Conn {
                r: pos(r, 0)?,
                term: r.str(1)?.to_owned(),
                net: r.str(2)?.to_owned(),
            });
        }
        _ => cv.unknown.push(r.clone()),
    }
    Ok(())
}

/// Refs unique; parents, owners and connection targets exist.
pub(crate) fn check_refs(cv: &Cv) -> Result<(), String> {
    let mut refs: Vec<u32> = cv
        .pinfigs
        .iter()
        .map(|x| x.r)
        .chain(cv.insts.iter().map(|x| x.r))
        .chain(cv.spins.iter().map(|x| x.r))
        .chain(cv.wires.iter().map(|x| x.r))
        .chain(cv.labels.iter().map(|x| x.r))
        .chain(cv.shapes.iter().map(|x| x.r))
        .collect();
    let n = refs.len();
    refs.sort_unstable();
    refs.dedup();
    if refs.len() != n {
        return Err("a ref is used twice".into());
    }
    let has = |r: u32| refs.binary_search(&r).is_ok();
    for l in &cv.labels {
        if l.parent != 0 && !has(l.parent) {
            return Err(format!("label {} glued to unknown ref {}", l.r, l.parent));
        }
    }
    for p in &cv.props {
        if p.r != 0 && !has(p.r) {
            return Err(format!("property {} of unknown ref {}", p.name, p.r));
        }
    }
    for c in &cv.conns {
        if !cv.insts.iter().any(|i| i.r == c.r) {
            return Err(format!("conn of unknown instance {}", c.r));
        }
    }
    for f in cv.pinfigs.iter().filter(|f| f.dir.is_empty()) {
        if !cv.terms.iter().any(|t| t.name == f.term) {
            return Err(format!("pinfig {} of unknown terminal {}", f.r, f.term));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------
// Serialization: Dump → records (§4.2 order, refs assigned after sorting)
// ---------------------------------------------------------------------------------------

/// The `cdf`, `param` and `siminfo` records of `cdf` (§4.2): params in CDF order, siminfo
/// as stored (the dump writes it sorted by simulator).
pub fn cdf_records(cdf: &Cdf) -> Vec<Record> {
    let mut out = vec![Record::new("cdf", vec![sfyx::text(&cdf.label_set)])];
    for p in &cdf.params {
        out.push(Record::new(
            "param",
            vec![
                s(&p.name),
                s(&p.ty),
                s(&p.def),
                s(&p.prompt),
                s(&p.units),
                Value::List(p.flags.iter().map(|f| s(f)).collect()),
                Value::List(p.choices.iter().map(|f| s(f)).collect()),
                sfyx::text(&p.callback),
            ],
        ));
    }
    for si in &cdf.siminfo {
        let fields = si
            .fields
            .iter()
            .map(|(k, v)| {
                let v = match v {
                    SimVal::Str(x) => s(x),
                    SimVal::List(l) => Value::List(l.iter().map(|x| s(x)).collect()),
                    SimVal::Pairs(p) => pairs(p.iter().map(|(a, b)| (a.as_str(), b.as_str()))),
                };
                Value::List(vec![s(k), v])
            })
            .collect();
        out.push(Record::new(
            "siminfo",
            vec![s(&si.sim), Value::List(fields)],
        ));
    }
    debug_assert_eq!(out.len(), 1 + cdf.params.len() + cdf.siminfo.len());
    out
}

/// The records of `d` in §4.2 order: header; per library (sorted), its `ref` cells then
/// its `design` cells (each sorted by name) with CDF, cellviews (sorted by view, objects
/// in kind order, each kind sorted by name, else coordinates, else text) and text views;
/// maestro records; `end`. Refs are renumbered 1… in emission order, so the same tables
/// always give the same records.
pub fn records(d: &Dump) -> Vec<Record> {
    let mut out = vec![Record::new("sfyx", vec![i(1), s(&d.producer)])];
    let mut order: Vec<&DCell> = d.cells.iter().collect();
    order.sort_by(|a, b| (&a.lib, a.design, &a.name).cmp(&(&b.lib, b.design, &b.name)));
    let mut lib: Option<&str> = None;
    for c in order {
        if lib != Some(c.lib.as_str()) {
            out.push(Record::new("lib", vec![s(&c.lib)]));
            lib = Some(&c.lib);
        }
        out.push(Record::new(
            "cell",
            vec![s(&c.name), s(if c.design { "design" } else { "ref" })],
        ));
        if let Some(cdf) = &c.cdf {
            out.extend(cdf_records(cdf));
        }
        let mut cvs: Vec<&Cv> = c.cvs.iter().collect();
        cvs.sort_by(|a, b| a.view.cmp(&b.view));
        for cv in cvs {
            cv_records(cv, &mut out);
        }
        let mut tvs: Vec<&TextView> = c.textviews.iter().collect();
        tvs.sort_by(|a, b| a.view.cmp(&b.view));
        for t in tvs {
            out.push(Record::new(
                "textview",
                vec![s(&t.view), s(&t.file), sfyx::text(&t.text)],
            ));
        }
    }
    // Maestro tests by (cell, test name), as the SKILL dumper writes them (§4.2).
    let mut tests: Vec<&Vec<Record>> = d.maestro.iter().collect();
    let key = |t: &Vec<Record>| -> (String, String) {
        let r = &t[0];
        (
            r.str(2).unwrap_or("").to_owned(),
            r.str(0).unwrap_or("").to_owned(),
        )
    };
    tests.sort_by_key(|t| key(t));
    for test in tests {
        out.extend(test.iter().cloned());
    }
    out.push(Record::new("end", vec![]));
    out
}

/// A wire's points as written: a two-point wire from its smaller end (x, then y), as the
/// SKILL dumper writes it, so drawing direction never shows as a difference (§4.2).
pub(crate) fn canon(p: &[[i32; 2]]) -> Vec<[i32; 2]> {
    let mut v = p.to_vec();
    if v.len() == 2 && v[1] < v[0] {
        v.swap(0, 1);
    }
    v
}

pub(crate) fn pts(v: &[[i32; 2]]) -> Value {
    Value::List(v.iter().flat_map(|p| [i(p[0]), i(p[1])]).collect())
}

pub(crate) fn cv_records(cv: &Cv, out: &mut Vec<Record>) {
    out.push(Record::new(
        "cv",
        vec![s(&cv.view), s(&cv.vtype), i(cv.upi)],
    ));
    let mut terms: Vec<&Term> = cv.terms.iter().collect();
    terms.sort_by(|a, b| a.name.cmp(&b.name));
    for t in terms {
        out.push(Record::new(
            "term",
            vec![s(&t.name), s(&t.dir), s(&t.netexpr)],
        ));
    }
    // Old ref → new ref.
    let mut map: BTreeMap<u32, u32> = BTreeMap::new();
    let mut next = 1u32;
    let mut renum = |old: u32, map: &mut BTreeMap<u32, u32>| -> u32 {
        let n = next;
        next += 1;
        let prev = map.insert(old, n);
        assert!(prev.is_none(), "provisional ref {old} used twice");
        n
    };
    let mut pf: Vec<&PinFig> = cv.pinfigs.iter().collect();
    pf.sort_by(|a, b| (&a.term, a.b).cmp(&(&b.term, b.b)));
    let pf: Vec<(u32, &PinFig)> = pf.into_iter().map(|p| (renum(p.r, &mut map), p)).collect();
    let mut ins: Vec<&Inst> = cv.insts.iter().collect();
    ins.sort_by(|a, b| (&a.name, a.xy, &a.lib, &a.cell).cmp(&(&b.name, b.xy, &b.lib, &b.cell)));
    let ins: Vec<(u32, &Inst)> = ins.into_iter().map(|p| (renum(p.r, &mut map), p)).collect();
    let mut sp: Vec<&Spin> = cv.spins.iter().collect();
    sp.sort_by(|a, b| (&a.term, a.xy).cmp(&(&b.term, b.xy)));
    let sp: Vec<(u32, &Spin)> = sp.into_iter().map(|p| (renum(p.r, &mut map), p)).collect();
    let mut wi: Vec<&Wire> = cv.wires.iter().collect();
    wi.sort_by_key(|w| (canon(&w.pts), w.width));
    let wi: Vec<(u32, &Wire)> = wi.into_iter().map(|p| (renum(p.r, &mut map), p)).collect();
    let mut la: Vec<&Label> = cv.labels.iter().collect();
    la.sort_by(|a, b| {
        (a.xy, &a.text, &a.layer, &a.purpose, &a.kind, &a.orient)
            .cmp(&(b.xy, &b.text, &b.layer, &b.purpose, &b.kind, &b.orient))
    });
    let la: Vec<(u32, &Label)> = la.into_iter().map(|p| (renum(p.r, &mut map), p)).collect();
    let mut sh: Vec<&Shape> = cv.shapes.iter().collect();
    sh.sort_by(|a, b| {
        (&a.coords, &a.kind, &a.layer, &a.purpose).cmp(&(&b.coords, &b.kind, &b.layer, &b.purpose))
    });
    let sh: Vec<(u32, &Shape)> = sh.into_iter().map(|p| (renum(p.r, &mut map), p)).collect();
    let rr = |old: u32| -> u32 {
        if old == 0 {
            0
        } else {
            *map.get(&old)
                .unwrap_or_else(|| panic!("unknown provisional ref {old}"))
        }
    };
    for (r, p) in pf {
        let mut f = vec![
            i(r as i32),
            s(&p.term),
            i(p.b[0]),
            i(p.b[1]),
            i(p.b[2]),
            i(p.b[3]),
        ];
        if !p.dir.is_empty() {
            f.push(s(&p.dir));
        }
        out.push(Record::new("pinfig", f));
    }
    for (r, x) in ins {
        out.push(Record::new(
            "inst",
            vec![
                i(r as i32),
                s(&x.name),
                s(&x.lib),
                s(&x.cell),
                s(&x.view),
                i(x.xy[0]),
                i(x.xy[1]),
                s(&x.orient),
            ],
        ));
    }
    for (r, x) in sp {
        out.push(Record::new(
            "spin",
            vec![
                i(r as i32),
                s(&x.term),
                s(&x.dir),
                s(&x.lib),
                s(&x.cell),
                s(&x.view),
                i(x.xy[0]),
                i(x.xy[1]),
                s(&x.orient),
            ],
        ));
    }
    for (r, w) in wi {
        out.push(Record::new(
            "wire",
            vec![i(r as i32), pts(&canon(&w.pts)), i(w.width)],
        ));
    }
    for (r, l) in la {
        out.push(Record::new(
            "label",
            vec![
                i(r as i32),
                i(rr(l.parent) as i32),
                s(&l.text),
                i(l.xy[0]),
                i(l.xy[1]),
                s(&l.orient),
                s(&l.justify),
                s(&l.font),
                i(l.height),
                s(&l.kind),
                s(&l.layer),
                s(&l.purpose),
            ],
        ));
    }
    for (r, x) in sh {
        out.push(Record::new(
            "shape",
            vec![
                i(r as i32),
                s(&x.kind),
                s(&x.layer),
                s(&x.purpose),
                Value::List(x.coords.iter().map(|v| i(*v)).collect()),
                i(x.width),
            ],
        ));
    }
    let mut props: Vec<(u32, &Prop)> = cv.props.iter().map(|p| (rr(p.r), p)).collect();
    props.sort_by(|a, b| (a.0, &a.1.name).cmp(&(b.0, &b.1.name)));
    for (r, p) in props {
        out.push(Record::new(
            "prop",
            vec![i(r as i32), s(&p.name), s(&p.ty), s(&p.value)],
        ));
    }
    let mut conns: Vec<(u32, &Conn)> = cv.conns.iter().map(|c| (rr(c.r), c)).collect();
    conns.sort_by(|a, b| (a.0, &a.1.term).cmp(&(b.0, &b.1.term)));
    for (r, c) in conns {
        out.push(Record::new(
            "conn",
            vec![i(r as i32), s(&c.term), s(&c.net)],
        ));
    }
}

/// The `schemifyNet` property value export writes when Virtuoso's spelling of a net does not
/// read back as it (§3.3): the Schemify net text, ` global` appended for a global label.
pub(crate) fn net_prop(net: &str, global: bool) -> String {
    if global {
        format!("{net} global")
    } else {
        net.to_owned()
    }
}
