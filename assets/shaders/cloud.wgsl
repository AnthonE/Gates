// The desktop's cloud deck, per pixel (`deck.rs` `CloudMaterial`), and the
// browser's under WebGPU. Drawn at the far plane in the OPAQUE pass, where
// the composed cube used to be, so `AtmosphereNode::RenderSky` composites it
// as `inscattering + T·cloud` exactly as it did the cube (`sky.rs`'s
// header): the clouds keep the atmosphere's aerial perspective and the sky
// behind them stays the atmosphere's. What changes is that a cloud is
// shaded per pixel rather than read off a 256² face six pixels a texel.
//
// Two decks, one uniform:
//   · `march.x == 0` — the flat deck: `sky::compose_range`'s native branch
//     on the GPU, with `deck.wgsl`'s fine detail. Cheap; LOW.
//   · `march.x > 0` — a slab of cloud from `slab.x` to `slab.y` metres,
//     raymarched: the deck's coverage field gives each column its cloud and
//     its height, a tiling 3D noise erodes the edges into cauliflower, and a
//     short march toward the light shades it (Beer–Lambert, a second softer
//     scattering order, a two-lobe phase for the silver lining).
// Both write premultiplied cloud over the moon's halo, in deck units, the
// cube's contract, so every reader of the deck reads one sky. The weather's
// fog over it is `sky_fog.wgsl`'s, drawn after the atmosphere.
#import bevy_pbr::mesh_view_bindings::{view, globals}
#import bevy_pbr::utils::coords_to_viewport_uv
#import "shaders/deck.wgsl"::{deck_point, deck_fade, deck_detail, slab_warp, slab_shape}

// Field for field `deck::CloudParams`. Colours linear, deck units (1.0 is
// one `sky::CLOUD_NITS`), already graded by the hour and the weather.
struct Cloud {
    // xyz a lit top; w the field threshold (1 − cover).
    top: vec4<f32>,
    // xyz a grey base; w the coverage edge's width.
    base: vec4<f32>,
    // xyz toward the light that shapes the slab (the sun, the moon at
    // night), y lifted so a march leaves the slab; w unused.
    light: vec4<f32>,
    // xy toward the sun, horizontal; zw the flat deck's light-march step.
    sun: vec4<f32>,
    // xyz toward the moon; w the halo's cosine.
    moon: vec4<f32>,
    // xyz the halo at its centre; w the moonlit scatter's cosine.
    halo: vec4<f32>,
    // xyz the moonlit scatter at the moon; w unused.
    scatter: vec4<f32>,
    // x altitude over noise scale; y the horizon cutoff; z the fade above
    // it; w the flat deck's fine detail amplitude.
    shape: vec4<f32>,
    // xy the field's offset, noise units; z one over its period; w half a
    // texel, uv.
    drift: vec4<f32>,
    // x `CORE_SHADE`; y `RIM_GLOW`; z `SIDE_VIEW_Y`; w cd/m² per deck unit.
    shade: vec4<f32>,
    // xyz the air's optical depth straight up, per channel; w 1 where the
    // atmosphere lays its transmittance on per channel, 0 through its mean.
    hue: vec4<f32>,
    // x the slab's floor, m; y its ceiling, m; z the field's metres per
    // noise unit; w metres per tile of the detail noise.
    slab: vec4<f32>,
    // x view steps (0: the flat deck); y light steps; z extinction per
    // metre at density 1; w 1 where TAA averages the jitter away.
    march: vec4<f32>,
    // x the field's texel, m; y the detail's texel, m; zw unused.
    texel: vec4<f32>,
    // xyz how far the detail noise has moved, m: with the deck downwind,
    // and slowly up, so a cumulus boils; w unused.
    wind: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> cl: Cloud;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var field_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var field_samp: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var detail_tex: texture_3d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(4) var detail_samp: sampler;

struct VertexIn {
    @location(0) position: vec3<f32>,
}

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
}

@vertex
fn vertex(in: VertexIn) -> VertexOut {
    var out: VertexOut;
    // Depth 0 is infinitely far under reverse-Z: drawn only where nothing
    // is, and read by the atmosphere as sky.
    out.clip = vec4(in.position.xy, 0.0, 1.0);
    return out;
}

fn ray(position: vec2<f32>) -> vec3<f32> {
    let ndc = coords_to_viewport_uv(position, view.viewport) * vec2(2.0, -2.0) + vec2(-1.0, 1.0);
    let v = view.view_from_clip * vec4(ndc, 1.0, 1.0);
    return normalize((view.world_from_view * vec4(v.xyz / v.w, 0.0)).xyz);
}

const LUM: vec3<f32> = vec3(0.2126, 0.7152, 0.0722);
// `sky::HUE_MAX`.
const HUE_MAX: f32 = 3.0;
// The longest run a ray makes through the slab, m: the far deck is a
// band a few pixels tall, and past this it is the horizon fade's anyway.
const MAX_RUN_M: f32 = 14000.0;
// The light march's first step, m; each after it twice the last.
const LIGHT_FIRST_M: f32 = 40.0;

// `sky::deck_hue_for`: the hue of the atmosphere's transmittance at `y`,
// undone, so a cloud composited through it comes out grey and not tan.
fn deck_hue(y: f32) -> vec3<f32> {
    let yc = clamp(y, 0.0, 1.0);
    let deg = degrees(asin(yc));
    let m = 1.0 / (yc + 0.50572 * pow(deg + 6.07995, -1.6364));
    let t = max(exp(-cl.hue.xyz * m), vec3(1e-6));
    let least = min(t.r, min(t.g, t.b));
    let grey = min(dot(t, LUM), HUE_MAX * least);
    if cl.hue.w > 0.5 {
        return grey / t;
    }
    return vec3(grey / ((t.r + t.g + t.b) / 3.0));
}

// Henyey–Greenstein against isotropic: 1 for a sphere's average.
fn hg(g: f32, c: f32) -> f32 {
    let g2 = g * g;
    return (1.0 - g2) / pow(1.0 + g2 - 2.0 * g * c, 1.5);
}

// Two lobes, a fifth of the light thrown hard forward — the bright rim on
// a cloud between the eye and the sun — and the rest nearly even. Over its
// value side-on, so a cloud lit from the side is the `top` it is given.
fn phase(c: f32) -> f32 {
    return (0.2 * hg(0.7, c) + 0.8 * hg(-0.1, c)) / 0.836;
}

// The second scattering order's: light that has already scattered once
// has forgotten most of where it came from.
fn phase_soft(c: f32) -> f32 {
    return (0.2 * hg(0.35, c) + 0.8 * hg(-0.05, c)) / 0.9426;
}

// Interleaved gradient noise: the ray's start, a fraction of a step, so the
// steps band into grain rather than into contour lines.
fn ign(p: vec2<f32>) -> f32 {
    return fract(52.9829189 * fract(dot(p, vec2(0.06711056, 0.00583715))));
}

fn field_at(xz: vec2<f32>, lod: f32) -> f32 {
    let q = xz / cl.slab.z + cl.drift.xy;
    return textureSampleLevel(field_tex, field_samp, q * cl.drift.z + cl.drift.w, lod).r;
}

// Cloud density at `p` (world, m), `0..1`. The deck's own field decides
// where a column carries cloud and — the further past the threshold, the
// taller — how high it builds, so the slab casts the silhouette the flat
// deck, the stars and the sun's cover read off the same field. `detail`:
// 0 the bare shape, 1 eroded by the detail noise, 2 and by its finer pass.
fn density(p: vec3<f32>, lod: f32, dlod: f32, detail: i32) -> f32 {
    let h = (p.y - cl.slab.x) / (cl.slab.y - cl.slab.x);
    if h <= 0.0 || h >= 1.0 {
        return 0.0;
    }
    let c = (field_at(p.xz + slab_warp(p, h), lod + 0.5) - cl.top.w) / cl.base.w;
    let shape = slab_shape(h, c);
    if shape <= 0.0 || detail == 0 {
        return shape;
    }
    // Erode the edges into lumps, harder toward the top, where a cumulus
    // boils; the core stays whole. The finer pass is the same noise at a
    // third the size and turned, so the two lattices never line up: the
    // small cauliflower on the big.
    let w = (p + cl.wind.xyz) / cl.slab.w;
    var n = textureSampleLevel(detail_tex, detail_samp, w, dlod).r;
    if detail > 1 {
        let wf = vec3(w.z, w.x, w.y) * 3.1 + vec3(0.37, 0.11, 0.73);
        let nf = textureSampleLevel(detail_tex, detail_samp, wf, dlod + 1.63).r;
        n = n * 0.7 + nf * 0.3;
    }
    let e = (1.0 - n) * (0.55 + 0.3 * h);
    return clamp((shape - e) / (1.0 - e), 0.0, 1.0);
}

// The slab, raymarched: premultiplied cloud radiance and coverage.
//
// **Across the deck the eye is at its origin, as the flat deck's is**:
// `deck_point` projects a ray from the eye, never from the eye's place on
// the island, and the sun's cover (`SkyComposer::cover_at`), the stars and
// the moon all read the deck that way. A march from the eye's world x and
// z would put every cloud a kilometre or two from where those put it, and
// the sun would go out in a gap. Only the eye's height is its own.
fn march(d: vec3<f32>, px: vec2<f32>) -> vec4<f32> {
    let eye = vec3(0.0, view.world_position.y, 0.0);
    let dy = max(d.y, cl.shape.y);
    let t0 = max((cl.slab.x - eye.y) / dy, 0.0);
    let t1 = min((cl.slab.y - eye.y) / dy, t0 + MAX_RUN_M);
    if t1 <= t0 {
        return vec4(0.0);
    }
    // Two rates. Coarse steps cross the empty sky reading only the
    // shape; on finding cloud the march steps back and walks it at a
    // quarter of the step, eroded and lit. Inside a cloud the step is what
    // decides how much each pixel's dithered start shows, and a coarse one
    // through a crisp edge shows as a weave across the whole cloud.
    let n = i32(cl.march.x);
    let dt = (t1 - t0) / f32(n);
    let dtf = dt * 0.25;
    var seed = px;
    if cl.march.w > 0.5 {
        seed += 5.588238 * f32(globals.frame_count % 64u);
    }
    // A pixel's angle, so a far sample reads a coarser mip and the deck
    // does not shimmer as it drifts.
    let pix = 2.0 / (view.clip_from_view[1][1] * view.viewport.w);
    let l = cl.light.xyz;
    let ct = dot(d, l);
    let ph = phase(ct);
    let ph2 = phase_soft(ct);
    let sigma = cl.march.z;
    let lsteps = i32(cl.march.y);

    var trans = 1.0;
    var lum = vec3(0.0);
    var t = t0 + ign(seed) * dt;
    // Fine steps left before an empty run sends the march back to coarse.
    var fine = 0;
    for (var i = 0; i < 4 * n; i += 1) {
        if t >= t1 {
            break;
        }
        let p = eye + d * t;
        let stride = select(dt, dtf, fine > 0);
        let foot = max(pix * t, 0.5 * stride * length(d.xz));
        let lod = max(log2(foot / cl.texel.x), 0.0);
        let dlod = max(log2(foot / cl.texel.y), 0.0);
        if fine == 0 {
            if density(p, lod, dlod, 0) > 0.0 {
                t = max(t - 0.75 * dt, t0);
                fine = 6;
            } else {
                t += dt;
            }
            continue;
        }
        let rho = density(p, lod, dlod, 2);
        if rho > 0.002 {
            fine = 6;
            // Toward the light: how much cloud it came through to get here.
            var od = 0.0;
            var s = LIGHT_FIRST_M;
            var along = 0.0;
            for (var k = 0; k < lsteps; k += 1) {
                let q = p + l * (along + 0.5 * s);
                od += density(q, lod + f32(k), dlod + f32(k), select(0, 1, k < 2)) * s;
                along += s;
                s *= 2.0;
            }
            let tau = od * sigma;
            let sun = 0.8 * (ph * exp(-tau) + 0.35 * ph2 * exp(-0.25 * tau));
            // Sky light from above: a base sees less of the sky than a top.
            let hgt = clamp((p.y - cl.slab.x) / (cl.slab.y - cl.slab.x), 0.0, 1.0);
            let amb = mix(0.5, 1.0, hgt);
            let lit = cl.top.rgb * sun + cl.base.rgb * amb;
            let a = 1.0 - exp(-rho * sigma * dtf);
            lum += trans * a * lit;
            trans *= 1.0 - a;
            if trans < 0.02 {
                lum += trans * lit;
                trans = 0.0;
                break;
            }
        } else {
            fine -= 1;
        }
        t += dtf;
    }
    return vec4(lum, 1.0 - trans);
}

@fragment
fn fragment(in: VertexOut) -> @location(0) vec4<f32> {
    let d = ray(in.clip.xy);

    // The flat deck's samples, before any branch: they need derivatives.
    let q = deck_point(d, cl.shape.x, cl.shape.y) + cl.drift.xy;
    let qs = q + cl.sun.zw;
    let uv = q * cl.drift.z + cl.drift.w;
    let gx = dpdx(uv);
    let gy = dpdy(uv);
    let foot = max(length(dpdx(q)), length(dpdy(q)));
    let f0 = textureSampleGrad(field_tex, field_samp, uv, gx, gy).r;
    let fs0 = textureSampleGrad(field_tex, field_samp, qs * cl.drift.z + cl.drift.w, gx, gy).r;

    // Behind the cloud: nothing (the atmosphere paints the sky) but the
    // moon's halo, which it does not draw. The disk is `moon.wgsl`'s, added
    // over this with its phase: a round one here would show through a
    // crescent's dark side.
    var sky = vec3(0.0);
    let cm = dot(d, cl.moon.xyz);
    if d.y > 0.0 && cm > cl.moon.w {
        let k = (cm - cl.moon.w) / (1.0 - cl.moon.w);
        sky += cl.halo.rgb * k * k;
    }
    let s = max((cm - cl.halo.w) / (1.0 - cl.halo.w), 0.0);

    let fade = deck_fade(d.y, cl.shape.y, cl.shape.z);
    var pre = vec3(0.0);
    var cov = 0.0;
    if fade > 0.0 {
        if cl.march.x < 0.5 {
            let thresh = cl.top.w;
            let edge = cl.base.w;
            let f = f0 + cl.shape.w * deck_detail(q, foot);
            let f_sun = fs0 + cl.shape.w * deck_detail(qs, foot);
            let c0 = clamp((f - thresh) / edge, 0.0, 1.0);
            let lit = clamp((f - f_sun) * 6.0 + 0.5, 0.0, 1.0);
            let thick = clamp((f - thresh) / (3.0 * edge), 0.0, 1.0);
            let side = clamp((cl.shade.z - d.y) / (cl.shade.z - cl.shape.y), 0.0, 1.0);
            let under = cl.base.rgb * (1.0 - cl.shade.x * thick)
                + (cl.top.rgb - cl.base.rgb) * cl.shade.y * (1.0 - c0);
            let w = max(lit * (1.0 - 0.5 * thick), side * side * (3.0 - 2.0 * side));
            let cloud = mix(under, cl.top.rgb, w) + cl.scatter.rgb * s * s * (1.0 - 0.6 * thick);
            cov = c0 * fade;
            pre = cloud * cov;
        } else {
            let m = march(d, in.clip.xy);
            cov = m.a * fade;
            pre = (m.rgb + cl.scatter.rgb * s * s * (1.0 - 0.6 * m.a) * m.a) * fade;
        }
        pre *= deck_hue(d.y);
    }
    // The weather's fog is not here: it lies between the eye and the
    // atmosphere too, and `sky_fog.wgsl` lays it over both.
    let rgb = pre + sky * (1.0 - cov);
    return vec4(rgb * cl.shade.w * view.exposure, 1.0);
}
