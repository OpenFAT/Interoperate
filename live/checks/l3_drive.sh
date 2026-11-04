#!/usr/bin/env bash
# Lane L3 live check driver (gap C): runs l3_config on the Virtuoso host and does its
# Schemify half here, with this checkout's CLI.
#
#   live/checks/l3_drive.sh            (SFYCHECK_HOST, default vhost)
#
# Ships live/checks (as <work>/src_l3) and this checkout's schemify.il (as
# <work>/schemify_l3.il, so lanes running at once do not overwrite each other's) to the
# host, starts `run.sh l3_config` there, and when the session's first dump appears:
# imports it into a scratch Schemify project and exports top unchanged (l3_second.sfyx),
# then edits top's config (order, a cellinst rule) and exports again (l3_edited.sfyx).
# When the session's fourth dump (the edited config as hdb re-saved it) arrives, imports
# it and requires its config rules to equal the edited ones. Prints the check's lines and
# its own PASS/FAIL, removes the l3_* files from the host, and exits non-zero on a FAIL.
# Dumps and projects stay in $SFYCHECK_LOCAL (default a fresh temporary directory).
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$here/../.."
host="${SFYCHECK_HOST:?}"
work=sfytest/checks
lib=sfyprobe_l3
tmp="${SFYCHECK_LOCAL:-$(mktemp -d)}"
mkdir -p "$tmp/il"

cargo build -q --features cli --bin schemify-interop --manifest-path "$repo/Cargo.toml"
bin="${CARGO_TARGET_DIR:-$repo/target}/debug/schemify-interop"
"$bin" install-skill "$tmp/il" >/dev/null
ssh "$host" "bash -c 'mkdir -p $work/src_l3 $work/out && rm -f $work/out/l3_*'"
scp -q "$here"/*.il "$here"/run.sh "$host:$work/src_l3/"
scp -q "$tmp/il/schemify.il" "$host:$work/schemify_l3.il"

ssh "$host" "bash -c 'SFYCHECK_IL=\$HOME/$work/schemify_l3.il bash $work/src_l3/run.sh l3_config'" >"$tmp/run.txt" 2>&1 &
run=$!
# Waits for a complete dump on the host and copies it here.
fetch() {
  for _ in $(seq 400); do
    ssh "$host" "bash -c 'test -s $work/out/$1 && tail -1 $work/out/$1 | grep -qx ..end..'" && break
    kill -0 $run 2>/dev/null || break
    sleep 3
  done
  scp -q "$host:$work/out/$1" "$tmp/$1"
}
# The config record's rules in a bench file (its attribute lines but vcfg/vcfgsum), sorted.
rules() { sed -n '/^config /,/^$/p' "$1" | grep -v '^config \|^  vcfg\|^$' | sort; }

verdict="FAIL l3_drive: no first dump"
if fetch l3_first.sfyx; then
  mkdir "$tmp/proj" "$tmp/back"
  cp "$tmp/l3_first.sfyx" "$tmp/proj/"
  "$bin" virtuoso-import "$tmp/proj" l3_first.sfyx >"$tmp/import.txt"
  bench="$lib/top.shbench"
  "$bin" virtuoso-export "$tmp/proj" "$bench" -o l3_second.sfyx >"$tmp/export.txt"
  sed -i 's/^  order .*/  order schematic spice/; s/^  stop \(.*\)/  cellinst .\/mid I2 spice\n  stop \1/' "$tmp/proj/$bench"
  rules "$tmp/proj/$bench" >"$tmp/edited.rules"
  "$bin" virtuoso-export "$tmp/proj" "$bench" -o l3_edited.sfyx >"$tmp/export_edited.txt"
  scp -q "$tmp/proj/l3_second.sfyx" "$host:$work/out/l3_second.tmp"
  scp -q "$tmp/proj/l3_edited.sfyx" "$host:$work/out/l3_edited.tmp"
  ssh "$host" "bash -c 'cd $work/out && mv l3_edited.tmp l3_edited.sfyx && mv l3_second.tmp l3_second.sfyx'"
  verdict="FAIL l3_drive: no fourth dump"
  if fetch l3_fourth.sfyx; then
    cp "$tmp/l3_fourth.sfyx" "$tmp/back/"
    "$bin" virtuoso-import "$tmp/back" l3_fourth.sfyx >"$tmp/import_fourth.txt"
    if diff "$tmp/edited.rules" <(rules "$tmp/back/$bench") >"$tmp/rules.diff"; then
      verdict="PASS l3_drive: the edited config, as hdb re-saved it, reads back to the edit"
    else
      verdict="FAIL l3_drive: the edited config read back differs: $(cat "$tmp/rules.diff")"
    fi
  fi
fi
status=0
wait $run || status=$?
cat "$tmp/run.txt"
echo "$verdict"
ssh "$host" "bash -c 'rm -f $work/out/l3_*'" || true
echo "l3_drive: dumps and projects in $tmp"
[[ $verdict == PASS* ]] || exit 1
exit $status
