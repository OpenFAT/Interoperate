#!/usr/bin/env bash
# One lane's live check on the Virtuoso host (vhost), modelled on the research probes'
# harness (notes/interop-research/probes: run.sh, wrap.il):
#
#   scp -r live/checks vhost:sfytest/checks/src
#   scp schemify.il vhost:sfytest/checks/      (schemify-interop install-skill <dir>)
#   ssh vhost bash sfytest/checks/src/run.sh <check>
#
# <check> is a file stem here: l1_mae_xml … l9_cdfcb. The lane is the part before the first
# `_`; the check runs in that lane's scratch library sfyprobe_<lane>, created for the run
# and deleted after it (directory and cds.lib entry), so lanes never share one. The
# Virtuoso session holds /tmp/user-sfy-run.lock, on the host's local disk: one run at
# a time across every lane. Output: <work>/out/<check>.txt (the check's lines, `end` last)
# and <check>.CDS.log. Only analogLib, basic and cdsDefTechLib are used.
#
# Environment: SFYCHECK_WORK (default ~/sfytest/checks), SFYCHECK_IL (default
# <work>/schemify.il), SFYCHECK_ENV (the csh file that sets up Cadence, default
# ~/sfytest/env.csh), CDS (the Virtuoso install), SFYCHECK_TIMEOUT (seconds, default 2400).
# Exit status: 0 a PASS and no FAIL, 1 a FAIL or no `end`, 2 usage or setup, 3 nothing
# checked (only SKIP lines: the lane's check is still a stub).
set -uo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
check="${1:-}"
[[ -n $check && -f "$here/$check.il" && $check != common && $check != wrap ]] || {
  echo "usage: run.sh <check>   (one of: $(cd "$here" && printf '%s ' l[0-9]*_*.il | sed 's/\.il//g'))" >&2
  exit 2
}
lane="${check%%_*}"
lib="sfyprobe_$lane"
work="${SFYCHECK_WORK:-$HOME/sfytest/checks}"
il="${SFYCHECK_IL:-$work/schemify.il}"
envfile="${SFYCHECK_ENV:-$HOME/sfytest/env.csh}"
CDS="${CDS:?}"
out="$work/out"
[[ -f $il ]] || { echo "run.sh: no $il (schemify-interop install-skill writes one)" >&2; exit 2; }
mkdir -p "$out"
cd "$work" || exit 2
[[ -f cds.lib ]] || cat > cds.lib <<CDSLIB
DEFINE analogLib $CDS/tools/dfII/etc/cdslib/artist/analogLib
DEFINE basic $CDS/tools/dfII/etc/cdslib/basic
DEFINE cdsDefTechLib $CDS/tools/dfII/etc/cdsDefTechLib
CDSLIB

# Removes the lane's scratch library: its directory and its cds.lib entry.
drop_lib() {
  rm -rf "${work:?}/$lib"
  sed -i "/^DEFINE $lib /d" cds.lib
}
drop_lib
trap drop_lib EXIT

rm -f "$out/$check.txt"
flock /tmp/user-sfy-run.lock csh -f -c "source $envfile >& /dev/null; cd $work; \
  setenv SFYCHECK $check; setenv SFYCHECK_DIR $here; setenv SFYCHECK_LIB $lib; \
  setenv SFYCHECK_WORK $work; setenv SFYCHECK_IL $il; setenv SFYCHECK_OUT $out/$check.txt; \
  timeout ${SFYCHECK_TIMEOUT:-2400} virtuoso -nograph -restore $here/wrap.il -log $out/$check.CDS.log < /dev/null" \
  >/dev/null 2>&1
echo "$check: virtuoso exit $?"
[[ -f "$out/$check.txt" ]] && cat "$out/$check.txt"
grep -qx end "$out/$check.txt" 2>/dev/null || { echo "$check: FAIL (no end line: the session died)"; exit 1; }
grep -q '^FAIL' "$out/$check.txt" && exit 1
grep -q '^PASS' "$out/$check.txt" || exit 3
exit 0
