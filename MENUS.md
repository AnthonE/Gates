# Gates · MENUS.md — the interaction surface, ours against the reference

An audit, not a law and not a queue. It answers two questions: **which
systems does the reference game have and how deep is each one**, and
**what do we have against them.** `NOW.md` stays the only list that
answers "what next" — items get cut from this survey into that queue,
never the other way. Numbers stay in `CONTENT.md`; nothing here is a knob.

Dated 2026-08-04. A row that disagrees with the code is wrong — fix the
row. The evidence is in `reference/`, regenerable.

## 1 · Method, and why the mod loaders are the right instrument

You cannot read a shipped game's feature list off its binary, but a mod
loader has to name every method it intercepts, because a hook exists
exactly where a modder needed to stop a player action. Oxide's patcher
project is that list as *data*, and `reference/rip-hooks.py` extracts it:

**852 patch entries · 277 distinct game classes · 38 categories**, from
`OxideMod/Oxide.Rust`'s `resources/Rust.opj` (MIT). Cross-checked for
coverage against `CarbonCommunity/Carbon.Hooks.{Base,Community}` (115 hook
names, mostly newer surfaces — clans, racked weapons, apartments, CUI
drag/drop), which is GPL-3.0 and therefore cited only, never extracted
from. Full output: `reference/rust-systems.txt`.

Three things fall out that a name list alone cannot give:

- **The class is the system.** `BasePlayer` carries 68 hooks,
  `PlayerInventory` 15, `BaseOven` 9, `ItemContainer` 3. Ranking classes
  by hook count ranks systems by how many verbs they actually have — which
  is the "how much is left to build" question, answered by measurement.
- **The signature is the payload.** `Item.MoveToContainer(ItemContainer,
  Int32, Boolean, Boolean, BasePlayer, Boolean)` states exactly what an
  item-move must carry: target container, target slot, and who did it.
  That is most of a wire format, sitting in someone else's build script.
- **The category is a second opinion.** Oxide grouped its own hooks 38
  ways. Where that grouping disagrees with ours, one of us is modelling
  the game wrong.

Cross-read against the five UI frames already in `Rust Images/` —
`inventory.jpeg`, `crafting.png`, `storageandtoolchest.jpeg`, and the two
maps. Where a hook table and a frame disagree about whether something is
one screen or two, the frame wins.

**Scale, stated honestly.** Of the 852 entries, roughly **205 sit in
categories `DESIGN.md` §2 scopes in** (Item 75, Structure 38, Resource 29,
Weapon 29, Crafting 8, Fuel 7, Traps 7, Primitive 7, World 3, TechTree 2),
about **353 are squarely out** (Vehicle 55, NPC 25, Vending 24, Electronic
23, Radio 21, Naval 19, Apartments 18, Turret 15, and the rest), and
**294 are `Player`/`Entity` plumbing** that splits both ways. The number
that matters is not 852. It is ~205, and we are perhaps a third of the way
into it.

## 2 · The systems, by depth — the "what needs fleshing out" answer

Depth is measured, not planned:

- **cut** — deliberately out of `DESIGN.md` §2's v1
- **none** — nothing exists
- **stub** — content or constants exist; no behaviour reaches them
- **v0** — one verb works end to end; the system's *shape* is absent
- **half** — several verbs live, load-bearing ones missing

| system | reference surface (classes · hooks) | Gates | what "deeper" means here |
|---|---|---|---|
| **Inventory / items** | `Item` 11, `PlayerInventory` 15, `ItemContainer` 3, `DroppedItem` 3 | **stub** | slots exist in the sim, six are drawn, and no verb moves anything between them. No move, split, stack, drop, or equip |
| **Containers** | `LootContainer` 8, `PlayerLoot` 6, `StorageContainer`, `Locker` | **v0** | bags only, take-all-that-fits. Boxes are placeable and inert (§4) |
| **Smelting / fuel** | `BaseOven` 9, `EntityFuelSystem` 5, +`Fuel` cat 7 | **stub** | the furnace is a proximity token for recipe gating, not a machine. No fuel, no burn, no input/output |
| **Building** | `BuildingBlock` 9, `Planner` 5, `BuildingPrivlidge` 4, `DecayEntity` 6, `Door` 3, `CodeLock` 7, `KeyLock` 4 | **half** | place, upgrade, door toggle, owner-bool lock, hearth feed. Missing: demolish, rotate, pickup, auth list, actual codes |
| **Crafting** | `ItemCrafter` 7, `Workbench` 6, `ResearchTable` 5, `Recycler` 6, TechTree 2 | **half** | queue, cancel, station-proximity gate all work. No blueprints, no research, no recycler — and `DESIGN.md` §8 says blueprints survive a wipe |
| **Combat** | `BaseProjectile` 8, `BaseMelee` 4, `BaseCombatEntity` 7, +`Weapon` cat 29 | **v0** | melee on one button. No reload, ammo switch, or ADS — five of eight button bits are still free — and no wound state |
| **Gather** | `ResourceDispenser` 4, `CollectibleEntity` 2 | **v0→ok** | swing plus the weak-spot bonus, which is the reference's own `OnDispenserBonus`. Closest system to done for its scope |
| **Survival** | metabolism, `MedicalTool` | **v0** | eat, drink, three meters. No wounded/revive — `woundedplayers.jpeg` is in our own reference set — no temperature, no comfort |
| **Respawn / bags** | `SleepingBag` 8 | **v0** | respawn-on-bag landed. No pick-your-bag screen (`world.rs:153` names it), no naming, no public/friends bit |
| **Teams** | `RelationshipManager` 12, `Clan` 8 | **none** | `DESIGN.md` §2 scopes teams UI out of v1 — informal groups work day one |
| **Trade / vending** | `VendingMachine` 17, `Shop` 3 | **none** | `ALPHA.md` holds it behind A2/A3; operator act, not a loop's |
| **Map / markers** | `MapEntity`, `BasePlayer` marker RPCs ~8 | **none** | the client already builds terrain from the seed, so the data is local and free |
| Vehicles · electricity · industrial · NPC · apartments · phones · fishing · missions | ~353 hooks | **cut** | `DESIGN.md` §2, explicitly |

**Read the table this way.** Three systems are `stub` — inventory,
containers, smelting — and all three are stubs for the *same reason*, which
is §6. Two are `half` in a way that shows: building has the verbs that
create and none that undo, and crafting has the loop that spends and none
that persists across a wipe.

## 3 · The screens

**HAVE** means a player can open it today.

> **One client.** This table used to annotate rows `(browser)` and `(native)`.
> The browser client is cut (`DECISIONS.md` 2026-08-06), so an unqualified
> **HAVE** now means the native client has it, and a `(browser)` annotation
> left anywhere below is a row nobody has re-checked since the cut.

| screen | reference evidence | Gates | note |
|---|---|---|---|
| Hotbar | `inventory.jpeg` (6 cells) | **HAVE** | keys 1–6, `sel` on the input frame. Native: `render/hud.rs` |
| Vitals (hp / hydration / calories) | `inventory.jpeg` bottom-right | **HAVE** | a 0-max meter is undrawn, not drawn empty |
| Chat + log | — | **HAVE** | local 20 m default, `/g` global. **Both clients** since 2026-08-06 — the native composer is `T`/`Enter`, and `/g` is stripped before `sanitize` so choosing the channel costs none of the 48-byte cap |
| Craft panel + queue | `crafting.png` | **HAVE** (native) · **PARTIAL** (browser) | The native client's `Tab` screen has the reference frame's shape: a category rail with live counts, a search box, a scrolling recipe grid with unaffordable rows dimmed rather than hidden, a detail pane with craft time and the station badge, the AMOUNT/ITEM TYPE/TOTAL/HAVE table, a favourite star, a quantity stepper with an "all you can pay for" step, and a queue strip with the head job's countdown and cancel on click. **The rail is not the reference's**: it buckets by station, deployable-ness and component-ness rather than by item class, because `EventMsg::Catalog` ships names and no class — see `crate::ui::craft`. The browser client is still the flat list |
| Build strip / ghost | `building.jpeg` | **HAVE** (both) | Browser: wheel cycles row, R/F change level, RMB places. Native: the **radial** the operator's frame shows — hold `B`, outer ring shape, inner ring material, centre reads name, blurb, hp and price, a segment the content has no piece for drawn dead — **and since 2026-08-06 it places.** `ui/place.rs` aims the cell (edge pieces re-address to the canonical west/north boundary; a foundation ignores the level stepper), `render/ghost.rs` draws it, and the ghost is coloured by a local verdict that answers the four refusals a client can check *in the server's own words*. A red ghost still sends — the verdict is advisory, and vetoing on it would make its blind spots authoritative |
| Compass strip | `gameplayfoundbase.jpeg`, `mapstylized.jpg` | **HAVE** | eight-point card plus degrees, from `look::bearing_deg` so the map and the strip cannot disagree. Bearing only — the reference also pins markers to it (death skull, map pin) and ours carries none, because no world position is exposed client-side and `ALPHA.md` §1 has a rule about position that an operator should read first |
| **Inventory grid** | `inventory.jpeg` — 6×4 main + wear doll + quick-craft | **PARTIAL** | 30 gated slots (6 belt + 24 grid), slot-indexed and drag/drop against the sim's move verb, **on both clients** — the native one landed 2026-08-06 with four gestures (drag, right-drag half, ctrl-drag one, right-click uses) and a ghost under the cursor. No wear doll and no quick-craft, which is what keeps it off HAVE |
| **Container panel** | `storageandtoolchest.jpeg` | **HAVE** | bag 30 / box 12 against `limits.rs`, cross-container drag both ends, close abandons what it cannot resolve. Was marked MISSING here long after it shipped — the judge's ranked fix 2 on `pass-20260805-074623-01` |
| **Loot panel (bags)** | — | **PARTIAL** | `backpack.rs` says a container UI "is its own slice" |
| **Wear / armor doll** | `inventory.jpeg`, four protection % readouts | **MISSING** | |
| **Map** | `mapraw.jpg`, `mapstylized.jpg` | **HAVE** (both) | `M` opens it. Both clients paint the whole island from the SAME `terrain::splat_from` the 3D ground blends by, hillshaded from the north-west; 16×16 grid, A–P west-east and 1–16 north-south; your position and heading. The native port (`ui/map.rs`, `render/map.rs`) is pure and gated in the code tier — the whole file is positional payload and a map has no golden, so the north/south flip and the pixel-centre sampling are both asserted, and the painted island is checked against `terrain::height` at ~190 samples with a non-vacuity count. Still no markers of any kind on either: what may be pinned at all is a design call |
| **Respawn screen** | — | **HAVE** (native) | `render/death.rs`, `Screen::Dead`. Dying used to end the session on this client: `dead` was set and read by nothing and `ACT_RESPAWN` had no key. Two anchors (bag / beach), the sentence carries no position (`ALPHA.md` §1, asserted structurally), an unknown `DEATH_BY_*` reports as itself, and the wake says which anchor actually answered. Missing against the reference: no pick-your-bag list, no kill-cam |
| **Team / contacts** | `inventory.jpeg` — `CONTACTS` tab, `CREATE TEAM` | **MISSING** | cut from v1 |
| **Code-lock keypad** | — | **MISSING** | |
| **Hearth auth list** | — | **MISSING** | |
| **Furnace panel** | — | **MISSING** | |
| **Repair bench** | — | **MISSING** | no condition, so no repair |
| **Research / tech tree** | — | **MISSING** | |
| **Recycler** | `DESIGN.md` §2 (haven) | **MISSING** | |
| **Vending / skin vendor / bank** | `DESIGN.md` §2–3 (haven) | **MISSING** | A2/A3 |
| **Server select / intro** | the reference's main menu (PLAY GAME → server list) | **HAVE** | the native client's first screen (`render/menu.rs`). Title, a status line that always says *why* the list is what it is, one row per shard from `scry-shardlist-v1`, plus a Direct row that is always present so a failed fetch never leaves nothing to click. Click or number key; Esc quits. **A failed connect returns here with the reason** instead of `exit(1)`, which is the whole point of the state machine. Skipped by `--capture` and by `--server` (the launcher already picked). Missing against the reference: no ping, no player counts (nothing measures them — `DECISIONS.md` §open), no favourites, no filter/search, no modded/official split. Now also carries an `S  Settings` row, and hands off to `Screen::Loading` rather than straight to the world |
| **Loading screen** | the reference's join screen | **HAVE** | `render/loading.rs`. The state between the welcome and a playable world, which is ~25 frames of ring building at one chunk of each per frame. The bar is the MEAN of the three rings' resident counts and the counts are printed beside it (`GROUND n/25  SCATTER n/25  CLUTTER n/25`, off the shipped ring sizes); nothing on this screen reads a clock, and the far mesh is a separate bit so a full bar cannot end the screen without a horizon. The world renders behind the opaque overlay, which is where the pipelines specialize. Esc goes back to the server list. Missing against the reference: no tip text, no map name, no per-player join queue |
| **Settings / options** | `Rust Images/` options frames | **PARTIAL** | `render/settings.rs`, reachable from the intro screen (`S`) and from the Esc menu. The reference's shape — a category rail with the selected one blocked out in olive, a pane of label/control rows — with six categories and eight settings that do something: vsync, fullscreen, field of view, mouse sensitivity, invert look, and the three audio buses (master, game, ambience — the reference's `audio.master`/`audio.game`/`audio.ambience`, 0-100% in tenths). **A category with nothing behind it says so in a sentence rather than drawing greyed rows**, which is why AUDIO names music, voice chat and sound occlusion as facts rather than drawing three dead sliders. KEYBINDS is the read-only bind list this table used to call missing. Missing against the reference: no search, no rebinding, no per-setting reset, and nothing is written to disk — settings live for the run |
| **Escape / options** | the reference's pause menu | **HAVE** | `render/pause.rs`. Esc in the world opens the intro screen seen from inside it — same title, same rows, same number keys — with four verbs: resume, settings, disconnect, quit. **Disconnect actually leaves**: `WorldEntity` marks every root the world spawned and `render::world_teardown` despawns the set, resets the four ring indexes and drops `Net`/`WorldId`, so the server list is reachable without killing the process. The pointer is released on the way in and taken back on resume, and the input frame is zeroed so pausing mid-stride does not walk the player through the menu. **An open in-game panel gets Escape first** and consumes the press, so closing an inventory does not also open this |

## 4 · Dark content — declared, baked, reachable by nothing

The sharpest finding, because it is not "we have not built X" but "we ship
a thing that does nothing." Each already survives `test_content`.

1. **`ARCH_BOX`** — `box_small` (150 hp) and `box_large` (250 hp) bake
   through `content/bake.rs` into a live archetype. `ARCH_BOX` appears in
   exactly two places in the whole workspace: the constant, and the bake
   arm that produces it. A player can craft a box, place a box, and then
   owns a box.
2. **`ARCH_FIRE`** — `fire_pit`, 100 hp, same story, zero references.
3. **`ARCH_FURNACE`** — reachable, but only as a proximity token for
   station-gated recipes (`craft.rs:205`). Not a machine: no fuel, no
   input, no output, no burn.
4. **`content/armor.toml`** — rows with `slot` and `reduction_pct`,
   validated against `balance.rs`'s anchor-2 band on every boot, read by
   no combat path. There is no way to wear one.
5. **`content/skins.toml`** — empty, and correctly so: `ALPHA.md` §2 holds
   it dark until A3. Listed so a later pass does not mistake it for the
   same defect as the four above.

Items 1–4 are the cheapest real gameplay in the repo: content, bake,
validation and placement all exist already.

## 5 · What the addon ecosystem says about the gaps

The plugin catalog is not a target, but the shape of it is evidence. The
perennial top installs are, near enough: Kits, GUIShop, Backpacks, Clans,
NTeleportation, Furnace Splitter, Vanish, NoEscape, Raidable Bases,
Economics.

Three of those ten — **Backpacks, Furnace Splitter, GUIShop** — exist
because inventory space, oven management and trade are the surfaces
players touch most and vanilla's are the most cramped. That is direct,
independent evidence for §2's three stubs: an entire modding economy grew
on top of exactly the systems we have not built.

Two more — **Kits, NTeleportation** — solve naked-respawn and travel-time
friction. Both run against `DESIGN.md` §1's pillars and should stay
unbuilt. Noted so "popular plugin" is never read as "missing feature."

**Economics** is a currency backbone that does nothing alone and that
everything else hooks into. OBOL already holds that role under the
what the house does not sell (`BUSINESS.md`), so the lesson is only that the hook-point
matters.

## 6 · The keystone, and the order the dependencies impose

Not a queue — `NOW.md` is the queue. This is the order the dependencies
*already* impose, so a cut into `NOW.md` does not invert them.

**Everything in §2 marked `stub` is one missing verb.** An inventory grid,
a storage box, a furnace, and a wear doll are all the same screen: two
containers and a way to move an item from one to the other. The reference
states its own signature —

```
Item.MoveToContainer(ItemContainer target, Int32 slot, Boolean allowStack,
                     Boolean ignoreStackLimit, BasePlayer player, Boolean ...)
ItemContainer.CanAcceptItem(Item, Int32 slot)   // the refusal
Item.SplitItem(Int32)                            // the other half
Item.MaxStackable()                              // what bounds both
```

— which is a wire format with our names on it: a container address, a slot
index, a count, and a sim-side refusal. It is a wall-6 change (version bump
plus regenerated goldens in the same commit) and it is the single
highest-leverage protocol slice on this page.

1. **Container addressing + the move verb.** Nothing below is possible
   without it; each panel built before it would invent its own.
2. **Containers** (`ARCH_BOX`) — (1) with a second container on screen.
   Retires two dark archetypes and upgrades bag-looting from take-all to
   per-slot for free.
3. **Furnace as a machine** (`ARCH_FURNACE`, `ARCH_FIRE`) — (1) plus a burn
   tick. `BaseOven`'s 9 hooks are the shape; `OnFindBurnable` and
   `OnFuelConsume` are the two that matter.
4. **Wear slots** — (1) plus a slot-type check. Lights `armor.toml` and the
   four protection readouts in `inventory.jpeg` in one move.
5. **The respawn screen** — already named as its own slice in `world.rs`.
6. **Code locks and hearth authorization** — the first item here that is a
   *social* mechanic rather than a container, and the point at which a base
   means something against another player.

Each of 1–4 is bounded, has its content already validated, and touches no
wall except the wire version on (1).
