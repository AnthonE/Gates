//! Rock formations (`boulder.rs`): big enough to be landmarks, clear of the
//! roads and sites, and solid — a flank stops a body and a crown holds one.

use sim_core::boulder::{self, Dome};
use sim_core::occupy::Scratch;
use sim_core::terrain;

const SEEDS: [u64; 3] = [20260731, 0x0047_4154_4553, 0xDEAD_BEEF];

fn domes(seed: u64, haven: &terrain::Haven) -> Vec<Dome> {
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
        let d = domes(seed, &haven);
        let big = d.iter().filter(|d| d.r >= 8.0).count();
        assert!(d.len() >= 60, "seed {seed:#x}: only {} rock domes", d.len());
        assert!(
            big >= 8,
            "seed {seed:#x}: only {big} domes 16 m or more across"
        );
        for dome in &d {
            assert!(
                dome.h > 1.0 && dome.r >= 1.0,
                "seed {seed:#x}: a dome too small to be one: {dome:?}"
            );
        }
    }
}

#[test]
fn no_rock_stands_on_a_road_or_a_site() {
    for seed in SEEDS {
        let haven = terrain::haven(seed);
        for d in domes(seed, &haven) {
            let road = d.r + terrain::ROAD_SHOULDER_HALF_W;
            assert!(
                haven.ring.dist2(d.x, d.z) >= road * road,
                "seed {seed:#x}: a rock on the ring road at ({}, {})",
                d.x,
                d.z
            );
            assert!(
                !terrain::in_haven(&haven, d.x, d.z) && !terrain::in_waystation(&haven, d.x, d.z),
                "seed {seed:#x}: a rock on a site at ({}, {})",
                d.x,
                d.z
            );
        }
    }
}

#[test]
fn a_flank_stops_a_body_and_a_crown_holds_one() {
    let seed = SEEDS[0];
    let mut scratch = Scratch::live(seed);
    let haven = terrain::haven(seed);
    let d = domes(seed, &haven)
        .into_iter()
        .filter(|d| d.r >= 6.0)
        .max_by(|a, b| a.h.total_cmp(&b.h))
        .expect("a big rock on the shipped seed");
    let mut occ = scratch.occupants();
    // At the foot of its flank, a body standing on the ground is stopped.
    let (fx, fz) = (d.x + d.r * 0.9, d.z);
    let feet = terrain::ground(seed, &haven, fx, fz);
    assert!(
        occ.blocks(seed, fx, fz, feet),
        "a body at the foot of a {:.1} m rock walked into it",
        d.h
    );
    // Well clear of it, nothing stops the same body.
    let (ox, oz) = (d.x + d.r + 3.0, d.z);
    let feet = terrain::ground(seed, &haven, ox, oz);
    assert!(
        !occ.blocks(seed, ox, oz, feet),
        "a body 3 m off a rock is stopped by it"
    );
    // On the crown, the rock is the ground.
    let top = d.y + d.h;
    let g = occ.ground(seed, d.x, d.z, top);
    assert!(
        (g - top).abs() < 0.6,
        "the crown of a rock is not ground: {g} against a top at {top}"
    );
}
