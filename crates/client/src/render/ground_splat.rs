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
//! The G channel of the data array's roughness layers,
//! `textures::ROUGH_AO_LAYER0`.. (bindings 114–117 until 2026-09-12, then
//! layers of a rough/AO array until 2026-10-07). All
//! four ground identities publish an `<role>_ao.jpg`, all four were
//! git-tracked and staged into every depot by `ci/depot.py`, and **this
//! shader sampled twelve textures and none of them** — `occlusion_texture`
//! appeared zero times in `crates/`. `ART.md` §4 names this exact
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
//! ## Sixteen textures became three arrays, then two (2026-09-12, 2026-10-07)
//!
//! Bindings 101–117 were sixteen `texture_2d`s and one sampler, and the
//! sampler paragraph below was right about the axis that binds on a desktop
//! adapter and wrong about the one that binds in a browser: WebGL2's
//! `max_sampled_textures_per_shader_stage` is **16, per stage, summed over
//! every bind group in the pipeline** — the view's shadow and environment
//! maps, `StandardMaterial`'s six slots and ours — and the material group
//! alone was 22. The browser refused the pipeline layout before a frame.
//! `textures::GroundArrays` is the answer: the identities (and the road's
//! aggregate behind them) as layers of one `texture_2d_array` cost one
//! binding.
//!
//! **Three arrays fit WebGL2 and not WebGPU.** WebGPU guarantees the same 16,
//! and there the atmosphere's transmittance LUT is bound too: the view's
//! seven, the LUT, `StandardMaterial`'s six and albedo, normal and rough/AO
//! made 17, and Chrome refuses the pipeline — which under WebGPU loses the
//! frame, not the mesh. `render::quality::FRAGMENT_TEXTURES` is that budget,
//! and the ground is now exactly it: TWO arrays, the sRGB albedo and one
//! linear data array (normals, then roughness in R and AO in G of one layer
//! at `textures::ROUGH_AO_LAYER0` — one format, one sampler, all data). Albedo
//! cannot join them because its sRGB decode happens before the filter. The
//! packing is exact (filtering is per channel; `textures::pack_rg`) and turns
//! two taps into one: 15 a pixel on flat ground where there were 20.
//! **Same texels, same filter, same chain** — a layer samples exactly as the
//! standalone texture did, which is what lets this stay ONE shader for both
//! targets rather than a web variant (`findings/web-build-20260909.md` §15).
//! The one bounded exception is `textures::lift`: roughness and AO ship at
//! 512² beside 1024² normals and gain an upsampled top level to share their
//! array, read only where the old map was magnified.
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
use bevy::mesh::{MeshVertexAttribute, MeshVertexBufferLayoutRef};
use bevy::pbr::{
    ExtendedMaterial, MaterialExtension, MaterialExtensionKey, MaterialExtensionPipeline,
    StandardMaterial,
};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError, VertexFormat,
};
#[cfg(not(target_arch = "wasm32"))]
use bevy::render::storage::ShaderStorageBuffer;
use bevy::shader::ShaderRef;

use super::terrain_mesh::{
    ALBEDO_LUMA_FLOOR, GROUND_ALBEDO, GROUND_TILE_M, WET_SATURATION, WET_VALUE,
};
use super::textures::GroundArrays;
use sim_core::terrain;

/// The shader, resolved against the asset root `bin/gates.rs` sets.
pub const SHADER: &str = "shaders/ground_splat.wgsl";

/// Signed cross-road distance, cyclic dash phase, and validity.
pub const ATTRIBUTE_MARKINGS: MeshVertexAttribute =
    MeshVertexAttribute::new("RoadMarkings", 0x7061696e, VertexFormat::Float32x4);

/// Independent road coverage, ring then branch. Keep UV0's tangent frame and
/// UV1's macro/wetness modifiers intact; packed floats do not interpolate.
pub const ATTRIBUTE_ROAD: MeshVertexAttribute =
    MeshVertexAttribute::new("RoadCoverage", 0x726f6164, VertexFormat::Float32x2);

/// The two zones the ground under a vertex is in: x how arid, 0..1
/// (`terrain::aridity`), or on a rock the NEGATED metres it stands above the
/// ground (`RockSoup::mesh`), which the shader clamps out of the desert and
/// takes off the height the spread reads; y the Verdigris' edge noise, within ±1
/// (`terrain::fel_wander`), from which the shader rebuilds the heart and the
/// live spread (`blight_at`, `sim_core::blight`). The desert's warm sand and
/// the Verdigris' ash and obsidian are tints on the four identities, not
/// identities of their own, so the splat stays four — and one `vec2` at one
/// location keeps WebGL2's attribute count where the desert left it.
pub const ATTRIBUTE_ZONE: MeshVertexAttribute =
    MeshVertexAttribute::new("Zone", 0x7a6f6e65, VertexFormat::Float32x2);

/// The desert's sand, linear: a pale warm tan, where the beach's sand is a
/// pale grey-beige (`terrain_mesh::GROUND_ALBEDO[0]`). sRGB about
/// (180, 160, 124), the colour of the reference game's arid biome — it was
/// (170, 137, 95), an orange the reference never shows. **(knob)**
pub const DESERT_SAND_ALBEDO: [f32; 3] = [0.45, 0.35, 0.20];

/// Lying snow, linear (snow v0): sRGB about 205 grey, under fresh snow's
/// ~0.8 so a sunlit drift keeps its relief rather than clipping white.
/// **(knob)**
pub const SNOW_ALBEDO: f32 = 0.6;
/// How far under the lowest place it was seen falling the snow fades in, m:
/// a degree of air at the lapse `[climate]` ships.
pub const SNOW_FADE_M: f32 = 10.0;
/// How far past the band of aridity it was seen falling on the snow fades
/// out (`WeatherNow::snow_arid`), in aridity.
pub const SNOW_ARID_FADE: f32 = 0.1;

/// The grass the desert dries, linear: a golden straw, sRGB about (156, 137, 89),
/// the reference game's arid grass. **(knob)**
pub const DRY_GRASS_ALBEDO: [f32; 3] = [0.33, 0.25, 0.10];

/// The desert's rock, linear: a pale warm sandstone, sRGB about
/// (200, 173, 139), where the rest of the island's is granite (0.285, 0.254,
/// 0.202 — already beige, so anything short of this reads as the same grey
/// rock). The reference game tints one rock by the biome it stands in
/// (`reference/ROCKS.md`). **(knob)**
pub const DESERT_ROCK_ALBEDO: [f32; 3] = [0.58, 0.42, 0.26];

/// How far the grass identity goes to [`DRY_GRASS_ALBEDO`] at full aridity.
/// All the way: the desert's grass patches are straw through, and the turf
/// at its edge, at less than full aridity, still keeps some green. **(knob)**
pub const DRY_GRASS_MIX: f32 = 1.0;

/// The desert's grass patches' wavelength, metres. **(knob)** The reference
/// game's arid floor is golden grass and open sand in patches tens of metres
/// across, not sand alone.
pub const DESERT_GRASS_M: f32 = 34.0;

/// The aridity over which the grass patches come in (they are the desert's,
/// not the dry meadow's, which is already grass).
pub const DESERT_GRASS_ARID: (f32, f32) = (0.2, 0.6);

/// How much of the desert's sand goes to grass, 0..1, at `(x, z)`: the patch
/// field alone, before aridity. The ground shader paints the same field
/// (`ground_splat.wgsl`'s `desert_grass`, with these numbers as literals) and
/// the clutter grows its dry tufts on it, so the tufts stand on the golden
/// ground and not on bare sand.
pub fn desert_grass(x: f32, z: f32) -> f32 {
    let (x, z) = (x / DESERT_GRASS_M, z / DESERT_GRASS_M);
    let n = 0.65 * plane_value(x, z) + 0.35 * plane_value(x * 2.7 + 17.0, z * 2.7 + 5.0);
    let t = ((n - 0.5) / 0.14).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// `ground_splat.wgsl`'s `value3` on the plane y = 0, bit for bit as far as
/// f32 goes: the shader's lattice hash, its quintic fade.
fn plane_value(x: f32, z: f32) -> f32 {
    fn lowbias32(mut x: u32) -> u32 {
        x ^= x >> 16;
        x = x.wrapping_mul(0x7feb_352d);
        x ^= x >> 15;
        x = x.wrapping_mul(0x846c_a68b);
        x ^= x >> 16;
        x
    }
    let key = |cx: i32, cz: i32| {
        let h =
            lowbias32((cx as u32).wrapping_mul(73_856_093) ^ (cz as u32).wrapping_mul(83_492_791));
        (h >> 8) as f32 * (1.0 / 16_777_216.0)
    };
    let (ix, iz) = (x.floor(), z.floor());
    let (fx, fz) = (x - ix, z - iz);
    let q = |f: f32| f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    let (ux, uz) = (q(fx), q(fz));
    let (ix, iz) = (ix as i32, iz as i32);
    let a = key(ix, iz) + (key(ix + 1, iz) - key(ix, iz)) * ux;
    let b = key(ix, iz + 1) + (key(ix + 1, iz + 1) - key(ix, iz + 1)) * ux;
    a + (b - a) * uz
}

/// The turf, linear: the grass cards' own mean colour (the opaque texels of
/// `clutter::CARD_ATLAS`), which the grass identity is drawn as where the
/// blades have sunk (`foliage::GRASS_FADE_END_M`) and partly under them
/// ([`TURF_NEAR`]).
///
/// **Why the ground does not simply carry this as its albedo.** The tufts
/// average about twice the ground's value (this against
/// `GROUND_ALBEDO[1]`), so every gap between them read as a hole and the
/// meadow past the fade dropped to dark felt; the Rust frames' turf keeps
/// its colour to the horizon (2026-10-21). `GROUND_ALBEDO` is the identity
/// the fill's bounce and `ART.md` §3's bands are pinned to, so the turf is a
/// distance blend over it instead.
pub const TURF_ALBEDO: [f32; 3] = [0.106, 0.117, 0.023];
/// How much of [`TURF_ALBEDO`] the grass identity carries inside the grass's
/// fade, under the tufts. **(knob)**
pub const TURF_NEAR: f32 = 0.6;

/// The Verdigris' ash, linear: what sand and forest litter go to at full
/// corruption. A green-black grey at the albedo floor's edge (`ART.md`: no
/// pure black). **(knob)**
pub const FEL_ASH_ALBEDO: [f32; 3] = [0.058, 0.062, 0.054];
/// Its dead turf, linear: what the grass goes to — a scorched olive-brown.
/// **(knob)**
pub const FEL_DEAD_ALBEDO: [f32; 3] = [0.074, 0.070, 0.040];
/// Its obsidian, linear: what the rock goes to — black glass with a cold cast.
/// **(knob)**
pub const FEL_ROCK_ALBEDO: [f32; 3] = [0.050, 0.055, 0.056];
/// What obsidian's roughness is multiplied by: glass, not granite. **(knob)**
pub const FEL_ROCK_ROUGH: f32 = 0.75;
/// The fissure plates' size, metres. **(knob)**
pub const FEL_CELL_M: f32 = 7.0;
/// A fissure's half-width at its widest, in plate cells (0.018 × 7 m: a
/// crack about a quarter-metre across, breathing narrower along its run).
/// **(knob)**
pub const FEL_CRACK_W: f32 = 0.018;
/// The scorched rim either side of a fissure, cells. **(knob)**
pub const FEL_RIM_W: f32 = 0.04;
/// The share of plate edges that glow at the heart; fewer toward the edge.
/// **(knob)**
pub const FEL_CRACK_SHARE: f32 = 0.55;
/// The pool field's line (its value noise sits in 0..1), and its wavelength,
/// metres: pools are where the field clears the line. **(knob)**
pub const FEL_POOL_AT: f32 = 0.66;
pub const FEL_POOL_M: f32 = 26.0;
/// The glow by day and by night, linear and exposure-weight 0 — between the
/// town's lapis (1.6 by night) and its bulbs (6.0), the band `beasts::FEL_GLOW`
/// sits in. `rig::day_night` blends the two by the hour. **(knob)**
pub const FEL_GLOW_DAY: [f32; 3] = [0.10, 0.55, 0.07];
pub const FEL_GLOW_NIGHT: [f32; 3] = [0.45, 2.60, 0.32];
/// The fissures' slow breath, cycles a second. **(knob)**
pub const FEL_PULSE_HZ: f32 = 0.35;

/// The spread's advancing edge, drawn brighter so the creep reads as moving:
/// the window of the creep's own fade coordinate it lights (rising over the
/// first two, falling over the last two), how much it adds to the fissures'
/// glow, and how far the front must have walked from wipe day's (units of
/// `CONTINENT_RADIUS`) before it shows at all, so wipe day is the heart
/// alone. **(knob)**
pub const FRONT_RIM_T: [f32; 4] = [0.30, 0.45, 0.65, 0.85];
pub const FRONT_RIM_GAIN: f32 = 1.5;
pub const FRONT_RIM_ONSET: f32 = 0.01;

/// The discs the ground can draw at once: outbreaks and hot spots
/// (`sim_core::blight::MAX_DISCS`, with room).
pub const BLIGHT_DISCS: usize = 12;
const _: () = assert!(sim_core::blight::MAX_DISCS <= BLIGHT_DISCS);

/// The live half of the spread's uniform, packed from a moment
/// (`sim_core::blight::Now`): the front, the disc count and the rim's gain;
/// the ward's centre; and each disc as (x, z, radius, depth), a hot spot's
/// depth negative.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BlightLive {
    pub now: Vec4,
    pub ward: Vec4,
    pub discs: [Vec4; BLIGHT_DISCS],
}

impl BlightLive {
    pub fn of(n: &sim_core::blight::Now) -> Self {
        let rim =
            FRONT_RIM_GAIN * ((n.front - sim_core::blight::F0) / FRONT_RIM_ONSET).clamp(0.0, 1.0);
        let mut discs = [Vec4::ZERO; BLIGHT_DISCS];
        for (out, d) in discs.iter_mut().zip(n.discs.iter().take(n.n as usize)) {
            *out = Vec4::new(d.x, d.z, d.r, if d.hot { -d.depth } else { d.depth });
        }
        Self {
            now: Vec4::new(n.front, n.n as f32, rim, 0.0),
            ward: Vec4::new(n.ward.0, n.ward.1, 0.0, 0.0),
            discs,
        }
    }

    /// Whether `p` already carries this moment.
    pub fn matches(&self, p: &GroundSplatParams) -> bool {
        p.blight_now == self.now && p.blight_ward == self.ward && p.blight_discs == self.discs
    }

    pub fn write(&self, p: &mut GroundSplatParams) {
        p.blight_now = self.now;
        p.blight_ward = self.ward;
        p.blight_discs = self.discs;
    }
}

/// Proposed material defaults, DECISIONS.md §open, road surface v1.
/// Linear reflectances stay inside ART.md §5's band. Aggregate comes from
/// the CC0 Gravel004 map (`textures::GroundMaps::aggregate`, array layer 4),
/// at pavement grading rather than scree size.
pub const ROAD_PAVEMENT_ALBEDO: [f32; 3] = [0.065, 0.070, 0.072];
pub const ROAD_DIRT_ALBEDO: [f32; 3] = [0.18, 0.135, 0.085];
pub const ROAD_AGGREGATE_TILE_M: f32 = 1.0;
/// Restrained erosion of the interpolated edge by the aggregate's own relief.
pub const ROAD_EDGE_WEAR: f32 = 0.12;
/// Binder flattens the photographed loose aggregate's relief on pavement.
pub const ROAD_PAVEMENT_RELIEF: f32 = 0.25;
/// Equal grit/aggregate mixture on the unpaved branch.
pub const ROAD_DIRT_AGGREGATE: f32 = 0.5;
/// Fade the new surface over the outermost guaranteed complete chunk. The
/// nearest edge of the 5×5 terrain ring is two chunks from the camera; its
/// sub-chunk movement can only move the other edges farther away.
pub const ROAD_FADE_END_M: f32 =
    super::terrain_mesh::CHUNK_M * super::terrain_mesh::NEAR_RADIUS as f32;
pub const ROAD_FADE_START_M: f32 = ROAD_FADE_END_M - super::terrain_mesh::CHUNK_M;

/// Proposed worn paint reflectances and coverage, DECISIONS.md road markings v1.
pub const ROAD_PAINT_YELLOW: [f32; 3] = [0.42, 0.32, 0.09];
pub const ROAD_PAINT_WHITE: [f32; 3] = [0.50, 0.48, 0.42];
pub const ROAD_PAINT_OPACITY: f32 = 0.65;
/// Four float ulps tolerate interpolation rounding, not material blending.
pub const ROAD_PAINT_VALID_EPS: f32 = 4.0 * f32::EPSILON;

/// Coverage follows the sim's published bands. The ring wins a junction;
/// shoulders remain dusty ground rather than extending the paved width.
pub fn road_coverage(
    ring: sim_core::terrain::RoadBand,
    side: sim_core::terrain::RoadBand,
) -> [f32; 2] {
    use sim_core::terrain::{RoadBand, ROAD_WEAR_SHOULDER};
    let pavement = if ring == RoadBand::Carriageway {
        1.0
    } else {
        0.0
    };
    let dirt = match side {
        RoadBand::Carriageway => 1.0,
        RoadBand::Shoulder => ROAD_WEAR_SHOULDER,
        RoadBand::Off => 0.0,
    };
    [pavement, dirt * (1.0 - pavement)]
}

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
/// ⚠ **`rock` moved 3.7128 → 4.0820 on 2026-08-27**, when that identity's
/// source was replaced (aCG `Rock023`, a stratified CLIFF, → aCG `Gravel004`,
/// isotropic scree — `assets/textures/MANIFEST.md`). Its mean linear luma fell
/// 0.2693 → 0.2450, so the gain that places that mean at 1 rose by the same
/// 9.0%. Nothing was authored here; the file changed and the gate said so,
/// which is the whole reason this constant is re-measured rather than typed.
/// ⚠ **And 4.0820 → 9.9542 on 2026-09-24** (`Gravel004` → aCG `Rock032`, a
/// weathered slab): scree read as a cobbled road at a player's feet above the
/// treeline. Gravel004 stayed as the road's aggregate, at [`AGGREGATE_GAIN`].
/// ⚠ **2026-10-11: sand 5.5398 → 10.1466 and rock 9.9542 → 3.6626**, two
/// swaps to sources with a published size (PH `sand_02`, aCG `Rock043L`).
pub const GRAIN_GAIN: [f32; 4] = [10.1466, 4.0292, 9.6954, 3.6626];

/// `1 / linear-luma mean` of the road's aggregate map (`aggregate_albedo.jpg`,
/// `Gravel004`) — [`GRAIN_GAIN`]'s construction for the one layer that is not
/// an identity. Measured, and re-measured by `tests/ground_splat.rs`.
pub const AGGREGATE_GAIN: f32 = 4.0820;

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
///
/// ⚠ **`rock` moved 0.6108 → 0.5359 with the same 2026-08-27 source swap.**
/// Granite was already the smoothest of the four and scree is smoother still,
/// which is worth a second look when someone can boot it: it is now 0.43 below
/// sand and the wet term ([`WET_ROUGH`]) multiplies from there. If a mountain
/// reads as glossy in a frame, this is the number to suspect first — the same
/// sentence this block already carried about 0.88 → 0.611, one swap later.
/// ⚠ **And back up to 0.6971 with the 2026-09-24 slab (`Rock032`)** — still
/// the smoothest identity by 0.22.
/// ⚠ **2026-10-11: sand 0.9631 → 0.8417 (`sand_02`) and rock → 0.6408
/// (`Rock043L`).** Granite stays the smoothest, by 0.20.
pub const ROUGH_MEAN: [f32; 4] = [0.8417, 0.9364, 0.9197, 0.6408];

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

/// How much the caustic web lifts the albedo of ground under shallow water at
/// its brightest (`ground_splat.wgsl::caustic`): 1.0 doubles it on a thread.
pub const CAUSTIC_GAIN: f32 = 1.1;
/// Depth where the caustics start to fade, and where they are gone, metres.
pub const CAUSTIC_FADE_FROM_M: f32 = 1.5;
pub const CAUSTIC_FADE_TO_M: f32 = 5.0;
/// How far light travels through the water per metre of depth on its way to
/// the bed and back to the eye: down, then up again.
pub const SEABED_PATH: f32 = 2.0;

/// The uniform, laid out to match `GroundSplat` in the shader.
#[derive(Clone, Default, ShaderType, Debug)]
pub struct GroundSplatParams {
    /// `xyz` the identity's authored linear albedo.
    ///
    /// **`w` is spare but for `identity[0].w`, the rain's wetness** (weather
    /// v0, `weather::update` → `terrain_mesh::Ring::set_rain_wet`): how soaked
    /// the island looks, `0..1`, applied to up-facing ground through the same
    /// `wetted` as the shoreline. The others carried `IDENTITY_ROUGH` until the
    /// roughness maps landed and are zero; a `vec3` in a uniform array has a
    /// 16-byte stride anyway, so the slots cost nothing.
    pub identity: [Vec4; 4],
    pub gain: Vec4,
    /// x = `WET_VALUE`, y = `WET_SATURATION`, z = `ALBEDO_LUMA_FLOOR`,
    /// w = [`BLEND_DEPTH`].
    pub tune: Vec4,
    /// x = [`HEIGHT_INFLUENCE`], y = [`NORMAL_Z_FLOOR`], z = [`WET_ROUGH`],
    /// w = [`ROAD_DIRT_AGGREGATE`].
    ///
    /// **These are here rather than as WGSL `const`s because a knob that lives
    /// only in a shader is a knob nothing can cross-check.** `ci/gates.sh`'s
    /// knob registry scans `.rs/.js/.mjs` and refused the `DECISIONS.md` rows
    /// for both until they moved — which is the registry working exactly as
    /// intended, and the reason to pass them through the uniform.
    pub blend: Vec4,
    /// x = [`WALL_ON`], y = [`WALL_SHARPNESS`],
    /// z = [`super::terrain_mesh::UV_PER_M`], w = [`ROAD_PAVEMENT_RELIEF`].
    ///
    /// `z` is the ground's own projection scale, sent rather than repeated: the
    /// wall tap builds a UV in metres and must land on the same texel density
    /// as the top tap, and a second literal `0.25` in WGSL is a copy that can
    /// drift from the one `heightfield` writes.
    pub wall: Vec4,
    /// Per identity, what the mesh's UV must be multiplied by so that identity
    /// repeats every [`GROUND_TILE_M`] metres — `1 / (UV_PER_M × tile_m)`.
    ///
    /// **Sent as a multiplier rather than as the tile size itself** so the
    /// shader does no division and the reference density stays in one place:
    /// `heightfield` writes UV at [`UV_PER_M`] and this is the only thing that
    /// re-spreads it. Sand and rock are drawn at the 4 m reference, so their
    /// entries are exactly `1.0` and their sampling is bit-unchanged.
    pub tile: Vec4,
    /// xyz = pavement linear albedo; w = aggregate UV multiplier.
    pub pavement: Vec4,
    /// xyz = branch linear albedo; w = edge wear.
    pub road_dirt: Vec4,
    /// x/y = distance fade start/end, derived from the terrain ring; z/w =
    /// the grass's fade start/end (`foliage::GRASS_FADE_*`), where the turf
    /// takes over.
    pub road_lod: Vec4,
    /// xyz = paint reflectance; w = maximum worn coverage.
    pub paint_yellow: Vec4,
    pub paint_white: Vec4,
    /// x/y = stripe width and radial edge offset; z = validity roundoff tolerance.
    pub paint_geometry: Vec4,
    /// x = [`ROCK_CELL_M`], y = [`ROCK_FINE_M`], z = [`ROCK_TILT`],
    /// w = [`ROCK_FINE_TILT`].
    pub rock_a: Vec4,
    /// x = [`ROCK_SHADE`], y = [`ROCK_WEATHER_M`], z = [`ROCK_WEATHER`],
    /// w = [`ROCK_WARP_M`].
    pub rock_b: Vec4,
    /// x = [`ROCK_CRACK_SHARE`], y = [`ROCK_CRACK_W`], z = [`ROCK_CRACK_DARK`],
    /// w = [`ROCK_STREAK`].
    pub rock_c: Vec4,
    /// x = [`ROCK_STREAK_W_M`], y = [`ROCK_STREAK_H_M`], z = [`ROCK_FACE_ON`],
    /// w = [`ROCK_FACE_FULL`].
    pub rock_d: Vec4,
    /// x = [`ROCK_SEAM`], y = [`ROCK_FLAT_LEAN`], z/w =
    /// [`ROCK_LEAN_ON`]/[`ROCK_LEAN_FULL`].
    pub rock_e: Vec4,
    /// x = [`MOTTLE_M`], y = [`MOTTLE_AMP`]. zw reserved and zero.
    pub mottle: Vec4,
    /// x = [`AGGREGATE_GAIN`]: the road's layer is not an identity, so
    /// `gain` has no slot for it. yzw reserved and zero.
    pub aggregate: Vec4,
    /// The per-pixel cliff: x/y = [`CLIFF_TAN_LO`]/[`CLIFF_TAN_HI`],
    /// z = [`CLIFF_NOISE`], w = [`CLIFF_NOISE_M`].
    pub cliff: Vec4,
    /// x = [`WALL_PLANE_BLEND`], y/z = [`PROJECT_TRI_COS`]/
    /// [`PROJECT_NORMAL_COS`]. w reserved and zero.
    pub wall_planes: Vec4,
    /// x = `terrain::SEA_LEVEL`, y = [`CAUSTIC_GAIN`], z/w =
    /// [`CAUSTIC_FADE_FROM_M`]/[`CAUSTIC_FADE_TO_M`].
    pub caustics: Vec4,
    /// xyz = `water::EXTINCT`, w = [`SEABED_PATH`]: the water's own colour,
    /// put on the bed under it.
    pub water_tint: Vec4,
    /// xyz = [`DESERT_SAND_ALBEDO`], w = [`DRY_GRASS_MIX`]: what the sand and
    /// the grass go to as the ground dries (`ATTRIBUTE_ARID`).
    pub desert: Vec4,
    /// xyz = [`DRY_GRASS_ALBEDO`], w = [`DESERT_GRASS_M`].
    pub dry_grass: Vec4,
    /// xyz = [`DESERT_ROCK_ALBEDO`]. w reserved and zero.
    pub desert_rock: Vec4,
    /// Snow (snow v0): x how white the weather has laid the ground and y the
    /// height it lies from, both live (`terrain_mesh::Ring::set_snow`);
    /// z = [`SNOW_ALBEDO`], w = [`SNOW_FADE_M`].
    pub snow: Vec4,
    /// x..y the band of aridity it lies on (`WeatherNow::snow_arid`), live;
    /// z = [`SNOW_ARID_FADE`]; w reserved and zero.
    pub snow_band: Vec4,
    /// xyz = [`FEL_ASH_ALBEDO`], w = [`FEL_CRACK_SHARE`].
    pub fel_ash: Vec4,
    /// xyz = [`FEL_DEAD_ALBEDO`], w = [`FEL_CELL_M`].
    pub fel_dead: Vec4,
    /// xyz = [`FEL_ROCK_ALBEDO`], w = [`FEL_ROCK_ROUGH`].
    pub fel_rock: Vec4,
    /// rgb = the glow at this hour (`Ring::set_fel_glow`), w =
    /// [`FEL_PULSE_HZ`].
    pub fel_glow: Vec4,
    /// x = [`FEL_CRACK_W`], y = [`FEL_RIM_W`], z = [`FEL_POOL_AT`], w =
    /// [`FEL_POOL_M`].
    pub fel_shape: Vec4,
    /// The heart: xy where it stands, z 1 / `CONTINENT_RADIUS`, w its wander.
    pub blight_heart: Vec4,
    /// x, y the heart's whole and outer edge; z the beach's top; w the coast's
    /// fade, metres.
    pub blight_fel: Vec4,
    /// The creep: x its band, y its depth, z the ridges' hold, w the ward's.
    pub blight_creep: Vec4,
    /// x, y the ridges' ramp, metres; z, w the ward's inner and outer radius.
    pub blight_ridge: Vec4,
    /// The discs' edges: an outbreak's wander and band, a hot spot's.
    pub blight_disc: Vec4,
    /// The front rim's window in the creep's fade coordinate.
    pub blight_rim: Vec4,
    /// Live: x the front, y how many discs, z the rim's gain now.
    pub blight_now: Vec4,
    /// Live: xy the ward's centre.
    pub blight_ward: Vec4,
    /// xyz = [`TURF_ALBEDO`], w = [`TURF_NEAR`].
    pub turf: Vec4,
    /// Live: each disc, (x, z, radius, depth), a hot spot's depth negative.
    /// Last, so the array closes the struct on both sides.
    pub blight_discs: [Vec4; BLIGHT_DISCS],
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
            blend: Vec4::new(
                HEIGHT_INFLUENCE,
                NORMAL_Z_FLOOR,
                WET_ROUGH,
                ROAD_DIRT_AGGREGATE,
            ),
            wall: Vec4::new(
                WALL_ON,
                WALL_SHARPNESS,
                super::terrain_mesh::UV_PER_M,
                ROAD_PAVEMENT_RELIEF,
            ),
            tile: Vec4::from_array(tile_multipliers()),
            pavement: Vec4::new(
                ROAD_PAVEMENT_ALBEDO[0],
                ROAD_PAVEMENT_ALBEDO[1],
                ROAD_PAVEMENT_ALBEDO[2],
                GROUND_TILE_M[3] / ROAD_AGGREGATE_TILE_M,
            ),
            road_dirt: Vec4::new(
                ROAD_DIRT_ALBEDO[0],
                ROAD_DIRT_ALBEDO[1],
                ROAD_DIRT_ALBEDO[2],
                ROAD_EDGE_WEAR,
            ),
            road_lod: Vec4::new(
                ROAD_FADE_START_M,
                ROAD_FADE_END_M,
                super::foliage::GRASS_FADE_START_M,
                super::foliage::GRASS_FADE_END_M,
            ),
            paint_yellow: Vec4::new(
                ROAD_PAINT_YELLOW[0],
                ROAD_PAINT_YELLOW[1],
                ROAD_PAINT_YELLOW[2],
                ROAD_PAINT_OPACITY,
            ),
            paint_white: Vec4::new(
                ROAD_PAINT_WHITE[0],
                ROAD_PAINT_WHITE[1],
                ROAD_PAINT_WHITE[2],
                ROAD_PAINT_OPACITY,
            ),
            paint_geometry: Vec4::new(
                super::road_markings::ROAD_PAINT_WIDTH_M,
                super::road_markings::ROAD_EDGE_OFFSET_M,
                ROAD_PAINT_VALID_EPS,
                0.0,
            ),
            rock_a: Vec4::new(ROCK_CELL_M, ROCK_FINE_M, ROCK_TILT, ROCK_FINE_TILT),
            rock_b: Vec4::new(ROCK_SHADE, ROCK_WEATHER_M, ROCK_WEATHER, ROCK_WARP_M),
            rock_c: Vec4::new(ROCK_CRACK_SHARE, ROCK_CRACK_W, ROCK_CRACK_DARK, ROCK_STREAK),
            rock_d: Vec4::new(
                ROCK_STREAK_W_M,
                ROCK_STREAK_H_M,
                ROCK_FACE_ON,
                ROCK_FACE_FULL,
            ),
            rock_e: Vec4::new(ROCK_SEAM, ROCK_FLAT_LEAN, ROCK_LEAN_ON, ROCK_LEAN_FULL),
            mottle: Vec4::new(MOTTLE_M, MOTTLE_AMP, 0.0, 0.0),
            aggregate: Vec4::new(AGGREGATE_GAIN, 0.0, 0.0, 0.0),
            cliff: Vec4::new(CLIFF_TAN_LO, CLIFF_TAN_HI, CLIFF_NOISE, CLIFF_NOISE_M),
            wall_planes: Vec4::new(WALL_PLANE_BLEND, PROJECT_TRI_COS, PROJECT_NORMAL_COS, 0.0),
            caustics: Vec4::new(
                sim_core::terrain::SEA_LEVEL,
                CAUSTIC_GAIN,
                CAUSTIC_FADE_FROM_M,
                CAUSTIC_FADE_TO_M,
            ),
            water_tint: Vec4::new(
                super::water::EXTINCT[0],
                super::water::EXTINCT[1],
                super::water::EXTINCT[2],
                SEABED_PATH,
            ),
            desert: Vec4::new(
                DESERT_SAND_ALBEDO[0],
                DESERT_SAND_ALBEDO[1],
                DESERT_SAND_ALBEDO[2],
                DRY_GRASS_MIX,
            ),
            dry_grass: Vec4::new(
                DRY_GRASS_ALBEDO[0],
                DRY_GRASS_ALBEDO[1],
                DRY_GRASS_ALBEDO[2],
                DESERT_GRASS_M,
            ),
            desert_rock: Vec4::new(
                DESERT_ROCK_ALBEDO[0],
                DESERT_ROCK_ALBEDO[1],
                DESERT_ROCK_ALBEDO[2],
                0.0,
            ),
            snow: Vec4::new(0.0, super::weather::NO_SNOW_FLOOR, SNOW_ALBEDO, SNOW_FADE_M),
            snow_band: super::weather::NO_SNOW_ARID
                .extend(SNOW_ARID_FADE)
                .extend(0.0),
            fel_ash: Vec4::new(
                FEL_ASH_ALBEDO[0],
                FEL_ASH_ALBEDO[1],
                FEL_ASH_ALBEDO[2],
                FEL_CRACK_SHARE,
            ),
            fel_dead: Vec4::new(
                FEL_DEAD_ALBEDO[0],
                FEL_DEAD_ALBEDO[1],
                FEL_DEAD_ALBEDO[2],
                FEL_CELL_M,
            ),
            fel_rock: Vec4::new(
                FEL_ROCK_ALBEDO[0],
                FEL_ROCK_ALBEDO[1],
                FEL_ROCK_ALBEDO[2],
                FEL_ROCK_ROUGH,
            ),
            fel_glow: Vec4::new(
                FEL_GLOW_DAY[0],
                FEL_GLOW_DAY[1],
                FEL_GLOW_DAY[2],
                FEL_PULSE_HZ,
            ),
            fel_shape: Vec4::new(FEL_CRACK_W, FEL_RIM_W, FEL_POOL_AT, FEL_POOL_M),
            blight_heart: Vec4::new(
                terrain::FEL_HEART.0,
                terrain::FEL_HEART.1,
                1.0 / terrain::CONTINENT_RADIUS,
                terrain::FEL_WANDER,
            ),
            blight_fel: Vec4::new(
                terrain::FEL_R.0,
                terrain::FEL_R.1,
                terrain::BEACH_MAX_H,
                sim_core::blight::COAST_M,
            ),
            blight_creep: Vec4::new(
                sim_core::blight::BAND,
                sim_core::blight::CREEP_DEPTH,
                sim_core::blight::RIDGE,
                sim_core::blight::WARD,
            ),
            blight_ridge: Vec4::new(
                sim_core::blight::RIDGE_LO_M,
                sim_core::blight::RIDGE_HI_M,
                sim_core::blight::WARD_IN_M,
                sim_core::blight::WARD_OUT_M,
            ),
            blight_disc: Vec4::new(
                sim_core::blight::OUTBREAK_WANDER,
                sim_core::blight::OUTBREAK_BAND,
                sim_core::blight::HOT_WANDER,
                sim_core::blight::HOT_BAND,
            ),
            blight_rim: Vec4::from_array(FRONT_RIM_T),
            // Wipe day's front until a moment is written, so an unwritten
            // material draws the heart alone.
            blight_now: Vec4::new(sim_core::blight::F0, 0.0, 0.0, 0.0),
            blight_ward: Vec4::ZERO,
            turf: Vec4::new(TURF_ALBEDO[0], TURF_ALBEDO[1], TURF_ALBEDO[2], std::env::var("GATES_TUNE_TURF_NEAR").ok().and_then(|v| v.parse().ok()).unwrap_or(TURF_NEAR)), // TUNE
            blight_discs: [Vec4::ZERO; BLIGHT_DISCS],
        }
    }
}

/// The four UV multipliers, derived from [`GROUND_TILE_M`] and [`UV_PER_M`].
///
/// Free-standing so `tests/ground_tiling.rs` can re-derive them from the two
/// constants rather than reading them back out of a built uniform — the same
/// arrangement `GRAIN_GAIN` has with the files it is measured from.
pub fn tile_multipliers() -> [f32; 4] {
    let mut m = [0.0f32; 4];
    for k in 0..4 {
        m[k] = 1.0 / (super::terrain_mesh::UV_PER_M * GROUND_TILE_M[k]);
    }
    m
}

// ── The rock face ──────────────────────────────────────────────────────────
//
// **A smooth heightfield lights a scarp as one value, and that is what the
// island's cliffs looked like: one pale sheet the height of a house** — every
// shelf edge `terrain::REMAP_LUT` manufactures, and every summit, at the one
// granite value `GROUND_ALBEDO` gives them (the operator's frames, 2026-09-23:
// *"our world is still weak terrain wise"*). The photograph cannot answer it:
// it tiles at 1.8 m and averages to its own mean by thirty.
//
// So the rock carries structure the mesh does not have, all of it procedural
// and all of it in world space, so a scarp and a flat summit cut the same
// blocks and no frame has to be chosen on a curved face:
//
//  - **blocks** — a jittered 3D lattice, read through a small warp so a
//    boundary wanders like a joint rather than ruling a polygon edge;
//  - **facets** — each block, and each block of a finer lattice inside it,
//    leans its normal by its own draw, so a face catches the sun in patches;
//  - **cracks** — a key per PAIR of blocks, and only [`ROCK_CRACK_SHARE`] of
//    pairs are cracks, so a crack runs the length of one boundary and stops;
//  - **weathering** — broad patches, the scale that reads across a valley;
//  - **streaks** — a lattice stretched tall, only on faces too steep for turf.
//
// **What it deliberately does not do, both measured on the capture before they
// were cut.** It moves no splat weight: a first cut let blocks decide rock
// against turf at the lip and foot, and the foot of every scarp read as white
// paving stones laid on grass. And it draws no joint on every boundary: an
// outline round every block (tried twice, thin and then pillowed) reads as
// crazy paving or a dry-stone wall, never as a cliff.
//
// **Brightness-neutral by construction**, the same promise `MACRO_AMP` makes:
// the value spread, the weathering and the streaks are each odd about the
// middle of a symmetric draw, so each has mean 1, and only the cracks darken —
// a sliver of the face, gone before a crack is a pixel wide. The mean granite
// is still `GROUND_ALBEDO[3]`, which `fill::GROUND_MIX` folds.
//
// **Everything fades with the block's size on screen** (the fragment's own
// footprint, taken before any branch), so a block a few pixels across stops
// leaning and a distant face does not sparkle.

/// Edge of one rock block, metres. Large enough that a scarp a player stands
/// under shows a handful, small enough that one across a valley still reads.
pub const ROCK_CELL_M: f32 = 7.0;
/// Edge of one fine block inside the large ones, metres.
pub const ROCK_FINE_M: f32 = 2.2;
/// How far a block's facet leans, as a tangent-plane offset per unit normal.
/// Each component of the draw is in ±1, so the lean tops out near 33°.
pub const ROCK_TILT: f32 = 0.38;
/// The fine blocks' lean, riding on the large blocks'.
pub const ROCK_FINE_TILT: f32 = 0.16;
/// Peak per-block departure of the face's value from its mean.
pub const ROCK_SHADE: f32 = 0.06;
/// Wavelength of the weathering patches, metres, and their peak departure.
pub const ROCK_WEATHER_M: f32 = 14.0;
pub const ROCK_WEATHER: f32 = 0.12;
/// How far the block lattice is pushed about before it is read, metres.
pub const ROCK_WARP_M: f32 = 1.6;
/// Share of block boundaries that are cracks.
pub const ROCK_CRACK_SHARE: f32 = 0.16;
/// A crack's half-width in block units, and how dark its centre goes.
///
/// At most 0.125: a crack shows from half a pixel wide, so it is gone by a
/// block footprint of `2 × ROCK_CRACK_W` pixels, inside the 0.25 where
/// `near_big` reaches zero and the shader skips the 7 m pass.
pub const ROCK_CRACK_W: f32 = 0.025;
const _: () = assert!(ROCK_CRACK_W <= 0.125);
pub const ROCK_CRACK_DARK: f32 = 0.2;
/// Peak departure of the streaks: darker in a streak, lighter between.
pub const ROCK_STREAK: f32 = 0.2;
/// The streak lattice's cell, metres: narrow across a face and tall down it.
pub const ROCK_STREAK_W_M: f32 = 1.6;
pub const ROCK_STREAK_H_M: f32 = 14.0;
/// `sin(tilt)` where streaks begin (~37°) and are full (~58°): water runs
/// down a face, it does not stripe a slope a player walks up.
pub const ROCK_FACE_ON: f32 = 0.6;
pub const ROCK_FACE_FULL: f32 = 0.85;
/// How far into a block its own lean and value hold, in block units. Nearer
/// a boundary than this they turn over to the neighbour's, and on it the two
/// are even, so a block has no outline. Stepped at the boundary, every
/// summit read as a camouflage of dark and pale blobs and every face as
/// polygons pasted on (operator, 2026-10-17: "the rock doesnt blend well").
pub const ROCK_SEAM: f32 = 0.25;
/// The share of the facets' lean a gentle summit keeps: a scarp breaks into
/// planes, a summit is not a field of slabs tipped at random…
pub const ROCK_FLAT_LEAN: f32 = 0.35;
/// …and `sin(tilt)` where the rest grows in (~20°) and is whole (~44°).
pub const ROCK_LEAN_ON: f32 = 0.35;
pub const ROCK_LEAN_FULL: f32 = 0.7;

/// The ground's mid-scale break-up, per pixel: wavelength, metres (a second
/// octave rides at 1/2.3 of it), and peak departure from the mean. It
/// replaced a hash per mesh vertex, which interpolated across each triangle
/// into blotches one vertex step wide — on sand, which has little grain of
/// its own to hide them, a mottle the eye read before the sand.
pub const MOTTLE_M: f32 = 6.0;
pub const MOTTLE_AMP: f32 = 0.05;

/// **The cliff is decided per pixel, not per vertex.** The classifier's cliff
/// veto is near-binary, and a binary weight interpolated across triangles
/// draws the triangles: a grass/rock edge of regular teeth one vertex step
/// wide (8 m on the far mesh). The mesh now carries the biome weights alone
/// (`terrain_mesh::heightfield` asks `splat_from` at zero slope) and the
/// shader forces rock from the interpolated normal's own tilt, with a
/// world-space noise moving the threshold so the edge wanders like scree.
///
/// Rise/run where rock starts and where it is full — `terrain`'s
/// `SPLAT_CLIFF_BAND` (0.95–1.43), so the rock the ground draws is the rock
/// the clutter and the scatter were told about. A first cut at 0.82 painted
/// granite over every 35° meadow and stood the litter's stalks on it.
pub const CLIFF_TAN_LO: f32 = 0.95;
pub const CLIFF_TAN_HI: f32 = 1.40;
/// How far the noise moves that threshold, in rise/run, peak to peak.
pub const CLIFF_NOISE: f32 = 0.45;
/// The noise's wavelength, metres; a second octave rides at 0.37 of it.
pub const CLIFF_NOISE_M: f32 = 6.0;

/// Where the wall planes turn on, as `sin(tilt)`.
///
/// The ground's UV is a planar XZ projection, so on a face of tilt θ the
/// photograph is stretched by `1/cos θ` along the fall line — 1.41× at 45°,
/// 2.9× at 70°, unbounded at vertical. The wall planes (vertical, at fixed
/// bearings 45° apart) are stretched by `1/sin θ` down the face instead, so
/// the two are exact complements and cross over at 45°. Below this gate the
/// wall taps are skipped entirely.
///
/// ⚠ **The wall planes are fixed world axes, never a frame built from the
/// face.** Until 2026-10-02 the wall tap read the vertical plane through the
/// face's own fall line (`DECISIONS.md` materials v4), `dot(wp.xz, across)`
/// with `across` from the per-pixel normal, so that the plane would rotate
/// through `|n.x| = |n.z|` rather than swap there. But a frame that turns with
/// the surface, multiplied by a world coordinate of ~1500 m, sweeps the UV
/// 8–77 m of photograph per metre of cliff wherever the face's bearing changes
/// (p10–p99 on the face north of 1510,640): fine lines along every contour,
/// the operator's "stretch lines horizontal" on every cliff. Fixed planes
/// blended by [`WALL_PLANE_BLEND`] rather than swapped have no seam, and four
/// of them leave no face more than 22.5° off one — 1.08× of stretch across it,
/// where the X and Z planes alone left √2 on a diagonal face, measured as a
/// horizontal smear on the face north of 580,2578.
///
/// ⚠ **Gated at 30°, not 45°.** At 45° the two weights are equal, so gating
/// there switched the wall tap on at a 50% share in one pixel — a hard seam
/// line along every 45° contour, which read as pencil lines on the hills. At
/// 30° the sharpened share is ~0.02%, so the step is invisible.
pub const WALL_ON: f32 = 0.5;

/// The exponent the two plane weights are raised to before blending.
///
/// Quilez's own prescribed value for the non-remapped form: a linear blend at a
/// 70° face still hands ~32% of the sample to the top plane while that plane is
/// stretched 2.9×, which is the smear the wall tap exists to remove. At k = 8
/// that share is 0.05%, and 45° and below are untouched because [`WALL_ON`]
/// owns them. Proposed default, not spoken.
///
/// **16 since 2026-09-28.** At 8 the two projections share every face between
/// ~35° and ~55°, and a hillside is mostly that band: two copies of one
/// photograph at two stretches, which read as cross-hatching over the whole
/// rock face. At 16 they share ~42°–48°.
pub const WALL_SHARPNESS: f32 = 16.0;

/// The share of each 45° step between two wall planes over which a face reads
/// both: the middle 15°. Everywhere else it reads the one plane within 15° of
/// its bearing, stretched at most 1.04× across it, so two thirds of a wall
/// pays for one plane's taps.
pub const WALL_PLANE_BLEND: f32 = 1.0 / 3.0;

/// The cosine between a triangle's own plane and its smoothed normal past
/// which the projection is chosen by the triangle (~20°), and within which
/// by the normal (~10°).
///
/// The planar and wall taps are stretched by the triangle they are laid on,
/// and the normal is a central difference across its neighbours. They agree
/// to 4° on 90% of steep ground, and part by 15–25° where the cliff relief
/// cuts a step one vertex wide: there the riser read the top plane, smeared
/// three to ten times down the fall line, and the tread beside it a wall
/// (operator, 2026-10-17: the smear at the foot of the faces). Inside 10° the
/// normal still decides, because it is continuous across a triangle edge and
/// a triangle's plane is not.
pub const PROJECT_TRI_COS: f32 = 0.94;
pub const PROJECT_NORMAL_COS: f32 = 0.985;

/// Two arrays — albedo, and normal/roughness/AO — and one sampler.
///
/// **One sampler for every layer, and that is the constraint that scales.**
/// Each map wants the identical tiling and anisotropy descriptor
/// `textures::tiling` builds, and a sampler each would have put this bind
/// group at 32 in the fragment stage before `StandardMaterial`'s own were
/// counted — far over the 16 a downlevel adapter guarantees. That argument
/// was made about samplers and it turned out to be true of TEXTURES too, on
/// the one adapter that is exactly the floor (WebGL2, and WebGPU once the
/// atmosphere's LUT is counted; the module header). So the twenty maps are two
/// `texture_2d_array`s now, and the count this material adds to the fragment
/// stage is two textures and one sampler — `tests/ground_splat.rs` holds the
/// pipeline to `render::quality::FRAGMENT_TEXTURES`.
///
/// ⚠ **The roughness slice cost no new VRAM and the AO slice DID**, which is
/// the one place those two otherwise-identical changes differed.
/// `textures::MapSet::load` had always loaded `<role>_rough.jpg`, so those
/// four were resident and uploaded from the day the maps landed — paid for and
/// unread, and binding them was free. `<role>_ao.jpg` was **not** loaded by
/// anything until 2026-08-25. The arrays reverse that once more: the twenty
/// sources are dropped once the two arrays exist (`textures::stack_ground`),
/// so the ground's residency is the arrays alone.
#[derive(Asset, AsBindGroup, TypePath, Clone)]
pub struct GroundSplat {
    #[uniform(100)]
    pub params: GroundSplatParams,
    /// The four albedo photographs, `Rgba8UnormSrgb`, in `terrain::splat`'s
    /// order, then the road's aggregate — **and the shared sampler, at 102.** The derive refuses a
    /// `sampler` attribute with no `texture` beside it, so the one sampler
    /// every array uses hangs off this one; which one is arbitrary and
    /// stable, and the arrays are built with an identical descriptor.
    #[texture(101, dimension = "2d_array")]
    #[sampler(102)]
    pub albedo: Handle<Image>,
    /// Every linear map, `Rgba8Unorm`, the same five-layer order per family:
    /// tangent-space normals at `0..5`, then at `textures::ROUGH_AO_LAYER0`
    /// roughness in R and ambient occlusion in G (B a constant 0, A a
    /// constant 255) — all DATA, loaded `is_srgb = false`.
    ///
    /// AO is **`ART.md` §4's MEDIUM scale** — the one occlusion term a light
    /// rig cannot supply, indirect only. Every ground role is in
    /// `textures::ROLES_WITH_AO`, and `textures::stack_ground` panics on one
    /// that is not rather than stacking an unresolved handle, which would
    /// sample BLACK and put the whole island in shadow.
    #[texture(103, dimension = "2d_array")]
    pub data: Handle<Image>,
    /// The cloud deck's coverage field (`sky::SoftField`, bytes packed four
    /// to a word), for its shadows on the land (`cloud_shadow.rs`), and
    /// their lanes. **A storage buffer, not a texture**: the arrays above
    /// already fill the sixteen sampled textures a downlevel adapter allows
    /// under the atmosphere (`the_ground_fits_the_fragment_texture_budget`),
    /// and a buffer counts against none of them. **Native only**, where the
    /// shadows are drawn. `cloud_shadow::NO_FIELD` until
    /// `cloud_shadow::drive` writes the deck's.
    #[cfg(not(target_arch = "wasm32"))]
    #[storage(104, read_only, visibility(fragment))]
    pub cloud_field: Handle<ShaderStorageBuffer>,
    #[cfg(not(target_arch = "wasm32"))]
    #[uniform(105)]
    pub cloud: super::cloud_shadow::CloudShadowParams,
}

impl GroundSplat {
    /// Bind the two arrays `textures::stack_ground` built.
    pub fn new(arrays: &GroundArrays) -> Self {
        Self {
            params: GroundSplatParams::new(),
            albedo: arrays.albedo.clone(),
            data: arrays.data.clone(),
            #[cfg(not(target_arch = "wasm32"))]
            cloud_field: super::cloud_shadow::NO_FIELD,
            #[cfg(not(target_arch = "wasm32"))]
            cloud: super::cloud_shadow::CloudShadowParams::off(),
        }
    }
}

impl MaterialExtension for GroundSplat {
    fn vertex_shader() -> ShaderRef {
        "shaders/ground_vertex.wgsl".into()
    }

    fn specialize(
        _pipeline: &MaterialExtensionPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        _key: MaterialExtensionKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        // Extend the existing layout: the prepass assigns different locations
        // from the forward pass and must keep its own mapping. Both layouts
        // use the complete interleaved mesh stride. Unused attributes are legal.
        let road = layout.0.get_layout(&[
            ATTRIBUTE_ROAD.at_shader_location(8),
            ATTRIBUTE_MARKINGS.at_shader_location(9),
            ATTRIBUTE_ZONE.at_shader_location(10),
        ])?;
        descriptor.vertex.buffers[0]
            .attributes
            .extend(road.attributes);
        // The deck's shadows, where the bind group carries the field
        // (`cloud_field`): the fragment imports `cloud_shadow.wgsl` on this.
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(fragment) = descriptor.fragment.as_mut() {
            fragment.shader_defs.push("CLOUD_SHADOWS".into());
        }
        Ok(())
    }

    fn fragment_shader() -> ShaderRef {
        SHADER.into()
    }

    /// The deferred path would need the same override and does not have it, so
    /// it is left to the forward path this client already runs on.
    fn deferred_fragment_shader() -> ShaderRef {
        ShaderRef::Default
    }
}
