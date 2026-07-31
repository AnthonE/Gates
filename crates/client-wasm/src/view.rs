//! The client side of the snapshot pipeline, pure (no I/O): apply
//! datagrams, keep the applied-snapshot ring the delta baselines come
//! from, produce ack fields. Lives in `client-wasm` so the browser (via
//! the wasm bridge), the server's bot client, and the smoke/budget gates
//! all reconstruct through the one implementation — the load tool can
//! never drift from the thing it load-tests.
//!
//! Contract highlights (NETCODE.md §3 + protocol doc):
//! - apply only monotonically newer snapshots; discard stale, never ack it;
//! - a zero-state snapshot (`baseline_age == 0`) **clears the class-D
//!   map** — it is the authoritative restart of the entity set;
//! - the delta baseline is the *decoded content* of the applied snapshot
//!   at `tick − baseline_age`, byte-identical to what the server recorded.

use protocol::{decode_snapshot, peek_snapshot_header, EntityState, Nudge, Snapshot, WireError};
use sim_core::limits::{SENT_SNAPSHOT_RING, SNAPSHOT_INTERVAL_TICKS};

fn ring_index(tick: u32) -> usize {
    (tick as u64 / SNAPSHOT_INTERVAL_TICKS) as usize % SENT_SNAPSHOT_RING
}

#[derive(Debug, PartialEq, Eq)]
pub enum Applied {
    /// Snapshot applied; `delta` says whether it rode a baseline.
    Ok { delta: bool },
    /// Older than the newest applied — discarded, not acked.
    Stale,
    /// Named a baseline we no longer hold (anomaly: the server only
    /// baselines snapshots we acked, and our ring matches its depth).
    NoBaseline,
}

pub struct ClientView {
    ring: Box<[Option<Snapshot>]>,
    /// The render map: freshest known state per entity id.
    pub entities: Vec<(u32, EntityState)>,
    pub newest_applied: Option<u32>,
    pub last_executed_seq: u16,
    pub nudge: Nudge,
}

impl ClientView {
    pub fn new() -> Self {
        let mut ring = Vec::with_capacity(SENT_SNAPSHOT_RING);
        ring.resize_with(SENT_SNAPSHOT_RING, || None);
        Self {
            ring: ring.into_boxed_slice(),
            entities: Vec::with_capacity(128),
            newest_applied: None,
            last_executed_seq: 0,
            nudge: Nudge::Ok,
        }
    }

    pub fn get(&self, id: u32) -> Option<&EntityState> {
        self.entities.iter().find(|(e, _)| *e == id).map(|(_, s)| s)
    }

    /// Apply one snapshot datagram.
    pub fn apply(&mut self, bytes: &[u8]) -> Result<Applied, WireError> {
        let header = peek_snapshot_header(bytes)?;
        if let Some(newest) = self.newest_applied {
            if header.tick <= newest {
                return Ok(Applied::Stale);
            }
        }
        let baseline_tick = header.tick.wrapping_sub(header.baseline_age as u32);
        let snap = if header.baseline_age == 0 {
            decode_snapshot(bytes, &[])?
        } else {
            let Some(base) = self.ring[ring_index(baseline_tick)]
                .as_ref()
                .filter(|s| s.header.tick == baseline_tick)
            else {
                return Ok(Applied::NoBaseline);
            };
            decode_snapshot(bytes, base.entities())?
        };

        if header.baseline_age == 0 {
            self.entities.clear();
        }
        for &id in snap.removed() {
            self.entities.retain(|(e, _)| *e != id);
        }
        for e in snap.entities() {
            match self.entities.iter_mut().find(|(id, _)| *id == e.id) {
                Some((_, s)) => *s = *e,
                None => self.entities.push((e.id, *e)),
            }
        }
        self.newest_applied = Some(header.tick);
        self.last_executed_seq = header.last_executed_seq;
        self.nudge = header.nudge;
        self.ring[ring_index(header.tick)] = Some(snap);
        Ok(Applied::Ok {
            delta: header.baseline_age != 0,
        })
    }

    /// The most recently applied snapshot — the interpolation feed reads
    /// its decoded entities right after `apply` returns `Ok`.
    pub fn newest(&self) -> Option<&Snapshot> {
        let t = self.newest_applied?;
        self.ring[ring_index(t)]
            .as_ref()
            .filter(|s| s.header.tick == t)
    }

    /// The redundant ack header for the next input datagram
    /// (NETCODE.md §3): newest applied tick + 32 bits of applied history.
    pub fn ack_fields(&self) -> (u16, u32) {
        let Some(newest) = self.newest_applied else {
            return (0, 0);
        };
        let mut bits = 0u32;
        for n in 1..=32u32 {
            let t = newest.wrapping_sub(n);
            let held = self.ring[ring_index(t)]
                .as_ref()
                .is_some_and(|s| s.header.tick == t);
            if held {
                bits |= 1 << (n - 1);
            }
        }
        (newest as u16, bits)
    }
}

impl Default for ClientView {
    fn default() -> Self {
        Self::new()
    }
}
