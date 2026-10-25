//! Virtuoso symbol cellview → Schemify symbol (spec `interop.md` §6.2 symbol rows, §3.5 labels):
//! shapes, arcs, NLP labels.

use super::*;

/// Text height (1/`upi` inch) → Schemify `size` milli (§3.5).
pub(super) fn size_milli(height: i32, upi: i32) -> i32 {
    profile::size_of_height(profile::height_from_virtuoso(height, upi))
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
                let n = 32;
                let mut pts = Vec::new();
                for k in 0..n {
                    let t = std::f64::consts::TAU * f64::from(k) / f64::from(n);
                    pts.push(cx + (f64::from(w) / 2.0 * t.cos()).round() as i32);
                    pts.push(cy + (f64::from(h) / 2.0 * t.sin()).round() as i32);
                }
                return Some(("poly".into(), pts, None, true));
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

/// Degrees (milli) of micro-radians.
pub(super) fn mdeg(urad: i32) -> i32 {
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

/// The `[@…]` groups of NLP text that is nothing but groups (`\` escapes inside); `None`
/// when text sits outside a group or a group is unclosed.
pub(super) fn nlp_groups(t: &str) -> Option<Vec<&str>> {
    let mut out = Vec::new();
    let mut rest = t;
    while !rest.is_empty() {
        if !rest.starts_with("[@") {
            return None;
        }
        let mut esc = false;
        let end = rest.char_indices().find_map(|(k, ch)| match ch {
            _ if esc => {
                esc = false;
                None
            }
            '\\' => {
                esc = true;
                None
            }
            ']' => Some(k),
            _ => None,
        })?;
        out.push(&rest[..=end]);
        rest = &rest[end + 1..];
    }
    debug_assert!(out.iter().all(|g| g.starts_with("[@") && g.ends_with(']')));
    Some(out)
}

/// NLP label text → Schemify template (§3.5 import), `None` when dropped.
pub(super) fn nlp_in(text: &str, label_set: &str) -> Option<String> {
    let t = text.trim();
    if let Some(n) = t
        .strip_prefix("cdsParam(")
        .and_then(|x| x.strip_suffix(')'))
    {
        let k: usize = n.trim().parse().ok()?;
        let p = label_set
            .split_whitespace()
            .filter(|w| !w.starts_with('-'))
            .nth(k.checked_sub(1)?)?;
        return Some(format!("{p}={{{p}}}"));
    }
    if t == "cdsName()" {
        return Some("{name}".into());
    }
    // Several groups (`[@w:%/][@l]`, what export writes for a stext with several holes):
    // each translated, concatenated.
    let groups = nlp_groups(t)?;
    if groups.len() > 1 {
        return groups.iter().map(|g| nlp_in(g, label_set)).collect();
    }
    let inner = t.strip_prefix("[@")?.strip_suffix(']')?;
    // Split on unescaped ':', keeping which chars were escaped: an escaped `%` is text, only
    // a bare one stands for the value.
    let mut parts: Vec<Vec<(char, bool)>> = vec![Vec::new()];
    let mut esc = false;
    for ch in inner.chars() {
        if esc {
            parts.last_mut().expect("one").push((ch, true));
            esc = false;
        } else if ch == '\\' {
            esc = true;
        } else if ch == ':' {
            parts.push(Vec::new());
        } else {
            parts.last_mut().expect("one").push((ch, false));
        }
    }
    let text = |p: &[(char, bool)]| -> String { p.iter().map(|c| c.0).collect() };
    let name = text(&parts[0]);
    let p = name.as_str();
    let hole = match p {
        "instanceName" => "name",
        "partName" | "cellName" => "cell",
        "refDes" => return None,
        x if x.starts_with("userPartName") || x.starts_with("phyPartName") => return None,
        x => x,
    };
    match parts.len() {
        1 => Some(format!("{{{hole}}}")),
        2 | 3 => {
            // pre%post; the fallback (third part) is lost.
            let fmt = &parts[1];
            let (pre, post) = match fmt.iter().position(|&c| c == ('%', false)) {
                Some(k) => (text(&fmt[..k]), text(&fmt[k + 1..])),
                None => (text(fmt), String::new()),
            };
            Some(format!(
                "{}{{{hole}}}{}",
                ael::literal_to_template(&pre),
                ael::literal_to_template(&post)
            ))
        }
        _ => None,
    }
}
