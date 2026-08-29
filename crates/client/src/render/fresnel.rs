//! The one place a `reflectance` is decided, and the arithmetic behind it.
//!
//! **Every material in this client was authored 8–70× below physical, and the
//! cause is that Bevy's `reflectance` is not a 0..1 "how shiny" slider.** It is
//! a remap: `F0 = 0.16 · reflectance²`, so the DEFAULT of 0.5 is the ordinary
//! dielectric 4% and everything below it is darker than any real dielectric
//! surface. Authored against the slider reading, the island came out at
//!
//! | surface | was | F0 | against 4% |
//! |---|---|---|---|
//! | ground | 0.18 | 0.52% | 7.7× under |
//! | bark | 0.08 | 0.10% | 39× under |
//! | foliage | 0.10 | 0.16% | 25× under |
//! | clutter | 0.12 | 0.23% | 17× under |
//! | wood | 0.14 | 0.31% | 13× under |
//! | rock | 0.20 | 0.64% | 6.3× under |
//! | stone | 0.26 | 1.08% | 3.7× under |
//! | mobs | 0.06 | 0.06% | 69× under |
//!
//! **The proof that this is a misreading and not a style is in the tree**:
//! `water.rs` derives its own from the physics in a comment —
//! `reflectance = sqrt(0.02/0.16) = 0.354` — and notes that the plane it
//! replaced shipped 0.55, *"nearly two and a half times too specular"*. One
//! module knew the formula; none of the others did.
//!
//! **The consequence was measured before the cause was found.**
//! `render/ground_splat.rs` wired four per-texel roughness maps and recorded
//! the result as a null: near-band contrast −0.4%, luma −0.1%, both inside the
//! harness's own run-to-run spread. Roughness shapes a specular lobe and
//! nothing else, so a map redistributing 1/8 of the energy a real surface puts
//! there has nothing to shape. `DECISIONS.md` §open ("ground specular v0") has
//! the finding and names this owner.
//!
//! ## Why a module rather than eleven corrected literals
//!
//! Because eleven literals is how it happened. A `reflectance:` at a material's
//! own site is a number with no unit next to it, and the next material added
//! copies its neighbour. These constants carry the F0 they encode in their
//! name and their doc, `tests/fresnel.rs` holds each equal to
//! [`reflectance_for`] applied to that F0, and a site that wants something else
//! has to say which physical surface it is claiming to be.

/// Bevy's mapping from a `StandardMaterial::reflectance` to the normal-incidence
/// specular reflectance it actually delivers.
///
/// `bevy_pbr`'s `calculate_F0` is
/// `0.16 · reflectance² · (1 − metallic) + base_color · metallic`; this is the
/// dielectric half, which is the half every surface on this island uses.
pub fn f0_of(reflectance: f32) -> f32 {
    0.16 * reflectance * reflectance
}

/// The inverse: the `reflectance` to author for a wanted F0.
pub fn reflectance_for(f0: f32) -> f32 {
    (f0 / 0.16).sqrt()
}

/// Ordinary dielectrics — soil, stone, bark, wood, leaf, cloth, painted metal.
///
/// 4% is the value every real-time PBR reference converges on for "a dielectric
/// with nothing special about it" (Lagarde & de Rousiers, *Moving Frostbite to
/// PBR* §3.2; Filament's material model). It is also Bevy's own default, which
/// is worth stating plainly: **the correct value for almost everything here was
/// what you get by not writing the field at all.**
pub const DIELECTRIC: f32 = 0.5;

/// Skin, hide and flesh — the animals and the player.
///
/// 2.8% rather than 4%: skin is the standard exception in every reference above,
/// and a pig lit like wet granite is the failure this avoids.
pub const FLESH: f32 = 0.418_330_1;

/// Water. **Already correct in `water.rs` before this module existed** and
/// restated here so the sea is inside the one table rather than beside it —
/// `WATER_REFLECTANCE` keeps its own derivation and `tests/fresnel.rs` holds
/// the two equal.
pub const WATER: f32 = 0.353_553_4;

/// Fresh unpainted metal read through a dielectric lobe.
///
/// The ore nodes and the barrel are `metallic` well below 1, so part of their
/// specular still comes through this term; a metal that also carries a
/// dielectric F0 below its neighbours reads as plastic. Kept at
/// [`DIELECTRIC`]'s value rather than raised: above 4% the dielectric lobe
/// starts standing in for metalness the `metallic` channel should be carrying,
/// which is the same category error in the other direction.
pub const METAL_DIELECTRIC: f32 = DIELECTRIC;
