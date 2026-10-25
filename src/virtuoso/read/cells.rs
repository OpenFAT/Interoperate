//! One Virtuoso cell → one Schemify document (spec `interop.md` §6.1, §7.3): the build, derived
//! interfaces, cellview props and stash.

use super::*;

// ---------------------------------------------------------------------------------------
// Cell build
// ---------------------------------------------------------------------------------------

/// Per-cellview accounting for count conservation (§12).
#[derive(Default)]
pub(super) struct Tally {
    pub(super) out: usize,
    pub(super) reported: usize,
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
    let sym = c.cv("symbol");
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
        let label_set = c.cdf.as_ref().map_or("", |d| d.label_set.as_str());
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
            });
        }
    }
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
    pub(super) type ImplOut = (String, String, Vec<(String, String)>, Option<String>);
    let mut impls: Vec<ImplOut> = Vec::new();
    match kind {
        FileKind::Shcomp => impls.push(("schematic".into(), "schematic".into(), Vec::new(), None)),
        FileKind::Shprim => {
            let mut formats = Vec::new();
            if let Some(cdf) = &c.cdf {
                if let Some(si) = cdf.sim("spectre") {
                    formats.push(("spectre".to_owned(), spectre_format(c, si, &params, &pins)));
                }
                if let Some(si) = cdf.sim("auCdl") {
                    formats.push(("cdl".to_owned(), cdl_format(c, si, &params, &pins, notes)));
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
            impls.push(("spice".into(), "spice".into(), formats, None));
        }
        _ => {}
    }
    for tv in &c.textviews {
        let (name, lang) = match tv.view.as_str() {
            "veriloga" => ("veriloga", "veriloga"),
            "functional" | "verilog" => ("verilog", "verilog"),
            other => {
                notes.review("format", &cell, format!("text view {other} not imported"));
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
            if let Some(d) = &p.doc {
                push_attr(&mut doc, "doc", &[d]);
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
        let label_set = c.cdf.as_ref().map_or("", |d| d.label_set.as_str());
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
                notes.review(
                    "template",
                    &cell,
                    format!("symbol label `{}` dropped", l.text),
                );
                continue;
            };
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
pub(super) fn derived_pins(sv: &Cv) -> Vec<PinRow> {
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

pub(super) fn derived_symbol(doc: &mut Document<'static>, pins: &[PinRow]) {
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

/// Cellview properties: the stash (`schemifyRecords<k>`, joined in numeric order) and
/// everything else reported.
pub(super) fn cv_props(
    sv: &Cv,
    cell: &str,
    notes: &mut Notes,
    tally: &mut Tally,
) -> Option<String> {
    for p in sv.props.iter().filter(|p| p.r == 0) {
        if stash_chunk(&p.name).is_some() {
            tally.out += 1;
        } else {
            notes.dropped(
                "display-only",
                cell,
                format!("cellview property {}", p.name),
            );
            tally.reported += 1;
        }
    }
    stash_of(sv)
}

/// The chunk number of a `schemifyRecords<k>` property name.
pub(super) fn stash_chunk(name: &str) -> Option<u32> {
    name.strip_prefix("schemifyRecords")?.parse().ok()
}

/// The stash text of a cellview: its `schemifyRecords<k>` properties joined in numeric order.
pub(super) fn stash_of(sv: &Cv) -> Option<String> {
    let mut chunks: Vec<(u32, &str)> = sv
        .props
        .iter()
        .filter(|p| p.r == 0)
        .filter_map(|p| Some((stash_chunk(&p.name)?, p.value.as_str())))
        .collect();
    chunks.sort();
    (!chunks.is_empty()).then(|| chunks.iter().map(|c| c.1).collect())
}

/// Reports conns and unknown records; counts properties of objects (consumed by
/// their owners' builders) as out.
pub(super) fn report_rest(sv: &Cv, cell: &str, notes: &mut Notes, tally: &mut Tally) {
    for c in &sv.conns {
        notes.dropped(
            "display-only",
            cell,
            format!("conn {} {} (loader-only)", c.term, c.net),
        );
    }
    for r in &sv.unknown {
        notes.dropped(
            "unknown-record",
            cell,
            format!("`{}` on line {}", r.tag, r.line),
        );
    }
    tally.reported += sv.conns.len() + sv.unknown.len();
    tally.out += sv.props.iter().filter(|p| p.r != 0).count();
}

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
