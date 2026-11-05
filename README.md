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
| xschem → Schemify | designs and PDK device libraries (`interop.xschem_import`, `schemify-interop xschem-import`) |
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
    grant = { "read_project", "write_project",
              "read:ANALOG_ROOT", "read:XSCHEM_LIBRARY", "read:PDK_ROOT" },
    fuel = 100000000000 },
}
```

The `read:<NAME>` grants are for the xschem import only: each lets the plugin read (never
write) under the directory environment variable `<NAME>` names, as `${NAME}/…` paths
([xschem](#xschem) below). Leave out the ones you do not use; the other commands need only
`read_project` and `write_project`.

Pin a release with `version = "v0.1.0"`. For one project (so everyone who opens it gets the
plugin): copy `interop.wasm` into the project and add it to the project manifest:

```
plugin interop path plugins/interop.wasm
  grant read_project
  grant write_project
  grant read:ANALOG_ROOT
  grant read:XSCHEM_LIBRARY
  grant read:PDK_ROOT
  fuel 100000000000
```

`fuel` is the per-event instruction budget (plugin-abi §2). The default, 10^8, is too small for
a conversion of a real design (a Virtuoso export of a small project with a PDK needed about
10^11), so give the ceiling, 10^11: with more fuel than the default the plugin runs on a worker
thread and the editor does not freeze while it converts. A conversion that runs out says so in
the status line (`interop: command interop.virtuoso_export: out of fuel (budget N), …`).

In the editor the File menu gains **Export to Virtuoso (.sfyx)…**, **Import from Virtuoso
(.sfyx)…**, **Export to xschem…**, **Import from xschem…** and **Write Virtuoso SKILL bridge
(schemify.il)**. Without
the editor, the same conversions run from the command line:

```
cargo install --git https://github.com/OpenFAT/Interoperate --features cli   # or a release binary, or `nix run github:OpenFAT/Interoperate --`

schemify-interop virtuoso-export <project> <cell-file|dir>... -o <file.sfyx> [--lib L=V]... [--core-cells]
schemify-interop virtuoso-import <project> <file.sfyx> [--lib L=V]... [--analoglib]
schemify-interop xschem-export   <project> <cell-file|dir>... -o <dir>
schemify-interop xschem-import   <lib-dir>... -o <out-dir> [--root <dir>]... [--models <file>[:<section>]]... [--xschemrc <file>]...
                                 [--spiceinit <file>] [--tech <file>] [--cdf <dump>] [--no-core]   (spec import.md)
schemify-interop install-skill   <dir>
schemify-interop pdk-derived     <project> <char.sfyx>                   (derived parameters, below)
schemify-interop virtuoso-refresh <project> --scratch <lib> -o <dir> | --apply <dir>
schemify-interop maestro-results <project> <dump.sfyx> <bench.shbench>   (Maestro results, below)
schemify-interop layout-export   <project> <bundle-dir>                  (layout views, below)
schemify-interop layout-import   <project> <lib>/<cell>[/<view>] -o <bundle-dir>
schemify-interop pdk-scan        [--stdin] --root <dir> --entry <file> --dialect spice|hspice|spectre [--section S] [--lib L] | --remote-script | --protocol   (PDK scan below)
schemify-interop pdk-map         --scan <dir> (--dump <cdfnames.sfyx> | --host <ssh name> --vlib <lib>...) [--lib <name>] -o interop.map | --remote-script --vlib <lib>...   (PDK map below)
schemify-interop pdk-models      --root <dir> --entry <relpath>... --from <d> --to <d> -o <outdir>   (d: spice|hspice|spectre|vacask|xyce; PDK models below)
schemify-interop ciel            <args>...      (ciel from PATH, else a private venv pinned to ciel 3.2.0 under Schemify's data directory, <data>/tools/ciel-3.2.0; needs python3; install output on stderr, ciel's exit status)
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
analyses come back as one code block named after the test. Monte Carlo and optimizer records
go back only into a view that came from Virtuoso, and anything without a maestro form stays on
the Schemify side; the report lists them.

### Different libraries for the same devices (`interop.map`)

When the Schemify library and the Virtuoso library name the same devices differently (an
xschem-imported sky130 against a Virtuoso sky130 kit), write `interop.map` at the project
root. It maps cell, pins and parameters, including value scaling. Such instances are
re-mastered: their connections ride on labels, so no net changes even though the two symbols
differ in shape (spec §6.4, §8.2).

### xschem

`schemify-interop xschem-export <project> top.shbench -o xs/` writes `xs/<lib>/<cell>.sym|.sch`
and an `xschemrc` fragment. Core devices land on xschem's own `devices/*.sym`, and cells first
imported from xschem point back at their original symbols while unedited (header `xfp`, the
fingerprint of the import); one edited in Schemify is written as its own cell, with a report
row. The test suite netlists the export
with a real xschem and compares it with Schemify's netlist.

The import runs in the editor (File ▸ **Import from xschem…**) or headless:

```
interop.xschem_import <lib-dir>... [--root <dir>]... [--xschemrc <file>]... [--models <file>[:<section>]]... --out <out-dir>

schemify cmd . interop.xschem_import '${XSCHEM_LIBRARY}/devices' '${ANALOG_ROOT}/design' \
    --root '${XSCHEM_LIBRARY}' \
    --xschemrc '${PDK_ROOT}/sky130A/libs.tech/xschem/xschemrc' --out imported
```

It is `schemify-interop xschem-import` (same job, same bytes out) without the command line's
host-only inputs (`--spiceinit --tech --cdf --no-core`) and its output check. Every `.sym` and
`.sch` under each `<lib-dir>` is converted; a `--root` only joins the search path, so the
symbols it holds must also be under a `<lib-dir>`. Paths are project-relative, or `${NAME}/…`
under a directory the plugin has the grant `read:<NAME>` for:

| Grant | Reads |
|---|---|
| `read_project` | project-relative paths |
| `read:ANALOG_ROOT` | `${ANALOG_ROOT}/…`: your design trees |
| `read:XSCHEM_LIBRARY` | `${XSCHEM_LIBRARY}/…`: xschem's own libraries (`devices/`) |
| `read:PDK_ROOT` | `${PDK_ROOT}/…`: a PDK's xschem library, `xschemrc` and model cards |
| `write_project` | the output directory, inside the project |

Any other variable works the same once granted. A path outside the project and every granted
root (an absolute path, `..`, or `${NAME}` without its grant) is refused by Schemify before
anything is read, and the plugin logs `interop: <path>: refused: outside the project and every
granted read:<NAME> root`. Outputs land in `<out-dir>/<lib>/<cell>.<ext>`, with
`schemify.project` and `import-report.{md,txt}`.

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
are followed for the selected section.

### PDK map (remote PDKs)

`schemify-interop pdk-map --scan <dir> (--dump <cdfnames.sfyx> | --host <ssh name> --vlib <lib>...) [--lib <name>] -o interop.map`
writes the `interop.map` rows that send a scanned kit's cells to the kit's own Virtuoso
cells, so the rows come from the kit rather than from someone's notes. `<dir>` holds the
scanned `.shprim` cells of Schemify library `--lib` (default: the directory's name). The dump
is what `SfyCdfNames("<file>" list("<vlib>" ...))` writes in Virtuoso: for each cell of each
library, its base CDF's `model` default, its spectre simInfo `termOrder`, its CDF parameter
names and its symbol's terminal names. No values, no geometry.

A scanned cell `X` goes to the Virtuoso cell whose `model` default is `X` (ignoring case)
and whose `termOrder` has as many terminals as `X` has pins. Pins pair by position, and each
parameter name the two share becomes a `param` row. Ties go first to the cell whose symbol
has exactly the `termOrder` terminals (a symbol with its body pin beats one that inherits
it), then to the library named first, then to the cell name. `interop.report.md` (`.txt`)
beside the map gives each cell's choice and the reason for it (`mapped`) and says why each
other cell was not mapped (`needs_review:unmapped`: no candidate, or a pin-count mismatch).
The map passes `Mapping::parse` before it is written. It holds the kit's names, so keep it
out of public repositories, like a mirror.

After a remote scan refresh, PDK-Manager makes the dump on the kit host and maps it
locally in one command: `pdk-map --scan <cells> --host <ssh name> --vlib <vlib>... -o
interop.map` runs `ssh <host> sh -s` (whatever the login shell) on a script that starts
Virtuoso in batch there on `SfyCdfNames` and prints only the dump. `pdk-map
--remote-script --vlib <vlib>...` prints that script, for a caller that runs ssh itself and
passes the output to `--dump`. Nothing about the site is built in: the script takes these
variables from the environment `pdk-map` runs in:

| Variable | On the host |
|---|---|
| `SFY_CADENCE_ENV` | The site's or kit's Cadence setup script, sourced before Virtuoso starts: through `csh` (as `live/checks/run.sh` sources its `SFYCHECK_ENV`), or through `sh` when it ends in `.sh`. A relative path is relative to the login directory. |
| `VIRTUOSO` | The Virtuoso command, with arguments (default `virtuoso`, from the setup's `PATH`); the script adds `-nograph -restore <file> -log <file>`. |
| `CDS_LIC_FILE`, `CDS_LIC_ONLY` | Set after the setup script, so they win over it. Unset here: the setup's own. |
| `SFY_CDS_DIR` | The directory whose `cds.lib` defines the libraries, where Virtuoso runs (default the login directory). |
| `SFY_VIRTUOSO_TIMEOUT` | Seconds the session may take (default 1800). |

When Virtuoso cannot start (no command after the setup, an unreadable setup script, no
dump), the command fails with the reason and these variable names; the session log, if
any, stays on the host and only its path is printed. The script's temporary directory is
removed otherwise.

### PDK models (local kits)

`schemify-interop pdk-models --root <dir> --entry <relpath>... --from <d> --to <d> -o <outdir>`
converts each model file a kit reaches into another dialect at the same relative path,
reporting losses by SpiceRack diagnostic code (`diag <relpath> <code> <count>`, never the
cards). The root must contain every file the kit includes (sky130: `--root sky130A`).

## What does not cross

Every conversion lists its losses in the report. The main ones:

- Config views (hierarchy editor) now round-trip: their rules become the bench's `config`
  record, a Maestro test bound to one keeps that binding, and an unedited config goes back
  byte for byte. Lost after you edit one in Schemify: views with no Schemify impl, duplicate
  views and comments. Not carried at all: a config view on a cell that is not a bench,
  `prop.cfg`, and the library names inside the text when `SfyImport ?lib` renames a library.
- Maestro setups now round-trip in full: the view's own setup rides in the bench, and a
  re-export updates the tests in place with only what you changed, instead of recreating
  them.
- Monte Carlo and optimizer setups map onto `montecarlo` and `optimize` records (samples,
  seed, sampling, nominal; algorithm, budget, goals from specs, var ranges as `vary`), and
  an edit goes back into the stored setup as only that change. They go back only into a
  view that came from Virtuoso: a bench made in Schemify has no setup to write them into,
  so they are reported. Reported, not sent: other algorithms, the optimizer's seed, config
  and corner. Monte Carlo draws do not match Spectre's sample by sample.
- A Maestro view of several tests becomes one bench with a `test` record per test: its own
  var values, enable, config, outputs and code; device parameters become `param` lines and
  `calcVal` becomes `result()` (runs refuse it for now). Lost: run plans, reliability,
  checks and asserts. A test's corner selection and design cell cross (`test corner`,
  `test design`). Kept only in the stored setup, not editable in Schemify, each a report
  row: spec weights, tolerances, `info` and minimize/maximize goals, per-corner spec
  overrides, an output's evaluation over corners, a test's own simulator, other maestro
  views of the cell. Disabled vars and outputs still export enabled.
- Maestro corner groups and a swept `temperature` keep their first point or model section;
  analysis forms map onto Spectre through ADE's own defaults (`hb` tones and `sens` are
  reported). Calculator functions without a Schemify form (`bandwidth`, `riseTime`, …) come
  back as commented outputs; `.ic`, `.nodeset` and `.meas` lines stay code.
- ADE L states (`SfyAdeLExport`, `SfyAdeLImport`) map onto the bench like a Maestro test:
  design, variables, temperature, analyses, model files and outputs (no corners or specs).
  An unedited bench gives the state back byte for byte; an edited one becomes a Maestro
  test, never an ADE L state (Interop writes no ADE L SKILL). A component it cannot read,
  such as the `convergence` archive, is kept and reported.
- Maestro results (`SfyAdeResults`, then `schemify-interop maestro-results <project> <dump>
  <bench>`) become one imported Schemify run of the bench: each corner and design point is
  a job with its waveforms, values and spec verdicts, read exactly from the history's
  `.rdb`. Only the first test maps. Reported, not mapped: the other tests, values reduced
  across corners or sweeps, Monte Carlo statistics, and the `near` verdict (it comes across
  as `fail`). sp, stb, pss and noise totals and the operating point come across; noise
  contributions, pac sidebands, stb margins and `dcOpInfo` device values are skipped lines,
  and a pss harmonic axis is a plain sweep.
- Layout views cross as GDS (`SfyLayoutExport`, `schemify-interop layout-export`/
  `layout-import`, `SfyLayoutImport`): `<cell>.<view>.gds` beside the cell for KLayout,
  with a generated `.lyp`/`.lyt`. An unchanged GDS gives the OA view back byte for byte; an
  edited one is streamed in by `strmin` and the report lists KLayout's diff. Lost after an
  edit: PCell parameters, nets other than pins, cellview and shape properties, label font
  and height, vias (they come back as instances), blockages, rows, markers, groups,
  boundaries, constraints, and edits inside existing sub-cells. Extracted views become a
  `code` block.
- CDF callbacks: a design library's callbacks of the plain assignment form
  (`cdfgData->w->value = …` over numbers, params and arithmetic) now cross as Schemify
  `computed` math. The rest stay Virtuoso's, each with its reason in the report, and so do a
  PDK's derived parameters (`ad`, `as`, finger widths): see "Derived parameters" above.
- On a cellview you redraw, every surviving object keeps how Virtuoso drew it (label fonts,
  justification, flags and offsets, bus widths, noConn names). What does not: a multi-point
  wire's segmentation (Virtuoso splits it into 2-point wires). Instance properties and
  textDisplays now survive the redraw too.
- Schematics: what Virtuoso holds and Schemify has no record for is counted per kind
  (`skipped`): shapes, textDisplays, user properties, non-rectangular pin figures (as their
  box), unsaved edits. Not carried, each a row: multi-sheet designs (each sheet a pinless
  sub-cell, nets across sheets not joined), SKILL PCell symbols (the default variant),
  Constraint Manager constraints, a library's `libInit.il` and library CDF, OA views the
  dump leaves out, text views other than Verilog-A/Verilog, design management. Virtuoso's
  own connectivity rides in the dump, and any net the import draws differently is a row.
- PDKs: HSPICE-D and auLvs simInfo, `opParamExprList`, `termMapping` and netlist
  procedures stay Virtuoso's, one row per cell; an inherited connection that defaults to
  another one gets the inner default net; a cell past 256 parameters keeps the netlisted
  ones first.
- xschem: non-default corners, sweeps, outputs, specs, optimizer records, waivers. From
  xschem: graphs, images and launchers, Tcl, bus taps, parametric bus widths, per-element
  value lists (`?1 a,b,…`, kept for the way back to xschem), `lvs_ignore`, xschemrc
  netlister settings, CDL netlist text, module parameters and core devices in Verilog; each
  is a report row. A bench copy (`X__top`) exports as its own file and is not merged back.

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
| `NETLIST=none corpus/import_all.sh DIR && corpus/roundtrip.sh DIR` | the corpus through xschem → Schemify → xschem, re-netlisted by real xschem and compared with the goldens (checks the export; nightly ratchets its agree count, RATCHET.md) |
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
| pdk-map | `src/pdk_map.rs`, `src/cli/pdk_map.rs`, `skill/sfy_pdkmap.il`, `live/checks/pdkmap_*` |

Each of L1 to L9 also owns its live check, `live/checks/l<N>_*.il` (`live/checks/run.sh`).
The host comes from `SFY_HOST` (its ssh name; required, no default): a lane with a driver
runs `SFY_HOST=<host> live/checks/l<N>_drive.sh`, which ships `live/checks` and starts
`run.sh` there; any other check runs as `run.sh`'s header shows, `ssh "$SFY_HOST" bash
sfytest/checks/src/run.sh <check>`.
The registration points: a new `.sfyx` record kind is a row in its lane's section of
`src/virtuoso/kinds.rs`; mock Cadence functions are rows in the area's `ROWS` and bodies in
its `call` (`tests/skill/mockdb/`); the SKILL parts are already in `src/skill.rs`'s `PARTS`
(load order, `schemify.il` and the lint), and the CLI subcommands in `src/cli/mod.rs`.

Rules for every lane:

- Never commit `interop.wasm` or `Cargo.lock`: the integration rebuilds both. The working
  tree's `Cargo.lock` holds the local path sources of `.cargo/config.toml`'s patches, so stage
  files by name (`git add <paths>`, never `git commit -a` or `git add -A`) and, before each
  commit, check that `git diff --cached --name-only` lists none of `Cargo.lock`,
  `interop.wasm`, `.cargo/` and `tests/fixtures/cadence/`.
- The spec text (`docs/spec/interop.md`, in the Schemify repository) and "What does not
  cross" above are edited only at integration.
- Build with a `CARGO_TARGET_DIR` of the lane's own.
- The Cadence probe fixtures in `tests/fixtures/cadence/` are local-only (listed in
  `.git/info/exclude`): a test that reads them gets its path from `common::cadence_fixture`
  and returns when it gets `None`, so it skips where they are absent.

To move to a newer Schemify: set every `rev` in `Cargo.toml` to the new commit,
`cargo update -p schemify-abi` (and the other Schemify crates), `./build.sh`, test, and commit
`Cargo.toml`, `Cargo.lock` and `interop.wasm` together.
