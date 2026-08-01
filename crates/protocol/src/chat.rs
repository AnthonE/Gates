//! Chat, both directions (ALPHA.md §1: "global text + 20 m local,
//! profanity untouched, rate-limited server-side, on the bidi lane";
//! DESIGN.md §5.9 and NETCODE.md §2 both list chat on the reliable
//! stream). C→S is its own kind (`KIND_CHAT`); S→C rides the event lane
//! as a subtype, like every other fan-out.
//!
//! Chat is the only **player-authored** payload on the wire, so it is the
//! one place bytes arrive that no schema constrains. The rule here: a
//! line is sanitized at both edges — the encoder refuses to build a bad
//! one, the decoder refuses to accept one — so nothing downstream (the
//! sim thread, the fan-out, the browser DOM) ever holds text that failed
//! the check. Sanitizing is idempotent and the decoder refuses a line it
//! would have changed, so the wire form is canonical: one line, one
//! encoding, which is what makes the golden mean something.
//!
//! Text is a fixed `[u8; CHAT_MAX_BYTES]`, never a `String` — the fan-out
//! runs on the sim thread (CLAUDE.md wall 3), and the protocol crate is
//! under the sim's discipline besides.
//!
//! **Chat is not sim state.** It never becomes a `Command`, never enters
//! `World`, and never reaches the WAL — so a replay reproduces a round
//! without reproducing what anyone typed, and determinism cannot depend
//! on it. That is the whole reason this module exists beside the action
//! lane instead of inside it.

use crate::bits::{BitReader, BitWriter, WireError};
use crate::{expect_zero_padding, KIND_BITS, KIND_CHAT};

/// Longest chat line on the wire, in UTF-8 bytes. Bounded by the server's
/// own C→S frame acceptance (`MAX_STREAM_MSG_BYTES` = 64): 48 text bytes
/// plus the 7-bit header encode to 50 B, which leaves the length prefix
/// and a margin. Proposed default, DECISIONS.md §open ("chat v0").
pub const CHAT_MAX_BYTES: usize = 48;

/// Length field width: 6 bits hold 0..63, covering `CHAT_MAX_BYTES`.
/// A forged length past the cap is refused at decode.
const CHAT_LEN_BITS: u32 = 6;

/// One sanitized chat line: fixed bytes, no allocation, `Copy` so it
/// crosses rings by value like every other wire record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChatText {
    bytes: [u8; CHAT_MAX_BYTES],
    len: u8,
}

impl Default for ChatText {
    fn default() -> Self {
        Self::EMPTY
    }
}

impl ChatText {
    pub const EMPTY: Self = Self {
        bytes: [0; CHAT_MAX_BYTES],
        len: 0,
    };

    /// The line's bytes — valid UTF-8 by construction (`sanitize` is the
    /// only way to build a non-empty one).
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len as usize]
    }

    pub fn len(&self) -> usize {
        self.len as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Trim and validate a raw line. `None` ⇒ refuse, never repair past
    /// the trim: empty, over-long, not UTF-8, or carrying a control
    /// character (which is how a chat line would otherwise smuggle a
    /// newline into a log or a terminal). Profanity is untouched —
    /// survival chat is survival chat (ALPHA.md §1).
    pub fn sanitize(raw: &[u8]) -> Option<Self> {
        // ASCII whitespace only: every byte of a multi-byte UTF-8
        // sequence is ≥ 0x80, so trimming here can never split a
        // codepoint and leave the `from_utf8` below to catch it.
        let mut a = 0;
        let mut b = raw.len();
        while a < b && raw[a].is_ascii_whitespace() {
            a += 1;
        }
        while b > a && raw[b - 1].is_ascii_whitespace() {
            b -= 1;
        }
        let s = &raw[a..b];
        if s.is_empty() || s.len() > CHAT_MAX_BYTES {
            return None;
        }
        let text = core::str::from_utf8(s).ok()?;
        if text.chars().any(|c| c.is_control()) {
            return None;
        }
        let mut out = Self::EMPTY;
        out.bytes[..s.len()].copy_from_slice(s);
        out.len = s.len() as u8;
        Some(out)
    }
}

/// One decoded C→S chat frame. `global` is the sender's channel choice;
/// the server enforces what that means (everyone, or the local radius).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChatMsg {
    pub global: bool,
    pub text: ChatText,
}

/// Encode a C→S chat line. Refuses (`Range`) anything `sanitize` rejects,
/// so a client can never put a line on the wire the server would only
/// throw away.
pub fn encode_chat(raw: &[u8], global: bool, buf: &mut [u8]) -> Result<usize, WireError> {
    let text = ChatText::sanitize(raw).ok_or(WireError::Range)?;
    let mut w = BitWriter::new(buf);
    w.write(KIND_CHAT, KIND_BITS)?;
    w.write_bit(global)?;
    w.write(text.len() as u32, CHAT_LEN_BITS)?;
    for &b in text.as_bytes() {
        w.write(b as u32, 8)?;
    }
    Ok(w.finish())
}

/// Total decode of one C→S chat frame — client-driven bytes, so the same
/// never-panic contract as the input datagrams and the action lane.
///
/// A frame whose text `sanitize` would have altered is `Malformed`, not
/// silently repaired: the wire carries canonical lines, and a peer that
/// skipped the encoder gets refused rather than relayed.
pub fn decode_chat(buf: &[u8]) -> Result<ChatMsg, WireError> {
    let mut r = BitReader::new(buf);
    if r.read(KIND_BITS)? != KIND_CHAT {
        return Err(WireError::Malformed);
    }
    let global = r.read_bit()?;
    let text = read_text(&mut r)?;
    expect_zero_padding(&mut r)?;
    Ok(ChatMsg { global, text })
}

/// The shared line body: length, bytes, sanitize-or-refuse. Used by both
/// directions so a relayed line is held to exactly the rule the sender's
/// line was.
pub(crate) fn read_text(r: &mut BitReader) -> Result<ChatText, WireError> {
    let len = r.read(CHAT_LEN_BITS)? as usize;
    if len == 0 || len > CHAT_MAX_BYTES {
        return Err(WireError::Malformed);
    }
    let mut raw = [0u8; CHAT_MAX_BYTES];
    for b in raw.iter_mut().take(len) {
        *b = r.read(8)? as u8;
    }
    let text = ChatText::sanitize(&raw[..len]).ok_or(WireError::Malformed)?;
    if text.len() != len {
        return Err(WireError::Malformed); // sanitize changed it: not canonical
    }
    Ok(text)
}

pub(crate) fn write_text(w: &mut BitWriter, text: &ChatText) -> Result<(), WireError> {
    if text.is_empty() {
        return Err(WireError::Range);
    }
    w.write(text.len() as u32, CHAT_LEN_BITS)?;
    for &b in text.as_bytes() {
        w.write(b as u32, 8)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(s: &str) -> ChatText {
        ChatText::sanitize(s.as_bytes()).expect("clean line")
    }

    #[test]
    fn sanitize_trims_and_keeps_utf8() {
        let t = line("  hearth is up  ");
        assert_eq!(t.as_bytes(), b"hearth is up");
        let t = line("wall's at 4 — bring stone");
        assert_eq!(t.len(), "wall's at 4 — bring stone".len());
    }

    #[test]
    fn sanitize_refuses_the_bad_shapes() {
        assert!(ChatText::sanitize(b"").is_none());
        assert!(ChatText::sanitize(b"   ").is_none());
        assert!(ChatText::sanitize(&[b'a'; CHAT_MAX_BYTES + 1]).is_none());
        assert!(ChatText::sanitize(b"two\nlines").is_none());
        assert!(ChatText::sanitize(b"bell\x07").is_none());
        assert!(ChatText::sanitize(&[0xff, 0xfe]).is_none());
        // Exactly at the cap is fine.
        assert!(ChatText::sanitize(&[b'a'; CHAT_MAX_BYTES]).is_some());
    }

    #[test]
    fn round_trip() {
        let mut buf = [0u8; 64];
        for global in [false, true] {
            let n = encode_chat(b"raid at the north wall", global, &mut buf).expect("encode");
            let msg = decode_chat(&buf[..n]).expect("decode");
            assert_eq!(msg.global, global);
            assert_eq!(msg.text.as_bytes(), b"raid at the north wall");
        }
    }

    #[test]
    fn encode_refuses_what_sanitize_refuses() {
        let mut buf = [0u8; 64];
        assert_eq!(encode_chat(b"  ", false, &mut buf), Err(WireError::Range));
        assert_eq!(encode_chat(b"a\nb", false, &mut buf), Err(WireError::Range));
        assert_eq!(
            encode_chat(&[b'x'; CHAT_MAX_BYTES + 1], false, &mut buf),
            Err(WireError::Range)
        );
    }

    #[test]
    fn decode_refuses_a_non_canonical_line() {
        // Hand-built frame carrying an untrimmed line — the encoder can't
        // produce this, so only a forging peer can.
        let mut buf = [0u8; 64];
        let mut w = BitWriter::new(&mut buf);
        w.write(KIND_CHAT, KIND_BITS).unwrap();
        w.write_bit(false).unwrap();
        let raw = b" hi ";
        w.write(raw.len() as u32, CHAT_LEN_BITS).unwrap();
        for &b in raw {
            w.write(b as u32, 8).unwrap();
        }
        let n = w.finish();
        assert_eq!(decode_chat(&buf[..n]), Err(WireError::Malformed));
    }

    #[test]
    fn decode_refuses_a_forged_control_char() {
        let mut buf = [0u8; 64];
        let mut w = BitWriter::new(&mut buf);
        w.write(KIND_CHAT, KIND_BITS).unwrap();
        w.write_bit(true).unwrap();
        let raw = b"a\x1b[2Jb";
        w.write(raw.len() as u32, CHAT_LEN_BITS).unwrap();
        for &b in raw {
            w.write(b as u32, 8).unwrap();
        }
        let n = w.finish();
        assert_eq!(decode_chat(&buf[..n]), Err(WireError::Malformed));
    }

    #[test]
    fn decode_refuses_a_forged_length() {
        let mut buf = [0u8; 96];
        let mut w = BitWriter::new(&mut buf);
        w.write(KIND_CHAT, KIND_BITS).unwrap();
        w.write_bit(false).unwrap();
        w.write(63, CHAT_LEN_BITS).unwrap(); // past CHAT_MAX_BYTES
        for _ in 0..63 {
            w.write(b'a' as u32, 8).unwrap();
        }
        let n = w.finish();
        assert_eq!(decode_chat(&buf[..n]), Err(WireError::Malformed));
    }

    #[test]
    fn a_full_line_fits_the_stream_frame() {
        let mut buf = [0u8; crate::MAX_STREAM_MSG_BYTES];
        let n = encode_chat(&[b'z'; CHAT_MAX_BYTES], true, &mut buf).expect("cap line encodes");
        assert!(n <= crate::MAX_STREAM_MSG_BYTES);
    }
}
