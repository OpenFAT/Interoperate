//! One import's lookup tables (`Ctx`): where each dump cell lands, the interface every
//! master offers its instances, and dump coordinates in Schemify units (spec `interop.md`
//! §3.1, §6.1, §7.2 ids).

use super::*;

/// Where one dump cell lands.
#[derive(Clone, Debug, Default)]
pub(super) struct Target {
    /// A file is produced.
    pub(super) write: bool,
    /// Schemify library (`core` for the bundled cells).
    pub(super) slib: String,
    /// Project-relative directory.
    pub(super) dir: String,
    /// File stem.
    pub(super) stem: String,
    /// File kind.
    pub(super) kind: Option<FileKind>,
}

impl Target {
    pub(super) fn path_of(&self, ext: &str) -> String {
        cell_path(&self.dir, &self.stem, ext)
    }

    pub(super) fn path(&self) -> String {
        cell_path(
            &self.dir,
            &self.stem,
            self.kind.map_or("shprim", FileKind::as_str),
        )
    }
}

/// A pin of an interface, Schemify spelling and coordinates.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct PinRow {
    pub(super) name: String,
    pub(super) dir: String,
    pub(super) at: [i32; 2],
    pub(super) inherit: Option<String>,
    pub(super) alias: Option<String>,
}

/// What instances of a master need: pins (port order), params with their Schemify kinds
/// and defaults, and the netSet property name of each inherited pin.
#[derive(Clone, Debug, Default)]
pub(super) struct Iface {
    pub(super) pins: Vec<PinRow>,
    /// `(name, kind, default)`.
    pub(super) params: Vec<(String, String, String)>,
    /// `(virtuoso netSet prop, schemify pin)`.
    pub(super) inherit_props: Vec<(String, String)>,
    /// Params a rule computes (`computed`): an instance takes no value for them.
    pub(super) computed: Vec<String>,
    /// Params only Virtuoso's callbacks derive (`vderived true`): an instance carries
    /// their values as `cached` lines (D20 as amended 2026-10-15).
    pub(super) vderived: Vec<String>,
    /// What those depend on (header `vinputs`).
    pub(super) vinputs: Vec<String>,
}

impl Iface {
    pub(super) fn kind(&self, p: &str) -> Option<&str> {
        self.params.iter().find(|x| x.0 == p).map(|x| x.1.as_str())
    }
    pub(super) fn def(&self, p: &str) -> Option<&str> {
        self.params.iter().find(|x| x.0 == p).map(|x| x.2.as_str())
    }
}

/// One import's lookup tables.
pub(super) struct Ctx<'a> {
    pub(super) dump: &'a Dump,
    pub(super) map: &'a Mapping,
    opts: &'a ImportOptions,
    pub(super) existing: &'a Files,
    /// Schemify library → directory (manifest, then options).
    pub(super) libs: BTreeMap<String, String>,
    pub(super) targets: Vec<Target>,
    pub(super) ifaces: Vec<Option<Iface>>,
    /// (lib, cell) of every cell a maestro test names.
    benches: BTreeSet<(String, String)>,
}

/// Schemify library of Virtuoso library `vlib`.
pub(super) fn slib_of(vlib: &str, opts: &ImportOptions) -> String {
    if let Some(s) = opts.lib_map.get(vlib) {
        return s.clone();
    }
    match vlib {
        ROOT_LIB => String::new(),
        CORE_LIB => "core".into(),
        l => l.into(),
    }
}

pub(super) fn consumed(lib: &str, cell: &str) -> bool {
    (lib == "basic" && matches!(cell, "ipin" | "opin" | "iopin" | "noConn"))
        || devices::is_global_symbol(lib, cell)
}

impl<'a> Ctx<'a> {
    pub(super) fn new(
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
        // Virtuoso names are case-sensitive; a case-insensitive file system (macOS, Windows)
        // keeps one of two files whose paths differ only in case (S15).
        let mut seen: BTreeMap<String, String> = BTreeMap::new();
        for t in ctx.targets.iter().filter(|t| t.write) {
            let p = t.path();
            if let Some(q) = seen.insert(p.to_lowercase(), p.clone()) {
                notes.review(
                    "name",
                    &p,
                    format!("differs from {q} only in case: one file on a case-insensitive disk"),
                );
            }
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
        // A kit cell a map row sends back to a Schemify cell of the same name (xschem's
        // `sky130_fd_pr:nfet_01v8` ↔ the kit's `sky130_fd_pr/nfet_01v8`): its instances
        // point at that cell (`master`), and a mirror would overwrite it. Without the
        // cell in the project the mirror is written (with the row's `spice`, §8.2).
        if !c.design
            && self
                .map
                .by_virtuoso(&c.lib, &c.name)
                .is_some_and(|r| r.cellref == format!("{}:{}", t.slib, t.stem))
            && ["shcomp", "shprim"].iter().any(|ext| {
                let p = cell_path(&t.dir, &t.stem, ext);
                let mirror = format!("provenance \"virtuoso {}/{}\"", c.lib, c.name);
                // The mirror itself is re-imported (and takes the row's `spice`).
                self.existing
                    .get(&p)
                    .is_some_and(|b| !String::from_utf8_lossy(b).contains(&mirror))
            })
        {
            return t;
        }
        // A cell of another library the project already holds as a component (an earlier
        // import of that library, S2): its instances point there, no pinned leaf beside it.
        if !c.design
            && self
                .existing
                .contains_key(&cell_path(&t.dir, &t.stem, "shcomp"))
        {
            notes.add(
                "remapped",
                &format!("{}/{}", c.lib, c.name),
                format!(
                    "instances use {} (a reference cell here)",
                    t.path_of("shcomp")
                ),
            );
            return t;
        }
        if c.is_global_symbol() {
            notes.add(
                "remapped",
                &format!("{}/{}", c.lib, c.name),
                format!(
                    "supply symbol ({}): its instances become global labels",
                    c.symbol().map_or("", |s| s.terms[0].name.as_str())
                ),
            );
            return t;
        }
        let sym = c.symbol().is_some();
        let sch = c.cv("schematic");
        t.kind = if !c.design && c.symbol().is_some_and(|s| s.terms.is_empty()) {
            // A kit's or Cadence's pinless symbol (a sheet border) is no device either.
            notes.dropped(
                "display-only",
                &format!("{}/{}", c.lib, c.name),
                "a symbol without pins is not imported".to_owned(),
            );
            None
        } else if !c.design {
            Some(FileKind::Shprim)
        } else if self.benches.contains(&(c.lib.clone(), c.name.clone())) {
            Some(FileKind::Shbench)
        } else if sym && sch.is_some() {
            Some(FileKind::Shcomp)
        } else if c.symbol().is_some_and(|s| s.terms.is_empty()) {
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

    /// True when instances of `lib/cell` are global labels: analogLib's and basic's supply
    /// symbols, and any dump cell [`DCell::is_global_symbol`] (S11).
    pub(super) fn global_symbol(&self, lib: &str, cell: &str) -> bool {
        devices::is_global_symbol(lib, cell)
            || self
                .find(lib, cell)
                .is_some_and(|k| self.dump.cells[k].is_global_symbol())
    }

    /// The dump index of `lib/cell`.
    pub(super) fn find(&self, lib: &str, cell: &str) -> Option<usize> {
        self.dump
            .cells
            .iter()
            .position(|c| c.lib == lib && c.name == cell)
    }

    fn iface_of_dump(&self, k: usize) -> Option<Iface> {
        let c = &self.dump.cells[k];
        let mut i = Iface::default();
        if let Some(sym) = c.symbol() {
            i.pins = pins_in(c, sym, &mut Notes::default());
            for p in &i.pins {
                if p.inherit.is_some() {
                    let prop = inherit_prop(sym, &p.name)
                        .unwrap_or_else(|| crate::virtuoso::write::vname(&p.name));
                    i.inherit_props.push((prop, p.name.clone()));
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
                if p.computed.is_some() {
                    i.computed.push(p.name.clone());
                }
                i.params.push((p.name, p.kind, p.default));
            }
            for p in cdf.params.iter().filter(|p| p.ty == "netSet") {
                if let Some(pin) = i
                    .pins
                    .iter()
                    .find(|x| crate::virtuoso::write::vname(&x.name) == p.name)
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
    pub(super) fn iface_of_text(bytes: &[u8]) -> Option<Iface> {
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
                        let prop = attr("vinherit")
                            .unwrap_or_else(|| crate::virtuoso::write::vname(&row.name));
                        i.inherit_props.push((prop, row.name.clone()));
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
                _ => {} // the interface is the pins and params
            }
        }
        Some(i)
    }

    /// Where an instance of `lib/cell` points and what its master looks like.
    pub(super) fn master(&self, lib: &str, cell: &str) -> Master {
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
pub(super) struct Master {
    pub(super) slib: String,
    pub(super) dir: String,
    pub(super) stem: String,
    pub(super) iface: Option<Iface>,
    /// `map` row (reverse lookup).
    pub(super) row: Option<usize>,
    /// analogLib → core re-mastering: device row, waveform.
    pub(super) alib: Option<(usize, usize)>,
    /// A core source: current (true) or voltage.
    pub(super) source: Option<bool>,
}

impl Master {
    pub(super) fn cellref(&self, from_dir: &str) -> Option<String> {
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
pub(super) fn orient_in(s: &str) -> Orient {
    profile::orient_from_virtuoso(s).unwrap_or(Orient::R0)
}

/// Converts a dump point, reporting a rounded conversion.
pub(super) fn pt(xy: [i32; 2], upi: i32, what: &str, cell: &str, notes: &mut Notes) -> [i32; 2] {
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
pub(super) fn box_in(b: &[i32], upi: i32, what: &str, cell: &str, notes: &mut Notes) -> [i32; 4] {
    assert!(b.len() >= 4, "a box has 4 coordinates");
    let p = pt([b[0], b[1]], upi, what, cell, notes);
    let q = pt([b[2], b[3]], upi, what, cell, notes);
    profile::norm_box(p, q)
}

/// The centre of a box (exact through doubling).
pub(super) fn centre(b: [i32; 4], upi: i32, what: &str, cell: &str, notes: &mut Notes) -> [i32; 2] {
    let x = i64::from(b[0]) + i64::from(b[2]);
    let y = i64::from(b[1]) + i64::from(b[3]);
    match (i32::try_from(x), i32::try_from(y), upi.checked_mul(2)) {
        (Ok(x), Ok(y), Some(u2)) => pt([x, y], u2, what, cell, notes),
        _ => pt([b[0], b[1]], upi, what, cell, notes),
    }
}

/// Id assignment for one file: `schemifyId` when valid and unused, else
/// `IdRng::for_import("<lib>/<cell>", ordinal)`.
pub(super) struct Ids {
    seed: String,
    taken: BTreeSet<String>,
    ordinal: u32,
}

impl Ids {
    pub(super) fn new(seed: String) -> Ids {
        Ids {
            seed,
            taken: BTreeSet::new(),
            ordinal: 0,
        }
    }

    /// The id of the next object (in dump order), from `wanted` when possible.
    pub(super) fn next(&mut self, wanted: Option<&str>, reserved: &BTreeSet<String>) -> String {
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
