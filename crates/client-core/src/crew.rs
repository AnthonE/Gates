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

/// Whether a line can open with `name` and be read as for that body: the
/// shard takes the first word as typed, trailing `:,!.?;` off, so a name
/// with a space in it or such an ending cannot be addressed.
pub fn addressable(name: &str) -> bool {
    !name.is_empty()
        && !name.bytes().any(|b| b.is_ascii_whitespace())
        && !name.ends_with([':', ',', '!', '.', '?', ';'])
}
