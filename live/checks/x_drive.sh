#!/usr/bin/env bash
# Lane X live check driver (xschem → Schemify → Virtuoso with interop.map re-mastering, and
# back to xschem): runs x_xs2v on the Virtuoso host and does the Schemify half here
# (tests/xschem_virtuoso.rs live_xs2v, run in `nix develop` for its real xschem).
#
#   SFY_HOST=<host> live/checks/x_drive.sh   (SFY_HOST: the Virtuoso host's ssh name, required)
#
# Work directory on the host: ~/sfytest/checks-x (its own cds.lib, from sfytest/checks/cds.lib
# when absent; the run lock is run.sh's, shared). Ships live/checks, pdkmap_kit.il (as
# x_kit.il), build.il (as x_build.il) and this checkout's schemify.il; starts `run.sh x_xs2v`
# with the scratch library sfyprobe_x kept; when the session's SfyCdfNames dump appears, runs
# the `prepare` step (pdk-map, the re-mastered export) and hands out/x_xs2v_in.sfyx back;
# after the session netlists sfyprobe_x/tb with `si` (spectre; the kit cell and the user
# primitive stop at their symbol, as the kit has no spectre view) under the run lock, runs the
# `verify` step on the session's SfyExport dump and si's netlist, and deletes the library.
# Exit 1 on any FAIL. Files stay in $SFYCHECK_LOCAL (default a fresh temporary directory).
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
host="${SFY_HOST:-}"
[[ -n $host ]] || { echo "usage: SFY_HOST=<host> $0   (SFY_HOST: the Virtuoso host's ssh name)" >&2; exit 2; }
work=sfytest/checks-x
src=$work/src
lib=sfyprobe_x
tmp="${SFYCHECK_LOCAL:-$(mktemp -d)}"
mkdir -p "$tmp/il"
step() { # prepare | verify
  (cd "$repo" && SCHEMIFY_XS2V_DIR="$tmp" SCHEMIFY_XS2V_STEP="$1" SCHEMIFY_XS2V_LIB="$lib" \
    nix develop -c cargo test -q --features cli --test xschem_virtuoso live_xs2v -- \
      --ignored --exact --nocapture)
}

(cd "$repo" && nix develop -c cargo build -q --features cli --bin schemify-interop)
"${CARGO_TARGET_DIR:-$repo/target}/debug/schemify-interop" install-skill "$tmp/il" >/dev/null
ssh "$host" bash -s <<SETUP
mkdir -p $src $work/out
[ -f $work/cds.lib ] || grep -v '^DEFINE sfyprobe_' sfytest/checks/cds.lib > $work/cds.lib
find $work/out -name 'x_xs2v*' -delete
SETUP
scp -q "$here"/*.il "$here"/run.sh "$host:$src/"
scp -q "$repo/tests/fixtures/skill/pdkmap_kit.il" "$host:$src/x_kit.il"
scp -q "$repo/tests/fixtures/skill/build.il" "$host:$src/x_build.il"
scp -q "$tmp/il/schemify.il" "$host:$work/schemify.il"

drop() {
  ssh "$host" bash -s <<DROP
rm -rf "\$HOME/$work/$lib"
sed -i '/^DEFINE $lib /d' "\$HOME/$work/cds.lib"
DROP
}
trap drop EXIT

ssh "$host" bash -s >"$tmp/run.txt" 2>&1 <<RUN &
env SFYCHECK_WORK=\$HOME/$work SFYCHECK_KEEP=1 bash $src/run.sh x_xs2v
RUN
run=$!
for _ in $(seq 300); do
  ssh "$host" bash -s <<WAIT && break
test -s $work/out/x_xs2v_cdf.sfyx && grep -q '^("end")' $work/out/x_xs2v_cdf.sfyx
WAIT
  kill -0 $run 2>/dev/null || break
  sleep 3
done
fails=0
if scp -q "$host:$work/out/x_xs2v_cdf.sfyx" "$tmp/cdf.sfyx"; then
  step prepare | tee "$tmp/prepare.txt" || fails=1
  scp -q "$tmp/in.sfyx" "$host:$work/out/x_xs2v_in.tmp"
  ssh "$host" bash -s <<MV
mv $work/out/x_xs2v_in.tmp $work/out/x_xs2v_in.sfyx
MV
else
  echo "FAIL driver: no SfyCdfNames dump from the session"; fails=1
fi
status=0
wait $run || status=$?
cat "$tmp/run.txt"
(( status == 0 )) || fails=1

# si (spectre) of tb, one Cadence tool at a time (the run lock).
ssh "$host" bash -s >"$tmp/si.txt" 2>&1 <<SI || true
d=\$HOME/$work/si; rm -rf "\$d"; mkdir -p "\$d"
printf 'INCLUDE %s\n' "\$HOME/$work/cds.lib" > "\$d/cds.lib"
cat > "\$d/si.env" <<'ENV'
simLibName = "$lib"
simCellName = "tb"
simViewName = "schematic"
simSimulator = "spectre"
simNotIncremental = 't
simReNetlistAll = 't
simViewList = '("spectre" "schematic" "symbol")
simStopList = '("spectre" "symbol")
simNetlistHier = 't
hnlNetlistFileName = "netlist"
ENV
flock /tmp/\$(id -un)-sfy-run.lock csh -f -c "source \$HOME/sfytest/env.csh >& /dev/null; cd \$d; timeout 600 si -batch -command nl"
echo "si exit \$?"
ls "\$d"
SI
cat "$tmp/si.txt"
scp -q "$host:$work/si/netlist" "$tmp/si.scs" || echo "FAIL driver: si wrote no netlist"
scp -q "$host:$work/out/x_xs2v_back.sfyx" "$tmp/back.sfyx" || echo "FAIL driver: no SfyExport dump"
if [[ -f $tmp/si.scs && -f $tmp/back.sfyx ]]; then
  step verify | tee "$tmp/verify.txt" || fails=1
else
  fails=1
fi
echo "x_drive: files in $tmp"
[[ $fails == 0 ]] || exit 1
