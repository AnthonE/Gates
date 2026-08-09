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

**§6, unchanged:** a number with a reference equivalent and no reason of
ours to differ takes theirs and cites it at the row. When we differ, the
row says why.

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
| logistics divisor | **1.37×** (travel cost 27% of the walk) | ~10–30× |
| threat divisor | **1.0×** — nothing can kill a farmer | ~2–5× |

So the honest reading of the 20× gap is **not** that `farm_per_min = 50`
is wrong. Apply the reference's own decomposition to our at-node ceiling
of 1373/min and you land at 9–69/min — **50 sits inside that band.** What
is missing is not a corrected constant; it is the friction that would
make 50 the right answer. Our island is a farming paradise: trees dense
enough that a walker spends 27% of its time travelling, no carry limit
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
| pig 150 hp, drops 5 raw meat | their boar (sources disagreed 80 vs 150) | `mobs.toml` |
| hunger 500 · hydration 250 | theirs | `balance.toml` |
| cooked meat 50/3 · berries 10/20 · mushrooms 15/5 +3 hp | their feeds-vs-hydrates split | `consumables.toml` |
| `upkeep_pct_per_day = 10` | ~10% of build cost per 24 h (rising toward ~30% when the cupboard runs low) | `balance.toml` — **found matching 2026-08-09**; §4.1 filed upkeep as "a different mechanism, not a different rate", and the rate turns out to be theirs too. Their low-stock ramp is the part we do not have |

Two bands moved as arithmetic fallout, both spoken: `wall_breach_swings_min`
150 → 60, and the raid ratio re-pricing to 1.04/1.73/3.46.

---

## 2 · Outstanding — the queue

Ranked by what a returning player notices, which is `BALANCE.md` §5's
order. **Status** is one of: `BLOCKED-RESEARCH` (we do not have their
number), `READY` (we have it and could land it), `NEEDS-MECHANISM` (the
number is meaningless until something else is built).

| # | number | status | what it costs to take |
|---|---|---|---|
| 1 | **gather yields / node totals** | **`READY` for totals** (§4.1) · `BLOCKED-RESEARCH` for per-hit | Their totals are now known — stone **1000** and metal **600** EXACT, sulfur DISPUTED 300/200, tree 500–1000 by prefab — and our schema needs only the total, since we declare `hits × yield_per_hit`. Breaks **both** node bands at once: stone 1000 against `node_yield` [250,400] is 3.3× our 300, and their 15–28 hits are outside `node_hits` [8,12]. Per §7 that is a look-at-the-band moment, not a refusal. Also re-prices `wood_wall_minutes` and every farm-minute anchor, so bands, yields and the re-speak land in ONE commit. `farm_per_min`'s ceiling gate catches the half that used to be silent. **Read §0 first** — taking a 3.3× total onto an island with a 1.37× logistics term is where "faster than theirs, not equal to theirs" actually bites. |
| 2 | **per-material damage resistance** | `READY` (mechanism build, not a lookup) | The biggest *model* gap, and `BALANCE.md` §4.1 calls it a build: a schema column plus a sim multiply. Their stone wall takes 4 satchels and their sheet metal 23; ours takes 8 because one `structure` column serves every material. Until this exists, their raid numbers above stone cannot be taken at all — the ladder has nowhere to go. |
| 3 | **smelt rates and craft times** | `BLOCKED-RESEARCH` | `BALANCE.md` §4.3: same `farm_per_min` dependency, smaller blast radius. §4.2 already retired the excuse ("no reason was ever given beyond inertia"). Craft seconds are ignored by the anchors by declaration, so this moves *play* without moving the anchors — the cheapest real row here once the numbers exist. |
| 4 | **the animal roster** | `BLOCKED-RESEARCH` + `NEEDS-MECHANISM` | Chicken, stag, wolf, bear all have roles there; we have a pig. Health and drops are lookups. The wolf and bear are `NEEDS-MECHANISM` — they exist to threaten, and nothing can hurt a player yet. |
| 5 | **logistics friction** — carry limits, node density, deposit trips | `NEEDS-MECHANISM` | **The largest single term in the reference's economy (~10–30×) and the one we charge almost none of** (§0: ours measures 1.37×). Not one number but a set: how far apart nodes sit (`terrain::scatter` density), whether a full pocket forces a trip home, and whether nodes are scarce enough to walk for. Nothing here is blocked on research — it is blocked on nobody having decided the island should be harder to farm. Until it moves, every yield we take from them lands in a world that charges a fraction of what theirs does. |
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

Same source posture as `DOORS.md` §0, and it must be read before any
figure here is used: **this box's proxy blocks the entire Rust web**
(`wiki.facepunch.com`, `rust.fandom.com`, `rustclash`, `rusthelp`,
`umod.org` all refused). Three GitHub raw files were genuinely fetched —
§4.4's plugin syntax and §4.3's `spawn.*` defaults are the *only* figures
below verified against primary text. **Everything else is a search
engine's paraphrase of a page nobody opened**, including every devblog
quotation. Two contamination cases were caught in the making: one SEO
site's sulfur figure propagating into several summaries and wearing
several hats, and a "tree guide" that turned out to be a different game.

Confidence labels are mandatory and never averaged (§0's rule, and why
our boar remembers both 80 and 150).

### 4.1 · Node totals — vanilla 1×, best tool

| node | conf | theirs | ours |
|---|---|---|---|
| stone | EXACT | **1000** | 300 |
| metal ore | EXACT | **600** (+2 HQM, DISPUTED 1–2 chance-based) | 300 |
| sulfur ore | **DISPUTED** | **300** (several sources, long-established) vs **200** (one SEO site, claimed July-2026 verification) vs ~250 | 300 |
| tree (wood) | DISPUTED | **500 / 750 / 1000** by tree prefab; "large trees ~650"; small sources (logs, driftwood) ~¼ | 300 |

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

**(a) Their marker pays SPEED; ours pays YIELD.** Facepunch, Devblog 170:
the node's overall yield does not increase — *"you will not actually earn
more resources, but by using skill and good aim you can harvest the ore
faster."* Ore hotspot is 150% on that hit, stacking to 300%, reset to zero
on a miss, moving but never more than ~a foot and never to the far side,
and **invisible at night** without a lit hat. The tree mark appears only
*after* the first hit, placed relative to the first impact, later marks
closer together and always travelling the same direction; a metal hatchet
ramps 16 wood/hit by +2 per mark hit, capped at 30.

Ours (`weak_spot_bonus_pct = 50`) pays 1.5× *extra yield* on the hit. So
**theirs makes a skilled player faster and everyone equally rich; ours
makes a skilled player richer.** That is a pillar choice, not a constant —
and it is load-bearing for us in a way it is not for them, because our
at-node ceiling gate (`balance.rs`) computes the weak bonus as extra
yield. Adopting their model changes that arithmetic. `DECISIONS.md` §open.

**(b) They have a finishing bonus and we have none.** The final strike on
an ore node pays ~20% of the node total, stated purpose *"to mitigate
cherry picking and leaving half finished nodes around the map"* (Devblog
166) — and HQM is available **only** as that final-strike bonus. A tree
withholds **half** its wood to the moment it falls (Devblog 187). We have
no equivalent, so a half-chopped node costs a player nothing here.

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

### 4.5 · Still missing

- **stone hatchet wood total** — absent from every table found; the exact
  number `BALANCE.md` had wrong
- **modern (post-2017) hit counts** per node per tool — sources give
  clear *times*, never hits, because the marker makes hits a function of
  aim
- **bare-hand yields** — only the rock's ~⅓ ratio, no absolutes
- **per-species tree totals** in current Rust; biome effects as numbers
  ("swamp trees yield less", never by how much)
- **smelt and craft times** per recipe (§2 row 3)
- **animal health and drops** beyond the boar (§2 row 4)
- **whether the 0.8 tool ratio is an engine constant** — unresolvable
  under this posture: confirming it needs decompiled source, which the IP
  rail forbids. Mark permanently unresolvable, not merely unknown.

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
properly. The single highest-value unfetched source is an *Austrian
Journal of Political Science* paper studying the Hobbesian/Lockean state
of nature in Rust — real methodology, and its headline finding (players
favour non-violent and defensive behaviour over offensive) **cuts against
a large threat term**. Blocked here; worth fetching from a box with open
egress.
