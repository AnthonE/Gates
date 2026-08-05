//! Ground clutter: the sub-metre population that answers `ART.md` rule 4.
//!
//! The rule is a number — "any visible ground patch larger than ~3 m² inside
//! 15 m carries scatter" — so this suite is arithmetic, not a screenshot. The
//! wall it holds is `test_no_bare_patch_inside_fifteen_metres`, which MEASURES
//! the largest empty disc rather than trusting the grid argument that predicts
//! it. Everything else here defends a premise that measurement rests on.

use sim_core::terrain::{
    self, Clutter, ClutterElem, CLUTTER_CELLS_PER_SIDE, CLUTTER_CELLS_PER_TILE, CLUTTER_CELL_M,
    CLUTTER_NONE, CLUTTER_PER_TILE, CLUTTER_TILE_M, LAND_MIN_H,
};

const SEEDS: [u64; 3] = [0x4741_5445_53, 1, 0xDEAD_BEEF];

/// `ART.md` rule 4, verbatim: "~3 m²".
const MAX_BARE_M2: f32 = 3.0;
/// The near field the rule is about.
const NEAR_R_M: f32 = 15.0;

/// Every element within `r` of (ox, oz), brute-forced off the cell grid.
fn elements_near(seed: u64, ox: f32, oz: f32, r: f32) -> Vec<ClutterElem> {
    let c0x = ((ox - r) / CLUTTER_CELL_M) as i32 - 1;
    let c1x = ((ox + r) / CLUTTER_CELL_M) as i32 + 1;
    let c0z = ((oz - r) / CLUTTER_CELL_M) as i32 - 1;
    let c1z = ((oz + r) / CLUTTER_CELL_M) as i32 + 1;
    let mut out = Vec::new();
    for cz in c0z..=c1z {
        for cx in c0x..=c1x {
            let e = terrain::clutter_cell(seed, cx, cz);
            if e.kind != Clutter::None {
                out.push(e);
            }
        }
    }
    out
}

/// A land point near the island centre to stand on, for a seed.
fn a_land_origin(seed: u64, bearing: usize) -> (f32, f32) {
    let c = 1024.0f32;
    // Walk outward on one of eight rays until the ground is comfortably land
    // and not a cliff — the same conditions a spawn would want.
    let (dx, dz) = match bearing % 4 {
        0 => (1.0, 0.0),
        1 => (0.0, 1.0),
        2 => (-1.0, 0.0),
        _ => (0.0, -1.0),
    };
    let mut r = 0.0f32;
    while r < 700.0 {
        let (x, z) = (c + dx * r, c + dz * r);
        if terrain::height(seed, x, z) > LAND_MIN_H + 3.0
            && terrain::slope(seed, x, z) < 0.6
            && terrain::height(seed, x + 20.0, z + 20.0) > LAND_MIN_H
            && terrain::height(seed, x - 20.0, z - 20.0) > LAND_MIN_H
        {
            return (x, z);
        }
        r += 8.0;
    }
    (c, c)
}

/// THE WALL. Stand somewhere on land, look at the ground inside 15 m, and
/// measure the largest disc that contains no clutter at all. `ART.md` rule 4
/// caps that disc's area at ~3 m².
///
/// Query points on water are excluded and nothing else is: a shoreline inside
/// the ring is genuinely bare and the rule is about ground. That exclusion is
/// the one place this gate could be talked into passing something it should
/// not, so it is a height test against the same `LAND_MIN_H` the population
/// itself uses — not a distance, not a mask, and not tuneable.
#[test]
fn test_no_bare_patch_inside_fifteen_metres() {
    // Radius of the largest disc allowed to be empty, from its area.
    let max_r = (MAX_BARE_M2 / core::f32::consts::PI).sqrt();
    let mut worst = 0.0f32;
    let mut worst_where = (0u64, 0.0f32, 0.0f32);
    let mut checked = 0usize;

    for &seed in SEEDS.iter() {
        for bearing in 0..4 {
            let (ox, oz) = a_land_origin(seed, bearing);
            // Elements out to 15 m + a margin, so a query point at the rim
            // still sees everything that could be its nearest neighbour.
            let els = elements_near(seed, ox, oz, NEAR_R_M + 3.0);
            assert!(
                !els.is_empty(),
                "seed {seed:#x} bearing {bearing}: no clutter at all within \
                 {NEAR_R_M} m of a land origin ({ox:.1}, {oz:.1})"
            );

            let step = 0.5f32;
            let n = (NEAR_R_M / step) as i32;
            for j in -n..=n {
                for i in -n..=n {
                    let qx = ox + i as f32 * step;
                    let qz = oz + j as f32 * step;
                    let dx = qx - ox;
                    let dz = qz - oz;
                    if dx * dx + dz * dz > NEAR_R_M * NEAR_R_M {
                        continue;
                    }
                    if terrain::height(seed, qx, qz) < LAND_MIN_H {
                        continue; // water's edge is not a bare ground patch
                    }
                    let mut best = f32::MAX;
                    for e in els.iter() {
                        let ex = e.x - qx;
                        let ez = e.z - qz;
                        let d2 = ex * ex + ez * ez;
                        if d2 < best {
                            best = d2;
                        }
                    }
                    checked += 1;
                    let d = best.sqrt();
                    if d > worst {
                        worst = d;
                        worst_where = (seed, qx, qz);
                    }
                }
            }
        }
    }

    assert!(checked > 10_000, "only {checked} land query points sampled");
    let area = core::f32::consts::PI * worst * worst;
    // Report the margin. A gate that only speaks when it fails cannot tell
    // the next pass whether it is at 10% of the cap or at 99%.
    println!(
        "rule 4: worst bare disc {worst:.3} m = {area:.2} m² of {MAX_BARE_M2} m² \
         ({:.0}% of cap), {checked} land points over {} seeds",
        100.0 * area / MAX_BARE_M2,
        SEEDS.len()
    );
    assert!(
        worst <= max_r,
        "ART.md rule 4: a bare disc of radius {worst:.3} m ({area:.2} m²) at \
         seed {:#x} ({:.1}, {:.1}) — the cap is {max_r:.3} m ({MAX_BARE_M2} m²). \
         {checked} land points sampled.",
        worst_where.0,
        worst_where.1,
        worst_where.2
    );
}

/// The premise the coverage guarantee rests on: the splat weights normalize
/// to 255, so every land cell draws SOMETHING. If this rots — a fifth band, a
/// changed normalizer — coverage stops being structural and the wall above
/// becomes the only thing holding it up.
#[test]
fn test_splat_weights_are_normalized_on_land() {
    let mut land = 0usize;
    for &seed in SEEDS.iter() {
        for j in 0..40 {
            for i in 0..40 {
                let x = 64.0 + i as f32 * 48.0;
                let z = 64.0 + j as f32 * 48.0;
                let h = terrain::height(seed, x, z);
                if h < LAND_MIN_H {
                    continue;
                }
                land += 1;
                let w = terrain::splat(seed, x, z);
                let sum: u32 = w.iter().map(|v| *v as u32).sum();
                assert!(
                    (254..=256).contains(&sum),
                    "splat at ({x}, {z}) seed {seed:#x} sums to {sum}, not 255 ± 1: {w:?}"
                );
            }
        }
    }
    assert!(
        land > 500,
        "only {land} land samples — the scan missed the island"
    );
}

/// The claim that makes this a population and not noise: a tuft stands where
/// the ground is grass, a pebble where it is sand, a twig on litter, a shard
/// on rock. Asserted as the strong form — for each kind, the mean weight of
/// ITS OWN channel under its own feet beats the mean of every other channel
/// under those same feet.
#[test]
fn test_each_kind_stands_on_its_own_splat_channel() {
    let mut sums = [[0.0f64; 4]; 4]; // [kind][channel]
    let mut counts = [0usize; 4];
    for &seed in SEEDS.iter() {
        for j in 0..90 {
            for i in 0..90 {
                // A coprime stride so the scan is not aligned to any field.
                let cx = 400 + i * 31;
                let cz = 400 + j * 29;
                if cx >= CLUTTER_CELLS_PER_SIDE || cz >= CLUTTER_CELLS_PER_SIDE {
                    continue;
                }
                let e = terrain::clutter_cell(seed, cx, cz);
                if e.kind == Clutter::None {
                    continue;
                }
                let k = e.kind as usize - 1;
                let w = terrain::splat(seed, e.x, e.z);
                for c in 0..4 {
                    sums[k][c] += w[c] as f64;
                }
                counts[k] += 1;
            }
        }
    }
    for k in 0..4 {
        assert!(
            counts[k] > 100,
            "kind {k} drew only {} times in the whole scan — too few to judge",
            counts[k]
        );
        let own = sums[k][k] / counts[k] as f64;
        for c in 0..4 {
            if c == k {
                continue;
            }
            let other = sums[k][c] / counts[k] as f64;
            assert!(
                own > other,
                "kind {k} stands on mean channel-{k} weight {own:.1} but \
                 channel-{c} weight {other:.1} — the population does not \
                 follow the surface it is drawn from"
            );
        }
    }
}

/// Nothing grows in the sea, and the veto is the population's own height
/// test rather than a separate coastline.
#[test]
fn test_clutter_never_stands_in_water() {
    for &seed in SEEDS.iter() {
        let mut wet = 0usize;
        for j in 0..120 {
            for i in 0..120 {
                let cx = i * 26;
                let cz = j * 26;
                let e = terrain::clutter_cell(seed, cx, cz);
                if e.kind == Clutter::None {
                    continue;
                }
                assert!(
                    e.y >= LAND_MIN_H,
                    "clutter at ({:.1}, {:.1}) stands at y {:.2}, below LAND_MIN_H {LAND_MIN_H}",
                    e.x,
                    e.z,
                    e.y
                );
                wet += 1;
            }
        }
        assert!(
            wet > 1000,
            "seed {seed:#x}: only {wet} live cells in the island scan"
        );
    }
}

/// The carriageway keeps its grit and loses its grass — the one place the
/// population overrides the splat, so it gets its own assertion rather than
/// riding on the mix test above.
#[test]
fn test_carriageway_grows_grit_not_grass() {
    let mut on_road = 0usize;
    for &seed in SEEDS.iter() {
        for j in 0..CLUTTER_CELLS_PER_SIDE / 7 {
            for i in 0..CLUTTER_CELLS_PER_SIDE / 7 {
                let (cx, cz) = (i * 7, j * 7);
                let e = terrain::clutter_cell(seed, cx, cz);
                if e.kind == Clutter::None {
                    continue;
                }
                if terrain::road_band(seed, e.x, e.z) != terrain::RoadBand::Carriageway {
                    continue;
                }
                on_road += 1;
                assert_eq!(
                    e.kind,
                    Clutter::Pebble,
                    "the coast road's carriageway grew {:?} at ({:.1}, {:.1})",
                    e.kind,
                    e.x,
                    e.z
                );
            }
        }
    }
    // A gate that found no road is a gate that asserted nothing.
    assert!(
        on_road > 200,
        "only {on_road} carriageway cells sampled — the scan missed the road ring"
    );
}

/// Same seed, same field, from any call order; a different seed moves it.
#[test]
fn test_clutter_is_deterministic_and_seed_dependent() {
    let mut a = [CLUTTER_NONE; CLUTTER_PER_TILE];
    let mut b = [CLUTTER_NONE; CLUTTER_PER_TILE];
    let mut c = [CLUTTER_NONE; CLUTTER_PER_TILE];
    let (tx, tz) = (64, 64);
    let na = terrain::clutter_fill(SEEDS[0], tx, tz, &mut a);
    let nb = terrain::clutter_fill(SEEDS[0], tx, tz, &mut b);
    let nc = terrain::clutter_fill(SEEDS[1], tx, tz, &mut c);
    assert_eq!(na, nb);
    for i in 0..na {
        assert_eq!(a[i].kind, b[i].kind);
        assert_eq!(a[i].x.to_bits(), b[i].x.to_bits());
        assert_eq!(a[i].y.to_bits(), b[i].y.to_bits());
        assert_eq!(a[i].z.to_bits(), b[i].z.to_bits());
        assert_eq!(a[i].yaw, b[i].yaw);
        assert_eq!(a[i].scale.to_bits(), b[i].scale.to_bits());
    }
    let same = (0..na.min(nc))
        .filter(|&i| a[i].x.to_bits() == c[i].x.to_bits())
        .count();
    assert!(
        same * 4 < na,
        "a different seed reproduced {same} of {na} positions — the field is not seeded"
    );
}

/// A caller with a short buffer gets a thinner field, never an overrun. The
/// bridge's buffer is exactly `CLUTTER_PER_TILE`; anything else calling this
/// (a test, a future far-tile at a coarser stride) may not be.
#[test]
fn test_a_short_buffer_truncates_and_never_overruns() {
    let mut small = [CLUTTER_NONE; 7];
    let n = terrain::clutter_fill(SEEDS[0], 64, 64, &mut small);
    assert!(
        n <= 7,
        "clutter_fill wrote {n} elements into a 7-element buffer"
    );
    let mut full = [CLUTTER_NONE; CLUTTER_PER_TILE];
    let m = terrain::clutter_fill(SEEDS[0], 64, 64, &mut full);
    assert!(
        m >= n,
        "the short fill returned more than the full one ({n} > {m})"
    );
    // The prefix must agree: truncation drops the tail, it does not resample.
    for i in 0..n {
        assert_eq!(small[i].x.to_bits(), full[i].x.to_bits());
    }
}

/// The grid arithmetic the coverage guarantee is derived from. Structural:
/// it cannot pass by measuring nothing, and it fails the moment a constant
/// moves without its siblings.
#[test]
fn test_the_grid_divides_exactly() {
    assert_eq!(
        CLUTTER_CELLS_PER_TILE as f32 * CLUTTER_CELL_M,
        CLUTTER_TILE_M,
        "a tile is not a whole number of cells"
    );
    assert_eq!(
        CLUTTER_PER_TILE,
        (CLUTTER_CELLS_PER_TILE * CLUTTER_CELLS_PER_TILE) as usize
    );
    assert_eq!(
        CLUTTER_CELLS_PER_SIDE as f32 * CLUTTER_CELL_M,
        terrain::ISLAND_SIZE,
        "the cell grid does not cover the island exactly"
    );
    // The guarantee itself, as arithmetic: a disc of radius cell*sqrt(2)
    // contains a whole cell wherever it is centred, so that is the largest
    // disc that can be empty — and it must be inside ART.md rule 4.
    let guaranteed_r = CLUTTER_CELL_M * core::f32::consts::SQRT_2;
    let area = core::f32::consts::PI * guaranteed_r * guaranteed_r;
    assert!(
        area <= MAX_BARE_M2,
        "the grid's own bound is {area:.2} m², over rule 4's {MAX_BARE_M2} m²"
    );
}
