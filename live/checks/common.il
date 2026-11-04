;; common.il: what every live check shares (run.sh, wrap.il). Every check writes one text
;; file, $SFYCHECK_OUT, flushed per line, so a session that dies on a dialog still leaves
;; what it got. Verdict lines start with PASS, FAIL or SKIP; chkClose writes `end`.

chkPort = nil
chkLib = nil

procedure(chkOpen()
  chkPort = outfile(getShellEnvVar("SFYCHECK_OUT") "w")
  fprintf(chkPort "check %s virtuoso %L lib %s\n" getShellEnvVar("SFYCHECK") getVersion(t)
    getShellEnvVar("SFYCHECK_LIB"))
  drain(chkPort)
  t)

;; One observation: label => value.
procedure(chkLine(label val)
  fprintf(chkPort "%s => %L\n" label val)
  drain(chkPort)
  val)

;; Evaluates the quoted form (or calls the function) inside errset: its value, or the error.
procedure(chkE(label form)
  let((r)
    r = errset(if(listp(form) eval(form) funcall(form)) t)
    if(r
      then fprintf(chkPort "%s => %L\n" label car(r))
      else fprintf(chkPort "%s ERROR %L\n" label getqq(errset errset)))
    drain(chkPort)
    car(r)))

procedure(chkPass(msg) fprintf(chkPort "PASS %s\n" msg) drain(chkPort) t)
procedure(chkFail(msg) fprintf(chkPort "FAIL %s\n" msg) drain(chkPort) nil)
procedure(chkSkip(msg) fprintf(chkPort "SKIP %s\n" msg) drain(chkPort) nil)

;; Passes when ok is non-nil, else fails; returns ok.
procedure(chkThat(msg ok)
  if(ok then chkPass(msg) else chkFail(msg))
  ok)

procedure(chkClose()
  foreach(cv dbGetOpenCellViews() errset(dbPurge(cv)))
  when(chkPort
    fprintf(chkPort "end\n")
    close(chkPort)
    chkPort = nil)
  t)
