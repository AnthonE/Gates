# Gates · NETCODE.md — the multiplayer, in full (v0.1)

> Extends `DESIGN.md` §5–7; where they disagree, this file wins. Grounded in
> the canon (Gaffer on Games, Source, Overwatch GDC, Quake 3) and in what
> Facepunch actually shipped for the same four mechanics — citations inline.
> Everything here is buildable as written; knobs are marked **(knob)**.
>
> **The client is native (Bevy) and the browser one is deleted** (operator,
> 2026-08-06). Almost nothing in this file turned on that — the replication
> classes, the budgets and the lag-comp arithmetic are properties of the
> traffic, not the client — so the edit was narrow and is confined to §2:
> four JS-API queue knobs that no longer have an API, the
> `serverCertificateHashes` rules, and three config *reasons* that cited
> Chrome. ⚠ **That edit over-corrected on the certificate row and §2.2 now
> says so**: the P-256 / 14-day rules were read as *Chrome's*, and therefore
> as gone with Chrome, when the pinned wtransport enforces them client-side
> in Rust — so the dev pin was buildable the whole time it was recorded as
> dead. Fixed 2026-08-10 with the validating client. **Where a
> browser-era mechanism left a behaviour with no native owner, §2.1 says so
> rather than quietly dropping it.**

## 0 · The one law this file adds

`DESIGN.md` v0.1 described one pipeline: delta snapshots on datagrams at
15 Hz. That is correct for things that move every tick and ruinous for a
survival world, because a survival world is almost entirely things that
**don't**: a 4,000-block base changes a few times an hour, a sleeper never
moves, a settled dropped item is furniture. The law:

**Every entity belongs to exactly one replication class, chosen by how its
state changes — continuous, discrete, or global — and each class has its own
pipeline. Nothing rides the snapshot path unless it moves.**

## 1 · The replication classes

| class | what's in it | changes | pipeline | steady-state cost |
|---|---|---|---|---|
| **D — dynamic** | awake players, projectiles in flight, items mid-physics | every tick | unreliable datagrams: delta snapshots vs acked baseline, priority-filled, 15 Hz | per-tick, bounded by datagram budget |
| **S — structural** | building blocks, doors, tool cupboards, storage, settled items, death backpacks, **sleepers** | discrete events (placed, damaged, opened, looted, slept) | reliable **chunk event streams** with version numbers (§5) | zero between events |
| **G — global** | time of day, wipe clock, shard notices | slow | a few bytes in every snapshot header + reliable notices | ~free |

Entities **transition**: a dropped item is D while it falls (≤ ~2 s), then a
`settle` event moves it to S with a resting transform. A player is D while
connected and becomes S the moment they disconnect (a sleeper is a player
entity that stopped emitting inputs — same id, same inventory, new pose).
A building block is born S and dies S; it never touches a datagram.

Why this is the whole ballgame: steady-state datagram traffic is *awake
players + projectiles + settling items* — a few dozen entities — so the
1100 B budget holds at 100 players with headroom. The megabase costs a
one-time stream on approach and rare events after. The priority accumulator
only ever ranks class D. And the sim's rewind ring for lag compensation
holds awake players only (§8), so combat rewind stays a few KB.

## 2 · Transport mapping (updated)

| lane | carries |
|---|---|
| **datagrams** (unreliable) | C→S input frames · S→C class-D delta snapshots + G header |
| **uni streams** S→C | join bundle (seed, tick, catalog hash) · **chunk state at version V** on subscribe · keyframes on ack-gap recovery |
| **one bidi stream** | handshake · transactions (place, craft, loot, bank, buy — §6) · **chunk event fan-out** S→C · chat · clock/dilation control |

QUIC gives every chunk stream and the transaction lane independence — a
megabase streaming in never head-of-line-blocks a door event or a chat
line, and a lost snapshot datagram is simply dead (never retransmitted;
the next one supersedes it).

⚠ **This section carried a browser-support matrix until 2026-08-15** — a
Baseline claim across four engines, a WebSocket fallback lane held open as a
knob, and a minimum Firefox version for the dev-cert flow. **All of it was
dead**: the browser client was cut 2026-08-06 and there is no second client.
§2.2 had already been swept for this (its keep-alive and migration rows both
say "retired with the browser" in as many words); this half had not, which is
the sweep-one-file-and-not-its-neighbour shape `CLAUDE.md` warns about.

**What is actually true, and it is a smaller claim than "we run
WebTransport."** `wtransport` is built on **quinn** and we enable its `quinn`
feature in both crates, so the QUIC underneath is ordinary QUIC and we
already reach past the wrapper for it — `QuicTransportConfig` and
`IpBindConfig` in `net.rs` are quinn's own types. What WebTransport still
adds is the **HTTP/3 session layer**: an extended-CONNECT handshake
(`endpoint.accept()` → `IncomingSession` → `request.accept()`), a
`https://{addr}` URL shape that `elo-shardlist-v1` bakes into every
published row, and a per-datagram session-id prefix against the 1 100-byte
budget.

**That layer now has no user, and it is not free.** The one remotely
triggerable panic this project has ever pinned around lives *in* it — two
bytes on the CONNECT stream (#317) — which is why we are on a git rev of an
unreleased third party rather than a published crate, and §2.2's own ⚠ says
nothing records or gates that the pin contains the fix. The self-signed cert
rules we enforce (P-256, 14-day validity) are WebTransport-spec rules written
for browsers.

**Not changed here, because it is a flag-day, not a refactor.** The handshake
is the thing that would change, so there is no version to negotiate — an old
client would simply fail to connect, and two platform depots plus a public
shard are live. `NOW.md` §0wt carries it, with the window: bundle it with the
next `min_client` floor raise, which is already a flag-day.

### 2.1 · The "real UDP" fine print — congestion control, measured

RFC 9221 is explicit: datagrams are exempt from **flow** control but never
from **congestion** control — the sender must delay or drop when the
controller says so, no opt-out. What that means for us, concretely:

- **quinn's pacer is a non-issue at our shape.** It only spreads bursts
  bigger than 10 datagrams (token bucket, 2 ms granularity); we send 1–3
  datagrams per client per tick. Pacing bites bulk-over-datagrams, which
  we never do — bulk rides streams.
- **Loss is the real risk**: quinn's default CUBIC halves cwnd on loss,
  and a collapsed window at 100 ms RTT can drop below a 30 Hz send rate.
  Three mitigations, all shipped here: stay far under the path's capacity
  (~300 kbit/s per client against multi-Mbit paths, Fiedler's fixed-rate
  doctrine); **always `send_datagram()` (drop-oldest), never
  `send_datagram_wait()`** — a CC stall must cost freshness, not latency;
  and the degradation ladder (DESIGN L6) sheds our rate before the
  controller has to. BBR (loss-insensitive) exists in quinn behind an
  "experimental" label — we ship CUBIC and A/B BBR behind a server flag.
  [rfc-editor.org/rfc/rfc9221 · quinn docs/source · netlab.tkk.fi
  RTP-over-QUIC study]
- **Datagram size**: 1,200 B is the guaranteed QUIC path floor; quinn's
  DPLPMTUD (on by default, ceiling 1452) can grow it server-side. Our
  design number stays **≤ 1,100 B payload**. **Clamp every send against
  the live `max_datagram_size()`** (server: `net.rs`). The *silent* form of
  this failure was the browser's — a JS write over `maxDatagramSize`
  resolved its promise and sent nothing — and that trap died with the
  browser client. The ceiling itself did not: it is a property of the path,
  wtransport reports it the same way, and an oversized write is still a
  write that does not arrive.
- **Queue tuning is now quinn's, not the JS API's.** This bullet used to
  specify `congestionControl: "low-latency"`, `outgoingMaxAge: 50` ms, a
  small `incomingHighWaterMark`, and `WebTransport.getStats()` for the
  HUD's loss/RTT — **four browser-API knobs on a client that is gone**.
  What is actually configured now lives one table down and is server-side
  (`datagram_send_buffer_size` 64 KiB, idle timeout, keep-alive). The two
  *behaviours* the JS knobs bought are worth keeping and **neither has a
  named native owner yet**: stale outbound inputs dying in the queue rather
  than arriving as a post-stall burst, and drop-oldest on inbound overflow.
  Ours is a bounded SPSC ring with an explicit overflow policy
  (`DESIGN.md` §6), which is the right shape — nobody has checked it
  against these two properties. Client-side loss/RTT telemetry has no
  native source wired at all.

### 2.2 · Transport config of record (server)

| knob | value | why |
|---|---|---|
| wtransport version | **git-pin ≥ commit `0f7609a`** (or 0.7.2 once cut). ⚠ The tree pins `rev = a11e6a8e…` (`server/Cargo.toml:26`, `client/Cargo.toml:185`), resolving to `0.7.1` from git. **Nothing in the repo records whether `a11e6a8` descends from `0f7609a`, and nothing gates it** — write the ancestry down next to the pin when this seam is next touched | 0.7.1 has a remotely triggerable panic — two bytes on the CONNECT stream kill a worker (#317); fixed on main 2026-07-25, unreleased as of this writing |
| congestion control | CUBIC, and **selectable since 2026-08-15**: `shard.toml` `cc = "cubic" \| "bbr"` (`config::Congestion` → `QuicTransportConfig::congestion_controller_factory`). An unknown value is a **boot failure**, not a fallback — an A/B that silently runs CUBIC while the operator reads the result as BBR's is worse than no A/B. This row claimed a `--cc` flag from the day it was written and nothing implemented it until then, so "measure before trusting" had never once been runnable | BBR is labeled experimental in quinn; measure before trusting. §2.1 names the reason to care: CUBIC halves cwnd on loss and a collapsed window at 100 ms RTT can fall below a 30 Hz send rate. `net_congestion_events` is the reading that decides it — **and nobody has run the A/B yet** |
| datagram send buffer | 64 KiB (down from quinn's 1 MiB default) | bounds worst-case queued staleness at our rate; snapshots replace, never accumulate |
| MTU | initial/min 1200, DPLPMTUD on, ceiling 1452 (defaults) | free server→client headroom; design number stays 1,100 |
| idle timeout / keep-alive | 30 s / **server-side keep-alive 10 s** | both shipped in `net.rs`. The old reason — "the JS API cannot send keep-alives" — is retired with the browser; quinn can keep-alive from either end. Server-side stays because the effective idle timeout is the min of both peers, so the end that must not time out is the one that should send |
| UDP socket | SO_RCVBUF/SNDBUF **asked** at 8 MiB via `with_bind_socket` (`net::bind_udp`, `UDP_BUF_BYTES`), **and read back**, since 2026-08-15. The readback is the row's real content: `setsockopt` does not fail when the kernel refuses — it clamps to `net.core.rmem_max` and returns success — so `ShardStats` carries `net_rcvbuf_asked` beside `net_rcvbuf_bytes` (and the send pair) and the two contradict each other out loud when the sysctl is low. **The sysctl half is still ops and still owed**; what changed is that nothing now *believes* it was granted. Measured 2026-08-15 on the dev container: `rmem_max` 4 MiB, so the 8 MiB request is clamped and the pair says so. The 2026-08-11 box that found this row unbuilt was at the distro default 212992 (~208 KiB) — the exact number the why column calls too small. Gated by `net.rs`'s `the_socket_buffer_records_what_it_got_not_what_it_asked`, which asserts the **pair** and deliberately not the size: a wall that only passes on a tuned box teaches people to skip it | quinn is one socket for all connections and its README warns the OS defaults (~208 KiB) are too small |
| admission | quinn defaults **plus a policy, since 2026-08-15**: past `ADMIT_RETRY_AT` (2× `MAX_PLAYERS`) in-flight handshakes an *unvalidated* address is answered with a QUIC Retry; past `ADMIT_REFUSE_AT` (4×) the attempt is refused before any crypto. Counted as `admit_retried` / `admit_refused`. Until then every refusal we had — `refused_version|build|auth|ticket|full` — fired **after** a completed handshake (QUIC, TLS, CONNECT, SIWE, and an entitlement round trip), so a full shard was an amplifier. **The question this row asked is answered: the wrapper does not hide the hook.** `IncomingSession` *is* `quinn::Incoming` and re-exports `retry()`, `refuse()`, `ignore()` and `remote_address_validated()` — so admission was never a reason to drop WebTransport (§0wt keeps its other reasons). ⚠ `retry()` **panics** if the address is already validated, which is why the call site is a guard and not an optimisation | rides QUIC's built-in 3× anti-amplification + retry tokens, which are in the protocol and were always ours; what was missing was the *policy* on top. Deliberately **not** the polite refusal — `REFUSE_FULL` carries a reason to a client that completed a handshake, which is the right answer to a full shard and the wrong one to a flood |
| certs, dev | `Identity::self_signed`; the server computes the SHA-256 and `shard` prints it; the client pins it with `--cert-hash` | **The printed hash stopped being vestigial on 2026-08-10.** This row used to say the P-256 / 14-day / short-validity rules "were Chrome's" and that "nothing enforces them now" — **wrong on the facts**: the pinned wtransport enforces all three CLIENT-side in `ServerHashVerification` (`SELF_MAX_VALIDITY = 14 days`, `OID_EC_P256`, current time inside the window), and `Identity::self_signed` already builds exactly such a certificate. So the dev path needed no new machinery: `--cert-hash` feeds the printed digest to `with_server_certificate_hashes` and that shard, alone, is trusted. Note the pin does **not** consult a name, which is why a dev shard's SANs (`localhost`, `127.0.0.1`, `::1`) do not have to follow it onto a LAN address |
| certs, prod | ordinary ACME cert on the game's own subdomain, no hashes | unchanged, and load-bearing. **The client validates as of 2026-08-10** (operator; `DECISIONS.md` that date): the platform root store for every non-loopback address with no pin, `with_server_certificate_hashes` when `--cert-hash` names one, and `with_no_cert_validation` on loopback only. The warning that used to sit here — that publishing was blocked on this — is discharged. Why it mattered more than a checklist item: **SIWE has no channel binding**, so an on-path relay that terminates the player's QUIC and opens its own to the shard is admitted *as the victim* with the key never leaving the wallet. Gated by `crates/server/tests/tls_posture.rs`, which raises a self-signed shard on a NON-loopback address and asserts the refusal, the pin, a wrong pin, and the carve-out |
| migration | server tolerates NAT rebinding (default on); **client treats network change as death** | the behaviour is unchanged; the reason is not. It read "Chrome ships no client-side QUIC migration", and there is no Chrome — quinn *can* migrate. Fast-reconnect stays an app feature (§6.3, §10) because it has to handle the cases migration cannot (a dead server, a new address that must re-prove SIWE), not because the client is incapable |

## 3 · Class D — the hot pipeline

Refinements over `DESIGN.md` §5, with the canon's shipped parameters:

- **Acks, redundantly** — Gaffer's header verbatim: every input datagram
  carries `snapshot_ack: u16` (newest snapshot tick received) plus
  `ack_bits: u32` (bit n ⇒ tick `ack − n` also received), so every ack is
  repeated ~32×; ack loss is a non-event. The server deltas each client
  against the **newest acked baseline** in its ring of the last 32 sent
  states (~2 s at 15 Hz). [gafferongames.com/post/reliable_ordered_messages]
- **Resync is not a special path** — the Quake 3 trick: if a client's ack
  falls outside the ring (loss burst, tab-out, join), the baseline becomes
  the **canonical zero-state** and the same delta encoder produces an
  absolute snapshot; the priority accumulator streams the world back in
  over the next few datagrams, nearest first. No keyframe machinery, no
  second code path. [fabiensanglard.net/quake3/network.php]
- **Inputs carry their unacked history** — all inputs the server hasn't
  confirmed, capped at 10 frames (≈ 333 ms of loss cover — Rocket League's
  shipped number; Overwatch does the same "everything since your last
  acked frame"). One lost datagram costs nothing; three in a row still
  cost nothing.
- **Quantize both sides.** The server's own sim runs on the quantized
  values it transmits (position 3 cm x/z · 1 cm y chunk-relative, yaw u16,
  pitch u8, velocity ±81.92 m/s at 1 cm/s — wide enough for the spoken terminal
  50 m/s — with a 1-bit **at-rest flag** that elides it; widths pinned by
  `test_protocol_golden`, registered in DECISIONS §open), so client and
  server never disagree by a rounding error —
  Gaffer's rule, and load-bearing for our bit-identical wasm prediction.
  Players have no free orientation (yaw/pitch only) and projectiles face
  their velocity, so no quaternion ever rides a datagram; full transforms
  exist only in class-S events, where the stream carries them uncompressed.
  [gafferongames.com/post/snapshot_compression]
- **Priority accumulator** (per client × class-D entity), v0 weights:
  `accum += w · 1/(1 + d/32 m)` per tick; players w=100, projectiles w=80,
  settling items w=30. Send order = accumulator desc; sent → reset to 0;
  didn't fit → keeps accruing (Gaffer's exact scheme, from the 901-cube
  demo that hit a 256 kbps budget). Staleness ceilings preempt: a visible
  player may never exceed 4 unsent snapshots (266 ms), a projectile 2.
  A snapshot that won't fit **sheds entities, never fragments** — at 1%
  loss, a 10-fragment packet is effectively 9.5% loss.
  [gafferongames.com/post/state_synchronization ·
  gafferongames.com/post/packet_fragmentation_and_reassembly]
- **Interpolation, with velocity.** Remote entities render 133 ms in the
  past by default — 2 × the 66.7 ms snapshot interval, Source's shipped
  `cl_interp` rule, tolerating exactly one lost snapshot — widening
  adaptively to 200 ms on lossy links (Gaffer's tolerate-two rule) and
  floored at 100 ms. Interpolation is **Hermite using the snapshot's
  velocity**, not linear: at ≤ 15 Hz linear visibly pulses; Hermite at the
  same rate shows no artifacts. Both straddling snapshots missing →
  extrapolate linearly for at most **250 ms** (Source's bound), then
  freeze honestly. [developer.valvesoftware.com/wiki/
  Source_Multiplayer_Networking · gafferongames.com/post/snapshot_interpolation]
- **Correction smoothing** on reconciliation error, Gaffer's blend:
  exponential 0.95/frame for errors ≤ 25 cm, 0.85 for ≥ 1 m, hard snap
  beyond a few meters. Corrections read as a nudge, never a teleport.

## 4 · Time — the client runs ahead, and the server steers it

The model, named plainly (it's Overwatch's command-frame scheme): **the
client's sim clock runs ahead of the server's by RTT/2 + one buffered
tick**, so each input arrives just before the tick that consumes it. The
server holds a per-client input buffer with a target depth of 1–2 ticks
and never waits: an empty buffer means it reuses the previous input and
the client will hear about it.

Two feedback loops keep the buffer at target — both shipped mechanisms:

1. **Time dilation** (Overwatch): every snapshot header carries a 2-bit
   nudge — `ok / faster / slower / hard-resync`. On `faster` the client
   shortens its 33.3 ms frame by ~5% (Overwatch runs 16 ms frames at
   15.2 ms when starving — the same ratio) until the buffer refills, then
   dilates back. `hard-resync` (buffer empty or > 6 ticks, or a
   tab-throttle return) recomputes the offset outright from fresh pings.
   [gdcvault.com/play/1024001 — Overwatch Gameplay Architecture and Netcode]
2. **Server-side consume throttle** (Rocket League's fallback, credited by
   them to the Overwatch talk): when dilation can't keep up, the server
   consumes 0, 1, or 2 buffered inputs in a tick to re-center the buffer —
   a minor, bounded desync instead of an unbounded drift.
   [media.gdcvault.com/gdc2018/presentations/Cone_Jared_It_Is_Rocket.pdf]

Result: just-in-time inputs across drifting clocks, changing RTT, and
browser tab throttling, with no per-frame guessing anywhere. **The native
client does not get throttled by a backgrounded tab**, but it loses none of
this: an unfocused window, a vsync change and a compositor stall produce the
same drifting-clock problem, and the scheme was never tab-specific — it is
the reason it needs no per-frame guessing on either client.

## 5 · Class S — chunk epochs, and one spine for save + network

The structural world replicates as an **event-sourced log, scoped by the
same 64 m AOI chunks the grid already has**:

- Every chunk holds `version: u32` (monotonic) + a ring of its last 256
  events + a lazily-rebuilt full-state blob (`~16 B/block`: archetype, grid
  pos, tier, hp, flags).
- **Subscribe** (AOI-enter, with the same hysteresis as class D): server
  opens/reuses the chunk's lane and sends either the full state at current
  V (uni stream, nearest chunks first — a 5,000-block megabase ≈ 80 kB,
  once) or, if the client was here recently and its V is inside the ring,
  just the event tail. Unsubscribe remembers V for the hysteresis window,
  so edge-dancing is nearly free.
- **While subscribed**: every structural change arrives as a reliable,
  ordered event bumping V — `placed, upgraded, damaged, destroyed,
  door_state, container_changed, slept, woke, settled, despawned`. Events
  are a few bytes each.
- **Coalescing under a raid storm**: damage events per block collapse to
  latest-HP at a max rate of one per block per 4 ticks; `destroyed` always
  ships immediately. A raid on a megabase is dozens of events/s, not
  thousands **(knob: rates)**.
- **Stream-in is budgeted on BOTH ends** — this was Rust's single biggest
  frame-drop fix, shipped as the Air Power update's "iterative entity
  networking" after two rounds of grid re-granularization: the server
  drips chunk state at N entities per tick per client (Rust ships 512
  steady / 1024 on spawn — our shape, our numbers TBD by bench), and the
  **client applies and tears down in budgeted slices per frame** too —
  Facepunch measured a 30 ms client spike per network cell left, ~15 cells
  at once, before they fixed teardown. A megabase may take half a second
  to fade in; it may never hitch a frame. Client-side decode is zero-copy
  (typed-array views over the payload, no per-packet objects) — Rust's
  formative GC lesson (Devblog 79: the minute-cadence server freezes WERE
  the per-update allocate-serialize-copy) applies to the JS heap
  verbatim. [rust.facepunch.com/news/february-update · devblog-151 ·
  devblog-165 · devblog-79]

This whole section is Facepunch's mature design, arrived at the hard way:
Rust servers run **150k–350k entities** almost entirely structural, viable
only because unchanged entities send nothing (change-driven updates + a
server-side serialized-state cache, `server.netcache`) — which is exactly
what chunk epochs formalize. [devblog-42 · carbonmod convar dump]

**The spine trick — the WAL, the replay log, and the wire are one stream.**
A structural mutation is appended once by the sim as an event; the storage
thread persists it (DESIGN §8), chunk subscribers receive it, and the
deterministic replay re-applies it. One source of truth; three consumers.
If the network ever shows a client a wall the save doesn't have, that is
not a bug class we can ship — the streams are the same bytes.

## 6 · The four mechanics, concretely

### 6.1 · Buildings

- **Place**: client shows a local ghost (visual only, never trusted), sends
  a `place` transaction on the bidi lane. Server validates — privilege zone
  (§6.2), collision, terrain, stability, materials — applies, WALs, fans
  the event; the ack turns the ghost real (or a nack names the refusal).
  Latency on placement is one RTT and it reads fine; nobody notices
  ~100 ms on a deliberate act the way they notice it on movement.
- **Stability** is computed in `sim-core` on the server — support
  propagation from foundations, no weight physics (Rust's deliberate
  choice; don't reinvent it), collapse below a threshold (~5%, their
  shipped number). **Propagation is amortized through a work queue with a
  per-tick budget** — Rust time-boxes it (`stability.stabilityqueue`) so a
  megabase collapse cascades over many ticks instead of stalling one;
  ours is the same queue under hot-path law L4. The **client preview runs
  the same `sim-core` wasm**, so the green/red ghost and the percent
  readout cost zero network and can never disagree with the server beyond
  one in-flight event — cleaner than Rust, whose client-side percent
  plumbing isn't even publicly documented. [devblog-35 · carbonmod
  convar dump]
- **Damage/decay**: coalesced events (§5). Upkeep/decay ticks are sim-side;
  clients only hear the resulting HP events.
- **Doors**: class S events, shipped immediately (no coalescing — a door is
  latency-sensitive). Reliable-lane delivery is ~½ RTT, same as a snapshot
  would be. Your own door plays its animation optimistically on input;
  remote doors move on the event.

### 6.2 · Tool cupboard (building privilege)

Rust's cupboard was exploited for years — stacking, hidden external TCs,
wall-burying — and every fix that stuck was a **rule simplification, not
a check**. We start where they ended (Building 3.0):

- **One cupboard per building, and privilege is emitted by the building's
  own connected blocks** — not a sphere around the box. A volume derived
  from simple geometry gets gamed through geometry; a volume that *is* the
  building leaves nothing to stack. [devblog-189 · rustafied.com
  Building 3.0]
- Authorization is **by presence** (stand at it, press use) and the list
  lives server-side only. The wire carries it exactly once: the
  interaction response when you open a cupboard you're authorized on.
  Rust's protobuf structurally implies the list rides the entity to every
  nearby client — the kind of field ESP tools read — and we design by the
  rule that came out of researching it: **any replicated field is public.
  So don't replicate it.** Every other client gets **one bit in the
  snapshot header** — building-blocked here or not — which is all the HUD
  ever displays anyway.
- Two of Rust's valves are load-bearing and come along **(knobs on
  values, not on existence)**: destroyed-cupboard **grief protection** —
  paid-up upkeep lingers ~24 h so raiders can't insta-decay a base — and
  the sanctioned raid exception (twig and ladders placeable even when
  blocked), which is what keeps a privileged base raidable at all.
- Placement/authorize/deauthorize/clear are transactions; zone recompute
  is sim-side; every privilege change is a WAL event, so disputes replay.

### 6.3 · Sleepers

- Disconnect ≠ despawn. The player entity stays — with a **standing grace
  window** first: for 10 s **(knob)** the body remains upright, fully
  killable, no invulnerability of any kind; a reconnect proving the same
  address (SIWE) inside the window resumes seamlessly (this is also the
  network-change story, §10 — a network change kills the connection, so a
  Wi-Fi handoff is a disconnect, and it should cost a scare, not a death).
  At grace end:
  pose → lying, collider → low capsule, class D → S with a `slept` event
  carrying the rest transform. Zero per-tick cost from that moment.
  Combat-logging buys nothing either way — the body is present and
  killable from the first second.
- Sleepers are lootable (container interaction, server-validated range/LOS)
  and killable (damage events). Metabolism while sleeping: paused v1
  **(knob** — Rust keeps it running**)**. Despawn: never during a wipe
  cycle — the same-entity-plus-flag model Facepunch has shipped since
  Devblog 7, persisted in the world save — with Rust's one pragmatic
  exception adopted: a sleeper inside the **haven** is removed after
  20 min **(knob)**, so the safe zone can't become a free storage locker.
  The body in the field is the brand's honesty about logging off.
  [rust.facepunch.com/news/friday-devblog-7 · corrosionhour.com
  safe-zone rule]
- Client renders sleepers cheap and culls them aggressively (Rust added a
  dedicated sleeper cull distance and measured 20–30% client FPS back) —
  they're static meshes, not animated characters. [devblog-122]
- Reconnect: rebind session → `woke` event → back to class D, with the
  server-streamed state the client needs already covered by the join
  bundle + chunk subscribe (a returning player is just a late joiner
  standing where their body is).
- **Lag comp excludes sleepers** (§8): they cannot move, so hits test
  against present state; the rewind ring stays awake-players-only.

### 6.4 · Dropped items, backpacks, projectiles that stick

- **Death** spawns one **backpack container entity** holding the whole
  inventory — one entity, not thirty. This is exactly Rust's shipped
  consolidation (ragdoll corpse ~5 min, then it collapses into a single
  backpack; Facepunch's stated reason was cost), minus the ragdoll: v1
  goes straight to the backpack, a cosmetic corpse mesh is content for
  later **(knob)**. Despawn is Rust's tunable shape — **one base constant
  × a rarity multiplier** (theirs: 5 min base → ~5/20/40/60 min tiers,
  backpack lifetime = base + best-item tier) **(knob: the constant)**.
  [rustafied.com 2017-06 QoL · carbonmod convar dump]
- **Deliberate drop / throw**: `spawn` event → class D for the ballistic
  arc → `settled` event with the resting transform → class S furniture
  with a despawn timer. **Rest is forced deterministically, never left to
  a physics engine's sleep heuristic** (Gaffer's rule from networked
  physics: he disabled PhysX sleep and forced rest himself): a 16-tick
  ring of positions per hot item — no significant movement in 16 ticks →
  it rests, exactly there. A hard 2 s deadline backstops pathological
  cases: past it, the item settles where it is. No item may stay hot.
  [gafferongames.com/post/networked_physics_in_virtual_reality]
- **Arrows/spears that stick** follow the same arc: D in flight, `stuck`
  event (surface + transform), S until picked up or despawned.
- **Items never collide with each other — only with the world.** Rust
  shipped item-item collision and then globally disabled it under 200-
  player load; we skip the intermediate lesson. Settled items also shed
  their physics state entirely (Rust grew a convar to strip sleeping
  rigidbodies; our class transition IS that, by construction).
  [devblog-56 · carbonmod convar dump]
- **Bounds** (DESIGN L4): per-chunk cap on class-S loose items; overflow
  despawns oldest-lowest-tier first, loudly in the log. A loot-fountain
  griefer hits the cap, not the tick budget.

## 7 · Interest management (both classes, one grid)

The 64 m grid serves both pipelines: class D snapshot membership *and*
class S chunk subscriptions come from the same enter/leave sets with the
same hysteresis (176 m in / 208 m out). One spatial truth, two products.
Subscribe cost is dominated by first-visit chunk state; the join bundle
pre-streams the spawn ring (9 chunks) before the first keyframe so a fresh
spawn never sees pop-in of the base they spawned beside.

⚠ **That paragraph is the design; the tree has the radius and not the
grid** (measured 2026-08-10, `reference/NETWORK.md` §9.3; half of what it
found is closed). There is still no grid and no chunk subscription. Class D
is a flat scan of `MAX_PLAYERS + MAX_MOBS` with a radial hysteresis
compare, per client per tick — 16,400 distance tests per tick at cap, which
is affordable and is not the problem.

**Class S is filtered as of 2026-08-18** (`server/src/interest.rs`,
`DECISIONS.md` §open "class-S interest v0"), and it is filtered on class
D's own numbers, which is the sentence above taken as far as it goes
without a grid: the piece walk is aimed from an anchor, streams
`AOI_EXIT_CM`, and re-arms when the player leaves the anchor by
`AOI_EXIT_CM − AOI_ENTER_CM` — so a completed walk holds every piece within
`AOI_ENTER_CM`, and the `EV_PIECE_PLACED` broadcast uses the same predicate
rather than a second opinion. Measured on a 2,291-piece island with 454
pieces in range: a joiner's walk went 2,291 → 454 records, 11,384 → 2,258
bytes, complete at tick 72 → 19. `server/tests/piece_interest.rs` is the
gate; §11's `test_stream_in` is still unbuilt and is a different assertion
(the client's per-frame apply/teardown budget, which nothing measures).

**What is still §5's design and not the tree**: there is no chunk version,
no subscribe/unsubscribe, and therefore no way to tell a client to *drop*
what it holds — so a piece removal is still broadcast to everyone
unfiltered, because nothing else can un-say a wall, and a re-arm re-walks
the whole in-range set rather than the difference. Deploys and backpacks
are unfiltered on both counts, and the deployable walk still restarts on a
removal (`reference/NETWORK.md` §9.2.1's amplifier, one store over).

**The planned extension is occlusion, and it lands here** (`DECISIONS.md`
2026-08-04 · `NOW.md` 18): a class-D member fully occluded by terrain is
dropped from the snapshot set rather than merely deprioritized, which is
the genre's proven anti-ESP measure and costs no client trust. It filters
these same enter/leave sets — one spatial truth stays one — and it is
cheaper here than where it was proven, because a seeded heightfield lets
the occlusion grid bake at worldgen into a fixed structure the tick only
reads. Class S is untouched: chunk subscriptions are terrain you are
standing on, not information about anyone.

## 8 · Lag compensation, scoped tight

Ring of collider transforms, **awake players only**, 30 sim-ticks (1 s of
history — the window both Quake 3 and Source shipped). The rewind time is
Source's exact formula, which most homebrew implementations get wrong by
omitting the third term:

```
rewind_to = server_now − that_client's_latency − that_client's_ACTUAL_interp_delay
```

— the server tracks each client's current interpolation delay (it varies
per §3) rather than assuming the default. Two bounds on top, both from
Overwatch's shipped tuning: **favoring is clamped to 250 ms** of rewind,
and **above ~220 ms RTT it turns off entirely** — the laggy shooter is
extrapolated instead and the dodge wins. You can be shot around a corner
by at most a quarter-second of someone else's connection, never a second.
[developer.valvesoftware.com/wiki/Source_Multiplayer_Networking ·
gdcvault.com/play/1024001]

Sleepers, buildings, and settled items test at **present** time — they
don't move; there is nothing to rewind — which is what keeps the ring at
100 awake players × 30 ticks × ~16 B ≈ **48 kB**, preallocated. Ballistic
projectiles are sim entities and need no rewind at all; they hit when and
where the sim says.

**Built, 2026-08-29/30, and the projectile sentence above needed one word
splitting off it.** Two verbs rewind and one deliberately does not.
`combat::strike` (melee) and `ranged::hitscan` (firearms) both resolve their
target scan against `rewind::pose_at` at the tick's granted favour;
`ranged::step` — the arrow already in the air — resolves against present
bodies, which is what the paragraph above means and is now a **type**
(`ranged::Pose`) rather than a claim. A hitscan is *not* a ballistic
projectile: it has no flight, it resolves on the tick its trigger came down,
and for one pass it was the only fight on the shard decided by ping. The
launch aim of an arrow is refused on the record (`DECISIONS.md` §open) — it
is not a gap. The favour is minted by the client and clamped by the server
to `REWIND_MAX_TICKS`; **the server does not compute it yet**, which is the
part of this section that is still a plan (`NOW.md` §0lc).

## 9 · Budgets (updated for the classes)

| thing | number |
|---|---|
| **the tick itself, 100 clients in one AOI cell, all acking, all swinging** | **~0.8 ms of the 33.3 ms budget** — sim ~0.15, interest + events + drip ~0.24, snapshot encode ~0.43. Measured 2026-08-11, `cargo run --release -p server --bin profile`; the periodic whole-world passes are `state_hash` 85 µs one tick in 32 and `encode_world` 24 µs one tick in 1,800. The one row here that was never measured, and the reason the AOI scan needs no spatial structure |
| class D entities in a client's interest set, typical / cap | ~15 / 64 |
| per-client downstream, steady | ≤ 20 kB/s (snapshots) + bursts on approach (chunk streams) |
| chunk full-state, dense base chunk | ≤ 24 kB (1,500 blocks × 16 B) |
| megabase approach burst | ≤ 100 kB across ~4 chunks, nearest-first, stream-paced |
| structural events, raid storm, per subscriber | ≤ 40 events/s after coalescing |
| WAL append rate at 100 players, raid-heavy | ≤ 2,000 events/s (storage thread's problem, sized 10×) |
| rewind ring | 100 awake players × 30 ticks × ~16 B ≈ 48 kB, preallocated |

## 10 · Failure and rejoin matrix

| failure | what happens |
|---|---|
| lost snapshot datagram | nothing; next snapshot supersedes (baselines make gaps free) |
| lost input datagram | covered by unacked-input redundancy (≤ 10 frames ≈ 333 ms); beyond that the server reuses the last input (you glide briefly), dilation refills the buffer |
| ack gap > baseline ring | baseline drops to the canonical zero-state; the same delta path streams the world back by priority (Q3's dummy-gamestate move) |
| chunk event lane stalls | QUIC retransmits (reliable); if the stream resets, resubscribe at V → tail replay |
| client stops stepping (was: a throttled background tab; now a suspended laptop, an unfocused window, a compositor stall) | dilation hard-resync on return; > 30 s → treated as disconnect: **you become a sleeper where you stand**. The cause changed with the client and the handling did not — §4 says why it never was tab-specific |
| network change (Wi-Fi↔cellular) | the connection dies; the client reconnects and proves the same address (SIWE) — inside the 10 s standing grace it's seamless, past it you wake your sleeper |
| client disconnect | standing grace 10 s, then sleeper (§6.3); the same proved address wakes the body |
| server crash | DESIGN L7: restart < 10 s from snapshot + WAL; clients auto-reconnect, rejoin as wakers; ≤ 1 tick of acked transactions lost, and none that were acked |
| mid-raid restart | the raid's events are WAL'd before ack — the wall you blew is still blown |

## 11 · The CI gates this file asks for — ⚠ **none of them exist**

> **Retitled 2026-08-10.** This section was headed "Added CI gates" and
> listed seven. A grep over `crates/` and `ci/` returns **zero hits for all
> seven**. Nothing here was ever built, and the old title asserted the
> opposite — which is precisely the failure `CLAUDE.md` names as the
> dangerous one: a doc that reads as covered while nothing checks it. The
> designs below are good and are kept verbatim as designs; every one of
> them is unbuilt. `reference/NETWORK.md` §9.3 carries the audit, and §9.2
> ranks the gaps three of these gates would have caught —
> `test_stream_in` and `test_raid_storm` both bear directly on the class-S
> join walk (§9.2.1), and `netem profiles` is the only item here that would
> exercise the reorder/loss/dup torture list at all.
>
> Two of them are also named as enforcement elsewhere and marked there:
> `test_raid_storm` in `CLAUDE.md` wall 4 and `DESIGN.md` §12.
>
> ⚠ **`test_raid_storm` now exists — and it is not the one below.** As of
> 2026-08-14 `crates/sim-core/tests/raid_storm.rs` carries that name for
> wall 4's meaning: 64 synthetic players raiding each other in `sim-core`,
> asserting every store's cap per tick. It speaks to no socket, subscribes
> nobody, and times nothing. **The gate specified below is the wire half
> and is still unbuilt** — coalescing caps, tick p99 and byte counts under
> 20 subscribers are all outside what a `World` can see. So this list is
> six unbuilt, not seven, and the one that landed answers a different
> question. Read the name with its crate attached.

- `test_chunk_epoch`: fuzz subscribe/unsubscribe/re-subscribe against a
  mutating chunk; client-reconstructed state must equal server state at
  every version.
- `test_class_transitions`: item drop→settle, player sleep→wake,
  arrow fly→stick under packet loss; no entity may be visible in both
  pipelines or neither.
- `test_raid_storm`: scripted 4-satchel raid on a 3,000-block base with 20
  subscribers; assert coalescing caps, tick p99, and byte counts — and
  that the collapse cascade amortizes across ticks (the stability queue
  never blows the budget in one).
- `test_stream_in`: drive a client past a megabase at sprint speed; server
  drip stays within its per-tick entity budget and the client harness
  records no frame over budget during apply **or teardown** (Facepunch's
  measured failure was teardown).
- `netem profiles` in the client harness: 50 ms/0%, 150 ms/5%, 250 ms/10%,
  plus a 2 s total blackout — the feel bar (DESIGN §9) must hold on the
  first two, degrade honestly on the rest.
- `test_sleeper_soak`: 500 sleepers + 50 awake bots for an hour — sleepers
  must cost zero datagram bytes and zero tick-time beyond collision.
- `bench_transport`: headless wtransport load clients (there is **no
  published quinn/wtransport benchmark at hundreds of connections with
  small frequent datagrams** — nobody has done our measurement for us);
  gate: 100 conns × 30 Hz each way on the reference box, p99 datagram
  latency and zero endpoint-task saturation.

## 12 · Sources

The load-bearing ones; inline cites above point here.

**The canon**
- gafferongames.com — /post/snapshot_interpolation · /post/snapshot_compression · /post/state_synchronization · /post/networked_physics_in_virtual_reality · /post/reliable_ordered_messages · /post/packet_fragmentation_and_reassembly · /post/reliability_ordering_and_congestion_avoidance_over_udp
- developer.valvesoftware.com/wiki/Source_Multiplayer_Networking — interp/lag-comp formulas and shipped defaults
- gdcvault.com/play/1024001 — Overwatch Gameplay Architecture and Netcode (command frames, time dilation ~5%, favor-the-shooter bounds ~0.5 s history / off above ~220 ms RTT)
- fabiensanglard.net/quake3/network.php — pure delta-snapshot model, 32-state ack window, zero-baseline fallback
- media.gdcvault.com/gdc2018/presentations/Cone_Jared_It_Is_Rocket.pdf — input buffering, consume-throttle, ≤10-frame input redundancy

**Facepunch's shipped record** (rust.facepunch.com/news/…)
- friday-devblog-7 (sleepers are the same entity) · devblog-35 (stability) · devblog-42 (150k entities @60fps) · devblog-56 (item physics retreat) · devblog-79 (GC freezes = per-update allocation; pooled, zero-copy fix) · devblog-122 (sleeper culling, 20–30% fps) · devblog-151/-165/february-update (grid granularity → iterative budgeted streaming, 512/1024 batches) · devblog-189 (Building 3.0: one cupboard per building, privilege from the building) · maintenance 2025-09 (~350 m grid cells, parallel player jobs)
- carbonmod.gg/references/rust-convars — machine-extracted convar defaults (stability.*, itemdespawn 300 s, netcache, updatebatch, upkeep brackets)
- rustafied.com (Building 3.0; 2017-06 QoL: corpse→backpack, rarity despawn tiers) · corrosionhour.com (cupboard exploits; safe-zone sleeper rule)

**Transport**
- RFC 9221 (datagrams are congestion-controlled, no fragmentation) · RFC 9002 (loss/CC behavior)
- quinn docs + source — CUBIC default, BBR experimental, pacer constants (10-datagram minimum burst), DPLPMTUD 1200→1452, send_datagram drop-oldest semantics, single-socket model + buffer advice, Incoming::retry()/refuse()
- github.com/BiagioFesta/wtransport — #317 remote-panic (fixed on main 2026-07-25, pin ≥ 0f7609a), config surface, with_custom_transport
- W3C WebTransport spec + MDN — silent oversize-datagram drop, outgoingMaxAge, incoming drop-oldest, getStats, worker support, serverCertificateHashes rules (ECDSA P-256, <14 days)
- webkit.org/blog/17862 — Safari 26.4 ships WebTransport (datagrams included) · caniuse.com/webtransport (~88%) · bugzilla.mozilla.org/1873263 (Firefox <125 cert-hash bug)
- gafferongames.com/post/why_cant_i_send_udp_packets_from_a_browser — the pre-RFC-9221 critique this design answers

**Honest gaps carried forward**: no published quinn/wtransport benchmark at
our shape (hence `bench_transport` is a gate, not an assumption); Chrome's
live `maxDatagramSize` behavior mid-connection unconfirmed (hence the
runtime clamp); Rust's current post-RakNet transport undocumented; whether
Rust replicates cupboard auth lists is structurally implied, not proven
(hence we simply don't).
