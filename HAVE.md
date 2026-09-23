# Gates · HAVE.md — what the game is today

The inventory of what a player can **do**, drawn off the tree with commands
rather than off any doc's memory of itself. Every count here is the output of
a command in §8; re-run them rather than trusting a number, because a count in
prose drifts the day after it is written (`CLAUDE.md` says this about
`props.js` and about the icons, twice, and it was right both times).

> **Why this file did not exist until now.** Every document in this repo is
> organised around **the gap**. `NOW.md` is 3,351 lines of what is missing.
> `DECISIONS.md` is 349 knob rows. `MENUS.md` scores us against the reference
> game and marks the cells we lose. `reference/` is 26 documents about how
> somebody else solved it. That is the right instrument for *building* and it
> is a terrible instrument for *seeing*: the tree has never once been read
> forwards. Read only the queue and this looks like a project that has not
> started. It is 295 k lines of Rust, 189 test suites, and a survival game you
> can boot, join over a real network, and play until something kills you.
>
> **This file owns nothing.** It is a mirror, so when it disagrees with the
> tree the tree wins and this gets fixed. It is not a queue (`NOW.md`), not a
> spec (`DESIGN.md`), and not a promise (`WORLD.md`).

---

## 1 · The loop, in the order a player meets it

This is one continuous session on a live shard, not a list of features that
exist separately.

**You arrive.** A double-click opens a boot splash, a main menu with a server
browser (search, favourites, live player counts polled off each shard's
`/status.json`), and a loading screen that fills as the world streams in.
Joining is a real handshake over QUIC/WebTransport: a wallet signature (SIWE)
proves who you are, so the shard files your character under an identity that
survives a restart.

**You wake on a beach** with a rock and a torch. The island is 2,048 m across,
generated from a seed — the same seed on the server and in your client, so
nothing about the ground is transmitted. It has biomes, a coast road with
faded markings, forests with an understory, rock formations, ore nodes,
barrels, crates, waystations, a haven pad and a freight depot. Day and night
run on an 80-minute cycle (70 min light, 10 min dark) and the sun arcs.

**You hit things.** Swing at a tree, a stone node, a metal node, a sulfur
node, a bush — five gatherable classes, each with a weak spot that pays a
bonus for hitting it. Trees fall. Barrels break and **spill 3D items on the
ground** you walk over and pick up, rather than turning into a sack.

**You craft.** 45 recipes, queued with a time and cancellable, gated by a
station ladder — nothing, then workbench 1/2/3, then a furnace — and a
bench above a recipe's own rung crafts it in half the time, two above in a
quarter. Eleven recipes need a blueprint first, as in the reference. The craft
screen is the reference's shape: category rail with live counts, search,
unaffordable rows dimmed, a detail pane with cost/time/station, a favourite
star, a quantity stepper, and a queue strip with a countdown.

**You build.** Hold right-mouse for the radial: 11 shapes (foundation, wall,
doorway, window, wall frame, floor, stairs, roof, and triangle foundation /
floor / roof) × 4 tiers (twig, wood, stone, metal) = 44 pieces. A ghost shows
the placement and is coloured by the server's own four refusal reasons. You
can upgrade a piece, repair it, demolish it inside a grace window, hang a
wooden or metal door on a doorway, and put a **code lock** on the door with a
remembered authorised list. A hearth claims the ground around it, and the
claim charges **upkeep** — stop feeding it and the base decays by tier.

**You store and carry.** 30 inventory slots (6 belt + 24 grid), a wear doll
with armour that reduces damage, small and large boxes, and full drag-and-drop
between any two containers with five gestures including the reference's own
right-click quick-move.

**You survive.** Health, hydration and calories tick down; you eat berries,
mushrooms, corn and meat, and drink at water. A fire pit or furnace **cooks**
raw meat into cooked or, if you forget it, burnt. A bandage or medkit heals.

**You fight.** Melee with limb bands and a headshot multiplier, a bow and a
crossbow with recoverable arrows, a revolver with a reload, and a satchel
charge with a fuse that blasts a hole in somebody's wall. Hits are rewound
against the shooter's own view (lag compensation), hit direction shows on
screen, and armour subtracts.

**You go down.** A lethal blow does not always kill: you fall into a wounded
state for 40–50 seconds, crawling at a third of walking pace, and a hashed
roll at the end either stands you up or makes the corpse. Your body drops a
backpack anyone can loot.

**You come back.** Respawn on a beach or on a sleeping bag you placed. The
island, your base, the boxes, the bodies and what you were carrying all
survive a server restart — world file and player save are separate, because a
wipe deletes one and keeps the other.

**And there is a second ladder.** Junk from barrels feeds a **research table**
(spend a sample and junk to learn a recipe permanently) and a **tech tree**
at a workbench, and a **recycler** turns loot back into components.

**Two animals share the island with you**: a pig that flees and a wolf that
hunts you, both with voices.

---

## 2 · The verb set

**30 commands the simulation accepts** (`sim-core/src/world.rs`, `enum
Command`) and **21 actions the wire carries** (`protocol`, `enum ActionMsg`).
Not a plan — the dispatch is in `server/src/core.rs` and every one of them has
a test.

```
Join JoinAs Leave Wake Evict          — session
Input InputPair                        — movement, look, swing, jump
Craft CraftCancel Research Unlock      — the two progression ladders
Place PlaceDeploy Upgrade Repair Demolish
Feed Access                            — hearth upkeep, lock auth
Use Loot Pickup OpenWorldCont Move     — containers and the world
Consume Drink Reload Throw Respawn     — survival and combat
AdminTeleport AdminGive                — the admin lane
```

**11 interaction prompts** (`ui::interact::Verb`): Door, Bag, Box, Hearth,
Fire, Recycler, Research, Crate, Take, TechTree — resolved by three separate
picks because a deployable, a terrain occupant and a loose stack are three
different kinds of thing to aim at.

---

## 3 · The content

All of it is data (`content/*.toml`), validated at boot, hashed into the WAL
header so a replay replays the content it was played under — wall 7.

| file | rows | what |
|---|---|---|
| `items.toml` | **57** | resources, components, tools, weapons, armour, food, deployables |
| `recipes.toml` | **45** | the craft ladder, T0 → T3, with times and station gates |
| `building.toml` | **44** | 11 shapes × 4 tiers, with hp per tier |
| `deployables.toml` | **14** | bag, hearth, two boxes, fire, furnace, 3 benches, recycler, research table, code lock, 2 doors |
| `weapons.toml` | **11 + 2** | 7 melee, 2 bows, 1 firearm, 1 throwable; wood and metal arrows |
| `gatherables.toml` | **5** | tree, stone / metal / sulfur node, bush |
| `consumables.toml` | **7** | food and medical |
| `cooking.toml` | **6** | raw → cooked → burnt |
| `research.toml` | **11** | the reference's learned set, one tree per bench |
| `loot.toml` | **3** | barrel, crate, cache |
| `armor.toml` | **3** | burlap hood, burlap tunic, roadsign vest |
| `mobs.toml` | **2** | pig (80 hp), wolf (100 hp) |
| `balance.toml` | — | spawn kit, decay ladder, survival rates, the TTK anchors |

**45 sound cues**, generated at boot rather than shipped as files
(`sound/synth.rs`) — footsteps per surface, impacts per material, swings,
gather, craft-done, refusals, hit, hurt, death, place, splash, tree-fall, UI,
three bed layers, birds, a nine-part adaptive music bed (calm/tense/combat ×
open/turn/close), wolf howl and growl, pig snort, bow and gun reports, and
head/limb hit confirms.

**24 3D models**, **78 icons**, **42 textures** shipped and licence-audited
(`assets/*/MANIFEST.md`, `CREDITS.md`).

---

## 4 · The skeleton — the part that is actually rare

A survival game is not hard to describe and is very hard to make honest. This
is the half that most projects of this age do not have, and it is gated, so it
is checkable rather than claimed.

- **Determinism.** `sim-core` is pure: no I/O, no clock, no threads, no hash
  iteration, no trig, a restricted float set. Same build + seed + input log →
  the same state hashes, every time (`test_replay`).
- **Native and wasm agree bit for bit.** The sim compiles to a second,
  deliberately hostile target and the state digests are diffed byte for byte
  (`test_parity_wasm`).
- **Zero heap allocation in the tick** after warmup, proven by a counting
  allocator over 100 bots × 300 ticks (`test_alloc_zero`).
- **Everything is bounded.** Every queue and store has a cap and a stated
  overflow policy, driven at the ceiling by `raid_storm` (64 players through
  build/lock/plant/guess/move/loot at once) and `combat_storm` (50 duels, 954
  deaths).
- **The wire cannot drift by accident** — byte goldens per packet type, a
  version bump required in the same commit, plus a role gate that checks every
  event's payload against its own doc line, because a byte golden is blind to
  a swapped field.
- **30 Hz tick, 100 players, ≤1,100 B per client snapshot**, prediction and
  reconciliation on the local player, interpolation on everyone else, and
  server-side lag compensation that rewinds the world to what the shooter saw.
- **189 test suites** across seven crates, all run by `./ci/gates.sh`.

**Two clients, one codebase.** The native Bevy desktop client and the browser
build are the same Rust — the same `ClientCore`, the same protocol, the same
handshake — separated by one trait with one method. There is no JavaScript
reimplementation of anything.

**It ships.** `ci/depot.py` packages a depot for the elo launcher on Linux and
Windows, `ci/build_web.sh` stages the page, a public shard runs at
`game.elopros.com` with persistence and wallet auth on, and a tagged release
re-runs every gate before it builds.

---

## 5 · The screens

**HAVE** = a player can open it today.

Boot splash · main menu with server browser · loading screen · settings
(6 categories, 9 live settings, 17 keybind rows) · pause menu that really
disconnects · HUD with hotbar and vitals · compass · chat (local + global) ·
inventory grid with wear doll · container panel · craft panel with queue ·
build radial with ghost · tech tree panel · map (held `G`, hillshaded,
16×16 lettered grid, your heading, bed/hearth/death marks) · respawn screen ·
death screen · bug-report screen.

**Missing**: team/contacts, keypad UI, hearth auth list, furnace panel, repair
bench, vending, kill-cam, pick-your-bag list.

---

## 6 · What it honestly cannot do yet

Kept to one screen on purpose — the long version is `NOW.md`, and reading the
long version first is how this tree got a reputation with itself for being
further behind than it is.

- **No wipe machinery.** Nothing resets a shard on a schedule; a seed change
  is a manual wipe and blueprints do not survive one because no wipe does.
- **No coins in world.** JUNK is earned and spent (recycler, research), but
  the claim rail, the bank terminal, the ledger and extraction are unbuilt —
  `ALPHA.md` stages this deliberately (A1→A2→A3) and it is an operator act.
- **No teams, no vending, no vehicles, no electricity** — cut from v1 in
  `DESIGN.md` §2, not missing by accident.
- **Two animals, one biome's worth of variety.** The roster is thin.
- **The desktop client is played and the browser one is not.** The operator
  plays this regularly (`DECISIONS.md` 2026-09-17) — the visual gate is a
  person, deliberately, and it runs. What has genuinely never been seen on
  hardware is the **browser** build: every frame of it so far is SwiftShader in
  a headless test, so its sky colour, its load time and its look on a
  high-DPI display are unmeasured. ⚠ This bullet claimed nobody played the game
  at all until 2026-09-17; `NOW.md` §LOOK carries why that was wrong and what
  the real gap is (answers with no channel back into the tree, not an act
  nobody performs).
- **No soak.** Wall 3 holds on clippy alone; nothing has run for four hours.

---

## 7 · The shape of the work, measured

Over the last four weeks this tree took **61 merges** — 6, 13, 18 and 24 by
week, rising. `DECISIONS.md` §open holds **244 named slices** (`wounded v0`,
`map legibility v2`, `forest density v1`, `browser audio v0` …), which is the
real changelog and has never been published as one. §9 of
`reference/PATCHES.md` is about exactly that.

---

## 8 · Re-derive every number above

```sh
# content
for f in content/*.toml; do echo -n "$f "; grep -c '^\[\[' "$f"; done
# the verb set
awk 'NR>=1212 && /^}/{exit} NR>=1212' crates/sim-core/src/world.rs | grep -E '^    [A-Z]'
sed -n '/pub enum ActionMsg/,/^}/p' crates/protocol/src/lib.rs | grep -E '^    [A-Z]'
sed -n '/pub enum Verb/,/^}/p' crates/client/src/ui/interact.rs | grep -E '^    [A-Z]'
# scale
find crates -name '*.rs' | xargs wc -l | tail -1
ls crates/*/tests/*.rs | wc -l
sed -n '/pub enum Cue/,/^}/p' crates/sound/src/lib.rs | grep -cE '^ {4}[A-Z]'
ls assets/models/**/*.glb assets/icons/*.png | wc -l
# the walls
./ci/gates.sh
# the cadence
git log --since="12 weeks ago" --date=format:'%Y-W%V' --pretty=format:'%ad' | sort | uniq -c
```
