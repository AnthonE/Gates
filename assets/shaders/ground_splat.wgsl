// The ground's four identities, each with its own relief.
//
// The first WGSL in this tree. It exists because `StandardMaterial` has one
// base-colour slot and `terrain::splat` resolves four identities, so until now
// sand, turf, forest litter and granite all shared ONE greyscale detail map:
// granite had stone's value and not stone's surface (`NOW.md` §0gp).
//
// ── What this shader may and may not do ───────────────────────────────────
//
// It computes a base colour and nothing else. `pbr_input_from_standard_material`
// builds the whole PBR input, this replaces one field of it, and Bevy's own
// `apply_pbr_lighting` and `main_pass_post_lighting_processing` do the rest —
// so lighting, shadows, fog and the tonemap are untouched. That is deliberate
// twice over: it is `CLAUDE.md`'s "Bevy draws, it does not decide" at the
// render layer, and it keeps this shader OUT of the coupled set (tonemap, sky,
// exposure, fog are one owner, and splitting them across passes has measurably
// made things worse). A splat material that also nudged exposure would be a
// second brightness owner with no gate to notice.
//
// ── Luminance, never chroma ───────────────────────────────────────────────
//
// Each identity contributes its source's LUMINANCE divided by that source's
// own mean, and the colour stays the vertex's. `ART.md` §7: a modifier that
// must set a colour multiplies a mean-1 luminance field, not its chroma — and
// a per-channel gain placing a source's mean may not stretch that source's
// colour deviation by more than x1, which three of these four sources fail
// (grass 2.454, sand 2.073, litter 3.586; only rock's 1.054 clears it). A
// luminance field has span 1.000 by construction. So the photograph
// contributes exactly what a vertex hash cannot — measured high-frequency
// relief, four different reliefs where there was one — and every colour
// decision `tests/ground_identity.rs` gates stays where it was.
//
// ── The approximation, stated ─────────────────────────────────────────────
//
// The vertex colour already carries the weighted identity mix `Σ wᵢ Cᵢ`, and
// this shader multiplies it by one scalar — the height-blended relief `B`. So
// the delivered colour is `(Σ wᵢ Cᵢ) · B(w, raw, field)`, where the exact form
// would be `Σ wᵢ Cᵢ Lᵢ`: every identity's colour carrying its OWN relief
// rather than all four sharing the blend's.
//
// The two are IDENTICAL wherever one weight is 1 — B collapses to that
// identity's own field — and the probe behind §0gp measured max weight
// p50 = 1.000 with 92.2% of land samples above 0.8, so they agree on the
// overwhelming majority of the island. Where they differ, the error is a
// cross-term whose factors are mean-1 fields, so it has zero mean over the
// texture: it reads as slightly reduced grain contrast in a transition band,
// never as a colour shift.
//
// Taking the exact form would mean moving the authored colour out of
// `ATTRIBUTE_COLOR` and into a uniform, which retires the three gates that
// read it (`ground_identity.rs`, `ground_mix.rs`,
// `ground_where_the_green_goes.rs`). Not worth a correction with zero mean on
// under 8% of the ground.

#import bevy_pbr::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, apply_pbr_lighting, main_pass_post_lighting_processing},
}

// The Rust twin is `GroundSplatParams` in `render/terrain_mesh.rs`.
//
// `gain` is each identity's `1 / mean linear luma`, in `terrain::splat`'s
// order — sand · grass · forest litter · rock — measured off the shipped files
// and gated by `tests/ground_maps.rs`.
//
// `blend_depth` is `GROUND_BLEND_DEPTH`. It is declared in Rust rather than as
// a `const` here because `ci/knob_registry.mjs` resolves `DECISIONS.md`
// declarations against `.rs`/`.js`/`.mjs` and cannot read a `.wgsl` — a knob
// that lived only in this file would be invisible to the gate that exists to
// catch invented numbers.
struct GroundSplatParams {
    gain: vec4<f32>,
    blend_depth: f32,
}
@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> ground: GroundSplatParams;

@group(#{MATERIAL_BIND_GROUP}) @binding(101) var sand_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var ground_samp: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(103) var grass_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(104) var litter_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(105) var rock_tex: texture_2d<f32>;

// `ATTRIBUTE_UV_1` carries the four splat weights packed two `u8` to an `f32`.
//
// **This is the Rust `unpack_splat` in `render/terrain_mesh.rs`, line for
// line, and it has to stay that way** — a divergence between the two does not
// crash and fails no gate, it draws the wrong identity or a seam between two
// chunks. `tests/ground_splat.rs` §D pins the radix on both sides so a change
// to one is a change to a value the other reads.
//
// The decode is exact rather than nearly so: every value written is an integer
// below 65,536, an f32 holds every integer below 2^24, and the divisor is a
// power of two so the division only shifts the exponent — `floor` cannot land
// a ulp on the wrong side of an integer.
fn unpack_splat(uv_b: vec2<f32>) -> vec4<f32> {
    let hi = floor(uv_b / 256.0);
    let lo = uv_b - hi * 256.0;
    return vec4<f32>(hi.x, lo.x, hi.y, lo.y) / 255.0;
}

// One identity's RAW linear luminance, in [0, 1]. Rec.709, matching every
// other luma in this client (`fill.rs`, `props.rs`, `terrain_mesh.rs`,
// `tree.rs`).
//
// The sample is already linear: the four albedos load with `is_srgb = true`,
// so the hardware has undone the transfer function before this runs — which is
// the same reason the gains were measured in linear.
//
// **Raw here, gained at the call site, and the split is load-bearing.** This
// value plays two roles that need two ranges. As the thing being BLENDED it
// wants the mean-1 field (raw x gain), so each identity delivers the authored
// colour on average. As the HEIGHT deciding which identity wins a texel it
// must be commensurate with the splat weights, which are [0, 1] — and the
// gained field is not: the gains are 3.7 to 9.7, so a bright litter texel
// reaches h ~ 3.9 while no weight can exceed 1.
//
// That was this shader's first form and it was wrong. `hw = h + w` with h in
// [0, 4] and w in [0, 1] is `hw ~ h`: the winner is whichever PHOTOGRAPH is
// brightest at that texel, and the classifier the whole slice exists to
// express barely participates. It is invisible in a frame — all four fields
// are mean-1, so the result still looks like ground — which is exactly why it
// is written down here.
fn luma(tex: texture_2d<f32>, uv: vec2<f32>) -> f32 {
    let c = textureSample(tex, ground_samp, uv).rgb;
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);

    let w = unpack_splat(in.uv_b);

    // Raw, in [0, 1] — the HEIGHT, commensurate with the weights.
    let raw = vec4<f32>(
        luma(sand_tex, in.uv),
        luma(grass_tex, in.uv),
        luma(litter_tex, in.uv),
        luma(rock_tex, in.uv),
    );
    // Mean-1 — the VALUE, so each identity delivers its authored colour on
    // average. Same four samples, second role.
    let field = raw * ground.gain;

    // Height blend (Mishkinis). The winner is whichever identity stands
    // highest once its splat weight is added to its own relief; anything
    // within `blend_depth` of it still contributes, everything below is cut.
    // Normalising by the surviving weight is what keeps the result mean-1 —
    // without it a texel where only one identity survives would be dimmer than
    // one where two do.
    //
    // Both terms are [0, 1] here, which is the formulation's own assumption:
    // the weight decides WHICH identity and the height decides where the seam
    // between two comparable ones falls. A bright identity can still take a
    // texel from a darker neighbour at a boundary — that is the effect, not a
    // defect — but it can no longer take one where its weight is zero.
    //
    // ⚠ Albedo luminance is a PROXY for height, not a height. The honest
    // signal is a displacement map, deliberately not sourced
    // (`assets/textures/MANIFEST.md`), or the `*_ao.jpg` set, which ships and
    // is read by nothing. Either would be a better height than "how bright is
    // this texel".
    let hw = raw + w;
    let cut = max(max(hw.x, hw.y), max(hw.z, hw.w)) - ground.blend_depth;
    let k = max(hw - vec4<f32>(cut), vec4<f32>(0.0));
    // `sum` cannot be zero: the cut is `max(hw) - blend_depth`, so the winning
    // component is always exactly `blend_depth` above it. The guard is for a
    // NaN arriving from a corrupt texture, not for the arithmetic.
    let sum = k.x + k.y + k.z + k.w;
    let relief = select(1.0, dot(k, field) / sum, sum > 0.0);

    pbr_input.material.base_color = vec4<f32>(
        pbr_input.material.base_color.rgb * relief,
        pbr_input.material.base_color.a,
    );

    pbr_input.material.base_color = alpha_discard(pbr_input.material, pbr_input.material.base_color);

    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
