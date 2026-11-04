//! Maestro's disabled outputs ⇄ `output … enable false` (spec `interop.md` §9.11, lane L11).
//!
//! Maestro disables an output by clearing its Plot check box (live ICADVM20.1: an output
//! added with `?plot nil` gets no value in the `.rdb` and is `visible 0` there). The flag
//! lives only in the test's state, as the `plot` field of its `sevOutputStruct`
//! (`<field Name="plot" Type="list">nil</field>`), which `maestate` carries: the import reads
//! it there. The export sends `outenable <name> 0|1` where the bench differs from the
//! stored state, and the loader sets the output's `plot` (`maeGetTestOutputs` answers the
//! session's own structs). Not through the state: `axlImportSetup … 'overwrite` gives the
//! tests their states from elsewhere than the view's `active.state` (live ICADVM20.1, probe
//! l11: a Plot set there came back off; G40 holds for what the session reads at open only).

use super::*;

/// The value text of field `key` on line `t` (`<field Name="key" Type="…">value</field>`).
fn field<'t>(t: &'t str, key: &str) -> Option<&'t str> {
    let rest = t.strip_prefix(&format!("<field Name=\"{key}\" Type=\""))?;
    rest.split_once("\">")?.1.strip_suffix("</field>")
}

/// An output struct being read: (indent of its line, name, (plot field's line, plot on)).
type Open = (usize, Option<String>, Option<(usize, bool)>);

/// Each output struct of state element `el`: `(name, line of its plot field, plot on)`.
fn plots(el: &str) -> Vec<(String, usize, bool)> {
    let mut out = Vec::new();
    let mut cur: Option<Open> = None;
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

/// Export side, after `with_stored`: in `records` (one view's tests, each from its
/// `maestro` record), one `outenable <name> 0|1` per output whose enable in bench `c`
/// differs from its Plot in the test's stored state (`maestate`), or that is disabled and
/// has no stored state; none for the others.
pub(super) fn out_flags(c: &Cell, records: &mut Vec<Record>) {
    let states: Vec<(String, Vec<(String, bool)>)> = records
        .iter()
        .filter(|r| r.tag == "maestate")
        .filter_map(|r| {
            let el = r.text(1).ok()?;
            let plots = plots(&el).into_iter().map(|p| (p.0, p.2)).collect();
            Some((r.str(0).ok()?.to_owned(), plots))
        })
        .collect();
    let mut out: Vec<Record> = Vec::with_capacity(records.len());
    let mut flags: Vec<Record> = Vec::new();
    let mut test = String::new();
    for r in std::mem::take(records) {
        match r.tag.as_str() {
            "outenable" => continue,
            "maestro" => {
                out.append(&mut flags);
                test = r.str(0).unwrap_or("").to_owned();
            }
            "output" => {
                let n = r.str(0).unwrap_or("").to_owned();
                let on = c
                    .records(RecordKind::Output)
                    .find(|&k| c.field(k, 0) == n)
                    .is_none_or(|k| c.attr(k, "enable") != Some("false"));
                let was = states
                    .iter()
                    .find(|s| s.0 == test)
                    .and_then(|s| s.1.iter().find(|p| p.0 == n))
                    .map(|p| p.1);
                if was.map_or(!on, |w| w != on) {
                    flags.push(Record::new("outenable", vec![s(&n), i(i32::from(on))]));
                }
            }
            _ => {}
        }
        out.push(r);
    }
    out.append(&mut flags);
    *records = out;
}

#[cfg(test)]
mod tests {
    use super::*;

    const EL: &str = "\t<Test Name=\"T1\">\n\t\t<field Name=\"outputList\" Type=\"skillList\">\n\t\t\t<field Name=\"outputList_0\" Type=\"defstruct\">sevOutputStruct\n\t\t\t\t<field Name=\"name\" Type=\"string\">\"vmax\"</field>\n\t\t\t\t<field Name=\"waveSpec\" Type=\"skillDpl\">\n\t\t\t\t\t<field Name=\"plot\" Type=\"list\">nil</field>\n\t\t\t\t</field>\n\t\t\t\t<field Name=\"plot\" Type=\"symbol\">t</field>\n\t\t\t</field>\n\t\t\t<field Name=\"outputList_1\" Type=\"defstruct\">sevOutputStruct\n\t\t\t\t<field Name=\"name\" Type=\"string\">\"dis\"</field>\n\t\t\t\t<field Name=\"plot\" Type=\"list\">nil</field>\n\t\t\t</field>\n\t\t</field>\n\t</Test>\n";

    #[test]
    fn plot_flags_read_by_output_only() {
        // The nested `waveSpec` plot field is not the output's.
        assert_eq!(plot_off(EL), ["dis"]);
        assert_eq!(
            plot_off(&EL.replace(
                "list\">nil</field>\n\t\t\t</field>\n\t\t</field>",
                "symbol\">t</field>\n\t\t\t</field>\n\t\t</field>"
            )),
            Vec::<String>::new()
        );
    }
}
