# reference/dumps/20260814-techtree-benchladder.md — the bench-ladder survey, raw

**A dump, not a doc** (`SOURCES.md` §"the pipe back in"): the operator
relayed this on 2026-08-14 from a loop assistant answering "do we have
the building pieces / how does scrap research work". Its sources are
community wikis and server-host blogs — summary tier, self-flagged as
conflicting — and a same-day page pass both **vindicated its caution and
dated its table**: the numbers it carries are the **pre-Oct-2025
economy**, and the reference has since moved twice (Meta Shift, Pivot or
Die — both already at primary/corroborated tier in `SOURCES.md` §3b).

Cells corrected at the page, 2026-08-14 (`rusthelp.com/items/*`):

| its claim | current, at the page | which update moved it |
|---|---|---|
| Workbench 2 = "500 scrap (+ mats)" | **500 frags + 20 HQM + 5 Basic Blueprint Fragments**, hp 500 | Meta Shift (Oct 2025) took scrap OUT of benches, put fragments in |
| Workbench 3 = "1,250 scrap (+ mats)" | **1,000 frags + 100 HQM + 5 Advanced Blueprint Fragments**, hp 750 | same |
| "Bench ladder alone ~1,800 scrap" | true of the pre-Oct-2025 game only | same |
| Explosives "500 scrap to research" | **120 scrap** (very rare, post-cut) | Pivot or Die (Nov 2025) cut 500→120 |
| Mixing Table "75 scrap" | **30 scrap** (uncommon, post-cut); craft 200 frags + 100 stone | Pivot or Die cut 75→30 |
| "Blueprint by tier T1 75 · T2 125 · T3 250 · 500+" | post-cut table is **15/30/60/120** (primary, the Pivot or Die post itself) | same |

Confirmed at the page: explosives' recipe (50 gunpowder + 3 LGF + 10
sulfur + 10 frags, WB3) and the two-system model — item pages carry BOTH
a research-table cost and a **tech-tree path total** (explosives 600
scrap, mixing table 90), so the whole tree is walkable page by page
without the JS viewer. Unconfirmed, single-summary: the mixing table's
"gunpowder 33% cheaper in charcoal".

Its tree-claims all verify: six square shapes and 5b4915f's four ratios
(`NOW.md` §0ac already queues the missing window/frame/triangles),
`research.toml`'s powder-era-only gate, one workbench, OBOL-is-scrap.

What consumed this: `RIPLIST.md` §2 row 7 · `NOW.md` §0tt ·
`DECISIONS.md` 2026-08-14 (the spoken direction) · `SOURCES.md` §1.
The text below is verbatim.

---

Building pieces — no, we never got them

  Six shapes, all square. The Shape enum is exactly Foundation, Wall, Doorway, Floor, Stairs, Roof, and building.toml is those six across four grades = 24 pieces.
  No triangle, no window, no half-wall, no ramp.

  The commit you're thinking of is 5b4915f — "A hundred building prices are four ratios, and one is a refusal." It's a research commit: it added
  reference/BUILDING.md §7b documenting their 20 shapes × 5 grades, and the finding that a hundred prices normalise to four ratios — 1.0 foundation/wall/stairs, 0.7
  an opening, 0.5 a horizontal above ground, 0.25 a triangle floor. That doc "owns nothing" by its own header. So we priced the catalogue we don't have: we already
  know what a triangle floor should cost before a triangle exists.

  How scrap research works in Rust now

  Two parallel systems, and that's the part worth stealing:

  - Research Table — put a looted item on it plus scrap, get its blueprint. Cheap, but only for things you've already found.
  - Tech Tree — at a workbench, unlock along connected paths. Costs more per item, but reaches things you've never seen, in a planned order.

  The intended play is both: research what you loot, tech-tree the gaps.

  ┌────────────────────┬─────────────────────────────────────────────────────────────────────────────────────────┐
  │        gate        │                                          cost                                           │
  ├────────────────────┼─────────────────────────────────────────────────────────────────────────────────────────┤
  │ Blueprint by tier  │ T1 75 scrap · T2 125 · T3 250 · 500+ endgame                                            │
  ├────────────────────┼─────────────────────────────────────────────────────────────────────────────────────────┤
  │ Workbench 1        │ 500 wood + 100 metal frags                                                              │
  ├────────────────────┼─────────────────────────────────────────────────────────────────────────────────────────┤
  │ Workbench 2        │ 500 scrap (+ mats)                                                                      │
  ├────────────────────┼─────────────────────────────────────────────────────────────────────────────────────────┤
  │ Workbench 3        │ 1,250 scrap (+ mats)                                                                    │
  ├────────────────────┼─────────────────────────────────────────────────────────────────────────────────────────┤
  │ Bench ladder alone │ ~1,800 scrap                                                                            │
  ├────────────────────┼─────────────────────────────────────────────────────────────────────────────────────────┤
  │ Mixing Table       │ 75 scrap — makes gunpowder 33% cheaper in charcoal                                      │
  ├────────────────────┼─────────────────────────────────────────────────────────────────────────────────────────┤
  │ Explosives         │ 500 scrap to research, T3 bench, then 50 gunpowder + 3 LGF + 10 sulfur + 10 metal frags │
  └────────────────────┴─────────────────────────────────────────────────────────────────────────────────────────┘

  We're closer than you'd think, with one big hole

  We already have the research table: content/research.toml + sim-core/research.rs. And the currency question is already settled — OBOL is scrap (operator,
  2026-08-10, "the carried, item-stack kind"). Only the powder era is gated, deliberately: the file's header says gating the hatchet you need to gather the wood to
  build the bench to research the hatchet is "the kind of circle a tech tree gets wrong once and never lives down."

  The hole is the bench ladder. We have workbench1 and nothing above it. In Rust that ladder is ~1,800 scrap and it's the bulk of the progression you're asking for
  — gunpowder isn't gated by its own cost so much as by the three benches and the T3 gate standing in front of explosives. We have one bench, one research cost, and
  no tech tree. That's why our satchel is 42 minutes of gathering instead of a multi-hour arc.

  Do not paste those numbers into content/ yet. They're community wikis and server-host blogs — secondary sources, and they already conflict (one gives T1 as 75,
  another as 20–75). RIPLIST.md §1e's caveat covers exactly this, and §0's threat frame is the sharper warning: their costs are priced for contested farming, and
  ours isn't contested yet. A 500-scrap explosive gate without their raid pressure is just tedium. This is a RIPLIST row to source properly, not a number to take.

  Happy to draft that RIPLIST row and a bench-ladder NOW item with the open questions marked, if you want it as raw material for your list.

  Sources: Rust Tech Tree 2026 — rustlite (https://www.rustlite.com/tools/tech-tree) · Tech Tree & Workbench Guide — XGamingServer
  (https://xgamingserver.com/blog/rust-tech-tree-workbench-guide/) · Scrap Calculator — XGamingServer (https://xgamingserver.com/tools/rust/scrap-calculator) ·
  Research Table Guide — Corrosion Hour (https://www.corrosionhour.com/rust-research-table/) · How to Get Blueprints — Corrosion Hour
  (https://www.corrosionhour.com/how-to-get-blueprints-in-rust/) · Mixing Tables — TheGamer (https://www.thegamer.com/rust-mixing-table-guide/) · Explosives — Rust
  Wiki (https://rust.fandom.com/wiki/Explosives)
