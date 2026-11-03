# PDK CDF callbacks: is characterization needed?

Question: does a Schemify⇄Virtuoso interop plugin need to characterize PDK CDF callbacks
(`SfyCharacterize` + `virtuoso::characterize` rule fitting), or is there a simpler design?

**Answer: the rule search is not needed. Two things are: knowing which parameters are
derived and what they depend on, and getting Virtuoso's values for them. Neither needs
fitting.** The plugin goal is "use Schemify as your schematic editor while your team stays
in Virtuoso", so a Virtuoso licence is already present. A discovery-only probe
(`vderived`/`vinputs`), the existing cache and staleness check, and an on-demand
"recompute in Virtuoso" call are exact for any PDK. They need a fraction of the code.

Spec references below are to
`~/.cargo/git/checkouts/schemify-aab686d6da4873b8/1752960/docs/spec/interop.md` (cited
as `interop.md:N`). Schemify decisions are cited as `decisions.md:N` (same checkout,
`docs/dev/decisions.md`).

## 1. What the plugin does today

### Without characterization

- **The mirror carries no callbacks.** A mirrored PDK cell carries the CDF params but not
  their callbacks (`interop.md:377`). The importer reports each one as
  `param X: callback not carried` (`src/virtuoso/read.rs:1513-1517`).
- **`SfyImport ?callbacks t` runs the PDK's callbacks the way the Edit Properties form
  does** (`interop.md:199`, `interop.md:206`; `skill/sfy_cells.il:1085-1114`):
  1. Every value is set first.
  2. The cell CDF is bound as `cdfgData`/`cdfgForm`.
  3. Each set param's callback runs in CDF order.
  4. `cdfUpdateInstParam` writes the result to the instance.
- **A param the PDK locks (`editable "nil"`) is not sent; its callbacks derive it**
  (`skill/sfy_cells.il:1205-1208`). The import log records how the PDK changed the
  values: `clamped … sent X now Y` for a value it rewrote, and `derived … file X now Y`
  for a locked param that came back different (`interop.md:206`).
- **What the README says crosses:** "Derived parameters (`ad`, `as`, finger widths) are
  computed by the PDK when you load with `?callbacks t`" (`README.md:217-218`).

### The cache and staleness machinery (D20)

- **Every instance carries Virtuoso's value with a digest of its inputs.** A `vderived`
  param is stored on each instance as `cached <param> <value> <digest>`
  (`decisions.md:117`; `interop.md:408`; written by `src/virtuoso/read.rs:954-990`,
  `2879-2885`).
- **A stale value blocks Run.** When an input changes, the digest no longer matches.
  Elab then reports `E039`, which blocks Run and is waivable (`docs/spec/netlist.md:185`).
- **Export sends only fresh values.** A stale one is left out, so Virtuoso's callbacks
  recompute it (`src/virtuoso/write.rs:1966-1973`).
- **The next import brings it back fresh.** The end-to-end test
  `a_derived_value_goes_stale_in_schemify_and_comes_back_fresh` covers this
  (`tests/characterize.rs:399-509`).

**There is one gap: today `vderived` and `vinputs` are only ever written by
`characterize::apply_rules`** (`src/virtuoso/characterize.rs:1431`; the only reader of
the flags is `read.rs:1185-1223`). Without a characterization, a derived `ad` arrives as an
ordinary override, is not marked stale, and is exported back as a stale value. It is
skipped only if the PDK locks it. So option A below is not complete yet: it needs the
discovery half of `SfyCharacterize`.

### What characterization adds (`interop.md:406-408`)

- **Discovery.** Each candidate param is nudged once from its defaults. This builds an
  influence graph, which gives the inputs and the derived params
  (`skill/sfy_char.il:104-183`). It costs about one point per candidate.
- **Sweep.** Each input is scanned in ×1.2 steps, step boundaries are bisected to 1e-5,
  and pairs of inputs are scanned together. The cap is 4000 points per device
  (`skill/sfy_char.il:186-395`).
- **Rule search.** For each derived param, the search tries these forms in order: a
  constant, a step table, a table times a monomial, and the affine form
  `m·(α·u+T(x))` over up to 2 inputs, each optionally split by a bool or choice input.
  The search is combinatorial (`src/virtuoso/characterize.rs:330-1060`). A kept rule
  becomes `computed <expr>`. A param no rule explains falls back to `vderived`.
- **Legality.** Clamp and snap runs become `valid <expr>`, which gives `E038`
  (`characterize.rs:1060-1300`; `decisions.md:116`).
- **Holdout check:** three unmeasured points (`characterize.rs:1301-1405`).

The benefit is that derived values can be recomputed offline in Schemify without
Virtuoso, and an illegal input is flagged at edit time. The cost is about 1000 lines of
Rust fitting and legality code, about 300 lines of SKILL sweep, about 690 lines of tests,
and a sweep of up to 4000 points per device. The result is still only partial: any param
outside the form library stays `vderived` anyway.

## 2. Do compact models compute the derived params when they are omitted?

| Model | AD/AS/PD/PS omitted | NRD/NRS omitted | LOD (SA/SB/SD) and WPE (SC*) omitted | Source |
|---|---|---|---|---|
| BSIM4 4.8 | **Computed** from NF, geoMod, DMCG/DMCI/DMDG/DMCGT, MIN: "If (AS is given) Aseff = AS Else Aseff computed from NF, DWJ, geoMod, DMCG, DMCI, DMDG, DMCGT, and MIN" (§12.2.1, p.103). DMCG defaults to 0 and DMCI defaults to DMCG (App. A.11), so the area is 0 unless the model card sets them. NF splits W itself: "Weffcj = Wdrawn/NF − …" (Eq. 1.17). | With rgeoMod=0 (default): "Source diffusion resistance Rsdiff is not generated" (§12.2.2) | **Off, not computed**: SA/SB "If not given or (<=0), stress effect will be turned off"; SC "If not given or <=0.0, turn off WPE" (App. A.14, A.15) | BSIM4v4.8.0 manual, UC Berkeley (ngspice mirror; bsim.berkeley.edu returned 401): https://ngspice.sourceforge.io/external-documents/models/BSIM480_Manual.pdf |
| BSIM4 in ngspice | Computed, same path: `b4temp.c` calls `BSIM4PAeffGeo` unless `sourceAreaGiven`/`drainAreaGiven`. `.option DEFAD` applies only to level 1-3/6/9 models: "the default is 0" (§11.1.5). The BSIM4 code never reads it. | as BSIM4 | as BSIM4 | ngspice-47 manual https://ngspice.sourceforge.io/docs/ngspice-47-manual.pdf §7.6.1, §11.1.5; ngspice source `src/spicelib/devices/bsim4/b4temp.c:2021-2034` |
| BSIM-CMG 111 | **Zero, not computed**: ASEJ/ADEJ/PSEJ/PDEJ/ASEO/ADEO/PSEO/PDEO all default to 0 (§6.1). The junction current keeps only the fin gate-edge term `TFIN·NFINtotal·Jswgs` (Eq. 3.538). | 0 for RGEOMOD=0. RGEOMOD=1 computes the resistance from LRSD/HFIN/FPITCH/NFIN (§3.15.2.2). | no LOD/WPE instance params of the BSIM4 kind (unverified beyond §6.1) | BSIM-CMG 111.0.0 Technical Manual (SIMetrix mirror): https://www.simetrix.co.uk/supplementary/Docs/BSIM-CMG/BSIM-CMG_111.0.0_Technical_Manual.pdf |
| BSIM-IMG 102.9.6 | **Zero, not computed** (`AS`/`AD`/`PS`/`PD` default to 0.0; they feed only the BOX capacitance) | 0 (`RSourceGeo = RSHS*NRS`) | none | Verilog-A `bsimimg_body.include:99-110, 1224-1266` (mirror github.com/dwarning/VA-Models `code/bsimimg/vacode/`; no manual reachable: unverified against the manual) |
| L-UTSOI 102.7 (CEA-Leti FD-SOI) | **Constant default, not computed**: ASOURCE defaults to 1e-12 m², PSOURCE to 1e-6 m (§2.3); per finger it uses ASOURCE/NF (Eq. 3.1-3.6) | no NRS/NRD | SA/SB/SD default to 0; the stress model needs `SA>0 && SB>0` (and SD>0 for NF>1) (`L_UTSOI_102_module.include:1215`): **off** | CEA-Leti code package https://www.cea.fr/cea-tech/leti/l-utsoisupport/Documents/LUTSOI102/L_UTSOI_10270_code_package.zip (User's Manual §2.3, `vacode/L_UTSOI_102_module.include:140`) |
| Spectre built-ins | unverified (no public Cadence document found) | unverified | unverified | — |

Plainly:

- **BSIM4:** if the areas are omitted, the model computes them from W/L/NF and its own
  geometry params, but only when the model card sets DMCG/DMCI. If LOD and WPE are
  omitted, they are switched off, which changes the simulated device.
- **BSIM-CMG, BSIM-IMG, L-UTSOI:** omitted means zero or a constant. Nothing is computed.

So option C (omit the derived params) changes simulation results for every model listed,
including LOD/WPE in BSIM4. It is not a general design.

## 3. How open PDKs avoid SKILL callbacks

- **sky130:** the derived values are expressions in the xschem symbol template.
  - StefanSchippers/xschem_sky130 @ `2bfd7d6`, `sky130_fd_pr/nfet_01v8.sym:29`:
    `ad=\\"expr('int((@nf + 1)/2) * @W / @nf * 0.29')\\"`. Lines 30-33 do the same for
    `pd as ps nrd nrs`.
  - LOD is fixed at `sa=0 sb=0 sd=0` (`:34`).
  - The finger-width check is a check-only Tcl proc (`xschemrc:510`
    `proc sky130_fet_drc`). It warns and rewrites nothing.
  - Permalink: https://github.com/StefanSchippers/xschem_sky130/blob/2bfd7d654e12492d47986b78e68bb770e633b5da/sky130_fd_pr/nfet_01v8.sym#L29
- **GF180MCU:** the same scheme, written as SPICE expressions that the simulator
  evaluates.
  - fossi-foundation/globalfoundries-pdk-libs-gf180mcu_fd_pr @ `5e4be87`,
    `cells/xschem/gf180mcu_fd_pr/nfet_03v3.sym:30`: `ad=\\"'int((nf+1)/2) * W/nf * 0.18u'\\"`.
    Lines 31-34 cover `pd as ps nrd nrs`.
- **IHP SG13G2:** the schematic carries no area or perimeter params. The model subckt
  computes them.
  - IHP-GmbH/IHP-Open-PDK @ `5e6d592`. The symbol format is only
    `w=@w l=@l ng=@ng m=@m` (`ihp-sg13g2/libs.tech/xschem/sg13g2_pr/sg13_lv_nmos.sym:20`).
  - The subckt defaults to `as=0 ad=0 pd=0 ps=0` and says "if as = 0, calculate value,
    else take it" (`libs.tech/ngspice/models/sg13g2_moslv_mod.lib:66-70`). It then
    computes `as='max(w/ng,wmin)*(z1+((ng-1)/2)*z2)'` (`:75-81`).
  - The one derived param that needs writing back, finger width `ws`, is an
    **iPDK-style Tcl callback**: `iPDK_setParamValue ws [Ftos [expr $tmpw/$tmpng] 3]u`
    (`libs.tech/klayout/python/sg13g2_pycell_lib/callbacks/mos_cb.tcl:67`). It is bound in
    `callbacks.json:172-186`.
  - KLayout's `coerce_parameters` runs these callbacks through a `tkinter.Tcl()`
    interpreter (IHP-GmbH/pycell4klayout-api @ `ba6ef9c`, `source/python/cni/dlo.py:301-352`).
- **KLayout:** `coerce_parameters_impl` "is supposed to adjust parameters to render a
  consistent parameter set and to fix parameter range errors"
  (https://www.klayout.de/doc/code/class_PCellDeclarationHelper.html).

**Pattern:** when the PDK author controls the kit, derived values become expressions, are
computed in the model subckt, or are handled by a Tcl/Python hook the PDK ships. None of
this helps with a commercial kit whose callbacks are SKILL and often encrypted
(`interop.md:377`). Schemify cannot get the formulas from the PDK author.

## 4. How other tools handle SKILL callbacks outside Virtuoso

- **Cadence itself:** a callback "Specifies a SKILL routine to be executed whenever the
  value of the parameter changes". Also: "Callbacks are called only from the Add Instance
  and Edit Object Properties forms. As a consequence, many problems are associated with
  callbacks and their use is discouraged."
  - Source: Cadence *Component Description Format User Guide* 6.1.4, Feb 2011. This copy
    is a third-party mirror: https://amarketplaceofideas.com/wp-content/uploads/2015/08/cdfuser.pdf.
  - Callbacks are a form-layer feature, which is why `SfyCallbacks` replays the form's
    sequence.
  - Batch helpers such as `CCSinvokeCdfCallbacks` are documented only on Cadence's
    login-gated support site: unverified.
- **Si2 / IPL iPDK:** the iPDK uses "open standard languages, Tcl and Python, for
  parameterized layout cells, callbacks, and technology files". It "also includes SKILL
  callbacks and CDF files to provide compatibility with… Cadence IC6.1"
  (TSMC, https://pr.tsmc.com/english/news/1565).
  - Si2 lists an "OpenPDK Design Parameter and Callback Specification V1.0"; the program
    is archived and the spec is behind a request form (https://si2.org/si2-openstandards/).
  - **The foundry ships the callbacks twice. Nothing translates SKILL.**
- **Synopsys Custom Compiler:** "Supported extension languages include TCL, Python and
  C++. Custom Compiler supports the interoperable process design kit (iPDK) standard"
  (https://www.synopsys.com/content/dam/synopsys/implementation&signoff/datasheets/custom-compiler-ds.pdf).
  Synopsys makes no public claim to run SKILL CDF callbacks (unverified either way).
- **Keysight ADS:**
  - The RFIC Dynamic Link runs *inside* Virtuoso. Its spectre-to-ADS migration "is not a
    complete translator… not as a 100% guaranteed translator"
    (https://edadownload.software.keysight.com/eedl/ads/2011/pdf/dynlnklc.pdf).
  - Keysight's own iPDK demo uses "parameter callbacks written in OA-TCL"
    (https://docs.keysight.com/download/attachments/4402214/mmw_demo_ipdk_userguide.pdf).
  - ADS callbacks "in AEL or Lisp": the quote comes from a search-engine index of
    docs.keysight.com, so it is unverified.
- **Siemens:** "automatic conversion of third party PDKs, including callback and PCell
  code" (whitepaper:
  https://static.sw.cdn.siemens.com/siemens-disw-assets/public/1xqViRATpGkS6iFOXguaxE/en-US/Siemens-SW-Full-flow-solution-for-analog-mixed-signal-IC-design-WP-84095-D9.pdf).
  It gives no method and does not say SKILL.
- **Empyrean Aether and Silvaco:** they publicly claim iPDK support only.

**Pattern:** no vendor publicly claims a general way to execute or infer SKILL callbacks
outside Virtuoso. The industry answer is either a second, Tcl copy of the callbacks or
running inside Virtuoso (Keysight's Dynamic Link). Characterization is the plugin
inventing a third route, and no one else ships it.

## 5. Alternatives

| | Exact | Virtuoso licence at edit time | Code | Any PDK | Notes |
|---|---|---|---|---|---|
| **A. No fitting; `vderived` + cache + E039; refresh on round trip** | yes (values are Virtuoso's) | no (only at round trip) | small: keep discovery (`sfy_char.il:75-183`), `inputs_digest`, `cached` read/write; add `apply_vderived` (~40 lines) | yes | Already implemented except that `vderived`/`vinputs` must come from discovery, not from failed fits (§1). No edit-time legality, but the import log's `clamped` lines report rewrites after the fact (`interop.md:206`). |
| **B. A + on-demand callback service** | yes | yes, when refreshing | small: `SfyCharMeasure` (`sfy_char.il:75`) already places a scratch instance, sets the inputs with callbacks, and reads back every netlisted param. `SfyCharacterize ?points` is already "measure these exact points". One call per stale instance writes `cached` lines. | yes | It also gives exact legality: the read-back shows what the PDK rewrote. Latency of `virtuoso -nograph` startup is unmeasured; a live session (the `live/` kit) avoids it. |
| **C. Omit derived params from the netlist** | **no** (§2: LOD/WPE off; areas 0 in CMG/IMG; constant in UTSOI) | no | none | no | Only correct where the PDK's own subckt derives them (IHP, §3). That cannot be detected generically. |
| **D. Direct regression instead of search** | exact only when the verify step passes; same form ceiling | no | medium: still needs the 4000-point sweep, the tolerance model and the verifier | partial | Log-space least squares fits monomials, but the step tables (`steps(x,…)`) and `if()` splits still need breakpoint location. It still falls back to `vderived`, so A's machinery stays anyway. |
| **E. Current search (+ faster screening)** | exact where verified; partial coverage | no | largest (~1000 Rust + ~300 SKILL + ~690 test lines; sweep ≤4000 pts/device) | partial | The screening work in the uncommitted `characterize.rs` diff speeds it up but does not change the coverage. |

## 6. Recommendation

Adopt **A + B**:

1. **Keep:** the `cached`/digest/E039 machinery (`read.rs:954-990`, `write.rs:1966-1973`,
   `characterize::inputs_digest`, `canonical_value`), `SfySetParams`/`SfyCallbacks`, and
   the discovery half of `SfyCharacterize` (`SfyCharDiscover`/`SfyCharPick`,
   `sfy_char.il:104-183`).
2. **Reduce `pdk-rules` to marking only:** every derived param becomes `vderived true`
   and the header gets `vinputs`. No fitting.
3. **Add one refresh command:** for each stale instance, run `SfyCharacterize ?points`
   with its inputs (`SfyCharMeasure`) and write the fresh `cached` lines. Report rewritten
   inputs as `clamped`.
4. **Delete:** the sweep and bisection (`sfy_char.il:186-394`), `Grid`, `Body`, `Rule`,
   `Device::rules`, legality, `holdout_points`, `check`, `strip_overrides`
   (`characterize.rs:330-1405`, `1526-1548`), the rule and legality tests in
   `tests/characterize.rs`, and the `pdk.rules*`/`pdk.holdout` live checks. Drop the
   in-flight screening diff.
5. **Keep `computed`/`valid` (D12/D13) for PDKs that publish formulas** (sky130/GF180
   symbol expressions, §3), written by hand or by the xschem importer. Do not infer them.
6. **Revisit offline recompute (D) only if users must simulate in Schemify without
   Virtuoso reachable.** That case contradicts the plugin's stated goal.

**Cost:** an edited derived value is unknown until Virtuoso recomputes it, so Run blocks
on E039 until then. That is the honest state. Every other tool in §4 either runs the
PDK's own code or a second copy of it.

## Adopted (2026-11-02)

A + B, implemented in this repository:

- `SfyCharacterize` probes each candidate at a few values (×2, ×½, ×10; counts +1 +2 +7;
  the other boolean; every other choice) and records which netlisted params move; with
  `?inputs` and `?points` (name/value pairs) it measures exactly those points. The sweep,
  bisection, cross and random points are gone (`skill/sfy_char.il` 416 → 256 lines).
- `virtuoso::characterize`: `Device::derived`, `mark_derived` (`vderived`, header
  `vinputs`; a param the project already `computed`s is left alone), `refresh_wants`,
  `refresh_skill`, `refresh_apply`. The rule fitting, legality and holdout code is gone
  (1716 → ~800 lines including the refresh).
- CLI: `pdk-derived <project> <char.sfyx>` replaces `pdk-rules`;
  `virtuoso-refresh <project> --scratch <lib> -o <dir>` / `--apply <dir>` is new. A
  refresh sends only the inputs an instance overrides: the others stay at Virtuoso's own
  default text, which a callback may read differently from a respelling (`atof("20n")`).
- Live kit: `pdk.mark.*` and `pdk.refresh.*` (a resized instance refreshed by measuring,
  against the callbacks' values for the resized design loaded into Virtuoso) replace
  `pdk.rules.*`, `pdk.holdout.*` and `pdk.rules.check.*`.

### Proposed replacement for interop.md §8.3 (Schemify core)

> `(SfyCharacterize path lib cells scratch ?inputs ?points)` finds which netlisted
> parameters each of `cells` derives through its CDF callbacks, and measures them at
> given inputs. **Discovery:** candidates as before; each is nudged from the defaults
> (number ×2, ×½, ×10; count +1, +2, +7; the other boolean; every other choice) with
> every callback running, and the netlisted params any nudge changes are recorded
> (`probe`); inputs are read from the probes as an influence graph seeded by
> `paramLabelSet` (unchanged). **Points:** with `?inputs` (names) and `?points` (lists of
> `(input value-text)` pairs, an input left out at its CDF default) each point is
> measured (`pt`). `schemify-interop pdk-derived <project-dir> <char.sfyx>` marks every
> netlisted param an input's probe moved, other than the inputs, `vderived true` on the
> mirror cell, the header `vinputs <input>…` naming the inputs; a param with `computed`
> is left to its rule. **Cached derived values:** unchanged (D20 as amended 2026-10-15).
> **Refresh:** `schemify-interop virtuoso-refresh <project-dir> --scratch <lib> -o <dir>`
> writes `<dir>/refresh.il`, one `SfyCharacterize ?inputs ?points` per mirror cell with
> one point per distinct set of overrides among the instances whose cached values are
> stale or missing (E039), overrides as the export's CDF text; loaded in Virtuoso, it
> measures them on a scratch schematic and touches no design. `--apply <dir>` writes the
> measured values as fresh `cached` lines (digest of the instance's inputs). No rule is
> fitted: every derived value is Virtuoso's. D13's `valid` rules are no longer inferred
> (an author may still write them).
