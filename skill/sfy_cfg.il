;; sfy_cfg.il: Virtuoso config views (hierarchy editor, hdb*) to and from .sfyx config
;; records (interop spec 4.2, 9.3). Entry points called by sfy_cells.il: SfyCfgDump (per
;; design cell during SfyExport) and SfyCfgLoad (per config record during SfyImport, after
;; the schematics and before maestro, whose tests may run the config).
;;
;; SKILL has no hdb getters for a config's lists and rules (G34), and hdb has no setters for
;; comments, notes or const and rewrites the rules in its own order, so the expand.cfg text is
;; what crosses: the dumper reads it, the loader lets hdb create the view (master.tag, the
;; top cellview) and then writes the dumped text over the expand.cfg hdbSave wrote.

;; ("config" view text) per view of the cell that holds an expand.cfg, by view name.
(procedure (SfyCfgDump port lib cell)
  (let ((f nil) (p nil) (segs nil) (line nil))
    (foreach v (sort (getSGq (getSGq (ddGetObj lib cell) views) name) nil)
      (when (setq f (ddGetObj lib cell v "expand.cfg"))
        (setq p (infile (getSGq f readPath)))
        (if (null p)
          (SfyRefuse 0 (sprintf nil "cannot read %s/%s/%s/expand.cfg" lib cell v))
          (progn
            (setq segs nil)
            (while (gets line p) (setq segs (cons line segs)))
            (close p)
            (SfyPut port "config" (list v (SfyText (reverse segs))))))))))

;; ("config" view text) of design cell c: hdbOpen "w" with the cell's schematic as top,
;; hdbSave, hdbClose, the dumped text written over expand.cfg, then hdbOpen "r" must read it
;; back, or the view is refused. An existing view is refused unless ?overwrite.
(procedure (SfyCfgLoad c lr)
  (let ((tlib (arrayref c "lib")) (cell (arrayref c "name")) (view (nth 1 (cadr lr)))
        (text (nth 2 (cadr lr))) (h nil) (ok nil) (id nil) (p nil))
    (cond
      ((and (ddGetObj tlib cell view) (not sfyOverwrite))
        (SfyRefuse (car lr) (sprintf nil "%s/%s/%s exists; pass ?overwrite t" tlib cell view)))
      ((null (setq h (car (errset (hdbOpen tlib cell view "w")))))
        (SfyRefuse (car lr) (sprintf nil "hdbOpen %s/%s/%s \"w\" failed" tlib cell view)))
      ((progn
         (setq ok (and (hdbSetTopCellViewName h tlib cell "schematic") (hdbSave h)))
         (hdbClose h)
         (not ok))
        (SfyRefuse (car lr) (sprintf nil "hdb could not save %s/%s/%s" tlib cell view)))
      ((null (setq id (ddGetObj tlib cell view "expand.cfg" nil "a")))
        (SfyRefuse (car lr) (sprintf nil "no expand.cfg in %s/%s/%s" tlib cell view)))
      ((null (setq p (outfile (ddGetObjWritePath id) "w")))
        (SfyRefuse (car lr) (sprintf nil "cannot write %s" (ddGetObjWritePath id))))
      (t
        (foreach chunk (if (stringp text) (list text) text)
          (fprintf p "%s" chunk))
        (close p)
        (setq h (car (errset (hdbOpen tlib cell view "r"))))
        (if (null h)
          (SfyRefuse (car lr) (sprintf nil "hdb cannot read the expand.cfg written to %s/%s/%s" tlib cell view))
          (progn
            (hdbClose h)
            (SfyCount "config")))))))
