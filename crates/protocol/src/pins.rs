//! Map marks shared with a team (wire v157, Rust's team markers): a
//! player's whole set of marks, as the action lane carries it to the shard
//! (`ActionMsg::Pins`) and the event lane carries it on to each teammate
//! (`EventMsg::TeamPins`).
//!
//! A set goes whole every time it changes, never as a diff, so the next one
//! repairs a lost or late one. The shard keeps none (v0 is a relay), so a
//! mate who joins sees your marks when your client sees them join and sends
//! its set again (`client::ui::map::PinShare`).

use crate::bits::{BitReader, BitWriter, WireError};

/// How many marks a player can put on their map (Rust's five), and so how
/// many one set carries.
pub const PINS_MAX: usize = 5;
/// How many colours a mark can be, by index (the client's palette,
/// `ui::map::PIN_COLOURS`).
pub const PIN_COLOURS: u8 = 6;
/// How many pictures a mark can have, by index, 0 being its number
/// (`ui::map::PIN_ICONS`).
pub const PIN_ICONS: u8 = 6;
/// A mark's label, characters.
pub const PIN_LABEL_MAX: usize = 10;
/// A mark's position on each axis, whole metres, as a team member's is: the
/// island is 4096 m across, so 12 bits hold all of it.
const PIN_POS_BITS: u32 = 12;
/// The largest coordinate that fits.
pub const PIN_POS_MAX_M: u16 = (1 << PIN_POS_BITS) - 1;
const PIN_COUNT_BITS: u32 = 3;
const PIN_COLOUR_BITS: u32 = 3;
const PIN_ICON_BITS: u32 = 3;
const PIN_LABEL_LEN_BITS: u32 = 4;
/// A label character: printable ASCII fits seven bits.
const PIN_CHAR_BITS: u32 = 7;
const _: () = assert!(PINS_MAX < 1 << PIN_COUNT_BITS);
const _: () = assert!(PIN_COLOURS as u32 <= 1 << PIN_COLOUR_BITS);
const _: () = assert!(PIN_ICONS as u32 <= 1 << PIN_ICON_BITS);
const _: () = assert!(PIN_LABEL_MAX < 1 << PIN_LABEL_LEN_BITS);
const _: () = assert!(sim_core::terrain::ISLAND_SIZE as u32 <= 1 << PIN_POS_BITS);

/// A byte a label may hold: printable ASCII, the space included.
pub fn pin_label_byte(b: u8) -> bool {
    (0x20..=0x7E).contains(&b)
}

/// One mark: where, in whole metres of world XZ, its colour and picture as
/// indexes, and its label. The label is trimmed (no space at either end)
/// and its unused tail is zero, so two marks that look alike are equal.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WirePin {
    pub x_m: u16,
    pub z_m: u16,
    pub colour: u8,
    pub icon: u8,
    pub label_len: u8,
    pub label: [u8; PIN_LABEL_MAX],
}

impl WirePin {
    /// A mark with `label`, or `None` when the label is longer than
    /// [`PIN_LABEL_MAX`] or holds a byte the wire refuses. The label is
    /// trimmed first; colour, icon and position are checked by
    /// [`Self::valid`] when the set is encoded.
    pub fn new(x_m: u16, z_m: u16, colour: u8, icon: u8, label: &str) -> Option<Self> {
        let l = label.trim_matches(' ').as_bytes();
        if l.len() > PIN_LABEL_MAX || !l.iter().all(|&b| pin_label_byte(b)) {
            return None;
        }
        let mut pin = Self {
            x_m,
            z_m,
            colour,
            icon,
            label_len: l.len() as u8,
            ..Self::default()
        };
        pin.label[..l.len()].copy_from_slice(l);
        Some(pin)
    }

    /// The label, empty for none.
    pub fn label(&self) -> &str {
        let n = (self.label_len as usize).min(PIN_LABEL_MAX);
        std::str::from_utf8(&self.label[..n]).unwrap_or("")
    }

    /// Every field in range, the label printable ASCII with no space at
    /// either end, and its tail zero.
    pub fn valid(&self) -> bool {
        let n = self.label_len as usize;
        if self.x_m > PIN_POS_MAX_M
            || self.z_m > PIN_POS_MAX_M
            || self.colour >= PIN_COLOURS
            || self.icon >= PIN_ICONS
            || n > PIN_LABEL_MAX
        {
            return false;
        }
        let (used, tail) = self.label.split_at(n);
        used.iter().all(|&b| pin_label_byte(b))
            && used.first() != Some(&b' ')
            && used.last() != Some(&b' ')
            && tail.iter().all(|&b| b == 0)
    }

    fn write(&self, w: &mut BitWriter) -> Result<(), WireError> {
        w.write(self.x_m as u32, PIN_POS_BITS)?;
        w.write(self.z_m as u32, PIN_POS_BITS)?;
        w.write(self.colour as u32, PIN_COLOUR_BITS)?;
        w.write(self.icon as u32, PIN_ICON_BITS)?;
        w.write(self.label_len as u32, PIN_LABEL_LEN_BITS)?;
        for &b in &self.label[..self.label_len as usize] {
            w.write(b as u32, PIN_CHAR_BITS)?;
        }
        Ok(())
    }

    fn read(r: &mut BitReader) -> Result<Self, WireError> {
        let mut pin = Self {
            x_m: r.read(PIN_POS_BITS)? as u16,
            z_m: r.read(PIN_POS_BITS)? as u16,
            colour: r.read(PIN_COLOUR_BITS)? as u8,
            icon: r.read(PIN_ICON_BITS)? as u8,
            label_len: r.read(PIN_LABEL_LEN_BITS)? as u8,
            ..Self::default()
        };
        if pin.label_len as usize > PIN_LABEL_MAX {
            return Err(WireError::Malformed);
        }
        for b in pin.label.iter_mut().take(pin.label_len as usize) {
            *b = r.read(PIN_CHAR_BITS)? as u8;
        }
        if !pin.valid() {
            return Err(WireError::Malformed);
        }
        Ok(pin)
    }
}

/// A player's whole set of marks, oldest first. `n == 0` is none, which is
/// how a set is cleared on a teammate's map. The tail past `n` is zero.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PinSet {
    pub n: u8,
    pub pins: [WirePin; PINS_MAX],
}

impl PinSet {
    pub const NONE: Self = Self {
        n: 0,
        pins: [WirePin {
            x_m: 0,
            z_m: 0,
            colour: 0,
            icon: 0,
            label_len: 0,
            label: [0; PIN_LABEL_MAX],
        }; PINS_MAX],
    };

    /// The marks in the set.
    pub fn pins(&self) -> &[WirePin] {
        &self.pins[..(self.n as usize).min(PINS_MAX)]
    }

    /// Add a mark at the end. False, and nothing added, when the set is full.
    pub fn push(&mut self, pin: WirePin) -> bool {
        let n = self.n as usize;
        if n >= PINS_MAX {
            return false;
        }
        self.pins[n] = pin;
        self.n += 1;
        true
    }

    /// No more than [`PINS_MAX`], every mark [`WirePin::valid`], and a
    /// zeroed tail.
    pub fn valid(&self) -> bool {
        let n = self.n as usize;
        n <= PINS_MAX
            && self.pins[..n].iter().all(WirePin::valid)
            && self.pins[n..].iter().all(|p| *p == WirePin::default())
    }

    pub(crate) fn write(&self, w: &mut BitWriter) -> Result<(), WireError> {
        if !self.valid() {
            return Err(WireError::Range);
        }
        w.write(self.n as u32, PIN_COUNT_BITS)?;
        for p in self.pins() {
            p.write(w)?;
        }
        Ok(())
    }

    pub(crate) fn read(r: &mut BitReader) -> Result<Self, WireError> {
        let mut set = Self::NONE;
        set.n = r.read(PIN_COUNT_BITS)? as u8;
        if set.n as usize > PINS_MAX {
            return Err(WireError::Malformed);
        }
        for p in set.pins.iter_mut().take(set.n as usize) {
            *p = WirePin::read(r)?;
        }
        Ok(set)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A mark's label is trimmed, and one too long or with a byte past
    /// printable ASCII is no mark.
    #[test]
    fn a_label_is_trimmed_printable_ascii() {
        let p = WirePin::new(1, 2, 0, 0, " RAID ").unwrap();
        assert_eq!(p.label(), "RAID");
        assert!(p.valid());
        assert!(WirePin::new(1, 2, 0, 0, "ELEVENCHARS").is_none());
        assert!(WirePin::new(1, 2, 0, 0, "caf\u{e9}").is_none());
        assert!(WirePin::new(1, 2, 0, 0, "TAB\t").is_none());
        let full = WirePin::new(1, 2, 0, 0, "0123456789").unwrap();
        assert_eq!(full.label(), "0123456789");
    }

    /// Out-of-range fields, a space at an end and a dirty tail are not
    /// valid, and a set past its cap takes no more.
    #[test]
    fn a_bad_mark_or_set_is_not_valid() {
        let ok = WirePin::new(PIN_POS_MAX_M, 0, PIN_COLOURS - 1, PIN_ICONS - 1, "A B").unwrap();
        assert!(ok.valid());
        for bad in [
            WirePin {
                x_m: PIN_POS_MAX_M + 1,
                ..ok
            },
            WirePin {
                colour: PIN_COLOURS,
                ..ok
            },
            WirePin {
                icon: PIN_ICONS,
                ..ok
            },
            WirePin {
                label_len: PIN_LABEL_MAX as u8 + 1,
                ..ok
            },
            WirePin { label_len: 2, ..ok },
        ] {
            assert!(!bad.valid(), "{bad:?}");
        }
        let mut lead = ok;
        lead.label[0] = b' ';
        assert!(!lead.valid(), "a leading space");
        let mut set = PinSet::NONE;
        for _ in 0..PINS_MAX {
            assert!(set.push(ok));
        }
        assert!(!set.push(ok), "five at most");
        assert!(set.valid());
        let mut dirty = PinSet::NONE;
        dirty.pins[0] = ok;
        assert!(!dirty.valid(), "a mark past n");
    }
}
