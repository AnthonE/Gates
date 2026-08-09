# reference/SOURCES.md — what to read, and which question it settles

**This box cannot reach any of it.** Every Rust domain answers `403` at
the egress gateway — `rust.facepunch.com`, `wiki.facepunch.com`,
`rust.fandom.com`, `reddit.com`, `rustlabs.com`, `umod.org` — which is an
organization policy denial, not a transient failure, and the proxy's own
README says to report it rather than route around it. GitHub raw is
reachable, and that is the entire list of what is.

So this file exists for a human with a browser. Each row says **what to
look for**, not just where — a link with no question attached comes back
as a link with no answer attached. Bring numbers back with their
confidence (EXACT / APPROX / DISPUTED — record both, never average) and
they land in `RIPLIST.md` §4/§5.

Ordered by what it unblocks.

**What is actually reachable from here** (probed 2026-08-09):
`api.github.com` and `raw.githubusercontent.com` — and the API is bound
to this session's own repos, so it cannot even be searched. Everything
else in this file answers 403. Blocked domains must not be pulled
through a third-party fetch proxy or a cache mirror: that is routing
around the denial, and datamined dumps would breach our own
nothing-decompiled rail besides. **WebSearch still works** — summaries,
never page text — which is how every figure in `RIPLIST.md` §4/§5
arrived.

**The pipe back in**: paste raw text into a file on the working branch,
or into chat. `raw.githubusercontent.com` is readable, so a file pushed
to this repo can be parsed directly. Raw dumps are fine — no need to
tidy them; the parsing is the cheap half.

---

## 0 · The dump list — exact questions, in priority order

Answer inline next to each; a number without its unit or tool is worth
little. Mark anything a source hedges on ("about 20%") as hedged.

### Tier 1 — unblocks the biggest queued row (`RIPLIST.md` §2 row 1)

For **each** of stone node, metal node, sulfur node, tree:

1. total yield per node, vanilla 1×
2. hits/swings to clear it
3. yield per hit, if stated
4. all of the above **per tool** — rock, stone pickaxe/hatchet, metal
   pickaxe/hatchet, salvaged icepick/axe, jackhammer/chainsaw
5. does it vary by tree species / node size / biome, and by how much

Where: `wiki.facepunch.com/rust/` (Ore_nodes and per-tool pages),
`rustlabs.com` tool pages, `rust.fandom.com/wiki/Nodes`.
**Settle the sulfur split while there: 300 or 200?**

### Tier 2 — confirms mechanics we shipped on 2026-08-09

6. **Devblog 170** — the hotspot's exact percentages (150% base → 300%
   ceiling?), and the *"you will not actually earn more resources… you
   can harvest the ore faster"* line **verbatim**. Our whole marker
   model rests on a paraphrase of this sentence.
7. **Devblog 166** — the final-strike share: exactly 20%, or "about"?
   And is HQM obtainable *only* from that strike?
8. **Devblog 187** — the tree's half-standing/half-on-the-fall split.
9. **Any patch note after ~2020** touching either minigame. Our only
   hard hit-count table is from 2017 and predates both — this is the
   single biggest staleness risk in the whole research.

Where: `rust.facepunch.com/news` (devblog index).

### Tier 3 — balance surface we have never touched

10. **Smelt times** — seconds per ore in a furnace, wood burned per ore,
    and whether a furnace smelts in parallel or sequentially (two
    sources contradicted each other on this).
11. **Craft times** — seconds per item, and the workbench tier gating.
12. **Animal health and drops** — chicken, boar, stag, wolf, bear.
13. **Upkeep** — the exact %/24 h and the low-stock ramp (we match the
    ~10% but not the ramp).

### Tier 4 — the threat/logistics decomposition, our weakest evidence

14. The **Austrian Journal of Political Science** paper on the
    Hobbesian/Lockean state of nature in Rust (`oezp.at`) — its actual
    encounter percentages. Highest-value single item on this page.
15. **r/playrust**, searched for "sulfur per hour", "how long to T3",
    "solo vs group" — player-reported throughput on **vanilla 1×**, and
    how much of a session is farming vs travel vs fighting. Reddit was
    unreachable to the research agent by fetch *and* by search, so we
    have none of the primary community source.

---

## 1 · Would settle a queued row today

| source | what to look for | unblocks |
|---|---|---|
| **Devblog 166** (`rust.facepunch.com/news/`, Mar 2017) | The ore finishing bonus. Is the final-strike share **exactly** 20% or "about" 20%? Their own text reportedly hedges. Also: is HQM *only* obtainable from that final strike? | `finish_bonus_pct = 20` is shipped on our ore nodes off this; a precise number replaces our reading of a summary |
| **Devblog 170** (2017) | The ore hotspot: 150% base rising to 300%, resetting to zero on a miss. Confirm the ceiling and whether the ramp is per-hit-linear. Confirm the "you will not earn more resources, only faster" line verbatim — **our whole marker model now rests on it** | our `weak_spot_bonus_pct` semantics |
| **Devblog 186 / 187 / 188** (Nov–Dec 2017) | The tree minigame. The metal hatchet's 16→30 per-hit ramp (+2 per mark hit); the **half-on-the-fall** split; whether the mark truly never appears on the first hit | `finish_bonus_pct = 50` on our tree, and the ramp we did *not* copy |
| **`wiki.facepunch.com` → Ore nodes** | Per-node totals and, if listed, per-hit yields and hit counts for **stone, metal, sulfur** — the three we have totals for but no per-hit data | `RIPLIST.md` §2 row 1, the largest queued row |
| **rustlabs.com** (tool pages) | Gather-rate multipliers per tool, if they publish them as stats rather than measurements. Our 0.8 stone-vs-best ratio is inferred from two data points | the tool ladder |

## 2 · Would settle a disputed number

| source | what to look for | why |
|---|---|---|
| **Any wiki, sulfur node** | Total per node: **300 or 200?** Sources split, and the 200 camp traces to one SEO site claiming a 2026 re-verification | `RIPLIST.md` §4.1 records both; a live check resolves it |
| **Any wiki, tree yields** | Per-species totals. "500 / 750 / 1000 by prefab" is one unattributed claim; "large ~650" is another | our tree total, if row 1 lands |
| **Patch notes, 2024–2026** | Whether any of the 2017-era mechanics above have been reworked since. **Our best hit-count data is from 2017 and predates both minigames** — this is the single biggest staleness risk in the research | everything in §1 |

## 3 · Would settle the threat/logistics decomposition

This is where our evidence is weakest — `RIPLIST.md` §5 rests on a source
cluster caught contradicting itself 3–6×.

| source | what to look for | why |
|---|---|---|
| **Austrian Journal of Political Science**, "The Potential for Survival Games as a Research Medium in Political Science: Investigating the Hobbesian and Lockean State of Nature in Rust" (`oezp.at`) | Its actual encounter percentages — how often players met violence vs avoided it | **The highest-value single fetch on this list.** Real methodology, and its headline finding (players favour defensive over offensive violence) *cuts against* a large threat term |
| **r/playrust** — search "sulfur per hour", "how long to T3", "solo vs group" | Player-reported throughput on **vanilla 1×**, and how much of a session is farming vs travel vs fighting | Reddit was unreachable to the research agent by fetch *and* by search; it is the primary community source and we have none of it |
| **PC Gamer, "Pivot or Die" coverage** | The solo-vs-clan progression divergence, and the radiation-timer mechanic gating monument access | The clearest documented case of threat acting as a hard progression ceiling rather than a rate penalty |

## 4 · Reachable from here, and already used

`raw.githubusercontent.com` works, which is how the only three verified
figures in `RIPLIST.md` arrived:

- `Calytic/oxideplugins` → `rust/GatherManager.md` — the gather-rate
  plugin's real command syntax. **Vanilla has no gather convar**; this is
  where "2× servers" actually comes from.
- `bitfabrikken/RustCommands` → `README.md` — the `spawn.*` defaults
  (`min_density 0.5`, `player_scale 2`, …).

If a future pass needs more, prefer GitHub-hosted mirrors of wikis and
convar dumps over the wikis themselves — it is the one door that opens.

---

**Sourcing rails, unchanged** (`reference/README.md`, `ART.md` §7):
public sources only, nothing decompiled, no file copied, no proper nouns
and no traced art. What crosses is integers, each cited at its
`content/*.toml` row.
