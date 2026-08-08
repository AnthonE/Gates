# reference/ANIMALS.md — how survival games do animal mobs

Research, not law. Dated 2026-08-08, written to answer one question the
operator asked directly: *how did the reference game and its neighbours do
animals, and what is the best solution for us?* §9 is the part that changed
what we built; everything above it is other people's evidence.

Read against `reference/SPAWN.md` (which owns *placement*, and whose §3 is
the same `SpawnHandler` populations that own animals), `TERRAIN.md` §2 (our
slot model), and `NETCODE.md` §1 (why an animal is class D).

## 0 · Provenance, and it is the clean kind

**This file is `AUDIO.md`'s posture, not `SPAWN.md`'s.** Nothing here comes
from a decompiled assembly. The sources are public devblogs, the public
server convar list, the community wikis that datamine published values, and
the published mechanics of three other games. No code was read, transcribed
or adapted, and nothing from any of them ships.

One honest limitation, stated because it shapes what the tables below can
claim: **`rust.facepunch.com` is blocked by this container's egress proxy**,
so the devblog facts here are quoted through search summaries of those pages
rather than read off them directly. They are consistent across independent
retellings (Rustafied, PCGamesN, the wikis, server-host guides), which is
the strongest form the constraint allows — but a later pass with direct
access should re-read devblogs 155–186 and the Contacts update post before
treating any number below as measured. Where a claim is a *mechanism* rather
than a number, the risk is low; where it is a number, it is marked.

## 1 · The four questions every one of them answers

Every game surveyed answers the same four, and the differences between them
are entirely in which answer they pick — not in which questions exist.

| # | question | why it is load-bearing |
|---|---|---|
| A | **Where does an animal walk?** | navmesh, steering, or grid — decides the boot cost and the memory |
| B | **How often does it think?** | every frame vs a fixed rate — decides the server's tick budget |
| C | **What happens when nobody is near?** | dormant, despawned, or fully simulated — decides the population ceiling |
| D | **Where does a dead one come back?** | its own spot, a re-rolled spot, or nowhere | decides whether the world drifts over a wipe |

## 2 · Rust — a navmesh, a fixed think rate, and dormancy

The reference game's answers, in the order it arrived at them.

**A — a baked Unity navmesh.** Animals path on a navmesh the *server*
generates. The costs are all documented in public and all of them are boot
costs: generation takes minutes, it uses **100% of the CPU while it runs**,
and every server-host guide warns operators about the startup stall. They
spent real devblog effort on it — the navmesh resolution was reduced,
described as having "no negative effect on animal behavior" while cutting
**load times to a quarter** and dropping CPU and memory with it. There is a
convar-selectable alternative, a *navmesh grid* (`nav_grid 1`), which exists
because one mechanism did not fit every server. Animals can only move within
their navmesh patch.

**B — a fixed think rate.** The AI was changed to "think at a fixed rate
rather than trying to think every frame", and the devblogs frame this as a
**server performance** change, not a behaviour one. The shipped convar is
`ai.tickrate` (default 5), alongside `ai.think` as a global on/off — an
admin's first lever when a server is struggling is turning AI thinking off
entirely, which tells you what fraction of the frame it was.

**C — dormancy.** NPCs are made dormant beyond a distance from any player.
This is the mechanism that lets one shard hold a large population at all.

**D — re-rolled, not restored.** Animals are a `SpawnHandler` population
(`reference/SPAWN.md` §3), and populations do not remember where anything
died: a death decrements a live count, and the 60 s repopulate tick samples
the distribution fresh. Over a wipe the population **migrates** toward the
peak of its own spawn filter.

**Two behaviour details worth stealing and one worth not.** Animals were
restricted from roaming too far from where they spawned, explicitly so they
would stop "ending up at the coast of the island" — the same bug our leash
prevents. Some animals were given a **blind spot behind them** so a player
can sneak up. And the 2021 Contacts update replaced three separate AI
systems with one, which is a lesson about *not* letting a second AI system
start.

**Numbers, marked as unverified.** The public wikis put the boar at
aggressive-until-hurt, fleeing below ~50% health, dropping raw pork, fat,
cloth, leather and bone. Health values differ between datamining sites and
between console/PC builds; nothing below §9 depends on one.

## 3 · Valheim — zone spawners and a ring around the player

Valheim has **five** spawn systems and the interesting one is zone
spawning, because it is the opposite architecture to a population:

- A spawn position is drawn **40–80 m from a random player in the zone** —
  a ring, so nothing appears in your face and nothing spawns where you
  cannot eventually find it.
- Each spawner retries on a cadence, with the number of attempts derived
  from elapsed time and **capped by a creature limit**.
- Physical spawners skip if there are already 2–3 enemies within 20 m, and
  skip entirely if there are **100 enemies within 1000 m**. Two caps, local
  and global, doing different jobs.
- Night-only spawns **despawn at daybreak**.
- Player-built structures create an invisible area that suppresses zone
  spawning, spawn points and event spawners — but deliberately *not*
  physical spawners.

The transferable idea is the pair of caps: a local one for texture, a
global one for the server. `SPAWN.md` §3.4 finds the reference game doing
the same thing with `ClusterSize` and `localCap`, which is the strongest
signal in this file — two unrelated codebases converging on cluster-plus-cap.

## 4 · Minecraft — the mob cap is the whole design

Minecraft's answer to question C is the most aggressive of the four and it
is worth naming precisely because it is the one we did **not** take:

- A spawn cycle runs **every tick**, in eligible chunks around each player.
- Caps are **two**: a global cap and a per-player cap, split by category
  (hostile, passive, water, ambient), each with its own ceiling.
- Mobs spawn in **packs** of 1–4, not one at a time.
- Despawn is distance-driven and two-tier: **immediate past 128 blocks**,
  and past 32 blocks a **1-in-800 per-tick roll** if the mob has not taken
  damage in 30 s.
- A `PersistenceRequired` tag exempts a mob from all of it.

The 1-in-800 roll is the detail that says what this design is for: the
population is a *flow*, not a roster. Mobs are cheap, anonymous and
constantly replaced, and no individual mob is expected to survive being
walked away from. That is a completely coherent answer to a different game
than ours.

## 5 · What the tick-budget literature says, independent of any game

The pattern is the same everywhere it is written down: AI replans every N
ticks rather than every tick, and pathfinding is **budgeted across ticks**
rather than run all at once. Minecraft server operators have a name for the
distance-scaled version — DAB, "Dynamic Activation of Brain" — where mobs
further from players think less often rather than not at all. Which is
Rust's dormancy with a gradient instead of a cliff.

## 6 · The comparison, in one table

| | Rust | Valheim | Minecraft | **Gates** |
|---|---|---|---|---|
| walks on | baked navmesh | physics + steering | block grid | **analytic heightfield** |
| think rate | fixed (`ai.tickrate` 5) | spawner cadence | per-tick cycle | **fixed, phase-offset by slot** |
| far from players | dormant | zone inactive | despawn | **dormant (hard skip)** |
| population | target density, live count | local + global caps | mob caps by category | **fixed roster of slots** |
| a death returns | re-rolled from the distribution | re-rolled | re-rolled | **at its own home** |
| costs at boot | minutes at 100% CPU | — | — | **~1,500 terrain probes** |

## 7 · The one thing three of them have and we do not

**None of our animals fight back.** Rust's boar charges, its wolves pack up
and hunt, and its 2021 AI rework was largely about making NPCs coordinate.
Ours flees and only flees.

That is a v0 scope call and not a design conclusion, and §9.5 says what it
costs to change.

## 8 · Sources

Devblogs and update posts (via search summaries — see §0): devblog-155,
-156, -157, -164, -168, -170, -186 (navmesh resolution, baking on startup,
`nav_grid`, dormancy, fixed think rate, roam restriction, blind spots,
metabolism); the June 2021 **Contacts** update (three AI systems merged into
one). Server-host and admin references for the convar layer (`ai.tickrate`,
`ai.think`, `nav_disable`). Community wikis for datamined animal values,
marked unverified throughout. Valheim: the wiki's spawning-mechanics and
creature-spawner pages. Minecraft: the wiki's mob-spawning page and
independent write-ups of the cap and despawn rules. Tick-budget practice:
general game-server profiling write-ups plus the Minecraft server-tuning
literature on DAB.

## 9 · What this means for Gates

The part that is not research. This is what `sim-core/src/mob.rs` does and
why, stated against the four questions.

### 9.1 · A — no navmesh, and it is not a shortcut

**Their navmesh exists because their world is a file.** A `.map` is data:
the walkable surface is not knowable without computing it, so they compute
it once, at boot, at 100% CPU, and then spend devblogs cutting its
resolution to get the boot time back.

Ours is a **pure function**. `terrain::height` and `terrain::slope` answer
at any point on the island for the cost of a few hashes, with no bake, no
file, no memory and no boot stall. So an animal steers and lets
`movement::step` accept or refuse the step — and the whole class of problem
a navmesh solves does not arise. This is the single largest thing the
research changed: the first instinct was to ask how to build a cheap
navmesh, and the correct answer was that we already have something better
and it is the same thing that made the terrain deterministic.

The second-order win is bigger than the first. Because the animal drives the
**same `InputFrame` a player does**, it gets piece collision, tree
collision, the cliff ratio, step-up, the wade slowdown and the world border
for free — in the exact code the client predicts players with, quantized by
the same function. There is no second movement implementation to keep in
step with the first, which is the mistake that would have cost the most
later.

### 9.2 · B — a fixed think rate, phase-offset, which is strictly better than theirs

We take their lesson and sharpen it. `MOB_THINK_TICKS` is 15 (half a
second), and mob `i` thinks on ticks where `tick % 15 == i % 15`, so the
per-tick decision cost is `MAX_MOBS / 15` ≈ 4 animals rather than 64.
Movement still integrates every tick for every waking animal, because a body
that stepped at 2 Hz would visibly stutter and the interpolator cannot
invent what the sim did not send.

Their version is a convar an admin turns *off* when the server struggles.
Ours cannot be turned off, because there is nothing to turn off: the work is
bounded by construction (wall 4), which is what `test_alloc_zero` running
100 bots against a full roster with zero allocations is evidence of.

### 9.3 · C — dormancy, and ours is replay-safe rather than a cache

An animal with no non-sleeping player inside `MOB_WAKE_CM` (240 m) does not
step at all. That is Rust's mechanism.

The thing that makes it *ours* is that the predicate reads player positions,
which are **sim state** — so a replay wakes exactly the animals the live run
woke, on the same ticks, and dormancy costs determinism nothing. A cache
keyed on anything outside `World` would have been a wall-5 violation wearing
an optimisation's clothes. 240 m is deliberately outside `AOI_EXIT_CM`
(208 m): nothing freezes while a client can still see it.

We did **not** take Minecraft's answer. A despawn-and-respawn flow makes
animals anonymous, and the roster model below depends on them not being.

### 9.4 · D — a dead animal comes back at its own home

This is the one place we deliberately diverge from all three.

Their populations re-roll, and `SPAWN.md` §3.5 measures what that produces:
a Rust forest **migrates** toward the peak of its filter over a wipe,
because every death is re-sampled from a peaked distribution. For trees we
already rejected that in favour of `TERRAIN.md` §2's slot model, and an
animal roster is the same argument with legs: `MAX_MOBS` fixed slots, each
with a home the seed chose once at world construction, each hatching back at
its own home after `respawn_ticks`.

What that buys is a **stable world**. "There are usually pigs in the valley
east of my base" is a true sentence about a Gates island and cannot be one
about a re-rolled population. It also makes the roster cheap to reason
about: 64 slots, no allocation, no spawn scheduler, no density scan, no
quadtree — the entire `SpawnHandler` apparatus of `SPAWN.md` §3 collapses
into an array and a per-slot deadline, because our seed can answer the
question their distribution had to sample.

What we *did* steal is their sampling posture, stated in `SPAWN.md` §3.2 and
correct in general: **sample cheaply and approximately, reject exactly.**
`home_of` draws a uniform point and rejects it against real terrain.

### 9.5 · What v0 does not have, in the order it should be added

1. **Nothing fights back** (§7). A mob→player damage path needs a new death
   cause on the wire and a reason for a player to be hit by something they
   cannot hit back at reliably — which is a combat-feel question, not a
   plumbing one.
2. **No corpse.** A killed animal leaves the snapshot and is gone; the loot
   is paid into the killer's inventory as `EV_GATHER`. A butchering verb
   (hit the corpse with a tool for meat) is the reference's actual
   interaction and it is a verb, not a species.
3. **No meat.** Cut because there was no cooking — and `sim-core/oven.rs`
   landed the same day this did, with an **empty** cook table for the
   mirror-image reason ("cooking wants a raw food and the island pays
   none"). So the blocker is gone and what remains is two content rows and
   a `drops` line. Left for a spoken call on the food set rather than taken
   here (`NOW.md` §0m and §0v).
4. **No blind spot.** Their "sneak up from behind" is a bearing test in
   `think` and about four lines; it wants the crouch verb to mean something
   first.
5. **No packs.** Their wolves and deer form them, and it is the most
   expensive item on this list — a second animal reading a *first* animal's
   state is the thing that turns a roster into an AI system, which is
   exactly what their 2021 rework was cleaning up after.
