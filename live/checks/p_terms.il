;; p_terms.il: lane P live check (gaps2 P1 S8 G7), driven by p_drive.sh. Draws in chkLib,
;; with the PDK library $SFYCHECK_PLIB (default cmos22fdsoi; its nfet/pfet: three symbol
;; pins, the bulk an inherited terminal of the stopping views; its opnpcres: sx likewise):
;; - inv: pfet MP and nfet MN, pins IN OUT VDD VSS; inv_tb: inv I0 on labelled stubs;
;; - bulk: analogLib's 3-terminal nmos M0 (bulk [@bulk_n:%:gnd!] in spectre's stopping view,
;;   progn(bn) in auCdl's termOrder) and pmos M1 with bn set to "vdd!";
;; - res: the PDK resistor R0 between a and b, and a stub on net VSS!.
;; Then dumps inv_tb, bulk and res with their refs to out/p_terms.sfyx (SfyExport) and
;; prints each ref's dumped terms. p_drive.sh netlists the cells with si and compares.

;; A wire from inst's pin term to a label net, (dx dy) away from the pin centre.
procedure(chkPStub(cv inst term net dx dy)
  let((b c w)
    b = car(car(setof(x inst~>master~>terminals x~>name == term))~>pins)~>fig~>bBox
    c = dbTransformPoint(list((xCoord(car(b)) + xCoord(cadr(b))) / 2
                              (yCoord(car(b)) + yCoord(cadr(b))) / 2) inst~>transform)
    w = car(schCreateWire(cv "draw" "full" list(c list(xCoord(c) + dx yCoord(c) + dy))
          0.0625 0.0625 0.0))
    schCreateWireLabel(cv w list(xCoord(c) + dx yCoord(c) + dy) net "lowerLeft" "R0" "stick"
      0.0625 nil)))

procedure(chkPSet(inst name value)
  cdfFindParamByName(cdfGetInstCDF(inst) name)->value = value)

procedure(chkPPin(cv name dir xy)
  schCreatePin(cv dbOpenCellViewByType("basic"
    if(dir == "input" "ipin" if(dir == "output" "opin" "iopin")) "symbol")
    name dir nil xy "R0"))

let((plib cv p n x ok)
  plib = or(getShellEnvVar("SFYCHECK_PLIB") "cmos22fdsoi")
  ;; inv
  cv = dbOpenCellViewByType(chkLib "inv" "schematic" "schematic" "w")
  p = dbCreateInst(cv dbOpenCellViewByType(plib "pfet" "symbol") "MP" 0:1 "R0")
  n = dbCreateInst(cv dbOpenCellViewByType(plib "nfet" "symbol") "MN" 0:-1 "R0")
  chkPSet(p "w" "200n") chkPSet(n "w" "200n")
  chkPStub(cv p "g" "IN" -0.25 0) chkPStub(cv n "g" "IN" -0.25 0)
  chkPStub(cv p "d" "OUT" 0.25 0) chkPStub(cv n "d" "OUT" 0.25 0)
  chkPStub(cv p "s" "VDD" 0 0.25) chkPStub(cv n "s" "VSS" 0 -0.25)
  chkPPin(cv "IN" "input" -2:0) chkPPin(cv "OUT" "output" 2:0)
  chkPPin(cv "VDD" "inputOutput" 0:2.5) chkPPin(cv "VSS" "inputOutput" 0:-2.5)
  ;; The pins on their nets: labelled stubs.
  foreach(e list(list("IN" -2:0 -2:0.25) list("OUT" 2:0 2:0.25) list("VDD" 0:2.5 0.25:2.5)
                 list("VSS" 0:-2.5 0.25:-2.5))
    let((w) w = car(schCreateWire(cv "draw" "full" list(cadr(e) caddr(e)) 0.0625 0.0625 0.0))
      schCreateWireLabel(cv w caddr(e) car(e) "lowerLeft" "R0" "stick" 0.0625 nil)))
  chkLine("inv schCheck" schCheck(cv))
  dbSave(cv) dbClose(cv)
  schViewToView(chkLib "inv" chkLib "inv" "schematic" "symbol" "schSchemToPinList"
    "schPinListToSymbol")
  ;; inv_tb
  cv = dbOpenCellViewByType(chkLib "inv_tb" "schematic" "schematic" "w")
  x = dbCreateInst(cv dbOpenCellViewByType(chkLib "inv" "symbol") "I0" 0:0 "R0")
  chkPStub(cv x "IN" "in" -0.25 0) chkPStub(cv x "OUT" "out" 0.25 0)
  chkPStub(cv x "VDD" "vdd" 0 0.25) chkPStub(cv x "VSS" "gnd!" 0 -0.25)
  chkLine("inv_tb schCheck" schCheck(cv))
  dbSave(cv) dbClose(cv)
  ;; bulk
  cv = dbOpenCellViewByType(chkLib "bulk" "schematic" "schematic" "w")
  n = dbCreateInst(cv dbOpenCellViewByType("analogLib" "nmos" "symbol") "M0" 0:0 "R0")
  p = dbCreateInst(cv dbOpenCellViewByType("analogLib" "pmos" "symbol") "M1" 2:0 "R0")
  chkPSet(n "model" "nch") chkPSet(p "model" "pch") chkPSet(p "bn" "vdd!")
  foreach(e list(list(n "D" "n_D") list(n "G" "n_G") list(n "S" "n_S")
                 list(p "D" "n_D") list(p "G" "n_G") list(p "S" "p_S"))
    chkPStub(cv car(e) cadr(e) caddr(e) 0 0.25))
  ;; M1's bulk net vdd! (auCdl: "neither a global net nor defined on the current cellView").
  let((w) w = car(schCreateWire(cv "draw" "full" list(4:0 4:0.25) 0.0625 0.0625 0.0))
    schCreateWireLabel(cv w 4:0.25 "vdd!" "lowerLeft" "R0" "stick" 0.0625 nil))
  chkLine("bulk schCheck" schCheck(cv))
  dbSave(cv) dbClose(cv)
  ;; res
  cv = dbOpenCellViewByType(chkLib "res" "schematic" "schematic" "w")
  x = dbCreateInst(cv dbOpenCellViewByType(plib "opnpcres" "symbol") "R0" 0:0 "R0")
  chkPStub(cv x "in" "a" 0 0.25) chkPStub(cv x "out" "b" 0 -0.25)
  ;; Its resbulk defaults to [@lSup:%:VSS!], which si evaluates in this cellview: a net
  ;; VSS! must be there (OSSGLD-84 without it).
  let((w) w = car(schCreateWire(cv "draw" "full" list(1:0 1:0.25) 0.0625 0.0625 0.0))
    schCreateWireLabel(cv w 1:0.25 "VSS!" "lowerLeft" "R0" "stick" 0.0625 nil))
  chkLine("res schCheck" schCheck(cv))
  dbSave(cv) dbClose(cv)
  ;; auCdl's progn(bn) as the dump spells it reads back as the list (SfySimElem).
  chkThat("progn(bn) reads back as a list"
    let((b) b = nth(3 caddr(cadr(SfySimInfoValue(
                  '("siminfo" "auCdl" (("termOrder" ("D" "G" "S" "progn(bn)"))))))))
      chkLine("progn(bn) back" b)
      listp(b) && car(b) == 'progn && cadr(b) == 'bn))
  ;; The dump.
  ok = SfyExport(strcat(getShellEnvVar("SFYCHECK_WORK") "/out/p_terms.sfyx") chkLib
         list("inv_tb" "bulk" "res"))
  chkThat("SfyExport of inv_tb bulk res" ok)
  chkPass("p_terms drawn"))
