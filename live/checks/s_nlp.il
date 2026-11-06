;; s_nlp.il: lane S, gaps2 G23 (an NLP label with several [@...] groups in one text) and S17
;; (radio CDF params). Lists the NLP labels of every analogLib symbol that hold more than one
;; group, the NLP functions this Virtuoso has, draws a symbol with a two-group label and a
;; radio param, dumps it with this checkout's SfyExport, loads the dump into sfyprobe_s2
;; (deleted at the end) and reads the label text and the param type back.

chkWork = getShellEnvVar("SFYCHECK_WORK")
chkDump = strcat(chkWork "/out/s_nlp.sfyx")
chkTwo = "[@w:w=%:w?][@l: l=%]"

;; Counts the "[@" groups of a label text.
procedure(chkGroups(s)
  length(parseString(strcat(" " s " ") "[")) - 1)

chkE("NLP functions" lambda(() listFunctions("[Nn][Ll][Pp]")))
chkE("analogLib multi-group NLP labels" lambda(() let((out cv)
  foreach(c ddGetObj("analogLib")~>cells~>name
    when(ddGetObj("analogLib" c "symbol")
      cv = dbOpenCellViewByType("analogLib" c "symbol" nil "r")
      foreach(l setof(x cv~>shapes x~>objType == "label" && x~>labelType == "NLPLabel")
        when(chkGroups(l~>theLabel) > 1 out = cons(list(c l~>theLabel) out)))
      dbClose(cv)))
  out)))
chkE("nmos4 labels" lambda(() let((cv) cv = dbOpenCellViewByType("analogLib" "nmos4" "symbol" nil "r")
  prog1(mapcar(lambda((l) list(l~>theLabel l~>labelType l~>layerName l~>purpose))
      setof(x cv~>shapes x~>objType == "label"))
    dbClose(cv)))))

;; A symbol with a two-group NLP label and a radio param.
let((cv net fig cdf)
  cv = dbOpenCellViewByType(chkLib "two" "symbol" "schematicSymbol" "w")
  dbCreateRect(cv list("device" "drawing") list(-0.5:-0.25 0.5:0.25))
  net = dbMakeNet(cv "a")
  dbCreateTerm(net "a" "inputOutput")
  fig = dbCreateRect(cv list("pin" "drawing") list(-0.5125:-0.0125 -0.4875:0.0125))
  dbCreatePin(net fig "a")
  chkE("two label" lambda(() let((l) l = dbCreateLabel(cv list("annotate" "drawing") 0:0.3 chkTwo "lowerLeft" "R0" "stick" 0.0625)
    l~>labelType = "NLPLabel" list(l~>theLabel l~>labelType))))
  dbSave(cv) dbClose(cv)
  cdf = cdfCreateBaseCellCDF(ddGetObj(chkLib "two"))
  cdfCreateParam(cdf ?name "w" ?prompt "w" ?type "string" ?defValue "1u")
  cdfCreateParam(cdf ?name "l" ?prompt "l" ?type "string" ?defValue "")
  cdfCreateParam(cdf ?name "style" ?prompt "style" ?type "radio" ?choices '("a" "b") ?defValue "a")
  cdfSaveCDF(cdf))
chkCv = dbOpenCellViewByType(chkLib "twotop" "schematic" "schematic" "w")
chkI = schCreateInst(chkCv dbOpenCellViewByType(chkLib "two" "symbol") "X0" 0:0 "R0")
dbReplaceProp(chkI "w" "string" "3u")
;; Whatever an NLP evaluator this Virtuoso has, tried on the instance's label.
chkE("try evaluators" lambda(() let((l out)
  l = car(setof(x chkI~>master~>shapes x~>objType == "label"))
  foreach(f listFunctions("[Nn][Ll][Pp]")
    out = cons(list(f car(errset(apply(f list(l chkI)) nil))) out))
  out)))
dbSave(chkCv) dbClose(chkCv)

chkE("SfyExport" lambda(() SfyExport(chkDump chkLib '("twotop"))))
chkLib2 = "sfyprobe_s2"
chkE("lib2" lambda(() ddCreateLib(chkLib2 strcat(chkWork "/" chkLib2))))
chkE("lib2 tech" lambda(() techBindTechFile(ddGetObj(chkLib2) "cdsDefTechLib")))
chkE("SfyImport into lib2" lambda(() SfyImport(chkDump ?lib chkLib2 ?overwrite t ?check nil)))
chkBack = car(errset(let((cv) cv = dbOpenCellViewByType(chkLib2 "two" "symbol" nil "r")
  prog1(car(setof(x cv~>shapes x~>objType == "label" && x~>labelType == "NLPLabel"))~>theLabel dbClose(cv)))))
chkLine("lib2 label" chkBack)
chkThat("a two-group NLP label loads back as drawn" chkBack == chkTwo)
chkE("lib2 style paramType" lambda(() cdfFindParamByName(cdfGetBaseCellCDF(ddGetObj(chkLib2 "two")) "style")~>paramType))
chkE("drop lib2" lambda(() ddDeleteObj(ddGetObj(chkLib2))))
