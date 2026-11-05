//! A dump cell → its fresh Schemify document (`build`): header, pins, params, simInfo
//! formats and symbol graphics (spec `interop.md` §6.2 symbol rows, §6.3); the schematic
//! part goes through `schematic_in`. Also the derived box symbol of a cell without one
//! (§6.1).

use super::*;

/// The symbol pins of a dump cell (§6.2 symbol pin row): one pin per terminal at its first
/// pin rectangle, further rectangles as alias pins, in spectre `termOrder` else sorted.
pub(super) fn pins_in(c: &DCell, sym: &Cv, notes: &mut Notes) -> Vec<PinRow> {
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
        let name = term_pin(sym, t);
        if net.review {
            notes.review(
                "name",
                &cell,
                format!("terminal `{}` kept verbatim", t.name),
            );
        }
        let dir = profile::dir_from_virtuoso(&t.dir);
        let inherit =
            parse_netexpr(&t.netexpr).map(|(prop, d)| netset_default(c, &prop, notes).unwrap_or(d));
        let figs: Vec<&PinFig> = sym.pinfigs.iter().filter(|f| f.term == t.name).collect();
        let main = figs
            .iter()
            .find(|f| sym.prop(f.r, "schemifyPin").is_none())
            .or(figs.first());
        let at = main.map_or([0, 0], |f| {
            centre(f.b, sym.upi, &format!("pin {}", t.name), &cell, notes)
        });
        out.push(PinRow {
            name: name.clone(),
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
            let alias = sym
                .prop(f.r, "schemifyPin")
                .map_or(format!("{name}_{n}"), str::to_owned);
            out.push(PinRow {
                name: alias,
                dir: dir.into(),
                at: centre(f.b, sym.upi, &format!("pin {}", t.name), &cell, notes),
                inherit: None,
                alias: Some(name.clone()),
            });
        }
    }
    out
}

/// The Schemify pin of symbol terminal `t`: a pinless inherited one (a stopping view's,
/// SfyStopTerms) is named by its net expression's property (GF22-style nfet `b`
/// `[@lSup:%:VSS!]` → `lSup`), the name instances set and export writes back (§6.2), unless
/// another terminal has that name; any other by its own name.
pub(super) fn term_pin(sym: &Cv, t: &Term) -> String {
    let pinless = !sym.pinfigs.iter().any(|f| f.term == t.name);
    match parse_netexpr(&t.netexpr) {
        // A drawn inherited terminal keeps its name; its property goes to `vinherit`.
        // ponytail: two inheriting one property would give two pins of one name.
        Some((p, _)) if pinless && !sym.terms.iter().any(|x| x.name == p) => {
            net_from_virtuoso(&p).text
        }
        _ => net_from_virtuoso(&t.name).text,
    }
}

/// The default net of inherited property `prop` from a netSet CDF param of that name,
/// which every instance stores (storeDefault). A default that is itself a net expression
/// (a GF22-style resistor's `resbulk` `[@lSup:%:VSS!]`: the bulk follows lSup) gives its
/// default net, reported: Schemify's `inherit` has no chained connection.
fn netset_default(c: &DCell, prop: &str, notes: &mut Notes) -> Option<String> {
    let p = c
        .cdf
        .as_ref()?
        .params
        .iter()
        .find(|p| p.ty == "netSet" && p.name == prop && !p.def.trim().is_empty())?;
    if let Some((q, d)) = parse_netexpr(&p.def) {
        notes.review(
            "inherit",
            &format!("{}/{}", c.lib, c.name),
            format!(
                "{prop} defaults to the inherited {q} ({}): the default net {d} only",
                p.def
            ),
        );
        return Some(d);
    }
    parse_netexpr(&format!("[@{prop}:%:{}]", p.def.trim())).map(|x| x.1)
}

/// The property instances override inherited pin `pin` by, when it is not the pin's own
/// name: a drawn terminal whose net expression names another property (kept as the pin's
/// `vinherit`, so import reads such an instance property as `inherit` and export writes
/// it back).
pub(super) fn inherit_prop(sym: &Cv, pin: &str) -> Option<String> {
    let t = sym.terms.iter().find(|t| term_pin(sym, t) == pin)?;
    let (p, _) = parse_netexpr(&t.netexpr)?;
    (crate::virtuoso::write::vname(pin) != p).then_some(p)
}

/// `[@p:%:d!]` → `(p, d)` with the default's `!` dropped (`gnd!` → `0`, §3.3).
pub(super) fn parse_netexpr(e: &str) -> Option<(String, String)> {
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

/// A Schemify graphic: form, values (milli-units), arc angles (micro-radians), closed.
pub(super) type GraphicIn = (String, Vec<i32>, Option<(i32, i32)>, bool);

/// A Virtuoso arc/ellipse/… as Schemify graphic `(form, values)` in milli-units, or
/// `None` when it is not representable. `upi` converts.
pub(super) fn shape_in(sh: &Shape, upi: i32, cell: &str, notes: &mut Notes) -> Option<GraphicIn> {
    let p = |k: usize, notes: &mut Notes| {
        pt(
            [sh.coords[2 * k], sh.coords[2 * k + 1]],
            upi,
            &sh.kind,
            cell,
            notes,
        )
    };
    match sh.kind.as_str() {
        "line" | "path" => {
            let pts: Vec<i32> = (0..sh.coords.len() / 2).flat_map(|k| p(k, notes)).collect();
            if pts.len() == 4 {
                Some(("line".into(), pts, None, false))
            } else {
                Some(("poly".into(), pts, None, false))
            }
        }
        "polygon" => {
            let pts: Vec<i32> = (0..sh.coords.len() / 2).flat_map(|k| p(k, notes)).collect();
            Some(("poly".into(), pts, None, true))
        }
        "rect" => {
            let b = box_in(&sh.coords, upi, &sh.kind, cell, notes);
            Some(("rect".into(), b.to_vec(), None, false))
        }
        "ellipse" | "arc" => {
            let (a, b) = (p(0, notes), p(1, notes));
            let (w, h) = ((b[0] - a[0]).abs(), (b[1] - a[1]).abs());
            let cx = (a[0] + b[0]) / 2;
            let cy = (a[1] + b[1]) / 2;
            if w != h {
                notes.review(
                    "format",
                    cell,
                    format!("{} with an elliptic box drawn as a polygon", sh.kind),
                );
                return Some((
                    "poly".into(),
                    ellipse_poly([a[0], a[1], b[0], b[1]]),
                    None,
                    true,
                ));
            }
            if sh.kind == "ellipse" {
                return Some(("circle".into(), vec![cx, cy, w / 2], None, false));
            }
            Some((
                "arc".into(),
                vec![cx, cy, w / 2],
                Some((sh.coords[4], sh.coords[5])),
                false,
            ))
        }
        _ => None,
    }
}

/// The 32-point polygon an ellipse in box `b` (two opposite corners, milli-units) comes
/// in as.
pub(crate) fn ellipse_poly(b: [i32; 4]) -> Vec<i32> {
    let (w, h) = ((b[2] - b[0]).abs(), (b[3] - b[1]).abs());
    let (cx, cy) = ((b[0] + b[2]) / 2, (b[1] + b[3]) / 2);
    let n = 32;
    let mut pts = Vec::new();
    for k in 0..n {
        let t = std::f64::consts::TAU * f64::from(k) / f64::from(n);
        pts.push(cx + (f64::from(w) / 2.0 * t.cos()).round() as i32);
        pts.push(cy + (f64::from(h) / 2.0 * t.sin()).round() as i32);
    }
    pts
}

/// Degrees (milli) of micro-radians.
fn mdeg(urad: i32) -> i32 {
    (f64::from(urad) / 1e6)
        .to_degrees()
        .mul_add(1000.0, 0.0)
        .round() as i32
}

/// Schemify `(start, sweep)` milli-degrees of a Virtuoso arc's counter-clockwise
/// `(start, stop)` micro-radians (y flipped).
pub(super) fn arc_angles(a0: i32, a1: i32) -> (i32, i32) {
    let d0 = mdeg(a0);
    let d1 = mdeg(a1);
    let sweep = (d1 - d0).rem_euclid(360_000);
    let sweep = if sweep == 0 { 360_000 } else { sweep };
    ((-d1).rem_euclid(360_000), sweep)
}

/// Merges arcs that continue one another on the same box (an export of a sweep > π).
pub(super) fn merge_arcs(shapes: &[&Shape]) -> Vec<Shape> {
    let mut v: Vec<Shape> = shapes.iter().map(|s| (*s).clone()).collect();
    let mut k = 0;
    while k < v.len() {
        if v[k].kind == "arc"
            && let Some(j) = (0..v.len()).find(|&j| {
                j != k
                    && v[j].kind == "arc"
                    && v[j].coords[..4] == v[k].coords[..4]
                    && v[j].layer == v[k].layer
                    && v[j].purpose == v[k].purpose
                    && v[j].coords[4] == v[k].coords[5]
            })
        {
            let stop = v[j].coords[5];
            v[k].coords[5] = stop;
            v.remove(j);
            if j < k {
                k -= 1;
            }
            continue;
        }
        k += 1;
    }
    v
}

/// Attributes of an imported graphic.
#[derive(Clone, Copy, Default)]
pub(super) struct Style {
    pub(super) closed: bool,
    pub(super) fill: bool,
    pub(super) dash: bool,
}

/// Writes one Schemify graphic record (rank 2 when `id` is `None`, else a rank-3 shape).
pub(super) fn push_graphic(
    doc: &mut Document<'static>,
    id: Option<&str>,
    form: &str,
    vals: &[i32],
    angles: Option<(i32, i32)>,
    style: Style,
) {
    let Style { closed, fill, dash } = style;
    match id {
        Some(id) => push_record(doc, RecordKind::Shape, &[id, form]),
        None => {
            doc.begin_record(match form {
                "line" => RecordKind::Line,
                "rect" => RecordKind::Rect,
                "arc" => RecordKind::Arc,
                "circle" => RecordKind::Circle,
                _ => RecordKind::Poly,
            });
        }
    }
    for v in vals {
        doc.push_field_coord(*v);
    }
    if let Some((a0, a1)) = angles {
        let (s, w) = arc_angles(a0, a1);
        doc.push_field(&coord_text(s));
        doc.push_field(&coord_text(w));
    }
    if dash {
        push_attr(doc, "dash", &["1"]);
    }
    if fill && form != "line" {
        push_attr(doc, "fill", &["true"]);
    }
    if closed {
        push_attr(doc, "closed", &["true"]);
    }
}

/// The fresh document of dump cell `k` (before the merge) and its stash text, if any.
pub(super) fn build(
    ctx: &Ctx<'_>,
    k: usize,
    pending: &mut Vec<Pending>,
    notes: &mut Notes,
) -> (Document<'static>, Option<String>) {
    let c = &ctx.dump.cells[k];
    let t = &ctx.targets[k];
    let kind = t.kind.expect("only written cells are built");
    let cell = format!("{}/{}", c.lib, c.name);
    let mut doc = Document::new(kind);
    let sym = c.symbol();
    let sch = c.cv("schematic");
    // Header.
    // The symbol's selection box as drawn (`vibox <x0> <y0> <x1> <y1>`, or `none`): export
    // draws it again rather than the box around the symbol's contents.
    if let Some(sv) = sym.filter(|_| kind != FileKind::Shbench) {
        let boxes: Vec<&Shape> = sv
            .shapes
            .iter()
            .filter(|s| s.layer == "instance" && s.purpose == "drawing" && s.kind == "rect")
            .collect();
        match boxes[..] {
            [] => push_attr(&mut doc, "vibox", &["none"]),
            [b] if b.coords.len() == 4 => {
                let v = box_in(&b.coords, sv.upi, "selection box", &cell, notes);
                doc.push_attr("vibox");
                for x in v {
                    doc.push_value_coord(x);
                }
            }
            _ => {}
        }
    }
    if kind == FileKind::Shprim {
        push_attr(
            &mut doc,
            "provenance",
            &[&format!("virtuoso {}/{}", c.lib, c.name)],
        );
    }
    // Inherited pins from net-expression labels (§6.2).
    let mut expr_pins: Vec<(String, String)> = Vec::new();
    if let Some(s) = sch {
        for l in &s.labels {
            if let Some((p, d)) = parse_netexpr(&l.text)
                && !expr_pins.iter().any(|x| x.0 == p)
            {
                expr_pins.push((p, d));
            }
        }
    }
    let mut params = c
        .cdf
        .as_ref()
        .map(|d| params_in(c, d, notes))
        .unwrap_or_default();
    // A symbol label may show an instance property the cell's CDF does not declare (live run:
    // Cadence's sample library shows `[@w]/[@l]` on cells without a CDF). Such a name becomes
    // an empty string param, so the label template and instances that set it stay valid.
    if let Some(sv) = sym.filter(|_| kind != FileKind::Shbench) {
        let label_set = c.cdf.as_ref().map_or("", Cdf::cds_param_set);
        let mut extra: BTreeSet<String> = BTreeSet::new();
        for l in sv
            .labels
            .iter()
            .filter(|l| l.kind != "normal" && l.layer != "pin")
        {
            let Some(tmpl) = nlp_in(&l.text, label_set) else {
                continue;
            };
            let Ok(tp) = schemify_expr::Template::parse(&tmpl) else {
                continue;
            };
            for id in tp.program().idents() {
                // Template built-ins (elab): not params.
                let builtin = matches!(
                    id,
                    "name" | "cell" | "pins" | "in_pins" | "out_pins" | "params"
                );
                if !builtin && !params.iter().any(|p| p.name == id) {
                    extra.insert(id.to_owned());
                }
            }
        }
        for n in extra {
            notes.review(
                "template",
                &cell,
                format!("symbol label shows `{n}`, which the CDF does not declare: declared empty"),
            );
            params.push(ParamIn {
                name: n,
                kind: "string".into(),
                default: String::new(),
                unit: None,
                doc: None,
                choices: Vec::new(),
                computed: None,
            });
        }
    }
    cap_params(c, &mut params, &cell, notes);
    let mut pins: Vec<PinRow> = match (sym, sch) {
        (Some(sv), _) => pins_in(c, sv, notes),
        (None, Some(sv)) if kind == FileKind::Shcomp => derived_pins(sv),
        _ => Vec::new(),
    };
    if kind != FileKind::Shbench {
        for (p, d) in &expr_pins {
            let name = net_from_virtuoso(p).text;
            if !pins.iter().any(|x| x.name == name) {
                pins.push(PinRow {
                    name,
                    dir: "inout".into(),
                    at: [0, 0],
                    inherit: Some(d.clone()),
                    alias: None,
                });
            }
        }
    } else if !expr_pins.is_empty() {
        notes.review(
            "name",
            &cell,
            "net expressions in a bench have no inherited pin to land on",
        );
    }
    // Impls.
    // (name, lang, formats, body)
    type ImplOut = (String, String, Vec<(String, String)>, Option<String>);
    let mut impls: Vec<ImplOut> = Vec::new();
    match kind {
        FileKind::Shcomp => impls.push(("schematic".into(), "schematic".into(), Vec::new(), None)),
        FileKind::Shprim => {
            let mut formats = Vec::new();
            if let Some(cdf) = &c.cdf {
                if let Some(si) = cdf.sim("spectre") {
                    formats.push((
                        "spectre".to_owned(),
                        spectre_format(c, si, &params, &pins, notes),
                    ));
                }
                if let Some(si) = cdf.sim("auCdl") {
                    formats.push(("cdl".to_owned(), cdl_format(c, si, &params, &pins, notes)));
                }
                if let Some(why) = siminfo_not_carried(cdf) {
                    notes.review("format", &cell, why);
                }
            }
            let spice = ctx
                .map
                .by_virtuoso(&c.lib, &c.name)
                .and_then(|r| r.spice.clone())
                .or_else(|| {
                    (c.lib == "analogLib")
                        .then(|| devices::by_alib(&c.name).map(|(d, w)| d.ngspice[w].to_owned()))
                        .flatten()
                });
            if let Some(s) = spice {
                formats.insert(0, ("spice".into(), s));
            }
            for f in &mut formats {
                f.1 = optional_strings(&f.1, &params);
            }
            // A spectre `iprobe` (basic's cds_thru, live ICADVM20.1: `T0 (x y) iprobe`) is a
            // 0 V source in SPICE: the same short, and the same probe of its current.
            let probe = formats
                .iter()
                .find(|f| f.0 == "spectre")
                .and_then(|f| f.1.strip_prefix("{name} (")?.strip_suffix(") iprobe"))
                .map(|pins| format!("V{{name}} {pins} 0"));
            if let Some(p) = probe.filter(|_| !formats.iter().any(|f| f.0 == "spice")) {
                formats.insert(0, ("spice".into(), p));
            }
            if formats.is_empty() {
                // basic's patch has no simInfo (live ICADVM20.1): si renames nets by it.
                notes.review(
                    "format",
                    &cell,
                    "no simInfo for spectre or auCdl: no netlist format (E013 when netlisted)",
                );
            }
            impls.push(("spice".into(), "spice".into(), formats, None));
        }
        _ => {}
    }
    for tv in &c.textviews {
        let (name, lang) = match tv.view.as_str() {
            "veriloga" => ("veriloga", "veriloga"),
            "functional" | "verilog" => ("verilog", "verilog"),
            other => {
                // A spectre or auCdl text view is the netlist Virtuoso stops at: a library
                // made from this import gets a symbol copy there instead (gaps2 S4).
                let stop = if matches!(other, "spectre" | "auCdl") {
                    ": a new library gets the symbol as this stop view, not this netlist"
                } else {
                    ""
                };
                notes.review(
                    "format",
                    &cell,
                    format!("text view {other} ({}) not imported{stop}", tv.file),
                );
                continue;
            }
        };
        if kind == FileKind::Shbench {
            notes.review("format", &cell, format!("text view {} of a bench", tv.view));
            continue;
        }
        impls.push((name.into(), lang.into(), Vec::new(), Some(tv.text.clone())));
    }
    if impls.len() > 1 {
        push_attr(&mut doc, "default", &[&impls[0].0]);
    }
    // Rank 1.
    for p in &pins {
        doc.begin_record(RecordKind::Pin);
        doc.push_field(&p.name);
        doc.push_field(&p.dir);
        doc.push_field_coord(p.at[0]);
        doc.push_field_coord(p.at[1]);
        if let Some(i) = &p.inherit {
            push_attr(&mut doc, "inherit", &[i]);
            if let Some(v) = sym.and_then(|sv| inherit_prop(sv, &p.name)) {
                push_attr(&mut doc, "vinherit", &[&v]);
            }
        }
        if let Some(a) = &p.alias {
            push_attr(&mut doc, "alias", &[a]);
        }
        // The pin's name label as drawn, where export would draw another (§6.2); after the
        // format's own attributes, where the writer puts unknown ones (the fingerprint).
        if let Some(sv) = sym.filter(|_| p.alias.is_none())
            && let Some(v) = pin_label_as_drawn(sv, &p.name, p.at, &cell, notes)
        {
            push_attr(&mut doc, "vplabel", &v);
        }
        // Its pin rectangle as drawn, when not export's ±PIN_HALF square (`vpfig <w> <h>`).
        if let Some(f) = sym
            .filter(|_| p.alias.is_none())
            .and_then(|sv| main_pin_fig(sv, &p.name).map(|f| (sv, f)))
        {
            let (sv, f) = f;
            // pins_in already reported any rounding of this rectangle: a scratch report.
            let scratch = &mut Notes::default();
            let a = pt([f.b[0], f.b[1]], sv.upi, "pin figure", &cell, scratch);
            let b = pt([f.b[2], f.b[3]], sv.upi, "pin figure", &cell, scratch);
            let (w, h) = ((b[0] - a[0]).abs(), (b[1] - a[1]).abs());
            let usual = 2 * crate::virtuoso::write::PIN_HALF;
            if (w, h) != (usual, usual) {
                push_attr(
                    &mut doc,
                    "vpfig",
                    &[profile::coord_text(w), profile::coord_text(h)],
                );
            }
        }
    }
    if kind != FileKind::Shbench {
        for p in &params {
            doc.begin_record(RecordKind::Param);
            doc.push_field(&p.name);
            doc.push_field(&p.default);
            if p.kind != "number" {
                push_attr(&mut doc, "kind", &[&p.kind]);
            }
            if let Some(u) = p.unit {
                push_attr(&mut doc, "unit", &[u]);
            }
            if p.kind == "choice" && !p.choices.is_empty() {
                push_attr(&mut doc, "choices", &p.choices);
            }
            // A radio CDF param is a choice drawn as radio buttons (`vradio`, gaps2 S17), so
            // export writes radio, not cyclic, for an edited cell too.
            if c.cdf
                .as_ref()
                .is_some_and(|d| d.params.iter().any(|q| q.name == p.name && q.ty == "radio"))
            {
                push_attr(&mut doc, "vradio", &["true"]);
            }
            if let Some(d) = &p.doc {
                push_attr(&mut doc, "doc", &[d]);
            }
            if let Some(e) = &p.computed {
                push_attr(&mut doc, "computed", &[e]);
            }
        }
    }
    for (name, lang, formats, _) in &impls {
        doc.begin_record(RecordKind::Impl);
        doc.push_field(name);
        doc.push_field(lang);
        for (d, f) in formats {
            push_attr(&mut doc, "format", &[d, f]);
        }
    }
    // Rank 2: the symbol.
    let mut stash_text: Option<String> = None;
    if let Some(sv) = sym {
        let mut tally = Tally::default();
        tally.out += sv.terms.len() + sv.pinfigs.len();
        let label_set = c.cdf.as_ref().map_or("", Cdf::cds_param_set);
        let body: Vec<&Shape> = sv
            .shapes
            .iter()
            .filter(|s| !(s.layer == "instance" && s.purpose == "drawing"))
            .collect();
        let selboxes = sv.shapes.len() - body.len();
        if selboxes > 0 {
            notes.dropped("selection-box", &cell, format!("{selboxes} object(s)"));
            tally.reported += selboxes;
        }
        let merged = merge_arcs(&body);
        tally.out += body.len();
        for sh in &merged {
            match shape_in(sh, sv.upi, &cell, notes) {
                Some((form, vals, ang, closed)) => push_graphic(
                    &mut doc,
                    None,
                    &form,
                    &vals,
                    ang,
                    Style {
                        closed,
                        fill: sh.purpose == "drawing1",
                        dash: false,
                    },
                ),
                None => notes.dropped("display-only", &cell, format!("symbol {}", sh.kind)),
            }
        }
        let mut pin_labels = 0;
        for l in &sv.labels {
            if l.layer == "pin" && (l.purpose == "label" || l.purpose == "annotate") {
                pin_labels += 1;
                continue;
            }
            let tmpl = match l.kind.as_str() {
                "normal" => Some(ael::literal_to_template(&l.text)),
                _ => nlp_in(&l.text, label_set),
            };
            tally.out += 1;
            let Some(tmpl) = tmpl.filter(|t| !t.is_empty()) else {
                let mode = c
                    .cdf
                    .as_ref()
                    .and_then(|d| d.attrs.iter().find(|a| a.0 == "paramDisplayMode"))
                    .filter(|_| l.text.starts_with("cdsParam("));
                notes.review(
                    "template",
                    &cell,
                    match mode {
                        Some((_, m)) => format!(
                            "symbol label `{}` dropped (paramDisplayMode {m}: no params shown)",
                            l.text
                        ),
                        None => format!("symbol label `{}` dropped", l.text),
                    },
                );
                continue;
            };
            // A group's fallback (`[@w:w=%:w?]`) is what Virtuoso shows for an unset value;
            // the template has no such branch (live ICADVM20.1: no SKILL evaluates NLP
            // labels, so the drawn text goes back as it was, `vlabel`).
            let fb = if l.kind == "NLP" {
                nlp_fallbacks(&l.text)
            } else {
                Vec::new()
            };
            if !fb.is_empty() {
                notes.review(
                    "template",
                    &cell,
                    format!(
                        "`{}` → `{tmpl}`: the fallback(s) `{}` for an unset value not shown",
                        l.text,
                        fb.join("`, `")
                    ),
                );
            }
            // Virtuoso writes `w:7u` for a value at the CDF default and `w=7u` for one the
            // instance sets (skcompref cdsParam); the template has one form.
            if l.text.starts_with("cdsParam(") && tmpl.contains('=') {
                notes.review(
                    "template",
                    &cell,
                    format!(
                        "`{}` → `{tmpl}`: Virtuoso shows `:` for a value at its default",
                        l.text
                    ),
                );
            }
            let at = pt(l.xy, sv.upi, "label", &cell, notes);
            doc.begin_record(RecordKind::Stext);
            doc.push_field_coord(at[0]);
            doc.push_field_coord(at[1]);
            doc.push_field(orient_in(&l.orient).as_str());
            doc.push_field(&tmpl);
            push_attr(
                &mut doc,
                "size",
                &[&coord_text(size_milli(l.height, sv.upi))],
            );
            if l.justify.ends_with("Center") {
                doc.push_attr("hcenter");
            }
            if l.justify.starts_with("center") {
                doc.push_attr("vcenter");
            }
            // As drawn, for export while the template stays this one (§6.2).
            let drawn = [
                &l.layer, &l.purpose, &l.justify, &l.font, &l.kind, &l.text, &tmpl,
            ];
            push_attr(&mut doc, "vlabel", &drawn);
            if let Some(f) = flags_of(l) {
                push_attr(&mut doc, "vflags", &f);
            }
        }
        if pin_labels > 0 {
            notes.dropped("pin-label", &cell, format!("{pin_labels} object(s)"));
            tally.reported += pin_labels;
        }
        let st = cv_props(sv, &cell, notes, &mut tally);
        if kind == FileKind::Shprim {
            stash_text = st;
        }
        report_rest(sv, &cell, notes, &mut tally);
        assert_eq!(
            tally.out + tally.reported,
            sv.objects(),
            "{cell} symbol: count conservation"
        );
    }
    if kind == FileKind::Shcomp && sym.is_none() {
        derived_symbol(&mut doc, &pins);
    }
    // Rank 3: the schematic.
    if let Some(sv) = sch {
        let st = schematic_in(ctx, k, sv, &mut doc, &pins, pending, notes);
        if kind != FileKind::Shprim {
            stash_text = st;
        }
    }
    // Rank 5: text view bodies.
    for (name, _, _, body) in &impls {
        if let Some(b) = body {
            doc.begin_record(RecordKind::Body);
            doc.push_field(name);
            for line in b.strip_suffix('\n').unwrap_or(b).split('\n') {
                doc.push_verbatim(line.trim_end_matches('\r'));
            }
        }
    }
    (doc, stash_text)
}

/// Pins of a schematic without a symbol: one per schematic pin, inputs on the left,
/// outputs and inouts on the right, 20 units apart (§6.1).
fn derived_pins(sv: &Cv) -> Vec<PinRow> {
    let mut terms: Vec<(String, String)> = sv
        .spins
        .iter()
        .map(|s| {
            (
                net_from_virtuoso(&s.term).text,
                profile::dir_from_virtuoso(&s.dir).to_owned(),
            )
        })
        .collect();
    terms.sort();
    terms.dedup_by(|a, b| a.0 == b.0);
    box_pins(terms)
}

/// Pins `(name, dir)` in the order given, laid out for [`derived_symbol`].
fn box_pins(terms: Vec<(String, String)>) -> Vec<PinRow> {
    let (mut l, mut r) = (0, 0);
    terms
        .into_iter()
        .map(|(name, dir)| {
            let left = dir == "in";
            let n = if left { &mut l } else { &mut r };
            let at = [if left { -40_000 } else { 40_000 }, 20_000 * *n];
            *n += 1;
            PinRow {
                name,
                dir,
                at,
                inherit: None,
                alias: None,
            }
        })
        .collect()
}

/// Writes a pin record per `(name, dir)`, in order, and their box symbol: the shape of the
/// editor's "Symbol from ports" (inputs left, the rest right, a box, `{name}`).
#[cfg(feature = "cli")]
pub(crate) fn box_symbol(doc: &mut Document<'static>, terms: Vec<(String, String)>) {
    let pins = box_pins(terms);
    for p in &pins {
        doc.begin_record(RecordKind::Pin);
        doc.push_field(&p.name);
        doc.push_field(&p.dir);
        doc.push_field_coord(p.at[0]);
        doc.push_field_coord(p.at[1]);
    }
    derived_symbol(doc, &pins);
}

fn derived_symbol(doc: &mut Document<'static>, pins: &[PinRow]) {
    let rows = pins.iter().map(|p| p.at[1]).max().unwrap_or(0);
    doc.begin_record(RecordKind::Rect);
    for v in [-20_000, -10_000, 20_000, rows + 10_000] {
        doc.push_field_coord(v);
    }
    for p in pins {
        doc.begin_record(RecordKind::Line);
        let edge = if p.at[0] < 0 { -20_000 } else { 20_000 };
        for v in [p.at[0].min(edge), p.at[1], p.at[0].max(edge), p.at[1]] {
            doc.push_field_coord(v);
        }
    }
    doc.begin_record(RecordKind::Stext);
    doc.push_field_coord(-20_000);
    doc.push_field_coord(-20_000);
    doc.push_field("R0");
    doc.push_field("{name}");
}

/// The main pin figure of symbol pin `name`: its terminal's first rectangle without a
/// `schemifyPin` prop (those are alias pins, §6.2).
pub(super) fn main_pin_fig<'a>(sv: &'a Cv, name: &str) -> Option<&'a PinFig> {
    let alias_fig = |r: u32| sv.props.iter().any(|p| p.r == r && p.name == "schemifyPin");
    let fig = sv
        .pinfigs
        .iter()
        .find(|f| net_from_virtuoso(&f.term).text == name && !alias_fig(f.r));
    debug_assert!(fig.is_none_or(|f| f.dir.is_empty()), "a symbol pin figure");
    fig
}

/// Params a Schemify cell holds at most (its doc table capacity: a load past it panics,
/// `Params exceeds its capacity`; analogLib `port` has 421 CDF params, live).
pub(super) const PARAMS_MAX: usize = 1 << 8;

/// Cuts `params` to [`PARAMS_MAX`], in CDF order, keeping first those a netlist or label
/// reads (any simulator's `instParameters`/`otherParameters`, `paramLabelSet`, a computed
/// rule), then those with a default; the rest are reported `dropped:param` (an instance
/// setting one keeps its override, as for any undeclared name).
fn cap_params(c: &DCell, params: &mut Vec<ParamIn>, cell: &str, notes: &mut Notes) {
    if params.len() <= PARAMS_MAX {
        return;
    }
    let mut read: BTreeSet<&str> = BTreeSet::new();
    if let Some(cdf) = &c.cdf {
        for si in &cdf.siminfo {
            for k in ["instParameters", "otherParameters"] {
                if let Some(SimVal::List(l)) = si.get(k) {
                    read.extend(l.iter().map(String::as_str));
                }
            }
        }
        read.extend(cdf.label_set.split_whitespace());
    }
    let rank = |p: &ParamIn| {
        if read.contains(p.name.as_str()) || p.computed.is_some() {
            0
        } else if !p.default.is_empty() {
            1
        } else {
            2
        }
    };
    let mut order: Vec<usize> = (0..params.len()).collect();
    order.sort_by_key(|&k| rank(&params[k]));
    let mut keep = vec![false; params.len()];
    for &k in &order[..PARAMS_MAX] {
        keep[k] = true;
    }
    let mut k = 0;
    let mut gone: Vec<String> = Vec::new();
    params.retain(|p| {
        k += 1;
        if !keep[k - 1] {
            gone.push(p.name.clone());
        }
        keep[k - 1]
    });
    assert_eq!(params.len(), PARAMS_MAX);
    notes.dropped(
        "param",
        cell,
        format!(
            "{} CDF params past Schemify's {PARAMS_MAX} per cell (unread by any netlist, \
             label or rule first): {}",
            gone.len(),
            gone.join(" ")
        ),
    );
}
