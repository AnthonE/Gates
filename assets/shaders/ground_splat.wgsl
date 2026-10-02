// The ground's four identities, each with its own photograph.
//
// **What this replaces.** Until now every identity shared ONE greyscale detail
// map and ONE `perceptual_roughness`, so granite had stone's *value* and not
// stone's *surface* (`terrain_mesh.rs`'s own note, and `NOW.md` §0gm). A
// `StandardMaterial` has one base-colour slot, which is the limitation this
// file exists to remove.
//
// **All three channels are now the photograph's** — albedo relief, normal
// and, since 2026-08-16, roughness. The roughness maps were the last third and
// they were the cheapest: they had been loaded, uploaded and resident since the
// day the set landed, and nothing sampled them.
//
// **Three `texture_2d_array`s, not sixteen textures** (2026-09-12). A
// fragment stage may hold 16 sampled textures on WebGL2, counted across EVERY
// bind group in the pipeline — the view's shadow and environment maps,
// `StandardMaterial`'s six slots, and this material — and sixteen ground maps
// put this group alone at 22; the browser refused the pipeline layout. A layer
// costs no binding, so each identity is a layer of its family's array, in
// `terrain::splat`'s order, and the road's aggregate rides behind them as a
// fifth: albedo (5 layers), normal (5), and roughness with AO behind it (10:
// roughness 0–4, AO 5–9, `textures::AO_LAYER0`). A layer
// samples exactly as the standalone texture did — same filter, same chain,
// same bytes — so this is one shader for the desktop and the browser, and
// the native before/after capture is what says the desktop frame did not move.
//
// **Why the maps contribute LUMINANCE and never colour.** `ART.md` §7 bounds a
// mean-placing correction: a sourced map's colour deviation may not be
// stretched by more than ×1. Measured over the four ground sources the gain
// spans are grass 2.454, sand 2.073, litter 3.586, rock 1.08 (`Rock032`) —
// only rock clears it. So each map is reduced to its own mean-1 luminance field (span
// 1.000 by construction, because every channel becomes the same channel) and
// the colour stays entirely the authored splat's. The photograph contributes
// exactly the thing a noise field cannot encode: measured high-frequency
// relief. That is not a workaround for the rule; it is what the rule asks for.
//
// **The weights arrive in `COLOR`, not packed into `UV_1`.** Packing two `u8`
// per `f32` was scouted and is wrong: the rasterizer interpolates the PACKED
// value, and `floor(p / 256)` then mixes the low byte into the high one. It is
// exact at both vertices and up to 50% wrong in the middle of the triangle —
// i.e. precisely at identity boundaries, where it shows. `COLOR` carries four
// independently-interpolated weights instead, which also moves the identity mix
// from per-vertex to per-pixel.

#import bevy_pbr::{
    forward_io::{VertexOutput, FragmentOutput},
    mesh_view_bindings::view,
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{
        alpha_discard,
        apply_pbr_lighting,
        main_pass_post_lighting_processing,
        calculate_tbn_mikktspace,
    },
}

struct GroundSplat {
    // Per identity, in `terrain::splat`'s order — sand · grass · litter · rock:
    // `xyz` the authored LINEAR albedo (`terrain_mesh::GROUND_ALBEDO`). `w` is
    // RESERVED and zero — it carried the per-identity roughness scalar until
    // the roughness maps landed at 110–113, and roughness is now per texel.
    identity: array<vec4<f32>, 4>,
    // Per identity, `1 / linear-luma mean` of its albedo map, so each map
    // delivers a mean of 1 and multiplies the authored colour without moving it.
    gain: vec4<f32>,
    // x = WET_VALUE, y = WET_SATURATION, z = ALBEDO_LUMA_FLOOR, w = blend depth.
    tune: vec4<f32>,
    // x = HEIGHT_INFLUENCE, y = NORMAL_Z_FLOOR, z = WET_ROUGH, w = road aggregate share.
    // Passed in rather than declared here: a knob that lives only in a shader
    // is one the knob registry cannot see, and `ci/gates.sh` refuses its
    // `DECISIONS.md` row.
    blend: vec4<f32>,
    // x = WALL_ON, y = WALL_SHARPNESS, z = UV_PER_M, w = pavement relief.
    // ⚠ This struct's field list and order must match `GroundSplatParams`
    // exactly — a uniform whose two sides disagree about layout is garbage in
    // every field after the first mismatch, and nothing about that failure
    // looks like a layout bug. `tests/ground_splat.rs` scrapes both and fails
    // on a disagreement.
    wall: vec4<f32>,
    // Per identity, the factor the mesh UV is multiplied by so that identity
    // repeats every `terrain_mesh::GROUND_TILE_M[k]` metres rather than at the
    // shared 4 m reference — `1 / (UV_PER_M * tile_m)`. Sand and rock are 1.0.
    // A photograph has an authored real-world size and the four sources do not
    // share one; drawing them all at 4 m put `forrest_ground_01` at 2× life
    // size and `brown_mud_leaves_01` at 3×.
    tile: vec4<f32>,
    // xyz linear albedo; w fine-aggregate UV multiplier.
    pavement: vec4<f32>,
    // xyz linear albedo; w bounded edge erosion.
    road_dirt: vec4<f32>,
    // x/y = fade start/end; the far lattice cannot resolve the road.
    road_lod: vec4<f32>,
    paint_yellow: vec4<f32>,
    paint_white: vec4<f32>,
    paint_geometry: vec4<f32>,
    // The rock face (`ground_splat::ROCK_*`): x = block size (m), y = fine
    // block size (m), z = block tilt, w = fine tilt.
    rock_a: vec4<f32>,
    // x = per-block value spread, y = weathering wavelength (m), z = its
    // strength, w = how far the block lattice is warped (m).
    rock_b: vec4<f32>,
    // x = share of block boundaries that are cracks, y = crack half-width
    // (cells), z = crack darkening, w = streak strength.
    rock_c: vec4<f32>,
    // x/y = streak lattice width/height (m), z/w = `sin(tilt)` where streaks
    // start / are full.
    rock_d: vec4<f32>,
    // x = the road aggregate's own `1 / linear-luma mean` (`AGGREGATE_GAIN`):
    // it is layer 4, not an identity, so `gain` has no slot for it. yzw
    // reserved and zero.
    aggregate: vec4<f32>,
    // The per-pixel cliff (`ground_splat::CLIFF_*`): x/y = rise/run where rock
    // starts / is full, z = how far the noise moves that, w = its wavelength (m).
    cliff: vec4<f32>,
    // x = `WALL_PLANE_BLEND`: the share of each 45° step between two wall
    // planes over which both are read. yzw reserved and zero.
    wall_planes: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> splat: GroundSplat;
// The four albedo photographs, one layer each: sand 0 · grass 1 · litter 2 ·
// rock 3, the same order everywhere in this file — and the road's aggregate at
// 4, which no splat weight reaches.
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var albedo_maps: texture_2d_array<f32>;
// **One sampler for every layer of every array.** Every map wants the same
// tiling/anisotropy descriptor, and a sampler each would have put this
// material at 32 samplers in the fragment stage on top of `StandardMaterial`'s
// own — far over the 16 a downlevel adapter guarantees. Textures were called
// the cheap axis here until WebGL2 said 16 of those too (the header), which is
// why the maps are arrays now and this is still one sampler.
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var ground_sampler: sampler;
// The tangent-space normal maps, same layer order, aggregate at 4.
@group(#{MATERIAL_BIND_GROUP}) @binding(103) var normal_maps: texture_2d_array<f32>;
// Roughness at layers 0–4 and ambient occlusion at 5–9 (`textures::AO_LAYER0`).
// Both greyscale and loaded `is_srgb = false` because a roughness map is DATA —
// decoding one as sRGB would bend every value toward the dark end and the
// ground would read uniformly glossy. AO is `ART.md` §4's MEDIUM scale, the
// one occlusion term a light rig cannot supply; all four shipped in every depot
// and were sampled by nothing until 2026-08-25.
@group(#{MATERIAL_BIND_GROUP}) @binding(104) var rough_ao_maps: texture_2d_array<f32>;

const LUMA: vec3<f32> = vec3<f32>(0.2126, 0.7152, 0.0722);

// `terrain_mesh::wetted`, ported verbatim — darker, and more saturated about its
// own luma. The Rust side stays the reference and `tests/ground_splat.rs` holds
// the two together over a sweep; if you change one, change both.
fn wetted(c: vec3<f32>, wet: f32) -> vec3<f32> {
    if wet <= 0.0 {
        return c;
    }
    let luma = dot(c, LUMA);
    // The soak, floored: a surface already at or under the band's dark end does
    // not get darker, and one above it may not be taken through.
    var value = 1.0;
    if luma > splat.tune.z {
        value = max(1.0 - wet * (1.0 - splat.tune.x), splat.tune.z / luma);
    }
    let chroma = 1.0 + wet * splat.tune.y;
    return max((vec3(luma) + (c - vec3(luma)) * chroma) * value, vec3(0.0));
}

// A tangent-space normal as a surface gradient. Summing GRADIENTS is what makes
// a blend of normals mean anything: averaging the vectors themselves pulls every
// mix toward the flat +Z and quietly flattens exactly the relief this material
// was built to deliver.
fn to_gradient(n: vec3<f32>) -> vec2<f32> {
    return n.xy / max(n.z, splat.blend.y);
}

fn unpack_normal(t: vec4<f32>) -> vec3<f32> {
    // Two-channel reconstruction would need the material flag; these are plain
    // three-channel tangent-space maps, so the decode is the whole of it.
    return normalize(t.xyz * 2.0 - 1.0);
}

// Analytic box filter preserves a thin stripe's area at grazing angles.
fn paint_line(distance: f32, width: f32, footprint: f32) -> f32 {
    let half = width * 0.5;
    let pixel = max(footprint, 1e-5);
    return clamp((min(distance + pixel * 0.5, half)
        - max(distance - pixel * 0.5, -half)) / pixel, 0.0, 1.0);
}

// --- The rock face: blocks, cracks and streaks ------------------------------
//
// Every number that shapes the face arrives in the uniform (`rock_a`–`rock_d`)
// for the reason `blend` gives: a knob that lives only in a shader is one the
// knob registry cannot see. The literals left below are the octave ratios and
// the footprint fades — structure, not taste.

// Chris Wellons' lowbias32: a full-avalanche integer hash in five ops.
fn lowbias32(x0: u32) -> u32 {
    var x = x0;
    x = x ^ (x >> 16u);
    x = x * 0x7feb352du;
    x = x ^ (x >> 15u);
    x = x * 0x846ca68bu;
    x = x ^ (x >> 16u);
    return x;
}

fn cell_key(c: vec3<i32>) -> u32 {
    return lowbias32(bitcast<u32>(c.x) * 73856093u
        ^ bitcast<u32>(c.y) * 19349663u
        ^ bitcast<u32>(c.z) * 83492791u);
}

fn unit_of(h: u32) -> f32 {
    return f32(h >> 8u) * (1.0 / 16777216.0);
}

// Three independent draws in [0, 1) off one cell key.
fn draw3(h: u32) -> vec3<f32> {
    let a = lowbias32(h ^ 0x9e3779b9u);
    let b = lowbias32(a ^ 0x85ebca6bu);
    let c = lowbias32(b ^ 0xc2b2ae35u);
    return vec3<f32>(unit_of(a), unit_of(b), unit_of(c));
}

struct RockCell {
    // The nearest block's own key, for every per-block draw.
    key: u32,
    // The second-nearest block's, so the boundary between the two has a key.
    key2: u32,
    // Distance to the nearest block boundary, in cell units.
    edge: f32,
}

// Nearest-feature cells over 3D world space, so a steep face and a flat
// outcrop cut the same blocks and no frame has to be chosen on the surface.
fn rock_cell(p: vec3<f32>) -> RockCell {
    let base = vec3<i32>(floor(p));
    let f = p - floor(p);
    var d1 = 1e9;
    var d2 = 1e9;
    var f1 = vec3<f32>(0.0);
    var f2 = vec3<f32>(0.0);
    var k1 = 0u;
    var k2 = 0u;
    for (var k: i32 = -1; k <= 1; k = k + 1) {
        for (var j: i32 = -1; j <= 1; j = j + 1) {
            for (var i: i32 = -1; i <= 1; i = i + 1) {
                let o = vec3<i32>(i, j, k);
                let key = cell_key(base + o);
                let pt = vec3<f32>(o) + vec3<f32>(0.15) + draw3(key) * 0.7;
                let d = dot(pt - f, pt - f);
                if d < d1 {
                    d2 = d1;
                    f2 = f1;
                    k2 = k1;
                    d1 = d;
                    f1 = pt;
                    k1 = key;
                } else if d < d2 {
                    d2 = d;
                    f2 = pt;
                    k2 = key;
                }
            }
        }
    }
    let n = normalize(f2 - f1);
    var out: RockCell;
    out.key = k1;
    out.key2 = k2;
    out.edge = max(-dot(f - 0.5 * (f1 + f2), n), 0.0);
    return out;
}

// Value noise over a 3D lattice, quintic-faded, in [0, 1].
fn value3(p: vec3<f32>) -> f32 {
    let i = vec3<i32>(floor(p));
    let f = p - floor(p);
    let u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    let n000 = unit_of(cell_key(i));
    let n100 = unit_of(cell_key(i + vec3<i32>(1, 0, 0)));
    let n010 = unit_of(cell_key(i + vec3<i32>(0, 1, 0)));
    let n110 = unit_of(cell_key(i + vec3<i32>(1, 1, 0)));
    let n001 = unit_of(cell_key(i + vec3<i32>(0, 0, 1)));
    let n101 = unit_of(cell_key(i + vec3<i32>(1, 0, 1)));
    let n011 = unit_of(cell_key(i + vec3<i32>(0, 1, 1)));
    let n111 = unit_of(cell_key(i + vec3<i32>(1, 1, 1)));
    let x00 = mix(n000, n100, u.x);
    let x10 = mix(n010, n110, u.x);
    let x01 = mix(n001, n101, u.x);
    let x11 = mix(n011, n111, u.x);
    return mix(mix(x00, x10, u.y), mix(x01, x11, u.y), u.z);
}

// --- The wall planes ----------------------------------------------------------

// `rough_ao_maps`' first AO layer — `textures::AO_LAYER0`, which
// `tests/ground_tiling.rs` holds this to.
const AO_LAYER0: i32 = 5;

// The vertical planes a steep face reads its maps off: four, at fixed
// bearings 45° apart (`wall_axis`), of which a pixel blends the two nearest
// its face — so no face is more than 22.5° off a plane and its photograph is
// stretched at most 1.08× across it. Each UV is LINEAR in the world position,
// so its gradients are exact and nothing turns with the surface. Everything
// is at the 4 m reference; `wall_tap` applies the tile.
struct Wall {
    // Each plane's u axis (`wall_axis`), UV and gradients.
    axis_a: vec2<f32>,
    uv_a: vec2<f32>,
    dx_a: vec2<f32>,
    dy_a: vec2<f32>,
    axis_b: vec2<f32>,
    uv_b: vec2<f32>,
    dx_b: vec2<f32>,
    dy_b: vec2<f32>,
    // Plane b's share of the wall; plane a takes the rest.
    share_b: f32,
}

// The u axis of wall plane `k` (0..4): the horizontal across a face whose
// normal bears `k × 45°` from +X in the XZ plane. A constant per plane, which
// is what keeps every wall UV linear in `wp`.
fn wall_axis(k: i32) -> vec2<f32> {
    var axes = array<vec2<f32>, 4>(
        vec2<f32>(0.0, 1.0),
        vec2<f32>(-0.70710678, 0.70710678),
        vec2<f32>(-1.0, 0.0),
        vec2<f32>(-0.70710678, -0.70710678),
    );
    return axes[k];
}

// One identity's four maps, read off the wall planes.
struct WallTap {
    albedo: vec4<f32>,
    // The photograph's relief as a world-space offset, not yet laid in the
    // surface: each plane's tangent-space gradient on that plane's own axes.
    relief: vec3<f32>,
    rough: f32,
    ao: f32,
}

// One plane's four maps, weighted by its share. Explicit-gradient samples
// only: this runs under the non-uniform wall branch, where an implicit
// derivative is undefined.
fn wall_plane(uv0: vec2<f32>, dx0: vec2<f32>, dy0: vec2<f32>, axis: vec2<f32>, layer: i32, tile: f32, k: f32) -> WallTap {
    let uv = uv0 * tile;
    let dx = dx0 * tile;
    let dy = dy0 * tile;
    var t: WallTap;
    t.albedo = k * textureSampleGrad(albedo_maps, ground_sampler, uv, layer, dx, dy);
    let g = to_gradient(unpack_normal(textureSampleGrad(normal_maps, ground_sampler, uv, layer, dx, dy)));
    // u runs along the plane's axis and the image's up is world up.
    t.relief = k * vec3<f32>(g.x * axis.x, g.y, g.x * axis.y);
    t.rough = k * textureSampleGrad(rough_ao_maps, ground_sampler, uv, layer, dx, dy).r;
    t.ao = k * textureSampleGrad(rough_ao_maps, ground_sampler, uv, layer + AO_LAYER0, dx, dy).r;
    return t;
}

fn wall_tap(wall: Wall, layer: i32, tile: f32) -> WallTap {
    var t: WallTap;
    if wall.share_b < 1.0 {
        t = wall_plane(wall.uv_a, wall.dx_a, wall.dy_a, wall.axis_a, layer, tile, 1.0 - wall.share_b);
    }
    if wall.share_b > 0.0 {
        let b = wall_plane(wall.uv_b, wall.dx_b, wall.dy_b, wall.axis_b, layer, tile, wall.share_b);
        t.albedo = t.albedo + b.albedo;
        t.relief = t.relief + b.relief;
        t.rough = t.rough + b.rough;
        t.ao = t.ao + b.ao;
    }
    return t;
}

@fragment
fn fragment(in: VertexOutput, @location(8) road: vec2<f32>, @location(9) markings: vec4<f32>, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    // Keep the standard view vector, flags and material coverage. The splat
    // assigns its surface colour below.
    // COLOR.a is the rock weight, not opacity. Only the material's resident
    // chunk mask owns coverage; keep the original weights for the splat below.
    var material_in = in;
    material_in.color.a = 1.0;
    var pbr_input = pbr_input_from_standard_material(material_in, is_front);
    pbr_input.material.base_color = alpha_discard(pbr_input.material, pbr_input.material.base_color);

    // One projection, four densities. `in.uv` is the mesh's shared planar XZ
    // UV at the 4 m reference (`terrain_mesh::UV_PER_M`); `splat.tile` spreads
    // each identity to its own photograph's authored size. Every tap of an
    // identity's maps — albedo, roughness, normal, AO, and the wall taps below
    // — must use ITS tile and no other, or the relief stops being registered
    // with the colour it came from. Derivatives are implicit here and scale
    // with the UV, so mip selection follows for free; `wall_tap` takes its
    // gradients explicitly and has to scale them by hand.
    let uv0 = in.uv * splat.tile.x;
    let uv1 = in.uv * splat.tile.y;
    let uv2 = in.uv * splat.tile.z;
    let uv3 = in.uv * splat.tile.w;
    // A separate, fixed projection: interpolating UV scale would sweep many
    // texture repeats across the verge. Only the sampled surfaces are blended.
    // Its own photograph, layer 4 (`Gravel004`): the rock identity is a slab
    // now, and a road is aggregate.
    let uv_road = in.uv * splat.tile.w * splat.pavement.w;
    let road_albedo = textureSample(albedo_maps, ground_sampler, uv_road, 4);
    let road_normal = textureSample(normal_maps, ground_sampler, uv_road, 4);
    let road_rough = textureSample(rough_ao_maps, ground_sampler, uv_road, 4).r;
    let road_ao = textureSample(rough_ao_maps, ground_sampler, uv_road, 9).r;
    var a0 = textureSample(albedo_maps, ground_sampler, uv0, 0);
    var a1 = textureSample(albedo_maps, ground_sampler, uv1, 1);
    var a2 = textureSample(albedo_maps, ground_sampler, uv2, 2);
    var a3 = textureSample(albedo_maps, ground_sampler, uv3, 3);

    // --- Triplanar: the planes a slope needs ---------------------------------
    //
    // `in.uv` is a planar XZ projection, so on a face of tilt θ the photograph
    // is stretched by `1/cos θ` along the fall line — 2.9× at 70°, unbounded at
    // vertical. Above `WALL_ON` every map is also read off fixed vertical
    // planes (`Wall`), stretched `1/sin θ` down the face and at most 1.08×
    // across it.
    //
    // ⚠ **The wall planes are FIXED world axes, and that is load-bearing.** The
    // wall tap used to read the plane through the face's own fall line,
    // `dot(wp.xz, across)` with `across` from the per-pixel normal. A frame that
    // turns with the surface, multiplied by a world coordinate of order 1500 m,
    // sweeps the UV 8–77 m of photograph per metre of cliff (p10–p99, the face
    // north of 1510,640) wherever the face's bearing changes; with the mip
    // picked from the true footprint, the photograph was drawn squeezed
    // twenty-fold into fine lines along the contours — the "horizontal stretch
    // lines" on every cliff (operator, 2026-10-02). An axis plane's UV is
    // linear in `wp`, so its gradients are exact and nothing can sweep it.
    //
    // Derivatives are undefined under non-uniform control flow and the wall
    // branch below is non-uniform by construction, so they are taken here.
    let wp = in.world_position.xyz;
    let dp_dx = dpdx(wp);
    let dp_dy = dpdy(wp);

    let wn = normalize(in.world_normal);
    let sin_tilt = length(wn.xz);
    let cos_tilt = abs(wn.y);
    // `pow(cos, k)` against `pow(sin, k)`: the two foreshortenings are exact
    // complements, so this crosses over at 45° by construction rather than by a
    // tuned threshold.
    let w_top = pow(cos_tilt, splat.wall.y);
    let w_wall = pow(sin_tilt, splat.wall.y);
    var wall_mix = 0.0;
    if sin_tilt > splat.wall.x {
        wall_mix = w_wall / max(w_top + w_wall, 1e-6);
    }

    // **Height blend, and the classifier stays soft.** A linear blend of four
    // weights reads as a wash where two identities meet; a height blend lets the
    // louder surface's own relief win the contested band, which is what a real
    // boundary between turf and scree looks like. The map's luminance is the
    // height proxy — displacement is deliberately not sourced
    // (`assets/textures/MANIFEST.md`) — and `depth` is deliberately generous:
    // sharpening this produces bubble-shaped regions along every seam.
    // **The height only breaks ties; it never outvotes the classifier.**
    // `splat_from` delivers near-pure identities (max weight p50 = 1.000, 92.2%
    // of samples above 0.8), so the weights already decide almost everywhere
    // and the only place a height belongs is the narrow contested band where
    // two of them are close. Centring the height on 0 and scaling it by
    // `HEIGHT_INFLUENCE` is what bounds it to that band: at ±0.15 it cannot
    // overturn a weight gap wider than 0.3.
    // **The height is the GRAIN centred on zero, not the raw luma.** Every
    // grain field has a mean of 1 by construction, so `grain − 1` is each map's
    // own relief about its own mean and no identity carries a systematic
    // advantage. Raw luma does: grass's mean (0.248) is 2.4× litter's (0.103)
    // and rock's (0.100), so it would win contested bands on brightness alone.
    // Clamped because a gain of ~10 lets one litter or rock texel reach a
    // grain of ~4, and an outlier texel may sharpen a seam without being
    // allowed to move it.
    //
    // **Measured as a no-op, and kept anyway.** Swapping raw luma for this
    // moved a six-frame capture by +0.1% contrast and 0.05 luma — nothing. The
    // reason is the line above: `splat_from` is near-binary (92.2% of samples
    // over 0.8), so the contested band this arbitrates is a sliver of the
    // island. It stays because "no identity wins by being brighter" is a
    // property worth having when the classifier eventually softens, not because
    // it bought a frame anything today.
    var w = clamp(in.color, vec4(0.0), vec4(1.0));

    // --- The cliff, per pixel -------------------------------------------------
    //
    // The mesh carries the biome weights alone; the cliff veto is decided here
    // from the interpolated normal's own tilt. Decided per vertex it was a
    // near-binary weight interpolated across triangles, and that draws the
    // triangles: a grass/rock edge of regular teeth one vertex step wide. A
    // world-space noise moves the threshold so the edge wanders like scree.
    let tan_tilt = sin_tilt / max(cos_tilt, 0.05);
    let reach = splat.cliff.z * 0.5;
    if tan_tilt > splat.cliff.x - reach {
        var cn = 0.0;
        if tan_tilt < splat.cliff.y + reach {
            let q = wp / splat.cliff.w;
            cn = value3(q) * 0.65 + value3(q * 2.7 + vec3<f32>(3.1, 7.7, 1.3)) * 0.35 - 0.5;
        }
        let cliff = smoothstep(splat.cliff.x, splat.cliff.y, tan_tilt + cn * splat.cliff.z);
        w = w * (1.0 - cliff) + vec4<f32>(0.0, 0.0, 0.0, cliff);
    }

    // The wall taps, per identity and only where it has weight: on a cliff the
    // veto above has left rock alone, so a cliff pays one identity's four maps
    // on one plane, or two where it turns between planes. Skipped whole below `WALL_ON`, which
    // is nearly every flat metre of the island. `wall_tap` samples with
    // explicit gradients, which is what makes the branch legal.
    var wm = vec4<f32>(0.0);
    var t0: WallTap;
    var t1: WallTap;
    var t2: WallTap;
    var t3: WallTap;
    if wall_mix > 0.0 {
        let s = splat.wall.z;
        // The face's bearing, folded to half a turn (a plane reads both of
        // its sides) and counted in the 45° steps between planes: plane `ka`
        // and the next one round share it, over the middle
        // `WALL_PLANE_BLEND` of each step and nowhere else.
        var bearing = atan2(wn.z, wn.x);
        if bearing < 0.0 {
            bearing = bearing + 3.14159265;
        }
        let steps = bearing / 0.78539816;
        let ka = min(i32(steps), 3);
        let kb = (ka + 1) % 4;
        var wall: Wall;
        wall.share_b = clamp((steps - f32(ka) - 0.5) / splat.wall_planes.x + 0.5, 0.0, 1.0);
        wall.axis_a = wall_axis(ka);
        wall.uv_a = vec2<f32>(dot(wp.xz, wall.axis_a), -wp.y) * s;
        wall.dx_a = vec2<f32>(dot(dp_dx.xz, wall.axis_a), -dp_dx.y) * s;
        wall.dy_a = vec2<f32>(dot(dp_dy.xz, wall.axis_a), -dp_dy.y) * s;
        wall.axis_b = wall_axis(kb);
        wall.uv_b = vec2<f32>(dot(wp.xz, wall.axis_b), -wp.y) * s;
        wall.dx_b = vec2<f32>(dot(dp_dx.xz, wall.axis_b), -dp_dx.y) * s;
        wall.dy_b = vec2<f32>(dot(dp_dy.xz, wall.axis_b), -dp_dy.y) * s;
        if w.x > 0.0 {
            t0 = wall_tap(wall, 0, splat.tile.x);
            wm.x = wall_mix;
        }
        if w.y > 0.0 {
            t1 = wall_tap(wall, 1, splat.tile.y);
            wm.y = wall_mix;
        }
        if w.z > 0.0 {
            t2 = wall_tap(wall, 2, splat.tile.z);
            wm.z = wall_mix;
        }
        if w.w > 0.0 {
            t3 = wall_tap(wall, 3, splat.tile.w);
            wm.w = wall_mix;
        }
    }
    a0 = mix(a0, t0.albedo, wm.x);
    a1 = mix(a1, t1.albedo, wm.y);
    a2 = mix(a2, t2.albedo, wm.z);
    a3 = mix(a3, t3.albedo, wm.w);

    // Each map's raw linear luminance, in [0, 1]. This is the HEIGHT.
    let luma = vec4<f32>(
        dot(a0.rgb, LUMA),
        dot(a1.rgb, LUMA),
        dot(a2.rgb, LUMA),
        dot(a3.rgb, LUMA),
    );
    // The same field with its mean placed at 1. This is the GRAIN — what
    // multiplies the authored colour.
    //
    // **These two must not be the same vector.** The gains run 3.7 to 9.7, so a
    // bright litter texel reaches a grain of ~4 while a weight can only ever
    // reach 1: feed the grain to the height blend below and it resolves
    // whichever texture happens to be brightest at that texel, ignoring the
    // classifier entirely and painting a four-way random mosaic. That is an
    // arithmetic argument and it is the whole of the reason — the first
    // before/after capture run for it compared two different parts of the
    // island (the shard hashes a spawn per player id unless `dev_spawn` pins
    // it), so it measured a place and not a change.
    let grain = luma * splat.gain;

    // --- The rock face ------------------------------------------------------
    //
    // A smooth heightfield lights a scarp as one value, so the rock carries
    // its own structure: facets at two scales that each catch the sun at their
    // own angle, a few long cracks, weathering, and streaks down the steep
    // faces. Nothing here moves a weight — the classifier still decides where
    // rock is; this decides what rock looks like.
    let fp_m = max(length(dp_dx), length(dp_dy));
    let big_px = fp_m / splat.rock_a.x;
    let fine_px = fp_m / splat.rock_a.y;
    let near_big = 1.0 - smoothstep(0.05, 0.25, big_px);
    let near_fine = 1.0 - smoothstep(0.05, 0.25, fine_px);
    var rock_shade = 1.0;
    var rock_tilt = vec3<f32>(0.0);
    if w.w > 0.002 {
        var shade = 1.0;
        var t_big = vec3<f32>(0.0);
        // Skipped where it would add nothing: every output of the 7 m pass is
        // scaled by `near_big`, and a crack needs half a pixel of width, which
        // `ROCK_CRACK_W` keeps inside `near_big` (a const assert holds it).
        // On distant rock this was ~140 hashes a pixel for a no-op.
        if near_big > 0.0 {
            // The block lattice is read through a warp so a boundary wanders
            // like a joint and is never a ruled polygon edge.
            let warp = vec3<f32>(value3(wp / 4.0), value3(wp / 4.0 + vec3<f32>(31.7, 11.3, 5.9)), value3(wp / 4.0 + vec3<f32>(7.1, 23.9, 17.3))) * 2.0 - 1.0;
            let big = rock_cell((wp + warp * splat.rock_b.w) / splat.rock_a.x);
            let d = draw3(big.key);
            shade = 1.0 + splat.rock_b.x * (2.0 * d.y - 1.0) * near_big;
            // Only some boundaries are cracks: a key per PAIR of blocks, so a
            // crack runs the length of the boundary it chose and stops.
            let pair = cell_key(vec3<i32>(bitcast<i32>(min(big.key, big.key2)), bitcast<i32>(max(big.key, big.key2)), 7));
            if unit_of(pair) < splat.rock_c.x {
                let crack_px = splat.rock_c.y / max(big_px, 1e-5);
                let on = smoothstep(0.5, 1.5, crack_px);
                shade = shade * (1.0 - splat.rock_c.z * on * (1.0 - smoothstep(0.0, splat.rock_c.y, big.edge)));
            }
            t_big = (draw3(big.key ^ 0x68e31da4u) * 2.0 - 1.0) * (splat.rock_a.z * near_big);
        }
        var t_fine = vec3<f32>(0.0);
        if near_fine > 0.0 {
            let fine = rock_cell(wp / splat.rock_a.y + vec3<f32>(17.3, 5.1, 9.7));
            t_fine = (draw3(fine.key ^ 0x1b873593u) * 2.0 - 1.0) * (splat.rock_a.w * near_fine);
            shade = shade * (1.0 + 0.5 * splat.rock_b.x * (2.0 * draw3(fine.key).y - 1.0) * near_fine);
        }
        rock_tilt = t_big + t_fine;
        // Weathering: broad patches, the scale a player reads from across a
        // valley.
        let wx = value3(wp / splat.rock_b.y) * 0.65 + value3(wp / (splat.rock_b.y * 0.37)) * 0.35;
        shade = shade * (1.0 + splat.rock_b.z * (2.0 * wx - 1.0));
        // Water streaks down a face: a lattice stretched nine times taller than
        // it is wide, on faces too steep for turf.
        let face = smoothstep(splat.rock_d.z, splat.rock_d.w, sin_tilt);
        if face > 0.0 {
            let sw = splat.rock_d.x;
            let sh = splat.rock_d.y;
            let st = 0.65 * value3(vec3<f32>(wp.x / sw, wp.y / sh, wp.z / sw))
                + 0.35 * value3(vec3<f32>(wp.x / (sw * 0.44), wp.y / (sh * 0.36), wp.z / (sw * 0.44)));
            // Odd about 0.5, and the lattice's values are symmetric about 0.5,
            // so the face is lightened between streaks exactly as much as it
            // is darkened in them: the mean stays the classifier's granite.
            let streak = 2.0 * smoothstep(0.38, 0.62, st) - 1.0;
            shade = shade * (1.0 - splat.rock_c.w * face * streak);
        }
        rock_shade = shade;
    }

    let relief = clamp(grain - vec4(1.0), vec4(-1.0), vec4(1.0));
    let h = w + relief * splat.blend.x;
    let peak = max(max(h.x, h.y), max(h.z, h.w)) - splat.tune.w;
    let b = max(h - vec4(peak), vec4(0.0));
    let terrain_bw = b / max(b.x + b.y + b.z + b.w, 1e-4);
    // Erode only the edge: centres stay covered, off-road stays untouched.
    // Dust is the existing road splat underneath, so a broken edge exposes a
    // shoulder rather than a black transparent seam. No independent noise.
    let road_grain = dot(road_albedo.rgb, LUMA) * splat.aggregate.x;
    let wear = clamp(road_grain - 1.0, -1.0, 1.0) * splat.road_dirt.w;
    let road_fade = 1.0 - smoothstep(splat.road_lod.x, splat.road_lod.y,
        length(in.world_position.xz - view.world_position.xz));
    let paving = clamp(road.x + wear * 4.0 * road.x * (1.0 - road.x), 0.0, 1.0) * road_fade;
    let dirt = clamp(road.y, 0.0, 1.0) * (1.0 - paving) * road_fade;
    let ground = 1.0 - paving - dirt;
    // Both road tiers use fine aggregate. The branch mixes in loose grit;
    // borrowing the terrain's 4 m scree here made it read as cobblestones.
    // These same weights own colour grain, roughness and occlusion.
    let bw = terrain_bw * ground + vec4(dirt * (1.0 - splat.blend.w), 0.0, 0.0, 0.0);
    let road_weight = paving + dirt * splat.blend.w;

    // The authored colour, per pixel rather than per vertex.
    var base = vec3(0.0);
    for (var i = 0u; i < 4u; i = i + 1u) {
        var k = terrain_bw[i];
        if i == 3u {
            k = k * rock_shade;
        }
        base = base + splat.identity[i].xyz * k;
    }
    base = base * ground + splat.pavement.xyz * paving + splat.road_dirt.xyz * dirt;
    // Coordinates extend into the shoulder; validity is independent. A
    // triangle touching a rejected interval or branch mouth stays unpainted.
    // Phase is cyclic; no wrapped scalar can create a seam stripe.
    let footprint = fwidth(markings.x);
    // Use BOTH phase components: the cosine derivative alone vanishes at
    // each dash centre and would alias when several dashes fit in a pixel.
    let phase = markings.yz;
    let phase_dx = dpdx(phase);
    let phase_dy = dpdy(phase);
    let phase_len2 = max(dot(phase, phase), 1e-5);
    let phase_footprint = (abs(phase.y * phase_dx.x - phase.x * phase_dx.y)
        + abs(phase.y * phase_dy.x - phase.x * phase_dy.y)) / phase_len2;
    let phase_pixel = max(fwidth(markings.z), 1e-5);
    let resolved_dash = smoothstep(-phase_pixel * 0.5, phase_pixel * 0.5, markings.z);
    // A half-period footprint is the Nyquist limit. Fade from quarter-period
    // resolution to the exact mean coverage (equal paint and gap lengths).
    let dash = mix(resolved_dash, 0.5,
        smoothstep(1.57079632679, 3.14159265359, phase_footprint));
    let center_paint = paint_line(markings.x, splat.paint_geometry.x, footprint) * dash;
    let edge_paint = paint_line(abs(markings.x) - splat.paint_geometry.y,
        splat.paint_geometry.x, footprint);
    let paint_valid = step(1.0 - splat.paint_geometry.z, markings.w);
    let worn_paint = paving * paint_valid * clamp(road_grain, 0.0, 1.0);
    base = mix(base, splat.paint_yellow.xyz, center_paint * worn_paint * splat.paint_yellow.w);
    base = mix(base, splat.paint_white.xyz, edge_paint * worn_paint * splat.paint_white.w);
    // The macro break-up, then the waterline — in that order, so a wet vertex
    // keeps its own grain instead of having it multiplied back in at full dry
    // strength. `terrain_mesh::vertex_color` states why.
    base = base * in.uv_b.x;
    // Rain (weather v0): `identity[0].w` carries how wet the weather has
    // made the island, and it soaks what faces the sky — a cliff face sheds
    // it. The shoreline's own wet band is the floor under it.
    let rain_wet = splat.identity[0].w * clamp(in.world_normal.y, 0.0, 1.0);
    let wet = max(in.uv_b.y, rain_wet);
    base = wetted(base, wet);

    // The photograph, last: a scalar field with a mean of 1, so it contributes
    // relief and not colour.
    let lit = dot(bw, grain) + road_weight * road_grain;
    pbr_input.material.base_color = vec4(base * lit, 1.0);

    // **Roughness, per texel.** Until 110–113 landed this was `Σ wᵢ·roughᵢ`
    // over four authored scalars, and before those it was one shared 0.92 — so
    // granite had stone's value and not stone's SURFACE, which is the sentence
    // this whole material exists to retire. The maps close the last third of
    // it.
    //
    // **Blended by `bw`, the same weights as the colour and the normal**, and
    // deliberately not by anything cleverer. Averaging roughness across a seam
    // does lose specular variance — the Toksvig/LEAN problem — but the fix for
    // that is a variance term the sources do not carry, and using a DIFFERENT
    // weight vector here than the colour uses would put an identity's albedo
    // and its gloss in different places on the ground. One weight vector for
    // all three channels is the property worth keeping.
    //
    // **Taken whole, with no mean placed.** `ART.md` §7's mean-1 construction
    // exists to stop a photograph moving an authored COLOUR that §3 measured
    // off reference frames; §3 has no roughness row, so there is no authored
    // level for a map to move — the map is the only measurement in the room.
    // `ground_splat::ROUGH_MEAN` records what the four now measure and the gate
    // re-measures it, so a source swap changes the surface loudly.
    // Each identity's wall taps take their share here as they did the albedo's.
    let rough_map = mix(
        vec4<f32>(
            textureSample(rough_ao_maps, ground_sampler, uv0, 0).r,
            textureSample(rough_ao_maps, ground_sampler, uv1, 1).r,
            textureSample(rough_ao_maps, ground_sampler, uv2, 2).r,
            textureSample(rough_ao_maps, ground_sampler, uv3, 3).r,
        ),
        vec4<f32>(t0.rough, t1.rough, t2.rough, t3.rough),
        wm,
    );
    // Wet ground is smoother — `WET_VALUE`'s missing third. `terrain_mesh.rs`
    // states the physics and then states why it could not have it: roughness
    // "cannot vary per vertex without the shader `RENDER.md` §8 owns". This is
    // that shader. Same shape as the value keep in `wetted`, one line below the
    // one that darkens and saturates the same texel.
    //
    // **No clamp here, deliberately.** The result is provably in [0, 1]:
    // `sum(bw) + road_weight = 1`, a texture sample is in [0, 1] by format, so this is a
    // convex combination of values in range, and `wet_keep` is in
    // [`WET_ROUGH`, 1]. `apply_pbr_lighting` applies Filament's 0.089 floor
    // itself (`bevy_pbr`'s `pbr_lighting.wgsl`) — restating that number here
    // would be a hand-kept mirror of another crate's constant, which is the
    // drift `CLAUDE.md` names twice. `tests/ground_splat.rs` holds our knob
    // clear of it instead, which is the half that IS ours.
    let wet_keep = 1.0 - wet * (1.0 - splat.blend.z);
    pbr_input.material.perceptual_roughness = (dot(bw, rough_map) + road_weight * road_rough) * wet_keep;

    // The relief, blended as gradients. The planar taps' share is applied on
    // the mesh's own written tangent frame; each wall tap's is already a world
    // offset on its plane's axes, laid in the surface by removing its component
    // along the normal — so a steep face carries the photograph's relief, and
    // not a planar tap's streak of it.
    let g = to_gradient(unpack_normal(textureSample(normal_maps, ground_sampler, uv0, 0))) * (bw.x * (1.0 - wm.x))
        + to_gradient(unpack_normal(textureSample(normal_maps, ground_sampler, uv1, 1))) * (bw.y * (1.0 - wm.y))
        + to_gradient(unpack_normal(textureSample(normal_maps, ground_sampler, uv2, 2))) * (bw.z * (1.0 - wm.z))
        + to_gradient(unpack_normal(textureSample(normal_maps, ground_sampler, uv3, 3))) * (bw.w * (1.0 - wm.w));
    // Binder flattens loose aggregate relief. Blend gradients from fixed
    // projections so the verge retains the ground's original relief.
    let road_g = to_gradient(unpack_normal(road_normal));
    let road_relief = paving * splat.wall.w + dirt * splat.blend.w;
    let tbn = calculate_tbn_mikktspace(pbr_input.world_normal, in.world_tangent);
    let wall_relief = t0.relief * (bw.x * wm.x) + t1.relief * (bw.y * wm.y)
        + t2.relief * (bw.z * wm.z) + t3.relief * (bw.w * wm.w);
    pbr_input.N = normalize(tbn * vec3(g + road_g * road_relief, 1.0)
        + wall_relief - dot(wall_relief, wn) * wn);
    // The block's facet, only where the surface is rock and only across the
    // surface: the tilt's component along N is removed so a facet leans and
    // never lifts.
    let n0 = pbr_input.N;
    let lean = (rock_tilt - dot(rock_tilt, n0) * n0) * bw.w;
    pbr_input.N = normalize(n0 + lean);

    var out: FragmentOutput;
    // ── Ambient occlusion, blended by the same weights ──────────────────
    //
    // **`min`, never a multiply, and that is `ART.md` §4 in one line**: "Never
    // sum or multiply two occlusion terms of the same scale. Frostbite takes
    // `min(bakedAO, ssAO)` to avoid double-darkening." Bevy's own
    // `pbr_fragment` already applies exactly that rule between a material's
    // `occlusion_texture` and SSAO, and `pbr_input.diffuse_occlusion` arrives
    // here holding the SSAO term alone — the ground's base `StandardMaterial`
    // has no occlusion slot, because these four maps are per-identity and it
    // has one. So this is the same fold, one level up.
    //
    // **Diffuse only.** §4 again: the medium scale is `indirectDiffuse *= ao`,
    // indirect only, and "specular occlusion is a separate term, not the
    // diffuse one reused" — applying this to specular is visibly wrong at
    // grazing angles. `pbr_input.specular_occlusion` is left as Bevy computed
    // it from SSAO.
    // Layers 5–8 of the rough/AO array: `textures::AO_LAYER0` plus the
    // identity, and `tests/ground_tiling.rs` holds these literals to it.
    let ao = dot(
        bw,
        mix(
            vec4<f32>(
                textureSample(rough_ao_maps, ground_sampler, uv0, 5).r,
                textureSample(rough_ao_maps, ground_sampler, uv1, 6).r,
                textureSample(rough_ao_maps, ground_sampler, uv2, 7).r,
                textureSample(rough_ao_maps, ground_sampler, uv3, 8).r,
            ),
            vec4<f32>(t0.ao, t1.ao, t2.ao, t3.ao),
            wm,
        ),
    );
    pbr_input.diffuse_occlusion = min(pbr_input.diffuse_occlusion, vec3<f32>(ao + road_weight * road_ao));

    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
