//! Orders a teammate types on team chat, for the scripted policy.
//!
//! Jev and an external agent read what they hear themselves; `--scripted`
//! makes no model calls, so this is how it hears its team: a handful of
//! plain words, matched whole. Only a **team-channel** line from a person
//! on its team is read (`mind::Scripted::hear`): the shard sends a team
//! line only to the team, so a stranger cannot give an order, and an
//! agent's own replies are never taken as one.
//!
//! `/t follow me` · `/t stay` · `/t go get wood` (stone, ore, cloth) ·
//! `/t loot` (the crates, the vaults) · `/t feed the works` (deposit for
//! the quota) · `/t go home` · `/t patrol` · `/t defend` · `/t help` ·
//! `/t the code is 1234`.
//!
//! A line may open with who it is for ([`for_me`]): a job (`/t guards
//! patrol`, `/t farmers get wood`), a teammate's name (`/t Miller-3
//! follow`), or `all`. A line that opens with none of them is for everyone.
//! That first word is no order ([`to_me`]): `Stone-2 follow me` is a
//! follow for Stone-2, not a stone run.

use crate::agent::lock::LockCode;
use crate::agent::society::Role;
use crate::mind::{Goal, Who};
use client_core::crew::{self, Audience};

/// What a teammate asked for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Order {
    /// Stay with them until told otherwise.
    Follow,
    /// Hold here until told otherwise; ends a follow.
    Stay,
    /// One supply run of that kind for the team's box.
    Supply(Goal),
    /// One go at that goal: a loot run (`Goal::Loot`) or a feed of a
    /// work's quota (`Goal::FeedWork`).
    Run(Goal),
    /// Walk home, the team's base when it lives there.
    Home,
    /// Walk the ring round home until told otherwise.
    Patrol,
    /// Home is struck: get in and fight from there, once.
    Defend,
    /// Say what it understands.
    Help,
}

/// What `help` answers, within one chat line.
pub const HELP: &str =
    "say: follow, stay, wood, stone, ore, cloth, loot, feed, home, patrol, defend; start with guards, a name or all";

/// The words of a line, lowercased, split on anything not a letter or a
/// digit. No allocation: the caller walks them.
fn words(text: &[u8]) -> impl Iterator<Item = &[u8]> {
    text.split(|b| !b.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
}

fn is(word: &[u8], any: &[&str]) -> bool {
    any.iter().any(|a| word.eq_ignore_ascii_case(a.as_bytes()))
}

/// The order in a team line, if it gives one. `help` wins, then a stop,
/// then a run, then home, then follow: "stop following" stops. A supply
/// word names what to fetch and outranks a feed, which names where it
/// goes: `deposit wood` is a wood run for the box, `feed the works` a feed.
pub fn order(text: &[u8]) -> Option<Order> {
    let mut found: Option<Order> = None;
    let rank = |o: Order| match o {
        Order::Help => 0,
        Order::Stay => 1,
        Order::Run(Goal::FeedWork) => 3,
        Order::Supply(_) | Order::Run(_) => 2,
        Order::Defend => 4,
        Order::Home => 5,
        Order::Patrol => 6,
        Order::Follow => 7,
    };
    for w in words(text) {
        let o = if is(w, &["help", "commands", "orders"]) {
            Order::Help
        } else if is(w, &["stay", "stop", "wait", "hold", "halt"]) {
            Order::Stay
        } else if is(w, &["wood", "logs", "lumber"]) {
            Order::Supply(Goal::SupplyWood)
        } else if is(w, &["stone", "stones", "rock", "rocks"]) {
            Order::Supply(Goal::SupplyStone)
        } else if is(w, &["ore", "metal", "sulfur", "sulphur"]) {
            Order::Supply(Goal::SupplyOre)
        } else if is(w, &["cloth", "hemp"]) {
            Order::Supply(Goal::SupplyHemp)
        } else if is(w, &["loot", "crates", "vault", "vaults"]) {
            // Never "gate": "meet at the gate" is no order.
            Order::Run(Goal::Loot)
        } else if is(w, &["feed", "deposit", "quota"]) {
            Order::Run(Goal::FeedWork)
        } else if is(w, &["defend", "defence", "defense", "protect"]) {
            Order::Defend
        } else if is(w, &["home", "base"]) {
            Order::Home
        } else if is(w, &["patrol", "sweep", "perimeter"]) {
            Order::Patrol
        } else if is(w, &["follow", "following", "come", "escort"]) {
            Order::Follow
        } else {
            continue;
        };
        if found.is_none_or(|f| rank(o) < rank(f)) {
            found = Some(o);
        }
    }
    found
}

/// The first word of a line as typed, trailing punctuation off (`Miller-3:`
/// is `Miller-3`).
fn head(text: &[u8]) -> &[u8] {
    let start = text
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .unwrap_or(text.len());
    let rest = &text[start..];
    let end = rest
        .iter()
        .position(u8::is_ascii_whitespace)
        .unwrap_or(rest.len());
    let mut w = &rest[..end];
    while let [body @ .., last] = w {
        if matches!(last, b':' | b',' | b'!' | b'.' | b'?' | b';') {
            w = body;
        } else {
            break;
        }
    }
    w
}

/// The line past its first word, as [`head`] splits it.
fn past_head(text: &[u8]) -> &[u8] {
    let start = text
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .unwrap_or(text.len());
    let rest = &text[start..];
    let end = rest
        .iter()
        .position(u8::is_ascii_whitespace)
        .unwrap_or(rest.len());
    &rest[end..]
}

/// The job a line's audience word names (`client_core::crew::AUDIENCE`).
fn job(a: Audience) -> Option<Role> {
    match a {
        Audience::All => None,
        Audience::Guards => Some(Role::Guard),
        Audience::Farmers => Some(Role::Farmer),
        Audience::Runners => Some(Role::Runner),
        Audience::Founder => Some(Role::Founder),
    }
}

/// Whether a team line is for this body: its job (`role`, in a society),
/// none of its teammates `mates` by name, or everyone. A line opening with
/// a job is for that job's bodies only; with a teammate's name, for them
/// only (its own name is never in `mates`, so a line naming it is for it).
/// The audience words are `client_core::crew::AUDIENCE`, which the TEAM
/// screen's buttons keep too.
pub fn for_me(text: &[u8], role: Option<Role>, mates: &[Who]) -> bool {
    let w = head(text);
    if w.is_empty() {
        return true;
    }
    if let Some(a) = crew::audience(w) {
        return job(a).is_none_or(|j| role == Some(j));
    }
    !mates
        .iter()
        .any(|m| !m.name.as_str().is_empty() && w.eq_ignore_ascii_case(m.name.as_str().as_bytes()))
}

/// What a team line says to this body, for [`order`]: `None` when it is
/// for someone else ([`for_me`]), else the line past the first word when
/// that word said who it is for (an audience, or `me`, its own name as
/// its teammates see it), else the whole line. `Rock follow me`, heard by
/// Rock, is a follow, not a stone run; `rocks please` is still a stone run.
pub fn to_me<'a>(text: &'a [u8], me: &[u8], role: Option<Role>, mates: &[Who]) -> Option<&'a [u8]> {
    if !for_me(text, role, mates) {
        return None;
    }
    let w = head(text);
    let routed = crew::audience(w).is_some()
        || (!me.is_empty() && !w.is_empty() && w.eq_ignore_ascii_case(me));
    Some(if routed { past_head(text) } else { text })
}

/// A lock code in a team line: a lone number of one to four digits, the
/// same rule `examples/claude_agent.py` keeps. Two numbers is no code.
/// The digit in a name opening the line (`Miller-3 follow me`, the TEAM
/// screen's order buttons) is no code either: a first word that is a name
/// (`client_core::crew::is_name`, the rule the buttons keep) is skipped
/// (`code: 1234` and `#1234` still give one).
pub fn code(text: &[u8]) -> Option<LockCode> {
    let text = if crew::is_name(head(text)) {
        past_head(text)
    } else {
        text
    };
    let mut found = None;
    for w in words(text) {
        if !w.iter().all(u8::is_ascii_digit) {
            continue;
        }
        if found.is_some() || w.len() > 4 {
            return None;
        }
        let n = w.iter().fold(0u16, |n, d| n * 10 + u16::from(d - b'0'));
        found = Some(LockCode::given(n)?);
    }
    found
}

/// A person nearby asking to team up, in local chat.
pub fn asks_to_team(text: &[u8]) -> bool {
    words(text).any(|w| is(w, &["team", "join", "invite", "squad", "group", "duo"]))
}

/// A teammate asking to be healed (`heal me`, `need a bandage`, `I'm
/// hurt`): no order, but one of the signs that offers `heal:m<k>` for them
/// (`explorer`'s `sense_hurt_mates`), since the shard sends nobody's hp
/// but one's own.
pub fn asks_heal(text: &[u8]) -> bool {
    words(text).any(|w| {
        is(
            w,
            &[
                "heal", "heals", "healing", "bandage", "bandages", "syringe", "syringes", "meds",
                "medic", "hurt", "bleeding",
            ],
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_teammate_asks_to_be_healed_in_plain_words() {
        for yes in [
            "heal me",
            "need a bandage!",
            "I'm hurt",
            "medic",
            "syringe pls",
        ] {
            assert!(asks_heal(yes.as_bytes()), "{yes}");
        }
        for no in ["help", "health is fine", "go get wood", "healthy"] {
            assert!(!asks_heal(no.as_bytes()), "{no}");
        }
        assert_eq!(order(b"heal me"), None, "no order");
    }

    #[test]
    fn plain_words_are_orders() {
        let cases: [(&str, Option<Order>); 28] = [
            ("guards patrol", Some(Order::Patrol)),
            ("defend the base!", Some(Order::Defend)),
            ("patrol, then come home", Some(Order::Home)),
            ("stop patrolling", Some(Order::Stay)),
            ("follow me", Some(Order::Follow)),
            ("come here", Some(Order::Follow)),
            ("Follow", Some(Order::Follow)),
            ("stay", Some(Order::Stay)),
            ("stop following me", Some(Order::Stay)),
            ("go get wood", Some(Order::Supply(Goal::SupplyWood))),
            ("get stone pls", Some(Order::Supply(Goal::SupplyStone))),
            ("need metal ore", Some(Order::Supply(Goal::SupplyOre))),
            ("cloth!", Some(Order::Supply(Goal::SupplyHemp))),
            ("all loot", Some(Order::Run(Goal::Loot))),
            ("go open the VAULTS", Some(Order::Run(Goal::Loot))),
            ("feed the works", Some(Order::Run(Goal::FeedWork))),
            (
                "deposit for the quota then go home",
                Some(Order::Run(Goal::FeedWork)),
            ),
            ("feed the gate", Some(Order::Run(Goal::FeedWork))),
            ("deposit at the works", Some(Order::Run(Goal::FeedWork))),
            // What to fetch outranks where it goes.
            ("deposit wood", Some(Order::Supply(Goal::SupplyWood))),
            (
                "feed the box some stone",
                Some(Order::Supply(Goal::SupplyStone)),
            ),
            ("quota: ore", Some(Order::Supply(Goal::SupplyOre))),
            ("meet me at the gate", None),
            ("go home", Some(Order::Home)),
            ("help", Some(Order::Help)),
            ("follow me, help?", Some(Order::Help)),
            ("nice", None),
            ("wooden door", None),
        ];
        for (text, want) in cases {
            assert_eq!(order(text.as_bytes()), want, "{text}");
        }
    }

    #[test]
    fn a_line_opening_with_a_job_or_a_name_is_for_them_only() {
        let mate = |name: &str| Who {
            id: 7,
            name: protocol::Name::new(name).unwrap(),
            address: protocol::Address::GUEST,
        };
        let mates = [mate("Miller-3"), mate("Ash")];
        let guard = Some(Role::Guard);
        assert!(for_me(b"follow me", guard, &mates), "nobody named: all");
        assert!(for_me(b"all: get wood", None, &mates));
        assert!(for_me(b"guards patrol", guard, &mates));
        assert!(!for_me(b"guards patrol", Some(Role::Farmer), &mates));
        assert!(!for_me(b"Guards, patrol", None, &mates), "no job, no guard");
        assert!(!for_me(b"Miller-3 follow", guard, &mates), "a teammate's");
        assert!(!for_me(b"ash: stay", guard, &mates));
        assert!(for_me(b"Miller-4 follow", guard, &mates), "its own name");
        assert!(for_me(b"  ", guard, &mates));
    }

    /// The word that said who a line is for is no order: a name that is
    /// an order word, or holds one, and `all` before a run.
    #[test]
    fn the_addressee_is_no_order() {
        let mate = |name: &str| Who {
            id: 7,
            name: protocol::Name::new(name).unwrap(),
            address: protocol::Address::GUEST,
        };
        let mates = [mate("Ash")];
        let heard = |text: &str, me: &str| {
            to_me(text.as_bytes(), me.as_bytes(), None, &mates).and_then(order)
        };
        assert_eq!(heard("Stone-2 follow me", "Stone-2"), Some(Order::Follow));
        assert_eq!(heard("rock: stay", "Rock"), Some(Order::Stay));
        assert_eq!(heard("Help-1 follow me", "Help-1"), Some(Order::Follow));
        assert_eq!(heard("Hold-1 follow me", "Hold-1"), Some(Order::Follow));
        assert_eq!(
            heard("rocks please", "Rock"),
            Some(Order::Supply(Goal::SupplyStone))
        );
        assert_eq!(
            heard("stone please", "Rock"),
            Some(Order::Supply(Goal::SupplyStone))
        );
        assert_eq!(heard("follow me", "Rock"), Some(Order::Follow));
        assert_eq!(
            heard("all get wood", ""),
            Some(Order::Supply(Goal::SupplyWood))
        );
        assert_eq!(heard("crew, stay", ""), Some(Order::Stay));
        assert_eq!(heard("Ash stay", "Rock"), None, "a teammate's");
        // Its own name not told yet: the whole line, as before.
        assert_eq!(
            heard("Rock follow me", ""),
            Some(Order::Supply(Goal::SupplyStone))
        );
        assert_eq!(heard("", "Rock"), None);
    }

    #[test]
    fn a_lone_short_number_is_a_code() {
        assert_eq!(code(b"the code is 1234").map(LockCode::wire), Some(1234));
        assert_eq!(code(b"0420").map(LockCode::wire), Some(420));
        assert_eq!(code(b"7").map(LockCode::wire), Some(7));
        assert_eq!(code(b"12345"), None, "too long for the keypad");
        assert_eq!(code(b"1234 or 5678"), None, "two numbers is no code");
        assert_eq!(code(b"get wood"), None);
        assert_eq!(code(b"wood2"), None, "a digit inside a word");
    }

    #[test]
    fn asking_to_team_is_a_word_not_a_substring() {
        assert!(asks_to_team(b"wanna team?"));
        assert!(asks_to_team(b"can I JOIN you"));
        assert!(!asks_to_team(b"teammate"));
        assert!(!asks_to_team(b"hello"));
    }

    #[test]
    fn help_fits_one_line() {
        assert!(HELP.len() <= protocol::ChatText::CAP);
        assert!(protocol::ChatText::sanitize(HELP.as_bytes()).is_some());
    }

    /// A name opening the line is not a code, whatever digit it carries;
    /// a number after it still is.
    #[test]
    fn a_name_with_a_digit_is_no_code() {
        assert_eq!(code(b"Miller-3 follow me"), None);
        assert_eq!(code(b"Miller-3: stay"), None);
        assert_eq!(code(b"  bob_42"), None);
        assert_eq!(
            code(b"Miller-3 the code is 1234").map(LockCode::wire),
            Some(1234)
        );
        assert_eq!(code(b"1234 is it").map(LockCode::wire), Some(1234));
        assert_eq!(code(b"code: 1234").map(LockCode::wire), Some(1234));
        assert_eq!(code(b"code:1234").map(LockCode::wire), Some(1234));
        assert_eq!(code(b"#1234").map(LockCode::wire), Some(1234));
    }
}
