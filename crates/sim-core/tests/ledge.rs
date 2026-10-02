//! Stepping off an edge drops a body beside the thing it stood on, never
//! into it — the Black Ziggurat's terraces and a base's roof alike.
//!
//! Driven through `movement::step`, the code the shard runs and the client
//! predicts with, so what passes here is what the server enforces.

// A test reports where it went wrong: host code, not sim code.
#![allow(
    clippy::disallowed_macros,
    clippy::disallowed_methods,
    clippy::disallowed_types
)]

use sim_core::build::{
    self, BUILD_CELL_M, LEVEL_H_M, LOC_EDGE_XLO, LOC_EDGE_ZLO, LOC_PLANE, SHAPE_FLOOR,
    SHAPE_FOUNDATION, SHAPE_WALL,
};
use sim_core::collide::{plane_blocked, ColIndex};
use sim_core::input::{InputFrame, BTN_CROUCH, BTN_JUMP, BTN_SPRINT};
use sim_core::kit;
use sim_core::movement::{self, Body, POS_XZ_Q, POS_Y_Q, STEP_UP};
use sim_core::occupy::{Occupants, Pristine, Scratch, SlotCache};
use sim_core::terrain::{self, Haven, ScatterTable};

/// The yaw LUT bearing pointing along (dx, dz).
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

fn pos(b: &Body) -> (f32, f32, f32) {
    (
        b.qx as f32 * POS_XZ_Q,
        b.qz as f32 * POS_XZ_Q,
        b.qy as f32 * POS_Y_Q,
    )
}

/// A tiny deterministic generator for the fuzz walks.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) as u32
    }
}

fn random_frame(rng: &mut Lcg, toward: u16) -> InputFrame {
    let r = rng.next();
    let mut buttons = 0;
    if r & 0x30 == 0 {
        buttons |= BTN_JUMP;
    }
    if r & 0xC0 == 0x40 {
        buttons |= BTN_CROUCH;
    }
    if r & 0x300 == 0x100 {
        buttons |= BTN_SPRINT;
    }
    InputFrame {
        yaw: toward.wrapping_add((rng.next() & 0xFFFF) as u16),
        move_x: (rng.next() % 255) as i32 as i8,
        move_z: (rng.next() % 255) as i32 as i8,
        buttons,
        ..InputFrame::default()
    }
}

// --- the ziggurat -----------------------------------------------------------

const ZIG_SEED: u64 = 20_260_731;

fn zig_occ<'a>(h: &'a Haven, table: &'a ScatterTable, cache: &'a mut SlotCache) -> Occupants<'a> {
    Occupants {
        doors: 0,
        table,
        haven: h,
        harvested: &Pristine,
        cache,
    }
}

/// A terrace's edge: a local start on its top, the local outward direction,
/// and the top's height above the floor.
#[derive(Clone, Copy)]
struct Edge {
    start: (f32, f32),
    out: (f32, f32),
    top: f32,
}

const fn edge(x: f32, z: f32, ox: f32, oz: f32, top: f32) -> Edge {
    Edge {
        start: (x, z),
        out: (ox, oz),
        top,
    }
}

/// An edge of every terrace, away from the front stairs (|x| <= 3) so a
/// step box is not what the body lands on.
const EDGES: [Edge; 8] = [
    edge(25.7, 15.0, 1.0, 0.0, 10.0),
    edge(-25.7, 15.0, -1.0, 0.0, 10.0),
    edge(15.0, 25.7, 0.0, 1.0, 10.0),
    edge(15.0, -25.7, 0.0, -1.0, 10.0),
    edge(19.7, 10.0, 1.0, 0.0, 15.0),
    edge(-10.0, 19.7, 0.0, 1.0, 15.0),
    edge(13.7, 0.0, 1.0, 0.0, 20.0),
    edge(7.7, 0.0, 1.0, 0.0, 25.0),
];

/// Walk off a terrace's edge, wait `wait` ticks, then press back toward the
/// face. The body must never overlap the stone it fell beside.
#[test]
fn stepping_off_a_terrace_never_lands_inside_it() {
    let h = terrain::haven(ZIG_SEED);
    let z = h.ziggurat;
    assert!(z.live);
    let table = ScatterTable::alpha_default();
    let p = z.placed();
    let mut failures = Vec::new();
    for Edge { start, out, top } in EDGES {
        let (sx, sz) = kit::to_world(&p, start.0, start.1);
        let (ox, oz) = kit::to_world(&p, start.0 + out.0, start.1 + out.1);
        let away = bearing(ox - sx, oz - sz);
        let back = away.wrapping_add(0x8000);
        for creep in [10i8, 40, 127] {
            for wait in [0u32, 2, 5, 8, 12, 20, 40] {
                let mut cache = SlotCache::new();
                let mut occ = zig_occ(&h, &table, &mut cache);
                let mut b = Body {
                    qx: movement::quant_xz(sx),
                    qy: movement::quant_y(z.floor_y + top),
                    qz: movement::quant_xz(sz),
                    qvy: 0,
                    grounded: true,
                };
                let mut fell = false;
                let mut left = wait;
                for t in 0..200u32 {
                    let (_, _, y) = pos(&b);
                    if !fell && y < z.floor_y + top - 0.005 {
                        fell = true;
                    }
                    let f = if !fell {
                        InputFrame {
                            yaw: away,
                            move_z: creep,
                            ..InputFrame::default()
                        }
                    } else if left > 0 {
                        left -= 1;
                        InputFrame::default()
                    } else {
                        InputFrame {
                            yaw: back,
                            move_z: 127,
                            ..InputFrame::default()
                        }
                    };
                    movement::step(ZIG_SEED, &h, &ColIndex::new(), &mut occ, &mut b, &f);
                    let (x, wz, y) = pos(&b);
                    if occ.blocks(ZIG_SEED, x, wz, y) {
                        let (lx, lz) = kit::to_local(&p, x, wz);
                        failures.push(format!(
                            "edge {start:?} creep {creep} wait {wait}: tick {t} overlaps stone at \
                             local ({lx:.2}, {lz:.2}) feet {:.2}",
                            y - z.floor_y
                        ));
                        break;
                    }
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} walks ended in the stone:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// Random hands at every terrace edge — sprints, jumps, crouches, every
/// heading: the body is never left overlapping the stone.
#[test]
fn fumbling_at_a_terrace_edge_never_enters_the_stone() {
    let h = terrain::haven(ZIG_SEED);
    let z = h.ziggurat;
    let table = ScatterTable::alpha_default();
    let p = z.placed();
    let mut rng = Lcg(0x2161_6667);
    let mut failures = Vec::new();
    for run in 0..240u32 {
        let Edge { start, out, top } = EDGES[run as usize % EDGES.len()];
        let jitter = (rng.next() % 600) as f32 * 0.01 - 3.0;
        let (lx, lz) = (start.0 + out.1 * jitter, start.1 + out.0 * jitter);
        let (sx, sz) = kit::to_world(&p, lx, lz);
        let (ox, oz) = kit::to_world(&p, lx + out.0, lz + out.1);
        let toward = bearing(ox - sx, oz - sz);
        let mut cache = SlotCache::new();
        let mut occ = zig_occ(&h, &table, &mut cache);
        let mut b = Body {
            qx: movement::quant_xz(sx),
            qy: movement::quant_y(z.floor_y + top),
            qz: movement::quant_xz(sz),
            qvy: 0,
            grounded: true,
        };
        for t in 0..300u32 {
            let f = random_frame(&mut rng, toward);
            movement::step(ZIG_SEED, &h, &ColIndex::new(), &mut occ, &mut b, &f);
            let (x, wz, y) = pos(&b);
            if occ.blocks(ZIG_SEED, x, wz, y) {
                let (lx, lz) = kit::to_local(&p, x, wz);
                failures.push(format!(
                    "run {run}: tick {t} overlaps stone at local ({lx:.2}, {lz:.2}) feet {:.2}",
                    y - z.floor_y
                ));
                break;
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} of 240 runs ended in the stone:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

// --- a base -----------------------------------------------------------------

const SEED: u64 = 20260731;
const CX: u16 = 682;
const CZ: u16 = 682;

fn haven() -> &'static Haven {
    use std::cell::OnceCell;
    thread_local! {
        static H: OnceCell<&'static Haven> = const { OnceCell::new() };
    }
    H.with(|h| *h.get_or_init(|| Box::leak(Box::new(terrain::haven(SEED)))))
}

fn put(cols: &mut ColIndex, cx: u16, cz: u16, level: u8, loc: u8, shape: u8) {
    let band = build::terrain_band(SEED, haven(), CX, CZ);
    let plate = (band - build::terrain_band(SEED, haven(), cx, cz)) as i8;
    cols.add(cx, cz, level, loc, shape, plate);
}

/// A sealed 2×2 box: foundations, a storey of wall all round, a roof.
fn sealed_box() -> ColIndex {
    let mut cols = ColIndex::new();
    for dx in 0..2u16 {
        for dz in 0..2u16 {
            put(&mut cols, CX + dx, CZ + dz, 0, LOC_PLANE, SHAPE_FOUNDATION);
            put(&mut cols, CX + dx, CZ + dz, 1, LOC_PLANE, SHAPE_FLOOR);
        }
    }
    for d in 0..2u16 {
        put(&mut cols, CX, CZ + d, 0, LOC_EDGE_XLO, SHAPE_WALL);
        put(&mut cols, CX + 2, CZ + d, 0, LOC_EDGE_XLO, SHAPE_WALL);
        put(&mut cols, CX + d, CZ, 0, LOC_EDGE_ZLO, SHAPE_WALL);
        put(&mut cols, CX + d, CZ + 2, 0, LOC_EDGE_ZLO, SHAPE_WALL);
    }
    cols
}

/// Inside the box's one room: centre between the walls' inner faces and the
/// head under the roof.
fn in_room(x: f32, z: f32, y: f32) -> bool {
    let (x0, z0) = (CX as f32 * BUILD_CELL_M, CZ as f32 * BUILD_CELL_M);
    let x1 = x0 + 2.0 * BUILD_CELL_M;
    let z1 = z0 + 2.0 * BUILD_CELL_M;
    let roof = build::column_floor_y(SEED, haven(), CX, CZ, 0) + LEVEL_H_M;
    x > x0 && x < x1 && z > z0 && z < z1 && y < roof - STEP_UP
}

/// A body standing exactly ON a wall's plane, inside the wall's storey —
/// where creeping off a roof's high edge used to put it — leaves by the high
/// side only. The plane belongs to the cell `build_cell_of` puts it in.
#[test]
fn a_body_on_a_wall_plane_leaves_by_the_high_side_only() {
    let mut cols = ColIndex::new();
    put(&mut cols, CX, CZ, 0, LOC_PLANE, SHAPE_FOUNDATION);
    put(&mut cols, CX + 1, CZ, 0, LOC_EDGE_XLO, SHAPE_WALL);
    put(&mut cols, CX, CZ + 1, 0, LOC_EDGE_ZLO, SHAPE_WALL);
    let feet = build::column_floor_y(SEED, haven(), CX, CZ, 0) + 0.5;
    // The quantized point exactly on each plane.
    let px = movement::quant_xz((CX + 1) as f32 * BUILD_CELL_M) as f32 * POS_XZ_Q;
    let pz = movement::quant_xz((CZ + 1) as f32 * BUILD_CELL_M) as f32 * POS_XZ_Q;
    assert_eq!(build::build_cell_of(px), (CX + 1) as i32);
    assert_eq!(build::build_cell_of(pz), (CZ + 1) as i32);
    let mid = CZ as f32 * BUILD_CELL_M + 1.5;
    let blocked = |x: f32, z: f32, nx: f32, nz: f32| {
        sim_core::collide::blocked(SEED, haven(), &cols, x, z, nx, nz, feet)
    };
    assert!(
        blocked(px, mid, px - 0.09, mid),
        "x plane: walked to the low side"
    );
    assert!(
        !blocked(px, mid, px + 0.09, mid),
        "x plane: held on the high side"
    );
    let midx = CX as f32 * BUILD_CELL_M + 1.5;
    assert!(
        blocked(midx, pz, midx, pz - 0.09),
        "z plane: walked to the low side"
    );
    assert!(
        !blocked(midx, pz, midx, pz + 0.09),
        "z plane: held on the high side"
    );
    // The rule leans on the wall's side and the cell agreeing about a body
    // on the plane, at every boundary of the island, not just this one.
    for k in 1..=(sim_core::terrain::ISLAND_SIZE / BUILD_CELL_M) as i32 {
        let plane = k as f32 * BUILD_CELL_M;
        let x = movement::quant_xz(plane) as f32 * POS_XZ_Q;
        assert_eq!(
            x - plane < 0.0,
            build::build_cell_of(x) < k,
            "boundary {k}: the wall and the cell disagree about {x}"
        );
    }
}

/// Creep to a roof edge one quantum at a time, wait, then push back toward
/// the base — every edge, every wait. The room below stays sealed.
#[test]
fn creeping_off_a_roof_never_drops_into_the_room() {
    let cols = sealed_box();
    let mut sc = Scratch::barren();
    let roof = build::column_floor_y(SEED, haven(), CX, CZ, 0) + LEVEL_H_M;
    let (x0, z0) = (CX as f32 * BUILD_CELL_M, CZ as f32 * BUILD_CELL_M);
    let span = 2.0 * BUILD_CELL_M;
    // (start on the roof, outward direction)
    let starts = [
        ((x0 + span - 0.09, z0 + 3.0), (1.0, 0.0)),
        ((x0 + 0.09, z0 + 3.0), (-1.0, 0.0)),
        ((x0 + 3.0, z0 + span - 0.09), (0.0, 1.0)),
        ((x0 + 3.0, z0 + 0.09), (0.0, -1.0)),
        ((x0 + span - 0.09, z0 + span - 0.09), (1.0, 1.0)),
    ];
    let mut failures = Vec::new();
    for (start, out) in starts {
        let away = bearing(out.0, out.1);
        let back = away.wrapping_add(0x8000);
        for creep in [12i8, 20, 40, 127] {
            for wait in [0u32, 3, 6, 9, 12, 20, 40] {
                let mut b = Body {
                    qx: movement::quant_xz(start.0),
                    qy: movement::quant_y(roof),
                    qz: movement::quant_xz(start.1),
                    qvy: 0,
                    grounded: true,
                };
                let mut fell = false;
                let mut left = wait;
                for t in 0..200u32 {
                    let (_, _, y) = pos(&b);
                    if !fell && y < roof - 0.005 {
                        fell = true;
                    }
                    let f = if !fell {
                        InputFrame {
                            yaw: away,
                            move_z: creep,
                            buttons: BTN_CROUCH,
                            ..InputFrame::default()
                        }
                    } else if left > 0 {
                        left -= 1;
                        InputFrame::default()
                    } else {
                        InputFrame {
                            yaw: back,
                            move_z: 127,
                            ..InputFrame::default()
                        }
                    };
                    movement::step(SEED, haven(), &cols, &mut sc.occupants(), &mut b, &f);
                    let (x, z, y) = pos(&b);
                    // In the room is the bug; down the roof's flank inside
                    // its slab is how a body got there (the veto lift).
                    let flank = plane_blocked(SEED, haven(), &cols, x, z, y);
                    if in_room(x, z, y) || flank {
                        failures.push(format!(
                            "start {start:?} creep {creep} wait {wait}: tick {t} {} at \
                             ({:.2}, {:.2}) feet {:.2} under the roof",
                            if flank { "in the slab" } else { "in the room" },
                            x - x0,
                            z - z0,
                            roof - y
                        ));
                        break;
                    }
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} creeps ended in the room or its roof's slab:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
