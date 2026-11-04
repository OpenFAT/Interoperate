;; l3_config.il: lane L3 live check, config views (sfy_cfg.il, gap C). Run through
;; l3_drive.sh, which does the Schemify half on its own machine. This session draws a
;; three-level hierarchy in chkLib (top: I0, I1 of mid; mid: I2 of leaf; leaf: R0 of
;; analogLib res), writes top/config through hdb with every rule kind (global lib, view and
;; stop lists; cell view list, lib list, binding and stop; instance binding, lib list, view
;; list and stop; occurrence bindings two and three levels deep), dumps it to
;; out/l3_first.sfyx, and waits for the driver's out/l3_second.sfyx (Interop import and
;; export, unchanged) and out/l3_edited.sfyx (the same with the Schemify config edited).
;; It loads the second and requires expand.cfg to come back byte for byte, and hdb to save
;; what it reads from it unchanged; then loads the edited one, has hdb re-save it, and dumps
;; that to out/l3_fourth.sfyx, whose rules the driver compares with the edit.

;; The text of the file at path.
procedure(chkL3Text(path)
  let((p line s)
    s = ""
    p = infile(path)
    when(p
      while(gets(line p) s = strcat(s line))
      close(p))
    s))

procedure(chkL3Cfg() chkL3Text(ddGetObj(chkLib "top" "config" "expand.cfg")~>readPath))

;; Prints expand.cfg line by line.
procedure(chkL3Show(label)
  foreach(l parseString(chkL3Cfg() "\n") chkLine(label l)))

;; text without its //Revision line (hdb counts saves there).
procedure(chkL3NoRev(text)
  buildString(setof(l parseString(text "\n") !rexMatchp("^//Revision" l)) "\n"))

;; hdb opens the config for append and saves it: t when both answer t.
procedure(chkL3Resave()
  let((h ok)
    h = hdbOpen(chkLib "top" "config" "a")
    ok = and(h hdbSave(h))
    when(h hdbClose(h))
    and(ok t)))

;; Waits up to secs seconds for every file of paths; t when they all exist.
procedure(chkL3Wait(paths secs)
  let((n)
    n = 0
    while(and(n < secs !forall(f paths isFile(f)))
      ipcSleep(2)
      n = n + 2)
    forall(f paths isFile(f))))

procedure(chkL3Sym(cell)
  let((cv)
    cv = dbOpenCellViewByType(chkLib cell "symbol" "schematicSymbol" "w")
    dbCreateRect(cv list("device" "drawing") list(list(-0.25 -0.25) list(0.25 0.25)))
    dbSave(cv)
    dbClose(cv)))

;; A schematic of insts, each (name lib cell x).
procedure(chkL3Sch(cell insts)
  let((cv m)
    cv = dbOpenCellViewByType(chkLib cell "schematic" "schematic" "w")
    foreach(i insts
      m = dbOpenCellViewByType(cadr(i) caddr(i) "symbol" nil "r")
      dbCreateInst(cv m car(i) list(cadddr(i) 0.0) "R0"))
    dbSave(cv)
    dbClose(cv)))

procedure(chkL3Rule(h spec rule)
  chkE(sprintf(nil "rule %L %L" spec rule) list('hdbSetObjBindRule h list('quote spec) list('quote rule))))

let((out first second edited fourth h orig)
  out = strcat(getShellEnvVar("SFYCHECK_WORK") "/out/")
  first = strcat(out "l3_first.sfyx")
  second = strcat(out "l3_second.sfyx")
  edited = strcat(out "l3_edited.sfyx")
  fourth = strcat(out "l3_fourth.sfyx")
  chkL3Sch("leaf" list(list("R0" "analogLib" "res" 0.0)))
  chkL3Sym("leaf")
  chkL3Sch("mid" list(list("I2" chkLib "leaf" 0.0)))
  chkL3Sym("mid")
  chkL3Sch("top" list(list("I0" chkLib "mid" 0.0) list("I1" chkLib "mid" 1.0)))
  h = chkE("hdbOpen w" list('hdbOpen chkLib "top" "config" "w"))
  chkE("top" list('hdbSetTopCellViewName h chkLib "top" "schematic"))
  chkE("libList" list('hdbSetDefaultLibListString h strcat(chkLib " analogLib basic")))
  chkE("viewList" list('hdbSetDefaultViewListString h "spectre cmos_sch schematic veriloga"))
  chkE("stopList" list('hdbSetDefaultStopListString h "spectre"))
  chkL3Rule(h list(list(chkLib "mid" nil nil)) list('hdbcViewListRule list("schematic" "symbol")))
  chkL3Rule(h list(list(chkLib "leaf" nil nil)) list('hdbcLibListRule list(chkLib)))
  chkL3Rule(h list(list("analogLib" "res" nil nil)) list('hdbcBindingRule list(nil nil "spectre")))
  chkL3Rule(h list(list(chkLib "leaf" nil nil)) list('hdbcStopRule t))
  chkL3Rule(h list(list(chkLib "top" "schematic" "I1")) list('hdbcBindingRule list(nil nil "schematic")))
  chkL3Rule(h list(list(chkLib "top" "schematic" "I1")) list('hdbcLibListRule list(chkLib)))
  chkL3Rule(h list(list(chkLib "mid" "schematic" "I2")) list('hdbcViewListRule list("schematic")))
  chkL3Rule(h list(list(chkLib "top" "schematic" "I0")) list('hdbcStopRule t))
  chkL3Rule(h list(list(chkLib "top" "schematic" "I0") list(chkLib "mid" "schematic" "I2"))
    list('hdbcBindingRule list(chkLib "leaf" "schematic")))
  chkL3Rule(h list(list(chkLib "top" "schematic" "I1") list(chkLib "mid" "schematic" "I2")
                   list(chkLib "leaf" "schematic" "R0"))
    list('hdbcBindingRule list("analogLib" "res" "spectre")))
  chkThat("hdb saves the config" and(chkE("hdbSave" list('hdbSave h)) chkE("hdbClose" list('hdbClose h))))
  orig = chkL3Cfg()
  chkL3Show("expand.cfg")
  when(chkThat("first dump" SfyExport(first chkLib list("top")))
    when(chkThat("the driver's round trip arrived" chkL3Wait(list(second edited) 900))
      chkThat("load the round trip" SfyImport(second ?overwrite t ?check nil))
      chkThat("expand.cfg comes back byte for byte" equal(chkL3Cfg() orig))
      chkThat("hdb re-saves the loaded file" chkL3Resave())
      chkL3Show("re-saved")
      chkThat("hdb read the loaded file to the same rules"
        equal(chkL3NoRev(chkL3Cfg()) chkL3NoRev(orig)))
      chkThat("load the edited config" SfyImport(edited ?overwrite t ?check nil))
      chkL3Show("edited as loaded")
      chkThat("hdb re-saves the edited file" chkL3Resave())
      chkL3Show("edited re-saved")
      chkThat("fourth dump" SfyExport(fourth chkLib list("top"))))))
