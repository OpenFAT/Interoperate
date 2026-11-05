;; p_char.il: lane P live check (gaps2 P10), on the PDK library $SFYCHECK_PLIB (default
;; cmos22fdsoi): discovery (SfyCharacterize, no ?inputs) of the BJT vnpn, the diode
;; diodenx, the MOS cap ncap and the MOM cap apmom into out/p_char2.sfyx, each char record
;; printed; then nfet's dimensionMode: w, wf, as, ad of an instance set with the callbacks
;; to wf 200n nf 2 in each mode, and nfet's discovery again into out/p_char3.sfyx (is
;; dimensionMode an input).
procedure(chkPChars(path)
  let((p l)
    p = infile(path)
    while(gets(l p)
      when(or(rexMatchp("^(\"char\"" l) rexMatchp("^(\"probe\" \"dimensionMode\"" l))
        chkLine("char" l)))
    close(p)))

let((plib w cv i)
  plib = or(getShellEnvVar("SFYCHECK_PLIB") "cmos22fdsoi")
  w = getShellEnvVar("SFYCHECK_WORK")
  chkLine("characterize vnpn diodenx ncap apmom"
    SfyCharacterize(strcat(w "/out/p_char2.sfyx") plib list("vnpn" "diodenx" "ncap" "apmom") chkLib))
  chkPChars(strcat(w "/out/p_char2.sfyx"))
  cv = dbOpenCellViewByType(chkLib "fw" "schematic" "schematic" "w")
  foreach(mode list("TotalWidth" "FingerWidth")
    i = dbCreateInst(cv dbOpenCellViewByType(plib "nfet" "symbol") strcat("M" mode) 0:0 "R0")
    chkLine(sprintf(nil "set %s" mode)
      SfySetParams(i list(list("dimensionMode" mode) list("wf" "200n") list("nf" "2")) t))
    chkLine(sprintf(nil "%s values" mode)
      mapcar(lambda((n) list(n cdfFindParamByName(cdfGetInstCDF(i) n)~>value))
        list("dimensionMode" "w" "wf" "nf" "as" "ad" "ps" "pd"))))
  dbSave(cv)
  chkLine("characterize nfet"
    SfyCharacterize(strcat(w "/out/p_char3.sfyx") plib list("nfet") chkLib))
  chkPChars(strcat(w "/out/p_char3.sfyx"))
  chkPass("p_char ran"))
