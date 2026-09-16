//! `test_terrain_golden` (TERRAIN.md §0/§7): fixed seed → pinned hash of
//! 64×64 sampled heights + 256 scatter results. The wasm half of the same
//! assertion runs in ci/gates.sh via ci/parity.mjs against the same pin.
//! Plus shape sanity: the generator must produce an island, not a puddle.

use sim_core::probe::{
    probe_road_point, probe_side_road_point, probe_terrain, probe_window_origin,
    PROBE_ROAD_BEARINGS, PROBE_ROAD_RADII, PROBE_SIDE_ROAD_SAMPLES, PROBE_WINDOW_CELLS,
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
/// Regenerated here from `0x17FA_A7E3_3CAE_FB50` for **the side road**
/// (2026-09-16, `reference/ROADS.md` §9.2.2–3): the inland site now has a
/// road to it, and `road_band` stopped being a pure function of
/// `(seed, x, z)`.
///
/// **Three things move it and nothing else can**, in falling order of size:
///
/// - `scatter` and the clutter population veto the carriageway and draw
///   barrels on the shoulder, and there are now ~330 more road cells per
///   island in the interior. That is the bulk of it.
/// - `probe_sites` hashes each road's polyline and then walks it, 129 band
///   samples apiece — bytes that did not exist before, so the digest would
///   move on shape alone.
/// - `splat_road` paints the new band, which `probe_terrain`'s height window
///   does not see but its scatter windows do.
///
/// **What does NOT move it is every existing site**, and that is structural
/// rather than lucky: `solve_side_roads` runs after `pick_minor`, and every
/// solver that chooses a site calls `ring_band` — the ring half, which is the
/// same function it always called under a new name. A site cannot be moved by
/// a road that is a consequence of where it landed.
///
/// Regenerated from `0xAA93_9FA3_DF14_702C` for **the inland site**
/// (2026-09-16, `reference/ROADS.md` §9.2.1): `INLAND_SITES` 0 -> 1, so
/// `Haven::minor` is three slots instead of two and the third holds a site
/// the road ring is not the reference curve for.
///
/// **The reach is bounded and the bound is stated**, because "a new site
/// moved the golden" could hide anything. Three things move it and nothing
/// else can:
///
/// - `probe_sites` hashes `minor` as an array, so a third entry is three more
///   floats and a `kind` byte per site — a change of SHAPE, which would move
///   the digest even if every existing site were bit-identical, and they are:
///   `pick_minor`'s ring loop is untouched and runs first, and the pad is
///   resolved before either.
/// - `probe_scatter` hashes a window at every entry of `minor`, so it now
///   covers one more disc.
/// - The inland site carves, like every site (`site_sweep`, `stamp_of`, both
///   of which iterate the array), so ground inside one more `blend_m` moves.
///   `tests/carve.rs` §C holds that nothing outside any footprint changes at
///   all, bit for bit, islandwide — and it CAUGHT this one, because its site
///   list read `0..WAYSTATIONS` and the new disc read as a leak.
///
/// Regenerated from `0x700C_77A5_8F33_97B4` for **world structure v1**
/// (2026-09-15) — the largest deliberate worldgen change since the shape
/// pass, and five mechanisms rather than one:
///
/// - `MOIST_FREQ` 1/700 -> 1/240 at three octaves, so the Forest biome is
///   ~20 woods rather than one continent. This moves the biome, the splat
///   and therefore the scatter mix at most land samples.
/// - A **shore terrace** in `height` (`terrain::shore_terrace`): every
///   sample with `0 < h < 16` is reshaped. `f(0) = 0` exactly, so the h = 0
///   set — the coastline — is bit-for-bit where it was, and `f(h) = h`
///   outside the band, so nothing above 16 m moved at all.
/// - A **second coastline term** (`COAST_BAY_FREQ`/`COAST_BAY_WOBBLE`),
///   which DOES move the h = 0 set: this is the part that changes the
///   island's outline, and with it where the road ring and the three
///   authored sites land.
/// - The **treeline transfer** (`EDGE_TREE_TO_BUSH`), which moves the
///   occupant on some border cells and no positions.
/// - `Slot::species`, a new byte this probe hashes.
///
/// Anything but the coast term is bounded and the bounds are asserted next
/// door: `tests/relief.rs` holds the terrace's joins exactly, and
/// `tests/forest.rs` holds what the moisture field and the transfer did.
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
/// Regenerated again the same day, from `0x97E7_4336_299A_D2FD`, for the second
/// half of the same work: a site's floor is cut to the level of LOWEST ERROR
/// over the ground it flattens (`Haven::floor_y`, `site_floor_y`) rather than
/// to the raw height at its centre — the reference's own terrain anchoring,
/// `reference/MONUMENTS.md` §9.2b. `probe_sites` hashes the new datum, and the
/// three seated arrangements move with it.
///
/// A previous regeneration, kept because it explains the value it replaced: the
/// road shoulder's barrel rate stopped being one number and became two — dense
/// on the coast's sheltered arcs, sparse on the open shore (`terrain::in_bay`,
/// TERRAIN.md §1 stage 7's "junk piles at bay mouths" line). And before that,
/// the scatter pass stopped choosing a biome row and started blending four
/// (`terrain::scatter_row`), a delta confined to the ~11% of land cells no
/// single splat channel owns outright.
/// Regenerated here from `0x7356_4E57_06B4_74D9` for **the shape of the
/// island itself** (2026-08-26): `remap` stopped being piecewise-linear
/// between its knots and became a monotone cubic, the highland's ridged blend
/// became a three-octave ridged multifractal on a `fade`d gate, and a detail
/// ladder was added after the curve. The island had been rendering as a
/// stack of terraces with a contour line at every one of the LUT's 16 knots;
/// `terrain.rs`'s own `remap` docs carry the mechanism.
///
/// This is a bigger move than the carve was — the carve touched three discs
/// and this touches every sample above the waterline — so what bounds it is
/// stated rather than implied: the **coast does not move**, because
/// `REMAP_LUT`'s first three segments have equal secants (the cubic through
/// them IS the old straight line) and the detail rides on `shelf`, which is 0
/// at sea level. The road ring, the haven solve and the clutter waterline
/// veto are all gated on that and all stayed green through the change without
/// a tolerance moving.
///
/// **Moved `0x9033_206F_0ECB_E2A4` → `0x700C_77A5_8F33_97B4` at forest density
/// v1** (2026-09-14, operator: *"when can we get forest fr?"*). The Forest
/// row went 350 → 700‰ with its grove field capped at the field's mean and
/// re-normalized (`ScatterTable::clump_cap`), so roughly every other Forest
/// cell that drew nothing draws a tree: ~39 → ~94 stems/ha, the scatter half
/// of this digest on ~45% of the land. Heights did not move — the change is
/// entirely in `scatter`, and `probe_terrain`'s height window would read the
/// same. Deliberate, regenerated in the commit that caused it.
const GOLDEN_TERRAIN_HASH: u64 = 0xD068_06D7_B0DD_146F;

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
#[derive(Default)]
struct Authored {
    shelters: i32,
    crates: i32,
    caches: i32,
    /// **Counted since the inland tier landed, and it is what keeps this gate
    /// from being green over nothing.** That tier stands no containers
    /// (`terrain::INLAND_CRATES`), so every count above reads zero at its
    /// window — which is exactly what an empty window reads, and exactly the
    /// failure the doc below says this test exists to refuse. The canopy is
    /// the one occupant every lesser site owes.
    canopies: i32,
}

fn window_occupants(seed: u64, haven: &terrain::Haven, x: f32, z: f32) -> Authored {
    let table = ScatterTable::alpha_default();
    let (cx0, cz0) = probe_window_origin(x, z);
    let mut a = Authored::default();
    for cz in cz0..cz0 + PROBE_WINDOW_CELLS {
        for cx in cx0..cx0 + PROBE_WINDOW_CELLS {
            match terrain::scatter(seed, &table, haven, cx, cz).occupant {
                Occupant::HavenShelter => a.shelters += 1,
                Occupant::CrateSlot => a.crates += 1,
                Occupant::CacheSlot => a.caches += 1,
                Occupant::WaystationCanopy => a.canopies += 1,
                _ => {}
            }
        }
    }
    a
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
        let w = window_occupants(seed, &h, h.x, h.z);
        let (shelters, crates, caches) = (w.shelters, w.crates, w.caches);
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
                "seed {seed:#x}: lesser site {i} ({:?}) is not live, so the \
                 parity surface is covering `Waystation::NONE` at the island \
                 corner rather than a site (tests/waystation.rs owns the tier)",
                ws.kind
            );
            let w = window_occupants(seed, &h, ws.x, ws.z);
            let (shelters, crates, caches) = (w.shelters, w.crates, w.caches);
            // The canopy FIRST, because on a tier that stands no containers
            // it is the only thing separating this window from empty sea.
            assert_eq!(
                w.canopies, 1,
                "seed {seed:#x}: the golden's window at lesser site {i} \
                 ({:?}) holds {} canopies — a site the digest cannot see is \
                 the hole this gate exists to refuse",
                ws.kind, w.canopies
            );
            assert_eq!(
                caches,
                terrain::site_crates(ws.kind),
                "seed {seed:#x}: the golden's window at lesser site {i} \
                 ({:?}) holds {caches} of {} containers",
                ws.kind,
                terrain::site_crates(ws.kind)
            );
            // The pad's own container kind is the pad's alone, for the same
            // reason its greybox is: a `CrateSlot` here would be the lesser
            // tier drawing `loot.crate`, which is precisely what it did until
            // the two kinds were split.
            assert_eq!(
                crates, 0,
                "seed {seed:#x}: lesser site {i} stands {crates} of the pad's \
                 own container kind — the lesser tier is paying the \
                 destination's loot table"
            );
            // The greybox is the destination's alone — it is what makes the
            // pad read as the better place on sight, and the gradient the
            // lesser tier exists to create depends on it staying there.
            assert_eq!(
                shelters, 0,
                "seed {seed:#x}: lesser site {i} grew a shelter — the tier \
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
                // `ring_band`: this sweep is the RING's coverage, and a
                // side road crossing a radial would flatter it.
                if terrain::ring_band(seed, px, pz) != terrain::RoadBand::Off {
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

        // And the side-road half of `probe_sites`, for exactly the same
        // reason and against a harder failure: that sweep walks a polyline
        // whose two ends are stored floats, so a road that came back dead —
        // or one whose ends collapsed to the same point — would sample one
        // spot 129 times and hash a constant that looks like coverage. This
        // is the count of samples that land on the road's own surface.
        for (i, road) in h.roads.iter().enumerate() {
            assert!(
                road.live,
                "seed {seed:#x}: side road {i} is dead, so the parity surface \
                 is sampling the island's origin 129 times (tests/road.rs \
                 owns whether a seed is allowed a dead road)"
            );
            let mut on = 0i32;
            for k in 0..PROBE_SIDE_ROAD_SAMPLES {
                let (sx, sz) = probe_side_road_point(road, k);
                if terrain::side_band(&h, sx, sz) == terrain::RoadBand::Carriageway {
                    on += 1;
                }
            }
            // Every sample but the two endpoints is strictly inside the
            // segment, and `ROAD_HALF_W` is 2 m, so a walk along the line
            // itself is on the carriageway at every one of them. A floor of
            // "nearly all" rather than "all" leaves the ends their rounding.
            assert!(
                on >= PROBE_SIDE_ROAD_SAMPLES - 2,
                "seed {seed:#x}: only {on} of {PROBE_SIDE_ROAD_SAMPLES} \
                 samples along side road {i} land on its own carriageway — \
                 the sweep is not walking the road it names"
            );
        }
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

    // TERRAIN.md §6: ~14–17k live slots per seed since forest density v1
    // (~8–12k before it). The band is the doc's and `tests/scatter.rs`'s; a
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
        (13_000..=19_000).contains(&live),
        "live slots {live} outside TERRAIN.md §6's 13–19k band (trees {trees}, ore {ore}, barrels {barrels})"
    );
    assert!(trees > 1_000, "island needs wood: {trees} trees");
    assert!(ore > 300, "island needs ore: {ore} nodes");
    assert!(barrels > 50, "the loot route needs barrels: {barrels}");
}
