;; wrap.il: one live check in a -nograph Virtuoso (run.sh sets its environment). Loads
;; common.il and schemify.il, creates the lane's scratch library $SFYCHECK_LIB (attached to
;; cdsDefTechLib), loads the check $SFYCHECK inside errset, then always purges every open
;; cellview and exits, so a check that signals never leaves the session at its prompt.
;; Not shipped, not linted: plain SKILL, globals prefixed chk.
let((dir r)
  dir = getShellEnvVar("SFYCHECK_DIR")
  load(strcat(dir "/common.il"))
  chkOpen()
  r = errset(progn(
        load(getShellEnvVar("SFYCHECK_IL"))
        chkLib = getShellEnvVar("SFYCHECK_LIB")
        unless(ddGetObj(chkLib)
          ddCreateLib(chkLib strcat(getShellEnvVar("SFYCHECK_WORK") "/" chkLib)))
        techBindTechFile(ddGetObj(chkLib) "cdsDefTechLib")
        load(strcat(dir "/" getShellEnvVar("SFYCHECK") ".il"))) t)
  unless(r
    chkFail(sprintf(nil "check signalled: %L" getqq(errset errset))))
  chkClose()
  exit())
