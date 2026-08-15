//! Gate: **the card, the body and the map name the same four directions.**
//!
//! `DECISIONS.md` 2026-08-15 settles the axes — north is `+Z`, east is `-X` —
//! and the reason it needed settling is the reason this file exists. The
//! defect was a *reflection*: `bearing_deg` computed `atan2(fx, fz)` and
//! called `+X` east, which is the mirror of what a right-handed Y-up world
//! makes it, so every cardinal on the compass strip sat 180° from the truth
//! and a player turning right watched the number fall.
//!
//! **Nothing in this repo could see it.** The bearing is not on the wire, so
//! `test_protocol_golden` is blind; it is not in `state_hash`, so `test_replay`
//! is blind; it is a `f32` through a `f32` function, so clippy is blind; and a
//! pixel statistic cannot read a compass. It survived every gate for weeks and
//! was found by a person turning around. That is the class this file closes.
//!
//! ## Why the assertions look like this
//!
//! Each one is a **cross-check between two independently written pieces of
//! code**, never a restatement of either. `crates/client/src/ui/map.rs` used
//! to carry `assert!(px_west < px_east)` over two hand-picked x values, which
//! is a monotonicity claim wearing a compass's clothes: green whichever way
//! the x term ran, so it held all the way through the mirrored era and would
//! have held after a fix that mirrored it back. `NOW.md` §0gj named it and
//! this is the replacement.
//!
//! It also closes the shape the decision row worried about out loud —
//! **half a switch is worse than none** (`CLAUDE.md`'s mingw entry). The
//! compass, the map's x term and the map's lighting are three expressions of
//! one fact in three files, and flipping any one of them alone reddens
//! something here.

use client::look::{bearing_deg, bearing_of, right_dir, yaw_u16, MOUSE_RAD_PER_PX};
use client::ui::map::{self, GRID_M};
use sim_core::terrain::ISLAND_SIZE;
use sim_core::yaw_dir;

/// The four cardinals as world-XZ unit directions, taken from the decision
/// row rather than from any function under test.
const CARDINALS: [(&str, f32, f32, f32); 4] = [
    ("north", 0.0, 1.0, 0.0),
    ("east", -1.0, 0.0, 90.0),
    ("south", 0.0, -1.0, 180.0),
    ("west", 1.0, 0.0, 270.0),
];

/// The bearing a movement across the MAP IMAGE reads as, from the image's own
/// convention: up is north, right is east. Deliberately written from the
/// picture rather than from `world_to_map`, so it is a second opinion and not
/// the same arithmetic twice.
fn image_bearing(dpx: f32, dpy: f32) -> f32 {
    (dpx.atan2(-dpy).to_degrees()).rem_euclid(360.0)
}

/// The convention, stated once, checked against the owner.
#[test]
fn the_card_names_the_axes_the_decision_row_named() {
    for (name, dx, dz, want) in CARDINALS {
        let got = bearing_of(dx, dz);
        assert!(
            (got - want).abs() < 0.01,
            "{name} = ({dx}, {dz}) bears {got}°, not {want}°"
        );
    }
}

/// **The physical anchor.** A compass card's east is the right hand of a body
/// facing north — that is what "east" MEANS, and it is the whole derivation
/// the 2026-08-15 row rests on.
///
/// `right_dir` is the function `crates/client/tests/look.rs` checks against
/// Bevy's own `Transform::right()` rather than against its derivation, so
/// this assertion reaches all the way to the engine's basis: if the card and
/// the renderer's idea of "right" ever disagree, one of the two files goes
/// red.
#[test]
fn facing_north_the_bodys_right_is_east() {
    // The yaw whose forward the card calls north.
    let mut north_yaw = None;
    for i in 0..256u32 {
        let yaw = i as f32 * std::f32::consts::TAU / 256.0;
        if bearing_deg(yaw) < 0.001 {
            north_yaw = Some(yaw);
            break;
        }
    }
    let yaw = north_yaw.expect("no yaw reads as due north");
    let (fx, fz) = yaw_dir(yaw_u16(yaw));
    assert!(fz > 0.99, "the card's north is not +Z: ({fx}, {fz})");

    let (rx, rz) = right_dir(yaw_u16(yaw));
    let deg = bearing_of(rx, rz);
    assert!(
        (deg - 90.0).abs() < 1.0,
        "facing north, the body's right ({rx}, {rz}) bears {deg}° — east is 90°"
    );
}

/// **The cross-check the whole file is for: walk any bearing, and the map
/// marker moves that bearing across the picture.**
///
/// Continuous rather than four-cardinal, so a reflection on either axis is
/// caught wherever it hides. It links three files that were written
/// separately — `sim-core`'s yaw basis, `look::bearing_of`, and
/// `ui::map::world_to_map` — into one claim, and **flipping any one of them
/// alone breaks it by 180° on that axis.** That is the "half a switch"
/// detector: the map's x term and the compass cannot be fixed one at a time.
#[test]
fn the_map_moves_the_way_the_card_says() {
    let c = ISLAND_SIZE * 0.5;
    let size = 512;
    let step = 60.0;
    for i in 0..64u32 {
        let yaw = -std::f32::consts::PI + i as f32 * 0.1;
        let deg = bearing_deg(yaw);
        let (fx, fz) = yaw_dir(yaw_u16(yaw));

        let (px0, py0) = map::world_to_map(c, c, size);
        let (px1, py1) = map::world_to_map(c + fx * step, c + fz * step, size);
        let moved = image_bearing(px1 - px0, py1 - py0);

        let d = (moved - deg).abs();
        let d = d.min(360.0 - d);
        assert!(
            d < 0.5,
            "the card said {deg}° and the map marker went {moved}° \
             (yaw {yaw}, forward ({fx}, {fz}))"
        );
    }
}

/// The grid lettering is the same fact a third time, and it is the one a
/// player reads aloud to a friend. Walking east climbs the alphabet; walking
/// north counts down the rows.
#[test]
fn the_grid_squares_run_the_way_the_card_says() {
    let c = ISLAND_SIZE * 0.5;
    let here = map::grid_label(c, c);
    assert!(!here.is_empty(), "the island centre has no square");

    let letter = |s: &str| s.as_bytes()[0];
    let number = |s: &str| s[1..].parse::<u32>().expect("a row number");

    for (name, dx, dz, _) in CARDINALS {
        let there = map::grid_label(c + dx * GRID_M, c + dz * GRID_M);
        match name {
            "east" => assert!(
                letter(&there) > letter(&here),
                "east went {here} → {there} — the alphabet runs backwards"
            ),
            "west" => assert!(letter(&there) < letter(&here), "west went {here} → {there}"),
            "north" => assert!(
                number(&there) < number(&here),
                "north went {here} → {there} — row 1 is the north edge"
            ),
            _ => assert!(
                number(&there) > number(&here),
                "south went {here} → {there}"
            ),
        }
    }
}

/// **The picture, not the constant.** `LIGHT[0]`'s sign and `paint`'s write
/// index are two halves of one claim — the island is lit from the image's
/// upper-left — and each is green on its own with the other flipped, which is
/// exactly the failure mode the decision row calls half a switch.
///
/// So this reads the painted pixels back and asks the picture. Sampled on
/// slopes that run left–right in the IMAGE and are near flat up–down, so the
/// two axes cannot cover for each other: the face whose uphill side is toward
/// the image's lower-right catches the light and must come out brighter.
///
/// **The margin is measured, not chosen.** Seed 20260731 at 192², lit vs
/// unlit mean luma: **121.8 / 109.5 (+12.3)** correct; **105.4 / 127.6
/// (−22.2)** with `LIGHT[0]` alone reverted — fully inverted; **116.3 /
/// 112.5 (+3.8)** with the write index alone reverted, where the light is
/// right and the pixels it lands on are mirrored, so neighbouring columns
/// leave a residue of the true correlation. The threshold sits between the
/// weakest defect and the true signal. That third case is why this is worth
/// its runtime: it is the half-a-switch state, and it is the one no
/// constant-checking assertion in `ui/map.rs` can see.
#[test]
fn the_painted_island_is_lit_from_the_upper_left() {
    let seed = 20260731u64;
    let size = 192usize;
    let mut buf = vec![0u8; size * size * 4];
    map::paint(seed, size, &mut buf);

    let step = ISLAND_SIZE / size as f32;
    let half = step * 0.5;
    let (mut lit, mut lit_n) = (0.0f64, 0u32);
    let (mut dark, mut dark_n) = (0.0f64, 0u32);

    for j in 1..size - 1 {
        for i in 1..size - 1 {
            let (x, z) = (half + i as f32 * step, half + j as f32 * step);
            let h = sim_core::terrain::height(seed, x, z);
            if h <= sim_core::terrain::SEA_LEVEL {
                continue; // Water carries no hillshade, by design.
            }
            // In the image, column `px = size-1-i` grows east (`-x`) and row
            // `py = size-1-j` grows south (`-z`).
            let rise_right = sim_core::terrain::height(seed, x - step, z) - h;
            let rise_down = sim_core::terrain::height(seed, x, z - step) - h;
            // Isolate the left–right axis: a real tilt across the image, and
            // near nothing up–down, so this cannot pass on the z term.
            if rise_right.abs() < 1.5 || rise_down.abs() > 0.3 {
                continue;
            }
            let o = ((size - 1 - j) * size + (size - 1 - i)) * 4;
            let luma =
                0.2126 * buf[o] as f64 + 0.7152 * buf[o + 1] as f64 + 0.0722 * buf[o + 2] as f64;
            // Uphill toward the image's lower-right = the face turned toward
            // a light in the upper-left.
            if rise_right > 0.0 {
                lit += luma;
                lit_n += 1;
            } else {
                dark += luma;
                dark_n += 1;
            }
        }
    }

    // A vacuous pass is the failure mode this island has already produced
    // once (`CLAUDE.md`: a beige smear satisfied 36 checks).
    assert!(lit_n > 200 && dark_n > 200, "only {lit_n}/{dark_n} samples");
    let (lit, dark) = (lit / lit_n as f64, dark / dark_n as f64);
    assert!(
        lit > dark + 8.0,
        "slopes facing the image's upper-left average {lit:.1} luma and the \
         ones facing away {dark:.1} — the map is lit from the wrong side"
    );
}

/// The strip a player reads is the same number, and it never prints `-00`.
/// The signed-zero trap the decision row names: `(-fx).atan2(fz)` is `-0.0`
/// due north and `f32::rem_euclid`'s guard is `r < 0.0`, which `-0.0` slips
/// through.
#[test]
fn every_bearing_a_player_can_reach_prints_three_digits() {
    let mut yaw = 0.0f32;
    for _ in 0..720 {
        let d = bearing_deg(yaw);
        let s = format!("{d:03.0}");
        assert!(
            s.len() == 3 && s.bytes().all(|b| b.is_ascii_digit()),
            "yaw {yaw} printed {s:?}"
        );
        // 1 px of mouse at a time, all the way round and past the wrap.
        yaw = client::look::yaw_after(yaw, 1.0, MOUSE_RAD_PER_PX * 4.0);
    }
}
