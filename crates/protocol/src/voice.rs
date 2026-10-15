//! Proximity voice (v111): one Opus frame per datagram, both directions,
//! on [`KIND_VOICE`].
//!
//! C→S `encode_voice_up` carries the talker's own frame: a sequence number
//! and the payload. S→C `encode_voice` is the server's relay of it to one
//! listener, with the talker's player id in front. The server picks who
//! gets a copy (`reference/VOICE.md` §9.1: the cull is the server's or it
//! is a wallhack), so a client never receives a frame from outside the
//! radius and its own falloff is cosmetic.
//!
//! The payload is opaque to the wire: an Opus packet (`crates/voice`),
//! capped at [`VOICE_MAX_BYTES`] so a frame always fits the datagram
//! budget. Like chat, voice is never sim state: it is relayed, never
//! simulated, and never reaches the WAL.

use crate::bits::{BitReader, BitWriter, WireError};
use crate::{expect_zero_padding, KIND_BITS, KIND_VOICE};

/// Longest Opus payload one frame carries. 20 ms at 64 kbps is 160 bytes;
/// the codec runs near 24 kbps (~60 B), so this is headroom, not a target.
pub const VOICE_MAX_BYTES: usize = 160;

/// The last frame of one push of the talk key: a listener's talking light
/// goes out on it, and an agent's ears end the utterance there.
pub const VOICE_END: u8 = 1;

const FLAG_BITS: u32 = 4;
const LEN_BITS: u32 = 8;

/// One decoded voice frame. `from` is 0 on the C→S side (the server knows
/// who sent it) and the talker's player id on the S→C side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VoiceFrame {
    pub from: u32,
    pub seq: u16,
    pub flags: u8,
    len: u8,
    bytes: [u8; VOICE_MAX_BYTES],
}

impl VoiceFrame {
    pub const EMPTY: Self = Self {
        from: 0,
        seq: 0,
        flags: 0,
        len: 0,
        bytes: [0; VOICE_MAX_BYTES],
    };

    /// A frame around `payload`; `None` when it is empty or over the cap.
    pub fn new(seq: u16, flags: u8, payload: &[u8]) -> Option<Self> {
        if payload.is_empty() || payload.len() > VOICE_MAX_BYTES || flags > VOICE_END {
            return None;
        }
        let mut f = Self::EMPTY;
        f.seq = seq;
        f.flags = flags;
        f.len = payload.len() as u8;
        f.bytes[..payload.len()].copy_from_slice(payload);
        Some(f)
    }

    pub fn payload(&self) -> &[u8] {
        &self.bytes[..self.len as usize]
    }

    pub fn is_end(&self) -> bool {
        self.flags & VOICE_END != 0
    }
}

impl Default for VoiceFrame {
    fn default() -> Self {
        Self::EMPTY
    }
}

fn write_body(w: &mut BitWriter, f: &VoiceFrame) -> Result<(), WireError> {
    w.write(f.seq as u32, 16)?;
    w.write(f.len as u32, LEN_BITS)?;
    for &b in f.payload() {
        w.write(b as u32, 8)?;
    }
    Ok(())
}

fn read_body(r: &mut BitReader, from: u32, flags: u8) -> Result<VoiceFrame, WireError> {
    let seq = r.read(16)? as u16;
    let len = r.read(LEN_BITS)? as usize;
    if len == 0 || len > VOICE_MAX_BYTES {
        return Err(WireError::Malformed);
    }
    let mut f = VoiceFrame::EMPTY;
    for b in f.bytes.iter_mut().take(len) {
        *b = r.read(8)? as u8;
    }
    f.from = from;
    f.seq = seq;
    f.flags = flags;
    f.len = len as u8;
    expect_zero_padding(r)?;
    Ok(f)
}

fn read_flags(r: &mut BitReader) -> Result<u8, WireError> {
    if r.read(KIND_BITS)? != KIND_VOICE {
        return Err(WireError::Malformed);
    }
    let flags = r.read(FLAG_BITS)? as u8;
    if flags > VOICE_END {
        return Err(WireError::Malformed);
    }
    Ok(flags)
}

/// C→S: the talker's own frame.
pub fn encode_voice_up(f: &VoiceFrame, buf: &mut [u8]) -> Result<usize, WireError> {
    if f.len == 0 {
        return Err(WireError::Range);
    }
    let mut w = BitWriter::new(buf);
    w.write(KIND_VOICE, KIND_BITS)?;
    w.write(f.flags as u32, FLAG_BITS)?;
    write_body(&mut w, f)?;
    Ok(w.finish())
}

/// Total decode of a C→S voice datagram (client-driven bytes: never panics).
pub fn decode_voice_up(buf: &[u8]) -> Result<VoiceFrame, WireError> {
    let mut r = BitReader::new(buf);
    let flags = read_flags(&mut r)?;
    read_body(&mut r, 0, flags)
}

/// S→C: `f` relayed to a listener, from player `f.from`.
pub fn encode_voice(f: &VoiceFrame, buf: &mut [u8]) -> Result<usize, WireError> {
    if f.len == 0 {
        return Err(WireError::Range);
    }
    let mut w = BitWriter::new(buf);
    w.write(KIND_VOICE, KIND_BITS)?;
    w.write(f.flags as u32, FLAG_BITS)?;
    w.write(f.from, 32)?;
    write_body(&mut w, f)?;
    Ok(w.finish())
}

/// Total decode of an S→C voice datagram.
pub fn decode_voice(buf: &[u8]) -> Result<VoiceFrame, WireError> {
    let mut r = BitReader::new(buf);
    let flags = read_flags(&mut r)?;
    let from = r.read(32)?;
    read_body(&mut r, from, flags)
}

/// Bytes an S→C voice datagram can take: header, id, seq, length, payload.
pub const VOICE_DATAGRAM_MAX: usize = 1 + 4 + 2 + 1 + VOICE_MAX_BYTES;
const _: () = assert!(VOICE_DATAGRAM_MAX <= sim_core::limits::DATAGRAM_BUDGET_BYTES);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_both_directions() {
        let f = VoiceFrame::new(65_000, VOICE_END, &[1, 2, 3, 250]).unwrap();
        let mut buf = [0u8; VOICE_DATAGRAM_MAX];
        let n = encode_voice_up(&f, &mut buf).unwrap();
        assert_eq!(n, 4 + 4);
        let back = decode_voice_up(&buf[..n]).unwrap();
        assert_eq!(back, f);
        assert!(back.is_end());

        let mut relayed = f;
        relayed.from = 0x0123_4507;
        let n = encode_voice(&relayed, &mut buf).unwrap();
        assert_eq!(n, 8 + 4);
        assert_eq!(decode_voice(&buf[..n]).unwrap(), relayed);
    }

    #[test]
    fn refuses_bad_frames() {
        assert!(VoiceFrame::new(0, 0, &[]).is_none());
        assert!(VoiceFrame::new(0, 0, &[0; VOICE_MAX_BYTES + 1]).is_none());
        assert!(VoiceFrame::new(0, 2, &[1]).is_none());
        let full = VoiceFrame::new(1, 0, &[7; VOICE_MAX_BYTES]).unwrap();
        let mut buf = [0u8; VOICE_DATAGRAM_MAX];
        let n = encode_voice(&full, &mut buf).unwrap();
        assert_eq!(n, VOICE_DATAGRAM_MAX);
        // Truncated, trailing junk, a forged length, another kind.
        assert!(decode_voice(&buf[..n - 1]).is_err());
        let n = encode_voice_up(&full, &mut buf).unwrap();
        let mut long = buf;
        long[n] = 0xff;
        assert!(decode_voice_up(&long[..n + 1]).is_err());
        let mut forged = buf;
        forged[3] = 0;
        assert!(decode_voice_up(&forged[..n]).is_err());
        assert!(decode_voice_up(&[0x00, 0, 0, 1, 1]).is_err());
    }
}
