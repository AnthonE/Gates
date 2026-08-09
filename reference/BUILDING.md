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

Dated 2026-08-08. §9 is the part that changes what we build.

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

**Same honesty note as `DOORS.md`**: this container's egress proxy refuses
every one of those page fetches, so tiers 2 and 3 arrived as **search-result
summaries of those pages, not the pages**. Where two summaries disagreed I
say so in the text rather than picking (§6 has the one that matters).

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

Tier 3 (community wikis, guides and decay calculators, **via search
summary**): the 10-player list cap, the `E`/hold-`E` interaction, the 24
slots and 24-hour readout, the ~16 m / six-floor / 16 m-cushion figures, the
§5 decay ladder, and the §6 windows — where two sources disagree.

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
