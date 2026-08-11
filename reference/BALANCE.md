# reference/BALANCE.md — the reference game's numbers, and which of ours match

Research **and** a standing instruction, which makes this file different
from every other `reference/*.md`: the rest own nothing, and this one owns
the answer to "where did that number come from".

Dated 2026-08-08, written on the operator's call: *"id like to balance the
game similar to rust so people dont get too lost when playing this the first
time."* That supersedes the posture `CONTENT.md` §0 carried for one day
("no table here is copied"). §6 is the part that binds; §4 is the honest
audit of what has *not* moved, rewritten after the operator asked why.

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
| hunger meter | 500 | 100 | **500** |
| hydration meter | 250 | 100 | **250** |
| cooked meat | ~50 cal, barely hydrates | 40 of 100 | **50 cal / 3 water** |
| berries | hydrate, few calories | 15 cal / 5 water | **10 cal / 20 water** |
| mushrooms | ~15 cal, heals 3 | 20 cal, no heal | **15 cal / 5 water / 3 hp** |
| boar meat drop | ~5 | 3 | **5** |

Every one of those passed the existing bands with two moved, both stated in
`DECISIONS.md`: `wall_breach_swings_min` 150 → 60 (a 250-hp wood wall cannot
take 150 swings from a weapon that can open a 200-hp door in 50), and the
satchel's `structure` 500 → 125, which is what "four satchels for a stone
wall" means in a model with one damage column.

The anchors that came out the other side: raid ratio **0.69 / 1.38 / 2.77**
(wood/stone/metal, band [1.0, 3.0] on stone), door breach **50** swings,
wall breach **63 / 125 / 250**, TTK 4–5 across every melee row.

## 4 · What has NOT moved — separated into real reasons and excuses

Rewritten 2026-08-08 after the operator asked the right question: *"explain
the random reasons we decided to roll our own numbers instead of borrowing a
10 year old game."* The word **random** was doing work, and it was earned —
the first version of this section listed six reasons as if they were six
design decisions, and on audit **three of them were cost dressed as
principle.** Those three are now separated out and two of them are done.

### 4.1 · Real reasons — a different model, not a different number

These would produce *false familiarity*: matching the number without the
mechanism behind it looks like a match and behaves like something else,
which is worse than plainly differing.

- **No per-material damage resistance.** Theirs scales incoming damage by
  what it hits, which is why a stone wall takes 4 satchels soft side and a
  sheet-metal wall takes 23. We have one `structure` column and
  `hp ÷ structure`, so our metal wall takes 8. Ordering right, early game
  right, ladder above stone compressed. **This is the biggest one and it is
  a build, not a decision**: a schema column plus a sim multiply.
- **The armour ladder.** Their protection is per damage type; ours is a flat
  percentage. Copying their percentages onto our model would mislead.
- **Upkeep and decay.** Their tool cupboard consumes resources scaled by
  building privilege radius. Ours is a different mechanism, not a different
  rate.
- **The animal roster** (`ANIMALS.md` §9). The boar's *health* is theirs;
  its respawn, population and dormancy are ours **on purpose** — their
  population re-rolls a death and the herd migrates over a wipe, and a
  stable world is a thing we chose.

### 4.2 · Excuses, now retired

- ~~**The survival meters.**~~ The reason given was "a bar's maximum is a
  display scale". That is true and it was not the reason: the real reason
  was that moving 100 → 500 forced rescaling every consumable and broke one
  of *our* bands (`best_food_min`, the rule that a forageable food must buy
  20 minutes). I kept our number to avoid confronting our band — and the
  band was the thing that was wrong, because the reference's food economy is
  **meat-centric** and forage is *supposed* to be marginal there. **Done
  2026-08-08**: meters are 500/250, consumables carry the reference's split
  (meat feeds and barely hydrates, forage hydrates and barely feeds), and the
  band now scans what an animal drops as well as what a node pays, which is
  what makes the pig the answer instead of a bush.
- ~~**Craft times.**~~ Listed as ours; no reason was ever given beyond
  inertia. Knowable and shallow. **Still to do** — see §4.3.

### 4.3 · Deferred with a stated cost, which is not the same as a reason

- ~~**Gather yields and node totals.**~~ **DONE 2026-08-10** — the scale
  is theirs now (`RIPLIST.md` §1a, `DECISIONS.md` Spoken). What follows is
  kept as it stood, because the paragraph after it is the finding that
  survived and the prediction inside it turned out to be **wrong in an
  instructive way**: moving the yields re-priced *no* anchor at all, since
  every anchor is priced off `farm_per_min` and not off what a node pays.
  Theirs: a tree is ~460 wood over ~16
  hits with a stone hatchet, a stone node 1000, sulfur 300. Ours are ~200
  over 10 hits and in the same *shape* but a different *scale*.
  ⚠ **The tree figure is misattributed and the correction is
  `RIPLIST.md` §4** (2026-08-09): 459 wood over 16 hits is the **metal**
  hatchet in the source table, and that table is from 2017 — it predates
  the tree minigame that restructured per-hit yield entirely. The stone
  hatchet's wood total is one of the numbers no public source records.
  This is not
  a lookup because `[globals] farm_per_min` is a **separately declared**
  number that the anchors price everything with — and nothing currently
  checks that it agrees with `yield_per_hit`, which is its own latent
  defect. Moving the yields without re-deriving `farm_per_min`,
  `node_yield`, `node_hits` and re-checking four anchors would leave the
  economy asserting one thing and playing another.
- **Smelt and craft times.** Same dependency, smaller blast radius.

**And checking that dependency turned up something larger, so it is written
here rather than left in a chat.** `[globals] farm_per_min` declares wood
and stone at **50/min**. The sim's own numbers say a tree pays 200 wood in
10 swings at the 38-tick cadence — **947/min standing at the node**, 1421
with a metal hatchet. Nineteen to twenty-eight times apart. The declared
rate is presumably meant to include walking between nodes, but nothing says
so and nothing checks it, so **every farm-minute anchor in the balance
system — `starter_minutes` 85.6, `satchel_minutes` 29.6, upkeep 8.56/day,
`wood_wall_minutes` 7.0 — is priced off a number with no measured relation
to the game.**

Worse, the anchors do not agree with the *prose*: `CONTENT.md` §3 targets a
starter base at **~45 min solo** and the computed anchor is **85.6**, 1.9×
over, with no band asserting either way. The pacing table is a claim nobody
has ever checked.

That is not a reference-alignment problem and it would exist if the
reference did not. It is the economy's own foundation being unmeasured, and
it should be fixed **before** the gather yields move, or the move will be
tuning one unmeasured number against another.

*(2026-08-09: the checking half landed — `balance.rs` refuses a declared
rate above the sim's at-node ceiling, weak mark included, and the travel
term is measured: `server tests/farmwalk.rs` walked 969 wood/min
effective at the kit hatchet — 71.6% duty against that tool's 1353/min
at-node ceiling, which is the node's whole payout over the fewest
swings that can exhaust it, not the 947 above. "Nothing checks it" above is kept as the finding
it was; the semantics speak and the yield move are what remain —
`DECISIONS.md` §open.)*

**Nothing is live** — no season, no wipe, no player holding a number in
their head from a shard of ours. So the cost of moving these is a
re-derivation and a red band, not a broken save. That is the operator's
point and it is correct: take more of the math now, tune later.

## 5 · What is still wrong for a returning player, ranked

1. **The boar does not fight back.** Theirs charges and flees under half
   health; ours only flees. This is the single biggest "that's not how it
   goes" moment left, and it is a mechanic (`ANIMALS.md` §9.5 item 3) —
   with a wire cost found since: the death-cause field has been saturated
   at 2 bits since v24, so it is a widening.
2. **No per-material resistance** (§4) — the raid ladder is compressed above
   stone.
3. **One animal.** Theirs has chicken, boar, stag, wolf, bear, each with a
   role. We have a pig. (The pig itself is no longer thin — it drops a
   corpse, snorts and trots as of 2026-08-09.)
4. **No radiation, no monuments, no scientists.** Out of alpha scope
   entirely (`ALPHA.md` §5), listed so nobody reads their absence as a
   balance decision.

## 6 · The standing instruction (rewritten 2026-08-10)

**Take theirs. That is the default and it does not need an argument.**
Operator, 2026-08-10: *"I REALLY REALLY REALLY REALLY REALLY WANNA USE
RUST NUMBERS."* The old wording — "no reason of ours to differ" — read as
neutral and was not: it put the burden on the person taking the number,
so every row needed a case made for it and any hesitation won by default.
The burden is the other way round now.

**6.1 · No case is needed to take. A case is needed to differ**, in the
row, in one sentence.

**6.2 · The only admissible reason to differ is that the MECHANISM
differs.** Theirs scales damage by the material it hits and we have one
`structure` column; theirs re-rolls population on death and ours is
seeded. Those are real. These are not, and §4 caught all three wearing
principle's clothes once already:

- *"It would break a band of ours."* → §7. Look at the band; a band is
  ours and can be the stale half.
- *"The sources are not certain enough."* → §6.3. Uncertainty picks a
  number, it does not defer one.
- *"It is more than a lookup / nobody has built the thing yet."* → that
  is a **cost**, and a cost gets a row in `RIPLIST.md` §2 with what it
  costs. Writing it here as a reason is how a deferral becomes permanent.

**6.3 · A split source is not a blocker, and "carry both" is not an
outcome.** Every row ships exactly one number. Walk this ladder and stop
at the first rung that separates the sources:

1. **Score each source on the cells we can already check.** Whichever
   gets the checkable cells right wins the cell we came for. This settled
   sulfur 300-vs-200 in a single pass (`RIPLIST.md` §4.1) after two
   passes of stalling — the page claiming 200 also called stone 750 and
   metal 500, both wrong against figures we hold EXACT.
2. **Source tier**: devblog primary text > wiki > SEO aggregator.
3. **The more complete artifact**: a full table with consistent units
   beats a single figure quoted in prose.
4. **Recency.** The game moves, and the newer reading describes the game
   a returning player just left.
5. **Still tied? Take one anyway and ship it.** Note the loser in the row
   so a later pass can flip it in one line. A tie is not a reason to keep
   our own invented number, which is what deferring actually does.

**6.5 · A band of ours yields to a number of theirs, by default.**
Operator, 2026-08-10: *"i think we should lighten our own math and lean on
them for now."* §7 below already said a band break is a reason to look at
the band; this makes the default explicit and puts the burden on keeping
our own arithmetic rather than on taking theirs — the same inversion §6
itself went through, one level up. Our anchors and bands stay: they are
the safety rail that catches a number our sim cannot express, and
`test_content` still refuses content that breaks one. What changes is who
wins a disagreement. **A band is ours, derived from our own declared
inputs, and it is no longer evidence about their number** — re-speak it in
`DECISIONS.md` and move on. `wood_wall_minutes` [5, 9] → [3, 5] is the
first one taken this way, and §7's story is why the rule was needed: the
band had been a fence around a number nobody had ever compared.

The one thing this does **not** license is re-speaking a band to admit a
number that our sim means differently — that is §6.2's mechanism test and
it is unchanged. Lighten the arithmetic, not the model.

**6.4 · What survives from the old rule, and it is the only part that was
ever right: never average.** 80 and 150 do not make 115. That is a number
nobody reported and no reading of the game supports, and false precision
is worse than picking a side. Pick one, say which, keep the other written
down.

The thing that keeps all of this from becoming "copy the spreadsheet" is
unchanged: `test_content`'s bands still decide whether a number may land,
so a reference value that does not fit our sim is refused here exactly as
an invented one would be — and then §7 asks which of the two is stale.

The rails are unchanged and they were never about arithmetic: no traced
art, no proper nouns, nothing decompiled (`ART.md` §7, `reference/README.md`).

## 7 · And the rule this file learned about itself

**"It would break a band of ours" is not a reason to keep a number — it is
a reason to look at the band.** Our bands encode *our* design rules, and a
rule written when the content was different can be the stale thing in the
comparison. The `best_food_min` band said a forageable food must be worth
crossing the island for; the reference disagrees, meat is the answer there,
and the band was quietly vetoing the operator's stated goal until someone
asked why.

So when a reference number is refused by a band, the question is which of
the two is out of date — and the answer goes in `DECISIONS.md` either way,
because a band that moves silently is exactly what `CONTENT.md` §4 exists to
prevent.
