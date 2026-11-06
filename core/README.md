# core: Schemify's built-in device library, as data

The 23 cells every Schemify project has as `lib core builtin` (format spec §4), copied from the
Schemify core at `c5db29e178f5504a81d538e7311d5969f1c7512d`, the revision whose behaviour this
plugin was tested against. Interop reads them to map xschem's and Virtuoso's devices onto
them (their pins, parameters and netlist formats), and its translation check resolves `core:`
references against them. `src/core.rs` compiles them in (`core_library`, `core_file`).

They are here only because Schemify's public SDK does not ship the core library yet; when it
does, read it from there and delete this directory. Refresh the copies when the Schemify the
plugin targets changes a core cell (pins, order and coordinates never move; parameters and
formats may).
