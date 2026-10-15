//! Walls between you and a sound (sound occlusion).
//!
//! The reference has muffled sound through walls since 2016: every source
//! casts a ray to the listener, and a blocked one plays through a low-pass
//! at a lower gain. Here the ray is the shot walk every bullet in the sim
//! takes (`collide::shot_hit` over the client's own piece index), so a
//! wall, a floor, a roof or a shut door muffles, and an open doorway, an
//! open window and a stair do not — exactly what a round would pass.
//!
//! Each piece crossed counts, up to [`MAX_WALLS`]: a raid heard through
//! your own wall is muffled, one heard through two is muffled more.

use sim_core::collide::{self, ColIndex, PieceHit};
use sim_core::terrain::Haven;

/// The walk's step, metres. Walls are cell edges and any step inside a 3 m
/// cell finds them; floors are point-sampled, and a 0.25 m step cannot
/// cross a 0.3 m slab between two samples.
const STEP_M: f32 = 0.25;
/// Where the walk stops short of the source, metres, so the wall a sound is
/// made on (a hit on it, a door shutting in it) does not muffle it.
const SHORT_M: f32 = 0.35;
/// How far from each end the walk looks, metres. Walls stand around bases,
/// so what muffles a sound is near the ear or near the source; the open
/// ground between a far blast and you is not walked, and the cost is
/// bounded at `2 · NEAR_END_M / STEP_M` steps whatever the distance.
pub const NEAR_END_M: f32 = 40.0;
/// Walls past this many count the same.
pub const MAX_WALLS: u8 = 2;
/// The walk's radius, metres: an arrowhead's. **Not zero**: the shot walk
/// tests a floor or a roof only for a positive radius (its plane test is
/// `distance < r`), so a zero-width ray would hear straight through slabs.
const RAY_R_M: f32 = 0.05;

/// How many built pieces stand between `ear` and `src` (world metres), up
/// to [`MAX_WALLS`]. Pure, no allocation: sim-core's shot walk over `cols`.
pub fn walls_between(
    seed: u64,
    haven: &Haven,
    cols: &ColIndex,
    ear: [f32; 3],
    src: [f32; 3],
) -> u8 {
    if cols.is_empty() {
        return 0;
    }
    let d = [src[0] - ear[0], src[1] - ear[1], src[2] - ear[2]];
    let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    let end = len - SHORT_M;
    if end.is_nan() || end <= STEP_M {
        return 0;
    }
    let at = |s: f32| {
        let t = s / len;
        [ear[0] + d[0] * t, ear[1] + d[1] * t, ear[2] + d[2] * t]
    };
    // The ear's end, then the source's: one span when they meet.
    let near = end.min(NEAR_END_M);
    let spans = [(0.0, near), ((end - NEAR_END_M).max(near), end)];
    let mut walls = 0u8;
    let mut last: Option<PieceHit> = None;
    for (a, b) in spans {
        if b <= a {
            continue;
        }
        let mut prev = at(a);
        let mut s = a;
        while s < b {
            s = (s + STEP_M).min(b);
            let p = at(s);
            if let Some((hit, _)) = collide::shot_hit(
                seed, haven, cols, prev[0], prev[2], p[0], p[2], p[1], RAY_R_M,
            ) {
                if last != Some(hit) {
                    last = Some(hit);
                    walls += 1;
                    if walls >= MAX_WALLS {
                        return walls;
                    }
                }
            }
            prev = p;
        }
    }
    walls
}

/// What `walls` pieces do to a sound: its gain, and the cutoff it is heard
/// through, Hz. `None` for a clear line. One wall is about −6 dB and dull,
/// two about −10 dB and a thud.
pub fn muffle(walls: u8) -> Option<(f32, f32)> {
    match walls {
        0 => None,
        1 => Some((0.5, 1_000.0)),
        _ => Some((0.3, 500.0)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_core::build::{self, BUILD_CELL_M, LOC_EDGE_XLO, LOC_PLANE, SHAPE_FLOOR, SHAPE_WALL};
    use sim_core::terrain;

    const SEED: u64 = 20260731;
    const CX: u16 = 682;
    const CZ: u16 = 682;

    fn haven() -> &'static Haven {
        use std::cell::OnceCell;
        thread_local! {
            static H: OnceCell<&'static Haven> = const { OnceCell::new() };
        }
        H.with(|h| *h.get_or_init(|| Box::leak(Box::new(terrain::haven(SEED)))))
    }

    fn put(cols: &mut ColIndex, cx: u16, cz: u16, level: u8, loc: u8, shape: u8) {
        let band = build::terrain_band(SEED, haven(), CX, CZ);
        let plate = (band - build::terrain_band(SEED, haven(), cx, cz)) as i8;
        cols.add(cx, cz, level, loc, shape, plate);
    }

    /// A point `dx, dz` into the test cell at `rise` metres over its floor.
    fn at(dx: f32, dz: f32, rise: f32) -> [f32; 3] {
        let y = build::column_floor_y(SEED, haven(), CX, CZ, 0) + rise;
        [
            CX as f32 * BUILD_CELL_M + dx,
            y,
            CZ as f32 * BUILD_CELL_M + dz,
        ]
    }

    #[test]
    fn a_wall_muffles_and_an_empty_island_does_not() {
        let mut cols = ColIndex::new();
        let (ear, src) = (at(1.5, 1.5, 1.6), at(-4.0, 1.5, 1.4));
        assert_eq!(walls_between(SEED, haven(), &cols, ear, src), 0);
        // The cell's low-x edge: between the ear (inside) and the source.
        put(&mut cols, CX, CZ, 0, LOC_EDGE_XLO, SHAPE_WALL);
        assert_eq!(walls_between(SEED, haven(), &cols, ear, src), 1);
        // A second wall one cell further out is counted too.
        put(&mut cols, CX - 1, CZ, 0, LOC_EDGE_XLO, SHAPE_WALL);
        assert_eq!(walls_between(SEED, haven(), &cols, ear, src), 2);
        // A sound on the ear's own side of the walls is clear.
        assert_eq!(
            walls_between(SEED, haven(), &cols, ear, at(2.5, 2.5, 1.0)),
            0
        );
        assert!(muffle(0).is_none());
        let (g1, hz1) = muffle(1).unwrap();
        let (g2, hz2) = muffle(2).unwrap();
        assert!(g2 < g1 && g1 < 1.0 && hz2 < hz1);
    }

    #[test]
    fn a_floor_muffles_the_storey_below_and_a_hit_on_the_wall_is_not_its_own_wall() {
        let mut cols = ColIndex::new();
        put(&mut cols, CX, CZ, 1, LOC_PLANE, SHAPE_FLOOR);
        let up = at(1.5, 1.5, build::LEVEL_H_M + 1.6);
        let down = at(1.4, 1.6, 1.0);
        assert_eq!(walls_between(SEED, haven(), &cols, up, down), 1);
        // A hit on the far face of a wall: the walk stops short of it.
        put(&mut cols, CX, CZ, 0, LOC_EDGE_XLO, SHAPE_WALL);
        let ear = at(1.5, 1.5, 1.6);
        assert_eq!(
            walls_between(SEED, haven(), &cols, ear, at(-0.05, 1.5, 1.5)),
            0
        );
        assert_eq!(
            walls_between(SEED, haven(), &cols, ear, at(-1.0, 1.5, 1.5)),
            1
        );
    }

    #[test]
    fn a_far_sound_walks_only_the_ends() {
        // A wall beside a blast 900 m off still muffles it; nothing in the
        // middle is walked, so the cost does not grow with the distance.
        let mut cols = ColIndex::new();
        put(&mut cols, CX, CZ, 0, LOC_EDGE_XLO, SHAPE_WALL);
        let ear = at(-900.0, 1.5, 1.6);
        let src = at(1.5, 1.5, 1.0);
        assert_eq!(walls_between(SEED, haven(), &cols, ear, src), 1);
    }
}
