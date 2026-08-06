# Gates · NOW.md — what next

The only list that answers "what should the loop pick up." Top item first.
Done items are deleted, not checked — history lives in git and
`DECISIONS.md`. A loop iteration starts here, ends with gates green.
An item is ≤ ~25 lines (`CLAUDE.md` §loop discipline); detail belongs in
`DECISIONS.md` §open or a `gates-loop/findings/` note.

> **Rebuilt 2026-08-05, then pruned again the same day.** The file had
> reached 2040 lines: `merge=union` means three lanes append and nothing
> ever deletes, so it accumulated ~12 items whose own titles said "done this
> pass", a duplicate, and a large block of browser-renderer work the client
> pivot retires. **Nine more "done this pass" items had accumulated by that
> evening** — 966 lines again within hours of the rebuild, which is the
> merge strategy and not the lanes' discipline. Pruning is therefore
> recurring maintenance, not a one-off: if it is not automated, budget it
> every few passes. Everything removed is in git. Nothing open was dropped
> — §0q exists because two judge-ranked gaps were written down **only**
> inside a done item and would have gone with it.

---

## 0t · the native pine is generated — what it bought, and what it owes

**Landed.** `crates/client/src/render/tree.rs`: the near-ring conifer is
`bevy_procedural_tree` (MIT/Apache, ez-tree's algorithm in Rust — the same
generator `web/src/props.js` already depends on), used as ONE pure function
returning two meshes. No plugin, no ECS. `props.rs`'s whorl builder stays as
the far-LOD silhouette. Gate: `crates/client/tests/tree.rs`, 6 assertions,
headless.

Three things it settled, all measured against a frame rather than argued:

- **`BranchForce` pointing down is a trap.** The crate builds one global
  `Quat::from_rotation_arc(Y, dir)` and slerps every section toward it, so
  `dir = -Y` hits the antipodal singularity and bends the whole tree sideways.
  Droop is the limb ANGLE's job. Owed upstream as a bug report.
- **Card AREA, not card count, is what closes a canopy.** 11 cards of 0.18 m
  measured inside every bound and rendered as a spindly stick, because the
  needle mask cuts ~60% of every card away. Coverage 1.20 → 16.0 at 16 cards
  of 0.55 m on shorter limbs, with radius unchanged. Only the capture said so.
- **Radius is a distribution, not a number.** 1.75 m limbs measured 1.65 on one
  seed and 1.717 — over `PINE_MAX_R` — on another. Swept over 11 seeds; the
  shipped value holds at 1.464 with ~14% margin, the same margin `props.js`
  took for the same stated reason.

**Owed, in rank order.** (1) The billboard LOD — 328 trees × 5.9 k tris is
1.9 M against DESIGN §9's 1.5 M, so the full ring is knowingly over budget and
only the ~80 m band is affordable; `tests/tree.rs` prints the arithmetic.
(2) `aWind` — `StandardMaterial` cannot read a custom attribute, so wind needs
the custom material `RENDER.md` already lists. (3) The needle card is generated
(`tree::needle_image`, like `sky.rs`'s cubemap); a photographed sprig is a
later swap, not a prerequisite.

## 0u · the frame budgets are browser numbers and nobody has re-derived them

**Doc pass landed** (`DESIGN.md` §9, `RENDER.md` §6, `ART.md` §7,
`TERRAIN.md` §4/§6, `NETCODE.md` §4, `CLAUDE.md` traps): every performance
claim now says which platform it was chosen for. What it found is one real
open question, and it is not a doc problem.

`DESIGN.md` §9's four budgets were all set for a WebGL page. Three of them
no longer describe what constrains us:

- **initial load < 15 MB** and `ART.md` §7's **12 MB texture payload** are
  the same number: a first-visit *download*. A depot install is not one, so
  the constraint is gone and 2K/4K re-sourcing is unblocked. What is real
  natively is VRAM and disk, and nothing has measured either.
- **< 300 draw calls / < 1.5 M tris** are WebGL-shaped. Bevy's automatic
  batching and a native wgpu backend are not bound where a WebGL context
  was, and two shipped numbers are already rationed against the 1.5 M:
  `CLUTTER_RICH_PER_TILE = 96` (a 20% share) and the conifer ring's
  "over budget" verdict (1.9 M).
- **60 fps on a mid laptop iGPU** survives — a hardware floor, not a
  platform one.

**Nothing was renumbered.** These are `(knob)` and therefore spoken, and a
budget raised by the loop that then justifies the loop's own triangle count
is exactly the wrong direction of travel. The measurement is small: capture
on a real GPU at the ring's p90 tree count, read draw calls and frame time
off `RenderDiagnosticsPlugin` (its wall-clock half is not assertable —
`CLAUDE.md`), and propose into `DECISIONS.md` §open. Related: the anisotropy
ceiling `BASE_ANISOTROPY_MAX = 4` was set because *"a second browser tab did
not reach the world at all on this box"* — the reason is a software
rasterizer running two tabs, and it does not transfer.

## 0c6 · systems lane request: bridge `terrain::haven(seed)`

One export. `terrain::haven(seed)` already returns the pad and the waystation
array in one struct (`terrain.rs:799`); nothing in `client-wasm/src/bridge.rs`
exposes it, so a client learns a destination exists only by standing in its
chunk. `map.js`'s `resolveMarks` takes world positions and is already gated, so
this is a caller change on the ui side and not a rewrite. Ranked gap 1 of
`pass-20260805-111501-04` is the reason; the container verb is the other half.

## 0a · world lane: skirt residual — the ring's hard edge

*(Residual 1, the sand sweep, landed and is deleted. Residual 2 is below and
its cost has not changed, but its blocker is now named properly.)*

**The clutter ring still ends hard** at ~32–45 m rather than thinning into the
fog. `web/src/clutter.js` names why it is parked: the cheap fix is a per-frame
player-relative shader term, which is a new program, and the prewarm gate
counts program links after `inWorld`. The recipe: thin stochastically by
instance hash (so the same elements survive at a given range and nothing pops),
then scale survivors to zero. Budget and prewarm the program, or drive it off
the existing shared wind uniform.

**It is blocked on evidence, not only cost.** `clutter.js` asks whether the
edge reads at all at 32–48 m and answers "a question for the visual judge, not
for a guess made here" — and no frames are captured for this lane and no visual
judge scores it. So nobody can say whether the edge is worth a program link.
**This is the flip the old item predicted: it is now "does it look right", not
"is there one", so moving it needs frames back on this lane first.**

One finding from residual 1, kept because a later pass would otherwise fix it
in the wrong file: beach skirts are thin — 1.19 elements a tile against
inland's 5.27 — because `scatter` puts 0.22 prop centres a tile on the coast
against 0.95 inland, not because the skirt thins itself. The two ratios match
to a tenth. That is the scatter table, not `terrain.rs`'s skirt path.

## 0q · The judge-ranked gaps nobody has claimed

Lifted out of a "done this pass" item before it was pruned (2026-08-05) —
it was the only place two of these were written down. Both merge-gate
judges rank this set above everything else in the repo, and all of it is
`crates/`/wire work no single-surface lane may take.

1. **Shore barrels as a second destination class.** The road now pays
   unevenly (§0a2's bays landed), and the haven pad is the one place worth
   walking to. A second class on the shore would give the ring two ends
   rather than one. Nothing else in this file mentions it.
2. **The wipe.** Named by both judges, described nowhere. It is a shard
   lifecycle act with an economy half (`ALPHA.md` A1→A3) and an operator
   half (`CLAUDE.md`: wipes of a live shard are operator-only), so the
   loop's share is the mechanism, never the trigger. Needs scoping before
   it can be an item.

The other four in that set are already carried: the satchel's raid verb and
its structure-damage path in §0r, the bow in §5, `jump` in §5.

---

## 0r · The raid loop has offence now — what it still cannot do *(from `findings/pass-20260805-113001-02-judge.md` gap 1 and `-03-judge.md` gap 1, both ranked first)*

Landed (systems): `sim-core/charge.rs` — plant the held throwable at an
address, fuse from content, damage on the tick it runs out through the same
`damage_piece`/`damage_deploy` a swing uses. `ACT_THROW`/`EV_CHARGE_PLACED`,
`PROTO_VER` 23. Knobs in `DECISIONS.md` §open ("satchel fuse v0", "raid wire
v23"). `bake_combat` no longer drops throwable rows, so `balance.toml`'s raid
ratio finally divides by a number the sim holds.

Remaining, in order:

1. **No key plants one — the ui lane owns this.** `client_action_throw` and
   `client_charge_{key,info,fuse}` are exported; nothing in `web/src` calls
   them and nothing draws a countdown on a charged wall. `APPLIED2_CHARGE`
   is the flag to poll. Without it the raid is gated and unplayed.
2. **No blast radius** (systems) — **the content half landed 2026-08-05,
   the arithmetic did not.** `blast_m` is schema'd, validated, refused at
   zero, baked to `ThrowDef::blast_cm` and walked into `canon::hash`; the
   knob is `DECISIONS.md` §open ("satchel blast v0", PROPOSED at 3 m).
   **Nothing reads it** — `charge.rs` is still `place` + `tick_fuses`, so a
   charge damages only the address it was planted on and the anchor's
   arithmetic stays exactly `piece hp ÷ structure`. What remains is the
   falloff and a bounded multi-target scan; `combat::raid`'s 3x3
   column-index ring is the shape to copy. The content hash has already
   moved for this, so the cost is paid whether or not the slice is taken.
3. **Nothing is hurt by standing in one** (systems). `ThrowDef::damage` is
   now carried for throwables instead of discarded by the bake, but
   `EV_CHARGE_PLACED` still has no player-damage half, so the defender's
   seconds are free to spend standing on the charge. Fourth `DEATH_BY_*`
   if taken. Lands with 2 or not at all — they share the falloff.
4. **The action lane is full** (systems, register not work). `ACT_MAX` = 15.
   The next C->S verb widens `ACTION_SUB_BITS` and regenerates all 74
   goldens — read `the_action_lane_has_the_room_it_claims` before proposing
   one.

## 0i · The checklist landed — two things it did not do *(ui lane)*

§0c is closed: `LEARN_TASKS` names all eleven verb keys in the world, each row
struck through on first use, and `ui_smoke` group Y classifies all 21 key
literals in `main.js` + `input.js` — a twelfth bind is red until the player is
told about it. 1731 → 1861 checks; mutants M41–M48, all red.

What it did not do:

1. **No keybinds SCREEN.** `MENUS.md` surveys one and the reference ecosystem
   exposes one; the checklist teaches the default binds and cannot rebind
   anything. That is a bigger item and it needs an operator word on whether
   binds are persisted (localStorage is the obvious answer and is not mine to
   invent). Not blocking: nothing is undiscoverable now.
2. **`ci/ui_mutants.sh` staleness is still ungated** — judge -04's ranked fix
   2, untouched here. `e743b59` shipped four stale mutants and nothing was red
   until a human ran the script; this pass added eight more anchors that can
   rot the same way. Wants a `--check` mode (parse `run_mut`, assert each
   anchor matches its file exactly once, no writes) wired into `gates.sh`'s
   code tier. Wrinkle the judge already found: M23's `to` argument is the
   empty string, so a naive whitespace filter drops the record.

Seen, but only as DOM: a throwaway browser built the real `Hud` over the
shipped `index.html` and screenshotted the overlay — it sits top-left, clear
of the compass, ticks where it should. That is diagnosis, not evidence. No
shard, no renderer, no capture; `browser_smoke` is off this run, so nothing
above rests on it.

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

- ~~Nothing can press it~~ — **landed in pass-20260805-111501-04, see §0h.**
  R presses it, the prompt is fourth in `CENTRE_ORDER`, and the store bit is
  gated. The native client still picks it up with §1 slice 1.
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

**It ships 2026-08-05.** `ci/depot.py` packages the build as a scry depot and
`crates/client/src/{args,scry}.rs` give it the launcher's interface —
`--server`/`--identity` from a depot's launch block, and the vendored scry SDK
for who is playing. Run end to end: real depot served over HTTP,
`scry install gates` by slug, hashes verified, digest equal across
packager/origin/client, and the installed binary joined a live shard (3 joins,
2380 inputs, 0 bad, 0 dropped). Publishing and notarizing are operator acts and
are NOT done.

Two findings, both from running it rather than reading it:

- **The depot ships `assets/`, not just the binary.** The render slice loads
  `textures/*` at runtime and Bevy answers a missing texture with a white
  fallback *and keeps drawing* — so a binary-only depot would install
  perfectly, start perfectly, and render untextured. Same failure shape
  `gates.rs` already documents for the asset root. The packager refuses to
  build a depot with no assets, and `--self-test` covers it.
- **A dirty tree gave two different binaries the same build id.** A build id
  is a directory name on a player's disk; it is now keyed on the binary's own
  hash when the tree is dirty.

The IPv6 endpoint bind that killed the packaged build at startup was fixed
independently on `main` (`3c50e35`) while this was in flight; that fix is the
one in the tree, and this lane's duplicate was dropped at the merge.

Next slices, roughly in order:

1. **Input** — keyboard/mouse into `ClientCore::set_input`. Every verb
   exists server-side; nothing native can press one yet. **This is now the
   top gap**: a player can install and start the game and cannot move in it.
2. **Terrain** — mesh `sim_core::terrain`. It is a pure function of the
   seed and both sides already agree on it, so this is meshing, not
   design. `web/src/terrain.js` is the reference for *what* to draw.
3. **A native visual gate** — item 2 below. The pivot's real debt.
4. **HUD, inventory, container panel** against the wire that already
   carries them (v19 `ACT_CONTAINER` / `SUB_CONT_SYNC`).
**The visual plan is `RENDER.md`**, and **R0–R6 plus R8 have landed**: input,
the capture harness, the terrain mesh, the light rig under one owner, the
scatter and clutter population *and its prop skirts*, the CC0 photograph on
the ground, SSAO + SMAA + bloom, a procedural cloud deck, the HUD and the
viewmodel.

Measured both sides through one estimator (`ci/native_bar.py`, medians over
six vantages against the six outdoor-daylight reference frames): p50 **90.2**
vs 91.4 · near **79.8** vs 80.5 · saturation **32.9%** vs 33.2% · p90 **155.7**
vs 170.2 — and near-ground neighbour contrast **0.26 → 6.25** against the
reference's 5.40, with chroma-per-luma 0.163 against 0.252 confirming that is
texture and not aliasing. `ART.md` §3's row that six browser passes never
moved is past the bar.

What remains, ranked by the measurement — `RENDER.md` §8 carries the list:

1. **The gate asserts.** The harness captures and the bar measures; neither
   FAILS yet, and nothing in `ci/gates.sh` runs either. Still the pivot's debt.
2. **p10 58.6 vs 41.0** — a uniform ambient buys rule 3's floor at the price of
   the darks. A hemisphere fill gets both.
3. **Cloud form** — the deck reads as stratus where `ART.md` asks for cumulus.
4. **The four-way splat material** — one map serves all four ground identities
   today (`StandardMaterial` has one base-colour slot).

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
gate to write. **Its design is `RENDER.md` §5** — the capture protocol, the
vantage list, and the assertion order (structural claims before any
statistic, because a beige smear passed all 36 of `vantages`' checks). Two
notes still: lavapipe is a CPU rasterizer, so budget on frame COUNT, never
on frame time; and one live renderer at a time. Prefer Bevy's off-screen
readback to the `xwd` above — the recipe proves the box can see, but `xwd`
is absent on some boxes and a gate should not need a window server.

**The nearly-free half is done: the render feature compiles under a lint
gate now.** `cargo clippy -p client --features render --all-targets -D
warnings` is green, and it caught three findings on its first run. Before it,
`render` was off by default and `[[bin]] gates` is `required-features =
["render"]`, so cargo skipped the file — reproduced on a throwaway crate, the
skipped bin held `this is not rust at all !!!` and `cargo clippy --all-targets
-- -D warnings` exited 0. **It is not in `ci/gates.sh` yet**; the native
renderer tier that runs it, `gates --capture` and `ci/native_bar.py` is what
this item now is.

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

Reconfirmed 2026-08-05: tab B failed on the `programsAtInWorld` assertion at
87.9 s, on the salvage pass's branch AND identically on unmodified `main`.
Fourth confirmation, same class.

---

## 3b · `backpack_wire` cannot run in debug — found 2026-08-05

`cargo test -p server --test backpack_wire` aborts with a **stack overflow**
in `a_kill_puts_a_bag_on_every_client_and_the_loot_takes_it_off`. Identical
on a clean tree, so it is not a diff. It is invisible to CI because
`ci/gates.sh:164` runs `cargo test --workspace --release`, and the release
frame layout fits — so the gate is green and the suite is still unrunnable
the way a developer runs it.

Not "widen the stack": a test whose frames only fit under optimization is
one refactor away from failing in release too, and the wall it guards
(bag-on-death reaching every client) would go with it. Worth finding what
is actually oversized — likely a `World` or fixture held by value — rather
than raising `RUST_MIN_STACK` and calling it fixed.

---

## 4 · The event lane's payloads are law with no gate — 24 of 29 now gated

Swap `a` and `b` at an `events.push` site and every wall stays green: the
encoder is untouched (`test_protocol_golden`), the ring is not in
`state_hash` (`test_replay`), every field is `u32` (clippy). The hole
`reference/FINDINGS.md` §1 measured in the reference — 49 Oxide commits on
hook arguments, ~27 correcting a payload that had already shipped wrong.

**Landed (systems, `loop/event-refusal-roles`):** the refusal family —
`EV_CRAFT_REFUSED`, `EV_BUILD_REFUSED`, `EV_DEPLOY_REFUSED` — has role
checks. 55 of the lane's 103 emit sites, previously unroled, all shaped
`(player, reason, 0)`. Two causes per code so `b` is proven a channel and
not a constant. Trap found and written into the fixture: `BUILDER` is id 4
and `REFUSE_B_REACH`/`REFUSE_D_REACH` are both ordinal **4**, so the
obvious out-of-reach cause is the one case where a swap reads green.

Also: `coverage_is_stated_not_implied` could lie in both directions and no
longer can. `COVERED`/`NOT_COVERED` name each code as well as numbering it,
both are cross-checked against `world.rs`'s declarations, and coverage must
be witnessed by a real `only(&w, EV_*)` call. Five mutations proved each
gate red — a swapped emit site, an unearned claim, a name/value transpose.

**Remains:** 5 codes with no role check — `EV_SLOT_RESPAWNED`,
`EV_WEAK_MARK`, `EV_CRAFT_DONE`, `EV_BAG_REMOVED`, `EV_RESPAWN`. The last
three are cheap (`bag_respawn.rs` and `gather.rs` already drive the
causes); `EV_SLOT_RESPAWNED` needs a respawn timer to elapse and is the
one worth its own look. Note `CLAUDE.md`'s trap list still reads "law with
no gate" flat — left alone deliberately, it is a shared doc mid-run.

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

- **Jump — the sim half landed, the key does not exist yet (ui lane).**
  `BTN_JUMP` (bit 3) and `JUMP_SPEED` ship and are gated (`tests/jump.rs`,
  and the bots jump so it rides alloc/replay/parity), `PROTO_VER` is 22.
  **Nothing presses it:** `web/src/input.js:193` assembles the button byte by
  hand as `(sprint?1:0)|(primary?4:0)`, so the ui lane adds a Space keybind
  and `|(jump?8:0)` there — one line, and until it lands jump is unreachable
  in play. Prediction needs nothing: `predict.rs` runs the same
  `movement::step`. The native client (`crates/client`) needs the same bit in
  its own input path when §1 slice 1 lands.
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

One tag is left and it failed. **Do not merge it to clear the list** —
failed work in the trunk is the one thing the judge exists to prevent.

| tag | what | why it is here |
|---|---|---|
| `salvage/ranged-v0` | ranged weapons — `ranged.rs` (402), `pitch_lut.rs` (285), `tests/shoot.rs` (695), `ci/gen_pitch_lut.py` | judged FAIL, wall 6 — and the branch's own `NOW.md` text says why: *"already on the wire, so the wire did not move and `PROTO_VER` did not bump"*. A shot arrives as `EV_HIT`/`EV_HEALTH`/`EV_DEATH` and nothing else, so **no client can tell an arrow from a swing and nothing can draw the projectile**. The wire half it names as missing — an `EV_SHOT` code, its subtype, a `PROTO_VER` bump, 66 regenerated goldens — is the rebuild's whole scope. **It does NOT need a new action code**: the bow fires on the existing `BTN_PRIMARY`, adds no `ACT_*`, so `ACT_MAX` being full (§0r item 4) does not apply and `ACTION_SUB_BITS` does not move. An earlier note here said otherwise and was wrong. The judge report was pruned; this reading is off the diff. **The sim half is good work and survives the desktop pivot untouched** — pure `sim-core`/content, nothing in `web/` or `crates/client`, bounded (`MAX_ARROWS` 128, `MAX_ARROW_LIFE_TICKS` 120, integer `ARROW_STEP_MM`), 695 lines of tests. Four rebase conflicts against current `main` (`bake.rs`, `combat.rs`, `limits.rs`, `world.rs`), all in files that moved under it. **Start from the branch — this is a slice, not a rewrite** |

Cleared 2026-08-05 (operator pass, not the runner). Every dropped tag's
work is still held by its `loop/*` branch — the tags went, the commits did
not:

- **`bay-slots` landed** (`87e7fea`). Reviewed and merged unjudged: it is
  pure `sim-core`, keeps the yaw LUT (wall 1), redistributes rather than
  raises so `HAVEN_PRIZE_RATIO_MIN` is undisturbed, and regenerates the
  terrain golden in the same commit. Full `ci/gates.sh` green.
- **`blast-radius` landed** (`8f2623d`, corrected in `3c79ad2`) as inert
  plumbing. Three defects fixed on the way in: an invented knob (its toml
  comment cited a `DECISIONS.md` row that did not exist — now written as
  *satchel blast v0*, PROPOSED), five comments asserting a
  `charge::falloff` that does not exist, and a silent contradiction of
  `satchel fuse v0`'s stated scope. **Nothing reads `blast_cm`**; the
  falloff is the next slice, and the content hash has already moved for it.
- **`container-contents-wire`, `container-contents-2`** — dropped as
  duplicates; wire v19 is in trunk via `loop/container-sync`. Worth
  knowing: the **judged-PASS** branch was parked and the **unjudged** one
  merged (`bd62d33`, "NO JUDGE SCORED THIS"). Outcome fine, route backwards
  — which is why the old preamble here claimed nothing judged PASS was
  stranded while `container-contents-2`'s own tip was a PASS merge.
- **`cont-max-mirror`** — dropped, verified absorbed: `ci/ui_smoke.mjs:1368`
  carries the identical `rustConst` alias-resolver.
- **`container-sync`** — dropped, already an ancestor of `main`. It was
  never listed here at all.
- **`bark-photo`, `m1-surface-grain`** — dropped. Both are `web/` texture
  and material work the native pivot retires (§1).

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

## 5c · The protocol golden has never fuzzed a button above bit 1 *(systems lane)*

Found while landing jump (§5). `goldens.rs:262` draws the input fixture's
`buttons` from `rng.next_bounded(4)`, so `v22_input_full.bin` exercises only
`BTN_SPRINT` and `BTN_CROUCH`. `BTN_PRIMARY` has been outside that draw since
M1 and `BTN_JUMP` is outside it now — the field is 8 bits wide either way, so
the golden still pins the *layout* correctly and nothing is currently wrong on
the wire. What it cannot see is a future encoder that masks or reorders the
high nibble.

Deliberately not fixed in the jump commit: widening the draw changes fixture
bytes, and changing golden bytes for a reason unrelated to the version's
meaning muddies the one signal wall 6 reads. It wants its own commit, where
"the bytes moved because the fuzz widened" is the whole story — and that is a
`PROTO_VER` judgement call, because the answer may be that a golden's fuzz
range is not part of the wire contract at all and the bytes may move without a
turn. Decide that first; it is the actual question.

Same shape one level down: `decode_input` reads all 8 bits with no domain
check, so bits 4–7 round-trip as meaningless buttons. That is §5b's forgery
slack, not drift, and belongs with §5b's pass.


## 8 · UN-RECONCILED: the remote trunk's NOW.md, as the union merge left it

`ef18529` merged `origin/main` (PR #10 and two doc commits). `NOW.md` is
`merge=union`, so ~1275 lines of the remote trunk's own item list were
appended verbatim — and landed INSIDE §7, ahead of the milestone list.
They are moved here intact. **Nothing has been deleted and nothing has been
judged.**

**This is not a queue. Do not pick from it.** It is two divergent trunks'
work stapled together: many items are marked LANDED and several restate
items §0–§6 already carry in their current form. Reconciling it means
deciding item by item which is still true, which is why it was not done in
the merge that created it.

**RESOLVED by the operator, 2026-08-05: "we are focusing on desktop rust
build now."** This section was holding open a conflict between §1's native
pivot and a "three.js stays for the web demo" row. The word settles it —
**desktop is the build, web is the demo** — so the three.js items in here
are **deprioritized**, not merely un-adjudicated. `DECISIONS.md` has the
row. The web client is not deleted and its gates still run; it is simply
not where work goes.

Read that together with the same operator's verdict on why this section is
full of visual passes: **they were wheel-spinning.** Not a discovery about
content density — wasted motion, measured against having built worlds with
this tooling faster and with less circling. **So the bar for any visual
work that follows, on the native client, is a visibly better picture in
reasonable time.** A pass that produces tuned constants instead of that is
the failure mode to stop early, and this section is 1,100 lines of what
that failure mode looks like when it is allowed to run.

**Two things in here outrank the rest and should be lifted out first:**

- **"The world is empty, and that — not shading — is why it reads plain"**
  *(Operator, 2026-08-05, on captured frames: "the screenshots look like
  trash lol I think we spun our wheels".)* Measured against
  `Rust Images/genericview.jpeg`: the gap is mesh density and content —
  grass, understory, branch structure — not pixel math, which is why eight
  passes of surface-field work could not close it.
- **"`ci/vantages.mjs` passes frames that contain no scene"** — 36 checks
  green on a beige smear with no sky, no horizon and no object, scoring the
  highest detail of four vantages. `CLAUDE.md` now carries this as a trap;
  §2's native visual gate must not inherit the hole.

1. **`main` is RED: the pine's prop contrast sits exactly on its floor.**
   *(Operator, 2026-08-04: land the wind + felling lane anyway and record it.
   `DECISIONS.md` §Spoken. This is the one item that outranks everything below
   it, because every pass after it inherits a red `ci/gates.sh`.)*
   **Measured, two independent runs, same value.** `browser_smoke`'s prop
   probe: pine `contrast x1.15` against `PROP_MIN_CONTRAST_RATIO = 1.15`,
   asserted with `>=` on a value that rounds onto the floor. Before the wind +
   felling merge the same probe read **x1.22 at mask 15.22%**; after it,
   **x1.15 at mask 11.53%**.
   **The cause is the denominator, not the field.** This ratio is
   `(baseline + added) / baseline`, and the baseline is the flat state's own
   detail — facet edges, the vertex ramp, the shadow map. Pine geometry v1's
   five whorls put more structure into that flat state, so the same field
   divides by more. The floor's own comment predicted exactly this ("a prop
   with structure of its own can never score what a smooth heightfield does")
   and was calibrated at x1.26 on the four-primitive canopy that no longer
   exists. `gmHash4` is NOT the cause: reverting it leaves the value at x1.15.
   **The fix is the pine's field against its new silhouette, single owner** —
   not the floor. Lowering `PROP_MIN_CONTRAST_RATIO` is inventing a knob to
   pass a gate, which is the one move the merge rubric exists to catch. If the
   floor is genuinely wrong for a whorled canopy, that is a measurement and a
   spoken number, not an edit.
1. **The world is empty, and that — not shading — is why it reads plain.**
   *(Operator, 2026-08-05, on captured frames: "the screenshots look like
   trash lol I think we spun our wheels".)*
   **Measured by looking.** Four `ci/vantages.mjs` captures against
   `Rust Images/genericview.jpeg`. Ours: a heightfield, pines that are stacked
   cones, rocks that are flat-shaded dodecahedrons, a cloudless gradient sky.
   Theirs: thousands of instanced grass blades as real geometry, an understory
   of bushes and flowers, trees with branch structure and leaf masses, a stone
   building, water, a character. **The difference is mesh density and content,
   not pixel math** — which is why eight passes of surface-field work (61 tuned
   constants, triplanar fields, analytic gradients, chroma neutrality) could
   not close it. `CLAUDE.md`'s "a judge names the symptom; fix the cause",
   second instance.
   **The work, roughly in order of visible return**: ground-cover instancing
   (the single largest gap in the comparison) · an understory layer · trees
   with branches instead of cone whorls — `@dgreenheck/ez-tree` is already a
   dependency and `PINE_VARIANTS = 1` says how little of it we use · a second
   species. Each is an instanced-geometry budget question against DESIGN §9's
   1.5 M triangle cap (frame peak measured 1.06 M), not a shader question.
   **Not this item**: any renderer change. Today's frames settle that TSL and
   wgpu both address the wrong cause (`DECISIONS.md` 2026-08-05).
1. **`ci/vantages.mjs` passes frames that contain no scene.** *(Found
   2026-08-05 while capturing the above.)*
   `slope.png` — a beige-and-blue streaked smear with no sky, no horizon and
   no object in it — **passed all 36 checks and scored the highest detail of
   the four vantages**: `detail 14.28 luma/px (flat 4.34, x3.3), chroma
   0.111`. The worst-looking frame scored best.
   The cause is that every assertion is a per-pixel statistic — contrast
   ratio, chroma spread, luma neutrality — and none of them can see whether
   the frame is a picture of anything. A gate that green-lights a sceneless
   frame is why statistics kept improving while the frames did not.
   The fix is a structural assertion, not a tighter threshold: a vantage must
   contain sky above a horizon and a countable number of distinct objects
   (props `terrain.nearestProps` already resolves) before any pixel statistic
   is read. A vantage that frames neither is a FAIL, never a skip — the same
   posture prop probe 15f already takes.
1. **Tab B should be a bot, not a second browser.** *(Operator, 2026-08-04:
   "i think we need the tab stuff every few hours at this rate". The tiering
   half landed; this is the half that removes the flakes. `DECISIONS.md`
   §Spoken.)*
   **Evidence, measured the night it was spoken.** Eleven gate failures across
   seven runs: **nine were the harness fighting itself** — four dev-shard bind
   races against a previous run still releasing port 4460, five tab B flakes
   (connection closed, chat unheard, and three 60 s timeouts, one of them
   reporting `inWorld=true` and timing out anyway, which is verbatim the clock
   bug `CLAUDE.md` names). Two were real findings. A gate whose failures are
   82% environment is measuring the box.
   **What tab B uniquely asserts, and where each belongs.** Mutual AOI and the
   remote walk, chat local/global routing including the 20 m radius negative,
   `snapshots > 0`, zero oversize datagrams. All but the last are **netcode**,
   and `crates/server/src/bin/bots.rs` already drives that path natively —
   `DECISIONS.md` records the client's netcode core as pure and native-tested,
   sharing code paths with the bot client. Move them to a bot-driven check and
   they become deterministic and seconds-long. The datagram clamp stays in a
   browser (it is a browser-specific `maxDatagramSize` behaviour) but needs
   only tab A to send.
   **Then one tab is the whole browser gate**, and the two-tab case survives
   only as the joining-cost check — which must assert on program links after
   `inWorld`, never on elapsed milliseconds.
1. **The renderer moves to `WebGPURenderer` + TSL.** *(Operator, 2026-08-04:
   "for the record i am upgrading asap the graphics". `DECISIONS.md` §Spoken.
   The costed plan is `MIGRATION.md` — read §6 before picking this up.)*
   *Read before picking this up. Two things changed on 2026-08-05. The browser
   client became **second class** (direction: desktop primary, web the demo),
   so this single-owner migration is aimed at the demo. And the captured frames
   say the visual gap is **content density, not shading** — cone trees and
   dodecahedron rocks against a reference full of instanced grass and branched
   canopies — so no renderer change scores against it. Item 2 is the one that
   does. This is **not** held: the 2026-08-04 word stands until the operator
   moves it. But it is not next, the sequencing is an open knob
   (`DECISIONS.md` §open), and a pass this size should be confirmed first.*
   Four steps, in order, because each one done later costs more. **Do not
   compress them into one pass.**
   0. **Bump three `0.178.0` → `0.185.1`, alone, on WebGL.** `shadows.js`
      throws at boot if three renamed a shadow uniform; read that on a clean
      tree, not inside a rewrite.
   1. **Port the 12 probes to render targets + async bodies, still on
      WebGL** — 43 `readPixels` sites, 126 `browser_smoke` references, and
      every existing assertion must prove the port changed no number. The
      centre of gravity: after the swap instead means a window with no visual
      gates at all. `farShadowProbe` needs its corner math re-derived too.
   2. **Re-derive the prewarm COUNT** (`renderer.info.programs.length` has no
      WebGPU equivalent); prove it catches the same event class.
   3. **Swap the renderer and rebuild the material path together** —
      `scene.js`, `materials.js`, `shadows.js`, `terrain.js`, `main.js`. One
      owner, one lane, no parallel loop. `CSMShadowNode` and
      `TileShadowNode` ship with three as worked references; its
      `transpiler/` converts our GLSL bodies mechanically.
   Visual work (clouds, `SkyMesh`, GTAO) is step 4 and not a prerequisite.
   Mixing it in is how a renderer swap becomes unreviewable.
1. **The projection's own arithmetic, twice — and both were Quilez's rules,
   stated in his article, shipped wrong here first. — LANDED**
   *(`DECISIONS.md` §open, "materials v5". Operator, 2026-08-04: "figure out
   where the math we are using is wrong".)*
   materials v4 put the base maps on a fall-line biplanar projection and the
   cliffs still streaked. The cause was not the projection, it was two
   arithmetic errors inside it:
   - **The wall tap's footprint was differentiated after the frame instead of
     before it.** `gmAcross` is per-fragment, so `dFdx(dot(p.xz, across))`
     expands to `dot(dFdx(p.xz), across) + dot(p.xz, dFdx(across))`, and the
     second term is the frame turning, multiplied by a WORLD coordinate
     (~1568 here). A 1e-4 rad/px rotation injects 0.16 m/px of fake footprint
     against a true ~0.002 — `textureGrad` picked a mip about **seven levels**
     too coarse, in bands following the terrain's curvature.
   - **The plane blend had no sharpening exponent.** cos and sin are the two
     planes' foreshortenings, so a linear blend at 69.5° hands **32.3%** of
     the sample to the top plane while that plane is stretched **×2.86**.
   Fixed at `BASE_WALL_SHARPNESS = 8.0` (Quilez's own stated value) and by
   projecting `dFdx(position)` onto the frame. Measured: near-cliff neighbour
   contrast **7.42 → 14.58 luma/px**, far cliff **2.88 → 4.35**, and the new
   vantage gate's slope chroma **0.705 → 0.127** — from double its ceiling to
   inside the reference band (0.077–0.193). Every vantage at or under 45° is
   bit-identical, because the wall tap does not run there.
   The bump's own clamp saturation went **68.0% → 4.9%** of a near cliff in
   the same pass, from the surface-gradient reformulation plus a per-octave
   share of `BUMP_MAX_SLOPE`.
1. **The event lane's payloads are law with no gate — close the other
   twenty codes.** *(Operator, 2026-08-04: top priority. The first five
   landed with `test_event_roles`; this is the rest of the ledger.)*
   **The hole, stated once.** Every event is `push(code, a, b, c)` over
   three untyped `u32`s, and the `/// EV_*: a = … b = …` lines in
   `world.rs` are the only statement of which is which. Swap two at an
   emit site and every wall stays green: `test_protocol_golden` pins the
   *encoder's* bytes and an emit site is not the encoder; `state_hash`
   excludes the event ring by design (derived output, not sim state);
   every field is a `u32`, so the swap type-checks. `EV_DEATH` is `a` who
   died, `b` who killed — swap those and every kill feed on every client
   credits the corpse, silently and forever.
   **Why this outranks the queue rather than joining it.** It is the
   single largest identifiable bug class in the reference ecosystem's own
   history: 49 commits in `OxideMod/Oxide.Rust` touch a hook's arguments
   and ~27 correct a payload that had **already shipped wrong**, four of
   them more than once (`OnEntityBuilt`, `OnCollectiblePickup`,
   `OnEntityReskin`, `OnItemStacked`). Their patcher pinned an `MSILHash`
   per patched method — the exact analogue of our byte-golden — and it
   caught none of them, because a hash over the *shape* of a payload is
   blind to the meaning of the fields inside it. `reference/FINDINGS.md`
   §1 has the receipts. This is not a hypothetical wall; it is the wall
   the reference walked into for a decade.
   **Landed already**, `crates/sim-core/tests/event_roles.rs`: five codes
   checked by role against a real cause — `EV_HIT`, `EV_HEALTH`,
   `EV_DEATH`, `EV_BAG_DROPPED`, `EV_GATHER` — plus two disciplines that
   make the file able to fail. `distinct3` refuses a check whose three
   fields are not mutually distinguishable, because a permutation would
   satisfy it otherwise; `only` refuses zero *and* two, which makes it a
   double-emit gate as well (their `Removed duplicate OnBonusItemDrop
   hook` and two rounds of `Fixed double deprecated hook call with
   OnActiveItemChange/d` are the same family). `coverage_is_stated_not_implied`
   pins the ledger at 5/25 so the gate can never read as "the event lane
   is covered" while covering five, and a new `EV_*` cannot land without
   someone classifying it.
   **This item is the remaining twenty.** Priority inside it is by *swap
   silence*, not by code order — an event whose fields are different kinds
   of thing is far harder to get wrong than one carrying two player ids or
   two hp readings. In order: `EV_DRANK` (b = water restored, c = hp cost
   — two small ints), `EV_VITALS` (b and c are both `food<<16|water`
   packs), `EV_STOCK` (a = feeder, b = cell key, c = level),
   `EV_DEPLOY_PLACED` and `EV_DOOR` (both end in a player id after two
   packed fields), `EV_STRUCT_HIT` (c packs damage over hp-left), then the
   refusal codes and the sync/def batches, which are the safest and should
   go last. Move `UNCOVERED` in the same commit that moves `COVERED`.
   **What would be stronger than more tests, if a pass wants the bigger
   swing.** A payload-role table both the emit site and the check read, so
   a swap is a *compile* error rather than a test failure. That is a
   larger change than this item and should not block it — twenty role
   checks are worth having either way, and they are what would prove the
   table correct when it lands.
   **A trap this file already paid for, so the next pass does not.** The
   first cut asserted on the tick it *sent* the swing and read an empty
   ring twice. The sim auto-repeats a held button, so every swing after
   the first resolves inside the cooldown, on a tick the test never sent
   an input for — `until` steps until the code appears rather than
   predicting when. And a wrapper struct holding a `World` by value
   overflows a test thread's stack in an unoptimized build, exactly as
   `combat.rs` warns; the helpers here take `&mut World` and never put a
   second one in the frame.
1. **The scatter is white noise and a forest is not — give the occupant
   draw a continuous fitness field.** *(Operator, 2026-08-04: "should we
   [upgrade the stack]? unless its unity larp to get around unity jank."
   Mostly it is. `reference/SPAWN.md` §9.3/§9.4 is the residue that isn't.)*
   **Scope discipline first, because the research it comes from is large
   and this item is not.** `reference/SPAWN.md` reports four placement
   systems in the reference game. Three of them — a population that is a
   *count* rather than a slot list, a quadtree importance sampler, and
   physics-query occupancy with an attempt budget — are all downstream of
   one Unity constraint: a choppable tree must be a GameObject with a
   collider and a network identity, so it is *already* networked and
   persisted, so placement never had to be a pure function. **None of that
   is portable and none of it is proposed.** Our slot model is the better
   half of that trade and `TERRAIN.md` §0 is the reason the island costs
   zero bytes to join. What survives the filter is one change inside one
   function.
   **The defect.** `terrain::scatter` draws one hash per 8 m cell and
   decides that cell alone, against a per-biome weight row indexed by a
   *discrete* `biome()`. Independent draws are white noise: uniform-density
   speckle with no groves and no clearings, and a hard density step exactly
   where `biome()` changes. `TERRAIN.md` §1 stage 6 sells forest as "wood,
   cover, low visibility" and stage 5's masks are continuous; the scatter is
   the one consumer that throws that continuity away. The reference game
   gets the texture from `ClusterSizeMin..Max` objects drawn out of one
   quadtree leaf, braked by a 2×-density cap over a 20 m cell — a stateful
   sampler we cannot and should not have.
   **The change.** Make the cell's weight continuous and let one extra
   noise channel carry the clumping:
   - `weight = biome_row[occupant] × clump(seed, x, z)` where `clump` is a
     low-frequency value-noise field — the shape `moisture()` already is,
     at a wavelength that makes groves rather than biomes **(knob)**.
   - Accept on a **squared** fitness, the reference's own `factor² ≥ rand`
     rather than `factor ≥ rand`, so a biome edge falls off quadratically
     into a soft tail instead of stepping. §9.4 is right that this is free;
     it is also *only expressible* once the fitness is continuous, which is
     why these are one item and not two.
   - Still one hash draw, still `O(1)`, still pure, still no trig. The
     restricted-float and no-libm walls do not move.
   **What it reddens, and the order to take it in.** This is a worldgen
   change under wall 5, so every fixture it moves is regenerated **in the
   same commit** or it does not merge:
   - `test_terrain_golden` — `GOLDEN_TERRAIN_HASH` moves by construction.
   - `test_terrain_shape_sanity` — the live-slot band (8–12k), and trees >
     1000 / ore > 300 / barrels > 50. **This is the actual work.** A
     mean-1 multiplier roughly preserves the count but not the variance,
     and the slope and water vetoes are nonlinear in it, so the weight rows
     need re-tuning against the band rather than assumed through it.
   - `world::tests::spawn_ring_lands_on_a_clear_beach` — asserts every
     spawn is 4 m clear of every slot. More clumping makes that harder to
     satisfy; if it reddens, that is a real signal about clump amplitude,
     not a test to widen.
   - `test_replay`'s `GOLDEN_FINAL_HASH` — only if that script's gather
     path touches a slot whose occupant changed. Determine empirically;
     do not pre-emptively regenerate a hash that did not move.
   - `ci/parity.mjs` needs nothing: gates.sh **diffs** native against wasm
     rather than pinning either, so both halves move together for free —
     which is exactly what that gate is for.
   - Clippy's sim walls and `test_alloc_zero` are untouched: no allocation,
     no new float op outside the permitted set.
   **The knob, before the code.** `clump` wavelength and amplitude are two
   numbers nobody has spoken. By `CLAUDE.md` they go into `DECISIONS.md`
   §open first and reach `terrain.rs` second, and the knob-registry gate
   will hold them there.
   **Explicitly not in scope**, so a later pass does not smuggle them in
   under this heading: population counts, respawn-elsewhere, any entity per
   tree, any sampler with state, and the operator census verb from
   `SPAWN.md` §9.7 (worth doing, unrelated, its own item when someone wants
   it).
1. **The sun cannot rise until the ground's structure moves from bump into
   albedo — and that is now a measurement, not a hunch.**
   *(What is left of the lighting iteration after `DECISIONS.md` §open
   "lighting v1" landed the rest of it. The register, the transfer, the fill's
   earth half, the sky, the fog and every gate that scores them are done and
   green; the one thing the item asked for that did NOT ship is the sun's
   elevation, and it did not ship because a wall said no.)*
   The arithmetic, from the row: a normal perturbed by δ changes `N·L` on flat
   ground by `cot(elevation)·δ` relative, so the ground's whole bump relief
   scales with cot. With the shipped field byte-identical and only
   `SUN_ELEVATION` moved, `browser_smoke` 15 measures
   | elevation | cot  | frame moved | mean Δluma | brightened, worst yaw |
   |-----------|------|-------------|------------|-----------------------|
   | 0.36 rad  | 2.66 | 11.20%      | ~19        | +0.4%  (floor 0.2%)   |
   | 0.50 rad  | 1.83 |  2.03%      | 7.2–8.4    | +0.01%                |
   | 0.785 rad | 1.00 |  0.47%      | 7.0–7.8    | +0.00%                |
   The last column is the blocker: 15's two-sidedness separates a field from a
   wash, and the pass before this one built a bump fix, measured it and
   declined to ship it rather than spend that margin. Raising the sun spends
   it twenty times over.
   **The exit condition is stated so it can be checked**: when the ground
   holds assertion 15's margins with its bump contribution removed — i.e.
   when its structure is carried by albedo rather than by relief — this
   constant can rise, and the reference frames' midday register comes with
   it. That work is the GROUND's albedo structure — item 7's "re-place the
   meso octave" and the bump-vs-albedo balance beneath it — which now has a
   second, independent reason to be next. Nothing else about the light rig is
   waiting on anything.
   Smaller things the lighting owner measured and did not take:
   - **The sky has no clouds**, so its own tonal span inside one frame is 16–89
     levels where the reference's is a few hundred. The dome is a shader now
     and the seam, the dither and the sun disc are in it; cumulus is a
     separate slice and probably a `threejs-volumetric-clouds` one.
   - **Water still has no wave normals.** The specular agrees with the sun by
     construction (same light) and the horizon no longer steps, but the judge's
     "amorphous Gaussian smear with no specular structure" is about the surface
     it sits on, and a flat plane has none.
   - **The prop field's own amplitude fell 11% (rock) and 28% (pine)** when the
     transfer's toe came off, disclosed in the §open row rather than netted
     against the 48%/67% rise in its delivered floor. The toe was exaggerating
     dark surfaces; the surfaces are now honestly lit and honestly thin, which
     is the materials lane's number to move.
1. **The ground's chroma noise — the artifact the last pass shipped. — LANDED**
   *(GAP PASS, iteration 2. From `findings/pass-20260803-145507-01-visual.md`
   ranked gap 1: "Kill the near-ground chroma confetti — it is a live render
   artifact in four of six frames and it is a sampling bug, not an art task."
   The report's own instruction was that nothing else in its list should be
   attempted while a visible render bug is in half the capture.)*
   **The cause was not the one the report ranked first, and the difference
   matters.** Its three suspects were, in order, the `textureGrad` derivatives
   across a splat discontinuity; `BASE_ANISOTROPY_MAX = 4` at ~80° incidence;
   and the per-identity gain amplifying mip-level chroma noise. It is the
   third, and it is arithmetic rather than sampling: the mean-placing gain is
   `color / measured mean` PER CHANNEL and it multiplies the whole sample, so
   a source dragged unevenly across channels has its per-channel NOISE dragged
   with it. `rock` needs ×13.45 on blue, whose source mean (0.034 linear) sits
   near its own JPEG chroma floor.
   **The instrument is what made this decidable**, and it is the reason not to
   act on a ranked gap's literal sentence (`CLAUDE.md`'s "a judge names the
   symptom; fix the cause"): resolve the near-ground high-frequency residual
   ALONG the local mean colour versus ORTHOGONAL to it. The thirteen
   `Rust Images/` frames that actually contain ground run 0.077–0.193 (median
   0.120); our six judged frames ran 0.659/0.798/0.237/0.284/0.760/0.092 —
   every frame showing ground is over the reference maximum, and the only frame
   with no near ground in it is the only one inside the band. **Our
   along-colour term was inside the reference range the whole time.** So the
   defect was never amplitude, and both of the report's first two suspects are
   amplitude fixes that would have cost the detail 15h asserts.
   `BASE_ANISOTROPY_MAX` is deliberately untouched.
   Shipped: `BASE_CHROMA_STRETCH_MAX = 1.0`, applied per layer as
   `min(1, MAX / span)` off each source's own measured gain span (sand 0.72,
   grass 0.61, litter 0.26, rock 0.17). Mean preservation became a property of
   the tap's shape rather than of its tuning — see `DECISIONS.md` §open. 15h is
   unmoved (5.90/8.61 against 5.91/8.58) because the along-colour term is
   unmoved; only chroma falls. Gated at **15i**, a CEILING, with the unbounded
   leg rendered live every run so the suppression is a number and not a claim
   about a commit.
   **What this did NOT do, and the next pass should not be misled about it.**
   The frame moved 0.434 → 0.317 (level) and 0.313 → 0.243 (down). **That is
   still 1.6× over the reference maximum of 0.193.** The wall is at 0.35, which
   is where the tree is, not where the references are — 15h's own argument for
   splitting a target from a floor, applied to a ceiling. Two reasons it stops
   there, and only one of them is this knob's to fix:
   (a) the two vantages 15i measures sit at a spawn that is 99.2% grass, where
   the bound is weakest (grass keep 0.61); `litter` and `rock`, where it bites
   hardest, are ~absent there. So the gate measures the fix at its weakest,
   which is the right direction for a wall but understates the fix.
   (b) **the luma-only floor — every keep at 0 — is 0.186/0.174**, already
   above the reference median of 0.120. Most of what remains is therefore NOT
   the photograph: it is the tint octave's deliberate off-colour deviation
   (15d asserts it at ×1.43), the sky dither and the fog. Tightening
   `BASE_CHROMA_STRETCH_MAX` below 1.0 cannot reach the references on its own
   and would start discarding measured colour the references demonstrably
   carry. Per `CLAUDE.md`'s coupled-lighting law that remaining set has one
   owner, and it is the lighting pass, not this one.
   **A gate defect this pass found in its own first cut, recorded because it is
   the more useful half of the lesson.** The reference band was first measured
   with a 2×2-box residual while the probe used a 4-neighbour-mean one, giving
   0.336 instead of 0.193 — and 0.336 would have walled our 0.317 in as a pass.
   A ceiling computed by a different estimator than the frame it judges is not
   a ceiling. Both are now the probe's estimator, and the reference set is
   restricted to the thirteen frames that actually contain ground (the four UI
   screenshots and the top-down map render were two of the five highest
   readings in the unrestricted set).
   Also cleared here, from the same pass's merge-gate judge (ranked fix 1):
   `DECISIONS.md` §open, `NOW.md` and 15h's comment block all claimed the
   shipped frame measures 6.00/8.59 luma/px, which was the aniso-16
   configuration that was cut. They now say what `base detail:` prints. Its
   ranked fix 3 (the `grain`/`tint`/`base` toggle checks reading a snapshot
   captured before any probe ran) is NOT fixed — it is inherited convention and
   is left for a pass that owns those three; 15i's own restore check reads live.
1. **The renderer has never had real detail to sample — give it some.**
   *(Slice 1's projection defect is fixed — `DECISIONS.md` §open,
   "materials v4": the base maps were sampled on world XZ and smeared `1/u`
   along every fall line, every octave in the file retired on the horizontal
   footprint rather than the world one, and snow replaced the albedo instead
   of scaling it. Level ground is bit-identical; 15h/15i/15e unmoved. The
   crosshatch that remains is the item above, not this one.)*
   *(Operator, 2026-08-03, `DECISIONS.md`: real assets allowed, CC0 is the bar.
   `ART.md` §7 is the policy; `assets/textures/` is the working set, already
   committed and manifested. This is the wiring.)*
   **This item is a BOUNDED EXCEPTION to the visual ration** (item 5, operator
   2026-08-03). It runs consecutively until its two slices are merged —
   expected two to three passes — and then the ration resumes at one visual
   pass in four with this lane's remainder. The exception is bounded because
   the ration exists to stop an unsatisfiable bar from eating the queue
   forever, and this is the opposite: a defined piece of wiring with a stated
   done condition. The gameplay lane (item 5) is next in line the moment
   slice 2 merges — a pass that finds this item already done takes item 5,
   not another visual item.
   The number this is about: `ART.md` §3's near-ground neighbour contrast is
   **6.3 luma in the references and 0.26 in ours**, and eight visual passes of
   noise octaves have not moved it. A 1K photographed albedo carries that
   detail by construction.
   **Slice 1 — the ground — LANDED** (`DECISIONS.md` §open, "ground base maps
   v0"). Albedo/normal/roughness for all four identities, at each identity's
   own declared tile, under every existing layer rather than instead of them.
   The mean is preserved by construction — each layer's linear mean is measured
   at load and divided out, so the palette keeps the mean and the photograph
   contributes the variance, which is also what pulls the off-band `rock` pick
   into §3's band without editing the file (measured gain span ×5.72, exactly
   as `MANIFEST.md` predicted). Gated at `browser_smoke` **15h**, the first
   assertion in that file whose sharp number is an absolute rather than a
   ratio: **5.90 luma/px at the level vantage and 8.61 near-ground, against
   0.41–0.47 from the octaves alone**, with §3's 6.3 printed beside it every
   run. Three texture units, 3.1 MB of §7's 12 MB, and ≤12 fetches/fragment —
   ≤24 at a wall since materials v5 put the tap on two planes.
   Two things fell out of it, both recorded in the §open row: the octave probes
   (15b/15c/15d) now hold `uBase` at 0 across every leg, because a ratio cannot
   answer "what did this octave add" once something two orders of magnitude
   larger is in the denominator — their floors are untouched and 15b now scores
   ×8.65 against ×2.0. And **15e's ship leg is a wall now**, at the unchanged
   ×1.35: the quad-locked mosaic reads ×1.00 against ×3.12/×6.15. That is
   dilution, not a fix — see item 7's first want, which is unchanged.
   **An open debt on these same probes**, raised as ranked fix 3 by the
   merge-gate judge of `findings/pass-20260803-145507-02-judge.md` and left
   deliberately unfixed there because it is inherited convention across three
   pre-existing checks rather than one pass's slip: the `grain`/`tint`/`base`
   toggle checks read a snapshot captured *before* any probe ran, so they
   assert against a stale baseline. 15i's own restore check reads live and is
   the pattern to copy. Whoever next owns 15b/15c/15d fixes all three.
   **Slice 2 — the props — NEXT, and it is what this item still wants.** Same
   maps through `surfaceMaterial()` for bark, wood, stone, metal, cloth, ore.
   Props have no UVs, so they go through the triplanar path that already
   exists — this is why that work was worth doing. Three fetches per plane per
   map is nine, so the unit budget (3 of 16 today) and the fetch ceiling are
   the first thing to design against, not the last; `propProbe`'s 15f/15g
   floors are the ones to re-measure, the way 15b's were here.
   **Also left, and cheap:** the base tile is 0.59–1.00 m, which is what item 1
   asked for and is fine at the near-ground framings 15h measures — but it is
   a ~1 m repeat, and nothing yet measures whether it READS as one at 10–20 m.
   The visual judge is the right instrument for that; do not pre-tune it.
   Second, the base retires on `FADE_OCTAVE_CPP` (~36–60 m out) and that fade
   is doing double duty as the cost control on this box. If the far ground
   reads flat in a captured frame, the fade is where to look, and the honest
   fix is a cheaper far path rather than a wider fade.
   **Then, and only then, the trees.** Pines are four primitives
   (`terrain.js`, `pineGeometry`). `.claude/skills/threejs-procedural-vegetation`
   covers trunks, recursive branches, leaf cards, species presets and wind —
   a large upgrade with no binary shipped, and the 24 checked-in three.js skill
   packs have gone essentially unused. Read the skill before designing this.
   **This item is now mostly spent** — `DECISIONS.md` §open, "wind + felling
   v0" and "pine geometry v1 (whorls)". The pine is no longer four primitives:
   it is a tapered full-height trunk carrying five ragged, drooping whorls,
   102 triangles against 48, slenderness 1.53 → 2.41, 44 silhouette radii
   against ~18, and 23% of its area facing down so the canopy underside the
   fill and bounce poles were tuned against actually exists. `ci/pine_shape.mjs`
   scores all of it off the shipped builder, which is why `web/src/props.js`
   is a module importing THREE and nothing else. **What that gate also closed:**
   `world.rs` derived `SPAWN_CLEAR_M` from a sentence about a JS constant and
   nothing enforced it — a canopy widened for taste would have put fresh spawns
   back inside trees with every gate green. Read the vegetation skill before
   the NEXT thing here, which is needle cards and alpha (`ART.md` §5 asks for
   them by name and this slice deliberately did not spend a texture, a program
   variant or an `alphaTest` on them).
   **The motion landed first, out of order and on purpose.** Trees sway (one `aWind`
   cantilever weight per vertex, world-position phase, two octaves, technique
   from SeedThree re-expressed for WebGL) and a chopped tree now falls, on a
   bearing hashed from its own cell, leaving a stump that stands for the
   respawn window. Both are client-only: no sim state, no wire byte, no
   `PROTO_VER`. Three things that fell out of it and are worth carrying:
   - **Wind is the client's first animated uniform, and it takes the SIM TICK
     as its clock** (`terrain.update`). That is item 12's determinism paid for
     in advance rather than retrofitted — and `browser_smoke`'s new assertion
     13b checks the arithmetic (`t == tick/30 x speed`), so a later pass that
     reaches for `performance.now()` goes red instead of quietly making every
     future frame golden unrepeatable.
   - **The swaying pools own a wind-bearing depth material.** A displacement
     in the surface material alone leaves the shadow standing still, and that
     is invisible to every pixel assertion taken from the camera's side. If
     leaf cards or a second wind system arrive, they inherit this or they
     inherit the bug.
   - **The fall direction is hashed, not sent.** A tree should fall away from
     the axe, the sim knows where the chopper stood, and `EV_SLOT_HARVESTED`
     has spare `b` bits — but spending them is a `PROTO_VER` bump and
     regenerated goldens under wall 6. That is the next slice of this, and it
     is small.
   Still open, and now the whole of it: the pine's four primitives, and the
   billboard LOD (item 11). SeedThree's `impostor.js` is the reference for the
   second — two crossed cards baked front/side in a worker — and its emit side
   returns a `Group` per tree where this client needs an `InstancedMesh` pool.
2. **A death evicted you from your own base, and nothing you built said
   otherwise.** *(Gap pass. From the merge-gate judge's ranked gap 1 in
   `findings/archive-prestamp/pass-20260803-064506-04-judge.md` — "the one
   mechanic the genre uses to make a base worth building is placed, capped,
   hashed and inert" — ranked there as "higher impact than anything else on
   this list, because it is the mechanic that converts 'I built a base' into
   'I have a base'".)*
   **Landed** — `DECISIONS.md` §open, "respawn on bag v0". `ALPHA.md` §1 had
   already spoken the whole rule ("respawn-on-it with a per-anchor cooldown
   (~5 min **(knob)**)"), so this implements a spoken knob rather than
   inventing one: `BAG_COOLDOWN_TICKS = 9_000` is those five minutes at the
   30 Hz tick. A death now scans the deploy store for the dying player's own
   ready bags and wakes the body on the **nearest to where it fell**,
   spending that bag for its cooldown; killed again inside five minutes you
   walk to your next bag, and with none ready you are back on the ring
   exactly as before — which is what makes `BAG_CAP` a cap on how many
   deaths in a row a defender can answer. No wire moved: the client already
   learns the position from the next snapshot, and the subtype a respawn
   would spend belongs to the death screen below. Armed in
   `test_alloc_zero` and in a fourth parity probe whose printed **count** of
   bag wakes `ci/gates.sh` fails on at zero; structural in `test_replay`,
   whose script cannot kill anything.
   **Landed this pass** (`DECISIONS.md` §open, "the death screen + the
   choice · wire v16"): the flow `ALPHA.md` §1 actually specifies, and the
   half v2 could not express.
   - **A death is a body lying where it fell.** `World::die` drops the
     backpack and sets `Player::dead`; `World::wake` is a separate half only
     `Command::Respawn` reaches. **No timer releases it** — a span nobody
     spoke would be a knob invented into code, and the one thing the state
     exists for is that the player decides. A corpse keeps its id, deaths,
     position and facing and nothing else, every verb resolves through a new
     `live_slot_of`, and it is stepped by `movement` with a **zeroed** frame
     rather than skipped so the client's predictor agrees about a body it
     can still see.
   - **The choice is real, and refusing a bag does not spend it.** The beach
     button leaves the cooldown untouched, so walking away from a fight you
     have already lost costs nothing but the walk — `a_refused_bag_is_not_a_
     spent_bag` is that assertion. Asking for a bag you have not got is a
     beach, never a refusal: a player stuck behind a screen their button
     cannot dismiss has left the game.
   - **Wire v16, every part inside a field an earlier version widened.**
     `ACT_RESPAWN` is the 12th action of 16 (v12's bits) carrying one bit and
     **no bag id** — a forgeable id would let a client wake on someone else's
     bag. `SUB_RESPAWN` is the 36th event subtype of 64 (v13's bits) carrying
     the same bit *back*, because a bag inside its cooldown gets you a beach
     and nothing else would tell you. The one layout that moved is `Death`,
     which gained cause, weapon and range — and **still carries no position**,
     which is ALPHA §1's stated rule, not an omission. All three read off the
     victim's own record at encode (the corpse is still in its slot), 56
     goldens regenerated in the same commit plus two new.
   - **The gates are counted.** `test_replay`'s golden moved
     **structurally** — ten bytes per live body, and nothing on that surface
     can die. `test_alloc_zero` answers four screens a tick inside the window
     and walls `screen_ticks > 0` / `corpse_acted == 0`; `probe_bags` presses
     both answers on every bot every tick, which makes `ci/gates.sh`'s
     existing `wakes > 0` strictly stronger (a wake is now only reachable
     *through* the screen); `client_smoke` hand-frames our own death, a
     stranger's, and a forged fourth cause.
   - **No gate kills a body in a browser**, and the reason is content: melee
     wants a weapon neither smoke tab can gather, and the sea refuses a drink
     into a full meter, so salt suicide runs at the speed thirst drains.
     `browser_smoke` 17 asserts the half a browser can see. The honest way to
     close it is a `__gatesDebug` kill affordance on a **dev** shard only, or
     a smoke tab that gathers a rock first — both are their own slice.
   **What this item still wants**, in the order it is worth doing:
   - **A dropped `EV_RESPAWN` leaves a live body behind the overlay — the
     documented reconciliation does not exist.** The merge-gate judge failed
     this branch on it (check 9, doc/code truth,
     `findings/archive-prestamp/pass-20260803-121954-02-judge.md`), and it
     merged anyway on the operator's call 2026-08-04, so the defect is carried
     here rather than erased by the merge. `crates/server/src/core.rs:711–719`
     documents a client-side reconciliation that is not implemented, for a
     reachable failure: `EV_RESPAWN` is droppable at `MAX_EVENTS_PER_TICK` and
     is the only thing that can close the screen, so losing it strands a live
     player behind an overlay with its inputs zeroed. The fix is to implement
     the clause, not delete it — clear `ClientCore::dead` on an own-body
     snapshot that cannot be reconciled with a corpse, gated in
     `client_smoke.mjs`.
   - **The choice is beach-or-nearest, not a bag picker.** ALPHA §1 says
     "choose beach or a bag" and that is what shipped; what it is not is
     `inventory.jpeg`'s map of anchors to click. A picker needs the client
     to know which of *its* bags are ready, which is per-bag cooldown state
     the deploy sync deliberately does not carry (`DeployRec` has no room
     for it, by design) — so it is a wire slice, and it wants the map below
     more than it wants itself.
   - **A reconnect is still a ring spawn.** Only *death* consults a bag;
     `Command::Join` does not. That is the sleeper/haven lane (`NETCODE.md`
     §6.3, "haven sleeper timeout 20 min"), and a player who logs out in
     their own base should not have to die to get back into it.
   - **You still cannot navigate.** The same judge gap names the other
     half: no map, no compass strip, no markers — so a body that *does*
     fall back to the ring has nothing to walk home by, and
     `mapstylized.jpg` and `gameplayfoundbase.jpeg` are both in the
     reference set precisely for this. The compass strip is the cheap half
     and it is also the visual judge's HUD ask.
   - **A bed halves the cooldown** (ALPHA §1) — content, not code, once a
     second bag-class deployable exists.
   - **The kill feed still says less than the wire now carries.** `Death`
     crosses with cause, weapon and range as of v16 and the feed line is
     still `#N killed you`; the death screen reads all three and the feed
     reads two. One line in `main.js`, and it wants the nametags below more
     than it wants doing alone.
3. **The world was lit upside down, and there was no air in it.**
   *(Gap pass. From the visual judge's ranked gap 3 in
   `findings/pass-20260803-064506-01-visual.md` — "the daylight register is
   inverted and there is no atmosphere — one owner, one pass" — which also
   turns out to be the mechanism under its ranked gap 2 ("half of every
   object's screen area is a black identity-free silhouette") and under the
   **prop surfaces v0** row's own hand-off in `DECISIONS.md`, which wrote the
   arithmetic out and said the fix was this coupled edit and nothing a
   material can do.)*
   **Landed** — `DECISIONS.md` §open, "the daylight register". Sky and air
   taken together by one owner, because `CLAUDE.md`'s trap list says splitting
   them is how three passes get lost. The dome is a fragment program with a
   haze band, a sun disc and a dither instead of a 24×16 vertex ramp, and the
   fog near plane is inside the near ring it was 20 m outside of.
   `browser_smoke` assertion 16 gates it as counted differences of frames: sky
   ×1.79–2.28 over median ground (floor ×1.15), the haze lightening 100.0% of
   what it touches, the far third reading ×1.162 luma / ×0.713 saturation
   against the near third, and each band's own luma lift and saturation drop
   climbing on every step.
   **Its constants were superseded on 2026-08-04 by lighting v1** (the row
   below it in §open, merged from `loop/lighting-midday`), which re-metered the
   same coupled set on a branch that never saw this one. The gate above still
   runs and still passes; the numbers it runs against are v1's.
   **One finding of this item outlived its numbers and is now owed work.** The
   register handed the fog its colour **pre-transfer**, so the horizon seam was
   exact for the first time: three uploads `fog.color` in the renderer's output
   colour space and mixes it in after `tonemapping_fragment` and
   `colorspace_fragment` (r178 `WebGLRenderer` `getUnlitUniformColorSpace`), so
   one hex reaches the image as two values — the dome's tone-mapped and the
   fog's not. v1 instead shares one `THREE.Color` between the two and asserts
   that identity (assertion 17a), which pins the two INPUTS and leaves the two
   OUTPUTS a transfer apart: at the shipped horizon the dome's peak channel is
   past `StartCompression`, so the sky lands a few percent under the haze that
   is supposed to converge on it. Cheap to fix — put the fog's copy through the
   transfer the way the register did, with v1's toe removed — but it moves the
   register, which makes it lighting's owner's change and not a merge's.
   **What this item still wants**, in the order it is worth doing:
   - **The ambient floor — and it is BLOCKED, by a wall, not by difficulty.**
     This is the judge's third counted ask (no unlit face below 0.30 of its
     lit face) and the pass built it six ways and measured every one red. The
     prop gate's chroma ratio ships at ×1.12/×1.13 against a ×1.10 floor;
     every unit of ambient lands in that ratio's *denominator*, because light
     on a face that was rendering as near-black noise is chromaticity the
     material did not put there. The sky pole fails through the boulder and
     the bounce pole fails through the pine, so there is no split of the
     budget that raises the floor and leaves the ratio alone — and walking the
     key toward the fill is worse, costing 38% of the numerator, because a
     coloured light is what makes a mineral read. **The unblock is item 5
     below** ("nothing that is not the ground has a surface"): raise a prop's
     authored chroma and the same ambient clears ×1.10, and this becomes a
     two-line change. The six measurements are in `DECISIONS.md` so that pass
     starts from a bounded problem. Until then `DAYLIGHT_MIN_AMBIENT_FLOOR`
     sits at 0.15 as a regression wall on the 20.8–41.2% the rig delivers, and
     the metric itself wants widening: it reads non-sky pixels, which are
     mostly up-facing ground, so the one case it can never see is the one the
     judge measured — a canopy's underside at (2, 6, 0) needs an object-face
     probe.
   - **The sun's elevation, the other piece taken OUT of this pass rather
     than shipped.** The report asks for a near-midday register; the shadow
     gate's floors (15% of the sweep, 10% every yaw) were measured against
     terrain self-shadowing under a 21° sun, and raising it to 45° removes
     most of the 24.0% those floors were set against. A floor a change would
     breach means the change is not done — so this is a pass of its own:
     raise `SUN_ELEVATION`, re-derive the shadow floor **under the new sun**
     with the mutation controls the current floors carry, or add the shading
     term that keeps hillside relief readable when the sun is high.
   - **Contact grounding** (visual ranked fix 4). A contact-AO or dirt-skirt
     term wherever a trunk, boulder or deployable meets terrain — the judge's
     vertical scan into the boulder's base reads 93.7 → 95.1, flat to the
     contact point, so everything is a decal on the surface. Cheap next to the
     material work and it is the other half of "nothing is grounded".
   - **A deeper haze than 1400 m, if it can be paid for.** `FOG_FAR` is long
     on purpose: fog only ever removes contrast at distance, and the
     far-shadow and horizon gates measure shadow at 200–500 m. They cleared
     unmoved at 0.72% and 0.48% against 0.25% and 0.15%, so there is headroom
     to spend — but it is theirs, and spending it means re-measuring them, not
     assuming.
   - **The near-ground chroma residual this item inherited, and it is not
     ranked against the four above.** Handed over by the chroma pass
     (`BASE_CHROMA_STRETCH_MAX`, merged `2aa1d41`), which moved the orthogonal
     residual 0.434 → 0.317 level and 0.313 → 0.243 down and then stopped on
     purpose: **the luma-only floor — every keep at 0 — is 0.186/0.174**,
     already above the reference median of 0.120, against a reference maximum
     of 0.193. So most of what remains is not the photograph and not that
     knob's to spend — it is the tint octave's deliberate off-colour deviation
     (15d asserts ×1.43), the sky dither and the fog, which `CLAUDE.md`'s
     coupled-lighting law puts under **this** owner and no other. Tightening
     `BASE_CHROMA_STRETCH_MAX` further would start discarding colour the
     references demonstrably carry. The wall sits at **15i**, a ceiling of
     0.35 — where the tree is, not where the references are — and it measures
     at a spawn that is 99.2% grass, the layer where the bound is weakest, so
     it understates the fix in the safe direction.
4. **There was no clock and no pressure, so the loop had no engine.**
   *(Gap pass. From the merge-gate judge's
   ranked gap 1 in `findings/archive-prestamp/pass-20260802-163821-05-judge.md`
   — "you can log in, stand still for an hour, and be in precisely the state
   you started in"; `consumables.toml` authored five rows no sim code read.
   Ranked first across both gap lists, and squarely inside the operator's
   2026-08-03 gameplay lane below.)*
   Food and water fall on the sim's clock, an empty meter costs hp per
   minute, both empty stack, and eating puts them back — `DECISIONS.md`
   §open, "survival clock v0 + wire v14". Wire v14 spent the tenth action
   subtype and the 32nd–34th event subtypes, so **no field widened and no
   message moved by a bit.** Three gates arm the clock, each in its own
   fixture: `test_alloc_zero` (100 bodies, drained to empty, one starved
   and granted again, the eat verb landed and refused), `test_replay` (64
   bodies, two eating, hash pinned) and `test_parity_wasm` via
   `probe_combat` (native and wasm byte-identical).
   **Landed the pass after** (`DECISIONS.md` §open, "food you can get + the
   clock's death"): the clock has an answer, and a content set without one
   will not boot.
   - **A death by the clock is a death.** One line at the death site, where
     `combat::strike` already counts its own, so `spawn_pos_n(id, deaths)`
     walks the ring forward and a starved body stops waking up on the beach
     it starved on. `test_alloc_zero`'s staged starve asserts the count
     moved; new `crates/sim-core/tests/survival.rs` owns the consequence
     (`World::respawn` and the ring), which the module itself cannot reach.
     `test_replay`'s golden did **not** move, and that is a fact about the
     script — its fixture widens both spans past the 900 ticks it runs
     precisely so no body starves inside it.
   - **A node may pay two things.** `NodeDef.secondary`, one flat
     `(item, units)` pair from `[gatherable.secondary]`: the bush pays 5
     berries beside its 10 cloth, on its own `EV_GATHER` so the toast stack
     reads both. Flat by design — no tool row, no weak-spot bonus, because
     picking is not chopping. 45 minutes of hunger and 10 of thirst per
     bush against the shipped meters.
   - **A clock must have an answer**, and that is a wall now:
     `validate::structural` refuses content where a meter drains and no
     gatherable pays a consumable restoring it, and `test_content` prices
     the answer in the clock's own units (≥ 20 min of the hunger span,
     ≥ 5 min of the thirst span, per pickup) so one berry cannot satisfy a
     boolean. Loot deliberately does not count while no verb opens a
     container. The value reaches `canon.rs` too — the defect
     `[backpack]`'s ladder carried, caught here before it shipped.
   **Landed this pass** (`DECISIONS.md` §open, "the drink verb + wire v15"):
   thirst's real answer, and the ocean stopped being scenery.
   - **`ACT_DRINK`, the eleventh action of sixteen, and `Drank`, the 35th
     event subtype of sixty-four** — so no field widened and no message
     moved by a bit. The fifty-four existing goldens are byte-identical
     after regeneration; only `v15_hello.bin` differs, because `PROTO_VER`
     is inside it. `Drank` is its own subtype rather than a `Consumed` with
     an empty item because the drink *costs* hp and `Health` is absolute:
     a client that only heard the number could not name what took it.
   - **The sea is salt** — 25 water for 2 hp, `content/balance.toml`
     `[survival]`, derived against the answers already shipping rather than
     picked. 25 is one bush's worth of thirst in one press with no walk;
     2 hp is priced against the repair on the same shelf (a bandage is
     20 cloth, a bush pays 10, so two bushes buy back ten mouthfuls
     against the ten bushes it would take to drink the same water).
   - **The first verb here that reads the world, not the inventory**: five
     `terrain::height` taps at the feet and the four cardinal points of
     `build::BUILD_REACH_M` — reused, not given a reach of its own. Five
     and not a ring because trig is banned in the sim. Payload-free on the
     wire for a stronger reason than `Loot`'s: the heightfield is a pure
     function of the seed, so there is no position to forge.
   - **It can kill you**, through the module's one kill site — factored out
     of the starve path this commit, so the two ways the world can kill
     cannot disagree about what a death is.
   - **The `validate` wall widened in the same commit**, and `test_content`
     pins the widening from both sides: an armed drink alone answers
     thirst, and disarming it as well is refused. Three gates arm the verb,
     each in its own fixture — `test_alloc_zero` (a scanned shoreline, the
     salt death, the dry refusal, zero alloc delta), `test_replay` (hash
     re-pinned as a function of the verb's arithmetic) and
     `test_parity_wasm` via `probe_combat`, which presses it on every bot
     every tick because the answer is a float compare.
   **What this item still wants**, in the order it is worth doing:
   - **The status chips.** `spawnedrock.jpg` carries red `WET 36%` /
     `STARVING 2` above the vitals; an empty meter here only turns its own
     number red. The chip row is where a starving player is told *why*
     their hp is falling. (Also the visual judge's ranked gap 3, which
     asks for "a chip lane the survival clock can actually speak through" —
     so this one is claimable from either list.)
   - **Mushrooms and corn are still unreachable**, deliberately: they want
     a forest-floor pickup and a farming lane respectively, and inventing
     either to satisfy the new wall would have been inventing content.
   - **Day/night**, `DESIGN.md` §2's other half of the pair, still blocked
     behind the ground's structure moving from bump into albedo (item 5).
5. **Gameplay, and the ration that keeps it first.** *(Operator, 2026-08-03:
   "its for sure getting hung up on lighting of shadows we need gameplay and
   stuff… let it go and code for a long time." The visual judge is an
   absolute bar that cannot be satisfied, so its ranked gaps out-shout the
   gameplay judge's forever if the queue lets them — six consecutive visual
   passes proved it. This item is the counterweight.)* Work these lanes in
   order, top-down, one slice per pass as ever:
   - **The raid loop's missing verbs** — the repair verb + the hammer that
     swings it, then the satchel throwable (item 6 below carries the full
     shape and the content rows).
   - **Barrels and shore loot** — the merge-gate judge's own pick
     (pass-20260802-163821-05-judge.md round 3, gap 2: "the cheapest gap on
     this list to close — four of its five parts already built and green."
     One `open` verb on `BarrelSlot`, one roll against `loot.barrel`, one
     respawn timer).
   - **The remaining M1 survival verbs** (item 11's cut), smallest first.
   - **Join-time instrumentation** (item 8), then the **100-bot soak**:
     NETCODE §9's budgets have never met 100 real connections. Run
     `cargo run -p server --bin bots -- 100` against a dev shard on this
     box, hold it an hour, and record tick jitter, WAL append rate, and
     per-client bandwidth against the budget table — counts and bytes, no
     wall-clock assertions (CLAUDE.md's clock rule). The numbers land in a
     `DECISIONS.md` §open row as the measured baseline.
   - **Capture determinism** (item 10) — now including fixed-length FRAME
     SEQUENCES beside the stills (a walk, a swing, a door opening, water),
     engine-clock-driven; when clips exist, the visual panel gains a
     motion lens.
   **The visual ration:** at most ONE pass in four takes a visual item, and
   only from a judge's ranked gap — **suspended while item 1 (the CC0 texture
   wiring) runs, by the operator's 2026-08-03 call, and resuming the moment
   its slice 2 merges.** The lighting branch
   `loop/lighting-midday` is **PARKED at `0e00a90`** — judged FAIL four
   rounds (findings/pass-20260802-163821-05-judge.md; the code, constants
   and gates verified green in all four, every FAIL was prose truth) — and
   is the first candidate for a ration slot: resolve round 4's check-9/10
   objections, re-judge, merge. Its sun-elevation unlock condition stands.
   The visual items below (3, 4, 8) are rationed with it.
6. **Nothing that is not the ground has a surface.** *(Gap pass. From the
   visual judge's ranked gap 1 in
   `findings/pass-20260802-163821-02-visual.md` — "rock, wood and canopy are
   each one flat colour per facet, literally the rubric's own disqualifier",
   and "no amount of further terrain work reaches criterion 2 without this".
   Its gap 2 — the four artifact classes — is the terrain's, and is blocked on
   the coarse-octave slice item 6 already names, so this pass took the half
   that is not.)*
   **Landed this pass** (`DECISIONS.md` §open, "prop surfaces v0"): the field
   the ground has, extended to everything else.
   - **A triplanar two-octave field on every `surfaceMaterial`** — boulder,
     trunk, canopy, wall, door, ore, body. Triplanar because a prop has no UVs
     and is not a heightfield; the same three-tap normal blend and the same
     `/length(w)` deviation restoration `gmGrainTri` already uses.
   - **The gradient is analytic, so this bump cannot be the ground's dither.**
     Value noise with a quintic fade has an exact derivative out of the four
     corner hashes the value already costs. There is no screen derivative
     anywhere in the patch, so nothing in it can be constant across a 2×2 quad
     — the defect the pass before this one measured and could not fix on the
     terrain (`§open`, "the quad-constant gradient").
   - **Structure is what separates a rock from a log**, not amplitude: a ridge
     fold (`1 − |2n−1|`) turns a blob field into a crack network, a crevice
     term darkens the fold's low side so a crack reads as depth, and `scale` is
     a per-axis vec3 so wood's fissures run UP the trunk. Seven classes, seven
     distinct structures, asserted.
   - **The octave frequencies are set by the OBJECT, not by the ground.** The
     first cut used the ground's frequencies and measured 0.00% of the pine
     frame moved at 10 m: at 5.5 /m the canopy's field retires at 7.7 m. The
     coarse octave is now about a third of the object it sits on (canopy 1.0 m,
     boulder 0.8 m, bark 0.5 m across the grain), which retires at 21–42 m —
     the band the report is about.
   - **The pine's silhouette is ragged**, per-vertex and deterministic, pulling
     canopy rings IN only (a canopy that could grow would invalidate the spoken
     4 m beach-spawn clearance from the renderer). 40 → 48 triangles a pine;
     the measured frame peak did not move.
   - **The gate**: `browser_smoke` 15f, structural half plus a two-view probe
     aimed at instances terrain finds. Its sharp assertion is the field's own
     difference image — neighbour variation as a share of magnitude — because
     a wash scores **exactly** 0 there and the ship-vs-flat ratio it replaced
     is bounded by whatever facet detail the mesh already had.
   **Landed the pass after** (`DECISIONS.md` §open, "prop albedo v1"): the
   value the surface is delivered into — and the gate that has a unit in it.
   - **Every prop assertion was a ratio, and a ratio is scale-free.**
     `contrastRatio` is `(baseline+added)/baseline`, `diffStructure` is step
     over magnitude, `chromaRatio` is spread over spread — so a field swinging
     ±0.8 of a level on a surface delivering luma 6 scores *exactly* what the
     same field swinging ±17 levels on a surface delivering 120 scores. That is
     how v0 shipped green through all three while the visual judge, measuring
     the merged frames, found "a solid" (residual 1.23/255 over 7,800 px) and
     named the amplitude rather than the absence. `propProbe` now returns the
     delivered value as a p05/p50/p95 histogram beside `diffMean`, the field's
     own amplitude, both in 8-bit luma, and 15g walls the median and the
     amplitude at 24 and 2.2. Shipped 48/59 and 4.86/8.47 when that was
     written; **under lighting v1 it is 38/95 and 3.51/7.50** — the delivered
     value up, the field's own amplitude down, because the transfer's toe was
     exaggerating dark surfaces and no longer is.
   - **`ALBEDO_LUMA_BAND = [0.05, 0.55]`**, the linear luminance every authored
     dielectric albedo sits in, asserted over all seven archetypes at both ends
     of every ramp, derived through the renderer's own sRGB conversion rather
     than restated beside the hex. Two of nine bands were under the floor —
     pine trunk ×1.887, pine skirt ×1.106 — and were rescaled in linear, both
     ends together, so hue and ramp shape are exact.
   - **The shaded half is measured, and it is the light rig's, not albedo's.**
     A down-facing prop face receives only `FILL_GROUND × FILL_INTENSITY`, so
     it lands at `groundColor × 1.15 × albedo ÷ π × EXPOSURE` — for the pine
     skirt, **RGB (2,6,1) against the visual judge's measured (2,6,0)** on
     `03-canopy-up`, reproduced from the constants alone. At 3× the authored
     albedo it is still (5,17,3). **The lighting owner took it** (§open,
     "lighting v1"): the hemisphere's earth half is 2.4× and the transfer's
     quadratic toe is gone, the measured p05 went 12 → 20 (pine) and 29 → 43
     (rock), and `PROP_MIN_P05 = 16` is now written.
   **What this item still wants:**
   - **Bark and canopy are still one mesh each.** The field gives them a
     surface; it does not give the pine needle cards, a second species, or the
     trunk/bough separation the report asks for, and there is no undergrowth,
     no bushes and no grass instances anywhere on the ground.
   - **The rock and bark rebuild.** Granite's value range is now walled but not
     authored: rock carries the ground identity's HUE, the field's crevice
     darkening and a band-checked albedo, and still no albedo *structure* — no
     two-mineral granite, no vertical bark ridge. The dirt-ring base is still a
     pedestal rather than blended flush.
   - **Five of seven classes have no rendered coverage.** 15g's structural half
     scores all seven; its pixel half photographs `rock` and `foliage`, the two
     the probe can reliably find near the pinned spawn. `wood`, `stone`,
     `metal`, `ore` and `cloth` are asserted structurally and never seen.
   - **A prop-program budget.** The ground's fragment program is walled at
     96,000 chars and 8 noise sites; the prop program is now the second-biggest
     shader in the client and has neither. `propFacts().noiseSamples` publishes
     6; the wall is not written.
7. **The world reads untextured and shows its mesh — and both halves turned
   out to be arithmetic, not missing art.** *(Gap pass. From the visual
   judge's ranked gap 1 in
   `findings/pass-20260802-050932-01-visual.md`, which returned FAIL on all
   ten criteria with a blind reader identifying 0/6 of our frames as real.)*
   **Landed this pass** (`DECISIONS.md` §open, "materials v2"): the two
   defects the frames actually carried, both measured off
   `04-ground-down.png` before anything was changed.
   - The three structural octaves retired *past* Nyquist (meso 0.74, micro
     0.65 cycles per pixel), so each was still being sampled after it
     stopped being representable. Every octave now retires on the one law
     the grain octave was already written against — cycles per pixel, with
     the metres derived — and `browser_smoke` 15a2 asserts it over the
     whole table.
   - The bump reconstructed its gradient on the triangle, so a
     smooth-shaded heightfield rendered its own facets. It is solved in
     world XZ now, mesh-independent by construction, and gated as
     arithmetic in `ci/bump_basis.mjs` rather than as a screenshot.
   - **The knob registry drifted, and the drift is now gated.** The first
     cut of this work proposed `BUMP_MAX_SLOPE = 1.0`; measuring it (a 45°
     perturbation against a 21° key light) sent 0.55 to the shader while
     the `DECISIONS.md` §open row kept saying 1.0 — nine gates green over
     the disagreement, and the merge-gate judge caught it by reading
     (`findings/pass-20260802-050932-02-judge.md`, checks 4 and 9). The row
     now records the shipped value and its derivation, the same stale
     derivation is corrected in the shader comment that also carried it,
     and `ci/knob_registry.mjs` pins every §open knob declaration to the
     constant that actually ships — Rust and JS alike, unresolved and
     ambiguous names failing as loudly as a mismatch.
   **Landed the pass after** (`DECISIONS.md` §open, "materials v3"): the
   ground has a HUE that varies, and a gate that can see one.
   - **Per-class chromatic albedo, tiled at 0.5–1 m.** One noise sample at a
     per-identity tile scale (sand 0.59 m, rock 0.71 m, grass 0.91 m, litter
     1.00 m) driving a signed chromatic deviation per identity, added rather
     than lerped to, so each identity's authored colour stays its exact mean
     and what changed is the variance. Cost 6 → 7 of the 8 budgeted noise
     sample sites, and the budget was not widened to fit.
   - **The deviations are luminance-neutral, and that is a law rather than a
     taste.** Three scalar octaves and a per-identity grain already moved
     VALUE at four scales; nothing moved HUE, which is the defect stated as
     arithmetic (`k·(r,g,b)` has the chromaticity of `(r,g,b)`). Two earlier
     cuts swung both and each one spent assertion 15's directional margin,
     which at this spawn's yaw 0 is 0.5% against a 0.2% floor before anything
     is added.
   - **The report's "macro-variation octave to break tiling" was built twice
     and deleted twice**, off macro and then off meso, because an octave
     wider than the frame is a constant inside it and both read as a colour
     cast. The premise came with the words: value noise on world XZ does not
     repeat, so there was no tiling to break.
   - **A second measurement track**, because the luma probe every gate in
     this file used is structurally blind to a hue-only octave. 15d masks on
     chromaticity and asserts spread up, centre still, mean luma still, and
     warm and cool both present, at two views.
   **Landed this pass** (`DECISIONS.md` §open, "the quad-constant gradient"):
   the dither is measured, bisected and gated as arithmetic — and the fix is
   blocked on the coarse-octave slice below, which is now the top want for a
   second, independent reason.
   - The newest visual report's ranked gap 1 ("either a flat colour wash or a
     per-pixel dither") is **quad-locked**: measured on its own
     `05-held-level.png`, 1.9 luma/px of neighbour contrast inside each 2×2
     quad against 21.4 across quad boundaries. Only a screen derivative can do
     that — `dFdx`/`dFdy` are differences across the quad, so anything built
     from one is constant inside it. The splat wobble the report's ranked fix 1
     blamed reaches albedo per fragment and cannot produce that signal at all.
   - `scene.aliasProbe` (new, four states off existing uniforms) bisects it:
     zeroing gmH takes the ratio 6.15 → 1.01, and zeroing **grain's bump
     alone** does the same. Grain is the only octave whose fade band falls in
     the near field — 33 → 11 px per cycle is 1–4 m from the eye for a 12 cm
     tuft, where meso's equivalent is 165 m out and micro's 30 m.
   - The fix is a second sampling law: a reconstructed gradient is quad-constant,
     so an octave must retire as a BUMP before it retires as a colour. It was
     built and measured — ×1.01 at both vantages with within-quad detail
     unchanged — and **it is not in this commit**, because it reddens assertion
     15: at yaws 0 and 4.71 the surface probe finds 21% and 24% of the frame
     moved and *not one pixel brighter*, since the only thing the field
     brightened there was the mosaic. Three unblocks were built and measured
     and none is enough; the §open row has all six numbers.
   - **A textbook cause was tried, measured and removed**: `vGmPos.xz` is a
     world coordinate in the high hundreds, so a float32 varying reaches the
     fragment quantized to ~1.2e-4 m against a ~2e-3 m pixel — a 6% staircase on
     `dFdx`. Camera-relative coordinates for the Jacobian moved the ratio
     6.16 → 6.15. Not the cause here; the record is in the §open row.
   - **Still open and arithmetic**: the file states bump slope as
     `amp × bump / wavelength`, and a sinusoid's peak slope is 2π times that.
     Every per-octave slope in the comments and in the materials v2 §open row is
     6.3× understated, so `BUMP_MAX_SLOPE` at 0.55 is not the sum it is
     documented as — it is a bound the octaves exceed, and it clips. Re-deriving
     the amplitudes in the right convention is its own slice: it changes how
     much relief the ground has, so it wants the visual judge on it.
   **What this item still wants**, in the order the report ranked it — and the
   first one now blocks the bump law as well as the tint:
   - **Re-place the meso octave — tried, backed out, and now with a second
     reason to want it.** At 9.5 m the coarsest surviving octave completes a
     third of a cycle inside a typical 8 m ground framing; 4 m completes two
     and still retires far past any footprint this world produces. It went
     red on `browser_smoke` 15c, and not on the arithmetic: the splat wobble
     is driven by gmMeso, so moving it moves which identity owns a face, and
     grain reads its scale, contrast and ridge off those same weights. The
     second reason arrived with the tint: **the macro octave's own ±0.16
     albedo multiply is a cast, not a variation**, at every framing narrower
     than 48 m — measured, it is most of why the field darkens 95% of what it
     touches at yaw 0 and why assertion 15's two-sidedness has only 2.5× of
     margin there. A coarse octave that varies inside a frame would fix both.
     Do it as its own slice, with 15c's 46.6° face re-measured alongside it,
     because the coupling is the reason it is not a one-constant change.
     **This pass added the second reason and measured the third.** The bump's
     sampling law cannot land until this does: with the mosaic gone, yaws 0 and
     4.71 of `surfaceProbe` brighten *nothing*, because the field there is a
     macro cast and the artefact. Fading macro on the DUAL of the sampling law
     — cycles per FRAME, so an octave too coarse to vary inside the frame stops
     being a cast — was built and measured at +0.17% on yaw 4.71 against a 0.2%
     floor (from +0.04%) and +0.01% on yaw 0, applied to the albedo multiply
     and to the splat wobble alike. That is the shape of the fix and not enough
     of it; a coarse octave that genuinely varies inside an 8–25 m frame is.
   - **Splat transitions by height/slope/noise, and a wet-sand waterline.**
     `WET_RANGE` exists and paints; the report saw no shoreline in any
     vantage, so either the band is too narrow to read at capture framing
     or no vantage looks at one. Measure before tuning.
   - **The rock and bark rebuild** (report's ranked fix 8): granite albedo
     with granular grain and crevice darkening, the dirt-ring base blended
     flush instead of reading as a pedestal, vertical bark ridges. Note that
     the ground's rock identity now carries granite's HUE range (buff
     feldspar ↔ blue-grey biotite) and deliberately not its value range —
     that half was left for this slice's crevice darkening.
   - **Bark and canopy are still outside all of this.** Materials v0–v3 are
     the ground's splat material; the scatter pools are `surfaceMaterial()`
     bundles with baked vertex colours and a per-instance tint. The report
     asked for bark and canopy albedo in the same breath as grass and sand,
     and nothing in the tint octave reaches them.
   The lighting gap (`-visual.md` ranked gap 3) that this item used to defer
   to is **done** — `DECISIONS.md` §open, "lighting v1", one owner, one
   iteration, per `CLAUDE.md`'s coupled-lighting law. It hands this item two
   things: the register everything here will now be judged under, and item 1's
   measured finding that the ground's relief is what is holding the sun down.
   Re-measure before re-tuning: every number in this item was taken under the
   old transfer, and the toe that transfer had was inflating dark-end contrast
   by ~1.5x.
8. **A base can be broken into now, but it cannot be repaired, and a
   raid still ends in a shrug.**
   *(From the merge-gate judge's ranked gap 1 in
   `findings/archive-prestamp/pass-20260802-035930-01-judge.md`, and its
   two predecessors'.)*
   Melee v0, the death backpack, and piece damage all landed
   (`sim-core/combat.rs`, wire v11 → v13): a kill drops what you carried,
   and a swing that finds no node and no player breaks the wall, the
   doorway or the door in front of it. `content/weapons.toml`'s
   `structure` column says how much, `content/balance.toml`'s breach
   bands hold the door as the way in, and the three `DECISIONS.md` §open
   rows hold every bound and every deliberate omission.
   What the lane still wants, in the order it is worth doing:
   - **A repair verb, and the hammer that swings it.** Damage is now
     one-way: a chipped wall stays chipped until it decays away or falls.
     Every base in the genre is a repair loop, and without it the first
     raid a base survives still ends it. Content shape: a repair rate
     per material against the piece's own build cost, banded in
     `balance.toml` next to the breach bands.
   - **Throwables — the satchel has a price and no verb.** The raid
     ratio is computed from a weapon nobody can use; melee is the only
     armed raid tool, which is why the wall floor has to be so high.
     A throw needs an arc the sim can integrate and the client can
     predict, so it is M2's ballistic work, not a slice of its own.
   - **A container UI, and per-slot looting.** The take is all-that-fits
     today, which is honest but blunt: nobody can see what is in a bag
     before opening it, or leave the stone and take the gunpowder. The
     inventory screen in `inventory.jpeg` is the shape; a bag panel beside
     it is the slice. `EventMsg::BagRemoved` already carries *why* a bag
     went, so "someone got there first" has a feed line waiting for it.
   - **Armor and headshots.** `armor.toml` bakes into nothing; aim is
     planar, so there is no head. Both wait on M2's rewound raycasts.
   - **Ground drops for a full inventory.** `gather::inv_add` still loses
     the overflow — now that a ground container exists, that loss has
     somewhere honest to go.
   The wire counters, as of wire v15: the event subtype field is **35 of
   64** used (v13 widened it 5 → 6 bits, which is why there is room), and
   the action subtype field is **11 of 16**. The next C→S verb — a repair,
   a throw, a container open — is an action subtype, and there are five.
9. **`gmHash4` — four lattice corners in one `vec4` body, never gated.**
   The projection half of this item landed (materials v1 third pass,
   `DECISIONS.md` §open): the grain — and only the grain — is sampled
   triplanar, ridge-folded per plane before the blend and the blend's
   deviation restored by `1/|w|`. Measured on the 46.6° face this spawn
   offers: tilting the ground coarsens the world-XZ grain ×2.017 and the
   shipped one ×1.397, a gain of ×1.444 against a ×1.456 stretch, at
   ×1.044 amplitude. The gate for it (`browser_smoke` 15c) is a within-run
   comparison against a compiled `flatgrain` partner at two square-on
   cameras, and it goes red — ×1.000 — the moment the tap stops reading
   the normal.
   What is left of the old branch is `gmHash4`: the four lattice corners
   of a noise sample evaluated in one `vec4` body instead of four inlined
   scalar ones. It is image-identical by construction, and it has never
   been gated on its own — the pattern to copy is `noskip`: compile a
   `noh4` variant with materials v0's scalar hash and require the same
   frame. It matters more after the projection than before it, because
   grain now takes three noise samples where it took one, so the ground
   pays 6 sample sites per fragment against 4 — and a sample site is four
   hash evaluations.
   **Do not re-run the cost question on this box.** Every run of the gate
   takes it again; the six taken while grain was built read +14 ms
   (0.3× the floor), −74 (1.7×), −64 (1.3×), −110 (2.2×), −104 (8.2×),
   −62 (0.1×) — five of six the wrong sign, less work measured slower.
   Grain lands exactly where level 0's PCF landed, inside a floor that is
   a one-sample estimate itself and swung 13–603 ms across those same six
   runs. A seventh reading is not a tiebreak.
   The counted budget is the one that answers, and it now has three axes,
   all asserted: 81,820/96,000 program chars, 6/8 noise sample sites per
   fragment, 18/24 depth fetches. Price `gmHash4` there too — hash
   evaluations and program chars — not in ms.
10. **A tab that boots beside another live tab takes 34 s to reach the
   world. Nobody knows where those seconds go.**
   The third-tab version of this went red on 2026-08-01 16:26 (`inWorld`
   at 61.6 s of a 60 s window) and the recovery pass closed it, but by
   removing the *contention*, not the cost: the gate now closes tab A and
   tab B once their last assertion is made, so the public tab boots on an
   empty box and joins in **0.3 s**, and a structural check refuses to let
   it boot beside a live tab again. `JOIN_TIMEOUT_MS` was not touched.
   What that bought is the reading this item always wanted, at the harness
   level: **join time is monotonic in live tabs — 0.4 s alone, 34–36 s
   beside one, 55–61 s beside two.** The 34 s is the part still standing,
   and it is still the thinnest margin in the suite: tab B needs a live
   tab A (mutual AOI is M0's exit condition), so no amount of harness
   tidying can hand it a quiet box.
   The *client* half is therefore untouched and is the live risk. Grain
   did not cause it — the frame moved 630 → 638 ms, 1.3% — but nothing
   has measured where the seconds go, and every slice that adds a material
   or a program spends more of them. **Do not fix this by widening
   `JOIN_TIMEOUT_MS`.** Measure it first: the tab's own timeline from
   `#connect` to the first publish, split into wasm load, connect,
   handshake, first compile and first chunk. The cost probe already says a
   terrain program costs ~3 s to compile here, and a fresh tab compiles
   more than one.
11. **Nothing casts past 720 m, and nothing out there has a silhouette.**
   The horizon casts now (`DECISIONS.md` §open) but two limits are stated
   rather than solved: the coarsest clipmap level stops at 720 m because
   fog closes at 1000 m, and past the near ring the only caster is the
   8 m ground itself — the scatter stops at the ring's edge, so a forest
   at 400 m casts nothing and the gate measures the horizon on 2 of 4
   yaws for exactly that reason. A scatter LOD (billboard crosses,
   `TERRAIN.md` §4's "trees get two LODs") is the fix and it is a terrain
   job, not a shadow one. SeedThree's `impostor.js` (`CLAUDE.md` third-party
   credit) is a worked reference for the bake: two crossed alpha cards, 4 tris
   a tree, albedo baked lit-flat-white and re-lit at runtime plus world-space
   normal and roughness, two ortho cameras at 1024², off-thread in a worker,
   with the GPU readback's row order probed ONCE against a known image rather
   than assumed. Its emit side is the part to throw away — a `Group` of two
   `Mesh`es per tree, which at this forest's density is a draw call per trunk.
   Whatever LOD1 becomes, it sways: item 1's wind weight is a vertex attribute
   and a billboard has four vertices to put one on.
12. **A capture the same twice is a gate; a capture that drifts is a vibe.**
   Deterministic capture mode (operator, 2026-08-02, `DECISIONS.md` — the
   Claude-of-Duty adoption row): the client animates off the sim tick / an
   injected fixed-step clock in capture mode — today the RAF loop steps off
   `performance.now()` (`main.js`), so boot-time noise shifts every pixel —
   with a fixed seed, the existing `__gatesDebug.setView` shots, and ONE
   fresh page per shot (state leaks between shots on a shared page:
   exposure-like accumulators, particle age). Then a pixel-diff tool
   against blessed per-box goldens, exit nonzero on any moved pixel, used
   two ways: refactor/optimization passes assert zero diff; feature passes
   regenerate goldens in the same commit — `test_protocol_golden`'s
   discipline, wall 6, applied to frames. v1 scope: solo shard, camera-only
   shots, no remotes in frame. v2, if wanted later: render-from-WAL-replay
   (wall 5 already guarantees the state side). The clock conversion is the
   prerequisite, not the diff tool — grep every `performance.now()` and
   RAF-timestamp use in `web/src` and sort each into sim-driven, cosmetic
   (must switch to the engine clock in capture mode), or UI-only (excluded
   from shots). Settle by tick count, never by time.
