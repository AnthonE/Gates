// The ground's four identities, each with its own photograph.
//
// **What this replaces.** Until now every identity shared ONE greyscale detail
// map and ONE `perceptual_roughness`, so granite had stone's *value* and not
// stone's *surface* (`terrain_mesh.rs`'s own note, and `NOW.md` §0gm). A
// `StandardMaterial` has one base-colour slot, which is the limitation this
// file exists to remove.
//
// **All three channels are now the photograph's** — albedo relief (101–104),
// normal (105–108) and, since 2026-08-16, roughness (110–113). The roughness
// maps were the last third and they were the cheapest: they had been loaded,
// uploaded and resident since the day the set landed, and nothing sampled them.
//
// **Why the maps contribute LUMINANCE and never colour.** `ART.md` §7 bounds a
// mean-placing correction: a sourced map's colour deviation may not be
// stretched by more than ×1. Measured over the four ground sources the gain
// spans are grass 2.454, sand 2.073, litter 3.586, rock 1.054 — only rock
// clears it. So each map is reduced to its own mean-1 luminance field (span
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
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{
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
    // x = HEIGHT_INFLUENCE, y = NORMAL_Z_FLOOR, z = WET_ROUGH, w reserved.
    // Passed in rather than declared here: a knob that lives only in a shader
    // is one the knob registry cannot see, and `ci/gates.sh` refuses its
    // `DECISIONS.md` row.
    blend: vec4<f32>,
    // x = WALL_ON, y = WALL_SHARPNESS, z = UV_PER_M, w reserved.
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
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> splat: GroundSplat;
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var albedo_sand: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var albedo_grass: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(103) var albedo_litter: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(104) var albedo_rock: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(105) var normal_sand: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(106) var normal_grass: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(107) var normal_litter: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(108) var normal_rock: texture_2d<f32>;
// **One sampler for all twelve.** Every map wants the same tiling/anisotropy
// descriptor, and a sampler each would put this material at 24 samplers in the
// fragment stage on top of `StandardMaterial`'s own — far over the 16 a
// downlevel adapter guarantees. Textures are the cheap axis here and samplers
// are the one with a floor under it, which is why the roughness slice added
// four of the first and none of the second.
@group(#{MATERIAL_BIND_GROUP}) @binding(109) var ground_sampler: sampler;
// The roughness maps. Greyscale, loaded `is_srgb = false` because a roughness
// map is DATA — decoding one as sRGB would bend every value toward the dark end
// and the ground would read uniformly glossy.
@group(#{MATERIAL_BIND_GROUP}) @binding(110) var rough_sand: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(111) var rough_grass: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(112) var rough_litter: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(113) var rough_rock: texture_2d<f32>;
// Ambient occlusion, per identity — `ART.md` §4's MEDIUM scale, the one
// occlusion term a light rig cannot supply. All four shipped in every depot and
// were sampled by nothing until 2026-08-25.
@group(#{MATERIAL_BIND_GROUP}) @binding(114) var ao_sand: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(115) var ao_grass: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(116) var ao_litter: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(117) var ao_rock: texture_2d<f32>;

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

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    // Everything the standard path sets up — view vector, flags, the lot. Its
    // `base_color` is left holding the vertex `COLOR`, which here is the weight
    // vector rather than a colour; every write below is an assignment, so none
    // of it survives into the frame.
    var pbr_input = pbr_input_from_standard_material(in, is_front);

    // One projection, four densities. `in.uv` is the mesh's shared planar XZ
    // UV at the 4 m reference (`terrain_mesh::UV_PER_M`); `splat.tile` spreads
    // each identity to its own photograph's authored size. Every tap of an
    // identity's maps — albedo, roughness, normal, AO, and the wall tap below
    // — must use ITS uv and no other, or the relief stops being registered
    // with the colour it came from. Derivatives are implicit here and scale
    // with the UV, so mip selection follows for free; the wall tap takes its
    // gradients explicitly and has to scale them by hand.
    let uv0 = in.uv * splat.tile.x;
    let uv1 = in.uv * splat.tile.y;
    let uv2 = in.uv * splat.tile.z;
    let uv3 = in.uv * splat.tile.w;
    var a0 = textureSample(albedo_sand, ground_sampler, uv0);
    var a1 = textureSample(albedo_grass, ground_sampler, uv1);
    var a2 = textureSample(albedo_litter, ground_sampler, uv2);
    var a3 = textureSample(albedo_rock, ground_sampler, uv3);

    // --- Biplanar: the second tap a slope needs -----------------------------
    //
    // `in.uv` is a planar XZ projection, so on a face of tilt θ the photograph
    // is stretched by `1/cos θ` along the fall line. The second tap lives on
    // the vertical plane CONTAINING that fall line, whose stretch is `1/sin θ`
    // — the exact complement, so between the two the worst case anywhere is
    // 45° at 1.41× and a third tap would buy nothing.
    //
    // ⚠ **Derivatives are taken of the WORLD POSITION and never of the finished
    // wall UV, and they are taken here — before any branch.** Both halves are
    // load-bearing and `DECISIONS.md` materials v4 records the browser client
    // shipping the first one backwards. `gm_across` is per-fragment, so
    // `dpdx(dot(p.xz, across))` expands by the product rule to
    // `dot(dpdx(p).xz, across) + dot(p.xz, dpdx(across))` — and the second term
    // is the FRAME TURNING, multiplied by a world coordinate of order 1500 m.
    // A frame rotation of 1e-4 rad/px injects ~0.16 m/px against a true
    // footprint of ~0.002, which selects a mip about seven levels too coarse in
    // bands that follow the terrain's curvature. Quilez states the rule for the
    // axis-aligned case — take the gradients of `p` before the projection is
    // chosen — and holding a rotating frame fixed is that same rule.
    // Derivatives are also undefined under non-uniform control flow, and the
    // branch below is non-uniform by construction, which is the second reason
    // they are up here.
    let wp = in.world_position.xyz;
    let dp_dx = dpdx(wp);
    let dp_dy = dpdy(wp);

    let wn = normalize(in.world_normal);
    let horiz = vec2<f32>(wn.x, wn.z);
    let sin_tilt = length(horiz);
    let cos_tilt = abs(wn.y);
    // The contour direction — the horizontal axis ACROSS the fall line. On a
    // level face `sin_tilt` is 0, the wall tap is off, and this is never read.
    var across = vec2<f32>(1.0, 0.0);
    if sin_tilt > 1e-4 {
        across = vec2<f32>(-horiz.y, horiz.x) / sin_tilt;
    }
    // `pow(cos, k)` against `pow(sin, k)`: the two foreshortenings are exact
    // complements, so this crosses over at 45° by construction rather than by a
    // tuned threshold, and `WALL_ON` is that same angle written as `sin`.
    let w_top = pow(cos_tilt, splat.wall.y);
    let w_wall = pow(sin_tilt, splat.wall.y);
    var wall_mix = 0.0;
    if sin_tilt > splat.wall.x {
        wall_mix = w_wall / max(w_top + w_wall, 1e-6);
    }

    // Skipped whole below 45°, which is every flat metre of the island — 996 to
    // 998 land samples in 1000 on the seeds measured, so the four extra
    // fetches are paid on cliffs and nowhere else. `textureSampleGrad` is what
    // makes the branch legal: an explicit-gradient sample is defined under
    // non-uniform control flow where `textureSample` is not.
    if wall_mix > 0.0 {
        let s = splat.wall.z;
        let wall_uv = vec2<f32>(dot(wp.xz, across), wp.y) * s;
        let wall_ddx = vec2<f32>(dot(dp_dx.xz, across), dp_dx.y) * s;
        let wall_ddy = vec2<f32>(dot(dp_dy.xz, across), dp_dy.y) * s;
        // ⚠ **The gradients are scaled by the same factor as the UV.** They
        // are what picks the mip, so scaling `wall_uv` alone would leave every
        // identity whose tile is not 4 m sampling a level chosen for a density
        // it is no longer drawn at — grass one level too coarse, litter closer
        // to two. That is the same class of defect as the browser shipping
        // this tap's gradient backwards, which cost ~80× (materials v4); it is
        // silent, it is a blur rather than an error, and no gate that reads
        // values can see it. `tests/ground_tiling.rs` scrapes for it instead.
        a0 = mix(a0, textureSampleGrad(albedo_sand, ground_sampler, wall_uv * splat.tile.x, wall_ddx * splat.tile.x, wall_ddy * splat.tile.x), wall_mix);
        a1 = mix(a1, textureSampleGrad(albedo_grass, ground_sampler, wall_uv * splat.tile.y, wall_ddx * splat.tile.y, wall_ddy * splat.tile.y), wall_mix);
        a2 = mix(a2, textureSampleGrad(albedo_litter, ground_sampler, wall_uv * splat.tile.z, wall_ddx * splat.tile.z, wall_ddy * splat.tile.z), wall_mix);
        a3 = mix(a3, textureSampleGrad(albedo_rock, ground_sampler, wall_uv * splat.tile.w, wall_ddx * splat.tile.w, wall_ddy * splat.tile.w), wall_mix);
    }
    // **The relief stays the top tap's alone**, so the wall costs four fetches
    // and not twelve. `to_gradient` reads a tangent-space normal as a gradient
    // over the XZ heightfield, which is what lets the four blend as one
    // surface; a normal sampled on a VERTICAL plane describes a surface whose
    // up is world ±X or ±Z, and there is no honest reading of it as a height
    // over XZ. Roughness and AO are scalars whose stretch is invisible next to
    // the albedo's, and they stay planar for the same budget reason.

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
    // advantage. Raw luma does: rock's mean (0.269) is 2.6× litter's (0.103),
    // so it wins contested bands on brightness alone. Clamped because a gain of
    // 9.7 lets one litter texel reach a grain of ~4, and an outlier texel may
    // sharpen a seam without being allowed to move it.
    //
    // **Measured as a no-op, and kept anyway.** Swapping raw luma for this
    // moved a six-frame capture by +0.1% contrast and 0.05 luma — nothing. The
    // reason is the line above: `splat_from` is near-binary (92.2% of samples
    // over 0.8), so the contested band this arbitrates is a sliver of the
    // island. It stays because "no identity wins by being brighter" is a
    // property worth having when the classifier eventually softens, not because
    // it bought a frame anything today.
    let w = clamp(in.color, vec4(0.0), vec4(1.0));
    let relief = clamp(grain - vec4(1.0), vec4(-1.0), vec4(1.0));
    let h = w + relief * splat.blend.x;
    let peak = max(max(h.x, h.y), max(h.z, h.w)) - splat.tune.w;
    let b = max(h - vec4(peak), vec4(0.0));
    let bw = b / max(b.x + b.y + b.z + b.w, 1e-4);

    // The authored colour, per pixel rather than per vertex.
    var base = vec3(0.0);
    for (var i = 0u; i < 4u; i = i + 1u) {
        base = base + splat.identity[i].xyz * bw[i];
    }
    // The macro break-up, then the waterline — in that order, so a wet vertex
    // keeps its own grain instead of having it multiplied back in at full dry
    // strength. `terrain_mesh::vertex_color` states why.
    base = base * in.uv_b.x;
    base = wetted(base, in.uv_b.y);

    // The photograph, last: a scalar field with a mean of 1, so it contributes
    // relief and not colour.
    let lit = dot(bw, grain);
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
    let rough_map = vec4<f32>(
        textureSample(rough_sand, ground_sampler, uv0).r,
        textureSample(rough_grass, ground_sampler, uv1).r,
        textureSample(rough_litter, ground_sampler, uv2).r,
        textureSample(rough_rock, ground_sampler, uv3).r,
    );
    // Wet ground is smoother — `WET_VALUE`'s missing third. `terrain_mesh.rs`
    // states the physics and then states why it could not have it: roughness
    // "cannot vary per vertex without the shader `RENDER.md` §8 owns". This is
    // that shader. Same shape as the value keep in `wetted`, one line below the
    // one that darkens and saturates the same texel.
    //
    // **No clamp here, deliberately.** The result is provably in [0, 1]: `bw`
    // sums to 1, a texture sample is in [0, 1] by format, so the dot is a
    // convex combination of values in range, and `wet_keep` is in
    // [`WET_ROUGH`, 1]. `apply_pbr_lighting` applies Filament's 0.089 floor
    // itself (`bevy_pbr`'s `pbr_lighting.wgsl`) — restating that number here
    // would be a hand-kept mirror of another crate's constant, which is the
    // drift `CLAUDE.md` names twice. `tests/ground_splat.rs` holds our knob
    // clear of it instead, which is the half that IS ours.
    let wet_keep = 1.0 - in.uv_b.y * (1.0 - splat.blend.z);
    pbr_input.material.perceptual_roughness = dot(bw, rough_map) * wet_keep;

    // The relief, blended as gradients and applied on the mesh's own written
    // tangent frame.
    let g = to_gradient(unpack_normal(textureSample(normal_sand, ground_sampler, uv0))) * bw.x
        + to_gradient(unpack_normal(textureSample(normal_grass, ground_sampler, uv1))) * bw.y
        + to_gradient(unpack_normal(textureSample(normal_litter, ground_sampler, uv2))) * bw.z
        + to_gradient(unpack_normal(textureSample(normal_rock, ground_sampler, uv3))) * bw.w;
    let nt = normalize(vec3(g, 1.0));
    let tbn = calculate_tbn_mikktspace(pbr_input.world_normal, in.world_tangent);
    pbr_input.N = normalize(tbn * nt);

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
    let ao = dot(
        bw,
        vec4<f32>(
            textureSample(ao_sand, ground_sampler, uv0).r,
            textureSample(ao_grass, ground_sampler, uv1).r,
            textureSample(ao_litter, ground_sampler, uv2).r,
            textureSample(ao_rock, ground_sampler, uv3).r,
        ),
    );
    pbr_input.diffuse_occlusion = min(pbr_input.diffuse_occlusion, vec3<f32>(ao));

    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
