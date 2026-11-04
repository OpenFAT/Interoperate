;; pdkmap_names.il: the pdk-map live check, SfyCdfNames (skill/sfy_pdkmap.il). Run through
;; pdkmap_drive.sh, which compares the dump with the mock's and maps the scan fixture onto
;; it on its own machine. This session draws the made-up kit
;; (tests/fixtures/skill/pdkmap_kit.il, copied here as pdkmap_kit.il, with build.il's helpers
;; as pdkmap_build.il) in chkLib and dumps its names to out/pdkmap_names.sfyx.

let((dir path p line n)
  dir = getShellEnvVar("SFYCHECK_DIR")
  path = strcat(getShellEnvVar("SFYCHECK_WORK") "/out/pdkmap_names.sfyx")
  load(strcat(dir "/pdkmap_build.il"))
  load(strcat(dir "/pdkmap_kit.il"))
  chkE("draw kit" list('SfyFixPdkMapKit chkLib))
  chkThat("SfyCdfNames PASS" SfyCdfNames(path list(chkLib)))
  n = 0
  p = infile(path)
  when(p
    while(gets(line p)
      chkLine("record" line)
      when(rexMatchp("^(\"cdfnames\"" line) n = n + 1))
    close(p))
  chkThat("one cdfnames record per kit cell (5)" n == 5))
