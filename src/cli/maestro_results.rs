//! `maestro-results`: a maestro history's results → Schemify waves. Lane L4 fills it
//! (README "Development"); until then it refuses.

/// Runs `maestro-results`: refuses with status 2, not implemented yet.
pub(super) fn maestro_results(_args: &[String]) -> u8 {
    eprintln!("schemify-interop: maestro-results: not implemented");
    2
}
