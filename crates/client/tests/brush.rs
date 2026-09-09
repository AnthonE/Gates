//! `Clutter::Brush` — the understory, as arithmetic about the mesh
//! (`reference/FORESTS.md` §3.1, §9.1; `reference/PLANTS.md` §2).
//!
//! **What this gate is for.** `sim-core/tests/forest.rs` holds where brush
//! lands (the forest floor, ~24× the meadow's share) and how much of the
//! litter channel it takes. Neither of those can see the thing that makes the
//! layer worth having, because the sim does not know how tall anything is:
//! `Clutter::Brush` exists to fill the **0.5–2 m band** that was empty in both
//! populations — the tallest clutter was `TUFT_H` at 0.34 m, the standing
//! litter stalk is `FROND_H` at 0.19, and the scatter bush at ~7.9 per hectare
//! was the only thing above them. A brush that came out 0.3 m tall would pass
//! every gate in `sim-core` and be a second kind of grass.
//!
//! So this file asserts the mesh, and it is arithmetic rather than pixels —
//! `CLAUDE.md`'s rule for what may be gated about a frame: the sizes and the
//! material routing, in Rust, the shape `crates/client/tests/tree.rs` set.
//! Whether it *looks* like an understory is a person booting the game, and
//! that is deliberate.

#![cfg(feature = "render")]

use bevy::mesh::VertexAttributeValues;
use bevy::prelude::*;
use client::render::clutter::{element_mesh, masked, BRUSH_H, TUFT_H};
use sim_core::terrain::{Clutter, ClutterElem};

/// The band the shrub layer occupies, metres — the one `reference/PLANTS.md`
/// §2 counts as missing. Not a knob: it is the definition of the layer, and
/// `BRUSH_H` is what has to sit inside it.
const SHRUB_BAND_M: (f32, f32) = (0.5, 2.0);

/// **The sample index moves the POSITION, not just the yaw, and that is the
/// whole reason this helper exists.** `card` seeds its per-card height jitter
/// off the element's quantised x/z — not off yaw — so a sweep that holds the
/// position still and turns the element measures one draw 37 times and reports
/// it as a spread. The first cut of this file did exactly that and printed a
/// range of `0.866..0.866`, which is the tell: `CLAUDE.md`'s "a gate can be
/// exact, bit-for-bit, and aimed at nothing", in a test that had just been
/// written to avoid it.
fn elem(kind: Clutter, i: u8, scale: f32) -> ClutterElem {
    ClutterElem {
        kind,
        // Irrational-ish strides so the quantised hash key lands somewhere new
        // every sample instead of cycling.
        x: -12.0 + i as f32 * 0.37,
        y: 0.0,
        z: 41.0 + i as f32 * 0.61,
        yaw: i,
        scale,
    }
}

fn positions(m: &Mesh) -> Vec<Vec3> {
    match m.attribute(Mesh::ATTRIBUTE_POSITION) {
        Some(VertexAttributeValues::Float32x3(v)) => {
            v.iter().map(|p| Vec3::from_array(*p)).collect()
        }
        _ => panic!("clutter mesh has float3 positions"),
    }
}

/// Ground plane to highest vertex, `cover.rs`'s measure and for its reason: a
/// card sinks its root below the placement height (`CARD_SINK`), and burying a
/// mesh deeper is not standing it up.
fn stands(kind: Clutter, i: u8, scale: f32) -> f32 {
    let e = elem(kind, i, scale);
    let m = element_mesh(&e);
    positions(&m).iter().fold(0.0f32, |a, v| a.max(v.y - e.y))
}

/// The layer exists: brush stands in the shrub band and well over the turf.
///
/// Swept over POSITION (see `elem`), because `card` jitters each card's height
/// by ±20 % off a hash of the element's quantised x/z. Measured 0.686..0.866 m
/// across the sweep, against `BRUSH_H` 0.75 and `TUFT_H` 0.34.
///
/// Mutant: `BRUSH_H` at `TUFT_H` fails both halves; at 0.45 it clears the turf
/// and fails the band, which is the case worth catching — a brush shorter than
/// half a metre is a tall weed and leaves the layer empty.
#[test]
fn brush_stands_in_the_shrub_band() {
    let mut lowest = f32::MAX;
    let mut highest = 0.0f32;
    for i in (0..=255u8).step_by(7) {
        let h = stands(Clutter::Brush, i, 1.0);
        lowest = lowest.min(h);
        highest = highest.max(h);
        let tuft = stands(Clutter::Tuft, i, 1.0);
        assert!(
            h > tuft,
            "brush stands {h:.3} m against a tuft's {tuft:.3} at sample {i} — the \
             understory has to be a storey ABOVE the turf or it is a second grass"
        );
    }
    println!("brush stands {lowest:.3}..{highest:.3} m (TUFT_H {TUFT_H}, BRUSH_H {BRUSH_H})");
    assert!(
        lowest >= SHRUB_BAND_M.0 && highest <= SHRUB_BAND_M.1,
        "brush spans {lowest:.3}..{highest:.3} m, outside the {:?} shrub band it \
         exists to fill — under it the 0.5–2 m layer is still empty, over it the \
         thing is a tree and hides a standing player",
        SHRUB_BAND_M
    );
}

/// Brush routes through the alpha-masked material, like the tuft and unlike
/// every opaque kind.
///
/// **This is the assertion that catches a silent, total defect.** `stream`
/// splits one tile's elements into two meshes by `masked`, so a cutout kind
/// that answers `false` is drawn by the opaque shader — every card becomes a
/// solid grey quad, and the forest floor fills with rectangles. Nothing else
/// in this repo would notice: the vertex counts, the heights and the sim-side
/// share are all still exactly right. `masked`'s own doc states the rule; this
/// is the gate under it.
#[test]
fn brush_is_drawn_by_the_cutout_material() {
    for k in [
        Clutter::None,
        Clutter::Pebble,
        Clutter::Tuft,
        Clutter::Twig,
        Clutter::Shard,
        Clutter::Brush,
    ] {
        // **Exhaustive on purpose.** `grass_card.rs` already carries a
        // hand-written list of the opaque kinds, and a second copy of that
        // list here is the drift `CLAUDE.md` names twice — a mirror of
        // another module's surface goes stale, quietly, in the direction of
        // being silent about the newest thing. A `match` cannot: add a
        // seventh `Clutter` and this stops compiling until someone says
        // which material draws it, which is the whole difference between a
        // list and a law.
        let cutout = match k {
            Clutter::Tuft | Clutter::Brush => true,
            Clutter::None | Clutter::Pebble | Clutter::Twig | Clutter::Shard => false,
        };
        assert_eq!(
            masked(k),
            cutout,
            "{k:?} is routed to the wrong material: a cutout drawn opaque is a \
             grey rectangle, and a solid drawn masked is alpha-tested against a \
             texture it has no UVs for"
        );
    }
}

/// The card is scaled, not stretched.
///
/// `card` derives `half_w` from the per-card height (`hj * CARD_ASPECT * 0.5`),
/// so a taller card is a proportionally wider one and the atlas cell it samples
/// is never distorted — the defect `props::BUSH_CARD_HALF` exists to refuse on
/// the scatter bush, avoided here for free by reusing the builder. Asserted
/// rather than assumed, because "reuse the tuft builder at a bigger height" is
/// exactly the change someone would later replace with an authored quad.
///
/// The ratio is compared against the tuft's own rather than against a literal:
/// `CARD_ASPECT` is not this file's to know, and a gate that re-derives a
/// constant it cannot see is a second opinion fighting the first.
#[test]
fn brush_cards_keep_the_tufts_proportions() {
    for i in (0..=255u8).step_by(11) {
        let (bw, bh) = span(Clutter::Brush, i);
        let (tw, th) = span(Clutter::Tuft, i);
        let (br, tr) = (bw / bh, tw / th);
        // One part in a thousand: both come from the same builder with the
        // same jitter sequence, so the ratios are equal up to float order.
        assert!(
            br > tr * 0.999 && br < tr * 1.001,
            "brush cards are {br:.4} wide per unit tall against the tuft's \
             {tr:.4} at sample {i} — the atlas cell is square and a card that \
             stops being proportional stretches every leaf on it"
        );
    }
}

/// A kind's mesh extent in the horizontal and the vertical.
fn span(kind: Clutter, i: u8) -> (f32, f32) {
    let e = elem(kind, i, 1.0);
    let m = element_mesh(&e);
    let p = positions(&m);
    let (mut w, mut h) = (0.0f32, 0.0f32);
    for v in &p {
        // Horizontal extent from the element's own centre, so a card's two
        // ends are one width rather than two.
        w = w
            .max((v.x - e.x).max(e.x - v.x))
            .max((v.z - e.z).max(e.z - v.z));
        h = h.max(v.y - e.y);
    }
    (w, h)
}
