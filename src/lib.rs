//! The interop plugin (D24 as amended 2026-10-12, D41): conversions between Schemify and
//! other tools. xschem import and export (specs `import.md`, `interop.md` §11), Cadence
//! Virtuoso import and export through the `.sfyx` exchange file and the shipped SKILL bridge
//! (`interop.md` §4–§9). Never linked into the core: the editor loads it as a wasm guest
//! (`plugin`, `wasm32-unknown-unknown` only) and the native command line `schemify-interop`
//! (`cli`, feature `cli`) runs the same [`job::Job`] over the file system.
//!
//! The layout follows the user-facing maps in the README (ARCHITECTURE.md): one Schemify
//! `Design` in the middle, the tool-specific translation per tool, and a shared pipeline
//! (`design` load, `map` resolve, `nets`, `remap`, `merge`, report) that names no tool.
//!
//! Readers turn text into data-oriented tables and never evaluate, execute or resolve
//! anything; every reader is total over its input (malformed parts land in an issue table
//! with a location) and asserts count conservation on exit. Behaviour comes from the specs
//! and from golden outputs produced by running the tools, never from their source (D1).

// SIMD triage (simd-loops): inputs are single files of kilobytes (the 3,218-file corpus is
// ~60 MB, read in ~0.1 s), every scanner is a byte state machine (a chain dependency),
// so the scalar loops stay.

#![deny(unsafe_code)] // `forbid` would reject the guest SDK's `export!` (it allows it)
#![warn(missing_docs)]

pub mod check;
#[cfg(feature = "cli")]
pub mod cli;
pub mod decimal;
pub mod design;
pub mod job;
pub mod notes;
pub mod pdk;
pub mod virtuoso;
pub mod xschem;

// The guest target only: the WASI build of `schemify-interop` (CI determinism) must not
// export guest entry points that import the `schemify` host module.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod plugin;

/// Byte range into an arena string owned by the table that hands it out.
///
/// A `Span` only means something together with the arena it was cut from; each table
/// names its arena. Offsets fit `u32` because readers cap input at [`MAX_INPUT_BYTES`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Span {
    /// Byte offset of the first byte.
    pub start: u32,
    /// Length in bytes.
    pub len: u32,
}

impl Span {
    /// Returns the spanned text.
    ///
    /// # Panics
    ///
    /// Panics if the span is not a char-boundary range of `arena`, i.e. if it was cut
    /// from a different arena.
    pub fn of<'a>(&self, arena: &'a str) -> &'a str {
        let start = self.start as usize;
        &arena[start..start + self.len as usize]
    }

    pub(crate) fn new(start: usize, end: usize) -> Span {
        assert!(start <= end, "span start {start} after end {end}");
        assert!(end <= u32::MAX as usize, "span end {end} beyond u32");
        Span {
            start: start as u32,
            len: (end - start) as u32,
        }
    }
}

/// Largest input [`xschem::read`] accepts, in bytes (2^30, the cap of
/// `docs/spec/format.md` §2). Larger inputs are refused before parsing.
pub const MAX_INPUT_BYTES: usize = 1 << 30;

/// Appends `text` to `arena` and returns its span.
pub(crate) fn push_span(arena: &mut String, text: &str) -> Span {
    let start = arena.len();
    arena.push_str(text);
    Span::new(start, arena.len())
}
