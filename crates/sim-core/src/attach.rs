//! Weapon attachments — Rust's weapon mods: a holosight, an 8x scope, a
//! silencer, a muzzle brake or boost, a laser sight, a flashlight, an
//! extended magazine.
//!
//! An attachment is an item until it goes on a gun, and goes back to being
//! one when it comes off. On the gun it is a bit of the gun's own state
//! (`combat::GunState::mods`, in its stack's data): the scope is on *this*
//! rifle, and goes where the rifle goes — into a box, a bag, a looter's
//! hands. What each one does to the shot is `CombatContent::modded`; what
//! it does to the view is the client's.
//!
//! A gun holds one attachment per slot (`ModDef::group`): putting a second
//! sight on takes the first one off, into the pack.

use crate::combat::{held_gun, held_item, set_held_gun, CombatContent, GunState, RangedDef};
use crate::gather::{inv_add_spilling, GatherContent, ItemStack};
use crate::limits::INV_SLOTS;
use crate::world::{EventQueue, Player, EV_RELOAD};

/// The gun in `p`'s hand, if it takes attachments, and its state.
fn held(cc: &CombatContent, p: &Player) -> Option<(RangedDef, GunState)> {
    let def = cc.held_ranged(held_item(p))?;
    (def.gun.mods != 0).then(|| (def, held_gun(p)))
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
    let Some((def, mut g)) = held(cc, p) else {
        return false;
    };
    if def.gun.mods & (1 << bit) == 0 || g.mods & (1 << bit) != 0 {
        return false;
    }
    p.inv[s].count -= 1;
    if p.inv[s].count == 0 {
        p.inv[s] = ItemStack::default();
    }
    let group = cc.mods[bit as usize].group;
    for (b, m) in cc.mods.iter().enumerate() {
        if b != bit as usize && g.mods & (1 << b) != 0 && m.group == group {
            g.mods &= !(1 << b);
            give_back(gc, p, m.item, spill);
        }
    }
    g.mods |= 1 << bit;
    fit_magazine(cc, gc, events, p, def, g, spill);
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
    let Some((def, mut g)) = held(cc, p) else {
        return false;
    };
    if g.mods & (1 << bit) == 0 {
        return false;
    }
    g.mods &= !(1 << bit);
    give_back(gc, p, cc.mods[bit as usize].item, spill);
    fit_magazine(cc, gc, events, p, def, g, spill);
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

/// Write `g` back to the gun, fitting its magazine to what the attachments
/// now allow (an extended magazine coming off): rounds past the new ceiling
/// go back to the pack, and the pair is restated so the readout shows the
/// ceiling the gun now has.
fn fit_magazine(
    cc: &CombatContent,
    gc: &GatherContent,
    events: &mut EventQueue,
    p: &mut Player,
    def: RangedDef,
    mut g: GunState,
    spill: &mut [ItemStack; INV_SLOTS],
) {
    if def.magazine == 0 {
        set_held_gun(p, g);
        return;
    }
    let cap = cc.modded(def, g.mods).magazine;
    let over = g.loaded.saturating_sub(cap);
    if over > 0 {
        let round = g.round;
        inv_add_spilling(
            &mut p.inv,
            spill,
            round,
            over,
            gc.stack_max_of(round),
            gc.cond_max_of(round),
        );
        g.loaded = cap;
    }
    set_held_gun(p, g);
    events.push(EV_RELOAD, p.id, crate::ranged::mag_pair(g.loaded, cap), 0);
}
