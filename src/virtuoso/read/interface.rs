//! Virtuoso CDF and simInfo → Schemify interface (spec `interop.md` §6.3): impl formats from
//! simInfo.

use super::*;

// ---------------------------------------------------------------------------------------
// Formats from simInfo (§6.3)
// ---------------------------------------------------------------------------------------

/// A pin list for a template: explicit holes when every pin is an identifier, else `{pins}`.
pub(super) fn pins_template(order: &[String]) -> String {
    let ident = |s: &str| {
        let mut ch = s.chars();
        ch.next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && ch.all(|c| c.is_ascii_alphanumeric() || c == '_')
    };
    if order.iter().all(|p| ident(p)) && !order.is_empty() {
        order
            .iter()
            .map(|p| format!("{{{p}}}"))
            .collect::<Vec<_>>()
            .join(" ")
    } else {
        "{pins}".into()
    }
}

/// `(k, c)` per `instParameters` entry that names a CDF param (through `propMapping`).
pub(super) fn inst_params(si: &SimInfo, params: &[ParamIn]) -> Vec<(String, String, bool)> {
    let pm = si.pairs("propMapping");
    si.list("instParameters")
        .into_iter()
        .filter_map(|k| {
            let c = pm
                .iter()
                .find(|(s, _)| *s == k)
                .map_or(k.clone(), |(_, c)| c.clone());
            let p = params.iter().find(|p| p.name == c)?;
            Some((k, c, p.kind != "string"))
        })
        .collect()
}

/// Terminal order of a simInfo in Schemify pin names (falling back to the pin order).
pub(super) fn sim_order(si: &SimInfo, pins: &[PinRow]) -> Vec<String> {
    let ordinary: Vec<&PinRow> = pins.iter().filter(|p| p.alias.is_none()).collect();
    let order = si.list("termOrder");
    let named: Vec<String> = order.iter().map(|t| net_from_virtuoso(t).text).collect();
    if !named.is_empty()
        && named.len() == ordinary.len()
        && named.iter().all(|n| ordinary.iter().any(|p| p.name == *n))
    {
        named
    } else {
        ordinary.iter().map(|p| p.name.clone()).collect()
    }
}

pub(super) fn spectre_format(
    c: &DCell,
    si: &SimInfo,
    params: &[ParamIn],
    pins: &[PinRow],
) -> String {
    let order = sim_order(si, pins);
    let has_model = params
        .iter()
        .any(|p| p.name == "model" && !p.default.is_empty());
    let master = if has_model {
        "{model}".to_owned()
    } else {
        si.str("componentName")
            .filter(|s| !s.is_empty() && *s != "nil")
            .unwrap_or(&c.name)
            .to_owned()
    };
    let mut t = format!("{{name}} ({}) {master}", pins_template(&order));
    for (k, cn, num) in inst_params(si, params) {
        if num {
            t.push_str(&format!(" {k}={{{cn}}}"));
        } else {
            t.push_str(&crate::virtuoso::write::optional_param(&k, &cn));
        }
    }
    t
}

pub(super) fn cdl_format(
    c: &DCell,
    si: &SimInfo,
    params: &[ParamIn],
    pins: &[PinRow],
    notes: &mut Notes,
) -> String {
    let order = sim_order(si, pins);
    let kv: String = inst_params(si, params)
        .iter()
        .map(|(k, cn, _)| format!(" {k}={{{cn}}}"))
        .collect();
    let pins = pins_template(&order);
    if si.str("netlistProcedure") == Some("ansCdlSubcktCall") {
        let comp = si
            .str("componentName")
            .filter(|s| !s.is_empty() && *s != "nil")
            .unwrap_or(&c.name);
        let prefix = si
            .str("namePrefix")
            .filter(|s| !s.is_empty())
            .unwrap_or("X");
        return format!("{prefix}{{name}} {pins} / {comp}{kv}");
    }
    notes.review(
        "cdl-approximate",
        &format!("{}/{}", c.lib, c.name),
        "auCdl format from simInfo (research §3.4)",
    );
    let prefix = si
        .str("namePrefix")
        .filter(|s| !s.is_empty() && *s != "nil")
        .unwrap_or("X");
    let model_name = si.str("modelName").filter(|s| !s.is_empty() && *s != "nil");
    let has_model = params.iter().any(|p| p.name == "model");
    if matches!(prefix, "R" | "C" | "L" | "D")
        && let Some(m) = model_name
    {
        return format!("{prefix}{{name}} {pins}{kv} $[{m}]");
    }
    let model = match model_name {
        Some(m) => format!(" {m}"),
        None if has_model => " {model}".into(),
        None => format!(" {}", c.name),
    };
    format!("{prefix}{{name}} {pins}{model}{kv}")
}
