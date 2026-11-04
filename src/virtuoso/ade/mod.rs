//! Maestro (ADE Explorer/Assembler) ⇄ bench tables (spec `interop.md` §9).
//!
//! Spine of [`export`]: the bench's analysis block (`analysis_source`) → its lines
//! (analyses, test model files, `.temp`, `.param`) → the other code blocks (spectre text as
//! definition files) → vars and their sweeps → corners → outputs and specs → reports for
//! the rest → `Test::records` (§9 order). Spine of [`import`]: `parse` (every record
//! checked, with its line) → vars and sweeps → corners → the analysis block (SPICE when
//! every analysis has a SPICE form, else Spectre) → outputs and specs → definition files.
//!
//! Both directions go through one table, `Test`: the export fills it from a bench and
//! writes it; the import parses records into it and writes bench records. Names chosen on
//! import are those the export can carry, so an unchanged maestro re-imports to the same
//! record identities and the stash (§7.3) restores the bench byte for byte: the analysis
//! block is named after the test (the export names the test after that block), a sweep after
//! its var (the export maps only sweeps so named), a definition file `<name>.scs` after its
//! spectre block.
//!
//! Submodules: `export` (bench → test), `import` (test → bench, with the records' check),
//! `ocean` (the OCEAN script, §9.6), `outputs` (output expressions both ways), `syntax`
//! (the words both directions share), `runmode` (Monte Carlo and optimizer, to come),
//! `mae_xml` (Maestro's setup XML, parsed for the mappings) and `restore` (the view's own
//! setup kept in the bench header and sent back, §9.7).

// Data design (six questions): in = one bench document (≤ 2^22 records) or one maestro
// test's records; out = records or bench records. Rows per kind live in one `Vec` each
// (existence-based: no row for an absent kind), at most `ROWS_MAX` per kind, strings owned
// (values are short text). Lifetime: one call. Access: each kind is written once, in §9
// order, after sorting. Benches are independent (parallel by bench; one bench here).

use schemify_expr::{Mode, Program, Scratch, Value as XValue, write_num};
use schemify_format::{Document, FileKind, RecordKind};

use super::ael::{self, AelName, Name};
use crate::design::{Cell, Design, push_attr, push_record};
use crate::notes::Notes;
use crate::sfyx::{Error, Record, Value, i, pairs, s, text};

mod export;
mod import;
pub mod mae_xml;
mod ocean;
mod outputs;
mod restore;
mod runmode;
mod syntax;
#[cfg(test)]
mod tests;

pub use export::*;
pub use import::*;
use ocean::*;
pub use outputs::*;
pub use restore::{VMAEREC, VMAESDB, VMAESTATE, only_changed, with_stored};
use syntax::*;

/// Rows per kind (vars, analyses, corners, models, outputs, specs, definition files) of one
/// test; more is refused on import (a bench, at most 2^22 records, is asserted below it on
/// export).
pub const ROWS_MAX: usize = 1 << 16;

/// Points per sweep (run.md §2's bound).
pub const SWEEP_POINTS_MAX: u32 = 65_536;

/// One analysis row: `(name, enabled, options)`.
type Analysis = (String, bool, Vec<(String, String)>);

/// One maestro test and the setup it sees: the table both directions go through.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Test {
    name: String,
    lib: String,
    cell: String,
    view: String,
    sim: String,
    /// `(name, value, scope)`: scope `""` global, else a corner name.
    vars: Vec<(String, String, String)>,
    /// `(name, enabled, options)`.
    analyses: Vec<Analysis>,
    /// `(name, enabled)`.
    corners: Vec<(String, bool)>,
    /// `(corner, file, section)`: corner `""` = the test's model files.
    models: Vec<(String, String, String)>,
    /// `(name, kind, expr)`: kind `point` or `net`.
    outputs: Vec<(String, String, String)>,
    /// `(output, op, value)`: op `gt` or `lt`.
    specs: Vec<(String, String, String)>,
    /// `(file, text)`.
    defns: Vec<(String, String)>,
}

impl Test {
    /// The records of the test in §9 order: `maestro`; global `var`s by name; `analysis`
    /// by name; the test's `model`s in order; per corner (by name) `corner`, its `model`s in
    /// order and its `var`s by name; `output`s by name; `spec`s by (output, op); `defn`s by
    /// file.
    fn records(&self) -> Vec<Record> {
        let by_name = |a: &(String, String, String), b: &(String, String, String)| {
            (&a.0, &a.2).cmp(&(&b.0, &b.2))
        };
        let mut vars = self.vars.clone();
        vars.sort_by(by_name);
        let mut analyses = self.analyses.clone();
        analyses.sort_by(|a, b| a.0.cmp(&b.0));
        let mut corners = self.corners.clone();
        corners.sort();
        let mut outputs = self.outputs.clone();
        outputs.sort();
        let mut specs = self.specs.clone();
        specs.sort();
        let mut defns = self.defns.clone();
        defns.sort();
        let var =
            |v: &(String, String, String)| Record::new("var", vec![s(&v.0), s(&v.1), s(&v.2)]);
        let model =
            |m: &(String, String, String)| Record::new("model", vec![s(&m.0), s(&m.1), s(&m.2)]);
        let mut out = vec![Record::new(
            "maestro",
            vec![
                s(&self.name),
                s(&self.lib),
                s(&self.cell),
                s(&self.view),
                s(&self.sim),
            ],
        )];
        out.extend(vars.iter().filter(|v| v.2.is_empty()).map(var));
        for (n, en, opts) in &analyses {
            let kv = pairs(opts.iter().map(|(k, v)| (k.as_str(), v.as_str())));
            out.push(Record::new("analysis", vec![s(n), i(i32::from(*en)), kv]));
        }
        out.extend(self.models.iter().filter(|m| m.0.is_empty()).map(model));
        for (c, en) in &corners {
            out.push(Record::new("corner", vec![s(c), i(i32::from(*en))]));
            out.extend(self.models.iter().filter(|m| &m.0 == c).map(model));
            out.extend(vars.iter().filter(|v| &v.2 == c).map(var));
        }
        for (n, k, e) in &outputs {
            out.push(Record::new("output", vec![s(n), s(k), s(e)]));
        }
        for (o, op, v) in &specs {
            out.push(Record::new("spec", vec![s(o), s(op), s(v)]));
        }
        for (f, t) in &defns {
            out.push(Record::new("defn", vec![s(f), text(t)]));
        }
        let n = self.vars.len()
            + self.analyses.len()
            + self.corners.len()
            + self.models.len()
            + self.outputs.len()
            + self.specs.len()
            + self.defns.len();
        assert_eq!(out.len(), n + 1, "every row is one record");
        out
    }
}
