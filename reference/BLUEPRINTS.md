# reference/BLUEPRINTS.md — which blueprints the reference game makes you learn

Research, not law. `CLAUDE.md`'s table says what that means: nothing here
owns anything by itself. The operator adopted its split on 2026-09-22
(*"It needs to be just like Rust survival game"*, answered "Rust's split"),
so §9 is what was built off it: `content/recipes.toml`'s `blueprint` flags,
`content/research.toml`'s rows, prices and edges, and the tier rule in
`crates/content/src/validate.rs`.

## §0 · Provenance

**Strong, and fetched whole on 2026-09-22** — every page below answered from
this box (`SOURCES.md` §0: probe, never trust either reachability claim).

| tier | source | how |
|---|---|---|
| 1 | Pivot or Die devblog (2025-11-06) | fetched whole — the price ladder: *"drastically reducing tech tree and research costs: Common: 15 (down from 20) / Uncommon: 30 (down from 75) / Rare: 60 (down from 125) / Very Rare: 120 (down from 500)"*, and *"Removed tech tree tax"* |
| 1 | Tech Tree Update devblog (2020-12-03) | fetched — *"Each level of workbench has its own tech tree composed of all of the items within its tech level… unlock them in a linear fashion"* |
| 2 | `wiki.facepunch.com/rust/item/<shortname>`, **42 pages** | fetched whole — default/learned, research-table price, craft bench. Every default page carries *"This blueprint is known by default. Research is only needed to obtain a physical blueprint for the Industrial Crafter."* |
| 3 | `rusthelp.com/items/<slug>` (12 learned + 30 default) and the data behind `rusthelp.com/tools/techtree` | fetched whole (needs a browser User-Agent; bare curl gets `400 {}`) — node prices, forced prerequisite chains, path totals |
| 3 | `rustly.com/item/<slug>/` ("verified 2026-09-17", build 2633.288.1) | path totals, cross-checked — every one agrees with rusthelp |

**The wiki's tech-tree row loses** (`BALANCE.md` §6.3, rung 2 then rung 1):
it prints a flat 125 / 250 / 500 per bench level whatever the item — the
root node Salvaged Hammer reads 125 where rusthelp reads 30, the AK 500
against a node of 120 — which contradicts the devblog's own statement that
the tree is priced by the rarity ladder, and would make a revolver's node
dearer after a cut than before it. Node price = research price is what the
devblog says, what rusthelp and rustly print, and what their path totals
add up to. The wiki's research prices, benches and default/learned lines
agree with rusthelp on all 42 items and are used.

**What the sources do not contain**: an item's rarity (no page prints one —
the research price is the only trace of it), and whether a WB2 can open the
WB1 tree (search summaries disagree; `DECISIONS.md` 2026-09-22 settles ours).

## §1 · Most things are known; the tree gates the next tool, not the first

Of our 42 mappable recipes, **12 are learned in the reference and 30 are
known by default**. The default set is wider than a newcomer expects: every
bench, the research table, the furnace, the code lock, the sheet-metal door,
the crossbow, gunpowder and HV arrows are all known — some still need a
workbench to *craft*, but none needs a blueprint.

| our recipe | their item | status | research | their bench |
|---|---|---|---|---|
| hatchet_metal | Hatchet | **learned** | 30 | WB1 |
| pickaxe_metal | Pickaxe | **learned** | 30 | WB1 |
| pistol_ammo | Pistol Bullet | **learned** | 30 | WB1 |
| revolver | Revolver | **learned** | 30 | WB1 |
| satchel_charge | Satchel Charge | **learned** | 60 | WB1 |
| window_shutters | Wood Shutters | **learned** | 15 | none (node in the WB1 tree) |
| window_bars_metal | Metal Window Bars | **learned** | 30 | WB1 |
| armor_roadsign_body | Road Sign Jacket | **learned** | 30 | WB2 |
| window_glass | Strengthened Glass Window | **learned** | 30 | WB2 |
| garage_door | Garage Door | **learned** | 30 | WB2 |
| medkit | Large Medkit (and Medical Syringe) | **learned** | 30 | WB2 |
| gunpowder | Gun Powder | default | (120) | WB1 |
| arrow_metal | High Velocity Arrow | default | (15) | WB1 |
| crossbow | Crossbow | default | (30) | WB1 |
| box_large | Large Wood Box | default | (120) | none |
| lock_code, door_metal, research_table, furnace, workbench1–3 | — | default | — | various |
| rock, torch, stone tools, spears, bow, arrows, bandage, bag, boxes, fire, hearth, doors, plan, hammer, fuel, burlap | — | default | — | none |

(A default item's research price is only what it costs to make a physical
blueprint for their Industrial Crafter, which we do not have.) Two of ours
have no counterpart: the **recycler** is a monument fixture there, not a
craftable (both item pages 404; a search summary says deployable recyclers
are plugin-only [S]), and there is **no metal spear** (their stone spear is
an upgrade of the wooden one, and default).

## §2 · The tree: one per bench, each node behind exactly one other

- **Per bench, independent.** WB1's tree starts from three entry nodes
  (Tank Top 15, Salvaged Hammer 30, Water Bucket 15), WB2's from two
  (Hazmat Suit 60, Longsword 30).
- **Forced chains.** Every node has exactly one input; there is no second
  route to anything, and rustly states the rule: *"you cannot buy a node
  without buying everything upstream of it first."*
- **Node price = research price**, so the tree costs more only as the path:
  a revolver is 30 at a research table with one in hand and **420** through
  the tree.
- **Upgrades sit behind their lesser form**: bars behind shutters, the
  garage door behind the glass window, the large medkit behind the syringe,
  the revolver behind the pistol bullet, the satchel behind the beancan.

The chains for our learnable items (node prices; totals match rusthelp and
rustly):

- **Hatchet**: Salvaged Hammer 30 → Hatchet 30 = 60.
- **Pickaxe**: Salvaged Hammer 30 → Engineering Workbench 60 → Pickaxe 30 =
  120. *Not behind the hatchet* — a guess made before this was fetched said
  it was, and it is wrong.
- **Pistol Bullet**: the pickaxe chain → Salvaged Sword 15 → Reinforced
  Wooden Shield 60 → Compound Bow 30 → Mini Crossbow 30 → Mace 30 → Salvaged
  Cleaver 30 → Waterpipe Shotgun 30 → Flare 15 → Pistol Bullet 30 = 390.
- **Revolver**: the pistol-bullet chain → Revolver 30 = 420.
- **Satchel Charge**: the revolver chain → Beancan Grenade 30 → Satchel 60 =
  510.
- **Wood Shutters**: Salvaged Hammer 30 → Hatchet 30 → Wooden Ladder 60 →
  Wooden Barricade 15 → Barbed Wooden Barricade 30 → Wooden Floor Spikes 15
  → Wood Shutters 15 = 195.
- **Metal Window Bars**: the shutters chain → Metal Window Bars 30 = 225.
- **Road Sign Jacket** (WB2): Hazmat Suit 60 → Boots 30 → Coffee Can Helmet
  30 → Road Sign Jacket 30 = 150.
- **Strengthened Glass Window** (WB2): Longsword 30 → Salvaged Axe 60 →
  Salvaged Icepick 60 → Metal horizontal embrasure 15 → Metal vertical
  embrasure 15 → Glass 30 = 210.
- **Garage Door** (WB2): the glass chain → Garage Door 30 = 240.
- **Medical Syringe → Large Medkit** (WB2): Hazmat Suit 60 → Syringe 30 →
  Large Medkit 30 = 120.

## §9 · What it means for us

**§9.1 — the gated set is theirs (built 2026-09-22).** Eleven of our
recipes carry `blueprint = true`: the twelve learned rows above less the
syringe, which we do not ship separately (our medkit — 50 cloth + 10 low
grade at WB1 — takes the large medkit's row and price). Gunpowder and metal
arrows are **known by default** now, as theirs are; the crossbow and the
large box already were. Every research price is the item page's, and the
tree charges the same number (§0's ruling), one shared column as before.

**§9.2 — our bench ladder stays, and it is the one deliberate difference.**
Their revolver, pistol bullet and satchel are WB1 items; ours are WB2, WB2
and WB3 (bench ladder v0, spoken 2026-08-15) because our catalogue has no C4
or rockets and the satchel is the top of our raid ladder — a mechanism
difference, `BALANCE.md` §6.2's only admissible kind. The medkit goes the
other way (theirs WB2, ours WB1). The tree a node sits in follows OUR
station (`research::node_tier`), since the tree is walked at our benches.

**§9.3 — the edge rule, so no edge is invented.** A node's parent is its
**nearest ancestor in their chain that we also ship and that sits in the
same tier of ours**; with none, it is a root hanging off the bench. Applied:

| tier | tree |
|---|---|
| 1 | hatchet_metal → window_shutters → window_bars_metal · pickaxe_metal · medkit |
| 2 | pistol_ammo → revolver · armor_roadsign_body · window_glass → garage_door |
| 3 | satchel_charge |

Their satchel descends from the revolver and their pistol bullet from the
pickaxe; both links cross one of our tiers, so both are roots here — their
trees are independent per bench (§2), and `validate::structural` now refuses
a `requires` that crosses a tier rather than trusting that it cannot happen.

**§9.4 — what is left, and why it is small.** Their chains are long because
their catalogue is wide (a pistol bullet sits eleven nodes deep); ours are
one or two deep because the intermediate nodes — beancans, flares, embrasures,
the engineering workbench — do not exist here. Adding any of those items is a
content decision that should re-run §9.3's rule, not append an edge by hand.

**§9.5 — the table makes paper (built 2026-09-23).** Their research table is
a two-slot container — the item, the scrap — that runs about ten seconds,
refuses to be emptied while it runs, and leaves a blueprint *item* where the
sample was; the blueprint is learned by using it, which is what lets one be
traded, and researching something you already know is allowed because the
paper is the point (every default item page's *"Research is only needed to
obtain a physical blueprint"* is the same fact from the other side). Ours is
that, with the research table a box record, the wait on the ovens' stride,
the price taken when the research lands, and one paper item whose `cond`
carries its target — their single blueprint item with a per-instance target,
which our 64-slot item table would have forced anyway (`DECISIONS.md` §open
"research table v1"). The instant research-from-the-hand verb is gone.
