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

pub struct Autopilot<P> {
    primary: P,
    scripted: Scripted,
    /// Answer [`friend_away`] requests without asking `primary`.
    friend_gate: bool,
}

impl<P: DecisionSource> Autopilot<P> {
    pub fn new(primary: P, friend_gate: bool) -> Self {
        Self {
            primary,
            scripted: Scripted::default(),
            friend_gate,
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
        if self.friend_gate && friend_away(s) {
            return self.fly("my friend is away", s);
        }
        match self.primary.decide(s) {
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
        let mut a = Autopilot::new(Fails(0), false);
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
        let mut gated = Autopilot::new(Answers(0), true);
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
}
