;; p_kit.il: lane P live check: the whole PDK library $SFYCHECK_PLIB (default cmos22fdsoi)
;; dumped with HEAD's schemify.il to out/p_kit.sfyx (SfyExportLib), for the import's
;; counts (gaps2 P1, S9, P13): every stopping-view terminal a pinless term, the report rows.
;; The dump holds the kit's data: it stays on the host or in a scratch directory.
let((plib ok)
  plib = or(getShellEnvVar("SFYCHECK_PLIB") "cmos22fdsoi")
  ok = SfyExportLib(strcat(getShellEnvVar("SFYCHECK_WORK") "/out/p_kit.sfyx") plib)
  chkThat(sprintf(nil "SfyExportLib %s" plib) ok))
