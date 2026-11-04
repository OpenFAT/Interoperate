;; l3_config.il: lane L3 live check, config views (sfy_cfg.il). A stub until the lane writes it:
;; wrap.il has loaded common.il and schemify.il and created chkLib (sfyprobe_l3), so
;; the check only drives Virtuoso and writes PASS/FAIL lines (chkThat, chkE).

chkSkip("lane L3 has not written this check yet")
