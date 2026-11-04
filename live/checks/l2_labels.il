;; l2_labels.il: lane L2 live check, cosmetics on a redrawn cellview (gap D; the label,
;; wire and noConn parts of sfy_cells.il). Run through l2_drive.sh, which does the Schemify
;; half on its own machine. This session draws cell kit (tests/fixtures/skill/cosm_kit.il,
;; copied here as l2_kit.il) in chkLib with every cosmetic kind, dumps it to
;; out/l2_first.sfyx, waits for the driver's out/l2_second.sfyx (Interop import, one wire
;; label moved in Schemify, export) and out/l2_want.sfyx (the first dump with that label
;; moved), loads the second over the cell, dumps again to out/l2_third.sfyx, and requires
;; the third dump to equal the wanted one but for the schemify* properties export adds.

;; build.il's drawing helpers, which cosm_kit.il uses (database units, 160 per inch).
procedure(SfyFixU(v) quotient(float(v) 160.0))
procedure(SfyFixP(x y) list(SfyFixU(x) SfyFixU(y)))
procedure(SfyFixBox(x0 y0 x1 y1) list(SfyFixP(x0 y0) SfyFixP(x1 y1)))
procedure(SfyFixOpen(lib cell view vtype)
  let((cv)
    cv = dbOpenCellViewByType(lib cell view vtype "w")
    unless(cv error("l2: cannot open %s/%s/%s" lib cell view))
    cv))
procedure(SfyFixSave(cv)
  unless(dbSave(cv) error("l2: cannot save"))
  dbClose(cv))

;; The lines of the file at path, without the header and the schemify* properties.
procedure(chkL2Lines(path)
  let((p line out)
    p = infile(path)
    when(p
      while(gets(line p)
        unless(or(rexMatchp("^(\"sfyx\"" line) rexMatchp("^(\"prop\" [0-9]* \"schemify" line))
          out = cons(line out)))
      close(p))
    reverse(out)))

;; Waits up to secs seconds for every file of paths; t when they all exist.
procedure(chkL2Wait(paths secs)
  let((n)
    n = 0
    while(and(n < secs !forall(f paths isFile(f)))
      ipcSleep(2)
      n = n + 2)
    forall(f paths isFile(f))))

let((dir out first second want third got exp bad)
  dir = getShellEnvVar("SFYCHECK_DIR")
  out = strcat(getShellEnvVar("SFYCHECK_WORK") "/out/")
  first = strcat(out "l2_first.sfyx")
  second = strcat(out "l2_second.sfyx")
  want = strcat(out "l2_want.sfyx")
  third = strcat(out "l2_third.sfyx")
  load(strcat(dir "/l2_kit.il"))
  chkE("draw kit" list('SfyFixKit chkLib))
  when(chkThat("first dump" SfyExport(first chkLib list("kit")))
    chkLine("first dump" first)
    when(chkThat("the driver's round trip arrived" chkL2Wait(list(second want) 900))
      chkThat("load the redrawn cellview" SfyImport(second ?overwrite t))
      chkThat("third dump" SfyExport(third chkLib list("kit")))
      got = chkL2Lines(third)
      exp = chkL2Lines(want)
      bad = setof(l got !member(l exp))
      foreach(l bad chkLine("unexpected" l))
      foreach(l setof(l exp !member(l got)) chkLine("missing" l))
      chkThat("every other object is byte-identical after the redraw"
        and(exp equal(got exp))))))
