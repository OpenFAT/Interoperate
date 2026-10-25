//! PDK side inputs (D14), read by the xschem import: SPICE model cards
//! (bin ranges, `.subckt` ports, the SPICE lexer), layout tech-file grids and Cadence
//! `cdfDump` text. No tool, no job.

pub mod cdf;
pub mod spice;
pub mod tech;
