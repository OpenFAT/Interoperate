#!/usr/bin/env bash
# Lane L9 live check driver (design-library CDF callbacks): runs l9_cdfcb on the Virtuoso
# host and does its Schemify half here, with this checkout's CLI.
#
#   SFY_HOST=<host> live/checks/l9_drive.sh   (SFY_HOST: the Virtuoso host's ssh name, required)
#
# Ships live/checks, the kit fixture (as l9_kit.il), build.il's helpers (as l9_build.il) and
# this checkout's schemify.il to <host>:sfytest/checks/src_l9 (its own directory and
# schemify_l9.il, so no other lane's files change), starts `run.sh l9_cdfcb` there, and when
# the session's first dump appears: imports it into a scratch Schemify project and checks
# that amp's plain callback became `computed wf "w / nf"` and the branching one stayed
# Virtuoso's; exports top with I1 moved (l9_second.sfyx) and the first dump with I1 moved
# as Virtuoso should draw it (l9_want.sfyx); then edits amp's interface (a pin moved, a doc
# changed) so its CDF is derived again and exports amp (l9_cel.sfyx). Prints its own and the
# check's lines and exits 1 on any FAIL. Dumps and project stay in $SFYCHECK_LOCAL (default
# a fresh temporary directory, printed at the end).
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$here/../.."
host="${SFY_HOST:-}"
[[ -n $host ]] || { echo "usage: SFY_HOST=<host> $0   (SFY_HOST: the Virtuoso host's ssh name)" >&2; exit 2; }
work=sfytest/checks
src=$work/src_l9
lib=sfyprobe_l9
tmp="${SFYCHECK_LOCAL:-$(mktemp -d)}"
mkdir -p "$tmp/il"
fails=0
verdict() { if "$2"; then echo "PASS driver: $1"; else echo "FAIL driver: $1"; fails=1; fi; }

cargo build -q --features cli --bin schemify-interop --manifest-path "$repo/Cargo.toml"
bin="${CARGO_TARGET_DIR:-$repo/target}/debug/schemify-interop"
"$bin" install-skill "$tmp/il" >/dev/null
ssh "$host" "mkdir -p $src $work/out && find $work/out -name 'l9_*' -delete"
scp -q "$here"/*.il "$here"/run.sh "$host:$src/"
scp -q "$repo/tests/fixtures/skill/cdfcb_kit.il" "$host:$src/l9_kit.il"
scp -q "$repo/tests/fixtures/skill/build.il" "$host:$src/l9_build.il"
scp -q "$tmp/il/schemify.il" "$host:$work/schemify_l9.il"

ssh "$host" "env SFYCHECK_IL=\$HOME/$work/schemify_l9.il bash $src/run.sh l9_cdfcb" >"$tmp/run.txt" 2>&1 &
run=$!
for _ in $(seq 300); do
  ssh "$host" "test -s $work/out/l9_first.sfyx && grep -q '^(\"end\")' $work/out/l9_first.sfyx" && break
  kill -0 $run 2>/dev/null || break
  sleep 3
done
if scp -q "$host:$work/out/l9_first.sfyx" "$tmp/first.sfyx"; then
  mkdir "$tmp/proj"
  cp "$tmp/first.sfyx" "$tmp/proj/first.sfyx"
  "$bin" virtuoso-import "$tmp/proj" first.sfyx >"$tmp/import.txt"
  amp="$(ls "$tmp/proj/$lib"/amp.sh*)"
  top="$(ls "$tmp/proj/$lib"/top.sh*)"
  verdict "the plain callback became computed wf \"w / nf\"" \
    "$(grep -q '^  computed "w / nf"$' "$amp" && echo true || echo false)"
  verdict "only wf is computed (the branching callback stays Virtuoso's)" \
    "$([[ $(grep -c 'computed' "$amp") == 1 ]] && echo true || echo false)"
  verdict "the report says why the branching callback stays" \
    "$(grep -q 'param l: callback not carried: it branches' "$tmp/proj/first.report.md" && echo true || echo false)"
  verdict "I0 carries vprop and vtd" \
    "$([[ $(grep -c '^  v\(prop\|td\) ' "$top") -ge 6 ]] && echo true || echo false)"
  # The redraw: I1 moved 40 units.
  sed -i 's/^  at 200 0 R270$/  at 240 0 R270/' "$top"
  "$bin" virtuoso-export "$tmp/proj" "${top#"$tmp/proj/"}" -o second.sfyx >"$tmp/export.txt"
  sed 's/"I1" "\([a-zA-Z0-9_]*\)" "amp" "symbol" 200000 0/"I1" "\1" "amp" "symbol" 240000 0/' \
    "$tmp/first.sfyx" >"$tmp/want.sfyx"
  # The interface edit: amp's CDF is derived again, wf a CEL default.
  sed -i -e 's/^pin A in -40 0$/pin A in -40 10/' -e 's/^  doc Width$/  doc "Gate width"/' "$amp"
  "$bin" virtuoso-export "$tmp/proj" "${amp#"$tmp/proj/"}" -o cel.sfyx >"$tmp/export_cel.txt"
  verdict "the derived CDF has wf as a CEL default" \
    "$(grep -q '("param" "wf" "string" "iPar(\\"w\\") / iPar(\\"nf\\")"' "$tmp/proj/cel.sfyx" && echo true || echo false)"
  scp -q "$tmp/proj/second.sfyx" "$host:$work/out/l9_second.tmp"
  scp -q "$tmp/want.sfyx" "$host:$work/out/l9_want.tmp"
  scp -q "$tmp/proj/cel.sfyx" "$host:$work/out/l9_cel.tmp"
  ssh "$host" "cd $work/out && mv l9_want.tmp l9_want.sfyx && mv l9_cel.tmp l9_cel.sfyx && mv l9_second.tmp l9_second.sfyx"
fi
status=0
wait $run || status=$?
cat "$tmp/run.txt"
scp -q "$host:$work/out/l9_third.sfyx" "$tmp/third.sfyx" 2>/dev/null || true
echo "l9_drive: dumps and project in $tmp"
[[ $fails == 0 ]] || exit 1
exit $status
