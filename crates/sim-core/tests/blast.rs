//! The satchel's blast — an area, not an address (satchel blast v0;
//! `charge.rs`'s module header carries the design).
//!
//! What this file pins, in the order the slice argued it: the planted wall
//! still takes the FULL structure number (anchor 1's arithmetic is
//! undisturbed), a neighbouring wall takes the falloff's share, a body at
//! the epicentre dies with `DEATH_BY_CHARGE` and the planter's name on it,
//! the planter is not spared from their own bomb, a bystander at the edge
//! is hurt and not killed, and a save/load round trip mid-fuse keeps the
//! blast's two copied-at-plant numbers — the fields format 4 exists for.

#![allow(clippy::disallowed_macros)]

use sim_core::build::{foundation_terrain_ok, BuildContent, BUILD_CELL_M, LOC_EDGE_XLO, LOC_PLANE};
use sim_core::combat::CombatContent;
use sim_core::deploy::DeployContent;
use sim_core::gather::{GatherContent, ItemStack};
use sim_core::movement::Body;
use sim_core::world::{Command, World, DEATH_BY_CHARGE};
use sim_core::worldsave::WORLD_SAVE_MAX_BYTES;

/// The solved authored sites for `seed` — what `terrain::ground` needs in order
/// to know where the carve is.
///
/// Memoized per seed, and that is not premature: `terrain::haven` is a few
/// thousand `height` taps (a shoreline march, a bisect and a rosette per
/// candidate bearing), these suites call it from inside assertion loops, and
/// the first draft of this helper resolved it per call and took the workspace
/// test run past five minutes. It is a pure function of the seed, so caching
/// cannot change a result.
fn hv(seed: u64) -> &'static sim_core::terrain::Haven {
    use std::cell::RefCell;
    // A thread-local rather than a `Mutex`: `std::sync::Mutex` is on
    // `sim-core/clippy.toml`'s disallowed list (wall 3), and that list is
    // crate-scoped, so it binds this suite too. Per-thread is the right shape
    // anyway — the cache exists to stop a per-assertion recompute, not to be
    // shared.
    thread_local! {
        static CACHE: RefCell<Vec<(u64, &'static sim_core::terrain::Haven)>> =
            const { RefCell::new(Vec::new()) };
    }
    let hit = CACHE.with(|c| c.borrow().iter().find(|(s, _)| *s == seed).map(|&(_, h)| h));
    if let Some(h) = hit {
        return h;
    }
    let h: &'static sim_core::terrain::Haven = Box::leak(Box::new(sim_core::terrain::haven(seed)));
    CACHE.with(|c| c.borrow_mut().push((seed, h)));
    h
}

const SEED: u64 = 0xB1A57;
const RAIDER: u32 = 3;
const BYSTANDER: u32 = 4;

/// The satchel row in the combat probe fixture. `CombatContent::
/// probe_fixture` arms `throw` on every item with modest numbers; this
/// test wants real ones, so it writes its own row the way `hunt_world`
/// writes gaits.
const SATCHEL: u16 = 9;

fn cell_center(cx: u16, cz: u16) -> (f32, f32) {
    (
        (cx as f32 + 0.5) * BUILD_CELL_M,
        (cz as f32 + 0.5) * BUILD_CELL_M,
    )
}

fn buildable_cell(seed: u64) -> (u16, u16) {
    for r in 0..64i32 {
        for dz in -r..=r {
            for dx in -r..=r {
                if dx.abs() != r && dz.abs() != r {
                    continue;
                }
                let cx = (512 + dx).clamp(0, 1023) as u16;
                let cz = (512 + dz).clamp(0, 1023) as u16;
                let (x, z) = cell_center(cx, cz);
                if foundation_terrain_ok(seed, hv(seed), x, z) {
                    return (cx, cz);
                }
            }
        }
    }
    panic!("no buildable cell");
}

/// A world with a foundation, a low-x wall, a raider holding one satchel
/// (475 body / 125 structure / 3 m blast / 2 s fuse — the shipped row's
/// shape at a test-sized fuse), and a bystander. Returns (world, cx, cz).
fn raid_world() -> (World, u16, u16) {
    let mut w = World::new(SEED);
    w.gather = GatherContent::probe_fixture();
    w.build = BuildContent::probe_fixture();
    // Tall hp, written before placement copies it onto the records: the
    // probe rows die outright to one satchel, and a removed piece has no
    // hp left to read the falloff arithmetic off.
    w.build.pieces[0].hp = 1000;
    w.build.pieces[1].hp = 1000;
    w.deploy = DeployContent::probe_fixture();
    let mut cc = CombatContent::probe_fixture();
    cc.throw[SATCHEL as usize] = sim_core::combat::ThrowDef {
        damage: 475,
        structure: 125,
        fuse_ticks: 60,
        reach_cm: 200,
        blast_cm: 300,
    };
    w.combat = cc;

    let (cx, cz) = buildable_cell(SEED);
    let (x, z) = cell_center(cx, cz);
    w.dev_spawn = Some((x, z));
    w.tick(&[Command::Join { id: RAIDER }]);
    w.tick(&[Command::Join { id: BYSTANDER }]);
    w.players[0].body = Body::at(SEED, hv(SEED), x, z);
    // The bystander starts far outside every blast in this file.
    w.players[1].body = Body::at(SEED, hv(SEED), x + 60.0, z);

    w.players[0].inv[0] = ItemStack {
        item: 0,
        count: 20,
        cond: 0,
    };
    w.players[0].inv[1] = ItemStack {
        item: SATCHEL,
        count: 2,
        cond: 0,
    };
    w.tick(&[Command::Place {
        id: RAIDER,
        row: 0,
        cx,
        cz,
        level: 0,
        loc: LOC_PLANE,
        freehand: false,
    }]);
    w.tick(&[Command::Place {
        id: RAIDER,
        row: 1,
        cx,
        cz,
        level: 0,
        loc: LOC_EDGE_XLO,
        freehand: false,
    }]);
    assert_eq!(w.pieces.len(), 2, "foundation + wall");
    (w, cx, cz)
}

fn piece_hp(w: &World, cx: u16, cz: u16, level: u8, loc: u8) -> Option<u16> {
    w.pieces
        .entries()
        .iter()
        .find(|p| p.cx == cx && p.cz == cz && p.level == level && p.loc == loc)
        .map(|p| p.hp)
}

/// Plant on the wall with slot 1 selected (the satchel). The raider must
/// be in build reach when this runs; reposition AFTER planting.
fn plant(w: &mut World, cx: u16, cz: u16, loc: u8) {
    // Select the satchel: hotbar sel 1 rides the input frame.
    w.tick(&[Command::Input {
        id: RAIDER,
        frame: sim_core::input::InputFrame {
            seq: 900,
            sel: 1,
            ..sim_core::input::InputFrame::default()
        },
        favour: 0,
    }]);
    w.tick(&[Command::Throw {
        id: RAIDER,
        deploy: false,
        cx,
        cz,
        level: 0,
        loc,
    }]);
    assert_eq!(w.charges.len(), 1, "the plant must take");
}

/// Run the fuse out plus a tick.
fn wait_out(w: &mut World) {
    for _ in 0..62 {
        w.tick(&[]);
    }
    assert_eq!(w.charges.len(), 0, "the fuse must have run out");
}

/// The planted wall takes the full structure number — the falloff's zero
/// point is the anchor, so anchor 1's `piece hp ÷ structure` arithmetic
/// is exactly what it was before the blast existed. The foundation one
/// half-cell away takes a falloff share: hurt, less than the wall.
#[test]
fn the_planted_wall_takes_full_structure_and_the_floor_a_share() {
    let (mut w, cx, cz) = raid_world();
    let wall_before = piece_hp(&w, cx, cz, 0, LOC_EDGE_XLO).unwrap();
    let floor_before = piece_hp(&w, cx, cz, 0, LOC_PLANE).unwrap();
    plant(&mut w, cx, cz, LOC_EDGE_XLO);
    // The raider steps away during the fuse, so the body numbers stay out
    // of this test — which is also the verb working as designed.
    w.players[0].body = Body::at(
        SEED,
        hv(SEED),
        cell_center(cx, cz).0 + 20.0,
        cell_center(cx, cz).1,
    );
    wait_out(&mut w);

    let wall_after = piece_hp(&w, cx, cz, 0, LOC_EDGE_XLO).unwrap();
    let floor_after = piece_hp(&w, cx, cz, 0, LOC_PLANE).unwrap();
    assert_eq!(
        wall_before - wall_after,
        125,
        "the planted wall takes the whole structure number"
    );
    let floor_hit = floor_before - floor_after;
    assert!(
        floor_hit > 0 && floor_hit < 125,
        "the floor a half-cell away takes a falloff share, took {floor_hit}"
    );
}

/// A body standing at the wall dies to the blast, the cause is the
/// charge's, the killer is the planter, and the planter standing there
/// too is not spared — both corpses, one bomb.
#[test]
fn the_blast_kills_at_the_epicentre_and_spares_nobody() {
    let (mut w, cx, cz) = raid_world();
    let (x, z) = cell_center(cx, cz);
    // The bystander stands at the wall; the raider plants and then stays
    // close enough to die too (both inside the lethal ~2.4 m of a
    // 475-damage 3 m blast).
    w.players[1].body = Body::at(SEED, hv(SEED), x - 1.4, z);
    plant(&mut w, cx, cz, LOC_EDGE_XLO);
    w.players[0].body = Body::at(SEED, hv(SEED), x - 1.0, z + 0.5);
    wait_out(&mut w);

    assert!(w.players[1].dead, "the bystander at the wall dies");
    assert_eq!(w.players[1].death_cause, DEATH_BY_CHARGE);
    assert_eq!(
        w.players[1].death_by, w.players[0].id,
        "the killer is the planter"
    );
    assert!(
        w.players[0].dead,
        "the planter is not spared from their own bomb"
    );
    assert_eq!(w.players[0].death_cause, DEATH_BY_CHARGE);
    assert_eq!(
        w.players[0].death_by, w.players[0].id,
        "a self-kill names the self — the death screen's sentence needs it"
    );
}

/// A body near the blast's edge is hurt, not killed: the falloff reaches
/// zero at the radius, so standing at 2.9 m of a 3 m blast costs a
/// scratch and standing outside costs nothing.
#[test]
fn the_edge_of_the_blast_scratches() {
    let (mut w, cx, cz) = raid_world();
    let (x, z) = cell_center(cx, cz);
    let wall_x = cx as f32 * BUILD_CELL_M;
    // 2.9 m from the wall's anchor (the edge's midpoint), planar.
    w.players[1].body = Body::at(SEED, hv(SEED), wall_x + 2.9, z);
    let full = w.players[1].hp;
    plant(&mut w, cx, cz, LOC_EDGE_XLO);
    w.players[0].body = Body::at(SEED, hv(SEED), x + 20.0, z);
    wait_out(&mut w);
    assert!(
        !w.players[1].dead,
        "2.9 m of a 3 m blast must not kill a full body"
    );
    assert!(
        w.players[1].hp < full,
        "2.9 m of a 3 m blast must not be free either"
    );
}

/// A save written mid-fuse round-trips the blast's two copied-at-plant
/// numbers — the fields WORLD_SAVE_FORMAT 4 exists for. The loaded world
/// detonates with the same damage the saved one would have.
#[test]
fn a_fuse_survives_a_save_and_still_blasts() {
    let (mut w, cx, cz) = raid_world();
    let (x, z) = cell_center(cx, cz);
    w.players[0].body = Body::at(SEED, hv(SEED), x + 20.0, z);
    w.tick(&[Command::Input {
        id: RAIDER,
        frame: sim_core::input::InputFrame {
            seq: 900,
            sel: 1,
            ..sim_core::input::InputFrame::default()
        },
        favour: 0,
    }]);
    // Plant from in reach, then step away before the save.
    w.players[0].body = Body::at(SEED, hv(SEED), x - 1.0, z);
    w.tick(&[Command::Throw {
        id: RAIDER,
        deploy: false,
        cx,
        cz,
        level: 0,
        loc: LOC_EDGE_XLO,
    }]);
    assert_eq!(w.charges.len(), 1);
    let rec = w.charges.entries()[0];
    assert_eq!((rec.damage, rec.blast_cm), (475, 300), "copied at plant");
    w.players[0].body = Body::at(SEED, hv(SEED), x + 20.0, z);

    let mut buf = vec![0u8; WORLD_SAVE_MAX_BYTES];
    let n = w.save_world(&mut buf).expect("save mid-fuse");
    let mut w2 = World::new(SEED);
    w2.gather = GatherContent::probe_fixture();
    w2.build = BuildContent::probe_fixture();
    w2.deploy = DeployContent::probe_fixture();
    w2.combat = w.combat;
    w2.load(&buf[..n]).expect("load mid-fuse");
    let rec2 = w2.charges.entries()[0];
    assert_eq!(
        (rec2.damage, rec2.blast_cm, rec2.fires_at),
        (rec.damage, rec.blast_cm, rec.fires_at),
        "format 4 carries the blast whole"
    );
    let wall_before = piece_hp(&w2, cx, cz, 0, LOC_EDGE_XLO).unwrap();
    for _ in 0..70 {
        w2.tick(&[]);
    }
    let wall_after = piece_hp(&w2, cx, cz, 0, LOC_EDGE_XLO).unwrap();
    assert_eq!(
        wall_before - wall_after,
        125,
        "the loaded fuse detonates with the saved numbers"
    );
}
