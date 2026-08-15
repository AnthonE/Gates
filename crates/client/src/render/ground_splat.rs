//! The ground's splat material: four identities, four photographs.
//!
//! **The first WGSL in the tree** (`RENDER.md` §8 / R4). The shader is
//! `assets/shaders/ground_splat.wgsl` and its header carries the visual
//! argument; this module is the binding and the numbers.
//!
//! **Why an extension rather than a whole material.** Everything about the
//! ground except the base colour, the relief and the roughness is
//! `StandardMaterial`'s job and is already right — lighting, fog, the shadow
//! path, tonemapping. `ExtendedMaterial` keeps all of it and overrides the three
//! terms this slice is about.
//!
//! **It deliberately does NOT declare `#[bindless]`, and that is the point.**
//! `terrain_mesh.rs` recorded the blocker: in 0.18 `StandardMaterial` is
//! `#[bindless(index_table(range(0..31)))]`, so four maps blended by the splat
//! weights "needs a custom material". An extension that is not itself bindless
//! forces the whole `ExtendedMaterial` non-bindless, which retires that blocker
//! without touching Bevy.

use bevy::asset::Asset;
use bevy::pbr::{ExtendedMaterial, MaterialExtension, StandardMaterial};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;

use super::terrain_mesh::{ALBEDO_LUMA_FLOOR, GROUND_ALBEDO, WET_SATURATION, WET_VALUE};
use super::textures::GroundMaps;

/// The shader, resolved against the asset root `bin/gates.rs` sets.
pub const SHADER: &str = "shaders/ground_splat.wgsl";

/// The ground material as the world actually uses it.
pub type GroundMaterial = ExtendedMaterial<StandardMaterial, GroundSplat>;

/// Per identity, `1 / linear-luma mean` of its albedo map — sand · grass ·
/// litter · rock, in `terrain::splat`'s order.
///
/// **Measured off the shipped files, not read off a doc table** (2026-08-15,
/// Rec.709 luma of the sRGB-decoded linear means). Two cross-checks that the
/// method is the repo's own: the per-channel linear means reproduce
/// `terrain_mesh.rs`'s table to every digit it prints (grass 0.2910 0.2485
/// 0.1186 against its "0.291 0.249 0.119"), and `grass`'s gain here — 4.0292 —
/// lands within 0.7% of [`super::textures::GROUND_DETAIL_GAIN`] (4.0579), which
/// is the independently-derived gain of `ground_detail.jpg`. It has to: that
/// file IS grass's luminance, baked. The agreement is the check.
///
/// `tests/ground_splat.rs` re-measures all four off the files and fails on
/// drift, so a swapped source cannot silently keep the old gain.
pub const GRAIN_GAIN: [f32; 4] = [5.5398, 4.0292, 9.6954, 3.7128];

/// Per identity's perceptual roughness — sand · grass · litter · rock.
///
/// **The one number here that is not measured**, and it is a knob: it replaces
/// the single `perceptual_roughness: 0.92` every identity used to share, which
/// is the other half of "granite has stone's value and not stone's surface".
/// Ordered by how much a wet-looking specular lobe belongs on each surface —
/// damp sand is the smoothest thing on the island and dry needle litter the
/// roughest. `DECISIONS.md` §open carries it as "ground identity roughness v0";
/// the rough MAPS are the follow-up that makes these unnecessary.
pub const IDENTITY_ROUGH: [f32; 4] = [0.86, 0.93, 0.96, 0.88];

/// **How soft the height blend is.** A larger number is a wider contested band
/// and a wash; a smaller one is a sharper seam and, past about 0.1, visible
/// bubble-shaped regions along every boundary. `NOW.md` §0gm scouted 0.2 out of
/// the skills and it is kept as spoken. `DECISIONS.md` §open, "ground splat
/// blend depth v0".
pub const BLEND_DEPTH: f32 = 0.2;

/// How far a map's own relief may move the blend, against weights that run
/// 0..1. With the shader's clamp this caps the height's vote at ±0.15, so it
/// can only arbitrate a band where two weights are already within 0.3.
///
/// **Measured as a no-op** and kept as insurance: `splat_from` is near-binary
/// (92.2% of samples over 0.8), so the contested band is a sliver of the
/// island. `DECISIONS.md` §open, "ground splat material v0".
pub const HEIGHT_INFLUENCE: f32 = 0.15;

/// The floor under a tangent-space normal's `z` before it becomes a gradient.
///
/// **1e-4 is not a safe floor for a JPEG normal map.** These sources are
/// `.jpg`, so the blue channel carries compression noise and dips below 0.5 at
/// block edges; `n.xy / 1e-4` then returns a gradient in the thousands, the
/// blended normal points nearly sideways, and the diffuse term collapses. 0.2
/// caps the slope one texel can assert at ~5:1 — steeper than any real surface
/// detail, and finite.
pub const NORMAL_Z_FLOOR: f32 = 0.2;

/// The uniform, laid out to match `GroundSplat` in the shader.
#[derive(Clone, Default, ShaderType, Debug)]
pub struct GroundSplatParams {
    /// `xyz` the identity's authored linear albedo, `w` its roughness.
    pub identity: [Vec4; 4],
    pub gain: Vec4,
    /// x = `WET_VALUE`, y = `WET_SATURATION`, z = `ALBEDO_LUMA_FLOOR`,
    /// w = [`BLEND_DEPTH`].
    pub tune: Vec4,
    /// x = [`HEIGHT_INFLUENCE`], y = [`NORMAL_Z_FLOOR`], zw reserved.
    ///
    /// **These are here rather than as WGSL `const`s because a knob that lives
    /// only in a shader is a knob nothing can cross-check.** `ci/gates.sh`'s
    /// knob registry scans `.rs/.js/.mjs` and refused the `DECISIONS.md` rows
    /// for both until they moved — which is the registry working exactly as
    /// intended, and the reason to pass them through the uniform.
    pub blend: Vec4,
}

impl GroundSplatParams {
    /// Built from the same constants the CPU-side reference arithmetic reads, so
    /// the shader and `terrain_mesh::vertex_color` cannot disagree about a
    /// number without disagreeing about its source.
    pub fn new() -> Self {
        let mut identity = [Vec4::ZERO; 4];
        for k in 0..4 {
            let a = GROUND_ALBEDO[k];
            identity[k] = Vec4::new(a[0], a[1], a[2], IDENTITY_ROUGH[k]);
        }
        Self {
            identity,
            gain: Vec4::from_array(GRAIN_GAIN),
            tune: Vec4::new(WET_VALUE, WET_SATURATION, ALBEDO_LUMA_FLOOR, BLEND_DEPTH),
            blend: Vec4::new(HEIGHT_INFLUENCE, NORMAL_Z_FLOOR, 0.0, 0.0),
        }
    }
}

/// Four albedo maps, four normal maps and one sampler.
///
/// **One sampler for eight textures.** Each map wants the identical tiling and
/// anisotropy descriptor `textures::tiling` builds, and a sampler each would put
/// this bind group at 16 in the fragment stage before `StandardMaterial`'s own
/// are counted — over the 16 the downlevel limit guarantees. The textures are 8
/// against `StandardMaterial`'s ~6 for the same reason: the roughness maps are a
/// follow-up, not part of this slice.
#[derive(Asset, AsBindGroup, TypePath, Clone)]
pub struct GroundSplat {
    #[uniform(100)]
    pub params: GroundSplatParams,
    #[texture(101)]
    pub albedo_sand: Handle<Image>,
    #[texture(102)]
    pub albedo_grass: Handle<Image>,
    #[texture(103)]
    pub albedo_litter: Handle<Image>,
    /// **This one field carries the shared sampler too**, at binding 109. The
    /// derive refuses a `sampler` attribute with no `texture` beside it, so the
    /// one sampler all eight maps use has to hang off one of them; which one is
    /// arbitrary and stable, and they are loaded with an identical descriptor.
    #[texture(104)]
    #[sampler(109)]
    pub albedo_rock: Handle<Image>,
    #[texture(105)]
    pub normal_sand: Handle<Image>,
    #[texture(106)]
    pub normal_grass: Handle<Image>,
    #[texture(107)]
    pub normal_litter: Handle<Image>,
    #[texture(108)]
    pub normal_rock: Handle<Image>,
}

impl GroundSplat {
    /// Bind the four ground identities' maps, in `terrain::splat`'s order.
    pub fn new(maps: &GroundMaps) -> Self {
        Self {
            params: GroundSplatParams::new(),
            albedo_sand: maps.sand.albedo.clone(),
            albedo_grass: maps.grass.albedo.clone(),
            albedo_litter: maps.litter.albedo.clone(),
            albedo_rock: maps.rock.albedo.clone(),
            normal_sand: maps.sand.normal.clone(),
            normal_grass: maps.grass.normal.clone(),
            normal_litter: maps.litter.normal.clone(),
            normal_rock: maps.rock.normal.clone(),
        }
    }
}

impl MaterialExtension for GroundSplat {
    fn fragment_shader() -> ShaderRef {
        SHADER.into()
    }

    /// The deferred path would need the same override and does not have it, so
    /// it is left to the forward path this client already runs on.
    fn deferred_fragment_shader() -> ShaderRef {
        ShaderRef::Default
    }
}
