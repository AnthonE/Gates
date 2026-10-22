//! The Black Ziggurat: placed away from the town, on a carved floor, with
//! its route walkable — the front stairs to the summit, the ledges to the
//! blue and red readers — its doors shut until opened, and its crates on
//! their own rooms' floors.

// A test prints where it put things: host code, not sim code.
#![allow(
    clippy::disallowed_macros,
    clippy::disallowed_methods,
    clippy::disallowed_types
)]
use sim_core::{
    fmath::fabs,
    kit,
    monument::{self, Ziggurat},
    occupy::{Occupants, Pristine, SlotCache},
    terrain::{self, Haven, Occupant, ScatterTable},
};

const SEEDS: [u64; 12] = [
    1,
    2,
    7,
    42,
    99,
    1337,
    20_260_731,
    20_260_802,
    555_555,
    8_675_309,
    0xDEAD_BEEF,
    0x0BAD_C0DE,
];

fn occupants<'a>(
    h: &'a Haven,
    table: &'a ScatterTable,
    cache: &'a mut SlotCache,
    doors: u32,
) -> Occupants<'a> {
    Occupants {
        doors,
        table,
        haven: h,
        harvested: &Pristine,
        cache,
    }
}

#[test]
fn every_seed_places_a_ziggurat_far_from_the_town_on_a_flat_floor() {
    for seed in SEEDS {
        let h = terrain::haven(seed);
        let z = h.ziggurat;
        assert!(z.live, "seed {seed}: no ziggurat");
        let t = h.town;
        let d = ((z.x - t.x) * (z.x - t.x) + (z.z - t.z) * (z.z - t.z)).sqrt();
        println!(
            "seed {seed}: ziggurat ({:.0}, {:.0}) floor {:.1} rot {}  {d:.0} m from the town",
            z.x, z.z, z.floor_y, z.rot
        );
        assert!(
            d >= monument::ZIG_TOWN_CLEAR_M,
            "seed {seed}: {d} m from the town"
        );
        assert_eq!(z.x % monument::ZIG_SNAP_M, 0.0);
        assert_eq!(z.z % monument::ZIG_SNAP_M, 0.0);
        for gx in -19..=19 {
            for gz in -19..=19 {
                let (x, wz) = (z.x + gx as f32 * 2.0, z.z + gz as f32 * 2.0);
                let g = terrain::ground(seed, &h, x, wz);
                assert!(
                    fabs(g - z.floor_y) < 0.01,
                    "seed {seed}: {g} vs {}",
                    z.floor_y
                );
            }
        }
        assert!(!h
            .marks
            .iter()
            .any(|m| m.live && monument::covers(&z, m.x, m.z, 26.0)));
        assert!(terrain::build_reserved(&h, z.x, z.z, 0.0));
    }
}

/// Walk a body from local `from` to `to` in 0.1 m steps, stepping up onto
/// whatever is within a step and dropping onto whatever is below; returns
/// the feet at the end.
#[allow(clippy::too_many_arguments)]
fn walk(
    seed: u64,
    occ: &mut Occupants,
    z: &Ziggurat,
    from: (f32, f32),
    to: (f32, f32),
    mut feet: f32,
    what: &str,
) -> f32 {
    let p = z.placed();
    let n = (((to.0 - from.0).abs() + (to.1 - from.1).abs()) / 0.1) as i32;
    for i in 0..=n {
        let f = i as f32 / n.max(1) as f32;
        let (lx, lz) = (from.0 + (to.0 - from.0) * f, from.1 + (to.1 - from.1) * f);
        let (x, wz) = kit::to_world(&p, lx, lz);
        assert!(
            !occ.blocks(seed, x, wz, feet),
            "{what}: blocked at local ({lx:.1}, {lz:.1}) feet {:.2}",
            feet - z.floor_y
        );
        // Up a step, or down to whatever is below: a body falls.
        feet = occ.ground(seed, x, wz, feet).max(z.floor_y);
    }
    feet
}

#[test]
fn the_stairs_climb_to_the_summit_and_the_ledges_reach_the_readers() {
    for seed in [20_260_731u64, 42, 0xDEAD_BEEF] {
        let h = terrain::haven(seed);
        let z = h.ziggurat;
        let table = ScatterTable::alpha_default();
        let mut cache = SlotCache::new();
        let mut occ = occupants(&h, &table, &mut cache, 0);
        let f0 = z.floor_y;
        // Up the front, all five flights.
        let top = walk(
            seed,
            &mut occ,
            &z,
            (0.0, -40.0),
            (0.0, -6.0),
            f0,
            "front stairs",
        );
        assert!(
            fabs(top - (f0 + 25.0)) < 0.01,
            "seed {seed}: summit at {}",
            top - f0
        );
        // From the first landing round the east ledge to the blue reader,
        // stepping off the stairs before the next flight.
        let mut feet = walk(
            seed,
            &mut occ,
            &z,
            (0.0, -40.0),
            (0.0, -31.5),
            f0,
            "flight 0",
        );
        assert!(fabs(feet - (f0 + 5.0)) < 0.01);
        feet = walk(
            seed,
            &mut occ,
            &z,
            (0.0, -31.5),
            (5.0, -31.5),
            feet,
            "the landing",
        );
        feet = walk(
            seed,
            &mut occ,
            &z,
            (5.0, -31.5),
            (29.0, -29.0),
            feet,
            "south ledge",
        );
        let (rx, ry, rz) = monument::DOORS[1].reader;
        feet = walk(
            seed,
            &mut occ,
            &z,
            (29.0, -29.0),
            (rx, rz),
            feet,
            "east ledge",
        );
        assert!(
            fabs(feet - (f0 + ry)) < 0.01,
            "seed {seed}: blue reader level {}",
            feet - f0
        );
        // From the third landing round the north ledge to the red reader.
        let mut feet = walk(
            seed,
            &mut occ,
            &z,
            (0.0, -40.0),
            (0.0, -19.5),
            f0,
            "flights 0-2",
        );
        assert!(fabs(feet - (f0 + 15.0)) < 0.01);
        feet = walk(
            seed,
            &mut occ,
            &z,
            (0.0, -19.5),
            (5.0, -19.5),
            feet,
            "the landing",
        );
        feet = walk(
            seed,
            &mut occ,
            &z,
            (5.0, -19.5),
            (5.0, -17.0),
            feet,
            "onto the ledge",
        );
        feet = walk(
            seed,
            &mut occ,
            &z,
            (5.0, -17.0),
            (17.0, -17.0),
            feet,
            "ledge 2 south",
        );
        feet = walk(
            seed,
            &mut occ,
            &z,
            (17.0, -17.0),
            (17.0, 17.0),
            feet,
            "ledge 2 east",
        );
        let (rx, ry, rz) = monument::DOORS[2].reader;
        feet = walk(
            seed,
            &mut occ,
            &z,
            (17.0, 17.0),
            (rx, rz),
            feet,
            "ledge 2 north",
        );
        assert!(
            fabs(feet - (f0 + ry)) < 0.01,
            "seed {seed}: red reader level"
        );
        // The portal into the entry hall, to the green reader.
        let (rx, _, rz) = monument::DOORS[0].reader;
        walk(seed, &mut occ, &z, (0.0, 40.0), (rx, rz), f0, "portal");
    }
}

#[test]
fn a_shut_door_stops_a_body_and_an_open_one_does_not() {
    let seed = 20_260_731;
    let h = terrain::haven(seed);
    let z = h.ziggurat;
    let table = ScatterTable::alpha_default();
    let rooms = [
        // (door, outside, inside, feet)
        (0usize, (0.0, 14.0), (0.0, 6.0), 0.0),
        (1, (27.0, 0.0), (8.0, 0.0), 5.0),
        (2, (-8.0, 16.0), (-8.0, 2.0), 15.0),
    ];
    for (d, out, inside, y) in rooms {
        let mut cache = SlotCache::new();
        let mut occ = occupants(&h, &table, &mut cache, 0);
        let (bx, bz) = {
            let b = monument::door_box(d).unwrap();
            ((b[0] + b[3]) * 0.5, (b[2] + b[5]) * 0.5)
        };
        let (x, wz) = kit::to_world(&z.placed(), bx, bz);
        assert!(
            occ.blocks(seed, x, wz, z.floor_y + y),
            "door {d} open while shut"
        );
        let mut cache = SlotCache::new();
        let mut occ = occupants(&h, &table, &mut cache, 1 << d);
        walk(
            seed,
            &mut occ,
            &z,
            out,
            inside,
            z.floor_y + y,
            "through the open door",
        );
        assert!(!monument::in_doorway(
            &z,
            d,
            x + 5.0,
            wz + 5.0,
            z.floor_y + y
        ));
        assert!(monument::in_doorway(&z, d, x, wz, z.floor_y + y));
    }
}

#[test]
fn the_crates_sit_on_their_rooms_floors() {
    let seed = 20_260_731;
    let h = terrain::haven(seed);
    let z = h.ziggurat;
    let table = ScatterTable::alpha_default();
    let mut tiers = [0; 3];
    for k in 0..monument::CRATES.len() {
        let (x, y, wz, _, occ) = monument::crate_world(&z, k).unwrap();
        let s = terrain::scatter(
            seed,
            &table,
            &h,
            (x / terrain::CELL_SIZE) as i32,
            (wz / terrain::CELL_SIZE) as i32,
        );
        assert_eq!(s.occupant, occ, "crate {k}");
        assert_eq!((s.x, s.y, s.z), (x, y, wz), "crate {k}");
        match occ {
            Occupant::GreenCrate => tiers[0] += 1,
            Occupant::BlueCrate => tiers[1] += 1,
            Occupant::EliteCrate => tiers[2] += 1,
            _ => {}
        }
    }
    assert_eq!(tiers, [1, 1, 2]);
}

/// The exit lever opens its door with no card from inside — and not from
/// outside through the shut leaf. The blue lever (local 21, 0) is 2.8 m
/// from a body pressed against the leaf's outer face (x 22.6..23.4), inside
/// `SWIPE_REACH_M`, so reach alone opened the room to anyone.
#[test]
fn the_exit_lever_is_not_pulled_through_its_shut_door() {
    use sim_core::monument::REFUSE_S_REACH;
    use sim_core::movement::{quant_xz, quant_y, Body};
    use sim_core::world::{Command, World, EV_SWIPE, EV_SWIPE_REFUSED};
    let seed = 20_260_731;
    let mut w = World::new(seed);
    w.tick(&[Command::Join { id: 1 }]);
    let z = w.haven.ziggurat;
    assert!(z.live);
    let stand = |w: &mut World, lx: f32, lz: f32| {
        let (x, wz) = kit::to_world(&z.placed(), lx, lz);
        w.players[0].body = Body {
            qx: quant_xz(x),
            qy: quant_y(z.floor_y + 5.0),
            qz: quant_xz(wz),
            qvy: 0,
            grounded: true,
        };
    };
    stand(&mut w, 23.8, 0.0);
    w.tick(&[Command::Swipe { id: 1, door: 1 }]);
    assert!(
        w.events
            .entries()
            .iter()
            .any(|e| e.code == EV_SWIPE_REFUSED && e.b == REFUSE_S_REACH),
        "the lever is not pulled through the shut leaf"
    );
    assert_eq!(w.card_door_bits & 2, 0, "and the blue door stays shut");
    stand(&mut w, 21.5, 0.0);
    w.tick(&[Command::Swipe { id: 1, door: 1 }]);
    assert!(
        w.events
            .entries()
            .iter()
            .any(|e| e.code == EV_SWIPE && e.c == 1),
        "inside, the lever needs no card"
    );
    assert_eq!(w.card_door_bits & 2, 2, "and the door opens");
}

/// Only a live, awake body holds a card door open. A sleeper or a corpse
/// in the doorway held it for the rest of the wipe; now the door shuts and
/// puts the body out at its reader, so nobody is left inside a shut leaf
/// (where `movement::step`'s stuck-inside lift would let it walk through).
/// A sleeper woken after a restart — `card_doors` is not saved, so every
/// door is shut — is put out the same way.
#[test]
fn a_sleeper_or_a_corpse_does_not_hold_a_card_door() {
    use sim_core::movement::{quant_xz, quant_y, Body};
    use sim_core::world::{Command, World};
    let seed = 20_260_731;
    let mut w = World::new(seed);
    w.tick(&[Command::Join { id: 1 }]);
    let z = w.haven.ziggurat;
    assert!(z.live);
    let b = monument::door_box(0).unwrap();
    let (x, wz) = kit::to_world(&z.placed(), (b[0] + b[3]) * 0.5, (b[2] + b[5]) * 0.5);
    let in_door = Body {
        qx: quant_xz(x),
        qy: quant_y(z.floor_y),
        qz: quant_xz(wz),
        qvy: 0,
        grounded: true,
    };
    let (rx, _, rz) = monument::reader_world(&z, 0, false).unwrap();
    let at_reader = |w: &World| {
        let p = &w.players[0].body;
        (p.qx, p.qz) == (quant_xz(rx), quant_xz(rz))
    };
    let open_for = |w: &mut World, ticks: u64| {
        w.card_doors[0] = w.tick + ticks;
        w.players[0].body = in_door;
    };

    // Awake: the door waits on it.
    open_for(&mut w, 5);
    for _ in 0..40 {
        w.tick(&[]);
    }
    assert_eq!(
        w.card_door_bits & 1,
        1,
        "the door shut on a body awake in it"
    );

    // Dead and awake (on the death screen; `die` keeps it active where it
    // fell): the door shuts and the corpse is put out at the reader.
    open_for(&mut w, 5);
    w.players[0].dead = true;
    assert!(!w.players[0].sleeping, "setup: an awake corpse");
    for _ in 0..40 {
        w.tick(&[]);
    }
    assert_eq!(w.card_door_bits & 1, 0, "an awake corpse held the door");
    assert!(at_reader(&w), "the awake corpse was left in the shut leaf");
    w.players[0].dead = false;

    // Asleep: it shuts, and the body is put out at the reader.
    w.tick(&[Command::Leave { id: 1 }]);
    open_for(&mut w, 5);
    for _ in 0..40 {
        w.tick(&[]);
    }
    assert_eq!(w.card_door_bits & 1, 0, "a sleeper held the door");
    assert!(at_reader(&w), "the sleeper was left in the shut leaf");

    // Dead and asleep (a killed sleeper): the same.
    open_for(&mut w, 5);
    w.players[0].dead = true;
    for _ in 0..40 {
        w.tick(&[]);
    }
    assert_eq!(w.card_door_bits & 1, 0, "a corpse held the door");
    assert!(at_reader(&w), "the corpse was left in the shut leaf");

    // A restart: the sleeper is back in the doorway and every door is shut,
    // as a loaded world has them. Waking puts it out.
    w.players[0].dead = false;
    w.players[0].body = in_door;
    w.card_doors = [0; monument::CARD_DOORS];
    w.card_door_bits = 0;
    w.tick(&[Command::Wake {
        id: 0x0101,
        sleeper: 1,
    }]);
    assert!(!w.players[0].sleeping, "setup: the takeover woke the body");
    assert!(at_reader(&w), "a woken sleeper was left in the shut leaf");
}

/// A body killed in a card doorway leaves its bag at the reader with the
/// corpse, not in the leaf: the door shuts on a corpse a second later, a
/// bag's address never moves, and one dropped in the leaf was sealed in
/// the stone (no hand reached it from either side until the door opened).
#[test]
fn a_bag_dropped_in_a_card_doorway_is_not_sealed_in_its_leaf() {
    use sim_core::backpack::BackpackContent;
    use sim_core::combat::CombatContent;
    use sim_core::gather::ItemStack;
    use sim_core::limits::TICK_HZ;
    use sim_core::movement::{quant_xz, quant_y, Body};
    use sim_core::survival::SurvivalContent;
    use sim_core::world::{Command, World};
    let seed = 20_260_731;
    let mut w = World::new(seed);
    w.combat = CombatContent::probe_fixture();
    w.survival = SurvivalContent::probe_fixture();
    w.backpack = BackpackContent::probe_fixture();
    w.tick(&[Command::Join { id: 1 }]);
    let z = w.haven.ziggurat;
    assert!(z.live);
    let b = monument::door_box(0).unwrap();
    let (x, wz) = kit::to_world(&z.placed(), (b[0] + b[3]) * 0.5, (b[2] + b[5]) * 0.5);
    let at = |x: f32, y: f32, wz: f32| Body {
        qx: quant_xz(x),
        qy: quant_y(y),
        qz: quant_xz(wz),
        qvy: 0,
        grounded: true,
    };
    // Awake in the open doorway (it holds the door), carrying something,
    // and starving: the clock kills it there.
    w.card_doors[0] = w.tick + 5;
    w.players[0].body = at(x, z.floor_y, wz);
    w.players[0].inv[0] = ItemStack {
        item: 0,
        count: 1,
        cond: 0,
        skin: 0,
    };
    w.players[0].food = 0;
    w.players[0].water = 0;
    for _ in 0..120 * TICK_HZ {
        w.tick(&[]);
        if w.players[0].dead {
            break;
        }
    }
    assert!(w.players[0].dead, "setup: the clock never took the body");
    for _ in 0..40 {
        w.tick(&[]);
    }
    assert_eq!(
        w.card_door_bits & 1,
        0,
        "setup: the door shut on the corpse"
    );
    let bag = w
        .backpacks
        .entries()
        .iter()
        .position(|r| r.owner == 1)
        .expect("setup: the body dropped a bag");

    // A hand at the reader, the door shut, reaches the bag.
    w.tick(&[Command::Join { id: 2 }]);
    let s = w
        .players
        .iter()
        .position(|p| p.active && p.id == 2)
        .unwrap();
    let (rx, ry, rz) = monument::reader_world(&z, 0, false).unwrap();
    w.players[s].body = at(rx, ry, rz);
    assert!(
        w.bag_reachable(bag, s),
        "the bag was sealed in the shut leaf"
    );
    let r = w.backpacks.entries()[bag];
    assert_eq!(
        w.players[0].death_at,
        Some((r.qx, r.qz)),
        "the death marker is where the bag lies"
    );
}
