;; p_refresh.il: lane P live check (gaps2 P2, P10), run by p_drive.sh after Schemify's
;; `virtuoso-refresh --scratch` wrote <work>/refresh/refresh.il for the resized instances
;; (the kit nfet MN at w 400n nf 2, the resistor R0 at w 720n): loads it, so Virtuoso
;; measures their derived values into <work>/refresh/refresh.*.sfyx, and writes the truth
;; to out/p_truth.txt: the same sizes set with the callbacks on fresh instances
;; (SfySetParams, as the Edit Properties form), one `<inst> <param> <value>` line per CDF
;; parameter.
procedure(chkPTruth(port plib cell name params)
  let((cv i)
    cv = dbOpenCellViewByType(chkLib "truth" "schematic" "schematic" "a")
    i = dbCreateInst(cv dbOpenCellViewByType(plib cell "symbol") name 0:0 "R0")
    chkLine(sprintf(nil "truth %s %L" name params) SfySetParams(i params t))
    foreach(p cdfGetInstCDF(i)~>parameters
      fprintf(port "%s %s %s\n" name p~>name SfyAtomStr(p~>value)))
    dbDeleteObject(i)
    dbSave(cv)))

let((plib port il)
  plib = or(getShellEnvVar("SFYCHECK_PLIB") "cmos22fdsoi")
  il = strcat(getShellEnvVar("SFYCHECK_WORK") "/refresh/refresh.il")
  chkThat("refresh.il loads" errset(load(il) t))
  port = outfile(strcat(getShellEnvVar("SFYCHECK_WORK") "/out/p_truth.txt"))
  chkPTruth(port plib "nfet" "MN" list(list("w" "400n") list("nf" "2")))
  chkPTruth(port plib "opnpcres" "R0" list(list("w" "720n")))
  close(port)
  chkPass("p_refresh ran"))
