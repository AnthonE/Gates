//! Gate: the minimap is a picture of the island the player is standing on.
//!
//! `NOW.md` §0gp item 5 — *"`ui/map.rs` carries an independent minimap palette
//! that nothing holds against `GROUND_ALBEDO`, and it did not move with either
//! edit"*. Two edits landed on the ground's four identities on 2026-08-15 (the
//! re-place onto `ART.md` §3's luma column, then the splat material) and the
//! map's four colours did not move with either, because nothing connected
//! them. This is that connection.
//!
//! **It does not assert the two palettes are equal, and must not.** They are
//! read off different sources and they answer different questions.
//! `GROUND_ALBEDO` is the albedo of ground you are standing on, placed onto
//! `ART.md` §3's measured hue/sat/luma columns. The map palette is read off
//! `mapraw.jpg`, the one reference frame that is a map and nothing else, and
//! `ui/map.rs`'s header states the transform it applies on purpose: values
//! close together so the hillshade does the work, hues far apart so the
//! identities separate. A gate that flattened one onto the other would delete
//! a deliberate design.
//!
//! What it holds instead is that the transform is the one the header claims,
//! and that the two palettes still describe the same world:
//!
//! 1. **One map colour per ground identity**, counted off `terrain::splat`'s
//!    own return width rather than hand-kept.
//! 2. **Hue tracks**, identity by identity — with litter's divergence named,
//!    sized and deliberate rather than merely absent.
//! 3. **Value compresses**, never expands — the header's own claim.
//! 4. **Value ORDER agrees**, with the two disagreeing pairs listed by name.
//!    One of them is cartographic convention and one is suspected drift; both
//!    are in the table below rather than in a comment, so a THIRD inversion is
//!    a red gate and so is quietly fixing either of these.
//!
//! Headless — no GPU, no window, no shard. `#[cfg(feature = "render")]`
//! because `GROUND_ALBEDO` lives behind it and `ui::map` does not: this gate
//! is the seam between them, which is exactly the feature-line trap
//! `CLAUDE.md` names (a type read from `render::` has its checks on only one
//! side, so a workspace run is green twice and the Bevy gate is red).

#![cfg(feature = "render")]

use client::render::terrain_mesh::GROUND_ALBEDO;
use client::ui::map::{GRASS, LITTER, ROCK, SAND};
use sim_core::terrain;

/// `terrain::splat`'s order, and the only order either palette may be read in.
const NAMES: [&str; 4] = ["sand", "grass", "litter", "rock"];

/// The map's four ground colours, in that same order.
fn map_palette() -> [[f32; 3]; 4] {
    [SAND, GRASS, LITTER, ROCK]
}

/// `GROUND_ALBEDO` encoded to the 0–255 sRGB the map palette is written in.
///
/// The comparison has to happen in one space and this is the one the map is
/// authored in; encoding the ground rather than decoding the map keeps the
/// authored numbers in `map.rs` readable as the bytes they are.
fn ground_palette() -> [[f32; 3]; 4] {
    let enc = |c: f32| {
        let s = if c <= 0.003_130_8 {
            12.92 * c
        } else {
            1.055 * c.powf(1.0 / 2.4) - 0.055
        };
        s * 255.0
    };
    let mut out = [[0.0f32; 3]; 4];
    for (k, a) in GROUND_ALBEDO.iter().enumerate() {
        for ch in 0..3 {
            out[k][ch] = enc(a[ch]);
        }
    }
    out
}

fn luma(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

/// HSV hue in degrees. Both palettes are warm-to-green ground, so the hue
/// circle never wraps in practice, but [`arc`] handles it anyway.
fn hue(c: [f32; 3]) -> f32 {
    let max = c[0].max(c[1]).max(c[2]);
    let min = c[0].min(c[1]).min(c[2]);
    let d = max - min;
    if d <= 0.0 {
        return 0.0;
    }
    let h = if max == c[0] {
        60.0 * (((c[1] - c[2]) / d) % 6.0)
    } else if max == c[1] {
        60.0 * ((c[2] - c[0]) / d + 2.0)
    } else {
        60.0 * ((c[0] - c[1]) / d + 4.0)
    };
    if h < 0.0 {
        h + 360.0
    } else {
        h
    }
}

/// The short way round the hue circle, degrees.
fn arc(a: f32, b: f32) -> f32 {
    let d = (a - b).abs() % 360.0;
    if d > 180.0 {
        360.0 - d
    } else {
        d
    }
}

/// Leg 1. **One map colour per ground identity, and the count is derived.**
///
/// This is the leg that fires on the thing nobody will remember: a fifth
/// ground identity. `assets/textures/MANIFEST.md` carries `gravel` as the one
/// unbundled role and its blocker is exactly this — `terrain::splat` resolves
/// four, so a fifth identity is a sim-core change before it is an art change.
/// When that lands, the ground gains a colour, a photograph and a binding, and
/// **the map would silently keep painting four**, showing scree as whatever
/// the classifier folded it into. Reading the width off `splat_from`'s own
/// return rather than writing `4` here is what makes that a red gate instead
/// of a frame nobody questions.
#[test]
fn the_map_has_a_colour_for_every_ground_identity() {
    // The surface, not a memory of it — `CLAUDE.md`'s rule about hand-kept
    // mirrors, applied to an array width.
    let identities = terrain::splat_from(20.0, 0.05, 0.2).len();
    assert_eq!(
        identities,
        GROUND_ALBEDO.len(),
        "`terrain::splat` resolves {identities} identities and `GROUND_ALBEDO` \
         authors {} — the ground material itself is already wrong",
        GROUND_ALBEDO.len()
    );
    assert_eq!(
        map_palette().len(),
        identities,
        "`terrain::splat` resolves {identities} ground identities and \
         `ui/map.rs` paints {}.\n\
         A new identity needs a map colour in the same commit, or the minimap \
         paints it as whichever one the classifier folded it into and nothing \
         says so. See `assets/textures/MANIFEST.md` on `gravel`.",
        map_palette().len()
    );
    assert_eq!(NAMES.len(), identities, "this gate's own names drifted");
}

/// Leg 2. **Hue tracks the ground, identity by identity — except litter.**
///
/// Three of the four agree to within 2°: sand 1.6°, rock 1.0°, and grass 19.9°
/// which is the map pushing turf greener to separate it. Litter is 57.5° away
/// and that is a different hue FAMILY — the ground paints forest floor as warm
/// needle litter (38.0°, `ART.md` §3's dirt path) and the map paints it green
/// (95.5°), because `map.rs`'s header takes the reference's own convention:
/// *"the same green, darker — how the reference separates wooded from open
/// ground without a second hue."*
///
/// **The exception is asserted, not merely allowed.** If someone "fixes" the
/// map by browning its litter, this fires and points at the header, because
/// that change would cost the map its wooded/open read to buy an agreement no
/// player standing on the ground can see.
#[test]
fn hue_tracks_the_ground_except_where_the_map_says_why() {
    let g = ground_palette();
    let m = map_palette();

    for k in [0, 1, 3] {
        let d = arc(hue(g[k]), hue(m[k]));
        assert!(
            d <= 25.0,
            "{}: the map's hue is {:.1}° and the ground's is {:.1}° — {d:.1}° \
             apart.\nThe map may push a hue to separate an identity; past 25° \
             it is painting a different material.",
            NAMES[k],
            hue(m[k]),
            hue(g[k])
        );
    }

    let d = arc(hue(g[2]), hue(m[2]));
    assert!(
        d >= 30.0,
        "litter's map hue ({:.1}°) has come within {d:.1}° of the ground's \
         ({:.1}°).\nThat divergence is deliberate and load-bearing: the map \
         separates wooded from open ground by painting litter as a DARKER \
         GREEN, which is the reference's own convention and the reason the \
         minimap reads as terrain. If it was closed on purpose, delete this \
         assert and the paragraph in `ui/map.rs`'s header together.",
        hue(m[2]),
        hue(g[2])
    );
}

/// Leg 3. **The map compresses value; it never expands it.**
///
/// `ui/map.rs`'s header claims this outright — *"a palette with strong value
/// contrast would fight the hillshade and flatten the island"* — and nothing
/// checked it. Measured: the ground spans 2.239× darkest to brightest and the
/// map spans 1.913×, so the claim holds today with room. It is the hillshade's
/// contract: `SHADE_CLAMP` runs 0.45–1.35, and a palette that already carried
/// the contrast would drive relief off both ends of it.
#[test]
fn the_map_holds_less_value_contrast_than_the_ground() {
    let span = |p: [[f32; 3]; 4]| {
        let l: Vec<f32> = p.iter().map(|c| luma(*c)).collect();
        let hi = l.iter().cloned().fold(f32::MIN, f32::max);
        let lo = l.iter().cloned().fold(f32::MAX, f32::min);
        hi / lo
    };
    let g = span(ground_palette());
    let m = span(map_palette());
    assert!(
        m <= g,
        "the map spans {m:.3}× in value and the ground spans {g:.3}× — the map \
         now carries MORE contrast than the terrain it draws, which is what \
         `ui/map.rs`'s header says would fight the hillshade"
    );
}

/// Leg 4. **The two palettes agree about which ground is brighter — except
/// for two named pairs.**
///
/// This is the leg §0gp item 5 is actually about. A player reads the map,
/// picks the bright band, walks there and stands on it; if the map's value
/// order disagrees with the ground's, the map is a picture of a different
/// island. Two pairs disagree today and they are not the same kind of thing:
///
/// | pair | map | ground | what it is |
/// |---|---|---|---|
/// | grass / litter | grass brighter | litter brighter | **convention.** A map draws woodland darker than meadow; `map.rs`'s header states it and takes it from `mapraw.jpg`. Standing on it, litter is the brighter surface because its value is what absorbs granite's share of the held mean (`GROUND_ALBEDO`'s note). Both are right about their own question. |
/// | sand / rock | sand brighter | rock brighter | **suspected drift, and the reason this gate exists.** The 2026-08-15 re-place put granite on `ART.md` §3's own 147.0 luma, up from the ~109 it held when it was pinned to a ratio against litter. That moved granite above beach sand on the ground and the map's `ROCK` did not follow, because nothing connected them — which is §0gp item 5's sentence exactly. `DECISIONS.md` §open carries it as an operator question, because the fix is to depart from a `mapraw.jpg` reading and only a person looking at the map can say whether it still reads as terrain. |
///
/// The list is the assertion. A **third** inversion is a red gate, and so is
/// closing either of these two without deleting its row.
#[test]
fn value_order_agrees_except_for_the_two_pairs_named_here() {
    /// `(a, b)` — the map says `a` is brighter and the ground says `b` is.
    const DECLARED: [(usize, usize); 2] = [(1, 2), (0, 3)];

    let g = ground_palette();
    let m = map_palette();
    let mut found: Vec<(usize, usize)> = Vec::new();

    for i in 0..4 {
        for j in (i + 1)..4 {
            let map_says = luma(m[i]) > luma(m[j]);
            let ground_says = luma(g[i]) > luma(g[j]);
            if map_says != ground_says {
                // Normalised the way DECLARED reads it: the map's brighter one
                // first.
                found.push(if map_says { (i, j) } else { (j, i) });
            }
        }
    }
    found.sort_unstable();
    let mut want = DECLARED;
    want.sort_unstable();

    let show = |v: &[(usize, usize)]| {
        v.iter()
            .map(|(a, b)| {
                format!(
                    "{} ({:.1} map / {:.1} ground) over {} ({:.1} / {:.1})",
                    NAMES[*a],
                    luma(m[*a]),
                    luma(g[*a]),
                    NAMES[*b],
                    luma(m[*b]),
                    luma(g[*b])
                )
            })
            .collect::<Vec<_>>()
            .join("\n    ")
    };

    assert_eq!(
        found.as_slice(),
        want.as_slice(),
        "the map and the ground disagree about brightness in a different set \
         of pairs than the two declared.\n  found:\n    {}\n  declared:\n    \
         {}\n\
         A NEW inversion means one palette moved and the other did not — that \
         is §0gp item 5 happening again, and the map is now a picture of a \
         different island. A MISSING one means an inversion was closed; delete \
         its row in this test's doc comment and its `DECISIONS.md` §open entry \
         in the same commit.",
        show(&found),
        show(&want)
    );
}
