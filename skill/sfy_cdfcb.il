;; sfy_cdfcb.il: design-library CDF callbacks, read for Schemify computed params (interop
;; spec 4.2 param, 6.3). The dump gives a design cell's param record two more fields: where
;; its callback's code was read and that code as list text, which the Rust side translates
;; when it only assigns arithmetic over other parameters. Nothing here runs a callback.
;;
;; Only plain SKILL text of the design library itself is ever read: the callback string,
;; and for a callback that is one call of a procedure without arguments, that procedure's
;; definition in the library's libInit.il (the SKILL file Virtuoso loads with the library).
;; A reference or PDK library is never read (SfyDumpCdf asks only for design cells), nor a
;; compiled or encrypted file (libInit.cxt, .ile): a procedure defined anywhere else has no
;; readable source here and stays Virtuoso's.

;; (from code) for a design cell's parameter p of library lib: from "inline" with the
;; callback string's forms, or "libInit.il" with the procedure form; nil without a
;; callback or readable code.
(procedure (SfyCdfCbFields lib p)
  (let ((cb (getSGq p callback)) (forms nil) (def nil))
    (when (and (stringp cb) (nequal cb ""))
      ;; linereadstring reads every form of the string's first line (live ICADVM20.1).
      (setq forms (car (errset (linereadstring cb) nil)))
      (cond
        ((not (listp forms)) nil)
        ((and forms (null (cdr forms)) (listp (car forms)) (symbolp (caar forms))
              (null (cdar forms)))
          (setq def (SfyLibProc lib (caar forms)))
          (and def (list "libInit.il" (SfyText (list (SfyCbForm def))))))
        (forms (list "inline" (SfyText (list (SfyCbForm forms)))))))))

;; The form (procedure (name ...) ...) defining name in lib's libInit.il, or nil.
(procedure (SfyLibProc lib name)
  (let ((dir (getSGq (ddGetObj lib) readPath)) (port nil) (line nil) (out nil))
    (when (stringp dir)
      (setq port (infile (strcat dir "/libInit.il"))))
    (when port
      (while (and (null out) (setq line (car (errset (lineread port) nil))))
        (when (listp line)
          (foreach f line
            (when (and (null out) (listp f) (eq (car f) 'procedure) (listp (cadr f))
                       (eq (car (cadr f)) name))
              (setq out f)))))
      (close port))
    out))

;; A form as list text: lists in parentheses, strings quoted, integers with %d, a float x
;; as (float x) so that 2.0 stays a float, symbols by name. (%L prints SKILL's infix back,
;; live ICADVM20.1: (a - b).)
(procedure (SfyCbForm x)
  (cond
    ((null x) "nil")
    ((listp x) (strcat "(" (buildString (mapcar 'SfyCbForm x) " ") ")"))
    ((stringp x) (strcat "\"" x "\""))
    ((fixp x) (sprintf nil "%d" x))
    ((floatp x) (sprintf nil "(float %.17g)" x))
    ((eq x t) "t")
    ((symbolp x) (symbolToString x))
    (t (sprintf nil "%L" x))))

;; ---------------------------------------------------------------- textDisplays

;; td records (spec 4.2) of the textDisplays that instances own, sorted: ("td" owner assoc
;; name layer purpose x y orient justify font height drafted overbar visible nameVisible
;; valueVisible). assoc is label (name: the text of the master's label it shows), prop
;; (the instance property's name) or attr (the attribute's name); the flags are 0/1. Any
;; other textDisplay is counted skipped. xy and orient are where Virtuoso shows it.
(procedure (SfyDumpTds port tds refs upi)
  (let ((entries nil) (o nil) (k nil) (b nil))
    (foreach f tds
      (setq o (getSGq f owner))
      (setq k (case (getSGq f assocType)
                ("labelAssocType" "label") ("propAssocType" "prop") ("attrAssocType" "attr")
                (t nil)))
      (if (and k o (equal (getSGq o objType) "inst") (arrayref refs o))
        (progn
          (setq b (append
                    (list (arrayref refs o) k (getSGq f text) (getSGq f layerName)
                      (getSGq f purpose) (SfyDbu (xCoord (getSGq f xy)) upi)
                      (SfyDbu (yCoord (getSGq f xy)) upi) (getSGq f orient) (getSGq f justify)
                      (getSGq f font) (SfyDbu (getSGq f height) upi))
                    (mapcar (lambda (x) (if x 1 0))
                      (list (getSGq f isDrafted) (getSGq f isOverbar) (getSGq f isVisible)
                        (getSGq f isNameVisible) (getSGq f isValueVisible)))))
          (setq entries (cons (list b f b) entries)))
        (SfySkip "textDisplay")))
    (foreach e (SfySortByKey entries)
      (SfyPut port "td" (caddr e)))))

;; A td record back: the textDisplay of its owner instance showing that master label,
;; property or attribute, then placed where the record says (a label's textDisplay takes
;; its point in the master's coordinates, live ICADVM20.1, so xy and orient are set after).
;; Run after the properties are set, as a prop textDisplay needs its property.
(procedure (SfyLoadTd sfyLr)
  (let ((sfyF (cdr (cadr sfyLr))) (sfyO nil) (sfyA nil) (sfyN nil) (sfyPt nil) (sfyTd nil))
    (setq sfyO (SfyRef (nth 0 sfyF) sfyLr))
    (setq sfyN (nth 2 sfyF))
    (setq sfyA
      (and sfyO
        (case (nth 1 sfyF)
          ("label" (car (setof sfyX (getSGq (getSGq sfyO master) shapes)
                          (and (equal (getSGq sfyX objType) "label")
                               (equal (getSGq sfyX theLabel) sfyN)))))
          ("prop" (car (setof sfyX (getSGq sfyO prop) (equal (getSGq sfyX name) sfyN))))
          ("attr" sfyO)
          (t nil))))
    (setq sfyPt (SfyPt (nth 5 sfyF) (nth 6 sfyF)))
    (when sfyA
      (setq sfyTd
        (apply 'dbCreateTextDisplay
          (append
            (list sfyA sfyO (list (nth 3 sfyF) (nth 4 sfyF)) t sfyPt (nth 8 sfyF)
              (nth 7 sfyF) (nth 9 sfyF) (SfyUU (nth 10 sfyF)) (equal (nth 11 sfyF) 1)
              (equal (nth 12 sfyF) 1) (equal (nth 13 sfyF) 1) (equal (nth 14 sfyF) 1)
              (equal (nth 15 sfyF) 1))
            ;; The attribute an attr textDisplay shows.
            (and (equal (nth 1 sfyF) "attr") (list sfyN))))))
    (if sfyTd
      (progn
        (setSGq sfyTd sfyPt xy)
        (setSGq sfyTd (nth 7 sfyF) orient)
        (SfyCount "textDisplay"))
      (SfyCount (sprintf nil "skipped:textDisplay %s %s" (nth 1 sfyF) sfyN)))))
