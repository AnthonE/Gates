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

`umod.org`, `docs.carbonmod.gg` and `wiki.facepunch.com` are all
unreachable from the build box's egress policy; the repos are, which is
why the instrument is a clone and not a scrape.
