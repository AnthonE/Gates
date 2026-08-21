# Branch notes — `claude/base-math-server-client-4352ou`

**"Do research to see if the bases are showing up on client side like the
serverside claims… make sure the math is mathing"** (operator, 2026-08-21),
then two follow-ups off the frames it produced. Four commits, driven directly
rather than by the loop harness. This replaces the
`claude/recent-commits-review-t8ps2v` note.

Read `NOW.md` §0bl first — every item below is a line there, and `DECISIONS.md`
§open carries three new rows ("build plate v1", "piece flanks v0", and the
amended "build base lattice v0").

## What landed

**The answer to the question, first: it was never client/server drift.** Both
sides call one `column_floor_y`, so they agreed perfectly — about a staircase.
Of 5 726 buildable 4×4 footprints on the shipped island, **658 were flat**
(11.5%); 45.8% stepped half a metre across themselves and the worst stepped
10 m. The bases were showing up exactly as claimed. The claim was wrong.

**stairs** (3e8597b) — the one real drawn-vs-collided gap the sweep found. The
ramp slab was centred on the storey's mid-height, so its top face sat
`SLAB_T/(2·cos θ)` = **0.212 m** above the line `piece_ground` walks a rider
up, for the whole climb. The run was a typed 4.15 m against the cell's
4.2426 m diagonal and the 45° pitch was a literal that is right only while
`LEVEL_H_M == BUILD_CELL_M`. All three derived now.
`client/tests/lattice_geom.rs` is the gate, and it is new coverage rather than
a tightening: `ghost.rs` works in a piece's own (t, y) frame, so every claim it
makes is true of a piece drawn in the wrong cell — `base_transform` was the
half nothing read.

**build plate v1** (d63794f, 0179627) — `NOW.md` §0bl item 2, the v1 the
lattice row deferred. A column's floor is stored: the first foundation pins it,
orthogonal neighbours latch to the highest, and two limits refuse by name.
Wire v49 (4 bits on the piece record), save format 9, `state_hash` 12 → 13
bytes — the three costs that row priced. A connected base is one floor by
construction.

**piece flanks v0** (c804c63) — planes had no sides at all. A body walked into
the flank of a foundation and stood inside the slab and the drawn skirt;
measured with it off, a body sprinting at a 3 m-stilted plate **walks clean
through and out the far side**. `collide::plane_blocked`, plus a third
veto-lift in `movement::step` so a base built over you is something you walk
out of.

**the reference's offset, and the camera** (6c39343) — Devblog 187 fetched
whole: their snap offset is **one symmetric half-wall**. Ours shipped 6 up /
2 down that morning and was worse on our own island — ±3 moves a whole 4×4
from 86.7% of starts to 91.3%, an 8×8 from 62.1% to 70.8%, and halves the
deepest leg. Taken under `BALANCE.md` §6. `reference/BUILDING.md` §7c is the
research; §9 items 16–19 are what it means here.

## What is measured

`./ci/gates.sh` → **ALL GATES GREEN, EXIT=0** on the pushed tip, 12 banners,
including `test_protocol_golden`, `test_replay`, `test_alloc_zero`,
`test_terrain_golden`, `test_content`, `test_parity_wasm` and the
`--features render` tier. `node ci/knob_registry.mjs` → 377 declarations
pinned, 1 511 checks.

**Three suites are new and every one was mutant-proven**, 15 mutants across
them, and two of those mutants found assertions that were green on the bug
they were written for. Both are written into the files rather than quietly
fixed: `lattice_geom.rs`' ramp check sampled a *seam* inside each end, which a
0.09 m-short ramp still reaches; `flank.rs` read only where the body finished,
and without the flank the body ends up PAST the base rather than inside it.

**`PROTO_VER` 48 → 49 and `WORLD_SAVE_FORMAT` 8 → 9**, goldens regenerated in
the same commit as the layout change, and the fixture-input change that pins a
live plate is its own commit — `goldens.rs`' header requires that split and
this is the first time it has been exercised in both directions.

**Both replay goldens moved at piece flanks v0 and are regenerated.** That is
the honest signal: bodies stop where they used to pass.

**Looked at, not only computed.** `./ci/scene.sh --population 8 --settle 200`
on seed 20260731 — eight bots built a base over the real wire and the probe
photographed it. `NOW.md` §0bl item 1 asked for that counter-shot and it is
closed.

## Two pre-existing bugs surfaced on the way, both fixed here

1. **`PIECE_BYTES` said `11 + 8` from format 6 while the encoder wrote
   `12 + 8`** — `facing` joined the record and the constant did not move with
   it, so `WORLD_SAVE_MAX_BYTES` was 8 KiB short at the piece cap and **a
   shard at `MAX_PIECES` could not have saved**. Every check on the number was
   re-derived from the same wrong constant; the gate takes a difference of two
   real encodes now.
2. **`test_replay`'s world ends with ZERO pieces** — everything enters as
   twig, twig is never upkept, 900 ticks rots all of it — so
   `GOLDEN_FINAL_HASH` has never covered the piece store. Proven by mutating a
   field hashed since that gate was written and watching the pin hold.
   `GOLDEN_TRACE_HASH` folds every stamped hash rather than the last and goes
   red under all three piece-field mutants. **Wall 5's own gate had a hole in
   it and nothing else would have found it.**

## What remains

None of it is a red gate. Ranked, and each is a line in `NOW.md` §0bl:

1. **Freehand placement — the operator's, and the biggest.**
   `Command::Place` carries a cell address and no way to say *do not latch*,
   so a player cannot put a foundation at its own ground beside somebody
   else's plate — the first thing anyone tries on a slope. The reference has
   it and it is where their advanced base tech lives (bunkers, floor stacking,
   bridge bases). Costs an action-lane bit plus a UI decision. It is a
   **mechanic** question, so it wants a spoken call rather than a measurement.
   `reference/BUILDING.md` §9 item 19.
2. **The half wall.** Their answer to the gap a half-storey offset leaves on
   the floors above a stepped plate. `§7b.1` listed it missing before this;
   §7c.1 is why it matters more now. One shape code, and `SHAPE_BITS` is 4
   with codes to spare. §9 item 17.
3. **The stepped foundation, and DO NOT widen the plate limits instead.**
   §7c.2 is a published, tested negative result on exactly the change that
   will keep suggesting itself — they tried a three-metre gradient for our
   problem and reverted it, because *"building on flat became harder"* and it
   made door blocks clip. Ours would be a catalogue row plus a shape code.
   §9 item 18.
4. **The shot walk does not consult `plane_blocked`.** An arrow through a
   floor is its own item with its own answer; the lintel precedent says a body
   and an arrow may disagree only where somebody decided they should.
5. **The flank costs 153 µs a tick and a memo takes most of it back.**
   Measured A/B, `NOW.md` §0bl item 4b: `col_base_y` re-samples terrain per
   cell per candidate. `terrain_band` is pure in (seed, cell) so a
   direct-mapped memo is EXACT, not approximate — `occupy::SlotCache`'s own
   argument. Cheap, not urgent, and the number is written down so nobody has
   to re-measure to decide.
6. **A band-boundary wall still bases on its canonical cell** (§0bl item 3).
   The plate makes it rare rather than fixing it: inside one base every column
   shares a floor, so the slit is now only where two separately-started bases
   meet.
7. **The diagonal wall's √2 root scale stretches its UVs.** Cosmetic, pinned
   by `lattice_geom.rs` §D so it cannot grow, and `ART.md`'s business rather
   than this lane's.

## For the operator

**One spoken call is worth more than the rest put together: freehand.**
Everything else on that list is engineering with a known shape. Whether a
placement may decline the latch changes what a base can be, and no
measurement can answer it.

Two knob rows are in `DECISIONS.md` §open and both ship a default: **"build
plate v1"** (±3 bands, taken from the reference and measured against ours) and
**"piece flanks v0"** (`PLANE_THICKNESS_M`, and the note that `place` still
does not refuse a piece where a body stands — the reference does not either,
and refusing is its own grief vector).

**And one thing to look at rather than read.** The plate changes how every
base sits on the ground and the flank changes what you can walk into. Boot the
client on a hillside, put a row of foundations down, then try to walk into the
side of it. `ci/scene.sh --play` boots a populated world if you want somebody
else's base to try it on.
