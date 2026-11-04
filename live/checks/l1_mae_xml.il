;; l1_mae_xml.il: lane L1 live check, a Maestro view's own setup through Schemify and back
;; (sfy_ade.il, interop spec 9.7). wrap.il has loaded common.il and schemify.il and created
;; chkLib (sfyprobe_l1).
;;
;; Builds rc/maestro in chkLib from analogLib parts only: tests T1 (tran) and T2 (ac,
;; disabled), global and test variables, corners hot (with a parameter) and cold (disabled),
;; a parameter, outputs, specs (a weight, a tolerance), Monte Carlo options through
;; axlPutRunOption, global and local optimizer options, the Monte Carlo run mode. Then:
;;   1. dump (SfyExport), Schemify import and export (schemify-interop), load over the view
;;      (SfyImport ?overwrite t);
;;   2. the same load into a library without the view (deleted first);
;;   3. an edit in Schemify (var K), exported and loaded: only that record is sent.
;; After each, the setup read back through the getters must equal the one before, and the
;; axlExportSetup XML and active.state are copied to the out directory (before.*, after<k>.*)
;; for the comparison modulo mae_xml::normalize. No simulation runs.
;; Environment: SFYCHECK_CLI, the schemify-interop binary (default ~/sfytest/l1/schemify-interop).

chkCli = or(getShellEnvVar("SFYCHECK_CLI") strcat(getShellEnvVar("HOME") "/sfytest/l1/schemify-interop"))
chkDir = strcat(getShellEnvVar("SFYCHECK_WORK") "/l1scratch")
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

;; ---- rc schematic and a model file
let((cv v r c p)
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
  chkE("rc save" lambda(() dbSave(cv)))
  p = outfile(strcat(chkDir "/stats.scs") "w")
  fprintf(p "simulator lang=spectre\nparameters dmc=0\n")
  close(p))

;; ---- rc/maestro
chkS = chkE("open" lambda(() maeOpenSetup(chkLib "rc" "maestro" ?mode "a")))
chkSdb = chkE("sdb" lambda(() axlGetMainSetupDB(chkS)))
foreach(tn '("T1" "T2")
  chkE(strcat("createTest " tn) lambda(() maeCreateTest(tn ?lib chkLib ?cell "rc" ?view "schematic" ?simulator "spectre" ?session chkS)))
  chkE(strcat("modelFiles " tn) lambda(() maeSetEnvOption(tn ?options list(list("modelFiles" list(list(strcat(chkDir "/stats.scs") "")))) ?session chkS))))
chkE("tran T1" lambda(() maeSetAnalysis("T1" "tran" ?enable t ?options '(("stop" "20u") ("errpreset" "moderate")) ?session chkS)))
chkE("ac T2" lambda(() maeSetAnalysis("T2" "ac" ?enable t ?options '(("start" "1") ("stop" "1G") ("incrType" "Logarithmic") ("stepTypeLog" "Points Per Decade") ("dec" "5")) ?session chkS)))
chkE("var C" lambda(() maeSetVar("C" "1n" ?session chkS)))
chkE("var K" lambda(() maeSetVar("K" "7" ?session chkS)))
chkE("var rmc" lambda(() maeSetVar("rmc" "1k" ?session chkS)))
chkE("var C test T2" lambda(() maeSetVar("C" "5n" ?typeName "test" ?typeValue '("T2") ?session chkS)))
chkE("corner hot" lambda(() maeSetCorner("hot" ?enabled t ?session chkS)))
chkE("corner hot temp" lambda(() maeSetVar("temperature" "85" ?typeName "corner" ?typeValue '("hot") ?session chkS)))
chkE("corner cold" lambda(() maeSetCorner("cold" ?enabled nil ?session chkS)))
chkE("corner cold temp" lambda(() maeSetVar("temperature" "-40" ?typeName "corner" ?typeValue '("cold") ?session chkS)))
chkParam = strcat(chkLib "/rc/schematic/V0/v2")
chkE("parameter" lambda(() maeSetParameter(chkParam "1" ?session chkS)))
chkE("parameter hot" lambda(() maeSetParameter(chkParam "1.1" ?typeName "corner" ?typeValue '("hot") ?session chkS)))
chkE("out vmax" lambda(() maeAddOutput("vmax" "T1" ?outputType "point" ?expr "ymax(VT(\"/out\"))" ?session chkS)))
chkE("out vmin" lambda(() maeAddOutput("vmin" "T1" ?outputType "point" ?expr "ymin(VT(\"/out\"))" ?session chkS)))
chkE("out net" lambda(() maeAddOutput("out" "T1" ?outputType "net" ?signalName "/out" ?session chkS)))
chkE("out gain" lambda(() maeAddOutput("gain" "T2" ?outputType "point" ?expr "ymax(mag(VF(\"/out\")))" ?session chkS)))
chkE("spec vmax gt" lambda(() maeSetSpec("vmax" "T1" ?gt "0.5" ?session chkS)))
chkE("spec gain range weight" lambda(() maeSetSpec("gain" "T2" ?range '("0.1" "2") ?weight "2" ?session chkS)))
chkE("spec vmin tolerance weight" lambda(() maeSetSpec("vmin" "T1" ?tolerance '("0.1" "10%") ?weight "3" ?session chkS)))
chkE("disable T2" lambda(() maeSetSetup(?tests '("T2") ?enabled nil ?session chkS)))
foreach(kv '(("Monte Carlo Sampling" "mcnumpoints" "7") ("Monte Carlo Sampling" "mcmethod" "process")
             ("Monte Carlo Sampling" "samplingmode" "lhs") ("Monte Carlo Sampling" "montecarloseed" "4242")
             ("Monte Carlo Sampling" "mcnumbins" "5") ("Monte Carlo Sampling" "saveallplots" "1")
             ("Global Optimization" "numpoints" "50") ("Global Optimization" "timelimit" "7")
             ("Local Optimization" "effort" "coarse") ("Local Optimization" "numpoints" "10"))
  chkE(sprintf(nil "axlPutRunOption %L" kv) lambda(() axlSetRunOptionValue(axlPutRunOption(chkSdb car(kv) cadr(kv)) caddr(kv)))))
chkE("runMode MC" lambda(() maeSetCurrentRunMode(?runMode "Monte Carlo Sampling" ?session chkS)))
chkE("save" lambda(() maeSaveSetup(?lib chkLib ?cell "rc" ?view "maestro" ?session chkS)))
chkE("close" lambda(() maeCloseSession(?session chkS ?forceClose t)))

;; ---- what the view holds, read back
;; Everything the getters answer about rc/maestro, as one list, and the XML and the test
;; states copied to <out>/<tag>.sdb and <tag>.state.
procedure(chkRead(tag)
  let((s sdb out tests h)
    s = car(errset(maeOpenSetup(chkLib "rc" "maestro" ?mode "a")))
    sdb = s && car(errset(axlGetMainSetupDB(s)))
    when(sdb
      tests = sort(copy(car(errset(maeGetSetup(?session s)))) 'alphalessp)
      out = list(
        list("tests" tests)
        list("enabled tests" sort(copy(car(errset(maeGetSetup(?typeName "tests" ?enabled t ?session s)))) 'alphalessp))
        list("corners" sort(copy(car(errset(maeGetSetup(?typeName "corners" ?session s)))) 'alphalessp))
        list("cornersEnabled" mapcar(lambda((c) list(c car(errset(axlGetEnabled(axlGetCorner(sdb c))))))
                                    '("hot" "cold")))
        list("vars" mapcar(lambda((n) list(n car(errset(maeGetVar(n ?session s)))))
                          sort(copy(car(errset(maeGetSetup(?typeName "variables" ?session s)))) 'alphalessp)))
        list("T2 C" car(errset(axlGetVarValue(axlGetVar(axlGetTest(sdb "T2") "C")))))
        list("hot temperature" car(errset(maeGetVar("temperature" ?typeName "corner" ?typeValue "hot" ?session s))))
        list("parameter" car(errset(maeGetParameter(chkParam ?session s))))
        list("parameter hot" car(errset(maeGetParameter(chkParam ?typeName "corner" ?typeValue "hot" ?session s))))
        list("run mode" car(errset(maeGetCurrentRunMode(?session s))))
        list("run options" mapcar(lambda((m) list(m mapcar(lambda((n) list(n car(errset(axlGetRunOptionValue(axlGetRunOption(sdb m n))))))
                                                             cadr(car(errset(axlGetRunOptions(sdb m)))))))
                                  chkModes))
        list("specs" mapcar(lambda((x) list(x car(errset(axlGetSpecData(sdb cadr(x) car(x))))
                                             car(errset(axlGetSpecWeight(axlGetSpec(sdb strcat(car(x) "." cadr(x))))))))
                            '(("T1" "vmax") ("T1" "vmin") ("T2" "gain"))))
        list("tests detail"
          mapcar(lambda((tn) list(tn
                   sort(copy(car(errset(maeGetEnabledAnalysis(tn ?session s)))) 'alphalessp)
                   car(errset(maeGetAnalysis(tn car(car(errset(maeGetEnabledAnalysis(tn ?session s)))) ?session s)))
                   car(errset(maeGetEnvOption(tn ?option "modelFiles" ?session s)))
                   mapcar(lambda((o) list(o~>name SfyAdeOutExpr(o) SfyAdeOutSignal(o)))
                     car(errset(maeGetTestOutputs(tn ?session s))))))
            tests)))
      errset(axlExportSetup(s sdb sprintf(nil "%s/l1_%s.sdb" chkOut tag) chkTags)))
    when(s errset(maeCloseSession(?session s ?forceClose t)))
    h = ddGetObj(chkLib "rc" "maestro" "active.state")
    when(h system(sprintf(nil "cp %s %s/l1_%s.state" h~>readPath chkOut tag)))
    out))

;; Compares the read-back of tag with chkBefore: PASS, or FAIL with each differing part.
procedure(chkSame(tag got)
  let((bad)
    bad = setof(x chkBefore !member(x got))
    unless(bad chkPass(sprintf(nil "%s: every setting read back as before" tag)))
    foreach(x bad
      chkFail(sprintf(nil "%s: %s was %L, now %L" tag car(x) cadr(x) cadr(assoc(car(x) got)))))
    system(sprintf(nil "cd %s && (diff l1_before.sdb l1_%s.sdb; diff l1_before.state l1_%s.state) > l1_%s.diff 2>&1; echo \"%s diff lines: $(wc -l < l1_%s.diff)\" >> l1_diffs.txt"
      chkOut tag tag tag tag tag))
    null(bad)))

;; Runs schemify-interop with args (in chkDir/proj); t on exit status 0.
procedure(chkCliRun(label args)
  let((st)
    st = system(sprintf(nil "cd %s/proj && %s %s > %s/l1_%s.cli.txt 2>&1" chkDir chkCli args chkOut label))
    chkLine(sprintf(nil "schemify-interop %s" args) st)
    st == 0))

;; The tags of the records of an .sfyx file from its maestro record on, but maesdb,
;; maestate and end.
procedure(chkSent(path)
  let((p line out on tag)
    p = infile(path)
    when(p
      while(gets(line p)
        tag = car(parseString(line "(\" "))
        when(tag == "maestro" on = t)
        when(on && !member(tag '("maesdb" "maestate" "end"))
          out = cons(substring(line 1 min(60 strlen(line))) out)))
      close(p))
    reverse(out)))

chkBefore = chkRead("before")
chkLine("before" chkBefore)
system(sprintf(nil "rm -f %s/l1_diffs.txt" chkOut))

;; ---- 1. dump, Schemify, load over the view
chkA = strcat(chkDir "/proj/a.sfyx")
chkThat("dump" SfyExport(chkA chkLib '("rc")))
chkE("dump maesdb/maestate lines" lambda(() let((p line n) n = 0 p = infile(chkA)
  while(gets(line p) when(member(car(parseString(line "(\" ")) '("maesdb" "maestate")) n = n + 1))
  close(p) n)))
chkThat("schemify-interop virtuoso-import" chkCliRun("import" "virtuoso-import . a.sfyx"))
chkThat("schemify-interop virtuoso-export" chkCliRun("export" sprintf(nil "virtuoso-export . %s/rc.shbench -o b.sfyx" chkLib)))
chkLine("b.sfyx records sent" chkSent(strcat(chkDir "/proj/b.sfyx")))
chkThat("only the maestro record is sent unchanged" length(chkSent(strcat(chkDir "/proj/b.sfyx"))) <= 1)
chkThat("load over the view" SfyImport(strcat(chkDir "/proj/b.sfyx") ?overwrite t))
system(sprintf(nil "cp %s/proj/b.sfyx.log %s/l1_b1.log" chkDir chkOut))
chkSame("after1" chkRead("after1"))

;; ---- 2. into a library without the view
chkE("delete maestro view" lambda(() ddDeleteObj(ddGetObj(chkLib "rc" "maestro"))))
chkThat("maestro view gone" null(ddGetObj(chkLib "rc" "maestro")))
chkThat("load into no view" SfyImport(strcat(chkDir "/proj/b.sfyx") ?overwrite t))
system(sprintf(nil "cp %s/proj/b.sfyx.log %s/l1_b2.log" chkDir chkOut))
chkSame("after2" chkRead("after2"))

;; ---- 3. an edit in Schemify: var K
system(sprintf(nil "sed -i 's/^var K 7$/var K 8/' %s/proj/%s/rc.shbench" chkDir chkLib))
chkThat("schemify-interop virtuoso-export (edited)" chkCliRun("export3" sprintf(nil "virtuoso-export . %s/rc.shbench -o c.sfyx" chkLib)))
chkLine("c.sfyx records sent" chkSent(strcat(chkDir "/proj/c.sfyx")))
chkThat("the edit alone is sent" length(setof(l chkSent(strcat(chkDir "/proj/c.sfyx")) car(parseString(l "(\" ")) != "maestro")) == 1)
chkThat("load the edit" SfyImport(strcat(chkDir "/proj/c.sfyx") ?overwrite t))
system(sprintf(nil "cp %s/proj/c.sfyx.log %s/l1_c.log" chkDir chkOut))
let((got)
  got = chkRead("after3")
  chkThat("K is 8 after the edit" member('("K" "8") cadr(assoc("vars" got))))
  chkBefore = mapcar(lambda((x) if(car(x) == "vars"
                                   list("vars" subst('("K" "8") '("K" "7") cadr(x)))
                                   x))
                chkBefore)
  chkSame("after3" got))

;; ---- cleanup (run.sh removes chkLib)
system(sprintf(nil "rm -rf %s /tmp/sfy_%s_maesdb.sdb" chkDir getShellEnvVar("USER")))
