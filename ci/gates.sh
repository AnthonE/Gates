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

# The toolchain, resolved ONCE and named when it is absent.
#
# `rustup` installs cargo into `$HOME/.cargo/bin` and puts it on PATH from
# `$HOME/.cargo/env` — which Ubuntu's `.bashrc` sources only for INTERACTIVE
# shells and `.profile` only for LOGIN shells. A runner respawned into a shell
# that is neither has a working, installed cargo it cannot see, and on
# 2026-08-04 that is exactly what happened: the steward restarted a dead lane
# runner, and the rustfmt gate died as
#
#   ionice: failed to execute cargo: No such file or directory
#
# — an error naming `ionice`, 110 lines into the run, for a toolchain that was
# installed and working the whole time. Sourcing the file rustup wrote is what
# an interactive shell already does; it enables the gate and changes no
# assertion in it. CLAUDE.md's rule for the sibling case (`wasm32-unknown-
# unknown` missing) is the same one: a wall that cannot run is not a wall, so
# make it run rather than skip it.
if ! command -v cargo >/dev/null 2>&1 && [ -f "$HOME/.cargo/env" ]; then
  # shellcheck disable=SC1091
  . "$HOME/.cargo/env"
fi
# And if it is genuinely missing, say so HERE — before any gate — rather than
# leaving each shell-out to report it in whatever words its process prefix
# happens to use. Every binary this script execs, in one place.
for _tool in cargo node npm; do
  command -v "$_tool" >/dev/null 2>&1 ||
    fail "$_tool is not on PATH — every gate that shells out to it cannot run." \
      "This is a missing capability, not a defect in the tree: install it (cargo:" \
      "rustup, and source \$HOME/.cargo/env) rather than skipping the gates that need it."
done

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
# The committed half needs a base to diff against, and `origin/main` is not it:
# it does not resolve in a worktree cut for a parallel lane (operator, 2026-08-04,
# three-lane build), and a base that fails to resolve makes this half return
# NOTHING — so a committed renderer change would answer "no" and skip the
# renderer tier silently. That is the exact silent-skip class the trap list
# calls the worst bug in the file. Try the bases in order of trustworthiness and
# take the first that resolves; local `main` is present in every worktree.
renderer_base() {
  local b
  for b in "$(git merge-base HEAD main 2>/dev/null || true)" \
           "$(git merge-base HEAD origin/main 2>/dev/null || true)"; do
    [ -n "$b" ] && { echo "$b"; return 0; }
  done
  return 1
}

# Every path under `web/` is a renderer path EXCEPT the three HUD files below.
# The carve-out was proposed by the ui lane on 2026-08-04, built before it was
# asked for, and ARMED by the operator the same day (`DECISIONS.md` §Spoken).
# A one-line `<div>` move now pays `ui_smoke` at ~0.8 s instead of
# `browser_smoke` + `vantages` at ~19 min.
#
# It is armed on landed evidence, not on the saving: `ci/ui_smoke.mjs` asserts
# every `index.html`/`hud.js` contract `browser_smoke` holds, as a strict
# superset, with the coverage table in that file's header keyed to
# `browser_smoke` line numbers — and eleven mutants of those two files were run
# against it, all eleven red. The first attempt at this carve-out was judged
# FAIL precisely because the coverage was NOT equivalent: `#vitals` at inline
# `display:"block"` (`browser_smoke:1745`) was a value `ui_smoke` read and
# discarded, so the mutation escaped both gates.
#
# THE STANDING RULE, and it is the whole reason this is safe: a path joins this
# exemption list ONLY in a commit that also extends `ui_smoke` to cover what
# that path can break. Subtracting a path from the question below subtracts a
# gate from the merge — `auto` is the tier the loop merges on — so the list is
# the operator's, never a lane branch's.
renderer_touched() {
  local base
  base=$(renderer_base) || {
    # No base at all: cannot answer the question, so do NOT answer it "no".
    echo "  tier: no merge base against main — running the renderer tier rather than guessing." >&2
    return 0
  }
  local ui_only='^(web/index\.html|web/src/hud\.js|web/src/input\.js)$'
  {
    git diff --name-only HEAD 2>/dev/null || true
    git diff --name-only "$base"...HEAD 2>/dev/null || true
  } | grep -vE "$ui_only" \
    | grep -qE '^(web/|assets/textures/|ci/browser_smoke\.mjs|ci/vantages\.mjs)'
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

# Also pure text, also no build. The sim proves the haven pad concentrates
# containers; this proves the containers it concentrates are the richer kind.
# That half lives entirely in content/loot.toml, which no Rust gate reads —
# no verb opens a container yet — so without this the gradient is one
# rebalance away from being the defect the coast road already had.
echo "== gate: haven prize (the destination outpays the route, in content)"
$NICE node ci/haven_prize.mjs || fail "haven prize"

# The depot document, not the build. A depot with a bad path, a launch.exec
# that is not a staged file, or a placeholder the launcher does not fill
# produces a game that installs perfectly and never starts — on a player's
# machine, which is the only place it would be noticed. No compiler needed:
# it stages fakes and asserts the rules in about a second.
echo "== gate: depot packaging (the scry launcher's seam, docs/LAUNCHER.md §3)"
$NICE python3 ci/depot.py --self-test || fail "depot packaging"

# The other half of that seam. A shard list is the one document where a wrong
# value is an invitation to a server that turns the player away: an address
# the client cannot parse, a cap above the sim's own MAX_PLAYERS, or a player
# count nobody measured. It also reads the client parser's constants, so the
# generator and the game cannot drift apart on what they will accept.
echo "== gate: shard list (scry-shardlist-v1, docs/LAUNCHER.md §6)"
$NICE python3 ci/shardlist.py --self-test || fail "shard list"

echo "== gate: rustfmt"
$NICE cargo fmt --all --check || fail "rustfmt"

echo "== gate: clippy walls (-D warnings; sim walls via crates/sim-core/clippy.toml)"
$NICE cargo clippy --workspace --all-targets -- -D warnings || fail "clippy"
$NICE cargo clippy -p client-wasm --target wasm32-unknown-unknown -- -D warnings \
  || fail "clippy (wasm bridge)"

echo "== gate: native test suite (alloc_zero, replay, terrain_golden, protocol_golden, snapshot_budget, content, bot smoke, unit)"
$NICE cargo test --workspace --release || fail "cargo test"

# The native client, which the two gates above DO NOT SEE. `render` is off by
# default (`crates/client/Cargo.toml`: a default-on Bevy would put minutes onto
# a ~20 s clippy that runs in every lane on every health check), so
# `--workspace` compiles none of it — `NOW.md` §0v item 3 named that hole when
# the client was four screens, and it is now the whole game surface: every
# in-world verb, both picks, the ghost, the structures renderer, chat, the map
# and the death screen.
#
# Two commands, both cheap once Bevy is in the cache, and this is `RENDER.md`
# R0's probe. It is NOT a visual gate and does not pretend to be: it compiles
# the render path under `-D warnings` and runs the three renderer-tier suites
# (`tree`, `fell`, `look`), none of which needs a GPU or a window. What
# photographs these screens is still owed.
#
# Bevy's default features pull `wayland-client`, `alsa` and `libudev` through
# `winit`, `bevy_audio` and `bevy_gilrs`, and this client uses only the first —
# a box without those dev packages fails here at a build script rather than at
# a test. `alsa` in particular is a capability we are ASKING for and do not
# need (`CLAUDE.md`: the second question), and trimming the feature set is the
# fix; until then the requirement is stated here rather than discovered.
#
# `--lib` rides along with the three suites because the client's unit tests are
# behind the same feature: `cargo test --workspace` above compiles the crate
# WITHOUT `render`, so everything under `render::` is cfg'd out of it. Without
# this flag `render::loading`'s tests — which assert that a world is not loaded
# until the server has said where — are compiled by the clippy line above and
# run by nothing.
echo "== gate: native client (--features render: clippy + lib + the renderer-tier suites)"
echo "   (needs libwayland-dev + libasound2-dev + libudev-dev — Bevy defaults, not ours)"
$NICE cargo clippy -p client --features render --all-targets -- -D warnings \
  || fail "clippy (native client)"
$NICE cargo test -p client --features render --lib --test tree --test fell --test look \
  || fail "native client suites"

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
# The terrain and sites lines, asserted by COUNT rather than presence. The
# `diff` above cannot see an empty loop: drop the seed table on both sides and
# the two outputs still match exactly, so worldgen leaves the parity surface
# with every gate green. Three is the length of TERRAIN_SEEDS, duplicated by
# hand in examples/probe.rs and ci/parity.mjs; a divergence between those two
# shows as the diff, a truncation of both shows here.
for line in terrain sites; do
  n="$(grep -c "^$line " "$native_out" || true)"
  [ "$n" = "3" ] \
    || fail "test_parity_wasm: expected 3 '$line' lines from the probe, got '$n' — worldgen is not on the parity surface"
done
# The bags line carries a COUNT before its digest, and this reads it. Two
# targets can agree byte-for-byte about a path that never ran on either —
# a digest is only evidence of parity, never of coverage. `probe_bags`
# counts the deaths that woke on a bag; zero means the fixture stopped
# reaching the scan and the parity claim above it is empty.
#
# Since wire v16 this count is strictly stronger than it was. A death no
# longer wakes a body by itself: it lays it on the death screen, and only a
# `Command::Respawn` from that player reaches the scan. So a nonzero here
# proves the whole chain — the death, `World::die`, the corpse ticks, the
# answer, and the bag scan — ran on BOTH targets, not just the last link.
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

# The prop photograph's triplanar blend, as arithmetic. A triplanar blend loses
# 42% of a source's contrast at the three-way point unless the deviation is
# variance-normalized, and contrast is the ENTIRE reason the photograph was
# wired — so the correction is gated by a closed-form identity here rather than
# by a screenshot ten minutes downstream. Reads the shipped knobs out of
# `materials.js` rather than carrying its own copy.
echo "== gate: prop photo (mean-preserving ratio, symmetric clamp, variance-preserving triplanar)"
$NICE node ci/prop_photo.mjs || fail "prop photo"

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

# The pine's silhouette, and the sim constant derived from its width. Runs
# here and not earlier because it imports the shipped builder out of
# `web/src/props.js`, which needs three from the install above — a geometry
# gate that built its own tree to score would pass forever while the tree
# changed underneath it. No GPU, no shard: a vertex buffer is arithmetic.
echo "== gate: pine shape (silhouette counts + the SPAWN_CLEAR_M coupling)"
$NICE node ci/pine_shape.mjs || fail "pine shape"

# The ground population below the scatter grid, same standard and same place
# in the order for the same reason: it imports `web/src/clutter.js`. ART.md
# rule 4 itself is measured natively (`crates/sim-core/tests/clutter.rs`, the
# largest bare disc inside 15 m); this holds the drawn half to the placed half
# — three constants read from both languages, the kind table checked by name
# against the Rust enum, and the ring's triangle fleet ASSERTED against a
# declared share of DESIGN §9's budget rather than printed.
echo "== gate: clutter shape (the near-ground population + its fleet budget)"
$NICE node ci/clutter_shape.mjs || fail "clutter shape"

# The haven pad's greybox, same standard and same reason it sits here: it
# imports the shipped box list out of `web/src/props.js`. Three claims no
# `cargo test` can reach, because they straddle the Rust/JS line — the mesh
# fits the slot `HAVEN_SHELTER_HALF_M` reserved, the doorway is passable and
# the walls enclose something (asserted in both directions), and the peak
# clears a full-scale pine. Arithmetic over a box list: no GPU, no shard.
echo "== gate: haven shelter (the greybox fits the slot sim-core placed)"
$NICE node ci/haven_shelter.mjs || fail "haven shelter"

# The lesser tier's greybox, the sibling above's opposite claim. The shelter
# gate proves a room — walls that enclose, a doorway that is passable and stops
# being passable when filled in. This proves a roof: solid on exactly one side,
# open on the other three to the sim's own capsule, covered overhead, and NOT
# the pad's building at 0.6 scale (under half its height, squatter in aspect,
# timber against stone). Same place in the order for the same reason as its
# three neighbours — it imports the shipped box list out of `web/src/props.js`.
# Arithmetic over nine boxes: no GPU, no shard.
echo "== gate: waystation canopy (the second greybox is not the first, smaller)"
$NICE node ci/waystation_canopy.mjs || fail "waystation canopy"

# The volume the server blocks against the mesh the client draws, for every
# occupant rather than the one the shelter gate covers. `OCCUPANT_R_M` and
# `OCCUPANT_TOP_M` say of themselves that they are read off `ARCHETYPES`, and
# nothing checked that sentence: the Rust const-asserts hold the table equal
# to `occupant_volume()`'s match, but both live in one file and move together
# under one edit. Sits here for the same reason as its three neighbours — it
# imports the shipped builders out of `web/src/props.js`. Vertex buffers are
# arithmetic: no GPU, no shard.
echo "== gate: occupant volume (the blocked cylinder is the drawn mesh)"
$NICE node ci/occupant_volume.mjs || fail "occupant volume"

# The interaction surface: a real browser, no renderer. Sits in the CODE tier
# because it costs under a second — it renders no frames, creates no WebGL
# context, loads no wasm and starts no shard, so none of what makes the two
# gates below expensive applies. Here rather than earlier because playwright is
# a `web/` devDependency and comes from the install above.
#
# It is the coverage the armed carve-out above rests on: `renderer_touched`
# exempts exactly `index.html`, `hud.js` and `input.js`, and every other path
# under `web/` still schedules the renderer tier. This gate earns that on
# what it asserts: the composer that must swallow a keystroke so "w" is a
# letter and not a step forward, the death screen that must answer once, the
# chat line that goes in as another player's TEXT and never as markup, and the
# vitals stack whose argument order and row order disagree by design — the
# positional-payload shape CLAUDE.md's trap list names as where the reference
# ecosystem actually bled. Every one of those was a comment with no gate.
#
# It also holds, deliberately, a superset of every `index.html`/`hud.js`
# contract `ci/browser_smoke.mjs` asserts (group A and the inline-style checks
# in F). That is not redundancy for its own sake: it is the evidence the
# §open carve-out proposal needs, measured rather than promised.
echo "== gate: ui smoke (HUD, hotbar, composer, chat, vitals, death — real events, no renderer)"
$NICE node ci/ui_smoke.mjs || fail "ui smoke"

# The only gate that BOOTS the client — a real shard, the wasm bridge, a WebGL
# context, the world drawn. `ui_smoke` above is also a browser, but on a page
# where the game never started, and it cannot replace this: two hard boot bugs
# shipped green on 2026-07-31 because everything else tested the client's LOGIC
# natively or in node — a detached-buffer throw in WasmViews that stopped the
# client dead, and a terrain-worker race that killed the near ring while the
# far mesh still rendered (so screenshots looked fine). Both are invisible to
# every other gate here, `ui_smoke` included. Needs the release shard binary —
# build it first so a missing binary is a loud failure and never a skip.
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
