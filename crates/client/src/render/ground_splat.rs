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
//! ## What the roughness maps measured, which is ~nothing, and why that is the
//! finding rather than a disappointment
//!
//! Six vantages at the pinned `dev_spawn = 1500,600`, seed 20260731, Xvfb +
//! lavapipe, `--no-hud`, same shard, same build otherwise: **near-band
//! neighbour contrast 9.008 → 8.973 (−0.4%), mean luma 101.08 → 100.95
//! (−0.1%), near saturation 0.134 → 0.133 (−1.0%)**.
//!
//! **Those deltas are at the harness's own noise floor, and the floor was
//! measured rather than assumed.** Re-running a behaviourally identical build
//! twice moves contrast −0.3% and saturation −0.6% all by itself (the probe is
//! a live client against a live shard, so wind phase and clutter animation do
//! not repeat), and a third run put base → new at −0.7% contrast where the
//! first put it at −0.4%. A change smaller than the spread between two runs of
//! the same thing is not a measurement. **The only defensible reading is: no
//! detectable effect on the frame.** The albedo/normal half of this material
//! bought +32.8% contrast at the same spawn, which is two orders of magnitude
//! clear of that floor — so the instrument is not blind, this change is quiet.
//!
//! **The cause was one constant and it was not in this file.**
//! `terrain_mesh::ground_material` set `reflectance: 0.18`, and Bevy maps that
//! to normal-incidence specular as `F0 = 0.16 × reflectance²` — **0.0052, i.e.
//! 0.52%**, against the ~4% (reflectance 0.5) of an ordinary dielectric.
//! Roughness shapes the specular lobe and nothing else, so it was being asked
//! to redistribute about an eighth of the energy a real surface puts there.
//! The maps were bound, sampled per texel and correct; there was almost
//! nothing for them to shape.
//!
//! ✅ **FIXED 2026-08-25**, and this file's own reasoning is why it could be.
//! `render::fresnel` is the one place a `reflectance` is now decided and the
//! ground takes `fresnel::DIELECTRIC`. The ordering that slice insisted on is
//! what made it safe — *"turning up reflectance over a constant roughness
//! makes the whole island uniformly shiny, which is the defect, not the fix"*
//! — and the per-texel roughness field this file landed is exactly what it is
//! turned up over. It also turned out not to be one constant: **every**
//! material in the client was authored the same way, 8–70× under physical, so
//! the fix is a module rather than a number.
//!
//! ⚠ **The measurement above stands and has not been re-run.** The −0.4%
//! null result was measured with F0 at 0.52%; nobody has re-measured the
//! roughness maps' contribution now that there is energy for them to shape,
//! because that needs a GPU and a capture. Expect it to be non-null; do not
//! quote a number for it until someone takes one.
//!
//! ## The fourth map: ambient occlusion (2026-08-25)
//!
//! Bindings 114–117. All four ground identities publish an `<role>_ao.jpg`,
//! all four were git-tracked and staged into every depot by `ci/depot.py`, and
//! **this shader sampled twelve textures and none of them** — `occlusion_
//! texture` appeared zero times in `crates/`. `ART.md` §4 names this exact
//! term as the one scale a light rig cannot supply ("Medium … `indirectDiffuse
//! *= ao`, indirect only") and as the unblock for the ambient floor: raising
//! the fill lands everywhere including in the darks, while AO removes it only
//! where geometry occludes.
//!
//! **Folded with `min`, never multiplied**, which is §4's other half in one
//! line: "Never sum or multiply two occlusion terms of the same scale.
//! Frostbite takes `min(bakedAO, ssAO)` to avoid double-darkening." Bevy's own
//! `pbr_fragment` applies exactly that rule between a material's occlusion slot
//! and SSAO, so `pbr_input.diffuse_occlusion` arrives here carrying the SSAO
//! term alone — the base `StandardMaterial` has no occlusion texture because
//! these four are per-identity and it has one slot. This is the same fold, one
//! level up.
//!
//! **Diffuse only.** §4 again: "specular occlusion is a separate term, not the
//! diffuse one reused"; applying it to specular is visibly wrong at grazing
//! angles. `specular_occlusion` is left as Bevy computed it.
//!
//! ⚠ **Nothing compiles this shader.** `tests/ground_splat.rs` holds the
//! bindings equal across the WGSL and the Rust struct — both scraped now,
//! neither hand-kept — but a WGSL syntax or type error is a runtime failure
//! with every gate in this repo green. Boot it.
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

/// Per identity, the mean of its shipped `*_rough.jpg` — sand · grass ·
/// litter · rock, in `terrain::splat`'s order.
///
/// **This is a record, not a setting: the shader samples the maps and this is
/// what they measure.** Raw values, no sRGB decode — a roughness map is data
/// and is loaded `is_srgb = false`. `tests/ground_splat.rs` re-measures all
/// four off the files and fails on drift, so a swapped source cannot silently
/// change how rough an identity is.
///
/// ⚠ **It replaces `IDENTITY_ROUGH = [0.86, 0.93, 0.96, 0.88]`, and the
/// photograph disagreed with that knob about the ORDER, not just the level.**
/// The knob was authored "by how much a wet-looking specular lobe belongs on
/// each surface — damp sand is the smoothest thing on the island and dry
/// needle litter the roughest", giving sand < rock < grass < litter. Measured,
/// it is **rock ≪ litter < grass < sand**: dry beach sand is the *roughest*
/// of the four (0.963, and near-constant at sd 0.0065) and granite is by far
/// the smoothest (0.611). Both halves of the knob's premise were wrong —
/// `damp` sand is smooth and dry sand is not, and that difference is the wet
/// term's job ([`WET_ROUGH`]) rather than an identity's; and a hard mineral
/// face genuinely is smoother than needle litter. Granite moving 0.88 → 0.611
/// is the biggest single change here and it is the one to look at first.
pub const ROUGH_MEAN: [f32; 4] = [0.9631, 0.9364, 0.9197, 0.6108];

/// What a soaked surface keeps of its **dry roughness**.
///
/// `terrain_mesh::WET_VALUE`'s missing third: that file states the physics and
/// then states why it could not have it — *"a wet surface is also smoother, so
/// its specular tightens — is `perceptual_roughness`, which is per-material
/// and cannot vary per vertex without the shader `RENDER.md` §8 owns"*. This
/// module IS that shader, so the residual closes here, in the same shape
/// [`super::terrain_mesh::WET_VALUE`] uses for value: a keep-fraction the wet
/// factor ramps into.
///
/// It also carries the intent the retired `IDENTITY_ROUGH` was reaching for by
/// hand. "Damp sand is the smoothest thing on the island" is true and is now
/// true **by mechanism** — sand is the roughest identity dry and the wet band
/// is what smooths it — instead of being baked into a constant that then made
/// dry dune sand specular everywhere the tide never reaches.
///
/// PROPOSED, `DECISIONS.md` §open "ground roughness v1". Not measured off
/// anything: no reference frame in `ART.md` §3 carries a roughness row, and
/// water filling microrelief has no number in this tree. 0.75 is deliberately
/// short of the 0.55 value keeps — a visible tightening at the waterline, not
/// a mirror, because the identity actually at a waterline is the one whose map
/// has almost no relief to lose (sand, sd 0.0065).
pub const WET_ROUGH: f32 = 0.75;

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
    /// `xyz` the identity's authored linear albedo.
    ///
    /// **`w` is reserved and zero.** It carried `IDENTITY_ROUGH` until the
    /// roughness maps landed; roughness is now sampled per texel and there is
    /// no per-identity scalar left to send. The slot stays because a
    /// `vec3` in a uniform array has a 16-byte stride anyway — dropping it
    /// would change nothing about the layout and would cost the reader the
    /// note explaining where the roughness went.
    pub identity: [Vec4; 4],
    pub gain: Vec4,
    /// x = `WET_VALUE`, y = `WET_SATURATION`, z = `ALBEDO_LUMA_FLOOR`,
    /// w = [`BLEND_DEPTH`].
    pub tune: Vec4,
    /// x = [`HEIGHT_INFLUENCE`], y = [`NORMAL_Z_FLOOR`], z = [`WET_ROUGH`],
    /// w reserved.
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
            identity[k] = Vec4::new(a[0], a[1], a[2], 0.0);
        }
        Self {
            identity,
            gain: Vec4::from_array(GRAIN_GAIN),
            tune: Vec4::new(WET_VALUE, WET_SATURATION, ALBEDO_LUMA_FLOOR, BLEND_DEPTH),
            blend: Vec4::new(HEIGHT_INFLUENCE, NORMAL_Z_FLOOR, WET_ROUGH, 0.0),
        }
    }
}

/// Four albedo maps, four normal maps, four roughness maps, four AO maps — and
/// one sampler.
///
/// **One sampler for sixteen textures, and that is the constraint that
/// scales.** Each map wants the identical tiling and anisotropy descriptor
/// `textures::tiling` builds, and a sampler each would put this bind group at
/// 32 in the fragment stage before `StandardMaterial`'s own are counted — far
/// over the 16 a downlevel adapter guarantees. Textures are the cheap axis
/// (Bevy asks the adapter for its own limits, and every desktop adapter is far
/// past 16); samplers are the one with a hard floor under it. So the roughness
/// slice cost four bindings and zero samplers, and the AO slice cost four more
/// and **zero** again.
///
/// ⚠ **The roughness slice cost no new VRAM and the AO slice DOES**, which is
/// the one place these two otherwise-identical changes differ.
/// `textures::MapSet::load` had always loaded `<role>_rough.jpg`, so those four
/// were resident and uploaded from the day the maps landed — paid for and
/// unread, and binding them was free. `<role>_ao.jpg` was **not** loaded by
/// anything: it shipped in the depot and never reached the GPU. So AO is four
/// genuinely new 1K uploads here (and three more on the prop side), which is
/// real and small and worth not misremembering as free.
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
    // 109 is the shared sampler, declared above on `albedo_rock`.
    #[texture(110)]
    pub rough_sand: Handle<Image>,
    #[texture(111)]
    pub rough_grass: Handle<Image>,
    #[texture(112)]
    pub rough_litter: Handle<Image>,
    #[texture(113)]
    pub rough_rock: Handle<Image>,
    /// Ambient occlusion, per identity. **`ART.md` §4's MEDIUM scale** — the
    /// one occlusion term a light rig cannot supply, "between a surface's own
    /// features … what a fetched `*_ao.jpg` carries", indirect only.
    ///
    /// All four ground sources publish one and all four shipped in every depot
    /// unread until 2026-08-25: `occlusion_texture` appeared zero times in
    /// `crates/`, and this shader sampled twelve textures and none of them.
    /// `Option` is not needed here where `MapSet::ao` has one — every ground
    /// role is in `textures::ROLES_WITH_AO`, and `GroundSplat::new` asserts it
    /// rather than silently binding a default handle, which would sample BLACK
    /// and put the whole island in shadow.
    #[texture(114)]
    pub ao_sand: Handle<Image>,
    #[texture(115)]
    pub ao_grass: Handle<Image>,
    #[texture(116)]
    pub ao_litter: Handle<Image>,
    #[texture(117)]
    pub ao_rock: Handle<Image>,
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
            rough_sand: maps.sand.rough.clone(),
            rough_grass: maps.grass.rough.clone(),
            rough_litter: maps.litter.rough.clone(),
            rough_rock: maps.rock.rough.clone(),
            // **`expect`, not `unwrap_or_default`.** An unresolved handle in a
            // texture slot samples as black, and a black occlusion map puts the
            // entire island in full shadow — a spectacular failure that would
            // look like a lighting bug rather than a missing file. Every ground
            // role is in `textures::ROLES_WITH_AO`, so this cannot fire without
            // that list and `assets/textures/` having drifted apart, and it is
            // better to say so at boot than to draw a black world.
            ao_sand: ao(&maps.sand, "sand"),
            ao_grass: ao(&maps.grass, "grass"),
            ao_litter: ao(&maps.litter, "litter"),
            ao_rock: ao(&maps.rock, "rock"),
        }
    }
}

/// One ground role's AO handle, or a loud failure.
fn ao(m: &super::textures::MapSet, role: &str) -> Handle<Image> {
    m.ao.clone().unwrap_or_else(|| {
        panic!(
            "ground identity `{role}` has no AO map — every ground role must be              in textures::ROLES_WITH_AO with a matching assets/textures/             {role}_ao.jpg, or the splat shader samples an unresolved handle as              BLACK and the island draws in full shadow"
        )
    })
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
