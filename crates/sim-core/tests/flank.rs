//! A base has sides — walking into one stops you (piece flanks v0).
//!
//! **The defect this closes put the camera inside people's bases.**
//! `collide::blocked` walks edges and diagonals and `deploy_blocked` walks
//! solid deployables; a PLANE was ground and nothing else, so a body walked
//! straight into the flank of a foundation and stood inside the slab and the
//! drawn skirt under it. `NOW.md` §0bl item 4 named it — "walking into it from
//! downhill clips through" — and build plate v1 sharpened it rather than
//! easing it: a stilted base carries up to a storey of leg, and every
//! centimetre of that leg used to be walk-through.
//!
//! Driven through `movement::step` rather than through the predicate, which is
//! `tests/walk.rs`' rule and its header's reason: a predicate nobody calls
//! stops nothing. The occupant table is `Barren` — no trees, no boulders, no
//! crates — so a body that stops has been stopped by the BASE and not by a
//! pine that happened to be in the way. `walk.rs` is the mirror of this and
//! says so: it walks with an empty `ColIndex` precisely so a wall cannot take
//! the credit for what a trunk did.

// The measurements are the gate's output — `tests/walk.rs`' allow, its reason:
// the L5 wall bans format/print in SIM code and a harness is not sim code.
#![allow(clippy::disallowed_macros)]

use sim_core::build::{
    self, place, BuildContent, Pieces, BUILD_CELL_M, LEVEL_H_M, LOC_PLANE, PLATE_RISE_MAX_BANDS,
};
use sim_core::collide::{plane_blocked, ColIndex, CAPSULE_HEIGHT_M, CAPSULE_RADIUS_M};
// `fmath` only, never `f32::abs`: the walls' clippy list is crate-scoped, so
// it binds this suite exactly as it binds the sim (`tests/base_lattice.rs`
// says the same).
use sim_core::deploy::Deploys;
use sim_core::fmath::fabs;
use sim_core::gather::ItemStack;
use sim_core::input::InputFrame;
use sim_core::movement::{self, Body, POS_XZ_Q, POS_Y_Q};
use sim_core::occupy::Scratch;
use sim_core::terrain;
use sim_core::world::{EventQueue, Player, EV_BUILD_REFUSED};

fn hv(seed: u64) -> &'static terrain::Haven {
    use std::cell::RefCell;
    thread_local! {
        static CACHE: RefCell<Vec<(u64, &'static terrain::Haven)>> =
            const { RefCell::new(Vec::new()) };
    }
    if let Some(h) = CACHE.with(|c| c.borrow().iter().find(|(s, _)| *s == seed).map(|&(_, h)| h)) {
        return h;
    }
    let h: &'static terrain::Haven = Box::leak(Box::new(terrain::haven(seed)));
    CACHE.with(|c| c.borrow_mut().push((seed, h)));
    h
}

const SEED: u64 = 20260731;
const CX: u16 = 341;
const CZ: u16 = 341;

/// The cell centre, world XZ.
fn centre(cx: u16, cz: u16) -> (f32, f32) {
    (
        cx as f32 * BUILD_CELL_M + BUILD_CELL_M * 0.5,
        cz as f32 * BUILD_CELL_M + BUILD_CELL_M * 0.5,
    )
}

/// A foundation at (cx, cz) placed by the sim's own verb, and the store it
/// went into. `plate` is what it ended up latched at.
fn founded(cx: u16, cz: u16) -> Pieces {
    let mut pieces = Pieces::new();
    let bc = BuildContent::probe_fixture();
    let deploys = Deploys::new();
    let (ax, az) = build::anchor(cx, cz, LOC_PLANE);
    let mut p = Player {
        id: 7,
        active: true,
        body: Body::at(SEED, hv(SEED), ax, az),
        ..Player::default()
    };
    p.inv[0] = ItemStack {
        item: 0,
        count: 99,
        cond: 0,
    };
    let mut ev = EventQueue::default();
    place(
        SEED,
        hv(SEED),
        &bc,
        &deploys,
        &mut pieces,
        &mut p,
        0,
        0,
        cx,
        cz,
        0,
        LOC_PLANE,
        false,
        0,
        &mut ev,
    );
    assert!(
        !ev.entries().iter().any(|e| e.code == EV_BUILD_REFUSED),
        "the fixture foundation was refused"
    );
    pieces
}

/// Sprint `ticks` toward `yaw`, handing every tick's body to `saw`.
///
/// **Per tick, not endpoint-only, and that is the whole difference between
/// this gate and a green one that proves nothing.** The first draft read only
/// where the body finished: with the flank disabled it walked THROUGH the
/// base and out the far side, ending 4.5 m past it — outside the footprint,
/// so an endpoint check called that a pass. What the camera is actually
/// asking is whether the body was ever inside, and only the path can answer.
fn walk(
    cols: &ColIndex,
    from: (f32, f32),
    yaw: u16,
    ticks: u32,
    mut saw: impl FnMut(&Body),
) -> Body {
    let mut b = Body::at(SEED, hv(SEED), from.0, from.1);
    let mut sc = Scratch::barren();
    let f = InputFrame {
        yaw,
        move_z: 127,
        ..InputFrame::default()
    };
    for _ in 0..ticks {
        movement::step(SEED, hv(SEED), cols, &mut sc.occupants(), &mut b, &f);
        saw(&b);
    }
    b
}

/// The yaw LUT bearing pointing along (dx, dz) — `walk.rs`' own scan, so the
/// heading is one the sim can actually hold.
fn bearing(dx: f32, dz: f32) -> u16 {
    let (mut best, mut bd) = (0u16, f32::MIN);
    let n = (dx * dx + dz * dz).sqrt().max(1e-6);
    for y in 0..256u16 {
        let (fx, fz) = sim_core::yaw_dir(y << 8);
        let d = fx * (dx / n) + fz * (dz / n);
        if d > bd {
            bd = d;
            best = y << 8;
        }
    }
    best
}

// ---------------------------------------------------------------------------

/// A body cannot walk into the side of a stilted foundation.
///
/// **The whole slice in one assertion.** A plate stands a storey over the
/// ground beside it — the shape build plate v1 made common — and a body sprints
/// at it from three cells away. Before piece flanks it ended up at the cell
/// centre, inside the slab and inside the drawn skirt; now it stops outside
/// the footprint.
///
/// The plate is set on the index directly rather than marched out by `place`.
/// A first draft did march it and could only reach ONE band on this seed's
/// flat interior, which is 0.5 m — under `STEP_UP`, so no flank would ever
/// have fired and the test would have passed on an empty claim. Where a
/// stilt comes from is `tests/plate.rs`' subject; what a stilt DOES to a body
/// is this one's, and it is still driven end to end through `movement::step`.
#[test]
fn a_body_cannot_walk_into_the_side_of_a_base() {
    let plate = PLATE_RISE_MAX_BANDS as i8;
    let mut cols = ColIndex::new();
    for d in 0..3u16 {
        cols.add(
            CX + d,
            CZ,
            0,
            LOC_PLANE,
            sim_core::build::SHAPE_FOUNDATION,
            plate,
        );
    }
    let (fx, fz) = centre(CX + 1, CZ);
    let top = build::column_floor_y(SEED, hv(SEED), CX + 1, CZ, plate);
    let ground = build::column_floor_y(SEED, hv(SEED), CX + 1, CZ, 0);
    assert!(
        top - ground > sim_core::movement::STEP_UP,
        "the fixture's plate is inside the step rule, so no flank can fire"
    );

    // Stand on the ground three cells beyond the plate and sprint into it.
    let start = (fx + 3.0 * BUILD_CELL_M, fz);
    let yaw = bearing(fx - start.0, fz - start.1);
    // Every cell of the plate, checked on every tick of the walk.
    let cells: Vec<(f32, f32)> = (0..3u16).map(|d| centre(CX + d, CZ)).collect();
    let mut worst: Option<(f32, f32, f32)> = None;
    let b = walk(&cols, start, yaw, 200, |b| {
        let (bx, bz, by) = (
            b.qx as f32 * POS_XZ_Q,
            b.qz as f32 * POS_XZ_Q,
            b.qy as f32 * POS_Y_Q,
        );
        for &(cx, cz) in &cells {
            let inside = fabs(bx - cx) <= BUILD_CELL_M * 0.5 && fabs(bz - cz) <= BUILD_CELL_M * 0.5;
            // Head below the floor: the eye is in the slab or in the skirt.
            if inside && by + CAPSULE_HEIGHT_M < top && worst.is_none() {
                worst = Some((bx, bz, by));
            }
        }
    });
    assert!(
        worst.is_none(),
        "the body passed through the base at {:?} — inside a footprint whose floor \
is {top:.2}, i.e. through the skirt",
        worst.unwrap()
    );

    let (bx, bz) = (b.qx as f32 * POS_XZ_Q, b.qz as f32 * POS_XZ_Q);
    let by = b.qy as f32 * POS_Y_Q;
    // It came to rest somewhere the flank actually allows — the half a
    // position check cannot make: a body wedged where the predicate refuses
    // is a body the veto-lift is carrying.
    assert!(
        !plane_blocked(SEED, hv(SEED), &cols, bx, bz, by),
        "the body came to rest at ({bx:.2},{bz:.2},{by:.2}), which the flank refuses"
    );
    // And it did travel — every line above passes on a body that never moved.
    assert!(
        fabs(bx - start.0) > BUILD_CELL_M,
        "the body only moved {:.2} m; something else stopped it",
        fabs(bx - start.0)
    );
    // It stopped a capsule's radius off the plate rather than at its face,
    // which is the clearance that keeps the eye outside the drawn slab.
    let plate_edge = (CX + 3) as f32 * BUILD_CELL_M;
    assert!(
        bx >= plate_edge + CAPSULE_RADIUS_M - POS_XZ_Q,
        "the body stopped at x={bx:.2}; the plate ends at {plate_edge:.2} and the \
capsule is {CAPSULE_RADIUS_M} wide"
    );
}

/// Standing on your own base still works: every cell of a plate is walkable
/// from every other, and the flank never fires at your own level.
///
/// **The assertion that stops this slice from being a cage.** A flank that
/// forgot `STEP_UP`'s short circuit would make a base a set of cells you can
/// see and cannot cross.
#[test]
fn a_plate_stays_walkable_end_to_end() {
    let mut pieces = founded(CX, CZ);
    let bc = BuildContent::probe_fixture();
    let deploys = Deploys::new();
    let mut last = CX;
    for cx in CX + 1..CX + 5 {
        let (ax, az) = build::anchor(cx, CZ, LOC_PLANE);
        let mut p = Player {
            id: 7,
            active: true,
            body: Body::at(SEED, hv(SEED), ax, az),
            ..Player::default()
        };
        p.inv[0] = ItemStack {
            item: 0,
            count: 99,
            cond: 0,
        };
        let mut ev = EventQueue::default();
        place(
            SEED,
            hv(SEED),
            &bc,
            &deploys,
            &mut pieces,
            &mut p,
            0,
            0,
            cx,
            CZ,
            0,
            LOC_PLANE,
            false,
            0,
            &mut ev,
        );
        if ev.entries().iter().any(|e| e.code == EV_BUILD_REFUSED) {
            break;
        }
        last = cx;
    }
    assert!(last >= CX + 3, "the fixture needs a plate a few cells long");

    // Stand at the near end, on the plate, and walk to the far end.
    let (sx, sz) = centre(CX, CZ);
    let (ex, ez) = centre(last, CZ);
    let mut b = Body::at(SEED, hv(SEED), sx, sz);
    b.qy = (build::column_floor_y(SEED, hv(SEED), CX, CZ, 0) / POS_Y_Q) as i32;
    let mut sc = Scratch::barren();
    let f = InputFrame {
        yaw: bearing(ex - sx, ez - sz),
        move_z: 127,
        ..InputFrame::default()
    };
    for _ in 0..200 {
        movement::step(
            SEED,
            hv(SEED),
            pieces.cols(),
            &mut sc.occupants(),
            &mut b,
            &f,
        );
    }
    let bx = b.qx as f32 * POS_XZ_Q;
    assert!(
        bx >= ex - BUILD_CELL_M,
        "the body walked its own plate to x={bx:.2} and the far cell is at {ex:.2} — \
a flank is firing at its own level"
    );
}

/// A floor overhead is passed under; a foundation is not.
///
/// The one asymmetry in the rule, stated directly: a slab has air below it and
/// a footing does not, so `PLANE_THICKNESS_M` bounds the first and nothing
/// bounds the second.
#[test]
fn a_floor_lets_you_under_and_a_foundation_does_not() {
    let mut cols = ColIndex::new();
    // A floor two storeys up, with nothing under it.
    cols.add(CX, CZ, 2, LOC_PLANE, sim_core::build::SHAPE_FLOOR, 0);
    let (x, z) = centre(CX, CZ);
    let ground = build::column_floor_y(SEED, hv(SEED), CX, CZ, 0);
    assert!(
        !plane_blocked(SEED, hv(SEED), &cols, x, z, ground),
        "a floor two storeys up blocked a body standing under it"
    );

    // The same address at level 0 is a foundation, stilted a storey, and it
    // is solid all the way down.
    let mut cols = ColIndex::new();
    let plate = PLATE_RISE_MAX_BANDS as i8;
    cols.add(
        CX,
        CZ,
        0,
        LOC_PLANE,
        sim_core::build::SHAPE_FOUNDATION,
        plate,
    );
    assert!(
        plane_blocked(SEED, hv(SEED), &cols, x, z, ground),
        "a foundation stilted a storey let a body stand under it"
    );
    // ...and standing ON it is not blocked, at any plate.
    let top = build::column_floor_y(SEED, hv(SEED), CX, CZ, plate);
    assert!(
        !plane_blocked(SEED, hv(SEED), &cols, x, z, top),
        "a foundation blocked a body standing on its own floor"
    );
}

/// The flank holds the body a capsule's radius off the cell, which is what
/// keeps the eye out of the drawn slab.
///
/// **This is the number the whole ask is about.** The camera sits at the
/// body's centre; the drawn slab's face IS the cell boundary (it was 2 cm
/// inside it until the seam went, gap v1). Hold the centre at the boundary
/// and the eye is on a surface the near plane clips at 10 cm — you see
/// through your own base. Hold it a radius out and the eye is 0.4 m from
/// the face, which is the clearance a wall has had all along.
#[test]
fn the_flank_holds_a_capsules_radius_off_the_cell() {
    let mut cols = ColIndex::new();
    let plate = PLATE_RISE_MAX_BANDS as i8;
    cols.add(
        CX,
        CZ,
        0,
        LOC_PLANE,
        sim_core::build::SHAPE_FOUNDATION,
        plate,
    );
    let ground = build::column_floor_y(SEED, hv(SEED), CX, CZ, 0);
    let x0 = CX as f32 * BUILD_CELL_M;
    let (_, z) = centre(CX, CZ);
    // Just inside the radius: refused. Just outside: allowed.
    let inside = x0 - CAPSULE_RADIUS_M + 0.01;
    let outside = x0 - CAPSULE_RADIUS_M - 0.01;
    assert!(
        plane_blocked(SEED, hv(SEED), &cols, inside, z, ground),
        "the flank let a body within a radius of the cell"
    );
    assert!(
        !plane_blocked(SEED, hv(SEED), &cols, outside, z, ground),
        "the flank refused a body more than a radius from the cell"
    );
    // The clearance that matters: eye to the DRAWN face, against the near
    // plane. The drawn face sits on the cell boundary itself since the seam
    // went (gap v1; it was half a 4 cm seam inside) — restated as a literal
    // here because sim-core may not depend on the client;
    // `client/tests/lattice_geom.rs` owns the version that reads the real
    // extents.
    let drawn_face_inset = 0.0;
    assert!(
        CAPSULE_RADIUS_M + drawn_face_inset > 0.1,
        "the eye can reach inside the near plane of its own base"
    );
}

/// A body a base was built AROUND walks out of it.
///
/// **The veto-lift, and the reason it is not optional.** `build::place` asks
/// nothing about who is standing in the cell — a foundation can land on top of
/// a player, and on a shard with `dev_spawn` it routinely does. A flank that
/// vetoed every candidate would then pin that body inside the slab forever
/// with no verb that frees it: walking out is the only escape a capsule has,
/// so being stuck must never be absorbing. `movement::step` carries the same
/// latch for scattered occupants ("a node can respawn around a standing
/// player") and for solid deployables; this is the third, and it needed its
/// own test — every other assertion in this file passes with the lift deleted.
#[test]
fn a_body_built_around_can_still_walk_out() {
    let plate = PLATE_RISE_MAX_BANDS as i8;
    let mut cols = ColIndex::new();
    for d in 0..3u16 {
        cols.add(
            CX + d,
            CZ,
            0,
            LOC_PLANE,
            sim_core::build::SHAPE_FOUNDATION,
            plate,
        );
    }
    // Stand in the middle cell, on the ground, under the stilted plate — the
    // state a body reaches by having a base built over it.
    let (cx, cz) = centre(CX + 1, CZ);
    let ground = build::column_floor_y(SEED, hv(SEED), CX + 1, CZ, 0);
    let mut b = Body::at(SEED, hv(SEED), cx, cz);
    let by = b.qy as f32 * POS_Y_Q;
    assert!(
        plane_blocked(SEED, hv(SEED), &cols, cx, cz, by),
        "the fixture body is not actually inside the flank"
    );
    assert!(
        by < ground + LEVEL_H_M,
        "the fixture body is on the plate, not under it"
    );

    // Walk toward open ground. It must get clear of every cell of the plate.
    let out = (cx + 6.0 * BUILD_CELL_M, cz);
    let f = InputFrame {
        yaw: bearing(out.0 - cx, out.1 - cz),
        move_z: 127,
        ..InputFrame::default()
    };
    let mut sc = Scratch::barren();
    for _ in 0..200 {
        movement::step(SEED, hv(SEED), &cols, &mut sc.occupants(), &mut b, &f);
    }
    let (fx, fz, fy) = (
        b.qx as f32 * POS_XZ_Q,
        b.qz as f32 * POS_XZ_Q,
        b.qy as f32 * POS_Y_Q,
    );
    assert!(
        !plane_blocked(SEED, hv(SEED), &cols, fx, fz, fy),
        "the body is still inside the base at ({fx:.2},{fz:.2},{fy:.2}) — the flank is a grave"
    );
    assert!(
        fx > (CX + 3) as f32 * BUILD_CELL_M,
        "the body only reached x={fx:.2}; the plate ends at {:.2}",
        (CX + 3) as f32 * BUILD_CELL_M
    );
}
