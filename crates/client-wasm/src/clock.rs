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
