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
    /// Ambient occlusion, where the source published one.
    ///
    /// **`Option`, because only seven of the ten roles have a file** — the
    /// photogrammetry sets (grass, gravel, litter, metal, rock, sand, stone)
    /// ship `<role>_ao.jpg` and the three authored-surface sets (bark, wood,
    /// twig) do not. A missing map must be `None` and not a broken handle:
    /// `StandardMaterial::occlusion_texture` is itself an `Option`, and an
    /// unresolved handle in that slot samples as black, which would put every
    /// bark surface in full shadow.
    ///
    /// These seven files were **git-tracked, staged into every depot by
    /// `ci/depot.py`, and read by nothing** — 436 KB shipped to every player
    /// with `occlusion_texture` appearing zero times in `crates/`. `ART.md` §4
    /// names this exact term as the one scale a light rig cannot supply
    /// (*"Medium … `indirectDiffuse *= ao`, indirect only"*) and as the unblock
    /// for raising the ambient floor: fill lands everywhere, AO removes it only
    /// where the geometry occludes.
    pub ao: Option<Handle<Image>>,
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

/// Roles that ship an `<role>_ao.jpg`, which is not all of them.
///
/// **Derived from the tree by `tests/textures.rs`, not trusted from here.**
/// A hand-kept mirror of a directory listing is the drift `CLAUDE.md` names
/// twice — a role added to this list with no file loads a handle that samples
/// black, and a role with a file left off the list ships an unread texture
/// again, which is the bug this whole change is fixing.
pub const ROLES_WITH_AO: [&str; 7] = [
    "grass", "gravel", "litter", "metal", "rock", "sand", "stone",
];

impl MapSet {
    /// Load one role out of `assets/textures/<role>_{albedo,normal,rough}.jpg`,
    /// plus `_ao.jpg` for the roles that have one.
    pub fn load(assets: &AssetServer, role: &str) -> Self {
        Self {
            albedo: assets.load_with_settings(format!("textures/{role}_albedo.jpg"), tiling(true)),
            normal: assets.load_with_settings(format!("textures/{role}_normal.jpg"), tiling(false)),
            rough: assets.load_with_settings(format!("textures/{role}_rough.jpg"), tiling(false)),
            ao: ROLES_WITH_AO.contains(&role).then(|| {
                assets.load_with_settings(format!("textures/{role}_ao.jpg"), tiling(false))
            }),
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
///
/// ⚠ **Superseded 2026-08-15 and no longer loaded.** The splat material samples
/// each identity's own albedo and takes ITS luminance
/// (`render/ground_splat.rs`), so grass's pre-baked luminance field is now one
/// of four computed in the shader rather than the one field all four shared.
/// The constants stay because they are the cross-check that says the two
/// constructions agree — `GRAIN_GAIN[1]` measures 4.0292 off `grass_albedo.jpg`
/// against this 4.0579 off the baked file, and those have to be close or one of
/// them is wrong. The file still ships and `ground_where_the_green_goes.rs`
/// still gates it; nothing samples it. Deleting it is a separate call, because
/// a pre-baked luminance field is exactly what a cheaper LOD would want.
pub const GROUND_DETAIL: &str = "textures/ground_detail.jpg";
/// `1 / linear mean` of that field (0.2464), so the delivered mean is the
/// authored colour. Scalar, not per-channel — which is what makes the span 1.
pub const GROUND_DETAIL_GAIN: f32 = 4.0579;

/// The four ground identities' maps, in `terrain::splat`'s own order:
/// sand · grass · forest litter · rock. Only one of them can be sampled by a
/// `StandardMaterial` (it has one base-colour slot), which is exactly the
/// limitation a splat material exists to remove — see `RENDER.md`.
#[derive(Resource)]
pub struct GroundMaps {
    pub sand: MapSet,
    pub grass: MapSet,
    pub litter: MapSet,
    pub rock: MapSet,
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
