;; a_ade.il: lane A (gaps round 2) live check, run through a_drive.sh (which does the
;; Schemify half on its own machine). wrap.il has loaded common.il and schemify.il and
;; created chkLib (sfyprobe_a).
;;
;; Export side: loads $SFYCHECK_WORK/out/a_in.sfyx (the driver's Rust export of
;; tests/fixtures/live/a_ade/a_tb.shbench: .noise v(out), .dc, .ac lin, simulator options,
;; settle and max outputs) with SfyImport, netlists its test with ADE
;; (maeCreateNetlistForCorner) and runs Spectre on that netlist: the noise output nodes, the
;; dc step, lin=n-1 and the options must be in it and Spectre must finish; then the bench's
;; OCEAN script (a_in/ocean/a_tb.ocn, its exit() dropped) runs in this session and its
;; settlingTime and max outputs must print numbers (they were OCEAN errors).
;; Import side: builds rc (V0 vdc -> IPRB0 iprobe -> R0 -> out, C0) and a maestro view of
;; one test per kind the fixes map (noise as GUI users set it, stb, xf, dcmatch, pss+pac, a
;; temperature and a design-variable dc sweep, ac center-span), TO with simulator options,
;; a temperature list var beside the temp option, an evalType-corners output, spec kinds
;; min/info/tol, corner C1 a group, C2 disabled for N1, a definition file, TR bound to
;; cell rr, TH on hspiceD; dumps it to out/a_dump.sfyx for the driver and checks the
;; dump's new records. Writes out/a_live/ (netlists, logs).

chkW = getShellEnvVar("SFYCHECK_WORK")
chkOut = strcat(chkW "/out/a_live")
chkSim = strcat(chkW "/sim_a")
system(sprintf(nil "rm -rf '%s' '%s'; mkdir -p '%s'" chkOut chkSim chkOut))
envSetVal("asimenv.startup" "projectDir" 'string chkSim)

procedure(chkSet(inst name value)
  let((c)
    c = cdfFindParamByName(cdfGetInstCDF(inst) name)
    if(c then c~>value = value t else chkLine(sprintf(nil "no CDF param %s on %s" name inst~>name) nil))))

;; A wire stub from each named terminal's pin centre, away from the instance's centre.
procedure(chkStub(cv inst nets)
  let((bb c p2 w ib dx)
    ib = inst~>bBox
    foreach(term inst~>master~>terminals
      when(assoc(term~>name nets)
        bb = dbTransformBBox(car(term~>pins)~>fig~>bBox inst~>transform)
        c = list((xCoord(car(bb)) + xCoord(cadr(bb))) / 2.0 (yCoord(car(bb)) + yCoord(cadr(bb))) / 2.0)
        dx = if(xCoord(c) < (xCoord(car(ib)) + xCoord(cadr(ib))) / 2.0 - 0.01 then -0.25 else 0.25)
        p2 = list(xCoord(c) + dx yCoord(c))
        w = schCreateWire(cv "draw" "full" list(c p2) 0.0625 0.0625 0.0)
        schCreateWireLabel(cv car(w) p2 cadr(assoc(term~>name nets)) "lowerLeft" "R0" "stick" 0.0625 nil)))))

;; The lines of file path without their newline (nil when it cannot be read).
procedure(chkLines(path)
  let((port l out)
    port = and(isFile(path) infile(path))
    when(port
      while(gets(l port)
        out = cons(if(rexMatchp("\n$" l) substring(l 1 strlen(l) - 1) l) out))
      close(port))
    reverse(out)))

;; t when some line of lines holds every part of parts (rex patterns: `(` is literal,
;; there is no alternation).
procedure(chkHas(lines parts)
  exists(l lines forall(x parts rexMatchp(x l))))

;; Netlists test of session sess (Nominal) into chkOut/<label>, runs Spectre on it there;
;; answers (netlist lines, spectre exit line).
procedure(chkNetlist(sess test label)
  let((d)
    d = strcat(chkOut "/" label)
    chkE(sprintf(nil "netlist %s" label) lambda(() maeCreateNetlistForCorner(test "Nominal" d ?session sess)))
    system(sprintf(nil "cd '%s/netlist' && timeout 300 spectre -format psfascii -raw raw input.scs > spectre.out 2>&1; echo \"exit $?\" >> spectre.out" d))
    list(chkLines(strcat(d "/netlist/input.scs")) car(last(chkLines(strcat(d "/netlist/spectre.out")))))))

;; ---- export side: the driver's Rust export through SfyImport, ADE and Spectre
let((nl ok sess test)
  ok = chkE("SfyImport a_in.sfyx" lambda(() SfyImport(strcat(chkW "/out/a_in.sfyx") ?lib chkLib)))
  chkThat("SfyImport of the Rust export" ok)
  sess = chkE("open a_tb maestro" lambda(() maeOpenSetup(chkLib "a_tb" "maestro" ?mode "a")))
  test = car(chkE("tests" lambda(() maeGetSetup(?session sess))))
  chkE("analyses" lambda(() foreach(mapcar a maeGetEnabledAnalysis(test ?session sess) list(a maeGetAnalysis(test a ?session sess)))))
  chkE("sim options" lambda(() maeGetSimOption(test ?session sess)))
  nl = chkNetlist(sess test "export")
  chkLine("export netlist" setof(l car(nl) exists(w '("^noise " "^dc " "^ac " "^tran " "reltol") rexMatchp(w l))))
  chkThat("noise output nodes (out 0), not oprobe" chkHas(car(nl) '("^noise ( *out 0 *) noise" "iprobe=V1")))
  chkThat("dc has its step (incrType Linear)" chkHas(car(nl) '("^dc dc " "step=0.1")))
  chkThat(".ac lin 5 is lin=4 (5 points)" chkHas(car(nl) '("^ac ac " "lin=4")))
  chkThat("simulator options reltol 1e-5 and gmin 1e-14 netlisted" and(chkHas(car(nl) '("reltol=1e-5")) chkHas(car(nl) '("gmin=1e-14"))))
  chkThat(sprintf(nil "Spectre runs ADE's netlist of the export (%L)" cadr(nl)) cadr(nl) == "exit 0")
  chkE("close a_tb" lambda(() maeCloseSession(?session sess ?forceClose t))))

;; The OCEAN script as written, in an `ocean -nograph` of its own beside this session (the
;; run lock is this session's; design() in -nograph Virtuoso answered nil, live), design()
;; naming chkLib, from chkOut (the scratch library is in the work directory's cds.lib).
let((src lines port)
  src = strcat(chkW "/out/a_in/ocean/a_tb.ocn")
  lines = chkLines(src)
  port = outfile(strcat(chkOut "/a_tb.ocn") "w")
  foreach(l lines
    if(rexMatchp("^design" l)
      fprintf(port "design(%L %L %L)\n" chkLib "a_tb" "schematic")
      fprintf(port "%s\n" l)))
  close(port)
  system(sprintf(nil "cp '%s/cds.lib' '%s/' && cd '%s' && timeout 900 ocean -nograph < a_tb.ocn > ocean.log 2>&1" chkW chkOut chkOut))
  chkLine("OCEAN errors" setof(l chkLines(strcat(chkOut "/ocean.log")) rexMatchp("rror" l)))
  foreach(o '("st" "mx" "tau")
    chkLine(sprintf(nil "OCEAN %s" o) chkLines(sprintf(nil "%s/a_tb/%s.txt" chkOut o))))
  chkThat("OCEAN settlingTime of settle(v(out), 10m) is a number"
    chkHas(chkLines(strcat(chkOut "/a_tb/st.txt")) '("[0-9]e-0[0-9]")))
  chkThat("OCEAN max(v(out), 0.5) is a number"
    chkHas(chkLines(strcat(chkOut "/a_tb/mx.txt")) '("[0-9]\\.[0-9]"))))

;; ---- import side: a maestro view as GUI users set it, dumped
let((cv v i r c r1)
  cv = dbOpenCellViewByType(chkLib "rc" "schematic" "schematic" "w")
  v = schCreateInst(cv dbOpenCellViewByType("analogLib" "vdc" "symbol") "V0" 0:0 "R0")
  i = schCreateInst(cv dbOpenCellViewByType("analogLib" "iprobe" "symbol") "IPRB0" 3:0 "R0")
  r = schCreateInst(cv dbOpenCellViewByType("analogLib" "res" "symbol") "R0" 6:0 "R0")
  c = schCreateInst(cv dbOpenCellViewByType("analogLib" "cap" "symbol") "C0" 9:0 "R0")
  ;; R1 makes R0/R1 a divider: out is 0.5 V at the operating point only if gnd! is ground.
  r1 = schCreateInst(cv dbOpenCellViewByType("analogLib" "res" "symbol") "R1" 12:0 "R0")
  chkSet(r1 "r" "1k")
  chkStub(cv r1 '(("PLUS" "out") ("MINUS" "gnd!")))
  foreach(kv '(("vdc" "1") ("acm" "1") ("pacm" "1")) chkSet(v car(kv) cadr(kv)))
  chkSet(r "r" "1k")
  chkSet(c "c" "C")
  chkStub(cv v '(("PLUS" "in") ("MINUS" "gnd!")))
  chkStub(cv i '(("PLUS" "in") ("MINUS" "in2")))
  chkStub(cv r '(("PLUS" "in2") ("MINUS" "out")))
  chkStub(cv c '(("PLUS" "out") ("MINUS" "gnd!")))
  chkE("rc check" lambda(() schCheck(cv)))
  dbSave(cv) dbClose(cv)
  ;; rr: the cell test TR is bound to.
  cv = dbOpenCellViewByType(chkLib "rr" "schematic" "schematic" "w")
  v = schCreateInst(cv dbOpenCellViewByType("analogLib" "vdc" "symbol") "V0" 0:0 "R0")
  r = schCreateInst(cv dbOpenCellViewByType("analogLib" "res" "symbol") "R0" 3:0 "R0")
  chkSet(v "vdc" "1") chkSet(r "r" "1k")
  chkStub(cv v '(("PLUS" "a") ("MINUS" "gnd!")))
  chkStub(cv r '(("PLUS" "a") ("MINUS" "gnd!")))
  chkE("rr check" lambda(() schCheck(cv)))
  dbSave(cv) dbClose(cv))

chkFreq = '(("start" "1") ("stop" "1G") ("incrType" "Logarithmic") ("stepTypeLog" "Points Per Decade") ("dec" "10"))
chkAna = list(
  list("N1" "rc" list(list("noise" append(chkFreq '(("outType" "voltage") ("p" "/out") ("n" "/gnd!") ("iprobe" "/V0"))))))
  list("N2" "rc" list(list("noise" '(("start" "1") ("stop" "5") ("incrType" "Linear") ("stepTypeLin" "Number of Steps") ("lin" "4")
                                      ("outType" "voltage") ("p" "/out") ("n" "/gnd!") ("iprobe" "/V0")))))
  list("B1" "rc" list(list("stb" append(chkFreq '(("probe" "/IPRB0"))))))
  list("X1" "rc" list(list("xf" append(chkFreq '(("outType" "voltage") ("p" "/out") ("n" "/gnd!"))))))
  list("M1" "rc" list(list("dcmatch" '(("outType" "voltage") ("p" "/out") ("n" "/gnd!")))))
  list("PA" "rc" list(list("pss" '(("fund" "1M") ("harms" "3")))
                      list("pac" '(("start" "1k") ("stop" "100k") ("incrType" "Logarithmic") ("stepTypeLog" "Points Per Decade") ("dec" "5")))))
  list("D3" "rc" list(list("dc" '(("sweep" "Temperature") ("param" "temp") ("start" "-40") ("stop" "125") ("incrType" "Linear")
                                   ("stepTypeLin" "Step Size") ("step" "5")))))
  list("D4" "rc" list(list("dc" '(("sweep" "Design Variable") ("designVar" "C") ("param" "C") ("start" "1n") ("stop" "2n")
                                   ("incrType" "Linear") ("stepTypeLin" "Number of Steps") ("lin" "2")))))
  list("A2" "rc" list(list("ac" '(("rangeType" "Center-Span") ("center" "1k") ("span" "100") ("incrType" "Logarithmic")
                                   ("stepTypeLog" "Number of Steps") ("log" "20")))))
  list("TR" "rr" list(list("dc" '(("saveOppoint" t))))))

chkS = chkE("open rc maestro" list('maeOpenSetup chkLib "rc" "maestro" ?mode "a"))
chkSdb = chkE("sdb" lambda(() axlGetMainSetupDB(chkS)))
chkE("var C" lambda(() maeSetVar("C" "1n" ?session chkS)))
foreach(a chkAna
  chkE(strcat("createTest " car(a)) lambda(() maeCreateTest(car(a) ?lib chkLib ?cell cadr(a) ?view "schematic" ?simulator "spectre" ?session chkS)))
  errset(maeSetAnalysis(car(a) "tran" ?enable nil ?session chkS))
  foreach(an caddr(a)
    chkE(sprintf(nil "setAnalysis %s %s" car(a) car(an)) lambda(() maeSetAnalysis(car(a) car(an) ?enable t ?options cadr(an) ?session chkS)))))
chkE("createTest TO" lambda(() maeCreateTest("TO" ?lib chkLib ?cell "rc" ?view "schematic" ?simulator "spectre" ?session chkS)))
chkE("TO tran" lambda(() maeSetAnalysis("TO" "tran" ?enable t ?options '(("stop" "10u")) ?session chkS)))
chkE("TO simOptions" lambda(() maeSetSimOption("TO" ?options '(("reltol" "1e-5") ("gmin" "1e-14") ("maxnotes" "7") ("temp" "50")) ?session chkS)))
chkE("TO defn" lambda(() maeSetEnvOption("TO" ?options list(list("definitionFiles" list(strcat(chkOut "/defs.scs")))) ?session chkS)))
let((port) port = outfile(strcat(chkOut "/defs.scs") "w") fprintf(port "simulator lang=spectre\nparameters dd=1\n") close(port))
chkE("var temperature list" lambda(() maeSetVar("temperature" "-40,27,125" ?session chkS)))
chkE("out ymx" lambda(() maeAddOutput("ymx" "TO" ?outputType "point" ?expr "ymax(VT(\"/out\"))" ?session chkS)))
chkE("out ymn" lambda(() maeAddOutput("ymn" "TO" ?outputType "point" ?expr "ymin(VT(\"/out\"))" ?session chkS)))
chkE("out inf" lambda(() maeAddOutput("inf" "TO" ?outputType "point" ?expr "rms(VT(\"/out\"))" ?session chkS)))
chkE("out ymxa corners" lambda(() axlAddOutputExpr(chkS "TO" "ymxa" ?expr "ymax(VT(\"/out\"))" ?evalType "corners")))
chkE("out st" lambda(() maeAddOutput("st" "TO" ?outputType "point"
  ?expr "settlingTime(VT(\"/out\") ymin(xval(VT(\"/out\"))) t ymax(xval(VT(\"/out\"))) t 100*0.01/abs(value(VT(\"/out\") ymax(xval(VT(\"/out\"))))-value(VT(\"/out\") ymin(xval(VT(\"/out\"))))))" ?session chkS)))
chkE("spec ymx gt" lambda(() maeSetSpec("ymx" "TO" ?gt "0.05" ?session chkS)))
chkE("spec ymn min" lambda(() maeSetSpec("ymn" "TO" ?minimum "0" ?session chkS)))
chkE("spec inf info" lambda(() maeSetSpec("inf" "TO" ?info t ?session chkS)))
chkE("spec ymxa tol" lambda(() maeSetSpec("ymxa" "TO" ?tolerance '("0.1" "10%") ?session chkS)))
chkE("corner C1" lambda(() maeSetCorner("C1" ?enabled t ?session chkS)))
chkE("C1 temperature group" lambda(() maeSetVar("temperature" "-40,125" ?typeName "corner" ?typeValue '("C1") ?session chkS)))
chkE("corner C2 not for N1" lambda(() maeSetCorner("C2" ?enabled t ?disableTests '("N1") ?session chkS)))
chkE("C2 temperature" lambda(() maeSetVar("temperature" "85" ?typeName "corner" ?typeValue '("C2") ?session chkS)))
chkE("createTest TH hspiceD" lambda(() maeCreateTest("TH" ?lib chkLib ?cell "rc" ?view "schematic" ?simulator "hspiceD" ?session chkS)))
chkE("save" lambda(() maeSaveSetup(?lib chkLib ?cell "rc" ?view "maestro" ?session chkS)))
;; ADE's own netlists of the kinds, run by Spectre: what the Schemify block must match.
foreach(a cons('("TO") chkAna)
  let((nl)
    nl = chkNetlist(chkS car(a) strcat("ade_" car(a)))
    chkLine(sprintf(nil "ADE %s" car(a))
      list(setof(l car(nl) exists(w '("^noise " "^stb " "^xf " "^dcmatch " "^pss " "^pac " "^dc " "^ac " "^tran ") rexMatchp(w l)))
        cadr(nl)))))
chkE("close rc maestro" lambda(() maeCloseSession(?session chkS ?forceClose t)))
;; A second maestro view on rc: noted, not dumped.
let((sess)
  sess = chkE("open maestro_b" lambda(() maeOpenSetup(chkLib "rc" "maestro_b" ?mode "a")))
  chkE("maestro_b test" lambda(() maeCreateTest("TB2" ?lib chkLib ?cell "rc" ?view "schematic" ?simulator "spectre" ?session sess)))
  chkE("maestro_b save" lambda(() maeSaveSetup(?lib chkLib ?cell "rc" ?view "maestro_b" ?session sess)))
  chkE("close maestro_b" lambda(() maeCloseSession(?session sess ?forceClose t))))
chkThat("SfyExport rc" chkE("SfyExport" lambda(() SfyExport(strcat(chkW "/out/a_dump.sfyx") chkLib '("rc" "rr")))))
let((d)
  d = chkLines(strcat(chkW "/out/a_dump.sfyx"))
  chkThat("dump: the temperature var wins over the temp option" member("(\"var\" \"temperature\" \"-40,27,125\" \"\")" d))
  chkThat("dump: simulator options" member("(\"simoption\" \"reltol\" \"1e-5\")" d))
  chkThat("dump: N1 does not run C2" member("(\"cornerrun\" \"C2\" 0)" d))
  chkThat("dump: TR's design is rr" member("(\"testdesign\" \"sfyprobe_a\" \"rr\")" d))
  chkThat("dump: TH runs hspiceD" member("(\"maestro\" \"TH\" \"sfyprobe_a\" \"rc\" \"schematic\" \"hspiceD\")" d))
  chkThat("dump: the view maestro_b noted" chkHas(d '("maenote" "maestro view maestro_b")))
  chkThat("dump: evalType corners noted" chkHas(d '("maenote" "ymxa is evaluated over corners")))
  chkThat("dump: the min, info and tol specs noted" and(chkHas(d '("maenote" "spec of ymn")) chkHas(d '("maenote" "spec of inf")) chkHas(d '("maenote" "spec of ymxa")))))
system(sprintf(nil "rm -rf '%s'" chkSim))
