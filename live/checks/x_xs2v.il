;; x_xs2v.il: lane X live check, xschem → Schemify → Virtuoso with interop.map re-mastering
;; (inv_xschem rows X1-X3). Run through x_drive.sh, which does the Schemify half on its own
;; machine (tests/xschem_virtuoso.rs live_xs2v). This session draws a made-up sky130-shaped
;; kit cell in chkLib (nfet_01v8: CDF model sky130_fd_pr__nfet_01v8, string params w l nf m
;; in metres, spectre termOrder D G S B; build.il's helpers as x_build.il, pdkmap_kit.il's
;; SfyFixMapDev as x_kit.il), then:
;; - dumps its names with SfyCdfNames to out/x_xs2v_cdf.sfyx (pdk-map's input);
;; - waits for the driver's out/x_xs2v_in.sfyx (the xschem bench tb, re-mastered onto the kit
;;   cell by the map, design cells into chkLib) and loads it with SfyImport;
;; - runs schCheck on tb and requires no error;
;; - reads M1's w and l (2e-6 and 5e-7: xschem's W=2 L=0.5 in micrometres, scaled);
;; - dumps chkLib with SfyExport to out/x_xs2v_back.sfyx for the way back.
;; The driver netlists tb with si (spectre) after the session, the library kept.

procedure(chkXWait(path secs)
  let((n)
    n = 0
    while(and(n < secs !isFile(path))
      ipcSleep(2)
      n = n + 2)
    isFile(path)))

let((dir out cdfPath inPath backPath cdf cv r m1)
  dir = getShellEnvVar("SFYCHECK_DIR")
  out = strcat(getShellEnvVar("SFYCHECK_WORK") "/out/")
  cdfPath = strcat(out "x_xs2v_cdf.sfyx")
  inPath = strcat(out "x_xs2v_in.sfyx")
  backPath = strcat(out "x_xs2v_back.sfyx")
  load(strcat(dir "/x_build.il"))
  load(strcat(dir "/x_kit.il"))
  chkE("draw kit cell" lambda(() SfyFixMapDev(chkLib "nfet_01v8" "sky130_fd_pr__nfet_01v8"
    '("D" "G" "S" "B")
    '(("D" "inputOutput" 0 40) ("G" "input" -40 0) ("S" "inputOutput" 0 -40)
      ("B" "inputOutput" 40 0))
    '("w" "l" "nf" "m"))))
  ;; Spectre netlists the instance's sizes: instParameters beside the model.
  cdf = cdfGetBaseCellCDF(ddGetObj(chkLib "nfet_01v8"))
  setSGq(cdf list(nil 'spectre list(nil 'termOrder '("D" "G" "S" "B")
    'otherParameters '(model) 'instParameters '(w l nf m))) simInfo)
  cdfSaveCDF(cdf)
  chkThat("SfyCdfNames PASS" SfyCdfNames(cdfPath list(chkLib)))
  if(chkXWait(inPath 900)
    then
      chkThat("SfyImport of the driver's export PASS" SfyImport(inPath))
      cv = dbOpenCellViewByType(chkLib "tb" "schematic" nil "a")
      chkThat("tb was loaded" cv)
      when(cv
        r = chkE("schCheck tb" lambda(() schCheck(cv)))
        chkThat("schCheck: no errors" and(listp(r) car(r) == 0))
        dbSave(cv)
        m1 = car(setof(i cv~>instances i~>name == "M1"))
        chkThat("M1 is the kit cell" and(m1 m1~>libName == chkLib m1~>cellName == "nfet_01v8"))
        when(m1
          chkLine("M1 w" dbFindProp(m1 "w")~>value)
          chkLine("M1 l" dbFindProp(m1 "l")~>value)
          chkThat("M1 w is 2e-6 (W=2 um scaled)"
            abs(evalstring(dbFindProp(m1 "w")~>value) - 2e-6) < 1e-12)
          chkThat("M1 l is 5e-7 (L=0.5 um scaled)"
            abs(evalstring(dbFindProp(m1 "l")~>value) - 5e-7) < 1e-12))
        dbClose(cv))
      chkThat("SfyExport of chkLib PASS" SfyExport(backPath chkLib nil))
    else chkFail("no out/x_xs2v_in.sfyx from the driver within 900 s")))
