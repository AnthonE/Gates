# Gates · NOW.md — what next

The only list that answers "what should the loop pick up." Done items are
deleted, not checked — history lives in git and `DECISIONS.md`. An item is
≤ ~25 lines (`CLAUDE.md` §loop discipline); detail belongs in
`DECISIONS.md` §open or a `findings/` note.

> **Rebuilt 2026-08-25: 3,273 → 1,718 lines, 82 items → 76.** Every
> section was re-read against the tree with commands rather than against its
> own memory, and what came back is that almost nothing here was *finished* —
> it was **buried**. Two sections were closed outright (`0kit`, `5c`); the
> other eighty were nine parts landed narrative to one part live work, so what
> is deleted is the story of what already shipped and what is kept is the
> remainder, with a `file:line` on it.
>
> **Nothing open was dropped, and that was checked rather than claimed.** The
> rewrite was diffed against the original a second time, adversarially, asking
> only *what live work went missing* — and it found **nineteen** items, each
> re-verified against the tree before being restored: the head-look spine
> follow-up, the tech-tree panel's edges, the stairs-drawn-as-a-plate, the
> fleet raiding itself, what the `prove` slice actually costs, and fourteen
> more. One thing it flagged was **correctly** dropped and stays dropped: the
> `bevy_procedural_tree` bug report, which `render/tree.rs:241` says was filed.
> A prune that is not diffed back is a prune that loses things quietly.
>
> **Three sections that read as settled are not, and each was found by a
> command.** `5b` says **CLOSED** and covers two of four over-wide wire
> domains — the craft-refused and deploy-refused reasons are raw octets,
> unchecked at both ends and absent from `event.rs`'s `DOMAINS` table. `0zd`'s
> *"not owed, do not re-litigate"* rests on a blocker that **died**:
> `ItemStack` gained `cond: u16` in durability v0, which is the per-item
> instance data the key lock was refused for. `0bd`'s tree row is half-closed:
> `OCCUPANT_TOP_M[Tree]` cites a dead-code far-LOD constant, so the sim blocks
> 0.3 m of invisible ceiling over half the pool.
>
> **The queue's real shape: 25 of these items are the same one act** — not
> code, but a person booting the game on a machine with a GPU and looking. `§LOOK` at the
> bottom is that list, in one place for the first time; the swing, the decal,
> the death pose, LOW and MEDIUM, the far forest, the broadleaf, the announce
> stack and the whole audio bank have never been seen or heard by anyone.
> `CLAUDE.md` says the visual gate is a person and forbids building a pixel
> gate to replace them, so this is the bottleneck by design — but it had never
> been counted, and it is the largest single thing standing between this tree
> and a playtest.
>
> ⚠ **Section labels still collide** (`0a 0u 0v 0w 0x 0y 0z 4b`) because
> `merge=union` lets each lane pick "the next free letter" against a file that
> does not hold the others' picks. They are **not** renumbered — the citations
> are mostly in `DECISIONS.md`, which is the dated record and is not rewritten
> to match a later tidy — so read a `§`-citation as a hint and match on the
> title. `§Labels` at the end says which are ambiguous and what was deleted.
> When you next edit a colliding section, give it a label no other section has.
>
> ⚠ **One section had lost its heading entirely** and had been eaten by `0bl`
> for long enough that two crate comments cite it by a label the file did not
> contain. It is `§0sun` now.

---

# Buildable now — a loop can pick any of these

## 0fp · What the first-person pass left *(client lane)*

From play (operator, 2026-08-30): *"when i run my player is all blurry like
its snapping around"*, *"held items are shitty looking like again they are
not under the parent"*, *"the swing animation is the most underwhelming thing
ever"*, *"we need some kinda effect particle wise when u actually connect"*.
All four are answered (`DECISIONS.md` 2026-08-30 first-person feel v1, and
2026-08-31 for the two it left) — `ClientCore::eye_position`,
`VIEWMODEL_GRIP_M`, `viewmodel::swing_pose`, `render/impact.rs`,
`bodies::bind_hands`. What remains:

1. **Nobody has seen any of it** — §LOOK. The arc, the grip and the chips
   are gated as arithmetic (the item stays in frame, the grip reproduces the
   hold pose, the pool holds its cap) and *how they read* is a person with a
   GPU. The swing's timings especially: `VIEWMODEL_SWING_S` and the wind-up
   split are PROPOSED, not spoken.
2. ✅ **The remote body's item is in its hand** (2026-08-31). `bind_hands`
   binds `anim::HAND_BONE` per body and both hands compose one
   `viewmodel::grip`. The offset it replaced was **0.690 m wrong and on the
   wrong shoulder** — `bodies::RETIRED_BODY_PALM` carries the retraction.
3. **A gather burst is placed from the client's pick, not the sim's.**
   `EV_GATHER` carries an item and no cell, so `impact::strike` reads
   `verbs::Swung`. Right whenever the player is swinging at what they are
   looking at, which is always in practice and not by construction. A cell on
   the gather event would retire the seam; it is a wire field for a cosmetic,
   so it is priced here rather than taken.
4. ✅ **A mob takes a blow and throws chips** (2026-08-31) — `impact::struck`
   splits the id space and `mobs::flank_h_of` reads the height off the
   shipped mesh table.
5. **A remote body's own swing is still the rig's `Sword_Attack`, and the
   first-person one is not.** Two strokes for one event: 1.053 s of authored
   clip out there against 0.45 s of `swing_pose` in here, so a duel's two
   views disagree about how long a swing takes. Deliberate
   (`VIEWMODEL_SWING_S`'s doc: your own apex must not lag the cue) and
   unmeasured — nobody has watched the two side by side.

## 0cs · The fight at population — what the combat storm left *(sim lane)*

From the merge-gate judge's ranked gap 3, `findings/pass-20260829-153230-21-judge.md`
(*"nothing has ever fought — at population, over a link, or in front of a
person"*). ✅ **The deposit half landed 2026-08-30**:
`sim-core/tests/combat_storm.rs` seats all 100 seats as 50 duels — 25 knife
fights, 25 gunfights on `bots::brawl_step` — and over 600 ticks banks 954
deaths, pins the bag store at `MAX_BACKPACKS` **evicting through
`World::die`**, and fills the event ring. Four mutants, four kills.
✅ **The withdrawal half landed 2026-08-30** — `tests/loot_storm.rs`
fills to the cap, then runs four 8-tick bursts of 100 bodies fighting
*and* sending `Command::Loot`; every burst begins full and reaches zero.
Eight mutants, six killed, two survivors explained in its header. Its two
carried findings are in that header and in `CLAUDE.md`'s trap list (an
event count taken while the ring overflows is an undercount). What
remains:

1. **Nobody moves, so the rewind is a lookup and not a correction.**
   `brawl_step` sends `move_x`/`move_z` zero and every command carries a
   drawn `favour`, so `Rewind::pose_at` runs a hundred distinct depths a
   tick against static poses. A duellist that strafes would make the depth
   change the *answer*, which is the claim `§0lc` still cannot make.
2. **The raid storm still has no bodies in it** — `§0rs` item 1, unchanged.
   The two storms are siblings by design (arming the raid fixture would
   desaturate its five equality assertions), so what is missing is the
   *third* case: a blast that takes a wall and the person behind it.

## 0mag · Reload v1 — what the magazine still cannot do *(systems+client lane)*

*Landed 2026-08-30, gap 1 of `findings/pass-20260829-153230-18-judge.md`
(also -17's gap 3): a firearm never reloaded, so a fight was an unbroken
stream of clicks and three passes of damage-ladder work had nothing to be
read against. Wire v59, world save format 12, `crates/sim-core/tests/
reload.rs`. `DECISIONS.md` §open has the six knobs. **Item 6 landed
2026-08-30**: `botclient.rs` answers its own dry click and re-asks on every
BUSY, gated by `bot_smoke::test_bots_reload_over_the_wire`, which arms the
fleet because no bot has ever held a firearm. Relabelled from `0rl`, which
named two sections (judge fix 3, pass -19). **Item 1 landed 2026-08-30**:
`die` sheds `mag`/`mag_round` into the same `shed` buffer `worn` uses, four
lines and no content lookup — the pair of arrays already names the item and
counts it. Gated as conservation (`a_death_sheds_the_cylinder_into_the_bag`:
every round the body owned is in a bag), with the shipped slot cost measured
at 1 of 28 free (`the_shed_magazines_fit_the_bag_they_shed_into`). Four
mutants, four kills — and the fourth only after the cond assertion was split
out, because a top-up never restamps `cond` and it was reading the pack's
zero as the magazine's.*

1. **A reconnect off `PlayerSave` finds the cylinder empty.** The world
   save carries the magazine (it had to — `state_hash` folds it), the
   store's per-player record does not: that is a second format bump in
   `server/src/store.rs` and this slice did not spend it.
2. **No unload, and no ammo switch.** The reference refunds a partial
   magazine and adopts the new round at `StartReload`; ours refuses to mix
   (`REFUSE_RL_DRY`). Both need `reload` to see a stack ceiling, which
   means `GatherContent` reaching a `CombatContent` caller.
3. **The dry click rides `rate_ticks`; the reference gives it its own
   1.0 s.** `BaseProjectile.ServerUse` starts a fixed attack cooldown on an
   empty magazine rather than the weapon's cadence. Ours is 0.4 s on the
   revolver. `DECISIONS.md` §open carries it as an unspoken knob.
4. **Nobody has seen or heard it** (§LOOK). The readout over the hotbar,
   whether `R` reading as reload-not-repair is a surprise, and whether
   `Cue::Place` borrowed for a seated magazine reads as a reload are all
   unmeasured — capture is off and this box has no sound card.

## 0vs · The newest visual report's ranked gaps are **closed** — steer from the judge *(any lane)*

Checked with commands rather than read, 2026-08-30, because this is a gap pass
and the visual half is the older half: the newest `-visual.md` is
`pass-20260815-042118-11`, capture has been off since 2026-08-28, and **two of
its three ranked gaps no longer describe this tree.**

- Gap 1 (*"the island paints one material"*, 1.09x granite:turf) —
  `render/ground_splat.rs` exists and `terrain_mesh.rs` documents **2.28x**,
  past the 2.06x the report asked for, gated by
  `granite_stands_clear_of_the_ground_it_shares`.
- Gap 2 (*"`grep -rn DirectionalLightShadowMap|ShadowFilteringMethod|
  shadow_depth_bias|shadow_normal_bias` returns nothing"*) — it returns
  `render/quality.rs:42,164` now: cascades, shadow-map size and SSAO are
  tiered.
- Gap 3 (no structure, no character, no viewmodel in any frame) — **partly**:
  `capture.rs` has `Subject::Player` and `Subject::Build` extra shots. The
  panels-off rule and the missing hands are still real.

The **judge** report's gap 2 is partly stale the same way, and its own grep is
why: `airdrop|heli|Airdrop` cannot find **site guards v0**, which landed
2026-08-14 and is gated (`tests/guard.rs`, six sections). What was genuinely
missing is the half this pass took — the guard roster was **flat** while
`ci/haven_prize.mjs` gates a strictly rising three-tier prize, so the richest
site cost the same to rob as the middle one. `HAVEN_GUARDS`/`WAYSTATION_GUARDS`
now rise with it (`DECISIONS.md` §open, "the guard chain").

Still open from that gap and larger than a pass: a **world event** — a timed,
announced window at the pad — and a guard **loot tier** (§0m 3, which needs a
third species and five client arms). The judge's gap 3, no session or wipe
boundary, is untouched and is the bigger of the two.

## 0h3 · The other flaky dial: `connection closed by peer: 261` *(server lane)*

`ci/gates.sh` went red then green on an unchanged tree (2026-08-30). **Two
different mechanisms, and only one is fixed.** The reproduced one was ours and
is closed: `bot_frame` re-rolls `sel` at random every frame, so the satchel a
raid step selected was clobbered before the throw executed and eight raiders
armed zero charges on a loaded box (`botclient::HeldSel`, 18/18 clean under the
load that broke it twice).

The other is **not diagnosed**. `raider 0 failed: connect: connection closed by
peer: 261` — H3_FRAME_UNEXPECTED (RFC 9114 §8.1), empty reason, so not one of
our `REFUSE_*` answers (0..=5, always with `refuse_text`). Seen once in a gate
run, never in ~60 loaded runs here. wtransport's server driver emits 0x105 from
three sites (`driver/mod.rs:598,637`, `driver/streams/settings.rs:123`), all
about H3 framing our code never touches; our admission gate is not it
(`ADMIT_REFUSE_AT` is 400 against 58 bots).

What is done: the raider panic now prints `handshake_errors` / `admit_refused` /
`admit_retried` off the shard, so the next occurrence is readable in the log
instead of costing a pass. What is **not** done and should not be guessed:
widening `is_load_shed` past `0x107` — its own gate asserts `!0x0106` and
`!0x0108` with the reason, and a predicate widened to a range is how a refusal
starts being retried. Next step is server-side wtransport tracing on a repro,
or a pin bump past `a11e6a8` if upstream has a framing fix.

## 0lc · Lag compensation — **on**, and never fired over a real link *(sim lane)*

**Live since slice 5 (2026-08-30).** `stats::favour_for` mints
`min((T−S)+3, 7)` at the one site that builds `Command::Input`; zero for an
unacked client and past the ceiling. Four counters on `/status.json`.

**Slice 6 (2026-08-30) put the GUN's rewind on the parity surface** — item 2
below, now closed. `combat::probe_fixture` gained a hitscan row (item 6, its
round item 7) and `probe_combat` re-arms it every tick, because `World::die`
clears `inv`. `ranged::hitscan` is the only shot path that reads the ring
(`Pose::Rewound`; the arrow stays live), so before this `test_parity_wasm`
covered the melee reader alone while this item read as though it covered
both. The count gated is the **consequence** — a hitscan shot whose shooter
was rewound that tick, 2415 of them at 500×256 — not "a gun fired".
Gates: `ci/gates.sh` on the full run, `tests/rewind.rs` in `cargo test`.
Both mutants red (fixture row inert, favours collapsed to zero → 0 each);
measurements in `findings/note-20260830-the-gun-rides-the-parity-surface.md`.

**The lesson stands and belongs in a trap list.** The sixteen gates the mint
landed with were ALL green under the `favour: 0` literal that shipped, and a
digest is evidence of parity, never of coverage. Gate the consequence.

Remaining — **one item, and it is not a loop's to do:**

1. **Nobody has fired at a moving remote player over a real link.** ~4.1
   ticks on loopback says nothing about 200 ms, and the clamp is not
   re-derivable from this box. Needs one session with two `--features render`
   clients and `/status.json` read at both ends — `favour_clamped` is the
   number that says whether 7 is right. §LOOK, and the judge's own caveat.

## 0hrt · Being hit points somewhere — the rest of the fight *(systems+client lane)*

*From `findings/pass-20260829-153230-06-judge.md` gap 2.* Wire v57 gave the
**victim** a fact: `EV_HURT`, a 16-sector world bearing drawn as a fading
arc. Items 2, 3 and 5 have landed. What is left:

1. **Being shot is silent** — **half landed 2026-08-30** (hurt weight v0):
   `sound::hurt::request` takes the **fall and the event**, so the three
   routes `damage_routes.rs` marks silent stay audible by construction, a
   blow armor ate whole is no longer silent, and the cue's gain is the
   blow's weight against `hp_max` instead of a flat 0.80. Eight mutants red;
   `DECISIONS.md` §open has the knobs. Two residuals, both about the *cue*
   rather than the routes:
   · **The mixer still cannot say where.** `Feed` merges a bearing per
     sector and only the arc reads it. `Cue::Hurt` is `positional: false`
     with `radius_m: 0`, and turning that on needs a projected position at
     an invented distance — the bearing is a direction, not a place.
   · **Two blows in one frame are one voice**, now a heavier one (the
     damage sums before the weight is taken), which is not two sounds. The
     120 ms cooldown is per-CUE and binds inside a frame on purpose
     (`a_cooldown_binds_within_one_frame`), so this is a second row, not a
     smaller number. No camera shake either (§0pvp item 2).
2. ~~One arc, latest wins~~ **Landed 2026-08-30** (hurt direction v1):
   `Feed` merges by bearing, `Toast` holds `HURT_ARCS = 3` independent
   clocks, a fourth direction evicts the oldest and never the new blow.
3. ~~`headshot_mult` is still unread~~ **Landed 2026-08-30** (headshot v0).
   What it left open is §0hs.
4. **Nobody has seen the arc — now less so, and worse.** No capture vantage
   stands where a shot can land on the camera, and three arcs at once on a
   116 px ring is exactly the thing arithmetic cannot judge: whether three
   28° smears read as three directions or as a red halo (§LOOK).
5. ~~Two damage routes say nothing~~ **Landed 2026-08-30.** A bite reads the
   animal's post-step body, a blast the epicentre's horizontal bearing.
   `damage_routes.rs`'s `ROUTES` grew an announce column so the next route
   is born announced or born exempt; `tests/hurt_routes.rs` drives both from
   two sides each.


## 0hs · The body-part ladder — what limb band v0 left *(systems lane)*

*Item 2 landed 2026-08-30 (limb band v0): `LIMB_BAND_M = 0.85`, a
`limb_pct` column on all eleven weapon rows at the reference's 50, and
`head_crossed` replaced by `ranged::part_crossed` → `collide::Part`, whose
derived `Ord` **is** §7's most-significant-part rule. `DECISIONS.md` §open
has the knob; `reference/PROJECTILES.md` §9.4b has the design. Eleven
mutants run, ten caught outright and the eleventh — `balance.rs`'s `!=`
weakened to `<` — survived until `the_body_part_ladder_refuses_what_it_names`
landed, because shipped content agrees with a band by construction.*

1. **The clip's mutant still survives, and the fixture this item used to
   propose cannot be built.** `exit.min(stop_t)` dropped at both damage
   sites reddens nothing. Measured this pass: the world must go solid
   *inside* the far half of the victim's own 0.8 m footprint, which a wall
   cannot do to a body standing legally in front of it. The route that
   works is the arrow's tick boundary (`world_stop` returns 1.0 and
   `BodyHit::exit` is unclipped) — a `tests/shoot.rs`-shaped fixture
   driving `step`. `tests/headshot.rs`'s header has the arithmetic.
2. **No arm, and no per-weapon override has been used.** What shipped is a
   *leg* band — a cylinder cannot tell an arm from the chest beside it.
   Every row carries the band, so the override §0 names exists and nothing
   exercises it; the first weapon that should differ is the case that says
   whether the geometry should widen.
3. ~~Nothing tells you which part it was~~ **Landed 2026-08-30 (hit rung
   v0)**, judge gap 1 of `pass-20260829-153230-17`. The rung rides two
   spare bits of `EV_HIT.c` (`world::hit_c`; the damage is a `u16` in a
   `u32` and `a`/`b` were both spent), `PROTO_VER` 57 → 58, and reaches
   the screen as three marker colours and three cues. Eight mutants red.
   `DECISIONS.md` §open has the four cosmetic knobs. Three residuals:
   · **Nobody has seen or heard it** (§LOOK) — capture is off and this box
     has no sound card, so whether gold reads as a skull and whether the
     limb cue reads as *lighter* rather than *quieter* is unmeasured.
   · **The marker changes colour and not shape.** A rung is one channel
     wide on screen; the reference pushes the ticks outward too, which is
     a `Node` mutation per tick rather than a `BackgroundColor` write.
   · **`Toast::hit_damage` is still stored and never drawn** — the free
     surface a number-per-rung would use, unchanged by this slice.

## 0tl · The torch lights the ground — what it still cannot do *(client+systems lane)*

*Two gap passes deep. The light landed 2026-08-29 (torch light v0) and
**items 3 and 4 landed the same day** (torch fuel v0, wire v55): a torch
burns 1 000 hundredths of condition a minute — the reference's 1/6 point a
second, so five minutes exactly — and right-click puts it out. There is no
`lit` flag: a flame is derived on both sides from the `BTN_LIGHT` latch,
the item's `light_burn` row and its `cond`. `DECISIONS.md` §open has the
reading and the two save formats it cost.*

1. **Nobody has seen it, and no gate here can.** `rig::CAPTURE_DAY_FRAC`
   pins every capture to noon, so no frame any visual judge has scored was
   shot after dark. A night vantage needs a per-vantage day fraction, which
   is capture plumbing, not a `VANTAGES` row — and capture is off. `§LOOK`.
2. **The ambient is the ceiling, not the lumens.** `pool_radius_m` says the
   torch beats 60-lux night out to **0.89 m** and the campfire to 1.09 m —
   pools you stand in, not lights you see by. `NIGHT_AMBIENT_LUX` is 240×
   moonlight by its own doc, and raising a flame to compensate is the
   failure `threejs-exposure-color-grading` names. The fix is one owner over
   `rig`'s coupled set (`CLAUDE.md` §traps) and a look only a person can
   judge. Do not touch it from a lane that is also changing a light.
3. **A torch still wears nothing when you HIT with it.** The reference
   charges ~7 condition a landed swing on top of the burn; ours has no
   `condition_loss` row naming it and V3 forbids an unreachable one, so
   this wants a node or a combat row first, not a number.
4. **Nothing says a torch went out.** The client learns it from `cond`
   arriving at 0 on `SUB_INV`, which is one round trip late and silent —
   no cue, no toast, no flicker. The mixer has no `Cue` for either edge.
5. ~~No other player's torch lights anything.~~ **Landed 2026-08-29**
   (remote hands v0, wire v56): `EntityState` carries `held` (the item id
   in the selected hotbar slot, or nothing) and `lit` (`light::is_lit`,
   server-resolved because two of its three facts are the holder's own),
   and `bodies.rs` hangs a posed mesh and a `PointLight` off every remote.
   Three residuals, each its own slice:
   · **The item does not swing with the arm.** `BODY_PALM` is a fixed
     offset on the body root — the rig has one bound bone (`HEAD_BONE`)
     and `models/stumpy.glb`'s hand bone name is unverified. Reads across
     a clearing, reads as floating up close.
   · **Nobody has seen it.** No capture contains two players, and the one
     that would is a night vantage (item 1). `§LOOK`.
   · **The worst-case datagram is now 1058 B of 1100** (`snapshot_cap`,
     994 B at v55). The record cap still binds first, but the next field
     on this record is ~5 B/bit of headroom, not free.
6. **The flame is not drawn.** No emissive and no flame geometry — the head
   is lit from 4 cm above its crown instead. `nothing_held_glows` still
   holds and did not have to move; a real flame is a VFX slice. **Now two
   hands wide**: `bodies::BodyFlame` is the same lightless flame on every
   remote.

## 0shot · A gun is heard — what it still cannot be *seen* doing *(client lane)*

*The sound half landed at gun report v0 (wire v54, 2026-08-29):
`EV_SHOT` fires on the hitscan path with `speed == 0` meaning instant and
the low `u16` carrying the reach in decimetres, and the mixer plays it at
the shooter — 100 m for a gun against 40 m for a bow, both the
reference's. `DECISIONS.md` §open has the reading and the cap it cost.*

1. **Nothing draws the beam or the flash.** `tracer.rs` declines to fly an
   instant shot (it would hang a motionless streak for four seconds) and
   draws nothing in its place, so a firearm is now loud and still
   invisible. **No wire work is owed** — the reach crosses on `EV_SHOT`
   already, which is why it was put there — so this is a pure render
   slice: a line from the muzzle along (yaw, pitch) of the carried length,
   alive for a frame or two rather than `MAX_ARROW_LIFE_TICKS`, and not a
   `Tracers` slot's shape. A muzzle flash is the same event and a
   different primitive.
2. **A distant shot is quiet, not muffled.** `sound::falloff` is amplitude
   only, so the 100 m report arrives with its full brightness at 99 m.
   Air absorbs treble long before it absorbs bass, which is most of how a
   player judges *how far away* a gunfight is. That belongs to the mixer,
   not to the waveform, and it would pay for every positional cue at once.

## 0eq · Equipment, after armor v1 *(systems+client lane)*

*Gap pass, from `findings/pass-20260828-065501-05-judge.md` ranked gap 1
("you cannot put armor on, so every fight in Gates is naked against
naked"). The verb landed this pass — wire v51, `CONT_WEAR`. What is below
is what that opened rather than what it closed.*

✅ **1, 2 and 4 landed 2026-08-28.** The readout was wire v52; the wear
view moved off the ground subscription with **no wire change at all**
(`DECISIONS.md` §open, wear stream v1). Item 2's stated mechanism was
**wrong and is worth keeping as the correction**: it said the total was
"off state the client already has", and the client had names and
condition ceilings only — `worn_pct` needed two columns on the catalog
drip, which is why that one was a bump and not a panel edit.

3. **A body is not drawn wearing anything.** `worn` crosses the wire only
   as the owner's own panel; the mannequin has no armor mesh and no wire
   fact to key one off. Wants a design pass before a byte — it is AOI
   fan-out of what everyone is wearing, which is the raid intelligence
   `container_wire.rs`'s wear test refuses to broadcast today.
5. **Nobody has looked at the paperdoll**, and it is now on screen far
   more often — it draws on every inventory screen rather than only when
   the body was the open container. Two rounded `Node`s, no asset, and no
   frame in `findings/` contains it. `§LOOK`.
   ⚠ **The three-panel row has never been measured against the screen.**
   Pack + body + container now sit side by side where two did; nothing
   here knows whether that fits at 1280 wide or wraps. One number an
   operator can read off a frame.
6. **Armor still does not wear out.** §9.4's condition, now that the
   catalog carries `cond_max` beside the reduction: a worn piece has both
   halves on the client and debits neither. `§0dur` owns it.

## 0gs · What ground surface v1 left open *(client lane)*

The ground stopped repeating and `rock` stopped being a wall (`DECISIONS.md`
§open, ground surface v1). Five things it did not do; two are closed.

1. **Nobody has booted it**, and the biplanar tap is the sharp end: it is WGSL
   that no GPU in this container can compile, so it is gated by *scrapes* of
   its own source and by nothing that has run it. `§LOOK`.
2. ✅ **The tiling is per identity now** (`DECISIONS.md` §open, ground tiling
   v1, 2026-08-28) — 4.0 sand · 2.0 grass · 1.3 litter · 4.0 rock. What it
   leaves open is the **rule 7 half**: litter's repeat is 3.1× more frequent
   than it was, `MACRO_M`'s 48 m break-up is the only thing standing against
   it, and no arithmetic here can see whether that is enough. `§LOOK` with
   item 1 — grass and litter changed, sand and rock are bit-unchanged.
3. **`rock` is one identity doing three jobs** — alpine ground, the cliff face
   the slope veto forces, and the ore-node prop. Scree is right for the first
   and arguable for the other two; the runner-up (`Gravel005`) was passed over
   for exactly this. Splitting cliff from ground is a fifth splat channel and
   a `CONT_KIND_BITS`-shaped question, not a texture swap.
4. **`ROUGH_MEAN[3]` is now 0.536**, 0.43 below sand, and the wet term
   multiplies down from there. If a mountain reads as glossy in the first
   frame anyone draws, that is the number to suspect.
5. ✅ **The prose around the constants is gated now** (`tests/
   manifest_measured.rs`, 2026-08-28 — three tables and three statements, all
   proven red under mutants). It closed the judged `sand` defect and found
   that the `Gravel004` swap had left `Rock023`'s numbers in four more places
   plus a dead `pub ROCK_GAIN`. **What it leaves open is `gravel`**: the one
   role in `MANIFEST.md` with no `bundled` tick, so nothing loads it and this
   gate does not read it. It is the obvious source for item 3's cliff, which
   is the only reason to keep the row.

⚠ **This item is now `§LOOK`'s, not a loop's, except for item 3.** Items 1, 2
and 4 all end at "boot it and look", item 5 is closed, and item 3 needs both a
texture and a `[u8; 4]` → 5 widening of `terrain::splat` — which is the scatter
mix, `Biome`'s four rows, the minimap palette and `test_terrain_golden` in one
commit, not a texture swap. **Sized 2026-08-28 and it is not one pass**:
30 source files, 22 test files red, the shader's five four-tap blocks — and
the blocker is that `ATTRIBUTE_COLOR` carries the four weights and is FULL,
so a fifth needs a vertex channel that does not exist. `map_palette.rs`
already reddens on the arity, which is the one free part.


## 0wg · What worldgen shape v1 left open *(sim+client lane)*

The island stopped rendering as a contour map (`DECISIONS.md` §open, worldgen
shape v1: `remap` is a monotone cubic, detail rides after the curve, the
highland blend is a ridged multifractal). Four things it did not do.

1. **Nobody has booted it** — the whole slice is hillshades of
   `terrain::height` and arithmetic gates. The operator's screenshot has not
   been re-shot, and `--features render` does not build on the container it
   landed in (three `-dev` packages, `CLAUDE.md` §container). Belongs to
   `§LOOK`, and it is the only item there that is a *regression* risk rather
   than an unseen feature: the shape under every prop, tree and clutter tile
   moved.
2. **The lowlands are still flat**, and that is a choice this slice made
   rather than a defect it missed. Detail is weighted by `shelf²`, so a 14 m
   shelf keeps 2.6% of `DETAIL_AMP` where the summit keeps all of it — bought
   deliberately, because a linear weight costs the doorway, the wolf hunt and
   the replay build floor at ~8 m (measured, three gates, clean dose-response).
   If the flats should have relief, the answer is not a bigger amplitude, it
   is relief that does not fight `foundation_terrain_ok` — a field that varies
   *between* build cells and is flat *within* one.
3. **18.75 m is the finest relief worldgen may author**, because `FAR_STEP` is
   8 m and the far mesh cannot resolve below ~16 m of wavelength. Anything
   finer wants the far mesh's step to come down first, which is a budget
   question nobody has costed.
4. **The statistical contour gate does not exist and should not be rebuilt**
   until someone has a metric that separates. Binning |∇‖∇h‖| by elevation was
   built and measured: 3.58–4.65× the median before the fix, 1.54–3.52× after,
   overlapping over four seeds, because it cannot tell a crease from the LUT's
   designed cliffs. `tests/contour.rs` gates the mechanism instead.


## Sim, content and gameplay verbs *(systems lane)*


## 5 · Gameplay still missing, in rough order of what a player notices

Items 1–3 are a **spoken operator call**, not a builder's proposal — 2026-08-10:
*"ranged tracks the reference game as closely as we can, and arrows come back"*
(`DECISIONS.md`; `reference/PROJECTILES.md` §9 is the sized list).

1. ✅ **A shot chips the wall it stops on** (ranged structure damage v0,
   2026-08-28), **and a deployable too** (deploy shots v0, same day —
   §0mk item 2). Arrow and bullet, `collide::{shot_stop, deploy_stop}` name
   the address, `World::chip` charges the right store. Neither invented a
   knob, and **a swing marks the same plank now** (melee raid mark v0,
   2026-08-28 — §0mk item 1). What they leave open: nobody has watched a
   wall come down, a bench fall, or a decal draw at all (`§LOOK`).
2. ✅ **An arrow lies where it lands** (arrow recovery v0, 2026-08-28) —
   §9.7's pieces 1 and 2: `sim-core/src/spent.rs`'s store, the 15 % break
   roll keyed on (seed, tick, slot), the 10 s lodge for an arrow that drew
   blood. Both numbers are §5's, taken whole into `content/balance.toml`.
   Hashed and saved (`WORLD_SAVE_FORMAT` 10).
   ✅ **And you can pick one up** (arrow recovery v1, 2026-08-29, wire
   v53) — `V`, `spent::pickup`, reach aliased to `BUILD_REACH_M`, in 3D.
   Two stated gaps, both in `spent.rs`: a lodged arrow does not travel
   with the body it is in (it lies at the impact point), and an arrow that
   expires in mid-air leaves nothing, because it has no landing point.
   **§9.6 is unblocked and not done**: their ~35 bow damage was refused
   only because our ammunition never came back, and now it does — so the
   row is a `RIPLIST.md` take against `CONTENT.md` §4's bands, not a
   research question.
3. ~~`headshot_mult` is armed and unread~~ **Landed 2026-08-30** (headshot
   v0): §7's rule, ranged only. §9.4 has what shipped; §0hs has what did
   not. The ×0.5 limb is deliberately untaken — a third part is where the
   two-part reduction stops working.
4. **A forest-floor pickup archetype, and a farming lane.** Both code.
   `server/tests/farmwalk.rs` measures a gather rate; it is not farming.
5. **The tech tree is one edge deep.** `requires` and the `validate`
   reachability check ship, and `bake_research` has a caller now, but every
   row in `content/research.toml` depends on a root. Still absent: a
   blueprint ITEM (learning is instant and personal, so there is nothing to
   trade), and the wipe schedule `DESIGN.md` §8 promises blueprints will
   outlive.
6. **Day/night reads nothing but the mobs and the hand.** `mob::think` is
   nocturnal, a held torch lights the ground, and since torch fuel v0 it
   burns five minutes and can be put out (`§0tl`) — so night has a counter
   with a price. ⚠ What it still has **no reason** is the other half, and
   no item owns it: nothing in `content/` can only be had after dark
   (`findings/pass-20260829-153230-04-judge.md` gap 2). Still missing:
   crops, moon and stars in the night sky, and a set-time verb — moving
   the clock means moving the tick.


## 0pvp · What a fight still cannot do *(systems lane)*

1. **The flinch is attacker-side only** — `EV_HIT` is unicast, so the recoil
   is on one screen and nobody has seen the pose. The *victim's* half landed
   as `EV_HURT` (wire v57, §0hrt); a **bystander** flinch is still refused on
   fan-out grounds (`DECISIONS.md` §open "attacker-side flinch v0").
2. **No positional hit sound** — a flesh impact needs a waveform `sound/
   synth.rs` does not generate. Nobody has heard `Cue::RemoteSwing` either.
3. **A gun is heard but not seen** — the crack landed (gun report v0, wire
   v54): `ranged::hitscan` raises `EV_SHOT` at `speed == 0` and the mixer
   plays it at the shooter, 100 m against a bow's 40 m. No muzzle flash and
   no beam yet, and the reach is already on the wire for whoever draws
   them — `§0shot`. The firearm death cause landed at v53.
4. **Armor can be worn (armor v1, wire v51) — what it still owes.** The
   equip half is done: `CONT_WEAR` is kind 4, the field is 3 bits, and
   `move_item` carries it with no new verb. Remaining, none of it blocking:
   `balance.rs`'s anchor is still slot-blind and ceiling-only — needs
   `armor_extra_hits_max` re-spoken or the ladder re-priced; damage types
   and hit areas (`reference/ARMOR.md` §9.3, deferred by name); condition
   on a worn piece (§9.4 — rides on the reduction path, which now exists);
   `move_penalty_pct` still reaches no line of `movement.rs`
   (`bake.rs:866`), and §9.5 item 4 wants its non-stacking rule spoken
   first. ✅ The paperdoll and the protection readout landed 2026-08-28
   (wire v52, `§0eq`); nobody has looked at either.
5. **No lag compensation, but the ring is in** — `sim-core/src/rewind.rs`
   landed 2026-08-30 (slice 2) with **no reader**, so a fight is still
   resolved against present-tick positions. Slices 3–5 are `§0lc`; no wire
   bump is owed.
6. ✅ **Nothing had fought at population** — closed 2026-08-30 by
   `sim-core/tests/combat_storm.rs` (`§0cs`), which drives 100 bodies
   through melee, hitscan, death, the corpse bag and the spawn ring. What
   it does *not* cover is in `§0cs`; `raid_storm.rs`'s bodies are `§0rs`.


## 0mk · A swing at a piece marks nothing, and a deployable eats no shot *(systems+client lane)*

✅ **The floor half landed 2026-08-25** (shot planes v0, `DECISIONS.md` §open):
`collide::cell_planes_stop_shot` is the body walk's slab set at the
arrowhead's radius, and `render/decal.rs::plane_face` gives a mark on a slab
a `±Y` normal instead of a wall's. Item 1 was behind it and is now free.

1. ✅ **A swing at a built piece marks it now** (melee raid mark v0,
   2026-08-28). `combat::piece_mark` is the point, `SURF_BUILT` the kind,
   and the deployable arm rides `deploy_stop`'s own clamp — so a hatchet
   and a bow scuff one plank and `EV_IMPACT` has three producers. No wire
   byte, no `PROTO_VER`, no client line. `tests/mark.rs` proves the point
   is ON the piece for all ten `loc` arms; eleven mutants, all red. Flesh
   stays unmarked by choice (one spare code). **What it does not do**: the
   mark ignores aim, so it is the nearest point of the piece rather than
   the point swung at — a triangle and a diagonal therefore take one spot
   per piece. Giving them more wants a ray this arm does not have.
2. ✅ **A solid deployable stops a shot now** (deploy shots v0, 2026-08-28):
   `collide::deploy_stop` is `deploy_blocked` with a projectile's profile,
   `ranged::Struck` is what lets one four-part address say which store it
   came from, and `World::chip` charges `damage_deploy` flat — no side, no
   removal budget, which is what `combat::raid` and `charge::detonate`
   already pay. What it leaves open is the **door's own volume**: a shut
   door blocks as an *edge* through `ColMasks::shut_*`, which `shot_stop`
   already walks, so nothing is owed there — but a door standing OPEN is
   still air to a shot, exactly as it is to a body, and whether that is
   right is a design question nobody has asked.
   ⚠ **Its first two miss fixtures claimed a mutant class they did not
   catch** (judged FAIL, `findings/pass-20260828-065501-02-judge.md` fix 1):
   both offset x and z at once, so either extent test rejects the sample
   alone and deleting the other was invisible; and every hit fired down the
   exact centre, where the clamp is the identity. Fixed 2026-08-28 — one
   axis per row on the miss, an off-centre-but-inside row on the hit, and
   all four mutants (`ex := 0.0`, `ez := 0.0`, `ex := x - cxm`,
   `ez := z - czm`) now redden a named case in **both** `shoot.rs` and
   `chip.rs`. **The lesson generalises past this function**: a fixture that
   violates two conditions at once tests neither.
3. **The piece address on `EV_IMPACT`** — 27 bits against 4 spare pad bits,
   11 bytes. What still needs it: a **rim** (`plane_face` declines the
   ambiguous strip by design) and a **diagonal wall, 45° out**, since the
   built arm still snaps to the dominant horizontal axis.
4. **Spray paint is a deployable, not a decal**: a `limits.rs` cap, a
   `worldsave.rs` slot, build privilege, decay, moderation. Stencil or
   painted is the call to make first.
5. ✅ **All four of the shot walk's `loc` arms have an address case now**
   (2026-08-28, the third ranked fix of two consecutive judge reports).
   `chip.rs` covered `LOC_EDGE_XLO` alone, so **both** ternaries that name a
   loc — `cell_edges_stop_shot`'s and `cell_diags_block`'s — could have been
   replaced by their left branch with every gate in the repo green.
   `walled_world_at` builds one wall at any loc from any stance and
   `struck_address` unpacks the payload; four mutants (edge → always XLO,
   edge swapped, diag → always A, diag → always B) each redden their own
   case. The diagonals were the harder half and the reason is worth keeping:
   they anchor at the **cell centre**, so the stance is 4.24 m out (inside
   `BUILD_REACH_M`, barely), `shot_stop` runs both edge walks first so the
   approach must enter from a side with no edge piece, and `body_overlaps`
   makes A and B conflict, so they need two `World`s.
   ⚠ Still open, found while doing this: `cell_edges_stop_shot`'s
   `(bx+1, bz)` / `(bx, bz+1)` rows return the **neighbour's** address
   (`collide.rs:1641`), and nothing asserts that a shot stopping on a cell's
   high face names cell+1 rather than the cell it was sampled in.
6. ✅ **`deploy_stop`'s vertical band has a floor gate now** (2026-08-28,
   the judge's first ranked fix on `pass-…-03`). `y.clamp(bottom, bottom+h)`
   → `y.min(bottom+h)` ran the whole `sim-core` suite green, because every
   case in `shoot.rs` and `chip.rs` fired straight DOWN and the floor rail
   never binds from above — a box was an infinitely deep column and a bench
   on the storey above would eat a level shot on the ground floor.
   `shoot.rs`'s `a_shot_fired_up_stops_on_the_furnace_underside_not_below_it`
   fires up instead; the two answers are 1.4 m apart and `surf` cannot tell
   them apart, so the assertion is on y. Proven red under that mutant.

⚠ **Nobody has seen a decal**: no `ForwardDecal` renders under lavapipe at
any size, alpha or orientation. One boot on a real GPU settles it.


## 0wc · What world containers v0 still owes *(systems lane)*

1. **Nobody has opened one in the running game** — the prompt, the panel
   title, the drag out of a 30-slot grid, an emptied crate. Route: derive
   the anchor as `container_wire.rs:1307` does, set `dev_spawn` in
   `shard.toml` (`server/src/config.rs:361`), boot. §0p3 has the command.
2. **An emptied crate says nothing at a distance**, so a wasted trip is
   normal on a populated shard. Wants a lid state on the mesh
   (`render/props.rs` has one `crate_box`) or a shorter refill window.
3. **The guard has no loot tier of its own** — `guard.rs`'s
   `a_guard_pays_what_a_wolf_pays` holds it to a wolf's meat and fat. A
   tier wants a third species, and a third kind still falls through to
   the pig in `render/mobs.rs`, `sound/voice.rs` and `ui/death.rs`;
   `loot.toml` cannot carry it (`content/src/validate.rs:887` refuses
   zero hits).
4. **Nobody has fought a guard in the running game.** Same as 1.
5. **`inventory.rs:110`'s `slots_in` is the same defect one function
   over** — `CONT_BOX => BOX_SLOTS, _ => INV_SLOTS`, right only because a
   world container is `INV_SLOTS` wide. ⚠ The gate that looks like coverage
   is not: `a_world_crate_is_drawn_from_the_crate_store` reads `0..INV_SLOTS`,
   so a fifth ground kind of a different width draws the wrong slot count
   silently and that test stays green. Wants an explicit arm under
   `container_wire.rs:1359`'s `CONT_MAX` compile guard.


## 0pr · What predator v0 still owes *(systems lane)*

1. **Nobody has heard any of it.** `client/src/bin/soundbank.rs` dumps the
   bank to WAV; ears are the gate that has not run. Listen for the two
   cadences (`HOWL_PERIOD_S` 75 s, `GROWL_PERIOD_S` 2.5 s), the 0.5×
   night sense and the 16 predators — all four are arithmetic.
2. **A wolf pays no hide and no bone** — `content/mobs.toml` drops meat
   and fat only; refused in the roster slice because it drags recipes and
   `ui::icons::STEMS` in with it.
3. **Night still costs the player nothing.** Nocturnal senses made the
   hour a tactic, not the dark dangerous. The sourced follow-on is **not**
   more tuning of `night_spook_cm` — it is a night-only roster variant
   (Minecraft and Valheim gate *spawns* on darkness). The judge's gap 1
   wanted a warmth stat; `survival.rs:60` still records no temperature.
4. **The growl radius has no gate.** `sound/mod.rs:565` names §0pr as
   holding it: `CUES[Growl].radius_m` (14 m) must stay inside the wolf's
   night notice radius (15 m), and a `mobs.toml` edit reddens nothing.


## 0m · The pig is in — what the roster still owes *(systems lane)*

Research `reference/ANIMALS.md` §9.5. A wolf joined the roster since
this was written (predator v0), so it is no longer just the pig.

1. **A butchering VERB** — a tool-gated harvest on the body.
   `ui::interact::Verb` has no arm for it; the corpse bag
   (`mob::strike` → `backpack::stand_up`) is where its output goes.
2. **The combat-feel half of mob attack is minimal** — the victim sees
   hp drop and hears nothing species-specific (`sound/voice.rs` is
   presence, not reaction), so an aggro cue and a damage-direction tick
   are owed, and the charge costs the mob nothing to hold.
3. **The massing is boxy up close** — at 8 m the head barely separates
   from the body; `render/mobs.rs` is still a box massing.
4. **`MAX_MOBS = 64` has never met a playtest** — derived from the wire
   budget rather than felt, and the one number a player answers.
5. **Whether `ttk_melee` should widen** so a rock is worse than a
   crafted spear by more than one hit — `DECISIONS.md` §open, "tools as
   weapons".


## 0ctl · Four controls the player expects and the sim has no verb for *(systems lane)*

Bind each **in the commit that gives it a verb**: all four re-confirmed
unbuilt, and a key that does nothing is worse than an absent one.

1. **Reload (`R`).** No magazine, loaded state or reload verb anywhere;
   `ranged::draw`/`hitscan` spend from the inventory. Needs loaded-round
   state on the stack — `0dur`'s `ItemStack` question; `R` is repair.
2. **ADS / secondary (RMB).** No `BTN_SECONDARY`; `BTN_MASK` holds four
   bits, and RMB is already deploy-place, the build wheel and the half-stack
   grab. Needs a held-item modality answer before a bit (`PROTO_VER` bump).
3. **Flashlight (`F`).** The light exists now (torch light v0) and burns
   whenever the torch is in the hand — what `F` needs is the *toggle*, which
   is `§0tl` items 3 and 4: a lit bit and a fuel debit. `nothing_held_glows`
   turned out not to be in the way at all; it forbids a carried EMISSIVE,
   which is a different mechanism from a carried light.
⚠ **Both keys are already conditionally bound**: `ghost.rs:153/156` give `R`
and `F` the build ghost's level up/down while the wheel is up, and
`verbs.rs:245` is `R`'s repair arm otherwise. Bind over them knowingly.
4. **Voice (hold `V`).** No capture, codec, `KIND_*` or fan-out;
   `reference/VOICE.md` §9 settles both design questions.
Also open: the viewmodel sways in free look (`viewmodel.rs` reads `eye.yaw`
= `look.yaw + look.free_yaw`). §open "free look v0".


## 0sp2 · What the spill still cannot say *(systems lane)*

The first two need a wire field (`DECISIONS.md` §open):

- **A partial spill is still invisible.** Some fits, some does not, and
  the shortfall never leaves the sim — the wire carries what reached the
  hands and never what was paid, so `+3 × Wood` cannot say the other 7
  fell. The ring is `client-core/src/core.rs:900`, item index only.
- **The four give-backs say nothing at all** — demolish refund, pick-up,
  unbolt, craft cancel emit no payout event, spilled or not. Operator:
  those two together are what a wire field buys.
- **The merge ignores ownership** — a spill lands in whatever bag is
  nearest, including someone else's death bag
  (`sim-core/src/backpack.rs:51`). §open carries it.
- **Nobody has seen one.** Proven headless only, the "pack full — Wood
  dropped at your feet" line included: no frame in this repo has ever
  shown it.


## 0bl · The lattice's residuals: a seam, a memo, and a shot with no flanks *(client+sim lane)*

1. **A band-boundary wall bases on its canonical cell** and hangs one band over
   the lower plate — an arrow-sized slit. The lower column is the honest base;
   needs `collide` and the renderer together. Rare since the plate, not fixed.
2. **The flank costs 153 µs a tick and the shot walk now pays too; one memo
   takes most of both back.** `col_base_y` re-samples terrain per cell per
   candidate. `build::terrain_band` is pure in (seed, cell), so a direct-mapped
   memo is exact (`occupy::SlotCache`'s argument) and nothing memoizes it.
   **Measured 2026-08-25** on `examples/shot_cost --base`: 100 shooters
   volleying while stood on a slab is **1.25 → 3.07 ms** a tick against the
   same run with nothing built, because `cell_planes_stop_shot` taps
   `col_base_y` per sample over a plane-bearing column. Two callers now, same
   fix, still not urgent — the number is an aligned volley, not play.
3. ✅ **The shot walk reads the planes** (shot planes v0, 2026-08-25) —
   `cell_planes_stop_shot`, gated by `tests/shoot.rs`' floor block. What it
   still does not read is `ColMasks::solid`: see §0mk item 2.
4. **The half wall** — the reference's answer to the gap a half-storey plate
   offset leaves on upper floors; `build.rs` has eleven `SHAPE_*`, no half.
5. **The stepped foundation — and DO NOT widen the plate limits instead.**
   `reference/BUILDING.md` §7c.2 is a published, tested negative result on
   exactly that change: they tried a three-metre gradient on `foundation.steps`
   for our problem, it helped mountains, hurt flats and clipped their door
   blocks, and they reverted it. Ours is a catalogue row plus a shape code
   (§9 item 18), never a knob. Recorded here because it will keep suggesting
   itself.
6. **The diagonal wall's √2 root scale stretches its UVs** —
   `render/structures.rs:1272` turns the slab ±45° and scales `SQRT_2` along
   its length. Pinned so it cannot grow; `ART.md`'s business, not a defect.
7. **Operator:** whether `place` should refuse a piece whose cell a body stands
   in (`DECISIONS.md` §open "piece flanks v0"), and nobody has played the aimed
   freehand bit — which rides no golden either, closing which means scripting
   `sim-core/src/probe.rs` to build beside a built neighbour.


## 0ac · The catalogue's inserts, the soft face's look, and the diagonal price *(systems lane)*

1. **The inserts are unbuilt** — bars, glass, shutters, the garage door
   (`reference/BUILDING.md` §7b.4's second purchase, §9.13's remainder).
   Each is a deployable pass of its own; `content/building.toml` says so
   at both socket rows, and `place_deploy` still requires
   `SHAPE_DOORWAY`.
2. **The soft face has no visual identity.** `build::soft_side` prices
   the swing and labels the HUD prompt; nothing in
   `render/structures.rs` reads it, so the label is the only tell. Also
   owed: floor sides (needs a vertical attack direction) and the pairing
   with `RIPLIST.md` §2's per-material resistance.
3. **Triangles want a look and a price call**: a capture pass on a
   diagonal base in the booted game (the person is the visual gate); the
   wall-on-diagonal price — ~1.41× the length, today priced by the
   socket (`DECISIONS.md` §open "triangles v0", open for the operator,
   with the wheel at 11 wedges); and hard/soft's identity on tri halves.


## 0tt · The bench ladder's craft rebate, unbuilt *(systems lane)*

1. **The craft rebate** (`RIPLIST.md` §2 row 3) — 50% faster one bench
   up, 75% two up — is unblocked and untaken. `deploy::bench_near`
   answers a bool; it would have to answer "best rung in reach", and
   `craft::enqueue` would read it.
2. **The panel draws indents, not edges.** `ui/techtree.rs:49` says so in its
   own comment ("an indent (and one day a line)"); a line renderer between
   parent and child is cosmetic and waits for a real look at the screen.
3. **The operator has not seen it** — the tree panel, the two greybox
   benches, the tier badges. The visual gate is a person (`CLAUDE.md`);
   boot the game, stand at a bench, press `E`.


## 0tree · How deep the research tree goes, and the blueprint nobody can trade *(systems lane)*

1. **The tree's depth is still an unspoken pacing call.** It carries three
   edges now — `content/research.toml`: roadsign body behind medkit (:103),
   revolver and satchel behind gunpowder (:111, :116) — so the "one edge
   deep" reading is retired, and `DECISIONS.md` §open "research ladder v0"
   is stale in the same direction: it says revolver-behind-gunpowder is
   deliberately unauthored and `research.toml:116` authors it. What is open
   is how many more edges, over which bench tier now that workbench 2/3
   exist (§0tt). Do not invent one; fix the DECISIONS row when it is spoken.
2. **No blueprint ITEM**, so learning stays instant and personal and there
   is nothing to trade — the half that makes another player's progress
   interesting. Unbuilt, and it is a wire change
   (`crates/sim-core/src/research.rs` header records the omission).
3. **Nobody has seen the research/tech-tree panel work.** `ui/techtree.rs`
   and `render/panels/tech.rs` are gated headless only (`client/tests/ui.rs`
   §M); past `decode_event` nothing has been looked at. Same residual as
   §0tt item 3 — boot it, stand at a bench, press `E`.


## 0rs · Bodies are out of the raid storm *(systems lane)*

1. **Bodies are out of the storm.** `sim-core/tests/raid_storm.rs`'s own
   fixture sets the throwable's `damage` to 0 (`storm_combat`, line 212),
   deliberately — a blast that killed the players would measure a
   graveyard instead of a cap. `MAX_BACKPACKS` and the death/respawn ring
   are no longer un-driven — `tests/combat_storm.rs` reaches both at
   population (`§0cs`, 2026-08-30) — so what is left here is narrower and
   real: **no fixture takes a wall and the person behind it in one blast.**
   The shipped charge does 475 and `charge.rs:526` hurts
   bodies, so the arithmetic exists; what is missing is a bounded gate
   that drives it at the tick's command ceiling without the run ending in
   a few ticks.
2. **The fleet raids ITSELF, and that is a design gap rather than a bug.**
   Attacker and owner never share a plot (`peak_shared_plot == 1`, measured),
   so every raid in the tree is an attacker blasting the foundation it laid
   four steps earlier — `raid_shape.rs:33` says it outright: *"a self-raid is
   a poor game and a perfectly good `EV_STRUCT_HIT`."* It does not stop the
   raid, so it is not the explanation for anything; it means no fixture in
   this tree has ever modelled two parties. `§0pop`'s `index % 2` owner/
   attacker split is the knob that would.


## 0rc · The wire raid's two unmeasured differences *(systems lane)*

1. **The tree contradicts itself about dropped actions.**
   `server/tests/raid_shape.rs:73` and `server/src/botclient.rs:399` both
   say `push_action` silently drops the rest, so a lost step 4 leaves step
   5 throwing at nothing. `server/src/core.rs:722` says the opposite — a
   deferred action stays ringed — and the code agrees: `net.rs:2054` pops
   the action ring only through an open hand, and the stream reader at
   `net.rs:1519` sleeps on a full ring rather than dropping. One of the
   two is wrong; the harness's "this leans optimistic" argument rests on
   it, so settle it before quoting that argument again.
2. **The jitter buffer's held-item timing.** `Client::consume_input`
   (`server/src/client.rs:576`) executes one buffered frame per tick, so
   the frame carrying `charge_slot` need not be in force when the throw
   lands. Cannot be the whole story: 27 charges did arm.


## 0r · A blast is silent and cannot be stopped *(systems + audio lanes)*

Offence landed (`sim-core/charge.rs`, `tests/blast.rs`, `DEATH_BY_CHARGE`).
What it still cannot do:

1. **No detonation sound and no detonation visual.** The `Cue` enum
   (`client/src/sound/mod.rs:96`) has no blast voice, and there is no
   `EV_BLAST` — the client learns of a blast only through `EV_STRUCT_HIT`
   and `EV_HEALTH`, so a near-miss is silent. Audio lane; wants either a
   cue keyed off the existing events or an event of its own.
2. **No dud chance and no defuse verb.** Stated in the tree at
   `sim-core/src/charge.rs:38` — a fuse that has started always detonates.
   Each is its own verb.


## 0aa · Building rights: the roster's third customer is missing *(systems lane)*

1. **No `AutoTurret`, so the roster has two customers and not three.**
   `sim-core/roster.rs` exists because the reference has four; ours has
   the lock's auth/guest lists and the hearth's crew. `grep -rni turret
   crates/ content/` returns only that header comment and one
   `ARCH_TURRET` example in a protocol doc line.

⚠ Three doc comments in `sim-core/{deploy,claim}.rs` still cite "§0aa
   item 1" / "items 1–2" under the section's OLD numbering; renumber or
   re-point them if this item moves.


## 5d · The agent player: the trust ledger is minted and nobody reads it *(systems lane)*

`PLAYERS.md` has the spec — verb set, observation encoder, four walls. Wall 3
is built (`EV_TRUST` code 39, `World::log_trust`, six checks in
`crates/sim-core/tests/event_roles.rs`); the other three are not.

Remains, in order:
- **Nothing reads it.** `ShardCore`'s drain ends `_ => {}`
  (`crates/server/src/core.rs:2465`) and no file under `crates/server/` names
  `EV_TRUST`, so no shard-hour is recorded until a server lane sinks it.
- **A dropped row is gone** — it rides the 256-seat drop-newest ring
  (`MAX_EVENTS_PER_TICK`, `limits.rs:624`), and unlike every other event a
  resync cannot re-derive a fact about a moment.
- `TRUST_GIVE` waits on the give verb; there is still no player-to-player give.
- Then the verb table, wall 1's subset gate in the same commit, then an agent
  client that plays badly. Entry price and earnings are `ALPHA.md`.


## 4 · A payload swap is still not a compile error

Every `EV_*` code carries a role check against a real cause and
`NOT_COVERED` is empty (`sim-core/tests/event_roles.rs`), with the seat kept
for the next code. What remains is not tests:

1. **The stronger form: a payload-role table both the emit site and the
   check read, so an a/b swap is a *compile* error** rather than a gated
   value (`reference/FINDINGS.md` §1 end). The gate says so itself
   (`event_roles.rs:3486`) and calls it a different shape of work — bigger
   than one pass.

⚠ The ledger is **40** codes now, not the 32 this heading claims
(`event_roles.rs:3498`); fix the number when you next touch the line.


## 0q · The gaps nobody has claimed

`crates/`/wire work no single-surface lane may take.

1. **The UDP buffer's ops half.** `net::bind_udp` asks 8 MiB and records
   what it got; this box grants 4 MiB (`rmem_max`). Raising the sysctl on
   the public shard is an operator act.
2. **Shore barrels as a second destination class.** The road pays unevenly
   and the haven pad is the one place worth walking to; a second class on
   the shore would give the ring two ends. Nothing else in this file
   mentions it.
3. **The wipe.** Named by both judges, described nowhere; `wipe-now` is in
   no crate. Economy half (`ALPHA.md` A1→A3) and operator half
   (`CLAUDE.md`), so the loop's share is the mechanism, never the trigger.
   Needs scoping before it can be an item. (`ALPHA.md` §Admin lane cites it
   as "§0q item 2" — one of the two numbers is wrong.)
4. **What the soak still owes.** Ticks, AOI and bytes are all measured
   (`DECISIONS.md` §open, the 100-bot baseline). Missing: tick jitter as a
   **distribution** rather than a threshold crossing, and an **hour** — the
   run was 25 minutes, so slow leaks are not excluded. Contention now has
   its instruments (`sim-core/tests/raid_storm.rs`, and `botclient.rs`
   drives `bots::raid_step` over the wire); nobody has re-run the soak with
   them.

⚠ Delete the duplicate "you cannot stand ON anything" and "the soak has
never been run" items — both landed and both contradict items above them.


## 0zd · Doors and locks — the key lock's blocker died and nobody re-took it *(systems lane)*

Locks landed whole 2026-08-08/09 (`sim-core/lock.rs`, `reference/DOORS.md`,
`DECISIONS.md` §open "lock v1") — boxes, the guest/pickup tier, the keypad
panel. One thing survives, and it survives because a **refusal outlived its
reason**.

1. ⚠ **The key lock was refused for a blocker that is now paid.** The stated
   cost was that keys need per-item instance data `ItemStack` has no room for;
   `ItemStack` gained `pub cond: u16` on 2026-08-16 with durability v0
   (`sim-core/src/gather.rs:532`), so instance data now runs through the
   inventory, the wire and the save — the four costs `DOORS.md` §9.7 named.
   The exclusion therefore rests on one unreviewed reason (the reference
   abandoned it in Devblog 193), which may well still be the right answer —
   but it has not been re-taken since the blocker was paid, and the false
   premise is written in **three** places: here, `reference/DOORS.md` §9.7 and
   the `DECISIONS.md` 2026-08-08 row. Correct all three or re-take the call.
2. **The knob registry contradicts the tree.** `DECISIONS.md` still declares
   *"Client: `L` opens a keypad **HUD line**, not a panel"* while the client
   ships `render/hud.rs::pad_overlay`. `CLAUDE.md` calls `DECISIONS.md`
   authoritative on every knob, so the registry is the thing to fix.

Still not owed, and this half stands: door tiers past wood and metal are a
content row, not a mechanic.


## Wire, shard and persistence *(server lane)*


## 0fan · The event lane's fan-out — four arms filtered, nineteen to go *(server lane)*

1. **Decide `EVENT_RING_CAP`.** It was raised to **128** and this item said 64
   in three places until 2026-08-30 — read `limits.rs`, not this line. Measured
   81 of 128 under the worst fixture built (`snapshot_budget.rs`). Raise it
   again (322 B a slot per connection) or batch a tick's events (`PROTO_VER`
   work). Operator's call; `DECISIONS.md` §open (event-lane fan-out v0) has the
   trade. The other half of the same number: the ring is still **larger** than
   `MAX_PLAYERS` (100) now, so the 65-swinger case that motivated it is bought
   — what is not bought is item 4.
2. ⚠ **Operator, a game question**: should the OWNER hear their own door
   knocked from anywhere on the island? Nothing has an owner check to hang it
   on (`server/src/core.rs` EV_KNOCK arm, `hud.rs`).
3. **The deploy walk is unaimed, and it blocks `EV_DOOR`/`EV_OVEN`** — `core.rs`
   streams `deploys.entries()` whole. Order: (a) aim `EV_DEPLOY_PLACED` the way
   `EV_PIECE_PLACED` is, (b) aim the walk on the same anchor, (c) then those two
   become filterable. `server/tests/deploy_wire.rs` pins the current truth and
   its counts go red when the seam is aimed. Sizing, so nobody re-derives it:
   the band is **3.2% of the island's area** against `MAX_DEPLOYS` 1024
   (`findings/swing-fanout-20260824.md`).
4. **The storm is combat only, and the two fixtures do not compose.** Three
   arms of twenty-two; `raid_storm.rs` drives the other verbs at the command
   ceiling with nobody swinging. Measured 2026-08-30, and it is now a defect
   rather than a tidiness argument: firing peaks at 196/256 sim events, a
   hundred clients holding R at 100/256, **both green alone** — and both at
   once is 256/256 with 88 dropped and all 100 clients resynced, the
   self-amplifying case. The costs are simply additive and neither half bounds
   the other. Not gated, because every answer is an unspoken knob;
   `findings/note-20260830-two-storms-are-additive.md` has the table and a
   third candidate, now `DECISIONS.md` §open "refusal coalescing v0"
   (2026-08-30, judge fix 1 of pass -20): suppress a refusal that repeats
   verbatim, which removes traffic that carries no information rather than
   buying room for it. Three things there are unspoken — the window, whether
   a suppressed refusal is counted, and `pump_events` versus ~40 refusal
   sites. **A per-lane budget that each lane passes is not a budget when the
   cap is shared.**


## 0n1 · Class-S interest — the grid is still missing *(server lane)*

The radius filter landed 2026-08-18 (`crates/server/src/interest.rs`, gate
`crates/server/tests/piece_interest.rs`, `DECISIONS.md` §open "class-S
interest v0"). What remains, ranked.

1. **The grid.** No chunk version, no subscribe/unsubscribe, so no client
   can be told to forget a region — which is why removals stay broadcast
   and why a re-arm re-walks the in-range set instead of the difference.
   `NETCODE.md` §5/§7 proper, and it wants a wire change.
2. **Deploys and backpacks are unfiltered.** `server/src/core.rs:1975` says
   so outright ("the deploy walk is unaimed"), and the deployable walk still
   restarts on a removal (`deploy_sync_cursor = 0`, core.rs:2396); the
   backpack walk restarts on a loot or despawn (core.rs:2819).
   `reference/NETWORK.md` §9.2.1's amplifier, one store over.
3. `test_stream_in` (`NETCODE.md` §11) is still unbuilt — no `.rs` file in
   the tree mentions it. This gate counts records, not the client's
   per-frame apply/teardown budget, which is the other half.


## 0tx · The transport's three residuals *(server lane)*

Config and telemetry landed 2026-08-15 (`DECISIONS.md` §open "transport
truth v0"; gate `crates/server/tests/transport.rs`). Open, ranked:

1. **Nobody has run the A/B.** `cc = "bbr"` is selectable in `shard.toml`
   and untested against CUBIC on a real path. `net_congestion_events` is
   the reading. Wants a shard with real players, not loopback.
2. **The sysctl half of the socket buffer is ops and still owed.** The code
   asks 8 MiB; `net.core.rmem_max` on the public shard's box decides. The
   readback pair (`net_rcvbuf_asked` / `net_rcvbuf_bytes`) now says which,
   so check it before tuning anything else.
3. **No client-side telemetry.** All of the above is server-side. The HUD
   still has no loss/RTT source, and `crates/client/src/lib.rs:291` holds an
   `Arc<Connection>` it never asks anything — nothing under
   `crates/client/src/` calls `stats()` or `rtt()`.


## 0sp · The encoder is the tick's largest phase now *(server lane)*

`crates/server/src/bin/profile.rs` reports elapsed time and **must not become
a gate**; `valgrind --tool=callgrind` gives the per-function ranking.

1. **The encoder is now the largest phase** — ~0.43 ms of a 0.83 ms tick at
   100 clients in one AOI cell. Nothing else in the tick is close.
2. **`World::scatter_clear` still resolves cells cold per spawn pick**
   (`crates/sim-core/src/world.rs:1541` — three `terrain::scatter` calls per
   candidate, no `SlotCache`). It is **not** the crosshair's three-line fix:
   it is `&self`, and its 3×3 window *moves every candidate* along the spawn
   ring, so the cells are distinct and a memo only pays across repeated
   picks. Measure a respawn storm before threading `&mut self` through the
   picker.
3. The soak still owes tick jitter and real bytes (§0q item 4, `CLAUDE.md`
   wall 3's ⚠).


## 0y · Persistence — the three questions still open *(server lane)*

1. **A sleeper does not block movement.** Players never collided, so sleeping
   changed nothing; the question is unanswered rather than decided.
   Lootable-alive is item 1 of whatever comes after.
2. **The same-window rejoin.** A victim reconnecting in the very window that
   evicts them gets the store record fetched *before* the eviction save is
   filed — one window wide, the save ring's freshness class; the takeover
   hint already refuses to wake a condemned body (`server/core.rs:487`).
3. **Still no WAL, and the world file answered what a WAL would have
   forced**: a world load is an *origin*, not a command — the WAL header pins
   the origin hash beside the seed and the content hash, and replay starts
   there. `worldsave.rs`'s module header has the argument. Recorded because
   `§0ad2` item 4's set-time refusal leans on it.
4. **Still ungated:** the three-thread shutdown path end to end, and
   `KeySlot`'s id match (`server/net.rs:573`). Measured by hand only — a
   signal test is a clock test (`CLAUDE.md`): SIGTERM flushes and exits,
   SIGKILL leaves no `.tmp` and the next boot resumes off the last cadence
   save. Nothing in `crates/server/tests/` drives it.


## 0ad2 · What the admin lane still cannot do *(server lane)*

Six verbs plus `/bug` ship on the chat lane with no wire change, gated by
`server/tests/admin_wire.rs` (7) and `protocol::admin` (6). Open:

1. **A ban dies with the process.** `server/src/admin.rs:173`'s `Bans` is
   memory only (`net.rs:605` constructs it fresh). Persisting one wants its
   own file with its own format version — sharing the player store's header
   would wipe it on the next seed change.
2. **Nothing has typed a command against a live shard.** Every branch is
   gated headless; the socket half (`conn.close` with `REFUSE_ADMIN`,
   `net.rs:791`) has never been driven end to end, and the client has no
   dialog for it — `client/src/lib.rs:486` reads `refuse_text` at connect
   only.
3. **The anomaly log has no reader.** JSONL on purpose so `jq` is the
   reader, but nothing summarises a session, and the alpha gate's "zero
   silent failures" wants a verdict. (`ci/reports.py` is the `/bug` board,
   not this.)
4. **No `/who`, and no set-time** — the latter blocked by choice: day/night
   derives from the tick, so it wants the wire field §0y4 did not spend.
   ⚠ `/tp <id>` exists and the two-arg form is a decided refusal
   (`protocol/src/admin.rs`), so drop it from this list.


## 4b · The domain gate's one file-local residual

`SOURCES` (`protocol/src/event.rs:4351`) reads every `sim-core` module both
ways and every enumeration width is classified. One residual:

1. **`death_causes_are_a_closed_ledger` still scrapes `world.rs` alone.**
   `sim-core/tests/event_roles.rs:3704` opens with
   `include_str!("../src/world.rs")`, so its *contiguity* claim is
   file-local. Narrow, since the protocol gate catches a stray value
   crate-wide. `sim-core/tests/domain_ledger.rs` now applies the same
   scrape to three other families and is the pattern to follow.

⚠ This label collides with the world-lane §4b further down the file; give one
of the two a distinct label so a crate citation can name which.


## 0pop · The inhabitants nobody has run for longer than a test *(server lane)*

1. **Nobody has run one for longer than a test.** `DEFAULT_SHIFT_SECS = 300`
   (`crates/server/src/population.rs:47`) and `tests/population.rs`
   exercises ~0.2 s of it, so re-manning, the `RECONNECT_BACKOFF_MS` backoff
   and the shift report are gated only by construction. Cheapest next step:
   set `population = 8` in a real `shard.toml` (it is still commented out at
   `shard.toml.example:298`), run the shard, read the population line.
2. **Nobody has checked what an inhabitant can afford.** The shipped kit is
   a rock and a torch (`content/balance.toml` `[[spawn_kit]]`), while the
   raid rows a post plays are a fixture — the satchel is granted directly in
   `crates/server/tests/bot_smoke.rs`, never crafted. Judge -18 §B.2 is the
   live half of this.
3. Two proposed defaults stay open in `DECISIONS.md` §open ("shard
   population v0"): the 300 s shift and the 2 s backoff, plus what N an
   alpha shard should actually run and whether the owner/attacker split
   should stay `index % 2`.


## 5b · The wire still accepts two refusal reasons the sim can never mean *(server lane)*

⚠ **This section said CLOSED and covered half the problem.** The decode
narrowing landed for the bag `why` and the consume `reason` (`REFUSE_C_MAX`,
derived, `protocol/src/event.rs:365`). Two more domains are unbounded end to
end, and the tree names this section as their record: `sim-core/src/craft.rs:78`
says in its own source that the craft-refused subtype *"writes a full byte and
the wire bounds nothing, which `NOW.md` §5b already carries as the decode-side
gap."*

1. **Craft-refused.** Six reasons (`craft.rs` `REFUSE_RECIPE`..`REFUSE_BLUEPRINT`,
   1..=5) and deliberately **no `REFUSE_C_MAX`** — the name is taken by
   `survival.rs`'s consume refusals, whose prefix the domain gate scans
   crate-wide, so the obvious constant would collide. That is a naming problem
   wearing a wire problem's clothes; pick a name the scanner distinguishes.
2. **Deploy-refused.** `deploy.rs:314-318` declares `REFUSE_D_KIND`..
   `REFUSE_D_REACH` and there is **no `REFUSE_D_MAX` anywhere in the tree**.
3. Neither appears in `event.rs`'s `DOMAINS` table, so
   `every_domain_fits_its_wire_field` cannot see them and the encode site
   bounds nothing.

No `PROTO_VER` turn is owed — this is the narrowing rule at `PROTO_VER`
(`protocol/src/lib.rs`), the same judgement `5c` already made.


## The frame, the screens and the client's own hot path *(client lane)*


## 0fill · The darks, second half: the transfer *(client lane)*

The hemisphere fill landed (`render/fill.rs`, `tests/fill.rs`); the transfer
did not. Cast shadow on *open* ground is an up-facing surface and no
hemisphere darkens it, so the measured p10 is untouched (79.9 against
`ART.md` §3's 49).

- The rig's floor arithmetic is written in the wrong space. `rig.rs` sets
  `fill = 0.30 × sun_on_flat` for rule 3's "shaded ≥ 0.30 of lit", but the
  delivered *linear* ratio is 0.229 — under the floor it aims at — while the
  judge measures 0.725 in *display* luma. Rule 3 is a pixel ratio and the
  constant is an illuminance one; both readings cannot be acted on at once.
- So the lever is the tone curve, not the fill: `Tonemapping::TonyMcMapface`
  (`rig.rs:212`) plus `Exposure { ev100: 14.2 }` (:206, 0.8 stop off
  `SUNLIGHT`) is what puts 0.229 linear at 0.725 display.
- **Do not do this blind.** It is the coupled set (`CLAUDE.md`: three
  parallel passes 60→66, one sequential owner → 26) and the last correction
  overshot. One owner, one iteration, with the frame open.
⚠ **And the next pass to capture must know the tonal baseline moved.** Every
`-visual.md` in `findings/` predates `rig::DayPin`, so it was shot at a
24–27° sun against the pinned noon a capture run now takes. Its luma, sky and
shadow numbers are **not comparable** to the next report's — do not read a
brightness delta there as the effect of a render change.

- Blocked on a *capability*, not priority: a pass that can capture should
  take it before anything below. §0gp item 1 (8.0% mean luma) and item 3b
  (`reflectance: 0.18` → F0 0.52%) are debts against this same owner.


## 0gc · A blade shaded exactly like the dirt it stood in — LANDED *(client lane)*

**Landed 2026-08-25.** `Soup::tri_ramp` takes the blend as a function of the
vertex; `blade()` ramps 1.0 at the root (the ground's normal, so `ART.md`
rule 2 keeps the blade bedded) to `BLADE_TIP_BLEND = 0.75` at the tip. `tri`
delegates to it, so none of its twenty-odd other call sites moved. Gate:
`tests/contact.rs::a_blade_separates_from_the_ground_it_grows_out_of`, red on
the shipped value, where it prints what it was — **tip normal y = 0.9978**,
the ground's normal to three decimals. Knob: `DECISIONS.md` §open, clutter
contact v0.

⚠ **This item carried TWO false mechanisms and both are dead.** The winding
claim went to `a_blades_two_triangles_do_not_wind_opposite_ways` (dot > 0.99
over a 128-case sweep). The `double_sided` claim — that Bevy negates the
shading normal on a back-facing fragment — was checked against Bevy 0.18.1 on
2026-08-25 and is **also false**: `pbr_functions.wgsl:130-134` guards that
negation with `#ifndef VERTEX_TANGENTS`, `mesh.rs:2410` defines
`VERTEX_TANGENTS` whenever the layout carries `ATTRIBUTE_TANGENT`, and
`Soup::mesh` calls `generate_tangents()` on every clutter tile. **No blade is
ever flipped.** Do not turn `double_sided` off — it changes nothing here and
would black out the real back faces.

What is left is the one number: **0.75 is invented and nobody has judged it**,
against `ART.md` §5's "blades catch a rim of sun at their tips".


## 0gp · The ground splat's residuals: a projection, a specular, and five prop maps *(client lane)*

1. **Still planar XZ, not biplanar** — a vertical face stretches;
   `assets/shaders/ground_splat.wgsl` projects no other way (`RENDER.md` R4).
2. ~~**`reflectance: 0.18` → F0 = 0.52%**~~ **LANDED 2026-08-25, and it was
   not one constant**: *every* material in the client was authored 8–70× under
   physical, because Bevy's `reflectance` is a remap (`F0 = 0.16 × r²`) whose
   default 0.5 already IS the dielectric 4%. One owner now,
   `render/fresnel.rs`; `tests/fresnel.rs` reads every prop material back out of
   the asset store and fails anything outside 1.5–6% F0 (red on the shipped
   `bark` 0.08 = 0.10%). The ordering this item insisted on held — the
   per-texel field landed first, so it is turned up over a field and not a
   scalar. ⚠ **The −0.4% roughness null result has NOT been re-measured** with
   energy in the lobe. `DECISIONS.md` §open "specular v0".
2b. ~~The four ground `*_ao.jpg` are read by nothing~~ **LANDED 2026-08-25** —
   bindings 114–117, blended by the same `bw` as colour, normal and roughness,
   folded into `diffuse_occlusion` with `min` per `ART.md` §4 (never a
   multiply: two occlusion terms of one scale double-darken). Diffuse only.
   The binding gate now scrapes BOTH the WGSL and the Rust struct — it held the
   shader against a hand-kept list that *claimed* to be the struct and never
   read it. ⚠ Nothing in this repo compiles the WGSL; a syntax error there is
   green in CI and dead at boot. **Booted 2026-08-26 under lavapipe: it draws.**
3. **`ground_detail.jpg` is loaded by nothing** — `textures::GROUND_DETAIL` has
   no load site; the shader derives the field from `grass_albedo`. Deleting it
   is a separate call: a pre-baked field is what a cheaper LOD would want.
4. **Operator:** granite passed beach sand and the minimap's `ROCK` did not
   follow; fixing it departs from a `mapraw.jpg` reading (`DECISIONS.md` §open
   "minimap palette v0"; `client/tests/map_palette.rs` pins it by name).
5. **The five PROP roughness maps are unread and `render/props.rs`'s reason is
   false** — Bevy multiplies `metallic` (default 0.0) by the map's B channel.
   It needs a LEVEL call: the map whole loses the authored `rock 0.88` /
   `ore_stone 0.80` split; mean-placing wants 1.44 and Bevy clamps at 1.0.


## 0gi · What the island still cannot show: no occluder at blade scale *(client lane)*

Items 1–3 are struck and gated (`sim-core/tests/relief.rs`,
`client/tests/ground_where_the_green_goes.rs`, `client/tests/daynight.rs`).

4. **An occluder at blade scale is missing.** SSAO is enabled and this item
   was twice wrong about it: it is at `rig.rs:284`, at Medium,
   and "no SSAO anywhere" is stale. The paint read is `clutter.rs`'s NORMALS
   (§0gc). What nothing pays is `ART.md` rule 2 for the tile — the clutter
   mesh carries `NotShadowCaster` (`clutter.rs:504`) and a blade's dark base
   darkens the blade, never the ground under it.
5. **Litter still wins every mix, and this item's numbers are stale.** After
   §0gp's albedo re-place, recomputing off `terrain_mesh::GROUND_ALBEDO`
   gives litter **2.49×** grass's value (not 3.2×) and grass must hold
   **≥78.0%** of a grass/litter blend to read green-dominant (not 82.1%).
   The gate (`ground_where_the_green_goes.rs`
   `grass_must_hold_most_of_a_mix_for_the_ground_to_read_green`) asserts
   only `> 0.66` and `> 2.0×`, so it stayed green through the drift. The
   mosaic is not itself a defect; the boundary still never reads as grass.


## 0w · The props' remaining gaps — darks, density, unread roughness *(client lane)*

1. **The p10 gap, still the top visual one** — 71.0 against a reference 41.0
   (`RENDER.md` §0). The hemisphere fill landed (`render/fill.rs`,
   `tests/fill.rs`) and bought direction, not the p10; the transfer half is
   what is left (`RENDER.md` §5 item 6). One owner in the coupled set.
2. **Trees are small and sparse in the midground** — an empty green plain
   between near clutter and far ridge. `terrain::scatter`'s density and the
   conifer's scale, not a material; the same ceiling §0t item 2 prices.
3. **The dirt skirt is nobody's.** `props::SINK_M` (0.06 m) sinks every prop
   and `tests/greybox.rs` evaluates "nothing floats"; crowding where a
   boulder meets turf is still missing (`ART.md` rule 2).
4. **The far mesh speckles.** Grazing-angle aliasing on the 8 m LOD;
   `textures.rs` pins `anisotropy_clamp: 4` for a browser reason that did not
   survive the port (`ART.md` §7) — a proposal, not an edit.
5. **Roughness maps unread — ten now** (`assets/textures/*_rough.jpg`).
   Blocked on an ORM packing step: `metallic_roughness_texture` is
   glTF-packed and its B channel is metallic (`render/props.rs:1090`).


## 0out · The horizon has trees — what the outer ring owes *(client lane)*

Landed 2026-08-25. `props::OUTER_RADIUS = 5` streams an 11×11 chunk ring of
TREE-ONLY hulls past `NEAR_RADIUS`, one entity each (`spawn_outer_tree`) — no
`Topple`, no stump, no canopy, no `VisibilityRange`. ~1,260 trees at 105 tris
= ~132 k against `DESIGN.md` §9's 1.5 M. The radius it replaces was sized when
a tree cost 5,900 triangles and never re-derived after `impostor_of` made it
105. Planted on `terrain_mesh::far_ground_y`, not `slot.y`: the ground drawn
out there is the 8 m far mesh minus `FAR_DROP`, measured **0.630 m** off the
heightfield at worst. Gates: `tests/outer_ring.rs` (4); one mutant caught a
worthless assertion in the first draft.

1. **The hull is untextured** — it wears `foliage` (white, vertex-coloured, no
   map), so the midground is flat green shapes and this ring multiplied them by
   four. `WANTED.md` §9.5's leaf texture is the cheapest fix and serves the
   bush too. **Highest-value item here.**
2. **The harvest sweep got denser and that was a named cost.**
   `harvest_changed` measured 1,500 props × a full 16,384 set at 2.34 ms and
   warned that a denser ring is the case that worsens. Outer hulls carry
   `Fellable` for correctness, so the count roughly doubles on frames where the
   harvested set moves. The real fix is that `HarvestedSet::contains` is a
   linear scan. Unmeasured on a GPU.
3. **Only trees.** Boulders and barrels still stop at `NEAR_RADIUS` — a
   sub-pixel lump costs an entity and changes no silhouette.


## 0t · the forest — what it still owes *(client lane)*

1. **The broadleaf has never been LOOKED at.** `SPECIES` is two rows, pool 6;
   every check on it is arithmetic. Boot it and look — likely wrong are
   `children`/`angle[1]` and leaf `count`/`size`; `PLANTS.md` §3.1 has
   ez-tree's 15 presets to take real numbers from. A species is a row.
2. **The density ceiling** — one occupant per 8 m `CELL_SIZE` cell.
   `PLANTS.md` §3.2 prices the three ways up; all sim-core, none cheap, the
   cheapest (`CELL_SIZE` 8 → 4) quadruples live `SlotLives` rows against
   `TERRAIN.md` §6's budget. Not a rendering change.
3. **The billboard LOD is optional now, not owed** — `impostor_of`'s 105-tri
   hull took the p90 ring 1.94 M → 510 k, under `DESIGN.md` §9's 1.5 M.
   `TERRAIN.md` §4's octahedral billboard is the cheaper end, still unbuilt.
4. **`aWind`** — `StandardMaterial` cannot read a custom attribute, so wind
   needs the custom material `RENDER.md` lists. Gets LOD1 for free.
5. **Sub-canopy empty, shrub layer one blob** (`Occupant::Bush`, `PLANTS.md`
   §2): ez-tree's `bush_*` presets and a small tree at 40 % are new
   `Occupant` variants plus scatter rows.
6. **The needle card is generated** (`tree::needle_image`); `WANTED.md` §9.5
   is the swap, the highest-value texture on that page.


## 0a · The clutter ring still ends on a line *(client lane)*

`render/clutter.rs` has no distance term: `CLUTTER_RING = 2` over
`CLUTTER_TILE_M = 16.0` puts a hard edge at ~32–45 m. Two findings stand:

1. **The fade's recipe**, already proven at the other boundary
   (`sim-core/terrain.rs::swept_here` cites this item): thin
   stochastically by instance hash so the same elements survive at a given
   range, then scale the survivors to zero. Whether the edge reads at all
   at that distance is a question for a person with the game booted, not
   for a guess.
2. **Beach skirts are thin because of the scatter table, not the skirt
   path** — ~0.22 prop centres a tile on the coast against ~0.95 inland.
   ⚠ Neither ratio is in the tree; both are browser-era measurements and
   want re-measuring against `terrain::scatter` before they are acted on.


## 0y · The sea is a volume — what it still cannot do *(client lane)*

1. **The last hard edge needs the depth prepass.** The alpha ramp is a
   *vertex* quantity off `terrain::height`, so it rings against a
   boulder, a foundation or a player in the shallows. Sample the prepass
   in the fragment, fade alpha as scene depth nears the water's own.
   Needs an `ExtendedMaterial` and WGSL (`RENDER.md` §8); both already
   exist in the tree (`assets/shaders/ground_splat.wgsl`), and **the third
   input exists too** — SSAO already puts a `DepthPrepass` on the camera, so
   the fragment has something to sample.
2. **One sea state, no weather.** A storm is `WAVES` scaled by a scalar
   the sim would have to publish — wire, not renderer.
3. **Nothing reflects.** `reference/WATER.md` §5/§6 first: reflections
   are the expensive half and the payoff is the sky.
4. **Underwater is audio-only.** A colour grade under the surface is a
   second owner of the frame's haze; it wants the lighting owner.
5. **The submerged duck is not a filter** — rodio gives gain, rate and
   panning; a real low-pass needs a DSP node.
6. **`Splash` is the only producer of the waterline** — no stroke, no
   wake, no interactive deformation.


## 1 · The native pivot — the one visual gap left of it

R0–R6 and R8 all landed and the browser client is deleted. What survives:

1. **Cloud form.** The deck reads stratus where `ART.md` §4 asks for
   cumulus; the p90 gap is 25 luma. `RENDER.md` §8 ranks it second, behind
   the gate-asserts item, and no other section in this file owns it.

⚠ Drop the rest of this section's list. The hemisphere fill landed
2026-08-15 (`render/fill.rs`, `client/tests/fill.rs`) and the four-way
splat landed the same day (`render/ground_splat.rs`,
`client/tests/ground_splat.rs` — four identities, four photographs), so
neither is a gap. The depot is published; the republish that is still owed
is §0win's, not this item's.


## 0chr · The clips the wire cannot yet ask for *(client lane)*

1. **The clips outrun the states that would play them.** `interp::RemoteState`
   carries only id/pos/yaw/pitch/live/sleeping/dead, so `Jump_Loop`,
   `Swim_Fwd_Loop` and the crouch pair sit unplayable in `stumpy.glb` — each
   needs a fact on the wire. Crouch is an input bit the sim ignores
   (`render/input.rs:289`) and never reaches a snapshot.
2. **The gather swing is `Sword_Attack`** (operator, 2026-08-17): no asset is
   owed, and the blocker is item 1.
3. **The item is not parented to the hand** — `render/viewmodel.rs:572` says so
   in its own comment; `ViewArms::hand` holds the bone, and the grip offset and
   tilt need re-deriving against the arm's frame, which is judged by looking.
4. **No render layer**, so the arms and the held item can clip into a wall: a
   second camera would duplicate the exposure/tonemap/atmosphere owner.
5. **The head's pitch is clamped, not distributed.** `ANIM_HEAD_PITCH_MAX`
   is 0.9 rad and `anim.rs:811` says the remainder is dropped rather than
   spread — "distributing it across the spine is the follow-up this constant
   exists to make obvious." A steep look up or down bends nothing below
   the neck.
6. **The hand reads large and the fingers splay** — 24 joints, no finger bones,
   so a re-import or a sculpted grip is the only lever.
7. **Unlooked-at**: `Death01` on a real body, the collapsed off arm, the
   sleeper tint. ⚠ "No frame has a body in it" is stale — `render/capture.rs`'s
   scene pass shoots `7-player.png`, staged by `ci/scene.sh`.


## 0hand · Four items still draw the generic stand-in *(client lane)*

1. **Metal hatchet/pickaxe/spear** — no asset (`assets/models/WANTED.md` §5.6,
   while `content/items.toml` already ships `item.hatchet_metal` and
   `item.pickaxe_metal`), and reusing the stone glb would need a second
   material to not lie about the head.
2. **Fire pit** — `assets/models/deploy/fire.glb` bakes a LIT emissive, so a
   carried unlit one would glow and `held_assets.rs::nothing_held_glows`
   refuses it. Needs an unlit variant or a generated `heldgen` row.
3. **Resources, ammo, bandage, lock** — no models; the stand-in covers them.
   `ui::hold::HELD_MODELS` is 14 rows and none of them is these.
4. **The swing still pitches the item and not the arm**, so a mid-swing frame
   shows the fist behind the arc. Same fix as §0chr: parent the item to
   `ViewArms::hand` (`render/viewmodel.rs:572` records why it has not been),
   retune the grip against the arm's frame, then look at it.


## 0dur · Durability: the words, the wearers, the bench *(client lane)*

1. The pip is drawn in all four cells that hold a stack, but **the detail
   pane still says nothing in words** — `render/panels/craft.rs::build_detail`
   never reads `cond`.
2. **Weapons and armour do not wear.** `sim-core/src/combat.rs` says so in as
   many words, `condition_loss` rows exist only in `content/gatherables.toml`,
   and there is no `sim-core/src/armor.rs`. `reference/DURABILITY.md` §5 left
   both unsourced (per shot / when hit), so this is a research row, not a
   build item, and wear-on-swing-at-players is a mechanism question (`tools as
   weapons`, `DECISIONS.md` §open).
3. **Repair is v1 by decision** (Q3: re-craft is the repair). When a bench
   lands it is `Station::Workbench1..3` (`content/src/schema.rs`) plus a
   blueprint check, never a new deployable, and `DURABILITY.md` §3's 0.20
   ratio stays DISPUTED until someone checks it against the in-game price.


## 0ps · Pieces: staged damage, the missing shapes, the repeated wall *(client lane)*

1. **Damage bands have never been staged**: one row of one material, hit a
   known number of times, photographed at each band. ⚠ §0mk — no decal
   renders under lavapipe, so a headless run cannot check marked surfaces.
2. **11 shapes against the reference's 20** (`BUILDING.md` §7b.1):
   `sim-core/src/build.rs` declares `SHAPE_FOUNDATION`..`SHAPE_TRI_ROOF` only
   — no half/low wall, floor frame, steps, ramp, 3 of 4 stairs. Rule 6 is
   silhouette before surface, so this outranks more material work.
3. **A base is a hundred identical walls at one rotation** (rule 7).
   `render/structures.rs` sets `uv_transform` from the tier's scale alone; the
   fix is a pool of per-tier variants (offset + tint) by address hash.
4. **Trim** — lashings, plank seams, a capstone rim; `shape_parts` is the
   place, but price the entity count first at `MAX_PIECES` 8192.
5. **Deployables got the wire fix, no damage visual** — the deploy material
   takes no `hurt` term where a piece does — and nothing shows which face was
   struck.
6. **Roughness maps still unwired**: scalar `perceptual_roughness` only. An
   ORM packing step would serve terrain+props+pieces at once.


## 0u · Stairs are a plate, and a lock cannot be aimed at a door *(client lane)*

1. **Stairs are still a flat pitched slab** in both the ghost and the standing
   piece — a ramp drawn as a plate, with no steps in it. Shared between the
   two, so at least they agree, and `sim-core/tests/base_lattice.rs` holds the
   tread a player walks to the ramp the sim walks. This is the SHAPE being
   undetailed, not `§0ps` item 2's missing stair variants.
2. **A lock aimed at a DOOR is unreachable.** `ui::place::deploy_target`
   special-cases `PLACE_DOORWAY` only, so `PLACE_DOOR` — the code lock's
   placement class (`content/deployables.toml`) — falls through to
   `SHAPE_FOUNDATION` at level 0 and targets the plane. On a box the `L`
   verb works. Noted at the call site, not built.


## 0x · The client makes sound — what it cannot yet hear *(client lane)*

1. **Nobody has heard it and nothing scores it** — `ART.md` has no audio
   section and this box has no device. `cargo run -p client --bin soundbank
   -- <dir>` writes every cue to WAV; sourcing is `assets/sound/WANTED.md`.
2. **The score is programmer art.** `synth::score` generates the nine
   `music::PIECES`; swapping in recorded pieces is one function
   (`synth::render`'s music arm). Two bumps we cannot take: weapon equipped,
   projectile near-miss.
3. **The `--capture` run is still by hand** and is the only proof most audio
   systems execute. `tests/music.rs` is the cheaper shape — any audio system
   with no world in its arguments could be gated that way.
4. **Two cues have no producer:** `ImpactWood`/`ImpactMetal` need to know
   WHAT was hit, and `UiClick` appears only as the mixer's placeholder
   `Request` — it wants a hook in the per-screen click handlers.
5. **No occlusion**; the prerequisite is a geometry query, and the correct
   one is the sim's (`collide.rs`), not a raycast against render meshes.
6. **Crickets** are a content-free companion pass — a night-gated `Cue`, the
   bird layer's shape with the predicate inverted (`render/audio.rs:672`).


## 0x · The native client — the feature trim and the dropped anchors *(client lane)*

1. **Trim Bevy's default features — with a verified build, not a guess.**
   `crates/client/Cargo.toml` still takes bevy with defaults on. Unused by
   grep: `bevy_gilrs` (no `Gamepad` anywhere — the one real system-dep win,
   `libudev`) and `vorbis` (the bank is WAV we generate). Load-bearing:
   `bevy_audio`, `bevy_gltf`/`bevy_animation`, x11 and wayland. Attempted
   2026-08-06 and backed out on disk, not code — and a green compile is not
   evidence: Bevy answers a missing decoder with a white fallback. Wants
   headroom and a `--capture` run someone looks at.
2. **World-space anchors are still dropped.** The HUD half landed
   (`hud::readout` pins the struct-hit fraction and the charge clock under
   the toast); the wall's own number at the wall itself and a clock on the
   charge mesh are not built, `charge_deploy` stays unread, and `stock_addr`
   is set in `client-core` and read nowhere under `crates/client/src`, so
   nothing says WHICH hearth. None is blocked.


## 0z · The Bevy-draws rule's missing gate *(client lane)*

1. **R-G4 is still the missing half of the Bevy-draws rule.** Placement has a
   gate; the no-gameplay-state-in-the-ECS rule has none. Its answer is the
   renderer-attached/detached state-hash equality (`RENDER.md` §5, line 889),
   and nothing under `crates/client/tests/` compares a state hash.
2. **Nothing photographs the wait.** A capture run exercises it and
   `render/capture.rs::PLACE_FRAMES` (300) bounds it; *seeing* it is §0p2
   item 3's viewer, which is also unbuilt.


## 0v · Players are people — what the rig still cannot say *(client lane)*

1. **Crouch, jump and swim are wired to nothing.** The clips are in the
   file; the snapshot carries no grounded bit on a remote body and no
   crouch bit, so `BodyAnim` cannot see them — `render/audio.rs::
   remote_steps` names the same gap. A protocol change (wall 6: version
   bump + regenerated goldens in one commit), not a client one.
2. **Nobody holds anything.** The viewmodel is first-person only; a
   remote mannequin has empty hands (`render/bodies.rs` spawns no held
   mesh). The rig has hand joints, so this is an attachment to a named
   joint rather than new art.
3. **Root motion is ignored.** The `_RM` variants are unreferenced in
   `crates/client/src`, so feet slide at speeds between the clips'
   authored ones — the fix is scaling playback rate to speed, a knob
   nobody has measured.
4. **A plain worn-steel albedo is the missing texture.** The axe head
   carries no map; the only metal in `assets/` is ribbed corrugated
   sheet (`render/viewmodel.rs`, `assets/textures/MANIFEST.md`).


## 0p2 · What the UI still owes *(client lane)*

1. **Rotate is still not a verb** — and the piece HAS a facing now
   (`PieceRec::facing`, hard/soft v0), so the asymmetry it waited on exists.
   `ACTION_SUB_BITS` is 5 and `ACT_MAX` is 18: the lane holds it.
2. **The hammer wheel's centre readout names the verb, not the target or the
   upgrade's cost** (`panels/wheel.rs`, `hammer::label`/`blurb`). Filling it
   wants `verbs::Near` at draw time, which `panels::rebuild` does not take.
3. **Nothing here can photograph a panel.** `render/panels/` (3,540 lines) is
   unreachable from `--capture`. Wanted: a **viewer, not a gate** — open each
   panel against a stocked fixture, write a PNG per screen, assert nothing.
4. **Fourteen distinct font sizes is not a scale** (`font`/`font_bold` sites
   in `render/`). Collapsing to five may not be done blind: they were
   budgeted against 720p and the first cut clipped a column at both ends.
5. **Surveyed and refused, do not re-survey:** `bevy_hui`, `bevy_lunex`,
   `bevy_feathers` (~5,400 lines of screens into a data-driven plugin) and
   the freegameui.net MCP (403s here, bypasses `bake_icons.py` and
   `tests/ui.rs` §G, pre-coloured kits fight tint-at-draw).


## 0w · The native menus — the rail and the untested gesture *(client lane)*

1. **The rail is not the reference's, and one wire field would fix it.**
   `EventMsg::Catalog` ships display names only, so a category rail by item
   class is not computable client-side (`ui/craft.rs:14-28`). A class byte
   per item, a `PROTO_VER` bump and regenerated goldens in the same commit
   (wall 6) buys the frame's real rail. Today's buckets are honest but they
   are not that.
2. **The drag is gated as arithmetic, not as a gesture.** `tests/ui.rs` §B
   holds the split arithmetic (`a_half_drag_sends_half`); press → ghost →
   release → send against a live shard is verified by inspection only.


## 0v · The menu flow — the served list and the untested hangup *(client lane)*

1. **Nothing re-checks that the SERVED shard document matches `shards.toml`.**
   `ci/shardlist.py --self-test` is a pure generator by design — no network,
   which is what lets it run in `ci/gates.sh` — so the diff between what it
   produces and what `GET /api/launcher/servers/gates` actually returns is a
   command somebody runs. The three days the list was served and dark
   (2026-08-20 → 08-23) are what that costs; `ops/certbot-deploy-hook.sh`
   now refuses a chain that does not cover the published name, which closes
   the certificate half only.
2. **Ungated, by hand only:** the end-to-end kill-the-shard-mid-play run
   behind `Screen::Disconnected`. Nothing under `crates/client/tests/`
   enters that state except `report_key.rs`'s key check.


## 0pw · Skinned meshes still specialize on arrival *(client lane)*

`render/prewarm.rs` warms every `StandardMaterial` off `AssetEvent::Added`.
What it does not warm is named in the module (lines 52–57):

1. **Skinned meshes are a different pipeline key** — a body's skin is a
   `SkinnedMesh` component, so the first remote player to walk into view still
   specializes on arrival, and the native symptom is a pop, not a hitch.
2. **The measure has no gate.** `PipelineCache::pipelines()` is public but
   lives in the render world and needs a GPU, so the count stays unasserted;
   `crates/client/tests/prewarm.rs` gates only what reaches the ECS.


## 0pf · The client's CPU frame — four measured leftovers *(client lane)*

1. **`ground_slope`'s four taps are ~80% of what a tile now spends** and the
   stencil is not takeable — it moves every splat byte, so it is a design
   change with a golden behind it, not an optimisation.
2. **`water::animate` clones ~677 KiB into the render world every frame** —
   `Assets::get_mut` deep-clones a `MAIN_WORLD` mesh on modification. Measured
   by `crates/client/examples/frame_cost.rs`: 7,921 vertices / 676.6 KiB,
   stream+animate 0.69 ms on a still frame. The fix is the vertex shader
   `render/water.rs` §57 names, and no `.wgsl` exists in the tree yet. Nothing
   on this box runs WGSL, so do it AFTER someone can boot on a GPU.
3. **Per-frame leftovers, measured and small** (under 50 µs together):
   `verbs::resolve` scans the piece mirror twice a frame and wants the 3×3
   `ColIndex` neighbourhood; `bodies::stream`/`mobs::stream` re-find each
   interpolator slot by linear scan after `ids()` knew it; `audio::fell`
   fetches a `GlobalTransform` to test `is_changed()`; `hud::update` rebuilds
   its strings; the ring streamers probe a full map every frame.
4. **The sea's tangent `w` is `-1` (`water.rs:947`), the ground's is `+1`
   (`terrain_mesh.rs:711`)**, same planar XZ set. One flips the ripple map's
   green channel — boot the game and look, do not guess.


## 0u · the frame budgets are browser numbers and nobody has re-derived them

`DESIGN.md` §9's budgets were set for a WebGL page and three no longer
describe what constrains us. The docs now say so; the measurement is still
owed and no gate or knob has moved.

1. **< 300 draw calls / < 1.5 M tris are WebGL-shaped.** Two shipped numbers
   are rationed against the 1.5 M: `CLUTTER_RICH_PER_TILE = 96`
   (`sim-core/terrain.rs:2919`) and the conifer ring's 1.9 M verdict.
2. **Nothing measures the native cost.** No `RenderDiagnosticsPlugin` in the
   tree, and no VRAM or disk figure anywhere. Capture on a real GPU at the
   ring's p90 tree count, read draw calls and frame time (its wall-clock
   half is not assertable — `CLAUDE.md`), and propose into `DECISIONS.md`
   §open. Renumbering is spoken, never taken by the loop.
3. **`BASE_ANISOTROPY_MAX = 4`** was chosen for a software-rasterizer reason
   that does not transfer. ⚠ It is no longer a constant — only a comment at
   `client/src/render/textures.rs:60`.

Initial load < 15 MB and `ART.md` §7's 12 MB payload are already retired in
the docs; 60 fps on a mid laptop iGPU survives as a hardware floor.


## 0p3 · Photographing a panel — the screen the recipe cannot reach *(client lane)*

The site recipe stays (two `DECISIONS.md` §open rows cite it):
`terrain::haven(seed)` / `haven_shelter` / `waystation_canopy` give the
coordinates, `shard.toml`'s `dev_spawn = "x,z"` stands the capture camera
there, then `Xvfb :9 -screen 0 1280x720x24 &`, the shard, and
`VK_DRIVER_FILES=/usr/share/vulkan/icd.d/lvp_icd.json DISPLAY=:9
WGPU_BACKEND=vulkan target/release/gates --server 127.0.0.1:4433
--capture <dir>` — six vantages, ~40 s. They face N/E/S/W, so stand on the
opposite side of what you want in frame. **This asserts nothing and must not
become a gate** (`CLAUDE.md`: the visual gate is a person).

Still owed, from §0p2 item 3: **the panels.** `render/capture.rs` knows two
subjects, `Player` and `Build`, and names no panel anywhere, so inventory,
crafting and the wheel are seen only by a human with a shard up. Wanted is a
viewer that opens each panel against a stocked fixture and writes a PNG per
screen — the camera pointed at a screen rather than at a place.


## 0vj · The capture probe ships frames with no record of what went wrong *(harness lane)*

⚠ The old premise is stale and should not be re-queued: the shell wrapper
landed outside this repo. `gates-loop/art/capture-native.sh` is what shot the
2026-08-14 frames (`CLAUDE.md` §the loop, `RENDER.md` §capture, and this file's
own §0gi item citing `capture-native.sh:44`), and a `-visual.md` was written.
Whether the loop is running at all is `CLAUDE.md`'s business, not an item here.

What is still open, and is ours rather than the harness's: the probe writes
PNGs only — `crates/client/src/render/capture.rs` joins four
`{idx}-{label}.png` paths and contains no `manifest` and no json — while the
visual judge's prompt asks for a `manifest.json` carrying the run's errors. A
capture that reports what the client logged while shooting is better evidence
than six pictures alone.


## 0bd · The tree blocks 0.3 m of ceiling nobody draws *(client+sim lane)*

The barrel was measured and closed (0.585 ⌀ × 0.88, gated both ways by
`greybox.rs::every_drawn_archetype_fits_the_volume_the_sim_blocks`). The tree
row is the one `greybox.rs` **excuses**, and it is only half closed.

1. **`OCCUPANT_TOP_M[Tree] = 5.7` cites dead code.** `terrain.rs:4245` reads
   `5.7, // Tree — PINE_TRUNK_H`, and `PINE_TRUNK_H` (`render/props.rs:45`)
   belongs to `pine_mesh`, which carries
   `#[allow(dead_code, reason = "the far-LOD silhouette, per TERRAIN.md §4")]`
   — nobody draws it. The drawn broadleaf is `SPECIES[1].height_m = 5.4`
   (`render/tree.rs:127`), so the sim blocks 0.3 m of invisible ceiling over
   half the pool. Nothing measures it: `greybox.rs:210`'s excuse holds only
   the **trunk radius**, by name.
   The fix is the barrel's — bound the drawn mesh over its own height band in
   `tests/tree.rs` and take the number off it, rather than pasting one.
2. **`assets/models/WANTED.md` §2.8 still briefs the loot barrel at the
   retired browser guess `0.9 ⌀ × 0.95`.** A mesh bought to that spec reddens
   the greybox gate on arrival, which is a purchase this repo would pay for.


## Numbers, worldgen and the arc *(content + world lanes)*


## 0b · Balance — the reference rows still outstanding *(content lane)*

⚠ **Derive the raid ratio, never quote it** — `Content::load_dir(…)` then
`.anchors()`, five lines. Four quoted readings have gone stale in two days.
⚠ Two operator rules (2026-08-10): a band of ours yields to a number of
theirs by default (`BALANCE.md` §6.5), and a number ABSENT from
`RIPLIST.md` has not thereby been decided either.

`reference/RIPLIST.md` §2 is the queue and the six steps; read it before
touching a balance number and do not re-derive the list here.

1. Next unblocked row is **1g**, the research ladder's per-item ordering
   (`READY`, page tier). Settle the era question (§1f) before taking it.
2. Blocked, researched, numbers already written down: **1j** `armor.toml` —
   one re-anchor of `content/tests/content.rs::band_breaks_refused`, best
   landed inside equipment v0; **1i** `loot.toml` — needs a `guaranteed`
   column on `LootEntry`, and the half-take measures 9× worse than nothing.
3. **No per-material damage resistance**: `content/src/schema.rs:281` has one
   `structure` column, so the ladder above stone is compressed (row 2).
4. Gather yields, smelt and craft times are still ours; per-hit yields and
   sub-second precision (row 3a) are schema work.
5. **Logistics friction (~10–30×) outranks mob→player damage (~2–5×)** —
   model threat as trip shape, never as a rate multiplier (rows 5, 6).


## 0n2 · Monuments — the solver is two hand-written tiers *(world lane)*

Read `reference/MONUMENTS.md` §9 first (§0: the weakest provenance here).

1. **§9.3, the solver.** `haven()` + `pick_minor` give two kinds of site, the
   separation floor is one hand-asserted constant (`WAYSTATION_MIN_SEP_M`,
   `sim-core/src/terrain.rs:1033`), no reservation ledger — §1's starvation
   shape at five tiers. **The trigger is a third destination kind.**
2. **Arrows pass through every deployable** — `sim-core/src/ranged.rs` never
   asks the solid nibbles, same class as its piece gap.
3. **Whether a sleeper blocks is unanswered** (§0y item 1) — a design call.
4. Two art rows in `DECISIONS.md` §open: the shelter's corner posts stand
   1.2 m proud of its roof and read as stubs; swept ground reads as
   scattered shards at 2 m because of the pebble mesh.
5. Then §9.4: per-entity interest ranges, then nav. Vertical AOI layers are
   premature; moving monuments are refused on the record. ⚠ This section's
   "class S has no interest filter at all" is **stale** — it landed
   2026-08-18 (§0n1).


## 4b · The world lane: what the second tier left open

1. **Every deployable comes from a player**, so a destination still offers
   no verb you cannot perform at your own base — the recycler is craftable.
   The missing mechanism is an **authored worldgen deployable**: a
   `DeployRec` standing at the pad that no player placed, which must answer
   to persistence (a restart must not duplicate it) and to `pick_up`
   (nobody pockets the haven's machine). The tree already reserves the
   case: `sim-core/src/world.rs:1611` treats owner `0` as "the authored-site
   case arriving early". Systems lane. Bank and vendor stay blocked on an
   operator act.
2. **Nothing threatens the walk between destinations.** Guards v0 leashes
   wolves to a site's `SiteFootprint` (`tests/guard.rs`), so the SITES are
   contested and the ground between them is empty. Note the promotion:
   `MONUMENTS.md` §9.4 item 4 said nav enters "the moment an NPC defends a
   monument", and one does — guards route through `movement::step`, so they
   slide along a shelter wall rather than path around it.

⚠ The pad-carve bullet is closed: there is no `DECISIONS.md` §open "site
carve v0" row. The carve is the dated 2026-08-16 row and its three
constants are pinned by `sim-core/tests/carve.rs` §A.


## 7 · Milestones — the arc is `DESIGN.md` §11; the queue adds two gates and one item *(systems lane)*

Read the arc in `DESIGN.md` §11 (M0 landed → M1 → M2 → M3 → M4, with exit
conditions); `ALPHA.md` §6 folds into it. Nothing is restated here.

Two gates sit between milestones and belong to the queue:
- **A1 playtest** (operator schedules): 10–20 testers, one wipe cycle, after
  M3 and before A2/A3 arming (`ALPHA.md` §2). A loop proposes it, never runs it.
- **Arming A2, then A3** is an operator act (`CLAUDE.md` §loop discipline).

One item the arc does not carry, still unbuilt — no visibility test exists in
`crates/server/src/interest.rs` or `sim-core`:
1. **Anti-ESP occlusion culling** — the measure the genre proved (Facepunch,
   2025, network-wide default), so this is a shipped industry default rather
   than a speculative one. Server-side, costing no client trust. The
   grid is a pure function of the seed, so it is bakeable at worldgen and a
   lookup in the tick. Sequence after M2: it wants real sightlines to tune
   against.

2. **The vendored SDK seam is ours and has no gate for upstream movement.**
   `crates/client/src/elo_overlay.rs` must stay byte-identical to
   `scry-forge`'s `sdk/rust/elo_overlay.rs` (`CLAUDE.md` §vendored); the pin
   catches a LOCAL edit and is blind to upstream moving. The check is a command
   you run when you touch it: `sha256sum` must appear in upstream
   `sdk/SHA256SUMS` — in `scry-forge`, never the `scryward` mirror, which lags
   and gives a false green. Derive the launcher's real state from elo, never
   from this file.
   ⚠ **This is no longer hypothetical.** It sat 326 lines behind once (2026-08-09,
   caught before it cost anything), and on 2026-08-29 the second drift had been
   breaking **every login** for eight days: upstream's rename moved the socket
   env and all three default paths, so the client found no launcher on a box
   running one and the shard refused it as a guest. Re-vendored the same day.
   A gate is still owed and CI cannot hold it (the number lives in another repo,
   on morr); the cheap half is a nightly job that fetches `sdk/SHA256SUMS` and
   compares — that is the shape to build, not another in-tree assertion.

Standing rule: anything a playtest breaks jumps this queue; anything a wall
catches jumps the playtest.


---

# OP · the operator lane — a loop cannot pick any of these


The sections below are the operator's. Nothing in them is a queue entry for a
builder; they sit at the bottom of the file so a pass reaches pickable work
first.


## LOOK · Boot it on a GPU and look — the act 27 items are waiting on *(operator)*

**This is the queue's largest single blocker and it had never been counted.**
`CLAUDE.md` retired the pixel gate on purpose — `vantages.mjs` passed all 36
checks on a beige smear — and says the visual gate is a person. That is the
right call and it is not free: it means every slice landed since is *gated as
arithmetic and unseen*, and the list below is what has accumulated. **Do not
build a replacement pixel gate.** One session with the client open closes most
of it.

Two of these are not taste, they are unresolved defects:

- **The ground's whole surface changed** (§0gs). A new `rock` texture, a macro
  break-up over every identity, and a biplanar tap on faces above 45° that no
  GPU here can compile. Gated as arithmetic and source scrapes; unseen.
  Since 2026-08-28 also **per-identity tiling** — grass draws at 2 m and
  litter at 1.3 m instead of a shared 4 m, so both are at life size and both
  repeat more often. Sand and rock are bit-unchanged. The question a frame
  answers and no gate can: does litter's 3.1×-finer repeat read as a lattice
  (`ART.md` rule 7), or does `MACRO_M` dissolve it?
- **Worldgen's shape changed under every frame** (§0wg). `remap` became a
  monotone cubic and a detail ladder landed after it, so the ground under
  every prop, tree and clutter tile moved. It is gated as arithmetic and
  hillshades; the operator's own screenshot of the terraced mountain has not
  been re-shot. This is the one item here where looking could find a
  regression rather than an unseen feature.

- **Nobody has stood in the dark holding a torch** (§0tl). Every capture in
  this repo's history is pinned to noon by `rig::CAPTURE_DAY_FRAC`, so the
  ten minutes in eighty that are night have never been photographed at all —
  with or without a light in the hand. Two questions only a frame answers:
  does a 600 lm pool at 0.89 m read as *carrying a light*, and does the
  torch's own head read as the source now that it is lit from 4 cm above its
  crown rather than emitting? Gated as arithmetic and one call-site scrape.

- **Nobody has watched a wall come down under arrow fire** (§5.1, ranged
  structure damage v0, 2026-08-28). The sim's half is gated to the payload
  byte — six `World`-level cases, seven mutants run and caught — and the
  question a frame answers is whether a shot at a wall *reads* as a raid:
  the mark, the hp readout and the collapse arriving together. It shares
  the decal blocker directly below: if no decal draws, an arrow chipping a
  wall is a wall silently losing hp.
- **No `ForwardDecal` renders under lavapipe at any size, alpha or
  orientation** (§0mk). The sim's half is confirmed to the centimetre; the
  frame shows no mark. That is a claim about this box — the client logs
  *"Too many textures in mesh pipeline view layout"* on boot — and one boot on
  real hardware settles whether every decal in the tree works or none does.
- **The sea's tangent `w` is `-1` and the ground's is `+1`** for the identical
  planar XZ parameterisation (§0pf item 4). One of them flips the ripple map's
  green channel. Look, do not guess.

Then, in the order a player would notice:

1. **A remote body's swing** (§0sw) — the arc has never been on a screen. The
   failure it would catch is a clip-table array width that panics the first
   time somebody swings near you.
2. **A body falling** (§0chr) — `Death01` is gated end to end and unseen; kill
   something and watch.
3. **The flinch, and the remote swing's sound** (§0pvp 1–2). Also **the
   hurt arc** (§0hrt 4) and **the three hitmarker rungs** (§0hs 3) — no
   capture vantage stands anywhere a shot can land, and hit rung v0 shipped
   three marker colours and three cues that nobody has seen or heard: this
   box has no sound card, so the *limb* cue in particular is unmeasured
   against the one thing it must not read as, which is a miss.
4. **The whole audio bank** (§0x, §0pr) — nobody has heard one cue, and nine
   of them are music. `cargo run -p client --bin soundbank -- <dir>`.
5. **LOW and MEDIUM** (§0gq) — the ladder's order is arithmetic, where each
   rung sits is a judgement. Is MEDIUM still the game?
6. **The far forest at 80 m** (§0lod) — the hull is opaque where a canopy is
   mostly air, so it should read *denser* than the near tree. That, not a
   popping silhouette, is the defect to look for.
7. **The broadleaf** (§0t item 1) — likeliest wrong are crown spread and leaf
   count/size.
8. **The announce stack** (§0tq) — whether 0.52 alpha on the deepest row reads
   and whether the `…+N more` suffix shifts the sentence under the eye.
   ⚠ This one cannot be closed by a `--capture` run at all: nothing in
   `render/capture.rs` can force a five-fact stack, so it needs live play.
9. **The tech-tree panel at a bench** (§0tt, §0tree) — press `E`.
10. **A world crate and a site guard** (§0wc 1, 4) — `dev_spawn` puts the
    camera at the pad; §0p3 has the command.
11. **The freehand build bit on a hillside** (§0bl item 5) — whether a height
    that changes as you sweep one cell reads as control or as twitch.
12. **The sky's swept bearing** (§0sun item 1) — a full revolution per cycle,
    exact and gated, and a visible change nobody asked for.
13. **The collapsed off arm and the sleeper tint** (§0chr item 6), **a spill
    line** (§0sp2), **the map's marked set** (§0a), **a diagonal base**
    (§0ac item 3), **the clutter ring's hard edge at ~32–45 m** (§0a).

And the two that need a machine rather than a look: **nobody has started the
Windows build on Windows** (§0win) and **nobody has ever joined the public
shard** (§0ab item 2).


## 0gq · Nobody has seen LOW or MEDIUM *(client lane)*

`config::Quality` is LOW/MEDIUM/HIGH and `render/quality.rs` is the table.
HIGH is the frame that shipped and `tests/quality.rs` holds that column, so
the default frame did not move — which is why it could land unlooked-at.

1. **Operator: walk the knob down and look.** The ladder's ORDER is arithmetic,
   but where each rung sits is a judgement and the visual gate here is a
   person. Is MEDIUM still the game?
2. **A render scale is the biggest lever and is not here** — Bevy renders to
   the window surface, so a scaled path is an off-screen target and a blit, its
   own slice. Same for the clutter and prop rings, which decide which tiles
   exist rather than how they draw (`ART.md` rule 4 is a floor a tier may not
   cross).


## 0sun · The sun's bearing sweeps — two calls the operator has not made *(client lane + operator)*

⚠ This block had **no heading in NOW.md** — it was orphaned after §0bl;
`client/tests/daynight.rs:268` and `sim-core/src/limits.rs:902` cite it as §0sun.

1. **Look at the sky before anyone builds more of it.** The cloud deck turns a
   full revolution per cycle about the vertical — horizon band fastest, zenith
   pivoting in place, the opposite signature to advection. Exact and gated as
   arithmetic (`client/tests/sun.rs`, `daynight.rs`), but a visible change
   nobody asked for, and there is no pixel gate by policy. The physically right
   answer is advection plus a lit term at sample time rather than baked.
2. **The noon bearing is southwest** (`RIG_SUN_AZIMUTH = 2.35`, 225.4°), so the
   path is SE → SW → NW rather than E → S → W. Moving it moves noon and retires
   every judged frame — its own pass, a re-capture, and the operator's word
   (`DECISIONS.md` §open "sun arc v0" carries both residuals).
3. In-lane and small: `render/capture.rs` spells the sky vantage's yaw as the
   literal `2.35` (line 217) rather than `RIG_SUN_AZIMUTH`, so if the pin ever
   moves that vantage stops looking at the sun.


## 0die · Two calls the operator still owes on the death screen *(operator)*

1. **Showing is not choosing.** The death map marks your beds, but
   `ActionMsg::Respawn` carries one bit (`on_bag`, `protocol/src/lib.rs`), so
   `World::wake` still takes the nearest ready bag through
   `deploys.claim_bag`. Letting a player click the bed they want is a bag
   index on the action plus a `claim_bag` that honours it — a wire bump, and
   an operator call on whether the choice is wanted at all.
2. `SUB_BAGS` is sent on a death and nowhere else — the one
   `encode_event_bags` call is in `server/src/core.rs`'s death path — so the
   `ready` bit ages while a player sits on the screen. Nothing is wrong today
   (the sim decides and `woke` says which anchor answered); re-send on the
   bed's own placement and removal if it starts to matter.
3. One operator call (`DECISIONS.md` §open, "death backpack v0"): whether
   five minutes is the intended floor for a common-only bag now the kit
   guarantees one.


## 0a · Is the map's marked set the right one? *(operator — a taste call)*

The marker layer and both ends of the trip are built: `world_to_map`,
`MAP_MARKS_MAX = 64` drop-newest, the own-bag and own-bed tags, and bag
choice v0 on the wire (`SUB_BAGS`, `server/tests/bag_choice.rs`), so the
death screen names your own bags and says which are spent.

One question is left and only the operator can answer it:

1. **Is the marked set right?** `MarkKind` (`client/src/ui/map.rs:263`) is
   haven, waystation, bed, spent bed, hearth, backpack. Boxes and doors
   stay unmarked deliberately — worth a look with the game booted before
   that stays the shipped answer.

The death-position half is settled (`DECISIONS.md` 2026-08-16 and
`ALPHA.md` §1: no corpse marker, no player marker) and needs no item.


## 0v · The furnace's ore rows want an operator's number *(systems lane)*

The furnace's three ore rows — `recipe.metal_frags`, `recipe.sulfur`,
`recipe.charcoal` (`content/recipes.toml` lines 362–384) — are still
station-gated crafts, not oven conversions. Moving them into
`sim-core/oven.rs` is the reference's model (`BaseOven`) and re-prices
the whole powder chain against `CONTENT.md` §4's bands: a balance pass
with an operator's number on it, not a refactor.


## 0rn · The rename's six loose ends *(operator, mostly)*

The 2026-08-21 row in `DECISIONS.md` has what moved. These are what could not,
each blocked on something outside this tree.

1. **(operator, wallet)** `scry.json` needs a re-sign at seq 6. Its `_next`
   points at `scry.moreright.xyz/api/library/GAME-REPO.md`, which is **410
   Gone**, and `_version` cites `ci/scry_manifest.py`, which is
   `ci/elo_manifest.py` now. Both are inside the signed bytes, so nothing here
   may touch them — the key is on morr. Filenames and the `"scry": 1` key stay
   whatever `/api/library/GAME-REPO.md` says on the day it is signed.
2. ✅ **Done 2026-08-29 — and it was load-bearing, not cosmetic.** The launcher's
   rename had landed on 2026-08-21, the same day as ours, and the re-vendor did
   not follow: `SCRY_LAUNCHER_SOCKET` → `ELO_LAUNCHER_SOCKET` plus all three
   default socket paths, so the client looked for a door that no longer exists
   and **every login had been refused for eight days** (no launcher → guest →
   `REFUSE_AUTH`, which reads as a signature failure). Re-vendored to
   `934a2b5d…`, and the file takes upstream's name with it —
   `crates/client/src/elo_overlay.rs`, because the local name tracks upstream's,
   which is what "never renamed" meant. Call sites checked: no shape changed;
   `play_message`'s first line moved (`scry play` → `elo play`) and is still
   called nowhere. The signed bytes were re-checked against
   `elo-broker::protocol::prove_message` and **had not moved at all**.
3. **The coins are being redeployed and are not out yet** (operator,
   2026-08-21), and **the listing copy left the tree with them** — `marketing/`
   is deleted, so there is nothing here to keep in step. `/api/onchain` naming
   SCRY, OBOL and MYRRH is the outgoing deployment, not drift. No number for
   the new coins may be typed in this repo: that copy is `scry-forge`'s, where
   the contracts and pool seeds are.
4. **`elo-shardlist-v1` is emitted and unread.** `ci/shardlist.py` writes the
   new kind; the live served document carries no `kind` field at all, so
   nothing breaks today — but the launcher should accept it before the next
   publish.
5. **The junk icon is a picture of two coins.** `assets/icons/junk.png` is
   still `delapouite/two-coins`, which was right for a coin and is wrong for
   scrap. An art call, on the CC-BY rail.
6. **`fix/us-east-shard` is closed** (2026-08-28). Nothing is owed: `shards.toml`
   already said `us-east-1` via the 2026-08-23 host move, `ci/depot.py`'s
   docstring fix landed independently on 08-14, and the two DECISIONS rows the
   branch carried — the 08-12 Virginia naming call and the 08-11 `servers.url`
   one — were salvaged into the ledger. The branch is deleted.
## 0rep · Where a filed report goes, and what it pays *(client lane + operator)*

1. **Nothing reads them but `ci/reports.py`**, which folds a directory onto its
   fingerprints and prints the board. No page serves it. **(operator: where.)**
2. **No intake, deliberately** — the client opens no socket and the player
   decides what happens to the file. An endpoint is its own slice.
3. **The `report` signing family does not exist in the launcher** — shipping set
   is `play`/`review`/`vow`/`hive`/`braid`/`store`. `report.rs::sign_text` is
   built to `sdk/PROTOCOL.md`'s rules and is refused today, which is the correct
   failure. Fix it upstream, in `scry-forge`, then re-vendor.
4. **A report pays its reporter** and the rail is built — a PR carries `Closes
   reports: <fingerprint>`. **Two things left, both operator:** how much against
   the PR's 100,000, and whether it pays on the merge or earlier;
   `DECISIONS.md` §open (bug reports v0) has the trade. Nothing pays until (3)
   lands either way: an unsigned wallet is a claim, and paying a claim pays
   whoever typed it.


## 0dsc · Discord presence is dark until an application exists *(operator — one act)*

Everything in code is built and gated (`crates/client/src/discord.rs`,
`render/presence.rs`, `render/settings.rs`, `config.rs`). It stays dark
because `GATES_DISCORD_APP_ID` has no value and no default.

1. Create the Discord application, set `GATES_DISCORD_APP_ID`, and name it
   `Gates` — the portal's application name is the word drawn after
   "Playing", which is what retires the lowercase `gates`.
2. For Ask-to-Join on a friend not already running the game, register the
   URL scheme in the portal (`elo://` or `gates://`). That path is
   `deeplink.rs` and needs no code; the already-running path is built.
3. Optional: a 512×512 or 1024×1024 image under the asset key `gates`.
   There is no Gates mark in this repo — and since `marketing/` was deleted
   there are no coin marks either — so Discord draws no image until one exists.

⚠ The detectable-list submission stays unverified: no current form was
found. A question for Discord, not a step, and nothing above depends on it.


## 0win · Nobody has started the Windows build on Windows *(operator)*

The packager stages the mingw runtime (`ci/depot.py` `runtime_dlls`) and
`nightly.yml`'s two-platform `depot` job runs the staged exe under wine, so
the loader is covered. ✅ **And it is now SHIPPED, which it was not before:**
`0.6.0-gb069a63b8` published to both platforms 2026-08-29, staging
`libstdc++-6.dll`, `libgcc_s_seh-1.dll` and `libwinpthread-1.dll` — the
build before it named `libstdc++-6.dll` in `requires.libs` and could not
start (`0xc000007b`). Smoke-run under wine before publishing, cold prefix,
`gates.exe --help` exit 0 with real help on stdout. Read the served
document before quoting a row: this line has named a stale one twice.

1. **Unmeasured**: nobody has started the depot build on a real Windows
   machine. The wine leg is a cold prefix answering `--help` — the loader
   and nothing after it. The next Windows boot is the measurement; a
   failure past `loader_init` belongs in a different item.
2. **Unmeasured, same class**: the GitHub release zip is msvc, not mingw,
   and nobody has checked whether it needs the VC++ redist.
   `release.yml`'s notes name Linux's three `-dev` packages and say
   nothing for Windows.
3. **Not ours to fix, and now actively WRONG rather than merely thin**:
   elo's launcher manifest on morr tells a player the Windows row bundles
   nothing. That was true when written and stopped being true on
   2026-08-29 — three DLLs ship beside the exe. The "never been run" half
   still stands (wine is a loader check, not Windows). The copy is morr's,
   not in `scry.json`, so no re-sign here can fix it.


## 0rl · The release path — two operator acts, and a tester's question *(platform lane)*

1. **Every draft since is unpublished.** `v0.2.0` is still the only
   published release (2026-08-13); `v0.1.0`, `v0.3.0`, `v0.5.0` and
   `v0.6.0` sit as drafts with six assets each and `v0.4.0` has no release
   row at all, while the tree is on 0.7.0. Read the API rather than this
   line — it has been stale at every version since it was written. The act
   is: open the newest draft, read what is attached, publish.
2. **`min_client` has never been raised on a live shard.** The order is
   publish the release FIRST and raise the floor after; `refused_build`
   climbing days later is how you find out you did it backwards.
3. **The macOS and Linux artifacts have never been RUN.** All six release
   jobs compile, link, stage and archive on real runners, and `nightly.yml`
   now starts the staged Windows build under wine (`gates.exe --help`,
   not allowed to skip) — but nothing here has a Mac to start one on. That
   is a tester's question, not CI's.


## 0ab · The store seam — what only an operator can finish *(platform lane)*

⚠ Every `scry.moreright.xyz` in this repo's prose is stale: the host was
retired 2026-08-20 and answers 410. The platform is `elopros.com`
(`ci/depot.py`, `ci/shardlist.py`, `ci/publish_depot.py` already moved).

1. **Publishing is an operator act, every release.** A build goes live when
   the origin's `published.json` names it and the digest is notarized, and
   `elo digest` — the one implementation of the notarized number — is not
   runnable from this box by construction.
2. **Nobody has ever joined the public shard.** `game.elopros.com:61234` is
   in the served list and `status.json` answers, but the tools here cannot
   measure a join: `bots` takes a `SocketAddr`, so it cannot dial the name
   the certificate is issued for (`server/tests/tls_posture.rs`), and it
   carries no wallet, so `require_auth = true` refuses it correctly. The
   first real join is a person with the published build.
3. **`elo://` is not registered with the desktop.** That is the launcher's
   installer, not this repo; `crates/client/src/deeplink.rs` is ready.
4. Re-run `./ci/shardlist.py` and re-copy `servers.json` to the origin
   whenever a row in `shards.toml` changes.


## 0ad · The ticket door waits on a deployed contract and a spoken sweep *(platform lane)*

1. **Nothing has been driven against a real ticket contract.**
   `ScryGameTicket:GATES` is not deployed, so `/of/<wallet>` answers
   `ticketed: false, entitled: true` for everyone and the door is a
   pass-through by design. Every branch is unit-tested against the
   response shapes elo serves (`tickets.py`); none has met the live
   route. First real check is the day the contract is deployed, and the
   honest way to run it is one wallet that owns a copy and one that does
   not.
2. **The sweep interval is unspoken.** `DEFAULT_SWEEP_SECS = 120` is a
   documented default, not an operator sentence, and it is the whole
   security property — how long a sold copy keeps playing.
   `DECISIONS.md` §open carries the row ("ticket door v0", PROPOSED).
3. **No `prove` call site**, so a join still costs the player a consent
   dialog on every join. The vendored SDK has `Overlay::prove` and
   `crates/client/src/elo.rs` says the slice is unbuilt. **The cost is why
   it is still open**: `prove` has the launcher compose the message, so the
   launcher writes its own `Issued At`, the server can no longer rebuild
   identical bytes and must PARSE an EIP-4361 message — and the wire has to
   carry that message, which IS a layout change (wall 6: version bump +
   goldens in the same commit). A slice, not a line.


## 0sl · The shard list reaches the game — two operator acts, in order

The tree half is done (`ci/depot.py:174` `LAUNCH_ARGS`, gated at :723;
`client/src/args.rs` parses `--servers`; `shards.toml` `id = "us-east-1"`).
The order is not a preference — a depot using `{servers}` needs a launcher
that knows it, and no depot document can declare a launcher floor, so an
older launcher refuses the whole launch:

1. **Ship the launcher** carrying `ARG_VARS` with `servers` in it
   (scry-forge, `launcher-rs`).
2. **Re-publish Gates' depot document** so `launch.args` carries
   `--servers {servers}`: `python3 ci/depot.py`, then the depot ceremony in
   elo `docs/client/LAUNCHER.md` §8. The re-package is owed anyway — the
   published document names `scry.moreright.xyz`, retired 2026-08-20 and
   answering 410 (commit c9a5e84).

Until (2) the fix is inert and the in-game browser stays empty. `--servers <url>`
on the command line is the workaround; joining from the Servers window works.


## 0s · The front door — the two acts that are not ours *(client lane)*

1. **The backdrop does not move**, and that knob is the operator's
   (`DECISIONS.md` §open "menu backdrop v0": *open for the operator — motion,
   and which vantage*). Bevy decodes no video; a loop is a frame sequence,
   ~4–12 MB for three seconds at 720p/20fps. The shipped still
   (`assets/menu/backdrop.jpg`) is a `--capture --no-hud` plate of our own
   island, so a better one is a command, not an art commission.
2. **Nothing publishes `news`/`store`/`workshop`**, so all three read "the
   launcher's manifest names no link for this yet" (`ui/hub.rs:183`). The
   client side is done; the remaining act is the platform's — add the keys
   beside `servers.url` in `data/launcher/gates.manifest.json`, which is not
   in this tree.
3. **Ungated, by hand only:** the star, the search box, the filters and the
   OPEN IN LAUNCHER click, driven headless with `xdotool` and looked at,
   never against a populated list or a live launcher.
4. **The splash cannot cover its own first ~3 s** — wgpu adapter enumeration
   and window creation precede the first Bevy frame. A second process would;
   not taken.


## 0wt · Dropping the HTTP/3 layer needs an operator-chosen flag-day *(server lane)*

We are not missing real QUIC — `wtransport` is quinn and `net.rs` already
uses `QuicTransportConfig` / `IpBindConfig`. What is vestigial is the HTTP/3
session layer on top: extended-CONNECT, the `https://{addr}` URL shape, a
session-id prefix on every datagram against the 1 100-byte budget.

The case is not speed. Our one remote-panic trap lives in that layer (#317),
which is why we depend on a git rev of an unreleased crate — and
`NETCODE.md` §2.2's ⚠ still says nothing records or gates that
`rev = a11e6a8e…` descends from the fix. Removing the layer retires the pin,
the trap and the browser-shaped cert rules in one move.

The seam is thin (client `connect`, server `accept`, `tls_posture.rs`,
`botclient.rs`, `Shard::url`); **the cost is the flag-day** — the handshake
changes, so nothing negotiates and an old client just fails. Two depots and
a public shard are live, and `elo-shardlist-v1` publishes the url shape.

**Not its own pass.** Bundle it with the next `min_client` floor raise, or
with the next touch of the wtransport pin. Wants the operator's word on
timing — publishing and floor raises are operator acts.


## 0wd · A new world register is proposed — blocked on the operator's word

`WORLD.md` is a roadmap rather than a v1 spec; `DECISIONS.md` §open carries
the row and nothing is spoken. A loop cannot pick this up.

Three findings about the tree rather than the fiction:
- `ART.md`'s bar and the visual rubric are measured off the reference set and
  the rubric is checksummed outside this repo, so an obsidian world scores as a
  defect by construction and no builder can fix it. Three operator acts: palette,
  reference set, rubric style section; until then no visual pass chases this.
- A ward would invalidate `CONTENT.md` §4 anchor 2 without reddening
  `test_content` — the TTK bands compute against `balance.toml:13`'s
  `player_hp = 100`. Conditional: the ward is undecided.
- Extraction and world states are one system or they are two; the terminal
  lands at A2 (`ALPHA.md` §2), and a bespoke gate first pays for one idea twice.

Cheapest slice if spoken: a radial third input to `biome(h, moist)`
(`terrain.rs:497`) plus regenerated goldens.


## 0gh · The GitHub job-agent seam — the acts still owed *(operator lane)*

Two listed acts are done, do not re-litigate: `scry.sig.json` is signed
(seq 5, sha matches `scry.json`; `--print` now offers seq 6), and the repo
description no longer says "three.js frontend".

- **(operator, GitHub)** Branch protection on `main` requiring the `gates`
  check — still off (`protected: false`); until GitHub enforces it the merge
  gate is policy. Caveat: `gates.yml` path-filters, so a docs-only PR reports
  no check; the fix is a same-named instant no-op for those paths.
- **(operator, once)** Settle `gates-pr` end to end on the next accepted PR:
  pay by public transfer, append the row elo-side. 0 forks, paid ledger `[]`.
- **(operator, GitHub)** The About **homepage** field still points at
  `https://scry.moreright.xyz` — retired 2026-08-20, answers 410 (c9a5e84).
- The manifest's `jobs` block is unwritten: `scry.json` has no `jobs` key, so
  this repo posts no board lane and the six rows stay house-side.

Not owed: no issues queue, no auto-pay or auto-merge, no webhook.


---

## Labels · what was deleted, and which citations are ambiguous

Kept because `crates/` and `ci/` cite `NOW.md §<label>` in doc comments, and a
citation to a section this file no longer holds is a pointer to nothing. Read
a `§`-citation as a hint and match on the title.

**Closed and deleted 2026-08-25** — both verified against the tree, not against
their own text. History is in git.

- **`§0kit`** (the rock, the two doors, the boot rule). Both stated remainders
  closed 2026-08-17: `wake`'s three doors are gated in `sim-core/tests/
  persist.rs` and `sleepers.rs`, each red-proven both ways, and the kit's boot
  rule is `validate::structural` + `parse_shard_toml`'s `MAX_SPAWN_KIT` check.
  Its title had been stale against its own body for a week.
- **`§5c`** (the protocol golden's button octet). Both named gates exist —
  `protocol/tests/protocol_golden.rs::the_input_golden_fuzzes_the_whole_button_octet`
  and `::the_loc_fuzz_covers_each_stores_whole_domain` — and the judgement it
  asked for is written at `PROTO_VER` (`protocol/src/lib.rs`) and in
  `goldens.rs`'s header. Nothing open.

**Folded into `§LOOK` 2026-08-25** — each had nothing left but "a person must
look at this", so the three of them are one line each in that list rather than
three sections of their own: **`§0lod`** (the far forest's swap band, `§LOOK`
6), **`§0sw`** (a remote body's swing, `§LOOK` 1 — the array-width panic it
would catch is recorded there), **`§0tq`** (the announce stack's alpha and its
`…+N more` suffix, `§LOOK` 8).

**Ambiguous labels** — these resolve to two or three sections each, so a bare
`§`-citation cannot say which. `0v` is three ways ambiguous.

| label | resolves to |
|---|---|
| `0a` | the clutter ring's fade *(client)* · the island's map *(ui, operator)* |
| `0u` | the ghost's lock-on-a-door *(client)* · the frame budgets *(client)* |
| `0v` | the furnace's ore rows *(systems)* · players are people *(client)* · the menu flow *(client)* |
| `0w` | the props' gaps *(client)* · the native menus *(client)* |
| `0x` | the client's sound *(client)* · the native client's trim *(client)* |
| `0y` | the sea *(client)* · persistence *(server)* |
| `0z` | the Bevy-draws rule's gate *(client)* — doors is `§0zd` now |
| `4b` | the world lane *(world)* · the domain gate *(platform)* |

**Renamed this pass**, because the collision was load-bearing rather than
cosmetic: doors and locks `§0z` → **`§0zd`** (it collided with the Bevy audit
`§0z`, and three doc comments in `sim-core/{deploy,claim}.rs` cite "§0aa
item 1" / "items 1–2" under numbering that has since moved — re-point them
when you next touch that file).

**Dangling the other way**: `sim-core/src/collide.rs` sends an arrow-through-a-
floor to `NOW.md §0ar`, **a label this file has never had**. It lives in
`§0mk` item 2 and `§0bl` item 3.
