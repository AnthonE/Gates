//! **Who is playing, proved rather than claimed** — SIWE (EIP-4361) over the
//! handshake stream.
//!
//! ## What this replaced, and why it had to go
//!
//! The first version of this file carried an opaque *session token* on the
//! Steam model: the launcher issues a handle, the game relays it, the shard
//! asks the issuer. Two things were wrong with it here, and only one was the
//! security half.
//!
//! **The security half:** the shard never asked anybody, so a token was a
//! bare name and anyone who knew yours was you.
//!
//! **The half that mattered more:** a session token *rotates*. The shard
//! filed your save under the token's own bytes, so a new session meant a new
//! key meant a new character — persistence would have silently done nothing
//! for every real player, with every gate green. We built persistence on an
//! identity that did not exist and would not have been stable if it had.
//! That is the bug this module exists to make impossible.
//!
//! ## The shape now, and it is the ordinary one
//!
//! 1. Client opens the handshake stream and sends [`Hello`](crate::Hello).
//! 2. Server sends a [`Challenge`] — a random 32-byte nonce it chose.
//! 3. Client asks its launcher to sign the SIWE message built by
//!    [`siwe_message`], and sends back [`Auth`]: address + 65-byte signature.
//! 4. Server rebuilds the *same* message, recovers the signer from the
//!    signature, and admits the connection only if it equals the claimed
//!    address.
//!
//! **The identity is the address**, and that is the whole point: it is
//! stable forever (it is a public key hash, not a session), it is public
//! (safe to write to a save file — `store.rs` demands a *name*, never a
//! credential), and it verifies with no network call at all. The "what does
//! a shard do when it cannot reach the issuer" question that the token model
//! could not answer simply does not arise: `ecrecover` is arithmetic.
//!
//! ## One builder, both sides — the drift that would otherwise be certain
//!
//! [`siwe_message`] lives *here*, in the crate both the client and the server
//! depend on, and both call it. A signature is over exact bytes, so two
//! implementations of "the message" is two chances to disagree about a space
//! — and the symptom of disagreeing is *every login failing* with both sides
//! looking correct. There is one implementation and a golden over its output.
//!
//! ## The nonce is per-connection and never stored
//!
//! The server generates it on the handshake task's own stack and compares
//! against that local value. There is no nonce table to size, expire, or
//! sweep, and a signature captured on one connection is worthless on another
//! because no other connection ever chose that nonce. Bounded by
//! construction rather than by a cap (wall 4 with nothing to configure).
//!
//! Nothing here logs a signature or an address.

use crate::bits::{BitReader, BitWriter, WireError};

/// An Ethereum address: 20 bytes, the last 20 of `keccak256` of the
/// uncompressed public key. **This is the player key** — what a save is
/// filed under (`server/src/store.rs`) and what a sleeping body is claimed
/// with (`server/src/worldfile.rs`).
///
/// Raw bytes on the wire rather than the 42-character `0x…` text, because
/// the text has a case question (EIP-55 checksumming) and a key that
/// compared differently depending on capitalisation would file one player
/// under two saves. Rendering is [`Address::to_hex`], and it is lowercase.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub struct Address(pub [u8; ADDRESS_BYTES]);

pub const ADDRESS_BYTES: usize = 20;
/// r ‖ s ‖ v — 32 + 32 + 1, the shape `personal_sign` returns everywhere.
pub const SIGNATURE_BYTES: usize = 65;
/// The server's per-connection nonce. 32 bytes because that is what SIWE
/// implementations use and there is no reason to be interesting about it.
pub const NONCE_BYTES: usize = 32;

impl Address {
    /// The all-zero address, which is **a guest and never a player**. A
    /// shard with `require_auth = false` admits one and remembers nobody,
    /// which is exactly what every shard did before identity existed.
    ///
    /// Safe as a sentinel because no private key produces it: it would take
    /// a keccak preimage, which is the assumption the whole scheme already
    /// rests on. So a guest can never collide with somebody's save.
    pub const GUEST: Self = Self([0; ADDRESS_BYTES]);

    pub fn is_guest(&self) -> bool {
        *self == Self::GUEST
    }

    /// Lowercase `0x…`, 42 characters. Lowercase and not EIP-55 on purpose:
    /// this string is what gets written to a save file, and a checksummed
    /// one would make the key depend on how the address was capitalised
    /// where it came from.
    pub fn to_hex(&self) -> [u8; 42] {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut out = [0u8; 42];
        out[0] = b'0';
        out[1] = b'x';
        for (i, b) in self.0.iter().enumerate() {
            out[2 + i * 2] = HEX[(b >> 4) as usize];
            out[3 + i * 2] = HEX[(b & 0xf) as usize];
        }
        out
    }

    /// Parse `0x…` (either case) into an address. `None` on any other shape
    /// — length, prefix or a non-hex digit. Used by `--identity` and by
    /// anything reading an address out of a file.
    pub fn from_hex(s: &[u8]) -> Option<Self> {
        if s.len() != 42 || s[0] != b'0' || (s[1] != b'x' && s[1] != b'X') {
            return None;
        }
        let nib = |c: u8| -> Option<u8> {
            match c {
                b'0'..=b'9' => Some(c - b'0'),
                b'a'..=b'f' => Some(c - b'a' + 10),
                b'A'..=b'F' => Some(c - b'A' + 10),
                _ => None,
            }
        };
        let mut a = [0u8; ADDRESS_BYTES];
        for i in 0..ADDRESS_BYTES {
            a[i] = (nib(s[2 + i * 2])? << 4) | nib(s[3 + i * 2])?;
        }
        Some(Self(a))
    }
}

/// Prints the address, which is safe and useful — it is a public name, not a
/// credential, and an operator debugging "who took that base" needs to read
/// it. The opposite rule from the session token this replaced, and the
/// difference is the whole reason the address model is better: there is no
/// secret in the identity any more.
impl core::fmt::Debug for Address {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let h = self.to_hex();
        f.write_str(core::str::from_utf8(&h).unwrap_or("0x?"))
    }
}

/// A `personal_sign` signature, r ‖ s ‖ v.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Signature(pub [u8; SIGNATURE_BYTES]);

impl Default for Signature {
    fn default() -> Self {
        Self::NONE
    }
}

impl Signature {
    pub const NONE: Self = Self([0; SIGNATURE_BYTES]);
    pub fn is_none(&self) -> bool {
        self.0 == [0; SIGNATURE_BYTES]
    }
}

/// Never prints the bytes. Not because a signature is a credential — it is
/// spent the moment its nonce is — but because a signature in a log is
/// noise that looks like a secret, and the habit is worth keeping.
impl core::fmt::Debug for Signature {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Signature({} bytes)", SIGNATURE_BYTES)
    }
}

/// S→C: sign this. The nonce the server chose for **this connection only**.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Challenge {
    pub nonce: [u8; NONCE_BYTES],
    /// Unix seconds, for the `Issued At` line. The server's clock, echoed
    /// back by nobody — it is part of the signed text, so the client cannot
    /// move it.
    pub issued_at: u64,
}

impl Default for Challenge {
    fn default() -> Self {
        Self {
            nonce: [0; NONCE_BYTES],
            issued_at: 0,
        }
    }
}

impl core::fmt::Debug for Challenge {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Challenge(issued_at {})", self.issued_at)
    }
}

/// C→S: this is me, and here is the proof.
///
/// A guest sends [`Address::GUEST`] and [`Signature::NONE`]; a shard with
/// `require_auth = false` admits it and remembers nobody. One message shape
/// for both cases rather than an optional one, because a handshake with two
/// shapes is a handshake with two code paths and one of them gets less
/// testing.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Auth {
    pub address: Address,
    pub signature: Signature,
}

/// The exact text that gets signed — **EIP-4361, and the one implementation
/// both sides use.**
///
/// Written into a caller buffer rather than returned as a `String` so this
/// crate keeps allocating nothing; [`SIWE_MESSAGE_MAX`] is the ceiling and
/// the return is the length. The fields and their order are the standard's:
/// changing either invalidates every signature in flight and is a
/// `PROTO_VER` turn like any other wire change, which is why the golden in
/// this module pins the bytes.
///
/// `domain` is the shard's own name, and it is what stops a signature
/// collected by one server being replayed at another.
///
/// ⚠ **These bytes are the scry launcher's, not ours, and that is the point.**
/// This used to be a message Gates composed and asked the launcher to `sign`;
/// the launcher refused every one of them, because `sign` classifies a message
/// by its first line (`scry <family>`) and an EIP-4361 message begins with a
/// domain. The refusal became `None`, `None` means "connect as a guest", and a
/// shard with `require_auth` answered `REFUSE_AUTH` — a login that looked like
/// a signing failure and was really a message nothing would ever sign.
///
/// The verb that works is `prove`, where the LAUNCHER writes every word so a
/// game cannot smuggle a sentence into a signature. That makes this function a
/// **recomputation of somebody else's format**: it must equal
/// `scry_broker::protocol::prove_message` byte for byte or nothing verifies.
/// Four of its fields are theirs and are not ours to prefer — the statement,
/// `https://` (not a custom scheme), chain 4663, and an ISO-8601 `Issued At`.
///
/// `address` is the **EIP-55 checksummed** text, not [`Address::to_hex`]'s
/// lowercase. EIP-4361 requires it, reference parsers reject an all-lowercase
/// address, and the launcher binds the checksummed form into the bytes it
/// signs — so the case is load-bearing and the caller must have keccak to
/// produce it. `Address::to_hex` is deliberately still lowercase for every
/// other use.
pub fn siwe_message(
    domain: &str,
    address: &str,
    game: &str,
    nonce: &[u8; NONCE_BYTES],
    issued_at: u64,
    out: &mut [u8; SIWE_MESSAGE_MAX],
) -> usize {
    let mut w = TextWriter { out, at: 0 };
    w.s(domain);
    w.s(" wants you to sign in with your Ethereum account:\n");
    w.s(address);
    w.s("\n\nProve your identity to play ");
    w.s(game);
    w.s(". This signature authorises nothing and moves no funds.\n\nURI: https://");
    w.s(domain);
    w.s("\nVersion: 1\nChain ID: ");
    w.num(CHAIN_ID);
    w.s("\nNonce: ");
    w.hex(nonce);
    w.s("\nIssued At: ");
    w.iso8601(issued_at);
    w.at.min(SIWE_MESSAGE_MAX)
}

/// The chain the launcher names in a proof. **Theirs, not ours**
/// (`scry_broker::protocol::CHAIN_ID`) — we recompute their message, so a
/// number we preferred here would simply fail to verify.
pub const CHAIN_ID: u64 = 4663;

/// Ceiling for [`siwe_message`]: the fixed text is ~220 bytes, the address
/// 42, the nonce 64 hex, the timestamp ≤ 20, and `domain` appears twice and
/// is capped by [`DOMAIN_MAX`].
pub const SIWE_MESSAGE_MAX: usize = 512;
/// Longest shard domain that fits the message above with room to spare.
pub const DOMAIN_MAX: usize = 64;

struct TextWriter<'a> {
    out: &'a mut [u8; SIWE_MESSAGE_MAX],
    at: usize,
}

impl TextWriter<'_> {
    fn bytes(&mut self, b: &[u8]) {
        let end = (self.at + b.len()).min(SIWE_MESSAGE_MAX);
        let n = end - self.at.min(end);
        self.out[self.at.min(end)..end].copy_from_slice(&b[..n]);
        self.at += b.len();
    }
    fn s(&mut self, s: &str) {
        self.bytes(s.as_bytes());
    }
    fn hex(&mut self, b: &[u8]) {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        for x in b {
            self.bytes(&[HEX[(x >> 4) as usize], HEX[(x & 0xf) as usize]]);
        }
    }
    fn num(&mut self, mut v: u64) {
        let mut buf = [0u8; 20];
        let mut n = 0;
        loop {
            buf[n] = b'0' + (v % 10) as u8;
            v /= 10;
            n += 1;
            if v == 0 {
                break;
            }
        }
        for i in (0..n).rev() {
            self.bytes(&[buf[i]]);
        }
    }
    /// Zero-padded to `w` digits — `07`, never `7`.
    fn pad2(&mut self, v: u64) {
        self.bytes(&[b'0' + (v / 10) as u8, b'0' + (v % 10) as u8]);
    }
    /// `YYYY-MM-DDTHH:MM:SSZ`, which is what the launcher writes.
    ///
    /// Reimplemented rather than pulled in: this crate takes no dependency for
    /// four lines of arithmetic, and the algorithm is Howard Hinnant's
    /// `civil_from_days`, the same one `scry_broker::protocol` uses. The pinned
    /// golden below is what actually holds the two together — an agreement
    /// between two crates in two repos is a claim, and only a test is evidence.
    fn iso8601(&mut self, unix_secs: u64) {
        let days = (unix_secs / 86_400) as i64;
        let rem = unix_secs % 86_400;
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z.rem_euclid(146_097);
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let y = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = (doy - (153 * mp + 2) / 5 + 1) as u64;
        let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u64;
        let y = if m <= 2 { y + 1 } else { y } as u64;
        // Four digits, and a shard whose clock says year 10000 has a worse
        // problem than this line.
        self.pad2(y / 100);
        self.pad2(y % 100);
        self.bytes(b"-");
        self.pad2(m);
        self.bytes(b"-");
        self.pad2(d);
        self.bytes(b"T");
        self.pad2(rem / 3600);
        self.bytes(b":");
        self.pad2((rem % 3600) / 60);
        self.bytes(b":");
        self.pad2(rem % 60);
        self.bytes(b"Z");
    }
}

pub(crate) fn write_challenge(w: &mut BitWriter, c: &Challenge) -> Result<(), WireError> {
    for b in c.nonce.iter() {
        w.write(*b as u32, 8)?;
    }
    w.write(c.issued_at as u32, 32)?;
    w.write((c.issued_at >> 32) as u32, 32)?;
    Ok(())
}

pub(crate) fn read_challenge(r: &mut BitReader) -> Result<Challenge, WireError> {
    let mut nonce = [0u8; NONCE_BYTES];
    for b in nonce.iter_mut() {
        *b = r.read(8)? as u8;
    }
    let lo = r.read(32)? as u64;
    let hi = r.read(32)? as u64;
    Ok(Challenge {
        nonce,
        issued_at: lo | (hi << 32),
    })
}

pub(crate) fn write_auth(w: &mut BitWriter, a: &Auth) -> Result<(), WireError> {
    for b in a.address.0.iter() {
        w.write(*b as u32, 8)?;
    }
    for b in a.signature.0.iter() {
        w.write(*b as u32, 8)?;
    }
    Ok(())
}

pub(crate) fn read_auth(r: &mut BitReader) -> Result<Auth, WireError> {
    let mut address = [0u8; ADDRESS_BYTES];
    for b in address.iter_mut() {
        *b = r.read(8)? as u8;
    }
    let mut signature = [0u8; SIGNATURE_BYTES];
    for b in signature.iter_mut() {
        *b = r.read(8)? as u8;
    }
    Ok(Auth {
        address: Address(address),
        signature: Signature(signature),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The address for private key 1, which is a widely published vector.
    /// Pinned here because **this is the one number in the whole scheme that
    /// silently ruins everything if it is wrong**: derive the address a
    /// different way and every player's save is filed under a key that is
    /// not theirs, consistently, with every other test passing.
    const PRIV1: &[u8] = b"0x7e5f4552091a69125d5dfcb7b8c2659029395bdf";

    #[test]
    fn hex_round_trips_and_is_lowercase() {
        let a = Address::from_hex(PRIV1).expect("a known-good address parses");
        assert_eq!(&a.to_hex(), PRIV1, "rendering is not lowercase 0x-hex");
        // Mixed case in, identical bytes out — the EIP-55 question cannot
        // reach the key.
        let mixed = b"0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf";
        assert_eq!(Address::from_hex(mixed), Some(a), "case changed the key");
    }

    #[test]
    fn a_malformed_address_is_refused_rather_than_padded() {
        assert_eq!(Address::from_hex(b""), None);
        assert_eq!(Address::from_hex(b"0x"), None);
        assert_eq!(Address::from_hex(&PRIV1[..41]), None, "one short");
        assert_eq!(
            Address::from_hex(b"7e5f4552091a69125d5dfcb7b8c2659029395bdf"),
            None
        );
        let mut bad = *b"0x7e5f4552091a69125d5dfcb7b8c2659029395bdg";
        assert_eq!(Address::from_hex(&bad), None, "g is not hex");
        bad[41] = b'f';
        assert!(Address::from_hex(&bad).is_some());
    }

    /// The guest sentinel is not a player and cannot become one.
    #[test]
    fn the_guest_address_is_nobody() {
        assert!(Address::GUEST.is_guest());
        assert!(!Address::from_hex(PRIV1).unwrap().is_guest());
        assert_eq!(&Address::GUEST.to_hex()[..2], b"0x");
    }

    /// **The signed text, byte for byte.** A signature is over exact bytes,
    /// so this golden is the thing that makes client and server agree — and
    /// the failure it prevents is not subtle in effect (every login is
    /// refused) but is very subtle in cause (one space).
    #[test]
    fn the_siwe_message_is_pinned_byte_for_byte() {
        let a = Address::from_hex(PRIV1).unwrap();
        let mut nonce = [0u8; NONCE_BYTES];
        for (i, b) in nonce.iter_mut().enumerate() {
            *b = i as u8;
        }
        let mut buf = [0u8; SIWE_MESSAGE_MAX];
        // EIP-55 checksummed, as the launcher supplies it. Lowercase here
        // would recompute to different bytes and verify nothing.
        let checksummed = "0x7E5F4552091A69125d5DfcB7b8C2659029395Bdf";
        let n = siwe_message(
            "gates.example",
            checksummed,
            "gates",
            &nonce,
            1_770_000_000,
            &mut buf,
        );
        let text = core::str::from_utf8(&buf[..n]).expect("the message is utf-8");
        // **This is scry's format, transcribed.** It must equal what
        // `scry_broker::protocol::prove_message` writes for the same inputs;
        // the launcher signs THAT, and a shard that recomputes anything else
        // rejects every honest login. Changing a byte here is changing what
        // the launcher does, not what we prefer.
        assert_eq!(
            text,
            "gates.example wants you to sign in with your Ethereum account:\n\
             0x7E5F4552091A69125d5DfcB7b8C2659029395Bdf\n\n\
             Prove your identity to play gates. This signature authorises \
             nothing and moves no funds.\n\n\
             URI: https://gates.example\nVersion: 1\nChain ID: 4663\n\
             Nonce: 000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f\n\
             Issued At: 2026-02-02T02:40:00Z"
        );
        assert!(n < SIWE_MESSAGE_MAX, "the ceiling must have headroom");
        let _ = a;
    }

    /// The timestamp is the field this whole change exists for, so it is
    /// pinned away from a single happy value: a leap day, a year boundary,
    /// and midnight, each of which a hand-rolled civil-from-days gets wrong
    /// in its own way.
    #[test]
    fn iso8601_matches_the_launchers_formatter() {
        let cases = [
            (0u64, "1970-01-01T00:00:00Z"),
            (951_782_400, "2000-02-29T00:00:00Z"), // a leap day
            (1_767_225_599, "2025-12-31T23:59:59Z"), // one second before a year
            (1_767_225_600, "2026-01-01T00:00:00Z"), // and the year itself
            (1_770_000_000, "2026-02-02T02:40:00Z"),
        ];
        for (secs, want) in cases {
            let mut buf = [0u8; SIWE_MESSAGE_MAX];
            let mut w = TextWriter {
                out: &mut buf,
                at: 0,
            };
            w.iso8601(secs);
            let at = w.at;
            let got = core::str::from_utf8(&buf[..at]).expect("utf-8");
            assert_eq!(got, want, "unix {secs}");
        }
    }

    /// The nonce reaches the text. Without this, every connection would sign
    /// the same string and a captured signature would be a permanent
    /// password.
    #[test]
    fn a_different_nonce_is_a_different_message() {
        let hex = Address::GUEST.to_hex();
        let a = core::str::from_utf8(&hex).expect("utf-8");
        let mut one = [0u8; SIWE_MESSAGE_MAX];
        let mut two = [0u8; SIWE_MESSAGE_MAX];
        let n1 = siwe_message("d", a, "gates", &[1; NONCE_BYTES], 5, &mut one);
        let n2 = siwe_message("d", a, "gates", &[2; NONCE_BYTES], 5, &mut two);
        assert_eq!(n1, n2);
        assert_ne!(one[..n1], two[..n2], "the nonce did not reach the text");
    }

    /// The domain reaches the text, which is what stops a signature given to
    /// one shard being replayed at another.
    #[test]
    fn a_different_domain_is_a_different_message() {
        let hex = Address::GUEST.to_hex();
        let a = core::str::from_utf8(&hex).expect("utf-8");
        let mut one = [0u8; SIWE_MESSAGE_MAX];
        let mut two = [0u8; SIWE_MESSAGE_MAX];
        let n1 = siwe_message("shard-a", a, "gates", &[0; NONCE_BYTES], 5, &mut one);
        let n2 = siwe_message("shard-b", a, "gates", &[0; NONCE_BYTES], 5, &mut two);
        assert_ne!(one[..n1], two[..n2], "the domain did not reach the text");
    }

    /// A domain at the cap still fits, and an over-long one truncates rather
    /// than running off the buffer. Truncation is safe here and refusal is
    /// not needed: both sides build the same text from the same domain, so a
    /// truncated one still agrees with itself — and `DOMAIN_MAX` is checked
    /// where the domain is configured, not here.
    #[test]
    fn the_message_buffer_cannot_be_overrun() {
        let long = "x".repeat(4096);
        let mut buf = [0u8; SIWE_MESSAGE_MAX];
        let hex = Address::GUEST.to_hex();
        let a = core::str::from_utf8(&hex).expect("utf-8");
        let n = siwe_message(&long, a, "gates", &[0; NONCE_BYTES], 0, &mut buf);
        assert!(n >= SIWE_MESSAGE_MAX, "an over-long domain should saturate");
    }
}
