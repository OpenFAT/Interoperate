#!/usr/bin/env bash
# Lane P live check driver (gaps2 P1 S8 G7): runs p_terms on the Virtuoso host (a PDK
# inverter, analogLib's 3-terminal MOS and a PDK resistor drawn in sfyprobe_p and dumped,
# the library kept), si-netlists them there in spectre and auCdl (p_si.sh), deletes the
# library, then fetches the dump and the netlists and compares Schemify's netlists of the
# import with them here (tests/virtuoso_rust.rs live_p_terms).
#
#   SFY_HOST=<host> live/checks/p_drive.sh
#
# Host side: SFYCHECK_WORK (default sfytest/checks-p, under the host's home; a cds.lib there
# that mounts the PDK), SFYCHECK_ENV (default sfytest/pdk/env.csh), SFYCHECK_PLIB (default
# cmos22fdsoi). The kit's data stays out of the repository: the files land in
# $SFYCHECK_LOCAL (default a fresh temporary directory, printed at the end).
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$here/../.."
host="${SFY_HOST:-}"
[[ -n $host ]] || { echo "usage: SFY_HOST=<host> $0" >&2; exit 2; }
work="${SFYCHECK_WORK:-sfytest/checks-p}"
envf="${SFYCHECK_ENV:-sfytest/pdk/env.csh}"
plib="${SFYCHECK_PLIB:-cmos22fdsoi}"
lib=sfyprobe_p
tmp="${SFYCHECK_LOCAL:-$(mktemp -d)}"
mkdir -p "$tmp/il"

cargo build -q --release --features cli --bin schemify-interop --manifest-path "$repo/Cargo.toml"
"${CARGO_TARGET_DIR:-$repo/target}/release/schemify-interop" install-skill "$tmp/il" >/dev/null
ssh "$host" "mkdir -p $work/src $work/out"
tar cf - -C "$here" . | ssh "$host" "tar xf - -C $work/src"
tar cf - -C "$tmp/il" schemify.il | ssh "$host" "tar xf - -C $work"

drop() {
  ssh "$host" bash -s <<DROP
rm -rf "\$HOME/$work/$lib"
sed -i '/^DEFINE $lib /d' "\$HOME/$work/cds.lib"
DROP
}
trap drop EXIT

status=0
ssh "$host" bash -s >"$tmp/run.txt" 2>&1 <<RUN || status=$?
export SFYCHECK_WORK=\$HOME/$work SFYCHECK_ENV=\$HOME/$envf SFYCHECK_PLIB=$plib SFYCHECK_KEEP=1
bash \$HOME/$work/src/run.sh p_terms && bash \$HOME/$work/src/p_si.sh
RUN
cat "$tmp/run.txt"
ssh "$host" "cd $work/out && tar cf - p_terms.sfyx p_terms.sfyx.log p_si" | tar xf - -C "$tmp"
for f in "$tmp"/p_si/*.net; do cp "$f" "$tmp/$(basename "$f")"; done
SFY_P_DIR="$tmp" cargo test -q --release --manifest-path "$repo/Cargo.toml" \
  --test virtuoso_rust live_p_terms -- --ignored --nocapture || status=1
echo "p_drive: files in $tmp"
exit $status
