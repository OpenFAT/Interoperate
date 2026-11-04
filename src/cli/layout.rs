//! `layout-export`, `layout-import`: layout through KLayout. Lane L8 fills it (README
//! "Development"); until then both refuse.

/// Runs `cmd`: refuses with status 2, not implemented yet.
pub(super) fn layout(cmd: &str, _args: &[String]) -> u8 {
    eprintln!("schemify-interop: {cmd}: not implemented");
    2
}
