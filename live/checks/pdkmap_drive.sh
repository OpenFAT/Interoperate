#!/usr/bin/env bash
# pdk-map live check driver: runs pdkmap_names on the Virtuoso host (SfyCdfNames over a
# made-up kit drawn in the scratch library sfyprobe_pdkmap, which run.sh deletes after the
# session) and does the Schemify half here, with this checkout's CLI.
#
#   SFY_HOST=<host> live/checks/pdkmap_drive.sh   (SFY_HOST: the Virtuoso host's ssh name, required)
#
# Ships live/checks, the kit fixture (as pdkmap_kit.il), build.il's helpers (as
# pdkmap_build.il) and this checkout's schemify.il to <host>:sfytest/checks/src_pdkmap (its
# own directory and schemify_pdkmap.il), runs `run.sh pdkmap_names` there (under the host's
# run lock), fetches out/pdkmap_names.sfyx, requires it to be the mock's dump of the kit
# (tests/pdk_map.rs) and `pdk-map` of tests/fixtures/pdk_map/scan onto it to be the mock's
# map. Prints its own and the check's lines and exits 1 on any FAIL. Files stay in
# $SFYCHECK_LOCAL (default a fresh temporary directory, printed at the end).
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$here/../.."
host="${SFY_HOST:-}"
[[ -n $host ]] || { echo "usage: SFY_HOST=<host> $0   (SFY_HOST: the Virtuoso host's ssh name)" >&2; exit 2; }
work=sfytest/checks
src=$work/src_pdkmap
lib=sfyprobe_pdkmap
tmp="${SFYCHECK_LOCAL:-$(mktemp -d)}"
mkdir -p "$tmp/il"
fails=0
verdict() { if "$2"; then echo "PASS driver: $1"; else echo "FAIL driver: $1"; fails=1; fi; }

cargo build -q --features cli --bin schemify-interop --manifest-path "$repo/Cargo.toml"
bin="${CARGO_TARGET_DIR:-$repo/target}/debug/schemify-interop"
"$bin" install-skill "$tmp/il" >/dev/null
ssh "$host" "mkdir -p $src $work/out && find $work/out -name 'pdkmap_*' -delete"
scp -q "$here"/*.il "$here"/run.sh "$host:$src/"
scp -q "$repo/tests/fixtures/skill/pdkmap_kit.il" "$host:$src/pdkmap_kit.il"
scp -q "$repo/tests/fixtures/skill/build.il" "$host:$src/pdkmap_build.il"
scp -q "$tmp/il/schemify.il" "$host:$work/schemify_pdkmap.il"

status=0
ssh "$host" "env SFYCHECK_IL=\$HOME/$work/schemify_pdkmap.il bash $src/run.sh pdkmap_names" >"$tmp/run.txt" 2>&1 || status=$?
cat "$tmp/run.txt"
if scp -q "$host:$work/out/pdkmap_names.sfyx" "$tmp/cdf.sfyx"; then
  cat >"$tmp/want.sfyx" <<WANT
("sfyx" 1 "schemify.il 1")
("cdfnames" "$lib" "nfoo" "nfoo" ("D" "G" "S" "B") ("model" "w" "l" "nf" "m") ("D" "G" "S"))
("cdfnames" "$lib" "nfoo_b" "nfoo" ("D" "G" "S" "B") ("model" "w" "l" "nf" "m") ("B" "D" "G" "S"))
("cdfnames" "$lib" "nocdf" "" () () ("A" "B"))
("cdfnames" "$lib" "pbar" "pbar" ("D" "G" "S") ("model" "w" "l") ("D" "G" "S"))
("cdfnames" "$lib" "rbaz" "rbaz" ("PLUS" "MINUS") ("model" "r" "m") ("MINUS" "PLUS"))
("end")
WANT
  verdict "the dump is the mock's" "$(diff "$tmp/want.sfyx" "$tmp/cdf.sfyx" >&2 && echo true || echo false)"
  "$bin" pdk-map --scan "$repo/tests/fixtures/pdk_map/scan" --dump "$tmp/cdf.sfyx" --lib fkit \
    -o "$tmp/interop.map"
  cat >"$tmp/want.map" <<WANT
map fkit:nfoo virtuoso $lib nfoo_b
  pin d D
  pin g G
  pin s S
  pin b B
  param w w
  param l l
  param nf nf
  param m m
map fkit:rbaz virtuoso $lib rbaz
  pin p PLUS
  pin n MINUS
  param r r
  param m m
WANT
  verdict "pdk-map writes the mock's map" "$(diff "$tmp/want.map" "$tmp/interop.map" >&2 && echo true || echo false)"
  cat "$tmp/interop.report.txt"
else
  verdict "the dump came back" false
fi
echo "pdkmap_drive: files in $tmp"
[[ $fails == 0 ]] || exit 1
exit $status
