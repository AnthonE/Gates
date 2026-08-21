# reference/BUILDING.md — how the reference game decides who may build

Ripped facts, not design. `rust-systems.txt` answers *what systems exist*,
`SPAWN.md` *how the world gets placed*, `AUDIO.md` *what a player hears*,
`SAVES.md` *what survives a restart*, `DOORS.md` **who is allowed
through a door**; this file answers the sibling question one system over —
**who is allowed to build here, and what stops a base rotting**.

It exists because `DOORS.md` §9 kept pointing at it. The lock's remembered
list is the answer to "who may pass"; our hearth's claim is still
`owner == placer`, which is *lock v0's shape in the building system*. The
operator noticed (2026-08-08: *"a lot of this ties down to building rights
and toolchest"*) and that is exactly right — same defect, same fix, and the
reference solved them separately and differently.

Dated 2026-08-08; **§7b added 2026-08-10** and answers a second question
the first draft did not — *what may be built, and what does a shape cost*
— because the operator asked for the cost grammar next and it turns out to
be four ratios doing the work of a hundred numbers. **§7c added 2026-08-21**
and answers the third — *how high does a piece sit, and how did they decide*
— because build plate v1 shipped two numbers with no source and it turns out
they published both the answer and the experiment that produced it. §9 is the
part that changes what we build; items 11–15 are §7b's and 16–19 are §7c's.

⚠ **§7c already changed a shipped number the day it was written** (§9 item
16), which is the best argument this directory has for being read before a
knob is picked rather than after.

## 0 · Provenance — read this first

`DOORS.md` §0's three tiers, unchanged and ranked the same way:

1. **`reference/rust-systems.txt`** — in this tree, MIT, regenerable. A
   *hook* table, so what it proves is the **shape**. §1 reads the object
   model off it and nothing more, and it is the strongest section here.
2. **The developer's own devblogs**, by number and date. This system has
   an unusually good paper trail: Building 3.0 was a numbered, announced
   rewrite, so the *reasons* are published rather than inferred.
3. **Community wikis, guides and decay calculators** for numbers. Weakest
   tier, and weaker here than in `DOORS.md` — decay figures are the single
   most re-tuned set of numbers in that game.

**Same honesty note as `DOORS.md`, and it has since been half-repealed.**
When §§1–7b were written, this container's egress proxy refused every one of
those page fetches, so tiers 2 and 3 arrived as **search-result summaries of
those pages, not the pages**. Where two summaries disagreed I say so in the
text rather than picking (§6 has the one that matters).

⚠ **§7c is not like that: its devblogs were fetched whole** (2026-08-21, a
different container). `SOURCES.md` §0's rule is the reason to say so rather
than to leave the blanket sentence standing — reachability is a property of
the box, not of the hosts, so *probe* rather than trusting either claim. The
practical consequence is that §7c's two quotes are transcribed from the pages
and the rest of this file's are not, and a reader deciding how much weight to
put on a sentence should know which kind they are holding.

Nothing here was decompiled. Nothing ships.

## 1 · The object model, read off the hook table

```
BuildingPrivlidge     [4]  AddAuthorize · ClearList · RemoveSelfAuthorize
                           GetProtectedMinutes(Boolean)
AutoTurret            [3]  AddSelfAuthorize · ClearList · RemoveSelfAuthorize
VehiclePrivilege      [3]  AddSelfAuthorize · ClearList · RemoveSelfAuthorize
ItemModDeployable     [1]  OnCupboardAuthorize → OnDeployed(BaseEntity, BasePlayer)
(BaseEntity)          [1]  OnBuildingPrivilege → GetBuildingPrivilege(
                             OBB, Boolean, Single, BuildingPrivlidge)
ServerBuildingManager [2]  Merge(Building, Building) · Split(Building)
BuildingBlock         [8]  CanAffordUpgrade · CanChangeGrade · DoRotation
                           DoUpgradeToGrade ×2 · Hurt · SetWallpaper · RemoveWallpaper
DecayEntity           [3]  CanDemolish · DoDemolish · DoImmediateDemolish
Planner               [4]  CanAffordToPlace · CanBuild · OnConstructionPlace · OnEntityBuilt
```

Five structural facts, and they are the valuable half of this document:

1. **The authorized list is a reusable component, used three times.** The
   same three method names — add, remove-self, clear — hang off
   `BuildingPrivlidge`, `AutoTurret` and `VehiclePrivilege`, three classes
   with nothing else in common. Add the locks' remembered list from
   `DOORS.md` and the reference has **four** access lists built to one
   pattern. That is a component, not a coincidence.
2. **Privilege is a volume query, not a distance.**
   `GetBuildingPrivilege(OBB, …)` takes an **oriented bounding box** and
   returns the privilege covering it. You do not ask "how far is the
   cupboard"; you ask "is *this volume* privileged". §3 is the devblog
   that made it so.
3. **A building has identity, and buildings merge and split.**
   `ServerBuildingManager.Merge(Building, Building)` and `Split(Building)`
   — pieces belong to a named building, connecting two makes one, and
   breaking a connector makes two. Every per-building rule (one cupboard
   per building, upkeep for *this* base) rests on that identity existing.
4. **Placing a deployable can authorize you.** `ItemModDeployable`'s
   `OnDeployed(BaseEntity, BasePlayer)` fires `OnCupboardAuthorize` — the
   act of putting the cupboard down is the act of joining its list.
5. **Demolish belongs to `DecayEntity`, not to `BuildingBlock`.** It is a
   property of *anything that decays*, and there are two of it:
   `DoDemolish` (the windowed one a player asks for) and
   `DoImmediateDemolish`.

## 2 · The cupboard, and its list

- **Placing it authorizes you** (§1 fact 4). No separate step.
- **`E` toggles**: press once to add yourself, press again to remove
  yourself. **Hold `E`** for a radial menu carrying **Clear List**.
- **Vanilla caps the list at 10 players.** They bounded it; so must we
  (wall 4), and the reference having done the same is worth knowing before
  arguing about the number.
- **Authorization grants three things** that are usually stated as one:
  build inside the privilege, **pick up deployables** inside it, and be
  authorized to the traps that require building privilege (flame and
  shotgun traps).
- **One tool cupboard per building** (Building 3.0). This is the rule that
  needs §1 fact 3 to be expressible at all.

## 3 · Privilege was a sphere and became a volume (Devblog 185, Nov 2017)

The published change is one sentence: building privilege is **emitted by
the building blocks** instead of by an exact radius around the cupboard.

What it settles into, per the wiki and the guides:

- roughly **16 m** out from the cupboard along **connected foundations**,
  and up to **six floors** high;
- beyond the outermost connected block, roughly a **16 m cushion** in
  which nobody else may build.

The consequence is the part worth copying: **"building blocked" is a fact
about the base's footprint, not about where the cupboard is standing.** A
sphere centred on a point is the wrong shape for a building, and they
shipped that shape for years before saying so.

## 4 · Upkeep, and the bug that caused it (Devblog 189, Dec 2017)

Building 3.0's headline. The published reason is a **failure of the old
decay rule**, in their own framing: decay was suspended by *activity*, so
as long as a door was opened a base would not decay for days — and because
the check did not care whose door, **a neighbour opening theirs protected
yours**, indefinitely, after you had left the server. Nothing ever decayed.

The replacement is a **cost**, not a timer:

- The cupboard holds **24 slots** of upkeep materials and displays how much
  of a **24-hour** period it can cover.
- Upkeep pays a **fraction of what the base cost to build**, in the **same
  materials** it is built from.
- **It is per material.** If a base is stone and metal and the *stone* runs
  out, only the stone parts start losing health.
- Devblog 190 extended the protection: **deployables inside the privilege**
  stop decaying too, not just building blocks.

## 5 · Decay, when nothing is paying

Community numbers (tier 3 — treat as ratios). Time to gone, at full health,
with no upkeep:

| grade | decays in |
|---|---|
| twig | ~1 h |
| wood | ~3 h |
| stone | ~5 h |
| sheet metal | ~8 h |
| armoured | ~12 h |

Two shape facts under the table:

1. **The ladder is inverted against toughness on purpose** — the tougher
   the grade, the *slower* it rots, so upgrading buys time as well as hp.
2. **Damaged decays faster, proportionally**: a piece at 50 % health takes
   half as long.
3. **It eats inward.** The outermost pieces exposed to the environment go
   first, and it works toward the core until the base is gone.

## 6 · The grace window: demolish and rotate

The hammer's `E` menu carries **demolish** and **rotate**, both time-boxed
from the moment a piece is placed or upgraded.

**The sources disagree about the window and I am not picking a winner.**
One says a flat **10 minutes** to demolish after placing, plus 10 minutes to
rotate after an upgrade; another says **~10 minutes for foundations and ~30
for most other pieces**, and adds that where **no cupboard covers the
piece, anyone** may demolish it — not only its owner. That last clause is
the interesting one whether or not the number is right, because it is
`DOORS.md` §5's rule again: **unclaimed structure is anyone's.**

## 7 · The verb inventory, complete

The checklist §9 scores us against:

| # | verb | who | notes |
|---|---|---|---|
| 1 | place a piece | anyone with privilege, or where none exists | `Planner.CanBuild` |
| 2 | place a cupboard | anyone, on an unclaimed building | one per building |
| 3 | authorize self | anyone who can reach an unlocked cupboard | `E` |
| 4 | deauthorize self | anyone on the list | `E` again |
| 5 | clear the list | authorized | hold `E` → radial |
| 6 | stock the cupboard | anyone who can open it | 24 slots, 24 h readout |
| 7 | upgrade a piece | authorized, can afford | `CanChangeGrade` + `CanAffordUpgrade` |
| 8 | rotate a piece | authorized, inside the window | `DoRotation` |
| 9 | demolish a piece | authorized inside the window; **anyone** if unclaimed | `DecayEntity` |
| 10 | repair a piece | authorized | hammer, left click |
| 11 | pick up a deployable | authorized, inside privilege | §2 |
| 12 | raid it | anyone with explosives | privilege is not armour |

## 7b · The catalogue, and the cost grammar

Added 2026-08-10, from the operator's own reading of the current data
(*"we need to work on building more"*). **Provenance: tier 3.** These are
catalogue-and-price numbers of the kind §0 ranks weakest, arriving here
transcribed rather than fetched, and prices are the second-most re-tuned
set of numbers in that game after §5's decay figures. What makes them
worth writing down anyway is not the absolute numbers — it is that they
are **internally consistent to four ratios**, and a transcription error
does not usually survive that test.

### 7b.1 The catalogue is 20 × 5

Twenty structural shapes, each available in five grades — twig, wood,
stone, sheet metal, armoured — for 100 block/grade combinations. That is
the whole structural menu; everything else that looks like a building
piece is a **deployable inserted into one** (§7b.4).

| family | shapes |
|---|---|
| ground | square foundation · triangle foundation · foundation steps |
| horizontal | square floor · triangle floor · floor frame · triangle floor frame |
| vertical | wall · half wall · low wall · doorway · window · wall frame |
| circulation | ramp · L stairs · U stairs · spiral stairs · triangle spiral stairs |
| cover | roof · triangle roof |

Two footprint families — **square and equilateral triangle** — generate
all of it. Nothing is freeform, and that is the point: honeycomb, airlocks,
offset bunkers, rounded exteriors, shooting floors and external-cupboard
connectors are all *emergent from snapping those two footprints together*,
not authored as separate pieces.

### 7b.2 Grades: the price moves, the health doubles

| grade | paid in | full-wall cost | hp |
|---|---|---|---|
| twig | wood | 50 | 10 |
| wood | wood | 200 | 250 |
| stone | stone | 300 | 500 |
| sheet metal | metal frags | 200 | 1,000 |
| armoured | HQM | 25 | 2,000 |

**HP is a property of the grade alone.** A stone floor, a stone wall and a
stone triangle foundation are all 500. The geometry moves the *price* and
never the health — §7b.3 is what that implies and it is the most important
line in this document.

The ladder is × 2 at every rung (250 → 500 → 1,000 → 2,000), which is why
progression reads instantly to a player who has never seen a number.

### 7b.3 The grammar is four ratios, and one of them is a deliberate refusal

Normalise every shape against the full wall of its own grade and the whole
100-row table collapses to **four numbers**, holding across all five
grades (the HQM column rounds at small integers — 13/25 = 0.52 — and
otherwise it is exact):

| ratio | shapes |
|---|---|
| **1.0** | square foundation · wall · **half wall** · all four stairs |
| **0.7** | doorway · window |
| **0.5** | triangle foundation · square floor · floor frame · wall frame · low wall · steps · ramp · roof · triangle roof |
| **0.25** | triangle floor · triangle floor frame |

Two axes generate it. **Family price** — a piece that holds the base up or
walls it in is 1.0, an opening is 0.7, a horizontal surface above ground
is 0.5 — and **footprint**, where a triangle is half its square at the
same tier. The two compose: a triangle floor is ½ × 0.5 = 0.25.

**The generative rule is not volume, it is the socket.** Price tracks what
the piece *denies an attacker* net of what you still owe to close it:

- A wall denies everything and spends the socket → 1.0.
- A doorway denies everything **except a door-shaped hole you must buy a
  door for** → 0.7, and the door is a second purchase (§7b.4).
- A wall frame is mostly hole and needs a large insert → 0.5.
- A floor above ground is not what stops a raid coming in the front → 0.5.

Which explains the famous anomaly. **A half wall costs a full wall.** Two
stacked half walls are 600 stone where one wall is 300, and experienced
builders therefore do not casually stack them. It is not a mis-tune: a
half wall **spends the wall socket** — nothing else can go there — so it
is charged as a wall, and the discount you might expect for half the
geometry would make the half wall a cheaper full wall wherever a shooting
slot was wanted anyway. The 1.0 is the design refusing an arbitrage. The
low wall, which does *not* occupy a wall socket, is 0.5.

**And the ratio table has a 4× spread in defensive value per resource**,
because §7b.2's hp does not move with it. At stone: a wall is 300 for 500
hp (1.67 hp per stone), a triangle foundation is 150 for the same 500
(3.33), a triangle floor is 75 for the same 500 (6.67). That is the
economic engine under every base shape that game is known for — honeycomb
is not a trick players found in spite of the pricing, it is **what the
pricing pays for**, at exactly 2× and 4× a wall's rate. Cost varies by
shape, health does not, and geometry becomes the optimisation.

### 7b.4 Twig is a scaffold, and the two purchases are separate

Placement and grade are two acts. A **building plan** (20 wood) places any
shape, always **as twig**; a **hammer** (100 wood) upgrades it, and the
grade's cost is paid **in addition to** the twig already spent. So a wood
wall is 50 + 200 = 250 wood, not 200, and the model is:

```
terrain → twig structural skeleton → permanent material
```

Twig is the **editable draft**: 10 hp, ~1 h decay (§5), cheap enough to
lay out a whole base and re-lay it. It is the mechanism that makes the
§6 grace window mostly unnecessary — you find out the foundation is wrong
while it still costs 50 wood.

The **socket system** is the same separation one level down. A structural
frame holds a deployable insert, bought separately and destroyed
separately:

| socket | insert |
|---|---|
| doorway | wooden door 300 wood · sheet metal door 150 frags · armoured door 20 HQM + 5 gears |
| wall frame | sheet metal double door 200 frags · garage door 300 frags + 2 gears · armoured double door 25 HQM + 5 gears · shopfront |
| window | bars · glass · shutters · embrasures |
| floor frame | ladder hatch · floor grill |

Garage doors need a level 2 workbench and armoured doors a level 3; wooden
and sheet metal doors are default blueprints. **Wall grade and opening
grade are independent** — an armoured doorway may hold a wooden door — and
the attacker simply breaks whichever component is cheapest. Ownership is
not on the door either: a lock is a third purchase (key lock 75 wood, code
lock 100 frags), which is `DOORS.md`'s whole subject.

### 7b.5 Hard side and soft side

Every piece has an outside (hard) and an inside (soft) face, and the soft
face is dramatically more vulnerable to melee — stone especially. A base
built with its soft sides facing out is far weaker than its bill of
materials suggests. One rule, and it turns placement *orientation* into
skill expression for free.

### 7b.6 Upkeep is per grade, which makes a mixed base pay three bills

§4's per-material rule meets §7b.2's grades: a base whose shell is stone,
whose core is metal and whose loot room is armoured draws stone **and**
frags **and** HQM from one cupboard simultaneously, and running out of any
one of them rots only that grade's pieces. Structural blocks, doors and
window inserts count toward upkeep; loose interior deployables such as
sleeping bags generally do not, and high external walls inside privilege
are exempt. **A bigger base is not merely expensive to build — it is
expensive to keep**, which is the sentence that makes this a survival game
rather than a construction toy.

## 7c · Where a piece sits VERTICALLY, and what they tried first

Added 2026-08-21, and unlike the rest of this file **the two load-bearing
quotes here were fetched whole** — see §8's note on why that sentence needed
writing. It exists because build plate v1 shipped two numbers nobody had a
source for (`DECISIONS.md` §open), and it turns out the reference published
both the answer and the experiment that produced it.

### 7c.1 A piece snaps to a SOCKET, and the offset is half a wall

Placement is socket-based rather than addressed: a foundation exposes wall
sockets on its edges and a floor socket above, a wall exposes sockets on its
top and its vertical edges, and a piece must snap to one. The first foundation
of a base is the exception — it goes on open ground, and everything else grows
outward from it. (Tier 3.)

The vertical part is the half we needed, and Devblog 187 (Dec 2017) states it
directly:

> "I started by allowing foundations to snap to each other at a vertical
> offset of half a wall. Then I added half height walls that can be used to
> even out that offset on the higher floors."

Their wall is 3 m. So the snap offset is **±1.5 m, one symmetric number**, and
the half wall exists specifically to close the gap that offset leaves once you
build up from a stepped plate. Rustafied's Building 3.0 write-up adds the
limits without publishing them, which is itself worth knowing:

> "Both square and triangle foundations will snap at different levels to each
> other, provided the height is not too high or low off the ground."

Two limits, unstated, in a shipped game a decade old. Ours are `DECISIONS.md`
§open rows for the same reason theirs are not in a devblog: they are tuning.

Roofs occupy the wall socket at their bottom end, so a roof and a wall cannot
share a position — an anti-honeycomb rule riding the same socket mechanism
(Devblog 187).

### 7c.2 They tried the generous version and reverted it

This is the finding, and it is the one thing in this file that changed a
number the same day it was written. Devblog 85 (Vince), on `foundation.steps`:

> "I proposed the idea of using a three metre gradient for our
> foundation.steps block earlier in October. The goal was to ease building
> creation, so that you did not end up with walls ending at different heights
> as soon as you built on uneven terrain."

That is our problem statement in their words — the 2026-08-15 screenshots said
the same thing about our island. And they measured it:

> "We tested it, and while it worked perfectly for mountain bases where slope
> angle reach 45 degrees easily, building on flat became harder. Door blocks
> clipping were a tad nasty, too."

> "Ideally we'd have a foundation steps block that can adapt between 1.5m and
> 3m, but for the work required as of now it doesn't seem to be worth the
> while."

**A generous vertical allowance degrades the common case to serve the rare
one.** They kept the half-wall snap and a dedicated *stepped shape*, and threw
away the wide gradient. Note the second sentence too: the failure was not only
in feel — a 3 m gradient made *door blocks clip*, which is the geometry half
and the one a gate could have caught.

### 7c.3 Declining the snap is a mechanic, not an omission

Freehand placement — putting a block down without letting it take a socket —
is a real and widely-taught Rust technique, and it is where most of their
advanced base tech comes from: floor stacking, multi-TC layouts, bunkers,
bridge bases. (Tier 3, guides and tutorials; the exact input is not something
these sources state consistently, so this file does not.)

What matters for us is the shape of the claim: a build system with one snap
rule and no way out of it is strictly less expressive than theirs, and the
players who care most about building are the ones who notice.

## 8 · Sources

Tier 1 (in-tree, MIT): `reference/rust-systems.txt` — `BuildingPrivlidge`,
`AutoTurret`, `VehiclePrivilege`, `ServerBuildingManager`, `BuildingBlock`,
`DecayEntity`, `Planner`, `ItemModDeployable`, and the `OnBuildingPrivilege`
/ `GetBuildingPrivilege(OBB, …)` signature.

Tier 2 (developer devblogs, **via search summary — see §0**): Devblog 185
(Nov 2017, privilege emitted by building blocks rather than a radius);
Devblog 189 (Dec 2017, Building 3.0 and upkeep, with the door-opening decay
bug as its stated motivation); Devblog 190 (deployables protected inside
privilege).

Tier 2 also, and **fetched whole rather than summarised** (2026-08-21, §0's
⚠): Devblog 187 (Dec 2017 — the half-wall snap offset, half height walls, and
roofs taking the wall socket) and Devblog 85 (Vince — the three-metre
`foundation.steps` gradient, the test that killed it, and the 1.5–3 m
adaptive block they decided was not worth the work). Devblog 158 was fetched
too and carries only the intent (*"the possibility of half height snapping of
foundations and half height walls"*), which is useful for dating.

Tier 3 for §7c: Rustafied's Building 3.0 write-up (the "not too high or low
off the ground" phrasing, and the half wall's dimensions), and the community
freehand-placement guides for §7c.3 — where the technique is well attested and
the exact input is not stated consistently, so this file states the technique
and not the input.

Tier 3 (community wikis, guides and decay calculators, **via search
summary**): the 10-player list cap, the `E`/hold-`E` interaction, the 24
slots and 24-hour readout, the ~16 m / six-floor / 16 m-cushion figures, the
§5 decay ladder, and the §6 windows — where two sources disagree.

Tier 3 also, and by a **different route worth naming**: all of §7b — the
20-shape catalogue, the five grades, the 100-row cost table, the door and
lock prices, the workbench tiers, hard/soft sides. It came from the
**operator, 2026-08-10**, transcribed rather than fetched, stating it was
checked against current data. Treat it exactly as the rest of tier 3 and
no worse: the internal consistency in §7b.3 (100 rows, four ratios, five
grades) is stronger evidence of faithful transcription than any single
figure here, and the ratios are the part §9 acts on. The absolute prices
are the part to re-check before they are copied anywhere.

## 9 · What it means for us

Owned by `sim-core/deploy.rs` (the hearth, upkeep, decay) and
`sim-core/build.rs` (place, upgrade, repair).

1. **Our hearth is their cupboard, and its claim is lock v0's bug.**
   `Deploys::foreign_claim` is `h.owner == placer` — one id, no list, no
   way to share. It is the *same defect* we just fixed for doors, in the
   system that gates every other build verb: `build::place`,
   `build::upgrade` and `build::repair` all call it. This is the item.
2. **The auth list must be shared code, not a third copy.** The reference
   built one pattern and used it three times (§1 fact 1); we now have one
   in `lock.rs` and need a second. The bounded list, "refuse never evict",
   add/remove/clear, and the grant test should move to one module both
   call — a `crew.rs`, or `lock.rs`'s list generalised. Writing it twice
   is how the two drift into two different eviction rules.
3. **Radius versus volume, and we are better placed than they were.**
   Ours is a 24 m planar circle from the hearth's cell centre — the shape
   they shipped and then replaced for a stated reason (§3). But their fix
   needed an OBB query against physics; **ours would be a flood fill over
   the build grid**, because our pieces are already cells and already
   connected. That is deterministic, allocation-free with a bounded
   frontier, and gate-able. It is strictly cheaper for us than for them.
4. **It needs a building identity we do not have** (§1 fact 3). No `Merge`,
   no `Split`, no per-building anything. `build::collapse_from` already
   walks the support graph, which is the same graph a building id would be
   derived from — so the machinery half exists and is unnamed.
5. **Upkeep is close, and wrong in one stated way.** Ours charges per cost
   row from the first hearth in radius **that can pay the whole charge**,
   and an unpaid *piece* decays. Theirs is per material: run out of stone
   and only the stone rots. Ours is all-or-nothing, which makes a
   half-stocked hearth do nothing, and theirs makes it do half. Theirs is
   better and it is not more code.
6. **Our decay rate is flat and theirs is a ladder.** `DECAY_PCT_PER_PERIOD`
   is 5 % for everything; §5's ratios make tougher grades rot slower, so an
   upgrade buys time as well as hp. That is a **content** change
   (`balance.toml`, per material) and it is the cheapest real improvement
   in this document.
7. **Demolish and rotate do not exist here at all** (§7 verbs 8 and 9),
   and demolish is the answer to a question every player asks in their
   first hour: *I put the foundation in the wrong place.* It is also
   `DOORS.md` §9.10's missing pickup verb wearing a different coat — the
   two want one verb with a grace window and an "unclaimed is anyone's"
   clause, not two.
8. **Their decay bug is a warning we can still walk into.** §4's failure
   was a decay rule keyed on **activity** rather than **cost**, and it was
   gameable by a stranger. Ours is keyed on cost already, which means we
   inherited the fix without the bug — worth writing down so nobody
   "optimises" upkeep into an activity check later.
9. **What we should NOT copy**: wallpaper, `VehiclePrivilege` (no
   vehicles), the OBB query (item 3 — our grid gives it to us cheaper),
   and turret authorization (no turrets, though it is the third customer
   for item 2's shared list when it lands).
10. **Verb by verb**, §7's twelve scored: 1 place ✅ · 2 place a cupboard
    ✅ (hearth) · 3 authorize self ❌ · 4 deauthorize ❌ · 5 clear list ❌ ·
    6 stock it ✅ (`feed`, and a gift — anyone may stock) · 7 upgrade ✅ ·
    8 rotate ❌ · 9 demolish ❌ · 10 repair ✅ · 11 pick up a deployable ❌ ·
    12 raid ✅. **Five of the twelve are the same missing idea** — 3, 4, 5
    are the list, and 9, 11 are the windowed pickup.

⚠ **Items 1, 2, 5, 6, 7 and verbs 3/4/5/8/9/11 landed 2026-08-08/09** —
the crew list, the claim volume, per-material decay, demolish. `NOW.md`
§0aa is the live scorecard; read it before treating anything above as
outstanding. Items 3, 4 and 8 stand as written.

The rest of this section is **§7b's half**, added 2026-08-10.

11. **Our cost grammar had no shape economy at all, and theirs is the
    game. TAKEN WHOLE 2026-08-10 — the ratios first, then the
    absolutes.** Ours priced foundation, wall, floor and roof identically
    (350 wood / 350 stone / 200 frags), the doorway at 0.8 and stairs at
    0.57, so every shape bought the same hp for the same resource and the
    only reason to prefer one was that it was the shape that fit; theirs
    spans 4× (§7b.3). `content/building.toml` is now their grade base
    (twig 50 / wood 200 / stone 300 / metal 200) with §7b.3's ratios off
    it, which makes our 24 cells their 24 cells for the six shapes we
    have.

    **The part worth remembering is why it was off at all**, because
    nothing had noticed: our `cost` column was **never taken and never
    queued**. The 2026-08-08 balance pass took the hp ladder and the
    satchel out of this very file and left `cost` alone, and
    `RIPLIST.md` opened no row for it, so 350/350/200 stood as first
    written in the M1 build slice — derived from our own `farm_per_min`
    and compared against nothing. What exposed it was the **node take**:
    once a tree paid *their* 810 wood, a wall priced at *ours* cost
    **1.75× theirs in trees**, numerator theirs and denominator ours.
    That is `BALANCE.md` §4.1's false-familiarity trap one level out, and
    the general lesson is the one `RIPLIST.md` §1a already carries from
    the other side: **taking one half of a ratio is worse than taking
    neither**, because the halves are what a player feels and neither
    half alone is checked by anything.
12. **Twig is a mechanism and we do not have it.** We have one act where
    they have two: our `place` names a finished grade and pays for it
    outright, so a misplaced stone wall costs a stone wall. Adding a rung
    below wood is not a content row — it only means anything if `place`
    **refuses everything above it**, which is what makes the skeleton a
    draft and the hammer the commitment. It also gives `demolish`'s grace
    window (§6) a much smaller job, and gives our claim/upkeep system a
    grade it should deliberately **never** protect (§7b.4: twig is
    scaffold, so it always rots).
13. **Openings are already sockets here, and only doorways know it.** Our
    doorway takes a door deployable and the door has its own hp and its
    own lock — §7b.4's exact separation, built. The window and the wall
    frame are the same idea with the insert unbuilt, and they are the two
    cheapest catalogue additions we could make: `SHAPE_BITS` is 3 and we
    use 6 of its 8 codes, so **two shapes fit with no wire widening**.
    Their prices are already decided by §7b.3 (0.7 and 0.5).
14. **Triangles are the biggest single gap and the most expensive.** They
    are half the reason that game's bases look the way they do (§7b.1),
    and our grid cannot express one: `build.rs`'s cell holds one plane,
    one riser and two canonical edges, all square. A triangle footprint
    is a different address space, not a new row — so this is the one item
    here that is a **grid change**, and it should be costed as one rather
    than smuggled in behind the cheap items.
15. **What we should NOT copy, this half**: five grades (ours is three
    plus twig, and armoured wants an HQM economy we do not have), the
    workbench tiers gating door blueprints (`research.toml` is our
    answer to that question and it is a different one), and hard/soft
    sides (§7b.5) — which is a *good* rule we should want, but it needs
    a facing on every piece and an attack direction on every swing, and
    it is worth its own pass rather than a corner of this one.
16. **The plate offset is theirs now, and the asymmetric one we shipped
    first was worse on our own island** (§7c.1, `DECISIONS.md` §open
    "build plate v1"). Build plate v1 landed with `PLATE_RISE_MAX_BANDS`
    6 and `PLATE_SINK_MAX_BANDS` 2, reasoned from how the two directions
    LOOK — a plate over its ground is a leg, a plate under it is the hill
    through the floor. Devblog 187 says theirs is **one symmetric number**,
    half a wall, and `BALANCE.md` §6's default takes it without a case. It
    is also just better: swept over 1 598 buildable starts on the shipped
    seed, ±3 against 6/2 moves a whole 4×4 from 86.7% of starts to 91.3%,
    a 6×6 from 74.7% to 81.7%, an 8×8 from 62.1% to 70.8%, and **halves
    the deepest leg**, 3.0 m → 1.5 m. The sink is the knob that binds and
    the rise was buying nothing.
17. **We have no half wall, and a half-storey offset is exactly what
    creates the need for one.** §7b.1 already listed it as missing from
    our catalogue; §7c.1 is why it matters more now than it did then —
    their half wall exists *specifically* to even out the snap offset on
    the floors above a stepped plate. Ours has the offset and not the
    piece, so a base that steps carries the gap upward forever. Cheap:
    one shape code, and `SHAPE_BITS` is 4 with codes to spare since
    triangles.
18. **Do not widen the plate limits to fix a slope — add the stepped
    shape.** §7c.2 is a published, tested negative result on precisely
    the change that will keep suggesting itself: a wide vertical
    allowance helps mountains and hurts flats, and it made their door
    blocks clip. Our version of "a stepped shape" is a foundation whose
    top is a plate and whose footprint spans two bands, which is a
    catalogue row plus a shape code rather than a knob.
19. **Freehand is the open one, and it is bigger than it sounds**
    (§7c.3). Our `Command::Place` carries a cell address and no way to
    say "do not latch" — so a player cannot put a foundation at its own
    ground beside somebody else's plate, which is the first thing anyone
    tries on a slope. The cost is an action-lane bit and a UI decision,
    and the prize is the whole class of building their best players do.
    It is a *mechanic* question rather than a balance one, so it wants
    the operator rather than a measurement.
