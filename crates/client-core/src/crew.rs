//! The crew orders a human types on team chat for agent teammates
//! (`server::agent::orders`), as the TEAM screen's buttons send them: a
//! label and the words. Here, not in the client, so the server crate (which
//! depends on this one) pins every line to the order it means
//! (`crates/server/tests/crew_orders.rs`).

/// One order: the button's label and the team line's words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Order {
    pub label: &'static str,
    pub words: &'static str,
}

pub const FOLLOW: Order = Order {
    label: "FOLLOW",
    words: "follow me",
};
pub const STAY: Order = Order {
    label: "STAY",
    words: "stay",
};
pub const HOME: Order = Order {
    label: "HOME",
    words: "go home",
};
pub const PATROL: Order = Order {
    label: "PATROL",
    words: "patrol",
};
pub const DEFEND: Order = Order {
    label: "DEFEND",
    words: "defend",
};
pub const WOOD: Order = Order {
    label: "WOOD",
    words: "get wood",
};
pub const STONE: Order = Order {
    label: "STONE",
    words: "get stone",
};
pub const ORE: Order = Order {
    label: "ORE",
    words: "get ore",
};

/// Every order, as the ALL row lists them.
pub const ORDERS: [Order; 8] = [FOLLOW, STAY, HOME, PATROL, DEFEND, WOOD, STONE, ORE];

/// The orders one named body gets a button for.
pub const ONE: [Order; 2] = [FOLLOW, STAY];

/// The word that opens a line for every body (`for_me`'s `all`).
pub const ALL: &str = "all";

/// Who a line is for when its first word is not a body's name: every
/// body, or one job's (`server::agent::society::Role`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Audience {
    All,
    Guards,
    Farmers,
    Runners,
    Founder,
}

/// The first words that make a line an audience's (`for_me`), any case.
/// One home for the shard's ears and the buttons: a body that goes by one
/// of these is never [`addressable`].
pub const AUDIENCE: [(&str, Audience); 13] = [
    (ALL, Audience::All),
    ("everyone", Audience::All),
    ("everybody", Audience::All),
    ("crew", Audience::All),
    ("y'all", Audience::All),
    ("guard", Audience::Guards),
    ("guards", Audience::Guards),
    ("farmer", Audience::Farmers),
    ("farmers", Audience::Farmers),
    ("runner", Audience::Runners),
    ("runners", Audience::Runners),
    ("founder", Audience::Founder),
    ("boss", Audience::Founder),
];

/// The audience a first word names, if it is one of [`AUDIENCE`].
pub fn audience(word: &[u8]) -> Option<Audience> {
    AUDIENCE
        .iter()
        .find(|(w, _)| word.eq_ignore_ascii_case(w.as_bytes()))
        .map(|&(_, a)| a)
}

/// The line for everyone: `all <words>`.
pub fn to_all(o: Order) -> String {
    format!("{ALL} {}", o.words)
}

/// The line for one body by name: `<name> <words>`.
pub fn to_one(name: &str, o: Order) -> String {
    format!("{name} {}", o.words)
}

/// Whether a line's first word is a name to the shard's ears: a letter in
/// it and nothing but letters, digits and `-_.'`. The shard skips such a
/// word when it looks for a lock code (`server::agent::orders::code`), so
/// the digit in `Miller-3` is no code; the one in `Ash#2` or `007` would be.
pub fn is_name(word: &[u8]) -> bool {
    word.iter().any(u8::is_ascii_alphabetic)
        && word
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'\''))
}

/// Whether a line can open with `name` and be read as for that body and
/// nothing more: a name ([`is_name`]: no space, no lock code in it), not
/// ending in `:,!.?;`, which the shard takes off the first word, and no
/// [`audience`] word (`Boss` or `Guard` would send it to a job).
pub fn addressable(name: &str) -> bool {
    is_name(name.as_bytes())
        && !name.ends_with([':', ',', '!', '.', '?', ';'])
        && audience(name.as_bytes()).is_none()
}
