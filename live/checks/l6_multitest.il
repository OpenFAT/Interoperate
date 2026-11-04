;; l6_multitest.il: lane L6 live check, a Maestro view of several tests through Schemify
;; and back (sfy_ade.il, interop spec 9). Run through l6_drive.sh, which does the Schemify
;; half on its own machine. wrap.il has loaded common.il and schemify.il and created chkLib
;; (sfyprobe_l6).
;;
;; Builds rc/maestro in chkLib from analogLib parts only: tests T1 (tran), T2 (ac) and T3
;; (tran, disabled); global vars C, K, rmc and T2's own C=5n; the parameter V0/v2 globally
;; and in corner hot; corner cold disabled; outputs vmax and its dependent v2 on T1, and on
;; T2 x2, which reads T1's vmax (calcVal). Dumps it to out/l6_first.sfyx and waits for the
;; driver's out/l6_second.sfyx (Interop import and export, unchanged) and l6_edited.sfyx
;; (T2's C edited to 7n in Schemify). Loads the second and requires every setting to read
;; back as before; loads the edited one and requires only T2's C to change; then dumps
;; out/l6_fourth.sfyx for the driver. No simulation runs.

chkTags = '("vars" "tests" "parameters" "corners" "runoptions" "specs")
chkTests = '("T1" "T2" "T3")
chkParam = strcat(chkLib "/rc/schematic/V0/v2")

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

;; Waits up to secs seconds for every file of paths; t when they all exist.
procedure(chkWait(paths secs)
  let((n)
    n = 0
    while(and(n < secs !forall(f paths isFile(f)))
      ipcSleep(2)
      n = n + 2)
    forall(f paths isFile(f))))

;; ---- rc schematic
let((cv v r c)
  cv = dbOpenCellViewByType(chkLib "rc" "schematic" "schematic" "w")
  v = schCreateInst(cv dbOpenCellViewByType("analogLib" "vpulse" "symbol") "V0" 0:0 "R0")
  r = schCreateInst(cv dbOpenCellViewByType("analogLib" "res" "symbol") "R0" 2:0 "R0")
  c = schCreateInst(cv dbOpenCellViewByType("analogLib" "cap" "symbol") "C0" 4:0 "R0")
  foreach(kv '(("v1" "0") ("v2" "1") ("tr" "1n") ("tf" "1n") ("pw" "5u") ("per" "10u") ("acm" "1"))
    chkSet(v car(kv) cadr(kv)))
  chkSet(r "r" "rmc")
  chkSet(c "c" "C")
  chkStub(cv v '(("PLUS" "in") ("MINUS" "gnd!")))
  chkStub(cv r '(("PLUS" "in") ("MINUS" "out")))
  chkStub(cv c '(("PLUS" "out") ("MINUS" "gnd!")))
  chkE("rc check" lambda(() schCheck(cv)))
  chkE("rc save" lambda(() dbSave(cv))))

;; ---- rc/maestro
chkS = chkE("open" lambda(() maeOpenSetup(chkLib "rc" "maestro" ?mode "a")))
foreach(tn chkTests
  chkE(strcat("createTest " tn) lambda(() maeCreateTest(tn ?lib chkLib ?cell "rc" ?view "schematic" ?simulator "spectre" ?session chkS))))
chkE("tran T1" lambda(() maeSetAnalysis("T1" "tran" ?enable t ?options '(("stop" "20u")) ?session chkS)))
chkE("ac T2" lambda(() maeSetAnalysis("T2" "ac" ?enable t ?options '(("start" "1") ("stop" "1G") ("incrType" "Logarithmic") ("stepTypeLog" "Points Per Decade") ("dec" "5")) ?session chkS)))
chkE("tran T3" lambda(() maeSetAnalysis("T3" "tran" ?enable t ?options '(("stop" "10u")) ?session chkS)))
chkE("var C" lambda(() maeSetVar("C" "1n" ?session chkS)))
chkE("var K" lambda(() maeSetVar("K" "7" ?session chkS)))
chkE("var rmc" lambda(() maeSetVar("rmc" "1k" ?session chkS)))
chkE("var C test T2" lambda(() maeSetVar("C" "5n" ?typeName "test" ?typeValue '("T2") ?session chkS)))
chkE("corner hot" lambda(() maeSetCorner("hot" ?enabled t ?session chkS)))
chkE("corner hot temp" lambda(() maeSetVar("temperature" "85" ?typeName "corner" ?typeValue '("hot") ?session chkS)))
chkE("corner cold" lambda(() maeSetCorner("cold" ?enabled nil ?session chkS)))
chkE("corner cold temp" lambda(() maeSetVar("temperature" "-40" ?typeName "corner" ?typeValue '("cold") ?session chkS)))
chkE("parameter" lambda(() maeSetParameter(chkParam "1" ?session chkS)))
chkE("parameter hot" lambda(() maeSetParameter(chkParam "1.1" ?typeName "corner" ?typeValue '("hot") ?session chkS)))
chkE("out vmax" lambda(() maeAddOutput("vmax" "T1" ?outputType "point" ?expr "ymax(VT(\"/out\"))" ?session chkS)))
chkE("out v2 (dependent)" lambda(() maeAddOutput("v2" "T1" ?outputType "point" ?expr "vmax*2" ?session chkS)))
chkE("out x2 (T1's vmax)" lambda(() maeAddOutput("x2" "T2" ?outputType "point" ?expr "calcVal(\"vmax\" \"T1\")*2" ?session chkS)))
chkE("out gain" lambda(() maeAddOutput("gain" "T2" ?outputType "point" ?expr "ymax(mag(VF(\"/out\")))" ?session chkS)))
chkE("disable T3" lambda(() maeSetSetup(?tests '("T3") ?enabled nil ?session chkS)))
chkE("save" lambda(() maeSaveSetup(?lib chkLib ?cell "rc" ?view "maestro" ?session chkS)))
chkE("close" lambda(() maeCloseSession(?session chkS ?forceClose t)))

;; ---- what the view holds, read back
;; Every setting the getters answer about rc/maestro, as (name value) pairs; the setup XML
;; is copied to out/l6_<tag>.sdb.
procedure(chkRead(tag)
  let((s sdb out)
    s = car(errset(maeOpenSetup(chkLib "rc" "maestro" ?mode "a")))
    sdb = s && car(errset(axlGetMainSetupDB(s)))
    when(sdb
      out = list(
        list("tests" sort(copy(car(errset(maeGetSetup(?session s)))) 'alphalessp))
        list("enabled" mapcar(lambda((tn) list(tn car(errset(axlGetEnabled(axlGetTest(sdb tn)))))) chkTests))
        list("corners" mapcar(lambda((c) list(c car(errset(axlGetEnabled(axlGetCorner(sdb c)))))) '("hot" "cold")))
        list("vars" mapcar(lambda((n) list(n car(errset(maeGetVar(n ?session s)))))
                          sort(copy(car(errset(maeGetSetup(?typeName "variables" ?session s)))) 'alphalessp)))
        list("own vars" mapcar(lambda((tn) list(tn
                                 mapcar(lambda((n) list(n car(errset(axlGetVarValue(axlGetVar(axlGetTest(sdb tn) n))))))
                                   cadr(car(errset(axlGetVars(axlGetTest(sdb tn))))))))
                               chkTests))
        list("parameter" car(errset(maeGetParameter(chkParam ?session s))))
        list("parameter hot" car(errset(maeGetParameter(chkParam ?typeName "corner" ?typeValue "hot" ?session s))))
        list("outputs" mapcar(lambda((tn) list(tn
                                mapcar(lambda((o) list(o~>name SfyAdeOutExpr(o)))
                                  car(errset(maeGetTestOutputs(tn ?session s))))))
                              chkTests))
        list("analyses" mapcar(lambda((tn) list(tn car(errset(maeGetEnabledAnalysis(tn ?session s))))) chkTests)))
      errset(axlExportSetup(s sdb sprintf(nil "%s/l6_%s.sdb" chkOut tag) chkTags)))
    when(s errset(maeCloseSession(?session s ?forceClose t)))
    out))

;; Compares the read-back of tag with want: PASS, or FAIL with each differing part.
procedure(chkSame(tag want got)
  let((bad)
    bad = setof(x want !member(x got))
    unless(bad chkPass(sprintf(nil "%s: every setting read back as expected" tag)))
    foreach(x bad
      chkFail(sprintf(nil "%s: %s was %L, now %L" tag car(x) cadr(x) cadr(assoc(car(x) got)))))
    null(bad)))

let((first second edited fourth before)
  chkOut = strcat(getShellEnvVar("SFYCHECK_WORK") "/out")
  first = strcat(chkOut "/l6_first.sfyx")
  second = strcat(chkOut "/l6_second.sfyx")
  edited = strcat(chkOut "/l6_edited.sfyx")
  fourth = strcat(chkOut "/l6_fourth.sfyx")
  before = chkRead("before")
  chkLine("before" before)
  chkThat("three tests, T3 disabled, T2's own C"
    and(member('("T3" nil) cadr(assoc("enabled" before)))
        member('("T2" (("C" "5n"))) cadr(assoc("own vars" before)))))
  when(chkThat("first dump" SfyExport(first chkLib '("rc")))
    when(chkThat("the driver's round trip arrived" chkWait(list(second edited) 900))
      chkThat("load the round trip" SfyImport(second ?overwrite t))
      chkSame("round trip" before chkLine("after round trip" chkRead("after1")))
      chkThat("load the edit of T2's C" SfyImport(edited ?overwrite t))
      chkSame("edit" subst('("T2" (("C" "7n"))) '("T2" (("C" "5n"))) before)
        chkLine("after edit" chkRead("after2")))
      chkThat("fourth dump" SfyExport(fourth chkLib '("rc"))))))

;; ---- cleanup (run.sh removes chkLib)
system(sprintf(nil "rm -f /tmp/sfy_%s_maesdb.sdb" getShellEnvVar("USER")))
