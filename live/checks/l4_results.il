;; l4_results.il: lane L4 live check, results to waves (sfy_results.il). In chkLib
;; (sfyprobe_l4, analogLib and basic only): an RC schematic, a maestro test T1 with a tran
;; and an ac, var C swept 1n,2n, corners Nominal and hot (85 C), outputs vmax and vmin and a
;; spec; one single run, then SfyAdeResults of its history into $SFYCHECK_WORK/out/l4, and
;; per exported point the point's own netlist re-run by Spectre as nutbin, the oracle the
;; Rust side compares against bit for bit (tests/maestro_results.rs, the ignored
;; `live_history_against_nutbin`). The simulation tree ($SFYCHECK_WORK/sim_l4) is removed
;; at the end; out/l4 stays for that comparison.
;;
;; The run waits with maeWaitUntilDone (never ?waitUntilDone t); run.sh's timeout bounds it.

chkW = getShellEnvVar("SFYCHECK_WORK")
chkOut = strcat(chkW "/out/l4")
chkSim = strcat(chkW "/sim_l4")
system(sprintf(nil "rm -rf '%s' '%s'; mkdir -p '%s'" chkOut chkSim chkOut))
envSetVal("asimenv.startup" "projectDir" 'string chkSim)

procedure(chkSet(inst name value)
  let((c)
    c = cdfFindParamByName(cdfGetInstCDF(inst) name)
    if(c then c~>value = value t else chkLine(sprintf(nil "no CDF param %s on %s" name inst~>name) nil))))

;; A wire stub from each named terminal's pin centre, labelled with its net (p_lib.il).
procedure(chkStub(cv inst nets)
  let((bb c p2 w)
    foreach(term inst~>master~>terminals
      when(assoc(term~>name nets)
        bb = dbTransformBBox(car(term~>pins)~>fig~>bBox inst~>transform)
        c = list((xCoord(car(bb)) + xCoord(cadr(bb))) / 2.0 (yCoord(car(bb)) + yCoord(cadr(bb))) / 2.0)
        p2 = list(xCoord(c) + 0.25 yCoord(c))
        w = schCreateWire(cv "draw" "full" list(c p2) 0.0625 0.0625 0.0)
        schCreateWireLabel(cv car(w) p2 cadr(assoc(term~>name nets)) "lowerLeft" "R0" "stick" 0.0625 nil)))))

;; ---- rc/schematic
let((cv v r c)
  cv = dbOpenCellViewByType(chkLib "rc" "schematic" "schematic" "w")
  v = schCreateInst(cv dbOpenCellViewByType("analogLib" "vpulse" "symbol") "V0" 0:0 "R0")
  r = schCreateInst(cv dbOpenCellViewByType("analogLib" "res" "symbol") "R0" 2:0 "R0")
  c = schCreateInst(cv dbOpenCellViewByType("analogLib" "cap" "symbol") "C0" 4:0 "R0")
  foreach(kv '(("v1" "0") ("v2" "1") ("td" "0") ("tr" "1n") ("tf" "1n") ("pw" "5u") ("per" "10u") ("acm" "1"))
    chkSet(v car(kv) cadr(kv)))
  chkSet(r "r" "1k")
  chkSet(c "c" "C")
  chkStub(cv v '(("PLUS" "in") ("MINUS" "gnd!")))
  chkStub(cv r '(("PLUS" "in") ("MINUS" "out")))
  chkStub(cv c '(("PLUS" "out") ("MINUS" "gnd!")))
  chkE("schCheck rc" list('schCheck cv))
  chkThat("rc schematic saved" dbSave(cv))
  dbClose(cv))

;; ---- maestro: T1 (tran + ac), C = 1n,2n, Nominal + hot, one single run
chkS = chkE("maeOpenSetup" list('maeOpenSetup chkLib "rc" "maestro" ?mode "a"))
chkHist = nil
when(chkS
  chkE("createTest" list('maeCreateTest "T1" ?lib chkLib ?cell "rc" ?view "schematic" ?simulator "spectre" ?session chkS))
  chkE("tran" list('maeSetAnalysis "T1" "tran" ?enable t
    ?options ''(("stop" "20u") ("strobeperiod" "0.2u") ("strobeoutput" "strobeonly")) ?session chkS))
  chkE("ac" list('maeSetAnalysis "T1" "ac" ?enable t
    ?options ''(("start" "1") ("stop" "1G") ("incrType" "Logarithmic") ("stepTypeLog" "Points Per Decade") ("dec" "5"))
    ?session chkS))
  chkE("var C" list('maeSetVar "C" "1n,2n" ?session chkS))
  chkE("corner hot" list('maeSetCorner "hot" ?enabled t ?session chkS))
  chkE("hot temperature" list('maeSetVar "temperature" "85" ?typeName "corner" ?typeValue ''("hot") ?session chkS))
  chkE("out vmax" list('maeAddOutput "vmax" "T1" ?outputType "point" ?expr "ymax(VT(\"/out\"))" ?session chkS))
  chkE("out vmin" list('maeAddOutput "vmin" "T1" ?outputType "point" ?expr "ymin(VT(\"/out\"))" ?session chkS))
  chkE("spec vmax" list('maeSetSpec "vmax" "T1" ?gt "0.5" ?session chkS))
  chkE("save" list('maeSaveSetup ?lib chkLib ?cell "rc" ?view "maestro" ?session chkS))
  chkHist = chkE("run" list('maeRunSimulation ?session chkS))
  chkE("wait" list('maeWaitUntilDone ''All))
  chkE("close" list('maeCloseSession ?session chkS ?forceClose t)))
chkThat("a history ran" stringp(chkHist))

;; ---- the dump, then the nutbin oracle of every exported point
when(stringp(chkHist)
  chkLine("history" chkHist)
  chkThat("SfyAdeResults PASS" SfyAdeResults(chkOut chkLib "rc" chkHist))
  let((p l rec jobs psf wf stem nl)
    p = infile(strcat(chkOut "/" chkHist ".sfyx"))
    when(p
      while(l = lineread(p)
        when(listp(l) && car(car(l)) == "job" jobs = cons(car(l) jobs)))
      close(p))
    jobs = reverse(jobs)
    chkLine("jobs" mapcar(lambda((j) list(nth(1 j) nth(2 j) nth(3 j) nth(4 j) nth(5 j))) jobs))
    chkThat("4 points (2 design points x Nominal, hot)" length(jobs) == 4)
    chkThat("every point exported its waveforms" forall(j jobs nth(5 j) != ""))
    foreach(j jobs
      psf = nth(4 j)
      wf = nth(5 j)
      when(wf != ""
        stem = substring(wf 1 strlen(wf) - 4)
        nl = strcat(substring(psf 1 strlen(psf) - 3) "netlist")
        system(sprintf(nil "cd '%s' && timeout 600 spectre -format nutbin -raw '%s/%s.nutbin' input.scs > '%s/%s.spectre.log' 2>&1"
          nl chkOut stem chkOut stem))
        chkThat(sprintf(nil "nutbin of %s %L %d" nth(1 j) nth(2 j) nth(3 j))
          isFile(sprintf(nil "%s/%s.nutbin" chkOut stem)))))))

system(sprintf(nil "rm -rf '%s'" chkSim))
