;; p_layout.il: lane P live check (gaps2 P8, P9) on the PDK library $SFYCHECK_PLIB
;; (default cmos22fdsoi) and its technology $SFYCHECK_PTECH (default cmos22fdsoi_tech):
;; chkLib bound to the kit's technology; top/layout holds a kit nfet PCell (w 400n nf 2)
;; and a rect on the kit's M1. SfyLayoutExport writes out/p_lay: its layermap must be the
;; technology's own file, strmout's summary free of errors, the PCell's sub-master a GDS
;; structure. The GDS then goes back in through strmin with that map into library
;; <chkLib>_in and out again: the same polygons, rects and paths on every layer.
let((plib tl tech cv dir lm gds2 back r)
  plib = or(getShellEnvVar("SFYCHECK_PLIB") "cmos22fdsoi")
  tl = or(getShellEnvVar("SFYCHECK_PTECH") strcat(plib "_tech"))
  dir = strcat(getShellEnvVar("SFYCHECK_WORK") "/out/p_lay")
  chkLine("bind" techBindTechFile(ddGetObj(chkLib) tl))
  tech = techGetTechFile(ddGetObj(chkLib))
  chkLine("tech" tech~>libName)
  cv = dbOpenCellViewByType(chkLib "top" "layout" "maskLayout" "w")
  r = dbCreateRect(cv list("M1" "drawing") list(0:0 1:0.1))
  chkLine("rect" and(r t))
  chkE("pcell" lambda(()
    dbCreateParamInst(cv dbOpenCellViewByType(plib "nfet" "layout") "M0" 2:0 "R0" 1
      list(list("w" "string" "400n") list("nf" "string" "2")))))
  chkLine("instances" mapcar(lambda((i) list(i~>name i~>master~>cellName
    and(i~>master~>superMaster t) length(i~>master~>shapes))) cv~>instances))
  dbSave(cv) dbClose(cv)
  chkE("export" lambda(() SfyLayoutExport(chkLib "top" dir)))
  lm = SfyLayoutTechMap(tech)
  chkLine("tech layermap" lm)
  chkThat("the bundle's layermap is the technology's"
    and(lm sh(sprintf(nil "cmp -s %s %s/layermap" lm dir))))
  chkLine("strmout.sum errors" sh(sprintf(nil "grep -i -q 'error' %s/strmout.sum" dir)))
  chkE("strmout.sum" lambda(() let((p l out) p = infile(strcat(dir "/strmout.sum"))
    while(gets(l p) when(rexMatchp("[Ss]tructure\\|[Ee]rror\\|[Ww]arning\\|[Cc]ells" l) out = cons(l out)))
    close(p) reverse(out))))
  chkThat("strmout.sum has no error" !sh(sprintf(nil "grep -i -q ' error' %s/strmout.sum" dir)))
  ;; In and out again.
  back = strcat(chkLib "_in")
  gds2 = strcat(dir "/back.gds")
  chkLine("strmin"
    sh(sprintf(nil "cd %s && strmin -library %s -strmFile %s/layout.gds -layerMap %s -attachTechFileOfLib %s -pinAttNum 100 -logFile %s/strmin.log -summaryFile %s/strmin.sum >/dev/null 2>&1"
      getShellEnvVar("SFYCHECK_WORK") back dir lm tl dir dir)))
  chkLine("strmout again"
    sh(sprintf(nil "cd %s && strmout -library %s -topCell top -view layout -strmFile %s -layerMap %s -ignoreCurrentTimestamp -convertPin geometryAndText -pinAttNum 100 -logFile %s/strmout2.log -summaryFile %s/strmout2.sum >/dev/null 2>&1"
      getShellEnvVar("SFYCHECK_WORK") back gds2 lm dir dir)))
  ;; The GDS library and the sub-master's structure name differ (the library's, the PCell
  ;; sub-master's nfet_CDNS_0): the shapes per layer and the object counts are compared.
  ;; Texts aside: the strmin'd sub-master is a plain cell whose pins strmout writes as
  ;; geometry and text, where the PCell's went out as geometry only (live: 13 pin texts).
  chkThat("strmin then strmout: the same polygons, rects and paths on every layer"
    sh(sprintf(nil "cd %s && for f in strmout.sum strmout2.sum; do sed -n '/Statistics of Layers/,/Summary/p' $f | awk 'NF > 8 {print $1, $2, $3, $4, $5, $6, $7}' > $f.lp; done && diff strmout.sum.lp strmout2.sum.lp > layers.diff" dir)))
  chkPass("p_layout ran"))
