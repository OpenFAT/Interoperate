;; s_schematic.il: lane S (gaps round 2, inv_schematic S1 S3 S5 S6 S11 G8 E1). Draws, in the
;; lane library, the cells whose Virtuoso connectivity the import must match, prints
;; Virtuoso's own instTerm -> net table for each (after schCheck), and dumps them with
;; this checkout's SfyExport to $SFYCHECK_WORK/out/s_schematic.sfyx (the dump carries the
;; same table as `tnet` records). s_drive.sh netlists the kept library with si and compares
;; on the Schemify side; tests/virtuoso_gaps.rs grades the committed copy.
;;   conn    a wire ending on a pin box's edge, inside it off centre, on its corner, just
;;           outside it; a wire through a box off its centre; a pin centre mid-wire; pins
;;           abutting with one centre and overlapping off centre; crossing wires; an unglued
;;           label; a T; a copied analogLib vdd as a user supply symbol (S11)
;;   alt     sub placed through symbol and symbol_alt (pins elsewhere), analogLib res
;;           through symbol_xform (S3)
;;   ordtop  ord, whose symbol pins were created z a m, no CDF (G8 port order)
;;   e1      analogLib nmos and res at their CDF defaults (E1: empty values)
;;   ms      a multi-sheet schematic (S5): what the index holds
;;   skip    a dot and a donut in a symbol, a user cellview property (S6)
;;   pc      basic cds_thru between two nets, basic patch's simInfo (S12)

chkRes = dbOpenCellViewByType("analogLib" "res" "symbol")

;; Centre and bBox of an instance terminal's first pin, in the schematic's coordinates.
procedure(chkPin(inst tn)
  let((term bb)
    term = car(setof(x inst~>master~>terminals x~>name == tn))
    bb = dbTransformBBox(car(term~>pins)~>fig~>bBox inst~>transform)
    list(list((xCoord(car(bb)) + xCoord(cadr(bb))) / 2.0 (yCoord(car(bb)) + yCoord(cadr(bb))) / 2.0) bb)))

procedure(chkWire(cv pts @optional label)
  let((w)
    w = schCreateWire(cv "draw" "full" pts 0.0625 0.0625 0.0)
    when(label
      schCreateWireLabel(cv car(w) car(last(pts)) label "lowerLeft" "R0" "stick" 0.0625 nil))
    w))

;; A labelled stub to the right of each (terminal net) pair's pin centre.
procedure(chkStub(cv inst nets)
  foreach(kv nets
    let((c)
      c = car(chkPin(inst car(kv)))
      chkWire(cv list(c list(xCoord(c) + 0.25 yCoord(c))) cadr(kv)))))

;; Virtuoso's instTerm -> net table and its nets.
procedure(chkConn(label cv)
  foreach(i cv~>instances
    chkLine(sprintf(nil "%s conn %s" label i~>name)
      sort(mapcar(lambda((it) list(it~>name it~>net~>name)) i~>instTerms)
        lambda((a b) alphalessp(car(a) car(b))))))
  chkLine(sprintf(nil "%s nets" label) sort(cv~>nets~>name 'alphalessp)))

;; schCheck, its markers' messages when it complains, and the save.
procedure(chkCheck(label cv)
  let((r)
    r = chkE(strcat(label " check") lambda(() schCheck(cv)))
    when(r && (car(r) > 0 || cadr(r) > 0)
      chkE(strcat(label " markers") lambda(() mapcar(lambda((m) list(m~>msg m~>shortMsg)) cv~>markers)))
      chkE(strcat(label " marker shapes") lambda(() mapcar(lambda((s) list(s~>objType s~>purpose s~>theLabel s~>bBox))
        setof(s cv~>shapes s~>layerName == "marker")))))
    dbSave(cv)
    r))

;; A hand-drawn symbol: a box and one square pin per (name x) at y 0.
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

;; A schematic of one analogLib res between pins named a and b (cell's symbol: chkSym).
procedure(chkSub(cell pins)
  let((cv r)
    cv = dbOpenCellViewByType(chkLib cell "schematic" "schematic" "w")
    r = schCreateInst(cv chkRes "RS0" 0:0 "R0")
    chkStub(cv r list(list("PLUS" car(pins)) list("MINUS" cadr(pins))))
    foreach(p pins
      schCreatePin(cv dbOpenCellViewByType("basic" "iopin" "symbol") p "inputOutput" nil
        list(-2 if(p == car(pins) 1 -1)) "R0"))
    chkCheck(cell cv)
    dbClose(cv)))

;; ---- conn (S1, S11): res pins are 0.05 squares, PLUS at the origin, MINUS at 0:-0.375.
chkCv = dbOpenCellViewByType(chkLib "conn" "schematic" "schematic" "w")
chkR = list(nil)
foreach(k '(1 2 3 5 7 8)
  chkR = append1(chkR list(k schCreateInst(chkCv chkRes sprintf(nil "R%d" k) list(2.0 * (k - 1) 0) "R0"))))
procedure(chkI(k) cadr(assoc(k cdr(chkR))))
chkLine("res PLUS pin" chkPin(chkI(1) "PLUS"))
chkLine("res MINUS pin" chkPin(chkI(1) "MINUS"))
let((c bb)
  ;; R1 PLUS: a wire ending on the box's left edge (n_edge); R1 MINUS: one ending inside the
  ;; box, off its centre (n_in).
  c = car(chkPin(chkI(1) "PLUS")) bb = cadr(chkPin(chkI(1) "PLUS"))
  chkWire(chkCv list(list(xCoord(car(bb)) yCoord(c)) list(xCoord(car(bb)) - 0.5 yCoord(c))) "n_edge")
  c = car(chkPin(chkI(1) "MINUS"))
  chkWire(chkCv list(list(xCoord(c) + 0.0125 yCoord(c) - 0.0125) list(xCoord(c) + 0.0125 yCoord(c) - 0.5)) "n_in")
  ;; R2 PLUS: a wire through the centre (n_mid); R2 MINUS: a wire through the box off its
  ;; centre (n_off).
  c = car(chkPin(chkI(2) "PLUS"))
  chkWire(chkCv list(list(xCoord(c) - 0.5 yCoord(c)) list(xCoord(c) + 0.5 yCoord(c))) "n_mid")
  c = car(chkPin(chkI(2) "MINUS"))
  chkWire(chkCv list(list(xCoord(c) - 0.5 yCoord(c) + 0.0125) list(xCoord(c) + 0.5 yCoord(c) + 0.0125)) "n_off")
  ;; R4's PLUS on R3's MINUS (one centre, no wire); R6's PLUS box overlapping R5's MINUS box
  ;; off centre (0.025 apart).
  c = car(chkPin(chkI(3) "MINUS"))
  chkR = append1(chkR list(4 schCreateInst(chkCv chkRes "R4" list(xCoord(c) yCoord(c)) "R0")))
  c = car(chkPin(chkI(5) "MINUS"))
  chkR = append1(chkR list(6 schCreateInst(chkCv chkRes "R6" list(xCoord(c) + 0.025 yCoord(c)) "R0")))
  ;; R7 PLUS: a wire ending on the box's corner (n_corner); R7 MINUS: one ending 1/160 inch
  ;; outside the box (n_out).
  bb = cadr(chkPin(chkI(7) "PLUS"))
  chkWire(chkCv list(list(xCoord(car(bb)) yCoord(cadr(bb))) list(xCoord(car(bb)) - 0.5 yCoord(cadr(bb)))) "n_corner")
  bb = cadr(chkPin(chkI(7) "MINUS")) c = car(chkPin(chkI(7) "MINUS"))
  chkWire(chkCv list(list(xCoord(car(bb)) - 0.00625 yCoord(c)) list(xCoord(car(bb)) - 0.5 yCoord(c))) "n_out")
  ;; R8 PLUS up to the middle of a bar (T, n_tee); R8 MINUS down a wire with a label over it,
  ;; glued to nothing (n_float).
  c = car(chkPin(chkI(8) "PLUS"))
  chkWire(chkCv list(list(xCoord(c) - 0.5 yCoord(c) + 0.5) list(xCoord(c) + 0.5 yCoord(c) + 0.5)) "n_tee")
  chkWire(chkCv list(c list(xCoord(c) yCoord(c) + 0.5)))
  c = car(chkPin(chkI(8) "MINUS"))
  chkWire(chkCv list(c list(xCoord(c) yCoord(c) - 0.5)))
  dbCreateLabel(chkCv list("wire" "label") list(xCoord(c) yCoord(c) - 0.25) "n_float" "lowerLeft" "R0" "stick" 0.0625))
;; Two wires crossing with no dot.
chkWire(chkCv list(-1:1.5 1:1.5) "n_ch")
chkWire(chkCv list(0:1 0:2) "n_cv")
chkStub(chkCv chkI(4) '(("MINUS" "n4m")))
chkStub(chkCv chkI(5) '(("PLUS" "n5p")))
chkStub(chkCv chkI(6) '(("MINUS" "n6m")))
;; S11: a copy of analogLib vdd as a user supply symbol, wired up to R3 PLUS (net vdd!).
dbCopyCellView(dbOpenCellViewByType("analogLib" "vdd" "symbol") chkLib "myvdd" "symbol" nil nil t)
let((c v)
  c = car(chkPin(chkI(3) "PLUS"))
  v = schCreateInst(chkCv dbOpenCellViewByType(chkLib "myvdd" "symbol") "V9" list(xCoord(c) yCoord(c) + 0.5) "R0")
  chkLine("myvdd terms" v~>master~>terminals~>name)
  chkWire(chkCv list(c car(chkPin(v car(v~>master~>terminals~>name))))))
chkCheck("conn" chkCv)
chkConn("conn" chkCv)
chkLine("conn cellview props" chkCv~>prop~>name)
dbClose(chkCv)

;; ---- alt (S3): sub's symbol pins A -0.5, B 0.5; symbol_alt A -0.25, B 0.25.
chkSub("sub" '("A" "B"))
chkSym("sub" "symbol" '(("A" -0.5) ("B" 0.5)))
chkSym("sub" "symbol_alt" '(("A" -0.25) ("B" 0.25)))
chkCv = dbOpenCellViewByType(chkLib "alt" "schematic" "schematic" "w")
chkStub(chkCv schCreateInst(chkCv dbOpenCellViewByType(chkLib "sub" "symbol") "S0" 0:0 "R0") '(("A" "a0") ("B" "b0")))
chkStub(chkCv schCreateInst(chkCv dbOpenCellViewByType(chkLib "sub" "symbol_alt") "S1" 0:2 "R0") '(("A" "a1") ("B" "b1")))
chkE("res symbol_xform pins" lambda(() let((cv) cv = dbOpenCellViewByType("analogLib" "res" "symbol_xform")
  mapcar(lambda((x) list(x~>name car(x~>pins)~>fig~>bBox)) cv~>terminals))))
chkE("res symbol pins" lambda(() mapcar(lambda((x) list(x~>name car(x~>pins)~>fig~>bBox)) chkRes~>terminals)))
chkStub(chkCv schCreateInst(chkCv dbOpenCellViewByType("analogLib" "res" "symbol_xform") "RX" 4:0 "R0") '(("PLUS" "xp") ("MINUS" "xm")))
chkCheck("alt" chkCv)
chkConn("alt" chkCv)
chkLine("alt inst views" mapcar(lambda((i) list(i~>name i~>viewName)) chkCv~>instances))
dbClose(chkCv)

;; ---- ordtop (G8): ord's schematic pins a m z, its symbol's terminals made z a m, no CDF.
chkSub("ord" '("z" "m"))
let((cv)
  ;; the third pin, a, on its own stub
  cv = dbOpenCellViewByType(chkLib "ord" "schematic" "schematic" "a")
  chkWire(cv list(-1:-2 -0.5:-2) "a")
  schCreatePin(cv dbOpenCellViewByType("basic" "iopin" "symbol") "a" "inputOutput" nil -1:-2 "R0")
  chkCheck("ord" cv)
  dbClose(cv))
chkSym("ord" "symbol" '(("z" -0.5) ("a" 0) ("m" 0.5)))
chkLine("ord symbol terminals order" dbOpenCellViewByType(chkLib "ord" "symbol")~>terminals~>name)
chkLine("ord cdf" cdfGetBaseCellCDF(ddGetObj(chkLib "ord")))
chkCv = dbOpenCellViewByType(chkLib "ordtop" "schematic" "schematic" "w")
chkStub(chkCv schCreateInst(chkCv dbOpenCellViewByType(chkLib "ord" "symbol") "X0" 0:0 "R0") '(("z" "nz") ("a" "na") ("m" "nm")))
chkCheck("ordtop" chkCv)
chkConn("ordtop" chkCv)
dbClose(chkCv)

;; ---- e1 (E1): analogLib nmos (3 terminals) and res at their defaults.
chkCv = dbOpenCellViewByType(chkLib "e1" "schematic" "schematic" "w")
let((m)
  m = schCreateInst(chkCv dbOpenCellViewByType("analogLib" "nmos" "symbol") "M0" 0:0 "R0")
  chkStub(chkCv m mapcar(lambda((x) list(x strcat("n" x))) m~>master~>terminals~>name))
  chkLine("nmos props" mapcar(lambda((p) list(p~>name p~>value)) m~>prop))
  chkLine("nmos cdf empties" mapcar(lambda((p) p~>name) setof(p cdfGetInstCDF(m)~>parameters p~>value == ""))))
let((r)
  r = schCreateInst(chkCv chkRes "R0" 2:0 "R0")
  cdfFindParamByName(cdfGetInstCDF(r) "r")~>value = "1k"
  chkStub(chkCv r '(("PLUS" "nD") ("MINUS" "nS")))
  chkLine("res cdf empties" mapcar(lambda((p) p~>name) setof(p cdfGetInstCDF(r)~>parameters p~>value == ""))))
chkCheck("e1" chkCv)
dbClose(chkCv)

;; ---- ms (S5): a multi-sheet schematic, the index and its sheets.
chkCv = dbOpenCellViewByType(chkLib "ms" "schematic" "schematic" "w")
schCreateInst(chkCv chkRes "R0" 0:0 "R0")
dbSave(chkCv)
chkE("ms sheet 1" '(schCreateSheet chkCv 1 "US_8ths" "Asize" "symbol"))
chkE("ms sheet 2" '(schCreateSheet chkCv 2 "US_8ths" "Asize" "symbol"))
chkE("ms isIndex" '(schIsIndexCV chkCv))
chkE("ms save" '(dbSave chkCv))
chkLine("ms views" ddGetObj(chkLib "ms")~>views~>name)
chkLine("ms index instances" mapcar(lambda((i) list(i~>name i~>libName i~>cellName i~>viewName)) chkCv~>instances))
dbClose(chkCv)

;; ---- skip (S6): what the dumper leaves out, in a symbol and a schematic.
let((cv)
  cv = dbOpenCellViewByType(chkLib "skip" "symbol" "schematicSymbol" "w")
  dbCreateRect(cv list("device" "drawing") list(-0.5:-0.25 0.5:0.25))
  let((net fig)
    net = dbMakeNet(cv "A")
    dbCreateTerm(net "A" "inputOutput")
    fig = dbCreateRect(cv list("pin" "drawing") list(-0.5125:-0.0125 -0.4875:0.0125))
    dbCreatePin(net fig "A"))
  chkE("skip dot" lambda(() dbCreateDot(cv list("device" "drawing") 0:0)))
  chkE("skip donut" lambda(() dbCreateDonut(cv list("device" "drawing") 0.2:0 0.05 0.02)))
  dbSave(cv) dbClose(cv)
  cv = dbOpenCellViewByType(chkLib "skip" "schematic" "schematic" "w")
  schCreatePin(cv dbOpenCellViewByType("basic" "iopin" "symbol") "A" "inputOutput" nil -1:0 "R0")
  dbReplaceProp(cv "teamNote" "string" "x")
  chkCheck("skip" cv)
  dbClose(cv))

;; ---- pc (S12): basic cds_thru between R0 MINUS (x) and R1 PLUS (y).
chkCv = dbOpenCellViewByType(chkLib "pc" "schematic" "schematic" "w")
chkStub(chkCv schCreateInst(chkCv chkRes "R0" 0:0 "R0") '(("PLUS" "x0") ("MINUS" "x")))
chkStub(chkCv schCreateInst(chkCv chkRes "R1" 2:0 "R0") '(("PLUS" "y") ("MINUS" "y0")))
chkTh = schCreateInst(chkCv dbOpenCellViewByType("basic" "cds_thru" "symbol") "T0" 1:-1 "R0")
chkE("cds_thru terms" lambda(() chkTh~>master~>terminals~>name))
chkE("cds_thru pins" lambda(() mapcar(lambda((x) chkPin(chkTh x)) chkTh~>master~>terminals~>name)))
chkE("cds_thru cdf" lambda(() cdfGetBaseCellCDF(ddGetObj("basic" "cds_thru"))~>simInfo))
chkE("cds_thru stubs" lambda(() chkStub(chkCv chkTh '(("src" "x") ("dst" "y")))))
chkE("patch simInfo (none: no netlist form)" lambda(() cdfGetBaseCellCDF(ddGetObj("basic" "patch"))~>simInfo))
chkCheck("pc" chkCv)
chkConn("pc" chkCv)
dbClose(chkCv)

;; ---- the dump (tnet records) and what it holds.
chkE("SfyExport" lambda(() SfyExport(strcat(getShellEnvVar("SFYCHECK_WORK") "/out/s_schematic.sfyx") chkLib
  '("alt" "conn" "e1" "ms" "ordtop" "pc" "skip"))))
let((p l n tn)
  p = infile(strcat(getShellEnvVar("SFYCHECK_WORK") "/out/s_schematic.sfyx"))
  n = 0 tn = 0
  while(gets(l p)
    when(substring(l 1 7) == "(\"tnet\"" tn = tn + 1)
    when(substring(l 1 10) == "(\"skipped\"" n = n + 1))
  close(p)
  chkThat(sprintf(nil "the dump carries Virtuoso's instTerm table (%d tnet records)" tn) tn > 20)
  chkThat(sprintf(nil "the dump carries skipped counts (%d records)" n) n > 0))
