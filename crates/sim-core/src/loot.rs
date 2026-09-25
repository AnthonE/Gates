//! Loot — the weighted container roll (CONTENT.md §5, DESIGN.md §2's
//! "the loot tension source"). A smashed barrel does not pay into the
//! smasher's inventory; it rolls its table into a container standing where
//! the barrel stood, and the smasher loots that. The road pulls players
//! out of their bases because the barrels along it are worth the walk, and
//! a container you must stand over to empty is the moment that costs.
//!
//! Content reaches the sim only as a baked `LootContent` — the shard bakes
//! it from `content/loot.toml` at boot (CLAUDE.md wall 7); the inert
//! `EMPTY` default rolls nothing, and `probe_fixture()` is a synthetic
//! table for the parity/replay/alloc gates.
//!
//! Pure and fixed-capacity like the rest of the crate: the roll is integer
//! arithmetic over `splitmix64`, no float touches it at all, and it writes
//! into a caller-owned array through `gather::inv_add` — the same adder a
//! player's inventory uses, so a loot container stacks and overflows by
//! exactly the rules an inventory does.

use crate::gather::{inv_add, GatherContent, ItemStack, NO_ITEM};
use crate::limits::{
    INV_SLOTS, MAX_ITEM_DEFS, MAX_LOOT_ENTRIES, MAX_LOOT_GUARANTEED, MAX_LOOT_TABLES,
};
use crate::rng::{cell_hash, splitmix64};

/// Noise channel for the loot roll. Sim-side, like gather's 97/98
/// (worldgen channels live in terrain.rs and stay below 96).
const CH_LOOT: u32 = 99;

/// Where a guaranteed row's count draw salts from. The weighted loop salts
/// its two draws per roll with `2n+1` and `2n+2` for `n` below
/// `MAX_LOOT_ROLLS`, so any salt from here up can never collide with one —
/// and a FIXED row draws nothing at all, which is what lets shipped
/// content take the column without moving any other roll.
const GUARANTEED_SALT: u64 = 0x8000;

/// Baked table index for `container = "barrel"`. The container *name* is
/// content and the *index* is code, exactly as `deploy.rs`'s `ARCH_*` maps
/// an archetype string to a row: content may re-price a barrel, it may not
/// invent a container the sim has no verb for.
pub const LOOT_BARREL: usize = 0;
/// Baked table index for `container = "crate"` — the haven pad's container
/// (`terrain::Occupant::CrateSlot`). **Opened by `worldcont.rs`** since
/// world containers v0; this line said "No verb opens one yet" for the
/// whole time the pad stood there paying nobody, which is what the
/// merge-gate judge eventually ranked first.
pub const LOOT_CRATE: usize = 1;
/// Baked table index for `container = "cache"` — the waystations', the tier
/// between the road's barrel and the pad's crate
/// (`terrain::Occupant::CacheSlot`). It exists as its own index because the
/// container's KIND is the only thing a table is selected by: while the
/// lesser tier placed `CrateSlot`, it paid the destination's table and the
/// gradient was geometry alone. `ci/haven_prize.mjs` gates the ordering.
pub const LOOT_CACHE: usize = 2;

/// One weighted row. Counts are inclusive bounds.
#[derive(Clone, Copy, Debug)]
pub struct LootEntryDef {
    pub item: u16,
    pub weight: u16,
    pub count_min: u16,
    pub count_max: u16,
}

impl LootEntryDef {
    pub const EMPTY: Self = Self {
        item: NO_ITEM,
        weight: 0,
        count_min: 0,
        count_max: 0,
    };
}

/// One container archetype's baked table. `len == 0` ⇒ inert (the roll is
/// a no-op), which is what an unarmed content set and `EMPTY` both look
/// like.
#[derive(Clone, Copy, Debug)]
pub struct LootTableDef {
    pub entries: [LootEntryDef; MAX_LOOT_ENTRIES],
    /// Sum of `entries[..len].weight`. Baked once so the roll never sums
    /// a table it is about to index — one multiply-shift, then one walk.
    pub total_weight: u32,
    pub len: u16,
    /// Rolls per open, inclusive bounds.
    pub rolls_min: u16,
    pub rolls_max: u16,
    /// Swings to smash the container that holds this table. Content
    /// (`content/loot.toml`), never a literal here — DECISIONS.md §open
    /// "barrel smash hits".
    pub hits: u16,
    /// Rows every open pays after the weighted draw, `count_min..=count_max`
    /// of each, `weight` unused (loot guaranteed column v0). The reference's
    /// container ladder is denominated in exactly this: a barrel's 2 scrap
    /// is not a roll, it is a certainty, and a weighted row cannot say so.
    pub guaranteed: [LootEntryDef; MAX_LOOT_GUARANTEED],
    pub guaranteed_len: u16,
}

impl LootTableDef {
    pub const INERT: Self = Self {
        entries: [LootEntryDef::EMPTY; MAX_LOOT_ENTRIES],
        total_weight: 0,
        len: 0,
        rolls_min: 0,
        rolls_max: 0,
        hits: 0,
        guaranteed: [LootEntryDef::EMPTY; MAX_LOOT_GUARANTEED],
        guaranteed_len: 0,
    };
}

/// Every loot table the sim knows. Construction input like the seed.
#[derive(Clone, Copy, Debug)]
pub struct LootContent {
    pub tables: [LootTableDef; MAX_LOOT_TABLES],
}

impl LootContent {
    /// Inert: every container rolls nothing. `World::new` starts here; the
    /// boot path installs the baked table before the first tick.
    pub const EMPTY: Self = Self {
        tables: [LootTableDef::INERT; MAX_LOOT_TABLES],
    };

    /// The table for a container index, or `None` when the index is past
    /// the store or the table is inert. Every caller needs both tests, so
    /// they are one function — the same posture `stack_max_of` takes.
    #[inline]
    pub fn table(&self, which: usize) -> Option<&LootTableDef> {
        if which >= MAX_LOOT_TABLES {
            return None;
        }
        let t = &self.tables[which];
        (t.len > 0 && t.total_weight > 0 && t.rolls_min > 0).then_some(t)
    }

    /// Swings to smash the container holding table `which`; 0 when the
    /// table is inert, which the caller reads as "not smashable".
    #[inline]
    pub fn hits(&self, which: usize) -> u16 {
        self.table(which).map_or(0, |t| t.hits)
    }

    /// Roll `which` into `out`, addressed by the cell key that spawned it.
    /// Returns the number of distinct units written — zero means the table
    /// was inert or nothing fit, and the caller decides whether an empty
    /// container is worth standing up (it is not).
    ///
    /// Determinism: every draw comes from `splitmix64` over the cell hash
    /// mixed with `tick`, so the same WAL replays the same barrel into the
    /// same container, while a barrel smashed again after its respawn
    /// pays a different roll — the cell alone would make one barrel pay
    /// the same thing forever.
    ///
    /// The weighted pick is Lemire's multiply-shift (`rng::next_bounded`'s
    /// trick) over the baked `total_weight`, so there is no modulo bias
    /// against the low-weight rows — and the revolver is weight 1 of 91.
    /// Bias there is not cosmetic: it is the rarest thing on the island.
    pub fn roll_into(
        &self,
        which: usize,
        gc: &GatherContent,
        seed: u64,
        cell: u32,
        tick: u64,
        out: &mut [ItemStack; INV_SLOTS],
    ) -> u16 {
        let Some(t) = self.table(which) else {
            return 0;
        };
        // Addressed by the same `cell_key` the slot event carries
        // (`gather::cell_key`, `EV_SLOT_HARVESTED.a`), so "which barrel
        // paid this" is one value everywhere rather than a pair here and a
        // key there.
        let (cx, cz) = ((cell >> 16) as i32, (cell & 0xFFFF) as i32);
        let base = cell_hash(seed, cx, cz, CH_LOOT) ^ tick;
        let span = (t.rolls_max.max(t.rolls_min) - t.rolls_min) as u64 + 1;
        let rolls = t.rolls_min as u64 + splitmix64(base) % span;

        let mut written = 0u16;
        let mut n = 0u64;
        while n < rolls {
            // Two independent draws per roll: which row, then how many.
            // Mixing the roll index in means roll 2 is not roll 1.
            let pick = splitmix64(base ^ ((n * 2 + 1) << 32));
            let target = (((pick >> 32) * t.total_weight as u64) >> 32) as u32;
            let mut acc = 0u32;
            let mut e = LootEntryDef::EMPTY;
            let mut i = 0usize;
            while i < t.len as usize {
                acc += t.entries[i].weight as u32;
                if acc > target {
                    e = t.entries[i];
                    break;
                }
                i += 1;
            }
            if e.item == NO_ITEM || e.item as usize >= MAX_ITEM_DEFS {
                n += 1;
                continue; // an inert row cannot pay
            }
            let cspan = (e.count_max.max(e.count_min) - e.count_min) as u64 + 1;
            let count = e.count_min as u64 + splitmix64(base ^ ((n * 2 + 2) << 32)) % cspan;
            let cap = gc.stack_max_of(e.item);
            if cap == 0 {
                n += 1;
                continue; // an item the ladder cannot stack cannot be held
            }
            // A rolled item is a mint, so it arrives at its own ceiling —
            // a barrel paying a dead tool would be a reward that is not one.
            written = written.saturating_add(inv_add(
                out,
                e.item,
                count as u16,
                cap,
                gc.cond_max_of(e.item),
            ));
            n += 1;
        }

        // The guaranteed rows, after the draw so the weighted stacks land
        // in the slots they always did: every open pays them. A ranged row
        // takes one salted draw ([`GUARANTEED_SALT`]); a fixed one takes
        // none. Bounded by `MAX_LOOT_GUARANTEED`, one `inv_add` each.
        let mut g = 0usize;
        while g < (t.guaranteed_len as usize).min(MAX_LOOT_GUARANTEED) {
            let e = t.guaranteed[g];
            g += 1;
            if e.item == NO_ITEM || e.item as usize >= MAX_ITEM_DEFS {
                continue;
            }
            let cspan = (e.count_max.max(e.count_min) - e.count_min) as u64 + 1;
            let count = if cspan == 1 {
                e.count_min as u64
            } else {
                e.count_min as u64 + splitmix64(base ^ ((GUARANTEED_SALT + g as u64) << 32)) % cspan
            };
            let cap = gc.stack_max_of(e.item);
            if cap == 0 {
                continue;
            }
            written = written.saturating_add(inv_add(
                out,
                e.item,
                count as u16,
                cap,
                gc.cond_max_of(e.item),
            ));
        }
        written
    }

    /// Synthetic table for the parity/replay/alloc gates. Deliberately
    /// unlike game content: two weighted rows over the fixture's items 0 and
    /// 1, plus one guaranteed row of item 2, so a bot run covers the
    /// weighted walk, the guaranteed pay and the stack cap without any real
    /// number living in code.
    pub fn probe_fixture() -> Self {
        let mut c = Self::EMPTY;
        let mut t = LootTableDef::INERT;
        t.entries[0] = LootEntryDef {
            item: 0,
            weight: 3,
            count_min: 2,
            count_max: 5,
        };
        t.entries[1] = LootEntryDef {
            item: 1,
            weight: 1,
            count_min: 1,
            count_max: 1,
        };
        t.len = 2;
        t.total_weight = 4;
        t.rolls_min = 1;
        t.rolls_max = 2;
        t.hits = 2;
        // One guaranteed row (loot guaranteed column v0), RANGED so its
        // count takes the salted draw inside parity, replay and alloc — a
        // fixed row draws nothing, and the unit tests below own that half.
        // Item 2, so the weighted rows' count bounds stay assertable apart.
        t.guaranteed[0] = LootEntryDef {
            item: 2,
            weight: 0,
            count_min: 1,
            count_max: 2,
        };
        t.guaranteed_len = 1;
        c.tables[LOOT_BARREL] = t;
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::gather::cell_key as cell;

    fn empty() -> [ItemStack; INV_SLOTS] {
        [ItemStack {
            item: 0,
            count: 0,
            cond: 0,
            skin: 0,
        }; INV_SLOTS]
    }

    #[test]
    fn an_inert_table_rolls_nothing() {
        let gc = GatherContent::probe_fixture();
        let mut out = empty();
        assert_eq!(
            LootContent::EMPTY.roll_into(LOOT_BARREL, &gc, 1, cell(2, 3), 4, &mut out),
            0
        );
        assert!(out.iter().all(|s| s.count == 0));
    }

    #[test]
    fn the_same_cell_and_tick_roll_the_same_thing() {
        let gc = GatherContent::probe_fixture();
        let lc = LootContent::probe_fixture();
        let (mut a, mut b) = (empty(), empty());
        lc.roll_into(LOOT_BARREL, &gc, 7, cell(11, 13), 900, &mut a);
        lc.roll_into(LOOT_BARREL, &gc, 7, cell(11, 13), 900, &mut b);
        assert_eq!(a, b);
        assert!(a.iter().any(|s| s.count > 0), "the fixture always pays");
    }

    #[test]
    fn a_later_smash_of_the_same_cell_is_a_new_roll() {
        // The cell alone would make one barrel pay the same thing forever.
        let gc = GatherContent::probe_fixture();
        let lc = LootContent::probe_fixture();
        let mut differed = false;
        for tick in 0..64u64 {
            let (mut a, mut b) = (empty(), empty());
            lc.roll_into(LOOT_BARREL, &gc, 7, cell(11, 13), tick, &mut a);
            lc.roll_into(LOOT_BARREL, &gc, 7, cell(11, 13), tick + 1, &mut b);
            differed |= a != b;
        }
        assert!(differed, "tick never entered the draw");
    }

    #[test]
    fn every_rolled_row_is_in_the_table_and_inside_its_count_range() {
        let gc = GatherContent::probe_fixture();
        let lc = LootContent::probe_fixture();
        for tick in 0..512u64 {
            let mut out = empty();
            lc.roll_into(LOOT_BARREL, &gc, 0xBEEF, cell(3, 5), tick, &mut out);
            for s in out.iter().filter(|s| s.count > 0) {
                match s.item {
                    // rolls_max 2, so at most two of a row stack up.
                    0 => assert!((2..=10).contains(&s.count), "item 0 count {}", s.count),
                    1 => assert!((1..=2).contains(&s.count), "item 1 count {}", s.count),
                    // The guaranteed row, paid once per open.
                    2 => assert!((1..=2).contains(&s.count), "item 2 count {}", s.count),
                    other => panic!("item {other} is not in the fixture table"),
                }
            }
        }
    }

    #[test]
    fn the_roll_count_stays_inside_the_declared_band() {
        let gc = GatherContent::probe_fixture();
        let lc = LootContent::probe_fixture();
        for tick in 0..512u64 {
            let mut out = empty();
            lc.roll_into(LOOT_BARREL, &gc, 0xF00D, cell(9, 9), tick, &mut out);
            let units: u32 = out.iter().map(|s| s.count as u32).sum();
            // 1 roll of the cheapest row at worst, 2 rolls of the richest
            // at best, plus the guaranteed row's 1..=2: [2, 12] over the
            // fixture's bounds.
            assert!((2..=12).contains(&units), "{units} units off one barrel");
        }
    }

    #[test]
    fn a_weight_one_row_is_reachable_and_stays_rare() {
        // Lemire over the baked total, not a modulo: the rare row must be
        // drawable at all, and must not be drawn a quarter of the time.
        let gc = GatherContent::probe_fixture();
        let lc = LootContent::probe_fixture();
        let mut rare = 0u32;
        let mut runs = 0u32;
        for tick in 0..4096u64 {
            let mut out = empty();
            lc.roll_into(LOOT_BARREL, &gc, 0x5EED, cell(1, 2), tick, &mut out);
            runs += 1;
            if out.iter().any(|s| s.item == 1 && s.count > 0) {
                rare += 1;
            }
        }
        assert!(rare > 0, "the weight-1 row never came up in {runs} rolls");
        assert!(
            rare * 2 < runs,
            "the weight-1 row came up {rare}/{runs} — the walk is not weighted"
        );
    }

    #[test]
    fn hits_reports_the_content_number_and_zero_when_inert() {
        assert_eq!(LootContent::probe_fixture().hits(LOOT_BARREL), 2);
        assert_eq!(LootContent::probe_fixture().hits(LOOT_CRATE), 0);
        assert_eq!(LootContent::EMPTY.hits(LOOT_BARREL), 0);
        assert_eq!(LootContent::probe_fixture().hits(MAX_LOOT_TABLES + 1), 0);
    }

    /// The guaranteed row pays on EVERY open, inside its band, and reaches
    /// both ends of it — the certainty the reference's scrap ladder is built
    /// on, and the thing a weighted row could not express.
    #[test]
    fn a_guaranteed_row_pays_every_open_inside_its_band() {
        let gc = GatherContent::probe_fixture();
        let lc = LootContent::probe_fixture();
        let (mut lo, mut hi) = (false, false);
        for tick in 0..512u64 {
            let mut out = empty();
            lc.roll_into(LOOT_BARREL, &gc, 0xACE, cell(4, 4), tick, &mut out);
            let paid: u32 = out
                .iter()
                .filter(|s| s.item == 2 && s.count > 0)
                .map(|s| s.count as u32)
                .sum();
            assert!(
                (1..=2).contains(&paid),
                "open {tick} paid {paid} of the guaranteed row"
            );
            lo |= paid == 1;
            hi |= paid == 2;
        }
        assert!(lo && hi, "a ranged row reaches both ends of its band");
    }

    /// A FIXED guaranteed row draws nothing, so the weighted half rolls
    /// bit-identically with or without it — the property that let the
    /// shipped tables take the column without moving any other roll.
    #[test]
    fn a_fixed_guaranteed_row_draws_nothing() {
        let gc = GatherContent::probe_fixture();
        let mut plain = LootContent::probe_fixture();
        plain.tables[LOOT_BARREL].guaranteed_len = 0;
        let mut fixed = plain;
        fixed.tables[LOOT_BARREL].guaranteed[0] = LootEntryDef {
            item: 2,
            weight: 0,
            count_min: 3,
            count_max: 3,
        };
        fixed.tables[LOOT_BARREL].guaranteed_len = 1;
        for tick in 0..256u64 {
            let (mut a, mut b) = (empty(), empty());
            plain.roll_into(LOOT_BARREL, &gc, 9, cell(6, 7), tick, &mut a);
            fixed.roll_into(LOOT_BARREL, &gc, 9, cell(6, 7), tick, &mut b);
            for (x, y) in a.iter().zip(b.iter()) {
                if y.item == 2 && y.count > 0 {
                    continue;
                }
                assert_eq!(x, y, "the fixed row moved a weighted stack");
            }
            let g: u32 = b
                .iter()
                .filter(|s| s.item == 2 && s.count > 0)
                .map(|s| s.count as u32)
                .sum();
            assert_eq!(g, 3, "a fixed row pays exactly its count");
        }
    }
}
