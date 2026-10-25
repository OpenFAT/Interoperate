# Architecture: the mapping is the code

The README's two maps (*Virtuoso ↔ Schemify*, *xschem ↔ Schemify*) are the layout. Each
tool is a directory; each direction (`read` = tool → Schemify, `write` = Schemify → tool) is
a spine file whose child modules are the rows of that tool's map. Spines run once per
conversion, rows are straight passes over the tool's tables. Normative behaviour stays in
Schemify's [interop.md](https://github.com/OmarSiwy/Schemify/blob/main/docs/spec/interop.md)
and [import.md](https://github.com/OmarSiwy/Schemify/blob/main/docs/spec/import.md).

## Layout

```
src/
  lib.rs  job.rs  cli.rs  plugin.rs      the spine of a conversion and its two drivers
  design.rs  notes.rs  check.rs  decimal.rs
                                         shared leaves: the Schemify-shaped Design, the
                                         report, the output oracle, exact decimals
  pdk/      spice tech cdf               D14 side inputs (model cards, tech grids, cdfDump)
  xschem/   file attr                    .sch/.sym record syntax, attribute strings
            read  read/{cells,interface,emit}
            write write/{symbol,schematic,values}
            symbol template tcl devices gather limits report
  virtuoso/ sfyx dump                    exchange-file codec, the dump model (both ways)
            read  read/{cells,interface,symbol,schematic}
            write write/{interface,symbol,schematic}
            ael profile devices ade map plan nets remap align merge characterize skill
```

| Map row | xschem | Virtuoso |
|---|---|---|
| file syntax, the tool's own model | `file`, `attr` | `sfyx`, `dump` |
| which tool files make which cells; libraries | `read/cells`, `gather` | `read/cells`, `plan`, `map` (`interop.map`) |
| interface: pins, params, formats | `read/interface`, `symbol`, `write/symbol` | `read/interface`, `write/interface` |
| symbol graphics and labels | `read/emit`, `write/symbol` | `read/symbol`, `write/symbol` |
| schematic objects, placement | `read/emit`, `write/schematic`, `file::ORIENT_TABLE` | `read/schematic`, `write/schematic`, `profile`, `nets`, `remap`, `align` |
| devices ↔ `core:` | `devices` | `devices` |
| values, expressions, names | `template`, `tcl`, `write/values` | `ael`, `profile` |
| simulation setup | `read/emit` (code, `.param` → vars), `write/schematic` | `ade` (maestro, OCEAN) |
| keeping Schemify-only data on re-import | — (see Decisions) | `merge`, fingerprints in `mod` |
| PDK limits and callbacks | `limits` (from `pdk/`) | `characterize` |
| accounting | `report` (count conservation), rows into `notes` | `notes` |

## Module DAG

```
schemify-format  schemify-doc  schemify-expr  schemify-abi
        ▲
design  notes  check  decimal  pdk              names no tool, no driver
        ▲
xschem/            virtuoso/                    never name each other or a driver
        ▲               ▲
job (spines, the command grammar)
        ▲
cli  plugin                                     bytes only
```

Enforced by `tests/deps.rs` over `use crate::` lines. Inside a tool, `write` imports `read`
only where the spec makes the export run the import (Virtuoso's stash image, §7.3).

## Spines

- `job::Job::step`: one arm per command; both exports load through `Job::design`; every
  conversion ends in `finish` (the report beside a file output as `<stem>.report.{md,txt}`,
  inside a directory output as `interop.report.{md,txt}`).
- `job::parse`: the one command grammar (`job::USAGE`). The CLI adds the project directory
  and the D14 side-input flags; the editor panel maps `interop.virtuoso_export` to
  `virtuoso-export`.
- `xschem::read::translate_library`: read → classify → plan symbols → cells → libraries →
  emit → outputs (+ the `report`, asserted to conserve records per input file).
- `xschem::write::export`: closure → target per cell → symbol and schematic per cell →
  `xschemrc`.
- `virtuoso::write::export`: `plan::resolve` → per cell CDF, symbol, schematic (with
  `remap` edits) → `ade` per bench → image import → `merge::stash` → `dump::records`.
- `virtuoso::read::import`: `sfyx::read` → `dump::dump` → interfaces → one fresh document
  per target → `ade::import` → `merge::merge` with the local file or the stash.

## Decisions

Rule: one behaviour, no knob, unless something today needs the second answer. Each was
measured before it was taken.

| Decision | Outcome | Evidence |
|---|---|---|
| One written report | done: `notes::Notes` for every command; the xschem importer's `Report` is its private count-conservation bookkeeping, converted by `Report::to_notes` | 210 call sites unchanged; corpus identical but the renamed report |
| One command grammar | done: `job::parse` | two parsers and an inferred output path deleted |
| One producer string | done: `virtuoso::PRODUCER` | an option nothing read |
| Per-tool directories, rows as files | done | moves only; corpus import byte-identical, suite green |
| A `trait Tool` over both tools | **not taken** | the imports share no shape (directory rounds vs one file); every method would have one caller (`Job::step`). The shared interface is the tables (`Design`, `Notes`) and the same spine/row layout per tool |
| Delete `--core-cells`, `--analoglib`, `--no-core` | **not taken** | forcing analogLib broke 7 tests: the core-library and generated-project round trips (spec §7.4), OCEAN goldens, the mock live kit. Re-mastering changes pin geometry; the round-trip oracle needs the `schemify_core` path and users want analogLib |
| xschem import merges with existing files | **not taken** | `merge` keeps Virtuoso's "Schemify-only" attributes (`format spice`, `min max grid`, `ignore`), which the xschem import *produces*: a re-import would keep stale formats and limits. Revisit when someone re-imports xschem libraries over an edited project; then ownership becomes a per-tool attribute table |
| `check_output` after every import | **not taken** | it checks a self-contained output tree; a Virtuoso import writes into an existing project, covered by its round-trip oracles |

## Oracles

- Suite: `cargo test --features cli` (`tests/guest.rs` needs a core revision with
  `IoOp::FsList`).
- The xschem corpus import is byte-identical across refactors: `corpus/import_all.sh`
  before and after, `diff -r`.
- `import(export(x)) ≡ x` per tool and fixture (`virtuoso_rust`, `virtuoso_e2e`,
  `xschem_export`); xschem's goldens (`xschem_differential`, nightly).
- Determinism: `determinism/run.sh`.

## Adding a feature

- Something new crosses (a map row): the row file of each direction that has it, a report
  reason for what does not, a row in the spec and in the README map.
- A new tool: `src/<tool>/` with `read.rs`/`write.rs` spines and row children, its
  `devices.rs`, a `job::Command` variant and a `job::parse` arm, a README map, a `RULES`
  row in `tests/deps.rs`.
- A new PDK side input: a reader in `pdk/`, consumed by the row that needs it.
- A new option: only when two users need different answers today; then it is data
  (`--lib L=V`, an `interop.map` row), not a branch.
