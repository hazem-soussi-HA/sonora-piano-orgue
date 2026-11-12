#!/usr/bin/env bash
# Build the Sonor machine (WSL / Linux side: wasm core only).
set -e
cd "$(dirname "$0")"
CARGO="${CARGO:-$HOME/.cargo/bin/cargo}"
[ -x "$CARGO" ] || CARGO=cargo

echo "[1/1] wasm core ..."
"$CARGO" build --release --target wasm32-unknown-unknown -p sonor-core
cp target/wasm32-unknown-unknown/release/sonor_core.wasm ../public/sonor.wasm
echo "      -> ../public/sonor.wasm"
echo
echo "the native windows listener is built with build.cmd on windows."
