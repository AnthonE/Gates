//! Gate: what a blow on stone or metal throws that is hot, and that it stays
//! inside its bound.
//!
//! `tests/impact.rs`'s posture, one layer over: `render/sparks.rs` keeps the
//! whole of a spark's state and motion on `Sparks` — `ignite`, `step`, `draw`
//! — so every law below runs with no `World`, no GPU and no shard. What no
//! test here claims is that the systems are SCHEDULED.

#![cfg(feature = "render")]

use bevy::math::{Quat, Vec3};
use client::render::impact::{Burst, Matter};
use client::render::sparks::{
    burst_size, Sparks, SPARK_BURST_METAL, SPARK_BURST_STONE, SPARK_HEAT, SPARK_LEN_MAX_M,
    SPARK_LEN_MIN_M, SPARK_LIFE_S, SPARK_POOL, SPARK_WIDTH_M,
};

fn burst_at(at: Vec3, away: Vec3, matter: Matter) -> Burst {
    Burst { at, away, matter }
}

/// Wall 4, on a client-driven path: the overflow policy is drop-OLDEST, so
/// the assertion is that a saturated pool keeps taking bursts and never
/// draws past its cap.
#[test]
fn the_pool_is_bounded_and_says_so() {
    let mut p = Sparks::default();
    for _ in 0..40 {
        p.ignite(&burst_at(Vec3::ZERO, Vec3::Y, Matter::Metal));
    }
    assert_eq!(p.bursts, 40, "every burst was taken, none refused");
    assert_eq!(p.live(), SPARK_POOL, "a saturated pool draws exactly its cap");
    assert!(
        p.stolen >= (40 * SPARK_BURST_METAL - SPARK_POOL) as u64,
        "past the cap every spark has to come from an older one, got {} steals",
        p.stolen
    );
}

/// Only the two hard matters spark. Wood, flesh, plant and dirt throw
/// nothing — a hatchet does not strike fire from a trunk — and a burst on
/// them costs the pool nothing, not even a count.
#[test]
fn only_stone_and_metal_spark() {
    assert_eq!(burst_size(Matter::Metal), SPARK_BURST_METAL);
    assert_eq!(burst_size(Matter::Stone), SPARK_BURST_STONE);
    let (metal, stone) = (burst_size(Matter::Metal), burst_size(Matter::Stone));
    assert!(
        metal > stone && stone > 0,
        "metal throws a shower, stone a few, and both throw some"
    );
    for m in [Matter::Wood, Matter::Flesh, Matter::Plant, Matter::Dirt] {
        assert_eq!(burst_size(m), 0, "{m:?} sparks, and it is not metal");
        let mut p = Sparks::default();
        p.ignite(&burst_at(Vec3::ZERO, Vec3::Y, m));
        assert_eq!(p.live(), 0, "a burst on {m:?} lit sparks");
        assert_eq!(p.bursts, 0, "a burst on {m:?} was counted as sparking");
    }
    let mut p = Sparks::default();
    p.ignite(&burst_at(Vec3::ZERO, Vec3::Y, Matter::Metal));
    assert_eq!(p.live(), SPARK_BURST_METAL, "one metal burst is its full count");
}

/// A shower leaves the face it came off, every spark of it.
#[test]
fn a_shower_leaves_the_face_it_came_off() {
    let at = Vec3::new(3.0, 1.0, -2.0);
    let away = Vec3::new(0.0, 0.0, 1.0);
    let mut p = Sparks::default();
    p.ignite(&burst_at(at, away, Matter::Metal));
    let mut worst = 1.0f32;
    for i in 0..SPARK_POOL {
        let Some((pos, _, _, _)) = p.draw(i) else {
            continue;
        };
        worst = worst.min((pos - at).normalize().dot(away));
    }
    assert!(
        worst > 0.0,
        "a spark left at {worst:.2} against the normal — into the thing that was hit"
    );
}

/// A spark is a streak along its own velocity, and it cools: white at the
/// blow, red at the end, gone after. The rotation's local +X is the
/// velocity's direction, the length is inside its band, and the rung climbs
/// the ladder without ever stepping back.
#[test]
fn a_spark_streaks_along_its_velocity_and_cools() {
    let mut p = Sparks::default();
    p.ignite(&burst_at(Vec3::ZERO, Vec3::Y, Matter::Stone));
    let dt = 1.0 / 90.0;
    let mut last_rung = vec![0usize; SPARK_POOL];
    let mut seen_hot = false;
    let mut seen_cold = false;
    let mut first_width = None;
    let mut last_width = None;
    for _ in 0..((SPARK_LIFE_S * 1.3 / dt) as usize) {
        for (i, last) in last_rung.iter_mut().enumerate() {
            let Some((_, rot, scale, rung)) = p.draw(i) else {
                continue;
            };
            assert!(
                (SPARK_LEN_MIN_M..=SPARK_LEN_MAX_M).contains(&scale.x),
                "a streak drew {:.3} m long, outside its band",
                scale.x
            );
            assert!(scale.y <= SPARK_WIDTH_M && scale.y == scale.z, "a streak is round");
            assert!(
                rung >= *last,
                "spark {i} went from rung {} back to {rung} — it warmed up",
                *last
            );
            *last = rung;
            seen_hot |= rung == 0;
            seen_cold |= rung == SPARK_HEAT.len() - 1;
            if first_width.is_none() {
                first_width = Some(scale.y);
            }
            last_width = Some(scale.y);
            // The rotation carries local +X onto the direction of travel:
            // step the same spark and check it moved that way.
            let x: Vec3 = rot * Vec3::X;
            assert!((x.length() - 1.0).abs() < 1e-4, "a rotation is a rotation");
        }
        p.step(dt);
    }
    assert!(seen_hot && seen_cold, "the ladder was not walked end to end");
    assert_eq!(first_width, Some(SPARK_WIDTH_M), "a spark starts full width");
    assert!(
        last_width.unwrap() < SPARK_WIDTH_M * 0.5,
        "the last frame drew a spark still {:.4} m wide — it pops instead of going out",
        last_width.unwrap()
    );
    assert_eq!(p.live(), 0, "sparks outlived their own lifetime");
}

/// The streak's orientation really is the velocity: after one step the
/// spark has moved along the axis the rotation reports. Checked on the
/// draw's own output rather than on any private field.
#[test]
fn the_streak_points_where_the_spark_is_going() {
    let mut p = Sparks::default();
    p.ignite(&burst_at(Vec3::ZERO, Vec3::Y, Matter::Metal));
    let before: Vec<(Vec3, Quat)> = (0..SPARK_POOL)
        .filter_map(|i| p.draw(i).map(|d| (d.0, d.1)))
        .collect();
    p.step(0.005);
    let after: Vec<Vec3> = (0..SPARK_POOL)
        .filter_map(|i| p.draw(i).map(|d| d.0))
        .collect();
    assert_eq!(before.len(), after.len());
    for ((pos, rot), next) in before.iter().zip(&after) {
        let moved = (*next - *pos).normalize();
        let axis = *rot * Vec3::X;
        assert!(
            moved.dot(axis) > 0.99,
            "the streak points {axis:?} and the spark moved {moved:?}"
        );
    }
}

/// A spark slows in the air, falls, and goes out.
#[test]
fn sparks_slow_down_fall_and_go_out() {
    let mut p = Sparks::default();
    p.ignite(&burst_at(Vec3::new(0.0, 4.0, 0.0), Vec3::Y, Matter::Metal));
    let dt = 1.0 / 60.0;
    let speed = |p: &mut Sparks| -> f32 {
        let a: Vec<Vec3> = (0..SPARK_POOL)
            .filter_map(|i| p.draw(i).map(|d| d.0))
            .collect();
        p.step(dt);
        let b: Vec<Vec3> = (0..SPARK_POOL)
            .filter_map(|i| p.draw(i).map(|d| d.0))
            .collect();
        assert_eq!(a.len(), b.len(), "nothing should retire this early");
        a.iter().zip(&b).map(|(x, y)| x.distance(*y)).sum::<f32>() / dt
    };
    let early = speed(&mut p);
    for _ in 0..4 {
        p.step(dt);
    }
    let late = speed(&mut p);
    assert!(
        late < early * 0.9,
        "the shower is still going {late:.2} m/s summed after {early:.2} — no drag"
    );
    // Gravity as a second difference of height, `impact.rs`'s method.
    let rise = |p: &mut Sparks| -> f32 {
        let a: f32 = (0..SPARK_POOL)
            .filter_map(|i| p.draw(i).map(|d| d.0.y))
            .sum();
        p.step(dt);
        let b: f32 = (0..SPARK_POOL)
            .filter_map(|i| p.draw(i).map(|d| d.0.y))
            .sum();
        b - a
    };
    let r0 = rise(&mut p);
    let r1 = rise(&mut p);
    assert!(r1 < r0, "the shower climbs faster over time — nothing pulls it down");
    for _ in 0..((SPARK_LIFE_S * 1.4 / dt) as usize) {
        p.step(dt);
    }
    assert_eq!(p.live(), 0, "sparks outlived their own lifetime");
}

/// The ladder is hot at the top, HDR (past 1.0 — the bloom is the glow),
/// and strictly cooler at every rung down it.
#[test]
fn the_heat_ladder_is_hot_and_monotone() {
    let lum = |c: &[f32; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    assert!(lum(&SPARK_HEAT[0]) > 1.0, "the hottest rung is not HDR; nothing will bloom");
    for w in SPARK_HEAT.windows(2) {
        assert!(
            lum(&w[0]) > lum(&w[1]),
            "the ladder warms up between {:?} and {:?}",
            w[0],
            w[1]
        );
        // And it reddens: the blue share falls as it cools.
        let share = |c: &[f32; 3]| c[2] / (c[0] + c[1] + c[2]);
        assert!(share(&w[0]) >= share(&w[1]), "a cooling spark went bluer");
    }
}

/// Two bursts are not the same burst.
#[test]
fn two_bursts_are_not_the_same_burst() {
    let mut p = Sparks::default();
    p.ignite(&burst_at(Vec3::ZERO, Vec3::Y, Matter::Metal));
    let first: Vec<Vec3> = (0..SPARK_POOL)
        .filter_map(|i| p.draw(i).map(|d| d.0))
        .collect();
    let mut q = Sparks::default();
    for _ in 0..2 {
        q.ignite(&burst_at(Vec3::ZERO, Vec3::Y, Matter::Metal));
    }
    let second: Vec<Vec3> = (SPARK_BURST_METAL..SPARK_BURST_METAL * 2)
        .filter_map(|i| q.draw(i).map(|d| d.0))
        .collect();
    assert_eq!(first.len(), second.len());
    assert!(
        first.iter().zip(&second).any(|(a, b)| a.distance(*b) > 1e-3),
        "the second burst is the first one again — a canned effect"
    );
}
