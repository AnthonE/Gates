//! The TEAM screen's order buttons (`client_core::crew`) against the crew's
//! ears (`server::agent::orders`): the vocabulary lives in two crates and
//! this pins them together. Every button's line, bare, for `all` and for a
//! body by name, is the order it is labelled, reaches only who it names,
//! and carries no lock code.

use client_core::crew::{self, Order as Button};
use server::agent::orders::{code, for_me, order, Order};
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

const NAMES: [&str; 3] = ["Miller-3", "Ash", "Oak_12"];

/// A body's teammates: every name but its own.
fn mates_of(me: &str) -> Vec<Who> {
    NAMES.iter().filter(|n| **n != me).map(|n| who(n)).collect()
}

#[test]
fn every_button_is_the_order_it_says() {
    for b in crew::ORDERS {
        let want = Some(meant(b));
        assert_eq!(order(b.words.as_bytes()), want, "{}", b.words);
        let all = crew::to_all(b);
        assert_eq!(order(all.as_bytes()), want, "{all}");
        assert_eq!(code(all.as_bytes()), None, "{all}");
        for name in NAMES {
            assert!(crew::addressable(name));
            let one = crew::to_one(name, b);
            assert_eq!(order(one.as_bytes()), want, "{one}");
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
