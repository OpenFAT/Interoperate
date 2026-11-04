//! A dump cell's base CDF and simInfo → Schemify params, `cached` lines and netlist formats
//! (spec `interop.md` §6.3; `cached` lines: D20 as amended 2026-10-15).
//!
//! The CDF tables ([`Cdf`], [`CdfParam`], [`SimInfo`]), the §6.3 import table
//! (`params_in`), and the Spectre and CDL formats built from simInfo.

use super::*;

/// One CDF parameter (`param`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CdfParam {
    /// Name.
    pub name: String,
    /// `string int float boolean radio cyclic button netSet`.
    pub ty: String,
    /// Default value text.
    pub def: String,
    /// Prompt.
    pub prompt: String,
    /// Units name or `""`.
    pub units: String,
    /// Flags (`parseAsNumber parseAsCEL storeDefault dontSave hidden`).
    pub flags: Vec<String>,
    /// Choices.
    pub choices: Vec<String>,
    /// Callback text.
    pub callback: String,
}

/// A simInfo value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SimVal {
    /// A string or symbol name.
    Str(String),
    /// A list of names (`termOrder`, `instParameters`, `otherParameters`).
    List(Vec<String>),
    /// Pairs (`propMapping`, `termMapping`).
    Pairs(Vec<(String, String)>),
}

/// One simulator's simInfo.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SimInfo {
    /// Simulator (`spectre`, `auCdl`, …).
    pub sim: String,
    /// `(key, value)` in record order.
    pub fields: Vec<(String, SimVal)>,
}

impl SimInfo {
    /// Field `key`.
    pub fn get(&self, key: &str) -> Option<&SimVal> {
        self.fields.iter().find(|f| f.0 == key).map(|f| &f.1)
    }

    /// Field `key` as a string.
    pub fn str(&self, key: &str) -> Option<&str> {
        match self.get(key)? {
            SimVal::Str(s) => Some(s),
            _ => None,
        }
    }

    /// Field `key` as a list (a string field is a one-element list).
    pub fn list(&self, key: &str) -> Vec<String> {
        match self.get(key) {
            Some(SimVal::List(v)) => v.clone(),
            Some(SimVal::Str(s)) if !s.is_empty() && s != "nil" => vec![s.clone()],
            _ => Vec::new(),
        }
    }

    /// Field `key` as pairs.
    pub fn pairs(&self, key: &str) -> Vec<(String, String)> {
        match self.get(key) {
            Some(SimVal::Pairs(v)) => v.clone(),
            _ => Vec::new(),
        }
    }
}

/// A cell's base CDF.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cdf {
    /// `paramLabelSet`.
    pub label_set: String,
    /// Parameters, CDF order.
    pub params: Vec<CdfParam>,
    /// simInfo per simulator.
    pub siminfo: Vec<SimInfo>,
}

impl Cdf {
    /// simInfo of `sim`.
    pub fn sim(&self, sim: &str) -> Option<&SimInfo> {
        self.siminfo.iter().find(|s| s.sim == sim)
    }
}

/// The CDF parameter types a `param` record may carry.
pub const CDF_TYPES: [&str; 8] = [
    "string", "int", "float", "boolean", "radio", "cyclic", "button", "netSet",
];

/// The `cached <param> <value> <digest>` lines of an instance (D20 as amended 2026-10-15):
/// each derived param's value as Virtuoso had it (`derived`; a param Virtuoso did not store
/// is at its default), with the digest of the instance's input values (`overrides`, else
/// the master's defaults). None when an input or a value is not a constant: the instance
/// then has no cached value, and elab reports it stale once an input is set (E039).
pub(super) fn cached_lines(
    iface: &Iface,
    overrides: &[(String, String)],
    mut derived: Vec<(String, String)>,
) -> Vec<[String; 3]> {
    if iface.vderived.is_empty() || iface.vinputs.is_empty() {
        return Vec::new();
    }
    let template = |n: &str| matches!(iface.kind(n), Some("string" | "choice"));
    let digest = crate::virtuoso::characterize::inputs_digest(&iface.vinputs, |n| {
        let v = overrides
            .iter()
            .find(|o| o.0 == n)
            .map(|o| o.1.as_str())
            .or_else(|| iface.def(n))?;
        Some((v, template(n)))
    });
    let Some(digest) = digest else {
        return Vec::new();
    };
    for p in &iface.vderived {
        if !derived.iter().any(|d| &d.0 == p)
            && let Some(def) = iface.def(p)
        {
            derived.push((p.clone(), def.to_owned()));
        }
    }
    derived.sort_by_key(|(n, _)| iface.params.iter().position(|p| &p.0 == n));
    derived
        .into_iter()
        .filter_map(|(p, v)| {
            let v = crate::virtuoso::characterize::canonical_value(&v, template(&p))?;
            Some([p, v, digest.clone()])
        })
        .collect()
}

/// A constant CDF value Virtuoso printed for a param of Schemify `kind`, as the import
/// writes it (the instance-parameter translation below, without parameters or variables);
/// `None` when it is not one.
pub(crate) fn constant_from_cdf(kind: &str, value: &str) -> Option<String> {
    match kind {
        "bool" => ael::bool_from_cdf(value).map(str::to_owned),
        "int" => Some(value.trim().to_owned()),
        "string" | "choice" => Some(ael::literal_to_template(value)),
        _ => ael::from_ael(value, |_| AelName::Unknown, |_| None).map(|t| si_text(&t.text)),
    }
}

/// One imported param.
#[derive(Clone, Debug)]
pub(super) struct ParamIn {
    pub(super) name: String,
    pub(super) kind: String,
    pub(super) default: String,
    pub(super) unit: Option<&'static str>,
    pub(super) doc: Option<String>,
    pub(super) choices: Vec<String>,
}

/// The §6.3 import table over a cell's CDF.
pub(super) fn params_in(c: &DCell, cdf: &Cdf, notes: &mut Notes) -> Vec<ParamIn> {
    let cell = format!("{}/{}", c.lib, c.name);
    let names: Vec<&str> = cdf.params.iter().map(|p| p.name.as_str()).collect();
    let mut out = Vec::new();
    for p in &cdf.params {
        if !p.callback.trim().is_empty() {
            notes.review(
                "callback",
                &cell,
                format!("param {}: callback not carried", p.name),
            );
        }
        let (kind, default) = match p.ty.as_str() {
            "button" | "netSet" => continue,
            "int" => (
                "int",
                if p.def.trim().parse::<i64>().is_ok() {
                    p.def.trim().to_owned()
                } else {
                    "0".into()
                },
            ),
            "float" => (
                "number",
                own_default(&p.def, &names).unwrap_or_else(|| "0".into()),
            ),
            "boolean" => (
                "bool",
                ael::bool_from_cdf(&p.def).unwrap_or("false").to_owned(),
            ),
            "radio" | "cyclic" => ("choice", ael::literal_to_template(&p.def)),
            _ => {
                let numeric = p.flags.iter().any(|f| f == "parseAsNumber");
                match own_default(&p.def, &names).filter(|_| numeric) {
                    Some(d) => ("number", d),
                    None => ("string", ael::literal_to_template(&p.def)),
                }
            }
        };
        out.push(ParamIn {
            name: p.name.clone(),
            kind: kind.into(),
            default,
            unit: ael::unit_from_cdf(&p.units),
            doc: (!p.prompt.is_empty() && p.prompt != p.name).then(|| p.prompt.clone()),
            choices: p.choices.clone(),
        });
    }
    out
}

/// A CDF default as a Schemify expression when it is numeric: translates, compiles, and
/// names only params of the same cell (`iPar("x")` → `x`).
fn own_default(def: &str, names: &[&str]) -> Option<String> {
    if def.trim().is_empty() {
        return None;
    }
    let t = ael::from_ael(
        def,
        |n| {
            if names.contains(&n) {
                AelName::Param
            } else {
                AelName::Unknown
            }
        },
        |n| names.contains(&n).then(|| n.to_owned()),
    )?;
    if t.review.is_some() {
        return None;
    }
    let p = schemify_expr::Program::compile(&t.text, schemify_expr::Mode::Scalar).ok()?;
    p.idents()
        .all(|i| names.contains(&i))
        .then(|| si_text(&t.text))
}

/// A number literal as engineering text (`150.00n` → `150n`, `900.0m`, `0.9` → `900m`); any
/// other text unchanged. Virtuoso reformats the numbers of parseAsNumber CDF parameters
/// (live ICADVM20.1 run: `0.15u` came back `150.00n`), so both sides of the stash
/// comparison (§7.3) read them through this.
pub fn si_text(t: &str) -> String {
    let Ok(x) = schemify_expr::parse_num(t.trim()) else {
        return t.to_owned();
    };
    if x == 0.0 || !x.is_finite() {
        return if x == 0.0 { "0".into() } else { t.to_owned() };
    }
    // Shortest round-trip mantissa digits and decimal exponent of |x|.
    let e = format!("{:e}", x.abs());
    let Some((m, exp)) = e.split_once('e') else {
        return t.to_owned();
    };
    let Ok(exp) = exp.parse::<i32>() else {
        return t.to_owned();
    };
    let digits: String = m.chars().filter(char::is_ascii_digit).collect();
    let eng = exp.div_euclid(3) * 3;
    let suffix = match eng {
        -15 => "f",
        -12 => "p",
        -9 => "n",
        -6 => "u",
        -3 => "m",
        0 => "",
        3 => "k",
        6 => "meg",
        9 => "g",
        12 => "t",
        _ => return t.to_owned(),
    };
    // The point goes after 1 + (exp - eng) digits.
    let int_len = usize::try_from(exp - eng + 1).unwrap_or(1);
    let mut d = digits;
    while d.len() < int_len {
        d.push('0');
    }
    let (i, f) = d.split_at(int_len);
    let f = f.trim_end_matches('0');
    let sign = if x < 0.0 { "-" } else { "" };
    if f.is_empty() {
        format!("{sign}{i}{suffix}")
    } else {
        format!("{sign}{i}.{f}{suffix}")
    }
}

/// A pin list for a template: explicit holes when every pin is an identifier, else `{pins}`.
pub(crate) fn pins_template(order: &[String]) -> String {
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
fn inst_params(si: &SimInfo, params: &[ParamIn]) -> Vec<(String, String, bool)> {
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
fn sim_order(si: &SimInfo, pins: &[PinRow]) -> Vec<String> {
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
        "auCdl format from simInfo",
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
