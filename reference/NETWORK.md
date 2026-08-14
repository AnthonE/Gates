# reference/NETWORK.md — how the reference game does netcode

Ripped facts, not design. `rust-systems.txt` answers *what systems exist*,
`SAVES.md` *what survives a restart*, `DOORS.md` *who gets through*; this
file answers **what the reference game learned about moving state to a
client**, across twelve years of shipping it wrong in public and saying so.

Dated 2026-08-10. §9 is the part that changes what we build, and unlike the
other `reference/*.md` it is written **against the tree**: every claim in it
carries a `file:line` and was checked on this commit.

## 0 · Provenance — read this first

`DOORS.md` §0's ranking, with its caveat, and one addition.

1. **The developer's own devblogs and update posts**, by number and name.
   A sentence a developer published about their own work is the strongest
   tier available for netcode, because none of this is in the hook table —
   `rust-systems.txt` names verbs, not pipelines.
2. **Community wikis, host guides and press coverage** for corroboration of
   a number. Weakest tier.

**The proxy caveat is unchanged and applies to every citation below.** Every
attempt to fetch `rust.facepunch.com` from this container was refused
(`EGRESS_BLOCKED`), so the devblog facts here come through **search-result
summaries of those pages, not the pages themselves**. A summary can drop a
qualifier. Where a number appears below it was corroborated by at least two
independent summaries, and where it wasn't, the sentence says so.

Nothing here was decompiled. Nothing here ships. Every architecture named
below is theirs; §9 is ours.

## 1 · The thesis, which is not about the socket

Twelve years of their public record converge on one claim, and it is worth
stating before any of the detail:

> **The expensive part of multiplayer is not the transport. It is deciding
> which client is allowed to know what — and everything downstream of that
> decision.**

They changed networking library twice, changed transport once more, and
spent every one of those migrations discovering that the library was not
what was costing them. What cost them was: interest management, snapshot
construction, serialization allocation, validation, and doing all five again
per player. The corollary they eventually made explicit:

> **The cheapest entity to serialize is an entity you decide not to send.**

## 2 · Do not replicate the world (2014, Devblog 32)

The first implementation sent every entity to a joining client, and could
effectively send the entity state again after a respawn. They changed it so
a client received global entities first, then entities around its spawn
position. The same cleanup compressed larger packets and removed unused
replicated variables — about **20 bytes per entity**.

Twenty bytes is nothing until it is multiplied by the entity count, by the
player count, and by the update rate. That multiplication is the whole
subject.

## 3 · The library became the problem, twice

**Lidgren (2014–15).** Two failures worth memorising, because neither
presented as a network error.

- Reliable messages sometimes were not delivered. Clients did not learn
  entities existed. The symptom players reported was **walls missing**.
  Traced to a Lidgren update and reverted (Devblog 41).
- After ~50 players had connected and a few hours had passed, everyone on
  the server dropped out every few minutes. The cause was Lidgren's packet
  **recycling** structure growing without bound and periodically stalling
  the networking thread. Because the symptom looked external, the
  investigation started at **DDoS** (Devblog 43).

**RakNet (2015, Devblog 48/50).** They switched partly for native C++ over
C#, hoping to cut managed-runtime pressure, and partly for RakNet's
battle-tested reputation. Then entities began disappearing: the server was
certain it had sent entity 48320, the client was certain it never arrived.
They added an **entity checksum** to the networking process to detect the
mismatch — the immediate effect of which was that clients missing entities
got **kicked**, which is a diagnosis, not a fix. The actual cause: when a
packet exceeded MTU and RakNet fragmented it, a reassembly in which **the
last fragment arrived before another fragment** produced a wrongly
reconstructed packet. They patched RakNet themselves.

Alongside the fix they added network statistics, traffic logging, netgraph
and diagnostics. That instrumentation is arguably the more durable half.

The lesson generalizes past RakNet: **`RELIABLE_ORDERED` on the tin is not
proof of the implementation.** Fragmentation, reordering, loss, duplicates,
late arrivals, burst loss, MTU transitions and queue overflow are things you
torture-test yourself.

**And RakNet then became the debt.** By 2020 upstream had been dormant for
years while their fork accumulated fixes. They modularized the transport so
backends could coexist and put Valve's Steam Networking Sockets behind
`-swnet`, opt-in, recommending community servers stay on RakNet until it was
proven. A planned default flip in late 2022 was postponed when more problems
turned up. **The migration pattern is the takeaway: a compatibility layer,
never a flag day.**

## 4 · The freeze was serialization, not transport (Devblog 79)

Servers froze periodically at otherwise acceptable frame rates. Profiling
put the network system among their largest garbage producers. The pipeline
was: fill a protobuf object → serialize it → allocate a `byte[]` → hand it
to RakNet → let the collector clean up, once per entity update, times every
entity, times every player.

Two fixes: **pool the protobuf/network objects**, and **serialize directly
into the network stream** instead of through an intermediate `byte[]`.

> Bandwidth can be entirely healthy while memory churn destroys the tick.

## 5 · A byte budget is not a time budget (Devblog 77)

A network message can arrive about an entity that is still being spawned
locally, so `Spawn #100 / Update #100 / RPC #100 / Destroy #100` has to
survive creation crossing a frame boundary. They tried deferring entity
spawns and found that the hard part is precisely the messages that arrive in
between. The shipped workaround: **a per-frame time budget for network
processing, past which all remaining messages are delayed to the next
frame.** (Building-skin spawns got the same treatment at a 1 ms budget.)

So the cost of a network message is:

```
bytes + deserialize + entity lifecycle + gameplay processing
```

50 KB that instantiates 300 prefabs is worse than 400 KB that does not. And
**stream-out is the half everyone forgets** — teardown spikes too.

## 6 · Client responsiveness plus server verification

They did not make shooting wait for the server, and they did not trust the
client's damage claim. Projectiles are client-simulated and the server
independently verifies the claim against its own representation: weapon
capability, fire rate, projectile speed, line of sight, trajectory
checkpoints, ricochets, periodic position updates (Devblogs 110/123).

The correct framing is not client-authority → server-authority. It is:

```
client responsiveness  +  server authority/verification
```

**And verification produced its own bug class.** The server's view is never
the client's — latency, interpolation, tick boundaries, physics disagreement
— so the checks rejected **legitimate hits**. An over-strict projectile-speed
anti-cheat check rejected real bullets (Devblog 118). The fix was never
"validate everything":

> Validate against tolerance envelopes sized by what the network could
> plausibly have produced — and log the reason a rejection fired.

Separately, authority kept being pulled out of places it had lingered:
ladder movement was client-authoritative and trivially cheatable until they
added server verification (Devblog 155).

## 7 · Interest management, sharpened for a decade

This is where their real scalability wins are.

- **Network groups.** The world is divided into chunks; a player subscribes
  to nearby chunks; the server networks those instead of iterating the world
  per player.
- **Vertical layers** (The Big QOL Update). The grid was 2D, then they added
  underground train tunnels — and a player on the surface and one directly
  below shared an X/Z cell. Tunnel entities no longer network to the surface
  and vice versa: **up to ~0.4 ms of server frame time** saved in one
  scenario, just by not networking tunnel NPCs to surface players.
- **Shape and per-entity range** (Spring Clean, April 2026). The AOI region
  had been a large square, which networks much further toward the corners
  than toward the edge midpoints; it became a blocky circle. And different
  entity kinds got **different network ranges** — decor and dropped items
  network only up close, vehicles much further. Underground culling improved
  again, and the streaming path stopped needlessly flushing queues per
  player.

## 8 · Then: can the player actually see it?

Distance still leaks. An enemy five metres away behind a wall is inside any
radius, gets sent, and an ESP cheat renders it. They first stopped
networking specific hidden information (buried stashes, Waves of Change),
then pursued **server-side occlusion** as what they called a holy grail: if
terrain blocks the line, the player is not sent at all, and a cheat cannot
render what never arrived. Experimented through 2024, **default-on for
players in 2025** (Road Renegades), terrain-only at first, with rocks and
cliffs slated to join the occlusion grid bake. It was rolled out gradually
on official servers at 350 population.

**It is not free, and the way it is not free is instructive.** Reported at up
to **~8 ms** in bad cases with ~10,000 occlusion queries. The shape is
combinatorial: 350 players is 350 × 349 = 122,150 directed pairs before any
culling. Their mitigations were deduplicating reciprocal pairs (A→B answers
B→A), caching results for the duration of a frame, and parallelizing.

Which yields the staging rule:

```
1  cheap broad-phase spatial rejection
2  network groups
3  distance, per entity kind
4  expensive visibility
5  serialize
```

**Never run stage 4 over the universe.**

### 8.1 · Threads help only after the dataflow is separable

Multithreaded networking shipped experimental in early 2023 (`-networkthread`),
cautiously, because threading bugs in networking are catastrophic and hard to
reproduce; a month of fixes later it was default on clients and servers
(Industrial Update → Eye In The Sky). Along the way: a thread spinning at
100%, memory-pool contention, thread-safety fixes.

Then the better lesson, in late 2025 (Pivot Or Die). Their multithreaded
player-update processing ran **as slow as the serial version**. All threads
including main bottlenecked on one lock: the pool takes a lock per type,
which is fast single-threaded and does not scale. Total time across threads
for pushing entity snapshots reached **100 ms**. Rather than rewrite the
pool, they **pre-allocated the necessary state on the main thread** so
workers stopped contending.

> Multithreading a bad dataflow gives you a multithreaded bad dataflow.
> Ownership has to be separable before jobs help.

By Common Ground (July 2026) the jobs architecture was default in
production, with work continuing on parallel network-subscription updates,
anti-cheat processing, snapshot work, and occlusion pair gathering — plus
eliminating the job system's own allocations.

### 8.2 · Authority does not have to cover every degree of freedom

Corpses: the server was authoritative over the gameplay-relevant loot
position while the client simulated the detailed ragdoll cosmetically. Smart
— and the two simulations disagreed, so moving vehicles and differing
collision geometry could stretch the visual ragdoll away from the
authoritative bag, eventually forcing a rework (10 Years Of Rust).

> Make the server authoritative over gameplay, not over every visual degree
> of freedom — and know where the two can visibly diverge.

---

## 9 · What it means for us

Measured against the tree on 2026-08-10. Every line carries a cite; where
the answer is "absent", that is stated as absent rather than implied.

### 9.1 · What we already got right, and should not relitigate

These are not aspirations — they are in the code, and several of them are
end-states the reference game took years to reach.

- **A joining client is not sent the world.** `Welcome` is four fields —
  `player_id`, `seed`, `tick`, `dev` (`protocol/src/lib.rs:460`). Terrain is
  regenerated from the seed. §2's 2014 mistake is unreachable *for terrain*
  by construction. (Not for built structures — see 9.2.1.)
- **Nothing rides the snapshot path unless it moves.** `encode_snapshot`
  iterates players and mobs and nothing else (`server/src/core.rs:2354`,
  `:2360`). Building pieces, deployables, backpacks and settled items are
  event-replicated. This is §1's thesis, structurally.
- **The AOI test is already radial, not square.** `d2 <= AOI_ENTER_CM²`
  (`server/src/core.rs:2189`) — we start where Spring Clean 2026 finished,
  with hysteresis (176 m in / 208 m out, `limits.rs:43`) on top.
- **A snapshot sheds entities, it never fragments.** Each `add_entity` is
  checked against the 1,100 B budget and an overflow skips that entity and
  tries the next, breaking after `FILL_OVERFLOW_STREAK`
  (`server/src/core.rs:2396–2408`). §3's RakNet fragmentation bug has no
  analogue on our snapshot lane because we never hand the transport
  something that needs fragmenting.
- **No GC, and also no per-tick heap on the sim thread.** `SnapMsg` is an
  inline `[u8; 1100]` (`server/src/slot.rs:123`) on a lock-free SPSC ring
  (`rtrb`), so §4's pipeline — object → serialize → `byte[]` → transport —
  has no allocation in it at all. Every `vec!` in `ShardCore` is in a
  constructor (`core.rs:208–288`). There is no `Mutex`/`RwLock` in
  `core.rs`.
- **Movement is server-simulated from buttons, not accepted as position.**
  `InputFrame` carries `seq, buttons, yaw, pitch, move_x, move_z, sel`
  (`sim-core/src/input.rs:56`) — no position field exists on the wire. §6's
  ladder-authority class of bug is structurally unreachable for movement.
  Undeclared button bits are refused at the door (`net.rs` `accept_input`,
  gated by `tests/domain_ledger.rs`).
- **Quantize-both-sides is real and shared.** `movement::step` is the same
  code on the server and in the predictor, over integer quanta
  (`sim-core/src/movement.rs:1–11`); arrows integrate in integer
  millimetres-per-tick (`sim-core/src/ranged.rs:11–22`). §6's
  "server's view is never the client's" is narrowed to genuine network
  disagreement rather than rounding.
- **The worst case is gated.** `server/tests/snapshot_budget.rs` clusters
  the full 100-player cap inside one AOI cell and asserts the byte budget,
  the staleness ceiling, own-entity presence, and byte-exact client
  reconstruction. That is a real gate on the thing §7 is about.
- **`send_datagram`, never `send_datagram_wait`**, on all three senders
  (`net.rs:1084`, `client/src/lib.rs:462`, `botclient.rs:132`).

### 9.2 · Ranked gaps — reachable first

**9.2.1 · The class-S join walk sends the whole world, and any removal
restarts it.** *(high — reachable at two players)*

`pump_events` walks `self.world.pieces.entries()` from a per-client cursor,
`PIECE_SYNC_BATCH = 32` per tick, with **no distance filter of any kind**
(`server/src/core.rs:1872–1877`; batch at `protocol/src/event.rs:52`). Same
shape for deploys (1,024 cap, batch 24) and backpacks. At `MAX_PIECES =
8_192` and 30 Hz that is **256 ticks ≈ 8.5 s** of drip to teach one joiner
about every structure on the island, near or far.

The restart is the sharp edge. A piece removal while a client's cursor is
inside the store resets that client to zero with a fresh reset batch
(`core.rs:1663–1670`) — correct under the store's swap-remove, and
deliberately tested as such (`server/tests/deploy_wire.rs:6`). But the cost
was never bounded: **on a 3,000-piece base a full walk is ~94 ticks (3.1 s),
and any raid removes pieces faster than that**, so a client joining during a
raid can be walked back to zero indefinitely and never finish learning the
world. This is §2's mistake and §2's re-send amplification in one place.

*Smallest honest fix:* not the chunk-subscription rewrite `NETCODE.md` §7
describes. Two cheap things first — count restarts and walk completions
(`piece_walk_restarts`, `piece_walk_completes`) so the livelock is
**visible**, and make the restart resume from a re-validated cursor rather
than zero where the swap-remove provably did not move anything before it.
The interest filter is the real fix and is a `NOW.md` item, not a patch.

**9.2.2 · An event-ring overflow costs the whole world, and that makes the
overflow more likely.** *(high — reachable whenever a client stalls)*

`ev_resync()` zeroes **every** walk cursor at once — pieces, deploys, bags,
catalog, recipes, piece defs, deploy defs (`server/src/client.rs:249–261`) —
and it fires when `send(Lane::Event, …)` returns false, i.e. when the
client's event ring is full, i.e. when the client is slow to drain
(`core.rs:1675–1678`). So: slow client → ring fills → resync → thousands of
records re-queued → ring fills sooner. **The recovery mechanism feeds the
condition it recovers from.**

This is structurally §3's Lidgren stall — a resource-recovery path that
makes the pressure worse — and it will present as a client that periodically
loses and re-learns every wall it can see, not as a network error.

*Smallest honest fix:* the counters (`ev_resyncs` already exists at
`stats.rs:68` — it is the *rate* that is unwatched) plus a test that drives a
non-draining client and asserts the resync count does not grow
superlinearly. A hysteresis or backoff on repeated resyncs is the design
question; `DECISIONS.md` §open is where it goes.

**9.2.3 · Shedding is invisible.** *(medium — the budget can be silently
saturated)*

When `add_entity` returns `Overflow` or `Cap`, `encode_snapshot` skips the
entity and bumps **nothing** (`core.rs:2396–2408`; contrast the
`encode_range_errors` bump three lines down). So the one mechanism by which
snapshot quality degrades under load has no counter. `snapshot_budget.rs`
proves the budget *holds*; nothing reports how close it runs or how often it
sheds.

*Smallest honest fix:* a `snap_entities_shed` counter at the two sites, and
an assert in `snapshot_budget.rs` that the clustered-cap scene both sheds
and stays inside the staleness ceiling. Cheap, and it is §3's instrumentation
lesson exactly.

**9.2.4 · The client applies network state with no per-frame budget.**
*(medium — reachable on join and on every resync)*

`drain_lane` loops `try_recv` until the channel is empty with no cap
(`client/src/lib.rs:218–227`) over a 256-deep channel
(`client/src/lib.rs:345`). Each message can be a 32-piece `PieceSync`, so a
single frame can apply **up to ~8,192 pieces** and whatever mesh work Bevy
then does. This is §5, unmitigated — and 9.2.1/9.2.2 are exactly the two
paths that fill that channel. The channel also carries `Vec<u8>` per
message, which is a per-frame allocation on a path CLAUDE.md's trap list
says must not have one.

*Smallest honest fix:* an applied-message cap per frame with the remainder
left in the channel, and a counter for frames that hit it.

**9.2.5 · `wall 2`'s gate stops before the replication pipeline.**
*(medium — the exact place Facepunch's stall lived is ungated)*

`tests/alloc_zero.rs` drives `World::new` / `World::tick`
(`alloc_zero.rs:258`) — `sim-core` only. It never constructs a `ShardCore`
and never calls `encode_snapshot` or `pump_events`, both of which run **on
the sim thread**. So an allocation introduced into the snapshot encoder or
the event pump would not redden any gate. §4 is the whole reason that
matters: their freeze was in the replication pipeline, not the simulation.

*Smallest honest fix:* a second counting-allocator binary in `crates/server`
that drives `ShardCore::tick` with the cap connected and asserts a zero
delta after warmup. Same allocator, same shape, one crate over.

**9.2.6 · Forty counters exist and three are readable.** *(medium)*

`ShardStats` collects 40+ atomics including `input_ring_drops`,
`snap_ring_skips`, `snap_send_errors`, `forced_resyncs`, `ev_resyncs`
(`server/src/stats.rs`). `GET /status.json` returns `players`,
`max_players`, `tick` (`server/src/status.rs:14`). Nothing else is printed,
logged or exposed. An operator cannot see any of §3's diagnostic surface
without a debugger.

Also absent, per-client and entirely: **RTT, loss, bytes sent, snapshot size
distribution, interest-set size, input-buffer depth, dilation-state
distribution, refusal counts by code, tick-time p50/p99.** Tick health is one
counter, `ticks_dropped`, bumped only past `MAX_TICK_BACKLOG`
(`net.rs:1551–1556`) — a shard can spend 25 ms of a 33 ms tick forever and
report nothing.

*Smallest honest fix:* widen `/status.json` to the full counter set. It is a
JSON writer over atomics the sim thread already stores, on a thread that
already exists and holds no lock.

**9.2.7 · The client's datagram send is unclamped and its failures are
discarded.** *(low — not reachable at current frame sizes)*

`net.rs:1077–1084` clamps the server's snapshot against the live
`max_datagram_size()` and counts the failure. The client does neither:
`let _ = self.connection.send_datagram(&self.input_buf[..len])`
(`client/src/lib.rs:462`), same at `botclient.rs:132`. CLAUDE.md's trap list
says the clamp stays on both paths. Today an input frame cannot approach
1,100 B, so the overflow is not reachable — but the **discarded error** is
reachable now, and it means a client whose inputs stop leaving has no
counter anywhere saying so.

**9.2.8 · No occlusion, no lag compensation, no vertical AOI.** *(roadmap,
not defects — recorded so nobody re-derives them)*

- `grep -rni 'occlu|line_of_sight|visib'` over `crates/` returns nothing on
  any network path. Every player inside 176 m is sent with position,
  velocity, grounded, sleeping, yaw and pitch (`core.rs:2255` `wire_entity`)
  regardless of what is between them. §8's ESP surface is fully open. Our
  cost shape is far kinder than theirs — 100 players is 9,900 directed pairs
  against their 122,150, and a seeded heightfield bakes — but it is
  unbuilt.
- No rewind ring, no favour-the-shooter, nothing. Melee and arrows resolve
  at present time (`sim-core/src/combat.rs:37`, which says so). At 100 ms
  RTT a moving target is hit where the server has it, not where the shooter
  saw it. `NETCODE.md` §8 specifies the Source three-term formula and the
  Overwatch bounds in full detail; none of it exists.
- AOI is planar x/z (`limits.rs:41`, `core.rs:2181`). That is §7's
  pre-tunnel state — and it is **correct for us today** because we have no
  verticality. It becomes their bug the day caves or stacked bases land, so
  it belongs in the same commit as the first one of those.
- Projectiles are not replicated at all (`ranged.rs:38`: "no arrow drawn on
  any screen"), so §6's client-projectile/server-verify split has nothing to
  audit yet. When it lands, the tolerance-envelope lesson is the one to read
  first — over-strict verification rejecting real hits is a *worse* player
  experience than the cheat it prevents.

### 9.3 · The doc/code delta, called out separately

`CLAUDE.md` warns that a doc reading as covered while nothing checks it is
the dangerous failure. `NETCODE.md` §11 is titled **"Added CI gates"** and
lists seven. **Zero of the seven exist**, verified by grep over `crates/`
and `ci/`:

| named in `NETCODE.md` §11 | in the tree |
|---|---|
| `test_chunk_epoch` | absent |
| `test_class_transitions` | absent |
| `test_raid_storm` | absent — **the wire storm below is; the name is now taken.** `crates/sim-core/tests/raid_storm.rs` (2026-08-14) is wall 4's caps gate under the same name and answers a different question: no socket, no subscribers, nothing timed |
| `test_stream_in` | absent |
| `netem profiles` | absent |
| `test_sleeper_soak` | absent |
| `bench_transport` | absent |

Three of them are the direct gates for the gaps above: `test_stream_in` is
9.2.1 and 9.2.4, `test_raid_storm` is 9.2.1's restart storm, `netem` is the
only thing that would exercise §3's torture list. The section has been
retitled and marked in `NETCODE.md` rather than deleted — the designs are
good, they are simply unbuilt.

Two further doc claims that the tree does not support:

- **§7 "the 64 m grid serves both pipelines… class S chunk subscriptions
  come from the same enter/leave sets"** — there is no grid and no chunk
  subscription. Class D is a flat O(`MAX_PLAYERS` + `MAX_MOBS`) scan per
  client per tick (`core.rs:2163`, `:2208`), which is 16,400 distance tests
  per tick at cap and entirely affordable; class S has no interest filter at
  all (9.2.1). One spatial truth, one product, not two.
- **§2.2 "wtransport git-pin ≥ commit `0f7609a`"** — the tree pins
  `rev = a11e6a8e…` (`crates/server/Cargo.toml:26`, `client/Cargo.toml:185`),
  resolving to `0.7.1` from git (`Cargo.lock`). Nothing in the repo records
  whether `a11e6a8` descends from `0f7609a`, and nothing gates it. Given
  that the pin exists to dodge a **two-byte remote panic**, the ancestry is
  worth writing down once next to the pin. This is §3's lesson at its
  smallest: the middleware version is a thing you verify, not a thing you
  assume.

### 9.4 · What does not transfer

- **The GC lessons, partially.** §4's *pause class* — a collector stopping
  the world — does not exist for us. Its *cause* does: allocator pressure
  and copies still stall, and a native pipeline compile is a bigger hitch
  than a WebGL link ever was. What transfers is the discipline (§9.1's
  inline `SnapMsg`), not the fear of the collector.
- **Their scale numbers.** 350 players, 122,150 occlusion pairs, 8 ms
  occlusion budgets, 100 ms of pool contention. We cap at 100 (`limits.rs:7`),
  which is 9,900 pairs. Their *shape* transfers; their *thresholds* do not,
  and quoting one as if it were ours is `reference/BALANCE.md` §4.1's
  false-familiarity trap in a different system.
- **The parallel-jobs arc (§8.1), for now.** Our sim thread is single and
  the walls forbid locks on it; we have no jobs to contend. Read it before
  the first one lands, and read the ordering claim rather than the fix: the
  dataflow has to be separable *first*.
- **`-swnet` / transport modularity.** We are on one transport with no
  installed base to migrate. What survives is the rollout doctrine — a
  compatibility layer, opt-in, never a flag day — which is worth having
  written down before the first shard is public.

## 10 · Sources

Tier 1 — the developer's own posts, all under `rust.facepunch.com/news/`,
all reached as search summaries (§0):

- `devblog-32` — spawn-local streaming, ~20 B/entity trimmed
- `devblog-41` — Lidgren reliable-delivery regression, missing walls
- `devblog-43` — Lidgren packet-recycling stall, mass disconnects
- `devblog-48` — the move to RakNet, and why
- `devblog-50` — the fragmentation bug, entity checksums, netgraph/diagnostics
- `devblog-77` — per-frame network processing budget; the 1 ms skin-spawn budget
- `devblog-79` — the network system as a garbage source; pooling; direct serialization
- `devblog-110`, `devblog-118`, `devblog-123` — projectile verification, and its false rejections
- `devblog-155` — ladder movement, client authority removed
- `the-big-qol-update` — vertical network layers, ~0.4 ms
- `waves-of-change` — not networking hidden information
- `cctv-update`, `elevator-update`, `prototype-17` — transport modularity, `-swnet`, the postponed flip
- `industrial-update`, `eye-in-the-sky`, `make-your-mark` — multithreaded networking
- `road-renegades` — server occlusion default-on
- `maintenance` — occlusion cost, dedup, caching
- `pivot-or-die` — the pool lock; 100 ms; preallocate-on-main
- `spring-clean` (April 2026) — circular AOI, per-entity ranges
- `common-ground` (July 2026) — jobs default, occlusion pair gathering
- `10-years-of-rust` — corpse authority split

Tier 2 — corroboration only: `x.com/Alistair_McF` (occlusion at 350 pop,
terrain-only), host/press summaries of Spring Clean and Common Ground.

Our own canon sources stay in `NETCODE.md` §12; this file does not restate
them.
