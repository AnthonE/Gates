//! Gate: the rock face (`render/ground_splat.rs` §the rock face) keeps the
//! three promises its header makes, which are arithmetic and so can be held
//! without a GPU.
//!
//! 1. **Every number reaches the shader through the uniform, in its slot.**
//!    The WGSL reads `splat.rock_a`–`rock_d` by component; a swapped slot is
//!    not an error on either side, it is a face tilted by its crack width.
//! 2. **It is brightness-neutral by construction.** The value spread, the
//!    weathering and the streaks each map a draw that is symmetric about 0.5
//!    through a function that is ODD about 0.5, so each has mean 1 — the claim
//!    that lets `fill::GROUND_MIX` keep folding `GROUND_ALBEDO[3]` as the
//!    granite. Scraped here, because an edit that breaks it (a streak that
//!    only darkens, a threshold pair that no longer straddles the middle) is a
//!    one-token change that no value test downstream can see: the face just
//!    gets darker, and brightness is the coupled light owner's, not this
//!    file's (`CLAUDE.md` traps).
//! 3. **The shape knobs sit where the header says they were measured.** The
//!    crack share stays a minority — every boundary a crack read as crazy
//!    paving on the capture — and the streaks stay off ground a player walks.
//!
//! There is no pixel gate here and there must not be one (`CLAUDE.md`).
//! Headless — no GPU, no window, no shard.
//!
//! **`assertions_on_constants` is allowed for `tests/water.rs`'s reason**: the
//! third test asserts relations between knobs, which fold at compile time
//! today and stop folding the day somebody edits one.

#![cfg(feature = "render")]
#![allow(clippy::assertions_on_constants)]

use client::render::ground_splat::{
    GroundSplatParams, ROCK_CELL_M, ROCK_CRACK_DARK, ROCK_CRACK_SHARE, ROCK_CRACK_W,
    ROCK_FACE_FULL, ROCK_FACE_ON, ROCK_FINE_M, ROCK_FINE_TILT, ROCK_SHADE, ROCK_STREAK,
    ROCK_STREAK_H_M, ROCK_STREAK_W_M, ROCK_TILT, ROCK_WARP_M, ROCK_WEATHER, ROCK_WEATHER_M,
};

const SHADER: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../assets/shaders/ground_splat.wgsl"
);

fn shader() -> String {
    std::fs::read_to_string(SHADER).expect("shader")
}

/// The fragment's rock block: from its header comment to the reflectance
/// relief below it. Scoped so a match elsewhere in the file cannot pass for
/// one here.
fn rock_block(src: &str) -> &str {
    let from = src
        .split_once("// --- The rock face ---")
        .expect("no rock-face block in the fragment — the scrape broke, and a gate that matches nothing passes for free")
        .1;
    from.split_once("let relief = clamp(")
        .expect("the rock block no longer ends where it did")
        .0
}

#[test]
fn every_rock_knob_reaches_the_shader_in_its_slot() {
    let p = GroundSplatParams::new();
    let want = [
        (p.rock_a.x, ROCK_CELL_M, "rock_a.x", "ROCK_CELL_M"),
        (p.rock_a.y, ROCK_FINE_M, "rock_a.y", "ROCK_FINE_M"),
        (p.rock_a.z, ROCK_TILT, "rock_a.z", "ROCK_TILT"),
        (p.rock_a.w, ROCK_FINE_TILT, "rock_a.w", "ROCK_FINE_TILT"),
        (p.rock_b.x, ROCK_SHADE, "rock_b.x", "ROCK_SHADE"),
        (p.rock_b.y, ROCK_WEATHER_M, "rock_b.y", "ROCK_WEATHER_M"),
        (p.rock_b.z, ROCK_WEATHER, "rock_b.z", "ROCK_WEATHER"),
        (p.rock_b.w, ROCK_WARP_M, "rock_b.w", "ROCK_WARP_M"),
        (p.rock_c.x, ROCK_CRACK_SHARE, "rock_c.x", "ROCK_CRACK_SHARE"),
        (p.rock_c.y, ROCK_CRACK_W, "rock_c.y", "ROCK_CRACK_W"),
        (p.rock_c.z, ROCK_CRACK_DARK, "rock_c.z", "ROCK_CRACK_DARK"),
        (p.rock_c.w, ROCK_STREAK, "rock_c.w", "ROCK_STREAK"),
        (p.rock_d.x, ROCK_STREAK_W_M, "rock_d.x", "ROCK_STREAK_W_M"),
        (p.rock_d.y, ROCK_STREAK_H_M, "rock_d.y", "ROCK_STREAK_H_M"),
        (p.rock_d.z, ROCK_FACE_ON, "rock_d.z", "ROCK_FACE_ON"),
        (p.rock_d.w, ROCK_FACE_FULL, "rock_d.w", "ROCK_FACE_FULL"),
    ];
    for (got, knob, slot, name) in want {
        assert_eq!(
            got.to_bits(),
            knob.to_bits(),
            "{slot} carries {got}, not {name} = {knob}"
        );
    }
    // And the shader reads each slot — a slot nothing reads is a knob that
    // moves nothing, which is the registry's failure in another language.
    let src = shader();
    let block = rock_block(&src);
    for (_, _, slot, name) in want {
        assert!(
            block.contains(&format!("splat.{slot}")),
            "the rock block never reads `splat.{slot}` ({name}) — the knob ships and moves nothing"
        );
    }
    // And no shape number came back as a WGSL literal constant.
    assert!(
        !src.contains("const ROCK_"),
        "a `const ROCK_*` is declared in the shader — a knob that lives only in \
         WGSL is one the knob registry cannot see (`GroundSplatParams::blend`)"
    );
}

/// `2.0 * x - 1.0` of a draw symmetric about 0.5 is odd, so the term it scales
/// has mean exactly 1. Each of the three modulations must keep that form.
#[test]
fn the_face_is_lightened_as_much_as_it_is_darkened() {
    let src = shader();
    let block = rock_block(&src);
    for (what, needle) in [
        ("the per-block value", "splat.rock_b.x * (2.0 * d.y - 1.0)"),
        ("the fine blocks' value", "(2.0 * draw3(fine.key).y - 1.0)"),
        ("the weathering", "splat.rock_b.z * (2.0 * wx - 1.0)"),
    ] {
        assert!(
            block.contains(needle),
            "{what} no longer has the odd form `{needle}` — a term that is not \
             odd about the middle of its draw moves the granite's mean"
        );
    }
    // The streak's sigmoid is odd about 0.5 iff its thresholds straddle 0.5
    // symmetrically: smoothstep(a, 1 − a, 1 − x) = 1 − smoothstep(a, 1 − a, x).
    let line = block
        .lines()
        .find(|l| l.contains("let streak = 2.0 * smoothstep("))
        .expect("no odd streak sigmoid — a streak that only darkens moves the granite's mean");
    let args = line
        .split_once("smoothstep(")
        .unwrap()
        .1
        .split(',')
        .take(2)
        .map(|t| {
            t.trim()
                .parse::<f64>()
                .expect("a streak threshold is not a literal")
        })
        .collect::<Vec<_>>();
    assert!(
        (args[0] + args[1] - 1.0).abs() < 1e-9 && args[0] < 0.5,
        "the streak thresholds {args:?} do not straddle 0.5 symmetrically, so \
         the streak term is not odd and the face's mean moves"
    );
    assert!(
        block.contains("(1.0 - splat.rock_c.w * face * streak)"),
        "the streak no longer multiplies as 1 − k·face·(odd term)"
    );
}

#[test]
fn the_shape_knobs_sit_where_they_were_measured() {
    assert!(
        ROCK_FINE_M < ROCK_CELL_M * 0.5,
        "the fine blocks must sit inside the large ones"
    );
    assert!(
        ROCK_CRACK_SHARE > 0.0 && ROCK_CRACK_SHARE <= 0.5,
        "a crack on most boundaries outlines every block — crazy paving on the capture"
    );
    assert!(
        ROCK_CRACK_W < 0.1,
        "a crack wider than a tenth of a block is a joint, and joints read as masonry"
    );
    assert!((0.0..1.0).contains(&ROCK_CRACK_DARK));
    // Each lean component is in ±TILT, so the worst lean is atan(√3·TILT):
    // held under 45° for the two scales together, or a facet faces the ground.
    assert!(3f32.sqrt() * (ROCK_TILT + ROCK_FINE_TILT) < 1.0);
    for amp in [ROCK_SHADE, ROCK_WEATHER, ROCK_STREAK] {
        assert!(
            amp > 0.0 && amp < 0.5,
            "a modulation of {amp} can drive the face toward black"
        );
    }
    assert!(
        ROCK_STREAK_H_M > 4.0 * ROCK_STREAK_W_M,
        "a streak is tall, or it is a blotch"
    );
    assert!(ROCK_FACE_ON < ROCK_FACE_FULL && ROCK_FACE_FULL <= 1.0);
    // Streaks start past 30°: a slope a player climbs is not a face water runs down.
    assert!(ROCK_FACE_ON > 0.5);
    assert!(
        ROCK_WARP_M < ROCK_CELL_M * 0.5,
        "a warp past half a block tears the lattice"
    );
}
