# schemify-interop pdk-scan --remote-script: run on the PDK host as
# `sh -s -- '<abs path>'...`. Prints `sfyframe 1`, one frame per path (`f <bytes> <path>`
# and the file's structural lines; `f 0 ! <path>` when unreadable), then `end`. Kept:
# .subckt .ends .model .lib .endl .include .inc .param .option(s) .prot(ect) .unprot(ect)
# lines and the `+` lines continuing them; `.model` cut to `.model name type`; `.param` and
# `.subckt` values dropped (`a=1` -> `a=`), ports kept. Nothing inside .prot is kept.
# Spectre: subckt, inline subckt, ends, model (cut to `model name type`), parameters (values
# replaced by 0, which its grammar reads), include, section, endsection, library,
# endlibrary, simulator and ahdl_include lines, with their `\` continuations; `//` lines
# are comments.
# Verilog-A (.va, .vams): module headers to their `;`, endmodule, and parameter declarations
# as names only (`parameter real a=0, b=0;`); comments, attributes and strings dropped.
# POSIX sh and awk only; writes nothing.
LC_ALL=C
export LC_ALL
printf 'sfyframe 1\n'
for p in "$@"; do
  case "$p" in *.va|*.vams|*.VA|*.VAMS) va=1 ;; *) va=0 ;; esac
  if [ -f "$p" ] && [ -r "$p" ] && t=$(awk -v va="$va" '
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
        if (st == 1) { if (c == " " || c == "\t") continue; st = 2; if (zero) out = out "0" }
        if (q != "") { if (c == q) q = ""; continue }
        if (c == "\047" || c == "\"") { q = c; continue }
        if (c == "{" || c == "(") { depth++; continue }
        if (c == "}" || c == ")") { if (depth > 0) depth--; continue }
        if ((c == " " || c == "\t") && depth == 0) { st = 0; out = out c }
      }
      if (st == 2 && depth == 0 && q == "") st = 0
      return out
    }
    va == 1 {
      line = $0
      if (vcom) { if (!index(line, "*/")) next; line = substr(line, index(line, "*/") + 2); vcom = 0 }
      while (match(line, /\/\*([^*]|\*[^\/])*\*\//)) line = substr(line, 1, RSTART - 1) " " substr(line, RSTART + RLENGTH)
      if (index(line, "/*")) { line = substr(line, 1, index(line, "/*") - 1); vcom = 1 }
      if (index(line, "//")) line = substr(line, 1, index(line, "//") - 1)
      gsub(/\(\*[^*]*\*\)/, "", line)
      gsub(/"[^"]*"/, "\"\"", line)
      n = split(line, w, " "); k = (n ? w[1] : "")
      if (mcont || k == "module" || k == "macromodule") { print line; mcont = (line !~ /;/); next }
      if (pcont || k == "parameter") {
        names = ""; t = line
        while (match(t, /[A-Za-z_][A-Za-z0-9_$]*[ \t]*=[^=]/)) {
          nm = substr(t, RSTART, RLENGTH); sub(/[ \t]*=.*/, "", nm)
          names = names (names == "" ? "" : ", ") nm "=0"; t = substr(t, RSTART + RLENGTH)
        }
        if (names != "") print "parameter real " names ";"
        pcont = (line !~ /;/); next
      }
      if (k == "endmodule") print "endmodule"
      next
    }
    {
      line = uncomment($0)
      # A Spectre line continuing a kept one (the kept one ended in a backslash).
      if (cont) {
        cont = (line ~ /\\[ \t]*$/)
        print (keep == 2 ? strip(line) : line)
        next
      }
      n = split(line, w, " ")
      if (n == 0) next
      k = tolower(w[1])
      if (substr(k, 1, 2) == "//") next
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
      keep = 0; st = 0; depth = 0; q = ""; zero = 0
      if (k == ".prot" || k == ".protect") { prot = 1; print k; next }
      if (k == ".model") { m = w[3]; sub(/\(.*/, "", m); print w[1], w[2], m; next }
      if (k == ".param" || k == ".subckt") { keep = 2; print strip(line); next }
      if (k == ".ends" || k == ".lib" || k == ".endl" || k == ".include" || k == ".inc" ||
          k == ".option" || k == ".options" || k == ".unprot" || k == ".unprotect") {
        keep = 1; print line; next
      }
      # Spectre.
      if (k == "model") { m = w[3]; sub(/[({].*/, "", m); print w[1], w[2], m; next }
      if (k == "parameters" || k == "subckt" || (k == "inline" && tolower(w[2]) == "subckt")) {
        keep = 2; zero = 1; print strip(line); cont = (line ~ /\\[ \t]*$/); next
      }
      if (k == "ends" || k == "include" || k == "section" || k == "endsection" ||
          k == "library" || k == "endlibrary" || k == "simulator" || k == "ahdl_include") {
        keep = 1; print line; cont = (line ~ /\\[ \t]*$/)
      }
    }' "$p" && printf x); then
    t=${t%x}
    printf 'f %s %s\n%s' "$(printf %s "$t" | wc -c | tr -d ' ')" "$p" "$t"
  else
    printf 'f 0 ! %s\n' "$p"
  fi
done
printf 'end\n'
