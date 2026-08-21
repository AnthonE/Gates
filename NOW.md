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
>
> ⚠ **Section labels collide, and it is the merge strategy again.** Eight are
> duplicated (`0a 0u 0v 0w 0x 0y 0z 4b` — `0v` three times), because
> `merge=union` lets each lane pick "the next free letter" against a file
> that does not yet have the others' picks. About 39 citations in other docs
> point at an ambiguous target that way; `NOW.md §0y` is nine of them and
> resolves to either the sea or persistence. **Not renumbered here**: the
> citations are mostly in `DECISIONS.md`, which is the dated record and is
> not rewritten to match a later tidy. Read a `§`-citation as a hint and
> match on the title, and when you next edit a colliding section, give it a
> label no other section has.

---

## 0rn · The rename's six loose ends *(operator, mostly)*

The 2026-08-21 row in `DECISIONS.md` has what moved. These are what could not,
each blocked on something outside this tree.

1. **(operator, wallet)** `scry.json` needs a re-sign at seq 6. Its `_next`
   points at `scry.moreright.xyz/api/library/GAME-REPO.md`, which is **410
   Gone**, and `_version` cites `ci/scry_manifest.py`, which is
   `ci/elo_manifest.py` now. Both are inside the signed bytes, so nothing here
   may touch them — the key is on morr. Filenames and the `"scry": 1` key stay
   whatever `/api/library/GAME-REPO.md` says on the day it is signed.
2. **The SDK is re-vendored, never renamed.** `crates/client/src/scry_overlay.rs`
   is still sha-pinned against `scry-forge`. When the launcher's rename lands,
   `cp` + re-pin + check the CALL SITES — `Overlay::title` and `play_message`
   both changed shape under us before while everything compiled.
3. **The tickers are a redeploy, not an edit.** `/api/onchain` names SCRY, OBOL
   and MYRRH. `marketing/` is frozen at those three with a header saying why;
   whether to redeploy under ELO/JUNK/ORBS is unspoken and is `scry-forge`'s
   act, not this repo's.
4. **`elo-shardlist-v1` is emitted and unread.** `ci/shardlist.py` writes the
   new kind; the live served document carries no `kind` field at all, so
   nothing breaks today — but the launcher should accept it before the next
   publish.
5. **The junk icon is a picture of two coins.** `assets/icons/junk.png` is
   still `delapouite/two-coins`, which was right for a coin and is wrong for
   scrap. An art call, on the CC-BY rail.
6. **`fix/us-east-shard` is unmerged and already true.** The published list
   says `us-east-1` / "Gates US East 1"; `shards.toml` still says `eu-1`. That
   branch carries the fix plus two DECISIONS rows written on the origin and
   never landed here.

## 0rep · A player can file a report; four things around it cannot *(client lane + operator)*

Landed 2026-08-20. `F7` writes `gates-report-<stamp>-<fp>.md`, a `.json`
beside it and a `.png` of the frame, into the screenshot directory:
build (`VERSION`/`GIT_SHA`/`PROTO_VER`), the seed, position, the netcode
counters `ClientCore` already kept, and a stranger's line quoted in a fence it
cannot escape. `crates/client/tests/report.rs` is the gate, and every assertion
carrying the untrusted-prose rule was proven red under its own mutant.
A panic writes the same document, chained ahead of the default hook.
`DECISIONS.md` §open, bug reports v0, has the four bounds.

What it does NOT do, in the order the value drops:

1. **Nothing reads them but `ci/reports.py`**, which folds a directory onto its
   fingerprints and prints the board. No page serves it. **(operator: where.)**
2. **No intake, deliberately** — the client opens no socket and the player
   decides what happens to the file. An endpoint is its own slice.
3. **The `report` signing family does not exist in the launcher** — shipping
   set is `play`/`review`/`vow`/`hive`/`braid`/`store` (`scry-broker`'s
   `signer.rs`). `Report::sign_text` is built to `sdk/PROTOCOL.md`'s rules and
   is refused today, which is the correct failure. Upstream, in `scry-forge`.
4. **A report pays its reporter** (operator, 2026-08-21) and the rail is built:
   a PR carries `Closes reports: <fingerprint>`, so the merge that pays its
   author says which reporters it owes. **Two things left, both operator:** how
   much against the PR's 100,000, and whether it pays on the merge or earlier —
   `DECISIONS.md` §open has the trade. Nothing pays until (3) lands either way:
   an unsigned wallet is a claim, and paying a claim pays whoever typed it.

---

## 0pw · Every material is drawn once before it is needed *(client lane)*

LANDED 2026-08-20, and it corrects the trap it closes. `CLAUDE.md` said a
native pipeline compile is *"a bigger stall"* than the WebGL link it inherited
the fear from; `synchronous_pipeline_compilation` is **false** by default, so
Bevy builds on a task pool and SKIPS a draw that is not ready. **The native
failure is a pop, not a hitch** — and you look for it differently.

Most of it was already covered by a good accident: `world_running` includes
`Screen::Loading`, so the world draws while the bar fills and everything
streamed specializes there. What was left is the event-driven half — a tracer,
a build ghost, a highlight, a mob, a held item. `render/prewarm.rs` warms every
`StandardMaterial` off `AssetEvent::Added` rather than from a list, because a
hand-kept list is the drift `CLAUDE.md` names twice elsewhere.

**Still open, and named in the module**: skinned meshes are a different
pipeline key and nothing warms them, so the first remote player to walk into
view still specializes on arrival. And the measure is still a COUNT with no
gate — `PipelineCache::pipelines()` is public but lives in the render world and
needs a GPU.

Knob: `DECISIONS.md` §open, pipeline prewarm v0. Gates: `tests/prewarm.rs` (5),
five mutants caught. Two things the first cut got wrong: the warm draw was
0.374 px on a 4K panel at the narrowest fov, and it was sized in the vertices
rather than as a transform scale — a mikktspace panic waiting to happen.

## 0gq · The renderer has a budget knob — LANDED 2026-08-20, unlooked-at *(client lane)*

There was no graphics setting at all: the GRAPHICS tab held a field of view
and three `Row::Fact`s saying the rest was fixed, so a player whose GPU could
not hold the frame had one lever and it was the fps cap. `config::Quality` is
LOW/MEDIUM/HIGH now and `render/quality.rs` is the table — AO, SMAA, bloom,
shadow cascades with their reach and map size, and the tree LOD swap distance.
Knob: `DECISIONS.md` §open, graphics tiers v0.

**HIGH is the frame that shipped, exactly**, and `tests/quality.rs` holds that
column against the literals `rig.rs` used to carry. That is the whole safety
argument for landing it unlooked-at: the default frame did not move.

**Nobody has seen LOW or MEDIUM**, which is the residual. The ladder's ORDER
is arithmetic — a cascade re-rasterizes every caster, and the swap distance
decides how much forest that is — but where each rung sits is a judgement, and
the visual gate here is a person. Walk the knob down and see whether MEDIUM is
still the game.

**A render scale is the biggest lever and is not here**: Bevy renders to the
window surface, so a scaled path is an off-screen target and a blit, its own
slice. Same for the clutter and prop rings, which decide which tiles exist
rather than how they draw (`ART.md` rule 4 is a floor a tier may not cross).

Gates: `tests/quality.rs` (5, new). Eight mutants caught, and two gates needed
a second draft: the clamp one passed a wrap, because five steps on a three-rung
ladder lands right anyway; and the applier REACTED to a settings change, so a
tier chosen last session never reached a camera that spawns several frames
after the file loads. It reconciles on `Added<EyeCam>` now.

## 0lod · The far forest is a hull — LANDED 2026-08-20, unlooked-at *(client lane)*

The ring's triangle debt is paid: **1,935,200 → 509,630 tris** at the prop
ring's p90 of 328 trees (printed by `tests/tree.rs`, not quoted). Past
`TREE_LOD_SWAP_M` (80 m — the affordable row of that suite's own band table) a
tree is one 105-triangle opaque hull instead of a 5.9 k bark mesh plus an
alpha-masked canopy, crossfaded over 15 m by `VisibilityRange`. The hull is
**lathed through the generated tree's own vertices**, so it cannot drift from
what it replaces and a broadleaf dome does not wear a conifer's cone. Knob:
`DECISIONS.md` §open, tree LOD v0.

Triangles are the smaller half: SSAO forces a depth and a normal prepass,
`rig.rs` runs four cascades, and `bevy_light` honours the same ranges the
camera does — so the swap pays back about seven times over, on paper.

**On paper is the residual, and it is the next action.** No GPU has ever run
this client (`RENDER.md` §6) and **nobody has looked at the far forest**. The
hull is opaque where a canopy is mostly air, so it will read denser than the
near tree. Walk to 80 m and look at the swap band.

Gates: `tests/tree.rs` (13), `tests/tree_lod.rs` (5, new — it drives the real
`spawn_slot`). Eight mutants caught, two only after a first draft missed them:
the winding gate asserted normals, which `Soup::tri` blends toward the volume
so an inside-out hull still shades outward; and the hull first carried
`FellPart::Trunk`, the variant `audio.rs` reads as *speaks for the slot* — one
chop would have played `TreeFall` twice and doubled every tree's cover count.

## 0chr · The player is a stick-man now — what the seven clips cannot cover *(client lane)*

Landed 2026-08-17: `stumpy.glb` replaces the mannequin as every drawn body.
It measures **1.800 m with its feet on y = 0**, which is the sim's capsule
exactly, so `ANIM_RIG_H_M` is 1.8 and the scale is 1. `ci/import_char.py`
stands the Z-up export up and renames the clips; `ci/ktx_pack.py` takes it
28.9 → 2.8 MB; `crates/client/tests/rig_asset.rs` gates all of it and four of
its five checks were watched going red against the raw file.

**It has 53 clips, not 7.** `ci/retarget_anim.py` moved the mannequin's whole
library onto the new 24-joint skeleton the same day — nine seconds, +1.0 MB,
no credits. Rotations only (a source bone's translation is that skeleton's
limb length) plus the hips through the hip-height ratio. `Clip::Sprint` plays
a real `Sprint_Loop` again.

⚠ **Its first cut put the arms 43° low and only a person looking caught it.**
`A_TPose` retargeting to a T-pose, the legs, the walk and the spine were all
correct, so every check that had been run passed; the operator opened the
bench and said an arm was inside the model. Cause: the two rigs rest in
different POSES — source a true T, this character an A — and transferring a
delta from each rig's own rest assumes they do not. Arms 36–43° apart, spine
and legs under 17°, which is exactly why the failure was limbs-only and
survived a contact sheet. Fixed by anchoring on an aligned virtual rest.
**Nothing gates this and nothing can**: it is a per-bone measurement whose
answer is "does the pose look right".

**Heads follow the wire's pitch since 2026-08-17.** `bodies.rs` read `yaw` and
never `pitch`, so a remote faced where it walked and stared at the horizon
forever. `anim::head_look` runs between the animation and the propagation —
the one window where overriding a bone is a single quaternion — and the pitch
axis is **derived from the rest pose, not typed**, because this rig's neck is
not axis-aligned and a typed `Vec3::X` swings the head sideways instead of
nodding. Clamped to ±0.9 rad; the spine split is the follow-up.
`crates/client/tests/head_look.rs` gates it and was proven red under a typed
axis.

What is left:

1. **The clips outrun the states that would play them.** `bodies.rs` knows a
   remote's position, yaw, pitch, sleeping flag and — since wire v48 —
   `dead`, so `Jump_Loop`, `Swim_Fwd_Loop` and the crouch pair sit in the
   file unplayable. Each needs a fact on the wire or derivable from it: a
   sim/wire question, not an art one.

   **`Death01` came off this list 2026-08-18** and it is the worked example.
   A corpse keeps its slot until its owner leaves the death screen, so a
   killed player was drawn standing at idle and the one thing a fight has to
   read — *is that person still in it* — was answerable only from the kill
   feed. The fact is one unconditional bit beside `sleeping`
   (`DECISIONS.md`, "remote death fact v0"), and the clip is the second
   non-looping one in `anim.rs`: a STATE rather than a transient, which is
   why it needed a bit where the swing needed an event. Gated
   (`server/tests/client_loop.rs::a_killed_body_is_marked_dead_on_the_wire`,
   proven red with the fill removed; three `anim.rs` units, proven red).
   ⚠ **Nothing has seen the pose**, exactly like the swing below: the wire is
   gated end to end and `Death01` playing on a real body has never been on a
   screen. Boot the game and kill something.

   **`Hit_Chest` came off this list 2026-08-18 too, and it took the cheap
   half.** `pop_hit` no longer drops the victim id, so the flinch is drawn
   for the ATTACKER only, off a field that was already on the wire.
   The broadcast version (everyone sees the recoil) is still unpriced and
   still the same fan-out §0sw owes a soak; §0pvp item 1 and the
   `DECISIONS.md` row carry the asymmetry.

**First-person arms landed 2026-08-17, and are in a captured game frame.**
The claim that "hide everything but the arms is not achievable on this asset"
was wrong: what was missing was not a trick but **something to hide**.
`ci/split_arms.py` splits the mesh by skin weight into `char1_arms` (8,184
tris, 26%) and `char1_body`, two nodes sharing one skeleton, one material and
one set of vertex buffers — they differ only in their index array, so the
split costs 0.4 MB and no second copy of the mesh. Hiding half is then a
`Visibility`.

`VIEWMODEL_ARMS` is **derived, not dialled in**: the rig is measured to face
+Z, a Bevy camera looks down −Z, so the arms are yawed 180°, and the offset is
whatever puts `Pistol_Idle_Loop`'s right hand exactly on `VIEWMODEL_HOLD` —
chosen by computing where every one of the 53 clips puts that hand in view
space, and it is the only two-handed hold that LOOPS. Measured in the running
client: the hand lands at **(0.321, −0.305, −0.522)** against a target of
(0.32, −0.30, −0.52).

⚠ **It still drew nothing until frustum culling was turned off**, and that is
the skinned-mesh AABB fact arriving a *third* time in a third disguise: the
culler tests the BIND box against the mesh NODE's transform, and that node
hangs under an armature carrying `scale 0.01`, so it was testing a 2 cm blob
1.5 m below the eye while the GPU skinned the vertices right in front of the
camera. Everything else was already correct — which is why the diagnostic that
measures the hand exists and stayed in.

What is left on the arms:

- **The item is not parented to the hand.** It does not need to be — the hand
  is on the hold point and `animate` moves both with one motion — but
  parenting is the better design and needs the grip retuned against the arm's
  frame, which is judged by looking, not derived. `ViewArms::hand` records the
  bone so that change is one line.
- **No render layer, so the arms can clip into a wall.** Deliberately not
  built: a second camera has to duplicate exposure, tonemap and atmosphere,
  and `CLAUDE.md`'s trap list says that coupled set has exactly one owner.
  The held item has had the same flaw since it existed.
- **The hand reads large and the fingers are splayed rather than gripping,
  and no clip can fix the second half.** The skeleton is 24 joints with no
  finger bones at all, so every hand in every clip is the bind pose forever.
  That is a mesh property; a re-import or a sculpted grip is the only lever.
  `--bin modelview <file> --eye --hide char1_body` previews the geometry.

**The off hand is gone, 2026-08-20** (operator: *"our models hands are a bit
crossed?"*). `Pistol_Idle_Loop` is the rig's only two-handed hold that loops,
which is why it was picked, and a two-handed grip on a one-handed rock puts
the support palm **62–66 mm** from the hold hand and **31 mm nearer the eye**,
so it drew in front of the item — the tightest of all 46 (clip × hold hand)
pairs in the rig's 53 clips, and the left arm crosses the midline to get
there. `VIEWMODEL_HIDDEN_ARM` collapses `LeftShoulder` in `dress_arms`, which
takes the whole chain with it to a point at ndc y ≈ −1.8. Hiding beat swapping
clips because **every one-handed idle this rig owns presents its LEFT hand**
(torch, spell, shield), so a swap moves the item into the wrong hand, re-derives
the one placement measured in a running client, and points away from
`Sword_Attack` — item 2 below, which swings the right arm. One arm is also
what the reference draws for a one-handed tool. `tests/viewmodel_arms.rs` is
the gate, and it holds the derivation `VIEWMODEL_ARMS` only ever printed at
runtime. **Nobody has looked at it yet** — that is owed.

2. **The gather swing is `Sword_Attack`** (operator, 2026-08-17 — the
   reference game swings one animation at everything). No asset is owed; the
   blocker is item 1, a fact on the wire.
3. **No in-game frame has been shot with a body in it.** The bench proves
   the asset and a capture run proves the client boots and connects, but the
   six fixed bearings had no bot in frame. The sleeper tint —
   `anim::build`'s cold copy of the model's own material — has been read and
   not seen.
4. **The mannequin stays** (3.2 MB): a retarget is reproducible only while
   both rigs are in the tree, and its rest pose and bone names do not survive
   into the output.

---

## 0pvp · What a fight still cannot do — the readiness audit *(systems lane)*

Taken 2026-08-18 against the tree rather than against the docs, and every
line below is a command somebody ran. **What works end to end**: melee
player-vs-player (`combat::strike`, TTK 3–5 by band), the bow (`ranged.rs`,
server-simulated, integer ballistics), the satchel's blast, death →
backpack → bag respawn, the kill feed, and since v48 a corpse that falls
(§0chr). Six gaps, cheapest first:

1. ~~A hit body does not flinch.~~ **Landed 2026-08-18, attacker-side, no
   wire byte.** `pop_hit` carries `(victim, damage)` now — the field was
   already there — and `Clip::Flinch` plays `Hit_Chest`. Two knobs, both
   measured off the file: `FLINCH_CLIP_S` is its length and
   `FLINCH_BLEND_S` is its **apex** (0.16667 s, where the pose peaks at
   19.92° off rest), because the general 0.18 s blend finishes *after*
   that and would draw the apex at partial weight. The flinch and the
   swing are one slot with the newest winning; death outranks both.
   ⚠ **The asymmetry is real and is a PROPOSED row** (`DECISIONS.md`,
   "attacker-side flinch v0"): `EV_HIT` is unicast, so the recoil happens
   on one screen in the world. Reverse it by saying so — the symmetric
   version is a new broadcast, not an edit. **Nobody has seen the pose.**
2. ~~A remote's swing is silent.~~ **Landed 2026-08-18.**
   `Cue::RemoteSwing` — positional, appended after `Growl`, the local
   swing's own waveform by delegation, radius and gain read off the local
   row (20 m / 0.45: numbers that row has carried and nothing has ever
   read, because a non-positional cue has no distance). 40 ms cooldown,
   not the local 120: it is per-CUE, so it is the crew's stagger.
   Produced from `feed.swings()` against the transform `bodies::stream`
   just wrote, so the sound and the arc cannot disagree.
   **Nobody has heard it.** A positional *hit* sound is not in this: it
   needs a flesh-impact waveform `synth.rs` does not generate.
3. ~~The revolver is a charged dead end.~~ **Landed 2026-08-19 — it
   fires, and it kills in five.** `bake_combat` no longer drops firearm
   rows: a bow and a gun bake through one `bake_ranged`, differing by
   `RangedDef::hitscan`, and `validate` refuses **both** halves of the
   pairing that decides it (a bow's round must own `[[ammo]]` ballistics,
   a firearm's must not) so the sim never re-derives which it holds.
   `ranged::hitscan` is `ranged::step` with the flight deleted — the same
   `world_stop` then `nearest_body`, so a bullet and an arrow cannot
   disagree about a trunk — run after the player loop for the arrow's two
   reasons. **No content edit, no `PROTO_VER` bump, and `test_replay`'s
   hash did not move** (the probe fixture arms no hitscan row). Gate:
   `sim-core/tests/gun.rs`, 10 checks, all ten proven red, plus three in
   `content/tests/content.rs`; `damage_routes.rs` needed no new row and
   caught a direct `v.hp -=` when it was tried.
   ⚠ **A gun has no muzzle flash, no crack and no tracer.** `EV_SHOT`
   carries a speed and a drop the client re-flies, and zeroes would hang a
   still tracer at the muzzle for four seconds — so a firearm speaks only
   through `EV_IMPACT` and `EV_HIT`. A voice is a new event or a spoken
   reading of `EV_SHOT`'s spare patterns.
   ⚠ **A firearm death reports `DEATH_BY_ARROW`**, whose name now lies.
   `DEATH_BY_BULLET` was built and reverted: protocol's
   `every_domain_fits_its_wire_field` refuses a seventh cause as a wire
   change even though 3 bits hold it, and it is right. The screen reads
   correctly anyway — the weapon is a wire field.
   ⚠ **Priced** (`findings/hitscan-cost-20260819.md`): 295 terrain taps a
   shot against an arrow's 16, so 100 aligned shots cost 20 ms of a 33 ms
   tick until the walk was cut at the nearest body and a *miss*'s bounded
   at `MAX_HITSCAN_MARK_SAMPLES` (10.9 m, cosmetic). Now 1.2 ms in a
   gunfight, 6.5 ms spraying at sky.

4. **Armor reduces damage. Landed 2026-08-19 — the burlap shirt turns a
   rock's five hits into six.** `bake_combat` installs `content/armor.toml`
   and `combat::hurt` reads it off `Player::worn` (`WEAR_SLOTS = 2`,
   indexed **by** slot; a piece pays only in the slot its baked row names).
   The four hit routes are blunted, the three metabolic/keypad ones still
   call `hurt_unreduced`. **The set is one number and both slots sum** —
   aim is planar, so crediting only the body piece would leave
   `armor_burlap_head` charged and dead. A corpse sheds its plates into the
   death bag via `drain_spill`, so a full pocket costs the killer a walk,
   never an item. `PLAYER_SAVE_BYTES` 256 → 268, `SAVE_FORMAT` 3 → 4,
   `WORLD_SAVE_FORMAT` 7 → 8. **`test_replay` moved deliberately,
   `0xDFFD…47C6` → `0xE6C1…FB21`** — deleting the `worn` loop from
   `state_hash` returns it bit for bit, so those twelve bytes are the only
   cause. 17 gates, all proven red. One scalar, not damage types —
   `DECISIONS.md` §open "armor reduction v0" argues it against our data.
   ⚠ **Nothing can EQUIP it — an operator call, three exits.** (a) the wire
   (`CONT_WEAR`: `CONT_KIND_BITS` 2 → 3, `PROTO_VER` 48 → 49, 96 goldens —
   `findings/armor-design-20260818.md` §4 prices it); (b) a spoken
   spawn-wear default; (c) auto-protect from the inventory, a different game.
   ⚠ **The balance anchor is known-misleading and was left so.**
   `balance.rs:117` credits a head piece against *body* hits, has no floor
   and cannot see a set: head 10 % + roadsign 25 % is **+3** on four weapons
   against `armor_extra_hits_max = 2` — and applying armor moves no content
   number, so `test_content` stays green while its meaning rots. The fix
   needs the band re-spoken or the ladder re-priced: operator, not loop.
   What landed instead moves no band — `hits_to_kill` pinned against
   `combat::reduce` for every (weapon, set) pair, which caught the design
   note's own proposed arithmetic disagreeing (6 vs 7 on `hatchet_stone`
   at 35 %). Still open: types, hit areas, condition, `move_penalty_pct`.
5. **No lag compensation — but the shard can now say how stale an aim is.**
   Slice 1 of `findings/lagcomp-design-20260818.md` §7 landed 2026-08-18,
   `crates/server/` only: each buffered input frame is stamped with the
   `snapshot_ack` its first datagram carried, and raw **`T − S`** is folded
   into `ShardStats` (samples/sum/max/unacked/refused + an 8-bucket
   histogram) and published on `/status.json`. **Measured, not derived**:
   100 bots × 60 s on loopback, **mean 1.107 ticks (36.9 ms), max 3, and
   nothing at or past 4** — inside §0.1's prediction, but the reading of it
   changes, because loopback RTT is ~0 and the number is still 1.1 ticks.
   Raw staleness is the input buffer plus snapshot age, **not RTT**: a floor
   no network improvement removes. Numbers and load conditions in
   `DECISIONS.md` §open ("aim staleness v0"); gate
   `crates/server/tests/lagcomp_measure.rs`, six mutants proven red.
   **What remains is slices 2–5**: the ring in `sim-core` (four constants
   into `limits.rs`, and `INTERP_DELAY_TICKS` moving there is why the
   published number is raw — do not double-count it), `Command::Input`
   carrying `favour`, `strike` rewinding, and the server minting it. Both
   `strike` and the arrow still resolve on present server state, so a fight
   is still led rather than aimed. No wire bump is owed at any slice.
   ⚠ The design note's §2.2 claim that `push_frame` "drops a frame it has
   already seen" is **wrong** — it overwrites an unexecuted one, so
   keep-first had to be written.
6. **Nothing has fought at population.** `raid_storm.rs:516` says so in
   its own source — *"nobody swings"* — so wall 4's caps are gated one
   site at a time on every combat path, and `EV_SWING`'s AOI-free fan-out
   is still unpriced (§0sw). Planned:
   `findings/combat-soak-design-20260818.md`. Two findings worth the read
   before anything else: **`EVENT_RING_CAP` (64) is smaller than
   `MAX_PLAYERS` (100)**, so 65 simultaneous swingers resync every client
   at once and a resync re-drips seven cursors; and the **cheapest slice
   is no code at all** — `bots.rs:53-60` already presses `BTN_PRIMARY`
   1-in-3, so re-running the 100-bot soak prices the fan-out today.

## 0dsc · Discord presence is built, detailed and dark *(operator — one act)*

The operator saw the game named in Discord and asked how it knew. Measured:
Discord matches running exes against its detectable database — **22,455
entries on 2026-08-16, none of them `gates` or `gates.exe`** — so that was
the manual "Add it!", which shows the bare process name.

**That database is curated and it is NOT the open door.** Rich presence takes
only an application id, which any developer creates with no review. Checked
in the data: VS Code, Spotify, Photoshop, Figma, Blender, OBS, IntelliJ and
Neovim are all **absent** from the 22,455, and all appear in Discord statuses
daily — as rich presence under their own ids.

Built, and the operator's 2026-08-16 call took it past the address question:
the verb + where on the island + party + elapsed, and **Ask to Join**, both
behind two settings under `SOCIAL` — `discord_presence` (on) and
`discord_share_server` (off, opt-in, and it is a consent rather than a knob:
`DECISIONS.md` has the pair rule and the gates). Still **dark** without
`GATES_DISCORD_APP_ID`.

**The one operator act:**
1. Create the Discord application, set `GATES_DISCORD_APP_ID`, and **name it
   `Gates`** — the portal's application name is the word drawn after
   "Playing", which is what retires the lowercase `gates`.
2. For Ask-to-Join on a friend who is *not* running the game, register the
   URL scheme in the portal (`elo://` or `gates://`). That path is
   `deeplink.rs` and needs no code. The already-running path is built.
3. Optional: upload a 512×512 or 1024×1024 image under the asset key
   `gates`. **There is no Gates mark in this repo** — `marketing/` holds only
   the JUNK and ORBS coin marks, which are the economy's and not the
   game's. Without one Discord simply draws no image.

⚠ The detectable-list submission stays **unverified** — self-serve game
selling is deprecated and no current form was found. A question for Discord,
not a step, and nothing above depends on it.

`DECISIONS.md` 2026-08-16 (spoken) and §open "discord rich presence v0".

---

## 0win · The published Windows depot cannot start *(operator — republish)*

A player ran the launcher's Windows build on 2026-08-16 and got
`gates.exe — Application Error … unable to start correctly (0xc000007b)`
before a frame. Measured off the live depot: `0.2.0-gbed9e02d6`'s
`requires.libs` **names `libstdc++-6.dll`** — mingw's C++ runtime, reached
through `basis-universal-sys`, absent from a stock Windows box — while the
staged tree holds three files and `launch.env` is `{}`. Bundle-nothing is a
Linux rule and does not transfer. `0xc000007b` rather than a missing-DLL
box means that machine *has* a 32-bit copy of the name on its search path.

**Fixed in the packager** (`ci/depot.py`): `runtime_dlls` sorts each import
by `x86_64-w64-mingw32-gcc -print-file-name`, stages the toolchain's ones
beside the exe — transitively, since `libgcc_s_seh-1.dll` and
`libwinpthread-1.dll` are `libstdc++-6.dll`'s imports and appear nowhere in
the exe's own table — and drops them from `requires.libs`. Beside the exe
so the shipped copy also shadows the player's 32-bit one.

**The leg is built too.** `nightly.yml`'s `depot` job is a two-platform
matrix now, mingw set to `-posix` on gcc *and* g++ with the threading model
asserted before the build, so a Windows depot is cut nightly by a recipe
that is read rather than remembered — it had been hand-cut on one box, which
is how this shipped. Its "what was packaged" step carries the assertion
`0.2.0` needed: bundled must be non-empty, every promised DLL must be in the
file list, no `libstdc++`/`libgcc`/`libwinpthread` may be left in `libs`,
and the notice must travel. Proven red against the published `0.2.0`
document and green against a repackage of it. Then the check no document
can make: the leg **runs the staged exe under wine** (`--help`, ~7 s, no
window), which fails with `loader_init ... c0000135` the moment the runtime
is not beside it. That is the first thing in this repo that has ever
verified a Windows build starts.

~~What remains is operator-only~~ — **done 2026-08-16 with v0.4.0**
(`DECISIONS.md`). The live `win-x86_64` row is `0.4.0-g193a8d2a6`: 140
files, the three DLLs staged beside `gates.exe`, `LICENSE-MINGW-RUNTIME.txt`
travelling with them, and no `libstdc++`/`libgcc`/`libwinpthread` left in
`requires.libs` for the player to find. Verified on the **served** document,
not the staged one. Notarized `0x4a1ac31e…`.

**What is still not measured is the thing this item is named for**: nobody
has started it on a real Windows machine. CI's wine leg is the strongest
evidence there is and it is not that — it is a cold prefix answering
`--help`, which exercises the loader and nothing after it. The next Windows
boot is the measurement; if it fails, the failure is past `loader_init` and
this item is the wrong file for it.

Unmeasured, same class: the GitHub **release** zip is msvc, not mingw, and
nobody has checked whether it needs the VC++ redist. Its notes list Linux's
three `-dev` packages and say nothing for Windows.

Also stale and **not ours to fix**: elo's own launcher manifest
(`/data/apps/scry-forge/data/launcher/gates.manifest.json` on morr) still
tells a player the Windows row bundles nothing and has never been run. A
player reads that row.

> **Playtest items, 2026-08-15 — the operator played the shard and five came
> out of it** (`DECISIONS.md` 2026-08-15). **Four landed the same day**
> (`0kit`, `0eat`, `0die`'s defect half, `0sun`) and `0dur` — the wall-6
> slice that could not land in the run that designed it — landed
> 2026-08-15/16, so each item below is what REMAINS of one.

## 0ctl · Four controls the player expects and the sim has no verb for *(systems lane)*

The 2026-08-16 control row copied the reference's scheme key for key. Six
bindings landed; **four were refused because they are slices, not keystrokes**,
and each would have been a key that does nothing — worse than an absent one.
Bind each **in the commit that gives it a verb**, never before.

1. **Reload (`R`).** No magazine, loaded state or reload verb exists in any
   crate. Firing spends an arrow straight out of the inventory
   (`ranged::draw`), and `ranged::hitscan` spends a round the same way, so
   the revolver fires but is never *loaded*. Needs a loaded-round
   state on the weapon stack — which is `0dur`'s per-instance `ItemStack` field
   question wearing a different hat, so **read that row first; the two should
   probably land together or agree on a shape.** `R` is repair until then.
2. **ADS / secondary attack (RMB).** No `BTN_SECONDARY`, no aim or spread
   state. The button is also fully spoken for — deploy-place, the build wheel,
   the inventory's half-stack grab — so this needs a held-item modality answer
   before it needs a bit. A new button bit is a `PROTO_VER` bump even though
   the octet does not move (`sim-core/input.rs` states the precedent).
3. **Flashlight (`F`).** No held light source. `item.torch` is an inert prop
   with no weapons row, and `tests/held_assets.rs::nothing_held_glows` forbids
   a carried emissive **by name** — so this starts by deciding that test's
   fate, which is the point of it. `F` is the ghost's level-down while the
   build wheel is up and free otherwise.
4. **Voice chat (hold `V`).** Nothing exists: no capture, no codec, no
   `KIND_*`, no fan-out. `reference/VOICE.md` §9 is the research and already
   settles the two design questions (it is not its own transport; a
   client-side attenuation of a broadcast stream is a wallhack).

**Also open, and smaller than any of the four:** the viewmodel sways with the
camera during free look, because it is parented to it and reads `eye.yaw`. The
reference tilts the held item instead. Cosmetic, and named in §open
"free look v0".

## 0kit · The rock landed; two doors and a boot rule remain *(systems lane)*

Landed: kit → rock + torch, the four swung nodes' `hand` rows **deleted**
(`hand = 0` is a refused boot; the bush keeps its only row), `gather::swing`
refuses a swing the node pays nothing for, `World::wake` re-grants, and
`shard.toml`'s `dev_spawn_kit` keeps the fat kit on a dev box. **Items 1
and 2 landed 2026-08-15/16 with the durability wire slice**: the refused
swing returns `Swing::Refused` and the raid arm declines it (gated both
ways in `tests/gather.rs`), and `EV_GATHER_REFUSED` names the held item —
*your Torch cannot harvest this* — through the pump, `client-core`'s ring,
the shared feed queue and `ui::refusals::GATHER` (wire v42). Remainders:

1. **Closed 2026-08-17.** All three of `wake`'s doors are gated now:
   `persist.rs`/`sleepers.rs` drive the dead restore and the dead-sleeper
   takeover, each proven red under a skipped wake and a deleted `grant_kit`.
2. **Closed 2026-08-17, both halves.** A kit holding no tool any swung node
   pays is a refused boot when no `hand` row exists (`validate::structural`,
   gated in `content.rs`, red-proven both ways), and `parse_shard_toml`'s
   `dev_spawn_kit` arm checks `MAX_SPAWN_KIT` at the push site — refuses,
   never truncates (gated in `config.rs`, red-proven).

## 0mk · Arrows leave marks; swings and paint do not *(systems+client lane)*

Landed 2026-08-16 (operator: *"when i hit a tree it needs a mark on it,
when i shot the ground it needs bullet holes"*): `EV_IMPACT` (wire v45)
carries where an arrow stopped and which of `ranged::step`'s three
predicates stopped it, and `render/decal.rs` draws it as a pooled
`ForwardDecal` — the first decal in the tree. The sim already computed
both and threw them away. Knobs and the full argument: `DECISIONS.md`
§open, "surface marks v0".

Three gaps, in the order they are worth closing.

1. **Landed 2026-08-18 for a struck node, and it cost no wire byte.** The
   premise here was wrong in a useful way: this said the mark needed the
   same missing fact as §0sw, and it did not. `EV_IMPACT` was never an
   arrow's fact — it is *a surface was struck at this point*, already
   broadcast, already carrying a quantized point and a surface class, and
   `render/decal.rs` was already its only reader. So `gather::swing`
   pushes it where a landed swing bit an occupant, on the skin
   (`occupant_volume` × slot scale) facing the swinger, at the shared eye
   height — no `PROTO_VER` bump, no golden, no client line. Three gates in
   `tests/gather.rs`, five red-proofs. **Still open: a swing at a built
   PIECE marks nothing** (`combat::raid` is the site, `SURF_BUILT` the
   kind) and it is deliberately behind item 2 — a piece mark faces the
   wrong way on a floor today, and volume on a known-wrong path is not
   progress. Flesh stays unmarked: `SURF_BITS` holds one spare code and
   spending it on blood is a deliberate act nobody has asked for.
2. **⚠ This item's symptom is UNREACHABLE, and the real defect is one
   layer down** (measured 2026-08-18). It says a mark on a floor gets a
   wall's normal — but no arrow has ever hit a floor: `collide::shot_blocked`
   (`collide.rs:1014`) calls only `cell_edges_stop_shot` and
   `cell_diags_block`, which read the wall/door/window/frame masks and the
   two diagonals, and **never `ColIndex::planes`** — the field
   `SHAPE_FOUNDATION | SHAPE_FLOOR | SHAPE_ROOF` live in (`collide.rs:211`;
   its only readers are `is_empty`, `piece_ground` and a test). So an arrow
   fired down inside a base passes through every floor and lands on the
   terrain as `SURF_GROUND`. **Fix that first** — a bullet that ignores
   floors is a raid defect, not a decal one — and only then the address on
   the message. What IS reachable in the facing arm today is a diagonal
   wall, 45° out. Bit accounting for when it is time: a full piece address
   is 27 bits, the message has 4 spare pad bits, so it grows to 11 bytes
   (`MAX_EVENT_MSG_BYTES` is 320, so there is room to grow, not room
   inside). `SURF_BITS`'s fourth value is dead-but-present, refused at both
   ends and pinned by the wire-domain table.
   Also open: a swing at a piece still marks nothing, and that is deliberate
   until this is settled.
3. **Spray paint is not this.** A player-authored mark that persists is a
   deployable, not a decal: a cap in `limits.rs`, a slot in
   `worldsave.rs`, a build-privilege question, decay, and — if the mark is
   painted rather than picked from N authored stencils — moderation
   forever. Decide stencil-vs-painted before any of it.

⚠ **NOBODY HAS SEEN A DECAL, and it cannot be checked on a headless box**
(measured 2026-08-18). The sim's half is confirmed: a swing at the boulder
beside spawn emits `EV_IMPACT` at `1024.74, 12.86, 1025.07` with `surf 1`,
0.98 m from the slot centre — that slot's scaled radius exactly — and the
capture probe now walks to a node, swings, and points the camera at those
coordinates. **The frame shows no mark.** Then the discriminating runs:
5.5× `SIZE_M` — nothing; the boot-time prewarm decal at full alpha, 1.2 m
across, flat on the terrain with an up normal — nothing. So **no
`ForwardDecal` renders under lavapipe at any size, alpha or orientation**,
and the arrow marks of 2026-08-16 have never been seen either.

That is a claim about THIS BOX and not about the game: a software adapter
is the environment that would degrade first, and the client logs *"Too many
textures in mesh pipeline view layout, this might cause us to hit
`max_sampled_textures_per_shader_stage` in some environments"* on boot,
which is the leading suspect since a forward decal adds a binding. **One
boot on a real GPU settles it** — swing at a rock and look — and that is an
operator act, not a loop's. Until then treat every decal in this tree as
unverified rather than as working or broken.

Also open, and cheap: nothing prewarms the *other* materials. `decal.rs`
pays `CLAUDE.md`'s shader-prewarm trap for its own pipeline and no module
else does, on a trap with no gate since `browser_smoke` went.

## 0sw · The swing is drawn in first person only *(client lane)*

Landed 2026-08-16 (operator: *"we need some animation atleast showing the
rock swing"*): `ui::swing::SwingCadence` mirrors `gather::swing`'s rule
(`BTN_PRIMARY` down, `tick >= next_swing`) over `Feed::server_tick_est`, so a
**miss** animates — previously only a landed hit or a gather did, which meant
the commonest swing in the game drew nothing. `Feed` stays as a backstop,
gated on the arm being at rest so a hit arriving mid-stroke cannot restart
the arc.

⚠ **This item said the other half was an ASSET gap and that was false**
(corrected 2026-08-18, and it would have cost a mesh purchase). The shipped
`assets/models/mannequin.gltf` carries **46 clips** including `Sword_Attack`
(1.5 s), `Punch_Cross` and `Punch_Jab`, all on the same 53-joint skeleton —
`MANIFEST.md` and `WANTED.md` both say 46 and only this line said
otherwise. Nothing needs buying.

**Landed 2026-08-18.** `EV_SWING` (broadcast, outcome-free, wire v47)
carries the swinger's id and nothing else — no position, because the
snapshot already says where every body is, and the one thing a client
cannot derive is that the arm moved. Pushed from `gather::swing`'s cadence
gate, the only line that runs once per swing whatever the swing finds, so a
**whiff animates** — which is the commonest swing in the game. `Clip::Swing`
is the first one-shot in `anim.rs` (`.repeat()` omitted; `RepeatAnimation::
default()` is `Never`), living beside the gait as `BodyAnim::swing_s` rather
than inside `clip`, which `observe` recomputes from speed every frame.
Gates: the role check on a whiff, a server routing gate proven red both
unicast and armless, the ring's drop-oldest identity, and a source scrape
that catches the four-literal array width that would otherwise panic the
first time somebody swings near you.

⚠ **Nothing has ever played the clip.** `client/tests/anim.rs` is a source
scrape by construction, `client-core/tests/wire.rs` stops at the ring, and
no headless test spawns a `SceneRoot`. The wire half is gated end to end;
the arc itself is unseen, exactly like the decal above. And the throughput
half of the fan-out is unpriced: `EV_SWING` is one broadcast per swing per
player with no AOI filter, so a 100-player shard swinging pays 100× the
per-client event rate — `raid_storm.rs` cannot see it, because that gate's
bots never press `BTN_PRIMARY`. A soak with swinging bots is the
measurement, and it is the same gap wall 4 has had since the soak landed.

Two things remain, and the first needs a word rather than work. **The clip
is `Punch_Cross` and the ask was a rock swing** — arithmetic, not taste:
the sim allows a swing every 1.267 s and `Sword_Attack` is 1.5 s, 1.68 s
with the blend, so every arc would be cut off by the next. Accept the
punch or shorten the sword clip (`DECISIONS.md` §open).
~~And the swing is silent for a remote.~~ **Closed 2026-08-18** — `Cue::RemoteSwing`, positional, §0pvp item 2.
The lane now exists for `Death01` and `Hit_Chest` too — **and neither used
it**: a death is a condition rather than an instant, so it landed as a wire
bit (v48, §0chr), and `Hit_Chest` landed attacker-side off `EV_HIT`'s
already-present victim id (§0pvp item 1) rather than as a broadcast. So the
unpriced fan-out above is still `EV_SWING`'s alone.

## 0die · Two questions to re-take, no defect left *(operator)*

Mechanism 3 — *"the kit is fresh-arm only, so you wake naked and stay that
way"* — is retired by §0kit's re-grant.

**Two mechanisms this item listed were wrong, in opposite directions.** (2)
said every kit item is `common` → ×1 → five minutes; `items.toml:79` makes
`item.metal_frags` `uncommon` (×4), so the *old* kit's bag lived **20** min.
(1)'s *"outside interest range"* is wrong outright — `EV_BAG_DROPPED` is
broadcast with no distance test. The real mechanism is `MAP_MARKS_MAX = 64`,
drop-newest, bags pushed **last** in `resolve_marks` with no owner filter, so
on a busy shard your own bag is the first mark the cap eats.

**Two things landed 2026-08-16 and they are different bags** — the word is
overloaded in this tree and the two items below were nearly merged by
mistake. The **death backpack** (`WireBag`, your loot) now outranks the
anchor tier at the mark cap: the wire carries no bag owner on purpose, so
`ClientCore::own_bag` joins the `BagDropped` against the dead predicted body
(`OWN_BAG_NEAR_M`) and `resolve_marks` pushes the tagged one directly behind
the authored tier — bags as a class outrank the bed/hearth mirror now, and
both maps draw in reverse resolve order so cap rank and draw-on-top rank are
one rule. The **sleeping bag** (`ARCH_BAG`, where you wake) got bag
choice v0 (operator; `DECISIONS.md`): the death screen offers the bag row
only if you own one and draws a map of your own beds — no corpse marker,
which is how `ALPHA.md` §1 survives it — off a new own-fact `SUB_BAGS`
(`PROTO_VER` 43), because `DeployRec::owner` is off the wire too.

Left, in order of cheapness:

1. ~~Beds did not get the ranking backpacks did.~~ **Done 2026-08-17**:
   `resolve_marks` takes `own_bags()` and pushes mirror beds matching an own
   anchor by address directly behind the own bag, ahead of every stranger
   tier — gated (`ui/map.rs::your_own_bed_outranks_a_shard_of_strangers_beds`,
   proven red against the unranked order). No wire, no word.
2. **Showing is not choosing.** The death map marks three beds and
   `ACT_RESPAWN` carries one bit, so the sim still takes the nearest ready
   one. Letting a player click the bed they want is a bag index on the action
   plus a `claim_bag` that honours it — a wire bump, and an operator call on
   whether the choice is wanted at all.
3. `SUB_BAGS` is sent **on a death and nowhere else**, so the `ready` bit
   ages while a player sits on the screen. Nothing is wrong today (the sim
   decides and `woke` says which anchor answered); re-send on the bed's own
   placement and removal if it starts to matter.
4. One operator call (`DECISIONS.md` §open, "death backpack v0"): whether
   five minutes is the intended floor for a common-only bag now the kit
   guarantees one.
## 0dur · Durability landed; the number is invisible *(client lane)*

**Landed 2026-08-15/16, all four questions closed** (`DECISIONS.md` §open
"item durability v0" has the ledger; the dated rows hold the answers).
`cond` on `ItemStack`, wear per (tool, node) as content, the Q4 dead-tool
guard, `EV_GATHER_REFUSED`, wire v42, save formats 3/7, eight gates each
proven red. What remains, in rank order:

1. **Closed 2026-08-17 — the bar is drawn.** All four cells that hold a
   stack call `pip_fraction` against the catalog's ceiling: the hotbar
   (`render/hud.rs`, a trough per cell spawned once and hidden when there
   is nothing to say), your grid, the container's, and the drag ghost,
   which had to have it because that tile is a copy of the cell it came
   out of. Colours are the vitals' own measured pair rather than a new
   one and there is no warning band (`DECISIONS.md` §open, "durability
   pip v0"). Two gates on top of §Q's four pure-value tests, each proven
   red: a call-site scan over both files, and one on the panel's redraw
   key, which watched `(item, count)` and now watches `cond` too — nothing
   wears with the screen open today, so that half is the door shut before
   repair walks through it. **Looked at**, which is the visual gate: a
   capture with a forced 35 % rock reads as a green bar in a dark trough
   inside the cell border, and the hotbar count badge lifts 3 px to clear
   it. NOT done: the detail pane still says nothing in words.
2. **Weapons and armour do not wear.** `reference/DURABILITY.md` §5 left
   both unsourced (per shot / when hit), so there is nothing to take yet —
   a research row, not a build item, and wear-on-swing-at-players is a
   mechanism question (`tools as weapons`, §open).
3. **Repair is v1 by decision** (Q3: re-craft is the repair). When a bench
   lands it is `Station::Workbench1..3` + a blueprint check, never a new
   deployable, and §3's 0.20 ratio stays DISPUTED until someone checks it
   against the in-game price.
4. **Landed 2026-08-17 — the save readers refuse un-mintable condition**:
   both boot paths now check `cond` against the baked ceilings, refused
   never clamped (`server/src/cond.rs` has the why; gated in
   `persist_store.rs` + `world_persist.rs`, each proven red).

## 0ps · Pieces wear a photograph and show damage — what is left *(client lane)*

From the operator's 2026-08-16 ask, with the reference's twig/wood/stone
foundations attached. **Two slices landed**, numbers in `DECISIONS.md` §open,
gates `client/tests/pieces.rs` + `sim-core/build.rs` §tests:

- **piece surface v0** — the tier table had three rows against the sim's four
  materials, so every piece drew one rung off and twig had no look at all.
  Fixed, and all four tiers now wear albedo + normal off the already-shipped
  CC0 maps, with metre-scaled UVs, tangents and a mean-1 face tint.
- **structure damage v0 (wire v44)** — piece and deploy hp were never on the
  wire, so `Target::damaged()` answered true for everything and the "not
  damaged" guard had never fired. A 3-bit band now rides both records,
  derived at the encode boundary: no `state_hash` change, no save bump.

Remaining, ranked:

1. ~~Nobody has looked at either, and the probe still cannot stage it~~ —
   **half struck 2026-08-20: pieces are photographed now, by somebody else.**
   The probe still cannot build (a piece is a verb behind a wheel and a
   material cost), and it turned out not to need to: `population = N` already
   seats bots that build a twig base over the real wire, `dev_spawn` makes
   them the camera's neighbours, and `dev_spawn_kit` pays for the wood. The
   capture harness grew a scene pass that finds the nearest base and the
   nearest body and points the camera at them (`7-player.png`,
   `8-build.png`); `ci/scene.sh` is the rig. Measured: eight foundations and
   floors up a hillside, from 4.8 m, on seed 20260731.
   **What is left** is the staged half — one row of one material, hit a known
   number of times, photographed at each band. The population builds what it
   likes, so damage bands are still luck. ⚠ And see §0mk: no decal renders
   under lavapipe at all, so a headless run cannot check surfaces that carry
   marks either.
2. **The catalogue is 11 shapes against the reference's 20** (`BUILDING.md`
   §7b.1) — no half/low wall, floor frame, steps, ramp, 3 of 4 stairs. Rule 6
   is silhouette before surface, so this outranks more material work.
3. **A base is a hundred identical walls at one rotation** (rule 7). Fix is a
   pool of per-tier variants (`uv_transform` offset + tint) by address hash.
4. **Trim** — lashings, plank seams, a capstone rim; `shape_parts` is the
   place, but price the entity count first at `MAX_PIECES` 8192.
5. **Deployables got the wire fix, no damage visual** (materials baked in the
   `.glb`), and nothing shows which face was struck.
6. **Twig wears bark; roughness maps still unwired** — a twig set via
   `CANDIDATES.md`, and an ORM packing step would serve terrain+props+pieces.

## 0bl · Pieces line up on a lattice now — what the stored plate would add *(client+sim lane)*

From the operator's 2026-08-15 screenshots (*"bad news about the building
system and pieces lining out"*). **Landed 2026-08-15/16** — build base
lattice v0, `DECISIONS.md` §open has the knobs and the three mechanisms:
one height implementation (`build::column_floor_y`, 0.5 m vertical lattice,
flushness bit-equal, two formula copies retired), the terrain-following
foundation skirt (`structures::foundation_part`, one emit for piece and
ghost), and the ghost aimed by the LOOK ray (`place::aim_from_look`)
instead of `feet + yaw·3.5`. Gates: `sim-core/tests/base_lattice.rs`,
`client/tests/ghost.rs` §footing, `place.rs` §aim. Remaining, ranked:

1. **Nobody has looked at it.** Every claim is arithmetic; the screenshots
   that opened this deserve their counter-shot. Boot, build a row on the
   same hillside, look.
2. **The stored plate is the real v1** — the reference's model: first
   foundation pins a height, neighbours latch to it, too-high/too-low
   refusals, stilts past one band. Costs a wire field + save bump + mirror
   change (§open row prices it). Until then a slope steps every `q/slope`
   metres and the player cannot choose where.
3. **A band-boundary wall bases on its canonical cell** — it can hang one
   band over the lower plate (an arrow-sized slit under it). The lower of
   its two columns is the honest base; needs `collide` + render together.
4. **The skirt draws and does not block** — walking into it from downhill
   clips through. Piece side collision is its own slice.



Landed: `to_sun` takes the **hour** and derives both coordinates, so no
caller can pair this morning's height with this afternoon's bearing.
`RIG_SUN_ARC = π`, derived from our own equinoctial elevation arch
(`DECISIONS.md` §open, "sun arc v0"). Noon is bit-identical, so no judged
frame became incomparable. The coupled set was taken as one owner; the
**cloud deck** was the one member that broke, and `sky::deck_rotation` fixes
it exactly rather than by rebaking 393 k texels.

1. **The deck fix has no gate — the highest-value thing here.** It is one
   call site (`rig.rs:594`) and no fixture in `crates/client/tests/` ever
   constructs a `Skybox`, so `day_night`'s `if let Some(mut sky)` branch is
   dead in every suite: deleting the line leaves all 28 green, proven.
   `daynight.rs`'s own fixture comment records this silent-no-op one
   component earlier. In-lane fix: a `MinimalPlugins` app with `Sun` +
   `EyeCam(AmbientLight, EnvironmentMapLight, Skybox)`, run `day_night` at
   two hours, assert `Skybox::rotation`. `sky.brightness` is ungated the same
   way and the same fixture closes both.
2. **Look at the sky before anyone builds more of it.** The deck now turns a
   full revolution per cycle at 5.7°/min about the vertical — horizon band
   fastest, zenith pivoting in place, the opposite signature to advection.
   Exact and gated as arithmetic, but a visible change nobody asked for, and
   there is no pixel gate by policy.
3. **The noon bearing is southwest** (225.4°), so the path is SE → SW → NW.
   Moving it moves noon and retires every judged frame — its own pass, a
   re-capture, and the operator's word. `capture.rs:79` spells the sky
   vantage's yaw as the literal `2.35` rather than `RIG_SUN_AZIMUTH`, so if
   the pin ever moves that vantage stops looking at the sun.

## 0gp · The four ground identities are three paints *(client lane)*

**GAP PASS item — `pass-20260815-042118-10-visual.md` ranked gap 1** ("the
island is one tan material with no edges"). **The albedo half landed 2026-08-15.**

The judge's mechanism was right and its cause was not where it looked. A probe
over 34,806 land samples at the capture spawn: `splat_from` is *not* mushy
(max weight p50 = 1.000, 92.2% above 0.8) and reproduces `ART.md` §0's granite
share to the digit (8.89%). The island was tan because **forest litter and
granite were 1.059× apart in value and 1.0° in hue while owning 89.4% of the
land inside 300 m** — four identities, three paints. Re-placed onto §3's own
luma column (sand 117.0, turf 64.5, granite 147.0; litter 102.8 absorbs the
mean): granite:turf 1.91× → **2.28×**, granite:litter 1.059× → **1.429×**,
brightness-neutral to −0.024% so the coupled owner keeps brightness. New gate
`granite_stands_clear_of_the_ground_it_shares`, red on the old constants;
`ground_mix.rs`'s debt test is now a pin on the held mean. Knobs:
`DECISIONS.md` §open "ground identity separation v0".

**The larger half landed 2026-08-15 — the splat material.** Each identity
carries its own photograph now: `assets/shaders/ground_splat.wgsl` (the first
WGSL in the tree) + `render/ground_splat.rs`, four albedo and four normal maps
on one shared sampler, per-identity roughness where one shared 0.92 stood.
Measured at a pinned `dev_spawn` with the mix stated — 1500,600, litter 611‰,
rock 329‰ — **near-ground neighbour contrast 6.43 → 8.53, +32.8%**, every frame
improved. `tests/ground_splat.rs` is the gate.

**Two of the three things scouted here were right and one was wrong**, which is
worth keeping because the wrong one looks cheaper and someone will re-propose
it. The `#[bindless]` route is right and is what shipped. Height blending is
right *and measured as a no-op* (+0.1%) — `splat_from` is near-binary, so the
band it arbitrates is a sliver; it is kept as insurance. **The packed-`UV_1`
route is broken**: the rasterizer interpolates the packed value, so
`floor(p/256)` mixes the low byte into the high one — exact at both vertices,
50% wrong mid-triangle, i.e. at identity boundaries. The weights ride
`ATTRIBUTE_COLOR` and the two scalar modifiers ride `UV_1` instead, which also
made the identity mix per-pixel.

What is still open here, in rank order:

1. ⚠ **It costs 8.0% mean luma and that is not this material's to spend.**
   Granite having granite's relief means more self-shadow; the number belongs to
   the coupled tonemap/sky/exposure/fog owner (`CLAUDE.md` traps), so it is a
   debt against §0fill rather than something to correct here.
2. **The projection is still planar XZ, not biplanar** — a vertical face still
   stretches. `RENDER.md` R4's remaining half.
3. ~~The roughness maps are still unread~~ **LANDED 2026-08-16** — bindings
   110–113, sampled per texel, blended by the same `bw` as colour and normal,
   plus wet-band smoothing. Four textures and **zero** samplers: samplers are
   the axis with the 16 floor, textures are not, and Bevy asks the adapter for
   its own limits. ⚠ **No detectable effect on the frame** (contrast −0.4%
   over six vantages, inside the harness's own ~0.3% run-to-run spread, which
   this pass measured — `RENDER.md` §5) and landed anyway, because the cause is
   item 3b and the ordering only runs one way.
3b. **The ground's specular is off: `reflectance: 0.18` → F0 = 0.52%**, where a
   dielectric is ~4% (`F0 = 0.16 × reflectance²`). Roughness shapes the
   specular lobe and nothing else, so the maps have almost nothing to shape.
   The constant is an undocumented one in `terrain_mesh::ground_material`. **It
   is the coupled lighting owner's** (§0fill), not the ground material's, and
   it is now unblocked: raising it over one shared roughness makes the island
   uniformly shiny, raising it over the per-texel field that now exists is the
   fix. `DECISIONS.md` §open "ground specular v0".
4. **`ground_detail.jpg` is now loaded by nothing** — it is grass's baked
   luminance field and the shader computes the same thing from `grass_albedo`.
   It still ships and is still gated as a file; deleting it is a separate call,
   because a pre-baked field is what a cheaper LOD would want.
5. ~~`ui/map.rs` carries an independent minimap palette that nothing holds~~
   **GATED 2026-08-16** — `crates/client/tests/map_palette.rs`. Hue tracks to
   within 2° on three of four, value span is properly compressed, and the two
   value-order inversions are pinned by name. One is convention (woodland
   darker than meadow); **one is the drift this item suspected** — granite
   passed beach sand on the ground in the 2026-08-15 re-place and the map's
   `ROCK` did not follow. Fixing it departs from a `mapraw.jpg` reading, so it
   is an operator call: `DECISIONS.md` §open "minimap palette v0".
6. **The five PROP roughness maps are still unread, and their recorded reason
   is false too.** `props.rs` repeats the ORM story; Bevy computes
   `metallic *= metallic_roughness.b` against a `metallic` that defaults to
   **0.0**, so a greyscale map in that slot cannot make anything metal. What it
   actually needs is a level decision — `perceptual_roughness` is a *multiplier*
   there, so shipping the map whole means factor 1.0 and losing the authored
   `rock 0.88` / `ore_stone 0.80` distinction, while mean-placing it wants
   0.88/0.611 = 1.44 and Bevy clamps at 1.0. Not slipped into the ground
   slice: one owner per pass, and it is a look question rather than a
   binding one. (`viewmodel.rs` is not in the way — its `wood` material
   takes `PropMaps` at the default `metallic: 0.0`, and the `steel` one
   that runs `metallic: 0.55` carries no maps at all, by the sourcing gap
   recorded beside it.)

## 0tree · The research ladder exists — what it is still one edge short of *(systems lane)*

From `pass-20260815-042118-07-judge.md` ranked gap 1 ("a session has no
ladder"). **Landed 2026-08-15, and the cause was deeper than the gap.**
`bake_research` had no caller: a live shard ran `ResearchContent::EMPTY`,
every `Command::Research` refused `REFUSE_R_ITEM`, and the six
`blueprint = true` recipes were uncraftable by anyone — with every gate
green, because no gate booted a shard and asked what it installed. Wired,
plus the ladder the gap asked for: `requires` on `[[research]]`, resolved
at bake into a `Player::known` mask, `REFUSE_R_LOCKED` checked before the
price. Gates: `crates/server/tests/boot_tables.rs` (4, all proven red on
the original defect), `content.rs` +5, `sim-core/tests/research.rs` +2.

What remains, in order:

1. **The tree is one edge deep**, and honestly so — only the satchel's
   dependency on gunpowder is implied by `recipes.toml`. Revolver-behind-
   gunpowder is a pacing call nobody spoke (`DECISIONS.md` §open, "research
   ladder v0"); the bench tier that would carry the rest is §0tt, where the
   era is also unspoken. **Do not invent either.**
2. **No blueprint ITEM**, so learning stays instant and personal and there
   is nothing to trade — the judge named this as the half that makes another
   player's progress interesting. Unbuilt, and it is a wire change.
3. ~~Twelve other tables threaded positionally~~ — done, `net::SimTables`.
4. ~~The mask was lost at four doors and unreadable at the fifth~~ — done
   2026-08-15. `die`/`wake`/`seat` cleared `known` via `..Player::default()`
   (dying and reconnecting each deleted every blueprint bought with JUNK),
   and `SUB_RESEARCH`/`SUB_RESEARCH_REFUSED`/`SUB_KNOWN` had encoders,
   `EventMsg` variants and `ClientCore` handlers but **no `decode_event`
   arms** — every research frame the server ever sent decoded `Malformed`.
   So the verb was dead wire on top of a sim that was correct and tested,
   which is `boot_tables.rs`'s defect one layer out: nothing asked whether
   a frame we send can be read. Gates: `every_encoder_has_a_decoder`
   (protocol, found two of the five), `every_player_field_is_classified_
   across_a_death` (persist), the `EV_KNOWN` role check driven through a
   real starvation death, and `seat`'s restore arm now names every field so
   the **compiler** refuses the next omission. All proven red.

   One thing it leaves. **Nobody has seen the research panel work** — the
   client half is unverified past `decode_event`, and no pass since has
   been able to capture.

5. ~~The lane had no byte pin, and no gate could notice~~ — done
   2026-08-15, and it was the **third** instance of this item's own defect
   class. `ACT_RESEARCH`/`SUB_RESEARCH`/`SUB_RESEARCH_REFUSED`/`SUB_KNOWN`
   landed at v32 and reached v37 with no golden fixture; `FIXTURES` is a
   hand-written manifest, so `test_protocol_golden` was green over four
   unchecked encoders. Two gates that check a **set** rather than a value:
   `every_encoder_has_a_golden` (all 64 encoders must be written by
   `gen_goldens`, no exemption list) and `every_action_encoder_has_a_decoder`
   (`event.rs`'s pairing gate in the direction nobody asked — green on
   arrival, 18 codes/18 arms). Plus the four fixtures. All proven red; no
   `PROTO_VER` bump owed or taken, and none of the 86 existing fixtures
   moved a byte.

## 0fill · The darks, second half: the transfer *(client lane)*

From `pass-20260815-042118-01-visual.md` ranked gap 2 ("put the darks back and
ground every object — one owner, one pass"). **Its first half landed
2026-08-15**: the fill is a hemisphere now (`render/fill.rs`,
`tests/fill.rs`), so down-facing faces get the ground's own warm bounce at
0.60 of the sky half instead of a blue sky they are not looking at.

**What that half deliberately did NOT do, and why the gap stays open.** The
sky half was carried across unchanged — that is what made it safe to land
blind, since up-facing ground could not move — so the measured p10 (79.9
against `ART.md` §3's 49) is untouched. Cast shadow on *open* ground is an
up-facing surface; no hemisphere darkens it.

The remaining half is the **transfer**, and it needs eyes on a frame:

- The rig's floor arithmetic is written in the wrong space. `rig.rs` set
  `fill = 0.30 × sun_on_flat` to satisfy rule 3's "shaded ≥ 0.30 of lit", but
  the delivered *linear* ratio is `fill/(sun+fill)` = 0.229 — under the floor
  it was aiming at — while the judge measures 0.725 in *display* luma. Rule 3
  is a pixel ratio; the constant was derived as an illuminance one. Both
  readings cannot be acted on at once and they point opposite ways.
- So the lever is the tone curve, not the fill: TonyMcMapface's gentle
  roll-off plus the 0.8-stop exposure lift is what puts a 0.229 linear ratio
  at 0.725 display and leaves 0.00–0.12% of pixels under luma 30 against a
  reference median of 4.14%.
- **Do not do this blind.** It is the coupled set (`CLAUDE.md`: three parallel
  passes 60→66, one sequential owner → 26), and the last time this rig moved
  toward "too dark" the correction overshot the other way. One owner, one
  iteration, with the frame open.
- **Passed over 2026-08-15** (the §0gk pass), for the line above and no other
  reason: that runner took frame capture out of the builder's hands, so the
  pass could not open one. It stays top. It is blocked on a *capability*, not
  on priority — a pass that can capture should take it before anything below.

## 0gc · A blade is shaded exactly like the dirt it stands in *(client lane)*

From `pass-20260815-042118-03-visual.md` ranked gap 2 ("nothing grows on the
ground and nothing is grounded"). **The chip half landed 2026-08-15** —
`CHIP_VOLUME_BLEND` and `CHIP_SINK` in `clutter.rs`, gate
`crates/client/tests/contact.rs` (4 tests, both constants proven red at 0.0).

**What is left is the blades, and the cause is now known.** `blade()` blends
every normal fully to +Y. That is the GROUND's normal, so a blade takes the
same sun cosine and the same `fill_at` sample as the dirt under it, and albedo
is the only thing separating grass from ground — "reads as paint" as
arithmetic. The quad's own facet is `lean/√(1+lean²)` off horizontal, i.e.
0.215–0.489 vertical over the drawn range, so the forced blend discards the
dominant component.

**The reason the code gave for it is false** and `tests/contact.rs` §winding
computes that: the two triangles do *not* wind opposite ways (dot > 0.99 over
a 128-case sweep). The real blackener is the tile material's `double_sided`
flip — Bevy negates the shading normal on a back-facing fragment, and seven
blades at seven yaws put half of them there.

So the fix is **not** a blend constant: it is a per-vertex ramp, ground normal
at the root to the blade's own facing at the tip, which needs `Soup::tri` to
take a blend *function* rather than one `f32`. **Do not land it blind** — it
is a shading change and this pass could not open a frame. Knobs:
`DECISIONS.md` §open "clutter contact v0".

## 0gi · The island reads as one surface — two causes, one landed *(client+sim lane)*

From the visual judge's ranked gap 1, pass `20260814-142610-01`: *"the whole
island is hue 29–35°, and zero pixels of `ART.md` §3's grass band (63–74°)
exist on the ground"*. Measurements in
`gates-loop/findings/note-20260814-the-island-has-one-hue.md`.

Landed 2026-08-14: `GROUND_ALBEDO` re-placed against §3 — litter was the most
saturated surface on the island and warm, so it took the hue of every mix, and
it is 37.6% of the land. Brightness is gap 2's owner (`rig.rs`), so the ground
mean was held rather than moved — **0.0495% under Rec.601, 1.20% under
Rec.709**, and the "±0.01%" this line used to claim was neither. The repo uses
both estimators (`ground_identity.rs:139` 601, `terrain_mesh.rs:209` and
`water.rs:714` 709) and they disagree by 24×; the constraint is also the one of
four with no gate. Gate `crates/client/tests/ground_identity.rs`, 5 tests, 4 red
on the old constants. 58.1% of land in the grass band, p10 38.0° / p90 68.5°.

Remaining, in order:

1. ~~Granite is authored and never drawn~~ — **struck, and the reason it was
   struck is itself retracted (2026-08-14, both the same day).** It is not that
   the shipped island is a flat 1-in-40: that came from sweeping `-1024..1024`
   on a world centred at (1024, 1024) — **one quadrant**, and not the one the
   camera stands in. Over the whole square seed 20260731 reaches 106.00 m,
   slope 2.665 and granite on **10.0%** of its land (44-island median 7.2%),
   and **8.9% within 300 m of the capture spawn**, where the median island
   paints 0%. Granite is authored, reachable and near the camera. No seed
   moved. The bands still may not: they are ramps centred on
   `CLIFF_SLOPE_RATIO` and `biome()`'s Highland edge, and
   `crates/sim-core/tests/relief.rs` is red under that edit and now also under
   the quadrant window's return.
2. ~~The missing green is the renderer's~~ — **struck 2026-08-14. Nothing eats
   it.** Gate `crates/client/tests/ground_where_the_green_goes.rs`, 5 tests,
   red under two inversions; measurements in
   `gates-loop/findings/note-20260814-where-the-green-goes.md`. The material
   side is exactly hue-preserving (worst 0.000061° over 39,300 samples, in
   LINEAR space — the encoded space reads 0.262° and that is the sRGB curve,
   not a tint), and `ground_detail.jpg` is neutral at every percentile. The
   island is a **mosaic**: `SPLAT_MOIST_BAND` is 0.08 wide across a moisture
   field spanning ~0.9, so 48.8% of land reads as grass alone, 34.7% as litter
   alone, only 7.0% blends — and the two are 30.5° apart in hue.
3. ~~The capture probe's ground colour is a draw~~ — **struck 2026-08-15, both
   halves pinned.** The *place* was already pinned outside this repo:
   `gates-loop/art/capture-native.sh:44` writes `dev_spawn = 1155,140`, so the
   frames stopped riding `spawn_pos`'s per-id bearing draw before this item was
   read. The *hour* was not, and it landed here: `rig::DayPin` pins a
   `--capture` run's tick to noon — the one fraction where `sun_elevation`
   returns `RIG_SUN_ELEVATION` exactly. It pins the **tick**, not the sun,
   because `render/audio.rs` reads the same clock through `is_night`.
   Measured: a capture shard boots at tick 0 and the probe fired after the
   build, so the sun was **24.5° at tick 0, 27.3° typical, 30.4°** on a slow
   box — a 5.9° swing, at or below `ART.md` §1's 30–40° band, rising with build
   time. Gate `tests/daynight.rs` +5 (8 total), 4 red unpinned and 3 red on a
   wrong pin. Knob: `DECISIONS.md` §open "capture clock v0".

   **What the next pass must know: the tonal baseline moved.** Every `-visual.md`
   before this one was shot at 24–27°, so its luma, sky and shadow numbers are
   not comparable to the next report's — the first frames at the authored
   register are the ones the runner captures after this merges, and nobody has
   looked at them yet. Do not read a brightness delta in the next report as the
   effect of a render change.
4. **The judge read real geometry as paint** — *cause found 2026-08-15, and
   this line was wrong about it.* "No SSAO anywhere" is false: `rig.rs:223`
   carries `ScreenSpaceAmbientOcclusion` at Medium, Bevy 0.18 auto-requires
   its two prepasses via `#[require]`, and Bevy seeds `required_limits` from
   `adapter.limits()` rather than wgpu's default 4, so it clears the `< 5`
   storage-texture refusal and loads. The paint read is `clutter.rs`'s
   NORMALS, not a missing AO pass — §0gc. What is genuinely missing is an
   occluder at blade scale (`NotShadowCaster`); `ART.md` rule 2.
5. **The mosaic is not itself a defect, but litter wins every mix.** Grass is
   the darkest identity and litter 3.2× brighter, so grass needs **≥82.1%** of
   a blend to still read green. That is why the boundary never reads as grass.

## 0pop · The shard has inhabitants — what they cannot yet do *(server lane)*

From `findings/pass-20260813-230343-18-judge.md` §B.1, ranked first by three
consecutive judges: *"a shard has no inhabitants, so none of the last four
passes' work is reachable by a player."*

**Landed 2026-08-14.** `shard.toml` grows `population = N` and
`crates/server/src/population.rs` seats it — bots dialled over the shard's
own wire after the bind, full handshake, `run_bot`, so the server cannot tell
one from a player. Resident rather than a fleet: a post runs a bounded shift,
reports its `BotReport` into a gauge, and is re-manned until the shutdown
flag. Bounded at `MAX_PLAYERS - 1`, so a seat always stays a person's, and
refused outright beside `require_auth` (an inhabitant is a guest, so an
authenticating shard would have it re-dial its own closed door). Gate:
`crates/server/tests/population.rs`, 8 tests; the live one seats 4 on an
ephemeral shard and waits on `joins`/`input_dg_ok`/`actions_ok`, observed red
with no post manned (`joins 0, live 0, inputs 0, actions 0`).
Measured, 4 seated: joins 4, inputs 10, actions 3, 0 malformed, one 100 ms
look. `bin/bots`'s row resolver now calls the same one.

Remaining, ranked:
1. **Nobody has run one for longer than a test.** A shift is 300 s and the
   suite exercises ~0.2 s of it, so re-manning, the backoff and the shift
   report are gated only by construction. Cheapest next step: `population = 8`
   in `shard.toml`, run it, read the population line.
2. **They act, but nobody has checked what they can afford.** The suite uses
   the shipped spawn kit deliberately and asserts only that actions land —
   judge -18 §B.2 is the live half of this, and the satchel is still granted
   everywhere rather than crafted.
3. Two proposed defaults are in `DECISIONS.md` §open ("shard population v0").

---

## 0rc · The raid completes — what is left of it *(systems lane)*

From `findings/pass-20260813-230343-16-judge.md` §B.1, ranked the largest
playable gap in the game: *"You cannot get into anyone's base."*

**Landed this pass.** The cheap next step named by
`findings/note-20260814-charge-never-detonates.md`, taken: the detonation is
gated in the sim on *shipped* content — `crates/server/tests/raid.rs`, twig
foundation at hp 10 against `structure` 125, the real 300-tick fuse. It went
**green first run**, so the verb was never broken. That clears that note's
suspect list: `detonate`'s scan, the `find_index` re-resolve, the overkill
case (`dealt` is clamped to the piece's 10, not the charge's 125), the fuse
length. Also read and cleared this pass: the wire encoder's `EV_STRUCT_HIT`
arm, and any early sweep of a live charge — the only writes to
`World::charges` are `place`, `tick_fuses` and the save restore.

**The arrangement is cleared too (2026-08-14).** The instrumented run this
item asked for is `crates/server/tests/raid_shape.rs` — the wire's seating,
walk, one-action-per-tick cadence and one-frame hotbar selection, replayed
into `World::tick` for 905 ticks with no socket. **It raids: 21 plants, 12
`EV_STRUCT_HIT`, first breach tick 355.** So both things this item suspected
are wrong. *Attacker and owner never share a plot* is true — measured as an
integer for the first time, `peak_shared_plot == 1` — and does not stop the
raid, because an attacker plants on the foundation it laid four steps earlier
and a blast is area-not-address. And the plants do not cluster late: first at
tick 55, 17 of 21 due inside. (The shared-plot gap is still real as *design* —
judge -17 §B.3 — but it is not the explanation, and the two were one thing.)

**What remains, ranked.** All wire-only, since the harness is the optimistic
case and still raids.
1. **Dropped actions skip, not retry** — `core::wants_action` takes one per
   client per tick; a lost step 4 leaves step 5 throwing at nothing.
2. The jitter buffer's held-item timing. Cannot be the whole story: 27
   charges did arm. Chain:
   `findings/note-20260814-the-arrangement-raids.md`.

**Struck 2026-08-14, do not re-run it** (judge -18 ranked fix 1, checked
here). This list's old #1 said 905 ticks "was `30 s × TICK_HZ`, not a
reading". `TICK_HZ` is 30 (`limits.rs:15`) and 30 × 30 is **900**, so 905
cannot be that product; 905/30 = 30.17, which is the "30.2 Hz" the note
quotes, i.e. the Hz was derived *from* 905 and 905 is the measurement. The
window held.

---

## 0rs · The bots raid on the wire — what a naked raider cannot reach *(systems lane)*

From `findings/pass-20260813-230343-{13,14}-judge.md` gap 1, *"a player has
no opponent"* — ranked 1 in both. Item 1 landed 2026-08-14.

Landed before: `bots::raid_step` + `test_raid_storm`, driven straight into
`World::tick`. Landed now, the wire half: `botclient.rs` derives its plot
from **its own body** (`build_cell_of`, re-seated every `RAID_CYCLE`, so a
walking bot is not stuck out of reach), feeds `raid_step`, and writes the
frame through the same `encode_action_*` the native client calls — the
server cannot tell a raiding bot from a player. `bin/bots` raids by default
(`walk` restores the old behaviour) with rows read from `content/` by id.
Gate: `test_bots_raid_over_the_wire`, proven red three ways — a constant
plot cell, a suppressed write, a `raid_step` verb with no encoder arm.
Measured, 8 raiders × 4 s: ~110 actions each, 12 plot re-seats, plots
scattered, 0 unencodable, 0 malformed server-side, and the sim answered
47–59 build + 23–48 deploy + 12–23 move refusals apiece.

Item 1 (the fleet could not afford to play) landed 2026-08-14: a fixture
raid kit — satchel / box / wood / lock, every index and count read out of
shipped content, at the slots `RaidRows` addresses. The owner cycle now
builds and locks for real. Measured, 8 × 4 s: ~66 pieces placed, 3 deploys,
18 charges armed, `auths` 0..15 per owner — all four flatly 0 before. Proven
red three ways: a naked fleet, a kit whose layout drifts from `charge_slot`,
a dropped `ChargePlaced` arm. `struct_hits` did **not** move; that is §0rc,
not this item. (Cited as `§0rf` until 2026-08-14 — a label this file has
never had, in all three places, judge -18 ranked fix 2.)

Remaining, ranked:

1. **Bodies are out of the storm** — the throwable's `damage` is 0, so the
   raid never kills and `MAX_BACKPACKS` plus the death/respawn ring are the
   one client-driven family it misses.

(Item 2 — `CLAUDE.md` wall 4's ⚠ claiming `test_raid_storm` does not exist —
is struck: corrected in both places 2026-08-15 by the run's doc owner, with
the correction itself dated so a reader can see which way the error ran.)

---

## 0tq · The HUD says every fact of a frame, not the last one *(client lane)*

**Gap pass, from `findings/pass-20260813-230343-10-judge.md` ranked gap 2**
("the game speaks one sentence per frame and silently drops the rest").

Landed 2026-08-14: `hud::Toast` is a bounded queue — `TOAST_LINES = 4` rows,
newest at the top where the single line always sat, one clock each,
drop-oldest and counted. The same report's ranked fix 1 was the proof it
mattered on shipped content: a tree pays a secondary, so one swing into a
full pack says two spill lines and the single slot showed the mushrooms and
ate the wood. Five hud tests, each proven red under its own revert. Also
that report's ranked fix 2 (a positive control in `spill.rs`, so half one's
three negatives cannot go green on a swing that missed) and fix 3b (the
word "measured" in `DECISIONS.md`).

Landed 2026-08-14 (second slice, judge ranked fix 3 of pass -11): eviction
reads a `Rank` instead of a position. A line is an `Alarm` when the fact
dies with it — a refusal, a spill, a charge going live — and the cap eats
every recoverable `Note` before it touches one; only an all-alarm stack
falls back to oldest-outright, and a push is never refused. Drawing order
is still recency, so a frame under the cap is unchanged. 29 sites moved to
`warn`. `dropped` has a reader too: `unseen` counts the burst, clears when
the stack empties, and rides the last live row as a suffix (`…+2 more`) —
a suffix because the bottom row is where a rescued alarm now sits. Four
tests, each observed red under its own revert. Left:

Landed 2026-08-14 (third slice, judge ranked fixes 1–3 of pass -12): the
arithmetic half of "nobody has looked at it". The layout numbers were
literals inside `setup` — the two spawns computed one rule twice — and are
now named constants and four derived functions the spawns call, gated by
four tests, each observed red under its own revert. What they found: the
pitch is a **percent** of window height and the type size is **px**, so the
stack self-overlaps below a 600 px window against the 720 the client opens
at (read off `Window::default()`, not typed) — a 120 px margin nobody had
computed. Also that `TOAST_LINES` × `TOAST_ROW_DIM` multiply: at 8 rows the
deepest draws at alpha 0, so the cap would hold a line nothing can show.
Plus the two cheap fixes — a repeat now raises a line's rank and never
lowers it (the one write path that skipped the field eviction reads), and
the three places naming the old drop-oldest policy now name the shipped one.
Left:

- **Nobody has LOOKED at it**, and this is now the whole of what remains.
  Same as §0sp2's last bullet: no frame in this repo has ever shown one
  line, let alone four with a `…+N more` suffix. What the gates above
  cannot answer is whether 0.52 alpha on the deepest row reads, and whether
  the suffix — appended into a centre-justified row — shifts the sentence
  under the eye. Needs a capture with a forced five-fact stack; the probe
  has no way to force one, and a frame this loop scores itself is
  diagnosis, never evidence.

## 0sp2 · The spill speaks now — for the whole of one, not part of one *(systems lane)*

Landed 2026-08-14: six producers, one drain (`World::drain_spill`), and the
same day the signal, which was this item's own open half. **It was a
client-side read of facts already on the wire, and the answer is written
down now rather than guessed at** — the zero was always there
(`EV_CRAFT_DONE` has declared "0 = full inventory" since it landed) and the
client discarded it, gather on an `if added > 0`, craft by printing
`crafted 0 × Stone Hatchet`, which said a craft had failed that had in fact
succeeded and was on the floor. Cost: one guard in `gather::swing` so the
zero has exactly one cause (a swing the cumulative schedule owed nothing
produced the identical event), a ring in `client-core`, and the HUD line
*"pack full — Wood dropped at your feet"*. No wire change, no version bump,
no knob. Left, and the first two need a wire field:

- **A partial spill is still invisible.** Some fits, some does not, and the
  shortfall never leaves the sim — the wire carries what reached the hands
  and never what was paid, so `+3 × Wood` cannot say the other 7 fell.
- **The four give-backs say nothing at all** — demolish refund, pick-up,
  unbolt, craft cancel emit no payout event, spilled or not. Operator:
  those two together are what a wire field buys (`DECISIONS.md` §open).
- **The merge ignores ownership** — a spill lands in whatever bag is
  nearest, including someone else's death bag. §open carries it.
- **Nobody has seen one.** Judge gap 1 stands: proven headless only. The
  new line included — no frame in this repo has ever shown it.

## 0wc · The crate opens — what world containers v0 left *(systems lane)*

Gap-pass item, from the merge-gate judge's ranked gap 1
(`findings/pass-20260813-230343-04-judge.md`): the destination gradient was
fully built and gated and *paid nobody* — `loot.rs:33` said "No verb opens
one yet." Landed 2026-08-14: `CONT_WORLD` is a fourth container kind, the
open re-derives the cell through `terrain::scatter`, the refill is lazy
inside `open` so the store costs the tick nothing, and the crate rides the
existing move/refusal/sync path (`DECISIONS.md` §open "world containers
v0"; wire v37; save format 5; `tests/worldcont.rs`, 17 checks).

**The panel was wired to the wrong store, and it shipped green
(2026-08-14).** The server's per-tick container drip dispatched the kind as
`if kind == CONT_BAG { backpacks } else { deploys }` — true for the two
ground kinds alive when it was written, and silently false the day
`CONT_WORLD` landed: the crate's panel read `deploys.box_slot` with a
`world_conts` index. It cannot panic (64 world containers index safely into
1 024 deploys), so **opening the pad's crate drew an empty panel over four
units of loot**, with 17 sim checks and 86 protocol fixtures green over it.
Fixed by making `World::cont_slot` — which had all three arms — `pub` and
the drip's only answer; `container_wire.rs` gains
`a_world_crate_is_drawn_from_the_crate_store` (proven red under the old
dispatch: `left: []`) and a `CONT_MAX` compile guard so a fifth kind breaks
that file until someone covers it. Two stale protocol claims went with it
(the kind field saturated at v37; `kind > CONT_MAX` now refuses nothing).

Owed, in rank order:

1. **Nobody has opened one in the running game — and this pass is why that
   matters, not why it is settled.** A headless test found the defect
   above, but only because someone went looking; the reason to look was
   that no one had booted it. Still unverified with the client attached:
   the prompt string, the panel title, the drag out of a 30-slot grid, what
   an emptied crate looks like. The capture probe **cannot** substitute —
   `VANTAGES` (`render/capture.rs`) is yaw/pitch from the spawn eye with no
   position, so it can only ever photograph wherever the player already is.
   **Standing the probe at the pad is NOT unbuilt work** — this line said it
   was and the judge measured it false (pass -07, ranked fix 1):
   `shard.toml dev_spawn = "x,z"` is parsed (`config.rs:283`), carried into
   the world (`net.rs:1618`) and returned ahead of the spawn ring
   (`world.rs:1213`). Derive the crate's anchor the way `a_pad_crate` does,
   put it in `shard.toml`, boot. That is the cheapest route to the
   verification this item asks for.
2. **An emptied crate says nothing at a distance.** The only way to learn
   the pad is farmed out is to walk to it, which makes a wasted trip the
   normal case once a shard is populated. Wants either a visible lid
   state on the mesh or the refill window shortened; the mesh is
   `render/props.rs`, the knob is in §open.
3. ~~The prize is still unguarded~~ — **landed 2026-08-14** (site guards
   v0, `DECISIONS.md` §open; `sim-core/tests/guard.rs`, 13 checks). A guard
   is a wolf slot whose home is inside a site and whose leash is that
   site's `SiteFootprint::scatter_m` instead of its species' `roam_cm` —
   both pure in the slot ordinal, so no wire field and no client change.
   Two per site, 6 of the 16 wolves.
   Owed off it: **the guard has no loot tier of its own** — it drops a
   wolf's meat and fat, so the reason to fight it is the crates behind it.
   The tier wants a third *species* (its own `drops`), and that is a client
   change, not a content row: every species match in the client is a `_ =>`
   fall-through, so a third kind would draw and sound as a pig until five
   arms are written (`render/mobs.rs`, `sound/voice.rs`, `ui/death.rs`).
   Forcing it through `loot.toml` instead does not work — `validate`
   requires `hits > 0` ("swings to open"), which a mob has no meaning for.
4. **Nobody has fought a guard in the running game.** Same standing as
   item 1 and now more of the pad's story: the wolf that hatches at the
   crates has never been seen, heard or fought with the client attached.
5. **`inventory::slots_in` is the same defect shape one function over**
   (judge, pass -07, fix 2): `CONT_BOX => BOX_SLOTS, _ => INV_SLOTS`, and
   the drip takes the panel's width from it. Right today only because a
   world container happens to be `INV_SLOTS` wide; a fifth ground kind of a
   different width draws the wrong slot count silently, and
   `a_world_crate_is_drawn_from_the_crate_store` reads `0..INV_SLOTS` so it
   would not catch it. Wants an explicit arm under the same `CONT_MAX`
   compile guard `container_wire.rs` just gained.

## 0pr · The wolf hunts — what predator v0 left *(systems lane)*

Predator v0 landed 2026-08-14: the wolf is a content row, and **nothing in
`mob.rs` branches on species** — a hunter is `brave_pct = 0` plus a notice
radius. `DECISIONS.md` §open "predator v0" has the numbers, the sources and
the phase-locked-bite bug the stride exposed. (Trimmed to the bound
2026-08-14; the landing story is in that row, not here.)

**Item 3 landed 2026-08-14 — pointing the other way.** The sim reads the
clock: the wolf hunts **worse** after dusk (30 m → 15 m), because no game in
the survey publishes a night sense ratio above 1×. §open "nocturnal senses".

**Item 1 landed 2026-08-14.** `sound/pig.rs` is `sound/voice.rs` and reads the
species off the roster slot, so a wolf howls (88 m) and growls (14 m) instead
of snorting. The register switch is not a knob — it is `CUES[Growl].radius_m`
read back out. §open "wolf voice v0" has the sources, the four places the
research changed the design, and the three follow-ons it names.

Owed, in rank order:

1. **Nobody has heard any of it.** Every claim is arithmetic — ZCR, sustain
   ratio, cadence bands. `bin/soundbank.rs` dumps the bank to WAV; ears are
   the gate that has not run, and the knobs to listen for are the two
   cadences (75 s, 2.5 s), the 0.5× night sense, and **16 predators** — all
   four are arithmetic nobody has playtested.
2. **A wolf pays no hide and no bone** — refused in the roster slice because
   it drags recipes and `ui::icons::STEMS` in with it.
3. **Night still costs the player nothing.** Nocturnal senses made the hour a
   *tactic*; it did not make the dark dangerous. The sourced follow-on is
   **not** more tuning of `night_spook_m` — it is a night-only roster variant
   (Minecraft and Valheim both gate *spawns* on darkness). The judge's gap 1
   wanted a warmth stat, which is the bigger version of the same hole.

## 0sp · The tick has been profiled — where it goes *(server lane)*

`crates/server/src/bin/profile.rs` (new, 2026-08-11) builds the stated worst
case — `MAX_PLAYERS` in one AOI cell, roster alive, store filled, everyone
acking and swinging — and splits sim from netcode by ablation. **Not a gate
and must not become one**: it reports elapsed time. `valgrind
--tool=callgrind` gives the per-function ranking, the half this box repeats.

**It settles half of §0q item 4.** A full tick at 100 clients is ~0.8 ms of
33.3; the AOI scan is O(clients × (players + mobs)) and ~0.24 ms of it, so
**the linear scan needs no spatial structure** — the soak still owes jitter
and real bytes. `state_hash` is 85 µs one tick in 32 and `encode_world`
24 µs one in 1,800: `reference/SAVES.md` §4's freeze is not ours.

Landed with it, −28 % instructions: `movement::step`'s duplicate terrain
fan; the AOI rank sort → two selections (the single largest item, ~23 %),
gated by `snapshot_budget.rs`'s `the_rank_band_agrees_with_a_full_sort…`;
the encoder's quadratic baseline scan; a whole-field `BitWriter::write`; and
the one that was a **spike** — `gather::swing` read `terrain::scatter` cold
instead of through `SlotCache`, so a hundred aligned swing cooldowns cost
1.9 ms in one `World::tick`, now 0.28.

**The client half landed 2026-08-13.** `resolve_swing` reads through
`ClientCore`'s own `SlotCache` — the predictor's, warm with the cells this
frame's movement step just filled — via `ClientCore::island`. Counted, not
timed: 61 frames of crosshair on one node cost **9 `scatter` calls, not
549** (`SlotCache::resolves`, a memo statistic, never hashed). Four gates in
`tests/ui.rs`, three proven red under their own mutation; the fourth is a
call-site scan refusing a direct `scatter(` on this path
(`tls_callsite.rs`'s shape). `render/props.rs` is deliberately excluded —
64 *distinct* cells once per chunk is not a memo's case.

Open: the encoder is now the largest phase (~0.43 ms of the 0.83), and
`World::scatter_clear` still resolves cells cold per spawn pick. **It is
not the same three-line fix** — it is `&self`, and unlike the crosshair its
3×3 window *moves every candidate* along the spawn ring, so the cells are
distinct and a memo only pays across repeated picks. Measure a respawn
storm before threading `&mut self` through the picker.

---

## 0wt · WebTransport outlived its only user *(server lane)*

**Operator, 2026-08-15:** *"have we not moved on to just like a real
transport? we dont do web now"* — correct, and `NETCODE.md` §2 had a browser
support matrix in it until that question was asked (fixed there).

We are not missing real QUIC: `wtransport` is quinn, we enable its `quinn`
feature, and `net.rs` already uses quinn's own `QuicTransportConfig` and
`IpBindConfig`. What is vestigial is the **HTTP/3 session layer** on top —
extended-CONNECT (`endpoint.accept()` → `IncomingSession` →
`request.accept()`), the `https://{addr}` URL shape, a session-id prefix on
every datagram against the 1 100-byte budget.

⚠ **One of §0wt's reasons died on 2026-08-15**: §0tx checked whether the
wrapper hides quinn's `Incoming`, and it does not — `IncomingSession` *is*
`quinn::Incoming` and re-exports `retry()`/`refuse()`. QUIC-level admission
is built and needed no flag-day. The reasons below stand.

The case to drop it is not speed. It is that **our one remote-panic trap
lives in that layer** (#317, two bytes on the CONNECT stream), which is why
we depend on a git rev of an unreleased third party instead of a published
crate — and §2.2's ⚠ says nothing records or gates that the pin even
contains the fix. Removing the layer retires the pin, the trap and the
browser-shaped cert rules (P-256, 14-day) in one move.

The seam is thin — client `connect`, server `accept`, a handful of config
types, `tls_posture.rs`, `botclient.rs`, `Shard::url`. **The cost is not the
code, it is the flag-day**: the handshake itself changes, so nothing
negotiates and an old client just fails. Two depots and a public shard are
live, and `elo-shardlist-v1` publishes the url shape.

So: **not its own pass.** Bundle it with the next `min_client` floor raise,
which is already a flag-day, or with the next touch of the wtransport pin
(§2.2 marks that seam owed anyway). Wants the operator's word on timing —
publishing and floor raises are operator acts.

## 0tx · The transport tells the truth now — LANDED 2026-08-15, three residuals *(server lane)*

`NETCODE.md` §2.2 is headed *config of record* and three of its rows
described code that did not exist. Found by grepping the table row by row
instead of reading it. All three are built, plus the telemetry that makes
them measurable — no wire change, no `PROTO_VER`, no golden.

**Landed:** socket buffers asked at 8 MiB **and read back** (the readback is
the feature — `setsockopt` clamps to `rmem_max` and returns success, so
`net_rcvbuf_asked` sits beside `net_rcvbuf_bytes` and they disagree out
loud); a QUIC-level admission gate (Retry for unvalidated addresses past 2×
`MAX_PLAYERS`, refuse past 4×, ordering asserted at **compile time**);
`shard.toml` `cc = "cubic" | "bbr"` with an unknown value refused at boot;
and `writer_task` sampling quinn's `ConnectionStats` as deltas.
Gates: `crates/server/tests/transport.rs` (boots a shard, dials it, proves
the numbers move) plus three unit tests. `DECISIONS.md` §open "transport
truth v0" has the five knobs.

**The §0wt question is answered and the answer is no:** `IncomingSession`
*is* `quinn::Incoming` and re-exports `retry()`/`refuse()`/
`remote_address_validated()`, so admission was never a reason to drop
WebTransport. §0wt keeps its other reasons.

**Residuals, in rank order:**
1. **Nobody has run the A/B.** `cc = "bbr"` is now selectable and untested
   against CUBIC on a real path. `net_congestion_events` is the reading.
   Wants a shard with real players, not loopback.
2. **The sysctl half of the socket buffer is still ops and still owed.** The
   code asks; `net.core.rmem_max` on the public shard's box decides. The
   readback pair now says which, so this is finally checkable rather than
   assumed — check it before tuning anything else.
3. **No client-side telemetry.** All of the above is server-side. The HUD
   still has no loss/RTT source, and `client/src/lib.rs` holds a
   `Connection` it never asks anything — the same gap, other end.

## 0n1 · Class-S interest — the radius lands, the grid does not *(server lane)*

`reference/NETWORK.md` §9.2.1. `pump_events` used to drip the **entire**
piece store to every client with no distance test anywhere. Both halves of
that are now closed: the removal restart went with the tail-down walk, and
the filter landed 2026-08-18 (`server/src/interest.rs`, `DECISIONS.md`
§open "class-S interest v0").

The walk is aimed from an anchor, streams `AOI_EXIT_CM`, and re-arms at
`AOI_EXIT_CM − AOI_ENTER_CM` — class D's own band, so §7's "one spatial
truth" is one set of numbers. `EV_PIECE_PLACED` takes the same predicate; a
removal does not, because nothing can yet tell a client to forget. Measured
on a 2,291-piece island with 454 in range: **2,291 → 454 records, 11,384 →
2,258 bytes, done at tick 72 → 19**; a full walk is bounded at 32 ticks
(was 256). Gate: `server/tests/piece_interest.rs`, red both ways.

**What remains, ranked.**
1. **The grid.** No chunk version, no subscribe/unsubscribe, so no client
   can be told to forget a region — which is why removals stay broadcast
   and why a re-arm re-walks the in-range set instead of the difference.
   That is §5/§7 proper and wants a wire change; this lane could not take
   one (`protocol` was another lane's this window).
2. **Deploys and backpacks are unfiltered**, and the deployable walk still
   restarts on a removal — §9.2.1's amplifier, one store over.
3. `test_stream_in` (§11) is still unbuilt: this gate counts records, not
   the client's per-frame apply/teardown budget, which is the other half.

---

## 0n2 · Monuments — the solver is two hand-written tiers *(world lane)*

Research landed 2026-08-10: `reference/MONUMENTS.md` (operator briefing —
**§0 says its provenance is the weakest here**, so read §9, not §1–§8, before
building). §9.1 is what we already got right and must not relitigate; §9.2 is
built this pass (`SiteFootprint` / `site_sweep` — a site publishes masks, not
a radius; clutter no longer grows across the pad).

Landed since: §9.3a (the drawn structure is derived from the sim's box table
— one list, so the mirror cannot drift again — plus `tests/greybox.rs` over
**every** archetype), §9.3b (the world file refuses an island that moved under
the same seed), and the debug/release probe diff that closes float contraction
on the one axis this box can reach. **All of it looked at** (§0p3 has the
recipe); two art rows fell out and are in `DECISIONS.md` §open — the shelter's
corner posts stand 1.2 m proud of its roof and read as stubs, and swept ground
reads as scattered shards at 2 m because of the pebble mesh. **The collision
skirt is closed** (operator, 2026-08-10): every occupant blocks what it draws
now, within a millimetre, and the gate holds it there.

**Deploy collision landed 2026-08-11** (deploy collision v0, `DECISIONS.md`
§open): six archetypes block at the client's own authored volumes, tops are
ground, and `tests/greybox.rs` §D holds the sim and drawn tables equal.
Residue, one line each: arrows still pass through every deployable
(`ranged.rs` never asks the solid nibbles — same class as its piece gap),
and whether a sleeper blocks stays unanswered (§0y item 1, untouched).

**The carve is BUILT AND ARMED** (2026-08-16, operator). Stage 8 makes its flat
pad now: 128 seeds, every site fully flat, worst floor 8.06 m → 0.000. Two
mechanisms carry it and both were forced by measurement — `blend_m` (the ramp
runs 12 m past the scatter mask, because confining it there built a 2.09
rise/run wall on rough seeds) and `max_cut` (the cut is clamped to what the ramp
can carry). `WAYSTATION_RADIUS_M` is derived now, 11.0 → 15.01. **`test_terrain_golden`
moved and is regenerated in the same commit — this is a wipe**; the probe
hashes windows over all three sites and they stand on the ring, so a golden
that had held still would have meant the carve missed them. `DECISIONS.md`
2026-08-16 has the numbers.

**The reference's placement datum is taken** (2026-08-17, operator).
`Haven::floor_y` / `site_floor_y`: a site's floor cuts to the level of lowest
error over the ground it flattens, not the raw height at its centre — their
terrain anchoring, `reference/MONUMENTS.md` §9.2b. Worst required cut 4.909 →
4.100 m over 384 sites; all still flatten. A separate field from `y`, so site
selection and every existing reader are untouched. ⚠ It bought floor headroom
and **not** the gentler ramp §9.2b predicted: the clamp binds out in the band
(deepest cut wanted 11.777 m against a 6.320 m cap) where the datum barely
moves, so the carved gradient is unchanged at 0.5951.

§9.3 is the gap and it is not urgent yet: `haven()` + `pick_minor` produce two
kinds of site, the separation floor is one hand-asserted constant
(`WAYSTATION_MIN_SEP_M`), and there is no reservation ledger. That is correct
at two tiers and is §1's starvation shape at five. **The trigger to fix it is
a third destination kind, not a spare pass** — the per-tier check chains stay
separate by a call the code already records.

Ranked after that, all from §9.4: class S still has no interest filter at all
(§0n1 — and a monument is the worst place to discover it), per-entity interest
ranges, then nav the day something defends a site. Vertical AOI layers are
premature (no underground) and moving monuments are refused on the record.

---

## 0p3 · You can photograph any authored site, and it is a config line *(client lane)*

Found 2026-08-10 while checking the greybox fix by eye. §0p2 item 3 asks for a
**viewer** for the screens nothing can photograph; four fifths of one already
exists and nobody had connected the pieces. The capture harness stands its
camera at the player's spawn, and `shard.toml`'s `dev_spawn = "x,z"` puts that
spawn anywhere. So:

1. `terrain::haven(seed)` / `haven_shelter` / `waystation_canopy` give the
   coordinates; stand 15–35 m off on the bearing you want in frame.
2. `Xvfb :9 -screen 0 1280x720x24 &`, then the shard, then
   `VK_DRIVER_FILES=/usr/share/vulkan/icd.d/lvp_icd.json DISPLAY=:9
   WGPU_BACKEND=vulkan target/release/gates --server 127.0.0.1:4433
   --capture <dir>` — six vantages, ~40 s.
3. The vantages face N/E/S/W, so place the camera on the opposite side of
   what you want to see. Two of four attempts missed for this reason.

**This asserts nothing and must not become a gate** (`CLAUDE.md`: the visual
gate is a person, and `vantages.mjs` is why). What it changes is the cost of
looking, which was "boot the game on a machine with a GPU" and is now a
command on this box. Still owed from §0p2 item 3: the panels, which need the
camera pointed at a screen rather than at a place.

## 0pf · The client's CPU frame — the ranked five are landed *(client lane)*

Landed 2026-08-11 (`water::animate` 1.01 → 0.38 ms; `terrain_mesh::heightfield`
28 → 5.4 ms) and **2026-08-19, which took the remaining four of the five**.
Measurements and the method: `findings/client-frame-20260819.md`. Headline, all
release on this box:

- `clutter_fill` **2.87 → 1.01 ms a tile** (one tile a frame), the 5×5 ring
  74.1 → 27.7 ms. Three bit-identical cuts: a caller-owned lattice memo
  (`terrain::Lattice`), the rich stratum refusing on its own roll before
  resolving the ground it would be tested against (42% of a tile's `height`
  bill), and one `splat_from` feeding both the acceptance rate and the kind.
- **`water::stream` 3.31 → 0.97 ms on a walking snap.** `SNAP_M` is 4 ×
  `STEP_M`, so a one-axis step slides every core coordinate onto the same
  `f32` — the sweep carries what it already answered and re-taps only the
  skirt and the columns that entered. Item 2's "no half-lattice left to share"
  was true of the skirt and false of the core.
- **The far mesh is off the frame.** `heightfield` runs on
  `AsyncComputeTaskPool`; the ~190 ms `Loading` frame with the session pump
  inside it is one `meshes.add` now. Near chunks too, so a ring advance costs
  no build. Its own arithmetic moved least of anything here (203 → 153 ms far,
  5.9 → 5.2 ms near) — it already shared its taps, and the win is the thread.
- Per-frame: `structures::stream` and `props::harvest` run on a `Feed::applied`
  bit rather than every frame (up to 117 µs and 2.34 ms of scanning at the
  caps); `decal::fade` and `ghost::track` no longer allocate per frame; a
  stationary body no longer re-propagates its 55-node skeleton.

Gates: `sim-core/tests/lattice.rs` (11), `client/tests/ground_async.rs` (4),
`client/tests/water_carry.rs` (8), `client/tests/frame_gates.rs` (4) — each
carries its own mutant table, and two of them are there because a first draft
was green under a mutant it was written to catch.

Still open, and item 5 is unchanged:

1. **`ground_slope`'s four taps are ~80% of what a tile now spends** and the
   stencil is not takeable — it moves every splat byte, so it is a design
   change with a golden behind it, not an optimisation.
2. **`water::animate` clones ~677 KiB into the render world every frame**
   (`Assets::get_mut` marks the mesh modified; a `MAIN_WORLD` mesh is deep-
   cloned on every modification). No flag fixes it; the fix is the vertex
   shader `render/water.rs` §57 already names. Its own doc says "no
   allocation" and that is true of the system body, not of the frame.
   **MEASURED 2026-08-20 rather than quoted** (`examples/frame_cost.rs`): the
   sea is **7,921 vertices / 676.6 KiB**, and stream+animate on a still frame
   is 0.69 ms. So the whole prize is ~0.7 ms of CPU plus a 677 KiB memcpy and
   upload — real, and **not obviously worth the slice it costs**: the port
   moves four waves, their analytic gradients, the shoal attenuation and two
   foam terms into WGSL, and nothing on this box can run WGSL, so it would
   land unvalidated on the most visual surface in the game. Recommend it
   AFTER someone can boot the client on a GPU, not before.
3. **The per-frame leftovers, all measured and all small.** `verbs::resolve`
   scans the piece mirror twice a frame (25.7 µs at the 8,192 cap, ~3.5 µs at
   512) and wants the 3×3 cell neighbourhood `ColIndex` already maintains;
   `bodies::stream` and `mobs::stream` re-find each interpolator slot by linear
   scan after `ids()` already knew it (~15 µs at a full shard); `audio::fell`
   fetches a `GlobalTransform` for every fellable to test `is_changed()` where
   a `Changed<>` filter would do; `hud::update` builds 10–16 strings a frame,
   most identical to the last frame's; the three ring streamers retain and
   probe a full map every frame even when the eye has not left its cell. Under
   50 µs together on a normal frame — listed so the next pass does not have to
   re-measure them, not because they are worth a slice.
4. **The sea's tangent `w` and mikktspace's disagree.** `water.rs` writes `-1`
   for a planar XZ UV set; mikktspace answers `+1` for the identical
   parameterisation on the ground (asserted, `tests/ground.rs`). One of them
   flips the ripple map's green channel. Which is right is a question about how
   that map was authored — boot the game and look, do not guess.

## 0bd · The barrel is the measured drum — LANDED 2026-08-18, one residual *(client+sim lane)*

The drawn barrel and the blocked barrel were one number in two files and both
were the deleted browser client's `CylinderGeometry(0.45, 0.45, 0.95)`. They are
the measured 55-gallon drum now — **0.585 m across by 0.88 tall** — so radius
**0.2925**, half-height **0.44**, `archetype_lift` 0.44 (the base IS the slot's
ground) and `OCCUPANT_TOP_M` **0.88**. Mesh and volume moved in one commit
because they cannot move apart: `greybox.rs`'s
`every_drawn_archetype_fits_the_volume_the_sim_blocks` fails a mesh narrower
than the blocked volume by more than `SLACK_R_M`. `test_replay`'s golden
regenerated with it (wall 5); the knob is `DECISIONS.md` §open, barrel
proportions v1.

**Residual — the tree's trunk radius is the same class and nothing measures
it.** `OCCUPANT_R_M[Tree] = 0.26` cites `CylinderGeometry(0.13, 0.26)`, browser
geometry, and the tree is the one row `greybox.rs` *excuses*: `tests/tree.rs`
measures the canopy against `PINE_MAX_R` and the base against y = 0, never the
trunk against 0.26. A generated conifer's trunk could be any width with both
gates green. Unlike the barrel there is no second source to take, so this is a
measurement to make — bound the bark mesh's radius over the trunk's own height
band in `tests/tree.rs` — not a number to paste. The box rows (`CrateSlot`,
`CacheSlot`, both authored structures) are browser-cited too and are fine:
greybox holds each to its drawn mesh in both directions.

## 0b · Balance sits on the reference's numbers now — what is still off *(content lane)*

Landed 2026-08-08 (operator: *"balance the game similar to rust so people
dont get too lost"*). `reference/BALANCE.md` is the research and §6 is the
standing instruction. Building blocks are 250/500/1000, a stone wall takes
four satchels, tool and melee damage are theirs, the pig is a 150-hp boar.
Two bands moved and the raid ratio re-priced itself. ⚠ **Derive the raid
ratio, never quote it** — `Content::load_dir(…)` then `.anchors()`, five
lines. This paragraph carried three readings and every one went stale
inside two days; the fourth was quoted here until 2026-08-18 and was wrong
too, which is the whole argument for the probe over the sentence.

**`reference/RIPLIST.md` is the queue for this item** — what is taken,
outstanding or blocked, and the six steps for executing a row. Read it
before touching a balance number; do not re-derive that list here.

⚠ **Two rules changed on 2026-08-10 and both are operator-spoken.**
(a) *"lighten our own math and lean on them for now"* — a band of ours
yields to a number of theirs by default (`BALANCE.md` §6.5); re-speak it
rather than treating it as evidence. (b) A number **absent** from
`RIPLIST.md` has not been decided either: asking that question found six
of twelve content files with zero coverage.

**Rows 1b, 1c and 1d all landed the same day** — building costs, the
craft column and deployable hp, `RIPLIST.md` §1c is the record. What is
left of that thread, in order:

1. **Row 1e is CLOSED** — all four uncovered content files have coverage,
   at page tier 2026-08-18: `cooking.toml`'s recycler taken (`RIPLIST.md`
   §1f). `armor.toml` and `loot.toml` are both **researched at page tier
   and blocked** — rows **1j** and **1i** — with their numbers written
   down so nobody re-researches them. 1j is one fixture string in
   `crates/content/tests/content.rs` (the take passes every band; it is
   `band_breaks_refused`'s anchor that breaks) and belongs inside
   equipment v0. 1i is a schema field: their container ladder is a
   *guaranteed* scrap payout and `LootEntry` cannot express a certain
   drop — the half-take measures **9× worse than leaving it alone**.
   Next unblocked: §2 row **1g**, the research ladder's ordering.
2. ✅ **Egress is OPEN here** and the old "a browser is the only route"
   note is retired: facepunch wiki 200 and its item pages carry full
   Protection / Recycle / Research tables, `rusthelp.com` 200 — **400
   without a browser User-Agent**, which reads as a dead host — only
   `wiki.rustclash.com` still 403s. Probe; `SOURCES.md` §0 is right that
   this is a property of the container.
3. **Both of this pass's findings were a reason of ours nobody had scored
   against the source — §6.2's fourth costume, and the hardest to see,
   because it reads as admissible.** `BALANCE.md` §4.1 filed armour under
   *real* reasons ("protection is per damage type") when their projectile
   and melee cells are **equal** on all three pieces we own — retracted,
   and the retraction lands even though the numbers are blocked.
   And two files priced junk against "a ~10-scrap barrel" that was never
   measured; the page says 2.42. Detail in `RIPLIST.md` §1f/§1h.

Closed 2026-08-11 by the operator, all three: the rock **is** craftable
(15 → 10 stone, and the tier-4 source beat my prior — §1c says so), JUNK
**is** scrap so the research table takes its 20 (my refusal answered
itself), and the cupboard is **stronger** here on purpose — hp 500 →
1,000, the metal rung, so taking a base's privilege costs one more wall.
Nothing about the cupboard is outstanding now; it is a chosen difference.

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
`RIPLIST.md` §2: no per-material damage resistance (one `structure`
column, so the ladder above stone is compressed); and gather yields, smelt
and craft times are still ours — node totals are `READY` now, per-hit
yields are not, and our schema does not need them. Upkeep, decay and the
armour ladder differ on purpose (`BALANCE.md` §4.1), though the upkeep
*rate* turned out to match theirs. (Struck 2026-08-14: "the boar does not
fight back" and "one animal" were falsified by `mob attack v0` and the
wolf and had been left standing — the last judge's ranked fix 1.)

---

## 0rl · The release path ran, on all three platforms *(platform lane)*

`.github/workflows/release.yml` builds the client and shard for Linux,
Windows and macOS on a `v*` tag, re-runs the gates on the tagged commit,
refuses a tag that disagrees with `[workspace.package] version`, and drafts
the release.

1. ~~macOS has never been compiled, Windows only typechecked~~ — **retired
   2026-08-11, by the tag.** `v0.1.0`'s release run is green in all six
   jobs, including `build (macos-latest, aarch64-apple-darwin)` and `build
   (windows-latest, x86_64-pc-windows-msvc)`. So msvc linking and the Apple
   toolchain are no longer written-and-unproven: they compiled, linked,
   staged and archived on real runners. **The three artifacts have still
   never been RUN** — nothing here has a Mac or a Windows box to start one
   on — so the honest state moved from "does it build" to "does it launch",
   which is a tester's question and not CI's.
2. ~~No `LICENSE` file~~ — **done 2026-08-11** (MIT, © MoreRight DAO;
   `DECISIONS.md`). `LICENSE` + `NOTICE` ship in both the release archive and
   the elo depot, gated by `ci/depot.py --self-test`.
3. **The draft is drafted and nobody has published it.** That is the one
   operator act left on the release itself: open it, read what is attached,
   publish. Until then the tag exists and the download does not.
4. **`min_client` has never been raised on a live shard**, and the public
   shard now running prints `admits clients of any release`. The order is
   publish the release FIRST and raise the floor after; `refused_build`
   climbing days later is how you find out you did it backwards.

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
   both. The nightly artifact is the tree those acts consume, and it
   **exists**: `gates-depot-<run>` off nightly run 31475002978, 34 MB, live.
   ⚠ Two things checked 2026-08-11 and worth knowing before you reach for
   it. The `depot` job in that run passed while the run reads **failure** —
   the `nightly` job failed at "gates first", at 08:50Z, which is before the
   toolchain pin merged that evening, so it is the same red every `gates`
   run had that day and should be green on the next fire. And **neither
   publish act can happen from this box**: `elopros.com` is a
   different host (Cloudflare-fronted, not 5.161.193.186) and there is no
   `elo` binary on PATH, so `elo digest` — the one implementation of the
   number that gets notarized — is not runnable here by construction.
2. **The shard list is written, generated, and not yet served.**
   `shards.toml` exists now and `./ci/shardlist.py` writes the one-row
   document; `manifest.servers.url` on elo's side is still `null`, so the
   launcher's Servers window and our own menu stay dark for the same missing
   file. elo's serving half is confirmed live rather than assumed —
   `GET /api/launcher/servers/gates` answers **404**, its documented
   "publishes none", not the 503 it reserves for "could not look".
   Everything downstream of that one publish exists: live counts via
   `status_url` (answering now), and join links
   (`elo://join/gates/host:port`, `deeplink.rs`). Registering the scheme
   with the desktop is the launcher's installer, and is not done.
3. **`prove` has no call site** — and this is now the *only* thing left in
   the identity seam, because the ticket door landed on the handshake we
   already have (2026-08-11, `entitle.rs`). The address is proved today:
   the shard picks the nonce AND the `issued_at`, the client composes the
   message through the one shared `protocol::siwe_message`, and the server
   rebuilds identical bytes and `ecrecover`s. That is sound, and it is why
   entitlement needed no wire change.
   What `prove` buys is the *consent prompt*: `sign_siwe` makes the launcher
   sign a string this process composed, so the player clicks through a dialog
   on every join; `Overlay::prove` has the launcher compose it, which fires no
   prompt by construction. **The cost is real and is why this is still open:**
   the launcher writes its own `Issued At`, so the server can no longer
   rebuild the bytes and must PARSE an EIP-4361 message instead — and the wire
   has to carry that message, which IS a layout change (wall 6: version bump +
   goldens in the same commit). Worth doing for the prompt alone; it is a
   slice, not a line.
4. ~~**The depot is Linux only.**~~ — **retired 2026-08-14** (the packager
   takes `--platform` and bakes it into `root`), and both platforms are
   published from one commit as of v0.4.0. This line survived two days past
   the change it described, which is the ⚠ at the top of `CLAUDE.md`: `ls`
   the file, do not trust a doc's memory of it.
5. **The public shard is up and no one has ever joined it** (2026-08-11).
   Boot, persistence, the SIGTERM flush and the status endpoint are all
   measured; the join is not, and **the tools here cannot measure it**:
   `bots` takes a `SocketAddr`, so it cannot dial `game.elopros.com` by
   name at all — which is the half that matters, because the certificate is
   issued for the name and the client validates against the platform root
   store on a non-loopback address (`tls_posture.rs`) — and it carries no
   wallet, so `require_auth = true` refuses it correctly. The first real
   join is a person with the published build, which is why it sits behind
   §0rl item 3.

---

## 0ad · The ticket door is armed but nobody has sold a copy *(platform lane)*

Landed 2026-08-11 (`crates/server/src/entitle.rs`). A shard with
`entitle_origin` set asks elo `GET /api/ticket/gates/of/<wallet>` at join
and `POST …/check` for the whole roster every `entitle_sweep_secs`, refusing
with `REFUSE_TICKET` and kicking on a **definite on-chain zero only** — a
failed read admits and bumps `entitle_unknown`, because an RPC outage that
booted every paying player is worse than the freeloader it catches. Unset is
the default and checks nothing, which is what every test and every community
shard runs (`DECISIONS.md` 2026-08-04: one build, two populations).

What is left, in order:

1. **Nothing has been driven against a real ticket contract**, because
   `ScryGameTicket:GATES` is not deployed — elo's `deployments.json` has no
   address, so `/of/<wallet>` answers `ticketed: false, entitled: true` for
   everyone and the door is a pass-through by design. Every branch is unit-
   tested against the response shapes elo actually serves (`tickets.py`),
   and none has met the live route. **First real check is the day the
   contract is deployed**, and the honest way to run it is one wallet that
   owns a copy and one that does not.
2. **The sweep interval is unspoken.** 120 s is a documented default, not an
   operator sentence, and it is the whole security property — how long a sold
   copy keeps playing. `DECISIONS.md` §open carries the row.
3. **No `prove`**, so a join still costs the player a consent dialog — §0ab
   item 3 has what that slice actually needs.

---

## 0tt · The bench ladder and the tech tree — LANDED 2026-08-15, three residuals *(systems lane)*

Spoken 2026-08-14 (*"we should copy the tech tree ui thing too"* —
`DECISIONS.md` has the row): their two-system model — research what you
loot at the table, tech-tree the gaps at the bench — plus the workbench
tech-tree UI. What exists: `research.rs` + `research.toml` (powder era
only, deliberately), `item.workbench` (tier 1), JUNK-is-scrap, and since
2026-08-15 the dependency EDGE (`requires`, §0tree) — so what this item
still owns is the BENCH half. What is
missing is the whole ladder: no workbench 2/3, nothing above
`workbench1` in `recipes.toml`'s station column, no tree, no UI — and
`RIPLIST.md` §2 row 3's craft rebate is blocked on exactly this ladder.
The era was spoken (*"YES do it pre 2025 please"* — `DECISIONS.md`
2026-08-15, and §open "bench ladder v0" carries the derived numbers) and
the whole slice landed in one pass: workbench 2/3, the tiered station
gate (`bench_near`'s ≥), the tech tree as a `requires` column over
`Player::known` (`research::unlock` — no sample, which un-deadends the
satchel and roadsign, blueprint-gated items in NO loot table), the bench's
`E` opening a tree panel, wire v38. Landing it found a live wire defect:
the three research events had **no decoder arms since v32** — every
research toast and `Known` restate was decode-refused client-side, caught
by the first goldens ever to pin that lane (91 fixtures now).

Remains, each small and none blocking:
1. **The craft rebate** (§2 row 3) is unblocked now — 50% faster one
   bench up, 75% two up — a `craft.rs` lookup once someone takes it.
2. **The panel draws indents, not edges**: a line renderer between
   parent and child is cosmetic and waits for a real look at the screen.
3. **The operator has not seen it** — the tree panel, the two greybox
   benches, the tier badges. The visual gate is a person (`CLAUDE.md`);
   boot the game, stand at a bench, press `E`.

## 0ac · The catalogue — what twig and the cost grammar left *(systems lane)*

Landed 2026-08-10 (operator: *"we need to work on building more"*).
`reference/BUILDING.md` §7b is the research, `DECISIONS.md` §open "twig
v0" the slice: placement is twig-only and the hammer commits it, twig is
never upkept, and **the whole cost column is theirs** (`RIPLIST.md` row
1b). §9 items 11, 12 and **13** are done; 14 and 15 are not, in cost
order:

1. ~~The window and the wall frame~~ **Landed 2026-08-15** (§open
   "catalogue v1", wire v38): 32 cells at §7b.3's ratios, the window
   blocks a body and passes an arrow through its 1.2 × 1.2 aperture
   (`collide::shot_blocked` — arrows fly at their own radius now, the
   fix `ranged.rs` owed), the frame blocks only its drawn rim, and the
   doorway's lintel stops arrows. What remains of §9.13 is the
   **inserts** — bars, glass, shutters, the garage door — each a
   deployable pass of its own (§7b.4's second purchase), none started.
2. ~~Hard and soft sides~~ **Landed 2026-08-15** (§open "hard/soft v0",
   wire v39, save format 6): soft faces the placer, the hard face of any
   edge piece takes 1 structure a swing whatever the tool, the HUD's
   prompt labels the side you stand on off the same `build::soft_side`
   the swing is priced with. Still owed from §9.15: a **visual** identity
   for the soft face (a texture or tint — the label is the only tell
   today), floor sides (needs a vertical attack direction), and the
   pairing with `RIPLIST.md` §2's per-material resistance.
3. ~~Triangles~~ **Landed 2026-08-15** (§open "triangles v0", wire v40):
   the grid change, costed as one — four half-cell locs, two diagonal
   wall slots, three shapes at §7b.3's exact ratios, the half-cell right
   triangle deliberately instead of their unaddressable equilateral
   (the §open row carries the full case). What remains of §9.14: a
   **capture pass** to look at a diagonal base in the booted game (the
   person is the visual gate), the wall-on-diagonal price question
   (§open), and hard/soft's visual identity extended to tri halves.

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
`tests/water.rs` (28) and eight assertions in `tests/sound.rs` — and the
water suite only started running in CI on 2026-08-11 (§0pf). Remaining:

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
2. ~~Nothing fights back~~ — **done 2026-08-11** (mob attack v0,
   `DECISIONS.md` §open: the widening landed as wire v36, the pig charges
   whole and flees hurt, `DEATH_BY_MOB` names the corpse). Residue: the
   combat-feel half is minimal — the victim sees hp drop and hears nothing
   pig-specific; an aggro snort cue and a damage-direction tick are audio/
   HUD follow-ups, and the charge costs the pig nothing to hold.
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
5. **Roughness maps are still unread** — all nine of them. Blocked on an ORM
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
2. **The centre readout names the verb, not the target or the upgrade's
   cost.** The wedges are glyphs now (2026-08-09, `ui::hammer::verb_icon`,
   gated with the shape wheel's in `tests/ui.rs` §G); what is still text-only
   is the middle, and filling it wants `verbs::Near` at draw time, which
   `panels::rebuild` does not hold.
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
6. **Surveyed and refused: the freegameui.net asset MCP** (~2,100 CC0 UI
   SVGs, 2026-08-09). CC0 is welcome (`ART.md` §7) and the licence was never
   the objection: its gateway 403s from this box while `raw.githubusercontent
   .com` answers 200, so it cannot serve the loop; a tool that writes files
   into `assets/` bypasses `bake_icons.py` and §G, which are what make an
   icon re-bakeable; and pre-coloured button/gauge kits fight the
   tint-at-draw design the whole icon path is built on. A second source in
   the baker is the cheap version and is what item 2's glyphs used.

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
   soundbank -- <dir>` writes every cue to WAV. Looking already paid twice
   (the flat wind bed, then its fix overshooting); neither was reachable
   from a statistic that only asked "does it have energy". **The score
   raises the stakes on this item rather than answering it**: nine of those
   WAVs are music, and music is the thing a listener judges fastest. The
   sourcing queue for replacing cues — inventory, delivery spec, the
   ElevenLabs sheet (`DECISIONS.md` 2026-08-11), CC0/CC-BY candidates — is
   `assets/sound/WANTED.md`.
2. **The score is built and unheard** (2026-08-11). `reference/AUDIO.md`
   §8's whole design is `sound/music.rs`: gap timer, a theme of sectioned
   pieces, tiers picked at section boundaries off bumps we already had.
   What remains is the half that was always the blocker — `synth::score`
   generates nine placeholder pieces, so **the system is real and the music
   is programmer art**. Swapping in recorded pieces is a change to one
   function (`synth::render`'s music arm); the licence posture in `synth`'s
   header is why they are generated and not sourced. Two inputs the
   reference bumps on and we cannot: a weapon *equipped* (we bump on the
   swing instead) and a projectile near-miss (`reference/PROJECTILES.md`).
3. **The `--capture` run is still by hand**, and it is the only thing that
   proves *most* of the audio systems execute at all. It needs Xvfb, lavapipe
   and a shard, which is why it is not in `ci/gates.sh` yet. The score is the
   exception and shows the cheaper shape: `tests/music.rs` builds a bare
   `App` (`MinimalPlugins`, no window, no device, its own clock) and asserts
   the two music systems run and spawn what the director names. Every audio
   system with no world in its arguments could be gated that way.
4. **Two cues still have no producer**: `ImpactWood`/`ImpactMetal` need to
   know WHAT was hit, which the gather toast does not say, and `UiClick`
   needs a hook in the per-screen click handlers.
5. **No occlusion, and it needs a prerequisite rather than a pass.** A wall
   between you and a sound needs a geometry query, and the correct one is
   the sim's (`collide.rs`), not a raycast against render meshes.
6. **The ambience layer is one bird, and now it has a clock.** Birds are
   gated to daylight off the server's tick (day/night v0), so the
   prerequisite this item named is paid: **crickets are now a content-free
   companion pass** — a night-gated `Cue`, the bird layer's shape with the
   predicate inverted. The reference's localized-emitter *system* is still
   a later slice (§9.3: it arrives with a cull budget).

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
2. **Closed 2026-08-10.** The greybox mirror is one list now — the drawn
   structure is derived from the sim's box table (`props::authored`), so the
   drift cannot recur — and `crates/client/tests/greybox.rs` gates the rest,
   including the occupant table for everything that is not a tree. The sim's
   list won the authority call, and the props' invisible collision skirt is
   closed too (a boulder blocked 0.39 m wider than it drew; the rows carry
   measured bounds now and the gate is an equality check). `TERRAIN.md` §7.1
   has it. **Still uncovered: the clutter ring.**
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
STORE / WORKSHOP are the elo launcher's and hand off to it
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

1. **The document exists now; the two acts that serve it are on elo's
   box.** `shards.toml` is written and `./ci/shardlist.py` produces
   `target/servers.json` — one row, `game.elopros.com:61234`, carrying a
   `status_url`. What is left is exactly what it always was and no more:
   copy the document into `$SCRY_DEPOTS_DIR/gates/`, **then** set
   `servers.url`. In that order — `servers.url` pointing at a file that is
   not there is an error dialog on a game that is running fine, which is
   worse than the honest "no shards published" both readers draw now.
   ⚠ **elo's half is confirmed live, not assumed**: `GET
   /api/launcher/servers/gates` answers **404** as of 2026-08-11, which is
   its documented "publishes none" and is a different answer from the 503 it
   reserves for "could not look". The route is built and waiting for bytes.
   (The 2026-08-10 finding this replaces was that `/depot/` was not a
   `location` on that origin at all, so the url printed here could only 404
   for the wrong reason.)
2. ~~Player counts: three steps on a box~~ — **two of the three are done
   2026-08-11, and the count is live.** `status_addr = "127.0.0.1:8431"` is
   in `shard-public.toml` and the url is in `shards.toml`. The third step
   was "open that TCP port (the cloud firewall too)" and it was **not taken,
   on purpose**: the endpoint binds LOOPBACK and nginx fronts it on the 443
   this box already serves, so `https://game.elopros.com/gates/status.json`
   needs no console act, carries the same certificate as everything else we
   publish, and puts a buffer in front of a status thread that answers
   serially by design. It answers `{"players":0,"max_players":100,"tick":T}`
   right now. Both readers still draw `?` until item 1's copy happens —
   there is no list to draw a row in.
3. **Ungated, by hand only:** the end-to-end kill-the-shard-mid-play run
   behind `Screen::Disconnected`.

## 0t · the forest — what it owes, re-ranked off `reference/PLANTS.md`

Landed: `render/tree.rs` calls `bevy_procedural_tree` as ONE pure function.
**Felling v0** (2026-08-10): a chopped tree topples on a bearing derived from
the cell key, keeps its own mesh, and stays down — gate `tests/fell.rs`, knob
`DECISIONS.md` felling v0. Gates: `tests/tree.rs`, `tests/fell.rs`.

**The order below is `PLANTS.md` §6.2's and it inverts what this item used to
say.** LOD was rank 1; it is now rank 3, because clumping puts MORE stems in
the near ring and an LOD tuned against today's lattice is tuned against a
distribution we are about to replace. Measure between the two.

1. **Species v0 landed; the broadleaf has never been LOOKED at.** `SPECIES` is
   a two-row table (conifer 6.6 m / 2.9 m-wide broadleaf), pool 6, and
   `SPAWN_CLEAR_M` rose 4.0 → 4.5 with the arithmetic finally gated in Rust
   (`a_fresh_spawn_stands_clear_of_the_widest_tree` — `ci/pine_shape.mjs` was
   a dead citation). **Every check on it is arithmetic and arithmetic cannot
   say whether it reads as a tree.** Boot it and look; the parameters most
   likely to be wrong are `children`/`angle[1]` (crown spread) and leaf
   `count`/`size`, and `reference/PLANTS.md` §3.1 has ez-tree's 15 presets to
   pull real ash/aspen/oak numbers from instead of our derived-from-defaults
   block. More species is now a row in `SPECIES`, not a refactor.
2. ~~The scatter lattice~~ — **this item was wrong and is retired.**
   `terrain::clump` has always existed: an fBm field `scatter` multiplies the
   whole weight row by, squared for a ragged edge, gated by
   `sim-core/tests/scatter.rs` against a closed-form independent-draw null.
   Groves and clearings are built. What is actually open is the density
   **ceiling** — one occupant per 8 m cell — and `reference/PLANTS.md` §3.2
   prices the three ways to raise it. All are sim-core, none is cheap, and
   the cheapest (`CELL_SIZE` 8 → 4) quadruples the live `SlotLives` rows
   against `TERRAIN.md` §6's budget. Do not start it as a rendering change.
3. **The billboard LOD.** 328 trees × 5.9 k tris is 1.9 M against DESIGN §9's
   1.5 M. Octahedral impostors beat SeedThree's crossed cards (a card edge-on
   disappears); `PLANTS.md` §3.3 has both. Whatever LOD1 becomes, it sways.
4. **`aWind`** — `StandardMaterial` cannot read a custom attribute, so wind
   needs the custom material `RENDER.md` already lists. Gets LOD1 for free.
5. **The sub-canopy and shrub layers are empty** (`PLANTS.md` §2). ez-tree's
   three `bush_*` presets and a small tree at 40 % are new `Occupant`
   variants plus scatter rows once item 1 lands.
6. **The needle card is generated** (`tree::needle_image`); `WANTED.md` §9.5
   is the swap, and it is the highest-value texture on that page.
7. **Owed upstream as a bug report:** `BranchForce` pointing down hits the
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

## 0ad2 · The admin lane is built — what it still cannot do *(server lane)*

Landed 2026-08-11 (admin v0, `DECISIONS.md` §open). Six verbs on the chat
lane with **no wire change**, the anomaly log with its counter sweep, and
`/bug`. Gated by `tests/admin_wire.rs` (7) and `protocol::admin` (6).
Remaining, in order:

1. **A ban dies with the process.** `Bans` is memory only; persisting one
   wants its own file with its own format version, because sharing the
   player store's header would wipe it on the next seed change.
2. **Nothing has typed a command against a live shard.** Every branch is
   gated headless; the socket half (`conn.close` with `REFUSE_ADMIN`, and
   the client's dialog for it) has never been driven end to end.
3. **The log has no reader.** It is JSONL on purpose so `jq` is the
   reader, but nothing summarises a session — and the alpha gate's "zero
   silent failures" wants a *verdict*, which is a script somebody runs
   after a playtest, not a counter.
4. **No `/who`, no `/tp <a> <b>`, no set-time.** The last is blocked by
   choice: day/night derives from the tick, so moving the clock means
   moving the tick — it wants the wire field §0y4 deliberately did not
   spend.

## 0q · The gaps nobody has claimed

Lifted out of "done this pass" items before pruning (2026-08-05, again
2026-08-09) — each was written down **only** inside a done item. All of it
is `crates/`/wire work no single-surface lane may take.

1. ~~The UDP socket buffer is a `NETCODE.md` row and nothing else~~ —
   **landed 2026-08-15 with the transport-truth pass** (`net::bind_udp`,
   `UDP_BUF_BYTES` 8 MiB asked AND read back into `ShardStats`, gated by
   `the_socket_buffer_records_what_it_got_not_what_it_asked`; the §2.2 row
   is rewritten). This item stood a full day after the code landed and a
   parallel lane rebuilt the feature from a stale base off it — struck
   2026-08-16, the lane dropped. **The ops half is still owed**: this box
   grants 4 MiB of the 8 asked (`rmem_max`), and raising the sysctl on the
   public shard is an operator act. A stale item is not a small cost; it is
   a whole lane's work spent twice.
2. **Shore barrels as a second destination class.** The road pays unevenly
   now (the bay slots landed) and the haven pad is the one place worth
   walking to. A second class on the shore would give the ring two ends
   rather than one. Nothing else in this file mentions it.
3. **The wipe.** Named by both judges, described nowhere. A shard lifecycle
   act with an economy half (`ALPHA.md` A1→A3) and an operator half
   (`CLAUDE.md`: wipes of a live shard are operator-only), so the loop's
   share is the mechanism, never the trigger. Needs scoping before it can be
   an item.
3. ~~You cannot stand ON anything~~ — **done 2026-08-11** (deploy collision
   v0: `slot_ground` beside `slot_blocks`, occupant and deploy tops are
   ground under the lid rule; the plinth, crate, boulder and box tops all
   stand; gated in `tests/solid_deploy.rs`).
4. **The 100-bot soak RAN 2026-08-12** — baseline in `DECISIONS.md` §open.
   Headline: **`dropped-ticks 0`** over ~61,500 ticks with 100 clients, and
   **0 shed** of 17.5 M AOI entities offered, so the tick budget and the
   interest band both held at a population they had never met. The anomaly
   log's whole path was proven in the same run (8 bots against a full shard
   made `refused_full` move, and the file gained exactly that line).
   ~~real **bytes**~~ — **counted 2026-08-18.** Four lanes apart in
   `ShardStats` (datagram/stream × in/out, each a byte total *and* a message
   count, because bytes alone cannot tell "more packets" from "fatter
   packets"), aggregate on the shard and per-client on `bin/bots`, served by
   `/status.json`. So the next soak divides by `secs` and reports a measured
   kB/s/client instead of a ceiling. Not counted, and stated in `stats.rs`:
   the handshake (a per-join constant, not a rate) and QUIC's own framing
   (`net_sent_packets` is the other half of that ratio).
   **Three things it still does not have**, each its own small item: jitter
   as a **distribution** rather than a threshold crossing, an **hour** (this
   was 25 minutes, so slow leaks are not excluded), and **contention** —
   bots walk, they do not raid, so wall 4's caps are still gated one site at
   a time.
4. **You cannot stand ON anything.** `movement::step` asks `slot_blocks` and
   nothing asks a ground query for occupants — the shelter's plinth reads as
   a kerb you sink into, crate and boulder tops the same (`terrain.rs`'s
   plinth doc still says "nothing here makes a body stand on the plinth").
   Belongs beside `collide::piece_ground`, a `slot_ground` next to
   `slot_blocks`; the fourteen-box table is already there for it. Systems
   lane.
5. **The 100-bot soak has never been run.** `NETCODE.md` §9's budgets have
   never met 100 real connections: `cargo run -p server --bin bots -- 100`
   against a dev shard, held an hour — tick jitter, WAL append rate,
   per-client bandwidth recorded as counts and bytes, never wall-clock
   asserts (`CLAUDE.md`'s clock rule). The numbers land in a `DECISIONS.md`
   §open row as the measured baseline. **The AOI half is settled without
   it** (§0sp, 2026-08-11): 100 clients in one cell cost ~0.8 ms of a
   33.3 ms tick, so the linear scan needs no spatial structure. What a soak
   still owes is what a profiler cannot see — sockets, jitter, real bytes.

---

## 0r · The raid loop has offence now — what it still cannot do *(systems lane)*

Landed: `sim-core/charge.rs` — plant the held throwable at an address, fuse
from content, damage through the same `damage_piece`/`damage_deploy` a swing
uses (`ACT_THROW`/`EV_CHARGE_PLACED`; knobs `DECISIONS.md` §open "satchel
fuse v0"). X plants it natively and the HUD counts it down. Remaining:

1. ~~No blast radius~~ / ~~nothing is hurt by standing in one~~ — **both
   done 2026-08-11** (satchel blast v0, `DECISIONS.md` §open: linear
   falloff over a bounded one-cell ring, bodies take `damage` with the
   planter included, `DEATH_BY_CHARGE` on the v36 widening,
   `WORLD_SAVE_FORMAT` 4 carries the blast and fixed the mid-fuse-save
   refusal found on the way). Residue: no detonation sound or visual —
   the client learns of a blast only through `EV_STRUCT_HIT`/`EV_HEALTH`,
   so a near-miss is silent (audio lane); dud and defuse stay unbuilt.

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
  done.** The build ships as an elo depot (`ci/depot.py`, gated by
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

- **The recycler exists and the haven does not have one.** Landed
  (recycler v0, `DECISIONS.md` §open): `ARCH_RECYCLER` converts salvage on
  `oven::sweep` with the burn skipped, `CookRow::count` plus multi-row
  firing pays several outputs off one clock, wire v31. What is still open
  is the half this item was really about: **every deployable comes from a
  player placing one**, so the recycler is craftable and a destination
  still offers no verb you cannot perform at your own base. An authored
  worldgen deployable is the missing mechanism — a `DeployRec` standing at
  the pad that no player placed, which has to answer to persistence (a
  restart must not duplicate it) and to `pick_up` (nobody pockets the
  haven's machine). Systems lane. Bank and vendor stay blocked on an
  operator act.
- ~~**The waystations want a silhouette**~~ **LANDED** — `WAYSTATION_CANOPY_BOXES`,
  9 rows, deliberately not a shrunk `HAVEN_SHELTER` (4 posts, a knee-high
  parapet, 4.1 m). `DECISIONS.md` §open "waystation canopy v0" has the
  derivation; this bullet described the tree before it.
- **The pad carve's SEAM is built and the cut is dark** (2026-08-16, §0n2).
  The count in this bullet was wrong too — 65 `height` reads, 31 inside
  `terrain.rs`, ~18 consumers — and the number was never the hard part: the
  split is by role, and `tests/height_roles.rs` now holds it. What remains is
  two spoken numbers, both in `DECISIONS.md` §open "site carve v0".
- **Nothing threatens the walk between them — the SITES are contested now,
  the road is not.** Guards v0 landed (wolf slots leashed to a site's
  `SiteFootprint`, `tests/guard.rs`), so this bullet's original claim is
  stale; what is still true is that the ground *between* destinations is
  empty. Note the promotion it causes: `MONUMENTS.md` §9.4 item 4 said nav
  enters "the moment an NPC defends a monument", and one does — guards route
  through `movement::step`, so they slide along a shelter wall rather than
  path around it.

## 4b · The domain gate reads the crate now — one residual

Landed 2026-08-05: `SOURCES` reads all `sim-core` modules both ways and
every enumeration width is classified. Remains:
`death_causes_are_a_closed_ledger` (`event_roles.rs`) still scrapes
`world.rs` alone — narrow, since the protocol gate catches a stray value
crate-wide, but its *contiguity* claim is file-local.

---

## 5 · Gameplay still missing, in rough order of what a player notices

- **The arrow does no structure damage, and it is as fat as a body.** An
  arrow that reaches a wall stops dead rather than chipping it, and
  `collide::blocked` bakes `CAPSULE_RADIUS_M` into its query, so an arrow
  threads a doorway but never an arrow slit. The honest fix for the second
  is a radius parameter on `collide` — a `sim-core` change with a
  replay-gate consequence, so it wants its own commit.
  **Operator, 2026-08-10: ranged tracks the reference game as closely as we
  can, and arrows come back** (`DECISIONS.md`; `reference/PROJECTILES.md` §9
  is the sized list). Landed off it: ballistics on the round (§9.3), and
  `EV_SHOT` + the tracer, so the arrow is visible at last (§9.2, wire v33).
  Next is **arrow recovery** (§9.7) — the spent-arrow store, the ~15 % break
  and the 10 s lodge, and the first verb in the protocol addressed to a
  world position rather than a build cell, which is why it is a protocol
  pass and not an afternoon. It gates §9.6: their bow damage is priced
  against arrows that come back, so no bow number may track theirs until it
  lands. Then `headshot_mult`, armed-and-unread since the content crate
  (§9.4) — §7 says take the most significant body part, never the first
  intersection.
- ~~**The revolver still cannot fire.**~~ **Landed 2026-08-19** — hitscan
  did not want the rewound raycast after all (§0pvp item 3).
- ~~**Dropped loot should land somewhere you can find, not inside the
  floor**~~ — **landed 2026-08-14.** Six producers call `inv_add_spilling`
  (`gather`, `craft`, `build`, `deploy`, `lock`) and `World::drain_spill`
  stands a bag up at your feet; the client says so. §0sp2 has what remains.
- **Mushrooms and corn drop now** (2026-08-09, content rows only): the
  tree's secondary pays 1 mushroom a swing — the forest floor through the
  tree that shades it — and the coast-road barrel rolls a 2–4 corn ration.
  `content.rs::every_consumable_the_content_ships_is_reachable` gates the
  general form (every consumable producible by a live verb chain). Still
  owed, and both are code: a standalone forest-floor pickup archetype and a
  farming lane. The open verb landed 2026-08-14 and the reachable set widened
  2026-08-17: `validate.rs`'s clock and `content.rs`'s walk both count every
  verb-openable container (`bake::container_index`); the stale reason is fixed.
- ~~Day/night does not exist~~ — **landed 2026-08-11** (day/night v0,
  `DECISIONS.md` §open): 45-minute cycle, 70 % day, derived from the tick
  with **no wire field**, driven through the rig's coupled-set owner.
  What it does NOT do: no gameplay reads the clock (no nocturnal mobs, no
  crops, no torch — the survival clock `DESIGN.md` §2 pairs it with is
  still hunger and thirst alone), no moon or stars in the night sky, and
  no set-time admin verb — moving the clock means moving the tick, so
  that one wants the wire field this slice deliberately did not spend.
- **The coin loop is closed and the tech TREE is not.** JUNK is paid by
  the recycler and burned at the research table (research v0, and the
  operator's 2026-08-10 call that JUNK is scrap — what stages is the claim
  rail). What research does NOT have is depth: a row unlocks one recipe
  and depends on nothing, so there is no ladder, no tier, no "unlocks the
  next". The reference has a research table *and* a tech tree and they are
  separate systems; ours is the first. A tree is a content graph over the
  bits `Player::known` already carries — a `requires` column and a
  reachability check in `validate` — not a change to the sim. Also absent:
  a blueprint ITEM (learning is instant and personal, so there is nothing
  to trade) and the wipe schedule `DESIGN.md` §8 promises blueprints will
  outlive, which is unbuilt because no wipe is.
  ⚠ **Half struck 2026-08-15 (§0tree).** The `requires` column and the
  `validate` reachability check are built, and the reason the verb felt
  absent was worse than shallow: `bake_research` had no caller, so research
  did not work at all on a live shard. Still true above: the blueprint ITEM,
  the wipe, and that the tree is one edge deep.
- ~~No verb opens a world container~~ — **landed 2026-08-14** (world
  containers v0). This bullet denied a verb the same commit shipped and
  stood for a whole pass; it is the merge-gate judge's ranked fix 1
  (`findings/pass-20260813-230343-05-judge.md`, check 9). Residue is
  §0wc's list, not this one.

---

## 5b · The wire accepts values the sim can never mean — **CLOSED 2026-08-17**

The decode side landed: `why == 3` / `reason` 4..15 refuse as `Malformed`,
counted at the client pump; both `*_MAX`s derive from sim-core and the exempt
list carries them; no `PROTO_VER` turned — the narrowing rule is written at
`PROTO_VER` (lib.rs), and the button octet stays whole at the codec by
decision (`decode_input`'s doc — the wall is `accept_input`'s). The one
residue (`server/core.rs`'s stale "encoder bounds the width" pump comment)
was fixed in the same merge window.

---

## 7 · Milestones — the arc is `DESIGN.md` §11; this is what the queue adds

**Read the arc there, not here.** M0 (landed) → M1 survival verbs → M2
combat true → M3 JUNK → M4 the counter and the door, with each one's exit
condition. `ALPHA.md` §6 folds into the same section, and this list used to
restate it under a second numbering — two lists, one arc, drifting apart.
Struck 2026-08-11; nothing was lost, because everything struck was a
paraphrase of §11.

Two gates sit **between** those milestones and belong to the queue rather
than to the arc:

- **A1 playtest** (operator schedules): 10–20 testers, one wipe cycle,
  after M3 and before A2/A3 arming (`ALPHA.md` §2). A loop proposes it and
  never runs it.
- **Arming A2, then A3** is an operator act, not a milestone anyone here
  completes (`CLAUDE.md` §loop discipline).

And two items the arc does not carry, which stay real work:

1. **Anti-ESP occlusion culling** — the measure the genre proved
    (Facepunch, 2025, network-wide default). Server-side, costs no client
    trust, and the occlusion grid is a pure function of the seed, so it is
    bakeable at worldgen and a lookup in the tick. Sequence after M2: it
    wants real sightlines to tune against.
2. **~~The launcher, in Rust, with the wallet in it~~ — BUILT, and not in
    this repo** (`DECISIONS.md` 2026-08-04 asked for it). It shipped in
    `scry-forge` as `launcher-rs/` — one binary with no runtime to install,
    an account generated on the holder's machine and written as an
    encrypted keystore, and both our depots published and notarized on
    2026-08-10. It reached this list because the row itself said it is
    *"the platform's client for the whole cascade, not a Gates
    accessory"* — which is exactly why it was never ours to build. **What
    is still ours** is the seam: `crates/client/src/scry_overlay.rs` stays
    byte-identical to the SDK upstream (`CLAUDE.md` §vendored), and
    `ci/shardlist.py` writes the document the launcher's Servers window
    reads. Derive the launcher's real state from elo, never from this
    line.

Standing rule: anything a playtest breaks jumps this queue; anything a wall
catches jumps the playtest.

## 5c · The protocol golden never fuzzed a button above bit 1 — **CLOSED 2026-08-18**

`input_full` draws the whole octet now, one fixture moved
(`v46_input_full.bin`), and **no `PROTO_VER` turned**: the judgement call
the item asked for went the way it framed it — a golden's fuzz range is the
test's coverage, not the wire's meaning, so nothing two v46 builds exchange
changed. The rule is written as the narrowing rule's third clause at
`PROTO_VER` (lib.rs) and in `goldens.rs`'s header, which now says which of
the two reasons to regenerate takes a bump and which does not.

`the_input_golden_fuzzes_the_whole_button_octet` reads the fixture BYTES
(every bit set somewhere, and clear somewhere). Its measured job is the
re-narrowing: with the draw wide a masking encoder already reddens the
golden's round-trip, but a narrowed draw regenerates green everywhere else
and this gate alone fails. `goldens.rs:894`'s `loc: next_bounded(4)` looked like
the same defect and is not — four IS the deploy store's domain
(`loc_max(true)`), a wider draw would be unencodable, and the piece lane
already draws all ten; `the_loc_fuzz_covers_each_stores_whole_domain` says
so rather than a comment.

## 5d · The agent player has a spec and no code *(systems lane)*

`PLAYERS.md` landed 2026-08-05 — the verb set, the observation encoder, four
walls. `bots.rs` already drives synthetic input; the intent layer is missing.

**LANDED 2026-08-18 — the condition is logged.** `EV_TRUST` (code 39):
a = actor, b = counterparty, c = `TRUST_*` << 8 | `PRESENCE_*`, pushed by
`World::log_trust` from four sites — a leaf worked (`deploy::use_door`), a
lock's code accepted (`lock_op`), a hearth crew seat taken (`crew_op`), a
box/oven/bag moved through (`World::move_item`). It fires only when the
record's owner is somebody else; `owner == actor`, `owner == 0` and a
`mob::mob_id` are silent. Presence is three values, not a bool — awake,
asleep, **gone** (the body left its slot). Sim-side only, so no wire byte
moved and wall 6 is untouched. Six checks in `tests/event_roles.rs` — four
causes, five silences, one ledger parse — and 15 mutants reproduced red
before any of it was believed.

Remains, in order:
- **Nothing reads it.** `ShardCore`'s drain hits `_ => {}`: a row is minted
  and dropped, so no shard-hour is recorded until a server lane sinks it.
- **A dropped row is gone** — it rides the 256-seat drop-newest ring, and
  unlike every other event a resync cannot re-derive a fact about a moment.
- `TRUST_GIVE` waits on the give verb; there is no player-to-player give.
- Then the verb table, wall 1's subset gate in the same commit, then an agent
  client that plays badly. Entry price and earnings are `ALPHA.md`.

## OP · the operator lane — a loop cannot pick any of these

Moved to the bottom 2026-08-13, unchanged. They sat at the top of the
file for a week, so every pass read ~100 lines of work it is not allowed
to do before reaching an item it could take. They are still live and
still the operator's; nothing here is a queue entry for a builder.

## 0vj · The visual judge is off, and the port back is one script *(operator lane — harness)*

**A loop cannot do this**: the harness is outside the repo by design
(`CLAUDE.md` §the loop that builds this repo). Recorded here so the loop's
missing half is work rather than an absence nobody notices.

The loop restarted 2026-08-13 with `GATES_CAPTURE=0`. `art/capture.mjs`
drives Playwright against the browser client and has been dead since
2026-08-06 — it would fail every pass. So **no frames are captured and no
`-visual.md` is written**: every render pass until this lands is scored by
the merge-gate judge alone, which is the blind-pass condition the visual half
was built to end (M1 slices 15–20).

The replacement is already in the tree and needs no new design:
`crates/client/src/render/capture.rs` — `gates --capture DIR`, the same six
fixed vantages, settling on ring state rather than a clock, `--no-hud` for a
clean plate. `CLAUDE.md` carries this box's working `VK_DRIVER_FILES` + `Xvfb`
invocation. What is missing is a shell wrapper the runner can call in place of
`capture.mjs`, plus re-pointing `TRIPWIRE_FILES` and flipping the default back.

One repo-side half is genuinely ours and worth doing first either way: the
probe writes PNGs only, and the visual judge's prompt asks for a
`manifest.json` carrying the run's errors. A capture that reports what the
client logged while shooting is better evidence than six pictures alone.

## 0sl · The shard list reaches the game *(operator lane — two acts, in order)*

**A loop cannot finish this.** The tree half landed; publishing is the
operator's.

What was wrong: the public shard is up and its list is served
(`/api/launcher/servers/gates`, `servers.url` set), and the in-game browser
was still empty on every launch that did not come through the launcher's
Servers window — nothing on the argv could carry the url. elo gained a
`{servers}` placeholder; `ci/depot.py`'s `LAUNCH_ARGS` now asks for it.
`shards.toml` also said `eu-1` while the served document said `us-east-1`,
so the next regeneration would have re-published a row key nobody's
favourites matched; the served name won.

The two acts, and **this order is not a preference** — a depot using
`{servers}` needs a launcher that knows it, and nothing in the depot
document can declare a launcher floor, so an older launcher refuses the
whole launch:

1. **Ship the launcher** carrying `ARG_VARS` with `servers` in it
   (scry-forge, `launcher-rs`).
2. **Re-publish Gates' depot document**, so `launch.args` carries
   `--servers {servers}`. `python3 ci/depot.py`, then the depot ceremony in
   elo `docs/client/LAUNCHER.md` §8.

Until (2), the fix is inert and the browser stays empty — `--servers <url>`
on the command line is the workaround, and joining from the Servers window
already works.

## 0wd · A new world register is proposed *(operator lane — blocked, skip)*

**A loop cannot pick this up.** Logged here so it is visible, not queued.
`WORLD.md` (new, 2026-08-10) carries an exploratory operator direction, and
is a **roadmap rather than a v1 spec** — nothing in it competes with the
alpha. `DECISIONS.md` §open has the row; nothing is spoken.

Three findings in it are about the tree rather than the fiction:

- **`ART.md`'s bar and the visual rubric are measured off the reference set,
  and the rubric is checksummed outside this repo.** If the register
  changes, every visual pass is scored against pine-and-granite while
  building obsidian, and the builder cannot fix it. Three operator acts —
  palette, a reference set, rubric style section — and 2026-08-01's art row
  already names that exit. **Until then, no visual pass chases this.**
- **A ward would invalidate `CONTENT.md` §4 anchor 2 without reddening
  `test_content`.** The TTK bands compute against `balance.toml`'s
  `globals.player_hp = 100`; a second regenerating pool makes them measure a
  different quantity while staying green. Conditional — the ward is
  explicitly undecided and nothing else depends on it.
- **Extraction and world states are one system or they are two.** An opened
  gate at the bank terminal and a repaired monument are the same object: a
  bounded, tick-expiring, hashed, broadcast state. The terminal lands at A2
  (`ALPHA.md` §2); if it ships a bespoke gate first, that is one idea paid
  for twice.

Cheapest real slice if it is ever spoken: the biome gradient — a radial third
input to `biome(h, moist)` (`terrain.rs:263`) plus regenerated terrain
goldens. `WORLD.md` §9.2 has the full order, and §9.1 the timing: **decide
the register early, build it late.**

## 0gh · The GitHub job-agent seam — the door is built; three acts remain *(operator lane + docs)*

Assessed 2026-08-11; the write-up is scry-forge `docs/builders/GITHUB-JOBS.md`.
Built already: `AGENTS.md` §the deal, the PR template's submit line, `gates`
CI on every code PR, 100,000 ELO standing on elo's board (`DECISIONS.md`
2026-08-09). The board's paid ledger is `[]`; no outside fork has opened a PR.

- **(operator, GitHub)** Branch protection on `main` requiring the `gates`
  check — PRs #56–58 merged over days of red CI before the toolchain pin
  (`DECISIONS.md` §open, the compiler); until GitHub enforces it the merge
  gate is policy. Caveat: the workflow path-filters, so a docs-only PR
  reports no check; the fix is a same-named instant no-op for those paths.
- **(operator, wallet)** Sign `scry.sig.json` seq 1 — and the tooling is
  already here, so **no key is ever pasted**: `./ci/elo_manifest.py
  --print` shows the exact text, sign it in whatever holds the steward
  key, then `./ci/elo_manifest.py --sign --seq 1 --signature 0x…
  --wallet 0x…`. Unsigned, elo applies nothing — the store row and
  update feed are wired and inert. **It now buys more than the row:**
  elo's manifest standard grew a `jobs` block (`GAME-REPO.md` §4b), so
  once signed, this repo posts its own board lane's picked work from
  `scry.json` — guidance rows, never a price — and the six rows elo
  currently keeps house-side move here.
- **(operator, once)** Settle `gates-pr` end to end on the next accepted
  PR: pay by public transfer, append the row elo-side — the board's
  `settled_to_a_worker` stops being zero in public.
- **(operator, GitHub)** The repo description still says "three.js
  frontend" — stale since the browser cut, and it is the first line a
  stranger reads above `AGENTS.md`. GitHub → About; no API path for it here.
- ~~Milestones live twice~~ — **done 2026-08-11.** `DESIGN.md` §11 owns
  the arc, §7 here points at it, M0's seven dead checkboxes are prose.

Not owed, stated so it is not re-litigated: no issues queue (this file is
the queue), no auto-pay or auto-merge (merge is the act that pays, a hand
act), no webhook (the store seam stays a commit and a poke).

