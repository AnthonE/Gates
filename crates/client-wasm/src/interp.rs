//! Remote-entity interpolation (DESIGN.md §5.7, NETCODE.md §3): remote
//! players render `INTERP_DELAY_TICKS` in the past, blended between the
//! two straddling snapshot samples. Linear, not Hermite, at v0: the wire
//! carries no horizontal velocity yet (NETCODE §3's Hermite rides the M2
//! combat-feel pass with it). A buffer that runs dry freezes honestly —
//! extrapolation likewise waits on wire velocity.

use protocol::EntityState;
use sim_core::limits::MAX_SNAPSHOT_ENTITIES;
use sim_core::movement::{POS_XZ_Q, POS_Y_Q};

/// Default interpolation delay: 2 × the 66.7 ms snapshot interval
/// (NETCODE.md §3's shipped rule) = 4 sim ticks. The adaptive widening to
/// 200 ms on lossy links rides the M2 feel pass with the loss telemetry.
pub const INTERP_DELAY_TICKS: f64 = 4.0;
/// Per-entity history depth: 16 samples ≈ 1 s at 15 Hz (proposed default,
/// DECISIONS.md §open, client fill-ins). Overflow policy: drop oldest.
const HISTORY: usize = 16;

#[derive(Clone, Copy, Default)]
struct Sample {
    tick: u32,
    e: EntityState,
}

/// One interpolated remote, dequantized to render space.
#[derive(Clone, Copy, Default)]
pub struct RemoteState {
    pub id: u32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// Blended shortest-arc wire yaw, still on the 0..65536 scale.
    pub yaw: f32,
    /// Blended wire pitch, 0..255 scale.
    pub pitch: f32,
    /// False when the sample is a clamp (buffer dry or stale) — the
    /// honest-freeze flag.
    pub live: bool,
    /// Nobody is driving this body (`world.rs` `Player::sleeping`). Taken
    /// from the newer of the two samples rather than blended, because it is
    /// a fact and not a quantity: there is no halfway between asleep and
    /// awake to lerp to, and rounding one would make the flag flicker for a
    /// frame at every transition.
    pub sleeping: bool,
}

fn dequant(s: &Sample, out: &mut RemoteState) {
    out.id = s.e.id;
    out.x = s.e.qx as f32 * POS_XZ_Q;
    out.y = s.e.qy as f32 * POS_Y_Q;
    out.z = s.e.qz as f32 * POS_XZ_Q;
    out.yaw = s.e.yaw as f32;
    out.pitch = s.e.pitch as f32;
    out.sleeping = s.e.sleeping;
}

pub struct Interp {
    used: [bool; MAX_SNAPSHOT_ENTITIES],
    ids: [u32; MAX_SNAPSHOT_ENTITIES],
    hist: [[Sample; HISTORY]; MAX_SNAPSHOT_ENTITIES],
    head: [u8; MAX_SNAPSHOT_ENTITIES],
    len: [u8; MAX_SNAPSHOT_ENTITIES],
}

impl Interp {
    pub fn new() -> Self {
        Self {
            used: [false; MAX_SNAPSHOT_ENTITIES],
            ids: [0; MAX_SNAPSHOT_ENTITIES],
            hist: [[Sample::default(); HISTORY]; MAX_SNAPSHOT_ENTITIES],
            head: [0; MAX_SNAPSHOT_ENTITIES],
            len: [0; MAX_SNAPSHOT_ENTITIES],
        }
    }

    fn slot_of(&self, id: u32) -> Option<usize> {
        (0..MAX_SNAPSHOT_ENTITIES).find(|&i| self.used[i] && self.ids[i] == id)
    }

    /// One snapshot sample. Ticks arrive monotonically (the view applies
    /// only newer snapshots). A full table ignores new ids — it holds the
    /// wire's own entity cap, so the server's removals free slots first.
    pub fn push(&mut self, tick: u32, e: &EntityState) {
        let slot = match self.slot_of(e.id) {
            Some(s) => s,
            None => {
                let Some(s) = (0..MAX_SNAPSHOT_ENTITIES).find(|&i| !self.used[i]) else {
                    return;
                };
                self.used[s] = true;
                self.ids[s] = e.id;
                self.head[s] = 0;
                self.len[s] = 0;
                s
            }
        };
        let (head, len) = (self.head[slot] as usize, self.len[slot] as usize);
        let at = (head + len) % HISTORY;
        self.hist[slot][at] = Sample { tick, e: *e };
        if len == HISTORY {
            self.head[slot] = ((head + 1) % HISTORY) as u8;
        } else {
            self.len[slot] = (len + 1) as u8;
        }
    }

    pub fn remove(&mut self, id: u32) {
        if let Some(s) = self.slot_of(id) {
            self.used[s] = false;
            self.len[s] = 0;
        }
    }

    /// Zero-state snapshot: the authoritative restart of the entity set.
    pub fn clear(&mut self) {
        self.used = [false; MAX_SNAPSHOT_ENTITIES];
        self.len = [0; MAX_SNAPSHOT_ENTITIES];
    }

    pub fn ids(&self) -> impl Iterator<Item = u32> + '_ {
        (0..MAX_SNAPSHOT_ENTITIES)
            .filter(|&i| self.used[i] && self.len[i] > 0)
            .map(|i| self.ids[i])
    }

    fn get(&self, slot: usize, i: usize) -> &Sample {
        &self.hist[slot][(self.head[slot] as usize + i) % HISTORY]
    }

    /// Sample entity `id` at float server tick `at`. Between two samples:
    /// linear blend (yaw shortest-arc). Beyond the newest or before the
    /// oldest: clamp with `live = false`.
    pub fn sample(&self, id: u32, at: f64, out: &mut RemoteState) -> bool {
        let Some(slot) = self.slot_of(id) else {
            return false;
        };
        let n = self.len[slot] as usize;
        if n == 0 {
            return false;
        }
        let newest = self.get(slot, n - 1);
        if at >= newest.tick as f64 {
            dequant(newest, out);
            out.live = at <= newest.tick as f64 + f64::EPSILON;
            return true;
        }
        let oldest = self.get(slot, 0);
        if at <= oldest.tick as f64 {
            dequant(oldest, out);
            out.live = false;
            return true;
        }
        for i in (1..n).rev() {
            let s1 = self.get(slot, i);
            let s0 = self.get(slot, i - 1);
            if at >= s0.tick as f64 {
                let span = (s1.tick - s0.tick) as f64;
                let t = ((at - s0.tick as f64) / span) as f32;
                let mut a = RemoteState::default();
                let mut b = RemoteState::default();
                dequant(s0, &mut a);
                dequant(s1, &mut b);
                out.id = a.id;
                out.x = a.x + (b.x - a.x) * t;
                out.y = a.y + (b.y - a.y) * t;
                out.z = a.z + (b.z - a.z) * t;
                let mut dyaw = s1.e.yaw as i32 - s0.e.yaw as i32;
                if dyaw > 32767 {
                    dyaw -= 65536;
                } else if dyaw < -32768 {
                    dyaw += 65536;
                }
                let mut yaw = a.yaw + dyaw as f32 * t;
                if yaw < 0.0 {
                    yaw += 65536.0;
                } else if yaw >= 65536.0 {
                    yaw -= 65536.0;
                }
                out.yaw = yaw;
                out.pitch = a.pitch + (b.pitch - a.pitch) * t;
                // The newer sample's, not a blend: `sleeping` is a fact
                // about the body, and the two facts either side of `t` are
                // "awake" and "asleep" with nothing between them. Taking
                // `b` means the client is asleep for the interpolation
                // window rather than awake for it, which is the safe
                // direction — the alternative draws a body as a player for
                // an extra ~100 ms after the server stopped treating it as
                // one.
                out.sleeping = s1.e.sleeping;
                out.live = true;
                return true;
            }
        }
        false
    }
}

impl Default for Interp {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ent(id: u32, qx: i32, yaw: u16) -> EntityState {
        EntityState {
            id,
            qx,
            qy: 500,
            qz: 40_000,
            qvy: 0,
            grounded: true,
            sleeping: false,
            yaw,
            pitch: 100,
        }
    }

    #[test]
    fn lerps_between_straddling_samples() {
        let mut it = Interp::new();
        it.push(10, &ent(5, 1000, 0));
        it.push(12, &ent(5, 2000, 0));
        let mut r = RemoteState::default();
        assert!(it.sample(5, 11.0, &mut r));
        assert!(r.live);
        assert!((r.x - 1500.0 * 0.03).abs() < 1e-3);
    }

    #[test]
    fn yaw_blends_the_short_way_around_the_wrap() {
        let mut it = Interp::new();
        it.push(10, &ent(5, 0, 0xFFF0));
        it.push(12, &ent(5, 0, 0x0010));
        let mut r = RemoteState::default();
        assert!(it.sample(5, 11.0, &mut r));
        // Midpoint of the 32-step short arc across zero.
        assert!(r.yaw > 65520.0 || r.yaw < 16.0, "yaw {}", r.yaw);
    }

    #[test]
    fn dry_buffer_freezes_honestly() {
        let mut it = Interp::new();
        it.push(10, &ent(5, 1000, 0));
        it.push(12, &ent(5, 2000, 0));
        let mut r = RemoteState::default();
        assert!(it.sample(5, 30.0, &mut r));
        assert!(!r.live, "clamped to newest must not read live");
        assert!((r.x - 2000.0 * 0.03).abs() < 1e-3);
    }

    #[test]
    fn remove_and_clear_forget_entities() {
        let mut it = Interp::new();
        it.push(10, &ent(5, 1000, 0));
        it.push(10, &ent(6, 1000, 0));
        it.remove(5);
        let mut r = RemoteState::default();
        assert!(!it.sample(5, 10.0, &mut r));
        assert_eq!(it.ids().count(), 1);
        it.clear();
        assert_eq!(it.ids().count(), 0);
    }

    #[test]
    fn history_overflow_drops_oldest() {
        let mut it = Interp::new();
        for k in 0..20u32 {
            it.push(k * 2, &ent(5, k as i32 * 100, 0));
        }
        let mut r = RemoteState::default();
        // Tick 0 fell out of the 16-deep ring; the oldest held is 8.
        assert!(it.sample(5, 0.0, &mut r));
        assert!(!r.live);
        assert!((r.x - 400.0 * 0.03).abs() < 1e-3);
    }
}
