//! ADE analysis forms ⇄ Spectre statements (spec `interop.md` §9.2): `forms.tsv` holds, per
//! analysis kind, the form fields ADE answers untouched (ICADVM20.1's `.cdsenv`) and the
//! parameters Spectre's statement takes (`spectre -h <kind>`, SPECTRE21.10); [`to_spectre`]
//! writes an analysis as the statement ADE's netlister makes of it (live netlists, gaps
//! round 2: `live/checks/a_probe.il`), [`from_spectre`] reads one back as the fields that
//! netlist so.

use std::sync::OnceLock;

use super::*;

/// The table: `forms.tsv`, generated on the Virtuoso host (`live/checks/a_forms.py`).
const FORMS: &str = include_str!("forms.tsv");

/// `(defaults (kind, field, value), params (kind, names))` of [`FORMS`].
type Table = (
    Vec<(&'static str, &'static str, &'static str)>,
    Vec<(&'static str, Vec<&'static str>)>,
);

fn table() -> &'static Table {
    static T: OnceLock<Table> = OnceLock::new();
    T.get_or_init(|| {
        let (mut d, mut s) = (Vec::new(), Vec::new());
        for l in FORMS.lines().filter(|l| !l.starts_with('#')) {
            match l.split('\t').collect::<Vec<_>>()[..] {
                ["d", k, f, v] => d.push((k, f, v)),
                ["s", k, names] => s.push((k, names.split(' ').collect())),
                _ => panic!("forms.tsv: malformed row `{l}`"),
            }
        }
        (d, s)
    })
}

/// What ADE answers for form field `field` of analysis `kind` when nothing set it (kind
/// `options`: a simulator option, `maeGetSimOption`); `None` for a field without a default.
pub(super) fn form_default(kind: &str, field: &str) -> Option<&'static str> {
    let d = &table().0;
    d.iter().find(|r| r.0 == kind && r.1 == field).map(|r| r.2)
}

/// Analyses that compute the large-signal steady state the small-signal periodic ones
/// (`pac pnoise pxf pstb hbac hbnoise …`) read: netlisted before them.
pub(super) const STEADY: [&str; 4] = ["pss", "hb", "qpss", "envlp"];

/// Whether field `field` of analysis `kind` holds `v` as nothing set it: its form default,
/// or that default as the one-element list ICADVM20.1 answers for some fields (`("1")`,
/// live: pac `outmodharmvec`, hb `lsspharms`).
pub(super) fn is_form_default(kind: &str, field: &str, v: &str) -> bool {
    let d = form_default(kind, field);
    d == Some(v)
        || v.strip_prefix("(\"")
            .and_then(|x| x.strip_suffix("\")"))
            .is_some_and(|x| !x.contains('"') && d == Some(x))
}

/// Whether `name` is a parameter of Spectre's `kind` statement (`options` for simulator
/// options).
pub(super) fn is_spectre_param(kind: &str, name: &str) -> bool {
    table()
        .1
        .iter()
        .any(|(k, names)| *k == kind && names.contains(&name))
}

/// An ADE path (`/IPRB0`, `/I0/out`, `/gnd!`) as a Spectre name: `/` dropped, hierarchy
/// joined by `.`, ground `0`, other characters outside `[A-Za-z0-9_.]` escaped with `\`
/// (netlist.md §7.6.11).
fn path_to_spectre(v: &str) -> String {
    let p = v.strip_prefix('/').unwrap_or(v);
    if p == "gnd!" {
        return "0".into();
    }
    let mut out = String::with_capacity(p.len());
    for ch in p.chars() {
        match ch {
            '/' => out.push('.'),
            c if c.is_ascii_alphanumeric() || c == '_' => out.push(c),
            c => {
                out.push('\\');
                out.push(c);
            }
        }
    }
    out
}

/// The inverse of [`path_to_spectre`]; a value already in ADE's form (`/x`) is kept.
fn path_from_spectre(v: &str) -> String {
    if v.starts_with('/') {
        return v.to_owned();
    }
    if v == "0" {
        return "/gnd!".into();
    }
    let mut out = String::from("/");
    let mut esc = false;
    for ch in v.chars() {
        match ch {
            _ if esc => {
                out.push(ch);
                esc = false;
            }
            '\\' => esc = true,
            '.' => out.push('/'),
            c => out.push(c),
        }
    }
    out
}

/// Fields holding an instance path, written by [`path_to_spectre`].
const PATHS: [&str; 4] = ["dev", "probe", "oprobe", "iprobe"];

/// Analysis `kind` with form fields `opts` (form defaults already dropped, §9.2) as the
/// Spectre statement ADE netlists, `<kind> [(<p> <n>)] <kind> k=v …`, and the notes of the
/// fields it leaves out. The rules are the netlister's, read off its live netlists: output
/// nodes `p`/`n` when `outType` is `voltage` (else the probe), the sweep variable
/// (`Temperature` → `param=temp`, `Design Variable` → `param=<var>`), the range
/// (`Center-Span` → `center`/`span`), the step from `incrType`/`stepTypeLin`/`stepTypeLog`
/// (`Automatic` netlists none), instance paths without `/`. Any other field is written when
/// it is a parameter of Spectre's statement, else left out with a note. `sens` is written as
/// ADE writes an untouched one, `sens ( ) for ( )`.
pub(super) fn to_spectre(kind: &str, opts: &[(String, String)]) -> (String, Vec<String>) {
    let set = |k: &str| opts.iter().find(|o| o.0 == k).map(|o| o.1.as_str());
    let get = |k: &str| set(k).or_else(|| form_default(kind, k));
    let mut notes = Vec::new();
    let mut used: Vec<&str> = vec!["anaName"];
    let mut params: Vec<(String, String)> = Vec::new();
    let mut nodes: Vec<String> = Vec::new();
    let drop = |k: &str, why: &str, notes: &mut Vec<String>| {
        if let Some(v) = set(k).filter(|v| !v.is_empty()) {
            notes.push(format!("{kind} {k}={v}: {why}"));
        }
    };
    if kind == "sens" {
        for (k, v) in opts.iter().filter(|o| o.0 != "anaName") {
            notes.push(format!("sens {k}={v}: no Spectre form written"));
        }
        return ("sens ( ) for ( )".into(), notes);
    }
    // The output: nodes, or the probe component.
    used.extend(["outType", "p", "n"]);
    match get("outType") {
        Some(o) if !o.eq_ignore_ascii_case("voltage") => {
            for k in ["p", "n"] {
                drop(
                    k,
                    &format!("outType {o} netlists no output nodes"),
                    &mut notes,
                );
            }
        }
        _ => {
            if let Some(p) = set("p").filter(|p| !p.is_empty()) {
                nodes.push(path_to_spectre(p));
                if let Some(n) = set("n").filter(|n| !n.is_empty()) {
                    nodes.push(path_to_spectre(n));
                }
                used.push("oprobe");
                drop("oprobe", "outType voltage netlists the nodes", &mut notes);
            }
        }
    }
    // The sweep variable.
    used.extend(["sweep", "designVar", "deviceParam", "modelParam"]);
    let param = set("param")
        .or_else(|| set("deviceParam"))
        .or_else(|| set("modelParam"));
    match get("sweep") {
        Some("Temperature") => params.push(kv("param", param.unwrap_or("temp"))),
        Some("Design Variable") => {
            if let Some(v) = set("designVar").or(param) {
                params.push(kv("param", v));
            }
        }
        _ => {
            if let Some(v) = param {
                params.push(kv("param", v));
            }
        }
    }
    used.push("param");
    // The range and the step.
    used.extend(["rangeType", "incrType", "stepTypeLin", "stepTypeLog"]);
    if get("rangeType") == Some("Center-Span") {
        used.extend(["start", "stop"]);
        drop("start", "rangeType Center-Span", &mut notes);
        drop("stop", "rangeType Center-Span", &mut notes);
    } else {
        used.extend(["center", "span"]);
        drop("center", "rangeType Start-Stop", &mut notes);
        drop("span", "rangeType Start-Stop", &mut notes);
    }
    let step = match (get("incrType"), get("stepTypeLin"), get("stepTypeLog")) {
        (Some("Linear"), Some("Number of Steps"), _) => Some("lin"),
        (Some("Linear"), _, _) => Some("step"),
        (Some("Logarithmic"), _, Some("Number of Steps")) => Some("log"),
        (Some("Logarithmic"), _, _) => Some("dec"),
        (None, ..) => None,
        _ => Some(""),
    };
    if let Some(keep) = step {
        for k in ["step", "lin", "dec", "log"] {
            if k != keep {
                used.push(k);
                let why = format!(
                    "incrType {} netlists {}",
                    get("incrType").unwrap_or(""),
                    if keep.is_empty() { "no step" } else { keep }
                );
                drop(k, &why, &mut notes);
            }
        }
    }
    // Choosers of the probe and input forms: their meaning is in the fields they enable.
    for k in ["probeType", "stbProbeMode", "inType"] {
        used.push(k);
        if let (Some(v), Some(d)) = (set(k), form_default(kind, k))
            && v != d
        {
            notes.push(format!("{kind} {k}={v}: only {d} is written"));
        }
    }
    for (k, v) in opts {
        if used.contains(&k.as_str()) || v.is_empty() {
            continue;
        }
        if is_spectre_param(kind, k) {
            let v = if PATHS.contains(&k.as_str()) {
                path_to_spectre(v)
            } else {
                v.clone()
            };
            params.push((k.clone(), v));
        } else {
            notes.push(format!(
                "{kind} {k}={v}: not a parameter of Spectre's {kind}"
            ));
        }
    }
    let mut line = kind.to_owned();
    if !nodes.is_empty() {
        line.push_str(&format!(" ({})", nodes.join(" ")));
    }
    line.push(' ');
    line.push_str(kind);
    for (k, v) in &params {
        line.push_str(&format!(" {k}={}", spectre_word(v)));
    }
    (line, notes)
}

/// The form fields of Spectre statement `<kind> [(nodes)] <kind> params` (the inverse of
/// [`to_spectre`] on its own output): nodes → `outType voltage` with `p`/`n`; `dev` →
/// `sweep "Component Parameter"`, `mod` → `Model Parameter`, `param=temp` alone →
/// `Temperature`, another `param` alone → `Design Variable`; `center`/`span` → `rangeType
/// Center-Span`; `step lin dec log` → their `incrType` and step type; `oprobe` → `outType
/// probe`; instance paths with ADE's `/`.
pub(super) fn from_spectre(
    kind: &str,
    nodes: &[String],
    params: &[(String, String)],
) -> Vec<(String, String)> {
    let has = |k: &str| params.iter().any(|p| p.0 == k);
    let mut out = Vec::new();
    if let Some(p) = nodes.first() {
        out.push(kv("outType", "voltage"));
        out.push(kv("p", &path_from_spectre(p)));
        if let Some(n) = nodes.get(1) {
            out.push(kv("n", &path_from_spectre(n)));
        }
    }
    if has("oprobe") && nodes.is_empty() {
        out.push(kv("outType", "probe"));
    }
    let param = params.iter().find(|p| p.0 == "param").map(|p| p.1.as_str());
    if has("dev") {
        out.push(kv("sweep", "Component Parameter"));
    } else if has("mod") {
        out.push(kv("sweep", "Model Parameter"));
    } else if param == Some("temp") {
        out.push(kv("sweep", "Temperature"));
    } else if let Some(v) = param {
        out.push(kv("sweep", "Design Variable"));
        out.push(kv("designVar", v));
    }
    if has("center") || has("span") {
        out.push(kv("rangeType", "Center-Span"));
    }
    for (k, v) in params {
        match k.as_str() {
            "step" => out.extend([kv("incrType", "Linear"), kv("stepTypeLin", "Step Size")]),
            "lin" => out.extend([
                kv("incrType", "Linear"),
                kv("stepTypeLin", "Number of Steps"),
            ]),
            "dec" => out.extend([
                kv("incrType", "Logarithmic"),
                kv("stepTypeLog", "Points Per Decade"),
            ]),
            "log" => out.extend([
                kv("incrType", "Logarithmic"),
                kv("stepTypeLog", "Number of Steps"),
            ]),
            _ => {}
        }
        let v = if PATHS.contains(&k.as_str()) {
            path_from_spectre(v)
        } else {
            v.clone()
        };
        out.push((k.clone(), v));
    }
    let _ = kind;
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(o: &[(&str, &str)]) -> Vec<(String, String)> {
        o.iter().map(|(k, v)| kv(k, v)).collect()
    }

    #[test]
    fn the_table_holds_the_measured_defaults() {
        assert_eq!(form_default("noise", "outType"), Some("probe"));
        assert_eq!(form_default("dc", "incrType"), Some("Automatic"));
        assert_eq!(form_default("tran", "finalTimeOp"), Some("t"));
        assert_eq!(form_default("options", "reltol"), Some("1e-3"));
        assert!(is_spectre_param("stb", "probe") && !is_spectre_param("stb", "probeType"));
        assert!(is_spectre_param("options", "gmin"));
    }

    /// ADE's own netlists (live, `a_probe`): the fields maeGetAnalysis answered, form defaults
    /// dropped, against the statement ADE wrote.
    #[test]
    fn statements_are_adess_netlists() {
        type Case<'a> = (&'a str, &'a [(&'a str, &'a str)], &'a str);
        let cases: &[Case] = &[
            (
                "noise",
                &[
                    ("start", "1"),
                    ("stop", "1G"),
                    ("incrType", "Logarithmic"),
                    ("dec", "10"),
                    ("outType", "voltage"),
                    ("p", "/out"),
                    ("n", "/gnd!"),
                    ("iprobe", "/V0"),
                ],
                "noise (out 0) noise start=1 stop=1G dec=10 iprobe=V0",
            ),
            (
                "noise",
                &[
                    ("start", "1"),
                    ("stop", "1G"),
                    ("incrType", "Logarithmic"),
                    ("dec", "10"),
                    ("oprobe", "/R0"),
                    ("iprobe", "/V0"),
                ],
                "noise noise start=1 stop=1G dec=10 oprobe=R0 iprobe=V0",
            ),
            (
                "noise",
                &[
                    ("rangeType", "Center-Span"),
                    ("center", "1k"),
                    ("span", "100"),
                    ("incrType", "Linear"),
                    ("step", "10"),
                    ("outType", "voltage"),
                    ("p", "/out"),
                    ("n", "/gnd!"),
                ],
                "noise (out 0) noise center=1k span=100 step=10",
            ),
            (
                "stb",
                &[
                    ("start", "1"),
                    ("stop", "1G"),
                    ("incrType", "Logarithmic"),
                    ("dec", "10"),
                    ("probe", "/IPRB0"),
                ],
                "stb stb start=1 stop=1G dec=10 probe=IPRB0",
            ),
            (
                "dc",
                &[
                    ("sweep", "Design Variable"),
                    ("designVar", "C"),
                    ("param", "C"),
                    ("start", "1n"),
                    ("stop", "2n"),
                    ("incrType", "Linear"),
                    ("stepTypeLin", "Number of Steps"),
                    ("lin", "2"),
                ],
                "dc dc param=C start=1n stop=2n lin=2",
            ),
            (
                "ac",
                &[
                    ("rangeType", "Center-Span"),
                    ("center", "1k"),
                    ("span", "100"),
                    ("incrType", "Logarithmic"),
                    ("stepTypeLog", "Number of Steps"),
                    ("log", "20"),
                ],
                "ac ac center=1k span=100 log=20",
            ),
            ("pz", &[("p", "/out"), ("n", "/gnd!")], "pz (out 0) pz"),
            (
                "pss",
                &[("fund", "1M"), ("harms", "3"), ("errpreset", "moderate")],
                "pss pss fund=1M harms=3 errpreset=moderate",
            ),
            ("sens", &[], "sens ( ) for ( )"),
        ];
        for (kind, o, want) in cases {
            let (line, notes) = to_spectre(kind, &opts(o));
            assert_eq!(&line, want, "{kind} {o:?} ({notes:?})");
        }
        // ADE netlists no step for an Automatic sweep, and drops the voltage input fields.
        let (line, notes) = to_spectre(
            "ac",
            &opts(&[("start", "1"), ("stop", "1G"), ("dec", "10")]),
        );
        assert_eq!(line, "ac ac start=1 stop=1G");
        assert_eq!(notes, ["ac dec=10: incrType Automatic netlists no step"]);
        let (_, notes) = to_spectre("noise", &opts(&[("inType", "voltage"), ("inVsrc", "/V0")]));
        assert_eq!(notes.len(), 2, "{notes:?}");
    }

    #[test]
    fn from_spectre_reads_to_spectre_back() {
        for (kind, o) in [
            (
                "noise",
                opts(&[
                    ("outType", "voltage"),
                    ("p", "/out"),
                    ("n", "/gnd!"),
                    ("start", "1"),
                    ("stop", "1G"),
                    ("incrType", "Logarithmic"),
                    ("stepTypeLog", "Points Per Decade"),
                    ("dec", "10"),
                    ("iprobe", "/X1/V0"),
                ]),
            ),
            (
                "dc",
                opts(&[
                    ("sweep", "Temperature"),
                    ("param", "temp"),
                    ("start", "-40"),
                    ("stop", "125"),
                    ("incrType", "Linear"),
                    ("stepTypeLin", "Step Size"),
                    ("step", "5"),
                ]),
            ),
        ] {
            let (line, _) = to_spectre(kind, &o);
            let w = super::super::spectre_words(&line).unwrap();
            let (nodes, rest) = match w[1].strip_prefix('(') {
                Some(_) => {
                    let end = w.iter().position(|x| x.ends_with(')')).unwrap();
                    let n: Vec<String> = w[1..=end]
                        .join(" ")
                        .trim_matches(['(', ')'])
                        .split_whitespace()
                        .map(str::to_owned)
                        .collect();
                    (n, &w[end + 2..])
                }
                None => (Vec::new(), &w[2..]),
            };
            let params: Vec<(String, String)> = rest
                .iter()
                .map(|p| {
                    let (k, v) = p.split_once('=').unwrap();
                    kv(k, v)
                })
                .collect();
            let mut back = from_spectre(kind, &nodes, &params);
            let mut want = o.clone();
            back.sort();
            want.sort();
            assert_eq!(back, want, "{line}");
        }
    }
}
