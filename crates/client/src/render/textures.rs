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
    /// The luminance field the ground actually samples today.
    pub detail: Handle<Image>,
}

pub fn load(mut commands: Commands, assets: Res<AssetServer>) {
    commands.insert_resource(GroundMaps {
        sand: MapSet::load(&assets, "sand"),
        grass: MapSet::load(&assets, "grass"),
        litter: MapSet::load(&assets, "litter"),
        rock: MapSet::load(&assets, "rock"),
        detail: assets.load_with_settings(GROUND_DETAIL, tiling(true)),
    });
}
