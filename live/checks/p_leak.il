;; p_leak.il: lane P live check (gaps2 P11, P2) on the PDK library $SFYCHECK_PLIB (default
;; cmos22fdsoi), with HEAD's schemify.il:
;; - P11: SfyCharacterize of nfet at the point nf=8 w=2u, then the cell CDF's nf/w and a
;;   fresh instance's nf/w must be the defaults (the cell-CDF leak fixed in a5f470c);
;; - P2: an nfet instance's as/ad/ps/pd at nf=1 and, after SfySetParams with the callbacks
;;   (the Edit Properties form's route), at nf=2, at w=400n and at w=400n nf=2; then
;;   discovery (no ?inputs) into out/p_disc.sfyx, its char record and nf's probe printed.
procedure(chkPVals(inst names)
  mapcar(lambda((n) list(n cdfFindParamByName(cdfGetInstCDF(inst) n)~>value)) names))

let((plib c cv i j geo r)
  plib = or(getShellEnvVar("SFYCHECK_PLIB") "cmos22fdsoi")
  c = cdfGetCellCDF(ddGetObj(plib "nfet"))
  chkLine("P11 before" list(cdfFindParamByName(c "nf")~>value cdfFindParamByName(c "w")~>value))
  chkLine("P11 characterize" SfyCharacterize(strcat(getShellEnvVar("SFYCHECK_WORK") "/out/p_leak.sfyx")
    plib list("nfet") chkLib ?inputs list("nf" "w")
    ?points list(list(list("nf" "8") list("w" "2u")))))
  c = cdfGetCellCDF(ddGetObj(plib "nfet"))
  chkLine("P11 after" list(cdfFindParamByName(c "nf")~>value cdfFindParamByName(c "w")~>value))
  cv = dbOpenCellViewByType(chkLib "leak" "schematic" "schematic" "w")
  i = dbCreateInst(cv dbOpenCellViewByType(plib "nfet" "symbol") "M1" 0:0 "R0")
  chkLine("P11 new instance" chkPVals(i list("nf" "w")))
  chkThat("P11 no leak: the cell CDF and a new instance keep nf 1"
    cdfFindParamByName(c "nf")~>value == "1"
    && cdfFindParamByName(cdfGetInstCDF(i) "nf")~>value == "1")
  geo = list("nf" "w" "as" "ad" "ps" "pd")
  chkLine("P2 nf=1" chkPVals(i geo))
  r = SfySetParams(i list(list("nf" "2")) t)
  chkLine("P2 set nf=2" r)
  chkLine("P2 nf=2" chkPVals(i geo))
  j = dbCreateInst(cv dbOpenCellViewByType(plib "nfet" "symbol") "M2" 2:0 "R0")
  chkLine("P2 set w=400n" SfySetParams(j list(list("w" "400n")) t))
  chkLine("P2 w=400n" chkPVals(j geo))
  ;; nf=2 on 80n was put back to 1 by the kit's callback (wf below its minimum): at 400n.
  chkLine("P2 set nf=2 at w=400n" SfySetParams(j list(list("nf" "2")) t))
  chkLine("P2 nf=2 w=400n" chkPVals(j append(geo list("wf"))))
  dbSave(cv)
  ;; P2: discovery finds nf now (its nudges at the default width are all put back).
  chkLine("P2 discover" SfyCharacterize(strcat(getShellEnvVar("SFYCHECK_WORK") "/out/p_disc.sfyx")
    plib list("nfet") chkLib))
  let((p l)
    p = infile(strcat(getShellEnvVar("SFYCHECK_WORK") "/out/p_disc.sfyx"))
    while(gets(l p)
      when(or(rexMatchp("^(\"char\"" l) rexMatchp("^(\"probe\" \"nf\"" l))
        chkLine("P2 record" l)))
    close(p))
  chkPass("p_leak ran"))
