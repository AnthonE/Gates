//! The code on its own locks, for when the base has them (lane C).
//!
//! A player picks a code and keeps it to themselves. An agent derives its
//! code from something only it holds, so it is the same every session
//! without being written down anywhere: the agent key (a wallet agent
//! passes a signature only its key can make), or for a guest the welcome
//! seed, together with the agent's name, so two agents on one island do
//! not share a code.
//!
//! The code never leaves this type in readable form: no `Display`, a
//! `Debug` that prints stars, and nothing in the summary, the logs or
//! stdout. The one way out is [`LockCode::wire`], for the access verb.

use sim_core::lock::CODE_MAX;
use tiny_keccak::{Hasher, Keccak};

/// Keeps the codes this module makes apart from any other use of the same
/// secret.
const DOMAIN: &[u8] = b"gates/agent-lock-code/v1";

/// A lock code, kept from being printed.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct LockCode(u16);

impl std::fmt::Debug for LockCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LockCode(****)")
    }
}

impl LockCode {
    /// The code for the agent called `name` holding `secret`.
    pub fn derive(secret: &[u8], name: &str) -> Self {
        let mut k = Keccak::v256();
        // Lengths first, so no two (secret, name) pairs hash the same
        // bytes.
        k.update(DOMAIN);
        k.update(&(secret.len() as u64).to_le_bytes());
        k.update(secret);
        k.update(&(name.len() as u64).to_le_bytes());
        k.update(name.as_bytes());
        let mut out = [0u8; 32];
        k.finalize(&mut out);
        let mut wide = [0u8; 8];
        wide.copy_from_slice(&out[..8]);
        let range = u64::from(CODE_MAX) + 1;
        Self((u64::from_le_bytes(wide) % range) as u16)
    }

    /// A guest agent's code: it holds no key, so the welcome seed stands
    /// in for one. Anyone who knows the seed and the name can work it out,
    /// so a wallet agent derives from its key instead.
    pub fn for_guest(seed: u64, name: &str) -> Self {
        Self::derive(&seed.to_le_bytes(), name)
    }

    /// The code as the access verb carries it (`0..=CODE_MAX`).
    pub fn wire(self) -> u16 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lock_code_is_stable_private_to_its_holder_and_never_printed() {
        let a = LockCode::derive(b"key one", "jev");
        assert_eq!(
            a,
            LockCode::derive(b"key one", "jev"),
            "the same every session"
        );
        assert!(a.wire() <= CODE_MAX);
        assert_ne!(a, LockCode::derive(b"key two", "jev"), "another key");
        assert_ne!(a, LockCode::derive(b"key one", "jev-1"), "another agent");
        // The boundary between secret and name is part of what is hashed.
        assert_ne!(LockCode::derive(b"ab", "c"), LockCode::derive(b"a", "bc"));
        assert_eq!(LockCode::for_guest(7, "jev"), LockCode::for_guest(7, "jev"));
        assert_ne!(LockCode::for_guest(7, "jev"), LockCode::for_guest(8, "jev"));
        // Spread over the whole range, not bunched at one end.
        let mut seen = [false; 10];
        for i in 0..200u32 {
            let c = LockCode::derive(&i.to_le_bytes(), "jev").wire();
            seen[usize::from(c / 1000)] = true;
        }
        assert!(seen.iter().all(|&s| s));
        assert_eq!(format!("{a:?}"), "LockCode(****)");
    }
}
