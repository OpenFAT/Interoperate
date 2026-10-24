//! `interop.map`: cell, pin and parameter translation between Schemify libraries and
//! Virtuoso libraries (spec `interop.md` §8.2). [`crate::plan::resolve`] reads it.
//!
//! The file is line records without a header (format.md §2). It is read with the format
//! crate's own reader by prefixing a `journal` header, the one file kind whose records are
//! all kept as written, so the lexical rules (quoting, escapes, attributes) are exactly
//! the format's.

use std::collections::BTreeMap;

use schemify_format::{Document, FileKind, parse};

/// Rows a mapping file may hold, a bound on the reader's work.
pub const ROWS_MAX: usize = 1 << 16;

/// One `map` row: a Schemify cell and the Virtuoso cell it is exchanged with.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MapRow {
    /// The Schemify cell, `lib:cell`.
    pub cellref: String,
    /// Virtuoso library.
    pub lib: String,
    /// Virtuoso cell.
    pub cell: String,
    /// Virtuoso view (default `symbol`).
    pub view: String,
    /// `(schemify pin, virtuoso terminal)`.
    pub pins: Vec<(String, String)>,
    /// `(schemify param, virtuoso param, scale)`: Virtuoso value = Schemify value × scale.
    pub params: Vec<(String, String, Option<String>)>,
    /// Schemify params neither sent nor taken.
    pub drops: Vec<String>,
    /// `(virtuoso param, value)` always sent.
    pub sets: Vec<(String, String)>,
    /// The ngspice template of the Virtuoso cell's mirror.
    pub spice: Option<String>,
    /// Identical geometry: no re-mastering, pins and params by name.
    pub same: bool,
    /// 1-based line of the `map` record.
    pub line: u32,
}

/// A parsed mapping file. Empty = identity everywhere (mirrors, `schemify_core`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Mapping {
    /// `map` rows, in file order.
    pub maps: Vec<MapRow>,
}

impl Mapping {
    /// Parses `interop.map` text.
    ///
    /// # Errors
    ///
    /// Every malformed row, one per line (`line N: …`): a record other than `map`,
    /// wrong arity, an attribute the record does not take, a `scale` that is not an
    /// expression, `same` beside other attributes, a duplicate cell, Virtuoso target,
    /// pin or param, a param both mapped and dropped, more than [`ROWS_MAX`] rows.
    pub fn parse(text: &str) -> Result<Mapping, Vec<String>> {
        let src = format!("schemify 1 journal\n{text}");
        let doc = parse(src.as_bytes()).map_err(|e| {
            e.diagnostics
                .iter()
                .map(|d| {
                    format!(
                        "line {}: {} {}",
                        d.line.saturating_sub(1),
                        d.code.as_str(),
                        d.detail
                    )
                })
                .collect::<Vec<_>>()
        })?;
        assert_eq!(doc.kind(), FileKind::Journal);
        let mut m = Mapping::default();
        let mut errs = Vec::new();
        if doc.record_count() as usize > ROWS_MAX + 1 {
            return Err(vec![format!("more than {ROWS_MAX} rows")]);
        }
        for r in 1..doc.record_count() {
            let line = doc.record_line(r).saturating_sub(1);
            let fields: Vec<&str> = doc.fields(r).collect();
            if doc.verbatim(r).len() > 0 {
                errs.push(format!("line {line}: verbatim lines are not allowed"));
            }
            match doc.record_word(r) {
                "map" => match map_row(&doc, r, &fields, line) {
                    Ok(row) => m.maps.push(row),
                    Err(e) => errs.extend(e),
                },
                w => errs.push(format!("line {line}: unknown record `{w}` (map)")),
            }
        }
        // Uniqueness across rows.
        let mut seen: BTreeMap<&str, u32> = BTreeMap::new();
        let mut targets: BTreeMap<(&str, &str), u32> = BTreeMap::new();
        for row in &m.maps {
            if let Some(l) = seen.insert(&row.cellref, row.line) {
                errs.push(format!(
                    "line {}: {} is already mapped on line {l}",
                    row.line, row.cellref
                ));
            }
            if let Some(l) = targets.insert((&row.lib, &row.cell), row.line) {
                errs.push(format!(
                    "line {}: Virtuoso cell {}/{} is already the target of line {l}",
                    row.line, row.lib, row.cell
                ));
            }
        }
        if errs.is_empty() {
            assert!(m.maps.len() < doc.record_count() as usize);
            Ok(m)
        } else {
            Err(errs)
        }
    }

    /// The `map` row of Schemify cell `cellref` (`lib:cell`).
    pub fn by_cellref(&self, cellref: &str) -> Option<&MapRow> {
        self.maps.iter().find(|r| r.cellref == cellref)
    }

    /// The `map` row whose Virtuoso target is `lib/cell`.
    pub fn by_virtuoso(&self, lib: &str, cell: &str) -> Option<&MapRow> {
        self.maps.iter().find(|r| r.lib == lib && r.cell == cell)
    }
}

fn attr_values<'a>(doc: &'a Document<'_>, a: u32) -> Vec<&'a str> {
    doc.attr_values(a).collect()
}

fn map_row(doc: &Document<'_>, r: u32, f: &[&str], line: u32) -> Result<MapRow, Vec<String>> {
    let mut errs = Vec::new();
    if !(f.len() == 4 || f.len() == 5) || f[1] != "virtuoso" {
        return Err(vec![format!(
            "line {line}: `map <lib:cell> virtuoso <lib> <cell> [view]`"
        )]);
    }
    if f[0]
        .split_once(':')
        .is_none_or(|(l, c)| l.is_empty() || c.is_empty() || c.contains(':'))
    {
        errs.push(format!(
            "line {line}: `{}` is not a `lib:cell` reference",
            f[0]
        ));
    }
    let mut row = MapRow {
        cellref: f[0].to_owned(),
        lib: f[2].to_owned(),
        cell: f[3].to_owned(),
        view: f.get(4).unwrap_or(&"symbol").to_string(),
        line,
        ..MapRow::default()
    };
    let mut others = 0;
    for a in doc.attrs(r) {
        let at = doc.attr_line(a).saturating_sub(1);
        let v = attr_values(doc, a);
        let key = doc.attr_key(a);
        if key != "same" {
            others += 1;
        }
        let ok = match (key, v.as_slice()) {
            ("pin", [s, t]) => {
                if row.pins.iter().any(|p| p.0 == *s || p.1 == *t) {
                    errs.push(format!("line {at}: pin {s} or {t} mapped twice"));
                }
                row.pins.push(((*s).into(), (*t).into()));
                true
            }
            ("param", [s, t]) => {
                row.params.push(((*s).into(), (*t).into(), None));
                true
            }
            ("param", [s, t, "scale", k]) => {
                if schemify_expr::Program::compile(k, schemify_expr::Mode::Scalar).is_err() {
                    errs.push(format!("line {at}: scale `{k}` is not an expression"));
                }
                row.params
                    .push(((*s).into(), (*t).into(), Some((*k).into())));
                true
            }
            ("drop", [p]) => {
                row.drops.push((*p).into());
                true
            }
            ("set", [p, val]) => {
                row.sets.push(((*p).into(), (*val).into()));
                true
            }
            ("spice", [t]) => {
                if row.spice.replace((*t).into()).is_some() {
                    errs.push(format!("line {at}: second `spice`"));
                }
                true
            }
            ("same", []) => {
                row.same = true;
                true
            }
            _ => false,
        };
        if !ok {
            errs.push(format!(
                "line {at}: `{key}` with {} value(s) is not a map attribute (pin, param, drop, set, spice, same)",
                v.len()
            ));
        }
    }
    if row.same && others > 0 {
        errs.push(format!("line {line}: `same` takes no other attributes"));
    }
    let mut names: Vec<&str> = row.params.iter().map(|p| p.0.as_str()).collect();
    names.sort_unstable();
    if names.windows(2).any(|w| w[0] == w[1]) {
        errs.push(format!("line {line}: a param is mapped twice"));
    }
    if let Some(d) = row.drops.iter().find(|d| names.contains(&d.as_str())) {
        errs.push(format!("line {line}: param {d} is both mapped and dropped"));
    }
    if errs.is_empty() { Ok(row) } else { Err(errs) }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = "map sky130:nfet_01v8 virtuoso sky130_fd_pr nfet_01v8\n  pin d D\n  pin g G\n  pin s S\n  pin b B\n  param W w scale 1e-6\n  param L l scale 1e-6\n  param nf nf\n  param mult m\n  drop ad\n  spice \"X{name} {D} {G} {S} {B} sky130_fd_pr__nfet_01v8 w={w} l={l} nf={nf} m={m}\"\n";

    #[test]
    fn parses_the_spec_example() {
        let m = Mapping::parse(EXAMPLE).unwrap();
        assert_eq!(m.maps.len(), 1);
        let r = &m.maps[0];
        assert_eq!(
            (r.lib.as_str(), r.cell.as_str(), r.view.as_str()),
            ("sky130_fd_pr", "nfet_01v8", "symbol")
        );
        assert_eq!(r.pins.len(), 4);
        assert_eq!(r.params[0], ("W".into(), "w".into(), Some("1e-6".into())));
        assert_eq!(r.drops, ["ad"]);
        assert!(r.spice.as_deref().unwrap().starts_with("X{name}"));
        assert_eq!(r.line, 1);
        assert!(m.by_virtuoso("sky130_fd_pr", "nfet_01v8").is_some());
        assert!(Mapping::parse("").unwrap().maps.is_empty());
    }

    #[test]
    fn every_error_is_listed() {
        for (bad, want) in [
            ("frob x\n", "unknown record"),
            ("map a:b virtuoso L\n", "map <lib:cell>"),
            ("map ab virtuoso L C\n", "not a `lib:cell`"),
            ("map a:b virtuoso L C\n  wat 1\n", "not a map attribute"),
            ("map a:b virtuoso L C\n  same\n  pin x X\n", "`same` takes"),
            (
                "map a:b virtuoso L C\n  param x y scale )\n",
                "not an expression",
            ),
            (
                "map a:b virtuoso L C\nmap a:b virtuoso L D\n",
                "already mapped",
            ),
            (
                "map a:b virtuoso L C\nmap a:c virtuoso L C\n",
                "already the target",
            ),
            (
                "map a:b virtuoso L C\n  pin x X\n  pin x Y\n",
                "mapped twice",
            ),
            (
                "map a:b virtuoso L C\n  param x X\n  drop x\n",
                "both mapped and dropped",
            ),
            ("class c virtuoso_target h\n", "unknown record `class`"),
            ("map \"a\n", "E028"),
        ] {
            let e = Mapping::parse(bad).expect_err(bad);
            assert!(e.iter().any(|l| l.contains(want)), "{bad}: {e:?}");
            assert!(e.iter().all(|l| l.starts_with("line ")), "{e:?}");
        }
        let two = Mapping::parse("frob\nmap x virtuoso a b\n").unwrap_err();
        assert_eq!(two.len(), 2, "{two:?}");
    }
}
