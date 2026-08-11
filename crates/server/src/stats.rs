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
}
