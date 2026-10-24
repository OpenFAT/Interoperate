# interop: Schemify ⇄ Cadence Virtuoso and xschem

Use Schemify as your schematic editor while your team stays in Cadence Virtuoso, and move
designs to and from xschem. A [Schemify](https://github.com/OmarSiwy/Schemify) plugin, never
part of the core (D24 as amended, D41). Normative spec: [interop.md](https://github.com/OmarSiwy/Schemify/blob/main/docs/spec/interop.md); xschem
import: [import.md](https://github.com/OmarSiwy/Schemify/blob/main/docs/spec/import.md).

Two interfaces: the **plugin** (`interop.wasm`, loaded by the editor) and, secondary, the
**native CLI** `schemify-interop` for the same conversions without the editor.

| Direction | What crosses |
|---|---|
| Virtuoso → Schemify | cells (symbol, schematic, CDF parameters, Verilog-A/Verilog views), PDK libraries (symbols, CDF, simInfo → netlist templates), maestro setups (variables, analyses, corners, outputs, specs) |
| Schemify → Virtuoso | the same |
| xschem → Schemify | designs and PDK device libraries (`schemify-interop xschem-import`) |
| Schemify → xschem | `.sch`/`.sym`, core devices onto xschem's `devices/` symbols |

Nothing Cadence-made is linked or shipped. Virtuoso runs one SKILL file, `schemify.il`, that
dumps and loads a plain data file (`.sfyx`). All translation happens in Schemify, in tested
Rust code. The SKILL side only moves data, and its data is never evaluated (`lineread`, D7).

## Virtuoso ↔ Schemify map

Where each part of Virtuoso lives in Schemify, and whether the plugin carries it across.
**Yes**: both ways. **Kept**: Schemify-only, survives a round trip but Virtuoso never sees it.
**No**: neither side's version crosses. Exact rules: spec §6 (cells), §9 (ADE).

**Design data**

| Virtuoso | Schemify | Crosses |
|---|---|---|
| library (`cds.lib`) | `lib` line in `schemify.project`; project root ↔ `schemify`, `core` ↔ `schemify_core`, rename with `--lib L=V` | yes |
| cell with symbol + schematic | `.shcomp` | yes |
| device cell (symbol, no schematic), PDK cell | `.shprim` (mirror, `provenance virtuoso <lib>/<cell>`) | yes |
| testbench cell / cell with a `maestro` view | `.shbench` | yes |
| `symbol` view | the cell's symbol (`i` to open) | yes |
| `schematic` view | the cell's schematic | yes |
| `veriloga`, `verilog`/`functional` views | impl `veriloga`, `verilog` | yes |
| CDF parameters | `param` (kinds and units: spec §6.3) | yes |
| CDF `prompt` | param `doc` | yes |
| CDF callbacks | not run; `computed`/`valid` rules from `SfyCharacterize`, or `SfyImport ?callbacks t` | no |
| simInfo `spectre` / `auCdl` | impl `format spectre` / `format cdl` | yes |
| (none) | `format spice`/`xyce`, param `min max step grid`, `prose` docs, `waive` | kept |
| analogLib (`res cap vdc vpulse nmos4 iprobe …`) | `core:` devices (spec §6.5) | yes |
| `basic/ipin opin iopin` | `port` | yes |
| `basic/noConn` | `noconn` | yes |
| `gnd!`, `vdd!` and other global symbols | global `label` | yes |
| `netSet` / inherited connections `[@p:%:d!]` | `inherit` | yes |
| Hierarchy Editor `config` view | bench `config` | no |
| Constraint Manager | `constraint` records | no (Schemify's kept) |
| Check and Save (`schCheck`) | live checks, `schemify check` | runs on load |
| OSS / CDL netlisting | `n`, `schemify netlist --dialect spectre\|cdl` | not needed |
| two PDKs naming one device differently | `interop.map` | yes |

**ADE (Explorer, Assembler, maestro)**

| Virtuoso | Schemify | Crosses |
|---|---|---|
| maestro view, one test | one `.shbench` | yes |
| more tests in one view | one bench per test | first test only |
| design variables | `var` | yes |
| parametric sweep (comma list) | `sweep` (`values`/`lin`/`log`) | yes |
| corners (model files, sections, vars, temperature) | `corner` (`lib`, `include`, `set`, `temp`) | yes |
| analyses (tran, ac, dc, noise, saved op point) | `.tran .ac .dc .noise .op` in the analysis `code` block | yes |
| test model files | `.lib`/`.include` | yes |
| definition files (`.scs`) | `code … spectre` | yes |
| outputs (calculator) | `output` (spec §9.1 table) | yes |
| specs `>` `<` | `spec … min max` | yes |
| Monte Carlo | `montecarlo` + var `dist` | no (Schemify's kept) |
| optimization (GXL) | `optimize` | no (Schemify's kept) |
| OCEAN script | generated `ocean/<cell>.ocn`; `schemify run` | export only |
| ADE Verifier | none | no |
| ViVA waveforms, Calculator | waves dock (ngspice, Xyce and LTspice raw files), output expressions | no (results never cross) |
| Spectre | Spectre netlist dialect; Schemify itself runs ngspice or ESPice | n/a |

**Outside Schemify.** Layout (L/XL/EXL, pcells, ModGen), DRC/LVS (PVS, Pegasus, Calibre),
extraction (Quantus, extracted views), EAD, Voltus-Fi, RF and AMS Designer have no Schemify
side. A Schemify schematic sent to Virtuoso is a normal schematic, so all of those work on it
there. For LVS outside Virtuoso, `schemify netlist --dialect cdl` writes the CDL. SKILL
automation stays in Virtuoso. Schemify's own scripting is Lua config (`init.lua`) and wasm
plugins.

## xschem ↔ Schemify map

Same reading as above. Exact rules: import [import.md](https://github.com/OmarSiwy/Schemify/blob/main/docs/spec/import.md)
§1–§6, export [interop.md](https://github.com/OmarSiwy/Schemify/blob/main/docs/spec/interop.md) §11.
xschem has no ADE: its simulation setup is the netlist text placed on the schematic, so the
second table is short.

**Design data**

| xschem | Schemify | Crosses |
|---|---|---|
| a library directory on `XSCHEM_LIBRARY_PATH` (`xschemrc`) | `lib <dir>` line in `schemify.project`, one library per directory (`sym`/`sch`/`xschem` folders skipped); export writes an `xschemrc` fragment | yes |
| `set NAME $PDK_ROOT/…` in the PDK `xschemrc` | `set NAME …` in `schemify.project`; `${NAME}` in netlist text | import only |
| `X.sym` (`type=subcircuit`) + `X.sch` | `.shcomp` | yes |
| `X.sym` alone (a device with a `format`) | `.shprim` | yes |
| `X.sch` alone, or holding netlist text | `.shbench` | yes |
| `X.sch` with `ipin`/`opin`/`iopin` but no `.sym` (an xschem LCC) | `.shcomp` with a derived interface, plus the bench copy `X__top.shbench` | import only |
| `B 5` pin boxes (`name`, `dir`, `sim_pinnumber`) | `pin` (order kept; same-named pins become `alias` pins) | yes |
| `template` `k=v` pairs | `param` (kind inferred: number, bool, string, choice) | yes |
| (none) | param `unit min max step grid doc show`, `prose` docs, `waive`, `constraint` | kept |
| `format`, `lvs_format`, `spectre_format`, `verilog_format` | impl `format spice` / `cdl` / `spectre` / `verilog` | yes |
| `device_model`, `spice_sym_def` | impl `body` | yes (export writes `device_model`) |
| `spice_ignore=true\|open\|short` | impl or inst `ignore` | yes |
| `extra=` nets, a standalone `@VDD` in a format | inherited pins, inst `inherit` | yes |
| `devices/*.sym` (`res capa ind vsource isource nmos4 pmos4 npn pnp diode vcvs vccs cccs ccvs ammeter spice_probe switch_ngspice delay_line bsource parax_cap`) | `core:` devices (`DEVICE_MAP`) | yes |
| `lab_pin`, `lab_wire`, `gnd`, `vdd` | `label` (`global true` for globals) | yes |
| `ipin`/`opin`/`iopin` instance | `port` | yes |
| `noconn.sym` | `noconn` | yes |
| `N` wire (`bus=true`) | `wire` (`bus true`) | yes |
| `C` instance (`rot`, `flip`) | `inst` (`at x y <orient>`, table in import.md §3.1) | yes |
| symbol `L B P A` drawings, symbol `T` text | `line rect poly arc circle`, `stext` | yes |
| schematic `L B P A` drawings | `shape` | export only (import drops them) |
| schematic `T` text | `text` (display-only Tcl texts dropped) | yes |
| PDK `drc=` procs, model-card bins, tech-file grid | param `min` / `max` / `grid` | import only |
| instance `format=` override, `schematic=` alternate | variant cells `X__<inst>`, `X__<alt>` | import only |
| embedded symbols `[…]`, `bus_tap`, symbol generators `x.tcl(…)` | — | no (reviewed) |
| launchers, logos, titles, graphs, images, `architecture` blocks | — | dropped |

**Simulation**

| xschem | Schemify | Crosses |
|---|---|---|
| `code`, `code_shown`, `netlist_commands`, `simulator_commands` (spice) | `code <name> spice` block | yes |
| `.param k=v` lines in that text | `var` | yes (export writes a `.param` block) |
| `.lib`, `.include`, `.temp` lines | kept in the `code` block; export writes the default `corner` as a block | partial |
| `only_toplevel=true` text and devices | the `X__top.shbench` copy | import only |
| xschem graphs, `ngspice::get_node` annotations, the Simulate menu | waves dock, op annotation, `s` / `schemify run` | no (results never cross) |
| (none) | other `corner`s, `sweep`, `montecarlo`, `optimize`, `output`, `spec`, `config` | kept |
| VHDL / Verilog / tEDAx netlisting (`G` `V` `E` blocks) | `format verilog` only | partial |

**Outside Schemify.** xschem's Tcl scripting, its built-in logic simulator (`0`–`4`), symbol
generators and waveform graphs have no Schemify side. The editor itself keeps xschem's keys
and gestures (the Schemify guide, `editor.md`).

[ARCHITECTURE.md](ARCHITECTURE.md) is the plan to lay the code out along these two maps:
one file per row, each holding both directions of that row.

## Install

For yourself, in every project: add it to your Schemify config (`init.lua`) and run
`schemify pack sync`:

```lua
schemify.pack.add {
  { src = "https://github.com/OmarSiwy/Schemify-Interop", name = "interop",
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
cargo install --git https://github.com/OmarSiwy/Schemify-Interop --features cli   # or a release binary, or `nix run github:OmarSiwy/Schemify-Interop --`

schemify-interop virtuoso-export <project> <cell-file>... -o <file.sfyx> [--lib L=V]... [--core-cells]
schemify-interop virtuoso-import <project> <file.sfyx> [--lib L=V]... [--analoglib]
schemify-interop xschem-export   <project> <cell-file>... -o <dir>
schemify-interop xschem-import   <lib-dir>... -o <out-dir> [...]        (spec import.md)
schemify-interop install-skill   <dir>
```

Paths after `<project>` are relative to it. Each conversion writes a report next to its output
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

In Virtuoso: `SfyExportLib("/tmp/gpdk045.sfyx" "gpdk045")`. In Schemify:
`schemify-interop virtuoso-import <project> gpdk045.sfyx`. You get a library `gpdk045/` with
one `.shprim` per device: the same symbol, pins and CDF parameters (as Schemify parameters
with units and choices), and Spectre and CDL templates built from simInfo. Designs that place
these cells export back to the real PDK cells by name, so geometry and parameters match exactly.

To simulate with ngspice in Schemify, give each mirror device a `format spice` template, or add
`spice "…"` to its row in `interop.map`. Formats you add survive later re-imports. A commercial
PDK's mirror holds its symbols and parameter names: keep that library out of public repositories.

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

## What does not cross

Every conversion lists its losses in the report. The main ones:

- Virtuoso config views (hierarchy editor), and maestro Monte Carlo and optimizer setups. They
  are kept on the Schemify side.
- PDK CDF callbacks inside Schemify. Derived parameters (`ad`, `as`, finger widths) are
  computed by the PDK when you load with `?callbacks t`.
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
- **xschem export** is checked against real xschem netlists.
- **What cannot be proven here** is listed as numbered assumptions in spec §13. `SfySelfTest`
  and the probe settle them on the first real run.

## Code

| File | Does |
|---|---|
| `src/design.rs`, `src/nets.rs`, `src/profile.rs` | Schemify files as tables, connectivity, coordinate/orientation/name rules |
| `src/sfyx.rs` | the exchange-file codec |
| `src/virtuoso/` | `write` (export), `read` (import), `ael` (values), `devices` (analogLib table), `ade` (maestro) |
| `src/merge.rs`, `src/remap.rs`, `src/map.rs` | keeping Schemify-only data, re-mastering, `interop.map` |
| `src/xschem_out.rs`, `src/translate.rs` … | xschem export and import |
| `src/job.rs` | one conversion as pure steps, driven by `src/plugin.rs` (editor) and `src/cli.rs` |
| `skill/*.il` | the parts of `schemify.il` |
| `tests/` | `virtuoso_rust`, `virtuoso_skill`, `skill_lint`, `virtuoso_e2e`, `ade`, `exercise1` (a course's benches: maestro + OCEAN goldens), `xschem_export`, and the xschem import suites |

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
| `determinism/run.sh <schemify-interop command> <out>` | the import every platform must write byte for byte |
| `cd fuzz && cargo +nightly fuzz run xschem_read` | the fuzzers (`fuzz/seeds.sh` seeds them) |

CI (`.github/workflows/`): `ci.yml` runs the suite on Linux, macOS and Windows, compares the
import across them and WASI, rebuilds the guest and fails when the committed module differs;
`nightly.yml` runs the corpus ratchet, the fuzzers, and the suite against Schemify `main`;
`release.yml` builds the CLI for every platform into a draft release on a `v*` tag.

To move to a newer Schemify: set every `rev` in `Cargo.toml` to the new commit,
`cargo update -p schemify-abi` (and the other Schemify crates), `./build.sh`, test, and commit
`Cargo.toml`, `Cargo.lock` and `interop.wasm` together.
