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

# Tiers (operator, 2026-08-04). Everything above the renderer gates is code:
# deterministic, headless, and it finishes in a couple of minutes. The two
# renderer gates are neither — they boot Chromium against swiftshader on a box
# with no GPU path for them, render thousands of frames at roughly a frame a
# second, and together they are the overwhelming majority of this script's wall
# clock (measured 2026-08-04: 1,062 s green end to end, of which the renderer
# tier is the bulk; a single browser_smoke run is 8-10 min).
#
# The reason to tier rather than trim: a pass that changed a server config and
# three docs was paying 3.6 million probed pixels four times to prove it. The
# rule is the one CLAUDE.md already applies to lighting — the owner of a change
# pays its cost. A pass that touched the renderer runs the renderer gates; a
# pass that did not, need not, and `auto` decides that by reading the diff
# rather than by asking the builder to be honest about it.
#
#   all   (default, and what CI runs) every gate in this file
#   fast  code gates only — stops before browser smoke and vantages
#   auto  code gates, plus the renderer tier IF the diff touches renderer paths
#
# What this deliberately does NOT do is let a short run claim a long one's
# result: `fast` and a skipping `auto` print their own final line, never
# "ALL GATES GREEN". A pass reporting gates green must say which tier it ran.
TIER="${1:-${GATES_TIER:-all}}"
case "$TIER" in
  all | fast | auto) ;;
  *) fail "unknown tier '$TIER' — one of: all (default), fast, auto" ;;
esac

# Renderer paths, as a question about the diff. Both halves are read: what is
# committed on this branch and what is still in the working tree, because a
# builder runs this before committing as often as after.
renderer_touched() {
  {
    git diff --name-only HEAD 2>/dev/null || true
    git diff --name-only origin/main...HEAD 2>/dev/null || true
  } | grep -qE '^(web/|assets/textures/|ci/browser_smoke\.mjs|ci/vantages\.mjs)'
}

RUN_RENDERER=1
if [ "$TIER" = "fast" ]; then
  RUN_RENDERER=0
elif [ "$TIER" = "auto" ]; then
  if renderer_touched; then RUN_RENDERER=1; else RUN_RENDERER=0; fi
fi

# Cheapest gate in the file — pure text, no build — so it runs first: a knob
# that disagrees with its registry entry should not cost a ten-minute compile
# to discover. `CLAUDE.md` calls `DECISIONS.md` authoritative on every knob,
# and on 2026-08-02 `BUMP_MAX_SLOPE` shipped at 0.55 while its §open row still
# read 1.0 — nine gates green over the disagreement, caught only by a judge
# reading the diff. This is that reading, mechanized.
echo "== gate: knob registry (DECISIONS.md §open declares what the code ships)"
command -v node >/dev/null || fail "node missing — knob registry gate cannot run"
$NICE node ci/knob_registry.mjs || fail "knob registry"

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
grep -q '^bags ' "$native_out" || fail "probe output has no bags line — respawn-on-bag not exercised"
# The bags line carries a COUNT before its digest, and this reads it. Two
# targets can agree byte-for-byte about a path that never ran on either —
# a digest is only evidence of parity, never of coverage. `probe_bags`
# counts the deaths that woke on a bag; zero means the fixture stopped
# reaching the scan and the parity claim above it is empty.
bag_wakes="$(awk '/^bags /{print $5}' "$native_out")"
[ -n "$bag_wakes" ] && [ "$bag_wakes" -gt 0 ] \
  || fail "test_parity_wasm: the bags probe woke nobody on a bag (count '$bag_wakes') — the respawn scan is not actually on the parity surface"

echo "== gate: client wasm bridge smoke (raw C ABI, the browser's calling path)"
$NICE node ci/client_smoke.mjs || fail "client bridge smoke"

# The terrain bump's gradient reconstruction, as arithmetic. The defect it
# holds — a heightfield rendering its own triangulation — is a discontinuity in
# a formula, so it is evaluated on both sides of one triangle edge rather than
# photographed. No GPU, no shard, no threshold that moves with a driver.
echo "== gate: bump basis (world-XZ gradient is continuous across a triangle edge)"
$NICE node ci/bump_basis.mjs || fail "bump basis"

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
if [ "$RUN_RENDERER" = "0" ]; then
  echo "== renderer tier NOT run (tier: $TIER) — browser smoke, vantages"
  echo "CODE GATES GREEN — renderer tier skipped, so this is NOT 'all gates green'."
  echo "Run './ci/gates.sh' with no argument before merging anything that touches the renderer."
  exit 0
fi

echo "== gate: browser smoke (real shard, real WebTransport, real browser)"
$NICE cargo build -p server --bin shard --release || fail "shard build"
$NICE node ci/browser_smoke.mjs || fail "browser smoke"

# The one gate that looks anywhere other than the beach. `browser_smoke` above
# fires every material assertion from one spawn whose own 40 m ring is 100%
# under 10 degrees of tilt, on one seed — three defects shipped green through
# that blind spot in as many passes. This runs the same probes at a 69 degree
# face, above the snow line, and on a second seed, one tab at a time so a
# loaded box cannot turn it into a clock.
echo "== gate: vantages (the material off the beach: slope, snow line, second seed)"
$NICE node ci/vantages.mjs || fail "vantages"

echo "ALL GATES GREEN"
