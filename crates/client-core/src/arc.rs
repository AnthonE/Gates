//! The arc as this client knows it (`ARC.md`, wire v95): every work's row
//! and names as dripped at join, each work's state as the server last sent
//! it, and the unlocks the island holds.
//!
//! Pure tables. The panels and the HUD read them (`crate::core::ClientCore::
//! arc`); nothing here decides anything the sim has not already decided.

use protocol::{UNLOCK_NAME_BYTES, WORK_NAME_BYTES};
use sim_core::limits::{MAX_WORKS, MAX_WORK_INPUTS};
use sim_core::works::{WorkDef, WORK_SEALED};

/// One work: its row (once `known`), its names, and its state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WorkView {
    /// The row has arrived.
    pub known: bool,
    pub def: WorkDef,
    pub name: [u8; WORK_NAME_BYTES],
    pub name_len: u8,
    pub floor_name: [u8; UNLOCK_NAME_BYTES],
    pub floor_len: u8,
    pub ceiling_name: [u8; UNLOCK_NAME_BYTES],
    pub ceiling_len: u8,
    /// `sim_core::works::WORK_*`.
    pub state: u8,
    pub fuel: u32,
    pub got: [u32; MAX_WORK_INPUTS],
    /// This player's share, basis points of a quota.
    pub mine: u32,
}

impl WorkView {
    pub fn name(&self) -> &str {
        core::str::from_utf8(&self.name[..self.name_len as usize]).unwrap_or("?")
    }

    pub fn floor_name(&self) -> &str {
        core::str::from_utf8(&self.floor_name[..self.floor_len as usize]).unwrap_or("")
    }

    pub fn ceiling_name(&self) -> &str {
        core::str::from_utf8(&self.ceiling_name[..self.ceiling_len as usize]).unwrap_or("")
    }

    /// The quota met so far, per mille across every line (each line weighs
    /// the same, the way credit does).
    pub fn progress_pm(&self) -> u32 {
        let n = self.def.n_inputs as usize;
        if n == 0 {
            return 0;
        }
        let mut sum = 0u64;
        for (i, g) in self.def.inputs.iter().zip(self.got.iter()).take(n) {
            sum += (*g as u64).min(i.need as u64) * 1000 / i.need.max(1) as u64;
        }
        (sum / n as u64) as u32
    }
}

/// Every work, the unlocks held, and a generation that moves whenever any
/// of it does (what a panel's change detection compares).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArcView {
    /// Works on the shard, as the rows say.
    pub count: u8,
    pub works: [WorkView; MAX_WORKS],
    pub unlocks: u32,
    pub gen: u32,
}

impl Default for ArcView {
    fn default() -> Self {
        ArcView {
            count: 0,
            works: [WorkView::default(); MAX_WORKS],
            unlocks: 0,
            gen: 0,
        }
    }
}

impl ArcView {
    /// The works whose rows have arrived, with their index.
    pub fn known(&self) -> impl Iterator<Item = (usize, &WorkView)> {
        self.works
            .iter()
            .enumerate()
            .take(self.count as usize)
            .filter(|(_, w)| w.known)
    }

    /// The act the shard is in: the highest act of any work no longer
    /// sealed, and act I before one is (`sim_core::works::Works::act`).
    pub fn act(&self) -> u8 {
        self.known()
            .filter(|(_, w)| w.state != WORK_SEALED)
            .map(|(_, w)| w.def.act)
            .max()
            .unwrap_or(1)
            .max(1)
    }

    /// Whether unlock code `u` is held (`sim_core::works::holds`).
    pub fn holds(&self, u: u8) -> bool {
        sim_core::works::holds(self.unlocks, u)
    }

    /// The work that grants unlock code `u`, floor or ceiling, if its row is
    /// here: what a locked recipe or offer names as the thing to light.
    pub fn granter(&self, u: u8) -> Option<(usize, &WorkView)> {
        if u == sim_core::works::NO_UNLOCK {
            return None;
        }
        self.known()
            .find(|(_, w)| w.def.floor == u || w.def.ceiling == u)
    }
}

/// A small drop-oldest ring, the own-fact rings' posture.
#[derive(Clone, Copy, Debug)]
pub struct Ring<T: Copy + Default, const N: usize> {
    items: [T; N],
    head: usize,
    len: usize,
}

impl<T: Copy + Default, const N: usize> Default for Ring<T, N> {
    fn default() -> Self {
        Ring {
            items: [T::default(); N],
            head: 0,
            len: 0,
        }
    }
}

impl<T: Copy + Default, const N: usize> Ring<T, N> {
    pub fn push(&mut self, v: T) {
        if self.len == N {
            self.head = (self.head + 1) % N;
            self.len -= 1;
        }
        self.items[(self.head + self.len) % N] = v;
        self.len += 1;
    }

    pub fn pop(&mut self) -> Option<T> {
        if self.len == 0 {
            return None;
        }
        let v = self.items[self.head];
        self.head = (self.head + 1) % N;
        self.len -= 1;
        Some(v)
    }
}

/// One speaker: where they stand, their name and topic titles, and the last
/// thing they said to this player (`topic`, the words).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpeakerView {
    pub known: bool,
    pub spot: sim_core::spot::Spot,
    pub name: [u8; protocol::ARC_NAME_BYTES],
    pub name_len: u8,
    pub n_topics: u8,
    pub topics: [[u8; protocol::ARC_NAME_BYTES]; sim_core::lore::MAX_TOPICS],
    pub topic_lens: [u8; sim_core::lore::MAX_TOPICS],
    pub said_topic: u8,
    pub said: [u8; protocol::ARC_TEXT_BYTES],
    pub said_len: u16,
}

/// One inscription: where it stands, and its text once this player read it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InscriptionView {
    pub known: bool,
    pub spot: sim_core::spot::Spot,
    pub text: [u8; protocol::ARC_TEXT_BYTES],
    pub len: u16,
}

/// One mechanism: where it stands, its name, and its dials now.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MechView {
    pub known: bool,
    pub spot: sim_core::spot::Spot,
    pub name: [u8; protocol::ARC_NAME_BYTES],
    pub name_len: u8,
    pub n_dials: u8,
    pub values: u8,
    pub dials: [u8; sim_core::limits::MAX_DIALS],
    pub resting: bool,
}

impl Default for SpeakerView {
    fn default() -> Self {
        SpeakerView {
            known: false,
            spot: Default::default(),
            name: [0; protocol::ARC_NAME_BYTES],
            name_len: 0,
            n_topics: 0,
            topics: [[0; protocol::ARC_NAME_BYTES]; sim_core::lore::MAX_TOPICS],
            topic_lens: [0; sim_core::lore::MAX_TOPICS],
            said_topic: 0,
            said: [0; protocol::ARC_TEXT_BYTES],
            said_len: 0,
        }
    }
}

impl Default for InscriptionView {
    fn default() -> Self {
        InscriptionView {
            known: false,
            spot: Default::default(),
            text: [0; protocol::ARC_TEXT_BYTES],
            len: 0,
        }
    }
}

fn utf8(b: &[u8]) -> &str {
    core::str::from_utf8(b).unwrap_or("")
}

impl SpeakerView {
    pub fn name(&self) -> &str {
        utf8(&self.name[..self.name_len as usize])
    }
    pub fn topic(&self, t: usize) -> &str {
        match self.topics.get(t) {
            Some(b) => utf8(&b[..self.topic_lens[t] as usize]),
            None => "",
        }
    }
    pub fn said(&self) -> &str {
        utf8(&self.said[..self.said_len as usize])
    }
}

impl InscriptionView {
    pub fn text(&self) -> &str {
        utf8(&self.text[..self.len as usize])
    }
}

impl MechView {
    pub fn name(&self) -> &str {
        utf8(&self.name[..self.name_len as usize])
    }
}

/// The people, the stones and the locks, the ancients' alphabet, and the
/// glyphs this player reads (wire v96). Boxed on the core: ~14 kB.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoreView {
    pub speakers: [SpeakerView; sim_core::limits::MAX_SPEAKERS],
    pub n_speakers: u8,
    pub inscriptions: [InscriptionView; sim_core::limits::MAX_INSCRIPTIONS],
    pub n_inscriptions: u8,
    pub mechs: [MechView; sim_core::limits::MAX_MECHS],
    pub n_mechs: u8,
    pub alphabet: [u8; sim_core::limits::MAX_GLYPHS],
    pub alphabet_len: u8,
    pub glyphs: u64,
    pub gen: u32,
}

impl Default for LoreView {
    fn default() -> Self {
        LoreView {
            speakers: [SpeakerView::default(); sim_core::limits::MAX_SPEAKERS],
            n_speakers: 0,
            inscriptions: [InscriptionView::default(); sim_core::limits::MAX_INSCRIPTIONS],
            n_inscriptions: 0,
            mechs: [MechView::default(); sim_core::limits::MAX_MECHS],
            n_mechs: 0,
            alphabet: [0; sim_core::limits::MAX_GLYPHS],
            alphabet_len: 0,
            glyphs: 0,
            gen: 0,
        }
    }
}

impl LoreView {
    /// The alphabet as the server sent it.
    pub fn alphabet(&self) -> &[u8] {
        &self.alphabet[..self.alphabet_len as usize]
    }

    /// Glyph `ch`'s place in the alphabet, if it is one.
    pub fn glyph_of(&self, ch: u8) -> Option<usize> {
        self.alphabet().iter().position(|&a| a == ch)
    }

    /// Whether this player reads character `ch` (a space always reads).
    pub fn reads(&self, ch: u8) -> bool {
        ch == b' '
            || self
                .glyph_of(ch)
                .is_some_and(|g| self.glyphs & (1 << g) != 0)
    }

    pub fn known_speakers(&self) -> impl Iterator<Item = (usize, &SpeakerView)> {
        self.speakers
            .iter()
            .enumerate()
            .take(self.n_speakers as usize)
            .filter(|(_, s)| s.known)
    }

    pub fn known_inscriptions(&self) -> impl Iterator<Item = (usize, &InscriptionView)> {
        self.inscriptions
            .iter()
            .enumerate()
            .take(self.n_inscriptions as usize)
            .filter(|(_, s)| s.known)
    }

    pub fn known_mechs(&self) -> impl Iterator<Item = (usize, &MechView)> {
        self.mechs
            .iter()
            .enumerate()
            .take(self.n_mechs as usize)
            .filter(|(_, s)| s.known)
    }
}
