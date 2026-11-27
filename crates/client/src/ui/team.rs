//! The team (wire v108), as text and arithmetic: the panel's rows, the
//! prompts, and which teammates get a mark over their heads.
//!
//! Rust's shape (`server::team` has the rest): teammates are listed bottom
//! left, green awake, grey asleep, red down; a green mark rides over each
//! one within a hundred metres whatever stands between; the map shows them
//! wherever they are. Pure so the rules are testable without a window.

use protocol::{
    TeamView, TEAM_ASLEEP, TEAM_AWAKE, TEAM_DEAD, TEAM_OP_ACCEPT, TEAM_OP_DECLINE, TEAM_OP_KICK,
    TEAM_OP_LEAVE, TEAM_OP_PROMOTE,
};

/// How far a teammate's mark is drawn over their head, metres: Rust's
/// hundred. Past it, the map has them.
pub const MARK_REACH_M: f32 = 100.0;

/// The invite key's line, beside the other prompts.
pub fn invite_prompt(who: &str) -> String {
    format!("[N] INVITE {} TO TEAM", who.to_uppercase())
}

/// The line an invite waiting on you shows.
pub fn invited_line(who: &str) -> String {
    format!("{who} invited you to their team  ·  [Y] join  ·  [M] team")
}

/// What a member's state reads as beside their name.
pub fn state_word(state: u8) -> &'static str {
    match state {
        TEAM_ASLEEP => "asleep",
        TEAM_DEAD => "down",
        _ => "",
    }
}

/// One line of the panel; `id` is the member's (0 for the header).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub text: String,
    pub state: u8,
    pub me: bool,
    pub id: u32,
}

/// A press on the TEAM screen (`M`): the team verb it sends, as the shard
/// takes it (`server::team`). The shard's team view is the answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Button {
    pub label: &'static str,
    pub op: u8,
    pub target: u32,
}

/// The buttons beside a member: kick and promote, for the leader only, on
/// anyone but themselves.
pub fn member_buttons(team: &TeamView, me: u32, member: u32) -> Vec<Button> {
    if team.count == 0 || team.leader != me || member == me || !team.has(member) {
        return Vec::new();
    }
    vec![
        Button {
            label: "PROMOTE",
            op: TEAM_OP_PROMOTE,
            target: member,
        },
        Button {
            label: "KICK",
            op: TEAM_OP_KICK,
            target: member,
        },
    ]
}

/// The screen's own buttons: leave, on a team; join or decline, with an
/// invite waiting and no team.
pub fn team_buttons(team: &TeamView) -> Vec<Button> {
    if team.count > 0 {
        return vec![Button {
            label: "LEAVE TEAM",
            op: TEAM_OP_LEAVE,
            target: 0,
        }];
    }
    if team.invite != 0 {
        return vec![
            Button {
                label: "JOIN",
                op: TEAM_OP_ACCEPT,
                target: team.invite,
            },
            Button {
                label: "DECLINE",
                op: TEAM_OP_DECLINE,
                target: 0,
            },
        ];
    }
    Vec::new()
}

/// What the status line says once a press has gone out.
pub fn sent_line(op: u8) -> &'static str {
    match op {
        TEAM_OP_LEAVE => "leaving the team…",
        TEAM_OP_KICK => "kicking them…",
        TEAM_OP_PROMOTE => "handing them the lead…",
        TEAM_OP_ACCEPT => "joining…",
        TEAM_OP_DECLINE => "declined",
        _ => "",
    }
}

/// An order button (`client_core::crew`): its label and the team line it
/// sends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrderLine {
    pub label: &'static str,
    pub line: String,
}

/// A row of the ORDERS section: who it is for (`ALL`, or an agent's name,
/// `id` 0 for all) and its buttons.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrderRow {
    pub who: String,
    pub id: u32,
    pub orders: Vec<OrderLine>,
}

/// The order wheel (`docs/PLAYERS.md`): the team lines a crew's orders
/// are, typed for you. `crew_of` is whom an agent serves (`EventMsg::Crew`):
/// `Some(you)` takes your orders; `None` is an agent with no commander,
/// which takes any teammate's, or one whose commander is away, off the team
/// or an agent, which takes only theirs (the shard sends 0 for all of
/// these). Nothing unless a teammate agent is `None` or yours: then an ALL
/// row with every order, and a FOLLOW and STAY row by its bare name
/// (`name_of`: the tag's name, never the streamer-mode one) per agent
/// crewed to you, when that name can open a line (`crew::addressable`)
/// and no other member, you included, goes by it.
pub fn order_rows(
    team: &TeamView,
    me: u32,
    is_agent: impl Fn(u32) -> bool,
    crew_of: impl Fn(u32) -> Option<u32>,
    name_of: impl Fn(u32) -> Option<String>,
) -> Vec<OrderRow> {
    use client_core::crew;
    if team.count == 0 {
        return Vec::new();
    }
    let mut members: Vec<_> = team.members().iter().map(|m| m.id).collect();
    members.sort_by_key(|&id| id != team.leader);
    let crew: Vec<u32> = members
        .into_iter()
        .filter(|&id| id != me && is_agent(id) && crew_of(id).is_none_or(|of| of == me))
        .collect();
    if crew.is_empty() {
        return Vec::new();
    }
    // Only one sure to be yours gets a row by name: a `None` may be
    // another's, whose commander is away.
    let named: Vec<u32> = crew
        .into_iter()
        .filter(|&id| crew_of(id) == Some(me))
        .collect();
    let mut out = vec![OrderRow {
        who: "ALL".to_string(),
        id: 0,
        orders: crew::ORDERS
            .iter()
            .map(|&o| OrderLine {
                label: o.label,
                line: crew::to_all(o),
            })
            .collect(),
    }];
    // A body drops a line opening with any teammate's name (`for_me`), and
    // names are not unique: one another member shares gets no row.
    let shared = |id: u32, name: &str| {
        team.members()
            .iter()
            .any(|m| m.id != id && name_of(m.id).is_some_and(|n| n.eq_ignore_ascii_case(name)))
    };
    for id in named {
        let Some(name) = name_of(id).filter(|n| crew::addressable(n) && !shared(id, n)) else {
            continue;
        };
        out.push(OrderRow {
            orders: crew::ONE
                .iter()
                .map(|&o| OrderLine {
                    label: o.label,
                    line: crew::to_one(&name, o),
                })
                .collect(),
            who: name,
            id,
        });
    }
    out
}

/// What the status line says once an order has gone out.
pub fn ordered_line(line: &str) -> String {
    format!("ordered: {line}")
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
        id: 0,
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
            id: m.id,
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

// ---------------------------------------------------------------------------
// Giving a sleeping bag (Rust's assign bag, `ActionMsg::BagGive`)
// ---------------------------------------------------------------------------

/// The press beside a teammate while the TEAM screen is up for a bag.
pub const GIVE_BAG: &str = "GIVE BAG";

/// What `E` on your own bag says with nobody to give it to.
pub const NO_BAG_TAKER: &str = "no teammate to give it to  ·  [N] invites one";

/// The line over the rows while a bag is being given.
pub const BAG_LINE: &str = "Give this sleeping bag to a teammate: it becomes theirs to wake on.";

/// Who your sleeping bag can go to, in the panel's order (leader first):
/// every teammate but you, asleep or down included, since a bag is where
/// they wake next. The shard takes a give only for a teammate
/// (`server::team`), so nobody else is offered.
pub fn bag_takers(team: &TeamView, me: u32) -> Vec<u32> {
    let mut out: Vec<u32> = team
        .members()
        .iter()
        .map(|m| m.id)
        .filter(|&id| id != me)
        .collect();
    out.sort_by_key(|&id| id != team.leader);
    out
}

/// What the status line says once a give has gone out.
pub fn bag_sent_line(who: &str) -> String {
    format!("giving your bag to {who}…")
}

/// The line a bag's receiver reads (`EventMsg::BagGiven`): who gave it, as
/// every surface draws them (`ui::names::shown`, streamer-safe).
pub fn bag_given_line(core: &client_core::core::ClientCore, by: u32) -> String {
    format!(
        "{} GAVE YOU A SLEEPING BAG",
        crate::ui::names::shown(core, by)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::TeamMember;

    /// A bag given, off the wire, is the receiver's line naming the giver
    /// as every surface does: by their tag once it has landed, by id before.
    #[test]
    fn a_bag_given_reads_who_gave_it() {
        use client_core::core::ClientCore;
        const GIVER: u32 = 259;
        let mut core = Box::new(ClientCore::new(42, 1, 0));
        let mut buf = [0u8; protocol::MAX_EVENT_MSG_BYTES];
        let len = protocol::encode_event_bag_given(GIVER, &mut buf).unwrap();
        core.on_stream(&buf[..len]).expect("the give applies");
        assert_eq!(core.pop_bag_given(), Some(GIVER));
        assert_eq!(core.pop_bag_given(), None, "one bag, one line");
        assert_eq!(bag_given_line(&core, GIVER), "#259 GAVE YOU A SLEEPING BAG");

        let address =
            protocol::Address::from_hex(b"0x7e5f4552091a69125d5dfcb7b8c2659029395bdf").unwrap();
        let name = protocol::Name::new("Quillon").unwrap();
        let len =
            protocol::encode_event_tag(GIVER, &address, &name, 0, &protocol::Guild::NONE, &mut buf)
                .unwrap();
        core.on_stream(&buf[..len]).expect("the tag applies");
        assert_eq!(
            bag_given_line(&core, GIVER),
            "Quillon GAVE YOU A SLEEPING BAG"
        );
    }

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

    /// The leader gets kick and promote on everyone else; a member gets
    /// neither; anyone on a team can leave; an invite offers join and
    /// decline, with join aimed at whoever invited.
    #[test]
    fn the_team_screen_offers_only_what_the_shard_takes() {
        let t = team();
        let ops = |b: Vec<Button>| b.iter().map(|b| (b.op, b.target)).collect::<Vec<_>>();
        assert_eq!(
            ops(member_buttons(&t, 2, 1)),
            [(TEAM_OP_PROMOTE, 1), (TEAM_OP_KICK, 1)]
        );
        assert!(member_buttons(&t, 2, 2).is_empty(), "not on yourself");
        assert!(
            member_buttons(&t, 1, 3).is_empty(),
            "a member is not the leader"
        );
        assert!(member_buttons(&t, 2, 9).is_empty(), "not on a stranger");
        assert_eq!(ops(team_buttons(&t)), [(TEAM_OP_LEAVE, 0)]);
        let mut invited = TeamView::NONE;
        invited.invite = 7;
        assert_eq!(
            ops(team_buttons(&invited)),
            [(TEAM_OP_ACCEPT, 7), (TEAM_OP_DECLINE, 0)]
        );
        assert!(team_buttons(&TeamView::NONE).is_empty());
        let r = rows(&t, 1, [0.0, 0.0], |id| format!("#{id}"));
        assert_eq!(r.iter().map(|r| r.id).collect::<Vec<_>>(), [0, 2, 1, 3]);
    }

    /// A bag goes to every teammate but you, leader first, whatever their
    /// state; with no team there is nobody.
    #[test]
    fn a_bag_is_offered_to_every_teammate_but_you() {
        assert_eq!(bag_takers(&team(), 1), [2, 3]);
        assert_eq!(bag_takers(&team(), 2), [1, 3]);
        assert!(bag_takers(&TeamView::NONE, 1).is_empty());
        let mut alone = TeamView::NONE;
        alone.leader = 1;
        alone.count = 1;
        alone.members[0] = TeamMember {
            id: 1,
            state: TEAM_AWAKE,
            x_m: 0,
            z_m: 0,
        };
        assert!(bag_takers(&alone, 1).is_empty(), "a team of one");
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
