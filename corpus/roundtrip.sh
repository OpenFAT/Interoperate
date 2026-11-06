#!/usr/bin/env bash
# xschem -> Schemify -> xschem round trip over the corpus (spec interop.md §11): every bench
# that import_all.sh mapped is exported back with `schemify-interop xschem-export`, netlisted
# by the same pinned xschem as golden.sh (the export first on the search path), and compared
# with the bench's golden under spicecmp. The comparison is xschem against xschem, so it
# checks the export alone: a bench that agrees in tests/xschem_differential.rs and differs
# here is an export bug.
#
#   corpus/roundtrip.sh <import-dir> [name-regex]
#
# <import-dir> is the output of `NETLIST=none corpus/import_all.sh <import-dir>`. Writes
# <import-dir>/roundtrip/<bench>/{spice,cdl}/ (export, netlist, logs) and
# <import-dir>/roundtrip/RESULTS.txt (`<bench> <spice|cdl> agree|golden-unevaluated|differ|…`),
# then prints the counts. Exit status: 0, or 2 on a usage or setup error.
set -uo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/.." && pwd)"
cache="$here/.cache"
rt="${1:?usage: roundtrip.sh <import-dir> [name-regex]}"
filter="${2:-}"
[[ -f "$rt/benches.map" ]] || { echo "no $rt/benches.map: run NETLIST=none corpus/import_all.sh $rt" >&2; exit 2; }
[[ -d "$cache/golden" ]] || { echo "no goldens: run corpus/golden.sh" >&2; exit 2; }

nixpkgs_rev=331800de5053fcebacf6813adb5db9c9dca22a0c # golden.sh's xschem 3.4.7
if [[ -z "${XSCHEM:-}" ]]; then
  XSCHEM="$(nix build --no-link --print-out-paths "github:NixOS/nixpkgs/$nixpkgs_rev#xschem")/bin/xschem"
  PATH="$(nix build --no-link --print-out-paths "github:NixOS/nixpkgs/$nixpkgs_rev#tcl^out")/bin:$PATH"
fi
# spicecmp (Schemify's tools/spicecmp) is not in Schemify's public SDK: SPICECMP names a build.
cmp="${SPICECMP:?set SPICECMP to a spicecmp binary (Schemify tools/spicecmp, not public)}"
cargo build -q --release --manifest-path "$repo/Cargo.toml" --features cli --bin schemify-interop || exit 2
bin="${CARGO_TARGET_DIR:-$repo/target}/release/schemify-interop"

work="$rt/roundtrip"
mkdir -p "$work"
results="$work/RESULTS.txt"
: > "$results"
while read -r name grp cell root pdk; do
  [[ -n "$filter" && ! "$name" =~ $filter ]] && continue
  rm -rf "${work:?}/$name"
  for mode in spice cdl; do
    target=$cell tcl=() ext=spice
    if [[ $mode == cdl ]]; then # LVS checks the component, as import_all.sh netlists it
      comp="${cell%__top.shbench}"; comp="${comp%.shbench}"; comp="${comp%.shcomp}.shcomp"
      [[ -f "$rt/$grp/$comp" ]] && target=$comp
      tcl=(--tcl 'set lvs_netlist 1') ext=cdl
    fi
    x="$work/$name/$mode"
    mkdir -p "$x/net"
    if ! "$bin" xschem-export "$rt/$grp" "$target" -o "$x/export" > "$x/export.log" 2>&1; then
      echo "$name $mode export-failed" >> "$results"; continue
    fi
    sch="$x/export/${target%.*}.sch"
    # The group's rcfile (golden.sh), then the export ahead of every library.
    printf 'source %s\nset XSCHEM_LIBRARY_PATH %s:$XSCHEM_LIBRARY_PATH\nsource %s\n' \
      "$cache/rc/$grp.rc" "$x/export" "$x/export/xschemrc" > "$x/rc"
    ( cd "$x/net" || exit
      [[ $root != - ]] && export PDK_ROOT="$root"
      [[ $pdk != - ]] && export PDK="$pdk"
      HOME="$cache/home" timeout 120 "$XSCHEM" -x -r -s -o "$x/net" --rcfile "$x/rc" "${tcl[@]}" \
        --script "$cache/rc/netlist.tcl" "$sch" < /dev/null ) > "$x/xschem.log" 2>&1
    out="$x/net/$(basename "${sch%.sch}").spice"
    [[ -s $out ]] || { echo "$name $mode no-netlist" >> "$results"; continue; }
    g="$cache/golden/$name.$ext"
    theirs="$(grep -m1 -o '[^ ]*/corpus/\.cache' "$g" || true)"
    alias=(--alias "$cache=\$CORPUS")
    [[ -n "$theirs" ]] && alias+=(--alias "$theirs=\$CORPUS")
    if "$cmp" --quiet "${alias[@]}" "$g" "$out" > "$x/cmp.txt" 2>&1; then
      echo "$name $mode agree" >> "$results"
    elif "$cmp" --quiet --bind-free "${alias[@]}" "$g" "$out" > /dev/null 2>&1; then
      echo "$name $mode golden-unevaluated" >> "$results"
    else
      echo "$name $mode differ" >> "$results"
    fi
  done
done < "$rt/benches.map"
awk '{ split($1, s, "-"); c[s[1] " " $2 " " $3]++ } END { for (k in c) print c[k], k }' "$results" | sort -k2,3
