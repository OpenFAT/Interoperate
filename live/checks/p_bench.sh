#!/usr/bin/env bash
# Lane P live check (gaps2 P15), on the Virtuoso host with this checkout's CLI there: a
# Schemify bench over the kit inverter p_terms dumped (out/p_terms.sfyx, imported into
# <work>/p15/proj) with two foundry corners, exported (virtuoso-export: the .sfyx and its
# OCEAN script), loaded and run by p_bench.il (ADE's netlist and Spectre per corner, the
# OCEAN script), under the run lock.
#
#   P_MODELS=<kit model library> P_SECTIONS="tt_pre ff_pre" SFYCHECK_WORK=… SFYCHECK_ENV=… \
#     BIN=<schemify-interop> bash p_bench.sh
#
# The kit's paths come from the environment; nothing of the kit is in this file.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
work="${SFYCHECK_WORK:?}"
models="${P_MODELS:?the kit model library}"
sections=(${P_SECTIONS:-tt_pre ff_pre})
d="$work/p15"
rm -rf "$d"; mkdir -p "$d/proj"
printf 'schemify 1 project\n' > "$d/proj/schemify.project"
cp "$work/out/p_terms.sfyx" "$d/proj/"
"$BIN" virtuoso-import "$d/proj" p_terms.sfyx >/dev/null
b="$d/proj/sfyprobe_p/inv_bench.shbench"
{
  sed -n '/^inst /,$p' "$d/proj/sfyprobe_p/inv_tb.shbench"
  cat <<'EOF'

inst pv0001 core:vsource
  at 500 0 R0
  name V0
  value 0.8
inst pv0002 core:vsource
  at 600 0 R0
  name V1
  value "PULSE(0 0.8 0 10p 10p 1n 2n)"

wire pw0001 500 -60 500 -30
wire pw0002 500 30 500 60
wire pw0003 600 -60 600 -30
wire pw0004 600 30 600 60

label pn0001 500 -60 R0 vdd
label pn0002 500 60 R0 0
label pn0003 600 -60 R0 in
label pn0004 600 60 R0 0

output tfall "cross(v(out), 0.4)"

code sim spice
  | .tran 1p 2n
EOF
  for s in "${sections[@]}"; do
    printf '\ncorner %s\n  lib %s %s\n' "$s" "$models" "$s"
  done
} > "$b.body"
printf 'schemify 1 shbench\n\n' > "$b"
cat "$b.body" >> "$b"; rm "$b.body"
"$BIN" virtuoso-export "$d/proj" sfyprobe_p/inv_bench.shbench -o p15.sfyx --lib sfyprobe_p=sfyprobe_p
cp "$d/proj/p15.sfyx" "$work/out/p15.sfyx"
rm -rf "$work/out/p15_ocean"; cp -r "$d/proj/ocean" "$work/out/p15_ocean"
SFYCHECK_PSECTIONS="${sections[*]}" bash "$here/run.sh" p_bench
