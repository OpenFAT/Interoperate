;; l6_multitest.il: lane L6 live check, multiple tests and run plans (sfy_ade.il). A stub until the lane writes it:
;; wrap.il has loaded common.il and schemify.il and created chkLib (sfyprobe_l6), so
;; the check only drives Virtuoso and writes PASS/FAIL lines (chkThat, chkE).

chkSkip("lane L6 has not written this check yet")
