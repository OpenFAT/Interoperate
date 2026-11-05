#!/usr/bin/env bash
# Lane A (gaps round 2) live check driver: runs a_ade on the Virtuoso host and does its
# Schemify half here, with this checkout's CLI.
#
#   SFY_HOST=<host> live/checks/a_drive.sh   (SFY_HOST: the Virtuoso host's ssh name, required)
#
# Exports tests/fixtures/live/a_ade (bench a_tb) with virtuoso-export and ships it, its OCEAN
# script, live/checks and this checkout's schemify.il to the host's work directory
# (SFYCHECK_WORK there, default ~/sfytest/checks-a), runs `run.sh a_ade` (the run lock), then
# imports the session's dump out/a_dump.sfyx here, checks the bench it gives (the SPICE and
# Spectre forms, the test records, the report rows), netlists each of its tests with
# Schemify (tests/virtuoso_live.rs `a_dump_netlist`) and runs Spectre on every netlist on
# the host under the run lock. Prints the check's lines and its own PASS/FAIL lines; exits
# non-zero on a FAIL. Files stay in $SFYCHECK_LOCAL (default a fresh temporary directory).
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
host="${SFY_HOST:-}"
[[ -n $host ]] || { echo "usage: SFY_HOST=<host> $0" >&2; exit 2; }
work="${SFYCHECK_REMOTE:-sfytest/checks-a}"
tmp="${SFYCHECK_LOCAL:-$(mktemp -d)}"
mkdir -p "$tmp/il" "$tmp/proj" "$tmp/nl"

cargo build -q --release --features cli --bin schemify-interop --manifest-path "$repo/Cargo.toml"
bin="${CARGO_TARGET_DIR:-$repo/target}/release/schemify-interop"
"$bin" install-skill "$tmp/il" >/dev/null
cp "$repo"/tests/fixtures/live/a_ade/* "$tmp/proj/"
"$bin" virtuoso-export "$tmp/proj" a_tb.shbench -o a_in.sfyx >"$tmp/export.txt"
ssh "$host" "bash -c 'mkdir -p $work/src $work/out/a_in $work/out/a_nl && rm -rf $work/out/a_dump.sfyx $work/out/a_nl/*'"
scp -q "$here"/*.il "$here"/run.sh "$host:$work/src/"
scp -q "$tmp/il/schemify.il" "$host:$work/schemify.il"
scp -q "$tmp/proj/a_in.sfyx" "$host:$work/out/a_in.sfyx"
scp -q -r "$tmp/proj/ocean" "$host:$work/out/a_in/"
status=0
ssh "$host" "bash -c 'SFYCHECK_WORK=\$HOME/$work bash $work/src/run.sh a_ade'" >"$tmp/run.txt" 2>&1 || status=$?

verdicts=()
check() { if eval "$2"; then verdicts+=("PASS a_drive: $1"); else verdicts+=("FAIL a_drive: $1"); fi; }
if scp -q "$host:$work/out/a_dump.sfyx" "$tmp/a_dump.sfyx"; then
  SFY_A_DUMP="$tmp/a_dump.sfyx" SFY_A_CELL=sfyprobe_a/rc.shbench SFY_A_OUT="$tmp/nl" \
    cargo test -q --release --manifest-path "$repo/Cargo.toml" --test virtuoso_live a_dump_netlist -- --ignored >"$tmp/netlist.txt" 2>&1 \
    || verdicts+=("FAIL a_drive: Schemify netlists (see $tmp/netlist.txt)")
  mkdir -p "$tmp/imp"
  printf 'schemify 1 project\n' >"$tmp/imp/schemify.project"
  cp "$tmp/a_dump.sfyx" "$tmp/imp/"
  "$bin" virtuoso-import "$tmp/imp" a_dump.sfyx >"$tmp/import.txt"
  b="$tmp/imp/sfyprobe_a/rc.shbench"
  r="$tmp/imp/a_dump.report.txt"
  check "noise of p/n comes back as .noise v(out) V0" "grep -q '^  | .noise v(out) V0 dec 10 1 1g$' $b"
  check "noise lin=4 comes back as lin 5 (5 points)" "grep -q '^  | .noise v(out) V0 lin 5 1 5$' $b"
  check "stb is Spectre's statement" "grep -q '^  | stb stb start=1 stop=1G dec=10 probe=IPRB0$' $b"
  check "pss before pac" "sed -n '/^code PA spectre/,/^code /p' $b | grep -n . | grep -q '4:  | pac pac'"
  check "dc temperature sweep is param=temp" "grep -q '^  | dc dc param=temp start=-40 stop=125 step=5$' $b"
  check "TO's simulator options" "grep -q '^  | .options gmin=1e-14 reltol=1e-5$' $b"
  check "TO's temperature list: its first point" "sed -n '/^code TO/,/^code /p' $b | grep -q '^  | .temp -40$'"
  check "N1 does not run C2" "sed -n '/^test N1$/,/^test /p' $b | grep -q '^  corner C1$'"
  check "TR is bound to rr" "sed -n '/^test TR$/,/^test /p' $b | grep -q '^  design ./rr$'"
  check "reported: hspiceD, evalType, specs, temperature sweep, corner group" \
    "grep -q 'simulator hspiceD' $r && grep -q 'evaluated over corners' $r && grep -q 'spec of inf' $r && grep -q 'temperature sweep' $r && grep -q 'a corner group' $r"
  ls "$tmp/nl" >"$tmp/nl.txt"
  check "a Schemify netlist per test ($(tr '\n' ' ' <"$tmp/nl.txt"))" "ls $tmp/nl/*.scs >/dev/null 2>&1"
  if ls "$tmp/nl"/*.scs >/dev/null 2>&1; then
    scp -q "$tmp/nl"/*.scs "$host:$work/out/a_nl/"
    ssh "$host" bash -s >"$tmp/spectre.txt" 2>&1 <<EOF || true
cd \$HOME/$work/out/a_nl
for f in *.scs; do
  flock /tmp/\$(id -un)-sfy-run.lock csh -f -c "source ~/sfytest/env.csh >& /dev/null; cd \$HOME/$work/out/a_nl; timeout 300 spectre -format psfascii -raw \${f%.scs}.raw \$f >& \${f%.scs}.log"
  echo "\$f exit \$? \$(grep -cE 'ERROR' \${f%.scs}.log) errors"
done
EOF
    # Spectre judges the tests whose block is Spectre (a SPICE block is ngspice's: the
    # Spectre dialect leaves it out, netlist.md §7.6.11).
    while read -r f _ code n _; do
      grep -q "^code ${f%.scs} spectre" "$b" || continue
      check "Spectre runs Schemify's netlist $f (exit $code, $n ERROR lines)" "[ '$code' = 0 ] && [ '$n' = 0 ]"
    done < <(grep ' exit ' "$tmp/spectre.txt")
    # Ground: the D4 sweep's out is R0/R1's divider of V0's 1 V at every point.
    scp -q -r "$host:$work/out/a_nl/D4.raw" "$tmp/" 2>/dev/null || true
    cat "$tmp"/D4.raw/*.dc 2>/dev/null | grep '^"out" ' >"$tmp/ground.txt" || true
    check "gnd! is Spectre's ground in Schemify's netlist: out = 0.5 V ($(grep -m1 "[0-9]" "$tmp/ground.txt"))" \
      "grep -q '^\"out\" 5.000000000000000e-01' $tmp/ground.txt"
  fi
else
  verdicts+=("FAIL a_drive: no dump")
fi
cat "$tmp/run.txt"
printf '%s\n' "${verdicts[@]}"
echo "a_drive: files in $tmp"
printf '%s\n' "${verdicts[@]}" | grep -q '^FAIL' && exit 1
exit $status
