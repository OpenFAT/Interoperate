//! xschem ⇄ Schemify (README "xschem ↔ Schemify map"; specs `import.md`, `interop.md` §11).
//!
//! | Map rows | File |
//! |---|---|
//! | `.sch`/`.sym` record syntax | `file` (reader), `attr` (attribute strings) |
//! | which symbols and schematics are cells, libraries, search path | `read` (`classify_symbols`, `plan_cells`, `name_libs`), `gather` |
//! | symbol interface: pins, params, formats | `symbol`, `read` (`plan_symbols`), `template` |
//! | `devices/*.sym` ↔ `core:` | `devices` |
//! | values, expressions, Tcl | `template`, `tcl` |
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
