//! Gate: the ground's splat material says the same thing on both sides of the
//! GPU boundary.
//!
//! Four identities now carry four photographs (`render/ground_splat.rs` +
//! `assets/shaders/ground_splat.wgsl`), which puts arithmetic in a place no
//! Rust test can execute. So this gate checks the three things that CAN be
//! checked without a GPU, and each one is a real failure that has a way of
//! going unnoticed:
//!
//! 1. **The gains are the shipped files' own.** They are constants in Rust and
//!    they describe four `.jpg`s; swap a source and the constant is silently
//!    wrong. Re-measured off the files here.
//! 2. **The shader's bindings and the Rust struct's agree.** A mismatched
//!    binding index is not a compile error on either side — it is a wrong
//!    texture at runtime, and the frame does not announce it. Derived by
//!    scraping the WGSL rather than hand-kept, which is `CLAUDE.md`'s rule
//!    about mirrors: read the surface, and let a shape the scrape cannot
//!    classify fail loudly rather than skip.
//! 3. **The CPU reference still composes to the old colour.** The shader's
//!    colour path is `Σ wᵢ·albedoᵢ → × break-up → wetted`, which is exactly
//!    `terrain_mesh::vertex_color`'s body. Holding them equal is what makes
//!    `vertex_color` a usable reference for what the shader does.
//!
//! **There is no pixel gate here and there must not be one** (`CLAUDE.md`: a
//! pixel statistic cannot see whether the frame is a picture of anything, and
//! ours proved it on a beige smear). What is gated about a frame is arithmetic.
//!
//! Headless — no GPU, no window, no shard.

#![cfg(feature = "render")]

use client::render::ground_splat::{GRAIN_GAIN, IDENTITY_ROUGH};
use client::render::terrain_mesh::{self, GROUND_ALBEDO};

const SHADER: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../assets/shaders/ground_splat.wgsl"
);

const ROLES: [&str; 4] = ["sand", "grass", "litter", "rock"];

/// One map's mean LINEAR luminance, Rec.709, decoded from sRGB — the same
/// construction `ground_where_the_green_goes.rs` uses on the detail map, and
/// the same one the shader performs per-texel on a value the GPU has already
/// linearised.
fn mean_linear_luma(role: &str) -> f64 {
    let path = format!(
        "{}/../../assets/textures/{role}_albedo.jpg",
        env!("CARGO_MANIFEST_DIR")
    );
    let bytes = std::fs::read(&path).unwrap_or_else(|e| {
        panic!(
            "{role}'s albedo is not on disk at {path}: {e}\n\
             It ships — `assets/textures/MANIFEST.md` carries its row."
        )
    });
    let img = image::load_from_memory(&bytes)
        .unwrap_or_else(|e| panic!("{role}_albedo.jpg did not decode: {e}"))
        .to_rgb8();

    let mut sum = [0.0f64; 3];
    for px in img.pixels() {
        for (ch, s) in sum.iter_mut().enumerate() {
            let u = f64::from(px.0[ch]) / 255.0;
            *s += if u <= 0.040_45 {
                u / 12.92
            } else {
                ((u + 0.055) / 1.055).powf(2.4)
            };
        }
    }
    let n = f64::from(img.width()) * f64::from(img.height());
    let m = [sum[0] / n, sum[1] / n, sum[2] / n];
    0.2126 * m[0] + 0.7152 * m[1] + 0.0722 * m[2]
}

/// Leg 1. Every gain is `1 / mean linear luma` of the file it names.
///
/// **Why this is worth a gate rather than a comment.** `MANIFEST.md` says
/// "when a better source is found, drop it in with the same name" — a file
/// swap is the *designed* way to change art here. The gain is what makes a map
/// deliver a mean of 1 so it multiplies the authored colour without moving it;
/// a swapped file with a stale gain shifts the whole island's brightness, which
/// is the coupled lighting owner's business and not this material's
/// (`CLAUDE.md` traps).
#[test]
fn the_grain_gains_are_the_shipped_files_own() {
    for (k, role) in ROLES.iter().enumerate() {
        let mean = mean_linear_luma(role);
        let want = 1.0 / mean;
        let got = f64::from(GRAIN_GAIN[k]);
        let rel = (got - want).abs() / want;
        assert!(
            rel < 0.005,
            "{role}: GRAIN_GAIN[{k}] is {got:.4} and the file measures \
             {want:.4} (mean linear luma {mean:.5}, {:.2}% off).\n\
             If the source was swapped deliberately, re-measure and update \
             `GRAIN_GAIN` in `render/ground_splat.rs`.",
            rel * 100.0
        );
    }
}

/// Leg 1b. The gain does what it is for: mean × gain = 1.
///
/// Stated separately from the equality above because it is the *property* that
/// matters — a luminance field with a mean of 1 has gain span 1.000 by
/// construction, which is how all four sources clear `ART.md` §7's ×1
/// deviation rule where only `rock` clears it as colour.
#[test]
fn every_map_delivers_a_mean_of_one() {
    for (k, role) in ROLES.iter().enumerate() {
        let delivered = mean_linear_luma(role) * f64::from(GRAIN_GAIN[k]);
        assert!(
            (delivered - 1.0).abs() < 0.005,
            "{role}: delivers a mean of {delivered:.4}, not 1 — it would \
             move the island's brightness rather than only its relief"
        );
    }
}

/// Leg 2. The shader binds exactly what the Rust side declares, at the same
/// indices.
///
/// **Scraped, not hand-kept.** `CLAUDE.md` has this exact lesson twice — the
/// `props.js` citation count and the destructive-ring verb list both drifted
/// because a doc mirrored another file's surface by hand. So the expected set
/// is read out of the WGSL, and anything the scrape cannot classify is a loud
/// failure rather than a skip.
#[test]
fn the_shader_and_the_rust_side_bind_the_same_slots() {
    let src = std::fs::read_to_string(SHADER)
        .unwrap_or_else(|e| panic!("the ground splat shader is not at {SHADER}: {e}"));

    let mut found: Vec<(u32, String)> = Vec::new();
    for line in src.lines() {
        let Some(rest) = line.split_once("@binding(") else {
            continue;
        };
        let (num, tail) = rest
            .1
            .split_once(')')
            .unwrap_or_else(|| panic!("unterminated @binding( in: {line}"));
        let n: u32 = num
            .trim()
            .parse()
            .unwrap_or_else(|e| panic!("@binding({num}) is not a number ({e}) in: {line}"));
        // `var<uniform> splat: X` / `var name: texture_2d<f32>` / `var s: sampler`
        let name = tail
            .split_once("var")
            .and_then(|(_, v)| v.split(':').next())
            .map(|v| v.trim_start_matches(['<', '>']).trim())
            .map(|v| v.rsplit('>').next().unwrap_or(v).trim().to_string())
            .unwrap_or_else(|| panic!("could not read a var name out of: {line}"));
        found.push((n, name));
    }
    found.sort_unstable();

    // What `GroundSplat`'s `AsBindGroup` derive declares, in the order the
    // struct declares it. This list is the thing under test — if you add a
    // texture to the struct, this fails until it is added here AND to the
    // shader, which is the point.
    let want: Vec<(u32, &str)> = vec![
        (100, "splat"),
        (101, "albedo_sand"),
        (102, "albedo_grass"),
        (103, "albedo_litter"),
        (104, "albedo_rock"),
        (105, "normal_sand"),
        (106, "normal_grass"),
        (107, "normal_litter"),
        (108, "normal_rock"),
        (109, "ground_sampler"),
    ];

    let got: Vec<(u32, &str)> = found.iter().map(|(n, s)| (*n, s.as_str())).collect();
    assert_eq!(
        got, want,
        "the shader's bindings and the Rust struct's have drifted.\n\
         A wrong index is not a compile error on either side — it is a wrong \
         texture at runtime, and the frame does not announce it."
    );
}

/// Leg 2b. The sampler count stays at one.
///
/// Eight maps with a sampler each would put this bind group at 16 samplers in
/// the fragment stage before `StandardMaterial`'s own are counted, over the 16
/// the downlevel limit guarantees. That is a runtime validation failure on a
/// stricter adapter and nothing else in the tree would catch it.
#[test]
fn the_eight_maps_share_one_sampler() {
    let src = std::fs::read_to_string(SHADER).expect("shader");
    let n = src.matches(": sampler;").count();
    assert_eq!(
        n, 1,
        "{n} samplers declared in ground_splat.wgsl — it must stay 1"
    );
}

/// Leg 3. The split reference still composes to the colour it replaced.
///
/// `vertex_color` is no longer what the mesh carries, but it is still the
/// arithmetic the shader performs, and it is the only executable statement of
/// it we have. Holding the recomposition equal over a sweep is what lets a
/// reader trust it as the reference.
#[test]
fn the_weights_and_the_modifiers_recompose_to_the_old_colour() {
    // A sweep that reaches both modifiers: heights across and below the
    // waterline, gradients from flat to steep, and every identity pure plus a
    // four-way mix.
    let sets: [[u8; 4]; 6] = [
        [255, 0, 0, 0],
        [0, 255, 0, 0],
        [0, 0, 255, 0],
        [0, 0, 0, 255],
        [64, 64, 64, 63],
        [10, 200, 40, 5],
    ];
    let mut checked = 0usize;
    for w in sets {
        for y in [-3.0f32, 0.0, 0.4, 1.2, 4.0, 30.0] {
            for grad in [0.0f32, 0.05, 0.4, 2.0] {
                for (x, z) in [(0.0f32, 0.0f32), (12.5, -7.25), (1024.0, 1024.0)] {
                    let want = terrain_mesh::vertex_color(y, w, x, z, grad);

                    // The shader's path, in Rust: weights and modifiers in,
                    // colour out, with the photograph's mean-1 field at its
                    // mean of exactly 1 (the grain is what the GPU adds).
                    let s = terrain_mesh::vertex_splat(w);
                    let m = terrain_mesh::vertex_mods(y, x, z, grad);
                    let mut c = [0.0f32; 3];
                    for (k, f) in s.iter().enumerate() {
                        for ch in 0..3 {
                            c[ch] += GROUND_ALBEDO[k][ch] * f;
                        }
                    }
                    let got = terrain_mesh::wetted([c[0] * m[0], c[1] * m[0], c[2] * m[0]], m[1]);

                    for ch in 0..3 {
                        assert!(
                            (got[ch] - want[ch]).abs() < 1e-6,
                            "w={w:?} y={y} grad={grad} at ({x}, {z}) channel {ch}: \
                             recomposed {} vs vertex_color {}",
                            got[ch],
                            want[ch]
                        );
                    }
                    checked += 1;
                }
            }
        }
    }
    assert_eq!(checked, 6 * 6 * 4 * 3, "the sweep did not run");
}

/// Leg 4. **The weights interpolate; a packed pair does not.** This is a
/// regression guard on a design decision, and it exists because the packed
/// form was scouted in `NOW.md` §0gm and looks cheaper.
///
/// Packing two `u8` into one `f32` and letting the rasterizer interpolate is
/// exact at both ends of an edge and wrong everywhere between: the interpolated
/// value is `256·lerp(hi) + lerp(lo)`, and `floor(p / 256)` then carries
/// `lerp(lo)`'s magnitude into the high byte. Measured at the midpoint of an
/// edge between two pure identities the low weight lands 128/255 out — half the
/// range — and it does it precisely at identity boundaries, which is where a
/// splat is looked at.
///
/// `ATTRIBUTE_COLOR` carries four independent floats instead, which the
/// rasterizer interpolates componentwise and correctly.
#[test]
fn a_packed_weight_pair_would_not_survive_interpolation() {
    let pack = |hi: f32, lo: f32| hi * 256.0 + lo;
    let unpack = |p: f32| {
        let hi = (p / 256.0).floor();
        (hi, p - hi * 256.0)
    };

    // Two vertices, each a pure identity — the ordinary case at a seam.
    let a = pack(255.0, 0.0);
    let b = pack(0.0, 255.0);

    let mut worst = 0.0f32;
    for i in 0..=10 {
        let t = i as f32 / 10.0;
        let (hi, lo) = unpack(a + (b - a) * t);
        let (want_hi, want_lo) = (255.0 * (1.0 - t), 255.0 * t);
        worst = worst.max((hi - want_hi).abs().max((lo - want_lo).abs()));
    }
    assert!(
        worst > 100.0,
        "the packed form now round-trips through interpolation (worst error \
         {worst:.1}/255) — if that is genuinely true the comment in \
         `ground_splat.wgsl` needs rewriting, but check the arithmetic first"
    );

    // And the shipped form: componentwise interpolation of four weights is
    // exact, because there is nothing packed to come apart.
    for i in 0..=10 {
        let t = i as f32 / 10.0;
        let wa = terrain_mesh::vertex_splat([255, 0, 0, 0]);
        let wb = terrain_mesh::vertex_splat([0, 255, 0, 0]);
        let mixed: Vec<f32> = wa
            .iter()
            .zip(wb.iter())
            .map(|(x, y)| x + (y - x) * t)
            .collect();
        let sum: f32 = mixed.iter().sum();
        assert!(
            (sum - 1.0).abs() < 1e-6,
            "weights stopped summing to 1 at t={t}: {mixed:?}"
        );
    }
}

/// Every identity's roughness is a plausible dielectric, and they are not all
/// the same number — which was the defect this replaced.
#[test]
fn the_identities_have_their_own_roughness() {
    for (k, r) in IDENTITY_ROUGH.iter().enumerate() {
        assert!(
            (0.3..=1.0).contains(r),
            "identity {k}'s roughness {r} is outside the dielectric range"
        );
    }
    let first = IDENTITY_ROUGH[0];
    assert!(
        IDENTITY_ROUGH.iter().any(|r| *r != first),
        "all four identities share one roughness again — that is the \
         `perceptual_roughness: 0.92` this material replaced"
    );
}
