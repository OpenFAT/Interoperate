;; l9_cdfcb.il: lane L9 live check, design-library CDF callbacks and instance CDF values
;; on a redraw (sfy_cdfcb.il). Run through l9_drive.sh, which does the Schemify half on its
;; own machine. This session draws the kit (tests/fixtures/skill/cdfcb_kit.il, copied here
;; as l9_kit.il, with build.il's helpers as l9_build.il) in chkLib: amp, whose w and nf
;; callbacks are a plain assignment (wf = w / nf) and whose l callback branches, both in
;; chkLib's libInit.il, and top, whose I0 carries CDF values, properties of types Schemify
;; has no param for and three textDisplays. Then:
;; - runs amp's callbacks as the Edit Properties form does (SfySetParams ?callbacks) for
;;   three (w nf) sets and keeps the wf they derive;
;; - dumps to out/l9_first.sfyx and waits for the driver's out/l9_second.sfyx (top with I1
;;   moved), out/l9_want.sfyx (the first dump with I1 moved) and out/l9_cel.sfyx (amp with
;;   its interface edited, so its CDF is derived: wf a CEL default);
;; - loads the second, dumps to out/l9_third.sfyx and requires it to equal the wanted one
;;   but for the schemify* properties export adds (I0's values, properties and
;;   textDisplays survive the redraw);
;; - loads the CEL export and requires amp's wf default, evaluated with each set's w and nf,
;;   to equal what the callback derived.

;; The lines of the file at path, without the header and the schemify* properties.
procedure(chkL9Lines(path)
  let((p line out)
    p = infile(path)
    when(p
      while(gets(line p)
        unless(or(rexMatchp("^(\"sfyx\"" line) rexMatchp("^(\"prop\" [0-9]* \"schemify" line))
          out = cons(line out)))
      close(p))
    reverse(out)))

;; Waits up to secs seconds for every file of paths; t when they all exist.
procedure(chkL9Wait(paths secs)
  let((n)
    n = 0
    while(and(n < secs !forall(f paths isFile(f)))
      ipcSleep(2)
      n = n + 2)
    forall(f paths isFile(f))))

;; A CEL default with iPar("w") and iPar("nf") replaced by numbers, evaluated (bare: a
;; bracketed number reads as a call).
procedure(chkL9Cel(cel w nf)
  let((e)
    rexCompile("iPar(\"w\")")
    e = rexReplace(cel sprintf(nil "%.17g" w) 0)
    rexCompile("iPar(\"nf\")")
    e = rexReplace(e sprintf(nil "%d" nf) 0)
    chkLine(sprintf(nil "CEL with w %g nf %d" w nf) e)
    evalstring(e)))

let((dir out first second want third cel sets derived cv inst got exp bad def v ok)
  dir = getShellEnvVar("SFYCHECK_DIR")
  out = strcat(getShellEnvVar("SFYCHECK_WORK") "/out/")
  first = strcat(out "l9_first.sfyx")
  second = strcat(out "l9_second.sfyx")
  want = strcat(out "l9_want.sfyx")
  third = strcat(out "l9_third.sfyx")
  cel = strcat(out "l9_cel.sfyx")
  load(strcat(dir "/l9_build.il"))
  load(strcat(dir "/l9_kit.il"))
  chkE("draw kit" list('SfyFixCbKit chkLib))
  chkE("load libInit.il" list('load strcat(ddGetObj(chkLib)~>readPath "/libInit.il")))
  ;; The callbacks' own values, on a scratch instance (never saved).
  sets = list(list(0.000001 1) list(0.000003 2) list(0.0000025 7))
  cv = dbOpenCellViewByType(chkLib "cbrun" "schematic" "schematic" "w")
  inst = dbCreateInst(cv dbOpenCellViewByType(chkLib "amp" "symbol" nil "r") "X0" 0:0 "R0")
  derived = mapcar(lambda((s)
      chkLine(sprintf(nil "SfySetParams w %g nf %d ?callbacks" car(s) cadr(s))
        SfySetParams(inst list(list("w" car(s)) list("nf" cadr(s))) t))
      cdfFindParamByName(cdfGetInstCDF(inst) "wf")->value)
    sets)
  chkLine("callback wf" derived)
  chkThat("the plain callback derives w / nf"
    forall(x mapcar(lambda((s d) list(s d)) sets derived)
      abs(cadr(x) - car(car(x)) / cadr(car(x))) < 1e-18))
  dbPurge(cv)
  when(chkThat("first dump" SfyExport(first chkLib list("amp" "top")))
    chkLine("first dump" first)
    when(chkThat("the driver's exports arrived" chkL9Wait(list(second want cel) 900))
      chkThat("load the redrawn top" SfyImport(second ?overwrite t))
      chkThat("third dump" SfyExport(third chkLib list("amp" "top")))
      got = chkL9Lines(third)
      exp = chkL9Lines(want)
      bad = setof(l got !member(l exp))
      foreach(l bad chkLine("unexpected" l))
      foreach(l setof(l exp !member(l got)) chkLine("missing" l))
      chkThat("I0's CDF values, properties and textDisplays survive the redraw"
        and(exp equal(got exp)))
      chkThat("load amp with its derived CDF" SfyImport(cel ?overwrite t))
      def = cdfFindParamByName(cdfGetBaseCellCDF(ddGetObj(chkLib "amp")) "wf")->defValue
      chkLine("wf default" def)
      ok = stringp(def)
      foreach(x mapcar(lambda((s d) list(s d)) sets derived)
        v = car(errset(chkL9Cel(def car(car(x)) cadr(car(x))) t))
        chkLine("CEL value" v)
        unless(and(floatp(v) abs(v - cadr(x)) <= 1e-12 * abs(cadr(x))) ok = nil))
      chkThat("the CEL default evaluates to the callback's value for 3 input sets" ok))))
