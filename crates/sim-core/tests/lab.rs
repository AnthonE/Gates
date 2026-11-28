//! The lab under the island, v0 (`lab.rs`, `ARC.md` act IV): one keep on
//! every island we try has the lab under it; its door holds until the Gate
//! opens and then a body walks in at the blockhouse and down the stair to
//! the floor; the court above still holds a body up; nothing walks out
//! through a wall; the crates stand on the lab's floor; and the dwellers
//! stand up in its rooms only once the Gate opens, and stay there.
// Host-side test: labels are Strings, and `abs` measures, nothing simulates.
#![allow(
    clippy::disallowed_types,
    clippy::disallowed_macros,
    clippy::disallowed_methods
)]

use sim_core::collide::ColIndex;
use sim_core::input::InputFrame;
use sim_core::lab::{self, FLOOR_Y, LAB_DOOR_BIT};
use sim_core::landmark::{self, Landmark};
use sim_core::limits::{MAX_LAB_DWELLERS, TICK_HZ};
use sim_core::mob::{self, MobContent};
use sim_core::movement::{self, quant_xz, quant_y, Body, POS_XZ_Q, POS_Y_Q};
use sim_core::occupy::Scratch;
use sim_core::terrain::{self, ScatterTable, CELL_SIZE};
use sim_core::world::World;

const SHIPPED: u64 = 20_260_731;
const SEEDS: [u64; 6] = [SHIPPED, 1, 7, 11, 42, 1234];

fn host(h: &terrain::Haven) -> Landmark {
    *lab::host(&h.marks).expect("no lab on this island")
}

/// The yaw byte that walks along local (`lx`, `lz`) of `m`.
fn heading(m: &Landmark, lx: f32, lz: f32) -> u16 {
    let (wx, wz) = landmark::to_world(m, lx, lz);
    let (dx, dz) = (wx - m.x, wz - m.z);
    let best = (0..256u16)
        .max_by(|&a, &b| {
            let (ax, az) = sim_core::yaw_dir(a << 8);
            let (bx, bz) = sim_core::yaw_dir(b << 8);
            (ax * dx + az * dz).total_cmp(&(bx * dx + bz * dz))
        })
        .unwrap();
    best << 8
}

fn local(m: &Landmark, b: &Body) -> (f32, f32, f32) {
    let (x, z) = (b.qx as f32 * POS_XZ_Q, b.qz as f32 * POS_XZ_Q);
    let (lx, lz) = landmark::to_local(m, x, z);
    (lx, b.qy as f32 * POS_Y_Q - m.y, lz)
}

/// A body standing at local (`lx`, `ly`, `lz`) of `m`.
fn stand(m: &Landmark, lx: f32, ly: f32, lz: f32) -> Body {
    let (x, z) = landmark::to_world(m, lx, lz);
    Body {
        qx: quant_xz(x),
        qy: quant_y(m.y + ly),
        qz: quant_xz(z),
        qvy: 0,
        grounded: true,
    }
}

/// **One keep, deep enough, on every island.** The host is the commonest
/// kind, high enough that its floor is dry, and its ground is cut to the pit
/// under the court while the keep's own vault stands where it stood.
#[test]
fn every_island_cuts_its_lab_under_one_keep() {
    for seed in SEEDS {
        let h = terrain::haven(seed);
        assert_eq!(
            h.marks.iter().filter(|m| m.lab).count(),
            1,
            "seed {seed}: not exactly one lab"
        );
        let m = host(&h);
        assert_eq!(m.kind, lab::HOST_KIND, "seed {seed}");
        assert!(
            m.live && m.y >= lab::MIN_BASE_Y,
            "seed {seed}: base {}",
            m.y
        );
        let (cx, cz) = landmark::to_world(&m, 0.0, 0.0);
        let pit = terrain::ground(seed, &h, cx, cz);
        assert!(
            (pit - (m.y + lab::PIT_Y)).abs() < 0.01,
            "seed {seed}: the court's ground is {pit:.2}, base {:.2}",
            m.y
        );
        assert!(pit > terrain::BEACH_MAX_H, "seed {seed}: the lab is wet");
        // Outside the cut and its apron the ground is the host's own.
        let (fx, fz) = landmark::to_world(&m, -20.0, 18.0);
        let mut bare = h;
        for mk in bare.marks.iter_mut() {
            mk.lab = false;
        }
        assert_eq!(
            terrain::ground(seed, &h, fx, fz).to_bits(),
            terrain::ground(seed, &bare, fx, fz).to_bits(),
            "seed {seed}: the carve reached past its apron"
        );
    }
}

/// **The crates are the lab's.** Each lab crate is its own cell's slot,
/// seated on the lab's floor; the court's crates are gone from the host
/// and its vault is still there.
#[test]
fn the_lab_crates_stand_on_its_floor_and_the_court_crates_are_gone() {
    let table = ScatterTable::alpha_default();
    for seed in SEEDS {
        let h = terrain::haven(seed);
        let m = host(&h);
        let slot = |x: f32, z: f32| {
            terrain::scatter(
                seed,
                &table,
                &h,
                (x / CELL_SIZE) as i32,
                (z / CELL_SIZE) as i32,
            )
        };
        for k in 0..lab::CRATES.len() {
            let (x, y, z, _, occ) = lab::crate_world(&m, k).unwrap();
            let s = slot(x, z);
            assert_eq!(s.occupant, occ, "seed {seed}: lab crate {k} lost its cell");
            assert_eq!((s.x, s.y, s.z), (x, y, z));
            assert!((s.y - (m.y + FLOOR_Y)).abs() < 1e-4);
        }
        let mut k = 0;
        while let Some((x, z, _, occ)) = landmark::anchor(&m, k) {
            let a = landmark::anchors(m.kind)[k];
            k += 1;
            let s = slot(x, z);
            if lab::over_pit(a.0, a.1) {
                assert!(
                    !(s.occupant == occ && s.x == x && s.z == z),
                    "seed {seed}: the court's {occ:?} still stands over the lab"
                );
            } else {
                assert_eq!(s.occupant, occ, "seed {seed}: the host lost its {occ:?}");
            }
        }
    }
}

/// Walk `frame` for `ticks`, returning the body's local track.
fn walk(
    seed: u64,
    scratch: &mut Scratch<sim_core::occupy::Pristine>,
    doors: u32,
    body: &mut Body,
    frame: &InputFrame,
    ticks: usize,
) -> Vec<(f32, f32, f32)> {
    let h = scratch.haven;
    let m = host(&h);
    let cols = ColIndex::new();
    let mut occ = scratch.occupants();
    occ.doors = doors;
    let mut out = Vec::with_capacity(ticks);
    for _ in 0..ticks {
        movement::step(seed, &h, &cols, &mut occ, body, frame);
        out.push(local(&m, body));
    }
    out
}

/// **The door holds, then it opens on the stair.** From the apron in front
/// of the blockhouse a body walking in stops at the shut door; with the
/// lab's bit set it walks through, down the sixteen steps and out onto the
/// lab's floor in the hall.
#[test]
fn the_door_holds_until_open_then_the_stair_goes_down_to_the_floor() {
    for seed in SEEDS {
        let mut scratch = Scratch::live(seed);
        let h = scratch.haven;
        let m = host(&h);
        let north = InputFrame {
            yaw: heading(&m, 0.0, 1.0),
            move_z: 127,
            ..InputFrame::default()
        };
        let (sx, sz) = landmark::to_world(&m, 5.7, -23.0);
        let start = Body::at(seed, &h, sx, sz);

        let mut b = start;
        let shut = walk(seed, &mut scratch, 0, &mut b, &north, 10 * TICK_HZ as usize);
        let (_, y, z) = *shut.last().unwrap();
        assert!(
            z < -19.85 && z > -21.0,
            "seed {seed}: the shut door let a body to z {z:.2}"
        );
        assert!(y.abs() < 0.3, "seed {seed}: at the door at {y:.2}");

        let mut b = start;
        let open = walk(
            seed,
            &mut scratch,
            LAB_DOOR_BIT,
            &mut b,
            &north,
            20 * TICK_HZ as usize,
        );
        let (x, y, z) = *open.last().unwrap();
        assert!(
            z > -7.5 && (y - FLOOR_Y).abs() < 0.05,
            "seed {seed}: the stair stopped a body at ({x:.2}, {y:.2}, {z:.2})"
        );
        // Down, never up through the ceiling or into the dirt.
        for &(_, y, z) in &open {
            assert!(
                (FLOOR_Y - 0.02..0.2).contains(&y),
                "seed {seed}: {y:.2} at z {z:.2}"
            );
        }
        // And back up and out the way it came.
        let south = InputFrame {
            yaw: heading(&m, 0.0, -1.0),
            move_z: 127,
            ..InputFrame::default()
        };
        let back = walk(
            seed,
            &mut scratch,
            LAB_DOOR_BIT,
            &mut b,
            &south,
            20 * TICK_HZ as usize,
        );
        assert!(
            back.iter().any(|&(_, y, z)| z < -20.5 && y.abs() < 0.3),
            "seed {seed}: the way out stopped a body at {:?}",
            back.last()
        );
        for &(x, _, z) in back.iter().chain(open.iter()) {
            assert!(
                !(-19.4..=-7.0).contains(&z) || (4.4..=7.0).contains(&x),
                "seed {seed}: through the stair's wall at ({x:.2}, {z:.2})"
            );
        }
    }
}

/// **The court still holds a body up.** Walking across the keep's flagged
/// court over the lab, a body stays on the flags.
#[test]
fn the_court_over_the_lab_holds_a_body_up() {
    for seed in SEEDS {
        let mut scratch = Scratch::live(seed);
        let m = host(&scratch.haven);
        let east = InputFrame {
            yaw: heading(&m, 1.0, 0.0),
            move_z: 127,
            ..InputFrame::default()
        };
        let mut b = stand(&m, -6.0, 0.06, -6.0);
        let track = walk(
            seed,
            &mut scratch,
            LAB_DOOR_BIT,
            &mut b,
            &east,
            4 * TICK_HZ as usize,
        );
        let (x, _, _) = *track.last().unwrap();
        assert!(x > 3.0, "seed {seed}: the court stopped a body at x {x:.2}");
        for &(x, y, _) in &track {
            assert!(
                y > -0.1,
                "seed {seed}: fell into the lab at x {x:.2} ({y:.2})"
            );
        }
    }
}

/// **Nothing walks out through a wall.** From the middle of each room a
/// body walking any of eight ways for ten seconds is still in the lab, on
/// its floor.
#[test]
fn nobody_walks_out_of_the_lab_but_by_its_stair() {
    for seed in [SHIPPED, 7, 42] {
        let mut scratch = Scratch::live(seed);
        let m = host(&scratch.haven);
        for (sx, sz) in [(0.0, -4.0), (-3.5, 3.5), (3.5, 3.5)] {
            for k in 0..8 {
                let (dx, dz) = sim_core::yaw_dir((k * 32) << 8);
                let frame = InputFrame {
                    yaw: heading(&m, dx, dz),
                    move_z: 127,
                    buttons: sim_core::input::BTN_SPRINT,
                    ..InputFrame::default()
                };
                let mut b = stand(&m, sx, FLOOR_Y, sz);
                let track = walk(seed, &mut scratch, LAB_DOOR_BIT, &mut b, &frame, 300);
                for &(x, y, z) in &track {
                    let in_rooms = x.abs() <= 7.0 && z.abs() <= 7.0;
                    let in_shaft = (4.4..=7.0).contains(&x) && (-20.0..=-6.9).contains(&z);
                    assert!(
                        in_rooms || in_shaft,
                        "seed {seed}: out of the lab at ({x:.2}, {z:.2}) from ({sx}, {sz})"
                    );
                    // On the floor, or on the stair up out of it.
                    let floor = (y - FLOOR_Y).abs() < 0.05;
                    assert!(
                        floor || (in_shaft && z < -7.0 && y < 0.2),
                        "seed {seed}: off the floor ({y:.2}) at ({x:.2}, {z:.2})"
                    );
                }
            }
        }
    }
}

/// Light a work past act II: THE SEVERED GATE is open.
fn open_gate(w: &mut World) {
    use sim_core::works::{WorksContent, WORK_LIT};
    let mut wc = WorksContent::EMPTY;
    wc.count = 1;
    wc.defs[0].act = 3;
    wc.defs[0].floor = 1;
    w.works_def = wc;
    w.works.w[0].state = WORK_LIT;
    w.works.refresh(&w.works_def);
    assert!(w.works.gate_open(&w.works_def), "the Gate is not open");
}

fn dwellers(w: &World) -> impl Iterator<Item = (usize, &mob::Mob)> {
    w.mobs
        .m
        .iter()
        .enumerate()
        .filter(|(s, _)| mob::is_lab_slot(*s))
}

/// **The dwellers wait for the Gate, then keep the lab.** Enrolled and
/// shut in until THE SEVERED GATE opens; the tick it does, the lab's door
/// stands open and every dweller is up in its room; half a minute on, each
/// is still in the lab.
#[test]
fn the_dwellers_stand_up_in_the_lab_once_the_gate_opens_and_stay() {
    let seed = 11;
    let mut w = World::new(seed);
    w.mob = MobContent::probe_fixture();
    for _ in 0..10 * TICK_HZ as u64 {
        w.tick(&[]);
    }
    assert_eq!(w.card_door_bits & LAB_DOOR_BIT, 0, "the lab is open");
    assert_eq!(
        dwellers(&w).filter(|(_, m)| m.homed).count(),
        MAX_LAB_DWELLERS
    );
    assert_eq!(dwellers(&w).filter(|(_, m)| m.alive).count(), 0);
    let m = host(&w.haven);

    open_gate(&mut w);
    w.tick(&[]);
    assert_ne!(
        w.card_door_bits & LAB_DOOR_BIT,
        0,
        "the Gate opened, the lab did not"
    );
    assert_eq!(
        dwellers(&w).filter(|(_, m)| m.alive).count(),
        MAX_LAB_DWELLERS
    );
    for _ in 0..30 * TICK_HZ as u64 {
        w.tick(&[]);
    }
    for (s, d) in dwellers(&w) {
        assert!(d.alive, "dweller {s} died alone");
        let (x, y, z) = local(&m, &d.body);
        let in_rooms = x.abs() <= 7.0 && z.abs() <= 7.0;
        let in_stair = (4.4..=7.0).contains(&x) && (-20.0..=-6.9).contains(&z);
        assert!(
            in_rooms || in_stair,
            "dweller {s} left the lab: ({x:.2}, {y:.2}, {z:.2})"
        );
        assert_eq!(mob::kind_of(s), mob::MOB_STRAY);
    }
}

/// The lab's boxes collide only where the door is shut: the leaf stops a
/// body with the bit clear and not with it set, and nothing else of the
/// lab knows the mask.
#[test]
fn only_the_door_follows_the_mask() {
    let h = terrain::haven(SHIPPED);
    let m = host(&h);
    let (x, z) = landmark::to_world(&m, 5.7, -19.7);
    assert!(lab::blocks(&h.marks, 0, x, z, m.y + 0.1, 0.3, 1.7));
    assert!(!lab::blocks(
        &h.marks,
        LAB_DOOR_BIT,
        x,
        z,
        m.y + 0.1,
        0.3,
        1.7
    ));
    let (x, z) = landmark::to_world(&m, 3.0, -15.0);
    assert!(lab::blocks(
        &h.marks,
        LAB_DOOR_BIT,
        x,
        z,
        m.y + 0.1,
        0.3,
        1.7
    ));
    assert!(lab::blocks(&h.marks, 0, x, z, m.y + 0.1, 0.3, 1.7));
}

/// **No bite through a partition.** A dweller held at the hall's wall is a
/// metre from the hall across it, inside a stray's reach; the line between
/// them is walled. Across a room, through a doorway and down the stair (at
/// the higher body's feet) it is not.
#[test]
fn a_bite_never_crosses_a_lab_wall() {
    let h = terrain::haven(SHIPPED);
    let m = host(&h);
    let line = |a: (f32, f32), b: (f32, f32), ly: f32| {
        let (x0, z0) = landmark::to_world(&m, a.0, a.1);
        let (x1, z1) = landmark::to_world(&m, b.0, b.1);
        lab::walled(&h.marks, LAB_DOOR_BIT, x0, z0, x1, z1, m.y + ly)
    };
    // The hall's wall, and the wall between the rooms.
    assert!(line((-1.5, -0.4), (-1.5, -1.6), FLOOR_Y));
    assert!(line((-0.6, 3.5), (0.6, 3.5), FLOOR_Y));
    // Inside one room, and through the hall's west doorway.
    assert!(!line((-3.5, 3.5), (-3.5, 1.5), FLOOR_Y));
    assert!(!line((-3.8, -0.4), (-3.8, -1.6), FLOOR_Y));
    // Down the stair: two treads apart, walked at the upper one's feet.
    assert!(!line((5.7, -14.75), (5.7, -13.75), 0.1 - 0.5 * 6.0));
}
