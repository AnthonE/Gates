//! Own-capsule prediction and reconciliation (DESIGN.md §5.6): the local
//! body steps through the same `sim_core::movement` code the server runs,
//! immediately on input. Each snapshot carries the authoritative own state
//! at `last_executed_seq`; the predictor compares it against its ring of
//! predicted bodies — equal means the prediction held bit for bit, which
//! quantize-both-sides makes the common case (NETCODE.md §3) — and on a
//! mismatch rewinds to the server state, replays the unacked input tail,
//! and folds the visual jump into a render-only smoothing offset.

use protocol::EntityState;
use sim_core::collide::ColIndex;
use sim_core::input::InputFrame;
use sim_core::limits::MAX_INPUT_FRAMES;
use sim_core::movement::{self, Body, POS_XZ_Q, POS_Y_Q};
use sim_core::occupy::Occupants;

/// Predicted-body ring depth, keyed by seq: covers the unacked tail plus
/// acks in flight (32 seqs ≈ 1 s of input history).
const RING: usize = 32;

/// Correction smoothing (NETCODE.md §3, Gaffer's blend): exponential decay
/// **in wall time** — τ 325 ms for small errors, 100 ms from 1 m up,
/// blended in between — and a hard snap past `SNAP_AT_M` (NETCODE says "a
/// few meters"). Time constants since netcode v2 S3: the rates shipped as
/// per-RENDER-FRAME multipliers (0.95 / 0.85), so the same correction
/// lingered five times longer at 30 fps than at 144 — a smoothing whose
/// speed depended on the GPU. The τ values are those multipliers read at
/// 60 fps (`0.95^60 ≈ e^-60/325ms·16.7ms`, `0.85 ≈ e^-16.7/100`), so a
/// 60 fps client feels exactly what it shipped with and every other frame
/// rate now feels the same thing. Snap threshold, blend window, and dead
/// zone are proposed defaults (DECISIONS.md §open, client fill-ins).
const TAU_NEAR_MS: f32 = 325.0;
const TAU_FAR_MS: f32 = 100.0;
const BLEND_FROM_M: f32 = 0.25;
const BLEND_TO_M: f32 = 1.0;
const SNAP_AT_M: f32 = 4.0;
const DEAD_ZONE_M: f32 = 0.01;

/// The minor-correction band, in quanta (netcode v2 S3): a reconcile whose
/// disagreement fits inside it — one x/z quantum (3 cm) per axis, 5 cm of
/// y, 5 cm/s of vertical velocity, grounded agreeing — is counted as
/// `corrections_minor` rather than a misprediction. **Classification only,
/// deliberately**: every mismatch still adopts the server body, replays
/// the tail, and folds the jump into the smoothing offset, so convergence
/// stays bit-exact and every exact-equality gate keeps its meaning. What
/// the split buys is an honest diagnostic — post-S1 the starved-tick
/// residue is sub-quantum jiggle the smoothing hides completely, and a
/// `mispredictions` counter that lumped it in with real divergence would
/// send whoever reads the netgraph hunting for a defect the player cannot
/// feel. (Rocket League's "large difference requires correction" skips the
/// rollback below the band; ours costs two movement steps, so honesty is
/// the only thing worth buying here.)
const MINOR_XZ_Q: i32 = 1;
const MINOR_Y_Q: i32 = 5;
const MINOR_VY_Q: i32 = 5;

pub struct Predictor {
    seed: u64,
    /// False until the first snapshot carrying our own entity adopts the
    /// authoritative spawn state; prediction is meaningless before it.
    pub started: bool,
    /// Predicted body after applying the newest local input.
    pub body: Body,
    /// Unacked input tail, oldest first — the wire's redundancy source and
    /// the reconciliation replay source. Overflow policy: drop oldest, the
    /// wire cap (`limits.rs MAX_INPUT_FRAMES`); past it the server was
    /// reusing inputs anyway and the next reconcile corrects us.
    tail: [InputFrame; MAX_INPUT_FRAMES],
    tail_len: usize,
    ring_seq: [u16; RING],
    ring_body: [Body; RING],
    ring_valid: [bool; RING],
    /// Render-only correction offset in meters, decayed per render frame.
    /// Never sim state: the predicted body is already authoritative-shaped.
    err: [f32; 3],
    /// Reconciles where the ring held `last_executed_seq` bit-identical.
    pub confirmations: u64,
    /// Reconciles that had to rewind and replay past the minor band
    /// (real divergence: a collision disagreement, a missed input, a
    /// world change mid-flight).
    pub mispredictions: u64,
    /// Reconciles that rewound inside the minor band (netcode v2 S3) —
    /// sub-quantum jiggle, mostly the residue of starved-tick decay,
    /// invisible under the smoothing. Same rewind, different ledger.
    pub corrections_minor: u64,
}

impl Predictor {
    pub fn new(seed: u64) -> Self {
        Self {
            seed,
            started: false,
            body: Body::default(),
            tail: [InputFrame::default(); MAX_INPUT_FRAMES],
            tail_len: 0,
            ring_seq: [0; RING],
            ring_body: [Body::default(); RING],
            ring_valid: [false; RING],
            err: [0.0; 3],
            confirmations: 0,
            mispredictions: 0,
            corrections_minor: 0,
        }
    }

    pub fn tail(&self) -> &[InputFrame] {
        &self.tail[..self.tail_len]
    }

    /// The seed this predictor steps under — the one `movement::step` is
    /// handed, and therefore the one the shared `SlotCache` holds lines for.
    /// Read by `ClientCore::island`, so a caller asking the cache about a
    /// cell cannot supply a *different* seed and silently flush it.
    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// One local input: append to the tail and, once started, step the
    /// predicted body and record it under the frame's seq. `cols` is the
    /// client piece mirror's collision index — the predictor collides
    /// with the same walls the server does, one in-flight placement of
    /// skew at most (a mismatch there is just a reconcile).
    pub fn step(&mut self, frame: InputFrame, cols: &ColIndex, occ: &mut Occupants) {
        if self.tail_len == MAX_INPUT_FRAMES {
            self.tail.copy_within(1.., 0);
            self.tail_len -= 1;
        }
        self.tail[self.tail_len] = frame;
        self.tail_len += 1;
        if self.started {
            // The haven comes off `occ` rather than being threaded in
            // separately, so the ground the predictor steps on is by
            // construction the same one its collision bundle was built
            // against — two sources for one island is how a client and a
            // server come to disagree about where the floor is.
            let haven = occ.haven;
            movement::step(self.seed, haven, cols, occ, &mut self.body, &frame);
            self.record(frame.seq);
        }
    }

    fn record(&mut self, seq: u16) {
        let i = seq as usize % RING;
        self.ring_seq[i] = seq;
        self.ring_body[i] = self.body;
        self.ring_valid[i] = true;
    }

    fn matches(b: &Body, e: &EntityState) -> bool {
        b.qx == e.qx && b.qy == e.qy && b.qz == e.qz && b.qvy == e.qvy && b.grounded == e.grounded
    }

    fn adopt(e: &EntityState) -> Body {
        Body {
            qx: e.qx,
            qy: e.qy,
            qz: e.qz,
            qvy: e.qvy,
            grounded: e.grounded,
        }
    }

    /// One snapshot's authoritative own state. Drops the acked tail
    /// prefix; confirms the ring or rewinds-and-replays (the replay
    /// collides through `cols` like the live steps did).
    pub fn reconcile(
        &mut self,
        own: &EntityState,
        last_executed: u16,
        cols: &ColIndex,
        occ: &mut Occupants,
    ) {
        let mut keep = 0;
        for i in 0..self.tail_len {
            let ahead = self.tail[i].seq.wrapping_sub(last_executed);
            if (1..0x8000).contains(&ahead) {
                self.tail[keep] = self.tail[i];
                keep += 1;
            }
        }
        self.tail_len = keep;

        if self.started {
            let i = last_executed as usize % RING;
            if self.ring_valid[i]
                && self.ring_seq[i] == last_executed
                && Self::matches(&self.ring_body[i], own)
            {
                self.confirmations += 1;
                return;
            }
            // Classify before the rewind: which ledger this disagreement
            // belongs to. Both take the identical adopt-replay-smooth path
            // below — see MINOR_XZ_Q's doc for why the band buys honesty
            // and deliberately nothing else.
            let b = if self.ring_valid[i] && self.ring_seq[i] == last_executed {
                Some(self.ring_body[i])
            } else {
                None
            };
            let minor = b.is_some_and(|b| {
                (b.qx - own.qx).abs() <= MINOR_XZ_Q
                    && (b.qz - own.qz).abs() <= MINOR_XZ_Q
                    && (b.qy - own.qy).abs() <= MINOR_Y_Q
                    && (b.qvy - own.qvy).abs() <= MINOR_VY_Q
                    && b.grounded == own.grounded
            });
            if minor {
                self.corrections_minor += 1;
            } else {
                self.mispredictions += 1;
            }
        }

        let old = self.position();
        self.body = Self::adopt(own);
        let haven = occ.haven;
        for i in 0..self.tail_len {
            let f = self.tail[i];
            movement::step(self.seed, haven, cols, occ, &mut self.body, &f);
            self.record(f.seq);
        }
        if self.started {
            let new = self.position();
            self.err[0] += old[0] - new[0];
            self.err[1] += old[1] - new[1];
            self.err[2] += old[2] - new[2];
            let m2 =
                self.err[0] * self.err[0] + self.err[1] * self.err[1] + self.err[2] * self.err[2];
            if m2 > SNAP_AT_M * SNAP_AT_M {
                self.err = [0.0; 3]; // teleport-grade: snap, don't glide
            }
        }
        self.started = true;
    }

    /// Predicted position in meters (the sim-truth one, no smoothing).
    pub fn position(&self) -> [f32; 3] {
        [
            self.body.qx as f32 * POS_XZ_Q,
            self.body.qy as f32 * POS_Y_Q,
            self.body.qz as f32 * POS_XZ_Q,
        ]
    }

    /// Render position: predicted + the decaying correction offset.
    pub fn render_position(&self) -> [f32; 3] {
        let p = self.position();
        [p[0] + self.err[0], p[1] + self.err[1], p[2] + self.err[2]]
    }

    /// Current correction magnitude in meters (HUD/diagnostics).
    pub fn error_magnitude(&self) -> f32 {
        (self.err[0] * self.err[0] + self.err[1] * self.err[1] + self.err[2] * self.err[2]).sqrt()
    }

    /// Decay the correction offset by the wall time that actually passed —
    /// call once per render frame with the frame's dt. Time-based since
    /// netcode v2 S3 (`TAU_NEAR_MS`'s doc has the history): `exp(-dt/τ)`
    /// makes ten 16.7 ms frames decay exactly what one 167 ms frame does,
    /// so the smoothing's feel stops depending on the frame rate.
    pub fn decay_error(&mut self, dt_ms: f64) {
        let m2 = self.err[0] * self.err[0] + self.err[1] * self.err[1] + self.err[2] * self.err[2];
        if m2 == 0.0 {
            return;
        }
        let m = m2.sqrt();
        if m < DEAD_ZONE_M {
            self.err = [0.0; 3];
            return;
        }
        let t = ((m - BLEND_FROM_M) / (BLEND_TO_M - BLEND_FROM_M)).clamp(0.0, 1.0);
        let tau = TAU_NEAR_MS + (TAU_FAR_MS - TAU_NEAR_MS) * t;
        let rate = (-(dt_ms as f32) / tau).exp();
        for v in &mut self.err {
            *v *= rate;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_core::input::BTN_SPRINT;
    use sim_core::world::World;

    use sim_core::occupy::Scratch;

    const SEED: u64 = 0xC11E27;

    fn frame(seq: u16, move_z: i8) -> InputFrame {
        InputFrame {
            seq,
            buttons: if seq.is_multiple_of(3) { BTN_SPRINT } else { 0 },
            yaw: seq.wrapping_mul(2557),
            pitch: 0,
            move_x: (seq % 5) as i8 - 2,
            move_z,
            sel: (seq % 6) as u8,
        }
    }

    fn own_state(w: &World, id: u32) -> EntityState {
        let p = w.players.iter().find(|p| p.active && p.id == id).unwrap();
        EntityState {
            id,
            qx: p.body.qx,
            qy: p.body.qy,
            qz: p.body.qz,
            qvy: p.body.qvy,
            grounded: p.body.grounded,
            sleeping: p.sleeping,
            dead: p.dead,
            yaw: p.frame.yaw,
            pitch: p.frame.pitch,
            // The predictor reconciles the OWN body, whose hand this
            // client already knows from `SUB_INV` — v56's two fields are
            // for the bodies that are not yours, and neither is part of
            // what a misprediction can be about.
            held: None,
            lit: false,
        }
    }

    /// Clean lockstep against a real world: after adoption, every
    /// reconcile confirms bit-identically — zero mispredictions.
    #[test]
    fn prediction_is_bit_exact_in_lockstep() {
        use sim_core::world::Command;
        let mut world = World::new(SEED);
        world.tick(&[Command::Join { id: 7 }]);
        let cols = Box::new(ColIndex::new());
        // The predictor collides with the same island the server does.
        let mut occ = Scratch::live(SEED);
        let mut p = Predictor::new(SEED);
        p.reconcile(&own_state(&world, 7), 0, &cols, &mut occ.occupants()); // adopt spawn

        for seq in 1..=200u16 {
            let f = frame(seq, 127);
            p.step(f, &cols, &mut occ.occupants());
            world.tick(&[Command::Input {
                id: 7,
                frame: f,
                favour: 0,
            }]);
            p.reconcile(&own_state(&world, 7), seq, &cols, &mut occ.occupants());
        }
        assert_eq!(p.mispredictions, 0);
        assert_eq!(p.confirmations, 200);
        assert_eq!(p.error_magnitude(), 0.0);
    }

    /// Lockstep with pieces standing: the predictor collides with the
    /// mirrored walls exactly as the server does — walking into a wall
    /// pins both at the slab with zero mispredictions.
    #[test]
    fn prediction_collides_bit_exact_through_the_mirror() {
        use sim_core::build::{BuildContent, LOC_EDGE_XLO, LOC_PLANE};
        use sim_core::gather::ItemStack;
        use sim_core::world::Command;

        // The browser-smoke anchor: seed + cell guarded walkable natively.
        let seed = 20260731u64;
        let mut world = World::new(seed);
        world.build = BuildContent::probe_fixture();
        world.dev_spawn = Some((1024.5, 1024.5));
        world.tick(&[Command::Join { id: 7 }]);
        world.players[0].inv[0] = ItemStack {
            item: 0,
            count: 50,
            cond: 0,
        };
        world.tick(&[
            Command::Place {
                id: 7,
                row: 0,
                cx: 341,
                cz: 341,
                level: 0,
                loc: LOC_PLANE,
                freehand: false,
            },
            Command::Place {
                id: 7,
                row: 1,
                cx: 341,
                cz: 341,
                level: 0,
                loc: LOC_EDGE_XLO,
                freehand: false,
            },
        ]);
        assert_eq!(world.pieces.len(), 2, "fixture placements must land");

        // The client mirror: same records, its own index (core.rs path).
        let mut cols = Box::new(ColIndex::new());
        for r in world.pieces.entries() {
            let shape = world.build.pieces[r.row as usize].shape;
            cols.add(r.cx, r.cz, r.level, r.loc, shape, r.plate);
        }

        // The predictor collides with the same island the server does.
        let mut occ = Scratch::live(seed);
        let mut p = Predictor::new(seed);
        p.reconcile(&own_state(&world, 7), 0, &cols, &mut occ.occupants());
        for seq in 1..=150u16 {
            // Strafe -x into the low-x wall, forever.
            let f = InputFrame {
                seq,
                move_x: -127,
                ..InputFrame::default()
            };
            p.step(f, &cols, &mut occ.occupants());
            world.tick(&[Command::Input {
                id: 7,
                frame: f,
                favour: 0,
            }]);
            p.reconcile(&own_state(&world, 7), seq, &cols, &mut occ.occupants());
        }
        assert_eq!(p.mispredictions, 0, "mirror and server must agree");
        let x = p.position()[0];
        assert!(
            x >= 341.0 * 3.0 + 0.5,
            "the wall never engaged: x {x} walked through the slab"
        );
    }

    /// A server-side disagreement rewinds, replays the tail, and folds the
    /// jump into a smoothing offset that decays away.
    #[test]
    fn misprediction_rewinds_and_smooths() {
        use sim_core::world::Command;
        let mut world = World::new(SEED);
        world.tick(&[Command::Join { id: 7 }]);
        let cols = Box::new(ColIndex::new());
        // The predictor collides with the same island the server does.
        let mut occ = Scratch::live(SEED);
        let mut p = Predictor::new(SEED);
        p.reconcile(&own_state(&world, 7), 0, &cols, &mut occ.occupants());

        // Client predicts 4 unacked walks the server never saw the same
        // way: server executes a different input for seq 1.
        for seq in 1..=4u16 {
            p.step(frame(seq, 127), &cols, &mut occ.occupants());
        }
        let lie = InputFrame {
            move_z: -127,
            ..frame(1, 127)
        };
        world.tick(&[Command::Input {
            id: 7,
            frame: lie,
            favour: 0,
        }]);
        let auth = own_state(&world, 7);
        p.reconcile(&auth, 1, &cols, &mut occ.occupants());
        assert_eq!(p.mispredictions, 1);
        assert_eq!(p.tail().len(), 3, "acked prefix dropped");
        assert!(p.error_magnitude() > 0.0, "visual error captured");
        for _ in 0..600 {
            p.decay_error(1000.0 / 60.0);
        }
        assert_eq!(p.error_magnitude(), 0.0, "offset decays to zero");
    }

    /// **The smoothing is a function of wall time, not of how often the
    /// GPU finishes a frame** (netcode v2 S3). One 167 ms frame and ten
    /// 16.7 ms frames decay a correction identically; under the old
    /// per-frame multiplier the ten-frame path decayed it ten times as
    /// hard, so a 144 fps client barely saw corrections a 30 fps client
    /// watched glide for half a second. Red by construction on that code.
    #[test]
    fn the_correction_decay_is_frame_rate_independent() {
        let seed_err = |p: &mut Predictor| {
            p.err = [0.10, 0.0, 0.0];
        };
        let mut coarse = Predictor::new(SEED);
        seed_err(&mut coarse);
        coarse.decay_error(167.0);
        let mut fine = Predictor::new(SEED);
        seed_err(&mut fine);
        for _ in 0..10 {
            fine.decay_error(16.7);
        }
        let (a, b) = (coarse.error_magnitude(), fine.error_magnitude());
        assert!(
            (a - b).abs() < 1e-4,
            "one 167 ms frame decayed to {a} where ten 16.7 ms frames \
             decayed to {b} — the smoothing depends on the frame rate"
        );
        assert!(a < 0.10, "wall time passed, the correction must shrink");
    }

    #[test]
    fn tail_drops_oldest_at_the_wire_cap() {
        let cols = Box::new(ColIndex::new());
        // The predictor collides with the same island the server does.
        let mut occ = Scratch::live(SEED);
        let mut p = Predictor::new(SEED);
        for seq in 1..=(MAX_INPUT_FRAMES as u16 + 5) {
            p.step(frame(seq, 0), &cols, &mut occ.occupants());
        }
        assert_eq!(p.tail().len(), MAX_INPUT_FRAMES);
        assert_eq!(p.tail()[0].seq, 6, "oldest dropped first");
    }
}
