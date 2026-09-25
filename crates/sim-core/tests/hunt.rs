//! Hunting at range: an arrow or a bullet that meets an animal hurts it,
//! as a swing always has (`ranged::Quarry`, `mob::hurt_slot`). Until this,
//! both passed straight through — `mob_cast` was melee's alone, while the
//! animals' own hearing (`noise.rs`) was written for a bow in the grass.
//!
//! Driven through `World::tick`, because the path is the world's: the
//! ranged passes find the animal, and `World` lands the hit.

use sim_core::combat::{AmmoDef, CombatContent, RangedDef, NO_MAG};
use sim_core::gather::{ItemStack, NO_ITEM};
use sim_core::input::{InputFrame, BTN_PRIMARY};
use sim_core::limits::MAX_MOBS;
use sim_core::mob::{self, MobContent, MOB_PIG};
use sim_core::movement::{Body, POS_XZ_Q, POS_Y_Q};
use sim_core::nav::yaw_toward;
use sim_core::pitch_dir;
use sim_core::ranged::ARROW_EYE_MM;
use sim_core::world::{Command, World};

const SEED: u64 = 11;
const GUN: u16 = 5;
const ROUND: u16 = 6;
const BOW: u16 = 3;
const ARROW: u16 = 4;

/// A revolver (`tests/gun.rs`' numbers) and a bow (`tests/shoot.rs`').
fn weapons() -> CombatContent {
    let mut c = CombatContent::EMPTY;
    c.player_hp = 100;
    c.ranged[GUN as usize] = RangedDef {
        damage: 20,
        ammo: [ROUND, NO_ITEM, NO_ITEM, NO_ITEM],
        rate_ticks: 12,
        hitscan: true,
        range_mm: 50_000,
        structure: 0,
        headshot_mult: 2,
        limb_pct: 50,
        magazine: 8,
        reload_ticks: 102,
        mag_slot: 0,
    };
    c.ranged[BOW as usize] = RangedDef {
        damage: 30,
        ammo: [ARROW, NO_ITEM, NO_ITEM, NO_ITEM],
        rate_ticks: 60,
        hitscan: false,
        range_mm: 60_000,
        structure: 0,
        headshot_mult: 2,
        limb_pct: 50,
        magazine: 0,
        reload_ticks: 0,
        mag_slot: NO_MAG,
    };
    c.ammo[ARROW as usize] = AmmoDef {
        speed_mmpt: 1333,
        drop_mmpt2: 22,
    };
    c
}

/// A joined player holding `weapon` in slot 0 (and a loaded magazine or a
/// quiver), and one pig 8 m east of them — every other animal retired.
fn range_with(weapon: u16) -> (World, usize) {
    let mut w = World::new(SEED);
    w.env = sim_core::weather::Env::CLEAR;
    w.combat = weapons();
    w.mob = MobContent::probe_fixture();
    w.dev_spawn = Some(w.spawn_pos(1));
    w.tick(&[Command::Join { id: 1 }]);
    let pig = (0..MAX_MOBS)
        .find(|&s| mob::kind_of(s) == MOB_PIG && w.mobs.m[s].alive)
        .expect("a live pig");
    for (i, m) in w.mobs.m.iter_mut().enumerate() {
        if i != pig {
            m.alive = false;
        }
    }
    let b = w.players[0].body;
    let (px, pz) = (b.qx as f32 * POS_XZ_Q, b.qz as f32 * POS_XZ_Q);
    let haven = w.haven;
    let m = &mut w.mobs.m[pig];
    m.body = Body::at(SEED, &haven, px + 8.0, pz);
    m.home_qx = m.body.qx;
    m.home_qz = m.body.qz;
    m.path.clear();
    let p = &mut w.players[0];
    p.inv[0] = ItemStack {
        item: weapon,
        count: 1,
        cond: 0,
        skin: 0,
    };
    if weapon == GUN {
        p.mag[0] = 8;
        p.mag_round[0] = ROUND;
    } else {
        p.inv[7] = ItemStack {
            item: ARROW,
            count: 10,
            cond: 0,
            skin: 0,
        };
    }
    (w, pig)
}

/// The yaw and pitch bytes from the player's eye to the middle of the pig,
/// off the sim's own LUTs.
fn aim_at(w: &World, pig: usize) -> (u16, u8) {
    let (p, m) = (w.players[0].body, w.mobs.m[pig].body);
    let eye = p.qy as f32 * POS_Y_Q + ARROW_EYE_MM as f32 / 1000.0;
    let mid = m.qy as f32 * POS_Y_Q + f32::from(w.mob.def(MOB_PIG).body_h_cm) * 0.005;
    let (dx, dz) = (
        (m.qx - p.qx) as f32 * POS_XZ_Q,
        (m.qz - p.qz) as f32 * POS_XZ_Q,
    );
    let (run, rise) = ((dx * dx + dz * dz).sqrt(), mid - eye);
    let pitch = (0..=255u8)
        .max_by(|&a, &b| {
            let score = |p: u8| {
                let (ch, sv) = pitch_dir(p);
                ch * run + sv * rise
            };
            score(a).total_cmp(&score(b))
        })
        .expect("a pitch");
    (yaw_toward(dx, dz, 0), pitch)
}

fn press(w: &mut World, seq: u16, buttons: u8, yaw: u16, pitch: u8) {
    let frame = InputFrame {
        seq,
        buttons,
        yaw,
        pitch,
        sel: 0,
        ..InputFrame::default()
    };
    w.tick(&[Command::Input {
        id: 1,
        frame,
        favour: 0,
    }]);
}

#[test]
fn a_bullet_hurts_the_animal_it_meets() {
    let (mut w, pig) = range_with(GUN);
    let full = w.mobs.m[pig].hp;
    assert!(full > 0);
    let (yaw, pitch) = aim_at(&w, pig);
    press(&mut w, 1, BTN_PRIMARY, yaw, pitch);
    let m = &w.mobs.m[pig];
    assert!(
        m.hp < full,
        "a revolver round at a pig 8 m off passed through it (hp {} of {full})",
        m.hp
    );
    assert_eq!(m.target, 0, "the shot pig did not turn on its shooter");
}

#[test]
fn an_arrow_hurts_the_animal_it_meets() {
    let (mut w, pig) = range_with(BOW);
    let full = w.mobs.m[pig].hp;
    let (yaw, pitch) = aim_at(&w, pig);
    press(&mut w, 1, BTN_PRIMARY, yaw, pitch);
    // Eight metres at 1.3 m a tick: well inside twenty ticks of flight.
    for seq in 2..22 {
        press(&mut w, seq, 0, yaw, pitch);
    }
    assert!(
        w.mobs.m[pig].hp < full,
        "an arrow at a pig 8 m off passed through it (hp {} of {full})",
        w.mobs.m[pig].hp
    );
}
