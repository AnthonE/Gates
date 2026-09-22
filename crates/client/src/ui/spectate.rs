//! What a spectator's screen says about whose view it is (spectators v0,
//! wire v73; `NETCODE.md` §2.3). Pure, so the words are testable in the code
//! tier; `render/spectate.rs` is the Bevy half that draws them.
//!
//! **The name is a label and the address is the identity.** A display name
//! is self-declared (`protocol::Name`) — two agents can call themselves
//! "jev" — so wherever the shard proved an address it rides beside the name,
//! shortened, and a viewer can tell the two apart. A guest agent has no
//! address to show, and says so rather than inventing one.

use protocol::{Address, Watch};

/// `0x7e5f…5bdf` — enough of an address to tell two apart at a glance.
pub fn short(a: &Address) -> String {
    let hex = a.to_hex();
    let s = core::str::from_utf8(&hex).unwrap_or("0x?");
    format!("{}…{}", &s[..6], &s[s.len() - 4..])
}

/// The line a watcher reads: `SPECTATING <who>`, with the proven address
/// beside a declared name, and a note when the feed is a person's delayed
/// one rather than an agent's live one.
pub fn label(w: &Watch) -> String {
    let named = !w.name.is_empty();
    let proven = !w.address.is_guest();
    let who = match (named, proven) {
        (true, true) => format!("{} · {}", w.name.as_str(), short(&w.address)),
        (true, false) => format!("{} (guest)", w.name.as_str()),
        (false, true) => short(&w.address),
        (false, false) => "a guest agent".to_string(),
    };
    if w.agent {
        format!("SPECTATING {who}")
    } else {
        format!("SPECTATING {who} · delayed feed")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::Name;

    fn addr() -> Address {
        Address::from_hex(b"0x7e5f4552091a69125d5dfcb7b8c2659029395bdf").unwrap()
    }

    #[test]
    fn a_named_proven_agent_shows_both_and_the_address_is_what_tells_them_apart() {
        let w = Watch {
            address: addr(),
            agent: true,
            name: Name::new("jev").unwrap(),
        };
        assert_eq!(label(&w), "SPECTATING jev · 0x7e5f…5bdf");
    }

    #[test]
    fn every_shape_says_who_without_inventing_anything() {
        let guest_named = Watch {
            address: Address::GUEST,
            agent: true,
            name: Name::new("jev").unwrap(),
        };
        assert_eq!(label(&guest_named), "SPECTATING jev (guest)");
        let unnamed = Watch {
            address: addr(),
            agent: true,
            name: Name::EMPTY,
        };
        assert_eq!(label(&unnamed), "SPECTATING 0x7e5f…5bdf");
        let nobody = Watch::default();
        assert!(label(&nobody).starts_with("SPECTATING a guest agent"));
        let person = Watch {
            agent: false,
            ..unnamed
        };
        assert!(
            label(&person).ends_with("delayed feed"),
            "{}",
            label(&person)
        );
    }
}
