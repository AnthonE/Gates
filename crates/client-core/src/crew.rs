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
/// nothing more: a name ([`is_name`]: no space, no lock code in it), and
/// not ending in `:,!.?;`, which the shard takes off the first word.
pub fn addressable(name: &str) -> bool {
    is_name(name.as_bytes()) && !name.ends_with([':', ',', '!', '.', '?', ';'])
}
