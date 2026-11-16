#!/usr/bin/env bash
# Build the renderer to WASM and generate the JS glue into demo/pkg/.
# Usage: crates/renderer/build-demo.sh [--dev]   (default: release)
# This is tools/build-renderer.sh with the demo's output directory. See that
# script for what it needs (the wasm32 target and a matching wasm-bindgen-cli).
set -euo pipefail
cd "$(dirname "$0")/../.."
exec bash tools/build-renderer.sh --out crates/renderer/demo/pkg "$@"
