//! An arrow that strikes a dial's drum turns it one notch, from range, and
//! the archer whose arrow shows the answer is paid (`mech::shot`), driven
//! through `World::tick`.

#![allow(clippy::disallowed_macros)]

use sim_core::combat::{AmmoDef, CombatContent, RangedDef, NO_MAG};
use sim_core::gather::{GatherContent, ItemStack, NO_ITEM};
use sim_core::input::{InputFrame, BTN_PRIMARY};
use sim_core::mech::{answers, Drums, MechContent, OP_TURN};
use sim_core::movement::{Body, POS_Y_Q};
use sim_core::spot::{Spot, SITE_LANDMARK0};
use sim_core::world::{Command, World, EV_ARC_DID, EV_MECH_SOLVED};

const BOW: u16 = 12;
const ARROW: u16 = 13;
const ID: u32 = 1;
const P: usize = 0;
/// How far from the drum the archer stands, metres.
const RANGE_M: f32 = 8.0;

fn world() -> Box<World> {
    let mut w = Box::new(World::new(20261007));
    let mut g = GatherContent::probe_fixture();
    g.stack_max[BOW as usize] = 1;
    g.stack_max[ARROW as usize] = 64;
    g.item_count = ARROW + 1;
    w.gather = g;
    let mut c = CombatContent::EMPTY;
    c.player_hp = 100;
    c.ranged[BOW as usize] = RangedDef {
        damage: 40,
        ammo: [ARROW, NO_ITEM, NO_ITEM, NO_ITEM],
        rate_ticks: 30,
        hitscan: false,
        range_mm: 60_000,
        structure: 0,
        head_pct: 150,
        limb_pct: 50,
        magazine: 0,
        reload_ticks: 0,
        mag_slot: NO_MAG,
        draw_ticks: 0,
        gun: Default::default(),
    };
    c.ammo[ARROW as usize] = AmmoDef {
        speed_mmpt: 1333,
        drop_mmpt2: 11,
        damage_pct: 100,
        fire_ticks: [0, 0],
        blast: Default::default(),
    };
    w.combat = c;
    // The fixture's lock, moved out of the town (no weapon draws in the
    // safe zone) to the first landmark on the island, as the ring lock
    // stands at the stones.
    let mut mc = MechContent::probe_fixture();
    let kind = w
        .haven
        .marks
        .iter()
        .find(|m| m.live)
        .expect("a landmark")
        .kind as u8;
    mc.defs[0].spot = Spot {
        site: SITE_LANDMARK0 + kind,
        nth: 0,
        x_cm: 0,
        y_cm: 0,
        z_cm: 2000,
    };
    w.mech_def = mc;
    w.arc.salt = 0xFEED;
    w.tick(&[Command::Join { id: ID }]);
    w.players[P].inv[0] = ItemStack {
        item: BOW,
        count: 1,
        cond: 0,
        skin: 0,
    };
    w.players[P].inv[7] = ItemStack {
        item: ARROW,
        count: 8,
        cond: 0,
        skin: 0,
    };
    w
}

/// Stand the archer `RANGE_M` off dial 0, square to the row so no other
/// drum is in the line, and loose one arrow at the drum's middle. Returns
/// the `EV_ARC_DID` and `EV_MECH_SOLVED` payloads the flight raised.
fn shoot_dial_0(w: &mut World) -> Vec<(u8, u32, u32, u32)> {
    let drums = Drums::stand(&w.mech_def, w.seed, &w.haven);
    assert!(drums.n >= 2, "the lock's dials stand on the island");
    let (a, b) = (drums.d[0], drums.d[1]);
    let (rx, rz) = (b.x - a.x, b.z - a.z);
    let l = (rx * rx + rz * rz).sqrt();
    let (x, z) = (a.x - rz / l * RANGE_M, a.z + rx / l * RANGE_M);
    w.players[P].body = Body::at(w.seed, &w.haven, x, z);
    let eye = w.players[P].body.qy as f32 * POS_Y_Q + 1.5;
    let d = (a.x - x, a.y + 0.6 - eye, a.z - z);
    let n = (d.0 * d.0 + d.1 * d.1 + d.2 * d.2).sqrt();
    let (yaw, pitch, _) = sim_core::ranged::wire_angles((d.0 / n, d.1 / n, d.2 / n));
    let frame = InputFrame {
        buttons: BTN_PRIMARY,
        yaw,
        pitch,
        sel: 0,
        seq: w.players[P].frame.seq.wrapping_add(1),
        ..InputFrame::default()
    };
    let mut seen = Vec::new();
    let mut look = |w: &World| {
        for e in w.events.entries() {
            if e.code == EV_ARC_DID || e.code == EV_MECH_SOLVED {
                seen.push((e.code, e.a, e.b, e.c));
            }
        }
    };
    w.tick(&[Command::Input {
        id: ID,
        frame,
        favour: 0,
    }]);
    look(w);
    let frame = InputFrame {
        buttons: 0,
        seq: frame.seq.wrapping_add(1),
        ..frame
    };
    w.tick(&[Command::Input {
        id: ID,
        frame,
        favour: 0,
    }]);
    look(w);
    for _ in 0..30 {
        if w.arrows.is_empty() {
            break;
        }
        w.tick(&[]);
        look(w);
    }
    assert!(w.arrows.is_empty(), "the arrow never landed");
    // Clear the cadence for the next shot.
    for _ in 0..30 {
        w.tick(&[]);
    }
    seen
}

#[test]
fn an_arrow_turns_the_dial_it_strikes_and_a_solving_shot_pays_the_archer() {
    let mut w = world();
    let def = w.mech_def.defs[0];
    let want = answers(w.arc.salt, w.seed, 0, &def);
    // Every dial on its answer but the first, which is two notches short.
    w.arc.mechs[0].dials = want;
    w.arc.mechs[0].dials[0] = (want[0] + def.values - 2) % def.values;

    let seen = shoot_dial_0(&mut w);
    assert_eq!(seen, vec![(EV_ARC_DID, ID, OP_TURN as u32, 0)]);
    assert_eq!(
        w.arc.mechs[0].dials[0],
        (want[0] + def.values - 1) % def.values
    );
    assert_eq!(
        w.arc.mechs[0].dials[1..3],
        want[1..3],
        "only the struck dial turned"
    );

    let seen = shoot_dial_0(&mut w);
    assert_eq!(
        seen,
        vec![
            (EV_ARC_DID, ID, OP_TURN as u32, 0),
            (EV_MECH_SOLVED, 0, ID, 0)
        ]
    );
    assert!(w.arc.mechs[0].rest_until > w.tick, "the lock rests");
    assert_eq!(
        sim_core::craft::inv_count(&w.players[P].inv, def.reward),
        def.reward_n as u32,
        "the archer was paid"
    );
}
