# reference/CRAFTING.md — how the reference game tells a player what crafting is doing

Ripped facts, not design. `rust-systems.txt` answers *what systems exist*,
`DOORS.md` *who is allowed through a door*, `DURABILITY.md` *how an item
wears out*; this file answers **what the crafting surface SAYS** — what the
menu, the queue, the HUD and the wire indicate while a craft is queued,
running, refused and done — because on 2026-09-13 the operator put two
frames of the reference beside two of ours and asked what it does that ours
does not. `MENUS.md` §3 already scored our craft panel **HAVE**; this is why
HAVE and *looks the same* are different claims.

Dated 2026-09-13. §9 is the part that changes what we build.

## 0 · Provenance — read this first

Five sources, ranked, and the ranking is the honest part.

1. **`reference/rust-systems.txt`** — in this tree, MIT, regenerable. A
   *hook* table, so it proves the **shape**: which class owns the queue,
   which verbs exist on it, and what each one takes. §1 reads the object
   model off it and nothing more.
2. **The developer's own pages, fetched whole.** `rust.facepunch.com` and
   `wiki.facepunch.com` were both reachable from this box on 2026-09-13
   (`SOURCES.md` §0: probe, do not trust either prior claim) — Devblog 45,
   Devblog 62, Devblog 187, the March 2025 Crafting Update and the wiki's
   `item/workbench1` page arrived as pages, not summaries. A sentence a
   developer published about their own work.
3. **The public client command list**, mirrored on GitHub
   (`QuietusPlus/Rust-Easy_Config`, fetched whole). Names only, no
   semantics — `note.craft_add` is a fact that a command exists, and what it
   carries is inference, marked as such in §4.
4. **The operator's two frames of the reference** (this conversation). The
   same posture as `MENUS.md`'s frames: strongest for LAYOUT — what sits
   where, what is a picture and what is a word — and silent on mechanism.
   §3 is read off them and says so line by line.
5. **Plugin pages.** `umod.org` returned **403 through this proxy** for
   every page, so Craft UI, Craft Queue Saver, Game Tip API and UI Notify
   arrived as *search-result summaries* (`DOORS.md` §0's caveat, in full);
   `codefling.com`'s Crafting Panel page and `rustly.com`'s guide fetched
   whole; the Fandom wiki answered 402. A plugin is evidence of two things —
   what the vanilla surface does (a plugin that "recreates the stock UI"
   lists it) and what players felt it under-signals (what the plugin adds).

Nothing here was decompiled. Nothing here ships: no asset, no name, no
number copied into `content/` without being re-priced.

## 1 · The object model, read off the hook table

```
ItemCrafter        [4]  CanFastTrackCraftTask · FastTrackTask(Int32)
                        OnItemCraft           · CraftItem(ItemBlueprint, BasePlayer,
                                                 Item/InstanceData, Int32, Int32, Item, Boolean, Int32)
                        OnItemCraftCancelled  · CancelTask(Int32)
                        OnItemCraftFinished   · FinishCrafting(ItemCraftTask)
                   [2]  CanCraft              · CanCraft(ItemBlueprint, Int32, Boolean)
                        OnIngredientsCollect  · CollectIngredients(ItemBlueprint, ItemCraftTask,
                                                 Int32, BasePlayer, Boolean)
PlayerBlueprints   [1]  CanCraft              · CanCraft(Int32, Int32, BasePlayer)
IndustrialCrafter  [1]  OnItemCraft           · RunJob()
Workbench          [4]  OnExperiment{Start,Started,End,Ended}
                   [2]  OnTechTreeNodeUnlock(ed) · RPC_TechTreeUnlock
ResearchTable      [4]  OnItemResearch(ed) · OnResearchCostDetermine · ScrapForResearch
PlayerInventory         OnInventoryNetworkUpdate · SendUpdatedInventoryInternal(
                                                 PlayerInventory/Type, ItemContainer, NetworkInventoryMode)
```

Seven structural facts fall out of that, and they are the valuable half of
this document:

1. **The queue is an object on the player, and a job has an id.** Both
   `CancelTask` and `FastTrackTask` take an `Int32` — a task uid, not a
   position. A cancel names *the job*, so a queue that moved under the
   click cannot cancel the wrong one. Ours sends an index
   (`CancelJob(usize)` → `ACT_CANCEL`), which `panels/craft.rs` already
   flags as "only valid for as long as the queue it was drawn from".
2. **Ingredients are collected at enqueue.** `CollectIngredients` is a step
   of `CraftItem`, taking the blueprint, the task and the amount — so the
   inventory drops the moment CRAFT is clicked, and the HAVE column falls
   with it. Ours does the same (`sim-core/src/craft.rs`: "inputs consumed
   up front for the whole batch"), and the operator's second frame shows it
   — HAVE 325 → 25 on one click.
3. **The blueprint gate is a separate object asked separately.**
   `PlayerBlueprints.CanCraft(itemid, skin, player)` beside
   `ItemCrafter.CanCraft(blueprint, amount, free)`: *do you know it* and
   *can you pay for it here* are two predicates on two classes. Same split
   as our `RecipeDef::blueprint` against `Player::known` versus the
   station-and-inputs check — and it is why a locked cell and an
   unaffordable cell must not draw the same (§3).
4. **Fast-track is a verb.** A queued job can be pulled to the head
   (`CanFastTrackCraftTask`). Community guides bind it to a right-click on
   the queue's cancel cross; the hook proves the verb, the binding is tier 5.
5. **"Free" rides the same verb.** `CanCraft(…, Boolean)` and `CraftItem(…,
   Boolean, …)` carry an admin/instant flag through the ordinary path —
   `craft.instant` is not a second crafter.
6. **A machine crafts through the same hook name.** `IndustrialCrafter`
   binds `OnItemCraft` to `RunJob()` — one verb, two callers, which is the
   shape `MENUS.md` §4 wants for the furnace it calls dark content.
7. **The bench is a place, not a crafter.** `Workbench`'s hooks are
   experiments and the tech tree; nothing on it crafts. The crafter checks
   proximity — `craft.rs`'s `bench_near` at enqueue is the same reading.

And one wire fact: `OnInventoryNetworkUpdate` names a `NetworkInventoryMode`,
so the inventory reaches the client as a container push. The HAVE column is
the inventory mirror; no craft message carries it.

## 2 · The queue as they shipped it — three devblogs and a wiki page

**Devblog 45 (2015).** *"When you are crafting something a timer will show
up, along with how many others are queued."* *"You can cancel an item you
have queued by right clicking."* And, unprompted: *"This is a lot more
simplified compared to the previous version — and you do lose a degree of
control… but we'll see how it goes."* The blueprint list gained an *All*
category that *"show[s] how many you have unlocked"* — the rail's counts are
that old.

**Devblog 62 (Garry, 2015) — the redesign the current screen descends
from.** Click a blueprint to see *"required resources, craft quantity,
duration, and current ingredient amounts"*; *"+/-"* buttons and a *">"*
button to maximise, with manual entry; the queue *"at the bottom"*, where
*"the currently crafting item is green"*; a cross icon cancels; the queue
scrolls sideways by dragging. Then the line this document was written for:

> *"It also shows a notice on your HUD, so you can see how long is left on
> your craft without having to keep opening the inventory menu."*

**Devblog 187.** The *quick craft* column — the list of things you can
craft right now, beside the inventory — repopulated as items became
craftable and *"[took] a very long time as it tried to find the appropriate
skin icon"*, freezing during a loot; fixed by making *"the quick craft items
always show as the default icon."* Two facts: quick craft is a **live
affordability list**, and its icon is the trap.

**The wiki, `item/workbench1`, fetched whole.** Craft time for the level-1
bench itself: **30 s with no bench, 15 s at level 1, 7 s at level 2, 7 s at
level 3.** A halving per rung above the recipe's own, with a floor — the
search summary that reached this box said *"1/8 at three levels below"*, and
the page disagrees (7 s, not 3.75). The page wins; the summary is what
`DOORS.md` §0 warned a summary does.

**The March 2025 Crafting Update.** Two new benches (cooking, engineering —
*"a dedicated tech tree for water, electricity, and industrial systems"*),
*"over 40 items"* with shorter craft times, a crafting-quality potion. No
change to the queue or its indicators — the 2015 surface is still the one.

## 3 · The menu, read off the operator's frames

Two frames, one player, searches `door` and `shack`. Layout only; every
line here is a picture and none is a mechanism.

**The shell.** Two tabs across the top with glyphs — **CONTACTS**,
**INVENTORY** — the crafting screen being a third state of the same frame.
A dark, warm, translucent panel; uppercase condensed bold everywhere.

**The rail.** FAVOURITE, COMMON, then eleven *classes* with a count each:
CONSTRUCTION 13, ITEMS 27, RESOURCES 2, CLOTHING 25, TOOLS 10, MEDICAL 1,
WEAPONS 11, AMMO 5, TRAPS 1, ELECTRICAL 3, OTHER 1. **The counts did not
move between the two searches**, so they are the class's size and not the
filtered set — the search narrows the grid and leaves the rail alone.
FAVOURITE and COMMON carry no count. The selected class is a solid blue
block with a white glyph.

**The grid.** Every cell is **a picture of the item**. A locked item stays
in the grid at reduced brightness with a **padlock glyph over the picture**,
and is still clickable — it opens the detail pane, which is how a player
finds out what a research table is for. A favourite carries a **★ at the
cell's top-right corner**. The search field spans the grid's width below it
and is **highlighted amber while it has focus** (the `door` frame) and
plain when it does not (`shack`).

**The detail pane.** The picture again, the name in uppercase, a
**★ FAVOURITED** toggle beneath the picture, then **a paragraph of
description** (*"A cheap door to secure your base. Its vulnerability to fire
and weak explosive resistance makes the door a temporary solution…"*, and the
pickup instructions in the same paragraph). A **SKINS** section with its own
search and a picture of the selected skin. Top-right, two badges: a **clock
with the seconds** (`15.0`) and a **`+1`** — the count one craft yields. The
ingredient table headed **AMOUNT · ITEM TYPE · TOTAL · HAVE**, and when HAVE
is short the AMOUNT and TOTAL cells go **amber** (`600 · Wood · 600 · 33`).
Bottom: the stepper **− [1] + ▶|** and **CRAFT**, lit when affordable and
**dimmed to the panel's grey when not** (`shack`, 33 of 600 wood).

**The queue.** A strip labelled **CRAFTING QUEUE** across the bottom. A
queued job is **its picture with a green chip `⏱ 14s`** — icon and
countdown, no words. Devblog 62 says the running one is green and the cross
cancels; the frame shows the chip and no cross at rest, so the cross is
likely a hover state (unverified).

**The HUD around it.** A hotbar of pictures with stack counts; the vitals as
three filled bars with glyphs. The craft notice Devblog 62 describes is not
in either frame — both were shot with the menu open.

## 4 · The wire behind the indicators — the `note.*` vocabulary

The public client command list carries four commands the **server** invokes
on the client, and their names are the whole of what tier 3 proves:

```
note.craft_add      note.craft_start      note.craft_done      note.inv
inventory.toggle    inventory.togglecrafting
```

Read as an event vocabulary — and this paragraph is inference from the
names, not a fetched semantics — they are: *a job entered the queue*, *the
head started* (this is the one that must carry a duration, because it is
what a countdown counts from), *a unit finished*, and *an inventory delta
note*. Three consequences:

1. **The queue is server state pushed as events**, not a client-side
   fiction; the client counts down locally from what `craft_start` gave it.
   Ours is the same shape — `EventMsg::CraftQ` is "the authoritative own
   craft queue after a change" and `craft_eta_ticks` is the head's remaining
   ticks — so the HUD notice in §2 needs **no wire change** from us.
2. **`note.inv` is its own lane.** An item arriving or leaving is a note
   beside the vitals, separate from chat and from the toast. A craft's
   consumed inputs and its finished output both surface there.
3. **Crafting is its own toggle.** `inventory.togglecrafting` beside
   `inventory.toggle`: their `Q` opens the craft state of the inventory
   frame directly, which is exactly the `Q`-only-opens rule `MENUS.md`
   records for ours.

Beside them, `gametip.showtoast <style> <text>` (search-summary tier: `0`
blue, `1` red, `3` short blue) is the server-driven banner — the *"requires
workbench"* class of message rides it, and the umod **Game Tip API** exists
to wrap it, which is how a search summary can be trusted that far.

## 5 · What the plugins say, and what each is evidence of

| plugin | source tier | what it does | what it is evidence of |
|---|---|---|---|
| **Craft UI** (umod) | summary | *"a complete recreation of the stock Facepunch crafting UI using CUI"* — adds **a red close button**, a **CRAFT button that turns green when you can craft the amount (grey when you can't)**, numbers to craft; admins set ingredients, block items, set craft rate | The vanilla surface's shape (rail, grid, detail, queue), and one indicator players wanted louder: **affordability as a colour on the button** |
| **Craft Queue Saver** (nivex) | summary | saved the queue on disconnect and shutdown — *"now part of the game"* | The queue persists with the player. Ours: `NOW.md` §0y's persistence questions do not mention it |
| **Crafting Panel** (codefling) | whole | *"~90% similar to the design of Rust's in-game crafting panel"*: favourites, search, *"functional queue"*, item cooldowns, economy purchase, *"notifications in the status bar on the right"*, *"customizable sound effects"* | A modder's inventory of the vanilla panel: favourites + search + queue + a **right-side status bar** + a **sound** |
| **Quality Crafting** | summary | crafting skill and quality tiers, with per-craft notifications | Craft completion is a notification event modders hang on |
| **UI Notify / Notify** (Mevent), **Game Tip API** | summary | toast plumbing other plugins call | The toast is a shared lane, not a crafting feature |
| **Crafting Controller**, instant-craft plugins | summary | per-item time multipliers and blocklists; `craft.instant` is the vanilla admin convar | Time and price are server knobs; nothing about them is client-side |

## 6 · The bench ladder is a time rebate

§2's wiki row is the whole mechanism: a recipe at rung *n* crafted at a bench
of rung *n+1* takes half, at *n+2* a quarter, and the ladder floors (7 s
twice). `RIPLIST.md` §2 row 3 and `NOW.md` §0tt already carry this as the
unbuilt craft rebate; this file adds only that the reference's numbers were
read off a fetched page rather than a summary.

## 7 · What this document could not settle

- **Where the HUD notice sits and what it draws** (a picture, a bar, a
  number). Devblog 62 says it exists; neither frame has the menu closed.
  The operator plays the reference and can answer this by looking, which
  is cheaper than another search.
- **Whether the right-click cross is fast-track.** The verb is in the hook
  table; the binding is a community guide.
- **What COMMON is.** A bucket with no count; possibly the last-crafted or
  the starter set. Not taken (§9.2).
- **The small amber corner mark** on the first cell of the `door` frame.
  Not the favourite star (that is a ★ elsewhere in the same grid).
- **Whether a queue pauses while the player is dead or asleep.** No source
  reached said either way.

## 8 · Sources

- `reference/rust-systems.txt` §Item (`ItemCrafter`, `ResearchTable`,
  `PlayerInventory`), §Crafting, §Industrial, §Workbench — in tree.
- Devblog 45, Devblog 62, Devblog 187 — `rust.facepunch.com/news/…`,
  fetched whole 2026-09-13.
- *Crafting Update* (March 2025) — `rust.facepunch.com/news/crafting-update`,
  fetched whole.
- `wiki.facepunch.com/rust/item/workbench1` — fetched whole.
- `QuietusPlus/Rust-Easy_Config` `docs/Rust-Commands.md` — fetched whole
  (raw GitHub).
- `codefling.com/plugins/crafting-panel` — fetched whole.
  `rustly.com/guides/rust-crafting-guide/` — fetched whole (locked items
  *"greyed out with a little lock on it"*; *"the craft menu lists whichever
  you're short on"*).
- umod: Craft UI, Craft Queue Saver, Quality Crafting, UI Notify, Game Tip
  API — **search summaries only** (403).
- The operator's two reference frames, 2026-09-13.

## 9 · What it means for us

### 9.1 · The audit — our frame against theirs, ranked by what a player sees

Ours, for the record (the operator's two frames of Gates, same day): a
seven-bucket rail with filtered counts, an eight-column grid of white glyphs
tinted by state with the word `LOCKED` under a gated one, a search rectangle,
a detail pane (name, seconds, `* favourite`, the four-column table with
shortfalls in red, `− 1 + >|`, CRAFT, *"you can pay for N"*), and a queue
strip reading `Wooden Spear x1 · 10.0s click to cancel`. Every number on it is
right and gated (`ui/craft.rs`). What differs is what is a picture and what
is a word, and what happens when the menu is closed.

1. **Nothing on the HUD while the menu is closed.** Devblog 62's one
   sentence. We hold `jobs`, `jobs_count` and `craft_eta_ticks` in
   `ClientCore` already (`EventMsg::CraftQ`), so a chip — the head job's
   picture and its countdown, beside the hotbar — is a draw and no wire.
   **First, because it is the cheapest thing on this list and the only one
   a player meets every craft.**
2. **The queue is words.** Theirs: picture + green `⏱ 14s`. Ours: a label
   with `click to cancel`. Same data; `build_queue` is the only site.
3. **Every cell is a picture there and a glyph here.** The structural fix is
   not a bigger icon set: it is **rendering our own models to item images**
   at bake time — `modelview --shot` already shoots a glTF headless, and
   `assets/models/MANIFEST.md` lists what ships. Items with no model keep
   the glyph (`Icons::item` already falls back). Our own renders, so the
   licence rail is untouched; the IP rail is untouched because nothing is
   traced. This is the one item here that is a pipeline rather than a draw.
4. **Locked is a word, not a glyph.** One padlock silhouette from
   game-icons over a dimmed picture reads at a glance; `LOCKED` at 8 px has
   to be read. Cheap, and it keeps §1.3's rule — locked and unaffordable
   must not look alike.
5. **Craft-done is a line in the feed.** Ours says `crafted 1 × Wooden Spear`
   in the toast queue; theirs is a `note.inv` beside the vitals and a
   sound. `feed.crafted()` is already the single drain (`CLAUDE.md`'s
   one-drain rule) and `sound/mod.rs::Cue::CraftDone` already chimes, so
   the whole change is where the HUD draws the line.
6. **CRAFT's affordability is a dim, not a colour.** The one thing a
   community plugin exists to add. A green CRAFT is a palette knob —
   `DECISIONS.md` §open, ui palette v1 — not a code call.
7. **The rail is buckets, not classes** — `NOW.md` §0w item 1, one wire
   byte and a `PROTO_VER` bump. Their counts are class totals; ours are
   filtered. Take theirs when the byte lands (it is what was measured).
8. **No description.** `content/items.toml` has no blurb field and
   `ItemCatalog` carries names only. A capped text column per item is
   content (48 rows to write) plus wire (wall 6). Worth it, and not first.
9. **No fast-track.** A verb on the queue (§1.4): sim + wire. Small, and
   it changes what a cancel index means — take §1.1's task id with it.
10. **No time rebate at a bench** — `NOW.md` §0tt, unchanged by this file
    except that the ladder's numbers are now fetched rather than summarised.
11. **No quick-craft column** — `MENUS.md`'s PARTIAL on the inventory.
    Devblog 187 says: default icon only, or it freezes under a loot.
12. **The `+1` badge.** We do not show `out_count`; one label.

### 9.2 · What not to take

- **SKINS.** That is the store (`BUSINESS.md`), A2/A3, and not the panel's
  business.
- **COMMON.** Undefined here (§7); a bucket nobody can state is a bucket
  nobody can gate.
- **Cooldowns, quality tiers, economy purchase** — plugin inventions.
- **A CUI-style rebuild.** Craft UI recreates the vanilla panel to change
  three things; ours is already ours.

### 9.3 · The order

Draw-only first (1, 2, 4, 6, 12), then the picture pipeline (3), then the
content-and-wire pair (7, 8) in one `PROTO_VER` turn, then the two sim verbs
(9, 10). Nothing in §9 invents a number; the one colour is a §open row.
