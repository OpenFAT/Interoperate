;; p_simopt.il: lane P probe (the kit's selftest.* FAIL on simoption additionalArgs): loads
;; out/p_simopt_a.sfyx (a SfySelfTest first dump, maestro test opamp_tb) into chkLib and
;; prints maeGetSimOption's additionalArgs for the loaded test, then after
;; maeSetSimOption additionalArgs "vdsatmod=gds", and for a test made from nothing.
procedure(chkPOpt(sess test)
  car(setof(kv car(errset(maeGetSimOption(test ?session sess)))
    listp(kv) && car(kv) == "additionalArgs")))

let((sess)
  chkLine("load" SfyImport(strcat(getShellEnvVar("SFYCHECK_WORK") "/out/p_simopt_a.sfyx")
    ?lib chkLib ?overwrite t ?check nil))
  sess = car(errset(maeOpenSetup(chkLib "opamp_tb" "maestro" ?mode "a")))
  chkLine("tests" car(errset(maeGetSetup(?session sess))))
  chkLine("loaded additionalArgs" chkPOpt(sess "opamp_tb"))
  chkE("get one" list('maeGetSimOption "opamp_tb" ?option "additionalArgs" ?session sess))
  chkLine("set" errset(maeSetSimOption("opamp_tb" ?options list(list("additionalArgs" "vdsatmod=gds"))
    ?session sess) t))
  chkLine("after set" chkPOpt(sess "opamp_tb"))
  chkLine("set reltol" errset(maeSetSimOption("opamp_tb" ?options list(list("reltol" "1e-4"))
    ?session sess) t))
  chkLine("reltol after" car(errset(maeGetSimOption("opamp_tb" ?option "reltol" ?session sess))))
  chkLine("env additionalArgs" envGetVal("spectre.opts" "additionalArgs"))
  foreach(k list("acSeverity" "additionalArgs" "reltol" "gmin" "vthmod")
    chkLine(sprintf(nil "opt %s" k)
      list(car(errset(maeGetSimOption("opamp_tb" ?option k ?session sess)))
           errset(envGetVal("spectre.opts" k)))))
  chkLine("all options" length(car(errset(maeGetSimOption("opamp_tb" ?session sess)))))
  chkE("new test" list('maeCreateTest "fresh" ?lib chkLib ?cell "opamp_tb" ?view "schematic"
    ?simulator "spectre" ?session sess))
  chkLine("fresh additionalArgs" chkPOpt(sess "fresh"))
  errset(maeCloseSession(?session sess ?forceClose t))
  chkPass("p_simopt ran"))
