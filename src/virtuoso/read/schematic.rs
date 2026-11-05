//! A schematic cellview's objects → Schemify records (`schematic_in`, spec `interop.md`
//! §6.2 schematic rows, §6.4): instances, pins, wires (rejoined where Virtuoso split them),
//! labels and notes.

use super::*;

/// The schematic objects (§6.2) of dump cell `k` into `doc`; returns the stash text.
/// An instance waiting for the Schemify-side re-mastering pass (§6.4).
pub(super) struct Pending {
    /// Instance id.
    pub(super) id: String,
    /// The new master.
    pub(super) cellref: String,
    /// Its overrides, already in the new master's names.
    pub(super) attrs: Vec<(String, String)>,
    /// Device row of [`devices::DEVICES`].
    pub(super) device: usize,
}

pub(super) fn schematic_in(
    ctx: &Ctx<'_>,
    k: usize,
    sv: &Cv,
    doc: &mut Document<'static>,
    pins: &[PinRow],
    pending: &mut Vec<Pending>,
    notes: &mut Notes,
) -> Option<String> {
    let c = &ctx.dump.cells[k];
    let t = &ctx.targets[k];
    let kind = t.kind.expect("written");
    let cell = format!("{}/{}", c.lib, c.name);
    let upi = sv.upi;
    let mut tally = Tally::default();
    // Every schemifyId in dump order claims first; duplicates get fresh ids (§7.2).
    let reserved: BTreeSet<String> = BTreeSet::new();
    let mut ids = Ids::new(format!("{}/{}", c.lib, c.name));
    // Properties by owner: one pass, so per-object lookups stay logarithmic.
    let mut by_owner: BTreeMap<u32, Vec<&Prop>> = BTreeMap::new();
    for p in &sv.props {
        by_owner.entry(p.r).or_default().push(p);
    }
    let prop = |r: u32, name: &str| {
        by_owner
            .get(&r)
            .and_then(|v| v.iter().find(|p| p.name == name))
            .map(|p| p.value.as_str())
    };
    let id_of = |r: u32| prop(r, "schemifyId");
    // Pre-claim: ids wanted by objects, in dump order, so a fresh id never steals one.
    let mut claimed: BTreeSet<String> = BTreeSet::new();
    for p in sv
        .props
        .iter()
        .filter(|p| p.name == "schemifyId" && is_id(&p.value))
    {
        claimed.insert(p.value.clone());
    }
    let reserved = if claimed.is_empty() {
        reserved
    } else {
        claimed
    };
    let own_params: Vec<String> = c
        .cdf
        .as_ref()
        .map(|d| d.params.iter().map(|p| p.name.clone()).collect())
        .unwrap_or_default();
    let bench = kind == FileKind::Shbench;
    // Pin figures of the instances as Virtuoso draws them (placed box, its centre), and the
    // labels that join a pin of another view to the symbol's (S3).
    let mut figs: Vec<([i32; 4], [i32; 2])> = Vec::new();
    let mut joins: Vec<Join> = Vec::new();
    // Instances.
    for inst in &sv.insts {
        tally.out += 1;
        let at = pt(
            inst.xy,
            upi,
            &format!("instance {}", inst.name),
            &cell,
            notes,
        );
        let o = orient_in(&inst.orient);
        if inst.lib == "basic" && inst.cell == "noConn" {
            let id = ids.next(id_of(inst.r), &reserved);
            doc.begin_record(RecordKind::Noconn);
            doc.push_field(&id);
            doc.push_field_coord(at[0]);
            doc.push_field_coord(at[1]);
            // Its name and orientation (§7.5); a name export generates (`NC<k>`) is not kept.
            let generated = inst
                .name
                .strip_prefix("NC")
                .is_some_and(|k| !k.is_empty() && k.bytes().all(|b| b.is_ascii_digit()));
            if !generated {
                push_attr(doc, "vinst", &[&inst.name]);
            }
            if o != Orient::R0 {
                push_attr(doc, "vorient", &[o.as_str()]);
            }
            continue;
        }
        // A master with no pins (a sheet border, a title block) draws nothing Schemify keeps.
        if let Some(j) = ctx.find(&inst.lib, &inst.cell)
            && !ctx.targets[j].write
            && !ctx.dump.cells[j].is_global_symbol()
            && ctx.dump.cells[j]
                .symbol()
                .is_some_and(|s| s.terms.is_empty())
        {
            notes.dropped(
                "display-only",
                &cell,
                format!("{}: {}/{} has no pins", inst.name, inst.lib, inst.cell),
            );
            continue;
        }
        // A multi-sheet index (S5): its sheets are cells of their own in Virtuoso, which
        // netlists them as one level; here each is a pinless sub-cell.
        if inst.view == "msymbol" {
            notes.review(
                "name",
                &cell,
                format!(
                    "{}: sheet {}/{} of a multi-sheet schematic is a sub-cell: nets it shares \
                     with other sheets by name are not joined",
                    inst.name, inst.lib, inst.cell
                ),
            );
        }
        if ctx.global_symbol(&inst.lib, &inst.cell) || prop(inst.r, "schemifyNet").is_some() {
            // A global label at the symbol's pin point.
            let gi = ctx
                .find(&inst.lib, &inst.cell)
                .and_then(|g| ctx.ifaces[g].clone());
            let (pin_at, net) = match gi.as_ref().and_then(|i| i.pins.first()) {
                Some(p) => (
                    crate::design::place(p.at, o, at).unwrap_or(at),
                    ctx.find(&inst.lib, &inst.cell)
                        .and_then(|g| ctx.dump.cells[g].symbol())
                        .and_then(|s| s.terms.first())
                        .map_or_else(|| format!("{}!", inst.cell), |t| t.name.clone()),
                ),
                None => {
                    notes.review(
                        "name",
                        &cell,
                        format!(
                            "{}/{} symbol not in the dump: label at its origin",
                            inst.lib, inst.cell
                        ),
                    );
                    (at, format!("{}!", inst.cell))
                }
            };
            let (text, global) = match prop(inst.r, "schemifyNet") {
                Some(sn) => net_of_prop(sn),
                None => {
                    let n = net_from_virtuoso(&net);
                    (n.text, n.global)
                }
            };
            let id = ids.next(id_of(inst.r), &reserved);
            push_anchored(doc, RecordKind::Label, &id, pin_at, o, &text);
            if global {
                push_attr(doc, "global", &["true"]);
            }
            // The symbol instance's name, so export draws the same instance (§6.1); a name
            // the exporter generated (`sfyG<n>`) is not worth keeping.
            if !inst.name.starts_with("sfyG") {
                push_attr(doc, "vinst", &[&inst.name]);
            }
            if inst.lib != "analogLib" {
                push_attr(doc, "vlib", &[&inst.lib]);
            }
            // A supply symbol not named after its net (a kit's or a user's `myvdd`, S11).
            if inst.cell != net.trim_end_matches('!') {
                push_attr(doc, "vcell", &[&inst.cell]);
            }
            continue;
        }
        let m = ctx.master(&inst.lib, &inst.cell);
        let Some(cref) = m.cellref(&t.dir) else {
            notes.review(
                "name",
                &cell,
                format!(
                    "{}: master {}/{} is a root cell seen from another directory",
                    inst.name, inst.lib, inst.cell
                ),
            );
            continue;
        };
        if m.iface.is_none() {
            notes.review(
                "name",
                &cell,
                format!(
                    "{}: master {}/{} is not in the dump or the project",
                    inst.name, inst.lib, inst.cell
                ),
            );
        }
        // §6.4 on the Schemify side: the instance keeps its analogLib mirror (whose pins
        // are known) until the re-mastering pass moves it onto the core cell.
        let remap_old = m
            .alib
            .and_then(|_| ctx.find(&inst.lib, &inst.cell))
            .filter(|&j| ctx.targets[j].write);
        let new_cref = cref.clone();
        let cref = match remap_old {
            Some(j) => {
                let tj = &ctx.targets[j];
                Master {
                    slib: tj.slib.clone(),
                    dir: tj.dir.clone(),
                    stem: tj.stem.clone(),
                    ..Master::default()
                }
                .cellref(&t.dir)
                .unwrap_or(cref)
            }
            None => {
                if m.alib.is_some() {
                    notes.add(
                        "remapped",
                        &cell,
                        format!(
                            "{}: {}/{} → {new_cref} in place (analogLib geometry unknown)",
                            inst.name, inst.lib, inst.cell
                        ),
                    );
                }
                cref
            }
        };
        let id = ids.next(id_of(inst.r), &reserved);
        doc.begin_record(RecordKind::Inst);
        doc.push_field(&id);
        doc.push_field(&cref);
        doc.push_attr("at");
        doc.push_value_coord(at[0]);
        doc.push_value_coord(at[1]);
        doc.push_value(o.as_str());
        push_attr(doc, "name", &[&profile::inst_from_virtuoso(&inst.name)]);
        let props: Vec<&Prop> = by_owner.get(&inst.r).cloned().unwrap_or_default();
        if props
            .iter()
            .any(|p| p.name == "nlAction" && p.value == "ignore")
        {
            push_attr(doc, "ignore", &["true"]);
        }
        let iface = m.iface.clone().unwrap_or_default();
        let vview = view_pins(
            ctx,
            sv,
            inst,
            (o, at),
            &iface,
            &cell,
            notes,
            &mut figs,
            &mut joins,
        );
        let row = m.row.map(|r| &ctx.map.maps[r]);
        let dev = m.alib.map(|(d, _)| &devices::DEVICES[d]);
        // Sources: the waveform params become the `value` text (§6.6).
        let src = m.source.or_else(|| dev.and_then(|d| d.source));
        let mut overrides: Vec<(String, String)> = Vec::new();
        let mut derived: Vec<(String, String)> = Vec::new();
        let mut inherits: Vec<(String, String)> = Vec::new();
        let raw = |n: &str| props.iter().find(|p| p.name == n).map(|p| p.value.clone());
        let mut wave_params: BTreeMap<String, String> = BTreeMap::new();
        // Properties Schemify has no param for, kept with their Virtuoso type (vprop).
        let mut vprops: Vec<&Prop> = Vec::new();
        let plain_master = row.is_none() && dev.is_none() && src.is_none();
        for p in &props {
            match p.name.as_str() {
                "schemifyId" | "nlAction" => continue,
                _ => {}
            }
            if let Some((_, pin)) = iface.inherit_props.iter().find(|x| x.0 == p.name) {
                let net = net_from_virtuoso(&p.value).text;
                // Virtuoso stores the pin's default net on every instance (live ICADVM20.1
                // run: `VSS!` for a pin inheriting VSS); that is no override.
                let default = iface
                    .pins
                    .iter()
                    .find(|x| &x.name == pin)
                    .and_then(|x| x.inherit.as_deref());
                let bare = |n: &str| n.trim_end_matches('!').to_owned();
                if default.is_some_and(|d| bare(d) == bare(&net)) {
                    continue;
                }
                inherits.push((pin.clone(), net));
                continue;
            }
            if p.ty == "netSet" {
                // An override of a net expression deeper than the master's own inherited
                // pins: Schemify's `inherit` names a pin of the master, so it has no place.
                notes.review(
                    "inherit",
                    &cell,
                    format!(
                        "{}: netSet {} = {} overrides a net expression below {}/{}",
                        inst.name, p.name, p.value, inst.lib, inst.cell
                    ),
                );
                continue;
            }
            if matches!(p.ty.as_str(), "ILList" | "time" | "ILExpr" | "NLPExpr")
                || (plain_master && p.ty != "string" && iface.kind(&p.name).is_none())
            {
                vprops.push(p);
                continue;
            }
            if let Some(current) = src
                && devices::source_cdf_params(current).contains(&p.name.as_str())
            {
                wave_params.insert(p.name.clone(), p.value.clone());
                continue;
            }
            // Renames back.
            let (sname, scale) = if let Some(row) = row {
                if row.sets.iter().any(|s| s.0 == p.name) {
                    continue;
                }
                match row.params.iter().find(|x| x.1 == p.name) {
                    Some(x) => (x.0.clone(), x.2.clone()),
                    None => (p.name.clone(), None),
                }
            } else if let Some(dv) = dev {
                match dv.params.iter().find(|x| x.1 == p.name) {
                    Some(x) => (x.0.to_owned(), None),
                    // The core cell has no such param (res tc1, MOS ad): reported (S13).
                    None => {
                        notes.dropped(
                            "unmapped",
                            &cell,
                            format!(
                                "{}: {} = {} has no param on {new_cref}",
                                inst.name, p.name, p.value
                            ),
                        );
                        continue;
                    }
                }
            } else {
                (p.name.clone(), None)
            };
            let pkind = iface.kind(&sname).map(str::to_owned);
            let value = match (pkind.as_deref(), p.ty.as_str()) {
                (_, "boolean") => ael::bool_from_cdf(&p.value).map(str::to_owned),
                (Some("int"), _) | (_, "int") => Some(p.value.trim().to_owned()),
                // An expression over the parent's params (`pPar("w")*2`) is that hole
                // (`{w*2}`, §3.4), the form export reads for a param of unknown kind too;
                // any other text is literal.
                (k, _)
                    if matches!(k, Some("string" | "choice"))
                        || (k.is_none() && p.value.contains("pPar(")) =>
                {
                    Some(
                        Some(&p.value)
                            .filter(|v| v.contains("pPar("))
                            .and_then(|v| {
                                ael::from_ael(
                                    v,
                                    |n| {
                                        if own_params.iter().any(|x| x == n) {
                                            AelName::Param
                                        } else {
                                            AelName::Unknown
                                        }
                                    },
                                    |_| None,
                                )
                            })
                            .filter(|t| {
                                t.review.is_none()
                                    && schemify_expr::Program::compile(
                                        &t.text,
                                        schemify_expr::Mode::Scalar,
                                    )
                                    .is_ok()
                            })
                            .map_or_else(
                                || ael::literal_to_template(&p.value),
                                |t| format!("{{{}}}", si_text(&t.text)),
                            ),
                    )
                }
                (Some("bool"), _) => ael::bool_from_cdf(&p.value).map(str::to_owned),
                (kind, _) => {
                    let tr = ael::from_ael(
                        &p.value,
                        |n| {
                            if own_params.iter().any(|x| x == n) {
                                AelName::Param
                            } else if bench {
                                AelName::BenchVar
                            } else {
                                AelName::Unknown
                            }
                        },
                        |n| {
                            raw(n)
                                .and_then(|v| {
                                    ael::from_ael(&v, |_| AelName::Param, |_| None).map(|t| t.text)
                                })
                                .or_else(|| iface.def(n).map(str::to_owned))
                                .map(|v| format!("({v})"))
                        },
                    );
                    let tr = tr.map(|mut x| {
                        x.text = si_text(&x.text);
                        x
                    });
                    match tr {
                        Some(tr)
                            if schemify_expr::Program::compile(
                                &tr.text,
                                schemify_expr::Mode::Scalar,
                            )
                            .is_ok() =>
                        {
                            if let Some(why) = tr.review {
                                notes.review(
                                    why,
                                    &cell,
                                    format!("{}: {} = {}", inst.name, p.name, p.value),
                                );
                            }
                            Some(tr.text)
                        }
                        _ if kind.is_none() => Some(ael::literal_to_template(&p.value)),
                        _ => {
                            notes.review(
                                "expression",
                                &cell,
                                format!("{}: {} = `{}` dropped", inst.name, p.name, p.value),
                            );
                            None
                        }
                    }
                }
            };
            let Some(mut value) = value else { continue };
            if let Some(f) = scale {
                value = format!("({value})/({f})");
                if let Some(v) = ael::fold_numeric(&value) {
                    value = v;
                }
            }
            // A computed param's value is its rule's (an override would be E030); a
            // derived one is Virtuoso's, cached with the inputs it came from (§8.3).
            if iface.computed.contains(&sname) {
                continue;
            }
            if iface.vderived.contains(&sname) {
                derived.push((sname, value));
                continue;
            }
            overrides.push((sname, value));
        }
        if let Some(current) = src
            && (!wave_params.is_empty() || dev.is_some())
        {
            let wave = match (dev, m.alib) {
                (Some(_), Some((_, w))) => w,
                _ => wave_params
                    .get("srcType")
                    .and_then(|s| devices::WAVEFORMS.iter().position(|w| w.1 == s))
                    .unwrap_or(0),
            };
            let text = devices::format_source(wave, current, |n| {
                wave_params.get(n).filter(|v| !v.is_empty()).map(|v| {
                    ael::from_ael(v, |_| AelName::Param, |_| None).map_or(v.clone(), |t| t.text)
                })
            });
            overrides.push(("value".into(), text));
        }
        // Instance overrides in the master's param order (then the unknown ones).
        overrides.sort_by_key(|(n, _)| {
            iface
                .params
                .iter()
                .position(|p| p.0 == *n)
                .unwrap_or(usize::MAX)
        });
        if remap_old.is_some() {
            notes.add(
                "remapped",
                &cell,
                format!("{}: {}/{} → {new_cref}", inst.name, inst.lib, inst.cell),
            );
            let tds = sv.tds.iter().filter(|t| t.owner == inst.r).count();
            if tds > 0 {
                notes.dropped(
                    "display-only",
                    &cell,
                    format!("{}: {tds} textDisplay(s) of the old master", inst.name),
                );
            }
            pending.push(Pending {
                id: id.clone(),
                cellref: new_cref,
                attrs: overrides,
                device: m.alib.map(|a| a.0).expect("remap_old needs a device row"),
            });
            continue;
        }
        let cached = cached_lines(&iface, &overrides, derived);
        for (n, v) in overrides {
            push_attr(doc, &n, &[&v]);
        }
        for (p, n) in inherits {
            push_attr(doc, "inherit", &[p, n]);
        }
        for [p, v, d] in cached {
            push_attr(doc, "cached", &[p, v, d]);
        }
        // One `vprop` line, its values in (name, type, value) triples: an instance attribute
        // is an override, and a second line of one key is E024 (live: a sheet instance of a
        // multi-sheet index has five).
        if !vprops.is_empty() {
            let vals: Vec<&str> = vprops
                .iter()
                .flat_map(|p| [p.name.as_str(), p.ty.as_str(), p.value.as_str()])
                .collect();
            push_attr(doc, "vprop", &vals);
        }
        // Its textDisplays, at their offset from the instance's origin (DBU, Virtuoso's
        // axes), so they follow a move.
        // All on one `vtd` line, eleven values each (two lines of one key are E024).
        let mut vtd: Vec<String> = Vec::new();
        for t in sv.tds.iter().filter(|t| t.owner == inst.r) {
            let (dx, dy) = (t.xy[0] - inst.xy[0], t.xy[1] - inst.xy[1]);
            let flags: String = t.flags.iter().map(|&f| if f { '1' } else { '0' }).collect();
            vtd.extend([
                t.assoc.clone(),
                t.name.clone(),
                t.layer.clone(),
                t.purpose.clone(),
                dx.to_string(),
                dy.to_string(),
                t.orient.clone(),
                t.justify.clone(),
                t.font.clone(),
                t.height.to_string(),
                flags,
            ]);
        }
        if !vtd.is_empty() {
            let v: Vec<&str> = vtd.iter().map(String::as_str).collect();
            push_attr(doc, "vtd", &v);
        }
        if let Some(v) = vview {
            push_attr(doc, "vview", &[v]);
        }
    }
    // Schematic pins.
    for s in &sv.spins {
        tally.out += 1;
        let at = pt(s.xy, upi, &format!("pin {}", s.term), &cell, notes);
        let o = orient_in(&s.orient);
        let name = sv
            .prop(s.r, "schemifyNet")
            .map_or_else(|| net_from_virtuoso(&s.term).text, str::to_owned);
        let id = ids.next(id_of(s.r), &reserved);
        if !bench && !pins.iter().any(|p| p.name == name) {
            notes.review(
                "name",
                &cell,
                format!("schematic pin {name} has no symbol terminal"),
            );
        }
        let kind = if bench {
            RecordKind::Label
        } else {
            RecordKind::Port
        };
        push_anchored(doc, kind, &id, at, o, &name);
        // A pin master other than the one export picks for the direction (Cadence's
        // sample library uses basic/sympin view symbolr): kept, so export uses it again.
        let usual = profile::pin_master(&s.dir);
        if (s.lib.as_str(), s.cell.as_str(), s.view.as_str()) != ("basic", usual, "symbol") {
            push_attr(doc, "vpin", &[&s.lib, &s.cell, &s.view]);
        }
    }
    // Schematic pins drawn as bare pin shapes: ports at the rectangle centre, carrying the
    // rectangle's size (`vfig <w> <h>`, an unknown attribute the format keeps) so export
    // draws the same shape again; in a bench, labels (as `spin`s).
    for f in &sv.pinfigs {
        tally.out += 1;
        let b = box_in(&f.b, upi, &format!("pin {}", f.term), &cell, notes);
        let name = sv
            .prop(f.r, "schemifyNet")
            .map_or_else(|| net_from_virtuoso(&f.term).text, str::to_owned);
        let id = ids.next(id_of(f.r), &reserved);
        let kind = if bench {
            RecordKind::Label
        } else {
            RecordKind::Port
        };
        let centre = [(b[0] + b[2]) / 2, (b[1] + b[3]) / 2];
        push_anchored(doc, kind, &id, centre, Orient::R0, &name);
        if !bench {
            doc.push_attr("vfig");
            doc.push_value_coord(b[2] - b[0]);
            doc.push_value_coord(b[3] - b[1]);
        }
    }
    // Wires: segments, stubs removed.
    let mut wire_pts: BTreeMap<u32, Vec<[i32; 2]>> = BTreeMap::new();
    let mut ends: Vec<[i32; 2]> = Vec::new();
    let mut segs: Vec<[i32; 4]> = Vec::new();
    let all_pts: Vec<DumpWire<'_>> = sv
        .wires
        .iter()
        .filter(|w| prop(w.r, "schemifyStub").is_none())
        .map(|w| {
            let pts = w
                .pts
                .iter()
                .map(|p| pt(*p, upi, "wire", &cell, notes))
                .collect();
            (w.r, pts, id_of(w.r).filter(|i| is_id(i)))
        })
        .collect();
    let (rejoined, absorbed) = rejoin_split_wires(&all_pts);
    let unswapped = {
        let stashed = stash_of(sv)
            .and_then(|s| merge::parse_stash(&s, kind).ok())
            .map(|d| stash_wires(&d))
            .unwrap_or_default();
        let drawn: Vec<(u32, [[i32; 2]; 2], &str)> = all_pts
            .iter()
            .filter(|(r, _, _)| !absorbed.contains(r))
            .filter_map(|(r, pts, id)| {
                let pts = rejoined.get(r).unwrap_or(pts);
                let [a, b] = pts[..] else { return None };
                Some((*r, seg_key(a, b), (*id)?))
            })
            .collect();
        unswap_wire_ids(&drawn, &stashed)
    };
    let id_of = |r: u32| unswapped.get(&r).map(String::as_str).or_else(|| id_of(r));
    for w in &sv.wires {
        tally.out += 1;
        let pts: Vec<[i32; 2]> = w
            .pts
            .iter()
            .map(|p| pt(*p, upi, "wire", &cell, notes))
            .collect();
        wire_pts.insert(w.r, pts.clone());
        if prop(w.r, "schemifyStub").is_some() || absorbed.contains(&w.r) {
            continue;
        }
        let pts = rejoined.get(&w.r).cloned().unwrap_or(pts);
        // A bus width other than export's: `vwidth <w>` (§7.5).
        let width = profile::height_from_virtuoso(w.width, upi) as i32;
        let mut first = true;
        for seg in pts.windows(2) {
            if seg[0] == seg[1] {
                continue;
            }
            ends.extend([seg[0], seg[1]]);
            segs.push([seg[0][0], seg[0][1], seg[1][0], seg[1][1]]);
            let id = ids.next(if first { id_of(w.r) } else { None }, &reserved);
            first = false;
            let xy = [seg[0][0], seg[0][1], seg[1][0], seg[1][1]];
            crate::remap::push_wire(doc, &id, xy, w.width > 0);
            if w.width > 0 && width != crate::virtuoso::write::BUS_WIDTH {
                doc.push_attr("vwidth");
                doc.push_value_coord(width);
            }
        }
    }
    // Virtuoso puts a wire on a pin when the wire ends anywhere on the pin's figure, edge and
    // corner included, or runs through it off its centre, and joins two pins whose figures
    // overlap (live ICADVM20.1, S1). Schemify joins exact points, so a stub (`vjoin`: export
    // leaves it out) takes each such point to the pin's centre.
    // ponytail: O(points × pin figures); a wire through a figure is matched when it is
    // axis-parallel, a diagonal one is left to the connectivity oracle.
    let on =
        |b: &[i32; 4], p: [i32; 2]| (b[0]..=b[2]).contains(&p[0]) && (b[1]..=b[3]).contains(&p[1]);
    let mut stubbed: BTreeSet<([i32; 2], [i32; 2])> = BTreeSet::new();
    for (b, c) in &figs {
        for e in &ends {
            if on(b, *e) && e != c {
                stubbed.insert((*e, *c));
            }
        }
        for w in &segs {
            let (xs, ys) = (
                w[0].min(w[2])..=w[0].max(w[2]),
                w[1].min(w[3])..=w[1].max(w[3]),
            );
            let p = if w[1] == w[3] && xs.contains(&c[0]) {
                [c[0], w[1]]
            } else if w[0] == w[2] && ys.contains(&c[1]) {
                [w[0], c[1]]
            } else {
                continue;
            };
            if on(b, p) && p != *c {
                stubbed.insert((p, *c));
            }
        }
    }
    for (i, (b, c)) in figs.iter().enumerate() {
        for (d, e) in &figs[i + 1..] {
            let overlap = b[0] < d[2] && d[0] < b[2] && b[1] < d[3] && d[1] < b[3];
            if overlap && c != e {
                stubbed.insert((*c, *e));
            }
        }
    }
    for (p, q) in &stubbed {
        let corner = [q[0], p[1]];
        for (a, z) in [(*p, corner), (corner, *q)] {
            if a != z {
                let id = ids.next(None, &reserved);
                crate::remap::push_wire(doc, &id, [a[0], a[1], z[0], z[1]], false);
                push_attr(doc, "vjoin", &["true"]);
            }
        }
    }
    if !stubbed.is_empty() {
        notes.add(
            "rounding",
            &cell,
            format!(
                "{} point(s) Virtuoso joins on a pin figure off its centre: stubs to the pin",
                stubbed.len()
            ),
        );
    }
    // Labels.
    let mut dropped_alias = 0;
    let mut dropped_stub = 0;
    for l in &sv.labels {
        tally.out += 1;
        // The label the loader glued to a re-mastering stub (§6.4) goes with its stub: the
        // re-mastered instance's own pins meet the drawn wires again, and the stub's net name
        // would be a second name for that net (E003 on a live analogLib round trip).
        // A label with an id is the user's (one on a bare pin travels on a stub too): kept.
        if l.parent != 0 && prop(l.parent, "schemifyStub").is_some() && id_of(l.r).is_none() {
            dropped_stub += 1;
            tally.out -= 1;
            tally.reported += 1;
            continue;
        }
        let o = orient_in(&l.orient);
        let origin = pt(l.xy, upi, "label", &cell, notes);
        match (l.layer.as_str(), l.purpose.as_str()) {
            ("annotate", "drawing3") => {
                dropped_alias += 1;
                tally.out -= 1;
                tally.reported += 1;
            }
            ("wire", "label") | ("pin", "annotate") => {
                let at = match wire_pts.get(&l.parent) {
                    Some(p) => project(origin, p),
                    None => {
                        if l.parent == 0 {
                            notes.review(
                                "name",
                                &cell,
                                format!("label `{}` is glued to nothing", l.text),
                            );
                        }
                        origin
                    }
                };
                let (text, global) = if let Some((p, _)) = parse_netexpr(&l.text) {
                    (net_from_virtuoso(&p).text, false)
                } else if let Some(sn) = prop(l.r, "schemifyNet") {
                    net_of_prop(sn)
                } else {
                    let n = net_from_virtuoso(&l.text);
                    if n.review {
                        notes.review("name", &cell, format!("net `{}` kept verbatim", l.text));
                    }
                    (n.text, n.global)
                };
                let id = ids.next(id_of(l.r), &reserved);
                push_anchored(doc, RecordKind::Label, &id, at, o, &text);
                if global {
                    push_attr(doc, "global", &["true"]);
                }
                // What export would draw otherwise (§6.2): the net expression itself, and a
                // style other than export's (lowerLeft, stick, LABEL_HEIGHT), so the label
                // comes back as drawn.
                if parse_netexpr(&l.text).is_some() {
                    push_attr(doc, "vexpr", &[&l.text]);
                }
                let h = profile::height_from_virtuoso(l.height, upi).to_string();
                let usual = crate::virtuoso::write::LABEL_HEIGHT.to_string();
                if (l.layer.as_str(), l.purpose.as_str()) != ("wire", "label") {
                    push_attr(
                        doc,
                        "vstyle",
                        &[&l.justify, &l.font, &h, &l.layer, &l.purpose],
                    );
                } else if (l.justify.as_str(), l.font.as_str(), &h)
                    != ("lowerLeft", "stick", &usual)
                {
                    push_attr(doc, "vstyle", &[&l.justify, &l.font, &h]);
                }
                // Drawn away from its wire: the offset from the point on it (§7.5).
                if origin != at {
                    doc.push_attr("voff");
                    doc.push_value_coord(origin[0] - at[0]);
                    doc.push_value_coord(origin[1] - at[1]);
                }
                if let Some(f) = flags_of(l) {
                    push_attr(doc, "vflags", &f);
                }
            }
            // Note text, and any other unglued label (a schematic pin's name on pin/label
            // in Cadence's sample library): a text; what export would not draw by default
            // goes in `vlabel <layer> <purpose> <justify> <font> <kind>` (an unknown
            // attribute the format keeps) so it comes back as drawn.
            _ => {
                let id = ids.next(id_of(l.r), &reserved);
                push_anchored(doc, RecordKind::Text, &id, origin, o, &l.text);
                push_attr(doc, "size", &[&coord_text(size_milli(l.height, upi))]);
                let v = [&l.layer, &l.purpose, &l.justify, &l.font, &l.kind];
                if v != ["text", "drawing", "upperLeft", "stick", "normal"] {
                    push_attr(doc, "vlabel", &v);
                }
                if let Some(f) = flags_of(l) {
                    push_attr(doc, "vflags", &f);
                }
            }
        }
    }
    if dropped_stub > 0 {
        notes.dropped(
            "display-only",
            &cell,
            format!("{dropped_stub} re-mastering stub label(s)"),
        );
    }
    if dropped_alias > 0 {
        notes.dropped(
            "display-only",
            &cell,
            format!("{dropped_alias} wire alias label(s)"),
        );
    }
    // Net and instance names that differ only in case are one name to ngspice (S15).
    let nets = sv
        .labels
        .iter()
        .filter(|l| l.layer == "wire")
        .map(|l| l.text.as_str());
    let insts = sv.insts.iter().map(|i| i.name.as_str());
    for (what, all) in [
        ("nets", nets.collect::<Vec<_>>()),
        ("instances", insts.collect()),
    ] {
        let mut names: BTreeMap<String, BTreeSet<&str>> = BTreeMap::new();
        for n in all {
            names.entry(n.to_lowercase()).or_default().insert(n);
        }
        for v in names.values().filter(|v| v.len() > 1) {
            let v: Vec<&str> = v.iter().copied().collect();
            notes.review(
                "name",
                &cell,
                format!(
                    "{what} {} differ only in case: one name to ngspice",
                    v.join(" ")
                ),
            );
        }
    }
    joins.sort();
    joins.dedup();
    for (at, net, global) in &joins {
        let id = ids.next(None, &reserved);
        push_anchored(doc, RecordKind::Label, &id, *at, Orient::R0, net);
        if *global {
            push_attr(doc, "global", &["true"]);
        }
        push_attr(doc, "vjoin", &["true"]);
    }
    // Shapes.
    let shapes: Vec<&Shape> = sv.shapes.iter().collect();
    let merged = merge_arcs(&shapes);
    tally.out += shapes.len();
    let mut dots = 0;
    for sh in &merged {
        if sh.layer == "wire" {
            dots += 1;
            continue;
        }
        if sh.layer != "text" {
            notes.dropped(
                "display-only",
                &cell,
                format!("{} on {}/{}", sh.kind, sh.layer, sh.purpose),
            );
            continue;
        }
        match shape_in(sh, upi, &cell, notes) {
            Some((form, vals, ang, closed)) => {
                let id = ids.next(id_of(sh.r), &reserved);
                push_graphic(
                    doc,
                    Some(&id),
                    &form,
                    &vals,
                    ang,
                    Style {
                        closed,
                        fill: false,
                        dash: sh.purpose == "drawing1",
                    },
                );
                // What the Schemify form does not hold (§7.5): a path's width, and the box of
                // an elliptic ellipse, which comes in as a polygon of its points.
                if sh.kind == "path" && sh.width > 0 {
                    doc.push_attr("vpath");
                    doc.push_value_coord(profile::height_from_virtuoso(sh.width, upi) as i32);
                }
                if sh.kind == "ellipse" && form == "poly" {
                    doc.push_attr("vellipse");
                    for v in box_in(&sh.coords, upi, "ellipse", &cell, notes) {
                        doc.push_value_coord(v);
                    }
                }
            }
            None => notes.dropped("display-only", &cell, format!("note {}", sh.kind)),
        }
    }
    if dots > 0 {
        notes.dropped("solder-dot", &cell, format!("{dots} object(s)"));
    }
    // textDisplays: each on its instance (`vtd`), or reported with a re-mastered one.
    tally.out += sv.tds.len();
    let st = cv_props(sv, &cell, notes, &mut tally);
    report_rest(sv, &cell, notes, &mut tally);
    assert_eq!(
        tally.out + tally.reported,
        sv.objects(),
        "{cell} schematic: count conservation"
    );
    st
}

/// A label joining a pin of a view other than the symbol to the symbol's (S3): point, net
/// (Schemify syntax), global.
pub(super) type Join = ([i32; 2], String, bool);

/// The pin figures of `inst` (orientation and origin `place`, Schemify units) in the view
/// it is placed through, into `figs`. When that view is not the one its master's Schemify
/// symbol came from (`symbol_alt`, `symbol_xform`, …), each of its pins that lies elsewhere
/// than the symbol's gets a label pair in `joins`, at both points, named by Virtuoso's own
/// net of that terminal (`tnet`). Returns the view to keep on the instance (`vview`).
#[allow(clippy::too_many_arguments)]
fn view_pins<'a>(
    ctx: &Ctx<'_>,
    sv: &Cv,
    inst: &'a Inst,
    place: (Orient, [i32; 2]),
    iface: &Iface,
    cell: &str,
    notes: &mut Notes,
    figs: &mut Vec<([i32; 4], [i32; 2])>,
    joins: &mut Vec<Join>,
) -> Option<&'a str> {
    let (o, at) = place;
    let put = |p: [i32; 2]| crate::design::place(p, o, at).unwrap_or(at);
    let Some(c) = ctx.find(&inst.lib, &inst.cell).map(|j| &ctx.dump.cells[j]) else {
        // A master outside the dump (core, a mapped kit cell): its view goes back as it came.
        return (inst.view != "symbol").then_some(inst.view.as_str());
    };
    let main = c.symbol().map(|s| s.view.as_str());
    // A view without terminals of its own (analogLib's `symbol_xform`, live ICADVM20.1) shows
    // the symbol's: Virtuoso connects its instances at the symbol's pins.
    let own = c
        .cv(&inst.view)
        .filter(|v| v.vtype == "schematicSymbol" && !v.terms.is_empty());
    let Some(vcv) = own.or_else(|| c.cv(&inst.view).and(c.symbol())) else {
        if main.is_some_and(|m| m != inst.view) {
            notes.review(
                "name",
                cell,
                format!(
                    "{}: view {}/{}/{} is not in the dump: the pins of {} assumed",
                    inst.name,
                    inst.lib,
                    inst.cell,
                    inst.view,
                    main.unwrap_or("symbol")
                ),
            );
            return Some(&inst.view);
        }
        return None;
    };
    let mut scratch = Notes::default();
    for f in &vcv.pinfigs {
        let b = box_in(&f.b, vcv.upi, "pin", cell, &mut scratch);
        let ctr = centre(f.b, vcv.upi, "pin", cell, &mut scratch);
        let (p, q) = (put([b[0], b[1]]), put([b[2], b[3]]));
        figs.push((profile::norm_box(p, q), put(ctr)));
    }
    if Some(inst.view.as_str()) == main {
        return None;
    }
    if own.is_none() {
        return Some(&inst.view);
    }
    let alt = pins_in(c, vcv, &mut scratch);
    let mut odd: Vec<&str> = alt
        .iter()
        .filter(|a| !iface.pins.iter().any(|p| p.name == a.name))
        .map(|a| a.name.as_str())
        .collect();
    let mut joined = 0;
    for p in iface.pins.iter().filter(|p| p.inherit.is_none()) {
        let Some(a) = alt.iter().find(|a| a.name == p.name) else {
            odd.push(&p.name);
            continue;
        };
        if a.at == p.at {
            continue;
        }
        match sv
            .tnets
            .iter()
            .find(|t| t.r == inst.r && net_from_virtuoso(&t.term).text == p.name)
        {
            Some(t) => {
                let n = net_from_virtuoso(&t.net);
                joins.push((put(a.at), n.text.clone(), n.global));
                joins.push((put(p.at), n.text, n.global));
                joined += 1;
            }
            None => notes.review(
                "name",
                cell,
                format!(
                    "{}: pin {} of view {} lies elsewhere than the symbol's, and the dump has                      no Virtuoso net for it",
                    inst.name, p.name, inst.view
                ),
            ),
        }
    }
    if !odd.is_empty() {
        notes.review(
            "name",
            cell,
            format!(
                "{}: pins {} are not in both {} and {} of {}/{}",
                inst.name,
                odd.join(" "),
                inst.view,
                main.unwrap_or("symbol"),
                inst.lib,
                inst.cell
            ),
        );
    }
    notes.add(
        "remapped",
        cell,
        format!(
            "{}: {}/{}/{} placed as {}/{} ({joined} pin(s) joined by labels)",
            inst.name, inst.lib, inst.cell, inst.view, inst.lib, inst.cell
        ),
    );
    Some(&inst.view)
}

/// A dumped wire for [`rejoin_split_wires`]: (ref, points, its schemifyId when valid).
pub(super) type DumpWire<'a> = (u32, Vec<[i32; 2]>, Option<&'a str>);

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
