//! Where each cell of an export goes (spec `interop.md` §8.2 lookups): the closure of the
//! tops, then per cell how it crosses. Reads the Design and the mapping; writes the [`Plan`].

use std::collections::BTreeMap;

use crate::design::Design;
use crate::map::Mapping;
use crate::virtuoso::devices;

/// How a cell crosses to Virtuoso.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum How {
    /// Not in the export's closure.
    Unused,
    /// A user cell written in full (`"design"` cell).
    Design,
    /// A Virtuoso mirror (`provenance virtuoso <lib>/<cell>`): referenced, never written.
    Mirror,
    /// A bundled core cell written to `schemify_core` (or renamed by `--lib core=…`).
    Core,
    /// Sent to the Virtuoso cell of `map` row `k`.
    Map(u32),
    /// Re-mastered onto analogLib (`--analoglib`), device row `k` of [`devices::DEVICES`];
    /// sources pick the waveform cell per instance.
    AnalogLib(u32),
}

/// Library of the bundled core cells in Virtuoso.
pub const CORE_LIB: &str = "schemify_core";

/// Virtuoso library of the project's root directory (Schemify library `""`) unless
/// `--lib =<name>` renames it.
pub const ROOT_LIB: &str = "schemify";

/// The export plan: per Design cell, where it goes. Columns are parallel to `d.cells`.
#[derive(Clone, Debug, Default)]
pub struct Plan {
    /// How each cell crosses.
    pub how: Vec<How>,
    /// Virtuoso library.
    pub lib: Vec<String>,
    /// Virtuoso cell.
    pub cell: Vec<String>,
    /// Virtuoso view instances of the cell use.
    pub view: Vec<String>,
}

impl Plan {
    /// True when instances of cell `ci` change master geometry (§6.4).
    pub fn remastered(&self, ci: usize, m: &Mapping) -> bool {
        match self.how[ci] {
            How::AnalogLib(_) => true,
            How::Map(k) => !m.maps[k as usize].same,
            _ => false,
        }
    }
}

/// The Virtuoso library name of Schemify library `lib` (`--lib` renames, `""` → [`ROOT_LIB`],
/// `core` → [`CORE_LIB`]).
pub fn virtuoso_lib(lib: &str, lib_map: &BTreeMap<String, String>) -> String {
    if let Some(v) = lib_map.get(lib) {
        return v.clone();
    }
    match lib {
        "" => ROOT_LIB.into(),
        "core" => CORE_LIB.into(),
        l => l.into(),
    }
}

/// The closure of `tops` (cells instantiated from them, transitively), as a mask over
/// `d.cells`.
pub fn closure(d: &Design, tops: &[usize]) -> Vec<bool> {
    let mut seen = vec![false; d.cells.len()];
    let mut stack: Vec<usize> = tops.to_vec();
    while let Some(ci) = stack.pop() {
        assert!(ci < d.cells.len(), "top {ci} is not a cell");
        if std::mem::replace(&mut seen[ci], true) {
            continue;
        }
        let c = &d.cells[ci];
        for r in c.records(schemify_format::RecordKind::Inst) {
            if let Some(t) = d.resolve(ci, c.field(r, 1)) {
                stack.push(t);
            }
        }
    }
    seen
}

/// The provenance target `lib/cell` of a Virtuoso mirror cell.
pub fn mirror_of(c: &crate::design::Cell) -> Option<(String, String)> {
    let p = c.header("provenance")?;
    let rest = p.strip_prefix("virtuoso ")?;
    let (l, cell) = rest.trim().split_once('/')?;
    (!l.is_empty() && !cell.is_empty()).then(|| (l.to_owned(), cell.to_owned()))
}

/// Resolves every cell of the closure of `tops` (§8.2 lookups, in order: a `map` row; a
/// mirror; `core:*` → `schemify_core` or analogLib with `analoglib`; a user cell).
///
/// # Errors
///
/// One line per problem: a pin of a mapped cell without a `pin` row, a `pin`/`param` row
/// naming something the cell lacks.
pub fn resolve(
    d: &Design,
    tops: &[usize],
    m: &Mapping,
    analoglib: bool,
    lib_map: &BTreeMap<String, String>,
) -> Result<Plan, Vec<String>> {
    let used = closure(d, tops);
    let n = d.cells.len();
    let mut p = Plan {
        how: vec![How::Unused; n],
        lib: vec![String::new(); n],
        cell: vec![String::new(); n],
        view: vec![String::new(); n],
    };
    let mut errs = Vec::new();
    for (ci, &u) in used.iter().enumerate() {
        if !u {
            continue;
        }
        let c = &d.cells[ci];
        let cellref = format!("{}:{}", c.lib, c.name);
        let (how, lib, cell, view) =
            if let Some(k) = m.maps.iter().position(|r| r.cellref == cellref) {
                let row = &m.maps[k];
                let pins = crate::design::sym_pins(c);
                if !row.same {
                    for (k2, pn) in pins.name.iter().enumerate() {
                        if pins.alias[k2].is_none() && !row.pins.iter().any(|x| &x.0 == pn) {
                            errs.push(format!(
                                "{cellref}: pin {pn} has no `pin` row (interop.map line {})",
                                row.line
                            ));
                        }
                    }
                }
                for (s, _) in &row.pins {
                    if !pins.name.contains(s) {
                        errs.push(format!(
                            "{cellref}: interop.map line {}: no pin {s}",
                            row.line
                        ));
                    }
                }
                for (s, _, _) in &row.params {
                    if !c
                        .records(schemify_format::RecordKind::Param)
                        .any(|r| c.field(r, 0) == s)
                    {
                        errs.push(format!(
                            "{cellref}: interop.map line {}: no param {s}",
                            row.line
                        ));
                    }
                }
                (
                    How::Map(k as u32),
                    row.lib.clone(),
                    row.cell.clone(),
                    row.view.clone(),
                )
            } else if let Some((l, cl)) = mirror_of(c) {
                (How::Mirror, l, cl, "symbol".into())
            } else if c.lib == "core" {
                match devices::DEVICES
                    .iter()
                    .position(|dv| dv.core == c.name && !dv.alib.is_empty())
                {
                    Some(k) if analoglib => (
                        How::AnalogLib(k as u32),
                        "analogLib".into(),
                        devices::DEVICES[k].alib[0].into(),
                        "symbol".into(),
                    ),
                    _ => (
                        How::Core,
                        virtuoso_lib("core", lib_map),
                        c.name.clone(),
                        "symbol".into(),
                    ),
                }
            } else {
                (
                    How::Design,
                    virtuoso_lib(&c.lib, lib_map),
                    c.name.clone(),
                    "symbol".into(),
                )
            };
        p.how[ci] = how;
        p.lib[ci] = lib;
        p.cell[ci] = cell;
        p.view[ci] = view;
    }
    assert_eq!(p.how.len(), n);
    if errs.is_empty() { Ok(p) } else { Err(errs) }
}
