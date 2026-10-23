//! Weapon attachments — Rust's weapon mods: a holosight, an 8x scope, a
//! silencer, a muzzle brake or boost, a laser sight, a flashlight, an
//! extended magazine.
//!
//! An attachment is an item until it goes on a gun, and goes back to being
//! one when it comes off. On the gun it is a bit of `Player::mods`, keyed by
//! the gun's weapon row as the magazine is (`RangedDef::mag_slot`): the scope
//! is on *your* rifle the way the rounds are in *your* rifle, it is saved
//! with you, and a death sheds it into the bag as the item (`World::die`),
//! so the killer loots it. What each one does to the shot is
//! `CombatContent::modded`; what it does to the view is the client's.
//!
//! A gun holds one attachment per slot (`ModDef::group`): putting a second
//! sight on takes the first one off, into the pack.

use crate::combat::{held_item, CombatContent};
use crate::gather::{inv_add_spilling, GatherContent, ItemStack};
use crate::limits::{INV_SLOTS, MAX_MAGS};
use crate::world::{EventQueue, Player, EV_RELOAD};

/// The gun in `p`'s hand and its magazine row, if it takes attachments.
fn gun_row(cc: &CombatContent, p: &Player) -> Option<(crate::combat::RangedDef, usize)> {
    let def = cc.held_ranged(held_item(p))?;
    let row = def.mag_slot as usize;
    (row < MAX_MAGS && def.gun.mods != 0).then_some((def, row))
}

/// Put the attachment in inventory slot `slot` on the gun in `p`'s hand.
/// Refused (`false`, nothing moved) for anything that is not an attachment,
/// a hand with no gun, a gun that does not take it, or one already wearing
/// it. Whatever filled the same slot of the gun comes off into the pack.
pub fn attach(
    cc: &CombatContent,
    gc: &GatherContent,
    events: &mut EventQueue,
    p: &mut Player,
    slot: u8,
    spill: &mut [ItemStack; INV_SLOTS],
) -> bool {
    let s = slot as usize;
    if s >= INV_SLOTS || p.inv[s].count == 0 {
        return false;
    }
    let item = p.inv[s].item;
    let Some(bit) = cc.mod_bit(item) else {
        return false;
    };
    let Some((def, row)) = gun_row(cc, p) else {
        return false;
    };
    if def.gun.mods & (1 << bit) == 0 || p.mods[row] & (1 << bit) != 0 {
        return false;
    }
    p.inv[s].count -= 1;
    if p.inv[s].count == 0 {
        p.inv[s] = ItemStack::default();
    }
    let group = cc.mods[bit as usize].group;
    for (b, m) in cc.mods.iter().enumerate() {
        if b != bit as usize && p.mods[row] & (1 << b) != 0 && m.group == group {
            p.mods[row] &= !(1 << b);
            give_back(gc, p, m.item, spill);
        }
    }
    p.mods[row] |= 1 << bit;
    fit_magazine(cc, gc, events, p, def, row, spill);
    true
}

/// Take attachment `bit` off the gun in `p`'s hand, into the pack.
pub fn detach(
    cc: &CombatContent,
    gc: &GatherContent,
    events: &mut EventQueue,
    p: &mut Player,
    bit: u8,
    spill: &mut [ItemStack; INV_SLOTS],
) -> bool {
    if bit as usize >= cc.mods.len() {
        return false;
    }
    let Some((def, row)) = gun_row(cc, p) else {
        return false;
    };
    if p.mods[row] & (1 << bit) == 0 {
        return false;
    }
    p.mods[row] &= !(1 << bit);
    give_back(gc, p, cc.mods[bit as usize].item, spill);
    fit_magazine(cc, gc, events, p, def, row, spill);
    true
}

fn give_back(gc: &GatherContent, p: &mut Player, item: u16, spill: &mut [ItemStack; INV_SLOTS]) {
    inv_add_spilling(
        &mut p.inv,
        spill,
        item,
        1,
        gc.stack_max_of(item),
        gc.cond_max_of(item),
    );
}

/// The magazine after an attachment changed it (an extended magazine coming
/// off): rounds past the new ceiling go back to the pack, and the pair is
/// restated so the readout shows the ceiling the gun now has.
fn fit_magazine(
    cc: &CombatContent,
    gc: &GatherContent,
    events: &mut EventQueue,
    p: &mut Player,
    def: crate::combat::RangedDef,
    row: usize,
    spill: &mut [ItemStack; INV_SLOTS],
) {
    if def.magazine == 0 {
        return;
    }
    let cap = cc.modded(def, p.mods[row]).magazine;
    let over = p.mag[row].saturating_sub(cap);
    if over > 0 {
        let round = p.mag_round[row];
        inv_add_spilling(
            &mut p.inv,
            spill,
            round,
            over,
            gc.stack_max_of(round),
            gc.cond_max_of(round),
        );
        p.mag[row] = cap;
    }
    events.push(EV_RELOAD, p.id, crate::ranged::mag_pair(p.mag[row], cap), 0);
}
