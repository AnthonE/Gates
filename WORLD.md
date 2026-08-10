# Gates · WORLD.md — the register, and what it would cost

**Owns the fiction. Owns nothing in `crates/`.** Written 2026-08-10 from an
operator conversation that was explicitly exploratory — *"help me think"*,
*"help me consider and log all of this"* — so nothing here is spoken and
nothing here may land on its strength alone. `DECISIONS.md` §open carries the
one row that points at this file; when a line of it is ruled on, that row
moves to §Spoken and this file becomes the description of something real.

Read it before proposing art, monuments, or a second health pool. It exists
so the next pass does not re-derive a world that was already designed, and so
the five places this direction **collides with a live gate** (§7) are known
before somebody trips one.

The house style applies: where a claim is measured off the tree it carries a
`file:line`, and where it is taste it says so.

---

## 0 · The pitch, in one paragraph

Same skeleton, different world. Rust's verbs are untouched — chop, mine,
craft, build an ugly box, get jumped while farming, put metal doors on
everything — but the island is the corpse of an ancient civilization that
was doing things nobody now understands, and the wilderness that grew back
over it is not from here. Obsidian for mass, lapis for circuitry, gold for
joints and seals. Not a temple map: an *infrastructure* map, because you
should be able to infer what a place was **for**. The player is not the hero
of this. The player is a rat living in the ruins of gods, and the whole
register depends on protecting that contrast.

The reference for the corrupted interior is Outland/Mordor — volcanic glass,
green fissures, broken sky. The reference for the coast is unchanged: a
beach you could survive on.

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
one enormous piece of geometry visible from everywhere buys the whole
fiction from every vantage on the island.

**Our four biomes are `Beach · Meadow · Forest · Highland`** and
`biome(h, moist)` is a pure function of height and moisture
(`crates/sim-core/src/terrain.rs:246,263`). The gradient above needs a
**third input — distance from island centre** — and that is the single
cheapest structural change in this whole document (§8).

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
tell what a place was for by looking at it.** A player who works out that
the flooded chamber was a pump house, unprompted, is having the experience
the map is for.

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
| **The Black Ziggurat** | colossal stepped obsidian pyramid, top blown off, gold elevators, lapis conduits. Visible from half the map | Launch Site — **and the hero silhouette** |
| **The Severed Gate** | two obsidian pylons across a canyon, shattered gold portal ring between them, debris still hanging | Trainyard / power plant |
| **The Sunken Treasury** | half a city under the ocean, glowing lapis lines, air pockets in sealed chambers | *no analogue* — the ocean is currently scenery |
| **The Fel Quarry** | spiral open-pit mine, black terraces, gold machinery, things crawling out of the bottom | Giant Excavator |
| **The Hanging Gardens** | ruined vertical city, roots, waterfalls, rope bridges, spores | vertical PvP; no clean analogue |
| **The Golden Spine** | ancient aqueduct/highway crossing miles of map, intact in sections | the road ring — *navigation, not a destination* |
| **The Crucible** | foundry inside a volcano; obsidian moulds, molten rivers, broken golems | smelting; partial Excavator |
| **The Hollow Colossus** | a fallen construct you enter through a wound in its armour. Black frame, lapis nerves, gold joints | dungeon; no analogue |
| **The Observatory** | mountain-top rotating gold rings around a black reflecting pool | satellite dish |
| **The Ossuary** | subterranean necropolis, thousands of obsidian niches, no natural light | Military Tunnels |
| **The Lapis Wells** | vast circular shafts descending to a subterranean lake, pumps still running | Oil Rig — *inverted* |
| **The Crown** | offshore island: seven obsidian towers around a gold spire, bridges between | Cargo Ship / Oil Rig, and the horizon's threat |

**Two of these are structurally different from the rest and worth naming
separately.** The Golden Spine is not a destination — it is the road ring
with a story, and it costs nothing extra because we already generate a road
(`sim-core/tests/road.rs`). The Black Ziggurat is not primarily a place — it
is a silhouette, and it pays for itself from every point on the island before
anyone can enter it. If exactly two things get built, build those two.

---

## 4 · The ward (the second health pool)

A regenerating shield over the health bar. Health does not meaningfully
regenerate; the ward does, after a few seconds without damage. Combat then
has two different kinds of damage:

- **ward damage is pressure** — it costs tempo and nothing else
- **health damage is attrition** — it costs meds, cloth, food, downtime

That converts the reference game's fight rhythm, which is
`engage → hit → syringe → peek → syringe → peek`, into
`engage → crack → push → kill`. The break should be *audible and
unmistakable*, because the real product of the mechanic is a shared
vocabulary: **he's cracked, push him.**

The second thing it buys is a densely dangerous world that is not a tax.
A bite from something in the woods costs a ward and ten seconds; a *pack*
that breaks the ward costs actual resources. That is what lets the interior
be hostile without every thirty-second encounter being resource misery — and
without a hostile interior, §1's gradient has nothing at the far end of it.

### 4.1 · Ward is the one carried ancient thing, and that is why it must not be loot

Everything else ancient is **a place you go, never a thing you carry** (§5.1).
The ward breaks that rule, so it has to be handled deliberately: it is issued
by whatever puts you back on the beach when you die, it is **identical for
every player**, and it is **not upgradable, not lootable, not craftable**.

That is not flavour. A ward that appears in the loot ladder is a stat, a stat
invites tiers, and tiers are how a survival game becomes an MMO. Universal
and fixed keeps it a *rule of the world* — which is also the only version
that stays legible in a fight, because you always know exactly what the other
player has.

### 4.2 · The non-obvious consequence

**Ward regeneration rewards disengagement, and a base is the best
disengagement tool in the game.** A player who can break line of sight behind
a door they own regenerates for free; a player caught in the open cannot. So
ward quietly favours whoever already has a base near the fight — which is
already the stronger side. Worth measuring before it is tuned, not after.

### 4.3 · What it costs in this tree

Cheap in the sim, expensive in the balance:

- `Player` gains `ward` + `ward_max` + a since-damaged tick counter. Integer
  only, no clock, no allocation — this is the easy half.
- Regen is a per-tick decrement against a deadline in **ticks** (wall 5).
- It is predicted state, so it is subject to the quantize-both-sides law:
  the server sims on the values it transmits or prediction drifts on the one
  number the player is watching.
- A ward-break **event** and a role gate for its payload
  (`crates/sim-core/tests/event_roles.rs`).
- Wire: ward is on the local-player lane and on other bodies if it is drawn
  over their heads → version bump + regenerated goldens **in the same
  commit** (wall 6).
- **The balance collision is the real cost** — §7.2.

---

## 5 · Falling

**The tree has no fall damage.** Nothing in `crates/sim-core/` or
`content/` implements it; the only trace of falling in the sim is a terminal
fall speed used to bound a saved position (`sim-core/src/persist.rs:78`).

So "no fall damage" is not a removal — it is **the status quo**, and the
actual proposal is to *add* a mechanic: **a fall costs ward in proportion to
height, and never costs health.** That is strictly better than free falling,
and it is the right shape:

- a wall drop is nearly free
- a monument drop lands you **alive and completely cracked**

Which produces the decision the mechanic exists for: you can jump off a tower
to escape the man chasing you, but you land with no defence; you can assault
someone from above, but you arrive naked. Both are real choices with a price
tag, which is what "no fall damage" alone does not give you.

It also unlocks §3's vertical monuments as *gameplay* rather than scenery.
The Hanging Gardens can be 500 feet tall the moment a fall is survivable.

### 5.1 · The rule this belongs to

**Ancient technology is environmental, never inventory.** Every ancient thing
is a place you go, a machine you switch on, a route you take — never an item
in a bag and never a power on a character sheet. The ward is the sole
exception and §4.1 is why it is a safe one.

This is the rule that protects the operator's own framing: *the civilization
should be majestic, the players should still be rats*. The failure mode of
this entire direction is player-facing magic — a glowing sword, an ancient
armour tier, a spell. The moment a player carries the fantasy, the contrast
that makes the world work is gone and it is an MMO with building. **Player
gear stays crude forever. The world is what is magnificent.**

### 5.2 · What falling does to bases

Fall damage is load-bearing in the reference game's base design in a way that
is easy to miss: it is what makes roofs safe, what prices a failed jump, and
part of why vertical access is worth gating at all. Free falling means every
base is roof-accessible and every honeycomb is something to drop into.

The ward-cost fall answers it in the same motion — **you can drop into
someone's base, but you arrive cracked, inside their walls, next to people
who are not** — which is a fair trade rather than a hole. But it is a
*building* decision as much as a movement one, and `reference/BUILDING.md`
should be re-read against it before it lands.

---

## 6 · World states — the mechanic worth building

The strongest idea in the direction, and the one that changes what kind of
game this is. Monuments are not loot boxes with a card puzzle; they are
**working infrastructure that players can switch back on**, and switching one
on changes the whole shard.

The pattern is fixed and every state obeys it:

> **you get something / you unleash something.**

Without the second half, clans keep everything on permanently and the states
are just buffs. With it, every activation is a decision somebody else has to
react to.

| state | you get | you unleash |
|---|---|---|
| **The Crucible** — the forge lights | furnaces smelt ~2× faster, high-tier ore surfaces | the volcano region turns hostile; running furnaces emit visible light pillars |
| **The Observatory** — the rings turn | night vision, directional navigation, rare falling fragments | movement emits visible pulses on the map |
| **The Severed Gate** — the ring closes | rare dimensional resources, portals across the map | invasion: things wander far outside their range, night goes green-black |
| **The Golden Spine** — the network wakes | fast travel across the island | **everyone** can use it, including whoever is coming for you |
| **The Lapis Wells** — the water returns | crops and collectors surge, healing crafts cheaper | corruption spreads around every water source |
| **The Black Ziggurat** — the sun dims | shields recharge faster, gold veins open, ancient doors unlock across the map | daylight becomes twilight, everything hostile gets stronger |

The Ziggurat is the one that sells it. A fresh spawn is chopping a tree, the
sky goes black-green, a horn sounds across the whole island, and the screen
says **THE ZIGGURAT HAS AWAKENED**. An endgame clan has voluntarily turned
the server into Mordor because the rewards were worth it, and a naked on the
beach now has a story about a decision they had no part in.

That is the payoff: **the shard acquires a state.** Not "where is Launch
Site" — a map is learned once and then static — but *"why is the sky green?"*
*"Some idiots opened the Gate."* A world state is the cheapest generator of
multiplayer stories this design has, because it makes 100 players react to
one clan's decision without any of them being scripted.

### 6.1 · The griefing hole, which is real

The cheapest strategy on a 100-player shard is to **activate the punishing
half from inside your own walls and let it land on everyone else.** Every
state above has a downside that is global while the upside is local to
whoever is prepared for it, so a bored clan turning on the worst state hourly
is not an exploit — it is the optimal play.

Two candidate mitigations, both open knobs, neither obviously correct:

1. **The activator is marked** for the duration — the Observatory's pulse
   idea generalized. If you wake it, everybody can find you.
2. **Activation costs something scarce**, so it is self-limiting and reads
   as an investment rather than a prank.

The second is more likely right, because the first punishes only the clans
who are bad at hiding. Both should be decided before the first state ships,
not after the first weekend of it.

### 6.2 · The engineering shape (so it does not fight the walls)

This is a global mutable modifier touching gather rates, spawn rates,
lighting and hostility — the most invasive thing in this document — and it
only stays legal if it is built as **one bounded table**, never as branches
sprinkled through the systems it modifies:

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
- **The table is in `state_hash`.** A modifier that is not hashed is a
  determinism hole that only shows up as prediction drift.
- **Broadcast is a wire change** — version bump + goldens, same commit
  (wall 6) — plus an event with a role gate.
- Any new interaction verb needs an arm in `render/verbs.rs` or the
  workspace is green twice and the Bevy gate is red (`CLAUDE.md`, the
  feature-gated-match trap).

Built this way it is genuinely small: a table, a deadline, and a multiplier
lookup at ~6 call sites.

---

## 7 · What this collides with, concretely

Five, and they are not equally serious. **7.1 is an operator act and blocks
every visual pass; 7.2 is a green gate over a stale number.**

### 7.1 · The art rubric scores against the wrong world ⚠

`ART.md` is measured off `Rust Images/` — eighteen frames of a temperate
pine-and-granite island — and the loop's visual judge scores every pass
against `gates-loop/art/RUBRIC.md`, which lives **outside this repo and is
checksummed between passes** precisely so a builder cannot edit the criteria
it is graded on.

So the moment the world becomes obsidian and green fissures, **every visual
pass is scored as a defect by construction.** The builder cannot fix it, and
a pass that tries will circle — which is the exact failure mode `CLAUDE.md`
already warns about under "a judge names the symptom".

The way out is already written down. `DECISIONS.md` 2026-08-01 set the
target as *"rip rust for now"* and said in the same row: *"Revisit when a
concrete palette is spoken; that changes the style section of the visual
rubric only, never the ten criteria."* This is that revisit. It needs three
things, all operator acts:

1. a spoken palette,
2. a reference set for the new register — the equivalent of `Rust Images/`,
   because `ART.md`'s authority comes entirely from being *measured*, and
3. the rubric's style section updated to point at it.

**Until all three exist, no visual pass should chase this direction.** The
ten criteria — value separation, contact, density, no flat surfaces — are
about how a picture works and survive the change untouched.

### 7.2 · The ward silently invalidates the TTK anchor ⚠

`CONTENT.md` §4 anchor 2 declares TTK bands in body hits — melee 3–5, bow
3–4, revolver 4–6, headshot ×2, armour worth at most +2 hits — and
`test_content` computes them against `content/balance.toml`'s
`globals.player_hp = 100`, which is the same number `combat.rs` plays.

Add a ward and every one of those bands is measuring a different quantity.
The gate does not go red — **it goes quietly wrong**, which this repo already
knows is the worse failure. The anchor has to learn about effective HP in the
same commit that adds the pool, or a band that reads "3–5 hits" describes a
fight that takes eight.

Two further consequences worth stating:

- **`reference/BALANCE.md` §6.2 permits this.** "Take theirs" is the default
  and only a **mechanism** difference justifies differing — a second
  regenerating health pool is exactly that, the admissible kind. The
  direction does not break the balance discipline; it triggers its
  documented exception.
- But it means every damage number ripped so far was priced against 100 flat
  HP with no regeneration. Taking their numbers *and* adding a ward is taking
  half a system — the same shape as `reference/RIPLIST.md` §0's threat frame,
  where a yield taken without the interruption that balanced it is a
  false familiarity.

### 7.3 · The gradient is a third input to a two-input function

`biome(h, moist)` is pure and takes height and moisture
(`terrain.rs:263`). §1's coast→interior gradient is a **radial** term, which
is a third input. The change is small and contained, but its blast radius is
every terrain gate: `test_terrain_golden`, the slot list, the spawn ring, the
road ring, the haven pad. Regenerated goldens in the same commit, and it is a
single-lane change — no other branch touching terrain that window.

### 7.4 · Monument placement is partly unbuilt

`TERRAIN.md` §7 shows the haven pad selector **finds** flat ground rather
than **carving** it — best natural site measured at 3.76 m of relief. Every
monument in §3 is a large flat footprint on generated terrain, so they all
want the carve that does not exist yet. The Golden Spine is the exception:
it rides the road, which is already generated and already gated.

### 7.5 · Scope, against the repo's own pillar

`DESIGN.md` §2 puts **two monuments-lite** at v1 and pillar 1 is *the
skeleton is the product*. This document names twelve monuments, a second
health pool, a new fall mechanic, six world states and a new biome axis.
That is not an argument against any of it — it is an argument about **order**,
which is §8.

---

## 8 · Ordering, if any of it is spoken

Roughly by cost, and the first two are worth doing even if the rest never is:

1. **Name the corruption** (§1.1). Free, and every other item reads better
   after it.
2. **The biome gradient** (§7.3). One function, regenerated goldens. It is
   the load-bearing half of the fiction — exploration that *changes* as you
   walk inland — and it needs no art, no wire and no new system.
3. **The ward** (§4), with the TTK anchor extended in the same commit
   (§7.2). Self-contained, and it changes how the game feels more than
   anything else on this list.
4. **Falls cost ward** (§5). Nearly free once 3 exists; the sim already has
   the vertical velocity.
5. **One world state, end to end** (§6.2) — the table, the cap, the wire, the
   content multipliers, the expiry. Ship **the Crucible**, because a faster
   furnace is a boring effect and the machinery is the interesting part. One
   state proves the whole system; six is content afterwards.
6. **The hero silhouette** — the Ziggurat as geometry on the horizon, no
   interior. It buys the entire register from every vantage and needs
   nothing from §7.4.
7. **Monuments as places you enter.** Blocked on the terrain carve (§7.4)
   and on the art direction (§7.1). Last, and correctly so.

---

## 9 · What must be spoken before any of this lands

Collected for `DECISIONS.md`; none of it is a number a loop may invent.

- **The register itself** — is the world obsidian-and-corruption, or does
  `DECISIONS.md` 2026-08-01's *"rip rust for now"* stand? Everything else
  here is downstream of that one call.
- **The palette, the reference set, and the rubric's style section** (§7.1) —
  three operator acts, and the visual loop is scoring against the old world
  until all three land.
- **The corruption's name** (§1.1).
- **The ward numbers** — pool size, delay before regeneration, regeneration
  rate, and the fall-to-ward curve. Four knobs, and the TTK bands move with
  them.
- **Whether the ward is universal and fixed** (§4.1). This is a design rule,
  not a number, and it is the one that decides whether this stays a survival
  game.
- **The world-state anti-grief rule** (§6.1) — marked activator, scarce cost,
  or something better.
- **How many monuments the alpha actually carries**, against `DESIGN.md` §2's
  two.
