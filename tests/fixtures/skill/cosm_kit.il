;; <lib>/kit: a cell with every cosmetic a redraw must keep (gap D, tests/virtuoso_cosmetics.rs;
;; live/checks/l2_labels.il draws it in a real Virtuoso). Loaded after build.il and
;; (SfyFixStdlib); coordinates in database units as there.

(procedure (SfyFixKit @optional (lib "work"))
  (let ((cv nil) (net nil) (term nil) (fig nil) (l nil) (w nil))
    ;; The symbol: an overbar pin label, an NLP name label, an undrafted roman note.
    (setq cv (SfyFixOpen lib "kit" "symbol" "schematicSymbol"))
    (dbCreateRect cv (list "device" "drawing") (SfyFixBox -20 -20 20 20))
    (dbCreateRect cv (list "instance" "drawing") (SfyFixBox -40 -40 40 40))
    (foreach p (list (list "A" "input" -40) (list "Y" "output" 40))
      (setq net (dbMakeNet cv (car p)))
      (setq term (dbCreateTerm net (car p) (cadr p)))
      (setq fig (dbCreateRect cv (list "pin" "drawing")
                  (SfyFixBox (difference (caddr p) 2) -2 (plus (caddr p) 2) 2)))
      (dbCreatePin net fig nil term)
      (setq l (dbCreateLabel cv (list "pin" "label") (SfyFixP (caddr p) 0) (car p)
                "centerLeft" "R0" "stick" (SfyFixU 10)))
      (schGlueLabel l fig)
      (when (equal (car p) "A") (setSGq l t isOverbar)))
    (setq l (dbCreateLabel cv (list "instance" "label") (SfyFixP 0 30) "[@instanceName]"
              "lowerCenter" "R0" "stick" (SfyFixU 10)))
    (setSGq l "NLPLabel" labelType)
    (setq l (dbCreateLabel cv (list "annotate" "drawing") (SfyFixP 0 -30) "note"
              "lowerLeft" "R0" "roman" (SfyFixU 8)))
    (setSGq l nil isDrafted)
    (SfyFixSave cv)
    ;; The schematic.
    (setq cv (SfyFixOpen lib "kit" "schematic" "schematic"))
    (schCreateInst cv (dbOpenCellViewByType "analogLib" "res" "symbol" nil "r") "R0" (SfyFixP 0 0) "R0")
    ;; A named, rotated noConn.
    (schCreateInst cv (dbOpenCellViewByType "basic" "noConn" "symbol" nil "r") "nc1"
      (SfyFixP 200 -100) "R90")
    (schCreatePin cv (dbOpenCellViewByType "basic" "ipin" "symbol" nil "r") "A" "input" nil
      (SfyFixP -200 0) "R0")
    (schCreatePin cv (dbOpenCellViewByType "basic" "opin" "symbol" nil "r") "Y" "output" nil
      (SfyFixP 200 0) "R0")
    ;; A bus 4 DBU wide (export's is 6), its labels drawn off the wire.
    (setq w (car (schCreateWire cv "draw" "full" (list (SfyFixP -100 200) (SfyFixP 100 200))
                   0.0625 0.0625 (SfyFixU 4))))
    (schCreateWireLabel cv w (SfyFixP -60 210) "d<1:0>" "lowerLeft" "R0" "stick" (SfyFixU 10) nil)
    (schCreateWireLabel cv w (SfyFixP 40 220) "d<1:0>" "lowerLeft" "R0" "roman" (SfyFixU 12) nil)
    ;; A pin-annotate label (Virtuoso leaves it unglued, live ICADVM20.1), an overbar wire label.
    (setq w (car (schCreateWire cv "draw" "full" (list (SfyFixP 200 0) (SfyFixP 200 -100))
                   0.0625 0.0625 0.0)))
    (setq l (dbCreateLabel cv (list "pin" "annotate") (SfyFixP 210 -50) "Y" "centerLeft" "R90"
              "stick" (SfyFixU 8)))
    (setq l (schCreateWireLabel cv w (SfyFixP 200 -80) "Y" "lowerLeft" "R0" "stick" (SfyFixU 10) nil))
    (setSGq l t isOverbar)
    ;; Note labels with each flag.
    (setq l (schCreateNoteLabel cv (SfyFixP 0 -200) "OVR" "lowerLeft" "R0" "stick" (SfyFixU 10) "normalLabel"))
    (setSGq l t isOverbar)
    (setq l (schCreateNoteLabel cv (SfyFixP 0 -240) "DRF" "lowerLeft" "R0" "stick" (SfyFixU 10) "normalLabel"))
    (setSGq l nil isDrafted)
    (setq l (schCreateNoteLabel cv (SfyFixP 0 -280) "INV" "lowerLeft" "R0" "stick" (SfyFixU 10) "normalLabel"))
    (setSGq l nil isVisible)
    ;; Note shapes: a wide path and an elliptic ellipse.
    (dbCreatePath cv (list "text" "drawing") (list (SfyFixP 300 0) (SfyFixP 340 0) (SfyFixP 340 40))
      (SfyFixU 2))
    (dbCreateEllipse cv (list "text" "drawing") (SfyFixBox 300 100 380 140))
    (schCheck cv)
    (SfyFixSave cv)))
