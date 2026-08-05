# Gates · NOW.md — what next

The only list that answers "what should the loop pick up." Top item first.
Done items are deleted, not checked — history lives in git and
`DECISIONS.md`. A loop iteration starts here, ends with gates green.

0. **The outbound move marshalling is gated — done this pass (ui lane), kept
   for what it leaves open.**

   *(Gap pass, iteration 2. From ranked fix 4 of
   `findings/pass-20260805-002720-01-judge.md`: "main.js's marshalling of
   (fromKind, from, toKind, to) into the wasm call is covered only by
   `browser_smoke`, which is off this run. Closing it needs the marshalling
   extracted into something node-importable." Ranked fix 1 of the same report
   — `rustConst`'s unanchored regex — is folded in.)*

   `client_action_move` takes six `u32`s and every wall here is blind to
   swapping two: encoder untouched, action queue not in `state_hash`, one
   type throughout. Now `invmove.moveArgs()`, pure and node-imported, and the
   order is stated once as NAMES (`MOVE_ARG_ORDER`) which `ui_smoke` §N reads
   back out of `bridge.rs` — so it is checked against Rust, not against the
   client's own opinion. `main.js` spreads the result, leaving nothing at the
   call site to transpose. 11 mutants run, all 11 red, including an ABI
   reorder and a rename made in `bridge.rs` alone.

   Two things it leaves:
   - **`bag`, `from_kind`, `to_kind` are all 0 on every call this client can
     legally make**, so no value probe separates those three; the name-order
     check is what covers them. They become separable on the pass that opens
     a second container — the same pass that makes `bag` non-zero.
   - **`browser_smoke` and `vantages` are UNRUN** (operator's `GATES_TIER=fast`),
     and this pass edits `main.js`. Nothing here is claimed to boot.

1. **Recovery, 2026-08-05 — done, kept only for what it leaves open.**

   `ui_smoke`'s `CONT_MAX` check went red on a clean tree. Neither commit was
   wrong alone: the ui lane's `1fe35b0` pinned the Rust alias by NAME
   (`contMaxAlias === "CONT_BAG"`), the systems lane's `4d7a926` legitimately
   grew a third kind and moved that alias to `CONT_BOX`. The merge was red.
   Fixed as both (a) and (b) — see the commit. Mirror now names `CONT_BOX`;
   the gate resolves the alias to a NUMBER, so it is strictly stronger.

   Two things this leaves for the next pass:
   - **No masked gate behind it.** The runner pins `GATES_TIER=fast` this run
     (`restart.sh`), and `ui_smoke` is the last gate before that tier's exit —
     so unlike the usual first-red case there is nothing downstream to expect.
     `browser_smoke` and `vantages` stay UNRUN, by operator config, not by me.
   - **`loop/cont-max-mirror` is now redundant.** A previous pass's swept
     remainder; its diff is adopted here with a real commit message. Its
     salvage worktree is the operator's to remove, not a lane's.

1. **Container contents have never crossed the wire — for a bag either.**
   *(Gap pass, iteration 3, systems lane. From ranked gap 2 of
   `findings/pass-20260804-205133-02-judge.md` ("a raid takes nothing") and
   gap 1 of `-01`; the storage-and-address half landed this pass at wire
   v18, this is what it uncovered.)*

   `WireBag` carries an id and three position quanta and says contents
   "deliberately never cross". So the loot panel both judges rank first is
   not blocked on boxes — **it has no bag contents to draw either**, and
   `Command::Loot`'s take-all is the only container verb a client can
   express. A box changes nothing about that; it just makes the hole
   visible in a second place.

   The shape, and it is one slice for both containers: an open/close
   action subtype (12 of 16 used, three left), per-client "which container
   is open" state, and a contents sync sent **only to the opener** — not
   to everyone in AOI, which would be an ESP leak and a bandwidth bill for
   a panel nobody has open. Then `EV_MOVED` already tells the opener what
   changed. Wire bump + regenerated goldens, systems lane.

   Until it lands the ui lane cannot draw a container panel, and that is
   the honest blocker to quote rather than the box.
## 0. The second container panel — gap 1's other half *(ui lane)*

*From `findings/pass-20260804-205133-03-judge.md` gap 1, "there is nowhere to
put anything, so a base is scenery" — picked as this lane's gap-pass item.*

Landed this pass (the half that needed no `crates/` change): every address the
panel forms is a **(kind, slot) pair**, not a slot number. Bag slot 3 and self
slot 3 were the same integer, so the drag, the pending record, the verdict
match and the rollback all aliased; `ui_smoke` §M drives each. Report 03's
ranked fix 1 (only `len > 0` was asserted, so a transposed move encoded
green) is closed at two of its three hops: §M pins the panel→host argument
order, and `client_smoke` now decodes `client_action_move`'s bytes field by
field. **The third hop closed on 2026-08-05** — the marshalling is
`invmove.moveArgs()` and `ui_smoke` §N holds its order to `bridge.rs`; see
item 0.

Remaining otherwise, all `crates/` — the requests below. The panel still draws
exactly ONE container and says so (`hud.invContainers`); listing a second
there without cells and a contents source would promise a draw it cannot
perform.

> **Cross-lane request → systems, three items, all for gap 1** *(ui lane,
> 2026-08-04)*. In dependency order:
> 1. **Container contents on the wire.** No S→C message carries them:
>    `EventMsg::Inv` has no container id and `WireBag` is id + position, its
>    own doc saying contents stay sim-side. Without one a loot panel has
>    nothing to draw. Needs `(container id, slots)`; 26 event subtypes free.
> 2. **`client_move_readout` must carry the TO kind.** It packs
>    `reason<<24 | to_slot<<16 | from_kind<<8 | from_slot` — 8 bits spare.
>    `invmove.moveVerdict` therefore rejects every non-self FROM kind, which
>    is correct and load-bearing today: without the to-kind a bag verdict
>    cannot be told apart from a self one. That rejection is the last thing
>    between the panel and cross-container drags.
> 3. **`ARCH_BOX` needs slots and a container address** (`deploy.rs:80`), the
>    piece the judge named. (2) unblocks the panel; (3) gives it something
>    worth opening.

> **Cross-lane, not an item: `ui_smoke` is not flaky, and the fix is not the
> world lane's to make.** `ci/gates.sh` went RED then GREEN on an unchanged
> tree on 2026-08-04. Both runs ran the same 289 tests with 0 failures and the
> same gate list; the RED one died before any check executed, on
> `EADDRINUSE 127.0.0.1:8952` at `ci/ui_smoke.mjs:206`, because line 89 hard-codes
> that port and two lanes run the gate concurrently. Not a clock, not a
> timeout — the `bot_smoke`/IPv6 class, a contended resource. **The ui lane has
> already fixed it** (`|| 0` plus readback) and it is unmerged; a second fix
> from here would only conflict on the same line. Merge theirs. Until then
> `UI_SMOKE_PORT=<free>` is the documented override.

> **Cross-lane, not an item: the ui lane's flag-word blocker is cleared, and
> the read changed.** *(systems lane, 2026-08-04. Read this before wiring the
> drag.)* `APPLIED_MOVE` and `STREAM_ERR` were both `1 << 31`. Bit 31 stays the
> error sentinel — `main.js:759` already reads it that way and the fix must not
> need `web/` — so the move verdict moved to a **second applied word**:
> `core::APPLIED2_MOVE`, read through the new export **`client_applied2()`**.
> Word 0 cannot announce word 1 (bits 0..30 are flags, 31 is the sentinel), so
> call `client_applied2()` after *every* `client_on_stream`; it is zero on any
> message that set nothing, so an unconditional read cannot see a stale
> verdict. The ui half is unchanged otherwise: `client_move_readout()` into
> `invMoveVerdict`, on `APPLIED2_MOVE` instead of `APPLIED_MOVE`. Gated by
> `applied_word_is_full_and_bit_31_is_the_error_sentinel` (core.rs — the word
> is asserted *exactly* full, so the next flag cannot land on the sentinel) and
> by `ci/client_smoke.mjs` through the real C ABI. **Unverified in a browser:**
> `browser_smoke` is operator-disabled this run, so "the console.error is gone"
> is a claim the native and ABI gates support and no browser has checked.
00. **systems: the error must leave the flag word — it is NOT a one-line change.**
   *(Gap pass, ui lane. Gap 1 of BOTH `findings/pass-20260804-205133-01-judge.md`
   and `-02-judge.md`: "a player still cannot move a single item".)*

   The client half landed this pass — main.js arms `onInvMove` and routes the
   verdict, so a player can drag. It is armed over a workaround, and this is
   what retires it.

   Both reports call the cure "one constant in `crates/client-wasm`". **It is
   not, and a pass that starts there will hit a wall in ten minutes.**
   `core.rs:38-122` assigns every bit 0..31 of the `APPLIED_*` word — bit 31
   (`APPLIED_MOVE`) is the last one, and `core.rs:115-121` says so in its own
   comment. So `APPLIED_MOVE` has nowhere to move to. The thing that must
   leave the word is **`STREAM_ERR`** (`bridge.rs:64`), because it is not a
   flag at all — it is an error channel multiplexed into a full flag set by
   `client_on_stream`, which returns both.

   Cheapest shape: `client_on_datagram`'s, which already does this right —
   return a code, not flags. Or an out-of-band `client_stream_err()`. Either
   way `ci/client_smoke.mjs:543,807,816,822` assert the error meaning of bit 31
   and `:572,587` assert the move meaning, so both sides move in that commit.

   When it lands: delete `web/src/invmove.js` and its call site in `main.js`,
   and test bit 31 as `APPLIED_MOVE` directly. `ci/ui_smoke.mjs` group L
   already goes red on that commit and says exactly this in its failure text.

> **Cross-lane, not an item: `browser_smoke` is red on a CLEAN tree, and it is
> tab B, not the prop-contrast probe.** Measured 2026-08-04 from the ui lane,
> both on `lane/ui` HEAD `ecf1985` with nothing applied and on a branch off it:
> the same assertion both times — *"tab B: never reached the world —
> unresponsive"*, `__gatesDebug` never published, `2 tab(s) live`, ~68–70 s of
> liveness cap. Tab A reaches the world in under a second in both runs. This is
> the two-live-renderers class CLAUDE.md already names (2026-08-01), on a box
> with no GPU where Chromium is on SwiftShader — not a diff, and not a timeout
> to widen. The operator has `browser_smoke` switched off this run. Anything
> touching `web/` therefore cannot honestly claim the renderer tier; say so.

0. **world: the haven pad, and the road the client cannot see.**
   *(Gap pass. Both judge reports named "the island has nowhere to go" as their
   own top-or-second gap — `findings/archive-prestamp/pass-20260804-173640-01-judge.md`
   gap 3 and `-02-judge.md` gap 2. The coast road half landed this pass; this is
   what it leaves.)*
0. **world: the pad exists but nothing is on it, and the road is invisible.**
   *(The pad's placement + exclusion zone landed — `DECISIONS.md` §open "haven
   pad v0", `tests/haven.rs`. This is what it leaves.)*
0. **world: the pad pays now, but it is still bare ground.**
   *(Placement, exclusion zone and the container ring have landed —
   `DECISIONS.md` §open "haven pad v0" + "haven crates v0", `tests/haven.rs`,
   `ci/haven_prize.mjs`. This is what they leave.)*

   - **The carve, and it is cross-lane.** v0 *finds* a flat site; it does not
     make one. Measured worst relief is **3.76 m over a 32 m pad** — enough
     that a greybox building on it would float or bury a corner. Carving means
     writing `height`, and `terrain::height` has ~50 call sites across four
     crates (`movement.rs`, `collide.rs`, `build.rs`, `deploy.rs` are systems
     lane), and it cannot be half-threaded: a client mesh that sees the pad
     and a collision path that does not is a player standing in the air.
     **Request to the systems lane:** thread a `&Haven` (or a worldgen context
     carrying it) through `height` so the world lane can carve. Until then no
     POI on the pad can be flat — the crates already sit on up to 3.76 m of it.
   - **A structure, not just containers.** The pad has five crates and no
     walls. A greybox a player can walk into is the next thing that makes it a
     place, and it is what actually needs the carve above.
   - **The road reads as a gap, not a road.** `web/src/terrain.js` has no dirt
     band, so the carriageway is just a strip where nothing grows. Parked with
     the operator's "textures are not this lane's remit" call — reopen it if
     the lane's remit flips back. Touching `web/` costs the ~19 min tier.
   - Not done from stage 7: the flattening, and the denser bay-mouth slots (knob).
   - **Nobody has looked at a crate.** The archetype at `props.js` index 9 is
     unverified by anything but arithmetic; `browser_smoke` is off this run.

1. **The sim can play a survival game; the player cannot reach it.**
   *(Operator, 2026-08-04. This outranks every gate-building item below it.)*

   `crates/` ships 15 verbs — Craft, Place, PlaceDeploy, Loot, Upgrade, Lock,
   Feed, Drink, Consume, Use — against 48 items, 36 recipes, 18 building pieces,
   9 deployables, 6 weapons, 5 gatherables. The client renders **the first six
   inventory slots as text strings** (`main.js:1303-1308`) and has no inventory
   grid, no container view, and no way to move an item. Nearly all of that
   content is unreachable, which is the product gap — not test coverage.

   **This is a Rust clone first.** Work that makes it more playable outranks work
   that makes it more provable. Gates still ride along with the feature they
   protect — that is not negotiable and no wall moves — but a gate is no longer a
   valid item *by itself* unless a red wall demands it.

   - **systems:** container move / stack / split, validation ordered BEFORE the
     mutation and computed on the values the client predicted with. This is the
     ui lane's standing request and it blocks them. Then gathering, decay and
     upkeep behaviour — the loop that makes a day matter.
   - **ui:** the inventory grid, the loot/container panel, and drag-move against
     that refusal path. This is the single highest-leverage lane right now.
   - **looks (now the world lane):** what exists out there and where —
     scatter, occupants, monuments, greybox. Textures are parked; see below.

1. **The world is a beach with trees on it. Build the world, not its textures.**
   *(Operator, 2026-08-04. Retargets the `looks` lane; its charter is rewritten.)*

   **The spec exists and is unbuilt — do not design a new one.** `TERRAIN.md`
   §7 is the coast road: a ring ~40 m inland, flattened, dirt, **barrel spawn
   slots along it**, doing what Rust's roads do — pulling players out of their
   bases into a circulation loop where they meet — with zero monument art.
   §8 is the haven pad, and it is the monument hook: every later POI is "carve
   pad + exclusion zone + scatter table". `grep road crates/` returns nothing.
   Both halves are research-backed by `reference/SPAWN.md` (§9.3 their scatter
   clusters and ours does not, §9.4 the squared acceptance, §9.6 per-cell RNG).
   There is nowhere to go and nothing to find, and no texture fixes that.

   **Textures, materials and lighting polish are parked.** They are a solved
   science and not what this build is short of. Frames are no longer captured
   for that lane and no visual judge scores it — correct while the question is
   "is there a world here", wrong the moment it becomes "does it look right".
   Say so here if you think it has flipped.

   **Build it sim-side first, and it costs twenty seconds instead of nineteen
   minutes.** Scatter is already deterministic in `sim-core` (Occupants 1..7)
   and `web/src/props.js` only draws what worldgen decided, so a monument that
   lands in a worldgen slot is seeded, replayable, gated by `terrain_golden`,
   and pays no renderer tier at all. Give it a greybox mesh second, batched.

   Gate it as arithmetic — `ci/pine_shape.mjs` is the standard. Counts, spacing,
   slope, clearance and tri budgets are numbers. A greybox monument a player can
   walk into and a forest that clumps beat one more correct albedo.

1. **Two branches of texture work are unmerged and are NOT lost — read this
   before rebuilding either.** *(Operator, 2026-08-04. Not a queued item.)*

   Nothing judged PASS is stranded: every lane trunk adds nothing `main` lacks.
   These two failed or stopped, so the harness kept them rather than merging:

   - `loop/bark-photo` (tag `salvage/bark-photo`) — judged **FAIL** 2026-08-04,
     +438 lines in `materials.js`/`textures.js`. Report is in the looks lane's
     `findings/`.
   - `loop/m1-surface-grain` (tag `salvage/m1-surface-grain`) — +666 lines in
     `materials.js`/`scene.js`, stopped unmerged, its own `BRANCH-NOTES.md`.

   Both are **texture and material work, which is parked** (item above). Do not
   merge either to clear the list — failed work in the trunk is the one thing
   the judge exists to prevent. If textures are un-parked later, start from
   these branches rather than from scratch; if they are never un-parked, delete
   them in a commit that says so, as a stated decision rather than a skip.

1. **The barrel's systems half is done — the loop now waits on world and ui.**
   *(systems lane, 2026-08-04. Read this before picking the barrel item below.)*

   `BarrelSlot` is smashable: `hits` swings (content, `loot.toml`) open it, the
   table rolls by weight, and the roll stands up a **ground container** at the
   barrel's own address — `backpack.rs`'s store, not a new one, so `CONT_BAG`,
   the move verb, the loot verb, the sync walk and the wire all work unchanged.
   **`PROTO_VER` did not move.** Gates: `tests/loot.rs` (8), two `event_roles`
   payload checks (`EV_SLOT_HARVESTED` is off the uncovered ledger, 9→8),
   `bake_loot` + refusals in `content.rs`, and `test_replay`'s golden
   regenerated **behaviourally** with a `made >= 2` assert so it cannot go
   green on an unarmed fixture again.

   What is left, and neither is systems':
   - **world:** barrels only spawn on the beach today (`terrain.rs` weight row).
     `TERRAIN.md` §7's coast road is what puts them somewhere worth walking.
   - **ui:** the loot panel. It is a `CONT_BAG` container like a death bag, so
     one panel serves both — no new protocol to write against.

   Two §open rows landed with it: "barrel smash hits" and the call to reuse the
   ground-container store (which shares `MAX_BACKPACKS` 256 and its evict
   policy with death bags — stated there, not discovered later).

1. **Smash a barrel, pick up the loot. The whole loop, and most of it exists.**
   *(Operator, 2026-08-04. First concrete target of the playability item above.)*

   Already built: `content/loot.toml`'s `loot.barrel` (8 entries, revolver at
   weight 1), `Occupant::BarrelSlot`, the spoken "node/barrel respawn 20–45 min",
   and `balance.toml` pricing barrel drops in **road-minutes per unit** — the
   economy already assumes a road you run. Missing is the connective tissue.

   - **world:** `TERRAIN.md` §7's coast road, with barrel slots along it. §8's
     haven pad is the monument hook; build the road first, it is the loop.
   - **systems:** make `BarrelSlot` smashable. `gather.rs:32` says "Rock and
     BarrelSlot are not nodes" — that is the line to change. It rolls
     `loot.barrel` into a container, not straight into the inventory.
   - **ui:** the loot panel, against the container the roll lands in.

1. **Gravity is there and jump is not — and jump makes the lintel matter.**
   *(Operator, 2026-08-04. systems lane; it is a wire change, so only that lane.)*

   `movement.rs` already carries vertical velocity as integer quanta, so gravity
   exists and nothing can leave the ground. Add jump: an input bit, an impulse in
   quanta, walled float ops only, quantize-both-sides so prediction holds.

   **`collide.rs` predicted this and left the hole open on purpose** — a doorway
   "blocks only its posts (the 1.2 m opening passes; the lintel never matters at
   capsule height **until a jump exists**)". It exists now, so the lintel becomes
   real geometry and a jump into a doorway head must stop. Land both halves in
   one pass or a player will jump through a doorframe.

   Fall damage is the natural follow-on and is NOT part of this item.

1. **Dropped loot should land somewhere you can find, not inside the floor.**
   *(Operator, 2026-08-04. systems lane.)*

   A dropped item wants a short settle — gravity to the ground, a slide off a
   slope, friction to a stop — so it rolls a little and comes to rest where a
   player can see it. That is a memory hook, not decoration: "it went behind the
   rock" is how you find your own bag again.

   **This is not a physics engine and must not become one.** Integer quanta,
   walled float ops, a hard iteration cap in `limits.rs`, settle resolved and
   then frozen. `sim-core` has exactly one dependency and it stays that way — a
   rigid-body crate breaks walls 1, 2 and 5 at once, and cosmetic shards when a
   barrel breaks are client-only and never feed back.

1. **A base collapses when you take its legs out — today it floats.**
   *(Operator, 2026-08-04. systems lane.)*

   `supported()` runs at PLACEMENT (`build.rs:452`) and nowhere else. Destroy a
   foundation and everything above keeps hanging in the air. Rust collapses it,
   and that is central to how raiding feels — the raid is the game.

   Wanted: support re-evaluated when a piece dies, propagating to what rested on
   it. It is a graph reachability problem over the piece store, not physics —
   walk from grounded pieces, orphans fall. Bound the sweep in `limits.rs` and
   state the overflow policy; an unbounded cascade on a 500-piece base is a
   tick-time bomb, which is wall 4.

1. **There is a revolver in the loot table and nothing to fire it.**
   *(Operator, 2026-08-04. systems lane, after the three items above.)*

   `combat.rs` is melee-only — grep finds no projectile, ballistic or ranged
   path — while `loot.toml` drops `item.revolver` at weight 1 and
   `content/weapons.toml` authors six weapons. The rarest barrel drop in the
   game is currently a paperweight.

   Ranged v0 is the smallest honest fix, mirroring how melee landed: the swing
   that fells a tree also lands on a person, so the shot that hits a barrel also
   hits one. Lag compensation and rewound raycasts are `NOW.md` M2 and are NOT
   this item — say plainly in the commit what is unlagged.

1. **The container verb has no UI and no gate — and the systems half is not
   ours.** *(ui lane, 2026-08-04, after `ci/ui_smoke.mjs` landed.)*

1. **The container panel: the refusal path exists now, so the UI is
   startable.** *(ui lane, 2026-08-04, after `ci/ui_smoke.mjs` landed.
   Systems half landed 2026-08-04, wire v17.)*
1. **The inventory drag is built and gated; one bit in `crates/` stops it
   reaching the sim.** *(ui lane, 2026-08-04. Supersedes "the inventory screen
   draws all 30 slots now; it still cannot move one" — the panel half landed.)*

   In `hud.js` + `ci/ui_smoke.mjs` group K, inside the armed carve-out so it
   pays `ui_smoke` and not the renderer tier: `beginInvDrag` / `dropInvDrag` /
   `cancelInvDrag` / `invMoveVerdict`, driven by real pointer events, plus the
   `REFUSE_M_*` → sentence table read off `inventory.rs`. The ordering law is
   the whole point and every clause is a check — validate the address before
   touching a cell; ask the host to encode BEFORE drawing, because a drawn move
   with no frame behind it IS the divergence; one move in flight; a verdict
   applied only when its address matches the prediction; and an authoritative
   `setInventory` outranking the rollback snapshot. Eight mutants, all red.

   **Systems lane, one-line request — this is the blocker.** `APPLIED_MOVE`
   (`client-wasm/src/core.rs:122`) and `STREAM_ERR` (`client-wasm/src/bridge.rs:64`)
   are both `1 << 31`. `main.js:759` reads that bit as a decode error, so the
   first `Moved`/`MoveRefused` logs `console.error` — which fails the browser
   gates — and returns early, dropping the inventory diff in the same message.
   It needs a distinct sentinel; the flag word is full and `core.rs:122` says so.

   **The UI half left, once that clears:** set `hud.onInvMove` in `main.js`
   (the host owns the count — the panel is handed strings, and a panel parsing
   "wood ×8" back into an 8 would be inventing its own payload), then read
   `client_move_readout()` on `APPLIED_MOVE` into `invMoveVerdict`. That touches
   `main.js`, so it pays the renderer tier. Stack split and the loot/container
   panel are the slices after it.
1. **The drag's release side is closed; the arming decision is made.**
   *(ui lane, 2026-08-04, from the judge's ranked fixes 1–3,
   `findings/pass-20260804-205133-01-judge.md`. Not a new item — what remains
   of the drag is the systems blocker in the item above.)*

   The cancel was bound to `#inv`, so a release on the world — the release a
   player actually makes — was never seen: `invDrag` stayed on the source and
   the next press's release ran the drop against it. Press cell 8, sim asked to
   move cell 3. Now on `window` (`pointerup`, `pointercancel`, `blur`), scoped
   to the `pointerId` that began the drag.

   Two more of the same class found while in there, both fixed: a **second
   pointer's release** finished the first pointer's drag (the one-drag guard
   refuses the second *press* and never had anything to say about its
   *release*), and it must not cancel the live drag either. And ranked fix 3 is
   answered by **not offering the gesture**: `beginInvDrag` refuses while
   `onInvMove` is still `Hud.NO_MOVE_HOST`, so nothing dims and nothing toasts
   until a host claims the verb — arming is identity against that sentinel, so
   `main.js` assigning it is the whole of the arming step.

   Gated in `ui_smoke` group K (175 checks). Nine assertions added; eight
   mutants of `hud.js` run, all eight red. The ninth mutant — `cancelInvDrag`
   leaving `invDragPointer` set — **escaped** the first eight and is why the
   `doors` case exists: the two fields are one piece of state.
1. **The props' photograph: `wood` and `foliage` still have none.**
   *(From the visual judge's ranked gap 1, `findings/pass-20260804-153032-01-visual.md`:
   "the terrain got a sourced photograph this pass and the props did not — this
   is a coverage gap, not a tuning one." Half of it landed as `DECISIONS.md`
   §open "prop photograph v1"; this is the half that did not.)*

   `rock` and `ore` now sample the granite layer of the array the ground
   already had, triplanar, mean-preserving, luma only. Three things remain,
   in order of what the judge measured:

   - **`wood` gets bark.** `assets/textures/bark_{albedo,normal,rough}.jpg` are
     on disk, in `MANIFEST.md`, and imported by nothing. They are not in the
     ground's four-layer array, so this needs either a fifth layer (which moves
     `GROUND_LAYERS` and the splat index that is asserted against it — not
     free) or a second, prop-only array. The second is the smaller blast radius.
   - **`foliage` gets needle cards**, which is geometry, not a map — the judge
     is explicit that "no material work saves a smooth cone", and that is the
     generated pine in the item below, not a texture.
   - **The frequency split.** The field and the photograph are both live on
     `rock`/`ore` albedo now, and per the pack's own rule two uncorrelated
     deviations on one channel add variance rather than detail. The fix is to
     hand everything above the tile frequency to the photograph and leave the
     field the coarse per-instance patchiness a tiling map cannot supply —
     which means splitting `PROP_DETAIL_SHARE` into an albedo share and a bump
     share, since zeroing it today would take the bump with it.
   Not startable here: drag/drop, stack split, the loot panel.
   `client_action_loot()` is payload-free (`main.js:426`) so there is no
   container view to draw, and a drag the sim cannot refuse is the divergence
   CLAUDE.md's item-move trap describes.

   **Systems lane, unchanged one-line request:** container move/stack/split in
   `crates/`, validation ordered BEFORE the mutation and computed on the values
   the client predicted with. Three Oxide fixes in 28 minutes on one 2019 day
   were all splice-point moves that landed as *the server disconnecting the
   client*. The panel is built and gated; wiring a drag to it is a small pass
   once that refusal path exists.

   Deliberately not drawn: worn/armour slots. `inventory.jpeg` has a
   paperdoll, the client has no worn-slot data, and empty slots for a system
   that does not exist are decoration. The renderer-tier carve-out that would
   make this lane cheap is still **not armed** — `DECISIONS.md` §open.

1. **The generated pine is built, gated, bundled — and not drawn.**
   *(Found while recovering the red join gate, 2026-08-04. `DECISIONS.md` §open
   row "the pine is generated" and the comments in `props.js`/`terrain.js` all
   say the near ring draws it. It does not.)*

   `ARCHETYPES[1]` (`web/src/props.js:400`) carries no `parts:` key, so
   `terrain.js:193`'s `a.parts ? a.parts() : …` takes the else branch and the
   near ring still draws the 102-triangle cone. `pineParts` is imported at
   `terrain.js:45` and never called — the tell. `ci/pine_shape.mjs:315` calls
   `pineParts()` directly, so it scores a generator nothing renders and stays
   green either way; the bundle ships ez-tree's base64 textures regardless.

   Do not just add the key. Wired as-is the ring costs 416 × 6,496 × 3 passes
   ≈ **8.1 M triangles against DESIGN §9's 1.5 M** — 5×over, and
   `browser_smoke`'s own budget assertion would catch it. The billboard LOD
   (item below, `TERRAIN.md` §4) is the prerequisite, exactly as that commit's
   own message said. Two honest ways to close this: land the LOD first, or
   revert the wiring and the dependency and say so. Either way `pine_shape.mjs`
   should assert the FLEET cost — it already prints the 416-tree arithmetic
   eight lines above a ceiling justified by "~20 trees inside 40 m".

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

   **Update, 2026-08-04 (join-gate recovery pass).** On post-merge `main` this
   assertion PASSES: the probe read `contrast x1.15` and `1.15 >= 1.15` is
   true. It has not moved and nothing here is fixed — the value is still
   sitting on the floor and passes by rounding, so the next run that shaves a
   thousandth off it is red again. Treat this item as live.

1. **Two stopwatch-shaped waits survive in `browser_smoke`, and one masked
   gate is expected.** *(Left by the join-gate recovery pass, 2026-08-04.)*

   The join now ends on the client's own progress, not on elapsed time
   (`DECISIONS.md` §open, "the join is watched, not timed"). Two waits of the
   same shape were deliberately NOT touched, because a recovery pass fixes the
   red gate and not its neighbours:

   - `waitForRemote` (`browser_smoke.mjs`, `AOI_TIMEOUT_MS`) — 60 s of total
     elapsed time for a remote to enter AOI, polled serially. It has never
     fired. Under the same starvation that made the join a coin flip it gets
     very few polls, so it is the next one to go red.
   - `PLAY_MS` and the held-walk floors are wall-clock by construction.

   Both are subsumed by the item below (tab B as a bot), which removes the
   contention rather than instrumenting around it. That is the real fix.

   **And expect a masked gate.** `ci/gates.sh` stops at the first red, so
   everything downstream of the join has been unobserved for two health runs.
   A full run on the recovery branch reached the end, so nothing is known-red
   right now — but the prop-contrast item above is passing on a rounding tie,
   which is the most likely next red.

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

1. **The event lane's payloads are law with no gate — nine codes
   left.** *(Operator, 2026-08-04: top priority. Ledger now 18/27 — wire
   v17 added `EV_MOVED` and `EV_MOVE_REFUSED` and classified both in the
   same commit, which is the rule this item exists to enforce.)*

   Every event is `push(code, a, b, c)` over three untyped `u32`s and the
   `/// EV_*:` lines in `world.rs` are the only statement of which is
   which. Swap two at an emit site and every wall stays green: the golden
   pins the *encoder's* bytes and an emit site is not the encoder,
   `state_hash` excludes the ring by design, and a `u32`-for-`u32` swap
   type-checks. `reference/FINDINGS.md` §1 has why this outranks the queue.

   **Landed**, `crates/sim-core/tests/event_roles.rs`, 18 of 27 by role,
   with four disciplines that keep it able to fail: `distinct3`,
   `distinct_halves` and `distinct_triple` (a packed field whose parts
   match cannot show the pack reversed), and `only` (refuses zero *and*
   two, so it doubles as a double-emit gate). The raid arrangement added
   for `EV_STRUCT_HIT` stands the whole thing a storey, which is what made
   the `level` seat falsifiable for the first time.

   **The remaining nine**, by swap silence: `EV_WEAK_MARK` first, then
   `EV_SLOT_HARVESTED`, `EV_CRAFT_DONE`, `EV_RESPAWN`, `EV_BAG_REMOVED`,
   `EV_SLOT_RESPAWNED`, then the three refusal codes last. Move
   `UNCOVERED` in the same commit as `COVERED`.

   **The refusal three need a fixture move, not just a test.** Every
   refusal push is `(code, p.id, REFUSE_*, 0)` — same shape at all 46
   sites — so `a`/`b` are only distinguishable if the player id and the
   reason code differ. `EV_CONSUME_REFUSED` got there by accident (body 1
   vs `REFUSE_C_NOT_FOOD`); the build and deploy refusals have reason
   codes in the same 0..9 range as a player id and will need the raider's
   id moved, the way `BUILDER = 4` was moved for `EV_STOCK.c`.

   **Bigger swing, unbuilt:** a payload-role table both the emit site and
   the check read, making a swap a *compile* error. Should not block this.

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

   The wire counters, as of wire v17: the event subtype field is **37 of
   64** used (v13 widened it 5 → 6 bits, which is why there is room), and
   the action subtype field is **12 of 16**. The next C→S verb — a repair,
   a throw, a container open — is an action subtype, and there are three.

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

   **It now reaches past join, into the prewarm gate (ui lane, 2026-08-04).**
   Four runs, same commit: tab B seen at 87.5 s and the pass FAILED on
   `programsAtInWorld was never pinned`; at 63.5 s on a quiet box the same
   commit passed with 0 late links. Clean `lane/ui` at 90.9 s also passed, so
   the threshold is not a clean line — it is load. The pin needs four in-world
   RAF frames and the box was serving p50 1166 / p99 2600 ms frames, so tab B
   can be alive and answering `page.evaluate` while still short of four
   frames. The assertion is a COUNT and correct; what it depends on is a
   frame budget nobody has bounded. Read it as one more reason the seconds
   below matter, and confirm any red here on a quiet box before believing a
   diff caused it.

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

13. **M1 — survival verbs** + bags, hotbar, chat (`ALPHA.md` §1/§6).
   Gather, craft, build, and deployables are sim'd, on the wire, and
   solid (slice 13: chat — two channels on the reliable lane, local at
   20 m and global, sanitized at both edges and rate-limited per
   connection, deliberately outside the sim so a replay never depends on
   what anyone typed; wire v10 spends the last kind code on it, T opens
   the composer and `/g ` sends global, and the browser gate now types a
   line in one tab and reads it out of the other's DOM).
   Next: **shared access** — one owner id gates a door and a hearth
   today, so two friends cannot share a base; whether that arrives as a
   code lock, a hearth auth list doors inherit, or crews needs the
   operator's word (`DECISIONS.md` §open, lock v0 row) ·
   death/backpack/respawn-on-bag (bags place + cap now; the anchor lands
   there) · piece damage (M2's raid lane: hp exists and decays, nothing
   attacks it yet) · nametags (chat names a speaker by id today, because
   nothing has a name yet).
14. **M2 — combat true**: lag-comp ring + rewound raycasts · ballistic
   projectiles · satchel + damage-by-tier · day/night · netem feel bar.
15. **M3 — economy dark + ops**: OBOL machinery behind the A1 switch ·
   admin lane · backups · status page · error capture · `bench_transport`.
16. **A1 playtest** (operator schedules): 10–20 testers, one wipe cycle —
   then tune content bands from what the anomaly log and the replays say.
17. **M4 — arm A2, then A3** (operator acts): claim rail export · skin
   catalog · the board delivery (repo + playable link + a recorded round
   whose replay hash checks) on `munus-first-sale`.
18. **Anti-ESP occlusion culling — the measure the genre proved, and the one
   the seed makes cheap** (`DECISIONS.md` 2026-08-04). AOI at 176 m is the
   whole ESP defence today, and 176 m covers most engagements. Facepunch's
   answer, rolled out 2025 and defaulted network-wide, was to stop
   networking players fully occluded by terrain — and they pay to compute it
   live on a Unity server. Here the terrain is a pure function of the seed,
   so the occlusion grid bakes at worldgen into a fixed structure and the
   tick spends a lookup: no allocation, no clock, walls 1 and 2 intact.
   Lands as a filter on the enter/leave sets of `NETCODE.md` §7's one grid,
   with a golden beside `test_terrain_golden` and a bot-measured tick cost.
   Sequence after M2 — it wants real sightlines and real combat to tune
   against, and it buys nothing until a shard is armed.
19. **The launcher, in Rust, with the wallet in it** (`DECISIONS.md`
   2026-08-04). One static binary, `egui`, no webview: patcher, shard list,
   balances, and a self-custody wallet on `alloy` (`alloy-signer-local`,
   `keystore` + `mnemonic`) signing the EIP-191 `gates join <shard> <nonce>`
   the server already accepts — so no protocol moves and nothing enters the
   sim's blast radius. Key backup is the feature, not a footnote: phrase
   shown once and confirmed back, encrypted keystore only, never logged and
   never in the WAL, connect-existing kept first-class, and the plain
   sentence that the operator holds no keys and can restore nothing. M4
   adjacent — it is the platform's client for the whole cascade, not a Gates
   accessory, and it is the only place an anti-cheat bootstrapper could ever
   live if one is spoken.

20. **`cargo test --workspace` overflows a debug thread's stack; only
    `--release` (what CI runs) is green.** Pre-existing, not new: verified
    on `main` at `25f6ec8` before the backpack slice, where
    `snapshot_budget` aborts the same way. The cause is size, not logic —
    `World` is ~416 kB of fixed capacity and `ShardCore::new` builds it on
    the stack, so an unoptimized frame holds two or three copies against a
    2 MB limit. It bites anyone who types the obvious command. The fix is
    the one this slice already used for its own store: box the big
    fixed-capacity members (`Pieces`, `Deploys`, `SlotLives`) at
    construction, the way `ShardCore` already boxes its client array —
    one allocation at boot, none in the tick.

Standing rule: anything a playtest breaks jumps this queue; anything a
wall catches jumps the playtest.
