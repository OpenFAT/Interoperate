;; p_map.il: lane P live check (gaps2 P12), run after a Schemify export of a core:nmos4
;; bench mapped onto the kit's nfet with `pin b lSup` (the body an inherited terminal)
;; written to out/p_map.sfyx (lib sfyprobe_p): loads it into chkLib and prints M0's
;; netSet override, which carries the body net (the conn on lSup, SfyConnNetSet), and the
;; schCheck counts. With SFYCHECK_KEEP the library stays for p_si.sh (P_CELLS=pmap).
let((ok cv m)
  ok = SfyImport(strcat(getShellEnvVar("SFYCHECK_WORK") "/out/p_map.sfyx") ?lib chkLib)
  chkThat("SfyImport of p_map.sfyx" ok)
  cv = dbOpenCellViewByType(chkLib "pmap" "schematic" nil "a")
  m = car(setof(i cv~>instances i~>name == "M0"))
  chkLine("M0 master" list(m~>libName m~>cellName))
  chkLine("M0 lSup" list(dbGetPropByName(m "lSup")~>valueType dbGetPropByName(m "lSup")~>value))
  chkLine("pmap schCheck" schCheck(cv))
  dbSave(cv)
  chkThat("M0's body is the netSet lSup = VBB" dbGetPropByName(m "lSup")~>value == "VBB"))
