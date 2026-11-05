#!/usr/bin/env bash
# si netlists of lane P's kept scratch library (p_drive.sh runs it on the Virtuoso host,
# after `run.sh p_terms` with SFYCHECK_KEEP): every cell of $P_CELLS in spectre and auCdl
# (live/run.sh's si.env keys; auCdl with CDS_Netlisting_Mode Analog), one Virtuoso tool at
# a time under the run lock. Output: <work>/out/p_si/<cell>.<sim>.net, .log and .status.
set -uo pipefail
work="${SFYCHECK_WORK:-$HOME/sfytest/checks}"
envfile="${SFYCHECK_ENV:-$HOME/sfytest/env.csh}"
lib="${P_LIB:-sfyprobe_p}"
out="$work/out/p_si"
rm -rf "$out"; mkdir -p "$out"
for cell in ${P_CELLS:-inv_tb bulk res}; do
  for sim in spectre auCdl; do
    d="$work/si/$lib.$cell.$sim" command=nl
    rm -rf "$d"; mkdir -p "$d"
    printf 'INCLUDE %s\n' "$work/cds.lib" > "$d/cds.lib"
    if [[ $sim == auCdl ]]; then
      views="'(\"auCdl\" \"schematic\")"; stop="'(\"auCdl\")"; command=netlist
    else
      views="'(\"spectre\" \"schematic\" \"veriloga\")"; stop="'(\"spectre\")"
    fi
    cat > "$d/si.env" <<EOF
simLibName = "$lib"
simCellName = "$cell"
simViewName = "schematic"
simSimulator = "$sim"
simNotIncremental = 't
simReNetlistAll = 't
simViewList = $views
simStopList = $stop
simNetlistHier = 't
hnlNetlistFileName = "netlist"
EOF
    [[ $sim == auCdl ]] && cat >> "$d/si.env" <<'EOF'
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
EOF
    flock "/tmp/$(id -un)-sfy-run.lock" csh -f -c "source $envfile >& /dev/null; cd $d; \
      setenv CDS_Netlisting_Mode Analog; timeout 600 si -batch -command $command" \
      > "$out/$cell.$sim.log" 2>&1 < /dev/null
    echo "$?" > "$out/$cell.$sim.status"
    f="$(cd "$d" && ls netlist input.scs *.cdl 2>/dev/null | head -1)"
    [[ -n "$f" ]] && cp "$d/$f" "$out/$cell.$sim.net"
    echo "p_si: $cell $sim exit $(cat "$out/$cell.$sim.status")"
  done
done
