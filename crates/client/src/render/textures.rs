//! The photograph. `assets/textures/` — 9 CC0 PBR sets, 34 files, already
//! manifested and already *measured* — loaded for the first time by the
//! native client.
//!
//! **Why this matters more than any shader.** `ART.md` §3's last row is
//! near-ground neighbour contrast: the reference frames run 5.4–6.3 luma
//! between adjacent pixels on close ground, and our captures run 2.5. The
//! population (grass geometry) took that number from 0.26 to 2.5; the rest of
//! it is the **near-field grain under 5 cm**, which is measured
//! high-frequency detail a noise field cannot encode. That is §7's whole
//! argument for sourcing real maps, and the operator's call behind it:
//! *"if its CC0 im fine to pull in whatever helps us."*
//!
//! **Hybrid, not replacement** (§7). The maps supply base albedo, normal and
//! roughness; everything already built stays as the variation layer — the
//! splat weights the mesh carries in its vertex colours still choose the
//! identity, still ramp between them, and still multiply the photograph.
//!
//! **The budget that shaped these files is gone.** They were fetched at 1K
//! and re-encoded to fit a 12 MB *download* — a browser boot cost. A desktop
//! client pays it once from disk. Re-sourcing at 2K/4K is a later slice and
//! this module is where it lands; nothing else has to change.

use bevy::asset::AssetServer;
use bevy::image::{ImageAddressMode, ImageLoaderSettings, ImageSampler, ImageSamplerDescriptor};
use bevy::prelude::*;

/// One material's three maps, as the ground and the props want them.
///
/// `Default` is three unresolved handles — how the headless gates build a
/// `PropMaps` without an asset server or a file on disk. A material clones the
/// handle either way, so the path under test is the same one.
#[derive(Default)]
pub struct MapSet {
    pub albedo: Handle<Image>,
    pub normal: Handle<Image>,
    pub rough: Handle<Image>,
}

/// A tiling sampler. **Every map here is tiled and the default is not.**
/// Bevy's default address mode is ClampToEdge, and a clamped map on a 64 m
/// terrain chunk stretches one texel across the whole chunk — which reads as
/// no texture at all rather than as an error, so nothing would say so.
fn tiling(srgb: bool) -> impl Fn(&mut ImageLoaderSettings) + Send + Sync + 'static {
    move |s: &mut ImageLoaderSettings| {
        // `is_srgb` is not cosmetic: an albedo is authored in sRGB and a
        // normal or roughness map is raw data. Loading a normal map as sRGB
        // bends every normal toward the surface and the lighting goes subtly,
        // unfixably wrong — the class of bug that looks like "the sun is in
        // the wrong place".
        s.is_srgb = srgb;
        s.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
            address_mode_u: ImageAddressMode::Repeat,
            address_mode_v: ImageAddressMode::Repeat,
            address_mode_w: ImageAddressMode::Repeat,
            // The maps are 1K over a 4 m tile and the camera stands 1.6 m up,
            // so the ground is seen at a grazing angle almost everywhere.
            // Anisotropy is what stops that becoming a smear at the horizon;
            // `ART.md` §7 registers the browser's ceiling as
            // `BASE_ANISOTROPY_MAX = 4` and this holds to it.
            anisotropy_clamp: 4,
            ..ImageSamplerDescriptor::linear()
        });
    }
}

impl MapSet {
    /// Load one role out of `assets/textures/<role>_{albedo,normal,rough}.jpg`.
    pub fn load(assets: &AssetServer, role: &str) -> Self {
        Self {
            albedo: assets.load_with_settings(format!("textures/{role}_albedo.jpg"), tiling(true)),
            normal: assets.load_with_settings(format!("textures/{role}_normal.jpg"), tiling(false)),
            rough: assets.load_with_settings(format!("textures/{role}_rough.jpg"), tiling(false)),
        }
    }
}

/// The ground's detail field: `ground_detail.jpg`, a LUMINANCE-only map
/// derived from `grass_albedo.jpg` (CC0, Poly Haven `forrest_ground_01`).
///
/// **Why a derived greyscale rather than the source's own colour.** `ART.md`
/// §7 states the construction: a modifier that must set a colour multiplies
/// the surface's own **mean-1 luminance field**, so the authored colour is
/// the delivered mean and the relief's light and shade survive. It also
/// bounds the alternative — a per-channel gain placing a source's mean may
/// not stretch that source's colour deviation by more than ×1 — and measured
/// over the four ground sources, only `rock` clears it (span 1.054; grass
/// 2.454, sand 2.073, litter 3.586). A luminance field has span **1.000 by
/// construction**, because every channel is the same channel.
///
/// So this is not a workaround for the rule; it is what the rule asks for.
/// The chroma is entirely the splat's, and the photograph contributes exactly
/// the thing a noise field cannot encode: measured high-frequency relief.
///
/// Derived, not edited: the source file stays pristine and swappable, and
/// `assets/textures/MANIFEST.md` carries the row.
pub const GROUND_DETAIL: &str = "textures/ground_detail.jpg";
/// `1 / linear mean` of that field (0.2464), so the delivered mean is the
/// authored colour. Scalar, not per-channel — which is what makes the span 1.
pub const GROUND_DETAIL_GAIN: f32 = 4.0579;

/// Each ground identity's own `1 / mean linear luma`, in `terrain::splat`'s
/// order: sand · grass · forest litter · rock.
///
/// **This is [`GROUND_DETAIL_GAIN`]'s construction, once per identity, and it
/// is the whole reason the splat material does not reintroduce the failure
/// `ART.md` §7 names.** The obvious way to give granite stone's surface is to
/// sample `rock_albedo.jpg` into the base colour — and that multiplies the
/// authored splat colour by the photograph's OWN chroma, which is exactly the
/// "modifier that must set a colour multiplies a mean-1 luminance field, not
/// its chroma" rule. §7 bounds the alternative (a per-channel gain placing the
/// source's mean) at a ×1 stretch of the source's colour deviation, and
/// measured over these four sources only `rock` clears it — grass 2.454, sand
/// 2.073, litter 3.586 against rock's 1.054. Three of the four are inadmissible
/// as colour.
///
/// So each identity contributes its **luminance only**, divided by its own
/// mean, multiplied by that identity's authored [`GROUND_ALBEDO`] row. Span is
/// 1.000 per identity by construction, because every channel is the same
/// channel. What the four maps buy is what a vertex hash cannot encode —
/// measured high-frequency relief, four DIFFERENT reliefs where there was one
/// — and what stays authored is every colour decision
/// `ground_identity.rs` gates.
///
/// [`GROUND_ALBEDO`]: super::terrain_mesh::GROUND_ALBEDO
///
/// Measured off the shipped files by `tests/ground_maps.rs`, which fails if a
/// re-encode moves a mean without moving the constant:
///
/// | identity | mean linear luma | gain |
/// |---|---|---|
/// | sand | 0.18048 | 5.5407 |
/// | grass | 0.24790 | 4.0339 |
/// | forest litter | 0.10300 | 9.7088 |
/// | rock | 0.26934 | 3.7128 |
///
/// **Cross-checked against a second source rather than trusted**, which is the
/// discipline `CLAUDE.md`'s sweep-window trap asks for: the per-CHANNEL linear
/// means recorded independently in [`GROUND_ALBEDO`]'s own gain-span table
/// reduce to Rec.709 luma of 0.18085 · 0.24855 · 0.10317 · 0.26913 — every one
/// within 0.3% of the column above, measured by different code on a different
/// day for a different purpose. A third agreement falls out for free: grass's
/// 0.24790 against [`GROUND_DETAIL_GAIN`]'s field mean of 0.2464, which is
/// 0.6% and should be close, because `ground_detail.jpg` was derived from
/// `grass_albedo.jpg`.
pub const GROUND_MAP_GAIN: [f32; 4] = [5.5407, 4.0339, 9.7088, 3.7128];

/// The four ground identities' maps, in `terrain::splat`'s own order:
/// sand · grass · forest litter · rock.
///
/// **All four albedos are sampled since 2026-08-15** — `terrain_mesh::
/// GroundSplat` blends them by the splat weights, each as its own mean-1
/// luminance field. This doc used to say only one of them could be, which was
/// true of a `StandardMaterial` and is the limitation the splat material
/// exists to remove.
///
/// The `normal` and `rough` halves are still one-for-all: the material binds
/// `grass.normal` for every identity and a single `perceptual_roughness`.
/// `NOW.md` §0gp carries both.
#[derive(Resource)]
pub struct GroundMaps {
    pub sand: MapSet,
    pub grass: MapSet,
    pub litter: MapSet,
    pub rock: MapSet,
    /// The single luminance field the ground sampled before the splat material.
    /// **Still shipped and still gated** (`ground_where_the_green_goes.rs`), and
    /// no longer bound by anything — see `terrain_mesh::ground_material`.
    pub detail: Handle<Image>,
}

/// The prop identities' maps — the five sets that were fetched, manifested and
/// then never loaded by anything.
///
/// **These bind differently from the ground's, and the difference is the whole
/// reason the ground could only take one map.** `terrain_mesh` has four
/// identities blending into one `base_color_texture` slot, so its photograph
/// has to be a luminance field and the colour has to stay the splat's. A prop
/// has exactly one identity — granite is granite, bark is bark — so the
/// photograph IS the colour, `base_color` stays white, and no mean-placing
/// gain is applied at all.
///
/// That is not a shortcut around `ART.md` §7's deviation rule; it is the case
/// the rule is vacuous in. The rule bounds how far a per-channel gain may
/// stretch a source's colour deviation, and a gain of exactly 1 stretches it
/// by exactly 1. Measured off the shipped files (linear means, Rec.709 luma,
/// against `ALBEDO_LUMA_BAND = [0.05, 0.55]`):
///
/// | role | linear mean rgb | luma | albedo sd | in band |
/// |---|---|---|---|---|
/// | rock | 0.273 0.269 0.259 | 0.269 | 0.0933 | ✓ |
/// | bark | 0.128 0.105 0.064 | 0.107 | 0.0676 | ✓ |
/// | wood | 0.161 0.139 0.112 | 0.142 | 0.0661 | ✓ |
/// | stone | 0.237 0.202 0.106 | 0.203 | 0.1139 | ✓ |
/// | metal | 0.230 0.228 0.228 | 0.228 | 0.0689 | ✓ |
///
/// All five clear the band off the raw file, so every one of them ships its
/// colour whole. The per-instance and per-part variation that used to BE the
/// colour becomes a mean-1 field multiplying it (`props::tint1`).
#[derive(Resource)]
pub struct PropMaps {
    pub rock: MapSet,
    pub bark: MapSet,
    pub wood: MapSet,
    pub stone: MapSet,
    pub metal: MapSet,
}

pub fn load(mut commands: Commands, assets: Res<AssetServer>) {
    commands.insert_resource(GroundMaps {
        sand: MapSet::load(&assets, "sand"),
        grass: MapSet::load(&assets, "grass"),
        litter: MapSet::load(&assets, "litter"),
        rock: MapSet::load(&assets, "rock"),
        detail: assets.load_with_settings(GROUND_DETAIL, tiling(true)),
    });
    // Same paths as the ground's `rock`, and therefore the same handle: the
    // asset server keys on path plus settings, so naming it twice costs one
    // load and one residency, not two.
    commands.insert_resource(PropMaps {
        rock: MapSet::load(&assets, "rock"),
        bark: MapSet::load(&assets, "bark"),
        wood: MapSet::load(&assets, "wood"),
        stone: MapSet::load(&assets, "stone"),
        metal: MapSet::load(&assets, "metal"),
    });
}
