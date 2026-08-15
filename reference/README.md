# reference/ — ripped facts about the reference game

Not our design, not our queue, not law. A **ripped, regenerable
reference**: what the reference game's systems are, in enough detail to
size ours against them. `MENUS.md` is the analysis; this directory is the
evidence it was measured off.

Nothing here ships. No code from any source below is copied into the
game, the client, or the build.

## What is in here

| file | what |
|---|---|
| `rip-hooks.py` | the extractor — reproducible, no network of its own |
| `rust-systems.txt` | its output: 852 patch entries, 277 game classes, 38 categories |
| `FINDINGS.md` | what the two loaders' **commit history** teaches: which systems bled, and the one gate-shaped hole it exposes in ours |
| `SPAWN.md` | how the reference game places and respawns world objects — four systems split by networked-or-not, and what that costs us. **Different source, different licence posture: see below.** |
| `AUDIO.md` | how the reference game decides what a player hears — mixer groups and snapshots first, a 0.3 ms per-frame budget as a convar, localized ambience, the 2–5 kHz carve that makes room for footsteps and gunshots, and the four audio bugs it shipped. **Third source, and the cleanest of the three**: public devblogs and the convar list the game prints to any player who types `find audio`. Nothing decompiled, nothing extracted |
| `DOORS.md` | how the reference game decides **who is allowed through a door** — the lock is a separate entity from the door and a door with no lock is anyone's, the code lock's remembered list and its guest tier, the two goes it took them to rate-limit a keypad, knocking as a verb, an eleven-verb checklist we are scored against, and **§9 what it means for us**. **Sources ranked in its own §0**: the in-tree MIT hook table first (the object model is read straight off it), devblogs second, community wikis third — and every page fetch was refused by the egress proxy, so tiers 2 and 3 are *search summaries of those pages, not the pages*, which is a real weakening and is said there rather than here |
| `BUILDING.md` | how the reference game decides **who may build here** — the cupboard's authorized list, which is one component it reuses on three unrelated classes; privilege as a **volume emitted by the building blocks** (Devblog 185) rather than a sphere around the cupboard; upkeep as a *cost* and the activity-keyed decay bug that forced it (Devblog 189); the decay ladder; the demolish/rotate grace window; and **§9 what it means for us**. `DOORS.md`'s source ranking and its proxy caveat apply unchanged |
| `PROJECTILES.md` | how the reference game does **bows and arrows** — there is no `Bow` class (a bow is a one-round-magazine `BaseProjectile`, the same class as every gun), the projectile is **simulated on the client** and audited by a thirteen-convar tolerance budget, the arrow is an item three times over (inventory → projectile → world pickup, ~15 % break, 10 s lodge), the compound bow's draw prices exposure *and* durability, hit detection takes the **most significant** body part rather than the first intersection, and **§9 what it means for us**. `DOORS.md`'s source ranking applies; its proxy caveat applies again and in the opposite direction from `SOURCES.md`'s last reading — see its §0 |
| `SAVES.md` | how the reference game remembers a player — **there is no player save file**: the body stays in the world as a sleeper and is saved because it is an entity, the save and the wire on one base class, the stop-the-world stall it never fixed, the wipe split, and **§9 what it means for us**. Same clean posture as `AUDIO.md`; the operator adopted its model, so §9 is a plan |

| `MONUMENTS.md` | how the reference game decides **where a large authored place goes** — placement as a solve rather than a guess (three rewrites over ten years and still moving), the collision list every worldgen system built after monuments produced (rivers, cliffs, ice lakes, roads, ring roads, rails), terrain blending as authored per-monument masks instead of a flattened circle, the 2015 client-worldgen checksum mismatch they had to stop kicking for, vertical AOI layers, per-class interest ranges, what one moving monument actually costs, and **§9 what it means for us**. **Weakest provenance in this directory and its §0 says so at length**: an operator briefing summarising sources nobody here has opened, so no number in it may reach `content/` — the finding is the ORDER, which is checkable against our tree |

| `VOICE.md` | how the reference game does **proximity voice chat** — voice arrives as a `ServerMgr` message rather than an entity RPC (read straight off the hook table, §1), it began on Steam P2P until players read each other's **IP addresses off the session and DDoSed them**, the forced move onto the server (Devblog 189) is what later made loudspeakers, phones and tape recorders expressible at all, Steam's voice API hands you a codec and a microphone and **no transport**, the radius is a disclosure mechanic rather than a chat setting, and the three costs they have published are a talker-side hitch, a fan-out knee around 8–10 concurrent talkers, and moderation forever. **§9 what it means for us.** `DOORS.md`'s source ranking and its proxy caveat apply in full — and tier 1 earned its rank here, correcting a search summary that reported the pre-2017 P2P design in the present tense |

> ⚠ This table is **incomplete and was already incomplete before `MONUMENTS.md`
> was added to it**: `ANIMALS.md`, `WATER.md`, `NETWORK.md`, `BALANCE.md` and
> `RIPLIST.md` are in this directory and have no row here. `CLAUDE.md`'s doc
> table is the one that is kept current — read that one.

## How to regenerate

```
git clone --depth 1 https://github.com/OxideMod/Oxide.Rust.git /tmp/oxide
./reference/rip-hooks.py /tmp/oxide/resources/Rust.opj > reference/rust-systems.txt
```

The dump is committed so a pass can read it without network. Regenerate
when the reference game has moved and the question is whether a system
grew a verb we should care about — not on a schedule.

## Why a mod loader is the instrument

A shipped game does not publish its verb list, but a mod loader has to
name every method it intercepts, because a hook exists exactly where a
modder needed to stop a player action. Oxide's patcher project is that
list as data: for every hook, the game class it patches, the method
signature, and Oxide's own category for it.

Three things fall out that a hook *name* list cannot give you:

- **The class is the system.** `BasePlayer` carries 55 hooks; `BaseOven`
  9; `ItemContainer` 3. Ranking classes by hook count ranks systems by how
  many verbs they actually have.
- **The signature is the payload.** `Item.MoveToContainer(ItemContainer,
  Int32, Boolean, Boolean, BasePlayer, Boolean)` says exactly what an
  item-move has to carry — target container, target slot, and who did it.
  That is a wire-format spec sitting in someone else's build script.
- **The category is a second opinion.** Oxide grouped its own hooks 38
  ways. Where that grouping disagrees with ours, one of us is modelling
  the game wrong, and it is worth knowing which.

## Provenance and licence

- **`OxideMod/Oxide.Rust`** — `resources/Rust.opj`, **MIT**, © 2013–2020
  Oxide Team and Contributors. The sole source of `rust-systems.txt`.
  Facts only: hook names, class names, method signatures, categories.
- **`CarbonCommunity/Carbon.Hooks.Base`** and **`.Community`** — **GPL-3.0**.
  Used only as a *cross-check* on coverage (115 hook names, mostly newer
  surfaces: clans, racked weapons, apartments, CUI drag/drop). Deliberately
  **not** a source for anything committed here, so nothing in this repo is
  derived from GPL work.
- **The reference set** — the eighteen reference frames, which is what
  `MENUS.md` cross-reads the hook table against when the two disagree about
  whether something is one screen or two. **They are not in this repo**
  (removed 2026-08-11; they lived in `Rust Images/`): they are the reference
  game's screenshots and this repo is public, so carrying them here would be
  redistributing them — the same line this file draws around Carbon and the
  `SPAWN.md` decompile, applied to pictures. `ART.md` §0 has the posture, the
  recorded measurements, and the `GATES_REFERENCE_DIR` path for re-deriving
  them from your own copy.

**`SPAWN.md` is the exception, and it is flagged rather than buried.** Its
source is a community decompile of an Oxide-patched `Assembly-CSharp`
(`unet-dev/Decompiled-Assemblies`, protocol 179) — **proprietary code, not a
licensed dump**, so it does not get the treatment `rust-systems.txt` gets:
there is no extractor for it here and there never will be, nothing was
transcribed, and every algorithm in that file is described in prose and in
our own notation. Facts and behaviour only, in the same sense as the rest of
this directory — and, like the rest of it, nothing ships. `SPAWN.md` §0
states the terms; read them before adding to it.

`umod.org`, `docs.carbonmod.gg` and `wiki.facepunch.com` are all
unreachable from the build box's egress policy; the repos are, which is
why the instrument is a clone and not a scrape.
