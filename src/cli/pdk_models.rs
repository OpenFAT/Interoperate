//! `pdk-models`: SpiceRack's model conversion (`crate::pdk_models`), its report printed.

/// Runs `pdk-models` on `args`; its exit status.
pub(super) fn pdk_models(args: &[String]) -> u8 {
    let (out, code) = crate::pdk_models::run(args);
    print!("{out}");
    code
}
