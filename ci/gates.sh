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
$NICE cargo clippy -p client-wasm --target wasm32-unknown-unknown -- -D warnings \
  || fail "clippy (wasm bridge)"

echo "== gate: native test suite (alloc_zero, replay, terrain_golden, protocol_golden, snapshot_budget, content, bot smoke, unit)"
$NICE cargo test --workspace --release || fail "cargo test"

echo "== gate: wasm build (sim-core + protocol + client-wasm -> wasm32-unknown-unknown)"
rustup target list --installed | grep -q '^wasm32-unknown-unknown$' \
  || fail "wasm32-unknown-unknown target not installed"
$NICE cargo build -p sim-core -p protocol -p client-wasm --release --target wasm32-unknown-unknown \
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
grep -q '^combat ' "$native_out" || fail "probe output has no combat line — melee not exercised"

echo "== gate: client wasm bridge smoke (raw C ABI, the browser's calling path)"
$NICE node ci/client_smoke.mjs || fail "client bridge smoke"

echo "== gate: web bundle (npm ci + vite build; the wasm artifact must ride along)"
command -v npm >/dev/null || fail "npm missing — web gate cannot run"
mkdir -p web/public
cp target/wasm32-unknown-unknown/release/client_wasm.wasm web/public/client_wasm.wasm \
  || fail "client wasm artifact missing"
# --include=dev: this box exports NODE_ENV=production, which would silently
# omit vite — the build tool itself (a pass it didn't earn, trap list).
$NICE npm --prefix web ci --include=dev --no-audit --no-fund || fail "npm ci"
$NICE npm --prefix web run build || fail "vite build"
[ -f web/dist/client_wasm.wasm ] || fail "wasm artifact absent from web bundle"

# The only gate that runs the JS in a browser. Everything above tests the
# client's LOGIC natively or in node, which is why two hard boot bugs shipped
# green on 2026-07-31: a detached-buffer throw in WasmViews that stopped the
# client dead, and a terrain-worker race that killed the near ring while the
# far mesh still rendered (so screenshots looked fine). Both are invisible to
# every other gate here. Needs the release shard binary — build it first so a
# missing binary is a loud failure and never a skip.
echo "== gate: browser smoke (real shard, real WebTransport, real browser)"
$NICE cargo build -p server --bin shard --release || fail "shard build"
$NICE node ci/browser_smoke.mjs || fail "browser smoke"

echo "ALL GATES GREEN"
