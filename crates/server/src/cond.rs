//! The condition wall on the save readers — **a save may not carry a
//! condition no command can mint.**
//!
//! Both save decoders run without the content tables in scope:
//! `PlayerSave::read_le` and `worldsave::decode_into` bound every item
//! index and enforce the canonical-empty trio, but neither can ask what an
//! item's `condition_max` is, because that number lives in the baked
//! content and the decoders are deliberately content-free. So a save file
//! could smuggle in a stack whose `cond` sits above its item's ceiling, or
//! a nonzero `cond` on an item whose `condition_max` is 0 — states no
//! command can mint (every mint site passes `GatherContent::cond_max_of`,
//! and wear only ever subtracts), arriving through the ONE non-command
//! path into `World`. This module is the post-load check, run where
//! content IS in scope: the server's boot path.
//!
//! ## Refused, not clamped — and why
//!
//! `persist.rs` is the prior art: a body past `SAVE_Y_LIMIT_Q` is
//! *"refused rather than clamped: a body a kilometre under the seabed is a
//! corrupt record, not a player"*, and `hp > hp_max` is refused as
//! `HpOverMax`. An over-ceiling `cond` is the same violation one field
//! over — an item healthier than its own ceiling — so it takes the same
//! door. Two things make refusal the honest choice rather than the harsh
//! one:
//!
//! 1. **The innocent explanation is unreachable.** The one legitimate way
//!    a saved `cond` could stand above its ceiling is content moving under
//!    the file (the ceiling lowered after the save was written) — and both
//!    file headers pin the content hash and refuse a mismatch *before any
//!    record is read*. Past the header, an over-ceiling `cond` can only be
//!    a hand-edit or corruption, which is structural, not a value to round.
//! 2. **A clamp would launder the edit.** `cond` is hashed sim state
//!    (wall 5); a loader that quietly rounded a forged 65535 down to the
//!    ceiling would accept the edited file and hand its author a
//!    fully-repaired tool — repair is deliberately v1 (re-craft is the
//!    repair), so the clamp would be the only free repair in the game.
//!
//! Per-path policy, stated where each is implemented: the **player store**
//! counts the record as corrupt (`SaveLoad::corrupt` — one player starts
//! over, the file boots); the **world file** refuses the boot whole
//! (`worldfile::open`), because a world is loaded whole or not at all.

use sim_core::gather::{GatherContent, ItemStack};

/// One stack whose condition the loaded content could not have minted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CondViolation {
    /// Index into the slice that was checked.
    pub slot: usize,
    pub item: u16,
    pub cond: u16,
    /// The ceiling content declares for `item` (0 = carries no condition).
    pub max: u16,
}

/// The first stack in `stacks` carrying an un-mintable condition, or `None`
/// when every occupied slot is within its item's ceiling.
///
/// One predicate covers both smuggled shapes: `cond <= cond_max_of(item)`
/// refuses a worn item above its ceiling AND any nonzero `cond` on an item
/// whose ceiling is 0. Empty slots are skipped — `count == 0 && cond != 0`
/// is already refused by both decoders (the canonical-empty trio), so it
/// cannot reach this check.
pub fn violation(stacks: &[ItemStack], gather: &GatherContent) -> Option<CondViolation> {
    stacks.iter().enumerate().find_map(|(slot, s)| {
        let max = gather.cond_max_of(s.item);
        (s.count > 0 && s.cond > max).then_some(CondViolation {
            slot,
            item: s.item,
            cond: s.cond,
            max,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ceiling_holds_and_the_legal_band_passes() {
        let gc = GatherContent::probe_fixture(); // cond_max[0] = 400, [2] = 0
        let stack = |item, count, cond| ItemStack { item, count, cond };

        // Legal: at the ceiling, worn, dead (cond 0 with a ceiling), and a
        // conditionless item at 0.
        for s in [
            stack(0, 1, 400),
            stack(0, 1, 17),
            stack(0, 1, 0),
            stack(2, 5, 0),
        ] {
            assert_eq!(violation(&[s], &gc), None, "{s:?} is mintable");
        }

        // Over the ceiling by one.
        assert_eq!(
            violation(&[stack(0, 1, 401)], &gc),
            Some(CondViolation {
                slot: 0,
                item: 0,
                cond: 401,
                max: 400
            })
        );
        // Condition on an item that carries none.
        assert_eq!(
            violation(&[stack(2, 1, 1)], &gc),
            Some(CondViolation {
                slot: 0,
                item: 2,
                cond: 1,
                max: 0
            })
        );
        // The slot index names the offender, not the first slot.
        assert_eq!(
            violation(&[stack(0, 1, 400), stack(0, 1, 65535)], &gc).map(|v| v.slot),
            Some(1)
        );
    }
}
