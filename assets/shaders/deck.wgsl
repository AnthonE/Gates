// The cloud deck's geometry and fine detail on the GPU, shared by a
// browser's per-pixel sky (`sky_deck.wgsl`) and the moon (`moon.wgsl`), so
// a cloud hides the moon exactly where the sky draws that cloud. Texture
// reads stay in each shader: a WebGL2 build cannot pass a texture into a
// function.

// Where a ray meets the deck, noise units: `sky::TexelGeo::new`'s
// projection. `alt` is the deck's altitude over its noise scale; the ray's
// height is held at the horizon cutoff so the point stays finite under it.
fn deck_point(d: vec3<f32>, alt: f32, cutoff: f32) -> vec2<f32> {
    return d.xz * (alt / max(d.y, cutoff));
}

// How much of the sky a ray `y` above the horizon loses to the weather's
// fog, `0..=1`: `sky::SkyFog::band` short of its dust (`deck_haze` adds it).
// `air` is `SkyFog::lanes` — the ground fog and the rain haze, each straight
// up and level across the island.
fn deck_fog(y: f32, air: vec4<f32>) -> f32 {
    let s = max(y + 0.02, 1e-3);
    let tau = min(air.x / s, air.z) + min(air.y / s, air.w);
    return 1.0 - exp(-tau);
}

// `deck_fog` with the dust over arid ground in it: `sky::SkyFog::band`
// whole. `dust` is `SkyFog::dust_lanes` — x straight up through its layer,
// y level across the island. The layers' depths add, so their clear
// shares multiply.
fn deck_haze(y: f32, air: vec4<f32>, dust: vec4<f32>) -> f32 {
    let s = max(y + 0.02, 1e-3);
    return 1.0 - (1.0 - deck_fog(y, air)) * exp(-min(dust.x / s, dust.y));
}

// The raymarched slab's shape (`cloud.wgsl`), shared with the stars and
// the moon so a cloud hides them where it is drawn rather than where its
// base's outline is. A cumulus seen from the side rises far above that
// outline, and stars read off the flat deck shone through its flanks.
//
// How far a column wanders from its base by the slab's ceiling, m: a 2D
// field read straight up is an extrusion, and its grain runs up a cloud's
// side as stripes; none at the base, whose outline the sun's cover reads.
const SLAB_WARP_M: f32 = 90.0;

fn slab_warp(p: vec3<f32>, h: f32) -> vec2<f32> {
    return vec2(sin(h * 7.0 + p.z * 0.0031), sin(h * 6.0 + p.x * 0.0027)) * (SLAB_WARP_M * h);
}

// The slab's density before erosion at height `h` (`0..1` through it) in a
// column `c` edges past the field's threshold. A flat base at the
// condensation level and a dome over it: the field is near linear across a
// cloud's edge, so a height linear in it builds cones, and its square root
// builds the rounded top of a hemisphere. Steeper than the flat deck's edge:
// a cumulus has a skin.
fn slab_shape(h: f32, c: f32) -> f32 {
    if c <= 0.0 || h <= 0.0 || h >= 1.0 {
        return 0.0;
    }
    let top = 0.15 + 0.7 * sqrt(clamp(c * 0.4, 0.0, 1.0));
    let profile = smoothstep(0.0, 0.06, h) * (1.0 - smoothstep(top * 0.55, top, h));
    return smoothstep(0.0, 0.45, c) * profile;
}

// Where a ray from the eye (at height `eye_y`) is at its `i`th of `n` even
// steps through the slab `slab` (x floor m, y ceiling m), across the deck
// with the eye at its origin; w is the step's length, m. `cutoff` holds a
// low ray's height up so the run stays finite. The run is at most 14 km, as
// the clouds' own (`cloud.wgsl`'s `MAX_RUN_M`).
fn slab_step(d: vec3<f32>, eye_y: f32, slab: vec4<f32>, cutoff: f32, i: i32, n: i32) -> vec4<f32> {
    let dy = max(d.y, cutoff);
    let t0 = max((slab.x - eye_y) / dy, 0.0);
    let t1 = min((slab.y - eye_y) / dy, t0 + 14000.0);
    let dt = max(t1 - t0, 0.0) / f32(n);
    let t = t0 + (f32(i) + 0.5) * dt;
    return vec4(d.x * t, eye_y + d.y * t, d.z * t, dt);
}

// How much of the deck stands in front, by the ray's height: none under
// the cutoff, all of it `fade` above.
fn deck_fade(y: f32, cutoff: f32, fade: f32) -> f32 {
    return clamp((y - cutoff) / fade, 0.0, 1.0);
}

fn deck_hash(p: vec2<i32>) -> f32 {
    var h = (bitcast<u32>(p.x) * 0x8da6b343u) ^ (bitcast<u32>(p.y) * 0xd8163841u);
    h = (h ^ (h >> 15u)) * 0x2c1b3c6du;
    h = (h ^ (h >> 12u)) * 0x297a2d39u;
    h = h ^ (h >> 15u);
    return f32(h >> 8u) / 16777216.0;
}

fn deck_value(p: vec2<f32>) -> f32 {
    let i = vec2<i32>(floor(p));
    let f = fract(p);
    let s = f * f * (3.0 - 2.0 * f);
    let a = deck_hash(i);
    let b = deck_hash(i + vec2(1, 0));
    let c = deck_hash(i + vec2(0, 1));
    let d = deck_hash(i + vec2(1, 1));
    return mix(mix(a, b, s.x), mix(c, d, s.x), s.y);
}

// Two billowed octaves above the field's last (`sky::billow_tiled`'s
// cauliflower, continued past what the field's texels hold), centred on
// zero, at unit amplitude. The fold is rounded: a sharp one cuts thin cloud
// into a web of dark cracks. `foot` is a pixel's span on the deck, noise
// units: an octave a pixel spans fades out rather than shimmering.
fn deck_detail(q: vec2<f32>, foot: f32) -> f32 {
    var p = q * 16.0;
    var wave = 1.0 / 16.0;
    var amp = 1.0;
    var sum = 0.0;
    for (var o = 0; o < 2; o += 1) {
        let keep = clamp(wave / max(foot, 1e-6) - 1.0, 0.0, 1.0);
        let x = 2.0 * deck_value(p) - 1.0;
        sum += amp * keep * (sqrt(x * x + 0.09) - 0.55);
        // Twice the frequency and turned, so the lattices do not line up.
        p = vec2(p.x * 1.6 - p.y * 1.2, p.x * 1.2 + p.y * 1.6) + vec2(17.3, -9.1);
        wave *= 0.5;
        amp *= 0.5;
    }
    return sum;
}
