# Gates · MENUS.md — the interaction surface, ours against the reference

An audit, not a law and not a queue. It answers one question: **which
screens and which verbs does the reference game have, which do we have,
and what is the shape of the difference.** `NOW.md` stays the only list
that answers "what next" — items get cut from this survey into that queue,
never the other way. Numbers stay in `CONTENT.md`; nothing here is a knob.

Dated 2026-08-04. A row that disagrees with the code is wrong — fix the
row.

## 1 · Method, and why the plugin ecosystem is the right instrument

You cannot read a shipped game's menu list off its binary, but you can read
it off the mod loaders, because a hook exists exactly where a modder needed
to intercept a player action. Two loaders, both public:

- **Oxide/uMod** — `OxideMod/Oxide.Rust`, `resources/Rust.opj`. The patcher
  project is the whole hook table as data: 835 patch entries, **734 unique
  hook names** across 7 assemblies. This is the closest thing that exists to
  a machine-readable enumeration of the game's verbs.
- **Carbon** — `CarbonCommunity/Carbon.Hooks.Base` and `.Community`, the
  Harmony-based successor. **115 hook names**, mostly the newer surfaces
  (clans, racked weapons, apartments, CUI drag/drop) plus Carbon's own.

A hook name is a verb with a menu behind it. `OnLootEntity` means a
container panel exists; `OnItemSplit` means an inventory grid with a split
gesture exists; `OnCodeEntered` means a keypad exists. Counting them is how
this table got built rather than guessed.

Cross-checked against the five UI frames already in `Rust Images/` —
`inventory.jpeg`, `crafting.png`, `storageandtoolchest.jpeg`, `mapraw.jpg`,
`mapstylized.jpg` — which are the same reference set `ART.md` measures
against. Where a hook and a frame disagree about whether something is one
screen or two, the frame wins.

**What was deliberately not counted.** The plugin *catalog* (Kits, GUIShop,
Backpacks, Clans, NTeleportation, Furnace Splitter, Raidable Bases) is a
list of menus *operators bolt on*, not menus the game has. It is in §5
because it says something useful about which of our gaps are load-bearing,
but it is not a target — most of it is content, and some of it (teleport,
kits) is against the pillars.

## 2 · The screens

Status is measured, not planned. **HAVE** means a player can open it today.

| screen | reference evidence | Gates | note |
|---|---|---|---|
| Hotbar | `inventory.jpeg` (6 cells) | **HAVE** | `hud.js`, 6 cells, keys 1–6, `sel` on the input frame |
| Vitals (hp / hydration / calories) | `inventory.jpeg` bottom-right, 3 bars | **HAVE** | `Hud.setVitals`; a 0-max meter is undrawn, not drawn empty |
| Chat + log | — | **HAVE** | composer owns the keyboard; local 20 m default, `/g` global |
| Craft panel + queue | `crafting.png` | **PARTIAL** | flat list, click to enqueue, shift-click ×5. No categories, no search, no detail pane, no ingredient table, no favourites, no quantity stepper |
| Build strip / ghost | `building.jpeg` | **HAVE** | wheel cycles row, R/F change level, RMB places |
| **Inventory grid** | `inventory.jpeg` — 6×4 main + wear doll + quick-craft | **MISSING** | the sim has `INV_SLOTS`; the client shows 6 of them and nothing else |
| **Container / storage panel** | `storageandtoolchest.jpeg` | **MISSING** | see §3 — the boxes are already placeable and inert |
| **Loot panel (bags)** | — | **PARTIAL** | `Loot` is take-all-that-fits, nearest bag. `backpack.rs` says a container UI "is its own slice" |
| **Wear / armor doll** | `inventory.jpeg`, four protection % readouts | **MISSING** | `content/armor.toml` is validated by the balance band and read by nothing |
| **Map** | `mapraw.jpg`, `mapstylized.jpg` | **MISSING** | client already generates terrain from the seed, so the data is local |
| **Respawn screen** | — | **MISSING** | `world.rs:153` names it: respawn-on-bag landed, the pick-your-bag choice did not |
| **Team / contacts** | `inventory.jpeg` — `CONTACTS` tab, `CREATE TEAM` | **MISSING** | `DESIGN.md` §2 scopes teams UI out of v1 |
| **Code-lock keypad** | — | **MISSING** | `Lock` is an owner-only bool. No code, no auth list |
| **Hearth auth list** | — | **MISSING** | `Feed` exists; authorize/deauthorize/clear do not |
| **Furnace / oven panel** | — | **MISSING** | see §3 |
| **Repair bench** | — | **MISSING** | no condition, so no repair |
| **Research / tech tree** | — | **MISSING** | `DESIGN.md` §8 says blueprints survive a wipe; nothing mints one |
| **Recycler** | `DESIGN.md` §2 (haven) | **MISSING** | scoped, unbuilt |
| **Vending / skin vendor / bank** | `DESIGN.md` §2 (haven), §3 | **MISSING** | gated behind A2/A3 by `ALPHA.md`; operator act, not a loop's |
| **Escape / options** | — | **MISSING** | no settings, no disconnect, no keybind list |
| Sign / note, missions, phone, item radial | hooks + `inventory.jpeg` MISSIONS badge | **OUT** | not in `DESIGN.md` §2's v1 cut |

## 3 · The verbs

Gates has **eleven** action messages (`protocol::ActionMsg`) and **three**
input-frame buttons (`BTN_SPRINT`, `BTN_CROUCH`, `BTN_PRIMARY` — five of
eight bits still free).

| verb | Gates | reference hook(s) |
|---|---|---|
| move / look / sprint / crouch | **HAVE** (`InputFrame`) | `OnPlayerInput`, `OnPlayerTick` |
| swing / attack | **HAVE** (`BTN_PRIMARY`) | `OnPlayerAttack`, `OnMeleeAttack`, `OnDispenserGather` |
| craft, cancel craft | **HAVE** (`Craft`, `CraftCancel`) | `OnItemCraft`, `OnItemCraftCancelled`, `CanCraft` |
| place building piece | **HAVE** (`Place`) | `OnConstructionPlace`, `CanBuild`, `OnPayForPlacement` |
| deploy | **HAVE** (`Deploy`) | `OnItemDeployed`, `CanDeployItem`, `OnEntityBuilt` |
| upgrade piece | **HAVE** (`Upgrade`) | `OnStructureUpgrade`, `CanAffordUpgrade`, `CanChangeGrade` |
| open/close door | **HAVE** (`Use`) | `OnDoorOpened`, `OnDoorClosed` |
| lock/unlock | **PARTIAL** (`Lock`, bool) | `CanLock`, `OnCodeEntered`, `CanChangeCode`, `CanUseLockedEntity` |
| feed hearth | **HAVE** (`Feed`) | `OnCupboardAuthorize` family — ours is the upkeep half only |
| loot bag | **PARTIAL** (`Loot`) | `OnLootEntity`, `OnLootItem`, `OnPlayerLootEnd` |
| eat | **HAVE** (`Consume`) | `OnItemUse`, `OnHealingItemUse` |
| drink | **HAVE** (`Drink`) | `OnPlayerDrink` |
| **move / split / stack item** | **MISSING** | `CanMoveItem`, `OnItemSplit`, `OnItemStacked`, `OnMaxStackable`, `CanAcceptItem` |
| **drop item** | **MISSING** | `OnItemDropped`, `CanDropActiveItem`, `OnPlayerDropActiveItem` |
| **open container** | **MISSING** | `CanLootEntity`, `OnItemAddedToContainer`, `OnItemRemovedFromContainer` |
| **equip / wear** | **MISSING** | `CanWearItem`, `CanEquipItem`, `OnClothingItemChanged` |
| **jump** | **MISSING** | `collide.rs:14` names its own absence |
| **reload / switch ammo / ADS** | **MISSING** | `OnWeaponReload`, `OnMagazineReload`, `OnAmmoSwitch` |
| **demolish / rotate piece** | **MISSING** | `OnStructureDemolish`, `CanDemolish`, `OnStructureRotate` |
| **pick up deployable** | **MISSING** | `CanPickupEntity`, `OnEntityPickedUp` |
| **repair** | **MISSING** | `OnItemRepair`, `OnStructureRepair` |
| **research / study blueprint** | **MISSING** | `OnItemResearch`, `OnPlayerStudyBlueprint`, `OnTechTreeNodeUnlock` |
| **recycle** | **MISSING** | `OnItemRecycle`, `OnRecyclerToggle`, `CanBeRecycled` |
| **smelt / cook / fuel** | **MISSING** | `OnOvenToggle`, `OnOvenCook`, `OnFuelConsume`, `OnFindBurnable` |
| **wound / revive** | **MISSING** | `CanBeWounded`, `OnPlayerWound`, `OnPlayerRevive` — `woundedplayers.jpeg` is in the reference set |
| **hearth authorize** | **MISSING** | `OnCupboardAuthorize`, `OnCupboardDeauthorize`, `OnCupboardClearList` |
| **team invite / accept / leave** | **MISSING** | `OnTeamCreate`, `OnTeamMemberInvite`, `OnTeamAcceptInvite`, `OnTeamLeave` |
| **map marker** | **MISSING** | `OnMapMarkerAdd`, `OnMapMarkerRemove` |

## 4 · Dark content — declared, baked, and reachable by nothing

The sharpest finding, because it is not "we have not built X" but "we ship
a thing that does nothing." Each of these already survives `test_content`.

1. **`ARCH_BOX`** — `box_small` (150 hp) and `box_large` (250 hp) bake
   through `content/bake.rs` into a live archetype. Grepping `crates/`
   finds `ARCH_BOX` in exactly two places: the constant, and the bake arm
   that produces it. A player can craft a box, place a box, and then has a
   box. No container, no open verb, no slots.
2. **`ARCH_FIRE`** — `fire_pit`, 100 hp, same story, zero references.
3. **`ARCH_FURNACE`** — reachable, but only as a *proximity token* for
   station-gated recipes (`craft.rs:205`). It is not a machine: no fuel, no
   input, no output, no burn.
4. **`content/armor.toml`** — six-odd rows with `slot` and
   `reduction_pct`, validated by `balance.rs`'s anchor-2 band on every
   boot, and read by no combat path. There is no way to wear one.
5. **`content/skins.toml`** — empty, and correctly so: `ALPHA.md` §2 holds
   it dark until A3 and rows land by operator act. Listed here so a later
   pass does not mistake it for the same defect as the four above.

Items 1–4 are the cheapest real gameplay in the repo: the content, the
bake, the validation and the placement all exist already.

## 5 · What the addon ecosystem says about the gaps

The plugin catalog is not a target, but the *shape* of it is evidence.
The perennially top-installed plugins are, near enough in order: Kits,
GUIShop, Backpacks, Clans, NTeleportation, Furnace Splitter, Vanish,
NoEscape, Raidable Bases, Economics.

Three of those ten — **Backpacks, Furnace Splitter, GUIShop** — exist
because inventory space, oven management and trade are the surfaces
players touch most and vanilla's are the most cramped. That is a direct
signal about §4: the container gap is not a nice-to-have, it is the
surface an entire modding economy grew on top of.

Two more — **Kits, NTeleportation** — solve the naked-respawn and
travel-time frictions. Both are against `DESIGN.md` §1's pillars, and both
should stay unbuilt. Noted so that "popular plugin" is never mistaken for
"missing feature."

**Economics** is the one to read carefully: it is a currency backbone that
does nothing alone and that everything else hooks into. Gates already has
that role spoken for by OBOL (`DESIGN.md` §3.1) under the never-table, so
the lesson is only that the hook-point matters, not that we need the
plugin.

## 6 · What this survey suggests, in dependency order

Not a queue — `NOW.md` is the queue. This is the order the dependencies
actually impose, offered so a cut into `NOW.md` does not invert them.

1. **An inventory grid, and the item-move verb under it.** Everything
   below is a panel that moves items between two containers; without a
   move verb, each one has to invent its own. One wire verb
   (`MoveItem { from_container, from_slot, to_container, to_slot, count }`)
   plus a container addressing scheme is the single highest-leverage
   protocol slice on this page. It is a wire change — wall 6, so a version
   bump and regenerated goldens in the same commit.
2. **Containers** (`ARCH_BOX`), which is (1) with a second container on
   screen. Retires the dark box, and turns bag-looting from take-all into
   per-slot for free.
3. **Furnace as a machine** (`ARCH_FURNACE` + `ARCH_FIRE`), which is (1)
   with a burn tick. Retires two more dark archetypes.
4. **Wear slots**, which is (1) with a slot-type check, and which lights
   `armor.toml` and the protection readout in one move.
5. **The respawn screen**, already named as its own slice in `world.rs`.
6. **Code locks and hearth authorization** — the first thing on this list
   that is a *social* mechanic rather than a container, and the point at
   which a base means something against another player.

Each of 1–4 is bounded, has its content already validated, and touches no
wall except the wire version on (1).
