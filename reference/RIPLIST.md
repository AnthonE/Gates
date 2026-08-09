# reference/RIPLIST.md — the numbers we take, the ones we can't, and why

The queue for *"rip the reference's hard numbers now; tune them when we
have players"* (operator, 2026-08-09 — `DECISIONS.md` Spoken). One row per
number, so a pass can pick a row rather than re-derive the whole economy.

**This owns nothing.** `reference/BALANCE.md` §6 is the standing
instruction and `CONTENT.md` §4's bands still decide whether a number may
land. What this adds is the *worklist*: what is taken, what is outstanding,
what is blocked on research nobody has done, and what has no equivalent to
take. When a row lands, strike it here and cite the number at its own
`content/*.toml` row — the citation at the row is §6's requirement and this
file is not a substitute for it.

---

## 0 · The two rules that decide every row

**§6, unchanged:** a number with a reference equivalent and no reason of
ours to differ takes theirs and cites it at the row. When we differ, the
row says why.

**The threat frame** (operator, 2026-08-09: *"rust is hardcore PvP, your
NEVER solo farming and people and animals are randomly killing you"*).
Every number in that game is priced for a world where farming is
contested, interrupted, and frequently lost. Ours is not that world yet —
nothing fights back (`NOW.md` §0m item 2), no shard has held a hostile
population, and `tests/farmwalk.rs` measures an *unthreatened solo*
walker precisely because that is the only farmer this island can produce.

So the §4.1 trap has a second face. That section warns against copying a
*value* onto a different *mechanism* — their per-material raid costs onto
our single `structure` column. The threat frame is the same error one
level out: **their generous yields are balanced by interruption, and we
would be taking the yield without the interruption.** A tree that pays
~460 wood is not generous in a game where you are shot off it; it is
generous here. Two consequences a pass must hold:

- Taking a yield without the threat makes our early game *faster* than
  theirs, not equal to it. That may be fine for an alpha with no
  population — but it is a decision, and it goes in `DECISIONS.md`.
- The threat is therefore not a separate nice-to-have from the number
  work. Until something can kill a farmer, no measurement of ours means
  what theirs means. **Mob→player damage is the highest-value unblocking
  item on this list** even though it is not a number (§0m item 2).

---

## 1 · Taken (2026-08-08, cited at their rows)

Struck rows live in git; this is the standing list so a pass does not
re-litigate them. `BALANCE.md` §2 (matched before anyone tried) and §3
(moved on the balance pass) carry the sources.

| ours | theirs | where |
|---|---|---|
| player hp 100 | 100 | matched already |
| wooden door hp 200 | 200 | matched already |
| wood/stone/cloth stack 1000 | 1000 | matched already |
| building blocks 250 / 500 / 1000 | wood / stone / sheet metal | `building.toml` |
| satchel structure 125, body 475 | 4 satchels per stone wall soft side | `weapons.toml` |
| wooden spear 20 · stone tools 25 · metal tools 30 | theirs | `weapons.toml` |
| pig 150 hp, drops 5 raw meat | their boar (sources disagreed 80 vs 150) | `mobs.toml` |
| hunger 500 · hydration 250 | theirs | `balance.toml` |
| cooked meat 50/3 · berries 10/20 · mushrooms 15/5 +3 hp | their feeds-vs-hydrates split | `consumables.toml` |

Two bands moved as arithmetic fallout, both spoken: `wall_breach_swings_min`
150 → 60, and the raid ratio re-pricing to 1.04/1.73/3.46.

---

## 2 · Outstanding — the queue

Ranked by what a returning player notices, which is `BALANCE.md` §5's
order. **Status** is one of: `BLOCKED-RESEARCH` (we do not have their
number), `READY` (we have it and could land it), `NEEDS-MECHANISM` (the
number is meaningless until something else is built).

| # | number | status | what it costs to take |
|---|---|---|---|
| 1 | **gather yields / node totals** | `BLOCKED-RESEARCH` for 3 of 4 nodes | Their tree is ~460 over ~16 hits (our doc's figure, secondhand). Breaks **both** node bands at once — 16 hits is outside `node_hits` [8,12], and the total is outside `node_yield` [250,400]. Per §7 that is a look-at-the-band moment, not a refusal. Also re-prices `wood_wall_minutes` and every farm-minute anchor, so the bands and the yields move in ONE commit with the re-speak. `farm_per_min`'s ceiling gate (`balance.rs`) now catches the half that used to be silent. |
| 2 | **per-material damage resistance** | `READY` (mechanism build, not a lookup) | The biggest *model* gap, and `BALANCE.md` §4.1 calls it a build: a schema column plus a sim multiply. Their stone wall takes 4 satchels and their sheet metal 23; ours takes 8 because one `structure` column serves every material. Until this exists, their raid numbers above stone cannot be taken at all — the ladder has nowhere to go. |
| 3 | **smelt rates and craft times** | `BLOCKED-RESEARCH` | `BALANCE.md` §4.3: same `farm_per_min` dependency, smaller blast radius. §4.2 already retired the excuse ("no reason was ever given beyond inertia"). Craft seconds are ignored by the anchors by declaration, so this moves *play* without moving the anchors — the cheapest real row here once the numbers exist. |
| 4 | **the animal roster** | `BLOCKED-RESEARCH` + `NEEDS-MECHANISM` | Chicken, stag, wolf, bear all have roles there; we have a pig. Health and drops are lookups. The wolf and bear are `NEEDS-MECHANISM` — they exist to threaten, and nothing can hurt a player yet. |
| 5 | **mob→player damage** | `NEEDS-MECHANISM` | Not a number, and on this list because §0's threat frame makes it the gate on every other row's *meaning*. Costs a new death cause on a 2-bit field saturated since wire v24, so it is a wire widening (wall 6: version bump + regenerated goldens in one commit). |

---

## 3 · No equivalent to take

Naming these stops a future pass hunting for a number that was never
theirs.

- **`[globals] farm_per_min`.** The reference has no declared farm-rate
  currency at all. Their tuning knob is a *multiplier on the yield*
  (server `gather.rate` convars); ours is a derived abstraction sitting
  beside the yield, which is exactly how the two drifted 20–40× apart
  with every gate green. It exists because we gate balance at
  content-load time with no playtest data — it is a substitute for the
  telemetry they have and we do not. Its semantics are the open question
  in `DECISIONS.md` §open.
- **`component_minutes`** (road-minutes for barrel drops). Same class:
  our pricing model, not their number.
- **The band system itself** (`CONTENT.md` §4). Theirs is a decade of
  live iteration. Ours is arithmetic over TOML because nothing is live.
- **Upkeep and decay mechanism, armour ladder, animal
  respawn/population.** `BALANCE.md` §4.1 — different mechanisms on
  purpose, so their values would be false familiarity.

---

## 4 · The research gaps — what blocks §2 rows 1, 3, 4

`BALANCE.md` §0 is honest that every figure in it arrived as a *search
summary* through this box's egress proxy, and it recorded what it could:
node totals for stone (1000) and sulfur (300), and the tree at ~460 over
~16 hits. What it never recorded, and what a rip cannot proceed on:

- **per-hit yields** for stone, metal and sulfur nodes
- **hit counts** for stone, metal and sulfur nodes
- **tool gather multipliers** — the 1.5× metal-over-stone in our own data
  is OURS, derived from our numbers; the doc never records theirs
- **the bonus-marker numbers** — we ship `weak_spot_bonus_pct = 50` on
  four nodes; their tree marker and ore hotspot are the mechanism we
  copied, but the bonus size is not recorded
- **smelt and craft times**, per recipe
- **animal health and drops** beyond the boar

A pass that fills these updates `BALANCE.md` §0's provenance and strikes
the gap here. Confidence labels are mandatory: EXACT / APPROX / DISPUTED
(record both, never average) / UNKNOWN — §0's rule, and the reason our
boar has two remembered values.

---

## 5 · How to execute one row

1. **Read the row's blocker first.** `BLOCKED-RESEARCH` means find the
   number, not guess it. `NEEDS-MECHANISM` means the row is not yours.
2. **Compute what breaks before editing.** `cargo test -p content` is the
   whole balance system; a band break refuses the shard's boot, not just
   the test. The anchors that re-price off any raw-material change:
   `starter_minutes`, `satchel_minutes`, `wood_wall_minutes`,
   `upkeep_daily_minutes`, all three raid ratios.
3. **If a band refuses the number, look at the band** (§7). Ask which of
   the two is stale. Either answer goes in `DECISIONS.md` the same day —
   a band that moves silently is what `CONTENT.md` §4 exists to prevent.
4. **One commit**: the numbers, the bands they force, the fixture updates
   in `crates/content/tests/content.rs`, and the `DECISIONS.md` row. A
   half-landed re-derivation bricks every gate that loads content.
5. **Cite at the row**, in the `.toml`, with the confidence label. §6's
   requirement and the only part of this that survives the file.
6. **Say what the threat frame does to it** (§0) if the number is a
   yield, a pace, or a cost — one line in the commit body is enough.
