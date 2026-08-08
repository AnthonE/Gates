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

# There are no tiers any more (operator, 2026-08-06: the browser client is cut,
# and then: "we dont need it locally").
#
# The tiers existed because two gates — `browser_smoke` and `vantages` — booted
# Chromium against swiftshader on a box with no GPU path for them and cost ~19
# minutes of the script's ~1,062 s. Both photographed the three.js client. That
# client is gone from the tree, so the expensive half of this file is gone with
# it and every gate below is code: deterministic, headless, minutes not tens of
# minutes. Nothing left to opt out of.
#
# An argument is still ACCEPTED and ignored, because `lanes.sh` passes
# `GATES_TIER=fast` and a hard error there would be a red gate reporting a
# vocabulary problem rather than a defect. It is a no-op, said out loud rather
# than silently: this script now has exactly one behaviour and one final line.
TIER="${1:-${GATES_TIER:-all}}"
case "$TIER" in
  all | fast | auto)
    [ "$TIER" = "all" ] || echo "note: tier '$TIER' is a no-op — the renderer tier was cut with the browser client; running every gate."
    ;;
  *) fail "unknown tier '$TIER' — the tiers are gone; pass nothing." ;;
esac

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
$NICE cargo test -p client --features render --lib --test tree --test fell --test look --test ghost \
  --test mob_mesh \
  || fail "native client suites"

# **The only wasm in this repo, and it is not a client** (operator,
# 2026-08-08: "we use desktop build no more web"). `sim-core` and the
# `protocol` it needs are built for a second, deliberately hostile target so
# the gate below can diff their state hashes against native byte for byte —
# CLAUDE.md wall 1's enforcement, which is worth exactly as much with no
# browser in existence as it was with one. `client-core` used to be built
# here too, for its C-ABI bridge; both are gone.
echo "== gate: wasm build (sim-core + protocol -> wasm32-unknown-unknown, the determinism target)"
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

# WHAT WAS CUT HERE, and what it costs (operator, 2026-08-06).
#
# Eleven gates were deleted with the browser client, in two groups.
#
# Three photographed or drove it and are simply gone with their subject:
# `ui_smoke` (7,243 lines, 1,911 assertions on `index.html` + `hud.js`),
# `browser_smoke` (a real shard through a real WebGL context) and `vantages`.
# Nothing about the native client is less proven for their absence — none of
# them ever looked at it.
#
# The other eight held a REAL invariant against the wrong renderer: `pine
# shape`, `clutter shape`, `haven shelter`, `waystation canopy`, `occupant
# volume`, `bump basis`, `prop photo` and the `web bundle` build. Their claim
# was "the mesh the client draws == the volume the server blocks", checked by
# importing `web/src/props.js`. That claim still matters and the native client
# is not covered by it: those gates would have stayed green forever while
# `crates/client/src/render/props.rs` drifted from `sim-core`, because they
# were reading a file nobody ships. Green against a dead renderer is worse
# than absent — it reads as coverage.
#
# Partly re-earned, natively, and this is why the loss is bounded rather than
# total: `crates/client/tests/tree.rs` already asserts the same invariant for
# trees (`every_variant_fits_the_volume_the_sim_blocks`,
# `every_variant_is_rooted_at_the_ground`), and it runs in the native client
# gate above. Not re-earned: the haven shelter, the waystation canopy, the
# clutter ring, and the occupant table for everything that is not a tree.
# That is an honest hole, named here rather than papered over — the shape of
# the replacement is `tests/tree.rs`, in Rust, against the mesh we actually
# draw.

echo "ALL GATES GREEN"
