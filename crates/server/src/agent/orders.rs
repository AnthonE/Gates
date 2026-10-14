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
//! `/t go home` · `/t help` · `/t the code is 1234`.

use crate::agent::lock::LockCode;
use crate::mind::Goal;

/// What a teammate asked for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Order {
    /// Stay with them until told otherwise.
    Follow,
    /// Hold here until told otherwise; ends a follow.
    Stay,
    /// One supply run of that kind for the team's box.
    Supply(Goal),
    /// Walk home, the team's base when it lives there.
    Home,
    /// Say what it understands.
    Help,
}

/// What `help` answers, within one chat line.
pub const HELP: &str = "say: follow, stay, wood, stone, ore, cloth, home";

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
/// then a run, then home, then follow: "stop following" stops.
pub fn order(text: &[u8]) -> Option<Order> {
    let mut found: Option<Order> = None;
    let rank = |o: Order| match o {
        Order::Help => 0,
        Order::Stay => 1,
        Order::Supply(_) => 2,
        Order::Home => 3,
        Order::Follow => 4,
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
        } else if is(w, &["home", "base"]) {
            Order::Home
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

/// A lock code in a team line: a lone number of one to four digits, the
/// same rule `examples/claude_agent.py` keeps. Two numbers is no code.
pub fn code(text: &[u8]) -> Option<LockCode> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_words_are_orders() {
        let cases: [(&str, Option<Order>); 14] = [
            ("follow me", Some(Order::Follow)),
            ("come here", Some(Order::Follow)),
            ("Follow", Some(Order::Follow)),
            ("stay", Some(Order::Stay)),
            ("stop following me", Some(Order::Stay)),
            ("go get wood", Some(Order::Supply(Goal::SupplyWood))),
            ("get stone pls", Some(Order::Supply(Goal::SupplyStone))),
            ("need metal ore", Some(Order::Supply(Goal::SupplyOre))),
            ("cloth!", Some(Order::Supply(Goal::SupplyHemp))),
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
}
