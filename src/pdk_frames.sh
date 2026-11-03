# schemify-interop pdk-scan --remote-script: run on the PDK host as
# `sh -s -- '<abs path>'...`. Prints `sfyframe 1`, one frame per path (`f <bytes> <path>`
# and the file's structural lines; `f 0 ! <path>` when unreadable), then `end`. Kept:
# .subckt .ends .model .lib .endl .include .inc .param .option(s) .prot(ect) .unprot(ect)
# lines and the `+` lines continuing them; `.model` cut to `.model name type`; `.param` and
# `.subckt` values dropped (`a=1` -> `a=`), ports kept. Nothing inside .prot is kept.
# POSIX sh and awk only; writes nothing.
LC_ALL=C
export LC_ALL
printf 'sfyframe 1\n'
for p in "$@"; do
  if [ -f "$p" ] && [ -r "$p" ] && t=$(awk '
    # The line cut at its inline comment: `;`, a blank-delimited `$`, a blank then `//`.
    function uncomment(s,  t) {
      if (index(s, ";")) s = substr(s, 1, index(s, ";") - 1)
      t = " " s
      if (match(t, /[ \t]\$([ \t]|$)/)) s = substr(s, 1, RSTART - 1)
      t = " " s
      if (match(t, /[ \t]\/\//)) s = substr(s, 1, RSTART - 1)
      return s
    }
    # `s` with every value after `=` dropped; st (0 text, 1 before the value, 2 in it),
    # depth and q carry a value across continuation lines.
    function strip(s,  out, i, c) {
      out = ""
      for (i = 1; i <= length(s); i++) {
        c = substr(s, i, 1)
        if (st == 0) { out = out c; if (c == "=") st = 1; continue }
        if (st == 1) { if (c == " " || c == "\t") continue; st = 2 }
        if (q != "") { if (c == q) q = ""; continue }
        if (c == "\047" || c == "\"") { q = c; continue }
        if (c == "{" || c == "(") { depth++; continue }
        if (c == "}" || c == ")") { if (depth > 0) depth--; continue }
        if ((c == " " || c == "\t") && depth == 0) { st = 0; out = out c }
      }
      if (st == 2 && depth == 0 && q == "") st = 0
      return out
    }
    {
      line = uncomment($0)
      n = split(line, w, " ")
      if (n == 0) next
      k = tolower(w[1])
      if (prot) {
        if (k == ".unprot" || k == ".unprotect") { prot = 0; print k }
        next
      }
      if (substr(k, 1, 1) == "*") next
      if (substr(k, 1, 1) == "+") {
        rest = substr(line, index(line, "+") + 1)
        if (keep == 1) print "+" rest
        if (keep == 2) print "+" strip(rest)
        next
      }
      keep = 0; st = 0; depth = 0; q = ""
      if (k == ".prot" || k == ".protect") { prot = 1; print k; next }
      if (k == ".model") { m = w[3]; sub(/\(.*/, "", m); print w[1], w[2], m; next }
      if (k == ".param" || k == ".subckt") { keep = 2; print strip(line); next }
      if (k == ".ends" || k == ".lib" || k == ".endl" || k == ".include" || k == ".inc" ||
          k == ".option" || k == ".options" || k == ".unprot" || k == ".unprotect") {
        keep = 1; print line
      }
    }' "$p" && printf x); then
    t=${t%x}
    printf 'f %s %s\n%s' "$(printf %s "$t" | wc -c | tr -d ' ')" "$p" "$t"
  else
    printf 'f 0 ! %s\n' "$p"
  fi
done
printf 'end\n'
