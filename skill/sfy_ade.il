;; sfy_ade.il: maestro (ADE Explorer/Assembler) setups to and from .sfyx (interop spec
;; sections 4.2, 5.2, 9). Entry points called by sfy_cells.il: SfyAdeExport (during
;; SfyExport) and SfyAdeImport (during SfyImport).
;;
;; Every decision about meaning is made in Rust (virtuoso::ade); this file only copies the
;; maestro setup to records and back. Prefix form only (spec 5.1). Reads the dynamic
;; variables of sfy_cells.il: the log state, sfyOverwrite and sfyTargets.
;; GUESS(G6): maeOpenSetup ?mode "r" reads a setup in a background session, no window.
;; Sessions close with ?forceClose t inside errset: a session left open keeps the maestro
;; lock, and the next open fails (ASSEMBLER-8127; virtuoso-bridge-lite, maestro/lifecycle).
;; Every maestro call whose shape is a GUESS runs inside errset: a release that signals
;; instead of answering costs that record (logged), never the rest of the export or import.
;; A view also goes out with its own setup (spec 9.7): the axlExportSetup XML (maesdb) and
;; each test's active.state element (maestate); a load that carries them restores them first
;; (axlImportSetup ... 'overwrite) and applies its records in place on top.

;; ================================================================ export

;; Writes the maestro records (spec 9) of every cell of cells (names in library lib, sorted)
;; that has a maestro view: per test (by name) maestro, global vars, enabled analyses,
;; test model files, corners with their models and vars, outputs, specs, definition files.
(procedure (SfyAdeExport port lib cells)
  (foreach cell cells
    (when (ddGetObj lib cell "maestro")
      (SfyAdeDumpCell port lib cell))))

;; One maestro view, opened read-only and always closed. A read-only session answers setup
;; database 0 (ICADVM20.1, live run 2026-10-15: every axl call then signals ASSEMBLER-2404),
;; so the view is opened again in "a" mode, which answers a real one; nothing is saved and
;; the session is force-closed.
(procedure (SfyAdeDumpCell port lib cell)
  (let ((sess (car (errset (maeOpenSetup lib cell "maestro" ?mode "r")))) (sdb nil))
    (when sess
      (setq sdb (car (errset (axlGetMainSetupDB sess))))
      (when (or (null sdb) (equal sdb 0))
        (SfyAdeClose sess)
        (setq sess (car (errset (maeOpenSetup lib cell "maestro" ?mode "a"))))))
    (if (null sess)
      (SfyRefuse 0 (sprintf nil "cannot open %s/%s/maestro" lib cell))
      (unwindProtect
        (progn
          (setq sdb (car (errset (axlGetMainSetupDB sess))))
          (if (or (null sdb) (equal sdb 0))
            (SfyRefuse 0 (sprintf nil "%s/%s/maestro: no setup database" lib cell))
            (let ((stored (SfyAdeStored lib cell sess sdb)))
              (foreach test (sort (copy (car (errset (maeGetSetup ?session sess)))) nil)
                (unless (errset (SfyAdeDumpTest port lib cell sess sdb test stored) t)
                  (SfyRefuse 0 (sprintf nil "%s/%s/maestro test %s: SKILL error %s" lib cell test
                                 (SfyErrMsg))))
                ;; The view's own setup goes with its first test only.
                (setq stored nil)))))
        (SfyAdeClose sess)))))

;; The axlExportSetup/axlImportSetup tags: every part of the active setup an export holds.
(procedure (SfyAdeTags)
  (list "vars" "tests" "parameters" "corners" "runoptions" "specs"))

;; The scratch file the setup XML passes through (axlExportSetup and axlImportSetup take
;; a path), one per user, overwritten on every use.
(procedure (SfyAdeTmp)
  (sprintf nil "/tmp/sfy_%s_maesdb.sdb" (or (getShellEnvVar "USER") "user")))

;; The lines of file path, or nil when it cannot be read or is not absolute (infile would
;; search the SKILL path for a relative one).
(procedure (SfyAdeLines path)
  (let ((p (and (stringp path) (equal (substring path 1 1) "/") (isFile path) (infile path)))
        (line nil) (out nil))
    (when p
      (while (gets line p) (setq out (cons line out)))
      (close p))
    (reverse out)))

;; The view's own setup, kept for the way back (spec 9.7): the records ("maesdb" xml nil)
;; and ("maestate" test xml) per <Test> element of the view's active.state, by test name.
;; The XML is axlExportSetup's, its root text (the export file's base name) set to
;; "maesdb" so that a dump does not depend on the scratch path. axlExportSetup reports a
;; failure in the CIW rather than signalling: the file is emptied first and read back.
(procedure (SfyAdeStored lib cell sess sdb)
  (let ((path (SfyAdeTmp)) (p nil) (segs nil) (out nil) (cur nil) (name nil) (f nil))
    (setq p (outfile path "w"))
    (when p (close p))
    (errset (axlExportSetup sess sdb path (SfyAdeTags)))
    (foreach line (SfyAdeLines path)
      (setq segs (cons (if (and (geqp (strlen line) 8) (equal (substring line 1 8) "<setupdb"))
                         (strcat (car (parseString line ">")) ">maesdb\n")
                         line)
                   segs)))
    (if (null segs)
      (SfyLog (sprintf nil "%s/%s/maestro: axlExportSetup wrote nothing: its unmapped settings are not kept"
                lib cell))
      (setq out (list (list "maesdb" (SfyText (reverse segs)) nil))))
    (setq f (ddGetObj lib cell "maestro" "active.state"))
    (setq segs nil)
    (foreach line (SfyAdeLines (and f (getSGq f readPath)))
      (cond
        (cur
          (setq cur (cons line cur))
          (when (equal (car (parseString line " \t\n")) "</Test>")
            (setq segs (cons (list "maestate" name (SfyText (reverse cur))) segs))
            (setq cur nil)))
        ((equal (car (parseString line " \t\n")) "<Test")
          (setq name (cadr (parseString line "\"")))
          (setq cur (list line)))))
    (when cur
      (SfyLog (sprintf nil "%s/%s/maestro: active.state test %s is not closed: not kept" lib cell name)))
    (append out (mapcar (lambda (x) (cons "maestate" x)) (SfySortByKey (mapcar 'cdr segs))))))

;; Closes maestro session sess: ?forceClose t (virtuoso-bridge-lite closes so), else a plain
;; close for a release without it; logs when neither did, since the session's lock makes
;; the next open of that view fail (ASSEMBLER-8127).
(procedure (SfyAdeClose sess)
  ;; The plain close only when the forced one signals (a release without ?forceClose): its
  ;; nil answer is not trusted either way, and a plain close of a modified session could
  ;; ask to save.
  (unless (or (errset (maeCloseSession ?session sess ?forceClose t))
              (car (errset (maeCloseSession ?session sess))))
    (SfyLog (sprintf nil "maestro session %s did not close: close its view (or restart Virtuoso) before opening it again"
              sess))))

;; The expression text of maestro output so, or nil. ICADVM20.1 answers the parsed
;; expression, a list such as ymax(VT("/n1")) (live probe 2026-10-15); its printed form is
;; the ADE expression text.
(procedure (SfyAdeOutExpr so)
  (let ((e (car (errset (getSGq so expression)))))
    (unless (stringp e) (setq e (or (car (errset (getSGq so expr))) e)))
    (when (and e (listp e)) (setq e (sprintf nil "%L" e)))
    (and (stringp e) e)))

;; The signal of maestro output so (a net output), or nil.
(procedure (SfyAdeOutSignal so)
  (let ((o (car (errset (getSGq so signal)))))
    (unless (stringp o) (setq o (car (errset (getSGq so signalName)))))
    (and (stringp o) o)))

;; The texts output out of test answers (its expression and its signal) in session sess.
(procedure (SfyAdeOutTexts sess test out)
  (let ((r nil))
    (foreach so (car (errset (maeGetTestOutputs test ?session sess)))
      (when (equal (getSGq so name) out)
        (setq r (list (SfyAdeOutExpr so) (SfyAdeOutSignal so)))))
    r))

;; The records of one test, in spec 9 order; stored (SfyAdeStored's records) right after
;; its maestro record.
(procedure (SfyAdeDumpTest port lib cell sess sdb test stored)
  (let ((outs nil) (specs nil) (defs nil) (c nil) (m nil) (vl nil) (o nil) (p nil)
        (line nil) (segs nil) (e nil) (th nil) (tp nil) (sd nil))
    (SfyPut port "maestro" (list test lib cell "schematic" "spectre"))
    (foreach r stored (SfyPut port (car r) (cdr r)))
    ;; The variables this test sees: the global ones (GUESS(G18): maeGetSetup ?typeName
    ;; "variables" lists their names), then the test's own, which override a global of the
    ;; same name; the test temperature (simulator option temp, GUESS(G28)) as the var
    ;; "temperature", which the Rust side reads as the test temperature.
    (setq vl nil)
    (foreach n (car (errset (maeGetSetup ?typeName "variables" ?session sess)))
      (setq vl (cons (list n (SfyAtomStr (car (errset (maeGetVar n ?session sess))))) vl)))
    (setq th (car (errset (axlGetTest sdb test))))
    (when th
      (foreach n (cadr (car (errset (axlGetVars th))))
        (setq vl (cons (list n (SfyAtomStr (car (errset (axlGetVarValue (axlGetVar th n))))))
                   (setof x vl (nequal (car x) n))))))
    (setq tp (car (errset (maeGetSimOption test ?option "temp" ?session sess))))
    (when (and (stringp tp) (nequal tp ""))
      (setq vl (cons (list "temperature" tp) (setof x vl (nequal (car x) "temperature")))))
    (foreach x (SfySortByKey vl)
      (SfyPut port "var" (list (car x) (cadr x) "")))
    (unless (errset (progn
    (foreach a (sort (copy (maeGetEnabledAnalysis test ?session sess)) nil)
      (SfyPut port "analysis"
        (list a 1 (mapcar (lambda (kv) (list (SfyAtomStr (car kv)) (SfyAtomStr (cadr kv))))
                    (maeGetAnalysis test a ?session sess))))))
      t)
      (SfyRefuse 0 (sprintf nil "maestro %s analyses: SKILL error %s" test (SfyErrMsg))))
    (unless (errset (progn
    (foreach mf (maeGetEnvOption test ?option "modelFiles" ?session sess)
      (SfyPut port "model" (list "" (SfyAtomStr (car mf)) (SfyAtomStr (cadr mf))))))
      t)
      (SfyRefuse 0 (sprintf nil "maestro %s model files: SKILL error %s" test (SfyErrMsg))))
    (unless (errset (progn
    (foreach cn (sort (copy (maeGetSetup ?typeName "corners" ?session sess)) nil)
      (setq c (axlGetCorner sdb cn))
      (cond
        ;; ADE's built-in "Nominal" corner answers handle 0 (live ICADVM20.1 run): it is the
        ;; test's own setup, not a corner.
        ((and (or (null c) (equal c 0)) (equal cn "Nominal"))
          nil)
        ((or (null c) (equal c 0))
          (SfyRefuse 0 (sprintf nil "maestro %s: corner %s has no handle" test cn)))
        (t
          (SfyPut port "corner" (list cn (if (axlGetEnabled c) 1 0)))
          ;; axlGetModels answers (handle names), the names in setup order (live probe:
          ;; (1053 ("m.scs"))); a bare list of names is read too.
          (setq m (axlGetModels c))
          (foreach mn (if (and m (not (stringp (car m)))) (cadr m) m)
            (setq m (axlGetModel c mn))
            (if (or (null m) (equal m 0))
              (SfyRefuse 0 (sprintf nil "maestro %s: corner %s model %s has no handle" test cn mn))
              (SfyPut port "model"
                (list cn (SfyStrOr (axlGetModelFile m)) (SfyStrOr (axlGetModelSection m))))))
          ;; axlGetVars: (handle names) (IC23.1 reference).
          (setq vl (axlGetVars c))
          (foreach vn (sort (copy (cadr vl)) nil)
            (SfyPut port "var"
              (list vn (SfyAtomStr (axlGetVarValue (axlGetVar c vn))) cn)))))))
      t)
      (SfyRefuse 0 (sprintf nil "maestro %s corners: SKILL error %s" test (SfyErrMsg))))
    ;; IC23.1 outputs answer ~>expression and ~>signal (what virtuoso-bridge-lite reads live);
    ;; the reference names ~>expr and ~>signalName. GUESS(G29): one of the two pairs.
    (unless (errset (progn
    (foreach so (maeGetTestOutputs test ?session sess)
      (setq e (SfyAdeOutExpr so))
      (setq o (SfyAdeOutSignal so))
      (cond
        ;; ADE allows outputs without a name (a saved or plotted signal): no record.
        ((not (and (stringp (getSGq so name)) (nequal (getSGq so name) "")))
          (SfyLog (sprintf nil "skipped: maestro %s output without a name (%L %L)" test e o)))
        ((stringp e) (setq outs (cons (list (getSGq so name) "point" e) outs)))
        ((stringp o) (setq outs (cons (list (getSGq so name) "net" o) outs)))
        ;; A refusal, not a note: the outputs and their specs would be lost (G29).
        (t (SfyRefuse 0 (sprintf nil "maestro %s output %L: neither ~>expression ~>expr ~>signal nor ~>signalName answers a string"
                          test (getSGq so name))))))
    (foreach x (SfySortByKey outs)
      (SfyPut port "output" x)
      ;; axlGetSpecData: op "gt" "lt" or "range" with its values (shape below).
      (setq sd (errset (axlGetSpecData sdb (car x) test)))
      (unless sd
        (SfyLog (sprintf nil "skipped: maestro %s spec of %s: axlGetSpecData failed" test (car x))))
      ;; ICADVM20.1 answers one flat list, ("gt" "1") or ("range" "1" "2") (live probe);
      ;; a list of such lists, or (op (lo hi)), is read too.
      (setq sd (car sd))
      (foreach sp (if (and sd (stringp (car sd))) (list sd) sd)
        (case (SfyAtomStr (car sp))
          ("gt" (setq specs (cons (list (list (car x) "gt") (SfyAtomStr (cadr sp))) specs)))
          ("lt" (setq specs (cons (list (list (car x) "lt") (SfyAtomStr (cadr sp))) specs)))
          ("range"
            (setq specs (cons (list (list (car x) "gt")
                                (SfyAtomStr (if (listp (cadr sp)) (car (cadr sp)) (cadr sp))))
                          specs))
            (setq specs (cons (list (list (car x) "lt")
                                (SfyAtomStr (if (listp (cadr sp)) (cadr (cadr sp)) (caddr sp))))
                          specs)))
          (t (SfyLog (sprintf nil "skipped: maestro %s spec %s %L" test (car x) (car sp)))))))
    (foreach x (SfySortByKey specs)
      (SfyPut port "spec" (list (car (car x)) (cadr (car x)) (cadr x)))))
      t)
      (SfyRefuse 0 (sprintf nil "maestro %s outputs and specs: SKILL error %s" test (SfyErrMsg))))
    ;; GUESS(G22): the definitionFiles environment option is a list of file paths.
    (unless (errset (progn
    (foreach f (car (errset (maeGetEnvOption test ?option "definitionFiles" ?session sess)))
      (setq p (infile f))
      (if (null p)
        (SfyRefuse 0 (sprintf nil "maestro %s: cannot read definition file %s" test f))
        (progn
          (setq segs nil)
          (while (gets line p) (setq segs (cons line segs)))
          (close p)
          (setq defs (cons (list (car (last (parseString f "/"))) (SfyText (reverse segs)))
                       defs))))))
      t)
      (SfyRefuse 0 (sprintf nil "maestro %s definition files: SKILL error %s" test (SfyErrMsg))))
    (foreach x (SfySortByKey defs)
      (SfyPut port "defn" x))))

;; ================================================================ import

;; Creates the maestro setups that recs, the (line record) pairs of every maestro test of
;; the file, describe. A test lands in the target library of its cell (sfyTargets, else
;; lib, else the record's library); an existing maestro view is refused unless sfyOverwrite,
;; which replaces the test of the same name. Each refused call is logged with its line.
(procedure (SfyAdeImport recs lib)
  (let ((tests nil) (cur nil) (made nil) (sfyAdeGlobals nil))
    (foreach lr recs
      (if (equal (car (cadr lr)) "maestro")
        (progn
          (when cur (setq tests (cons (reverse cur) tests)))
          (setq cur (list lr)))
        (setq cur (cons lr cur))))
    (when cur (setq tests (cons (reverse cur) tests)))
    (foreach test (reverse tests)
      (setq made (SfyAdeLoadTest test lib made)))))

;; One test: open (creating) the maestro view in append mode, fill it, save, close. made
;; lists the "lib/cell" maestro views this import already wrote (a view of several tests is
;; not "existing" for its second test); returns it with this test's view added.
;; A test with the view's own setup (a maesdb record, spec 9.7) restores it first: the test
;; states go into the view's active.state before the session opens, the setup XML through
;; axlImportSetup ... 'overwrite, which replaces in place (G32); the records then hold only
;; what Schemify changed, applied without deleting any test.
(procedure (SfyAdeLoadTest test lib made)
  (let ((line (car (car test))) (m (cadr (car test))) (tlib nil) (cell nil) (sess nil)
        (sdbRec nil) (from nil))
    (setq cell (nth 3 m))
    (setq tlib (or (arrayref sfyTargets (strcat (nth 2 m) "/" cell)) lib (nth 2 m)))
    (setq sdbRec (car (setof lr (cdr test) (equal (car (cadr lr)) "maesdb"))))
    (when sdbRec (setq from (SfyAdeXmlLib (SfyAdeTextLines (nth 1 (cadr sdbRec))))))
    (cond
      ((null (ddGetObj tlib cell))
        (SfyRefuse line (sprintf nil "maestro %s: no cell %s/%s" (nth 1 m) tlib cell)))
      ((and (ddGetObj tlib cell "maestro") (not sfyOverwrite)
            (not (member (strcat tlib "/" cell) made)))
        (SfyRefuse line (sprintf nil "%s/%s/maestro exists; pass ?overwrite t" tlib cell)))
      ((and sdbRec (not (SfyAdeWriteStates tlib cell
                          (setof lr (cdr test) (equal (car (cadr lr)) "maestate")) from)))
        (SfyRefuse line (sprintf nil "cannot write the test states of %s/%s/maestro (active.state)"
                          tlib cell)))
      ((null (setq sess (car (errset (maeOpenSetup tlib cell "maestro" ?mode "a")))))
        (SfyRefuse line (sprintf nil "cannot open %s/%s/maestro (a %s/%s/maestro/maestro.sdb.cdslck left by a killed session blocks it)"
                          tlib cell tlib cell)))
      (t
        ;; The error is logged inside the body: the close's own errset would clear its
        ;; message on the way out.
        (unwindProtect
          (cond
            ((and sdbRec (not (SfyAdeRestore sess (cadr sdbRec) from tlib)))
              (SfyRefuse (car sdbRec) (sprintf nil "maestro %s: axlImportSetup did not restore the setup of %s/%s/maestro"
                                        (nth 1 m) tlib cell)))
            ((not (let ((sfyAdeRestore (and sdbRec t))) (errset (SfyAdeFill test tlib cell sess) t)))
              (SfyRefuse line (sprintf nil "maestro %s: SKILL error %s" (nth 1 m) (SfyErrMsg)))))
          (SfyAdeClose sess))
        (setq made (cons (strcat tlib "/" cell) made))))
    made))

;; The lines of text field x, each with its newline (Maestro's XML has no empty line).
(procedure (SfyAdeTextLines x)
  (mapcar (lambda (l) (strcat l "\n")) (parseString (if (stringp x) x (buildString x "")) "\n")))

;; The 1-based position of the first occurrence of part in s, or nil.
(procedure (SfyAdeFind s part)
  (let ((n (strlen part)) (k 1) (at nil))
    (while (and (null at) (leqp (plus k n -1) (strlen s)))
      (when (equal (substring s k n) part) (setq at k))
      (setq k (add1 k)))
    at))

;; s with the first occurrence of part replaced by new (s when there is none).
(procedure (SfyAdeSwap s part new)
  (let ((at (SfyAdeFind s part)))
    (if (null at)
      s
      (strcat (if (greaterp at 1) (substring s 1 (sub1 at)) "") new
        (or (substring s (plus at (strlen part))) "")))))

;; The design library of the first test of setup XML lines: the <value> after its
;; <option>lib line, or nil.
(procedure (SfyAdeXmlLib lines)
  (let ((prev nil) (w nil) (lib nil))
    (foreach l lines
      (setq w (car (parseString l " \t\n")))
      (when (and (null lib) (equal prev "<option>lib") (stringp w)
                 (geqp (strlen w) 15) (equal (substring w 1 7) "<value>"))
        (setq lib (substring w 8 (difference (strlen w) 15))))
      (setq prev w))
    lib))

;; lines (setup XML or test states) with design library from renamed to: a test's lib
;; option, a parameter location's first part, a state's designInfo. The view's setup names
;; the library it was dumped from; the load may target another (?lib, --lib).
(procedure (SfyAdeRelib lines from to)
  (let ((prev nil) (w nil) (out nil))
    (foreach l lines
      (setq w (car (parseString l " \t\n")))
      (cond
        ((or (null from) (equal from to)) nil)
        ((and (equal prev "<option>lib") (equal w (strcat "<value>" from "</value>")))
          (setq l (SfyAdeSwap l (strcat "<value>" from "<") (strcat "<value>" to "<"))))
        ((SfyAdeFind l (strcat "<location>" from "/"))
          (setq l (SfyAdeSwap l (strcat "<location>" from "/") (strcat "<location>" to "/"))))
        ((SfyAdeFind l "Name=\"designInfo\"")
          (setq l (SfyAdeSwap l (strcat "(\"" from "\" ") (strcat "(\"" to "\" ")))))
      (setq out (cons l out))
      (setq prev w))
    (reverse out)))

;; Writes the maestate records' test states (rec: (line ("maestate" test xml))) into the
;; active.state of maestro view tlib/cell, created first when missing, keeping the states of
;; other tests the file holds; their design library from becomes tlib. GUESS(G40): a test
;; that axlImportSetup restores takes its state from active.state as it is when the session
;; opens. t when written.
(procedure (SfyAdeWriteStates tlib cell recs from)
  (let ((sess nil) (f nil) (old nil) (head nil) (keep nil) (cur nil) (name nil) (p nil)
        (ours (mapcar (lambda (lr) (nth 1 (cadr lr))) recs)))
    (unless (ddGetObj tlib cell "maestro")
      (setq sess (car (errset (maeOpenSetup tlib cell "maestro" ?mode "a"))))
      (when sess
        (errset (maeSaveSetup ?lib tlib ?cell cell ?view "maestro" ?session sess))
        (SfyAdeClose sess)))
    (setq f (ddGetObj tlib cell "maestro" "active.state" nil "a"))
    (setq old (SfyAdeLines (and f (getSGq f readPath))))
    ;; The declaration and <statedb> lines of the file, else Maestro's own.
    (setq head (setof l old (member (car (parseString l " \t\n>")) (list "<?xml" "<statedb"))))
    (unless (equal (length head) 2)
      (setq head (list "<?xml version=\"1.0\"?>\n" "<statedb version=\"5\">active\n")))
    (foreach l old
      (cond
        (cur
          (unless (member name ours) (setq keep (cons l keep)))
          (when (equal (car (parseString l " \t\n")) "</Test>") (setq cur nil)))
        ((equal (car (parseString l " \t\n")) "<Test")
          (setq name (cadr (parseString l "\"")))
          (setq cur t)
          (unless (member name ours) (setq keep (cons l keep))))))
    (setq p (and f (outfile (ddGetObjWritePath f) "w")))
    (when p
      (foreach l head (fprintf p "%s" l))
      (foreach l (reverse keep) (fprintf p "%s" l))
      (foreach lr recs
        (foreach l (SfyAdeRelib (SfyAdeTextLines (nth 2 (cadr lr))) from tlib)
          (fprintf p "%s" l)))
      (fprintf p "</statedb>\n")
      (close p)
      t)))

;; Restores the setup XML of rec (("maesdb" xml digests)), its design library from renamed
;; to, into session sess with axlImportSetup ... "" 'overwrite (G32: replaces in place,
;; every test, corner, var, parameter, spec and run option once). Its errors print to the
;; CIW without signalling, so success is read back: t when the session then lists a test.
(procedure (SfyAdeRestore sess rec from to)
  (let ((path (SfyAdeTmp)) (p nil))
    (setq p (outfile path "w"))
    (when p
      (foreach l (SfyAdeRelib (SfyAdeTextLines (nth 1 rec)) from to)
        (fprintf p "%s" l))
      (close p)
      (and (car (errset (axlImportSetup sess path (SfyAdeTags) "" 'overwrite)))
           (car (errset (maeGetSetup ?session sess)))
           t))))

;; The parameter names the parameters statements (and .param lines) of the model files
;; define: a design variable of the same name makes every Spectre point fail at read-in
;; (SFE-59, spec 9.7). Files that cannot be read (a path through an unset variable) count
;; for nothing.
(procedure (SfyAdeModelParams files)
  (let ((names nil) (cont nil) (words nil) (prev nil) (lw nil) (kv nil))
    (foreach f files
      (setq cont nil)
      (foreach line (SfyAdeLines f)
        (setq words (parseString line " \t\n"))
        (when (or cont (member (car words) (list "parameters" ".param" ".PARAM")))
          (setq prev nil)
          (foreach w (if cont words (cdr words))
            (setq kv (parseString w "="))
            (cond
              ((and (equal w "=") prev) (setq names (cons prev names)))
              ((and (cdr kv) (nequal (substring w 1 1) "=")) (setq names (cons (car kv) names)))
              ((and (equal (length kv) 1) (equal (substring w (strlen w) 1) "=")
                    (greaterp (strlen w) 1))
                (setq names (cons (car kv) names))))
            (setq prev w)))
        (setq lw (car (last words)))
        (setq cont (and lw (equal (substring lw (strlen lw) 1) "\\")))))
    names))

;; t when opts is a list of (key value) strings.
(procedure (SfyAdeOptsOk opts)
  (not (exists kv opts
         (not (and (listp kv) (equal (length kv) 2) (stringp (car kv)) (stringp (cadr kv)))))))

;; A model name for file in corner cn not yet in used (a list of "corner/name"): the file's
;; base name, else base_k.
(procedure (SfyAdeModelName cn file used)
  (let ((base (car (last (parseString file "/")))) (k 1) (n nil))
    (setq n base)
    (while (member (strcat cn "/" n) used)
      (setq k (add1 k))
      (setq n (sprintf nil "%s_%d" base k)))
    n))

;; Sets the spec of output o of test name in session sess: bounds gt and lt (strings or
;; nil; both are a ?range), with weight w when it is a non-empty string.
(procedure (SfyAdeSetSpec sess name o gt lt w)
  (if (and (stringp w) (nequal w ""))
    (cond
      ((and gt lt) (maeSetSpec o name ?range (list gt lt) ?weight w ?session sess))
      (gt (maeSetSpec o name ?gt gt ?weight w ?session sess))
      (t (maeSetSpec o name ?lt lt ?weight w ?session sess)))
    (cond
      ((and gt lt) (maeSetSpec o name ?range (list gt lt) ?session sess))
      (gt (maeSetSpec o name ?gt gt ?session sess))
      (t (maeSetSpec o name ?lt lt ?session sess)))))

;; Writes the records of one test into the open session sess. sfyAdeRestore (bound by
;; SfyAdeLoadTest): the session holds the view's restored setup (spec 9.7), so the test is
;; kept when it exists (never deleted and created again, which would wipe what the records
;; do not carry) and an output that exists keeps its own definition.
(procedure (SfyAdeFill test tlib cell sess)
  (let ((restore sfyAdeRestore) (m (cadr (car test))) (name nil) (sdb nil) (models nil) (defs nil) (specs nil)
        (used nil) (opts nil) (f nil) (line nil) (c nil) (mh nil) (mn nil) (id nil) (p nil)
        (ok nil) (gt nil) (lt nil) (outs nil) (have nil) (clash nil) (old nil) (w nil))
    (setq name (nth 1 m))
    (prog ()
      (setq have (member name (car (errset (maeGetSetup ?session sess)))))
      (when (and have (not restore))
        (unless (car (errset (maeDeleteTest name ?session sess)))
          (SfyRefuse (car (car test)) (sprintf nil "cannot replace maestro test %s" name))
          (return nil))
        (setq have nil))
      (unless (or have
                  (car (errset (maeCreateTest name ?lib tlib ?cell cell ?view (nth 4 m)
                                 ?simulator (nth 5 m) ?session sess))))
        (SfyRefuse (car (car test)) (sprintf nil "cannot create maestro test %s" name))
        (return nil))
      (SfyCount (if have "maestro restored" "maestro"))
      (setq sdb (car (errset (axlGetMainSetupDB sess))))
      ;; The parameters of the test's model files, those of the records and those the
      ;; restored test already has: a design variable of one of these names is refused.
      (setq clash (SfyAdeModelParams
                    (append
                      (mapcar (lambda (lr) (nth 2 (cadr lr)))
                        (setof lr (cdr test)
                          (and (equal (car (cadr lr)) "model") (equal (nth 1 (cadr lr)) ""))))
                      (mapcar 'car
                        (setof x (car (errset (maeGetEnvOption name ?option "modelFiles"
                                                ?session sess)))
                          (listp x))))))
      (setq old (mapcar (lambda (o) (getSGq o name))
                  (car (errset (maeGetTestOutputs name ?session sess)))))
      (foreach lr (cdr test)
        (setq line (car lr))
        (setq f (cdr (cadr lr)))
        (setq ok (car (errset
          (case (car (cadr lr))
            ("var"
              (cond
                ((and (equal (nth 2 f) "") (member (nth 0 f) clash))
                  (SfyRefuse line (sprintf nil "maestro %s: design variable %s is also a parameter of the test's model files: not set, every point would fail (SFE-59; needs_review design-variable)"
                                    name (nth 0 f)))
                  'refused)
                ;; The test temperature is the simulator option temp; a global design
                ;; variable named temperature would set nothing.
                ((and (equal (nth 2 f) "") (equal (nth 0 f) "temperature"))
                  (maeSetSimOption name ?options (list (list "temp" (nth 1 f))) ?session sess))
                ((equal (nth 2 f) "")
                  ;; Test-level variables were dumped as each test's own globals: two
                  ;; tests of one view that disagree now share the later value.
                  (setq id (strcat tlib "/" cell "/" (nth 0 f)))
                  (when (and (assoc id sfyAdeGlobals)
                             (nequal (cadr (assoc id sfyAdeGlobals)) (nth 1 f)))
                    (SfyCount "maestro var conflict")
                    (SfyLog (sprintf nil "warning line %d: maestro %s: global var %s is %s in an earlier test of this view and %s here; both now see %s"
                              line name (nth 0 f) (cadr (assoc id sfyAdeGlobals)) (nth 1 f) (nth 1 f))))
                  (setq sfyAdeGlobals (cons (list id (nth 1 f)) sfyAdeGlobals))
                  (maeSetVar (nth 0 f) (nth 1 f) ?session sess))
                (t
                  (maeSetVar (nth 0 f) (nth 1 f) ?typeName "corner" ?typeValue (list (nth 2 f))
                    ?session sess))))
            ("analysis"
              ;; Boolean form fields (saveOppoint, finalTimeOp) take t or nil, not the
              ;; strings the dump holds: "t" sent as a string fails ADE-5019 (live run).
              (if (SfyAdeOptsOk (nth 2 f))
                (maeSetAnalysis name (nth 0 f) ?enable (equal (nth 1 f) 1)
                  ?options (mapcar (lambda (kv)
                                     (cond ((equal (cadr kv) "t") (list (car kv) t))
                                           ((equal (cadr kv) "nil") (list (car kv) nil))
                                           (t kv)))
                             (nth 2 f))
                  ?session sess)
                nil))
            ("corner" (maeSetCorner (nth 0 f) ?enabled (equal (nth 1 f) 1) ?session sess))
            ("model"
              (if (equal (nth 0 f) "")
                (setq models (cons (list (nth 1 f) (nth 2 f)) models))
                (progn
                  (setq c (and sdb (axlGetCorner sdb (nth 0 f))))
                  (setq mn (SfyAdeModelName (nth 0 f) (nth 1 f) used))
                  (setq used (cons (strcat (nth 0 f) "/" mn) used))
                  (setq mh (and c (axlPutModel c mn)))
                  (and mh (axlSetModelFile mh (nth 1 f))
                    (or (equal (nth 2 f) "") (axlSetModelSection mh (nth 2 f)))))))
            ("output"
              (setq outs (cons (nth 0 f) outs))
              (cond
                ((not (member (nth 0 f) old))
                  (if (equal (nth 1 f) "net")
                    (maeAddOutput (nth 0 f) name ?outputType "net" ?signalName (nth 2 f) ?session sess)
                    (maeAddOutput (nth 0 f) name ?outputType "point" ?expr (nth 2 f) ?session sess)))
                ((member (nth 2 f) (SfyAdeOutTexts sess name (nth 0 f))) t)
                (t
                  (SfyRefuse line (sprintf nil "maestro %s: output %s was changed in Schemify; Maestro keeps its own (edit it there)"
                                    name (nth 0 f)))
                  'refused)))
            ("spec" (setq specs (cons (list line f) specs)))
            ;; Restored by SfyAdeLoadTest before this test was filled.
            ("maesdb" t)
            ("maestate" t)
            ("defn"
              (setq id (ddGetObj tlib cell "maestro" (nth 0 f) nil "a"))
              (setq p (and id (outfile (ddGetObjWritePath id) "w")))
              (when p
                (foreach chunk (if (stringp (nth 1 f)) (list (nth 1 f)) (nth 1 f))
                  (fprintf p "%s" chunk))
                (close p)
                (setq defs (cons (ddGetObjWritePath id) defs))))
            (t nil)))))
        (cond
          ((eq ok 'refused) nil)
          (ok (SfyCount (strcat "maestro " (car (cadr lr)))))
          (t (SfyRefuse line (sprintf nil "maestro %s: %s %L did not land" name (car (cadr lr)) f)))))
      ;; One spec per output (GUESS(G21): both bounds are a ?range (lo hi)).
      (foreach o (reverse outs)
        (setq gt nil)
        (setq lt nil)
        (foreach s specs
          (when (equal (car (cadr s)) o)
            (setq line (car s))
            (if (equal (cadr (cadr s)) "gt")
              (setq gt (caddr (cadr s)))
              (setq lt (caddr (cadr s))))))
        (when (or gt lt)
          ;; A restored spec keeps its weight (axlGetSpecWeight answers "" when unset).
          (setq w (and restore sdb
                       (car (errset (axlGetSpecWeight (axlGetSpec sdb (strcat name "." o)))))))
          (setq ok (car (errset (SfyAdeSetSpec sess name o gt lt w))))
          (if ok
            (SfyCount "maestro spec set")
            (SfyRefuse line (sprintf nil "maestro %s: spec of %s did not land" name o)))))
      (when models (setq opts (cons (list "modelFiles" (reverse models)) opts)))
      (when defs (setq opts (cons (list "definitionFiles" (reverse defs)) opts)))
      (when (and opts (null (car (errset (maeSetEnvOption name ?options (reverse opts)
                                                ?session sess)))))
        (SfyRefuse (car (car test)) (sprintf nil "maestro %s: model or definition files did not land" name)))
      ;; The export's model files are the test's: a bench without any clears what the session
      ;; gave the new test (a PDK's libInit gives every new test its default model files; live
      ;; run). Not clearing them is logged, not refused: the setup is still usable. A
      ;; restored test keeps the model files of its state unless the records carry them.
      (unless (or models restore)
        (if (car (errset (maeSetEnvOption name ?options (list (list "modelFiles" nil)) ?session sess)))
          (SfyCount "maestro default models cleared")
          (SfyLog (sprintf nil "maestro %s: the session's default model files stay on the test" name))))
      (unless (car (errset (maeSaveSetup ?lib tlib ?cell cell ?view "maestro" ?session sess)))
        (SfyRefuse (car (car test)) (sprintf nil "cannot save %s/%s/maestro" tlib cell))))))
