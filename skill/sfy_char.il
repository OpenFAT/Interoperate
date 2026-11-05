;; sfy_char.il: SfyCharacterize, which finds which netlisted params a PDK device's CDF
;; callbacks derive from the parameters a user sets, and measures their values at given
;; sizes (spec interop.md 8.3). The callbacks are SKILL, often encrypted, and only Virtuoso
;; runs them: Schemify keeps their values (cached per instance) and asks here again when
;; an input changes. Everything happens on a scratch schematic that is never saved, and is
;; written as .sfyx records:
;;   ("char" lib cell (input ...) (netlisted ...) (kind per input) (default per input))
;;   ("probe" candidate kind set-text status (netlisted params it changed ...))
;;   ("pt" (set value per input, "" at its default) status (value per netlisted)
;;         (value read back per input))
;; Nothing here knows a device type or a parameter name: the inputs are found by
;; experiment (a candidate is an input when nudging it changes another netlisted param),
;; or named with ?inputs. Kinds: "num", "count", "bool", "choice". Prefix form only, every
;; Cadence call inside errset.

;; A CDF number string ("80n", "1.5u", "2") as a float; nil when it is not one.
(procedure (SfyCharNum s)
  (let ((n (and (stringp s) (strlen s))) (scale 1.0) (c nil) (h nil))
    (cond
      ((numberp s) (float s))
      ((and n (greaterp n 0))
        (setq c (substring s n 1))
        (setq h (substring s 1 1))
        (setq scale (case c ("f" 1e-15) ("p" 1e-12) ("n" 1e-9) ("u" 1e-6) ("m" 1e-3)
                            ("k" 1e3) ("K" 1e3) ("M" 1e6) ("G" 1e9) (t 1.0)))
        (and (or (SfyCharDigit h) (member h (list "-" "+" ".")))
             (if (equal scale 1.0)
               (and (or (SfyCharDigit c) (equal c ".")) (atof s))
               (and (greaterp n 1) (times (atof (substring s 1 (sub1 n))) scale)))))
      (t nil))))

;; True when c is one decimal digit (SKILL has no digit predicate).
(procedure (SfyCharDigit c)
  (and (member c (list "0" "1" "2" "3" "4" "5" "6" "7" "8" "9")) t))

;; True when string s is a whole number written with digits only.
(procedure (SfyCharWhole s)
  (and (stringp s) (greaterp (strlen s) 0)
       (null (setof c (parseString s "") (not (SfyCharDigit c))))))

;; The candidate p as (name kind default choices), or nil when it is not numeric, boolean
;; or a choice: an int is a "count", a string or float with a numeric default a "num" (a
;; "count" when written as a whole number), a boolean a "bool", a cyclic or radio a "choice".
(procedure (SfyCharCand p)
  (let ((ty (getSGq p paramType)) (d (getSGq p defValue)))
    (cond
      ((equal ty "int") (list (getSGq p name) "count" (if (fixp d) d (atoi (SfyAtomStr d))) nil))
      ((equal ty "boolean") (list (getSGq p name) "bool" (if (member d (list t "t" "TRUE" "true")) "t" "nil") nil))
      ((member ty (list "cyclic" "radio"))
        (and (getSGq p choices) (list (getSGq p name) "choice" (SfyAtomStr d) (mapcar 'SfyAtomStr (getSGq p choices)))))
      ((SfyCharWhole (SfyAtomStr d)) (list (getSGq p name) "count" (atoi (SfyAtomStr d)) nil))
      ((SfyCharNum (SfyAtomStr d)) (list (getSGq p name) "num" (SfyCharNum (SfyAtomStr d)) nil))
      (t nil))))

;; The text an input i (name kind default choices) is set to for value v: a string as it
;; is (a value Schemify sends, or a boolean's "t"/"nil"; nil itself means "at its
;; default"), a number printed for its kind.
(procedure (SfyCharText i v)
  (cond
    ((stringp v) v)
    ((equal (cadr i) "count") (sprintf nil "%d" (round v)))
    ((equal (cadr i) "num") (sprintf nil "%.9g" v))
    (t (SfyAtomStr v))))

;; The nudged values of input i, each from the defaults: a number at twice, half and ten
;; times its default (small ones for zero), a count one, two and seven more, the other
;; boolean, every other choice. Several, so a derived value that steps (a table over the
;; input) moves for at least one.
(procedure (SfyCharNudges i)
  (let ((d (caddr i)) (cs (cadddr i)))
    (case (cadr i)
      ("num" (if (zerop d) (list 1e-6 1e-5) (list (times 2 d) (times 0.5 d) (times 10 d))))
      ("count" (list (add1 d) (plus d 2) (plus d 7)))
      ("bool" (list (if (equal d "t") "nil" "t")))
      (t (or (setof c cs (nequal c d)) (list (car cs)))))))

;; Measures one point: a fresh instance of master m in cv, vals (one per input of ins, nil:
;; its default) set in input order with the callbacks running as SfyImport runs them,
;; every netlisted param and every input read back, the instance deleted. Returns
;; (set-texts status values readbacks).
(procedure (SfyCharMeasure cv m ins outs vals)
  (let ((inst nil) (row nil) (set nil) (status nil) (icdf nil) (p nil) (got nil) (back nil) (v vals))
    (foreach i ins
      (when (car v) (setq row (cons (list (car i) (SfyCharText i (car v))) row)))
      (setq set (cons (if (car v) (SfyCharText i (car v)) "") set))
      (setq v (cdr v)))
    (setq row (reverse row))
    (setq inst (car (errset (schCreateInst cv m "SFYCHAR" (list 0.0 0.0) "R0") t)))
    (setq status (if inst (car (errset (SfySetParams inst row t) t)) "no instance"))
    (setq icdf (and inst (car (errset (cdfGetInstCDF inst) t))))
    (foreach n outs
      (setq p (and icdf (cdfFindParamByName icdf n)))
      (setq got (cons (if p (SfyAtomStr (getSGq p value)) "") got)))
    (foreach i ins
      (setq p (and icdf (cdfFindParamByName icdf (car i))))
      (setq back (cons (if p (SfyAtomStr (getSGq p value)) "") back)))
    (when inst (errset (dbDeleteObject inst) t))
    (list (reverse set) (if (equal status t) "ok" (sprintf nil "%L" status)) (reverse got) (reverse back))))

;; Measures and writes one point; t when every callback ran.
(procedure (SfyCharPoint port cv m ins outs vals)
  (let ((r (SfyCharMeasure cv m ins outs vals)))
    (SfyPut port "pt" r)
    (equal (cadr r) "ok")))

;; The inputs of a cell: the candidates (every CDF param with a callback, and every
;; netlisted param) whose nudges, from the defaults, change some other netlisted param.
;; Returns (inputs probes how seeds), one probe per candidate (the names any of its nudges
;; changed, no values; the first nudge's text; ok when one nudge's callbacks all ran).
;; With names (?inputs) those are the inputs and nothing is probed.
(procedure (SfyCharDiscover cv m cdf outs names)
  (let ((cands nil) (c nil) (base nil) (r nil) (changed nil) (status nil) (set nil)
        (probes nil) (took nil) (ctx nil) (vals nil) (base2 nil))
    (foreach q (getSGq cdf parameters)
      (when (or (and (stringp (getSGq q callback)) (greaterp (strlen (getSGq q callback)) 0))
                (member (getSGq q name) outs) (member (getSGq q name) names))
        (setq c (SfyCharCand q))
        (when c (setq cands (cons c cands)))))
    (setq cands (reverse cands))
    (cond
      (names
        (let ((ins (setof c cands (member (car c) names))))
          (list ins nil "named" (length ins))))
      (t
        (setq base (caddr (SfyCharMeasure cv m nil outs nil)))
        (foreach c cands
          (setq changed nil)
          (setq status nil)
          (setq set nil)
          (setq took nil)
          (foreach v (SfyCharNudges c)
            (setq r (SfyCharMeasure cv m (list c) outs (list v)))
            (unless set (setq set (car (car r))))
            (when (or (null status) (equal (cadr r) "ok")) (setq status (cadr r)))
            (when (equal (cadr r) "ok")
              (when (SfyCharTook c r) (setq took t))
              (setq changed (SfyCharMoved c nil outs (caddr r) base changed))))
          ;; Every nudge put back by the callbacks (a finger count the default width
          ;; cannot split: live GF22FDX nfet, nf 2 at w 80n reads back 1; gaps2 P2):
          ;; again with the other declared numbers at ten times their defaults.
          (setq ctx (setof x cands (and (nequal x c) (equal (cadr x) "num")
                                        (member (car x) (SfyCharLabels cdf)))))
          (when (and (not took) (null changed) ctx)
            (setq vals (mapcar (lambda (x) (times 10 (caddr x))) ctx))
            (setq base2 (SfyCharMeasure cv m ctx outs vals))
            (when (equal (cadr base2) "ok")
              (foreach v (SfyCharNudges c)
                (setq r (SfyCharMeasure cv m (append ctx (list c)) outs (append vals (list v))))
                (when (equal (cadr r) "ok")
                  (setq status "ok")
                  (setq changed (SfyCharMoved c ctx outs (caddr r) (caddr base2) changed))))))
          ;; In netlisted order, as one nudge would list them.
          (setq changed (setof n outs (member n changed)))
          (setq probes (cons (list (car c) (cadr c) set status changed) probes)))
        (SfyCharPick cands (reverse probes) (SfyCharLabels cdf))))))

;; changed with the netlisted params (outs) whose values (got) differ from base, other
;; than c and the inputs ctx set.
(procedure (SfyCharMoved c ctx outs got base changed)
  (let ((k 0))
    (foreach n outs
      (when (and (nequal n (car c)) (not (assoc n ctx)) (nequal (nth k got) (nth k base))
                 (not (member n changed)))
        (setq changed (cons n changed)))
      (setq k (add1 k)))
    changed))

;; True when measurement r kept candidate c off its default: the value read back (r's
;; only input) is not the default's.
(procedure (SfyCharTook c r)
  (let ((b (car (cadddr r))) (d (SfyCharText c (caddr c))))
    (if (and (SfyCharNum b) (SfyCharNum d))
      (greaterp (abs (difference (SfyCharNum b) (SfyCharNum d)))
                (times 1e-6 (abs (SfyCharNum d))))
      (nequal b d))))

;; The CDF's paramLabelSet names (a leading "-" stripped): the PDK's own declared sizing
;; parameters.
(procedure (SfyCharLabels cdf)
  (let ((s (getSGq cdf paramLabelSet)))
    (and (stringp s)
         (mapcar (lambda (w) (if (equal (substring w 1 1) "-") (substring w 2) w))
                 (parseString s " ")))))

;; The names reachable from x over edges, a list of (name (names it moves)).
(procedure (SfyCharReach x edges)
  (let ((seen nil) (todo (list x)) (n nil) (e nil))
    (while todo
      (setq n (car todo))
      (setq todo (cdr todo))
      (setq e (assoc n edges))
      (foreach y (and e (cadr e))
        (unless (member y seen)
          (setq seen (cons y seen))
          (setq todo (cons y todo)))))
    seen))

;; The inputs, read from the probes as an influence graph (an edge x -> y when nudging x
;; changes netlisted y, its callbacks all running). Seeds: the paramLabelSet params with
;; an edge; then every other candidate with an edge that no seed moves (a param a seed
;; moves is derived, even when it is editable). Without a useful label set: the sources of
;; the graph (no edge into the strongly connected component from outside), at most 6.
;; Returns (inputs probes how seeds): the seeds first, then the others (CDF order each);
;; how "labels" or "sources"; seeds the count of seeds (every input for sources).
(procedure (SfyCharPick cands probes labels)
  (let ((edges nil) (seeds nil) (moved nil) (ins nil) (rx nil) (n 0))
    (foreach pr probes
      (when (and (car (last pr)) (equal (nth 3 pr) "ok"))
        (setq edges (cons (list (car pr) (car (last pr))) edges))))
    (setq seeds (setof c cands (and (member (car c) labels) (assoc (car c) edges))))
    (cond
      (seeds
        (foreach s seeds
          (foreach y (cadr (assoc (car s) edges))
            (unless (member y moved) (setq moved (cons y moved)))))
        (list (append seeds
                      (setof c cands (and (not (member c seeds)) (assoc (car c) edges)
                                          (not (member (car c) moved)))))
              probes "labels" (length seeds)))
      (t
        (foreach c cands
          (when (and (assoc (car c) edges) (lessp n 6))
            (setq rx (SfyCharReach (car c) edges))
            ;; A source: whatever reaches c, c reaches back (one component).
            (unless (setof e edges (and (nequal (car e) (car c))
                                        (member (car c) (SfyCharReach (car e) edges))
                                        (not (member (car e) rx))))
              (setq ins (cons c ins))
              (setq n (add1 n)))))
        (list (reverse ins) probes "sources" n)))))

;; Characterizes lib/cell into port (spec 8.3): its inputs and what each one moves, then
;; one point per entry of points, each a list of (input value-text) pairs (an input left
;; out is at its default). t when the cell had a CDF to read.
(procedure (SfyCharCell port cv lib cell names points)
  (let ((m nil) (cdf nil) (sp nil) (outs nil) (found nil) (ins nil))
    (setq m (car (errset (dbOpenCellViewByType lib cell "symbol" nil "r") t)))
    (setq cdf (car (errset (cdfGetCellCDF (ddGetObj lib cell)) t)))
    (setq sp (and cdf (cadr (assoc "spectre" (SfySimInfoFields (getSGq cdf simInfo))))))
    ;; The netlisted params by their CDF names, through propMapping (a kit resistor
    ;; netlists r from its CDF value: live GF22FDX opnpcres, gaps2 P10).
    (setq outs (mapcar (lambda (k) (or (cadr (assoc k (cadr (assoc "propMapping" sp)))) k))
                       (cadr (assoc "instParameters" sp))))
    (setq found (and m cdf outs (SfyCharDiscover cv m cdf outs names)))
    (setq ins (car found))
    (when (equal (caddr found) "sources")
      (SfyLog (sprintf nil "info %s/%s: no paramLabelSet param moves another; inputs are the influence graph's sources: %L"
                       lib cell (mapcar 'car ins))))
    (cond
      ((not (and m cdf outs))
        (SfyRefuse 0 (sprintf nil "%s/%s: no symbol, CDF or spectre instParameters to characterize" lib cell))
        nil)
      (t
        (SfyPut port "char"
          (list lib cell (mapcar 'car ins) outs (mapcar 'cadr ins)
                (mapcar (lambda (i) (SfyCharText i (caddr i))) ins)))
        (foreach pr (cadr found) (SfyPut port "probe" pr))
        (foreach pt (and ins points)
          (SfyCharPoint port cv m ins outs
                        (mapcar (lambda (i) (cadr (assoc (car i) pt))) ins)))
        (SfyCount "characterized")
        t))))

;; SfyCharacterize: characterizes cells (a list of names) of PDK library lib into path (an
;; .sfyx file; it names the PDK's parameters and values, so it stays on the machine it was
;; made on). scratch is a writable library for the throwaway schematic. ?inputs names the
;; inputs instead of discovering them; ?points, lists of (input value-text) pairs, measures
;; each (what schemify-interop virtuoso-refresh asks for). Writes <path>.log; t on PASS.
(procedure (SfyCharacterize path lib cells scratch @key (inputs nil) (points nil))
  (let ((sfyLogPort (SfyLogOpen path)) (sfyCountTab (makeTable "sfyCounts" nil))
        (sfyCountKeys nil) (sfyFails 0) (port nil) (cv nil))
    (SfyLog (sprintf nil "SfyCharacterize %s library %s cells %L" path lib cells))
    (setq cv (car (errset (dbOpenCellViewByType scratch "sfy_char" "schematic" "schematic" "w") t)))
    (setq port (and cv (outfile path "w")))
    (cond
      ((null (ddGetObj lib)) (SfyRefuse 0 (sprintf nil "library %s is not in cds.lib" lib)))
      ((null cv) (SfyRefuse 0 (sprintf nil "cannot open %s/sfy_char/schematic for writing" scratch)))
      ((null port) (SfyRefuse 0 (sprintf nil "cannot write %s" path)))
      (t
        (unwindProtect
          (progn
            (SfyPut port "sfyx" (list 1 (SfyProducer)))
            (foreach c cells
              (unless (errset (SfyCharCell port cv lib c inputs points) t)
                (SfyRefuse 0 (sprintf nil "%s/%s: SKILL error %s" lib c (SfyErrMsg)))))
            (SfyPut port "end" nil))
          (progn
            (close port)
            (errset (dbPurge cv) t)))))
    (SfyLogClose)))
