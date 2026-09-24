//! Animal routes (`nav.rs`): the grid A* the brain drives, gated on the one
//! thing it exists for — getting round a wall that straight-line steering
//! walks into forever.
//!
//! Every fixture is `occupy::Scratch::barren()`, so the only solid things in
//! the world are the walls a test builds and the terrain under them.

use sim_core::build::{self, BUILD_CELL_M, LOC_EDGE_XLO, LOC_EDGE_ZLO, SHAPE_WALL};
use sim_core::collide::ColIndex;
use sim_core::fmath::fabs;
use sim_core::input::InputFrame;
use sim_core::movement::{self, Body, POS_XZ_Q, POS_Y_Q};
use sim_core::nav::{self, Ground, Nav, NavPath, Plan};
use sim_core::occupy::Scratch;

const SEED: u64 = 7;

/// A build cell with an 11×11-cell (33 m) square of foundation-flat ground
/// around it, so a wall can stand and a body can walk every way round it.
fn flat_patch(sc: &Scratch<sim_core::occupy::Barren>) -> (u16, u16) {
    for r in 0..96i32 {
        for dz in -r..=r {
            for dx in -r..=r {
                if dx.abs() != r && dz.abs() != r {
                    continue;
                }
                let (cx, cz) = (340 + dx, 340 + dz);
                let flat = (-5..=5).all(|ez| {
                    (-5..=5).all(|ex| {
                        let x = ((cx + ex) as f32 + 0.5) * BUILD_CELL_M;
                        let z = ((cz + ez) as f32 + 0.5) * BUILD_CELL_M;
                        build::foundation_terrain_ok(SEED, &sc.haven, x, z)
                    })
                });
                if flat {
                    return (cx as u16, cz as u16);
                }
            }
        }
    }
    panic!("seed {SEED} has no flat 33 m patch near the centre");
}

/// A 27 m wall along z on the low-x edge of build column `cx`, centred on
/// `cz` — nine panels, too long for a capsule to slide off either end in a
/// straight line.
fn wall_line(cols: &mut ColIndex, cx: u16, cz: u16) {
    for k in -4i32..=4 {
        cols.add(cx, (cz as i32 + k) as u16, 0, LOC_EDGE_XLO, SHAPE_WALL, 0);
    }
}

/// Walk a body along a planned route the way `mob::step` does: steer at the
/// next waypoint, full walk speed. Returns where it stopped and whether the
/// route said it arrived.
fn walk(
    sc: &mut Scratch<sim_core::occupy::Barren>,
    cols: &ColIndex,
    body: &mut Body,
    path: &mut NavPath,
    ticks: u32,
) -> bool {
    let haven = sc.haven;
    let mut yaw = 0u16;
    for _ in 0..ticks {
        let Some((tx, tz)) = nav::waypoint(path, body.qx, body.qz) else {
            return path.arrived;
        };
        yaw = nav::yaw_toward((tx - body.qx) as f32, (tz - body.qz) as f32, yaw);
        let frame = InputFrame {
            yaw,
            move_z: 127,
            ..InputFrame::default()
        };
        movement::step(SEED, &haven, cols, &mut sc.occupants(), body, &frame);
    }
    path.arrived
}

/// **The route goes round the wall, and the straight line does not.**
///
/// Both halves, because the first alone is satisfied by a wall that is not
/// there: the control walks the same body straight at the goal and must
/// end up stopped against the wall, which is what every animal on the
/// island did before routes.
#[test]
fn a_route_goes_round_a_wall_that_a_straight_line_walks_into() {
    let mut sc = Scratch::barren();
    let (cx, cz) = flat_patch(&sc);
    let mut cols = Box::new(ColIndex::new());
    wall_line(&mut cols, cx, cz);
    let wall_x = cx as f32 * BUILD_CELL_M;
    let mid_z = (cz as f32 + 0.5) * BUILD_CELL_M;
    let (sx, gx) = (wall_x - 6.0, wall_x + 6.0);

    let haven = sc.haven;
    let start = Body::at(SEED, &haven, sx, mid_z);
    let mut nav = Box::new(Nav::new());
    let mut path = NavPath::default();
    let plan = {
        let mut occ = sc.occupants();
        let mut gr = Ground {
            seed: SEED,
            haven: &haven,
            cols: &cols,
            occ: &mut occ,
        };
        assert!(
            !nav.clear_line(&mut gr, sx, start.qy as f32 * POS_Y_Q, mid_z, gx, mid_z),
            "the wall must block the straight line, or this test proves nothing"
        );
        nav.plan(
            &mut gr,
            sx,
            start.qy as f32 * POS_Y_Q,
            mid_z,
            gx,
            mid_z,
            80,
            &mut path,
        )
    };
    assert_eq!(plan, Plan::Found, "a way round a 27 m wall exists");
    assert!(path.len >= 2, "a route round a wall has a corner");

    let mut body = start;
    assert!(
        walk(&mut sc, &cols, &mut body, &mut path, 1_200),
        "the body never finished the route"
    );
    let d2 = nav::dist2_cm(
        body.qx - movement::quant_xz(gx),
        body.qz - movement::quant_xz(mid_z),
    );
    assert!(d2 <= 150 * 150, "the route ended {d2} cm² from its goal");

    // The control: straight at the goal, no route.
    let mut body = start;
    let mut straight = NavPath {
        len: 1,
        next: 0,
        cx: [0; sim_core::limits::NAV_MAX_CORNERS],
        cz: [0; sim_core::limits::NAV_MAX_CORNERS],
        goal_qx: movement::quant_xz(gx),
        goal_qz: movement::quant_xz(mid_z),
        stop_cm: 80,
        partial: false,
        arrived: false,
    };
    assert!(
        !walk(&mut sc, &cols, &mut body, &mut straight, 1_200),
        "a straight walk crossed a wall"
    );
    let x = body.qx as f32 * POS_XZ_Q;
    assert!(
        x < wall_x,
        "the straight walk ended past the wall at x = {x}"
    );
}

/// **A goal inside a closed box is not reported as reached.** Four walls
/// round one build cell: the plan must come back as something other than
/// `Found`/`Direct`, and the route it does hand back must end outside.
#[test]
fn a_goal_walled_in_on_four_sides_is_not_found() {
    let sc0 = Scratch::barren();
    let (cx, cz) = flat_patch(&sc0);
    let mut cols = Box::new(ColIndex::new());
    cols.add(cx, cz, 0, LOC_EDGE_XLO, SHAPE_WALL, 0);
    cols.add(cx, cz, 0, LOC_EDGE_ZLO, SHAPE_WALL, 0);
    cols.add(cx + 1, cz, 0, LOC_EDGE_XLO, SHAPE_WALL, 0);
    cols.add(cx, cz + 1, 0, LOC_EDGE_ZLO, SHAPE_WALL, 0);
    let (gx, gz) = (
        (cx as f32 + 0.5) * BUILD_CELL_M,
        (cz as f32 + 0.5) * BUILD_CELL_M,
    );
    let (sx, sz) = (gx - 12.0, gz - 7.0);

    let mut sc = sc0;
    let haven = sc.haven;
    let start = Body::at(SEED, &haven, sx, sz);
    let mut nav = Box::new(Nav::new());
    let mut path = NavPath::default();
    let mut occ = sc.occupants();
    let mut gr = Ground {
        seed: SEED,
        haven: &haven,
        cols: &cols,
        occ: &mut occ,
    };
    let plan = nav.plan(
        &mut gr,
        sx,
        start.qy as f32 * POS_Y_Q,
        sz,
        gx,
        gz,
        80,
        &mut path,
    );
    assert!(
        matches!(plan, Plan::Partial | Plan::Unreachable),
        "a walled-in goal came back {plan:?}"
    );
    assert!(path.partial, "a route that stops short must say so");
    let (ex, ez) = (
        path.goal_qx as f32 * POS_XZ_Q,
        path.goal_qz as f32 * POS_XZ_Q,
    );
    let inside = fabs(ex - gx) < BUILD_CELL_M * 0.5 && fabs(ez - gz) < BUILD_CELL_M * 0.5;
    assert!(
        !inside,
        "the partial route ends inside the box at ({ex}, {ez})"
    );
}

/// **The tick's budget is spent and then refills.** Hard searches (the
/// walled-in goal, which floods until its cap) exhaust `NAV_EXPAND_PER_TICK`
/// and the next one is deferred rather than run; `begin_tick` puts the
/// budget back.
#[test]
fn searches_past_the_tick_budget_are_deferred() {
    let sc0 = Scratch::barren();
    let (cx, cz) = flat_patch(&sc0);
    let mut cols = Box::new(ColIndex::new());
    cols.add(cx, cz, 0, LOC_EDGE_XLO, SHAPE_WALL, 0);
    cols.add(cx, cz, 0, LOC_EDGE_ZLO, SHAPE_WALL, 0);
    cols.add(cx + 1, cz, 0, LOC_EDGE_XLO, SHAPE_WALL, 0);
    cols.add(cx, cz + 1, 0, LOC_EDGE_ZLO, SHAPE_WALL, 0);
    let (gx, gz) = (
        (cx as f32 + 0.5) * BUILD_CELL_M,
        (cz as f32 + 0.5) * BUILD_CELL_M,
    );
    let mut sc = sc0;
    let haven = sc.haven;
    let start = Body::at(SEED, &haven, gx - 12.0, gz - 7.0);
    let y = start.qy as f32 * POS_Y_Q;
    let mut nav = Box::new(Nav::new());
    let mut occ = sc.occupants();
    let mut gr = Ground {
        seed: SEED,
        haven: &haven,
        cols: &cols,
        occ: &mut occ,
    };
    nav.begin_tick();
    let mut deferred = false;
    for _ in 0..8 {
        let mut path = NavPath::default();
        if nav.plan(&mut gr, gx - 12.0, y, gz - 7.0, gx, gz, 80, &mut path) == Plan::Deferred {
            deferred = true;
            assert_eq!(path, NavPath::default(), "a deferred plan wrote a route");
            break;
        }
    }
    assert!(deferred, "eight flooding searches never ran out of budget");
    nav.begin_tick();
    let mut path = NavPath::default();
    assert_ne!(
        nav.plan(&mut gr, gx - 12.0, y, gz - 7.0, gx, gz, 80, &mut path),
        Plan::Deferred,
        "begin_tick did not refill the budget"
    );
}
