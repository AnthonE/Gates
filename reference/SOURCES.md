# reference/SOURCES.md — what to read, and which question it settles

**Most of it is reachable, and the line above used to say the opposite.**
Re-probed 2026-08-09 (second probe, same day): `rust.facepunch.com/news`
and `wiki.facepunch.com` **serve full page text**, and the devblog
quotations in `RIPLIST.md` §4.3 are now primary-text, not paraphrase. The
earlier "every Rust domain 403s" reading was wrong — whatever produced it
was not an organization policy denial, because the same hosts answer now.
Do not trust a blanket claim of blockage in this file again without
re-probing; that claim cost a whole research pass its confidence.

So this file exists for a human with a browser. Each row says **what to
look for**, not just where — a link with no question attached comes back
as a link with no answer attached. Bring numbers back with their
confidence (EXACT / APPROX / DISPUTED — record both, never average) and
they land in `RIPLIST.md` §4/§5.

Ordered by what it unblocks.

**The measured map** (re-probed 2026-08-09, and it is mixed — probe the
row you need rather than assuming either extreme):

| host | state | note |
|---|---|---|
| `rust.facepunch.com/news` | **OPEN** | devblogs served verbatim; note 170 is `/news/devblog170`, no hyphen, while 166/186/187 take the hyphen |
| `wiki.facepunch.com` | **OPEN** | serves prose, but carries **no** yield tables — the numbers are not there to take |
| `rusthelp.com` | **OPEN** | the per-tool yield tables; `/world/<node>` and `/items/<item>` |
| `corrosionhour.com`, `rustly.com`, `xgamingserver.com` | OPEN | SEO tier — usable only cross-checked, and `rustly` is measurably wrong (§4.1) |
| `wiki.rustclash.com` (ex-`rustlabs.com`) | **403** | bot protection, not policy; `rustlabs.com` 301s here |
| `rust.fandom.com` | **402** | Fandom paywall/bot wall |
| `reddit.com` | **BLOCKED** | refused by the *tool layer*, not the network — no workaround from here |
| `oezp.at` | **REDIRECT LOOP** | every article URL exceeds 10 redirects; abstract reachable by search only |
| `raw.githubusercontent.com` | OPEN | as before |

Unchanged rails: blocked hosts must not be pulled through a fetch proxy
or cache mirror, and datamined dumps breach the nothing-decompiled rail.
**WebSearch works** and is still the only route to Reddit-shaped and
paywalled material — summaries, never page text.

**The pipe back in**: paste raw text into a file on the working branch,
or into chat. `raw.githubusercontent.com` is readable, so a file pushed
to this repo can be parsed directly. Raw dumps are fine — no need to
tidy them; the parsing is the cheap half.

---

## 0 · The dump list — exact questions, in priority order

Answer inline next to each; a number without its unit or tool is worth
little. Mark anything a source hedges on ("about 20%") as hedged.

### Tiers 1–3 — ✅ ANSWERED 2026-08-09, do not re-run

All of it landed in `RIPLIST.md` §4.1–§4.6 from this box, once the hosts
above turned out to be open. Summary of what closed:

- **Tier 1** — per-tool totals for all three ore nodes and the tree
  (§4.1a), species/size bands (§4.1b), and **sulfur settled at 300**, by
  scoring the two candidate pages on cells we could already check.
- **Tier 2** — Devblogs 170, 166 and **186** read verbatim (§4.3). Three
  corrections fell out: the 20% is *hedged* in the original, the tree
  split is **186 not 187**, and the finish bonus **requires a proper
  tool**. The staleness question resolved the other way — the 2026 wiki
  still describes both minigames in the present tense.
- **Tier 3** — smelt (**parallel**, contradiction settled), craft-time
  rebates, the animal roster, and the upkeep ramp (§4.6).

**One Tier-1 item is now known to be unobtainable rather than unfound:**
modern hit counts do not exist publicly, because the marker made
hits-to-clear a function of aim. Sources publish *times*. Do not go
looking.

### Tier 4 — the threat/logistics decomposition, our weakest evidence

14. **STILL OPEN, and now fully identified.** Jan Byczkowski, *"The
    Potential for Survival Games as a Research Medium in Political
    Science: Investigating the Hobbesian and Lockean State of Nature in
    Rust"*, Austrian Journal of Political Science **54(2), 2025** —
    `oezp.at/OEZP/en/article/view/4231`, PDF galley
    `oezp.at/OEZP/en/article/download/4231/3257`. **Not blocked: the OJS
    instance redirect-loops (>10 hops) on every article URL from here,
    and `academia.edu` 403s.** A browser will very likely just open it.
    Bring back: encounter count, sample size, and the violent/non-violent
    and offensive/defensive percentages. The abstract is confirmed
    (players favour non-violent and *defensive* violence); it is the
    magnitudes we lack. Highest-value single item on this page.
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
