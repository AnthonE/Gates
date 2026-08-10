//! Ground clutter: the sub-metre population that answers `ART.md` rule 4.
//!
//! The rule is a number — "any visible ground patch larger than ~3 m² inside
//! 15 m carries scatter" — so this suite is arithmetic, not a screenshot. The
//! wall it holds is `test_no_bare_patch_inside_fifteen_metres`, which MEASURES
//! the largest empty disc rather than trusting the grid argument that predicts
//! it. Everything else here defends a premise that measurement rests on.

use sim_core::terrain::{
    self, Clutter, ClutterElem, Haven, Occupant, ScatterTable, CELL_SIZE, CLUTTER_BASE_PER_TILE,
    CLUTTER_CELLS_PER_SIDE, CLUTTER_CELLS_PER_TILE, CLUTTER_CELL_M, CLUTTER_NONE, CLUTTER_PER_TILE,
    CLUTTER_RICH_PER_TILE, CLUTTER_TILE_M, LAND_MIN_H, OCCUPANT_R_M, SKIRT_BAND_M, SKIRT_MAX,
    SKIRT_MIN, SKIRT_MIN_R_M, SKIRT_PER_TILE, SKIRT_SCAN_CELLS, SKIRT_TILE_CELLS,
};

const SEEDS: [u64; 3] = [0x0047_4154_4553, 1, 0xDEAD_BEEF];

/// `ART.md` rule 4, verbatim: "~3 m²".
const MAX_BARE_M2: f32 = 3.0;
/// The near field the rule is about.
const NEAR_R_M: f32 = 15.0;

/// Every element within `r` of (ox, oz), brute-forced off the cell grid —
/// BOTH strata, because a query that saw only the coverage draw would measure
/// a population the client does not draw.
fn elements_near(seed: u64, haven: &Haven, ox: f32, oz: f32, r: f32) -> Vec<ClutterElem> {
    let c0x = ((ox - r) / CLUTTER_CELL_M) as i32 - 1;
    let c1x = ((ox + r) / CLUTTER_CELL_M) as i32 + 1;
    let c0z = ((oz - r) / CLUTTER_CELL_M) as i32 - 1;
    let c1z = ((oz + r) / CLUTTER_CELL_M) as i32 + 1;
    let mut out = Vec::new();
    for cz in c0z..=c1z {
        for cx in c0x..=c1x {
            for e in [
                terrain::clutter_cell(seed, haven, cx, cz),
                terrain::clutter_rich_cell(seed, haven, cx, cz),
            ] {
                if e.kind != Clutter::None {
                    out.push(e);
                }
            }
        }
    }
    out
}

/// How many distinct stances the wall below takes per seed.
const ORIGINS_PER_SEED: usize = 24;

/// A land point to stand on, for a seed and a stance index.
///
/// THIS FUNCTION USED TO RETURN ONE POINT. It walked outward from the island
/// centre along four axis rays, stopping at the first sample that was land —
/// and the centre itself qualifies on every seed here, so all four bearings
/// returned (1024, 1024) and the wall's twelve origins were three. It sampled
/// 10 000+ query points and reported a worst disc, which is why nothing about
/// it looked thin; the coverage it actually had was one interior vantage per
/// seed. That is the shape `findings/pass-20260804-173640-01-visual.md` gap 2
/// names for a different gate in the same breath — "the gate measures at two
/// masked vantages that the artifact evades" — and the fix is the same one:
/// stand where the population is thinnest, not where it is convenient.
///
/// So the stance is now a spiral of `ORIGINS_PER_SEED` distinct bearings AND
/// radii, reaching from the island centre to the shore. Two things follow
/// that the old walk could not see: the ring crosses every biome the splat
/// has (the beach band is the outer radii, where `clutter_rich_cell` refuses
/// nearly every draw), and it crosses the coast road, whose carriageway is
/// the one place clutter is deliberately overridden.
fn a_land_origin(seed: u64, stance: usize) -> (f32, f32) {
    let c = 1024.0f32;
    // A golden-angle spiral: successive stances are far apart in bearing AND
    // in radius, so no two land in the same neighbourhood. Bearings come off
    // the yaw LUT rather than trig — sim-core forbids libm, and a test is in
    // the crate the wall applies to.
    // 157/256 of a turn is the golden angle to the nearest LUT index, so 24
    // stances spread without ever repeating a bearing.
    let idx = ((stance * 157) % 256) as u16;
    let (ux, uz) = sim_core::yaw_dir(idx << 8);
    // Reach outward across the whole island radius as the stance advances,
    // so the far stances are genuinely coastal and not just off-centre.
    let want = 40.0 + (stance as f32 / ORIGINS_PER_SEED as f32) * 860.0;

    // March in from the wanted radius until the ground is land and standable
    // — the same conditions a spawn would want. Marching INWARD rather than
    // outward is what keeps a coastal stance coastal: an outward walk that
    // starts at the centre always answers with the centre.
    let mut r = want;
    while r > 8.0 {
        let (x, z) = (c + ux * r, c + uz * r);
        if terrain::height(seed, x, z) > LAND_MIN_H + 1.0 && terrain::slope(seed, x, z) < 0.9 {
            return (x, z);
        }
        r -= 4.0;
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

    let mut stood: Vec<(f32, f32)> = Vec::new();
    for &seed in SEEDS.iter() {
        let haven = terrain::haven(seed);
        for bearing in 0..ORIGINS_PER_SEED {
            let (ox, oz) = a_land_origin(seed, bearing);
            stood.push((ox, oz));
            // Elements out to 15 m + a margin, so a query point at the rim
            // still sees everything that could be its nearest neighbour.
            let els = elements_near(seed, &haven, ox, oz, NEAR_R_M + 3.0);
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

    // THE STANCE IS PART OF THE WALL. `checked` counts query points, and a
    // gate can have a hundred thousand of those and still be standing in one
    // place — which is exactly what this test did before this pass: four
    // bearings that all resolved to the island centre, three real vantages
    // across three seeds, and a query count that hid it. Assert the spread
    // directly, so a future edit to `a_land_origin` that collapses it fails
    // HERE rather than quietly narrowing what the disc measurement covers.
    let mut apart = 0usize;
    for (i, a) in stood.iter().enumerate() {
        for b in stood.iter().skip(i + 1) {
            let (dx, dz) = (a.0 - b.0, a.1 - b.1);
            if dx * dx + dz * dz > 4.0 * NEAR_R_M * NEAR_R_M {
                apart += 1;
            }
        }
    }
    let pairs = stood.len() * (stood.len() - 1) / 2;
    assert!(
        stood.len() == SEEDS.len() * ORIGINS_PER_SEED && apart * 10 >= pairs * 9,
        "the wall stands in one place: {} origins, only {apart} of {pairs} pairs \
         more than {} m apart — the disc below is measured wherever these land, \
         so a collapsed stance is a gate that asserts less than it appears to",
        stood.len(),
        2.0 * NEAR_R_M
    );

    let area = core::f32::consts::PI * worst * worst;
    // The margin is NOT printed here. `println!` is a disallowed macro in this
    // crate — the L3 wall against I/O in the sim — and a test is not an
    // exemption worth carving. It was measured at 1.50 m², 50% of the cap, and
    // a measurement's home is `DECISIONS.md`/`TERRAIN.md`, which is where it
    // is. What a later pass needs from THIS file is the failure message below,
    // which names the disc, the seed and the point.
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

/// Elements in one clutter tile, both strata, and the tile's mean splat.
fn tile_census(seed: u64, haven: &Haven, tx: i32, tz: i32) -> (usize, usize, [u32; 4]) {
    let (cx0, cz0) = (tx * CLUTTER_CELLS_PER_TILE, tz * CLUTTER_CELLS_PER_TILE);
    let (mut base, mut rich, mut w) = (0usize, 0usize, [0u32; 4]);
    for j in 0..CLUTTER_CELLS_PER_TILE {
        for i in 0..CLUTTER_CELLS_PER_TILE {
            let (cx, cz) = (cx0 + i, cz0 + j);
            if terrain::clutter_cell(seed, haven, cx, cz).kind != Clutter::None {
                base += 1;
                let (x, z) = (cx as f32 * CLUTTER_CELL_M, cz as f32 * CLUTTER_CELL_M);
                let s = terrain::splat(seed, x, z);
                for (k, v) in s.iter().enumerate() {
                    w[k] += *v as u32;
                }
            }
            if terrain::clutter_rich_cell(seed, haven, cx, cz).kind != Clutter::None {
                rich += 1;
            }
        }
    }
    (base, rich, w)
}

/// DENSITY FOLLOWS THE BIOME — the half of `ART.md` rule 4 the coverage
/// stratum cannot answer, and the one the visual report asked for in its own
/// words ("populate the near ground… so density follows the biome").
///
/// The coverage stratum is uniform BY CONSTRUCTION: one element per land
/// cell, sand and meadow alike. That makes rule 4 provable and makes the
/// ground read as evenly dusted, which is the defect `reference/SPAWN.md` §9.3
/// names from the other side. So this asserts the opposite property about the
/// SECOND stratum: growing ground (grass + forest litter) must carry
/// materially more of it than bare ground (sand + rock) does.
///
/// It is a ratio, not a count, so it survives any future change to the
/// budget: `CLUTTER_RICH_PER_TILE` sets how much richness is affordable, and
/// this sets that the richness lands where the ground grows.
#[test]
fn test_richness_follows_the_biome() {
    let mut grow = (0usize, 0usize); // (rich elements, base elements)
    let mut bare = (0usize, 0usize);
    for &seed in SEEDS.iter() {
        let haven = terrain::haven(seed);
        for stance in 0..ORIGINS_PER_SEED {
            let (x, z) = a_land_origin(seed, stance);
            let (tx, tz) = ((x / CLUTTER_TILE_M) as i32, (z / CLUTTER_TILE_M) as i32);
            let (base, rich, w) = tile_census(seed, &haven, tx, tz);
            if base < 100 {
                continue; // mostly sea — not a ground sample
            }
            // Which identity owns this tile: the two growing channels against
            // the two bare ones, off the same `splat` the population reads.
            let bucket = if w[1] + w[2] > w[0] + w[3] {
                &mut grow
            } else {
                &mut bare
            };
            bucket.0 += rich;
            bucket.1 += base;
        }
    }
    assert!(
        grow.1 > 0 && bare.1 > 0,
        "the stance ring found only one ground type ({} growing cells, {} bare) — \
         the comparison this gate makes is vacuous",
        grow.1,
        bare.1
    );
    // Per base element, so tiles of different land area compare directly.
    let grow_rate = grow.0 as f32 / grow.1 as f32;
    let bare_rate = bare.0 as f32 / bare.1 as f32;
    assert!(
        grow_rate > bare_rate * 2.0,
        "richness does not follow the biome: growing ground carries {grow_rate:.3} extra \
         elements per cell and bare ground {bare_rate:.3} — under the 2× this stratum \
         exists to produce. ({} rich / {} cells growing, {} / {} bare.)",
        grow.0,
        grow.1,
        bare.0,
        bare.1
    );
}

/// AND IT CLUSTERS. `reference/SPAWN.md` §9.3, the diagnosis this pass did not
/// have to make: the reference's scatter clusters and ours is per-cell
/// independent, which is why ours reads as white noise. A second stratum drawn
/// off an independent coin per cell would be exactly that defect at a new
/// density — so this measures whether the acceptances are spatially
/// correlated, and it is the assertion that would fail if a later pass
/// simplified `clutter_richness_at` down to a constant rate.
///
/// The instrument is the index of dispersion. For independent draws the
/// variance of a per-block count equals its mean (Poisson); a field that
/// clusters puts the same total into fewer, fuller blocks and drives the
/// variance above it. Anything at or under 1.0 is dust.
/// Index of dispersion of the rich count over `block`×`block`-cell blocks,
/// with the block count, measured only on blocks that are entirely land.
fn rich_dispersion(block: i32) -> (f32, f32, usize) {
    let mut counts: Vec<f32> = Vec::new();
    for &seed in SEEDS.iter() {
        let haven = &terrain::haven(seed);
        for stance in 0..ORIGINS_PER_SEED {
            let (x, z) = a_land_origin(seed, stance);
            let (cx0, cz0) = (
                (x / CLUTTER_CELL_M) as i32 / block * block,
                (z / CLUTTER_CELL_M) as i32 / block * block,
            );
            for bj in 0..5 {
                for bi in 0..5 {
                    let mut n = 0f32;
                    let mut land = 0;
                    for j in 0..block {
                        for i in 0..block {
                            let cx = cx0 + (bi - 2) * block + i;
                            let cz = cz0 + (bj - 2) * block + j;
                            if terrain::clutter_cell(seed, haven, cx, cz).kind != Clutter::None {
                                land += 1;
                            }
                            if terrain::clutter_rich_cell(seed, haven, cx, cz).kind != Clutter::None
                            {
                                n += 1.0;
                            }
                        }
                    }
                    // Whole-land blocks only: a block half in the sea has a
                    // low count for a reason that is not clustering.
                    if land == block * block {
                        counts.push(n);
                    }
                }
            }
        }
    }
    let mean = counts.iter().sum::<f32>() / counts.len().max(1) as f32;
    let var =
        counts.iter().map(|c| (c - mean) * (c - mean)).sum::<f32>() / counts.len().max(1) as f32;
    (mean, var, counts.len())
}

#[test]
fn test_richness_clusters_rather_than_dusting() {
    // Two scales, because ONE scale cannot tell clustering from noise. For
    // independent draws the index of dispersion is 1.0 at EVERY block size;
    // for a field whose rate is spatially correlated it exceeds 1 and RISES
    // with the block, because enlarging the window adds correlated cells
    // rather than independent ones. So the load-bearing assertion is the
    // second one — the rise — and it is the thing a constant acceptance rate
    // could not fake at any tuning.
    //
    // The fine block is 3.2 m, near the cell scale; the coarse one is 12.8 m,
    // nearer the scale `clump` actually varies at. Measured 1.40 fine, 8.51
    // coarse when written — a sixfold rise across two doublings of the window.
    let (fine_m, fine_v, fine_n) = rich_dispersion(5);
    let (coarse_m, coarse_v, coarse_n) = rich_dispersion(20);
    assert!(
        fine_n > 200 && coarse_n > 20,
        "only {fine_n} fine and {coarse_n} coarse whole-land blocks — too few to \
         speak about variance"
    );
    assert!(
        fine_m > 0.5 && coarse_m > 0.5,
        "the richness stratum is near-empty (means {fine_m:.3} / {coarse_m:.3}) — \
         a gate on its shape cannot mean anything"
    );
    let (fine_d, coarse_d) = (fine_v / fine_m, coarse_v / coarse_m);
    assert!(
        fine_d > 1.2,
        "the richness stratum dusts rather than clusters: dispersion {fine_d:.2} over \
         {fine_n} blocks of 3.2 m, and 1.0 is an independent coin per cell — \
         SPAWN.md §9.3's exact defect"
    );
    assert!(
        coarse_d > fine_d * 1.3,
        "the richness stratum's dispersion does not RISE with scale — {fine_d:.2} at \
         3.2 m against {coarse_d:.2} at 12.8 m. A correlated field gains dispersion as \
         the window grows; an independent coin holds ~1.0 at every scale, so a flat \
         profile means the rate is not following anything spatial"
    );
}

/// THE BUDGET IS A BACKSTOP, NOT THE NORMAL PATH — and this is the gate that
/// holds it, because the failure it catches is one this pass shipped and then
/// measured its way out of.
///
/// `clutter_fill` scans row-major and stops adding richness at
/// `CLUTTER_RICH_PER_TILE`. The first draft accepted 71% of cells on wooded
/// ground against a budget of 96 in 625, so the budget was spent in the tile's
/// first few rows and every row after them carried none: a horizontal band
/// edge every 16 m, which is a worse artifact than the uniformity the stratum
/// exists to fix. `RICH_ACCEPT_MAX` is the fix — a RATE the ground can afford,
/// rather than a truncation of one it cannot.
///
/// So: fill real tiles, split each one's richness by which half of the tile it
/// landed in, and assert the two halves are comparable. A tile that spent its
/// budget early fails this even though every other test in this file passes,
/// which is exactly the point.
#[test]
fn test_richness_is_spread_across_its_tile() {
    let mut near = 0usize; // rich elements in the first half of the tile
    let mut far = 0usize; // ... and in the second
    let mut tiles = 0usize;
    let mut at_budget = 0usize;
    for &seed in SEEDS.iter() {
        let haven = &terrain::haven(seed);
        for stance in 0..ORIGINS_PER_SEED {
            let (x, z) = a_land_origin(seed, stance);
            let (tx, tz) = ((x / CLUTTER_TILE_M) as i32, (z / CLUTTER_TILE_M) as i32);
            let (base, _, _) = tile_census(seed, haven, tx, tz);
            if base < 600 {
                continue; // a tile with a shoreline in it is not a fair split
            }
            tiles += 1;
            let half = tz as f32 * CLUTTER_TILE_M + CLUTTER_TILE_M * 0.5;
            let mut rich = 0usize;
            let (cx0, cz0) = (tx * CLUTTER_CELLS_PER_TILE, tz * CLUTTER_CELLS_PER_TILE);
            for j in 0..CLUTTER_CELLS_PER_TILE {
                for i in 0..CLUTTER_CELLS_PER_TILE {
                    let e = terrain::clutter_rich_cell(seed, haven, cx0 + i, cz0 + j);
                    if e.kind == Clutter::None {
                        continue;
                    }
                    rich += 1;
                    if rich > CLUTTER_RICH_PER_TILE {
                        break; // what `clutter_fill` would have dropped
                    }
                    if e.z < half {
                        near += 1;
                    } else {
                        far += 1;
                    }
                }
            }
            if rich >= CLUTTER_RICH_PER_TILE {
                at_budget += 1;
            }
        }
    }
    assert!(
        tiles > 20 && near + far > 500,
        "only {tiles} whole-land tiles and {} elements — too little to judge a split",
        near + far
    );
    // A tile that spends its budget in row 0 puts everything in `near`.
    let (lo, hi) = (near.min(far) as f32, near.max(far) as f32);
    assert!(
        lo > hi * 0.6,
        "the richness stratum bands: {near} elements in the near half of a tile against \
         {far} in the far half over {tiles} tiles. Row-major truncation against \
         CLUTTER_RICH_PER_TILE={CLUTTER_RICH_PER_TILE} is what does this, and the rate — \
         not the cap — is where it is fixed."
    );
    // And the backstop must actually be a backstop.
    assert!(
        at_budget * 4 <= tiles,
        "{at_budget} of {tiles} tiles hit the {CLUTTER_RICH_PER_TILE}-element budget — \
         it is the normal path, not a backstop, so most rich tiles are being cut off \
         rather than populated at a rate they can afford"
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
        let haven = &terrain::haven(seed);
        for j in 0..90 {
            for i in 0..90 {
                // A coprime stride so the scan is not aligned to any field.
                let cx = 400 + i * 31;
                let cz = 400 + j * 29;
                if cx >= CLUTTER_CELLS_PER_SIDE || cz >= CLUTTER_CELLS_PER_SIDE {
                    continue;
                }
                let e = terrain::clutter_cell(seed, haven, cx, cz);
                if e.kind == Clutter::None {
                    continue;
                }
                let k = e.kind as usize - 1;
                let w = terrain::splat(seed, e.x, e.z);
                for (c, acc) in sums[k].iter_mut().enumerate() {
                    *acc += w[c] as f64;
                }
                counts[k] += 1;
            }
        }
    }
    for (k, row) in sums.iter().enumerate() {
        assert!(
            counts[k] > 100,
            "kind {k} drew only {} times in the whole scan — too few to judge",
            counts[k]
        );
        let own = row[k] / counts[k] as f64;
        for (c, total) in row.iter().enumerate() {
            if c == k {
                continue;
            }
            let other = total / counts[k] as f64;
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
        let haven = &terrain::haven(seed);
        let mut wet = 0usize;
        for j in 0..120 {
            for i in 0..120 {
                let cx = i * 26;
                let cz = j * 26;
                let e = terrain::clutter_cell(seed, haven, cx, cz);
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
        let haven = &terrain::haven(seed);
        for j in 0..CLUTTER_CELLS_PER_SIDE / 7 {
            for i in 0..CLUTTER_CELLS_PER_SIDE / 7 {
                let (cx, cz) = (i * 7, j * 7);
                let e = terrain::clutter_cell(seed, haven, cx, cz);
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
    let h0 = terrain::haven(SEEDS[0]);
    let h1 = terrain::haven(SEEDS[1]);
    let na = terrain::clutter_fill(SEEDS[0], &h0, tx, tz, &mut a);
    let nb = terrain::clutter_fill(SEEDS[0], &h0, tx, tz, &mut b);
    let nc = terrain::clutter_fill(SEEDS[1], &h1, tx, tz, &mut c);
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
    let h0 = terrain::haven(SEEDS[0]);
    let n = terrain::clutter_fill(SEEDS[0], &h0, 64, 64, &mut small);
    assert!(
        n <= 7,
        "clutter_fill wrote {n} elements into a 7-element buffer"
    );
    let mut full = [CLUTTER_NONE; CLUTTER_PER_TILE];
    let m = terrain::clutter_fill(SEEDS[0], &h0, 64, 64, &mut full);
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
    // The COVERAGE stratum is one element per cell — that is the premise the
    // guarantee below rests on, and it is asserted against the cell count
    // rather than against the grid cap, which now also carries the richness
    // stratum. Moving the total between the two strata fails here.
    assert_eq!(
        CLUTTER_BASE_PER_TILE,
        (CLUTTER_CELLS_PER_TILE * CLUTTER_CELLS_PER_TILE) as usize,
        "the coverage stratum is no longer one element per cell"
    );
    assert_eq!(
        CLUTTER_PER_TILE,
        CLUTTER_BASE_PER_TILE + CLUTTER_RICH_PER_TILE,
        "the grid cap is not the two strata"
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

// ── Prop-base skirts ───────────────────────────────────────────────────────
//
// The grid above is blind to props, which is what the visual judge named in
// `findings/pass-20260804-173640-01-visual.md` twice: gap 1 asked for the
// grid AND for clutter "crowded at every prop base", gap 3 named the symptom
// of the missing half — "a razor-clean intersection at its base", against
// `ART.md` rule 2, "nothing sits ON the ground, everything sits IN it".
//
// A skirt is not measurable as a bare disc, so this half of the suite gates
// the four properties the drawn result actually rests on: that a prop gets a
// ring, that the ring hugs its own published footprint, that the ring is
// spread rather than clumped on one side, and that no tile can overrun the
// buffer the client sized for it.

/// Does this tile hold a prop whose ring is big enough to judge spread on?
/// `test_a_skirt_is_spread_not_clumped` needs one, and a tile that merely has
/// SOME skirt need not have one — so the fixture below looks for it rather
/// than hoping, which is what it used to do.
fn tile_has_a_full_ring(seed: u64, table: &ScatterTable, haven: &Haven, tx: i32, tz: i32) -> bool {
    let c0x = tx * SKIRT_TILE_CELLS - 1;
    let c0z = tz * SKIRT_TILE_CELLS - 1;
    for dz in 0..4 {
        for dx in 0..4 {
            let slot = terrain::scatter(seed, table, haven, c0x + dx, c0z + dz);
            if terrain::skirt_count(slot.occupant) >= 8 {
                return true;
            }
        }
    }
    false
}

/// A tile with props in it, for a seed — the fixture the skirt tests stand on.
///
/// It searches the whole stance ring rather than one origin. It used to walk
/// east from `a_land_origin(seed, 0)` alone, which was the island centre for
/// every seed here; widening the wall's stance moved that point and this
/// fixture went with it, and the tile it landed on had props but no full ring
/// (`test_a_skirt_is_spread_not_clumped` said so). A fixture that depends on
/// one arbitrary point is the same defect as a wall that stands at one — so
/// this scans, prefers a tile that can answer every skirt test, and falls
/// back to any skirted tile rather than to a panic.
fn a_tile_with_props(seed: u64) -> (i32, i32, Haven, ScatterTable) {
    let table = ScatterTable::alpha_default();
    let haven = terrain::haven(seed);
    let mut fallback: Option<(i32, i32)> = None;
    for stance in 0..ORIGINS_PER_SEED {
        let (x, z) = a_land_origin(seed, stance);
        let (t0x, t0z) = ((x / CLUTTER_TILE_M) as i32, (z / CLUTTER_TILE_M) as i32);
        // A single tile is 16 m and the scatter grid is sparse enough that the
        // first can be empty, so walk a few from each stance.
        for d in 0..8 {
            let (tx, tz) = (t0x + d, t0z);
            let mut buf = [CLUTTER_NONE; SKIRT_PER_TILE];
            if terrain::skirt_fill(seed, &table, &haven, tx, tz, &mut buf) == 0 {
                continue;
            }
            if tile_has_a_full_ring(seed, &table, &haven, tx, tz) {
                return (tx, tz, haven, table);
            }
            fallback.get_or_insert((tx, tz));
        }
    }
    let (tx, tz) = fallback.unwrap_or_else(|| {
        panic!("seed {seed:#x}: no tile on the stance ring holds a skirted prop")
    });
    (tx, tz, haven, table)
}

/// A skirt exists at all, and it is made of the same four kinds the grid is.
/// The failure this catches is the one that reads as "shipped": a generator
/// that compiles, returns 0, and leaves every contact line exactly as razor
/// clean as the report found it.
#[test]
fn test_every_prop_gets_a_skirt() {
    for seed in SEEDS {
        let (tx, tz, haven, table) = a_tile_with_props(seed);
        let mut buf = [CLUTTER_NONE; SKIRT_PER_TILE];
        let n = terrain::skirt_fill(seed, &table, &haven, tx, tz, &mut buf);
        assert!(n > 0, "seed {seed:#x}: tile ({tx},{tz}) skirts nothing");
        for e in buf.iter().take(n) {
            assert!(
                e.kind != Clutter::None,
                "seed {seed:#x}: skirt_fill wrote an empty element inside its count"
            );
            assert!(
                e.y >= LAND_MIN_H,
                "seed {seed:#x}: a skirt element stands at y={} in the water",
                e.y
            );
        }
    }
}

/// Per-prop counts follow the published footprint, both rails included. This
/// is the arithmetic behind "a boulder is not skirted as thinly as a barrel".
#[test]
fn test_skirt_count_tracks_the_published_footprint() {
    let kinds = [
        Occupant::Tree,
        Occupant::StoneNode,
        Occupant::MetalNode,
        Occupant::SulfurNode,
        Occupant::Bush,
        Occupant::Rock,
        Occupant::BarrelSlot,
        Occupant::CrateSlot,
        Occupant::HavenShelter,
        Occupant::CacheSlot,
        Occupant::WaystationCanopy,
    ];
    assert_eq!(
        terrain::skirt_count(Occupant::None),
        0,
        "an empty cell must not be skirted"
    );
    for o in kinds {
        let n = terrain::skirt_count(o);
        assert!(
            (SKIRT_MIN..=SKIRT_MAX).contains(&n),
            "{o:?} draws {n} skirt elements, outside [{SKIRT_MIN}, {SKIRT_MAX}]"
        );
        // Reach is never tighter than one element's own footprint.
        assert!(
            terrain::skirt_base_r(o) > 0.0,
            "{o:?} has a zero skirt reach — the floor did not apply"
        );
    }
    // The ordering the reach floor exists to preserve: a wider prop is never
    // ringed by fewer elements than a narrower one.
    assert!(
        terrain::skirt_count(Occupant::Rock) >= terrain::skirt_count(Occupant::BarrelSlot),
        "a 1.5 m boulder is skirted more thinly than a 0.45 m barrel"
    );
}

/// Every element sits in the annulus its prop's footprint defines: outside
/// the collision radius (or it is buried in the mesh) and inside the band (or
/// it is a lawn, not a skirt). Measured against the slot's own position, so
/// this fails if the ring is ever re-centred by accident.
#[test]
fn test_skirt_elements_ring_their_own_prop() {
    for seed in SEEDS {
        let (tx, tz, haven, table) = a_tile_with_props(seed);
        let c0x = tx * SKIRT_TILE_CELLS - 1;
        let c0z = tz * SKIRT_TILE_CELLS - 1;
        let mut checked = 0;
        for dz in 0..4 {
            for dx in 0..4 {
                let (cx, cz) = (c0x + dx, c0z + dz);
                let slot = terrain::scatter(seed, &table, &haven, cx, cz);
                if slot.occupant == Occupant::None {
                    continue;
                }
                let r_b = terrain::skirt_base_r(slot.occupant);
                let n = terrain::skirt_count(slot.occupant);
                // Rebuild this prop's whole ring, unclipped, straight off the
                // tile buffer's own generator via a one-cell scan.
                let mut buf = [CLUTTER_NONE; SKIRT_PER_TILE];
                let got = terrain::skirt_fill(seed, &table, &haven, tx, tz, &mut buf);
                for e in buf.iter().take(got) {
                    let (ex, ez) = (e.x - slot.x, e.z - slot.z);
                    let d = (ex * ex + ez * ez).sqrt();
                    // Only elements belonging to THIS prop: the annulus is
                    // narrow enough that a neighbour's cannot fall inside it
                    // by accident at this distance test's tolerance.
                    if d > r_b + SKIRT_BAND_M + 0.001 {
                        continue;
                    }
                    assert!(
                        d >= r_b - 0.001,
                        "seed {seed:#x}: a {:?} skirt element is {d:.3} m out, inside its own \
                         {r_b:.3} m footprint",
                        slot.occupant
                    );
                    checked += 1;
                }
                assert!(n >= SKIRT_MIN);
            }
        }
        assert!(
            checked > 0,
            "seed {seed:#x}: tile ({tx},{tz}) yielded no element to range-check"
        );
    }
}

/// The ring is SPREAD, not clumped. This is the property angular stratification
/// buys, and the one a free-jitter ring silently loses: sixteen elements drawn
/// uniformly over a circle leave a bald arc often enough to see it.
///
/// Measured as quadrant occupancy on the fullest prop in a tile — with `n`
/// elements stratified over 4 quadrants, every quadrant holds at least
/// `floor(n/4) - 1` of them by construction, and free jitter does not.
#[test]
fn test_a_skirt_is_spread_not_clumped() {
    for seed in SEEDS {
        let (tx, tz, haven, table) = a_tile_with_props(seed);
        let c0x = tx * SKIRT_TILE_CELLS - 1;
        let c0z = tz * SKIRT_TILE_CELLS - 1;
        let mut tested = 0;
        for dz in 0..4 {
            for dx in 0..4 {
                let slot = terrain::scatter(seed, &table, &haven, c0x + dx, c0z + dz);
                let n = terrain::skirt_count(slot.occupant);
                // Only props whose whole ring is on land and inside the tile
                // can be judged on spread; a clipped ring is meant to be
                // partial. Rebuild unclipped from the generator.
                if n < 8 {
                    continue;
                }
                let mut quad = [0usize; 4];
                let mut on_land = 0;
                for i in 0..n {
                    let e = terrain::skirt_elem(seed, &haven, c0x + dx, c0z + dz, &slot, i, n);
                    if e.kind == Clutter::None {
                        continue;
                    }
                    on_land += 1;
                    let (ex, ez) = (e.x - slot.x, e.z - slot.z);
                    let q = ((ex >= 0.0) as usize) | (((ez >= 0.0) as usize) << 1);
                    quad[q] += 1;
                }
                if on_land < n {
                    continue; // partly in the water — spread is not its job
                }
                for (q, c) in quad.iter().enumerate() {
                    assert!(
                        *c + 1 >= n / 4,
                        "seed {seed:#x}: a {:?}'s {n}-element skirt puts only {c} in quadrant \
                         {q} — the stratification is not holding",
                        slot.occupant
                    );
                }
                tested += 1;
            }
        }
        assert!(
            tested > 0,
            "seed {seed:#x}: no prop with a full ring to check spread on"
        );
    }
}

/// THE BUDGET WALL. `SKIRT_PER_TILE` is what the client sizes its tile cache
/// and its pools for, so a tile that yields more is a silent drop at best.
/// Swept over real tiles, not argued from the bound.
#[test]
fn test_no_tile_exceeds_the_skirt_budget() {
    for seed in SEEDS {
        let table = ScatterTable::alpha_default();
        let haven = terrain::haven(seed);
        let (x, z) = a_land_origin(seed, 0);
        let (t0x, t0z) = ((x / CLUTTER_TILE_M) as i32, (z / CLUTTER_TILE_M) as i32);
        let mut worst = 0usize;
        for dz in -12..=12 {
            for dx in -12..=12 {
                let mut buf = [CLUTTER_NONE; SKIRT_PER_TILE];
                let n = terrain::skirt_fill(seed, &table, &haven, t0x + dx, t0z + dz, &mut buf);
                assert!(
                    n <= SKIRT_PER_TILE,
                    "seed {seed:#x}: tile ({},{}) yielded {n} skirt elements, over the \
                     {SKIRT_PER_TILE} the client sized for",
                    t0x + dx,
                    t0z + dz
                );
                worst = worst.max(n);
            }
        }
        assert!(worst > 0, "seed {seed:#x}: 625 tiles and not one skirt");
    }
}

/// A short buffer thins the skirt and never overruns — the same contract
/// `clutter_fill` carries, for the same reason: the bridge owns the buffer.
#[test]
fn test_a_short_skirt_buffer_truncates() {
    for seed in SEEDS {
        let (tx, tz, haven, table) = a_tile_with_props(seed);
        for cap in [1usize, 5, 17] {
            let mut buf = vec![CLUTTER_NONE; cap];
            let n = terrain::skirt_fill(seed, &table, &haven, tx, tz, &mut buf);
            assert!(
                n <= cap,
                "seed {seed:#x}: skirt_fill wrote {n} into a {cap}-element buffer"
            );
        }
    }
}

/// Deterministic on the seed and dependent on it — the replay wall, applied
/// to a population that is worldgen potential rather than sim state.
#[test]
fn test_skirts_are_deterministic_and_seed_dependent() {
    let (tx, tz, haven, table) = a_tile_with_props(SEEDS[0]);
    let mut a = [CLUTTER_NONE; SKIRT_PER_TILE];
    let mut b = [CLUTTER_NONE; SKIRT_PER_TILE];
    let na = terrain::skirt_fill(SEEDS[0], &table, &haven, tx, tz, &mut a);
    let nb = terrain::skirt_fill(SEEDS[0], &table, &haven, tx, tz, &mut b);
    assert_eq!(na, nb, "the same tile skirted two different counts");
    for i in 0..na {
        assert_eq!(
            a[i].kind, b[i].kind,
            "element {i} changed kind between calls"
        );
        assert_eq!(a[i].x, b[i].x, "element {i} moved in x between calls");
        assert_eq!(a[i].z, b[i].z, "element {i} moved in z between calls");
    }

    let haven2 = terrain::haven(SEEDS[1]);
    let mut c = [CLUTTER_NONE; SKIRT_PER_TILE];
    let nc = terrain::skirt_fill(SEEDS[1], &table, &haven2, tx, tz, &mut c);
    let same = na == nc && (0..na.min(nc)).all(|i| a[i].x == c[i].x && a[i].z == c[i].z);
    assert!(!same, "two seeds skirted the same tile identically");
}

// ── The coast: the ground every skirt test above walks past ────────────────
//
// The six skirt tests stand on `a_tile_with_props`, which walks east from a
// land origin near the island CENTRE. That is inland ground, and inland ground
// has no sand: swept over 900 inland tiles on these three seeds the skirt draws
// 4,745 elements and NOT ONE is a Pebble, while `skirt_elem`'s waterline veto
// returns `CLUTTER_NONE` exactly zero times out of 18,883 candidates.
//
// So two shipped branches had no test standing on them — the sand kind, and
// the veto itself. Both only exist where the land ends, which is also where a
// player spawns and where `TERRAIN.md` §7's coast road is meant to run.
//
// What the same sweep says is NOT wrong: beach skirts are thin (1.19 elements
// a tile against inland's 5.27), and that is prop density upstream in
// `scatter` — 0.22 prop centres a tile against 0.95 — not the skirt thinning
// itself. The ratios match to within a tenth. Nothing below gates a count per
// tile for that reason; a count would be gating the scatter table through the
// wrong file.

/// Tiles per side of the coastal block the three tests below sweep. It
/// straddles the waterline: the two seaward tiles are seabed and the rest
/// walks inland, so one block holds both the sand channel and the veto.
const COAST_BLOCK: i32 = 12;

/// Tiles the waterline actually runs through: some ground above `LAND_MIN_H`
/// and some below, inside the coastal annulus.
///
/// Defined by GROUND, never by yield. A first cut anchored one block on the
/// seed's +x ray and it landed on cliff coast — 517 skirt elements, 4.4%
/// Pebble and not one veto, because a cliff has no shallow margin for a ring
/// to cross. Searching for a block that produced Pebbles instead would have
/// been tuning the fixture to the assertion; this asks for the shape of the
/// ground and reports whatever stands on it.
fn waterline_tiles(seed: u64) -> Vec<(i32, i32)> {
    let c = 1024.0f32;
    let mut out = Vec::new();
    let span = (1050.0 / CLUTTER_TILE_M) as i32;
    let ct = (c / CLUTTER_TILE_M) as i32;
    for tz in (ct - span)..=(ct + span) {
        for tx in (ct - span)..=(ct + span) {
            let (mx, mz) = (
                tx as f32 * CLUTTER_TILE_M + CLUTTER_TILE_M * 0.5,
                tz as f32 * CLUTTER_TILE_M + CLUTTER_TILE_M * 0.5,
            );
            let d = ((mx - c) * (mx - c) + (mz - c) * (mz - c)).sqrt();
            if !(600.0..=1050.0).contains(&d) {
                continue;
            }
            let (mut land, mut sea) = (false, false);
            for j in 0..3 {
                for i in 0..3 {
                    let x = tx as f32 * CLUTTER_TILE_M + i as f32 * (CLUTTER_TILE_M * 0.5);
                    let z = tz as f32 * CLUTTER_TILE_M + j as f32 * (CLUTTER_TILE_M * 0.5);
                    if terrain::height(seed, x, z) < LAND_MIN_H {
                        sea = true;
                    } else {
                        land = true;
                    }
                }
            }
            if land && sea {
                out.push((tx, tz));
            }
        }
    }
    out
}

/// The inland control: a square block of tiles around a seed's land origin,
/// the same ground every skirt test above already stands on.
fn inland_tiles(seed: u64) -> Vec<(i32, i32)> {
    let (ix, iz) = a_land_origin(seed, 0);
    let tx0 = (ix / CLUTTER_TILE_M) as i32 - COAST_BLOCK / 2;
    let tz0 = (iz / CLUTTER_TILE_M) as i32 - COAST_BLOCK / 2;
    let mut out = Vec::new();
    for dz in 0..COAST_BLOCK {
        for dx in 0..COAST_BLOCK {
            out.push((tx0 + dx, tz0 + dz));
        }
    }
    out
}

/// A CONTIGUOUS block straddling the shoreline on a seed's +x ray — the
/// partition sweep needs neighbouring tiles, which a coastline scan does not
/// guarantee.
fn a_coast_block(seed: u64) -> (i32, i32, Haven, ScatterTable) {
    let table = ScatterTable::alpha_default();
    let haven = terrain::haven(seed);
    let c = 1024.0f32;
    // March out until the ground is sea and STAYS sea — an inland dip below
    // the waterline is a pond, and skirting a pond is not what this measures.
    let mut r = 200.0f32;
    let mut shore = None;
    while r < 1000.0 {
        if terrain::height(seed, c + r, c) < LAND_MIN_H
            && terrain::height(seed, c + r + 16.0, c) < LAND_MIN_H
            && terrain::height(seed, c + r + 32.0, c) < LAND_MIN_H
        {
            shore = Some(r);
            break;
        }
        r += 4.0;
    }
    let shore =
        shore.unwrap_or_else(|| panic!("seed {seed:#x}: the +x ray never reaches open sea"));
    let stx = ((c + shore) / CLUTTER_TILE_M) as i32;
    let stz = (c / CLUTTER_TILE_M) as i32;
    // Two tiles seaward of the shoreline tile, the remaining nine inland.
    (stx + 3 - COAST_BLOCK, stz - COAST_BLOCK / 2, haven, table)
}

/// Skirt elements over a set of tiles, counted by kind.
fn skirt_mix(
    seed: u64,
    table: &ScatterTable,
    haven: &Haven,
    tiles: &[(i32, i32)],
) -> ([usize; 5], usize) {
    let mut by_kind = [0usize; 5];
    let mut total = 0usize;
    for &(tx, tz) in tiles {
        let mut buf = [CLUTTER_NONE; SKIRT_PER_TILE];
        let n = terrain::skirt_fill(seed, table, haven, tx, tz, &mut buf);
        for e in buf.iter().take(n) {
            by_kind[e.kind as usize] += 1;
            total += 1;
        }
    }
    (by_kind, total)
}

/// The sand path, swept. The skirt shares `clutter_kind_at` with the grid, so
/// what it draws is whatever the splat under it says — and sand is a coastal
/// channel, which is why no inland sweep has ever exercised it.
///
/// Asserted as a CONTRAST rather than a threshold: the same generator over the
/// same number of tiles draws Pebbles on the coast and measurably fewer
/// inland. That is the claim "the kind tracks the ground" actually makes, and
/// unlike a fixed share it does not have to be re-tuned when the scatter table
/// moves. The margin is wide — the sweep behind this reads 43% against 0%.
#[test]
fn test_the_skirt_reaches_the_sand() {
    for seed in SEEDS {
        let table = ScatterTable::alpha_default();
        let haven = terrain::haven(seed);
        let coast_tiles = waterline_tiles(seed);
        assert!(
            coast_tiles.len() > 100,
            "seed {seed:#x}: only {} tiles on the waterline — the coastline scan missed the \
             island, so nothing below is measuring the coast",
            coast_tiles.len()
        );
        let (coast, coast_n) = skirt_mix(seed, &table, &haven, &coast_tiles);
        assert!(
            coast_n > 0,
            "seed {seed:#x}: {} waterline tiles skirt nothing at all",
            coast_tiles.len()
        );
        assert!(
            coast[Clutter::Pebble as usize] > 0,
            "seed {seed:#x}: {coast_n} skirt elements on the coast and not one Pebble — the \
             sand channel is unreachable from the skirt path"
        );

        let (inland, inland_n) = skirt_mix(seed, &table, &haven, &inland_tiles(seed));
        assert!(
            inland_n > 0,
            "seed {seed:#x}: the inland control block skirts nothing, so there is nothing to \
             contrast the coast against"
        );
        let coast_share = coast[Clutter::Pebble as usize] as f32 / coast_n as f32;
        let inland_share = inland[Clutter::Pebble as usize] as f32 / inland_n as f32;
        assert!(
            coast_share > inland_share + 0.10,
            "seed {seed:#x}: Pebble is {:.1}% of {coast_n} coastal skirt elements against \
             {:.1}% of {inland_n} inland — the skirt is not reading the sand splat",
            coast_share * 100.0,
            inland_share * 100.0
        );
    }
}

/// The waterline veto, executed. `skirt_elem` drops any element whose ground
/// is below `LAND_MIN_H` — "a prop on the waterline skirts only the half of
/// its ring that is on land". Inland that branch never runs, so until this
/// test it was shipped code no gate had entered.
///
/// Two halves, and the first is the one that rots quietly: the branch FIRES
/// somewhere on the coastline (a veto count of zero means the sweep missed the
/// water or the veto stopped working, and both are failures), and nothing that
/// survives it stands in the sea.
#[test]
fn test_the_waterline_veto_fires_where_the_land_ends() {
    for seed in SEEDS {
        let table = ScatterTable::alpha_default();
        let haven = terrain::haven(seed);
        let tiles = waterline_tiles(seed);
        assert!(
            tiles.len() > 100,
            "seed {seed:#x}: only {} tiles on the waterline — the coastline scan missed the \
             island",
            tiles.len()
        );
        let mut vetoed = 0usize;
        let mut kept = 0usize;
        for &(tx, tz) in tiles.iter() {
            let c0x = tx * SKIRT_TILE_CELLS - 1;
            let c0z = tz * SKIRT_TILE_CELLS - 1;
            for cz in 0..SKIRT_SCAN_CELLS {
                for cx in 0..SKIRT_SCAN_CELLS {
                    let (gx, gz) = (c0x + cx, c0z + cz);
                    let slot = terrain::scatter(seed, &table, &haven, gx, gz);
                    let n = terrain::skirt_count(slot.occupant);
                    for i in 0..n {
                        let e = terrain::skirt_elem(seed, &haven, gx, gz, &slot, i, n);
                        if e.kind == Clutter::None {
                            vetoed += 1;
                        } else {
                            kept += 1;
                            assert!(
                                e.y >= LAND_MIN_H,
                                "seed {seed:#x}: a surviving skirt element stands at y={}, \
                                 below the {LAND_MIN_H} waterline",
                                e.y
                            );
                        }
                    }
                }
            }
        }
        assert!(
            kept > 0,
            "seed {seed:#x}: {} waterline tiles generated no skirt candidates",
            tiles.len()
        );
        assert!(
            vetoed > 0,
            "seed {seed:#x}: {kept} skirt candidates across a block straddling the shoreline \
             and the waterline veto never fired once — either the sweep missed the coast or \
             the branch is dead"
        );
    }
}

/// "A prop straddling a tile edge is skirted once and not twice" — the tile
/// ownership claim in `terrain.rs`'s skirt note, which until now was a comment.
///
/// It rests on an arithmetic premise nobody had written down: a tile scans a
/// ONE-CELL apron, so the claim holds only while no prop's skirt reaches
/// further than one scatter cell. The widest is the haven shelter's 4.9498 m
/// plus the 0.45 m band, against an 8 m cell — 2.6 m of margin, and a wider
/// authored occupant would spend it in silence. The elements would be
/// generated by cells that no owning tile visits, so they would simply not be
/// drawn: no panic, no golden move, no clippy wall.
///
/// Part one gates that premise off the published radius table, so it grows
/// when a row does. Part two sweeps the coast and checks the partition element
/// by element — every candidate the block's cells generate is emitted by
/// exactly one of its tiles, and by the tile whose bounds contain it.
#[test]
fn test_a_prop_is_skirted_by_exactly_one_tile() {
    for (i, r) in OCCUPANT_R_M.iter().enumerate() {
        let base = if *r > SKIRT_MIN_R_M {
            *r
        } else {
            SKIRT_MIN_R_M
        };
        let reach = base + SKIRT_BAND_M;
        assert!(
            reach <= CELL_SIZE,
            "occupant row {i} reaches {reach} m, past the {CELL_SIZE} m apron `skirt_fill` \
             scans — its skirt would be generated by cells no owning tile visits"
        );
    }

    for seed in SEEDS {
        let (tx0, tz0, haven, table) = a_coast_block(seed);
        let mut emitted: Vec<Vec<ClutterElem>> = Vec::new();
        for dz in 0..COAST_BLOCK {
            for dx in 0..COAST_BLOCK {
                let mut buf = [CLUTTER_NONE; SKIRT_PER_TILE];
                let n = terrain::skirt_fill(seed, &table, &haven, tx0 + dx, tz0 + dz, &mut buf);
                // The partition is only legible on an untruncated fill: a
                // capped tile drops elements by contract, and this sweep could
                // not tell one of those from an unowned one.
                assert!(
                    n < SKIRT_PER_TILE,
                    "seed {seed:#x}: tile ({},{}) filled its {SKIRT_PER_TILE}-element buffer",
                    tx0 + dx,
                    tz0 + dz
                );
                emitted.push(buf[..n].to_vec());
            }
        }

        // Every cell any block tile scans — the block plus its one-tile apron,
        // which is a superset of the four-cell aprons the fills walked.
        let mut checked = 0usize;
        for dz in -1..=COAST_BLOCK {
            for dx in -1..=COAST_BLOCK {
                for cz in 0..SKIRT_TILE_CELLS {
                    for cx in 0..SKIRT_TILE_CELLS {
                        let gx = (tx0 + dx) * SKIRT_TILE_CELLS + cx;
                        let gz = (tz0 + dz) * SKIRT_TILE_CELLS + cz;
                        let slot = terrain::scatter(seed, &table, &haven, gx, gz);
                        let n = terrain::skirt_count(slot.occupant);
                        for i in 0..n {
                            let e = terrain::skirt_elem(seed, &haven, gx, gz, &slot, i, n);
                            if e.kind == Clutter::None {
                                continue;
                            }
                            // The tile whose half-open bounds contain it. Only
                            // block tiles have an emitted list to check.
                            // Floor-by-cast, wall 1's form: island coordinates
                            // are positive, so truncation IS floor here — and
                            // if it ever were not, the owner would be wrong and
                            // the assertion below would say so out loud.
                            let otx = (e.x / CLUTTER_TILE_M) as i32;
                            let otz = (e.z / CLUTTER_TILE_M) as i32;
                            let (bx, bz) = (otx - tx0, otz - tz0);
                            if !(0..COAST_BLOCK).contains(&bx) || !(0..COAST_BLOCK).contains(&bz) {
                                continue;
                            }
                            let hits: usize = emitted
                                .iter()
                                .map(|t| t.iter().filter(|q| q.x == e.x && q.z == e.z).count())
                                .sum();
                            assert_eq!(
                                hits, 1,
                                "seed {seed:#x}: element {i} of cell ({gx},{gz}) at ({},{}) \
                                 was emitted by {hits} tiles of the block, not by 1",
                                e.x, e.z
                            );
                            let own = &emitted[(bz * COAST_BLOCK + bx) as usize];
                            assert!(
                                own.iter().any(|q| q.x == e.x && q.z == e.z),
                                "seed {seed:#x}: the element at ({},{}) was emitted, but not \
                                 by tile ({otx},{otz}) whose bounds contain it",
                                e.x,
                                e.z
                            );
                            checked += 1;
                        }
                    }
                }
            }
        }
        assert!(
            checked > 0,
            "seed {seed:#x}: the coastal block owns no skirt element to partition"
        );
    }
}

// ── §S · The authored sites sweep their own floor ──────────────────────────
//
// `reference/MONUMENTS.md` §9.2. Until the site footprint block landed, this
// population had no `Haven` parameter at all: grass, twigs and scree grew
// straight across the haven pad and both waystations while the carriageway
// running through them was correctly swept to grit. The road got an override;
// the destination it leads to did not.
//
// **Every assertion below is exact, not statistical, and that is deliberate.**
// The natural way to test "the pad has less grass on it" is to count blades
// and compare — which would pass on a change that also thinned the wilderness,
// and would be blind to the failure that actually matters. So the control is
// the SAME seed rendered against an inert site list (`sites_parked_offshore`),
// and the three claims are differences against it:
//
//   1. where the sweep is full, the floor is grit and carries no understory;
//   2. where the sweep is zero, the field is bit-identical to the control —
//      this change may not move one element of the island;
//   3. across the band between them, BOTH outcomes occur — the transition is
//      a dithered population, not a ring drawn on the ground.
//
// Claim 3 is the one `MONUMENTS.md` §3 is about: the reference game shipped
// monuments on visible circular plateaus for years because a footprint was a
// radius. A gate that only checked 1 and 2 would pass a hard circle.

/// A site list parked off the island, so `site_sweep` answers 0.0 everywhere
/// a real scan looks. The control for every difference measured below —
/// `occupy.rs` builds the same thing for the same reason.
fn sites_parked_offshore() -> Haven {
    Haven {
        x: -100_000.0,
        z: -100_000.0,
        y: 0.0,
        relief: 0.0,
        phase: 0,
        shelter: 0,
        minor: [terrain::Waystation::NONE; terrain::WAYSTATIONS],
    }
}

/// Every live site of a seed, as (centre, footprint).
fn sites_of(haven: &Haven) -> Vec<(f32, f32, terrain::SiteFootprint)> {
    let mut v = vec![(haven.x, haven.z, terrain::HAVEN_FOOTPRINT)];
    for ws in haven.minor.iter().filter(|w| w.live) {
        v.push((ws.x, ws.z, terrain::WAYSTATION_FOOTPRINT));
    }
    v
}

/// The profile is a band, as a property of the function itself: 1.0 on the
/// floor, 0.0 at the scatter mask and outside it, strictly between across the
/// band, and never rising as you walk out.
///
/// Asserted on `site_sweep` directly as well as on the population, because a
/// population test cannot tell a narrow band from no band — and the band is
/// the whole difference between this and the circular plateau `MONUMENTS.md`
/// §3 is written about.
#[test]
fn test_the_sweep_profile_is_a_band_and_not_a_step() {
    for &seed in SEEDS.iter() {
        let haven = terrain::haven(seed);
        for (sx, sz, fp) in sites_of(&haven) {
            let at = |r: f32| terrain::site_sweep(&haven, sx + r, sz);
            assert_eq!(at(0.0), 1.0, "seed {seed:#x}: the site centre is not swept");
            assert_eq!(
                at(fp.swept_m),
                1.0,
                "seed {seed:#x}: the floor's own edge is not fully swept"
            );
            assert_eq!(
                at(fp.scatter_m),
                0.0,
                "seed {seed:#x}: the sweep reaches past the scatter mask"
            );
            let mid = (fp.swept_m + fp.scatter_m) * 0.5;
            let m = at(mid);
            assert!(
                m > 0.0 && m < 1.0,
                "seed {seed:#x}: sweep at the band's midpoint is {m}, so the \
                 boundary is a step and not a band"
            );
            // Monotone out, in 5 cm steps: a profile that rose anywhere would
            // draw a second edge further out than the one it is hiding.
            let mut prev = 1.0f32;
            let mut r = 0.0f32;
            while r <= fp.scatter_m + 1.0 {
                let v = at(r);
                assert!(
                    v <= prev + 1e-6,
                    "seed {seed:#x}: sweep rises from {prev} to {v} at r {r:.2}"
                );
                prev = v;
                r += 0.05;
            }
        }
    }
}

/// The three claims, measured cell by cell against the parked control.
#[test]
fn test_an_authored_site_sweeps_its_own_floor() {
    let far = sites_parked_offshore();
    // Pooled across seeds and sites: the band must contain both outcomes
    // somewhere, not in every one of the nine sites individually — a
    // waystation whose band is half in the sea has fewer cells to say it with.
    let (mut band_swept, mut band_kept) = (0usize, 0usize);
    let mut floor_cells = 0usize;

    for &seed in SEEDS.iter() {
        let haven = terrain::haven(seed);
        for (sx, sz, fp) in sites_of(&haven) {
            let pad = fp.scatter_m + 2.0 * CLUTTER_CELL_M;
            let c0x = ((sx - pad) / CLUTTER_CELL_M) as i32;
            let c1x = ((sx + pad) / CLUTTER_CELL_M) as i32;
            let c0z = ((sz - pad) / CLUTTER_CELL_M) as i32;
            let c1z = ((sz + pad) / CLUTTER_CELL_M) as i32;

            for cz in c0z..=c1z {
                for cx in c0x..=c1x {
                    let e = terrain::clutter_cell(seed, &haven, cx, cz);
                    let r = terrain::clutter_rich_cell(seed, &haven, cx, cz);
                    let e0 = terrain::clutter_cell(seed, &far, cx, cz);
                    let r0 = terrain::clutter_rich_cell(seed, &far, cx, cz);

                    // **The two strata are classified by their OWN positions,
                    // not by the cell's.** They jitter independently inside the
                    // same 0.64 m cell, so near a mask edge one can be swept
                    // while the other is not — and the first draft of this test
                    // judged both by the coverage element's position, which the
                    // hard-circle mutant caught as a false failure of claim 2.
                    // The point `swept_here` was asked about is the only point
                    // that can be asked about here.
                    if e0.kind == Clutter::None {
                        // Sea or off-island: the control has nothing to say and
                        // the site cannot have added anything.
                        assert_eq!(
                            e.kind,
                            Clutter::None,
                            "seed {seed:#x}: the sweep created an element in the sea"
                        );
                    } else {
                        let sweep = terrain::site_sweep(&haven, e0.x, e0.z);
                        if sweep >= 1.0 {
                            floor_cells += 1;
                            // 1 · grit, and the coverage guarantee survives it:
                            // Pebble rather than None, so no hole is punched in
                            // the population `test_no_bare_patch_inside
                            // _fifteen_metres` measures.
                            assert_eq!(
                                e.kind,
                                Clutter::Pebble,
                                "seed {seed:#x}: a fully swept floor at ({:.1}, {:.1}) grows \
                                 {:?} instead of grit",
                                e0.x,
                                e0.z,
                                e.kind
                            );
                        } else if sweep <= 0.0 {
                            // 2 · outside the mask nothing moved, bit for bit.
                            assert_eq!(
                                (e.kind, e.x.to_bits(), e.z.to_bits(), e.yaw),
                                (e0.kind, e0.x.to_bits(), e0.z.to_bits(), e0.yaw),
                                "seed {seed:#x}: the sweep changed a coverage element at \
                                 ({:.1}, {:.1}), outside every site's scatter mask",
                                e0.x,
                                e0.z
                            );
                        }
                    }

                    // The sweep only ever REMOVES understory, so a cell the
                    // control left empty must still be empty.
                    if r0.kind == Clutter::None {
                        assert_eq!(
                            r.kind,
                            Clutter::None,
                            "seed {seed:#x}: the sweep created an understory element"
                        );
                        continue;
                    }
                    let sweep = terrain::site_sweep(&haven, r0.x, r0.z);
                    if sweep >= 1.0 {
                        // 1 · a made floor has no understory at all.
                        assert_eq!(
                            r.kind,
                            Clutter::None,
                            "seed {seed:#x}: a fully swept floor at ({:.1}, {:.1}) carries \
                             an understory element",
                            r0.x,
                            r0.z
                        );
                    } else if sweep <= 0.0 {
                        // 2 · outside the mask nothing moved, bit for bit.
                        assert_eq!(
                            (r.kind, r.x.to_bits(), r.z.to_bits(), r.yaw),
                            (r0.kind, r0.x.to_bits(), r0.z.to_bits(), r0.yaw),
                            "seed {seed:#x}: the sweep changed an understory element at \
                             ({:.1}, {:.1}), outside every site's scatter mask",
                            r0.x,
                            r0.z
                        );
                    } else if r.kind == Clutter::None {
                        // 3 · in the band, count which way each element fell.
                        // The understory is the discriminator and the coverage
                        // kind is not: the splat draws Pebble on sand of its
                        // own accord, while `clutter_rich_cell` refuses
                        // outright when swept and is untouched when not.
                        band_swept += 1;
                    } else {
                        band_kept += 1;
                    }
                }
            }
        }
    }

    assert!(
        floor_cells > 500,
        "only {floor_cells} cells landed on a swept floor across every seed — \
         the scan is not reaching the sites it means to measure"
    );
    assert!(
        band_swept > 0 && band_kept > 0,
        "the band swept {band_swept} understory elements and kept {band_kept}: \
         a boundary that is all one or all the other is a ring drawn on the \
         ground, which is the failure reference/MONUMENTS.md §3 is about"
    );
}
