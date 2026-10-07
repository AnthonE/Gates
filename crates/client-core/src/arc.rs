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
