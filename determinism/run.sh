#!/usr/bin/env bash
# Determinism oracle (Schemify contributing.md rule 4): writes the importer outputs that must be
# byte-identical on every platform. CI runs it on each OS (and the WASI build under wasmtime)
# and diffs the trees (`.github/workflows/ci.yml`, job `determinism`).
#
#   determinism/run.sh '<schemify-interop command>' <out-dir>
#
# The command may carry a runner prefix ('wasmtime run --dir=.::/ schemify-interop.wasm').
# Every path it receives is relative to the repository root, the working directory of the
# call. Writes:
#   import/       `schemify-interop xschem-import` of determinism/xschem (hand-written)
#   import.status its exit code
# `*.log` files hold stderr for debugging and are not compared (messages may spell paths
# natively).
set -euo pipefail

interop="$1"
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/.." && pwd)"
mkdir -p "$2"
out="$(cd "$2" && pwd)"
# Outputs go to a directory under the repository so a WASI runner's `--dir=.` reaches them.
rel=".determinism-out"
cd "$repo"
rm -rf "$rel"
mkdir -p "$rel"
export LC_ALL=C

x="determinism/xschem"
set +e
$interop xschem-import "$x/devices" "$x/lib" --root "$x" -o "$rel/import" 2>"$rel/import.log" >>"$rel/import.log"
echo "$?" >"$rel/import.status"
set -e

rm -rf "${out:?}"/*
cp -R "$rel"/. "$out"/
rm -rf "$rel"
echo "determinism outputs: $(find "$out" -type f ! -name '*.log' | wc -l) files in $out"
