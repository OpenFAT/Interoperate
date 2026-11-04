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

/// The `vplabel` values of symbol pin `name` at `at` (milli-units): `none` when its main
/// pin figure has no pin label, the label's offset from the pin and its orient, justify,
/// font and height when they differ from export's (at the pin, R0, centerLeft, stick, half
/// LABEL_HEIGHT), else `None`.
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
    if d == [0, 0]
        && o == Orient::R0
        && (l.justify.as_str(), l.font.as_str(), h) == ("centerLeft", "stick", usual)
    {
        return None;
    }
    Some(vec![
        coord_text(d[0]),
        coord_text(d[1]),
        o.as_str().into(),
        l.justify.clone(),
        l.font.clone(),
        h.to_string(),
    ])
}
