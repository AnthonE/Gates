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
>
> **Pruned again 2026-08-09: 3,839 → 644 lines, 55 → 30 sections.** That
> day's ~15 landed slices were struck in place and are deleted here; the §8
> un-reconciled block and every 2026-08-05 "done this pass" item went with
> them, and the stale triage sections (ranged, jump, the browser gates) were
> verified against the tree before deletion — ranged landed 2026-08-06,
> jump is pressed natively, the `loop/*` branches and browser gates are
> gone. Five live gaps were lifted out of done items before they went: §0q
> gained two (standing ON occupants, the 100-bot soak) and §5 three (the
> invisible arrow, mushrooms/corn, day/night) — the same rescue the first
> prune performed.

---

## 0b · Balance sits on the reference's numbers now — what is still off *(content lane)*

Landed 2026-08-08 (operator: *"balance the game similar to rust so people
dont get too lost"*). `reference/BALANCE.md` is the research and §6 is the
standing instruction. Building blocks are 250/500/1000, a stone wall takes
four satchels, tool and melee damage are theirs, the pig is a 150-hp boar.
Two bands moved and the raid ratio re-priced itself to 1.04/1.73/3.46.

**The measurement landed 2026-08-09 and `reference/RIPLIST.md` is now the
queue for this item** — what is taken, what is outstanding, what blocks
each row, and the six steps for executing one. Read it before touching a
balance number; do not re-derive that list here.

Two results worth carrying at this level. `balance.rs` refuses a
`farm_per_min` above the sim's at-node ceiling and `tests/farmwalk.rs`
measures **969 wood/min, 71.6% duty**, ~19× the declared 50. But that
gap is a **debt owed by the world, not an error in the number**: their
ladder falls ~30× from at-node to real farming with no threat in it at
all, ours charges 1.40×, and applying their decomposition to our ceiling
puts the declared 50 inside the band. So the queue's ranking inverted —
**logistics friction (~10–30×) outranks mob→player damage (~2–5×)**, and
threat wants modelling as trip shape, never as a rate multiplier.

**Two gather mechanics are theirs as of 2026-08-09** (operator: the mark
must buy speed, not yield, and we need the finish bonus). A node's payout
is invariant at `hits × per-hit`; the glint spends its budget faster (a
tree falls in 7 swings instead of 10 for the same 300), and 20% of an ore
node / 50% of a tree is withheld for whoever lands the last swing. Gated
in `sim-core tests/gather.rs`; no wire byte moved.

Still wrong for a returning player, in rank order, all of it detailed in
`RIPLIST.md` §2: the boar does not fight back; no per-material damage
resistance (one `structure` column, so the ladder above stone is
compressed); one animal; and gather yields, smelt and craft times are
still ours — node totals are `READY` now, per-hit yields are not, and our
schema does not need them. Upkeep, decay and the armour ladder differ on
purpose (`BALANCE.md` §4.1), though the upkeep *rate* turned out to match
theirs.

---

## 0ab · The store seam — what the SDK re-vendor and the depot job left *(platform lane)*

Landed 2026-08-09. The vendored SDK was **326 lines behind upstream** with
every gate in both repos green: no Windows transport (`std::os::unix::net`
was imported unconditionally, so a Windows build of this client could not
compile), no `prove`, no `profile`. Re-vendored and re-pinned; upstream now
publishes `sdk/SHA256SUMS` and gates its own rustfmt-cleanliness, so the drift
check is one `sha256sum`. `nightly.yml` now builds `--features render` and
runs `ci/depot.py`, so the depot is a CI artifact instead of one box's output.

What remains, in order:

1. **Nothing here publishes.** A build goes live when the origin's
   `published.json` names it and the digest is notarized — operator acts,
   both. The nightly artifact is the tree those acts consume.
2. **The shard list is written and never served.** `ci/shardlist.py` produces
   the document and both consumers parse it, but `manifest.servers.url` on
   scry's side is `null`, so the launcher's Servers window and our own menu
   are dark for the same missing file. Everything downstream of that one
   publish now exists: live counts via `status_url`, and join links
   (`scry://join/gates/host:port`, `deeplink.rs`). Registering the scheme
   with the desktop is the launcher's installer, and is not done.
3. **`prove` has no call site.** `sign_siwe` hands the launcher a string this
   process composed; `Overlay::prove(server, nonce)` binds it to a name and a
   nonce the SHARD chose, which is the difference between a signature a shard
   can verify and one it can replay. Wiring it is a wire change (wall 6), so
   it waits for the identity-in-handshake slice, not for this one.
4. **The depot is Linux only.** `ci/depot.py` says so in its first line and
   scry's platform enum has the other rows. The SDK can now reach a launcher
   on Windows; nothing packages a Windows build of this game.

---

## 0aa · Building rights — what the four slices left standing *(systems lane)*

Landed 2026-08-08/09. `reference/BUILDING.md` is the research; the rows in
`DECISIONS.md` §open (hearth crew v1, privilege v1 + the claim cache,
demolish v1, upkeep/decay v1) are what was built — coverage asks the base's
own cached volume now, not a circle. What remains:

1. **No `AutoTurret`, so the roster has two customers and not three.**
   `roster.rs` exists because the reference has four; ours has two.

---

## 0z · Doors and locks — settled *(systems lane)*

Landed whole, 2026-08-08/09: `reference/DOORS.md` is the research,
`sim-core/lock.rs` the answer, `DECISIONS.md` §open "lock v1" the slice.
Locks on boxes, the pickup tier (a GUEST works the leaf and cannot lift),
and the keypad panel all followed. Nothing remains here.

**Not owed, and stated so it is not re-litigated**: the key lock (its keys
need per-item instance data `ItemStack` has no room for, and it is the
system the reference abandoned in Devblog 193) and door tiers past wood and
metal (a content row, not a mechanic).

---

## 0y · The sea is a volume — what it still cannot do *(client lane)*

Landed 2026-08-08: `render/water.rs` (eye-centred mesh, four-wave swell with
analytic normals, per-channel optics, shore foam standing off the waterline),
`terrain_mesh::wetted`, and `sound/water.rs`. Research `reference/WATER.md`;
knobs `DECISIONS.md` §open "water v0" / "water audio v0" (the §open row also
holds the five defects found by LOOKING, not by a gate). Gated by
`tests/water.rs` (22) and eight assertions in `tests/sound.rs`. Remaining:

1. **The last hard edge needs the depth prepass, and that is the next
   slice.** The alpha ramp is a *vertex* quantity off `terrain::height`, so
   it fades against the terrain and not against anything else — a boulder, a
   foundation or a player in the shallows gets a ring. The fix is standard:
   sample the depth prepass in the fragment, fade alpha and add foam as
   scene depth approaches the water's own. Needs an `ExtendedMaterial` and
   the **first WGSL in the tree** (`RENDER.md` §8); SSAO already puts a
   depth prepass on the camera, so the input exists.
2. **There is one sea state and no weather.** A storm is `WAVES` scaled by a
   scalar the sim would have to publish — wire, not renderer.
3. **Nothing reflects.** `reference/WATER.md` §5 says reflections are the
   expensive half and §6 says the payoff is the *sky* — which the
   atmosphere's specular already gives. Read both before starting.
4. **Underwater is audio-only.** A colour grade under the surface is a
   second owner of the frame's haze (`CLAUDE.md`'s coupled-lighting law); it
   wants the lighting owner, not this lane.
5. **The submerged duck is not a filter.** rodio gives gain, rate and
   panning; a real low-pass needs a DSP node. Stated in `SNAPSHOTS`.
6. **`Splash` is the only producer of the waterline.** No stroke, no wake,
   no interactive deformation — the reference merges an interactive sim into
   its own displacement (§3) and we have no producer for one.

## 0m · The pig is in — what the roster still owes *(systems lane)*

Landed 2026-08-08/09 (operator: *"let's get a pig in"*): 64 fixed roster
slots, homes from the seed, staggered think, dormancy at 240 m, a leash, a
flight, a corpse bag looted with E, a hashed snort, a distance-integrated
trot — and the kill→fire→meal loop closed with the oven (§0v below): four
content rows, gated by
`content.rs::the_kill_the_fire_and_the_meal_are_one_loop`. No navmesh: the
terrain is a pure function, so the animal steers and `movement::step`
decides. Research `reference/ANIMALS.md`; calls `DECISIONS.md` 2026-08-08
and §open ("pig voice v0", "pig gait v0"). Owed, in rank order — §9.5 has
the reasoning:

**Three defects were found by booting the game and looking, and every gate
was green through all of them** — which is what `CLAUDE.md`'s "the operator
boots the game and looks" is for. (1) `flee_pct = 100` made the pig run at
exactly the player's sprint, so it could never be caught or melee'd; now 70,
and `tests/content.rs` gates `flee_gait < 127`. (2) The massing wore
`props::tint1`, a **mean-1** modulation meant for a photograph, and rendered
near-white on an untextured material; `boxes_mesh_with` splits the two and
`tests/mob_mesh.rs` gates the mean. (3) `bodies.rs` drew a humanoid rig at
every pig's position as well, because its only filter was "not me".

**§0v below and this item closed each other** (operator: *"go ahead and
finish"*). The oven shipped cooking nothing because nothing on the island
was raw; the pig is the first thing that is, and raw meat is the only item
in the set you cannot eat — which is what gives the fire a job.

**Making the sim actually do it (`server/tests/hunt.rs`) found a hole, not
a tuning problem.** The kit had **no weapon in it**: `weapons.toml` armed
six things and no tool was one, so `held_melee` was `None` for every pocket
a fresh character owns and a hatchet could not hurt a pig, a player or a
door. Five content rows fixed it (`DECISIONS.md` 2026-08-08) and the hunt
now runs **10.1 s** from a 12 m start with the kit's own stone hatchet. The
test also reddens with the right message when `flee_pct` goes back to 100,
so yesterday's capture-found defect is gated rather than remembered.

Left open by that: whether `ttk_melee` should widen so a rock is
meaningfully worse than a crafted spear rather than one hit worse. A band
is a knob — `DECISIONS.md` §open, "tools as weapons".

1. **A butchering VERB** — the reference's actual interaction, a tool-gated
   harvest on the body. Its landing place exists now: the corpse bag
   (`mob::strike` → `backpack::stand_up`) is the verb's output.
2. **Nothing fights back.** Needs a mob→player damage path, a new death
   cause on the wire (**the 2-bit cause field is saturated since wire v24**,
   so this is the widening), and a combat-feel answer to being hit by
   something you cannot reliably hit back.
3. **The massing is boxy up close** — at 8 m the head barely separates from
   the body (captured 2026-08-08). Massing detail, not animation; the legs
   already trot.
4. **`MAX_MOBS = 64` has never met a playtest.** It is derived (the wire
   budget) rather than felt, and it is the one number a player answers.

---
## 0v · The fire cooks now — what it left open *(systems lane)*

Landed 2026-08-08/09: the oven (`sim-core/oven.rs`, `DECISIONS.md` §open
"oven v0"), the meat loop (§0m above), and the burnt state
(`item.burnt_meat`, gated by `content.rs::the_meal_left_on_the_fire_burns`).
Still open, and deliberately: the furnace's ore rows are station-gated
crafts in `recipes.toml`. Moving them into the oven is the reference's model
(`BaseOven`) and re-prices the whole powder chain against `CONTENT.md` §4's
bands — a balance pass with an operator's number on it, not a refactor.

## 0u · The ghost tells the truth — what it still cannot promise *(client lane)*

Landed 2026-08-07/09: the doorway ghost is three parts off
`structures::shape_parts` (the one table the piece and the ghost both emit
from), the deploy ghost mirrors the sim's own verdict while AIMING, and a
door aims an EDGE and is placeable at all. Gated in `tests/ghost.rs` against
the sim's own predicates. Remaining:

1. **Stairs are still a flat slab** in both the ghost and the piece — a ramp
   drawn as a plate. Shared, so at least they agree.
2. **A lock aimed at a DOOR is unreachable** — locks still target the plane
   (on a box the L verb works). Noted at `place::deploy_target`, not built.

## 0v · Players are people — what the rig still cannot say *(client lane)*

Landed 2026-08-07: remote bodies are a skinned mannequin (CC0, 46 clips,
`assets/models/MANIFEST.md`) with gait chosen from derived speed, facing the
wire's `yaw`, plus a held tool with bob/sway/swing (`render/viewmodel.rs`).
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
   position is the interpolator's; the `_RM` variants are deliberately unused.
   Feet will slide at speeds between the clips' authored ones — the fix is
   scaling playback rate to speed, which is a knob nobody has measured.
5. **A plain worn-steel albedo is the missing texture.** The axe head carries
   no map because the only metal in `assets/` is ribbed corrugated sheet
   (`viewmodel.rs` and `assets/textures/MANIFEST.md` both record it).

## 0w · The props carry a photograph — what is left after it *(client lane)*

Landed 2026-08-07: 34 CC0 textures shipped, `props::Soup` box-projects per
triangle (free on a soup — no shared vertices, no seam), `blob_mesh`
subdivides and displaces, bark/wood/stone/metal/rock are bound. Licence rail
widened the same day (`DECISIONS.md` 2026-08-07). Remaining, ranked by what
the captures show:

1. **The hemisphere fill, and it is now the top visual gap.** p10 71.0 against
   a reference 41.0 — props v1 moved it 13 the wrong way by removing the
   frame's accidental darks (`RENDER.md` §0). One owner, one iteration, inside
   the coupled lighting set; do not touch it from a parallel lane.
2. **Trees are small and sparse in the midground.** The wide vantages are an
   empty green plain between the near clutter and the far ridge, where the
   reference frames are dense. This is `terrain::scatter`'s density and the
   conifer's scale, not a material.
3. **Nothing sits IN the ground** (`ART.md` rule 2). The new boulder has a
   clean elliptical intersection with the turf and no crowding or dirt skirt.
4. **The far mesh speckles.** Grazing-angle aliasing on the 8 m LOD; the
   candidate is anisotropy, registered at 4 for a browser reason that does not
   survive the port (`ART.md` §7), so it is a proposal not an edit.
5. **The viewmodel is two untextured boxes**, and it is in a third of the frame.
6. **Roughness maps are still unread** — all nine of them. Blocked on an ORM
   packing step, not on a slot: `metallic_roughness_texture` is glTF-packed and
   its B channel is metallic, so a greyscale rough jpg would make every surface
   a half-metal.

## 0p2 · What the UI still owes *(client lane)*

The palette, the vitals bars, the icons, the baked wheel, the hammer wheel
and the typeface all landed (2026-08-07/09; `DECISIONS.md` "ui palette v1",
"ui type v0", §open `CELL_LINE_CHARS`; gates `tests/ui.rs` §F–§K). Left:

1. **Rotate is still not a verb, and a piece has no facing to turn.** A
   placed piece is `{cx, cz, level, loc, row, hp, uh}` — rotate waits on an
   asymmetry worth turning, not on lane room (`ACTION_SUB_BITS` is 5 since
   v30; the lane holds 32).
2. **The hammer wheel's wedges are text** (no verb icons baked; `glyph`'s
   fallback carries them) and the centre readout names the verb, not the
   target or the upgrade's cost.
3. **Nothing in this repo can photograph a panel.** `render/panels/` is not
   registered on a `--capture` run, so inventory, crafting and the wheel —
   ~1,400 lines and the screens a player spends the most time in — are seen
   only by a human with a shard up. Wanted: a **viewer, not a gate** — a
   mode that opens each panel against a stocked fixture and writes a PNG per
   screen. The visual-gate rule is retired and stays retired (`CLAUDE.md`);
   this asserts nothing.
4. **Twelve sizes is not a scale.** Collapsing to five is a real improvement
   and may not be done blind: the numbers were budgeted against 720p and the
   first cut clipped a column at both ends.
5. **Surveyed and refused: `bevy_hui`, `bevy_lunex`, `bevy_feathers`.**
   Taking `bevy_hui` would move ~5,400 lines of screen description into a
   plugin that spawns entities from data — the same reason
   `bevy_procedural_tree`'s plugin is deliberately unused. The iteration win
   it was wanted for is item 3's, and item 3 costs a fraction as much.

## 0y · Persistence takes the reference game's shape *(server lane)*

Landed 2026-08-07/09 and ARMED on the public shard: SIWE identity, sleepers,
the world file (temp-then-rename, identity table, backup rotation), graceful
shutdown, and two-phase eviction. Plan and sources: `reference/SAVES.md` §9;
knobs: `DECISIONS.md` §open "player persistence v0". What remains:

1. **A sleeper does not block movement** — players never collided, so
   sleeping changed nothing; the question is unanswered rather than decided.
   Lootable-alive is still item 1 of whatever comes after (Devblog 7 shipped
   it after standing too).
2. **The same-window rejoin.** A victim reconnecting in the very window that
   evicts them gets the store record fetched *before* the eviction save is
   filed — one window wide, the save ring's freshness class; the takeover
   hint already refuses to wake a condemned body.
3. **Blueprints** are the wipe-surviving payload the store split was shaped
   for; nothing to build until BPs exist.
4. **Still no WAL, and the world file answered what a WAL would have
   forced**: a world load is an *origin*, not a command — the WAL header
   pins the origin hash beside the seed and the content hash and replay
   starts there. `worldsave.rs`'s module header has the argument.
5. **Still ungated:** the three-thread shutdown path end to end, and
   `KeySlot`'s id match. Measured by hand 2026-08-07 (a signal test is a
   clock test — `CLAUDE.md`): SIGTERM flushes and exits, SIGKILL leaves no
   `.tmp` and the next boot resumes off the last cadence save.

## 0x · The client makes sound — what it cannot yet hear *(client lane)*

Landed 2026-08-06+: `sound/` is the pure model, `render/audio.rs` the Bevy
half, the bank is **generated at boot** (`sound/synth.rs` — a licence
posture, not a preference), one ring drain (`render/feed.rs`), remote
footsteps, the place cue. Research `reference/AUDIO.md`; every number is
`DECISIONS.md` §open "audio v0". Remaining, in order:

1. **Nothing scores it, because `ART.md` has no audio section at all** — and
   **nobody has heard it** (this box has no audio device), so it is honest
   programmer art until someone plays it. `cargo run -p client --bin
   soundbank -- <dir>` writes all 19 WAVs. Looking already paid twice (the
   flat wind bed, then its fix overshooting); neither was reachable from a
   statistic that only asked "does it have energy".
2. **Music is the highest-value unbuilt thing and the design is written
   down.** `reference/AUDIO.md` §8: gap timer, sectioned themes, an
   intensity scalar off event codes we already have. Every input exists;
   what does not exist is music — a **content** blocker, not engineering.
3. **The `--capture` run is still by hand**, and it is the only thing that
   proves the audio systems execute at all. It needs Xvfb, lavapipe and a
   shard, which is why it is not in `ci/gates.sh` yet.
4. **Two cues still have no producer**: `ImpactWood`/`ImpactMetal` need to
   know WHAT was hit, which the gather toast does not say, and `UiClick`
   needs a hook in the per-screen click handlers.
5. **No occlusion, and it needs a prerequisite rather than a pass.** A wall
   between you and a sound needs a geometry query, and the correct one is
   the sim's (`collide.rs`), not a raycast against render meshes.

## 0z · The world waits for the server now — what the Bevy audit left *(client lane)*

Landed 2026-08-06: the client no longer builds a world at an origin the
server never named (`RENDER.md` §1.1; `DECISIONS.md` §open; `tests/ui.rs`
§E), plus `--features hot`. Remaining, in order:

1. **R-G4 is still the missing half of the Bevy-draws rule.** Placement has
   a gate; the no-gameplay-state-in-the-ECS rule still has none. Its answer
   is the renderer-attached/detached state-hash equality (`RENDER.md` §5).
2. **Nothing photographs the wait.** A capture run exercises it and
   `capture::PLACE_FRAMES` bounds it; *seeing* it is §0p2 item 3's viewer.

## 0x · The native client can play the game now — what it still owes *(client lane)*

Landed 2026-08-06: every wire verb has a key, the decoded stores draw, Dead
and Map exist, the look/strafe inversion is fixed (`look.rs`). Remaining:

1. **Trim Bevy's default features — with a verified build, not a guess.**
   Genuinely unused, by grep: `bevy_gilrs` (no `Gamepad` anywhere — the one
   real system-dep win, `libudev`) and `vorbis` (the bank is WAV we
   generate). Load-bearing despite older notes: `bevy_audio` (audio v0),
   `bevy_gltf`/`bevy_animation` (the mannequin), x11 and wayland (a windowed
   game). Attempted 2026-08-06 and backed out for reasons that were not the
   code: a feature change invalidates every Bevy artifact (32G → 44G on a
   49G disk, `rust-lld` SIGBUS), and a green compile is not evidence — Bevy
   answers a missing decoder with a white fallback and keeps going. It wants
   disk headroom and a `--capture` run someone looks at.
2. **Uncovered since the browser-gate deletion**: the haven shelter, the
   waystation canopy, the clutter ring, and the occupant table for
   everything that is not a tree — eight deleted gates held "the mesh the
   client draws == the volume the server blocks". The replacement's shape is
   `crates/client/tests/tree.rs` — Rust, against the mesh we draw; cheap,
   and worth doing the next time one of those meshes is touched.
3. **World-space anchors are still dropped** (the HUD line landed —
   `hud::readout` pins struct-hit fraction and the charge clock under the
   toast): the wall's own number at the wall itself, a clock on the charge
   mesh (`charge_deploy` unread until that mesh half wants it), and
   `stock_addr` never says WHICH hearth. None is blocked.

## 0s · The front door — what the shell, the splash and the hub left *(client lane)*

Landed 2026-08-09/10 over two passes. `Screen::Boot` is the splash (the
launcher handshake and connect are states now, so a dead shard lands on the
server list instead of `exit(1)`). `render/ui.rs` owns the shell the five
reference frames share. PLAY GAME is the reference's table. NEWS / ITEM
STORE / WORKSHOP are the scry-works launcher's and hand off to it
(`ui/hub.rs` + `manifest.rs`), and the backdrop is **footage** under a scrim,
not a live scene — the operator's correction, and the cheap way round.
Three seams that were computed and dropped are now read: the claimed
address, the launcher's shard-list url, and the launcher connection itself.
Remaining:

1. **The backdrop does not move.** Bevy decodes no video; a loop is a frame
   sequence, ~12 MB for three seconds at 720p/20fps. That trade is the
   operator's — `DECISIONS.md` §open, "menu backdrop v0". The shipped still
   is a `--capture --no-hud` plate of our own island, so a better one is a
   command, not an art commission.
2. **Nothing publishes `news`/`store`/`workshop` yet**, so all three read
   "the launcher's manifest names no link for this". The client side is
   done; the remaining act is the platform's — add the keys beside
   `servers.url` in `data/launcher/gates.manifest.json`.
3. **Ungated, by hand only:** the star, the search box, the filters and the
   OPEN IN LAUNCHER click were driven headless with `xdotool` and looked at,
   never against a populated list or a live launcher (§0v item 1).
4. **The splash cannot cover its own first ~3 s** — wgpu adapter enumeration
   and window creation precede the first Bevy frame. A second process would;
   not taken.

## 0w · The native menus landed — what they cannot do *(client lane)*

Landed 2026-08-06: `Tab` inventory + crafting, `B` build wheel, drag/drop —
arithmetic in `ui/` (pure), drawn by `render/panels/`, 23+ assertions in
`tests/ui.rs`. Remaining:

1. **The rail is not the reference's, and one wire field would fix it.**
   `EventMsg::Catalog` ships display names only, so a category rail by item
   class is not computable client-side. A class byte per item, a `PROTO_VER`
   bump and regenerated goldens in the same commit (wall 6) buys the frame's
   real rail. Today's buckets are honest but they are not that.
2. **The drag is gated as arithmetic, not as a gesture.** The spawn kit
   removed the empty-pockets blocker; press → ghost → release → send against
   a live shard is still verified by inspection only.

## 0v · The menu flow landed — what it still cannot show *(client lane)*

Landed 2026-08-06+: server-select first, `Loading`/`Paused`/`Settings`, a
failed connect returns with the reason, settings persist (`crate::config`,
`DECISIONS.md` §open "settings v0"), and `Screen::Disconnected` latches a
hangup through the menu's own teardown. Remaining:

1. **Nothing is published, so every list is the Direct row.** The url is
   `DECISIONS.md` §open and an operator act: serve `servers.json`, set
   `servers.url` in scry's `data/launcher/gates.manifest.json`. Until then
   both the menu and the launcher's Servers window are correctly dark.
2. **Player counts: the code is done end to end; what is left is operator
   acts.** A row may carry `status_url` and both readers poll it every
   `STATUS_POLL_SECS` (`DECISIONS.md` §open "shard status poll v0"); the
   count is never baked into the document, because a generated number is
   stale before it is served. The three remaining steps are all on a box:
   set `status_addr` in `shard.toml`, open that TCP port (the cloud firewall
   too), and put the url in `shards.toml`. Until then every row draws `?`,
   which is correct.
3. **Ungated, by hand only:** the end-to-end kill-the-shard-mid-play run
   behind `Screen::Disconnected`.

## 0t · the native pine is generated — what it owes

Landed: `render/tree.rs` calls `bevy_procedural_tree` as ONE pure function —
no plugin, no ECS; `props.rs`'s whorl builder stays as the far-LOD
silhouette. Gate: `crates/client/tests/tree.rs`. Owed, in rank order:

1. **The billboard LOD.** 328 trees × 5.9 k tris is 1.9 M against DESIGN
   §9's 1.5 M, so the full ring is knowingly over budget and only the ~80 m
   band is affordable; `tests/tree.rs` prints the arithmetic. SeedThree's
   `impostor.js` is the worked reference (two crossed alpha cards baked
   front/side in a worker, readback row order probed once); its per-tree
   `Group` emit is the part to throw away — this client wants an instanced
   pool. Whatever LOD1 becomes, it sways: a billboard has four vertices to
   put a wind weight on.
2. **`aWind`** — `StandardMaterial` cannot read a custom attribute, so wind
   needs the custom material `RENDER.md` already lists.
3. **The needle card is generated** (`tree::needle_image`); a photographed
   sprig is a later swap, not a prerequisite.
4. **Owed upstream as a bug report:** `BranchForce` pointing down hits the
   antipodal singularity in `Quat::from_rotation_arc(Y, dir)` and bends the
   whole tree sideways — droop is the limb ANGLE's job.

## 0u · the frame budgets are browser numbers and nobody has re-derived them

**Doc pass landed** (`DESIGN.md` §9, `RENDER.md` §6, `ART.md` §7,
`TERRAIN.md` §4/§6, `NETCODE.md` §4): every performance claim now says which
platform it was chosen for. The open question is not a doc problem:
`DESIGN.md` §9's budgets were set for a WebGL page and three no longer
describe what constrains us —

- **initial load < 15 MB** and `ART.md` §7's **12 MB texture payload** are a
  first-visit *download*. A depot install is not one, so 2K/4K re-sourcing
  is unblocked; what is real natively is VRAM and disk, and nothing has
  measured either.
- **< 300 draw calls / < 1.5 M tris** are WebGL-shaped, and two shipped
  numbers are already rationed against the 1.5 M: `CLUTTER_RICH_PER_TILE =
  96` and the conifer ring's "over budget" verdict (1.9 M).
- **60 fps on a mid laptop iGPU** survives — a hardware floor.

**Nothing was renumbered.** These are `(knob)` and therefore spoken, and a
budget raised by the loop that then justifies the loop's own triangle count
is the wrong direction of travel. The measurement is small: capture on a
real GPU at the ring's p90 tree count, read draw calls and frame time off
`RenderDiagnosticsPlugin` (its wall-clock half is not assertable —
`CLAUDE.md`), and propose into `DECISIONS.md` §open. Related: the anisotropy
ceiling `BASE_ANISOTROPY_MAX = 4` was set for a software-rasterizer reason
that does not transfer.

## 0a · The clutter ring's fade — two findings kept *(client lane)*

The browser item was retired 2026-08-06 (`DECISIONS.md`); the native ring is
`render/clutter.rs` and still ends hard at ~32–45 m. Two findings survive
the deleted item (full text in git):

- The fade's recipe: thin stochastically by instance hash so the same
  elements survive at a given range, then scale survivors to zero — and
  whether the edge reads at all at that distance is a question for a person
  with the game booted, not for a guess.
- Beach skirts are thin because `scatter` puts 0.22 prop centres a tile on
  the coast against 0.95 inland — the two ratios match to a tenth. That is
  the scatter table's business, not the skirt path's.

## 0q · The gaps nobody has claimed

Lifted out of "done this pass" items before pruning (2026-08-05, again
2026-08-09) — each was written down **only** inside a done item. All of it
is `crates/`/wire work no single-surface lane may take.

1. **Shore barrels as a second destination class.** The road pays unevenly
   now (the bay slots landed) and the haven pad is the one place worth
   walking to. A second class on the shore would give the ring two ends
   rather than one. Nothing else in this file mentions it.
2. **The wipe.** Named by both judges, described nowhere. A shard lifecycle
   act with an economy half (`ALPHA.md` A1→A3) and an operator half
   (`CLAUDE.md`: wipes of a live shard are operator-only), so the loop's
   share is the mechanism, never the trigger. Needs scoping before it can be
   an item.
3. **You cannot stand ON anything.** `movement::step` asks `slot_blocks` and
   nothing asks a ground query for occupants — the shelter's plinth reads as
   a kerb you sink into, crate and boulder tops the same (`terrain.rs`'s
   plinth doc still says "nothing here makes a body stand on the plinth").
   Belongs beside `collide::piece_ground`, a `slot_ground` next to
   `slot_blocks`; the fourteen-box table is already there for it. Systems
   lane.
4. **The 100-bot soak has never been run.** `NETCODE.md` §9's budgets have
   never met 100 real connections: `cargo run -p server --bin bots -- 100`
   against a dev shard, held an hour — tick jitter, WAL append rate,
   per-client bandwidth recorded as counts and bytes, never wall-clock
   asserts (`CLAUDE.md`'s clock rule). The numbers land in a `DECISIONS.md`
   §open row as the measured baseline.

---

## 0r · The raid loop has offence now — what it still cannot do *(systems lane)*

Landed: `sim-core/charge.rs` — plant the held throwable at an address, fuse
from content, damage through the same `damage_piece`/`damage_deploy` a swing
uses (`ACT_THROW`/`EV_CHARGE_PLACED`; knobs `DECISIONS.md` §open "satchel
fuse v0"). X plants it natively and the HUD counts it down. Remaining:

1. **No blast radius** — the content half landed, the arithmetic did not.
   `blast_m` is schema'd, baked to `ThrowDef::blast_cm`, walked into
   `canon::hash`, and **nothing reads it**: a charge damages only the
   address it was planted on. What remains is the falloff and a bounded
   multi-target scan; `combat::raid`'s 3x3 column-index ring is the shape to
   copy. The content hash has already moved, so the cost is paid whether or
   not the slice is taken. Knob: `DECISIONS.md` §open "satchel blast v0"
   (PROPOSED 3 m).
2. **Nothing is hurt by standing in one.** `ThrowDef::damage` is carried and
   `EV_CHARGE_PLACED` has no player-damage half, so the defender's seconds
   are free to spend standing on the charge. A new `DEATH_BY_*` if taken —
   and the 2-bit cause field is saturated since wire v24, so that is a
   widening. Lands with item 1 or not at all — they share the falloff.

## 0a · The island has a map now — and the trip has both ends *(ui lane)*

Landed 2026-08-09: the marker layer — haven and waystations as rings,
bed/hearth/backpack marks, one projection (`world_to_map`), cap 64
drop-newest (`DECISIONS.md` §open "map markers v1"; seven lib tests in
`ui/map.rs`). Still open, neither of it ours:

- **Operator:** the death marker (`ALPHA.md` §1 keeps position off the death
  screen; the map touches no death fact today), and whether the marked set
  is right — boxes and doors stay unmarked deliberately.
- **Respawn — the gap's other half — is BLOCKED, measured.** The wire
  carries `Respawn { on_bag: bool }` and nothing else; no owner bit and no
  cooldown ride `DeployRec` (`deploy.rs`, "never the wire"). So the client
  cannot tell its own sleeping bags from anyone's, nor which are ready, nor
  name one. "Beach or each live bag" (`ALPHA.md` §1) is a wire change first
  — systems lane.

## 1 · The native pivot — what is left of it

The client is native (operator, 2026-08-05; `DECISIONS.md` has the row). The
session, input, terrain, lighting, scatter, HUD, panels, depot packaging —
R0–R6 plus R8 — all landed, and the browser client is deleted. Left:

- **Publishing and notarizing the depot are operator acts and are NOT
  done.** The build ships as a scry depot (`ci/depot.py`, gated by
  `--self-test`); the depot ships `assets/`, not just the binary.
- **The visual gaps, ranked by measurement** (`RENDER.md` §8 carries the
  list): the hemisphere fill (§0w item 1 — the top gap, coupled-lighting
  single owner), cloud form (the deck reads stratus where `ART.md` asks for
  cumulus), and the four-way splat material (one map serves all four ground
  identities today).

---

## 4 · The event lane's payloads are law — all 32 codes gated by role

Landed (`sim-core/tests/event_roles.rs`, finished on `lane/event-gates`):
every `EV_*` code carries a role check against a real cause, all a/b-swap
mutants reproduced red, `NOT_COVERED` is empty, and the ledger seat stays
for the next code. **Remains, and it is not tests**: the stronger form is a
payload-role table both the emit site and the check read, a swap as a
*compile* error (`reference/FINDINGS.md` §1 end) — bigger than one pass.
(The `CLAUDE.md` trap-list correction this item asked for is done — that
entry names the landed gate and keeps the mechanism as the lesson.)

---

## 4b · The world lane: what the second tier left open

- **A destination still offers no verb you cannot perform at your own
  base.** The recycler is the only one of `DESIGN.md` §2's three fixtures
  not blocked on an operator act (bank is A2/A3, vendor is skins). Container
  verb + `content/*.toml` yields — **systems lane**, and it is what turns a
  loot gradient into a reason.
- **The waystations want a silhouette, and it must be a *different* one.**
  Their containers and loot tables differ from the pad's now; the site
  itself is still two boxes on bare ground, and a second copy of
  `HAVEN_SHELTER` would make the two tiers look identical.
- **The pad carve is still unbuilt, and smaller than this file used to
  say**: `height` has 18 production call sites in 3 crates (not "~80 in
  four"), and `haven()` measures 12,463 taps mean over 16 seeds. Re-scope
  against 18 before assuming it cannot be a pass. Whether a tier should
  carve at all is **open for the operator** (`DECISIONS.md` §open,
  waystation canopy v0).
- **Nothing threatens the walk between them.** The pig flees and never
  fights — §0m item 2 is this gap seen from the other end.

## 4b · The domain gate reads the crate now — one residual

Landed 2026-08-05: `SOURCES` reads all `sim-core` modules both ways and
every enumeration width is classified. Remains:
`death_causes_are_a_closed_ledger` (`event_roles.rs`) still scrapes
`world.rs` alone — narrow, since the protocol gate catches a stray value
crate-wide, but its *contiguity* claim is file-local.

---

## 5 · Gameplay still missing, in rough order of what a player notices

- **The arrow is invisible and does no structure damage.** Ranged landed
  (salvaged 2026-08-06, wire v24) and `ranged.rs`'s own header records what
  is left: no `EV_SHOT`, so no client can draw a tracer — a shot arrives as
  `EV_HIT`/`EV_HEALTH`/`EV_DEATH` and nothing else; and an arrow that
  reaches a wall stops dead rather than chipping it — `collide::blocked`
  bakes `CAPSULE_RADIUS_M` into its query, so an arrow is as fat as a body
  and threads a doorway but never an arrow slit.
- **The revolver still cannot fire.** Hitscan wants M2's rewound raycast, so
  `bake_combat` drops firearm rows deliberately, not by omission.
- **Dropped loot should land somewhere you can find, not inside the floor**
  — and `gather::inv_add` still loses overflow, a documented policy
  (`DECISIONS.md` §open) pointing at exactly this slice.
- **Mushrooms and corn drop now** (2026-08-09, content rows only): the
  tree's secondary pays 1 mushroom a swing — the forest floor through the
  tree that shades it — and the coast-road barrel rolls a 2–4 corn ration.
  `content.rs::every_consumable_the_content_ships_is_reachable` gates the
  general form (every consumable producible by a live verb chain). Still
  owed, and both are code: a standalone forest-floor pickup archetype and a
  farming lane; plus the cache/crate open verb before loot-only food could
  sit at a destination — the gate deliberately counts barrel rows alone.
- **Day/night does not exist.** `DESIGN.md` §2 pairs it with the survival
  clock; nothing in `crates/` reads a time of day.

---

## 5b · The wire accepts values the sim can never mean

`every_domain_fits_its_wire_field` (`protocol/src/event.rs`) gates ten value
domains; the sim/server refusal side is closed (`lane/wire-values`:
`BAG_GONE_*`/`REFUSE_C_*` refused at the pump and counted, `buttons` bits
4–7 refused at `accept_input`, never a disconnect). Still open, the wire act
this item always named: the *decode* side — the client's decoder taking
`why == 3` / `reason` 4..15, the button octet — plus deriving the two
`*_MAX`s into protocol's exempt list and the `PROTO_VER` judgement for
narrowing what decodes. One protocol pass. Systems lane (`crates/protocol`).

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
    the operator holds no keys and can restore nothing. It is the
    platform's client for the whole cascade, not a Gates accessory.

Standing rule: anything a playtest breaks jumps this queue; anything a wall
catches jumps the playtest.

## 5c · The protocol golden has never fuzzed a button above bit 1 *(systems lane)*

Found while landing jump. `goldens.rs` draws the input fixture's `buttons`
from `rng.next_bounded(4)`, so the golden exercises only `BTN_SPRINT` and
`BTN_CROUCH` — `BTN_PRIMARY` and `BTN_JUMP` are outside the draw. The field
is 8 bits wide either way, so the golden still pins the *layout* and nothing
is currently wrong on the wire; what it cannot see is a future encoder that
masks or reorders the high nibble.

Deliberately its own commit: widening the draw changes fixture bytes, and
changing golden bytes for a reason unrelated to the version's meaning
muddies the one signal wall 6 reads. It is a `PROTO_VER` judgement call —
the answer may be that a golden's fuzz range is not part of the wire
contract at all. Decide that first; it is the actual question. Same shape
one level down: whether `decode_input` itself should narrow the unmeant
bits is the protocol pass §5b still owes.

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
