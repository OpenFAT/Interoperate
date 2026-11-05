"""a_forms.py: writes src/virtuoso/ade/forms.tsv (lane A, gaps round 2) from two texts taken
on the Virtuoso host:

  ssh <host> cat '$CDS/tools/dfII/etc/tools/spectre/.cdsenv' > cdsenv.txt
  ssh <host> bash -s <<'EOF' > spectre_h.txt
  for a in <kinds below, `options` for opts>; do echo "=== $a"
    csh -f -c "source ~/sfytest/env.csh >& /dev/null; spectre -h $a"; done
  EOF
  python3 live/checks/a_forms.py cdsenv.txt spectre_h.txt > src/virtuoso/ade/forms.tsv

`.cdsenv` rows `spectre.<kind>[Opts] <field> <type> <default>` give the fields ADE answers
untouched (maeGetAnalysis, maeGetSimOption for `opts`); `spectre -h` numbers the parameters
of each statement."""
import re
import sys

KINDS = ("tran dc ac noise xf stb pz sens dcmatch sp pss pac pnoise pxf pstb hb hbac hbnoise "
         "hbsp hbstb hbxf envlp acmatch qpss qpac qpnoise qpxf qpsp psp lf opts").split()
cdsenv, helps = sys.argv[1:3]
defaults = {}
for line in open(cdsenv, encoding="latin-1"):
    m = re.match(r"^\s*spectre\.(\w+)\t(\w+)\t(\w+)\t(.*?)(\t.*)?$", line.rstrip("\n"))
    if not m:
        continue
    k, f, ty, v = m.group(1), m.group(2), m.group(3), m.group(4)
    k = k[:-4] if k.endswith("Opts") else k
    if k not in KINDS or f == "enable":
        continue
    if ty == "toggle":
        ok = v.startswith("(") and v != "(nil)"
    elif ty == "boolean":
        ok, v = v.strip() == "t", "t"
    elif v.startswith('"') and v.endswith('"'):
        v = v[1:-1]
        ok = v != ""
    else:
        ok = v not in ("nil", "(nil)", "")
    if ok:
        defaults.setdefault(k, []).append((f, v))
params, cur = {}, None
for line in open(helps, encoding="latin-1"):
    if line.startswith("=== "):
        cur = line.split()[1]
        cur = "opts" if cur == "options" else cur
        continue
    m = re.match(r"^\d+\s+([A-Za-z_]\w*)", line)
    if m and cur:
        params.setdefault(cur, []).append(m.group(1))
print("# ADE (ICADVM20.1) analysis form fields with a default and Spectre (21.10) analysis parameters,")
print("# per analysis kind (spec interop.md §9.2). Generated 2026-11-04 on iccad4 from")
print("# $CDS/tools/dfII/etc/tools/spectre/.cdsenv (rows spectre.<kind> and spectre.<kind>Opts) and")
print("# `spectre -h <kind>` (its numbered parameter lines). Rows (TAB-separated):")
print("#   d kind field value   a field maeGetAnalysis (kind options: maeGetSimOption) answers untouched")
print("#   s kind param...      the parameters of Spectre's statement (options: the options statement)")
for k in KINDS:
    name = "options" if k == "opts" else k
    for f, v in defaults.get(k, []):
        assert "\t" not in v
        print(f"d\t{name}\t{f}\t{v}")
    if k in params:
        print(f"s\t{name}\t" + " ".join(dict.fromkeys(params[k])))
