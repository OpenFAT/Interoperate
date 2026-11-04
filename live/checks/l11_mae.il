;; l11_mae.il: lane L11 live check, a Maestro view's setup files and its disabled items
;; through Schemify and back (sfy_ade.il, virtuoso::ade::viewfiles and ::disabled, interop
;; spec 9.11). wrap.il has loaded common.il and schemify.il and created chkLib
;; (sfyprobe_l11). l11_probe.il found where the view keeps them.
;;
;; Builds rc/maestro in chkLib from analogLib parts only: test T1 (tran), vars A and K (K
;; disabled), outputs vmax and dis (dis with its Plot off), a run plan R1, a setup state
;; start (the local optimizer's starting point). Then:
;;   1. dump (SfyExport), Schemify import (the bench: K and dis enable false) and export
;;      (only the view's files go: no maesdb), load over the view: maestro.sdb,
;;      active.state and states/start.sdb byte for byte, and the run plan, the setup state,
;;      K disabled and dis Plot off read back;
;;   2. an edit in Schemify (var A 1 -> 2, dis enabled), exported and loaded: A is 2 and dis
;;      Plot on, and the run plan, the state and K's disable are still there.
;; Environment: SFYCHECK_CLI, the schemify-interop binary (default ~/sfytest/l11/schemify-interop).

chkCli = or(getShellEnvVar("SFYCHECK_CLI") strcat(getShellEnvVar("HOME") "/sfytest/l11/schemify-interop"))
chkDir = strcat(getShellEnvVar("SFYCHECK_WORK") "/l11scratch")
chkOut = strcat(getShellEnvVar("SFYCHECK_WORK") "/out")
chkView = strcat(getShellEnvVar("SFYCHECK_WORK") "/" chkLib "/rc/maestro")
system(sprintf(nil "rm -rf %s && mkdir -p %s/proj; rm -f %s/l11m_*" chkDir chkDir chkOut))
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

;; The md5 of each of chkFiles in the view, as one list.
procedure(chkSums()
  let((p line out)
    system(sprintf(nil "cd %s && md5sum active.state maestro.sdb states/start.sdb > %s/l11m_sums.txt 2>&1" chkView chkOut))
    p = infile(strcat(chkOut "/l11m_sums.txt"))
    when(p while(gets(line p) out = cons(line out)) close(p))
    reverse(out)))

;; What the view holds of the items under test: (run plan, setup states, disabled vars,
;; A's value, (output plot) per output of T1).
procedure(chkRead()
  let((s out)
    s = car(errset(maeOpenSetup(chkLib "rc" "maestro" ?mode "a")))
    when(s
      out = list(
        car(errset(maeGetRunPlan(?session s)))
        car(errset(axlSetupStates(s)))
        car(errset(maeGetSetup(?typeName "variables" ?enabled nil ?session s)))
        car(errset(maeGetVar("A" ?session s)))
        sort(mapcar(lambda((o) list(o~>name o~>plot)) car(errset(maeGetTestOutputs("T1" ?session s))))
          lambda((a b) alphalessp(car(a) car(b)))))
      errset(maeCloseSession(?session s ?forceClose t)))
    out))

;; Runs schemify-interop with args (in chkDir/proj); t on exit status 0.
procedure(chkCliRun(label args)
  let((st)
    st = system(sprintf(nil "cd %s/proj && %s %s > %s/l11m_%s.cli.txt 2>&1" chkDir chkCli args chkOut label))
    chkLine(sprintf(nil "schemify-interop %s" args) st)
    st == 0))

;; ---- rc schematic
let((cv v r c)
  cv = dbOpenCellViewByType(chkLib "rc" "schematic" "schematic" "w")
  v = schCreateInst(cv dbOpenCellViewByType("analogLib" "vpulse" "symbol") "V0" 0:0 "R0")
  r = schCreateInst(cv dbOpenCellViewByType("analogLib" "res" "symbol") "R0" 2:0 "R0")
  c = schCreateInst(cv dbOpenCellViewByType("analogLib" "cap" "symbol") "C0" 4:0 "R0")
  foreach(kv '(("v1" "0") ("v2" "1") ("tr" "1n") ("tf" "1n") ("pw" "5u") ("per" "10u"))
    chkSet(v car(kv) cadr(kv)))
  chkSet(r "r" "A*1k")
  chkSet(c "c" "1n")
  chkStub(cv v '(("PLUS" "in") ("MINUS" "gnd!")))
  chkStub(cv r '(("PLUS" "in") ("MINUS" "out")))
  chkStub(cv c '(("PLUS" "out") ("MINUS" "gnd!")))
  chkE("rc check" lambda(() schCheck(cv)))
  chkE("rc save" lambda(() dbSave(cv))))

;; ---- rc/maestro
chkS = chkE("open" lambda(() maeOpenSetup(chkLib "rc" "maestro" ?mode "a")))
chkSdb = chkE("sdb" lambda(() axlGetMainSetupDB(chkS)))
chkE("createTest T1" lambda(() maeCreateTest("T1" ?lib chkLib ?cell "rc" ?view "schematic" ?simulator "spectre" ?session chkS)))
chkE("tran T1" lambda(() maeSetAnalysis("T1" "tran" ?enable t ?options '(("stop" "20u")) ?session chkS)))
chkE("var A" lambda(() maeSetVar("A" "1" ?session chkS)))
chkE("var K" lambda(() maeSetVar("K" "7" ?session chkS)))
chkE("K disabled" lambda(() axlSetEnabled(axlGetVar(chkSdb "K") nil)))
chkE("out vmax" lambda(() maeAddOutput("vmax" "T1" ?outputType "point" ?expr "ymax(VT(\"/out\"))" ?session chkS)))
chkE("out dis (plot nil)" lambda(() maeAddOutput("dis" "T1" ?outputType "point" ?expr "ymin(VT(\"/out\"))" ?plot nil ?session chkS)))
chkE("run plan R1" lambda(() maeCreateNewRun(?session chkS ?runName "R1" ?newRunFromActiveSetup t)))
chkE("setup state start" lambda(() maeSaveSetupState("start" ?tags '("vars") ?session chkS)))
chkE("save" lambda(() maeSaveSetup(?lib chkLib ?cell "rc" ?view "maestro" ?session chkS)))
chkE("close" lambda(() maeCloseSession(?session chkS ?forceClose t)))
chkSums0 = chkLine("sums before" chkSums())
chkBefore = chkLine("before" chkRead())
chkThat("the view holds R1, start, K disabled and dis Plot off"
  car(chkBefore) == '("R1") && cadr(chkBefore) == '("start") && caddr(chkBefore) == '("K")
  && member('("dis" nil) nth(4 chkBefore)))

;; ---- 1. unchanged: the files alone, byte for byte
chkA = strcat(chkDir "/proj/a.sfyx")
chkThat("dump" SfyExport(chkA chkLib '("rc")))
chkThat("the dump holds maefile records for the setup files, states/start.sdb among them"
  zerop(system(sprintf(nil "grep -q '^(\"maefile\" \"maestro.sdb\" ' %s && grep -q '^(\"maefile\" \"states/start.sdb\" ' %s && ! grep -q '^(\"maefile\" \"history/' %s"
    chkA chkA chkA))))
system(sprintf(nil "grep -o '^(\"maefile\" \"[^\"]*\"' %s > %s/l11m_files.txt" chkA chkOut))
chkThat("schemify-interop virtuoso-import" chkCliRun("import" "virtuoso-import . a.sfyx"))
chkBench = sprintf(nil "%s/proj/%s/rc.shbench" chkDir chkLib)
system(sprintf(nil "cp %s %s/l11m_rc.shbench; cp %s/proj/a.sfyx %s/l11m_a.sfyx" chkBench chkOut chkDir chkOut))
chkThat("the bench: K and dis enable false, a vmaefile row per file"
  zerop(system(sprintf(nil "grep -A1 -x 'var K 7' %s | grep -qx '  enable false' && grep -A1 '^output dis ' %s | grep -qx '  enable false' && test $(grep -c '^  vmaefile ' %s) -eq $(grep -c '^(\"maefile\" ' %s)"
    chkBench chkBench chkBench chkA))))
chkThat("schemify-interop virtuoso-export" chkCliRun("export" sprintf(nil "virtuoso-export . %s/rc.shbench -o b.sfyx" chkLib)))
chkB = strcat(chkDir "/proj/b.sfyx")
chkThat("the unchanged export sends the files alone (no maesdb, no var)"
  zerop(system(sprintf(nil "! grep -q '^(\"maesdb\" ' %s && ! grep -q '^(\"var\" ' %s && test $(grep -c '^(\"maefile\" ' %s) -eq $(grep -c '^(\"maefile\" ' %s)" chkB chkB chkB chkA))))
chkThat("load over the view" SfyImport(chkB ?overwrite t))
system(sprintf(nil "cp %s.log %s/l11m_b.log" chkB chkOut))
chkThat("maestro.sdb, active.state and states/start.sdb byte for byte" chkSums() == chkSums0)
chkAfter1 = chkLine("after1" chkRead())
chkThat("after1: every item read back as before" chkAfter1 == chkBefore)

;; ---- 2. changed in Schemify: A 1 -> 2, dis enabled
system(sprintf(nil "sed -i 's/^var A 1$/var A 2/' %s && awk 'BEGIN{d=0} /^output dis /{d=1; print; next} d==1 && $0==\"  enable false\"{d=0; next} {d=0; print}' %s > %s.new && mv %s.new %s"
  chkBench chkBench chkBench chkBench chkBench))
chkThat("schemify-interop virtuoso-export (edited)" chkCliRun("export2" sprintf(nil "virtuoso-export . %s/rc.shbench -o c.sfyx" chkLib)))
chkC = strcat(chkDir "/proj/c.sfyx")
system(sprintf(nil "cp %s %s/l11m_c.sfyx" chkC chkOut))
chkThat("load the edit" SfyImport(chkC ?overwrite t))
system(sprintf(nil "cp %s.log %s/l11m_c.log; cp %s/active.state %s/l11m_after2.state; ls -la /tmp/sfy_%s_maesdb > %s/l11m_scratch.txt 2>&1"
  chkC chkOut chkView chkOut getShellEnvVar("USER") chkOut))
chkAfter2 = chkLine("after2" chkRead())
chkThat("after2: A is 2 and dis Plot on, R1, start and K's disable kept"
  chkAfter2 == list(car(chkBefore) cadr(chkBefore) caddr(chkBefore) "2" '(("dis" t) ("vmax" t))))

;; ---- cleanup (run.sh removes chkLib)
system(sprintf(nil "rm -rf %s %s/simulation/%s /tmp/sfy_%s_maesdb /tmp/sfy_%s_adel.b64" chkDir getShellEnvVar("HOME") chkLib getShellEnvVar("USER") getShellEnvVar("USER")))
