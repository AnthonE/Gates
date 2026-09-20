//! Movement through a multi-storey base, including the ceiling above a ramp.
//! Geometry is installed in the collision index to isolate movement from
//! placement. Every assertion observes the real shared movement step.

use sim_core::build::{
    self, BUILD_CELL_M, LEVEL_H_M, LOC_PLANE, LOC_RISER, LOC_TRI_XHI_ZHI, LOC_TRI_XHI_ZLO,
    LOC_TRI_XLO_ZHI, LOC_TRI_XLO_ZLO, SHAPE_FLOOR, SHAPE_FOUNDATION, SHAPE_ROOF, SHAPE_STAIRS,
    SHAPE_TRI_FLOOR, SHAPE_TRI_ROOF,
};
use sim_core::collide::{ColIndex, CAPSULE_HEIGHT_M, CAPSULE_RADIUS_M, PLANE_THICKNESS_M};
use sim_core::fmath::fabs;
use sim_core::input::{InputFrame, BTN_JUMP};
use sim_core::movement::{self, Body, POS_XZ_Q, POS_Y_Q};
use sim_core::occupy::Scratch;
use sim_core::terrain;

const SEED: u64 = 20260731;
const CX: u16 = 341;
const CZ: u16 = 341;

fn haven() -> &'static terrain::Haven {
    use std::cell::OnceCell;
    thread_local! {
        static H: OnceCell<&'static terrain::Haven> = const { OnceCell::new() };
    }
    H.with(|h| *h.get_or_init(|| Box::leak(Box::new(terrain::haven(SEED)))))
}

fn base() -> f32 {
    build::column_floor_y(SEED, haven(), CX, CZ, 0)
}

/// All fixture columns share the first foundation's plate, as placed bases do.
fn put(cols: &mut ColIndex, cx: u16, cz: u16, level: u8, loc: u8, shape: u8) {
    let band = build::terrain_band(SEED, haven(), CX, CZ);
    let plate = (band - build::terrain_band(SEED, haven(), cx, cz)) as i8;
    cols.add(cx, cz, level, loc, shape, plate);
}

fn body(dx: f32, dz: f32, rise: f32) -> Body {
    let mut b = Body::at(
        SEED,
        haven(),
        CX as f32 * BUILD_CELL_M + dx,
        CZ as f32 * BUILD_CELL_M + dz,
    );
    b.qy = movement::quant_y(base() + rise);
    b
}

fn y(b: Body) -> f32 {
    b.qy as f32 * POS_Y_Q
}

fn z(b: Body) -> f32 {
    b.qz as f32 * POS_XZ_Q
}

#[test]
fn a_covered_stairwell_stops_before_the_head_enters_the_floor() {
    let mut cols = ColIndex::new();
    put(&mut cols, CX, CZ, 0, LOC_PLANE, SHAPE_FOUNDATION);
    put(&mut cols, CX, CZ, 0, LOC_RISER, SHAPE_STAIRS);
    put(&mut cols, CX, CZ, 1, LOC_PLANE, SHAPE_FLOOR);
    let mut b = body(1.5, 0.09, 0.09);
    let start = b;
    let underside = base() + LEVEL_H_M - PLANE_THICKNESS_M;
    let mut sc = Scratch::barren();
    for _ in 0..100 {
        movement::step(
            SEED,
            haven(),
            &cols,
            &mut sc.occupants(),
            &mut b,
            &InputFrame {
                move_z: 127,
                ..InputFrame::default()
            },
        );
        assert!(
            y(b) + CAPSULE_HEIGHT_M <= underside,
            "ramp put the head at {} through ceiling {underside}",
            y(b) + CAPSULE_HEIGHT_M
        );
    }
    assert!(
        z(b) > z(start) + CAPSULE_RADIUS_M,
        "never approached the ceiling"
    );
    assert!(b.grounded, "blocked ramp must remain standable");
    for _ in 0..20 {
        movement::step(
            SEED,
            haven(),
            &cols,
            &mut sc.occupants(),
            &mut b,
            &InputFrame {
                move_z: -127,
                ..InputFrame::default()
            },
        );
    }
    assert!(
        z(b) < z(start),
        "the ceiling trapped the player on the ramp"
    );
}

#[test]
fn open_stairs_reach_two_upper_landings_and_return_to_ground() {
    let mut cols = ColIndex::new();
    for dz in 0..3 {
        put(&mut cols, CX, CZ + dz, 0, LOC_PLANE, SHAPE_FOUNDATION);
    }
    put(&mut cols, CX, CZ, 0, LOC_RISER, SHAPE_STAIRS);
    put(&mut cols, CX, CZ + 1, 1, LOC_PLANE, SHAPE_FLOOR);
    put(&mut cols, CX, CZ + 1, 1, LOC_RISER, SHAPE_STAIRS);
    put(&mut cols, CX, CZ + 2, 2, LOC_PLANE, SHAPE_FLOOR);
    let mut b = body(1.5, 0.09, 0.09);
    let start = b;
    let mut sc = Scratch::barren();
    let landing = (CZ + 2) as f32 * BUILD_CELL_M + 1.5;
    for _ in 0..100 {
        movement::step(
            SEED,
            haven(),
            &cols,
            &mut sc.occupants(),
            &mut b,
            &InputFrame {
                move_z: 127,
                ..InputFrame::default()
            },
        );
        if z(b) >= landing {
            break;
        }
    }
    assert!(
        z(b) >= landing,
        "could not step from the ramp onto its landing"
    );
    assert!(fabs(y(b) - base() - 2.0 * LEVEL_H_M) <= POS_Y_Q);
    for _ in 0..100 {
        movement::step(
            SEED,
            haven(),
            &cols,
            &mut sc.occupants(),
            &mut b,
            &InputFrame {
                move_z: -127,
                ..InputFrame::default()
            },
        );
        if z(b) <= z(start) {
            break;
        }
    }
    assert!(z(b) <= z(start), "could not descend the two flights");
    assert!(fabs(y(b) - base()) <= movement::STEP_UP && b.grounded);
}

/// A tap must hit the underside, cancel upward velocity, then land again.
fn jump(cols: &ColIndex, mut b: Body, has_ceiling: bool) {
    let start = y(b);
    let underside = start + LEVEL_H_M - PLANE_THICKNESS_M;
    let mut apex = start;
    let mut stopped = false;
    let mut sc = Scratch::barren();
    for t in 0..100 {
        movement::step(
            SEED,
            haven(),
            cols,
            &mut sc.occupants(),
            &mut b,
            &InputFrame {
                buttons: if t == 0 { BTN_JUMP } else { 0 },
                ..InputFrame::default()
            },
        );
        apex = apex.max(y(b));
        if has_ceiling {
            assert!(
                y(b) + CAPSULE_HEIGHT_M <= underside,
                "jump put head {} through underside {underside}",
                y(b) + CAPSULE_HEIGHT_M
            );
            stopped |= !b.grounded && b.qvy == 0 && y(b) > start;
        }
    }
    if has_ceiling {
        assert!(
            stopped,
            "the upward velocity was never cancelled at the ceiling"
        );
        assert!(
            apex > start + movement::STEP_UP,
            "the fixture never approached the ceiling"
        );
    } else {
        assert!(
            apex + CAPSULE_HEIGHT_M > underside,
            "an absent ceiling stopped the jump, or the fixture never reached its height"
        );
    }
    assert!(
        b.grounded && b.qvy == 0 && fabs(y(b) - start) <= POS_Y_Q,
        "a ceiling contact prevented landing"
    );
}

#[test]
fn a_jump_hits_floors_roofs_and_each_triangle_half() {
    for (shape, loc, inside, outside) in [
        (SHAPE_FLOOR, LOC_PLANE, (1.5, 1.5), None),
        (SHAPE_ROOF, LOC_PLANE, (1.5, 1.5), None),
        (
            SHAPE_TRI_FLOOR,
            LOC_TRI_XLO_ZLO,
            (0.6, 0.6),
            Some((2.4, 2.4)),
        ),
        (
            SHAPE_TRI_FLOOR,
            LOC_TRI_XHI_ZHI,
            (2.4, 2.4),
            Some((0.6, 0.6)),
        ),
        (
            SHAPE_TRI_ROOF,
            LOC_TRI_XHI_ZLO,
            (2.4, 0.6),
            Some((0.6, 2.4)),
        ),
        (
            SHAPE_TRI_ROOF,
            LOC_TRI_XLO_ZHI,
            (0.6, 2.4),
            Some((2.4, 0.6)),
        ),
    ] {
        let mut cols = ColIndex::new();
        put(&mut cols, CX, CZ, 0, LOC_PLANE, SHAPE_FOUNDATION);
        put(&mut cols, CX, CZ, 1, loc, shape);
        jump(&cols, body(inside.0, inside.1, 0.0), true);
        if let Some((x, z)) = outside {
            jump(&cols, body(x, z, 0.0), false);
        }
    }
}

#[test]
fn a_ceiling_reaches_into_the_neighbour_by_the_capsules_radius() {
    let mut cols = ColIndex::new();
    put(&mut cols, CX - 1, CZ, 0, LOC_PLANE, SHAPE_FOUNDATION);
    put(&mut cols, CX, CZ, 1, LOC_PLANE, SHAPE_FLOOR);
    jump(&cols, body(-CAPSULE_RADIUS_M + 0.1, 1.5, 0.0), true);
    jump(&cols, body(-CAPSULE_RADIUS_M - 0.1, 1.5, 0.0), false);
}

#[test]
fn ceiling_contacts_remain_safe_on_every_storey() {
    for lower in 0..sim_core::limits::MAX_BUILD_LEVELS as u8 - 1 {
        let mut cols = ColIndex::new();
        put(
            &mut cols,
            CX,
            CZ,
            lower,
            LOC_PLANE,
            if lower == 0 {
                SHAPE_FOUNDATION
            } else {
                SHAPE_FLOOR
            },
        );
        put(&mut cols, CX, CZ, lower + 1, LOC_PLANE, SHAPE_ROOF);
        jump(&cols, body(1.5, 1.5, lower as f32 * LEVEL_H_M), true);
    }
}

#[test]
fn the_lowest_overlapping_ceiling_wins_in_either_cell_order() {
    for first in [1, 2] {
        let mut cols = ColIndex::new();
        put(&mut cols, CX - 1, CZ, 0, LOC_PLANE, SHAPE_FOUNDATION);
        put(&mut cols, CX - 1, CZ, first, LOC_PLANE, SHAPE_FLOOR);
        put(&mut cols, CX, CZ, 3 - first, LOC_PLANE, SHAPE_FLOOR);
        jump(&cols, body(-0.3, 1.5, 0.0), true);
    }
}

#[test]
fn the_parity_probe_reaches_both_ceiling_contacts() {
    assert_eq!(sim_core::probe::probe_headroom() >> 32, 3);
}

#[test]
fn a_floor_placed_through_the_head_still_allows_escape() {
    let mut cols = ColIndex::new();
    put(&mut cols, CX, CZ, 0, LOC_PLANE, SHAPE_FOUNDATION);
    put(&mut cols, CX, CZ, 0, LOC_RISER, SHAPE_STAIRS);
    put(&mut cols, CX, CZ, 1, LOC_PLANE, SHAPE_FLOOR);
    let mut b = body(1.5, 1.5, 1.5);
    assert!(y(b) + CAPSULE_HEIGHT_M > base() + LEVEL_H_M - PLANE_THICKNESS_M);
    let mut sc = Scratch::barren();
    for _ in 0..30 {
        movement::step(
            SEED,
            haven(),
            &cols,
            &mut sc.occupants(),
            &mut b,
            &InputFrame {
                move_z: -127,
                ..InputFrame::default()
            },
        );
    }
    assert!(
        z(b) < CZ as f32 * BUILD_CELL_M,
        "placement trapped the body inside the floor"
    );
}
