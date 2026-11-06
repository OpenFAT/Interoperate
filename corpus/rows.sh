#!/usr/bin/env bash
# Per-row category ratchet (inv_xschem row X23): every bench and mode keeps its category or
# gets better, so a move between categories never hides inside an unchanged sum.
#
#   corpus/rows.sh <RESULTS.txt> <recorded rows>            check
#   corpus/rows.sh --record <RESULTS.txt> > <recorded rows>  write the record
#
# <RESULTS.txt> lines start `<bench> <mode> <category>` (the differential's netlist-out file,
# or roundtrip.sh's); a bench and mode absent from it agree. The record lists the rows that do
# not agree, sorted. Ranks: agree < golden-unevaluated < differ < anything else (failed-to-*,
# panic, export-failed, no-netlist). Exit 1 when a row ranks worse than recorded (each named);
# a better row is a notice asking for the record to be rewritten in the change that earned it.
set -euo pipefail

rows() { # <RESULTS.txt> → sorted `bench mode category` of the rows that do not agree
  awk '{ c = $3; sub(/:$/, "", c); if (c != "agree") print $1, $2, c }' "$1" | sort -u
}

if [[ ${1:-} == --record ]]; then
  rows "$2"
  exit 0
fi
results="$1"
recorded="$2"
awk '
  function rank(c) { return c == "agree" ? 0 : c == "golden-unevaluated" ? 1 : c == "differ" ? 2 : 3 }
  NR == FNR { was[$1 " " $2] = $3; next }
  { now[$1 " " $2] = $3 }
  END {
    status = 0
    for (k in now) {
      w = (k in was) ? was[k] : "agree"
      if (rank(now[k]) > rank(w)) { print "::error::" k ": " w " -> " now[k]; status = 1 }
      else if (rank(now[k]) < rank(w)) print "::notice::" k ": " w " -> " now[k] " (rewrite the record)"
    }
    for (k in was) if (!(k in now)) print "::notice::" k ": " was[k] " -> agree (rewrite the record)"
    exit status
  }
' "$recorded" <(rows "$results")
