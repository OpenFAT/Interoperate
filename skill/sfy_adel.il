;; sfy_adel.il: ADE L states to and from .sfyx (interop spec 9.8). Entry points:
;; SfyAdeLExport (a cell and one of its ADE L states) and SfyAdeLImport (the way back).
;;
;; ICADVM20.1 has no ADE L session; its states are files. A directory state is
;; <root>/<lib>/<cell>/<simulator>/<state>/ (root: ?stateDir, else asimenv saveDir, else
;; ~/.artist_states), a cellview state the view <state> of the cell. Their component files
;; are SKILL source: this file never loads or evaluates them. It copies every file
;; verbatim, through the shell's base64 (one component is a tar archive, which a SKILL
;; string cannot hold), and Rust (virtuoso::adel) reads them as data. Paths go to the shell
;; single-quoted; a path holding a quote is refused. Prefix form only (spec 5.1).

;; ================================================================ export

;; Dumps cell of library lib (with its design closure and, with ref, the masters outside
;; lib, as SfyExport does, but no maestro view) and its ADE L state `state` to path. from:
;; "dir" (a directory state under stateDir) or "cellview" (the view state of the cell).
;; Writes <path>.log; t on PASS.
(procedure (SfyAdeLExport path lib cell state @key (from "dir") (stateDir nil)
             (simulator "spectre") (ref t))
  (let ((sfyLogPort (SfyLogOpen path)) (sfyCountTab (makeTable "sfyCounts" nil))
        (sfyCountKeys nil) (sfyFails 0) (design nil) (refs nil) (root "") (dir nil) (port nil))
    (SfyLog (sprintf nil "SfyAdeLExport %s %s/%s state %s from %s" path lib cell state from))
    (SfyPreflight)
    (when (equal from "dir") (setq root (SfyAdeLRoot stateDir)))
    (setq dir (SfyAdeLDir lib cell state from root simulator))
    (cond
      ((null (ddGetObj lib cell)) (SfyRefuse 0 (sprintf nil "no cell %s/%s" lib cell)))
      ((not (member from (list "dir" "cellview")))
        (SfyRefuse 0 (sprintf nil "?from %L: \"dir\" or \"cellview\"" from)))
      ((null dir) (SfyRefuse 0 (sprintf nil "no directory for state %s of %s/%s" state lib cell)))
      ((not (isFile (strcat dir "/ADE_state.info")))
        (SfyRefuse 0 (sprintf nil "no ADE L state %s at %s (no ADE_state.info)" state dir)))
      ((not (errset
              (progn
                (setq design (SfyDesignClosure lib (list cell)))
                (when ref (setq refs (SfyRefMasters lib design)))
                (setq port (outfile path "w"))
                (if (null port)
                  (SfyRefuse 0 (sprintf nil "cannot write %s" path))
                  (unwindProtect
                    (progn
                      (SfyPut port "sfyx" (list 1 (SfyProducer)))
                      (SfyDumpCells port refs "ref")
                      (SfyDumpCells port (mapcar (lambda (c) (list lib c nil)) design) "design")
                      (SfyAdeLDump port (list state lib cell simulator from root) dir)
                      (SfyPut port "end" nil))
                    (close port))))
              t))
        (SfyRefuse 0 (sprintf nil "SKILL error %s" (SfyErrMsg)))))
    (SfyLogClose)))

;; The root of directory states: stateDir, else asimenv saveDir, else ~/.artist_states, a
;; leading ~ as $HOME and without a trailing /.
(procedure (SfyAdeLRoot stateDir)
  (let ((r (or stateDir (car (errset (envGetVal "asimenv" "saveDir"))) "~/.artist_states")))
    (unless (stringp r) (setq r "~/.artist_states"))
    (when (equal (substring r 1 1) "~")
      (setq r (strcat (or (getShellEnvVar "HOME") "") (or (substring r 2) ""))))
    (while (and (greaterp (strlen r) 1) (equal (substring r (strlen r) 1) "/"))
      (setq r (substring r 1 (sub1 (strlen r)))))
    r))

;; The directory of state `state` of lib/cell: <root>/<lib>/<cell>/<simulator>/<state> (from
;; "dir"), else the view's directory under the library's (from "cellview"); nil without the
;; library or for a path the shell could not take quoted.
(procedure (SfyAdeLDir lib cell state from root simulator)
  (let ((l (ddGetObj lib)) (d nil))
    (setq d (if (equal from "dir")
              (strcat root "/" lib "/" cell "/" simulator "/" state)
              (and l (stringp (getSGq l readPath)) (strcat (getSGq l readPath) "/" cell "/" state))))
    (and d (SfyAdeLSafe d) d)))

;; t when path can go to the shell inside single quotes.
(procedure (SfyAdeLSafe path)
  (not (member "'" (parseString path ""))))

;; The scratch file of the base64 copies, one per user.
(procedure (SfyAdeLTmp)
  (sprintf nil "/tmp/sfy_%s_adel.b64" (or (getShellEnvVar "USER") "user")))

;; The records of one state: ("adel" state lib cell simulator from root) (head), then
;; ("adelfile" name base64) per file of dir, by name.
(procedure (SfyAdeLDump port head dir)
  (let ((tmp (SfyAdeLTmp)) (b64 nil) (p nil))
    (SfyPut port "adel" head)
    (foreach f (sort (setof f (getDirFiles dir)
                       (and (not (member f (list "." "..")))
                            (isFile (strcat dir "/" f)) (not (isDir (strcat dir "/" f)))))
                 nil)
      (setq p (strcat dir "/" f))
      (setq b64 (SfyAdeLBase64 p tmp))
      (if b64
        (SfyPut port "adelfile" (list f (SfyText (list b64))))
        (SfyRefuse 0 (sprintf nil "cannot copy %s (base64)" p))))))

;; The base64 text of file path (through tmp), or nil.
(procedure (SfyAdeLBase64 path tmp)
  (let ((p nil) (line nil) (out nil))
    (when (and (SfyAdeLSafe path)
               (equal 0 (system (sprintf nil "base64 -w 0 '%s' > '%s'" path tmp))))
      (setq p (infile tmp))
      (when p
        (setq out "")
        (while (gets line p) (setq out (strcat out line)))
        (close p)))
    out))

;; ================================================================ import

;; Loads path, an export that may hold ADE L states (adel and adelfile records, spec 9.8):
;; first its cells and maestro tests through SfyImport (every other record copied to
;; <path>.cells.sfyx, which logs to <path>.cells.sfyx.log), then each state's files written
;; back byte for byte into library lib (default: the record's): a directory state under
;; stateDir (default: the root it was dumped from), a cellview state into the view of its
;; cell. An existing state is refused unless overwrite, which replaces its files and
;; deletes the files it no longer has. Writes <path>.log; t on PASS of both.
(procedure (SfyAdeLImport path @key (lib nil) (stateDir nil) (callbacks nil) (check t)
             (overwrite nil))
  (let ((sfyLogPort (SfyLogOpen path)) (sfyCountTab (makeTable "sfyCounts" nil))
        (sfyCountKeys nil) (sfyFails 0) (lines nil) (states nil) (rest nil) (tag nil) (n 0)
        (r nil) (p nil) (cells (strcat path ".cells.sfyx")))
    (SfyLog (sprintf nil "SfyAdeLImport %s" path))
    (SfyPreflight)
    (setq lines (SfyAdeLines path))
    (if (null lines)
      (SfyRefuse 0 (sprintf nil "cannot read %s (an absolute path)" path))
      (progn
        (foreach line lines
          (setq n (add1 n))
          (setq tag (SfyAdeLTag line))
          (cond
            ((null tag) (setq rest (cons line rest)))
            ((null (setq r (car (errset (linereadstring line)))))
              (SfyRefuse n "not readable as SKILL data"))
            ((equal tag "adel") (setq states (cons (list (list n r)) states)))
            ((null states) (SfyRefuse n "adelfile before any adel record"))
            (t (setq states (cons (append (car states) (list (list n r))) (cdr states))))))
        (setq p (outfile cells "w"))
        (if (null p)
          (SfyRefuse 0 (sprintf nil "cannot write %s" cells))
          (progn
            (foreach line (reverse rest) (fprintf p "%s" line))
            (close p)
            (if (SfyImport cells ?lib lib ?callbacks callbacks ?check check ?overwrite overwrite)
              (SfyLog (sprintf nil "cells and maestro: SfyImport %s: PASS" cells))
              (SfyRefuse 0 (sprintf nil "cells and maestro: SfyImport %s: FAIL (see its log)" cells)))
            (foreach st (reverse states)
              (unless (errset (SfyAdeLRestore st lib stateDir overwrite) t)
                (SfyRefuse (car (car st)) (sprintf nil "SKILL error %s" (SfyErrMsg)))))))))
    (SfyLogClose)))

;; "adel" or "adelfile" for a line holding that record, else nil.
(procedure (SfyAdeLTag line)
  (cond
    ((and (geqp (strlen line) 8) (equal (substring line 1 8) "(\"adel\" ")) "adel")
    ((and (geqp (strlen line) 12) (equal (substring line 1 12) "(\"adelfile\" ")) "adelfile")))

;; Writes one state back: st is ((line adelRecord) (line adelfileRecord)...).
(procedure (SfyAdeLRestore st lib stateDir overwrite)
  (let ((line (car (car st))) (r (cadr (car st))) (tlib nil) (cell nil) (name nil) (from nil)
        (sim nil) (root "") (dir nil) (names nil) (tmp (SfyAdeLTmp)) (p nil) (ok t) (f nil))
    (setq name (nth 1 r))
    (setq tlib (or lib (nth 2 r)))
    (setq cell (nth 3 r))
    (setq sim (nth 4 r))
    (setq from (nth 5 r))
    (when (equal from "dir") (setq root (or stateDir (nth 6 r))))
    (setq names (mapcar (lambda (lr) (nth 1 (cadr lr))) (cdr st)))
    (when (and (equal from "cellview") (ddGetObj tlib cell) (null (ddGetObj tlib cell name)))
      ;; A new view: DD makes it (and its directory) before the files land in it.
      (errset (ddGetObj tlib cell name nil nil "a")))
    (setq dir (SfyAdeLDir tlib cell name from root sim))
    (cond
      ((null (ddGetObj tlib cell)) (SfyRefuse line (sprintf nil "ADE L state %s: no cell %s/%s" name tlib cell)))
      ((null dir) (SfyRefuse line (sprintf nil "ADE L state %s: no directory for it in %s/%s" name tlib cell)))
      ((and (isFile (strcat dir "/ADE_state.info")) (not overwrite))
        (SfyRefuse line (sprintf nil "ADE L state %s exists at %s; pass ?overwrite t" name dir)))
      ((not (equal 0 (system (sprintf nil "mkdir -p '%s'" dir))))
        (SfyRefuse line (sprintf nil "ADE L state %s: cannot make %s" name dir)))
      (t
        (when (equal from "dir")
          ;; ADE marks every level of a state directory.
          (foreach d (list (strcat root "/" tlib) (strcat root "/" tlib "/" cell)
                       (strcat root "/" tlib "/" cell "/" sim))
            (setq f (strcat d "/.sevSaveDir"))
            (unless (isFile f)
              (setq p (outfile f "w"))
              (when p (close p)))))
        (foreach old (getDirFiles dir)
          (setq f (strcat dir "/" old))
          (when (and (not (member old (list "." ".."))) (not (member old names))
                     (isFile f) (not (isDir f)))
            (system (sprintf nil "rm -f '%s'" f))
            (SfyCount "adel file removed")))
        (foreach lr (cdr st)
          (setq f (strcat dir "/" (nth 1 (cadr lr))))
          (setq p (outfile tmp "w"))
          (when p
            (fprintf p "%s" (SfyTextValue (nth 2 (cadr lr))))
            (close p))
          (if (and p (equal 0 (system (sprintf nil "base64 -d '%s' > '%s'" tmp f))))
            (SfyCount "adel file")
            (progn
              (setq ok nil)
              (SfyRefuse (car lr) (sprintf nil "ADE L state %s: cannot write %s" name f)))))
        (when ok (SfyCount "adel state"))))))
