//! The code on its own locks, for when the base has them (lane C).
//!
//! A player picks a code and keeps it to themselves. An agent derives its
//! code from something only it holds, so it is the same every session
//! without being written down anywhere: the agent key (a wallet agent
//! passes a signature only its key can make), or for a guest a secret made
//! once for its install ([`guest_secret`]), together with the agent's
//! name, so two agents on one island do not share a code.
//!
//! The code never leaves this type in readable form: no `Display`, a
//! `Debug` that prints stars, and nothing in the summary, the logs or
//! stdout. The one way out is [`LockCode::wire`], for the access verb.

use sim_core::lock::CODE_MAX;
use std::io::{self, Read, Write};
use std::path::Path;
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

    /// A guest agent's code: it holds no key, so its install's secret
    /// ([`guest_secret`]) stands in for one, with the welcome seed keeping
    /// one island's codes apart from another's. The seed alone would not
    /// do: every client is sent it, and agents say their names.
    pub fn for_guest(install: &[u8; GUEST_SECRET_BYTES], seed: u64, name: &str) -> Self {
        let mut secret = [0u8; GUEST_SECRET_BYTES + 8];
        secret[..GUEST_SECRET_BYTES].copy_from_slice(install);
        secret[GUEST_SECRET_BYTES..].copy_from_slice(&seed.to_le_bytes());
        Self::derive(&secret, name)
    }

    /// The code as the access verb carries it (`0..=CODE_MAX`).
    pub fn wire(self) -> u16 {
        self.0
    }
}

/// Bytes of an install's guest secret.
pub const GUEST_SECRET_BYTES: usize = 32;

/// The install's guest secret kept at `path`: read when it is there,
/// otherwise made from the OS's randomness and written there once,
/// readable by its owner alone, so the codes stay the same every session.
pub fn guest_secret(path: &Path) -> io::Result<[u8; GUEST_SECRET_BYTES]> {
    let mut secret = [0u8; GUEST_SECRET_BYTES];
    match std::fs::File::open(path) {
        Ok(mut f) => {
            f.read_exact(&mut secret)?;
            return Ok(secret);
        }
        Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
        Err(_) => {}
    }
    getrandom::getrandom(&mut secret).map_err(|e| io::Error::other(e.to_string()))?;
    let mut open = std::fs::OpenOptions::new();
    open.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut open, 0o600);
    match open.open(path) {
        Ok(mut f) => {
            f.write_all(&secret)?;
            f.sync_all()?;
            Ok(secret)
        }
        // Another agent of this install made it first: theirs stands.
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => guest_secret(path),
        Err(e) => Err(e),
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
        let (one, two) = ([1u8; GUEST_SECRET_BYTES], [2u8; GUEST_SECRET_BYTES]);
        let guest = LockCode::for_guest(&one, 7, "jev");
        assert_eq!(guest, LockCode::for_guest(&one, 7, "jev"));
        assert_ne!(guest, LockCode::for_guest(&one, 8, "jev"), "another island");
        assert_ne!(
            guest,
            LockCode::for_guest(&two, 7, "jev"),
            "the seed and the name alone do not make it"
        );
        // Spread over the whole range, not bunched at one end.
        let mut seen = [false; 10];
        for i in 0..200u32 {
            let c = LockCode::derive(&i.to_le_bytes(), "jev").wire();
            seen[usize::from(c / 1000)] = true;
        }
        assert!(seen.iter().all(|&s| s));
        assert_eq!(format!("{a:?}"), "LockCode(****)");
    }

    #[test]
    fn a_guest_secret_is_made_once_kept_private_and_read_back() {
        let dir = std::env::temp_dir().join(format!("gates-lock-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("guest.secret");
        let _ = std::fs::remove_file(&path);
        let made = guest_secret(&path).unwrap();
        assert_ne!(made, [0; GUEST_SECRET_BYTES]);
        assert_eq!(guest_secret(&path).unwrap(), made, "the same every session");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o077, 0, "readable by its owner alone");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
