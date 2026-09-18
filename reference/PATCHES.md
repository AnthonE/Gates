# reference/PATCHES.md — how the reference game ships a month of work

Research, not law. `CLAUDE.md`'s table says what that means: nothing here owns
anything, and no number in it may reach `content/`. This one is unusual among
its neighbours in what it is about — every other `reference/*.md` describes a
**mechanic**, and this describes a **process**: the cadence, the two documents
a patch produces, and who signs them. Our answer to it does not exist yet;
`HAVE.md` §7 is the raw material and §9 below is the proposal.

## §0 · Provenance

**Tier 1 throughout, fetched whole on 2026-09-17**, and unusually clean even
for this directory: the process is public by construction — they publish it —
so nothing here is decompiled, summarised or remembered.

| tier | source | how |
|---|---|---|
| 1 | `rust.facepunch.com/news` pages 1–3 | fetched whole — 28 posts, every date and category tag |
| 1 | `rust.facepunch.com/news/breach-and-clear` | fetched whole — one devblog end to end, 641 lines, 19 bylined sections |
| 1 | `rust.facepunch.com/changes` | fetched whole — 5 consecutive patches, every bucket, every line counted |
| 1 | `commits.facepunch.com` | fetched whole — the live public commit feed and its counters |
| 1 | `wiki.facepunch.com/rust/server-wipe-timer` | fetched whole — the wipe schedule as convar defaults, verbatim |

`SOURCES.md` §0's instruction worked again: both hosts answered from this box,
as they did for `FORESTS.md`, `ROCKS.md`, `ROADS.md` and `MAP.md`. **Probe,
never trust either reachability claim.**

**What the sources do NOT contain**, said rather than guessed: how a post is
assigned or edited, who decides what makes the blog, how long before the
Thursday the content freezes, whether there is a staging branch policy in
writing, and what the vote widget is used for internally. §7 infers *one*
thing from vote counts and marks it as inference.

## §1 · The cadence is a named weekday, held for years

Not "roughly monthly". Measured across 19 months of the news index:

```
03 Sep 2026 · 06 Aug 2026 · 02 Jul 2026 · 04 Jun 2026 · 07 May 2026
02 Apr 2026 · 05 Mar 2026 · 05 Feb 2026 · 01 Jan 2026 · 06 Nov 2025
02 Oct 2025 · 04 Sep 2025 · 07 Aug 2025 · 03 Jul 2025 · 05 Jun 2025
01 May 2025 · 03 Apr 2025
```

Seventeen of nineteen months land on the **first Thursday**. The exceptions
are additive rather than slips — an extra mid-month devblog (23 Oct 2025,
10 Apr 2025, 18 Dec 2025), one Wednesday (03 Dec 2025) — and **every post in
28 but one is a Thursday**, community posts included.

The wipe is the same instant, and the wiki states it as the shipped default
rather than as folklore: *"Monthly: First Thursday every month at 19:00
(London time)"*, with `weekly` and `biweekly` as the two other server tags and
four convars to override.

**So the patch, the blog post and the wipe are one event.** That is the whole
trick and it is worth saying plainly: a player does not need to know a version
number, a release schedule or a changelog URL. They know *Thursday*. The
content, the reset and the announcement arrive together, which means the
announcement is read — by an audience that is logging in anyway.

## §2 · Three publications, three cadences, and the raw one is continuous

| tier | what | cadence | audience |
|---|---|---|---|
| **commits** | `commits.facepunch.com` — every commit, unedited | continuous, live | the invested few |
| **devblog** | `/news/<slug>` — curated, bylined, reasoned | monthly | everyone |
| **changelist** | `/changes` — four buckets of one-liners | per patch | returning players |

The commit feed is the surprise, and it is not a token gesture: **156,316
commits over 4,536 days — 1.44 per hour**, each row carrying the author's
handle, the branch, the changelist number and the message, updating with
commits fifteen minutes old. Anyone can watch the game being made, in
real time, including the merges from a DLC branch that has not been announced.

The three tiers do different jobs and the middle one is the only one that
costs writing. The raw tier is free — it already exists as the work — and its
value is *proof of life between the monthly posts*.

## §3 · The anatomy of a devblog post

One post, read end to end (`breach-and-clear`, 03 Sep 2026):

1. **A video at the top**, then **one paragraph** that names the headline
   features in a single sentence — *"This month's update brings you Monument
   Blockers, Breakable Attack Heli Armour, HQM Nodes, Upkeep Group Scaling,
   various item buffs, and QoL and balance changes, as well as many
   performance updates and much more!"*
2. **Nineteen sections, each with a byline** — Martyn Chapman, Jarryd Campi,
   Lukester, elhog, Ian Henderson, Adam W, Alfie B, Lewis T, matt.i, Pilgrim,
   Maverick, cipeaX, ElliotH, Grigler, Daniel P, Alex Webster. A person's name
   on each change. Some sections carry none, which reads as "nobody's in
   particular" rather than as an omission.
3. **A thumbs-up / thumbs-down widget per section.** Not per post — per
   *change*. §7 is what that produces.
4. **The reasoning, not just the change.** The upkeep tax section spends four
   paragraphs on why large groups are a balance problem, states the tier
   table exactly (first 4 players free, next 6 at 2 %/player, then 4 %/player,
   capped at 300 %), says what they expect, and ends *"we'll monitor this over
   time and make adjustments as necessary."* The monument blockers section
   says outright *"We don't think this will have an enormous impact on
   progression speed."* They publish the hypothesis and the uncertainty.
5. **Exact numbers in tables** where a number is the change — the heli armour
   panels are listed as `80% Left panel - 680hp`, four rows, no prose.
6. **A "Notable Changes" section** that sweeps the small balance moves: a
   three-word heading and one sentence each, ten of them.
7. **The server owner's escape hatch is named** — *"Server owners will find a
   set of ConVars in the Decay class that can be used to modify these
   behaviours, including disabling it entirely."*
8. **The full changelist inlined at the bottom**, identical to `/changes`.

A recurring piece of honesty worth copying: *"If this felt like dejavu,
you're right. We accidentally left it in the last dev blog. It's in-game this
time, we promise."*

## §4 · The changelist is four buckets, in a fixed order

`add_circle` **Features** → `arrow_circle_up` **Improvements** → `handyman`
**Fixed** → `remove_circle` **Removed**. Every patch, the same order, the same
four. Removed is omitted when empty. Each patch carries a **Patch Name** (the
devblog's title — *Breach and Clear*) and a separate **Changelist Title**
(*September Update Changelist*), so the marketing name and the filing name are
allowed to differ.

## §5 · A patch is 8 % features and 64 % fixes, measured

Five consecutive monthly patches, counted line by line off `/changes`:

| patch | date | Features | Improvements | Fixed | Removed | total |
|---|---|---:|---:|---:|---:|---:|
| Breach and Clear | 03 Sep 2026 | 8 | 52 | 74 | 1 | **135** |
| Power Trip | 06 Aug 2026 | 12 | 34 | 43 | 2 | **91** |
| Common Ground | 02 Jul 2026 | 6 | 23 | 91 | 2 | **122** |
| Built Different | 04 Jun 2026 | 10 | 23 | 50 | 0 | **83** |
| Upgrade hard, raid harder | 07 May 2026 | 7 | 22 | 71 | 2 | **102** |

Mean **107 lines a month**: 8.6 features (**8 %**), 30.8 improvements (29 %),
65.8 fixes (61 %), 1.4 removals.

**This is the finding to carry.** A month of shipped work on a thirteen-year-
old game is under a dozen new things and a hundred small ones, and they
publish all hundred. The headline is not what the month *was*; it is what the
month was *named*. Nobody reading the blog concludes the game is in trouble
because seventy-four lines are bug fixes — the volume reads as care.

## §6 · The wipe is an object in the world, not an administrative event

On the lowest level of a monument there is a nuclear warhead wired to a laptop
whose timer, labelled **"REPOPULATION UNIT SURVIVAL TEST"**, counts down to
the next wipe. It is driven by the server's own wipe tag and convars, so it is
correct on any schedule the host picks.

And the last 24 hours before a wipe **change the game**: two endgame events
arm automatically (`eventschedulewipeoffset.event_hours_before_wipe`, `0`
disables). Jets begin flying observation passes over the island — explicitly
*"no direct impact on game play"*, pure atmosphere — and Bradley APCs start
roaming the roads attacking anything they see, at `WorldSize / 1000 * 2` of
them, while scientist counts rise.

So the reset is diegetic at both ends: you can *see* how long the world has
left, and the world gets visibly more dangerous as it runs out. A wipe is
framed as an ending rather than as maintenance.

## §7 · The engineering posts score highest — an inference, marked as one

The per-section vote counts on the September post, sorted:

```
563 ↑ /  26 ↓   Upkeep group scaling        (systems design)
511 ↑ /  13 ↓   Medical Honey bandage       (new item)
503 ↑ /  23 ↓   HQM node                    (new resource)
426 ↑ /   7 ↓   Modular car handling        (feel)
220 ↑ /   3 ↓   Network Range Audit         (pure optimisation, no player-visible change)
216 ↑ /   4 ↓   Improved Server Event Saving(persistence correctness)
191 ↑ /   5 ↓   Physics broadphase / BVH    (engine internals)
168 ↑ /   4 ↓   New Navmesh                 (opt-in, server-side)
203 ↑ / 418 ↓   Marketplace Power           (a gating change players disliked)
149 ↑ / 142 ↓   Crystal Assault Rifle DLC   (a paid skin)
```

The absolute counts track how many people care about the subject. **The
ratios** are where the signal is: the four deepest-engineering sections score
between **42:1 and 55:1**, better than every gameplay feature in the post, and
far better than the two commercial items. The multithreading section is
written *for other Unity developers* — TLS variables, `Object.Ensure
RunningOnMainThread` — and still lands at 164 ↑ / 4 ↓.

**The inference** (and it is one — the sources do not say what votes are for):
an audience that plays a survival game will happily read a technical post
about a 44 s → 22 s occlusion bake, and will reward being told what the
engine work was. What they punish is a change that takes something away.

## §8 · What they do NOT do

- **No patch notes per commit.** The raw feed exists but is never presented as
  release notes; the curation is the product.
- **No version numbers in the player-facing language.** Patches are named
  (*Power Trip*, *Common Ground*), and the identifier a player uses is the
  month.
- **No apology posts.** The one meta-comment found in a full post is the
  cheerful *"we promise"* about a feature announced a month early.
- **No separation of "content patch" from "engine patch".** One Thursday,
  one post, art and netcode and balance in the same list.

---

## §9 · What it means for us

### §9.1 · We already produce tiers 1 and 3 and publish neither

`HAVE.md` §7 measured it: **61 merges in four weeks, rising — 6, 13, 18, 24**,
and `DECISIONS.md` §open holds **244 named slices** (`wounded v0`, `map
legibility v2`, `forest density v1`, `browser audio v0`). That is 1.4 merges a
day against their 1.44 commits an hour, on a team of this size, and it is
*already* the shape of a changelist — each row names the change, the knob, the
files and the date. Nobody outside this repo has ever seen it.

The gap is not that we lack content to announce. **It is that our writing all
points inward**: 3,351 lines of `NOW.md`, 349 knob rows, 26 reference docs.
Every one is addressed to the next agent, none to a player.

### §9.2 · The three tiers, priced for us

| tier | ours | cost to start |
|---|---|---|
| commits | the GitHub repo is already public | **zero** — it exists |
| changelist | `DECISIONS.md` §open rows since the last cut | ~1 h a month, mostly mechanical |
| devblog | a written post with sections and a byline | the real cost; ~a day |

The middle one is nearly free and is the one that makes the other two legible.
A changelist is derivable: every `§open` row already carries a name, a date, a
lane and the files, so the four buckets are a sort, not an authoring job.

### §9.3 · Take the cadence whole; it is not the part to be creative about

`BALANCE.md` §6's standing instruction applies to process as much as to
numbers: **take theirs, a case is needed only to differ.** A named weekday
held for years is worth more than a better-reasoned schedule that slips, and
the reason is in §1 — the patch, the announcement and the wipe being one
event is what makes the announcement get read.

What we cannot copy yet is the wipe half, because **no wipe machinery exists**
(`HAVE.md` §6). That is the dependency, not a reason to wait: the blog can
start before the wipe does, and the wipe joins the same Thursday when it lands.

### §9.4 · The 8 % rule is permission

§5 is the most useful thing in this document for a tree in our state. Their
monthly patch is **8.6 features and 96 small things**, published in full. Ours
is overwhelmingly small things — a normal map decoded straight, a cascade that
reaches past 12 m, a ring handoff that stops drawing a hull over every tree —
and the instinct here has been to treat those as not worth announcing, which
is exactly backwards. §7 says the deep-engineering sections score best.

**We have an unusual amount of the thing that scores best.** Determinism,
native/wasm bit-parity, zero-allocation ticks, a counting allocator, a role
gate that catches swapped event payloads, an 8 m far mesh casting false
shadows found by measurement — that is a year of devblog material sitting in
`findings/` and in commit bodies, already written, already honest.

### §9.5 · What to copy exactly, and what not to

**Copy**: the fixed weekday; the four buckets in their order; the byline per
section; the one-paragraph summary naming the headline items; publishing the
reasoning and the uncertainty (*"we'll monitor this and adjust"*); exact
numbers in tables where the number is the change; the small-changes sweep
section; naming the server-owner escape hatch; the full changelist at the
bottom of the post.

**Do not copy yet**: the per-section vote widget (it needs a site with
accounts before it means anything, and a vote count of 3 reads as failure
where 3 ↑ / 0 ↓ on a 20-reader post is fine); the patch *name* as the primary
identifier (we have a release version and a `PROTO_VER` that already do work
those names would confuse — `CLAUDE.md`'s three-version warning); a video at
the top before anyone has booted the game on a GPU.

**Decide, do not inherit**: whether our devblog is monthly or fortnightly.
Their month is a month because a wipe cycle is a month. Ours has no wipe yet,
and at 61 merges in four weeks a monthly post would be ~100 changelist lines —
which is, almost exactly, their number.

### §9.6 · The first one is not a process, it is a post

The failure mode this repo knows best is building the machine instead of the
thing (`CLAUDE.md`: a pixel gate that passed 36 checks on a beige smear).
**Do not build a changelog generator, a devblog template system or a `ci/`
gate for post structure.** Write one post about what is already in the tree —
`HAVE.md` is its outline and most of its body — publish it, and find out what
it costs before deciding what the cadence is.

**§9.7 · The wipe as an object.** §6 is free design advice we should bank now,
because it collides with `WORLD.md` and the collision is favourable.
`WORLD.md`'s whole fiction is that Gates is a threshold dimension and that the
wipe cadence *is* the fiction — banked JUNK leaves, carried JUNK does not. A
countdown a player can walk up to and read, plus a last-24-hours in which the
world visibly turns hostile, is the reference's version of exactly that idea,
and it is the cheapest possible proof that a wipe is an ending rather than
maintenance. `WORLD.md` §9.1's advice stands — decide the register early,
build it late — but the *timer* is a deployable with a number on it, and that
is buildable long before the register is settled.
