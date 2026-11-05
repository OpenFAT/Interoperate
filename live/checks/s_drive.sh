#!/usr/bin/env bash
# Lane S live check driver (gaps round 2): runs s_more, then s_schematic, on the Virtuoso host in its own
# work directory (sfytest/checks-s, the library kept), netlists the kept cells with si
# (spectre, and auCdl in CDS_Netlisting_Mode=Analog, the recipe of the research probes'
# p_gapsnl.sh) under the run lock, deletes the library, and fetches the dump, the check's
# lines and the netlists into $SFYCHECK_LOCAL (default a fresh temporary directory):
#
#   SFY_HOST=<host> live/checks/s_drive.sh
#
# tests/virtuoso_gaps.rs grades the result: copy s_schematic.sfyx to
# tests/fixtures/virtuoso/gaps.sfyx and the si netlists to tests/fixtures/virtuoso/gaps/si/.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$here/../.."
host="${SFY_HOST:-}"
[[ -n $host ]] || { echo "usage: SFY_HOST=<host> $0" >&2; exit 2; }
work=sfytest/checks-s
lib=sfyprobe_s
tmp="${SFYCHECK_LOCAL:-$(mktemp -d)}"
mkdir -p "$tmp/il" "$tmp/si"
# CARGO: the cargo command, words split (`nix develop -c cargo`).
${CARGO:-cargo} run -q --release --features cli --bin schemify-interop --manifest-path "$repo/Cargo.toml" \
  -- install-skill "$tmp/il" >/dev/null
ssh "$host" bash -s <<PREP
mkdir -p $work/src $work/out && rm -f $work/out/s_schematic* $work/out/s_more*
[ -f $work/cds.lib ] || cp sfytest/checks/cds.lib $work/cds.lib
# The sheet borders (S5's multi-sheet schematic), beside analogLib's install.
grep -q '^DEFINE US_8ths ' $work/cds.lib || sed -n 's|^DEFINE analogLib \(.*\)/artist/analogLib|DEFINE US_8ths \1/sheets/US_8ths|p' $work/cds.lib >> $work/cds.lib
PREP
scp -q "$here"/*.il "$here"/run.sh "$host:$work/src/"
scp -q "$tmp/il/schemify.il" "$host:$work/schemify.il"
drop() {
  ssh "$host" sh -s <<DROP
rm -rf "\$HOME/$work/$lib" "\$HOME/$work/${lib}2" "\$HOME/$work/si"
sed -i '/^DEFINE $lib /d; /^DEFINE ${lib}2 /d' "\$HOME/$work/cds.lib"
DROP
}
trap drop EXIT
status=0
# s_more first: run.sh drops the lane library when a check starts, so s_schematic's kept
# library (for si below) must come last.
ssh "$host" "env SFYCHECK_WORK=\$HOME/$work bash $work/src/run.sh s_more" \
  >"$tmp/run_more.txt" 2>&1 || status=$?
cat "$tmp/run_more.txt"
ssh "$host" "env SFYCHECK_KEEP=1 SFYCHECK_WORK=\$HOME/$work bash $work/src/run.sh s_schematic" \
  >"$tmp/run.txt" 2>&1 || status=$?
cat "$tmp/run.txt"
# si of the kept cells, one Virtuoso tool at a time (the run lock).
ssh "$host" 'flock /tmp/`id -un`-sfy-run.lock csh -f -c "source ~/sfytest/env.csh >& /dev/null; bash -s"' \
  >"$tmp/si.txt" 2>&1 <<SI || true
W=\$HOME/$work; O=\$W/out/s_si; rm -rf "\$O"; mkdir -p "\$O"
for cell in conn alt ordtop e1 pc; do for sim in spectre auCdl; do
  d="\$W/si/\$cell.\$sim"; rm -rf "\$d"; mkdir -p "\$d"
  printf 'INCLUDE %s\n' "\$W/cds.lib" > "\$d/cds.lib"
  if [ \$sim = auCdl ]; then views="'(\"auCdl\" \"schematic\")"; stop="'(\"auCdl\")"; cmd=netlist
  else views="'(\"spectre\" \"schematic\")"; stop="'(\"spectre\")"; cmd=nl; fi
  cat > "\$d/si.env" <<E2
simLibName = "$lib"
simCellName = "\$cell"
simViewName = "schematic"
simSimulator = "\$sim"
simNotIncremental = 't
simReNetlistAll = 't
simViewList = \$views
simStopList = \$stop
simNetlistHier = 't
hnlNetlistFileName = "netlist"
E2
  [ \$sim = auCdl ] && cat >> "\$d/si.env" <<'E3'
resistorModel = ""
shortRES = 2000.0
preserveRES = 't
checkRESVAL = 't
checkRESSIZE = 'nil
preserveCAP = 't
checkCAPVAL = 't
checkCAPAREA = 'nil
checkCAPPERI = 'nil
preserveDIO = 't
checkDIOAREA = 't
checkDIOPERI = 't
simPrintInhConnAttributes = 'nil
checkScale = "meter"
checkLDD = 'nil
pinMAP = 'nil
preserveBangInNetlist = 'nil
shrinkFACTOR = 0.0
globalPowerSig = ""
globalGndSig = ""
displayPININFO = 't
preserveALL = 't
setEQUIV = ""
incFILE = ""
E3
  (cd "\$d" && CDS_Netlisting_Mode=Analog timeout 600 si -batch -command \$cmd > "\$O/\$cell.\$sim.log" 2>&1 </dev/null; echo "exit \$?" >> "\$O/\$cell.\$sim.log")
  cp "\$d/netlist" "\$O/\$cell.\$sim.net" 2>/dev/null
done; done
ls "\$O"
SI
cat "$tmp/si.txt"
scp -q "$host:$work/out/s_schematic.sfyx" "$host:$work/out/s_schematic.sfyx.log" "$host:$work/out/s_schematic.txt" \
  "$host:$work/out/s_more.sfyx" "$host:$work/out/s_more.sfyx.log" "$host:$work/out/s_more.txt" "$tmp/" || status=1
scp -q "$host:$work/out/s_si/*" "$tmp/si/" || true
echo "s_drive: files in $tmp"
exit $status
