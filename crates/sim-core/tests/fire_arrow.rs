//! A fire arrow leaves a fire where it lands, not an arrow (`fire.rs`), and
//! the fire burns whoever stands in it — 2 health a second, for as long as
//! the round says — driven through `World::tick`.

#![allow(clippy::disallowed_macros)]

use sim_core::combat::{AmmoDef, CombatContent, RangedDef, NO_MAG};
use sim_core::gather::{GatherContent, ItemStack, NO_ITEM};
use sim_core::input::{InputFrame, BTN_PRIMARY};
use sim_core::movement::Body;
use sim_core::world::{fire_parts, Command, World, EV_FIRE};

const BOW: u16 = 6;
const FIRE_ARROW: u16 = 7;
/// Ten seconds, both ends, so the test knows when it goes out.
const BURN: u16 = 300;
const P: usize = 0;

/// One archer with a bow and fire arrows, on open ground well outside the
/// town (whose safe zone no fire may burn in).
fn world_with_fire_arrows() -> Box<World> {
    let mut w = Box::new(World::new(20261007));
    let mut g = GatherContent::EMPTY;
    g.stack_max[FIRE_ARROW as usize] = 64;
    g.stack_max[BOW as usize] = 1;
    g.item_count = FIRE_ARROW + 1;
    w.gather = g;
    let mut c = CombatContent::EMPTY;
    c.player_hp = 100;
    c.ranged[BOW as usize] = RangedDef {
        damage: 40,
        ammo: [FIRE_ARROW, NO_ITEM, NO_ITEM, NO_ITEM],
        rate_ticks: 38,
        hitscan: false,
        range_mm: 60_000,
        structure: 0,
        head_pct: 150,
        limb_pct: 50,
        magazine: 0,
        reload_ticks: 0,
        mag_slot: NO_MAG,
        draw_ticks: 0,
    };
    c.ammo[FIRE_ARROW as usize] = AmmoDef {
        speed_mmpt: 1333,
        drop_mmpt2: 11,
        damage_pct: 80,
        fire_ticks: [BURN, BURN],
    };
    w.combat = c;
    w.tick(&[Command::Join { id: 1 }]);
    let (x, z) = open_ground(&w);
    w.players[P].body = Body::at(w.seed, &w.haven, x, z);
    w.players[P].inv[0] = ItemStack {
        item: BOW,
        count: 1,
        cond: 0,
        skin: 0,
    };
    w.players[P].inv[7] = ItemStack {
        item: FIRE_ARROW,
        count: 8,
        cond: 0,
        skin: 0,
    };
    w
}

/// Dry land 200 m or more from the town, with nothing standing on it.
fn open_ground(w: &World) -> (f32, f32) {
    use sim_core::terrain::{self, Occupant};
    for i in 0..400 {
        let (x, z) = (w.haven.x + 200.0 + i as f32 * 3.0, w.haven.z + 40.0);
        let (cx, cz) = (
            (x / terrain::CELL_SIZE) as i32,
            (z / terrain::CELL_SIZE) as i32,
        );
        let clear = (cz - 1..=cz + 1).all(|oz| {
            (cx - 1..=cx + 1).all(|ox| {
                terrain::scatter(w.seed, &w.scatter, &w.haven, ox, oz).occupant == Occupant::None
            })
        });
        if clear && terrain::ground(w.seed, &w.haven, x, z) > terrain::SEA_LEVEL + 2.0 {
            return (x, z);
        }
    }
    panic!("this island drew no open ground east of the town");
}

/// Loose one arrow straight down at the archer's own feet, and tick until it
/// has landed. Returns the `EV_FIRE` payloads it raised.
fn loose_at_feet(w: &mut World) -> Vec<(u32, u32, u32)> {
    let mut fires = Vec::new();
    let frame = InputFrame {
        buttons: BTN_PRIMARY,
        pitch: 0,
        sel: 0,
        ..InputFrame::default()
    };
    w.tick(&[Command::Input {
        id: 1,
        frame,
        favour: 0,
    }]);
    let mut seen = |w: &World| {
        for e in w.events.entries().iter().filter(|e| e.code == EV_FIRE) {
            fires.push((e.a, e.b, e.c));
        }
    };
    seen(w);
    let frame = InputFrame {
        buttons: 0,
        seq: 1,
        ..frame
    };
    w.tick(&[Command::Input {
        id: 1,
        frame,
        favour: 0,
    }]);
    seen(w);
    for _ in 0..30 {
        if w.arrows.is_empty() {
            break;
        }
        w.tick(&[]);
        seen(w);
    }
    assert!(w.arrows.is_empty(), "the arrow never landed");
    fires
}

#[test]
fn a_fire_arrow_leaves_a_fire_and_no_arrow() {
    let mut w = world_with_fire_arrows();
    let lit = loose_at_feet(&mut w);

    assert!(w.spent.is_empty(), "nothing stands in a body");
    assert!(w.ground_items.is_empty(), "and nothing lies on the ground");
    assert_eq!(w.fires.len(), 1, "a fire burns where it landed");
    let f = w.fires.entries()[0];
    let feet = w.players[P].body;
    assert!(
        (f.qx - feet.qx).abs() <= 2 && (f.qz - feet.qz).abs() <= 2,
        "at the archer's feet ({}, {} against {}, {})",
        f.qx,
        f.qz,
        feet.qx,
        feet.qz
    );
    assert!((f.qy - feet.qy).abs() <= 3, "on the ground they stand on");
    assert_eq!(
        (f.owner, f.item),
        (w.players[P].id, BOW),
        "the archer's, with the bow"
    );

    assert_eq!(lit.len(), 1, "announced once");
    let (a, b, c) = lit[0];
    assert_eq!(fire_parts(a), (BURN, f.qx));
    assert_eq!((b as i32, c as i32), (f.qz, f.qy));
}

/// Mutant: a burn on every tick takes 60 a second; a burn that skipped the
/// archer would leave this body whole; a fire that never went out keeps
/// burning past ten seconds.
#[test]
fn the_fire_burns_two_a_second_until_it_goes_out() {
    let mut w = world_with_fire_arrows();
    loose_at_feet(&mut w);
    let f = w.fires.entries()[0];
    let hp0 = w.players[P].hp;
    // Every second on the world's stride until it goes out.
    while w.tick < f.until + 60 {
        w.tick(&[]);
    }
    let lit_at = f.until - u64::from(BURN);
    let expected = (lit_at + 1..f.until)
        .filter(|t| t.is_multiple_of(sim_core::fire::FIRE_PERIOD_TICKS))
        .count() as u16
        * sim_core::fire::FIRE_HP;
    assert!(w.fires.is_empty(), "it went out");
    assert_eq!(
        hp0 - w.players[P].hp,
        expected,
        "two a second for the ten seconds it burned, and no more"
    );
    assert!(expected >= 18, "{expected}");
}

#[test]
fn a_burning_fire_survives_a_save() {
    let mut w = world_with_fire_arrows();
    loose_at_feet(&mut w);
    let mut buf = vec![0u8; sim_core::worldsave::WORLD_SAVE_MAX_BYTES];
    let n = w.save_world(&mut buf).expect("a live world encodes");
    let mut back = Box::new(World::new(w.seed));
    back.gather = w.gather;
    back.combat = w.combat;
    back.load(&buf[..n]).expect("its own bytes must load");
    assert_eq!(back.fires.entries(), w.fires.entries());
}
