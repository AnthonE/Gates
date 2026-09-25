# Gates · NOW.md — what next

The backlog: what a builder or the loop can pick up, and what waits on the
operator. Done work is deleted, not ticked. An item says what is left and where
to start, not how it got here; the story is in git, `DECISIONS.md` and
`findings/`.

Rewritten for MVP mode 2026-09-24 (3,491 → 1,222 lines). The long version
is `git show 9a069f4:NOW.md` (the rewrite's own commit was squashed away). Labels are stable, because ~500 code comments
cite `NOW.md §<label>`; a label that is not here was closed (`§Labels`, at the
end). Item numbers are the original ones, gaps included. A pointer to
"`CLAUDE.md` §traps" or a numbered "wall" means the old manual,
`git show ecb9e21:CLAUDE.md`.

`§LOOK`, in the operator lane, is every question only a person looking at a
frame can settle.

---

# Buildable now — a loop can pick any of these

## 5sp · Spectator seats and signed agents are built — what they still owe *(net lane)*

Wire v73 (2026-09-22): a viewer's own client watches a consenting player
(`?spectate`, `--spectate`), and an agent joins a real shard with a key file
(`crates/agentkey`). `NETCODE.md` §2.3/§2.4 carry the design;
`server/tests/spectate_wire.rs` and `agent_join.rs` are the gates.

- **Operator acts, none done:** the coordinated publish v73 needs (the public
  shard and every published client move together — a v72 client is refused
  `REFUSE_VERSION`); `spectate = true` on any shard that should be watchable;
  a wallet, its entitlement and a `chmod 600` key file per hosted agent
  (§2.4's list); the page link from the watch host to `?spectate=0x…`.
- **jev lane:** `jev-watch`'s shard sets `cfg.spectate = Spectate::on()` and
  its bot joins `Join::Agent`; `jev-bot` grows `--agent-key PATH`, a name and
  `--server host:port` onto `botclient::run_agent_bot` — the JPEG feed can
  then retire behind the browser seat.
- **The browser loses the close reason.** A seat whose target left is closed
  with `REFUSE_WATCH_ENDED` as its code; the desktop `Session` reads it
  (`close_code`, and the disconnected screen says why) and the page's
  `WebTransport.closed` is not yet read, so a tab says "left the world".
- A delayed human feed's line is per seat in memory and reserved when the
  seat opens — 7.1 MB at 60 s, 5.0 MB of it an event line sized for eight
  messages a tick; reserving that half on demand makes it cost the traffic;
  `status.json` does not publish `spectators`; a seat does not see the
  target's open container panel (the container stream is per connection).

## 0gfx · Graphics rows and render scale are built *(client lane)*

- No hardware performance or appearance verdict yet: render scale, SMAA and bloom
  had only software-GPU smoke checks (`findings/render-scale-20260919.md`).

## 0wnd · Down, hand revive and medkit recovery are built *(sim+client lane)*

1. Medicine on a downed body (`reference/WOUNDED.md` §9): `Command::Consume` with a
   target. Content prices bandages and medkits but has no syringe row yet.
2. Refusals while down are silent (`live_slot_of` sends no event); the fix is each
   refused verb's own `REFUSE_*`.
3. No drag clip or voice: a remote crawl slides `Death01`'s pose (`render/anim.rs`)
   and the fall reuses `Cue::Death`.
4. The odds on screen are the odds at the fall; the sim re-reads the meters at the
   roll. Say so on the line, or resend.
5. §LOOK: `CRAWL_EYE_M`, `WOUND_ROLL_RAD`, `WOUND_DROP_S`, the vignette — never seen.

## 0site · Site art v0 landed — three things it left *(art + sim lane)*

1. §LOOK: nobody has booted `assets/models/site/{shelter,canopy}.glb` (only
   `tests/site_assets.rs`' arithmetic). Do the stated 1.196×/1.291× aspect
   stretches read as chunky-rustic or as wrong?
2. To retire the stretch, fit `SHELTER_BOXES` to the art (prompting for the aspect
   fails). It is sim truth (`test_replay`'s golden, the plinth consts,
   `SHELTER_CORNER_R_M`, `WAYSTATION_RADIUS_M`, guard aprons): a real slice.
3. `WANTED.md` §2 is done bar the stump (waits on §0stump); four §2 sizes were wrong,
   so read `occupant_volume`. Next: §4's seven greybox deployables, §5's fifteen held
   items (`meshy_gen` → `measure_glb` → `import_meshy`, ~6 keepers per 11 rolls).

## 0rk · The rock formations: levers measured and parked *(art + client lane)*

The re-packed maps (2026-09-16) and the kit-made nodes and boulders (2026-09-23,
`findings/kit-ore-nodes-20260923.md`) are in; nobody has seen them in the game
(`§LOOK`). What is left:

3. **The formation levers.** Tilt is out at these tolerances (`rock_b`/`rock_c`
   leave the blocked cylinder past 3°); a cluster per slot waits for a slab,
   because the top gate holds the main part at ≥ 0.97 scale; widening the Rock
   row is sim truth and the operator's call.

## 0rock · Rock is one tier drawn everywhere alike, and the cliff is bare *(sim + art lane)*

`reference/ROCKS.md` §9.7's ranking, measured with `examples/rock_stats.rs`:
rock is a shared rate field, so clumping cannot reach it and the cliff foot
carries no more rock or ore than open highland. What is left:

2. Rock species rides the trees' species field (§0fst item 4), so a region's
   rock family is a coin flip.
3. **Rock seeding v0** (§9.2): a coarse-grid parent draw for formations;
   boulders, small rocks and ore weighted by distance to the parent and the
   cliff mask. Totals conserved (`CONTENT.md` §4, `haven_prize`), and ore stays
   on the per-island budget (`terrain::ORE_TARGET`). Gates in §9.6. A wipe —
   the operator's.
4. **Cliff tier v0** (§9.3): `Occupant::Cliff` on the cell the veto empties, a
   box-list volume off its own slope; `ci/rock_kit.py gen --kind slab` builds
   the mesh once `ci/measure_glb.py` has a slab row. Tint and blend: `§LOOK`.

## 0rf · Rock face v0 — cliffs stopped being one pale sheet *(client lane)*

1. `§LOOK` on a GPU: every frame is lavapipe 1280×720 and the facet fade was
   tuned on that footprint (`findings/rock-face-20260923.md`).
2. A scarp's silhouette is still the smooth heightfield's — needs §0rock
   item 4's cliff mesh or finer worldgen relief (§0wg item 3).
3. Lip and foot are a hard contour line; the answer is scree at the foot
   (§0rock item 3). Blocks deciding rock vs turf read as paving — don't.
4. The 2.2 m fine facets vanish past ~20 m by design; whether that second
   lattice walk pays on a real GPU is unprofiled.

## 0anim · The animals cannot be bought until the client can move one *(client lane)*

- Client slice before the asset: generalise `anim.rs`'s clip machinery to mobs and
  author a skeleton, or deform a whole mesh as `render/mobs.rs`'s leg transforms do.
  Then pig `01a05e9f-775d` (wolf `01a05ea2-c78a` when wanted) can come in.
- The generator's rigging (`POST /openapi/v1/rigging`) refuses a quadruped, and its
  animal is one unskinned primitive: it can't be split onto `LEG_ANCHORS`.
- The brain has ten states and the mesh shows two: legs swing off position deltas
  and a sleeper lies down (`mobs::Gait::settle`); no bite, howl or flee pose.

## 0stump · A felled pine leaves scenery, not a second harvest *(systems lane)*

- Operator: stumps are collected for wood. Today a stump is only `render/props.rs`'s
  `FellPart::Stump`: no `content/gatherables.toml` row, no slot, no verb.
- Cheap shape: a second life on the tree's `SlotLives` entry (no new occupant, no
  wire byte). Expensive: a real `Occupant` at the skipped discriminant 8
  (`OCCUPANT_R_M` rows, archetype-table alignment).
- Buy `WANTED.md` §2.2's stump model after the verb, not before.

## 0kit · The build kit is the one row a generated mesh fights *(client lane)*

1. A piece's material is gameplay state: one mesh serves all four tiers, painted by
   one of `DMG_BANDS × N_TIERS` = 32 materials (`structures.rs`).
2. So buy geometry, not appearance: `should_texture: false`, keep the tier materials.
3. UVs bind: pieces carry metre-scaled UVs (`PIECE_UV_PER_M = 1.0`), Meshy an atlas
   unwrap. Fix: a metre box-projection mode in `import_meshy.py`, like `Soup::tiling`.
4. Dimensions are collision truth (`WALL_THICKNESS_M = 0.24`, a doorway's 2.1 m
   lintel underside), so try a doorway first or not at all.
- Not started (`WANTED.md` §3's six shapes). Order: 3 → one wall → look; 3 may sink it.

## 0dk · The player character is as dark as grass — measured *(art lane)*

4. It is the albedo: `stumpy.glb`'s baked `Material_1_baseColor` is linear luma
   0.056, level with grass and just above `ALBEDO_LUMA_BAND`'s 0.05 floor. Remote
   bodies, SSAO and the fill were measured and ruled out.
- The lift is nearly free: `base_color` ×2.4 (litter's 0.135) clips 0.14% of
  texels, ×3.0 (twig's 0.167) 0.34%. The other road is a Meshy re-bake
  (`assets/models/MANIFEST.md`).
- How far to lift is the operator's call (`ART.md` owns looks); it moves both
  hands and every remote at once.

## 0fp · The first-person pass, and the two judgements it is now waiting on *(client lane)*

0b. Unbuilt: the operator's haft "in line with the thumb with an open Palm".
   `VIEWMODEL_GRIP_Q` encodes a view pose, not the hand, so framed poses sit ~90°
   off the thumb; getting both means rotating the hand, which moves a visible arm.
1. The viewmodel reads oversized (true scale, 0.52 m from the eye; the hatchet at
   0.85 fills the lower-right quarter, `findings/capture-20260917.md`). Operator's
   taste call: shrink it, push it back (`VIEWMODEL_HOLD`) or keep it.
2b. The swing no longer chops: `swing_pose`'s arc is hard-coded in view axes for a
   −Z tool, so a +Y haft sweeps sideways (apex +70° vs −12°). Derive the strike
   axis from each item's rest direction; touches every held item, needs item 3.
3. The swing and chip burst run on `Time::delta_secs`, so lavapipe swallows them;
   only `./ci/scene.sh --play` on a GPU box shows either.
4. Look at the spear thrust (`viewmodel::thrust_pose`): 15.6 cm draw, 20.8 cm reach
   — thrust or twitch? Remote spears still play `Sword_Attack` (no right-hand thrust
   in the rig); the metal spear chops until `§0hand` item 1 lands its row.
- Unchased: two remote bodies read near-black in the first run's `7-player.png`
  (brown in the second). Look at the frames before it becomes a lighting item.

## 0nc · Netcode v2 landed — what the overhaul still owes *(client+server lane)*

1. Feel it at the bar: `netsim = "30,10,1"` on a dev shard with two humans — the
   stop test and the strafing-bro test. Every gate ran at zero RTT; operator's eyes.
2. NetLine gauges: the F4 net row (`render/hud.rs:2145`) shows err/confirm/mispredict
   only; `buffered_depth`, `repeat_count`, `playout_ticks()`, `jitter_ms`,
   `corrections_minor`, `DgRing::dropped` exist and nothing draws them.
3. The event lane is not shimmed (`DECISIONS.md` §open netsim row has the skew), and
   under netsim the stream lane leads its snapshots by lat_ms.
4. `RESYNC_AHEAD_TICKS = 3` is still a blind guess; it only matters for the first
   second after join/resync. Measure before touching.

## 0cs · The fight at population — what the combat storm left *(sim lane)*

1. `sim-core/tests/combat_storm.rs`'s duellists never move (`bots::brawl_step`), so
   `Rewind::pose_at` rewinds static poses. Strafing duellists would make the depth
   change the answer — the claim §0lc still cannot make.
2. No storm has a blast take a wall and the person behind it (§0rs item 1). That
   needs a third fixture — arming the raid storm would desaturate its five equality
   assertions.

## 0mag · Reload v1 — what the magazine still cannot do *(systems+client lane)*

1. A reconnect off `PlayerSave` finds the cylinder empty: the store's per-player record
   lacks the magazine the world save carries — a format bump in `server/src/store.rs`.
2. No unload, no ammo switch (the reference refunds a partial magazine and adopts the
   new round at `StartReload`; ours refuses, `REFUSE_RL_DRY`). Both need `reload` to
   see a stack ceiling: `GatherContent` reaching a `CombatContent` caller.
3. The dry click rides `rate_ticks` (0.4 s on the revolver); the reference gives it
   its own 1.0 s (`BaseProjectile.ServerUse`). Unspoken knob in `DECISIONS.md` §open.
4. §LOOK: the readout over the hotbar, `R` as reload-not-repair, and `Cue::Place`
   borrowed for a seated magazine — never seen or heard.

## 0vs · The newest visual report's ranked gaps are **closed** — steer from the judge *(any lane)*

- Visual gap 3 (`pass-20260815-042118-11`: no structure, character or viewmodel in
  a frame) is partly open: the panels-off rule and the missing hands.
- From the judge's gap 2, larger than a pass: a world event (a timed, announced
  window at the pad) and a guard loot tier (§0wc item 3: a third species, five client arms).
- The judge's gap 3, no session or wipe boundary, is untouched and the bigger one.

## 0h3 · The other flaky dial: `connection closed by peer: 261` *(server lane)*

- Undiagnosed, seen once: H3_FRAME_UNEXPECTED from wtransport's driver
  (`driver/mod.rs:598,637`, `driver/streams/settings.rs:123`). The raider panic now
  prints `handshake_errors`/`admit_refused`/`admit_retried` for the next one.
- Next: server-side wtransport tracing on a repro, or a pin bump past `a11e6a8` if
  upstream fixed framing. Don't widen `is_load_shed` past `0x107`: its gate forbids
  0x0106/0x0108, and a widened range retries refusals.

## 0lc · Lag compensation — **on**, and never fired over a real link *(sim lane)*

1. Not a loop's to do: fire at a moving remote player over a real link (~200 ms)
   with two `--features render` clients and `/status.json` read at both ends —
   `favour_clamped` says whether `stats::favour_for`'s ceiling of 7 is right. §LOOK.

## 0hrt · Being hit points somewhere — the rest of the fight *(systems+client lane)*

1. `Cue::Hurt` is not positional (`EV_HURT` is a bearing, not a place). Two blows in one
   frame are one heavier voice: the cooldown binds in-frame on purpose, so the fix is a
   second row (`a_cooldown_binds_within_one_frame`). No camera shake.
4. Nobody has seen the hurt arc (`§LOOK`; no vantage takes a shot at the camera): do three
   28° arcs on the 116 px ring read as three directions or as a red halo?

## 0hs · The body-part ladder — what limb band v0 left *(systems lane)*

1. The clip mutant (`exit.min(stop_t)` dropped at both damage sites) survives. A wall
   can't kill it; the arrow's tick boundary can — a `tests/shoot.rs`-shaped fixture
   driving `step` (arithmetic in `tests/headshot.rs`'s header).
2. No arm band (a cylinder can't tell arm from chest), and every weapon's `limb_pct` is
   50: the first that should differ decides whether the geometry widens
   (`reference/PROJECTILES.md` §9.4b).
3. Hit rung v0 is unseen and unheard (`§LOOK`: gold as a skull? limb cue lighter, not
   quieter?). The marker changes colour, not shape (the reference pushes ticks out, a
   `Node` mutation per tick). `Toast::hit_damage` is stored and never drawn.

## 0tl · The torch lights the ground — what it still cannot do *(client+systems lane)*

1. Nobody has seen it (`§LOOK`); a night frame is `ci/scene.sh --hour midnight` now.
2. `NIGHT_AMBIENT_LUX` (240× moonlight) caps the torch at a 0.89 m pool (`pool_radius_m`).
   Fix the ambient, not the flame: one owner over `rig`'s coupled set
   (`CLAUDE.md` §traps), judged by eye, never from a lane that is changing a light.
3. Hitting with a torch wears nothing (reference: ~7 condition a swing). V3 forbids an
   unreachable `condition_loss` row, so it needs a node or a combat row first.
4. Nothing says a torch went out: `cond` reaches 0 on `SUB_INV` a round trip late, and
   there is no `Cue` for either edge, no toast, no flicker.
5. A remote's held item doesn't swing (`BODY_PALM` is a fixed body-root offset; the hand
   bone in `models/stumpy.glb` is unverified) and is unseen (`§LOOK`, item 1). Worst-case
   datagram: 1058 B of 1100 (`snapshot_cap`), so the next `EntityState` field isn't free.
6. Your own torch draws no flame in first person; a remote's burns (`bodies::BodyFlame` with
   `fx::world::FireFx` at `TORCH_FIRE_SCALE`).

## 0eq · Equipment, after armor v1 *(systems+client lane)*

3. A body is not drawn wearing anything (no armor mesh; `worn` reaches only its owner).
   Design pass first: broadcasting it is raid intel `container_wire.rs`'s wear test refuses.
5. Unlooked (`§LOOK`): the paperdoll, and the inventory page with a container open at 1280.
6. Armor does not wear out (§9.4; the catalog has `cond_max`). `§0dur` owns it.
7. Right-click does not equip: `ui::slots::quick_move`'s `!looting` branch would ask
   `wearable_here` and send `CONT_WEAR`. Left out as a taste call, not for difficulty.

## 0gs · What ground surface v1 left open *(client lane)*

1. The biplanar wall tap has never run on a real GPU; it is gated by scrapes of
   its own WGSL (`tests/ground_tiling.rs`). `§LOOK`.
2. Per-identity tiling's rule-7 half: litter repeats 3.1× more often than it
   did and `MACRO_M`'s 48 m break-up is all that stands against it. `§LOOK`.
3. **`rock` still does two jobs**: alpine ground and the cliff face the slope
   veto forces (plus the prop fallback), all on the `Rock032` slab since
   2026-09-24; the road has its own aggregate layer (`aggregate_*`, array
   layer 4). Splitting cliff from ground is a fifth splat channel, and
   `ATTRIBUTE_COLOR` carries four weights and is full — a new vertex channel,
   ~30 files (sized 2026-08-28). `gravel_*` (unbundled) is the obvious cliff
   source.

## 0wg · What worldgen shape v1 and world structure v1 left open *(sim+client lane)*

1. Beach, coves, scrub band and species regions are unseen from the ground (`§LOOK`; only
   `examples/biome_map`). World structure v1 invalidates every save: a wipe, the operator's.
2. Lowlands are flat by choice (the `shelf²` weight spares the ~8 m build floor). Relief
   there wants a field that varies between build cells and is flat within one.
3. Relief finer than 18.75 m needs `FAR_STEP` (8 m) lowered first, which nobody has costed.
4. The shore terrace costs 2.9% of buildable 3×3s (accepted); re-open it with
   `examples/buildable` and `examples/biome_map`, not by feel.
5. Not a gate: a band whose two populations touch (|∇‖∇h‖| by elevation, coast perimeter).

## 0mtn · The island has mountains now: what interior massifs v0 left *(sim lane)*

Two gullied ranges in the 170–560 m annulus (`TERRAIN.md` §1 stage 4d,
`findings/interior-massifs-20260923.md`).

1. **A wipe, the operator's.** The ranges and the ore budget (2026-09-24) both
   move the world digest (`probe_terrain`); shipped together they are one wipe.
2. **Ore is budgeted per island** (`terrain::ORE_TARGET`, 160 metal / 120
   sulfur; `Haven::ore_pm`), but the scale caps at 1.75× on the Highland row's
   saturation rail, so rock-poor islands land short (seeds 42 and 555:
   147/101 and 138/100). Lifting them means reshaping the Highland row — the
   operator's call.
4. **Heights in the ranges still cost ~3.4×** (0.80 µs against 0.23 through a
   memo, `client/examples/frame_cost.rs`). The clutter tile is fixed (0.9–1.0
   ms on a range). A near chunk on a range is unmeasured: native runs it on
   `AsyncComputeTaskPool`, but in the browser that pool is the main thread.
   The finest gully octave is the lever.
5. **Side roads cannot route around a range.** Two opposed ranges keep a pass
   on every road seed; a third range needs routed roads first
   (`reference/ROADS.md` §9).
6. The gullies are a filter, not erosion: no fans, no widening valleys (that
   is a grid solve).

## 0ring · The coast ring has a continuous terrain bench *(sim lane)*

- Nobody has lapped it as a player: obstacles, entrances, a cut through a headland
  (`findings/road-continuity-20260919.md`; gate: `tests/road_continuity.rs`).
- It changes generated ground: rollout is the operator's world-compatibility call.
- The ring solve's startup cost is ungated.

## 0rd · Coastal routing is measured; production integration remains *(sim lane)*

Side road bend v0 left:
1. Unseen (`§LOOK`): road or wobble? `SIDE_ROAD_BEND_M` is the knob to turn after a look.
2. The wander is cosmetic by design (`ROADS.md` §7.1); a terrain-seeking v1 needs a gate
   that can tell it from the hash.
3. Both depot gates sit on the yard's Z axis (`depot.rs:28`), so approaches leave 180°
   apart until there is a second gate face or a second inland site.

Routing candidates are not shipped roads (`findings/road-network-prototype-20260916.md`,
`-p sim-core --example road_route`, `reference/ROADS.md` §2.1). Next, in order:
1. Pick stored geometry, query indexing, startup budget and failure policy from a broader
   seed sweep; hash the paths and prove native/wasm parity.
2. Validate obstacles, monument entrances, swept turns and player passage; smooth the
   angular candidate only under the same clearance checks.
3. Derive scatter, bay/barrel placement and client tier masks from the same geometry;
   keep the reward gates; assess save/world compatibility before any rollout.
4. Add a second useful inland connection, then optimize site distribution as a set.

- CC0 asphalt is surveyed in `findings/road-materials-20260916.md`, not shipped; branch
  grading, authored cracks and a distant ribbon remain.
- The inland site has no containers; arming rewards and its world register are operator
  decisions. Centre-directed spokes are no substitute for destinations.

## 0fst · The forest, after world structure v1 *(sim + client lane)*

1. Nobody has looked at any of it (`§LOOK`); first, is the scrub band a treeline or a moat?
2. Gate 5 (size classes) is the one `reference/FORESTS.md` §8 gate unbuilt; `Slot::scale`'s
   ±10 % is not a class. Design: §9.4; precedent: `Slot::species` (a byte off the hash).
3. The edge has no small-tree mesh; once gate 5 exists, a treeline draws the small class.
4. One species field serves every occupant (a birch region is a granite region); a second
   channel is one `CH_*` and one fBm read (`ROCKS.md` §9.1).
5. A third species is a slice: `SLOT_SPECIES` = 2, the draw is a bool against
   `species_share`, and `render/tree::SPECIES` grows with it (`props.rs` const assert).

## Sim, content and gameplay verbs *(systems lane)*

## 5 · Gameplay still missing, in rough order of what a player notices

Operator call for items 1–2: ranged tracks the reference (`reference/PROJECTILES.md` §9).
1. Nobody has watched a wall come down, a bench fall or a raid decal draw (`§LOOK`).
2. `sim-core/src/spent.rs`: a lodged arrow stays at the impact point instead of riding the
   body, and one that expires mid-air leaves nothing. §9.6 (the reference's ~35 bow
   damage) is unblocked: a `RIPLIST.md` take against `CONTENT.md` §4's bands.
4. A forest-floor pickup archetype and a farming lane (`server/tests/farmwalk.rs` isn't one).
5. Tree depth and the blueprint item are `§0tree`; still missing is the wipe schedule
   blueprints are promised to outlive (`DESIGN.md` §8).
6. Crops. Night has its reasons now (the cold costs hp, `exposure.rs`; a torch keeps
   wolves off) and its sky (moon, stars, `/time`); nothing in `content/` grows.

## 0pvp · What a fight still cannot do *(systems lane)*

1. Nobody has seen the flinch pose; a bystander flinch is refused on fan-out
   grounds (`DECISIONS.md` §open "attacker-side flinch v0").
2. Flesh is heard attacker-side only (`Cue::FleshHit` off `EV_HIT`; a broadcast
   overflowed the storm's event lane, #179); nobody has heard `Cue::RemoteSwing`.
4. Armor, none blocking: `balance.rs`'s anchor is slot-blind (re-speak
   `armor_extra_hits_max` or re-price); `reference/ARMOR.md` §9.3–9.4 owes damage
   types, hit areas and worn condition. `move_penalty_pct` (unread, `bake.rs:866`)
   waits on the operator speaking §9.5 item 4's non-stacking rule.
5. Lag compensation is on; only the real-link test is left (`§0lc` item 1).

## 0ray · What melee aim v1 left *(systems+client lane)*

1. Nobody has swung one on a screen (`§LOOK`, first item): aiming or stooping at
   a knee-high node? `bots::PITCH_LOW` was set to arm a gate, not by eye.
2. The crosshair names only scatter (`ui::interact::resolve_swing`); `melee::cast`
   already answers players, animals and walls too.
3. An animal is one cylinder with no part bands; `reference/ANIMALS.md` has no view.
4. The weak spot is stance-based (`gather`'s sector) and ignores the look.
5. `render/anim.rs` draws a remote swing level; the pitch is on the wire.
6. `E` on a world container is a ray now (`resolve_open`), narrower than the sim's
   planar `worldcont::open`. Look first; if stooping reads badly, pad this cast
   rather than add a second rule.

## 0mk · What piece marks and shot stops still owe *(systems+client lane)*

2. An OPEN door is air to a shot (as to a body): an unasked design question.
3. Rims and diagonals need a piece address on `EV_IMPACT`, whose pad bits the weapon kind spent (wire v76):
   a wider event and a `PROTO_VER` turn.
4. Spray paint is a deployable, not a decal (`limits.rs` cap, `worldsave.rs` slot,
   privilege, decay, moderation); decide stencil vs painted first.
5. Untested: `cell_edges_stop_shot`'s high-face stop names cell+1 (`collide.rs:1641`).
- `§LOOK`: #179's atlas marks on every surface, desktop and browser, and the
  weak-spot cross are all unseen.

## 0wc · What world containers v0 still owes *(systems lane)*

1. Nobody has opened one in the running game: anchor per `container_wire.rs:1307`,
   `dev_spawn` in `shard.toml` (`server/src/config.rs:361`), boot (§0p3).
1b. `takes_deposits` keys on kind; a fuel slot or vending machine needs it per instance.
2. An emptied crate looks full from afar: a lid state (`render/props.rs`), a
   shorter refill, or vanishing (`reference/LOOT.md` §9.4: a wire bit per crate).
2b. Ground items: no tumble (`reference/LOOT.md` §9.3), most draw as a pouch
    (`HELD_MODELS`), barrels pay 1–2 stacks (`content/loot.toml`, `ci/haven_prize.mjs`).
3. The guard pays a wolf's loot (`guard.rs`): a tier needs a third species, which
   falls to the pig in `render/mobs.rs`, `sound/voice.rs` and `ui/death.rs`
   (`loot.toml` can't carry it: `content/src/validate.rs:887` refuses zero hits).
4. Nobody has fought a guard in the running game (route as item 1).
5. `inventory.rs:110`'s `slots_in` treats every non-box kind as `INV_SLOTS` wide,
   untested; add an explicit arm under `container_wire.rs:1359`'s guard.

## 0pr · What predator v0 still owes *(systems lane)*

1. Nobody has heard it; `client/src/bin/soundbank.rs` dumps the bank to WAV.
2. A wolf drops no hide or bone (`content/mobs.toml`): recipes and icons come with it.
3. No night-only roster variant; the night's cost so far is the cold (`exposure.rs`).
4. No gate keeps the growl's 14 m (`CUES`' growl row, `crates/sound/src/lib.rs`) inside the wolf's
   15 m night notice radius; a `mobs.toml` edit reddens nothing.

## 0m · The pig is in — what the roster still owes *(systems lane)*

1. A butchering verb (tool-gated) on the body: no `ui::interact::Verb` arm; output
   goes to the corpse bag (`mob::strike`). Research: `reference/ANIMALS.md` §9.5.
2. Wolves howl on finding you (#178); pigs owe an aggro cue, every bite a hit-direction tick;
   holding a charge is free. Voices still run on a timer, not the brain's state
   (`sound/voice.rs`: near growls, far howls).
3. `render/mobs.rs` is box massing; at 8 m the head barely separates.
4. `MAX_MOBS = 64` came from the wire budget and has never met a playtest.
5. Should `ttk_melee` widen (rock vs spear)? `DECISIONS.md` §open "tools as weapons".
6. A blast hurts no animal: `charge::detonate` never takes `Mobs` (arrows and bullets do
   since 2026-09-25, `ranged::Quarry` → `mob::hurt_slot`).
7. The brain's numbers are code and shared by every species (`brain.rs`: 2 biters, 3 tries,
   60 s heal, 20 s howl, 7 m orbit, 40 % sleep; `noise.rs`: 100/15/25/200 m hearing); only
   sight, pack and fire fear are in `content/mobs.toml`.
8. Nothing shows the brain's state: no admin command, overlay or log.

## 0ctl · Four controls the player expects and the sim has no verb for *(systems lane)*

2. ADS / secondary (RMB): RMB already places, builds and half-grabs; answer the
   held-item modality before a `BTN_SECONDARY` bit (`PROTO_VER` bump).
3. Flashlight (`F`): the torch and its right-click `BTN_LIGHT` toggle exist (torch
   fuel v0, `render/input.rs`); `F` itself only nudges the plan's height.
4. Voice (hold `V`): no capture, codec, `KIND_*` or fan-out; `reference/VOICE.md` §9.
- ⚠ Bind each key in the commit that gives it a verb. With the plan in hand `R`/`F`
  nudge foundation height (`ghost::height_keys`); otherwise `R` repairs with a hammer
  out and reloads with anything else (`verbs.rs`).
- Free look sways the viewmodel (`viewmodel.rs` reads `eye.yaw`); §open "free look v0".

## 0sp2 · What the spill still cannot say *(systems lane)*

- A partial spill and the four give-backs (demolish refund, pick-up, unbolt, craft
  cancel) say nothing; one wire field buys both (operator; `DECISIONS.md` §open).
  The ring is `client-core/src/core.rs:900`, item index only.
- A spill merges into the nearest bag, even another player's death bag
  (`sim-core/src/backpack.rs:51`); §open carries it.
- No frame has shown a spill line ("pack full — Wood dropped…"); headless only.

## 0bl · Building catalogue and remaining playtest *(client+sim lane)*

1. Human playtest: snapping, corner posts/aprons, soft-face readability, walking a
   furnished base (`findings/building-circulation-20260921.md`).
2. Perf option: memo `col_base_y`'s terrain sampling (volley 1.25 → 3.07 ms/tick).
3. Diagonal-wall UV stretch: the √2 root scale stretches the slab texture (`ART.md`).
4. Operator calls (`DECISIONS.md` §open "piece flanks v0"; `reference/BUILDING.md`
   §9.24): placement inside a body; height-offset foundations vs privilege/stability.

## 0ac · Inserts, soft faces and diagonal price *(systems lane)*

2. Soft/hard wall-face shading awaits human review. Floor-side damage needs a
   vertical attack direction; per-material resistance is `RIPLIST.md` §2.
3. Diagonal price is an operator call (~1.41× the length, priced per socket;
   `DECISIONS.md` §open "triangles v0"); triangles want play on an occupied base.

## 0tt · The bench ladder's craft rebate and tree panel, unseen *(systems lane)*

3. The operator has not seen the tree panel, tabs, bench root or tier badges: boot,
   stand at a bench, `E`, buy a node, watch it turn KNOWN.

## 0tree · How deep the research tree goes, and what the blueprint item left *(systems lane)*

1. A new intermediate item (beancan, flare, embrasure) means re-running the edge
   rule (`reference/BLUEPRINTS.md` §9.3), never typing an edge.
2. Residuals, none a defect: a ground sheet reads "Blueprint" (no `cond` in stack
   sync); a late opener gets no wait bar; no bot holds a table, so `begin`/`table_sweep`
   and the fire's conversion ride no parity/replay surface.
3. Nobody has seen the panel or table work (`client/tests/ui.rs` §M, §V): at a
   table, `E`, a revolver and 30 junk, BEGIN, 10 s, take the paper, right-click it.

## 0rs · Bodies are out of the raid storm *(systems lane)*

1. No fixture blasts a wall and the person behind it (`raid_storm.rs` zeroes the
   throwable's damage on purpose): wants a bounded gate at the command ceiling.
2. The fleet only raids itself (`raid_shape.rs:33`); `§0pop`'s `index % 2`
   owner/attacker split is the knob that would model two parties.

## 0rc · The wire raid's two unmeasured differences *(systems lane)*

1. `raid_shape.rs:73`/`botclient.rs:399` say `push_action` drops; server `core.rs:722`
   and `net.rs:2054` keep it ringed. Settle it before quoting "leans optimistic".
2. `Client::consume_input` (`server/src/client.rs`) lets one frame's buttons act
   per tick, so `charge_slot` may not be in force when the throw lands.

## 0r · A charge cannot dud or be stopped *(systems lane)*

2. No dud chance, no defuse verb (`sim-core/src/charge.rs:38`); each its own verb.

## 0wx · Weather and exposure — what #176 left *(systems + client lane)*

1. Exposure is wet and cold only: no overheating (no desert, so the Dust preset was dropped), no comfort
   regen, and the cold burns no extra food or water. Burlap is the only warm clothing (`content/armor.toml`).
2. Lightning is a 0.35 s brightening: no bolt, no directional flash; `weather::Bolt::bearing` is never read.
3. WET and COLD say what, not why, and nothing confirms a roof or a fire is working (FREEZING now says
   what fixes it, `render/hud.rs`).
4. A sapling is the adult tree scaled 15 → 100 % in 16 steps: no sapling model.

## 0sk · Skins — what v0 left *(client + platform lane)*

1. No skins screen: the craft picker is the only place a skin shows in the world, and the menu's
   ITEM STORE entry (`ui/hub.rs`) opens nothing until §0s item 2's `store` link exists.
2. A look is a flat colour multiply: no per-skin texture or mesh; deployables are refused as targets;
   `season` does nothing.
3. A ground item carries no skin on the wire, and worn armour is not drawn at all (§0eq item 3).
4. A purchase lands at the next ownership check, asked when the inventory or crafting page opens and at
   most every 15 s; a check that fails at join owns nothing until the next.
5. **Operator:** confirm elo's catalog ids are per title: `/api/items/of/{wallet}` names no title and the
   shard ignores the response's `collection`.

## 0up · Upkeep v2 landed — what the reference's upkeep has that ours still lacks *(systems lane)*

1. Heal under upkeep (10 min unattacked, at the decay rate; Devblog 189): a per-piece
   last-hit clock is a `WORLD_SAVE_FORMAT` change and a wipe — ride the next bump.
2. Exact rent: `upkeep::Tax::charge` rounds each hour up. Charge period `k` the
   difference of cumulative floors `⌊cost·rate·k/24⌋`, require ≥ 1 held for a
   zero-charge period, and re-price the tests' 5-unit fixtures in hundreds.
3. A hearth panel (24 h cost per resource, live clock, crew HUD vital); no withdrawal.
4. Door and insert upkeep (Devblog 190): ours charge nothing in a claim.
5. Group tax: rent per authorized player past four, unmeasured vs `HEARTH_CREW_CAP`.
6. No gate runs the inside discount: roof an unpaid piece in the replay (`test_replay`).

## 0aa · Building rights: the roster's third customer is missing *(systems lane)*

1. No `AutoTurret`: `sim-core/roster.rs` serves only the lock lists and hearth crew.
- ⚠ `sim-core/{deploy,claim}.rs` doc comments cite §0aa's old item numbers; re-point.

## 5d · The agent player: the trust ledger is kept; the agent API is not *(systems lane)*

Local agent player (2026-09-22): `jev-bot` / `jev-watch` run a survivor on a
loopback island. Jev, the scripted policy or an external agent (JSON lines on
a child's pipes) picks goals; local skills gather, forage, craft by name and
equip, eat and drink, and answer the death screen in-game, under a request
floor of one a second and an hour/day spend guard. `crates/server/JEV.md` and
`WATCH.md` own commands and limits. Every jev bot is a watchable agent
(`NETCODE.md` §2.3): unsigned on loopback, or signed with `--agent-key`
(§2.4, `tests/jev_door.rs`); `--bots N` shares one island. The hosted
preview still runs the step-level bundle until the operator restages it.
Next: loot its own death bag, cooking, and an operator-provisioned wallet
plus a routable spectate shard for a public agent (operator acts).

`PLAYERS.md` has the spec — verb set, observation encoder, four walls. Wall 3
is built (`EV_TRUST` code 39, `World::log_trust`, six checks in
`crates/sim-core/tests/event_roles.rs`); walls 1 and 4 are built for the local
agent (`server/tests/agent_walls.rs`, `sim-core/tests/agent_input.rs`); wall 2
is not.

Remains, in order:
- ~~Nothing reads it~~ and ~~a dropped row is gone~~ — **built 2026-09-22
  (trust ledger v1).** `World::trust` is a per-tick ring no tick can overflow
  (`sim-core/trust.rs`: one `TrustSeat` per command, spent by value).
  `ShardCore::tick` drains it every tick into `<world_file>.trust/`
  (`server/trustlog.rs`), and `trust-log` reads it. Knobs:
  `DECISIONS.md` §open. The public shard logs once a build with it is
  deployed (operator act), and only because it has a `world_file`.
- **`Command::Loot` mints no trust row.** Emptying another player's bag
  with `Loot` is silent, while taking one stack of it with `Move` logs
  `TRUST_CONT`, so the record depends on which button was pressed. Decide
  whether a corpse bag is trust (`TRUST_CONT`'s doc says bags are), then
  re-measure `loot_storm.rs`'s counts in the same commit.
- `TRUST_GIVE` waits on the give verb; there is still no player-to-player give.
- Then the social verbs for agents (`give`, `authorize`, `speak`): human client
  first, and `agent_walls.rs` keeps the agent's set a subset. Entry price and
  earnings are `ALPHA.md`.

- Nothing reads `EV_TRUST`: a server lane must sink it (`server/src/core.rs:2465`).
- A dropped row is gone (drop-newest ring, `limits.rs:624`); resync can't re-derive it.
- `TRUST_GIVE` waits on a player-to-player give verb.
- Then the verb table with wall 1's subset gate, then an agent client that plays
  badly (spec: `PLAYERS.md`; entry price and earnings: `ALPHA.md`).

## 4 · A payload swap is still not a compile error

1. A payload-role table read by emit site and check, so an a/b swap won't compile
   (`reference/FINDINGS.md` §1 end; `event_roles.rs:3486`). Bigger than one pass.

## 0q · The gaps nobody has claimed

`crates/`/wire work no single-surface lane may take.

1. UDP buffer: raising `rmem_max` on the public shard (8 MiB asked) is an operator act.
2. Shore barrels as a second destination class, so the ring has two ends.
3. The wipe, unscoped: no `wipe-now` exists; the loop owns the mechanism, the operator
   the trigger. Economy: `ALPHA.md` A1→A3 (whose §Admin lane cites "§0q item 2").
4. The soak owes tick jitter as a distribution and an hour-long run (last: 25 min),
   re-run with the contention instruments (`raid_storm.rs`, `bots::raid_step`).

## 0zd · Doors and locks — the key lock's blocker died and nobody re-took it *(systems lane)*

1. The key lock's blocker is paid (`ItemStack.cond`, `gather.rs:532`): re-take the
   call (the reference dropped keys, Devblog 193) or fix `reference/DOORS.md` §9.7
   and the `DECISIONS.md` 2026-08-08 row.
2. `DECISIONS.md` says `L` opens a keypad HUD line, not a panel; the client ships
   `render/hud.rs::pad_overlay`. Fix the registry.

## Wire, shard and persistence *(server lane)*

## 0fan · The event lane's fan-out — four arms filtered, nineteen to go *(server lane)*

1. Operator's call (§open "event-lane fan-out v0"): raise `EVENT_RING_CAP` (read
   `limits.rs`; worst fixture 81/128; 322 B/slot/conn) or batch events (`PROTO_VER`).
2. Operator's call: should an owner hear their door knocked from anywhere? No owner
   check exists (`server/src/core.rs` `EV_KNOCK` arm, `hud.rs`).
3. Aim `EV_DEPLOY_PLACED` like `EV_PIECE_PLACED`, then the deploy walk, to free
   `EV_DOOR`/`EV_OVEN`; `deploy_wire.rs` reddens by design. Sizing:
   `findings/swing-fanout-20260824.md`.
4. The combat and raid storms pass alone but together fill the 256-event cap (88
   dropped, all resync; `findings/note-20260830-two-storms-are-additive.md`).
   Candidate: §open "refusal coalescing v0" (window, counting, site unspoken).

## 0n1 · Class-S interest — the grid is still missing *(server lane)*

1. The grid: no chunk version or subscribe/unsubscribe, so removals stay broadcast
   and a re-arm re-walks the in-range set (`NETCODE.md` §5/§7; a wire change).
2. Deploys and backpacks are unfiltered (`server/src/core.rs:1975`); their walks
   restart on a removal (`:2396`, `:2819`) — `reference/NETWORK.md` §9.2.1.
3. `test_stream_in` (`NETCODE.md` §11) is unbuilt; per-frame apply/teardown is ungated.

## 0tx · The transport's three residuals *(server lane)*

1. BBR vs CUBIC (`cc` in `shard.toml`) is untried on a real path with real
   players; `net_congestion_events` is the reading.
2. Ops: read `net_rcvbuf_asked`/`net_rcvbuf_bytes` before tuning; `rmem_max` decides.
3. No client telemetry: nothing asks the `Arc<Connection>` at
   `crates/client/src/lib.rs:291` for `stats()`/`rtt()`; the HUD has no loss/RTT.

## 0sp · The encoder is the tick's largest phase now *(server lane)*

1. The encoder is ~0.43 ms of a 0.83 ms tick (100 clients, one AOI cell). Rank with
   `valgrind --tool=callgrind`; `server/src/bin/profile.rs` must never be a gate.
2. `World::scatter_clear` (`sim-core/src/world.rs:1541`) resolves cells cold, but a
   memo only pays across repeated picks: measure a respawn storm before `&mut self`.
3. The soak still owes tick jitter and real bytes (§0q item 4).

## 0y · Persistence — the three questions still open *(server lane)*

1. Should a sleeper block movement? Unanswered; lootable-alive comes after.
2. Same-window rejoin: a victim reconnecting in its eviction window reads the store
   before the eviction save is filed (takeover hint: `server/core.rs:487`).
3. No WAL yet; `worldsave.rs`'s module header fixes its shape.
4. Ungated, hand-checked only: the three-thread shutdown path (SIGTERM flushes,
   SIGKILL leaves no `.tmp`) and `KeySlot`'s id match (`server/net.rs:573`).

## 0ad2 · What the admin lane still cannot do *(server lane)*

1. Bans are memory-only (`server/src/admin.rs:173`): persist them in their own file
   and format version (the player store's header wipes on a seed change).
2. Nobody has typed a command at a live shard: the `REFUSE_ADMIN` close
   (`net.rs:791`) is undriven, with no client dialog (`client/src/lib.rs:486`).
3. The anomaly log (JSONL) has no reader to give the alpha gate a verdict.
4. No `/who`. `/time` and `/weather` shipped (#176, stored in `weather::Env`) but answer
   only in the anomaly log, and nothing stops `dev_env` in a public `shard.toml`.

## 4b · The domain gate's one file-local residual

1. `death_causes_are_a_closed_ledger` (`sim-core/tests/event_roles.rs:3704`)
   scrapes `world.rs` alone; follow `sim-core/tests/domain_ledger.rs`.
- ⚠ The label `4b` also names the world lane's section (§Labels).

## 0pop · The inhabitants nobody has run for longer than a test *(server lane)*

1. Run it past a test: set `population = 8` in a real `shard.toml` (commented out
   at `shard.toml.example:298`), run the shard, read the population line.
2. Can an inhabitant afford its raid rows? Its kit is a rock and a torch
   (`content/balance.toml`); `bot_smoke.rs` grants the satchel. Judge -18 §B.2.
3. `DECISIONS.md` §open "shard population v0": the 300 s shift, the 2 s backoff,
   an alpha shard's N, the `index % 2` owner/attacker split.

## 5b · The wire still accepts two refusal reasons the sim can never mean *(server lane)*

1. Craft-refused (`craft.rs` `REFUSE_*`) has no max: `REFUSE_C_MAX` is taken by
   `survival.rs`'s consume refusals; pick a name the domain scanner tells apart.
2. Deploy-refused (`deploy.rs:314-318`) has no `REFUSE_D_MAX`.
3. Add both to `event.rs`'s `DOMAINS` (`every_domain_fits_its_wire_field`).
- No `PROTO_VER` bump owed: the narrowing rule at `PROTO_VER` (`protocol/src/lib.rs`).

## The frame, the screens and the client's own hot path *(client lane)*

## 0fill · The darks, second half: the transfer *(client lane)*

- Left: the transfer. Shadow on open ground faces up, so no hemisphere darkens it (p10 79.9 vs `ART.md` §3's 49).
- The lever is the tone curve, not the fill (`rig.rs`'s fill is illuminance, rule 3 a display ratio):
  `Tonemapping::TonyMcMapface` and `exposure_ev100(frac)` (it follows the day now), both `rig.rs`. One owner, frame open, not blind.
- ⚠ Every `findings/*-visual.md` predates `rig::DayPin`: its luma, sky and shadow numbers are not comparable.
- Blocked on a pass that can capture; it goes first. §0gp item 1 (8.0% mean luma) is this owner's debt too.

## 0gc · A blade shaded exactly like the dirt it stood in — the tip blend nobody has judged *(client lane)*

- `BLADE_TIP_BLEND = 0.75` is invented; judge it against `ART.md` §5's "blades catch a rim of sun at their
  tips" (knob: `DECISIONS.md` §open, clutter contact v0).
- Don't turn `double_sided` off: no blade is ever flipped, and it would black out the real back faces.

## 0gp · The ground splat's residuals: a projection, a specular, and five prop maps *(client lane)*

- ⚠ Nothing in CI compiles `assets/shaders/ground_splat.wgsl`: a syntax error
  is green in CI and dead at boot. Boot it (lavapipe works; `ci/scene.sh`).
1. Only the albedo has the biplanar wall tap; normal, roughness and AO stay
   planar XZ on steep faces (`RENDER.md` R4).
2. The −0.4% roughness null result has not been re-measured with energy in
   the specular lobe (`DECISIONS.md` §open "specular v0").
3. `ground_detail.jpg` is loaded by nothing (`textures::GROUND_DETAIL`);
   deleting it is a separate call — a pre-baked field is what a cheaper LOD
   would want.
4. **(operator)** Granite is brighter than beach sand and the minimap's `ROCK`
   does not follow; fixing it departs from a `mapraw.jpg` reading
   (`DECISIONS.md` §open "minimap palette v0"; `client/tests/map_palette.rs`).
5. The five PROP roughness maps are unread, and `render/props.rs`'s reason is
   false (Bevy multiplies `metallic`, default 0, by the map's B channel). It
   needs a level call: the map whole loses the authored `rock 0.88` /
   `ore_stone 0.80` split, and mean-placing wants ×1.44 where Bevy clamps at 1.

## 0gi · What the island still cannot show: no occluder at blade scale *(client lane)*

4. No occluder at blade scale: clutter is `NotShadowCaster` (`clutter.rs:504`), so a blade's dark base never
   darkens the ground (`ART.md` rule 2). SSAO is already on (`rig.rs:284`).
5. Litter (2.49× grass, `terrain_mesh::GROUND_ALBEDO`) wins every mix; grass needs ≥78.0% to read green, but
   `ground_where_the_green_goes.rs` asserts only `> 0.66` / `> 2.0×`.

## 0cards · The population wears photographs now — what is left *(client lane)*

1. Conifer needles are still generated (`tree::needle_image`); Set 9.5 (sprig atlas) is fetched: atlas + `base_color`.
2. Standing litter is still `stand`/`blade` geometry; Set 9.8 (fern atlas) is fetched: a `card` (`PLANTS.md` §6.3).
3. The outer ring's tree hulls are untextured (§0out item 1): the largest flat-green area left.
4. `tree::needle_mips` takes the bisection midpoint, not `hi` like `mipmap::preserve_coverage` (latent; gate pins it).

## 0w · The props' remaining gaps — darks, density, unread roughness *(client lane)*

1. Top visual gap: p10 71.0 vs 41.0 (`RENDER.md` §0); the transfer half is left (`RENDER.md` §5 item 6), one owner.
2. Midground trees are small and sparse: `terrain::scatter` density, conifer scale (the ceiling §0t item 2 prices).
3. The dirt skirt is nobody's: `props::SINK_M` sinks props; boulder-meets-turf crowding is missing (`ART.md` rule 2).
5. Ten `assets/textures/*_rough.jpg` are unread, blocked on ORM packing (B is metallic; `render/props.rs:1090`).

## 0out · The horizon has trees — what the outer ring owes *(client lane)*

1. **Highest value:** untextured hulls (`foliage`, no map); `WANTED.md` §9.5's leaf texture fixes them and the bush.
2. `harvest_changed`'s 2.34 ms is a floor since density v1 (no GPU number): fix `HarvestedSet::contains`'s linear scan.
3. Only trees: boulders and barrels stop at `NEAR_RADIUS` (a sub-pixel lump costs an entity, no silhouette).

## 0t · the forest — what it still owes *(client lane)*

1. Forest scale v0 (`DECISIONS.md` §open; bench only): (a) `OCCUPANT_TOP_M[Tree]` 5.7 m under a 14 m trunk, one
   sim row (`shoot.rs`); (b) re-read `tree.rs`'s band table at 14 m, since the hull shows past the ~50–61 m swap;
   (c) birch bark is a `CANDIDATES.md` row. Treeline 288 m is browser-era; a desktop budget buys `OUTER_RADIUS` 5.
2. Close the crown: `TREE_MAX_R` 2.9 → 4.0 lifts cover ~20% → >35%, moving `SPAWN_CLEAR_M` 4.5 → ~6.0 (spawn
   search, both goldens), one sim row and an `examples/tree_sweep.rs` sweep (`reference/FORESTS.md` §1.2).
4. `aWind`: `StandardMaterial` can't read a custom attribute; wind needs `RENDER.md`'s custom material (LOD1 free).
5. Sub-canopy empty, shrubs one blob (`Occupant::Bush`, `PLANTS.md` §2): ez-tree `bush_*` and a 40% small tree
   as new `Occupant` variants plus scatter rows.
6. Both cards are still generated (`tree::needle_image`, `tree::leaf_image`); `WANTED.md` §9.5 is the upgrade.
7. Canopy grain v0 is unseen (`§LOOK`). Sky through a near crown: raise `TWIGS`/`NEEDLES_PER_TWIG` (not
   `AXIS_NEEDLES`) and widen `tests/tree.rs::the_cards_hold_the_density_the_forest_was_built_at`. Also: 11 cm
   leaf vs a birch's 3–7 (`BROADLEAF_MAX_R`, sim); `NEEDLE_HI` luma 111 vs lit grass 59–70; `CANOPY_AO_GAMMA` invented.
8. The capture probe should walk one chunk before it shoots (`tests/ring_handoff.rs`); the browser's 55 m rung
   (`quality.rs`) rests on its own argument now.

## 0a · The clutter ring still ends on a line *(client lane)*

1. It ends hard at ~32–45 m (`CLUTTER_RING = 2` × `CLUTTER_TILE_M = 16.0`; `render/clutter.rs` has no distance term).
   Fade as `sim-core/terrain.rs::swept_here` (thin by hash, scale survivors to 0), once a person sees the edge read.
2. Beach skirts are thin from the scatter table (~0.22 vs ~0.95 prop centres a tile, browser-era, not in the
   tree): re-measure against `terrain::scatter` before acting.

## 0y · The sea is a volume — what it still cannot do *(client lane)*

1. The last hard edge: the alpha ramp is per-vertex, so it rings in the shallows. Fade on prepass depth in the
   fragment (`ExtendedMaterial` + WGSL, `RENDER.md` §8); SSAO already puts a `DepthPrepass` on the camera.
2. One sea state: a storm is `WAVES` × a scalar the sim would have to publish — wire, not renderer.
3. Nothing reflects: read `reference/WATER.md` §5/§6 first; the expensive half, and the payoff is the sky.
4. Underwater is audio-only; a colour grade there is a second haze owner — the lighting owner's.
5. The submerged duck is gain/rate/pan; a real low-pass needs a DSP stage in `sound::engine`.
6. `Splash` is the only waterline producer: no stroke, no wake, no interactive deformation.

## 1 · The native pivot — the one visual gap left of it

1. Cloud form: the deck reads stratus where `ART.md` §4 asks for cumulus (p90 gap 25 luma, measured on the old
   baked deck — re-measure #176's composed one first); `RENDER.md` §8 ranks it second, behind the gate-asserts item.

## 0chr · The clips the wire cannot yet ask for *(client lane)*

1. `interp::RemoteState` lacks the states for `stumpy.glb`'s `Jump_Loop`, `Swim_Fwd_Loop` and crouch pair;
   crouch moves only the animal brain (`brain.rs`), never the body.
2. The gather swing is `Sword_Attack` (operator), blocked on item 1; a remote spear plays it too, and a thrust
   for other players is an asset ask (`Punch_Jab` leads left; `Punch_Cross` was refused).
4. No render layer, so arms and held item clip into walls; a second camera would duplicate the exposure/tonemap owner.
5. Head pitch clamps at `ANIM_HEAD_PITCH_MAX` (0.9 rad) and `anim.rs:811` drops the rest: spread it down the spine.
6. The hand reads large. A grip wants a second baked pose swapped in when `held_model_in_hand` is `Some`,
   never a bigger curl (`ci/curl_hands.py`). Not commissioned (operator, 2026-09-01).
7. Unlooked-at: `Death01` on a real body, the collapsed off arm, the sleeper tint (`7-player.png`, `ci/scene.sh`).

## 0hand · Four items still draw the generic stand-in *(client lane)*

1. Metal hatchet/pickaxe/spear: no asset (`assets/models/WANTED.md` §5.6; `content/items.toml` ships two); the
   stone glb would need a second material for the head.
2. Fire pit: `assets/models/deploy/fire.glb` bakes a lit emissive that `held_assets.rs::nothing_held_glows` refuses;
   needs an unlit variant or a generated `heldgen` row.
3. Resources, ammo, bandage, lock: no models (not in `ui::hold::HELD_MODELS`).
4. The item has been parented to the hand with a re-derived grip since 2026-08-30 (`dress_arms`,
   `tests/viewmodel_arms.rs`); nobody has looked at a mid-swing frame to see if the fist still trails the arc.

## 0dur · Durability: the words, the wearers, the bench *(client lane)*

1. The detail pane says nothing in words: `render/panels/craft.rs::build_detail` never reads `cond`.
2. Weapons and armour don't wear (`condition_loss` only in `content/gatherables.toml`; no `sim-core/src/armor.rs`):
   a research row first (`reference/DURABILITY.md` §5); on-swing wear is `DECISIONS.md` §open "tools as weapons".
3. Repair is re-craft in v1 (Q3). A repair bench is `Station::Workbench1..3` (`content/src/schema.rs`) + a blueprint
   check, never a new deployable; `DURABILITY.md` §3's 0.20 stays DISPUTED until checked against the in-game price.

## 0ps · Pieces: staged damage, the catalogue, the repeated wall *(client lane)*

1. Damage bands were never staged (one row, hit N times, photographed per band); marks are a plain mesh now, so a capture draws them.
3. A hundred identical walls (rule 7): `render/structures.rs` wants per-tier variants (offset + tint) by address hash.
4. Trim (lashings, plank seams, capstone rim) in `shape_parts`; price the entity count at `MAX_PIECES` 8192 first.
5. Deployables show no damage (no `hurt` term in the deploy material), and nothing shows which face was struck.
6. Roughness maps unwired (scalar `perceptual_roughness`); one ORM packing step serves terrain, props and pieces.

## 0lock · Lock placement reaches doors and boxes *(client lane)*

- Check the door-edge and box-plane lock targets together in the building playtest.

## 0fx · What impact fx v1 left *(client lane)*

1. Nobody has seen any of it (`§LOOK` item 0): likely sparks too many, dust too opaque, the whoosh a beat late;
   nor #179's blood, blast, muzzle flash, tracers and fire.
2. A deployable's matter is a guess (`struct_point` says `Wood`): `DeployDef` wants a material byte (`CONTENT.md`).
3. Flesh is heard attacker-side only (§0pvp item 2); no cloud by choice, no mark by design (§0mk).
4. Sparks and grit bounce once; dust still passes through walls: a collision query, once a person has looked.
5. `RemoteSwing` (body) and `ImpactWood` (trunk) are two unlinked cues; one sound per blow is a later call.

## 0x · The client makes sound — what it cannot yet hear *(client lane)*

1. Nobody has heard it (`ART.md` has no audio section): `cargo run -p client --bin soundbank -- <dir>` writes
   every cue to WAV, or open the web page (`ci/build_web.sh`). Sourcing: `assets/sound/WANTED.md`.
2. The score is programmer art (`synth::score`); recorded pieces swap in at `synth::render`'s music arm. Two
   bumps we cannot take: weapon equipped, projectile near-miss.
3. The device path is ungated: cpal opening, the callback and the real rate need a person booting the game.
3. `--capture` by hand is the only proof most audio systems run; gate world-free ones the `tests/music.rs` way.
4. `UiClick` exists only as the mixer's placeholder `Request`; it wants a hook in the per-screen click handlers.
5. No occlusion: it needs the sim's geometry query (`collide.rs`), not a raycast against render meshes.
6. Crickets: a night-gated `Cue`, the bird layer with the predicate inverted (`is_day` in `render/audio.rs::bed`).

## 0x · The native client — the feature trim and the dropped anchors *(client lane)*

1. Trim bevy's default features (`crates/client/Cargo.toml`): `bevy_gilrs` (drops `libudev`), `vorbis`, `wav`,
   `bevy_audio`; keep `bevy_gltf`/`bevy_animation`, x11, wayland, `alsa`. Needs disk headroom and a looked-at
   `--capture`: a missing decoder draws white, so a green compile proves nothing.
2. World-space anchors are unbuilt (the wall's number at the wall, a clock on the charge mesh); `charge_deploy`
   is unread and `stock_addr` is read nowhere in `crates/client/src`, so nothing says which hearth. None is blocked.

## 0z · The Bevy-draws rule's missing gate *(client lane)*

1. R-G4: nothing gates the no-gameplay-state-in-the-ECS rule; build the renderer-attached vs detached
   state-hash equality (`RENDER.md` §5, line 889) under `crates/client/tests/`.
2. Nothing photographs the wait (`render/capture.rs::PLACE_FRAMES`); seeing it is §0p2 item 3's viewer.

## 0v · Players are people — what the rig still cannot say *(client lane)*

1. Crouch, jump, swim are wired to nothing. Jump: copy `grounded`/`qvy` onto `RemoteState` in `interp::dequant`
   (no wire work). Swim: no sim fact, only approximately derivable. Crouch: an S→C bit + `PROTO_VER` (the
   `BINDS` row already says it is a sneak).
2. Remote bodies hold nothing (`render/bodies.rs`): attach the held mesh to the rig's hand joint, no new art.
3. Feet slide between the clips' speeds (`_RM` variants unused): scale playback rate to speed, an unmeasured knob.
4. No worn-steel albedo: the axe head has no map (`render/viewmodel.rs`, `assets/textures/MANIFEST.md`).

## 0p2 · What the UI still owes *(client lane)*

2. Repair's exact price is not on the wire; the hammer names full hp (`findings/building-tools-20260920.md`).
3. Panel viewer (never a pixel gate): open each panel against a stocked fixture, write PNGs; `--capture` can't.
4. Font scale: fourteen sizes want a deliberate pass; a five-size cut clipped columns at 720p — not blind.
4c. Quick-move takes one slot per click; whole-stack scatter and hover-loot need a `take all` verb argued first.
5. Surveyed and refused: `bevy_hui`, `bevy_lunex`, `bevy_feathers`, the freegameui.net MCP.

## 0cq · The craft panel beside the reference's — pictures, words, and the closed menu *(client lane)*

`reference/CRAFTING.md` §9.1 ranks twelve gaps; #181 closed 1–5 (the HUD craft bar, picture queue with
its countdown, padlock, notices over the vitals, colour icons).

5. CRAFT dims when short; the community plugin paints it green — a palette knob, `DECISIONS.md` §open.
6b. 8 of 62 icons are 3D renders (`iconbake.rs` `SUBJECTS`); thin tools render as hairlines, so the rest are
   painted silhouettes (`ci/finish_icons.py`) until chunkier models land (§0hand items 1 and 3).
7. Then one `PROTO_VER` turn (the class byte, §0w item 1; a description column), then two sim verbs:
   fast-track by task id (§1.1/1.4) and the bench rebate (§0tt).

## 0w · The native menus — the rail and the untested gesture *(client lane)*

1. The rail wants a class byte per item in `EventMsg::Catalog` (`ui/craft.rs:14-28`): `PROTO_VER` + goldens together.
2. The drag is gated as arithmetic only (`tests/ui.rs` §B); press → ghost → release → send is by inspection.

## 0v · The menu flow — the served list and the untested hangup *(client lane)*

1. Nothing diffs the served list (`GET /api/launcher/servers/gates`) against `shards.toml`; `ci/shardlist.py
   --self-test` is offline by design, and `ops/certbot-deploy-hook.sh` covers only the certificate.
2. Ungated, by hand only: killing the shard mid-play into `Screen::Disconnected`.

## 0pw · Skinned meshes still specialize on arrival *(client lane)*

1. Skinned meshes are a different pipeline key: the first remote player in view still specializes on arrival
   (a pop). Named in `render/prewarm.rs` lines 52–57.
2. The pipeline count is unasserted (`PipelineCache::pipelines()` needs a GPU); `tests/prewarm.rs` gates the ECS side.

## 0pf · The client's CPU frame — four measured leftovers *(client lane)*

1. `ground_slope`'s four taps are ~80% of a tile; the stencil moves every splat byte — a design change + golden.
2. `water::animate` deep-clones ~677 KiB per frame (`Assets::get_mut`; `examples/frame_cost.rs`): the fix is the
   vertex shader `render/water.rs` §57 names, after a GPU boot.
3. Under 50 µs together: `verbs::resolve` (use the 3×3 `ColIndex`), the `bodies`/`mobs::stream` slot scans,
   `audio::fell`'s `GlobalTransform` fetch, `hud::update`'s strings, the ring streamers' full-map probe.
4. Sea tangent `w` `-1` (`water.rs:947`) vs ground `+1` (`terrain_mesh.rs:711`): one flips the ripple green. Look.

## 0u · the frame budgets are browser numbers and nobody has re-derived them

1. < 300 draw calls / < 1.5 M tris are WebGL-shaped; rationed against them: `CLUTTER_RICH_PER_TILE = 96`
   (`sim-core/terrain.rs:2919`) and the conifer ring's 1.9 M verdict.
2. Nothing measures native cost (no `RenderDiagnosticsPlugin`): on a real GPU at the ring's p90 tree count read
   draw calls + frame time (floor: 60 fps on a mid laptop iGPU); propose into `DECISIONS.md` §open, operator renumbers.
3. `BASE_ANISOTROPY_MAX = 4` was a software-rasterizer choice that does not transfer (now only a comment at
   `client/src/render/textures.rs:60`).

## 0p3 · Photographing a panel — the screen the recipe cannot reach *(client lane)*

- Site recipe (never a gate): `terrain::haven(seed)`/`haven_shelter`/`waystation_canopy` → `shard.toml`'s `dev_spawn`;
  `Xvfb :9 -screen 0 1280x720x24 &`, the shard, `VK_DRIVER_FILES=/usr/share/vulkan/icd.d/lvp_icd.json DISPLAY=:9
  WGPU_BACKEND=vulkan target/release/gates --server 127.0.0.1:4433 --capture <dir>`. Shots face N/E/S/W: stand opposite.
- Owed (§0p2 item 3): `render/capture.rs` knows only `Player`/`Build`; a viewer should open each panel against
  a stocked fixture and write a PNG per screen.

## 0vj · The capture probe ships frames with no record of what went wrong *(harness lane)*

- `crates/client/src/render/capture.rs` writes PNGs only; the visual judge's prompt wants a `manifest.json` of
  what the client logged while shooting.

## 0bd · The tree blocks 0.3 m of ceiling nobody draws *(client+sim lane)*

1. `OCCUPANT_TOP_M[Tree] = 5.7` (`terrain.rs:4245`) cites dead `PINE_TRUNK_H` (`render/props.rs:45`), not the
   drawn tree (`render/tree.rs:127`), and `greybox.rs:210` excuses it. Bound the drawn mesh's height band in
   `tests/tree.rs` and take the number off it, as for the barrel.
2. `assets/models/WANTED.md` §2.8 briefs the loot barrel at a retired `0.9 ⌀ × 0.95`; the gate holds 0.585 × 0.88.

## Numbers, worldgen and the arc *(content + world lanes)*

## 0b · Balance — the reference rows still outstanding *(content lane)*

- ⚠ Derive the raid ratio (`Content::load_dir(…)` then `.anchors()`), never quote it.
- Operator rules (2026-08-10): our band yields to their number by default (`BALANCE.md` §6.5); a number absent
  from `RIPLIST.md` is not thereby decided. `reference/RIPLIST.md` §2 is the queue: read it first.

1. Next unblocked: **1g**, the research ladder's per-item ordering (`READY`, page tier); settle §1f's era first.
2. Blocked, numbers written: **1j** `armor.toml`, re-anchoring `content/tests/content.rs::band_breaks_refused`,
   best inside equipment v0.
3. No per-material damage resistance: one `structure` column (`content/src/schema.rs:281`) compresses row 2.
4. Gather yields, smelt and craft times are still ours; per-hit yields and sub-second precision (row 3a) are schema work.
5. Logistics friction (~10–30×) beats mob→player damage (~2–5×): threat is trip shape, never a multiplier (rows 5, 6).

## 0n2 · Monuments — the depot is the first kit; the roster still needs variety *(world lane)*

Read `reference/MONUMENTS.md` §9 first (§0: the weakest provenance here).

1. Next: quarry and relay kits with distinct terrain needs, then the roster together. Re-derive placement (600 m
   separation, 300 m inland search; `INLAND_SITES` alone can't) and rewards as one (caches 4 vs Haven's 5; the
   depot has no loot or guards). Coast-ring continuity is §0rd.
2. Arrows pass through every deployable: `sim-core/src/ranged.rs` never asks the solid nibbles.
3. Whether a sleeper blocks is unanswered (§0y item 1) — a design call.
4. Art rows (`DECISIONS.md` §open): the shelter's posts stand 1.2 m proud of its roof; swept ground reads as shards.
5. Then §9.4: per-entity interest ranges (nav landed, #178); vertical AOI layers are premature, moving monuments refused.

## 4b · The world lane: what the second tier left open

1. An authored worldgen deployable: a `DeployRec` no player placed, restart-safe and immune to `pick_up` (owner
   `0` is reserved, `sim-core/src/world.rs:1611`). Systems lane. Bank and vendor stay blocked on an operator act.
2. Nothing threatens the walk between sites (guards leash to a `SiteFootprint`). Nav landed with the
   animal brain (#178: A* on a 1 m grid, round walls, trees and cliffs).

## 7 · Milestones — the arc is `DESIGN.md` §11; the queue adds two gates and one item *(systems lane)*

The arc is `DESIGN.md` §11 (M0 landed → M1 → M2 → M3 → M4); `ALPHA.md` §6 folds into it.

- **A1 playtest** (operator schedules): 10–20 testers, one wipe cycle, after M3 and before A2/A3 arming
  (`ALPHA.md` §2); a loop proposes it, never runs it.
- **Arming A2, then A3** is an operator act.

1. **Anti-ESP occlusion culling**, server-side (none in `crates/server/src/interest.rs` or `sim-core`): the grid
   is a pure function of the seed — bake at worldgen, look up in the tick. After M2.
2. **`elo_overlay.rs` has no upstream-drift gate** (drift once broke every login for eight days): its `sha256sum`
   must be in `scry-forge`'s `sdk/SHA256SUMS`, never the `scryward` mirror. Build a nightly fetch-and-compare;
   derive the launcher's real state from elo, never from this file.

Standing rule: anything a playtest breaks jumps this queue; anything a wall catches jumps the playtest.

---

# OP · the operator lane — a loop cannot pick any of these

## LOOK · Questions a frame settles, waiting on an answer *(operator)*

The operator plays the game, so "unseen" only means no answer is recorded: an
entry whose answer is in `DECISIONS.md` is done — check there first. A look is
never replaced by a pixel gate.

- Ore node beside a boulder (§0rk): node vs boulder at 10 m unprompted? metal seam glint under the game's light? sulfur crust vs paint? does a hillside node float on its downhill edge (expected at today's 0.5 lift; the proposed 0.3 lift is the fix, `DECISIONS.md` scatter art v1)?
- Depot, looking down its road (§0rd): does the 25–57 m wander go somewhere or look nudged? gate approach square-on at the apron? far end ever hidden by terrain (the case for more than `SIDE_ROAD_BEND_M = 60`)? No-GPU first look: `cargo run -p client --example map_png`.
- Smash a barrel (§0wc 2b): a loose sack reads as loot, not debris? findable in grass? smaller than a death bag at 10 m? two stacks told apart by the prompt alone?
- Open a bag and right-click (§0p2, §0wc, §0eq item 5): the inventory page with a container open is body + pack + container in one row (~950 px at 58 px slots) — does it read, and does it fit at 1280?
- One tree, then a stand (§0t): needles, not fern fronds? dark inside, lit outside? lit top, shaded underside? too much sky through it (fix: card counts in `tree.rs`)? The broadleaf's 11 cm leaves separately (§0t item 7).
- Hold G (§0mp): road casing or scratch? 256 grid labels an index or a mesh? 18 px badges blob a base's beds? site names collide? No-GPU island half: `cargo run -p client --example map_png`.
- Go down (§0wnd, `render/wounded.rs`): the drop to `CRAWL_EYE_M` and roll, vignette, two-number line, a remote body's fallen pose sliding at a crawl; checklist `reference/WOUNDED.md` §9.5.
- The ground (§0gs; a defect, not taste): new `rock`, macro break-up, the >45° biplanar tap (never compiled here); does litter's 1.3 m repeat read as a lattice (`ART.md` rule 7) or does `MACRO_M` dissolve it?
- The ranges and the summit slab (§0mtn, §0gs item 3): stand on the shipped seed at `776,1392` (103 m, flat; `./ci/scene.sh --spawn 776,1392`). Does `Rock032` read as rock at your feet and along the summit, and do the ranges read as mountains from the lowland?
- Worldgen (§0wg; could find a regression): re-shoot the operator's terraced-mountain screenshot — `remap`'s monotone cubic and the detail ladder moved the ground under every prop, tree and clutter tile.
- Night, torch in hand (§0tl; `ci/scene.sh --hour midnight`): does the 600 lm pool at 0.89 m read as carrying a light, and the torch head as its source?
- A wall under arrow fire (§5.1): do the mark, hp readout and collapse arrive together and read as a raid?
- Chop a tree, hit ground and a wall (§0mk): do #179's atlas marks read per material — holes, gashes, dents, scorch?
- Weather and night (#176): storm, fog, rain, dusk, stars and moon on a real GPU and in a browser, and the rain and thunder beds. Does a roof read as shelter (rain cleared, wind and rain quieter)? `ci/scene.sh --hour … --weather …` pins a frame.
- The animals (§0pr, §0m, §0anim): a pig asleep lies down; a pack circles both ways; an animal shot at range turns on its shooter. Does any of it read at 30 m?
- Sea vs ground ripple (§0pf item 4): tangent `w` is −1 on the sea and +1 on the ground for the same XZ mapping — which flips the green channel? Look, don't guess.

0. The blow, whole (§0fx, §0mk): one whoosh per arm swing when spammed? pick sparks a shower or a firework? dust weight or smoke? contact thock/crunch/clank; the weak-spot cross brightening on `WEAK SPOT` (numbers: `DECISIONS.md` §open). And #179's blood, blast, muzzle flash and tracers, and its recorded Kenney takes against the synth.
1. A remote body's swing (§0sw): never drawn; a clip-table array width could panic on the first nearby swing.
2. A body falling (§0chr): kill something and watch `Death01`.
3. The flinch and remote swing sound (§0pvp 1–2), the hurt arc (§0hrt 4), the three hitmarker rungs (§0hs 3): live combat only; does the *limb* cue sound unlike a miss?
4. The audio bank, nine cues of it music (§0x, §0pr): `cargo run -p client --bin soundbank -- <dir>`.
5. LOW and MEDIUM (§0gq): walk the knob down — is MEDIUM still the game?
6. The far forest at 80 m (§0lod): does the opaque hull read denser than the near tree? That, not popping, is the defect.
7. The broadleaf (§0t item 1): crown spread and leaf count/size are the likeliest wrong.
8. The announce stack (§0tq), live play only (`--capture` can't force five facts): does the 0.52-alpha deepest row read? does `…+N more` shift the sentence?
9. The tech-tree panel at a bench (§0tt, §0tree): press `E`.
10. A world crate and a site guard (§0wc 1, 4): `dev_spawn` puts the camera at the pad (§0p3 has the command).
11. Freehand build and the aimed band on a hillside (§0bl items 5, 8): height changing across one cell — control or twitch (`R`/`F` step it)?
13. The collapsed off arm and the sleeper tint (§0chr item 6), a spill line (§0sp2), the map's marked set (§0a), a diagonal base (§0ac item 3), the clutter ring's hard edge at ~32–45 m (§0a).

- Needs a machine, not a look: the Windows build on Windows (§0win).

## 0gq · Nobody has seen LOW or MEDIUM *(client lane)*

1. **Operator: walk `config::Quality` down and look** (`render/quality.rs`): the
   order is gated, where each rung sits is the call. Is MEDIUM still the game?
2. Clutter and prop rings (which tiles exist) remain separate streaming work; no
   tier may cross `ART.md` rule 4.

## 0sun · Noon southwest — one call the operator has not made *(operator)*

2. **Operator: keep noon southwest** (`RIG_SUN_AZIMUTH = 2.35`, SE → SW → NW)?
   Moving it retires every judged frame (`DECISIONS.md` §open "sun arc v0").

## 0die · Two calls the operator still owes on the death screen *(operator)*

1. **Operator: should a player pick the bed they wake at?** `ActionMsg::Respawn`
   carries one bit (`on_bag`) and `World::wake` takes the nearest ready bag; a yes
   is a bag index on the action plus a `claim_bag` that honours it (wire bump).
2. `SUB_BAGS` is sent only on death (`server/src/core.rs`), so `ready` ages on the
   screen; re-send on bed placement/removal if that starts to matter.
3. **Operator:** is five minutes the right floor for a common-only bag now the kit
   guarantees one? (`DECISIONS.md` §open "death backpack v0")

## 0mp · The map became a chart — what it still is not *(client lane + operator)*

1. Player-placed markers, the real gap: five each with colour, icon and label,
   team-shared (`reference/MAP.md` §3); needs a wire message and a cap (`protocol` lane).
2. Clock/radius markers (§4): nothing uses one yet; copy `MarkKind::BedSpent`'s
   weight first.
3. **Operator:** toggleable grid labels? The reference ships them off (§1).
4. **Operator:** should waystation and inland look different? They share a word
   and glyph (`resolve_marks`); a yes is an icon.
5. `MAP_PX` 1024 for crisper roads paints in 1.06 s vs 263 ms (hidden behind the
   loading bar by `prepaint`) — a trade, not a free knob.
6. Unseen — §LOOK (hold G).

## 0a · Is the map's marked set the right one? *(operator — a taste call)*

1. **Operator:** is `MarkKind` (`client/src/ui/map.rs:263`) the right set — haven,
   waystation, bed, spent bed, hearth, backpack, with boxes and doors unmarked on
   purpose? Look with the game booted (§LOOK 13) before that ships.

## 0v · The furnace's ore rows want an operator's number *(systems lane)*

- **Operator's number:** the furnace's ore rows (`content/recipes.toml:362–384`)
  are station crafts; making them `sim-core/oven.rs` conversions (the reference's
  `BaseOven`) re-prices the powder chain against `CONTENT.md` §4 — a balance pass.

## 0rn · The rename's six loose ends *(operator, mostly)*

1. **(operator, wallet)** Re-sign `scry.json` at seq 6 (key on morr): `_next`
   answers 410, `_version` names `ci/scry_manifest.py` (now `ci/elo_manifest.py`).
   Hands off the signed bytes; names follow `/api/library/GAME-REPO.md` on the day.
3. **(operator)** Coins are being redeployed; `/api/onchain`'s SCRY/OBOL/MYRRH is
   the outgoing set. Never type a new-coin number here — it's `scry-forge`'s copy.
4. The launcher should accept `elo-shardlist-v1` (`ci/shardlist.py`) before the
   next publish.

## 0rep · Where a filed report goes, and what it pays *(client lane + operator)*

1. **Operator: where are reports read?** Only `ci/reports.py` prints the board;
   no page serves it.
2. No intake, on purpose (the client opens no socket); an endpoint is its own slice.
3. The launcher lacks a `report` signing family, so `report.rs::sign_text` is
   refused. Add it upstream (`scry-forge`), re-vendor; unblocks (4).
4. **Operator:** how much a report pays against a PR's 100,000, and on merge or
   earlier (`DECISIONS.md` §open "bug reports v0"); the rail is built.

## 0dsc · Discord presence is dark until an application exists *(operator — one act)*

1. Create the Discord application named `Gates` and set `GATES_DISCORD_APP_ID` —
   lights up the built presence (`crates/client/src/discord.rs`).
2. Register `elo://` or `gates://` in the portal — Ask-to-Join for a friend not
   running the game (`deeplink.rs`, no code).
3. Optional: a 512² or 1024² image under asset key `gates` (no Gates mark exists).

⚠ The detectable-list submission is unverified (no form found); nothing depends on it.

## 0win · Nobody has started the Windows build on Windows *(operator)*

1. **Start the depot build on a real Windows machine** — wine in `nightly.yml`
   proves only the loader; a failure past `loader_init` is a new item.
2. Does the msvc GitHub release zip need the VC++ redist? `release.yml`'s notes
   say nothing for Windows.
3. Not ours: elo's launcher manifest on morr says the Windows row bundles nothing
   (three DLLs ship); fix it on morr — no `scry.json` re-sign can.

## 0rl · The release path — two operator acts, and a tester's question *(platform lane)*

1. **Publish the newest draft release** after reading its assets; only `v0.2.0`
   is published (check the API, not this line).
2. **Raise `min_client` on a live shard only after (1)**; a climbing
   `refused_build` means it was done backwards.
3. **Tester:** start the macOS and Linux release artifacts — never run; CI only builds them.

## 0ab · The store seam — what only an operator can finish *(platform lane)*

⚠ `scry.moreright.xyz` anywhere in prose is dead (410); the platform is `elopros.com`.

1. **Operator, every release: publish** — live once the origin's `published.json`
   names the build and its digest is notarized (`elo digest` can't run here).
2. Nothing measures a join on demand: `bots` dials a `SocketAddr`, not the cert's
   name (`server/tests/tls_posture.rs`), and has no wallet; a person must join.
3. `elo://` isn't registered with the desktop — the launcher installer's job
   (`crates/client/src/deeplink.rs` is ready).
4. **Operator:** on a `shards.toml` row change, re-run `./ci/shardlist.py` and
   copy `servers.json` to the origin.

## 0ad · The ticket door waits on a deployed contract and a spoken sweep *(platform lane)*

1. **When `ScryGameTicket:GATES` is deployed**, drive the door (`tickets.py`) with
   one wallet that owns a copy and one that doesn't; until then all are `entitled: true`.
2. **Operator: speak the sweep interval** — `DEFAULT_SWEEP_SECS = 120` is how long
   a sold copy keeps playing (`DECISIONS.md` §open "ticket door v0", PROPOSED).
3. No `prove` call site (`crates/client/src/elo.rs`): every join costs a consent
   dialog. `Overlay::prove` means the server parses the launcher's EIP-4361
   message, carried on the wire — `PROTO_VER` bump + goldens; a slice.

## 0sl · The shard list reaches the game — two operator acts, in order

1. **Ship the launcher** with `servers` in `ARG_VARS` (`scry-forge`, `launcher-rs`)
   first — an older launcher refuses a depot that uses `{servers}`.
2. **Then re-publish the depot** with `--servers {servers}` (`python3 ci/depot.py`,
   elo `docs/client/LAUNCHER.md` §8): fills the empty in-game browser and drops the
   dead `scry.moreright.xyz`. Meanwhile: `--servers <url>`.

## 0s · The front door — the two acts that are not ours *(client lane)*

1. **Operator: menu backdrop motion and vantage** (`DECISIONS.md` §open "menu
   backdrop v0"). A loop is a frame sequence (~4–12 MB for 3 s at 720p/20 fps); a
   better still is one `--capture --no-hud` run.
2. **Platform:** add `news`/`store`/`workshop` beside `servers.url` in
   `data/launcher/gates.manifest.json` — lights the hub's links (`ui/hub.rs:183`).
3. Star, search, filters and OPEN IN LAUNCHER were only driven headless
   (`xdotool`), never against a populated list or a live launcher.
4. The splash can't cover its first ~3 s; a second process could — not taken.

## 0web · The browser client — **spoken; transport, identity, the renderer, a sky, models and a playable page DONE in headless Chromium; a real GPU and a real wallet next** *(client lane)*

History: `findings/web-build-20260909.md` (§17 newest). ⚠ WebTransport over HTTP/3
is a browser's only path — never drop that layer (struck §0wt).

1. **A real GPU:** sky colour, retina stretch and load time are unmeasured on
   hardware. (A person's wallet first signed in live on 2026-09-13;
   `protocol::SIGN_WAIT_SECS` is 60.)
3. `wasm-opt -Os` grows the gzipped module (8.57 → 8.97 MB); `build_web.sh` runs it
   only if one is present or named by `WASM_OPT`.
4. Where the 2048 cap binds, `stretch` magnifies the UI; a DPR change mid-session
   costs a UI-scale change (`web.rs` header).
5. No aerial perspective or sun disk in a browser (dusk is coloured, `DUSK_GLOW`); a
   dome shader is next if the sky reads flat on a GPU.
6. Wasm heap ~700 MB (findings §17.6): every image keeps a main-world copy; first
   cut is `RENDER_WORLD` for model maps and photographs (`web::heap_report`).
7. **The worklet is unheard and unmeasured in a tab:** read `app.js`'s long-task
   counter beside `web::heap_report` (`findings/browser-audio-20260913.md` §4). Also
   open: Firefox, Safari, `REPORT_EVERY = 96`, `Diag::stats.dropped` vs frame time.
8. Headless pointer lock floods ~30k events/s into Bevy's unbounded buffers; if a
   real browser ever does, cap it in winit's coalesced-event loop.

- A browser's sun is pure white (no transmittance), so its lit ground reads cooler.
- The launcher relay refuses SIWE (`meter/signin.py::_guard_ask_text`): a key
  held only in the desktop launcher has no web door.
- Nothing gates that `wtransport rev = a11e6a8e…` holds the #317 fix; the pin is permanent.
- Both builds draw marks as one batched atlas mesh (`render/decal.rs`, #179), so a
  capture can photograph them; nobody has looked (§LOOK 0).
- **Operator:** publish with `ci/publish_web.sh` (served by
  `scry-forge/deploy/nginx/elopros.com.conf`).
- **Operator:** Cloudflare doesn't cache the 8.6 MB module; a cache rule needs a
  purge-on-publish beside it, and no box has the credential.
- After a handshake change run `slow-signer.mjs` beside `smoke.mjs`. The harness
  (`~/gates-browser-smoke/` on morr) is SwiftShader at ~0.8 fps — look at a frame
  before trusting a short run; each run leaves a sleeper.

## 0wd · A new world register is proposed — blocked on the operator's word

- **Operator: speak `WORLD.md`** (row in `DECISIONS.md` §open) — unblocks the
  cheapest slice, a radial third input to `biome(h, moist)` (`terrain.rs:497`) + goldens.
- **Operator, three acts:** palette, reference set, rubric style section; until
  then `ART.md`'s bar scores an obsidian world as a defect and no visual pass chases it.
- A ward would break `CONTENT.md` §4 anchor 2 without reddening `test_content`
  (TTK uses `balance.toml:13`'s `player_hp = 100`).
- Extraction and world states: one system or two? Decide before a bespoke gate
  (terminal at A2, `ALPHA.md` §2).

## 0gh · The GitHub job-agent seam — the acts still owed *(operator lane)*

- **(operator, GitHub)** Protect `main` requiring the `gates` check (until then the
  merge gate is policy); add a same-named no-op for paths `gates.yml` filters out.
- **(operator, once)** Settle `gates-pr` on the next accepted PR: public transfer,
  row appended elo-side.
- **(operator, GitHub)** The About homepage still points at dead `scry.moreright.xyz`.
- `scry.json` has no `jobs` key, so this repo posts no board lane (rows stay house-side).

---

## Labels · closed labels and ambiguous citations

Code cites `NOW.md §<label>`, sometimes with an item number. Read a citation as
a hint and match on the title.

**Closed** — the section is gone; `git show 9a069f4:NOW.md` has its last text:
`§5c` and the old `§0kit` (rock, doors and boot rule; the label now names the
build-kit section), both 2026-08-25; `§0lod`, `§0sw` and `§0tq`, folded into
`§LOOK` 6, 1 and 8; the ghost's door-preview `§0u` (closed in `2e5f500`; code in
`ui/place.rs`, `render/ghost.rs` and `render/structures.rs` still cites its items
1–3 — the `§0u` here is the frame budgets); `§0wt` (the HTTP/3 layer, struck — its
warning lives in `§0web`), 2026-09-24; `§0shot` (muzzle flash, tracers and the
distance low-pass landed in #179; last text `git show 1e046dc:NOW.md`), 2026-09-25.

**Retitled 2026-09-24**, same label: `§0mk`, `§0tt`, `§0tree`, `§0gc`, `§0rk`.

**Item numbers that moved**: `§0web` items 3, 5, 6 and 8, cited from
`client/tests/mipmap_retouch.rs`, `client/tests/bank.rs`,
`client/src/webassets.rs` and `client/src/render/audio.rs`, name work that has
landed; those numbers now point at other items. `ALPHA.md` cites the wipe as
"§0q item 2"; it is item 3. Doc comments in `sim-core/{deploy,claim}.rs` cite
"§0aa item 1" / "items 1–2" under numbering that has since moved.

**Ambiguous** — these resolve to two or three sections:

| label | resolves to |
|---|---|
| `0a` | the clutter ring's fade *(client)* · the island's map *(ui, operator)* |
| `0v` | the furnace's ore rows *(systems)* · players are people *(client)* · the menu flow *(client)* |
| `0w` | the props' gaps *(client)* · the native menus *(client)* |
| `0x` | the client's sound *(client)* · the native client's trim *(client)* |
| `0y` | the sea *(client)* · persistence *(server)* |
| `0z` | the Bevy-draws rule's gate *(client)* — doors is `§0zd` |
| `4b` | the world lane *(world)* · the domain gate *(platform)* |

**Renamed**: doors and locks `§0z` → `§0zd`.
