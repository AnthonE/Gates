//! Deployables as movement collision, and tops as ground — the two halves
//! of "the world is solid" that landed last (deploy collision v0;
//! `NOW.md` §0n2 "no deployable blocks movement" and §0q item 3 "you
//! cannot stand ON anything").
//!
//! Three layers, `walk.rs`/`solid.rs`'s split applied twice over:
//!
//! 1. **Geometry** — `terrain::slot_ground` on hand-built slots: which
//!    occupant tops are ground, and what the lid rule refuses.
//! 2. **Mechanics** — `movement::step` against a raw `ColIndex` with a
//!    solid nibble set by hand: the veto, the veto-lift escape, the jump
//!    onto a box top, the clear.
//! 3. **Wiring** — the real verbs on a real `World`: `PlaceDeploy` walls
//!    the cell, `Demolish` frees it, and a save/load round trip keeps the
//!    wall (`rebuild_doors`' solid half).
//!
//! Plus the one walk this file makes on live terrain: through the haven
//! shelter's doorway and onto its plinth, which `terrain.rs` said nothing
//! made a body stand on.

// The measurements are the gate's output — `tests/solid.rs`'s allow and
// reason: the L5 wall bans format/print in SIM code; a harness is not sim.
#![allow(clippy::disallowed_macros)]

use sim_core::build::{foundation_terrain_ok, BuildContent, BUILD_CELL_M, LOC_PLANE};
use sim_core::collide::{self, ColIndex, CAPSULE_RADIUS_M, NO_SURFACE};
use sim_core::deploy::{
    solid_vol, DeployContent, DeployDef, ARCH_BAG, ARCH_BOX, ARCH_FURNACE, DEPLOY_VOL,
    PLACE_FOUNDATION,
};
use sim_core::gather::{GatherContent, ItemStack};
use sim_core::input::{InputFrame, BTN_JUMP};
use sim_core::movement::{self, Body, POS_XZ_Q, POS_Y_Q, STEP_UP};
use sim_core::occupy::Scratch;
use sim_core::terrain::{
    self, Occupant, Slot, CELLS_PER_SIDE, CELL_SIZE, OCCUPANT_R_M, OCCUPANT_TOP_M, SHELTER_BOXES,
};
use sim_core::world::{Command, World};
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

const SEED: u64 = 0x50_11D0;
const PLAYER: u32 = 3;

/// Row 4 in the deploy fixture: a furnace (solid). Row 5: a bag (not).
const FURNACE_ROW: u16 = 4;
const BAG_ROW: u16 = 5;
const FURNACE_ITEM: u16 = 6;
const BAG_ITEM: u16 = 7;
const FOUNDATION_ROW: u16 = 0;

fn fixture() -> DeployContent {
    let mut d = DeployContent::probe_fixture();
    d.defs[4] = DeployDef {
        arch: ARCH_FURNACE,
        placement: PLACE_FOUNDATION,
        hp: 100,
        item: FURNACE_ITEM,
        ..DeployDef::INERT
    };
    d.defs[5] = DeployDef {
        arch: ARCH_BAG,
        placement: PLACE_FOUNDATION,
        hp: 50,
        item: BAG_ITEM,
        ..DeployDef::INERT
    };
    d.def_count = 6;
    d
}

fn cell_center(cx: u16, cz: u16) -> (f32, f32) {
    (
        (cx as f32 + 0.5) * BUILD_CELL_M,
        (cz as f32 + 0.5) * BUILD_CELL_M,
    )
}

/// `box_container.rs`'s search: nearest buildable cell to the island
/// centre, ring order so it is deterministic per seed.
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
    panic!("no buildable cell within 64 cells of centre");
}

fn pos(b: &Body) -> (f32, f32, f32) {
    (
        b.qx as f32 * POS_XZ_Q,
        b.qy as f32 * POS_Y_Q,
        b.qz as f32 * POS_XZ_Q,
    )
}

/// `f32::abs` is walled everywhere, tests included (`tests/solid.rs`'s
/// note): a magnitude is the max of the signed pair.
fn mag(v: f32) -> f32 {
    v.max(-v)
}

fn walk(yaw: u16, buttons: u8) -> InputFrame {
    InputFrame {
        seq: 1,
        buttons,
        yaw,
        move_z: 127,
        ..InputFrame::default()
    }
}

// ---------------------------------------------------------------- geometry

fn slot_at_origin(occupant: Occupant) -> Slot {
    Slot {
        occupant,
        x: 0.0,
        y: 0.0,
        z: 0.0,
        yaw: 0,
        scale: 1.0,
    }
}

/// A rock's top is ground when the feet can reach it and refused when the
/// lid says it is a wall face; a tree's crown is never reachable from its
/// own floor. The lid rule IS the whole difference between "standable"
/// and "you sink through the world" here, so both directions are pinned.
#[test]
fn occupant_tops_obey_the_lid() {
    let rock = slot_at_origin(Occupant::Rock);
    let rock_top = OCCUPANT_TOP_M[Occupant::Rock as usize];
    let rock_r = OCCUPANT_R_M[Occupant::Rock as usize];
    // Feet just under the top (a landing jump arc): standable.
    let g = terrain::slot_ground(&rock, 0.0, 0.0, rock_top - 0.1);
    assert!(
        mag(g - rock_top) <= 1e-6,
        "rock top {rock_top} should be ground for feet at {}, got {g}",
        rock_top - 0.1
    );
    // Feet on the floor: the top is more than STEP_UP up — a wall face.
    if rock_top > STEP_UP {
        let g = terrain::slot_ground(&rock, 0.0, 0.0, 0.0);
        assert_eq!(
            g, NO_SURFACE,
            "a {rock_top} m top is not ground from the floor"
        );
    }
    // Outside the footprint: nothing.
    let g = terrain::slot_ground(&rock, rock_r + 0.05, 0.0, rock_top - 0.1);
    assert_eq!(g, NO_SURFACE, "ground past the footprint edge");

    // The tree: its crown top is never reachable from its own floor.
    let tree = slot_at_origin(Occupant::Tree);
    let g = terrain::slot_ground(&tree, 0.0, 0.0, 0.0);
    assert_eq!(
        g, NO_SURFACE,
        "a tree top must never be ground from the floor"
    );
}

/// The shelter's plinth: +0.2 above the slot's ground, inside the 7×7
/// footprint — the exact surface `terrain.rs`'s plinth note said nothing
/// stood on. And the roof is NOT ground from the floor (the lid again).
#[test]
fn the_plinth_is_ground_and_the_roof_is_not() {
    let s = slot_at_origin(Occupant::HavenShelter);
    let plinth_top = SHELTER_BOXES[0][1] + SHELTER_BOXES[0][4] * 0.5;
    assert!(
        mag(plinth_top - 0.2) < 1e-6,
        "the plinth top moved: {plinth_top}"
    );
    let g = terrain::slot_ground(&s, 0.0, 0.0, 0.0);
    assert!(
        mag(g - plinth_top) <= 1e-6,
        "standing in the shelter, ground should be the plinth top {plinth_top}, got {g}"
    );
    // From the plinth, the 4 m roof is a ceiling, not a lift.
    assert!(
        g < 1.0,
        "the roof must not be the answer from floor height: {g}"
    );
}

// --------------------------------------------------------------- mechanics

/// A furnace nibble vetoes the walk at the inflated volume edge; clearing
/// it frees the cell. Raw `ColIndex`, barren occupants — nothing else can
/// take credit.
#[test]
fn a_solid_nibble_blocks_and_clears() {
    let mut occ = Scratch::barren();
    let mut cols = Box::new(ColIndex::new());
    let (cx, cz) = buildable_cell(SEED);
    let (fx, fz) = cell_center(cx, cz);
    cols.set_solid(cx, cz, 0, Some(ARCH_FURNACE));

    // Start one cell south, walk north (+Z is yaw 0).
    let mut b = Body::at(SEED, hv(SEED), fx, fz - 2.5);
    for _ in 0..240 {
        movement::step(
            SEED,
            hv(SEED),
            &cols,
            &mut occ.occupants(),
            &mut b,
            &walk(0, 0),
        );
    }
    let (_, _, z) = pos(&b);
    let (_, h, hd) = solid_vol(ARCH_FURNACE).expect("furnace is solid");
    assert!(h > 0.0);
    assert!(
        z <= fz - hd - CAPSULE_RADIUS_M + 2.0 * POS_XZ_Q,
        "furnace failed to block: z {z} vs face {}",
        fz - hd - CAPSULE_RADIUS_M
    );
    assert!(z > fz - 2.0, "the walk never approached the furnace");

    cols.set_solid(cx, cz, 0, None);
    for _ in 0..240 {
        movement::step(
            SEED,
            hv(SEED),
            &cols,
            &mut occ.occupants(),
            &mut b,
            &walk(0, 0),
        );
    }
    assert!(
        pos(&b).2 > fz + 1.0,
        "a cleared solid must stop blocking: z {}",
        pos(&b).2
    );
}

/// A body the furnace was placed AROUND walks out — being inside is never
/// absorbing (the occupant escape, applied to deploys).
#[test]
fn a_body_inside_a_solid_escapes() {
    let mut occ = Scratch::barren();
    let mut cols = Box::new(ColIndex::new());
    let (cx, cz) = buildable_cell(SEED);
    let (fx, fz) = cell_center(cx, cz);
    let mut b = Body::at(SEED, hv(SEED), fx, fz);
    cols.set_solid(cx, cz, 0, Some(ARCH_FURNACE));
    for _ in 0..240 {
        movement::step(
            SEED,
            hv(SEED),
            &cols,
            &mut occ.occupants(),
            &mut b,
            &walk(0, 0),
        );
    }
    let (_, _, z) = pos(&b);
    assert!(
        z > fz + 1.0,
        "a body placed inside must be able to walk out: z {z} vs centre {fz}"
    );
}

/// Jumping at a box lands ON it: the top is ground the vertical pass
/// snaps to, not a shell the body falls through into the volume.
#[test]
fn a_jump_lands_on_the_box_top() {
    let mut occ = Scratch::barren();
    let mut cols = Box::new(ColIndex::new());
    let (cx, cz) = buildable_cell(SEED);
    let (fx, fz) = cell_center(cx, cz);
    cols.set_solid(cx, cz, 0, Some(ARCH_BOX));
    let terr = terrain::height(SEED, fx, fz);
    let (_, h, _) = solid_vol(ARCH_BOX).expect("box is solid");
    // With feet at jump height, the best surface in the cell is the box
    // top (terrain + lift + h — well inside a 2 m lid).
    let top = collide::piece_ground(SEED, hv(SEED), &cols, fx, fz, terr + 2.0);
    assert!(top > NO_SURFACE, "the box top must be a surface");
    assert!(
        top > terr + h - 0.1,
        "the surface should be the box top, not the floor: {top} vs terrain {terr}"
    );

    let mut b = Body::at(SEED, hv(SEED), fx, fz - 1.6);
    let mut stood_on_top = false;
    for _ in 0..300 {
        movement::step(
            SEED,
            hv(SEED),
            &cols,
            &mut occ.occupants(),
            &mut b,
            &walk(0, BTN_JUMP),
        );
        let (x, y, z) = pos(&b);
        let over = mag(x - fx) <= 0.6 && mag(z - fz) <= 0.35;
        if over && b.grounded {
            assert!(
                y >= top - h - 0.02,
                "grounded inside the box volume: feet {y}, box base {}",
                top - h
            );
            if mag(y - top) <= 0.03 {
                stood_on_top = true;
            }
        }
    }
    assert!(
        stood_on_top,
        "300 hop ticks never stood on the box top at {top}"
    );
}

// ------------------------------------------------------------------ wiring

/// The world with a player standing beside a foundation, holding one
/// furnace and one bag.
fn world() -> (World, u16, u16) {
    let mut w = World::new(SEED);
    w.gather = GatherContent::probe_fixture();
    w.build = BuildContent::probe_fixture();
    w.deploy = fixture();
    let (cx, cz) = buildable_cell(SEED);
    let (x, z) = cell_center(cx, cz);
    // Spawn one cell south of the build cell, outside the future volume.
    w.dev_spawn = Some((x, z - BUILD_CELL_M));
    w.tick(&[Command::Join { id: PLAYER }]);
    w.players[0].body = Body::at(SEED, hv(SEED), x, z - BUILD_CELL_M);
    w.players[0].inv[0] = ItemStack {
        item: 0,
        count: 5,
        cond: 0,
    };
    w.players[0].inv[1] = ItemStack {
        item: FURNACE_ITEM,
        count: 1,
        cond: 0,
    };
    w.players[0].inv[2] = ItemStack {
        item: BAG_ITEM,
        count: 1,
        cond: 0,
    };
    w.tick(&[Command::Place {
        id: PLAYER,
        row: FOUNDATION_ROW,
        cx,
        cz,
        level: 0,
        loc: LOC_PLANE,
        freehand: false,
    }]);
    assert_eq!(w.pieces.len(), 1, "the fixture needs its foundation");
    (w, cx, cz)
}

/// Walk the player north for `ticks` through the real command path.
fn drive(w: &mut World, ticks: usize, buttons: u8) {
    for i in 0..ticks {
        w.tick(&[Command::Input {
            id: PLAYER,
            frame: InputFrame {
                seq: (i + 2) as u16,
                buttons,
                yaw: 0,
                move_z: 127,
                ..InputFrame::default()
            },
        }]);
    }
}

/// The real verb walls the cell; `Demolish` (pick-up) frees it. This is
/// the lockstep test — the two writes `place_deploy` and `drop_deploy`
/// make against the collision index, observed through the walk.
#[test]
fn place_walls_the_cell_and_pickup_frees_it() {
    let (mut w, cx, cz) = world();
    let (_, fz) = cell_center(cx, cz);
    w.tick(&[Command::PlaceDeploy {
        id: PLAYER,
        row: FURNACE_ROW,
        cx,
        cz,
        level: 0,
        loc: LOC_PLANE,
    }]);
    assert_eq!(w.deploys.len(), 1, "the furnace must place");

    drive(&mut w, 240, 0);
    let (_, _, z) = pos(&w.players[0].body);
    let (_, _, hd) = solid_vol(ARCH_FURNACE).unwrap();
    assert!(
        z <= fz - hd - CAPSULE_RADIUS_M + 2.0 * POS_XZ_Q,
        "a placed furnace must block the walk: z {z}"
    );

    w.tick(&[Command::Demolish {
        id: PLAYER,
        deploy: true,
        cx,
        cz,
        level: 0,
        loc: LOC_PLANE,
    }]);
    assert_eq!(w.deploys.len(), 0, "pick-up must remove the furnace");
    drive(&mut w, 240, 0);
    assert!(
        pos(&w.players[0].body).2 > fz,
        "a picked-up furnace must stop blocking: z {}",
        pos(&w.players[0].body).2
    );
}

/// A bag never blocks: same address, same walk, straight through.
#[test]
fn a_bag_does_not_wall_its_cell() {
    let (mut w, cx, cz) = world();
    let (_, fz) = cell_center(cx, cz);
    w.tick(&[Command::PlaceDeploy {
        id: PLAYER,
        row: BAG_ROW,
        cx,
        cz,
        level: 0,
        loc: LOC_PLANE,
    }]);
    assert_eq!(w.deploys.len(), 1, "the bag must place");
    drive(&mut w, 240, 0);
    assert!(
        pos(&w.players[0].body).2 > fz,
        "a bag must not block the walk: z {}",
        pos(&w.players[0].body).2
    );
}

/// The wall survives a save and a load — `rebuild_doors`' solid half. The
/// index is derived state the file never carries, so this is the one test
/// that can catch a loaded shard whose furniture went walk-through.
#[test]
fn the_wall_survives_a_save_and_a_load() {
    let (mut w, cx, cz) = world();
    let (_, fz) = cell_center(cx, cz);
    w.tick(&[Command::PlaceDeploy {
        id: PLAYER,
        row: FURNACE_ROW,
        cx,
        cz,
        level: 0,
        loc: LOC_PLANE,
    }]);
    assert_eq!(w.deploys.len(), 1);

    let mut buf = vec![0u8; WORLD_SAVE_MAX_BYTES];
    let n = w.save_world(&mut buf).expect("save");
    let blob = &buf[..n];
    let mut w2 = World::new(SEED);
    w2.gather = GatherContent::probe_fixture();
    w2.build = BuildContent::probe_fixture();
    w2.deploy = fixture();
    w2.load(blob).expect("load");
    w2.dev_spawn = Some(cell_center(cx, cz));
    // The loaded world has the sleeper; drive the same body via a fresh
    // join at the same spot south of the furnace.
    w2.tick(&[Command::Join { id: PLAYER + 1 }]);
    let (x, _) = cell_center(cx, cz);
    let slot = w2.live_slot_of(PLAYER + 1).expect("joined");
    w2.players[slot].body = Body::at(SEED, hv(SEED), x, fz - BUILD_CELL_M);
    for i in 0..240 {
        w2.tick(&[Command::Input {
            id: PLAYER + 1,
            frame: InputFrame {
                seq: (i + 2) as u16,
                buttons: 0,
                yaw: 0,
                move_z: 127,
                ..InputFrame::default()
            },
        }]);
    }
    let (_, _, hd) = solid_vol(ARCH_FURNACE).unwrap();
    let z = w2.players[slot].body.qz as f32 * POS_XZ_Q;
    assert!(
        z <= fz - hd - CAPSULE_RADIUS_M + 2.0 * POS_XZ_Q,
        "a loaded furnace must still block: z {z}"
    );
}

// ------------------------------------------------------------- live terrain

/// Through the haven shelter's doorway and onto the plinth: the feet end
/// ~0.2 m above the slot's own ground while inside the footprint. The one
/// walk on live terrain, at the one authored place every seed has.
#[test]
fn walking_into_the_shelter_stands_on_the_plinth() {
    let seed = 7;
    let mut occ = Scratch::live(seed);
    let haven = terrain::haven(seed);
    let (sx, sz, _) = terrain::haven_shelter(&haven);
    // The real slot, from scatter — its y and yaw are the fixture, not a
    // re-derivation (walk.rs's find(), narrowed to the shelter's cell).
    let table = terrain::ScatterTable::alpha_default();
    let mut found: Option<Slot> = None;
    let (scx, scz) = ((sx / CELL_SIZE) as i32, (sz / CELL_SIZE) as i32);
    'scan: for cz in (scz - 2).max(0)..(scz + 3).min(CELLS_PER_SIDE) {
        for cx in (scx - 2).max(0)..(scx + 3).min(CELLS_PER_SIDE) {
            let s = terrain::scatter(seed, &table, &haven, cx, cz);
            if s.occupant == Occupant::HavenShelter {
                found = Some(s);
                break 'scan;
            }
        }
    }
    let slot = found.expect("the shelter slot is within two cells of its own anchor");
    let cols = Box::new(ColIndex::new());

    // The doorway is on local +Z: approach along the slot's own yaw
    // bearing from 6 m out, walking inward.
    let (dirx, dirz) = sim_core::yaw_dir((slot.yaw as u16) << 8);
    let start = (slot.x + dirx * 6.0, slot.z + dirz * 6.0);
    let mut b = Body::at(seed, hv(seed), start.0, start.1);
    // Face the shelter: the yaw whose direction is -dir.
    let mut best_yaw = 0u16;
    let mut best_dot = f32::NEG_INFINITY;
    for i in 0..256u16 {
        let (ex, ez) = sim_core::yaw_dir(i << 8);
        let dot = ex * -dirx + ez * -dirz;
        if dot > best_dot {
            best_dot = dot;
            best_yaw = i << 8;
        }
    }
    let mut on_plinth = false;
    for _ in 0..400 {
        movement::step(
            seed,
            hv(seed),
            &cols,
            &mut occ.occupants(),
            &mut b,
            &walk(best_yaw, 0),
        );
        let (x, y, z) = pos(&b);
        let (dx, dz) = (x - slot.x, z - slot.z);
        // Inside the plinth footprint (7×7 local; the walk comes straight
        // down the axis so the world/local distinction cancels on it).
        if dx * dx + dz * dz <= 2.0 * 2.0 {
            let plinth = slot.y + 0.2 * slot.scale;
            assert!(
                y >= plinth - 0.05,
                "inside the shelter the feet sank below the plinth: {y} vs {plinth}"
            );
            if mag(y - plinth) <= 0.05 {
                on_plinth = true;
            }
        }
    }
    assert!(
        on_plinth,
        "the walk never stood on the plinth (blocked outside, or sank)"
    );
}

// ------------------------------------------------------------------- table

/// The volume table's non-zero rows are the client's authored sizes; the
/// zero rows are exactly the four the doc names. Digit-for-digit copy of
/// the client table is asserted from the client side (`tests/greybox.rs`);
/// this side pins the *shape* of the table so a new archetype cannot land
/// without deciding.
#[test]
fn the_volume_table_covers_every_archetype() {
    assert_eq!(DEPLOY_VOL.len(), 12, "a new archetype needs a volume row");
    for (arch, [w, h, d]) in DEPLOY_VOL.iter().enumerate() {
        let solid = solid_vol(arch as u8).is_some();
        assert_eq!(
            solid,
            *h > 0.0,
            "solid_vol and the table disagree at arch {arch}"
        );
        if solid {
            assert!(
                *w > 0.0 && *d > 0.0,
                "a solid row needs a footprint: {arch}"
            );
        }
    }
    // The four walk-over rows, by name, so a bag growing a volume is a
    // decision and not a typo.
    for arch in [0u8, 3, 6, 7] {
        assert!(
            solid_vol(arch).is_none(),
            "arch {arch} (bag/fire/door/lock) must stay walk-over"
        );
    }
}
