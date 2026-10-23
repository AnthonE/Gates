//! Weapon attachments (`sim_core::attach`, Rust's weapon mods): one per slot
//! of a gun, from the pack and back to it, what each does to the shot, the
//! silencer on the wire, and the scope coming off in the corpse.

#![allow(clippy::disallowed_macros)]

use sim_core::combat::{
    CombatContent, Gun, ModDef, RangedDef, MOD_MAGAZINE, MOD_MUZZLE, MOD_SIGHT,
};
use sim_core::craft::inv_count;
use sim_core::gather::{GatherContent, ItemStack, NO_ITEM};
use sim_core::limits::INV_SLOTS;
use sim_core::world::{EventQueue, Player, EV_RELOAD};

const GUN: u16 = 5;
const ROUND: u16 = 6;
const HOLO: u16 = 20;
const SCOPE: u16 = 21;
const SILENCER: u16 = 22;
const EXT_MAG: u16 = 23;
const BOOST: u16 = 24;

fn content() -> CombatContent {
    let mut c = CombatContent::EMPTY;
    c.player_hp = 100;
    c.ranged[GUN as usize] = RangedDef {
        damage: 50,
        ammo: [ROUND, NO_ITEM, NO_ITEM, NO_ITEM],
        rate_ticks: 4,
        rate_sub: 1024,
        hitscan: true,
        range_mm: 100_000,
        structure: 0,
        head_pct: 200,
        limb_pct: 50,
        magazine: 30,
        reload_ticks: 100,
        mag_slot: 2,
        draw_ticks: 0,
        gun: Gun {
            aim: 20,
            hip: 250,
            // Holo, scope, silencer and the extended magazine; not the boost.
            mods: 0b0000_1111,
            ..Gun::NONE
        },
    };
    let row = |item, group| ModDef {
        item,
        group,
        ..ModDef::NONE
    };
    c.mods[0] = ModDef {
        aim_pct: 80,
        ..row(HOLO, MOD_SIGHT)
    };
    c.mods[1] = row(SCOPE, MOD_SIGHT);
    c.mods[2] = ModDef {
        damage_pct: 75,
        velocity_pct: 75,
        quiet: true,
        ..row(SILENCER, MOD_MUZZLE)
    };
    c.mods[3] = ModDef {
        magazine_pct: 125,
        ..row(EXT_MAG, MOD_MAGAZINE)
    };
    c.mods[4] = ModDef {
        rate_pct: 110,
        ..row(BOOST, MOD_MUZZLE)
    };
    c
}

fn gather() -> GatherContent {
    let mut g = GatherContent::EMPTY;
    for item in [ROUND, HOLO, SCOPE, SILENCER, EXT_MAG, BOOST] {
        g.stack_max[item as usize] = if item == ROUND { 128 } else { 1 };
    }
    g
}

fn armed() -> Player {
    let mut p = Player {
        id: 1,
        active: true,
        hp: 100,
        hp_max: 100,
        ..Player::default()
    };
    p.inv[0] = ItemStack {
        item: GUN,
        count: 1,
        ..ItemStack::default()
    };
    for (slot, item) in [
        (10, HOLO),
        (11, SCOPE),
        (12, SILENCER),
        (13, EXT_MAG),
        (14, BOOST),
    ] {
        p.inv[slot] = ItemStack {
            item,
            count: 1,
            ..ItemStack::default()
        };
    }
    p
}

fn attach(c: &CombatContent, p: &mut Player, slot: u8) -> bool {
    let mut spill = [ItemStack::default(); INV_SLOTS];
    sim_core::attach::attach(
        c,
        &gather(),
        &mut EventQueue::default(),
        p,
        slot,
        &mut spill,
    )
}

/// A sight goes on from the pack and takes its bit on the gun's row; a
/// second sight takes the first one's place and the first goes back.
#[test]
fn one_attachment_a_slot_and_the_old_one_back_to_the_pack() {
    let c = content();
    let mut p = armed();
    assert!(attach(&c, &mut p, 10), "the holosight goes on");
    assert_eq!(p.mods[2], 0b0001);
    assert_eq!(inv_count(&p.inv, HOLO), 0, "it left the pack");
    assert!(attach(&c, &mut p, 11), "the scope goes on in its place");
    assert_eq!(p.mods[2], 0b0010, "one sight at a time");
    assert_eq!(inv_count(&p.inv, HOLO), 1, "the holosight came back");
    assert!(attach(&c, &mut p, 12), "a muzzle is another slot");
    assert_eq!(p.mods[2], 0b0110);
    // A gun that does not take the boost refuses it, and nothing moves.
    assert!(!attach(&c, &mut p, 14));
    assert_eq!(inv_count(&p.inv, BOOST), 1);
    // Nor does an empty hand take anything.
    let mut bare = armed();
    bare.inv[0] = ItemStack::default();
    assert!(!attach(&c, &mut bare, 10));
}

/// What attachments do to the shot: the sight's cone, the silencer's damage,
/// speed and quiet, the magazine's quarter more, the boost's cadence.
#[test]
fn attachments_change_the_gun_the_shot_leaves() {
    let c = content();
    let def = c.ranged[GUN as usize];
    let holo = c.modded(def, 0b0001);
    assert_eq!(holo.gun.aim, 16);
    assert_eq!(holo.gun.hip, 250, "the holosight leaves the hip alone");
    let quiet = c.modded(def, 0b0100);
    assert_eq!((quiet.damage, quiet.gun.velocity_pct), (37, 75));
    assert!(c.quiet(&def, 0b0100) && !c.quiet(&def, 0b0001));
    assert_eq!(c.modded(def, 0b1000).magazine, 37);
    // A bit the gun does not take does nothing, whatever the mask says.
    assert_eq!(c.modded(def, 0b1_0000), def);
    let mut takes_boost = def;
    takes_boost.gun.mods |= 0b1_0000;
    let fast = c.modded(takes_boost, 0b1_0000);
    assert_eq!(fast.rate_sub, 1024 * 100 / 110);
    assert_eq!(c.mod_bit(SCOPE), Some(1));
    assert_eq!(c.mod_bit(ROUND), None);
}

/// Taking the extended magazine off a magazine fuller than the gun now
/// holds sends the rounds past the ceiling back to the pack, and restates
/// the magazine so the readout shows it.
#[test]
fn an_extended_magazine_coming_off_returns_the_rounds_past_the_ceiling() {
    let c = content();
    let mut p = armed();
    assert!(attach(&c, &mut p, 13));
    p.mag[2] = 37;
    p.mag_round[2] = ROUND;
    let mut spill = [ItemStack::default(); INV_SLOTS];
    let mut ev = EventQueue::default();
    assert!(sim_core::attach::detach(
        &c,
        &gather(),
        &mut ev,
        &mut p,
        3,
        &mut spill
    ));
    assert_eq!(p.mods[2], 0);
    assert_eq!(p.mag[2], 30);
    assert_eq!(inv_count(&p.inv, ROUND), 7, "seven rounds came back");
    assert_eq!(inv_count(&p.inv, EXT_MAG), 1, "and the magazine itself");
    let pair = ev
        .entries()
        .iter()
        .find(|e| e.code == EV_RELOAD)
        .expect("the magazine is restated")
        .b;
    assert_eq!(pair, sim_core::ranged::mag_pair(30, 30));
    // Nothing on: a detach refuses.
    assert!(!sim_core::attach::detach(
        &c,
        &gather(),
        &mut ev,
        &mut p,
        3,
        &mut spill
    ));
}
