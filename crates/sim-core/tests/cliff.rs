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

/// Whether the solid surface at (`x`, `z`) is a face by the sim's law
/// (`movement::face_fall`).
fn face<H: sim_core::occupy::Harvested>(
    sc: &mut Scratch<H>,
    seed: u64,
    haven: &Haven,
    x: f32,
    z: f32,
) -> bool {
    movement::face_fall(&mut sc.occupants(), seed, haven, x, z).is_some()
}

/// Walked up a face on a diagonal from its foot, a body never stands on bare
/// ground too steep to climb: it cannot step up onto it, and slides off. Until
/// 2026-10-17 the ratio held only the rise along a step, so a body walked up
/// any face on a slant and stood inside the rock the client draws proud of
/// it (`render/cliff_skin.rs`).
#[test]
fn a_body_cannot_climb_a_face_on_a_diagonal_or_stand_on_one() {
    let cols = ColIndex::new();
    let seed = 20_260_731u64;
    let haven = terrain::haven(seed);
    let mut sc = Scratch::live(seed);
    let (mut walks, mut pressed) = (0, 0);
    let mut x = 300.0f32;
    while x < terrain::ISLAND_SIZE - 300.0 && walks < 24 {
        let mut z = 300.0f32;
        while z < terrain::ISLAND_SIZE - 300.0 && walks < 24 {
            z += 13.0;
            if terrain::ground_slope(seed, &haven, x, z) < terrain::CLIFF_SLOPE_RATIO * 1.6 {
                continue;
            }
            // Uphill, then down the fall line to the face's foot.
            let gx = terrain::ground(seed, &haven, x + 1.0, z)
                - terrain::ground(seed, &haven, x - 1.0, z);
            let gz = terrain::ground(seed, &haven, x, z + 1.0)
                - terrain::ground(seed, &haven, x, z - 1.0);
            let l = (gx * gx + gz * gz).sqrt();
            let (ux, uz) = (gx / l, gz / l);
            let (mut fx, mut fz) = (x, z);
            let mut k = 0;
            while face(&mut sc, seed, &haven, fx, fz) && k < 40 {
                fx -= ux * 0.5;
                fz -= uz * 0.5;
                k += 1;
            }
            if k == 40 {
                continue;
            }
            let mut b = Body::at(seed, &haven, fx, fz);
            let y0 = b.qy as f32 * POS_Y_Q;
            // 35° off the contour, uphill.
            let (cs, sn) = (0.82, 0.57);
            let (wx, wz) = (-uz * cs + ux * sn, ux * cs + uz * sn);
            let f = InputFrame {
                seq: 1,
                yaw: bearing(wx, wz),
                move_z: 127,
                ..InputFrame::default()
            };
            for _ in 0..300 {
                movement::step(seed, &haven, &cols, &mut sc.occupants(), &mut b, &f);
                let (bx, bz) = (b.qx as f32 * POS_XZ_Q, b.qz as f32 * POS_XZ_Q);
                let feet = b.qy as f32 * POS_Y_Q;
                let bare = sc.occupants().body_ground(seed, bx, bz, feet) < feet - 0.01;
                let at_face = bare && face(&mut sc, seed, &haven, bx, bz);
                pressed += at_face as usize;
                assert!(
                    !(b.grounded && at_face),
                    "a body stands on a face at ({bx:.2}, {bz:.2}), feet {feet:.2}, \
                     having walked from the foot at ({fx:.1}, {fz:.1}) at {y0:.2}"
                );
            }
            walks += 1;
            x += 61.0;
        }
        x += 61.0;
    }
    assert!(walks >= 8, "only {walks} faces to walk at");
    assert!(
        pressed > 0,
        "no walk ever reached a face: the test walks nowhere"
    );
}
