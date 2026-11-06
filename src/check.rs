//! The translation oracle (docs/dev/testing.md `import` row): every generated cell file parses
//! with zero errors and is in canonical form, no library holds two kinds of one cell, every
//! cell reference resolves through the generated manifest, and every expression and template
//! compiles with only identifiers its scope defines.

use std::collections::{BTreeMap, HashMap};

use crate::expr::{Mode, Program, Template};
use schemify_format::{Document, FileKind, RecordKind, parse, write};

use crate::core::core_library;
use crate::template::BUILTINS;

/// Checks translated outputs (`(path, bytes)` as [`crate::translate_library`] returns
/// them). Returns one line per problem; empty means every check passed.
///
/// # Time complexity
///
/// O(total bytes) plus one expression compile per parameter, override and template.
pub fn check_output(files: &[(String, Vec<u8>)]) -> Vec<String> {
    let mut problems = Vec::new();
    // The manifest's library names (format spec §4); `core` is the implicit builtin.
    let mut declared = vec!["core".to_owned()];
    if let Some((_, m)) = files.iter().find(|f| f.0 == "schemify.project") {
        match parse(m) {
            Ok(d) if d.kind() == FileKind::Project => declared.extend(
                (1..d.record_count())
                    .filter(|&r| d.record_kind(r) == RecordKind::Lib)
                    .map(|r| d.field(r, 0).unwrap_or("").to_owned()),
            ),
            Ok(_) => return vec!["schemify.project: not a project manifest".into()],
            Err(e) => return vec![format!("schemify.project: {e}")],
        }
    }
    let core: Vec<(&str, &[u8])> = core_library().to_vec();
    let mut libs: BTreeMap<&str, Vec<(&str, &[u8])>> = BTreeMap::new();
    libs.insert("core", core);
    for (path, bytes) in files {
        let Some((lib, rel)) = path.split_once('/') else {
            continue;
        };
        if [".shprim", ".shcomp", ".shbench"]
            .iter()
            .any(|e| rel.ends_with(e))
        {
            libs.entry(lib).or_default().push((rel, bytes));
        }
    }
    // (lib, cell) → bytes, for parameter kinds of instance targets.
    let mut cells: HashMap<(&str, &str), &[u8]> = HashMap::new();
    for (lib, listing) in &libs {
        if !declared.iter().any(|d| d == lib) {
            problems.push(format!("library {lib}: not in the manifest"));
        }
        for (rel, bytes) in listing {
            let stem = rel.rsplit_once('.').map_or(*rel, |(s, _)| s);
            if cells.insert((lib, stem), bytes).is_some() {
                problems.push(format!("{lib}/{stem}: two kinds of one cell (E020)"));
            }
        }
    }
    for (lib, listing) in &libs {
        if *lib == "core" {
            continue;
        }
        for (rel, bytes) in listing {
            let at = format!("{lib}/{rel}");
            let model = match parse(bytes) {
                Ok(m) => m,
                Err(e) => {
                    for d in e.diagnostics.iter().filter(|d| !d.code.is_warning()) {
                        problems.push(format!("{at}:{}: {} {}", d.line, d.code, d.detail));
                    }
                    continue;
                }
            };
            if write(&model).as_bytes() != *bytes {
                problems.push(format!("{at}: is not in canonical form"));
            }
            for r in 1..model.record_count() {
                if model.record_kind(r) == RecordKind::Inst {
                    let cellref = model.field(r, 1).unwrap_or("");
                    let (tlib, cell) = match cellref.strip_prefix("./") {
                        Some(c) => (*lib, c),
                        None => cellref.split_once(':').unwrap_or(("", cellref)),
                    };
                    if !cells.contains_key(&(tlib, cell)) {
                        problems.push(format!(
                            "{at}:{}: unresolved cell {cellref}",
                            model.record_line(r)
                        ));
                    }
                }
            }
            check_expressions(&at, lib, &model, &cells, &mut problems);
        }
    }
    problems
}

/// A cell's parameters: name → kind word.
fn params_of(d: &Document) -> Vec<(String, String)> {
    (1..d.record_count())
        .filter(|&r| d.record_kind(r) == RecordKind::Param)
        .map(|r| {
            let kind = d
                .find_attr(r, "kind")
                .and_then(|a| d.attr_value(a, 0))
                .unwrap_or("number");
            (d.field(r, 0).unwrap_or("").to_owned(), kind.to_owned())
        })
        .collect()
}

/// Whether `d`'s param `name` is `computed` (an instance may not override it).
fn computed(d: &Document, name: &str) -> bool {
    (1..d.record_count()).any(|r| {
        d.record_kind(r) == RecordKind::Param
            && d.field(r, 0) == Some(name)
            && d.find_attr(r, "computed").is_some()
    })
}

/// A cell's pin names as a template names them: a bus pin `y[5:2]` by its base `y` (elab).
fn pins_of(d: &Document) -> Vec<String> {
    (1..d.record_count())
        .filter(|&r| d.record_kind(r) == RecordKind::Pin)
        .map(|r| {
            let name = d.field(r, 0).unwrap_or("");
            name.split('[').next().unwrap_or(name).to_owned()
        })
        .collect()
}

/// Compiles expression `text`; `Err` names the failure or the first identifier outside
/// `scope`.
fn expr(text: &str, scope: impl Fn(&str) -> bool) -> Result<(), String> {
    let p =
        Program::compile(text, Mode::Scalar).map_err(|e| format!("{:?} {}", e.code, e.message))?;
    match p.idents().find(|i| !scope(i)) {
        Some(i) => Err(format!("unknown identifier {i}")),
        None => Ok(()),
    }
}

/// [`expr`] for a template.
fn template(text: &str, scope: impl Fn(&str) -> bool) -> Result<(), String> {
    let t = Template::parse(text).map_err(|e| format!("{:?} {}", e.code, e.message))?;
    match t.program().idents().find(|i| !scope(i)) {
        Some(i) => Err(format!("unknown identifier {i}")),
        None => Ok(()),
    }
}

fn check_expressions(
    at: &str,
    lib: &str,
    d: &Document,
    cells: &HashMap<(&str, &str), &[u8]>,
    problems: &mut Vec<String>,
) {
    let params = params_of(d);
    let pins = pins_of(d);
    let vars: Vec<String> = (1..d.record_count())
        .filter(|&r| d.record_kind(r) == RecordKind::Var)
        .map(|r| d.field(r, 0).unwrap_or("").to_owned())
        .collect();
    let is_param = |n: &str| params.iter().any(|p| p.0 == n);
    let mut fail = |line: u32, what: &str, text: &str, why: String| {
        problems.push(format!("{at}:{line}: {what} `{text}`: {why}"));
    };
    for r in 1..d.record_count() {
        let line = d.record_line(r);
        match d.record_kind(r) {
            RecordKind::Param => {
                let kind = params
                    .iter()
                    .find(|p| Some(p.0.as_str()) == d.field(r, 0))
                    .map_or("number", |p| p.1.as_str());
                let default = d.field(r, 1).unwrap_or("");
                let res = match kind {
                    "string" | "choice" => template(default, is_param),
                    _ => expr(default, is_param),
                };
                if let Err(e) = res {
                    fail(line, "param default", default, e);
                }
                for key in ["min", "max", "step", "grid"] {
                    if let Some(v) = d.find_attr(r, key).and_then(|a| d.attr_value(a, 0))
                        && let Err(e) = expr(v, is_param)
                    {
                        fail(line, key, v, e);
                    }
                }
            }
            RecordKind::Impl => {
                for a in d.attrs(r).filter(|&a| d.attr_key(a) == "format") {
                    let tpl = d.attr_value(a, 1).unwrap_or("");
                    let scope = |i: &str| {
                        is_param(i) || pins.iter().any(|p| p == i) || BUILTINS.contains(&i)
                    };
                    if let Err(e) = template(tpl, scope) {
                        fail(line, "format", tpl, e);
                    }
                }
            }
            RecordKind::Stext => {
                let tpl = d.field(r, 3).unwrap_or("");
                if let Err(e) = template(tpl, |i| i == "name" || i == "cell" || is_param(i)) {
                    fail(line, "stext", tpl, e);
                }
            }
            RecordKind::Var => {
                let v = d.field(r, 1).unwrap_or("");
                if let Err(e) = expr(v, |i| vars.iter().any(|x| x == i)) {
                    fail(line, "var", v, e);
                }
            }
            RecordKind::Inst => {
                let cellref = d.field(r, 1).unwrap_or("");
                let (tlib, cell) = match cellref.strip_prefix("./") {
                    Some(c) => (lib, c),
                    None => cellref.split_once(':').unwrap_or(("", cellref)),
                };
                let Some(target) = cells.get(&(tlib, cell)).and_then(|b| parse(b).ok()) else {
                    continue;
                };
                let tparams = params_of(&target);
                let scope = |i: &str| is_param(i) || vars.iter().any(|v| v == i);
                for a in d.attrs(r) {
                    let key = d.attr_key(a);
                    // Interop's own attributes (§7.5, `xsprop` the kept xschem value), not
                    // overrides.
                    if matches!(
                        key,
                        "at" | "name" | "ignore" | "inherit" | "vview" | "vprop" | "vtd"
                    ) || key == crate::xschem_out::XSPROP
                    {
                        continue;
                    }
                    let v = d.attr_value(a, 0).unwrap_or("");
                    let Some((_, kind)) = tparams.iter().find(|p| p.0 == key) else {
                        fail(line, "override of unknown param", key, cellref.to_owned());
                        continue;
                    };
                    if computed(&target, key) {
                        fail(
                            line,
                            "override of computed param (E030)",
                            key,
                            cellref.to_owned(),
                        );
                        continue;
                    }
                    let res = match kind.as_str() {
                        "string" | "choice" => template(v, scope),
                        // Number overrides may name bench `.param`s kept in code blocks:
                        // only the syntax is checked here; elab resolves the names.
                        _ => Program::compile(v, Mode::Scalar)
                            .map(|_| ())
                            .map_err(|e| format!("{:?} {}", e.code, e.message)),
                    };
                    if let Err(e) = res {
                        fail(line, key, v, e);
                    }
                }
            }
            _ => {} // no expression of its own
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_library_is_valid() {
        let files: Vec<(String, Vec<u8>)> = Vec::new();
        assert_eq!(check_output(&files), Vec::<String>::new());
        for (name, bytes) in core_library() {
            let d = parse(bytes).unwrap_or_else(|e| panic!("{name}: {e:?}"));
            assert_eq!(
                schemify_format::write(&d).as_bytes(),
                *bytes,
                "{name} is not canonical"
            );
        }
    }

    #[test]
    fn reports_bad_templates_and_unknown_overrides() {
        let lib = "schemify 1 shprim\n\npin a inout 0 0\npin y[5:2] out 10 0\n\nparam w 1\n\nimpl spice spice\n  format spice \"{name} {a} {y} {zz}\"\n";
        let bench = "schemify 1 shbench\n\ninst 0abcde ./x\n  at 0 0 R0\n  name X1\n  q 3\n";
        let files = vec![
            (
                "schemify.project".to_owned(),
                b"schemify 1 project\n\nlib l path l\n".to_vec(),
            ),
            ("l/x.shprim".to_owned(), lib.as_bytes().to_vec()),
            ("l/b.shbench".to_owned(), bench.as_bytes().to_vec()),
        ];
        let p = check_output(&files);
        assert!(
            p.iter().any(|l| l.contains("unknown identifier zz")),
            "{p:?}"
        );
        assert!(
            p.iter().any(|l| l.contains("override of unknown param")),
            "{p:?}"
        );
    }
}
