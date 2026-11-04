;; l11_probe.il: lane L11 probe (not a round trip): where a Maestro view keeps what the
;; axlExportSetup XML does not (run plans, reliability, setup states), how it stores a
;; disabled variable and a disabled output, and GUESSes G49 (samplingmode lds) and G50
;; (the local optimizer's algorithm key is effort). wrap.il has loaded common.il and
;; schemify.il and created chkLib (sfyprobe_l11). Output files: <out>/l11p_*.
;;
;; Builds rc/maestro from analogLib parts: T1 (tran) and T2 (a copy), vars A B (a bowl:
;; output cost = (A-3)^2 + (B-2)^2, minimized), rmc (reads dmc, which stats.scs varies),
;; K (disabled), outputs vmax, cost and dis (Plot off), a run plan R1, a reliability setup,
;; a setup state start (A=1 B=5). Then:
;;   1. the view's files (md5) and the setup XML; the getters of disabled items;
;;   2. axlImportSetup 'overwrite of the XML over the saved view: which files change;
;;   3. the view's files copied into a new view: do run plan, reliability, state read back;
;;   4. one single run (are K and dis netlisted/evaluated), one 3-point Monte Carlo with
;;      samplingmode lds (its netlist and log), and two Local Optimizations, effort fine
;;      and coarse (the points each visits, from the .rdb).

chkDir = strcat(getShellEnvVar("SFYCHECK_WORK") "/l11scratch")
chkOut = strcat(getShellEnvVar("SFYCHECK_WORK") "/out")
chkView = strcat(getShellEnvVar("SFYCHECK_WORK") "/" chkLib "/rc/maestro")
chkTags = '("vars" "tests" "parameters" "corners" "runoptions" "specs")
system(sprintf(nil "rm -rf %s && mkdir -p %s/sim; rm -f %s/l11p_*" chkDir chkDir chkOut))
chkE("projectDir" lambda(() envSetVal("asimenv.startup" "projectDir" 'string strcat(chkDir "/sim"))))
chkStats = strcat(chkDir "/stats.scs")
let((p)
  p = outfile(chkStats "w")
  fprintf(p "simulator lang=spectre\nparameters dmc=0\nstatistics {\n  process {\n    vary dmc dist=gauss std=1\n  }\n}\n")
  close(p))

procedure(chkSet(inst name value)
  let((c)
    c = cdfFindParamByName(cdfGetInstCDF(inst) name)
    if(c then c~>value = value t else chkLine(sprintf(nil "no CDF param %s on %s" name inst~>name) nil))))

procedure(chkStub(cv inst nets)
  let((bb c p2 w)
    foreach(term inst~>master~>terminals
      when(assoc(term~>name nets)
        bb = dbTransformBBox(car(term~>pins)~>fig~>bBox inst~>transform)
        c = list((xCoord(car(bb)) + xCoord(cadr(bb))) / 2.0 (yCoord(car(bb)) + yCoord(cadr(bb))) / 2.0)
        p2 = list(xCoord(c) + 0.25 yCoord(c))
        w = schCreateWire(cv "draw" "full" list(c p2) 0.0625 0.0625 0.0)
        schCreateWireLabel(cv car(w) p2 cadr(assoc(term~>name nets)) "lowerLeft" "R0" "stick" 0.0625 nil)))))

;; Runs shell command cmd, its output to <out>/l11p_<name>.txt.
procedure(chkSh(name cmd)
  chkLine(sprintf(nil "sh %s" name) system(sprintf(nil "(%s) > %s/l11p_%s.txt 2>&1" cmd chkOut name))))

;; The view's files with their md5, into <out>/l11p_files_<tag>.txt.
procedure(chkFiles(tag dir)
  chkSh(strcat("files_" tag) sprintf(nil "cd %s && find . -type f -o -type d | sort | while read f; do if [ -f \"$f\" ]; then echo \"$(md5sum < \"$f\" | cut -c1-12) $(stat -c %%s \"$f\") $f\"; else echo \"dir $f\"; fi; done" dir)))

;; Waits for the run of history h (string) to finish; its value.
procedure(chkWait(h)
  chkE(sprintf(nil "wait %L" h) lambda(() maeWaitUntilDone('All))))

;; ---- rc schematic
let((cv v r c)
  cv = dbOpenCellViewByType(chkLib "rc" "schematic" "schematic" "w")
  v = schCreateInst(cv dbOpenCellViewByType("analogLib" "vpulse" "symbol") "V0" 0:0 "R0")
  r = schCreateInst(cv dbOpenCellViewByType("analogLib" "res" "symbol") "R0" 2:0 "R0")
  c = schCreateInst(cv dbOpenCellViewByType("analogLib" "cap" "symbol") "C0" 4:0 "R0")
  foreach(kv '(("v1" "0") ("v2" "1") ("tr" "1n") ("tf" "1n") ("pw" "5u") ("per" "10u"))
    chkSet(v car(kv) cadr(kv)))
  chkSet(r "r" "rmc")
  chkSet(c "c" "B*1n")
  chkStub(cv v '(("PLUS" "in") ("MINUS" "gnd!")))
  chkStub(cv r '(("PLUS" "in") ("MINUS" "out")))
  chkStub(cv c '(("PLUS" "out") ("MINUS" "gnd!")))
  chkE("rc check" lambda(() schCheck(cv)))
  chkE("rc save" lambda(() dbSave(cv))))

;; ---- rc/maestro
chkS = chkE("open" lambda(() maeOpenSetup(chkLib "rc" "maestro" ?mode "a")))
chkSdb = chkE("sdb" lambda(() axlGetMainSetupDB(chkS)))
chkE("createTest T1" lambda(() maeCreateTest("T1" ?lib chkLib ?cell "rc" ?view "schematic" ?simulator "spectre" ?session chkS)))
chkE("modelFiles T1" lambda(() maeSetEnvOption("T1" ?options list(list("modelFiles" list(list(chkStats "")))) ?session chkS)))
chkE("tran T1" lambda(() maeSetAnalysis("T1" "tran" ?enable t ?options '(("stop" "20u")) ?session chkS)))
chkE("var A" lambda(() maeSetVar("A" "1" ?session chkS)))
chkE("var B" lambda(() maeSetVar("B" "5" ?session chkS)))
chkE("var rmc" lambda(() maeSetVar("rmc" "A*1k*(1+0.05*dmc)" ?session chkS)))
chkE("var K" lambda(() maeSetVar("K" "7" ?session chkS)))
chkE("out vmax" lambda(() maeAddOutput("vmax" "T1" ?outputType "point" ?expr "ymax(VT(\"/out\"))" ?session chkS)))
chkE("out cost" lambda(() maeAddOutput("cost" "T1" ?outputType "point" ?expr "((VAR(\"A\") - 3)*(VAR(\"A\") - 3) + (VAR(\"B\") - 2)*(VAR(\"B\") - 2))" ?session chkS)))
chkE("out dis (plot nil)" lambda(() maeAddOutput("dis" "T1" ?outputType "point" ?expr "ymin(VT(\"/out\"))" ?plot nil ?session chkS)))
chkE("out nosave (save nil)" lambda(() maeAddOutput("nosave" "T1" ?outputType "point" ?expr "ymin(VT(\"/in\"))" ?save nil ?session chkS)))
chkE("spec cost minimize" lambda(() maeSetSpec("cost" "T1" ?minimum "0" ?session chkS)))
chkE("spec vmax gt" lambda(() maeSetSpec("vmax" "T1" ?gt "0.1" ?session chkS)))
chkE("createTest T2 (copy)" lambda(() maeCreateTest("T2" ?sourceTest "T1" ?session chkS)))
chkE("save 1" lambda(() maeSaveSetup(?lib chkLib ?cell "rc" ?view "maestro" ?session chkS)))
chkFiles("plain" chkView)
chkSh("plain_sdb" sprintf(nil "cat %s/maestro.sdb" chkView))

;; ---- disabled variable K: every route the reference names
chkE("var K handle" lambda(() axlGetVar(chkSdb "K")))
chkE("axlGetEnabled var K" lambda(() axlGetEnabled(axlGetVar(chkSdb "K"))))
chkE("axlSetEnabled var K nil" lambda(() axlSetEnabled(axlGetVar(chkSdb "K") nil)))
chkE("axlGetEnabled var K after" lambda(() axlGetEnabled(axlGetVar(chkSdb "K"))))
chkE("variables (enabled default)" lambda(() maeGetSetup(?typeName "variables" ?session chkS)))
chkE("variables ?enabled t" lambda(() maeGetSetup(?typeName "variables" ?enabled t ?session chkS)))
chkE("variables ?enabled nil" lambda(() maeGetSetup(?typeName "variables" ?enabled nil ?session chkS)))
chkE("variables ?enabled all" lambda(() maeGetSetup(?typeName "variables" ?enabled 'all ?session chkS)))
chkE("axlGetAllVarsDisabled" lambda(() axlGetAllVarsDisabled(chkSdb)))
chkE("axlGetEnabledGlobalVarPerTest K T1" lambda(() axlGetEnabledGlobalVarPerTest(chkSdb "K" "T1")))
;; ---- disabled output: the outputs' fields
foreach(so maeGetTestOutputs("T1" ?session chkS)
  chkE(sprintf(nil "output %L" so~>name) lambda(() list(so~>name so~>plot so~>save so~>displayIt so~>evalType))))
chkE("output fields" lambda(() car(maeGetTestOutputs("T1" ?session chkS))~>?))

;; ---- run plan, reliability, setup state
chkE("maeCreateNewRun R1" lambda(() maeCreateNewRun(?session chkS ?runName "R1" ?newRunFromActiveSetup t)))
chkE("maeGetRunPlan" lambda(() maeGetRunPlan(?session chkS)))
chkE("maeAddRelxSetup rel1" lambda(() maeAddRelxSetup("rel1" "T1" "T2" ?session chkS)))
chkE("maeGetRelxSetups" lambda(() maeGetRelxSetups(?session chkS)))
chkE("maeSaveSetupState start" lambda(() maeSaveSetupState("start" ?tags '("vars") ?session chkS)))
chkE("setup states" lambda(() axlSetupStates(chkS)))
chkE("var A range" lambda(() maeSetVar("A" "0.5:0.1:6" ?session chkS)))
chkE("var B range" lambda(() maeSetVar("B" "0.5:0.1:6" ?session chkS)))
foreach(kv '(("Monte Carlo Sampling" "mcnumpoints" "3") ("Monte Carlo Sampling" "mcmethod" "process")
             ("Monte Carlo Sampling" "samplingmode" "lds") ("Monte Carlo Sampling" "montecarloseed" "4242")
             ("Local Optimization" "effort" "fine") ("Local Optimization" "numpoints" "10")
             ("Local Optimization" "startingstateorpoint" "Starting State"))
  chkE(sprintf(nil "axlPutRunOption %L" kv) lambda(() axlSetRunOptionValue(axlPutRunOption(chkSdb car(kv) cadr(kv)) caddr(kv)))))
chkE("maeSetStartingPointForOptimization" lambda(() maeSetStartingPointForOptimization(?setupState "start" ?runMode "Local Optimization" ?session chkS)))
chkE("save 2" lambda(() maeSaveSetup(?lib chkLib ?cell "rc" ?view "maestro" ?session chkS)))
chkE("axlExportSetup" lambda(() axlExportSetup(chkS chkSdb strcat(chkOut "/l11p_export.sdb") chkTags)))
chkFiles("full" chkView)
chkSh("full_sdb" sprintf(nil "cat %s/maestro.sdb" chkView))
chkSh("full_state_outputs" sprintf(nil "grep -n -E 'Name=\"(name|plot|save|displayIt)\"' %s/active.state" chkView))
chkSh("full_states" sprintf(nil "cd %s && ls -laR states test_states documents 2>&1; for f in $(find states test_states documents -type f 2>/dev/null); do echo \"== $f\"; cat \"$f\"; done" chkView))
chkE("close 1" lambda(() maeCloseSession(?session chkS ?forceClose t)))

;; ---- 2. axlImportSetup 'overwrite of the XML over the saved view
system(sprintf(nil "rm -rf %s/keep && cp -a %s %s/keep" chkDir chkView chkDir))
chkS = chkE("open 2" lambda(() maeOpenSetup(chkLib "rc" "maestro" ?mode "a")))
chkE("open+save only" lambda(() maeSaveSetup(?lib chkLib ?cell "rc" ?view "maestro" ?session chkS)))
chkFiles("opensave" chkView)
chkE("axlImportSetup overwrite" lambda(() axlImportSetup(chkS strcat(chkOut "/l11p_export.sdb") chkTags "" 'overwrite)))
chkE("save after import" lambda(() maeSaveSetup(?lib chkLib ?cell "rc" ?view "maestro" ?session chkS)))
chkE("runPlan after import" lambda(() maeGetRunPlan(?session chkS)))
chkE("relx after import" lambda(() maeGetRelxSetups(?session chkS)))
chkE("close 2" lambda(() maeCloseSession(?session chkS ?forceClose t)))
chkFiles("imported" chkView)
chkSh("diff_import" sprintf(nil "diff %s/keep/maestro.sdb %s/maestro.sdb; diff %s/keep/active.state %s/active.state | head -40" chkDir chkView chkDir chkView))

;; ---- 3. the files into a new view (cell rc2, same schematic through the files)
chkE("copy rc schematic to rc2" lambda(() dbCopyCellView(dbOpenCellViewByType(chkLib "rc" "schematic") chkLib "rc2" "schematic")))
chkE("rc2 maestro view (DD)" lambda(() ddGetObj(chkLib "rc2" "maestro" nil nil "a")))
system(sprintf(nil "cd %s/keep && for f in $(find . -type f ! -name '*.cdslck' ! -path './history/*' ! -path './results/*'); do mkdir -p \"$(dirname %s/../../rc2/maestro/$f)\"; cp \"$f\" %s/../../rc2/maestro/$f; done; sed -i 's#<value>rc</value>#<value>rc2</value>#' %s/../../rc2/maestro/maestro.sdb"
  chkDir chkView chkView chkView))
chkFiles("rc2" strcat(chkView "/../../rc2/maestro"))
chkS = chkE("open rc2" lambda(() maeOpenSetup(chkLib "rc2" "maestro" ?mode "a")))
chkE("rc2 tests" lambda(() maeGetSetup(?session chkS)))
chkE("rc2 runPlan" lambda(() maeGetRunPlan(?session chkS)))
chkE("rc2 relx" lambda(() maeGetRelxSetups(?session chkS)))
chkE("rc2 setup states" lambda(() axlSetupStates(chkS)))
chkE("rc2 vars" lambda(() maeGetSetup(?typeName "variables" ?enabled 'all ?session chkS)))
chkE("rc2 K enabled" lambda(() axlGetEnabled(axlGetVar(axlGetMainSetupDB(chkS) "K"))))
chkE("close rc2" lambda(() maeCloseSession(?session chkS ?forceClose t)))

;; ---- 4. runs
chkS = chkE("open 4" lambda(() maeOpenSetup(chkLib "rc" "maestro" ?mode "a")))
chkSdb = chkE("sdb 4" lambda(() axlGetMainSetupDB(chkS)))
chkE("T2 off" lambda(() maeSetSetup(?tests '("T2") ?enabled nil ?session chkS)))
chkE("A single" lambda(() maeSetVar("A" "1" ?session chkS)))
chkE("B single" lambda(() maeSetVar("B" "5" ?session chkS)))
chkH = chkE("run single" lambda(() maeRunSimulation(?runMode "Single Run, Sweeps and Corners" ?session chkS)))
chkWait(chkH)
chkSh("single_netlist" sprintf(nil "find %s/sim -name input.scs | head -3 | xargs grep -n -i 'parameters\\|K='" chkDir))
foreach(o '("vmax" "cost" "dis" "nosave")
  chkE(sprintf(nil "single value %s" o) lambda(() maeGetOutputValue(o "T1" ?historyName chkH))))
chkSh("single_rdb" sprintf(nil "sqlite3 %s/%s/rc/maestro/results/maestro/%s.rdb 'select r.name, v.pointID, v.value from result r join resultValue v on v.resultID = r.id;'" getShellEnvVar("SFYCHECK_WORK") chkLib chkH))

chkH = chkE("run MC lds" lambda(() maeRunSimulation(?runMode "Monte Carlo Sampling" ?session chkS)))
chkWait(chkH)
chkSh("mc_netlist" sprintf(nil "grep -r -n -i -E 'montecarlo|sampling' %s/sim --include=*.scs | head -20; grep -r -n -i -E 'sampling|lds|low.discrep|sobol' %s/sim --include=*.log --include=*.out | head -20; grep -n -i -E 'sampling|lds|discrep' %s/%s/rc/maestro/results/maestro/*.log | head -20"
  chkDir chkDir getShellEnvVar("SFYCHECK_WORK") chkLib))

;; Local Optimization, effort fine then coarse, each from the starting state.
chkE("A range" lambda(() maeSetVar("A" "0.5:0.1:6" ?session chkS)))
chkE("B range" lambda(() maeSetVar("B" "0.5:0.1:6" ?session chkS)))
foreach(eff '("fine" "coarse")
  chkE(sprintf(nil "effort %s" eff) lambda(() axlSetRunOptionValue(axlPutRunOption(chkSdb "Local Optimization" "effort") eff)))
  chkE("startingstateorpoint" lambda(() axlSetRunOptionValue(axlPutRunOption(chkSdb "Local Optimization" "startingstateorpoint") "Starting State")))
  chkE("start point" lambda(() maeSetStartingPointForOptimization(?setupState "start" ?runMode "Local Optimization" ?session chkS)))
  chkE("save opt" lambda(() maeSaveSetup(?lib chkLib ?cell "rc" ?view "maestro" ?session chkS)))
  chkH = chkE(sprintf(nil "run LocalOpt %s" eff) lambda(() maeRunSimulation(?runMode "Local Optimization" ?session chkS)))
  chkWait(chkH)
  chkSh(strcat("opt_" eff) sprintf(nil "sqlite3 %s/%s/rc/maestro/results/maestro/%s.rdb \"select pv.pointID, p.name, pv.value from parameterValue pv join parameter p on p.id = pv.parameterID where p.name in ('A','B') order by pv.pointID, p.name;\"; sqlite3 %s/%s/rc/maestro/results/maestro/%s.rdb '.tables'"
    getShellEnvVar("SFYCHECK_WORK") chkLib chkH getShellEnvVar("SFYCHECK_WORK") chkLib chkH)))
chkSh("opt_logs" sprintf(nil "cd %s/%s/rc/maestro/results/maestro && ls; for f in LocalOpt*.log; do echo \"== $f\"; head -80 $f; done" getShellEnvVar("SFYCHECK_WORK") chkLib))
chkE("save end" lambda(() maeSaveSetup(?lib chkLib ?cell "rc" ?view "maestro" ?session chkS)))
chkE("close 4" lambda(() maeCloseSession(?session chkS ?forceClose t)))
chkFiles("end" chkView)
chkPass("probe ran")

;; ---- cleanup (run.sh removes chkLib)
system(sprintf(nil "rm -rf %s /tmp/sfy_%s_maesdb.sdb" chkDir getShellEnvVar("USER")))
