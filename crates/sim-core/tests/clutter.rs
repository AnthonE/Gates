//! Ground clutter: the sub-metre population that answers `ART.md` rule 4.
//!
//! The rule is a number — "any visible ground patch larger than ~3 m² inside
//! 15 m carries scatter" — so this suite is arithmetic, not a screenshot. The
//! wall it holds is `test_no_bare_patch_inside_fifteen_metres`, which MEASURES
//! the largest empty disc rather than trusting the grid argument that predicts
//! it. Everything else here defends a premise that measurement rests on.

use sim_core::terrain::{
    self, Clutter, ClutterElem, Haven, Occupant, ScatterTable, CLUTTER_CELLS_PER_SIDE,
    CLUTTER_CELLS_PER_TILE, CLUTTER_CELL_M, CLUTTER_NONE, CLUTTER_PER_TILE, CLUTTER_TILE_M,
    LAND_MIN_H, SKIRT_BAND_M, SKIRT_MAX, SKIRT_MIN, SKIRT_PER_TILE, SKIRT_TILE_CELLS,
};

const SEEDS: [u64; 3] = [0x0047_4154_4553, 1, 0xDEAD_BEEF];

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

/// A tile with props in it, for a seed — the fixture the skirt tests stand on.
fn a_tile_with_props(seed: u64) -> (i32, i32, Haven, ScatterTable) {
    let table = ScatterTable::alpha_default();
    let haven = terrain::haven(seed);
    let (x, z) = a_land_origin(seed, 0);
    let (t0x, t0z) = ((x / CLUTTER_TILE_M) as i32, (z / CLUTTER_TILE_M) as i32);
    // Walk a few tiles until one actually holds a prop; a single tile is 16 m
    // and the scatter grid is sparse enough that the first can be empty.
    for d in 0..24 {
        let (tx, tz) = (t0x + d, t0z);
        let mut buf = [CLUTTER_NONE; SKIRT_PER_TILE];
        if terrain::skirt_fill(seed, &table, &haven, tx, tz, &mut buf) > 0 {
            return (tx, tz, haven, table);
        }
    }
    panic!("seed {seed:#x}: no tile within 24 of the land origin holds a skirted prop");
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
                    let e = terrain::skirt_elem(seed, c0x + dx, c0z + dz, &slot, i, n);
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
