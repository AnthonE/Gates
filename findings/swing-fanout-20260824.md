# `EV_SWING`'s fan-out, priced — 2026-08-24

`NOW.md` §0sw closed with *"the throughput half of the fan-out is
unpriced: `EV_SWING` is one broadcast per swing per player with no AOI
filter, so a 100-player shard swinging pays 100× the per-client event
rate."* This is the measurement, the filter that came out of it, and — the
half that matters more — **the part of that sentence that is wrong.**

## 0 · What was actually true

`pump_events` (`server/core.rs`) routes 40 event codes. Counted
2026-08-24: **15 unicast, 21 broadcast with no filter, 1 AOI-filtered
(`EV_PIECE_PLACED`), 1 unrouted (`EV_TRUST`)**. `EV_SWING` was one of the
21 — `for slot in 0..MAX_PLAYERS`, guarded only by `connected`.

The client throws away every copy it cannot draw, and does so silently by
design. `client-core/src/core.rs` stores the id **unvalidated**; both
readers (`render/bodies.rs`, `render/audio.rs`) iterate *bodies* and test
membership of the swing slice, so an id naming no body matches nothing.
`render/audio.rs` says so in its own doc: *"A swinger outside AOI cannot
either. No body, no transform, no sound — which is the honest cull."*

So the cost was pure waste, not a wrong picture.

## 1 · Measured

`server/tests/snapshot_budget.rs`, sparse fixture — 36 connections on a
6 × 6 grid at 320 m pitch, every pair past `AOI_EXIT_CM` (208 m), interest
sets asserted empty before anything is measured:

```
n=36  swings=72  frames_sent=72  frames_skipped=2520  ratio=36.0x
```

One `EV_SWING` frame is **6 B** on the event lane. The ratio is the
connection count exactly, because a swing nobody can see now reaches only
the hand that swung.

Derived, not measured, with its inputs named: `SWING_INTERVAL_TICKS` is 38
and `bots.rs` rolls `BTN_PRIMARY` 1-in-3, so a bot swings ≈ **0.73/s**. A
100-player shard is then ≈ 73 swings/s, and unfiltered that is **73
frames/s and ~438 B/s per client**, almost none of which a dispersed
client can draw.

## 2 · Three things the `NOW.md` sentence got wrong

1. **The filter does not fix the burst it names.** 65 players swinging at
   once are 65 players *in a fight*, and fighters are inside each other's
   176 m by construction. On the clustered fixture the filter is a no-op,
   and `the_filter_buys_nothing_on_a_clustered_shard` pins that rather than
   letting the commit imply otherwise.
2. **The post-filter bound is 64 and `EVENT_RING_CAP` is 64.** The rank
   band caps an interest set at `AOI_RANK_EXIT` = `MAX_SNAPSHOT_ENTITIES` =
   64, which is exactly the ring. Filtering moves the worst case 100 → 64
   and buys **zero** headroom for the other twenty broadcast arms. Anyone
   reading this as "wall 4 closed" is reading it wrong.
3. **Steady state was never 100/tick.** It is ≈ 2.4 swings/tick shard-wide
   (§1). The honest defect was *sustained waste on a dispersed shard*, not
   an imminent storm. Phase-correlated joins are the only thing that stacks
   it.

## 2b · The other two arms, and the objection that had to be checked

Landed the same day. **`EV_SHOT` is the same call** — but only after the
obvious objection was tested rather than waved off: *a swing is local to a
body, a projectile travels*, so a shot could matter to somebody who cannot
see the hand that loosed it. Two independent reasons it does not, and the
first alone settles it:

1. **`render/tracer.rs` already refuses it.** *"A shot from someone outside
   AOI, or one that arrived before their first snapshot. Nothing to hang it
   on, so it is dropped rather than drawn from the origin."* That is the
   same set the filter reads, so nothing that was ever drawn stops being
   drawn. `feed.shots()` has exactly one consumer, so there is no second
   path to check.
2. **The arithmetic agrees.** The longest `range_m` in
   `content/weapons.toml` is **80** against `AOI_ENTER_CM` = 176 m, so a
   shot from outside a client's band cannot put a projectile within 96 m
   of it. This one is a content-vs-limit relationship that a new gun breaks
   in silence, so it is now
   `content.rs::no_weapon_outranges_the_interest_band` — proven red by
   raising the crossbow to 200 m.

**`EV_IMPACT` is different and is the one that changes behaviour.** It
names a place, not a body, so class-D interest cannot answer it;
`point_event_visible` measures the stop point against the class-S anchor
`EV_PIECE_PLACED` already uses. And unlike a swing or a shot, a client
*can* place a decal with no body — `render/decal.rs` will spawn one at any
distance from a **fixed pool that evicts**, so a sub-pixel mark past 208 m
was taking a slot from a mark at the player's feet. That eviction is what
this removes; what it costs is a 0.22 m quad past 208 m.

## 2c · The storm, run — and the ring is the binding cap, not the queue

`NOW.md` §0fan's *"nobody has run an event storm"* is closed.
`snapshot_budget.rs::the_event_lane_holds_at_population` stands 100 bodies
at arm's length on one plane, each firing a **lethal hitscan round every
tick**, and it is the first test in the tree to model the per-connection
ring at its real cap — `EVENT_RING_CAP` lives in `net.rs` as an `rtrb`, so
every other suite hands `tick_bare` a closure returning `true` and can
never see a refusal. The closure's bool *is* the ring's verdict, so
counting pushes per slot and refusing past 64 is the ring itself.

| cap | bounds | peak | headroom |
|---|---|---|---|
| `MAX_EVENTS_PER_TICK` 256 | sim events in one tick | **144** | 1.8× |
| `EVENT_RING_CAP` 64 | pushes to one client in one tick | **50** | **1.28×** |

**The per-connection ring is the binding cap and the sim's own queue is
not close.** That settles which number §open should argue about, with a
measurement instead of a chain of reasoning.

**The amplification is measured too.** Shrinking `MAX_EVENTS_PER_TICK` to
128 so the queue actually overflows: **16 dropped sim events produced 100
client resyncs**, and `ev_resyncs_dropped` accounted for all 100 of them —
so in that scenario none came from a refused push. That ratio is the case
for splitting the counter: the two causes are a whole shard versus one
slow connection, and one number could not tell them apart.

**`ev_interest_skipped` is 0 throughout**, which is the third independent
confirmation of §2's caveat: bodies at arm's length are inside each other's
interest by construction, so the filters buy nothing in a fight.

Three mistakes were made building this fixture and each looked like a
routing bug rather than a fixture bug — worth knowing, because the counts
stayed plausible through all three:

1. **Wire pitch 0 is straight down**, not level (128 is). Every shot went
   into the shooter's own feet, producing exactly one mark each.
2. **`ranged::hitscan` silently refuses a reach it cannot sample** — more
   than `MAX_HITSCAN_SAMPLES` taps at `ARROW_STEP_MM`, i.e. past ~54 m. Its
   comment says `bake_combat` makes that unreachable, which is true of
   shipped content and not of a hand-built fixture. An 80 m gun fired
   nothing at all.
3. **`hitscan` checks `hp == 0` where `draw` does not.** These players
   spawned before any combat content existed, so `player_hp` was 0 and
   every gun refused in silence while bows worked.

## 3 · What is still open, in the order it bites

- **The overflow path is self-amplifying**, and this is now the top item
  because §2c measured how little headroom stands in front of it. A refused
  ring push calls `ev_resync`, and recovery re-drips up to 13 message sites
  per tick into the ring that just refused; convergence is ≈ 64 ticks ≈
  2.1 s per client. Peak observed is 50 of 64. Nothing is broken today and
  nothing says how close it is either — that is what §open has to answer.
- ~~`EventQueue::dropped` reaches no `ShardStats` field~~ /
  ~~`ev_resyncs` conflates two causes~~ — **both closed 2026-08-24**
  (`ev_sim_dropped`, `ev_resyncs_dropped`; the first is watched by name in
  `anomaly.rs`, since a filter firing is normal and this never is).
  ⚠ **Neither fires under shipped limits**, by construction — the queue does
  not overflow at population, which is the good news §2c reports. They were
  proven live by shrinking `MAX_EVENTS_PER_TICK` to 128, not by a scenario.
- **The remaining arms were audited and the rule is RESIDUE, not address.**
  This note previously said `EV_DOOR`, `EV_KNOCK` and `EV_OVEN` were all
  "instants at a place"; only the knock is. `EV_OVEN`'s own doc says
  "Absolute, never a delta", and a door's state is the same. `EV_KNOCK`
  landed filtered — its arm's objection ("a defender asleep on the far
  side of their own base") does not survive the radius, since a base is
  tens of metres inside a 208 m band, and what it was really doing was
  toasting every knock on the island onto every screen with **no owner
  check anywhere** (`hud.rs`).
- **The deploy walk is unaimed, and it is the blocker for the other two.**
  `deploys.entries()` streams whole to every client, where the piece walk
  streams what is within `PIECE_INTEREST_CM` of its anchor and counts the
  rest. So a client 400 m away holds a door's record, and a filtered state
  change would leave it wrong forever. **Documented deferral, not an
  oversight** — `core.rs`: the deployable walk "was left reading upward
  until its own placement seam is proven". Order: aim `EV_DEPLOY_PLACED`
  as `EV_PIECE_PLACED` already is, then the walk, then the two events.
  The band is **3.2%** of the island's area against `MAX_DEPLOYS` 1024;
  `deploy_wire.rs` pins the present truth with three counts that go red —
  deliberately — when that seam is aimed.
- **Nobody has run a swinging soak.** `raid_storm.rs:516` still says
  *"nobody swings"*, and it cannot be the place: its
  `PLAYERS * STEPS_PER_TICK == MAX_COMMANDS_PER_TICK` is a compile-time
  equality with no budget left for a swing.

## 4 · One gate is weaker than it looks, written down rather than papered over

`a_connection_whose_body_is_gone_still_hears_the_shard` gates the
fail-open for an unmeasured interest array. **The state is constructed**,
by removing a body from the world under a live connection. The obvious
route — queue more joins than one tick can land — does *not* reach it, and
this test asserted the opposite until it was run: `MAX_COMMANDS_PER_TICK`
is 256 against a 100-slot world, so every queued `Join` drains in the same
`world.tick` that precedes `update_interest`. What is gated is the routing
decision. What is **not** gated is that any caller produces the state; the
production windows are two-phase eviction (`slots_short`) and a
sleeper-occupied slot table, neither of which this fixture stands up.

Kept anyway, because the mutant that deletes the fail-open passed all
thirteen other gates — the branch is exactly the kind that rots unseen.
