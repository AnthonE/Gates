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

## 0y · The sea is a volume — what it still cannot do *(client lane)*

Landed 2026-08-08. Water was one translucent plane at one alpha, and the last
visual report's note that it "has no wave normals" was the smallest half of it.
Now: `render/water.rs` (one eye-centred mesh, four-wave swell with analytic
normals retired at each wave's own Nyquist, per-channel optics off
`exp(-depth·σ)`, shore foam slope-weighted, a tiling ripple normal map with its
own mips), `terrain_mesh::wetted` (the land side of the waterline), and
`sound/water.rs` + `sound::Snapshots` (a surf bed, a submerged snapshot, a
splash). Research is `reference/WATER.md`; knobs are `DECISIONS.md` §open,
"water v0" and "water audio v0". Gated by `crates/client/tests/water.rs` (22)
and eight new assertions in `tests/sound.rs`.

**Two defects were found by looking, not by a gate**, which is the point:
straight alpha blending scaled the sky's Fresnel reflection away in shallow
water (the shallows rendered as wet sand), and a shallow→deep body-colour lerp
made the sea a grey sheet because it treated extinction as if it darkened the
water's *own* light. Both are physics errors with physics fixes; both are
written up where they were made.

**The shoreline was a second pass, from the operator holding a reference frame
beside ours and asking why ours is a cut.** Reading that frame: there is no
edge in it anywhere — a wide damp gradient, then wet sand, then water thin
enough to be only a sheen, and the white is a soft streaky wash standing
*offshore*. Three of ours were wrong. Foam peaked at zero depth, which outlines
the seam it should hide (it now peaks 0.6 m out and is zero at the edge); its
band edges were exact iso-depth contours of a smooth heightfield, which reads
as drafting (a noise field displaces them into lobes); and the damp band was
keyed on height alone, so it was sixty metres wide on a gentle beach and sixty
centimetres on a steep one (it is bounded by a horizontal run as well). Plus a
surge, because a moving edge cannot be a hard edge.

Remaining, in order:

1. **The last hard edge needs the depth prepass, and that is the next slice.**
   The alpha ramp is a *vertex* quantity off `terrain::height`, so it fades
   correctly against the terrain and not at all against anything else — a
   boulder, a foundation or a player in the shallows gets a ring, because no
   vertex of the sea has heard of them. The fix is standard: sample the depth
   prepass in the fragment, fade alpha and add foam as the scene depth
   approaches the water's own. It needs an `ExtendedMaterial` and the **first
   WGSL in the tree** (`RENDER.md` §8), and SSAO already puts a depth prepass
   on the camera, so the input exists.
2. **There is one sea state and no weather.** The wave set is a constant, so
   it is always this calm. A storm is `WAVES` scaled by a scalar the sim would
   have to publish, which makes it wire, not renderer.
3. **Nothing reflects.** `reference/WATER.md` §5 says reflections are the
   expensive half and §6 says the payoff is the *sky* — which the atmosphere's
   specular already gives. A screen-space pass is a real want and not a cheap
   one; read those two sections before starting.
4. **Underwater is audio-only.** A colour grade under the surface is a second
   owner of the frame's haze, which `CLAUDE.md`'s coupled-lighting law
   forbids; it wants the lighting owner, not this lane. The mix ducks and the
   bed goes dark, and that is all.
5. **The submerged duck is not a filter.** rodio gives gain, rate and panning;
   a real low-pass needs a DSP node. Stated in `SNAPSHOTS`, not implied.
6. **`Splash` is the only producer of the waterline.** Swimming has no stroke,
   no wake and no interactive deformation — the reference merges an
   interactive sim into its own displacement (§3) and we have no producer for
   one.
## 0m · The pig is in — what the roster still owes *(systems lane)*

Landed 2026-08-08 (operator: *"let's get a pig in"*). 64 fixed roster slots,
homes drawn from the seed, staggered think, dormancy at 240 m, a leash, a
flight, and a kill that pays fat and cloth. No navmesh: the terrain is a pure
function, so the animal steers and `movement::step` decides — which also
means it inherits tree and piece collision for free. Research is
`reference/ANIMALS.md`; the design call is `DECISIONS.md` 2026-08-08.

**Three defects were found by booting the game and looking, and every gate
was green through all of them** — which is what `CLAUDE.md`'s "the operator
boots the game and looks" is for. (1) `flee_pct = 100` made the pig run at
exactly the player's sprint, so it could never be caught or melee'd; now 70,
and `tests/content.rs` gates `flee_gait < 127`. (2) The massing wore
`props::tint1`, a **mean-1** modulation meant for a photograph, and rendered
near-white on an untextured material; `boxes_mesh_with` splits the two and
`tests/mob_mesh.rs` gates the mean. (3) `bodies.rs` drew a humanoid rig at
every pig's position as well, because its only filter was "not me".

**Read §0v directly below this one.** It landed the same day, from another
lane, and the two items are each other's missing half: the oven cooks
nothing because "nothing on this island is raw", and the pig is the first
thing on the island that could be. Closing it is content only — a
raw/cooked item pair, a `drops` row here, a cook row in `cooking.toml` —
and it is left undone because the food set is a spoken knob (`DECISIONS.md`
§open, "the food set"), not because it is hard. Whichever lane takes it
should take both items at once.

Owed, in rank order — `reference/ANIMALS.md` §9.5 has the reasoning:

1. **No corpse.** A killed pig leaves the snapshot and is gone; the loot is
   paid straight into the killer's inventory as `EV_GATHER`. The reference's
   actual interaction is a *butchering verb* on the body, which is a verb to
   design, not a species to add.
2. **Nothing fights back.** Needs a mob→player damage path, a new death
   cause on the wire, and a combat-feel answer to being hit by something you
   cannot reliably hit back.
3. **No sound.** `sound/synth.rs` generates the bank at boot and has no
   voice for an animal; the reference identifies a boar by its snorting
   before you see it.
4. **No animation, and the massing is boxy up close.** Legs do not move, so
   a walking pig slides, and at 8 m the head barely separates from the body
   (captured 2026-08-08). `anim.rs` drives the player rig off a glTF and there is no
   equivalent here; the cheapest honest version is a per-leg transform off
   the interpolated speed, not a rig.
5. **`MAX_MOBS = 64` has never met a playtest.** It is derived (§ the wire
   budget) rather than felt, and it is the one number a player answers.

---
## 0v · The fire cooks nothing, because nothing on this island is raw *(systems lane)*

Landed 2026-08-08: the oven (`sim-core/oven.rs`, `DECISIONS.md` §open "oven
v0"). A campfire lights on `C`, opens on `E`, burns its wood at the
reference's own rate, banks charcoal at the reference's own 75%, cooks what
`content/cooking.toml` names, and snuffs itself when it runs dry. The
furnace is the same class — a fire and a furnace differ only by which cook
rows name them, which is how the reference builds it (`BaseOven`).

**What is missing is the food, and it is not the oven's fault.** The table
ships with zero `[[cook]]` rows: the alpha food set is berries, mushrooms
and corn, only berries are payable by anything in the world, and none of
the three is a thing you cook. The meat was cut with the animals that would
drop it (`DECISIONS.md` §open, "food set"). So the shipped fire's job is
fuel → charcoal — real, and a T0 source for the powder chain — and cooking
is a table with no rows.

Two ways to close it, and the choice is the operator's because it is the
food-set knob:

- **A raw food the world pays.** An animal (a spawn class, a strike, a
  drop), or the forest-floor pickup mushrooms have wanted since the
  survival clock landed — that one is a scatter occupant, so it moves
  `test_terrain_golden`. Then one `[[cook]]` row and one consumable row,
  no code.
- **The burnt state.** Reference-true and free once food exists: a burnt
  row is a cook row whose input is the cooked item, so it is content, not
  a mechanic — but it cannot be demonstrated before there is a cooked
  item to overcook.

Also still open, and deliberately: the furnace's ore rows are still
station-gated crafts in `recipes.toml`. Moving them into the oven is the
reference's model and re-prices the whole powder chain against
`CONTENT.md` §4's bands — a balance pass with an operator's number on it,
not a refactor.

## 0u · The ghost tells the truth — what it still cannot promise *(client lane)*

Landed 2026-08-07. The build ghost drew a doorway as a SOLID SLAB, so the
preview of a doorway hid the one thing a doorway is (`RENDER.md` §8: the
opening is what `collide::edge_hit` refuses, and drawing it elsewhere makes the
frame lie about where a player can walk). It is three parts now — two posts and
a lintel — off numbers shared with `structures.rs` rather than copied.

Also landed: a **deploy ghost** (right-click outside build mode used to place a
box blind, and `deploy_key`'s own header says guessing wrong costs the item);
the refusal reason and working level shown while AIMING rather than after the
click; and `NotShadowCaster`, which the module header had claimed since it was
written while the code cast a shadow the whole time.

Gated: `crates/client/tests/ghost.rs`, five assertions against
`sim_core::collide::doorway_solid_at` — the sim's own predicate, extracted so
the renderer is checked against the rule rather than against a copy of it.
Three mutants run, all red.

Remaining:

1. **Which parts exist and where they go is still written twice** — once in
   `ghost::shape_parts`, once in `structures::spawn_piece`. The dimensions are
   shared and the doorway is gated against the sim, but a shape added to one
   and not the other passes. One shared parts table both emit from is the fix.
2. **The deploy ghost says WHERE, not WHETHER.** `place::verdict` answers for
   build pieces only; the `REFUSE_D_*` set (needs a floor, needs a doorway,
   hearth claim) is uncheckable client-side, so the preview is deliberately
   neutral-coloured. Colouring it needs those checks mirrored, which is the
   quantize-both-sides law applied to placement.
3. **A door aimed at a doorway is not previewed as a door.** `deploy_key` sends
   a plane-shape target and lets the sim answer `REFUSE_D_DOOR`; the ghost
   inherits that and draws the door's box on the cell body rather than in the
   edge it would fill.
4. **Stairs are still a flat slab** in both the ghost and the piece — a ramp
   drawn as a plate. Shared, so at least they agree.

## 0v · Players are people — what the rig still cannot say *(client lane)*

Landed 2026-08-07. Remote bodies were `Capsule3d` pills that slid and never
faced anything, though the wire has carried `yaw` since the first snapshot and
`bodies.rs` never read it. They are a skinned mannequin now (CC0, 46 clips,
`assets/models/MANIFEST.md`) with gait chosen from derived speed, plus a held
tool with bob/sway/swing (`render/viewmodel.rs`).

Remaining, ranked:

1. **Crouch, jump and swim are wired to nothing.** The clips are in the file
   and the WIRE does not carry the facts — no grounded bit, no crouch bit — so
   `BodyAnim` cannot see them. This is a protocol change (wall 6: version bump
   + regenerated goldens in the same commit), not a client one.
2. **No attack, gather or death animation on a remote.** `Feed` carries the
   LOCAL player's hits only; a remote's swing is not a fact the client is told
   about. `EV_*` has the events — this needs the draw path to read them per
   body, which is the same gap `RENDER.md` §8 item 4 names for pieces.
3. **Nobody holds anything.** The viewmodel is first-person only; a remote
   mannequin has empty hands. The rig has hand joints, so this is an attachment
   to a named joint rather than new art.
4. **Root motion is ignored.** `Jog_Fwd_Loop` translates in place here because
   position is the interpolator's; the `_RM` variants in the library are the
   root-motion cuts and are deliberately unused. Feet will slide at speeds
   between the clips' authored ones — the fix is scaling playback rate to
   speed, which is a knob nobody has measured.
5. **A plain worn-steel albedo is the missing texture.** The axe head carries
   no map because the only metal in `assets/` is ribbed corrugated sheet
   (`viewmodel.rs` and `assets/textures/MANIFEST.md` both record it).

## 0w · The props carry a photograph — what is left after it *(client lane)*

Landed 2026-08-07. Every non-ground surface was a flat `base_color`: 34 CC0
textures shipped and **2 were sampled**, because no procedural mesh in the
client had a UV. `props::Soup` box-projects per triangle (free on a soup — no
shared vertices, so no seam and no shader), `blob_mesh` subdivides and displaces
instead of being a 20-triangle icosahedron, and bark/wood/stone/metal/rock are
bound. Licence rail widened the same day: `DECISIONS.md` 2026-08-07.

Remaining, ranked by what the captures show:

1. **The hemisphere fill, and it is now the top visual gap.** p10 71.0 against
   a reference 41.0 — props v1 moved it 13 the wrong way by removing the
   frame's accidental darks (`RENDER.md` §0). One owner, one iteration, inside
   the coupled lighting set; do not touch it from a parallel lane.
2. **Trees are small and sparse in the midground.** The wide vantages are an
   empty green plain between the near clutter and the far ridge, where the
   reference frames are dense. This is `terrain::scatter`'s density and the
   conifer's scale, not a material.
3. **Nothing sits IN the ground** (`ART.md` rule 2). The new boulder has a
   clean elliptical intersection with the turf and no crowding or dirt skirt —
   more visible now that the rock reads as a real object.
4. **The far mesh speckles.** Grazing-angle aliasing on the 8 m LOD; the
   candidate is anisotropy, registered at 4 for a browser reason that does not
   survive the port (`ART.md` §7), so it is a proposal not an edit.
5. **The viewmodel is two untextured boxes**, and it is in a third of the frame.
6. **Roughness maps are still unread** — all nine of them. Blocked on an ORM
   packing step, not on a slot: `metallic_roughness_texture` is glTF-packed and
   its B channel is metallic, so a greyscale rough jpg would make every surface
   a half-metal.
## 0p · The UI has a face and a measured palette — icons are what is left *(client lane)*

**Second pass, 2026-08-07** (operator: *"the one you made looks super sub par…
im willing to do anything to help us recreate this UI"*). The palette was
re-derived off `Rust Images/crafting.png` and was not a near miss — cool
near-black selecting in olive, against a reference that is warm, twice as
light, and selects in **blue**. The vitals became **bars** off the same frame.
`DECISIONS.md` "ui palette v1" has the samples and the one caveat.

**Half-answered 2026-08-07** (operator: *"just remove the inner ring"*). The
wheel merged two reference menus into one; the material ring is now cut and a
blueprint places the bottom rung, which is what the reference's does. Nothing
was lost — `sim_core::build::upgrade` and `ACT_UPGRADE` predate this and `U`
already sends it. `DECISIONS.md` "the build wheel is one ring".

## 0p2 · What the UI still owes *(client lane)*

**0 · Before the hammer's wheel: the action lane is FULL.** Demolish and
rotate have no `Command` in `sim-core`, and adding either is not "add a code":
`ACTION_SUB_BITS` is 4 and all sixteen codes are live, with a `const` assert
and a unit test (`the_action_lane_has_the_room_it_claims`) both stating the
room as **zero**. A seventeenth action widens the field to 5 bits, which moves
every action message by one bit — a `PROTO_VER` turn and all 72 goldens
regenerated, the v12 turn again. That is a price worth paying **once, for both
verbs at the same time** (5 bits holds 32), not twice.

And **rotate would be invisible in our game today.** A placed piece is
`{cx, cz, level, loc, row, hp, uh}` — no facing, nothing to turn. In the
reference rotation matters because a wall has a soft side and a doorway has a
hinge side; ours are symmetric boxes. So rotate is a facing field across the
store, `persist.rs`, the world save, the snapshot wire and the renderer, in
service of a difference a player cannot see. **Demolish first, rotate when
pieces have an asymmetry worth turning.**

**1 · The hammer wants its own wheel.** The mouse is held-item modal now
(`DECISIONS.md` 2026-08-07) and the plan's half is done: hold right for the
shape ring, left click places, the ghost is up for as long as the plan is
out. The hammer's half is not. It has left-click repair and keyboard
`U`/`R`/`X`, but **holding right with a hammer opens nothing** — deliberately,
because opening the shape wheel would place with the wrong verb.
What it wants is the reference's second radial: demolish, rotate, upgrade,
repair, pick up, **firing on pick rather than latching**. `ring.rs` already
bakes an annulus from a segment count, so the drawing is close to free; the
work is a second `Panel`, a second segment set, and an action-on-release path
that the shape wheel deliberately does not have. Two verbs it would want that
the sim has no command for: **rotate** and **demolish**.

**2 · ~~A starter kit~~** — landed 2026-08-07, `DECISIONS.md` "spawn kit v0".
Kept below only for the reasoning, which generalises: content is the
replay-safe place to put anything that changes what a join produces.

**Was: a starter kit, for testing** (operator: *"we might have to give players
starter items for a bit for testing LOL"*). Real need — a fresh character
spawns empty, so the wheel reads `350 Wood (0)` and the build flow cannot be
exercised at all. **Not improvised**, because the two obvious routes both
cross a wall: a new grant command is a wire change (wall 6, version bump +
goldens), and a `shard.toml` flag that changes what `Join` seats would make a
WAL replay diverge from the run that wrote it (wall 5) unless the flag is in
the header. The route that is safe by construction is **content**: a spawn
kit in `content/*.toml`, because the content hash is already pinned into the
WAL header, so a replay replays the kit it was played under. That is a
schema + validation + `world.rs` slice, not a patch.


Landed 2026-08-07. Nothing owned the typeface: all 42 `TextFont` sites were
`..default()`, so every screen drew in Bevy's embedded debug mono. The face is
**Roboto Condensed**, measured off the reference's own public source rather
than a screenshot (`Facepunch/Rust.Community` defaults its UI text to
`RobotoCondensed-Bold.ttf`), embedded with `include_bytes!` because an
unresolved `Handle<Font>` draws *nothing* and `OnEnter(Loading)` runs before
`Startup`. Bold is the default weight and regular is prose — 31 sites against
12. `DECISIONS.md` "ui type v0"; gate `tests/ui.rs` §F, a call-site grep.

Verified by picture, not by compile: six vantages under Xvfb + lavapipe, zero
panics, and — for the first time — **the panels too**, by driving a live
client with `xdotool` (Tab, then hold B) and grabbing the root window. That is
by hand and reproducible by nobody, which is item 1.

Remaining, in order. **Item 1 blocks items 3 and 4:**

1. **Nothing in this repo can photograph a panel.** `render/panels/` is not
   registered on a `--capture` run, so inventory, crafting and the wheel —
   ~1,400 lines and the screens a player spends the most time in — are seen
   only by a human with a shard up. Wanted: a **viewer, not a gate** — a mode
   that opens each panel against a stocked fixture and writes a PNG per
   screen. The visual-gate rule is retired and stays retired (`CLAUDE.md`);
   this asserts nothing. `panels/mod.rs`'s refusal ("a gate whose frames
   depend on a keystroke") was right about a *gate* and does not bind a
   viewer.
2. **DONE 2026-08-07 — icons, and the wheel rebuilt around them.** The wheel
   was *"worse than even avg programmer art"* (operator) and the diagnosis was
   structural, not colour: `building.jpeg` is a **cream annulus cut into
   wedges** with a line-art glyph in each and the world showing through the
   middle, and ours was a flat dark disc with rounded label boxes floating on
   it. The annulus is now a baked texture (`render/panels/ring.rs`, ten images
   at plugin-build time, geometry from the same `Rings` that `pick` resolves
   with) and every cell and wedge carries an icon from **game-icons.net**
   (CC BY 3.0, `ci/bake_icons.py`, 57 PNGs). Gate `tests/ui.rs` §G.
   *Left behind:* two items share a glyph where the set had no second
   candidate, and the wheel is still **two** rings where the reference has one
   — see the hammer/blueprint note above, which is the same finding.

   ~~**ICONS — the single largest gap, and the only one blocked on art.**~~
   Every cell in `crafting.png` is item artwork; every cell of ours is a
   clipped word. The reference wheel is a cream ring of red line-art glyphs;
   ours is a dark disc of text boxes. Two halves with very different costs:
   the **six building shapes** are isometric solids and are generatable the
   way `tree::needle_image` and `sky.rs` already generate one, so that half
   needs no art at all; the **48 item icons** are real content and cannot be
   traced from the reference (the IP rail). `ART.md` §7 permits real assets.
   **This is the operator question**, and it gates how the panels ever stop
   reading as a spreadsheet.
3. **Two defects the first look found, both cheap.** An item name overflows
   its 44 px cell — `Gunpowde`, `Workbenc`, `Metal Fragments` bleeding over
   its border — so the browser is least readable exactly where content grew;
   it wants clipping plus an abbreviation, not a bigger cell (the 720p budget
   is why). And the wheel's hint line is drawn at `bottom: 40px`, straight
   through the hotbar behind it.
4. **Twelve sizes is not a scale.** Collapsing to five is a real improvement
   and may not be done blind: the numbers were budgeted against 720p and the
   first cut clipped a column at both ends.
5. **Surveyed and refused: `bevy_hui`, `bevy_lunex`, `bevy_feathers`.**
   `bevy_hui` is the closest to the XML-and-hooks shape the operator asked
   about, and taking it would move ~5,400 lines of screen description out of
   Rust and into a plugin that spawns entities from data — the same reason
   `bevy_procedural_tree`'s own plugin is deliberately unused. The iteration
   win it is wanted for is item 1's, and item 1 costs a fraction as much.

## 0y · Persistence takes the reference game's shape *(server lane)*

> **ARMED on the public shard, 2026-08-07** (operator: *"ok yea turn it all
> on please"*, then *"SIWE is what we are using now… rework this so its
> normal"*). `shard-public.toml` carries `save_file`, `world_file`,
> `require_auth = true` and `domain`.
>
> **Identity is real now**, which is what the earlier version of this note was
> waiting for. `auth.rs` is no longer a stub: a joiner signs a nonce this
> shard chose and the shard recovers the signer, so the player key is a
> verified Ethereum address — **stable across sessions**, which the session
> token was not. That second half is the one that mattered: a rotating key
> would have handed every returning player a new character with every gate
> green. Wire v27; the boot-time `AUTH WARNING` is deleted rather than
> reworded. `DECISIONS.md` 2026-08-07 has both calls and what they do not
> authorize (the deploy itself).

Player half landed 2026-08-07; then we read how the reference game does it and
**there is no player save file there** — the body stays in the world as a
sleeper, saved because it is an entity. Operator adopted that model; backup
rotation landed with the call. Plan, sources and reasoning: `reference/SAVES.md`
§9. Knobs: `DECISIONS.md` §open "player persistence v0". In order:

1. ~~**Sleepers: the body stays.**~~ **DONE 2026-08-07.** `Leave` sleeps the
   body instead of clearing `active`; it stands, takes no input, keeps its
   metabolism, and is killable through the ordinary `die` (the bag drops).
   `Command::Wake { id, sleeper }` is the return — two ids because the sim
   cannot recognise anybody, so `ShardCore` holds the key→id arrow outside it.
   Wire v26 (one bit beside `grounded`, both encoders). Eviction policy:
   longest-asleep, `World::evictions`. Gates: `tests/sleepers.rs` (10),
   `client_loop.rs` end-to-end. **Left open:** a sleeper does *not* block
   movement — players never collided, so this changed nothing and the question
   is still unanswered rather than decided. Lootable-alive is still item 1 of
   whatever comes after; Devblog 7 shipped it after standing too.
2. ~~**The world is persisted.**~~ **DONE 2026-08-07.** `sim-core/worldsave.rs`
   is the codec (bodies, pieces, deployables + bag cooldowns, hearth stock, box
   contents, ground bags, fuses, harvested slots, sweeps, tick);
   `server/worldfile.rs` is the file — temp-then-rename, seed + content-hash
   pinned, backup rotation, and the **identity table** that makes a saved
   sleeper claimable (an id is per-connection, so bodies alone are unclaimable).
   Wall 5 holds because the load is *construction*, before tick 0: same build +
   same **origin** + same stream → same hashes, gated by
   `two_shards_loading_one_file_stay_in_lockstep`. Knobs: `world_file`,
   `world_save_interval_ticks` (1800). Gates: 12 + 2 in sim-core, 7 in
   `server/tests/world_persist.rs`.
3. **The store's job is now exactly §9.2's, and one hole is left in it.** A
   sleeper's record is still frozen at the moment they left — `disconnect`
   reads it before the `Leave`, and the sweep walks *connection* slots. With
   the world file on this no longer costs a restart (the body itself
   persists), so the window narrowed to one case: **eviction**. A sleeper
   raided and then evicted for slot pressure comes back from the stale record.
   The fix is two-phase eviction — the server picks the victim, takes its
   save, and queues `Command::Evict { id }` *before* the join, instead of
   `seat` evicting on its own authority. Deterministic (the id is in the
   stream) and small; not built.
4. **Blueprints** are the wipe-surviving payload the split was shaped for;
   nothing to build until BPs exist.
5. **Still no WAL, and item 2 answered the question it was going to force.**
   A world load is an *origin*, not a command, so a WAL does not have to carry
   world state — its header pins the origin hash beside the seed and the
   content hash, and replay starts there. `worldsave.rs`'s module header has
   the argument. That is a design, not a file format.
6. ~~**No graceful shutdown.**~~ **DONE 2026-08-07**, and the second half was
   the half that mattered: the flush landed first and **nothing set the flag**,
   so `systemctl stop` still killed the process where it stood and the whole
   path had never once run. `bin/shard.rs` now catches SIGINT/SIGTERM and waits
   on `store_stopped` — raised by the storage thread when its rings are dry and
   *abandoned*, so the wait ends when the last byte is written rather than after
   a duration somebody guessed. Ordering is gated
   (`the_shutdown_flush_takes_the_world_before_it_drops_the_players`).
   **Measured by hand 2026-08-07** (no gate — a signal test is a clock test,
   and `CLAUDE.md` is explicit): cadence saves land (14 worlds, 0 skipped, 0
   failed); SIGTERM prints, flushes, exits; a reboot resumes the tick; and
   **SIGKILL leaves no `.tmp` and the next boot resumes off the last cadence
   save** — which is the crash case, and the one temp-then-rename exists for.
   **Still ungated:** the three-thread path end to end, and `KeySlot`'s id
   match.

## 0x · The client makes sound — what it cannot yet hear *(client lane)*

Landed 2026-08-06. `crates/client/src/sound/` is the model (pure, headless,
**code tier** — 30 assertions in `tests/sound.rs`), `render/audio.rs` is the
Bevy half. 19 cues, two buses, a bounded mixer, per-surface footsteps off
`terrain::splat`, a crossfaded wind bed, and three working volume sliders.
Research is `reference/AUDIO.md`; every number is `DECISIONS.md` §open
"audio v0". **There are no audio assets** — `sound/synth.rs` generates the
bank at boot, which is a licence posture (§ that file), not a preference.

Remaining, in order:

1. **Nothing scores it, because `ART.md` has no audio section at all.** The
   bank has a *gate* (energy, no clipping, no click, loop seam continuous,
   surfaces differ in brightness) and no *bar*. That asymmetry is the same one
   `CLAUDE.md`'s beige-smear entry is about: a statistic cannot tell whether
   the frame is a picture of anything, and none of these can tell whether the
   bank sounds like a forest. **Nobody has heard it** — this box has no audio
   device — so it is honest programmer art until someone plays it.
   `cargo run -p client --bin soundbank -- <dir>` writes all 19 WAVs, which is
   how you listen without launching a client and walking to a tree. Looking
   already paid twice: a waveform plot found the wind bed was a flat hiss (its
   gust LFOs were slower than its own loop) and then that the fix overshot into
   the ambience dropping out. Both are gates now; neither was reachable from a
   statistic that only asked "does it have energy".
2. **Music is the highest-value unbuilt thing and the design is already
   written down.** `reference/AUDIO.md` §8: a 4–8 minute gap timer, themes of
   sectioned pieces, an intensity scalar bumped by events we already have as
   integer codes, transitions only at section boundaries. Every input exists.
   What does not exist is music — a generated bank makes tones, not themes.
   That is a **content** blocker, not an engineering one.
3. **CI now compiles the native client, and that is half of R-G0.** The
   `native client (--features render)` gate landed on main this same day —
   clippy `-D warnings` plus the render-tier suites — so the hole §0v item 3
   named is closed for *compiling*. What still runs only by hand is the other
   half: a `--capture` run against a live shard, which wrote all six vantages
   with **zero panics** and is the only thing that proves the audio systems
   execute at all. It caught this slice's one runtime defect —
   `OnEnter(Loading)` runs *before* `Startup`, so the bank could not be a
   `Startup` system — and it needs Xvfb, lavapipe and a shard, which is why
   it is not in `ci/gates.sh` yet.
4. **DONE, and it went wrong first — `render/feed.rs`.** This item used to
   say a second reader of the core's destructive `pop_*` rings would silently
   split the events, and that the fix was one drain into a resource both read.
   It arrived on the merge: `hud::feedback` (HUD lane) and `audio::feed`
   (this lane) each popped the same six rings, **git merged them with no
   conflict**, and the HUD — scheduled first — ate every event before the
   mixer saw one. No test could see it; each half is correct alone. There is
   now one drain, and `tests/sound.rs` greps `src/render/` for a second
   `pop_*` call site, because the defect is a call site and not a value.
5. **Four of the nineteen cues have no producer**, which is `MENUS.md` §4's
   dark-content defect inside a thing that just shipped: `Place`,
   `ImpactWood`, `ImpactMetal` and `UiClick` are generated, in the table and
   playable, and nothing asks for them. `Place` is the cheap one now that
   `structures.rs` streams piece and deploy changes with cells — it is the
   second positional cue and would exercise that path against something other
   than a falling tree. The two impacts need to know WHAT was hit, which the
   gather toast does not say. `UiClick` needs a hook in the per-screen click
   handlers.
6. **Remote players are silent.** Only the local body has an odometer, so
   another player's footsteps — the sound that decides fights — do not exist.
   `bodies.rs` has their interpolated transforms; a `Steps` per remote body
   and a positional step cue is the slice.
7. **No occlusion, and it needs a prerequisite rather than a pass.** A wall
   between you and a sound needs a geometry query, and the correct one is the
   sim's (`collide.rs`), not a raycast against render meshes.
## 0z · The world waits for the server now — what the Bevy audit left *(client lane)*

Landed 2026-08-06. The client was building a world the server had not named —
the welcome carries a seed and **no position**, and an unplaced `Predictor`
reports the world **origin**, which is a real place here rather than a
sentinel, so the rings streamed it. Measured on seed 20260731: the shard
places at `1001.6, 1935.3` — **2,179 m from the origin on a 2,048 m island**,
the whole diagonal. Every connect wasted the first frames' chunks; the severe
case (bar full at the origin, `InWorld` around an unplaced player) is a race
the first snapshot normally wins. `RENDER.md` §1.1 is the rule,
`DECISIONS.md` §open the mechanism, `tests/ui.rs` §E the **code**-tier gate.

Also landed: `--features hot` (asset hot-reload — `bevy_asset`'s watcher, not
`bevy_scene`), and the claim that `bevy_ui` is dead weight is corrected in
both places that carried it — it is ~5,400 lines and every screen we have.

Remaining, in order:

1. **Trim Bevy's default features — no longer only a payload win.** The gate
   at `ci/gates.sh` already names the reason: `alsa-sys` is pulled by
   `bevy_audio`, which has **zero call sites**, and a box without the dev
   package fails at a build script. `3d` pulls `audio` + `scene` as an
   umbrella, so this means `default-features = false` and an enumeration —
   cheap, but it needs a verified build, not a guess. Keep `bevy_ui`,
   `bevy_picking`, `jpeg`. (`libudev-dev` and the runtime `libxkbcommon-x11-0`
   are NOT this item: gamepads and X11 are both wanted. `CLAUDE.md`
   §environment lists all four.)
2. **R-G4 is still the missing half of §1.** Placement has a gate now; the
   no-gameplay-state rule still has none. Its answer is the
   renderer-attached/detached state-hash equality.
3. **Nothing photographs the new wait.** `ci/gates.sh` compiles the render
   path and now runs the client's lib tests under it (`--lib` added here), so
   the placement arithmetic is gated twice. What no gate does is *look*: a
   capture run exercises the wait on every run and `capture::PLACE_FRAMES`
   bounds it, but the native visual gate is still §2's, unbuilt.
## 0y · ~~`web/` is cut — decide what that means to the tree~~ **(ANSWERED, done)**

**Operator, 2026-08-06**: *"we have it all backed up on github… we dont need it
locally."* All three questions answered and executed in the same pass —
`DECISIONS.md` has the row.

1. **The three gates: deleted**, along with five more that imported
   `web/src/props.js` and three that read `materials.js`. Eleven total.
2. **`web/` itself: deleted.** It is in git history on GitHub, which is what
   makes it still readable as the reference implementation of every verb —
   `git show <commit>:web/src/interact.js`.
3. **`client-core`: kept**, and so is `test_parity_wasm`. Two codegen backends
   agreeing is a real determinism check and it is cheap; a missing browser does
   not repeal wall 1.

**What it left owed**, and this is the only live part: eight of the eleven held
"the mesh the client draws == the volume the server blocks" against the browser
renderer. `crates/client/tests/tree.rs` already re-earns that natively for
trees. **Uncovered**: the haven shelter, the waystation canopy, the clutter
ring, and the occupant table for everything that is not a tree. The
replacement's shape is that test file — Rust, against the mesh we draw. Cheap,
and worth doing the next time one of those meshes is touched.

**Do not write a pixel gate.** The visual-gate rule is retired, not owed
(`CLAUDE.md`); `vantages` passed 36 checks on a beige smear. Booting the game
and looking is the visual gate now.

## 0x · The native client can play the game now — what it still owes *(client lane)*

Landed 2026-08-06. Twelve of the wire's sixteen `ACT_*` verbs plus `KIND_CHAT`
had no key in this client; four did. All of them do now, and the three sets
that were decoded into `ClientCore` and drawn by nothing — pieces,
deployables, backpacks — are drawn. Also: **the look and the strafe were both
inverted** (operator-reported; `crates/client/src/look.rs` has the derivation
and `tests/look.rs` checks the client's right-vector against Bevy's own
`Transform` basis).

New screens: `Dead` (dying used to end the session), `Map`. New keys: `E` use/
loot/open, `G` eat, `H` drink, `L` lock, `U` upgrade, `R` repair, `X` plant a
charge, `T`/`Enter` chat, `M` map, RMB place. New HUD: crosshair, centre
prompt, toast, compass, hitmarker.

Remaining, in order:

1. ~~`ci/gates.sh` never builds `--features render`~~ **DONE** — the
   `native client (--features render)` gate is in `ci/gates.sh` and green:
   clippy `-D warnings` plus `--lib --test tree --test fell --test look`.
2. ~~Nothing in this repo looks at a frame~~ **CLOSED BY DECISION, not by
   work** (operator, 2026-08-06). The eleven browser gates are deleted and the
   visual-gate rule is **retired**: `vantages` passed all 36 checks on a beige
   smear, so the automated version did not work, and booting the game and
   looking is cheaper and cannot be fooled by a wash. **Do not write a
   replacement pixel gate.** What is still worth gating about a frame is
   arithmetic — the mesh fits the volume the sim blocks — and its shape is
   `crates/client/tests/tree.rs`. Uncovered by the deletion and worth a cheap
   native test when one of these is next touched: the haven shelter, the
   waystation canopy, the clutter ring, and the occupant table for everything
   that is not a tree.
3. **No swing prompt.** `interact.js`'s second resolver (`resolveSwing`, a 2 m
   cone with a vertical window and a point-blank exception over the scatter
   cells) has no native port, so the crosshair names what `E` would do and
   never what a swing would hit.
4. **The Bevy feature trim — and this item was WRONG, which is the finding.**
   It said wayland and alsa are unused. **`bevy_audio` is load-bearing since
   audio v0**: `render/audio.rs` uses `AudioSource`, `SpatialListener`,
   `PlaybackSettings` and `Volume`, so **alsa is required** and trimming it
   silences every cue while compiling clean. `Cargo.toml`'s own comment
   contradicts itself the same way — "nothing makes a sound" four lines above
   the paragraph explaining why `wav` is mandatory. Both predate the audio
   slice. x11/wayland stay too: it is a windowed game.

   Actually unused, by grep and not by comment: **`bevy_gilrs`** (no `Gamepad`
   reference anywhere — this is the only real system-dep win, `libudev`),
   `bevy_gltf`/`bevy_scene`/`bevy_animation` (every mesh is procedural), and
   **`vorbis`** (we generate WAV in memory; a Vorbis decoder for buffers we
   made ourselves buys nothing).

   **Attempted 2026-08-06 and backed out**, twice-blocked and neither reason
   is the code. (a) A feature change invalidates every Bevy artifact, so
   `target/debug/deps` held two full sets and 32G became 44G on a 49G disk —
   `rust-lld` took SIGBUS mid-link, the same ENOSPC signature as the morning.
   (b) The failure mode is **invisible on this box**: no display, so a green
   compile is not evidence. The precedent is in that same comment block — a
   missing `jpeg` decoder made Bevy draw a white fallback *and keep going*,
   and three material changes measured byte-identical statistics before
   anyone read the log. Do not land this on a compile alone; it wants disk
   headroom and a `--capture` run that someone looks at.
5. **`bodies::stream` allocates a `Vec` per frame** and scans it linearly to
   retire remotes. `structures::stream` does the same job with a generation
   stamp and no allocation; this is the same fix, four lines.
6. **Five read-side signals are still decoded and dropped.** The verb list is
   complete; this is not. `pop_death` is the kill FEED (every death, not your
   own — the death SCREEN reads `core.dead` and the `own_death_*` fields and
   is done); `struct_hit` is the damage number on a wall you are breaking;
   `charge_placed` is the countdown on a charged one; `stock`/`stock_addr` is
   what a hearth is holding; `mark_cell`/`mark8` is the gather weak spot,
   which is the reference's own `OnDispenserBonus` and the closest thing this
   game has to a skill expression. Each is a small HUD slice on top of what
   this branch built, and none of them is blocked.

## 0s · The six unlanded loop branches — triaged, and five are dead

Checked 2026-08-06 against `origin/main`. **9 commits exist only on this box**
(nothing here is on GitHub). Verdicts:

- **`loop/ranged-v0` — TAKE.** 1,969 lines, **zero `web/` files**: `ranged.rs`
  (402), `pitch_lut.rs` (285), `tests/shoot.rs` (695), `gen_pitch_lut.py`, plus
  `combat.rs`/`limits.rs`/`occupy.rs`/`world.rs`/content bake. Main has **no
  ranged code at all**, while `content/weapons.toml` already ships `item.bow`
  and `loot.toml` ships wood and metal arrows — content with nothing to use it.
  `DECISIONS.md` 2026-08-05 says it is "explicitly still wanted". Test-merged:
  **10 conflict hunks over 4 files**, and all four big new files land clean.
- **`loop/container-contents`, `loop/container-contents-wire` — DROP.**
  Superseded. They bump the wire v18→v19; main is at **`PROTO_VER 23`** and
  already ships `ACT_CONTAINER`, `SUB_CONT_SYNC`, the `core.rs` container view
  and a `container_wire.rs` **larger** than either branch's.
- **`loop/cont-max-mirror` — DROP.** Touches only `ci/ui_smoke.mjs` and
  `web/src/invmove.js`. Both deleted.
- **`loop/m1-surface-grain` — DROP.** `web/` only, and its own
  `BRANCH-NOTES.md` says it is red on purpose and marks its rewrite
  "do not re-land".
- **`loop/bark-photo` — DROP the branch, KEEP the finding.** Its code is
  `web/src/materials.js` + `textures.js`. The finding survives it:
  **`assets/textures/bark_{albedo,normal,rough}.jpg` are on disk and unused**,
  and `render/tree.rs` builds a bark mesh it shades procedurally. Sampling
  three maps that already ship is a small native slice.

If `ranged-v0` is not taken, these branches can be deleted — but they are
**local-only**, so deleting is final.

## 0w · The native menus landed — the four things they cannot do *(client lane)*

Landed 2026-08-06: `Tab` opens inventory + crafting, `B` holds the radial build
wheel, and drag/drop moves items. The arithmetic is `crates/client/src/ui/`
(pure, headless) and `render/panels/` only draws it — 23 assertions in
`crates/client/tests/ui.rs` run in the **code** tier. Verified against a live
shard under Xvfb + lavapipe: rail counts, filter, search box, detail pane with
the AMOUNT/ITEM TYPE/TOTAL/HAVE table, stepper, and the wheel's two rings.

Remaining, in order:

1. ~~No build ghost~~ **— landed 2026-08-06** (§0x). `ui/place.rs` aims it,
   `render/ghost.rs` draws it, and the local verdict answers the four
   refusals a client can check in the server's own words.
2. **The rail is not the reference's, and one wire field would fix it.**
   `EventMsg::Catalog` ships display names only, so a category rail by item
   class is not computable client-side. A class byte per item, a `PROTO_VER`
   bump and regenerated goldens in the same commit (wall 6) buys the frame's
   real rail. Today's buckets are honest but they are not that.
3. **No gate photographs any of it, and none compiles it either.** The
   panels are deliberately not registered on a `--capture` run — a probe
   harness that could open one is a gate whose frames depend on a keystroke.
   That is §0v item 3's hole seen from the other side: `ci/gates.sh` never
   builds `--features render`, so these ~1,400 lines are covered by
   `tests/ui.rs`'s arithmetic and by nothing else. Both native probes were run
   by hand for this slice and both are green.
4. **The drag is gated as arithmetic, not as a gesture.** A fresh shard gives
   a player nothing to drag, and blind swinging under Xvfb did not land a
   gather node, so press → ghost → release → send is verified by inspection
   only. A dev kit (or a shard fixture with a stocked inventory) is what
   would close it.

## 0v · The menu flow landed — what it still cannot show *(client lane)*

**Extended 2026-08-06 (client lane):** `Loading`, `Paused` and `Settings` are
states now. The welcome no longer means "in the world" — the ~25 frames of ring
building have a screen with a real bar on it, Esc opens the intro screen from
inside the world, disconnect actually tears the world down (`WorldEntity` +
`render::world_teardown`), and settings has the reference's rail-and-pane shape
with five settings that do something.

Landed (operator, 2026-08-06 — `DECISIONS.md`): the native client opens on a
server-select screen instead of connecting before the window exists. A
`Screen` state machine, `WorldId` built when the welcome names the seed, and
**a failed connect returns to the menu with the reason** instead of `exit(1)`.
`--capture` and `--server` skip the server list (they have already chosen), so
the visual gate and the launcher's join path are untouched.
`ci/shardlist.py` writes `scry-shardlist-v1` and is gated; scry's launcher now
bounds and validates that document instead of rendering it raw.

Remaining, in order:

1. **Nothing is published, so every list is the Direct row.** The url is
   `DECISIONS.md` §open and an operator act: serve `servers.json`, set
   `servers.url` in scry's `data/launcher/gates.manifest.json`. Until then both
   the menu and the launcher's Servers window are correctly dark.
2. **No player counts, and this is the honest half.** `players`/`ping_ms` are
   omitted, never zeroed — the shard serves no status endpoint. `stats.rs`
   already holds `joins`/`leaves` as atomics and its header names the status
   page as what would read them. That endpoint is the whole slice.
3. **`ci/gates.sh` never builds `--features render`, so nothing in CI compiles
   the native client at all** — the code tier's `clippy --workspace` and
   `cargo test --workspace` both run with the feature off (it is off by default
   for the ~20 s clippy that would otherwise pull Bevy), and the renderer tier
   is `web/`-only. `RENDER.md` R0 names the probe that would close it
   (`clippy -p client --features render --all-targets -D warnings` plus a
   `--capture` run); both were run by hand for this slice and both are green.
   Until it is wired, the four menu screens are covered only by unit tests and
   nothing photographs one — and a menu vantage is now cheaper than it was,
   because the capture harness passes THROUGH `Loading`.
4. **Settings are forgotten on exit.** `settings.rs` holds five working
   settings and writes none of them: a config path, a format and a version for
   when a knob is renamed is its own slice, and the footer says so rather than
   pretending. `DECISIONS.md` §open "settings v0" is the row.
5. **No `Screen::Disconnected`.** A shard that drops the session mid-play still
   leaves the client sitting in a dead world — `pause::Disconnect` is a verb
   the *player* takes, and the involuntary half has no state. `world_teardown`
   is now the piece that makes it cheap to add.

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
array in one struct (`terrain.rs:799`); nothing in `client-core/src/bridge.rs`
exposes it, so a client learns a destination exists only by standing in its
chunk. `map.js`'s `resolveMarks` takes world positions and is already gated, so
this is a caller change on the ui side and not a rewrite. Ranked gap 1 of
`pass-20260805-111501-04` is the reason; the container verb is the other half.

## 0a · ~~world lane: skirt residual — the ring's hard edge~~ **(MOOT)**

**Retired 2026-08-06 by the browser cut** (`DECISIONS.md`). Every line below
is about `web/src/clutter.js` — a per-frame player-relative shader term, the
WebGL program-link budget, and the prewarm gate that counts links after
`inWorld`. None of those exist natively: Bevy specializes pipelines, not GL
programs, and the native clutter ring is `render/clutter.rs`. **The finding at
the bottom survives and is worth keeping** — beach skirts are thin because
`scatter` puts 0.22 prop centres a tile on the coast against 0.95 inland, not
because the skirt thins itself, and that is the scatter table's business on
either client. The rest is history. Kept unpruned this pass rather than
deleted, because whoever builds the native ring's fade should read what the
browser learned about it first.

<details><summary>the original item</summary>

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

</details>

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
while `client-core` reads it off wasm and the server off native. Its other
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


## 5d · The agent player has a spec and no code *(systems lane)*

`PLAYERS.md` landed 2026-08-05 — the verb set, the observation encoder, and
four walls with their gates. Nothing under it is built. `sim-core/bots.rs`
already drives deterministic synthetic input, so the missing piece is the
intent layer above it, not a new client.

Smallest useful slice, and it is not the API: **log the condition.** Every
trust-bearing verb (door, TC authorize, container access, give) gains an
event carrying whether the counterparty was online, landing inside
`tests/event_roles.rs` with two causes per code in the same commit (§4's
discipline). That field is the whole measurement `SUBSTRATE.md` §3 turns on,
it is ordinary game state a human client already sees, and retrofitting it
makes every shard-hour logged before it worthless. It is also independently
useful: offline-raid telemetry is a thing the game wants anyway.

Then the verb table, then an agent client that plays badly. Wall 1 (agent
verbs ⊆ human verbs) wants its gate in the same commit as the table, not
after — it is a subset assertion over two lists and cheap while both are
small.

Not this lane's call: what an agent pays to enter and what it earns
(`ALPHA.md` + scry side).


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

## 0. The bow fires — done this pass *(systems lane)*

From `findings/pass-20260805-053501-01-judge.md` ranked gap 1, "every fight is a
walk-up club fight — there is no ranged weapon in the sim". `weapons.toml` had
carried the bow, the crossbow, their ammo and their ballistics through
validation and the content hash since the content crate was written, and
`bake_combat` threw every row away at `kind != Melee`. Now `bake_bow` converts
m/s to **mm/tick** once at boot and `ranged.rs` flies the arrow in integers.
Design and the six knobs: `DECISIONS.md` §open, ranged v0. `frame.pitch` is read
by the sim for the first time, through a generated `pitch_lut.rs`; the byte was
already on the wire, so the wire did not move and `PROTO_VER` did not bump.
Gated by `tests/shoot.rs` (14 tests, 4 seeds) and two `content.rs` bake gates;
three mechanisms mutation-checked to go red on their own break.
`GOLDEN_FINAL_HASH` is **unmoved**, which is the claim: the arrow store hashes
on the player idiom (skip-if-inactive, no length prefix), so it is invisible to
the replay until somebody fires.

Left:

- **Nobody can see the arrow.** No wire event, so no client can draw a tracer —
  a shot arrives as `EV_HIT`/`EV_HEALTH`/`EV_DEATH` and nothing else. The wire
  half (an `EV_SHOT` code, its subtype, a `PROTO_VER` bump and 66 regenerated
  goldens) is **systems lane**; the tracer itself is the client lane's.
- **An arrow stops on a wall instead of chipping it**, and `collide::blocked`
  bakes `CAPSULE_RADIUS_M` into its own query, so an arrow is as fat as a body:
  it threads a doorway and never an arrow slit. A radius parameter on `collide`
  is the fix, and it also wants structure damage. Systems lane.
- **The revolver still cannot fire.** Hitscan wants M2's rewound raycast, so
  `bake_combat` drops firearm and throwable rows deliberately, not by omission.

## 0. The island is solid now — done this pass *(systems lane)*

From `findings/archive-prestamp/pass-20260805-020919-02-judge.md` ranked gap 1,
"nothing on this island is solid". `occupy.rs` is the consumer half of the seam
`terrain.rs` drew: `movement::step` now asks it on every candidate move, so
trees, boulders, nodes, barrels, crates and the shelter's walls stop a body.
Answers the world lane's standing request ("the occupant query exists, please
call it") and the systems bullet of "nothing in the world is solid". Cache
design and the reasoning: `DECISIONS.md` §open, occupant collision v0.
Gated by `tests/walk.rs` (7 tests, 4 seeds); each of the four mechanisms was
mutation-checked to go red on its own break. Wire did not move — no
`PROTO_VER` bump, `test_protocol_golden` untouched. `GOLDEN_FINAL_HASH`
regenerated in the same commit; every behavioural floor in `replay.rs::run`
still passes, so the bots' script survives a solid world.

Left:

- **You still cannot stand ON anything.** The ground query is the other half of
  the same seam and it is not built — `terrain.rs:1197` names the cost: the
  shelter's plinth reads as a 0.2 m kerb you sink into rather than a step.
  Crate and boulder tops are the same. It belongs beside `collide::piece_ground`
  and wants a `slot_ground` next to `slot_blocks`; the fourteen-box table is
  already there for it. Systems lane.
- **The client draws no collision it can see.** Nothing here changed `web/`, so
  a player learns a trunk is solid by bumping it. Not a defect, just untested by
  eye — no vantage was captured this pass. *(ui/looks lane, if it wants it.)*
- **`SlotLives::find` is a linear scan** over up to 16,384 entries, and the
  query calls it once per blocking slot. Bounded and off the common path (it is
  asked only after something already blocks), but a late-wipe world with
  thousands of harvested cells pays it in the tick. Measure before fixing.

## 0. E tells you what it does — done this pass *(ui lane)*, kept for what it leaves

From `findings/pass-20260805-002720-04-judge.md` ranked gap 3: "the island
never tells a player what it offers", E five verbs deep in a blind fallthrough
chain, "does something you did not choose, silently".

Landed: `web/src/interact.js`, the one resolver. Two ranks — **aimed** (in
front, within `INTERACT_AIM_RADIUS_M` of the aim line; nearest of those wins,
as a raycast would) always beats **nearby** (everything else in reach, nearest
wins, which is the old chain's behaviour kept so no verb that worked stops
working). The five scans in `main.js` are gone; E dispatches on the pick, L's
lock takes the same pick filtered to `VERB_DOOR`, and `#prompt` under the
crosshair draws `promptFor` of that same pick off the HUD timer. Archetypes and
reach are the sim's (`deploy.rs`, `build.rs`), not restated literals.
`ui_smoke` §Q: 381 checks total, 28 mutants run, all 28 red.

Leaves open:
- **The aim radius is a proposed default**, `DECISIONS.md` §open (interact aim
  radius v0). 1.0 m from the deployable's own scale; the operator has not
  spoken a precision.
- **Nothing highlights the picked thing in the world.** The prompt names it;
  the box itself does not light up. That is a looks-lane material change, not
  ours — a NOW.md line, not a cross-lane request, until someone wants it.
- ~~**Gathering and building have no prompt**, only deployables do.~~ Gathering
  closed 2026-08-05 (`## 0. The crosshair answers for the swing too` below).
  **Building still has none** — placement is a preview mesh with no text, and
  it is the last verb with nothing under the crosshair.
- **Unverified in a real browser end to end.** `ui_smoke` drives the DOM and
  the resolver; `browser_smoke` is off this run, so nothing here claims a box
  was opened by aiming at one on a live shard.

0. **The collapse path's per-tick budget, and box handle 0 — done this pass
   (systems lane).**

   *(2026-08-05. Judge `pass-20260805-020919-01`'s ranked fixes 1 and 2, plus
   the ui lane's cross-lane request 4 — NOW.md's top systems item.)*

   `MAX_COLLAPSE_PIECES` bounds one cascade and a tick holds many:
   `upkeep_sweep` never returned after a removal the way `support_sweep`
   does, so its 64 visits could each seed a cascade, and raiders add up to
   `MAX_PLAYERS` more. Measured with the budget removed: **103 removals in
   one tick** against the 64 the 256-slot ring was sized for. Now one
   tick-local `MAX_REMOVALS_PER_TICK` threaded through raid, decay, support
   sweep and cascade; overflow **defers before the piece leaves the store**.
   `DECISIONS.md` §open "collapse budget v0".

   Request 4 answered without moving a bit of the packing: `box_index`
   guards handle 0 and `place_deploy` refuses the one address that mints it
   — the pair `Backpacks` already has. `BOX_KEY_LAYOUT` and `ui_smoke` §P
   untouched. **ui lane: `moveArgs` may now trust that 0 is never a box.**

   Left: `EV_PIECE_REMOVED`'s payload is pinned for the collapse producer
   only (judge fix 2); the raid and decay producers are still counted, not
   read. And judge fix 3 — **a box on a collapsing floor** — is still
   ungated: every collapse test uses `DeployContent::EMPTY`, so
   `EV_DEPLOY_REMOVED` and the `world.rs` spill drain have no collapse-side
   coverage. Both are systems lane.

0. **A box can be opened — done this pass (ui lane), kept for what it leaves.**

   *(2026-08-05. Report 03's ranked gap 2, "you can deploy a box and never
   open it — there is no storage, only looting.")*

   Its stated blocker was already gone: `ARCH_BOX`'s slots and container
   address (cross-lane request 3 below) landed in the systems lane's
   `4d7a926` on 2026-08-04, and the ui comment saying they were awaited was
   written a day AFTER them. `deployRecs` has held `cx`/`cz`/`level` as
   integers all along. So the whole job was the one thing the request
   sequenced last — a gate on the packing — plus `tryOpenBox` on E.

   `invmove.boxKey` mirrors `deploy.rs:316`, layout stated as data
   (`BOX_KEY_LAYOUT`) and read back out of `deploy.rs` by `ui_smoke` §P.
   Why it needed a gate at all: `box_key` packs `cx<<16 | cz<<4`, and every
   other packing this client touches is `cx<<16 | cz`. The habitual form is
   wrong here by twelve bits — not a crash, a handle naming a real box in
   another cell — and no wall in the repo can see it. 313 checks, 12 mutants
   red, including the three that only a structural check catches: the
   packing restated at the call site with the value CORRECT, a fourth term
   added in `deploy.rs` with the value IDENTICAL, and the build grid
   outgrowing the packing with both sides agreeing.

   Three things it leaves:
   - **A box at grid origin (0,0,0) can be opened and not moved into.**
     `box_key(0,0,0) == 0` and `moveArgs` refuses a ground handle of 0,
     correctly — `deploy.rs:424`'s `box_index` has no zero guard, so 0 is a
     real address. Request 4 below. Biasing it client-side was refused: a
     JS-only fudge inside an address Rust decodes is worse than the corner.
   - **Cross-container drag into a box still waits on request 2** (the TO
     kind in `client_move_readout`). E opens the panel; the drag between it
     and the inventory is what that request unblocks.
   - **Nothing here is claimed to boot.** `browser_smoke` and `vantages` are
     UNRUN (operator's `GATES_TIER=fast`) and this pass edits `main.js`.

> **Cross-lane request → systems (ui lane, 2026-08-05). Request 4: give
> `deploy.rs:424`'s `box_index` a zero guard, or bias `box_key` so no valid
> address packs to 0.** `backpack.rs:333` already guards zero and
> `box_index` does not, so the two disagree and neither side can rely on
> either. Until then a box at cell (0,0) level 0 is openable but not a legal
> move destination, because `moveArgs` must refuse handle 0 to avoid sending
> "no container known" as a real address.
0. **A base comes down when you take its legs out — landed this pass
   (systems lane).**
   *(Gap pass. From `findings/archive-prestamp/pass-20260805-011412-01-judge.md`,
   ranked gap 3: "`supported()` runs at placement and nowhere else".)*

   `build::collapse_from` re-checks support around each removed address and
   drops what no longer stands, breadth-first over `dependents()` — the exact
   inverse of `supported()`, sitting beside it, gated by removing **every**
   piece of six fixtures in turn and requiring the same world a naive fixed
   point reaches. Wired into both death paths (a raid swing and the decay
   sweep). Capped at `MAX_COLLAPSE_PIECES = 64` a tick, derived from the
   256-slot event ring, with `support_sweep` finishing the remainder on later
   ticks so the cap costs latency and not correctness. Replay golden
   regenerated — checked, not assumed: with the new hash field held out and
   the sweep disabled the world still differs, so the cascade is really
   changing what the script replays.

   What it does **not** do, ranked:
   - A collapsed piece pays nothing back. `drop_piece` drops no loot and
     refunds no material, so a raid's whole reward is still what was in the
     containers it broke. Whether rubble should pay is unspoken — a
     `DECISIONS.md` §open row, not a number to invent. systems lane.
   - A large collapse arrives as a burst of `EV_PIECE_REMOVED` in one tick.
     The wire carries it and nobody has watched a client take one. ui lane.
   - The rest of that judge gap is untouched: `combat.rs` is still melee-only
     (its own item below), and what a raid is *for* is the container panel
     (ui lane).

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

1. **The container panel is built; two things in `crates/` bound what it can
   say.** *(ui lane, 2026-08-05. The panel, the cross-container drag, the
   open key and `ci/ui_smoke.mjs` group O landed — 279 checks, 8 mutants red.)*

   - **`client_move_readout` still has no TO kind** (request 2 below, 8 spare
     bits). So `hud.invMoveVerdict` matches a verdict on three carried fields
     plus the one-move-in-flight rule, and a self-3-to-box-3 verdict is the
     same word as a self-3-to-self-3 one. `hud.abandonContainerMove` is what
     keeps that honest — the moment the open container changes, a move with an
     end in it is given up rather than matched against whatever is open now.
     Not a hole; a narrower match than it looks, and gated as such.
   - ~~**Only BAGS can be opened.**~~ Closed 2026-08-05 — item 0 above.
   - **Nothing here is claimed to boot.** `browser_smoke` and `vantages` are
     UNRUN (operator's `GATES_TIER=fast`) and this pass edits `main.js`,
     `wasm.js` and `index.html`.

## 0. The crosshair answers for the swing too — done this pass *(ui lane)*

*2026-08-05. The top item's own remainder ("gathering and building have no
prompt"), plus ranked fix 1 of `findings/pass-20260805-002720-05-judge.md`.*

`interact.js` gains a SECOND resolver, `resolveSwing` — not four more verbs in
the first one, because the two picks share no term: E reaches 5 m and ranks an
aim radius, a swing reaches `gather::REACH_M` (2 m) through a 30° cone with a
±3 m window and a point-blank bypass, over a 3×3 block of 8 m terrain cells.
It transcribes `gather.rs:494-532` and invents nothing: the client sends a
swing as a button bit and the sim picks the node alone, so any other rule names
a node the arm does not hit. Nodes come from `terrain.cellEntry`, already
public. E's pick still wins the line; the swing prompt fills the silence.
`ui_smoke` §R: 433 checks (was 381), 20 mutants run, all 20 red.

The judge's ranked fix 1 is closed in the same file: `promptFor` was only ever
called with `{open:false, locked:false}`, so flattening the door's whole
open/locked branch shipped green (its mutant M14). Now walked at all three
states.

Leaves open:
- **Which prompt outranks which is unspoken** — `DECISIONS.md` §open ("swing
  prompt precedence v0"). E-first is argued, not decided.
- **The tie test found a hole in its own first version.** A tie between cells
  (0,0) and (2,2) is kept by (0,0) under BOTH loop nestings, so transposing the
  loops escaped green; cells (2,0)/(0,2) swap under the transpose and catch it.
- **Nothing here is claimed to boot.** `browser_smoke` and `vantages` are UNRUN
  (operator's `GATES_TIER=fast`) and this pass edits `main.js`.
- **Building still has no prompt** — the last verb without one.

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
> 1. ~~**Container contents on the wire.**~~ **Answered at wire v19** —
>    `EventMsg::ContSync` carries (kind, handle, slots) to the opener alone.
>    Item 1 above has the four bridge exports it arrives through.
> 2. **`client_move_readout` must carry the TO kind.** It packs
>    `reason<<24 | to_slot<<16 | from_kind<<8 | from_slot` — 8 bits spare.
>    `invmove.moveVerdict` therefore rejects every non-self FROM kind, which
>    is correct and load-bearing today: without the to-kind a bag verdict
>    cannot be told apart from a self one. That rejection is the last thing
>    between the panel and cross-container drags.
> 3. **`ARCH_BOX` needs slots and a container address** (`deploy.rs:80`), the
>    piece the judge named. (2) unblocks the panel; (3) gives it something
>    worth opening.
> **Cross-lane request, systems lane: nothing in the world is solid.**
> `collide.rs` knows only built pieces — `blocked`/`piece_ground` take a
> `ColIndex` and never a `Slot`. So a player walks through every tree, boulder,
> barrel and now through the haven shelter's walls. The world lane can place a
> building but cannot make you stop at it. One entry point would do it: a
> `terrain`-side occupant query the movement path can call, owned by systems
> because `movement.rs` and `collide.rs` are theirs.

## world: the pad has a building on it, and it is hollow in two senses

From `findings/pass-20260805-002720-01-judge.md` ranked gap 3 — the authored
clearing stopped reading as authored once scatter clumping made empty forest
windows ordinary, and the gap's own fix was "the greybox, gated as arithmetic".
Landed: `Occupant::HavenShelter = 10`, one slot carrying a fourteen-box
structure (6.2 m room, 2.4 x 2.8 m doorway, tower to 9.2 m over a 6.6 m pine),
placed `HAVEN_SHELTER_R_M` off the pad center by a bounded search against
`road_band` — the pad's center is the carriageway, and `tests/road.rs` caught
the first draft blocking the loop. Gated by `tests/haven.rs` (9 tests) and
`ci/haven_shelter.mjs` (40 checks, doorway passability asserted in both
directions). `DECISIONS.md` §open "haven shelter v0" has the rest.

What remains, in the order it is worth doing:

- **This lane's half is now COMPLETE, including the shelter; the wiring is
  not.** The box list landed (`DECISIONS.md` §open "shelter volume v0"):
  `terrain::SHELTER_BOXES`, `slot_blocks` routing the shelter to a narrow
  phase, `tests/solid.rs` at 14 tests, `ci/haven_shelter.mjs` at 156 checks
  holding all 14 × 6 fields equal to `props.js`. **Nothing calls any of it** —
  see the request below. Nothing further is owed here from this side.
- **Nobody has looked at it.** No frame has been captured since it landed; the
  claim "a player can tell they arrived" rests on arithmetic alone. The lane
  charter says to say so when the item flips from "is there a world here" to
  "does it look right" — placing a building is the flip. **This lane wants
  frames again.**
- **Interior is empty.** The five containers stand outside the walls, on the
  ring they were already on. Putting some inside is a `haven_crate` change and
  would move a measured prize ratio; it needs its own pass.
- The pad is still not carved (3.76 m of relief under a flat-based building —
  the plinth buries 1.4 m of that, which is a cover, not a fix).

## world: the trunk radius is pinned to a builder that no longer ships

From `findings/pass-20260805-002720-03-judge.md` ranked fix 4, deliberately
left by the pass that did the other three. `OCCUPANT_R_M[Tree]` = 0.26 is read
off `props.js:348`'s `CylinderGeometry(0.13, 0.26, …)`, but the near-ring pine
now ships from `ez-tree` (`props.js:556`) and the cone is only the LOD1 start.
So the server's trunk and the drawn trunk agree by assumption, not by gate —
the same class as the shelter's box list before this pass, one occupant over.
`ci/pine_shape.mjs` already imports the shipped builder and already prints a
1.52 m canopy radius, so the fix is one assertion pinning the GENERATED
trunk's radius to `OCCUPANT_R_M[Tree]`. Not done here because it may not hold:
if ez-tree's trunk is not 0.26 m the fix is a table change with a real number
behind it, not a one-line assert, and that deserves its own measurement.
> 3. ~~**`ARCH_BOX` needs slots and a container address**~~ — **answered in
>    `4d7a926`, 2026-08-04**, before this request was read. `BOX_SLOTS` is on
>    the wire and `box_key` is the address; the ui half landed 2026-08-05
>    (item 0 above). (2) is the one still open.

> **Cross-lane request, systems lane: the occupant query exists, please call
> it.** *(world lane, 2026-08-05.)* `terrain::slot_blocks(&slot, x, z, feet_y,
> capsule_r, capsule_h) -> bool` is pure, allocation-free, sqrt-free and takes
> an ALREADY-RESOLVED slot — never a seed, because `scatter` costs a `height`
> fan plus a `moisture`, a `clump` and a `road_band` per cell and must never be
> re-derived inside a movement step. `terrain::OCCUPANT_PROBE_CELLS` (= 1) is
> the neighbourhood to scan and it is proved complete, not assumed: every slot
> lies inside its own cell, drawn ones by their ±3 m jitter and authored ones
> because `scatter` only returns them for the cell they fall in. **Updated
> 2026-08-05: the widest reach is now 5.845 m, not 2.050 m** — the shelter's
> bounding circle is 4.9498 m and it eats most of the 8 m margin on its own.
> Still complete, still 1, and the const block still proves it, but the
> headroom is gone: the next occupant wider than a boulder breaks the 3×3 and
> the build will say so at the definition.
> `slot_blocks` **did not change signature** and no golden moved. The shelter
> is the one occupant with a narrow phase behind the radius — that is internal,
> the call site is identical for every occupant. **Unverified in play:** no
> body has ever been stopped by one of these — `tests/solid.rs` gates the
> shapes and the predicate, and that is all it can gate from this side.

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

   Both reports call the cure "one constant in `crates/client-core`". **It is
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
> **Cross-lane, not an item: `browser_smoke` is RED on a clean trunk, and it
> is the lighting gap, not a regression.** The renderer tier is switched off
> this run (`GATES_TIER=fast`), so the loop is not seeing it. Run in full on
> 2026-08-05 it fails `TONAL_MAX_P10`: **p10 luma 112 against a ceiling of
> 60** (reference bar 40.5), register `p10 112 · p50 143 · p90 187`.
> **Confirmed pre-existing** — `lane/looks` unmodified fails with the
> identical assertion and the identical number, so the trap list's `git
> stash` check has already been paid. It is the visual judge's own ranked
> "cut ambient fill, restore darkness through AO not exposure", and by
> `CLAUDE.md` tonemap/sky/exposure/fog are **one owner, sequential** — not
> this lane's, and not four parallel passes'.

0. **world: the forest clusters now; the biome edge is still a step.**
   *(`terrain::clump` landed — `DECISIONS.md` §open "scatter clumping v0",
   `crates/sim-core/tests/scatter.rs`. Dispersion 0.98–1.05 → 2.90–3.34
   against a closed-form null, density held. This replaces "The scatter is
   white noise and a forest is not", deleted from further down the file:
   that item specified this change and predicted every fixture it moved,
   correctly. This is the part of it that is left.)*

   - **`SPAWN.md` §9.4's other half is not done.** Density now ramps across
     a biome boundary but *composition* still snaps: `biome()` is a hard
     classifier (`h > 52.0`, `moist > 0.05`), so one cell draws from the
     forest row and its neighbour from the meadow row. The fix is blending
     the two rows over a band, and it is not a local edit — `biome()` is
     also read by the client splat and by spawn selection, so softening it
     is a decision about what `biome()` returns.
   - **One field scales every occupant.** Trees, bushes and rocks clump on
     the same noise. Right for a clearing, wrong for ore: a metal node has
     no reason to care where the trees are. A second channel for the
     mineral rows is the obvious next slice, and it is cheap — the machinery
     is now in `terrain.rs` and the gate generalises.
   - **The forest row peaks at 945 of 1,000 per-mille in a grove.**
     `test_no_biome_row_saturates` holds it, but that is 5.5% of headroom:
     the next pass that raises a weight row or the clump ceiling hits the
     rail, and past it density falls silently. Budget it before spending it.
   - **Nobody has looked at a grove.** The claim is arithmetic only; no
     frame has been captured since the field landed.

1. **world: the pad pays now, but it is still bare ground.**
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
   (`client-core/src/core.rs:122`) and `STREAM_ERR` (`client-core/src/bridge.rs:64`)
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
