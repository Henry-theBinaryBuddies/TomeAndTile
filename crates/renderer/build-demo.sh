#!/usr/bin/env bash
# Build the renderer to WASM and generate the JS glue into demo/pkg/.
# Usage: crates/renderer/build-demo.sh [--dev]   (default: release)
# Needs the wasm32 target and wasm-bindgen-cli at EXACTLY the version of the
# wasm-bindgen crate in Cargo.lock:
#   rustup target add wasm32-unknown-unknown
#   cargo install wasm-bindgen-cli --version <that version> --locked
set -euo pipefail
cd "$(dirname "$0")/../.."

profile=release
flag=--release
if [ "${1:-}" = "--dev" ]; then profile=debug; flag=; fi

want=$(awk '/^name = "wasm-bindgen"$/ {getline; gsub(/[^0-9.]/, "", $0); print; exit}' Cargo.lock)
have=$(wasm-bindgen --version 2>/dev/null | awk '{print $2}' || true)
if [ "$want" != "$have" ]; then
  echo "wasm-bindgen CLI is '${have:-missing}' but Cargo.lock wants $want." >&2
  echo "Run: cargo install wasm-bindgen-cli --version $want --locked" >&2
  exit 1
fi

# shellcheck disable=SC2086
cargo build -p tt-renderer --target wasm32-unknown-unknown $flag
wasm-bindgen --target web --no-typescript \
  --out-dir crates/renderer/demo/pkg \
  "target/wasm32-unknown-unknown/${profile}/tt_renderer.wasm"
ls -l crates/renderer/demo/pkg
