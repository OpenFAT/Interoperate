#!/usr/bin/env bash
# Lane L2 live check driver (gap D): runs l2_labels on the Virtuoso host and does its
# Schemify half here, with this checkout's CLI.
#
#   SFY_HOST=<host> live/checks/l2_drive.sh   (SFY_HOST: the Virtuoso host's ssh name, required)
#
# Ships live/checks, the kit fixture (as l2_kit.il) and this checkout's schemify.il to
# <host>:sfytest/checks, starts `run.sh l2_labels` there, and when the session's first dump
# appears: imports it into a scratch Schemify project, moves the second `d[1:0]` wire label
# 20 units along its bus, exports the cell, and sends back that export (l2_second.sfyx) and
# the first dump with the label moved as Virtuoso should draw it (l2_want.sfyx). Prints the
# check's lines and exits with run.sh's status. Also reports how the real dump of the kit
# differs from the mock's golden (tests/fixtures/skill/cosm_kit.sfyx), for information.
# The three dumps and the Schemify project stay in $SFYCHECK_LOCAL (default a fresh
# temporary directory, printed at the end).
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$here/../.."
host="${SFY_HOST:-}"
[[ -n $host ]] || { echo "usage: SFY_HOST=<host> $0   (SFY_HOST: the Virtuoso host's ssh name)" >&2; exit 2; }
work=sfytest/checks
lib=sfyprobe_l2
tmp="${SFYCHECK_LOCAL:-$(mktemp -d)}"
mkdir -p "$tmp/il"

cargo build -q --features cli --bin schemify-interop --manifest-path "$repo/Cargo.toml"
bin="${CARGO_TARGET_DIR:-$repo/target}/debug/schemify-interop"
"$bin" install-skill "$tmp/il" >/dev/null
ssh "$host" "mkdir -p $work/src $work/out && find $work/out -name 'l2_*' -delete"
scp -q "$here"/*.il "$here"/run.sh "$host:$work/src/"
scp -q "$repo/tests/fixtures/skill/cosm_kit.il" "$host:$work/src/l2_kit.il"
scp -q "$tmp/il/schemify.il" "$host:$work/schemify.il"

ssh "$host" "bash $work/src/run.sh l2_labels" >"$tmp/run.txt" 2>&1 &
run=$!
for _ in $(seq 300); do
  ssh "$host" "test -s $work/out/l2_first.sfyx && grep -q '^(\"end\")' $work/out/l2_first.sfyx" && break
  kill -0 $run 2>/dev/null || break
  sleep 3
done
if scp -q "$host:$work/out/l2_first.sfyx" "$tmp/first.sfyx"; then
  mkdir "$tmp/proj"
  cp "$tmp/first.sfyx" "$tmp/proj/first.sfyx"
  "$bin" virtuoso-import "$tmp/proj" first.sfyx >"$tmp/import.txt"
  cell="$(ls "$tmp/proj/$lib"/kit.sh*)"
  sed -i 's/^\(label [0-9a-z]*\) 40 -200 R0 d\[1:0\]$/\1 60 -200 R0 d[1:0]/' "$cell"
  grep -q ' 60 -200 R0 d\[1:0\]$' "$cell" || { echo "l2_drive: no d[1:0] label at 40 -200 to move"; cat "$cell"; }
  "$bin" virtuoso-export "$tmp/proj" "${cell#"$tmp/proj/"}" -o second.sfyx >"$tmp/export.txt"
  sed 's/"d<1:0>" 40000 220000/"d<1:0>" 60000 220000/' "$tmp/first.sfyx" >"$tmp/want.sfyx"
  scp -q "$tmp/proj/second.sfyx" "$host:$work/out/l2_second.tmp"
  scp -q "$tmp/want.sfyx" "$host:$work/out/l2_want.tmp"
  ssh "$host" "mv $work/out/l2_want.tmp $work/out/l2_want.sfyx && mv $work/out/l2_second.tmp $work/out/l2_second.sfyx"
  # The real dump of the kit against the mock's (library work there).
  kit() { sed -n '/^("cell" "kit" "design")/,$p' "$1" | grep -v '"schemify'; }
  diff <(kit "$repo/tests/fixtures/skill/cosm_kit.sfyx") <(kit "$tmp/first.sfyx") \
    >"$tmp/mock.diff" && echo "l2_drive: the real dump of kit equals the mock's" \
    || { echo "l2_drive: the real dump of kit differs from the mock's:"; cat "$tmp/mock.diff"; }
fi
status=0
wait $run || status=$?
cat "$tmp/run.txt"
scp -q "$host:$work/out/l2_third.sfyx" "$tmp/third.sfyx" 2>/dev/null || true
echo "l2_drive: dumps and project in $tmp"
exit $status
