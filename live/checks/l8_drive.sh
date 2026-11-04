#!/usr/bin/env bash
# Lane L8 live check driver (gap H): runs l8_layout on the Virtuoso host and does its
# Schemify half here, with this checkout's CLI and KLayout.
#
#   SFY_HOST=<host> live/checks/l8_drive.sh   (SFY_HOST: the Virtuoso host's ssh name, required;
#                                             KLAYOUT, default klayout on PATH, else nixpkgs#klayout)
#
# Ships live/checks and this checkout's schemify.il to <host>:sfytest/checks, starts
# `run.sh l8_layout` there, and when the session's export bundle appears: takes it into a
# scratch Schemify project (layout-export), checks the GDS, the cell's vlayout and that
# KLayout loads the generated .lyt/.lyp over the GDS; writes the restore bundle of the
# unchanged GDS (layout-import), moves the rect at (0,0;2,1) on y0 by +0.5 in x with
# KLayout, writes the strmin bundle and checks its report; sends both bundles back and
# lets the session go on. Prints its own PASS/FAIL lines, then the check's; exits 1 on a
# FAIL of either, else with run.sh's status. The bundles and the project stay in
# $SFYCHECK_LOCAL (default a fresh temporary directory, printed at the end).
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$here/../.."
host="${SFY_HOST:-}"
[[ -n $host ]] || { echo "usage: SFY_HOST=<host> $0   (SFY_HOST: the Virtuoso host's ssh name)" >&2; exit 2; }
work=sfytest/checks
lib=sfyprobe_l8
tmp="${SFYCHECK_LOCAL:-$(mktemp -d)}"
mkdir -p "$tmp/il"
kl="${KLAYOUT:-$(command -v klayout || echo "$(nix build nixpkgs#klayout --no-link --print-out-paths)/bin/klayout")}"
fails=0
verdict() { if eval "$2"; then echo "l8_drive: PASS $1"; else echo "l8_drive: FAIL $1"; fails=1; fi; }
klb() { QT_QPA_PLATFORM=offscreen "$kl" -b "$@"; }

cargo build -q --features cli --bin schemify-interop --manifest-path "$repo/Cargo.toml"
bin="${CARGO_TARGET_DIR:-$repo/target}/debug/schemify-interop"
"$bin" install-skill "$tmp/il" >/dev/null
ssh "$host" "mkdir -p $work/src $work/out && find $work/out -maxdepth 1 -name 'l8_*' -exec rm -rf {} +"
scp -q "$here"/*.il "$here"/run.sh "$host:$work/src/"
scp -q "$tmp/il/schemify.il" "$host:$work/schemify.il"

ssh "$host" "bash $work/src/run.sh l8_layout" >"$tmp/run.txt" 2>&1 &
run=$!
for _ in $(seq 300); do
  ssh "$host" "test -f $work/out/l8_export.ready" && break
  kill -0 $run 2>/dev/null || break
  sleep 3
done
if scp -rq "$host:$work/out/l8_export" "$tmp/bundle"; then
  p="$tmp/proj"
  rm -rf "$p"; mkdir -p "$p/$lib"
  printf 'schemify 1 shbench\n' >"$p/$lib/top.shbench"
  "$bin" layout-export "$p" "$tmp/bundle" || fails=1
  gds="$p/$lib/top.layout.gds"
  verdict "the project's GDS is the bundle's" "cmp -s '$gds' '$tmp/bundle/layout.gds'"
  verdict "the cell file names it (vlayout layout <sha256>)" \
    "grep -q \"^  vlayout layout $(sha256sum "$gds" | cut -c1-64)\$\" '$p/$lib/top.shbench'"
  verdict "the stash holds the OA files" "cmp -s '$p/.schemify/virtuoso/$lib/top/layout/oa/layout.oa' '$tmp/bundle/oa/layout.oa'"
  cat >"$tmp/lyp.py" <<'EOF'
import pya
t = pya.Technology.technology_from_xml(open(tech).read())
o = t.load_layout_options
v = pya.LayoutView()
v.load_layout(gds, o, True)
v.load_layer_props(lyp)
ly = v.cellview(0).layout()
print("lyt", t.name, "dbu", t.dbu, "map", o.layer_map.to_string().replace("\n", " ; "))
print("top", ly.top_cell().name, "arrays", sum(1 for i in ly.top_cell().each_inst() if i.is_regular_array()))
it = v.begin_layers()
while not it.at_end():
    q = it.current()
    print("layer", q.name, q.source, "frame %06x" % (q.frame_color & 0xffffff), "fill %06x" % (q.fill_color & 0xffffff), "dither", q.dither_pattern, "width", q.width, "visible", q.visible)
    it.next()
EOF
  klb -r "$tmp/lyp.py" -rd tech="$p/$lib/cdsDefTechLib.lyt" -rd lyp="$p/$lib/cdsDefTechLib.lyp" -rd gds="$gds" >"$tmp/klayout.txt" 2>&1 || true
  sed 's/^/l8_drive: klayout: /' "$tmp/klayout.txt"
  verdict "KLayout loads the .lyt/.lyp over the GDS (y0 yellow)" "grep -q '^layer y0.drawing 207/0@1 frame ffff00' '$tmp/klayout.txt'"
  verdict "KLayout sees the mosaic as an array" "grep -q '^top top arrays 1' '$tmp/klayout.txt'"
  "$bin" layout-import "$p" "$lib/top" -o "$tmp/imp_a" || fails=1
  verdict "unchanged GDS: a restore bundle" "grep -q '(\"mode\" \"restore\")' '$tmp/imp_a/layout.sfyx' && cmp -s '$tmp/imp_a/oa/layout.oa' '$tmp/bundle/oa/layout.oa'"
  cat >"$tmp/move.py" <<'EOF'
import pya
ly = pya.Layout()
ly.read(gds)
top = ly.cell("top")
li = ly.find_layer(207, 0)
hit = [s for s in top.shapes(li).each() if (s.is_box() or s.is_polygon() or s.is_simple_polygon()) and s.bbox() == pya.Box(0, 0, 2000, 1000)]
for s in hit:
    s.transform(pya.Trans(500, 0))
ly.write(gds)
print("moved", len(hit))
EOF
  klb -r "$tmp/move.py" -rd gds="$gds" >"$tmp/move.txt" 2>&1 || fails=1
  sed 's/^/l8_drive: klayout: /' "$tmp/move.txt"
  KLAYOUT="$kl" "$bin" layout-import "$p" "$lib/top" -o "$tmp/imp_b" >"$tmp/imp_b.txt" || fails=1
  sed 's/^/l8_drive: /' "$tmp/imp_b.txt"
  verdict "edited GDS: a strmin bundle with the export's layer map" "grep -q '(\"mode\" \"strmin\")' '$tmp/imp_b/layout.sfyx' && cmp -s '$tmp/imp_b/layermap' '$tmp/bundle/layermap'"
  verdict "the report names the PCell instance" "grep -q 'PCell instance PC0 of $lib/pc' '$tmp/imp_b/interop.report.txt'"
  verdict "the report lists the moved rect (KLayout's diff)" "grep -q 'cell top layer 207/0: box 1 removed, 1 added' '$tmp/imp_b/interop.report.txt'"
  scp -rq "$tmp/imp_a" "$host:$work/out/l8_imp_a"
  scp -rq "$tmp/imp_b" "$host:$work/out/l8_imp_b"
  ssh "$host" "touch $work/out/l8_go"
else
  echo "l8_drive: FAIL no export bundle from the session"
  fails=1
fi
status=0
wait $run || status=$?
cat "$tmp/run.txt"
scp -q "$host:$work/out/l8_strmin.log" "$tmp/strmin.log" 2>/dev/null || true
echo "l8_drive: bundles and project in $tmp"
[[ $fails == 0 ]] || exit 1
exit $status
