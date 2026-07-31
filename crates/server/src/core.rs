//! ShardCore: the sim thread's whole world — `sim_core::World` plus every
//! client's netcode state, AOI, priority fill, and snapshot encoding into
//! caller-provided sends. Pure: no I/O, no clock, no locks; the net layer
//! drives it through rings and it allocates only in `new` (L1–L4). Tests
//! drive it directly, no sockets required.

use crate::client::ClientNetState;
use crate::stats::ShardStats;
use protocol::{EntityState, InputDatagram, SnapshotEncoder, SnapshotHeader, WireError};
use sim_core::limits::{
    AOI_ENTER_CM, AOI_EXIT_CM, DATAGRAM_BUDGET_BYTES, MAX_COMMANDS_PER_TICK, MAX_PLAYERS,
    MAX_SNAPSHOT_ENTITIES, SNAPSHOT_INTERVAL_TICKS, STALENESS_CEILING,
};
use sim_core::world::{Command, Player, World};

/// Priority accumulator v0 weights (NETCODE.md §3): players w=100; the
/// distance falloff half-scale is 32 m. Other classes land with their
/// entities.
const PRIORITY_W_PLAYER: f32 = 100.0;
const PRIORITY_HALF_SCALE_M: f32 = 32.0;

/// Consecutive byte-overflow refusals before the fill loop stops trying
/// smaller records (bounded work per snapshot, not a wire number).
const FILL_OVERFLOW_STREAK: u32 = 3;

pub struct ShardCore {
    pub world: World,
    pub clients: Box<[ClientNetState]>,
    /// Joins/leaves queued between ticks (accept/cleanup driven). Overflow
    /// policy: refuse — the caller retries next tick.
    queued: [Command; MAX_COMMANDS_PER_TICK],
    queued_len: usize,
    /// Scratch: baseline copy (borrow-splits the client during encode).
    baseline_buf: [EntityState; MAX_SNAPSHOT_ENTITIES],
    /// Scratch: what actually got encoded, for `record_sent`.
    sent_buf: [EntityState; MAX_SNAPSHOT_ENTITIES],
    removed_buf: [u32; MAX_SNAPSHOT_ENTITIES],
    /// Scratch: encode target; the closure receives its bytes.
    dg_buf: [u8; DATAGRAM_BUDGET_BYTES],
}

impl ShardCore {
    pub fn new(seed: u64) -> Self {
        let mut clients = Vec::with_capacity(MAX_PLAYERS);
        clients.resize_with(MAX_PLAYERS, ClientNetState::new);
        Self {
            world: World::new(seed),
            clients: clients.into_boxed_slice(),
            queued: [Command::Leave { id: 0 }; MAX_COMMANDS_PER_TICK],
            queued_len: 0,
            baseline_buf: [EntityState::default(); MAX_SNAPSHOT_ENTITIES],
            sent_buf: [EntityState::default(); MAX_SNAPSHOT_ENTITIES],
            removed_buf: [0; MAX_SNAPSHOT_ENTITIES],
            dg_buf: [0; DATAGRAM_BUDGET_BYTES],
        }
    }

    fn queue(&mut self, cmd: Command) -> bool {
        // Half the command budget is reserved for the per-tick inputs.
        if self.queued_len >= MAX_COMMANDS_PER_TICK - MAX_PLAYERS {
            return false;
        }
        self.queued[self.queued_len] = cmd;
        self.queued_len += 1;
        true
    }

    /// Install a client on `slot` with player `id`. False ⇒ retry next
    /// tick (queue full — refuse, never grow).
    #[must_use]
    pub fn connect(&mut self, slot: usize, id: u32) -> bool {
        if !self.queue(Command::Join { id }) {
            return false;
        }
        self.clients[slot].reset(id);
        true
    }

    /// Tear a client down. The world keeps no sleeper yet (M0): leave
    /// removes the entity; sleepers arrive with their milestone.
    pub fn disconnect(&mut self, slot: usize) {
        let id = self.clients[slot].id;
        if self.clients[slot].connected {
            // Queue overflow here would strand the world entity; the
            // reserve (MAX_PLAYERS of headroom) makes that impossible for
            // real leave rates.
            let _ = self.queue(Command::Leave { id });
            self.clients[slot].connected = false;
        }
    }

    /// One decoded input datagram from this client: acks first (they ride
    /// every datagram), then the frame tail into the seq buffer.
    pub fn push_input(&mut self, slot: usize, dg: &InputDatagram) {
        let c = &mut self.clients[slot];
        if !c.connected {
            return;
        }
        c.on_acks(dg.snapshot_ack, dg.ack_bits);
        for f in dg.frames() {
            c.push_frame(*f);
        }
    }

    /// One fixed tick: queued joins/leaves + one consumed input per client
    /// → `World::tick`, then interest/priority accrual, then — on the
    /// 15 Hz cadence — one encoded snapshot per connected client handed to
    /// `send(slot, bytes)`.
    pub fn tick(&mut self, stats: &ShardStats, mut send: impl FnMut(usize, &[u8])) {
        let mut cmds = [Command::Leave { id: 0 }; MAX_COMMANDS_PER_TICK];
        let mut n = self.queued_len;
        cmds[..n].copy_from_slice(&self.queued[..n]);
        self.queued_len = 0;
        for slot in 0..MAX_PLAYERS {
            let c = &mut self.clients[slot];
            if !c.connected {
                continue;
            }
            if let Some(frame) = c.consume_input() {
                if n < MAX_COMMANDS_PER_TICK {
                    cmds[n] = Command::Input { id: c.id, frame };
                    n += 1;
                }
            }
        }
        self.world.tick(&cmds[..n]);

        for slot in 0..MAX_PLAYERS {
            if self.clients[slot].connected {
                self.update_interest(slot, stats);
            }
        }

        if self.world.tick.is_multiple_of(SNAPSHOT_INTERVAL_TICKS) {
            for slot in 0..MAX_PLAYERS {
                if !self.clients[slot].connected {
                    continue;
                }
                if let Some(len) = self.encode_snapshot(slot, stats) {
                    ShardStats::bump(&stats.snap_sent);
                    send(slot, &self.dg_buf[..len]);
                }
            }
        }
    }

    /// Resolve the world slot player `id` landed in (join order is
    /// deterministic but not slot-aligned with connections).
    fn world_slot_of(world: &World, id: u32) -> Option<usize> {
        world.players.iter().position(|p| p.active && p.id == id)
    }

    /// AOI v0 (DESIGN.md §5.5, radius-only): planar hysteresis band, enter
    /// 176 m / exit 208 m, plus the NETCODE.md §3 priority accrual for
    /// everything inside. Entities leaving the client's world (range,
    /// disconnect, or slot reuse) go to the pending-removal set until an
    /// acked snapshot covers them.
    fn update_interest(&mut self, slot: usize, stats: &ShardStats) {
        let c = &mut self.clients[slot];
        if c.own_wslot == usize::MAX
            || !self.world.players[c.own_wslot].active
            || self.world.players[c.own_wslot].id != c.id
        {
            c.own_wslot = match Self::world_slot_of(&self.world, c.id) {
                Some(w) => w,
                None => return, // join command still queued
            };
        }
        let own = self.world.players[c.own_wslot].body;
        let mut overflow = false;
        for w in 0..MAX_PLAYERS {
            let p = &self.world.players[w];
            let live = p.active && p.id != c.id;
            if !live || c.tracked_id[w] != p.id {
                // Tenant left or changed: the id the client knew is gone.
                if c.interest[w] {
                    overflow |= !c.pending_add(c.tracked_id[w]);
                    c.interest[w] = false;
                }
                c.accum[w] = 0.0;
                c.unsent[w] = 0;
                c.tracked_id[w] = if live { p.id } else { 0 };
                if !live {
                    continue;
                }
            }
            let dx = (p.body.qx - own.qx) as i64 * 3;
            let dz = (p.body.qz - own.qz) as i64 * 3;
            let d2 = dx * dx + dz * dz;
            if c.interest[w] {
                if d2 > AOI_EXIT_CM * AOI_EXIT_CM {
                    c.interest[w] = false;
                    c.accum[w] = 0.0;
                    c.unsent[w] = 0;
                    overflow |= !c.pending_add(p.id);
                }
            } else if d2 <= AOI_ENTER_CM * AOI_ENTER_CM {
                c.interest[w] = true;
                c.accum[w] = 0.0;
                c.unsent[w] = 0;
                c.pending_remove(p.id);
            }
            if c.interest[w] {
                let d_m = ((d2 as f32).sqrt()) * 0.01;
                c.accum[w] += PRIORITY_W_PLAYER / (1.0 + d_m / PRIORITY_HALF_SCALE_M);
            }
        }
        if overflow {
            c.force_resync();
            ShardStats::bump(&stats.forced_resyncs);
        }
    }

    fn wire_entity(p: &Player) -> EntityState {
        EntityState {
            id: p.id,
            qx: p.body.qx,
            qy: p.body.qy,
            qz: p.body.qz,
            qvy: p.body.qvy,
            grounded: p.body.grounded,
            yaw: p.frame.yaw,
            pitch: p.frame.pitch,
        }
    }

    /// Priority-filled snapshot for one client (DESIGN.md §5.5): removals
    /// first, then own entity, then stale-preempted and accumulator-ranked
    /// interest entities until budget or cap. Returns the encoded length,
    /// or None only on an encoder-refusal bug (counted, never panicking).
    fn encode_snapshot(&mut self, slot: usize, stats: &ShardStats) -> Option<usize> {
        let tick = self.world.tick;
        let c = &mut self.clients[slot];
        if c.own_wslot == usize::MAX {
            return None; // not in the world yet: nothing to snapshot
        }

        let (age, baseline_len) = match c.baseline(tick) {
            Some((age, snap)) => {
                let n = snap.entity_count as usize;
                self.baseline_buf[..n].copy_from_slice(snap.entities());
                (age, n)
            }
            None => (0, 0),
        };
        let header = SnapshotHeader {
            tick: tick as u32,
            baseline_age: age,
            last_executed_seq: c.last_executed,
            nudge: c.nudge,
        };
        let mut enc = match SnapshotEncoder::begin(
            &mut self.dg_buf,
            &header,
            &self.baseline_buf[..baseline_len],
        ) {
            Ok(enc) => enc,
            Err(_) => {
                ShardStats::bump(&stats.encode_range_errors);
                return None;
            }
        };

        // Removals — only against a real baseline: a zero-state snapshot
        // clears the client's class-D map by definition (Q3 semantics), so
        // removal ids would be wasted bytes there.
        let mut n_removed = 0usize;
        if age > 0 {
            for i in 0..c.pending().len() {
                let id = c.pending()[i];
                match enc.add_removed(id) {
                    Ok(()) => {
                        self.removed_buf[n_removed] = id;
                        n_removed += 1;
                    }
                    Err(_) => break, // cap/budget: the rest stay pending
                }
            }
        }

        // Candidates: own entity first (reconciliation needs it every
        // snapshot), then interest by (stale-preempt, accumulator).
        let mut order: [(u16, f32, bool); MAX_PLAYERS] = [(0, 0.0, false); MAX_PLAYERS];
        let mut n_cand = 0usize;
        for w in 0..MAX_PLAYERS {
            if c.interest[w] && self.world.players[w].active {
                order[n_cand] = (w as u16, c.accum[w], c.unsent[w] >= STALENESS_CEILING - 1);
                n_cand += 1;
            }
        }
        order[..n_cand]
            .sort_unstable_by(|a, b| b.2.cmp(&a.2).then(b.1.total_cmp(&a.1)).then(a.0.cmp(&b.0)));

        let mut n_sent = 0usize;
        let own = Self::wire_entity(&self.world.players[c.own_wslot]);
        match enc.add_entity(&own) {
            Ok(()) => {
                self.sent_buf[n_sent] = own;
                n_sent += 1;
            }
            Err(_) => {
                ShardStats::bump(&stats.encode_range_errors);
                return None;
            }
        }
        let mut sent_mask = [false; MAX_PLAYERS];
        let mut overflow_streak = 0u32;
        for &(w, _, _) in order[..n_cand].iter() {
            let w = w as usize;
            let e = Self::wire_entity(&self.world.players[w]);
            match enc.add_entity(&e) {
                Ok(()) => {
                    self.sent_buf[n_sent] = e;
                    n_sent += 1;
                    sent_mask[w] = true;
                    overflow_streak = 0;
                }
                Err(WireError::Overflow) => {
                    overflow_streak += 1;
                    if overflow_streak >= FILL_OVERFLOW_STREAK {
                        break;
                    }
                }
                Err(WireError::Cap) => break,
                Err(_) => {
                    ShardStats::bump(&stats.encode_range_errors);
                }
            }
        }

        let len = match enc.finish() {
            Ok(len) => len,
            Err(_) => {
                ShardStats::bump(&stats.encode_range_errors);
                return None;
            }
        };

        for (w, &was_sent) in sent_mask.iter().enumerate() {
            if !c.interest[w] {
                continue;
            }
            if was_sent {
                c.accum[w] = 0.0;
                c.unsent[w] = 0;
            } else {
                c.unsent[w] = c.unsent[w].saturating_add(1);
            }
        }
        c.record_sent(
            tick,
            &self.sent_buf[..n_sent],
            &self.removed_buf[..n_removed],
        );
        Some(len)
    }
}
