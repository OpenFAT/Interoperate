;; l7_adel.il: lane L7 live check, ADE L states through Schemify and back (sfy_adel.il,
;; interop spec 9.8). wrap.il has loaded common.il and schemify.il and created chkLib
;; (sfyprobe_l7).
;;
;; Builds rc (analogLib parts only) and a maestro test T1 whose asi session (ICADVM20.1
;; has no ADE L session) gets design variables, a tran analysis, outputs and a model file;
;; saves it as ADE L states st1 (directory, under the scratch dir) and st2 (cellview), then
;; deletes the maestro view, so only the states describe the setup. Then, per state:
;;   (a) dump (SfyAdeLExport), Schemify import and export (schemify-interop), the state
;;       deleted, load (SfyAdeLImport): every state file must come back byte for byte;
;;   (b) for st1, an edit in Schemify (var C 1n -> 2n), exported and loaded: the bench lands in
;;       rc/maestro as test st1 with its variables, and the state files are not rewritten.
;; Environment: SFYCHECK_CLI, the schemify-interop binary (default ~/sfytest/l7/schemify-interop).

chkCli = or(getShellEnvVar("SFYCHECK_CLI") strcat(getShellEnvVar("HOME") "/sfytest/l7/schemify-interop"))
chkDir = strcat(getShellEnvVar("SFYCHECK_WORK") "/l7scratch")
chkOut = strcat(getShellEnvVar("SFYCHECK_WORK") "/out")
chkStates = strcat(chkDir "/states")
system(sprintf(nil "rm -rf %s && mkdir -p %s/proj1 %s/proj2 %s" chkDir chkDir chkDir chkStates))
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
  foreach(kv '(("v1" "0") ("v2" "1") ("tr" "1n") ("tf" "1n") ("pw" "5u") ("per" "10u")) chkSet(v car(kv) cadr(kv)))
  chkSet(r "r" "rmc")
  chkSet(c "c" "C")
  chkStub(cv v '(("PLUS" "in") ("MINUS" "gnd!")))
  chkStub(cv r '(("PLUS" "in") ("MINUS" "out")))
  chkStub(cv c '(("PLUS" "out") ("MINUS" "gnd!")))
  chkE("rc check" lambda(() schCheck(cv)))
  chkE("rc save" lambda(() dbSave(cv)))
  p = outfile(strcat(chkDir "/models.scs") "w")
  fprintf(p "simulator lang=spectre\nparameters dmc=0\n")
  close(p))

;; ---- T1 and its asi session, saved as the two ADE L states
chkS = chkE("open" lambda(() maeOpenSetup(chkLib "rc" "maestro" ?mode "a")))
chkE("createTest T1" lambda(() maeCreateTest("T1" ?lib chkLib ?cell "rc" ?view "schematic" ?simulator "spectre" ?session chkS)))
chkE("tran" lambda(() maeSetAnalysis("T1" "tran" ?enable t ?options '(("stop" "20u")) ?session chkS)))
chkE("modelFiles" lambda(() maeSetEnvOption("T1" ?options list(list("modelFiles" list(list(strcat(chkDir "/models.scs") "")))) ?session chkS)))
chkE("out vmax" lambda(() maeAddOutput("vmax" "T1" ?outputType "point" ?expr "ymax(VT(\"/out\"))" ?session chkS)))
chkE("out net" lambda(() maeAddOutput("out" "T1" ?outputType "net" ?signalName "/out" ?session chkS)))
chkE("test var C" lambda(() maeSetVar("C" "1n" ?typeName "test" ?typeValue '("T1") ?session chkS)))
chkE("test var rmc" lambda(() maeSetVar("rmc" "1k" ?typeName "test" ?typeValue '("T1") ?session chkS)))
chkA = chkE("asi session" lambda(() maeGetTestSession("T1" ?session chkS)))
chkE("asi desVars before" lambda(() asiGetDesignVarList(chkA)))
chkE("asiSetDesignVarList" lambda(() asiSetDesignVarList(chkA '(("C" "1n") ("rmc" "1k")))))
chkE("asi desVars" lambda(() asiGetDesignVarList(chkA)))
chkE("asi outputs" lambda(() mapcar(lambda((o) list(o~>name o~>expression o~>signal)) asiGetOutputList(chkA))))
chkE("save st1 (dir)" lambda(() asiSaveState(chkA ?name "st1" ?option 'dir ?stateDir chkStates ?lib chkLib ?cell "rc" ?simulator "spectre" ?description "l7 dir state")))
chkE("save st2 (cellview)" lambda(() asiSaveState(chkA ?name "st2" ?option 'cellview ?lib chkLib ?cell "rc" ?simulator "spectre" ?description "l7 cellview state")))
chkE("close" lambda(() maeCloseSession(?session chkS ?forceClose t)))
chkE("delete maestro view" lambda(() ddDeleteObj(ddGetObj(chkLib "rc" "maestro"))))
chkThat("maestro view gone" null(ddGetObj(chkLib "rc" "maestro")))

chkSt1 = sprintf(nil "%s/%s/rc/spectre/st1" chkStates chkLib)
chkSt2 = sprintf(nil "%s/rc/st2" ddGetObj(chkLib)~>readPath)
chkThat("st1 saved" isFile(strcat(chkSt1 "/ADE_state.info")))
chkThat("st2 saved" isFile(strcat(chkSt2 "/ADE_state.info")))
system(sprintf(nil "cp -r %s %s/orig_st1 && cp -r %s %s/orig_st2 && cp %s/variables %s/l7_variables.txt"
  chkSt1 chkDir chkSt2 chkDir chkSt1 chkOut))
chkE("st1 files" lambda(() sort(getDirFiles(chkSt1) 'alphalessp)))

;; Runs schemify-interop with args in chkDir/<proj>; t on exit status 0.
procedure(chkCliRun(proj label args)
  let((st)
    st = system(sprintf(nil "cd %s/%s && %s %s > %s/l7_%s.cli.txt 2>&1" chkDir proj chkCli args chkOut label))
    chkLine(sprintf(nil "schemify-interop %s" args) st)
    st == 0))

;; The tags of the records of an .sfyx file from the first maestro or adel record on.
procedure(chkTags(path)
  let((p line out on tag)
    p = infile(path)
    when(p
      while(gets(line p)
        tag = car(parseString(line "(\" "))
        when(member(tag '("maestro" "adel")) on = t)
        when(on && tag != "end" out = cons(tag out)))
      close(p))
    reverse(out)))

;; t when directory dir holds exactly the files of orig, byte for byte (diff -r).
procedure(chkSameDir(label orig dir)
  let((st)
    st = system(sprintf(nil "diff -r %s %s > %s/l7_%s.diff 2>&1" orig dir chkOut label))
    chkThat(sprintf(nil "%s: the state files came back byte for byte" label) st == 0)))

;; ---- (a) per state: dump, Schemify, delete, load
foreach(x list(list("st1" "dir" "proj1" chkSt1) list("st2" "cellview" "proj2" chkSt2))
  let((state from proj dir a b)
    state = car(x) from = cadr(x) proj = caddr(x) dir = cadddr(x)
    a = sprintf(nil "%s/%s/a.sfyx" chkDir proj)
    b = sprintf(nil "%s/%s/b.sfyx" chkDir proj)
    chkThat(sprintf(nil "%s dump" state) SfyAdeLExport(a chkLib "rc" state ?from from ?stateDir chkStates))
    system(sprintf(nil "cp %s.log %s/l7_%s_a.log" a chkOut state))
    chkThat(sprintf(nil "%s schemify-interop virtuoso-import" state) chkCliRun(proj sprintf(nil "%s_import" state) "virtuoso-import . a.sfyx"))
    system(sprintf(nil "cp %s/%s/%s/rc.shbench %s/l7_%s.shbench" chkDir proj chkLib chkOut state))
    chkThat(sprintf(nil "%s schemify-interop virtuoso-export" state) chkCliRun(proj sprintf(nil "%s_export" state) sprintf(nil "virtuoso-export . %s/rc.shbench -o b.sfyx" chkLib)))
    chkLine(sprintf(nil "%s b.sfyx records" state) chkTags(b))
    chkThat(sprintf(nil "%s unchanged: the state goes back, no maestro test" state)
      car(chkTags(b)) == "adel" && !member("maestro" chkTags(b)))
    if(from == "dir"
      system(sprintf(nil "rm -rf %s" dir))
      chkE(sprintf(nil "%s delete view" state) lambda(() ddDeleteObj(ddGetObj(chkLib "rc" state)))))
    chkThat(sprintf(nil "%s gone" state) !isFile(strcat(dir "/ADE_state.info")))
    chkThat(sprintf(nil "%s load" state) SfyAdeLImport(b ?overwrite t))
    system(sprintf(nil "cp %s.log %s/l7_%s_b.log; cp %s.cells.sfyx.log %s/l7_%s_b_cells.log" b chkOut state b chkOut state))
    chkSameDir(state sprintf(nil "%s/orig_%s" chkDir state) dir)
    when(from == "cellview"
      chkThat("st2 is a view again" ddGetObj(chkLib "rc" "st2") != nil))))

;; ---- (b) st1 edited in Schemify: var C 1n -> 2n
let((bench c st)
  bench = sprintf(nil "%s/proj1/%s/rc.shbench" chkDir chkLib)
  system(sprintf(nil "sed -i 's/^var C 1n$/var C 2n/' %s" bench))
  chkThat("st1 edited: schemify-interop virtuoso-export" chkCliRun("proj1" "st1_export_edited" sprintf(nil "virtuoso-export . %s/rc.shbench -o c.sfyx" chkLib)))
  c = sprintf(nil "%s/proj1/c.sfyx" chkDir)
  chkLine("c.sfyx records" chkTags(c))
  chkThat("st1 edited: a maestro test, no state" car(chkTags(c)) == "maestro" && !member("adel" chkTags(c)))
  system(sprintf(nil "cp %s/proj1/c.report.txt %s/l7_c.report.txt" chkDir chkOut))
  chkThat("st1 edited: load" SfyAdeLImport(c ?overwrite t))
  system(sprintf(nil "cp %s.log %s/l7_c.log; cp %s.cells.sfyx.log %s/l7_c_cells.log" c chkOut c chkOut))
  st = car(errset(maeOpenSetup(chkLib "rc" "maestro" ?mode "a")))
  chkThat("st1 edited: rc/maestro opens" st)
  when(st
    chkThat("st1 edited: test st1" member("st1" car(errset(maeGetSetup(?session st)))))
    foreach(kv '(("C" "2n") ("rmc" "1k"))
      chkThat(sprintf(nil "st1 edited: var %s is %s" car(kv) cadr(kv))
        car(errset(maeGetVar(car(kv) ?session st))) == cadr(kv)))
    chkE("st1 edited: tran" lambda(() maeGetAnalysis("st1" "tran" ?session st)))
    chkE("st1 edited: outputs" lambda(() mapcar(lambda((o) o~>name) maeGetTestOutputs("st1" ?session st))))
    errset(maeCloseSession(?session st ?forceClose t)))
  chkSameDir("st1_after_maestro" sprintf(nil "%s/orig_st1" chkDir) chkSt1))

;; ---- cleanup (run.sh removes chkLib)
system(sprintf(nil "rm -rf %s /tmp/sfy_%s_adel.b64" chkDir getShellEnvVar("USER")))
