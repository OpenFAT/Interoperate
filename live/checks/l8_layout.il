;; l8_layout.il: lane L8 live check, layout through KLayout (sfy_layout.il, gap H). Driven by
;; l8_drive.sh, which does the Schemify half with this checkout's CLI and KLayout. wrap.il
;; has loaded common.il and schemify.il and created chkLib (sfyprobe_l8, cdsDefTechLib).
;;
;; Builds sub/layout (a rect), a SKILL PCell pc, top/schematic (empty) and top/layout (rect,
;; polygon, path, label, pin, an instance of sub, a PCell instance, a 2x3 mosaic of sub);
;; exports top/layout twice (the GDS bytes must be equal) into $out/l8_export, writes
;; l8_export.ready and waits for l8_go, by which the driver has put two import bundles in
;; $out: l8_imp_a (the GDS unchanged: restore) and l8_imp_b (one rect moved by +0.5 in x
;; in KLayout: strmin). Then:
;; (a) top/layout edited in Virtuoso and saved: the restore is refused while the view is open
;;     for editing and, closed, without ?overwrite; with it, the view's files equal the
;;     exported ones byte for byte and a cellview held open read-only shows the original
;;     after the refresh; restoring again answers unchanged.
;; (b) the strmin import: the rect is at its new place, the pin is back, the PCell instance
;;     is an instance of a plain cell, sub's layout.oa and top's schematic are untouched.

chkOutDir = strcat(getShellEnvVar("SFYCHECK_WORK") "/out")
chkExp = strcat(chkOutDir "/l8_export")
chkExp2 = strcat(chkOutDir "/l8_export2")
chkVdir = strcat(getShellEnvVar("SFYCHECK_WORK") "/" chkLib "/top/layout")

procedure(chkShapes(cv) length(cv~>shapes))
procedure(chkNear(a b) abs(a - b) < 1e-6)
procedure(chkBoxIs(bb x0 y0 x1 y1)
  chkNear(xCoord(car(bb)) x0) && chkNear(yCoord(car(bb)) y0) && chkNear(xCoord(cadr(bb)) x1) && chkNear(yCoord(cadr(bb)) y1))

;; ---- the library
chkCv = dbOpenCellViewByType(chkLib "sub" "layout" "maskLayout" "w")
dbCreateRect(chkCv list("y0" "drawing") list(0:0 1:1))
dbSave(chkCv) dbClose(chkCv)
chkE("pcell define"
  `(pcDefinePCell (list (ddGetObj ,chkLib) "pc" "layout") ((w 1.0) (l 0.5))
     (let ((cv pcCellView))
       (dbCreateRect cv (list "y1" "drawing") (list (list 0 0) (list w l))))))
chkCv = dbOpenCellViewByType(chkLib "top" "schematic" "schematic" "w")
dbSave(chkCv) dbClose(chkCv)
chkCv = dbOpenCellViewByType(chkLib "top" "layout" "maskLayout" "w")
dbCreateRect(chkCv list("y0" "drawing") list(0:0 2:1))
dbCreatePolygon(chkCv list("y1" "drawing") list(3:0 4:0 4:2 3.5:2.5 3:2))
dbCreatePath(chkCv list("y0" "drawing") list(0:3 4:3 4:5) 0.2)
dbCreateLabel(chkCv list("y0" "drawing") 1:0.5 "hello" "centerCenter" "R0" "roman" 0.5)
let((net term fig)
  net = dbMakeNet(chkCv "P")
  term = dbCreateTerm(net "P" "input")
  fig = dbCreateRect(chkCv list("y1" "drawing") list(5:0 5.5:0.5))
  dbCreatePin(net fig "P"))
dbCreateInst(chkCv dbOpenCellViewByType(chkLib "sub" "layout") "IS0" 10:0 "R90")
chkE("pcell inst" lambda(() dbCreateParamInst(chkCv dbOpenCellViewByType(chkLib "pc" "layout") "PC0" 12:0 "R0" 1 list(list("w" "float" 2.0)))))
chkE("mosaic" lambda(() dbCreateSimpleMosaic(chkCv dbOpenCellViewByType(chkLib "sub" "layout") "M0" 20:0 "R0" 2 3 1.5 1.5)))
chkN0 = chkShapes(chkCv)
chkLine("top shapes" chkN0)
dbSave(chkCv) dbClose(chkCv)

;; ---- export
chkE("export" lambda(() SfyLayoutExport(chkLib "top" chkExp)))
chkE("export again" lambda(() SfyLayoutExport(chkLib "top" chkExp2)))
chkThat("the bundle has every file"
  forall(f '("layout.gds" "layermap" "display.drf" "layout.sfyx" "oa/layout.oa" "oa/master.tag") isFile(strcat(chkExp "/" f))))
chkThat("strmout -ignoreCurrentTimestamp twice: the same GDS bytes"
  sh(sprintf(nil "cmp -s %s/layout.gds %s/layout.gds" chkExp chkExp2)))
chkThat("the bundle holds no lock file" !sh(sprintf(nil "ls %s/oa | grep -q cdslck" chkExp)))
chkLine("layermap" sh(sprintf(nil "grep -q '^ *y0 *drawing *207 *0' %s/layermap" chkExp)))
chkLine("pcell record" sh(sprintf(nil "grep -q '\"pcell\" \"PC0\" \"%s\" \"pc\"' %s/layout.sfyx" chkLib chkExp)))
close(outfile(strcat(chkOutDir "/l8_export.ready") "w"))

chkN = 0
while(!isFile(strcat(chkOutDir "/l8_go")) && chkN < 900
  chkN = chkN + 1
  sh("sleep 1"))
chkImpA = strcat(chkOutDir "/l8_imp_a")
chkImpB = strcat(chkOutDir "/l8_imp_b")

if(!isFile(strcat(chkImpB "/layout.sfyx")) then
  chkFail("the driver's import bundles did not arrive")
else
  ;; ---- (a) restore
  chkCv = dbOpenCellViewByType(chkLib "top" "layout" "maskLayout" "a")
  dbCreateRect(chkCv list("y0" "drawing") list(30:30 31:31))
  dbSave(chkCv)
  chkThat("restore refused while the view is open for editing"
    !car(errset(SfyLayoutImport(chkImpA ?overwrite t))))
  chkLine("  refusal" getqq(errset errset))
  dbClose(chkCv)
  chkRo = dbOpenCellViewByType(chkLib "top" "layout" nil "r")
  chkLine("shapes after the Virtuoso edit" chkShapes(chkRo))
  chkThat("restore refused without ?overwrite (the view changed in Virtuoso)"
    !car(errset(SfyLayoutImport(chkImpA))))
  chkLine("  refusal" getqq(errset errset))
  chkThat("restore with ?overwrite" chkE("import a" lambda(() SfyLayoutImport(chkImpA ?overwrite t))) == 'restore)
  chkThat("(a) the view's files equal the exported ones byte for byte"
    sh(sprintf(nil "cd %s/oa && for f in *; do cmp -s \"$f\" %s/\"$f\" || exit 1; done && test $(ls %s | wc -l) = $(ls | wc -l)" chkExp chkVdir chkVdir)))
  chkThat("the cellview held open read-only shows the original after dbRefreshCellView" chkShapes(chkRo) == chkN0)
  dbClose(chkRo)
  chkThat("a fresh open shows the original" let((cv n) cv = dbOpenCellViewByType(chkLib "top" "layout" nil "r") n = chkShapes(cv) dbClose(cv) n == chkN0))
  chkThat("restoring onto the exported view answers unchanged" chkE("import a again" lambda(() SfyLayoutImport(chkImpA))) == 'unchanged)

  ;; ---- (b) strmin
  sh(sprintf(nil "cp -p %s/%s/sub/layout/layout.oa %s/l8_sub_before.oa" getShellEnvVar("SFYCHECK_WORK") chkLib chkOutDir))
  chkThat("strmin import" chkE("import b" lambda(() SfyLayoutImport(chkImpB))) == 'strmin)
  chkLine("top/layout after strmin" ddGetObj(chkLib "top" "layout"))
  chkCv = chkE("open top after strmin" lambda(() dbOpenCellViewByType(chkLib "top" "layout" nil "r")))
  if(!chkCv then
    chkFail("(b) top/layout does not open after strmin")
  else
    chkLine("shapes" mapcar(lambda((s) list(s~>objType s~>layerName s~>bBox)) chkCv~>shapes))
    chkThat("(b) the moved rect is at x 0.5..2.5"
      exists(s chkCv~>shapes s~>layerName == "y0" && member(s~>objType '("rect" "polygon")) && chkBoxIs(s~>bBox 0.5 0.0 2.5 1.0)))
    chkThat("(b) no rect is left at x 0..2"
      !exists(s chkCv~>shapes s~>layerName == "y0" && member(s~>objType '("rect" "polygon")) && chkBoxIs(s~>bBox 0.0 0.0 2.0 1.0)))
    chkThat("(b) pin P is back (-pinAttNum 100)" member("P" chkCv~>terminals~>name))
    chkLine("instances" mapcar(lambda((i) list(i~>name i~>cellName i~>objType)) chkCv~>instances))
    chkThat("(b) the PCell instance is an instance of the plain cell pc_CDNS_0"
      exists(i chkCv~>instances i~>cellName == "pc_CDNS_0"))
    chkThat("(b) the mosaic came back" exists(i chkCv~>instances i~>objType == "mosaic"))
    dbClose(chkCv))
  chkThat("(b) sub/layout untouched (noOverwrite)"
    sh(sprintf(nil "cmp -s %s/l8_sub_before.oa %s/%s/sub/layout/layout.oa" chkOutDir getShellEnvVar("SFYCHECK_WORK") chkLib)))
  chkThat("(b) top/schematic untouched" ddGetObj(chkLib "top" "schematic"))
  sh(sprintf(nil "cp %s/strmin.log %s/l8_strmin.log" chkImpB chkOutDir)))
