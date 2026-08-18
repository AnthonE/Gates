//! Shard counters: plain atomics, written from any thread (an atomic store
//! is not a syscall, a lock, or an allocation — sim-thread safe), read by
//! tests, the smoke gate, and later the status page. Integer-only by
//! design (L5: diagnostics are numbers, not strings).

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

#[derive(Default)]
pub struct ShardStats {
    /// Sim ticks completed.
    pub ticks: AtomicU64,
    /// The world tick, published every tick for the welcome message and
    /// the status page — the sim thread stores, everyone else loads.
    pub current_tick: AtomicU64,
    /// Ticks abandoned because the thread fell > the backlog bound behind.
    pub ticks_dropped: AtomicU64,
    pub joins: AtomicU64,
    pub leaves: AtomicU64,
    /// Bodies with a live connection right now — a gauge like `sleepers`,
    /// mirrored off `ShardCore::connected` each tick, never accumulated
    /// here. It exists because `joins - leaves` is NOT this number: a
    /// refused install rides the LEAVING sweep out and bumps `leaves` with
    /// no matching `join`, so the counter pair drifts one short per
    /// refusal while the array it would describe does not. The status
    /// endpoint (`status.rs`) reads this, and it is the honest count the
    /// shard list's `players` column has been waiting on.
    pub players: AtomicU64,
    pub refused_version: AtomicU64,
    /// Joins refused for being below this shard's `min_client`
    /// (`REFUSE_BUILD`). **Counted apart from `refused_version` because the
    /// two say different things about the same shard**: a version refusal is
    /// a wire mismatch — somebody is on a build that cannot talk to this one
    /// at all, and there is nothing to decide. This one is the shard's own
    /// policy working, and it is the counter an operator watches after
    /// raising the floor: a number that keeps climbing days later means the
    /// release nobody could reach was never actually published.
    pub refused_build: AtomicU64,
    /// Joins refused for want of a proven identity (`REFUSE_AUTH`): a
    /// signature that failed to verify, or a guest where `require_auth`
    /// demands an address. Counted separately from `refused_version`
    /// because the two mean opposite things about a shard: a version
    /// refusal is a client that needs updating, an auth refusal is a shard
    /// doing its job.
    pub refused_auth: AtomicU64,
    /// Joins refused because the chain says this wallet holds no copy
    /// (`REFUSE_TICKET`, `entitle.rs`). **Only a definite on-chain zero
    /// lands here** — that is the whole point of keeping it apart from the
    /// counter below.
    pub refused_ticket: AtomicU64,
    /// Ticket checks that could not be answered — a timeout, an unreachable
    /// origin, an `entitled: null`. **Every one of these ADMITTED a player**,
    /// which is deliberate (an outage must not empty the shard) and is
    /// exactly why it is counted: a fail-open that nobody can see is a fail-
    /// open nobody fixes. A shard whose ticket door is armed and whose
    /// `refused_ticket` is 0 while this climbs is a shard checking nothing.
    pub entitle_unknown: AtomicU64,
    /// Roster sweeps that kicked somebody — a copy sold mid-session. Same
    /// rule: a definite zero and nothing else.
    pub entitle_kicked: AtomicU64,
    pub refused_full: AtomicU64,
    pub handshake_errors: AtomicU64,
    /// Input datagrams decoded and ringed.
    pub input_dg_ok: AtomicU64,
    /// Input datagrams that failed to decode (client-driven bytes; never a
    /// panic, always a count).
    pub input_dg_bad: AtomicU64,
    /// Input datagrams that decoded but carried a value the sim can never
    /// mean — a button bit outside `BTN_MASK` (NOW.md §5b). Counted apart
    /// from `input_dg_bad` because the two accuse differently: bad bytes
    /// are line noise or a broken client, a forged in-layout value is a
    /// client this version cannot have built. Dropped, never a disconnect —
    /// the datagram lane's loss policy is redundancy, not framing trust.
    pub input_dg_forged: AtomicU64,
    /// Inbound ring full — datagram dropped (redundancy re-carries).
    pub input_ring_drops: AtomicU64,
    /// Snapshots encoded and handed to rings.
    pub snap_sent: AtomicU64,
    /// Outbound ring full — that client skipped a snapshot.
    pub snap_ring_skips: AtomicU64,
    /// send_datagram refused (too large / not connected).
    pub snap_send_errors: AtomicU64,
    /// Entities refused by the encoder's range check — a sim bug counter,
    /// asserted zero in tests.
    pub encode_range_errors: AtomicU64,
    /// Interest entities a snapshot could not carry — the datagram budget
    /// or `MAX_SNAPSHOT_ENTITIES` refused them and the accumulator re-offers
    /// them next time (`core.rs` `encode_snapshot`).
    ///
    /// **Not an error counter, and that is why it needs to exist.** Shedding
    /// is the designed degradation: the snapshot sheds entities rather than
    /// fragmenting, which is correct and is what keeps a 1% loss rate from
    /// becoming 9.5% (NETCODE.md §3). But it was previously the only path in
    /// the whole pipeline by which quality drops under load with *nothing*
    /// recording it — `snapshot_budget.rs` proves the budget holds and could
    /// not tell you how close it runs. A shard where this climbs is a shard
    /// whose players are seeing staler bodies than the design promises, and
    /// before this counter the only symptom was a complaint.
    /// `reference/NETWORK.md` §3 is the reason: their fragmentation bug and
    /// their Lidgren stall were both found by instrumentation, not by
    /// reasoning about the code.
    pub snap_entities_shed: AtomicU64,
    /// Interest entities a snapshot **offered** the fill — the ranked
    /// candidate list, one count per candidate per snapshot, summed over
    /// every client.
    ///
    /// The denominator `snap_entities_shed` has never had. A shard that has
    /// shed a million entities is either healthy (it offered ten million) or
    /// on fire (it offered a million and one), and nothing in the pipeline
    /// could tell those apart: shedding is a *ratio*, and only one half of
    /// it was counted. This is the other half, and it is also the only
    /// number that says how much work the AOI fill is actually doing —
    /// `NOW.md` §0's 100-bot soak exists to decide whether the linear scan
    /// needs a spatial structure, and that decision is this counter divided
    /// by `ticks`.
    ///
    /// The client's own entity is **not** counted. It rides ahead of the
    /// ranked list, is never a candidate, and the budget is never allowed to
    /// refuse it; counting it would add a constant `snapshots × 1` to this
    /// and to `snap_entities_sent` alike and flatter the ratio.
    ///
    /// That exclusion, and the placement of the bump *below* the own-entity
    /// `return None`, are both invisible to the identity `snap_entities_sent`
    /// names — three counters can agree with each other while all three are
    /// wrong the same way. What sees them is `snapshot_budget.rs`
    /// `the_offered_counter_is_the_interest_set_and_nothing_else`, which sums
    /// the interest flags independently over the snapshots that went out and
    /// requires this counter to equal that sum.
    pub snap_candidates: AtomicU64,
    /// Interest entities a snapshot **carried** of what it was offered —
    /// `snap_candidates` minus `snap_entities_shed`, counted rather than
    /// derived so the three can be checked against each other
    /// (`snapshot_budget.rs` `the_fill_counts_what_it_offered_and_what_it_
    /// carried` asserts the identity, which is the only thing that notices
    /// if one of them starts describing a different pass).
    ///
    /// Distinct from `snap_sent`, which counts **snapshots**: this counts
    /// the entity records inside them, and excludes the own entity for the
    /// reason `snap_candidates` gives.
    pub snap_entities_sent: AtomicU64,
    /// Class-S join walks restarted from zero (`core.rs`, the swap-remove
    /// rule): a removal landed while a client's sync cursor was still
    /// inside the store, so that client's walk begins again with a reset
    /// batch.
    ///
    /// Correct under the store's swap-remove — a walk reading *upward*
    /// cannot trust a cursor into a set that reshuffled beneath it — and
    /// **unbounded in cost**, which is the half worth counting. A full walk
    /// is `store_len / 32` ticks; removals arriving faster than that walk
    /// the client back to zero indefinitely, and a raid removes pieces far
    /// faster than that. The client's symptom is a world that never
    /// finishes loading, which reads as anything but a network problem
    /// (`reference/NETWORK.md` §9.2.1 has the arithmetic).
    ///
    /// ⚠ **The piece walk no longer restarts and no longer bumps this.**
    /// It reads the store from the tail down, where the entry a
    /// swap-remove moves is always one already sent, so the cursor
    /// survives (`core.rs` `drip_client` carries the argument). What is
    /// still counted here is the **deployable** walk, which still reads
    /// upward and still restarts on a removal — the same defect, one store
    /// over, and the reason this counter keeps its name.
    pub piece_walk_restarts: AtomicU64,
    /// Piece walks that reached the end — a client that has been sent every
    /// piece the store held when its walk began.
    ///
    /// The other half of `piece_walk_restarts`, and it exists because that
    /// counter alone cannot answer the only question an operator has: a
    /// shard reporting restarts cannot be told apart from a shard where the
    /// walk restarted twice and then *finished* — the first is a world that
    /// never arrives, the second is a hiccup. One completion per client per
    /// walk, so `completes` short of the join count is a shard where
    /// somebody is still waiting.
    pub piece_walk_completes: AtomicU64,
    /// Piece walks re-armed because the player walked `PIECE_REARM_CM` out
    /// from under the anchor the walk was aimed from (class-S interest v0,
    /// `interest.rs`).
    ///
    /// The *intended* restart, and it is the one number that says what the
    /// filter costs: a re-arm re-walks everything now in range rather than
    /// the difference, so `rearms` × the local base is the redundancy v0
    /// pays for having no chunk grid to subscribe to. Bounded by movement —
    /// at most one per 32 m travelled, ~5.8 s of sprinting — which is what
    /// keeps it apart from `piece_walk_restarts`, whose bound was a raid's
    /// removal rate and therefore was not one.
    pub piece_walk_rearms: AtomicU64,
    /// Piece-store entries a walk scanned and skipped as out of interest.
    ///
    /// The filter's yield, and the honest denominator for it: `skipped`
    /// against `PIECE_SCAN_BATCH × walking ticks` says how much of the
    /// island a shard's clients are being spared, and a shard where it
    /// stays near zero is one whose players all stand in the same place —
    /// not one where the filter is off.
    pub piece_sync_skipped: AtomicU64,
    /// `EV_PIECE_PLACED` broadcasts a connection was not sent because the
    /// piece is outside its class-S interest.
    pub piece_events_skipped: AtomicU64,
    /// Clients forced back to the zero-state baseline by a bookkeeping
    /// overflow (pending removals) — the honest escape hatch.
    pub forced_resyncs: AtomicU64,
    /// Event-lane messages accepted by per-connection rings.
    pub ev_sent: AtomicU64,
    /// Event-lane resyncs: a refused ring push or a dropped sim event
    /// restarted a client's harvested-set walk (limits.rs policy).
    pub ev_resyncs: AtomicU64,
    /// Event-lane stream writes that failed (connection dying).
    pub ev_send_errors: AtomicU64,
    /// C→S action frames decoded and ringed.
    pub actions_ok: AtomicU64,
    /// C→S action frames that failed to decode — client-driven bytes on
    /// the reliable lane, so the session drops (framing trust is gone).
    pub actions_bad: AtomicU64,
    /// C→S chat lines decoded and ringed.
    pub chat_ok: AtomicU64,
    /// C→S chat frames refused by the decoder — bad UTF-8, a control
    /// character, a forged length, a non-canonical line. Counted, never a
    /// dropped session: chat text is the one payload a *client bug* can
    /// plausibly malform, and killing the connection over a stray byte
    /// would be a worse outcome than swallowing it.
    pub chat_bad: AtomicU64,
    /// Chat lines the per-connection rate limiter refused (ALPHA.md §1:
    /// "rate-limited server-side"). The sender is not told; the line is
    /// simply not said.
    pub chat_rate_limited: AtomicU64,
    /// Chat lines dropped by a full inbound ring (limits.rs policy).
    pub chat_ring_drops: AtomicU64,
    /// Chat relays a recipient's event ring refused. Chat has no sync
    /// walk to restart, so unlike every other event this does not force
    /// an `ev_resync` — the line is simply lost, and counted.
    pub chat_undelivered: AtomicU64,
    /// Joins that arrived as a *saved* character rather than a fresh one
    /// (`store.rs`). The one number that says persistence is doing anything:
    /// a shard with a save file whose `saves_restored` stays 0 while
    /// `joins` climbs is remembering nobody, and that is a defect the
    /// operator can see without reading a log.
    pub saves_restored: AtomicU64,
    /// Joins that took over the body they left behind instead of reading a
    /// record — the sleeper path (`world.rs` `Command::Wake`). Counted
    /// beside `saves_restored` rather than folded into it because the two
    /// answer different questions: this one says sleepers are working, and
    /// a shard where it stays 0 while `sleepers_evicted` climbs is one
    /// where bodies are being reaped before their owners get back.
    pub takeovers: AtomicU64,
    /// Sleeping bodies deleted to free a slot for a join (`world.rs`
    /// `Command::Evict` — two-phase, so the victim's current save was
    /// filed first; the policy is `ShardCore::evict_victim`). Mirrored out
    /// of `World::evictions`; this is the operator-facing copy. Nonzero
    /// means the shard is past `MAX_PLAYERS` distinct recent visitors and
    /// somebody came back to a record instead of a body.
    pub sleepers_evicted: AtomicU64,
    /// Bodies asleep in the world right now — a gauge, not a counter, set
    /// each publish rather than bumped.
    pub sleepers: AtomicU64,
    /// Whole worlds written to disk (`worldfile.rs`). The number that says
    /// world persistence is doing anything, and the one to look at first
    /// when a restart loses a base.
    pub world_saves_written: AtomicU64,
    /// World saves the cadence asked for and could not take, because no
    /// buffer had come back from the store thread yet — the stated overflow
    /// policy of `WORLD_RING_CAP` (skip, never wait). A shard where this
    /// climbs is one whose disk cannot keep up with its own save interval,
    /// and the fix is a longer `world_save_interval_ticks` and not a deeper
    /// queue: the next save takes a *fresher* world, so a skip costs
    /// nothing a later save does not replace.
    pub world_saves_skipped: AtomicU64,
    /// World writes that failed at the filesystem. A shard that cannot
    /// persist keeps running, exactly as it does for a player record —
    /// dropping everyone because a disk filled is worse than forgetting.
    pub world_save_errors: AtomicU64,
    // ── Transport telemetry (NETCODE.md §2.2) ───────────────────────────
    //
    // Read off quinn's own `ConnectionStats` in `writer_task`, one sample
    // every `NET_SAMPLE_EVERY` polls. **Deltas, not gauges**, for the four
    // counters below: each sample adds what changed since that connection's
    // previous sample, so the shard total keeps meaning something as
    // connections come and go. A gauge would read whatever the last player
    // to be sampled happened to have.
    //
    // Why this exists at all: until 2026-08-15 nothing in `crates/` called
    // `stats()`, `rtt()` or any other transport reading, so every congestion
    // claim in `NETCODE.md` was unfalsifiable and the 100-bot soak's
    // headline (16.5 kB/s/client) was a ceiling somebody computed rather
    // than a number anybody measured.
    /// Packets quinn's loss detection declared lost, summed over every
    /// connection. The first number to look at when players report rubber-
    /// banding: `snap_send_errors` says *we* refused to send, this says the
    /// path ate it.
    pub net_lost_packets: AtomicU64,
    /// Packets sent, same summing. Only useful beside `net_lost_packets` —
    /// a loss *rate* is the reading, and neither half means much alone.
    pub net_sent_packets: AtomicU64,
    /// Times the congestion controller reacted to loss. §2.1 names this as
    /// the real risk: CUBIC halves cwnd here, and a collapsed window at
    /// 100 ms RTT can fall below our 30 Hz send rate. Climbing steadily is
    /// the signal to A/B `cc = "bbr"`.
    pub net_congestion_events: AtomicU64,
    /// Times a path black hole was detected (MTU dropped to the floor).
    /// Rare and load-bearing when nonzero — it means somebody's path is
    /// silently eating full-size datagrams.
    pub net_black_holes: AtomicU64,
    /// Worst round-trip seen since boot, in microseconds — `fetch_max`, so
    /// a high-water mark rather than a current reading. A gauge of "now"
    /// across 100 connections has no single value; the worst does.
    pub net_rtt_us_max: AtomicU64,
    /// Path MTU from the most recent sample, in bytes. A plain last-writer-
    /// wins gauge and deliberately not a min: what it answers is "did
    /// DPLPMTUD get above the 1 200 floor", and any recent connection
    /// answers that.
    pub net_mtu_last: AtomicU64,
    /// **What the OS actually granted for the UDP receive buffer**, read
    /// back after the bind rather than assumed from what we asked for
    /// (`net::bind_udp`). This is the whole point of the readback: Linux
    /// caps the request at `net.core.rmem_max` *silently*, so a shard can
    /// ask for 8 MiB, get ~208 KiB, and nothing anywhere would say so.
    /// Compare against `net_rcvbuf_asked`.
    pub net_rcvbuf_bytes: AtomicU64,
    /// What we asked for. Present so the pair can disagree out loud.
    pub net_rcvbuf_asked: AtomicU64,
    /// Granted UDP send buffer, same readback and the same reason.
    pub net_sndbuf_bytes: AtomicU64,
    /// Asked-for UDP send buffer.
    pub net_sndbuf_asked: AtomicU64,
    // ── Lane bytes (NOW.md §0q item 4, "real bytes") ────────────────────
    //
    // The soak's headline bandwidth number was a **ceiling** — budget ×
    // rate × clients, computed by hand — because nothing in the shard had
    // ever counted a byte. These four pairs are the measurement, and they
    // are four pairs rather than one number for two reasons.
    //
    // **The lanes are counted apart** because they degrade apart: the
    // datagram lane sheds under budget pressure and the stream lane
    // backpressures, so a single conflated total would move for reasons an
    // operator could not tell apart, which is worse than no number.
    //
    // **Bytes carry a count** because bytes alone cannot separate "more
    // packets" from "fatter packets" — the same kB/s is a snapshot rate
    // problem or an AOI fill problem depending on which half moved, and
    // those have opposite fixes.
    //
    // **Aggregate, not per-client**, deliberately: the send/receive sites
    // hold a slot index into `SlotTable`, which is a table of state *words*
    // and nothing else, so a per-client byte table would be a new structure
    // invented for a counter. The per-client half already exists on the
    // other end — `botclient::BotReport` measures one client's own bytes and
    // `bin/bots` prints a line per bot — so a soak gets its distribution
    // there and its shard truth here, divided by the `players` gauge.
    //
    // **Payload bytes, on the steady-state lanes.** What QUIC then spends on
    // headers, ACKs and encryption is not here; `net_sent_packets` is, so the
    // two together are the overhead ratio. The handshake is excluded too — a
    // per-join constant is not part of a rate, and threading a counter
    // through five accept-path helpers to say so would buy nothing.
    /// Snapshot datagram payload bytes the socket **accepted** — counted on
    /// `Ok` from `send_datagram` and nowhere else, so an oversize refusal or
    /// a dead connection moves `snap_send_errors` and leaves this alone.
    /// Bytes we asked to send are not bytes that went (`net_rcvbuf_bytes`
    /// exists for the same reason one lane up).
    pub net_dg_out_bytes: AtomicU64,
    /// Snapshot datagrams accepted. `net_dg_out_bytes / this` is the mean
    /// snapshot size, which is the number `DATAGRAM_BUDGET_BYTES` is a
    /// ceiling for and which nothing has ever compared against it.
    pub net_dg_out_count: AtomicU64,
    /// Input datagram payload bytes that **arrived**, counted before the
    /// decode and regardless of it: a forged or malformed datagram cost the
    /// same path the same bytes, and a bandwidth number that only counted
    /// the well-formed ones would flatter a shard under exactly the load it
    /// matters under.
    pub net_dg_in_bytes: AtomicU64,
    /// Input datagrams that arrived. Sums with `input_dg_ok` +
    /// `input_dg_bad` + `input_dg_forged`, which is the cross-check.
    pub net_dg_in_count: AtomicU64,
    /// Event-lane bytes written to the reliable stream, **length prefix
    /// included** — the prefix is on the wire, so a count that left it out
    /// would understate the lane by two bytes per frame, and the event lane
    /// is the one that sends many small frames.
    pub net_stream_out_bytes: AtomicU64,
    /// Event-lane frames written. Counted only on a write that returned
    /// `Ok`; a frame that died half-written is counted as `ev_send_errors`
    /// and its partial bytes are deliberately not guessed at.
    pub net_stream_out_frames: AtomicU64,
    /// C→S action/chat bytes read off the reliable stream, prefix included
    /// and counted before the demux, so a chat line the limiter refuses
    /// still cost what it cost.
    pub net_stream_in_bytes: AtomicU64,
    /// C→S frames read.
    pub net_stream_in_frames: AtomicU64,
    /// Connection attempts answered with a QUIC Retry — the address had not
    /// proved it can receive, and the shard was busy enough to make it
    /// prove that before spending a handshake. Costs a legitimate joiner one
    /// extra round trip and costs a spoofed source everything.
    pub admit_retried: AtomicU64,
    /// Connection attempts refused before any handshake, because in-flight
    /// handshakes were past the hard cap. Distinct from `refused_full`,
    /// which is a *polite* refusal carrying a reason to a client that
    /// completed a handshake; this one is a flood defence and says nothing.
    pub admit_refused: AtomicU64,

    /// **The storage thread has written everything and stopped.** The one
    /// flag here, and it is not a statistic — it is how `bin/shard.rs`
    /// knows a graceful shutdown has actually reached the disk before it
    /// calls `exit`.
    ///
    /// Exact rather than timed, which matters because the alternative is a
    /// sleep somebody picked: the sim thread flushes and drops its
    /// producers, the accept loop drains until abandoned and drops its own,
    /// the storage thread drains until *its* rings are abandoned and sets
    /// this. Every hop waits on a producer being dropped, so the chain ends
    /// exactly when the last byte is written and not a moment that happened
    /// to look long enough.
    pub store_stopped: AtomicBool,
    /// A world blob the sim thread refused, which `bin/shard.rs` had
    /// already accepted into a trial world. Unreachable by construction and
    /// counted anyway, because the alternative to counting it is a shard
    /// silently running a fresh island under everybody's bases.
    pub world_load_errors: AtomicU64,
    /// Records handed to the store's index — a leave, the autosave sweep
    /// finding a player whose state moved, or the shutdown flush taking
    /// every connected player on the way down (`net.rs`, the sim thread's
    /// tail). Those are the only three producers.
    pub saves_taken: AtomicU64,
    /// Records that pushed another player out: the table was full, so the
    /// least recently saved slot was taken (`store.rs`'s stated overflow
    /// policy). A shard where this climbs is remembering more distinct
    /// players than `MAX_SAVED_PLAYERS` holds, and somebody is losing a base
    /// every time it ticks up.
    pub saves_evicted: AtomicU64,
    /// Records dropped because a ring between the sim, the index and the
    /// file was full. Bounded-everything's stated cost: freshness, never
    /// latency — the next sweep re-takes the same player.
    pub save_ring_drops: AtomicU64,
    /// Records written to disk and flushed.
    pub saves_written: AtomicU64,
    /// Write or flush failures. A shard that cannot persist keeps running —
    /// dropping every player because a disk is full would be a worse
    /// outcome than a shard that forgets — so this is the only place that
    /// failure is visible.
    pub save_write_errors: AtomicU64,
    // No counter for records refused at load, deliberately: corruption is a
    // boot-time fact about a file, not a running rate, so it is reported once
    // in the line that says the shard came up (`store::SaveLoad`, printed by
    // `bin/shard.rs`). A counter nothing ever writes reads as "this never
    // happens" rather than "nobody is counting".
    /// Admin kicks and bans that took effect (admin v0).
    pub admin_kicked: AtomicU64,
    /// Admin acts refused: not on the allowlist, an unknown verb, a target
    /// who had already left, or a full ring. **One counter for all four**,
    /// because the anomaly log carries which — a counter answers "is this
    /// happening", the log answers "what happened".
    pub admin_refused: AtomicU64,
    /// Anomaly records the log ring refused (`anomaly.rs`, wall 4's drop
    /// policy). **Deliberately not in `anomaly::WATCHED`**: a full log ring
    /// logging its own overflow is the one line guaranteed to make the
    /// overflow worse.
    pub anomaly_dropped: AtomicU64,
}

impl ShardStats {
    pub fn bump(field: &AtomicU64) {
        field.fetch_add(1, Ordering::Relaxed);
    }

    /// Add `n` to a counter in one operation. `bump` in a loop is the same
    /// number and a different cost — and for `snap_entities_shed` it is also
    /// the wrong number, because an entity can be refused twice on one pass
    /// and must be counted once (`core.rs` says where).
    pub fn add(field: &AtomicU64, n: u64) {
        field.fetch_add(n, Ordering::Relaxed);
    }

    /// Record one message on a lane: its count and its bytes, in one call.
    ///
    /// Paired on purpose. Two `add` calls at the site are the same two
    /// atomics right up until somebody adds a lane and writes one of them,
    /// and a byte total with no message count cannot tell "more packets"
    /// from "fatter packets" — which is the whole reason the byte counters
    /// come in pairs (see the lane-bytes block above).
    pub fn add_msg(count: &AtomicU64, bytes: &AtomicU64, n: usize) {
        count.fetch_add(1, Ordering::Relaxed);
        bytes.fetch_add(n as u64, Ordering::Relaxed);
    }

    pub fn get(field: &AtomicU64) -> u64 {
        field.load(Ordering::Relaxed)
    }

    /// Publish a gauge — a number that is *read off* the world each tick
    /// rather than accumulated, so it must be assigned and never bumped.
    pub fn set(field: &AtomicU64, v: u64) {
        field.store(v, Ordering::Relaxed);
    }

    /// Raise a flag. `Release`/`Acquire` rather than `Relaxed`, unlike every
    /// counter here: `store_stopped` is read to decide that *other* writes —
    /// the file the storage thread just closed — have happened, and a
    /// relaxed store would let the reader see the flag without seeing the
    /// work. A counter nobody orders anything against does not need this;
    /// a handshake does.
    pub fn raise(field: &AtomicBool) {
        field.store(true, Ordering::Release);
    }

    pub fn raised(field: &AtomicBool) -> bool {
        field.load(Ordering::Acquire)
    }

    /// A counter by its field name — the anomaly sweep's whole interface
    /// (`anomaly::WATCHED`).
    ///
    /// **A match rather than reflection, and the table below is the point**:
    /// Rust has no field lookup by string, so the alternative was for the
    /// sweep to hold references to thirty counters and for every future
    /// counter to need threading through. Here a name that does not resolve
    /// is `None` and a gate catches it
    /// (`anomaly::the_watch_list_names_real_counters`), which is a louder
    /// failure than a silently unwatched field.
    ///
    /// Only the counters worth watching are listed; load gauges are absent
    /// for the reason `WATCHED`'s doc gives.
    pub fn by_name(&self, name: &str) -> Option<&AtomicU64> {
        Some(match name {
            "ticks_dropped" => &self.ticks_dropped,
            "refused_version" => &self.refused_version,
            "refused_build" => &self.refused_build,
            "refused_auth" => &self.refused_auth,
            "refused_ticket" => &self.refused_ticket,
            "refused_full" => &self.refused_full,
            "entitle_unknown" => &self.entitle_unknown,
            "entitle_kicked" => &self.entitle_kicked,
            "handshake_errors" => &self.handshake_errors,
            "input_dg_bad" => &self.input_dg_bad,
            "input_dg_forged" => &self.input_dg_forged,
            "input_ring_drops" => &self.input_ring_drops,
            "snap_ring_skips" => &self.snap_ring_skips,
            "snap_send_errors" => &self.snap_send_errors,
            "encode_range_errors" => &self.encode_range_errors,
            "snap_entities_shed" => &self.snap_entities_shed,
            "forced_resyncs" => &self.forced_resyncs,
            "ev_resyncs" => &self.ev_resyncs,
            "ev_send_errors" => &self.ev_send_errors,
            "actions_bad" => &self.actions_bad,
            "chat_bad" => &self.chat_bad,
            "chat_rate_limited" => &self.chat_rate_limited,
            "chat_ring_drops" => &self.chat_ring_drops,
            "chat_undelivered" => &self.chat_undelivered,
            "piece_walk_restarts" => &self.piece_walk_restarts,
            "save_ring_drops" => &self.save_ring_drops,
            "saves_evicted" => &self.saves_evicted,
            "save_write_errors" => &self.save_write_errors,
            "world_save_errors" => &self.world_save_errors,
            "world_load_errors" => &self.world_load_errors,
            "sleepers_evicted" => &self.sleepers_evicted,
            "admin_kicked" => &self.admin_kicked,
            "admin_refused" => &self.admin_refused,
            _ => return None,
        })
    }
}
