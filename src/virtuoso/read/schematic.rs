//! Virtuoso schematic cellview → Schemify schematic (spec `interop.md` §6.2 schematic rows):
//! wires, labels, pins, instances, notes.

use super::*;

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
            continue;
        }
        if devices::is_global_symbol(&inst.lib, &inst.cell) {
            // A global label at the symbol's pin point.
            let gi = ctx
                .find(&inst.lib, &inst.cell)
                .and_then(|g| ctx.ifaces[g].clone());
            let (pin_at, net) = match gi.as_ref().and_then(|i| i.pins.first()) {
                Some(p) => (
                    crate::design::place(p.at, o, at).unwrap_or(at),
                    ctx.find(&inst.lib, &inst.cell)
                        .and_then(|g| ctx.dump.cells[g].cv("symbol"))
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
        let row = m.row.map(|r| &ctx.map.maps[r]);
        let dev = m.alib.map(|(d, _)| &devices::DEVICES[d]);
        // Sources: the waveform params become the `value` text (§6.6).
        let src = m.source.or_else(|| dev.and_then(|d| d.source));
        let mut overrides: Vec<(String, String)> = Vec::new();
        let mut derived: Vec<(String, String)> = Vec::new();
        let mut inherits: Vec<(String, String)> = Vec::new();
        let raw = |n: &str| props.iter().find(|p| p.name == n).map(|p| p.value.clone());
        let mut wave_params: BTreeMap<String, String> = BTreeMap::new();
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
                    None => continue,
                }
            } else {
                (p.name.clone(), None)
            };
            let pkind = iface.kind(&sname).map(str::to_owned);
            let value = match (pkind.as_deref(), p.ty.as_str()) {
                (_, "boolean") => ael::bool_from_cdf(&p.value).map(str::to_owned),
                (Some("int"), _) | (_, "int") => Some(p.value.trim().to_owned()),
                (Some("string" | "choice"), _) => Some(ael::literal_to_template(&p.value)),
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
        let mut first = true;
        for seg in pts.windows(2) {
            if seg[0] == seg[1] {
                continue;
            }
            let id = ids.next(if first { id_of(w.r) } else { None }, &reserved);
            first = false;
            let xy = [seg[0][0], seg[0][1], seg[1][0], seg[1][1]];
            crate::virtuoso::remap::push_wire(doc, &id, xy, w.width > 0);
        }
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
                let h = profile::height_from_virtuoso(l.height, upi);
                if (l.justify.as_str(), l.font.as_str(), h)
                    != (
                        "lowerLeft",
                        "stick",
                        i64::from(crate::virtuoso::write::LABEL_HEIGHT),
                    )
                {
                    push_attr(doc, "vstyle", &[&l.justify, &l.font, &h.to_string()]);
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
            }
            None => notes.dropped("display-only", &cell, format!("note {}", sh.kind)),
        }
    }
    if dots > 0 {
        notes.dropped("solder-dot", &cell, format!("{dots} object(s)"));
    }
    let st = cv_props(sv, &cell, notes, &mut tally);
    report_rest(sv, &cell, notes, &mut tally);
    assert_eq!(
        tally.out + tally.reported,
        sv.objects(),
        "{cell} schematic: count conservation"
    );
    st
}

/// The point of polyline `pts` nearest `p`: the projection on a horizontal or vertical
/// segment, else the nearest end (§6.2).
pub(super) fn project(p: [i32; 2], pts: &[[i32; 2]]) -> [i32; 2] {
    let mut best = pts[0];
    let mut bd = i64::MAX;
    let d2 = |a: [i32; 2], b: [i32; 2]| {
        let (dx, dy) = (
            i64::from(a[0]) - i64::from(b[0]),
            i64::from(a[1]) - i64::from(b[1]),
        );
        dx * dx + dy * dy
    };
    for s in pts.windows(2) {
        let (a, b) = (s[0], s[1]);
        let q = if a[1] == b[1] {
            [p[0].clamp(a[0].min(b[0]), a[0].max(b[0])), a[1]]
        } else if a[0] == b[0] {
            [a[0], p[1].clamp(a[1].min(b[1]), a[1].max(b[1]))]
        } else if d2(p, a) <= d2(p, b) {
            a
        } else {
            b
        };
        if d2(p, q) < bd {
            bd = d2(p, q);
            best = q;
        }
    }
    best
}
