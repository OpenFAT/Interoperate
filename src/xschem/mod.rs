//! xschem ⇄ Schemify (README "xschem ↔ Schemify map"; specs `import.md`, `interop.md` §11).
//!
//! | Map rows | File |
//! |---|---|
//! | `.sch`/`.sym` record syntax, orientations | `file`, `attr` |
//! | which symbols and schematics are cells; libraries; the search path | `read/cells`, `gather` |
//! | symbol interface: pins, params, formats, limits | `read/interface`, `symbol`, `limits`, `write/symbol` |
//! | records of a cell (graphics, schematic, code) | `read/emit`, `write/schematic` |
//! | `devices/*.sym` ↔ `core:` | `devices` |
//! | values, expressions, Tcl, templates | `template`, `tcl`, `write/values` |
//! | import accounting (count conservation) | `report` |
//! | the two spines | `read::translate_library` (import), `write::export` (export) |

pub mod attr;
pub mod devices;
pub mod file;
pub mod gather;
pub(crate) mod limits;
pub mod read;
pub mod report;
pub mod symbol;
pub mod tcl;
pub mod template;
pub mod write;
