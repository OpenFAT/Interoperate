;; l8_layout.il: lane L8 live check, layout through KLayout (sfy_layout.il). A stub until the lane writes it:
;; wrap.il has loaded common.il and schemify.il and created chkLib (sfyprobe_l8), so
;; the check only drives Virtuoso and writes PASS/FAIL lines (chkThat, chkE).

chkSkip("lane L8 has not written this check yet")
