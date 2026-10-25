//! Symbol interfaces (spec `import.md` §2): pins, params, impls and formats, device limits.

use super::*;

impl<'o> Tables<'o> {
    // ------------------------------------------------------------------ 3. plan symbols

    pub(super) fn plan_symbols(&mut self, acc: &mut Acc) {
        for s in 0..self.syms.len() {
            if self.syms[s].class != SymClass::Cell {
                continue;
            }
            if let Some(map) = self.core_map(s) {
                self.targets[s] = Some(Target::Core(map));
                continue;
            }
            let plan = self.plan_symbol(s, acc);
            self.sym_ifaces[s] = Some(iface_of_plan(&plan));
            self.plans[s] = Some(plan);
        }
    }

    /// The core row for symbol `s` when it is a mapped `devices/` symbol whose pins
    /// (renamed) sit exactly where the core cell's pins are.
    pub(super) fn core_map(&mut self, s: usize) -> Option<&'static DeviceMap> {
        let path = &self.srcs[self.syms[s].src as usize].path;
        if !self.opts.map_core || file_of(dir_of(path)) != "devices" {
            return None;
        }
        let map = devices::device_map(stem_of(path))?;
        let iface = self.core_ifaces.entry(map.cell).or_insert_with(|| {
            iface_of_cell_bytes(schemify_doc::core_file(map.cell).expect("mapped cell exists"))
        });
        let sym = &self.syms[s].iface;
        let same = sym.pins.name.len() == iface.pins.len()
            && (0..sym.pins.name.len()).all(|p| {
                let name = map.pin(sym.pin_name(p));
                iface
                    .pins
                    .iter()
                    .any(|(n, xy)| n == name && *xy == sym.pins.center[p])
            });
        same.then_some(map)
    }

    pub(super) fn k_line(&self, s: usize) -> u32 {
        let g = &self.srcs[self.syms[s].src as usize].file.globals;
        (0..g.tag.len())
            .find(|&i| matches!(g.tag[i], GlobalTag::K | GlobalTag::G))
            .map_or(0, |i| g.line[i])
    }

    pub(super) fn plan_symbol(&mut self, s: usize, acc: &mut Acc) -> Plan {
        let file_id = self.syms[s].src;
        let line = self.k_line(s);
        let sym = &self.syms[s];
        let iface = &sym.iface;
        let tpl = &iface.template;
        let attrs = &iface.attrs;
        let fmt = attrs.get("format");
        let mut reviews: Vec<Review> = Vec::new();
        let mut tokens: Vec<String> = Vec::new();
        let implicit = implicit_nets(iface);
        // Pins in netlist order.
        let mut pins: Vec<PinOut> = Vec::new();
        for p in 0..iface.pins.name.len() {
            let mut name = iface.pin_name(p).trim().to_owned();
            if name.is_empty() || name.contains(char::is_whitespace) {
                reviews.push(review(Reason::Name, format!("pin {p} has no usable name")));
                name = format!("pin{p}");
            }
            // A repeated name is the same port at a second position (xschem shorts them:
            // goldens `doublepin`, `xcross`, `short.sym`): an alias pin `<base>_<k>[…]`.
            let xs = name.clone();
            let alias = pins.iter().any(|q| q.xs == name);
            if alias {
                let (base, bus) = name.split_at(name.find('[').unwrap_or(name.len()));
                let mut k = 2;
                while pins.iter().any(|q| q.name == format!("{base}_{k}{bus}")) {
                    k += 1;
                }
                name = format!("{base}_{k}{bus}");
            }
            let dir = match iface.pins.dir[p] {
                PinDir::In => "in",
                PinDir::Out => "out",
                _ => "inout",
            };
            pins.push(PinOut {
                xs,
                name,
                dir,
                xy: iface.pins.center[p],
                inherit: None,
                alias,
            });
        }
        // A format that names its pins with `@@P` instead of `@pinlist` fixes the port
        // order: xschem's `.subckt` lists ports in that order (IHP std cells: `@@Y @@A`
        // over symbol order A, Y). Pins it does not name keep their order after.
        if let Some(f) = fmt.filter(|f| !f.contains("@pinlist")) {
            let mut order: Vec<String> = Vec::new();
            for t in crate::xschem::attr::at_tokens(f)
                .filter(|t| t.kind == crate::xschem::attr::AtKind::PinNet)
            {
                if pins.iter().any(|q| q.name == t.name) && !order.iter().any(|o| o == t.name) {
                    order.push(t.name.to_owned());
                }
            }
            let rank = |q: &PinOut| {
                order
                    .iter()
                    .position(|o| *o == q.name)
                    .unwrap_or(order.len())
            };
            pins.sort_by_key(rank); // stable: unnamed pins keep file order
        }
        for (k, net) in &implicit {
            if pins.iter().any(|q| &q.name == k) {
                reviews.push(review(
                    Reason::Name,
                    format!("net parameter {k} is also a pin"),
                ));
                continue;
            }
            pins.push(PinOut {
                name: k.clone(),
                xs: k.clone(),
                dir: "inout",
                xy: [0, 0],
                inherit: Some(net.clone()),
                alias: false,
            });
        }
        // Parameters from the template.
        let spiceprefix = tpl.get("spiceprefix").map(|v| v.trim().to_owned());
        let avail: Vec<&str> = attrs
            .get("available_models")
            .unwrap_or("")
            .split_whitespace()
            .collect();
        let keys: Vec<String> = (0..tpl.len()).map(|i| tpl.key(i).to_owned()).collect();
        let mut params: Vec<ParamOut> = Vec::new();
        let mut dropped_keys: Vec<String> = Vec::new();
        for i in 0..tpl.len() {
            let k = tpl.key(i);
            let skip = matches!(
                k,
                "name" | "spiceprefix" | "device_model" | "spice_ignore" | "lvs_ignore"
            ) || implicit.iter().any(|(n, _)| n == k);
            if skip {
                continue;
            }
            if params.iter().any(|p| p.name == k) {
                continue; // xschem uses the first occurrence
            }
            if !is_ident(k) || !k.starts_with(|c: char| c.is_ascii_alphabetic()) {
                // Not a name any `@token` can reach (`+` continuation artefacts, `300`).
                dropped_keys.push(k.to_owned());
                continue;
            }
            let reserved = template::RESERVED.contains(&k)
                || matches!(k, "at" | "ignore" | "inherit")
                || implicit.iter().any(|(n, _)| n == k);
            if reserved {
                reviews.push(review(
                    Reason::Name,
                    format!("parameter `{k}` cannot be represented"),
                ));
                continue;
            }
            let v = tpl.value(i);
            let (mut kind, mut default, review) = infer(v, &avail, &keys);
            if kind == Kind::Number && self.textual.contains(&(s as u32, k.to_owned())) {
                (kind, default) = (Kind::Str, literal(v.trim()));
            }
            reviews.extend(review);
            params.push(ParamOut {
                name: k.to_owned(),
                default,
                kind,
                choices: if kind == Kind::Choice {
                    avail.iter().map(|a| a.to_string()).collect()
                } else {
                    Vec::new()
                },
                attrs: Vec::new(),
            });
        }
        // Templates per dialect.
        // Geometric pins named like a parameter are renamed (templates resolve pin names
        // before parameters; xschem keeps the two namespaces apart).
        let mut rename: Vec<(String, String)> = Vec::new();
        for k in 0..pins.len() {
            if pins[k].inherit.is_none() && params.iter().any(|p| p.name == pins[k].name) {
                let mut n = format!("{}_pin", pins[k].xs);
                while pins.iter().any(|q| q.name == n) || params.iter().any(|q| q.name == n) {
                    n.push('_');
                }
                rename.push((pins[k].xs.clone(), n.clone()));
                pins[k].name = n;
            }
            // `D[5..0]` is the bracket-less bundle `D5,…,D0` (as in labels).
            let bundle = dotted_bus(&pins[k].name);
            if bundle != pins[k].name && !pins[k].alias {
                rename.retain(|(x, _)| *x != pins[k].xs);
                rename.push((pins[k].xs.clone(), bundle.clone()));
                pins[k].name = bundle;
            }
        }
        let pin_names: Vec<String> = pins
            .iter()
            .filter(|p| p.inherit.is_none())
            .map(|p| p.xs.clone())
            .collect();
        let implicit_names: Vec<String> = implicit.iter().map(|(n, _)| n.clone()).collect();
        let subcircuit = sym.ty == "subcircuit";
        let mut leaf_formats = Vec::new();
        let mut schematic_formats = Vec::new();
        let mut spice_failed = false;
        for (key, dialect) in [
            ("format", "spice"),
            ("lvs_format", "cdl"),
            ("spectre_format", "spectre"),
            ("verilog_format", "verilog"),
        ] {
            let Some(f) = attrs.get(key).filter(|f| !f.is_empty()) else {
                continue;
            };
            if f.trim().is_empty() {
                // A blank format (`lvs_format=" "`) netlists nothing (golden
                // sky130_mismatch: the instance is absent from the LVS netlist).
                leaf_formats.push((dialect, String::new()));
                schematic_formats.push((dialect, String::new()));
                continue;
            }
            for schematic in [false, true] {
                let names: Vec<String> = params.iter().map(|p| p.name.clone()).collect();
                let ctx = FormatCtx {
                    params: &names,
                    pins: &pin_names,
                    implicit: &implicit_names,
                    spiceprefix: spiceprefix.as_deref(),
                    schematic,
                    rename: &rename,
                };
                match template::convert_format(f, &ctx) {
                    Ok(c) => {
                        for u in &c.unknown {
                            if !params.iter().any(|p| &p.name == u) {
                                tokens
                                    .push(format!("@{u} (declared as an empty string parameter)"));
                                params.push(ParamOut {
                                    name: u.clone(),
                                    default: String::new(),
                                    kind: Kind::Str,
                                    choices: Vec::new(),
                                    attrs: Vec::new(),
                                });
                            }
                        }
                        if !schematic {
                            tokens
                                .extend(c.dropped.iter().map(|d| format!("@{d} (rendered empty)")));
                        }
                        // Re-convert when unknown tokens were declared, so the template sees them.
                        let text = if c.unknown.is_empty() {
                            c.text
                        } else {
                            let names: Vec<String> =
                                params.iter().map(|p| p.name.clone()).collect();
                            template::convert_format(
                                f,
                                &FormatCtx {
                                    params: &names,
                                    ..ctx
                                },
                            )
                            .map(|c| c.text)
                            .unwrap_or(c.text)
                        };
                        if schematic {
                            schematic_formats.push((dialect, text));
                        } else {
                            leaf_formats.push((dialect, text));
                        }
                    }
                    Err(r) => {
                        if !schematic {
                            spice_failed |= dialect == "spice";
                            reviews.push(review(r.reason, format!("{key}: {}", r.detail)));
                        }
                    }
                }
            }
        }
        // Impl language and body.
        let device_model = attrs
            .get("device_model")
            .or_else(|| tpl.get("device_model"))
            .filter(|d| !d.trim().is_empty());
        let lang = match device_model {
            Some(d) if d.contains("pre_osdi") => "veriloga",
            Some(d) if d.contains("d_cosim") => "verilog",
            _ => "spice",
        };
        let mut body = Vec::new();
        for text in [
            device_model,
            attrs.get("spice_sym_def").filter(|d| !d.trim().is_empty()),
        ]
        .into_iter()
        .flatten()
        {
            match netlist_text(text, self.opts.pdk_vars) {
                Ok(t) => body.extend(lines(&t)),
                Err(detail) => reviews.push(review(Reason::Tcl, detail)),
            }
        }
        // `spice_sym_def` is the cell's netlist text (it replaces the schematic in xschem),
        // and xschem orders `@pinlist` like its `.subckt` line, inline or in the file an
        // `.include` names (golden tb_symbol_include): spell the pins out in that order.
        let sym_def = attrs
            .get("spice_sym_def")
            .filter(|d| !d.trim().is_empty())
            .and_then(|d| netlist_text(d, self.opts.pdk_vars).ok())
            .map(|t| lines(&t));
        let stem = stem_of(&self.srcs[sym.src as usize].path).to_owned();
        let ports = sym_def
            .as_deref()
            .and_then(|l| self.subckt_ports(sym.src, l, &stem));
        if let Some(ports) = ports {
            match pin_order(&pins, &ports) {
                Some(holes) => {
                    for (_, t) in &mut leaf_formats {
                        *t = t.replace("{pins}", &holes);
                    }
                }
                None => reviews.push(review(
                    Reason::PinToken,
                    format!(
                        "spice_sym_def .subckt ports `{}` do not match the pins",
                        ports.join(" ")
                    ),
                )),
            }
        }
        let ignore_src = attrs
            .get("spice_ignore")
            .or_else(|| tpl.get("spice_ignore"));
        let ignore = match ignore_src.map(str::trim) {
            Some(v) if v.starts_with("tcleval(") => {
                reviews.push(review(Reason::Tcl, format!("spice_ignore={v}")));
                None
            }
            Some(v @ ("open" | "short")) => Some(v.to_owned()),
            Some(v) if truthy(v) == Some(true) => Some("true".to_owned()),
            _ => None,
        };
        let has_spice = leaf_formats.iter().any(|(d, _)| *d == "spice");
        // Without a usable format xschem does not netlist the device: never guess one.
        let leaf_ignore = if spice_failed || (!has_spice && !subcircuit) {
            Some("true".to_owned())
        } else {
            ignore.clone()
        };
        // Device limits (D14).
        let xs_params: Vec<(String, String)> = (0..tpl.len())
            .map(|i| (tpl.key(i).to_owned(), tpl.value(i).to_owned()))
            .collect();
        let file_name = file_of(&self.srcs[sym.src as usize].path).to_owned();
        let (lims, lim_review) = limits::derive(&limits::Inputs {
            params: &xs_params,
            format: fmt,
            drc: attrs.get("drc"),
            file_name: &file_name,
            models: self.opts.models,
            procs: self.opts.drc,
            grid_pm: self.opts.grid_pm,
        });
        for l in lims {
            if let Some(p) = params
                .iter_mut()
                .find(|p| p.name == l.param && p.kind == Kind::Number)
            {
                p.attrs.push((l.attr, l.expr));
            }
        }
        reviews.extend(lim_review.into_iter().map(|d| review(Reason::Drc, d)));
        // CDF choices.
        if let Some(cdf) = self.opts.cdf {
            apply_cdf(cdf, stem_of(&file_name), &mut params, &mut reviews);
        }
        let prefix = tpl
            .get("name")
            .map(|n| {
                n.trim()
                    .chars()
                    .take_while(char::is_ascii_alphabetic)
                    .collect::<String>()
            })
            .filter(|p| !p.is_empty());
        for r in reviews {
            acc.item(file_id, line, Class::Review(r.reason), 0, r.detail);
        }
        for t in tokens {
            acc.item(file_id, line, Class::Token, 0, t);
        }
        for k in dropped_keys {
            acc.item(
                file_id,
                line,
                Class::Dropped(Drop::Malformed),
                0,
                format!("template key `{k}`"),
            );
        }
        assert!(pins.iter().all(|p| !p.name.is_empty()), "pins are named");
        assert!(
            (1..pins.len()).all(|i| !pins[..i].iter().any(|q| q.name == pins[i].name)),
            "pin names are unique"
        );
        assert!(
            (1..params.len()).all(|i| !params[..i].iter().any(|q| q.name == params[i].name)),
            "param names are unique"
        );
        Plan {
            pins,
            params,
            leaf: ImplOut {
                name: lang,
                lang,
                formats: leaf_formats,
                ignore: leaf_ignore,
                body,
            },
            ignore,
            schematic_formats,
            subcircuit,
            sym_def: sym_def.is_some(),
            prefix,
            spiceprefix,
        }
    }

    /// The port names of the `.subckt` that netlist text `text` of symbol source `src`
    /// defines, or of the file its `.include` names (a side input, resolved beside the
    /// symbol); the subckt named `stem` (case-insensitively) if several, else the first.
    /// `None` when there is no such line or the file is not an input.
    pub(super) fn subckt_ports(
        &self,
        src: u32,
        text: &[String],
        stem: &str,
    ) -> Option<Vec<String>> {
        let mut found = spice::subckt_lines(&text.join("\n"));
        if found.is_empty() {
            let inc = text.iter().find_map(|l| {
                let w: Vec<&str> = l.split_whitespace().collect();
                (w.len() == 2 && w[0].eq_ignore_ascii_case(".include"))
                    .then(|| w[1].trim_matches(['"', '\'']))
            })?;
            let path = join(dir_of(&self.srcs[src as usize].path), inc);
            found = spice::subckt_lines(self.side.get(&path)?);
        }
        let pick = found
            .iter()
            .position(|(n, _)| n.eq_ignore_ascii_case(stem))
            .unwrap_or(0);
        found.into_iter().nth(pick).map(|(_, ports)| ports)
    }

    pub(super) fn iface_of(&self, t: Target) -> &Iface {
        match t {
            Target::Core(m) => &self.core_ifaces[m.cell],
            Target::Cell(c) => {
                let s = self.cells[c as usize]
                    .sym
                    .expect("cell targets have a symbol");
                self.sym_ifaces[s as usize]
                    .as_ref()
                    .expect("cell symbols are planned")
            }
        }
    }
}
