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
    /// The callback's code as the dumper read it from a design library (`inline` or
    /// `libInit.il`, and the forms as list text; sfy_cdfcb.il), never written back.
    pub source: Option<(String, String)>,
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
    /// The rule a design-library callback assigns it by (`computed`, §6.3).
    pub(super) computed: Option<String>,
}

/// The detail of a reference cell's callback row before `group_kit_callbacks`.
const KIT_CALLBACK: &str = "kit callback not carried: ";

/// A kit's callback rows, one per param of every reference cell (15066 for GF22FDX), as
/// one row per (library, callback function): how many params of how many cells, the first
/// as an example (gaps2 P13). A design cell's rows stay as they are.
pub(super) fn group_kit_callbacks(notes: &mut Notes) {
    // (lib, function) → (params, cells, example)
    let mut g: BTreeMap<(String, String), (usize, BTreeSet<String>, String)> = BTreeMap::new();
    notes.rows.retain(|r| {
        let ref_row = r.class == "needs_review:callback" && r.cell.contains('/');
        let Some((param, cb)) = r.detail.split_once(": ").filter(|_| ref_row) else {
            return true;
        };
        let Some(cb) = cb
            .strip_prefix(KIT_CALLBACK)
            .filter(|_| param.starts_with("param "))
        else {
            return true;
        };
        let (lib, cell) = r.cell.split_once('/').expect("lib/cell");
        // `GFcdfCallback('w)` and `(pdkCb)` alike: the function's name.
        let fun = cb
            .trim_start_matches('(')
            .split(['(', ' ', ')'])
            .next()
            .unwrap_or(cb)
            .to_owned();
        let e = g
            .entry((lib.to_owned(), fun))
            .or_insert_with(|| (0, BTreeSet::new(), format!("{cell} {param}: {cb}")));
        e.0 += 1;
        e.1.insert(cell.to_owned());
        false
    });
    for ((lib, fun), (n, cells, first)) in g {
        notes.review(
            "callback",
            &lib,
            format!(
                "{fun}: callbacks of {n} params in {} cells not carried (e.g. {first})",
                cells.len()
            ),
        );
    }
}

/// The §6.3 import table over a cell's CDF.
pub(super) fn params_in(c: &DCell, cdf: &Cdf, notes: &mut Notes) -> Vec<ParamIn> {
    let cell = format!("{}/{}", c.lib, c.name);
    let names: Vec<&str> = cdf.params.iter().map(|p| p.name.as_str()).collect();
    let rules = computed_rules(c, cdf, notes);
    let mut out = Vec::new();
    for p in &cdf.params {
        if !c.design && !p.callback.trim().is_empty() {
            notes.review(
                "callback",
                &cell,
                format!("param {}: {KIT_CALLBACK}{}", p.name, p.callback.trim()),
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
            computed: rules.iter().find(|r| r.0 == p.name).map(|r| r.1.clone()),
        });
    }
    out
}

/// The `computed` rules of a design cell's callbacks (§6.3), `(param, expression)`, with a
/// report line per callback: what it became, or why it stays Virtuoso's. A reference
/// (PDK) cell's callbacks are never read. Two callbacks assigning one parameter, and rules
/// that depend on one another in a cycle, stay Virtuoso's.
fn computed_rules(c: &DCell, cdf: &Cdf, notes: &mut Notes) -> Vec<(String, String)> {
    if !c.design {
        return Vec::new();
    }
    let cell = format!("{}/{}", c.lib, c.name);
    let mut all: Vec<(&str, Vec<Rule>)> = Vec::new();
    for p in cdf.params.iter().filter(|p| !p.callback.trim().is_empty()) {
        match callback_rules(cdf, p) {
            Ok(rules) => all.push((&p.name, rules)),
            Err(why) => notes.review(
                "callback",
                &cell,
                format!("param {}: callback not carried: {why}", p.name),
            ),
        }
    }
    // A target two callbacks assign differently has no single rule (the callbacks of `w`
    // and `nf` both setting `wf = w / nf` is one rule).
    let differs = |r: &Rule, all: &[(&str, Vec<Rule>)]| {
        all.iter()
            .flat_map(|x| &x.1)
            .any(|q| q.target == r.target && q.expr != r.expr)
    };
    let mut bad: Vec<(&str, String)> = Vec::new();
    for (p, rules) in &all {
        if let Some(r) = rules.iter().find(|r| differs(r, &all)) {
            bad.push((
                p,
                format!("{} is also assigned by another callback", r.target),
            ));
        }
    }
    // Then cycles: keep peeling rules whose inputs no remaining rule assigns.
    let mut left: Vec<&Rule> = all
        .iter()
        .filter(|x| !bad.iter().any(|b| b.0 == x.0))
        .flat_map(|x| &x.1)
        .collect();
    loop {
        let before = left.len();
        let targets: Vec<String> = left.iter().map(|r| r.target.clone()).collect();
        left.retain(|r| r.reads.iter().any(|n| targets.contains(n)));
        if left.len() == before {
            break;
        }
    }
    for (p, rules) in &all {
        if let Some(r) = rules
            .iter()
            .find(|r| left.iter().any(|l| l.target == r.target))
        {
            bad.push((
                p,
                format!("{} depends on itself through other rules", r.target),
            ));
        }
    }
    let mut out = Vec::new();
    for (p, rules) in &all {
        if let Some((_, why)) = bad.iter().find(|b| b.0 == *p) {
            notes.review(
                "callback",
                &cell,
                format!("param {p}: callback not carried: {why}"),
            );
            continue;
        }
        for r in rules {
            notes.add(
                "callback",
                &cell,
                format!(
                    "param {p}: callback becomes {} computed `{}`",
                    r.target, r.expr
                ),
            );
            if !out.iter().any(|o: &(String, String)| o.0 == r.target) {
                out.push((r.target.clone(), r.expr.clone()));
            }
        }
    }
    out
}

/// One assignment of a callback: `target = expr` over the params `reads`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Rule {
    pub(crate) target: String,
    pub(crate) expr: String,
    pub(crate) reads: Vec<String>,
}

/// A callback form as `sfy_cdfcb.il`'s `SfyCbForm` writes it.
#[derive(Clone, Debug, PartialEq)]
enum Form {
    List(Vec<Form>),
    Str(String),
    Atom(String),
}

/// Nesting a callback form may have (the text is the dump's, so its depth is bounded).
const FORM_DEPTH_MAX: u32 = 64;

/// Parses one list-text form; `None` on anything else (an unbalanced or trailing text).
fn parse_form(t: &str) -> Option<Form> {
    fn go(b: &[u8], k: &mut usize, depth: u32) -> Option<Form> {
        while *k < b.len() && b[*k].is_ascii_whitespace() {
            *k += 1;
        }
        match b.get(*k)? {
            b'(' => {
                if depth >= FORM_DEPTH_MAX {
                    return None;
                }
                *k += 1;
                let mut items = Vec::new();
                loop {
                    while *k < b.len() && b[*k].is_ascii_whitespace() {
                        *k += 1;
                    }
                    if *b.get(*k)? == b')' {
                        *k += 1;
                        return Some(Form::List(items));
                    }
                    items.push(go(b, k, depth + 1)?);
                }
            }
            b')' => None,
            b'"' => {
                let start = *k + 1;
                let end = start + b[start..].iter().position(|&c| c == b'"')?;
                *k = end + 1;
                Some(Form::Str(
                    String::from_utf8_lossy(&b[start..end]).into_owned(),
                ))
            }
            _ => {
                let start = *k;
                while *k < b.len() && !b[*k].is_ascii_whitespace() && !b"()\"".contains(&b[*k]) {
                    *k += 1;
                }
                Some(Form::Atom(
                    String::from_utf8_lossy(&b[start..*k]).into_owned(),
                ))
            }
        }
    }
    let b = t.as_bytes();
    let mut k = 0;
    let f = go(b, &mut k, 0)?;
    b[k..].iter().all(u8::is_ascii_whitespace).then_some(f)
}

/// The rules of one design-library callback (§6.3): it must only assign, to float or int
/// parameters, expressions of number literals, other parameters' values, arithmetic and
/// `sqrt exp log min max abs floor ceiling sin cos expt pow`. Anything else (a branch,
/// a local variable, a database or string call, a call of another procedure, code the
/// dumper could not read) is the reason it stays Virtuoso's.
pub(crate) fn callback_rules(cdf: &Cdf, p: &CdfParam) -> Result<Vec<Rule>, String> {
    if p.callback.contains('\n') {
        return Err("a callback of several lines".into());
    }
    let Some((from, code)) = &p.source else {
        return Err(format!(
            "`{}` has no readable code: a procedure the library's libInit.il does not \
             define (a PDK's, compiled or encrypted) stays Virtuoso's",
            p.callback.trim()
        ));
    };
    let form = parse_form(code).ok_or("its code does not read")?;
    let body = match (from.as_str(), form) {
        ("inline", Form::List(forms)) => forms,
        ("libInit.il", Form::List(f)) => match f.as_slice() {
            [Form::Atom(h), Form::List(sig), body @ ..] if h == "procedure" => {
                if sig.len() != 1 {
                    return Err("its procedure takes arguments".into());
                }
                body.to_vec()
            }
            _ => return Err("its libInit.il form is not a procedure".into()),
        },
        _ => return Err(format!("code from `{from}`")),
    };
    if body.is_empty() {
        return Err("it assigns nothing".into());
    }
    let mut out: Vec<Rule> = Vec::new();
    for f in &body {
        let (target, value) = assignment(cdf, f)?;
        let mut reads = Vec::new();
        let (expr, int) = expr_of(cdf, value, &mut reads)?;
        let ty = cdf
            .params
            .iter()
            .find(|q| q.name == target)
            .map(|q| q.ty.as_str());
        match ty {
            Some("float") => {}
            Some("int") if int => {}
            Some("int") => return Err(format!("it assigns a float to int parameter {target}")),
            Some(t) => return Err(format!("it assigns {target}, a {t} parameter")),
            None => unreachable!("assignment checks the target"),
        }
        if reads.contains(&target) {
            return Err(format!("it reads {target}, the parameter it assigns"));
        }
        if out.iter().any(|r| r.target == target) {
            return Err(format!("it assigns {target} twice"));
        }
        if schemify_expr::Program::compile(&expr, schemify_expr::Mode::Scalar).is_err() {
            return Err(format!("`{expr}` does not compile"));
        }
        reads.sort();
        reads.dedup();
        out.push(Rule {
            target,
            expr,
            reads,
        });
    }
    Ok(out)
}

/// Why a call of `head` is not plain arithmetic.
fn refused(head: &str) -> String {
    match head {
        "if" | "when" | "unless" | "cond" | "case" | "caseq" | "and" | "or" | "not" => {
            format!("it branches (`{head}`)")
        }
        "let" | "prog" | "setq" | "for" | "foreach" | "while" => {
            format!("it uses a local variable or loop (`{head}`)")
        }
        "sprintf"
        | "strcat"
        | "substring"
        | "atof"
        | "atoi"
        | "evalstring"
        | "cdfParseFloatString"
        | "aelNumber"
        | "parseString"
        | "concat" => {
            format!("it handles strings (`{head}`)")
        }
        h if ["db", "dd", "ge", "hi", "sch", "cdf", "env", "le"]
            .iter()
            .any(|p| h.starts_with(p)) =>
        {
            format!("it calls the database or the editor (`{head}`)")
        }
        h => format!("it calls `{h}`"),
    }
}

/// `(putpropq REF value value)` or `(setSGq REF value value)` (SKILL's reading of
/// `cdfgData->x->value = …` and `~>`): the target param and the value form.
fn assignment<'f>(cdf: &Cdf, f: &'f Form) -> Result<(String, &'f Form), String> {
    match f {
        Form::List(items) => match items.as_slice() {
            [Form::Atom(h), r, v, Form::Atom(a)]
                if (h == "putpropq" || h == "setSGq") && a == "value" =>
            {
                Ok((param_ref(cdf, r)?, v))
            }
            [Form::Atom(h), ..] => Err(refused(h)),
            _ => Err("a form that is not a call".into()),
        },
        _ => Err("a form that is not an assignment".into()),
    }
}

/// `(getq cdfgData P)`, `(getSGq cdfgData P)` or `(cdfFindParamByName cdfgData "P")`: P,
/// a param of the CDF.
fn param_ref(cdf: &Cdf, f: &Form) -> Result<String, String> {
    let Form::List(items) = f else {
        return Err("it assigns something other than a parameter".into());
    };
    let name = match items.as_slice() {
        [Form::Atom(h), Form::Atom(d), Form::Atom(p)]
            if (h == "getq" || h == "getSGq") && d == "cdfgData" =>
        {
            p
        }
        [Form::Atom(h), Form::Atom(d), Form::Str(p)]
            if h == "cdfFindParamByName" && d == "cdfgData" =>
        {
            p
        }
        [Form::Atom(h), ..] => return Err(refused(h)),
        _ => return Err("it assigns something other than a parameter".into()),
    };
    if !cdf.params.iter().any(|q| &q.name == name) {
        return Err(format!("it names {name}, which the CDF does not declare"));
    }
    Ok(name.clone())
}

/// A value form as a Schemify expression and whether SKILL computes it as an integer (an
/// integer quotient truncates in SKILL, never in Schemify, so it is refused).
fn expr_of(cdf: &Cdf, f: &Form, reads: &mut Vec<String>) -> Result<(String, bool), String> {
    // An operand inside a binary expression: bracketed when it is one itself (a blank
    // outside brackets) or a negation.
    let operand = |t: String| {
        let mut depth = 0i32;
        let compound = t.starts_with('-')
            || t.chars().any(|c| {
                depth += i32::from(c == '(') - i32::from(c == ')');
                c == ' ' && depth == 0
            });
        if compound { format!("({t})") } else { t }
    };
    let items = match f {
        Form::Atom(a) if a.parse::<i64>().is_ok() => return Ok((a.clone(), true)),
        Form::Atom(a) if a.parse::<f64>().is_ok_and(f64::is_finite) => {
            return Ok((si_text(a), false));
        }
        Form::Atom(a) => return Err(format!("it reads the variable `{a}`")),
        Form::Str(_) => return Err("it handles strings (a string literal)".into()),
        Form::List(items) => items,
    };
    let [Form::Atom(head), args @ ..] = items.as_slice() else {
        return Err("a form that is not a call".into());
    };
    // A parameter's value.
    if let ([r, Form::Atom(v)], "getq" | "getSGq") = (args, head.as_str())
        && v == "value"
    {
        let n = param_ref(cdf, r)?;
        let ty = cdf
            .params
            .iter()
            .find(|q| q.name == n)
            .map_or("", |q| q.ty.as_str());
        if !matches!(ty, "float" | "int") {
            return Err(format!(
                "it reads {n}, a {ty} parameter (its value is text)"
            ));
        }
        reads.push(n.clone());
        return Ok((n, ty == "int"));
    }
    const OPS: [&str; 16] = [
        "plus",
        "times",
        "difference",
        "minus",
        "quotient",
        "float",
        "sqrt",
        "exp",
        "sin",
        "cos",
        "log",
        "abs",
        "floor",
        "ceiling",
        "min",
        "max",
    ];
    if !OPS.contains(&head.as_str()) && head != "expt" && head != "pow" {
        return Err(refused(head));
    }
    let mut a = Vec::new();
    for x in args {
        a.push(expr_of(cdf, x, reads)?);
    }
    let all_int = a.iter().all(|x| x.1);
    let join = |op: &str, a: Vec<(String, bool)>| {
        a.into_iter()
            .map(|x| operand(x.0))
            .collect::<Vec<_>>()
            .join(op)
    };
    let one = |a: &[(String, bool)]| -> Result<String, String> {
        match a {
            [x] => Ok(x.0.clone()),
            _ => Err(format!("`{head}` takes one argument")),
        }
    };
    Ok(match (head.as_str(), a.len()) {
        ("plus", 1..) => (join(" + ", a), all_int),
        ("times", 1..) => (join(" * ", a), all_int),
        ("difference" | "minus", 1) => (format!("-{}", operand(a[0].0.clone())), all_int),
        ("difference", 2..) => (join(" - ", a), all_int),
        ("quotient", 2..) if all_int => {
            return Err("it divides integers (SKILL truncates, Schemify does not)".into());
        }
        ("quotient", 2..) => (join(" / ", a), false),
        ("float", 1) => (one(&a)?, false),
        ("sqrt" | "exp" | "sin" | "cos", _) => (format!("{head}({})", one(&a)?), false),
        ("log", _) => (format!("ln({})", one(&a)?), false),
        ("abs", _) => (format!("abs({})", one(&a)?), all_int),
        ("floor", _) => (format!("floor({})", one(&a)?), true),
        ("ceiling", _) => (format!("ceil({})", one(&a)?), true),
        ("min" | "max", 2..) => (
            format!(
                "{head}({})",
                a.into_iter().map(|x| x.0).collect::<Vec<_>>().join(", ")
            ),
            all_int,
        ),
        ("expt" | "pow", 2) => (format!("pow({}, {})", a[0].0, a[1].0), all_int),
        ("plus" | "times" | "difference" | "quotient" | "min" | "max" | "expt" | "pow", _) => {
            return Err(format!("`{head}` with {} arguments", a.len()));
        }
        (h, _) => return Err(refused(h)),
    })
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

/// The pin part of a format from a simInfo's termOrder: a terminal by its pin's hole
/// (`term_pin`, so a stopping view's inherited terminal is its inherited pin); a
/// `progn(<param>)` entry (analogLib's auCdl bulk) by the net the param's value names, a
/// terminal of the cell or else a net (live ICADVM20.1, p_terms: `bn` `S` netlists the S
/// terminal's net, `vdd!` that net). Any other entry, or a drawn pin it leaves out, gives
/// the pin order, reported (gaps2 S8, P1).
fn sim_pins(
    c: &DCell,
    si: &SimInfo,
    params: &[ParamIn],
    pins: &[PinRow],
    notes: &mut Notes,
) -> String {
    let ordinary: Vec<&PinRow> = pins.iter().filter(|p| p.alias.is_none()).collect();
    let names: Vec<String> = ordinary.iter().map(|p| p.name.clone()).collect();
    let fallback = pins_template(&names);
    let order = si.list("termOrder");
    if order.is_empty() || fallback == "{pins}" {
        return fallback;
    }
    let cell = format!("{}/{}", c.lib, c.name);
    let sym = c.cv("symbol");
    let mut holes = Vec::new();
    let mut named = Vec::new();
    for t in &order {
        let progn = t
            .strip_prefix("progn(")
            .or_else(|| t.strip_prefix("(progn "))
            .and_then(|x| x.strip_suffix(')'))
            .map(str::trim)
            .filter(|p| params.iter().any(|x| x.name == *p));
        if let Some(p) = progn {
            // A terminal's name picks that pin's net; any other value is the net itself.
            let mut e = p.to_owned();
            for term in sym
                .iter()
                .flat_map(|s| s.terms.iter().map(move |x| (s, x)))
                .rev()
            {
                e = format!(
                    "{p} == \"{}\" ? {} : {e}",
                    term.1.name,
                    term_pin(term.0, term.1)
                );
            }
            holes.push(format!("{{{e}}}"));
            continue;
        }
        let n = match sym.and_then(|s| s.terms.iter().find(|x| x.name == *t).map(|x| (s, x))) {
            Some((s, term)) => term_pin(s, term),
            None => net_from_virtuoso(t).text,
        };
        if !names.contains(&n) {
            notes.review(
                "format",
                &cell,
                format!(
                    "{} termOrder names `{t}`, which no pin is: the pin order",
                    si.sim
                ),
            );
            return fallback;
        }
        holes.push(format!("{{{n}}}"));
        named.push(n);
    }
    if let Some(p) = ordinary
        .iter()
        .find(|p| p.inherit.is_none() && !named.contains(&p.name))
    {
        notes.review(
            "format",
            &cell,
            format!(
                "{} termOrder leaves out pin `{}`: the pin order",
                si.sim, p.name
            ),
        );
        return fallback;
    }
    holes.join(" ")
}

/// What a cell's simInfo holds that its formats do not carry, as one report line (gaps2
/// S9, P14): spectre and auCdl keys outside the ones the formats read (a netlist
/// procedure, string/enum parameter lists, terminal mappings and operating-point
/// expressions for results, …) and every other simulator's simInfo (hspiceD, auLvs, ams).
pub(super) fn siminfo_not_carried(cdf: &Cdf) -> Option<String> {
    // ponytail: one row per cell, keys only; a kit has hundreds of cells with the same few.
    let read = |sim: &str, k: &str, v: &SimVal| {
        let empty = match v {
            SimVal::Str(s) => s.is_empty() || s == "nil",
            SimVal::List(l) => l.is_empty(),
            SimVal::Pairs(p) => p.is_empty(),
        };
        empty
            || matches!(
                k,
                "termOrder" | "instParameters" | "propMapping" | "componentName"
            )
            || (k == "otherParameters"
                && matches!(v, SimVal::List(l) if l.iter().all(|x| x == "model")))
            || (sim == "auCdl" && matches!(k, "namePrefix" | "modelName"))
            || (sim == "auCdl" && k == "netlistProcedure")
    };
    let mut out = Vec::new();
    for si in &cdf.siminfo {
        match si.sim.as_str() {
            "spectre" | "auCdl" => {
                let keys: Vec<&str> = si
                    .fields
                    .iter()
                    .filter(|(k, v)| !read(&si.sim, k, v))
                    .map(|(k, _)| k.as_str())
                    .collect();
                if !keys.is_empty() {
                    out.push(format!("{} {}", si.sim, keys.join(" ")));
                }
            }
            other => out.push(format!("{other} (all)")),
        }
    }
    (!out.is_empty()).then(|| format!("simInfo not carried: {}", out.join("; ")))
}

pub(super) fn spectre_format(
    c: &DCell,
    si: &SimInfo,
    params: &[ParamIn],
    pins: &[PinRow],
    notes: &mut Notes,
) -> String {
    let pins = sim_pins(c, si, params, pins, notes);
    // The model param names the master when it has a default or simInfo passes it to the
    // netlister (`otherParameters`: analogLib nmos, model "" set per instance; live
    // ICADVM20.1 p_terms `M0 (n_D n_G n_S 0) nch`).
    let other = si.list("otherParameters");
    let has_model = params
        .iter()
        .any(|p| p.name == "model" && (!p.default.is_empty() || other.contains(&p.name)));
    let master = if has_model {
        "{model}".to_owned()
    } else {
        si.str("componentName")
            .filter(|s| !s.is_empty() && *s != "nil")
            .unwrap_or(&c.name)
            .to_owned()
    };
    let mut t = format!("{{name}} ({pins}) {master}");
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
    let holes = sim_pins(c, si, params, pins, notes);
    // `k=` per instParameters entry; one naming an inherited pin is a node, which auCdl
    // writes `$k=<net>` (live ICADVM20.1, p_terms: `RR0 a b $SUB=VSS! $[opnpcres] r=…`).
    let kv = |skip: &str| -> String {
        inst_params(si, params)
            .iter()
            .filter(|x| x.0 != skip)
            .map(|(k, cn, _)| {
                let node = pins.iter().any(|p| p.inherit.is_some() && p.name == *cn);
                format!(" {}{k}={{{cn}}}", if node { "$" } else { "" })
            })
            .collect()
    };
    if si.str("netlistProcedure") == Some("ansCdlSubcktCall") {
        let comp = si
            .str("componentName")
            .filter(|s| !s.is_empty() && *s != "nil")
            .unwrap_or(&c.name);
        let prefix = si
            .str("namePrefix")
            .filter(|s| !s.is_empty())
            .unwrap_or("X");
        return format!("{prefix}{{name}} {holes} / {comp}{}", kv(""));
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
        return format!("{prefix}{{name}} {holes}{} $[{m}]", kv(""));
    }
    match model_name {
        Some(m) => format!("{prefix}{{name}} {holes} {m}{}", kv("")),
        // The model is the master; auCdl writes no `model=` beside it (live ICADVM20.1,
        // p_terms: `MMN OUT IN VSS inh_lSup nfet m=1 w=200n …`, gaps2 G7).
        None if has_model => format!("{prefix}{{name}} {holes} {{model}}{}", kv("model")),
        None => format!("{prefix}{{name}} {holes} {}{}", c.name, kv("")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A design cell CDF: `w` `l` float, `nf` int, `wf` `area` float, `s` string, `cb`'s
    /// callback with code `(from, code)`.
    fn cdf(cb: &str, from: &str, code: &str) -> Cdf {
        let p = |name: &str, ty: &str| CdfParam {
            name: name.into(),
            ty: ty.into(),
            def: "1".into(),
            ..CdfParam::default()
        };
        let mut c = Cdf {
            params: vec![
                p("w", "float"),
                p("l", "float"),
                p("nf", "int"),
                p("wf", "float"),
                p("area", "float"),
                p("s", "string"),
            ],
            ..Cdf::default()
        };
        let q = c.params.iter_mut().find(|q| q.name == cb).unwrap();
        q.callback = "cb()".into();
        q.source = Some((from.into(), code.into()));
        c
    }

    const W: &str = "(getq (getq cdfgData w) value)";
    const NF: &str = "(getq (getq cdfgData nf) value)";

    fn rules(code: &str) -> Result<Vec<(String, String)>, String> {
        let c = cdf("nf", "inline", code);
        callback_rules(&c, &c.params[2])
            .map(|v| v.into_iter().map(|r| (r.target, r.expr)).collect())
    }

    fn one(code: &str) -> String {
        match rules(code) {
            Ok(v) => {
                assert_eq!(v.len(), 1, "{v:?}");
                format!("{} = {}", v[0].0, v[0].1)
            }
            Err(e) => panic!("{code}: {e}"),
        }
    }

    fn why(code: &str) -> String {
        rules(code).expect_err(code)
    }

    #[test]
    fn plain_assignments_become_rules() {
        // cdfgData->wf->value = cdfgData->w->value / cdfgData->nf->value (live ICADVM20.1).
        assert_eq!(
            one(&format!(
                "((putpropq (getq cdfgData wf) (quotient {W} {NF}) value))"
            )),
            "wf = w / nf"
        );
        // ~> and cdfFindParamByName, the math functions, float and int literals.
        assert_eq!(
            one(&format!(
                "((setSGq (getSGq cdfgData area) (times (sqrt {W}) (max (getSGq (getSGq cdfgData l) value) (float 9.9999999999999995e-07)) (expt {NF} 2)) value))"
            )),
            "area = sqrt(w) * max(l, 1u) * pow(nf, 2)"
        );
        assert_eq!(
            one(&format!(
                "((putpropq (cdfFindParamByName cdfgData \"wf\") (difference (log {W}) (minus (abs {W})) (floor {W}) (ceiling (exp {W})) (sin {W}) (cos {W}) (min {W} (float 2))) value))"
            )),
            "wf = ln(w) - (-abs(w)) - floor(w) - ceil(exp(w)) - sin(w) - cos(w) - min(w, 2)"
        );
        // A float divisor makes an int quotient real, as in SKILL.
        assert_eq!(
            one(&format!(
                "((putpropq (getq cdfgData wf) (quotient {NF} (float 2)) value))"
            )),
            "wf = nf / 2"
        );
        // Several assignments; an int target takes an int expression.
        let c = cdf(
            "w",
            "libInit.il",
            &format!(
                "(procedure (myCb) (putpropq (getq cdfgData wf) (quotient {W} {NF}) value) (putpropq (getq cdfgData nf) (plus 1 2) value))"
            ),
        );
        let r = callback_rules(&c, &c.params[0]).unwrap();
        assert_eq!(
            r.iter()
                .map(|r| format!("{} = {}", r.target, r.expr))
                .collect::<Vec<_>>(),
            ["wf = w / nf", "nf = 1 + 2"]
        );
        assert_eq!(r[0].reads, ["nf", "w"]);
    }

    #[test]
    fn everything_else_stays_virtuosos_with_its_reason() {
        let set = |e: &str| format!("((putpropq (getq cdfgData wf) {e} value))");
        for (code, want) in [
            (
                format!("((if (greaterp {NF} 1) then (setq a 1) else (setq a 2)))"),
                "it branches (`if`)",
            ),
            ("((let (x) (setq x 1)))".into(), "local variable"),
            (set("(dbGet cdfgData)"), "database or the editor (`dbGet`)"),
            (
                set("(cdfParseFloatString (getq (getq cdfgData s) value))"),
                "handles strings",
            ),
            (set("(sprintf nil \"%g\" 1)"), "handles strings (`sprintf`)"),
            (set("(pdkWidth 1)"), "it calls `pdkWidth`"),
            ("((pdkCallback))".into(), "it calls `pdkCallback`"),
            (set(&format!("(quotient {NF} 2)")), "divides integers"),
            (
                set("(getq (getq cdfgData s) value)"),
                "reads s, a string parameter",
            ),
            (
                "((putpropq (getq cdfgData s) 1 value))".into(),
                "assigns s, a string parameter",
            ),
            (
                "((putpropq (getq cdfgData nf) (float 1) value))".into(),
                "a float to int parameter nf",
            ),
            (
                set("(getq (getq cdfgData wf) value)"),
                "reads wf, the parameter it assigns",
            ),
            (
                set("(getq (getq cdfgData zz) value)"),
                "names zz, which the CDF does not declare",
            ),
            (set("x"), "reads the variable `x`"),
            (set("(min 1)"), "`min` with 1 arguments"),
            (
                "((putpropq (getq cdfgData wf) 1 value) (putpropq (getq cdfgData wf) 2 value))"
                    .into(),
                "assigns wf twice",
            ),
            ("()".into(), "assigns nothing"),
            (
                "((putpropq (getq cdfgData wf) 1 value)".into(),
                "does not read",
            ),
        ] {
            let got = why(&code);
            assert!(got.contains(want), "{code}: {got}");
        }
        let mut c = cdf(
            "w",
            "libInit.il",
            "(procedure (cb x) (putpropq (getq cdfgData wf) 1 value))",
        );
        assert!(
            callback_rules(&c, &c.params[0])
                .unwrap_err()
                .contains("takes arguments")
        );
        c.params[0].source = None;
        assert!(
            callback_rules(&c, &c.params[0])
                .unwrap_err()
                .contains("no readable code")
        );
        c.params[0].callback = "a()\nb()".into();
        assert!(
            callback_rules(&c, &c.params[0])
                .unwrap_err()
                .contains("several lines")
        );
    }

    fn cell(design: bool) -> DCell {
        DCell {
            lib: "work".into(),
            name: "amp".into(),
            design,
            ..DCell::default()
        }
    }

    #[test]
    fn reference_and_pdk_cells_are_never_translated() {
        // Even with code in the record (the dumper never writes it for a ref cell).
        let c = cdf(
            "nf",
            "inline",
            &format!("((putpropq (getq cdfgData wf) (quotient {W} {NF}) value))"),
        );
        let mut notes = Notes::default();
        let p = params_in(&cell(false), &c, &mut notes);
        assert!(p.iter().all(|p| p.computed.is_none()));
        assert_eq!(
            notes.text().matches("callback not carried").count(),
            1,
            "{}",
            notes.text()
        );
        assert!(!notes.text().contains("becomes"));
        let mut notes = Notes::default();
        let p = params_in(&cell(true), &c, &mut notes);
        assert_eq!(
            p.iter()
                .find(|p| p.name == "wf")
                .unwrap()
                .computed
                .as_deref(),
            Some("w / nf")
        );
        assert!(
            notes
                .text()
                .contains("param nf: callback becomes wf computed `w / nf`"),
            "{}",
            notes.text()
        );
    }

    #[test]
    fn conflicting_and_cyclic_rules_stay_virtuosos() {
        let assign = |t: &str, from: &str| {
            format!(
                "((putpropq (getq cdfgData {t}) (times 2 (getq (getq cdfgData {from}) value)) value))"
            )
        };
        let mut c = cdf("w", "inline", &assign("wf", "l"));
        c.params[1].callback = "cb()".into();
        c.params[1].source = Some(("inline".into(), assign("wf", "w")));
        let mut notes = Notes::default();
        assert!(
            params_in(&cell(true), &c, &mut notes)
                .iter()
                .all(|p| p.computed.is_none())
        );
        assert_eq!(
            notes
                .text()
                .matches("also assigned by another callback")
                .count(),
            2
        );
        // The same rule from two callbacks (w's and nf's both setting wf = 2 * l) is one.
        c.params[1].source = Some(("inline".into(), assign("wf", "l")));
        let p = params_in(&cell(true), &c, &mut Notes::default());
        assert_eq!(p[3].computed.as_deref(), Some("2 * l"));
        // wf from area, area from wf: a cycle; l from w stays fine.
        let mut c = cdf("w", "inline", &assign("wf", "area"));
        c.params[1].callback = "cb()".into();
        c.params[1].source = Some(("inline".into(), assign("area", "wf")));
        c.params[5].callback = "cb()".into();
        c.params[5].source = Some(("inline".into(), assign("l", "w")));
        let mut notes = Notes::default();
        let p = params_in(&cell(true), &c, &mut notes);
        let computed: Vec<&str> = p
            .iter()
            .filter(|p| p.computed.is_some())
            .map(|p| p.name.as_str())
            .collect();
        assert_eq!(computed, ["l"], "{}", notes.text());
        assert_eq!(notes.text().matches("depends on itself").count(), 2);
    }
}
