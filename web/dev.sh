#!/usr/bin/env bash
# The dev loop (CLAUDE.md commands): build the client wasm, copy it where
# vite serves it, keep a background watcher rebuilding on crate changes,
# run vite in the foreground. Heavy jobs are niced — this box shares cores.
set -euo pipefail
cd "$(dirname "$0")"

build_wasm() {
  (cd .. && nice -n 15 ionice -c3 cargo build -p client-wasm --release \
    --target wasm32-unknown-unknown)
  mkdir -p public
  cp ../target/wasm32-unknown-unknown/release/client_wasm.wasm public/client_wasm.wasm
  echo "client_wasm.wasm refreshed"
}

build_wasm
[ -d node_modules ] || npm install --no-audit --no-fund

(
  while sleep 2; do
    if [ -n "$(find ../crates -name '*.rs' -newer public/client_wasm.wasm \
      -print -quit 2>/dev/null)" ]; then
      build_wasm || echo "wasm rebuild failed — fix and save again"
    fi
  done
) &
WATCHER=$!
trap 'kill "$WATCHER" 2>/dev/null || true' EXIT

npx vite
