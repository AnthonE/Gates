# reference/RIPLIST.md — the numbers we take, the ones we can't, and why

The queue for *"rip the reference's hard numbers now; tune them when we
have players"* (operator, 2026-08-09 — `DECISIONS.md` Spoken). One row per
number, so a pass can pick a row rather than re-derive the whole economy.

**This owns nothing.** `reference/BALANCE.md` §6 is the standing
instruction and `CONTENT.md` §4's bands still decide whether a number may
land. What this adds is the *worklist*: what is taken, what is outstanding,
what is blocked on research nobody has done, and what has no equivalent to
take. When a row lands, strike it here and cite the number at its own
`content/*.toml` row — the citation at the row is §6's requirement and this
file is not a substitute for it.

---

## 0 · The two rules that decide every row

**§6, rewritten 2026-08-10 and now the stronger of the two rules:** take
theirs, by default, with no case to make. A case is needed only to
*differ*, and the only admissible one is that the **mechanism** differs —
not effort, not a band, not source uncertainty, each of which §6.2 names
as a cost wearing principle's clothes. A split source never blocks a row:
§6.3's ladder breaks the tie and the row ships one number. Never average.

**The friction frame** (operator, 2026-08-09: *"rust is hardcore PvP,
your NEVER solo farming and people and animals are randomly killing
you"*). Every number in that game is priced for a world where farming is
contested, interrupted, and frequently lost — so a yield of theirs
carries friction we may not charge. The §4.1 trap has a second face
here: that section warns against copying a *value* onto a different
*mechanism*; this is the same error one level out. **A tree that pays
~460 wood is not generous in a world where you are shot off it. It is
generous here.**

**And §5's research says which friction actually dominates, which was a
surprise.** Decomposing the reference's own gather ladder: at-node
ceiling → best real farming is **~30×, and contains no threat at all** —
it is node density, travel, carry limits, deposit trips, node respawn and
smelt latency. Threat and contest are only **~2–5×** on top of that.
Logistics outweighs danger by an order of magnitude.

Against our own measurement that is damning, and the arithmetic is in
`tests/farmwalk.rs`:

| | ours, measured | theirs, researched |
|---|---|---|
| logistics divisor | **1.40×** (travel cost 29% of the walk) | ~10–30× |
| threat divisor | **1.0×** — nothing can kill a farmer | ~2–5× |

So the honest reading of the 20× gap is **not** that `farm_per_min = 50`
is wrong. Apply the reference's own decomposition to our at-node ceiling
of 1353/min and you land at 9–68/min — **50 sits inside that band.** What
is missing is not a corrected constant; it is the friction that would
make 50 the right answer. Our island is a farming paradise: trees dense
enough that a walker spends 29% of its time travelling, no carry limit
forcing a deposit trip, no scarcity, and nothing hunting it.

Three consequences a pass must hold:

- **Taking their yields onto this island makes our early game far faster
  than theirs, not equal to it** — because we would take the yield
  without any of the friction that priced it. That may be fine for an
  alpha with no population, but it is a decision and it goes in
  `DECISIONS.md`.
- **Logistics friction is the higher-value lever, and this list had it
  wrong.** Carry limits, node density and deposit trips are a ~10–30×
  term; mob→player damage is a ~2–5× one. Both are owed (§2 rows 5–6) —
  but the ranking is the opposite of what "hardcore PvP" first suggests,
  and the operator's frame is vindicated in *direction* while the
  magnitude sits somewhere else entirely.
- **Threat is not a flat rate multiplier and must not be modelled as
  one.** §5 is emphatic: in the reference it acts on *trip shape* —
  shorter trips, lower load factor, and a probability of losing the whole
  load — and contested ground actually pays **1.5–2.5× more** per hour,
  because players buy throughput with danger. A design that just slows a
  threatened farmer down is modelling the wrong thing.

---

## 1 · Taken (2026-08-08, cited at their rows)

Struck rows live in git; this is the standing list so a pass does not
re-litigate them. `BALANCE.md` §2 (matched before anyone tried) and §3
(moved on the balance pass) carry the sources.

| ours | theirs | where |
|---|---|---|
| player hp 100 | 100 | matched already |
| wooden door hp 200 | 200 | matched already |
| wood/stone/cloth stack 1000 | 1000 | matched already |
| building blocks 250 / 500 / 1000 | wood / stone / sheet metal | `building.toml` |
| satchel structure 125, body 475 | 4 satchels per stone wall soft side | `weapons.toml` |
| wooden spear 20 · stone tools 25 · metal tools 30 | theirs | `weapons.toml` |
| pig **80** hp, drops 5 raw meat | their boar — split 80/150, tie broken for 80 on 2026-08-10 (`BALANCE.md` §6.3 rung 3) | `mobs.toml` |
| hunger 500 · hydration 250 | theirs | `balance.toml` |
| cooked meat 50/3 · berries 10/20 · mushrooms 15/5 +3 hp | their feeds-vs-hydrates split | `consumables.toml` |
| `upkeep_pct_per_day = 10` | ~10% of build cost per 24 h (rising toward ~30% when the cupboard runs low) | `balance.toml` — **found matching 2026-08-09**; §4.1 filed upkeep as "a different mechanism, not a different rate", and the rate turns out to be theirs too. Their low-stock ramp is the part we do not have |

Two bands moved as arithmetic fallout, both spoken: `wall_breach_swings_min`
150 → 60, and the raid ratio re-pricing to 1.04/1.73/3.46.

## 1a · The node totals (2026-08-10) — row 1, and what it actually cost

Operator, 2026-08-10 (*"lets apply that"*, on the balance deltas these
documents imply). `DECISIONS.md` carries the call.

| node | theirs, best tool | ours before | ours now | conf |
|---|---|---|---|---|
| stone | 1000 | 300 | **1000** | EXACT |
| metal ore | 600 | 300 | **600** | EXACT |
| sulfur ore | 300 | 300 | **300** (unmoved) | EXACT |
| tree (large) | 867 metal hatchet | 300 | **870** | EXACT per tool |

And the ladder below the best tool, which is the §4.2 finding rather than
a constant — a worse tool draws less of the same node:

| | rock | stone tool | metal tool |
|---|---|---|---|
| stone node | 380 (theirs 375) | 790 (794) | 1000 |
| metal node | 250 | 490 (485) | 600 |
| sulfur node | 100 | 260 (257) | 300 |
| tree | 500 | 810 | 870 (867) |

Per-hit is `total ÷ 10` rounded, so each total lands within 1.3% of
theirs. `hand` rows have no reference equivalent and kept our own
relation to the rock.

**Three things this row taught, all of which contradict what it predicted
of itself.**

1. **`node_hits` did not have to move.** The row said both node bands
   break. Only `node_yield` did (400 → 1000). Their 15–28 hits are
   unobtainable rather than merely unfound (§4.1a), our schema needs only
   the total, so the hit count stayed at 10 and its band stayed put. A
   band you do not have to move is a band you should not move.
2. **It did NOT re-price `wood_wall_minutes` or any farm-minute anchor** —
   the row asserted it would, and that was wrong in a way worth keeping.
   Every cost anchor is priced off `[globals] farm_per_min`, never off the
   node's actual yield, so tripling what a node pays moved **no** anchor:
   `starter_minutes`, `satchel_minutes`, `wood_wall_minutes`, the three
   raid ratios and daily upkeep are all bit-identical across this commit.
   That decoupling *is* the latent defect `BALANCE.md` §4.3 named, seen
   from the other side: the economy prices farming with a declared number
   that the game's own yields cannot contradict.
3. **So the declared/at-node gap widened rather than closed** — ~24–68×
   before, ~24–135× after, and the farmwalk's measured effective rate went
   969 → 3927 wood/min while its **duty held at 71.6% to the decimal**.
   Duty measures the walk's shape, and a pure payout rescale cannot move
   it; that invariance is the evidence this was a clean scale move and not
   a pacing change. `farm_per_min` was deliberately left alone: §3 says it
   has no reference equivalent to take, and its semantics are the open
   operator knob. Tuning it here would have been tuning one unmeasured
   number against another, which §4.3 of `BALANCE.md` explicitly warns
   against.

**What did not come with the totals**, each for a stated reason: the flat
**2 HQM** on a metal node's finishing strike (no HQM tier to gate); the
proper-tool requirement on the finish bonus (§4.3b — a rock still finishes
a node for full value here, and that is a schema column plus a sim branch,
not a number); and the per-species tree spread (§4.1b — one `tree`
archetype, so ours is their *large* tree and the variance is a terrain
question).

---

## 1c · The craft and hp take (2026-08-10) — rows 1c + 1d, and a new source tier

⚠ **Provenance, and it is weaker than anything else in this file.** Every
page a number here could come from is **blocked by this box's egress
proxy** — `wiki.facepunch.com` and `rusthelp.com` both answer
`EGRESS_BLOCKED`, and the search layer returns summaries only. So the
operator pulled the table by hand **through a second assistant**, which
makes this a *summary of pages neither of us read*: one tier below
`BUILDING.md` §0's tier 3, which was at least our own search summary.
Call it **tier 4** and treat it accordingly — the numbers landed because
§6.1 says no case is needed to take, not because this source is good.

Two things partly redeem it and both are checks rather than trust:

1. **Five rows came back exactly equal to ours** — small wood box 100
   wood, large wood box 250 wood + 50 frags, metal hatchet 100 wood + 75
   frags, hunting bow 200 wood + 50 cloth, wooden door **hp** 200, small
   box **hp** 150 — plus the code lock at 100 frags, taken deliberately at
   lock v1. Independent agreement on rows nobody was trying to match is
   §6.3 rung 1 working: the source gets right the cells we can check.
   **It also says something about us**: parts of our content were taken
   from that game informally at some point and never recorded, which is
   the same shape as the metal wall already sitting at their 200 frags.
2. **Every band held with no re-speak** — six anchors, measured below.

**Egress re-probed 2026-08-11 (the box that took §1e), and it is closed
harder here**: the fetch tool answers `EGRESS_BLOCKED` for every host —
`example.com` included, so it is the tool's policy and not a Rust-domain
rule — the shell proxy 403s CONNECT outright, and only the search layer
and `raw.githubusercontent.com` work. The re-verify this section owes
still needs a box (or a human) that can open a page.

**I disputed two of its claims and was wrong about one of them, which is
the third check on this source and the least comfortable.** I refused
`rock` at "10 stone → 1, craftable" on the grounds that the rock is the
item you spawn holding — **the operator confirmed the source, 2026-08-11**
(*"you can make a rock for 10 stone. it lets a naked with nothing pick up
lose rock and make a rock"*), and ours moved 15 → 10. A tier-4 source beat
a confident prior; that is worth remembering the next time this document
is inclined to trust a reading over a reference. The tool cupboard's hp
was the other, and it went the other way — see §1c-b below.

### What moved

| row | ours before | theirs, taken |
|---|---|---|
| tool cupboard | 300 wood + 100 stone | **1,000 wood** |
| building plan | 50 wood | 20 wood |
| hammer | 75 wood | 100 wood |
| wooden door | 200 wood | 300 wood |
| sheet metal door | 200 frags | 150 frags |
| campfire | 100 wood + 50 stone | 100 wood |
| furnace | 200 stone + 50 wood + 25 lowgrade | 200 stone + 100 wood + 50 lowgrade |
| sleeping bag | 60 cloth | 30 cloth |
| workbench 1 | 500 wood + 100 **stone** | 500 wood + 100 **frags** |
| research table | 200 wood + 75 frags | 200 frags |
| stone hatchet · stone pickaxe | 100 wood + 50 stone | 200 wood + 100 stone |
| wooden spear | 120 wood | 300 wood |
| wooden arrow | → 3 | → **2** (same inputs) |
| metal pickaxe | 100 wood + 75 frags | 100 wood + 125 frags |
| crossbow | 150 wood + 75 frags + 2 rope | 200 wood + 75 frags + 2 rope |
| pistol ammo | 10 frags + 20 gunpowder → 4 | 10 frags + **5** gunpowder → 4 |
| gunpowder | 20 charcoal + 10 sulfur → 10 | 30 charcoal + 20 sulfur → 10 |
| torch | 30 wood + 10 cloth | 30 wood + 1 cloth + 1 lowgrade |
| bandage | 20 cloth | **4 cloth** |
| low grade fuel | 1 fat + 1 cloth → 4 | 3 fat + 1 cloth → 4 |
| burlap headwrap · shirt | 15 · 25 cloth | 10 · 20 cloth |
| **hp** sheet metal door | 800 | **250** |
| **hp** sleeping bag | 50 | 200 |
| **hp** large box | 250 | 300 |
| **hp** campfire | 100 | 50 |
| **hp** furnace | 300 | 500 |
| **hp** workbench 1 | 400 | 500 |
| **hp** research table | 400 | 200 |
| rock | 15 stone | 10 stone *(2026-08-11)* |
| research table | *(+ nothing)* | **+ 20 obol** — their 20 scrap *(2026-08-11)* |

### What was refused, and why each is a real reason

None of these is a cost wearing principle's clothes (§6.2); each is a
**mechanism or a missing item**, which is the one admissible ground.

- **metal spear · metal arrow** — no vanilla equivalent. Theirs are a
  *stone* spear (a wooden spear + 20 stone) and a *high velocity* arrow;
  ours are our own items at a rung theirs does not have.
- **road sign vest · medkit** — theirs run through leather, road signs,
  sewing kits and medical syringes. We have none of those items, and
  inventing them to hold a price is the tail wagging the dog.
- **satchel charge · revolver** — both are component chains we do not
  have (beancans and a small stash; a metal pipe). **Both already agree
  with theirs on the half we can express**: 240 gunpowder and 125 frags
  respectively, unchanged, which is more evidence for §1 above.
- ~~**research table's 20 scrap**~~ — **RETRACTED 2026-08-11, and it was
  never a real reason.** I refused it saying "we have no scrap, our
  currency is `item.obol` on purpose", which is a sentence that answers
  itself: **OBOL *is* scrap**, said in three places in this tree —
  `DESIGN.md` §3.1 is titled "OBOL — the working coin (the scrap)",
  `research.toml` and `cooking.toml` both carry the operator's own
  2026-08-10 quote. The row is now 200 frags + **20 obol**, whole. This is
  §6.2's failure mode caught in the act: not a mechanism difference, an
  item I did not recognise under our own name for it.
- **recycler** — not craftable there at all; it is a monument fixture.
  Ours is craftable by design and is the economy's arming point.
- **code lock hp** — theirs carries no standalone health because a lock
  is not independently destructible there. **My reason for refusing was
  wrong and the conclusion was right by accident**: I wrote "ours is a
  deployable with hp", and it is not — `deploy.rs` mints **no
  `DeployRec`** for `ARCH_LOCK`, in its own words *"a lock is not a
  deployable — it is a record about one"*. Our model already matches
  theirs exactly: you break the door, not the lock. What the row really
  found is that `deployables.toml`'s `lock_code` carries `hp = 100`
  that **nothing reads** — dead data on a required schema field, left in
  place and now labelled in the file.

### §1c-b · The cupboard is ours on purpose (operator, 2026-08-11)

The one cell where we now differ from them **deliberately**, and it is an
operator call rather than a research gap: *"make cupboard stronger rather
than weak. plus i think most people blow the locks off."*

Their tool cupboard is fragile — one summary put it at 100 hp — and it
survives by being buried behind everything else, which their own raid
meta bears out: the entry point is what gets blown, not the cupboard.
Ours goes the other way. **`item.hearth` hp 500 → 1,000**, the top of the
building ladder, so taking a base's privilege costs what another metal
wall costs. Derived rather than picked: 1,000 is `building.toml`'s metal
rung, already theirs, so the cupboard is exactly one more wall.

`DECISIONS.md` §open carries the row. It is filed here as a **difference
we chose**, not an outstanding take, so the queue stops listing it.

### The ×2 hypothesis is dead, and that is worth recording

The previous pass noticed four rows sitting at exactly half of theirs and
asked whether the whole file had been systematically halved. **It had
not.** The full table runs in both directions and at several magnitudes:
the stone hatchet is ½ theirs, the sleeping bag is **2×** theirs, the
bandage is **5×**, pistol ammo's gunpowder is **4×**, the metal pickaxe
is 0.6×, and the metal hatchet is exact. There is no transform — the
column was simply authored row by row against nothing, which is row 1b's
finding restated with a bigger sample.

### Measured, all six anchors, all in band with no re-speak

| anchor | before 1c/1d | after | band |
|---|---|---|---|
| starter base | 77.9 farm-min | **90.4** | — |
| satchel | — | 42.4 farm-min | — |
| stone raid ratio | 1.52 | **1.88** | [1.00, 3.00] |
| door breach swings | — | 50 | [30, 80] |
| daily upkeep | 6.92 | **8.17** | ≤ 15 |
| wood wall | 4.0 | 4.0 | [3, 5] |

The starter base rose 16 % on the cupboard alone, and the raid ratio rose
with it — a base that costs more to build is worth more to defend, which
is the anchor doing its job rather than a break.

## 1e · The stack-size take (2026-08-11) — row 1e's cheapest column, and an egress re-probe

**The probe first, because §2's row 2 promised one.** This box is *worse*
than the one that took §1c: the fetch tool answers `EGRESS_BLOCKED` for
every host including a plain `example.com` — so it is the tool's egress
policy, not a Rust-domain rule — the shell proxy answers 403 to CONNECT
for `wiki.facepunch.com`, `rusthelp.com`, `rust.facepunch.com` and
`corrosionhour.com`, and `gist.githubusercontent.com` is refused where
`raw.githubusercontent.com` (probed 200) is not. What works is the
**search layer**, so this take is **tier 3** — our own search summaries,
one tier above §1c's tier 4 and the tier `BUILDING.md` §7b landed at.
**§1c's re-verify therefore stays owed**: no page was read here either.

### What moved (5 cells)

| row | ours before | theirs, taken | conf |
|---|---|---|---|
| pistol ammo | 100 | **128** | EXACT — multiple independent summaries agree |
| wooden arrow | 100 | **64** | APPROX — ×64 stated for the HV arrow; the class shares it |
| metal arrow | 100 | **64** | our item (§1c refused its recipe); the class stack travels |
| bandage | 10 | **3** | APPROX — stated once, corroborated by a player petition to raise it *to 5* |
| gunpowder | 500 | **1000** | APPROX — one source states it; every other craft resource sits there |

**One forced edit rode along**: the spawn kit granted 5 bandages, a kit
entry may neither exceed the item's own stack nor repeat, so the kit
carries **3** now — `validate.rs` would have refused the boot otherwise.
No band or anchor reads a stack size, so nothing re-priced.

### What matched, now confirmed at the row

wood · stone · metal ore · sulfur ore · metal frags (all 1000 — §4.4 had
these already) · cloth 1000 (§1) · **low grade fuel 500** · **satchel
charge 10** · **obol = their scrap, 1000**. The §1 pattern again: rows
nobody was trying to match that agree anyway. Everything that stacks 1
here stacks 1 there (tools, weapons, wearables, deployables) — trivially
matched, uncited.

### What this box could not source, left open on purpose

Absence from this list was mistaken for a decision once (row 1b), so each
open cell is named: **animal fat** (ours 500; theirs likely 1000,
unconfirmed), **charcoal** and **sulfur** (ours 1000, likely matches,
unconfirmed), **gears / rope / tarp** (ours 50), **berries / mushrooms /
corn** (ours 50), **raw / cooked / burnt meat** (ours 20), **medkit**
(§1c's refusal extends — no equivalent item). Search snippets carry these
items' recipes and lore but never their stack fields, and the pages that
would answer all of them in one read (`lone.design`'s all-in-one list,
`wiki.rustclash.com`'s item pages) are exactly what a fetch cannot open.
A browser answers this in five minutes; the rows are cited open in
`items.toml` so the file stops reading as decided.

### Two sources discarded by §4.1's test, kept as receipts

- A Fandom **Item Cost List** summary offered "charcoal 42, sulfur 125,
  animal fat 375" as stacks — against wood and stone held EXACT at 1000.
  Crafting-cost quantities wearing a stack column's clothes; discarded.
- One summary offered "arrows stack to 10", read off a **Legacy**
  hunting-bow page, against the HV arrow's stated ×64. The staleness trap
  §4.2 names, seen in a stack column; discarded.

## 2 · Outstanding — the queue

Ranked by what a returning player notices, which is `BALANCE.md` §5's
order. **Status** is one of: `BLOCKED-RESEARCH` (we do not have their
number), `READY` (we have it and could land it), `NEEDS-MECHANISM` (the
number is meaningless until something else is built).

⚠ **`NEEDS-MECHANISM` is a BUILD ORDER, not a shelf** (operator,
2026-08-10, on finding that the "did not change" list was a list of
everything they wanted). Every row below is wanted; none is declined. The
status says what has to exist first, and that is a sequence — so a row
sitting here is a row nobody has started, never a row that was decided
against. §6.2 is the matching rule on the other side: a cost may be
recorded as a cost and never rewritten as a reason to differ.

⚠ **And a number that is NOT on this list has not thereby been decided
either** — which is the second half of the same lesson and cost a whole
column to learn (row 1b, 2026-08-10). `building.toml`'s `cost` had no row
here, no reason recorded anywhere, and read as settled purely by absence;
it turned out to be 1.75× theirs. Rows 1c, 1d and 1e exist because that
question was then asked of every other content file and **six of the
twelve had zero coverage**. Completeness of this list is now part of what
it is for: when you take a row, look for the column beside it.

| # | number | status | what it costs to take |
|---|---|---|---|
| 1 | ~~**gather yields / node totals**~~ | ✅ **TAKEN 2026-08-10** | Struck — see §1 and §1a. Totals are theirs, the hit count stayed ours, one band ceiling moved. |
| 1b | ~~**building block costs**~~ | ✅ **TAKEN 2026-08-10** | **This row never existed until the number was already taken, and that is the entry worth reading.** Our `cost` column in `building.toml` — 350 wood / 350 stone / 200 frags for every shape — was written in the M1 build slice off our own `farm_per_min` and **never compared to theirs**. The 2026-08-08 balance pass took the hp ladder and the satchel out of that very file and left `cost` alone; this list opened no row, so nothing was tracking it as outstanding and nothing read as wrong. Row 1's node take is what exposed it: once a tree paid *their* 810 wood, a wall priced at *ours* cost **1.75× theirs in trees**. Taken whole — grade base twig 50 / wood 200 / stone 300 / metal 200, `BUILDING.md` §7b.3's shape ratios off it — so the 24 cells are theirs. One band re-spoken (`wood_wall_minutes` [5, 9] → [3, 5], value 4.0) under §6.2/§7. **The lesson for the rows below**: taking one half of a ratio is worse than taking neither, and a row that is not on this list is not thereby fine — it may simply never have been looked at. |
| 1c | ~~**the `recipes.toml` cost column**~~ | ✅ **TAKEN 2026-08-10** — 23 of 39 rows moved, 5 already matched, 8 have no equivalent | Struck — §1c below has the row-by-row table, the provenance caveat, and the eight refusals. Headline: the **tool cupboard 300 wood + 100 stone → 1,000 wood**, the wooden door 200 → 300, the sheet metal door 200 → 150 frags, the building plan 50 → 20, the hammer 75 → 100. **Every band held with no re-speak.** |
| 1d | ~~**`deployables.toml` hp**~~ | ✅ **TAKEN 2026-08-10**, in the same commit as 1c by design | 7 of 12 moved, 2 already matched, 3 refused. The one that matters: **sheet metal door 800 → 250 hp**, which is what makes the door the breach point their design intends rather than a second wall. Wooden door 200 and small box 150 were already theirs. Not taken: the code lock (their lock has no standalone hp — it is not independently destructible, which is a mechanism difference from `lock.rs`), the recycler (no equivalent — theirs is a monument fixture, ours is craftable by design, `DECISIONS.md` "recycler v0"), and the **tool cupboard's own hp**, where the source declined to answer and one earlier search summary said 100 against our 500. That last one is the largest open cell on this list. |
| 1e | **the files with no coverage at all** — ~~`items.toml` stack sizes~~ ✅ **TAKEN 2026-08-11** (§1e) | `READY` (research not started on the rest) | `armor.toml` · `cooking.toml` · `loot.toml` · `research.toml` (scrap costs) still have zero coverage. Named here because row 1b proved that **absence from this list has been mistaken for a decision**. `armor.toml` has a real §4.1 reason (per-damage-type vs our flat %) and still deserves a row saying so; the other three have no reason recorded anywhere, which is not the same as having one. The stack-size half is struck: §1e took 5 cells at tier 3, confirmed 9 matches, and left 12 cells **explicitly open with the reason named** — those opens are part of this row's remaining work. |
| 2 | **per-material damage resistance** | `READY` (mechanism build, not a lookup) | The biggest *model* gap, and `BALANCE.md` §4.1 calls it a build: a schema column plus a sim multiply. Their stone wall takes 4 satchels and their sheet metal 23; ours takes 8 because one `structure` column serves every material. Until this exists, their raid numbers above stone cannot be taken at all — the ladder has nowhere to go. |
| 3 | **smelt rates** ✅ · **craft-time rebate** | smelt: ✅ **TAKEN 2026-08-10** · rebate: `NEEDS-MECHANISM` | Smelt landed via §6.3's ladder — rung 3 picked metal 2.5 / sulfur 2.5 over metal 3.3 / sulfur 1.7, and **the shape was the real win**: theirs smelt alike where ours had sulfur at half of metal, so sulfur went 1 → 2. Both rows sit at 2 because `seconds` is integer (row 3a). The mechanism half already matched — their furnace is parallel per slot and `oven::sweep` is too. **The rebate (50% one tier up, 75% two) is blocked on a ladder we do not have**: every crafted row in `recipes.toml` is `none` or `workbench1`, so there is no second tier for a rebate to key off. Build the tier ladder, then this is a lookup. |
| 3a | **sub-second smelt/craft precision** | `NEEDS-MECHANISM` (schema) | Their 2.5 s is not expressible: `Recipe::seconds` is a `u32` baked as `seconds × TICK_HZ`, so content can only say 2 or 3 while the sim happily runs 75 ticks. Widen the content field (tenths, or ticks outright) and the smelt rows can carry their real number. Small, self-contained, and it unblocks every future time that is not a whole second. |
| 4 | **the animal roster** | chicken/stag: `READY` · wolf/bear: `NEEDS-MECHANISM` (row 6) | Research is **closed**, not blocked — §4.6 has hp and drops for all five: chicken 25, boar 80 (taken), stag ~80, wolf 100, bear 400. The cost is one line of code per species, not a lookup: `mob.rs` holds `MOB_KINDS = 1` and a species ordinal, and its own comment says the array "is what makes a second a content row" — so add the ordinal, then the numbers are content. **Chicken and stag are landable today** (they flee, which is all our AI does). Wolf and bear are not worth adding until row 6 lands, because an animal that exists to threaten and cannot hurt you is scenery. |
| 5 | **logistics friction** — carry limits, node density, deposit trips | `NEEDS-MECHANISM` | **The largest single term in the reference's economy (~10–30×) and the one we charge almost none of** (§0: ours measures 1.40×). Not one number but a set: how far apart nodes sit (`terrain::scatter` density), whether a full pocket forces a trip home, and whether nodes are scarce enough to walk for. Nothing here is blocked on research — it is blocked on nobody having decided the island should be harder to farm. Until it moves, every yield we take from them lands in a world that charges a fraction of what theirs does. |
| 6 | **mob→player damage** | `NEEDS-MECHANISM` | Not a number. Ranked below logistics on the *magnitude* evidence (~2–5× against ~10–30×), which is the opposite of where this list first put it — but it is the half the operator asked for and the one a player feels, since a threat that cannot hurt you is scenery. Costs a new death cause on a 2-bit field saturated since wire v24, so it is a wire widening (wall 6: version bump + regenerated goldens in one commit). §5's warning applies: model it as trip shape and load loss, never as a flat rate multiplier. |

---

## 3 · No equivalent to take

Naming these stops a future pass hunting for a number that was never
theirs.

- **`[globals] farm_per_min`.** The reference has no declared farm-rate
  currency at all — and **vanilla has no gather-rate knob whatsoever**
  (§4.4; the familiar `gather.rate` is an Oxide *plugin*, not engine
  surface, which is worth knowing before anyone cites it as precedent).
  Their rate is the yield itself. Ours is a derived abstraction sitting
  beside the yield, which is exactly how the two drifted 20–40× apart
  with every gate green. It exists because we gate balance at
  content-load time with no playtest data — it is a substitute for the
  telemetry they have and we do not. Its semantics are the open question
  in `DECISIONS.md` §open.
- **`component_minutes`** (road-minutes for barrel drops). Same class:
  our pricing model, not their number.
- **The band system itself** (`CONTENT.md` §4). Theirs is a decade of
  live iteration. Ours is arithmetic over TOML because nothing is live.
- **Upkeep and decay mechanism, armour ladder, animal
  respawn/population.** `BALANCE.md` §4.1 — different mechanisms on
  purpose, so their values would be false familiarity.

---

## 4 · The gather research (2026-08-09) — what we have and what is missing

**Source posture upgraded 2026-08-09 (second pass).** The paragraph that
stood here said the proxy blocked the entire Rust web and that every
figure below was "a search engine's paraphrase of a page nobody opened."
A re-probe disproved it: `rust.facepunch.com/news` and `rusthelp.com`
serve full text. **Every devblog quotation in §4.3 is now read from the
devblog itself**, and §4.1's per-tool tables are read from a page, not a
summary. `SOURCES.md` carries the measured host map.

What did not change is the contamination warning, and it now has a
receipt — see §4.1. Confidence labels stay mandatory and are never
averaged — but a confidence label is not a licence to defer, and
§6.3's ladder now breaks a tie rather than letting a row carry two
numbers forever (which is what the boar did for two days).

### 4.1 · Node totals — vanilla 1×, best tool

| node | conf | theirs | ours |
|---|---|---|---|
| stone | EXACT | **1000** | 300 |
| metal ore | EXACT | **600** (+**2** HQM, flat, not chance-based) | 300 |
| sulfur ore | **EXACT — dispute settled 2026-08-09** | **300** | 300 |
| tree (wood) | EXACT per tool, banded per species | **1000** best tool on a large tree; species bands below | 300 |

**The sulfur dispute is closed, and the way it closed is the useful part.**
The 200 camp traces to one SEO page (`rustly.com`) — and that same page,
in the same table, calls stone **750** and metal **500**. Both are wrong
against figures we already hold as EXACT, and wrong in the same direction.
A source that misses two checkable numbers does not get to arbitrate the
third. The page that gets stone **1000** and metal **600** right
(`rusthelp.com`) says sulfur **300**, and inherits that credibility.
Generalise it: **score a source on the cells you can already check before
reading the cell you came for.** That test cost nothing and settled a row
that had been disputed for two passes.

### 4.1a · Per-tool totals — read from tables, not summaries

Ore, total per node (`rusthelp.com`; the three best tools tie on total and
differ only in time):

| tool | stone | metal | +HQM | sulfur |
|---|---|---|---|---|
| jackhammer | 1000 | 600 | 2 | 300 |
| salvaged icepick | 1000 | 600 | 2 | 300 |
| pickaxe | 1000 | 600 | 2 | 300 |
| stone pickaxe | 794 | 485 | 2 | 257 |
| salvaged hammer | 536 | 358 | **0** | 146 |
| bone club | 450 | 286 | **0** | 134 |
| rock | 375 | 250 | **0** | 100 |

Wood, total from one large tree (`corrosionhour.com`): rock **500**,
stone hatchet **810**, metal hatchet **867**, salvaged axe **1000**,
chainsaw **1000**.

**The HQM column is the finding, not the yields.** HQM is 0 for exactly
the three tools Devblog 166 says cannot trigger the finishing bonus — an
independent table, gathered years later, reproducing a devblog sentence it
never cites. That is the strongest corroboration in this document, and it
means §4.3(b)'s finish-bonus model is confirmed twice over.

**Nobody publishes hit counts any more, and that is a fact about the
mechanic.** Every current source gives *durations* (jackhammer 4 s →
rock ~1 m 4 s, roughly halved when hotspots are struck) and no hit counts
at all. Post-minigame, hits-to-clear is not a constant — it is a function
of how many marks the player hits. The 2017 table in §4.2 is not merely
old, it measures a quantity the game no longer holds fixed. **Do not go
looking for a modern hit-count table; it does not exist because it cannot.**

### 4.1b · Species and size variance (Tier-1 question 5, answered)

Wood total is set by species and size, where ore total is set by tool:
large beech/oak **1000** (max), smaller pine/palm **~¾**, fallen logs and
driftwood **~¼** (`rusthelp` bands them 500–1000 large, 250–750 medium,
50–200 sapling, 125–300 dead log). Biome is listed as a *location* for
nodes — Forest, Snow, Tundra, Desert, Jungle — with **no yield variation
by biome** stated for ore. Devblog 186 adds that small bare trees and some
palms have the **minigame disabled entirely** because you hit them in the
same place anyway — a species carve-out in the mechanic, not just a number.

**Their totals are known; their per-hit numbers mostly are not — and our
schema does not need them.** We declare `hits × yield_per_hit`, so a total
is takeable on its own by choosing our own hit count. That is what
unblocks §2 row 1 without the missing per-hit data.

What it costs: their stone node is **3.3× our 300**, and `node_yield` is
banded [250, 400]. Their hit counts run 15–28 against `node_hits` [8, 12].
Both bands move or the shard refuses to boot — a §7 look-at-the-band
moment, spoken in `DECISIONS.md`, in the same commit as the numbers.

### 4.2 · Tool scaling — the structural finding, stronger than any constant

**A worse tool extracts LESS of the same node** — the pool is not fixed
per node, it is fixed per node *and tool*. Stone pickaxe takes 794 of a
1000 stone node and 485 of a 600 metal node: **~0.8 of best**, and the two
independent measurements agreeing to within 0.014 is the most trustworthy
number in this whole document. A rock takes ~⅓.

Our `gatherables.toml` already produces this shape by another route
(fixed `hits`, per-tool `yield_per_hit`), so the *model* needs no change —
only the scale.

**One thing we cannot express, and should decide rather than drift:**
their ladder is two-axis. The metal pickaxe is *slower* per node than the
stone pickaxe (27.4 s vs 22.67 s) while yielding more; speed is bought
separately, by the salvaged icepick (14 s) and jackhammer. Our single
`yield_per_hit` row makes a better tool strictly faster AND richer.

Legacy per-tool wood, **2017, PRE-MINIGAME — the only hard hit-count
table that exists publicly, and it predates the mechanic that restructured
per-hit yield**: rock 28 hits/275 wood, bone club 34/309, **metal hatchet
16/459**, salvaged axe 15/750. That metal-hatchet row is the one
`BALANCE.md` §4.3 misattributed to the stone hatchet.

### 4.3 · Two mechanism divergences that outrank the numbers

**(a) Their marker pays SPEED; ours did too, as of 2026-08-09.** ✅ TAKEN
(operator: *"we need the marker to just be faster not more yield"*;
`DECISIONS.md` Spoken). A node holds `hits × HIT_UNIT` of budget, a
marked swing spends `HIT_UNIT + weak_spot_bonus_pct` of it and is paid
pro rata, so the total is invariant and the glint empties a tree in 7
swings instead of 10. The research that produced this is below.

**Devblog 170, now read from the devblog and not a summary** — the whole
marker model rested on a paraphrase of this sentence, so here it is
whole, typo and all:

> It should be noted that you will not actually earn more resources, but
> by using skill and good aim you can harvest the ore faster than just
> AFK spamming a node infront of you.

Ore hotspot is **150%** on that hit, stacking to a maximum of **300%**,
and *"if you miss, this bonus is reset to zero."* The 150/300/reset trio
is confirmed twice — the devblog and the current `wiki.facepunch.com`
Ore_nodes page, which still describes the mechanic in the present tense.
The hotspot is **invisible at night** without a light.

**Still unverified, and it should stop being quoted as if it were not:**
the metal hatchet's *16 wood/hit ramping +2 per mark hit, capped at 30*.
Devblog 186 says only that *"every time you hit one of the marks, your
gathering multiplier increases"* — no numbers. The 16→30 figures came
from a summary and no primary text has been found for them.

Ours *used to* pay 1.5× extra yield on the hit, which made a skilled
player richer where theirs makes one faster and everyone equally rich.
That was a pillar difference rather than a constant, and it was
load-bearing for us in a way it is not for them: the at-node ceiling gate
computed the bonus as extra yield. Both moved together — the ceiling is
now the invariant total over the fewest swings, 2030/min.

**(b) They have a finishing bonus — and now so do we.** ✅ TAKEN
2026-08-09 (operator: *"we need the finish bonus"*). Re-read from the
devblogs themselves 2026-08-09, and two details were wrong here:

- **The 20% is hedged in the original.** Devblog 166, verbatim: *"The
  final hit will yield a bonus of about 20% of the total, which is not
  only satisfying but should mitigate cherry picking."* **"about 20%"** —
  so `finish_bonus_pct = 20` is a *reading* of a hedge, not a taken
  constant. It is a fine reading; it is not EXACT, and this row should
  stop implying it is.
- **The tree split is Devblog 186, not 187.** Verbatim: *"You now receive
  half while harvesting and the other half as a finishing bonus."* Our
  `finish_bonus_pct = 50` is right; only the citation was wrong. Note it
  is a *finishing bonus*, not a fall-triggered payout — the fall is the
  tell (*"the tree will actually fall over now!"*), not the trigger.
  Devblog 187 is the *marker placement* pass (first X relative to first
  impact, later marks closer and always travelling one direction).

**And a rule we did not have: the bonus requires a real tool.** Devblog
166 — *"you can only receive this bonus with a proper tool. Bone clubs
and stones do not trigger it."* §4.1a's HQM column reproduces this
exactly. We do not model it, so a rock currently finishes a node for full
value where theirs pays nothing. That is a gap, and it is the cheap half
of a tool ladder.

HQM is available **only** as that final-strike bonus, at a flat **2**.
Both shares are taken as `finish_bonus_pct`: 20 on each ore node, 50 on
the tree, cited at their rows. It is a redistribution and never a bonus
on top, so a half-chopped node is what costs a player.

**Not taken, and worth naming**: HQM gated *only* to the final strike is
their sharpest version of this and we have no HQM tier to gate. If one
lands, the finish share is where it belongs.

**Respawn is not a timer.** No per-node respawn exists; population is
managed by a spawn handler, which corroborates `SPAWN.md` §3.5 exactly.
Verified defaults: `spawn.tick_populations` 60 s, `min_density` 0.5,
`max_density` 1, `min_rate` 0.5, `max_rate` 1, `player_base` 100,
`player_scale` 2. Ours is a 20–45 min per-slot timer — a different
mechanism, so their numbers do not transfer (§4.1 of `BALANCE.md`).

### 4.4 · There is no vanilla gather knob

Every "2×/5×" server runs an Oxide/Carbon plugin (GatherManager), not a
convar. The plugin exposes two *orthogonal* knobs, and the distinction is
the useful part: `gather.rate dispenser|pickup|quarry|survey <res> <mult>`
scales **what a hit pays** (node empties in fewer hits, total unchanged),
while `dispenser.scale tree|ore|corpse <mult>` scales **what the node
holds**. A correct 2× server sets both. Stack sizes confirmed at **1000**
for wood, stone, metal ore, sulfur ore and metal fragments — ours already
match. Teas raise totals (+20/35/50% ore, +200% pure wood); one 1000-wood
stack smelts ~200 metal ore / 400 sulfur / 100 HQM [APPROX].

### 4.5 · Closed 2026-08-09, and what is left

Closed by the second pass: **stone hatchet wood total = 810** (the number
`BALANCE.md` had wrong); **bare-hand/rock absolutes** (§4.1a, no longer a
ratio); **per-species tree totals** (§4.1b); **smelt, craft, animals,
upkeep** (§4.6).

Still missing, and two of these are now known to be *unobtainable* rather
than merely unfound:

- **modern per-tool hit counts** — **cannot exist.** §4.1a: the marker
  makes hits a function of aim, so current sources publish times. Stop
  looking.
- **whether the 0.8 tool ratio is an engine constant** — permanently
  unresolvable: confirming it needs decompiled source, which the IP rail
  forbids.
- **biome effects as numbers** — every source lists biomes as *locations*
  for nodes and none states a yield delta. Likely there is none for ore;
  unconfirmed for trees.
- **the metal hatchet 16→30 per-hit ramp** — quoted for two passes, no
  primary text (§4.3a).
- **the `oezp.at` encounter percentages** — §5.5, and the one fetch this
  box still cannot make.

### 4.6 · Tier 3 — the balance surface nobody had touched

All from current secondary sources, cross-checked where possible; none of
it is devblog-primary, so **APPROX unless marked**.

**Smelting.** Metal ore and sulfur ore **2.5 s each**, HQM **10 s** (a
second source says 3.3 / 1.7 / 6.7 — *recorded, not averaged*, per §0).
Wood cost ~**1.67 wood per ore** in a standard furnace, ~**0.33** in a
large furnace; the electric furnace burns none. Every wood burned has
~**75%** chance of 1 charcoal.

**The furnace is PARALLEL, and this was the contradiction to settle.**
Each slot smelts one ore simultaneously, so splitting 10 000 sulfur across
five furnaces costs the *same wood* and one-fifth the time. More furnaces
never cost more wood. Anyone modelling a furnace as a sequential queue
gets throughput wrong by the slot count.

**Craft times.** Gating is by workbench tier; the interesting part is that
a *higher* bench than required pays a speed rebate — **50% faster one tier
up, capped at 75% two tiers up**, and the player must stay in range for
the duration. Worked example: a revolver **10 s at T1 → 2.5 s at T3**.

**Animals** (HP / notable drops):

| animal | hp | drops |
|---|---|---|
| chicken | 25 | 2 chicken breast, 6 cloth, 12 bone |
| boar | **~80** | 8 pork, 40 fat, 20 leather, 10 cloth, 50 bone |
| stag | ~80 | 4–5 venison, 10 fat, 50 leather, 25 cloth, 50 bone |
| wolf | 100 | 5 wolf meat, 10 fat, 75 leather, 35 cloth, 40 bone |
| bear | 400 | 19 bear meat, 100 fat, 100 leather, 50 cloth, 150 bone |

**The boar reads ~80 here, and we shipped 150 — flipped to 80 on
2026-08-10.** §1 took 150 from a source that was *already* split 80/150,
so 150 was never better-evidenced, only earlier. This paragraph used to
end "`mobs.toml` should carry both until someone breaks the tie", which
sounded careful and was not: with no tiebreak procedure attached, its
only effect was to preserve the first guess indefinitely. `BALANCE.md`
§6.3 is that procedure now, and rung 3 decides this row — 80 sits in a
complete five-species table with consistent units, 150 in prose.

**Upkeep — the ramp we were missing.** ~**10%**/24 h while the cupboard is
above **50%** stocked, ramping toward ~**30%**/24 h below it: a **3×**
penalty for a thin stockpile, and the threshold is the half-full line.
`balance.toml` already matches the 10%; the threshold and the 3× are the
new half. Decay-once-empty, from `wiki.facepunch.com` and so the firmest
figure in this section: twig 1 h, wood 3 h, stone 5 h, metal 8 h,
armoured 12 h.

---

## 5 · How to execute one row

1. **Read the row's blocker first.** `BLOCKED-RESEARCH` means find the
   number, not guess it. `NEEDS-MECHANISM` means the row is not yours.
2. **Compute what breaks before editing.** `cargo test -p content` is the
   whole balance system; a band break refuses the shard's boot, not just
   the test. The anchors that re-price off any raw-material change:
   `starter_minutes`, `satchel_minutes`, `wood_wall_minutes`,
   `upkeep_daily_minutes`, all three raid ratios.
3. **If a band refuses the number, look at the band** (§7). Ask which of
   the two is stale. Either answer goes in `DECISIONS.md` the same day —
   a band that moves silently is what `CONTENT.md` §4 exists to prevent.
4. **One commit**: the numbers, the bands they force, the fixture updates
   in `crates/content/tests/content.rs`, and the `DECISIONS.md` row. A
   half-landed re-derivation bricks every gate that loads content.
5. **Cite at the row**, in the `.toml`, with the confidence label. §6's
   requirement and the only part of this that survives the file.
6. **Say what the threat frame does to it** (§0) if the number is a
   yield, a pace, or a cost — one line in the commit body is enough.

---

## 5 · The session research (2026-08-09) — what the friction actually is

Same posture as §4 and **worse**: this agent got **zero** page fetches —
every domain refused, and Reddit was unreachable by fetch *and* by
domain-restricted search, so the primary community source for this
question was simply unavailable. Every claim below is a search engine's
paraphrase.

**And the numeric layer here is actively contaminated.** It is dominated
by a cluster of SEO/affiliate server-host sites that contradict each
other — one was caught asserting both "8,800 sulfur = 2–4 hours of solo
farming" (2,200–4,400/hr) and "a 20-C4 raid ≈ 3 hours" (~14,700/hr), a
3–6× self-contradiction inside one source family. **Treat every
per-hour figure as order-of-magnitude at best.** What survives is the
*ordering*, which every source agrees on.

### 5.1 · The ladder — the finding that matters

| tier | sulfur/hr | what it includes |
|---|---|---|
| A · pure at-node ceiling | ~180,000 | tool speed × node total, nodes adjacent and infinite — the analogue of our farmwalk |
| B · best optimized real farming | ~6,000 | uncontested, routes known, transport |
| C · typical learned-route solo | 3,000–6,000 | decent tools + transport |
| D · rule-of-thumb solo | 2,200–4,400 | the "8,800 sulfur = 2–4 h" consensus |
| E · exposed contested session | 800–1,600 | the one figure explicitly conditioned on danger |

**A → B is ~30× and contains no threat whatsoever.** B → E is ~4–7×, and
that band is where danger lives. Hence §0's decomposition: **logistics
~10–30×, threat ~2–5×.** Confidence LOW on the magnitudes, MODERATE on
the ordering.

### 5.2 · Threat acts on trip shape, not on rate

Every mitigation the sources describe is about *shape*, not speed: don't
linger in the open with a full inventory, sprint to a monument and sprint
home, farm at night, "run short fuel loads so a lost fight never costs a
full hopper". And contested Tier-3 ground pays **1.5–2.5× more per hour**
than the safe route — players buy throughput *with* danger. So the
load-bearing effects are **reduced load factor per trip** and **a
probability of losing the whole load**, never a slower swing.

Death drops the entire inventory. The corpse persists ~5 min, then a
backpack whose despawn scales with the best item inside — ~10 min total
for a naked, up to ~65 min for top-tier gear, so **the better geared you
are the longer your recovery window**, which is a deliberate inversion
worth noticing. Ours already ships this shape (`death backpack v0`, base
5 min × rarity).

### 5.3 · Pacing, for comparison against `CONTENT.md` §3

| milestone | theirs | ours |
|---|---|---|
| first base (bag, door, cupboard) | ~10 min | — |
| Workbench 1 | ~1 h | — |
| starter base | — | 85.6 farm-min computed, ~45 min claimed |
| Tier 3 (the C4/AK gate) | <90 min speedrun · **3–5 h clan · 8–12 h casual solo** | — |

The solo:clan ratio for the same milestone is **~2–3×, and sublinear in
headcount** — a five-person clan gets there ~2.5× faster, not 5×. Also
recorded: their radiation-gated puzzle rooms now act as a contested-access
gate groups can hold and solos cannot, which is a *hard progression
ceiling* rather than a rate penalty — the clearest documented case of the
threat term changing what is reachable rather than how fast.

### 5.4 · Raid costs, the most reliable numbers in the report

Multiply attested and matching long-published constants: rocket **1,400
sulfur**, C4 **2,200 sulfur** + 20 tech trash; stone wall **2 C4 (4,400)**
or 4 rockets or 10 satchels; armored wall **4 C4 (8,800)** or 15 rockets.
Useful as a cross-check on our own raid ratio rather than as a lookup —
their sulfur is not our sulfur until §2 row 1 lands.

### 5.5 · Gaps, and one worth a fetch from another box

Nothing public gives a **time-split of a session**, a **deaths-per-hour**
figure, a **contested-vs-uncontested throughput ratio**, or **offline-raid
prevalence** — the four numbers that would settle the threat term
properly.

**The `oezp.at` paper is identified but still unread, and the reason
changed** (2026-08-09). It is Jan Byczkowski, *"The Potential for Survival
Games as a Research Medium in Political Science: Investigating the
Hobbesian and Lockean State of Nature in Rust"*, **Austrian Journal of
Political Science vol. 54 no. 2 (2025)**,
`oezp.at/OEZP/en/article/view/4231` (PDF galley `/download/4231/3257`).
It is **not** blocked by policy — every article URL on that OJS instance
exceeds 10 redirects, and `academia.edu` 403s. So it is a broken fetch,
not a denial, and a browser would very likely just open it.

Confirmed from the abstract, three independent search summaries agreeing:
in the game's anarchic environment — *which in certain aspects encourages
violence by lowering the stakes* — **players nonetheless favour
non-violent behaviour and defensive violence over offensive violence.**

**What is still missing is every number in it**: encounter count, sample
size, and the violent/non-violent and offensive/defensive percentages.
The direction of the finding is established and it cuts against a large
threat term; the magnitude is not. **Do not price a threat term off the
abstract** — the headline is qualitative and §5.2 already warns that
threat acts on trip shape rather than rate.

**r/playrust remains unavailable and is now known to be unavailable at the
tool layer**, not the network: fetches are refused before egress, so no
amount of open proxy fixes it. Vanilla-1× throughput, "how long to T3" and
solo-vs-group session shape are still entirely unsourced. A human with a
browser is the only route to both this and the paper — `SOURCES.md` §0
Tier 4 is where they belong, and both rows stand.
