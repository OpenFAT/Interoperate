//! Maestro (ADE Explorer/Assembler) ⇄ bench tables (spec `interop.md` §9).
//!
//! A bench is one maestro test, or one per `test` record (lane L6); per test, the spine of
//! [`export`]: the test's analysis block (`analysis_source`) → its lines
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
//! (the words both directions share), `runmode` (Monte Carlo and optimizer, §9.9),
//! `mae_xml` (Maestro's setup XML, parsed for the mappings), `restore` (the view's own
//! setup kept in the bench header and sent back, §9.7) and `viewfiles` (the view's setup
//! files kept verbatim, §9.11).

// Data design (six questions): in = one bench document (≤ 2^22 records) or one maestro
// test's records; out = records or bench records. Rows per kind live in one `Vec` each
// (existence-based: no row for an absent kind), at most `ROWS_MAX` per kind, strings owned
// (values are short text). Lifetime: one call. Access: each kind is written once, in §9
// order, after sorting. Benches are independent (parallel by bench; one bench here).

use crate::expr::{Mode, Program, Scratch, Value as XValue, write_num};
use schemify_format::{Document, FileKind, RecordKind};

use super::ael::{self, AelName, Name};
use crate::design::{Cell, Design, push_attr, push_record};
use crate::notes::Notes;
use crate::sfyx::{Error, Record, Value, i, pairs, s, text};

mod disabled;
mod export;
mod forms;
mod import;
pub mod mae_xml;
mod ocean;
mod outputs;
mod restore;
mod runmode;
mod syntax;
#[cfg(test)]
mod tests;
pub mod viewfiles;

pub use export::*;
use forms::*;
pub use import::*;
use mae_xml::Setup;
use ocean::*;
pub use outputs::*;
pub use restore::{VMAEREC, VMAESDB, VMAESTATE, only_changed, with_stored};
pub use runmode::export_run_modes;
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
    /// `(name, value)`: the test's own values of global vars (`testvar`).
    own: Vec<(String, String)>,
    /// `testenable`: `Some(false)` disables the test, `Some(true)` enables it again.
    enable: Option<bool>,
    /// `(path, value, scope)`: device parameters (`parameter`), path `inst[/inst…]/prop`
    /// under the test's design, scope `""` global, else a corner name.
    params: Vec<(String, String, String)>,
    /// `(name, enabled)`: a global var's enable (`varenable`, lane L11).
    var_enable: Vec<(String, bool)>,
    /// `(name, enabled)`: an output's Plot check box, Maestro's enable (`outenable`).
    out_enable: Vec<(String, bool)>,
    /// `(name, value)`: the test's simulator options but `temp` (`simoption`; a dump holds
    /// every one, form defaults included).
    sim_opts: Vec<(String, String)>,
    /// `(corner, runs)`: whether this test runs the corner (`cornerrun`; Maestro's
    /// per-corner disabled tests).
    corner_runs: Vec<(String, bool)>,
    /// `(lib, cell)`: the test's design when it is not the maestro view's cell
    /// (`testdesign`).
    design: Option<(String, String)>,
    /// `(class, text)`: a setting the dumper read that no record carries (`maenote`); the
    /// import reports it `needs_review:<class>`.
    notes: Vec<(String, String)>,
}

impl Test {
    /// The records of the test in §9 order: `maestro`; `testdesign`; global `var`s by
    /// name; `testvar`s by name; `testenable`; `varenable`s by name; `simoption`s by name;
    /// `analysis` by name; the test's `model`s in order; per corner (by name) `corner`, its
    /// `model`s in order and its `var`s by name; `cornerrun`s by corner; `parameter`s by
    /// (scope, path); `output`s by name; `spec`s by (output, op); `outenable`s by name;
    /// `defn`s by file; `maenote`s in order.
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
        out.extend(
            self.design
                .iter()
                .map(|(l, c)| Record::new("testdesign", vec![s(l), s(c)])),
        );
        out.extend(vars.iter().filter(|v| v.2.is_empty()).map(var));
        let mut own = self.own.clone();
        own.sort();
        out.extend(
            own.iter()
                .map(|(n, v)| Record::new("testvar", vec![s(n), s(v)])),
        );
        out.extend(
            self.enable
                .map(|en| Record::new("testenable", vec![i(i32::from(en))])),
        );
        let flags = |rows: &[(String, bool)], tag: &str| {
            let mut rows = rows.to_vec();
            rows.sort();
            rows.into_iter()
                .map(|(n, en)| Record::new(tag, vec![s(&n), i(i32::from(en))]))
                .collect::<Vec<_>>()
        };
        out.extend(flags(&self.var_enable, "varenable"));
        let mut sim_opts = self.sim_opts.clone();
        sim_opts.sort();
        out.extend(
            sim_opts
                .iter()
                .map(|(n, v)| Record::new("simoption", vec![s(n), s(v)])),
        );
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
        out.extend(flags(&self.corner_runs, "cornerrun"));
        let mut params = self.params.clone();
        params.sort_by(|a, b| (&a.2, &a.0).cmp(&(&b.2, &b.0)));
        out.extend(
            params
                .iter()
                .map(|(p, v, sc)| Record::new("parameter", vec![s(p), s(v), s(sc)])),
        );
        for (n, k, e) in &outputs {
            out.push(Record::new("output", vec![s(n), s(k), s(e)]));
        }
        for (o, op, v) in &specs {
            out.push(Record::new("spec", vec![s(o), s(op), s(v)]));
        }
        out.extend(flags(&self.out_enable, "outenable"));
        for (f, t) in &defns {
            out.push(Record::new("defn", vec![s(f), text(t)]));
        }
        out.extend(
            self.notes
                .iter()
                .map(|(c, t)| Record::new("maenote", vec![s(c), s(t)])),
        );
        let n = self.vars.len()
            + self.analyses.len()
            + self.corners.len()
            + self.models.len()
            + self.outputs.len()
            + self.specs.len()
            + self.defns.len()
            + self.own.len()
            + usize::from(self.enable.is_some())
            + self.params.len()
            + self.var_enable.len()
            + self.out_enable.len()
            + self.sim_opts.len()
            + self.corner_runs.len()
            + usize::from(self.design.is_some())
            + self.notes.len();
        assert_eq!(out.len(), n + 1, "every row is one record");
        out
    }
}
