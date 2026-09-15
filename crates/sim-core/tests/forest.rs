//! Is the forest a forest? The per-biome structure of the scatter field
//! (`reference/FORESTS.md` §8, `TERRAIN.md` §1 stage 9).
//!
//! **Why this file exists, stated as the hole it fills.** `tests/scatter.rs`
//! already holds two things well: the field *clusters* (dispersion against a
//! closed-form independent-draw null) and the clump field *redistributes
//! density without spending it* (island-wide live slots in
//! `TERRAIN.md` §6's 8,000–12,000). Both are correct and neither can see the
//! defect this file was written for, because **both are island-wide and the
//! defect is per-biome**. Raise the meadow's trees and drop the forest's by
//! the same count and every assertion in that file stays green while the
//! forest stops being a distinguishable place.
//!
//! That is not hypothetical. The understory gate below was written against
//! the scatter grid's bush and was **red on the tree the day it was written**
//! — Forest drew bushes at 50‰ against Meadow's 70‰, so our forest floor was
//! thinner than our open field, the exact inversion of the reference's own
//! forest pass (Devblog 67, *"Made forests appear thicker by adding more
//! bushes"*; `reference/FORESTS.md` §5 step 3). It could not be fixed there:
//! `tests/scatter.rs::test_no_biome_row_saturates` caps the Forest row's bush
//! at 70.4‰, which is the Meadow's weight exactly, so the gate now measures
//! the layer where it actually fits — `Clutter::Brush`. The history is kept
//! because the arithmetic that closed the scatter route is the finding, and
//! someone will otherwise re-propose the two-number edit. **The rail moved
//! on 2026-09-14** (`ScatterTable::clump_cap` holds the Forest's field at
//! its mean and re-normalizes, so the row may total 748‰), and the tree took
//! the room — the understory stays where it is, because a fixed-count layer
//! has no ceiling to spend.
//!
//! **What a gate here may and may not assert.** These are *distribution*
//! gates, and a band wide enough to hold both the current tree and the
//! defect reads as coverage while checking nothing — `CLAUDE.md`'s
//! `lattice.rs` entry, one level up. So every band below was set from a
//! measurement printed by this file and then **proven red under a stated
//! mutant**, named on each test. The mutants are all one-line moves of the
//! weight table, which is the thing these gates exist to watch.
//!
//! Nothing here needs a crown radius, so nothing here can answer "does the
//! canopy close" — that is a render fact (the crown is
//! `tree::SPECIES[_].max_r_m`, in the client crate) and it lives in
//! `crates/client/tests/canopy.rs`.

// The measurements ARE the gate's output, exactly as in `tests/scatter.rs`
// and `tests/haven.rs`: the L5 wall bans format/print in SIM code, and a test
// harness is not sim code. The float walls are not relaxed — no `powf`, no
// `f64::sqrt`, no `abs`; everything below is `+ − × ÷` and a comparison.
#![allow(clippy::disallowed_macros)]

use sim_core::terrain::{
    self, Biome, Clutter, Occupant, ScatterTable, BRUSH_SHARE_PERMILLE, CELLS_PER_SIDE, CELL_SIZE,
    CLUTTER_NONE, CLUTTER_PER_TILE, CLUTTER_TILE_M, LAND_MIN_H,
};

/// Seeds every check runs over, matching `tests/scatter.rs`'s list for the
/// reason that file gives: one island can be atypical, and a threshold that
/// only holds on the shipped seed is a pin rather than a gate. Seed 7 in
/// particular carries a third less forest than seed 1.
const SEEDS: [u64; 4] = [0, 1, 7, 12345];

const BIOMES: usize = 4;

/// Square metres of ground one scatter cell owns. The grid draws at most one
/// occupant per cell, so `10_000 / CELL_AREA_M2` — **156.25 per hectare** —
/// is the hard ceiling on the density of anything, at any weight. Worth
/// naming because it is the number `reference/FORESTS.md` §1.2 prices the
/// whole forest against: a closed canopy needs more stems than this grid can
/// hold, so the cell size and not the weight table is what caps the forest.
const CELL_AREA_M2: f64 = (CELL_SIZE * CELL_SIZE) as f64;

/// Occupants of one island, counted per biome over the whole 0..2048 square.
///
/// **Whole-island on both axes, deliberately.** `sim-core/tests/relief.rs`
/// holds the retraction of a sweep that ran `-1024..1024` and therefore
/// sampled one quadrant of an island centred at `(ISLAND_SIZE/2,
/// ISLAND_SIZE/2)` — consistent across forty seeds, and aimed at the wrong
/// place the whole time. Iterating the cell grid `0..CELLS_PER_SIDE` is the
/// window that cannot make that mistake, because it *is* the world.
struct Census {
    /// Land cells whose biome is this one.
    cells: [u32; BIOMES],
    tree: [u32; BIOMES],
    /// The scatter grid's bush. Kept for the report rather than for a gate:
    /// it is a harvestable slot (berries, cloth) and NOT the understory —
    /// see `the_forest_understory_is_thicker_than_the_meadows` for why the
    /// layer could not live here.
    bush: [u32; BIOMES],
}

impl Census {
    /// Occupants per hectare of this biome's ground. Zero cells reads zero
    /// rather than dividing — an island with no highland is a real island,
    /// and the caller's assertion should fail on the count, not on a NaN.
    fn per_ha(&self, counts: &[u32; BIOMES], b: Biome) -> f64 {
        let i = b as usize;
        if self.cells[i] == 0 {
            return 0.0;
        }
        f64::from(counts[i]) / f64::from(self.cells[i]) * (10_000.0 / CELL_AREA_M2)
    }

    fn trees_per_ha(&self, b: Biome) -> f64 {
        self.per_ha(&self.tree, b)
    }

    fn bushes_per_ha(&self, b: Biome) -> f64 {
        self.per_ha(&self.bush, b)
    }
}

fn census(seed: u64) -> Census {
    let table = ScatterTable::alpha_default();
    let haven = terrain::haven(seed);
    let mut c = Census {
        cells: [0; BIOMES],
        tree: [0; BIOMES],
        bush: [0; BIOMES],
    };
    for cz in 0..CELLS_PER_SIDE {
        for cx in 0..CELLS_PER_SIDE {
            let x = cx as f32 * CELL_SIZE + CELL_SIZE * 0.5;
            let z = cz as f32 * CELL_SIZE + CELL_SIZE * 0.5;
            let h = terrain::height(seed, x, z);
            // Sea is not a biome with a density; `biome()` would still
            // answer Beach below the land line and that would dilute the
            // beach's own numbers with open water.
            if h < LAND_MIN_H {
                continue;
            }
            let b = terrain::biome(h, terrain::moisture(seed, x, z)) as usize;
            c.cells[b] += 1;
            match terrain::scatter(seed, &table, &haven, cx, cz).occupant {
                Occupant::Tree => c.tree[b] += 1,
                Occupant::Bush => c.bush[b] += 1,
                _ => {}
            }
        }
    }
    c
}

fn report(seed: u64, c: &Census) {
    println!(
        "seed {seed}: cells [beach {} meadow {} forest {} highland {}]",
        c.cells[0], c.cells[1], c.cells[2], c.cells[3]
    );
    println!(
        "  trees/ha   beach {:.2}  meadow {:.2}  forest {:.2}  highland {:.2}  (contrast {:.3})",
        c.trees_per_ha(Biome::Beach),
        c.trees_per_ha(Biome::Meadow),
        c.trees_per_ha(Biome::Forest),
        c.trees_per_ha(Biome::Highland),
        c.trees_per_ha(Biome::Forest) / c.trees_per_ha(Biome::Meadow),
    );
    println!(
        "  bushes/ha  meadow {:.2}  forest {:.2}  (ratio {:.3})",
        c.bushes_per_ha(Biome::Meadow),
        c.bushes_per_ha(Biome::Forest),
        c.bushes_per_ha(Biome::Forest) / c.bushes_per_ha(Biome::Meadow),
    );
}

/// **Gate 3 — the understory is denser inside the forest than outside it.**
///
/// A forest is not a canopy over bare ground — that is an orchard, and
/// `reference/PLANTS.md` §2 counts the two layers we are missing. The
/// reference bought its forest's thickness with the shrub layer rather than
/// with stems (Devblog 67, *"Made forests appear thicker by adding more
/// bushes"*), which is also where our own budget says to spend: the element
/// count is fixed per tile, so a brush costs no extra draw at all.
///
/// **This measures the CLUTTER layer, and the reason is arithmetic rather
/// than preference.** The obvious place for the understory is the scatter
/// grid beside `Occupant::Bush`, and the grid provably cannot hold one:
/// `tests/scatter.rs::test_no_biome_row_saturates` caps a biome row at
/// `1000 / max(clump)` = **370‰**, the Forest row already spends 350, and its
/// fixed costs leave the bush **70.4‰** — which is exactly the Meadow's bush
/// weight. So on that grid the forest's understory can at best TIE the open
/// field's, and only by spending every per-mille left in the row. The first
/// cut of this gate asserted a scatter-bush ratio and was unsatisfiable at any
/// weight; the ceiling is the finding (`reference/FORESTS.md` §9.1), and
/// `Clutter::Brush` is where the layer went instead.
///
/// Mutant: `BRUSH_SHARE_PERMILLE` at 0 removes the layer and fails this on
/// the first seed; equalising it across channels would fail it too, because
/// the brush is drawn from the forest-litter channel and that is what makes
/// it land in the woods rather than on the lawn.
#[test]
fn the_forest_understory_is_thicker_than_the_meadows() {
    for seed in SEEDS {
        let u = understory(seed);
        u.report(seed);
        let (forest, meadow) = (u.per_ha(Biome::Forest), u.per_ha(Biome::Meadow));
        assert!(
            forest > meadow * UNDERSTORY_RATIO_MIN,
            "seed {seed}: the forest floor carries {forest:.0} brush/ha against the \
             meadow's {meadow:.0} — under {UNDERSTORY_RATIO_MIN}x. The shrub layer is \
             what makes a treeline opaque and moving through woods feel different \
             from crossing a meadow; without it a forest is an orchard."
        );
    }
}

/// The forest's understory must beat the meadow's by this much. **(knob)**
///
/// Not 1.0: a hair over parity would leave the two layers indistinguishable to
/// a player, which is the state this gate exists to refuse. Brush is drawn out
/// of the forest-litter splat channel, so the ratio is really "how much more
/// litter-identity ground does the forest have", and it measures far above
/// this — measured **23.96 / 55.47 / 91.58 / 26.49** across the four seeds —
/// because brush is drawn from the forest-litter splat channel and a meadow
/// has very little of that identity. The floor is set at a fifth of the worst
/// island: the meadow term is the noisy one (34–132 per hectare against the
/// forest's steady ~3,100), so a tight floor here would be a pin on the
/// meadow's litter share rather than a gate on the understory.
const UNDERSTORY_RATIO_MIN: f64 = 5.0;

/// Brush counted over a stride sample of tiles, per biome.
///
/// **Sampled rather than swept, and the arithmetic is why.** The clutter
/// population is `CLUTTER_CELLS_PER_SIDE` = 3,200 on a side — 10.24 M cells,
/// each costing a ground tap — so an island-wide sweep is not a gate, it is a
/// batch job. A stride of `TILE_STRIDE` tiles takes a regular sample across
/// the whole 0..2048 square (still the whole world, per `Census`'s note on
/// `relief.rs`) at a few hundred fills, which is milliseconds and plenty of
/// statistics for a ratio.
struct Understory {
    tiles: [u32; BIOMES],
    brush: [u32; BIOMES],
}

impl Understory {
    fn per_ha(&self, b: Biome) -> f64 {
        let i = b as usize;
        if self.tiles[i] == 0 {
            return 0.0;
        }
        let area_m2 = f64::from(self.tiles[i]) * (CLUTTER_TILE_M * CLUTTER_TILE_M) as f64;
        f64::from(self.brush[i]) / area_m2 * 10_000.0
    }

    fn report(&self, seed: u64) {
        println!(
            "seed {seed}: brush/ha  meadow {:.0}  forest {:.0}  (ratio {:.3})  \
             [tiles meadow {} forest {}]",
            self.per_ha(Biome::Meadow),
            self.per_ha(Biome::Forest),
            self.per_ha(Biome::Forest) / self.per_ha(Biome::Meadow),
            self.tiles[Biome::Meadow as usize],
            self.tiles[Biome::Forest as usize],
        );
    }
}

/// Tiles between samples. Coprime with nothing in particular, but deliberately
/// not a power of two: the scatter grid is 8 m and a tile is 16, so an even
/// stride can sit on one phase of both forever.
const TILE_STRIDE: i32 = 7;

fn understory(seed: u64) -> Understory {
    let haven = terrain::haven(seed);
    let mut buf = [CLUTTER_NONE; CLUTTER_PER_TILE];
    let mut u = Understory {
        tiles: [0; BIOMES],
        brush: [0; BIOMES],
    };
    let tiles_per_side = (CELLS_PER_SIDE as f32 * CELL_SIZE / CLUTTER_TILE_M) as i32;
    let mut tz = 0;
    while tz < tiles_per_side {
        let mut tx = 0;
        while tx < tiles_per_side {
            let x = tx as f32 * CLUTTER_TILE_M + CLUTTER_TILE_M * 0.5;
            let z = tz as f32 * CLUTTER_TILE_M + CLUTTER_TILE_M * 0.5;
            let h = terrain::height(seed, x, z);
            if h >= LAND_MIN_H {
                let b = terrain::biome(h, terrain::moisture(seed, x, z)) as usize;
                u.tiles[b] += 1;
                let n = terrain::clutter_fill(seed, &haven, tx, tz, &mut buf);
                for e in buf.iter().take(n) {
                    if e.kind == Clutter::Brush {
                        u.brush[b] += 1;
                    }
                }
            }
            tx += TILE_STRIDE;
        }
        tz += TILE_STRIDE;
    }
    u
}

/// **Gate 3b — the brush really is the share of the litter channel it claims.**
///
/// The mechanism, not the island: `kind_from_splat` is a pure function of four
/// bytes and a roll, so the share is exactly checkable and does not need a
/// world to measure it. This is the `tests/contour.rs` pattern — gate the law,
/// which is exact and runs in microseconds, rather than a statistic over the
/// terrain, which is neither.
///
/// It also pins the thing that would otherwise be silent: brush comes out of
/// channel 2's *own* interval, so it must not change how often channels 0, 1
/// and 3 win. A sub-draw that leaked would thin the grass with nothing
/// reporting it — proven by moving the split to channel 1, which reddens this
/// and the island gate together.
///
/// ⚠ **This gate reads `BRUSH_SHARE_PERMILLE`, so it cannot see that constant
/// move** — set the share to zero and it stays green, correctly, because the
/// implementation still matches the declaration. Deleting the layer is
/// `the_forest_understory_is_thicker_than_the_meadows`'s to catch, and it
/// does. Two gates, two questions: *is the code what the knob says* and *is
/// the knob worth anything*.
#[test]
fn the_brush_is_a_split_of_the_litter_channel_and_nothing_else() {
    // A pure forest-litter ground: every draw lands in channel 2.
    let litter = [0u8, 0, 255, 0];
    let (mut brush, mut twig) = (0u32, 0u32);
    for roll in 0..255u64 {
        match terrain::kind_from_splat(litter, roll) {
            Clutter::Brush => brush += 1,
            Clutter::Twig => twig += 1,
            other => panic!("pure litter ground drew {other:?}"),
        }
    }
    let share = f64::from(brush) / f64::from(brush + twig) * 1000.0;
    println!("pure-litter ground: brush {brush} twig {twig} = {share:.1} per-mille");
    // One roll of slack each way: the interval is integer arithmetic over 255
    // steps, so the achievable share is quantised to about 4 per-mille.
    let want = f64::from(BRUSH_SHARE_PERMILLE);
    assert!(
        share > want - 8.0 && share < want + 8.0,
        "brush is {share:.1} per-mille of the litter channel, not the \
         {BRUSH_SHARE_PERMILLE} `BRUSH_SHARE_PERMILLE` declares"
    );

    // The other three channels are untouched by the split.
    for (ch, want) in [
        (0usize, Clutter::Pebble),
        (1, Clutter::Tuft),
        (3, Clutter::Shard),
    ] {
        let mut w = [0u8; 4];
        w[ch] = 255;
        for roll in 0..255u64 {
            assert_eq!(
                terrain::kind_from_splat(w, roll),
                want,
                "channel {ch} drew something other than {want:?} — the brush split \
                 has leaked out of channel 2"
            );
        }
    }
}

/// **Gate 2 — a forest is distinguishable from a field by walking into it.**
///
/// This is the assertion `tests/scatter.rs` structurally cannot make. Its
/// live-slot band is island-wide, so a change that moved stems from the
/// forest into the meadow keeps the total inside 8,000–12,000 and passes,
/// while erasing the only thing that makes the two biomes different places.
/// A player learns a biome at its boundary, so the gate is on the *step*
/// across it.
///
/// Measured well below the 260 : 70 = 3.71 the weight table reads, and that
/// gap is correct rather than a discrepancy: `scatter` draws against
/// `splat_from`'s soft blend of four channels, not against a hard biome
/// pick, so boundary cells carry a mixture by design
/// (`test_scatter_mix_ramps_where_it_used_to_step`). What this gate holds is
/// the density a player actually meets, which is the blended one.
///
/// Mutant: flattening the two rows to a common tree weight (both 165‰) holds
/// every island-wide gate green and fails this on the first seed.
#[test]
fn the_forest_is_a_different_place_from_the_meadow() {
    for seed in SEEDS {
        let c = census(seed);
        report(seed, &c);
        let (forest, meadow) = (c.trees_per_ha(Biome::Forest), c.trees_per_ha(Biome::Meadow));
        assert!(
            forest > meadow * FOREST_CONTRAST_MIN,
            "seed {seed}: forest {forest:.2} stems/ha against meadow {meadow:.2} is a \
             contrast of {:.3}, under {FOREST_CONTRAST_MIN} — the two biomes have \
             stopped being different places. Island-wide gates cannot see this: \
             moving stems from one biome to the other keeps every total intact.",
            forest / meadow
        );
    }
}

/// How many times denser the forest's canopy must be than the meadow's.
/// **(knob)**
///
/// Measured **7.607 / 7.245 / 7.985 / 7.045** at forest density v1
/// (2026-09-14) against the table's own 640 : 70 = 9.14, the shortfall
/// being the splat blend described above — it was 3.482 / 3.454 / 3.565 /
/// 3.211 against 260 : 70 before, and the floor was 2.5. 5.0 leaves 29 %
/// under the worst island and is well clear of a flattened table's 1.0,
/// which is the pair of cases it has to separate.
const FOREST_CONTRAST_MIN: f64 = 5.0;

/// **Gate 1's sim half — each biome's stem density holds a stated band.**
///
/// A contrast ratio alone is satisfiable by making the *meadow* emptier, so
/// the ratio needs an anchor underneath it. This is that anchor, and it is
/// two-sided on purpose: the floor catches a forest quietly thinning, and
/// the ceiling catches a density rise that outran the frame's count cap
/// (`reference/FORESTS.md` §8 gate 7 — built 2026-09-14 as
/// `client::render::tree::TREE_LOD_CAP`, the same slice that raised this
/// floor; `client/tests/tree_cap.rs` is its gate and the cap is sized off
/// the density this band admits).
///
/// **The floor is the operator's call, 2026-09-14** (*"when can we get
/// forest fr?"*). It was ~39 stems/ha, which `reference/FORESTS.md` §1.1
/// prices at under 7 % canopy cover — below the 10 % that makes the word
/// "forest" true — and it held there because the saturation rail capped
/// the Forest row at 370‰ (`ScatterTable::clump_cap` is the way past it).
/// Measured **96.29 / 92.91 / 94.70 / 93.02** after; the grid's own ceiling
/// is 156.25/ha, and the rail admits ~110/ha for the tree entry alone, so
/// the ceiling below is the most the shipped field can hand a row before
/// something structural (`CELL_SIZE`, `CLUMP_FLOOR`) moves.
///
/// Mutant: dropping Forest's tree weight 640 → 520‰ (~76/ha) fails the
/// floor; the ceiling cannot be reached by a weight (the const block in
/// `terrain.rs` refuses a row that saturates), so its mutant is the field:
/// `CLUMP_FLOOR` 0.15 → 0.6 lifts the capped mean and the normalizer with
/// it, and this reads the rise. Both keep island-wide live slots in band.
#[test]
fn each_biomes_density_holds_its_band() {
    for seed in SEEDS {
        let c = census(seed);
        let forest = c.trees_per_ha(Biome::Forest);
        assert!(
            (FOREST_STEMS_MIN..=FOREST_STEMS_MAX).contains(&forest),
            "seed {seed}: forest is {forest:.2} stems/ha, outside \
             {FOREST_STEMS_MIN}..={FOREST_STEMS_MAX}. The grid's hard ceiling is \
             {:.2}/ha (one occupant per {CELL_AREA_M2} m²) — if this is high, the \
             LOD count cap owes a slice first (`reference/FORESTS.md` §8 gate 7).",
            10_000.0 / CELL_AREA_M2
        );
        // The beach must stay a beach — the spawn zone's sightlines read on
        // it. **A ceiling and not zero, and the difference is a finding.**
        // `alpha_default` gives Beach a tree weight of *nought*, and the
        // first draft of this gate asserted the count was zero on that
        // reading. It is not: the beach carries ~4.8 stems/ha, because
        // `biome()` hard-classifies a cell while `scatter` draws against
        // `splat_from`'s soft blend of four channels, so a beach cell near
        // the meadow line draws part of the meadow's row. The two functions
        // answer different questions and a zero in the table is not a zero
        // on the ground.
        let beach = c.trees_per_ha(Biome::Beach);
        assert!(
            beach < c.trees_per_ha(Biome::Meadow),
            "seed {seed}: the beach carries {beach:.2} stems/ha, at or above the \
             meadow's — it is a blend of the neighbouring row and must stay the \
             thinnest land on the island."
        );
    }
}

/// The band the forest's canopy density must sit in, stems per hectare.
/// **(knob)**
///
/// Measured **92.91–96.29** on the four seeds when the band was set at
/// forest density v1 (37.07–38.31 when it was 32–80). The floor is slack
/// enough for an atypical island and tight enough that losing a fifth of
/// the forest reddens it; the ceiling is `reference/FORESTS.md` §8 gate 7's
/// tripwire rather than a design target — the client's count cap is sized
/// against this band, so a forest past it is a forest the cap was not
/// measured for.
const FOREST_STEMS_MIN: f64 = 80.0;
const FOREST_STEMS_MAX: f64 = 120.0;

// ── World structure v1: the edge, the species paint, and the stand count ───
//
// `reference/FORESTS.md` §8's gates 4 and 6 were both written UNWRITABLE —
// each named a mechanism that had to exist first, which is the point of
// naming them. Both mechanisms landed in world structure v1, so both gates
// are written here, and the third below is the one the moisture change is
// actually for: a forest that is *many* woods rather than one continent is a
// property of the SET of forest cells, and no per-cell or island-wide number
// can see it.

/// How close to the middle of the grass/litter transition a cell has to be to
/// count as "on the treeline" for this file. Read off the splat exactly as
/// `terrain::edge_factor` does — one definition of where a forest stops, per
/// that function's own doc.
fn edge_of(seed: u64, haven: &terrain::Haven, x: f32, z: f32) -> f32 {
    let h = terrain::ground(seed, haven, x, z);
    let w = terrain::splat_from(
        h,
        terrain::moisture(seed, x, z),
        terrain::ground_slope(seed, haven, x, z),
    );
    let (a, b) = (f64::from(w[1]), f64::from(w[2]));
    if a + b <= 0.0 {
        return 0.0;
    }
    (4.0 * a * b / ((a + b) * (a + b))) as f32
}

/// **Gate 4 — the edge exists** (`reference/FORESTS.md` §8).
///
/// "Cells within N m of a forest/meadow boundary carry a different occupant
/// mix from forest-core cells." Their `Forestside` is a separate plant list
/// of small trees and bushes; ours is a transfer inside the weight row
/// (`terrain::EDGE_TREE_TO_BUSH`), so what must show is a mix that has moved
/// AWAY from trees and TOWARD bushes while the ground is still forest ground.
///
/// Mutant, run: `EDGE_TREE_TO_BUSH = 0.0`. The treeline still thins on its
/// own, because the splat blend already mixes the meadow's row in — 53
/// stems/ha against the core's 94, **1.76x** — and that is exactly why the
/// floor is 2.5 and not 1.2. The bush half moves further: 7/ha against the
/// core's 3 without the transfer, against 32 and 4 with it. Swapping
/// `ROW_TREE`/`ROW_BUSH` at the transfer site inverts both ratios and the
/// tree floor catches that one.
#[test]
fn the_treeline_is_scrub_and_the_forest_core_is_not() {
    for seed in SEEDS {
        let haven = terrain::haven(seed);
        let (mut e_cells, mut e_tree, mut e_bush) = (0u32, 0u32, 0u32);
        let (mut c_cells, mut c_tree, mut c_bush) = (0u32, 0u32, 0u32);
        for cz in 0..CELLS_PER_SIDE {
            for cx in 0..CELLS_PER_SIDE {
                let x = cx as f32 * CELL_SIZE + CELL_SIZE * 0.5;
                let z = cz as f32 * CELL_SIZE + CELL_SIZE * 0.5;
                if terrain::ground(seed, &haven, x, z) < LAND_MIN_H {
                    continue;
                }
                let occ =
                    terrain::scatter(seed, &ScatterTable::alpha_default(), &haven, cx, cz).occupant;
                let e = edge_of(seed, &haven, x, z);
                // Core: the litter channel owns the cell outright. Edge: the
                // two channels are within a factor of ~3 of each other.
                let w = terrain::splat_from(
                    terrain::ground(seed, &haven, x, z),
                    terrain::moisture(seed, x, z),
                    terrain::ground_slope(seed, &haven, x, z),
                );
                if e >= 0.5 && w[2] > 0 {
                    e_cells += 1;
                    match occ {
                        Occupant::Tree => e_tree += 1,
                        Occupant::Bush => e_bush += 1,
                        _ => {}
                    }
                } else if u32::from(w[2]) > u32::from(w[1]) * 4 {
                    c_cells += 1;
                    match occ {
                        Occupant::Tree => c_tree += 1,
                        Occupant::Bush => c_bush += 1,
                        _ => {}
                    }
                }
            }
        }
        assert!(
            e_cells > 200 && c_cells > 200,
            "seed {seed}: {e_cells} treeline / {c_cells} core cells — one \
             population is too small for the comparison below to mean anything"
        );
        let per = |n: u32, d: u32| f64::from(n) / f64::from(d) * (10_000.0 / CELL_AREA_M2);
        let (et, eb) = (per(e_tree, e_cells), per(e_bush, e_cells));
        let (ct, cb) = (per(c_tree, c_cells), per(c_bush, c_cells));
        println!(
            "seed {seed}: treeline {e_cells} cells — {et:.0} tree/ha {eb:.0} bush/ha | \
             core {c_cells} cells — {ct:.0} tree/ha {cb:.0} bush/ha"
        );
        assert!(
            ct / et >= EDGE_THINNING_MIN,
            "seed {seed}: the treeline carries {et:.0} stems/ha against the \
             core's {ct:.0} — only {:.2}x thinner, under the \
             {EDGE_THINNING_MIN}x floor. A border that draws the core's own \
             mix is a density gradient, not a treeline \
             (reference/FORESTS.md §3.1).",
            ct / et
        );
        assert!(
            eb / cb.max(0.01) >= EDGE_SCRUB_MIN,
            "seed {seed}: the treeline carries {eb:.0} bushes/ha against the \
             core's {cb:.0} — only {:.2}x, under the {EDGE_SCRUB_MIN}x floor. \
             The transfer is supposed to put INTO the border what it takes \
             out; thinning alone is a clearing.",
            eb / cb.max(0.01)
        );
        assert!(
            et > 0.0,
            "seed {seed}: the treeline carries no trees at all — `Forestside` \
             is small trees AND bushes, and a bare ring around every wood is a \
             different defect from the one this replaced"
        );
    }
}

/// Floor on core-to-edge stem thinning. Measured **3.10–3.44x** on the four
/// seeds at `EDGE_TREE_TO_BUSH = 0.62`; floored ~20% under the worst so
/// coastline and moisture variance cannot trip it, and proven red with the
/// transfer at 0.
const EDGE_THINNING_MIN: f64 = 2.5;
/// Floor on edge-to-core bush enrichment, same derivation. Measured
/// **7.0–8.4x**, which is the larger of the two effects because the core's
/// bush density is small.
const EDGE_SCRUB_MIN: f64 = 4.0;

/// **Gate 6 — species distribution** (`reference/FORESTS.md` §8).
///
/// That gate was impossible while species was `slot.yaw % pool` on the
/// client: "no `sim-core` test can assert anything about species", and naming
/// the precondition was the deliverable. `Slot::species` is the precondition,
/// met — so this is the gate, and it is one assert exactly as §8 predicted.
///
/// The claim is not "a draw happened" but **"a region is dominated"**, which
/// is §3.2's `Alt`: where the field says one species, the trees standing
/// there are mostly that species, and the minority is still present.
///
/// Mutants: draw species from a bare hash bit instead of the field (both
/// shares collapse to ~50% and both floors go red), or invert the comparison
/// (the two shares swap and both go red).
#[test]
fn a_species_region_is_dominated_and_not_pure() {
    for seed in SEEDS {
        let haven = terrain::haven(seed);
        // [species 0, species 1] counted where the field is at each rail.
        let mut lo = [0u32; 2];
        let mut hi = [0u32; 2];
        let mut seen = [0u32; 2];
        for cz in 0..CELLS_PER_SIDE {
            for cx in 0..CELLS_PER_SIDE {
                let s = terrain::scatter(seed, &ScatterTable::alpha_default(), &haven, cx, cz);
                if s.occupant != Occupant::Tree {
                    continue;
                }
                assert!(
                    (s.species as usize) < terrain::SLOT_SPECIES as usize,
                    "seed {seed}: slot species {} is outside 0..{}",
                    s.species,
                    terrain::SLOT_SPECIES
                );
                seen[s.species as usize] += 1;
                let share = terrain::species_share(seed, s.x, s.z);
                if share < SPECIES_RAIL_LO {
                    lo[s.species as usize] += 1;
                } else if share > SPECIES_RAIL_HI {
                    hi[s.species as usize] += 1;
                }
            }
        }
        let lo_n = lo[0] + lo[1];
        let hi_n = hi[0] + hi[1];
        assert!(
            lo_n > 100 && hi_n > 100,
            "seed {seed}: {lo_n} trees where the field rails low and {hi_n} \
             where it rails high — the field spends too little of the island \
             at its own ends for this to be a measurement of anything"
        );
        let dom_lo = f64::from(lo[0]) / f64::from(lo_n);
        let dom_hi = f64::from(hi[1]) / f64::from(hi_n);
        println!(
            "seed {seed}: species {seen:?} islandwide; share<{SPECIES_RAIL_LO} -> \
             {:.0}% species 0; share>{SPECIES_RAIL_HI} -> {:.0}% species 1",
            dom_lo * 100.0,
            dom_hi * 100.0
        );
        assert!(
            dom_lo >= SPECIES_DOMINANCE_MIN && dom_hi >= SPECIES_DOMINANCE_MIN,
            "seed {seed}: a region the field calls species 0 is only \
             {:.0}% species 0 and one it calls species 1 only {:.0}% — under \
             the {:.0}% floor the paint is not painting, and a forest cannot \
             have a birch stand you can walk out of.",
            dom_lo * 100.0,
            dom_hi * 100.0,
            SPECIES_DOMINANCE_MIN * 100.0
        );
        assert!(
            dom_lo <= SPECIES_DOMINANCE_MAX && dom_hi <= SPECIES_DOMINANCE_MAX,
            "seed {seed}: a railed region is {:.0}%/{:.0}% pure, over the \
             {:.0}% ceiling — the draw has stopped being a draw and become a \
             paint, which is the hard-edged `Alt` region §3.2 says theirs is \
             NOT.",
            dom_lo * 100.0,
            dom_hi * 100.0,
            SPECIES_DOMINANCE_MAX * 100.0
        );
        assert!(
            seen[0] > 0 && seen[1] > 0,
            "seed {seed}: species {seen:?} — an island with one species on it"
        );
    }
}

/// Where the species field counts as railed. Not 0 and 1: `species_share`
/// clamps, so its extremes are reached over real area, and a rail this test
/// can find has to be inside them.
const SPECIES_RAIL_LO: f32 = 0.15;
const SPECIES_RAIL_HI: f32 = 0.85;
/// Floor on how dominant a railed region's own species is. Measured
/// **90–95%** across the four seeds; floored well under so field variance
/// cannot trip it, and red at ~50% under a bare-hash draw.
const SPECIES_DOMINANCE_MIN: f64 = 0.75;
/// Ceiling on the same, so the draw stays a draw. A region that is 100% one
/// species is a stencil, and the minority IS the mechanic
/// (`reference/FORESTS.md` §3.2).
const SPECIES_DOMINANCE_MAX: f64 = 0.99;

/// **The forest is many woods, not one continent** — the property the
/// moisture scale exists for, and the one every other gate in this file is
/// blind to.
///
/// `each_biomes_density_holds_its_band` and `the_forest_is_a_different_place_
/// from_the_meadow` are both satisfied by a forest that is a single
/// island-spanning mass, because a density and a contrast are averages over
/// a SET and say nothing about its shape. Measured before world structure v1
/// (`examples/biome_map`, three seeds): **3–6 connected patches, with 63–98%
/// of all forest inside one of them**. That is a continent with a tree on it.
///
/// Mutants: `MOIST_FREQ` back to 1/700 (patches fall to 3–7 and the largest
/// share goes over 60%), or `MOIST_OCTAVES` to 1.
#[test]
fn the_forest_is_woods_rather_than_one_continent() {
    for seed in SEEDS {
        let haven = terrain::haven(seed);
        let n = (CELLS_PER_SIDE * CELLS_PER_SIDE) as usize;
        let mut is_forest = vec![false; n];
        let mut total = 0u32;
        for cz in 0..CELLS_PER_SIDE {
            for cx in 0..CELLS_PER_SIDE {
                let x = cx as f32 * CELL_SIZE + CELL_SIZE * 0.5;
                let z = cz as f32 * CELL_SIZE + CELL_SIZE * 0.5;
                let h = terrain::ground(seed, &haven, x, z);
                if h < LAND_MIN_H {
                    continue;
                }
                if terrain::biome(h, terrain::moisture(seed, x, z)) == Biome::Forest {
                    is_forest[(cz * CELLS_PER_SIDE + cx) as usize] = true;
                    total += 1;
                }
            }
        }
        // Four-neighbour flood fill. A wood is a place you can walk across
        // without leaving it, which is what connectivity means here.
        let mut seen = vec![false; n];
        let mut patches: Vec<u32> = Vec::new();
        let mut stack: Vec<i32> = Vec::new();
        for start in 0..n {
            if !is_forest[start] || seen[start] {
                continue;
            }
            let mut size = 0u32;
            seen[start] = true;
            stack.push(start as i32);
            while let Some(p) = stack.pop() {
                size += 1;
                let (cx, cz) = (p % CELLS_PER_SIDE, p / CELLS_PER_SIDE);
                for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                    let (jx, jz) = (cx + dx, cz + dz);
                    if jx < 0 || jz < 0 || jx >= CELLS_PER_SIDE || jz >= CELLS_PER_SIDE {
                        continue;
                    }
                    let j = (jz * CELLS_PER_SIDE + jx) as usize;
                    if is_forest[j] && !seen[j] {
                        seen[j] = true;
                        stack.push(j as i32);
                    }
                }
            }
            patches.push(size);
        }
        // Copses of a cell or two are noise on the classifier's contour, not
        // woods; count the ones big enough to stand in.
        let woods = patches.iter().filter(|&&s| s >= WOOD_MIN_CELLS).count();
        let largest = patches.iter().copied().max().unwrap_or(0);
        let share = f64::from(largest) / f64::from(total.max(1));
        println!(
            "seed {seed}: {total} forest cells in {} patches — {woods} of them \
             >= {WOOD_MIN_CELLS} cells; largest {largest} ({:.0}% of all forest)",
            patches.len(),
            share * 100.0
        );
        assert!(
            woods >= WOODS_MIN,
            "seed {seed}: only {woods} woods of {WOOD_MIN_CELLS}+ cells — \
             under the {WOODS_MIN} floor the Forest biome is a continent, and \
             every density gate in this file is happy with that"
        );
        assert!(
            share <= LARGEST_WOOD_MAX,
            "seed {seed}: {:.0}% of all forest is inside ONE patch, over the \
             {:.0}% ceiling. A player who walks out of that one has left the \
             forest for good.",
            share * 100.0,
            LARGEST_WOOD_MAX * 100.0
        );
    }
}

/// Smallest patch that counts as a wood rather than a speck on the
/// classifier's contour: 24 cells is ~0.15 ha, about 40 m across.
const WOOD_MIN_CELLS: u32 = 24;
/// Floor on how many woods an island has. Measured **12 / 16 / 29 / 17** on
/// the four gate seeds at `MOIST_FREQ = 1/240`, 3 octaves. Floored a third
/// under the worst of those, because seed 0 is the atypical island this
/// file's header warns about and a band that only just holds it is a pin.
const WOODS_MIN: usize = 8;
/// Ceiling on the share of all forest inside one patch. Measured
/// **0.54 / 0.38 / 0.16 / 0.22** on the same seeds.
///
/// ⚠ **Both of these bands are wider than the measurement, on purpose, and
/// the mutant is what makes them gates rather than decoration.** Run at the
/// 1/700 × 2-octave field they replaced, the same four seeds read **4 / 3 /
/// 3 / 2 woods** and **0.98 / 0.69 / 0.99 / 0.97** of all forest inside one
/// patch — every seed red on both halves, by a factor rather than a margin.
/// A band fitted to the current numbers would have caught that too, and
/// would then have gone red on the next island that happened to be seed 0,
/// which sits at 12 woods and 0.54.
const LARGEST_WOOD_MAX: f64 = 0.70;
