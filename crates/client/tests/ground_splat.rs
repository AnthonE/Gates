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

use client::render::ground_splat::{GRAIN_GAIN, ROUGH_MEAN, WET_ROUGH};
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
        (110, "rough_sand"),
        (111, "rough_grass"),
        (112, "rough_litter"),
        (113, "rough_rock"),
        (114, "ao_sand"),
        (115, "ao_grass"),
        (116, "ao_litter"),
        (117, "ao_rock"),
    ];

    // ── And the RUST struct, which this gate never actually read ────────
    //
    // The list above is hand-kept on purpose — it is the tripwire that makes
    // adding a texture a three-place edit. But its doc called it "what
    // `GroundSplat`'s `AsBindGroup` derive declares" while reading nothing of
    // the sort, so shader and list could agree perfectly while the struct that
    // actually builds the bind group had drifted from both. That is
    // `CLAUDE.md`'s hand-kept-mirror trap exactly, and the whole failure this
    // suite exists to prevent is a binding index nothing announces at runtime.
    //
    // Indices only, not names: binding 109 is the shared sampler, which the
    // derive hangs off the `albedo_rock` FIELD while the shader calls it
    // `ground_sampler` — the two sides legitimately disagree about that one
    // name and agreeing about the number is the thing that matters.
    let rust_src = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/render/ground_splat.rs"
    ))
    .expect("ground_splat.rs is not where this test expects it");
    let mut rust_slots: Vec<u32> = Vec::new();
    for attr in ["#[uniform(", "#[texture(", "#[sampler("] {
        for (_, rest) in rust_src.split_once(attr).into_iter().flat_map(|_| {
            rust_src
                .match_indices(attr)
                .map(|(i, _)| (i, &rust_src[i + attr.len()..]))
        }) {
            let num = rest
                .split_once(')')
                .unwrap_or_else(|| panic!("unterminated {attr} in ground_splat.rs"))
                .0;
            rust_slots.push(
                num.trim()
                    .parse()
                    .unwrap_or_else(|e| panic!("{attr}{num}) is not a number: {e}")),
            );
        }
    }
    rust_slots.sort_unstable();
    let shader_slots: Vec<u32> = {
        let mut v: Vec<u32> = found.iter().map(|(n, _)| *n).collect();
        v.sort_unstable();
        v
    };
    assert_eq!(
        rust_slots, shader_slots,
        "the WGSL and `GroundSplat`'s own attributes bind different slots. \
         A slot the struct declares and the shader does not read is a texture \
         uploaded for nobody; a slot the shader reads and the struct does not \
         declare is an unbound sample, which reads as BLACK."
    );

    let got: Vec<(u32, &str)> = found.iter().map(|(n, s)| (*n, s.as_str())).collect();
    assert_eq!(
        got, want,
        "the shader's bindings and the Rust struct's have drifted.\n\
         A wrong index is not a compile error on either side — it is a wrong \
         texture at runtime, and the frame does not announce it."
    );
}

/// Leg 2b. The sampler count stays at one, however many maps arrive.
///
/// Twelve maps with a sampler each would put this bind group at 24 samplers in
/// the fragment stage before `StandardMaterial`'s own are counted, far over the
/// 16 a downlevel adapter guarantees. That is a runtime validation failure on a
/// stricter adapter and nothing else in the tree would catch it.
///
/// **This is the axis that binds, and the roughness slice is the proof.** It
/// added four textures and zero samplers; textures are cheap here because Bevy
/// asks the adapter for its own limits, and samplers are the one with a hard
/// floor under them. A future map set must arrive the same way.
#[test]
fn every_map_shares_one_sampler() {
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

/// One map's mean RAW value — no sRGB decode, because a roughness map is data
/// and `textures::tiling(false)` loads it that way. Decoding it as colour is
/// the exact bug the `is_srgb` comment in `textures.rs` warns about, so the
/// measurement has to be made the way the GPU will see it.
fn mean_raw(role: &str, kind: &str) -> f64 {
    let path = format!(
        "{}/../../assets/textures/{role}_{kind}.jpg",
        env!("CARGO_MANIFEST_DIR")
    );
    let bytes = std::fs::read(&path)
        .unwrap_or_else(|e| panic!("{role}'s {kind} map is not on disk at {path}: {e}"));
    let img = image::load_from_memory(&bytes)
        .unwrap_or_else(|e| panic!("{role}_{kind}.jpg did not decode: {e}"))
        .to_rgb8();
    let mut sum = 0.0f64;
    for px in img.pixels() {
        sum += f64::from(px.0[0]) / 255.0;
    }
    sum / (f64::from(img.width()) * f64::from(img.height()))
}

/// Leg 5. `ROUGH_MEAN` is the shipped files' own, re-measured.
///
/// **Same argument as leg 1 and a sharper consequence.** A swapped albedo with
/// a stale gain shifts brightness, which is at least visible; a swapped
/// roughness map changes how the whole island catches the sun and nothing in
/// the tree states what it should be, because `ART.md` §3 has no roughness row.
/// This constant is the only written record of what the four surfaces measure,
/// so it has to be held to the files or it is a comment.
#[test]
fn the_roughness_means_are_the_shipped_files_own() {
    for (k, role) in ROLES.iter().enumerate() {
        let want = mean_raw(role, "rough");
        let got = f64::from(ROUGH_MEAN[k]);
        let rel = (got - want).abs() / want;
        assert!(
            rel < 0.005,
            "{role}: ROUGH_MEAN[{k}] is {got:.4} and the file measures \
             {want:.4} ({:.2}% off).\n\
             If the source was swapped deliberately, re-measure and update \
             `ROUGH_MEAN` in `render/ground_splat.rs` — and LOOK at the \
             frame, because this number is the island's specular.",
            rel * 100.0
        );
    }
}

/// Leg 5b. The four identities do not share one roughness, and every one of
/// them is a plausible dielectric.
///
/// The property the retired `IDENTITY_ROUGH` was gated on, restated against
/// the maps — a source set that measured four near-identical means would put
/// us back at the shared `perceptual_roughness: 0.92` while looking like it
/// had four maps, and only a number would say so.
#[test]
fn the_identities_do_not_share_one_roughness() {
    for (k, r) in ROUGH_MEAN.iter().enumerate() {
        assert!(
            (0.3..=1.0).contains(r),
            "identity {k}'s roughness {r} is outside the dielectric range"
        );
    }
    let lo = ROUGH_MEAN.iter().copied().fold(f32::MAX, f32::min);
    let hi = ROUGH_MEAN.iter().copied().fold(f32::MIN, f32::max);
    assert!(
        hi - lo > 0.1,
        "the four roughness means span only {:.3} ({lo:.3}..{hi:.3}) — that is \
         the shared `perceptual_roughness: 0.92` again, wearing four maps",
        hi - lo
    );
}

/// Leg 5c. **Granite is the smooth one, and it is the whole point.**
///
/// This is a claim about the WORLD, not about a file: a hard mineral face is
/// smoother than dry beach sand or needle litter, and the retired
/// `IDENTITY_ROUGH` had it exactly backwards (rock 0.88 against sand 0.86).
/// Pinning the ordering here is what makes a future source swap that quietly
/// reverses it a red gate rather than a frame nobody re-measures.
#[test]
fn granite_is_smoother_than_the_soft_ground() {
    let [sand, grass, litter, rock] = ROUGH_MEAN;
    for (name, other) in [("sand", sand), ("grass", grass), ("litter", litter)] {
        assert!(
            rock < other,
            "granite ({rock:.3}) is not smoother than {name} ({other:.3}) — \
             either the rock source was swapped for something matte or the \
             other one was swapped for something polished. A mineral face is \
             the smoothest ground on this island."
        );
    }
}

/// Leg 5d. Wet ground is smoother than dry ground, and never a mirror.
///
/// [`WET_ROUGH`] is a keep-fraction the wet factor ramps into, the same shape
/// `WET_VALUE` uses for value. Two ways it can be wrong and both are silent:
/// at 1.0 the waterline stops tightening at all and the residual
/// `terrain_mesh.rs` names is re-opened while looking closed; low enough and
/// the shore becomes a chrome band, which on the smoothest identity's own
/// beach is where it would show first.
#[test]
fn the_wet_band_tightens_without_becoming_a_mirror() {
    assert!(
        (0.4..1.0).contains(&WET_ROUGH),
        "WET_ROUGH is {WET_ROUGH} — at 1.0 or above wet ground never tightens, \
         and under 0.4 the waterline is chrome"
    );
    // The identity actually at a waterline, fully soaked.
    let wet_sand = ROUGH_MEAN[0] * WET_ROUGH;
    assert!(
        wet_sand > 0.5,
        "soaked sand lands at {wet_sand:.3} perceptual roughness — that is a \
         specular sheet, not a wet beach"
    );
    // Bevy floors `perceptual_roughness` at 0.089; the smoothest identity
    // soaked must stay clear of it, or the clamp is doing the authoring.
    let wet_rock = ROUGH_MEAN[3] * WET_ROUGH;
    assert!(
        wet_rock > 0.089,
        "soaked granite lands at {wet_rock:.3}, at or under Bevy's own \
         roughness floor — the clamp would be choosing the value"
    );
}

/// **The uniform's two sides declare the same fields in the same order.**
///
/// `AsBindGroup` writes `GroundSplatParams`'s bytes and the shader reads them
/// back through its own `struct GroundSplat`. Nothing checks that the two agree
/// — the binding gate above checks binding *numbers*, and a uniform whose two
/// sides disagree about layout still binds. Every field after the first
/// mismatch is then garbage, and the symptom is a wrong-looking ground rather
/// than anything that names a layout. Adding `wall` on 2026-08-27 is exactly
/// the edit that could have done it.
#[test]
fn the_uniform_declares_the_same_fields_on_both_sides() {
    fn fields(src: &str, header: &str, strip: &str) -> Vec<String> {
        let body = src
            .split_once(header)
            .unwrap_or_else(|| panic!("no `{header}` in the source"))
            .1;
        let body = body.split_once("\n}").expect("unterminated struct").0;
        body.lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with("//") && !l.starts_with("///"))
            .filter_map(|l| l.trim_start_matches(strip).trim().split(':').next())
            .map(|n| n.trim().to_string())
            .filter(|n| !n.is_empty())
            .collect()
    }
    let wgsl = std::fs::read_to_string(SHADER).expect("shader");
    let rust = std::fs::read_to_string("src/render/ground_splat.rs").expect("ground_splat.rs");
    let shader_fields = fields(&wgsl, "struct GroundSplat {", "");
    let rust_fields = fields(&rust, "pub struct GroundSplatParams {", "pub ");
    assert_eq!(
        shader_fields, rust_fields,
        "the ground uniform's two sides disagree about their fields. WGSL says \
         {shader_fields:?}; `GroundSplatParams` says {rust_fields:?}. Whichever \
         is right, the shader is reading bytes the CPU did not write there."
    );
    assert!(
        shader_fields.len() >= 5,
        "only {} fields scraped — the parse broke rather than the struct \
         matching, which is a gate that passes by matching nothing",
        shader_fields.len()
    );
}

/// **The biplanar wall tap samples with explicit gradients, and its
/// derivatives are taken outside every branch.**
///
/// Two ways to write this wrong, and `DECISIONS.md` materials v4 records the
/// browser client shipping the first:
///
/// - `dpdx` of the finished wall UV instead of of the world position. The
///   frame is per-fragment, so the product rule adds a term that is the frame
///   *turning* multiplied by a world coordinate of order 1500 m — an error of
///   about eighty times, which selects a mip seven levels too coarse in bands
///   following the terrain's curvature.
/// - `textureSample` inside the wall branch. Implicit-gradient sampling is
///   undefined under non-uniform control flow, and this branch is non-uniform
///   by construction.
///
/// Neither shows up as a compile error or a wrong value in any test — they show
/// up as a smear on a cliff — so the gate is a scrape, the same shape as
/// `sim-core/tests/height_roles.rs`.
#[test]
fn the_wall_tap_is_gradient_sampled_and_its_derivatives_are_not_in_a_branch() {
    let src = std::fs::read_to_string(SHADER).expect("shader");
    let frag = src
        .split_once("fn fragment(")
        .expect("no fragment entry point")
        .1;

    // Every derivative in the fragment sits at the function body's own depth.
    let mut depth = 0i32;
    let mut deriv_lines = 0;
    for line in frag.lines() {
        let code = line.split("//").next().unwrap_or("");
        if code.contains("dpdx(") || code.contains("dpdy(") {
            deriv_lines += 1;
            assert_eq!(
                depth,
                1,
                "a derivative is taken at brace depth {depth} in the fragment: \
                 `{}`. Derivatives are undefined under non-uniform control \
                 flow; hoist it to the function body.",
                line.trim()
            );
        }
        depth += code.matches('{').count() as i32 - code.matches('}').count() as i32;
    }
    assert!(
        deriv_lines >= 2,
        "found {deriv_lines} derivative lines in the fragment — the scrape \
         broke, and a gate that matches nothing passes for free"
    );

    // The wall block samples with explicit gradients, four times, and never
    // implicitly.
    let block = frag
        .split_once("if wall_mix > 0.0 {")
        .expect("no wall branch — the biplanar tap is gone")
        .1
        .split_once("\n    }")
        .expect("unterminated wall branch")
        .0;
    assert_eq!(
        block.matches("textureSampleGrad(").count(),
        4,
        "the wall branch does not take exactly four explicit-gradient samples: \
         {block}"
    );
    assert!(
        !block.contains("textureSample("),
        "the wall branch uses implicit-gradient `textureSample`, which is \
         undefined in non-uniform control flow: {block}"
    );
    assert!(
        block.contains("dot(dp_dx.xz, across)") && block.contains("dot(dp_dy.xz, across)"),
        "the wall gradients are not the world position's projected onto the \
         frame — see this test's docs for what that costs: {block}"
    );
}

/// **No ground identity may be a photograph of a wall.**
///
/// ⚠ **This is the gate `assets/textures/MANIFEST.md` measured and did not
/// ship, and the omission cost exactly what an ungated claim costs.** That file
/// scored 74 candidates on three axes to replace `cliff_side` in 2026-08-04,
/// one of them "anisotropy (>1.3 = strata)", rejected `Rock022` on it (1.58),
/// and recorded the winner at 1.05 — then wrote, correctly, "The anisotropy
/// figure is not a shipped gate". `Rock023` is tagged `cliff` and `wall` by
/// ambientCG itself and measures **0.853** here: the swap replaced a
/// stratified cliff with a stratified cliff, the doc said otherwise, and
/// nothing in CI could tell. It shipped for 23 days as the identity that
/// dresses every summit above 60 m, and the defect the operator reported was
/// "the texture has a pattern to it, like rows and lines".
///
/// The estimator is the repo's own convention (`DECISIONS.md` materials v4
/// measures a "directional-anisotropy ratio ... |dx| over |dy|"), so 1.0 is
/// isotropic and a horizontally bedded rock face is well under it. The band is
/// wide on purpose: this is a veto on a source that is *directional*, not a
/// target to tune a photograph towards.
#[test]
fn no_ground_identity_is_a_photograph_of_a_wall() {
    // ⚠ **Measured on a 256² reduction, and the first cut of this gate was
    // written at full resolution and PASSED ITS OWN MUTANT.** The strata are a
    // coarse feature and the grain over them is fine and isotropic, so at 1024²
    // the gradient sum is dominated by the grain and the direction washes out:
    // the same `Rock023` file reads **0.907 at 1024² and 0.798 at 256²**. The
    // reduction is not a shortcut, it is the measurement — "rows and lines" is
    // what a surface does at a distance, which is exactly a low-pass of it.
    const SIDE: u32 = 256;
    // Wide enough that the four shipped maps sit inside it with margin
    // (0.956–1.029 at this scale), narrow enough to refuse `Rock023` (0.798).
    const LO: f64 = 0.90;
    const HI: f64 = 1.10;
    let mut worst = (String::new(), 1.0f64);
    for name in ["sand", "grass", "litter", "rock"] {
        let full = image::open(format!("../../assets/textures/{name}_albedo.jpg"))
            .unwrap_or_else(|e| panic!("{name}_albedo.jpg: {e}"))
            .to_luma8();
        let img = image::imageops::resize(&full, SIDE, SIDE, image::imageops::FilterType::Triangle);
        let (w, h) = img.dimensions();
        let (mut dx, mut dy) = (0.0f64, 0.0f64);
        for y in 1..h - 1 {
            for x in 1..w - 1 {
                let at = |a: u32, b: u32| f64::from(img.get_pixel(a, b).0[0]);
                dx += (at(x + 1, y) - at(x - 1, y)).abs();
                dy += (at(x, y + 1) - at(x, y - 1)).abs();
            }
        }
        let a = dx / dy;
        println!("{name:>7} anisotropy |dx|/|dy| = {a:.3}");
        if (a - 1.0).abs() > (worst.1 - 1.0f64).abs() {
            worst = (name.to_string(), a);
        }
        assert!(
            (LO..=HI).contains(&a),
            "{name}_albedo.jpg has a directional anisotropy of {a:.3}, outside \
             [{LO}, {HI}] — it is a photograph with a grain in it, and the \
             ground lays it flat on a planar XZ projection so that grain \
             becomes rows across the terrain. See this test's docs."
        );
    }
    // Non-vacuity: the estimator must actually vary across the four, or it is
    // returning a constant and this passes for free.
    assert!(
        (worst.1 - 1.0).abs() > 0.005,
        "every identity measured within 0.005 of exactly 1.0 (worst: {} at \
         {:.4}) — the estimator is not reading the files",
        worst.0,
        worst.1
    );
}
