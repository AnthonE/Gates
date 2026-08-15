//! The four ground map sets, measured off the files that ship.
//!
//! `GROUND_MAP_GAIN` claims to be each identity's `1 / mean linear luma`, and
//! the splat material multiplies by it on every ground pixel of the island. A
//! claim about a file that nothing opens is the shape `CLAUDE.md`'s ⚠ names —
//! and the failure mode here is invisible in a diff twice over, because the
//! evidence is a binary and the symptom is a brightness shift on one identity
//! rather than an error.
//!
//! This is `ground_where_the_green_goes.rs` §Leg 2's construction applied to
//! the four sources instead of the one derived field.

use client::render::textures::GROUND_MAP_GAIN;

/// The identities in `terrain::splat`'s order, with the file stem each one
/// samples. The ORDER is load-bearing: `GROUND_MAP_GAIN[i]` is applied to the
/// map bound at slot `i`, so a permutation here is a permutation of which
/// photograph gets which correction.
const ROLES: [&str; 4] = ["sand", "grass", "litter", "rock"];

fn srgb_to_linear(u8v: u8) -> f32 {
    let u = f32::from(u8v) / 255.0;
    if u <= 0.040_45 {
        u / 12.92
    } else {
        ((u + 0.055) / 1.055).powf(2.4)
    }
}

/// The mean linear Rec.709 luma of one shipped albedo.
fn mean_luma(role: &str) -> f32 {
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

    let mut sum = 0.0f64;
    let n = u64::from(img.width()) * u64::from(img.height());
    for px in img.pixels() {
        let l = 0.2126 * srgb_to_linear(px.0[0])
            + 0.7152 * srgb_to_linear(px.0[1])
            + 0.0722 * srgb_to_linear(px.0[2]);
        sum += f64::from(l);
    }
    (sum / n as f64) as f32
}

/// Every gain is the reciprocal of its own source's measured mean.
///
/// Tolerance is 1%, matching `ground_where_the_green_goes.rs`'s gate on
/// `GROUND_DETAIL_GAIN` — tight enough that a re-encode at a different quality
/// is caught, loose enough that a decoder revision is not a false red.
#[test]
fn every_ground_gain_places_its_own_source_mean() {
    let mut report = String::new();
    let mut bad = Vec::new();

    for (i, role) in ROLES.iter().enumerate() {
        let mean = mean_luma(role);
        let want = 1.0 / mean;
        let have = GROUND_MAP_GAIN[i];
        let drift = (have / want - 1.0).abs();
        report.push_str(&format!(
            "  [{i}] {role:<7} mean {mean:.5}  ->  gain {want:.4}  (constant {have:.4}, {:.1}% off)\n",
            drift * 100.0
        ));
        if drift > 0.01 {
            bad.push(*role);
        }
    }

    assert!(
        bad.is_empty(),
        "GROUND_MAP_GAIN disagrees with the shipped files for: {bad:?}\n\
         Measured off `assets/textures/<role>_albedo.jpg`:\n{report}\n\
         Each gain must be `1 / mean linear luma` of its own source, so the \
         delivered mean is the authored `GROUND_ALBEDO` colour and the map \
         contributes only its relief (`ART.md` §7). If a map was re-encoded, \
         paste the measured column above into `render/textures.rs`."
    );
}

/// The four gains must be DISTINCT, and by a margin.
///
/// Two identities sharing a gain is not itself wrong — but these four sources
/// were selected by `MANIFEST.md` for different scores, so equal gains means
/// two slots are pointed at the same file. That is the same defect class §0gp
/// found in `GROUND_ALBEDO` (two of four identities were one paint), one layer
/// down, and it would be invisible for the same reason: the frame still
/// renders, and every other gate still passes.
#[test]
fn no_two_identities_share_a_gain() {
    for i in 0..4 {
        for j in (i + 1)..4 {
            let ratio = GROUND_MAP_GAIN[i].max(GROUND_MAP_GAIN[j])
                / GROUND_MAP_GAIN[i].min(GROUND_MAP_GAIN[j]);
            assert!(
                ratio > 1.01,
                "{} and {} have gains within 1% of each other ({} vs {}).\n\
                 Either two slots are bound to the same file, or two sources \
                 have the same mean — check which before widening this.",
                ROLES[i],
                ROLES[j],
                GROUND_MAP_GAIN[i],
                GROUND_MAP_GAIN[j]
            );
        }
    }
}
