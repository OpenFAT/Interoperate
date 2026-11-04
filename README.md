# interop: Schemify ⇄ Cadence Virtuoso and xschem

Use Schemify as your schematic editor while your team stays in Cadence Virtuoso, and move
designs to and from xschem. A [Schemify](https://github.com/OpenFAT/Schemify) plugin, never
part of the core (D24 as amended, D41). Normative spec: [interop.md](https://github.com/OpenFAT/Schemify/blob/main/docs/spec/interop.md); xschem
import: [import.md](https://github.com/OpenFAT/Schemify/blob/main/docs/spec/import.md).

Two interfaces: the **plugin** (`interop.wasm`, loaded by the editor) and, secondary, the
**native CLI** `schemify-interop` for the same conversions without the editor.

| Direction | What crosses |
|---|---|
| Virtuoso → Schemify | cells (symbol, schematic, CDF parameters, Verilog-A/Verilog views), PDK libraries (symbols, CDF, simInfo → netlist templates), maestro setups (variables, analyses, corners, outputs, specs) |
| Schemify → Virtuoso | the same, plus gm/Id sizing intent that is solved on the target PDK (PDK-agnostic flow) |
| xschem → Schemify | designs and PDK device libraries (`schemify-interop xschem-import`) |
| Schemify → xschem | `.sch`/`.sym`, core devices onto xschem's `devices/` symbols |

Nothing Cadence-made is linked or shipped. Virtuoso runs one SKILL file, `schemify.il`, that
dumps and loads a plain data file (`.sfyx`). All translation happens in Schemify, in tested
Rust code. The SKILL side only moves data, and its data is never evaluated (`lineread`, D7).

## Install

For yourself, in every project: add it to your Schemify config (`init.lua`) and run
`schemify pack sync`:

```lua
schemify.pack.add {
  { src = "https://github.com/OpenFAT/Interoperate", name = "interop",
    grant = { "read_project", "write_project" }, fuel = 100000000000 },
}
```

Pin a release with `version = "v0.1.0"`. For one project (so everyone who opens it gets the
plugin): copy `interop.wasm` into the project and add it to the project manifest:

```
plugin interop path plugins/interop.wasm
  grant read_project
  grant write_project
  fuel 100000000000
```

`fuel` is the per-event instruction budget (plugin-abi §2). The default, 10^8, is too small for
a conversion of a real design (a Virtuoso export of a small project with a PDK needed about
10^11), so give the ceiling, 10^11: with more fuel than the default the plugin runs on a worker
thread and the editor does not freeze while it converts. A conversion that runs out says so in
the status line (`interop: command interop.virtuoso_export: out of fuel (budget N), …`).

In the editor the File menu gains **Export to Virtuoso (.sfyx)…**, **Import from Virtuoso
(.sfyx)…**, **Export to xschem…** and **Write Virtuoso SKILL bridge (schemify.il)**. Without
the editor, the same conversions run from the command line:

```
cargo install --git https://github.com/OpenFAT/Interoperate --features cli   # or a release binary, or `nix run github:OpenFAT/Interoperate --`

schemify-interop virtuoso-export <project> <cell-file|dir>... -o <file.sfyx> [--lib L=V]... [--core-cells]
schemify-interop virtuoso-import <project> <file.sfyx> [--lib L=V]... [--analoglib]
schemify-interop xschem-export   <project> <cell-file|dir>... -o <dir>
schemify-interop xschem-import   <lib-dir>... -o <out-dir> [...]        (spec import.md)
schemify-interop install-skill   <dir>
schemify-interop pdk-derived     <project> <char.sfyx>                   (derived parameters, below)
schemify-interop virtuoso-refresh <project> --scratch <lib> -o <dir> | --apply <dir>
schemify-interop pdk-scan        --root <dir> --entry <file> --dialect spice|hspice|spectre [--section S]   (PDK scan below)
```

Paths after `<project>` are relative to it; a directory stands for every cell file under it. Each conversion writes a report next to its output
(`<name>.report.md`, `.report.txt`): everything that did not cross one to one, with a reason.

## First time with Virtuoso

The quickest check is the live test kit: run [`live/run.sh`](live/README.md) from your
Virtuoso project directory and read the verdict in `sfylive/out/report.md`. It does everything
below and more, in batch mode. To do it by hand instead:

The first session settles what this plugin could only guess about your Virtuoso release
(spec §13). Run these steps in order, in one Virtuoso session; every step writes a file, and
the files are what to send back.

**Before you start**

- Copy `schemify.il` and `sfy_probe.il` (written by `schemify-interop install-skill interop/`
  or the File menu item) to the machine that runs Virtuoso. `/tmp` below is on that machine.
- Create an empty library for scratch work in the Library Manager (`scratchLib` below; no
  technology needed, or attach the PDK's). It must be writable, not under design management
  (SOS/DesignSync), and never the library holding your designs.
- Close every schematic and ADE/maestro window of the cells you will use. An import refuses a
  cellview open for editing, and a maestro view open in a window keeps a lock.
- Open one schematic for editing and one maestro view by hand, then close them. That clears
  any "use next license level?" or PDK first-use dialog now, not halfway through a step. Answer
  "Always" if asked.

**Steps**

1. In the CIW: `load("/absolute/path/schemify.il")`, once per session (not also from
   `.cdsinit` this time). Check with `isCallable('SfySelfTest)`, which should print `t`.
2. `load("/absolute/path/sfy_probe.il")`, then
   `SfyProbe("scratchLib" ?pdkLib "yourPdk" ?pdkCell "nch")` (any PDK transistor; leave out
   the two keywords if there is no PDK). It writes `/tmp/sfy_probe.txt`, flushed section by
   section: the release, any function it lacks, and how it prints, orients, stores CDF data and
   answers the maestro calls. It touches only `scratchLib/sfy_probe` (a purged schematic and a
   saved maestro view). The `pdk` lines name your PDK's parameters: remove what you cannot
   share.
3. `SfySelfTest("myLib" "someCell" "scratchLib")`, once on a cell with PDK transistors and once
   on a testbench cell that has a maestro view. Each run dumps the cell, loads it into the
   scratch library, dumps it again and compares the two dumps. It prints `PASS` or the first
   lines that differ, and writes the same to `/tmp/sfy_selftest.txt`. The logs are
   `/tmp/sfy_selftest_a.sfyx.log` and `_b.sfyx.log`.
4. To try PDK callbacks, load that dump with callbacks on:
   `SfyImport("/tmp/sfy_selftest_a.sfyx" ?lib "scratchLib" ?overwrite t ?callbacks t)`, then
   look for `callback failed`, `clamped` and `derived` lines in its `.log`.

Every log starts with the Virtuoso version and the functions this release cannot call. It is
flushed line by line, so a run that stops on a dialog still leaves everything before it. The
CIW shows a `schemify: loading …` line before each cellview, so `CDS.log` shows where a run
stopped.

**Send back:** `/tmp/sfy_probe.txt`, `/tmp/sfy_selftest.txt`, `/tmp/sfy_selftest_*.sfyx` and
their `.log` files, every other `.sfyx.log` you produced, and `~/CDS.log`
(or the newest `CDS.log.N`).

## Workflows

### Bring a PDK into Schemify (mirror)

In Virtuoso: `SfyExportLib("/tmp/demopdk.sfyx" "demopdk")`. In Schemify:
`schemify-interop virtuoso-import <project> demopdk.sfyx`. You get a library `demopdk/` with
one `.shprim` per device: the same symbol, pins and CDF parameters (as Schemify parameters
with units and choices), and Spectre and CDL templates built from simInfo. Designs that place
these cells export back to the real PDK cells by name, so geometry and parameters match exactly.

To simulate with ngspice in Schemify, give each mirror device a `format spice` template, or add
`spice "…"` to its row in `interop.map`. Formats you add survive later re-imports. A commercial
PDK's mirror holds its symbols and parameter names: keep that library out of public repositories.

### Derived parameters (CDF callbacks)

A PDK's CDF callbacks derive netlisted parameters (`ad`, `pd`, `nrd`, …) from the ones you
set. Only Virtuoso runs them, so Schemify never computes these values: it keeps Virtuoso's.

1. Find which parameters are derived, once per PDK: in Virtuoso,
   `SfyCharacterize("/tmp/char.sfyx" "demopdk" list("nmos_demo" "pmos_demo") "scratchLib")`
   (a few probes per device on a throwaway schematic; the file names the PDK's parameters,
   so it stays on your machine). Then `schemify-interop pdk-derived <project> char.sfyx`
   marks them on the mirror cells (`vderived`).
2. Each instance then carries Virtuoso's values (`cached`) from the import. Resize it in
   Schemify and they are stale: Run is blocked (`E039`) and an export leaves them to the
   callbacks (`SfyImport ?callbacks t`), whose values the next import brings back.
3. To refresh them without sending the design: `schemify-interop virtuoso-refresh
   <project> --scratch scratchLib -o refresh/` writes `refresh/refresh.il`; load it in
   Virtuoso after `schemify.il` (it measures the stale instances' sizes on a scratch
   schematic and touches no design), then `schemify-interop virtuoso-refresh <project>
   --apply refresh/`.

### Bring a design from Virtuoso

`SfyExport("/tmp/amp.sfyx" "myLib" list("amp" "amp_tb"))` dumps the cells. The default
`?ref t` adds every master they use from other libraries, such as PDK and analogLib devices.
`?maestro t`, also the default, adds the testbenches' maestro setups. Then run
`schemify-interop virtuoso-import <project> amp.sfyx`.

Re-importing later merges with your files. Virtuoso owns the drawing, the instance parameters,
the CDF parameters and the maestro setup. Schemify keeps what Virtuoso has no field for:

- docs and variables,
- parameter bounds,
- waivers and constraints,
- ngspice formats,
- configs, sweeps, Monte Carlo and optimizer records.

A waiver whose instance is gone is dropped, and the report says so.

### Send your edits to Virtuoso

`schemify-interop virtuoso-export <project> lib/amp.shcomp -o amp.sfyx`, then in Virtuoso:
`SfyImport("/abs/amp.sfyx" ?overwrite t)`. Add `?callbacks t` to run the PDK's CDF callbacks
after setting parameters, as the Edit Properties form does. The loader then logs every value
the PDK clamped.

Each schematic is checked with `schCheck` and saved only when the check is clean.
`<file>.sfyx.log` lists every created object, every refusal and the result (`PASS`/`FAIL`).

Cellviews you did not change in Schemify since they came from Virtuoso are left exactly as
Virtuoso drew them, including fonts, label justification and instance-label styles. Only what
you edited is redrawn. Object identity rides on `schemifyId` properties, so round trips give
clean Schemify diffs.

### Testbenches and ADE (maestro)

A `.shbench` exports as the testbench schematic plus a maestro view with one test (Spectre):

| Schemify bench | Maestro |
|---|---|
| `var` | design variable (values in Spectre syntax: `1meg` → `1M`) |
| a `sweep` named after its var | the variable's comma list of points |
| `corner` (`lib`, `include`, `set`, `temp`) | corner with model files and sections, corner variables, `temperature` |
| `.tran .ac .dc .op .noise` lines of the analysis code block | analyses with their options |
| `.options errpreset=conservative` | the tran analysis's `errpreset` |
| `.temp`, `.param`, `.include`, `.lib` | variables and test model files |
| `code … spectre` blocks | definition files |
| `output` (`v()`, `i()`, `at`, `re`/`im`, `x()`, `deriv`, `max`, `cross`, `settle`, `phase_margin`, `op("M1","gm")`, …) | outputs (`VS("/net")`, `IF("/V1/PLUS")`, `value`, `real`/`imag`, `xval`, `cross`, `settlingTime`, `phaseMargin`, `OP("/M1" "gm")`, …); attribute `analysis op` picks the dataset per output |
| `spec` `min`/`max` | spec `>`/`<` (bounds over bench vars folded to numbers) |
| core sources, passives, ammeters | their analogLib cells (`vdc`, `idc`, `cap`, `iprobe`, …; `--core-cells` keeps the `schemify_core` mirrors); `dc {VGS} ac 1` → `vdc` `VGS`, `acm` `1` |

Each bench also gets an OCEAN script, `ocean/<cell>.ocn` beside the `.sfyx`, that runs the same
setup headless and prints every output to `<cell>/<output>.txt` (sweeps as `paramAnalysis`; an
output with `vs <other output>` is also printed as a `waveVsWave` plot):

```
SfyImport("/abs/ex.sfyx")                      ; in the CIW
cd ocean && ocean -nograph < dc_vgs.ocn         ; in a shell
```

Importing goes the other way. A comma list comes back as a `lin`/`log`/`values` sweep, and the
analyses come back as one code block named after the test. Monte Carlo and optimizer records,
configs, and anything without a maestro form stay on the Schemify side; the report lists them.

### Different libraries for the same devices (`interop.map`)

When the Schemify library and the Virtuoso library name the same devices differently (an
xschem-imported sky130 against a Virtuoso sky130 kit), write `interop.map` at the project
root. It maps cell, pins and parameters, including value scaling. Such instances are
re-mastered: their connections ride on labels, so no net changes even though the two symbols
differ in shape (spec §6.4, §8.2).

### xschem

`schemify-interop xschem-export <project> top.shbench -o xs/` writes `xs/<lib>/<cell>.sym|.sch`
and an `xschemrc` fragment. Core devices land on xschem's own `devices/*.sym`, and cells first
imported from xschem point back at their original symbols. The test suite netlists the export
with a real xschem and compares it with Schemify's netlist.

### PDK scan (remote PDKs)

`schemify-interop pdk-scan` prints a PDK's device interface as Schemify cells, for the
PDK-Manager plugin to serve as a library without anything from the kit landing on disk. It
runs next to the kit (`ssh host schemify-interop pdk-scan --root /pdk/kit --entry
models/top.lib --dialect hspice --section tt`) and reads only under `--root`. Each top-level
`.subckt` becomes a cell with its ports in order; each MOS `.model` (bins collapsed) one with
`d g s b`. The cells have `format spice` and `format spectre` and parameter names only:
no defaults, bins or card values (a parameter is written only when an instance sets it).
Other model types are counted in an `issue` line. `--protocol` prints `pdkscan 1`.

Without the binary on the kit host, `pdk-scan --remote-script` prints a POSIX `sh` script
that frames only the structural lines of the files it is given (`.model name type`,
`.subckt` ports and parameter names, sections, includes). Pipe its output into a local
`pdk-scan --stdin --root <dir> --entry <file> …`: same cells, plus a `need <path>` line per
include to fetch next. The output format is in `src/pdk_scan.rs`.

Spectre kits (`--dialect spectre`) are read with [SpiceRack](https://github.com/OpenFAT/SpiceRack),
the PDK store: every top-level `subckt` the entry reaches through `include` is a cell, names
in their own case, and top-level compact models (BSIM, PSP, HiSIM, UTSOI) get `d g s b`.
Lines SpiceRack does not read are counted, never shown. Includes inside a corner `section`
are followed once SpiceRack holds sections as data (an `issue` line counts them until then).

## What does not cross

Every conversion lists its losses in the report. The main ones:

- Virtuoso config views (hierarchy editor), and maestro Monte Carlo and optimizer setups. They
  are kept on the Schemify side.
- PDK CDF callbacks inside Schemify. Derived parameters (`ad`, `as`, finger widths) stay
  Virtuoso's: see "Derived parameters" above.
- Cosmetic Virtuoso fields on cellviews you redraw: label fonts, justification,
  `cdsName()` versus `[@instanceName]` labels.
- xschem: non-default corners, sweeps, outputs, specs, optimizer records, waivers.

## How this is tested without Virtuoso

No Cadence tool runs here, so:

- **The SKILL subset.** The SKILL is written in plain prefix form `(f a b)`, which Virtuoso
  reads everywhere. A test-only interpreter (`tests/skill/`) executes it exactly and refuses
  anything outside the subset: C-style calls, infix operators, scale-suffixed or zero-padded
  numbers.
- **The mock Virtuoso.** It implements every Cadence function the scripts call, from the
  manuals' signatures (arity and types checked). A lint fails on any call it does not know.
- **Both sides together** (`tests/virtuoso_e2e.rs`):
  - A Rust export loaded by the shipped SKILL, dumped by it, and imported by Rust is
    byte-identical.
  - A Virtuoso-made dump imported, exported and reloaded into the original library dumps
    identically.
- **The gm/Id solver** is written in both SKILL and Rust, and the two agree bit for bit on
  random vectors. The Rust one is within 2 % of an analytic device.
- **xschem export** is checked against real xschem netlists.
- **What cannot be proven here** is listed as numbered assumptions in spec §13. `SfySelfTest`
  and the probe settle them on the first real run.

## Code

| File | Does |
|---|---|
| `src/design.rs`, `src/nets.rs`, `src/profile.rs` | Schemify files as tables, connectivity, coordinate/orientation/name rules |
| `src/sfyx.rs` | the exchange-file codec |
| `src/virtuoso/` | `write/` (export), `read/` (import), `kinds` (the record kinds), `ael` (values), `devices` (analogLib table), `ade/` (maestro) |
| `src/merge.rs`, `src/remap.rs`, `src/map.rs` | keeping Schemify-only data, re-mastering, `interop.map` |
| `src/xschem_out.rs`, `src/translate.rs` … | xschem export and import |
| `src/job.rs` | one conversion as pure steps, driven by `src/plugin.rs` (editor) and `src/cli/` (one module per subcommand) |
| `skill/*.il` | the parts of `schemify.il` |
| `tests/` | `skill/` (the SKILL interpreter and `mockdb/`, the mock Virtuoso), `virtuoso_rust`, `virtuoso_skill`, `skill_lint`, `virtuoso_e2e`, `ade`, `exercise1` (a course's benches: maestro + OCEAN goldens), `xschem_export`, and the xschem import suites |

## Development

`nix develop` has the pinned Rust 1.95.0 (rust-overlay, with rust-lld), xschem, ngspice and wasmtime. The Schemify crates
are git dependencies pinned to one core commit in `Cargo.toml`; tests read copies of the core
fixtures in `tests/fixtures/schemify/`.

| Command | Does |
|---|---|
| `cargo test --features cli` | every native test; the corpus ones skip without `SCHEMIFY_CORPUS` |
| `./build.sh` | rebuilds `interop.wasm` (reproducible; commit it with the change) |
| `SCHEMIFY_REQUIRE_XSCHEM=1 cargo test --features cli --test guest --test xschem_export` | the built guest through Schemify's host, the export netlisted by real xschem |
| `corpus/fetch.sh && corpus/golden.sh` then `SCHEMIFY_CORPUS=corpus/.cache cargo test --release --test xschem_differential -- --nocapture` | the xschem differential ([corpus/README.md](corpus/README.md)) |
| `NETLIST=none corpus/import_all.sh DIR && corpus/roundtrip.sh DIR` | the corpus through xschem → Schemify → xschem, re-netlisted by real xschem and compared with the goldens (checks the export) |
| `determinism/run.sh <schemify-interop command> <out>` | the import every platform must write byte for byte |
| `cd fuzz && cargo +nightly fuzz run xschem_read` | the fuzzers (`fuzz/seeds.sh` seeds them) |

CI (`.github/workflows/`): `ci.yml` runs the suite on Linux, macOS and Windows, compares the
import across them and WASI, rebuilds the guest and fails when the committed module differs;
`nightly.yml` runs the corpus ratchet, the fuzzers, and the suite against Schemify `main`;
`release.yml` builds the CLI for every platform into a draft release on a `v*` tag.

### Parallel lanes

Interop's remaining Cadence work runs in parallel lanes, each owning disjoint files. Every
registration point already lists each lane's slot, so a lane adds code to its own files and
edits no shared list:

| Lane | Owns |
|---|---|
| L1 Maestro XML dump/load | `src/virtuoso/ade/import.rs`, `skill/sfy_ade.il`, `tests/skill/mockdb/mae.rs` |
| L2 label cosmetics | `src/virtuoso/read/labels.rs`, `src/virtuoso/write/labels.rs`, the label parts of `skill/sfy_cells.il` |
| L3 config views | `src/virtuoso/config.rs`, `skill/sfy_cfg.il`, `tests/skill/mockdb/hdb.rs` |
| L4 results to waves | `src/virtuoso/results.rs`, `skill/sfy_results.il`, `src/cli/maestro_results.rs` |
| L5 Monte Carlo and optimizer | `src/virtuoso/ade/runmode.rs` |
| L6 multi-test and run plans | `src/virtuoso/ade/export.rs` |
| L7 ADE L | `skill/sfy_adel.il`, `tests/skill/mockdb/asi.rs` |
| L8 layout through KLayout | `src/klayout.rs`, `skill/sfy_layout.il`, `src/cli/layout.rs` |
| L9 design-library CDF callbacks | `src/virtuoso/read/cdf.rs`, `skill/sfy_cdfcb.il` |
| L10 the PDK side | `src/pdk_models.rs`, `src/pdk_scan.rs` |

Each of L1 to L9 also owns its live check, `live/checks/l<N>_*.il` (`live/checks/run.sh`).
The registration points: a new `.sfyx` record kind is a row in its lane's section of
`src/virtuoso/kinds.rs`; mock Cadence functions are rows in the area's `ROWS` and bodies in
its `call` (`tests/skill/mockdb/`); the SKILL parts are already in `src/skill.rs`'s `PARTS`
(load order, `schemify.il` and the lint), and the CLI subcommands in `src/cli/mod.rs`.

Rules for every lane:

- Never commit `interop.wasm` or `Cargo.lock`: the integration rebuilds both.
- The spec text (`docs/spec/interop.md`, in the Schemify repository) and "What does not
  cross" above are edited only at integration.
- Build with a `CARGO_TARGET_DIR` of the lane's own.
- The Cadence probe fixtures in `tests/fixtures/cadence/` are local-only (listed in
  `.git/info/exclude`): a test that reads them gets its path from `common::cadence_fixture`
  and returns when it gets `None`, so it skips where they are absent.

To move to a newer Schemify: set every `rev` in `Cargo.toml` to the new commit,
`cargo update -p schemify-abi` (and the other Schemify crates), `./build.sh`, test, and commit
`Cargo.toml`, `Cargo.lock` and `interop.wasm` together.
