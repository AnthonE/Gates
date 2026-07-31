# Gates — design of record, v0.1

> the name is `Gates` (spoken 2026-07-31, DECISIONS.md). Everything here is
> buildable as written; knobs the operator must still rule on are marked
> **(knob)** and collected in §14. Written 2026-07-30, the night before the
> shell. This file drops at the repo root and is the design of record until
> superseded.

A browser survival game in the Rust (Facepunch) tradition: wake with nothing
on a hostile island, gather, craft, build a base, raid, lose it all, wipe,
again. 100 players a shard, no install — a link. OBOL is the working coin
you earn and risk in the world; SCRY and MYRRH buy skins and nothing else.
Backend in Rust (the language), frontend three.js, transport QUIC via
WebTransport. **The v1 deliverable is the skeleton** — netcode, determinism,
and the server discipline laws — built so every later system (content,
monuments, vehicles, AI) is data and systems plugged into a frame that
already doesn't blip.

This is a **separate product in its own repo**. It orbits scry (sold through
the Great Work board as quest `munus-first-sale`, settled in SCRY, coins from
the scry economy) but imports none of its code and none of its rules except
the ones restated here. Tickers are bare: SCRY, OBOL, MYRRH — never a `$`.

---

## 1 · Pillars

1. **The skeleton is the product.** Multiplayer, determinism, and the
   hot-path laws come first and are gated by CI. Content is data. If the
   frame is right, the game grows for years; if it's wrong, no amount of
   content saves it.
2. **Browser-native, zero install.** three.js + WebTransport. The crypto
   audience lives in a browser next to a wallet; the distance from tweet to
   in-game must be one click.
3. **The server never blips.** Fixed tick, no blocking, no locks, no
   allocations in the hot path, bounded everything, and a *defined*
   degradation ladder instead of an emergent freeze.
4. **The economy is honest.** Coins are earned in play and risked in play.
   The house sells appearance only. Settlement happens on RH-Chain through
   the same claim pattern scry already uses; the game server holds no keys
   and mints nothing.
5. **The core is deterministic.** Same build + same seed + same input log →
   same world, hash-checkable. That buys drift-free prediction, one-file bug
   repros, and a played round the scry board can machine-verify.

## 2 · The game, v1 loop

The Rust loop, cut to what a skeleton must prove:

- **Spawn** naked on a beach of a seeded island (~2km × 2km, heightfield
  terrain, biome tint by noise). Day/night cycle. Hunger/thirst minimal —
  a slow health drain past a timer, food to reset it **(knob: depth)**.
- **Gather**: trees, stone/ore nodes (hit the glowing weak spot for bonus —
  the Rust juice), and **salvage barrels** along roads/shore (the loot
  tension source; they drop components and OBOL salvage, §3).
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
  monthly map / blueprints survive one extra cycle)**. New seed, fresh
  island. What survives a wipe is exactly: blueprints (per schedule),
  banked OBOL, and skins. Nothing else.

Out of scope for v1, by design (the skeleton must not wait on them):
vehicles, farming, animal AI, electricity, teams UI (informal groups work
day one), more monuments, anti-ESP occlusion culling (§10).

## 3 · The economy

Three coins, three jobs, one law each. Every pair already has a live pool
on RH-Chain, so **the pools are the exchange** — the game never posts a
rate between coins, never converts in-game, never runs its own market
between them. A player holding any coin is one swap from any other.

### 3.1 · OBOL — the working coin (the scrap)

OBOL plays the role scrap plays in Rust: the ground-truth currency of grind,
trade, and progression — earned by play, spent in play, lootable in play.

**Two states, and the whole design is the difference:**

| state | where it lives | can you lose it? |
|---|---|---|
| **carried** | an item stack in your inventory, like any loot | yes — dropped on death, lost on wipe, raidable in your base |
| **banked** | your wallet's row in the shard ledger, written only at the haven's bank terminal | no — survives death and wipes, claimable on-chain |

Carried OBOL is what makes the game a survival game: the hoard in your base
is a raid target, the run to the haven with 300 OBOL on you is the tensest
walk in the game. Banked OBOL is what makes the economy real: a file-ledger
balance keyed to your wallet, exported at wipe (and on a posted cadence) as
a merkle root for the scry claim rail — the same play-accrues-off-chain,
settles-by-claim shape the town already uses (`ONCHAIN-LINE.md`). The game
server holds no keys, mints nothing, and the on-chain supply it draws from
is an operator-funded allotment on the scry side **(knob: allotment size and
claim cadence — an operator act, not a game mechanic)**.

**Faucets** (all in-world): salvage from barrels; recycler at the haven
(feed it components → OBOL); a trickle from monument crates.
**Sinks** (all burn against the shard ledger): blueprint research (the main
sink, exactly scrap's job), recycler service fee, market-stall listing fee
at the haven, and the bank terminal's deposit fee **(knob: default 2%)**.
Banking costs a little so carrying stays rational; the fee burns so the
house never earns from the game loop.

**Player-to-player is free.** Players may trade anything for anything,
including OBOL for a rifle. That is players trading with players — the
survival economy working. The wall is only ever about the **house**: §3.3.

### 3.2 · SCRY and MYRRH — the skin counter

The haven's vendor sells **appearance only**: weapon and clothing skins,
building cosmetics, dyes. Standard catalog priced in SCRY; limited/seasonal
drops priced in MYRRH (the scarcer, capped coin — that scarcity IS the
seasonal story) **(knob: final split and prices — posted at ship, never
invented here)**.

Purchase flow, v1 — custody zero, verification three-valued:

1. The catalog posts each skin's price and the shard's **till address**.
2. The buyer's wallet sends the exact amount on RH-Chain (chain 4663).
3. The game backend verifies the transfer against the explorer the same way
   scry's board verifies receipts: match on recipient, token, exact string
   amount; `true` unlocks, `false` says which field mismatched, and
   *explorer unreachable* is `null`/retry — never treated as false, never
   silently unlocked. x402 is the later upgrade path, not a v1 dependency.
4. The entitlement keys to the **wallet**, not the character: skins survive
   death, wipes, and shards. Tradable skins (on-chain editions) are a later
   gate, not v1.

Proceeds go to a posted address **(knob: fiscus vs a burn split — an
operator sentence; default: 100% to the posted fiscus address, burn 0%)**.

### 3.3 · The never-table

What money — any coin, any amount, anyone's — can **never** buy from the
house. This table is a wall, not a knob, and it is the whole reason a
crypto-native audience can trust a survival game with real coins in it:

| never for sale | because |
|---|---|
| damage, armor, speed, capacity, gather rate | pay-to-win kills the game |
| upkeep, decay pauses, protection windows | paying rent to not be raided is pay-to-win with extra steps |
| blueprints, tech, crafting speed | progression is play |
| loot odds, spawn quality, map intel | information is position |
| a better queue **(knob: default never — flipping this is an operator sentence)** | access is the last honest thing to sell and we default to not selling it |

The house sells appearance. Players sell each other everything. The pools
exchange the coins. That is the entire monetary constitution.

## 4 · Architecture

Cargo workspace, five crates, one law about where code may live:

```
gates/
  crates/
    sim-core/      # the deterministic heart: world state, movement, combat,
                   # building, economy rules. No I/O, no clock, no threads,
                   # no std collections that iterate nondeterministically.
                   # Compiles native (server) AND wasm32 (client prediction
                   # + shared worldgen). The only crate the game rules live in.
    protocol/      # packet schemas, bit-level codec, quantization tables,
                   # golden tests. Shared native/wasm. Zero game logic.
    server/        # tokio + wtransport termination, session/auth, AOI +
                   # snapshot encoding, WAL persistence, admin, bots.
    client-wasm/   # the client netcode core (snapshot view, prediction/
                   # reconciliation, interpolation, client clock) — pure,
                   # native-tested, shared with the server's bot client —
                   # plus a thin raw C-ABI wasm bridge to JS (no bindgen;
                   # the same pattern the parity probe ships).
  web/             # three.js app (vite): renderer, input, interpolation,
                   # UI overlay (plain DOM), wallet connect.
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
- **Shared movement, exactly shared**: the client predicts with the same
  `sim-core` machine code paths compiled to wasm — not a JS re-imp. Kept
  bit-identical across native/wasm by restricting sim-core float ops to
  `+ - * / sqrt min max clamp` (all IEEE-exact both targets), banning libm
  transcendentals (yaw → direction goes through a shared lookup table
  indexed by the quantized yaw byte), and never letting FMA contraction in
  (default Rust behavior). A CI test drives 10,000 random input sequences
  through both builds and asserts byte-equal output (§12).

## 5 · Netcode — the spine

### 5.1 · Transport: why QUIC, and what rides where

WebTransport gives the browser the two things WebSocket can't: **unreliable
datagrams** (state that's stale the moment a newer one exists must be
allowed to die in the network, not queue behind a lost packet) and
**independent streams** (a chunk download never head-of-line-blocks a chat
line). One connection, three lanes:

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
the game never sits behind scry's nginx. The deep transport spec, config
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

Client simulates its own capsule immediately through wasm `sim-core` —
same code, same constants, zero drift by construction (§4). Each snapshot
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

The sim keeps a ring of the last 8 snapshot-ticks of **collider state only**
(positions/stances — a few bytes × entities, preallocated). A fire command
carries the client's interp timestamp; the server clamps it (≤ 250 ms, ≤
connection RTT + slack), rewinds colliders to that time, raycasts, applies
damage at present time. Abuse margin is bounded by the clamp; the clamp is
logged per player and outliers surface in the anomaly log (§10).

### 5.9 · Join flow

Bidi stream: `hello{proto_ver}` → version gate → `session{guest_uuid}` or
wallet bind (§10) → server streams world metadata (seed, tick, time, your
spawn, catalog hash) on a uni stream while the sim preallocates the
connection's rings and slots → first keyframe → datagrams flow. A shard at
cap refuses at `hello` with a posted reason — never a hang.

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
- Float ops restricted as §4 (also what makes wasm prediction bit-exact).

`state_hash` (xxh3 over canonical entity state) computed every 32 ticks,
logged, and stamped into the WAL. `cargo run -p server --bin replay --
--wal <file>` re-simulates and must reproduce every stamped hash — CI runs
a recorded 5-minute fixture on every push (§12). This is also the bridge
back to the scry board: a played round's `(seed, WAL, final hash)` is
machine-checkable by recomputation — the one honest auto-verify shape — so
a later quest can gate on a real played round with no human in the loop.

## 8 · Persistence and wipes

- **WAL**: every econ/build/inventory transaction and all inputs, appended
  by the storage thread (group-fsync ~50 ms cadence; transactions ack to
  clients only after WAL append — L7's guarantee).
- **Snapshots**: every 5 min the sim publishes a compact world copy into a
  double buffer (bounded copy cost, measured in the tick budget); the
  storage thread serializes and rotates. Restart = snapshot + WAL tail.
- **Wipe**: archive the final WAL + hash chain (the shard's provable
  history), export the banked-OBOL ledger as a merkle root for the claim
  rail, post the root, roll the seed, clean world. Blueprints per the
  cadence knob; skins are wallet-side and untouched.

## 9 · Client (three.js)

- **Structure**: net worker owns the WebTransport session (datagram decode
  off the main thread, transferable buffers over); main thread runs sim
  prediction (wasm), interpolation, three.js scene. COOP/COEP headers set
  from day one so SharedArrayBuffer is available when wanted.
- **World**: terrain chunks generated client-side from the seed through the
  same wasm worldgen `sim-core` uses (zero terrain bandwidth, identical by
  the float discipline); server remains collision-authoritative.
  Buildables/nodes/barrels render as `InstancedMesh` pools per archetype —
  a base is hundreds of instances, not hundreds of draw calls. GLTF +
  Draco assets, one atlas per era tier.
- **Budgets**: 60 fps on a mid laptop iGPU; < 300 draw calls; < 1.5 M
  tris in view; initial load < 15 MB **(knob: art will fight this —
  the budget wins)**.
- **Feel order** (the skeleton's client acceptance): input latency ≤ 1
  frame to predicted response; corrections invisible at ≤ 150 ms RTT with
  5% loss (the test harness's netem profile).

## 10 · Security and fairness, v1 honest

- **The server is the only truth.** The client sends inputs and view
  angles; everything else — position, hits, inventory, crafting, building,
  economy — is server-computed. There is no client-claimed state to trust.
- Server-side sanity on every action: interact range + LOS checks, speed
  caps enforced by the sim itself (movement *is* server-simulated;
  prediction is cosmetic), fire-rate from item stats not packet cadence.
- **Session and identity**: guest UUID sessions play instantly. Binding a
  wallet = signing one EIP-191 message (`gates join <shard> <nonce>`) —
  same pattern as every scry game action; the wallet then owns banked OBOL,
  skins, and the character slot. One live session per wallet.
- Flood control at the edge: per-connection datagram/stream rate limits in
  the net tasks (the sim never sees a flood), input-rate clamp (§5.4),
  transaction budget (L4).
- **Honest gaps, v1**: no ESP/wallhack countermeasures beyond AOI (a snapshot
  you never receive can't be wallhacked — AOI is already the big one), no
  statistical anticheat, no replays-as-evidence UI. The anomaly log (lag-comp
  clamps, impossible-input counts, damage outliers) exists from day one so
  the data is there when those get built.

## 11 · Milestones

**M0 — the shell (the overnight).** Exit: two browser tabs walk around each
other on a seeded island through a real server, and the laws already bite.
- [ ] workspace + five crates, CI runs fmt/clippy/test on push
- [ ] `sim-core`: tick loop, seeded worldgen (heightfield), kinematic
      capsule move/collide vs terrain, command buffer, state_hash
- [ ] `protocol`: bit codec, input + snapshot v0 schemas, golden tests
- [ ] `server`: wtransport accept, session hello, rings, 30 Hz sim thread,
      AOI v0 (radius only), baseline+delta snapshots, keyframe recovery
- [ ] `client-wasm` + `web`: connect, predict/reconcile own capsule,
      interpolate the other guy, three.js terrain from shared worldgen
- [ ] gates live: zero-alloc test, replay test, golden tests, wasm/native
      parity test, 50-bot smoke
- [ ] `bots` bin: N capsules random-walking for load
**M1 — survival verbs.** Gather (nodes/trees/barrels), inventory, craft
ladder T0–T1, build grid + hearth + upkeep/decay, death/drop. Exit: two
strangers can fight over a base with bows.
**M2 — combat true.** Lag-comp ring + rewound raycasts, ballistic
projectile step, T2 firearm + satchel, damage model by material tier.
Exit: the netem profile (150 ms / 5% loss) feels fair on both ends.
**M3 — OBOL.** Salvage → recycler → carried/banked split, bank terminal +
fee burn, BP research sink, shard ledger + WAL settlement events, merkle
export at wipe, wipe machinery end-to-end on a test shard.
**M4 — the counter + the door.** Skin catalog, till verification
(three-valued), entitlements by wallet, wallet-bind flow, first public
shard, and the board's delivery: repo + playable link + a recorded round
whose replay hash checks.

## 12 · CI gates (the walls)

| gate | asserts |
|---|---|
| `test_alloc_zero` | 100 bots × 300 ticks after warmup: heap alloc/free count delta == 0 (counting allocator) |
| `test_replay` | recorded 5-min fixture WAL → every stamped state_hash reproduced exactly |
| `test_parity_wasm` | 10k random input sequences through native + wasm movement: byte-identical states |
| `test_protocol_golden` | every packet type's encoding is byte-stable against checked-in fixtures |
| `test_snapshot_budget` | worst-case scene (cap players, dense base) per-client snapshot ≤ 1100 B and staleness ceilings hold |
| `test_crash_recovery` | SIGKILL mid-tick → restart < 10 s, ledger/build state intact from WAL |
| `soak` (nightly) | 4 h, 200 bots: RSS slope ≈ 0, fd count flat, p99 sim < 8 ms, induced overload walks the L6 ladder and recovers |
| clippy walls | disallowed types/methods per L3/L5 + HashMap-iteration ban in `sim-core` |

A change that reddens a wall does not merge. The walls are the skeleton.

## 13 · How it ships through scry

Separate repo, separate brand, orbiting the town: the operator's own agent
claims `munus-first-sale` on the Great Work board, builds here, and puts
the delivery on the record (`POST /munus/{id}/claim` → build → `POST
/munus/{id}/submit` — both wallet-signed; the board's record is part of the
deliverable). Coins integrate as designed in §3; code never crosses either
way; scry docs are cited like any external source. Settlement for the quest
is scry's standing rule: the operator's eye and a public SCRY transfer.

## 14 · Open knobs (each needs the operator's word, none blocks M0–M2)

| knob | default until spoken |
|---|---|
| shard cap / reference hardware | 100 players / 4-core VPS |
| wipe cadence + BP survival | monthly map, BPs survive one cycle |
| hunger/thirst depth | minimal timer-drain v1 |
| bank deposit fee | 2%, burns |
| OBOL allotment + claim cadence | unset — scry-side operator act |
| skin catalog, prices, SCRY/MYRRH split | unpriced until posted |
| skin proceeds: fiscus vs burn split | 100% fiscus, 0% burn |
| queue priority for sale | never (flipping it is a sentence) |
| WebSocket fallback lane (for iOS < 26.4 — Safari 26.4+ has WebTransport) | not in alpha |
| game domain + hosting box | its own subdomain + UDP port, own cert, never behind scry's nginx |
