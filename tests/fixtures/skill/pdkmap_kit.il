;; pdkmap_kit.il: a made-up kit for SfyCdfNames and pdk-map (tests/pdk_map.rs; the live
;; check live/checks/pdkmap_names.il), drawn with build.il's helpers. In library lib:
;; - nfoo: model nfoo, spectre termOrder (D G S B), a 3-terminal symbol (D G S);
;; - nfoo_b: model nfoo, termOrder (D G S B), a 4-terminal symbol;
;; - pbar: model pbar, termOrder (D G S), where the scan has 4 pins;
;; - rbaz: model rbaz, termOrder (PLUS MINUS);
;; - nocdf: a symbol and no CDF.

;; A device cell: a symbol of pins (name dir x y), a base CDF with a model parameter
;; (default model) and the string parameters params, and a spectre simInfo termOrder order.
(procedure (SfyFixMapDev lib cell model order pins params)
  (let ((cdf nil))
    (SfyFixSymbol lib cell pins)
    (setq cdf (SfyFixCdf lib cell))
    (cdfCreateParam cdf ?name "model" ?type "string" ?defValue model ?prompt "Model")
    (foreach p params (cdfCreateParam cdf ?name p ?type "string" ?defValue "1" ?prompt p))
    (setSGq cdf (list nil 'spectre (list nil 'termOrder order 'otherParameters '(model)))
      simInfo)
    (cdfSaveCDF cdf)))

(procedure (SfyFixPdkMapKit lib)
  (let ((d (list "D" "inputOutput" 0 40)) (g (list "G" "input" -40 0))
        (s (list "S" "inputOutput" 0 -40)) (b (list "B" "inputOutput" 40 0)))
    (SfyFixMapDev lib "nfoo" "nfoo" (list "D" "G" "S" "B") (list d g s)
      (list "w" "l" "nf" "m"))
    (SfyFixMapDev lib "nfoo_b" "nfoo" (list "D" "G" "S" "B") (list d g s b)
      (list "w" "l" "nf" "m"))
    (SfyFixMapDev lib "pbar" "pbar" (list "D" "G" "S") (list d g s) (list "w" "l"))
    (SfyFixRbaz lib)
    (SfyFixSymbol lib "nocdf" (list (list "A" "inputOutput" 0 40)
                                (list "B" "inputOutput" 0 -40)))
    t))

(procedure (SfyFixRbaz lib)
  (SfyFixMapDev lib "rbaz" "rbaz" (list "PLUS" "MINUS")
    (list (list "PLUS" "inputOutput" 0 40) (list "MINUS" "inputOutput" 0 -40))
    (list "r" "m")))
