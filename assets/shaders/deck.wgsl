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
