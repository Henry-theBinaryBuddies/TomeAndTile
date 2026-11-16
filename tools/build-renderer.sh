#!/usr/bin/env bash
# Build the renderer (crates/renderer) to WASM and generate the JavaScript glue
# and TypeScript declarations that web/ imports.
#
# Usage: tools/build-renderer.sh [--dev] [--out DIR]
#   --dev      debug build (faster to compile, much larger .wasm). Default: release.
#   --out DIR  where to write the package, relative to the repository root.
#              Default: web/src/lib/renderer/pkg (git-ignored: `pkg/` is in .gitignore).
#
# Needs the wasm32 target and wasm-bindgen-cli at EXACTLY the version of the
# wasm-bindgen crate in Cargo.lock; the script checks and prints the command if not:
#   rustup target add wasm32-unknown-unknown
#   cargo install wasm-bindgen-cli --version <that version> --locked
set -euo pipefail

cd "$(dirname "$0")/.."

profile=release
flag=--release
out=web/src/lib/renderer/pkg
while [ $# -gt 0 ]; do
  case "$1" in
    --dev) profile=debug; flag="" ;;
    --out)
      [ $# -ge 2 ] || { echo "--out needs a directory" >&2; exit 2; }
      out=$2; shift ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
  shift
done

want=$(awk '/^name = "wasm-bindgen"$/ {getline; gsub(/[^0-9.]/, "", $0); print; exit}' Cargo.lock)
have=$(wasm-bindgen --version 2>/dev/null | awk '{print $2}' || true)
if [ -z "$want" ]; then
  echo "could not find wasm-bindgen in Cargo.lock" >&2
  exit 1
fi
if [ "$want" != "$have" ]; then
  echo "wasm-bindgen CLI is '${have:-missing}' but Cargo.lock wants $want." >&2
  echo "Run: cargo install wasm-bindgen-cli --version $want --locked" >&2
  exit 1
fi

# shellcheck disable=SC2086
cargo build -p tt-renderer --target wasm32-unknown-unknown $flag
wasm-bindgen --target web --out-dir "$out" \
  "target/wasm32-unknown-unknown/${profile}/tt_renderer.wasm"

# The generated glue is plain JavaScript, and SvelteKit's default tsconfig
# type-checks JavaScript, which reports about 2,000 implicit-any errors in it.
# The types web/ uses are in tt_renderer.d.ts, so tell the checker to skip the
# glue file itself.
glue="$out/tt_renderer.js"
{ printf '// @ts-nocheck\n'; cat "$glue"; } > "$glue.tmp"
mv "$glue.tmp" "$glue"

ls -l "$out"
