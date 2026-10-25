//! Which xschem files make which Schemify cells, and their libraries (spec `import.md` §1):
//! classification of symbols, (symbol, schematic) pairs, variants and alternates, library names.

use super::*;

impl<'o> Tables<'o> {
    // ------------------------------------------------------------------ 2. classify

    pub(super) fn classify_symbols(&mut self) {
        for s in 0..self.srcs.len() {
            if self.srcs[s].ext != Ext::Sym {
                continue;
            }
            let iface = symbol::extract(&self.srcs[s].file);
            let raw = iface.attrs.get("type").unwrap_or("").trim().to_owned();
            let ty = if raw.starts_with("tcleval(") && raw.contains("[symtype]") {
                "subcircuit".to_owned() // the known default of IHP's `symtype` proc
            } else {
                raw
            };
            let has_format = iface
                .attrs
                .get("format")
                .is_some_and(|f| !f.trim().is_empty());
            let class = match ty.as_str() {
                "label" => SymClass::Label,
                "ipin" | "opin" | "iopin" => SymClass::Port,
                "noconn" => SymClass::Noconn,
                "netlist_commands" | "spice_parameters" => SymClass::Code,
                "show_label" | "scope" => SymClass::Drop(Drop::Display),
                "launcher"
                | "logo"
                | "architecture"
                | "arch_declarations"
                | "attributes"
                | "package"
                | "port_attributes"
                | "use"
                | "timescale"
                | "verilog_preprocessor"
                | "generic"
                | "annotator" => SymClass::Drop(Drop::NonElectrical),
                "netlist_options" | "bus_tap" => SymClass::Review(Reason::Unsupported),
                "probe" if !has_format => SymClass::Drop(Drop::Display),
                // A pinless symbol whose format uses `extra` nets is a device with only
                // inherited pins (sky130 `fill_1`, `tap_1`: LVS goldens list them).
                // (`plan_symbol`'s pin rule: without one, a `.shprim` with no pin, E025.)
                _ if iface.pins.name.is_empty()
                    && has_format
                    && !implicit_nets(&iface).is_empty() =>
                {
                    SymClass::Cell
                }
                _ if iface.pins.name.is_empty() && has_format => SymClass::Code,
                _ if iface.pins.name.is_empty() => SymClass::Drop(Drop::NonElectrical),
                t if t.starts_with("tcleval(") => SymClass::Review(Reason::Tcl),
                _ => SymClass::Cell,
            };
            self.sym_of_src[s] = Some(self.syms.len() as u32);
            self.syms.push(Sym {
                src: s as u32,
                iface,
                ty,
                class,
            });
        }
        let n = self.syms.len();
        self.plans = (0..n).map(|_| None).collect();
        self.sym_ifaces = (0..n).map(|_| None).collect();
        self.targets = vec![None; n];
        assert!(
            self.syms
                .iter()
                .all(|s| self.srcs[s.src as usize].ext == Ext::Sym)
        );
    }

    /// Marks parameters that instances in the inputs use as free text, so their kind
    /// is `string` (an override there would otherwise be lost to review).
    pub(super) fn scan_overrides(&mut self) {
        for x in 0..self.srcs.len() as u32 {
            let f = &self.srcs[x as usize].file;
            for row in 0..f.instances.symref.len() {
                let Some(s) = self
                    .resolve(x, f.str(f.instances.symref[row]))
                    .and_then(|src| self.sym_of_src[src as usize])
                else {
                    continue;
                };
                let props = Attrs::parse(f.str(f.instances.props[row]));
                for i in 0..props.len() {
                    let v = props.value(i).trim();
                    if v.is_empty()
                        || v.contains("tcleval(")
                        || override_text(v, Kind::Number, &[], &[]).is_ok()
                    {
                        continue;
                    }
                    self.textual.insert((s, props.key(i).to_owned()));
                }
            }
        }
    }
}

impl<'o> Tables<'o> {
    // ------------------------------------------------------------------ 4. cells

    /// The schematic a cell symbol is implemented by, if any: `default_schematic=ignore`
    /// → none; `schematic=` → that file (IHP `hierarchy_config` → its known default, the
    /// same-named schematic in the sibling `sch` tree); else `<stem>.sch` beside it.
    pub(super) fn schematic_of(&self, s: usize, acc: &mut Acc) -> Option<u32> {
        let sym = &self.syms[s];
        let path = &self.srcs[sym.src as usize].path;
        let (dir, stem) = (dir_of(path), stem_of(path));
        if sym.iface.attrs.get("default_schematic").map(str::trim) == Some("ignore") {
            return None;
        }
        match sym
            .iface
            .attrs
            .get("schematic")
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            Some(v) if v.contains("[hierarchy_config") => {
                let sch_dir = dir.replace("/sym/", "/sch/");
                self.index.get(&format!("{sch_dir}/{stem}.sch")).copied()
            }
            Some(v) if v.starts_with("tcleval(") => {
                acc.item(
                    sym.src,
                    0,
                    Class::Review(Reason::Tcl),
                    0,
                    format!("schematic={v}"),
                );
                None
            }
            Some(v) => {
                let v = v.replace("@symname", stem);
                self.resolve(sym.src, &v)
                    .or_else(|| self.resolve(sym.src, &format!("{v}.sch")))
            }
            None => self
                .index
                .get(&format!("{dir}/{stem}.sch"))
                .copied()
                .or_else(|| self.along_path(path, "sch")),
        }
    }

    /// The input named like `path` with extension `ext`, looked up along the search path:
    /// `path` relative to the first root holding it (the name a reference would use) is
    /// tried under every root in order. xschem finds a symbol's default schematic this way,
    /// so a library kept as `symbols/` and `schematics/` search-path entries pairs `x.sym`
    /// with `x.sch` (spec §1).
    pub(super) fn along_path(&self, path: &str, ext: &str) -> Option<u32> {
        let rel = self.opts.roots.iter().find_map(|root| {
            let root = normalize(root);
            path.strip_prefix(root.as_str())?
                .strip_prefix('/')
                .map(str::to_owned)
        })?;
        let stem = rel.rsplit_once('.').map_or(rel.as_str(), |(s, _)| s);
        debug_assert!(!stem.is_empty());
        let name = format!("{stem}.{ext}");
        self.opts
            .roots
            .iter()
            .find_map(|root| self.index.get(&join(&normalize(root), &name)).copied())
    }

    pub(super) fn plan_cells(&mut self, acc: &mut Acc) {
        let mut consumed: HashSet<u32> = HashSet::new();
        for s in 0..self.syms.len() {
            if self.plans[s].is_none() {
                continue;
            }
            let sch = self.schematic_of(s, acc);
            let path = self.srcs[self.syms[s].src as usize].path.clone();
            let shared = sch.is_some_and(|x| !consumed.insert(x));
            let c = self.cells.len() as u32;
            self.cells.push(Cell {
                dir: dir_of(&path).to_owned(),
                name: stem_of(&path).replace(':', "_"),
                kind: if sch.is_some() {
                    FileKind::Shcomp
                } else {
                    FileKind::Shprim
                },
                sym: Some(s as u32),
                sch,
                tally_sym: true,
                tally_sch: !shared,
                copy_of: None,
                lib: 0,
            });
            self.targets[s] = Some(Target::Cell(c));
        }
        // A schematic as a symbol: xschem netlists a schematic's `ipin`/`opin`/`iopin`
        // instances as its interface (the LVS `.subckt` ports of a symbol-less schematic,
        // and the pins of a `.sch` placed as a symbol, LCC). One derived symbol per such
        // schematic that no cell symbol owns (the cell `<stem>`), or that is owned but
        // placed as a symbol (a second cell `<stem>__lcc`: its pins sit elsewhere).
        let placed = self.placed_schematics();
        for x in 0..self.srcs.len() as u32 {
            let owned = consumed.contains(&x);
            if self.srcs[x as usize].ext != Ext::Sch || (owned && !placed.contains(&x)) {
                continue;
            }
            let owner: Vec<String> = self
                .cells
                .iter()
                .find(|c| c.sch == Some(x) && c.sym.is_some())
                .and_then(|c| self.plans[c.sym.expect("checked") as usize].as_ref())
                .map(|p| p.pins.iter().map(|q| q.xs.clone()).collect())
                .unwrap_or_default();
            let Some((iface, ty)) = self.derived_iface(x, &owner) else {
                continue;
            };
            let s = self.syms.len();
            self.syms.push(Sym {
                src: x,
                iface,
                ty,
                class: SymClass::Cell,
            });
            self.plans.push(None);
            self.sym_ifaces.push(None);
            self.targets.push(None);
            let plan = self.plan_symbol(s, acc);
            self.sym_ifaces[s] = Some(iface_of_plan(&plan));
            self.plans[s] = Some(plan);
            self.sym_of_src[x as usize] = Some(s as u32);
            let path = &self.srcs[x as usize].path;
            let stem = stem_of(path).replace(':', "_");
            let c = self.cells.len() as u32;
            let name = if owned { format!("{stem}__lcc") } else { stem };
            if owned {
                acc.item(
                    x,
                    0,
                    Class::AltCell,
                    0,
                    format!("{name} (the schematic placed as a symbol)"),
                );
            }
            self.cells.push(Cell {
                dir: dir_of(path).to_owned(),
                name,
                kind: FileKind::Shcomp,
                sym: Some(s as u32),
                sch: Some(x),
                tally_sym: false,
                tally_sch: !owned,
                copy_of: None,
                lib: 0,
            });
            self.targets[s] = Some(Target::Cell(c));
            consumed.insert(x);
        }
        for x in 0..self.srcs.len() as u32 {
            if self.srcs[x as usize].ext != Ext::Sch || consumed.contains(&x) {
                continue;
            }
            let path = &self.srcs[x as usize].path;
            self.cells.push(Cell {
                dir: dir_of(path).to_owned(),
                name: stem_of(path).replace(':', "_"),
                kind: FileKind::Shbench,
                sym: None,
                sch: Some(x),
                tally_sym: false,
                tally_sch: true,
                copy_of: None,
                lib: 0,
            });
        }
        // A component schematic carrying netlist text (simulation commands, models) is
        // also a test bench in xschem: emit a bench copy `<cell>__top` for that text.
        for c in 0..self.cells.len() {
            let cell = &self.cells[c];
            let Some(x) = cell
                .sch
                .filter(|_| cell.kind == FileKind::Shcomp && cell.tally_sch)
            else {
                continue;
            };
            if !self.has_netlist_text(x) {
                continue;
            }
            let copy = Cell {
                dir: cell.dir.clone(),
                name: format!("{}__top", cell.name),
                kind: FileKind::Shbench,
                sym: None,
                sch: Some(x),
                tally_sym: false,
                tally_sch: false,
                copy_of: Some(c as u32),
                lib: 0,
            };
            acc.item(
                x,
                0,
                Class::AltCell,
                0,
                format!("{} (bench copy carrying the netlist text)", copy.name),
            );
            self.cells.push(copy);
        }
        // Alternate schematics requested by instances (`schematic=` on a `C` record).
        for x in 0..self.srcs.len() as u32 {
            let f = &self.srcs[x as usize].file;
            for row in 0..f.instances.symref.len() {
                let props = Attrs::parse(f.str(f.instances.props[row]));
                let Some(alt) = props
                    .get("schematic")
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                else {
                    continue;
                };
                let target = self
                    .resolve(x, f.str(f.instances.symref[row]))
                    .and_then(|src| self.sym_of_src[src as usize])
                    .and_then(|s| self.targets[s as usize]);
                let Some(Target::Cell(c)) = target else {
                    continue;
                };
                let Some(a) = self.alt_source(x, c, alt) else {
                    continue;
                };
                if Some(a) == self.cells[c as usize].sch || self.alt_cells.contains_key(&(c, a)) {
                    continue;
                }
                let base = &self.cells[c as usize];
                let name = format!(
                    "{}__{}",
                    base.name,
                    stem_of(&self.srcs[a as usize].path).replace(':', "_")
                );
                let new = Cell {
                    dir: base.dir.clone(),
                    name,
                    kind: FileKind::Shcomp,
                    sym: base.sym,
                    sch: Some(a),
                    tally_sym: false,
                    tally_sch: false,
                    copy_of: None,
                    lib: 0,
                };
                acc.item(
                    x,
                    f.instances.line[row],
                    Class::AltCell,
                    0,
                    format!("{} for schematic={alt}", new.name),
                );
                self.alt_cells.insert((c, a), self.cells.len() as u32);
                self.cells.push(new);
            }
        }
        assert!(
            self.cells
                .iter()
                .all(|c| c.sym.is_some() || c.sch.is_some())
        );
        assert!(
            self.cells
                .iter()
                .all(|c| (c.kind == FileKind::Shbench) == c.sym.is_none())
        );
    }

    /// Instance netlist text that differs from the symbol's: `(key, value)` per key of
    /// [`INSTANCE_TEXT`] the instance sets (a blank value counts: `lvs_format=" "`
    /// netlists nothing) to other text than symbol `s`, unless the key is a parameter
    /// of `target`.
    pub(super) fn overrides_of(
        &self,
        s: u32,
        target: Target,
        props: &Attrs,
    ) -> Vec<(&'static str, String)> {
        let sym = &self.syms[s as usize];
        let params = &self.iface_of(target).params;
        let mut out = Vec::new();
        for key in INSTANCE_TEXT {
            let Some(v) = props.get(key).filter(|v| !v.is_empty()) else {
                continue;
            };
            let differs = sym.iface.attrs.get(key).map(collapse_ws) != Some(collapse_ws(v));
            if differs && !params.iter().any(|p| p.0 == key) {
                out.push((key, v.to_owned()));
            }
        }
        assert!(out.len() <= INSTANCE_TEXT.len());
        out
    }

    /// Per-instance variant cells (spec §3): an instance whose `format`, `lvs_format` or
    /// `spice_sym_def` differs from its symbol's netlists through `<cell>__<instance>`,
    /// a copy of its target (its `schematic=` alternate included) with those attributes;
    /// instances with the same symbol, schematic and overrides share one. A
    /// `spice_sym_def` makes the copy a leaf netlisted by that text. A leaf copy renders
    /// `@symname` as the subckt the instance still calls: the `schematic=` stem when the
    /// instance sets one (xschem names the subckt after it), else the symbol's stem.
    pub(super) fn plan_variants(&mut self, acc: &mut Acc) {
        // (symbol, schematic, symname, overrides) → variant cell.
        let mut seen: HashMap<(u32, Option<u32>, String, String), u32> = HashMap::new();
        let mut wanted = Vec::new();
        for x in 0..self.srcs.len() as u32 {
            if self.srcs[x as usize].ext != Ext::Sch {
                continue;
            }
            let f = &self.srcs[x as usize].file;
            for row in 0..f.instances.symref.len() {
                let Some(s) = self
                    .resolve(x, f.str(f.instances.symref[row]))
                    .and_then(|src| self.sym_of_src[src as usize])
                    .filter(|&s| self.syms[s as usize].class == SymClass::Cell)
                else {
                    continue;
                };
                let Some(target) = self.targets[s as usize] else {
                    continue;
                };
                let props = Attrs::parse(f.str(f.instances.props[row]));
                let overrides = self.overrides_of(s, target, &props);
                if overrides.is_empty() {
                    continue;
                }
                let alt = props
                    .get("schematic")
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                    .map(str::to_owned);
                let name = props.get("name").unwrap_or("").trim().to_owned();
                wanted.push((x, row, s, target, overrides, alt, name));
            }
        }
        for (x, row, s, target, overrides, alt, inst) in wanted {
            let line = self.srcs[x as usize].file.instances.line[row];
            let src = self.syms[s as usize].src;
            let base_stem = stem_of(&self.srcs[src as usize].path).to_owned();
            let sym_def = overrides.iter().any(|o| o.0 == "spice_sym_def");
            let base = match target {
                Target::Cell(c) => Some(c),
                Target::Core(_) => None,
            };
            let alt_src = alt
                .as_deref()
                .zip(base)
                .and_then(|(a, c)| self.alt_source(x, c, a));
            let sch = match base {
                Some(c) if !sym_def => alt_src.or(self.cells[c as usize].sch),
                _ => None,
            };
            let symname = match alt.as_deref() {
                Some(a) if sch.is_none() => stem_of(a).to_owned(),
                _ => base_stem.clone(),
            };
            let text: Vec<String> = overrides.iter().map(|(k, v)| format!("{k}={v}")).collect();
            let key = (s, sch, symname.clone(), text.join("\n"));
            if let Some(&c) = seen.get(&key) {
                self.variants.insert((x, row as u32), c);
                continue;
            }
            // The copied symbol: the instance's text first, so every lookup sees it.
            let mut iface = self.syms[s as usize].iface.clone();
            for (k, v) in &overrides {
                iface.attrs.prepend(k, v);
            }
            if sch.is_none() {
                for k in ["format", "lvs_format", "spectre_format", "verilog_format"] {
                    let named = iface
                        .attrs
                        .get(k)
                        .filter(|v| v.contains("@symname"))
                        .map(|v| v.replace("@symname", &symname));
                    if let Some(v) = named {
                        iface.attrs.prepend(k, &v);
                    }
                }
            }
            let s2 = self.syms.len() as u32;
            let ty = self.syms[s as usize].ty.clone();
            self.syms.push(Sym {
                src,
                iface,
                ty,
                class: SymClass::Cell,
            });
            self.plans.push(None);
            self.sym_ifaces.push(None);
            self.targets.push(None);
            let textual: Vec<String> = self
                .textual
                .iter()
                .filter(|(t, _)| *t == s)
                .map(|(_, k)| k.clone())
                .collect();
            self.textual.extend(textual.into_iter().map(|k| (s2, k)));
            // The symbol's own findings were reported once, with the symbol.
            let active = std::mem::replace(&mut acc.active, false);
            let plan = self.plan_symbol(s2 as usize, acc);
            acc.active = active;
            if plan.pins.is_empty() {
                // The instance's text leaves a pinless symbol without its inherited pins:
                // no valid cell (`E025`), so the instance keeps its symbol's netlisting.
                acc.item(
                    x,
                    line,
                    Class::Review(Reason::InstanceText),
                    0,
                    format!("{}: the variant would have no pin", text.join(" ")),
                );
                continue;
            }
            if sch.is_none() && plan.leaf.ignore.as_deref() == Some("true") {
                acc.item(
                    x,
                    line,
                    Class::Review(Reason::InstanceText),
                    0,
                    format!("{}: the variant does not netlist", text.join(" ")),
                );
            }
            self.sym_ifaces[s2 as usize] = Some(iface_of_plan(&plan));
            self.plans[s2 as usize] = Some(plan);
            let (dir, base_name) = match base {
                Some(c) => (
                    self.cells[c as usize].dir.clone(),
                    self.cells[c as usize].name.clone(),
                ),
                None => (
                    dir_of(&self.srcs[x as usize].path).to_owned(),
                    base_stem.replace(':', "_"),
                ),
            };
            let inst: String = inst
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
                .collect();
            let name = format!("{base_name}__{inst}");
            acc.item(
                x,
                line,
                Class::AltCell,
                0,
                format!("{name} for {}", text.join(" ")),
            );
            let c = self.cells.len() as u32;
            self.cells.push(Cell {
                dir,
                name,
                kind: if sch.is_some() {
                    FileKind::Shcomp
                } else {
                    FileKind::Shprim
                },
                sym: Some(s2),
                sch,
                tally_sym: false,
                tally_sch: false,
                copy_of: None,
                lib: 0,
            });
            self.targets[s2 as usize] = Some(Target::Cell(c));
            self.variants.insert((x, row as u32), c);
            seen.insert(key, c);
        }
        assert!(
            self.variants
                .values()
                .all(|&c| (c as usize) < self.cells.len())
        );
    }

    /// Schematic sources some instance places as a symbol (`C {x.sch}`).
    pub(super) fn placed_schematics(&self) -> HashSet<u32> {
        let mut out = HashSet::new();
        for x in 0..self.srcs.len() as u32 {
            let f = &self.srcs[x as usize].file;
            for row in 0..f.instances.symref.len() {
                if let Some(t) = self.resolve(x, f.str(f.instances.symref[row]))
                    && self.srcs[t as usize].ext == Ext::Sch
                {
                    out.insert(t);
                }
            }
        }
        out
    }

    /// The interface of schematic `x` used as a symbol: its own `K` attributes (type
    /// `subcircuit` when it sets none) and one pin per `ipin`/`opin`/`iopin` instance in
    /// file order, at that instance's pin, named by its `lab` (first occurrence wins),
    /// direction `in`/`out`/`inout`. `None` when it has no port instance.
    pub(super) fn derived_iface(
        &self,
        x: u32,
        order: &[String],
    ) -> Option<(SymbolInterface, String)> {
        let f = &self.srcs[x as usize].file;
        let mut iface = symbol::extract(f);
        iface.pins = symbol::Pins::default();
        iface.names.clear();
        // Port rows ranked by the owning symbol's pin order (stable: others after, in
        // file order): xschem's `.subckt` of a placed schematic follows its `.sym`
        // (golden poweramp_lcc `mos_power_ampli`, loading `switch_rreal`).
        let mut rows: Vec<usize> = (0..f.instances.symref.len()).collect();
        let lab_of = |row: usize| {
            Attrs::parse(f.str(f.instances.props[row]))
                .get("lab")
                .map(|l| l.trim().to_owned())
        };
        rows.sort_by_cached_key(|&row| {
            lab_of(row)
                .and_then(|l| order.iter().position(|o| *o == l))
                .unwrap_or(order.len())
        });
        for row in rows {
            let Some(p) = self
                .resolve(x, f.str(f.instances.symref[row]))
                .and_then(|src| self.sym_of_src[src as usize])
                .filter(|&p| self.syms[p as usize].class == SymClass::Port)
            else {
                continue;
            };
            let port = &self.syms[p as usize];
            let props = Attrs::parse(f.str(f.instances.props[row]));
            let lab = props
                .get("lab")
                .or_else(|| port.iface.template.get("lab"))
                .unwrap_or("")
                .trim();
            let taken = (0..iface.pins.name.len()).any(|k| iface.pin_name(k) == lab);
            let Some(at) = self.anchor(f, row, p).filter(|_| !lab.is_empty() && !taken) else {
                continue;
            };
            iface
                .pins
                .name
                .push(crate::push_span(&mut iface.names, lab));
            iface.pins.dir.push(match port.ty.as_str() {
                "ipin" => PinDir::In,
                "opin" => PinDir::Out,
                _ => PinDir::InOut,
            });
            iface.pins.center.push(at);
        }
        if iface.pins.name.is_empty() {
            return None;
        }
        let ty = match iface.attrs.get("type").map(str::trim) {
            Some(t) if !t.is_empty() => t.to_owned(),
            _ => "subcircuit".to_owned(),
        };
        Some((iface, ty))
    }

    /// Whether schematic `x` holds netlist text: code instances or `S {}` text.
    pub(super) fn has_netlist_text(&self, x: u32) -> bool {
        let f = &self.srcs[x as usize].file;
        let s_text = (0..f.globals.tag.len()).any(|i| {
            f.globals.tag[i] == GlobalTag::S && !f.str(f.globals.text[i]).trim().is_empty()
        });
        s_text
            || (0..f.instances.symref.len()).any(|row| {
                self.resolve(x, f.str(f.instances.symref[row]))
                    .and_then(|src| self.sym_of_src[src as usize])
                    .is_some_and(|s| self.syms[s as usize].class == SymClass::Code)
                    || top_only(f, row)
            })
    }

    pub(super) fn alt_source(&self, from: u32, c: u32, value: &str) -> Option<u32> {
        let base = &self.cells[c as usize];
        let v = value.replace("@symname", &base.name);
        let sym_src = base.sym.map(|s| self.syms[s as usize].src)?;
        [from, sym_src].into_iter().find_map(|f| {
            self.resolve(f, &v)
                .or_else(|| self.resolve(f, &format!("{v}.sch")))
                .filter(|&src| self.srcs[src as usize].ext == Ext::Sch)
        })
    }
}

impl<'o> Tables<'o> {
    // ------------------------------------------------------------------ 5. libraries

    /// One library per cell directory, named after the directory (walking up past the
    /// generic `xschem`/`sym`/`sch` components); clashes get `_2`, `_3`, … in path order.
    pub(super) fn name_libs(&mut self) {
        let dirs: BTreeSet<String> = self.cells.iter().map(|c| c.dir.clone()).collect();
        let mut names: Vec<String> = Vec::with_capacity(dirs.len());
        let mut of_dir: HashMap<String, u16> = HashMap::new();
        for d in &dirs {
            let base = d
                .rsplit('/')
                .find(|c| !c.is_empty() && !matches!(*c, "xschem" | "sym" | "sch"))
                .unwrap_or("lib")
                .replace(':', "_");
            let mut name = base.clone();
            let mut k = 2;
            while name == "core" || names.contains(&name) {
                name = format!("{base}_{k}");
                k += 1;
            }
            assert!(names.len() < usize::from(u16::MAX), "too many libraries");
            of_dir.insert(d.clone(), names.len() as u16);
            names.push(name);
        }
        let mut taken: HashSet<(u16, String)> = HashSet::new();
        for c in &mut self.cells {
            c.lib = of_dir[&c.dir];
            let base = c.name.clone();
            let mut k = 2;
            while !taken.insert((c.lib, c.name.clone())) {
                c.name = format!("{base}_{k}");
                k += 1;
            }
        }
        assert!(
            !names
                .iter()
                .any(|n| n == "core" || n.is_empty() || n.contains([':', '/']))
        );
        self.lib_names = names;
    }
}
