//! The bit writer/reader under every packet (DESIGN.md §5.5: "a hand-rolled
//! bit writer"). LSB-first within each byte: the first bit written is bit 0
//! of byte 0. Fixed caller-owned buffers, no allocation, no panics on any
//! input — encode and decode both run on hot paths (the sim thread encodes
//! snapshots, net tasks decode client bytes), so every failure is a
//! `WireError`, never an unwind.

/// Every way a packet operation can refuse. Integer-friendly (L5): no
/// payloads, no strings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WireError {
    /// Writer buffer exhausted. Policy: the caller sheds/defers (limits.rs).
    Overflow,
    /// Reader ran past the end of the payload.
    Truncated,
    /// A count cap (limits.rs) would be exceeded.
    Cap,
    /// Encoder calls out of wire order (removals come before entities).
    Order,
    /// A value outside its wire range — a sim bug surfacing; never clamped
    /// silently, because the sim runs on the values it transmits.
    Range,
    /// Decoded bytes violate the schema (bad kind, bad count, delta against
    /// a baseline entry that isn't there, trailing garbage).
    Malformed,
}

pub struct BitWriter<'a> {
    buf: &'a mut [u8],
    /// Position in bits.
    pos: usize,
}

impl<'a> BitWriter<'a> {
    /// Zeroes the buffer: writes OR bits in, so untouched space must be 0
    /// (this is also what makes `patch` and `rewind_to` sound).
    pub fn new(buf: &'a mut [u8]) -> Self {
        buf.fill(0);
        Self { buf, pos: 0 }
    }

    pub fn bit_pos(&self) -> usize {
        self.pos
    }

    /// Write the low `bits` bits of `val` (1..=32). Bits above `bits` must
    /// be zero — anything else is a `Range` refusal, not a silent mask.
    pub fn write(&mut self, val: u32, bits: u32) -> Result<(), WireError> {
        debug_assert!((1..=32).contains(&bits));
        if bits < 32 && (val >> bits) != 0 {
            return Err(WireError::Range);
        }
        if self.pos + bits as usize > self.buf.len() * 8 {
            return Err(WireError::Overflow);
        }
        let mut val = val;
        let mut remaining = bits as usize;
        while remaining > 0 {
            let byte = self.pos / 8;
            let bit = self.pos % 8;
            let take = (8 - bit).min(remaining);
            let mask = ((1u16 << take) - 1) as u8;
            self.buf[byte] |= ((val as u8) & mask) << bit;
            val >>= take;
            self.pos += take;
            remaining -= take;
        }
        Ok(())
    }

    pub fn write_bit(&mut self, b: bool) -> Result<(), WireError> {
        self.write(b as u32, 1)
    }

    /// OR `val` into bits previously reserved (written as zeros) at
    /// `at_bit`. For the count fields patched in `finish` — the reserved
    /// region starts 0 and is patched exactly once, so OR is exact.
    pub fn patch(&mut self, at_bit: usize, val: u32, bits: u32) -> Result<(), WireError> {
        debug_assert!(at_bit + bits as usize <= self.pos);
        let saved = self.pos;
        self.pos = at_bit;
        let r = self.write(val, bits);
        self.pos = saved;
        r
    }

    /// Roll back to an earlier position, clearing everything after it, so
    /// a partially written record vanishes (the encoder's shed path).
    pub fn rewind_to(&mut self, pos: usize) {
        debug_assert!(pos <= self.pos);
        for b in pos..self.pos {
            self.buf[b / 8] &= !(1 << (b % 8));
        }
        self.pos = pos;
    }

    /// Byte length of what was written (trailing pad bits are zero by
    /// construction).
    pub fn finish(self) -> usize {
        self.pos.div_ceil(8)
    }
}

pub struct BitReader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> BitReader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    /// Read `bits` bits (1..=32), LSB-first.
    pub fn read(&mut self, bits: u32) -> Result<u32, WireError> {
        debug_assert!((1..=32).contains(&bits));
        if self.pos + bits as usize > self.buf.len() * 8 {
            return Err(WireError::Truncated);
        }
        let mut out: u32 = 0;
        let mut got = 0usize;
        let mut remaining = bits as usize;
        while remaining > 0 {
            let byte = self.pos / 8;
            let bit = self.pos % 8;
            let take = (8 - bit).min(remaining);
            let mask = ((1u16 << take) - 1) as u8;
            let piece = (self.buf[byte] >> bit) & mask;
            out |= (piece as u32) << got;
            got += take;
            self.pos += take;
            remaining -= take;
        }
        Ok(out)
    }

    pub fn read_bit(&mut self) -> Result<bool, WireError> {
        Ok(self.read(1)? != 0)
    }

    /// Bits left unread — the strict decoders require < 8 (byte padding
    /// only) and all of them zero, so trailing garbage never passes.
    pub fn remaining_bits(&self) -> usize {
        self.buf.len() * 8 - self.pos
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_mixed_widths() {
        let mut buf = [0u8; 16];
        let mut w = BitWriter::new(&mut buf);
        w.write(0b101, 3).unwrap();
        w.write(0xBEEF, 16).unwrap();
        w.write_bit(true).unwrap();
        w.write(0x1FFFF, 17).unwrap();
        w.write(0xDEAD_BEEF, 32).unwrap();
        let len = w.finish();
        let mut r = BitReader::new(&buf[..len]);
        assert_eq!(r.read(3).unwrap(), 0b101);
        assert_eq!(r.read(16).unwrap(), 0xBEEF);
        assert!(r.read_bit().unwrap());
        assert_eq!(r.read(17).unwrap(), 0x1FFFF);
        assert_eq!(r.read(32).unwrap(), 0xDEAD_BEEF);
        assert!(r.remaining_bits() < 8);
    }

    #[test]
    fn write_refuses_out_of_range_and_overflow() {
        let mut buf = [0u8; 2];
        let mut w = BitWriter::new(&mut buf);
        assert_eq!(w.write(0b100, 2), Err(WireError::Range));
        w.write(0xFFFF, 16).unwrap();
        assert_eq!(w.write_bit(true), Err(WireError::Overflow));
    }

    #[test]
    fn read_refuses_truncation() {
        let buf = [0xFFu8; 2];
        let mut r = BitReader::new(&buf);
        r.read(10).unwrap();
        assert_eq!(r.read(7), Err(WireError::Truncated));
    }

    #[test]
    fn rewind_clears_partial_record() {
        let mut buf = [0u8; 4];
        let mut w = BitWriter::new(&mut buf);
        w.write(0b101, 3).unwrap();
        let mark = w.bit_pos();
        w.write(0x7FF, 11).unwrap();
        w.rewind_to(mark);
        w.write(0, 11).unwrap();
        let len = w.finish();
        let mut r = BitReader::new(&buf[..len]);
        assert_eq!(r.read(3).unwrap(), 0b101);
        assert_eq!(r.read(11).unwrap(), 0);
    }

    #[test]
    fn patch_fills_reserved_zeros() {
        let mut buf = [0u8; 4];
        let mut w = BitWriter::new(&mut buf);
        w.write(0b11, 2).unwrap();
        let at = w.bit_pos();
        w.write(0, 7).unwrap();
        w.write(0b1, 1).unwrap();
        w.patch(at, 64, 7).unwrap();
        let len = w.finish();
        let mut r = BitReader::new(&buf[..len]);
        assert_eq!(r.read(2).unwrap(), 0b11);
        assert_eq!(r.read(7).unwrap(), 64);
        assert_eq!(r.read(1).unwrap(), 1);
    }
}
