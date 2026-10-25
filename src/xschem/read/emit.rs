//! Cells → Schemify documents (spec `import.md` §2–§5): every record of a cell, written through
//! the schematic output tables.

use super::*;

impl<'o> Tables<'o> {
    // ------------------------------------------------------------------ 6. emit

    pub(super) fn emit_cell(&self, c: usize, acc: &mut Acc) -> (String, Vec<u8>) {
        let cell = &self.cells[c];
        let lib = &self.lib_names[usize::from(cell.lib)];
        let ext = match cell.kind {
            FileKind::Shprim => "shprim",
            FileKind::Shcomp => "shcomp",
            _ => "shbench",
        };
        let path = format!("{lib}/{}.{ext}", cell.name);
        assert!(
            !cell.name.is_empty() && !cell.name.contains('/'),
            "cell names are file stems"
        );
        let mut doc = Document::new(cell.kind);
        let plan = cell.sym.map(|s| {
            self.plans[s as usize]
                .as_ref()
                .expect("cell symbols are planned")
        });
        assert!(
            cell.kind != FileKind::Shprim || plan.is_some_and(|p| !p.pins.is_empty()),
            "{}: a .shprim has at least one pin (E025)",
            cell.name
        );
        // Header.
        let impls = self.impls_of(cell, plan);
        if impls.len() > 1 {
            doc.push_attr("default");
            doc.push_value(impls[0].0);
        }
        if let Some(p) = plan.and_then(|p| p.prefix.as_deref()) {
            doc.push_attr("prefix");
            doc.push_value(p);
        }
        if let Some(s) = cell.sym {
            let ty = &self.syms[s as usize].ty;
            if !ty.is_empty() && !ty.contains(char::is_whitespace) {
                doc.push_attr("category");
                doc.push_value(ty);
            }
        }
        let main_src = cell
            .sym
            .map(|s| self.syms[s as usize].src)
            .or(cell.sch)
            .expect("a cell has a source");
        doc.push_attr("provenance");
        doc.push_value(&format!(
            "xschem {lib}/{}",
            file_of(&self.srcs[main_src as usize].path)
        ));
        // Ranks 1–2: the symbol (a schematic placed as a symbol has no drawing of its own).
        if let (Some(s), Some(plan)) = (cell.sym, plan) {
            acc.active = cell.tally_sym;
            emit_interface(&mut doc, plan, &impls);
            if self.srcs[self.syms[s as usize].src as usize].ext == Ext::Sym {
                self.emit_graphics(&mut doc, s, plan, acc);
            }
            acc.active = true;
        }
        // Rank 3: the schematic.
        let mut out = SchOut::default();
        if let Some(x) = cell.sch {
            let seed = format!("{lib}/{}", file_of(&self.srcs[x as usize].path));
            let mut ids = Ids {
                seed,
                taken: HashSet::new(),
            };
            let parents: Vec<String> = plan.map_or_else(Vec::new, |p| {
                p.params.iter().map(|q| q.name.clone()).collect()
            });
            // Alias pins are no ports: their target's port serves both positions.
            let pins: Vec<(String, String)> = plan.map_or_else(Vec::new, |p| {
                p.pins
                    .iter()
                    .filter(|q| !q.alias)
                    .map(|q| (q.xs.clone(), q.name.clone()))
                    .collect()
            });
            let sc = SchCtx {
                file: x,
                lib: cell.lib,
                bench: cell.kind == FileKind::Shbench,
                parents,
                pins,
            };
            // A bench copy defines the component's numeric parameters as bench variables
            // (xschem leaves them undefined at the top level; these are its defaults),
            // those the schematic's values mention: an unused one (`m`) would be a
            // `.param` xschem's netlist does not have.
            if let Some(base) = cell.copy_of {
                let plan = self.cells[base as usize]
                    .sym
                    .and_then(|s| self.plans[s as usize].as_ref());
                let words = self.value_words(x);
                for p in plan
                    .into_iter()
                    .flat_map(|p| &p.params)
                    .filter(|p| p.kind == Kind::Number)
                    .filter(|p| words.contains(&p.name.to_ascii_lowercase()))
                {
                    out.vars.push((p.name.clone(), p.default.clone()));
                    out.default_vars.push(p.name.clone());
                }
            }
            acc.active = cell.tally_sch;
            self.emit_schematic(&mut doc, &mut ids, &sc, &mut out, acc);
            acc.active = true;
        }
        // Rank 4: bench variables from `.param` lines.
        for (n, e) in &out.vars {
            doc.begin_record(RecordKind::Var);
            doc.push_field(n);
            doc.push_field(e);
        }
        // Rank 5: impl bodies.
        if let Some(plan) = plan
            && !plan.leaf.body.is_empty()
            && impls.iter().any(|i| i.0 == plan.leaf.name)
        {
            doc.begin_record(RecordKind::Body);
            doc.push_field(plan.leaf.name);
            plan.leaf.body.iter().for_each(|l| doc.push_verbatim(l));
        }
        if !out.body.is_empty() && impls.iter().any(|i| i.0 == "schematic") {
            doc.begin_record(RecordKind::Body);
            doc.push_field("schematic");
            out.body.iter().for_each(|l| doc.push_verbatim(l));
        }
        // Rank 6: bench code blocks.
        for (name, lang, text) in &out.codes {
            doc.begin_record(RecordKind::Code);
            doc.push_field(name);
            doc.push_field(lang);
            text.iter().for_each(|l| doc.push_verbatim(l));
        }
        (path, write(&doc).into_bytes())
    }

    /// The impls a cell gets, default first: `(name, lang, formats, ignore)`.
    pub(super) fn impls_of<'p>(&self, cell: &Cell, plan: Option<&'p Plan>) -> Vec<ImplRow<'p>> {
        let Some(p) = plan else { return Vec::new() };
        let leaf = (
            p.leaf.name,
            p.leaf.lang,
            p.leaf.formats.as_slice(),
            p.leaf.ignore.as_deref(),
        );
        let schematic = (
            "schematic",
            "schematic",
            p.schematic_formats.as_slice(),
            p.ignore.as_deref(),
        );
        match (cell.kind, p.subcircuit && !p.sym_def) {
            (FileKind::Shcomp, true) => vec![schematic],
            (FileKind::Shcomp, false) => vec![leaf, schematic],
            _ => vec![leaf],
        }
    }

    pub(super) fn emit_graphics(&self, doc: &mut Document, s: u32, plan: &Plan, acc: &mut Acc) {
        let sym = &self.syms[s as usize];
        let fid = sym.src;
        let f = &self.srcs[fid as usize].file;
        tally_common(f, fid, acc, false);
        let layer_ok = |l: u8| u32::from(l) < CAD_LAYERS;
        for i in 0..f.lines.layer.len() {
            if !layer_ok(f.lines.layer[i]) {
                acc.item(
                    fid,
                    f.lines.line[i],
                    Class::Dropped(Drop::Layer),
                    1,
                    format!("L layer {}", f.lines.layer[i]),
                );
                continue;
            }
            let [x1, y1, x2, y2] = f.lines.xy[i];
            doc.begin_record(RecordKind::Line);
            [x1, y1, x2, y2]
                .into_iter()
                .for_each(|v| doc.push_field_coord(v));
            push_layer(doc, f.lines.layer[i]);
            let props = Attrs::parse(f.str(f.lines.props[i]));
            if let Some(d) = props
                .get("dash")
                .filter(|d| d.trim().parse::<u32>().is_ok_and(|n| n > 0))
            {
                doc.push_attr("dash");
                doc.push_value(d.trim());
            }
            acc.out(fid, 1);
        }
        for i in 0..f.boxes.layer.len() {
            if f.boxes.layer[i] == symbol::PIN_LAYER {
                acc.out(fid, 1); // folded into a `pin` record
                continue;
            }
            if !layer_ok(f.boxes.layer[i]) {
                acc.item(
                    fid,
                    f.boxes.line[i],
                    Class::Dropped(Drop::Layer),
                    1,
                    format!("B layer {}", f.boxes.layer[i]),
                );
                continue;
            }
            let [x1, y1, x2, y2] = f.boxes.xy[i];
            doc.begin_record(RecordKind::Rect);
            [x1, y1, x2, y2]
                .into_iter()
                .for_each(|v| doc.push_field_coord(v));
            push_layer(doc, f.boxes.layer[i]);
            let props = Attrs::parse(f.str(f.boxes.props[i]));
            doc.push_attr("fill");
            doc.push_value(if props.get("fill").map(str::trim) == Some("false") {
                "false"
            } else {
                "true"
            });
            acc.out(fid, 1);
        }
        for i in 0..f.polygons.layer.len() {
            let (first, count) = (f.polygons.first[i] as usize, f.polygons.count[i] as usize);
            let mut pts = f.points[first..first + count].to_vec();
            let closed = pts.len() > 2 && pts.first() == pts.last();
            if closed {
                pts.pop();
            }
            if pts.len() < 2 || !layer_ok(f.polygons.layer[i]) {
                acc.item(
                    fid,
                    f.polygons.line[i],
                    Class::Dropped(Drop::Layer),
                    1,
                    "P with < 2 points or layer >= 22",
                );
                continue;
            }
            doc.begin_record(RecordKind::Poly);
            pts.iter().for_each(|p| {
                doc.push_field_coord(p[0]);
                doc.push_field_coord(p[1]);
            });
            push_layer(doc, f.polygons.layer[i]);
            let props = Attrs::parse(f.str(f.polygons.props[i]));
            if matches!(props.get("fill").map(str::trim), Some("true" | "full")) {
                doc.push_attr("fill");
                doc.push_value("true");
            }
            if closed {
                doc.push_attr("closed");
                doc.push_value("true");
            }
            acc.out(fid, 1);
        }
        for i in 0..f.arcs.layer.len() {
            if !layer_ok(f.arcs.layer[i]) {
                acc.item(
                    fid,
                    f.arcs.line[i],
                    Class::Dropped(Drop::Layer),
                    1,
                    format!("A layer {}", f.arcs.layer[i]),
                );
                continue;
            }
            let [cx, cy, r] = f.arcs.circle[i];
            let [start, sweep] = f.arcs.angles[i];
            if sweep.abs() >= 360_000 {
                doc.begin_record(RecordKind::Circle);
                [cx, cy, r]
                    .into_iter()
                    .for_each(|v| doc.push_field_coord(v));
            } else {
                doc.begin_record(RecordKind::Arc);
                [cx, cy, r, start, sweep]
                    .into_iter()
                    .for_each(|v| doc.push_field_coord(v));
            }
            push_layer(doc, f.arcs.layer[i]);
            let props = Attrs::parse(f.str(f.arcs.props[i]));
            if matches!(props.get("fill").map(str::trim), Some("true" | "full")) {
                doc.push_attr("fill");
                doc.push_value("true");
            }
            acc.out(fid, 1);
        }
        let names: Vec<String> = plan.params.iter().map(|p| p.name.clone()).collect();
        for i in 0..f.texts.text.len() {
            let props = Attrs::parse(f.str(f.texts.props[i]));
            let hidden = props.get("hide").is_some_and(|h| truthy(h) != Some(false));
            let tpl = stext_template(f.str(f.texts.text[i]), &names, plan.spiceprefix.as_deref());
            let (Some(tpl), false) = (tpl, hidden) else {
                acc.item(
                    fid,
                    f.texts.line[i],
                    Class::Dropped(Drop::Display),
                    1,
                    f.str(f.texts.text[i]),
                );
                continue;
            };
            let [x, y] = f.texts.xy[i];
            doc.begin_record(RecordKind::Stext);
            doc.push_field_coord(x);
            doc.push_field_coord(y);
            doc.push_field(orient(f.texts.rot[i], f.texts.flip[i]).as_str());
            doc.push_field(&tpl);
            doc.push_attr("size");
            doc.push_value_coord(f.texts.scale[i][0]);
            for flag in ["hcenter", "vcenter"] {
                if props.get(flag).and_then(truthy) == Some(true) {
                    doc.push_attr(flag);
                }
            }
            let layer = props
                .get("layer")
                .and_then(|l| l.trim().parse::<u8>().ok())
                .unwrap_or(3);
            push_layer(doc, layer);
            acc.out(fid, 1);
        }
        let netlist_records = f.wires.xy.len() + f.instances.symref.len();
        if netlist_records > 0 {
            acc.item(
                fid,
                0,
                Class::Dropped(Drop::SymbolNetlistRecord),
                netlist_records as u32,
                "N/C records drawn inside a symbol",
            );
        }
    }

    pub(super) fn emit_schematic(
        &self,
        doc: &mut Document,
        ids: &mut Ids,
        sc: &SchCtx,
        out: &mut SchOut,
        acc: &mut Acc,
    ) {
        let fid = sc.file;
        let f = &self.srcs[fid as usize].file;
        tally_common(f, fid, acc, true);
        // `S {}`: SPICE text emitted verbatim by xschem, inside the component's subckt when
        // the schematic is one (D17 as amended: a `.shcomp` carries `code` too).
        let g = &f.globals;
        for i in (0..g.tag.len()).filter(|&i| g.tag[i] == GlobalTag::S) {
            let text = f.str(g.text[i]);
            if text.trim().is_empty() {
                acc.out(fid, 1);
            } else {
                match netlist_text(text, self.opts.pdk_vars) {
                    Ok(t) => {
                        out.codes.push((
                            "schematic_spice".to_owned(),
                            "spice".to_owned(),
                            lines(&t),
                        ));
                        acc.out(fid, 1);
                    }
                    Err(d) => acc.item(fid, g.line[i], Class::Review(Reason::Tcl), 1, d),
                }
            }
        }
        // Pass 1: netlist text instances (their `.param`s become bench variables).
        let n = f.instances.symref.len();
        let mut sym_of_row: Vec<Option<u32>> = Vec::with_capacity(n);
        for row in 0..n {
            let s = self
                .resolve(fid, f.str(f.instances.symref[row]))
                .and_then(|x| self.sym_of_src[x as usize]);
            sym_of_row.push(s);
            let Some(s) = s.filter(|&s| self.syms[s as usize].class == SymClass::Code) else {
                continue;
            };
            let line = f.instances.line[row];
            let props = Attrs::parse(f.str(f.instances.props[row]));
            let ignored = props
                .get("spice_ignore")
                .or_else(|| self.syms[s as usize].iface.template.get("spice_ignore"))
                .and_then(truthy);
            if ignored == Some(true) {
                let detail = format!("netlist text instance {}", f.str(f.instances.symref[row]));
                acc.item(fid, line, Class::Dropped(Drop::SpiceIgnore), 1, detail);
                continue;
            }
            // In a component the text is the subckt's (code.sym and its kin default to
            // `only_toplevel=false`); `only_toplevel=true` text is the bench copy's.
            let top_only = props
                .get("only_toplevel")
                .or_else(|| self.syms[s as usize].iface.template.get("only_toplevel"))
                .and_then(truthy)
                == Some(true);
            if !sc.bench && top_only {
                let detail = format!("netlist text instance {}", f.str(f.instances.symref[row]));
                acc.item(fid, line, Class::Dropped(Drop::BenchCopy), 1, detail);
                continue;
            }
            match self.render_code(s, &props) {
                Ok((lang, text)) => {
                    let base = props
                        .get("name")
                        .map(str::trim)
                        .filter(|n| !n.is_empty())
                        .unwrap_or("code");
                    let name = unique(base, out.codes.iter().map(|c| c.0.as_str()));
                    out.codes.push((name, lang, lines(&text)));
                    acc.out(fid, 1);
                }
                Err(r) => acc.item(fid, line, Class::Review(r.reason), 1, r.detail),
            }
        }
        // A bench's `.param` lines become its variables; a component's stay in its
        // subckt text, where xschem writes them.
        for code in out.codes.iter_mut().filter(|_| sc.bench) {
            extract_params(&mut code.2, &mut out.vars, &mut out.default_vars);
        }
        let mut known: Vec<String> = sc.parents.clone();
        known.extend(out.vars.iter().map(|v| v.0.clone()));
        // Wires.
        for i in 0..f.wires.xy.len() {
            let [x1, y1, x2, y2] = f.wires.xy[i];
            if (x1, y1) == (x2, y2) {
                acc.item(fid, f.wires.line[i], Class::Dropped(Drop::ZeroWire), 1, "");
                continue;
            }
            doc.begin_record(RecordKind::Wire);
            doc.push_field(&ids.next(f.wires.line[i], 0));
            [x1, y1, x2, y2]
                .into_iter()
                .for_each(|v| doc.push_field_coord(v));
            out.extend_bbox([x1, y1]);
            out.extend_bbox([x2, y2]);
            let props = Attrs::parse(f.str(f.wires.props[i]));
            if props.get("bus").and_then(truthy) == Some(true) {
                doc.push_attr("bus");
                doc.push_value("true");
            }
            acc.out(fid, 1);
        }
        // Pass 2: everything else.
        for (row, sym_row) in sym_of_row.iter().enumerate() {
            let line = f.instances.line[row];
            let symref = f.str(f.instances.symref[row]);
            let Some(s) = *sym_row else {
                let lcc = self.resolve(fid, symref).is_some();
                let generator = symref.contains('(');
                let reason = if lcc || generator {
                    Reason::Unsupported
                } else {
                    Reason::MissingSymbol
                };
                let what = match (lcc, generator) {
                    (true, _) => "a schematic used as a symbol",
                    (_, true) => "a symbol generator",
                    _ => "symbol not found",
                };
                acc.item(
                    fid,
                    line,
                    Class::Review(reason),
                    1,
                    format!("{what}: {symref}"),
                );
                continue;
            };
            out.extend_bbox(f.instances.xy[row]);
            if !sc.bench && self.syms[s as usize].class == SymClass::Cell && top_only(f, row) {
                // xschem netlists `only_toplevel=true` devices only when this schematic
                // is the top (golden test_analog: n_diffamp's stimuli are not in its
                // subckt): the bench copy carries them.
                let detail = format!("only_toplevel instance {symref}");
                acc.item(fid, line, Class::Dropped(Drop::BenchCopy), 1, detail);
                continue;
            }
            match self.syms[s as usize].class {
                SymClass::Cell => self.emit_instance(doc, ids, sc, row, s, &known, out, acc),
                SymClass::Label => self.emit_label(doc, ids, sc, row, s, false, out, acc),
                SymClass::Port => self.emit_label(doc, ids, sc, row, s, true, out, acc),
                SymClass::Noconn => self.emit_noconn(doc, ids, sc, row, s, acc),
                SymClass::Code => {} // pass 1
                SymClass::Drop(d) => acc.item(fid, line, Class::Dropped(d), 1, symref),
                SymClass::Review(r) => acc.item(fid, line, Class::Review(r), 1, symref.to_owned()),
            }
        }
        // Texts.
        for i in 0..f.texts.text.len() {
            let text = f.str(f.texts.text[i]);
            let props = Attrs::parse(f.str(f.texts.props[i]));
            let display = text.trim().is_empty()
                || text.contains("tcleval(")
                || text.contains("@spice_get")
                || text.contains("@#")
                || props.find("name").is_some()
                || props.find("floater").is_some();
            if display {
                acc.item(fid, f.texts.line[i], Class::Dropped(Drop::Display), 1, text);
                continue;
            }
            doc.begin_record(RecordKind::Text);
            doc.push_field(&ids.next(f.texts.line[i], 0));
            let [x, y] = f.texts.xy[i];
            doc.push_field_coord(x);
            doc.push_field_coord(y);
            doc.push_field(orient(f.texts.rot[i], f.texts.flip[i]).as_str());
            doc.push_field(text);
            doc.push_attr("size");
            doc.push_value_coord(f.texts.scale[i][0]);
            acc.out(fid, 1);
        }
        // Drawings.
        let drawings =
            (f.lines.xy.len() + f.boxes.xy.len() + f.polygons.layer.len() + f.arcs.layer.len())
                as u32;
        if drawings > 0 {
            acc.item(
                fid,
                0,
                Class::Dropped(Drop::Graphic),
                drawings,
                format!("{drawings} L/B/P/A records"),
            );
        }
        // Interface pins bound by name only (xschem binds subcircuit ports by net name):
        // a port at a same-named label, else at a free point outside the drawing.
        if !sc.bench {
            let [mut fx, fy] = out.bbox_min();
            for (k, (xs, pin)) in sc.pins.iter().enumerate() {
                if out.ports.contains(pin) {
                    continue;
                }
                let at = out
                    .labels
                    .iter()
                    .find(|(n, _)| n == xs)
                    .map(|(_, xy)| *xy)
                    .unwrap_or_else(|| {
                        fx -= 20_000;
                        [fx, fy - 20_000]
                    });
                doc.begin_record(RecordKind::Port);
                doc.push_field(&ids.next(0, k as u32));
                doc.push_field_coord(at[0]);
                doc.push_field_coord(at[1]);
                doc.push_field(Orient::R0.as_str());
                doc.push_field(pin);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn emit_instance(
        &self,
        doc: &mut Document,
        ids: &mut Ids,
        sc: &SchCtx,
        row: usize,
        s: u32,
        known: &[String],
        out: &mut SchOut,
        acc: &mut Acc,
    ) {
        let fid = sc.file;
        let f = &self.srcs[fid as usize].file;
        let line = f.instances.line[row];
        let props = Attrs::parse(f.str(f.instances.props[row]));
        let sym = &self.syms[s as usize];
        let xs_tpl = &sym.iface.template;
        assert_eq!(self.syms[s as usize].class, SymClass::Cell);
        let mut target = self.targets[s as usize].expect("cell symbols have targets");
        // An instance with netlist text of its own instantiates its variant cell, which
        // already carries any `schematic=` alternate.
        let variant = self.variants.get(&(fid, row as u32)).copied();
        if let Some(v) = variant {
            target = Target::Cell(v);
        }
        let in_devices = file_of(dir_of(&self.srcs[sym.src as usize].path)) == "devices";
        if in_devices && acc.active {
            acc.device_instances += 1;
            acc.core_instances += u32::from(matches!(target, Target::Core(_)));
        }
        if let Some(alt) = props
            .get("schematic")
            .map(str::trim)
            .filter(|v| !v.is_empty() && variant.is_none())
        {
            let resolved = match target {
                Target::Cell(c) => self.alt_source(fid, c, alt).and_then(|a| {
                    if Some(a) == self.cells[c as usize].sch {
                        Some(c)
                    } else {
                        self.alt_cells.get(&(c, a)).copied()
                    }
                }),
                Target::Core(_) => None,
            };
            match resolved {
                Some(a) => target = Target::Cell(a),
                None => acc.item(
                    fid,
                    line,
                    Class::Review(Reason::Unsupported),
                    0,
                    format!("schematic={alt}"),
                ),
            }
        }
        let name = match props.get("name").map(str::trim).filter(|n| !n.is_empty()) {
            Some(n) => dotted_bus(n),
            None => {
                acc.item(
                    fid,
                    line,
                    Class::Review(Reason::Name),
                    0,
                    "instance without name",
                );
                format!("x{line}")
            }
        };
        let cellref = match target {
            Target::Core(m) => format!("core:{}", m.cell),
            Target::Cell(c) => {
                let cl = &self.cells[c as usize];
                if cl.lib == sc.lib {
                    format!("./{}", cl.name)
                } else {
                    format!("{}:{}", self.lib_names[usize::from(cl.lib)], cl.name)
                }
            }
        };
        let o = orient(f.instances.rot[row], f.instances.flip[row]);
        let [x, y] = f.instances.xy[row];
        doc.begin_record(RecordKind::Inst);
        doc.push_field(&ids.next(line, 0));
        doc.push_field(&cellref);
        doc.push_attr("at");
        doc.push_value_coord(x);
        doc.push_value_coord(y);
        doc.push_value(o.as_str());
        doc.push_attr("name");
        doc.push_value(&name);
        let mut used: Vec<&str> = vec!["name", "schematic", "spice_ignore", "lvs_ignore", "embed"];
        match props.get("spice_ignore").map(str::trim) {
            Some(v) if v.starts_with("tcleval(") => acc.item(
                fid,
                line,
                Class::Review(Reason::Tcl),
                0,
                format!("spice_ignore={v}"),
            ),
            Some(v @ ("open" | "short")) => {
                doc.push_attr("ignore");
                doc.push_value(v);
            }
            Some(v) if truthy(v) == Some(true) => {
                doc.push_attr("ignore");
                doc.push_value("true");
            }
            _ => {}
        }
        let iface = self.iface_of(target);
        let map = match target {
            Target::Core(m) => Some(m),
            Target::Cell(_) => None,
        };
        let xs_key = |core_name: &str| -> Vec<String> {
            match map {
                Some(m) => (0..xs_tpl.len())
                    .map(|i| xs_tpl.key(i))
                    .chain((0..props.len()).map(|i| props.key(i)))
                    .filter(|k| m.param(k) == core_name)
                    .map(str::to_owned)
                    .collect(),
                None => vec![core_name.to_owned()],
            }
        };
        for (pin, default) in &iface.implicit {
            used.push(pin);
            let v = props
                .get(pin)
                .or_else(|| xs_tpl.get(pin))
                .unwrap_or(default)
                .trim();
            if v.is_empty() || v.contains(['@', ' ', '\t', '\n', '{', '[']) {
                acc.item(
                    fid,
                    line,
                    Class::Review(Reason::Expression),
                    0,
                    format!("{pin}={v}"),
                );
                continue;
            }
            doc.push_attr("inherit");
            doc.push_value(pin);
            doc.push_value(v);
        }
        for (pname, pdefault, kind) in &iface.params {
            let keys = xs_key(pname);
            let inst_v = keys.iter().find_map(|k| props.get(k));
            keys.iter()
                .filter_map(|k| props.find(k))
                .for_each(|i| used.push(props.key(i)));
            let value = match map {
                Some(_) => inst_v.or_else(|| keys.iter().find_map(|k| xs_tpl.get(k))),
                None => inst_v
                    .filter(|v| collapse_ws(v) != collapse_ws(xs_tpl.get(pname).unwrap_or(""))),
            };
            let Some(v) = value else { continue };
            match override_text(v, *kind, known, &sc.parents) {
                Ok(text) if map.is_some() && text == *pdefault => {}
                Ok(text) => {
                    doc.push_attr(pname);
                    doc.push_value(&text);
                }
                Err(r) => acc.item(
                    fid,
                    line,
                    Class::Review(r.reason),
                    0,
                    format!("{pname}={}", r.detail),
                ),
            }
        }
        // Per-instance netlist text.
        let plan = match target {
            Target::Cell(c) => self.cells[c as usize]
                .sym
                .and_then(|s| self.plans[s as usize].as_ref()),
            Target::Core(_) => None,
        };
        let sym_dm = sym.iface.attrs.get("device_model");
        // A mapped device's *template* `device_model` is not emitted: goldens whose
        // `switch_ngspice` instances set another `model` (flip_flop_ngspice, led_driver,
        // solar_panel) carry no `.MODEL SW1` from its template.
        let dm = match map {
            Some(_) => props.get("device_model").or(sym_dm),
            None => props
                .get("device_model")
                .filter(|d| Some(*d) != xs_tpl.get("device_model").or(sym_dm)),
        };
        used.push("device_model");
        if let Some(dm) = dm.filter(|d| !d.trim().is_empty()) {
            // Emitted once per netlist, like xschem's device_model table: a bench code
            // block, or the component's schematic-impl body.
            match netlist_text(dm, self.opts.pdk_vars) {
                Ok(t) if sc.bench => {
                    let t = lines(&t);
                    if !out.codes.iter().any(|c| c.2 == t) {
                        let n = unique(
                            &format!("model_{name}"),
                            out.codes.iter().map(|c| c.0.as_str()),
                        );
                        out.codes.push((n, "spice".to_owned(), t));
                    }
                }
                Ok(t) => {
                    let t = lines(&t);
                    if !out.body.windows(t.len().max(1)).any(|w| w == t.as_slice()) {
                        out.body.extend(t);
                    }
                }
                Err(d) => acc.item(fid, line, Class::Review(Reason::Tcl), 0, d),
            }
        }
        for key in ["format", "lvs_format", "spice_sym_def", "spiceprefix"] {
            // An empty value is the same as an absent one in xschem.
            let Some(v) = props.get(key).filter(|v| !v.trim().is_empty()) else {
                used.push(key);
                continue;
            };
            used.push(key);
            if variant.is_some() && INSTANCE_TEXT.contains(&key) {
                continue; // the variant cell netlists it
            }
            let sym_v = match key {
                "spiceprefix" => plan
                    .and_then(|p| p.spiceprefix.as_deref())
                    .or_else(|| xs_tpl.get(key)),
                _ => sym.iface.attrs.get(key),
            };
            let is_param = iface.params.iter().any(|p| p.0 == key);
            // spiceprefix only matters where it was folded into the template.
            let inert = key == "spiceprefix" && plan.is_none_or(|p| p.spiceprefix.is_none());
            if !is_param && !inert && sym_v.map(collapse_ws) != Some(collapse_ws(v)) {
                acc.item(
                    fid,
                    line,
                    Class::Review(Reason::InstanceText),
                    0,
                    format!("{key}={v}"),
                );
            }
        }
        for i in 0..props.len() {
            let k = props.key(i);
            if !used.contains(&k) && acc.active {
                *acc.unused.entry(k.to_owned()).or_insert(0) += 1;
            }
        }
        acc.out(fid, 1);
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn emit_label(
        &self,
        doc: &mut Document,
        ids: &mut Ids,
        sc: &SchCtx,
        row: usize,
        s: u32,
        port: bool,
        out: &mut SchOut,
        acc: &mut Acc,
    ) {
        let fid = sc.file;
        let f = &self.srcs[fid as usize].file;
        let line = f.instances.line[row];
        let props = Attrs::parse(f.str(f.instances.props[row]));
        let sym = &self.syms[s as usize];
        let lab = props
            .get("lab")
            .or_else(|| sym.iface.template.get("lab"))
            .unwrap_or("")
            .trim();
        let Some(anchor) = self.anchor(f, row, s) else {
            acc.item(
                fid,
                line,
                Class::Review(Reason::Unsupported),
                1,
                "label symbol without one pin",
            );
            return;
        };
        if lab.is_empty() || lab.contains(['@', '\n']) || lab.contains("tcleval(") {
            acc.item(
                fid,
                line,
                Class::Review(Reason::Expression),
                1,
                format!("lab={lab}"),
            );
            return;
        }
        let o = orient(f.instances.rot[row], f.instances.flip[row]);
        let bound = sc
            .pins
            .iter()
            .find(|(xs, _)| xs == lab)
            .map(|(_, p)| p.as_str());
        let is_port = port && !sc.bench && bound.is_some();
        if port && !sc.bench && !is_port {
            acc.item(
                fid,
                line,
                Class::Review(Reason::Port),
                0,
                format!("{lab} is not a symbol pin; kept as a label"),
            );
        }
        doc.begin_record(if is_port {
            RecordKind::Port
        } else {
            RecordKind::Label
        });
        doc.push_field(&ids.next(line, 0));
        doc.push_field_coord(anchor[0]);
        doc.push_field_coord(anchor[1]);
        doc.push_field(o.as_str());
        let field = match (is_port, bound) {
            (true, Some(pin)) => pin.to_owned(),
            _ => dotted_bus(lab),
        };
        doc.push_field(&field);
        let global = props
            .get("global")
            .or_else(|| sym.iface.attrs.get("global"))
            .and_then(truthy)
            .unwrap_or(false);
        if global && !is_port {
            doc.push_attr("global");
            doc.push_value("true");
        }
        if is_port {
            out.ports.push(field);
        } else {
            out.labels.push((lab.to_owned(), anchor));
        }
        acc.out(fid, 1);
    }

    pub(super) fn emit_noconn(
        &self,
        doc: &mut Document,
        ids: &mut Ids,
        sc: &SchCtx,
        row: usize,
        s: u32,
        acc: &mut Acc,
    ) {
        let f = &self.srcs[sc.file as usize].file;
        let line = f.instances.line[row];
        let Some(at) = self.anchor(f, row, s) else {
            acc.item(
                sc.file,
                line,
                Class::Review(Reason::Unsupported),
                1,
                "noconn without one pin",
            );
            return;
        };
        doc.begin_record(RecordKind::Noconn);
        doc.push_field(&ids.next(line, 0));
        doc.push_field_coord(at[0]);
        doc.push_field_coord(at[1]);
        acc.out(sc.file, 1);
    }

    /// The schematic position of a one-pin symbol's pin for instance `row`.
    pub(super) fn anchor(&self, f: &XschemFile, row: usize, s: u32) -> Option<[i32; 2]> {
        let pins = &self.syms[s as usize].iface.pins;
        let [p] = pins.center.as_slice() else {
            return None;
        };
        place(
            *p,
            orient(f.instances.rot[row], f.instances.flip[row]),
            f.instances.xy[row],
        )
    }

    /// Renders a netlist-text instance (`code.sym`, `simulator_commands`, pinless
    /// devices such as `k.sym`) to `(lang, text)`.
    pub(super) fn render_code(&self, s: u32, props: &Attrs) -> Result<(String, String), Review> {
        let sym = &self.syms[s as usize];
        let tpl = &sym.iface.template;
        let get = |k: &str| props.get(k).or_else(|| tpl.get(k));
        let fmt = props
            .get("format")
            .or_else(|| sym.iface.attrs.get("format"))
            .unwrap_or("@value");
        let tcl_err = |detail: String| review(Reason::Tcl, detail);
        if fmt.contains("sim_is_") {
            let lang = get("simulator")
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .unwrap_or("ngspice");
            let text =
                netlist_text(get("value").unwrap_or(""), self.opts.pdk_vars).map_err(tcl_err)?;
            return Ok((lang.to_owned(), text));
        }
        let stem = stem_of(&self.srcs[sym.src as usize].path);
        let mut text = String::new();
        let mut last = 0;
        for t in at_tokens(fmt) {
            push_unescaped(&mut text, &fmt[last..t.offset as usize]);
            let value = match t.name {
                "symname" => Some(stem),
                n => get(n),
            };
            match (t.kind, value) {
                (crate::xschem::attr::AtKind::Percent, None) => text.push_str(t.name),
                (_, Some(v)) => text.push_str(v),
                _ => {}
            }
            last = t.offset as usize
                + 1
                + t.name.len()
                + usize::from(matches!(
                    t.kind,
                    crate::xschem::attr::AtKind::PinNet | crate::xschem::attr::AtKind::PinAttr
                ));
        }
        push_unescaped(&mut text, &fmt[last.min(fmt.len())..]);
        let text = netlist_text(&text, self.opts.pdk_vars).map_err(tcl_err)?;
        Ok(("spice".to_owned(), text))
    }

    /// Lowercase words of the attribute values of schematic `x`'s cell and netlist-text
    /// instances, and of its `S {}` text: the names its netlist can mention (graphs and
    /// launchers excluded).
    pub(super) fn value_words(&self, x: u32) -> HashSet<String> {
        let f = &self.srcs[x as usize].file;
        let mut words = HashSet::new();
        let mut add = |text: &str| {
            words.extend(
                text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                    .filter(|w| !w.is_empty())
                    .map(str::to_ascii_lowercase),
            );
        };
        for row in 0..f.instances.props.len() {
            let class = self
                .resolve(x, f.str(f.instances.symref[row]))
                .and_then(|src| self.sym_of_src[src as usize])
                .map(|s| self.syms[s as usize].class);
            if matches!(class, Some(SymClass::Cell | SymClass::Code)) {
                let a = Attrs::parse(f.str(f.instances.props[row]));
                (0..a.len()).for_each(|i| add(a.value(i)));
            }
        }
        let g = &f.globals;
        for i in (0..g.tag.len()).filter(|&i| g.tag[i] == GlobalTag::S) {
            add(f.str(g.text[i]));
        }
        words
    }
}

pub(super) struct SchCtx {
    pub(super) file: u32,
    pub(super) lib: u16,
    pub(super) bench: bool,
    /// The cell's parameters (empty for a bench): overrides referencing them become
    /// expressions or holes evaluated in this cell's scope.
    pub(super) parents: Vec<String>,
    /// Interface pin names (incl. inherited) a port may bind.
    pub(super) pins: Vec<(String, String)>,
}

#[derive(Default)]
pub(super) struct SchOut {
    pub(super) vars: Vec<(String, String)>,
    /// Bench-copy variables taken from the component's defaults: a bench `.param` of
    /// the same name replaces them (that is the value xschem's netlist defines).
    pub(super) default_vars: Vec<String>,
    pub(super) codes: Vec<(String, String, Vec<String>)>,
    pub(super) ports: Vec<String>,
    pub(super) labels: Vec<(String, [i32; 2])>,
    /// Model text for the schematic impl's `body` (component instances' device_model).
    pub(super) body: Vec<String>,
    pub(super) bbox: Option<[i32; 2]>,
}

impl SchOut {
    pub(super) fn extend_bbox(&mut self, p: [i32; 2]) {
        self.bbox = Some(match self.bbox {
            Some([x, y]) => [x.min(p[0]), y.min(p[1])],
            None => p,
        });
    }

    pub(super) fn bbox_min(&self) -> [i32; 2] {
        let [x, y] = self.bbox.unwrap_or([0, 0]);
        [x.saturating_sub(100_000), y.saturating_sub(100_000)]
    }
}
