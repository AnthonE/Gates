# Branch notes — `claude/building-placement-foundations-3gmwfk`

**Freehand placement.** The one item the previous branch's note called the
operator's and unmeasurable — *"whether a placement may decline the latch
changes what a base can be, and no measurement can answer it"* — was put to
the operator, answered, and built. This replaces the
`claude/base-math-server-client-4352ou` note.

Read `NOW.md` §0bl item 5 first, and `DECISIONS.md` §spoken 2026-08-22 for
the call itself. The §open row beside it prices the three numbers.

## The call, and the half of it that was research

The operator took **"any piece — full freehand"** off four options. Asked how
the player *expresses* it — hold a key, a build-mode toggle, a ghost that
cycles — they picked none of them and described the reference from memory
instead: *"when i played rust i think when u didnt get near an existing pieces
it was always freehand… u just aimed where u needed the piece"*, and *"can
search to see how it works"*.

**Searched, and the memory is right in a way that mattered.** The reference
has **no freehand input at all**. Placement there is continuous and
socket-based, a piece is *attracted* to a socket when your aim comes near one,
and a freehand piece is one aimed where nothing catches it. That is why their
own guides call the technique *"very tricky and non-intuitive, even to veteran
Rust players"* and teach it with the logs on a twig foundation and the compass
tics as visual references — a game with a freehand button would need none of
that. `reference/BUILDING.md` §7c.3 said the sources "do not state the input
consistently"; that was the misreading, and it is rewritten: the agreement is
that there is none.

**The mechanism does not port, and that is the finding.** Ours is
address-based — `Place` carries a cell and `plate_for` latches on exact
adjacency — so there is no "near" for a placement to miss and no room to be
adjacent-but-not-snapped. The bit has to be explicit where theirs is emergent.

**What ports is the aim.** `place::aim_from_look` already marches the look ray
to a real `(f32, f32)` and `place::target_at` quantizes it away, so the
sub-cell remainder was sitting there unused. It is the freehand input the
model was missing: near the shared edge with a built neighbour the placement
snaps, past `SNAP_BAND_FRAC` of the cell it declines, and the ghost's own
height is the preview. No key and no mode, which is the property of theirs
actually worth keeping.

## What landed

**`plate_for` gains a fourth case ahead of the neighbour scan.** With the bit
set an empty column takes band 0 — its own ground — so the two plate refusals
cannot fire for it, because band 0 is neither a stilt nor a cut. A refusal
about height is now exactly a refusal about a latch the player asked for.

**Case 1 is deliberately not declinable.** A piece entering a column that
already holds one still takes that column's plate, whatever the bit says. That
is the invariant `plate.rs` and the ghost both lean on, and the reference keeps
it too — their walls must still take a socket, so freehand there is a
foundation-and-floor technique rather than a general licence.

**The bit crosses the wire because it cannot be re-derived.** Which neighbour
is built is a fact the server already has; which floor the player *wanted* is
one only the client holds. `PROTO_VER` 49 → 50.

## What is measured

`./ci/gates.sh` → **ALL GATES GREEN, EXIT=0**, including
`test_protocol_golden`, `test_replay`, `test_alloc_zero`, `test_terrain_golden`,
`test_content`, `test_parity_wasm` and the `--features render` tier.
`node ci/knob_registry.mjs` → 379 declarations pinned, 1 519 checks.

**Six new gates, all six mutant-proven** — three in
`sim-core/tests/plate.rs` (what the bit MEANS) and three in
`client/tests/freehand.rs` (whether an aim can reach it). Each mutant killed
exactly its own gate: deleting the early-out kills the two behaviour gates and
leaves the case-1 guard standing; letting freehand bypass case 1 kills the
case-1 guard alone; inverting the snap comparison, returning freehand over
open ground, and narrowing the band each kill one client gate.

**The golden fixture pins the bit `true`, and that was measured rather than
chosen.** Pinned `false`, deleting the encoder's `w.write` leaves every byte
identical and `test_protocol_golden` passes — run both ways, 13 green on the
mutant. Pinned `true` it is red. A fixture whose new field carries the zero
value cannot tell a live bit from a dropped one.

**96 goldens rekeyed `v50_*`, of which exactly two changed bytes** —
`action_place` and the `hello` that carries the version. The other 94 are pure
renames, which is the honest signature of a width bump and worth checking for
on the next one.

**No save format moved.** The plate a freehand placement takes is an ordinary
plate, so `WORLD_SAVE_FORMAT` stays 9 and neither replay golden moved.

## Two gates that are weaker than they look, written down rather than papered over

**Freehand does not ride `test_replay` or `test_parity_wasm`, and I measured
that instead of assuming it.** The house precedent is explicit — a verb that
is merely *possible* inside those gates is not covered by them, and claiming
otherwise was a judged failure once already (`DECISIONS.md`, repair v0). So I
put the bit on all three surfaces, then ran the mutant: making `plate_for`'s
early-out inert left every parity digest and both replay goldens
**bit-identical**. Neither script ever places into an empty column with a
built orthogonal neighbour whose band differs, which is the only shape where
the bit can change a byte. A tick-167 foundation in the neighbour column was
tried next — it *did* move both goldens, and the mutant *still* passed, so it
was reverted rather than pay a wall-5 golden churn for coverage it does not
buy. What stayed is `alloc_zero`, which walks the branch, which is exactly
what that gate measures. The bit's behaviour is covered by `plate.rs`, over
real terrain, mutant-proven three ways. Closing the rest means engineering
`probe.rs`'s world script to construct the case; it is cheap there and is
written into `NOW.md` §0bl item 5.

## And one client gate weaker than it looks

`the_bit_flips_where_the_snap_band_ends` measures the flip position against
`SNAP_BAND_FRAC` itself, so narrowing the band moves the constant and the
assertion together and it stays green — run and confirmed. That is correct
(the band is a knob, not a law), but a reader who saw only that test would
over-trust it. The law-like consequence of the two-thirds value — that bands
measured from opposite sides *overlap*, so a cell wedged between two built
columns can never decline — is held by
`a_cell_between_two_columns_cannot_decline` with an explicit assert, and that
one does go red. The split is noted in the test file.

## What remains

Unchanged from the previous note except where freehand touched it. None of it
is a red gate.

1. **Nobody has played it.** The bit is *aimed*, so a placement's height
   changes as you sweep the crosshair across one cell. Whether that reads as
   control or as twitch is the one thing no gate here can score, and it is the
   first thing to check on a hillside.
2. **The half wall** — their answer to the gap a half-storey offset leaves on
   the floors above a stepped plate, and it matters *more* now: freehand makes
   deliberate steps reachable, so the gap it fills is one players will now
   create on purpose. One shape code, `SHAPE_BITS` is 4 with codes to spare.
   `reference/BUILDING.md` §9 item 17.
3. **The stepped foundation, and DO NOT widen the plate limits instead.**
   §7c.2 is a published, tested negative result on exactly that change.
   §9 item 18.
4. **The shot walk does not consult `plane_blocked`** — an arrow through a
   floor is its own item.
5. **The flank costs 153 µs a tick and a memo takes most of it back.**
   `col_base_y` re-samples terrain per cell per candidate; `terrain_band` is
   pure in (seed, cell) so a direct-mapped memo is exact, not approximate.
   Cheap, not urgent, number already written down.
6. **A band-boundary wall still bases on its canonical cell.** The plate made
   it rare, not fixed.
7. **The diagonal wall's √2 root scale stretches its UVs.** Pinned so it
   cannot grow; `ART.md`'s business.

## For the operator

**The mechanic is spoken and built; the three numbers under it are not.**
`DECISIONS.md` §open, "freehand placement v0" — and `SNAP_BAND_FRAC` is the
one to argue with, because it *is* the interface. Two thirds means snapping is
what happens and freehand is what you do; below a half it stops being true
that an interior cell of a base cannot decline, and a gate says so.

**And the thing to do rather than read is the same as last time, one step
further on.** Put a row of foundations on a hillside, walk to where the latch
refuses, then aim at the far edge of the next cell and watch the ghost drop to
its own ground. `ci/scene.sh --play` boots a populated world if you want
somebody else's base to try it beside.
