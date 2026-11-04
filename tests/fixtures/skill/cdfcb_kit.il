;; <lib>/amp and <lib>/top: design-library CDF callbacks, and instance CDF values,
;; properties and textDisplays a redraw must keep (lane L9: tests/virtuoso_cdfcb.rs;
;; live/checks/l9_cdfcb.il draws them in a real Virtuoso). Loaded after build.il and
;; (SfyFixStdlib); coordinates in database units as there.
;;
;; amp's CDF: w and nf call sfyKitWf, a plain assignment (wf = w / nf); l calls sfyKitArea,
;; which branches on nf. Both live in the library's libInit.il, written here.

(procedure (SfyFixCbKit @optional (lib "work"))
  (let ((p nil) (cdf nil) (cv nil) (m nil) (i0 nil) (lab nil))
    (setq p (outfile (strcat (getSGq (ddGetObj lib) readPath) "/libInit.il")))
    (fprintf p "(procedure (sfyKitWf) (putpropq (getq cdfgData wf) (quotient (getq (getq cdfgData w) value) (getq (getq cdfgData nf) value)) value))\n")
    (fprintf p "(procedure (sfyKitArea) (if (greaterp (getq (getq cdfgData nf) value) 1) (putpropq (getq cdfgData area) (times (getq (getq cdfgData w) value) (getq (getq cdfgData l) value)) value) (putpropq (getq cdfgData area) 0.0 value)))\n")
    (close p)
    ;; The symbol, with a [@w] label an instance's textDisplay can show.
    (SfyFixSymbol lib "amp" (list (list "A" "input" -40 0) (list "Y" "output" 40 0)))
    (setq cv (dbOpenCellViewByType lib "amp" "symbol" "schematicSymbol" "a"))
    (setq lab (dbCreateLabel cv (list "annotate" "drawing") (SfyFixP 0 -30) "[@w]"
                "lowerLeft" "R0" "stick" (SfyFixU 8)))
    (setSGq lab "NLPLabel" labelType)
    (SfyFixSave cv)
    (setq cv (SfyFixOpen lib "amp" "schematic" "schematic"))
    (schCreateInst cv (dbOpenCellViewByType "analogLib" "res" "symbol" nil "r") "R0" (SfyFixP 0 0) "R90")
    (schCreatePin cv (dbOpenCellViewByType "basic" "ipin" "symbol" nil "r") "A" "input" nil
      (SfyFixP -100 0) "R0")
    (schCreatePin cv (dbOpenCellViewByType "basic" "opin" "symbol" nil "r") "Y" "output" nil
      (SfyFixP 100 0) "R0")
    (schCheck cv)
    (SfyFixSave cv)
    (setq cdf (SfyFixCdf lib "amp"))
    (cdfCreateParam cdf ?name "w" ?type "float" ?defValue 0.000001 ?prompt "Width"
      ?callback "(sfyKitWf)")
    (cdfCreateParam cdf ?name "l" ?type "float" ?defValue 0.00000018 ?prompt "Length"
      ?callback "(sfyKitArea)")
    (cdfCreateParam cdf ?name "nf" ?type "int" ?defValue 1 ?prompt "Fingers" ?callback "(sfyKitWf)")
    (cdfCreateParam cdf ?name "wf" ?type "float" ?defValue 0.000001 ?prompt "Finger width")
    (cdfCreateParam cdf ?name "area" ?type "float" ?defValue 0.0 ?prompt "Area")
    (setSGq cdf (list nil 'spectre (list nil 'termOrder '(A Y) 'instParameters '(w l nf wf area)))
      simInfo)
    (cdfSaveCDF cdf)
    ;; top: I0 as the Edit Properties form leaves it (w and nf set, wf derived), with
    ;; properties Schemify has no param for and three textDisplays; I1 at the defaults.
    (setq m (dbOpenCellViewByType lib "amp" "symbol" nil "r"))
    (setq cv (SfyFixOpen lib "top" "schematic" "schematic"))
    (setq i0 (schCreateInst cv m "I0" (SfyFixP 0 0) "R0"))
    (schCreateInst cv m "I1" (SfyFixP 200 0) "R90")
    ;; As properties: a write through the instance CDF would leave the values in the cell
    ;; CDF for every instance made later in the session (live ICADVM20.1, mockdb cdf.rs).
    (dbReplaceProp i0 "w" "float" 0.000003)
    (dbReplaceProp i0 "nf" "int" 2)
    (dbReplaceProp i0 "wf" "float" 0.0000015)
    (dbReplaceProp i0 "note" "ILExpr" "1+2")
    (dbReplaceProp i0 "l" "ILExpr" "iPar(\"w\")*2")
    (dbReplaceProp i0 "stamp" "time" "Nov  4 12:00:00 2026")
    (dbReplaceProp i0 "count" "int" 7)
    (dbReplaceProp i0 "tag" "string" "x")
    (setq lab (car (setof x (getSGq m shapes) (equal (getSGq x theLabel) "[@w]"))))
    (dbCreateTextDisplay lab i0 (list "annotate" "drawing") t (SfyFixP 10 30) "lowerLeft" "R0"
      "stick" (SfyFixU 8) t nil t t t)
    (dbCreateTextDisplay (car (setof x (getSGq i0 prop) (equal (getSGq x name) "count"))) i0
      (list "annotate" "drawing") t (SfyFixP 10 -30) "centerLeft" "R0" "roman" (SfyFixU 8)
      t nil t t nil)
    (dbCreateTextDisplay i0 i0 (list "annotate" "drawing") t (SfyFixP 10 50) "centerLeft" "R0"
      "stick" (SfyFixU 8) t nil t nil t "name")
    (schCheck cv)
    (SfyFixSave cv)))
