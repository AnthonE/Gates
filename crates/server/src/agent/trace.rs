//! What a goal that came to nothing looked like from inside the body: the
//! route's last word on it, the trip into the team's base it was on, the
//! floor under it, the doors about and the teammates at its elbow. Kept
//! for the bug report its mind may file (`bugs.rs`), which carries the
//! body's position and the seed already; never part of the mind's turn.
//!
//! A few are kept, newest first, so a report written a goal or two after
//! the one that failed still has it. Copy and bounded: nothing is
//! allocated until a report asks for the words.

use crate::agent::doors::HOP_M;
use crate::agent::floors::Layer;
use crate::agent::supply::HomeTrace;
use crate::mind::{Goal, Why};
use serde_json::{json, Value};
use sim_core::limits::TICK_HZ;
use sim_core::nav::Plan;

/// Failures kept.
pub const FAIL_TRACES: usize = 4;
/// Teammates in sight this near a failure are counted: at its elbow, where
/// one could be what is in the way.
pub const MATE_NEAR_M: f32 = 3.0;

/// One goal that failed, as the body saw it.
#[derive(Clone, Copy, Debug)]
pub struct FailTrace {
    pub goal: Goal,
    pub why: Why,
    pub secs: u32,
    /// The tick it failed.
    pub at: u32,
    /// The route's last `Blocked` while it ran (`Route::blocked_by`): why,
    /// the plan in hand then, and when.
    pub route: Option<(&'static str, Option<Plan>, u32)>,
    /// The trip into the team's base it walked, if it walked one.
    pub home: Option<HomeTrace>,
    /// The box that trip was seeing to: its storey, and metres off.
    pub chest: Option<(u8, f32)>,
    /// Teammates in sight within [`MATE_NEAR_M`].
    pub mates_near: u8,
    /// The floor under it and the doors within [`HOP_M`]: filled on
    /// the frame it failed in, which has the client's mirror in hand.
    pub layer: Option<Layer>,
    pub doors_near: Option<u8>,
}

impl FailTrace {
    /// The words a report carries, `now` the tick it is written at.
    pub fn json(&self, now: u32) -> Value {
        json!({
            "goal": self.goal.label().as_str(),
            "why": self.why.text(),
            "seconds": self.secs,
            "ago_s": now.wrapping_sub(self.at) / TICK_HZ,
            "route": self.route.map(|(by, plan, at)| json!({
                "blocked_by": by,
                "plan": plan.map(plan_word),
                "before_s": self.at.wrapping_sub(at) / TICK_HZ,
            })),
            "home_trip": self.home.map(|h| json!({
                "trip": h.trip,
                "leg": h.leg,
                "failed_at": (!h.fail_site.is_empty()).then_some(h.fail_site),
                "doors_inside": h.passed,
                "doors_given_up": h.tried,
                "box_tries": h.tries,
            })),
            "box": self.chest.map(|(level, m)| json!({
                "level": level,
                "metres": (f64::from(m) * 10.0).round() / 10.0,
            })),
            "standing": match self.layer {
                Some(Layer::Ground) => json!("ground"),
                Some(Layer::Deck(y)) => json!(format!("a floor at {y:.1} m")),
                None => Value::Null,
            },
            "doors_near": self.doors_near,
            "teammates_near": self.mates_near,
        })
    }
}

/// The last few failures, newest first.
#[derive(Clone, Copy, Debug, Default)]
pub struct Failures {
    ring: [Option<FailTrace>; FAIL_TRACES],
    /// The slot the next one goes in.
    next: usize,
}

impl Failures {
    pub fn push(&mut self, t: FailTrace) {
        self.ring[self.next] = Some(t);
        self.next = (self.next + 1) % FAIL_TRACES;
    }

    /// Newest first.
    pub fn iter(&self) -> impl Iterator<Item = &FailTrace> + '_ {
        (1..=FAIL_TRACES)
            .filter_map(move |i| self.ring[(self.next + FAIL_TRACES - i) % FAIL_TRACES].as_ref())
    }

    pub fn last(&self) -> Option<&FailTrace> {
        self.iter().next()
    }

    /// The newest, while the facts its frame fills in are still to come.
    pub fn unfilled(&mut self) -> Option<&mut FailTrace> {
        let i = (self.next + FAIL_TRACES - 1) % FAIL_TRACES;
        self.ring[i].as_mut().filter(|t| t.layer.is_none())
    }

    /// The trace a bug report carries (`seen.trace`): the last failure,
    /// the ones before it, and the route's counts.
    pub fn json(&self, now: u32, route: Value, wedges: u64) -> Value {
        json!({
            "last_failure": self.last().map(|t| t.json(now)),
            "earlier_failures": self.iter().skip(1).map(|t| t.json(now)).collect::<Vec<_>>(),
            "near_m": {"doors": HOP_M, "teammates": MATE_NEAR_M},
            "route": route,
            "wedges": wedges,
        })
    }
}

fn plan_word(p: Plan) -> &'static str {
    match p {
        Plan::Direct => "direct",
        Plan::Found => "found",
        Plan::Toward => "toward",
        Plan::Partial => "partial",
        Plan::Unreachable => "unreachable",
        Plan::NoWay => "no_way",
        Plan::Deferred => "deferred",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trace(at: u32) -> FailTrace {
        FailTrace {
            goal: Goal::GoHome,
            why: Why::NoWayIn,
            secs: 0,
            at,
            route: Some(("plan_no_way", Some(Plan::NoWay), at - TICK_HZ)),
            home: None,
            chest: Some((0, 1.04)),
            mates_near: 1,
            layer: None,
            doors_near: None,
        }
    }

    #[test]
    fn the_ring_keeps_the_newest_few_and_says_each_one() {
        let mut f = Failures::default();
        assert!(f.last().is_none() && f.unfilled().is_none());
        for at in 1..=(FAIL_TRACES as u32 + 2) {
            f.push(trace(at * 100));
        }
        let ats: Vec<u32> = f.iter().map(|t| t.at).collect();
        assert_eq!(ats, [600, 500, 400, 300]);
        f.unfilled().unwrap().layer = Some(Layer::Ground);
        assert!(f.unfilled().is_none(), "filled once");
        let v = f.json(600 + 3 * TICK_HZ, json!({}), 2);
        let last = &v["last_failure"];
        assert_eq!(last["why"], Why::NoWayIn.text());
        assert_eq!(last["ago_s"], 3);
        assert_eq!(last["route"]["blocked_by"], "plan_no_way");
        assert_eq!(last["route"]["plan"], "no_way");
        assert_eq!(last["route"]["before_s"], 1);
        assert_eq!(last["box"]["metres"], 1.0);
        assert_eq!(last["standing"], "ground");
        assert_eq!(v["earlier_failures"].as_array().unwrap().len(), 3);
        assert!(v["earlier_failures"][0]["standing"].is_null());
        assert_eq!(last["teammates_near"], 1);
        assert_eq!(v["near_m"]["teammates"], f64::from(MATE_NEAR_M));
    }
}
