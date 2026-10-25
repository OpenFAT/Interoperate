//! Schemify interface → Virtuoso CDF and simInfo (spec `interop.md` §6.3).

use super::*;

// ---------------------------------------------------------------------------------------
// CDF and simInfo (§6.3)
// ---------------------------------------------------------------------------------------

/// Classifies identifiers of cell `c`'s own defaults: its params (`iPar`), its vars
/// (inlined).
pub(super) fn own_scope(c: &Cell) -> impl FnMut(&str) -> Name + '_ {
    move |n: &str| {
        if c.records(RecordKind::Param).any(|r| c.field(r, 0) == n) {
            Name::OwnParam
        } else if let Some(r) = c.records(RecordKind::Var).find(|r| c.field(*r, 0) == n) {
            Name::CellVar(c.field(r, 1).to_owned())
        } else {
            Name::Unknown
        }
    }
}

pub(super) fn cdf_out(c: &Cell, core: bool, notes: &mut Notes) -> Cdf {
    let mut cdf = Cdf::default();
    let src = if core {
        devices::by_core(&c.name).and_then(|dv| dv.source)
    } else {
        None
    };
    for r in c.records(RecordKind::Param) {
        let name = c.field(r, 0);
        if let (Some(current), "value") = (src, name) {
            cdf.params.extend(source_params(c.field(r, 1), current));
            continue;
        }
        cdf.params.push(param_out(c, r, notes));
    }
    // Inherited pins: netSet params carry the instance overrides (§6.2).
    let pins = sym_pins(c);
    for k in 0..pins.name.len() {
        if let (Some(net), None) = (&pins.inherit[k], &pins.alias[k]) {
            cdf.params.push(CdfParam {
                name: vname(&pins.name[k]),
                ty: "netSet".into(),
                def: net_to_virtuoso(net, true).0,
                prompt: pins.name[k].clone(),
                flags: vec!["storeDefault".into()],
                ..CdfParam::default()
            });
        }
    }
    let order: Vec<String> = pins
        .name
        .iter()
        .zip(&pins.alias)
        .filter(|(_, a)| a.is_none())
        .map(|(n, _)| vname(n))
        .collect();
    let inst_params: Vec<String> = c
        .records(RecordKind::Param)
        .filter(|r| c.attr(*r, "computed").is_none())
        .map(|r| c.field(r, 0).to_owned())
        .collect();
    match c.kind() {
        FileKind::Shcomp => {
            cdf.siminfo.push(SimInfo {
                sim: "spectre".into(),
                fields: vec![
                    ("termOrder".into(), SimVal::List(order.clone())),
                    ("instParameters".into(), SimVal::List(inst_params.clone())),
                ],
            });
            cdf.siminfo.push(SimInfo {
                sim: "auCdl".into(),
                fields: vec![
                    (
                        "netlistProcedure".into(),
                        SimVal::Str("ansCdlSubcktCall".into()),
                    ),
                    ("termOrder".into(), SimVal::List(order)),
                    ("instParameters".into(), SimVal::List(inst_params)),
                    ("namePrefix".into(), SimVal::Str("X".into())),
                ],
            });
        }
        FileKind::Shprim if core => {
            if let Some(dv) = devices::by_core(&c.name).filter(|dv| !dv.spectre.is_empty()) {
                let mut f = Vec::new();
                if dv.spectre == "{model}" {
                    f.push(("otherParameters".into(), SimVal::List(vec!["model".into()])));
                } else {
                    f.push(("componentName".into(), SimVal::Str(dv.spectre.into())));
                }
                f.push(("termOrder".into(), SimVal::List(order)));
                let have: Vec<&str> = cdf.params.iter().map(|p| p.name.as_str()).collect();
                let used: Vec<&(&str, &str)> = dv
                    .spectre_params
                    .iter()
                    .filter(|(_, p)| have.contains(p))
                    .collect();
                f.push((
                    "instParameters".into(),
                    SimVal::List(used.iter().map(|(s, _)| (*s).into()).collect()),
                ));
                let pm: Vec<(String, String)> = used
                    .iter()
                    .filter(|(s, p)| s != p)
                    .map(|(s, p)| ((*s).into(), (*p).into()))
                    .collect();
                if !pm.is_empty() {
                    f.push(("propMapping".into(), SimVal::Pairs(pm)));
                }
                cdf.siminfo.push(SimInfo {
                    sim: "spectre".into(),
                    fields: f,
                });
            } else {
                notes.review(
                    "format",
                    &c.path,
                    "no spectre simInfo for this core cell (§6.5)",
                );
            }
        }
        FileKind::Shprim => {
            let impl_r = c.records(RecordKind::Impl).next();
            let fmt = |dialect: &str| {
                impl_r.and_then(|r| {
                    c.doc
                        .attrs(r)
                        .filter(|a| {
                            c.doc.attr_key(*a) == "format"
                                && c.doc.attr_value(*a, 0) == Some(dialect)
                        })
                        .find_map(|a| c.doc.attr_value(a, 1))
                })
            };
            let params: Vec<(String, String)> = c
                .records(RecordKind::Param)
                .map(|r| {
                    (
                        c.field(r, 0).to_owned(),
                        c.attr(r, "kind").unwrap_or("number").to_owned(),
                    )
                })
                .collect();
            let pin_names: Vec<String> = pins
                .name
                .iter()
                .zip(&pins.alias)
                .filter(|(_, a)| a.is_none())
                .map(|(n, _)| n.clone())
                .collect();
            match fmt("spectre").map(|t| spectre_siminfo(t, &pin_names, &params)) {
                Some(Some(si)) => cdf.siminfo.push(si),
                other => {
                    if other.is_some() {
                        notes.review(
                            "format",
                            &c.path,
                            "format spectre is not `{name} ({pins}) master k={p}…`",
                        );
                    }
                    cdf.siminfo.push(SimInfo {
                        sim: "spectre".into(),
                        fields: vec![("termOrder".into(), SimVal::List(order.clone()))],
                    });
                }
            }
            if let Some(t) = fmt("cdl") {
                match cdl_siminfo(t, &pin_names, &params) {
                    Some(si) => cdf.siminfo.push(si),
                    None => notes.review(
                        "format",
                        &c.path,
                        "format cdl does not match a CDL netlist procedure",
                    ),
                }
            }
        }
        _ => {}
    }
    assert!(
        cdf.params
            .iter()
            .all(|p| CDF_TYPES.contains(&p.ty.as_str()))
    );
    cdf
}

/// The CDF params of a core source's `value` (§6.6): `srcType` and every waveform param,
/// defaults from parsing the core default.
pub(super) fn source_params(default: &str, current: bool) -> Vec<CdfParam> {
    let parsed = devices::parse_source(default, current);
    let mut out = Vec::new();
    for n in devices::source_cdf_params(current) {
        if n == "srcType" {
            out.push(CdfParam {
                name: n.into(),
                ty: "cyclic".into(),
                def: devices::WAVEFORMS[parsed.as_ref().map_or(0, |p| p.wave)]
                    .1
                    .into(),
                prompt: n.into(),
                choices: devices::WAVEFORMS.iter().map(|w| w.1.to_owned()).collect(),
                ..CdfParam::default()
            });
            continue;
        }
        let def = parsed
            .as_ref()
            .and_then(|p| p.params.iter().find(|x| x.0 == n))
            .map_or(String::new(), |x| x.1.clone());
        out.push(CdfParam {
            name: n.into(),
            ty: if n == "tvpairs" {
                "int".into()
            } else {
                "string".into()
            },
            def: if n == "tvpairs" && def.is_empty() {
                "0".into()
            } else {
                def
            },
            prompt: n.into(),
            flags: if n == "tvpairs" {
                vec![]
            } else {
                vec!["parseAsNumber".into(), "parseAsCEL".into()]
            },
            ..CdfParam::default()
        });
    }
    out
}

/// One Schemify param as a CDF parameter (§6.3 table).
pub(super) fn param_out(c: &Cell, r: u32, notes: &mut Notes) -> CdfParam {
    let name = c.field(r, 0);
    let default = c.field(r, 1);
    let kind = c.attr(r, "kind").unwrap_or("number");
    let mut p = CdfParam {
        name: name.to_owned(),
        prompt: c.attr(r, "doc").unwrap_or(name).to_owned(),
        units: c
            .attr(r, "unit")
            .and_then(ael::unit_to_cdf)
            .unwrap_or("")
            .to_owned(),
        ..CdfParam::default()
    };
    let num = |e: &str, notes: &mut Notes| match ael::to_ael(e, own_scope(c)) {
        Ok(t) => {
            if let Some(why) = t.review {
                notes.review(why, &c.path, format!("param {name}: {e}"));
            }
            t.text
        }
        Err(why) => {
            notes.review(
                why,
                &c.path,
                format!("param {name}: `{e}` has no AEL form; CDF default left empty"),
            );
            String::new()
        }
    };
    if let Some(comp) = c.attr(r, "computed") {
        notes.review(
            "computed",
            &c.path,
            format!("param {name} is computed: exported as a CEL default"),
        );
        p.ty = "string".into();
        p.flags = vec!["parseAsNumber".into(), "parseAsCEL".into()];
        p.def = num(comp, notes);
        return p;
    }
    match kind {
        "int" => {
            p.ty = "int".into();
            p.def = if default.parse::<i64>().is_ok() {
                default.to_owned()
            } else {
                notes.review(
                    "expression",
                    &c.path,
                    format!("param {name}: int default `{default}` is not a literal"),
                );
                "0".into()
            };
        }
        "bool" => {
            p.ty = "boolean".into();
            p.def = ael::bool_to_cdf(default)
                .unwrap_or_else(|| {
                    notes.review(
                        "expression",
                        &c.path,
                        format!("param {name}: bool default `{default}` is not a literal"),
                    );
                    "nil"
                })
                .into();
        }
        "choice" => {
            p.ty = "cyclic".into();
            p.choices = c
                .attr_values(r, "choices")
                .into_iter()
                .map(str::to_owned)
                .collect();
            p.def = literal(default, name, c, notes);
        }
        "string" => {
            p.ty = "string".into();
            p.def = literal(default, name, c, notes);
            // Virtuoso refuses units on a string parameter not parsed as a number (live
            // ICADVM20.1: "'parseAsNumber' field must be set if 'units' field is set").
            p.units.clear();
        }
        _ => {
            p.ty = "string".into();
            p.flags = vec!["parseAsNumber".into(), "parseAsCEL".into()];
            p.def = num(default, notes);
        }
    }
    p
}

pub(super) fn literal(t: &str, what: &str, c: &Cell, notes: &mut Notes) -> String {
    ael::template_to_literal(t).unwrap_or_else(|| {
        notes.review(
            "template",
            &c.path,
            format!("{what}: template `{t}` sent as text"),
        );
        t.to_owned()
    })
}

/// The pattern a string param takes in an imported spectre format: ` k=<c>` only when set.
pub fn optional_param(k: &str, c: &str) -> String {
    format!("{{{c} != \"\" ? \" {k}=\" + {c} : \"\"}}")
}

/// Splits a template into literal runs and `{hole}`s (`{{`/`}}` kept as text). `None` on
/// unbalanced braces.
pub(crate) fn template_items(t: &str) -> Option<Vec<(bool, String)>> {
    let mut out: Vec<(bool, String)> = Vec::new();
    let mut lit = String::new();
    let b: Vec<char> = t.chars().collect();
    let mut k = 0;
    while k < b.len() {
        match b[k] {
            '{' if b.get(k + 1) == Some(&'{') => {
                lit.push('{');
                k += 2;
            }
            '}' if b.get(k + 1) == Some(&'}') => {
                lit.push('}');
                k += 2;
            }
            '{' => {
                if !lit.is_empty() {
                    out.push((false, std::mem::take(&mut lit)));
                }
                let mut depth = 0;
                let mut q = false;
                let start = k + 1;
                let mut end = None;
                for (j, &ch) in b.iter().enumerate().skip(start) {
                    match ch {
                        '"' => q = !q,
                        '{' if !q => depth += 1,
                        '}' if !q && depth == 0 => {
                            end = Some(j);
                            break;
                        }
                        '}' if !q => depth -= 1,
                        _ => {}
                    }
                }
                let end = end?;
                out.push((true, b[start..end].iter().collect()));
                k = end + 1;
            }
            '}' => return None,
            c => {
                lit.push(c);
                k += 1;
            }
        }
    }
    if !lit.is_empty() {
        out.push((false, lit));
    }
    Some(out)
}

/// The `k={c}` tail items of a format: `(k, c)` per item, each ` k={c}` or the optional
/// form [`optional_param`] writes, every `c` one of `params`; `None` when the tail has any
/// other shape.
pub(super) fn param_tail(
    items: &[(bool, String)],
    params: &[(String, String)],
) -> Option<Vec<(String, String)>> {
    let mut out = Vec::new();
    let mut k = 0;
    let is_param = |c: &str| params.iter().any(|p| p.0 == c);
    while k < items.len() {
        match &items[k] {
            (false, lit)
                if lit.starts_with(' ')
                    && lit.ends_with('=')
                    && lit.len() > 2
                    && !lit[1..lit.len() - 1].contains(char::is_whitespace) =>
            {
                let key = lit[1..lit.len() - 1].to_owned();
                let (true, c) = items.get(k + 1)? else {
                    return None;
                };
                if !is_param(c) {
                    return None;
                }
                out.push((key, c.clone()));
                k += 2;
            }
            (true, hole) => {
                // `{c != "" ? " k=" + c : ""}`
                let c = hole.split_once(" != \"\" ? \" ")?.0.to_owned();
                let key = hole
                    .split_once(" != \"\" ? \" ")?
                    .1
                    .split_once("=\" + ")?
                    .0
                    .to_owned();
                if !is_param(&c) || optional_param(&key, &c) != format!("{{{hole}}}") {
                    return None;
                }
                out.push((key, c));
                k += 1;
            }
            _ => return None,
        }
    }
    Some(out)
}

/// The pins of a template's pin list: `{pins}` → all pins, else each `{pin}` in order
/// (they must be a permutation of `pins`).
pub(super) fn pin_list(items: &[(bool, String)], pins: &[String]) -> Option<Vec<String>> {
    if let [(true, h)] = items
        && h == "pins"
    {
        return Some(pins.to_vec());
    }
    let mut out = Vec::new();
    for (k, it) in items.iter().enumerate() {
        match it {
            (true, h) if pins.contains(h) => out.push(h.clone()),
            (false, l) if l == " " && k % 2 == 1 => {}
            _ => return None,
        }
    }
    let mut a = out.clone();
    a.sort();
    let mut b = pins.to_vec();
    b.sort();
    (a == b).then_some(out)
}

/// Reverses the §6.3 spectre rule: `{name} ({pins…}) master k={c}…` → spectre simInfo.
pub fn spectre_siminfo(t: &str, pins: &[String], params: &[(String, String)]) -> Option<SimInfo> {
    let items = template_items(t)?;
    let [(true, n), (false, open), rest @ ..] = items.as_slice() else {
        return None;
    };
    if n != "name" || open != " (" {
        return None;
    }
    let close = rest.iter().position(|it| !it.0 && it.1.starts_with(") "))?;
    let order = pin_list(&rest[..close], pins)?;
    let after = &rest[close].1[2..];
    let mut tail: Vec<(bool, String)> = rest[close + 1..].to_vec();
    let mut f = Vec::new();
    if after.is_empty() {
        // master is a hole: only `{model}`
        let (true, h) = tail.first()? else {
            return None;
        };
        if h != "model" {
            return None;
        }
        tail.remove(0);
        f.push(("otherParameters".into(), SimVal::List(vec!["model".into()])));
    } else {
        // `master k=` … : the literal holds the master and maybe the first ` k=`.
        let (master, first) = match after.split_once(' ') {
            Some((m, k)) => (m, Some(format!(" {k}"))),
            None => (after, None),
        };
        if master.is_empty() || master.contains(['{', '}']) {
            return None;
        }
        if let Some(k) = first {
            tail.insert(0, (false, k));
        }
        f.push(("componentName".into(), SimVal::Str(master.into())));
    }
    let kv = param_tail(&tail, params)?;
    f.push((
        "termOrder".into(),
        SimVal::List(order.iter().map(|p| vname(p)).collect()),
    ));
    f.push((
        "instParameters".into(),
        SimVal::List(kv.iter().map(|x| x.0.clone()).collect()),
    ));
    let pm: Vec<(String, String)> = kv.iter().filter(|x| x.0 != x.1).cloned().collect();
    if !pm.is_empty() {
        f.push(("propMapping".into(), SimVal::Pairs(pm)));
    }
    Some(SimInfo {
        sim: "spectre".into(),
        fields: f,
    })
}

/// Reverses the §6.3 auCdl rules (`X{name} {pins} / comp k={c}…`, or `P{name} {pins}
/// model K={c}… [$[M]]`).
pub fn cdl_siminfo(t: &str, pins: &[String], params: &[(String, String)]) -> Option<SimInfo> {
    let items = template_items(t)?;
    let [(false, prefix), (true, n), (false, sp), rest @ ..] = items.as_slice() else {
        return None;
    };
    if n != "name" || sp != " " || prefix.is_empty() || prefix.contains(' ') {
        return None;
    }
    // Pins run until the first literal that is not a single space.
    let end = rest
        .iter()
        .position(|it| !it.0 && it.1 != " ")
        .unwrap_or(rest.len());
    let order = pin_list(&rest[..end], pins)?;
    let lit = rest.get(end).map_or("", |x| x.1.as_str());
    let mut tail: Vec<(bool, String)> = rest.get(end + 1..).unwrap_or(&[]).to_vec();
    let mut f = Vec::new();
    let split_first = |s: &str, tail: &mut Vec<(bool, String)>| -> Option<String> {
        let s = s.strip_prefix(' ')?;
        match s.split_once(' ') {
            Some((m, k)) => {
                tail.insert(0, (false, format!(" {k}")));
                Some(m.to_owned())
            }
            None => Some(s.to_owned()),
        }
    };
    if let Some(after) = lit.strip_prefix(" / ") {
        let comp = split_first(&format!(" {after}"), &mut tail)?;
        f.push((
            "netlistProcedure".into(),
            SimVal::Str("ansCdlSubcktCall".into()),
        ));
        f.push(("componentName".into(), SimVal::Str(comp)));
        f.push(("namePrefix".into(), SimVal::Str(prefix.clone())));
    } else {
        f.push((
            "netlistProcedure".into(),
            SimVal::Str("ansCdlCompPrim".into()),
        ));
        f.push(("namePrefix".into(), SimVal::Str(prefix.clone())));
        // `$[M]` at the end (R C L D).
        let mut model_tag = None;
        if let Some((false, last)) = tail.last().cloned()
            && let Some(m) = last.strip_prefix(" $[").and_then(|x| x.strip_suffix(']'))
        {
            model_tag = Some(m.to_owned());
            tail.pop();
        }
        if lit.is_empty() {
            let (true, h) = tail.first()? else {
                return None;
            };
            if h != "model" {
                return None;
            }
            tail.remove(0);
            f.push(("otherParameters".into(), SimVal::List(vec!["model".into()])));
        } else if lit.starts_with(' ') && !lit.contains('=') && model_tag.is_none() {
            let m = split_first(lit, &mut tail)?;
            f.push(("modelName".into(), SimVal::Str(m)));
        } else {
            tail.insert(0, (false, lit.to_owned()));
        }
        if let Some(m) = model_tag {
            f.push(("modelName".into(), SimVal::Str(m)));
        }
    }
    let kv = param_tail(&tail, params)?;
    f.push((
        "termOrder".into(),
        SimVal::List(order.iter().map(|p| vname(p)).collect()),
    ));
    f.push((
        "instParameters".into(),
        SimVal::List(kv.iter().map(|x| x.0.clone()).collect()),
    ));
    let pm: Vec<(String, String)> = kv.iter().filter(|x| x.0 != x.1).cloned().collect();
    if !pm.is_empty() {
        f.push(("propMapping".into(), SimVal::Pairs(pm)));
    }
    Some(SimInfo {
        sim: "auCdl".into(),
        fields: f,
    })
}

pub(super) fn textviews_out(c: &Cell, notes: &mut Notes) -> Vec<TextView> {
    let mut out = Vec::new();
    for r in c.records(RecordKind::Impl) {
        let name = c.field(r, 0);
        let (view, file) = match c.field(r, 1) {
            "veriloga" => ("veriloga", "veriloga.va"),
            "verilog" => ("functional", "verilog.v"),
            _ => continue,
        };
        let body: Option<Vec<&str>> = c
            .records(RecordKind::Body)
            .find(|b| c.field(*b, 0) == name)
            .map(|b| c.doc.verbatim(b).collect());
        match body {
            Some(lines) => out.push(TextView {
                view: view.into(),
                file: file.into(),
                text: lines.join("\n") + "\n",
            }),
            None => notes.review(
                "format",
                &c.path,
                format!("impl {name}: an external `file` is not sent as a text view"),
            ),
        }
    }
    out
}
