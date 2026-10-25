//! xschem libraries → Schemify libraries (spec `import.md` §1–§7). Pure: bytes in,
//! bytes out, no file system.
//!
//! Spine ([`translate_library`]), one line per transform:
//! 1. `read_sources`: every `.sym`/`.sch` → [`XschemFile`] tables, sorted by path.
//! 2. `classify_symbols`: symbol attributes → class (cell, label, port, code, …).
//! 3. `plan_symbols`: cell symbols → interface plans (pins, params, impls, limits);
//!    `devices/` symbols listed in [`devices::DEVICE_MAP`] → `core` targets instead.
//! 4. `plan_cells`: (symbol, schematic) pairs → `.shprim`/`.shcomp`/`.shbench` cells,
//!    plus alternate-schematic cells requested by instances.
//! 5. `name_libs`: cell directories → library names.
//! 6. `emit_cell` per cell → canonical bytes.
//! 7. manifest + report, with count conservation asserted per input file.
//!
//! Data (six questions): in = N files of KB each (corpus: 3,342 files, 60 MB); out =
//! one cell file per (symbol, schematic) pair. Tables are per-kind `Vec`s indexed by
//! `u32` rows (sources, symbols, cells < 2^16 in any real library). Everything lives
//! for one call. The transform is embarrassingly parallel per cell, but at ~0.3 s for
//! the whole corpus it stays sequential.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use schemify_format::{
    Document, FileKind, IdRng, Orient, RecordKind, fresh_id, parse_coord, write,
};

use crate::design::place;
use crate::pdk::cdf::{CdfDump, CdfType};
use crate::pdk::spice::{self, ModelScan};
use crate::xschem::attr::{Attrs, at_tokens};
use crate::xschem::devices::{self, DeviceMap};
use crate::xschem::file::{self, GlobalTag, NumField, XschemFile, orient};
use crate::xschem::limits;
use crate::xschem::report::{Class, Drop, Item, Report};
use crate::xschem::symbol::{self, PinDir, SymbolInterface};
use crate::xschem::tcl::{self, DrcProc};
use crate::xschem::template::{
    self, ExprMode, FormatCtx, Reason, Review, is_ident, literal, review, spice_number,
    string_value,
};

mod cells;
mod emit;
mod interface;

/// Instance attributes that are netlist text of their own: an instance that sets one to
/// other text than its symbol gets a variant cell (spec §3).
const INSTANCE_TEXT: [&str; 3] = ["format", "lvs_format", "spice_sym_def"];

/// Most input files one call accepts (rows are `u32`, report items index files).
pub const MAX_FILES: usize = 1 << 20;

/// Translation knobs. `Default` maps `devices/` to `core` and has no PDK data.
#[derive(Clone, Copy, Debug)]
pub struct Options<'a> {
    /// The xschem library search path (`XSCHEM_LIBRARY_PATH`), as input-path prefixes
    /// in priority order. A symbol reference `r` resolves to the first `<root>/r` among
    /// the inputs, then to `r` next to the referencing schematic.
    pub roots: &'a [String],
    /// PDK path variables from the PDK `xschemrc` ([`tcl::pdk_vars`]): `$VAR` in
    /// netlist text becomes `${VAR}`, and each becomes a project `set` entry.
    pub pdk_vars: &'a [(String, String)],
    /// Map xschem `devices/*.sym` to the bundled `core` library (spec §6).
    pub map_core: bool,
    /// Model-card scans for L/W bin limits.
    pub models: &'a [ModelScan],
    /// Recognised `drc=` procs ([`tcl::drc_procs`]).
    pub drc: &'a [DrcProc],
    /// Manufacturing grid in picometres (tech file).
    pub grid_pm: Option<u32>,
    /// A Cadence `cdfDump`: radio/cyclic choices for same-named cells and params.
    pub cdf: Option<&'a CdfDump>,
}

impl Default for Options<'_> {
    fn default() -> Self {
        Options {
            roots: &[],
            pdk_vars: &[],
            map_core: true,
            models: &[],
            drc: &[],
            grid_pm: None,
            cdf: None,
        }
    }
}

/// Translates xschem libraries into Schemify files.
///
/// `inputs` are `(path, bytes)` with `/`-separated paths; `.sym` and `.sch` are
/// translated, in path order whatever the input order; any other input is a side file
/// read only where netlist text names it (a `spice_sym_def=".include f.cir"` gives the
/// `.subckt` port order, spec §2). Returns the output files as
/// `(path, bytes)` sorted by path — `<lib>/<cell>.<ext>`, `schemify.project`,
/// and the [`Report`] (the job writes it with the other rows of the conversion).
///
/// Deterministic: the same inputs and options give byte-identical outputs (ids come
/// from [`IdRng::for_import`] seeded with `<lib>/<file>` and the record's line).
///
/// # Panics
///
/// Panics if count conservation fails for any file (an importer bug) or if there are
/// more than [`MAX_FILES`] inputs.
pub fn translate_library(
    inputs: &[(&str, &[u8])],
    opts: &Options,
) -> (Vec<(String, Vec<u8>)>, Report) {
    assert!(inputs.len() <= MAX_FILES, "too many input files");
    let (mut t, mut acc) = Tables::read_sources(inputs, opts);
    t.classify_symbols();
    t.scan_overrides();
    t.plan_symbols(&mut acc);
    t.plan_cells(&mut acc);
    t.plan_variants(&mut acc);
    t.name_libs();
    let mut files: Vec<(String, Vec<u8>)> = Vec::with_capacity(t.cells.len() + 3);
    for c in 0..t.cells.len() {
        files.push(t.emit_cell(c, &mut acc));
    }
    files.push(("schemify.project".to_owned(), t.manifest().into_bytes()));
    let report = t.finish(acc);
    report.assert_conservation();
    files.sort_by(|a, b| a.0.cmp(&b.0));
    assert!(
        files.windows(2).all(|w| w[0].0 != w[1].0),
        "output paths are unique"
    );
    (files, report)
}

// ------------------------------------------------------------------------- tables

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ext {
    Sym,
    Sch,
}

/// One input file.
struct Src {
    path: String,
    ext: Ext,
    file: XschemFile,
    /// `file_version` 1.0 (or absent): symbol references carry no extension.
    v10: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SymClass {
    Cell,
    Label,
    Port,
    Noconn,
    /// Netlist text: `netlist_commands`, `spice_parameters`, pinless symbols with a format.
    Code,
    Drop(Drop),
    Review(Reason),
}

struct Sym {
    src: u32,
    iface: SymbolInterface,
    ty: String,
    class: SymClass,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Number,
    Bool,
    Str,
    Choice,
}

impl Kind {
    fn word(self) -> Option<&'static str> {
        match self {
            Kind::Number => None,
            Kind::Bool => Some("bool"),
            Kind::Str => Some("string"),
            Kind::Choice => Some("choice"),
        }
    }
}

struct PinOut {
    /// Schemify name (`xs` plus `_pin` when a parameter has the same name).
    name: String,
    /// xschem name.
    xs: String,
    dir: &'static str,
    xy: [i32; 2],
    inherit: Option<String>,
    /// A second pin of an earlier pin's xschem name: written `alias <that pin>` (xschem
    /// shorts same-named pins; netlist spec §3).
    alias: bool,
}

struct ParamOut {
    name: String,
    default: String,
    kind: Kind,
    choices: Vec<String>,
    attrs: Vec<(&'static str, String)>,
}

struct ImplOut {
    name: &'static str,
    lang: &'static str,
    formats: Vec<(&'static str, String)>,
    ignore: Option<String>,
    body: Vec<String>,
}

/// A cell symbol's translated interface.
struct Plan {
    pins: Vec<PinOut>,
    params: Vec<ParamOut>,
    /// Leaf impl (always present; ignored when the cell is a pure subcircuit).
    leaf: ImplOut,
    /// Formats for a schematic impl (subcircuit-style conversion).
    schematic_formats: Vec<(&'static str, String)>,
    subcircuit: bool,
    /// The symbol has a `spice_sym_def`: xschem netlists it by that text, so the leaf impl
    /// is the default even with a schematic.
    sym_def: bool,
    /// `spice_ignore` of the symbol: applies to every impl.
    ignore: Option<String>,
    prefix: Option<String>,
    spiceprefix: Option<String>,
}

/// What instances need to know about a target cell.
#[derive(Clone, Debug, Default)]
struct Iface {
    pins: Vec<(String, [i32; 2])>,
    implicit: Vec<(String, String)>,
    params: Vec<(String, String, Kind)>,
}

#[derive(Clone, Copy)]
enum Target {
    /// A converted cell (row in `cells`).
    Cell(u32),
    /// A core cell.
    Core(&'static DeviceMap),
}

struct Cell {
    dir: String,
    name: String,
    kind: FileKind,
    sym: Option<u32>,
    sch: Option<u32>,
    /// The symbol's records are counted by this cell (not by an alternate cell).
    tally_sym: bool,
    /// The schematic's records are counted by this cell (not shared or alternate).
    tally_sch: bool,
    /// A bench copy of component `copy_of`'s schematic (its netlist text needs a bench).
    copy_of: Option<u32>,
    lib: u16,
}

/// Records each input record's fate (spec §7 count conservation).
struct Acc {
    out: Vec<u32>,
    items: Vec<Item>,
    unused: BTreeMap<String, u32>,
    device_instances: u32,
    core_instances: u32,
    /// False while re-translating a schematic for an alternate cell.
    active: bool,
}

impl Acc {
    fn out(&mut self, file: u32, n: u32) {
        assert!(
            (file as usize) < self.out.len(),
            "file row {file} out of range"
        );
        if self.active {
            self.out[file as usize] += n;
        }
    }

    fn item(
        &mut self,
        file: u32,
        line: u32,
        class: Class,
        records: u32,
        detail: impl Into<String>,
    ) {
        if self.active {
            self.items.push(Item {
                file,
                line,
                class,
                records,
                detail: detail.into(),
            });
        }
    }
}

struct Tables<'o> {
    opts: &'o Options<'o>,
    srcs: Vec<Src>,
    index: HashMap<String, u32>,
    syms: Vec<Sym>,
    sym_of_src: Vec<Option<u32>>,
    plans: Vec<Option<Plan>>,
    sym_ifaces: Vec<Option<Iface>>,
    targets: Vec<Option<Target>>,
    core_ifaces: HashMap<&'static str, Iface>,
    /// (symbol, key) pairs some instance sets to text that is not a number expression
    /// (`value="port=1 z0=50 ac 1"`): such parameters are strings whatever their default.
    textual: HashSet<(u32, String)>,
    cells: Vec<Cell>,
    /// (base cell, alternate schematic src) → alternate cell.
    alt_cells: HashMap<(u32, u32), u32>,
    /// (schematic src, instance row) → its variant cell (instance netlist text, spec §3).
    variants: HashMap<(u32, u32), u32>,
    /// Inputs that are neither `.sym` nor `.sch`, by normalised path: files netlist text
    /// includes (`spice_sym_def=".include f.cir"`).
    side: HashMap<String, String>,
    lib_names: Vec<String>,
}

/// Normalises a `/` path lexically (`.`/`..`/empty components).
pub(crate) fn normalize(path: &str) -> String {
    let abs = path.starts_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for c in path.split('/') {
        match c {
            "" | "." => {}
            ".." if parts.last().is_some_and(|p| *p != "..") => {
                parts.pop();
            }
            ".." if abs => {}
            c => parts.push(c),
        }
    }
    let j = parts.join("/");
    if abs { format!("/{j}") } else { j }
}

pub(crate) fn dir_of(p: &str) -> &str {
    p.rsplit_once('/').map_or("", |(d, _)| d)
}

fn file_of(p: &str) -> &str {
    p.rsplit_once('/').map_or(p, |(_, f)| f)
}

fn stem_of(p: &str) -> &str {
    let f = file_of(p);
    f.rsplit_once('.').map_or(f, |(s, _)| s)
}

pub(crate) fn join(dir: &str, rel: &str) -> String {
    if dir.is_empty() || rel.starts_with('/') {
        normalize(rel)
    } else {
        normalize(&format!("{dir}/{rel}"))
    }
}

fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truthy(v: &str) -> Option<bool> {
    match v.trim().to_ascii_lowercase().as_str() {
        "true" | "on" | "yes" | "1" => Some(true),
        "false" | "off" | "no" | "0" | "" => Some(false),
        _ => None,
    }
}

/// Parses a core cell into the interface instances need.
fn iface_of_cell_bytes(bytes: &[u8]) -> Iface {
    let d = schemify_format::parse(bytes).expect("core cells parse");
    let mut f = Iface::default();
    for r in 1..d.record_count() {
        let field = |i| d.field(r, i).unwrap_or("");
        match d.record_kind(r) {
            RecordKind::Pin => {
                let inherit = d.find_attr(r, "inherit").and_then(|a| d.attr_value(a, 0));
                match inherit {
                    Some(net) => f.implicit.push((field(0).to_owned(), net.to_owned())),
                    None => f.pins.push((
                        field(0).to_owned(),
                        [
                            parse_coord(field(2)).unwrap_or(0),
                            parse_coord(field(3)).unwrap_or(0),
                        ],
                    )),
                }
            }
            RecordKind::Param => {
                let kind = match d.find_attr(r, "kind").and_then(|a| d.attr_value(a, 0)) {
                    Some("bool") => Kind::Bool,
                    Some("string") => Kind::Str,
                    Some("choice") => Kind::Choice,
                    _ => Kind::Number,
                };
                f.params
                    .push((field(0).to_owned(), field(1).to_owned(), kind));
            }
            _ => {}
        }
    }
    assert!(!f.pins.is_empty(), "core cells have pins");
    f
}

impl<'o> Tables<'o> {
    // ------------------------------------------------------------------ 1. read

    fn read_sources(inputs: &[(&str, &[u8])], opts: &'o Options<'o>) -> (Tables<'o>, Acc) {
        let (mut sorted, others): (Vec<_>, Vec<_>) = inputs
            .iter()
            .map(|(p, b)| (normalize(p), *b))
            .partition(|(p, _)| p.ends_with(".sym") || p.ends_with(".sch"));
        let side: HashMap<String, String> = others
            .into_iter()
            .map(|(p, b)| (p, String::from_utf8_lossy(b).into_owned()))
            .collect();
        sorted.sort_by(|a, b| a.0.cmp(&b.0));
        sorted.dedup_by(|a, b| a.0 == b.0);
        let mut srcs = Vec::with_capacity(sorted.len());
        let mut items = Vec::new();
        for (path, bytes) in sorted {
            let text = String::from_utf8_lossy(bytes);
            let file = match file::read(&text) {
                Ok(f) => f,
                Err(e) => {
                    items.push(Item {
                        file: srcs.len() as u32,
                        line: 0,
                        class: Class::Dropped(Drop::Malformed),
                        records: 0,
                        detail: format!("not read: {e:?}"),
                    });
                    XschemFile::default()
                }
            };
            let version = file.global(GlobalTag::Version).unwrap_or("");
            let v10 = !version.contains("file_version=") || version.contains("file_version=1.0");
            let ext = if path.ends_with(".sym") {
                Ext::Sym
            } else {
                Ext::Sch
            };
            srcs.push(Src {
                path,
                ext,
                file,
                v10,
            });
        }
        let index: HashMap<String, u32> = srcs
            .iter()
            .enumerate()
            .map(|(i, s)| (s.path.clone(), i as u32))
            .collect();
        assert!(
            srcs.windows(2).all(|w| w[0].path < w[1].path),
            "sources sorted and unique"
        );
        assert_eq!(index.len(), srcs.len());
        let n = srcs.len();
        let acc = Acc {
            out: vec![0; n],
            items,
            unused: BTreeMap::new(),
            device_instances: 0,
            core_instances: 0,
            active: true,
        };
        let t = Tables {
            opts,
            srcs,
            index,
            syms: Vec::new(),
            sym_of_src: vec![None; n],
            plans: Vec::new(),
            sym_ifaces: Vec::new(),
            targets: Vec::new(),
            core_ifaces: HashMap::new(),
            textual: HashSet::new(),
            cells: Vec::new(),
            alt_cells: HashMap::new(),
            variants: HashMap::new(),
            side,
            lib_names: Vec::new(),
        };
        (t, acc)
    }

    /// Resolves a symbol or schematic reference made from source `from`.
    fn resolve(&self, from: u32, symref: &str) -> Option<u32> {
        assert!((from as usize) < self.srcs.len());
        let mut r = symref.trim().to_owned();
        if r.is_empty() || r.contains('@') || r.contains("tcleval(") || r.contains('(') {
            return None;
        }
        if self.srcs[from as usize].v10 && !r.ends_with(".sym") && !r.ends_with(".sch") {
            r.push_str(".sym");
        }
        let here = dir_of(&self.srcs[from as usize].path);
        let look = |p: String| self.index.get(&p).copied();
        if r.starts_with('/') {
            return look(normalize(&r));
        }
        if r.starts_with("./") || r.starts_with("../") {
            return look(join(here, &r));
        }
        self.opts
            .roots
            .iter()
            .find_map(|root| look(join(&normalize(root), &r)))
            .or_else(|| look(join(here, &r)))
    }

    // ------------------------------------------------------------------ 7. outputs

    fn manifest(&self) -> String {
        let mut d = Document::new(FileKind::Project);
        for name in &self.lib_names {
            d.begin_record(RecordKind::Lib);
            d.push_field(name);
            d.push_field("path");
            d.push_field(name);
        }
        for (k, v) in self.opts.pdk_vars {
            d.begin_record(RecordKind::Set);
            d.push_field(k);
            d.push_field(v);
        }
        write(&d)
    }

    /// `path` relative to the shallowest search-path root that contains it (as given when
    /// none does), so a report reads the same on every machine (D32): the roots are
    /// where the checkout or PDK sits.
    fn report_path(&self, path: &str) -> String {
        let root = self
            .opts
            .roots
            .iter()
            .map(|r| normalize(r))
            .filter(|r| {
                !r.is_empty()
                    && path.len() > r.len()
                    && path.starts_with(r.as_str())
                    && path.as_bytes()[r.len()] == b'/'
            })
            .min_by_key(String::len);
        let rel = root.as_ref().map_or(path, |r| &path[r.len() + 1..]);
        assert!(root.is_none() || !rel.starts_with('/'));
        rel.to_owned()
    }

    fn finish(&self, mut acc: Acc) -> Report {
        // Symbols that became no cell: their records are carried by the records
        // their instances became (labels, ports, code) or by the core cell.
        for (si, sym) in self.syms.iter().enumerate() {
            let fid = sym.src as usize;
            if self.sym_of_src[fid] != Some(si as u32) {
                continue; // a variant's copy of a symbol: the canonical row reports it
            }
            if self.plans[si].is_some() {
                continue;
            }
            let records = records_in(&self.srcs[fid].file);
            let drop = if matches!(
                self.targets[self.sym_of_src[fid].expect("row") as usize],
                Some(Target::Core(_))
            ) {
                Drop::MappedToCore
            } else {
                Drop::NotACell
            };
            acc.item(
                sym.src,
                0,
                Class::Dropped(drop),
                records - acc.out[fid],
                format!("type={}", sym.ty),
            );
        }
        Report {
            files: self
                .srcs
                .iter()
                .map(|s| self.report_path(&s.path))
                .collect(),
            records_in: self.srcs.iter().map(|s| records_in(&s.file)).collect(),
            records_out: acc.out,
            items: acc.items,
            unused_attrs: acc.unused,
            device_instances: acc.device_instances,
            core_instances: acc.core_instances,
        }
    }
}

/// xschem's default number of layers (`cadlayers`); objects on higher layers are not
/// loaded by a default xschem, so they are dropped and reported.
const CAD_LAYERS: u32 = 22;

type ImplRow<'p> = (
    &'static str,
    &'static str,
    &'p [(&'static str, String)],
    Option<&'p str>,
);

/// Per-document id generator: `IdRng::for_import(seed, line·8 + sub)`, redrawn on
/// collision within the document.
struct Ids {
    seed: String,
    taken: HashSet<String>,
}

impl Ids {
    fn next(&mut self, line: u32, sub: u32) -> String {
        let mut rng = IdRng::for_import(&self.seed, line.wrapping_mul(8).wrapping_add(sub));
        let id = fresh_id(&mut rng, |c| self.taken.contains(c))
            .as_str()
            .to_owned();
        let fresh = self.taken.insert(id.clone());
        assert!(fresh);
        id
    }
}

fn records_in(f: &XschemFile) -> u32 {
    f.records_read + f.embedded.iter().map(|e| e.body.records_read).sum::<u32>()
}

/// Tallies the records every file kind treats alike: header records, comments,
/// malformed records, embedded symbols and roundings.
fn tally_common(f: &XschemFile, fid: u32, acc: &mut Acc, schematic: bool) {
    let g = &f.globals;
    for i in 0..g.tag.len() {
        let text = f.str(g.text[i]).trim();
        let other = match g.tag[i] {
            GlobalTag::Version | GlobalTag::K => false,
            GlobalTag::G => schematic, // VHDL text in a schematic; symbol attributes in a symbol
            GlobalTag::S => continue,  // the schematic pass decides; in a symbol: below
            GlobalTag::V | GlobalTag::E => true,
        };
        if other && !text.is_empty() {
            acc.item(
                fid,
                g.line[i],
                Class::Dropped(Drop::OtherNetlister),
                1,
                text,
            );
        } else {
            acc.out(fid, 1);
        }
    }
    if !schematic {
        for i in (0..g.tag.len()).filter(|&i| g.tag[i] == GlobalTag::S) {
            let text = f.str(g.text[i]).trim();
            if text.is_empty() {
                acc.out(fid, 1);
            } else {
                acc.item(
                    fid,
                    g.line[i],
                    Class::Dropped(Drop::OtherNetlister),
                    1,
                    text,
                );
            }
        }
    }
    acc.out(fid, f.comments);
    for is in &f.issues {
        acc.item(
            fid,
            is.line,
            Class::Dropped(Drop::Malformed),
            1,
            format!("{:?} ({})", is.kind, is.tag as char),
        );
    }
    for e in &f.embedded {
        acc.item(
            fid,
            e.line,
            Class::Dropped(Drop::Embedded),
            1 + e.body.records_read,
            "embedded symbol body",
        );
    }
    for r in &f.roundings {
        let what = match r.field {
            NumField::Coord => "coordinate",
            NumField::Scale => "text scale",
            NumField::Angle => "arc angle",
        };
        acc.item(
            fid,
            r.line,
            Class::Rounding,
            0,
            format!(
                "{what} col {} → {} milli (error {}/1000 milli)",
                r.col, r.milli, r.error_micro
            ),
        );
    }
}

fn push_layer(doc: &mut Document, layer: u8) {
    doc.push_attr("layer");
    doc.push_value(&layer.to_string());
}

fn emit_interface(doc: &mut Document, plan: &Plan, impls: &[ImplRow]) {
    for p in &plan.pins {
        doc.begin_record(RecordKind::Pin);
        doc.push_field(&p.name);
        doc.push_field(p.dir);
        doc.push_field_coord(p.xy[0]);
        doc.push_field_coord(p.xy[1]);
        if let Some(net) = &p.inherit {
            doc.push_attr("inherit");
            doc.push_value(net);
        }
        if let Some(target) = p
            .alias
            .then(|| plan.pins.iter().find(|q| !q.alias && q.xs == p.xs))
            .flatten()
        {
            doc.push_attr("alias");
            doc.push_value(&target.name);
        }
    }
    for p in &plan.params {
        doc.begin_record(RecordKind::Param);
        doc.push_field(&p.name);
        doc.push_field(&p.default);
        if let Some(k) = p.kind.word() {
            doc.push_attr("kind");
            doc.push_value(k);
        }
        if !p.choices.is_empty() {
            doc.push_attr("choices");
            p.choices.iter().for_each(|c| doc.push_value(c));
        }
        for (k, v) in &p.attrs {
            doc.push_attr(k);
            doc.push_value(v);
        }
    }
    for (name, lang, formats, ignore) in impls {
        doc.begin_record(RecordKind::Impl);
        doc.push_field(name);
        doc.push_field(lang);
        for (dialect, tpl) in formats.iter() {
            doc.push_attr("format");
            doc.push_value(dialect);
            doc.push_value(tpl);
        }
        if let Some(v) = ignore {
            doc.push_attr("ignore");
            doc.push_value(v);
        }
    }
}

/// A symbol text → `stext` template, or `None` for display-only text (OP annotation,
/// Tcl, pin attributes). Tokens that are not parameters, `name` or `cell` render
/// empty, as in xschem.
fn stext_template(text: &str, params: &[String], spiceprefix: Option<&str>) -> Option<String> {
    let display = [
        "tcleval(",
        "@spice_get",
        "@#",
        "ngspice::",
        "@path",
        "@schname",
    ];
    if text.trim().is_empty() || display.iter().any(|d| text.contains(d)) {
        return None;
    }
    let ctx = FormatCtx {
        params,
        pins: &[],
        implicit: &[],
        spiceprefix: Some(spiceprefix.unwrap_or("")),
        schematic: false,
        rename: &[],
    };
    let c = template::convert_format(text, &ctx).ok()?;
    let mut t = c.text;
    for u in c.unknown.iter().chain(["pins".to_owned()].iter()) {
        t = t.replace(&format!("{{{u}}}"), "");
    }
    let parsed = schemify_expr::Template::parse(&t).ok()?;
    let ok = parsed
        .program()
        .idents()
        .all(|i| i == "name" || i == "cell" || params.iter().any(|p| p == i));
    (ok && !t.trim().is_empty()).then_some(t)
}

/// An instance override value → the text stored for a parameter of `kind`.
fn override_text(
    v: &str,
    kind: Kind,
    known: &[String],
    parents: &[String],
) -> Result<String, Review> {
    let t = v.trim();
    let fail = |reason| review(reason, t);
    if t.contains("tcleval(") {
        return Err(fail(Reason::Tcl));
    }
    match kind {
        Kind::Number => {
            if let Some(n) = spice_number(t) {
                return Ok(n);
            }
            let inner = t
                .strip_prefix('\'')
                .and_then(|x| x.strip_suffix('\''))
                .or_else(|| t.strip_prefix('{').and_then(|x| x.strip_suffix('}')))
                .unwrap_or(t);
            let e = template::translate_expr(inner, ExprMode::Spice, known)
                .ok_or_else(|| fail(Reason::Expression))?;
            debug_assert!(schemify_expr::Program::compile(&e, schemify_expr::Mode::Scalar).is_ok());
            Ok(e)
        }
        Kind::Bool => truthy(t)
            .map(|b| b.to_string())
            .ok_or_else(|| fail(Reason::Expression)),
        Kind::Str | Kind::Choice => Ok(string_value(v, parents)),
    }
}

/// Moves closed `.param name=value …` lines (outside `.control`) into bench variables
/// when every value translates and only references names defined the same way.
fn extract_params(
    text: &mut Vec<String>,
    vars: &mut Vec<(String, String)>,
    defaults: &mut Vec<String>,
) {
    let mut keep = Vec::with_capacity(text.len());
    let mut control = false;
    for (k, l) in text.iter().enumerate() {
        let low = l.trim().to_ascii_lowercase();
        if low.starts_with(".control") {
            control = true;
        } else if low.starts_with(".endc") {
            control = false;
        }
        let continued = text
            .get(k + 1)
            .is_some_and(|n| n.trim_start().starts_with('+'));
        if control || continued || !low.starts_with(".param ") {
            keep.push(l.clone());
            continue;
        }
        // Component defaults give way to the bench's own definition.
        let replaced: Vec<(String, String)> = vars
            .iter()
            .filter(|v| !defaults.iter().any(|d| d.eq_ignore_ascii_case(&v.0)))
            .cloned()
            .collect();
        match param_pairs(l.trim()[6..].trim(), &replaced, vars) {
            Some(pairs) => {
                vars.retain(|v| !pairs.iter().any(|p| p.0.eq_ignore_ascii_case(&v.0)));
                defaults.retain(|d| !pairs.iter().any(|p| p.0.eq_ignore_ascii_case(d)));
                vars.extend(pairs);
            }
            None => keep.push(l.clone()),
        }
    }
    assert!(
        (1..vars.len()).all(|i| !vars[..i].iter().any(|v| v.0 == vars[i].0)),
        "bench variables are unique"
    );
    *text = keep;
}

/// The `k=v` pairs of one `.param` line, values translated; `None` unless every value
/// translates and names only `known` variables or earlier pairs. A name in `fixed`
/// (or repeated in the line) cannot be redefined.
fn param_pairs(
    s: &str,
    fixed: &[(String, String)],
    known: &[(String, String)],
) -> Option<Vec<(String, String)>> {
    let words = spice::words(s);
    let mut out: Vec<(String, String)> = Vec::with_capacity(words.len() / 3);
    for w in words.chunks(3) {
        let [name, "=", value] = w else { return None };
        if !is_ident(name) || !name.starts_with(|c: char| c.is_ascii_alphabetic()) {
            return None;
        }
        let value = value
            .strip_prefix('\'')
            .and_then(|v| v.strip_suffix('\''))
            .or_else(|| value.strip_prefix('{').and_then(|v| v.strip_suffix('}')))
            .unwrap_or(value);
        out.push(((*name).to_owned(), value.to_owned()));
    }
    let mut names: Vec<String> = known.iter().map(|v| v.0.clone()).collect();
    names.extend(out.iter().map(|p| p.0.clone()));
    let mut res = Vec::with_capacity(out.len());
    for (n, v) in out {
        if fixed.iter().any(|x| x.0.eq_ignore_ascii_case(&n))
            || res
                .iter()
                .any(|x: &(String, String)| x.0.eq_ignore_ascii_case(&n))
        {
            return None;
        }
        let e = template::translate_expr(&v, ExprMode::Spice, &names)?;
        let p = schemify_expr::Program::compile(&e, schemify_expr::Mode::Scalar).ok()?;
        if !p.idents().all(|id| names.iter().any(|n| n == id)) {
            return None;
        }
        res.push((n, e));
    }
    (!res.is_empty()).then_some(res)
}

/// The inherited pins of a symbol (spec §2): `extra=` nets in `extra` order, then the
/// `@body` idiom, each an identifier with a template value that `format` or `lvs_format`
/// uses as a standalone `@x` word, with its default net (`0` when the value is empty).
/// Goldens append extra pins to `.subckt` lines in that order.
fn implicit_nets(iface: &SymbolInterface) -> Vec<(String, String)> {
    let (attrs, tpl) = (&iface.attrs, &iface.template);
    let (fmt, lvs) = (attrs.get("format"), attrs.get("lvs_format"));
    let standalone = |f: Option<&str>, k: &str| {
        f.is_some_and(|f| f.split_whitespace().any(|w| w.strip_prefix('@') == Some(k)))
    };
    let mut implicit: Vec<(String, String)> = Vec::new();
    for k in attrs
        .get("extra")
        .unwrap_or("")
        .split_whitespace()
        .chain(["body"])
    {
        let Some(net) = tpl.get(k) else { continue };
        let net_param = standalone(fmt, k) || standalone(lvs, k);
        if net_param && is_ident(k) && !implicit.iter().any(|(n, _)| n == k) {
            let net = net.trim();
            let net = if net.is_empty() { "0" } else { net };
            implicit.push((k.to_owned(), net.to_owned()));
        }
    }
    assert!(implicit.iter().all(|(k, n)| !k.is_empty() && !n.is_empty()));
    implicit
}

/// Whether instance `row` of `f` sets `only_toplevel=true`.
fn top_only(f: &XschemFile, row: usize) -> bool {
    Attrs::parse(f.str(f.instances.props[row]))
        .get("only_toplevel")
        .and_then(truthy)
        == Some(true)
}

/// xschem's bracket-less bus form in a net or instance name: `A[3..0]` is the nets
/// `A3,A2,A1,A0` and `A[6..0..2]` steps by 2 (research: xschem-feature-inventory, bus
/// syntax; goldens name `G2[9..0]`'s devices `G29`…`G20`), as a Schemify bundle. Any
/// other text is returned unchanged.
pub(crate) fn dotted_bus(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(open) = rest.find('[') {
        let Some(close) = rest[open..].find(']').map(|c| open + c) else {
            break;
        };
        let base_start = rest[..open]
            .rfind(|c: char| c == ',' || c == '*' || c == ']' || c.is_whitespace())
            .map_or(0, |i| i + 1);
        let nums: Vec<i64> = rest[open + 1..close]
            .split("..")
            .map(|n| n.trim().parse::<i64>())
            .collect::<Result<_, _>>()
            .unwrap_or_default();
        let (from, to, step) = match nums[..] {
            [a, b] => (a, b, 1),
            [a, b, c] if c > 0 => (a, b, c),
            _ => {
                out.push_str(&rest[..=close]);
                rest = &rest[close + 1..];
                continue;
            }
        };
        let base = &rest[base_start..open];
        out.push_str(&rest[..base_start]);
        let dir = if to >= from { 1 } else { -1 };
        let count = ((to - from).abs() / step + 1).min(65_536);
        let bits: Vec<String> = (0..count)
            .map(|k| format!("{base}{}", from + dir * k * step))
            .collect();
        out.push_str(&bits.join(","));
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    out
}

fn unique<'a>(base: &str, taken: impl Iterator<Item = &'a str> + Clone) -> String {
    let mut name = base.to_owned();
    let mut k = 2;
    while taken.clone().any(|t| t == name) {
        name = format!("{base}_{k}");
        k += 1;
    }
    name
}

/// Appends `s` with `\x` → `x` (attribute-level escapes in netlist text).
fn push_unescaped(out: &mut String, s: &str) {
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(n) = chars.next() {
                out.push(n);
            }
        } else {
            out.push(c);
        }
    }
}

fn iface_of_plan(p: &Plan) -> Iface {
    Iface {
        pins: p
            .pins
            .iter()
            .filter(|q| q.inherit.is_none())
            .map(|q| (q.name.clone(), q.xy))
            .collect(),
        implicit: p
            .pins
            .iter()
            .filter_map(|q| q.inherit.as_ref().map(|n| (q.name.clone(), n.clone())))
            .collect(),
        params: p
            .params
            .iter()
            .map(|q| (q.name.clone(), q.default.clone(), q.kind))
            .collect(),
    }
}

/// Infers a parameter's kind and default text from its xschem template value.
fn infer(v: &str, avail: &[&str], keys: &[String]) -> (Kind, String, Option<Review>) {
    let t = v.trim();
    if t.eq_ignore_ascii_case("true") || t.eq_ignore_ascii_case("false") {
        return (Kind::Bool, t.to_ascii_lowercase(), None);
    }
    if let Some(n) = spice_number(t) {
        return (Kind::Number, n, None);
    }
    if let Some(body) = t
        .strip_prefix("expr(")
        .or_else(|| t.strip_prefix("expr_eng("))
        .or_else(|| t.strip_prefix("expr_eng4("))
        .and_then(|b| b.strip_suffix(')'))
    {
        let body = body.trim();
        let body = body
            .strip_prefix('\'')
            .and_then(|b| b.strip_suffix('\''))
            .unwrap_or(body);
        return match template::translate_expr(body, ExprMode::Xschem, keys) {
            Some(e) => (Kind::Number, e, None),
            None => (
                Kind::Str,
                String::new(),
                Some(review(Reason::Expression, t.to_owned())),
            ),
        };
    }
    if t.starts_with("tcleval(") {
        return (
            Kind::Str,
            String::new(),
            Some(review(Reason::Tcl, t.to_owned())),
        );
    }
    if !avail.is_empty() && avail.contains(&t) {
        return (Kind::Choice, t.to_owned(), None);
    }
    (Kind::Str, literal(t), None)
}

fn apply_cdf(cdf: &CdfDump, cell: &str, params: &mut [ParamOut], reviews: &mut Vec<Review>) {
    let Some(c) = (0..cdf.cells.cell.len()).find(|&c| cdf.str(cdf.cells.cell[c]) == cell) else {
        return;
    };
    for p in (0..cdf.params.name.len()).filter(|&p| cdf.params.cell[p] as usize == c) {
        let name = cdf.str(cdf.params.name[p]);
        let Some(out) = params.iter_mut().find(|q| q.name == name) else {
            continue;
        };
        if matches!(cdf.params.ty[p], CdfType::Radio | CdfType::Cyclic) && out.kind == Kind::Str {
            out.kind = Kind::Choice;
            out.choices = cdf.choices_of(p).map(str::to_owned).collect();
        }
        let callback = cdf.str(cdf.params.callback[p]);
        if !callback.trim().is_empty() {
            reviews.push(review(
                Reason::Tcl,
                format!("CDF callback on {name}: {callback}"),
            ));
        }
    }
}

/// Netlist text that may be `tcleval(…)`-wrapped → code/body text: PDK variables as
/// `${VAR}`, every literal `$` doubled (netlist spec §7.1 expansion syntax).
fn netlist_text(text: &str, vars: &[(String, String)]) -> Result<String, String> {
    let t = text.trim();
    match t.strip_prefix("tcleval(").and_then(|b| b.strip_suffix(')')) {
        Some(inner) => tcl::subst(inner, vars).or_else(|e| {
            // Tcl inside SPICE comment lines (`* path: $path`, `[xschem get …]`) has no
            // netlist effect: drop those lines rather than the whole block.
            let tcl_comment = |l: &&str| l.trim_start().starts_with('*') && l.contains(['[', '$']);
            if !inner.lines().any(|l| tcl_comment(&l)) {
                return Err(e);
            }
            let kept: Vec<&str> = inner.lines().filter(|l| !tcl_comment(l)).collect();
            tcl::subst(&kept.join("\n"), vars)
        }),
        None => Ok(text.replace('$', "$$")),
    }
}

/// Splits text into verbatim lines (no CR/LF), dropping leading and trailing blank lines.
fn lines(text: &str) -> Vec<String> {
    let v: Vec<String> = text
        .lines()
        .map(|l| l.trim_end_matches('\r').replace(['\r', '\0'], ""))
        .collect();
    let start = v
        .iter()
        .position(|l| !l.trim().is_empty())
        .unwrap_or(v.len());
    let end = v
        .iter()
        .rposition(|l| !l.trim().is_empty())
        .map_or(start, |e| e + 1);
    v[start..end].to_vec()
}

/// The bit names of a pin name: `A[2:0]` → `A[2] A[1] A[0]`, `A[3]`, a bundle `a,b`,
/// else the name itself.
fn pin_bits(name: &str) -> Vec<String> {
    let mut out = Vec::new();
    for t in name.split(',') {
        let range = t
            .strip_suffix(']')
            .and_then(|r| r.split_once('['))
            .and_then(|(base, r)| {
                let (a, b) = r.split_once(':').unwrap_or((r, r));
                Some((
                    base,
                    a.trim().parse::<i64>().ok()?,
                    b.trim().parse::<i64>().ok()?,
                ))
            });
        match range {
            Some((base, a, b)) if a.abs_diff(b) < 65_536 => {
                let step = if b >= a { 1 } else { -1 };
                let mut i = a;
                loop {
                    out.push(format!("{base}[{i}]"));
                    if i == b {
                        break;
                    }
                    i += step;
                }
            }
            _ => out.push(t.to_owned()),
        }
    }
    out
}

/// The `@pinlist` replacement `{P} {Q} …` ordering the ordinary pins like `ports` (a
/// `.subckt` line's bit names, matched case-insensitively): each pin's bits must appear
/// together and in its own order, and every pin exactly once. `None` otherwise.
fn pin_order(pins: &[PinOut], ports: &[String]) -> Option<String> {
    let ordinary: Vec<&PinOut> = pins
        .iter()
        .filter(|p| p.inherit.is_none() && !p.alias)
        .collect();
    let bits: Vec<Vec<String>> = ordinary.iter().map(|p| pin_bits(&p.name)).collect();
    let mut used = vec![false; ordinary.len()];
    let mut holes = String::new();
    let mut i = 0;
    while i < ports.len() {
        let rest = &ports[i..];
        let k = (0..ordinary.len()).find(|&k| {
            !used[k]
                && bits[k].len() <= rest.len()
                && bits[k]
                    .iter()
                    .zip(rest)
                    .all(|(b, p)| b.eq_ignore_ascii_case(p))
        })?;
        let base = ordinary[k].name.split('[').next().unwrap_or("");
        if !is_ident(base) {
            return None;
        }
        used[k] = true;
        if !holes.is_empty() {
            holes.push(' ');
        }
        holes.push('{');
        holes.push_str(base);
        holes.push('}');
        i += bits[k].len();
    }
    assert!(i == ports.len());
    used.iter().all(|u| *u).then_some(holes)
}

#[cfg(test)]
mod tests {
    use super::{PinOut, dotted_bus, pin_order};
    use crate::pdk::spice::subckt_lines;

    fn pin(name: &str) -> PinOut {
        PinOut {
            name: name.to_owned(),
            xs: name.to_owned(),
            dir: "inout",
            xy: [0, 0],
            inherit: None,
            alias: false,
        }
    }

    #[test]
    fn subckt_ports_order_the_pinlist() {
        // golden tb_symbol_include: `.subckt symbol_include z y[5] … VCC VSS A[2] … B C`.
        let text = "* a file\n.subckt symbol_include z y[5] y[4] y[3] y[2]\n+ VCC VSS\n* note\n+ A[2] A[1] A[0] B C W=10 L=1\n...\n.ends\n";
        let found = subckt_lines(text);
        assert_eq!(found.len(), 1);
        let (name, ports) = &found[0];
        assert_eq!(name, "symbol_include");
        assert_eq!(ports.len(), 12);
        let pins: Vec<PinOut> = ["A[2:0]", "Z", "B", "Y[5:2]", "C", "vcc", "VSS"]
            .into_iter()
            .map(pin)
            .collect();
        assert_eq!(
            pin_order(&pins, ports).as_deref(),
            Some("{Z} {Y} {vcc} {VSS} {A} {B} {C}")
        );
        // Negative space: a bus whose bits are split, a missing pin, an unknown port.
        let split: Vec<String> = ["z", "y[5]", "VCC", "y[4]"].map(String::from).to_vec();
        assert_eq!(pin_order(&pins, &split), None);
        assert_eq!(pin_order(&pins[..2], &ports[..1]), None);
        let unknown: Vec<String> = ["q"].map(String::from).to_vec();
        assert_eq!(pin_order(&pins[1..2], &unknown), None);
        assert!(subckt_lines("R1 a b 1k\n.ends\n").is_empty());
        // An inline comment is no port.
        assert_eq!(subckt_lines(".subckt x a b ; c\n")[0].1, ["a", "b"]);
    }

    #[test]
    fn dotted_buses_become_bundles() {
        assert_eq!(dotted_bus("G2[9..7]"), "G29,G28,G27");
        assert_eq!(dotted_bus("~D[1..3]"), "~D1,~D2,~D3");
        assert_eq!(dotted_bus("A[6..0..2],B"), "A6,A4,A2,A0,B");
        // Untouched: colon ranges, single bits, malformed ranges.
        for s in ["A[3:0]", "A[3]", "A[x..1]", "A[3..", "plain"] {
            assert_eq!(dotted_bus(s), s);
        }
    }
}
