;; p_names.il: lane P live check (gaps2 P6, P16): SfyCdfNames over the kit's libraries
;; ($SFYCHECK_PLIB, default cmos22fdsoi, and its _rf, _mmw and _esd libraries when
;; cds.lib has them) into out/p_names.sfyx, for pdk-map against a pdk-scan of the kit's
;; Spectre models. Names only: it stays on the host.
let((plib libs)
  plib = or(getShellEnvVar("SFYCHECK_PLIB") "cmos22fdsoi")
  libs = setof(l list(plib strcat(plib "_rf") strcat(plib "_mmw") strcat(plib "_esd")) ddGetObj(l))
  chkLine("libraries" libs)
  chkThat("SfyCdfNames"
    SfyCdfNames(strcat(getShellEnvVar("SFYCHECK_WORK") "/out/p_names.sfyx") libs)))
