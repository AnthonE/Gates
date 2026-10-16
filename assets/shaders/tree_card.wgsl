// The far treeline's card (`render/far_trees.rs`): every tree past the prop
// rings as one quad, turned to the camera about the vertical here and lit by
// `StandardMaterial`'s own fragment stage, the atlas read through
// `alpha_cut` as the prepass reads it.
//
// The mesh stores each corner at the tree's base line, lifted by its share of
// the height; `uv_b.x` is the corner's sideways offset in metres and `uv_b.y`
// its sideways position in -1..1, which the normal is rounded by.
#import bevy_pbr::{
    mesh_functions,
    forward_io::{Vertex, VertexOutput, FragmentOutput},
    mesh_view_bindings::view,
    view_transformations::position_world_to_clip,
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, apply_pbr_lighting, main_pass_post_lighting_processing},
    pbr_types::{STANDARD_MATERIAL_FLAGS_BASE_COLOR_TEXTURE_BIT, STANDARD_MATERIAL_FLAGS_UNLIT_BIT},
    pbr_bindings,
}
#ifdef VISIBILITY_RANGE_DITHER
#import bevy_pbr::pbr_functions::visibility_range_dither
#endif
#import "shaders/alpha_cut.wgsl"::{cut_read, cut_uv}

// `far_trees::CardParams`. params: x = prop chunk edge (m), y = 1 on a ring
// card (its entity is the ring's own tree, so it ignores the mask). window:
// the mask's first chunk and its side; bits: one per window chunk, set where
// a prop ring draws the trees.
struct CardParams {
    params: vec4<f32>,
    window: vec4<i32>,
    bits: vec4<u32>,
}
@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> card: CardParams;

fn ring_holds(wp: vec3<f32>) -> bool {
    let local = vec2<i32>(floor(wp.xz / card.params.x)) - card.window.xy;
    let side = card.window.z;
    if local.x < 0 || local.y < 0 || local.x >= side || local.y >= side {
        return false;
    }
    let i = u32(local.y * side + local.x);
    var word = card.bits.x;
    if i >= 96u {
        word = card.bits.w;
    } else if i >= 64u {
        word = card.bits.z;
    } else if i >= 32u {
        word = card.bits.y;
    }
    return ((word >> (i % 32u)) & 1u) != 0u;
}

@vertex
fn vertex(in: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let world_from_local = mesh_functions::get_world_from_local(in.instance_index);
    var wp = mesh_functions::mesh_position_local_to_world(world_from_local, vec4(in.position, 1.0)).xyz;

    // The prop rings own this chunk: collapse the card to one point off
    // screen, so its two triangles have no area and nothing rasterises.
    let island = card.params.y < 0.5;
    var hidden = island && ring_holds(wp);

    let to_cam = view.world_position.xyz - wp;
    // Past a couple of kilometres a tree is a few pixels, and the forest's
    // canopy reads as texture: keep a thinning share of the island cards,
    // chosen by a hash of the tree's own position so a card never flickers.
    if island {
        let d = length(to_cam.xz);
        let keep = clamp((1800.0 * 1800.0) / max(d * d, 1.0), 0.3, 1.0);
        let h = fract(sin(dot(floor(wp.xz * 4.0), vec2<f32>(12.9898, 78.233))) * 43758.5453);
        if h > keep {
            hidden = true;
        }
    }
    var flat = vec2<f32>(to_cam.x, to_cam.z);
    let len = length(flat);
    if len > 1e-4 {
        flat = flat / len;
    } else {
        flat = vec2<f32>(0.0, 1.0);
    }
    let facing = vec3<f32>(flat.x, 0.0, flat.y);
    let right = vec3<f32>(flat.y, 0.0, -flat.x);
#ifdef VERTEX_UVS_B
    let side_m = in.uv_b.x;
    let s = in.uv_b.y;
#else
    let side_m = 0.0;
    let s = 0.0;
#endif
    // A ring card carries its slot's scale in its transform; a tile's is baked.
    let scale = length(world_from_local[0].xyz);
    wp = wp + right * side_m * scale;

    // A crown is round, so the card is lit as one: the normal leans out
    // toward the edge it stands on and a little up.
    let n = normalize(facing * sqrt(max(1.0 - s * s, 0.0)) * 0.8 + right * s * 0.8 + vec3<f32>(0.0, 0.45, 0.0));

    out.world_position = vec4<f32>(wp, 1.0);
    out.position = position_world_to_clip(wp);
    if hidden {
        out.position = vec4<f32>(2.0, 2.0, 2.0, 1.0);
    }
    out.world_normal = n;
#ifdef VERTEX_UVS_A
    out.uv = in.uv;
#endif
#ifdef VERTEX_UVS_B
    out.uv_b = in.uv_b;
#endif
#ifdef VERTEX_TANGENTS
    out.world_tangent = vec4<f32>(right, 1.0);
#endif
#ifdef VERTEX_COLORS
    out.color = in.color;
#endif
#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    out.instance_index = in.instance_index;
#endif
#ifdef VISIBILITY_RANGE_DITHER
    out.visibility_range_dither = mesh_functions::get_visibility_range_dither_level(
        in.instance_index, world_from_local[3]);
#endif
    return out;
}

// `bevy_pbr`'s forward `pbr.wgsl` fragment, but the atlas is read through
// `alpha_cut` with gradients taken first thing, and the ring card's LOD
// crossfade cuts after it rather than before: a card whose alpha test
// disagrees with its prepass's leaves depth with no colour on it — the clear
// colour, unfogged (`alpha_cut.wgsl` has the whole story).
//
// A hand copy of Bevy 0.18.1's, so a Bevy upgrade that changes `pbr.wgsl`
// does not reach it. Left out, as the game uses none of them:
// `apply_decals` (clustered decals) and the OIT_ENABLED, FORWARD_DECAL,
// PREPASS_PIPELINE (deferred) and meshlet branches. Adopting one, or a Bevy
// upgrade, re-checks this copy and `foliage.wgsl`'s, which left out the same.
@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
#ifdef VERTEX_UVS
    let uv = cut_uv(in.uv);
#else
    let uv = vec2<f32>(0.0);
#endif
    let duv = vec4(dpdx(uv), dpdy(uv));

    var pbr_input = pbr_input_from_standard_material(in, is_front);
#ifdef VERTEX_UVS
    if (pbr_input.material.flags & STANDARD_MATERIAL_FLAGS_BASE_COLOR_TEXTURE_BIT) != 0u {
        var c = pbr_bindings::material.base_color;
#ifdef VERTEX_COLORS
        c *= in.color;
#endif
        pbr_input.material.base_color = c * cut_read(uv, duv);
    }
#endif
#ifdef VISIBILITY_RANGE_DITHER
    visibility_range_dither(in.position, in.visibility_range_dither);
#endif
    pbr_input.material.base_color = alpha_discard(pbr_input.material, pbr_input.material.base_color);

    var out: FragmentOutput;
    if (pbr_input.material.flags & STANDARD_MATERIAL_FLAGS_UNLIT_BIT) == 0u {
        out.color = apply_pbr_lighting(pbr_input);
    } else {
        out.color = pbr_input.material.base_color;
    }
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
