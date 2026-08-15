# Branch notes — `claude/recent-commits-review-t8ps2v`

**The operator's 2026-08-15 playtest, worked by three lanes in one tree, plus
a design pass for the fourth.** Written by the run's integration and doc
owner; this replaces the `loop/scatter-splat-mix` note, which said it carried
no handoff. The loop harness did not run this — the lanes were driven
directly and the operator commits.

## What landed

**§0kit (content + sim + server).** `[[spawn_kit]]` is two rows — `item.rock`
×1, `item.torch` ×1 — and the four swung nodes' `hand` rows are **deleted**
rather than zeroed, because `validate.rs:110` refuses `hand = 0` while
`bake.rs` initialises `hand_yield: 0`, so the absent row is the content-only
move. `gather::swing` now refuses a swing the node pays nothing for, above
`find_or_insert` and above the budget spend. `World::wake` re-grants the kit,
so the rule is *once per body*, not once per login. `shard.toml`'s new
`dev_spawn_kit` keeps the retired nine-entry kit reachable on a dev box, with
its counts in operator config and never in `crates/`.

**§0eat (client).** `ui::refusals::CONSUME` words all three `REFUSE_C_*` for
both verbs, `Refused::Consume` joins the feed's shared refusal queue, and
both landed halves say a line. **The drink was dead by the same mechanism and
closed with it.** Routing the refusal through the queue rather than reading
the latch privately is what earns the refusal cue from `render/audio.rs` for
free — a file this lane never touched.

**§0sun (client).** `to_sun` takes the hour and derives both coordinates, so
no caller can pair one hour's height with another's bearing. `RIG_SUN_ARC` is
π, derived rather than picked. **Noon is bit-identical**, so no frame a visual
judge has scored became incomparable. The cloud deck was the one member of the
coupled set that genuinely broke and `sky::deck_rotation` fixes it exactly.

**§0dur is designed and NOT built** — it is a wall-6 slice and no lane in this
run was authorised to move the wire. The whole plan, the measured blast radius
and the eight gates it earns are in `DECISIONS.md` §open, "item durability
v0"; `NOW.md` §0dur points at it in a line.

## What is measured

`./ci/gates.sh` → **ALL GATES GREEN, EXIT=0**, run in the foreground on the
integrated tree: 11 banners, 124 suites, 1,900 tests, including
`test_protocol_golden`, `test_replay`, `test_alloc_zero`,
`test_terrain_golden`, `test_content`, `test_parity_wasm` (native == wasm
byte-identical) and the `--features render` tier. `PROTO_VER` is **41,
untouched** — `git status -- crates/protocol/` is empty and no lane moved a
packet layout. `node ci/knob_registry.mjs` → 303 declarations pinned, 1215
checks; the four rows added to `DECISIONS.md` §open contributed the four new
pins and their numbers were verified against source by that gate.

Every gate each lane added was proven red under the defect it defends, and
each lane's work was independently re-verified by a second agent that
reproduced those proofs and hunted for more. One red proof written into a doc
comment turned out to be **false and was caught by being run** — no test in
`bag_respawn.rs` can tell a merging `grant_kit` from a writing one, because
both call sites grant into an inventory zeroed one line earlier. It was
renamed to what it actually proves and re-proven under a real defect. That is
`CLAUDE.md`'s own rule one level out: a plausible red proof that was never
executed is the same as no proof.

## What remains

`NOW.md`'s five playtest items are now each *what remains* of one. Ranked
across the run, and none of it is a red gate:

1. **A wrong-tool gather swing falls through to `combat::raid`**, which has
   no owner or privilege filter — a stone hatchet aimed at a stone node
   inside your own base takes structure off your own wall, silently. Opened
   by this run's `hand`-row deletion and proven by fixture. `NOW.md` §0kit.
2. **The 0sun deck fix has no gate.** No fixture in `crates/client/tests/`
   constructs a `Skybox`, so deleting the one call site leaves all 28 suites
   green — proven. `sky.brightness` is ungated the same way. `NOW.md` §0sun.
3. **The consume latch aliases** and it is reachable from the keyboard, not
   only from a hitch: `G` and `H` are two independent presses in one system.
   The fix is a ring on `ClientCore`, which no lane here owned.
4. Two `wake` doors have no gate; `validate.rs` has no rule coupling the
   deleted `hand` rows to the kit; `parse_shard_toml` pushes unbounded.

## For the operator

Four rows are waiting in `DECISIONS.md` §open and one of them blocks a whole
item: **"item durability v0"** carries four questions with recommendations
and derivations, and `NOW.md` §0dur cannot start until they are spoken.
**"sun arc v0"** ships π with its derivation. The other two — the
`dev_spawn_kit` key, and *a rockless living player is stranded until death* —
are consequences of the kit change that nobody spoke; the second is verified
against the shipped tables rather than inferred, and both of its remedies are
content rather than code.

**And there is one thing to look at rather than read.** The cloud deck now
turns a full revolution per cycle about the zenith. It is exact, it is gated
as arithmetic, and it is a visible change nobody asked for — there is no
pixel gate by policy and a person looking at the frame is the visual gate.
Boot the client and watch the sky for thirty seconds; the sun is arithmetic
and already gated, the clouds are the judgement call.
