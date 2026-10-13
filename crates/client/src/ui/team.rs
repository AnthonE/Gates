//! The team (wire v108), as text and arithmetic: the panel's rows, the
//! prompts, and which teammates get a mark over their heads.
//!
//! Rust's shape (`server::team` has the rest): teammates are listed bottom
//! left, green awake, grey asleep, red down; a green mark rides over each
//! one within a hundred metres whatever stands between; the map shows them
//! wherever they are. Pure so the rules are testable without a window.

use protocol::{TeamView, TEAM_ASLEEP, TEAM_AWAKE, TEAM_DEAD};

/// How far a teammate's mark is drawn over their head, metres: Rust's
/// hundred. Past it, the map has them.
pub const MARK_REACH_M: f32 = 100.0;

/// The invite key's line, beside the other prompts.
pub fn invite_prompt(who: &str) -> String {
    format!("[N] INVITE {} TO TEAM", who.to_uppercase())
}

/// The line an invite waiting on you shows.
pub fn invited_line(who: &str) -> String {
    format!("{who} invited you to their team  ·  [Y] join  ·  /team decline")
}

/// What a member's state reads as beside their name.
pub fn state_word(state: u8) -> &'static str {
    match state {
        TEAM_ASLEEP => "asleep",
        TEAM_DEAD => "down",
        _ => "",
    }
}

/// One line of the panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub text: String,
    pub state: u8,
    pub me: bool,
}

/// The panel: a header, then every member, leader first marked `*`, with
/// how far off they are; you read as "you". `name(id)` is the chat log's
/// label (`ui::names::of`). Empty with no team.
pub fn rows(team: &TeamView, me: u32, at: [f32; 2], name: impl Fn(u32) -> String) -> Vec<Row> {
    if team.count == 0 {
        return Vec::new();
    }
    let mut out = vec![Row {
        text: format!("TEAM {}/{}", team.count, protocol::TEAM_MAX),
        state: TEAM_AWAKE,
        me: false,
    }];
    let mut members: Vec<_> = team.members().to_vec();
    // The leader first, then the shard's order.
    members.sort_by_key(|m| m.id != team.leader);
    for m in members {
        let lead = if m.id == team.leader { "* " } else { "  " };
        let text = if m.id == me {
            format!("{lead}you")
        } else {
            let d = distance(at, m.x_m, m.z_m);
            let word = state_word(m.state);
            let word = if word.is_empty() {
                String::new()
            } else {
                format!("  {word}")
            };
            format!("{lead}{}  {:.0} m{word}", name(m.id), d)
        };
        out.push(Row {
            text,
            state: m.state,
            me: m.id == me,
        });
    }
    out
}

/// Whether a teammate standing `d` metres off in `state` gets a mark over
/// their head: up or down, within reach. A sleeper is on the map only.
pub fn marked(state: u8, d: f32) -> bool {
    state != TEAM_ASLEEP && d <= MARK_REACH_M
}

/// A teammate's mark: their name and how far.
pub fn mark_label(who: &str, state: u8, d: f32) -> String {
    match state_word(state) {
        "" => format!("{who}  {d:.0} m"),
        w => format!("{who}  {d:.0} m  {w}"),
    }
}

/// Metres on the ground from `at` to a member's whole-metre position.
pub fn distance(at: [f32; 2], x_m: u16, z_m: u16) -> f32 {
    let (dx, dz) = (f32::from(x_m) + 0.5 - at[0], f32::from(z_m) + 0.5 - at[1]);
    (dx * dx + dz * dz).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::TeamMember;

    fn team() -> TeamView {
        let mut v = TeamView::NONE;
        v.leader = 2;
        v.count = 3;
        v.members[0] = TeamMember {
            id: 1,
            state: TEAM_AWAKE,
            x_m: 100,
            z_m: 100,
        };
        v.members[1] = TeamMember {
            id: 2,
            state: TEAM_DEAD,
            x_m: 130,
            z_m: 140,
        };
        v.members[2] = TeamMember {
            id: 3,
            state: TEAM_ASLEEP,
            x_m: 0,
            z_m: 0,
        };
        v
    }

    #[test]
    fn the_panel_puts_the_leader_first_and_names_you() {
        let r = rows(&team(), 1, [100.5, 100.5], |id| format!("#{id}"));
        let text: Vec<_> = r.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(
            text,
            [
                "TEAM 3/8",
                "* #2  50 m  down",
                "  you",
                "  #3  141 m  asleep"
            ]
        );
        assert!(r[2].me);
        assert!(rows(&TeamView::NONE, 1, [0.0, 0.0], |_| String::new()).is_empty());
    }

    #[test]
    fn marks_ride_over_the_living_and_the_down_within_reach() {
        assert!(marked(TEAM_AWAKE, 99.0));
        assert!(marked(TEAM_DEAD, 10.0));
        assert!(!marked(TEAM_AWAKE, 101.0));
        assert!(!marked(TEAM_ASLEEP, 5.0));
        assert_eq!(mark_label("Ash", TEAM_AWAKE, 12.4), "Ash  12 m");
        assert_eq!(mark_label("Ash", TEAM_DEAD, 12.6), "Ash  13 m  down");
    }
}
