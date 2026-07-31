#!/usr/bin/env bash
# The gates — exactly what CI runs; run before every merge (CLAUDE.md).
# Every wall asserts or this script exits nonzero. A missing dependency is
# a loud failure, never a silent skip: a pass it didn't earn is the worst
# bug class.
set -euo pipefail
cd "$(dirname "$0")/.."

# This box shares cores with live services; be polite, stay honest.
NICE="nice -n 15 ionice -c3"

fail() {
  echo "GATE FAIL: $*" >&2
  exit 1
}

echo "== gate: rustfmt"
$NICE cargo fmt --all --check || fail "rustfmt"

echo "== gate: clippy walls (-D warnings; sim walls via crates/sim-core/clippy.toml)"
$NICE cargo clippy --workspace --all-targets -- -D warnings || fail "clippy"

echo "== gate: native test suite (alloc_zero, replay, terrain_golden, protocol_golden, unit)"
$NICE cargo test --workspace --release || fail "cargo test"

echo "== gate: wasm build (sim-core + protocol -> wasm32-unknown-unknown)"
rustup target list --installed | grep -q '^wasm32-unknown-unknown$' \
  || fail "wasm32-unknown-unknown target not installed"
$NICE cargo build -p sim-core -p protocol --release --target wasm32-unknown-unknown \
  || fail "wasm build"

echo "== gate: test_parity_wasm (native vs wasm, byte-equal digests)"
command -v node >/dev/null || fail "node missing — parity gate cannot run"
native_out="$(mktemp)"
wasm_out="$(mktemp)"
trap 'rm -f "$native_out" "$wasm_out"' EXIT
$NICE cargo run -p sim-core --release --example probe > "$native_out" \
  || fail "native probe"
$NICE node ci/parity.mjs > "$wasm_out" || fail "wasm probe"
diff -u "$native_out" "$wasm_out" \
  || fail "test_parity_wasm: native and wasm digests differ"
grep -q '^parity ' "$native_out" || fail "probe output empty — parity not exercised"

echo "ALL GATES GREEN"
