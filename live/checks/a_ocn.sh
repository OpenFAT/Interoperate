#!/usr/bin/env bash
# a_ocn.sh: lane A (gaps round 2) OCEAN probe on a plain RC netlist, run on the Virtuoso host
# under the run lock (as run.sh holds it):
#   ssh <host> bash -s < live/checks/a_ocn.sh
# The analysis keywords the OCEAN script writes (noise output nodes p/n with outType, dc
# with a step, ac lin), the netlist OCEAN makes of them and Spectre's run, and the pieces of
# the exported settlingTime expression one by one, each with its error text. Writes
# $SFYCHECK_WORK/out/a_ocn/ (default work ~/sfytest/checks-a) and prints facts.txt and the
# analysis lines of each netlist.
set -u
W="${SFYCHECK_WORK:-$HOME/sfytest/checks-a}"
D="$W/out/a_ocn"; rm -rf "$D"; mkdir -p "$D/net"; cd "$D" || exit 1
printf 'simulator lang=spectre\nglobal 0\n' > net/netlistHeader
printf '\n' > net/netlistFooter
cat > net/netlist <<'E1'
V0 (in 0) vsource type=pulse val0=0 val1=1 delay=1u rise=1n fall=1n width=50u period=100u mag=1
R0 (in out) resistor r=Rb
C0 (out 0) capacitor c=C
E1
# One OCEAN session per analysis variant: name|analysis call|result name (its points are counted).
variants=(
  'nz_v|analysis('"'"'noise ?start "1" ?stop "1G" ?dec "10" ?outType "voltage" ?p "/out" ?n "/gnd!" ?iprobe "/V0")|noise'
  'nz_v0|analysis('"'"'noise ?start "1" ?stop "1G" ?dec "10" ?p "/out" ?n "/gnd!" ?iprobe "/V0")|noise'
  'nz_o|analysis('"'"'noise ?start "1" ?stop "1G" ?dec "10" ?oprobe "/out" ?iprobe "/V0")|noise'
  'dc_s|analysis('"'"'dc ?dev "/V0" ?param "dc" ?start "0" ?stop "1" ?step "0.1")|dc'
  'dc_l|analysis('"'"'dc ?dev "/V0" ?param "dc" ?start "0" ?stop "1" ?step "0.1" ?incrType "Linear" ?stepTypeLin "Step Size")|dc'
  'ac_l|analysis('"'"'ac ?start "1" ?stop "5" ?lin "4")|ac'
)
for v in "${variants[@]}"; do
  n="${v%%|*}"; rest="${v#*|}"; call="${rest%|*}"; res="${rest##*|}"
  cat > "$n.ocn" <<E2
simulator('spectre)
design("./net/netlist")
resultsDir("./res_$n")
desVar("Rb" 1000) desVar("C" 1e-9)
$call
fp = outfile("./facts_$n.txt" "w")
fprintf(fp "run %L\n" errset(run() t))
fprintf(fp "results %L\n" car(errset(results())))
fprintf(fp "points %L\n" errset(let((w) selectResult('$res) w = getData(car(outputs())) list(drVectorLength(drGetWaveformXVec(w)) drGetElem(drGetWaveformXVec(w) 0) drGetElem(drGetWaveformXVec(w) 1)))))
close(fp)
exit()
E2
  flock "/tmp/$(id -un)-sfy-run.lock" csh -f -c "source ~/sfytest/env.csh >& /dev/null; cd $D; timeout 600 ocean -nograph < $n.ocn >& $n.log"
  echo "== $n: $call"; cat "facts_$n.txt" 2>/dev/null
  cp net/input.scs "$n.scs"; grep -E '^(noise|dc|ac) ' -A1 "$n.scs"
  grep -E 'ERROR|SFE-' "$n.log" | head -5
done
[ -n "${SFYOCN_SKIP_ST:-}" ] && exit 0
# settlingTime, piece by piece, on one tran run.
cat > st.ocn <<'E3'
simulator('spectre)
design("./net/netlist")
resultsDir("./res_st")
desVar("Rb" 1000) desVar("C" 1e-9)
analysis('tran ?stop "20u")
fp = outfile("./facts_st.txt" "w")
procedure(chkTry(label fn) let((r) r = errset(funcall(fn) t)
  fprintf(fp "%s => %L %L\n" label car(r) if(r nil getqq(errset errset))) drain(fp)))
fprintf(fp "run %L\n" errset(run() t))
y = VT("/out")
chkTry("x0 ymin(xval)" lambda(() ymin(xval(y))))
chkTry("x1 ymax(xval)" lambda(() ymax(xval(y))))
chkTry("x0 xmin" lambda(() xmin(y)))
chkTry("x1 xmax" lambda(() xmax(y)))
chkTry("x0 car xvec" lambda(() drGetElem(drGetWaveformXVec(y) 0)))
chkTry("v1 value last" lambda(() value(y ymax(xval(y)))))
chkTry("v0 value first" lambda(() value(y ymin(xval(y)))))
chkTry("s1 plain" lambda(() settlingTime(y 0 t 2e-05 t 1)))
chkTry("s2 float x0" lambda(() settlingTime(y 0.0 t 2e-05 t 1)))
chkTry("s3 ymin x0" lambda(() settlingTime(y ymin(xval(y)) t 2e-05 t 1)))
chkTry("s4 ymax x1" lambda(() settlingTime(y 0 t ymax(xval(y)) t 1)))
chkTry("s5 pct expr" lambda(() settlingTime(y 0 t 2e-05 t 100*(0.01)/abs(value(y 2e-05)-value(y 0)))))
chkTry("s6 exported" lambda(() settlingTime(y ymin(xval(y)) t ymax(xval(y)) t 100*(0.01)/abs(value(y ymax(xval(y)))-value(y ymin(xval(y)))))))
chkTry("s7 xmin xmax" lambda(() settlingTime(y xmin(y) t xmax(y) t 100*(0.01)/abs(value(y xmax(y))-value(y xmin(y))))))
chkTry("s8 last x 1.999999e-05" lambda(() settlingTime(y 0 t 1.9999e-05 t 1)))
chkTry("s9 x1 value" lambda(() let((x1) x1 = ymax(xval(y)) list(x1 value(y x1) settlingTime(y 0 t x1 t 1)))))
chkTry("s10 multiple nil" lambda(() settlingTime(y 0 t 2e-05 t 1 nil)))
chkTry("s11 number of points" lambda(() drVectorLength(drGetWaveformXVec(y))))
chkTry("s12 last x exact" lambda(() let((v) v = drGetWaveformXVec(y) drGetElem(v drVectorLength(v)-1))))
chkTry("s13 at last x" lambda(() let((v) v = drGetWaveformXVec(y) settlingTime(y 0 t drGetElem(v drVectorLength(v)-1) t 1))))
chkTry("s14 at first x" lambda(() let((v) v = drGetWaveformXVec(y) settlingTime(y drGetElem(v 0) t 2e-05 t 1))))
chkTry("s15 riseTime" lambda(() riseTime(y 0 t 2e-05 t 10 90)))
chkTry("s16 bandwidth" lambda(() 1))
chkTry("s17 overshoot" lambda(() overshoot(y 0 t 2e-05 t)))
chkTry("s18 delay" lambda(() delay(?wf1 VT("/in") ?value1 0.5 ?edge1 "rising" ?nth1 1 ?wf2 y ?value2 0.5 ?edge2 "rising" ?nth2 1)))
fprintf(fp "end\n")
close(fp)
exit()
E3
flock "/tmp/$(id -un)-sfy-run.lock" csh -f -c "source ~/sfytest/env.csh >& /dev/null; cd $D; timeout 600 ocean -nograph < st.ocn >& st.log"
echo "== settlingTime"; cat facts_st.txt
