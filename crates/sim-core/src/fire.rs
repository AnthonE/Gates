//! Fires on the ground: what a fire arrow leaves where it comes down
//! instead of an arrow. Rust's is `fireball_small_arrow`: it burns 20 to 40
//! seconds and takes 2 health a second off anything within half a metre.
//!
//! A fire is lit by `World` from the arrow's last position, dropped onto
//! whatever a body could stand on below it (`grounditem::rest_at`), so a
//! fire arrow into a wall or a trunk burns at its foot the way Rust's falls.
//! Water puts it out before it starts. It burns players and animals alike,
//! the archer included, on the sim's one-second stride, and the kill is the
//! archer's (`DEATH_BY_ARROW`, with the bow). Wood does not catch.

use crate::limits::{MAX_FIRES, TICK_HZ};
use crate::movement::{POS_XZ_Q, POS_Y_Q};

/// How far a fire reaches from its centre, metres, to the surface of a
/// body: Rust's radius.
pub const FIRE_REACH_M: f32 = 0.5;
/// Health a fire takes off each body in reach, once a [`FIRE_PERIOD_TICKS`]
/// — Rust's two a second, before armour.
pub const FIRE_HP: u16 = 2;
/// A fire burns what is in it once a second, on the world's own stride, so
/// every fire on the island bites on the same tick.
pub const FIRE_PERIOD_TICKS: u64 = TICK_HZ as u64;

/// One fire. Sim state, hashed and saved.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FireRec {
    /// Where it burns, in body quanta (`POS_XZ_Q`, `POS_Y_Q`): the ground
    /// under where the arrow came down.
    pub qx: i32,
    pub qy: i32,
    pub qz: i32,
    /// The tick it goes out. Absolute, a charge's `fires_at` reason.
    pub until: u64,
    /// Who shot it, for the kill.
    pub owner: u32,
    /// The weapon that shot it, for the death screen.
    pub item: u16,
    /// How far the arrow flew, for the death screen's range.
    pub range_cm: u16,
}

impl FireRec {
    /// Its centre, metres.
    pub fn at_m(&self) -> (f32, f32, f32) {
        (
            self.qx as f32 * POS_XZ_Q,
            self.qy as f32 * POS_Y_Q,
            self.qz as f32 * POS_XZ_Q,
        )
    }

    /// Whether an upright cylinder — feet at `(x, y, z)` metres, radius `r`,
    /// height `h` — is in reach: its side within [`FIRE_REACH_M`] of the
    /// centre in the plane, and the centre no lower than that below its feet
    /// nor above its head.
    pub fn reaches(&self, x: f32, y: f32, z: f32, r: f32, h: f32) -> bool {
        let (fx, fy, fz) = self.at_m();
        let (dx, dz) = (x - fx, z - fz);
        let reach = FIRE_REACH_M + r;
        dx * dx + dz * dz <= reach * reach && fy >= y - FIRE_REACH_M && fy <= y + h
    }
}

/// Every fire burning on the shard, oldest first. Insertion order is a
/// tick's own order, so it is the same on every box.
#[derive(Clone, Debug)]
pub struct Fires {
    entries: [FireRec; MAX_FIRES],
    len: usize,
}

impl Default for Fires {
    fn default() -> Self {
        Self::new()
    }
}

impl Fires {
    pub const fn new() -> Self {
        Self {
            entries: [FireRec {
                qx: 0,
                qy: 0,
                qz: 0,
                until: 0,
                owner: 0,
                item: 0,
                range_cm: 0,
            }; MAX_FIRES],
            len: 0,
        }
    }

    pub fn entries(&self) -> &[FireRec] {
        &self.entries[..self.len]
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Light one. Full, the oldest goes out to make room (`MAX_FIRES`).
    pub fn light(&mut self, f: FireRec) {
        if self.len == MAX_FIRES {
            self.entries.copy_within(1..MAX_FIRES, 0);
            self.len -= 1;
        }
        self.entries[self.len] = f;
        self.len += 1;
    }

    /// Put out every fire whose time has come, keeping the rest in order.
    pub fn expire_due(&mut self, tick: u64) {
        let mut kept = 0;
        for i in 0..self.len {
            if self.entries[i].until > tick {
                self.entries[kept] = self.entries[i];
                kept += 1;
            }
        }
        self.len = kept;
    }

    /// Replace the store with a saved one (`worldsave.rs`), at most
    /// `MAX_FIRES` of it.
    pub fn restore(&mut self, fires: &[FireRec]) {
        let n = fires.len().min(MAX_FIRES);
        self.entries[..n].copy_from_slice(&fires[..n]);
        self.len = n;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(x: i32, until: u64) -> FireRec {
        FireRec {
            qx: x,
            until,
            ..FireRec::default()
        }
    }

    #[test]
    fn a_full_store_puts_the_oldest_out_and_expiry_keeps_the_order() {
        let mut f = Fires::new();
        for i in 0..MAX_FIRES as i32 + 1 {
            f.light(at(i, 100 + i as u64));
        }
        assert_eq!(f.len(), MAX_FIRES);
        assert_eq!(f.entries()[0].qx, 1, "the first fire went out");
        f.expire_due(110);
        assert_eq!(f.entries()[0].qx, 11);
        assert!(f.entries().windows(2).all(|w| w[0].qx < w[1].qx));
    }

    #[test]
    fn a_fire_reaches_half_a_metre_past_a_body_and_not_over_its_head() {
        // At (3 m, 1 m, 3 m).
        let f = FireRec {
            qx: 100,
            qy: 100,
            qz: 100,
            ..FireRec::default()
        };
        let (r, h) = (0.4, 1.8);
        assert!(f.reaches(3.0, 1.0, 3.0, r, h));
        assert!(f.reaches(3.85, 1.0, 3.0, r, h));
        assert!(!f.reaches(3.95, 1.0, 3.0, r, h));
        // Standing on a floor 2 m above it.
        assert!(!f.reaches(3.0, 3.0, 3.0, r, h));
        // On a ledge 2 m below it, the fire above the head.
        assert!(!f.reaches(3.0, -1.0, 3.0, r, h));
    }
}
