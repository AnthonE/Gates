//! The lattice memo, and the two clutter refusals it landed beside.
//!
//! Everything in this pass was an OPTIMIZATION — it was allowed to change how
//! long the island takes to resolve and forbidden to change the island. That
//! is a bit-level claim, and `CLAUDE.md` is explicit that clutter is the one
//! surface where nothing supplies the evidence for free: `clutter_fill` is not
//! in `state_hash`, not in `test_terrain_golden`, not on the wasm parity
//! diff, and `tests/clutter.rs`'s determinism check compares the fill against
//! *itself*. Self-consistency is not identity with the previous build.
//!
//! So this suite is the evidence, in the shape `client/tests/ground.rs`
//! already uses for the mesh: a NAIVE rebuild written out in full here,
//! compared to the shipped path field by field on `to_bits()`. Four claims:
//!
//!  1. `Lattice` and the direct draw agree on `height` everywhere, including
//!     with several seeds interleaved through one table (the memo is keyed on
//!     the seed by an epoch, and the first draft of it was not — it returned
//!     another world's gradients and every value was plausible).
//!  2. The same, for every public `*_memo` twin.
//!  3. `clutter_fill` and `skirt_fill` emit exactly the element sequence the
//!     un-optimized law emits — the rich-roll early-out, the shared splat,
//!     the hoisted tile-ownership test and the tile range check are all
//!     refusals of work and none of them is a refusal of an element.
//!  4. `clutter_richness_at` never exceeds `RICH_ACCEPT_MAX` over the island.
//!     That is the *premise* of claim 3's early-out rather than a consequence
//!     of it: the early-out is exact because a roll at or above the ceiling
//!     was always going to lose, and if this bound ever moved the refusal
//!     would start eating elements silently.
//!
//! **Every claim was watched going red under its own mutant**, and the list is
//! written out because two of the mutants were run and did NOT redden a first
//! draft of this file:
//!
//! | mutant | result |
//! |---|---|
//! | the memo slot forgets its epoch | RED (1, 2) |
//! | the early-out threshold one too low | RED (the early-out test) |
//! | the shared road check dropped from the rich path | RED |
//! | the kind draw reads the wrong bits (`>> 1`) | RED |
//! | the shared splat resolved 0.1 m off the cell's own point | RED |
//! | the hoisted skirt ownership edge moved 0.5 m | RED |
//! | the hoisted tile range check drops tile 0 | GREEN — see below |
//!
//! The first draft was green under the second and fourth of those, and the
//! reason is worth more than the fix: its "naive" rebuild called
//! `clutter_rich_cell` for the per-cell law, so both sides of the comparison
//! carried the mutant. A rebuild that shares the function under test is a
//! rebuild of nothing. `refused_by_the_law` and `kind_by_the_law` are written
//! from the published parts instead — which is why `clutter_rich_draw`,
//! `clutter_kind_at` and `clutter_richness_at` are `pub`.
//!
//! The last row is an EQUIVALENT mutant on this island rather than a hole:
//! `CLUTTER_CELLS_PER_SIDE` is an exact multiple of `CLUTTER_CELLS_PER_TILE`,
//! so no tile straddles the edge, and every tile the range check could wrongly
//! drop is ocean — the island's radius is 960 m inside a 2,048 m square, so
//! tile 0 has never held an element. `the_tile_grid_divides_the_cell_grid`
//! pins the premise that makes it equivalent, because if that ever stopped
//! being true the hoist would start dropping live cells silently.

use sim_core::terrain::{
    self, Clutter, ClutterElem, Haven, Lattice, ScatterTable, CLUTTER_CELLS_PER_TILE,
    CLUTTER_CELL_M, CLUTTER_NONE, CLUTTER_PER_TILE, CLUTTER_RICH_PER_TILE, CLUTTER_TILE_M,
    LAND_MIN_H, RICH_ACCEPT_MAX, SKIRT_PER_TILE, SKIRT_TILE_CELLS,
};

/// The shipped seed, plus two that disagree with it about where the land is.
const SEEDS: [u64; 3] = [20_260_731, 1, 0xDEAD_BEEF];

/// A span that crosses the coast twice on every seed: the island runs 0..2048
/// and this walks it corner to corner with a stride that is not a divisor of
/// any lattice period, so no octave is sampled on its own grid.
fn sweep() -> impl Iterator<Item = (f32, f32)> {
    (0..260)
        .flat_map(|i| (0..260).map(move |j| (-120.0 + i as f32 * 8.317, -120.0 + j as f32 * 8.317)))
}

fn hv(seed: u64) -> Haven {
    terrain::haven(seed)
}

// ── 1. The memo is the draw ────────────────────────────────────────────────

#[test]
fn the_memo_and_the_direct_draw_agree_on_height() {
    let mut lat = Lattice::new();
    let mut n = 0u64;
    for (x, z) in sweep() {
        for s in SEEDS {
            let a = terrain::height(s, x, z);
            let b = terrain::height_memo(&mut lat, s, x, z);
            assert_eq!(
                a.to_bits(),
                b.to_bits(),
                "height({s:#x}, {x}, {z}): direct {a} memo {b}"
            );
            n += 1;
        }
    }
    assert!(n > 200_000, "the sweep got smaller: {n} samples");
}

/// The seed key, on its own. Three worlds through ONE table, alternating on
/// every single call so no slot survives two samples of the same seed — the
/// arrangement a per-caller memo would never see and a shared one always
/// would. Without the epoch this is the test that goes red.
#[test]
fn one_table_serves_three_seeds_without_mixing_them() {
    let mut lat = Lattice::new();
    let mut worst = 0u32;
    for (x, z) in sweep().take(20_000) {
        for s in SEEDS {
            let a = terrain::height(s, x, z).to_bits();
            let b = terrain::height_memo(&mut lat, s, x, z).to_bits();
            worst |= a ^ b;
        }
    }
    assert_eq!(worst, 0, "a seed read another seed's gradients");
}

#[test]
fn every_memo_twin_agrees_with_its_plain_form() {
    let table = ScatterTable::alpha_default();
    for s in SEEDS {
        let haven = hv(s);
        let mut lat = Lattice::new();
        for (x, z) in sweep().take(9_000) {
            assert_eq!(
                terrain::ground(s, &haven, x, z).to_bits(),
                terrain::ground_memo(&mut lat, s, &haven, x, z).to_bits(),
                "ground at ({x}, {z})"
            );
            assert_eq!(
                terrain::ground_slope(s, &haven, x, z).to_bits(),
                terrain::ground_slope_memo(&mut lat, s, &haven, x, z).to_bits(),
                "ground_slope at ({x}, {z})"
            );
            assert_eq!(
                terrain::slope(s, x, z).to_bits(),
                terrain::slope_memo(&mut lat, s, x, z).to_bits(),
                "slope at ({x}, {z})"
            );
            assert_eq!(
                terrain::moisture(s, x, z).to_bits(),
                terrain::moisture_memo(&mut lat, s, x, z).to_bits(),
                "moisture at ({x}, {z})"
            );
            assert_eq!(
                terrain::clump(s, x, z).to_bits(),
                terrain::clump_memo(&mut lat, s, x, z).to_bits(),
                "clump at ({x}, {z})"
            );
            assert_eq!(
                terrain::splat(s, x, z),
                terrain::splat_memo(&mut lat, s, x, z),
                "splat at ({x}, {z})"
            );
            assert_eq!(
                terrain::road_band(s, x, z),
                terrain::road_band_memo(&mut lat, s, x, z),
                "road_band at ({x}, {z})"
            );
        }
        // Scatter over its own grid rather than the sweep's metres.
        for c in 0..12_000i32 {
            let (cx, cz) = (c % 200, c / 200);
            let a = terrain::scatter(s, &table, &haven, cx, cz);
            let b = terrain::scatter_memo(&mut lat, s, &table, &haven, cx, cz);
            assert_eq!(a.occupant, b.occupant, "scatter occupant at ({cx}, {cz})");
            assert_eq!(a.x.to_bits(), b.x.to_bits(), "scatter x at ({cx}, {cz})");
            assert_eq!(a.y.to_bits(), b.y.to_bits(), "scatter y at ({cx}, {cz})");
            assert_eq!(a.z.to_bits(), b.z.to_bits(), "scatter z at ({cx}, {cz})");
            assert_eq!(a.yaw, b.yaw, "scatter yaw at ({cx}, {cz})");
            assert_eq!(
                a.scale.to_bits(),
                b.scale.to_bits(),
                "scatter scale at ({cx}, {cz})"
            );
        }
    }
}

// ── 2. The clutter refusals refuse work and never an element ───────────────

/// The identity of the element under comparison, passed as values rather than
/// as a built string: `sim-core`'s clippy wall disallows `format!` in this
/// crate and its tests are inside it, so an assertion message is written with
/// the assert macro's own arguments — `tests/height_roles.rs` states the same
/// rule about its own patterns.
#[derive(Clone, Copy)]
struct At {
    seed: u64,
    tile: (i32, i32),
    ix: usize,
    /// "grid" or "skirt", so a failure says which population it came from.
    layer: &'static str,
}

fn same_elem(a: &ClutterElem, b: &ClutterElem, at: At) {
    let (seed, (tx, tz), k, layer) = (at.seed, at.tile, at.ix, at.layer);
    assert_eq!(
        a.kind, b.kind,
        "{layer} seed {seed:#x} tile ({tx}, {tz}) element {k}: kind"
    );
    assert_eq!(
        a.x.to_bits(),
        b.x.to_bits(),
        "{layer} seed {seed:#x} tile ({tx}, {tz}) element {k}: x"
    );
    assert_eq!(
        a.y.to_bits(),
        b.y.to_bits(),
        "{layer} seed {seed:#x} tile ({tx}, {tz}) element {k}: y"
    );
    assert_eq!(
        a.z.to_bits(),
        b.z.to_bits(),
        "{layer} seed {seed:#x} tile ({tx}, {tz}) element {k}: z"
    );
    assert_eq!(
        a.yaw, b.yaw,
        "{layer} seed {seed:#x} tile ({tx}, {tz}) element {k}: yaw"
    );
    assert_eq!(
        a.scale.to_bits(),
        b.scale.to_bits(),
        "{layer} seed {seed:#x} tile ({tx}, {tz}) element {k}: scale"
    );
}

/// `clutter_fill`'s law written out the long way: every cell, both strata,
/// through the PUBLIC per-cell entry points, in the emission order the fill
/// promises. Nothing here is allowed to know about the early-out.
fn naive_clutter_fill(
    seed: u64,
    haven: &Haven,
    tile_x: i32,
    tile_z: i32,
    out: &mut Vec<ClutterElem>,
) {
    out.clear();
    let cx0 = tile_x * CLUTTER_CELLS_PER_TILE;
    let cz0 = tile_z * CLUTTER_CELLS_PER_TILE;
    let mut rich = 0usize;
    for j in 0..CLUTTER_CELLS_PER_TILE {
        for i in 0..CLUTTER_CELLS_PER_TILE {
            if out.len() >= CLUTTER_PER_TILE {
                return;
            }
            let e = terrain::clutter_cell(seed, haven, cx0 + i, cz0 + j);
            if e.kind != Clutter::None {
                out.push(e);
            }
            if rich >= CLUTTER_RICH_PER_TILE || out.len() >= CLUTTER_PER_TILE {
                continue;
            }
            let r = terrain::clutter_rich_cell(seed, haven, cx0 + i, cz0 + j);
            if r.kind != Clutter::None {
                out.push(r);
                rich += 1;
            }
        }
    }
}

/// `skirt_fill`'s law the long way — and deliberately in the OLD order: build
/// the whole element with `skirt_elem`, then test the kind, then test tile
/// ownership. The shipped path asks where before it asks what, so if the two
/// orders ever disagreed about an element this is what would say so.
fn naive_skirt_fill(
    seed: u64,
    table: &ScatterTable,
    haven: &Haven,
    tile_x: i32,
    tile_z: i32,
    out: &mut Vec<ClutterElem>,
) {
    out.clear();
    let x0 = tile_x as f32 * CLUTTER_TILE_M;
    let z0 = tile_z as f32 * CLUTTER_TILE_M;
    let x1 = x0 + CLUTTER_TILE_M;
    let z1 = z0 + CLUTTER_TILE_M;
    let c0x = tile_x * SKIRT_TILE_CELLS - 1;
    let c0z = tile_z * SKIRT_TILE_CELLS - 1;
    for dz in 0..terrain::SKIRT_SCAN_CELLS {
        for dx in 0..terrain::SKIRT_SCAN_CELLS {
            let cx = c0x + dx;
            let cz = c0z + dz;
            let slot = terrain::scatter(seed, table, haven, cx, cz);
            let count = terrain::skirt_count(slot.occupant);
            for i in 0..count {
                if out.len() >= SKIRT_PER_TILE {
                    return;
                }
                let e = terrain::skirt_elem(seed, haven, cx, cz, &slot, i, count);
                if e.kind == Clutter::None {
                    continue;
                }
                if e.x < x0 || e.x >= x1 || e.z < z0 || e.z >= z1 {
                    continue;
                }
                out.push(e);
            }
        }
    }
}

/// Tiles worth walking: a block around each seed's own pad (where the sweep,
/// the carve and the road all fire at once), plus a block at the island
/// centre, plus a strip that runs off the edge into the sea and past it.
fn tiles_of_interest(haven: &Haven) -> Vec<(i32, i32)> {
    let mut v = Vec::new();
    let px = (haven.x / CLUTTER_TILE_M) as i32;
    let pz = (haven.z / CLUTTER_TILE_M) as i32;
    for dz in -3..=3 {
        for dx in -3..=3 {
            v.push((px + dx, pz + dz));
        }
    }
    let c = (1024.0 / CLUTTER_TILE_M) as i32;
    for dz in -2..=2 {
        for dx in -2..=2 {
            v.push((c + dx, c + dz));
        }
    }
    // The coast, and then off the map entirely.
    for k in 0..24 {
        v.push((k * 6, c));
        v.push((c, k * 6));
    }
    v.push((-1, c));
    v.push((-40, -40));
    v.push((terrain::CLUTTER_CELLS_PER_SIDE / CLUTTER_CELLS_PER_TILE, c));
    v.push((9_000, 9_000));
    v
}

#[test]
fn the_fill_emits_exactly_what_the_unoptimized_law_emits() {
    let mut fast = vec![CLUTTER_NONE; CLUTTER_PER_TILE];
    let mut slow: Vec<ClutterElem> = Vec::new();
    let mut checked = 0u64;
    let mut elements = 0u64;
    for s in SEEDS {
        let haven = hv(s);
        let mut lat = Lattice::new();
        for (tx, tz) in tiles_of_interest(&haven) {
            let n = terrain::clutter_fill_memo(&mut lat, s, &haven, tx, tz, &mut fast);
            naive_clutter_fill(s, &haven, tx, tz, &mut slow);
            assert_eq!(
                n,
                slow.len(),
                "seed {s:#x} tile ({tx}, {tz}): fill wrote {n}, the law says {}",
                slow.len()
            );
            for (k, (a, b)) in fast[..n].iter().zip(slow.iter()).enumerate() {
                same_elem(
                    a,
                    b,
                    At {
                        seed: s,
                        tile: (tx, tz),
                        ix: k,
                        layer: "grid",
                    },
                );
            }
            checked += 1;
            elements += n as u64;
        }
    }
    assert!(checked >= 200, "only {checked} tiles walked");
    assert!(elements > 100_000, "only {elements} elements compared");
}

#[test]
fn the_skirt_emits_exactly_what_the_unoptimized_law_emits() {
    let table = ScatterTable::alpha_default();
    let mut fast = vec![CLUTTER_NONE; SKIRT_PER_TILE];
    let mut slow: Vec<ClutterElem> = Vec::new();
    let mut elements = 0u64;
    for s in SEEDS {
        let haven = hv(s);
        let mut lat = Lattice::new();
        for (tx, tz) in tiles_of_interest(&haven) {
            let n = terrain::skirt_fill_memo(&mut lat, s, &table, &haven, tx, tz, &mut fast);
            naive_skirt_fill(s, &table, &haven, tx, tz, &mut slow);
            assert_eq!(
                n,
                slow.len(),
                "seed {s:#x} tile ({tx}, {tz}): skirt wrote {n}, the law says {}",
                slow.len()
            );
            for (k, (a, b)) in fast[..n].iter().zip(slow.iter()).enumerate() {
                same_elem(
                    a,
                    b,
                    At {
                        seed: s,
                        tile: (tx, tz),
                        ix: k,
                        layer: "skirt",
                    },
                );
            }
            elements += n as u64;
        }
    }
    assert!(elements > 200, "only {elements} skirt elements compared");
}

/// The plain fill and the memo fill are one function; this says so, because
/// `clutter_fill` is what every test in `tests/clutter.rs` calls and
/// `clutter_fill_memo` is what the client calls.
#[test]
fn the_two_fill_entry_points_are_one_function() {
    let seed = SEEDS[0];
    let haven = hv(seed);
    let mut lat = Lattice::new();
    let mut a = vec![CLUTTER_NONE; CLUTTER_PER_TILE];
    let mut b = vec![CLUTTER_NONE; CLUTTER_PER_TILE];
    for (tx, tz) in tiles_of_interest(&haven).into_iter().take(40) {
        let na = terrain::clutter_fill(seed, &haven, tx, tz, &mut a);
        let nb = terrain::clutter_fill_memo(&mut lat, seed, &haven, tx, tz, &mut b);
        assert_eq!(na, nb);
        for k in 0..na {
            same_elem(
                &a[k],
                &b[k],
                At {
                    seed,
                    tile: (tx, tz),
                    ix: k,
                    layer: "grid",
                },
            );
        }
    }
}

// ── 3. The early-out, re-derived from outside ──────────────────────────────

/// Whether the richness stratum refuses this cell, rebuilt from the published
/// parts and NOT from `clutter_rich_cell`.
///
/// This is the independence the first draft of this file did not have. Its
/// naive fill called `clutter_rich_cell` for the per-cell law, so a mutant
/// that moved the early-out's threshold by one — refusing a cell whose roll is
/// `RICH_ACCEPT_MAX - 1` against ground rich enough to accept it, which is
/// 36% of land at 1-in-256 rolls — was green in every assertion in the file.
/// A rebuild that shares the function under test is a rebuild of nothing.
///
/// The four refusals, in any order, because each of them is final:
/// off the island, in the water, on the carriageway (where the rate is 0 and
/// therefore refuses every roll), below the rate, or on swept ground.
fn refused_by_the_law(seed: u64, haven: &Haven, cx: i32, cz: i32) -> bool {
    if !(0..terrain::CLUTTER_CELLS_PER_SIDE).contains(&cx)
        || !(0..terrain::CLUTTER_CELLS_PER_SIDE).contains(&cz)
    {
        return true;
    }
    let d = terrain::clutter_rich_draw(seed, cx, cz);
    let y = terrain::ground(seed, haven, d.x, d.z);
    if y < LAND_MIN_H {
        return true;
    }
    if terrain::road_band(seed, d.x, d.z) == terrain::RoadBand::Carriageway {
        return true;
    }
    if d.roll >= terrain::clutter_richness_at(seed, haven, d.x, d.z, y) {
        return true;
    }
    // `swept_here`'s body: `/ 256.0` so sweep 0 passes no byte and sweep 1
    // passes every one.
    (d.dither as f32) * (1.0 / 256.0) < terrain::site_sweep(haven, d.x, d.z)
}

/// The element the law would emit at a cell it does NOT refuse — position,
/// kind, yaw and scale, built from the published draw and the published kind
/// law rather than from `clutter_rich_cell`.
///
/// The refusal check above is only half the claim. `clutter_rich_cell` stopped
/// calling `clutter_kind_at` when the splat became shared: it calls the law's
/// tail with a splat it resolved itself and the `kind_bits` slice of its own
/// draw. That those two roads still arrive at the same kind is exactly the
/// positional-payload failure `CLAUDE.md` records — the right value in the
/// wrong position, with every other gate green — so it is checked against the
/// published law, not against the function under test.
fn kind_by_the_law(seed: u64, haven: &Haven, cx: i32, cz: i32) -> Clutter {
    let d = terrain::clutter_rich_draw(seed, cx, cz);
    let y = terrain::ground(seed, haven, d.x, d.z);
    // `swept` is false: a swept cell is refused one branch earlier, so the law
    // is only ever asked about unswept ground here.
    terrain::clutter_kind_at(seed, haven, d.x, d.z, y, d.kind_bits, false)
}

#[test]
fn the_early_out_refuses_exactly_what_the_rate_refuses() {
    let mut cells = 0u64;
    let mut kept = 0u64;
    for s in SEEDS {
        let haven = hv(s);
        // A block big enough that the one coincidence the early-out could get
        // wrong — a roll at the ceiling minus one over ground rich enough to
        // accept it — is hit hundreds of times rather than hoped for. Measured
        // on this island: 36% of land samples sit at the rate's ceiling, so the
        // coincidence is ~1 land cell in 710.
        let c = (900.0 / CLUTTER_CELL_M) as i32;
        for j in 0..380 {
            for i in 0..380 {
                let (cx, cz) = (c + i, c + j);
                let refused = terrain::clutter_rich_cell(s, &haven, cx, cz).kind == Clutter::None;
                assert_eq!(
                    refused,
                    refused_by_the_law(s, &haven, cx, cz),
                    "seed {s:#x} cell ({cx}, {cz}): the early-out and the rate disagree"
                );
                cells += 1;
                if !refused {
                    let e = terrain::clutter_rich_cell(s, &haven, cx, cz);
                    let d = terrain::clutter_rich_draw(s, cx, cz);
                    assert_eq!(
                        e.kind,
                        kind_by_the_law(s, &haven, cx, cz),
                        "seed {s:#x} cell ({cx}, {cz}): the shared splat and the \
                         published kind law disagree"
                    );
                    assert_eq!(e.x.to_bits(), d.x.to_bits(), "cell ({cx}, {cz}) x");
                    assert_eq!(e.z.to_bits(), d.z.to_bits(), "cell ({cx}, {cz}) z");
                    kept += 1;
                }
            }
        }
    }
    assert!(cells > 400_000, "only {cells} cells walked");
    assert!(
        kept > 5_000,
        "only {kept} cells kept an element — a block this size that grows almost \
         nothing tests the refusal and not the acceptance"
    );

    // ── And on the road, which the block above never touches ──────────────
    //
    // The carriageway is the one refusal `clutter_rich_cell` now makes for
    // ITSELF rather than by asking the rate: the shared splat made the road
    // check a single `road_band` call feeding both laws, where it used to sit
    // inside `clutter_richness_at` returning a rate of 0. Those are the same
    // refusal, and the inland block cannot say so — the ring lives at 600 to
    // 1000 m from the island centre and a block at 900 m is inside all of it.
    // Dropping the check is green in every other assertion in this file.
    //
    // The cells are FOUND rather than written down: a seed decides where its
    // own coastline is, so a hard-coded block would be right for one of the
    // three and quietly test nothing for the others.
    let mut on_road = 0u64;
    for s in SEEDS {
        let haven = hv(s);
        let mut found = 0u64;
        let c = terrain::ISLAND_SIZE * 0.5;
        let mut cz = 0;
        while cz < terrain::CLUTTER_CELLS_PER_SIDE {
            let mut cx = 0;
            while cx < terrain::CLUTTER_CELLS_PER_SIDE {
                let rd = terrain::clutter_rich_draw(s, cx, cz);
                let (x, z) = (rd.x, rd.z);
                let d2 = (x - c) * (x - c) + (z - c) * (z - c);
                // The bracket `road_band` itself answers inside; anything
                // outside it is off the ring in one compare.
                let ring = (terrain::ROAD_R_MIN * terrain::ROAD_R_MIN)
                    ..=(terrain::ROAD_R_MAX * terrain::ROAD_R_MAX);
                if ring.contains(&d2)
                    && terrain::road_band(s, x, z) == terrain::RoadBand::Carriageway
                {
                    let e = terrain::clutter_rich_cell(s, &haven, cx, cz);
                    assert_eq!(
                        e.kind == Clutter::None,
                        refused_by_the_law(s, &haven, cx, cz),
                        "seed {s:#x} carriageway cell ({cx}, {cz})"
                    );
                    found += 1;
                }
                cx += 5;
            }
            cz += 5;
        }
        assert!(
            found > 100,
            "seed {s:#x}: only {found} carriageway cells found — the scan missed \
             the ring, so the road refusal is untested on this seed"
        );
        on_road += found;
    }
    assert!(
        on_road > 500,
        "only {on_road} carriageway cells checked in total"
    );
}

/// Off-island cells, where the range check the fill hoisted to the tile lives.
#[test]
fn the_hoisted_tile_range_check_refuses_the_same_cells() {
    let s = SEEDS[0];
    let haven = hv(s);
    let edge = terrain::CLUTTER_CELLS_PER_SIDE;
    for (cx, cz) in [
        (-1, 10),
        (10, -1),
        (edge, 10),
        (10, edge),
        (edge - 1, edge - 1),
        (0, 0),
        (-9_000, -9_000),
    ] {
        assert_eq!(
            terrain::clutter_rich_cell(s, &haven, cx, cz).kind == Clutter::None,
            refused_by_the_law(s, &haven, cx, cz),
            "cell ({cx}, {cz})"
        );
    }
}

// ── 4. The premise the early-out rests on ──────────────────────────────────

/// `clutter_rich_cell` refuses a cell whose acceptance byte is at or above
/// `RICH_ACCEPT_MAX` without resolving any ground. That is the same refusal
/// the acceptance test would have made only while the rate cannot reach past
/// the ceiling. Measure it rather than argue it.
#[test]
fn the_richness_rate_never_reaches_past_its_ceiling() {
    let mut worst = 0u32;
    let mut sampled = 0u64;
    for s in SEEDS {
        let haven = hv(s);
        for (x, z) in sweep().take(30_000) {
            let y = terrain::ground(s, &haven, x, z);
            if y < LAND_MIN_H {
                continue;
            }
            let r = terrain::clutter_richness_at(s, &haven, x, z, y);
            assert!(
                r <= RICH_ACCEPT_MAX,
                "richness {r} at seed {s:#x} ({x}, {z}) is past the {RICH_ACCEPT_MAX} \
                 ceiling clutter_rich_cell's early-out rests on"
            );
            worst = worst.max(r);
            sampled += 1;
        }
    }
    assert!(sampled > 10_000, "only {sampled} land samples");
    // The ceiling has to be REACHABLE too, or the early-out is refusing cells
    // the ground would have accepted and this test would not notice.
    assert!(
        worst > RICH_ACCEPT_MAX / 2,
        "the richest ground found was {worst} of {RICH_ACCEPT_MAX} — either the \
         sweep missed the woods or the rate no longer spans its range"
    );
}

/// The population is still there. A refusal that quietly emptied the rich
/// stratum would pass every bit-identity test above if the naive rebuild used
/// the same broken cell function — so this one counts against the arithmetic
/// the stratum's own header states: ~12.5% of cells at the ceiling.
#[test]
fn the_rich_stratum_still_fires_at_the_rate_its_header_claims() {
    let seed = SEEDS[0];
    let haven = hv(seed);
    let mut rich = 0u64;
    let mut land = 0u64;
    let c = (1024.0 / CLUTTER_CELL_M) as i32;
    for j in 0..160 {
        for i in 0..160 {
            let (cx, cz) = (c + i, c + j);
            if terrain::clutter_cell(seed, &haven, cx, cz).kind != Clutter::None {
                land += 1;
            }
            if terrain::clutter_rich_cell(seed, &haven, cx, cz).kind != Clutter::None {
                rich += 1;
            }
        }
    }
    assert!(land > 20_000, "only {land} land cells in the block");
    let pct = 100.0 * rich as f64 / land as f64;
    assert!(
        rich > 0 && pct < 12.6,
        "rich stratum fired on {rich} of {land} cells ({pct:.2}%) — the ceiling \
         is {RICH_ACCEPT_MAX}/256 = 12.5%, so 0 means the early-out ate the \
         stratum and over 12.5% means the ceiling moved"
    );
}

/// The premise the hoisted tile range check rests on: a tile's cell span is
/// contiguous AND the grids divide, so "neither end is inside" really does mean
/// "no cell is inside". Cheap to state, and the thing that would make the hoist
/// wrong is a change to either constant rather than to the check.
#[test]
fn the_tile_grid_divides_the_cell_grid() {
    assert_eq!(
        terrain::CLUTTER_CELLS_PER_SIDE % CLUTTER_CELLS_PER_TILE,
        0,
        "the island is {} cells across and a tile is {} — a tile now straddles \
         the edge, so clutter_fill's whole-tile range check can drop cells that \
         are inside the world",
        terrain::CLUTTER_CELLS_PER_SIDE,
        CLUTTER_CELLS_PER_TILE
    );
}
