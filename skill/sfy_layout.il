;; sfy_layout.il: layout cellviews for the KLayout flow (gap H; the Rust half is
;; src/klayout.rs). Schemify does not model layout, it carries it:
;;
;; - SfyLayoutExport writes a bundle directory for `schemify-interop layout-export`:
;;   layout.gds (strmout -ignoreCurrentTimestamp: the same view gives the same bytes),
;;   layermap (the map strmout used), display.drf (the session's merged display resources),
;;   layout.sfyx (the technology's LPPs, PCell instances and the counts of what GDS does
;;   not carry) and oa/ (the view directory's files but its lock files).
;; - SfyLayoutImport applies the bundle `schemify-interop layout-import` writes: the
;;   stashed OA files back byte for byte (restore), or the edited GDS through strmin with
;;   the export's own layer map (a read-only tech cannot map automatically, XSTRM-74).
;;
;; XStream and the file work run through sh; both entry points signal on a failure.

;; s single-quoted for sh; a string holding a quote signals.
(procedure (SfyLayoutQ s)
  (when (member "'" (parseString s ""))
    (error "schemify: a path with a single quote: %s" s))
  (strcat "'" s "'"))

;; Runs cmd through sh, signalling when it exits non-zero.
(procedure (SfyLayoutSh what cmd)
  (unless (sh cmd)
    (error "schemify: %s failed: %s" what cmd))
  t)

;; The directory of view lib/cell/view, whether or not it exists.
(procedure (SfyLayoutDir lib cell view)
  (let ((l (ddGetObj lib)))
    (unless l (error "schemify: no library %s in cds.lib" lib))
    (strcat (getSGq l readPath) "/" cell "/" view)))

;; Writes record rec (a list: tag and fields) as one .sfyx line.
(procedure (SfyLayoutPut port rec)
  (SfyPutValue port rec)
  (fprintf port "\n"))

;; The records of the .sfyx file at path (lineread: data, never evaluated).
(procedure (SfyLayoutRecords path)
  (let ((p (infile path)) (recs nil) r)
    (unless p (error "schemify: cannot read %s" path))
    (while (setq r (lineread p))
      (when (listp r) (setq recs (cons (car r) recs))))
    (close p)
    (reverse recs)))

;; The open cellviews of lib/cell/view.
(procedure (SfyLayoutOpen lib cell view)
  (setof cv (dbGetOpenCellViews)
    (and (equal (getSGq cv libName) lib) (equal (getSGq cv cellName) cell)
      (equal (getSGq cv viewName) view))))

;; (SfyLayoutExport lib cell dir @optional (view "layout")): writes the bundle of
;; lib/cell/view into dir (created; an earlier bundle's oa/ is replaced) and returns dir.
;; Pins go out as shape and text with property 100 (-pinAttNum), which strmin reads back.
(procedure (SfyLayoutExport lib cell dir @optional (view "layout"))
  (let ((vdir (SfyLayoutDir lib cell view)) (q (SfyLayoutQ dir)) tech cv p)
    (unless (ddGetObj lib cell view) (error "schemify: no view %s/%s/%s" lib cell view))
    (setq tech (techGetTechFile (ddGetObj lib)))
    (unless tech (error "schemify: library %s has no technology" lib))
    (SfyLayoutSh "mkdir" (sprintf nil "rm -rf %s/oa && mkdir -p %s/oa" q q))
    (drDumpDrf (strcat dir "/display.drf"))
    (setq cv (dbOpenCellViewByType lib cell view nil "r"))
    (unless cv (error "schemify: cannot open %s/%s/%s" lib cell view))
    (setq p (outfile (strcat dir "/layout.sfyx") "w"))
    (unless p (error "schemify: cannot write %s/layout.sfyx" dir))
    (SfyLayoutPut p (list "sfyx" 1 "schemify.il SfyLayoutExport"))
    (SfyLayoutPut p (list "layout" lib cell view (getSGq tech libName)))
    (SfyLayoutPut p (list "dbu" (SfyAtomStr (techGetDBUPerUU tech "maskLayout"))))
    (foreach lp (getSGq tech lps)
      (SfyLayoutPut p (list "lp" (getSGq lp name) (getSGq lp purpose) (SfyStrOr (getSGq lp packet))
        (if (fixp (getSGq lp priority)) (getSGq lp priority) 0) (SfyAtomStr (getSGq lp visible)))))
    ;; A PCell instance's master is a sub-master: detect it by its superMaster (probe layout).
    (foreach i (getSGq cv instances)
      (let ((sm (car (errset (getSGq (getSGq i master) superMaster)))))
        (when sm
          (SfyLayoutPut p (list "pcell" (getSGq i name) (getSGq sm libName) (getSGq sm cellName)
            (buildString (mapcar (lambda (x) (sprintf nil "%s=%s" (getSGq x name) (SfyAtomStr (getSGq x value))))
              (getSGq i prop)) " "))))))
    (foreach kv (list
                  (list "net" (getSGq cv nets))
                  (list "prop" (getSGq cv prop))
                  (list "via" (car (errset (getSGq cv vias))))
                  (list "blockage" (car (errset (getSGq cv blockages))))
                  (list "row" (car (errset (getSGq cv rows))))
                  (list "marker" (car (errset (getSGq cv markers))))
                  (list "group" (car (errset (getSGq cv groups))))
                  (list "label" (setof s (getSGq cv shapes) (equal (getSGq s objType) "label")))
                  (list "shapeprop" (setof s (getSGq cv shapes) (getSGq s prop))))
      (SfyLayoutPut p (list "oa" (car kv) (length (cadr kv)))))
    (SfyLayoutPut p (list "end"))
    (close p)
    (dbClose cv)
    ;; XST_AUTO_LM writes the map it used to ./xStrmOut_layerMap.txt.
    (SfyLayoutSh "strmout"
      (sprintf nil "strmout -library %s -topCell %s -view %s -strmFile %s/layout.gds -layerMap XST_AUTO_LM -ignoreCurrentTimestamp -convertPin geometryAndText -pinAttNum 100 -logFile %s/strmout.log -summaryFile %s/strmout.sum >/dev/null 2>&1 && mv xStrmOut_layerMap.txt %s/layermap"
        (SfyLayoutQ lib) (SfyLayoutQ cell) (SfyLayoutQ view) q q q q))
    (SfyLayoutSh "copy of the view"
      (sprintf nil "cd %s && for f in *; do case $f in *cdslck*) ;; *) cp -p \"$f\" %s/oa/ ;; esac; done"
        (SfyLayoutQ vdir) q))
    dir))

;; (SfyLayoutImport dir @key (overwrite nil)): applies the import bundle in dir to its view
;; and returns restore, strmin, or unchanged (a restore onto the view as exported). A view
;; open for editing (its directory holds a lock file) is refused, and so is one changed in
;; Virtuoso since the export (its files are not those of oa.sha256) unless overwrite.
;; Restore replaces the view directory's files and refreshes the session's open cellviews
;; of it (dbRefreshCellView); strmin deletes the view, purging its open cellviews, then
;; streams the GDS in, keeping the library's existing cells below the top (noOverwrite).
(procedure (SfyLayoutImport dir @key (overwrite nil))
  (let ((recs (SfyLayoutRecords (strcat dir "/layout.sfyx"))) (q (SfyLayoutQ dir))
        lay mode lib cell view vdir qv exists same)
    (setq lay (cdr (assoc "layout" recs)))
    (setq mode (cadr (assoc "mode" recs)))
    (unless (and lay (member mode (list "restore" "strmin")))
      (error "schemify: %s is not a layout import bundle" dir))
    (setq lib (nth 0 lay))
    (setq cell (nth 1 lay))
    (setq view (nth 2 lay))
    (setq vdir (SfyLayoutDir lib cell view))
    (setq qv (SfyLayoutQ vdir))
    (when (sh (sprintf nil "ls -a %s 2>/dev/null | grep -q cdslck" qv))
      (error "schemify: %s/%s/%s is open for editing (a lock file in %s): close it first" lib cell view vdir))
    (setq exists (and (ddGetObj lib cell view) t))
    (setq same (and exists
      (sh (sprintf nil "cd %s && sha256sum --quiet --strict -c %s/oa.sha256 >/dev/null 2>&1" qv q))))
    (cond
      ((and same (equal mode "restore")) 'unchanged)
      ((and exists (not same) (not overwrite))
        (error "schemify: %s/%s/%s changed in Virtuoso since the export: ?overwrite t replaces it" lib cell view))
      ((equal mode "restore")
        (SfyLayoutSh "restore" (sprintf nil "mkdir -p %s && cd %s && rm -f ./* && cp -p %s/oa/* ." qv qv q))
        (foreach cv (SfyLayoutOpen lib cell view) (dbRefreshCellView cv))
        'restore)
      (t
        (foreach cv (SfyLayoutOpen lib cell view) (dbPurge cv))
        ;; The view's files wait in dir/prev until strmin has written its replacement.
        (when exists
          (SfyLayoutSh "copy of the view" (sprintf nil "rm -rf %s/prev && cp -pr %s %s/prev" q qv q))
          (unless (ddDeleteObj (ddGetObj lib cell view))
            (error "schemify: cannot delete %s/%s/%s" lib cell view)))
        (unless (sh (sprintf nil "strmin -library %s -strmFile %s/layout.gds -topCell %s -view %s -layerMap %s/layermap -pinAttNum 100 -writeMode noOverwrite -logFile %s/strmin.log -summaryFile %s/strmin.sum >/dev/null 2>&1"
                      (SfyLayoutQ lib) q (SfyLayoutQ cell) (SfyLayoutQ view) q q q))
          (when exists (sh (sprintf nil "mkdir -p %s && cp -p %s/prev/* %s/" qv q qv)))
          (error "schemify: strmin failed (%s/strmin.log)%s" dir (if exists "; the view's files are back" "")))
        'strmin))))
