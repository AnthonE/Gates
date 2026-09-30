//! The fight reflex: what runs over whatever goal is in hand when another
//! body turns on this one. It never waits on the mind.
//!
//! For now it is the survivor's old reflex, unchanged in what it does: a
//! blow (or a flee goal) starts a bounded retreat that faces back toward
//! the danger and backs away from it, renewed while a pursuer stays in
//! sight. The route picks the way, so a retreat goes round a wall instead
//! of into it. Lane A replaces the inside of [`Combat::assess`] with the
//! full controller; the frame arbitration around it stays.

use super::intent::{Intent, Look};
use super::route::{into_deeper_water, Route, Step};
use super::tracks::Tracks;
use client_core::core::ClientCore;
use protocol::EntityState;
use sim_core::input::BTN_SPRINT;
use sim_core::limits::TICK_HZ;
use sim_core::movement::POS_XZ_Q;
use sim_core::yaw_dir;

/// A retreat backs away this long after its last renewal.
pub const FLEE_TICKS: u32 = 6 * TICK_HZ;
/// A retreat routes to a point this far along its bearing, and on again
/// from wherever it arrives.
pub const FLEE_M: f32 = 24.0;
/// A pursuer this close renews a retreat whatever it is doing.
pub const PURSUER_NEAR_M: f32 = 32.0;
/// A goal paused by a fight shorter than this picks up where it left off;
/// a longer one ends it.
pub const RESUME_TICKS: u32 = 20 * TICK_HZ;

/// A retreat in progress.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Retreat {
    /// Tick of the last renewal.
    pub start: u32,
    /// The world bearing it backs away along.
    pub away: u16,
    /// Where the route is taking it, metres; set on the first frame.
    to: Option<[f32; 2]>,
}

/// What the reflex made of this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Assess {
    /// Nothing to answer: run the goal.
    Calm,
    /// Answering: this frame belongs to the fight, and the goal waits.
    Fight(Intent),
    /// The fight ended this frame, backing away along this bearing.
    Over { away: u16 },
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Combat {
    retreat: Option<Retreat>,
}

impl Combat {
    pub const fn new() -> Self {
        Self { retreat: None }
    }

    /// A new body: nothing it was running from is behind it now.
    pub fn forget(&mut self) {
        self.retreat = None;
    }

    /// The retreat running now, as (last renewal, bearing).
    pub fn retreat(&self) -> Option<(u32, u16)> {
        self.retreat.map(|r| (r.start, r.away))
    }

    /// In a fight: a frame now goes to the reflex, not the goal.
    pub fn engaged(&self) -> bool {
        self.retreat.is_some()
    }

    /// A blow landed: back away from its bearing, starting now. A received
    /// damage bearing is the human's hit indicator, not a position.
    pub fn on_hurt(&mut self, tick: u32, away: u16) {
        self.run(tick, away);
    }

    /// Run from a body the eyes found (a flee goal).
    pub fn flee(&mut self, tick: u32, away: u16) {
        self.run(tick, away);
    }

    fn run(&mut self, tick: u32, away: u16) {
        self.retreat = Some(Retreat {
            start: tick,
            away,
            to: None,
        });
    }

    /// One frame of the reflex.
    pub fn assess(
        &mut self,
        core: &mut ClientCore,
        body: &EntityState,
        tracks: &Tracks,
        route: &mut Route,
        tick: u32,
    ) -> Assess {
        let Some(mut r) = self.retreat else {
            return Assess::Calm;
        };
        if tracks.pursuer_in_sight(PURSUER_NEAR_M) {
            r.start = tick;
        }
        if tick.wrapping_sub(r.start) >= FLEE_TICKS {
            self.retreat = None;
            return Assess::Over { away: r.away };
        }
        let (x, z) = (body.qx as f32 * POS_XZ_Q, body.qz as f32 * POS_XZ_Q);
        if into_deeper_water(core, body, r.away) {
            r.away = r.away.wrapping_add(1 << 14);
            r.to = None;
        }
        let ahead = |away: u16| {
            let (dx, dz) = yaw_dir(away);
            [x + dx * FLEE_M, z + dz * FLEE_M]
        };
        let to = *r.to.get_or_insert_with(|| ahead(r.away));
        let mut step = route.to(core, body, to, 2.0, true, tick);
        if step == Step::Arrived {
            // Still in danger and out of road: on along the same bearing.
            let on = ahead(r.away);
            r.to = Some(on);
            step = route.to(core, body, on, 2.0, true, tick);
        }
        self.retreat = Some(r);
        // Keep the danger in view while retreating: turn to face back and
        // back away along the route, whatever the view is doing.
        let travel = match step {
            Step::Walk { yaw, .. } => yaw,
            Step::Arrived | Step::Blocked | Step::Wait => r.away,
        };
        Assess::Fight(Intent {
            look: Look::Heading(r.away.wrapping_add(1 << 15)),
            travel: Some(travel),
            buttons: BTN_SPRINT,
            ..Intent::IDLE
        })
    }
}
