//! Label values and placement on import: `schemifyNet` values (spec `interop.md` §3.3),
//! NLP label text → Schemify templates and text sizes (§3.5), and pin labels as Virtuoso
//! drew them (§6.2, §7.5). The label objects themselves are read by `build` and
//! `schematic_in`.

use super::*;

/// The `schemifyNet` property value export writes when Virtuoso's spelling of a net does not
/// read back as it (§3.3): the Schemify net text, ` global` appended for a global label.
pub(crate) fn net_prop(net: &str, global: bool) -> String {
    if global {
        format!("{net} global")
    } else {
        net.to_owned()
    }
}

/// The net text and globality of a `schemifyNet` value (the inverse of [`net_prop`]).
pub(super) fn net_of_prop(v: &str) -> (String, bool) {
    match v.strip_suffix(" global") {
        Some(n) => (n.to_owned(), true),
        None => (v.to_owned(), false),
    }
}

/// The `vflags <overbar> <drafted> <visible>` values (0/1) of label `l`, `None` at
/// Virtuoso's default `0 1 1` (§7.5; `write::with_flags` reads them).
pub(super) fn flags_of(l: &Label) -> Option<[&'static str; 3]> {
    let b = |x: bool| if x { "1" } else { "0" };
    (l.overbar || l.undrafted || l.hidden).then(|| [b(l.overbar), b(!l.undrafted), b(!l.hidden)])
}

/// Text height (1/`upi` inch) → Schemify `size` milli (§3.5).
pub(super) fn size_milli(height: i32, upi: i32) -> i32 {
    profile::size_of_height(profile::height_from_virtuoso(height, upi))
}

/// The `[@…]` groups of NLP text that is nothing but groups (`\` escapes inside); `None`
/// when text sits outside a group or a group is unclosed.
fn nlp_groups(t: &str) -> Option<Vec<&str>> {
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

/// The fallbacks (`[@w:w=%:none]`'s `none`: what Virtuoso shows while the property has no
/// value) of an NLP label's groups; the template keeps none of them.
pub(super) fn nlp_fallbacks(text: &str) -> Vec<String> {
    nlp_groups(text.trim())
        .unwrap_or_default()
        .iter()
        .filter_map(|g| {
            let inner = g.strip_prefix("[@")?.strip_suffix(']')?;
            let mut esc = false;
            let mut parts = vec![String::new()];
            for ch in inner.chars() {
                match ch {
                    _ if esc => {
                        parts.last_mut()?.push(ch);
                        esc = false;
                    }
                    '\\' => esc = true,
                    ':' => parts.push(String::new()),
                    _ => parts.last_mut()?.push(ch),
                }
            }
            (parts.len() == 3).then(|| parts[2].clone())
        })
        .collect()
}

/// NLP label text → Schemify template (§3.5 import), `None` when dropped.
pub(super) fn nlp_in(text: &str, label_set: &str) -> Option<String> {
    let t = text.trim();
    if let Some(n) = t
        .strip_prefix("cdsParam(")
        .and_then(|x| x.strip_suffix(')'))
    {
        // The k-th word of paramLabelSet; a `-` word shows its value only (cdfuser ch. 5,
        // "Show Value Only"), and still counts.
        let k: usize = n.trim().parse().ok()?;
        let w = label_set.split_whitespace().nth(k.checked_sub(1)?)?;
        return Some(match w.strip_prefix('-') {
            Some(p) if !p.is_empty() => format!("{{{p}}}"),
            Some(_) => return None,
            None => format!("{w}={{{w}}}"),
        });
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

/// The point of polyline `pts` nearest `p`: the projection on a horizontal or vertical
/// segment, else the nearest end (§6.2).
pub(crate) fn project(p: [i32; 2], pts: &[[i32; 2]]) -> [i32; 2] {
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

/// The `vplabel` values of symbol pin `name` at `at` (milli-units): `none` when its main
/// pin figure has no pin label, the label's offset from the pin and its orient, justify,
/// font and height (then its flags, when not `0 1 1`) when they differ from export's (at
/// the pin, R0, centerLeft, stick, half LABEL_HEIGHT, `0 1 1`), else `None`.
pub(super) fn pin_label_as_drawn(
    sv: &Cv,
    name: &str,
    at: [i32; 2],
    cell: &str,
    notes: &mut Notes,
) -> Option<Vec<String>> {
    let fig = main_pin_fig(sv, name)?;
    // The label glued to the pin's rectangle, else an unglued pin label of its terminal's
    // name (Cadence's sample library leaves them unglued; live run).
    let Some(l) = sv
        .labels
        .iter()
        .find(|l| l.parent == fig.r && l.layer == "pin")
        .or_else(|| {
            sv.labels
                .iter()
                .find(|l| l.parent == 0 && l.layer == "pin" && l.text == fig.term)
        })
    else {
        return Some(vec!["none".into()]);
    };
    let xy = pt(l.xy, sv.upi, "pin label", cell, notes);
    let d = [xy[0] - at[0], xy[1] - at[1]];
    let h = profile::height_from_virtuoso(l.height, sv.upi);
    let o = orient_in(&l.orient);
    let usual = i64::from(crate::virtuoso::write::LABEL_HEIGHT / 2);
    let flags = flags_of(l);
    if d == [0, 0]
        && o == Orient::R0
        && (l.justify.as_str(), l.font.as_str(), h) == ("centerLeft", "stick", usual)
        && flags.is_none()
    {
        return None;
    }
    let mut v = vec![
        coord_text(d[0]),
        coord_text(d[1]),
        o.as_str().into(),
        l.justify.clone(),
        l.font.clone(),
        h.to_string(),
    ];
    v.extend(flags.into_iter().flatten().map(str::to_owned));
    Some(v)
}
