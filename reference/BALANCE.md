# reference/BALANCE.md — the reference game's numbers, and which of ours match

Research **and** a standing instruction, which makes this file different
from every other `reference/*.md`: the rest own nothing, and this one owns
the answer to "where did that number come from".

Dated 2026-08-08, written on the operator's call: *"id like to balance the
game similar to rust so people dont get too lost when playing this the first
time."* That supersedes the posture `CONTENT.md` §0 carried for one day
("no table here is copied"), and §9 is the part that binds.

## 0 · Provenance, and the limit on it

**Public sources only, and thinner than `ANIMALS.md`'s.** Community wikis,
raid calculators, server-host guides and datamining sites — all secondary,
none authoritative, several disagreeing with each other. `rust.facepunch.com`
and the wikis themselves are behind this container's egress proxy, so every
number below is quoted through search summaries of those pages rather than
read off them.

That is a real limitation and it decides how the numbers are used: **a
figure enters our content only when independent retellings agree and it is
one a player would actually notice.** Where sources disagree (the boar's
health is given as both ~80 and ~150) the disagreement is recorded here
rather than averaged away. Nothing was decompiled and no file was copied;
what crossed is a handful of integers, each cited at its row.

## 1 · Why match at all — the product argument, stated once

A player arriving from the reference game carries a table in their head: a
stone wall is 500, a satchel does not open it alone, a hatchet kills in four
hits, a boar takes a while. **Every number of ours that contradicts that
table costs them a death to learn**, and none of those deaths teach anything
about *our* game. Matching where matching is free buys attention for the
places we are deliberately different — the deterministic core, the slot
world, the animal roster that stays where the seed put it.

It is not "clone the spreadsheet". It is: do not surprise a returning player
about a number that was never ours to have an opinion about.

## 2 · What matched already, before anyone tried

Worth listing first, because it says the sim was in the right register:

| | reference | ours |
|---|---|---|
| player health | 100 | 100 |
| wooden door | 200 | 200 |
| wood / stone / cloth stack | 1000 | 1000 |
| melee TTK band | 4–5 hits unarmoured | `ttk_melee = [3, 5]` |

## 3 · What moved to match (2026-08-08)

| | reference | ours before | ours now |
|---|---|---|---|
| wood building block | 250 | 750 | **250** |
| stone building block | 500 | 1750 | **500** |
| metal building block | 1000 (sheet) | 3000 | **1000** |
| satchels per stone wall | 4 (soft side) | 1 | **4** |
| satchel body damage | 475 | 500 | **475** |
| rock | 20 | 20 | 20 |
| wooden spear | 20 | 25 | **20** |
| stone hatchet / pickaxe | 25 | 22 / 20 | **25** |
| metal hatchet / pickaxe | 30 | 28 / 25 | **30** |
| metal spear (their stone spear) | 30 | 30 | 30 |
| boar | ~150 | 80 | **150** |

Every one of those passed the existing bands with two moved, both stated in
`DECISIONS.md`: `wall_breach_swings_min` 150 → 60 (a 250-hp wood wall cannot
take 150 swings from a weapon that can open a 200-hp door in 50), and the
satchel's `structure` 500 → 125, which is what "four satchels for a stone
wall" means in a model with one damage column.

The anchors that came out the other side: raid ratio **1.04 / 1.73 / 3.46**
(wood/stone/metal, band [1.0, 3.0] on stone), door breach **50** swings,
wall breach **63 / 125 / 250**, TTK 4–5 across every melee row.

## 4 · What deliberately did NOT move, and why

This is the more useful half of the file.

- **The survival meters stay 100 / 100.** Theirs are 500 calories and 250
  hydration. A bar's *maximum* is a display scale — nobody gets lost because
  the number is 100 — and matching it would have forced rescaling every
  consumable, which broke a design band (`best_food_min`, the rule that a
  forageable food must be worth crossing the island for) for zero
  player-facing familiarity. What a player actually feels is the drain rate
  and the ratio, and those are ours and banded.
- **Per-material damage resistance does not exist here, and it is the
  biggest structural gap.** Theirs is why a stone wall takes 4 satchels soft
  side and a sheet-metal wall takes 23 — damage is scaled by what it hits.
  We have one `structure` column per weapon and `hp ÷ structure`, so our
  metal wall takes 8 where theirs takes 23. The *ordering* is right and the
  early-game numbers are right; the late-game raid ladder is compressed.
  Closing it is a schema column plus a sim multiply, and it is filed rather
  than faked.
- **Gather yields and smelt rates stay ours.** They drive the farm-minute
  anchors (`node_yield`, `wood_wall_minutes`, `upkeep_solo_daily_max_min`),
  so moving them is a re-derivation of the whole economy rather than a
  lookup. Ours are already in the same register.
- **Craft times, upkeep, and decay stay ours.** Same reason, and their upkeep
  system (a tool cupboard consuming resources per building-privilege radius)
  is a different mechanism from ours, not a different number.
- **The armour ladder stays ours.** Their protection model is per-damage-type
  and ours is a flat percentage; matching the numbers without the model would
  read as matching and behave differently, which is worse than differing.
- **The animal roster is ours** (`ANIMALS.md` §9): the boar's *health* is
  theirs, its behaviour, respawn and population are not.

## 5 · What is still wrong for a returning player, ranked

1. **The boar does not fight back.** Theirs charges and flees under half
   health; ours only flees. This is the single biggest "that's not how it
   goes" moment left, and it is a mechanic (`ANIMALS.md` §9.5 item 2).
2. **No per-material resistance** (§4) — the raid ladder is compressed above
   stone.
3. **One animal.** Theirs has chicken, boar, stag, wolf, bear, each with a
   role. We have a pig.
4. **No radiation, no monuments, no scientists.** Out of alpha scope
   entirely (`ALPHA.md` §5), listed so nobody reads their absence as a
   balance decision.

## 6 · The standing instruction

**When a number has an equivalent in the reference and no reason of ours to
differ, take theirs and cite it at the row.** When we differ, the row says
why. That is the whole rule, and the thing that keeps it from becoming
"copy the spreadsheet" is the second half: `test_content`'s bands still
decide whether a number may land, so a reference value that does not fit our
sim is refused here exactly as an invented one would be.

The rails are unchanged and they were never about arithmetic: no traced
art, no proper nouns, nothing decompiled (`ART.md` §7, `reference/README.md`).
