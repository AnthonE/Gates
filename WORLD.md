# Gates · WORLD.md — the register, and what it would cost

**Owns the fiction. Owns nothing in `crates/`.** Written 2026-08-10 from two
operator conversations that were explicitly exploratory — *"help me think"*,
*"help me consider and log all of this"* — so nothing here is spoken and
nothing here may land on its strength alone. `DECISIONS.md` §open carries the
one row that points at this file.

**This is a roadmap, not a v1 specification** (operator, 2026-08-10: *"early
on we dont have to do this, it might be paths over time that evolve the
game"*). Nothing here competes with `DESIGN.md` §2's alpha or with pillar 1 —
the skeleton is still the product. What it does is decide *which direction*
the world grows when it grows, which is worth having early even though the
building is late (§9.1).

**It is also a ceiling.** Operator, same day: *"i think this is the max
deviation."* The sum of this document is the outer bound of how far Gates
moves from the survival game it is — not a floor to build up from. Classes,
levels, spells, quests, player-facing magic of any kind are outside it by
the operator's own line, and §7.1 is the rule that keeps them out.

Read it before proposing art, monuments, extraction rules, or a second health
pool. Its §8 is the useful half: five places this direction **collides with a
live gate**, two of them real.

The house style applies: where a claim is measured off the tree it carries a
`file:line`, and where it is taste it says so.

---

## 0 · The name is the fiction

The game has been called **Gates** since 2026-07-31, and the register that
fits it was named in one line: *"its called Gates so its some threshold
dimension… a broken place in time."*

That is the keystone, and it is better than "an island with ancient ruins on
it" for a reason that has nothing to do with taste: **it makes mechanics that
already exist legible, instead of needing new ones.** A fiction that explains
the game you already have is nearly free. A fiction that requires the game to
change is a content budget.

Everything below is already built or already designed, and the threshold
frame explains all of it at once:

| the mechanic | what the threshold makes it mean |
|---|---|
| you respawn on the beach forever | a threshold place does not let you leave by dying |
| the world wipes on a cadence | the gate cycles. A wipe is the world's own clock, not an admin act |
| the civilization is gone and the wilderness is wrong | something came through, and it is still here |
| **banked OBOL survives a wipe; carried OBOL does not** | what you *send out* leaves the broken place. What you carry stays in it (§4) |
| a ward, if one is ever built | what the threshold does to a body it keeps bringing back (§6) |

The last two are the ones that earn the frame. Extraction is the strongest
mechanic in this document and it exists *because of the name* rather than
being decorated by it.

One consequence for §3's catalogue: **The Severed Gate stops being one
monument out of twelve and becomes the thematic centre of the map.** If two
things are ever built, one of them is that.

### 0.1 · The risk register is low, and that is a real argument

Operator, 2026-08-10: *"some secret magitek from this civ isnt going to break
much."* That is correct, and §7.1 is why: the ancient tech is **environmental,
never inventory**. A world can be as strange as it likes as long as the
player's hands stay crude. Nothing in this document asks the sim for a spell,
a stat, or a character sheet — the deviation is entirely in what the world
*is*, and almost none of it in what a player *does*.

---

## 1 · The three registers, and the gradient between them

The map is not uniformly strange. It reads as a gradient from survivable to
catastrophic, and the gradient *is* the exploration content:

**coast → wilderness → ruins → corrupted interior → the disaster**

| register | what it looks like | what it is for |
|---|---|---|
| **The Living Coast** | blue water, green forest, oversized but recognizable plants, driftwood, stone | spawn. It almost seems survivable. This is where a naked lives |
| **The Lost Realm** | black obsidian, brilliant lapis, tarnished gold, waterfalls through ancient infrastructure | the mid game. Ruins are cover, landmarks, and loot |
| **The Interior** | volcanic glass, sickly green fissures, dead forest, floating geology, impossible sky | the end game. Where the disaster happened and the terrain is still breaking |

The first frame a new player sees should be almost normal, and the second
thing they do should be look inland at a mile-high broken arch. That
silhouette does more work than any amount of local detail, and it is cheap:
one enormous piece of geometry visible from everywhere buys the whole fiction
from every vantage on the island.

**The gradient is also load-bearing for §5** — it is what stops a hostile
default world from being unplayable on wipe day. Grade the world's condition
by register and the coast stays survivable while the interior is the thing
worth fixing.

**Our four biomes are `Beach · Meadow · Forest · Highland`** and
`biome(h, moist)` is a pure function of height and moisture
(`crates/sim-core/src/terrain.rs:246,263`). The gradient above needs a
**third input — distance from island centre** — and that is the single
cheapest structural change in this whole document (§9).

### 1.1 · The corruption needs our own name

*Fel* and *Outland* are Blizzard's words. The IP rail in `DECISIONS.md` is
narrow and about Facepunch, so this is not a rail violation — but adopting
another studio's proper noun wholesale is the same class of exposure for no
gain, and it costs one word to avoid.

**Proposed: verdigris** — the green that grows on copper and bronze as they
corrode. It is a real English word nobody owns, and it *explains itself*: a
civilization that seamed gold and alloy through everything it built would rot
green. That is a mechanism rather than a mood, which is the taste this repo
already has. The operator names it; `DECISIONS.md` §open carries the knob.
Until then this file says "the corruption" and means it literally.

---

## 2 · The one rule that makes the world work

**Treat the lost civilization like an actual civilization.** They needed
infrastructure, not temples. That single constraint is what stops a fantasy
survival map from reading as a theme park, and it is the strongest idea in
the whole direction:

| the reference game's | ours |
|---|---|
| water treatment | ritual hydraulic works |
| power plant | geothermal station |
| train yard | dimensional freight terminal |
| satellite dish | astronomical observatory |
| military tunnels | buried imperial arsenal |
| harbour | colossal black-stone shipyard |
| gas station | roadside pilgrim depot |
| supermarket | ruined bazaar / storehouse |

This preserves the thing the reference does exceptionally well: **you can
tell what a place was for by looking at it.** A player who works out that the
flooded chamber was a pump house, unprompted, is having the experience the
map is for.

The same rule scales down. Most ancient things should be small and
uncommented: black shrines, broken bridges, gold mile markers, watchtowers,
irrigation pumps, tomb entrances, collapsed caravanserais, statues whose
faces were deliberately chiselled away. Roadside density is what makes a
world feel inhabited-then-abandoned rather than decorated.

---

## 3 · The monument catalogue

Twelve named in conversation. Logged in full because the list is the point —
the *set* establishes the architectural language even if only two are ever
built. Each carries the reference-game analogue it is standing in for.

| monument | shape | analogue |
|---|---|---|
| **The Severed Gate** | two obsidian pylons across a canyon, shattered gold portal ring between them, debris still hanging | Trainyard / power plant — **and §0's thematic centre** |
| **The Black Ziggurat** | colossal stepped obsidian pyramid, top blown off, gold elevators, lapis conduits. Visible from half the map | Launch Site — **the hero silhouette** |
| **The Sunken Treasury** | half a city under the ocean, glowing lapis lines, air pockets in sealed chambers | *no analogue* — the ocean is currently scenery |
| **The Quarry** | spiral open-pit mine, black terraces, gold machinery, things crawling out of the bottom | Giant Excavator |
| **The Hanging Gardens** | ruined vertical city, roots, waterfalls, rope bridges, spores | vertical PvP; no clean analogue |
| **The Golden Spine** | ancient aqueduct/highway crossing miles of map, intact in sections | the road ring — *navigation, not a destination* |
| **The Crucible** | foundry inside a volcano; obsidian moulds, molten rivers, broken golems | smelting; partial Excavator |
| **The Hollow Colossus** | a fallen construct you enter through a wound in its armour. Black frame, lapis nerves, gold joints | dungeon; no analogue |
| **The Observatory** | mountain-top rotating gold rings around a black reflecting pool | satellite dish |
| **The Ossuary** | subterranean necropolis, thousands of obsidian niches, no natural light | Military Tunnels |
| **The Lapis Wells** | vast circular shafts descending to a subterranean lake, pumps still running | Oil Rig — *inverted* |
| **The Crown** | offshore island: seven obsidian towers around a gold spire, bridges between | Cargo Ship / Oil Rig, and the horizon's threat |

**Three of these are structurally different from the rest and worth naming
separately.** The Golden Spine is not a destination — it is the road ring
with a story, and it costs nothing extra because we already generate a road
(`sim-core/tests/road.rs`). The Black Ziggurat is not primarily a place — it
is a silhouette, and it pays for itself from every point on the island before
anyone can enter it. The Severed Gate is not primarily a monument — it is
where §4 happens.

---

## 4 · Extraction — what the haven is actually for

**The nearest-term idea in this document, and the one that most changes how
the game plays.** It is also the one that needs the least new fiction,
because §0 already supplies it: you cannot send coin out of a broken place
except through a gate, and a gate has to be opened.

### 4.1 · What already exists

`DESIGN.md` §3.1 has the whole seam built as design: carried OBOL is an item
that dies with you, banked OBOL is a ledger row that survives death and
wipes, and the only place the two convert is **the bank terminal at the
haven**, with a deposit fee **(knob: default 2%)**. `ALPHA.md` §2 stages it
at **A2** — A1 has no terminal at all.

So "extract your OBOL" is the bank terminal, and it is already the design.
What is new is a **gate on when you may use it**.

### 4.2 · The two proposals

Operator, 2026-08-10: *"i think you can only do it once every 24 hours. or
maybe even the server needs to work on activating something for the rest that
allows everyone to extract."*

They solve different problems and are not alternatives:

| | what it is | what it actually does |
|---|---|---|
| **per-player cooldown** | you may bank once per N hours | a **valve**. Stops the terminal being a save button, keeps carried OBOL genuinely at risk |
| **server-opened window** | the gate must be activated; while it is open, *everyone* may bank | a **convergence**. Puts 100 players on one route at a known time |

### 4.3 · The recommendation: guaranteed floor, contested ceiling

The window is the better mechanic and the more dangerous one.

Better, because it is the purest PvP driver in the design and it costs **no
PvE at all** — which is exactly the line the operator drew (*"im trying not
to push too much pve but also this is total pvp bait"*). Nothing has to be
fought. The event is that everybody who has been hoarding for two days now
has to *walk somewhere*, at the same time, carrying everything. The whole
island knows it. That is the tensest walk in the game, and `DESIGN.md` §3.1
already calls the run to the haven exactly that — the window just makes it
happen to a hundred people at once.

Dangerous, because if a dominant clan controls whether the gate opens, they
control everyone else's economy. That is politics for the top ten players and
misery for the other ninety.

**So split it.** The gate opens **on the world's own cycle** — a guaranteed
window nobody controls, on a posted cadence — and activation buys *more*:
an extra window, a longer one, a cheaper fee. Floor guaranteed, ceiling
contested. Nobody can be locked out of their own economy, and there is still
something worth fighting over.

Given a window, the per-player cooldown is probably redundant and definitely
should not be 24 h *on top* of it — two gates on one verb, and the second one
punishes short sessions and rewards logging in at the right hour. Pick the
window as primary; if a cooldown survives at all it should be short.

### 4.4 · Two rules to keep it from going wrong

- **The fight is the walk, not the terminal.** The haven is a no-damage zone
  (`DESIGN.md` §2) and it must stay one. An extraction window makes the
  *approach* and the *departure* lethal, which is correct, and puts nothing
  dangerous inside the safe zone, which is also correct. A window that turns
  the haven itself into a battlefield deletes the one place a new player can
  stand.
- **It is the same machinery as §5.** An opened gate is a world state:
  bounded table, tick expiry, in `state_hash`, broadcast on the wire. If
  extraction and world states are built as two systems, that is one idea
  paid for twice (§8.5).

---

## 5 · World states — and the world starts broken

The mechanic that changes what kind of game this is. Monuments are not loot
boxes with a card puzzle; they are **working infrastructure that players can
switch back on**, and switching one changes the whole shard.

### 5.1 · The inversion, which is the better version

The first pass of this design had players activating buffs that carried
downsides — *you get something, you unleash something*. The operator's second
pass inverted it: *"maybe the default state is debuffs are on and players
need to turn them off over the course of the reset."*

**That is strictly better, and it fixes the hole the first version had.**

The first version's problem was that griefing was the optimal play. Every
buff was local to whoever was prepared and every downside was global, so a
bored clan turning on the worst state hourly wasn't an exploit — it was
correct play. **You cannot grief with a debuff that is already the default.**

What the inversion buys on top of that:

- **The wipe cycle gets an arc.** Rust's cycle has no shape: day 1 and day 28
  are the same world with better guns. Here, wipe day is the world at its
  worst, mid-cycle is a contested repair project, and late cycle is a world
  that has been partly tamed — by players, visibly, and differently every
  wipe. That is a structure the genre does not have.
- **The conflict gets better, not smaller.** A repaired thing can be broken
  again. A restored monument is a PvP objective **that is not somebody's
  base** — which is exactly what a raid-heavy game is short of.
- **Politics for free.** Repairs cost resources and benefit everyone, so who
  pays? The clan that benefits most, while everyone else free-rides — and the
  clan currently winning has every reason to *keep* the world broken. Nobody
  has to script that; it falls out of the incentives.

### 5.2 · The two problems it creates, and their answers

- **Wipe day is now the hardest day, and that is when everyone is naked with
  a rock.** The answer is §1's gradient: **grade the world's condition by
  register.** The Living Coast is barely touched — the bootstrap has to
  survive, always — while the interior is unlivable. The repair project is
  about making the *interior* habitable, never about making spawn possible.
- **A fully repaired world is a boring late cycle.** Two answers and the
  second is better: repairs could decay (they are ancient machines that were
  broken for a reason, and it reuses upkeep's shape), or — better — **repair
  opens doors rather than removing threats.** Fixing the Wells shouldn't make
  the interior safe; it should make the bottom of the Wells reachable.

### 5.3 · The states themselves

Six sketched, and under the inversion each reads as *what is wrong* and
*what fixing it buys*:

| the broken thing | what it does to the world | what repairing it buys |
|---|---|---|
| **The Crucible** cold | smelting is slow everywhere; the volcano region is hostile | furnaces roughly double; high-tier ore surfaces |
| **The Observatory** dark | no navigation, night is genuinely blind | night vision, direction, rare falling fragments |
| **The Severed Gate** open | things wander far outside their range; the interior leaks | **it closes** — and while it is *held* open deliberately, extraction (§4) |
| **The Golden Spine** severed | the island is only as fast as your legs | fast travel — for everyone, including whoever is coming for you |
| **The Lapis Wells** dry | crops fail, collectors trickle, corruption clusters at water | farming and healing become viable |
| **The Black Ziggurat** awake | daylight is twilight, everything hostile is stronger | it sleeps — and the doors it holds shut across the map open |

The Ziggurat is the one that sells it either way round. A fresh spawn is
chopping a tree, the sky goes black-green, a horn sounds across the island,
and the screen says **THE ZIGGURAT HAS AWAKENED**. Under the inversion that
is the *starting* condition of a wipe rather than a clan's prank — which is
better, because it means every wipe opens with the whole server under the
same sky and the same problem.

That is the payoff: **the shard acquires a state.** Not "where is Launch
Site" — a map is learned once and then static — but *"why is the sky green?"*
*"Nobody's fixed the Ziggurat yet."*

### 5.4 · The engineering shape (so it does not fight the walls)

A global mutable modifier touching gather rates, spawn rates, lighting and
hostility is the most invasive thing in this document, and it only stays
legal as **one bounded table** — never as branches sprinkled through the
systems it modifies:

- **A fixed-size array of active states in `World`**, each a code plus an
  expiry **tick**. `limits.rs` gets `MAX_WORLD_STATES` and a stated overflow
  policy (wall 4). Refuse the activation, do not grow the table.
- **Codes, not strings** — no `String`/`format!` anywhere near the sim
  (wall 3).
- **Effects are content multipliers**, `content/*.toml`, validated at boot
  and hashed into the WAL header (wall 7). A world state that is a `match`
  arm in Rust is the version of this that ends the content wall.
- **Activation is a player command**, so it is in the WAL and replay is
  unaffected (wall 5). Expiry is a tick deadline; nothing reads a clock.
  **Under the inversion the default is not empty** — the wipe seeds the
  table, so the seeding is part of worldgen and must be deterministic.
- **The table is in `state_hash`.** A modifier that is not hashed is a
  determinism hole that shows up as prediction drift.
- **Broadcast is a wire change** — version bump + goldens, same commit
  (wall 6) — plus an event with a role gate.
- Any new interaction verb needs an arm in `render/verbs.rs` or the workspace
  is green twice and the Bevy gate is red (`CLAUDE.md`, the feature-gated
  match trap).

Built this way it is genuinely small: a table, a deadline, and a multiplier
lookup at ~6 call sites — and §4's extraction window is the same table.

---

## 6 · The ward — on the table, not decided

Operator, 2026-08-10: *"idk if we will even do the shields but its on the
table."* **Nothing else in this document depends on it**, which is worth
stating plainly, because the ward is the single item here that carries a real
gate collision (§8.2). If it never lands, the register is unaffected.

The idea: a regenerating shield over the health bar. Health does not
meaningfully regenerate; the ward does, after a few seconds without damage.
Combat then has two kinds of damage — **ward damage is pressure** (it costs
tempo and nothing else) and **health damage is attrition** (meds, cloth,
downtime). That converts the reference game's fight rhythm, `engage → hit →
syringe → peek → syringe → peek`, into `engage → crack → push → kill`. The
break should be audible and unmistakable, because the real product is a
shared vocabulary: **he's cracked, push him.**

The second thing it buys is a densely dangerous world that is not a tax: a
bite in the woods costs a ward and ten seconds, a *pack* that breaks it costs
real resources. That is what would let §1's interior be hostile without every
thirty-second encounter being resource misery — which matters more under §5's
inversion, where a hostile interior is the default condition.

### 6.1 · If it lands, it must not be loot

Everything else ancient is **a place you go, never a thing you carry**
(§7.1). The ward breaks that rule, so: it is issued by whatever puts you back
on the beach, it is **identical for every player**, and it is **not
upgradable, lootable, or craftable**.

Not flavour. A ward in the loot ladder is a stat, stats invite tiers, and
tiers are how a survival game becomes an MMO. Universal and fixed keeps it a
rule of the world — and it is the only version that stays legible in a fight,
because you always know exactly what the other player has.

### 6.2 · The non-obvious consequence

**Ward regeneration rewards disengagement, and a base is the best
disengagement tool in the game.** A player who can break line of sight behind
a door they own regenerates for free; a player caught in the open cannot. So
the ward quietly favours whoever already has a base near the fight — already
the stronger side. Worth measuring before it is tuned, not after.

### 6.3 · What it would cost in this tree

Cheap in the sim, expensive in the balance:

- `Player` gains `ward` + `ward_max` + a since-damaged tick counter. Integer
  only, no clock, no allocation — the easy half.
- Regen is a per-tick decrement against a deadline in **ticks** (wall 5).
- It is predicted state, so the quantize-both-sides law applies: the server
  sims on the values it transmits, or prediction drifts on the one number the
  player is watching.
- A ward-break **event** and a role gate for its payload
  (`crates/sim-core/tests/event_roles.rs`).
- Wire: version bump + regenerated goldens **in the same commit** (wall 6).
- **The balance collision is the real cost** — §8.2.

---

## 7 · Falling

**The tree has no fall damage.** Nothing in `crates/sim-core/` or `content/`
implements it; the only trace of falling in the sim is a terminal fall speed
used to bound a saved position (`sim-core/src/persist.rs:78`).

So "no fall damage" is not a removal — it is **the status quo**, and the
proposal is to *add* a mechanic: **a fall costs ward in proportion to height,
and never costs health.** A wall drop is nearly free; a monument drop lands
you alive and completely cracked. That is the decision the mechanic exists
for — jump off a tower to escape and you land with no defence; assault from
above and you arrive naked. Both are real choices with a price, which free
falling does not give you.

It also unlocks §3's vertical monuments as gameplay rather than scenery: the
Hanging Gardens can be 500 feet tall the moment a fall is survivable.

**It depends entirely on §6.** With no ward there is nothing for a fall to
spend, and the choice collapses to the status quo (free falls) or the
reference game's version (falls cost hp). Ranked accordingly in §9.

### 7.1 · The rule this belongs to

**Ancient technology is environmental, never inventory.** Every ancient thing
is a place you go, a machine you switch on, a route you take — never an item
in a bag and never a power on a character sheet. The ward is the sole
exception and §6.1 is why it is a safe one.

This is the rule that protects the operator's own framing: *the civilization
should be majestic, the players should still be rats*, and it is what makes
§0.1's "isnt going to break much" true rather than hopeful. The failure mode
of this entire direction is player-facing magic — a glowing sword, an ancient
armour tier, a spell. The moment a player carries the fantasy, the contrast
that makes the world work is gone and it is an MMO with building. **Player
gear stays crude forever. The world is what is magnificent.**

### 7.2 · What falling does to bases

Fall damage is load-bearing in the reference game's base design in a way that
is easy to miss: it is what makes roofs safe, what prices a failed jump, and
part of why vertical access is worth gating. Free falling means every base is
roof-accessible and every honeycomb is something to drop into.

The ward-cost fall answers it in the same motion — **you can drop into
someone's base, but you arrive cracked, inside their walls, next to people
who are not** — a fair trade rather than a hole. But it is a *building*
decision as much as a movement one, and `reference/BUILDING.md` should be
re-read against it before it lands.

---

## 8 · What this collides with, concretely

Five. **8.1 is an operator act and blocks every visual pass; 8.2 is a green
gate over a stale number.**

### 8.1 · The art rubric scores against the wrong world ⚠

`ART.md` is measured off the reference set — eighteen frames of a temperate
pine-and-granite island — and the loop's visual judge scores every pass
against `gates-loop/art/RUBRIC.md`, which lives **outside this repo and is
checksummed between passes** precisely so a builder cannot edit the criteria
it is graded on.

So the moment the world becomes obsidian and green fissures, **every visual
pass is scored as a defect by construction.** The builder cannot fix it, and
a pass that tries will circle — the exact failure `CLAUDE.md` warns about
under "a judge names the symptom".

The way out is already written down. `DECISIONS.md` 2026-08-01 set the target
as *"rip rust for now"* and said in the same row: *"Revisit when a concrete
palette is spoken; that changes the style section of the visual rubric only,
never the ten criteria."* This is that revisit. Three things, all operator
acts: a spoken palette; a reference set for the new register (the equivalent
of the reference set, because `ART.md`'s authority comes entirely from being
*measured*); and the rubric's style section updated to point at it.

**Until all three exist, no visual pass should chase this direction.** The
ten criteria — value separation, contact, density, no flat surfaces — are
about how a picture works and survive the change untouched.

### 8.2 · A ward would silently invalidate the TTK anchor ⚠

*Conditional on §6, which is undecided.*

`CONTENT.md` §4 anchor 2 declares TTK in body hits — melee 3–5, bow 3–4,
revolver 4–6, headshot ×2, armour worth at most +2 — and `test_content`
computes them against `content/balance.toml`'s `globals.player_hp = 100`,
the same number `combat.rs` plays.

Add a ward and every band measures a different quantity. The gate does not go
red — **it goes quietly wrong**, which this repo already knows is the worse
failure. The anchor must learn about effective hp in the same commit that
adds the pool, or a band reading "3–5 hits" describes a fight that takes
eight.

Two further consequences:

- **`reference/BALANCE.md` §6.2 permits it.** "Take theirs" is the default
  and only a **mechanism** difference justifies differing — a second
  regenerating health pool is exactly that, the admissible kind.
- But every damage number ripped so far was priced against 100 flat hp with
  no regeneration. Taking their numbers *and* adding a ward is taking half a
  system — the shape of `reference/RIPLIST.md` §0's threat frame, where a
  yield taken without the interruption that balanced it is a false
  familiarity.

### 8.3 · The gradient is a third input to a two-input function

`biome(h, moist)` is pure and takes height and moisture
(`terrain.rs:263`). §1's coast→interior gradient is a **radial** term, a
third input. The change is small and contained, but its blast radius is every
terrain gate: `test_terrain_golden`, the slot list, the spawn ring, the road
ring, the haven pad. Regenerated goldens in the same commit, and single-lane
— no other branch touching terrain that window.

### 8.4 · Monument placement is partly unbuilt

`TERRAIN.md` §7 shows the haven pad selector **finds** flat ground rather
than **carving** it — best natural site measured at 3.76 m of relief. Every
monument in §3 is a large flat footprint on generated terrain, so they all
want the carve that does not exist. The Golden Spine is the exception: it
rides the road, which is already generated and already gated.

### 8.5 · Extraction and world states are one system or they are two

§4's opened gate and §5's repaired monument are the same object: a bounded,
tick-expiring, hashed, broadcast world state activated by a player command.
Built separately — an extraction flag on the bank terminal here, a modifier
table there — that is one idea paid for twice, with two overflow policies,
two wire lanes and two places for a determinism hole. **Whichever lands
first should be built as the general table**, even if it carries exactly one
state.

The near-term consequence: the bank terminal arrives at **A2** (`ALPHA.md`
§2) and A1 has no terminal at all, so nothing is urgent — but if the terminal
ships with a bespoke gate before §5 exists, this collision has already
happened.

---

## 9 · Ordering, if any of it is spoken

### 9.1 · The one thing that is cheap now and expensive later

**The register is a decision to make early and build late.** Those are
separable, and the asymmetry matters: deciding costs a sentence today, while
*art made for the wrong register* has to be remade. Every prop, texture and
palette built for pine-and-granite between now and the pivot is work that an
obsidian world does not inherit.

So "we don't have to do this early" is right about the *building* and not
about the *deciding*. The cheapest possible version of this whole document is
the operator spending five minutes on §10's first three bullets and nothing
being built for months.

### 9.2 · The order

Roughly by cost. The first two are worth doing even if nothing else is:

1. **Name the corruption** (§1.1) and settle the register (§10). Free, and
   every other item reads better after it.
2. **The biome gradient** (§8.3). One function, regenerated goldens. It is
   the load-bearing half of the fiction — exploration that *changes* as you
   walk inland — and it needs no art, no wire, and no new system.
3. **The world-state table, with one state** (§5.4) — the table, the cap, the
   wire, the content multipliers, the expiry, the deterministic wipe-day
   seed. Ship the **Crucible**, because a cold furnace is a boring effect and
   the machinery is the interesting part. One state proves the system; six is
   content afterwards.
4. **The extraction window** (§4), on that table, when A2 brings the bank
   terminal. This is the highest gameplay-per-line item in the document and
   it is nearly free once 3 exists.
5. **The hero silhouette** — the Ziggurat as geometry on the horizon, no
   interior. Buys the entire register from every vantage; needs nothing from
   §8.4.
6. **The ward** (§6), if it is ever wanted, with the TTK anchor extended in
   the same commit (§8.2). Deliberately after 4: it is the only item with a
   real gate collision and nothing else depends on it.
7. **Falls cost ward** (§7). Nearly free once 6 exists; the sim already
   tracks vertical velocity.
8. **Monuments as places you enter.** Blocked on the terrain carve (§8.4) and
   the art direction (§8.1). Last, and correctly so.

---

## 10 · What must be spoken before any of this lands

Collected for `DECISIONS.md`; none of it is a number a loop may invent.

- **The register itself** — is the world obsidian-and-corruption, or does
  `DECISIONS.md` 2026-08-01's *"rip rust for now"* stand? Everything else is
  downstream of that one call, and §9.1 is why it is worth answering long
  before it is worth building.
- **The palette, the reference set, and the rubric's style section** (§8.1) —
  three operator acts, and the visual loop scores against the old world until
  all three land.
- **The corruption's name** (§1.1).
- **Extraction's gate** (§4) — window, cooldown, or both; the guaranteed
  cadence if there is a window; and whether activation buys extra windows.
- **Whether the world's default state is broken** (§5.1) — and if so, that
  the coast is graded out of it (§5.2), because that is what keeps wipe day
  playable.
- **Whether the ward exists at all** (§6), and if so its four numbers — pool,
  delay, rate, and the fall-to-ward curve — plus the rule that it is
  universal and never loot (§6.1). That last is a design rule rather than a
  number and is what decides whether this stays a survival game.
- **How many monuments the alpha carries**, against `DESIGN.md` §2's two.
