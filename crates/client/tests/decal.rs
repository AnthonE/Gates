//! Gate: the forward decal's prerequisites are still on the camera, and
//! they are held in a file that does not know this module exists.
//!
//! # Why a source scrape and not a runtime check
//!
//! `render::decal` draws `ForwardDecal`s, and Bevy's own usage note for
//! them is unambiguous: *"Any camera rendering a forward decal must have
//! the `DepthPrepass` component."* Ours does — but **not because anything
//! asked for it**. `rig.rs` carries `ScreenSpaceAmbientOcclusion` for the
//! lighting slice's sake, and in Bevy 0.18 that component is
//! `#[require(DepthPrepass, NormalPrepass)]`, so the prepass is up as a
//! side effect of an unrelated decision. `Msaa::Off` is there for the same
//! borrowed reason (SSAO warns without it; Bevy's decal note asks for it
//! on wasm).
//!
//! That is a dependency across a seam with nothing holding it. Someone
//! rebalancing the lighting could drop the AO, gain back a pass, and every
//! gate in this repo would stay green while marks quietly stopped blending
//! against the surfaces they lie on — a defect that shows up as *slightly
//! wrong shading*, which is the hardest kind to attribute and the kind
//! `CLAUDE.md` says a person looking is supposed to catch. A person
//! looking will not catch this one; they will see a decal and assume
//! decals look like that.
//!
//! So this reads `rig.rs` and fails with the sentence, which is
//! `tests/sound.rs`'s shape: **the defect is a call site, not a value**,
//! so the check is a grep for the call site. What it deliberately does NOT
//! do is assert *how* the prepass gets there — if someone adds
//! `DepthPrepass` explicitly and drops the AO, that is a correct fix and
//! this gate says so.
//!
//! ⚠ It is a hand-kept mirror of another file's contents, which
//! `CLAUDE.md` warns about twice. The mitigation is that it mirrors a
//! *predicate* rather than a number: there is no count here to drift.

#![cfg(feature = "render")]

const RIG: &str = include_str!("../src/render/rig.rs");

/// The camera still carries something that brings a depth prepass with it.
#[test]
fn the_camera_still_carries_the_prepass_a_forward_decal_needs() {
    let has_ssao = RIG.contains("ScreenSpaceAmbientOcclusion {");
    let has_explicit = RIG.contains("DepthPrepass");
    assert!(
        has_ssao || has_explicit,
        "`render/rig.rs` no longer spawns the camera with either \
         `ScreenSpaceAmbientOcclusion` (which is \
         `#[require(DepthPrepass, NormalPrepass)]` in Bevy 0.18) or an \
         explicit `DepthPrepass`.\n\n\
         `render::decal` draws `ForwardDecal`s, and a forward decal \
         without a depth prepass on its camera cannot read the surface \
         behind it — so it stops fading toward its edges and renders at \
         full opacity instead. That is `ART.md` rule 2's \"a clean \
         intersection edge reads as a decal\" arriving as a literal one, \
         and no gate in this repo scores a frame.\n\n\
         If the AO was removed deliberately, add `DepthPrepass` to the \
         camera bundle in `rig.rs` and this gate goes green again."
    );
}

/// MSAA is still off.
///
/// SSAO refuses to run with it on and says so in a log line, which is the
/// loud half. The quiet half is the decal's: Bevy's usage note flags MSAA
/// as a constraint for forward decals on wasm, and while this client is
/// native, an `Msaa` setting that changed under the renderer is exactly
/// the kind of thing that gets changed for one pass's sake without the
/// other passes being re-checked.
#[test]
fn msaa_is_still_off_for_the_passes_that_need_it_off() {
    assert!(
        RIG.contains("Msaa::Off"),
        "`render/rig.rs` no longer sets `Msaa::Off` on the camera. SSAO \
         refuses to run without it (and logs), and the forward decals in \
         `render::decal` share the constraint. Turning MSAA on is a \
         decision about at least three passes, not one."
    );
}

// ─── The two things a decal has to get right to be SEEN ──────────────────────
//
// Everything above this line checks that the decal *pass* is wired up. Both
// gates below check that a mark, once drawn, is actually visible — which is a
// different claim, and it was false in two independent ways from the first
// decal in this tree until 2026-09-09 with every gate in the repo green.
//
// They are here because the operator booted the game on real hardware, chopped
// a tree, and reported no mark. `NOW.md` §0mk had recorded that as "no
// `ForwardDecal` renders under lavapipe", i.e. as a property of the box. It was
// not: it was `MARK_LIFT_M` and the `SURF_WORLD` tint, both arithmetic, both
// checkable here without a GPU and without a pixel.
//
// Neither of these can be a screenshot. `CLAUDE.md` retired the pixel gate on
// purpose and forbids building a replacement; what may be gated about a frame
// is arithmetic, and the contrast of a colour against a measured photograph is
// arithmetic.

use client::render::decal::{tint, DEPTH_FADE_M, MARK_LIFT_M, SIZE_M};
use sim_core::ranged::{SURF_BUILT, SURF_GROUND, SURF_WORLD};

const MANIFEST: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../assets/textures/MANIFEST.md"
);

/// Rec. 709 relative luminance of a `Color`, in linear light — the quantity
/// `MANIFEST.md`'s `luma` column holds, so the two are directly comparable.
///
/// The sRGB→linear step is Bevy's own `to_linear()` rather than a transfer
/// function written out here. That is deliberate: it is the conversion the
/// renderer actually applies to this colour, so the number below is what the
/// GPU will shade with and not a second opinion about it. `decal.rs`'s own
/// unit tests already reach for the same call.
fn luma(c: bevy::prelude::Color) -> f32 {
    let l = c.to_linear();
    0.2126 * l.red + 0.7152 * l.green + 0.0722 * l.blue
}

/// The `luma` cell of `MANIFEST.md`'s **prop-bind** table for one role.
///
/// **Read rather than typed, and that matters.** `tests/manifest_measured.rs`
/// pins every cell of this table to the shipped `.jpg`'s own pixels, so a
/// number taken from here is a measurement of the file the game loads, at one
/// remove, with a gate holding the remove shut. A literal copied into this
/// file would be the hand-kept mirror `CLAUDE.md` warns about twice.
///
/// Anchored on the table's HEADER and not on the first row that mentions the
/// role. `MANIFEST.md` has a sourcing table earlier in the file with a `rock`
/// row of its own, and the first draft of this gate read that one and found a
/// `✓` where it wanted a float. It failed loudly, which is the only reason
/// that was a five-minute mistake instead of a number nobody checked.
fn material_luma(role: &str) -> f32 {
    const HEADER: &str = "| role | linear mean rgb | luma | albedo sd | gain span |";
    let md = std::fs::read_to_string(MANIFEST).unwrap_or_else(|e| panic!("{MANIFEST}: {e}"));
    let table = md.split(HEADER).nth(1).unwrap_or_else(|| {
        panic!(
            "MANIFEST.md no longer has the prop-bind table this gate reads \
             (looked for {HEADER:?}). The table moved or was renamed; a gate \
             that cannot find its source fails rather than guessing."
        )
    });
    let needle = format!("| `{role}` |");
    let row = table
        .lines()
        // `split` leaves the header line's own empty remainder first, and a
        // `take_while` that meets it stops before the table starts.
        .skip_while(|l| !l.trim_start().starts_with('|'))
        .take_while(|l| l.trim_start().starts_with('|'))
        .find(|l| l.trim_start().starts_with(&needle))
        .unwrap_or_else(|| {
            panic!(
                "MANIFEST.md's prop-bind table has no row for `{role}`. This gate \
                 reads that table rather than carrying its own copy of the number; \
                 a missing row is a loud failure and never a skip."
            )
        });
    let cells: Vec<&str> = row.split('|').map(str::trim).collect();
    assert!(
        cells.len() >= 5,
        "`{role}`'s prop-bind row has {} cells; the luma column is the second \
         after the role. The table's shape moved and this gate cannot read it.",
        cells.len()
    );
    cells[3]
        .parse::<f32>()
        .unwrap_or_else(|e| panic!("`{role}`'s luma cell {:?} did not parse: {e}", cells[3]))
}

/// A mark lies ON the surface it marks, so Bevy's depth fade gives it full
/// alpha and shifts its UV by nothing.
///
/// # The arithmetic this reproduces, and why it is not a mirror
///
/// `bevy_pbr-0.18.1/src/decal/forward_decal.wgsl` is the whole contract:
///
/// ```wgsl
/// let delta_uv = normal_depth * Vt.xy * vec2(1.0, -1.0) / view_steepness;
/// let alpha    = saturate(1.0 - (normal_depth * inv_depth_fade_factor));
/// ```
///
/// `normal_depth` is the separation between the decal quad and the geometry
/// behind it, measured along the decal's own normal — which for a quad we
/// place ourselves is exactly the offset we placed it at. There is no bounds
/// discard in that shader and no projection volume; alpha is **maximal at
/// zero** and gone at `depth_fade_factor`.
///
/// So this is not a mirror of Bevy's source, it is the consequence of two
/// published constants of ours meeting one published formula of theirs. The
/// mutant is the line this gate was written for: restore
/// `MARK_LIFT_M = DEPTH_FADE_M * 0.5` and the first assert reads 0.5 against
/// a floor of 0.9, and the second walks the scuff 30 cm off a 22 cm quad.
#[test]
fn a_mark_sits_on_its_surface_so_bevys_fade_gives_it_full_alpha() {
    let alpha = (1.0 - MARK_LIFT_M / DEPTH_FADE_M).clamp(0.0, 1.0);
    assert!(
        alpha >= 0.9,
        "a placed mark blends at alpha {alpha:.3} (lift {MARK_LIFT_M} m against \
         a {DEPTH_FADE_M} m fade). Bevy's forward decal is MAXIMAL at zero \
         separation — an offset does not admit the quad to a projection volume, \
         because there is no volume. Every mark in the game is dimmed by this, \
         on every surface, and no other gate here can see it."
    );

    // The parallax half. `delta_uv` grows as `normal_depth * tan(theta)`, so a
    // lifted quad slides its own scuff sideways as the view goes off-normal.
    // 60 degrees is not an extreme: it is roughly a standing player looking at
    // the ground a couple of metres ahead.
    let worst = MARK_LIFT_M * 60f32.to_radians().tan();
    assert!(
        worst < SIZE_M * 0.5,
        "at 60 degrees off-normal a lift of {MARK_LIFT_M} m displaces the sample \
         by {worst:.3} m, and the quad's half-width is only {:.3} m — the mark \
         leaves the surface it is drawn on and the player sees nothing, from \
         exactly the angles people stand at.",
        SIZE_M * 0.5
    );
}

/// Every mark's tint stands off the material it lands on, so the mark is
/// visible against it.
///
/// # Why a ratio and not a fixed colour
///
/// A tint is a taste call and this gate does not make one — it only refuses a
/// tint that cannot be *seen*, which is arithmetic. `SURF_GROUND` and
/// `SURF_BUILT` both clear the floor comfortably and always did; the floor is
/// set below them and above the value that shipped.
///
/// This is `§0gc`'s trap recurring — a blade shaded exactly like the dirt it
/// stood in — and until now nothing in this repo could catch it. `SURF_WORLD`
/// shipped at linear luma 0.089 against `bark`'s measured 0.107: a ratio of
/// 1.20, a difference of about five sRGB code values, on a photograph whose
/// own albedo sd is 0.068. The mark was inside the texture's noise, and a
/// person looking at it would not have said "wrong colour", they would have
/// said "no decal" — which is exactly what happened.
///
/// Mutant: put `SURF_WORLD` back to `srgb(0.42, 0.31, 0.19)` and the `bark`
/// row reads 1.20 against a floor of 1.6.
#[test]
fn every_marks_tint_stands_off_the_material_it_lands_on() {
    // A mark may be darker or lighter than what it lands on — heartwood is
    // pale, turned earth is dark — so the test is on the RATIO, taken the way
    // round that is greater than one.
    const FLOOR: f32 = 1.6;

    // `twig` and not `wood`: `build.rs` enters every piece into the world as
    // twig, so it is the tier a mark actually lands on.
    //
    // ⚠ **The ground row is the weak one and it says so.** Terrain is a
    // four-way splat (grass · sand · litter · rock) and no single luma is
    // "the" ground; `rock` is the one of the four carrying a measured row in
    // this table. It is kept because the ground tint clears the floor against
    // it by an order of magnitude, so the ambiguity cannot change that row's
    // verdict — but this row would NOT catch a ground tint tuned to collide
    // with grass specifically, and nothing here should be read as saying it
    // would.
    let cases = [
        (SURF_GROUND, "ground", material_luma("rock")),
        (SURF_WORLD, "a trunk", material_luma("bark")),
        (SURF_BUILT, "a built piece", material_luma("twig")),
    ];

    let mut worst: Option<(&str, f32)> = None;
    for (surf, what, surface) in cases {
        let t = luma(tint(surf));
        let ratio = if t > surface {
            t / surface
        } else {
            surface / t
        };
        println!("  surf {surf} on {what:14} tint {t:.4}  surface {surface:.4}  ratio {ratio:.2}");
        if worst.is_none_or(|(_, w)| ratio < w) {
            worst = Some((what, ratio));
        }
        assert!(
            ratio >= FLOOR,
            "a mark on {what} draws at linear luma {t:.4} against a surface \
             measured at {surface:.4} — a ratio of {ratio:.2}, under the {FLOOR} \
             floor. That is not a mark, it is a slightly different shade of the \
             same thing, and the player reports it as no decal at all."
        );
    }
    let (what, ratio) = worst.expect("three cases");
    println!("  tightest: {what} at {ratio:.2}x (floor {FLOOR})");
}
