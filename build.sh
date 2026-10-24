#!/usr/bin/env bash
# Builds the interop guest for wasm32-unknown-unknown into interop.wasm at the repository root
# (committed: a Schemify project loads it with `plugin interop path <dir>/interop.wasm`, and
# tests/guest.rs drives it through the real host). Fails when the module reaches 2048 KB
# (the format, doc, expr crates and every converter are linked in).
#
# Needs the wasm32-unknown-unknown std (rust-toolchain.toml; `nix develop` has Rust 1.95.0 with
# it and rust-lld). A toolchain without rust-lld (nixpkgs' rustc) links with wasm-ld instead,
# re-running the script under `nix shell nixpkgs#lld` when none is on PATH.
#
# Reproducible: CI rebuilds the module and fails when the committed one differs. The bytes
# depend only on the sources, Cargo.lock and the toolchain: build in `nix develop` (the
# flake.lock pins; CI does) or commit the `interop-wasm` artifact of the CI run. Host paths
# are remapped out of the module (panic locations would otherwise name this checkout).
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
host="$(rustc -vV | sed -n 's/^host: //p')"
# Unit separator between flags: paths may contain spaces.
export CARGO_ENCODED_RUSTFLAGS="--remap-path-prefix=$here=/schemify-interop"$'\x1f'"--remap-path-prefix=${CARGO_HOME:-$HOME/.cargo}=/cargo"

if [[ ! -x "$(rustc --print sysroot)/lib/rustlib/$host/bin/rust-lld" ]] \
   && [[ -z "${CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_LINKER:-}" ]]; then
  if command -v wasm-ld >/dev/null; then
    export CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_LINKER=wasm-ld
  else
    exec nix shell nixpkgs#lld -c env CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_LINKER=wasm-ld "$0" "$@"
  fi
fi

target="${CARGO_TARGET_DIR:-$here/target}"
cargo build --manifest-path "$here/Cargo.toml" --locked --release --target wasm32-unknown-unknown

wasm="$target/wasm32-unknown-unknown/release/interop.wasm"
limit=2048
size=$(wc -c <"$wasm")
if (( size >= limit * 1024 )); then
  echo "error: interop.wasm is $size bytes (limit $limit KB)" >&2
  exit 1
fi
cp "$wasm" "$here/interop.wasm"
echo "interop.wasm $size bytes -> $here"
