# Architecture: the mapping is the code

**Status: plan, 2026-10-24, decisions made.** § Today describes the code as it is. § Target
is where it goes. § Phases tracks what has landed; when every phase is checked, § Today is
deleted and this file is the architecture. The normative behaviour stays in Schemify's
[interop.md](https://github.com/OmarSiwy/Schemify/blob/main/docs/spec/interop.md) and
[import.md](https://github.com/OmarSiwy/Schemify/blob/main/docs/spec/import.md); this file
says where each rule lives and why the code has this shape.

## The one idea

The README's two maps (*Virtuoso ↔ Schemify*, *xschem ↔ Schemify*) are the architecture.
One Schemify-shaped `Design` sits in the middle. Each tool is a module whose files are the
rows of its map: one file per row group, holding the tool-side table for that row and the
two inverse transforms over it (to the tool, from the tool), with a round-trip test beside
them. Everything that does not name a tool (loading, closure, connectivity, merging,
reporting, the I/O state machine) is shared and lives at the crate root. A reader who has a
row of the map in mind opens one file; a reader with a file open sees one row of the map.

```
 tool files ──T::read──► Design ──plan · nets · remap──► T::write ──► tool files
                  ▲          │
                  │          └──store (merge with the project's files, §7.3)──► Schemify files
 Schemify files ──design::load──┘
```

## Decisions

Rule for every one: one behaviour, no knob, unless two users need different answers today.
Deletion before addition.

| # | Decision | Deletes | Why |
|---|---|---|---|
| 1 | **Devices always cross on the other tool's devices.** Core cells go to analogLib / `devices/*.sym`; analogLib and `devices/*.sym` instances come back as `core:` cells; `schemify_core` is written only for a core cell with no analogLib row (`bsource probe switch vswitch`). | `--core-cells`, `--analoglib`, `--no-core`; `ExportOptions::analoglib`, `ImportOptions::analoglib`, `Options::map_core`; the analogLib mirror library on import | Today three defaults disagree: the CLI and the panel export analogLib, the library API (and so every round-trip test) exports `schemify_core`, the import defaults to a mirror. No fixture places one of the five devices whose pins do not align (`ammeter vcvs vccs tline parax_cap`), so every round-trip oracle stays byte-identical; those five get one netlist-equivalence test (spicecmp) and the documented loss (spec §6.4). |
| 2 | **One report, one name rule.** `<stem>.report.{md,txt}` beside a file output (`amp.sfyx` → `amp.report.md`), `interop.report.{md,txt}` inside a directory output (xschem import and export). | `notes.rs`; `import-report.*`, `xschem-export.report.*`; `report::Drop`, `template::Reason` (consts instead); `job::summarize` (its facts become `info` rows) | Three conventions, two types, two writers, two spec sections for one thing. The base is a property of the command, not a branch. |
| 3 | **The xschem import merges** with the project's files like the Virtuoso import (spec §7.3). | the second import path, `translate::manifest`, the `maestro: bool` of `merge` | Re-importing a library today overwrites docs, waivers, bounds and ngspice formats that spec §7.1 says Schemify owns. Ownership becomes data: `owned` bits per cell. |
| 4 | **One command grammar**, the CLI's (`… -o <out> [--lib L=V]…`), parsed in `job.rs` for both drivers. The panel prompt says `-o`. The CLI's D14 side-input flags (`--models --spiceinit --tech --cdf --xschemrc`) stay a CLI-only prefix step that fills `ReadOpts`. | `plugin::command`, `plugin::xschem_import`, `cli::job_command`; the "last path ending in `.sfyx`" inference | Two parsers for one vocabulary. |
| 5 | **One producer**: `concat!("schemify-interop ", env!("CARGO_PKG_VERSION"))`. | `ExportOptions::producer` | A header field with no consumer that decides anything. |
| 6 | **`lib_dir` is a parameter of the stash oracle** (`virtuoso::image`), not an option. | `ImportOptions::lib_dir` | Nothing outside that call sets it. |
| 7 | **The tool seam is `trait Tool` as a generic bound**, one `match` on a closed `ToolKind` in `Job::step`. | the per-command bodies of `Job::step` | Two tools exist; one spine body instead of two or four; the compiler checks a third tool's signatures. No `dyn`: the kind is known when the command is parsed. |
| 8 | **Files are the map's rows**, with the small rows folded: libraries live in `cells.rs`, attribute strings stay their own lexer (`attrs.rs`). Net +6 files, every one from splitting `translate.rs` (3,578), `read.rs` (3,965) and `write.rs` (2,438) along the rows; bodies unchanged. | — | The user's ask (the architecture fits the mapping); moves add no degrees of freedom. |
| 9 | **Tables are SoA with `u32` handles and `Span`s into an arena**, capacities stated; `How` is a closed sum type; `owned` is a `u64` bitset over `RecordKind`. | `Vec<Item>`, `Vec<Note>` | House rule (data layout beats brevity); `Span` and the SoA shape already exist in this crate (`lib.rs`, `xschem.rs`, `design::Geom`). |

## Today

Three generations in one crate (32,110 lines), each with its own shape:

| Generation | Code | Shape |
|---|---|---|
| xschem import (D1, D14; ex `crates/import`, moved here by the D24 amendment of 2026-10-12) | `translate.rs` 3,578 · `xschem.rs` · `symbol.rs` · `attr.rs` · `template.rs` · `tcl.rs` · `core.rs` · `gather.rs` + D14 readers `spice.rs` `tech.rs` `cdf.rs` `limits.rs` | `(path, bytes)` in → `(path, bytes)` out. Never builds a `Design`; writes its own manifest (`translate.rs:2773`) and report files (`translate.rs:155`); never merges with existing files; typed `report::Report` with per-file count conservation |
| Virtuoso bridge (D41, 2026-10-12) | `virtuoso/{read 3,965, write 2,438, ade 2,949, ael, devices, characterize}.rs` + shared `design.rs` `nets.rs` `map.rs` `remap.rs` `align.rs` `merge.rs` `profile.rs` `sfyx.rs` `notes.rs` | `design::load → Design → map::resolve → Plan → write`; `sfyx::read → read::import` builds documents **and** merges inside one function (`read.rs:3403`); string-classed `notes::Notes` |
| xschem export (D24 as amended 2026-10-12) | `xschem_out.rs` 1,718 | `design::load → Design → export`; its own `Target` enum and `closure` instead of `map::Plan`; `Notes` |

What that costs, with the file and line:

- **Two reports.** `report::Report` (`Class::Review(template::Reason)`, `Item.records`, conservation asserted) and `notes::Notes` (`class: String`, no counts). Two Markdown writers, two spec sections (import.md §7, interop.md §12), three file-name conventions: `import-report.{md,txt}` (`translate.rs:155`), `<base>.report.{md,txt}` (`job.rs:279,533`), `<dir>/xschem-export.report.{md,txt}` (`job.rs:323`).
- **Three device defaults.** `cli.rs:194` and `plugin.rs:210` export analogLib, `ExportOptions::default()` (every test) exports `schemify_core`, `ImportOptions::default()` imports a mirror library; `--no-core` adds a fourth path for xschem.
- **Two device tables with one role.** `core::DEVICE_MAP` (xschem `devices/*.sym` ↔ core, pin and param renames) and `virtuoso::devices::DEVICES` (analogLib ↔ core, plus ngspice templates and Spectre simInfo). Same lookup contract, different homes (`core.rs` at the root, `devices.rs` in the tool).
- **Two "where does this cell go" decisions.** `map::How` (`Unused Design Mirror Core Map AnalogLib`, `map.rs:237`) and `xschem_out::Target` (`Own Device Label Original`, `xschem_out.rs:60`), two `closure` functions (`map.rs:299`, `xschem_out.rs:146`), two spellings of the `provenance` rule (`map::mirror_of`, `xschem_out::target_of`).
- **Two import paths.** The Virtuoso import merges with the project's files (`merge::merge`, fingerprints, stash); the xschem import overwrites. `check_output` runs only after the xschem import (`job.rs:396`).
- **Dependency direction.** The shared leaf imports a tool: `design::place` calls `translate::place` (`design.rs:678`). The xschem export imports the xschem import: `xschem_out.rs:30` uses `translate::ORIENT_TABLE`. `profile.rs` ("tool spellings as data", spec §3 covers both tools) holds only Virtuoso's.
- **The spine is a `match` with bodies.** `Job::step` (`job.rs:178–331`) has one body per command; `finish` serves two of four; the xschem import is a separate method with its own report writing and a 70-line `summarize`.
- **Two command parsers** for one vocabulary: `cli::job_command` (`-o`) and `plugin::command` ("the last path ending in `.sfyx`").
- **Stale headers.** `lib.rs:1–8` and `Cargo.toml` still say "Virtuoso import/export and xschem export are to come".

## Target

### Data

Six questions, answered once for the shared tables. In: tool files (xschem: thousands of
KB-sized `.sym`/`.sch`; Virtuoso: one `.sfyx` of MBs) or Schemify files. Out: the other
side's files plus a report. Everything lives for one conversion (the `Job` is the arena);
rows are `u32` handles; per-cell work is independent but sequential (0.3 s for the 3,342-file
corpus, `translate.rs` header), so no parallel pass. Strings live in an arena and are
referenced by `Span` (`lib.rs`).

| Table | Columns (SoA) | Range → width | Capacity | Producer → consumers |
|---|---|---|---|---|
| `Design` (`design.rs`) | `cells: Vec<Cell { path, lib, name, doc: Document }>`, `libs`, `resolved` | cells < 2^16 → `u32` handle (doc's per-project cap) | 2^16 | `design::load`, `T::read` → `plan`, `nets`, `T::write`, `store` |
| `Plan` (`plan.rs`) | `how: Vec<How>` where `How { Unused, Own, Original, Core(u32), Map(u32) }` | one per cell; row indices < 2^16 → `u32` | ‖ cells | `plan::resolve` → `remap`, `T::write` (each tool spells its target from `How` in its `cells.rs`) |
| `Nets`, `Edits` (`nets.rs`, `remap.rs`) | union-find over pins; pulled-back wires, stubs, labels | per cell < 2^22 objects | 2^22 | `nets::extract`, `remap::apply` → `T::write`, `store` |
| `Imported` (`tool.rs`) | `cell: Vec<Cell>`, `stash: Vec<Option<String>>`, `owned: Vec<u64>` (bitset over `RecordKind`, < 64 kinds, asserted), `headers: Vec<Vec<(&'static str, Vec<String>)>>` | ‖ cells | 2^16 | `T::read` → `store` |
| `Written` (`tool.rs`) | `path: Vec<String>`, `bytes: Vec<Vec<u8>>`, sorted by path | files < 2^20 | 2^20 | `T::write` → the driver |
| `Report` (`report.rs`) | `subject: Vec<u32>` (into `subjects`), `line: Vec<u32>`, `class: Vec<Class>` (`#[repr(u8)]`), `reason: Vec<&'static str>`, `records: Vec<u32>`, `detail: Vec<Span>`, `text: String`; `subjects: Vec<String>`, `records_in/out: Vec<u32>` | items < 2^20 (today's `NOTES_MAX`); lines < 2^30 → `u32`; records per file < 2^22 → `u32` | 2^20 items, further rows counted | every transform → the two report writers |
| `Mapping` (`map.rs`) | `interop.map` rows, tool word per row | < 2^16 rows | 2^16 | `Mapping::parse` → `plan`, `T::write`, `T::read` |
| tool-side containers | xschem: `XschemFile` (SoA per record kind), planning `Tables`; Virtuoso: `Dump { DCell { Cdf, Cv, Cv, TextView } }`, maestro `Test` | per tool, as today | as today | the tool's `file.rs`/`tables.rs` → its row files |

`owned` is spec §7.1 as data: the record kinds the tool owns for that cell. Virtuoso sets
`var code corner sweep output spec` when a maestro came, else none; xschem sets `code` and
`var` (its `.param` lines). `merge` keeps every unit of a kind the tool does not own. Today
this is `merge::schemify_owned(kind, file_kind, maestro)`, Virtuoso-shaped.

### Owners and the module DAG

```
schemify-format  schemify-doc  schemify-expr  schemify-abi            (external leaves)
        ▲
design.rs   report.rs   pdk/{spice,tech,cdf,limits}.rs                 (leaves: no tool, no job)
        ▲
plan.rs  nets.rs  remap.rs  align.rs  merge.rs  store.rs  map.rs  tool.rs
        ▲                                   ▲
   xschem/ (impl Tool)              virtuoso/ (impl Tool)              (one module per tool)
        ▲                                   ▲
                        job.rs (spines + Step machine + the command grammar)
                          ▲               ▲
                        cli.rs         plugin.rs                       (drivers: bytes only)
```

Rules, enforced by `tests/deps.rs` over the `use crate::` lines (the same shape as Schemify's
`crates/model/cli/tests/deps.rs`):

1. A root file never imports `xschem::` or `virtuoso::`.
2. A tool module never imports `job`, `cli`, `plugin`, or the other tool.
3. Inside a tool module, `write` never imports `read`: both import the tool's `values.rs` tables.
4. `pdk/` imports only external leaves (it reads PDK side files; it knows no tool).

### The shared interface (`tool.rs`)

```rust
pub enum ToolKind { Xschem, Virtuoso }          // closed; matched exhaustively once, in job.rs

pub trait Tool {
    const KIND: ToolKind;                          // the `provenance` word, the report title
    type ReadOpts;                                 // xschem: convert, roots, rcs, models, grid_pm, cdf; Virtuoso: lib_map
    type WriteOpts;                                // xschem: (); Virtuoso: lib_map
    /// What an import still needs, given what the driver has supplied: directories to
    /// list, paths to read, or the complete input set.
    fn inputs(o: &Self::ReadOpts, have: &Files, absent: &BTreeSet<String>, listings: &Listings) -> Need;
    /// Tool inputs → Design, with the per-cell merge data (`Imported`).
    fn read(inputs: &Files, m: &Mapping, o: &Self::ReadOpts, r: &mut Report) -> Result<Imported, Vec<String>>;
    /// Design + Plan → the tool's files.
    fn write(d: &Design, p: &Plan, m: &Mapping, o: &Self::WriteOpts, r: &mut Report) -> Result<Written, Vec<String>>;
    /// The device-table row a bundled core cell crosses on (`How::Core`).
    fn device_row(core: &str) -> Option<u32>;
}

pub enum Need { List(Vec<String>), Read(Vec<String>), Ready(Vec<String>) }
```

The trait is a generic bound only (no `dyn`): `job::export::<T>` and `job::import::<T>` are
written once and instantiated twice. `ToolKind` is matched in one place, `Job::step`, which
picks the instantiation. A tool is a module and a variant; the compiler lists the two arms.
Virtuoso's `inputs` asks for the `.sfyx`, then for the project files its cells target
(masters outside the dump resolve through them); xschem's lists the search path, reads the
`xschemrc`s, then follows references round by round (today's `Gather`, recomputed from the
listings each round; `ponytail:` memoize if a PDK import gets slow).

### Spines (`job.rs`)

One function per pipeline, one line per transform; the branch that picks the tool is in
`Job::step`, the branches that pick *which* transforms run are in the spine, the transforms
are straight passes over tables.

```
export<T>(files):
  mapping   = map::read(files["interop.map"])                              (absent = identity)
  design    = design::load(files, tops)                                    (Missing → Step::Read rounds)
  plan      = plan::resolve::<T>(design, tops, mapping)                    (closure; How per cell; interop.map rows checked)
  edits     = remap::apply(design, nets::extract(design), plan.remastered) (only cells whose master geometry changes)
  written   = T::write(design, plan, mapping, opts, report)
  Done { written + report files, summary }

import<T>(files, listings):
  need      = T::inputs(opts, files, absent, listings)                     (List / Read rounds until Ready)
  imported  = T::read(inputs, mapping, opts, report)                       (Design + stash + owned + headers)
  existing  = store::targets(imported)                                     (one Read round; skipped when nothing is missing)
  files     = store::merge_all(imported, existing, report)                 (merge §7.3, manifest rows, `updated` rows)
  problems  = check::check_output(files)                                   (every written cell loads in doc; both tools)
  Done { files + report files, summary }
```

The summary is one line for every command: `<what>: <n> files, <k> report rows, check: <p>
problems (<report path>)`. What the xschem import's `summarize` printed beyond that (core
coverage, parameters with limits) becomes `info` rows of the report.

`InstallSkill` and `pdk-rules` are not conversions: plain commands beside the spines
(`pdk-rules` stays CLI-only; it walks the file system).

### Per-tool files = the rows of the map

Each file: a header `//! <row>: <tool thing> ↔ <Schemify thing> (spec §…)`, the tool-side
table for the row, `fn to_<tool>(…)` and `fn from_<tool>(…)` as inverses, a round-trip test.
Where today's code is a method of a planning struct, the method moves into the row's file as
another `impl` block of the same struct; its body does not change.

| Map rows | `xschem/` | from today | `virtuoso/` | from today |
|---|---|---|---|---|
| file syntax (lexer, codec, writer records) | `file.rs`, `attrs.rs` | `xschem.rs`, `attr.rs`, the record emitters of `xschem_out.rs` | `file.rs` | `sfyx.rs` |
| the tool's own model (container tables) | `tables.rs` | `translate::Tables`, `Acc` (`translate.rs:301–460`) | `dump.rs` | `read::{Dump, DCell, Cv, …}` (`read.rs:39–440`) + `dump()` (records → tables), `write::records` (tables → records) |
| libraries, paths, provenance, output dirs; cells: which tool files make which cell kind | `cells.rs` | `translate::{name_libs, classify_symbols, plan_cells, plan_variants, scan_overrides}`, `gather.rs`, `xschem_out::{xschemrc, ROOT_DIR, CORE_DIR, target_of}` | `cells.rs` | `map::virtuoso_lib`, `read::{slib_of, targets}`, `ROOT_LIB`, `CORE_LIB`, `write::cell_out` kind rules, the spelling of `Plan` (`lib cell view`) |
| interface: pins, params, formats, inherited pins | `interface.rs` | `symbol.rs`, `translate::plan_symbols`, `template::convert_format`, `limits` hookup, the attribute half of `xschem_out::write_symbol` | `interface.rs` | `write::{cdf_out, param_out, spectre_siminfo, cdl_siminfo, cdf_records}`, the CDF/simInfo half of `read`, `with_fingerprints` |
| symbol graphics, `stext` | `symbol.rs` | the drawing half of `xschem_out::write_symbol`, `translate` rank-2 emit | `symbol.rs` | `write::{symbol_out, stext_out}`, `read` symbol parts |
| schematic objects, orientation, placement | `schematic.rs` | `translate::emit_cell` rank-3 parts, `xschem_out::write_schematic`, `ORIENT_TABLE` | `schematic.rs` | `write` schematic parts, `read` schematic parts (`rejoin_split_wires`, stubs, `conn`) |
| devices ↔ core | `devices.rs` | `core.rs` (`DEVICE_MAP`) | `devices.rs` | `devices.rs` (`DEVICES`, waveform sources) |
| values, expressions, names, units | `values.rs` | `template::{translate_expr, string_value, spice_number}`, `tcl.rs`, the value tables of `xschem_out.rs`, `translate::dotted_bus` | `values.rs` | `ael.rs`, `profile.rs` (coordinates, orientations, net and instance names) |
| simulation setup | `bench.rs` | `translate` code/`.param`/bench-copy parts, `xschem_out::bench_text` | `bench.rs` | `ade.rs` (maestro, OCEAN) |
| the two spines of the tool | `mod.rs` (`impl Tool`) | `translate_library`, `xschem_out::export` | `mod.rs` (`impl Tool`) | `read::import`, `write::export` |
| tool extras | — | | `characterize.rs`, `skill.rs` + `skill/*.il` | unchanged |

Shared, at the root: `design.rs` (load, `Geom`, `SymPins`, `place`), `plan.rs` (`closure`,
`How`, `resolve`), `nets.rs`, `remap.rs`, `align.rs`, `merge.rs`, `store.rs` (targets,
merge_all, manifest, `updated`), `map.rs` (`interop.map`), `report.rs`, `check.rs`,
`tool.rs`, `job.rs` (spines, `Step`, the command grammar), `cli.rs`, `plugin.rs`, and the D14
side-input readers under `pdk/` (`spice.rs`, `tech.rs`, `cdf.rs`, `limits.rs`): tool-neutral
readers of PDK files, consumed by the xschem import today and by `pdk-scan` tomorrow.

### Seams

| Seam | Axis | Rung | Record |
|---|---|---|---|
| which tool | kind: xschem, Virtuoso | 5 generic parameter (`trait Tool` as a bound) | S1 |
| report classes | data: per-tool reason words | 3 table (`&'static str` consts per tool) | S2 |
| device tables | data: per-tool rows | 3 table per tool, one lookup contract | S3 |
| who owns which record kinds | data: per tool, per cell | 3 table (`u64` bitset) | S4 |
| where a cell goes | kind: `How` | 4 closed enum, exhaustive matches in `plan`, `remap`, each `T::write` | S5 |

#### S1: which tool

- **Axis**: kind — xschem, Virtuoso.
- **Evidence**: two cases today (both directions each). Drill D1.
- **Owner**: `tool.rs` (the contract), `job.rs` (the one match).
- **Rung**: 5, a trait used only as a generic bound. Rung 4 (an enum matched in the spine) fails because the spine body would be written per arm or passed function pointers (rung 7); rung 8 (`dyn`) buys nothing, the kind is known when the command is parsed.
- **Cost**: residue none (two instantiations of two cold functions, once per conversion); change radius for a third tool 2 arms in `Job::step`, compiler-listed, plus one new module; interface weight `Tool`, `Need`, `Imported`, `Written`.
- **Drills passed**: D1, D4.
- **Proof**: n/a in the hot path (the spine runs once per job); code size: two monomorphizations, measured at phase 3 against the committed `interop.wasm` (the module budget is 2 MB).
- **Revisit when**: a tool is loaded at runtime (a plugin's plugin) → rung 9, already the editor's boundary.

#### S2: report classes

- **Axis**: data — the `needs_review:<reason>` and `dropped:<reason>` words differ per tool (import.md §7, interop.md §12).
- **Evidence**: two vocabularies today.
- **Owner**: `report.rs` (`Class`, the columns, the writers); each tool's `mod.rs` declares its reason consts.
- **Rung**: 3. Today's rung-4 `template::Reason` enum buys exhaustiveness only for `word()`, which a const already is. Rung 2 fails: the writers would branch on tool.
- **Cost**: residue none; a new reason is one const (0 bodies); weight `Class`.
- **Drills passed**: D3.
- **Revisit when**: a reason needs behaviour (a class that blocks the write) → a `Class` variant.

#### S3: device tables

- **Axis**: data — xschem `devices/*.sym` rows, analogLib rows; columns differ (Virtuoso carries ngspice templates, Spectre simInfo, waveform splits).
- **Evidence**: two tables today.
- **Owner**: each tool's `devices.rs`; `plan` reads them through `T::device_row`.
- **Rung**: 3, one table per tool. One struct for both would carry empty columns for xschem; the shared part is the lookup contract (`device_row`, pins by order, param renames), not the row type.
- **Cost**: residue none; a new device is one row; weight the two row types, each inside its tool.
- **Drills passed**: D1 (a third tool brings its own table).
- **Revisit when**: a third tool's rows equal one of these shapes → share that struct.

#### S4: who owns which record kinds (spec §7.1)

- **Axis**: data — per tool and per cell: Virtuoso owns bench records only when a maestro came; xschem owns `code` and the vars its `.param`s became.
- **Evidence**: today a Virtuoso-shaped function (`merge::schemify_owned(kind, file_kind, maestro)`); the xschem import needs the second case the day it merges (phase 4).
- **Owner**: `tool.rs` (`Imported.owned`), written by each `T::read`, read by `merge`.
- **Rung**: 3, a `u64` bitset per cell. Rung 2 fails: `merge` would branch on tool.
- **Cost**: residue 8 bytes per imported cell; a new owned kind is a bit set in one `read`; weight the bitset.
- **Drills passed**: D2.
- **Revisit when**: ownership becomes per record, not per kind (a user marks one var as Schemify's) → a per-record attribute, already how `vimg` digests work.

#### S5: where a cell goes

- **Axis**: kind — `Unused`, `Own` (written), `Original` (a `provenance <tool> …` cell: referenced, never written), `Core(row)` (a bundled core cell on the tool's device row), `Map(row)` (an `interop.map` row).
- **Evidence**: `map::How` and `xschem_out::Target` are the same five answers spelled twice; Virtuoso's `AnalogLib(k)` is `Core(k)` (decision 1 makes it the only path), its `Core` is `Own` into `schemify_core`; xschem's `Label` is a device-row kind (`gnd.sym`, `vdd.sym`).
- **Owner**: `plan.rs`; each tool's `cells.rs` spells the target (library, cell, view, or `<lib>/<cell>.sym`).
- **Rung**: 4, exhaustive matches (no `_ =>`) in `plan::resolve`, `remap`, and each `T::write`.
- **Cost**: residue none (per cell, not per element); a new variant is compiler-listed at 4 sites; weight `How`.
- **Drills passed**: D4.
- **Revisit when**: a sixth answer appears.

### Change drills

| ID | Change | Source | Edits | Radius | Weight Δ | Verdict |
|---|---|---|---|---|---|---|
| D1 | a third tool (KiCad eeschema or LTspice `.asc`) | user: "an abstract interface shared with xschem" | new `src/<tool>/` with `impl Tool`, `devices.rs`, `values.rs`, row files; `ToolKind` variant; `Job::step` 2 arms; command rows in `job.rs`; README map | 2 arms (compiler-listed) | +1 variant | pass |
| D2 | re-importing an xschem library keeps the project's docs, waivers, bounds and ngspice formats | spec §7.1 (owned data), today lost | phase 4 lands it: xschem `read` fills `owned`; `store::merge_all` does the rest | 0 after phase 4 | 0 | pass |
| D3 | a new mapping row: Virtuoso `config` views ↔ bench `config` (spec §9.3 "not mapped in v1") | spec §9.3, README map "no" | `virtuoso/bench.rs`: one `to_`/`from_` pair + a reason const; one spine line in `virtuoso::read`/`write`; spec §9 row; README row | 2 spine lines | +1 reason | pass |
| D4 | `interop.map` rows for two xschem libraries that name one device differently | spec §8.2 (Virtuoso today) | `Mapping::rows_for(Xschem)`; `plan` already yields `How::Map`; `xschem/cells.rs` spells the target; `remap` runs when geometry differs | 0 bodies (one `rows_for` filter) | 0 | pass |
| D5 | `schemify-interop pdk-scan` for the PDK manager (Schemify-PDK-Manager `docs/remote-pdks.md`) | backlog | `cli.rs`: one subcommand over `pdk/` readers; no tool touched | 0 | 0 | pass |

## Phases

Each phase lands on its own, green, with the oracle that proves it. The order is chosen so
that no phase edits a function body it does not have to: moves first where they are free,
new concepts only where two cases already exist. Sizes: S < 200 lines changed, M < 1,000,
L mostly moves.

- [ ] **0. Truth in headers** (S). `lib.rs` header table and `Cargo.toml` description say what the crate does; README "Code" table. Oracle: none needed.
- [ ] **1. Directions and duplicates that need no new concept** (S).
  `place` moves into `design.rs` (`translate` and `xschem_out` call it there). `ORIENT_TABLE` moves to an `xschem/values.rs` both xschem sides import. One `closure` in a new `plan.rs` (today's `map::closure`; `xschem_out::closure` deleted). `check_output` runs after every import in `job.rs`. Radius: 3 call sites. Oracle: the existing suites (`translate`, `xschem_export`, `virtuoso_rust`, `virtuoso_e2e`, `guest`).
- [ ] **2. One report, one name** (M). `notes.rs` merges into `report.rs` as the SoA table above; `Class` is `#[repr(u8)] { Review, Dropped, Rounding, OffGrid, Remapped, Clamped, Updated, UnknownToken, AltCell, Info }` with a `reason` column; `template::Reason` and `report::Drop` become consts in `xschem/mod.rs` (Virtuoso's in `virtuoso/mod.rs`). One Markdown and one text writer, called from one place in `job.rs` with the command's report base (decision 2); `translate_library` stops writing report files; `summarize` is deleted, its facts become `Info` rows. Spec: import.md §7 and interop.md §12 become one §12 with a reason list per tool; the name rule in both. Docs: `migrating.md`, `cli.md`, `corpus/import_all.sh`, `tests/translate.rs:112`. Radius: every `notes.review`/`Class::Review(Reason::…)` site, mechanical. Oracle: `tests/translate.rs` report goldens (renamed), `tests/ade.rs`, the determinism job (`determinism/run.sh`: reports byte-identical across platforms).
- [ ] **3. The tool seam, the two spines, one grammar** (M). `tool.rs` (`ToolKind`, `Tool`, `Need`, `Imported`, `Written`); `impl Tool for Xschem` in `xschem/mod.rs` wrapping `translate_library` and `xschem_out::export`; `impl Tool for Virtuoso` in `virtuoso/mod.rs` wrapping `read::import` and `write::export`; `job::{export, import}` generic; `Job::step` becomes the one `match`; `Job` keeps `listings` and the xschem `inputs` recomputes its `Gather` from them each round. `job::parse(words) -> Option<Command>` is the one grammar (decision 4): `cli.rs` calls it after its side-input prefix step, `plugin.rs` calls it with the panel's words; the prompts say `-o`. `producer` becomes a const (decision 5). Behaviour otherwise identical. Oracle: `tests/guest.rs` (the built guest writes byte-identically to the native job; add the panel grammar cases), `virtuoso_e2e`, `translate_corpus`. Measure `interop.wasm` size before and after (budget 2 MB).
- [ ] **4. The xschem import through `Design` and `store`** (M, behaviour change: decision 3). `emit_cell` returns `Cell`s (it already builds the `Document`); the xschem `read` sets `owned` = `code | var`; `store::merge_all` merges with the project's files and writes the manifest rows (today `translate::manifest`); `merge` takes the bitset instead of `maestro`. New behaviour: a re-import over an edited project keeps Schemify-only data and reports `updated` rows (D28 shows the diff in the editor). Spec: import.md §1 output paragraph points at interop.md §7 (now tool-neutral: "the tool owns the kinds `Imported.owned` names"). Oracle: the corpus ratchet is unchanged on an empty output directory (byte-identical, `corpus/golden.sh`); new test: import, add a `prose` and a `waive`, re-import → kept; a waiver whose instance is gone → `dropped:stale-schemify-data`.
- [ ] **5. One `Plan`, one device behaviour** (M, behaviour change: decision 1). `How { Unused, Own, Original, Core(u32), Map(u32) }` in `plan.rs`; `plan::resolve::<T>` uses `T::device_row` and `provenance <T::KIND> …`; the xschem export drops `Target`; the Virtuoso `Plan` columns `lib cell view` move to `virtuoso/cells.rs`. Delete `--core-cells`, `--analoglib`, `--no-core`, `ExportOptions::analoglib`, `ImportOptions::{analoglib, lib_dir}`, `Options::map_core`, the analogLib mirror path of the import (`How::Core` is the only path; a core cell without a row is `Own` into `schemify_core`). Spec: §6.1 and §6.5 lose the option sentences; §7.4's oracle says "analogLib". Tests: the round trips run on the one behaviour (byte-identical: no fixture places a stub device); one new test places `ammeter vcvs vccs tline parax_cap` and asserts netlist equivalence (spicecmp) plus the documented stubs. Docs: README, `migrating.md`, `cli.md`. Oracle: `xschem_export`, `virtuoso_rust`, `virtuoso_e2e`, the live kit's `Mode::Alib` becomes the only mode.
- [ ] **6. Files = map rows** (L, moves). Methods of `translate::Tables` move into `xschem/{cells, interface, symbol, schematic, bench}.rs` as further `impl Tables` blocks; `read.rs`/`write.rs` split the same way into `virtuoso/{dump, cells, interface, symbol, schematic, bench}.rs`; `core.rs` → `xschem/devices.rs`; `ael.rs` + `profile.rs` → `virtuoso/values.rs`; `template.rs` + `tcl.rs` → `xschem/values.rs`; `xschem.rs` → `xschem/file.rs`, `attr.rs` → `xschem/attrs.rs`, `sfyx.rs` → `virtuoso/file.rs`; `spice.rs` `tech.rs` `cdf.rs` `limits.rs` → `pdk/`. Each new file gets its `//!` row header and the round-trip test that already exists for it moves beside it. `tests/deps.rs` enforces the DAG. Public paths the tests and the PDK manager use are kept as `pub use` re-exports for one release (`interop::translate_library`, `interop::check_output`, `interop::sfyx`, `interop::design`). Radius: 0 bodies; every `use` line.
- [ ] **7. Docs** (S). README maps cite the file per row ("rules: `virtuoso/interface.rs`"); interop.md §1 (shape, spine table, tables, editor commands) rewritten to this file's § Target; import.md module references; Schemify `docs/dev/architecture.md` interop row; decisions.md amendment below; this file's § Today and § Decisions deleted (the decisions become the amendment).

### Decision amendment (decisions.md, D24/D41)

> 2026-10-24 | D24, D41 | The interop plugin is organized by the mapping (user decision): one
> `Design` in the middle; one module per tool whose files are the rows of that tool's
> user-facing map, each file holding the row's tool-side table and its two inverse
> transforms; the shared pipeline (`load`, `plan`, `nets`, `remap`, `store`/`merge`,
> `report`, `job`) names no tool. The tool seam is `trait Tool` as a generic bound (rung 5,
> no `dyn`), matched once on a closed `ToolKind`. One behaviour per direction, no option:
> core devices cross on analogLib and `devices/*.sym`, and come back as `core:` cells
> (`--core-cells`, `--analoglib`, `--no-core` removed; `schemify_core` holds only core cells
> without an analogLib row). The xschem import merges with the project's files like the
> Virtuoso import (spec §7 is tool-neutral; who owns which record kinds is data per cell).
> One report type, one spec section, one name rule (`<stem>.report.*` beside a file output,
> `interop.report.*` inside a directory output); one command grammar for the CLI and the
> editor.

## Invariants

- Every table of a conversion has one writer: `design::load`/`T::read` write `Design`; `plan::resolve` writes `Plan`; `remap::apply` writes `Edits`; `store` writes files. A tool's `write` reads `Design`, `Plan`, `Edits`; it writes only its `Written`.
- A root file names no tool; a tool names no driver and no other tool (`tests/deps.rs`).
- Count conservation per input: records in = records out + records reported, asserted by every `T::read` (import.md §7, interop.md §12).
- `import(export(x)) ≡ x` up to the documented losses, per tool, per fixture (`virtuoso_rust`, `xschem_export`); the built guest equals the native job byte for byte (`guest`).
- Determinism: the same inputs give byte-identical files and reports on every platform (`determinism/run.sh`, CI).

## Adding a feature

- A new row of a map (something that now crosses): the row's file in each tool that has it gets a `to_`/`from_` pair and a reason const; one spine line in the tool's `read`/`write`; a row in the spec and in the README map.
- A new tool: `src/<tool>/` with `impl Tool`, its `devices.rs` and `values.rs`, its row files; a `ToolKind` variant (the compiler lists the two arms); command rows; a README map.
- A new shared step (a check, a normalization): a transform at the root and one spine line in `job.rs`.
- A new PDK side input: a reader in `pdk/`, consumed by the row that needs it (`interface.rs` for limits).
- A new option: don't. Two users needing different answers today is the bar; then it is a row of data (`--lib L=V`), never a branch.
