# Gates — design of record, v0.1

> the name is `Gates` (spoken 2026-07-31, DECISIONS.md). Everything here is
> buildable as written; knobs the operator must still rule on are marked
> **(knob)** and collected in §14. Written 2026-07-30, the night before the
> shell. This file drops at the repo root and is the design of record until
> superseded.

> **The browser client is deleted** (operator, 2026-08-06; the crate that
> still said otherwise went 2026-08-08). This file was written for a
> three.js page and every claim that depended on one has been corrected in
> place — §1 pillar 2, §5.1, §9, §11, §14. Where a number was *chosen*
> because the target was a browser it now says so rather than being
> silently reused. `CLAUDE.md` has the posture, including how to read the
> deleted client out of git history when a question about a verb needs it.

A survival game in the Rust (Facepunch) tradition: wake with nothing on a
hostile island, gather, craft, build a base, raid, lose it all, wipe, again.
100 players a shard. JUNK is the working coin you earn and risk in the world;
ELO and ORBS buy skins and nothing else. Backend in Rust (the language), a
**native Rust desktop client** (Bevy), transport QUIC via WebTransport.
**The v1 deliverable is the skeleton** — netcode, determinism, and the server
discipline laws — built so every later system (content, monuments, vehicles,
AI) is data and systems plugged into a frame that already doesn't blip.

This is a **separate product in its own repo**. It orbits elo (sold through
the Great Work board as quest `munus-first-sale`, settled in ELO, coins from
the elo economy) but imports none of its code and none of its rules except
the ones restated here. Tickers are bare: ELO, JUNK, ORBS — never a `$`.

---

## 1 · Pillars

1. **The skeleton is the product.** Multiplayer, determinism, and the
   hot-path laws come first and are gated by CI. Content is data. If the
   frame is right, the game grows for years; if it's wrong, no amount of
   content saves it.
2. **One native client, and the game is sold.** A Rust desktop binary on
   Bevy (`RENDER.md` owns its path), delivered as an elo depot the launcher
   installs (`ci/depot.py`). This pillar read *"browser-native, zero
   install — three.js + WebTransport"* until 2026-08-05, and **both halves
   were repudiated, not merely outgrown**: the operator retired instant
   guest play as a pillar outright (*"NO INSTANT GUEST PLAY IS NOT A PILLAR
   IN A RUST CLONE LOL… everyone has to buy the game"*), then cut the
   browser client the next day. What survives is the thing the old pillar
   was actually about — the shortest distance from interest to in-game —
   and its answer is no longer a URL. **What it is instead is open**
   (`DECISIONS.md` §open, "the board's playable link").
3. **The server never blips.** Fixed tick, no blocking, no locks, no
   allocations in the hot path, bounded everything, and a *defined*
   degradation ladder instead of an emergent freeze.
4. **The economy is honest.** Coins are earned in play and risked in play.
   The house sells appearance only. Settlement happens on RH-Chain through
   the same claim pattern elo already uses; the game server holds no keys
   and mints nothing.
5. **The core is deterministic.** Same build + same seed + same input log →
   same world, hash-checkable. That buys drift-free prediction, one-file bug
   repros, and a played round the elo board can machine-verify.

## 2 · The game, v1 loop

The Rust loop, cut to what a skeleton must prove:

- **Spawn** naked on a beach of a seeded island (~2km × 2km, heightfield
  terrain, biome tint by noise). Day/night cycle. Hunger/thirst minimal —
  a slow health drain past a timer, food to reset it **(knob: depth)**.
- **Gather**: trees, stone/ore nodes (hit the glowing weak spot to fell it
  **faster** — the node's payout is fixed, so skill buys time, not riches;
  and a share of it is withheld until the felling blow, so walking away
  from a half-cut tree costs you), and **salvage barrels** along
  roads/shore (the loot tension source; they drop components and JUNK
  salvage, §3).
- **Craft**: tier ladder. T0 rock/torch/spear → workbench → T1 bow, stone
  tools, furnace, metal fragments → T2 revolver, metal tools, armor. Guns
  exist but are expensive; melee/bow era dominates a fresh wipe.
- **Build**: foundation-grid building (foundation, wall, doorway, door,
  floor, stairs, roof) in wood → stone → metal. A **hearth** (the tool
  cupboard analog) claims building privilege in a radius; **upkeep** drains
  materials from the hearth and unpaid buildings **decay** — the map heals
  itself between raids. Upkeep is material-only, never a coin (§3.3).
- **Fight**: melee, bow, one T2 firearm at v1. Death drops your whole
  inventory where you fell; bags despawn on a timer.
- **Raid**: doors and walls have HP and material tiers; T2 satchel charge
  is the v1 raid tool. Offline raids are legal v1; protection mechanics are
  content for later, not skeleton.
- **Two monuments-lite** at v1: a road with barrel spawns and a **haven** —
  the safe zone (no build, no damage) holding the recycler, the bank
  terminal, and the skin vendor (§3).
- **Wipe**: the world ends on a posted schedule **(knob: cadence, default
  weekly map / blueprints survive one extra cycle)** — moved from monthly by
  the operator, 2026-08-10, on the ground that an update already wipes the
  map, so a monthly promise described a world that did not last a month.
  **BP survival did not move with it**, which makes the blueprint rule
  materially more generous at this cadence than it was at the old one. New
  seed, fresh island. What survives a wipe is exactly: blueprints (per schedule),
  banked JUNK, and skins. Nothing else.

Out of scope for v1, by design (the skeleton must not wait on them):
vehicles, farming, ~~animal AI~~, electricity, teams UI (informal groups
work day one), more monuments, anti-ESP occlusion culling (§10).

**Animals are back in** (operator, 2026-08-08 — `DECISIONS.md`). The pig
walks, flees and can be killed for fat and cloth; the design and what it
deliberately does not have are `reference/ANIMALS.md` §9. What stays out of
scope is the thing this line was really about — an **AI system**: nothing
hunts, packs or fights back, and the roster is 64 fixed slots with a
staggered think, not a spawner with behaviours.

## 3 · The economy

Three coins, three jobs, one law each. Every pair already has a live pool
on RH-Chain, so **the pools are the exchange** — the game never posts a
rate between coins, never converts in-game, never runs its own market
between them. A player holding any coin is one swap from any other.

### 3.1 · JUNK — the working coin (the scrap)

JUNK plays the role scrap plays in Rust: the ground-truth currency of grind,
trade, and progression — earned by play, spent in play, lootable in play.

**Two states, and they are two different things wearing one name — read
this before reading anything else about the coin** (operator, 2026-08-10:
*"this isnt the same as the crypto coin u can cash out as"*). The carried
half is an item and nothing more; the banked half is the claim rail. Only
the second is redeemable, only the second stages (`ALPHA.md` §2), and
conflating them once already shipped an inert currency for a day:

| state | where it lives | can you lose it? |
|---|---|---|
| **carried** | an item stack in your inventory, like any loot | yes — dropped on death, lost on wipe, raidable in your base |
| **banked** | your wallet's row in the shard ledger, written only at the haven's bank terminal | no — survives death and wipes, claimable on-chain |

Carried JUNK is what makes the game a survival game: the hoard in your base
is a raid target, the run to the haven with 300 JUNK on you is the tensest
walk in the game. Banked JUNK is what makes the economy real: a file-ledger
balance keyed to your wallet, exported at wipe (and on a posted cadence) as
a merkle root for the elo claim rail — the same play-accrues-off-chain,
settles-by-claim shape the town already uses (`ONCHAIN-LINE.md`). The game
server holds no keys, mints nothing, and the on-chain supply it draws from
is an operator-funded allotment on the elo side **(knob: allotment size and
claim cadence — an operator act, not a game mechanic)**.

**Faucets** (all in-world): salvage from barrels; the recycler (feed it
components → JUNK); a trickle from monument crates. **Built**
(`content/cooking.toml`, recycler v0) — the recycler is a placeable
machine rather than a haven fixture for now, which is the one thing that
paragraph promises and the world does not yet do.

**Sinks.** The distinction the rest of this section turns on applies here
too, and this line used to blur it: *carried* sinks burn an item stack in
your pocket, and only the bank terminal's fee touches a ledger.

- **Blueprint research — the main sink, exactly scrap's job. Built**
  (research v0): a sample plus coin at a research table, per player,
  saved. Carried, not banked.
- Recycler service fee and the haven's market-stall listing fee: unbuilt,
  both carried.
- The bank terminal's deposit fee **(knob: default 2%)**: the one that
  burns against the shard ledger, and it arrives with the terminal at A2.
  Banking costs a little so carrying stays rational; the fee burns so the
  house never earns from the game loop.

**Player-to-player is free.** Players may trade anything for anything,
including JUNK for a rifle. That is players trading with players — the
survival economy working. The wall is only ever about the **house**: §3.3.

### 3.2 · ELO and ORBS — the skin counter

The haven's vendor sells **appearance only**: weapon and clothing skins,
building cosmetics, dyes. Standard catalog priced in ELO; limited/seasonal
drops priced in ORBS (the scarcer, capped coin — that scarcity IS the
seasonal story) **(knob: final split and prices — posted at ship, never
invented here)**.

Purchase flow, v1 — custody zero, verification three-valued:

1. The catalog posts each skin's price and the shard's **till address**.
2. The buyer's wallet sends the exact amount on RH-Chain (chain 4663).
3. The game backend verifies the transfer against the explorer the same way
   elo's board verifies receipts: match on recipient, token, exact string
   amount; `true` unlocks, `false` says which field mismatched, and
   *explorer unreachable* is `null`/retry — never treated as false, never
   silently unlocked. x402 is the later upgrade path, not a v1 dependency.
4. The entitlement keys to the **wallet**, not the character: skins survive
   death, wipes, and shards. Tradable skins (on-chain editions) are a later
   gate, not v1.

Proceeds go to a posted address **(knob: fiscus vs a burn split — an
operator sentence; default: 100% to the posted fiscus address, burn 0%)**.

### 3.3 · What we sell

**Moved to `BUSINESS.md`.** We sell IAP: the game itself at a uniform price,
skins and appearance, and players sell each other everything. The one thing the
house does not sell is an advantage over another player — stats, upkeep pauses,
blueprints, loot odds, queue priority. A skin is not an advantage.

It lives in its own file because it is a product decision the operator owns,
it has no gate, and carrying it inside the engineering docs cost context on
every pass that never touched money. Price and currency stay `DECISIONS.md`
§open.

## 4 · Architecture

Cargo workspace, five crates today, one law about where code may live:

```
gates/
  crates/
    sim-core/      # the deterministic heart: world state, movement, combat,
                   # building, economy rules. No I/O, no clock, no threads,
                   # no std collections that iterate nondeterministically.
                   # Compiles native AND wasm32 — the second target is the
                   # DETERMINISM GATE, not a web build: `test_parity_wasm`
                   # diffs its state hashes against native byte for byte,
                   # which is wall 1's enforcement and is worth the same
                   # with no browser in existence. The only crate the game
                   # rules live in.
    protocol/      # packet schemas, bit-level codec, quantization tables,
                   # golden tests. Built for both targets, same reason.
                   # Zero game logic.
    server/        # tokio + wtransport termination, session/auth, AOI +
                   # snapshot encoding, WAL persistence, admin, bots.
    client-core/   # the client netcode core (snapshot view, prediction/
                   # reconciliation, interpolation, client clock) — pure,
                   # NATIVE, and shared with the server's bot client. It
                   # was `client-wasm` and carried a raw C-ABI bridge for
                   # the browser; both went with the web client (operator,
                   # 2026-08-08). The desktop client links it as an rlib
                   # and compiles no wasm.
    client/        # the Bevy desktop client: the only client.
                   # SECOND CLASS as of DECISIONS.md 2026-08-05: the demo
                   # and the playable link, not the product. Allowed to
                   # sit below ART.md's bar; points at unarmed shards.
                   # A native renderer is NOT scheduled by that call.
  launcher/        # (M4, not a crate yet) the platform's desktop client:
                   # Rust + egui, one static binary, no webview — patcher,
                   # shard list, balances, self-custody wallet on alloy.
                   # Shares `protocol`, imports no sim code, and is built
                   # for the cascade, not for Gates alone.
                   # DECISIONS.md 2026-08-04.
```

**Thread model** — the picture the whole server hangs on:

```
 tokio net tasks (N)          sim thread (1, pinned)        storage thread (1)
┌─────────────────┐  SPSC   ┌──────────────────────┐  SPSC ┌────────────────┐
│ WebTransport    │ ring →  │ fixed 30 Hz tick:    │ ring →│ WAL append +   │
│ accept/read     │ (input) │  drain inputs        │ (evt) │ fsync, 5-min   │
│ per-connection  │         │  sim step (sim-core) │       │ snapshots,     │
│                 │  ← SPSC │  AOI + priority      │       │ ledger export  │
│ write datagrams │ ring    │  delta-encode into   │       └────────────────┘
│ + streams       │ (snap)  │  preallocated bufs   │
└─────────────────┘         └──────────────────────┘
```

- The sim thread **never** touches a socket, a file, a lock, or the wall
  clock. It drains bounded lock-free rings, steps the world, writes
  snapshots into preallocated per-client buffers, and publishes them to
  outbound rings. Everything else is someone else's thread.
- Rings are bounded SPSC (`rtrb` or equivalent), one inbound and one
  outbound per connection, preallocated at accept. Full inbound ring →
  oldest input dropped (the client re-sends; §5.4). Full outbound ring →
  that client skips a snapshot (they're not draining; their problem, not
  the tick's).
- **Shared movement, exactly shared**: the client predicts by calling the
  same `sim-core` — literally the same rlib, linked into the client
  process, no bridge and no re-implementation. Kept
  bit-identical across native/wasm by restricting sim-core float ops to
  `+ - * / sqrt min max clamp` (all IEEE-exact both targets), banning libm
  transcendentals (yaw → direction goes through a shared lookup table
  indexed by the quantized yaw byte), and never letting FMA contraction in
  (default Rust behavior). A CI test drives 10,000 random input sequences
  through both builds and asserts byte-equal output (§12). **Client and
  server are now both native, so the wasm half is no longer a shipping
  target — it is the instrument.** A second architecture that must agree
  bit for bit is what catches a float path quietly going
  platform-dependent, and it keeps its whole value with no browser in
  existence.

## 5 · Netcode — the spine

### 5.1 · Transport: why QUIC, and what rides where

WebTransport gives us the two things WebSocket can't: **unreliable
datagrams** (state that's stale the moment a newer one exists must be
allowed to die in the network, not queue behind a lost packet) and
**independent streams** (a chunk download never head-of-line-blocks a chat
line). One connection, three lanes:

> **Both reasons were written for a browser and both survive it unchanged** —
> they are properties of the traffic, not of the client. What the browser cut
> *does* change is that WebTransport is no longer forced: a native client
> could speak raw QUIC. It does not, and the cost of keeping WebTransport is
> approximately zero (`wtransport` wraps quinn and both ends already speak
> it), while the benefit is that a browser client remains possible later
> without a server rewrite. **Nobody has re-litigated this** and no decision
> is recorded either way; it is noted here so the next reader knows the
> choice is inherited rather than re-argued. `NETCODE.md` §2 owns the config
> and beats this section.

| lane | reliability | carries |
|---|---|---|
| **datagrams** | unreliable, unordered | C→S input frames · S→C delta snapshots (the two hot flows; both fit ≤ 1100 B, §5.5) |
| **uni streams** S→C | reliable, per-purpose | keyframe/baseline snapshots on AOI-enter and gap-recovery, world metadata at join, entitlement/catalog data |
| **one bidi stream** | reliable, ordered | join/auth handshake, transactions (craft, build-place, bank, purchase — anything that must not be lost or reordered), chat, admin |

Browser reality **(decision, revised after research)**: **all four majors**
— Safari 26.4 (2026-03) shipped WebTransport with datagrams, making it
Baseline (~88% global). The honest cut is iOS below 26.4, which has
nothing; the WebSocket fallback lane (reliable-only, fatter interp buffer,
an honest "degraded" badge) stays a **(knob: later)**, not skeleton. Dev
certs use `serverCertificateHashes` (ECDSA P-256, <14-day validity,
Firefox ≥ 125) so localhost works without a CA; production terminates on
its own UDP port with an ordinary ACME cert on the game's own subdomain —
the game never sits behind elo's nginx. The deep transport spec, config
of record, and all netcode detail live in `NETCODE.md`, which supersedes
this section where they differ.

### 5.2 · Time

30 Hz fixed sim tick (33.3 ms). Server tick number is the only clock the
game knows. Clients sync an offset estimate (EWMA over ping samples on the
bidi stream) and timestamp inputs in **client ticks**; the server maintains
a per-client adaptive jitter buffer (target: inputs arrive 1–2 ticks before
they're needed; buffer depth floats with measured jitter).

### 5.3 · Rates and budgets (the numbers table)

| thing | number | why |
|---|---|---|
| sim tick | 30 Hz | survival pacing; Rust ships 10–30 |
| snapshot send | 15 Hz per client (every 2nd tick), class-weighted | halves bandwidth; interp hides it |
| input send | one datagram per client render frame, coalesced ≥ 30 Hz | inputs are tiny; redundancy below |
| datagram budget | ≤ 1100 B payload | safe under QUIC's ~1200 B initial MTU, no fragmentation |
| downstream target | ≤ 20 kB/s per client typical, 30 kB/s cap | 15 Hz × ≤ 1100 B + streams |
| upstream target | ≤ 4 kB/s per client | inputs + occasional transactions |
| interp delay | 2 × snapshot interval + measured jitter (≈ 133–200 ms) | standard interpolation window |
| lag-comp rewind cap | 250 ms | generous to real latency, stingy to abusers |
| shard cap | 100 players **(knob)**, hard accept-cap at boot | preallocation needs a number |

### 5.4 · Inputs

An input frame is `(seq u16, client_tick, buttons bitfield, yaw u16, pitch
u8, move vec 2×i8)` — a few bytes. Every input datagram carries the **last
three frames** (current + two previous), so one lost datagram costs nothing
and two lost cost one frame. The server dedupes by seq, executes in seq
order, drops late-beyond-buffer inputs, and acknowledges the latest executed
seq inside every snapshot header (that ack drives reconciliation, §5.6).
Rate clamp: > 90 input frames/s sustained → warn, then disconnect (§10).

### 5.5 · Snapshots: baseline + delta, priority-filled

Per client, per snapshot tick:

1. **Interest set** from the AOI grid — uniform 64 m cells, subscribe
   radius ~176 m entering / ~208 m leaving (hysteresis so edge-dancers
   don't flap), plus global-class entities (time of day).
2. **Priority accumulator** per (client, entity): every tick each entity
   accrues `class_weight × f(distance)`; players near you accrue fastest,
   a far-off door barely. Sending an entity zeroes its accumulator.
3. **Fill the datagram**: highest accumulated priority first, delta-encoded
   against that client's **last-acked baseline**, until the 1100 B budget
   is spent. What doesn't fit stays accumulated and wins soon — eventual
   consistency with a per-class staleness ceiling (a visible player may
   never go > 4 snapshots unsent; if it would, it preempts).
4. Client acks snapshot tick numbers inside input datagrams (piggybacked);
   the server keeps a short ring of sent-state per client to delta against
   whatever tick the client last confirmed. Ack gap > ring depth (lost
   burst, tab-out) → the server pushes a fresh **keyframe** on a uni stream
   and deltas resume from it. No "please resend datagram" — this path *is*
   the recovery.

Encoding is a hand-rolled bit writer in `protocol` (this is the fun part
and the perf part: field presence masks, per-archetype schemas). Position
quantized chunk-relative to 3 cm in x/z, 1 cm in y (u16s + chunk id); yaw
u16, pitch u8; health u8 in 0.5 steps. Golden tests pin every packet's
bytes (§12) so the wire can't drift by accident.

### 5.6 · Prediction and reconciliation (local player)

Client simulates its own capsule immediately by calling `sim-core`
directly — same code, same constants, zero drift by construction (§4). Each snapshot
carries `last_executed_seq`; the client rewinds its predicted state to the
server's authoritative state at that seq and **replays every unacked
input** on top. Mispredictions (a door closed server-side first) correct
in one replay; the residual error smooths over ~100 ms (exponential) so
corrections read as a nudge, not a teleport. Everything *else* on screen is
interpolation (§5.7) — remote players are never predicted at v1
(extrapolation cap 100 ms on a gap, then they freeze honestly).

### 5.7 · Interpolation (everything else)

Remote entities render at `server_time − interp_delay`, buffered between
the two straddling snapshots, linear (hermite on velocity later if it ever
reads as needed). The delay is the price of smoothness and it's paid
knowingly; lag compensation gives it back in combat:

### 5.8 · Lag compensation (hits happen where you saw them)

The sim keeps a ring of the last 8 ticks of **collider state only**
(`sim_core::rewind`, preallocated). **Built 2026-08-29/30.** Two paragraph
claims here were wrong from the first draft and are corrected rather than
deleted, because both were load-bearing on how somebody would build it.

**A command does not carry a timestamp** — nothing latency-shaped is on the
wire, and `world.rs` says it never will be, because a rewind depth a client
can ask for is a rewind depth a client can forge. The server *mints* it
from a number the client was already sending: `snapshot_ack`, which says
which world the shooter was looking at. `favour = min((T − S) + 3, 7)` ticks
(`server/src/stats.rs::favour_for`; `NETCODE.md` §8 has the derivation).
`combat::strike` and `ranged::hitscan` resolve their target scan against the
rewound pose; the arrow in flight deliberately does not.

**The clamp is not logged per player**, and the ambition was wrong rather
than unbuilt: a per-client table for a diagnostic is a structure invented
for a counter. What surfaces in the anomaly log (§10) is `favour_disagree` —
a client claiming a staler view than the server watched it ack, which is the
shape abuse actually takes once staleness buys something.

### 5.9 · Join flow

Bidi stream: `hello{proto_ver, ver, build}` → **two** version gates →
`session{guest_uuid}` or wallet bind (§10) → server streams world metadata
(seed, tick, time, your spawn, catalog hash) on a uni stream while the sim
preallocates the connection's rings and slots → first keyframe → datagrams
flow. A shard at cap refuses at `hello` with a posted reason — never a hang.

Two gates because they are two questions (`crates/protocol/src/version.rs`
owns the table): `proto_ver` is the **exact** wire gate and a mismatch is
`REFUSE_VERSION`; `ver` is the client's **release**, checked against the
shard's `min_client` as a **minimum** — below it is `REFUSE_BUILD`, and a
client *newer* than the shard is admitted on purpose. `build` is a digest of
the client's build id, carried for the shard's records and gated by nothing.

## 6 · Hot-path laws

The blip budget is **zero**, and each law names its enforcement — a law
without a gate is a mood.

| # | law | enforcement |
|---|---|---|
| L1 | The sim thread makes **no syscalls** (no sockets, files, sleeps; monotonic clock read once per tick at the boundary) | code review wall + the soak test's tick-jitter assert (§12) |
| L2 | **Zero heap allocation in the tick** after warmup — pools, arenas, `ArrayVec`, preallocated buffers everywhere | counting `GlobalAlloc` wrapper; CI test runs 100 bots × 300 ticks and asserts alloc count delta == 0 (§12) |
| L3 | **No locks in the sim thread** — inter-thread traffic is bounded lock-free rings only | `clippy` disallowed-types (`Mutex`, `RwLock`, channels) in `server::sim` + `sim-core` |
| L4 | **Bounded everything** — rings, queues, entity caps, per-tick transaction budget (N transactions/tick, rest carry to next tick), per-client input rate. Every bound has a stated overflow policy (drop-oldest / defer / refuse) and none of them is "wait" | caps live in one `limits.rs`; review wall: no `Vec::push` on a client-driven path without a cap check |
| L5 | **No `String`, `format!`, or logging in the tick.** Diagnostics are integer event codes into a preallocated ring, drained and stringified by the storage thread | `clippy` disallowed-methods in the two sim crates |
| L6 | **Tick budget 33.3 ms; p99 sim step < 8 ms** at cap on reference hardware (4-core VPS). Over-budget trips the **degradation ladder, in order, automatically, logged**: ① shrink AOI radius 15% ② snapshot rate 15 → 10 Hz ③ refuse new joins ④ *never* stretch the tick | soak test asserts p99 + that induced overload walks the ladder and recovers |
| L7 | **Crash contract**: sim panic = fast abort + supervisor restart; state = last snapshot + WAL replay; ≤ 5 s of world regression, econ/build events ≤ 1 tick (WAL'd before ack). Restart target < 10 s | kill-9 test in CI: murder mid-tick, assert clean recovery + ledger intact |
| L8 | **The client keeps its own GC quiet**: no allocations in the render/net loop after warmup — pooled vectors/quaternions, preallocated typed-array rings for decode, no closures in the RAF path, UI in plain DOM outside the loop | perf harness page asserts zero minor-GC growth over 60 s idle-in-world (Chrome perf API) |

## 7 · Determinism and replay

**Contract**: same binary + same seed + same input log (every input of
every player, tick-stamped, plus join/leave/transaction events — which is
exactly the WAL) → the same world state, hash for hash.

Rules that make it true, all enforced in `sim-core`:

- No wall clock, no I/O, no thread nondeterminism (single sim thread).
- All randomness from one seeded PCG hierarchy (world seed → per-system
  streams); no `HashMap`/`HashSet` **iteration** anywhere in sim (clippy
  disallowed-types; state lives in slotmaps/dense vecs with explicit
  ordering).
- All mutations flow through a command buffer applied in a fixed order.
- Float ops restricted as §4 (also what keeps the wasm parity gate green).

`state_hash` (xxh3 over canonical entity state) computed every 32 ticks,
logged, and stamped into the WAL. A `replay` binary re-simulates a recorded
WAL and must reproduce every stamped hash. This is also the bridge back to
the elo board: a played round's `(seed, WAL, final hash)` is
machine-checkable by recomputation — the one honest auto-verify shape — so
a later quest can gate on a real played round with no human in the loop.

**None of the WAL paragraph above is built** (2026-08-07), and it was being
read as though it were: there is no `wal.rs`, no `replay` binary, and
`test_replay` runs the sim twice **in memory** off the same command stream
and compares hashes. That is real determinism and it is what wall 5 actually
gates today — it is not a recorded fixture and it saves nobody. Two things
followed from the confusion and are worth keeping in mind here: `CLAUDE.md`
and `AGENTS.md` both listed the `replay` command for months (removed), and a
whole design pass went into an admission ceremony for a game that handed you
a new character on every connection. §8's first two bullets are design, not
description. What IS built is the player half — one record per player, filed
under an opaque key, in `crates/server/src/store.rs` — and it is a different
artifact from the WAL with a different job: it answers "who was I", never
"what happened".

## 8 · Persistence and wipes

**Built today: the player, not the world.** `crates/server/src/store.rs` keeps
one fixed record per player — position, inventory, hp, the survival clock, the
craft queue — in a fixed-slot file, filed under the opaque `PlayerKey` the
admission seam returns, and restores it into the sim as `Command::JoinAs` so a
replay reproduces the restore. Exact at a clean leave, ≤ `MAX_PLAYERS` ticks
stale otherwise (a one-slot-per-tick autosave sweep). Structures, deployables,
container contents and hearths are **not** in it and still die with the
process, so a base outlives a disconnect and not a restart. The rest of this
section is the design for the world half and the ledger, and none of it is
built:

- **WAL**: every econ/build/inventory transaction and all inputs, appended
  by the storage thread (group-fsync ~50 ms cadence; transactions ack to
  clients only after WAL append — L7's guarantee).
- **Snapshots**: every 5 min the sim publishes a compact world copy into a
  double buffer (bounded copy cost, measured in the tick budget); the
  storage thread serializes and rotates. Restart = snapshot + WAL tail.
- **Wipe**: archive the final WAL + hash chain (the shard's provable
  history), export the banked-JUNK ledger as a merkle root for the claim
  rail, post the root, roll the seed, clean world. Blueprints per the
  cadence knob; skins are wallet-side and untouched.

## 9 · Client

**The native Rust + Bevy client is the only client** (operator, 2026-08-05;
`RENDER.md` owns its path). `web/` is **deleted** — not retiring, not
compiling, not gated (operator, 2026-08-06; the `client-wasm` crate that
still implied otherwise went 2026-08-08). **Every budget below was
nonetheless chosen for the browser**, and they are not yet re-derived for a
desktop binary on a real GPU, so a native measurement that exceeds one is
evidence about the budget, not automatically a defect. Re-deriving them is
`NOW.md` §0u.

- **Structure**: one process. `sim-core` is called directly — no wasm
  bridge, no worker, no transferable buffers — and `ClientCore` owns
  prediction and interpolation. Bevy draws and does not decide
  (`RENDER.md` §1).
- **World**: terrain chunks generated client-side from the seed by the same
  worldgen `sim-core` uses — by direct call, an rlib away — so terrain costs
  zero bandwidth and is identical by the float discipline; server remains
  collision-authoritative.
  Buildables/nodes/barrels render as instance pools per archetype — a base
  is hundreds of instances, not hundreds of draw calls.
- **Budgets, browser-era (knob: all four)**: 60 fps on a mid laptop iGPU;
  < 300 draw calls; < 1.5 M tris in view; initial load < 15 MB.
  - The **frame target survives the move** — it is a hardware floor, not a
    platform one.
  - The **load budget does not.** It was a first-visit *download*, paid over
    the network before anything drew. A desktop client installs a depot
    once (`ci/depot.py`) and pays disk on later boots, so the constraint it
    encoded no longer exists. `ART.md` §7's 12 MB texture payload is the
    same number wearing a different hat and retires with it.
  - The **draw-call and triangle ceilings are WebGL-shaped** and are the
    two that most need re-deriving: a native wgpu client with Bevy's
    automatic batching is not bound where a WebGL context was. Nothing has
    measured the native ceiling yet, so **no replacement number is written
    here** — proposing one goes to `DECISIONS.md` §open, not into this
    table. The first real pressure on it is already recorded: a full
    328-tree scatter ring at 5.9 k tris a conifer is 1.9 M
    (`crates/client/tests/tree.rs`), which is over 1.5 M and may or may not
    be over what the hardware minds.
- **Feel order** (the skeleton's client acceptance): input latency ≤ 1
  frame to predicted response; corrections invisible at ≤ 150 ms RTT with
  5% loss (the test harness's netem profile). Unchanged by the move.

## 10 · Security and fairness, v1 honest

- **The server is the only truth.** The client sends inputs and view
  angles; everything else — position, hits, inventory, crafting, building,
  economy — is server-computed. There is no client-claimed state to trust.
- Server-side sanity on every action: interact range + LOS checks, speed
  caps enforced by the sim itself (movement *is* server-simulated;
  prediction is cosmetic), fire-rate from item stats not packet cadence.
- **Session and identity**: the game is bought, so a session on an
  official shard starts from an entitlement rather than from nothing
  (`DECISIONS.md` 2026-08-05); *what* the purchase gates is §open. The
  `session{guest_uuid}` path in §9's handshake is unchanged and is what
  unarmed community, training, and demo shards run on. Binding a
  wallet = signing one EIP-191 message (`gates join <shard> <nonce>`) —
  same pattern as every elo game action; the wallet then owns banked JUNK,
  skins, and the character slot. One live session per wallet. The desktop
  launcher carries a self-custody wallet that signs this same message and
  nothing new (`DECISIONS.md` 2026-08-04): encrypted keystore on the
  player's own box, phrase shown once and confirmed back, connect-existing
  kept first-class, and the operator holding no keys stated to the player
  rather than only in pillar 4.
- Flood control at the edge: per-connection datagram/stream rate limits in
  the net tasks (the sim never sees a flood), input-rate clamp (§5.4),
  transaction budget (L4).
- **The armed set is the perimeter** (`DECISIONS.md` 2026-08-04). Economy
  arming is an operator-only act (§3, `ALPHA.md` §2), so protection scales
  with stake by construction: **official shards** are operator-run, armed,
  and JUNK-redeemable; **community and training shards** run unarmed and
  stay open to self-hosting and to agent players. An unarmed shard has
  nothing worth cheating for, which is what keeps agent play and any
  anti-cheat posture from competing — they are not on the same shard.
- **Honest gaps, v1**: no ESP/wallhack countermeasures beyond AOI (a snapshot
  you never receive can't be wallhacked — AOI is already the big one), no
  statistical anticheat, no replays-as-evidence UI. The anomaly log (lag-comp
  clamps, impossible-input counts, damage outliers) exists from day one so
  the data is there when those get built. The two gaps that survive an
  authoritative sim are **ESP inside AOI** and **aimbot**, and both have an
  answer that costs no client trust and needs no kernel hook: server-side
  occlusion culling (the genre's proven measure — and cheap here, because a
  seeded terrain bakes its occlusion grid once, `NOW.md` 18) and offline aim
  analysis over the WAL, which every round already produces as verifiable
  evidence. A kernel anti-cheat is **not integrated** and stays cut
  (`ALPHA.md` §5); it would need a native client to attach to, and it would
  ban the agent players the training goal depends on (`DECISIONS.md`
  2026-08-01).

## 11 · Milestones

> **This section owns the arc.** `ALPHA.md` §6 folds into it and `NOW.md` §7
> points at it; neither keeps a second copy. A milestone's *state* is derived,
> never stamped here — `./ci/gates.sh` and the tree answer that faster than a
> checkbox anyone has to remember to tick.

**M0 — the shell (the overnight). LANDED.** Exit met: two clients walk
around each other on a seeded island through a real server, and the laws
bite. The workspace is six crates rather than the five this line planned for
(`client-core` split out), the sim, wire, server, client and bot runner all
exist, and the gates are the ones `ci/gates.sh` runs — derive them from the
script, which is what CI executes. This item carried seven checkboxes,
**every one of them unticked while every one of them shipped**: dead state a
reader trusts, and the reason this section now states outcomes and points at
commands.
**M1 — survival verbs.** Gather (nodes/trees/barrels), inventory, craft
ladder T0–T1, build grid + hearth + upkeep/decay, death/drop. Exit: two
strangers can fight over a base with bows.
**M2 — combat true.** Lag-comp ring + rewound raycasts, ballistic
projectile step, T2 firearm + satchel, damage model by material tier.
Exit: the netem profile (150 ms / 5% loss) feels fair on both ends.
**M3 — JUNK.** Salvage → recycler and the BP research sink are **done**
(2026-08-10), which is the carried half of this milestone: the coin is
earned and spent in world, on shards nobody has to arm. What is left is
the claim rail and everything that touches it — carried/banked split, bank
terminal + fee burn, shard ledger + WAL settlement events, merkle export
at wipe, wipe machinery end-to-end on a test shard.
**M4 — the counter + the door.** Skin catalog, till verification
(three-valued), entitlements by wallet, wallet-bind flow, first public
shard, and the board's delivery: repo + playable link + a recorded round
whose replay hash checks. The playable link **was** the web demo — the job
second class was given (`DECISIONS.md` 2026-08-05). **That job no longer
exists**: the browser client is cut (`DECISIONS.md` 2026-08-06), and with it
the only artifact a board visitor could click and play. What replaces it is
`DECISIONS.md` §open ("the board's playable link") and is unanswered — the
candidates are the depot behind the elo launcher, a recorded round played
back off the WAL, or a board delivery of repo + replay hash with no link at
all. Until one is spoken this clause names a thing that is not built.

## 12 · CI gates (the walls)

| gate | asserts |
|---|---|
| `test_alloc_zero` | 100 bots × 300 ticks after warmup: heap alloc/free count delta == 0 (counting allocator) |
| `test_replay` | recorded 5-min fixture WAL → every stamped state_hash reproduced exactly |
| `test_parity_wasm` | 10k random input sequences through native + wasm movement: byte-identical states |
| `test_protocol_golden` | every packet type's encoding is byte-stable against checked-in fixtures |
| `test_snapshot_budget` | worst-case scene (cap players, dense base) per-client snapshot ≤ 1100 B and staleness ceilings hold |
| `test_crash_recovery` | **PLANNED, does not exist.** SIGKILL mid-tick → restart < 10 s, ledger/build state intact from WAL. What *is* built covers the restart half without the kill or the clock: `server/tests/world_persist.rs`, `sim-core/tests/{persist,worldsave}.rs` (a shard restart is a world you walk back into; a restore replays bit for bit) |
| `soak` (nightly) | **PLANNED, does not exist** — `nightly.yml` runs the gates, builds the release server and packages the depot, and has no soak job. 4 h, 200 bots: RSS slope ≈ 0, fd count flat, p99 sim < 8 ms, induced overload walks the L6 ladder and recovers |
| clippy walls | disallowed types/methods per L3/L5 + HashMap-iteration ban in `sim-core` |

A change that reddens a wall does not merge. The walls are the skeleton.

**Two rows above are plans, and they are marked so on purpose.** This table
is the design's list, not `ci/gates.sh`'s — derive the real one from the
script, which is what CI runs. The gap matters beyond this file: the missing
soak was cited as the enforcement of wall 3 in both `CLAUDE.md` and
`AGENTS.md`, and `test_raid_storm` — which also did not exist — as wall 4's.
The soak is still missing and wall 3 still holds on clippy alone.
**Wall 4's half landed 2026-08-14**: `crates/sim-core/tests/raid_storm.rs`
drives 64 synthetic players through build/lock/plant/guess/move/loot at the
tick's full command ceiling and asserts every store's cap per tick — it fills
the charge store to 64 of 64, pins the removal budget at 64 of 64 and
overflows the event ring on 90 of 400 ticks, so the caps are held *under
pressure* rather than observed at rest.

⚠ **`NETCODE.md` §11 names a different gate by the same name and it is still
absent.** That one is the *wire* storm — 20 subscribers, coalescing caps,
tick p99, byte counts — and none of that is in the sim-core gate, which
speaks to no socket and times nothing. Two gates, one name; check which
before citing either.

## 13 · How it ships through elo

Separate repo, separate brand, orbiting the town: the operator's own agent
claims `munus-first-sale` on the Great Work board, builds here, and puts
the delivery on the record (`POST /munus/{id}/claim` → build → `POST
/munus/{id}/submit` — both wallet-signed; the board's record is part of the
deliverable). Coins integrate as designed in §3; code never crosses either
way; elo docs are cited like any external source. Settlement for the quest
is elo's standing rule: the operator's eye and a public ELO transfer.

## 14 · Open knobs (each needs the operator's word, none blocks M0–M2)

| knob | default until spoken |
|---|---|
| shard cap / reference hardware | 100 players / 4-core VPS |
| game price + currency | unset — an operator act; no number invented here |
| what the purchase gates | the native client + the official armed shards; unarmed self-hosted shards stay free |
| ~~desktop client renderer~~ | **RESOLVED, not a knob.** Read "three.js stays for the web demo; a native renderer is unscheduled" (`DECISIONS.md` 2026-08-05). Both halves are gone: the native Bevy client shipped and the browser client was cut 2026-08-06. `RENDER.md` owns the path |
| kernel anti-cheat on armed shards | not integrated (`ALPHA.md` §5) |
| wipe cadence + BP survival | **weekly** map (operator, 2026-08-10), BPs survive one cycle |
| hunger/thirst depth | minimal timer-drain v1 |
| bank deposit fee | 2%, burns |
| JUNK allotment + claim cadence | unset — elo-side operator act |
| skin catalog, prices, ELO/ORBS split | unpriced until posted |
| skin proceeds: fiscus vs burn split | 100% fiscus, 0% burn |
| queue priority for sale | never (flipping it is a sentence) |
| WebSocket fallback lane (for iOS < 26.4 — Safari 26.4+ has WebTransport) | not in alpha |
| game domain + hosting box | its own subdomain + UDP port, own cert, never behind elo's nginx |
