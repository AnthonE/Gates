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

    let uv = in.uv;
    let a0 = textureSample(albedo_sand, ground_sampler, uv);
    let a1 = textureSample(albedo_grass, ground_sampler, uv);
    let a2 = textureSample(albedo_litter, ground_sampler, uv);
    let a3 = textureSample(albedo_rock, ground_sampler, uv);

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
        textureSample(rough_sand, ground_sampler, uv).r,
        textureSample(rough_grass, ground_sampler, uv).r,
        textureSample(rough_litter, ground_sampler, uv).r,
        textureSample(rough_rock, ground_sampler, uv).r,
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
    let g = to_gradient(unpack_normal(textureSample(normal_sand, ground_sampler, uv))) * bw.x
        + to_gradient(unpack_normal(textureSample(normal_grass, ground_sampler, uv))) * bw.y
        + to_gradient(unpack_normal(textureSample(normal_litter, ground_sampler, uv))) * bw.z
        + to_gradient(unpack_normal(textureSample(normal_rock, ground_sampler, uv))) * bw.w;
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
            textureSample(ao_sand, ground_sampler, uv).r,
            textureSample(ao_grass, ground_sampler, uv).r,
            textureSample(ao_litter, ground_sampler, uv).r,
            textureSample(ao_rock, ground_sampler, uv).r,
        ),
    );
    pbr_input.diffuse_occlusion = min(pbr_input.diffuse_occlusion, vec3<f32>(ao));

    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
