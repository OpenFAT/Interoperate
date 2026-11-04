;; sfy_pdkmap.il: SfyCdfNames, the names a PDK library's cells netlist by, for
;; `schemify-interop pdk-map` (interop spec 8.2), which maps a kit's scanned cells onto
;; them. Per cell, one record:
;;   ("cdfnames" lib cell model (termOrder ...) (param ...) (terminal ...))
;; model: the base CDF's model parameter default; termOrder: the spectre simInfo's, in
;; order; param: the base CDF's parameter names, in CDF order; terminal: the symbol's
;; terminal names, sorted. A cell without CDF, model parameter, spectre termOrder or symbol
;; gives "" or (). Names only: no default but the model's (itself a name), no geometry.

;; Dumps every cell of each library of libs (in that order, cells sorted) to path. Writes
;; <path>.log; t on PASS.
(procedure (SfyCdfNames path libs)
  (let ((sfyLogPort (SfyLogOpen path)) (sfyCountTab (makeTable "sfyCounts" nil))
        (sfyCountKeys nil) (sfyFails 0) (port nil))
    (SfyLog (sprintf nil "SfyCdfNames %s libraries %L" path libs))
    (SfyPreflight)
    (setq port (outfile path "w"))
    (if (null port)
      (SfyRefuse 0 (sprintf nil "cannot write %s" path))
      (unwindProtect
        (progn
          (SfyPut port "sfyx" (list 1 (SfyProducer)))
          (foreach lib libs
            (if (null (ddGetObj lib))
              (SfyRefuse 0 (sprintf nil "library %s is not in cds.lib" lib))
              (foreach cell (SfyCellNames lib)
                (unless (errset (SfyCdfNamesCell port lib cell) t)
                  (SfyRefuse 0 (sprintf nil "%s/%s: SKILL error %s" lib cell (SfyErrMsg)))))))
          (SfyPut port "end" nil))
        (close port)))
    (SfyLogClose)))

;; The cdfnames record of lib/cell.
(procedure (SfyCdfNamesCell port lib cell)
  (let ((cdf (cdfGetBaseCellCDF (ddGetObj lib cell))) (model nil) (order nil) (cv nil)
        (terms nil))
    (when cdf
      (setq model (cdfFindParamByName cdf "model"))
      (foreach s (SfySimInfoFields (getSGq cdf simInfo))
        (when (equal (car s) "spectre")
          (foreach kv (cadr s)
            (when (equal (car kv) "termOrder") (setq order (cadr kv)))))))
    (when (ddGetObj lib cell "symbol")
      (setq cv (dbOpenCellViewByType lib cell "symbol" nil "r"))
      (when cv
        (setq terms (sort (getSGq (getSGq cv terminals) name) nil))
        (dbClose cv)))
    (SfyPut port "cdfnames"
      (list lib cell (if model (SfyAtomStr (getSGq model defValue)) "")
        (if (listp order) order (list order))
        (and cdf (getSGq (getSGq cdf parameters) name))
        terms))
    (SfyCount "cdfnames")))
