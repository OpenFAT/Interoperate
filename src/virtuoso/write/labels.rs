//! Labels on export (spec `interop.md` §3.5): justification, height and font of every
//! label the exporter writes, symbol `stext` → NLP labels, and Schemify templates → NLP
//! text.

use super::*;

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
        ..Label::default()
    }
}

/// `l` with the flags of `vflags <overbar> <drafted> <visible>` (0/1, §7.5); without them,
/// export's `0 0 1`.
pub(super) fn with_flags(mut l: Label, vflags: &[&str]) -> Label {
    if let [o, d, v] = vflags {
        (l.overbar, l.drafted, l.hidden) = (*o == "1", *d == "1", *v == "0");
    }
    l
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
    let vflags = c.attr_values(r, "vflags");
    if let [layer, purpose, justify, font, kind, text, tmpl] = v[..]
        && tmpl == t
    {
        let style = [layer, purpose, justify, font, kind];
        let h = height(c, r, STEXT_SIZE_MILLI);
        let l = label(ids.take(), 0, text.into(), at, o, h, style);
        return Some(with_flags(l, &vflags));
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
    let (hc, vc) = (
        c.doc.find_attr(r, "hcenter").is_some(),
        c.doc.find_attr(r, "vcenter").is_some(),
    );
    let mut style = [layer, purpose, justify(hc, vc), "stick", kind];
    // After a template edit the text and kind follow the new template; the rest keeps how
    // Virtuoso drew it (§7.5), its justification while the centring still agrees.
    if let [l, p, j, f, ..] = v[..] {
        (style[0], style[1], style[3]) = (l, p, f);
        if (j.ends_with("Center"), j.starts_with("center")) == (hc, vc) {
            style[2] = j;
        }
    }
    let h = height(c, r, STEXT_SIZE_MILLI);
    let l = label(ids.take(), 0, text, at, o, h, style);
    Some(with_flags(l, &vflags))
}

/// The NLP text of a symbol label template with holes (§3.5): every hole a parameter of
/// `params`, bare or through the display formatter `eng(p)` (NLP shows the value as
/// entered), one `[@p:pre%post]` group per hole (`[@p]` when both are empty), the literal
/// before the first hole its `pre`, each literal after a hole its `post`, NLP-escaped.
/// `None` when a hole is anything else. Import's `read::nlp_in` reads it back.
pub(crate) fn template_to_nlp(items: &[(bool, String)], params: &[&str]) -> Option<String> {
    fn param<'a>(h: &'a str, params: &[&str]) -> Option<&'a str> {
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
