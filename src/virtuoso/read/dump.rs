//! `.sfyx` records → the typed dump ([`Dump`]): the reader's boundary check (spec
//! `interop.md` §4.2).
//!
//! [`dump`] checks every record's arity, element types and context and files it into its
//! cellview's table; maestro records go to `ade` raw. The tables ([`Dump`], [`Cv`] and its
//! rows) are also what `write` builds from a Design and serializes.

use super::*;
use crate::virtuoso::kinds::{self, Scope};

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
    /// `isOverbar`.
    pub overbar: bool,
    /// Not `isDrafted` (Virtuoso draws labels drafted by default, live ICADVM20.1).
    pub undrafted: bool,
    /// Not `isVisible`.
    pub hidden: bool,
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

/// A textDisplay an instance owns (`td`): it shows a CDF value of the instance on the
/// canvas (sfy_cdfcb.il).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextDisplay {
    /// Ref of the owner instance.
    pub owner: u32,
    /// What it shows: `label` (a master label), `prop` (a property), `attr` (an attribute).
    pub assoc: String,
    /// The master label's text, or the property's or attribute's name.
    pub name: String,
    /// Layer.
    pub layer: String,
    /// Purpose.
    pub purpose: String,
    /// Where Virtuoso shows it.
    pub xy: [i32; 2],
    /// Orientation.
    pub orient: String,
    /// Justification.
    pub justify: String,
    /// Font.
    pub font: String,
    /// Height, 1/upi inch.
    pub height: i32,
    /// `isDrafted isOverbar isVisible isNameVisible isValueVisible`.
    pub flags: [bool; 5],
}

/// A property (`prop`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Prop {
    /// Ref of the owner, 0 = the cellview.
    pub r: u32,
    /// Name.
    pub name: String,
    /// `string int float boolean ILList netSet time ILExpr NLPExpr`.
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
    /// Instances' textDisplays.
    pub tds: Vec<TextDisplay>,
    /// Properties.
    pub props: Vec<Prop>,
    /// Every ref an object record of this cellview defined (§4.2: unique per cellview).
    pub refs: BTreeSet<u32>,
    /// Connection requests.
    pub conns: Vec<Conn>,
    /// Virtuoso's own instance terminal → net table (`tnet`: a schematic's connectivity as
    /// its database holds it, the import's connectivity oracle).
    pub tnets: Vec<Conn>,
    /// `(kind, count)` of the objects the dumper left out (`skipped`).
    pub skipped: Vec<(String, u32)>,
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
            + self.tds.len()
            + self.props.len()
            + self.conns.len()
            + self.tnets.len()
            + self.skipped.len()
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
    /// Config views (`virtuoso::config`).
    pub configs: Vec<crate::virtuoso::config::ConfigView>,
    /// Line of the `cell` record.
    pub line: u32,
    /// `(view, master file)` of each view the dump does not carry (`skipview`).
    pub skipviews: Vec<(String, String)>,
}

impl DCell {
    /// The cellview named `view`.
    pub fn cv(&self, view: &str) -> Option<&Cv> {
        self.cvs.iter().find(|c| c.view == view)
    }

    /// The symbol the cell's Schemify file is drawn from: view `symbol`, else its first
    /// symbol cellview (a master only placed through `symbol_xform`, say).
    pub fn symbol(&self) -> Option<&Cv> {
        self.cv("symbol")
            .or_else(|| self.cvs.iter().find(|c| c.vtype == "schematicSymbol"))
    }

    /// A supply symbol that is no device (S11): one `x!` terminal, no schematic and no
    /// simInfo, as a copy of analogLib `vdd` in a kit or a user library is.
    pub fn is_global_symbol(&self) -> bool {
        self.cv("schematic").is_none()
            && self.cdf.as_ref().is_none_or(|d| d.siminfo.is_empty())
            && self
                .symbol()
                .is_some_and(|s| matches!(&s.terms[..], [t] if t.name.ends_with('!')))
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
    /// `(library, its CDF's param count, has libInit.il)` (`libcdf`).
    pub libcdfs: Vec<(String, u32, bool)>,
}

fn ints(r: &Record, v: &[Value], what: &str) -> Result<Vec<i32>, Error> {
    v.iter()
        .map(|x| {
            x.as_int()
                .ok_or_else(|| r.err(format!("{what} holds a non-integer")))
        })
        .collect()
}

fn strs(r: &Record, v: &[Value], what: &str) -> Result<Vec<String>, Error> {
    v.iter()
        .map(|x| {
            x.as_str()
                .map(str::to_owned)
                .ok_or_else(|| r.err(format!("{what} holds a non-string")))
        })
        .collect()
}

pub(super) fn orient(r: &Record, i: usize) -> Result<String, Error> {
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
        if kinds::scope(tag) == Some(Scope::Maestro) {
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
            && matches!(
                kinds::scope(tag),
                Some(Scope::File | Scope::Cell | Scope::Object)
            )
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
            "libcdf" => {
                r.arity(2)?;
                let l = lib.clone().ok_or_else(|| r.err("before any `lib`"))?;
                let n = u32::try_from(r.int(0)?).map_err(|_| r.err("a negative count"))?;
                d.libcdfs.push((l, n, r.int(1)? != 0));
            }
            "cdf" | "param" | "cdfattr" | "paramattr" | "siminfo" | "textview" | "skipview"
            | "config" | "cv"
                if !in_cell =>
            {
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
            "param" | "cdfattr" | "paramattr" | "siminfo" if !in_cdf => {
                return Err(r.err("before the cell's `cdf`"));
            }
            "cdfattr" => {
                r.arity(2)?;
                let cdf = d.cells.last_mut().unwrap().cdf.as_mut().unwrap();
                cdf.attrs.push((r.str(0)?.to_owned(), r.str(1)?.to_owned()));
            }
            "paramattr" => {
                r.arity(3)?;
                let cdf = d.cells.last_mut().unwrap().cdf.as_mut().unwrap();
                let name = r.str(0)?;
                let p = cdf
                    .params
                    .iter_mut()
                    .find(|p| p.name == name)
                    .ok_or_else(|| r.err(format!("paramattr of unknown param {name}")))?;
                p.attrs.push((r.str(1)?.to_owned(), r.str(2)?.to_owned()));
            }
            "skipview" => {
                r.arity(2)?;
                d.cells
                    .last_mut()
                    .unwrap()
                    .skipviews
                    .push((r.str(0)?.to_owned(), r.str(1)?.to_owned()));
            }
            "param" => {
                // Eight fields, or ten in a design cell whose callback's code the dumper
                // read (`from`, `code`; sfy_cdfcb.il).
                if r.fields.len() != 10 {
                    r.arity(8)?;
                }
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
                    source: match r.fields.len() {
                        10 => Some((r.str(8)?.to_owned(), r.text(9)?)),
                        _ => None,
                    },
                    attrs: Vec::new(),
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
            "config" => {
                r.arity(2)?;
                d.cells
                    .last_mut()
                    .unwrap()
                    .configs
                    .push(crate::virtuoso::config::ConfigView {
                        view: r.str(0)?.to_owned(),
                        text: r.text(1)?,
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
                    if kinds::scope(tag) == Some(Scope::Object) {
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

pub(super) fn object(cv: &mut Cv, r: &Record) -> Result<(), Error> {
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
    let schematic_only = matches!(r.tag.as_str(), "conn" | "tnet")
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
            // Twelve fields, or fifteen with `isOverbar isDrafted isVisible` as 0/1, which a
            // dumper writes only when they are not Virtuoso's `0 1 1`.
            let flag = |k: usize| match r.fields.get(k).map(|_| r.int(k)).transpose()? {
                None => Ok(None),
                Some(0) => Ok(Some(false)),
                Some(1) => Ok(Some(true)),
                Some(_) => Err(r.err("a label flag is 0 or 1")),
            };
            if r.fields.len() != 15 {
                r.arity(12)?;
            }
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
                overbar: flag(12)? == Some(true),
                undrafted: flag(13)? == Some(false),
                hidden: flag(14)? == Some(false),
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
                "string"
                    | "int"
                    | "float"
                    | "boolean"
                    | "ILList"
                    | "netSet"
                    | "time"
                    | "ILExpr"
                    | "NLPExpr"
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
        "td" => {
            r.arity(16)?;
            let assoc = r.str(1)?;
            if !matches!(assoc, "label" | "prop" | "attr") {
                return Err(r.err(format!("textDisplay of `{assoc}`")));
            }
            let mut flags = [false; 5];
            for (k, f) in flags.iter_mut().enumerate() {
                *f = match r.int(11 + k)? {
                    0 => false,
                    1 => true,
                    _ => return Err(r.err("a textDisplay flag is 0 or 1")),
                };
            }
            cv.tds.push(TextDisplay {
                owner: pos(r, 0)?,
                assoc: assoc.to_owned(),
                name: r.str(2)?.to_owned(),
                layer: r.str(3)?.to_owned(),
                purpose: r.str(4)?.to_owned(),
                xy: [r.int(5)?, r.int(6)?],
                orient: orient(r, 7)?,
                justify: r.str(8)?.to_owned(),
                font: r.str(9)?.to_owned(),
                height: r.int(10)?,
                flags,
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
        "tnet" => {
            r.arity(3)?;
            cv.tnets.push(Conn {
                r: pos(r, 0)?,
                term: r.str(1)?.to_owned(),
                net: r.str(2)?.to_owned(),
            });
        }
        "skipped" => {
            r.arity(2)?;
            let n = u32::try_from(r.int(1)?).map_err(|_| r.err("a negative count"))?;
            cv.skipped.push((r.str(0)?.to_owned(), n));
        }
        _ => cv.unknown.push(r.clone()),
    }
    Ok(())
}

/// Refs unique; parents, owners and connection targets exist.
fn check_refs(cv: &Cv) -> Result<(), String> {
    let mut refs: Vec<u32> = cv
        .pinfigs
        .iter()
        .map(|x| x.r)
        .chain(cv.insts.iter().map(|x| x.r))
        .chain(cv.spins.iter().map(|x| x.r))
        .chain(cv.wires.iter().map(|x| x.r))
        .chain(cv.labels.iter().map(|x| x.r))
        .chain(cv.shapes.iter().map(|x| x.r))
        // An instance inside a symbol (a sheet border's title block) is reported, and its
        // properties with it (live ICADVM20.1: US_8ths/Asize).
        .chain(
            cv.unknown
                .iter()
                .filter(|r| matches!(r.tag.as_str(), "inst" | "spin" | "wire"))
                .filter_map(|r| u32::try_from(r.int(0).ok()?).ok()),
        )
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
    for c in cv.conns.iter().chain(&cv.tnets) {
        if !cv.insts.iter().any(|i| i.r == c.r) {
            return Err(format!("conn of unknown instance {}", c.r));
        }
    }
    for t in &cv.tds {
        if !cv.insts.iter().any(|i| i.r == t.owner) {
            return Err(format!("textDisplay of unknown instance {}", t.owner));
        }
    }
    for f in cv.pinfigs.iter().filter(|f| f.dir.is_empty()) {
        if !cv.terms.iter().any(|t| t.name == f.term) {
            return Err(format!("pinfig {} of unknown terminal {}", f.r, f.term));
        }
    }
    Ok(())
}
