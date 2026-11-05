//! A cell's schematic → its `schematic` cellview (spec `interop.md` §6.2 schematic rows,
//! §6.4 edits): instances with their params and overrides, pins, wires, labels and notes.

use super::*;

/// Identifier classes of an override written in cell `c` (the instance's parent).
fn parent_scope(c: &Cell) -> impl FnMut(&str) -> Name + '_ {
    let bench = c.kind() == FileKind::Shbench;
    move |n: &str| {
        if c.records(RecordKind::Param).any(|r| c.field(r, 0) == n) {
            Name::ParentParam
        } else if let Some(r) = c.records(RecordKind::Var).find(|r| c.field(*r, 0) == n) {
            if bench {
                Name::BenchVar
            } else {
                Name::CellVar(c.field(r, 1).to_owned())
            }
        } else {
            Name::Unknown
        }
    }
}

/// The source waveform cell (index into the device row's `alib`) and params of a core
/// source instance's effective `value`.
fn source_props(
    value: &str,
    current: bool,
    c: &Cell,
    inst: &str,
    notes: &mut Notes,
) -> devices::Source {
    // A `{expr}` hole becomes its §3.4 value (bench vars bare: ADE design variables); a hole
    // without an AEL form makes the whole text unparsable, never a half-translated CDF.
    let mut reviews: Vec<(&'static str, String)> = Vec::new();
    let parsed = devices::parse_source_with(value, current, |e| {
        let t = ael::to_ael(e, parent_scope(c)).ok()?;
        if let Some(why) = t.review {
            reviews.push((why, format!("{inst}: `{e}` → `{}`", t.text)));
        }
        Some(t.text)
    });
    if parsed.is_some() {
        for (why, detail) in reviews {
            notes.review(why, &c.path, detail);
        }
    }
    parsed.unwrap_or_else(|| {
        notes.review(
            "source",
            &c.path,
            format!(
                "{inst}: `{value}` is not SPICE source text with number or {{expr}} values; \
                 sent as the DC value"
            ),
        );
        devices::Source {
            wave: 0,
            params: vec![(
                (if current { "idc" } else { "vdc" }).into(),
                value.to_owned(),
            )],
        }
    })
}

pub(super) fn schematic_out(
    d: &Design,
    ci: usize,
    plan: &Plan,
    m: &Mapping,
    ids: &mut Ids,
    notes: &mut Notes,
) -> Cv {
    let c = &d.cells[ci];
    let g = geom(d, ci);
    let n = nets::extract(d, ci, &g);
    let mut cv = Cv {
        view: "schematic".into(),
        vtype: "schematic".into(),
        upi: UPI,
        ..Cv::default()
    };
    // §6.4: re-mastered instances.
    let mut todo = Vec::new();
    // Instances placed by pin-pattern alignment (§6.4): index → the new master's origin.
    let mut aligned: BTreeMap<usize, [i32; 2]> = BTreeMap::new();
    for k in 0..g.inst_id.len() {
        let mi = g.inst_master[k] as usize;
        if !plan.remastered(mi, m) {
            continue;
        }
        let pins = sym_pins(&d.cells[mi]);
        let names: Vec<String> = match plan.how[mi] {
            How::Map(row) => pins
                .name
                .iter()
                .map(|p| {
                    m.maps[row as usize]
                        .pins
                        .iter()
                        .find(|x| &x.0 == p)
                        .map_or(String::new(), |x| x.1.clone())
                })
                .collect(),
            How::AnalogLib(row) => {
                let dv = &devices::DEVICES[row as usize];
                (0..pins.name.len())
                    .map(|k| dv.alib_pins.get(k).map_or(String::new(), |s| (*s).into()))
                    .collect()
            }
            _ => unreachable!("only map rows and analogLib re-master"),
        };
        // Known geometry: a mirror of the target cell in the Design.
        let target = (plan.lib[mi].as_str(), plan.cell[mi].as_str());
        let new_pins = d
            .cells
            .iter()
            .find(|x| map::mirror_of(x).is_some_and(|(l, cl)| (l.as_str(), cl.as_str()) == target))
            .map(|x| {
                // Its inherited pins are drawn nowhere: the loader connects them (netSet).
                let sp = sym_pins(x);
                (0..sp.name.len())
                    .filter(|&i| sp.inherit[i].is_none())
                    .map(|i| (vname(&sp.name[i]), sp.at[i]))
                    .collect::<Vec<_>>()
            })
            // analogLib's pins, as measured (devices::alib_pins).
            .or_else(|| match plan.how[mi] {
                How::AnalogLib(_) => devices::alib_pins(target.1)
                    .map(|v| v.iter().map(|(t, p)| ((*t).to_owned(), *p)).collect()),
                _ => None,
            });
        let pairs: Vec<(usize, String)> = names
            .into_iter()
            .enumerate()
            .filter(|(_, n)| !n.is_empty())
            .collect();
        // Pin-pattern alignment (§6.4): the new master goes where its pins land on the
        // drawn ones, in the instance's orientation; the pins that still miss get stubs.
        let at = new_pins.as_ref().and_then(|np| {
            align::fit_instance(&pins.at, &pairs, np, g.inst_orient[k], g.inst_at[k])
        });
        if let Some(a) = at {
            aligned.insert(k, a);
        }
        todo.push(Remaster {
            inst: k,
            pins: pairs,
            new_pins,
            at,
        });
        // A core cell on its §6.5 analogLib master is the export's default, not news; a
        // mapping-file row is reported.
        if matches!(plan.how[mi], How::Map(_)) {
            notes.add(
                "remapped",
                &c.path,
                format!(
                    "{}: {} → {}/{}",
                    g.inst_name[k], d.cells[mi].name, target.0, target.1
                ),
            );
        }
    }
    let edits = remap::apply(&g, &n, &todo);
    // Wires (after the edits) in Schemify coordinates, for gluing labels.
    // (wire ref, segment) of every wire label can glue to.
    let mut segs: Vec<(u32, [i32; 4])> = Vec::new();
    for w in 0..g.wire.len() {
        if edits.wire_dead.binary_search(&w).is_ok() {
            continue;
        }
        let mut seg = g.wire[w];
        for (_, end, p) in edits.wire_end.iter().filter(|x| x.0 == w) {
            seg[end * 2] = p[0];
            seg[end * 2 + 1] = p[1];
        }
        let a = conv(
            [seg[0], seg[1]],
            &format!("wire {}", g.wire_id[w]),
            c,
            notes,
        );
        let b = conv(
            [seg[2], seg[3]],
            &format!("wire {}", g.wire_id[w]),
            c,
            notes,
        );
        let r = ids.take();
        segs.push((r, seg));
        // A bus as wide as Virtuoso drew it (`vwidth <w>`, §7.5), else export's.
        let width = c
            .attr(g.wire_rec[w], "vwidth")
            .and_then(|v| parse_coord(v).ok());
        cv.wires.push(Wire {
            r,
            pts: vec![a, b],
            width: if g.wire_bus[w] {
                width.unwrap_or(BUS_WIDTH)
            } else {
                0
            },
        });
        id_prop(&mut cv, r, &g.wire_id[w]);
    }
    // Instances.
    let mut names: Vec<String> = Vec::new();
    let mut inst_ref = Vec::new();
    let mut stashed: BTreeMap<usize, bool> = BTreeMap::new();
    for k in 0..g.inst_id.len() {
        let mi = g.inst_master[k] as usize;
        let r = ids.take();
        inst_ref.push(r);
        let rec = g.inst_rec[k];
        let name = profile::inst_to_virtuoso(&g.inst_name[k]).unwrap_or_else(|| {
            notes.review(
                "name",
                &c.path,
                format!("instance name `{}` kept verbatim", g.inst_name[k]),
            );
            g.inst_name[k].clone()
        });
        names.push(name.clone());
        let mut cell = plan.cell[mi].clone();
        let master = &d.cells[mi];
        let src = match plan.how[mi] {
            How::Core | How::AnalogLib(_) => {
                devices::by_core(&master.name).and_then(|dv| dv.source)
            }
            _ => None,
        };
        let at = conv(
            aligned.get(&k).copied().unwrap_or(g.inst_at[k]),
            &format!("instance {}", g.inst_name[k]),
            c,
            notes,
        );
        id_prop(&mut cv, r, &g.inst_id[k]);
        match c.attr(rec, "ignore") {
            Some("true") => cv.props.push(Prop::string(r, "nlAction", "ignore")),
            Some(o) => notes.review(
                "expression",
                &c.path,
                format!("{}: `ignore {o}` has no Virtuoso form", g.inst_name[k]),
            ),
            None => {}
        }
        let row = match plan.how[mi] {
            How::Map(rw) if !m.maps[rw as usize].same => Some(&m.maps[rw as usize]),
            _ => None,
        };
        let dev = match plan.how[mi] {
            How::AnalogLib(rw) => Some(&devices::DEVICES[rw as usize]),
            _ => None,
        };
        let mut value_override: Option<String> = None;
        let fresh = derived_digest(c, rec, master);
        for a in c.doc.attrs(rec) {
            let key = c.doc.attr_key(a);
            let val = c.doc.attr_value(a, 0).unwrap_or("");
            // A value Virtuoso derived goes back only while its inputs are the ones it came
            // from (D20 as amended 2026-10-15); a stale one is left to the callbacks.
            let (key, val) = match key {
                "cached" => match (c.doc.attr_value(a, 1), c.doc.attr_value(a, 2)) {
                    (Some(v), Some(d)) if fresh.as_deref() == Some(d) => (val, v),
                    _ => continue,
                },
                _ => (key, val),
            };
            match key {
                "at" | "name" | "ignore" => continue,
                // A property Schemify has no param for, as Virtuoso had it; an override of
                // the same name wins.
                "vprop" => {
                    let (Some(ty), Some(v)) = (c.doc.attr_value(a, 1), c.doc.attr_value(a, 2))
                    else {
                        continue;
                    };
                    if c.attr(rec, val).is_none() {
                        cv.props.push(Prop::new(r, val, ty, v));
                    }
                    continue;
                }
                "vtd" => {
                    if let Some(t) = text_display(c, a, r, at) {
                        cv.tds.push(t);
                    }
                    continue;
                }
                "inherit" => {
                    let pin = val;
                    let net = c.doc.attr_value(a, 1).unwrap_or("");
                    // A netSet-typed property: the type the netlister reads as an
                    // inherited-connection override (G13).
                    cv.props.push(Prop::new(
                        r,
                        &vname(pin),
                        "netSet",
                        net_to_virtuoso(net, false).0,
                    ));
                    continue;
                }
                _ => {}
            }
            if src.is_some() && key == "value" {
                value_override = Some(val.to_owned());
                continue;
            }
            // Renames (map rows, analogLib rows).
            let (pname, scale) = if let Some(row) = row {
                if row.drops.iter().any(|x| x == key) {
                    continue;
                }
                match row.params.iter().find(|p| p.0 == key) {
                    Some(p) => (p.1.clone(), p.2.clone()),
                    None => (key.to_owned(), None),
                }
            } else if let Some(dv) = dev {
                match dv.params.iter().find(|p| p.0 == key) {
                    Some(p) => (p.1.to_owned(), None),
                    // An override equal to the core default, or of a param that only steers
                    // the ngspice netlist (`savecurrent`: Spectre saves through the test),
                    // loses nothing by staying behind.
                    None if NGSPICE_ONLY.contains(&key)
                        || master
                            .records(RecordKind::Param)
                            .any(|q| master.field(q, 0) == key && master.field(q, 1) == val) =>
                    {
                        continue;
                    }
                    None => {
                        notes.add(
                            "remapped",
                            &c.path,
                            format!(
                                "{}: param {key} has no analogLib counterpart; not sent",
                                g.inst_name[k]
                            ),
                        );
                        continue;
                    }
                }
            } else {
                (key.to_owned(), None)
            };
            let kind = master
                .records(RecordKind::Param)
                .find(|p| master.field(*p, 0) == key)
                .map(|p| master.attr(p, "kind").unwrap_or("number"));
            let val_scaled;
            let val = match &scale {
                Some(f) => {
                    let e = format!("({val})*({f})");
                    val_scaled = ael::fold_numeric(&e).unwrap_or(e);
                    val_scaled.as_str()
                }
                None => val,
            };
            if let Some(p) = override_prop(c, r, &pname, kind, val, &g.inst_name[k], notes) {
                // Virtuoso stores no CDF value equal to its default (storeDefault "no"), so an
                // override equal to the master's default is not sent: the stash (§7.3) keeps
                // it as Schemify-only data and a round trip restores it.
                let default = master
                    .records(RecordKind::Param)
                    .find(|q| master.field(*q, 0) == key)
                    .map(|q| master.field(q, 1).to_owned());
                let same = default.is_some_and(|dv| {
                    let dv = match &scale {
                        Some(f) => {
                            let e = format!("({dv})*({f})");
                            ael::fold_numeric(&e).unwrap_or(e)
                        }
                        None => dv,
                    };
                    override_prop(c, r, &pname, kind, &dv, "", &mut Notes::default())
                        .is_some_and(|d| d.ty == p.ty && d.value == p.value)
                });
                if !same {
                    cv.props.push(p);
                }
            }
        }
        if let Some(current) = src {
            let effective = value_override.clone().unwrap_or_else(|| {
                master
                    .records(RecordKind::Param)
                    .find(|p| master.field(*p, 0) == "value")
                    .map_or("0".into(), |p| master.field(p, 1).to_owned())
            });
            // Only an override is sent to schemify_core (D40); analogLib gets the value.
            if value_override.is_some() || dev.is_some() {
                let sp = source_props(&effective, current, c, &g.inst_name[k], notes);
                if let Some(dv) = dev {
                    cell = dv.alib[sp.wave].into();
                } else {
                    cv.props
                        .push(Prop::string(r, "srcType", devices::WAVEFORMS[sp.wave].1));
                }
                for (pn, pv) in sp.params {
                    let ty = if pn == "tvpairs" { "int" } else { "string" };
                    cv.props.push(Prop::new(r, &pn, ty, pv));
                }
            }
        }
        if let Some(row) = row {
            for (p, v) in &row.sets {
                cv.props.push(Prop::string(r, p, v.clone()));
            }
        }
        // A design-library callback's params, which the CDF going out as imported derives
        // only when its callbacks run (§6.3).
        if matches!(plan.how[mi], How::Design)
            && *stashed
                .entry(mi)
                .or_insert_with(|| matches!(super::records::cdf_as_imported(master), Some(Some(_))))
        {
            for (n, v) in super::cdf::stashed_computed(master, |n| c.attr(rec, n)) {
                cv.props.push(Prop::string(r, &n, v));
            }
        }
        cv.insts.push(Inst {
            r,
            name,
            lib: plan.lib[mi].clone(),
            cell,
            view: plan.view[mi].clone(),
            xy: at,
            orient: profile::orient_to_virtuoso(g.inst_orient[k]).into(),
        });
    }
    // Labels: the cell's, then the edits'.
    let pin_points: Vec<([i32; 2], [i32; 2])> = (0..g.inst_id.len())
        .flat_map(|k| {
            let ps = sym_pins(&d.cells[g.inst_master[k] as usize]);
            let (o, at) = (g.inst_orient[k], g.inst_at[k]);
            ps.at
                .iter()
                .zip(&ps.inherit)
                .filter(|(_, inh)| inh.is_none())
                .filter_map(move |(p, _)| crate::design::place(*p, o, at).map(|q| (q, at)))
                .collect::<Vec<_>>()
        })
        .collect();
    // (id, anchor, net, global, orientation, the label record)
    type LabelOut<'a> = (Option<&'a str>, [i32; 2], String, bool, Orient, Option<u32>);
    let mut all_labels: Vec<LabelOut> = (0..g.label_at.len())
        .map(|k| {
            let o = c.orient(g.label_rec[k], 3);
            (
                Some(g.label_id[k].as_str()),
                g.label_at[k],
                g.label_net[k].clone(),
                g.label_global[k],
                o,
                Some(g.label_rec[k]),
            )
        })
        .collect();
    for l in &edits.labels {
        all_labels.push((None, l.at, l.net.clone(), l.global, Orient::R0, None));
    }
    for (from, l) in &edits.stubs {
        let r = ids.take();
        let (a, b) = (conv(*from, "stub", c, notes), conv(l.at, "stub", c, notes));
        cv.wires.push(Wire {
            r,
            pts: vec![a, b],
            width: 0,
        });
        segs.push((r, [from[0], from[1], l.at[0], l.at[1]]));
        all_labels.push((None, l.at, l.net.clone(), l.global, Orient::R0, None));
    }
    for (id, at, net, global, o, rec) in all_labels {
        // The Virtuoso instance name and library of the global symbol it came from.
        let vinst = rec.and_then(|r| c.attr(r, "vinst"));
        let vlib = rec.and_then(|r| c.attr(r, "vlib")).unwrap_or("analogLib");
        let r = ids.take();
        let (text, review) = net_to_virtuoso(&net, global);
        if review {
            notes.review("name", &c.path, format!("net `{net}` kept verbatim"));
        }
        let back = net_from_virtuoso(&text);
        if (back.text.as_str(), back.global) != (net.as_str(), global) {
            cv.props
                .push(Prop::string(r, "schemifyNet", read::net_prop(&net, global)));
        }
        // A global net with an analogLib global symbol (`gnd!`, `vdd!`, …) is drawn as that
        // symbol, Virtuoso's idiom; import turns the symbol back into this label (§6.1).
        // GUESS(G3): the symbol's pin sits at its origin.
        // Only a label that came from such a symbol, or one on no wire (which would
        // otherwise need a stub), becomes the symbol; a wire label stays a wire label.
        let on_wire = segs.iter().any(|(_, s)| nets::on_segment(at, *s));
        if let Some(sym) = text
            .strip_suffix('!')
            .filter(|b| devices::GLOBAL_SYMBOLS.contains(b))
            .filter(|_| vinst.is_some() || !on_wire)
        {
            if let Some(id) = id {
                id_prop(&mut cv, r, id);
            }
            cv.insts.push(Inst {
                r,
                name: vinst.map_or_else(|| format!("sfyG{r}"), str::to_owned),
                lib: vlib.into(),
                cell: sym.to_owned(),
                view: "symbol".into(),
                xy: conv(at, "label", c, notes),
                orient: profile::orient_to_virtuoso(o).into(),
            });
            continue;
        }
        // Where Virtuoso drew it: `voff <dx> <dy>` from its point on the wire (§7.5).
        let off = match rec.map_or_else(Vec::new, |r| c.attr_values(r, "voff"))[..] {
            [dx, dy] => [dx, dy].map(|v| parse_coord(v).unwrap_or(0)),
            _ => [0, 0],
        };
        let origin = [at[0] + off[0], at[1] + off[1]];
        // What a label from Virtuoso was there: a net expression (`vexpr <text>`), a style
        // (`vstyle <justify> <font> <height> [<layer> <purpose>]`) and its flags (`vflags`),
        // drawn so again.
        let vexpr = rec.and_then(|r| c.attr(r, "vexpr"));
        let vstyle: Vec<&str> = rec.map_or_else(Vec::new, |r| c.attr_values(r, "vstyle"));
        let (justify, font, height, layer, purpose) = match vstyle[..] {
            [j, f, h] => (j, f, h.parse().unwrap_or(LABEL_HEIGHT), "wire", "label"),
            [j, f, h, l, p] => (j, f, h.parse().unwrap_or(LABEL_HEIGHT), l, p),
            _ => ("lowerLeft", "stick", LABEL_HEIGHT, "wire", "label"),
        };
        // ponytail: O(labels × wires) gluing; a sorted segment index for 10^5-wire cells.
        // Of the wires through the point, the one import projected the origin onto.
        let parent = match segs
            .iter()
            .filter(|(_, s)| nets::on_segment(at, *s))
            .min_by_key(|(_, s)| read::project(origin, &[[s[0], s[1]], [s[2], s[3]]]) != at)
        {
            Some((wr, _)) => *wr,
            // A label Virtuoso drew off the wire layer (`pin annotate`) and glued to nothing
            // stays unglued (live ICADVM20.1: such a label has no parent).
            None if layer != "wire" => 0,
            None => {
                // §6.2: a stub away from the instance whose pin this is.
                let origin = pin_points
                    .iter()
                    .find(|(p, _)| *p == at)
                    .map(|(_, o)| *o)
                    .unwrap_or([at[0] - 1, at[1]]);
                let dir = remap::away(origin, at);
                let to = [at[0] + dir[0] * STEP, at[1] + dir[1] * STEP];
                let sr = ids.take();
                let (a, b) = (conv(at, "stub", c, notes), conv(to, "stub", c, notes));
                cv.wires.push(Wire {
                    r: sr,
                    pts: vec![a, b],
                    width: 0,
                });
                cv.props.push(Prop::string(sr, "schemifyStub", "1"));
                // A second label on this point (a label kept on a re-mastered pin) glues
                // here too instead of drawing an identical stub, which Virtuoso refuses.
                segs.push((sr, [at[0], at[1], to[0], to[1]]));
                sr
            }
        };
        if let Some(id) = id {
            id_prop(&mut cv, r, id);
        }
        let kind = if vexpr.is_some() { "NLP" } else { "normal" };
        let text = vexpr.map_or(text, str::to_owned);
        let xy = conv(origin, "label", c, notes);
        let style = [layer, purpose, justify, font, kind];
        let vflags = rec.map_or_else(Vec::new, |r| c.attr_values(r, "vflags"));
        let l = label(r, parent, text, xy, o, height, style);
        cv.labels.push(with_flags(l, &vflags));
    }
    // Ports → schematic pins.
    let own = sym_pins(c);
    for k in 0..g.port_at.len() {
        let r = ids.take();
        let pin = &g.port_pin[k];
        let dir = own
            .name
            .iter()
            .position(|p| p == pin)
            .map_or("inputOutput", |q| dir_to_virtuoso(&own.dir[q]));
        let term = vname(pin);
        if net_from_virtuoso(&term).text != *pin {
            cv.props
                .push(Prop::string(r, "schemifyNet", read::net_prop(pin, false)));
        }
        let o = c.orient(g.port_rec[k], 3);
        id_prop(&mut cv, r, &g.port_id[k]);
        // A pin that came in as a bare pin shape (`vfig <w> <h>`) goes back as one.
        let fig = match c.attr_values(g.port_rec[k], "vfig")[..] {
            [w, h, ..] => parse_coord(w).ok().zip(parse_coord(h).ok()),
            _ => None,
        };
        if let Some((w, h)) = fig {
            let at = conv(g.port_at[k], "port", c, notes);
            cv.pinfigs.push(PinFig {
                r,
                term,
                b: [at[0] - w / 2, at[1] - h / 2, at[0] + w / 2, at[1] + h / 2],
                dir: dir.into(),
            });
            continue;
        }
        // The pin master it came in on (`vpin <lib> <cell> <view>`), else basic's for the
        // direction.
        let vpin: Vec<&str> = c.attr_values(g.port_rec[k], "vpin");
        let [lib, cell, view] = match vpin[..] {
            [l, c, v] => [l, c, v],
            _ => ["basic", profile::pin_master(dir), "symbol"],
        };
        cv.spins.push(Spin {
            r,
            term,
            dir: dir.into(),
            lib: lib.into(),
            cell: cell.into(),
            view: view.into(),
            xy: conv(g.port_at[k], "port", c, notes),
            orient: profile::orient_to_virtuoso(o).into(),
        });
    }
    // Noconns → basic/noConn instances, named and turned as Virtuoso had them (`vinst`,
    // `vorient`, §7.5), else `NC<k>` and R0.
    names.extend(
        g.noconn_rec
            .iter()
            .filter_map(|&r| c.attr(r, "vinst").map(str::to_owned)),
    );
    let mut nc = 0;
    for k in 0..g.noconn_at.len() {
        let r = ids.take();
        let vinst = c.attr(g.noconn_rec[k], "vinst");
        let name = vinst.map(str::to_owned).unwrap_or_else(|| {
            loop {
                let nm = format!("NC{nc}");
                nc += 1;
                if !names.contains(&nm) {
                    break nm;
                }
            }
        });
        let o = c
            .attr(g.noconn_rec[k], "vorient")
            .and_then(|o| o.parse().ok())
            .unwrap_or(Orient::R0);
        id_prop(&mut cv, r, &g.noconn_id[k]);
        cv.insts.push(Inst {
            r,
            name,
            lib: "basic".into(),
            cell: "noConn".into(),
            view: "symbol".into(),
            xy: conv(g.noconn_at[k], "noconn", c, notes),
            orient: profile::orient_to_virtuoso(o).into(),
        });
    }
    // Texts → note labels; shapes → note shapes.
    for r in c.records(RecordKind::Text) {
        let lr = ids.take();
        let o = c.orient(r, 3);
        id_prop(&mut cv, lr, c.field(r, 0));
        // A label that came from Virtuoso on another layer or with another style (`vlabel
        // <layer> <purpose> <justify> <font> <kind>`) goes back as drawn.
        let v: Vec<&str> = c.attr_values(r, "vlabel");
        let [layer, purpose, justify, font, kind] = match v[..] {
            [a, b, j, f, k] => [a, b, j, f, k],
            _ => ["text", "drawing", "upperLeft", "stick", "normal"],
        };
        let xy = conv(c.point(r, 1), "text", c, notes);
        let (text, h) = (c.field(r, 4).to_owned(), height(c, r, TEXT_SIZE_MILLI));
        let style = [layer, purpose, justify, font, kind];
        let l = label(lr, 0, text, xy, o, h, style);
        cv.labels.push(with_flags(l, &c.attr_values(r, "vflags")));
    }
    for r in c.records(RecordKind::Shape) {
        let dashed = c.attr(r, "dash").is_some_and(|x| x != "0");
        let purpose = if dashed { "drawing1" } else { "drawing" };
        let mut shapes = graphic_out(c, r, RecordKind::Shape, 1, "text", purpose, ids, notes);
        // What the Schemify form does not hold (§7.5): a path's width, and an elliptic
        // ellipse while its polygon is still the one its box came in as.
        if let Some(w) = c.attr(r, "vpath").and_then(|w| parse_coord(w).ok()) {
            for sh in shapes.iter_mut().filter(|s| s.kind == "line") {
                (sh.kind, sh.width) = ("path".into(), w);
            }
        }
        if let [x0, y0, x1, y1] = c.attr_values(r, "vellipse")[..]
            && let [Ok(x0), Ok(y0), Ok(x1), Ok(y1)] = [x0, y0, x1, y1].map(parse_coord)
            && coords(c, r, 2) == read::ellipse_poly([x0, y0, x1, y1])
            && let [sh] = &mut shapes[..]
        {
            let (a, b) = (
                conv([x0, y0], "ellipse", c, notes),
                conv([x1, y1], "ellipse", c, notes),
            );
            (sh.kind, sh.coords) = ("ellipse".into(), norm_box(a, b).to_vec());
        }
        for sh in shapes {
            id_prop(&mut cv, sh.r, c.field(r, 0));
            cv.shapes.push(sh);
        }
    }
    // §6.4 step 2: loader connections for masters of unknown geometry.
    for (k, term, l) in &edits.conns {
        cv.conns.push(Conn {
            r: inst_ref[*k],
            term: term.clone(),
            net: net_to_virtuoso(&l.net, l.global).0,
        });
    }
    assert_eq!(inst_ref.len(), g.inst_id.len());
    cv
}

fn id_prop(cv: &mut Cv, r: u32, id: &str) {
    cv.props.push(Prop::string(r, "schemifyId", id));
}

/// The digest of instance record `rec`'s input values (its overrides, else `master`'s
/// defaults) over `master`'s header `vinputs`, as [`crate::virtuoso::characterize::inputs_digest`]
/// computes it; `None` without `vinputs` or for an input that is not a constant.
fn derived_digest(c: &Cell, rec: u32, master: &Cell) -> Option<String> {
    let vinputs: Vec<String> = master
        .attr_values(0, "vinputs")
        .into_iter()
        .map(str::to_owned)
        .collect();
    if vinputs.is_empty() {
        return None;
    }
    crate::virtuoso::characterize::inputs_digest(&vinputs, |n| {
        let p = master
            .records(RecordKind::Param)
            .find(|q| master.field(*q, 0) == n)?;
        let template = matches!(master.attr(p, "kind"), Some("string" | "choice"));
        Some((c.attr(rec, n).unwrap_or(master.field(p, 1)), template))
    })
}

/// The textDisplay of instance ref `r` at `at` that attribute `a` (`vtd <assoc> <name>
/// <layer> <purpose> <dx> <dy> <orient> <justify> <font> <height> <flags>`) keeps.
fn text_display(c: &Cell, a: u32, r: u32, at: [i32; 2]) -> Option<TextDisplay> {
    let v = |k: usize| c.doc.attr_value(a, k);
    let int = |k: usize| v(k)?.parse::<i32>().ok();
    let f: Vec<bool> = v(10)?.chars().map(|x| x == '1').collect();
    Some(TextDisplay {
        owner: r,
        assoc: v(0)?.to_owned(),
        name: v(1)?.to_owned(),
        layer: v(2)?.to_owned(),
        purpose: v(3)?.to_owned(),
        xy: [at[0].checked_add(int(4)?)?, at[1].checked_add(int(5)?)?],
        orient: v(6)?.to_owned(),
        justify: v(7)?.to_owned(),
        font: v(8)?.to_owned(),
        height: int(9)?,
        flags: f.try_into().ok()?,
    })
}

/// A constant value of a param of Schemify `kind` as the CDF text [`override_prop`] sends
/// for it (a boolean as `TRUE`/`FALSE`); `None` when it has no such form.
pub(crate) fn constant_cdf_text(kind: &str, val: &str) -> Option<String> {
    match kind {
        "int" => val
            .trim()
            .parse::<i64>()
            .is_ok()
            .then(|| val.trim().to_owned()),
        "bool" => match val.trim() {
            "true" => Some("TRUE".into()),
            "false" => Some("FALSE".into()),
            _ => None,
        },
        "string" | "choice" => ael::template_to_literal(val),
        _ => ael::to_ael(val, |_| Name::Unknown).ok().map(|t| t.text),
    }
}

/// One instance override as a CDF property (§6.2, §3.4); `None` (reviewed) when it has
/// no Virtuoso form, so the instance keeps the CDF default.
fn override_prop(
    c: &Cell,
    r: u32,
    pname: &str,
    kind: Option<&str>,
    val: &str,
    inst: &str,
    notes: &mut Notes,
) -> Option<Prop> {
    let prop = |ty: &str, value: String| Prop::new(r, pname, ty, value);
    match kind {
        Some("int") => match val.trim().parse::<i64>() {
            Ok(_) => Some(prop("int", val.trim().to_owned())),
            Err(_) => {
                notes.review(
                    "expression",
                    &c.path,
                    format!("{inst}: {pname} = `{val}` is not a literal int"),
                );
                None
            }
        },
        Some("bool") => match val.trim() {
            "true" => Some(prop("boolean", "TRUE".into())),
            "false" => Some(prop("boolean", "FALSE".into())),
            _ => {
                notes.review(
                    "expression",
                    &c.path,
                    format!("{inst}: {pname} = `{val}` is not a literal bool"),
                );
                None
            }
        },
        Some("string" | "choice") | None => match ael::template_to_literal(val) {
            Some(t) => Some(prop("string", t)),
            // A value that is one hole (`{w}`, `{w*2}`) is its expression, evaluated in the
            // parent: `pPar("w")`, `pPar("w")*2` (§3.4).
            None if let Some(Ok(t)) =
                ael::one_hole(val).map(|e| ael::to_ael(e, parent_scope(c))) =>
            {
                if let Some(why) = t.review {
                    notes.review(why, &c.path, format!("{inst}: {pname} = `{val}`"));
                }
                Some(prop("string", t.text))
            }
            None => {
                notes.review(
                    "template",
                    &c.path,
                    format!("{inst}: {pname} = `{val}` has template holes"),
                );
                None
            }
        },
        _ => match ael::to_ael(val, parent_scope(c)) {
            Ok(t) => {
                if let Some(why) = t.review {
                    notes.review(why, &c.path, format!("{inst}: {pname} = `{val}`"));
                }
                Some(prop("string", t.text))
            }
            Err(why) => {
                notes.review(
                    why,
                    &c.path,
                    format!("{inst}: {pname} = `{val}` keeps the CDF default"),
                );
                None
            }
        },
    }
}
