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
    Extent3d, TextureDataOrder, TextureDimension, TextureFormat, TextureViewDescriptor,
    TextureViewDimension,
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
    /// **`Option`, because only eight roles have a file** — the
    /// photogrammetry sets (concrete, grass, gravel, litter, metal, rock,
    /// sand, stone) ship `<role>_ao.jpg` and the three authored-surface sets
    /// (bark, wood, twig) do not. A missing map must be `None` and not a broken handle:
    /// `StandardMaterial::occlusion_texture` is itself an `Option`, and an
    /// unresolved handle in that slot samples as black, which would put every
    /// bark surface in full shadow.
    ///
    /// The original seven files were **git-tracked, staged into every depot by
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
pub const ROLES_WITH_AO: [&str; 9] = [
    "grass",
    "gravel",
    "litter",
    "metal",
    "rock",
    "sand",
    "stone",
    "concrete",
    "aggregate",
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
    /// The road's aggregate (`Gravel004`), the arrays' fifth layer. Not an
    /// identity — no splat weight reaches it; the pavement and the unpaved
    /// branch sample it at `ground_splat::ROAD_AGGREGATE_TILE_M`. It was the
    /// `rock` identity until 2026-09-24, when the summits took a slab source
    /// and the road kept its grit.
    pub aggregate: MapSet,
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
/// | rock | 0.101 0.101 0.094 | 0.100 | 0.0368 | ✓ |
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
    /// The broadleaf's trunk: a real birch bark (TextureCan `wood_0027`, CC0),
    /// where it used to be `bark` brightened ×2.6.
    pub birch: MapSet,
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
        aggregate: MapSet::load(&assets, "aggregate"),
    });
    // Same paths as the ground's `rock`, and therefore the same handle: the
    // asset server keys on path plus settings, so naming it twice costs one
    // load and one residency, not two.
    commands.insert_resource(PropMaps {
        rock: MapSet::load(&assets, "rock"),
        bark: MapSet::load(&assets, "bark"),
        birch: MapSet::load(&assets, "birch"),
        wood: MapSet::load(&assets, "wood"),
        stone: MapSet::load(&assets, "stone"),
        metal: MapSet::load(&assets, "metal"),
    });
}

// ── The ground's arrays ─────────────────────────────────────────────────────

/// The ground's twenty maps as two `texture_2d_array`s — what
/// `ground_splat::GroundSplat` binds.
///
/// **Why arrays and not twenty textures.** How many sampled textures a
/// fragment stage may hold is a hard limit with a floor of 16, counted per
/// STAGE and summed over every bind group in the pipeline layout — the view's
/// shadow and environment maps, `StandardMaterial`'s six slots, and ours.
/// WebGL2 IS that floor (`downlevel_webgl2_defaults`), and sixteen ground
/// textures put the material group alone at 22: the browser refused the
/// pipeline before it drew a frame (2026-09-11). A layer costs no binding.
/// WebGPU guarantees the same 16 and the atmosphere's transmittance LUT takes
/// one of them, so three arrays was 17 there
/// (`render::quality::FRAGMENT_TEXTURES`); the ground is now TWO sampled
/// textures — the sRGB albedo, and every linear map in one
/// [`GroundArrays::data`], roughness and AO packed as two channels of one
/// layer ([`pack_rg`]). The sampler argument in `ground_splat.rs` stands: one
/// for all of them.
///
/// **The desktop draws the same texels.** A layer of a `2d_array` samples
/// exactly as the standalone texture did — same filter, same chain, same
/// anisotropy, same bytes ([`stack`] copies them) — so this is one shader for
/// both targets rather than a web variant. Packing is per channel and so
/// exact too. The one exception is bounded and stated at [`lift`]: roughness
/// and AO ship at 512² beside 1024² normals and gain a top level to share
/// their array, which only a tap that magnified them ever reads.
///
/// **VRAM**, chains included: albedo 28.0 MB, data 55.9 MB — 83.9 MB, against
/// 69.9 MB for the three arrays this replaced (whose 512² rough/AO array was
/// 14.0 MB). The +14 MB is the lift; packing is what holds it there rather
/// than at +42 MB for separate roughness and AO layers.
///
/// `RENDER_WORLD` only: an array's one job is to be uploaded, and the CPU
/// copy it was built from is the sources' — which are dropped with
/// [`GroundMaps`] once this exists.
#[derive(Resource, Clone)]
pub struct GroundArrays {
    /// `Rgba8UnormSrgb`, [`FAMILY_LAYERS`] layers: the four identities in
    /// `terrain::splat`'s order, then the road's aggregate.
    pub albedo: Handle<Image>,
    /// `Rgba8Unorm`, two families of [`FAMILY_LAYERS`] in that same order:
    /// tangent-space normals at `0..5`, then roughness-and-AO at
    /// [`ROUGH_AO_LAYER0`]`..10` — roughness in R, AO in G, B a constant 0 and
    /// A a constant 255 ([`pack_rg`]). One format, one tiling sampler, all
    /// data (`is_srgb = false`), which is what lets them share an array;
    /// albedo cannot join, because its sRGB decode happens before the filter.
    pub data: Handle<Image>,
}

/// Layers per map family: the four identities, then the road's aggregate.
pub const FAMILY_LAYERS: u32 = 5;
/// The first roughness-and-AO layer of [`GroundArrays::data`]; the normals
/// are the five below it.
///
/// The shader mirrors it as a WGSL const and indexes its planar taps by
/// literal; `tests/ground_tiling.rs` holds both to this.
pub const ROUGH_AO_LAYER0: u32 = FAMILY_LAYERS;
// The layer list `stack_ground` builds is five normals, then five packed
// roughness/AO layers; this is the one place that order and these numbers are
// tied together.
const _: () = assert!(FAMILY_LAYERS == 5 && ROUGH_AO_LAYER0 == 5);

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
/// set, not to stretch one member of it. ([`lift`] is the one sanctioned
/// exception, applied before this to the data array's packed roughness/AO
/// layers and nothing else.)
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

/// One plane of linear `Rgba8Unorm`: what [`pack_rg`] and [`lift`] work on.
fn linear_plane(image: &Image) -> bool {
    let d = &image.texture_descriptor;
    d.format == TextureFormat::Rgba8Unorm
        && d.dimension == TextureDimension::D2
        && d.size.depth_or_array_layers == 1
}

/// Two greyscale maps as one layer: `r`'s R channel in R, `g`'s R channel in
/// G, B a constant 0 and A a constant 255 — every level of both chains.
///
/// **Exact, because filtering is per channel.** The GPU filters R and G of a
/// texel independently, and `mipmap::chain` reduced each source's levels per
/// channel too (`Filter::Linear`, a plain average), so the packed chain is
/// the two original chains interleaved byte for byte and one tap reads both
/// maps at the value two taps read before. Roughness and AO are always read
/// at the same UV and layer, so packing them halves their taps. A greyscale
/// JPEG loads as `r = g = b` (`bevy_image` widens Luma8 to RGBA8), so R is the
/// whole of each source; B and A are constants so nothing can mistake them
/// for data.
///
/// `None` unless both are one plane of LINEAR `Rgba8Unorm` with matching size,
/// mip count, sampler and bytes.
pub fn pack_rg(r: &Image, g: &Image) -> Option<Image> {
    let (dr, dg) = (&r.texture_descriptor, &g.texture_descriptor);
    if !linear_plane(r)
        || !linear_plane(g)
        || dr.size != dg.size
        || dr.mip_level_count != dg.mip_level_count
        || r.sampler != g.sampler
    {
        return None;
    }
    let (rb, gb) = (r.data.as_ref()?, g.data.as_ref()?);
    let want = layer_bytes(dr.size.width, dr.size.height, dr.mip_level_count, 4);
    if rb.len() != want || gb.len() != want {
        return None;
    }
    let data: Vec<u8> = rb
        .chunks_exact(4)
        .zip(gb.chunks_exact(4))
        .flat_map(|(a, b)| [a[0], b[0], 0, 255])
        .collect();
    let mut packed = Image::new_uninit(dr.size, TextureDimension::D2, dr.format, r.asset_usage);
    packed.texture_descriptor.mip_level_count = dr.mip_level_count;
    packed.data = Some(data);
    packed.sampler = r.sampler.clone();
    Some(packed)
}

/// `image` at twice its size: a new level 0 upsampled from its own, over the
/// chain it already has. What lets a packed 512² roughness/AO map be a layer
/// of the 1024² data array without changing what the GPU reads. Per channel,
/// so a constant channel (the packed B and A) stays exactly that constant.
///
/// **The chain is kept, not rebuilt**, so new level `k + 1` IS old level `k`,
/// byte for byte, and doubling the size moves the hardware's LOD up by exactly
/// one: every tap that MINIFIED the old map — the whole ground past a few
/// metres — reads the same texels at the same trilinear weights. Only a tap
/// that magnified it reaches the new level 0, which is the old map's own
/// bilinear surface sampled at the new texel centres (¼ and ¾ between old
/// ones, wrapped, because every ground map tiles): the GPU's bilinear over it
/// equals the old one where both coordinates are ¼–¾ between old centres and
/// elsewhere cuts the corner by at most an eighth of the local second
/// difference per axis (plus half a byte of rounding). Pixel replication would
/// have kept no region exact.
///
/// The price is VRAM: a lifted layer is 4× the bytes of its source (+14 MB
/// over the five packed layers). Native 1024² roughness and AO would remove
/// both this and the corner.
///
/// `None` unless `image` is one plane of LINEAR `Rgba8Unorm` whose bytes match
/// its descriptor — an sRGB map would have to be upsampled in linear light,
/// and nothing lifted here is one.
pub fn lift(image: &Image) -> Option<Image> {
    let d = &image.texture_descriptor;
    if !linear_plane(image) {
        return None;
    }
    let (w, h) = (d.size.width as usize, d.size.height as usize);
    let src = image.data.as_ref()?;
    if w == 0
        || h == 0
        || src.len() != layer_bytes(d.size.width, d.size.height, d.mip_level_count, 4)
    {
        return None;
    }
    // New texel `x2`'s centre sits at old coordinate `x2 / 2 − ¼`: an even one
    // ¼ before old texel `x2 / 2`, an odd one ¼ after. The nearer old texel
    // takes ¾ and its neighbour on that side ¼.
    let taps = |x2: usize, n: usize| {
        let i = x2 / 2;
        (
            i,
            if x2.is_multiple_of(2) {
                (i + n - 1) % n
            } else {
                (i + 1) % n
            },
        )
    };
    let mut data = Vec::with_capacity(4 * w * h * 4 + src.len());
    for y2 in 0..2 * h {
        let (yn, yf) = taps(y2, h);
        for x2 in 0..2 * w {
            let (xn, xf) = taps(x2, w);
            let at = |x: usize, y: usize, c: usize| u32::from(src[(y * w + x) * 4 + c]);
            for c in 0..4 {
                // 9/16 · 3/16 · 3/16 · 1/16, rounded: integer, so exact and
                // the same on every target.
                let v = 9 * at(xn, yn, c) + 3 * at(xf, yn, c) + 3 * at(xn, yf, c) + at(xf, yf, c);
                data.push(((v + 8) / 16) as u8);
            }
        }
    }
    data.extend_from_slice(src);
    let mut lifted = Image::new_uninit(
        Extent3d {
            width: d.size.width * 2,
            height: d.size.height * 2,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        d.format,
        image.asset_usage,
    );
    lifted.texture_descriptor.mip_level_count = d.mip_level_count + 1;
    lifted.data = Some(data);
    lifted.sampler = image.sampler.clone();
    Some(lifted)
}

/// One layer of a family as [`stack_ground`] names it: a source image as it
/// is, or two greyscale sources [`pack_rg`]ed into one — and [`lift`]ed when
/// they are half the family's size, which only a packed layer ever may be.
enum Layer<'a> {
    One(&'a Handle<Image>),
    Pack(&'a Handle<Image>, &'a Handle<Image>),
}

impl Layer<'_> {
    fn handles(&self) -> impl Iterator<Item = &Handle<Image>> {
        let (a, b) = match *self {
            Layer::One(h) => (h, None),
            Layer::Pack(r, g) => (r, Some(g)),
        };
        std::iter::once(a).chain(b)
    }
}

/// One family's slot in [`Stacking`], its name for the panic message, and its
/// layers in order.
type Family<'a> = (&'a mut Option<Handle<Image>>, &'static str, &'a [Layer<'a>]);

/// What [`stack_ground`] has built so far. `Local` state: two slots, filled
/// one a frame.
#[derive(Default)]
pub struct Stacking {
    albedo: Option<Handle<Image>>,
    data: Option<Handle<Image>>,
}

/// Stack one family if all its layers are ready. `None` while any is not.
///
/// A [`Layer::Pack`] is packed, then lifted if it is exactly half layer 0's
/// size. A [`Layer::One`] is never resampled, so a half-size map anywhere else
/// (a normal map re-sourced at 512, say) is still the mismatch below rather
/// than something quietly upsampled.
///
/// # Panics
///
/// If the layers are all loaded and cannot be one array — a size, format or
/// sampler that disagrees within a family is a file swap that broke the set,
/// and drawing the island untextured over it would look like a lighting bug.
/// Saying so at boot is the same posture `GroundSplat::new` took for a missing
/// AO map.
fn try_stack(images: &mut Assets<Image>, family: &str, layers: &[Layer]) -> Option<Handle<Image>> {
    // The wait costs no allocation: only a frame that builds collects.
    if !layers
        .iter()
        .flat_map(Layer::handles)
        .all(|h| images.get(h).is_some_and(layer_ready))
    {
        return None;
    }
    let get = |h: &Handle<Image>| images.get(h).expect("checked ready above");
    let (w, h) = match layers.first() {
        Some(Layer::One(first)) => (get(first).width(), get(first).height()),
        Some(Layer::Pack(first, _)) => (get(first).width(), get(first).height()),
        None => (0, 0),
    };
    let built: Vec<Option<Image>> = layers
        .iter()
        .enumerate()
        .map(|(k, layer)| {
            let Layer::Pack(r, g) = *layer else {
                return None;
            };
            let packed = pack_rg(get(r), get(g)).unwrap_or_else(|| {
                panic!(
                    "the ground's {family} layer {k} cannot pack its two maps: both must be one plane of linear Rgba8Unorm with one size, mip count and sampler — re-source the SET (assets/textures/MANIFEST.md)"
                )
            });
            // A lift that cannot happen leaves the layer half-size, which
            // `stack` refuses below with the size in the message.
            let half = packed.width() * 2 == w && packed.height() * 2 == h;
            Some(half.then(|| lift(&packed)).flatten().unwrap_or(packed))
        })
        .collect();
    let fitted: Vec<&Image> = layers
        .iter()
        .zip(&built)
        .map(|(layer, b)| match (layer, b) {
            (_, Some(img)) => img,
            (Layer::One(src), None) | (Layer::Pack(src, _), None) => get(src),
        })
        .collect();
    let image = stack(&fitted).unwrap_or_else(|e| {
        panic!(
            "the ground's {family} maps cannot be one texture array: {e:?}. Every layer must share one size, format, mip count and sampler (packed roughness/AO may be exactly half the normals' size) — re-source the SET (assets/textures/MANIFEST.md), never one identity of it"
        )
    });
    Some(images.add(image))
}

/// Build [`GroundArrays`] out of [`GroundMaps`] — one family a frame, once
/// every one of a family's maps has had its mip pass — then drop the sources.
///
/// Runs after `mipmap::drain` in the same frame, so a chain finished this
/// frame is stacked this frame. **One family a frame, deliberately**: the
/// albedo array alone is ~28 MB of texels copied (the data array ~56 MB,
/// five of its layers packed and lifted), and `CLAUDE.md`'s stream-in rule is that a frame
/// pays for one of those, never both. Every frame after the sources are
/// dropped is one `Option` read.
pub fn stack_ground(
    mut commands: Commands,
    maps: Option<Res<GroundMaps>>,
    mut images: ResMut<Assets<Image>>,
    mut stacking: Local<Stacking>,
) {
    let Some(maps) = maps else {
        return;
    };
    let sets = [
        &maps.sand,
        &maps.grass,
        &maps.litter,
        &maps.rock,
        &maps.aggregate,
    ];
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
    let albedo = sets.map(|m| Layer::One(&m.albedo));
    // Normals, then roughness-in-R/AO-in-G from `ROUGH_AO_LAYER0` — the layer
    // order the shader indexes by literal.
    let data = [
        Layer::One(&sets[0].normal),
        Layer::One(&sets[1].normal),
        Layer::One(&sets[2].normal),
        Layer::One(&sets[3].normal),
        Layer::One(&sets[4].normal),
        Layer::Pack(&sets[0].rough, ao(0)),
        Layer::Pack(&sets[1].rough, ao(1)),
        Layer::Pack(&sets[2].rough, ao(2)),
        Layer::Pack(&sets[3].rough, ao(3)),
        Layer::Pack(&sets[4].rough, ao(4)),
    ];

    let s = &mut *stacking;
    let families: [Family; 2] = [
        (&mut s.albedo, "albedo", &albedo),
        (&mut s.data, "normal/rough/AO", &data),
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
    let (Some(albedo), Some(data)) = (s.albedo.clone(), s.data.clone()) else {
        return;
    };
    commands.insert_resource(GroundArrays { albedo, data });
    commands.remove_resource::<GroundMaps>();
}
