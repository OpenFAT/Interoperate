//! The `.sfyx` record kinds (spec `interop.md` §4.2): every tag the reader knows, with the
//! scope it belongs to, in one table. `read::dump` dispatches on [`scope`]: file and cell
//! records and cellview objects it files into the dump's tables itself, maestro records it
//! collects raw for `ade`, and a tag with no row it keeps for the report.
//!
//! A new record kind is a row in its lane's section below, read by the module that owns
//! the kind's scope. The writer (`write::records`) emits kinds in §4.2 order in code, not
//! through this table.

/// Where a record kind may appear, which decides who reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// A file-level context record (`lib`, `cell`).
    File,
    /// A record of the current cell: its CDF, simInfo, text views and the `cv` contexts.
    Cell,
    /// An object of the current cellview.
    Object,
    /// A record of a maestro test context (`maestro` opens one), read by `ade`.
    Maestro,
}

/// Every record kind the reader knows: `(tag, scope)`.
pub const KINDS: &[(&str, Scope)] = &[
    // ---- core (read::dump)
    ("lib", Scope::File),
    ("cell", Scope::File),
    ("cdf", Scope::Cell),
    ("param", Scope::Cell),
    ("siminfo", Scope::Cell),
    ("textview", Scope::Cell),
    ("cv", Scope::Cell),
    ("term", Scope::Object),
    ("pinfig", Scope::Object),
    ("inst", Scope::Object),
    ("spin", Scope::Object),
    ("wire", Scope::Object),
    ("label", Scope::Object),
    ("shape", Scope::Object),
    ("prop", Scope::Object),
    ("conn", Scope::Object),
    // ---- L9: instance textDisplays (sfy_cdfcb.il); callback code (no new kind: `param`'s
    // arity)
    ("td", Scope::Object),
    // ---- maestro (ade::check, ade::import; lanes L1, L6)
    ("maestro", Scope::Maestro),
    ("var", Scope::Maestro),
    ("analysis", Scope::Maestro),
    ("corner", Scope::Maestro),
    ("model", Scope::Maestro),
    ("output", Scope::Maestro),
    ("spec", Scope::Maestro),
    ("defn", Scope::Maestro),
    // ---- L1: maesdb, maestate
    ("maesdb", Scope::Maestro),
    ("maestate", Scope::Maestro),
    // ---- L2: label flags (no new kind: `label`'s arity)
    // ---- L3: config (read::dump files it into its cell, config::import maps it)
    ("config", Scope::Cell),
    // ---- L4: history, job (results::parse; a results dump of its own)
    ("history", Scope::File),
    ("job", Scope::File),
    // ---- L6: a test's own vars, its enable, device parameters
    ("testvar", Scope::Maestro),
    ("testenable", Scope::Maestro),
    ("parameter", Scope::Maestro),
    // ---- L7: ADE L states (`adel`, `adelfile`: `adel::split` turns them into a maestro
    // test before the reader sees them, so they have no row)
    // ---- L8: layout
    // ---- L11: a var's and an output's enable; a maestro view's files (`maefile`:
    // `ade::viewfiles::split` keeps them in the bench header before the reader sees them,
    // so they have no row)
    ("varenable", Scope::Maestro),
    ("outenable", Scope::Maestro),
    // ---- G2 lane S: Virtuoso's connectivity (`tnet`, the import's oracle) and the count of
    // each object kind the dumper left out (`skipped`)
    ("tnet", Scope::Object),
    ("skipped", Scope::Object),
    // CDF attributes beyond paramLabelSet and the param flags (`cdfattr`, `paramattr`), a
    // view the dump does not carry (`skipview`), a library's CDF and libInit.il (`libcdf`)
    ("cdfattr", Scope::Cell),
    ("paramattr", Scope::Cell),
    ("skipview", Scope::Cell),
    ("libcdf", Scope::File),
];

/// The scope of record kind `tag`; `None` for a tag no row names.
pub fn scope(tag: &str) -> Option<Scope> {
    KINDS.iter().find(|(t, _)| *t == tag).map(|&(_, s)| s)
}
