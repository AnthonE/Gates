# reference/SOURCES.md — what to read, and which question it settles

⚠ **Reachability is a property of the container, not of the hosts. Probe;
never read the state below as current.** This header has now been rewritten
in both directions and both rewrites were honest measurements:

- 2026-08-09 found `rust.facepunch.com/news` and `wiki.facepunch.com`
  **open**, serving full page text, and corrected an earlier blanket "every
  Rust domain 403s" claim that had cost a research pass its confidence. The
  devblog quotations in `RIPLIST.md` §4.3 are primary text from that pass.
- 2026-08-10 (`PROJECTILES.md`, a different container) found the same two
  hosts plus `wiki.rustclash.com` returning **`EGRESS_BLOCKED`** from the
  egress proxy. That pass ran tier 2 and tier 3 on search summaries alone
  and says so in its own §0.
- 2026-08-11 (the `RIPLIST.md` §1e stack-size pass, a remote-session box)
  found the fetch layer blocked for **every** host — `example.com`
  included, so it is the box's egress policy and not a per-domain rule —
  the shell proxy answering 403 to CONNECT, `gist.githubusercontent.com`
  refused. Working: **WebSearch** and **`raw.githubusercontent.com`**
  (probed 200). Tier 3 only from there.
- 2026-08-14 (the survey-intake pass, a remote-session box) found the
  `oezp.at` **PDF galley fetchable**: the `de`-route URL with a third
  path segment — `oezp.at/OEZP/de/article/download/4231/3257/10571` —
  served the whole 406 KB PDF on the first try, and tier-4 item 14
  closed at primary tier off it (`RIPLIST.md` §5.6). The 2026-08-09
  redirect loop was an honest measurement and did not reproduce for that
  one URL; the `en` article pages were not re-probed. WebSearch also
  worked; nothing else was probed.
- 2026-08-14, later the same day (the re-verify pass, same box): the
  fetch layer here is **broadly open** — `rusthelp.com` `/items/<slug>`
  and `/world/<node>` pages serve whole (the §1c/§1e re-verify ran off
  them at page tier), and `rust.facepunch.com/news/pivot-or-die` served
  the primary post. Still closed, re-probed the same hour:
  `wiki.rustclash.com` **403** (bot wall, unchanged since 2026-08-09)
  and `reddit.com` **refused at the tool layer** (tier-4 item 15 stays
  open). `wiki.facepunch.com` answered 404 on a guessed path — not
  probed conclusively. The lesson unchanged: this box ≠ the next box.

- 2026-08-15 (the durability pass, the game box) found
  `wiki.facepunch.com` **open and serving its stat tables** — five
  `/rust/item/<slug>` pages fetched whole on the first try, and
  `reference/DURABILITY.md` §2/§3 are read off the pages rather than off
  summaries. ⚠ **This corrects the map below, not just the state:** the
  table says this host "carries **no** yield tables — the numbers are not
  there to take", and that is false for `/rust/item/<slug>`, which carries
  a **Gather Rates** table (gather damage · bonus · **condition loss**, per
  resource) plus Craft and Repair cost tables. `RIPLIST.md` §4.1a took
  per-tool yields off `rusthelp.com` believing this host had none. Still
  closed, re-probed the same hour: `wiki.rustclash.com` **403** (bot wall,
  unchanged since 2026-08-09). Also stale as a result:
  `content/items.toml`'s header claim that "every page fetch is
  egress-blocked" was true for the box that wrote it and is not true here.

- 2026-08-15, **same day, different box** (the voice pass, a remote
  session) found **every host refused**: `rust.facepunch.com`,
  `wiki.facepunch.com`, `partner.steamgames.com` and
  `support.facepunchstudios.com` all returned `EGRESS_BLOCKED`.
  `wiki.facepunch.com` is the pointed one — the bullet directly above
  fetched five of its pages whole **on this date**, and it is shut here.
  **WebSearch worked**; `reference/VOICE.md` tiers 2–4 are summaries and
  its §0 says so. Two lessons, and the second is the one worth carrying:
  the container is the variable, *and a dated entry in this log is not a
  claim about today* — two entries can carry the same date and disagree,
  because they are measurements of different boxes. Probe.

Neither answer generalises. What does: a doc may record *what a probe
found, when, and on which pass* — it may not record "reachable" as a
standing fact, and a reader may not skip probing because a table below says
OPEN. The map is a starting guess with a date on it.

So this file exists for a human with a browser. Each row says **what to
look for**, not just where — a link with no question attached comes back
as a link with no answer attached. Bring numbers back with their
confidence (EXACT / APPROX / DISPUTED — record both, never average) and
they land in `RIPLIST.md` §4/§5.

Ordered by what it unblocks.

**The measured map** — *as probed on 2026-08-09, and the first two rows did
not hold on 2026-08-10 (see above)*. Probe the row you need:

| host | state | note |
|---|---|---|
| `rust.facepunch.com/news` | **OPEN** | devblogs served verbatim; note 170 is `/news/devblog170`, no hyphen, while 166/186/187 take the hyphen |
| `wiki.facepunch.com` | **OPEN** | ⚠ this note read "carries **no** yield tables — the numbers are not there to take" until 2026-08-15, and it was wrong about `/rust/item/<slug>`: those pages carry **Gather Rates** (damage · bonus · **condition loss**, per resource) plus Craft and Repair cost tables. The prose pages are what carry no numbers |
| `rusthelp.com` | **OPEN** | the per-tool yield tables; `/world/<node>` and `/items/<item>` |
| `corrosionhour.com`, `rustly.com`, `xgamingserver.com` | OPEN | SEO tier — usable only cross-checked, and `rustly` is measurably wrong (§4.1) |
| `wiki.rustclash.com` (ex-`rustlabs.com`) | **403** | bot protection, not policy; `rustlabs.com` 301s here |
| `rust.fandom.com` | **402** | Fandom paywall/bot wall |
| `reddit.com` | **BLOCKED** | refused by the *tool layer*, not the network — no workaround from here |
| `oezp.at` | **PDF OPEN** (2026-08-14, one URL) | the `de` download galley (`…/de/article/download/4231/3257/10571`) serves the PDF whole; the article pages redirect-looped on 2026-08-09 and were not re-probed |
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

14. ✅ **ANSWERED 2026-08-14, at primary tier.** The PDF galley opened
    from this box (§header) and `RIPLIST.md` §5.6 carries Tables 1–3
    whole: 146 players / 73 h; **52% / 48%** non-offensive / offensive as
    the per-player average; **76%** offending at least once; and the
    concentration statistic — 7 hyperactives (5%) at **13.42** offensive
    acts each against a 2.84 mean, while the 20 purely-offensive players
    sit *below* that mean. **The row's framing moved with the reading**:
    this was queued as "the missing magnitude" for the threat term, and
    it is not one — the paper measures the interaction *mix*, not farming
    throughput; no trips, loads, interruptions or deaths/hour are in it.
    What it settles is threat **shape** — heavy-tailed and concentrated,
    `RIPLIST.md` §5.2's trip-shape model said by a measurement. The
    magnitude question moves to the LOGISTICS row (§3b).
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
| ~~**`wiki.facepunch.com` → Ore nodes**~~ | ✅ **confirmed unobtainable 2026-08-14, at the page**: the node pages publish *durations* per tool, never hit counts or per-hit yields — §0's "do not go looking" verdict re-verified on primary text (`rusthelp.com/world/*-node`) | closed — `RIPLIST.md` §2 row 1's per-hit half stays ours by necessity, and our schema does not need it |
| ~~**rustlabs.com** (tool pages)~~ | ✅ **settled 2026-08-14 by the node pages instead**: no published multiplier exists — the ratio manifests as per-tool TOTALS on each node, now read at page tier for all three (stone pickaxe 794/1000 · 485/600 · 257/300 = **0.79–0.86 of best**, nine data points where we had two) | the tool ladder — `RIPLIST.md` §4.2's ~0.8 upgraded from inference to measurement (and widened: sulfur's 0.857 says it is a band, not a constant) |
| **the tech tree, node by node** (`rusthelp.com/items/<slug>` — each item page carries its research cost AND its tech-tree path total; the `tools/techtree` viewer is JS and wants a browser) | Per-node unlock costs along each bench's paths — enough of them to reconstruct the tree's shape — and whether the mixing table's gunpowder recipe is really 33% cheaper in charcoal (single-summary, unconfirmed) | `RIPLIST.md` §2 row 7: the bench rungs are sourced; the per-node walk is what a fetch-capable agent can do from here, page by page |

## 2 · Would settle a disputed number

| source | what to look for | why |
|---|---|---|
| ~~**Any wiki, sulfur node**~~ | ✅ **settled 2026-08-14 at the page** (`rusthelp.com/world/sulfur-node`): **300 with any proper tool** (jackhammer/icepick/pickaxe ×300, stone pickaxe ×257, rock ×100) — the 200 camp is refuted, or was reading a low-tool total | `RIPLIST.md` §4.1's EXACT holds; the per-tool column is §4.2's shape again |
| ~~**Any wiki, tree yields**~~ | ✅ **settled 2026-08-14 at the page** (`rusthelp.com/world/tree`): per-prefab RANGES, not three constants — large 500–1,000 · medium 376–750 · small 250–500 (some 125–250) · saplings 50–200 · swamp 150–300 · driftwood 125–300. Both prior claims were partial truths of this table ("500/750/1000" is the class ceilings; "large ~650" sits inside the large band) | our tree total — a `BALANCE.md` §6 pass may now take a banded species spread instead of one number |
| **Patch notes, 2024–2026** | Whether the 2017-era mechanics above were reworked since. **Swept 2026-08-14, nothing found**: current guides and the node pages describe the same 150→300% hotspot (reset on miss, speed-not-yield — the node pages carry a "Duration (Hotspots)" column and identical yields) in the present tense; the only node changes surfaced are cosmetic (the sparkle appears only after the first hit) and the type-purity rework our content already models. Not closed — a sweep proves absence of evidence, not absence — but the staleness risk is down from "single biggest" to background | everything in §1 |

## 3 · Would settle the threat/logistics decomposition

This is where our evidence is weakest — `RIPLIST.md` §5 rests on a source
cluster caught contradicting itself 3–6×.

| source | what to look for | why |
|---|---|---|
| ~~**Austrian Journal of Political Science**, "…the Hobbesian and Lockean State of Nature in Rust" (`oezp.at`)~~ | ✅ **read 2026-08-14** — `RIPLIST.md` §5.6 | Settled the *shape* (heavy-tailed, concentrated, mostly non-offensive), not the magnitude — §0 item 14 has why the framing moved |
| **r/playrust** — search "sulfur per hour", "how long to T3", "solo vs group" | Player-reported throughput on **vanilla 1×**, and how much of a session is farming vs travel vs fighting | Reddit was unreachable to the research agent by fetch *and* by search; it is the primary community source and we have none of it |
| ~~**PC Gamer, "Pivot or Die" coverage**~~ | ✅ **read 2026-08-14** — and the mechanic itself at **primary** tier from `rust.facepunch.com/news/pivot-or-die` (Nov 2025): monuments emit **unsurvivable radiation around loot/puzzle rooms 10 minutes before refresh**, and the puzzle resets only after **5 accumulated minutes clear of players**; the same post is the BP wipe + research-cost cut (common 20→15, uncommon 75→30, rare 125→60, very rare 500→120, scrap crafting costs removed from workbenches). PC Gamer's angle (summary tier): solos/small groups stall at WB1–2 while clans hold WB3 through monument control | The hard-ceiling case `RIPLIST.md` §5.3 cites, now with the mechanism named — and the primary anchor for §3b's PROGRESSION row |

## 3b · The systems queue (2026-08-14) — their game over time, not another number

An operator-relayed survey (raw text and per-claim tiers:
`reference/dumps/20260814-systems-survey.md`) landed one framing worth
adopting: the micromechanics are well-ripped, and the open value is in
the reference's **systems over time** — how a naked's logistics become a
veteran's, how a solo becomes a clan, how a quiet map is periodically
forced to converge, how chores disappear, how a month-long world stays
worth logging into. Seven candidate docs, in the survey's priority
order. Each row names the questions to bring back; **the pass that
writes a doc re-sources every claim first** — most below are
summary-tier, several postdate 2026-01, and two are already corroborated
by a second independent route (marked).

| doc | the questions | why now |
|---|---|---|
| `LOGISTICS.md` | ONE table: effective collection throughput **naked → established solo → established group** — travel speed, carried value per trip, acquisition/fuel overhead, deposit-trip interval, exposure time per unit gathered, transport-loss probability, effective gather/min. Material: the 2020 modular-vehicle update (roads widened for it), Feb 2026 modular boats with fuel-priced engines, the intermittently-opening Deep Sea | Decides whether §5.1's **10–30× logistics term is a constant or an early-wipe phenomenon** — `RIPLIST.md` §2 row 5 (the largest un-charged term in our economy) needs exactly this before the island is made harder to farm |
| `EVENTS.md` | Per event, across generations (airdrop → patrol heli → cargo ship → oil rig/hackable crates → wipe events → Deep Sea): **cadence · warning time · access investment · visible telegraph · objective dwell time · reward · exit exposure**. Cargo is the clean specimen (historically every 2–4 in-game days, announces itself by circling, boat to access, timed hack, winner defends a moving objective) | These seven columns are the knobs of `WORLD.md`'s extraction windows, and the reference ran that experiment for a decade — the strongest single map onto the register roadmap |
| `PROGRESSION.md` | The 2020 tech tree's rationale (deterministic paths so the unlucky still progress); **Oct 2025 "Meta Shift"** (✓ corroborated: BP fragments, 5 basic → WB2 / 5 advanced → WB3, non-craftable, monument-sourced, stated intent to push players into contested ground and slow clan snowballing); **Nov 2025's reversal** (✓ corroborated: BP wipe + sharply cheaper unlocks, "might become a regular thing"); May 2026 workbench rares (single-summary). Extract: **what fraction of progression is reachable without leaving your base, per stage** | A weeks-between-resets game *is* its progression pacing, and 2025–26 is a published two-sided experiment in exactly our cadence question — the lesson candidate: progression exists partly to manufacture reasons to cross dangerous space |
| `SOCIAL.md` | The July 2, 2026 "Common Ground" clans (✓ corroborated: craftable Clan Table, ≤100 members, roles/permissions/chat/MOTD/logs, clanmates on the map, tables standing in safe zones, researched at WB1). Bring back: what each role can authorize; team size vs clan size; which permissions are per-person vs inherited; map-visibility range; the claimed Hardcore disable (single-summary); how cupboards/locks/turrets/respawns key off membership; what happens on kick while assets stay authorized | It separates **membership → authority → communication → identification → position** — the decomposition `lock.rs` and `claim.rs` will face the day grouping outgrows ad-hoc teams, better read before that day than after |
| `INDUSTRY.md` | The 2023 Industrial update: **which chores mature players stop doing by hand, per wipe stage**; the published throughput bounds (16 containers/conveyor, 32 items of a stack per move, 5 s work interval, 12 stacks per industrial tick — all APPROX, single-summary) | The in-base half of LOGISTICS' decomposition (acquisition → transport → sorting → processing → crafting → redeploy) — late-wipe players contesting instead of sorting boxes is a week-3 retention mechanism |
| `MODERATION.md` | The F7 report payload (reporter/target identity, report type, server/location context, playtime, optional screenshot; console print or an HTTP endpoint), `combatlog <id>`'s fields, and the client-demo vs **full-server-demo** split (disk allocation, upload URL, playback client reportedly unfinished) | Wiki-documented, so plausibly reachable at primary tier. Pairs with `anomaly.rs`: the stack is report → immutable identifiers → **compact adjudication log** → escalate to replay evidence only when needed → action with reason and duration — and a deterministic core makes the expensive rung cheaper for us than it is for them |
| `TRADE.md` (safe zones · shops · raid windows — three mechanisms, kept separate) | Outpost's 2021 drone marketplace (20-scrap delivery, ≤5 min, 10-min terminal reservation, invulnerable drones — exchange de-risked *on purpose*, their stated goal being more trading); July 2026 rentable shops (hourly rent, 12 h minimum stake, escalating takeover, 24 h eviction recovery — existence ✓ corroborated, numbers single-summary); Softcore raid windows 18:00–21:00 server-local + the 1 h cupboard aging (✓ corroborated) | The haven and the bank/extraction seam map onto all three, and the raid window is their shipped answer to "I cannot be online 24/7 on a weeks-long shard" — read it before we invent ours |

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
