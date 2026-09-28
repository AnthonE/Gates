//! Rock formations (`boulder.rs`): big enough to be landmarks, clear of the
//! roads and sites, and solid — a side stops a body and a top holds one.

use sim_core::boulder::{self, Block, ON_GROUND};
use sim_core::occupy::Scratch;
use sim_core::terrain;

const SEEDS: [u64; 3] = [20260731, 0x0047_4154_4553, 0xDEAD_BEEF];

fn blocks(seed: u64, haven: &terrain::Haven) -> Vec<Block> {
    let n = boulder::cells_per_side();
    let mut out = Vec::new();
    for bz in 0..n {
        for bx in 0..n {
            let f = boulder::formation(seed, haven, bx, bz);
            out.extend(f.iter().copied());
        }
    }
    out
}

#[test]
fn the_island_has_rocks_and_some_of_them_are_huge() {
    for seed in SEEDS {
        let haven = terrain::haven(seed);
        let b = blocks(seed, &haven);
        let big = b.iter().filter(|b| b.hx.max(b.hz) >= 6.0).count();
        let tall = b
            .iter()
            .filter(|b| b.y1 - terrain::height(seed, b.x, b.z) >= 8.0)
            .count();
        let stacked = b.iter().filter(|b| b.on != ON_GROUND).count();
        assert!(
            b.len() >= 150,
            "seed {seed:#x}: only {} rock blocks",
            b.len()
        );
        assert!(
            big >= 8,
            "seed {seed:#x}: only {big} blocks 12 m or more long"
        );
        assert!(
            tall >= 8,
            "seed {seed:#x}: only {tall} blocks 8 m or more tall"
        );
        assert!(
            stacked >= 20,
            "seed {seed:#x}: only {stacked} stacked blocks"
        );
        for b in &b {
            assert!(
                b.hx >= boulder::BLOCK_HALF_FLOOR
                    && b.hz >= boulder::BLOCK_HALF_FLOOR
                    && b.y1 > b.y0 + 0.5,
                "seed {seed:#x}: a block too small to be one: {b:?}"
            );
            assert!(
                b.tx.max(-b.tx) <= boulder::TILT_MAX && b.tz.max(-b.tz) <= boulder::TILT_MAX,
                "seed {seed:#x}: a block whose top is a wall: {b:?}"
            );
        }
    }
}

#[test]
fn no_rock_stands_on_a_road_or_a_site() {
    for seed in SEEDS {
        let haven = terrain::haven(seed);
        for b in blocks(seed, &haven) {
            let road = b.radius() + terrain::ROAD_SHOULDER_HALF_W;
            assert!(
                haven.ring.dist2(b.x, b.z) >= road * road,
                "seed {seed:#x}: a rock on the ring road at ({}, {})",
                b.x,
                b.z
            );
            assert!(
                !terrain::in_haven(&haven, b.x, b.z) && !terrain::in_waystation(&haven, b.x, b.z),
                "seed {seed:#x}: a rock on a site at ({}, {})",
                b.x,
                b.z
            );
        }
    }
}

#[test]
fn a_side_stops_a_body_and_a_top_holds_one() {
    let seed = SEEDS[0];
    let mut scratch = Scratch::live(seed);
    let haven = terrain::haven(seed);
    let all = blocks(seed, &haven);
    // A broad seated block, and the side of it that stands tallest over the
    // ground outside it.
    let (b, fx, fz, feet) = all
        .iter()
        .filter(|b| b.on == ON_GROUND && b.hx.min(b.hz) >= 3.0)
        .flat_map(|b| {
            [(1.0f32, 0.0f32), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)]
                .into_iter()
                .map(move |(sx, sz)| {
                    let (lx, lz) = (sx * (b.hx + 0.1), sz * (b.hz + 0.1));
                    let (x, z) = b.to_world(lx, lz);
                    (*b, x, z, terrain::ground(seed, &haven, x, z))
                })
        })
        .max_by(|a, b| {
            let rise = |p: &(Block, f32, f32, f32)| {
                let (lx, lz) = p.0.to_local(p.1, p.2);
                p.0.top_local(lx, lz) - p.3
            };
            rise(a).total_cmp(&rise(b))
        })
        .expect("a broad rock on the shipped seed");
    let mut occ = scratch.occupants();
    assert!(
        occ.blocks(seed, fx, fz, feet),
        "a body at the foot of a {:.1} m block walked into it",
        b.y1 - feet
    );
    // Well clear of it, nothing stops the same body.
    let (ox, oz) = [(1.0f32, 0.0f32), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)]
        .into_iter()
        .map(|(sx, sz)| b.to_world(sx * (b.hx + 4.0), sz * (b.hz + 4.0)))
        .find(|&(x, z)| !boulder::covers(seed, &haven, x, z, 1.0))
        .expect("somewhere around a rock is clear of rock");
    let feet = terrain::ground(seed, &haven, ox, oz);
    assert!(
        !occ.blocks(seed, ox, oz, feet),
        "a body 4 m off a rock is stopped by it"
    );
    // On top, the rock is the ground.
    let top = b.y1;
    let g = occ.ground(seed, b.x, b.z, top);
    assert!(
        g > top - 0.6 && g < top + 0.6,
        "the top of a rock is not ground: {g} against a top at {top}"
    );
}
