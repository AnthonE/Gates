//! Remote-entity interpolation (DESIGN.md §5.7, NETCODE.md §3): remote
//! players render `limits::INTERP_DELAY_TICKS` in the past, blended between
//! the two straddling snapshot samples.
//!
//! **The delay itself is not declared here, and used to be.** This file
//! carried its own `f64` constant, hand-typed as `4.0`, until
//! lag compensation needed the same number on the *server* — the aim
//! staleness a rewind is minted from is `(T - S) + INTERP_DELAY_TICKS -
//! REWIND_ACK_BIAS_TICKS` (`findings/lagcomp-design-20260818.md` §2.2) — and
//! two crates typing one constant is the `props.js` drift `CLAUDE.md` opens
//! with. It lives in `sim_core::limits` now, as the `u8` it is, and this
//! module's one reader converts at the point of use. There is no second
//! spelling to keep in step, which is stronger than a gate on one. Linear, not Hermite, at v0: the wire
//! carries no horizontal velocity yet (NETCODE §3's Hermite rides the M2
//! combat-feel pass with it). A buffer that runs dry freezes honestly —
//! extrapolation likewise waits on wire velocity.

use protocol::EntityState;
use sim_core::limits::{MAX_MOBS, MAX_PLAYERS};
use sim_core::movement::{POS_XZ_Q, POS_Y_Q};

/// Per-entity history depth: 16 samples ≈ 0.5 s at 30 Hz (proposed
/// default, DECISIONS.md §open, client fill-ins). Overflow policy: drop
/// oldest.
const HISTORY: usize = 16;

/// The widest tick gap `sample` will lerp across (netcode v2 S2). Wider
/// straddles — a stall, a loss burst past the history, the far side of a
/// keyframe that kept this body's ring — read as a teleport and take the
/// newer sample, because a lerp across a gap is a body sliding through
/// walls at gap-speed. Twice `INTERP_DELAY_TICKS`: anything the playout
/// delay was sized to ride out lerps as before, anything past double it
/// is not motion the buffer ever promised to reconstruct.
const MAX_LERP_SPAN_TICKS: u32 = sim_core::limits::INTERP_DELAY_TICKS as u32 * 2;

/// Entities this table can hold at once — **the world's size, not the
/// wire's**, and the distinction is the whole reason the constant exists.
///
/// This table was sized `MAX_SNAPSHOT_ENTITIES` and is filled
/// *cumulatively*: a zero-state snapshot clears it and every snapshot after
/// that adds whatever ids it names, so what it holds is the union over many
/// datagrams while `MAX_SNAPSHOT_ENTITIES` bounds one of them. The server
/// now rank-caps the interest set to that same wire count
/// (`limits::AOI_RANK_EXIT`), so the union is bounded too — but the client
/// may not *depend* on that, because it cannot check it and because the day
/// the cap is raised the failure is silent and permanent: `push` refuses
/// the id, `ClientView` keeps it, and `render/bodies.rs` and
/// `render/mobs.rs` spawn only what `ids()` reports. The body sits inside
/// AOI, on the client's own map, and is never drawn.
///
/// So the client sizes for what the world can hold and lets the server's
/// cap be the one that binds. `MAX_PLAYERS + MAX_MOBS` is every class-D
/// entity a shard can have at once; nothing can be pushed that is not one
/// of them, so overflow here is unreachable rather than merely unlikely —
/// and it is still counted (`drops`), because wall 4 wants the policy
/// stated and an unreachable branch that silently discards a body is how
/// this bug got here in the first place.
pub const INTERP_SLOTS: usize = MAX_PLAYERS + MAX_MOBS;

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
    /// This body has been killed and has not respawned (`world.rs`
    /// `Player::dead`). Taken from the newer sample for `sleeping`'s reason
    /// — it is a fact, not a quantity — and in the same direction: a body
    /// reads as a corpse for the interpolation window rather than as a
    /// player for an extra ~100 ms after the server stopped treating it as
    /// one.
    pub dead: bool,
    /// **What this body is holding** — the content item id in its selected
    /// hotbar slot, or `None` for an empty hand (`protocol::EntityState`,
    /// wire v56). Taken from the newer sample for `sleeping`'s reason: an
    /// id is an identity, not a quantity, and there is no item halfway
    /// between a bow and a hatchet to lerp to.
    pub held: Option<u16>,
    /// That item is alight (`sim-core/light.rs` `is_lit`, resolved on the
    /// server). Newer sample again, and in the same direction the other
    /// two facts go: the flame goes out at the start of the window rather
    /// than burning for an extra ~100 ms after the sim put it out.
    pub lit: bool,
}

fn dequant(s: &Sample, out: &mut RemoteState) {
    out.id = s.e.id;
    out.x = s.e.qx as f32 * POS_XZ_Q;
    out.y = s.e.qy as f32 * POS_Y_Q;
    out.z = s.e.qz as f32 * POS_XZ_Q;
    out.yaw = s.e.yaw as f32;
    out.pitch = s.e.pitch as f32;
    out.sleeping = s.e.sleeping;
    out.dead = s.e.dead;
    out.held = s.e.held;
    out.lit = s.e.lit;
}

pub struct Interp {
    used: [bool; INTERP_SLOTS],
    ids: [u32; INTERP_SLOTS],
    /// The sample rings, one per slot — **boxed, and filled on the heap.**
    /// At `INTERP_SLOTS × HISTORY` samples this is ~86 kB, and
    /// `Box::new([[..]])` materialises the whole thing in the caller's
    /// frame before moving it. That is the shadow-stack trap `sim-core`'s
    /// `boxed_array` exists for (CLAUDE.md §traps, measured three times on
    /// 2026-08-08), and while `client-core` compiles native-only today,
    /// `ClientCore::new` is called from a Bevy startup system that has no
    /// more frame to spare than a wasm one. `vec!` allocates and fills
    /// where the allocation was going to happen anyway.
    hist: Box<[[Sample; HISTORY]; INTERP_SLOTS]>,
    head: [u8; INTERP_SLOTS],
    len: [u8; INTERP_SLOTS],
    /// Samples refused because the table was full — wall 4's stated
    /// overflow policy, counted rather than silent. It must read zero on
    /// any shard whose interest cap is honest; a non-zero reading is a body
    /// the client knows about and cannot draw, which is exactly the defect
    /// `tests/interp_capacity.rs` was written for.
    pub drops: u64,
}

impl Interp {
    pub fn new() -> Self {
        let Ok(hist) = vec![[Sample::default(); HISTORY]; INTERP_SLOTS]
            .into_boxed_slice()
            .try_into()
        else {
            unreachable!("the vec is INTERP_SLOTS long by construction")
        };
        Self {
            used: [false; INTERP_SLOTS],
            ids: [0; INTERP_SLOTS],
            hist,
            head: [0; INTERP_SLOTS],
            len: [0; INTERP_SLOTS],
            drops: 0,
        }
    }

    fn slot_of(&self, id: u32) -> Option<usize> {
        (0..INTERP_SLOTS).find(|&i| self.used[i] && self.ids[i] == id)
    }

    /// One snapshot sample. Ticks arrive monotonically (the view applies
    /// only newer snapshots).
    ///
    /// A full table counts the id and drops it. It holds every class-D
    /// entity a shard can have at once (`INTERP_SLOTS`), so nothing the
    /// server can legally send fills it — the branch is a bound, not a
    /// working path. It used to hold `MAX_SNAPSHOT_ENTITIES`, which is a
    /// *per-datagram* count against a table filled across datagrams, and
    /// the comment here claimed the server's removals would free slots
    /// first. They do not: an entity that never leaves the interest set
    /// never generates a removal, so the 65th distinct id since the last
    /// zero-state was refused for the rest of the session.
    pub fn push(&mut self, tick: u32, e: &EntityState) {
        let slot = match self.slot_of(e.id) {
            Some(s) => s,
            None => {
                let Some(s) = (0..INTERP_SLOTS).find(|&i| !self.used[i]) else {
                    self.drops += 1;
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

    /// Zero-state snapshot (netcode v2 S2): the keyframe restarts the
    /// ENTITY SET, not the motion history. This drops every id the
    /// keyframe does not name and keeps the sample rings of the ones it
    /// does — `clear()` here wiped them all, so every ack gap froze every
    /// remote body for a snapshot interval and then teleported it, which
    /// is most of what "players snap around the screen" was. The
    /// positions already held are not made wrong by a baseline reset:
    /// that reset is about delta-coding, and the interpolator never
    /// delta-codes. Bounded at `INTERP_SLOTS × MAX_SNAPSHOT_ENTITIES`
    /// compares, keyframes only.
    pub fn retain_present(&mut self, present: &[EntityState]) {
        for s in 0..INTERP_SLOTS {
            if self.used[s] && !present.iter().any(|e| e.id == self.ids[s]) {
                self.used[s] = false;
                self.len[s] = 0;
            }
        }
    }

    /// Full restart — session start and hard resync, where the history
    /// genuinely is from another life. Keyframes use `retain_present`.
    pub fn clear(&mut self) {
        self.used = [false; INTERP_SLOTS];
        self.len = [0; INTERP_SLOTS];
    }

    pub fn ids(&self) -> impl Iterator<Item = u32> + '_ {
        (0..INTERP_SLOTS)
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
                // A straddle wider than the buffer's own design depth is
                // not motion — it is a gap (a stall, a stretch of loss, a
                // respawn) — and lerping across it would slide the body
                // through the world at whatever speed the gap implies.
                // Take the newer sample outright: a teleport reads as a
                // teleport (the reference fixed the same "snapback near
                // your corpse" defect, devblog 74). `live` stays true —
                // the sample is real, only the path to it is unknown.
                if s1.tick - s0.tick > MAX_LERP_SPAN_TICKS {
                    dequant(s1, out);
                    out.live = true;
                    return true;
                }
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
                out.dead = s1.e.dead;
                out.held = s1.e.held;
                out.lit = s1.e.lit;
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
            dead: false,
            yaw,
            pitch: 100,
            held: None,
            lit: false,
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

    /// **The hand is a fact, so it is taken and not blended** — wire v56,
    /// `sleeping`'s rule applied to an id and a bit.
    ///
    /// There is no item halfway between a torch and a hatchet, so a
    /// midpoint sample must land on the NEWER one in both directions: a
    /// body that just raised a weapon is drawn holding it for the
    /// interpolation window rather than empty-handed for another ~100 ms,
    /// and a flame that just died is dark for that window rather than
    /// burning. Both directions are asserted, because "take the newer"
    /// and "take the one that is set" are the same code on half the cases.
    #[test]
    fn the_hand_comes_from_the_newer_sample_in_both_directions() {
        let lit = |held, lit| {
            let mut e = ent(5, 1000, 0);
            e.held = held;
            e.lit = lit;
            e
        };
        // Empty → torch alight: the midpoint already has it.
        let mut it = Interp::new();
        it.push(10, &lit(None, false));
        it.push(12, &lit(Some(3), true));
        let mut r = RemoteState::default();
        assert!(it.sample(5, 11.0, &mut r));
        assert_eq!(r.held, Some(3));
        assert!(r.lit);
        // Torch alight → put out: the midpoint is already dark, and the
        // stick is still in the hand.
        let mut it = Interp::new();
        it.push(10, &lit(Some(3), true));
        it.push(12, &lit(Some(3), false));
        let mut r = RemoteState::default();
        assert!(it.sample(5, 11.0, &mut r));
        assert_eq!(r.held, Some(3));
        assert!(!r.lit, "a blended bit would round the flame back on");
        // And a clamp (buffer dry) carries it too, not just the blend —
        // the two branches copy the record separately.
        let mut it = Interp::new();
        it.push(10, &lit(Some(3), true));
        let mut r = RemoteState::default();
        assert!(it.sample(5, 99.0, &mut r));
        assert!(!r.live, "one sample past its window is a clamp");
        assert_eq!(r.held, Some(3));
        assert!(r.lit);
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

    /// **A keyframe keeps the motion history of everyone it names**
    /// (netcode v2 S2). The old `clear()`-on-keyframe wiped every sample
    /// ring, so the recovery packet after an ack gap was itself the freeze
    /// -then-teleport the player saw. `retain_present` drops only the ids
    /// the keyframe does not name.
    #[test]
    fn a_keyframe_keeps_named_history_and_drops_the_rest() {
        let mut it = Interp::new();
        it.push(10, &ent(5, 1000, 0));
        it.push(12, &ent(5, 2000, 0));
        it.push(12, &ent(9, 4000, 0));
        // The keyframe names 5 and not 9.
        it.retain_present(&[ent(5, 2000, 0)]);
        let mut r = RemoteState::default();
        assert!(
            it.sample(5, 11.0, &mut r) && r.live,
            "the named body lost its history to the keyframe"
        );
        assert!(
            (r.x - 1500.0 * 0.03).abs() < 1e-3,
            "history intact, mid-lerp"
        );
        assert!(!it.sample(9, 12.0, &mut r), "the unnamed body is gone");
        assert_eq!(it.ids().count(), 1);
    }

    /// **A straddle wider than the buffer's design depth is a teleport,
    /// not motion** — the newer sample wins outright instead of the body
    /// sliding across the gap at gap-speed (a respawn would glide from
    /// the corpse to the beach through every wall between).
    #[test]
    fn a_wide_gap_jumps_instead_of_sliding() {
        let mut it = Interp::new();
        it.push(10, &ent(5, 1000, 0));
        it.push(10 + MAX_LERP_SPAN_TICKS + 5, &ent(5, 50_000, 0));
        let mut r = RemoteState::default();
        // Sampled mid-gap: the lerp would put the body in the void between.
        assert!(it.sample(5, 12.0, &mut r));
        assert!(r.live, "a real sample is live even across a gap");
        assert!(
            (r.x - 50_000.0 * 0.03).abs() < 1e-3,
            "mid-gap sample slid ({}) instead of jumping to the newer side",
            r.x
        );
        // A straddle at exactly the design depth still lerps.
        let mut it = Interp::new();
        it.push(10, &ent(5, 1000, 0));
        it.push(10 + MAX_LERP_SPAN_TICKS, &ent(5, 2000, 0));
        assert!(it.sample(5, 10.0 + MAX_LERP_SPAN_TICKS as f64 / 2.0, &mut r));
        assert!(
            (r.x - 1500.0 * 0.03).abs() < 1e-3,
            "an in-depth straddle stopped lerping"
        );
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
