//! The client clocks (NETCODE.md §4, the Overwatch command-frame scheme):
//! a dilated 30 Hz input clock that runs ahead of the server so inputs
//! arrive just before the tick that consumes them, and a smoothed
//! server-tick estimate the interpolation delay hangs off. Pure — real
//! milliseconds come in from the host; no clock is read here.

use protocol::Nudge;
use sim_core::limits::TICK_HZ;

pub const TICK_MS: f64 = 1000.0 / TICK_HZ as f64;

/// Dilation clamp (NETCODE.md §4: ~5%, Overwatch's shipped ratio — 16 ms
/// frames run at ~15.2 ms when starving). Since netcode v2 S4 this bounds
/// the proportional controller below rather than being the whole of the
/// answer.
const DILATE: f64 = 0.05;

/// The depth controller (netcode v2 S4, DECISIONS.md §open): the client
/// steers its input clock on the server's OWN gauge — the post-consume
/// buffer depth the v60 header reports — instead of following the 2-bit
/// nudge's quantization of it. Target Overwatch's/Rocket League's 1–2
/// buffered frames, with the piece the bang-bang lacked: a **deadband**.
/// Inside `TARGET ± DEADBAND` (depth 1–3) the period is exactly nominal —
/// the old scheme dilated at depth 3, so a buffer breathing between 2 and
/// 3 had the clock hunting and every hunt was a small misprediction.
/// One step past the band reaches the full ±`DILATE` (depth 0, starving —
/// the old `Faster`; depth 4+ — the old `Slower`); `KP` is the slope that
/// makes the response continuous at the band's edge and proportional if
/// the gauge ever reports finer than whole frames.
const DEPTH_TARGET: f64 = 2.0;
const DEPTH_DEADBAND: f64 = 1.0;
const DEPTH_KP: f64 = 0.05;
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
    /// Frame period multiplier, the depth controller's output:
    /// `1.0 ± DEPTH_KP·error`, clamped to `1.0 ± DILATE`.
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

    /// Snapshot header feedback: correct the server estimate, run the
    /// depth controller, and take any pending hard resync against the
    /// fresh server tick. Returns true when a hard resync was taken.
    ///
    /// `depth` is the header's `buffered_depth` (wire v60) — the gauge
    /// itself, where this used to follow the nudge's 2-bit quantization
    /// of it. `Nudge::HardResync` is still honored: it is starvation-
    /// driven (a full second of it, `server/client.rs`), a signal no
    /// depth reading carries, and the resync path is the one thing the
    /// nudge still owns. Its `Faster`/`Slower` rungs are vestigial —
    /// stamped by the server, ignored here since netcode v2 S4.
    pub fn on_snapshot(&mut self, tick: u32, nudge: Nudge, depth: u8) -> bool {
        let err = tick as f64 - self.server_est;
        if err.abs() > EST_SNAP_TICKS {
            self.server_est = tick as f64;
        } else {
            self.server_est += err * EST_GAIN;
        }
        let depth_err = f64::from(depth) - DEPTH_TARGET;
        self.period_scale = if depth_err.abs() <= DEPTH_DEADBAND {
            1.0
        } else {
            // Proportional on the error past the deadband's edge, so the
            // response is continuous at the boundary instead of stepping.
            let past = depth_err - DEPTH_DEADBAND.copysign(depth_err);
            (1.0 + past * DEPTH_KP).clamp(1.0 - DILATE, 1.0 + DILATE)
        };
        if nudge == Nudge::HardResync {
            self.resync_wanted = true;
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

    /// The controller, across the gauge (netcode v2 S4): starving runs
    /// the clock at the full -5%, a deep buffer at the full +5%, and the
    /// whole deadband — including depth 3, where the old bang-bang
    /// hunted — sits at exactly nominal.
    #[test]
    fn the_depth_controller_dilates_past_the_deadband_only() {
        let period_at = |depth: u8| {
            let mut c = ClientClock::new(0);
            c.on_snapshot(0, Nudge::Ok, depth);
            let mut steps = 0;
            for _ in 0..100 {
                steps += c.advance(TICK_MS);
            }
            steps
        };
        assert!(period_at(0) > 100, "starving must run the clock fast");
        for depth in 1..=3u8 {
            assert_eq!(period_at(depth), 100, "depth {depth} is home: no hunt");
        }
        assert!(period_at(4) < 100, "a deep buffer must slow the clock");
        assert_eq!(period_at(15), period_at(4), "clamped at the rail");
        // The vestigial nudge rungs no longer steer: a Faster stamp at a
        // healthy depth changes nothing.
        let mut c = ClientClock::new(0);
        c.on_snapshot(0, Nudge::Faster, 2);
        let mut steps = 0;
        for _ in 0..100 {
            steps += c.advance(TICK_MS);
        }
        assert_eq!(steps, 100, "the 2-bit nudge must not out-vote the gauge");
    }

    #[test]
    fn tab_sleep_caps_catchup_and_resyncs() {
        let mut c = ClientClock::new(0);
        assert_eq!(c.advance(5000.0), MAX_CATCHUP_STEPS);
        let resynced = c.on_snapshot(600, Nudge::Ok, 2);
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
        assert!(c.on_snapshot(500, Nudge::HardResync, 2));
        assert_eq!(c.client_tick, 500 + RESYNC_AHEAD_TICKS);
        assert_eq!(c.server_est, 500.0, "snap past the resync band");
    }

    #[test]
    fn estimate_tracks_gently_inside_the_band() {
        let mut c = ClientClock::new(100);
        c.on_snapshot(103, Nudge::Ok, 2);
        assert!((c.server_est - 100.3).abs() < 1e-9);
    }
}
