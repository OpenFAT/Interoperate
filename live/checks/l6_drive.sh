#!/usr/bin/env bash
# Lane L6 live check driver (gap F): runs l6_multitest on the Virtuoso host and does its
# Schemify half here, with this checkout's CLI.
#
#   SFY_HOST=<host> live/checks/l6_drive.sh   (SFY_HOST: the Virtuoso host's ssh name, required)
#
# Ships live/checks (as <work>/src_l6) and this checkout's schemify.il (as
# <work>/schemify_l6.il) to the host, starts `run.sh l6_multitest` there, and when the
# session's first dump appears: imports it into a scratch Schemify project, requires the
# bench's test records (T3 disabled, T2's own C, the parameters, the cross-test output),
# exports it unchanged (l6_second.sfyx: only the tests' maestro records may be sent), then
# edits T2's C to 7n and exports again (l6_edited.sfyx: only T2's testvar may be sent).
# When the session's fourth dump (after loading the edit) arrives, imports it and requires
# its test records to be the first import's with T2's C 7n. Prints the check's lines and
# its own PASS/FAIL lines, removes the l6_* files from the host, and exits non-zero on a
# FAIL. Dumps and projects stay in $SFYCHECK_LOCAL (default a fresh temporary directory).
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$here/../.."
host="${SFY_HOST:-}"
[[ -n $host ]] || { echo "usage: SFY_HOST=<host> $0   (SFY_HOST: the Virtuoso host's ssh name)" >&2; exit 2; }
work=sfytest/checks
lib=sfyprobe_l6
tmp="${SFYCHECK_LOCAL:-$(mktemp -d)}"
mkdir -p "$tmp/il"

cargo build -q --features cli --bin schemify-interop --manifest-path "$repo/Cargo.toml"
bin="${CARGO_TARGET_DIR:-$repo/target}/debug/schemify-interop"
"$bin" install-skill "$tmp/il" >/dev/null
ssh "$host" "bash -c 'mkdir -p $work/src_l6 $work/out && rm -f $work/out/l6_*'"
scp -q "$here"/*.il "$here"/run.sh "$host:$work/src_l6/"
scp -q "$tmp/il/schemify.il" "$host:$work/schemify_l6.il"

ssh "$host" "bash -c 'SFYCHECK_IL=\$HOME/$work/schemify_l6.il bash $work/src_l6/run.sh l6_multitest'" >"$tmp/run.txt" 2>&1 &
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
# The bench's test records and the lines that carry its tests' setup.
tests() { sed -n '/^test /,/^$/p' "$1"; grep -E '^param |^  param |^output x2 |^output v2 ' "$1" || true; }
# The records of an .sfyx file from its first maestro record on, but maesdb, maestate and
# end: `<test> <tag> <first field>`.
sent() {
  awk -F'"' '/^\("maestro"/ {t=$4; next} t != "" && !/^\("(maesdb|maestate|end)"/ {print t, $2, $4}' "$1"
}
verdicts=()
check() { if eval "$2"; then verdicts+=("PASS l6_drive: $1"); else verdicts+=("FAIL l6_drive: $1"); fi; }

if fetch l6_first.sfyx; then
  mkdir "$tmp/proj" "$tmp/back"
  cp "$tmp/l6_first.sfyx" "$tmp/proj/"
  "$bin" virtuoso-import "$tmp/proj" l6_first.sfyx >"$tmp/import.txt"
  bench="$lib/rc.shbench"
  tests "$tmp/proj/$bench" >"$tmp/first.tests"
  check "three test records, T3 disabled" "grep -q '^test T3' $tmp/first.tests && sed -n '/^test T3/,/^test \|^$/p' $tmp/first.tests | grep -q '  enable false'"
  check "T2's own C" "sed -n '/^test T2/,/^test /p' $tmp/first.tests | grep -q '  set C 5n'"
  check "the parameter, globally and in corner hot" "grep -q '^param V0/v2 1' $tmp/first.tests && grep -q '^  param V0/v2 1.1' $tmp/first.tests"
  check "the cross-test and dependent outputs as result()" "grep -q 'output x2 .*result(.*vmax.*T1' $tmp/first.tests && grep -q 'output v2 .*result(.*vmax.*T1' $tmp/first.tests"
  "$bin" virtuoso-export "$tmp/proj" "$bench" -o l6_second.sfyx >"$tmp/export.txt"
  sent "$tmp/proj/l6_second.sfyx" >"$tmp/second.sent"
  check "unchanged: nothing but the tests is sent ($(wc -l <"$tmp/second.sent") records)" "test ! -s $tmp/second.sent"
  sed -i 's/^  set C 5n$/  set C 7n/' "$tmp/proj/$bench"
  "$bin" virtuoso-export "$tmp/proj" "$bench" -o l6_edited.sfyx >"$tmp/export_edited.txt"
  sent "$tmp/proj/l6_edited.sfyx" >"$tmp/edited.sent"
  check "edited: T2's testvar C alone is sent ($(tr '\n' ';' <"$tmp/edited.sent"))" "test \"\$(cat $tmp/edited.sent)\" = 'T2 testvar C'"
  scp -q "$tmp/proj/l6_second.sfyx" "$host:$work/out/l6_second.tmp"
  scp -q "$tmp/proj/l6_edited.sfyx" "$host:$work/out/l6_edited.tmp"
  ssh "$host" "bash -c 'cd $work/out && mv l6_edited.tmp l6_edited.sfyx && mv l6_second.tmp l6_second.sfyx'"
  if fetch l6_fourth.sfyx; then
    cp "$tmp/l6_fourth.sfyx" "$tmp/back/"
    "$bin" virtuoso-import "$tmp/back" l6_fourth.sfyx >"$tmp/import_fourth.txt"
    tests "$tmp/back/$bench" >"$tmp/fourth.tests"
    sed 's/^  set C 5n$/  set C 7n/' "$tmp/first.tests" >"$tmp/want.tests"
    check "after the load, the tests are the first import's with T2's C 7n" "diff $tmp/want.tests $tmp/fourth.tests >$tmp/tests.diff"
  else
    verdicts+=("FAIL l6_drive: no fourth dump")
  fi
else
  verdicts+=("FAIL l6_drive: no first dump")
fi
status=0
wait $run || status=$?
cat "$tmp/run.txt"
printf '%s\n' "${verdicts[@]}"
ssh "$host" "bash -c 'rm -f $work/out/l6_* $work/schemify_l6.il; rm -rf $work/src_l6'" || true
echo "l6_drive: dumps and projects in $tmp"
printf '%s\n' "${verdicts[@]}" | grep -q '^FAIL' && exit 1
exit $status
