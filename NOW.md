# Gates · NOW.md — what next

The only list that answers "what should the loop pick up." Top item first.
Done items are deleted, not checked — history lives in git and
`DECISIONS.md`. A loop iteration starts here, ends with gates green.
An item is ≤ ~25 lines (`CLAUDE.md` §loop discipline); detail belongs in
`DECISIONS.md` §open or a `gates-loop/findings/` note.

> **Rebuilt 2026-08-05.** The file had reached 2040 lines: `merge=union`
> means three lanes append and nothing ever deletes, so it accumulated ~12
> items whose own titles said "done this pass", a duplicate, and a large
> block of browser-renderer work the client pivot retires. Everything
> removed is in git. Nothing open was dropped; where an item is retired
> rather than finished, it says so.

---

## 0c4 · GAP PASS (ui lane): the refusal walk now reads position, not just length *(done this pass)*

From `findings/pass-20260805-111501-02-judge.md` ranked fix 2 (and -01's fix 3,
the same hole stated softer). The judge exchanged the VALUES of
`REFUSE_D_HEARTH` (10) and `REFUSE_D_DOOR` (11) in `deploy.rs`, touched no JS,
and `ui_smoke` reported `1597 checks passed` while a player placing on a missing
hearth read "no door there" — `CLAUDE.md`'s positional-payload trap, in this
lane's own gate.

Closed: `checkNames` ties each sentence to its Rust constant's NAME, over all
five tables (the four in `refusals.js` plus `hud.js`'s move table, which is read
off the live panel). Three checks — complete both ways, contains its keyword,
and the keyword matches exactly ONE sentence in the table, which is what makes
any transposition red rather than only some. 1597 → 1684 checks. `ui_mutants.sh`
M31–M34 are the executable version: the judge's mutation verbatim, a JS
transposition each side, and softening a keyword. M28's anchor was stale (its
lines moved to `refusals.js`, so it matched zero times and the script was red on
a clean tree) — re-anchored, not deleted.

What it still does not do: read English. A wrong sentence containing its own
keyword passes. The ORDER is walled; the prose is not.

**All six ranked gaps in both reports are `crates/`/wire work this lane must not
touch** — the satchel's raid verb + a structure-damage path into `build::hp`,
the bow, shore barrels as a second destination class, `jump`, and the wipe.
Unclaimed, and the two judges rank them above everything else in the repo.
## 0a · world lane: skirt residual — the beach path, and the ring's hard edge

*(GAP PASS item, from `findings/pass-20260804-173640-01-visual.md` ranked gaps 1
and 3. The skirts themselves LANDED this pass — `terrain::skirt_fill`, 6 tests,
`ci/clutter_shape.mjs` at 54 checks. This is what they left open.)*

Two residuals, both cheap, neither blocking:

1. **The Pebble path is untested by sweep.** A 400 m box at the island centre
   yields 3 Pebble skirt elements in 1,875 tiles — correct behaviour (the kind
   law tracks the splat and there is almost no sand channel inland), but it
   means no sweep has exercised sand. A beach-tile sweep in `tests/clutter.rs`
   would close it. The kind law is `clutter_kind_at`, shared with the grid,
   which `test_each_kind_stands_on_its_own_splat_channel` does cover — so this
   is coverage of the skirt's *placement* on sand, not of the law.
2. **The clutter ring still ends hard** at ~32–45 m rather than thinning into
   the fog. `web/src/clutter.js` names it and names why it is parked: the cheap
   fix is a per-frame player-relative shader term, which is a new program, and
   the prewarm gate counts program links after `inWorld`. The recipe if someone
   takes it: thin stochastically by instance hash (so the same elements survive
   at a given range and nothing pops), then scale survivors to zero. Budget and
   prewarm the program, or drive it off the existing shared wind uniform.

**Nothing here has been seen.** No frames were captured this pass and
`browser_smoke`'s renderer tier is off by operator config, so every claim above
is placement arithmetic. The moment the item becomes "does the skirt look
right" rather than "is there one", this lane needs frames again.

## 0c3 · RECOVERY (systems lane): the same red, and it was propagation *(done this pass)*

`ci/gates.sh` was red on this lane's clean tree at `ui smoke`, same assertion in
both health runs, so not a flake. **The code was wrong and was already fixed** —
the ui lane landed the sentence in `b2a48bc`, judged PASS, and it was in `main`
before this pass started. `lane/systems` had simply not taken `main` since. So
this lane's red was *propagation lag, not a second defect*, and the fix was to
take `main` into the branch. Nothing reverted, nothing lost, and the ui lane's
wording ("cannot be repaired") is not forked — duplicating it here with a
different sentence is what would have conflicted.

The standing hazard, which is this lane's to carry: **growing `REFUSE_B_*` is a
two-file act and the second file is `web/`, which this lane may not touch.**
`ui_smoke` §W walks the constants against `interact.js`, so a sim commit that
adds a reason reddens *every* lane until the client half lands. Twice now (9,
then 10). `build.rs` now says so at the constants themselves; the durable fix,
if one is wanted, is a spoken call on whether the sentence table should be
generated rather than mirrored — not invented here.

**Expect next:** nothing was masked in the code tier — `ui smoke` is its last
gate, so the green below is the whole tier. The renderer tier (`browser smoke`,
`vantages`) has NOT run: tier `fast` for this whole run, and `browser_smoke` is
off by operator act. If either is red it is still red, and an `all` run is where
that surfaces.

## 0c · RECOVERY: the refusal table fell one short again *(ui lane — done this pass)*

`ci/gates.sh` was red on a clean tree at `ui smoke`: `build.rs` declares 11
`REFUSE_B_*` reasons and `interact.js` carried 10 sentences. The CODE was
wrong, not the gate — `REFUSE_B_UNPRICED` (10) landed from the sim lane in
`65e5110` and this lane's table stayed where it was, so a repair refused on a
piece whose baked row quotes no price would have reached the player as
`code 10`. Fixed by adding the sentence ("cannot be repaired"); nothing was
reverted and nothing was lost.

Worth recording because the gate is now two-for-two on the same class: §W was
written when `REFUSE_B_INTACT` (9) shipped as a bare number, and it caught the
very next reason the sim grew, on the run that grew it. A count in prose rots
the same way — `promptForBuild`'s comment said "nine reasons" and now names no
number.

**Nothing was masked behind it.** `ui smoke` is the last gate in the code tier,
so the green run below it is the whole tier, not a fresh first-red. What has
NOT run is the renderer tier (`browser smoke`, `vantages`) — off for this whole
run at tier `fast`, not skipped by this pass. If either is red it is still
red, and the next `all` run is where that shows up.

## 0c2 · The same bug, one file over: `DEPLOY_REFUSE_TEXT` *(ui lane)*

Found while scanning `65e5110` for other unmirrored constants, not built —
§0c was a recovery pass and this is a feature-shaped fix.

`main.js:160` holds `DEPLOY_REFUSE_TEXT`, 13 entries against `deploy.rs`'s
`REFUSE_D_KIND`(0)…`REFUSE_D_OWNER`(12). It matches *today*. It is also a
module-private `const` in `main.js`, so `ui_smoke` cannot import it and no
gate walks it — which is verbatim the condition `interact.js:736` describes
as the reason the build table had to move: "cannot live as a bare array in
`main.js` where nothing can walk it". A fourteenth `REFUSE_D_*` reaches the
player as `can't place: code 13` (`main.js:1260`), and §0c is the proof that
the sim does grow these.

The fix is the one already paid for: move the table to `interact.js`, export
it, and extend `ui_smoke` §W's walk to parse `REFUSE_D_*` out of `deploy.rs`
the way it parses `REFUSE_B_*` out of `build.rs`. Same shape for
`main.js:151`'s `REFUSE_TEXT` (5, vs `craft.rs`) and `:57`'s
`REFUSE_REASONS` (2) while the file is open.

Related and NOT this lane's: `bridge.rs:66` still exports
`DEPLOY_DEF_ROW_WORDS = 4`, so `SUB_DEPLOY_DEFS`' new `n_costs` + cost rows
stop at the wasm bridge and `web/src` cannot show what mending a door costs.
That is a systems-lane export, like §0e.

## 0e2 · The deploy-def stride is stale, and this lane cannot fix it alone
*(systems lane — BLOCKED on a web/ half)*

Found while doing §0e; both are `bridge.rs` and the handover at line 88 reads
as if they are the same size. They are not. `DEPLOY_DEF_ROW_WORDS = 4`
(`bridge.rs:67`) predates `n_costs` + cost rows on `SUB_DEPLOY_DEFS`, so what
mending a door costs stops at the bridge. The Rust half is four lines,
mirroring the piece path (`4 + 2 * MAX_PIECE_COSTS`, filled at `bridge.rs:387`).

**The blocker is that the stride is hardcoded in `web/`, which this lane may
not touch:** `wasm.js:96` views the table as `16 * 4`, `main.js:306/329` index
`rec.row * 4`, and `interact.js:527` declares it. Widening the Rust alone does
not redden a gate — it silently re-bases every one of those reads onto the
wrong words. That is worse than the current state, so it did not land.

**ui lane owns the second half.** The clean version is one commit across both,
or `interact.js:527`'s constant becoming the single reader that `ui_smoke`
walks against `bridge.rs` — the shape §W already uses for `REFUSE_B_*`.

## 0f · A proposal for the `renderer_touched` list — *(operator act)*

`gates.sh:110` reserves the exemption list to the operator, so this is a
proposal and not a change. `web/src/interact.js` is now pure, node-imported
and covered by §Q/§R/§V/§W with eleven mutants of its own; it cannot reach a
material, a shader or the terrain. A one-line edit to it currently costs the
~19-minute renderer tier. `map.js` and `invmove.js` have the same shape.
`main.js` does NOT — it builds three.js scenes and belongs where it is.
`web/src/refusals.js` (new) is the strongest case on the list: four string
tables and one accessor, no imports at all, fully walked by §W with fourteen
mutants red. It is NOT exempt today and correctly costs the renderer tier.

## 0g · The deploy-def cost rows stop at the bridge *(systems lane, cross-lane)*

Carried out of the finished §0c2 so it is not lost with it. `bridge.rs:66`
exports `DEPLOY_DEF_ROW_WORDS = 4`, so `SUB_DEPLOY_DEFS`' new `n_costs` and
its cost rows never reach `web/src` — the client cannot show what mending a
door costs, and `describeDeploy` reads `b+3` of a stride-4 row because that is
all there is. Same shape as §0e: a widened export, ~10 lines.

## 0d · The island validates at boot *(systems lane — done this pass)*

Closes the boot-refusal request filed twice — `## 0`'s "A short waystation tier
is silent on a shard" and `## 1b`'s "Systems lane, one boot-time call please"
(judge fix 1, inherited twice). Those two bullets are the **world lane's** lines,
so this lane did not edit them; they are answered, and their owner can delete
them.

`crates/server/src/boot.rs`: `check_seed(seed)` refuses an island whose authored
sites are short, called from `spawn_shard` before an identity is loaded or a
port is bound, so every path that raises a shard refuses the same seed. The
binary also prints the counter NOW.md said was missing —
`island ok: 3/3 authored sites`.

**Measured first, and it changes the claim: 0 of 20 000 seeds are short.**
Seeds 0..20 000 all give `sites_live == 3`; so do all eight seeds this repo
names. This is a **tripwire, not a live hole** — no seed reachable today takes
the short branch, and nothing here closes a gap a player can fall into. It
fires when a change to the ring, the separation floor or the candidate search
makes a short tier possible. `DECISIONS.md` §open carries the number.

What remains: nothing on this item. The knob-relaxation alternative
(`WAYSTATION_MIN_SEP_M` moved until the ring fills) was **not** taken — with no
short seed to relax for, it would be a number invented against no measurement.
## 0a · The canopy stands off the road, draws, and is gated *(world lane — done this pass)*

From the judge's **ranked fixes 1–6**, `pass-20260805-074623-04-judge.md`, which
FAILED `loop/waystation-canopy`. All six landed. Knob row: `DECISIONS.md` §open.

**Then `-05` FAILED it again on one wall, and that fix landed too** — this branch
carries both passes and is judged as one diff.

- **The weakened assert is restored as a law, not as the old line.** `-05` fix 1:
  `cleared >= 1` per seed became an aggregate, which is a loosening. The judge's own
  prescription is what shipped — keep `assert_eq!(furnished, want_furnished)`, keep
  the aggregate displacement, and add the per-seed claim the old line was reaching
  for: the control arm had an **opportunity**, i.e. the zones cover ≥ 1 scatter cell
  whose ground `scatter` would not veto outright. It is a law, not a draw —
  **measured worst 7 cells against a floor of 1** — and mutation-proved live *past*
  `furnished`, which asserts first and would otherwise mask it (`-05` check 2's own
  warning, applied to this test).
- **The stale "walks through the parapet" claim was in four files, not two.**
  `-05` fix 2 named `NOW.md` and `ci/waystation_canopy.mjs`; `ci/haven_shelter.mjs`
  and `TERRAIN.md` stage 8 carried it verbatim. All four now name the narrower
  residual with the line that proves it — see **Left behind** below.

- **Cause, not the sentence:** it stood at the site centre, and a waystation's
  centre is the coast road's centre line — `pick_minor` scores only candidates off
  the ring, and `haven_shelter_bearing`'s doc already said so. It now stands in a
  gap in the container pair at `WAYSTATION_CANOPY_OFF_M` (= the ring radius), on a
  bearing `waystation_canopy_bearing` accepts, folded into the phase search because
  the gaps move with the rotation. The road test is the **footprint's** — anchor ±
  the bounding radius along the island radial, the axis `road_band` measures on.
- `GOLDEN_TERRAIN_HASH` regenerated in the same commit; worldgen moved twice.
- **It draws:** `props.js` gained `WAYSTATION_CANOPY_PARTS`/`_PEAK`/`_BAND`,
  `canopyGeometry()` and `ARCHETYPES` row 12. `ci/waystation_canopy.mjs` is written
  and wired (97 checks) — `haven_shelter.mjs`'s sibling asserting the opposite
  shape, a roof not a room; negative-tested four ways. The five false doc claims
  are true now, and the parapet's bare `< 1.2` is `CAPSULE_HEIGHT_M * 2/3`.
- **Unverified:** nothing here was seen. No frame captured, `browser_smoke` off at
  the operator's tier — "it draws" means the buffer matches the sim, not that it is
  lit, sited or legible at range.

**Left behind:**
- *(systems)* Bodies **are** stopped — `movement.rs:158` → `occupy.rs::blocks` →
  `terrain::slot_blocks`, gated by `tests/solid.rs`. The old bullet here said the
  opposite and was false in four files; all four are corrected. The two real holes
  are each one grep: `combat.rs` carries no occupant term, so a shot passes through
  the parapet that stops a body (TERRAIN §1 stage 6 asks the forest for cover and
  gets none); and `collide::piece_ground` (`collide.rs:339`) reads built pieces
  only, so nothing scattered is standable — the deck and the pad's plinth are
  kerbs you sink into.
- *(any)* `tests/box_container.rs` overflows its stack in a **debug** build on a
  clean `lane/looks` — confirmed on a worktree at 750fd53, not from any diff here.
  `ci/gates.sh` runs `--release`, where it passes, so CI cannot see it.

---

## 0b · The map's grid and its arrow, made exact and gated *(ui lane — done this pass)*

From the judge's **ranked fixes 1–3**, `pass-20260805-074623-02-judge.md`, against
the map that landed the pass before. Taken ahead of §0's build prompt because both
were defects already on the trunk, one of them a hole in the gate this lane's speed
rests on.

- **The off-by-one was neither formula.** The report measured `paintMap`'s index
  flip against `worldToMap`'s extent flip — "exactly one row, always" — and noted
  that x came out exact. That asymmetry is the tell: sampling from 0 put every
  sample on its pixel's CORNER, so the island was painted half a cell out on BOTH
  axes, and only the flipped axis landed `floor` on a row boundary. Fixed at the
  origin (`main.js`, `orig = step / 2`), not in either projection. §U now sweeps all
  16 rows and all 16 columns, reads the painted band back, and asserts the painted
  row IS the projected row — with the origin read out of `main.js`'s source, because
  `paintMap` is handed a sampler and cannot see where it was sampled.
- **The marker's heading had no assertion at all.** M11 (rotation pinned north)
  survived all eleven of last pass's mutants. `hud.mapDir` parks the drawn direction
  beside `mapPos`; §U sweeps N/E/S/W plus one off-cardinal and asserts the vectors
  BY NAME rather than re-deriving `(sin, −cos)`, which would agree with a wrong
  formula too.
- **Both cosmetic knobs registered** — `MAP_SHADE_CLAMP`, `MAP_MARKER_PX` — in
  `DECISIONS.md` §open, now pinned by `ci/knob_registry.mjs`.

`ui_smoke` 561 → 635 checks; nine mutants run, nine red, including last pass's
survivor. §0a's remainder is untouched and still needs the other two lanes.

---

## 0a · The island has a map now — what it still cannot show *(ui lane)*

From the judge's **ranked gap 3**, `pass-20260805-074623-01-judge.md`: "There is
nowhere to go", leading with `MENUS.md:102` Map MISSING. **Landed:** M opens
`#map`; `map.js` paints the island from `terrain::splat_from` through the bridge
— one wasm fill, the same law the 3D ground blends by — hillshaded, 16×16
A–P/1–16 grid, your position and heading. `ui_smoke` 510 → 561 (§U); ten mutants
red, two of them gate holes this pass found and closed. `DECISIONS.md` §open has
`MAP_GRID_M` and the shade floor's derivation. **Gates ran `fast`** (renderer
tier off this run by operator act); the diff touches `main.js`, so `auto` would
schedule it and §3's two clean-trunk reds still stand.

What remains, and none of it is a UI call:

- **Systems lane, one export please:** `terrain_haven_xz(seed)` — the judge's
  own named item, now with a screen to land on. The map draws no marker at all,
  so the one authored destination is still unfindable. `terrain::haven` is pure
  and `bridge.rs:92` memoizes it already.
- **Operator: may the map pin anything?** `ALPHA.md` §1's "no map position"
  binds the DEATH screen and the map stays off it. A haven pin, a bag pin and a
  death marker are three separate calls; `mapstylized.jpg` shows all three.
- **Looks lane, information only:** the map paints the alpine channel as rock
  while the world whitens it above `materials.js`'s `SNOW_RANGE`.

**Respawn — the gap's other half — is BLOCKED, measured.** The wire carries
`Respawn { on_bag: bool }` and nothing else; no owner bit and no cooldown ride
`DeployRec` (`deploy.rs:232`, "never the wire"). So the client cannot tell its
own sleeping bags from anyone's, nor which are ready, nor name one. "Beach or
each live bag" (`ALPHA.md` §1) is a wire change first — systems lane.
## 0a · Repair — the door is mended now; nothing still presses the key

Landed this pass, closing the systems half of the item above and the judge's
ranked fix 1 (`findings/pass-20260805-074623-03-judge.md`, the one failed
check): `build::repair` reaches a **deployable**, so the door — the breach
point a raid actually uses — can be bought back. Both blockers answered:
`Deploys` gained `find_index`/`set_hp`, and the one-item price is replaced by
the deployable's **recipe**, joined onto `DeployDef::costs` at bake time and
divided by the same `repair_units` at the same `repair_cost_pct` (no new
number; `DECISIONS.md` §open, "repair v0, the deployable half").

Wire **v21**: `ACT_REPAIR` and `SUB_PIECE_REPAIRED` carry the leading bit that
picks the store, because `place_deploy` requires the doorway piece at the
*identical* address, so a door and its doorway are one address exactly. The
event reuses `STRUCT_DEPLOY_BIT`. 68 goldens rekeyed `v21_*`, two added (70)
so both bit values are pinned. `REFUSE_B_UNPRICED` is the eleventh build
refusal — it also closes a free-heal hole on the piece path, where a zero-cost
row fell through both cost loops and mended anyway. And `Command::Repair` now
actually rides `probe.rs`, `tests/replay.rs` and `tests/alloc_zero.rs`, which
is what the old `build.rs` comment falsely claimed the price alone did.

What remains:

- **Nothing can press it**, unchanged and now the whole of the gap. The
  browser client needs the verb bound and a prompt — **ui lane** (`web/`); the
  native client picks it up with §1 slice 1.
- ~~The deploy-defs drip carries no price~~ — **landed in the same commit that
  filed this line, which is why the line was wrong.** `encode_event_deploy_defs`
  writes `n_costs` and the cost rows, `decode_event` reads them into
  `DeployDef`, `v21_event_deploy_defs.bin` 22 → 43 B. A client can quote a
  repair before paying; only the prompt that shows it is left (**ui lane**).

Untested here for the same reason as before: no client sent a real repair, so
the round trip is proven by goldens, role and unit gates, not by a live shard.

---

## 0h · Repair is bound — R presses it, and the store bit is gated *(ui lane)*

Closes the `web/` half the item above left open ("nothing can press it"). The
bridge export had landed; `ui_smoke`'s own comment still said it had not.

- **R sends it**, out of build mode only — R is already the build-level raise,
  and the two branches sit in one if/else chain, so the binding is their
  ORDER. Gated as an ordering law, not as a condition (M40).
- **`nearestRepairable` walks BOTH stores** and reports which won. A door and
  its doorway share one address exactly, so the leading `deploy` bit is the
  only thing telling them apart, and it is a `u32` like the other four — the
  positional class with the discriminator in front. Exact ties go to the
  deployable; distance still outranks the store both ways.
- **The prompt ranks last** (build > E > swing > repair): it resolves off the
  feet, not the crosshair, so it may only fill a row that would be blank.
- `ui_smoke` 1684 → 1731 (§X); six new mutants, M35–M40. `bridge.rs`'s five
  parameters are parsed and `deploy` is pinned FIRST, so a systems-lane
  reorder reddens here.

**Systems lane, one bit please:** `EventMsg::StructHit` carries `deploy` and
`core.rs`'s `struct_hit` tuple drops it — `(cx, cz, level, loc, left, max)`.
So the client cannot attribute damage to a store at a shared address, and the
prompt names the verb but not the hp. Add the bit to the tuple and to
`client_struct_hit_info`'s spare bits and the row can read `340/500`.

---

## 0 · Half the verbs you own are undiscoverable — *(ui lane — both halves done)*

From the judge's **ranked gap 3**, `pass-20260805-063306-01-judge.md`. NOW.md
held no open ui-lane item, so the gap list supplied this one. Two halves:

- **A bearing readout — DONE.** Compass strip, top centre, `hud.js` +
  `index.html`. `ui_smoke` 442 → 510 checks (§S/§T); nine mutants red.
  **+Z is North, +X is East** — `DECISIONS.md` §open has the row and the
  conflict it resolves against `build.rs`'s `LOC_EDGE_N`.
- **The build prompt — DONE.** Build mode drew a ghost over the aimed cell
  while the row under the crosshair advertised `[LMB] CHOP TREE`. It now reads
  `[RMB] PLACE WOOD WALL`, with the shortfall of the first ingredient you
  cannot cover, and redraws on B and the wheel. Which of the three verbs gets
  the one row is `interact.centrePrompt` — pure, swept over all eight
  combinations (build > E > swing). `describePiece`/`describeDeploy` moved out
  of `run()` so the stride-8 decode is gateable arithmetic, and the shape and
  material labels came with them (walked against `build.rs`). `ui_smoke`
  635 → 1468 checks (§V); §Q/§R re-anchored, not relaxed.

Two things the compass could not carry, both needing another lane:

- **Systems lane, one export please:** the haven pad centre is not reachable
  client-side. `terrain::haven(seed)` is pure and `bridge.rs:92` already
  memoizes it, but nothing returns it — no getter beside `terrain_fill_slots`.
  One export and the pad can carry a compass marker; the judge's gap named
  the pad as the destination nobody can find.
- **A marker is a design call, not just an export.** `showDeath` states the
  standing rule (`ALPHA.md` §1, "no map position") and it is about the death
  screen, not the HUD — but a pad marker is close enough to it that an
  operator word is cheaper than a pass spent guessing.

---
## 0c · Nothing in the world names a key *(ui lane)*

What the build prompt could not fix, learned by building it: a contextual hint
can only describe a mode you have already entered. **B, C, M, T, G, H, U, L and
Tab are spelled out in exactly one place — `index.html`'s `#hint` paragraph on
the pre-connect screen — which is gone the moment you join.** A player who
misses it has no way back to it and no way to discover build mode, crafting or
the map at all. Every verb this lane has made legible sits behind a key nobody
is told about.

Two candidate shapes, and the reference has both:
`Rust Images/choppingtree.jpg` carries an onboarding checklist top-left that
retires itself as each verb is used; `MENUS.md` surveys the keybinds screen
that every loader in the reference ecosystem exposes. The checklist is the
cheaper one and it teaches in the world rather than in a menu.

Not started. Needs no other lane and no operator word — the strings are ours
(`CONTENT.md` owns names, and these are key names). `ui_smoke` can drive it
end to end: it is DOM and state, no renderer.

---
## 0 · The rest of `pass-20260805-074623-01`'s ranked fixes

*(GAP PASS, world lane. Its ranked fix **1** — the authored sites were not
on the native↔wasm parity surface — landed on `loop/site-parity`; see
`DECISIONS.md` §open "probe coverage v0". Measured before the fix: of the
golden's 256 cells, **zero** were inside `in_haven`/`in_waystation` on all
three probe seeds, so `haven()`'s value reached the digest through nothing
while `client-wasm` reads it off wasm and the server off native. Its other
two fixes were left, deliberately, and are below. That report's ranked
**gaps** 1–3 — projectiles, day/night + AI, the recycler — are all systems
lane; the newest visual report's gaps are all texture/material work, which
the operator parked for this lane on 2026-08-04.)*

- **A short waystation tier is silent on a shard** (ranked fix 2). `pick_minor`
  leaves `Waystation::NONE` when no candidate clears the separation floor.
  `tests/waystation.rs` refuses that over 16 seeds, but a shard boots whatever
  seed `shard.toml` names: on a seed the ring cannot fill, the island ships
  with one or zero waystations and no counter, event or log line. Wants a
  boot-time refusal in `crates/server` — **not this lane's file.** `probe_sites`
  now hashes each `live` flag, so a short tier at least moves the fingerprint
  on the three probe seeds; that is not the same as being loud on an arbitrary
  one. One-line cross-lane request: sim-core can export a
  `sites_complete(&Haven) -> bool` for the shard to call at boot.
- **The tier gradient is gated in containers per m², but a player collects
  loot** (ranked fix 3). A waystation crate and a pad crate are the same
  `crate` loot table, so per container the lesser tier pays exactly what the
  destination pays and only geometry separates them. `ci/haven_prize.mjs` knows
  nothing about waystations, so giving them their own table — or changing crate
  yields — moves the real gradient with every gate green. Wants that gate
  restated in **expected items per site**, not containers per m².

## 1 · The client is becoming a native Rust desktop app

*(Operator, 2026-08-05. `DECISIONS.md` has the row. This outranks the
milestones below and retires a block of browser work outright.)*

Two slices have landed and both are on `main`:

- `crates/client` — the session. Connects to an **unmodified** shard over
  the same `wtransport`/QUIC the browser used, same `PROTO_VER`. Measured
  against a live shard: `snap sent 135`, 120 applied client-side,
  `in ok/bad/drop 270/0/0`, `leaves 0`.
- `crates/client/src/bin/gates.rs` — Bevy 0.18 behind the optional
  `render` feature (default **off**; the code tier stays ~106 s). Chase
  camera, reference plane, a cuboid per body. **Runs, and draws** — 30 s
  under Xvfb + lavapipe against a live shard, frame captured, session
  healthy throughout (`in ok/bad/drop 729/0/0`, `snap sent 434`). Item 2
  has the recipe.

**The rule that holds the pivot together: Bevy draws, it does not decide.**
`sim-core` keeps the walls, `ClientCore` keeps prediction; the ECS reads
those and writes transforms. Gameplay state in a Bevy component would
retire the determinism walls with nothing in CI to notice.

Next slices, roughly in order:

1. **Input** — keyboard/mouse into `ClientCore::set_input`. Every verb
   exists server-side; nothing native can press one yet.
2. **Terrain** — mesh `sim_core::terrain`. It is a pure function of the
   seed and both sides already agree on it, so this is meshing, not
   design. `web/src/terrain.js` is the reference for *what* to draw.
3. **A native visual gate** — item 2 below. The pivot's real debt.
4. **HUD, inventory, container panel** against the wire that already
   carries them (v19 `ACT_CONTAINER` / `SUB_CONT_SYNC`).

Retired by this pivot rather than finished: `MIGRATION.md` (three.js →
`WebGPURenderer` + TSL) is **moot** — you do not port three.js *and*
replace it. The lighting red (`TONAL_MAX_P10`) goes with it: it is the
coupled tonemap/sky/exposure/fog set, and a port re-derives that set. Also
retired — the shader-arithmetic, texture-photograph, shadow-distance and
capture-drift items. All are in git before this commit. `web/` still
builds and is still gated; nothing is deleted until the native client can
replace it.

---

## 1b · One call the shard owes the world lane — *(world lane, cross-lane)*

*(This sits BELOW the operator's §1 on purpose. The last two world-lane
items were filed as `## 0` above it and the judge's fix 4 on
`pass-20260805-074623-02` was right that position is a claim: a leftover
does not outrank a spoken call. Both of those items are now done — see
`DECISIONS.md` §open "probe coverage v0" and "the tier gradient is paid in
items, not density".)*

- **Systems lane, one boot-time call please.** `terrain::sites_complete(&Haven)
  -> bool` and `sites_live(&Haven) -> u32` now exist and are pure.
  `pick_minor` leaves `Waystation::NONE` when no candidate clears the
  separation floor; `tests/waystation.rs` refuses that over 16 seeds and
  `test_golden_covers_authored_sites` over 3, but a shard boots whatever seed
  `shard.toml` names, so a seed outside those 19 ships an island a third
  smaller with **no counter, event or log line**. The refusal — or a
  knob-registered relaxation of `WAYSTATION_MIN_SEP_M` until the ring fills —
  belongs in `crates/server` at boot. Judge fix 1, inherited twice now.
- **What the tier still lacks is a silhouette, not a price.** The containers
  differ now (`CacheSlot`, smaller and greyer than the pad's crate) and so do
  their tables. The site itself is still two boxes on bare ground; §4b below
  holds that item and its rule — a second copy of `HAVEN_SHELTER` would make
  the two tiers look identical, so it has to be a *different* structure.

---

## 2 · The native visual gate — the recipe exists, the gate does not

Every visual gate is browser-shaped: `browser_smoke`'s 12 probes, 43
`readPixels` sites, `vantages`, the capture harness. A native client
inherits none of them, and `MIGRATION.md` already stated the rule this
inherits — **a render path that lands without its probes ships a client
with no visual gates at all**, which is forbidden outright.

**The box CAN see — proven 2026-08-05, and this is the recipe.** The
earlier claim here (no display, therefore no native visual gate) was
wrong. The client ran for 30 s and a frame was captured off it:

```
Xvfb :99 -screen 0 1280x720x24 &
DISPLAY=:99 WGPU_BACKEND=vulkan \
  VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json \
  ./target/debug/gates 127.0.0.1:<port>
DISPLAY=:99 xwd -root -silent | xwdtopnm | pnmtopng > frame.png
```

`AdapterInfo { name: "llvmpipe (LLVM 20.1.2)", backend: Vulkan }` — Mesa's
lavapipe software rasterizer, no GPU needed. The session stayed healthy
throughout: `in ok/bad/drop 729/0/0`, `snap sent 434`, `leaves 0`.

So a native visual gate is **buildable here now**, and it is the next
gate to write. Two notes for whoever writes it: lavapipe is a CPU
rasterizer, so budget on frame COUNT and pixel assertions, never on frame
time (`CLAUDE.md`: a gate that waits on a clock is not a gate on this
box); and one live renderer at a time, since two was the browser tier's
whole problem.

**What the first frame showed, unfixed:** the body draws and is lit, and
there is no ground under it. The reference plane is at `y = 0` while the
player spawns at terrain height, so it sits far below the camera and out
of frame. Fixed by slice 2 of item 1 (mesh the real heightfield); until
then the plane is decoration in the wrong place.

---

## 3 · `browser_smoke` is red on a clean trunk, twice over

Both measured, both confirmed pre-existing on unmodified trunks, neither
caused by a diff. Kept because they explain why the renderer tier has not
run since 2026-08-04 — not because they are queued work:

- **tab B never reaches the world** — two live renderers on a box with no
  GPU. `__gatesDebug` never publishes, ~68–70 s of liveness cap, while tab
  A reaches the world in under a second in the same run. The
  two-live-renderers class `CLAUDE.md` names. Not a timeout to widen.
- **`TONAL_MAX_P10`** — p10 luma 112 against a ceiling of 60 (reference bar
  40.5). Retired by item 1.

---

## 4 · The event lane's payloads are law with no gate

Nine `EV_*` codes carry positional `u32` fields whose meaning lives only in
a `/// EV_*: a = … b = …` comment in `world.rs`. Swap `a` and `b` at an
`events.push` site and every wall stays green: the encoder is untouched
(`test_protocol_golden` green), the event queue is not in `state_hash`
(`test_replay` green), and every field is `u32` (clippy green).

This is the hole `reference/FINDINGS.md` §1 measured in the reference
ecosystem — 49 Oxide commits touching hook arguments, ~27 correcting a
payload that had already shipped wrong, four hooks corrected more than
once, and their `MSILHash` (the exact analogue of our golden) caught none
of them. `event_roles.rs` covers part of this now; finish it.

---

## 4b · The world lane: what the second tier left open

*(From the ranked gaps of `pass-20260805-053501-01` §3 and `-02` §2 — "one
place on the island worth walking to". `waystations v0` landed the placement
half: 3 authored sites on the ring instead of 1, gated by
`tests/waystation.rs`. What it did not close, in order:)*

- **A destination still offers no verb you cannot perform at your own base.**
  `-02`'s gap 2 names the recycler as the only one of `DESIGN.md` §2's three
  fixtures not blocked on an operator act (bank is A2/A3, vendor is skins).
  Container verb + `content/*.toml` yields — **systems lane**, and it is what
  turns a loot gradient into a reason.
- **The waystations are two crates on bare ground.** The pad got a greybox
  when the judge found the clearing indistinguishable from natural scenery;
  the lesser tier will read the same way once the novelty is gone. It wants a
  silhouette, and it must be a *different* one — a second copy of
  `HAVEN_SHELTER` makes the tiers look identical.
- **The pad carve is still unbuilt**, and it is smaller than this file has
  been saying. Counted this pass: `height` has **18 production call sites in
  3 crates** (world 6, movement 4, deploy 2, bridge 2, collide 1, build 1,
  survival 1, probe 1), not the "~80 in four crates" that stood here — the
  other 86 are tests and examples. `DECISIONS.md`'s "~50 sites" and its
  "~1,000 height taps per `haven()`" are both stale too; `haven()` measures
  **12,463 taps mean** over 16 seeds. Still cross-lane, but re-scope it
  against 18 before assuming it cannot be a pass. Sites are still *found*
  flat, not made — and `DECISIONS.md` §open (waystation canopy v0) records
  that whether a tier should carve at all is **open for the operator**.
- **Nothing threatens you on the walk between them.** A circulation loop with
  no risk on it is a longer commute. No AI module exists anywhere in
  `crates/sim-core/src/`.
- ~~Composition steps at a biome edge~~ — **done this pass**
  (`loop/scatter-splat-mix`): `scatter` blends the four biome rows by the
  ground's own splat weights instead of picking one, so the props ramp across
  a boundary the way the material and the clutter under them already did.
  Worst per-sample jump 4 per-mille against the classifier's 190; reaches
  10.2–11.8% of land cells; density unmoved. `DECISIONS.md` §open "scatter
  mix v0" has the measurements and the one operator question (the cliff term
  puts scree on steep walkable ground — say if props should ignore it).
## 4b · The domain gate reads the crate now — three residuals

Landed 2026-08-05 (`loop/domain-gate-whole-crate`), from the
`pass-20260805-074623-01-judge.md` ranked fixes 1 and 2. The domain gate
scraped **one file per domain**, so `DEATH_BY_ARROW = 3` in `combat.rs`
left all three checks green while `encode_event_death` still returned
`Err(Range)` — the 2026-08-05 failure, one module over, with the gate
written to catch it watching the wrong file. Reproduced red-then-green,
all three below.

Now: `SOURCES` reads all 22 `sim-core` modules, members carry their file
and must sit in the domain's declared `home`;
`the_source_table_covers_the_whole_crate` checks `SOURCES` against
`lib.rs`'s own `mod` list both ways; `every_enumeration_width_is_classified`
scrapes `event.rs`'s 33 `*_BITS` and forces each into DOMAINS or a named
magnitude list. No wire move — `PROTO_VER` 19, goldens green.

What remains:

- **§4's other half.** Role coverage is still 19 of `EV_MAX` codes, 8
  uncovered (`coverage_is_stated_not_implied`). The a/b swap gate is the
  unfinished part; the value gate is done.
- **`death_causes_are_a_closed_ledger`** (`event_roles.rs`) still scrapes
  `world.rs` alone. Narrow now — the protocol gate catches a stray value
  crate-wide — but its *contiguity* claim is still file-local.
- **§5b below** is untouched and still wants its own pass.

---

## 5 · Gameplay still missing, in rough order of what a player notices

- **Jump.** Gravity is there, jump is not — and jump is what makes a lintel
  matter. Wire change, so systems lane only.
- **Ranged.** There is a revolver in `loot.barrel` and nothing to fire it.
  `salvage/ranged-v0` is a judged-**FAIL** attempt (wall 6, the wire
  drifted, reproduced executably). Read the report before rebuilding.
- **Dropped loot** should land somewhere you can find, not inside the floor.
- ~~Base repair, decay and upkeep~~ — **all three exist**: `build::repair`
  reaches both stores, `deploy::decay_of` decays what is uncovered, upkeep
  charges every `UPKEEP_PERIOD_TICKS`. What is hollow is the other side —
  the satchel is priced, craftable and anchors the raid ratio with **no
  verb**, so repair defends against nothing faster than a hatchet
  (`pass-20260805-074623-04-judge.md` gap 2, ranked in -03 and -04).
- **Death and your own base** — a death evicted you from what you built and
  nothing you built said otherwise.

---

## 5b · The wire accepts values the sim can never mean

`every_domain_fits_its_wire_field` (`protocol/src/event.rs`) now gates ten
value domains against the fields that carry them — a sim domain outgrowing
its wire field is the shape of the 2026-08-05 FAIL, and it is caught now.
Writing it measured two live holes running the *other* way, left unfixed on
purpose: narrowing what decodes is a wire act, and that pass was a gate.

- **`BAG_GONE_*`** — `encode_event_bag_removed` bounds against the *width*
  (`why >= 1 << BAG_GONE_BITS`), not the domain (largest live is 2), and
  the decoder does not bound it at all. `why == 3` round-trips as a
  removal reason that means nothing.
- **`REFUSE_C_*`** — 4 bits for a domain topping out at 3, and neither end
  bounds the upper edge; only `reason == 0` is refused. Values 4..15 cross
  intact.

Both are forgery slack, not drift: the sim cannot emit either today, so
nothing is broken for a player. The fix is the closed-set posture
`DEATH_BY_*` now has — a derived `*_MAX` on the sim side, checked at both
ends — and it wants its own pass because it changes what decodes, which
means deciding whether a narrowing owes `PROTO_VER` a bump.

Systems lane (`crates/protocol`, `crates/sim-core`).

---

## 6 · Unmerged work, kept deliberately

Nothing judged PASS is stranded. These failed or were stopped, and the
harness kept them rather than merging. **Do not merge one to clear the
list** — failed work in the trunk is the one thing the judge exists to
prevent. If a lane rebuilds any of these, start from the branch.

| tag | what | why it is here |
|---|---|---|
| `salvage/ranged-v0` | ranged weapons | judged FAIL, wall 6 |
| `salvage/bark-photo` | bark texture | judged FAIL; textures retired by the pivot |
| `salvage/m1-surface-grain` | surface grain | stopped unmerged; same |
| `salvage/container-contents-wire` | container wire v19 | duplicate of what landed |
| `salvage/container-contents-2` | container wire v19 | duplicate of what landed |
| `salvage/cont-max-mirror` | `CONT_MAX` fix | absorbed; content-identical to `main` |

---

## 7 · Milestones

13. **M1 — survival verbs** + bags, hotbar, chat (`ALPHA.md` §1/§6).
14. **M2 — combat true**: lag-comp ring + rewound raycasts · ballistic
    projectiles · the anomaly log.
15. **M3 — economy dark + ops**: OBOL machinery behind the A1 switch · the
    claim rail · shard ops.
16. **A1 playtest** (operator schedules): 10–20 testers, one wipe cycle.
17. **M4 — arm A2, then A3** (operator acts): claim rail export · skin rail
    · the desktop launcher.
18. **Anti-ESP occlusion culling** — the measure the genre proved
    (Facepunch, 2025, network-wide default). Server-side, costs no client
    trust, and the occlusion grid is a pure function of the seed, so it is
    bakeable at worldgen and a lookup in the tick. Sequence after M2: it
    wants real sightlines to tune against.
19. **The launcher, in Rust, with the wallet in it** (`DECISIONS.md`
    2026-08-04). One static binary, `egui`, no webview: patcher, shard
    list, balances, and a self-custody wallet on `alloy` signing the
    EIP-191 join the server already accepts — so no protocol moves and
    nothing enters the sim's blast radius. Key backup is the feature, not a
    footnote: phrase shown once and confirmed back, encrypted keystore
    only, never logged and never in the WAL, and the plain sentence that
    the operator holds no keys and can restore nothing. **Unchanged by the
    client pivot** — it was always Rust, and it is the platform's client
    for the whole cascade, not a Gates accessory.
20. **`cargo test --workspace` overflows a debug thread's stack**; only
    `--release` (what CI runs) is green. Pre-existing. The cause is size,
    not logic — `World` is ~416 kB of fixed capacity and `ShardCore::new`
    builds it on the stack, so an unoptimized frame holds two or three
    copies against a 2 MB limit. It bites anyone who types the obvious
    command. Fix: box the big fixed-capacity members (`Pieces`, `Deploys`,
    `SlotLives`) at construction, the way `ShardCore` already boxes its
    client array — one allocation at boot, none in the tick.

Standing rule: anything a playtest breaks jumps this queue; anything a wall
catches jumps the playtest.
