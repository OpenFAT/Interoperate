//! Schemify symbol → Virtuoso symbol cellview (spec `interop.md` §6.2 symbol rows, §3.5 labels).

use super::*;

// ---------------------------------------------------------------------------------------
// Symbol (§6.2 symbol rows, §3.5 labels)
// ---------------------------------------------------------------------------------------

pub(super) fn symbol_out(c: &Cell, ids: &mut Ids, notes: &mut Notes) -> Cv {
    let mut cv = Cv {
        view: "symbol".into(),
        vtype: "schematicSymbol".into(),
        upi: UPI,
        ..Cv::default()
    };
    let mut bbox: Option<[i32; 4]> = None;
    let grow = |p: [i32; 2], bbox: &mut Option<[i32; 4]>| {
        let b = bbox.get_or_insert([p[0], p[1], p[0], p[1]]);
        b[0] = b[0].min(p[0]);
        b[1] = b[1].min(p[1]);
        b[2] = b[2].max(p[0]);
        b[3] = b[3].max(p[1]);
    };
    let pins = sym_pins(c);
    let pin_recs: Vec<u32> = c.records(RecordKind::Pin).collect();
    for (k, &pin_rec) in pin_recs.iter().enumerate() {
        let at = conv(pins.at[k], &format!("pin {}", pins.name[k]), c, notes);
        let term = vname(pins.alias[k].as_deref().unwrap_or(&pins.name[k]));
        if pins.alias[k].is_none() {
            let netexpr = pins.inherit[k].as_ref().map_or(String::new(), |net| {
                format!("[@{}:%:{}]", term, net_to_virtuoso(net, true).0)
            });
            cv.terms.push(Term {
                name: term.clone(),
                dir: dir_to_virtuoso(&pins.dir[k]).into(),
                netexpr,
            });
        }
        let r = ids.take();
        // The rectangle as Virtuoso drew it (`vpfig <w> <h>`), else ±PIN_HALF.
        let [hw, hh] = match c.attr_values(pin_rec, "vpfig")[..] {
            [w, h] => [w, h].map(|x| parse_coord(x).map_or(PIN_HALF, |v| v / 2)),
            _ => [PIN_HALF, PIN_HALF],
        };
        cv.pinfigs.push(PinFig {
            r,
            term: term.clone(),
            b: [at[0] - hw, at[1] - hh, at[0] + hw, at[1] + hh],
            dir: String::new(),
        });
        grow([at[0] - hw, at[1] - hh], &mut bbox);
        grow([at[0] + hw, at[1] + hh], &mut bbox);
        if pins.alias[k].is_some() {
            cv.props
                .push(Prop::string(r, "schemifyPin", pins.name[k].clone()));
        }
        // The name label as Virtuoso drew it (`vplabel <dx> <dy> <orient> <justify> <font>
        // <height>` from the pin, or `vplabel none`), else export's.
        let v: Vec<&str> = c.attr_values(pin_rec, "vplabel");
        let (xy, orient, justify, font, height) = match v[..] {
            ["none"] => continue,
            [dx, dy, o, j, f, h] => {
                let d = [parse_coord(dx).unwrap_or(0), parse_coord(dy).unwrap_or(0)];
                let p = [pins.at[k][0] + d[0], pins.at[k][1] + d[1]];
                (
                    conv(p, "pin label", c, notes),
                    o,
                    j,
                    f,
                    h.parse().unwrap_or(LABEL_HEIGHT / 2),
                )
            }
            _ => (at, "R0", "centerLeft", "stick", LABEL_HEIGHT / 2),
        };
        let o = Orient::parse(orient).unwrap_or(Orient::R0);
        let style = ["pin", "label", justify, font, "normal"];
        cv.labels
            .push(label(ids.take(), r, term, xy, o, height, style));
    }
    for r in 1..c.doc.record_count() {
        let kind = c.doc.record_kind(r);
        if kind.rank() != 2 {
            continue;
        }
        if kind == RecordKind::Stext {
            if let Some(l) = stext_out(c, r, ids, notes) {
                grow(l.xy, &mut bbox);
                cv.labels.push(l);
            }
            continue;
        }
        let fill = c.attr(r, "fill") == Some("true");
        for sh in graphic_out(
            c,
            r,
            kind,
            1,
            "device",
            if fill { "drawing1" } else { "drawing" },
            ids,
            notes,
        ) {
            for p in sh
                .coords
                .chunks(2)
                .take(if sh.kind == "arc" { 2 } else { usize::MAX })
            {
                if p.len() == 2 {
                    grow([p[0], p[1]], &mut bbox);
                }
            }
            cv.shapes.push(sh);
        }
    }
    // The selection box as Virtuoso drew it (`vibox`), else the box around the contents.
    // ponytail: a kept box goes stale when the symbol grows in Schemify; union it with the
    // contents' box if that bites.
    let vibox: Vec<&str> = c.attr_values(0, "vibox");
    let bbox = match vibox[..] {
        ["none"] => None,
        [x0, y0, x1, y1] => {
            let v: Vec<i32> = [x0, y0, x1, y1]
                .iter()
                .map(|x| parse_coord(x).unwrap_or(0))
                .collect();
            let (p, q) = (
                conv([v[0], v[1]], "selection box", c, notes),
                conv([v[2], v[3]], "selection box", c, notes),
            );
            Some(norm_box(p, q))
        }
        _ => bbox,
    };
    if let Some(b) = bbox {
        cv.shapes.push(Shape {
            r: ids.take(),
            kind: "rect".into(),
            layer: "instance".into(),
            purpose: "drawing".into(),
            coords: b.to_vec(),
            width: 0,
        });
    }
    cv
}

/// Virtuoso justification of a Schemify text anchor (`hcenter`/`vcenter` flags).
pub fn justify(hcenter: bool, vcenter: bool) -> &'static str {
    match (hcenter, vcenter) {
        (false, false) => "upperLeft",
        (true, false) => "upperCenter",
        (false, true) => "centerLeft",
        (true, true) => "centerCenter",
    }
}

/// A label row of object `r` glued to `parent` (0: none). `style` is `[layer, purpose,
/// justify, font, kind]`, the order the `vlabel` attribute keeps them in, so a label read
/// back from `vlabel` and one export draws go through the same door.
pub(super) fn label(
    r: u32,
    parent: u32,
    text: String,
    xy: [i32; 2],
    o: Orient,
    height: i32,
    style: [&str; 5],
) -> Label {
    debug_assert!(r > 0 && r != parent, "a label is its own object");
    let [layer, purpose, justify, font, kind] = style;
    Label {
        r,
        parent,
        text,
        xy,
        orient: profile::orient_to_virtuoso(o).into(),
        justify: justify.into(),
        font: font.into(),
        height,
        kind: kind.into(),
        layer: layer.into(),
        purpose: purpose.into(),
    }
}

/// `height = size × 10_000` milli (§3.5); `size` absent → `default_milli`.
pub(super) fn height(c: &Cell, r: u32, default_milli: i32) -> i32 {
    let s = c
        .attr(r, "size")
        .and_then(|t| parse_coord(t).ok())
        .unwrap_or(default_milli);
    // Rounded as Virtuoso stores it, so the image predicts the read-back and an untouched
    // symbol keeps its stashed sizes (§7.3).
    profile::height_of_size(s)
}

/// Escapes NLP-reserved characters of literal text.
pub fn nlp_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if matches!(ch, '\\' | '[' | ']' | ':' | '%') {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

pub(super) fn stext_out(c: &Cell, r: u32, ids: &mut Ids, notes: &mut Notes) -> Option<Label> {
    let at = conv(c.point(r, 0), "stext", c, notes);
    let o = c.orient(r, 2);
    let t = c.field(r, 3);
    // A label from Virtuoso whose template is still the one it imported as goes back as
    // drawn: `vlabel <layer> <purpose> <justify> <font> <kind> <text> <template>`.
    let v: Vec<&str> = c.attr_values(r, "vlabel");
    if let [layer, purpose, justify, font, kind, text, tmpl] = v[..]
        && tmpl == t
    {
        let style = [layer, purpose, justify, font, kind];
        let h = height(c, r, STEXT_SIZE_MILLI);
        return Some(label(ids.take(), 0, text.into(), at, o, h, style));
    }
    let items = template_items(t)?;
    let params: Vec<&str> = c
        .records(RecordKind::Param)
        .map(|p| c.field(p, 0))
        .collect();
    let (text, kind, layer, purpose) = match items.as_slice() {
        [(true, h)] if h == "name" => ("[@instanceName]".to_owned(), "NLP", "instance", "label"),
        [(true, h)] if h == "cell" => ("[@partName]".to_owned(), "NLP", "device", "label"),
        [] => return None,
        _ if items.iter().all(|x| !x.0) => (
            items.iter().map(|x| x.1.as_str()).collect(),
            "normal",
            "annotate",
            "drawing",
        ),
        _ => {
            let Some(text) = template_to_nlp(&items, &params) else {
                notes.review("template", &c.path, format!("stext `{t}` dropped"));
                return None;
            };
            (text, "NLP", "annotate", "drawing")
        }
    };
    let j = justify(
        c.doc.find_attr(r, "hcenter").is_some(),
        c.doc.find_attr(r, "vcenter").is_some(),
    );
    let h = height(c, r, STEXT_SIZE_MILLI);
    Some(label(
        ids.take(),
        0,
        text,
        at,
        o,
        h,
        [layer, purpose, j, "stick", kind],
    ))
}

/// The NLP text of a symbol label template with holes (§3.5): every hole a parameter of
/// `params`, bare or through the display formatter `eng(p)` (NLP shows the value as
/// entered), one `[@p:pre%post]` group per hole (`[@p]` when both are empty), the literal
/// before the first hole its `pre`, each literal after a hole its `post`, NLP-escaped.
/// `None` when a hole is anything else. Import's `read::nlp_in` reads it back.
pub(crate) fn template_to_nlp(items: &[(bool, String)], params: &[&str]) -> Option<String> {
    pub(super) fn param<'a>(h: &'a str, params: &[&str]) -> Option<&'a str> {
        let p = h
            .strip_prefix("eng(")
            .and_then(|x| x.strip_suffix(')'))
            .unwrap_or(h)
            .trim();
        params.contains(&p).then_some(p)
    }
    let names = items
        .iter()
        .filter(|x| x.0)
        .map(|x| param(&x.1, params))
        .collect::<Option<Vec<&str>>>()?;
    let first = items.iter().position(|x| x.0)?;
    let mut pre: String = items[..first].iter().map(|x| x.1.as_str()).collect();
    let mut text = String::new();
    let mut k = first;
    for p in &names {
        let next = items[k + 1..]
            .iter()
            .position(|x| x.0)
            .map_or(items.len(), |n| k + 1 + n);
        let post: String = items[k + 1..next].iter().map(|x| x.1.as_str()).collect();
        if pre.is_empty() && post.is_empty() {
            text.push_str(&format!("[@{p}]"));
        } else {
            text.push_str(&format!(
                "[@{p}:{}%{}]",
                nlp_escape(&pre),
                nlp_escape(&post)
            ));
        }
        pre.clear();
        k = next;
    }
    debug_assert!(text.starts_with("[@"));
    Some(text)
}

/// Micro-radians of `deg` degrees, normalized into `[0, 2π)`.
pub fn urad(deg: f64) -> i32 {
    let d = deg.rem_euclid(360.0);
    (d.to_radians() * 1e6).round() as i32
}

/// A rank-2 graphic or a rank-3 shape (fields from `from`) as Virtuoso shapes.
#[allow(clippy::too_many_arguments)]
pub(super) fn graphic_out(
    c: &Cell,
    r: u32,
    kind: RecordKind,
    from: usize,
    layer: &str,
    purpose: &str,
    ids: &mut Ids,
    notes: &mut Notes,
) -> Vec<Shape> {
    let form = match kind {
        RecordKind::Line => "line",
        RecordKind::Rect => "rect",
        RecordKind::Arc => "arc",
        RecordKind::Circle => "circle",
        RecordKind::Poly => "poly",
        _ => c.field(r, 1),
    };
    let first = if kind == RecordKind::Shape {
        from + 1
    } else {
        0
    };
    let raw: Vec<&str> = c.doc.fields(r).skip(first).collect();
    let v = coords(c, r, first);
    let shape = |kind: &str, coords: Vec<i32>, ids: &mut Ids| Shape {
        r: ids.take(),
        kind: kind.into(),
        layer: layer.into(),
        purpose: purpose.into(),
        coords,
        width: 0,
    };
    let p = |k: usize, notes: &mut Notes| conv([v[2 * k], v[2 * k + 1]], form, c, notes);
    match form {
        "line" if v.len() == 4 => {
            let (a, b) = (p(0, notes), p(1, notes));
            vec![shape("line", vec![a[0], a[1], b[0], b[1]], ids)]
        }
        "rect" if v.len() == 4 => {
            let (a, b) = (p(0, notes), p(1, notes));
            vec![shape("rect", norm_box(a, b).to_vec(), ids)]
        }
        "circle" if v.len() == 3 => {
            let o = conv([v[0], v[1]], form, c, notes);
            // Virtuoso stores the box in whole DBUs, rounded outward (live ICADVM20.1: the
            // radius-2.5 bubble of a pmos at y 0 came back with box y -3..3), so the export
            // writes that box and the image predicts the read-back; the stash keeps the
            // circle (§7.3).
            let g = (MILLI_PER_INCH / i64::from(DBU)) as i32;
            let (lo, hi) = (
                |x: i32| x.div_euclid(g) * g,
                |x: i32| -(-x).div_euclid(g) * g,
            );
            vec![shape(
                "ellipse",
                vec![
                    lo(o[0] - v[2]),
                    lo(o[1] - v[2]),
                    hi(o[0] + v[2]),
                    hi(o[1] + v[2]),
                ],
                ids,
            )]
        }
        "arc" if v.len() == 5 => {
            let o = conv([v[0], v[1]], form, c, notes);
            let rad = v[2];
            let start: f64 = raw[3].parse().unwrap_or(0.0);
            let mut sweep: f64 = raw[4].parse().unwrap_or(0.0);
            let mut start = start;
            if sweep < 0.0 {
                start += sweep;
                sweep = -sweep;
            }
            let bx = vec![o[0] - rad, o[1] - rad, o[0] + rad, o[1] + rad];
            if sweep >= 360.0 {
                return vec![shape("ellipse", bx, ids)];
            }
            // Screen-clockwise in y-down is counter-clockwise in y-up with negated angles.
            let a0 = -(start + sweep);
            let a1 = -start;
            if sweep > 180.0 {
                let mid = a0 + sweep / 2.0;
                let mut s1 = bx.clone();
                s1.extend([urad(a0), urad(mid)]);
                let mut s2 = bx;
                s2.extend([urad(mid), urad(a1)]);
                vec![shape("arc", s1, ids), shape("arc", s2, ids)]
            } else {
                let mut s1 = bx;
                s1.extend([urad(a0), urad(a1)]);
                vec![shape("arc", s1, ids)]
            }
        }
        "poly" if v.len() >= 4 && v.len().is_multiple_of(2) => {
            let closed = c.attr(r, "closed") == Some("true");
            let mut pts: Vec<[i32; 2]> = (0..v.len() / 2).map(|k| p(k, notes)).collect();
            if closed && pts.len() > 1 && pts.first() == pts.last() {
                pts.pop();
            }
            let flat: Vec<i32> = pts.iter().flat_map(|q| [q[0], q[1]]).collect();
            if closed && pts.len() >= 3 {
                vec![shape("polygon", flat, ids)]
            } else {
                vec![shape("line", flat, ids)]
            }
        }
        _ => {
            notes.dropped(
                "display-only",
                &c.path,
                format!("{form} with {} values", v.len()),
            );
            Vec::new()
        }
    }
}
