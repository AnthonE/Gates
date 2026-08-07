//! Shard counters: plain atomics, written from any thread (an atomic store
//! is not a syscall, a lock, or an allocation — sim-thread safe), read by
//! tests, the smoke gate, and later the status page. Integer-only by
//! design (L5: diagnostics are numbers, not strings).

use std::sync::atomic::{AtomicU64, Ordering};

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
    pub refused_version: AtomicU64,
    /// Joins refused for want of a good session (`REFUSE_AUTH`). Counted
    /// separately from `refused_version` because the two mean opposite
    /// things about a shard: a version refusal is a client that needs
    /// updating, an auth refusal is a shard doing its job.
    pub refused_auth: AtomicU64,
    pub refused_full: AtomicU64,
    pub handshake_errors: AtomicU64,
    /// Input datagrams decoded and ringed.
    pub input_dg_ok: AtomicU64,
    /// Input datagrams that failed to decode (client-driven bytes; never a
    /// panic, always a count).
    pub input_dg_bad: AtomicU64,
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
    /// Sleeping bodies the world deleted to seat a join (`world.rs` `seat`).
    /// Mirrored out of `World::evictions`, which is where the policy lives;
    /// this is the operator-facing copy. Nonzero means the shard is past
    /// `MAX_PLAYERS` distinct recent visitors and somebody came back to a
    /// record instead of a body.
    pub sleepers_evicted: AtomicU64,
    /// Bodies asleep in the world right now — a gauge, not a counter, set
    /// each publish rather than bumped.
    pub sleepers: AtomicU64,
    /// Records handed to the store's index — a leave, or the autosave sweep
    /// finding a player whose state moved. Those are the only two producers:
    /// there is no shutdown flush (`NOW.md` §0y item 3 says why not).
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

    pub fn get(field: &AtomicU64) -> u64 {
        field.load(Ordering::Relaxed)
    }

    /// Publish a gauge — a number that is *read off* the world each tick
    /// rather than accumulated, so it must be assigned and never bumped.
    pub fn set(field: &AtomicU64, v: u64) {
        field.store(v, Ordering::Relaxed);
    }
}
