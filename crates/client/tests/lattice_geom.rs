//! The drawn piece stands where the sim collides it — in WORLD space.
//!
//! **What this adds over `tests/ghost.rs`, which already gates piece shapes.**
//! That suite works in a piece's own `(t, y)` frame: how far along its edge a
//! doorway post is solid, how high the lintel's underside sits. Everything it
//! asserts is true of a piece drawn in the wrong cell, at the wrong storey,
//! facing the wrong way, or pitched onto the wrong plane — because the frame
//! it works in is the piece's own, and `base_transform` is what puts that
//! frame in the world. The transform was the half nothing read.
//!
//! It cost 0.212 m. `shape_parts`' stair ramp was centred on the storey's
//! mid-height, which puts the slab's *centre plane* on the line
//! `collide::piece_ground` walks a player up and its **top face**
//! `SLAB_T / (2·cos θ)` above that — so for the whole climb a player's feet
//! were 21 cm inside the tread they could see. Every gate in the repo was
//! green: `ghost.rs` §pitch asserts the ramp *carries* a pitch and no other
//! shape does, which a 21 cm error satisfies. Measured 2026-08-21 by sweeping
//! the sim's own `piece_ground` against the drawn surface, which is what §A
//! now does on every pass.
//!
//! **The sim side is the sim's own entry points, never a restatement.** §A
//! calls `collide::piece_ground` through a real `ColIndex`; §C calls
//! `build::anchor`; §B reads `build::column_floor_y`. A test that re-derived
//! "the plane is at cx·3" could not catch that arithmetic being wrong, which
//! is `CLAUDE.md`'s byte-golden hole in a test's clothing.
//!
//! Headless: a transform is arithmetic and `ColIndex` is a bitset. No GPU, no
//! window, no asset server — `tests/ghost.rs`' posture.

use bevy::prelude::*;
use client::render::structures::{
    base_transform, foundation_part, level_base_y, piece_span, shape_parts, Part, PartKind, SEAM_M,
    SLAB_T,
};
use sim_core::build::{
    anchor, column_floor_y, BUILD_CELL_M, LEVEL_H_M, LOC_DIAG_A, LOC_DIAG_B, LOC_EDGE_XLO,
    LOC_EDGE_ZLO, LOC_PLANE, LOC_RISER, LOC_TRI_XHI_ZHI, LOC_TRI_XHI_ZLO, LOC_TRI_XLO_ZHI,
    LOC_TRI_XLO_ZLO, SHAPE_FLOOR, SHAPE_FOUNDATION, SHAPE_ROOF, SHAPE_STAIRS, SHAPE_TRI_FLOOR,
    SHAPE_TRI_FOUNDATION, SHAPE_TRI_ROOF, SHAPE_WALL,
};
use sim_core::collide::{piece_ground, ColIndex, NO_SURFACE, WALL_THICKNESS_M};
use sim_core::terrain;

/// The solved authored sites for `seed` — what `terrain::ground` needs in
/// order to know where the carve is. Memoized: `haven` is a few thousand
/// `height` taps and it is a pure function of the seed, so caching cannot
/// change a result (`tests/ghost.rs`' helper, for its reason).
fn hv(seed: u64) -> &'static terrain::Haven {
    use std::cell::RefCell;
    thread_local! {
        static CACHE: RefCell<Vec<(u64, &'static terrain::Haven)>> =
            const { RefCell::new(Vec::new()) };
    }
    if let Some(h) = CACHE.with(|c| c.borrow().iter().find(|(s, _)| *s == seed).map(|&(_, h)| h)) {
        return h;
    }
    let h: &'static terrain::Haven = Box::leak(Box::new(terrain::haven(seed)));
    CACHE.with(|c| c.borrow_mut().push((seed, h)));
    h
}

/// The shipped island and `ghost.rs`' own buildable cell.
const SEED: u64 = 20260731;
const CX: u16 = 341;
const CZ: u16 = 341;

/// The plates every sweep here runs over: flat on its own ground, mid-stilt,
/// the stilt ceiling, and cut into the hill (build plate v1,
/// `build::plate_for`). A base's floor is a stored CHOICE now, so a gate that
/// only ever asked about plate 0 would be asking about the one case the
/// feature does not change.
const PLATES: [i8; 4] = [
    0,
    3,
    sim_core::build::PLATE_RISE_MAX_BANDS as i8,
    -(sim_core::build::PLATE_SINK_MAX_BANDS as i8),
];

/// How far a drawn surface may sit from the collided one, metres.
///
/// **Not a tolerance for drift — an f32's own resolution at this cell.** The
/// sweep runs at world x ≈ 1024 m, where one ulp is 2⁻¹³ ≈ 1.2·10⁻⁴ m, and
/// the drawn side reaches its answer through a matrix inverse and a 45°
/// rotation while the sim's reaches it by adding two terms. A couple of ulp
/// apart is the two paths agreeing, not disagreeing: 1e-4 was the first draft
/// and the stilted sweeps sat exactly on it.
///
/// A millimetre still leaves the gate its whole point — the stair defect it
/// was written from was 212 mm, and every mutant in this file's own sweep is
/// centimetres. A tolerance is only dangerous when it is near the size of the
/// bug class it is meant to catch.
const EPS: f32 = 1.0e-3;

/// The parts a shape draws at an address, foundations included — the
/// per-address skirt is a different emit from the per-shape table and
/// `spawn_piece` picks between them, so a gate that only read the table would
/// be blind to the shape the game actually spawns most of.
fn parts_at(shape: u8, cx: u16, cz: u16, plate: i8) -> Vec<Part> {
    if matches!(shape, SHAPE_FOUNDATION | SHAPE_TRI_FOUNDATION) {
        return vec![foundation_part(
            SEED,
            hv(SEED),
            cx,
            cz,
            shape == SHAPE_TRI_FOUNDATION,
            plate,
        )];
    }
    let (parts, n) = shape_parts(shape);
    parts[..n].to_vec()
}

/// Is world point `p` inside anything the client draws for this piece?
fn drawn_solid(shape: u8, addr: (u16, u16, u8, u8), plate: i8, p: Vec3) -> bool {
    let root = base_transform(SEED, hv(SEED), addr, plate);
    parts_at(shape, addr.0, addr.1, plate).iter().any(|part| {
        let l = (root * part.transform())
            .to_matrix()
            .inverse()
            .transform_point3(p);
        let h = part.size * 0.5;
        let in_box = l.x.abs() <= h.x && l.y.abs() <= h.y && l.z.abs() <= h.z;
        // The prism is the box's NW half, split on the hypotenuse its own
        // mesh is built from (`tri_prism_mesh`): x/h.x + z/h.z ≤ 0.
        in_box && (part.kind == PartKind::Box || l.x / h.x + l.z / h.z <= 0.0)
    })
}

/// The highest drawn surface at (x, z), or `None` where the piece draws
/// nothing over that spot. Bisection rather than a march: a march's answer is
/// its own step size, and the defect this file exists to catch was 212 mm —
/// two orders above any step worth taking, but the NEXT one may not be.
fn drawn_top(
    shape: u8,
    addr: (u16, u16, u8, u8),
    plate: i8,
    x: f32,
    z: f32,
    from_y: f32,
) -> Option<f32> {
    let (mut air, mut solid) = (from_y, from_y);
    // Walk down in slab-sized bites until we are inside something.
    let mut y = from_y;
    while y > from_y - 6.0 {
        if drawn_solid(shape, addr, plate, Vec3::new(x, y, z)) {
            solid = y;
            break;
        }
        air = y;
        y -= 0.02;
    }
    if solid == from_y {
        return None;
    }
    for _ in 0..32 {
        let mid = (air + solid) * 0.5;
        if drawn_solid(shape, addr, plate, Vec3::new(x, mid, z)) {
            solid = mid;
        } else {
            air = mid;
        }
    }
    Some(solid)
}

/// A column index holding exactly this one piece — the sim's own derived
/// collision state, built by the sim's own `add`.
fn cols_with(cx: u16, cz: u16, level: u8, loc: u8, shape: u8, plate: i8) -> ColIndex {
    let mut c = ColIndex::new();
    c.add(cx, cz, level, loc, shape, plate);
    c
}

/// How close to a shape's own edge a sample may sit before it is skipped,
/// metres. Two different reasons, one number.
///
/// The **cell** boundary is the drawn seam ([`SEAM_M`]): a deliberate cosmetic
/// inset, pinned by §D, and re-litigating it inside a height sweep would only
/// make §A noisy. The **triangle's hypotenuse** is float noise on a shared
/// line: `collide::piece_ground`'s `in_half` is boundary-inclusive on both
/// sides on purpose, so a body on the seam of a NW+SE pair reads one floor
/// from either — and the two sides reach that line by different arithmetic
/// (the sim adds two cell offsets, the client inverts a rotated matrix), which
/// disagrees at 6·10⁻⁵ m. That is not drift; it is the last bit of an f32.
const SKIP_M: f32 = SEAM_M;

/// Is the sample within [`SKIP_M`] of this loc's own drawn boundary?
fn on_a_seam(loc: u8, dx: f32, dz: f32) -> bool {
    let anti = (dx + dz - BUILD_CELL_M).abs() <= SKIP_M;
    let main = (dz - dx).abs() <= SKIP_M;
    match loc {
        LOC_TRI_XLO_ZLO | LOC_TRI_XHI_ZHI => anti,
        LOC_TRI_XHI_ZLO | LOC_TRI_XLO_ZHI => main,
        _ => false,
    }
}

/// Sample points strictly inside the cell, far enough from any boundary that
/// [`SKIP_M`] cannot explain a disagreement.
fn inside_cell(cx: u16, cz: u16, loc: u8, n: usize) -> impl Iterator<Item = (f32, f32)> {
    let span = BUILD_CELL_M - 2.0 * SKIP_M;
    let (x0, z0) = (
        cx as f32 * BUILD_CELL_M + SKIP_M,
        cz as f32 * BUILD_CELL_M + SKIP_M,
    );
    (0..=n)
        .flat_map(move |i| {
            (0..=n).map(move |j| {
                (
                    x0 + span * i as f32 / n as f32,
                    z0 + span * j as f32 / n as f32,
                )
            })
        })
        .filter(move |&(x, z)| {
            !on_a_seam(
                loc,
                x - cx as f32 * BUILD_CELL_M,
                z - cz as f32 * BUILD_CELL_M,
            )
        })
}

// ---------------------------------------------------------------------------
// §A · You stand on the surface you can see
// ---------------------------------------------------------------------------

/// For every walkable shape, at every level it can hold, the surface
/// `collide::piece_ground` puts a player's feet on IS the top of what the
/// client draws — sampled across the cell, not just at its centre.
///
/// **The stair ramp is why this sweeps rather than spot-checks.** A slab
/// pitched onto the wrong plane is off by a constant everywhere, which one
/// sample at the cell centre catches; a slab of the wrong LENGTH, or one slid
/// along its own slope, is right in the middle and wrong at both ends. The
/// shipped ramp was both.
#[test]
fn the_drawn_surface_is_the_surface_the_sim_stands_you_on() {
    let cases: [(&str, u8, u8, u8); 8] = [
        ("foundation", SHAPE_FOUNDATION, 0, LOC_PLANE),
        ("floor L1", SHAPE_FLOOR, 1, LOC_PLANE),
        ("floor L3", SHAPE_FLOOR, 3, LOC_PLANE),
        ("roof L2", SHAPE_ROOF, 2, LOC_PLANE),
        ("stairs L0", SHAPE_STAIRS, 0, LOC_RISER),
        ("stairs L2", SHAPE_STAIRS, 2, LOC_RISER),
        ("tri foundation", SHAPE_TRI_FOUNDATION, 0, LOC_TRI_XLO_ZLO),
        ("tri floor", SHAPE_TRI_FLOOR, 1, LOC_TRI_XHI_ZHI),
    ];
    for (name, shape, level, loc) in cases {
        for plate in PLATES {
            let name = &format!("{name} @plate {plate}");
            let cols = cols_with(CX, CZ, level, loc, shape, plate);
            let base = level_base_y(SEED, hv(SEED), CX, CZ, level, plate);
            // Feet on the storey: `piece_ground`'s step rule refuses a surface
            // more than STEP_UP above the feet, and a rider on the ramp is at
            // the ramp's own height, so probe from the top of the storey.
            let mut checked = 0;
            for (x, z) in inside_cell(CX, CZ, loc, 16) {
                let feet = base + LEVEL_H_M;
                let sim = piece_ground(SEED, hv(SEED), &cols, x, z, feet);
                let drawn = drawn_top(
                    shape,
                    (CX, CZ, level, loc),
                    plate,
                    x,
                    z,
                    base + LEVEL_H_M + 2.0,
                );
                match (sim == NO_SURFACE, drawn) {
                    // Off the piece's own half (a triangle) — both must say so.
                    (true, None) => {}
                    (false, Some(d)) => {
                        assert!(
                            (d - sim).abs() <= EPS,
                            "{name}: at ({x:.3},{z:.3}) the sim stands you at {sim:.4} and the \
client draws {d:.4} — off by {:.4} m",
                            d - sim
                        );
                        checked += 1;
                    }
                    (true, Some(d)) => panic!(
                        "{name}: at ({x:.3},{z:.3}) the client draws a surface at {d:.4} and the \
sim has none"
                    ),
                    (false, None) => panic!(
                        "{name}: at ({x:.3},{z:.3}) the sim stands you at {sim:.4} and the client \
draws nothing"
                    ),
                }
            }
            assert!(checked > 100, "{name}: only {checked} samples landed on it");
        }
    }
}

/// The stair ramp reaches BOTH plates it joins: its tread is the storey base
/// at the cell's low-z boundary and the next storey's base at the high one.
///
/// **Sampled a millimetre inside each boundary, and that is the whole test.**
/// A run one metre short is a ramp you climb into a wall; a run centred wrong
/// is a step at the top. The shipped 4.15 m was 0.09 m short of the cell's
/// diagonal — and a first draft of this file sampled a *seam* inside each end
/// instead, which is 0.04 m, and the short ramp reaches 0.033 m. That draft
/// went green on the bug (proven, 2026-08-21, running the mutant this file's
/// own §A was written from): an assertion that samples inside the defect is
/// an assertion about nothing. The cell boundary is where the neighbouring
/// plate begins, so the boundary is where the tread has to be.
#[test]
fn the_ramp_meets_the_plate_at_both_ends() {
    for plate in PLATES {
        let cols = cols_with(CX, CZ, 0, LOC_RISER, SHAPE_STAIRS, plate);
        let base = level_base_y(SEED, hv(SEED), CX, CZ, 0, plate);
        let next = level_base_y(SEED, hv(SEED), CX, CZ, 1, plate);
        let x = CX as f32 * BUILD_CELL_M + BUILD_CELL_M * 0.5;
        let z0 = CZ as f32 * BUILD_CELL_M;
        let nick = 0.001f32;
        let slope = LEVEL_H_M / BUILD_CELL_M;
        for (label, zr, meets) in [("foot", nick, base), ("head", BUILD_CELL_M - nick, next)] {
            let z = z0 + zr;
            let sim = piece_ground(SEED, hv(SEED), &cols, x, z, base + LEVEL_H_M);
            assert!(
                (sim - (base + zr * slope)).abs() <= EPS,
                "{label} @plate {plate}: the sim's ramp reads {sim:.4} at z+{zr}"
            );
            assert!(
                (sim - meets).abs() <= nick * slope + EPS,
                "{label} @plate {plate}: the sim's ramp is {:.4} m off the plate it meets",
                (sim - meets).abs()
            );
            let drawn = drawn_top(
                SHAPE_STAIRS,
                (CX, CZ, 0, LOC_RISER),
                plate,
                x,
                z,
                base + LEVEL_H_M + 2.0,
            )
            .unwrap_or_else(|| {
                panic!("{label} @plate {plate}: the ramp draws nothing at z+{zr} of its own cell")
            });
            assert!(
                (drawn - sim).abs() <= EPS,
                "{label} @plate {plate}: the tread is drawn at {drawn:.4} and walked at \
{sim:.4} — off by {:.4} m",
                drawn - sim
            );
        }
    }
}

// ---------------------------------------------------------------------------
// §B · Mathematical cubes: one lattice, both sides
// ---------------------------------------------------------------------------

/// A storey is a cube: the drawn base of level N is the drawn base of level 0
/// plus exactly N storeys, and a wall drawn at level N spans from that base to
/// the base of level N+1 with no gap and no overlap.
///
/// Stated as an identity (`==` on f32, not a tolerance) because
/// `BUILD_BASE_Q_M`'s whole derivation is that flushness is bit-equal
/// (`build::column_floor_y`), and a storey term that needs an epsilon is a
/// storey term that has stopped being a multiple.
#[test]
fn every_storey_is_one_cube_tall_on_both_sides() {
    for plate in PLATES {
        let base0 = level_base_y(SEED, hv(SEED), CX, CZ, 0, plate);
        assert_eq!(
            base0,
            column_floor_y(SEED, hv(SEED), CX, CZ, plate),
            "level 0 is the column's own floor, not a copy of the rule"
        );
        // The plate is a whole number of quanta off the column's own ground,
        // exactly — a stilt is a lattice step, not a float.
        assert_eq!(
            base0 - column_floor_y(SEED, hv(SEED), CX, CZ, 0),
            plate as f32 * sim_core::build::BUILD_BASE_Q_M,
            "plate {plate} is not a whole number of bands off the terrain rule"
        );
        for level in 0..8u8 {
            let base = level_base_y(SEED, hv(SEED), CX, CZ, level, plate);
            assert_eq!(
                base,
                base0 + level as f32 * LEVEL_H_M,
                "level {level} is not {level} storeys over level 0 (plate {plate})"
            );
            // The wall at this level: base point at the storey base, top face
            // at the next storey's base.
            let (parts, n) = shape_parts(SHAPE_WALL);
            assert_eq!(n, 1, "a wall is one part");
            let w = parts[0];
            assert_eq!(w.size.y, LEVEL_H_M, "a wall is not one storey tall");
            assert_eq!(
                w.offset.y - w.size.y * 0.5,
                0.0,
                "a wall's foot is not on its own storey base"
            );
            if level + 1 < 8 {
                let next = level_base_y(SEED, hv(SEED), CX, CZ, level + 1, plate);
                assert_eq!(
                    base + w.offset.y + w.size.y * 0.5,
                    next,
                    "the wall at level {level} does not reach level {}",
                    level + 1
                );
            }
        }
    }
}

/// The cell is square and the storey is as tall as the cell is wide — the
/// premise every other assertion here rests on, and the one a future knob
/// change would break silently.
///
/// Not a taste claim: `collide::piece_ground` ramps a stair `LEVEL_H_M` over
/// `BUILD_CELL_M`, `structures::stairs_pitch` derives its angle from the same
/// pair, and the diagonal wall's √2 comes from the cell being square. Move
/// either number and this says so before the geometry quietly stops matching.
#[test]
fn the_build_lattice_is_cubic() {
    assert_eq!(
        BUILD_CELL_M, LEVEL_H_M,
        "the storey is not the cell's height"
    );
    assert_eq!(
        piece_span(),
        BUILD_CELL_M - SEAM_M,
        "a piece does not span its cell less the seam"
    );
    const {
        assert!(
            SLAB_T < LEVEL_H_M,
            "a plane slab is thicker than the storey it hangs under"
        )
    };
}

/// A plane piece's TOP is the level plane and its body hangs BELOW it — so a
/// floor never lifts the player above the storey the sim placed them on, and
/// the storey's clear headroom is the whole `LEVEL_H_M` minus the slab above.
#[test]
fn a_plane_hangs_under_its_own_plate() {
    for plate in PLATES {
        for shape in [
            SHAPE_FLOOR,
            SHAPE_ROOF,
            SHAPE_TRI_FLOOR,
            SHAPE_TRI_ROOF,
            SHAPE_FOUNDATION,
            SHAPE_TRI_FOUNDATION,
        ] {
            for part in parts_at(shape, CX, CZ, plate) {
                let top = part.offset.y + part.size.y * 0.5;
                assert!(
                    top.abs() <= EPS,
                    "shape {shape}: the plane's top is {top:.4} off its own plate"
                );
                assert!(
                    part.size.y > 0.0,
                    "shape {shape}: a plane with no thickness"
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// §C · The address puts the piece in the right cell, facing the right way
// ---------------------------------------------------------------------------

/// `base_transform`'s XZ is the sim's own `build::anchor` for every loc the
/// wire can name.
///
/// The straight edges are the sharp half: they are canonical to the cell's
/// low-x / low-z boundary, so a transform that anchored at the cell CENTRE
/// instead would draw every wall in a base half a cell off its own edge — and
/// `ghost.rs`, working in the piece's own frame, would still pass every one of
/// its assertions.
#[test]
fn the_transform_anchors_where_the_sim_anchors() {
    for loc in [
        LOC_PLANE,
        LOC_RISER,
        LOC_EDGE_XLO,
        LOC_EDGE_ZLO,
        LOC_DIAG_A,
        LOC_DIAG_B,
        LOC_TRI_XLO_ZLO,
        LOC_TRI_XHI_ZLO,
        LOC_TRI_XLO_ZHI,
        LOC_TRI_XHI_ZHI,
    ] {
        let t = base_transform(SEED, hv(SEED), (CX, CZ, 0, loc), 0);
        let (ax, az) = anchor(CX, CZ, loc);
        // A triangle's anchor is its CENTROID (thirds) and its drawn root is
        // the cell centre it is turned about — the one loc family where the
        // two are deliberately different points. Everything else must land on
        // the sim's own.
        if (LOC_TRI_XLO_ZLO..=LOC_TRI_XHI_ZHI).contains(&loc) {
            let (cx, cz) = (
                CX as f32 * BUILD_CELL_M + BUILD_CELL_M * 0.5,
                CZ as f32 * BUILD_CELL_M + BUILD_CELL_M * 0.5,
            );
            assert_eq!((t.translation.x, t.translation.z), (cx, cz), "tri {loc}");
            // ...and the centroid the sim measures reach to must fall inside
            // the half that is drawn, which is the claim that actually binds
            // the two together.
            assert!(
                drawn_solid(SHAPE_TRI_FLOOR, (CX, CZ, 1, loc), 0, {
                    let y = level_base_y(SEED, hv(SEED), CX, CZ, 1, 0) - SLAB_T * 0.5;
                    Vec3::new(ax, y, az)
                }),
                "tri {loc}: the sim's anchor is not inside the drawn half"
            );
            continue;
        }
        assert_eq!(
            (t.translation.x, t.translation.z),
            (ax, az),
            "loc {loc}: drawn at ({:.3},{:.3}), the sim anchors at ({ax:.3},{az:.3})",
            t.translation.x,
            t.translation.z
        );
    }
}

/// A low-z edge is the low-x edge turned a quarter, and both straddle the
/// boundary the sim canonicalises them to: half the slab either side of it.
#[test]
fn an_edge_piece_straddles_its_own_boundary() {
    let base = level_base_y(SEED, hv(SEED), CX, CZ, 0, 0);
    let mid = base + LEVEL_H_M * 0.5;
    let half = WALL_THICKNESS_M * 0.5;
    let (x0, z0) = (CX as f32 * BUILD_CELL_M, CZ as f32 * BUILD_CELL_M);
    let centre = BUILD_CELL_M * 0.5;
    for (loc, on, off) in [
        (LOC_EDGE_XLO, Vec3::X, Vec3::Z),
        (LOC_EDGE_ZLO, Vec3::Z, Vec3::X),
    ] {
        let at = |d: f32| Vec3::new(x0, mid, z0) + on * d + off * centre;
        for d in [-half + 1e-3, -half * 0.5, 0.0, half * 0.5, half - 1e-3] {
            assert!(
                drawn_solid(SHAPE_WALL, (CX, CZ, 0, loc), 0, at(d)),
                "loc {loc}: no wall drawn {d:.3} m off its boundary"
            );
        }
        for d in [-half - 1e-2, half + 1e-2] {
            assert!(
                !drawn_solid(SHAPE_WALL, (CX, CZ, 0, loc), 0, at(d)),
                "loc {loc}: wall drawn {d:.3} m off its boundary, past its thickness"
            );
        }
    }
}

/// A foundation stilted to the sim's ceiling is still buried by its own
/// skirt — the cross-crate inequality `build::PLATE_RISE_MAX_BANDS` states in
/// prose and neither crate can state in a `const`.
///
/// **The two halves live in two crates and nothing else joins them.** The sim
/// decides how far a plate may stand over its ground; `render/structures.rs`
/// decides how deep the drawn skirt goes and caps it at `SKIRT_MAX_M`. Raise
/// the stilt past that cap and the sim keeps taking placements the renderer
/// draws as a plank floating in the air, with every gate in both crates green
/// — the shape of the doc-claims-a-gate failure `CLAUDE.md`'s header is about.
/// Asserted at the address rather than in arithmetic, so it reads the emit
/// the game actually spawns.
#[test]
fn the_skirt_reaches_the_stilt_the_sim_allows() {
    use client::render::structures::{SKIRT_MAX_M, SKIRT_SINK_M};
    let rise = sim_core::build::PLATE_RISE_MAX_BANDS as f32 * sim_core::build::BUILD_BASE_Q_M;
    assert!(
        rise + SKIRT_SINK_M <= SKIRT_MAX_M,
        "a plate may stilt {rise} m and the skirt stops at {SKIRT_MAX_M} m"
    );
    // And measured on the emit: at the ceiling plate, the skirt's bottom is
    // still under every ground sample of its own cell.
    let plate = sim_core::build::PLATE_RISE_MAX_BANDS as i8;
    let part = foundation_part(SEED, hv(SEED), CX, CZ, false, plate);
    let top = level_base_y(SEED, hv(SEED), CX, CZ, 0, plate);
    let bottom = top + part.offset.y - part.size.y * 0.5;
    let x0 = CX as f32 * BUILD_CELL_M;
    let z0 = CZ as f32 * BUILD_CELL_M;
    for (px, pz) in [
        (x0, z0),
        (x0 + BUILD_CELL_M, z0),
        (x0, z0 + BUILD_CELL_M),
        (x0 + BUILD_CELL_M, z0 + BUILD_CELL_M),
        (x0 + BUILD_CELL_M * 0.5, z0 + BUILD_CELL_M * 0.5),
    ] {
        let ground = sim_core::terrain::ground(SEED, hv(SEED), px, pz);
        assert!(
            bottom <= ground,
            "at the stilt ceiling the skirt's bottom {bottom:.3} floats over ground {ground:.3}"
        );
    }
}

// ---------------------------------------------------------------------------
// §D · The pinned cosmetic deviations
// ---------------------------------------------------------------------------

/// The seam, restated for the two shapes that are NOT edge pieces, and the
/// diagonal's own version of it.
///
/// `ghost.rs` pins the doorway's `SEAM_M / 2` inset. These are the other two
/// places the same idea shows up, both measured 2026-08-21 and both pinned
/// here rather than tolerated: a plane is inset half a seam from its cell, and
/// a **diagonal wall is inset `SEAM_M·√2 / 2` — 28 mm, not 20** — because its
/// span is the straight wall's, stretched by the cell's diagonal ratio on the
/// root. At 28 mm no player can see or exploit it; if it ever grows this says
/// so before anybody has to notice a gap at a corner.
#[test]
fn the_drawn_insets_are_the_pinned_ones() {
    let seam = SEAM_M * 0.5;
    // A plane: half a seam in from each side of the cell.
    let (parts, _) = shape_parts(SHAPE_FLOOR);
    assert_eq!(parts[0].size.x, BUILD_CELL_M - SEAM_M);
    assert_eq!(parts[0].size.z, BUILD_CELL_M - SEAM_M);

    // A diagonal: measure the drawn slab's own ends against the span the sim
    // blocks (`collide`'s `DIAG_LEN_M`, the cell's diagonal).
    let sim_len = BUILD_CELL_M * std::f32::consts::SQRT_2;
    for loc in [LOC_DIAG_A, LOC_DIAG_B] {
        let root = base_transform(SEED, hv(SEED), (CX, CZ, 0, loc), 0);
        let (parts, _) = shape_parts(SHAPE_WALL);
        let m = (root * parts[0].transform()).to_matrix();
        let h = parts[0].size.z * 0.5;
        let a = m.transform_point3(Vec3::new(0.0, 0.0, -h));
        let b = m.transform_point3(Vec3::new(0.0, 0.0, h));
        let short = sim_len - (b - a).length();
        assert!(
            (short - SEAM_M * std::f32::consts::SQRT_2).abs() <= 1e-3,
            "diag {loc}: short by {short:.4} m, not the pinned {:.4}",
            SEAM_M * std::f32::consts::SQRT_2
        );
        assert!(
            short * 0.5 < 0.03 && short * 0.5 > seam,
            "diag {loc}: {:.4} m per end is outside the pinned band",
            short * 0.5
        );
    }
}
