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
    /// The body as it stood **one tick ago**, kept only so the camera can be
    /// drawn between two ticks instead of on them. See
    /// [`Predictor::eye_position`].
    ///
    /// Never read by the sim, never sent, never reconciled against: a
    /// rewind-and-replay overwrites it with wherever the replay's last-but-one
    /// step landed, which is the right answer for the one thing it is for.
    prev: Body,
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
            prev: Body::default(),
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
            // **Before the step, not after** — the pair this keeps is
            // (where the camera was at the last tick boundary, where it is
            // at this one), and `eye_position` walks between them.
            self.prev = self.body;
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
            self.mispredictions += 1;
        }

        let old = self.position();
        self.body = Self::adopt(own);
        // A rewind restarts the pair from the authoritative state; the replay
        // below then walks it forward exactly as `step` does, so the last two
        // steps of the replay leave `prev`/`body` one tick apart again. A
        // replay with an empty tail leaves them equal, which reads as "not
        // moving" for one frame and is the truth on the frame a snapshot
        // caught us standing still.
        self.prev = self.body;
        let haven = occ.haven;
        for i in 0..self.tail_len {
            let f = self.tail[i];
            self.prev = self.body;
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
        Self::position_of(&self.body)
    }

    /// Render position: predicted + the decaying correction offset.
    pub fn render_position(&self) -> [f32; 3] {
        let p = self.position();
        [p[0] + self.err[0], p[1] + self.err[1], p[2] + self.err[2]]
    }

    /// Where to draw the **camera**, `alpha` of the way from the previous
    /// tick's predicted body to this one's.
    ///
    /// ## Why the eye needs its own reader and the pick must not have one
    ///
    /// The predictor steps on a fixed 30 Hz clock (`ClientCore::advance`), so
    /// [`Predictor::render_position`] is a staircase: at 60 fps every second
    /// frame repeats the last one, at 144 fps four frames in five do. Sitting
    /// still that is invisible. Running, it is the whole picture — the eye
    /// jumps 8 cm at a sprint, holds for three frames, jumps again, and a
    /// player tracking the world with their own eyes reads the repeats as
    /// smear. That is the operator's *"all blurry like its snapping around"*
    /// (2026-08-30) and it is not a renderer bug: nothing downstream of here
    /// could smooth a position it is handed once per tick.
    ///
    /// **What this costs is one tick of camera latency and nothing else.**
    /// Drawing between the last two ticks means the eye is up to 33 ms behind
    /// the predictor — the standard price, and the alternative (extrapolating
    /// past the newest tick) overshoots every stop and every wall, which reads
    /// as rubber-banding on exactly the frames a player is looking hardest.
    /// **Aim is untouched**: `Eye::yaw`/`pitch` come from the mouse at frame
    /// rate (`render::input::place_eye`) and never from here, so the crosshair
    /// stays as immediate as it was.
    ///
    /// **And it is deliberately NOT what `render_position` returns.**
    /// `render::verbs::resolve` picks what a verb addresses off that function,
    /// on the quantize-both-sides law — it must resolve on the position the
    /// sim will answer for, not on a smoothed one a third of a tick behind it,
    /// or the client offers a verb the server declines at the edge of a reach
    /// radius. So this is a second reader for the camera alone, and the two
    /// agree exactly whenever the body is at rest.
    pub fn eye_position(&self, alpha: f32) -> [f32; 3] {
        // `is_finite` first, because `f32::clamp` passes a NaN through
        // unchanged — and a NaN reaching the camera transform is a black
        // frame, not a wobble. The clock cannot produce one today
        // (`ClientClock::alpha` divides by a period it has already tested);
        // this is the reader refusing to be the place that finds out.
        let a = if alpha.is_finite() {
            alpha.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let p = self.position();
        let q = Self::position_of(&self.prev);
        [
            q[0] + (p[0] - q[0]) * a + self.err[0],
            q[1] + (p[1] - q[1]) * a + self.err[1],
            q[2] + (p[2] - q[2]) * a + self.err[2],
        ]
    }

    fn position_of(b: &Body) -> [f32; 3] {
        [
            b.qx as f32 * POS_XZ_Q,
            b.qy as f32 * POS_Y_Q,
            b.qz as f32 * POS_XZ_Q,
        ]
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

    /// **The staircase, and that it is gone.** The defect this gate is
    /// written against was reported from play — *"when i run my player is all
    /// blurry like its snapping around"* — and its mechanism is that
    /// `render_position` is only ever written on a tick boundary. Sampled at
    /// a frame rate that is not the tick rate, the camera therefore repeats a
    /// position and then jumps.
    ///
    /// Rebuilt from the PUBLISHED parts rather than by calling the thing under
    /// test (`CLAUDE.md`'s naive-rebuild trap): the expected point is
    /// `position()`'s own two endpoints, and the walk is arithmetic written
    /// out here.
    #[test]
    fn the_eye_crosses_the_tick_instead_of_jumping_it() {
        use sim_core::world::Command;
        let mut world = World::new(SEED);
        world.tick(&[Command::Join { id: 7 }]);
        let cols = Box::new(ColIndex::new());
        let mut occ = Scratch::live(SEED);
        let mut p = Predictor::new(SEED);
        p.reconcile(&own_state(&world, 7), 0, &cols, &mut occ.occupants());

        // Walk a few ticks so the body is genuinely moving, then look at the
        // pair the eye is drawn between.
        for seq in 1..=8u16 {
            p.step(frame(seq, 127), &cols, &mut occ.occupants());
        }
        let now = p.position();
        let was = Predictor::position_of(&p.prev);
        let travelled: f32 = (0..3)
            .map(|i| (now[i] - was[i]).powi(2))
            .sum::<f32>()
            .sqrt();
        assert!(
            travelled > 0.05,
            "the fixture has to be moving for this to mean anything, got {travelled} m/tick"
        );

        // The two ends are exactly the two ticks, and nothing in between
        // leaves the segment.
        assert_eq!(p.eye_position(0.0), was, "alpha 0 is the previous tick");
        assert_eq!(p.eye_position(1.0), now, "alpha 1 is this tick");
        for k in 0..=10 {
            let a = k as f32 / 10.0;
            let got = p.eye_position(a);
            for i in 0..3 {
                let want = was[i] + (now[i] - was[i]) * a;
                assert!(
                    (got[i] - want).abs() <= 1e-6,
                    "alpha {a} axis {i}: {} vs {want}",
                    got[i]
                );
            }
        }

        // The staircase, stated as the property that failed: four frames at
        // 120 fps inside one 30 Hz tick must be four DIFFERENT places, where
        // `render_position` gives one place four times.
        let frames: Vec<[f32; 3]> = (0..4).map(|k| p.eye_position(k as f32 / 4.0)).collect();
        for w in frames.windows(2) {
            let d: f32 = (0..3)
                .map(|i| (w[1][i] - w[0][i]).powi(2))
                .sum::<f32>()
                .sqrt();
            assert!(d > 0.0, "two frames inside one tick drew the same position");
        }
        assert_eq!(
            p.render_position(),
            now,
            "the sim-truth reader every verb resolves against is untouched"
        );
    }

    /// Out-of-range alphas are clamped, not extrapolated: a tab that slept
    /// zeroes the clock's remainder and a renderer must not be the thing that
    /// throws the camera past the body it is drawing.
    #[test]
    fn the_eye_never_leaves_the_segment() {
        let mut p = Predictor::new(SEED);
        p.prev = Body {
            qx: 0,
            qy: 0,
            qz: 0,
            qvy: 0,
            grounded: true,
        };
        p.body = Body {
            qx: 1000,
            qy: 0,
            qz: 0,
            qvy: 0,
            grounded: true,
        };
        assert_eq!(p.eye_position(-5.0), p.eye_position(0.0));
        assert_eq!(p.eye_position(5.0), p.eye_position(1.0));
        assert_eq!(p.eye_position(f32::NAN), p.eye_position(0.0));
    }

    /// A body at rest draws at exactly one place, so the camera and the verb
    /// resolver cannot disagree about where you are standing when it matters.
    #[test]
    fn a_standing_body_draws_where_the_verbs_resolve() {
        use sim_core::world::Command;
        let mut world = World::new(SEED);
        world.tick(&[Command::Join { id: 7 }]);
        let cols = Box::new(ColIndex::new());
        let mut occ = Scratch::live(SEED);
        let mut p = Predictor::new(SEED);
        p.reconcile(&own_state(&world, 7), 0, &cols, &mut occ.occupants());
        // No movement input at all: `move_x`/`move_z` zero, no buttons.
        for seq in 1..=20u16 {
            p.step(
                InputFrame {
                    seq,
                    ..InputFrame::default()
                },
                &cols,
                &mut occ.occupants(),
            );
        }
        for k in 0..=8 {
            assert_eq!(
                p.eye_position(k as f32 / 8.0),
                p.render_position(),
                "at rest the two readers are the same number"
            );
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
            p.decay_error();
        }
        assert_eq!(p.error_magnitude(), 0.0, "offset decays to zero");
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
