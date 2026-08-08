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
| `SAVES.md` | how the reference game remembers a player — **there is no player save file**: the body stays in the world as a sleeper and is saved because it is an entity, the save and the wire on one base class, the stop-the-world stall it never fixed, the wipe split, and **§9 what it means for us**. Same clean posture as `AUDIO.md`; the operator adopted its model, so §9 is a plan |

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
- `Rust Images/` — the reference frames, already in the repo, which is what
  `MENUS.md` cross-reads the hook table against when the two disagree about
  whether something is one screen or two.

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
