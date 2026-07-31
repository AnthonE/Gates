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
}

impl ShardStats {
    pub fn bump(field: &AtomicU64) {
        field.fetch_add(1, Ordering::Relaxed);
    }

    pub fn get(field: &AtomicU64) -> u64 {
        field.load(Ordering::Relaxed)
    }
}
