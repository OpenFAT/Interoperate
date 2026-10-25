//! `.sfyx` records → typed dump → Schemify files (spec `interop.md` §6, §7.3).
//!
//! Spine of [`import`]: `sfyx::read` → `dump::dump` (records into the dump model, every
//! record checked, with its line) → interfaces of every dump cell (pins, params, simInfo
//! formats) → one fresh document per target file (`build`) → `ade::import` for benches
//! → `merge::merge` with the existing file or the export's stash → manifest rows → files.

use std::collections::{BTreeMap, BTreeSet};

use schemify_format::{Document, FileKind, IdRng, Orient, RecordKind, fresh_id, is_id, parse};

use super::ade;
use super::ael::{self, AelName};
use super::devices;
use super::dump::*;
use super::{Import, ImportOptions};
use crate::design::{
    Files, cell_path, copy_header, push_anchored, push_attr, push_lib, push_record,
};
use crate::notes::Notes;
use crate::virtuoso::map::Mapping;
use crate::virtuoso::merge;
use crate::virtuoso::plan::{CORE_LIB, ROOT_LIB};
use crate::virtuoso::profile::{self, coord_text, net_from_virtuoso, point_from_virtuoso};
use crate::virtuoso::sfyx::{self, Record};

mod cells;
mod interface;
mod schematic;
mod symbol;
use cells::*;
use interface::*;
use schematic::*;
use symbol::*;

// ---------------------------------------------------------------------------------------
// Import: dump → Schemify documents
// ---------------------------------------------------------------------------------------

/// Where one dump cell lands.
#[derive(Clone, Debug, Default)]
struct Target {
    /// A file is produced.
    write: bool,
    /// Schemify library (`core` for the bundled cells).
    slib: String,
    /// Project-relative directory.
    dir: String,
    /// File stem.
    stem: String,
    /// File kind.
    kind: Option<FileKind>,
}

impl Target {
    fn path(&self) -> String {
        cell_path(
            &self.dir,
            &self.stem,
            self.kind.map_or("shprim", FileKind::word),
        )
    }
}

/// A pin of an interface, Schemify spelling and coordinates.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct PinRow {
    name: String,
    dir: String,
    at: [i32; 2],
    inherit: Option<String>,
    alias: Option<String>,
}

/// What instances of a master need: pins (port order), params with their Schemify kinds
/// and defaults, and the netSet property name of each inherited pin.
#[derive(Clone, Debug, Default)]
struct Iface {
    pins: Vec<PinRow>,
    /// `(name, kind, default)`.
    params: Vec<(String, String, String)>,
    /// `(virtuoso netSet prop, schemify pin)`.
    inherit_props: Vec<(String, String)>,
    /// Params a rule computes (`computed`): an instance takes no value for them.
    computed: Vec<String>,
    /// Params only Virtuoso's callbacks derive (`vderived true`): an instance carries
    /// their values as `cached` lines (D20 as amended 2026-10-15).
    vderived: Vec<String>,
    /// What those depend on (header `vinputs`).
    vinputs: Vec<String>,
}

impl Iface {
    fn kind(&self, p: &str) -> Option<&str> {
        self.params.iter().find(|x| x.0 == p).map(|x| x.1.as_str())
    }
    fn def(&self, p: &str) -> Option<&str> {
        self.params.iter().find(|x| x.0 == p).map(|x| x.2.as_str())
    }
}

/// The `cached <param> <value> <digest>` lines of an instance (D20 as amended 2026-10-15):
/// each derived param's value as Virtuoso had it (`derived`; a param Virtuoso did not store
/// is at its default), with the digest of the instance's input values (`overrides`, else
/// the master's defaults). None when an input or a value is not a constant: the instance
/// then has no cached value, and elab reports it stale once an input is set (E039).
fn cached_lines(
    iface: &Iface,
    overrides: &[(String, String)],
    mut derived: Vec<(String, String)>,
) -> Vec<[String; 3]> {
    if iface.vderived.is_empty() || iface.vinputs.is_empty() {
        return Vec::new();
    }
    let template = |n: &str| matches!(iface.kind(n), Some("string" | "choice"));
    let digest = super::characterize::inputs_digest(&iface.vinputs, |n| {
        let v = overrides
            .iter()
            .find(|o| o.0 == n)
            .map(|o| o.1.as_str())
            .or_else(|| iface.def(n))?;
        Some((v, template(n)))
    });
    let Some(digest) = digest else {
        return Vec::new();
    };
    for p in &iface.vderived {
        if !derived.iter().any(|d| &d.0 == p)
            && let Some(def) = iface.def(p)
        {
            derived.push((p.clone(), def.to_owned()));
        }
    }
    derived.sort_by_key(|(n, _)| iface.params.iter().position(|p| &p.0 == n));
    derived
        .into_iter()
        .filter_map(|(p, v)| {
            let v = super::characterize::canonical_value(&v, template(&p))?;
            Some([p, v, digest.clone()])
        })
        .collect()
}

/// One import's lookup tables.
struct Ctx<'a> {
    dump: &'a Dump,
    map: &'a Mapping,
    opts: &'a ImportOptions,
    existing: &'a Files,
    /// Schemify library → directory (manifest, then options).
    libs: BTreeMap<String, String>,
    targets: Vec<Target>,
    ifaces: Vec<Option<Iface>>,
    /// (lib, cell) of every cell a maestro test names.
    benches: BTreeSet<(String, String)>,
}

/// Schemify library of Virtuoso library `vlib`.
fn slib_of(vlib: &str, opts: &ImportOptions) -> String {
    if let Some(s) = opts.lib_map.get(vlib) {
        return s.clone();
    }
    match vlib {
        ROOT_LIB => String::new(),
        CORE_LIB => "core".into(),
        l => l.into(),
    }
}

fn consumed(lib: &str, cell: &str) -> bool {
    (lib == "basic" && matches!(cell, "ipin" | "opin" | "iopin" | "noConn"))
        || devices::is_global_symbol(lib, cell)
}

impl<'a> Ctx<'a> {
    fn new(
        dump: &'a Dump,
        existing: &'a Files,
        map: &'a Mapping,
        opts: &'a ImportOptions,
        notes: &mut Notes,
    ) -> Ctx<'a> {
        let mut libs = existing
            .get("schemify.project")
            .and_then(|m| crate::design::manifest_libs(m).ok())
            .unwrap_or_default();
        for (l, d) in &opts.lib_dir {
            libs.entry(l.clone()).or_insert_with(|| d.clone());
        }
        let benches = dump
            .maestro
            .iter()
            .filter_map(|t| {
                let m = t.first()?;
                Some((m.str(1).ok()?.to_owned(), m.str(2).ok()?.to_owned()))
            })
            .collect();
        let mut ctx = Ctx {
            dump,
            map,
            opts,
            existing,
            libs,
            targets: Vec::new(),
            ifaces: Vec::new(),
            benches,
        };
        for k in 0..dump.cells.len() {
            let t = ctx.target(k, notes);
            ctx.targets.push(t);
        }
        for k in 0..dump.cells.len() {
            let i = ctx.iface_of_dump(k);
            ctx.ifaces.push(i);
        }
        ctx
    }

    fn dir_of(&self, slib: &str) -> String {
        if slib.is_empty() {
            return String::new();
        }
        self.libs
            .get(slib)
            .cloned()
            .unwrap_or_else(|| slib.to_owned())
    }

    fn target(&self, k: usize, notes: &mut Notes) -> Target {
        let c = &self.dump.cells[k];
        let slib = slib_of(&c.lib, self.opts);
        let mut t = Target {
            dir: self.dir_of(&slib),
            slib,
            stem: c.name.clone(),
            ..Target::default()
        };
        if t.slib == "core" || consumed(&c.lib, &c.name) {
            return t;
        }
        let sym = c.cv("symbol").is_some();
        let sch = c.cv("schematic");
        t.kind = if !c.design {
            Some(FileKind::Shprim)
        } else if self.benches.contains(&(c.lib.clone(), c.name.clone())) {
            Some(FileKind::Shbench)
        } else if sym && sch.is_some() {
            Some(FileKind::Shcomp)
        } else if c.cv("symbol").is_some_and(|s| s.terms.is_empty()) {
            // A symbol without terminals (a title block, a graphic) is no device.
            notes.dropped(
                "display-only",
                &format!("{}/{}", c.lib, c.name),
                "a symbol without pins is not imported".to_owned(),
            );
            None
        } else if sym {
            Some(FileKind::Shprim)
        } else if let Some(s) = sch {
            Some(if s.spins.is_empty() {
                FileKind::Shbench
            } else {
                FileKind::Shcomp
            })
        } else {
            notes.review(
                "format",
                &format!("{}/{}", c.lib, c.name),
                "no symbol or schematic view: not imported",
            );
            None
        };
        t.write = t.kind.is_some();
        t
    }

    /// The dump index of `lib/cell`.
    fn find(&self, lib: &str, cell: &str) -> Option<usize> {
        self.dump
            .cells
            .iter()
            .position(|c| c.lib == lib && c.name == cell)
    }

    fn iface_of_dump(&self, k: usize) -> Option<Iface> {
        let c = &self.dump.cells[k];
        let mut i = Iface::default();
        if let Some(sym) = c.cv("symbol") {
            i.pins = pins_in(c, sym, &mut Notes::default());
            for p in &i.pins {
                if p.inherit.is_some() {
                    i.inherit_props
                        .push((super::write::vname(&p.name), p.name.clone()));
                }
            }
        }
        if let Some(sch) = c.cv("schematic") {
            for l in &sch.labels {
                if let Some((p, d)) = parse_netexpr(&l.text) {
                    let name = net_from_virtuoso(&p).text;
                    if !i.pins.iter().any(|x| x.name == name) {
                        i.inherit_props.push((p.clone(), name.clone()));
                        i.pins.push(PinRow {
                            name,
                            dir: "inout".into(),
                            at: [0, 0],
                            inherit: Some(d),
                            alias: None,
                        });
                    }
                }
            }
        }
        if let Some(cdf) = &c.cdf {
            for p in params_in(c, cdf, &mut Notes::default()) {
                i.params.push((p.name, p.kind, p.default));
            }
            for p in cdf.params.iter().filter(|p| p.ty == "netSet") {
                if let Some(pin) = i
                    .pins
                    .iter()
                    .find(|x| super::write::vname(&x.name) == p.name)
                {
                    let pn = pin.name.clone();
                    if !i.inherit_props.iter().any(|x| x.0 == p.name) {
                        i.inherit_props.push((p.name.clone(), pn));
                    }
                }
            }
        }
        Some(i)
    }

    /// Interface of a Schemify cell file text.
    fn iface_of_text(bytes: &[u8]) -> Option<Iface> {
        let d = parse(bytes).ok()?;
        let mut i = Iface::default();
        if let Some(a) = d.find_attr(0, "vinputs") {
            i.vinputs = d.attr_values(a).map(str::to_owned).collect();
        }
        for r in 1..d.record_count() {
            match d.record_kind(r) {
                RecordKind::Pin => {
                    let attr = |k: &str| {
                        d.find_attr(r, k)
                            .and_then(|a| d.attr_value(a, 0))
                            .map(str::to_owned)
                    };
                    let row = PinRow {
                        name: d.field(r, 0)?.to_owned(),
                        dir: d.field(r, 1)?.to_owned(),
                        at: [
                            schemify_format::parse_coord(d.field(r, 2)?).ok()?,
                            schemify_format::parse_coord(d.field(r, 3)?).ok()?,
                        ],
                        inherit: attr("inherit"),
                        alias: attr("alias"),
                    };
                    if row.inherit.is_some() {
                        i.inherit_props
                            .push((super::write::vname(&row.name), row.name.clone()));
                    }
                    i.pins.push(row);
                }
                RecordKind::Param => {
                    let kind = d
                        .find_attr(r, "kind")
                        .and_then(|a| d.attr_value(a, 0))
                        .unwrap_or("number");
                    let name = d.field(r, 0)?.to_owned();
                    if d.find_attr(r, "computed").is_some() {
                        i.computed.push(name.clone());
                    }
                    let derived = d.find_attr(r, "vderived").and_then(|a| d.attr_value(a, 0));
                    if derived == Some("true") {
                        i.vderived.push(name.clone());
                    }
                    i.params
                        .push((name, kind.to_owned(), d.field(r, 1)?.to_owned()));
                }
                _ => {}
            }
        }
        Some(i)
    }

    /// Where an instance of `lib/cell` points and what its master looks like.
    fn master(&self, lib: &str, cell: &str) -> Master {
        if let Some(row) = self.map.by_virtuoso(lib, cell) {
            let (sl, stem) = row.cellref.split_once(':').unwrap_or(("", &row.cellref));
            return Master {
                slib: sl.to_owned(),
                dir: self.dir_of(sl),
                stem: stem.to_owned(),
                iface: self.project_iface(sl, stem),
                row: self
                    .map
                    .maps
                    .iter()
                    .position(|r| r.lib == lib && r.cell == cell),
                ..Master::default()
            };
        }
        if self.opts.analoglib
            && lib == "analogLib"
            && let Some((dv, wave)) = devices::by_alib(cell)
        {
            return Master {
                slib: "core".into(),
                stem: dv.core.into(),
                iface: self.project_iface("core", dv.core),
                alib: Some((
                    devices::DEVICES
                        .iter()
                        .position(|x| x.core == dv.core)
                        .expect("row"),
                    wave,
                )),
                ..Master::default()
            };
        }
        let slib = slib_of(lib, self.opts);
        if slib == "core" {
            return Master {
                stem: cell.into(),
                iface: self.project_iface("core", cell),
                source: devices::by_core(cell).and_then(|d| d.source),
                slib,
                ..Master::default()
            };
        }
        let iface = match self.find(lib, cell) {
            // The dump's interface, with what only the project's mirror knows: the params
            // its rules compute and the ones Virtuoso derives (§8.3).
            Some(k) => self.ifaces[k].clone().map(|mut i| {
                if let Some(p) = self.project_iface(&slib, cell) {
                    (i.computed, i.vderived, i.vinputs) = (p.computed, p.vderived, p.vinputs);
                }
                i
            }),
            None => self.project_iface(&slib, cell),
        };
        Master {
            dir: self.dir_of(&slib),
            slib,
            stem: cell.into(),
            iface,
            ..Master::default()
        }
    }

    fn project_iface(&self, slib: &str, stem: &str) -> Option<Iface> {
        if slib == "core" {
            let f = format!("{stem}.shprim");
            return schemify_doc::core_library()
                .iter()
                .find(|(n, _)| *n == f)
                .and_then(|(_, b)| Ctx::iface_of_text(b));
        }
        let dir = self.dir_of(slib);
        ["shcomp", "shprim"].iter().find_map(|ext| {
            let p = cell_path(&dir, stem, ext);
            self.existing.get(&p).and_then(|b| Ctx::iface_of_text(b))
        })
    }
}

/// A resolved instance master.
#[derive(Clone, Debug, Default)]
struct Master {
    slib: String,
    dir: String,
    stem: String,
    iface: Option<Iface>,
    /// `map` row (reverse lookup).
    row: Option<usize>,
    /// analogLib → core re-mastering: device row, waveform.
    alib: Option<(usize, usize)>,
    /// A core source: current (true) or voltage.
    source: Option<bool>,
}

impl Master {
    fn cellref(&self, from_dir: &str) -> Option<String> {
        if self.slib == "core" {
            return Some(format!("core:{}", self.stem));
        }
        if self.dir == from_dir {
            Some(format!("./{}", self.stem))
        } else if !self.slib.is_empty() {
            Some(format!("{}:{}", self.slib, self.stem))
        } else {
            None
        }
    }
}

/// The orientation of a dump record; the reader checked it ([`dump`]), so `R0` stands
/// only for a record built in memory.
fn orient_in(s: &str) -> Orient {
    profile::orient_from_virtuoso(s).unwrap_or(Orient::R0)
}

/// The net text and globality of a `schemifyNet` value (the inverse of [`net_prop`]).
fn net_of_prop(v: &str) -> (String, bool) {
    match v.strip_suffix(" global") {
        Some(n) => (n.to_owned(), true),
        None => (v.to_owned(), false),
    }
}

/// Converts a dump point, reporting a rounded conversion.
fn pt(xy: [i32; 2], upi: i32, what: &str, cell: &str, notes: &mut Notes) -> [i32; 2] {
    match point_from_virtuoso(xy[0], xy[1], upi) {
        Some(c) => {
            if !c.exact {
                notes.add(
                    "rounding",
                    cell,
                    format!("{what} at ({}, {})", xy[0], xy[1]),
                );
            }
            c.p
        }
        None => {
            notes.review("name", cell, format!("{what}: coordinate out of range"));
            [0, 0]
        }
    }
}

/// A dump box (`x1 y1 x2 y2`, either corner first) converted (roundings reported as `what`)
/// and normalized to `[min x, min y, max x, max y]`.
///
/// # Panics
///
/// When `b` holds fewer than 4 coordinates.
fn box_in(b: &[i32], upi: i32, what: &str, cell: &str, notes: &mut Notes) -> [i32; 4] {
    assert!(b.len() >= 4, "a box has 4 coordinates");
    let p = pt([b[0], b[1]], upi, what, cell, notes);
    let q = pt([b[2], b[3]], upi, what, cell, notes);
    profile::norm_box(p, q)
}

/// The centre of a box (exact through doubling).
fn centre(b: [i32; 4], upi: i32, what: &str, cell: &str, notes: &mut Notes) -> [i32; 2] {
    let x = i64::from(b[0]) + i64::from(b[2]);
    let y = i64::from(b[1]) + i64::from(b[3]);
    match (i32::try_from(x), i32::try_from(y), upi.checked_mul(2)) {
        (Ok(x), Ok(y), Some(u2)) => pt([x, y], u2, what, cell, notes),
        _ => pt([b[0], b[1]], upi, what, cell, notes),
    }
}

/// The symbol pins of a dump cell (§6.2 symbol pin row): one pin per terminal at its first
/// pin rectangle, further rectangles as alias pins, in spectre `termOrder` else sorted.
fn pins_in(c: &DCell, sym: &Cv, notes: &mut Notes) -> Vec<PinRow> {
    let cell = format!("{}/{}", c.lib, c.name);
    let order: Vec<String> = c
        .cdf
        .as_ref()
        .and_then(|d| d.sim("spectre"))
        .map(|s| s.list("termOrder"))
        .unwrap_or_default();
    let mut terms: Vec<&Term> = sym.terms.iter().collect();
    let all_listed = !order.is_empty() && terms.iter().all(|t| order.contains(&t.name));
    if all_listed {
        terms.sort_by_key(|t| order.iter().position(|o| *o == t.name).expect("listed"));
    } else {
        terms.sort_by(|a, b| a.name.cmp(&b.name));
    }
    let mut out = Vec::new();
    for t in terms {
        let net = net_from_virtuoso(&t.name);
        if net.review {
            notes.review(
                "name",
                &cell,
                format!("terminal `{}` kept verbatim", t.name),
            );
        }
        let dir = profile::dir_from_virtuoso(&t.dir);
        let inherit = parse_netexpr(&t.netexpr).map(|(_, d)| d);
        let figs: Vec<&PinFig> = sym.pinfigs.iter().filter(|f| f.term == t.name).collect();
        let main = figs
            .iter()
            .find(|f| sym.prop(f.r, "schemifyPin").is_none())
            .or(figs.first());
        let at = main.map_or([0, 0], |f| {
            centre(f.b, sym.upi, &format!("pin {}", t.name), &cell, notes)
        });
        out.push(PinRow {
            name: net.text.clone(),
            dir: dir.into(),
            at,
            inherit,
            alias: None,
        });
        let mut n = 1;
        for f in &figs {
            if main.is_some_and(|m| m.r == f.r) {
                continue;
            }
            n += 1;
            let name = sym
                .prop(f.r, "schemifyPin")
                .map_or(format!("{}_{n}", net.text), str::to_owned);
            out.push(PinRow {
                name,
                dir: dir.into(),
                at: centre(f.b, sym.upi, &format!("pin {}", t.name), &cell, notes),
                inherit: None,
                alias: Some(net.text.clone()),
            });
        }
    }
    out
}

/// `[@p:%:d!]` → `(p, d)` with the default's `!` dropped (`gnd!` → `GND`, §3.3).
fn parse_netexpr(e: &str) -> Option<(String, String)> {
    let inner = e.trim().strip_prefix("[@")?.strip_suffix(']')?;
    let mut parts = inner.splitn(3, ':');
    let p = parts.next()?;
    let fmt = parts.next()?;
    let d = parts.next()?;
    if p.is_empty() || fmt != "%" || d.is_empty() {
        return None;
    }
    let n = net_from_virtuoso(d);
    let def = if n.global {
        n.text
    } else {
        n.text.trim_end_matches('!').to_owned()
    };
    Some((p.to_owned(), def))
}

/// One imported param.
#[derive(Clone, Debug)]
struct ParamIn {
    name: String,
    kind: String,
    default: String,
    unit: Option<&'static str>,
    doc: Option<String>,
    choices: Vec<String>,
}

/// The §6.3 import table over a cell's CDF.
fn params_in(c: &DCell, cdf: &Cdf, notes: &mut Notes) -> Vec<ParamIn> {
    let cell = format!("{}/{}", c.lib, c.name);
    let names: Vec<&str> = cdf.params.iter().map(|p| p.name.as_str()).collect();
    let mut out = Vec::new();
    for p in &cdf.params {
        if !p.callback.trim().is_empty() {
            notes.review(
                "callback",
                &cell,
                format!("param {}: callback not carried", p.name),
            );
        }
        let (kind, default) = match p.ty.as_str() {
            "button" | "netSet" => continue,
            "int" => (
                "int",
                if p.def.trim().parse::<i64>().is_ok() {
                    p.def.trim().to_owned()
                } else {
                    "0".into()
                },
            ),
            "float" => (
                "number",
                own_default(&p.def, &names).unwrap_or_else(|| "0".into()),
            ),
            "boolean" => (
                "bool",
                ael::bool_from_cdf(&p.def).unwrap_or("false").to_owned(),
            ),
            "radio" | "cyclic" => ("choice", ael::literal_to_template(&p.def)),
            _ => {
                let numeric = p.flags.iter().any(|f| f == "parseAsNumber");
                match own_default(&p.def, &names).filter(|_| numeric) {
                    Some(d) => ("number", d),
                    None => ("string", ael::literal_to_template(&p.def)),
                }
            }
        };
        out.push(ParamIn {
            name: p.name.clone(),
            kind: kind.into(),
            default,
            unit: ael::unit_from_cdf(&p.units),
            doc: (!p.prompt.is_empty() && p.prompt != p.name).then(|| p.prompt.clone()),
            choices: p.choices.clone(),
        });
    }
    out
}

/// A CDF default as a Schemify expression when it is numeric: translates, compiles, and
/// names only params of the same cell (`iPar("x")` → `x`).
fn own_default(def: &str, names: &[&str]) -> Option<String> {
    if def.trim().is_empty() {
        return None;
    }
    let t = ael::from_ael(
        def,
        |n| {
            if names.contains(&n) {
                AelName::Param
            } else {
                AelName::Unknown
            }
        },
        |n| names.contains(&n).then(|| n.to_owned()),
    )?;
    if t.review.is_some() {
        return None;
    }
    let p = schemify_expr::Program::compile(&t.text, schemify_expr::Mode::Scalar).ok()?;
    p.idents()
        .all(|i| names.contains(&i))
        .then(|| si_text(&t.text))
}

/// A number literal as engineering text (`150.00n` → `150n`, `900.0m`, `0.9` → `900m`); any
/// other text unchanged. Virtuoso reformats the numbers of parseAsNumber CDF parameters
/// (live ICADVM20.1 run: `0.15u` came back `150.00n`), so both sides of the stash
/// comparison (§7.3) read them through this.
pub fn si_text(t: &str) -> String {
    let Ok(x) = schemify_expr::parse_num(t.trim()) else {
        return t.to_owned();
    };
    if x == 0.0 || !x.is_finite() {
        return if x == 0.0 { "0".into() } else { t.to_owned() };
    }
    // Shortest round-trip mantissa digits and decimal exponent of |x|.
    let e = format!("{:e}", x.abs());
    let Some((m, exp)) = e.split_once('e') else {
        return t.to_owned();
    };
    let Ok(exp) = exp.parse::<i32>() else {
        return t.to_owned();
    };
    let digits: String = m.chars().filter(char::is_ascii_digit).collect();
    let eng = exp.div_euclid(3) * 3;
    let suffix = match eng {
        -15 => "f",
        -12 => "p",
        -9 => "n",
        -6 => "u",
        -3 => "m",
        0 => "",
        3 => "k",
        6 => "meg",
        9 => "g",
        12 => "t",
        _ => return t.to_owned(),
    };
    // The point goes after 1 + (exp - eng) digits.
    let int_len = usize::try_from(exp - eng + 1).unwrap_or(1);
    let mut d = digits;
    while d.len() < int_len {
        d.push('0');
    }
    let (i, f) = d.split_at(int_len);
    let f = f.trim_end_matches('0');
    let sign = if x < 0.0 { "-" } else { "" };
    if f.is_empty() {
        format!("{sign}{i}{suffix}")
    } else {
        format!("{sign}{i}.{f}{suffix}")
    }
}

// ---------------------------------------------------------------------------------------
// Ids (§2, §7.2)
// ---------------------------------------------------------------------------------------

/// Id assignment for one file: `schemifyId` when valid and unused, else
/// `IdRng::for_import("<lib>/<cell>", ordinal)`.
struct Ids {
    seed: String,
    taken: BTreeSet<String>,
    ordinal: u32,
}

impl Ids {
    fn new(seed: String) -> Ids {
        Ids {
            seed,
            taken: BTreeSet::new(),
            ordinal: 0,
        }
    }

    /// The id of the next object (in dump order), from `wanted` when possible.
    fn next(&mut self, wanted: Option<&str>, reserved: &BTreeSet<String>) -> String {
        self.ordinal += 1;
        if let Some(w) = wanted
            && is_id(w)
            && self.taken.insert(w.to_owned())
        {
            return w.to_owned();
        }
        let mut rng = IdRng::for_import(&self.seed, self.ordinal);
        let id = fresh_id(&mut rng, |c| self.taken.contains(c) || reserved.contains(c));
        let s = id.as_str().to_owned();
        self.taken.insert(s.clone());
        s
    }
}

// ---------------------------------------------------------------------------------------
// Entry points
// ---------------------------------------------------------------------------------------

/// The fresh documents of every written dump cell (after `ade::import`), their stash
/// texts and whether a maestro setup came with them.
struct Fresh {
    docs: Vec<Option<(Document<'static>, Option<String>, bool)>>,
}

fn fresh(ctx: &Ctx<'_>, notes: &mut Notes) -> Fresh {
    let mut docs = Vec::new();
    let mut pending: Vec<Vec<Pending>> = Vec::new();
    for k in 0..ctx.dump.cells.len() {
        let mut p = Vec::new();
        if !ctx.targets[k].write {
            docs.push(None);
            pending.push(p);
            continue;
        }
        let (mut doc, st) = build(ctx, k, &mut p, notes);
        pending.push(p);
        let c = &ctx.dump.cells[k];
        let mut maestro = false;
        for test in &ctx.dump.maestro {
            let m = &test[0];
            if m.str(1).ok() == Some(c.lib.as_str()) && m.str(2).ok() == Some(c.name.as_str()) {
                maestro = true;
                ade::import(test, &mut doc, &c.name, notes);
            }
        }
        docs.push(Some((doc, st, maestro)));
    }
    if pending.iter().any(|p| !p.is_empty()) {
        remaster(ctx, &mut docs, &pending, notes);
    }
    Fresh { docs }
}

/// The Schemify-side re-mastering pass (§6.4): loads the fresh documents (with the
/// project's files and the core library), pulls wires back from the analogLib mirror
/// pins, labels them, and moves each pending instance onto its core cell with stubs and
/// labels at the core pins.
fn remaster(
    ctx: &Ctx<'_>,
    docs: &mut [Option<(Document<'static>, Option<String>, bool)>],
    pending: &[Vec<Pending>],
    notes: &mut Notes,
) {
    use crate::design::{geom, load_closed, sym_pins};
    let mut files: Files = ctx.existing.clone();
    for (k, d) in docs.iter().enumerate() {
        if let Some((doc, _, _)) = d {
            files.insert(
                ctx.targets[k].path(),
                schemify_format::write(doc).into_bytes(),
            );
        }
    }
    let mut manifest = Document::new(FileKind::Project);
    for (l, d) in &ctx.libs {
        push_lib(&mut manifest, l, d);
    }
    for t in ctx
        .targets
        .iter()
        .filter(|t| t.write && !t.slib.is_empty() && !ctx.libs.contains_key(&t.slib))
    {
        if (1..manifest.record_count()).all(|r| manifest.field(r, 0) != Some(t.slib.as_str())) {
            push_lib(&mut manifest, &t.slib, &t.dir);
        }
    }
    files.insert(
        "schemify.project".into(),
        schemify_format::write(&manifest).into_bytes(),
    );
    for (k, todo) in pending.iter().enumerate() {
        if todo.is_empty() {
            continue;
        }
        let path = ctx.targets[k].path();
        let d = match load_closed(&files, &[path.as_str()]) {
            Ok(d) => d,
            Err(p) => {
                let why = format!("re-mastering skipped: {}", p.join("; "));
                notes.review("name", &path, why);
                continue;
            }
        };
        let ci = d.find(&path).expect("loaded");
        let g = geom(&d, ci);
        let n = crate::virtuoso::nets::extract(&d, ci, &g);
        let mut rem = Vec::new();
        let mut masters = Vec::new();
        for p in todo {
            let Some(inst) = g.inst_id.iter().position(|x| *x == p.id) else {
                continue;
            };
            let core = format!("{}.shprim", devices::DEVICES[p.device].core);
            let new_pins = schemify_doc::core_library()
                .iter()
                .find(|(nm, _)| *nm == core)
                .and_then(|(_, b)| Ctx::iface_of_text(b))
                .map(|i| {
                    i.pins
                        .into_iter()
                        .map(|q| (q.name, q.at))
                        .collect::<Vec<_>>()
                });
            let old_pins = sym_pins(&d.cells[g.inst_master[inst] as usize]);
            let new_names: Vec<String> = new_pins
                .as_ref()
                .map_or(Vec::new(), |v| v.iter().map(|x| x.0.clone()).collect());
            let pairs: Vec<(usize, String)> = (0..old_pins.name.len())
                .filter_map(|q| new_names.get(q).map(|nm| (q, nm.clone())))
                .collect();
            // Pin-pattern alignment (§6.4): the core cell goes where its pins land on the
            // Virtuoso master's, in the instance's orientation.
            let at = new_pins.as_ref().and_then(|np| {
                let (o, at) = (g.inst_orient[inst], g.inst_at[inst]);
                crate::virtuoso::align::fit_instance(&old_pins.at, &pairs, np, o, at)
            });
            rem.push(crate::virtuoso::remap::Remaster {
                inst,
                pins: pairs,
                new_pins,
                at,
            });
            masters.push((
                inst,
                p.cellref.clone(),
                Some(
                    p.attrs
                        .iter()
                        .map(|(a, v)| (a.clone(), vec![v.clone()]))
                        .collect(),
                ),
            ));
        }
        let e = crate::virtuoso::remap::apply(&g, &n, &rem);
        let (doc, st, m) = docs[k].take().expect("pending cells are built");
        let mut taken: BTreeSet<String> = (1..doc.record_count())
            .filter(|&r| doc.record_kind(r).key() == schemify_format::KeyRule::Id)
            .filter_map(|r| doc.field(r, 0).map(str::to_owned))
            .collect();
        let seed = format!("{}/{}/remap", ctx.dump.cells[k].lib, ctx.dump.cells[k].name);
        let mut ord = 0u32;
        let new = crate::virtuoso::remap::apply_to_doc(&d.cells[ci].doc, &g, &e, &masters, || {
            ord += 1;
            let mut rng = IdRng::for_import(&seed, ord);
            let id = fresh_id(&mut rng, |c| taken.contains(c))
                .as_str()
                .to_owned();
            taken.insert(id.clone());
            id
        });
        docs[k] = Some((new, st, m));
    }
}

/// See [`super::import_targets`].
pub fn targets(sfyx: &str) -> Result<Vec<String>, Vec<String>> {
    let recs = sfyx::read(sfyx).map_err(|e| vec![e.to_string()])?;
    let d = dump(&recs).map_err(|e| vec![e.to_string()])?;
    let opts = ImportOptions::default();
    let mut out: BTreeSet<String> =
        ["schemify.project".to_owned(), "interop.map".to_owned()].into();
    for c in &d.cells {
        let slib = slib_of(&c.lib, &opts);
        if slib == "core" || consumed(&c.lib, &c.name) {
            continue;
        }
        for ext in ["shcomp", "shprim", "shbench"] {
            out.insert(cell_path(&slib, &c.name, ext));
        }
    }
    Ok(out.into_iter().collect())
}

/// See [`super::import`].
pub fn import(
    sfyx: &str,
    existing: &Files,
    map: &Mapping,
    opts: &ImportOptions,
) -> Result<Import, Vec<String>> {
    let recs = sfyx::read(sfyx).map_err(|e| vec![e.to_string()])?;
    let d = dump(&recs).map_err(|e| vec![e.to_string()])?;
    let mut notes = Notes::default();
    for r in &d.unknown {
        notes.dropped(
            "unknown-record",
            "-",
            format!("`{}` on line {}", r.tag, r.line),
        );
    }
    let ctx = Ctx::new(&d, existing, map, opts, &mut notes);
    let fr = fresh(&ctx, &mut notes);
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    let mut new_libs: BTreeMap<String, String> = BTreeMap::new();
    let manifest = existing
        .get("schemify.project")
        .and_then(|m| crate::design::manifest_libs(m).ok())
        .unwrap_or_default();
    for (k, f) in fr.docs.into_iter().enumerate() {
        let Some((doc, st, maestro)) = f else {
            continue;
        };
        let t = &ctx.targets[k];
        let path = t.path();
        let cell = path.clone();
        let local = existing
            .get(&path)
            .and_then(|b| merge::parse_stash(std::str::from_utf8(b).ok()?, doc.kind()).ok());
        let stash = st.and_then(|s| match merge::parse_stash(&s, doc.kind()) {
            Ok(d) => Some(d),
            Err(e) => {
                notes.review("format", &cell, format!("stash ignored: {e}"));
                None
            }
        });
        let out = merge::merge(
            &doc,
            local.as_ref(),
            stash.as_ref(),
            maestro,
            &cell,
            &mut notes,
        );
        let out = with_fingerprints(&out, &doc, ctx.dump.cells[k].cdf.as_ref());
        let text = schemify_format::write(&out);
        if let Some(old) = existing.get(&path)
            && old.as_slice() != text.as_bytes()
        {
            notes.add("updated", &cell, "the import changed this file");
        }
        if !t.slib.is_empty() && !manifest.contains_key(&t.slib) {
            new_libs.insert(t.slib.clone(), t.dir.clone());
        }
        files.push((path, text.into_bytes()));
    }
    if !new_libs.is_empty() {
        files.push((
            "schemify.project".into(),
            manifest_with(existing.get("schemify.project"), &new_libs).into_bytes(),
        ));
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    assert!(
        files.windows(2).all(|w| w[0].0 < w[1].0),
        "one file per path"
    );
    Ok(Import { files, notes })
}

/// `doc` (the merged file) with header attributes `vsym`/`vsch` set to the fingerprints of
/// the parts its kind has, taken from `fresh` (what the dump holds, before local data was
/// merged, §7.5): an export can then tell a cellview nobody changed on the Schemify side.
fn with_fingerprints(
    doc: &Document<'static>,
    fresh: &Document<'static>,
    cdf: Option<&Cdf>,
) -> Document<'static> {
    use super::{Part, fingerprint, fingerprint_attr};
    let parts: &[Part] = match doc.kind() {
        FileKind::Shprim => &[Part::Symbol],
        FileKind::Shcomp => &[Part::Symbol, Part::Schematic, Part::Interface],
        _ => &[Part::Schematic],
    };
    let mut out = Document::new(doc.kind());
    copy_header(&mut out, doc, &["vsym", "vsch", "vcdf", "vcdfrec"]);
    for p in parts {
        out.push_attr(fingerprint_attr(*p));
        out.push_value(&fingerprint(fresh, *p));
    }
    // The reverse stash of the CDF (§7.5): a component's CDF records as Virtuoso had them,
    // for export to send back while `vcdf` still matches (none: the cell had no CDF).
    if doc.kind() == FileKind::Shcomp {
        for r in cdf.map(cdf_records).unwrap_or_default() {
            let mut s = String::new();
            sfyx::write_record(&r, &mut s);
            out.push_attr("vcdfrec");
            out.push_value(s.trim_end());
        }
    }
    for r in 1..doc.record_count() {
        out.copy_record(doc, r);
    }
    assert_eq!(out.record_count(), doc.record_count());
    out
}

/// The manifest with `lib <name> path <dir>` rows added for `new`.
fn manifest_with(old: Option<&Vec<u8>>, new: &BTreeMap<String, String>) -> String {
    let mut doc = match old.and_then(|b| parse(b).ok()) {
        Some(src) => crate::design::owned_doc(&src),
        None => Document::new(FileKind::Project),
    };
    for (l, d) in new {
        push_lib(&mut doc, l, d);
    }
    schemify_format::write(&doc)
}

/// The fresh documents an import of `records` produces (no merge), keyed by (Virtuoso
/// library, cell): the export's stash oracle. Libraries map back through the inverse of
/// the export's renames; masters outside the dump resolve through `d`'s files.
pub(crate) fn image(
    records: &[Record],
    plan: &crate::virtuoso::plan::Plan,
    d: &crate::design::Design,
    map: &Mapping,
) -> BTreeMap<(String, String), Document<'static>> {
    let dump = dump(records).expect("the exporter writes valid records");
    let mut iopts = ImportOptions::default();
    for (ci, c) in d.cells.iter().enumerate() {
        if plan.how[ci] == crate::virtuoso::plan::How::Design {
            iopts.lib_map.insert(plan.lib[ci].clone(), c.lib.clone());
        }
    }
    iopts.lib_dir = d.libs.clone();
    let mut files: Files = Files::new();
    let mut manifest = Document::new(FileKind::Project);
    for (l, dir) in &d.libs {
        push_lib(&mut manifest, l, dir);
    }
    files.insert(
        "schemify.project".into(),
        schemify_format::write(&manifest).into_bytes(),
    );
    for c in &d.cells {
        if c.lib != "core" {
            files.insert(c.path.clone(), c.text().into_bytes());
        }
    }
    let mut notes = Notes::default();
    let ctx = Ctx::new(&dump, &files, map, &iopts, &mut notes);
    let fr = fresh(&ctx, &mut notes);
    let mut out = BTreeMap::new();
    for (k, f) in fr.docs.into_iter().enumerate() {
        if let Some((doc, _, _)) = f {
            out.insert((dump.cells[k].lib.clone(), dump.cells[k].name.clone()), doc);
        }
    }
    out
}

/// A segment with its ends in sorted order, the key two drawings of one wire share.
fn seg_key(a: [i32; 2], b: [i32; 2]) -> [[i32; 2]; 2] {
    if b < a { [b, a] } else { [a, b] }
}

/// The wires of a stash document: segment → id.
fn stash_wires(d: &Document<'_>) -> BTreeMap<[[i32; 2]; 2], String> {
    let mut out = BTreeMap::new();
    for r in 1..d.record_count() {
        if d.record_kind(r) != RecordKind::Wire {
            continue;
        }
        let c = |k| {
            d.field(r, k)
                .and_then(|f| schemify_format::parse_coord(f).ok())
        };
        if let (Some(id), Some(x0), Some(y0), Some(x1), Some(y1)) =
            (d.field(r, 0), c(1), c(2), c(3), c(4))
        {
            out.insert(seg_key([x0, y0], [x1, y1]), id.to_owned());
        }
    }
    out
}

/// Virtuoso merges collinear wires that touch and splits them again where a wire T-joins,
/// handing the pieces' schemifyIds out in its own order (live ICADVM20.1 run: two pieces came
/// back with their ids exchanged). Where wires drawn exactly as stashed carry each other's
/// stashed ids, the ids go back to their stashed segments. `drawn`: (ref, segment, id).
/// Returns ref → id for the wires to re-id.
// ponytail: all-or-nothing, applied only when the re-ids are a pure permutation of the ids
// they replace; a partial match (a wire also moved in Virtuoso) keeps Virtuoso's ids.
fn unswap_wire_ids(
    drawn: &[(u32, [[i32; 2]; 2], &str)],
    stashed: &BTreeMap<[[i32; 2]; 2], String>,
) -> BTreeMap<u32, String> {
    let moved: Vec<(u32, &str, &str)> = drawn
        .iter()
        .filter_map(|(r, s, id)| {
            let want = stashed.get(s)?;
            (want != id).then_some((*r, *id, want.as_str()))
        })
        .collect();
    let from: BTreeSet<&str> = moved.iter().map(|m| m.1).collect();
    let to: BTreeSet<&str> = moved.iter().map(|m| m.2).collect();
    if from != to || from.len() != moved.len() {
        return BTreeMap::new();
    }
    let out: BTreeMap<u32, String> = moved.iter().map(|m| (m.0, m.2.to_owned())).collect();
    debug_assert!(out.len() == from.len());
    out
}

/// The main pin figure of symbol pin `name`: its terminal's first rectangle without a
/// `schemifyPin` prop (those are alias pins, §6.2).
fn main_pin_fig<'a>(sv: &'a Cv, name: &str) -> Option<&'a PinFig> {
    let alias_fig = |r: u32| sv.props.iter().any(|p| p.r == r && p.name == "schemifyPin");
    let fig = sv
        .pinfigs
        .iter()
        .find(|f| net_from_virtuoso(&f.term).text == name && !alias_fig(f.r));
    debug_assert!(fig.is_none_or(|f| f.dir.is_empty()), "a symbol pin figure");
    fig
}

/// The `vplabel` values of symbol pin `name` at `at` (milli-units): `none` when its main
/// pin figure has no pin label, the label's offset from the pin and its orient, justify,
/// font and height when they differ from export's (at the pin, R0, centerLeft, stick, half
/// LABEL_HEIGHT), else `None`.
fn pin_label_as_drawn(
    sv: &Cv,
    name: &str,
    at: [i32; 2],
    cell: &str,
    notes: &mut Notes,
) -> Option<Vec<String>> {
    let fig = main_pin_fig(sv, name)?;
    // The label glued to the pin's rectangle, else an unglued pin label of its terminal's
    // name (Cadence's sample library leaves them unglued; live run).
    let Some(l) = sv
        .labels
        .iter()
        .find(|l| l.parent == fig.r && l.layer == "pin")
        .or_else(|| {
            sv.labels
                .iter()
                .find(|l| l.parent == 0 && l.layer == "pin" && l.text == fig.term)
        })
    else {
        return Some(vec!["none".into()]);
    };
    let xy = pt(l.xy, sv.upi, "pin label", cell, notes);
    let d = [xy[0] - at[0], xy[1] - at[1]];
    let h = profile::height_from_virtuoso(l.height, sv.upi);
    let o = orient_in(&l.orient);
    let usual = i64::from(super::write::LABEL_HEIGHT / 2);
    if d == [0, 0]
        && o == Orient::R0
        && (l.justify.as_str(), l.font.as_str(), h) == ("centerLeft", "stick", usual)
    {
        return None;
    }
    Some(vec![
        coord_text(d[0]),
        coord_text(d[1]),
        o.as_str().into(),
        l.justify.clone(),
        l.font.clone(),
        h.to_string(),
    ])
}

/// A dumped wire for [`rejoin_split_wires`]: (ref, points, its schemifyId when valid).
type DumpWire<'a> = (u32, Vec<[i32; 2]>, Option<&'a str>);

/// Virtuoso splits a wire where another wire T-joins it (`schCreateWire "draw"`, live
/// ICADVM20.1 run 2026-10-15; GUESS G5 said it would not), and the extra piece comes back
/// without the id or with a copy of it. Each such piece of a two-point wire is folded into the
/// collinear two-point wire it touches that owns the id, so the wire reads back as drawn.
/// Returns (owner ref → its rejoined two points, refs folded away). `wires`: (ref, points,
/// schemifyId) in dump order, stubs excluded.
fn rejoin_split_wires(wires: &[DumpWire<'_>]) -> (BTreeMap<u32, Vec<[i32; 2]>>, BTreeSet<u32>) {
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    let mut owners: Vec<(u32, [[i32; 2]; 2])> = Vec::new();
    let mut pieces: Vec<(u32, [[i32; 2]; 2])> = Vec::new();
    for (r, pts, id) in wires {
        let [a, b] = pts[..] else { continue };
        if a == b {
            continue;
        }
        match id {
            Some(i) if seen.insert(i) => owners.push((*r, [a, b])),
            _ => pieces.push((*r, [a, b])),
        }
    }
    let collinear = |s: [[i32; 2]; 2], p: [i32; 2]| {
        let d = [i64::from(s[1][0] - s[0][0]), i64::from(s[1][1] - s[0][1])];
        let v = [i64::from(p[0] - s[0][0]), i64::from(p[1] - s[0][1])];
        d[0] * v[1] - d[1] * v[0] == 0
    };
    let mut absorbed = BTreeSet::new();
    let mut changed = true;
    while changed {
        changed = false;
        for (pr, ps) in &pieces {
            if absorbed.contains(pr) {
                continue;
            }
            let hit = owners.iter_mut().find(|(_, os)| {
                collinear(*os, ps[0])
                    && collinear(*os, ps[1])
                    && (os.contains(&ps[0]) || os.contains(&ps[1]))
            });
            if let Some((_, os)) = hit {
                let mut all = [os[0], os[1], ps[0], ps[1]];
                all.sort_unstable();
                *os = [all[0], all[3]];
                absorbed.insert(*pr);
                changed = true;
            }
        }
    }
    let rejoined = owners
        .into_iter()
        .filter(|(r, s)| wires.iter().any(|(wr, pts, _)| wr == r && pts[..] != s[..]))
        .map(|(r, s)| (r, s.to_vec()))
        .collect();
    (rejoined, absorbed)
}

#[cfg(test)]
mod tests {
    use super::super::write::{
        cdl_siminfo, spectre_siminfo, template_items, template_to_nlp, urad,
    };
    use super::{
        BTreeMap, DCell, Notes, ParamIn, PinRow, SimInfo, SimVal, arc_angles, cdl_format,
        net_from_virtuoso, net_of_prop, net_prop, nlp_in, seg_key, si_text, spectre_format,
        unswap_wire_ids,
    };
    use proptest::prelude::*;

    fn pin(name: &str) -> PinRow {
        PinRow {
            name: name.into(),
            dir: "inout".into(),
            ..PinRow::default()
        }
    }

    fn param(name: &str, string: bool) -> ParamIn {
        ParamIn {
            name: name.into(),
            kind: if string { "string" } else { "number" }.into(),
            default: "1".into(),
            unit: None,
            doc: None,
            choices: Vec::new(),
        }
    }

    /// A simInfo's netlisting fields in the order export writes them: `head`, then
    /// `termOrder`, `instParameters` and `propMapping` (only for renamed parameters).
    fn siminfo(
        sim: &str,
        head: Vec<(String, SimVal)>,
        pins: &[String],
        kc: &[(String, String)],
    ) -> SimInfo {
        let mut fields = head;
        fields.push(("termOrder".into(), SimVal::List(pins.to_vec())));
        let ks = kc.iter().map(|x| x.0.clone()).collect();
        fields.push(("instParameters".into(), SimVal::List(ks)));
        let pm: Vec<(String, String)> = kc.iter().filter(|x| x.0 != x.1).cloned().collect();
        if !pm.is_empty() {
            fields.push(("propMapping".into(), SimVal::Pairs(pm)));
        }
        SimInfo {
            sim: sim.into(),
            fields,
        }
    }

    /// Distinct pin names (Virtuoso spelling, buses included), and CDF parameters
    /// `(simulator name k, CDF name c, string)`.
    fn netlisting() -> impl Strategy<Value = (Vec<String>, Vec<(String, String, bool)>)> {
        (
            prop::collection::btree_set("p[a-z]{0,2}(<[0-9]>)?", 0..4),
            prop::collection::btree_map("q[a-z]{0,2}", (any::<bool>(), any::<bool>()), 0..4),
        )
            .prop_map(|(pins, params)| {
                let ps = params
                    .into_iter()
                    .map(|(c, (renamed, string))| {
                        let k = if renamed { format!("k{c}") } else { c.clone() };
                        (k, c, string)
                    })
                    .collect();
                (pins.into_iter().collect(), ps)
            })
    }

    proptest! {
        /// Import's spectre format of a simInfo reads back, through export's reverse rule, as
        /// the same simInfo (§6.3): master, terminal order, parameters, renames, and the
        /// optional form of string parameters.
        #[test]
        fn spectre_siminfo_round_trips_through_the_format(
            (pins, ps) in netlisting(),
            master in "[A-Za-z][A-Za-z0-9_]{0,5}",
        ) {
            let kc: Vec<(String, String)> = ps.iter().map(|x| (x.0.clone(), x.1.clone())).collect();
            let head = vec![("componentName".to_owned(), SimVal::Str(master.clone()))];
            let si = siminfo("spectre", head, &pins, &kc);
            let names: Vec<String> = pins.iter().map(|p| net_from_virtuoso(p).text).collect();
            let rows: Vec<PinRow> = names.iter().map(|p| pin(p)).collect();
            let params: Vec<ParamIn> = ps.iter().map(|x| param(&x.1, x.2)).collect();
            let t = spectre_format(&DCell::default(), &si, &params, &rows);
            let kinds: Vec<(String, String)> =
                params.iter().map(|p| (p.name.clone(), p.kind.clone())).collect();
            prop_assert_eq!(spectre_siminfo(&t, &names, &kinds), Some(si), "via {}", t);
        }

        /// The same for auCdl's subcircuit call (`X{name} {pins} / comp k={c}…`).
        #[test]
        fn cdl_subckt_siminfo_round_trips_through_the_format(
            (pins, ps) in netlisting(),
            comp in "[A-Za-z][A-Za-z0-9_]{0,5}",
            prefix in "[A-Z]{1,2}",
        ) {
            let kc: Vec<(String, String)> = ps.iter().map(|x| (x.0.clone(), x.1.clone())).collect();
            let head = vec![
                ("netlistProcedure".to_owned(), SimVal::Str("ansCdlSubcktCall".into())),
                ("componentName".to_owned(), SimVal::Str(comp)),
                ("namePrefix".to_owned(), SimVal::Str(prefix)),
            ];
            let si = siminfo("auCdl", head, &pins, &kc);
            let names: Vec<String> = pins.iter().map(|p| net_from_virtuoso(p).text).collect();
            let rows: Vec<PinRow> = names.iter().map(|p| pin(p)).collect();
            let params: Vec<ParamIn> = ps.iter().map(|x| param(&x.1, x.2)).collect();
            let t = cdl_format(&DCell::default(), &si, &params, &rows, &mut Notes::default());
            let kinds: Vec<(String, String)> =
                params.iter().map(|p| (p.name.clone(), p.kind.clone())).collect();
            prop_assert_eq!(cdl_siminfo(&t, &names, &kinds), Some(si), "via {}", t);
        }

        /// A Schemify arc's `(start, sweep)` (milli-degrees, screen clockwise) exported as
        /// Virtuoso's counter-clockwise micro-radian `(start, stop)` reads back exactly, start
        /// normalized into a turn: a micro-radian is finer than half a milli-degree.
        #[test]
        fn arc_angles_round_trip(start in -720_000i32..720_000, sweep in 1i32..360_000) {
            let deg = |m: i32| f64::from(m) / 1000.0;
            // Export (`write::graphic_out`): y flips, so the angles negate and swap ends.
            let a0 = urad(-(deg(start) + deg(sweep)));
            let a1 = urad(-deg(start));
            prop_assert_eq!(arc_angles(a0, a1), (start.rem_euclid(360_000), sweep));
        }

        /// `schemifyNet` reads back as written for any net text without a space.
        #[test]
        fn net_props_round_trip(net in "[A-Za-z0-9_!<>:,*\\[\\]]{1,12}", global: bool) {
            prop_assert_eq!(net_of_prop(&net_prop(&net, global)), (net, global));
        }
    }

    /// A label template from literals and holes, braces of the literals doubled.
    fn template(lits: &[String], holes: &[&str]) -> String {
        let mut t = lits[0].replace('{', "{{").replace('}', "}}");
        for (h, l) in holes.iter().zip(&lits[1..]) {
            t.push_str(&format!("{{{h}}}"));
            t.push_str(&l.replace('{', "{{").replace('}', "}}"));
        }
        t
    }

    proptest! {
        /// Export's NLP text of a symbol label reads back as the same template (§3.5),
        /// whatever NLP-reserved characters (`\ [ ] : %`) and braces the literals hold.
        #[test]
        fn label_templates_round_trip_through_nlp(
            (holes, lits) in (1usize..4).prop_flat_map(|n| (
                prop::collection::vec(prop::sample::select(vec!["w", "l", "m", "nf"]), n),
                prop::collection::vec("[a-z %:\\[\\]\\\\{}=/]{0,5}", n + 1),
            )),
        ) {
            let t = template(&lits, &holes);
            let items = template_items(&t).expect("balanced");
            let nlp = template_to_nlp(&items, &["w", "l", "m", "nf"]).expect("every hole a param");
            prop_assert_eq!(nlp_in(&nlp, ""), Some(t.clone()), "{} via {}", t, nlp);
        }
    }

    proptest! {
        /// Respelling a number keeps its value (to rounding of the suffix scale), whatever
        /// its magnitude and sign: both sides of the stash comparison read numbers through
        /// `si_text`, so a respelling that moved the value would hide an edit.
        #[test]
        fn engineering_text_keeps_the_value(m in 1u32..10_000_000, e in -21i32..17, neg: bool) {
            let t = format!("{}{m}e{e}", if neg { "-" } else { "" });
            let x = schemify_expr::parse_num(&t).unwrap();
            let back = si_text(&t);
            let y = schemify_expr::parse_num(&back)
                .unwrap_or_else(|_| panic!("{t} became unreadable {back}"));
            prop_assert!((x - y).abs() <= x.abs() * 1e-14, "{} = {} became {} = {}", t, x, back, y);
        }
    }

    #[test]
    fn an_escaped_percent_is_text() {
        // `\%` is a literal percent (NLP); only the first bare `%` is the value.
        assert_eq!(nlp_in("[@w:50\\%=%]", "").as_deref(), Some("50%={w}"));
        assert_eq!(nlp_in("[@w:a\\%b%c]", "").as_deref(), Some("a%b{w}c"));
        assert_eq!(nlp_in("[@w:%:fallback]", "").as_deref(), Some("{w}"));
        assert_eq!(nlp_in("[@w:no value]", "").as_deref(), Some("no value{w}"));
        assert_eq!(nlp_in("[@refDes]", ""), None);
        assert_eq!(nlp_in("[@w:a:b:c]", ""), None);
        assert_eq!(nlp_in("[@w", ""), None);
    }

    #[test]
    fn swapped_wire_ids_go_back_only_as_a_permutation() {
        let (lo, hi) = (
            seg_key([320, 230], [320, 250]),
            seg_key([320, 250], [320, 270]),
        );
        let stashed: BTreeMap<_, _> = [(lo, "w13".to_owned()), (hi, "w14".to_owned())].into();
        // The ota run: the two pieces came back with each other's ids.
        let got = unswap_wire_ids(&[(1, hi, "w13"), (2, lo, "w14")], &stashed);
        assert_eq!(got, [(1, "w14".to_owned()), (2, "w13".to_owned())].into());
        // As stashed: nothing to do.
        assert!(unswap_wire_ids(&[(1, lo, "w13"), (2, hi, "w14")], &stashed).is_empty());
        // One piece moved in Virtuoso as well: not a permutation, Virtuoso's ids stay.
        let moved = seg_key([0, 0], [0, 10]);
        assert!(unswap_wire_ids(&[(1, hi, "w13"), (2, moved, "w14")], &stashed).is_empty());
    }

    #[test]
    fn numbers_read_as_engineering_text() {
        for (a, b) in [
            ("150.00n", "150n"),
            ("0.15u", "150n"),
            ("900.0m", "900m"),
            ("0.9", "900m"),
            ("1", "1"),
            ("2u", "2u"),
            ("1.8", "1.8"),
            ("-2.5meg", "-2.5meg"),
            ("10k", "10k"),
            ("0", "0"),
            ("W/2", "W/2"),
            ("1e-12", "1p"),
        ] {
            assert_eq!(si_text(a), b, "{a}");
        }
    }
}
