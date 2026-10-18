//! The autopilot: the scripted policy standing behind another source, so
//! a body never stands still for want of an answer.
//!
//! Two cases, both said in the reason so a log shows who decided:
//!
//! - **The source could not answer** — an MCP harness still thinking past
//!   its deadline, or gone; a model call that failed. The scripted policy
//!   answers that one request, and the source is asked again at the next.
//! - **Its friend is away** (`--friend`, [`friend_away`]). The scripted
//!   policy plays and the source is not asked at all: a buddy costs model
//!   calls only while the person it plays with is on the island.
//!
//! `mind.rs`'s rule (no substituted decisions) is about a source that
//! ANSWERED being overruled; here no answer came, and the substitute is
//! named as one.

use crate::mind::{Choice, DecisionSource, Reason, Scripted, SourceKind, Summary};

/// Its friend is named and not on the island: the autopilot's own turn.
pub fn friend_away(s: &Summary) -> bool {
    s.friend.set && !s.friend.online
}

/// No person to play for: no friend on the island, no human in view, and
/// none heard since the last request (`--alone autopilot`). Other agents
/// do not count, so a crowd of them costs nothing while the island is empty.
pub fn alone(s: &Summary) -> bool {
    !(s.friend.set && s.friend.online) && s.humans.count == 0 && !s.heard.from_human()
}

/// Which requests the scripted policy answers without asking the model,
/// and what it says it is doing.
#[derive(Clone, Copy)]
pub struct Gate {
    pub rule: fn(&Summary) -> bool,
    pub why: fn(&Summary) -> &'static str,
}

pub const FRIEND_AWAY: Gate = Gate {
    rule: friend_away,
    why: |_| "my friend is away",
};
/// `--alone autopilot`. A society member's crew coming together at THE GATE
/// is the scripted policy's too, whoever is near: the meeting is what makes
/// it a crew, and a mind that wanders off from it leaves the crew unformed.
pub const ALONE: Gate = Gate {
    rule: |s| alone(s) || crate::mind::crew_forming(s),
    why: |s| {
        if crate::mind::crew_forming(s) {
            "the crew is coming together"
        } else {
            "no one is near"
        }
    },
};

pub struct Autopilot<P> {
    primary: P,
    scripted: Scripted,
    /// Requests it answers without asking `primary`.
    gate: Option<Gate>,
}

impl<P: DecisionSource> Autopilot<P> {
    pub fn new(primary: P, gate: Option<Gate>) -> Self {
        Self {
            primary,
            scripted: Scripted::default(),
            gate,
        }
    }

    fn fly(&mut self, why: &str, s: &Summary) -> Result<Choice, String> {
        let mut choice = self.scripted.decide(s)?;
        choice.reason =
            Reason::from_text(&format!("autopilot ({why}): {}", choice.reason.as_str()));
        Ok(choice)
    }
}

impl<P: DecisionSource> DecisionSource for Autopilot<P> {
    fn kind(&self) -> SourceKind {
        self.primary.kind()
    }

    fn decide(&mut self, s: &Summary) -> Result<Choice, String> {
        if let Some(g) = self.gate.filter(|g| (g.rule)(s)) {
            return self.fly((g.why)(s), s);
        }
        let answer = self.primary.decide(s);
        // A plan the answer left is the autopilot's from now on, for the
        // turns the source misses.
        if let Some(plan) = self.primary.take_plan() {
            self.scripted.set_plan(plan);
        }
        match answer {
            Ok(choice) => Ok(choice),
            Err(e) => self.fly(&e, s),
        }
    }

    fn take_note(&mut self) -> Option<crate::bugs::Note> {
        self.primary.take_note()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mind::{FriendSense, Goal, Say};

    struct Fails(u32);
    impl DecisionSource for Fails {
        fn kind(&self) -> SourceKind {
            SourceKind::External
        }
        fn decide(&mut self, _: &Summary) -> Result<Choice, String> {
            self.0 += 1;
            Err("external agent did not answer in time".into())
        }
    }

    struct Answers(u32);
    impl DecisionSource for Answers {
        fn kind(&self) -> SourceKind {
            SourceKind::External
        }
        fn decide(&mut self, _: &Summary) -> Result<Choice, String> {
            self.0 += 1;
            Ok(Choice {
                goal: Goal::GatherWood,
                confidence: 1.0,
                reason: Reason::from_text("mine"),
                input_tokens: 0,
                output_tokens: 0,
                say: Say::Quiet,
                code: None,
            })
        }
    }

    fn summary() -> Summary {
        let mut s = Summary::EMPTY;
        for g in Goal::FIXED {
            s.offer(g);
        }
        s
    }

    #[test]
    fn a_missing_answer_is_flown_and_said_so() {
        let mut a = Autopilot::new(Fails(0), None);
        let c = a.decide(&summary()).unwrap();
        assert!(
            c.reason.as_str().starts_with("autopilot ("),
            "{}",
            c.reason.as_str()
        );
        assert_eq!(a.primary.0, 1, "the source is still asked every time");
        assert_eq!(a.kind(), SourceKind::External);
    }

    #[test]
    fn a_friend_away_never_reaches_the_model() {
        let mut s = summary();
        s.friend = FriendSense {
            set: true,
            ..FriendSense::NONE
        };
        let mut gated = Autopilot::new(Answers(0), Some(FRIEND_AWAY));
        let c = gated.decide(&s).unwrap();
        assert!(c.reason.as_str().contains("my friend is away"));
        assert_eq!(gated.primary.0, 0, "no model call while the friend is away");
        assert!(friend_away(&s));
        s.friend.online = true;
        assert_eq!(gated.decide(&s).unwrap().goal, Goal::GatherWood);
        assert_eq!(gated.primary.0, 1);
        assert!(
            !friend_away(&Summary::EMPTY),
            "no friend named is never away"
        );
    }

    #[test]
    fn alone_it_asks_no_model_and_a_person_wakes_it() {
        let mut s = summary();
        s.players.add(10.0, 0);
        let mut gated = Autopilot::new(Answers(0), Some(ALONE));
        let c = gated.decide(&s).unwrap();
        assert!(c.reason.as_str().contains("no one is near"));
        assert_eq!(gated.primary.0, 0, "another agent in view is not a person");

        s.humans.add(30.0, 2);
        gated.decide(&s).unwrap();
        assert_eq!(gated.primary.0, 1, "a person in view wakes the mind");

        let mut heard = summary();
        heard.heard.push(crate::mind::HeardLine {
            agent: true,
            ..crate::mind::HeardLine::EMPTY
        });
        assert!(alone(&heard), "an agent talking is not a person");
        heard.heard.push(crate::mind::HeardLine::EMPTY);
        assert!(!alone(&heard), "a person talking wakes it");

        let mut friend = summary();
        friend.friend = FriendSense {
            set: true,
            online: true,
            ..FriendSense::NONE
        };
        assert!(
            !alone(&friend),
            "a friend on the island, anywhere, wakes it"
        );
    }

    /// A plan its source leaves is what the autopilot plays on the turns
    /// the source misses: the first step not yet done, while it is offered;
    /// a step done (the pack holds its count) gives way to the next.
    #[test]
    fn the_turns_it_misses_follow_the_plan_its_source_left() {
        use crate::mind::{Name, Plan, PlanStep};
        struct Away(Option<Plan>);
        impl DecisionSource for Away {
            fn kind(&self) -> SourceKind {
                SourceKind::External
            }
            fn decide(&mut self, _: &Summary) -> Result<Choice, String> {
                Err("the harness is away".into())
            }
            fn take_plan(&mut self) -> Option<Plan> {
                self.0.take()
            }
        }
        let mut plan = Plan::default();
        plan.steps[0] = Some(PlanStep {
            goal: Goal::GatherWood,
            until: Some((Name::new(b"Wood").unwrap(), 100)),
        });
        plan.steps[1] = Some(PlanStep {
            goal: Goal::Build,
            until: None,
        });
        let mut s = Summary::EMPTY;
        for g in [Goal::Explore, Goal::GatherWood, Goal::Build, Goal::Wait] {
            s.offer(g);
        }
        let mut a = Autopilot::new(Away(Some(plan)), None);
        let c = a.decide(&s).unwrap();
        assert_eq!(c.goal, Goal::GatherWood, "{}", c.reason.as_str());
        assert!(c.reason.as_str().contains("the plan my mind left"));
        assert_eq!(plan.next(&s), Some(Goal::GatherWood));
        let mut bare = Summary::EMPTY;
        bare.offer(Goal::Build);
        assert_eq!(
            plan.next(&bare),
            None,
            "a step not on offer does not skip ahead"
        );
    }

    /// A crewmate not yet on its founder's team goes to THE GATE on the
    /// scripted policy's word, a person in view or not; once it is on the
    /// team, a person near wakes its own mind.
    #[test]
    fn a_crew_coming_together_is_never_the_model_s() {
        let mut s = summary();
        s.humans.add(10.0, 2);
        s.society = crate::mind::SocietySense {
            set: true,
            role: crate::agent::society::Role::Farmer,
            ..crate::mind::SocietySense::NONE
        };
        let mut gated = Autopilot::new(Answers(0), Some(ALONE));
        let c = gated.decide(&s).unwrap();
        assert!(c.reason.as_str().contains("the crew is coming together"));
        assert_eq!(gated.primary.0, 0);
        s.society.founded = true;
        gated.decide(&s).unwrap();
        assert_eq!(gated.primary.0, 1, "a person in view wakes it");
    }
}
