#!/usr/bin/env bash
# Builds the engine for the Foundry module:
#   crates/pf2e-wasm  →  foundry/wasm-pkg/pf2e_stockfish{.js,_bg.wasm,.d.ts}
#
# Requirements: rustup target wasm32-unknown-unknown, and the wasm-bindgen CLI
# at exactly the version pinned in crates/pf2e-wasm/Cargo.toml:
#   cargo install wasm-bindgen-cli --version 0.2.130 --locked
# Optional: wasm-opt (binaryen) for a further ~10-20% size reduction.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="$ROOT/foundry/wasm-pkg"
PINNED="$(sed -n 's/^wasm-bindgen = "=\(.*\)"/\1/p' "$ROOT/crates/pf2e-wasm/Cargo.toml")"

if ! command -v wasm-bindgen >/dev/null; then
  echo "wasm-bindgen CLI not found: cargo install wasm-bindgen-cli --version $PINNED --locked" >&2
  exit 1
fi
HAVE="$(wasm-bindgen --version | awk '{print $2}')"
if [[ "$HAVE" != "$PINNED" ]]; then
  echo "wasm-bindgen CLI is $HAVE but the crate pins $PINNED; install the matching CLI." >&2
  exit 1
fi

cd "$ROOT"
cargo build -p pf2e-wasm --target wasm32-unknown-unknown --profile wasm
rm -rf "$OUT"
wasm-bindgen --target web --out-dir "$OUT" --out-name pf2e_stockfish \
  "$ROOT/target/wasm32-unknown-unknown/wasm/pf2e_wasm.wasm"

if command -v wasm-opt >/dev/null; then
  wasm-opt -O3 --enable-bulk-memory --enable-nontrapping-float-to-int \
    "$OUT/pf2e_stockfish_bg.wasm" -o "$OUT/pf2e_stockfish_bg.wasm"
fi
ls -l "$OUT"
