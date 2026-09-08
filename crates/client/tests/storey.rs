//! The storey follows the aim (aimed level v0, `DECISIONS.md` §open).
//!
//! **What this gates.** `ui::place::aim_from_look` reads the storey a
//! placement goes on off what the look ray MET — a wall's face means the
//! storey above it, a floor's surface or its edge means that floor's storey,
//! bare ground means the first — the way the reference's sockets do
//! (`reference/BUILDING.md` §7d). Until 2026-09-05 the storey was a
//! client-side latch stepped by `R`/`F` while the build wheel was up, and the
//! 2026-09-04 playtest's *"i cant build a second story"* is what a storey
//! nobody could find looked like.
//!
//! **Every fixture is built through `build::place`**, the sim's own verb, so
//! the column index the aim marches against is the one the game would have —
//! and every placement an aim resolves is then DRIVEN through the same verb,
//! because a level the ghost resolves and the sim refuses is a second storey
//! nobody can build either. Headless: a ray march over a bitset and a
//! heightfield, no GPU, no window.

use client::ui::place::{aim_from_look, deploy_target_at, target_at, Aim, Met};
use sim_core::build::{
    column_floor_y, place, BuildContent, Pieces, BUILD_CELL_M, LEVEL_H_M, LOC_EDGE_XLO,
    LOC_EDGE_ZLO, LOC_PLANE, SHAPE_FLOOR, SHAPE_FOUNDATION, SHAPE_STAIRS, SHAPE_WALL,
};
use sim_core::deploy::{Deploys, PLACE_DOORWAY};
use sim_core::gather::ItemStack;
use sim_core::movement::Body;
use sim_core::terrain;
use sim_core::world::{EventQueue, Player, EV_BUILD_REFUSED, EV_PIECE_PLACED};

/// The shipped island and `ghost.rs`' own buildable cell.
const SEED: u64 = 20260731;
const CX: u16 = 341;
const CZ: u16 = 341;
/// `BuildContent::probe_fixture`'s rows.
const ROW_FOUNDATION: u16 = 0;
const ROW_WALL: u16 = 1;
const ROW_FLOOR: u16 = 2;
const ROW_DOORWAY: u16 = 3;
/// The camera's eye over the feet (`render::EYE_HEIGHT`, restated: this
/// suite does not link the render tier).
const EYE_M: f32 = 1.6;

fn hv() -> &'static terrain::Haven {
    use std::sync::OnceLock;
    static H: OnceLock<terrain::Haven> = OnceLock::new();
    H.get_or_init(|| terrain::haven(SEED))
}

struct Rig {
    bc: BuildContent,
    pieces: Pieces,
    deploys: Deploys,
}

impl Rig {
    fn new() -> Self {
        Self {
            bc: BuildContent::probe_fixture(),
            pieces: Pieces::new(),
            deploys: Deploys::new(),
        }
    }

    /// Drive the sim's own verb from a rich body standing at the address.
    fn place(&mut self, row: u16, cx: u16, cz: u16, level: u8, loc: u8) -> Result<(), u32> {
        let (ax, az) = sim_core::build::anchor(cx, cz, loc);
        let mut p = Player {
            id: 7,
            active: true,
            body: Body::at(SEED, hv(), ax, az),
            ..Player::default()
        };
        for (i, slot) in p.inv.iter_mut().enumerate().take(2) {
            *slot = ItemStack {
                item: i as u16,
                count: 99,
                cond: 0,
            };
        }
        let mut ev = EventQueue::default();
        place(
            SEED,
            hv(),
            &self.bc,
            &self.deploys,
            &mut self.pieces,
            &mut p,
            0,
            row,
            cx,
            cz,
            level,
            loc,
            false,
            0,
            &mut ev,
        );
        match ev.entries().iter().find(|e| e.code == EV_BUILD_REFUSED) {
            Some(e) => Err(e.b),
            None => {
                assert!(
                    ev.entries().iter().any(|e| e.code == EV_PIECE_PLACED),
                    "the verb neither placed nor refused"
                );
                Ok(())
            }
        }
    }

    /// Where the room's level-0 floor is.
    fn floor0(&self) -> f32 {
        column_floor_y(SEED, hv(), CX, CZ, 0)
    }

    /// The look ray from `eye` toward `to`, with the body at `feet`.
    fn aim(&self, eye: [f32; 3], to: [f32; 3], feet: [f32; 3]) -> Aim {
        let d = [to[0] - eye[0], to[1] - eye[1], to[2] - eye[2]];
        let n = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        aim_from_look(
            SEED,
            hv(),
            self.pieces.cols(),
            eye,
            [d[0] / n, d[1] / n, d[2] / n],
            feet,
        )
    }
}

/// A one-cell room: a foundation and its four walls, with or without the
/// floor over it.
fn room(ceiling: bool) -> Rig {
    let mut r = Rig::new();
    r.place(ROW_FOUNDATION, CX, CZ, 0, LOC_PLANE)
        .expect("foundation");
    r.place(ROW_WALL, CX, CZ, 0, LOC_EDGE_XLO)
        .expect("west wall");
    r.place(ROW_WALL, CX + 1, CZ, 0, LOC_EDGE_XLO)
        .expect("east wall");
    r.place(ROW_WALL, CX, CZ, 0, LOC_EDGE_ZLO)
        .expect("north wall");
    r.place(ROW_WALL, CX, CZ + 1, 0, LOC_EDGE_ZLO)
        .expect("south wall");
    if ceiling {
        r.place(ROW_FLOOR, CX, CZ, 1, LOC_PLANE).expect("ceiling");
    }
    r
}

/// The room's west boundary and north boundary, world metres.
fn corner() -> (f32, f32) {
    (CX as f32 * BUILD_CELL_M, CZ as f32 * BUILD_CELL_M)
}

/// Standing in the middle of a walled room and looking up at the top of a
/// wall puts the ceiling on: the wall's face means the storey above it, and
/// the cell is the one the ray came from — the room, not the outside.
#[test]
fn a_wall_face_means_the_storey_above_and_the_room_side() {
    let mut r = room(false);
    let (x0, z0) = corner();
    let f0 = r.floor0();
    let feet = [x0 + 1.5, f0, z0 + 1.5];
    let eye = [feet[0], f0 + EYE_M, feet[2]];
    // The east wall's upper half.
    let aim = r.aim(eye, [x0 + BUILD_CELL_M, f0 + 2.5, z0 + 1.5], feet);
    assert_eq!(aim.met, Met::Wall(0), "the east wall's face was not met");
    assert!(
        aim.at.0 < x0 + BUILD_CELL_M && aim.at.0 > x0 + BUILD_CELL_M - 0.5,
        "the aim point {:?} is not just inside the wall the ray came from",
        aim.at
    );
    assert_eq!(
        aim.level_for(SHAPE_FLOOR),
        1,
        "a floor goes on top of the wall"
    );
    assert_eq!(aim.level_for(SHAPE_WALL), 1, "a wall stacks on the wall");
    assert_eq!(aim.level_for(SHAPE_STAIRS), 0, "stairs stand beside it");
    assert_eq!(
        aim.level_for(SHAPE_FOUNDATION),
        0,
        "a foundation is the ground's"
    );
    assert_eq!(
        aim.level_for_deploy(),
        0,
        "a door lands in the wall's own storey"
    );

    // ...and the placement the aim resolved is one the sim takes.
    let t = target_at(aim.at.0, aim.at.1, SHAPE_FLOOR, aim.level_for(SHAPE_FLOOR));
    assert_eq!((t.cx, t.cz, t.level, t.loc), (CX, CZ, 1, LOC_PLANE));
    r.place(ROW_FLOOR, t.cx, t.cz, t.level, t.loc)
        .expect("the sim refused the ceiling the aim resolved");
}

/// The wall is met whatever phase of the march the crossing falls in — the
/// test is on the boundary the segment spans, not on a point inside a slab a
/// 0.25 m step can straddle.
#[test]
fn a_wall_face_is_met_at_every_step_phase() {
    let r = room(true);
    let (x0, z0) = corner();
    let f0 = r.floor0();
    for i in 0..12 {
        let x = x0 + 0.4 + i as f32 * 0.09;
        let feet = [x, f0, z0 + 1.5];
        let eye = [x, f0 + EYE_M, z0 + 1.5];
        let aim = r.aim(eye, [x0 + BUILD_CELL_M, f0 + 1.5, z0 + 1.5], feet);
        assert_eq!(
            aim.met,
            Met::Wall(0),
            "from x = {x:.3} the wall was not met"
        );
    }
}

/// A ray aimed at a wall's face just under the ceiling meets the WALL. The
/// first draft fed `collide::piece_ground` the ray's own height, and its
/// feet-shaped step lid declared a hit on the ceiling half a metre before
/// the ray reached it — so a roofed room could not have a wall stacked from
/// inside. The surface test is a crossing now.
#[test]
fn a_ceiling_does_not_catch_a_ray_aimed_under_it() {
    let r = room(true);
    let (x0, z0) = corner();
    let f0 = r.floor0();
    let feet = [x0 + 1.5, f0, z0 + 1.5];
    let eye = [feet[0], f0 + EYE_M, feet[2]];
    let aim = r.aim(
        eye,
        [x0 + BUILD_CELL_M, f0 + LEVEL_H_M - 0.3, z0 + 1.5],
        feet,
    );
    assert_eq!(aim.met, Met::Wall(0), "the ceiling caught the ray first");
}

/// Standing on the floor above and looking down past its edge continues it:
/// the neighbour's level plane crossed inside the empty cell is the floor
/// socket, and the next tile the sim accepts is the one the aim resolved.
#[test]
fn a_floors_edge_continues_it() {
    let mut r = room(true);
    let (x0, z0) = corner();
    let f0 = r.floor0();
    let feet = [x0 + 1.5, f0 + LEVEL_H_M, z0 + 1.5];
    let eye = [feet[0], feet[1] + EYE_M, feet[2]];
    let aim = r.aim(
        eye,
        [x0 + BUILD_CELL_M + 0.5, f0 + LEVEL_H_M, z0 + 1.5],
        feet,
    );
    assert_eq!(aim.met, Met::Socket(1), "the floor's edge was not a socket");
    assert_eq!(aim.level_for(SHAPE_FLOOR), 1);
    let t = target_at(aim.at.0, aim.at.1, SHAPE_FLOOR, aim.level_for(SHAPE_FLOOR));
    assert_eq!((t.cx, t.cz, t.level, t.loc), (CX + 1, CZ, 1, LOC_PLANE));
    r.place(ROW_FLOOR, t.cx, t.cz, t.level, t.loc)
        .expect("the sim refused the floor the socket resolved");
}

/// The socket has a band: the same floor's plane crossed well past its edge
/// is not "continue my floor" — the ray goes on to what the crosshair is on.
#[test]
fn the_floor_socket_ends_a_metre_past_the_edge() {
    let r = room(true);
    let (x0, z0) = corner();
    let f0 = r.floor0();
    let feet = [x0 + 1.5, f0 + LEVEL_H_M, z0 + 1.5];
    let eye = [feet[0], feet[1] + EYE_M, feet[2]];
    let near = r.aim(
        eye,
        [
            x0 + BUILD_CELL_M + client::ui::place::SOCKET_BAND_M - 0.1,
            f0 + LEVEL_H_M,
            z0 + 1.5,
        ],
        feet,
    );
    assert_eq!(
        near.met,
        Met::Socket(1),
        "inside the band the edge is a socket"
    );
    let far = r.aim(
        eye,
        [
            x0 + BUILD_CELL_M + client::ui::place::SOCKET_BAND_M + 0.1,
            f0 + LEVEL_H_M,
            z0 + 1.5,
        ],
        feet,
    );
    assert_ne!(
        far.met,
        Met::Socket(1),
        "past the band the plane still caught the ray"
    );
}

/// Looking up at a floor's edge from the ground is the same socket from the
/// other side.
#[test]
fn a_floors_edge_is_a_socket_from_below_too() {
    let r = room(true);
    let (x0, z0) = corner();
    let f0 = r.floor0();
    // Outside the east wall, on the ground, looking up at the ceiling's edge.
    let feet = [x0 + BUILD_CELL_M + 2.0, f0, z0 + 1.5];
    let eye = [feet[0], f0 + EYE_M, feet[2]];
    let aim = r.aim(
        eye,
        [x0 + BUILD_CELL_M + 0.3, f0 + LEVEL_H_M, z0 + 1.5],
        feet,
    );
    assert_eq!(
        aim.met,
        Met::Socket(1),
        "the edge seen from below was not a socket"
    );
    let t = target_at(aim.at.0, aim.at.1, SHAPE_FLOOR, aim.level_for(SHAPE_FLOOR));
    assert_eq!((t.cx, t.cz, t.level), (CX + 1, CZ, 1));
}

/// Standing on the floor above and aiming at it is that floor's storey, so a
/// wall aimed at its edge from on top stands on it — and the sim agrees.
#[test]
fn a_floor_surface_is_its_own_storey() {
    let mut r = room(true);
    let (x0, z0) = corner();
    let f0 = r.floor0();
    let feet = [x0 + 1.5, f0 + LEVEL_H_M, z0 + 1.5];
    let eye = [feet[0], feet[1] + EYE_M, feet[2]];
    let aim = r.aim(eye, [x0 + 2.8, f0 + LEVEL_H_M, z0 + 1.5], feet);
    assert_eq!(aim.met, Met::Floor(1), "the floor's surface was not met");
    assert_eq!(aim.level_for(SHAPE_WALL), 1);
    let t = target_at(aim.at.0, aim.at.1, SHAPE_WALL, aim.level_for(SHAPE_WALL));
    assert_eq!((t.cx, t.cz, t.level, t.loc), (CX + 1, CZ, 1, LOC_EDGE_XLO));
    r.place(ROW_WALL, t.cx, t.cz, t.level, t.loc)
        .expect("the sim refused the upper-storey wall the aim resolved");
}

/// A door aimed at a doorway on an upper storey lands in THAT doorway — the
/// doorway-class deploy takes the storey the frame is on, not the one above
/// it that a wall would stack to.
#[test]
fn a_door_lands_in_its_doorway_on_any_storey() {
    let mut r = room(true);
    r.place(ROW_DOORWAY, CX + 1, CZ, 1, LOC_EDGE_XLO)
        .expect("a doorway on the east wall's top");
    let (x0, z0) = corner();
    let f0 = r.floor0();
    let feet = [x0 + 1.5, f0 + LEVEL_H_M, z0 + 1.5];
    let eye = [feet[0], feet[1] + EYE_M, feet[2]];
    let aim = r.aim(
        eye,
        [x0 + BUILD_CELL_M, f0 + LEVEL_H_M + 1.0, z0 + 0.4],
        feet,
    );
    assert_eq!(aim.met, Met::Wall(1), "the doorway's frame was not met");
    assert_eq!(aim.level_for_deploy(), 1);
    let t = deploy_target_at(aim.at.0, aim.at.1, PLACE_DOORWAY, aim.level_for_deploy());
    assert_eq!((t.cx, t.cz, t.level, t.loc), (CX + 1, CZ, 1, LOC_EDGE_XLO));
    // ...while a wall would stack over it.
    assert_eq!(aim.level_for(SHAPE_WALL), 2);
}

/// Nothing met: the storey is the one the feet stand on, so looking at the
/// sky from an upper floor still builds on that floor, and from the ground
/// on the ground.
#[test]
fn the_sky_answers_the_storey_the_feet_stand_on() {
    let r = room(true);
    let (x0, z0) = corner();
    let f0 = r.floor0();
    let up = [x0 + 1.5, f0 + 30.0, z0 + 1.5];
    let on_ceiling = [x0 + 1.5, f0 + LEVEL_H_M, z0 + 1.5];
    let aim = r.aim(
        [on_ceiling[0], on_ceiling[1] + EYE_M, on_ceiling[2]],
        up,
        on_ceiling,
    );
    assert_eq!(aim.met, Met::Nothing);
    assert_eq!(aim.standing, 1, "the feet are on the first floor up");
    assert_eq!(aim.level_for(SHAPE_WALL), 1);

    let on_ground = [x0 + 1.5 + 2.0 * BUILD_CELL_M, f0, z0 + 1.5];
    let ground_y = terrain::ground(SEED, hv(), on_ground[0], on_ground[2]);
    let on_ground = [on_ground[0], ground_y, on_ground[2]];
    let aim = r.aim(
        [on_ground[0], ground_y + EYE_M, on_ground[2]],
        [on_ground[0], ground_y + 30.0, on_ground[2]],
        on_ground,
    );
    assert_eq!(aim.met, Met::Nothing);
    assert_eq!(aim.standing, 0);
}

/// Bare ground is the first storey, whatever the feet are on: from a floor
/// with an open side, the ground past its edge is level 0.
///
/// Not from a walled room's roof, and the reason is worth keeping: from an
/// eye 4.6 m over its floor, every ray that reaches ground inside the march
/// range crosses either the wall's band or the floor's socket band first —
/// which is not a defect, it is what "everything in reach from up there is
/// the base's edge" means. The first draft aimed nine metres out and got
/// `Met::Nothing`, because nine metres is past `AIM_RANGE_M`.
#[test]
fn bare_ground_is_the_first_storey() {
    let mut r = Rig::new();
    r.place(ROW_FOUNDATION, CX, CZ, 0, LOC_PLANE)
        .expect("foundation");
    r.place(ROW_WALL, CX, CZ, 0, LOC_EDGE_XLO)
        .expect("west wall");
    r.place(ROW_FLOOR, CX, CZ, 1, LOC_PLANE)
        .expect("a floor on the one wall");
    let (x0, z0) = corner();
    let f0 = r.floor0();
    // Standing at the open south edge — from the cell's centre the plane is
    // met (the own floor, or its socket) before any reachable ground is —
    // and looking down at the ground five metres out, which crosses the
    // plane past the socket band and reaches the ground inside the march.
    let feet = [x0 + 1.5, f0 + LEVEL_H_M, z0 + 2.8];
    let eye = [feet[0], feet[1] + EYE_M, feet[2]];
    let gx = x0 + 1.5;
    let gz = feet[2] + 5.0;
    let gy = terrain::ground(SEED, hv(), gx, gz);
    let aim = r.aim(eye, [gx, gy, gz], feet);
    assert_eq!(
        aim.met,
        Met::Ground,
        "the ground past the open side was not met"
    );
    assert_eq!(aim.level_for(SHAPE_WALL), 0);
    assert_eq!(aim.standing, 1, "the feet are still on the first floor up");
}
