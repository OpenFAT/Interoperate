//! Mock ADE L: the `asi*` session functions `sfy_adel.il` calls. Empty until lane L7 fills it
//! (README "Development"): its rows go in [`ROWS`], its bodies in [`call`].

use super::super::V;
use super::MockDb;

/// The rows of this area in the assembled `mockdb::SIGNATURES`.
pub const ROWS: &[(&str, &str)] = &[];

/// Handles a function of this area; `None` when `name` is not one.
pub fn call(_db: &mut MockDb, _name: &str, _args: &[V]) -> Option<Result<V, String>> {
    None
}
