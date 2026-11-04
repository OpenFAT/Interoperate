//! Mock config views: the hierarchy editor (`hdb*`) functions `sfy_cfg.il` calls. Empty until lane L3 fills it
//! (README "Development"): its rows go in [`ROWS`], its bodies in [`call`].

use super::super::V;
use super::MockDb;

/// The rows of this area in the assembled `mockdb::SIGNATURES`.
pub const ROWS: &[(&str, &str)] = &[];

/// Handles a function of this area; `None` when `name` is not one.
pub fn call(_db: &mut MockDb, _name: &str, _args: &[V]) -> Option<Result<V, String>> {
    None
}
