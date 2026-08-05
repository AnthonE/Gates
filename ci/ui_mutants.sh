#!/usr/bin/env bash
# The mutants `ci/ui_smoke.mjs` claims to catch — as a script, not as a
# sentence in a commit body.
#
# WHY THIS EXISTS. Three passes running have ended with a line like "nine
# mutants run, nine red" in `NOW.md`, and the judge of
# `pass-20260805-074623-03` named the problem exactly: that is "the one claim a
# judge cannot re-run". It asked for the list to be written into a findings
# note. A note would still be prose. This is the same list, executable — and it
# is how the two survivors that judge DID find (M7 and M9 below) were confirmed
# dead rather than assumed dead.
#
# It is deliberately NOT wired into `ci/gates.sh`. It edits shipped source
# files in place and restores them with `git checkout --`, which is a fine
# thing for a human or a loop to run against a committed tree and a bad thing
# to do inside the merge gate. It refuses outright on a dirty tree for the same
# reason: the restore would take your uncommitted work with it.
#
#   ./ci/ui_mutants.sh          run them all
#
# A mutant that comes back GREEN is a hole in `ui_smoke`, and closing it is the
# next pass's work. A mutant that comes back PATCH-FAILED is stale — the line
# it edits has moved or changed, and it is asserting nothing until it is
# re-anchored. Both are failures of this script and it exits nonzero on either.
set -uo pipefail
cd "$(dirname "$0")/.."

if ! command -v cargo >/dev/null 2>&1 && [ -f "$HOME/.cargo/env" ]; then
  # shellcheck disable=SC1091
  . "$HOME/.cargo/env"
fi
command -v node >/dev/null 2>&1 || {
  echo "node is not on PATH — the mutants cannot run" >&2
  exit 1
}
if [ -n "$(git status --porcelain)" ]; then
  echo "REFUSING: the tree is dirty. This script restores files with" >&2
  echo "\`git checkout --\`, which would discard uncommitted work. Commit first." >&2
  exit 1
fi

red=0
green=0
broken=0

# run_mut <name> <file> <from> <to>
run_mut() {
  local name="$1" file="$2" from="$3" to="$4"
  if ! python3 - "$file" "$from" "$to" <<'PY'
import sys, io
f, a, b = sys.argv[1], sys.argv[2], sys.argv[3]
s = io.open(f, encoding="utf8").read()
n = s.count(a)
if n != 1:
    sys.stderr.write("the anchor matched %d times, expected exactly 1\n" % n)
    sys.exit(1)
io.open(f, "w", encoding="utf8").write(s.replace(a, b))
PY
  then
    echo "  PATCH-FAILED  $name"
    broken=$((broken + 1))
    git checkout -- "$file"
    return
  fi
  if node ci/ui_smoke.mjs >/tmp/ui_mutant.out 2>&1; then
    echo "  *** GREEN ***  $name   <-- SURVIVED: ui_smoke has a hole here"
    green=$((green + 1))
  else
    echo "  red           $name"
    red=$((red + 1))
  fi
  git checkout -- "$file"
}

echo "== ui_smoke mutants (each: mutate one shipped line, run the gate, restore)"

# --- the map's geometry -----------------------------------------------------
# M7 and M12 are the judge's ranked fix 1 on `pass-20260805-074623-03` and its
# neighbour. M7 shipped GREEN at 635 checks: both height-field sweeps stub the
# sampler flat (`splatAt: () => GRASS_ONLY`), so a map whose SHAPE is right and
# whose biome colours are mirrored east-west was invisible to all of them.
run_mut "M7  map.js mirrors the world x handed to the sampler" web/src/map.js \
  'const x = src.x0 + i * src.step;' \
  'const x = src.x0 + (size - 1 - i) * src.step;'
run_mut "M12 map.js transposes the sampler's (x, z)" web/src/map.js \
  'src.splatAt(h, src.moistAt(x, z), slope)' \
  'src.splatAt(h, src.moistAt(z, x), slope)'

# M9 is the judge's ranked fix 2, and the sharpest one in the file: `mapDir`
# was pinned by M11's fix, so this leaves `mapDir` exact and hard-codes the
# TRIANGLE's own nose instead. Green at 635 checks, one line below where M11
# was closed — closing an instance is not closing a class.
run_mut "M9  hud.js hard-codes the marker's nose north" web/src/hud.js \
  'T[0] = px + dx * MAP_MARKER_PX;
    T[1] = py + dy * MAP_MARKER_PX;' \
  'T[0] = px + 0 * MAP_MARKER_PX;
    T[1] = py + -1 * MAP_MARKER_PX;'

# --- the build prompt's positional reads ------------------------------------
# The stride-8 def row is the shape `CLAUDE.md`'s trap list names: the right
# value in the wrong position, every field a number, byte-goldens blind.
run_mut "M13 describePiece swaps the (item, quantity) pair" web/src/interact.js \
  'const item = defs[b + PIECE_COST_AT + k * PIECE_COST_STRIDE];
    const qty = defs[b + PIECE_COST_AT + k * PIECE_COST_STRIDE + 1];' \
  'const qty = defs[b + PIECE_COST_AT + k * PIECE_COST_STRIDE];
    const item = defs[b + PIECE_COST_AT + k * PIECE_COST_STRIDE + 1];'
run_mut "M19 describeDeploy reads b+1 for the item id" web/src/interact.js \
  'const item = defs[row * DEPLOY_DEF_STRIDE + 3];' \
  'const item = defs[row * DEPLOY_DEF_STRIDE + 1];'
run_mut "M18 the shape labels are renumbered (wall <-> doorway)" web/src/interact.js \
  '["foundation", "wall", "doorway", "floor", "stairs", "roof"]' \
  '["foundation", "doorway", "wall", "floor", "stairs", "roof"]'
run_mut "M16 the shortfall names the LAST unmet ingredient" web/src/interact.js \
  'if (gap > 0 && !out.need)' \
  'if (gap > 0)'
run_mut "M17 exactly-enough counts as short (off-by-one)" web/src/interact.js \
  'if (gap > 0 && !out.need)' \
  'if (gap >= 0 && !out.need)'
run_mut "M22 promptForBuild drops the shortfall" web/src/interact.js \
  'return pick.need ? `${head} · NEED ${pick.need.toUpperCase()}` : head;' \
  'return head;'

# --- which verb wins the one row --------------------------------------------
run_mut "M14 centrePrompt puts the swing ahead of E" web/src/interact.js \
  '    promptForBuild(buildPick) ||
    promptFor(interactPick) ||
    promptForSwing(swingPick) ||
    promptForRepair(repairPick)' \
  '    promptForBuild(buildPick) ||
    promptForSwing(swingPick) ||
    promptFor(interactPick) ||
    promptForRepair(repairPick)'
run_mut "M15 centrePrompt puts E ahead of the build pick" web/src/interact.js \
  '    promptForBuild(buildPick) ||
    promptFor(interactPick) ||
    promptForSwing(swingPick) ||
    promptForRepair(repairPick)' \
  '    promptFor(interactPick) ||
    promptForBuild(buildPick) ||
    promptForSwing(swingPick) ||
    promptForRepair(repairPick)'

# --- and that the client actually routes through all of it ------------------
run_mut "M21 the build pick ignores build.on" web/src/main.js \
  '        build.on ? selDesc() : null,' \
  '        selDesc(),'
run_mut "M20 B stops redrawing the centre prompt" web/src/main.js \
  '      // The centre hint changes owner on this key — into and out of the build
      // row — so it is redrawn on the keypress and not up to 250 ms later on
      // the HUD timer. A hint that lags the ghost it describes is the defect.
      updatePrompt();
' ''
run_mut "M23 the wheel stops redrawing the centre prompt" web/src/main.js \
  '      // The wheel changes WHICH piece the hint names, so it redraws here too.
      updatePrompt();
' ''

# --- the piece's hp half ----------------------------------------------------
# The anchor is the positional payload here: both reach checks measure to it,
# so a swapped `half` term is the right value at the wrong corner and every
# byte-golden stays green while U and repair refuse at a range the server
# accepts. M24/M25 are the two swaps that matter.
run_mut "M24 the west edge anchors at the cell centre" web/src/interact.js \
  'out[0] = loc === LOC_EDGE_W ? x0 : x0 + half;' \
  'out[0] = x0 + half;'
run_mut "M25 the two edge anchors are exchanged" web/src/interact.js \
  'out[0] = loc === LOC_EDGE_W ? x0 : x0 + half;
  out[1] = loc === LOC_EDGE_N ? z0 : z0 + half;' \
  'out[0] = loc === LOC_EDGE_N ? x0 : x0 + half;
  out[1] = loc === LOC_EDGE_W ? z0 : z0 + half;'
# M26 is the bug this pass fixed, put back verbatim: a free variable in a file
# no gate can execute. It must be caught by the scanner, not by the call.
run_mut "M26 main.js reads an undeclared REACH again" web/src/main.js \
  '  const pieceAt = { x: 0, z: 0 };' \
  '  const pieceAt = { x: 0, z: REACH };'
run_mut "M27 the reach bound is not applied to the piece scan" web/src/interact.js \
  '  let bestD = INTERACT_REACH_M * INTERACT_REACH_M;
  for (const rec of world.recs) {' \
  '  let bestD = Infinity;
  for (const rec of world.recs) {'
# The refusal table and the repair/raid discriminator: the two reads that told
# a player the wrong thing about their own base.
#
# M28 was anchored in `interact.js` and went stale the day the four tables moved
# to `web/src/refusals.js` — it matched zero times, which this script reports as
# PATCH-FAILED and exits nonzero on, correctly: an anchor that finds nothing is
# asserting nothing. Re-anchored, not deleted.
run_mut "M28 the refusal table loses its last sentence" web/src/refusals.js \
  '  "nothing to upgrade into",
  "not damaged",' \
  '  "nothing to upgrade into",'
run_mut "M29 a repair announces itself as a breach" web/src/interact.js \
  'return flags & APPLIED_HIT_BIT ? `breaching ${left}/${max}` : `repaired ${left}/${max}`;' \
  'return `breaching ${left}/${max}`;'
run_mut "M30 structNews draws a bar off a missing denominator" web/src/interact.js \
  '  if (max === 0) return "";' \
  '  if (max === -1) return "";'

# M31-M34: the ORDER of a refusal table, which every check on it up to
# 2026-08-05 passed straight through. M31 is verbatim the mutation the judge of
# pass-20260805-111501-02 ran to prove the hole — it reported `1597 checks
# passed` while a player placing on a missing hearth read "no door there".
# The Rust side and the JS side are separate mutants on purpose: the wall has
# to hold whichever end moved, because either end moving is the same bug.
# A true SWAP, not a single renumber: the codes stay contiguous 0..12 and the
# counts stay equal, so contiguity and length both pass and only the meaning
# wall is left standing.
run_mut "M31 the sim exchanges REFUSE_D_HEARTH and REFUSE_D_DOOR" crates/sim-core/src/deploy.rs \
  'pub const REFUSE_D_HEARTH: u32 = 10;
/// A use request named an address holding no door.
pub const REFUSE_D_DOOR: u32 = 11;' \
  'pub const REFUSE_D_HEARTH: u32 = 11;
/// A use request named an address holding no door.
pub const REFUSE_D_DOOR: u32 = 10;'
run_mut "M32 two deploy sentences are transposed" web/src/refusals.js \
  '  "no hearth there",
  "no door there",' \
  '  "no door there",
  "no hearth there",'
run_mut "M33 two move sentences are transposed" web/src/hud.js \
  '  "that slot is not there",
  "there is nothing there",' \
  '  "there is nothing there",
  "that slot is not there",'
# M34 is the meaning wall's own vacuity: a keyword two sentences share cannot
# tell those two apart, so it would let them swap. Softening one is exactly how
# a future pass would "fix" M31 without fixing anything.
run_mut "M34 a refusal keyword is softened to one three sentences share" ci/ui_smoke.mjs \
  '      HEARTH: "no hearth",' \
  '      HEARTH: "hearth",'

# --- the repair verb's store bit -------------------------------------------
# M35-M40. `client_action_repair(deploy, cx, cz, level, loc)` reaches two
# stores through one address, because a door and its doorway occupy the
# IDENTICAL (cx, cz, level, loc). Every argument is a u32, so none of these six
# touches the encoder, moves a golden, or changes a state hash — the whole
# class is invisible to `test_protocol_golden` and `test_replay` alike, which
# is CLAUDE.md's positional-payload trap with the discriminator in front.
run_mut "M35 the store bit becomes a literal, so every repair mends a built piece" web/src/main.js \
  'ex.client_action_repair(best.store, best.cx' \
  'ex.client_action_repair(0, best.cx'
run_mut "M36 the repair address transposes cx and cz" web/src/main.js \
  'best.store, best.cx, best.cz, best.level, best.loc' \
  'best.store, best.cz, best.cx, best.level, best.loc'
# The tie is the ONLY case where the two stores are ambiguous, and it is the
# case a raid produces: the door in its doorway. Flipping who wins it sends
# every door repair at the doorway behind the door.
run_mut "M37 the door loses its own doorway's tie" web/src/interact.js \
  'if (d2 < bestD || (d2 === bestD && out.found && store > out.store)) {' \
  'if (d2 < bestD) {'
# The two stores read from one source: the store bit is then always right for
# the pieces and always wrong for the deployables, and no address changes.
run_mut "M38 the deploy store is fed from the piece store" web/src/main.js \
  '    repairWorld.deploys = deployRecs.values();' \
  '    repairWorld.deploys = pieceRecs.values();'
# Repair ranks last so it can never displace a verb the player is aiming at.
# Promoting it is a one-token move that no other check in the file would see.
run_mut "M39 repair outranks the three verbs resolved off the crosshair" web/src/interact.js \
  '    promptForBuild(buildPick) ||
    promptFor(interactPick) ||
    promptForSwing(swingPick) ||
    promptForRepair(repairPick)' \
  '    promptForRepair(repairPick) ||
    promptForBuild(buildPick) ||
    promptFor(interactPick) ||
    promptForSwing(swingPick)'
# The mode conflict is held by the ORDER of two branches in one if/else chain,
# not by a condition on either — so the mutation that breaks it is a move, and
# it leaves both branches present and individually correct.
run_mut "M40 R stops raising the build level and falls through to repair" web/src/main.js \
  '    } else if (build.on && (e.code === "KeyR" || e.code === "KeyF")) {' \
  '    } else if (e.code === "KeyF") {'

# --- the getting-started checklist (ui_smoke group Y) -----------------------
# The one that matters most: correct descriptions in the WRONG ORDER. Every
# count, every uniqueness check and every DOM read stays green — the drawn rows
# are compared against the same table that drew them — and the client tells a
# new player that B crafts and C builds. This is the exact shape the judge of
# pass-20260805-111501-02 landed on the refusal tables, one surface over, and
# only the per-code keyword wall can see it.
run_mut "M41 the checklist transposes B and C" web/src/hud.js \
  '  { code: "KeyC", what: "craft something" },
  { code: "KeyB", what: "enter build mode" },' \
  '  { code: "KeyC", what: "enter build mode" },
  { code: "KeyB", what: "craft something" },'
# Drawing the rows backwards: the table is untouched and correct, so this is
# invisible to everything except the positional read of the DOM.
run_mut "M42 the constructor draws the checklist bottom-up" web/src/hud.js \
  '    for (const t of LEARN_TASKS) {' \
  '    for (const t of [...LEARN_TASKS].reverse()) {'
# A row struck through by position rather than by code — the transposed payload
# in the one place this file addresses a list by number.
run_mut "M43 learnUse checks off the first open row, not the pressed one" web/src/hud.js \
  '    const row = this.learnRow.get(code);' \
  '    const row = [...this.learnRow.values()].find((r) => !r.classList.contains("ldone"));'
# The repeat guard. Without it a player who presses B twice retires two rows
# worth of counter and the panel leaves the screen with rows still open.
run_mut "M44 learnUse double-counts a repeat" web/src/hud.js \
  '    if (!row || row.classList.contains("ldone")) return false;' \
  '    if (!row) return false;'
# show() stops reading learnLeft: the completed checklist comes back on every
# join and there is no key to dismiss it.
run_mut "M45 show() puts the completed checklist back up" web/src/hud.js \
  '    this.learn.style.display = this.learnLeft > 0 ? "block" : "none";' \
  '    this.learn.style.display = "block";'
# The one-liner that reads as tidier code and quietly breaks the whole law: a
# single mark at the top of the dispatch strikes rows through for keys the
# composer swallowed and keys eatsKey refused.
run_mut "M46 main.js marks the checklist from e.code at one call site" web/src/main.js \
  '      hud.learnUse("KeyE");' \
  '      hud.learnUse(e.code);'
# A key that stops being classified. The forward walk is the half that closes
# the CLASS rather than the ten instances someone remembered to pin.
run_mut "M47 a bound key leaves the checklist unclassified" web/src/hud.js \
  '  { code: "KeyL", what: "lock a door" },' \
  ''
# R in build mode raises a floor; marking the row there is a checklist that
# lies about what the player just did. The guard is the ORDER of the two
# branches, so the mutation is a move and both branches stay individually
# correct — M40's shape, one law over.
run_mut "M48 the repair row is marked from the build-level branch" web/src/main.js \
  '      build.level = Math.max(0, Math.min(MAX_LEVEL, build.level + d));' \
  '      build.level = Math.max(0, Math.min(MAX_LEVEL, build.level + d));
      hud.learnUse("KeyR");'

echo
echo "mutants: $red red, $green survived, $broken stale"
if [ -n "$(git status --porcelain)" ]; then
  echo "REFUSING TO PASS: the tree did not come back clean — a restore failed." >&2
  exit 1
fi
[ "$green" = "0" ] && [ "$broken" = "0" ] || exit 1
echo "every mutant was caught"
