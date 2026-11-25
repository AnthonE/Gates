//! The TEAM screen's ORDERS section (`ui::team::order_rows`): which rows a
//! player gets and the team lines its buttons send. Headless, code tier.
//! That the shard reads each line as the order it is labelled is
//! `crates/server/tests/crew_orders.rs`.

use client::ui::team::{order_rows, OrderRow};
use protocol::{TeamMember, TeamView, TEAM_AWAKE};

const ME: u32 = 1;
const MILLER: u32 = 5;
const ASH: u32 = 6;
const BOB: u32 = 7;

fn team(ids: &[u32]) -> TeamView {
    let mut v = TeamView::NONE;
    v.leader = ids[0];
    v.count = ids.len() as u8;
    for (slot, &id) in v.members.iter_mut().zip(ids) {
        *slot = TeamMember {
            id,
            state: TEAM_AWAKE,
            x_m: 0,
            z_m: 0,
        };
    }
    v
}

fn name(id: u32) -> Option<String> {
    match id {
        MILLER => Some("Miller-3".into()),
        ASH => Some("Ash".into()),
        BOB => Some("Bob".into()),
        _ => None,
    }
}

fn rows(t: &TeamView, agents: &[u32], crew_of: impl Fn(u32) -> Option<u32>) -> Vec<OrderRow> {
    order_rows(t, ME, |id| agents.contains(&id), crew_of, name)
}

fn lines(r: &OrderRow) -> Vec<&str> {
    r.orders.iter().map(|o| o.line.as_str()).collect()
}

#[test]
fn a_team_of_people_has_no_orders() {
    let t = team(&[ME, BOB, ASH]);
    assert!(rows(&t, &[], |_| None).is_empty());
    assert!(rows(&TeamView::NONE, &[MILLER], |_| None).is_empty());
    // An agent you are not on a team with is not yours to order.
    assert!(rows(&team(&[ME, BOB]), &[MILLER], |_| None).is_empty());
}

#[test]
fn an_agent_teammate_gets_an_all_row_and_its_own() {
    let t = team(&[ME, MILLER, BOB]);
    let r = rows(&t, &[MILLER], |_| Some(ME));
    assert_eq!(r.len(), 2);
    assert_eq!((r[0].who.as_str(), r[0].id), ("ALL", 0));
    assert_eq!(
        lines(&r[0]),
        [
            "all follow me",
            "all stay",
            "all go home",
            "all patrol",
            "all defend",
            "all get wood",
            "all get stone",
            "all get ore",
        ]
    );
    assert_eq!((r[1].who.as_str(), r[1].id), ("Miller-3", MILLER));
    assert_eq!(lines(&r[1]), ["Miller-3 follow me", "Miller-3 stay"]);
    assert_eq!(
        r[1].orders.iter().map(|o| o.label).collect::<Vec<_>>(),
        ["FOLLOW", "STAY"]
    );
}

/// The shard says no crew both for an agent nobody commands (it takes any
/// teammate's orders) and for one whose commander is away (it takes only
/// theirs): the ALL row reaches the first, and neither gets a row by name.
#[test]
fn an_uncrewed_agent_gets_the_all_row_only() {
    let t = team(&[ME, MILLER, BOB]);
    let r = rows(&t, &[MILLER], |_| None);
    let who: Vec<_> = r.iter().map(|r| r.who.as_str()).collect();
    assert_eq!(who, ["ALL"]);
    // One yours beside it: the ALL row, and a row for yours.
    let t = team(&[ME, MILLER, ASH]);
    let r = rows(&t, &[MILLER, ASH], |id| (id == ASH).then_some(ME));
    let who: Vec<_> = r.iter().map(|r| r.who.as_str()).collect();
    assert_eq!(who, ["ALL", "Ash"]);
}

#[test]
fn an_agent_crewed_to_someone_else_is_hidden() {
    let t = team(&[BOB, ME, MILLER, ASH]);
    let crew_of = |id| match id {
        MILLER => Some(BOB),
        ASH => Some(ME),
        _ => None,
    };
    let r = rows(&t, &[MILLER, ASH], crew_of);
    let who: Vec<_> = r.iter().map(|r| r.who.as_str()).collect();
    assert_eq!(who, ["ALL", "Ash"]);
    // With every agent someone else's, nothing.
    assert!(rows(&t, &[MILLER], crew_of).is_empty());
}

#[test]
fn a_name_a_line_cannot_open_with_gets_no_row() {
    let t = team(&[ME, MILLER, 8, 9, 10, 11, 12, 13]);
    let named = |id| match id {
        MILLER => Some("Miller-3".into()),
        8 => Some("Big Al".into()),
        9 => Some("Al:".into()),
        // The shard would read a lock code out of these.
        10 => Some("Ash#2".into()),
        11 => Some("007".into()),
        // A job's word, or all's: the line would go to them.
        12 => Some("Boss".into()),
        13 => Some("crew".into()),
        _ => None,
    };
    let mine = |_| Some(ME);
    let r = order_rows(&t, ME, |id| id != ME, mine, named);
    let who: Vec<_> = r.iter().map(|r| r.who.as_str()).collect();
    assert_eq!(who, ["ALL", "Miller-3"]);
    // No name told yet: the ALL row still reaches it.
    let r = order_rows(&t, ME, |id| id == MILLER, mine, |_| None);
    let who: Vec<_> = r.iter().map(|r| r.who.as_str()).collect();
    assert_eq!(who, ["ALL"]);
}

/// Names are not unique, and a body drops a line opening with a teammate's
/// name: a name another member goes by (any case, you too) gets no row.
#[test]
fn a_name_two_members_share_gets_no_row() {
    let t = team(&[ME, MILLER, ASH, BOB]);
    // Two default buddies, one yours and one Bob's.
    let named = |id| match id {
        MILLER => Some("agent".into()),
        ASH => Some("Agent".into()),
        BOB => Some("Bob".into()),
        _ => None,
    };
    let crew_of = |id| match id {
        MILLER => Some(ME),
        ASH => Some(BOB),
        _ => None,
    };
    let r = order_rows(&t, ME, |id| id == MILLER || id == ASH, crew_of, named);
    let who: Vec<_> = r.iter().map(|r| r.who.as_str()).collect();
    assert_eq!(who, ["ALL"]);
    // Your own name too.
    let named = |id| match id {
        ME | MILLER => Some("Ash".into()),
        _ => None,
    };
    let r = order_rows(&t, ME, |id| id == MILLER, |_| Some(ME), named);
    let who: Vec<_> = r.iter().map(|r| r.who.as_str()).collect();
    assert_eq!(who, ["ALL"]);
}

#[test]
fn every_line_fits_a_chat_line() {
    let t = team(&[ME, MILLER, ASH]);
    // The longest name a session may declare.
    let long = |id| (id == ASH).then(|| "x".repeat(protocol::NAME_MAX_BYTES));
    let mut all = rows(&t, &[MILLER, ASH], |_| Some(ME));
    all.extend(order_rows(&t, ME, |id| id == ASH, |_| Some(ME), long));
    for row in &all {
        for o in &row.orders {
            assert!(o.line.len() <= protocol::CHAT_MAX_BYTES, "{}", o.line);
            assert!(
                protocol::ChatText::sanitize(o.line.as_bytes()).is_some(),
                "{}",
                o.line
            );
            assert!(!protocol::secret::holds_secret(&o.line), "{}", o.line);
        }
    }
}
