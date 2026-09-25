//! Skins: the look an item wears, and the thing the house sells
//! (`BUSINESS.md`). The shape is Rust's, where Steam holds what a player
//! owns and the game only applies it:
//!
//! - **An item instance carries its skin** (`ItemStack::skin`, Rust's
//!   `Item.skin`). The skin rides the item through every move, box, bag,
//!   death and save, so a looted skinned rifle keeps its look in the
//!   looter's hands. Nobody's ownership is checked to hold or use it.
//! - **Ownership is checked where a skin goes on, and only there**: a craft
//!   that names one (`craft::enqueue`) and a re-skin at a bench
//!   ([`reskin`], Rust's repair bench). Taking a skin off needs no
//!   ownership.
//! - **The owned set comes from outside the sim.** Steam's inventory there,
//!   the platform's item contract here (`server/src/skins.rs` reads it).
//!   It enters as `Command::SkinsOwned`, minted by the server alone, so a
//!   replayed stream reproduces what the session knew. It is session state:
//!   a new connection starts with none until the server says otherwise.
//! - **Appearance only.** A row names a catalog id and the item it fits.
//!   There is nothing else in it to sell (`CONTENT.md` §6).

use crate::craft::{REFUSE_SKIN, REFUSE_STATION, STATION_RADIUS_M, STATION_WORKBENCH1};
use crate::deploy::{DeployContent, Deploys};
use crate::limits::{INV_SLOTS, MAX_SKINS, SKIN_WORDS};
use crate::world::{EventQueue, Player, EV_CRAFT_REFUSED};

/// `ItemStack::skin` for an item wearing its own look.
pub const NO_SKIN: u16 = 0;

/// One baked skin row.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SkinDef {
    /// The id an `ItemStack` carries: the skin's `catalogId` on the
    /// platform's item contract. Never 0, unique across the table.
    pub catalog: u16,
    /// The item row this skin fits.
    pub covers: u16,
}

/// The baked catalog, in `content/skins.toml` order. Row indices are what
/// a [`SkinSet`] is keyed by; catalog ids are what items carry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SkinContent {
    pub defs: [SkinDef; MAX_SKINS],
    pub count: u16,
}

impl Default for SkinContent {
    fn default() -> Self {
        Self::EMPTY
    }
}

impl SkinContent {
    pub const EMPTY: Self = Self {
        defs: [SkinDef {
            catalog: 0,
            covers: 0,
        }; MAX_SKINS],
        count: 0,
    };

    /// The live rows.
    pub fn rows(&self) -> &[SkinDef] {
        &self.defs[..(self.count as usize).min(MAX_SKINS)]
    }

    /// The row carrying `catalog`, if this content knows it. A linear scan
    /// over at most `MAX_SKINS` rows, and only on the two verbs that put a
    /// skin on — never per tick.
    pub fn row_of(&self, catalog: u16) -> Option<usize> {
        if catalog == NO_SKIN {
            return None;
        }
        self.rows().iter().position(|d| d.catalog == catalog)
    }

    /// May a player owning `owned` put skin `catalog` on `item`? The skin
    /// has to exist, fit that item, and be theirs.
    pub fn may_wear(&self, owned: &SkinSet, catalog: u16, item: u16) -> bool {
        match self.row_of(catalog) {
            Some(row) => self.defs[row].covers == item && owned.has(row),
            None => false,
        }
    }
}

/// Which catalog rows one player owns, as a bitset over row indices.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SkinSet(pub [u64; SKIN_WORDS]);

impl SkinSet {
    pub const EMPTY: Self = Self([0; SKIN_WORDS]);

    /// Every row below `count`, the dev shard's "own everything".
    pub fn all(count: usize) -> Self {
        let mut s = Self::EMPTY;
        for row in 0..count.min(MAX_SKINS) {
            s.insert(row);
        }
        s
    }

    pub fn has(&self, row: usize) -> bool {
        row < MAX_SKINS && self.0[row / 64] & (1 << (row % 64)) != 0
    }

    /// Out of range is ignored rather than wrapped: a row past the table is
    /// a skin nobody can wear, and setting some other bit instead would be
    /// handing out a different one.
    pub fn insert(&mut self, row: usize) {
        if row < MAX_SKINS {
            self.0[row / 64] |= 1 << (row % 64);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.0.iter().all(|w| *w == 0)
    }

    pub fn count(&self) -> u32 {
        self.0.iter().map(|w| w.count_ones()).sum()
    }

    /// Little-endian words, for `state_hash` and the world save.
    pub fn to_le_bytes(&self) -> [u8; SKIN_WORDS * 8] {
        let mut out = [0u8; SKIN_WORDS * 8];
        for (i, w) in self.0.iter().enumerate() {
            out[i * 8..i * 8 + 8].copy_from_slice(&w.to_le_bytes());
        }
        out
    }

    pub fn from_le_bytes(b: &[u8; SKIN_WORDS * 8]) -> Self {
        let mut s = Self::EMPTY;
        for (i, w) in s.0.iter_mut().enumerate() {
            let mut word = [0u8; 8];
            word.copy_from_slice(&b[i * 8..i * 8 + 8]);
            *w = u64::from_le_bytes(word);
        }
        s
    }
}

/// Put skin `skin` on the item in inventory `slot`, or take its skin off
/// with [`NO_SKIN`] (`Command::Reskin`). Rust does this at a repair bench;
/// we have none, so any workbench in reach is the station.
///
/// The order is craft's (`craft::enqueue` checks the blueprint before the
/// station): the skin first, because not owning it is the refusal a walk
/// to a bench cannot fix. Refusals ride `EV_CRAFT_REFUSED` with craft's
/// reason codes. A success says nothing of its own; the slot's new skin
/// reaches the client in the next inventory diff.
pub fn reskin(
    sc: &SkinContent,
    dc: &DeployContent,
    deploys: &Deploys,
    p: &mut Player,
    slot: u8,
    skin: u16,
    events: &mut EventQueue,
) {
    let s = slot as usize;
    if s >= INV_SLOTS || p.inv[s].count == 0 {
        events.push(EV_CRAFT_REFUSED, p.id, REFUSE_SKIN, 0);
        return;
    }
    let item = p.inv[s].item;
    if skin != NO_SKIN && !sc.may_wear(&p.skins, skin, item) {
        events.push(EV_CRAFT_REFUSED, p.id, REFUSE_SKIN, 0);
        return;
    }
    let px = p.body.qx as f32 * crate::movement::POS_XZ_Q;
    let pz = p.body.qz as f32 * crate::movement::POS_XZ_Q;
    if !deploys.bench_near(dc, STATION_WORKBENCH1, px, pz, STATION_RADIUS_M) {
        events.push(EV_CRAFT_REFUSED, p.id, REFUSE_STATION, 0);
        return;
    }
    p.inv[s].skin = skin;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn content() -> SkinContent {
        let mut sc = SkinContent::EMPTY;
        sc.defs[0] = SkinDef {
            catalog: 7,
            covers: 3,
        };
        sc.defs[1] = SkinDef {
            catalog: 900,
            covers: 4,
        };
        sc.count = 2;
        sc
    }

    #[test]
    fn a_skin_goes_on_only_the_item_it_fits_and_only_if_owned() {
        let sc = content();
        let mut owned = SkinSet::EMPTY;
        assert!(!sc.may_wear(&owned, 7, 3), "not owned");
        owned.insert(0);
        assert!(sc.may_wear(&owned, 7, 3));
        assert!(!sc.may_wear(&owned, 7, 4), "wrong item");
        assert!(!sc.may_wear(&owned, 900, 4), "row 1 is not owned");
        assert!(!sc.may_wear(&owned, 8, 3), "no such catalog id");
        assert!(!sc.may_wear(&owned, NO_SKIN, 3), "0 is not a skin");
    }

    #[test]
    fn the_set_round_trips_and_ignores_rows_past_the_table() {
        let mut s = SkinSet::EMPTY;
        s.insert(0);
        s.insert(63);
        s.insert(64);
        s.insert(MAX_SKINS - 1);
        s.insert(MAX_SKINS);
        assert_eq!(s.count(), 4);
        assert!(s.has(63) && s.has(64) && !s.has(65) && !s.has(MAX_SKINS));
        assert_eq!(SkinSet::from_le_bytes(&s.to_le_bytes()), s);
        assert_eq!(SkinSet::all(3).count(), 3);
        assert!(SkinSet::EMPTY.is_empty());
    }
}
