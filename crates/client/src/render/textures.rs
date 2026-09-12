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
//! **These load with no mip chain and `render/mipmap.rs` builds one.** Bevy
//! 0.18 generates none for an ordinary image format — `ImageLoaderSettings`
//! has no such setting — and a one-level photograph minified across the
//! island is the static that module exists to remove. The sampler below is
//! already right for a chain (`linear()` sets `mipmap_filter` to Linear, and
//! `anisotropy_clamp: 4` needs one to mean anything); what was missing was
//! the chain itself.
//!
//! **The budget that shaped these files is gone.** They were fetched at 1K
//! and re-encoded to fit a 12 MB *download* — a browser boot cost. A desktop
//! client pays it once from disk. Re-sourcing at 2K/4K is a later slice and
//! this module is where it lands; nothing else has to change.

use bevy::asset::{AssetServer, RenderAssetUsages};
use bevy::image::{
    ImageAddressMode, ImageLoaderSettings, ImageSampler, ImageSamplerDescriptor,
    TextureFormatPixelInfo,
};
use bevy::prelude::*;
use bevy::render::render_resource::{
    Extent3d, TextureDataOrder, TextureDimension, TextureViewDescriptor, TextureViewDimension,
};

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

/// A cutout ATLAS's sampler — clamped, not tiled.
///
/// **The opposite of [`tiling`] on the one axis that matters.** A tiling map
/// wants `Repeat` because its UVs run to the hundreds; an atlas addresses
/// cells inside 0..1 and `Repeat` on one would let a filter tap wrap from the
/// left edge of the sheet to the right, splicing two unrelated cards together
/// at the seam. `ClampToEdge` is also Bevy's default, so this exists for the
/// other half: **anisotropy**, which the default leaves at 1. A grass card is
/// seen at a grazing angle almost always — the camera stands 1.6 m up and the
/// cards are 34 cm — and at anisotropy 1 that is a smear.
pub fn atlas(srgb: bool) -> impl Fn(&mut ImageLoaderSettings) + Send + Sync + 'static {
    move |s: &mut ImageLoaderSettings| {
        s.is_srgb = srgb;
        s.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
            address_mode_u: ImageAddressMode::ClampToEdge,
            address_mode_v: ImageAddressMode::ClampToEdge,
            address_mode_w: ImageAddressMode::ClampToEdge,
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
/// over the four ground sources, only `rock` clears it (span 1.108; grass
/// 2.444, sand 2.066, litter 3.559). A luminance field has span **1.000 by
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
///
/// **This is the LOADING shape, not the binding one.** The ground material
/// binds [`GroundArrays`], which [`stack_ground`] builds out of these once
/// every map has its mip chain — and then removes this resource, because
/// sixteen source textures whose every texel is also in an array are ~56 MB
/// of VRAM held for nobody. `PropMaps` shares the `rock` handles, so those
/// three stay resident either way.
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
/// against `ALBEDO_LUMA_BAND = [0.05, 0.55]`, at the files' full 1024²;
/// `tests/manifest_measured.rs` re-measures every cell):
///
/// | role | linear mean rgb | luma | albedo sd | in band |
/// |---|---|---|---|---|
/// | rock | 0.250 0.245 0.226 | 0.245 | 0.1379 | ✓ |
/// | bark | 0.128 0.105 0.064 | 0.107 | 0.0676 | ✓ |
/// | wood | 0.161 0.139 0.112 | 0.141 | 0.0661 | ✓ |
/// | stone | 0.237 0.202 0.107 | 0.203 | 0.1138 | ✓ |
/// | metal | 0.230 0.227 0.228 | 0.228 | 0.0688 | ✓ |
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

// ── The ground's arrays ─────────────────────────────────────────────────────

/// The ground's sixteen maps as three `texture_2d_array`s — what
/// `ground_splat::GroundSplat` binds.
///
/// **Why arrays and not sixteen textures.** How many sampled textures a
/// fragment stage may hold is a hard limit with a floor of 16 on a downlevel
/// adapter, and WebGL2 IS that floor (`downlevel_webgl2_defaults`:
/// `max_sampled_textures_per_shader_stage = 16`). It is counted per STAGE and
/// summed over every bind group in the pipeline layout — the view's shadow and
/// environment maps, `StandardMaterial`'s six slots, and ours — so sixteen
/// ground textures put the material group alone at 22 and the browser refused
/// the pipeline before it drew a frame (2026-09-11). A layer costs no
/// binding: four albedo maps are one texture here where they were four, and
/// roughness and AO — same size, same format, same sampler — share one array
/// ([`AO_LAYER0`]), so the whole ground is THREE sampled textures. The
/// sampler argument in `ground_splat.rs` stands: it is still one for all of
/// them.
///
/// **The desktop draws the same texels.** A layer of a `2d_array` samples
/// exactly as the standalone texture did — same filter, same chain, same
/// anisotropy, same bytes ([`stack`] copies them) — so this is one shader for
/// both targets rather than a web variant, and the native before/after
/// capture is what says so (`findings/web-build-20260909.md` §15).
///
/// `RENDER_WORLD` only: an array's one job is to be uploaded, and the CPU
/// copy it was built from is the sources' — which are dropped with
/// [`GroundMaps`] once this exists.
#[derive(Resource, Clone)]
pub struct GroundArrays {
    /// `Rgba8UnormSrgb`, four layers in `terrain::splat`'s order.
    pub albedo: Handle<Image>,
    /// `Rgba8Unorm`, four layers.
    pub normal: Handle<Image>,
    /// `Rgba8Unorm`, eight layers: roughness at `0..4`, AO at
    /// [`AO_LAYER0`]`..8`, each in `terrain::splat`'s order.
    pub rough_ao: Handle<Image>,
}

/// The first AO layer of [`GroundArrays::rough_ao`]; roughness is the four
/// below it. The shader indexes these as literals and `tests/ground_tiling.rs`
/// holds them to this constant.
pub const AO_LAYER0: u32 = 4;
// The layer list `stack_ground` builds puts AO after four roughness layers;
// this is the one place the two are tied together.
const _: () = assert!(AO_LAYER0 == 4);

/// Whether a source image may be a layer yet: it has the chain
/// `mipmap::drain` gives it, or it is one that pass will never touch.
///
/// **Derived from the mip pass's own predicate, never from the event it
/// reacts to.** An image is in `Assets<Image>` one frame before its `Added`
/// event reaches `mipmap::enqueue`, so "loaded and not pending" is true for
/// exactly one frame on an image that is about to get a chain — and an array
/// built in that window carries single-level layers with every gate green.
/// `mipmap::wants` is the one test that cannot disagree with `drain`, because
/// `drain` calls it.
pub fn layer_ready(image: &Image) -> bool {
    image.texture_descriptor.mip_level_count > 1 || !super::mipmap::wants(image)
}

/// Why a set of images cannot be one array.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StackError {
    /// No layers at all.
    Empty,
    /// The layer named has no CPU-side data to copy.
    NoData(usize),
    /// The layer named is not a plain single-layer 2D image.
    NotPlane(usize),
    /// The layer named disagrees with layer 0 about the property named.
    Mismatch { layer: usize, what: &'static str },
    /// The layer named holds fewer or more bytes than its own descriptor says
    /// a chain of its size holds.
    Bytes {
        layer: usize,
        want: usize,
        got: usize,
    },
}

/// Bytes one layer's whole chain holds: `mips` levels of a `w × h` image at
/// `bpp` bytes a texel, level 0 included. `mipmap::chain_bytes` is this with
/// the full chain and RGBA8 filled in.
pub fn layer_bytes(w: u32, h: u32, mips: u32, bpp: usize) -> usize {
    (0..mips)
        .map(|lvl| ((w >> lvl).max(1) as usize) * ((h >> lvl).max(1) as usize) * bpp)
        .sum()
}

/// Stack `layers` into one `2d_array` image — chains and all, layer-major.
///
/// Every layer must agree with the first about size, format, mip count and
/// sampler, or the result would sample one identity at a different density or
/// through a different transfer function from the rest; a set that disagrees
/// is refused whole rather than resampled, because "fits" here is a fact about
/// the FILES (`assets/textures/MANIFEST.md`) and the fix is to re-source the
/// set, not to stretch one member of it.
///
/// **Layer-major is wgpu's default and it is what a concatenation of chains
/// IS**: `Layer0Mip0 Layer0Mip1 … Layer1Mip0 …`, exactly the order
/// `mipmap::chain` writes one image's levels in. Stated on the image rather
/// than left to the default, so a reader of `data` knows what they are
/// looking at.
pub fn stack(layers: &[&Image]) -> Result<Image, StackError> {
    let first = *layers.first().ok_or(StackError::Empty)?;
    let d0 = &first.texture_descriptor;
    let bpp = d0
        .format
        .pixel_size()
        .map_err(|_| StackError::NotPlane(0))?;
    let want = layer_bytes(d0.size.width, d0.size.height, d0.mip_level_count, bpp);
    let mut data = Vec::with_capacity(want * layers.len());
    for (i, layer) in layers.iter().enumerate() {
        let d = &layer.texture_descriptor;
        if d.dimension != TextureDimension::D2 || d.size.depth_or_array_layers != 1 {
            return Err(StackError::NotPlane(i));
        }
        if d.size.width != d0.size.width || d.size.height != d0.size.height {
            return Err(StackError::Mismatch {
                layer: i,
                what: "size",
            });
        }
        if d.format != d0.format {
            return Err(StackError::Mismatch {
                layer: i,
                what: "format",
            });
        }
        if d.mip_level_count != d0.mip_level_count {
            return Err(StackError::Mismatch {
                layer: i,
                what: "mip count",
            });
        }
        if layer.sampler != first.sampler {
            return Err(StackError::Mismatch {
                layer: i,
                what: "sampler",
            });
        }
        let bytes = layer.data.as_ref().ok_or(StackError::NoData(i))?;
        if bytes.len() != want {
            return Err(StackError::Bytes {
                layer: i,
                want,
                got: bytes.len(),
            });
        }
        data.extend_from_slice(bytes);
    }
    let mut image = Image::new_uninit(
        Extent3d {
            width: d0.size.width,
            height: d0.size.height,
            depth_or_array_layers: layers.len() as u32,
        },
        TextureDimension::D2,
        d0.format,
        RenderAssetUsages::RENDER_WORLD,
    );
    // The count and the buffer are set together, as `mipmap::drain` sets them:
    // `Image::new` checks `data` against level 0 alone and would refuse a
    // chain.
    image.texture_descriptor.mip_level_count = d0.mip_level_count;
    image.data = Some(data);
    image.data_order = TextureDataOrder::LayerMajor;
    image.sampler = first.sampler.clone();
    // Without this the view is created `D2` over a texture with layers, and
    // the bind group refuses it at draw — loudly on desktop, and on WebGL2 as
    // a texture that silently samples nothing.
    image.texture_view_descriptor = Some(TextureViewDescriptor {
        dimension: Some(TextureViewDimension::D2Array),
        ..Default::default()
    });
    Ok(image)
}

/// One family's slot in [`Stacking`], its name for the panic message, and the
/// handles of its layers in order.
type Family<'a> = (
    &'a mut Option<Handle<Image>>,
    &'static str,
    &'a [&'a Handle<Image>],
);

/// What [`stack_ground`] has built so far. `Local` state: three slots, filled
/// one a frame.
#[derive(Default)]
pub struct Stacking {
    albedo: Option<Handle<Image>>,
    normal: Option<Handle<Image>>,
    rough_ao: Option<Handle<Image>>,
}

/// Stack one family if all its layers are ready. `None` while any is not.
///
/// # Panics
///
/// If the layers are all loaded and cannot be one array — a size, format or
/// sampler that disagrees within a family is a file swap that broke the set,
/// and drawing the island untextured over it would look like a lighting bug.
/// Saying so at boot is the same posture `GroundSplat::new` took for a missing
/// AO map.
fn try_stack(
    images: &mut Assets<Image>,
    family: &str,
    layers: &[&Handle<Image>],
) -> Option<Handle<Image>> {
    // The wait costs no allocation: only a frame that builds collects.
    if !layers
        .iter()
        .all(|h| images.get(*h).is_some_and(layer_ready))
    {
        return None;
    }
    let refs: Vec<&Image> = layers.iter().filter_map(|h| images.get(*h)).collect();
    let image = stack(&refs).unwrap_or_else(|e| {
        panic!(
            "the ground's {family} maps cannot be one texture array: {e:?}. Every layer must share one size, format, mip count and sampler — re-source the SET (assets/textures/MANIFEST.md), never one identity of it"
        )
    });
    Some(images.add(image))
}

/// Build [`GroundArrays`] out of [`GroundMaps`] — one family a frame, once
/// every one of a family's maps has had its mip pass — then drop the sources.
///
/// Runs after `mipmap::drain` in the same frame, so a chain finished this
/// frame is stacked this frame. **One family a frame, deliberately**: the
/// albedo array alone is ~22 MB of texels copied, and `CLAUDE.md`'s stream-in
/// rule is that a frame pays for one of those, never three. Every frame after
/// the sources are dropped is one `Option` read.
pub fn stack_ground(
    mut commands: Commands,
    maps: Option<Res<GroundMaps>>,
    mut images: ResMut<Assets<Image>>,
    mut stacking: Local<Stacking>,
) {
    let Some(maps) = maps else {
        return;
    };
    let sets = [&maps.sand, &maps.grass, &maps.litter, &maps.rock];
    // `expect`, not `unwrap_or_default`, and the reason is unchanged from when
    // this check lived on the material: an unresolved handle samples as BLACK,
    // and a black occlusion layer puts the whole island in shadow — a failure
    // that reads as a lighting bug rather than a missing file. Every ground
    // role is in `ROLES_WITH_AO`, so this fires only when that list and
    // `assets/textures/` have drifted apart, and boot is the place to say so.
    let ao = |k: usize| -> &Handle<Image> {
        sets[k].ao.as_ref().unwrap_or_else(|| {
            panic!(
                "ground identity #{k} has no AO map — every ground role must be in textures::ROLES_WITH_AO with a matching assets/textures/<role>_ao.jpg, or the splat shader samples an unresolved layer as BLACK and the island draws in full shadow"
            )
        })
    };
    let albedo = [
        &sets[0].albedo,
        &sets[1].albedo,
        &sets[2].albedo,
        &sets[3].albedo,
    ];
    let normal = [
        &sets[0].normal,
        &sets[1].normal,
        &sets[2].normal,
        &sets[3].normal,
    ];
    // Roughness first, AO from `AO_LAYER0` — the layer order the shader
    // indexes by literal.
    let rough_ao = [
        &sets[0].rough,
        &sets[1].rough,
        &sets[2].rough,
        &sets[3].rough,
        ao(0),
        ao(1),
        ao(2),
        ao(3),
    ];

    let s = &mut *stacking;
    let families: [Family; 3] = [
        (&mut s.albedo, "albedo", &albedo),
        (&mut s.normal, "normal", &normal),
        (&mut s.rough_ao, "rough/AO", &rough_ao),
    ];
    for (slot, family, layers) in families {
        if slot.is_some() {
            continue;
        }
        let Some(handle) = try_stack(&mut images, family, layers) else {
            // Not every layer has its chain yet; nothing later in the list is
            // tried, so the order is fixed and a family is never skipped.
            return;
        };
        *slot = Some(handle);
        // One a frame.
        break;
    }
    let (Some(albedo), Some(normal), Some(rough_ao)) =
        (s.albedo.clone(), s.normal.clone(), s.rough_ao.clone())
    else {
        return;
    };
    commands.insert_resource(GroundArrays {
        albedo,
        normal,
        rough_ao,
    });
    commands.remove_resource::<GroundMaps>();
}
