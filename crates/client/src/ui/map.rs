//! The island map: the palette, the hillshade, and the two positional facts
//! that make it right side up.
//!
//! Ported from `web/src/map.js`, and pure for its reasons plus one of its
//! own — **the whole file is positional payload**. `CLAUDE.md` is explicit
//! that a byte-golden is blind to what a field means and that positional
//! payloads are where the reference ecosystem actually bled; a map has no
//! golden at all, and getting a sign wrong produces a picture that is right
//! about everything and upside down. So [`paint`] is a function a headless
//! test can call and read the pixels back out of.
//!
//! ## The palette is a claim, not a starting point
//!
//! Read off `Rust Images/mapraw.jpg`, the one reference frame that is a map
//! and nothing else. The four ground colours are close in VALUE and far apart
//! in HUE deliberately: the reference map reads as terrain because relief does
//! the work and the biome tint is a wash over it, so a palette with strong
//! value contrast would fight the hillshade and flatten the island. Water is
//! the exception and is meant to be — a coastline you cannot find at a glance
//! is the whole failure this screen exists to fix.

use sim_core::terrain::{self, ISLAND_SIZE, SEA_LEVEL};

/// Metres per grid square. 2048 / 128 = 16 squares a side.
pub const GRID_M: f32 = 128.0;
/// Grid squares per side.
pub const GRID_COLS: usize = (ISLAND_SIZE / GRID_M) as usize;
/// Column letters, west to east — one per column, asserted rather than
/// trusted.
pub const GRID_LETTERS: &str = "ABCDEFGHIJKLMNOP";

/// How deep the water ramp runs before it is all floor colour, metres.
pub const DEEP_M: f32 = 12.0;

/// Beach and dune.
pub const SAND: [f32; 3] = [203.0, 185.0, 148.0];
/// Open meadow.
pub const GRASS: [f32; 3] = [109.0, 140.0, 74.0];
/// Forest litter: the same green, darker — how the reference separates wooded
/// from open ground without a second hue.
pub const LITTER: [f32; 3] = [78.0, 107.0, 58.0];
/// Rock: the alpine band and every cliff the splat law vetoes to rock.
pub const ROCK: [f32; 3] = [168.0, 152.0, 120.0];
/// The shelf, just off the beach.
pub const SEA_SHALLOW: [f32; 3] = [47.0, 106.0, 142.0];
/// Open water at [`DEEP_M`] and below.
pub const SEA_DEEP: [f32; 3] = [23.0, 50.0, 74.0];

/// The hillshade's light, as a unit vector in world axes (x east, y up,
/// z north).
///
/// Up and to the LEFT of the image, which is north-west — the convention
/// every printed relief map uses and the one `mapraw.jpg` follows. North-west
/// in world axes is `-x, +z`, and the map's north is `+z`
/// (`DECISIONS.md` §open, compass axes v0 — the same call the compass strip
/// rests on), so the sign of the z term here is the same fact the bearing
/// readout is.
pub const LIGHT: [f32; 3] = [-0.5, std::f32::consts::FRAC_1_SQRT_2, 0.5];

/// The lambert term becomes a gain: `SHADE_FLOOR + SHADE_GAIN * lambert`,
/// clamped.
///
/// **Derived, not chosen.** Flat ground has normal `(0, 1, 0)` and therefore
/// lambert `LIGHT[1]`, and flat ground must come out at the palette's own
/// colour — that is what makes the palette above a measurable claim. So
/// `floor + gain * LIGHT[1] == 1`. Fixing the gain at 0.6 (enough that a 20 m
/// ridge reads, little enough that a cliff face reaches the clamp rather than
/// black) gives the floor. The identity is asserted below rather than
/// computed here, so the assertion is not a tautology.
/// `map.js` carries this as `0.5757359312880714`, which is the `f64` the
/// identity gives. This is that number at `f32`, which is the precision the
/// browser's own canvas fill sees it at anyway.
pub const SHADE_GAIN: f32 = 0.6;
pub const SHADE_FLOOR: f32 = 0.575_735_9;
pub const SHADE_CLAMP: (f32, f32) = (0.45, 1.35);

/// The grid square a world position is in — `"H11"`, or `""` off the island.
///
/// Columns are letters west to east; rows are numbers north to south from 1.
/// **The row direction is the load-bearing half**: our north is `+z`, so row 1
/// is the `+z` edge and the row index counts DOWN in z — the same flip
/// [`paint`] applies to the image, which is why both live in one file.
pub fn grid_label(x: f32, z: f32) -> String {
    if !(0.0..ISLAND_SIZE).contains(&x) || !(0.0..ISLAND_SIZE).contains(&z) {
        return String::new();
    }
    let col = ((x / GRID_M) as usize).min(GRID_COLS - 1);
    let row = (((ISLAND_SIZE - z) / GRID_M) as usize).min(GRID_COLS - 1);
    let letter = GRID_LETTERS.as_bytes()[col] as char;
    format!("{letter}{}", row + 1)
}

/// Where a world position lands on a `size`-pixel map, in pixels.
///
/// By continuous EXTENT, where [`paint`]'s flip is by sample INDEX — and the
/// only thing that makes the two agree is where the samples sit. See
/// [`paint`].
pub fn world_to_map(x: f32, z: f32, size: usize) -> (f32, f32) {
    let s = size as f32;
    (
        x / ISLAND_SIZE * s,
        // North is +z and image rows grow south, so this flips.
        (ISLAND_SIZE - z) / ISLAND_SIZE * s,
    )
}

/// Paint the island into an RGBA buffer, `size × size`.
///
/// **Two positional facts, both of which have cost a pass elsewhere.**
///
/// 1. Sample row `j` grows with `+z`, which is NORTH, and image row `py`
///    grows DOWNWARD, which is SOUTH. So `py = size - 1 - j`. Get it wrong
///    and the map is right about everything and upside down.
/// 2. **Samples sit at pixel CENTRES.** A fill starting at 0 puts sample
///    `(i, j)` on the low-x, low-z corner of the pixel it fills; the pixel's
///    extent then runs from that sample to the next, so the painted island is
///    half a cell out on both axes — and on the flipped axis `world_to_map`
///    returns the boundary between two rows, and `floor` takes the southern
///    one every time. Sampling at centres puts `(i, j)` strictly inside the
///    pixel it painted, on both axes, with no boundary to tip over.
///
/// One pass, ~`size²` height taps plus an apron. Called once per session on
/// the first open, never per frame: the island is a function of the seed and
/// the seed does not change inside a session.
pub fn paint(seed: u64, size: usize, out: &mut [u8]) {
    assert!(out.len() >= size * size * 4, "buffer too small for the map");
    let step = ISLAND_SIZE / size as f32;
    let half = step * 0.5;
    let inv2s = 1.0 / (2.0 * step);

    for j in 0..size {
        let z = half + j as f32 * step;
        let py = size - 1 - j;
        for i in 0..size {
            let x = half + i as f32 * step;
            let h = terrain::height(seed, x, z);

            // Central differences at the map's own step. A smoothed slope
            // rather than `terrain::slope`'s ±1 m one — which is correct for a
            // map, and is also what the ground mesh feeds the splat law at its
            // own step, so the map and the world disagree no more than two
            // chunk resolutions of the world already do.
            let dhdx =
                (terrain::height(seed, x + step, z) - terrain::height(seed, x - step, z)) * inv2s;
            let dhdz =
                (terrain::height(seed, x, z + step) - terrain::height(seed, x, z - step)) * inv2s;

            let (r, g, b) = if h <= SEA_LEVEL {
                // Water: one ramp from shelf to floor, and NO hillshade. The
                // sea bed has relief and drawing it would put a second,
                // competing terrain inside the shape the eye is trying to read
                // as the coastline.
                let t = (-h / DEEP_M).clamp(0.0, 1.0);
                (
                    SEA_SHALLOW[0] + (SEA_DEEP[0] - SEA_SHALLOW[0]) * t,
                    SEA_SHALLOW[1] + (SEA_DEEP[1] - SEA_SHALLOW[1]) * t,
                    SEA_SHALLOW[2] + (SEA_DEEP[2] - SEA_SHALLOW[2]) * t,
                )
            } else {
                let slope = (dhdx * dhdx + dhdz * dhdz).sqrt();
                let w = terrain::splat_from(h, terrain::moisture(seed, x, z), slope);
                // They sum to ~255 by construction, and "~" is the splat law's
                // own word for it — dividing by the actual sum keeps the blend
                // a true average whatever the rounding did.
                let sum = (w[0] as f32 + w[1] as f32 + w[2] as f32 + w[3] as f32).max(1.0);
                let mix = |c: usize| {
                    (SAND[c] * w[0] as f32
                        + GRASS[c] * w[1] as f32
                        + LITTER[c] * w[2] as f32
                        + ROCK[c] * w[3] as f32)
                        / sum
                };
                // Hillshade. A heightfield's normal is `(-dh/dx, 1, -dh/dz)`
                // before normalising; the lambert term against `LIGHT` becomes
                // a multiplier, so the biome tint survives it as a tint.
                let inv = 1.0 / (dhdx * dhdx + 1.0 + dhdz * dhdz).sqrt();
                let lambert = (-dhdx * LIGHT[0] + LIGHT[1] + -dhdz * LIGHT[2]) * inv;
                let shade =
                    (SHADE_FLOOR + SHADE_GAIN * lambert).clamp(SHADE_CLAMP.0, SHADE_CLAMP.1);
                (mix(0) * shade, mix(1) * shade, mix(2) * shade)
            };

            let o = (py * size + i) * 4;
            out[o] = r.clamp(0.0, 255.0) as u8;
            out[o + 1] = g.clamp(0.0, 255.0) as u8;
            out[o + 2] = b.clamp(0.0, 255.0) as u8;
            out[o + 3] = 255;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn there_is_one_letter_per_column() {
        assert_eq!(GRID_LETTERS.len(), GRID_COLS);
        assert_eq!(GRID_COLS, 16);
    }

    /// The identity `SHADE_FLOOR` is derived from, asserted rather than
    /// computed — flat ground must come out at the palette's own colour, or
    /// the palette is not a claim about anything.
    #[test]
    fn flat_ground_shades_to_exactly_one() {
        let flat = SHADE_FLOOR + SHADE_GAIN * LIGHT[1];
        assert!((flat - 1.0).abs() < 1e-6, "flat ground shades to {flat}");
    }

    #[test]
    fn the_light_is_a_unit_vector_from_the_north_west() {
        let len = (LIGHT[0] * LIGHT[0] + LIGHT[1] * LIGHT[1] + LIGHT[2] * LIGHT[2]).sqrt();
        assert!((len - 1.0).abs() < 1e-6, "light is not unit: {len}");
        const { assert!(LIGHT[0] < 0.0, "the light must come from the west") };
        const { assert!(LIGHT[2] > 0.0, "the light must come from the north") };
    }

    /// Row 1 is the +z edge and rows count DOWN in z. The half of
    /// `grid_label` that is load-bearing.
    #[test]
    fn row_one_is_the_north_edge() {
        assert_eq!(grid_label(1.0, ISLAND_SIZE - 1.0), "A1");
        assert_eq!(grid_label(1.0, 1.0), format!("A{GRID_COLS}"));
        // Columns run west to east.
        assert_eq!(grid_label(ISLAND_SIZE - 1.0, ISLAND_SIZE - 1.0), "P1");
    }

    #[test]
    fn a_position_off_the_island_has_no_square() {
        assert_eq!(grid_label(-1.0, 10.0), "");
        assert_eq!(grid_label(10.0, ISLAND_SIZE), "");
    }

    #[test]
    fn world_to_map_flips_north_to_the_top() {
        let (_, py_north) = world_to_map(0.0, ISLAND_SIZE - 1.0, 256);
        let (_, py_south) = world_to_map(0.0, 1.0, 256);
        assert!(py_north < py_south, "north must be nearer the top");
        let (px_west, _) = world_to_map(1.0, 0.0, 256);
        let (px_east, _) = world_to_map(ISLAND_SIZE - 1.0, 0.0, 256);
        assert!(px_west < px_east);
    }

    /// The painted image and `world_to_map` must agree about which pixel a
    /// position is in — the half-cell bug the browser paid a pass for. A
    /// position sampled at a pixel centre must round-trip to that pixel.
    #[test]
    fn a_pixel_centre_round_trips_to_its_own_pixel() {
        let size = 64;
        let step = ISLAND_SIZE / size as f32;
        let half = step * 0.5;
        for (i, j) in [(0usize, 0usize), (17, 5), (63, 63), (31, 32)] {
            let (x, z) = (half + i as f32 * step, half + j as f32 * step);
            let (px, py) = world_to_map(x, z, size);
            assert_eq!(px.floor() as usize, i, "column for sample ({i},{j})");
            assert_eq!(py.floor() as usize, size - 1 - j, "row for sample ({i},{j})");
        }
    }

    /// The map is not upside down. Painted at a real seed, the northern half
    /// and the southern half must not be interchangeable — and specifically,
    /// the pixel `world_to_map` names for a known-land position must not be
    /// sea colour while the mirrored one is land.
    #[test]
    fn the_painted_island_matches_where_world_to_map_says_things_are() {
        let seed = 42u64;
        let size = 96usize;
        let mut buf = vec![0u8; size * size * 4];
        paint(seed, size, &mut buf);

        // Find a land sample and a sea sample by asking the terrain directly,
        // then check the painted pixel at each agrees about which it is.
        let step = ISLAND_SIZE / size as f32;
        let half = step * 0.5;
        let mut checked_land = 0;
        let mut checked_sea = 0;
        for j in (0..size).step_by(7) {
            for i in (0..size).step_by(7) {
                let (x, z) = (half + i as f32 * step, half + j as f32 * step);
                let h = terrain::height(seed, x, z);
                let (px, py) = world_to_map(x, z, size);
                let o = ((py as usize).min(size - 1) * size + (px as usize).min(size - 1)) * 4;
                let (r, g, b) = (buf[o] as f32, buf[o + 1] as f32, buf[o + 2] as f32);
                // Sea is the only thing here that is bluer than it is red.
                let bluish = b > r + 10.0;
                if h <= SEA_LEVEL {
                    assert!(bluish, "sea at ({x},{z}) painted {r},{g},{b}");
                    checked_sea += 1;
                } else {
                    assert!(!bluish, "land at ({x},{z}) painted {r},{g},{b}");
                    checked_land += 1;
                }
            }
        }
        // A vacuous pass is the failure mode: an island with no sea, or a
        // frame with no land, would satisfy every assertion above.
        assert!(checked_land > 20, "only {checked_land} land samples");
        assert!(checked_sea > 20, "only {checked_sea} sea samples");
    }

    #[test]
    fn every_pixel_is_opaque_and_none_is_left_black() {
        let size = 48;
        let mut buf = vec![0u8; size * size * 4];
        paint(7, size, &mut buf);
        for p in buf.chunks_exact(4) {
            assert_eq!(p[3], 255, "a transparent pixel");
            assert!(
                p[0] as u16 + p[1] as u16 + p[2] as u16 > 30,
                "an unpainted pixel"
            );
        }
    }
}
