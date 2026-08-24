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

## 3 · What is still open, in the order it bites

- **The overflow path is self-amplifying.** A refused ring push calls
  `ev_resync`, and recovery re-drips up to 13 message sites per tick into
  the ring that just refused. Convergence is ≈ 64 ticks ≈ 2.1 s per client,
  and a Ring-A drop (`EventQueue::dropped`, cap `MAX_EVENTS_PER_TICK` 256)
  resyncs **every connected client at once** — 100 clients × ~13 msgs × 64
  ticks ≈ 83 k extra messages from one 257th event.
- **`EventQueue::dropped` reaches no `ShardStats` field.** The trigger for
  a shard-wide resync is invisible to `/status.json`. One counter closes it.
- **`ev_resyncs` conflates two causes** — a refused push and a dropped sim
  event. Nothing can tell them apart, so nothing can be concluded from it.
- **Two more arms have identical shape**: `EV_SHOT` (`a` = shooter) is a
  drop-in for `body_event_visible`; `EV_IMPACT` is position-addressed and
  wants `interest::d2_cm` against the anchor instead.
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
