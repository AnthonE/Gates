//! Own-capsule prediction and reconciliation (DESIGN.md §5.6): the local
//! body steps through the same `sim_core::movement` code the server runs,
//! immediately on input. Each snapshot carries the authoritative own state
//! at `last_executed_seq`; the predictor compares it against its ring of
//! predicted bodies — equal means the prediction held bit for bit, which
//! quantize-both-sides makes the common case (NETCODE.md §3) — and on a
//! mismatch rewinds to the server state, replays the unacked input tail,
//! and folds the visual jump into a render-only smoothing offset.

use protocol::EntityState;
use sim_core::input::InputFrame;
use sim_core::limits::MAX_INPUT_FRAMES;
use sim_core::movement::{self, Body, POS_XZ_Q, POS_Y_Q};

/// Predicted-body ring depth, keyed by seq: covers the unacked tail plus
/// acks in flight (32 seqs ≈ 1 s of input history).
const RING: usize = 32;

/// Correction smoothing (NETCODE.md §3, Gaffer's blend): exponential decay
/// per render frame — 0.95 for small errors, 0.85 from 1 m up, blended in
/// between — and a hard snap past `SNAP_AT_M` (NETCODE says "a few
/// meters"). Rates are NETCODE-spoken; snap threshold, blend window, and
/// dead zone are proposed defaults (DECISIONS.md §open, client fill-ins).
const SMOOTH_NEAR: f32 = 0.95;
const SMOOTH_FAR: f32 = 0.85;
const BLEND_FROM_M: f32 = 0.25;
const BLEND_TO_M: f32 = 1.0;
const SNAP_AT_M: f32 = 4.0;
const DEAD_ZONE_M: f32 = 0.01;

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
    /// Reconciles that had to rewind and replay (mismatch or ring miss).
    pub mispredictions: u64,
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
        }
    }

    pub fn tail(&self) -> &[InputFrame] {
        &self.tail[..self.tail_len]
    }

    /// One local input: append to the tail and, once started, step the
    /// predicted body and record it under the frame's seq.
    pub fn step(&mut self, frame: InputFrame) {
        if self.tail_len == MAX_INPUT_FRAMES {
            self.tail.copy_within(1.., 0);
            self.tail_len -= 1;
        }
        self.tail[self.tail_len] = frame;
        self.tail_len += 1;
        if self.started {
            movement::step(self.seed, &mut self.body, &frame);
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
    /// prefix; confirms the ring or rewinds-and-replays.
    pub fn reconcile(&mut self, own: &EntityState, last_executed: u16) {
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
            self.mispredictions += 1;
        }

        let old = self.position();
        self.body = Self::adopt(own);
        for i in 0..self.tail_len {
            let f = self.tail[i];
            movement::step(self.seed, &mut self.body, &f);
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

    /// Decay the correction offset; call once per render frame.
    pub fn decay_error(&mut self) {
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
        let rate = SMOOTH_NEAR + (SMOOTH_FAR - SMOOTH_NEAR) * t;
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

    const SEED: u64 = 0xC11E27;

    fn frame(seq: u16, move_z: i8) -> InputFrame {
        InputFrame {
            seq,
            buttons: if seq.is_multiple_of(3) { BTN_SPRINT } else { 0 },
            yaw: seq.wrapping_mul(2557),
            pitch: 0,
            move_x: (seq % 5) as i8 - 2,
            move_z,
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
            yaw: p.frame.yaw,
            pitch: p.frame.pitch,
        }
    }

    /// Clean lockstep against a real world: after adoption, every
    /// reconcile confirms bit-identically — zero mispredictions.
    #[test]
    fn prediction_is_bit_exact_in_lockstep() {
        use sim_core::world::Command;
        let mut world = World::new(SEED);
        world.tick(&[Command::Join { id: 7 }]);
        let mut p = Predictor::new(SEED);
        p.reconcile(&own_state(&world, 7), 0); // adopt spawn

        for seq in 1..=200u16 {
            let f = frame(seq, 127);
            p.step(f);
            world.tick(&[Command::Input { id: 7, frame: f }]);
            p.reconcile(&own_state(&world, 7), seq);
        }
        assert_eq!(p.mispredictions, 0);
        assert_eq!(p.confirmations, 200);
        assert_eq!(p.error_magnitude(), 0.0);
    }

    /// A server-side disagreement rewinds, replays the tail, and folds the
    /// jump into a smoothing offset that decays away.
    #[test]
    fn misprediction_rewinds_and_smooths() {
        use sim_core::world::Command;
        let mut world = World::new(SEED);
        world.tick(&[Command::Join { id: 7 }]);
        let mut p = Predictor::new(SEED);
        p.reconcile(&own_state(&world, 7), 0);

        // Client predicts 4 unacked walks the server never saw the same
        // way: server executes a different input for seq 1.
        for seq in 1..=4u16 {
            p.step(frame(seq, 127));
        }
        let lie = InputFrame {
            move_z: -127,
            ..frame(1, 127)
        };
        world.tick(&[Command::Input { id: 7, frame: lie }]);
        let auth = own_state(&world, 7);
        p.reconcile(&auth, 1);
        assert_eq!(p.mispredictions, 1);
        assert_eq!(p.tail().len(), 3, "acked prefix dropped");
        assert!(p.error_magnitude() > 0.0, "visual error captured");
        for _ in 0..600 {
            p.decay_error();
        }
        assert_eq!(p.error_magnitude(), 0.0, "offset decays to zero");
    }

    #[test]
    fn tail_drops_oldest_at_the_wire_cap() {
        let mut p = Predictor::new(SEED);
        for seq in 1..=(MAX_INPUT_FRAMES as u16 + 5) {
            p.step(frame(seq, 0));
        }
        assert_eq!(p.tail().len(), MAX_INPUT_FRAMES);
        assert_eq!(p.tail()[0].seq, 6, "oldest dropped first");
    }
}
