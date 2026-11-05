;; s_more.il: lane S, gaps round 2, second pass (inv_schematic S4 S7 S10 S14 S16 S19 S20).
;; Draws the cases in the lane library, prints what Virtuoso holds, dumps them with this
;; checkout's SfyExport to $SFYCHECK_WORK/out/s_more.sfyx, loads that dump into a second
;; library (sfyprobe_s2, deleted at the end) and reads the CDF back from there.
;;   beh     a symbol plus a spectre text view and a verilogams view; old: only cmos_sch (S4)
;;   blk     a base CDF with doneProc, formInitProc, paramEvaluate, display modes, and a
;;           param with display/editable/use expressions (S7)
;;   lbl     paramLabelSet "-model l w": what cdsParam(1..3) show under each
;;           paramDisplayMode, and analogLib nmos4's (S16)
;;   pcs     a SKILL PCell symbol whose pin count is a parameter (S14)
;;   cg      a schematic with a Constraint Manager constraint (S19)
;;   dirty   a schematic open with an unsaved wire at export time (S20)
;;   the library CDF and a libInit.il (S10)

chkRes = dbOpenCellViewByType("analogLib" "res" "symbol")
chkWork = getShellEnvVar("SFYCHECK_WORK")
chkDump = strcat(chkWork "/out/s_more.sfyx")

procedure(chkSym(cell view pins)
  let((cv)
    cv = dbOpenCellViewByType(chkLib cell view "schematicSymbol" "w")
    dbCreateRect(cv list("device" "drawing") list(-0.5:-0.25 0.5:0.25))
    foreach(kv pins
      let((net term fig)
        net = dbMakeNet(cv car(kv))
        term = dbCreateTerm(net car(kv) "inputOutput")
        fig = dbCreateRect(cv list("pin" "drawing") list(list(cadr(kv) - 0.0125 -0.0125) list(cadr(kv) + 0.0125 0.0125)))
        dbCreatePin(net fig car(kv))))
    dbSave(cv) dbClose(cv)))

;; A text file at the library's lib/cell/view, with its master.tag.
procedure(chkText(cell view file text)
  let((d p)
    d = strcat(ddGetObj(chkLib)~>readPath "/" cell "/" view)
    createDirHier(d)
    p = outfile(strcat(d "/" file)) fprintf(p "%s" text) close(p)
    p = outfile(strcat(d "/master.tag")) fprintf(p "-- Master.tag File, Rev:1.0\n%s\n" file) close(p)
    t))

procedure(chkResSch(cell view)
  let((cv)
    cv = dbOpenCellViewByType(chkLib cell view "schematic" "w")
    schCreateInst(cv chkRes "R0" 0:0 "R0")
    dbSave(cv) dbClose(cv)))

;; ---- S4
chkSym("beh" "symbol" '(("a" -0.5) ("b" 0.5)))
chkE("beh spectre text" lambda(() chkText("beh" "spectre" "spectre.scs" "subckt beh a b\nR0 (a b) resistor r=1k\nends beh\n")))
chkE("beh verilogams text" lambda(() chkText("beh" "verilogams" "verilog.vams" "module beh(a, b); inout a, b; endmodule\n")))
chkE("ddsync" lambda(() ddReleaseObj(ddGetObj(chkLib))))
chkE("beh views" lambda(() ddGetObj(chkLib "beh")~>views~>name))
chkE("beh spectre files" lambda(() ddGetObj(chkLib "beh" "spectre")~>files~>name))
chkResSch("old" "cmos_sch")
chkE("old views" lambda(() ddGetObj(chkLib "old")~>views~>name))

;; ---- S7
chkSym("blk" "symbol" '(("a" -0.5) ("b" 0.5)))
chkResSch("blk" "schematic")
let((cdf)
  cdf = cdfCreateBaseCellCDF(ddGetObj(chkLib "blk"))
  cdfCreateParam(cdf ?name "mode" ?prompt "mode" ?type "cyclic" ?choices '("a" "b") ?defValue "a")
  cdfCreateParam(cdf ?name "w" ?prompt "w" ?type "string" ?defValue "1u"
    ?display "cdfgData->mode->value==\"a\"" ?editable "nil" ?use "t")
  chkE("blk set" lambda(() cdf->doneProc = "G_done" cdf->formInitProc = "G_init"
    cdf->paramEvaluate = "t nil nil nil nil" cdf->instDisplayMode = "instName"
    cdf->opPointLabelSet = "id" cdf->paramDisplayMode = "value" t))
  chkE("blk save" lambda(() cdfSaveCDF(cdf))))
procedure(chkCdfAttrs(lib)
  let((d p)
    d = cdfGetBaseCellCDF(ddGetObj(lib "blk"))
    p = cdfFindParamByName(d "w")
    list(d->doneProc d->formInitProc d->paramEvaluate d->instDisplayMode d->opPointLabelSet
      d->paramDisplayMode p->display p->editable p->use)))
chkE("blk attrs" lambda(() chkCdfAttrs(chkLib)))
chkE("blk cdf attribute names" lambda(() cdfGetBaseCellCDF(ddGetObj(chkLib "blk"))->?))
chkE("blk param attribute names" lambda(() car(cdfGetBaseCellCDF(ddGetObj(chkLib "blk"))->parameters)->?))

;; ---- S16: label values through cdsParam, as the symbol editor evaluates them.
chkSym("lbl" "symbol" '(("a" -0.5) ("b" 0.5)))
let((cdf)
  cdf = cdfCreateBaseCellCDF(ddGetObj(chkLib "lbl"))
  cdfCreateParam(cdf ?name "model" ?prompt "model" ?type "string" ?defValue "nch")
  cdfCreateParam(cdf ?name "l" ?prompt "l" ?type "string" ?defValue "1u")
  cdfCreateParam(cdf ?name "w" ?prompt "w" ?type "string" ?defValue "2u")
  cdf->paramLabelSet = "-model l w"
  cdfSaveCDF(cdf))
procedure(chkLabels(inst)
  let((ilInst)
    ilInst = inst
    mapcar(lambda((k) car(errset(evalstring(sprintf(nil "cdsParam(%d)" k))))) '(1 2 3 4))))
chkCv = dbOpenCellViewByType(chkLib "lbltop" "schematic" "schematic" "w")
chkI = schCreateInst(chkCv dbOpenCellViewByType(chkLib "lbl" "symbol") "X0" 0:0 "R0")
chkE("lbl defaults" lambda(() chkLabels(chkI)))
chkE("lbl set l" lambda(() cdfFindParamByName(cdfGetInstCDF(chkI) "l")->value = "3u" chkLabels(chkI)))
foreach(m '("value" "name" "both" "none")
  chkE(sprintf(nil "lbl paramDisplayMode %s" m) lambda(() let((cdf)
    cdf = cdfGetBaseCellCDF(ddGetObj(chkLib "lbl")) cdf->paramDisplayMode = m cdfSaveCDF(cdf)
    chkLabels(chkI)))))
chkN = schCreateInst(chkCv dbOpenCellViewByType("analogLib" "nmos4" "symbol") "M0" 2:0 "R0")
chkE("nmos4 labelSet displayMode" lambda(() list(cdfGetCellCDF(ddGetObj("analogLib" "nmos4"))->paramLabelSet
  cdfGetCellCDF(ddGetObj("analogLib" "nmos4"))->paramDisplayMode)))
chkE("nmos4 defaults" lambda(() chkLabels(chkN)))
chkE("nmos4 set" lambda(() cdfFindParamByName(cdfGetInstCDF(chkN) "model")->value = "nch"
  cdfFindParamByName(cdfGetInstCDF(chkN) "w")->value = "3u" chkLabels(chkN)))
dbSave(chkCv) dbClose(chkCv)

;; ---- S14: a PCell symbol with np pins.
chkE("pcs define" lambda(() pcDefinePCell(list(ddGetObj(chkLib) "pcs" "symbol" "schematicSymbol") ((np 2))
  let((cv)
    cv = pcCellView
    dbCreateRect(cv list("device" "drawing") list(-0.5:-0.25 0.5:0.25))
    for(k 1 np
      let((net fig x)
        x = -0.5 + float(k - 1) / float(np)
        net = dbMakeNet(cv sprintf(nil "p%d" k))
        dbCreateTerm(net sprintf(nil "p%d" k) "inputOutput")
        fig = dbCreateRect(cv list("pin" "drawing") list(list(x - 0.0125 -0.0125) list(x + 0.0125 0.0125)))
        dbCreatePin(net fig sprintf(nil "p%d" k))))
    t))))
chkCv = dbOpenCellViewByType(chkLib "pctop" "schematic" "schematic" "w")
chkP = car(errset(dbCreateParamInst(chkCv dbOpenCellViewByType(chkLib "pcs" "symbol") "P0" 0:0 "R0" 1 list(list("np" "int" 3)))))
chkE("pcs super isParamCell terms" lambda(() let((s) s = dbOpenCellViewByType(chkLib "pcs" "symbol")
  list(s~>isParamCell s~>superMaster s~>terminals~>name))))
chkE("pcs inst master" lambda(() list(chkP~>master~>isParamCell chkP~>master~>superMaster~>cellName chkP~>master~>terminals~>name)))
dbSave(chkCv) dbClose(chkCv)

;; ---- S19: a matched-parameters constraint between two instances.
chkCv = dbOpenCellViewByType(chkLib "cg" "schematic" "schematic" "w")
schCreateInst(chkCv chkRes "R0" 0:0 "R0")
schCreateInst(chkCv chkRes "R1" 2:0 "R0")
dbSave(chkCv)
chkE("cg constraintGroups before" lambda(() chkCv~>constraintGroups))
chkE("cg ciConCreate" lambda(() ciConCreate(ciCacheGet(chkCv) 'matchedParameters
  ?members list(list("R0" 'inst) list("R1" 'inst)))))
chkE("cg ciCacheSave" lambda(() ciCacheSave(ciCacheGet(chkCv))))
chkE("cg save" lambda(() dbSave(chkCv)))
chkE("cg constraintGroups" lambda(() mapcar(lambda((g) list(g~>name g~>objType length(g~>members)))
  chkCv~>constraintGroups)))
chkE("cg cache constraints" lambda(() length(ciCacheGet(chkCv)~>constraints)))
dbClose(chkCv)

;; ---- S10: a library CDF and a libInit.il.
chkE("libcdf" lambda(() let((d) d = cdfCreateBaseLibCDF(ddGetObj(chkLib))
  cdfCreateParam(d ?name "corner" ?prompt "corner" ?type "string" ?defValue "tt") cdfSaveCDF(d))))
chkE("libInit.il" lambda(() let((p) p = outfile(strcat(ddGetObj(chkLib)~>readPath "/libInit.il"))
  fprintf(p "procedure(G_cb() t)\n") close(p) t)))

;; ---- S20: an unsaved wire at export time.
chkCv = dbOpenCellViewByType(chkLib "dirty" "schematic" "schematic" "w")
schCreateInst(chkCv chkRes "R0" 0:0 "R0")
dbSave(chkCv) dbClose(chkCv)
chkCv = dbOpenCellViewByType(chkLib "dirty" "schematic" "schematic" "a")
schCreateWire(chkCv "draw" "full" list(0:0 0.5:0) 0.0625 0.0625 0.0)
chkE("dirty modifiedButNotSaved" lambda(() chkCv~>modifiedButNotSaved))

;; ---- the dump, and the loader on it
chkE("SfyExport" lambda(() SfyExport(chkDump chkLib '("beh" "blk" "cg" "dirty" "lbltop" "old" "pctop"))))
dbPurge(chkCv)
procedure(chkGrep(pat)
  let((p l out)
    p = infile(chkDump)
    while(gets(l p) when(rexMatchp(pat l) out = cons(l out)))
    close(p)
    reverse(out)))
chkE("cg views" lambda(() ddGetObj(chkLib "cg")~>views~>name))
procedure(chkCount(tags)
  let((n) n = 0
    foreach(x tags n = n + length(chkGrep(strcat("^(\"" x "\""))))
    n))
chkE("dump records" lambda(() mapcar(lambda((x) list(x chkCount(list(x))))
  '("textview" "skipview" "cdfattr" "paramattr" "libcdf" "skipped"))))
chkLib2 = "sfyprobe_s2"
chkE("lib2" lambda(() ddCreateLib(chkLib2 strcat(chkWork "/" chkLib2))))
chkE("lib2 tech" lambda(() techBindTechFile(ddGetObj(chkLib2) "cdsDefTechLib")))
chkE("SfyImport into lib2" lambda(() SfyImport(chkDump ?lib chkLib2 ?overwrite t ?check nil)))
chkE("lib2 blk attrs" lambda(() chkCdfAttrs(chkLib2)))
chkThat("the loader brings blk's CDF attributes back" equal(car(errset(chkCdfAttrs(chkLib))) car(errset(chkCdfAttrs(chkLib2)))))
;; A second load over the library, from a dump without them (an edited cell): kept.
chkE("strip attrs" lambda(() let((p q l)
  p = infile(chkDump) q = outfile(strcat(chkDump ".bare"))
  while(gets(l p) unless(rexMatchp("^(\"cdfattr\"" l) || rexMatchp("^(\"paramattr\"" l) fprintf(q "%s" l)))
  close(p) close(q) t)))
chkE("SfyImport bare over lib2" lambda(() SfyImport(strcat(chkDump ".bare") ?lib chkLib2 ?overwrite t ?check nil)))
chkThat("a load without them keeps the library's CDF attributes" equal(car(errset(chkCdfAttrs(chkLib))) car(errset(chkCdfAttrs(chkLib2)))))
chkE("lib2 beh spectre files (not a symbol copy)" lambda(() ddGetObj(chkLib2 "beh" "spectre")~>files~>name))
chkE("drop lib2" lambda(() ddDeleteObj(ddGetObj(chkLib2))))
chkThat("the dump names the text and other views" chkCount('("textview" "skipview")) >= 4)
chkThat("the dump carries the CDF attributes" chkCount('("cdfattr" "paramattr")) >= 6)
chkThat("the dump flags the PCell variant" length(chkGrep("pcell variant pins of P0")) == 1)
chkThat("the dump flags the library CDF" length(chkGrep("^(\"libcdf\"")) == 1)
chkThat("the dump flags the unsaved edits" length(chkGrep("unsaved")) == 1)
