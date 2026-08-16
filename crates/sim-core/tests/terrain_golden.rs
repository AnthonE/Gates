//! `test_terrain_golden` (TERRAIN.md §0/§7): fixed seed → pinned hash of
//! 64×64 sampled heights + 256 scatter results. The wasm half of the same
//! assertion runs in ci/gates.sh via ci/parity.mjs against the same pin.
//! Plus shape sanity: the generator must produce an island, not a puddle.

use sim_core::probe::{
    probe_road_point, probe_terrain, probe_window_origin, PROBE_ROAD_BEARINGS, PROBE_ROAD_RADII,
    PROBE_WINDOW_CELLS,
};
use sim_core::terrain::{self, Occupant, ScatterTable, CELLS_PER_SIDE};

const GOLDEN_SEED: u64 = 0x0047_4154_4553; // "GATES"

/// The seeds `examples/probe.rs` and `ci/parity.mjs` drive, kept in lockstep
/// with both by hand exactly as they are with each other. The coverage
/// assertion below is a claim about the parity surface, so it is worth
/// nothing unless it is made over the seeds that surface actually runs.
const PROBE_SEEDS: [u64; 3] = [GOLDEN_SEED, 0x1, 0xDEAD_BEEF];

/// Pinned fingerprint for GOLDEN_SEED. Regenerates only with an intentional
/// worldgen change, in the same commit (CLAUDE.md walls 5/6 discipline).
///
/// Regenerated here from `0xA217_658A_C65D_F3CB` because **the site carve was
/// armed** (operator, 2026-08-16): `SITE_STAMP_STRENGTH` 0.0 → 1.0, so the pad
/// and both waystations now MAKE their flat ground instead of standing on
/// whatever the argmax found. That is the largest-reach worldgen change since
/// this pin existed and it moves this digest by design.
///
/// **The reach is bounded and the bound is asserted elsewhere**, which is worth
/// stating because "the carve moved the golden" could hide anything. It moves
/// exactly three discs: `probe_sites` hashes windows over the pad and both
/// waystations, and all three stand on the road ring, so their windows sit
/// inside the carve. `tests/carve.rs` §C holds that no ground outside a site's
/// `blend_m` changes at all, bit for bit, islandwide — so what this digest sees
/// is the whole of what moved.
///
/// A previous regeneration, kept because it explains the value it replaced: the
/// road shoulder's barrel rate stopped being one number and became two — dense
/// on the coast's sheltered arcs, sparse on the open shore (`terrain::in_bay`,
/// TERRAIN.md §1 stage 7's "junk piles at bay mouths" line). And before that,
/// the scatter pass stopped choosing a biome row and started blending four
/// (`terrain::scatter_row`), a delta confined to the ~11% of land cells no
/// single splat channel owns outright.
const GOLDEN_TERRAIN_HASH: u64 = 0x97E7_4336_299A_D2FD;

#[test]
fn test_terrain_golden() {
    assert_eq!(
        probe_terrain(GOLDEN_SEED),
        GOLDEN_TERRAIN_HASH,
        "worldgen output drifted from the pinned golden; if intentional, \
         regenerate the golden in this same commit"
    );
    // A different seed produces a different island.
    assert_ne!(probe_terrain(GOLDEN_SEED ^ 1), GOLDEN_TERRAIN_HASH);
}

/// Count the authored occupants inside the probe's scatter window at a
/// position — over exactly the cells `probe_window_origin` hands the digest,
/// never a second copy of that arithmetic. A coverage test that recomputes
/// the window is a test of itself.
fn window_occupants(seed: u64, haven: &terrain::Haven, x: f32, z: f32) -> (i32, i32, i32) {
    let table = ScatterTable::alpha_default();
    let (cx0, cz0) = probe_window_origin(x, z);
    let (mut shelters, mut crates, mut caches) = (0i32, 0i32, 0i32);
    for cz in cz0..cz0 + PROBE_WINDOW_CELLS {
        for cx in cx0..cx0 + PROBE_WINDOW_CELLS {
            match terrain::scatter(seed, &table, haven, cx, cz).occupant {
                Occupant::HavenShelter => shelters += 1,
                Occupant::CrateSlot => crates += 1,
                Occupant::CacheSlot => caches += 1,
                _ => {}
            }
        }
    }
    (shelters, crates, caches)
}

/// The golden's COVERAGE, asserted as a count rather than trusted.
///
/// `GOLDEN_TERRAIN_HASH` moving is what catches worldgen drift, but a hash
/// cannot say WHAT it covers, and for the island's whole authored half the
/// answer used to be nothing. `probe_terrain` resolved `haven(seed)` and then
/// hashed only cells 120..136 — a ±64 m window on the island center — while
/// every authored site sits on the 600..1000 m road ring. Measured on these
/// three seeds before this gate existed, of that block's 256 cells the number
/// inside `in_haven` or `in_waystation` was **zero on all three**: the pad,
/// both waystations, all five pad crates, the shelter and all four waystation
/// crates could each have taken different coordinates on wasm than on native
/// with `test_terrain_golden` and `test_parity_wasm` both green, while
/// `client-core` reads the wasm answer and the server reads the native one.
///
/// So coverage is its own assertion. Re-centre a site window on empty sea and
/// the golden regenerates perfectly clean over nothing at all; this fails
/// instead, and names which site went dark.
#[test]
fn test_golden_covers_authored_sites() {
    for seed in PROBE_SEEDS {
        let h = terrain::haven(seed);

        // Sites are `WAYSTATION_MIN_SEP_M` (600 m) apart and a window is
        // 128 m across, so no window can be counting a neighbour's crates.
        let (shelters, crates, caches) = window_occupants(seed, &h, h.x, h.z);
        assert_eq!(
            shelters, 1,
            "seed {seed:#x}: the pad's greybox is not inside the golden's window at the pad"
        );
        assert_eq!(
            crates,
            terrain::HAVEN_CRATES,
            "seed {seed:#x}: the golden's window at the pad holds {crates} of \
             {} containers — the digest is not covering the pad it names",
            terrain::HAVEN_CRATES
        );
        // The KIND is on the parity surface too, not only the count. A
        // container's kind is what selects its loot table, so a pad that
        // resolved `CacheSlot` on one target and `CrateSlot` on the other
        // would be two islands paying two different prices with the digest
        // unable to say so — the count alone cannot see it.
        assert_eq!(
            caches, 0,
            "seed {seed:#x}: {caches} lesser-tier container(s) inside the pad's \
             window — the destination is drawing the tier below it"
        );

        for (i, ws) in h.minor.iter().enumerate() {
            assert!(
                ws.live,
                "seed {seed:#x}: waystation {i} is not live, so the parity \
                 surface is covering `Waystation::NONE` at the island corner \
                 rather than a site (tests/waystation.rs owns the tier itself)"
            );
            let (shelters, crates, caches) = window_occupants(seed, &h, ws.x, ws.z);
            assert_eq!(
                caches,
                terrain::WAYSTATION_CRATES,
                "seed {seed:#x}: the golden's window at waystation {i} holds \
                 {caches} of {} containers",
                terrain::WAYSTATION_CRATES
            );
            // The pad's own container kind is the pad's alone, for the same
            // reason its greybox is: a `CrateSlot` here would be the lesser
            // tier drawing `loot.crate`, which is precisely what it did until
            // the two kinds were split.
            assert_eq!(
                crates, 0,
                "seed {seed:#x}: waystation {i} stands {crates} of the pad's \
                 own container kind — the lesser tier is paying the \
                 destination's loot table"
            );
            // The greybox is the destination's alone — it is what makes the
            // pad read as the better place on sight, and the gradient the
            // lesser tier exists to create depends on it staying there.
            assert_eq!(
                shelters, 0,
                "seed {seed:#x}: waystation {i} grew a shelter — the tier \
                 gradient says the greybox belongs to the pad alone"
            );
        }

        // The road half of `probe_sites`, asserted for the same reason: a
        // sweep that never crosses the road hashes a constant, and a
        // constant pins nothing while looking exactly like coverage. At the
        // first draft's 8 radii this was 9–15 of 64 bearings.
        let mut bearings_hit = 0u16;
        let mut b = 0u16;
        while b < 256 {
            let mut hit = false;
            for r in 0..PROBE_ROAD_RADII {
                let (px, pz) = probe_road_point(b, r);
                if terrain::road_band(seed, px, pz) != terrain::RoadBand::Off {
                    hit = true;
                }
            }
            if hit {
                bearings_hit += 1;
            }
            b += 256 / PROBE_ROAD_BEARINGS;
        }
        assert_eq!(
            bearings_hit, PROBE_ROAD_BEARINGS,
            "seed {seed:#x}: the road sweep found the coast road on only \
             {bearings_hit} of {PROBE_ROAD_BEARINGS} bearings — the radial \
             step is too coarse to cross it, so those bearings hash a constant"
        );
    }
}

#[test]
fn test_terrain_shape_sanity() {
    let mut min_h = f32::INFINITY;
    let mut max_h = f32::NEG_INFINITY;
    for gz in 0..64i32 {
        for gx in 0..64i32 {
            let h = terrain::height(
                GOLDEN_SEED,
                gx as f32 * 32.0 + 16.0,
                gz as f32 * 32.0 + 16.0,
            );
            min_h = min_h.min(h);
            max_h = max_h.max(h);
        }
    }
    assert!(min_h < -5.0, "no sea floor: min sampled height {min_h}");
    assert!(max_h > 40.0, "no relief: max sampled height {max_h}");

    // TERRAIN.md §6: ~8–12k live slots per seed. The band is the doc's; a
    // seed outside it means the scatter weights drifted, not the seed.
    let table = ScatterTable::alpha_default();
    let haven = terrain::haven(GOLDEN_SEED);
    let mut live = 0u32;
    let mut trees = 0u32;
    let mut barrels = 0u32;
    let mut ore = 0u32;
    for cz in 0..CELLS_PER_SIDE {
        for cx in 0..CELLS_PER_SIDE {
            let s = terrain::scatter(GOLDEN_SEED, &table, &haven, cx, cz);
            match s.occupant {
                Occupant::None => {}
                Occupant::Tree => {
                    live += 1;
                    trees += 1;
                }
                Occupant::BarrelSlot => {
                    live += 1;
                    barrels += 1;
                }
                Occupant::StoneNode | Occupant::MetalNode | Occupant::SulfurNode => {
                    live += 1;
                    ore += 1;
                }
                _ => live += 1,
            }
        }
    }
    assert!(
        (8_000..=12_000).contains(&live),
        "live slots {live} outside TERRAIN.md §6's 8–12k band (trees {trees}, ore {ore}, barrels {barrels})"
    );
    assert!(trees > 1_000, "island needs wood: {trees} trees");
    assert!(ore > 300, "island needs ore: {ore} nodes");
    assert!(barrels > 50, "the loot route needs barrels: {barrels}");
}
