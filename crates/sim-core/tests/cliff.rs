//! Cliff crags are solid (`sim_core::cliff`). The operator walked through a
//! tooth standing at a lip on 2026-10-07: only the client knew it was there,
//! and a body may walk down a face and along it.

use sim_core::cliff::{self, Crag, SPOTS_PER_SIDE};
use sim_core::collide::{ColIndex, CAPSULE_RADIUS_M};
use sim_core::input::InputFrame;
use sim_core::movement::{self, Body, POS_XZ_Q, POS_Y_Q};
use sim_core::occupy::Scratch;
use sim_core::terrain::{self, Haven, Lattice};
use sim_core::yaw_dir;

/// Up to `want` crags of each kind on `seed`, from the island's middle rows.
fn crags(seed: u64, haven: &Haven, want: usize) -> (Vec<Crag>, Vec<Crag>) {
    let mut lat = Lattice::new();
    let (mut teeth, mut ledges) = (Vec::new(), Vec::new());
    let mut gj = SPOTS_PER_SIDE / 4;
    while gj < SPOTS_PER_SIDE * 3 / 4 && (teeth.len() < want || ledges.len() < want) {
        for gi in 0..SPOTS_PER_SIDE {
            let c = cliff::crag(&mut lat, seed, haven, gi, gj);
            if !c.is_some() {
                continue;
            }
            let list = if c.tooth { &mut teeth } else { &mut ledges };
            if list.len() < want {
                list.push(c);
            }
        }
        gj += 1;
    }
    (teeth, ledges)
}

/// The table bearing nearest the direction (`ux`, `uz`).
fn bearing(ux: f32, uz: f32) -> u16 {
    (0..256u16)
        .map(|k| k << 8)
        .max_by(|a, b| {
            let d = |y: u16| {
                let (fx, fz) = yaw_dir(y);
                fx * ux + fz * uz
            };
            d(*a).total_cmp(&d(*b))
        })
        .unwrap()
}

/// Walked down the fall line at a tooth from the walkable top above it, or
/// at a ledge from the face above it, a body stops against the crag or
/// stands on it, and its centre is never inside the rock below its top.
#[test]
fn a_body_walking_down_a_face_at_a_crag_never_walks_into_it() {
    let cols = ColIndex::new();
    for seed in [1u64, 7] {
        let haven = terrain::haven(seed);
        let mut sc = Scratch::live(seed);
        let (teeth, ledges) = crags(seed, &haven, 25);
        assert!(
            teeth.len() >= 10 && ledges.len() >= 10,
            "seed {seed}: only {} teeth and {} ledges to walk at",
            teeth.len(),
            ledges.len()
        );
        let mut touched = 0;
        for c in teeth.iter().chain(ledges.iter()) {
            let (dx, dz) = c.dir;
            let (sx, sz) = (c.x - dx * (c.hz + 4.0), c.z - dz * (c.hz + 4.0));
            let mut b = Body::at(seed, &haven, sx, sz);
            if sc.occupants().blocks(seed, sx, sz, b.qy as f32 * POS_Y_Q) {
                continue;
            }
            let f = InputFrame {
                seq: 1,
                yaw: bearing(dx, dz),
                move_z: 127,
                ..InputFrame::default()
            };
            let mut near = false;
            for _ in 0..150 {
                movement::step(seed, &haven, &cols, &mut sc.occupants(), &mut b, &f);
                let (x, z) = (b.qx as f32 * POS_XZ_Q, b.qz as f32 * POS_XZ_Q);
                let feet = b.qy as f32 * POS_Y_Q;
                near |= c.top(x, z, CAPSULE_RADIUS_M).is_some();
                if let Some(top) = c.top(x, z, 0.0) {
                    assert!(
                        feet >= top - 0.05,
                        "seed {seed}: a body walked into a {} at ({:.1}, {:.1}): feet \
                         {feet:.2} at ({x:.2}, {z:.2}) under its top {top:.2}",
                        if c.tooth { "tooth" } else { "ledge" },
                        c.x,
                        c.z
                    );
                }
            }
            touched += near as usize;
        }
        assert!(
            touched >= 10,
            "seed {seed}: only {touched} walks reached a crag"
        );
    }
}
