//! Maestro's disabled outputs ⇄ `output … enable false` (spec `interop.md` §9.11, lane L11).
//!
//! Maestro disables an output by clearing its Plot check box (live ICADVM20.1: an output
//! added with `?plot nil` gets no value in the `.rdb` and is `visible 0` there). The flag
//! lives only in the test's state, as the `plot` field of its `sevOutputStruct`
//! (`<field Name="plot" Type="list">nil</field>`), which `maestate` carries: the import reads
//! it there, and the export writes a changed flag back into the stored state, which the
//! loader restores before the session opens (G40). An output the stored state does not have
//! goes as `outenable <name> 0`, which the loader passes to `maeAddOutput ?plot`.

use super::*;

/// The value text of field `key` on line `t` (`<field Name="key" Type="…">value</field>`).
fn field<'t>(t: &'t str, key: &str) -> Option<&'t str> {
    let rest = t.strip_prefix(&format!("<field Name=\"{key}\" Type=\""))?;
    rest.split_once("\">")?.1.strip_suffix("</field>")
}

/// Each output struct of state element `el`: `(name, line of its plot field, plot on)`.
fn plots(el: &str) -> Vec<(String, usize, bool)> {
    let mut out = Vec::new();
    // (indent of the struct line, name, (plot line, on)).
    let mut cur: Option<(usize, Option<String>, Option<(usize, bool)>)> = None;
    for (k, l) in el.split_inclusive('\n').enumerate() {
        let t = l.trim();
        let indent = l.len() - l.trim_start().len();
        match &mut cur {
            Some((ind, name, plot)) if t == "</field>" && indent == *ind => {
                if let (Some(n), Some((at, on))) = (name.take(), plot.take()) {
                    out.push((n, at, on));
                }
                cur = None;
            }
            Some((ind, name, plot)) if indent == *ind + 1 => {
                if let Some(v) = field(t, "name") {
                    *name = v
                        .strip_prefix('"')
                        .and_then(|v| v.strip_suffix('"'))
                        .map(str::to_owned);
                } else if let Some(v) = field(t, "plot") {
                    *plot = Some((k, v != "nil"));
                }
            }
            Some(_) => {}
            None if t.starts_with("<field Name=\"outputList_")
                && t.ends_with(">sevOutputStruct") =>
            {
                cur = Some((indent, None, None));
            }
            None => {}
        }
    }
    out
}

/// The outputs of state element `el` whose Plot is off.
pub(super) fn plot_off(el: &str) -> Vec<String> {
    plots(el)
        .into_iter()
        .filter(|p| !p.2)
        .map(|p| p.0)
        .collect()
}

/// `el` with the plot field of each output in `set` (`(name, on)`) set, where it differs.
fn set_plots(el: &str, set: &[(&str, bool)]) -> Option<String> {
    let at: Vec<(usize, bool)> = plots(el)
        .into_iter()
        .filter_map(|(n, k, on)| {
            let want = set.iter().find(|s| s.0 == n)?.1;
            (want != on).then_some((k, want))
        })
        .collect();
    if at.is_empty() {
        return None;
    }
    let mut out = String::with_capacity(el.len());
    for (k, l) in el.split_inclusive('\n').enumerate() {
        match at.iter().find(|x| x.0 == k) {
            Some(&(_, on)) => {
                let indent = &l[..l.len() - l.trim_start().len()];
                let v = if on {
                    "Type=\"symbol\">t"
                } else {
                    "Type=\"list\">nil"
                };
                out.push_str(&format!("{indent}<field Name=\"plot\" {v}</field>\n"));
            }
            None => out.push_str(l),
        }
    }
    Some(out)
}

/// Export side, after `with_stored`: each stored `maestate` of `records` (one view's tests,
/// each from its `maestro` record) with the Plot of every output the test sends set to
/// bench `c`'s enable of that output.
pub(super) fn patch_states(c: &Cell, records: &mut [Record]) {
    let mut want: Vec<(String, String, bool)> = Vec::new();
    let mut test = String::new();
    for r in records.iter() {
        match r.tag.as_str() {
            "maestro" => test = r.str(0).unwrap_or("").to_owned(),
            "output" => {
                let Ok(n) = r.str(0) else { continue };
                let on = c
                    .records(RecordKind::Output)
                    .find(|&k| c.field(k, 0) == n)
                    .is_none_or(|k| c.attr(k, "enable") != Some("false"));
                want.push((test.clone(), n.to_owned(), on));
            }
            _ => {}
        }
    }
    for r in records.iter_mut().filter(|r| r.tag == "maestate") {
        let (Ok(t), Ok(el)) = (r.str(0).map(str::to_owned), r.text(1)) else {
            continue;
        };
        let set: Vec<(&str, bool)> = want
            .iter()
            .filter(|w| w.0 == t)
            .map(|w| (w.1.as_str(), w.2))
            .collect();
        if let Some(el) = set_plots(&el, &set) {
            r.fields[1] = text(&el);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EL: &str = "\t<Test Name=\"T1\">\n\t\t<field Name=\"outputList\" Type=\"skillList\">\n\t\t\t<field Name=\"outputList_0\" Type=\"defstruct\">sevOutputStruct\n\t\t\t\t<field Name=\"name\" Type=\"string\">\"vmax\"</field>\n\t\t\t\t<field Name=\"waveSpec\" Type=\"skillDpl\">\n\t\t\t\t\t<field Name=\"plot\" Type=\"list\">nil</field>\n\t\t\t\t</field>\n\t\t\t\t<field Name=\"plot\" Type=\"symbol\">t</field>\n\t\t\t</field>\n\t\t\t<field Name=\"outputList_1\" Type=\"defstruct\">sevOutputStruct\n\t\t\t\t<field Name=\"name\" Type=\"string\">\"dis\"</field>\n\t\t\t\t<field Name=\"plot\" Type=\"list\">nil</field>\n\t\t\t</field>\n\t\t</field>\n\t</Test>\n";

    #[test]
    fn plot_flags_read_and_set_by_output_only() {
        assert_eq!(plot_off(EL), ["dis"]);
        let on = set_plots(EL, &[("dis", true), ("vmax", false)]).unwrap();
        assert_eq!(plot_off(&on), ["vmax"]);
        // Only the two struct-level lines changed; the nested field kept its text.
        let changed = EL.lines().zip(on.lines()).filter(|(a, b)| a != b).count();
        assert_eq!((changed, EL.lines().count()), (2, on.lines().count()));
        assert_eq!(set_plots(&on, &[("dis", true)]), None);
        assert_eq!(
            set_plots(
                &set_plots(&on, &[("dis", false), ("vmax", true)]).unwrap(),
                &[]
            ),
            None
        );
        assert_eq!(
            set_plots(&on, &[("dis", false), ("vmax", true)]).unwrap(),
            EL
        );
    }
}
