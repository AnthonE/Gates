# reference/PROJECTILES.md — how the reference game does bows and arrows

Ripped facts, not design. `DOORS.md` answers who is allowed through a door,
`BUILDING.md` who may build here, `SAVES.md` what survives a restart; this
file answers **what happens between pulling a bowstring and a body falling
over**, because we shipped ranged v0 on 2026-08-06 and `ranged.rs`'s own
header ends by listing four things it does not do.

Dated 2026-08-10. §9 is the part that changes what we build.

## 0 · Provenance — read this first

Three sources, ranked, same ladder `DOORS.md` §0 sets:

1. **`reference/rust-systems.txt`** — in this tree, MIT, regenerable. A
   *hook* table, so what it proves is **shape**: which classes exist, which
   methods carry hooks, which signatures are shared. §1 reads the object
   model off it and does nothing more. This is the strongest section in the
   file and the only one with no caveat.
2. **The developer's own devblogs**, by number and date.
3. **Community wikis, guides and the convar dumps** for numbers. Weakest —
   player-maintained, mostly undated, over a decade of balance passes.
   Read them as *ratios that held*, never as today's values.

⚠ **The proxy blocked every page fetch again, and `SOURCES.md` says it
should not have.** That file records a 2026-08-09 re-probe finding
`rust.facepunch.com/news` and `wiki.facepunch.com` **open**, serving full
page text, and explicitly warns the next reader not to repeat a blanket
blockage claim without re-probing. So this pass re-probed rather than
assumed — and on **this** container, 2026-08-10, `rust.facepunch.com`,
`wiki.facepunch.com` and `wiki.rustclash.com` all return `EGRESS_BLOCKED`
from the egress proxy.

Both readings were honest measurements. The conclusion that survives them is
not "open" or "blocked" but that **reachability is a property of the
container, not of the hosts** — it changes between sessions and neither
answer may be cached in prose. Probe the row you need, in either direction.
`SOURCES.md` §0 is corrected to say so.

The practical cost here: tiers 2 and 3 below arrived as **search-result
summaries of those pages, not the pages**. A summary can drop a qualifier,
and numbers in §4 should be treated as approximate even where they are
written without a range. Tier 1 is read out of a file in this repo and is
exact — which is the second reason §1 leads.

Nothing here was decompiled. Nothing here ships.

## 1 · The object model, read off the hook table

Five entries carry the whole system, and the first one is the finding:

```
BaseProjectile [6]  CLProject(BaseEntity/RPCMessage)          ← OnWeaponFired
                    TryReloadMagazine(IAmmoContainer,Int32,Boolean)
                    SwitchAmmoTo(BaseEntity/RPCMessage)
                    UnloadAmmo(Item,BasePlayer)
                    StartReload(BaseEntity/RPCMessage)
                    DelayedModsChanged()
BasePlayer     [3]  OnProjectileAttack(BaseEntity/RPCMessage)  ← OnPlayerAttack
                    CreateWorldProjectile(HitInfo,ItemDefinition,
                                          ItemModProjectile,Projectile,Item)
                    OnProjectileRicochet(BaseEntity/RPCMessage)
AttackEntity   [1]  ValidateEyePos(BasePlayer,Vector3,Boolean) ← OnEyePosValidate
BaseMelee      [1]  DoAttackShared(HitInfo)                    ← OnPlayerAttack
```

Six structural facts fall straight out of that, and they are the valuable
half of this document:

1. **There is no `Bow` class.** A bow is a `BaseProjectile` — the same class
   as every rifle, pistol and the crossbow. It reloads a *magazine*
   (`TryReloadMagazine`), it can *switch ammo* (`SwitchAmmoTo`), it can
   *unload* (`UnloadAmmo`). A bow is a one-round magazine weapon and nothing
   about it is special-cased. Every gun mechanic they later built arrived at
   the bow for free.
2. **`CLProject` — the `CL` is `client`.** The method the *fire* hook hangs
   on takes an `RPCMessage`: the client creates the projectile and tells the
   server. This is the single most important fact in the file and §2 is
   about it.
3. **`OnProjectileAttack` is also an `RPCMessage`, and it is on
   `BasePlayer`.** The *hit* is a second client claim, sent separately from
   the shot, and the server's job is to disbelieve it. Contrast the melee
   line: `BaseMelee.DoAttackShared` — "Shared" is Unity's convention for
   code compiled into both builds, and it takes a `HitInfo`, not an
   `RPCMessage`. **Melee is resolved by shared code; ranged is resolved by
   the client and audited.** Two different trust models in one game.
4. **`ItemModProjectile` is a *mod on the ammo item*, not a field on the
   weapon.** It appears in `CreateWorldProjectile`'s signature alongside
   `ItemDefinition` and `Projectile`. The ballistics belong to the arrow. §9
   argues this is the one place our schema is shaped wrong.
5. **`CreateWorldProjectile(...)` ends its signature with `Item`.** A
   projectile that lands becomes an *item instance in the world* — the
   arrow you pull out of a tree is the arrow you fired, not a fresh one. It
   hangs off `BasePlayer` rather than off the projectile, and it has both a
   `Can…` and an `On…` hook, which is the pattern the loader uses for
   "refusable, then announced".
6. **Ricochet is a client RPC too** (`OnProjectileRicochet`). The client
   reports its own bounce. §3 is why that needed its own verification pass.

## 2 · The client fires, the server disbelieves

Facepunch's own summary, across devblogs 116–140: *the projectile code has
always been client-side, with server-side verifications to ensure clients
can't just send whatever damage output they want.*

That is not laziness, it is the only way to get a responsive gun over the
internet, and it is the same trade we make for movement. The client owns the
felt experience; the server owns the truth, arriving late. What is
instructive is **how much machinery the second half needed**, and that it
was built over roughly five years *after* shipping the first half.

The sequence, by devblog:

| # | date | what landed |
|---|---|---|
| 116–118 | 2016 | server-side verification for projectile **and** melee; false positives from over-strict **speed** verification fixed in 118 |
| 123 | 2016 | projectile **movement segmentation** — the movement sim tick is split at the exact time a direction change happens, rather than resolving ricochet once per frame |
| 140 | Dec 2016 | line-of-sight verification finished; **verified periodic position updates** for in-flight projectiles; ricochet verification fixes |

Three things worth taking from that table:

- **Verification was retrofitted, and it hurt.** 118 is a devblog spent
  apologising for legitimate bullets being rejected. A trust model bolted on
  after the fact spends years converging.
- **Line of sight is the check that pays.** Its stated purpose is blunt:
  stop people damaging sleepers, boxes and cupboards *through walls*. Not an
  aimbot measure — an exploit measure.
- **Segmentation is the same insight our sampler has.** A projectile that
  resolves collisions once per frame resolves them at the wrong *place*; you
  have to subdivide the tick's segment. They found it in 2016 for ricochet;
  `ranged.rs` has it as `ARROW_STEP_MM` for the same reason.

## 3 · The anti-hack ladder

The verification stack became convars. The ladder is stated as levels, and
reading it as a *build order* is the useful move — each level is a check
they added when the one below it proved insufficient:

```
antihack.projectile_protection
  0  disabled
  1  speed
  2  speed + entity
  3  speed + entity + LOS
  4  speed + entity + LOS + trajectory
  5  speed + entity + LOS + trajectory + update
  6  speed + entity + LOS + trajectory + tickhistory   ← default
```

With tolerances beside it, each of which is an admission that the check
alone was too sharp: `projectile_forgiveness 0.5`, `projectile_losforgiveness
0.2`, `projectile_anglechange 60`, `projectile_backtracking 0.01`,
`projectile_clientframes 2`, `projectile_serverframes 2`,
`projectile_damagedepth 2`, `projectile_impactspawndepth 1`,
`projectile_desync 1`, `projectile_terraincheck true`.

⚠ **Read the forgiveness constants, not the level list.** Seven of the
thirteen convars exist to *loosen* a check. `clientframes 2` and
`serverframes 2` are both "allow two frames of disagreement about when this
happened"; `losforgiveness 0.2` is "allow 20 cm of wall"; `damagedepth 2` is
"allow the hit to be 2 m inside the thing it hit". A verification system for
a client-simulated projectile is not a predicate, it is a **tolerance
budget**, and every one of those numbers is a place where a cheat fits
exactly inside the tolerance. Players still report `Projectile Invalid` on
ordinary lag, which is the same budget failing from the other side.

This is the strongest argument in the file for our design, and §9.1 makes it.

## 4 · The numbers

⚠ Tier 3. Approximate, undated, and balance-passed for a decade. Present as
ratios, not values.

**The bow.** ~35 damage on a body hit with a wooden arrow at full draw;
draw ~1 s; reload ~2.75 s. Crafts at workbench level 0 from **200 wood +
50 cloth**, and the blueprint is **known by default** — no scrap, no tech
tree, day-one weapon. Arrows are 25 wood + 10 stone per 2.

**Damage by body part** is a multiplier on the weapon: head ×2, chest ×1,
limbs ×0.5 for most weapons, with a **per-weapon override** so a precise
weapon can carry a higher head multiplier and a shotgun a lower one. Wiki
figures for the bow specifically say limbs 25–30, major parts 46–55, head
57–73 — which is a wider spread than a clean ×2/×1/×0.5 and is the kind of
detail a search summary mangles. Treat the *structure* (per-weapon override
on a shared multiplier table) as the fact and the numbers as illustrative.

**The arrow types** are the interesting part, because they are four
different ballistics on one weapon:

| arrow | relative | why you carry it |
|---|---|---|
| wooden | baseline | crafts from wood + stone |
| bone | less damage, better accuracy | bone fragments replace the stone — *cheaper than wood* |
| high velocity | **−20 % damage**, faster, flatter, longer | less lead on a moving target |
| fire | wooden-equivalent damage, low velocity, **area of effect** on impact that damages and slows | needs workbench 1 and tech-tree scrap |

Note what that table costs to build: **damage, velocity, drop and an impact
effect all vary per arrow while the bow stays the same object.** That is §1
fact 4 showing up as content.

## 5 · The arrow is an item twice, and that is the economy

The reference game's arrow is an item in your inventory, becomes a
projectile, and then becomes an item in the world again
(`CreateWorldProjectile`). The rules around the third state:

- ~**15 % chance to break** on impact and be destroyed.
- An arrow that **dealt damage** lodges in the target and can be retrieved
  after **10 seconds**.
- An arrow that **missed** can be picked up immediately.

Those three rules are one design, and the design is that **ammunition is
mostly durable and the loss is a tax, not a cost**. The 10-second lodge
timer is the part that reads as arbitrary and is not: it stops you
re-collecting the arrow you just shot someone with *during* the fight, so an
archer still runs dry in a sustained engagement while losing almost nothing
to a day of hunting.

⚠ **This is the `RIPLIST.md` §0 threat frame exactly.** Their ~35 bow damage
is priced against ammunition that comes back 85 % of the time. Ours is
priced against ammunition that never comes back. Taking their damage number
without their recovery loop is §4.1's false-familiarity trap: the number
matches and the weapon means something different.

## 6 · The draw

The hunting bow has a ~1 s draw and, per the wiki tier, fires at full draw.
The **compound bow** is where the mechanic is explicit and is worth reading
as the more considered design:

- Holding the draw raises **damage, range and projectile speed** together —
  a charged shot is stated as roughly double.
- You **cannot move** at full draw, and moving resets the draw.
- Holding a full draw **costs weapon durability per second**.

Three costs on one verb — exposure, mobility, and consumption of the weapon
itself — so a maximum-power shot is never free and never spammable. The
durability drain is the unusual one and the most transferable: it prices
*holding* rather than *firing*, which is the thing a patient player would
otherwise get for nothing.

## 7 · Hit detection: the most significant part, not the first one

Devblog 104's collision rework is the one non-obvious algorithm here.

- The hitbox is a **player collision mesh that does not change with
  clothing.** They had shipped the opposite, and it produced the bucket
  helmet — headgear that inflated the head hitbox so much it was a
  liability. Armour that changes your silhouette is a trap.
- On a hit, the system **considers every body part along the line of sight
  and damages the most significant one**, rather than taking the first
  intersection. Explicitly so that a visible torso is not saved by a hand
  that happened to be in front of it.

The second rule is a genuine design inversion and cheap to state: *first
intersection* is what a raycast gives you and it is the wrong answer,
because limbs are in front of bodies constantly and a ×0.5 limb hit on a
clearly-hit player reads as the game cheating. They also rebuilt hit
detection around **best-fit** rather than exact intersection.

## 8 · What they shipped broken

- **Over-strict speed verification rejecting real shots** (fixed 118), and
  an "AntiHack!" chat message from legacy code firing at innocent players.
- **Ricochet resolved per frame** rather than at the moment of deflection
  (fixed 123 by segmentation).
- **LOS verification incomplete for three devblogs**, with damage-through-
  walls on sleepers and boxes live in the meantime.
- **`Projectile Invalid` on ordinary latency** — still reported by players
  long after; the standard server-admin remedy is
  `antihack.projectile_protection 0`, i.e. turning the whole ladder off,
  which tells you what the tolerance budget costs to tune.

Every one of those is a consequence of §2: the client owns the projectile.

## 9 · What it means for us

Our ranged v0 (2026-08-06, `crates/sim-core/src/ranged.rs`) is **not** the
reference architecture, and the divergence is deliberate and correct.

### 9.1 · We already won the fight §2 and §3 describe — do not give it back

Their projectile is client-simulated and audited by thirteen tolerance
convars. Ours is simulated on the server in integer millimetres, on a
128-slot store, with the trajectory a pure function of `(yaw, pitch,
speed_mmpt, drop_mmpt2)` and no float in the integration. There is nothing
to forge: the client never tells us where its arrow is, so
`projectile_losforgiveness` has no analogue here and never should.

That costs us what it costs them to have: the shooter waits a round trip to
see a hit. Their trade bought responsiveness and thirteen convars of
tolerance. **Ours is the right trade for a determinism-gated sim** — a
client-authored projectile cannot go in the WAL, cannot be replayed, and
cannot be trusted in an RL training loop where the "client" is an agent
optimising against exactly those tolerances (`PLAYERS.md`).

Write this down because the pressure to reverse it will arrive as a
complaint about arrow lag, and the answer to that complaint is a client-side
*tracer* (§9.2), never a client-side *arrow*.

### 9.2 · The gap a player actually sees — **landed 2026-08-10**

**Done. The arrow is visible.** `EV_SHOT = 35` (wire v33) broadcasts the
shooter, the two aim angles, and the round's speed and drop;
`crates/client/src/render/tracer.rs` draws it.

The payload question this section flagged as "the real work" resolved by
**not carrying the origin at all**. The client knows where the shooter is
from the snapshot and `ARROW_EYE_MM` is a constant on both sides, so origin
is derivable; the item is not carried either, because an arrow is an arrow
to look at. What *had* to cross instead was the ballistics — `client-core`
holds no content tables, it is a wire and prediction layer — and carrying
them turned out to be the better design rather than a concession: handed
speed and drop in mm/tick, the tracer runs **the same integer integration
the sim runs**, so the drawn arc is not an approximation of the real one
that drifts over a second of flight. It is the same arithmetic. That is the
quantize-both-sides law applied to a tracer.

Everything the section predicted about cost was right, and one thing it did
not predict: `EV_MAX` → `EV_SHOT`, `PROTO_VER` → 33 with all 82 goldens
renamed and regenerated in the same commit plus one new
(`v33_event_shot.bin`), and `event_roles.rs`'s 35th row — proven red under a
b/c swap before being called done.

**It was authored as v31 and landed as v33.** Research v0 took `PROTO_VER`
32, `EV_RESEARCH`/`_REFUSED` 33–34 and `SUB_RESEARCH`/`_REFUSED`/`SUB_KNOWN`
44–46 while this branch was open, so the two collided head-on on both
numbering axes — `EV_SHOT` and `EV_RESEARCH` were both 33, `SUB_SHOT` and
`SUB_RESEARCH` both 44. That is exactly the case `CLAUDE.md` names when it
says `protocol` and `limits.rs` never land from two branches in one merge
window, and the collision is *silent* in the worst way: two ends agreeing on
bytes that mean two different things. Renumbering above them at the merge
(35, 47, v33) was the whole fix — no behaviour on either side moved.

**And it is a tracer, not a projectile.** Nothing downstream may read it;
the arrow that kills you is the server's and its `EV_HIT` arrives whether
or not anything was drawn. §9.1 is why that line is where it is.

### 9.3 · Our ballistics were on the wrong object — **landed 2026-08-10**

**Done.** `[weapon.ballistic]` is gone; `content/weapons.toml` carries an
`[[ammo]]` table and speed and drop belong to the round. The move was
value-preserving on purpose — `item.arrow_wood` is the bow's old 40 m/s and
`item.arrow_metal` the crossbow's old 55 — so nothing shipped flies
differently; what changed is what the schema can now express. A weapon's
`ammo` is a list (`MAX_WEAPON_AMMO = 4`, their four arrow types) and the sim
spends the first round the shooter carries.

Two consequences that were not free, both recorded in `DECISIONS.md`
§open ("ballistics on the ammo"): `life_ticks` could not stay baked on the
weapon, because flight is reach over speed and one bow's fast and slow
arrows cross the same range in different tick counts — so `RangedDef`
carries `range_mm` and `ranged::draw` divides at the shot. And the sampler
wall moved with the number, which is the better place for it: an
untraceably fast round is now refused whichever bow picks it up.

**Still not theirs**: no per-round damage column, so every arrow out of one
bow hits equally hard (their HV arrow is −20 %, and that multiplier is
unspoken — §open, not invented into content); and no `SwitchAmmoTo`, so list
order is the whole ammo policy.

The rest of this section is the argument as it stood before the change, kept
because it is why the shape is what it is.

---

### 9.3a · The argument (pre-2026-08-10)

`content/weapons.toml` puts `[weapon.ballistic]` on the **bow**:

```toml
[[weapon]]
id = "item.bow"
ammo = "item.arrow_wood"
[weapon.ballistic]
speed_mps = 40
drop_mps2 = 20
```

Theirs is `ItemModProjectile` — a mod on the **ammo item** (§1 fact 4). The
consequence is exact and it is a schema wall, not a balance one: **with
ballistics on the weapon, one bow can only ever fire one kind of arrow.**
Every row in §4's arrow table — high velocity trading 20 % damage for speed,
fire arrows trading velocity for an impact effect — is unreachable from our
schema no matter what numbers we choose. We have `item.arrow_wood` and
`item.arrow_metal` in `items.toml` today and they differ only by which
weapon spends them.

Moving `ballistic` from the weapon row to the ammo item is a content-schema
change (wall 7: `content/*.toml` + `validate` + `canon` + the content hash),
and it gets strictly harder every arrow we add first. It is the single
highest-leverage thing in this document and it is not urgent — which is
exactly the combination that means write it down now.

*(Written 2026-08-09; the operator called it the next day and §9.3 above
records what landed. Kept unedited — the reasoning is the reason the shape
is what it is, and a prediction is worth more when you can still read what
it said.)*

### 9.4 · `headshot_mult` was armed and unread — **BUILT 2026-08-30**

*(Kept in its original terms, with what landed appended. The prediction is
worth more when you can still read what it said.)*

Every row in `weapons.toml` carries `headshot_mult = 2`. It was validated
(`validate.rs`), band-checked (`balance.rs`), hashed into the content hash
(`canon.rs`) — and **no sim code read it**. `ranged.rs` said so plainly
("No headshot, so the whole body is one target") and `combat.rs` said the
same for melee.

That is the same shape `ranged.rs`'s own header opens with: content armed,
validated, hashed, and thrown away at the sim boundary. It was true of the
bow for months before ranged v0 salvaged it, and of `structure` until
2026-08-28.

§7 says what to build: **not** first-intersection. The body is one cylinder;
a head is a second, shorter cylinder at the top, and the rule is *most
significant part along the segment*, which for two parts is "if the head
interval is crossed at all, it is a headshot". The closest-approach solve in
`ranged.rs` already produces the `t` this needs.

**What landed (headshot v0), and the two places it differs from the
paragraph above.**

1. **A band, not a second cylinder.** `collide::HEAD_BAND_M = 0.25` is the
   top quarter-metre of the same 1.7 m cylinder. A second collider would
   need its own radius, its own entry solve and its own place in the hit
   decision, and with two parts it can answer no question the band cannot —
   the reduction §9.4 itself states is what makes the band sufficient.
2. **The `t` was necessary and not sufficient.** The closest approach is
   one point, and §7's rule is about a *crossing*. `nearest_body` now
   finishes the planar quadratic it was already taking the vertex of and
   returns `BodyHit { t, slot, enter, exit }`; `ranged::head_crossed` is the
   interval overlap. This is the difference between a shot that enters
   through the crown being a headshot and being scored at whatever the
   chest was — and it is the one behaviour a closest-approach reading gets
   wrong, gated as `a_shot_that_enters_through_the_crown_is_a_headshot_at_
   the_chest`.

The multiplier is applied to raw damage before armor reduction (a plate
stops a percentage of the blow that arrived), the span is clipped against
the world's stop (cover between a chest and a crown is cover), and both
`EV_HIT` and `EV_HURT` carry the scaled number — no wire change, because
the field already meant damage. **Melee did not get one**: `strike` is
planar, so §9.4's own "combat.rs says the same for melee" is still true and
is now a decision with a gate on it rather than an omission.

Gates: `crates/sim-core/tests/headshot.rs` (9 checks, ten mutants run and
the two survivors written down in its header) and `content`'s
`the_headshot_column_reaches_the_sim`.

### 9.4b · The limb, and the ordering it forced — **BUILT 2026-08-30**

§9.4's reduction was load-bearing and it did not survive the third part.
"With a head and a body and nothing else, most significant part crossed is
was the head interval crossed at all" is true and is *why* a bool was
enough; a leg band makes it false, because a span that misses the head has
two answers now. So `head_crossed` became `ranged::part_crossed` returning
`collide::Part`, and §7's rule is that enum's derived `Ord` — the `max`
over the bands a span touches. The inversion §7 exists for is now
assertable in one line: a shot crossing a shin **and** a chest is a chest
hit (`a_shin_on_the_way_into_a_chest_is_a_chest_hit`).

Three things differ from §0's paragraph.

1. **A band again, not a limb.** `LIMB_BAND_M = 0.85` is the bottom half of
   the same cylinder. A cylinder cannot tell an arm from the chest it hangs
   beside, so what shipped is *legs*, and §0's own note that the reference's
   bow figures spread wider than a clean ×2/×1/×0.5 is the reason not to
   pretend otherwise: the ratio is theirs, the geometry is ours.
2. **A percent, not a multiplier.** `limb_pct = 50` beside
   `headshot_mult = 2`, because one `u16` column cannot say both "double"
   and "half". That gives §0's **per-weapon override** for free at both
   ends, in data, with the identities (1 and 100) meaning "opts out".
   The satchel carries both — a blast has no anatomy.
3. **The band pins it exactly.** `[bands] limb_pct` is checked for equality
   on every non-throwable row, because the TTK band is measured on the
   chest and is green whatever a leg is worth.

**What §9.4's own gate could not do, measured.** `balance.rs`'s equality
weakened to `<` survived the whole workspace: shipped content agrees with
the band by construction, so a comparison that only fires *below* it never
fires. `content`'s `the_body_part_ladder_refuses_what_it_names` hands the
loader a row that disagrees; it did not exist for `headshot_mult` either.

Gates: `tests/headshot.rs` (13 checks; eleven more mutants run, all caught
once that refusal row existed) and `content`'s two ladder rows.

### 9.5 · The rest, in the order a player notices

1. **No structure damage.** An arrow that reaches a wall stops dead.
   `EV_STRUCT_HIT` (20) already exists and `weapons.toml` already carries
   `structure = 1` on both bows, so this is a resolver change, not a
   protocol or content one.
2. **An arrow is as fat as a body.** `collide::blocked` bakes
   `CAPSULE_RADIUS_M` into its query, so an arrow threads a doorway but
   never an arrow slit. The honest fix is a radius parameter on `collide`;
   it is on `NOW.md` and it is a `sim-core` change with a replay-gate
   consequence, so it wants its own commit.
3. **No arrow recovery** (§5). We spend the arrow permanently, so our
   economy is harsher than theirs by design-accident rather than by
   decision. Needs the world-item archetype that dropped loot also needs —
   one lane, two payoffs.
4. **No draw** (§6). Our bow fires on `BTN_PRIMARY` at the weapon's cadence.
   A charge mechanic is a `Player` field, a hold, and a velocity scale; the
   durability-per-second cost is the part worth stealing and the part
   nothing in our schema can express yet.
5. **No damage falloff.** The schema has no curve field. `CONTENT.md` §1
   describes one; it has never existed.

### 9.6 · Numbers we could take, and the one we should not

`BALANCE.md` §6 standing instruction: a number with a reference equivalent
and no reason of ours to differ takes theirs and cites it. Candidates:

| ours | theirs | verdict |
|---|---|---|
| bow damage 30 | ~35 | **hold.** §5 — theirs is priced against 85 % arrow recovery and ours against none. Move the recovery loop first, then the number |
| `rate_per_min = 30` (2.0 s) | ~1 s draw + ~2.75 s reload ≈ 3.75 s | ours is fast. A real candidate once draw exists, because their number is *two* mechanics and ours is one |
| headshot ×2 | ×2 head, ×1 chest, ×0.5 limbs | **taken whole** (§9.4 + §9.4b, both 2026-08-30): ×2 head, ×1 chest and ×0.5 limbs are all live. The third part did exactly what this row predicted — the band stopped being sufficient and §7's ordering is built for real (`collide::Part`'s `Ord`). What is *not* taken is the per-part **arm**: our limb is a leg band on a cylinder, and their spread is wider than a clean ×2/×1/×0.5 anyway |
| HV arrow −20 % damage | −20 % | take it whole — but it is blocked on §9.3 |

The bands in `CONTENT.md` §4 still decide whether any of it may land;
`ttk_bow = [3, 4]` against `player_hp = 100` means a bow row must sit in
25–34 damage, so their ~35 **would not load** without moving the band.

### 9.7 · Arrow recovery: why we do not have it, and what it costs

⚠ **HALF BUILT, 2026-08-28 — read the four pieces below as a checklist and
not as a plan.** Pieces **1 and 2 landed** (arrow recovery v0): the store
is `crates/sim-core/src/spent.rs`, the break roll is `spent::breaks` keyed
exactly as item 1 proposes, and both numbers came out of §5 into
`content/balance.toml` as `arrow_break_pct` and `arrow_lodge_s`. It is in
`state_hash` and in the world save (`WORLD_SAVE_FORMAT` 10);
`crates/sim-core/tests/arrow_recovery.rs` is the gate.
Pieces **3 and 4 did not**, and item 4's instruction is the reason they
were left together rather than half-taken: no player can pick an arrow up
yet, so the economy consequence at the foot of this section is **not yet
paid** and §9.6's hold on the bow's damage row still stands.
`SpentArrows::take_near` is written and gated as the thing that verb will
call. The paragraph below is left in its original tense because it is the
argument that produced the decomposition, not a status line.

Asked directly by the operator on 2026-08-10 ("why cant we get arrows
back?"). The honest answer is that **nothing prevents it** — no wall, no
determinism problem, no trade. It is unbuilt. But it is not a small build,
and the reason is specific and worth stating so the next pass does not
discover it halfway through.

**The blocker is that every verb we have is addressed to a building grid.**
`ActionMsg`'s whole interaction set — `Use`, `Repair`, `Demolish`, `Throw`,
`Feed`, `Access` — takes `(cx, cz, level, loc)`, a cell in the build grid.
That is the right shape for a door, a box or a wall, all of which are
*placed at addresses*. An arrow lands at an arbitrary point on a hillside.
There is no verb in the protocol that can name it, and no store that can
hold it.

So recovery is four pieces, and only the first is small:

1. **A spent-arrow store** in `sim-core` — bounded like `Arrows`, in
   `state_hash`, holding position, item and the tick it landed. The break
   roll is already tractable: `rng::splitmix64` over the seed, tick and slot
   is deterministic, which is what the 15 % would need (§5).
2. **The lodge timer** — 10 s at `TICK_HZ` is 300 ticks, and the rule is
   theirs: an arrow that dealt damage waits, an arrow that missed does not.
3. ✅ **A pickup verb the wire can carry** (arrow recovery v1, wire v53).
   `ActionMsg::Pickup` — and the shape it set as a precedent is the one
   this section did not list: **payload-free**, neither a world position
   nor a store index, because `ActionMsg::Loot` had already answered the
   question. The sim re-derives the pick from the sender's own body, so
   there is no id to forge and nothing reachable through a wall. A
   deliberate press, as the operator's direction requires.
4. ✅ **A protocol bump** (52 → 53), and it did **not** ride with
   `EV_SHOT`. That advice was right about the cost and wrong about the
   blocker: `EV_SHOT` carries a muzzle speed and a drop that the client
   re-flies, a hitscan has neither, and refusing to invent a meaning for
   those fields was a live refusal in `ranged::hitscan`'s own doc rather
   than a scheduling matter. Bundling would have held *"arrows come back"*
   behind an unmade decision. One extra bump, paid knowingly — and it
   cost exactly the one predicted, because **§9.2 landed the next pass**
   (gun report v0, wire v54): `speed == 0` reads as *instantaneous* and
   the low `u16` becomes a reach in decimetres, so a firearm raises the
   event, the mixer gives it a hundred-metre report and the tracer
   declines to fly it. The flash and the beam are still unbuilt and owe
   no further bump. `DECISIONS.md` §open, "gun report v0".

**The economy consequence, stated because it is the reason to want this.**
Our arrow is spent permanently, so our ammunition is strictly harsher than
theirs — and §9.6 already refuses their damage number on exactly that
ground. Recovery is therefore not a comfort feature; it is the thing that
has to land *before* the bow's numbers can track theirs at all. That makes
it the highest-value item left in this section, and `NOW.md` carries it as
one slice with §9.2.

---

**Sources.** Tier 1: `reference/rust-systems.txt` (in tree, MIT). Tier 2:
Facepunch devblogs 104, 116, 118, 123, 140. Tier 3: `rust.fandom.com`
(Hunting Bow, High Velocity Arrow, Projectiles), `wiki.rustclash.com`,
`rusthelp.com`, `corrosionhour.com` (compound bow), Rustafied (fire/bone
arrows, 2018-01-11), the community AntiHack convar dumps, and
`commits.facepunch.com/248512`. Tiers 2 and 3 reached as search summaries
only — see §0.
