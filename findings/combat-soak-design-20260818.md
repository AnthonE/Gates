# Combat at population: the gate design, and the price of a broadcast swing

Design note, 2026-08-18, by a read-only agent. Read-only pass; nothing built,
no cargo run. Answers `NOW.md` §0pvp item 6 ("nothing has fought at
population") and §0sw's outstanding half ("`EV_SWING`'s fan-out is unpriced").

⚠ **Independently re-checked before filing:** `raid_storm.rs:121-122`'s
`const _: () = assert!(PLAYERS * STEPS_PER_TICK == MAX_COMMANDS_PER_TICK)` is
verbatim as quoted; `MAX_PLAYERS = 100` (`limits.rs:7`) against
`EVENT_RING_CAP = 64` (`limits.rs:378`) — **the ring really is smaller than the
population**; `MAX_COMMANDS_PER_TICK = 256` (`:12`), `MAX_BACKPACKS = 256`
(`:395`), `MAX_EVENTS_PER_TICK = 256` (`:592`); `bots.rs:53-60` really does
press `BTN_PRIMARY` on `rng.next_bounded(3) == 0`; `KIND_BITS = 4`
(`protocol/src/lib.rs:671`) + `SUB_BITS = 6` (`event.rs:102`) + 32 = 42 bits,
and `tests/golden/v48_event_swing.bin` is **6 bytes** on disk, as predicted.
The rest is the agent's, at its stated provenance.

## 1 · Extend `raid_storm`, or a sibling? — **a sibling.**

`crates/sim-core/tests/raid_storm.rs` cannot carry combat, and the reason is
arithmetic rather than taste.

**The command ceiling is fully spent.** `raid_storm.rs:121-122`:

    const STEPS_PER_TICK: usize = MAX_COMMANDS_PER_TICK / PLAYERS;
    const _: () = assert!(PLAYERS * STEPS_PER_TICK == MAX_COMMANDS_PER_TICK);

64 players x 4 steps = 256 = `MAX_COMMANDS_PER_TICK` (`limits.rs:12`), exactly.
A swing is not a `Command` of its own — it is `BTN_PRIMARY` on a
`Command::Input` (`gather.rs:816`), one per swinging player per tick. Arming
swings therefore costs 64 more commands than the tick will take, and
`MAX_COMMANDS_PER_TICK`'s overflow policy is **defer/ignore** — "the sim itself
takes at most this many (`World::tick` ignores the excess)" (`limits.rs:9-12`).
So the fixture would silently drop a quarter of its raid steps, and every
saturation equality below would move for a reason nobody wrote down. The
alternative — spend one of the four raid steps on an input frame — changes the
detonation cadence, which is what the removal budget's saturation rests on.

**Three assertions are equalities pinned to saturation, and combat moves all
three.**

* `raid_storm.rs:462-465` — `peak_removals == MAX_REMOVALS_PER_TICK`, with a
  comment that says a drop below is *not* a benign regression: "it means the
  detonations stopped landing together and the gate went quiet."
* `raid_storm.rs:448-451` — `peak_charges == MAX_LIVE_CHARGES`.
* `raid_storm.rs:471-474` — `peak_events == MAX_EVENTS_PER_TICK`.

Deaths take bodies off their plots. `raid_storm.rs:319-326` seats each body on
its plot centre because "every verb here is reach-checked", and
`raid_storm.rs:530-535` asserts `peak_charges > 0` *precisely as the check that
the bodies are still standing where the fixture seated them*. A respawn walks
the ring (`world.rs:1939+`), so an armed storm empties every plot within ~40
ticks and the equalities go red for a reason that is not a regression in
anything they were written about.

**And the file states the exclusion as a design decision, not a gap.**
`raid_storm.rs:42-49`: "The throwable's `damage` is **0**: bodies are not in
this storm. A blast that killed the players would end the storm in a few ticks
and the gate would measure a graveyard instead of a cap."
`raid_storm.rs:212-222` implements it (`ThrowDef { damage: 0, ... }`), and
`raid_storm.rs:514-519` is the line this note was commissioned off. That
paragraph is *correct*; retiring it costs the gate its meaning.

### The sibling

`crates/sim-core/tests/melee_storm.rs`, `test_melee_storm` — name is free
(`ls crates/sim-core/tests/` on 2026-08-18: no collision). Same shape as
`raid_storm.rs`: a private `storm()` returning a `Storm` measurement struct with
every field a *number*, and the assertions in separate `#[test]`s so a failure
names the invariant (`raid_storm.rs:271-290` is the pattern). Its header states
the inverse exclusion: **no charges and no building — that is `raid_storm`'s,
and duplicating it would be a second implementation of a raid.**

The two files are then a pair: one drives every *claim* verb at the command
ceiling with no bodies in it, the other drives every *body* verb with no claim
verbs in it, and neither has to explain the other's numbers.

## 2 · What has to be armed

### 2a · Content

* `combat`: `CombatContent::probe_fixture()` (`combat.rs:289-320`)
  **unmodified** — item 0 is 34 body damage / 200 cm reach against
  `player_hp = 100`, so three swings kill, and `tests/combat.rs:136-179`
  already proves the count comes out of the fixture rather than a literal.
  Explicitly **not** `CombatContent::raid_fixture()` (`combat.rs:330-352`):
  that table is body-damage 0 on every row, which is the table for a probe that
  must survive.
* `gather`: `GatherContent::probe_fixture()` — required, because
  `gather::swing`'s cadence gate is what emits `EV_SWING` (`gather.rs:816-832`)
  and what hands the arm on to `combat::strike` (`world.rs:3244-3251`).
* `backpack`: **must be armed**, and this is the one a naive fixture misses.
  `backpack::drop_for` returns `None` on `base_ticks == 0` ("inert content: the
  module is disarmed", `backpack.rs:268-270`), so an unarmed ladder makes every
  death litter-free and the `MAX_BACKPACKS` half of the gate would pass on a
  world where no bag ever stood. Use `BackpackContent::probe_fixture()`
  (`backpack.rs:105`, `base_ticks = 90`) — short enough that the store churns
  inside a counted window instead of waiting on the shipped 5-minute ladder
  (`content/balance.toml:125-131`).
* `deploy`/`build`: leave at `World::new`'s defaults. A melee storm needs no
  pieces; `combat::raid` (`combat.rs:602`) is reached only on `Strike::Missed`
  + `Swing::Free` (`world.rs:3260-3288`) and gets its own opt-in arm in slice 4.

### 2b · Inventory

Every body needs fixture item 0 in the hotbar slot its frame selects.
`combat::held_item` (`combat.rs:440-447`) reads `p.inv[p.frame.sel as usize]`
and returns `NO_ITEM` on `count == 0`, and `tests/combat.rs:226-244` proves an
empty hand and a stack of firewood are the same swing: nothing. So the frame
must carry `sel: 0` and slot 0 must hold `ItemStack { item: 0, count: 1, cond: 0 }`
— `tests/combat.rs`'s `duel_world:56-70` is exactly this and is liftable verbatim.

The inventory has to be **re-granted after every death**: `world::die`
(`world.rs:1894-1936`) hands the whole inventory to the bag, and the respawn
wakes you naked. `raid_storm.rs:349-354`'s `restock` on a `RAID_CYCLE` boundary
is the licensed precedent ("Inventories are topped up every `RAID_CYCLE` ticks
by writing slots directly, the way `blast.rs` stocks its raider"); the melee
storm re-arms slot 0 for any body whose `hp == FIXTURE_HP && inv[0].count == 0`,
which is the tick after a respawn and no other.

### 2c · How two bodies get inside reach and cone, deterministically

Three liftable helpers, all in `crates/sim-core/tests/combat.rs`:

* `place_in_front(w, attacker, victim, yaw, dist)` at `combat.rs:87-92` — puts
  the victim `dist` metres along `yaw_dir(yaw)` from the attacker and re-seats
  via `Body::at(SEED, hv(SEED), ...)`, which is the same `hv()` memoizer
  `raid_storm.rs:81-99` carries.
* `swing_frame(seq, yaw)` at `combat.rs:73-83` — `BTN_PRIMARY`, `move_x: 0,
  move_z: 0`, `sel: 0`. Standing still is load-bearing: a moving body drifts out
  of a 2 m reach inside a cooldown.
* `cool_down(w)` at `combat.rs:103-107` — `SWING_INTERVAL_TICKS` bare ticks.
  **The storm does not want this**: it holds `BTN_PRIMARY` every tick and lets
  `gather.rs:816-819`'s own gate rate-limit, which is the cadence a player
  actually produces and the one the fan-out is priced against.

**Arrangement.** Reuse `raid_storm.rs:158-189`'s `plots()` and
`PLOT_SPACING = 7` (`raid_storm.rs:129`) unchanged — 21 m between plots against
a 2 m reach means no swing can cross plots and every kill is attributable. Two
bodies per plot: A at the cell centre, B via `place_in_front(w, a, b, YAW_A, 1.0)`.
Both face each other (`YAW_B = YAW_A.wrapping_add(u16::MAX / 2 + 1)`), so
**both** swing and both die — a one-sided duel halves the death rate and the
bag pressure.

Why 1.0 m and not 0.0: at `d2 <= POINT_BLANK_M2` (0.04, `gather.rs:96`) the
cone test is bypassed (`combat.rs:520`), which would make the gate green on a
facing bug. 1.0 m is inside the 2 m reach and outside point blank, so the cone
is genuinely under test — and `combat.rs:193-201`
(`out_of_the_aim_cone_is_a_miss`) is the single-site proof that it can say no.

**Self-restoring after death.** `Command::Respawn { id, on_bag: false }` the
tick after `players[slot].dead` goes true, then re-seat the body with the same
two-line arrangement. That re-seat is a *fixture act* stated in the header,
exactly as `restock` is: without it the ring scatters every body across the
island (`tests/combat.rs:378-391` proves generation 1 lands on a different
beach on purpose) and the storm decays into a walk.

## 3 · Which caps a fight presses that walking does not

Each with its cap, its stated overflow policy, and the observable a gate would
read. Wall 4's phrasing: a cap **and** a policy.

### The three it presses hardest

**1. `MAX_EVENTS_PER_TICK = 256`** (`limits.rs:592`) — policy **drop newest,
counted**; "the late-join slot sync re-derives anything a lost event failed to
announce."

A landed swing spends three ring slots: `EV_SWING` (`gather.rs:832`) plus
`EV_HIT` and `EV_HEALTH` (`combat.rs:542-543`); a kill spends two more,
`EV_DEATH` (`combat.rs:544`) and `EV_BAG_DROPPED` (via `world::die` ->
`backpacks.drop_for`, `world.rs:1898-1899`). At `MAX_PLAYERS = 100` on a
synchronised swing tick that is 300-500 pushes against 256. Walking produces
`EV_GATHER` at best, one per swing, and never the death family.

*Observable:* `w.events.len()` and `w.events.dropped` (`world.rs:690, 707`)
read after `w.tick`. Assert `peak_events == MAX_EVENTS_PER_TICK` and
`overflow_ticks > 0` (`raid_storm.rs:466-474`'s exact pair; the exact-fullness
assertion is what stops a ring that overflowed by growing). Then the half
`raid_storm` cannot state: **assert the world survived the drop** by running the
storm twice and comparing `state_hash()` — a drop-newest ring is deterministic
or it is not a policy.

**2. `EVENT_RING_CAP = 64`** (`limits.rs:378`) — policy **resync**; a refused
push flags `ev_resync` and every walk restarts (`server/src/client.rs:282-300`).

This is the sharpest of the three and the one nothing in the tree has ever met.
**The cap is smaller than the player count**: 64 < 100. `pump_events`' broadcast
arms push once per connected client per event (`server/src/core.rs:2015-2026`
for `EV_SWING`), and the ring lives per connection (`server/src/net.rs:1291`,
pushed at `net.rs:2120-2127`). So **any single tick carrying more than 64
broadcast events forces a resync on every connected client at once** — and
`EV_SWING` alone reaches that at 65 simultaneous swingers. A resync is not free:
`ev_resync` rewinds the catalog, recipe, research, piece-def, piece-sync,
deploy-sync and bag-sync cursors and re-drips all of them (`client.rs:283-300`),
i.e. the overflow *increases* event-lane traffic, which is the shape of a
cascade.

Walking cannot produce 64 broadcast events in a tick. A brawl can, and a mass
death produces `EV_DEATH` + `EV_BAG_DROPPED` broadcast per victim on top.

*Observable:* `ShardStats::ev_resyncs` and `ev_sent` (`server/src/stats.rs`),
read as integers after a counted number of ticks. Assert the policy holds rather
than that it never fires: resyncs > 0 under a synchronised swing tick, and every
client's walks complete within a bounded number of following ticks with the
catalog and bag sets re-derived — i.e. **nothing is permanently lost**, which is
what "resync" claims.

**3. `MAX_BACKPACKS = 256`** (`limits.rs:395`) — policy **evict the bag nearest
its own despawn**, counted by `EV_BAG_REMOVED` with `BAG_GONE_EVICTED`
(`backpack.rs:70, 316-323`). Explicitly **never refuse**: "a death that silently
kept the inventory would make the cap a way to dodge the loss"
(`limits.rs:388-390`).

100 bodies at a 3-swing TTK and a 38-tick cadence (`gather.rs:77`) is one death
per body per ~114 ticks, so the store crosses 256 in about 300 ticks of steady
brawling against a 90-tick fixture despawn. Walking makes a bag only from a
smashed barrel, and not at rate.

*Observable:* `w.backpacks.len()` (`backpack.rs:207`), and the event codes:
assert `peak_bags == MAX_BACKPACKS` (saturation, in `raid_storm`'s equality
discipline) **and** at least one `EV_BAG_REMOVED` carrying `b == BAG_GONE_EVICTED`
**and** that the number of bags stood up equals the number of deaths — the
"never refuse" half, which a count equality is the only way to see.

### Two more the storm should reach, staged behind the three

**4. `MAX_ARROWS = 128`** (`limits.rs:685`) — policy **refuse the shot**,
checked *before* the ammo leaves the quiver, and deliberately not drop-oldest
("stealing a live arrow out of the air to make room would make a hit depend on
how many other people were shooting"). A bow storm at `MAX_PLAYERS` is the only
thing that fills it. *Observable:* `peak_arrows == MAX_ARROWS`, plus the ammo
count in the shooter's inventory **unchanged** across a refused shot — that
second one is the whole of what "refuse before the quiver" means and no count of
arrows can see it. `tests/shoot.rs` is the single-site sibling.

**5. `MAP_MARKS_MAX = 64`** (`client/src/ui/map.rs:238`) — policy **drop newest,
and the refused count is kept on `Marks`** rather than discarded. Two
compile-time asserts already protect the *own* tier (`map.rs:245-248` and
`map.rs:255-258`), and §0die records that the own bag was moved ahead of the cap.
What is unmeasured is the runtime case those asserts were written for:
`resolve_marks` (`map.rs:401`) fed 100 stranger bags. *Observable:*
`Marks::dropped > 0` **and** the own bag and own beds still present in
`Marks::a`. This is a `crates/client` unit test, not a sim one, and it is the
cheapest item in this whole note.

`MAX_REMOVALS_PER_TICK = 64` (`limits.rs:643`) is deliberately **left to
`raid_storm`** — it already saturates it (`raid_storm.rs:462-465`). The one
thing a melee adds there is a second producer: `combat::raid` (`combat.rs:602`,
called at `world.rs:3278`) takes pieces off the board via a swing rather than a
fuse. That belongs in slice 4, and it belongs as a *reachability* assertion
(`EV_PIECE_REMOVED` attributable to a swing), never as a second equality
competing with `raid_storm`'s.

## 4 · Every assertion is observable state; not one is a millisecond

`CLAUDE.md`: "Assert on observable state (`inWorld`, `snapshots > n`) and never
on elapsed milliseconds... Widening a timeout is not a fix; it is the same bug
with a longer fuse." Three identical runs, two different failures, 2026-08-01.
`server/tests/raid_shape.rs:8-12` restates it for this exact family: *"do not
lengthen a wire gate to reach a detonation — that is a gate waiting on a clock."*

| claim | observable | read where |
|---|---|---|
| every cap holds, every tick | `w.events.len()`, `w.backpacks.len()`, `w.charges.len()`, `w.pieces.len()` | after each `w.tick`, `raid_storm.rs:369-393`'s loop |
| the cap was *under pressure* | peak == the constant | `raid_storm.rs:448-474` |
| the policy fired | the code the policy emits (`EV_BAG_REMOVED`/`BAG_GONE_EVICTED`) or the counter it keeps (`events.dropped`, `Marks::dropped`, `stats.ev_resyncs`) | — |
| the fight actually happened | distinct `EV_*` codes seen, >= 1 per verb | `raid_storm.rs:497-535`'s breadth test, extended with `EV_HIT`, `EV_DEATH`, `EV_BAG_DROPPED`, `EV_RESPAWN`, `EV_SWING` |
| deaths were real, not a hp bug | `sum(players.deaths)` > 0 **and** `tests/combat.rs:424`'s guard clause verbatim ("this test would pass on a world with no combat at all") | — |
| determinism through a graveyard | two `storm()` runs' `state_hash()` and `save_world` bytes | `raid_storm.rs:541-568` |
| the fan-out ratio | `swing frames sent / swings taken` — two counted integers | §5 |
| a rate ("per second") | `bytes x TICK_HZ / ticks`, an integer tick count converted by `limits.rs:15` | never an `Instant` |
| a socket-carried slice terminated | a **failure** bound that prints every counter, not a deadline anything passes by beating | `server/tests/population.rs:29-35`'s `LOOKS` |

The last row is the only place a clock is even in the room, and it is in the
shape `population.rs` already licensed: an iteration ceiling whose only job is to
fail loudly.

## 5 · Pricing the `EV_SWING` fan-out honestly

This is a **server** measurement. `raid_storm.rs` drives `World` directly
(`raid_storm.rs:367`) and never encodes a packet, so the sim-side sibling in
§1-4 cannot see a byte of it. Say so in `melee_storm.rs`'s header so nobody
looks for it there.

### The harness

`crates/server/tests/snapshot_budget.rs` carries it, and it is the only one of
the three candidates that carries it *without a clock*:

* `clustered_core` (`snapshot_budget.rs:31-50`) connects all `MAX_PLAYERS` and
  herds every body into a 60 m square — well inside `AOI_ENTER_CM`, so the
  AOI-filtered and unfiltered cases are distinguishable by construction.
* `core.push_input(slot, &InputDatagram)` (`snapshot_budget.rs:186-191`) feeds
  inputs with no `ClientCore` per player — 100 `ClientCore`s would be the stack
  problem `snapshot_budget.rs:26-32` already documents for `ShardCore` itself.
  `InputDatagram::push` takes a real `InputFrame` (`protocol/src/lib.rs:2158`),
  so `buttons: BTN_PRIMARY, sel: 0` goes in as a real frame.
* `core.tick_bare(stats, |lane, slot, bytes| ...)` (`server/src/core.rs:754`)
  hands the closure the **real encoded event bytes** off `pump_events`. Filter
  `Lane::Event` and `protocol::decode_event` each one to attribute it to
  `SUB_SWING` (`protocol/src/event.rs:2915`) — the same decode-what-was-sent
  discipline `server/tests/gather_wire.rs:99-120`'s `pump_seen` uses, and for
  the reason it states: "a client mirror can agree with the world for reasons
  the ENCODER never earned, so anything the wire alone is responsible for —
  routing, above all — has to be asserted on what was sent."
* Add `FRAME_PREFIX_BYTES` (`server/src/net.rs:1683`, = 2) per frame, because
  that is what the real writer counts (`net.rs:1660-1671`) and what
  `net_stream_out_bytes` means (`stats.rs:370-374`). A count that leaves it out
  understates the lane, and the event lane is the one that sends many small
  frames.

`client_loop.rs` is the wrong harness — it runs two clients
(`client_loop.rs:456-461`) and the whole question is the 100th.
`population.rs` and `bot_smoke.rs` are the wrong *first* harness: they are real
sockets on a shared four-core box, so they corroborate a number they cannot
establish.

### The number, and what it is divided by

Report four, and the fourth is the gate:

1. **Swing bytes per client per simulated second** — sum of `(6 + 2)` over
   `SUB_SWING` frames addressed to one slot, x `TICK_HZ` / ticks run.
2. **Swing frames per client per simulated second** — the count half.
   `stats.rs:333-337` is emphatic about why both: "bytes alone cannot separate
   'more packets' from 'fatter packets' — the same kB/s is a snapshot rate
   problem or an AOI fill problem depending on which half moved, and those have
   opposite fixes."
3. **The share**: (1) and (2) over the whole event lane, and over the snapshot
   lane (`DATAGRAM_BUDGET_BYTES` 1100 at `SNAPSHOT_INTERVAL_TICKS` 2 -> 15 Hz ->
   ~16.5 kB/s/client ceiling).
4. **The amplification**: `swing frames sent / swings taken`. Today that must
   equal the connected client count, and asserting it does is what makes the
   measurement a gate rather than a `println!`. When a filter lands, the same
   assertion becomes `== |interest set|` and the gate *is* the fix's proof.

### What the arithmetic predicts, so a measurement can disagree with it

A swing frame is `KIND_BITS 4` (`protocol/src/lib.rs:671`) + `SUB_BITS 6`
(`event.rs:102`) + 32 bits of swinger (`encode_event_swing`,
`event.rs:1988-1992`) = 42 bits, and `BitWriter::finish` is `div_ceil(8)`
(`protocol/src/bits.rs:115`) -> **6 B payload, 8 B on the stream.**

Cadence is `SWING_INTERVAL_TICKS = 38` (`gather.rs:77`) -> 30/38 =
**0.789 swings/s** per player holding primary. At 100:

* ~79 swings/s shard-wide -> **7,895 event frames/s shard-wide**;
* **~79 frames/s and ~632 B/s per client**;
* ~63 kB/s shard-wide on the reliable lane.

So the honest headline is that **the fan-out is cheap in bytes and expensive in
frames and ring slots.** 632 B/s against a ~16.5 kB/s snapshot lane is ~4%. But
79 ordered STREAM frames per second per client — 7,900 shard-wide — is 79 writer
wakeups per client per second against a `WRITER_POLL` of 2 ms (`net.rs:46`) and
a ring 64 deep (`limits.rs:378`). **The red condition is not kB/s. It is
`EVENT_RING_CAP` and `ev_resyncs`,** which is why §3 item 2 and this section are
one measurement taken twice.

### The cheapest slice of all: the existing soak already swings

`bot_frame` presses `BTN_PRIMARY` on a 1-in-3 roll (`bots.rs:53-60`), so a soak
bot swings every ~40 ticks — 0.75/s. The 100-bot soak of 2026-08-12 (`NOW.md`
§0q item 4) was **already** carrying ~75 swings/s of fan-out. It could not
report it for two reasons that both resolve today: `EV_SWING` landed 2026-08-18
(wire v47, §0sw) and the byte counters landed 2026-08-18 (`stats.rs:321-385`).
So **re-running the existing soak with no code change prices this**, using
`net_stream_out_bytes / net_stream_out_frames` for the mean event frame,
`net_stream_out_frames / ticks / players` for the per-client frame rate, and
`BotReport::ev_in_bytes` (`server/src/botclient.rs:84`) for the per-client
*distribution* that a shard total divided by a gauge can never be
(`stats.rs:338-345` makes exactly this argument).

Two caveats to write into whatever records that run. `bot_frame`'s bots swing at
scenery, so their `EV_SWING` fan-out is real but their
`EV_HIT`/`EV_DEATH`/`EV_BAG_DROPPED` traffic is not — the soak prices the swing
lane and not the death lane. And the handshake and QUIC's own framing are
excluded by construction (`stats.rs:346-350`); `net_sent_packets` is the other
half of the overhead ratio.

## 6 · What a red result means, and which fix the tree is shaped for

`pump_events` has no distance test anywhere except chat's
(`server/src/core.rs:1257-1272`), and **three** events share the posture, not
one: `EV_SHOT` (`core.rs:1976-2000`), `EV_SWING` (`core.rs:2002-2031`),
`EV_IMPACT` (`core.rs:2033-2067`) — each an unconditional
`for slot in 0..MAX_PLAYERS`. Any fix applies to all three.

Three shapes, one line each:

* **(a) AOI filter on the event lane** — skip a recipient whose interest set
  does not hold the event's subject.
* **(b) Rate limit** — cap broadcast cosmetics per client per tick.
* **(c) Coalesced swing record in the snapshot** — a `swinging` bit per entity
  beside `dead`, wire v48's shape.

**(a), and the tree is already shaped for a specific form of it.** The cheapest
correct filter is not a distance test: it is the interest flag the snapshot fill
already maintains and already ranks — `self.clients[slot].interest[w]`, which
`snapshot_budget.rs:56-62` reads directly and which
`AOI_RANK_ENTER`/`AOI_RANK_EXIT` (`limits.rs:100-116`) already bound at 45/64
with hysteresis. One array read per recipient, no arithmetic, no second opinion
about who is near whom, and it inherits the edge-dancer hysteresis for free.
That drops the fan-out from `N` to `|interest set| <= AOI_RANK_EXIT = 64`, and
on a spread-out shard to far less.

**The three events do not take the same form of (a), and that is the finding.**
`EV_SWING`'s `a` is the swinger's id and `EV_SHOT`'s `a` is the shooter's id
(`core.rs:1982`, `core.rs:2020`) — both name a **body**, so both take the
interest-flag test. `EV_IMPACT` packs a surface class and a coordinate into
`a`/`b`/`c` (`core.rs:2044-2047`) — it names a **place**, which is in nobody's
interest array, so it needs chat's arm: `live_wslot(to_slot)` plus a planar
compare against `AOI_ENTER_CM` (`core.rs:1261-1271`, liftable almost verbatim).
Two shapes over three events.

Why not (b): a rate limit drops a fact by arrival order rather than by
relevance, and this queue already *has* an overflow policy one layer down
(`EVENT_RING_CAP` -> resync, `limits.rs:373-378`). A second policy for one queue
is what wall 4 is against.

Why not (c): §0sw settled the shape argument on the neighbouring case — "a death
is a condition rather than an instant, so it landed as a wire bit (v48) and not
as an event." A swing is an instant. A bit sampled at
`SNAPSHOT_INTERVAL_TICKS = 2` (`limits.rs:55`) against a 38-tick cadence either
misses swings or has to latch, and latching is the condition/instant mismatch
arriving in a costume. It is also not free: 1 bit x 64 entities x 15 Hz
~= 120 B/s/client, flat — cheaper than 632 B/s today, and *more* than (a) leaves
on a spread shard.

**Three broadcasts stay broadcast and the fix must say so.** `EV_DEATH` is
deliberately not AOI'd — "a death is a world fact, and it is what a kill feed is
made of... the reference frames' feed reports kills nobody saw"
(`core.rs:1661-1663`). `EV_BAG_DROPPED` is a world fact that stays
(`core.rs:1740-1743`). `EV_HIT`/`EV_HEALTH` are already unicast
(`core.rs:1633-1640`) and are not in this family at all.

**And the wire cost of (a) is zero.** No `PROTO_VER` bump, no golden, no client
line — routing is not layout. That is the same argument §0mk item 1 made when a
struck node got its mark for free.

## 7 · Staging, cheapest first, with the gate each slice earns

**S0 — re-run the 100-bot soak. No code.** The counters and `EV_SWING` both
landed 2026-08-18; the bots already swing (`bots.rs:53-60`). Divide
`net_stream_out_bytes` and `net_stream_out_frames` by ticks and by `players`,
and read `BotReport::ev_in_bytes` per bot. *Earns:* a measured kB/s and frames/s
per client attributable to the event lane, and the first honest look at
`ev_resyncs` at population. *Gate:* none — this is a measurement, and it lands in
`DECISIONS.md` §open beside the 2026-08-12 baseline.

**S1 — `MAP_MARKS_MAX` at mass death.** One `crates/client` unit test against
`resolve_marks` (`map.rs:401`) with 100 stranger bags. *Gate:* `Marks::dropped >
0` and the own tier intact. Cheapest thing in the note, and it closes the runtime
half of two compile-time asserts.

**S2 — the fan-out gate.** New test in `crates/server/tests/snapshot_budget.rs`
(its `clustered_core` is the fixture): `MAX_PLAYERS` connected and clustered,
`BTN_PRIMARY` held, counted ticks, and the four numbers of §5. *Gate:* `swing
frames sent == swings taken x connected clients`, plus the byte and frame rates
printed. Red-proof: drop the broadcast loop to `send(..., ev.a's slot, ...)` and
the ratio goes to 1.

**S3 — `test_melee_storm`.** `crates/sim-core/tests/melee_storm.rs`, the sibling
of §1-4: 64 bodies in 32 duels, self-restoring across death,
`MAX_EVENTS_PER_TICK` and `MAX_BACKPACKS` saturated, determinism and a save/load
round trip through a graveyard. *Gate:* the caps, the policies' own
codes/counters, the breadth test extended with the death family, and
`state_hash` equality across two runs.

**S4 — the AOI filter, if S2 is red.** Interest-flag test for `EV_SWING` and
`EV_SHOT`, chat's distance arm for `EV_IMPACT`. *Gate:* S2's ratio assertion
becomes `== |interest set|`, and `gather_wire.rs:376-431`'s
`a_swing_reaches_every_client_not_just_the_swinger` has to be re-read and
probably re-titled — it currently asserts the opposite ("a broadcast reaches
every connected client the same number of times", `gather_wire.rs:427-430`) and
it is exactly the gate a filter would turn red. That is the gate doing its job,
and it is also a warning: **do not land S4 without reading that file**, because
turning it green the lazy way retires the routing proof.

**S5 — the ranged and structural arms.** `MAX_ARROWS` saturation and its
refuse-before-the-quiver invariant; `combat::raid` as a second producer of piece
removals. Both deliberately last: `NOW.md` §0pvp item 5 says there is no lag
compensation and §0mk item 2 says `collide::shot_blocked` never reads
`ColIndex::planes` (`collide.rs:1014`, `collide.rs:211`) — so an arrow ignores
floors today, and a population gate on a path with a known-wrong predicate is
volume, not progress.

## 8 · What this note does not claim

Nothing here was compiled or run — the box had another agent building. Every
`file:line` is a read, and the first act of any slice is to re-check its own
citations rather than trust this file's memory of them (`CLAUDE.md`: "`ls` the
file, do not trust either doc's memory of it"). The arithmetic in §5 is a
*prediction stated so a measurement can contradict it*, in the shape
`CLAUDE.md`'s sweep-window trap asks for: one number checked against a second
source before it earns a gate.
