//! The TEAM screen's order buttons (`client_core::crew`) against the crew's
//! ears (`server::agent::orders`): the vocabulary lives in two crates and
//! this pins them together. Every button's line, bare, for `all` and for a
//! body by name, is the order it is labelled, reaches only who it names,
//! and carries no lock code. A name that is an order word, or holds one,
//! is still only who the line is for.

use client_core::crew::{self, Audience, Order as Button};
use server::agent::orders::{code, for_me, order, to_me, Order};
use server::agent::society::Role;
use server::mind::{Goal, Who};

fn meant(b: Button) -> Order {
    match b.label {
        "FOLLOW" => Order::Follow,
        "STAY" => Order::Stay,
        "HOME" => Order::Home,
        "PATROL" => Order::Patrol,
        "DEFEND" => Order::Defend,
        "WOOD" => Order::Supply(Goal::SupplyWood),
        "STONE" => Order::Supply(Goal::SupplyStone),
        "ORE" => Order::Supply(Goal::SupplyOre),
        other => panic!("a button the crew has no order for: {other}"),
    }
}

fn who(name: &str) -> Who {
    Who {
        id: 7,
        name: protocol::Name::new(name).unwrap(),
        address: protocol::Address::GUEST,
    }
}

/// The last four are order words, or hold one: `Stone-2 follow me` read
/// whole would be a stone run.
const NAMES: [&str; 7] = [
    "Miller-3", "Ash", "Oak_12", "Stone-2", "Rock", "Hold-1", "Help-1",
];

/// A body's teammates: every name but its own.
fn mates_of(me: &str) -> Vec<Who> {
    NAMES.iter().filter(|n| **n != me).map(|n| who(n)).collect()
}

/// The order body `me` (in `role`) takes from a team line, if any.
fn heard_by(line: &str, me: &str, role: Option<Role>) -> Option<Order> {
    to_me(line.as_bytes(), me.as_bytes(), role, &mates_of(me)).and_then(order)
}

#[test]
fn every_button_is_the_order_it_says() {
    for b in crew::ORDERS {
        let want = Some(meant(b));
        assert_eq!(order(b.words.as_bytes()), want, "{}", b.words);
        let all = crew::to_all(b);
        assert_eq!(code(all.as_bytes()), None, "{all}");
        for me in NAMES {
            assert_eq!(heard_by(b.words, me, None), want, "{} by {me}", b.words);
            assert_eq!(heard_by(&all, me, None), want, "{all} by {me}");
        }
        for name in NAMES {
            assert!(crew::addressable(name));
            let one = crew::to_one(name, b);
            assert_eq!(heard_by(&one, name, None), want, "{one}");
            assert_eq!(code(one.as_bytes()), None, "{one}: a name is no code");
        }
    }
    for b in crew::ONE {
        assert!(
            crew::ORDERS.contains(&b),
            "{} is on the ALL row too",
            b.label
        );
    }
}

/// A name the shard would read a lock code out of (any printable ASCII is
/// a `protocol::Name`) gets no button of its own: the ALL row reaches it.
#[test]
fn a_name_with_a_code_in_it_gets_no_button() {
    for name in ["Ash#2", "007", "Bot(2)", "Unit:7", "a 1"] {
        assert!(protocol::Name::new(name).is_some(), "{name}");
        assert!(!crew::addressable(name), "{name}");
        let line = crew::to_one(name, crew::FOLLOW);
        assert!(code(line.as_bytes()).is_some(), "{line}: why it has none");
    }
}

#[test]
fn a_named_line_reaches_only_that_body() {
    let roles = [None, Some(Role::Guard), Some(Role::Farmer)];
    for b in crew::ORDERS {
        let all = crew::to_all(b);
        for me in NAMES {
            for role in roles {
                assert!(for_me(all.as_bytes(), role, &mates_of(me)), "{all}");
            }
        }
        for named in NAMES {
            let line = crew::to_one(named, b);
            for me in NAMES {
                for role in roles {
                    assert_eq!(
                        for_me(line.as_bytes(), role, &mates_of(me)),
                        me == named,
                        "{line} heard by {me}"
                    );
                }
            }
        }
    }
}

/// The FOLLOW and STAY buttons of a body whose name is an order word, or
/// holds one, are that order for it and nothing for the rest of the crew.
#[test]
fn a_name_that_is_an_order_word_is_only_who_it_is_for() {
    for named in ["Stone-2", "Rock", "Hold-1", "Help-1"] {
        for b in crew::ONE {
            let line = crew::to_one(named, b);
            for me in NAMES {
                let want = (me == named).then(|| meant(b));
                assert_eq!(heard_by(&line, me, None), want, "{line} by {me}");
            }
        }
    }
}

/// A name that is an audience word would send its line to a job or to
/// everyone: it gets no button of its own. The shard's ears read the same
/// list (`crew::AUDIENCE`).
#[test]
fn a_name_that_is_an_audience_word_gets_no_button() {
    for (word, _) in crew::AUDIENCE {
        for name in [word.to_string(), word.to_uppercase()] {
            assert!(protocol::Name::new(&name).is_some(), "{name}");
            assert!(!crew::addressable(&name), "{name}");
        }
    }
    let job = |a: Audience| match a {
        Audience::All => None,
        Audience::Guards => Some(Role::Guard),
        Audience::Farmers => Some(Role::Farmer),
        Audience::Runners => Some(Role::Runner),
        Audience::Founder => Some(Role::Founder),
    };
    for (word, a) in crew::AUDIENCE {
        // Even with a teammate who goes by it, the word is the audience.
        let line = format!("{word} stay");
        let mates = [who(word)];
        for role in Role::ALL {
            let mine = job(a).is_none_or(|j| j == role);
            assert_eq!(
                for_me(line.as_bytes(), Some(role), &mates),
                mine,
                "{line} heard by a {role:?}"
            );
            let heard = to_me(line.as_bytes(), b"Ash", Some(role), &mates).and_then(order);
            assert_eq!(heard, mine.then_some(Order::Stay), "{line}");
        }
    }
}
