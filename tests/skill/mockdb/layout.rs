//! Mock layout: the functions `sfy_layout.il` calls (lane L8). Signatures only: the layout
//! flow runs XStream and the file work through `sh`, which the mock does not emulate, so
//! the live check (`live/checks/l8_layout.il`) is its test; [`call`] handles nothing.

use super::super::V;
use super::MockDb;

/// The rows of this area in the assembled `mockdb::SIGNATURES` (skdfref, sktechfile,
/// SKILL Language Reference `sh`).
pub const ROWS: &[(&str, &str)] = &[
    ("sh", "t"),
    ("techGetTechFile", "d"),
    ("techGetDBUPerUU", "d t"),
    ("drDumpDrf", "t [g]"),
    ("dbRefreshCellView", "d"),
    ("ddDeleteObj", "d"),
];

/// Handles a function of this area; `None` when `name` is not one.
pub fn call(_db: &mut MockDb, _name: &str, _args: &[V]) -> Option<Result<V, String>> {
    None
}
