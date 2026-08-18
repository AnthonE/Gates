# Lag compensation — implementation plan, measured against the tree

Design note, 2026-08-18, by a read-only agent. Read `NETCODE.md` §8 first; this
is what a builder executes instead of re-deriving it. Nothing here is built and
no cargo command was run.

⚠ **Independently re-checked before filing:** `client-core/src/interp.rs:15` is
`pub const INTERP_DELAY_TICKS: f64 = 4.0;` with `:13-14` stating the adaptive
widening is unbuilt — **this is the fact the whole "no wire bump" argument rests
on**; `InputDatagram.snapshot_ack: u16` exists at `protocol/src/lib.rs:2127`;
sleepers really do run through the tick loop (`world.rs:3111+`), so §8's
exclusion rationale is wrong about this tree; `grep -rn 'Command::Input'
crates/` returns **98**, exactly as claimed. The rest is the agent's, at its
stated provenance.

## 0 · What is actually true today

- `combat::strike` (`combat.rs:483`) resolves a swing against **present** server
  positions: `combat.rs:511-513` reads `t.body.qx/qy/qz` out of the live
  `[Player; MAX_PLAYERS]`.
- `ranged::draw` (`ranged.rs:231`) builds the arrow's direction from
  `p.frame.yaw` / `p.frame.pitch` at `ranged.rs:279-280` — the aim in the input
  frame the server is executing.
- `grep -rn rewind crates/` returns `protocol/src/bits.rs:105`
  (`BitWriter::rewind_to`) and `client-core/src/predict.rs` (client prediction's
  own rewind-and-replay). No rewind ring, no history, no favour.
- `reference/NETWORK.md:453` already recorded this: *"No rewind ring, no
  favour-the-shooter, nothing."*

### 0.1 · How stale a fight actually is, in this tree

The staleness is **RTT + interpolation delay**, and the interpolation half is
neither optional nor small:

| | |
|---|---|
| `INTERP_DELAY_TICKS` | `4.0` ticks = 133.3 ms — `client-core/src/interp.rs:15` |
| `TICK_HZ` | 30 — `limits.rs:15` |
| melee reach | 1 m (hatchet/rock class), 2 m (spear class) — `content/weapons.toml:47,94` |
| walk / sprint | 3.0 / 5.5 m/s — `movement.rs:31-32` |
| capsule radius | 0.4 m — `collide.rs:54` |

| RTT | total staleness | in ticks | target displacement, sprint |
|---|---|---|---|
| 20 ms (LAN) | 153 ms | 4.6 | 0.84 m |
| 50 ms | 183 ms | 5.5 | 1.01 m |
| 100 ms | 233 ms | 7.0 | 1.28 m |
| 220 ms | 353 ms | 10.6 | 1.94 m |

**On localhost, a sprinting target is already 0.84 m out of place against a 1 m
reach.** This is not a "bad connection" feature; the interpolation delay alone
(0.73 m of sprint) eats most of the shortest weapon in the game. That is the
whole justification and it is measurable before anything is built (slice 1).

## 1 · What has to be rewound, and what the ring costs

### 1.1 · What `strike` actually reads of a target

`combat.rs:507-524`, the target scan, reads exactly three things off the
victim's body: `qx`, `qy`, `qz` (dequantized against `POS_XZ_Q = 0.03` /
`POS_Y_Q = 0.01`, `movement.rs:22-24`). It reads no yaw, no velocity, no
`grounded`. The aim cone (`CONE_COS`, 30 degrees) and the facing are the
**attacker's** and stay present-tick.

So §8's "ring of collider transforms" overstates the shape: a player here has no
orientable collider. `strike` is a planar distance + cone test
(`combat.rs:514-520`) and `ranged` is an axis-aligned cylinder
(`ranged.rs:443-447`). There is no transform to store — only a position.

`Body` (`movement.rs:83-89`) is `4 x i32 + bool` = 17 B of payload, **20 B laid
out** at align 4. Storing whole bodies costs 25% more for `qvy` and `grounded`,
which nothing on this path reads.

### 1.2 · The record

```
RewindPose { id: u32, qx: i32, qy: i32, qz: i32 }   // exactly 16 B
```

`id` is not padding. World slots are reused (`server/src/client.rs:82` keeps a
`tracked_id` for precisely this reason, and `Command::Wake`'s doc says an id is
"minted per connection and meaningless across two of them"). A row whose `id`
disagrees with the present tenant must fall back to present, or a rewind
resurrects the position of somebody else's body.

### 1.3 · The cost, at §8's depth and at the honest depth

`MAX_PLAYERS = 100` (`limits.rs:7`).

| depth | span | bytes | KiB |
|---|---|---|---|
| **30 ticks (§8's number)** | 1000 ms | **48,000 B** | **46.88 KiB** |
| 16 ticks | 533 ms | 25,600 B | 25.00 KiB |
| **8 ticks (recommended)** | 267 ms | **12,800 B** | **12.50 KiB** |

§8's "~= 48 kB" is exactly right in decimal kB and 2.3% optimistic in KiB.
**But 30 ticks contradicts §8's own clamp**: 250 ms of favouring is 7.5 ticks at
30 Hz, so 750 ms of that ring is unreachable by construction. See §8-errata 4.

**Recommended: `REWIND_TICKS = 8`** — a power of two, so the index is
`tick & (REWIND_TICKS - 1)` and not a modulo (wall 1 likes integers; wall 3
likes no division). Eight rows hold the seven the clamp can reach plus the row
being written. Plus a `[u64; 8]` of per-row tick stamps (64 B) so a cold or
wrapped row is detectable rather than assumed. **Total 12,864 B.**

### 1.4 · Where it lives, and the cap

**Where:** a new `crates/sim-core/src/rewind.rs`, held as
`pub rewind: Box<Rewind>` on `World` (`world.rs:1223`), built with
`crate::boxed_array` (`lib.rs:59`). Boxed for the stated reason `mobs` and
`backpacks` are (`world.rs:1278-1292`, `:1303-1308`): `World` is 434 kB and is
built on the stack by `ShardCore::new`, every wire test and `probe_parity`.
`.cargo/config.toml` states a 4 MiB wasm shadow stack, so 12.8 kB is not a risk
— but the boxing posture is the one the repo has paid for three times in one day
and it is not worth re-litigating for a new store.

**Wall 2 (zero allocation in the tick after warmup):** one `boxed_array` at
construction, nothing in `tick`. The per-tick write is
`MAX_PLAYERS x 16 B = 1,600 B` of field stores into a preallocated row —
sub-microsecond against the 0.8 ms tick measured in `NETCODE.md` §9. Gate:
`test_alloc_zero` is unchanged and must stay at zero.

**Wall 4 (a cap in `limits.rs` with a stated overflow policy):**

```rust
/// Rewind ring depth in sim ticks — 8 rows, 267 ms of body history
/// (NETCODE.md §8). A power of two so the index is a mask, never a modulo.
/// Overflow policy: **overwrite oldest**. It is a ring by construction and
/// the oldest row is older than `REWIND_MAX_TICKS` can ask for, so nothing
/// reachable is ever discarded. Cost: MAX_PLAYERS * REWIND_TICKS * 16 B
/// = 12,800 B, preallocated.
pub const REWIND_TICKS: usize = 8;

/// Favouring clamp, in ticks (NETCODE.md §8's 250 ms, Overwatch's bound).
/// 250 ms is 7.5 ticks at TICK_HZ and a rewind is an integer number of
/// ticks, so this is **7 (233.3 ms)** — under the doc's promise rather
/// than over it, because that number is a promise to the *victim*.
/// Overflow policy: **clamp**, twice — once where the value is minted
/// (server/src/core.rs) and once where it is applied (world.rs), the
/// second being the one that survives a forged or replayed command.
pub const REWIND_MAX_TICKS: u8 = 7;

/// The interpolation delay both ends agree on (client-core/src/interp.rs
/// re-expresses this as its f64). Moved here because the SERVER needs it
/// to know how stale a client's aim is, and a hand-kept mirror of another
/// crate's constant is exactly the drift CLAUDE.md's `props.js` line is
/// about — so it is one number with one home, not two.
pub const INTERP_DELAY_TICKS: u8 = 4;

/// Correction for the age of the newest snapshot a client had applied when
/// it made an input. Snapshots land every SNAPSHOT_INTERVAL_TICKS (2), so
/// the expected age of the newest one is half of that. Derived, not picked.
pub const REWIND_ACK_BIAS_TICKS: u8 = 1;
```

All four are **proposed defaults and belong in `DECISIONS.md` §open** ("lag
compensation v0") before they land in code — knobs are spoken, never invented.

⚠ `limits.rs` changes never land from two branches in one merge window
(`CLAUDE.md`, loop discipline). Check the window before starting slice 2.

### 1.5 · Sleepers

§8 excludes sleepers because "they don't move". **In this tree they do** — see
§8-errata 1. And the exclusion buys nothing anyway: the ring is a fixed
`[[RewindPose; MAX_PLAYERS]; REWIND_TICKS]` array, so skipping a sleeper saves
zero bytes and adds a branch to the hot write. **Record every active slot.** The
write is `if !p.active { row[i] = ZERO; continue; }` and nothing more.

## 2 · Where the rewind time comes from — and the wire does not have to move

### 2.1 · What the wire carries today

| field | where | says |
|---|---|---|
| `InputDatagram.snapshot_ack` (u16) | `protocol/src/lib.rs:2127` | the newest snapshot tick the client **applied** |
| `InputDatagram.ack_bits` (u32) | `protocol/src/lib.rs:2129` | 32 more applied ticks |
| `InputDatagram.first_client_tick` (u32) | `protocol/src/lib.rs:2138` | **"Nothing on the server reads it"** (`:2131`) |
| `SnapshotHeader{tick, baseline_age, last_executed_seq, nudge}` | `protocol/src/lib.rs:2289` | no latency echo, no interp report |
| `Nudge` | `protocol/src/lib.rs:2269` | the dilation feedback, not a measurement |

No latency field. No interp-delay field. `PROTO_VER = 48`
(`protocol/src/lib.rs:645`).

### 2.2 · The measurement that needs no new field

**Do not reconstruct Source's three terms. Measure the thing they add up to.**

An input datagram arriving with `snapshot_ack = S` is a statement by the client:
*"the newest world I had applied when I made these frames is server tick S."*
Its remotes were drawn `INTERP_DELAY_TICKS` behind that. So the world the
shooter was looking at is server tick `S - INTERP_DELAY_TICKS`, and if that
frame is executed at server tick `T`:

```
favour_ticks = (T - S) + INTERP_DELAY_TICKS - REWIND_ACK_BIAS_TICKS
```

`T` and `S` are both **server tick numbers**. There is no clock in it, no
client-reported latency, no client-reported interp delay. `(T - S)` folds RTT,
the input-buffer depth, and the client's own scheduling into one
honestly-measured number — which is *better* than the three-term formula,
because it measures the quantity the formula is trying to estimate instead of
estimating its parts.

The server already has both halves at one call site: `ShardCore::push_input`
(`server/src/core.rs:698`) holds `dg.snapshot_ack` and the frame tail together,
and hands the frames to `ClientNetState::push_frame` (`server/src/client.rs:457`).
Stamp each buffered frame with the ack it arrived under — a parallel
`in_view: [u16; INPUT_BUFFER_CAP]` beside `in_frames` (`client.rs:104`) — and
`consume_input` (`client.rs:522`) returns it with the frame.

Two properties worth writing down:

- **Duplicates keep the freshest evidence.** `push_frame` drops a frame it has
  already seen (`client.rs:462-465`), so the stamp is the one from the datagram
  that arrived *first*. A frame that only ever arrives inside a retransmit tail
  gets a stamp that is too new, which measures *less* staleness — under-favouring
  the shooter on exactly the inputs that were already lost. That is the safe
  direction.
- **The client cannot overstate.** `on_acks` (`client.rs:351`) only credits acks
  against snapshots the server actually sent, from its own ring.

### 2.3 · So: does a new field exist, and what does the bump cost?

**No new field is needed and `PROTO_VER` stays 48 for every slice in this plan.**
Say it plainly, because it is the design's biggest simplification and it turns
on a fact about the tree rather than a cleverness: **the "ACTUAL interp delay"
§8 asks the server to track does not vary.** `client-core/src/interp.rs:15` is
`pub const INTERP_DELAY_TICKS: f64 = 4.0;` and `:13-14` says the adaptive
widening to 200 ms "rides the M2 feel pass with the loss telemetry". Both ends
already share one compile-time constant. Move it to `limits.rs` (§1.4) and there
is nothing to report.

**The bump is priced for the day that stops being true.** When adaptive interp
lands, the client must report the delay it is actually using:

- `InputDatagram` gains 4 bits (`interp_delay_ticks`, 3..=18 => 100-600 ms).
  Layout change => wall 6.
- `PROTO_VER` 48 -> 49 in `protocol/src/lib.rs:645`.
- `encode_input` / `decode_input` (`protocol/src/lib.rs:2181` / `:2222`) gain the
  field, with a range refusal on decode mirroring the encode refusal — `sel`'s
  posture, not `buttons`'.
- **All 96 fixtures in `crates/protocol/tests/golden/` regenerated in the same
  commit**: `cargo run -p protocol --example gen_goldens`.
- The field becomes a *client claim*, so it inherits §6's verification burden —
  clamp it and cross-check it, never trust it.
- Loop discipline: `protocol` never lands from two branches in one merge window.

**None of that is owed by this plan.** Build the whole feature at `PROTO_VER 48`
and let the bump arrive with the feature that needs it.

## 3 · Wall 1 — how the rewind is expressed

`sim-core` is pure: no clock, no `HashMap` iteration, no libm/trig, floats
restricted to `+ - * / sqrt min max clamp floor-by-cast`, and `test_parity_wasm`
diffs native against wasm byte for byte.

**The rewind is an integer number of ticks and nothing else.**

- The offset is a `u8` in `0..=REWIND_MAX_TICKS`. There is no millisecond
  anywhere in `sim-core`.
- The ring index is `(tick & (REWIND_TICKS - 1)) as usize` — a mask, no division,
  no float.
- The lookup is an array index into a fixed array, in slot order. No map, no
  iteration order to disagree about.
- The rewound values are the **same quanta the live body holds** (`i32` of
  `POS_XZ_Q`/`POS_Y_Q`), so the dequantize at `combat.rs:511-513` is
  byte-identical arithmetic on byte-identical inputs — the quantize-both-sides
  law applied to history. Native and wasm cannot diverge because nothing new is
  computed; a stored `i32` is substituted for a live one.
- `World::tick` is the only clock (`limits.rs:15`: *"The tick number is the only
  clock"*), and `server_now` is `self.tick` (`world.rs:3050`).

The clock-reading half of §8's formula lives entirely in `crates/server/`, which
may read `Instant` (`server/src/net.rs:30` already does) — and even there it does
not have to (§2.2).

**Where the write goes:** at the end of `World::tick`, immediately before
`self.tick += 1` (`world.rs:3484`). Row `tick & 7` then holds end-of-tick poses
for tick `tick`, and during tick `T` a lookup of `k` ticks back reads row
`(T - k) & 7` with its stamp checked against `T - k`. `k == 0` must read the
**live** body, not the ring — see §4.3; it is what makes slice 3 a provable
no-op.

## 4 · Wall 5 — is a latency-dependent rewind a determinism violation?

**No. The latency is an input like any other command — but only if it is built
that way, and there are three conditions. This is the load-bearing question and
it deserves the full answer.**

### 4.1 · Why it is not a violation

Wall 5 is *same build + seed + WAL -> same state hashes*, and the WAL's
definition is in the tree, on `Command` (`world.rs`, the enum's own doc):
**"The WAL is exactly this stream plus the tick numbers (DESIGN.md §7)."**

Network conditions *already* determine the sim's output, comprehensively and by
design. Which tick an input lands on is a function of RTT, the dilation loop and
the input buffer (`server/src/client.rs:522` `consume_input` consumes 0, 1 or 2
frames depending on buffer depth). A dropped datagram makes the sim reuse the
last frame. None of that is a determinism problem, because determinism here is
*reproducibility from the recorded stream*, not independence from the network.
The stream records what the network produced.

A rewind depth is the same class of fact. It is not something the sim computes;
it is something the sim is *told*, exactly as it is told which button was
pressed.

### 4.2 · The three conditions

**(a) It must be computed outside `sim-core`.** The measurement lives in
`server/src/client.rs` (the stamp) and `server/src/core.rs:781` (the mint, where
`Command::Input { id, frame }` is built). `sim-core` never sees a snapshot ack,
an RTT or a clock. This is not decoration — it is what keeps wall 1 and wall 5
from arguing.

**(b) It must ride the command.** `Command::Input` widens to
`{ id, frame, favour: u8 }` (`world.rs:997`). Anything else is a side channel,
and `worldsave.rs`'s header states the rule from the other end: *"a mutation
that arrived through a side channel would replay as something else."*
`Command::AdminTeleport`'s doc makes the same argument for the same reason: *"a
body that moved by side channel would replay as a body that never moved, and
every hash after it would differ."*

**(c) The sim must re-clamp on receipt.** `favour.min(REWIND_MAX_TICKS)` in the
apply arm at `world.rs:2590` — sitting beside the two clamps that are *already
there* for exactly this reason: `frame.sel` is forced to 0 for a non-wire
command, and `frame.buttons &= BTN_MASK` because *"a non-wire command is masked
instead"*. A WAL, a bot and a test are all non-wire command sources; a favour of
200 from any of them must not index past the ring.

With (a)-(c), a replay of the recorded stream reproduces the recorded hashes
exactly, on any box, on any network, forever.

### 4.3 · The file that would have to record it, and what `persist.rs` already writes

**The file is `crates/sim-core/src/world.rs`** — the `Command` enum *is* the WAL
schema. There is no WAL file yet: `tests/replay.rs:1-5` says so in its own header
— *"Until the server's WAL exists this drives a deterministic in-memory command
script — the contract is identical, the fixture just isn't a file yet."* So the
cost today is one `u8` on a variant, and when the file lands the field is
serialized with everything else. There is no schema migration to pay.

**`crates/sim-core/src/persist.rs` is not that file and must not become it.** It
writes `PlayerSave` — the per-body record, `PLAYER_SAVE_BYTES = SCALARS_BYTES +
CRAFT_QUEUE*4 + INV_SLOTS*6` (`persist.rs:65`): body quanta, meters, heal,
counters, blueprint mask, craft queue, inventory. It carries **no netcode state
of any kind** — no seq, no ack, no latency — and its header says why: *"It is
what is saved, never who it is saved for."* A favour value is a property of one
tick's input, not of a body, and adding it there would bump `SAVE_FORMAT` in
`server/src/store.rs` and regenerate the golden for nothing.

**Two further decisions this question forces, both of which must be written into
the module header:**

- **The ring is not hashed.** It is derived from state that is already hashed, on
  the precedent of `Pieces::cols` (`worldsave.rs`: *"Derived, never hashed"*) and
  the event ring (`world.rs:3492`: *"The event ring is derived output and stays
  out"*). Two shards agreeing on every hash from tick 0 have identical rings by
  construction.
- **The ring is not saved**, on `worldsave.rs:53-56`'s arrows-in-flight
  precedent: *"Sub-second state whose whole meaning is a trajectory between two
  ticks."* But the ring is **not reconstructible at load**, where `cols` is — and
  that is the one place this could quietly break wall 5. The rule that closes it:
  `pose_at` falls back to the present body whenever the row's stamp is not the
  tick asked for, so a world loaded at tick `N` resolves every strike at present
  until tick `N+8`. That is deterministic given the *origin*, which is exactly
  the sentence `worldsave.rs` already widened wall 5 to: *"same build + same
  **origin** + same command stream -> same state hashes."* Say it in `rewind.rs`'s
  header or somebody will "fix" the fallback.

## 5 · Melee, arrow, or both

**Melee only. Recommended scope: `combat::strike` and nothing else.**

### 5.1 · What §8 says about projectiles, and whether it holds

> *"Ballistic projectiles are sim entities and need no rewind at all; they hit
> when and where the sim says."*

Checked against `ranged.rs`, this is **right about the mechanism it names and
wrong as a claim that the arrow has no lag-comp defect**:

- **Right:** the flight is a real sim entity — integer mm and mm/tick
  (`ranged.rs:11-22`), stepped after the whole player loop *"so a shot resolves
  against final positions"* (`world.rs:3388-3395`), resolving against present
  bodies each tick (`ranged.rs:418-452`). Rewinding the *flight* would be
  actively wrong: an arrow with a 45-tick life (`item.bow`, 60 m at 40 m/s) would
  be resolved against a world 7 ticks stale for its entire 1.5 s trip. §8's
  instinct is correct and the ring must not touch `ranged::step`.
- **Wrong:** the arrow's *launch* is not compensated. `ranged.rs:279-280` builds
  the direction from `p.frame.yaw`/`p.frame.pitch` — the shooter's aim, taken
  against a world 5-7 ticks old. At 30 m, a target strafing at walk speed was
  0.70 m from where the shooter aimed, against a `CAPSULE_RADIUS_M` of 0.4 m
  (`collide.rs:54`). That is the difference between a hit and a miss.

So the arrow *does* have a lag-comp defect, and the ring is not its fix. The two
candidate fixes are (a) accept it — projectile lead is genre-normal and Overwatch
does not rewind projectiles either — or (b) spawn-time catch-up: run
`k = favour` integration substeps at launch. **Recommend (a) at v0**, with the
measurement recorded, because (b) changes where an arrow collides and would need
its own gate, and because the flight-time lead already dwarfs it (0.75 s of
flight at 30 m against 0.23 s of network staleness).

### 5.2 · Why melee is the right scope

- It is where the error is largest **relative to the mechanic**: 0.84-1.28 m of
  displacement against a 1 m reach. The arrow's error is 0.70 m against a 60 m
  range.
- It is the smallest diff: one function (`combat.rs:483`), one call site
  (`world.rs:3251`), one existing test fixture (`tests/combat.rs:87`
  `place_in_front`).
- It already has parity coverage to extend: `probe_combat` (`probe.rs:701`)
  drives three-bot brawls through `test_parity_wasm`.

### 5.3 · Explicitly out of scope, recorded so nobody re-derives it

`ranged` (per §5.1); `mob::strike` (`world.rs:3266` — an animal's aim at a player
is equally stale, but animals are server-driven and have no client to favour);
`charge`'s blast (a fuse resolves on its own tick, `world.rs:3295-3305`);
headshots and hitboxes (no head exists — `combat.rs:36`); line-of-sight (there is
none anywhere, `reference/NETWORK.md:443`, so the rewind cannot create a
through-wall hit that is not already reachable at 1-2 m).

## 6 · The abuse surface

### 6.1 · What clamps the 250 ms

`REWIND_MAX_TICKS = 7` (233.3 ms), applied **three** times, deliberately:

1. **At the mint** (`server/src/core.rs:781`) — the policy.
2. **At the apply** (`world.rs:2590`, beside the existing `sel`/`buttons`
   clamps) — the one that survives a forged, replayed or bot-minted command.
3. **Structurally**, by the ring's own depth: with 8 rows there is no data older
   than 267 ms, so even a bug cannot rewind further. A cap you cannot exceed
   because the memory does not exist is worth more than a cap you check.

### 6.2 · What the server verifies rather than accepts

**The client sends no favour field at all.** That is the single biggest security
property of this design and it is free (§2.3). There is no new forgery surface:
`accept_input` (`server/src/net.rs:1390`) is unchanged and `input_dg_forged`
keeps its exact meaning.

The one thing the client controls is `snapshot_ack`, and the server derives
everything from it:

- **Overstating is impossible.** `on_acks` (`server/src/client.rs:351`) credits
  an ack only against a snapshot in its own sent ring.
- **Understating is possible and self-priced.** A client that acks stale
  snapshots buys up to 7 ticks of favour — and pays for it in its own delta
  compression, because `baseline()` (`client.rs:380`) picks the delta baseline
  from the newest ack. Understate far enough and the ring runs out and every
  snapshot is zero-state. The attack costs the attacker bandwidth to buy at most
  233 ms, i.e. one extra body-width of melee reach against a sprinting target.
- **The independent check.** `server/src/net.rs` has a real clock (`:30`,
  `Instant`). A wall-clock RTT estimate on the I/O thread can be compared against
  the ack-derived staleness; a persistent disagreement past a 2-tick band is the
  signature of a lying ack. That gets an `anomaly.rs` `WATCHED` entry
  (`server/src/anomaly.rs:168`) — call it `favour_disagree` — which is
  `DESIGN.md` §5.8's *"the clamp is logged per player and outliers surface in the
  anomaly log"* finally built rather than claimed. Use the **smaller** of the two
  estimates.
- **The rewind moves the victim, never the attacker.** The attacker's position,
  yaw, reach and cone all stay present-tick (`combat.rs:500-503`). A shooter
  cannot rewind themselves into range.
- **Eligibility and damage stay present-tick.** `t.active` / `t.hp == 0`
  (`combat.rs:508`) and the hp mutation (`combat.rs:536-537`) are the live
  values. You may hit where someone *was*; you may not damage someone who is
  already dead, and you cannot resurrect a corpse's hitbox.

### 6.3 · What is not defended, said out loud

A determined client running a deliberately stale ack gets up to 233 ms of favour,
bounded, priced in its own bandwidth, and visible in a counter. That is a
strictly better position than `DESIGN.md` §5.8's design, in which *"a fire
command carries the client's interp timestamp"* — a forgeable number the server
would have to talk itself out of trusting.

## 7 · Staging

Each slice lands green on its own. Cheapest first.

### Slice 1 — *"the shard can say how stale an aim is"* · server only · **worth landing alone**

Touches: `server/src/client.rs`, `server/src/core.rs`, `server/src/stats.rs`,
`server/src/status.rs`.

- `ClientNetState` gains `in_view: [u16; INPUT_BUFFER_CAP]` beside `in_frames`
  (`client.rs:104`); `push_frame` takes the ack; `push_input` (`core.rs:698`)
  passes `dg.snapshot_ack`; `consume_input` (`client.rs:522`) returns
  `(InputFrame, u16)`.
- `ShardCore::tick` computes
  `stale = (T - S) + INTERP_DELAY_TICKS - REWIND_ACK_BIAS_TICKS` and folds it
  into `ShardStats` as `aim_stale_samples` / `aim_stale_sum` / `aim_stale_max`,
  published on `/status.json`.
- **Gate:** new `crates/server/tests/lagcomp_measure.rs` — synthetic datagrams
  whose acks are N ticks old measure exactly `N + 3`; a client that never acks
  measures the clamp and not a wraparound.
- **Earns:** the number the entire design's constants are derived from, before
  any of them is chosen. No sim change, no hash change, no wire change,
  `PROTO_VER 48`.

**This is the slice worth landing alone.** It is the only one that can be checked
against a real link (`cargo run -p server --bin bots -- 100`, then read
`/status.json`) and it turns `REWIND_MAX_TICKS` from a doc's number into a
measurement. If the measured staleness on a real link is not what §0.1 predicts,
everything below changes and nothing has been wasted.

### Slice 2 — the ring · `sim-core`, no reader

Touches: `limits.rs` (the four constants), new `sim-core/src/rewind.rs`,
`world.rs` (the field and the write at `:3484`), `client-core/src/interp.rs:15`
(re-expressed off `limits`).

- **Gates:** new `crates/sim-core/tests/rewind.rs` — the ring holds where a body
  stood 7 ticks ago; a cold ring falls back to present; a reused slot falls back
  to present; the depth is `REWIND_TICKS`. `test_alloc_zero` unchanged.
  `test_parity_wasm` unchanged (nothing reads it). Re-measure
  `size_of::<World>()` against `.cargo/config.toml`'s 434 kB note and update it
  in the same commit.
- ⚠ `limits.rs` window: one owner.

### Slice 3 — `Command::Input` carries `favour` · mechanical, no behaviour

- `Command::Input { id, frame, favour: u8 }` (`world.rs:997`); the apply arm
  clamps it (`world.rs:2590`) into a **tick-local** `[u8; MAX_PLAYERS]` threaded
  through `apply` beside `removals` — the `removals` precedent verbatim
  (`world.rs:3045`: *"Deliberately not a `World` field... a store that lives
  across ticks is one `state_hash` has to answer for"*). Not on `Player`, so
  `state_hash` does not move and `persist.rs` does not change.
- **98 construction sites across 26 files** all become `favour: 0`
  (`grep -rn "Command::Input" crates/`). Mechanical, reviewable, and it puts the
  value at every call site, which is this repo's posture.
- **Gate — and this is the slice's whole proof:** `test_replay`'s pinned hash and
  every `probe_*` digest **must not move**. A field nothing reads changes no
  state. If any hash moves, the threading is wrong.
- *Alternative rejected:* a second `Command::InputAt` variant would cut the diff
  to three files and put two spellings of one command in the WAL. Do not.

### Slice 4 — `strike` rewinds

- `combat::strike` takes `&Rewind` and `favour: u8`; the target scan
  (`combat.rs:511-513`) reads `pose_at(j, favour, &players[j])` instead of the
  live body; `favour == 0` returns the live body, so the pre-slice behaviour is
  preserved bit-for-bit. `range_cm` (`combat.rs:531`) is measured on the rewound
  geometry — it is what the death screen reports and what the shooter saw.
- **Gates:** new cases in `tests/combat.rs` off the existing
  `place_in_front`/`swing_once` fixture — a target that walked out of a 1 m reach
  over 7 ticks is **hit at favour 7 and missed at favour 0**, both asserted,
  which is the only shape that proves the feature rather than the plumbing.
  `probe_combat` (`probe.rs:701`) drives a nonzero favour pattern so the path
  rides `test_parity_wasm` on both targets or neither.
- `test_replay`'s pinned hash and `probe_combat`'s digest **regenerate in this
  commit**, with the note saying it is behavioural and naming the case that moved
  it. That is wall 5 working.

### Slice 5 — the server sets it · turn it on

- `core.rs:781` mints `Command::Input { id, frame, favour: measured.min(REWIND_MAX_TICKS) }`
  from slice 1's stamp.
- The `favour_disagree` cross-check (§6.2) and its `anomaly.rs:168` `WATCHED` row.
- **Gates:** a server test that a client acking N-tick-old snapshots gets
  `min(N+3, 7)`; a disagreeing clock bumps the counter. `bot_smoke` and
  `client_loop` stay green.
- Land last, alone, after the operator can look at slice 1's numbers.

### Not in this plan

The arrow (§5.1), the "off above 220 ms RTT" rule (§8-errata 7), adaptive
interpolation delay and its `PROTO_VER` bump (§2.3), headshots, line-of-sight,
animals.

## 8 · `NETCODE.md` §8 errata — wrong about the tree, not merely unbuilt

`reference/NETWORK.md` §9.3 established that all seven of §11's "Added CI gates"
were unbuilt. These are a different and worse class: statements about how this
tree behaves that do not survive a read of it.

1. **"Sleepers, buildings, and settled items test at present time — they don't
   move; there is nothing to rewind."** Sleepers move. `world.rs:3111-3139` runs
   `movement::step` on every sleeping body each tick with a zeroed frame,
   precisely so *"a body that stopped falling when its owner disconnected would
   hang in the air over a base that decayed out from under it."* Benign for the
   hit test, but the exclusion it justifies is a non-optimization in a
   fixed-capacity ring (§1.5). **Drop the exclusion; record every active slot.**
2. **"Ballistic projectiles are sim entities and need no rewind at all."** Right
   about the flight, wrong as a claim about the arrow's correctness: the launch
   direction is a stale aim (`ranged.rs:279-280`). See §5.1. The sentence should
   keep its first clause and gain the second.
3. **"the server tracks each client's current interpolation delay (it varies per
   §3)."** It does not vary. `client-core/src/interp.rs:15` is a `const` 4.0 and
   `:13-14` says the adaptive widening is unbuilt. §8 asks the server to track a
   compile-time constant that both ends already share — which is the single
   largest simplification available and the reason no wire bump is owed (§2.3).
4. **The section contradicts itself on depth.** 30 ticks of history under a
   250 ms clamp leaves 750 ms of the ring permanently unreachable. §9's budget row
   (`rewind ring | ... ~= 48 kB`) is therefore ~4x the honest cost (12.8 kB). One
   of the two numbers has to move; the clamp is the one with a citation behind it.
5. **§1 and §9 disagree with each other.** §1 (`NETCODE.md:52-53`) says *"combat
   rewind stays a few KB"*; §9 says 48 kB. Both describe the same ring.
6. **`DESIGN.md` §5.8 specifies a different mechanism, and it is the forgeable
   one.** It says *"a ring of the last 8 **snapshot**-ticks"* (= 16 sim ticks, not
   30) and **"a fire command carries the client's interp timestamp"** — the client
   supplies the rewind time. `NETCODE.md` beats `DESIGN.md` §5 (`CLAUDE.md`'s
   table) and should here, for the reason in §6.3. `DESIGN.md:341`'s 250 ms cap
   row survives either way; §5.8's mechanism sentence needs correcting to point at
   §8, and its *"the clamp is logged per player and outliers surface in the
   anomaly log"* is buildable and should be built (slice 5).
7. **"above ~220 ms RTT it turns off entirely" is not implementable as stated
   against this design.** RTT never enters our arithmetic — we measure total
   staleness, not RTT — and at 220 ms RTT the wanted rewind is 10.6 ticks against
   a 7-tick clamp, so that shooter is already under-favoured by 3.6 ticks before
   any disable rule fires. Record it as a `DECISIONS.md` §open knob, not as a
   design.
8. **"Ring of collider transforms."** There is no collider transform to store:
   `strike` is a planar distance + cone (`combat.rs:514-520`) and `ranged` is an
   axis-aligned cylinder (`ranged.rs:443-447`). Three integers per body, not a
   transform.
9. **§9's budget table has no row for the ring's write cost** (100 x 16 B per
   tick), in a table whose header claims to be the budget. Trivial in size; the
   omission is the point.

## 9 · Files a builder will touch

Read-first: `sim-core/src/combat.rs:483-553` · `sim-core/src/world.rs:3038-3488`
· `server/src/client.rs:344-560` · `protocol/src/lib.rs:2118-2300` ·
`client-core/src/interp.rs:1-45`.

Changed, by slice: `server/src/client.rs`, `server/src/core.rs`,
`server/src/stats.rs` (1) · `sim-core/src/limits.rs`, `sim-core/src/rewind.rs`
(new), `sim-core/src/world.rs`, `client-core/src/interp.rs` (2) ·
`sim-core/src/world.rs` + 26 files of `Command::Input` sites (3) ·
`sim-core/src/combat.rs`, `sim-core/src/probe.rs`, `sim-core/tests/combat.rs` (4)
· `server/src/core.rs`, `server/src/anomaly.rs` (5).

Docs owed: `DECISIONS.md` §open (four knobs, before the code) · `NETCODE.md`
§8/§9 (the nine errata) · `DESIGN.md` §5.8 (the mechanism sentence) · `NOW.md`
§0pvp item 5 (points here).
