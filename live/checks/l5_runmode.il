;; l5_runmode.il: lane L5 live check, Maestro's Monte Carlo and optimizer setups through
;; Schemify and back (sfy_ade.il, virtuoso::ade::runmode, interop spec 9.8). wrap.il has
;; loaded common.il and schemify.il and created chkLib (sfyprobe_l5).
;;
;; Builds rc/maestro in chkLib from analogLib parts only: test T1 (tran), outputs and specs
;; (a weight, a tolerance), var C an optimizer range 1n:1n:3n, Monte Carlo options set
;; through axlPutRunOption (non-default), a global optimizer, and a local optimizer with a
;; starting point (a saved setup state with C = 2n). Then:
;;   0. the local optimizer's options and setup XML (where its algorithm and starting point
;;      live), and one 3-point Local Optimization run (maeWaitUntilDone, under run.sh's
;;      timeout) whose logs are searched for the algorithm's name. Live ICADVM20.1: the
;;      starting point is the run options startingstatename (which
;;      maeSetStartingPointForOptimization sets) and startingstateorpoint "Starting State"
;;      (which it does not set: without it the run refuses, ASSEMBLER-9077); no option or
;;      file names the algorithm, which the adexl form keeps as effort (fine Brent-Powell,
;;      coarse Hooke-Jeeves);
;;   1. dump (SfyExport), Schemify import and export (schemify-interop), load over the view:
;;      every run option, spec and var reads back as before;
;;   2. one Monte Carlo option changed in Schemify (samples 7 -> 9), exported and loaded:
;;      that option alone changed.
;; The XML before and after each step is copied to the out directory (l5_*.sdb).
;; Environment: SFYCHECK_CLI, the schemify-interop binary (default ~/sfytest/l5/schemify-interop).

chkCli = or(getShellEnvVar("SFYCHECK_CLI") strcat(getShellEnvVar("HOME") "/sfytest/l5/schemify-interop"))
chkDir = strcat(getShellEnvVar("SFYCHECK_WORK") "/l5scratch")
chkOut = strcat(getShellEnvVar("SFYCHECK_WORK") "/out")
chkTags = '("vars" "tests" "parameters" "corners" "runoptions" "specs")
chkModes = '("Monte Carlo Sampling" "Global Optimization" "Local Optimization")
system(sprintf(nil "rm -rf %s && mkdir -p %s/proj" chkDir chkDir))
chkLine("cli" chkCli)

procedure(chkSet(inst name value)
  let((c)
    c = cdfFindParamByName(cdfGetInstCDF(inst) name)
    if(c then c~>value = value t else chkLine(sprintf(nil "no CDF param %s on %s" name inst~>name) nil))))

;; A wire stub from each named terminal's pin centre, labelled with its net.
procedure(chkStub(cv inst nets)
  let((bb c p2 w)
    foreach(term inst~>master~>terminals
      when(assoc(term~>name nets)
        bb = dbTransformBBox(car(term~>pins)~>fig~>bBox inst~>transform)
        c = list((xCoord(car(bb)) + xCoord(cadr(bb))) / 2.0 (yCoord(car(bb)) + yCoord(cadr(bb))) / 2.0)
        p2 = list(xCoord(c) + 0.25 yCoord(c))
        w = schCreateWire(cv "draw" "full" list(c p2) 0.0625 0.0625 0.0)
        schCreateWireLabel(cv car(w) p2 cadr(assoc(term~>name nets)) "lowerLeft" "R0" "stick" 0.0625 nil)))))

;; ---- rc schematic
let((cv v r c)
  cv = dbOpenCellViewByType(chkLib "rc" "schematic" "schematic" "w")
  v = schCreateInst(cv dbOpenCellViewByType("analogLib" "vpulse" "symbol") "V0" 0:0 "R0")
  r = schCreateInst(cv dbOpenCellViewByType("analogLib" "res" "symbol") "R0" 2:0 "R0")
  c = schCreateInst(cv dbOpenCellViewByType("analogLib" "cap" "symbol") "C0" 4:0 "R0")
  foreach(kv '(("v1" "0") ("v2" "1") ("tr" "1n") ("tf" "1n") ("pw" "5u") ("per" "10u"))
    chkSet(v car(kv) cadr(kv)))
  chkSet(r "r" "1k")
  chkSet(c "c" "C")
  chkStub(cv v '(("PLUS" "in") ("MINUS" "gnd!")))
  chkStub(cv r '(("PLUS" "in") ("MINUS" "out")))
  chkStub(cv c '(("PLUS" "out") ("MINUS" "gnd!")))
  chkE("rc check" lambda(() schCheck(cv)))
  chkE("rc save" lambda(() dbSave(cv))))

;; The run options of every mode of setup database sdb: ((mode ((name value) ...)) ...).
procedure(chkRunOpts(sdb)
  mapcar(lambda((m) list(m mapcar(lambda((n) list(n car(errset(axlGetRunOptionValue(axlGetRunOption(sdb m n))))))
                                 cadr(car(errset(axlGetRunOptions(sdb m)))))))
    chkModes))

;; ---- rc/maestro
chkS = chkE("open" lambda(() maeOpenSetup(chkLib "rc" "maestro" ?mode "a")))
chkSdb = chkE("sdb" lambda(() axlGetMainSetupDB(chkS)))
chkE("createTest T1" lambda(() maeCreateTest("T1" ?lib chkLib ?cell "rc" ?view "schematic" ?simulator "spectre" ?session chkS)))
chkE("tran T1" lambda(() maeSetAnalysis("T1" "tran" ?enable t ?options '(("stop" "20u") ("errpreset" "conservative")) ?session chkS)))
chkE("var C start" lambda(() maeSetVar("C" "2n" ?session chkS)))
chkE("out vmax" lambda(() maeAddOutput("vmax" "T1" ?outputType "point" ?expr "ymax(VT(\"/out\"))" ?session chkS)))
chkE("out vmin" lambda(() maeAddOutput("vmin" "T1" ?outputType "point" ?expr "ymin(VT(\"/out\"))" ?session chkS)))
chkE("spec vmax gt weight" lambda(() maeSetSpec("vmax" "T1" ?gt "0.9" ?weight "2" ?session chkS)))
chkE("spec vmin tolerance" lambda(() maeSetSpec("vmin" "T1" ?tolerance '("0.1" "10%") ?session chkS)))
chkE("save (start)" lambda(() maeSaveSetup(?lib chkLib ?cell "rc" ?view "maestro" ?session chkS)))
;; The starting point: a setup state holding C = 2n, saved before C becomes a range.
chkE("maeSaveSetupState start" lambda(() maeSaveSetupState("start" ?tags '("vars") ?session chkS)))
chkE("setup states" lambda(() axlSetupStates(chkS)))
chkE("var C range" lambda(() maeSetVar("C" "1n:1n:3n" ?session chkS)))
foreach(kv '(("Monte Carlo Sampling" "mcnumpoints" "7") ("Monte Carlo Sampling" "mcmethod" "process")
             ("Monte Carlo Sampling" "samplingmode" "lhs") ("Monte Carlo Sampling" "montecarloseed" "4242")
             ("Monte Carlo Sampling" "mcstartingrunnumber" "3") ("Monte Carlo Sampling" "donominal" "0")
             ("Monte Carlo Sampling" "mcnumbins" "5") ("Monte Carlo Sampling" "saveallplots" "1")
             ("Global Optimization" "numpoints" "50") ("Global Optimization" "timelimit" "7")
             ("Local Optimization" "effort" "fine") ("Local Optimization" "numpoints" "3")
             ("Local Optimization" "startingstateorpoint" "Starting State"))
  chkE(sprintf(nil "axlPutRunOption %L" kv) lambda(() axlSetRunOptionValue(axlPutRunOption(chkSdb car(kv) cadr(kv)) caddr(kv)))))
chkE("maeSetStartingPointForOptimization" lambda(() maeSetStartingPointForOptimization(?setupState "start" ?runMode "Local Optimization" ?session chkS)))
chkE("save" lambda(() maeSaveSetup(?lib chkLib ?cell "rc" ?view "maestro" ?session chkS)))

;; ---- 0. where the local optimizer keeps its algorithm and starting point
chkLine("run options" chkRunOpts(chkSdb))
chkE("axlExportSetup probe" lambda(() axlExportSetup(chkS chkSdb strcat(chkOut "/l5_probe.sdb") chkTags)))
system(sprintf(nil "cd %s/%s/rc/maestro && cp active.state %s/l5_probe.state; ls -R > %s/l5_probe_view.txt; cat states/* > %s/l5_probe_states.txt 2>&1"
  getShellEnvVar("SFYCHECK_WORK") chkLib chkOut chkOut chkOut))
chkH = chkE("run Local Optimization" lambda(() maeRunSimulation(?runMode "Local Optimization" ?session chkS)))
chkThat("the Local Optimization with a starting point starts" stringp(chkH))
chkE("wait" lambda(() maeWaitUntilDone('All)))
chkE("best points" lambda(() maeGetNBestDesignPoints(?count 1)))
chkE("run mode after the run" lambda(() maeGetCurrentRunMode(?session chkS)))
chkE("save after the run" lambda(() maeSaveSetup(?lib chkLib ?cell "rc" ?view "maestro" ?session chkS)))
chkE("close" lambda(() maeCloseSession(?session chkS ?forceClose t)))
;; Every line of the run's files that names a local algorithm.
system(sprintf(nil "cd %s && cp %s/rc/maestro/results/maestro/LocalOpt.0.log %s/l5_LocalOpt.0.log; grep -rIi -o -E '.{0,50}(brent|powell|hooke|jeeves|bfgs|conjugate).{0,50}' %s %s/simulation/%s 2>/dev/null | sort -u | head -60 > %s/l5_algo.txt"
  getShellEnvVar("SFYCHECK_WORK") chkLib chkOut chkLib getShellEnvVar("HOME") chkLib chkOut))
chkE("algorithm lines" lambda(() let((p line n) n = 0 p = infile(strcat(chkOut "/l5_algo.txt"))
  when(p while(gets(line p) n = n + 1 chkLine("algo" line)) close(p)) n)))

;; ---- what the view holds, read back
;; The run options, specs, vars and run mode of rc/maestro, as one list; the XML copied to
;; <out>/l5_<tag>.sdb.
procedure(chkRead(tag)
  let((s sdb out)
    s = car(errset(maeOpenSetup(chkLib "rc" "maestro" ?mode "a")))
    sdb = s && car(errset(axlGetMainSetupDB(s)))
    when(sdb
      out = list(
        list("tests" sort(copy(car(errset(maeGetSetup(?session s)))) 'alphalessp))
        list("vars" mapcar(lambda((n) list(n car(errset(maeGetVar(n ?session s)))))
                          sort(copy(car(errset(maeGetSetup(?typeName "variables" ?session s)))) 'alphalessp)))
        list("run mode" car(errset(maeGetCurrentRunMode(?session s))))
        list("specs" mapcar(lambda((o) list(o car(errset(axlGetSpecData(sdb o "T1")))
                                             car(errset(axlGetSpecWeight(axlGetSpec(sdb strcat("T1." o)))))))
                            '("vmax" "vmin"))))
      out = append(out chkRunOpts(sdb))
      errset(axlExportSetup(s sdb sprintf(nil "%s/l5_%s.sdb" chkOut tag) chkTags)))
    when(s errset(maeCloseSession(?session s ?forceClose t)))
    out))

;; Compares the read-back got of tag with want: PASS, or FAIL with each differing part.
procedure(chkSame(tag got want)
  let((bad)
    bad = append(setof(x want !member(x got)) setof(x got !member(x want)))
    unless(bad chkPass(sprintf(nil "%s: every setting read back as expected" tag)))
    foreach(x bad
      chkFail(sprintf(nil "%s: %s is %L, expected %L" tag car(x) cadr(assoc(car(x) got)) cadr(assoc(car(x) want)))))
    null(bad)))

;; Runs schemify-interop with args (in chkDir/proj); t on exit status 0.
procedure(chkCliRun(label args)
  let((st)
    st = system(sprintf(nil "cd %s/proj && %s %s > %s/l5_%s.cli.txt 2>&1" chkDir chkCli args chkOut label))
    chkLine(sprintf(nil "schemify-interop %s" args) st)
    st == 0))

chkBefore = chkRead("before")
chkLine("before" chkBefore)

;; ---- 1. dump, Schemify, load over the view
chkA = strcat(chkDir "/proj/a.sfyx")
chkThat("dump" SfyExport(chkA chkLib '("rc")))
chkThat("schemify-interop virtuoso-import" chkCliRun("import" "virtuoso-import . a.sfyx"))
chkBench = sprintf(nil "%s/proj/%s/rc.shbench" chkDir chkLib)
system(sprintf(nil "cp %s %s/l5_rc.shbench; cp %s/proj/a.sfyx %s/proj/a.report.md %s/" chkBench chkOut chkDir chkDir chkOut))
chkThat("the bench holds montecarlo mc, optimize global and optimize local"
  zerop(system(sprintf(nil "grep -qx 'montecarlo mc' %s && grep -qx 'optimize global' %s && grep -qx 'optimize local' %s && grep -qx '  samples 7' %s && grep -qx '  algo brent-powell' %s"
    chkBench chkBench chkBench chkBench chkBench))))
chkThat("schemify-interop virtuoso-export" chkCliRun("export" sprintf(nil "virtuoso-export . %s/rc.shbench -o b.sfyx" chkLib)))
chkThat("load over the view" SfyImport(strcat(chkDir "/proj/b.sfyx") ?overwrite t))
system(sprintf(nil "cp %s/proj/b.sfyx.log %s/l5_b.log" chkDir chkOut))
chkSame("after1" chkRead("after1") chkBefore)

;; ---- 2. one Monte Carlo option changed in Schemify
system(sprintf(nil "sed -i 's/^  samples 7$/  samples 9/' %s" chkBench))
chkThat("schemify-interop virtuoso-export (edited)" chkCliRun("export2" sprintf(nil "virtuoso-export . %s/rc.shbench -o c.sfyx" chkLib)))
chkThat("load the edit" SfyImport(strcat(chkDir "/proj/c.sfyx") ?overwrite t))
system(sprintf(nil "cp %s/proj/c.sfyx.log %s/l5_c.log" chkDir chkOut))
chkSame("after2" chkRead("after2")
  mapcar(lambda((x) if(car(x) == "Monte Carlo Sampling"
                      list(car(x) subst('("mcnumpoints" "9") '("mcnumpoints" "7") cadr(x)))
                      x))
    chkBefore))
system(sprintf(nil "cd %s && diff l5_after1.sdb l5_after2.sdb > l5_after2.diff; echo \"after2 diff lines: $(wc -l < l5_after2.diff)\" >> %s/l5_after2.diff"
  chkOut chkOut))

;; ---- cleanup (run.sh removes chkLib)
system(sprintf(nil "rm -rf %s %s/simulation/%s /tmp/sfy_%s_maesdb.sdb" chkDir getShellEnvVar("HOME") chkLib getShellEnvVar("USER")))
