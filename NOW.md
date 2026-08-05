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
| `salvage/ranged-v0` | ranged weapons — `ranged.rs` (402), `pitch_lut.rs` (285), `tests/shoot.rs` (695), `ci/gen_pitch_lut.py` | judged FAIL, wall 6 — and the branch's own `NOW.md` text says why: *"already on the wire, so the wire did not move and `PROTO_VER` did not bump"*. A shot arrives as `EV_HIT`/`EV_HEALTH`/`EV_DEATH` and nothing else, so **no client can tell an arrow from a swing and nothing can draw the projectile**. The wire half it names as missing — an `EV_SHOT` code, its subtype, a `PROTO_VER` bump, 66 regenerated goldens — is the rebuild's real scope, and `ACT_MAX` is full (§0r item 4), so it also widens `ACTION_SUB_BITS`. The judge report itself was pruned; this reading is off the diff, not the report. **The sim half is good work** — bounded (`MAX_ARROWS` 128, `MAX_ARROW_LIFE_TICKS` 120, integer `ARROW_STEP_MM`) and heavily tested. Start from the branch, not from scratch |

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
