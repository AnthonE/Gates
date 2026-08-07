//! The session token a launcher hands the game, and the game hands the shard.
//!
//! **This is the Steam model and nothing cleverer.** scry's launcher knows who
//! is playing; it issues a session token; the game relays that token in its
//! `Hello`; the shard asks scry whether the token is good and gets back a
//! stable player key. Rust-the-game does exactly this with Steam auth tickets,
//! and the reason to copy it is that every part is boring and none of it needs
//! a wallet, a nonce round-trip, or a chain lookup to answer "who is this".
//!
//! **Gates never parses the token.** It is opaque bytes with a length, relayed
//! whole — so the shape of scry's session handle can change without touching
//! this wire, and Gates cannot accidentally start trusting something it read
//! out of the middle of a credential. The only structural claim made here is
//! that it fits in a `Hello`.
//!
//! **A token is not an identity until a shard has validated it.** Holding one
//! proves nothing: any patched client can put bytes in this field, exactly as
//! any patched client can claim an address. What makes it authentication is
//! the shard asking the issuer — which is `server`'s job, not this module's.
//! No token is ever logged, and `Debug` prints its length and never its bytes.

use crate::bits::{BitReader, BitWriter, WireError};

/// Longest session token on the wire, in bytes.
///
/// Derived the way `CHAT_MAX_BYTES` is, against the same ceiling:
/// `MAX_STREAM_MSG_BYTES` is 64, and a `Hello` spends 3 bits of kind, 16 of
/// version and 6 of length before the token starts — 48 bytes then encode to
/// 52 with room to spare, and the round number leaves margin for one more
/// small field before this lane needs widening.
///
/// **It is deliberately too small for a JWT**, and that is a design statement
/// rather than an oversight: a self-describing token would invite Gates to
/// read claims out of it and make decisions on them, which is the thing that
/// turns a relay into a second, worse auth implementation. 48 bytes is a
/// session HANDLE — an opaque lookup key the issuer resolves.
/// Proposed default, DECISIONS.md §open ("scry session auth v0").
pub const AUTH_TOKEN_MAX_BYTES: usize = 48;

/// Length field width: 6 bits hold 0..63, covering the cap above. A forged
/// length past it is refused at decode rather than clamped.
pub(crate) const AUTH_LEN_BITS: u32 = 6;

/// One opaque session token. Fixed bytes, no allocation, `Copy` like every
/// other wire record.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct AuthToken {
    bytes: [u8; AUTH_TOKEN_MAX_BYTES],
    len: u8,
}

impl Default for AuthToken {
    fn default() -> Self {
        Self::NONE
    }
}

/// Never prints the token. A credential in a log is a credential leaked, and
/// this type crosses `Debug` boundaries (`WireError` reporting, test output,
/// `bot_smoke`'s dumps) often enough that the derive would have found one.
impl core::fmt::Debug for AuthToken {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "AuthToken({} bytes)", self.len)
    }
}

impl AuthToken {
    /// No launcher, no token. **A normal, playable state** — the same posture
    /// `scry::Player::Anonymous` takes. An unauthenticated player is a guest
    /// on a shard that allows guests and is refused by one that does not;
    /// that is the shard's policy and not this type's.
    pub const NONE: Self = Self {
        bytes: [0; AUTH_TOKEN_MAX_BYTES],
        len: 0,
    };

    /// Wrap raw bytes. `None` ⇒ too long: refuse rather than truncate, because
    /// half a credential is not a credential and would fail validation in a
    /// way that looks like a bad token instead of a bad client.
    pub fn new(raw: &[u8]) -> Option<Self> {
        if raw.len() > AUTH_TOKEN_MAX_BYTES {
            return None;
        }
        let mut bytes = [0; AUTH_TOKEN_MAX_BYTES];
        bytes[..raw.len()].copy_from_slice(raw);
        Some(Self {
            bytes,
            len: raw.len() as u8,
        })
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len as usize]
    }

    pub fn len(&self) -> usize {
        self.len as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

/// Write the token's length and bytes. Shared by `encode_hello`.
pub(crate) fn write_token(w: &mut BitWriter<'_>, t: &AuthToken) -> Result<(), WireError> {
    w.write(t.len as u32, AUTH_LEN_BITS)?;
    for &b in t.as_bytes() {
        w.write(b as u32, 8)?;
    }
    Ok(())
}

/// Read a token. A length past the cap is `Malformed` — the cap is the range
/// check, the hotbar selector's posture.
pub(crate) fn read_token(r: &mut BitReader<'_>) -> Result<AuthToken, WireError> {
    let len = r.read(AUTH_LEN_BITS)? as usize;
    if len > AUTH_TOKEN_MAX_BYTES {
        return Err(WireError::Malformed);
    }
    let mut bytes = [0; AUTH_TOKEN_MAX_BYTES];
    for b in bytes.iter_mut().take(len) {
        *b = r.read(8)? as u8;
    }
    Ok(AuthToken {
        bytes,
        len: len as u8,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{decode_hello, encode_hello, Hello, MAX_STREAM_MSG_BYTES, PROTO_VER};

    #[test]
    fn a_token_round_trips_through_a_hello() {
        for raw in [
            &b""[..],
            &b"a"[..],
            &b"gates-fixture-token-0123456789ab"[..],
            &[0xffu8; AUTH_TOKEN_MAX_BYTES][..],
        ] {
            let token = AuthToken::new(raw).expect("within cap");
            let mut buf = [0u8; MAX_STREAM_MSG_BYTES];
            let n = encode_hello(
                &Hello {
                    proto_ver: PROTO_VER,
                    token,
                },
                &mut buf,
            )
            .expect("encodes");
            let back = decode_hello(&buf[..n]).expect("decodes");
            assert_eq!(
                back.token.as_bytes(),
                raw,
                "token drifted at {} bytes",
                raw.len()
            );
            assert_eq!(back.proto_ver, PROTO_VER);
        }
    }

    /// The longest legal token must still fit the frame the handshake
    /// accepts. This is the derivation in `AUTH_TOKEN_MAX_BYTES`'s doc,
    /// asserted rather than trusted — if either constant moves, the wire
    /// stops fitting its own envelope and this says so.
    #[test]
    fn the_biggest_token_fits_the_frame() {
        let token = AuthToken::new(&[0xab; AUTH_TOKEN_MAX_BYTES]).expect("at cap");
        let mut buf = [0u8; MAX_STREAM_MSG_BYTES];
        let n = encode_hello(
            &Hello {
                proto_ver: PROTO_VER,
                token,
            },
            &mut buf,
        )
        .expect("a token at the cap must fit MAX_STREAM_MSG_BYTES");
        assert!(n <= MAX_STREAM_MSG_BYTES, "{n} > {MAX_STREAM_MSG_BYTES}");
    }

    /// Over the cap is refused, never truncated: half a credential fails
    /// validation in a way that looks like a bad token instead of a bad
    /// client.
    #[test]
    fn an_over_long_token_is_refused_not_truncated() {
        assert!(AuthToken::new(&[0u8; AUTH_TOKEN_MAX_BYTES + 1]).is_none());
        assert!(AuthToken::new(&[0u8; AUTH_TOKEN_MAX_BYTES]).is_some());
    }

    /// A forged length past the cap is `Malformed` at decode. Built by hand
    /// because no encoder can produce it — which is exactly why the decoder
    /// has to refuse it rather than trusting its own writer.
    #[test]
    fn a_forged_length_is_malformed() {
        let mut buf = [0u8; MAX_STREAM_MSG_BYTES];
        let mut w = BitWriter::new(&mut buf);
        w.write(crate::KIND_HELLO, crate::KIND_BITS).unwrap();
        w.write(PROTO_VER as u32, 16).unwrap();
        w.write(63, AUTH_LEN_BITS).unwrap(); // 63 > AUTH_TOKEN_MAX_BYTES
        let n = w.finish();
        assert!(
            decode_hello(&buf[..n]).is_err(),
            "a 63-byte length must refuse"
        );
    }

    /// A fixed-capacity sink, because `format!` is a disallowed macro in
    /// this crate (`clippy.toml`, wall 3 — no `String` on the hot path) and
    /// the right answer to a wall is to obey it, not to `allow` past it in
    /// the one file that wanted a convenience.
    struct Sink {
        buf: [u8; 128],
        len: usize,
    }

    impl core::fmt::Write for Sink {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            let b = s.as_bytes();
            if self.len + b.len() > self.buf.len() {
                return Err(core::fmt::Error);
            }
            self.buf[self.len..self.len + b.len()].copy_from_slice(b);
            self.len += b.len();
            Ok(())
        }
    }

    /// A credential must never reach a log. `Debug` is hand-written for this
    /// and the test is what keeps a future `derive` from undoing it.
    #[test]
    fn debug_never_prints_the_token() {
        use core::fmt::Write;
        let secret = b"super-secret-session-handle";
        let t = AuthToken::new(secret).expect("fits");
        let mut sink = Sink {
            buf: [0; 128],
            len: 0,
        };
        write!(sink, "{t:?}").expect("fits the sink");
        let shown = core::str::from_utf8(&sink.buf[..sink.len]).expect("utf8");
        assert!(!shown.contains("super"), "token leaked into Debug");
        assert!(!shown.contains("secret"), "token leaked into Debug");
        assert!(
            shown.contains("27 bytes"),
            "expected a length, got: {shown}"
        );
    }
}
