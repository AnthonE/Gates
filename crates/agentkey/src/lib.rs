//! **An agent player's wallet key, and the one place in this repo a private
//! key is held** (agent players; `NETCODE.md` §2.4).
//!
//! A person proves who they are through the elo launcher, which holds their
//! key and shows them a consent prompt (`client::elo`); nothing in this
//! repo ever sees it. An agent player has no launcher and no person at the
//! prompt, and it still has to prove an address, because it pays the same
//! doors a player does: a shard with `require_auth` refuses a guest, and a
//! shard with a ticket door asks the chain about the address it proved
//! (`server::entitle`). So an agent holds its own key, and this crate is
//! where, under three rules that are properties of its API rather than
//! promises in a doc:
//!
//! 1. **Never taken from argv.** [`AgentKey::load`] takes a *path*. There is
//!    no constructor from a string a command line could carry — a key in
//!    argv is in `ps`, in shell history and in every crash report that
//!    prints its arguments.
//! 2. **Never logged.** [`AgentKey`] has no `Display`, its `Debug` prints the
//!    address and nothing else, and no [`KeyError`] carries a byte of the
//!    file it refused.
//! 3. **Never in the page.** The browser build cannot link this crate:
//!    `client` depends on it only under its `native` feature, which a wasm32
//!    build refuses (`client`'s own `compile_error!` pair).
//!
//! And one check it does make: the file's mode. A key readable by anyone but
//! its owner is refused at load with the mode in the message, the `ssh`
//! convention, because the failure it prevents is silent — a world-readable
//! key works perfectly until somebody else uses it.
//!
//! **What it signs is the shard's SIWE message and nothing else.**
//! [`AgentKey::sign_siwe`] composes the text itself, through
//! `protocol::siwe_message` — the function the shard rebuilds the same bytes
//! with — from the domain the agent dialled, the shard's nonce and its
//! `Issued At`. No caller hands this key a sentence to sign, so no caller
//! can smuggle one into a signature.

use k256::ecdsa::SigningKey;
use protocol::{siwe_message, Address, Signature, NONCE_BYTES, SIWE_MESSAGE_MAX};
use std::path::Path;
use tiny_keccak::{Hasher, Keccak};
use zeroize::Zeroize;

/// The largest key file [`AgentKey::load`] will read, in bytes. A key is 64
/// hex digits, an optional `0x` and a newline; anything past this is not a
/// key file, and reading it whole would let a wrong path (a log, a binary)
/// fill memory. Refused, not truncated.
pub const KEY_FILE_MAX_BYTES: usize = 256;

/// Why a key did not load. **No variant carries the file's contents** —
/// the path is the caller's, and the reason is a class, never a byte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyError {
    /// The file could not be opened or read (the OS's error, which names the
    /// path the caller gave and nothing inside it).
    Unreadable(String),
    /// The path names something that is not a regular file.
    NotAFile,
    /// Group or others may read or write it. `mode` is the permission bits,
    /// so the message can say what to `chmod`.
    TooOpen { mode: u32 },
    /// Longer than [`KEY_FILE_MAX_BYTES`] — not a key file.
    TooLarge,
    /// Not 64 hex digits (with an optional `0x` and surrounding whitespace).
    Malformed,
    /// 64 hex digits that are not a secp256k1 secret: zero, or past the
    /// group order.
    InvalidScalar,
    /// This platform cannot check a file's mode, so it will not load one.
    Unsupported,
}

impl std::fmt::Display for KeyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KeyError::Unreadable(why) => write!(f, "agent key: cannot read it ({why})"),
            KeyError::NotAFile => write!(f, "agent key: not a regular file"),
            KeyError::TooOpen { mode } => write!(
                f,
                "agent key: mode {mode:o} lets others read it — `chmod 600` the file \
                 (owner read/write only) and start again"
            ),
            KeyError::TooLarge => write!(
                f,
                "agent key: longer than {KEY_FILE_MAX_BYTES} bytes, so not a key file"
            ),
            KeyError::Malformed => write!(
                f,
                "agent key: not a secp256k1 secret — the file must hold 64 hex digits \
                 (an optional 0x, and nothing else)"
            ),
            KeyError::InvalidScalar => write!(
                f,
                "agent key: 64 hex digits, but zero or past the curve order — not a key"
            ),
            KeyError::Unsupported => write!(
                f,
                "agent key: this platform cannot check a file's permissions, so no key \
                 is loaded from a file here"
            ),
        }
    }
}

impl std::error::Error for KeyError {}

/// A loaded agent key. See the module header for the three rules it keeps.
pub struct AgentKey {
    sk: SigningKey,
    address: Address,
}

/// The address and nothing else. A key in a log is the one leak this crate
/// exists to make impossible, and `{:?}` is how a key reaches a log without
/// anybody deciding to print it.
impl std::fmt::Debug for AgentKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "AgentKey({:?})", self.address)
    }
}

impl AgentKey {
    /// Load a key from a file only its owner can read.
    ///
    /// The file holds the secret as 64 hex digits, optionally `0x`-prefixed,
    /// optionally surrounded by whitespace (a trailing newline is normal). On
    /// unix its permissions must grant nothing to group or others — `0600`
    /// or `0400`; anything else is refused with the mode in the message. On
    /// any other platform this refuses outright: a check that cannot be made
    /// is not made silently.
    pub fn load(path: &Path) -> Result<Self, KeyError> {
        #[cfg(unix)]
        {
            use std::io::Read;
            use std::os::unix::fs::PermissionsExt;
            let meta = std::fs::metadata(path).map_err(|e| KeyError::Unreadable(e.to_string()))?;
            if !meta.is_file() {
                return Err(KeyError::NotAFile);
            }
            let mode = meta.permissions().mode() & 0o777;
            if mode & 0o077 != 0 {
                return Err(KeyError::TooOpen { mode });
            }
            let mut file =
                std::fs::File::open(path).map_err(|e| KeyError::Unreadable(e.to_string()))?;
            let mut buf = [0u8; KEY_FILE_MAX_BYTES + 1];
            let mut n = 0;
            loop {
                match file.read(&mut buf[n..]) {
                    Ok(0) => break,
                    Ok(k) => {
                        n += k;
                        if n > KEY_FILE_MAX_BYTES {
                            buf.zeroize();
                            return Err(KeyError::TooLarge);
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(e) => {
                        buf.zeroize();
                        return Err(KeyError::Unreadable(e.to_string()));
                    }
                }
            }
            let parsed = parse_secret(&buf[..n]);
            buf.zeroize();
            let mut secret = parsed?;
            let key = Self::from_secret(&secret);
            secret.zeroize();
            key
        }
        #[cfg(not(unix))]
        {
            let _ = path;
            Err(KeyError::Unsupported)
        }
    }

    /// A key from its 32 secret bytes. For tests, which generate throwaway
    /// keys in-process; a deployment loads from a file ([`AgentKey::load`]).
    pub fn from_secret(secret: &[u8; 32]) -> Result<Self, KeyError> {
        let sk = SigningKey::from_bytes(secret.into()).map_err(|_| KeyError::InvalidScalar)?;
        let address = address_of(&sk);
        Ok(Self { sk, address })
    }

    /// The address this key proves — what a shard files the agent under,
    /// what entitlement is asked about, and what a spectator names to watch.
    pub fn address(&self) -> Address {
        self.address
    }

    /// Sign the SIWE message a shard asked for: `protocol::siwe_message` over
    /// the domain this agent DIALLED (never one the shard named — that is
    /// SIWE's whole domain binding), this key's checksummed address, the
    /// shard's nonce and its `Issued At`, under the EIP-191 envelope every
    /// wallet applies. The shard rebuilds the same bytes and recovers the
    /// signer (`server::auth::verify`).
    pub fn sign_siwe(&self, domain: &str, nonce: &[u8; NONCE_BYTES], issued_at: u64) -> Signature {
        let sum = self.address.to_checksum_hex();
        // `to_checksum_hex` writes ASCII; the fallback keeps this total.
        let addr = core::str::from_utf8(&sum).unwrap_or("0x");
        let mut text = [0u8; SIWE_MESSAGE_MAX];
        let n = siwe_message(domain, addr, protocol::SLUG, nonce, issued_at, &mut text)
            .min(SIWE_MESSAGE_MAX);
        self.sign_personal(&text[..n])
    }

    /// [`AgentKey::sign_siwe`] in the shape the desktop client's signer takes
    /// (`client::Session::connect_as`): the three inert values, the nonce as
    /// the lowercase hex the elo launcher's `prove` also takes. `None` for a
    /// nonce that is not 64 hex digits, which is a handshake bug rather than
    /// something to sign around.
    pub fn sign_proof_hex(
        &self,
        domain: &str,
        nonce_hex: &str,
        issued_at: u64,
    ) -> Option<Signature> {
        let b = nonce_hex.as_bytes();
        if b.len() != NONCE_BYTES * 2 {
            return None;
        }
        let mut nonce = [0u8; NONCE_BYTES];
        for (i, out) in nonce.iter_mut().enumerate() {
            *out = (hex_val(b[2 * i])? << 4) | hex_val(b[2 * i + 1])?;
        }
        Some(self.sign_siwe(domain, &nonce, issued_at))
    }

    /// `personal_sign`: keccak over the EIP-191 envelope, recoverable ECDSA,
    /// `v` as 27/28. Private: the only text this key signs is the one
    /// [`AgentKey::sign_siwe`] composed (module header).
    fn sign_personal(&self, message: &[u8]) -> Signature {
        let mut k = Keccak::v256();
        k.update(b"\x19Ethereum Signed Message:\n");
        k.update(message.len().to_string().as_bytes());
        k.update(message);
        let mut digest = [0u8; 32];
        k.finalize(&mut digest);
        // Deterministic (RFC 6979): no randomness, so nothing here can fail
        // for want of entropy. A failure would be a curve bug, and the empty
        // signature it yields is refused by any shard as a guest.
        let Ok((sig, rec)) = self.sk.sign_prehash_recoverable(&digest) else {
            return Signature::NONE;
        };
        let mut raw = [0u8; protocol::SIGNATURE_BYTES];
        raw[..64].copy_from_slice(&sig.to_bytes());
        raw[64] = 27 + rec.to_byte();
        Signature(raw)
    }
}

fn hex_val(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// 64 hex digits → 32 bytes. Whitespace around and an `0x` in front are
/// allowed; nothing else is. Never echoes what it refused.
fn parse_secret(text: &[u8]) -> Result<[u8; 32], KeyError> {
    let t = text.trim_ascii();
    let t = t
        .strip_prefix(b"0x")
        .or_else(|| t.strip_prefix(b"0X"))
        .unwrap_or(t);
    if t.len() != 64 {
        return Err(KeyError::Malformed);
    }
    let mut out = [0u8; 32];
    for (i, o) in out.iter_mut().enumerate() {
        match (hex_val(t[2 * i]), hex_val(t[2 * i + 1])) {
            (Some(h), Some(l)) => *o = (h << 4) | l,
            _ => {
                out.zeroize();
                return Err(KeyError::Malformed);
            }
        }
    }
    Ok(out)
}

/// The address of a key: the last 20 bytes of keccak256 over the
/// uncompressed public point without its `0x04` tag — `server::auth`'s
/// derivation, which `protocol::auth`'s golden pins against the published
/// vector for private key 1 (and this crate's tests pin again).
fn address_of(sk: &SigningKey) -> Address {
    let point = sk.verifying_key().to_encoded_point(false);
    let mut k = Keccak::v256();
    k.update(&point.as_bytes()[1..]);
    let mut h = [0u8; 32];
    k.finalize(&mut h);
    let mut a = [0u8; protocol::ADDRESS_BYTES];
    a.copy_from_slice(&h[12..]);
    Address(a)
}

#[cfg(test)]
mod tests {
    use super::*;
    use k256::ecdsa::{RecoveryId, Signature as EcdsaSig, VerifyingKey};

    /// Private key 1 — a published vector, never a real key.
    fn one() -> [u8; 32] {
        let mut s = [0u8; 32];
        s[31] = 1;
        s
    }

    fn recover(message: &[u8], sig: &Signature) -> Address {
        let mut k = Keccak::v256();
        k.update(b"\x19Ethereum Signed Message:\n");
        k.update(message.len().to_string().as_bytes());
        k.update(message);
        let mut digest = [0u8; 32];
        k.finalize(&mut digest);
        let s = EcdsaSig::from_slice(&sig.0[..64]).unwrap();
        let rec = RecoveryId::from_byte(sig.0[64] - 27).unwrap();
        let vk = VerifyingKey::recover_from_prehash(&digest, &s, rec).unwrap();
        let point = vk.to_encoded_point(false);
        let mut k = Keccak::v256();
        k.update(&point.as_bytes()[1..]);
        let mut h = [0u8; 32];
        k.finalize(&mut h);
        let mut a = [0u8; 20];
        a.copy_from_slice(&h[12..]);
        Address(a)
    }

    /// The address derivation, against the vector every Ethereum library
    /// publishes. Derived wrong, every agent would prove an address nobody
    /// holds, consistently, with every other test here green.
    #[test]
    fn private_key_one_is_the_published_address() {
        let k = AgentKey::from_secret(&one()).unwrap();
        assert_eq!(
            k.address(),
            Address::from_hex(b"0x7e5f4552091a69125d5dfcb7b8c2659029395bdf").unwrap()
        );
    }

    /// **The signature is over the exact SIWE bytes the shard rebuilds.**
    /// Recomputed here through `protocol::siwe_message` independently and
    /// recovered to this key's address.
    #[test]
    fn a_siwe_signature_recovers_to_the_key_over_the_shards_bytes() {
        let k = AgentKey::from_secret(&one()).unwrap();
        let nonce = [7u8; NONCE_BYTES];
        let sig = k.sign_siwe("game.example", &nonce, 1_770_000_000);
        let sum = k.address().to_checksum_hex();
        let mut text = [0u8; SIWE_MESSAGE_MAX];
        let n = siwe_message(
            "game.example",
            core::str::from_utf8(&sum).unwrap(),
            protocol::SLUG,
            &nonce,
            1_770_000_000,
            &mut text,
        );
        assert_eq!(recover(&text[..n], &sig), k.address());
        // And the hex-shaped entry signs the same bytes.
        let hex: String = nonce.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(
            k.sign_proof_hex("game.example", &hex, 1_770_000_000),
            Some(sig)
        );
        assert_eq!(k.sign_proof_hex("game.example", "zz", 1), None);
    }

    /// A different domain is a different message — the signature a relay
    /// collects for one shard does not verify at another.
    #[test]
    fn the_domain_reaches_the_signature() {
        let k = AgentKey::from_secret(&one()).unwrap();
        let n = [1u8; NONCE_BYTES];
        assert_ne!(
            k.sign_siwe("a.example", &n, 5),
            k.sign_siwe("b.example", &n, 5)
        );
    }

    /// The key never reaches `{:?}`.
    #[test]
    fn debug_prints_the_address_and_never_the_secret() {
        let k = AgentKey::from_secret(&one()).unwrap();
        let shown = format!("{k:?}");
        assert!(
            shown.contains("0x7e5f4552091a69125d5dfcb7b8c2659029395bdf"),
            "{shown}"
        );
        let secret_hex: String = one().iter().map(|b| format!("{b:02x}")).collect();
        assert!(!shown.contains(&secret_hex[48..]), "{shown}");
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("agentkey-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    /// **The mode check, both ways.** Owner-only loads; anything group or
    /// other may read is refused with the mode, and the refusal carries none
    /// of the file.
    #[cfg(unix)]
    #[test]
    fn a_key_file_others_can_read_is_refused_and_an_owner_only_one_loads() {
        use std::os::unix::fs::PermissionsExt;
        let secret = "0x0000000000000000000000000000000000000000000000000000000000000001\n";
        let p = scratch("mode.key");
        std::fs::write(&p, secret).unwrap();
        for (mode, ok) in [(0o600, true), (0o400, true), (0o644, false), (0o640, false)] {
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(mode)).unwrap();
            match AgentKey::load(&p) {
                Ok(k) => {
                    assert!(ok, "mode {mode:o} must be refused");
                    assert_eq!(
                        k.address(),
                        AgentKey::from_secret(&one()).unwrap().address()
                    );
                }
                Err(e) => {
                    assert!(!ok, "mode {mode:o} must load: {e}");
                    assert_eq!(e, KeyError::TooOpen { mode });
                    assert!(!e.to_string().contains("0001"), "{e}");
                }
            }
        }
        let _ = std::fs::remove_file(&p);
    }

    /// Everything that is not a key is refused as a class — and a refusal
    /// never echoes the bytes it read.
    #[cfg(unix)]
    #[test]
    fn a_file_that_is_not_a_key_is_refused_without_echoing_it() {
        use std::os::unix::fs::PermissionsExt;
        let p = scratch("bad.key");
        let cases: [(&[u8], KeyError); 4] = [
            (b"deadbeef\n", KeyError::Malformed),
            (
                b"0000000000000000000000000000000000000000000000000000000000000000",
                KeyError::InvalidScalar,
            ),
            (
                b"g000000000000000000000000000000000000000000000000000000000000001",
                KeyError::Malformed,
            ),
            (&[b'a'; KEY_FILE_MAX_BYTES + 1], KeyError::TooLarge),
        ];
        for (bytes, want) in cases {
            std::fs::write(&p, bytes).unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).unwrap();
            let e = AgentKey::load(&p).expect_err("not a key");
            assert_eq!(e, want);
            assert!(!e.to_string().contains("deadbeef"), "{e}");
        }
        assert_eq!(
            AgentKey::load(&std::env::temp_dir()).expect_err("a directory"),
            KeyError::NotAFile
        );
        let _ = std::fs::remove_file(&p);
    }
}
