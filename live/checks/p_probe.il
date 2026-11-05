;; p_probe.il: lane P probe (gaps2 P1 S8 S9 P14), read only. Run with a cds.lib that mounts
;; a PDK library besides analogLib; $SFYCHECK_PLIB names it (default cmos22fdsoi). Prints:
;; - per probed cell, the terminals (name, netExpr) of each netlisting view it has (symbol,
;;   spectre, auCdl, hspiceD, auLvs) and the spectre/auCdl termOrder as SKILL holds it;
;; - per library, the cells whose spectre termOrder names a terminal its symbol lacks, and
;;   whether a stopping view of that simulator's name has it (with which netExpr);
;; - per library, every spectre/auCdl/hspiceD/auLvs simInfo key with its cell count.
;; Only observations: chkPass once the sections ran.

procedure(chkPTerms(lib cell view)
  let((cv out)
    cv = dbOpenCellViewByType(lib cell view nil "r")
    when(cv
      out = mapcar(lambda((t0) list(t0~>name dbGetTermNetExpr(t0))) cv~>terminals)
      dbClose(cv))
    out))

procedure(chkPViews(lib cell)
  setof(v list("symbol" "spectre" "auCdl" "hspiceD" "auLvs")
    ddGetObj(lib cell v)))

procedure(chkPSim(lib cell sim)
  let((cdf)
    cdf = cdfGetCellCDF(ddGetObj(lib cell))
    cdf && get(cdf~>simInfo sim)))

;; The keys of a simInfo DPL.
procedure(chkPKeys(si)
  let((out l)
    l = cdr(si)
    while(l
      out = cons(car(l) out)
      l = cddr(l))
    reverse(out)))

procedure(chkPCells(lib)
  let((l)
    l = ddGetObj(lib)
    l && mapcar(lambda((c) c~>name) l~>cells)))

;; A termOrder entry as text (symbols and strings alike; a list in print form).
procedure(chkPName(x)
  cond((stringp(x) x) (symbolp(x) symbolToString(x)) (t sprintf(nil "%L" x))))

;; The cells of lib whose spectre termOrder names a terminal its symbol lacks.
procedure(chkPImplicit(lib)
  let((n sv st extra)
    n = 0
    foreach(c chkPCells(lib)
      sv = chkPSim(lib c "spectre")
      when(and(sv listp(sv->termOrder) ddGetObj(lib c "symbol"))
        st = mapcar(lambda((x) car(x)) chkPTerms(lib c "symbol"))
        extra = setof(x mapcar('chkPName sv->termOrder) !member(x st))
        when(extra
          n = n + 1
          when(n <= 15
            chkLine(sprintf(nil "implicit %s/%s" lib c)
              list(extra st chkPSim(lib c "auCdl") && chkPSim(lib c "auCdl")->termOrder
                and(ddGetObj(lib c "spectre")
                  setof(x chkPTerms(lib c "spectre") member(car(x) extra)))))))))
    chkLine(sprintf(nil "implicit-count %s" lib) n)))

;; Every simInfo key of lib's cells per simulator, with its cell count.
procedure(chkPSimKeys(lib)
  let((cells keys sv)
    cells = chkPCells(lib)
    foreach(s list("spectre" "auCdl" "hspiceD" "auLvs")
      keys = makeTable("k" 0)
      foreach(c cells
        sv = chkPSim(lib c s)
        when(sv foreach(k chkPKeys(sv) keys[k] = keys[k] + 1)))
      chkLine(sprintf(nil "simkeys %s %s" lib s)
        mapcar(lambda((k) list(k keys[k])) keys->?)))))

;; The first cell having each less common spectre key, and how many have it.
procedure(chkPKeyExamples(lib)
  let((n)
    foreach(k list('netlistProcedure 'otherParameters 'stringParameters 'enumParameters
                   'arrayParameters 'macroArguments 'componentName 'opParamExprList)
      n = setof(c chkPCells(lib) chkPSim(lib c "spectre") && get(chkPSim(lib c "spectre") k))
      when(n
        chkLine(sprintf(nil "spectre-%s %s" k lib)
          list(length(n) car(n) get(chkPSim(lib car(n) "spectre") k)))))))

let((plib sv)
  plib = or(getShellEnvVar("SFYCHECK_PLIB") "cmos22fdsoi")
  ;; Probed cells.
  foreach(lc list(list(plib "nfet") list(plib "pfet") list(plib "nfet_b") list(plib "opnpcres")
                  list(plib "ncap") list(plib "apmom") list(plib "vnpn") list(plib "diodenx")
                  list("analogLib" "nmos") list("analogLib" "pmos") list("analogLib" "npn")
                  list("analogLib" "pnp") list("analogLib" "presistor") list("analogLib" "nmos4"))
    when(ddGetObj(car(lc) cadr(lc))
      chkLine(sprintf(nil "views %s/%s" car(lc) cadr(lc)) chkPViews(car(lc) cadr(lc)))
      foreach(v chkPViews(car(lc) cadr(lc))
        chkE(sprintf(nil "terms %s/%s/%s" car(lc) cadr(lc) v) list('chkPTerms car(lc) cadr(lc) v)))
      foreach(s list("spectre" "auCdl" "hspiceD" "auLvs")
        sv = chkPSim(car(lc) cadr(lc) s)
        when(sv
          chkLine(sprintf(nil "termOrder %s/%s %s" car(lc) cadr(lc) s) sv->termOrder)
          when(sv->termMapping
            chkLine(sprintf(nil "termMapping %s/%s %s" car(lc) cadr(lc) s) sv->termMapping))
          when(sv->opParamExprList
            chkLine(sprintf(nil "opParamExprList-head %s/%s %s" car(lc) cadr(lc) s)
              car(sv->opParamExprList)))))))
    foreach(lib list(plib "analogLib") chkPImplicit(lib) chkPSimKeys(lib) chkPKeyExamples(lib))
  chkPass("p_probe sections ran"))
