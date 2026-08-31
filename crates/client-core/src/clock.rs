//! The client clocks (NETCODE.md §4, the Overwatch command-frame scheme):
//! a dilated 30 Hz input clock that runs ahead of the server so inputs
//! arrive just before the tick that consumes them, and a smoothed
//! server-tick estimate the interpolation delay hangs off. Pure — real
//! milliseconds come in from the host; no clock is read here.

use protocol::Nudge;
use sim_core::limits::TICK_HZ;

pub const TICK_MS: f64 = 1000.0 / TICK_HZ as f64;

/// Dilation step (NETCODE.md §4: ~5%, Overwatch's shipped ratio): `Faster`
/// shortens the input frame to 95% of a tick, `Slower` stretches to 105%.
const DILATE: f64 = 0.05;
/// Catch-up bound per advance: a tab that slept past this many ticks
/// hard-resyncs on the next snapshot instead of sprinting (proposed
/// default, DECISIONS.md §open, client fill-ins).
pub const MAX_CATCHUP_STEPS: u32 = 4;
/// Run-ahead re-established at start and on hard resync: the server's 1–2
/// tick buffer target plus one for flight time. Ping-derived offsets ride
/// a later slice (proposed default, DECISIONS.md §open).
pub const RESYNC_AHEAD_TICKS: u32 = 3;
/// Server-estimate feedback: gentle gain per snapshot, snap past the
/// hard-resync band (NETCODE §4 speaks ">6 ticks" as the resync trigger).
const EST_GAIN: f64 = 0.1;
const EST_SNAP_TICKS: f64 = 6.0;

pub struct ClientClock {
    /// The tick the next generated input frame is stamped with. May jump
    /// on hard resync; M0 consumers only ever difference it against the
    /// tail length, which survives jumps.
    pub client_tick: u32,
    acc_ms: f64,
    /// Frame period multiplier: 0.95 (`Faster`) / 1.0 / 1.05 (`Slower`).
    period_scale: f64,
    /// Smoothed estimate of the server's current tick, in float ticks.
    pub server_est: f64,
    resync_wanted: bool,
    /// Hard resyncs taken (HUD/diagnostics).
    pub resyncs: u64,
}

impl ClientClock {
    pub fn new(server_tick: u32) -> Self {
        Self {
            client_tick: server_tick.wrapping_add(RESYNC_AHEAD_TICKS),
            acc_ms: 0.0,
            period_scale: 1.0,
            server_est: server_tick as f64,
            resync_wanted: false,
            resyncs: 0,
        }
    }

    /// Advance real time; returns how many fixed input steps to run now
    /// (0..=`MAX_CATCHUP_STEPS`). Time beyond the catch-up bound is
    /// discarded and flags a resync — sprinting after a tab sleep would
    /// flood the server with stale inputs.
    pub fn advance(&mut self, dt_ms: f64) -> u32 {
        let dt = dt_ms.clamp(0.0, 1000.0);
        self.server_est += dt / TICK_MS;
        self.acc_ms += dt;
        let period = TICK_MS * self.period_scale;
        let mut steps = 0;
        while self.acc_ms >= period && steps < MAX_CATCHUP_STEPS {
            self.acc_ms -= period;
            steps += 1;
        }
        if self.acc_ms >= period {
            self.acc_ms = 0.0;
            self.resync_wanted = true;
        }
        steps
    }

    /// Snapshot header feedback: correct the server estimate, apply the
    /// dilation nudge, and take any pending hard resync against the fresh
    /// server tick. Returns true when a hard resync was taken.
    pub fn on_snapshot(&mut self, tick: u32, nudge: Nudge) -> bool {
        let err = tick as f64 - self.server_est;
        if err.abs() > EST_SNAP_TICKS {
            self.server_est = tick as f64;
        } else {
            self.server_est += err * EST_GAIN;
        }
        match nudge {
            Nudge::Ok => self.period_scale = 1.0,
            Nudge::Faster => self.period_scale = 1.0 - DILATE,
            Nudge::Slower => self.period_scale = 1.0 + DILATE,
            Nudge::HardResync => self.resync_wanted = true,
        }
        if self.resync_wanted {
            self.resync_wanted = false;
            self.client_tick = tick.wrapping_add(RESYNC_AHEAD_TICKS);
            self.acc_ms = 0.0;
            self.period_scale = 1.0;
            self.resyncs += 1;
            return true;
        }
        false
    }

    /// How far into the current input frame real time has got, in `[0, 1)`.
    ///
    /// The one thing on this clock a renderer wants: `advance` hands back
    /// whole steps and keeps the remainder, and the remainder is exactly the
    /// weight the camera should sit at between the last two predicted bodies
    /// (`Predictor::eye_position`). Without it the eye is redrawn at the same
    /// place for every frame inside one 33 ms tick, which is a staircase at
    /// any frame rate above 30.
    ///
    /// **Against the dilated period, not the nominal one**, so a `Faster` or
    /// `Slower` nudge changes how fast the eye crosses the gap and never how
    /// far: a fraction taken against `TICK_MS` while the clock is running a
    /// 105% frame would reach 1.0 early and hold there for the tail of every
    /// tick, which is the staircase back in miniature.
    ///
    /// Clamped rather than asserted: `advance` zeroes `acc_ms` on the
    /// catch-up bound and a hard resync, and a renderer must not be the thing
    /// that panics on a tab that slept.
    pub fn alpha(&self) -> f32 {
        let period = TICK_MS * self.period_scale;
        if period <= 0.0 {
            return 0.0;
        }
        (self.acc_ms / period).clamp(0.0, 1.0) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_at_the_fixed_rate() {
        let mut c = ClientClock::new(100);
        assert_eq!(c.client_tick, 103, "starts ahead by the resync margin");
        let mut steps = 0;
        for _ in 0..30 {
            steps += c.advance(TICK_MS);
        }
        assert_eq!(steps, 30);
    }

    /// The remainder the camera is drawn on: it walks a tick, wraps at the
    /// step, and never leaves `[0, 1)`.
    #[test]
    fn alpha_walks_the_tick_and_wraps_at_the_step() {
        let mut c = ClientClock::new(0);
        assert_eq!(c.alpha(), 0.0, "a fresh clock is on a tick boundary");
        // A quarter of a tick at a time: three frames inside one tick, then
        // the fourth steps and the remainder falls back to zero.
        let quarter = TICK_MS / 4.0;
        let mut seen = Vec::new();
        for _ in 0..4 {
            let steps = c.advance(quarter);
            seen.push((steps, c.alpha()));
        }
        assert_eq!(
            seen.iter().map(|(s, _)| *s).collect::<Vec<_>>(),
            vec![0, 0, 0, 1],
            "a quarter tick per frame steps once every fourth frame"
        );
        for (i, (_, a)) in seen.iter().enumerate() {
            assert!((0.0..1.0).contains(a), "frame {i}: alpha {a} left [0,1)");
        }
        assert!(
            seen[0].1 < seen[1].1 && seen[1].1 < seen[2].1,
            "the remainder has to grow across a tick, got {seen:?}"
        );
        assert!(seen[3].1 < seen[2].1, "the step resets it, got {seen:?}");
    }

    /// **Against the dilated period, not the nominal one.** Under a `Slower`
    /// nudge one real tick of time is 95% of a frame, so the remainder must
    /// read 0.95 and not 1.0 — a fraction taken against `TICK_MS` would
    /// saturate early and hold, which is the staircase in miniature.
    #[test]
    fn alpha_is_taken_against_the_dilated_frame() {
        let mut c = ClientClock::new(0);
        c.on_snapshot(0, Nudge::Slower);
        assert_eq!(
            c.advance(TICK_MS),
            0,
            "a stretched frame has not lapsed yet"
        );
        let a = c.alpha();
        assert!(
            (a - 1.0 / 1.05).abs() < 1e-3,
            "alpha must be the fraction of the STRETCHED frame, got {a}"
        );
        assert!(a < 1.0);
    }

    /// A tab that slept zeroes the remainder rather than leaving the camera
    /// pinned at the far end of a tick it never crossed.
    #[test]
    fn a_slept_tab_leaves_no_remainder_behind() {
        let mut c = ClientClock::new(0);
        c.advance(5000.0);
        assert_eq!(c.alpha(), 0.0);
    }

    #[test]
    fn faster_nudge_shortens_the_frame() {
        let mut c = ClientClock::new(0);
        c.on_snapshot(0, Nudge::Faster);
        let mut steps = 0;
        for _ in 0..100 {
            steps += c.advance(TICK_MS);
        }
        // 100 real ticks of time at 95% period ≈ 105 steps.
        assert!(steps > 100, "dilated clock must run ahead, got {steps}");
    }

    #[test]
    fn tab_sleep_caps_catchup_and_resyncs() {
        let mut c = ClientClock::new(0);
        assert_eq!(c.advance(5000.0), MAX_CATCHUP_STEPS);
        let resynced = c.on_snapshot(600, Nudge::Ok);
        assert!(resynced);
        assert_eq!(c.client_tick, 600 + RESYNC_AHEAD_TICKS);
        assert_eq!(c.resyncs, 1);
    }

    #[test]
    fn hard_resync_nudge_recenters() {
        let mut c = ClientClock::new(0);
        for _ in 0..10 {
            c.advance(TICK_MS);
        }
        assert!(c.on_snapshot(500, Nudge::HardResync));
        assert_eq!(c.client_tick, 500 + RESYNC_AHEAD_TICKS);
        assert_eq!(c.server_est, 500.0, "snap past the resync band");
    }

    #[test]
    fn estimate_tracks_gently_inside_the_band() {
        let mut c = ClientClock::new(100);
        c.on_snapshot(103, Nudge::Ok);
        assert!((c.server_est - 100.3).abs() < 1e-9);
    }
}
