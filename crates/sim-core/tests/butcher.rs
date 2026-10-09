//! Butchering (`NOW.md` §0m item 1): a tool with a `[butcher]` row, swung at
//! an animal's carcass, cuts one stack out of it per hit at the tool's rate
//! and wears the tool; a hand with no row passes through the carcass and cuts
//! nothing. Driven through `World::tick`, because the cast and the cut are
//! the world's.

use sim_core::backpack::BackpackContent;
use sim_core::combat::CombatContent;
use sim_core::gather::{GatherContent, ItemStack};
use sim_core::input::{InputFrame, BTN_PRIMARY};
use sim_core::limits::{INV_SLOTS, MAX_MOBS};
use sim_core::mob::{self, ButcherRow, MobContent, MOB_PIG};
use sim_core::movement::{Body, POS_XZ_Q, POS_Y_Q};
use sim_core::nav::yaw_toward;
use sim_core::pitch_dir;
use sim_core::world::{Command, World, EV_GATHER};

const SEED: u64 = 11;
/// Item indices well past every probe fixture's own rows.
const KNIFE: u16 = 80;
const CLUB: u16 = 81;
const MEAT: u16 = 82;
const FAT: u16 = 83;
const KNIFE_PCT: u16 = 150;
const KNIFE_WEAR: u16 = 30;
const KNIFE_COND: u16 = 10_000;

/// A joined player holding `tool`, every animal retired, and a pig's carcass
/// of 4 meat and 10 fat lying 1.2 m east of them. Returns the carcass id.
fn carcass_before(tool: u16) -> (Box<World>, u32) {
    let mut w = Box::new(World::new(SEED));
    w.env = sim_core::weather::Env::CLEAR;
    w.gather = GatherContent::probe_fixture();
    w.combat = CombatContent::probe_fixture();
    w.mob = MobContent::probe_fixture();
    w.backpack = BackpackContent::probe_fixture();
    for it in [KNIFE, CLUB] {
        w.gather.stack_max[it as usize] = 1;
    }
    w.gather.cond_max[KNIFE as usize] = KNIFE_COND;
    for it in [MEAT, FAT] {
        w.gather.stack_max[it as usize] = 1000;
    }
    w.gather.item_count = w.gather.item_count.max(FAT + 1);
    w.mob.butcher[0] = ButcherRow {
        tool: KNIFE,
        pct: KNIFE_PCT,
        wear: KNIFE_WEAR,
    };
    w.dev_spawn = Some(w.spawn_pos(1));
    w.tick(&[Command::Join { id: 1 }]);
    for m in w.mobs.m.iter_mut() {
        m.alive = false;
    }
    let pig = (0..MAX_MOBS)
        .find(|&s| mob::kind_of(s) == MOB_PIG)
        .expect("a pig slot");
    w.players[0].inv = [ItemStack::default(); INV_SLOTS];
    w.players[0].inv[0] = ItemStack {
        item: tool,
        count: 1,
        cond: if tool == KNIFE { KNIFE_COND } else { 0 },
        skin: 0,
    };
    let b = w.players[0].body;
    let at = Body::at(
        SEED,
        &w.haven,
        b.qx as f32 * POS_XZ_Q + 1.2,
        b.qz as f32 * POS_XZ_Q,
    );
    let mut items = [ItemStack::default(); INV_SLOTS];
    items[0] = ItemStack {
        item: MEAT,
        count: 4,
        cond: 0,
        skin: 0,
    };
    items[1] = ItemStack {
        item: FAT,
        count: 10,
        cond: 0,
        skin: 0,
    };
    let tick = w.tick;
    let id = w
        .backpacks
        .stand_up(
            &w.backpack,
            at.qx,
            at.qy,
            at.qz,
            mob::mob_id(pig),
            &items,
            tick,
            &mut w.events,
        )
        .expect("the carcass stood up");
    (w, id)
}

/// Yaw and pitch from the eye to the middle of the lying carcass.
fn aim(w: &World, id: u32) -> (u16, u8) {
    let p = w.players[0].body;
    let c = *w
        .backpacks
        .entries()
        .iter()
        .find(|b| b.id == id)
        .expect("the carcass");
    let eye = p.qy as f32 * POS_Y_Q + sim_core::ranged::eye_mm(false) as f32 / 1000.0;
    let mid = c.qy as f32 * POS_Y_Q + f32::from(w.mob.def(MOB_PIG).body_h_cm) * 0.0025;
    let (dx, dz) = (
        (c.qx - p.qx) as f32 * POS_XZ_Q,
        (c.qz - p.qz) as f32 * POS_XZ_Q,
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

/// Hold the button until one swing is taken; return what it paid, by item.
fn swing(w: &mut World, id: u32, seq: &mut u16) -> Vec<(u16, u16)> {
    let (yaw, pitch) = aim(w, id);
    for _ in 0..60 {
        *seq = seq.wrapping_add(1);
        let frame = InputFrame {
            seq: *seq,
            buttons: BTN_PRIMARY,
            yaw,
            pitch,
            ..InputFrame::default()
        };
        w.tick(&[Command::Input {
            id: 1,
            frame,
            favour: 0,
        }]);
        let swung = w
            .events
            .entries()
            .iter()
            .any(|e| e.code == sim_core::world::EV_SWING && e.a == 1);
        if swung {
            return w
                .events
                .entries()
                .iter()
                .filter(|e| e.code == EV_GATHER && e.a == 1)
                .map(|e| ((e.b >> 16) as u16, (e.b & 0xffff) as u16))
                .collect();
        }
    }
    panic!("no swing was taken in 60 ticks");
}

fn carcass_left(w: &World, id: u32) -> Option<[ItemStack; INV_SLOTS]> {
    w.backpacks
        .entries()
        .iter()
        .find(|b| b.id == id)
        .map(|b| b.items)
}

#[test]
fn a_knife_cuts_one_stack_a_swing_at_its_rate_and_wears() {
    let (mut w, id) = carcass_before(KNIFE);
    let mut seq = 0;

    let paid = swing(&mut w, id, &mut seq);
    assert_eq!(paid, vec![(MEAT, 6)], "the first cut is the meat, at 150%");
    assert_eq!(
        w.players[0].inv[0].cond,
        KNIFE_COND - KNIFE_WEAR,
        "a cut wears the knife by its row"
    );
    let left = carcass_left(&w, id).expect("the fat is still in it");
    assert_eq!(left[0].count, 0, "the meat was cut out");
    assert_eq!(left[1].count, 10, "one cut a swing, not the whole carcass");

    let paid = swing(&mut w, id, &mut seq);
    assert_eq!(paid, vec![(FAT, 15)], "the second cut is the fat, at 150%");
    assert!(
        carcass_left(&w, id).is_none(),
        "an emptied carcass leaves the world"
    );
}

#[test]
fn a_hand_with_no_butcher_row_cuts_nothing() {
    let (mut w, id) = carcass_before(CLUB);
    let mut seq = 0;
    let paid = swing(&mut w, id, &mut seq);
    assert!(paid.is_empty(), "a club with no row paid {paid:?}");
    let left = carcass_left(&w, id).expect("the carcass is untouched");
    assert_eq!((left[0].count, left[1].count), (4, 10));
}
