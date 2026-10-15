//! The Verdigris spreads (`blight.rs`): wipe day is exactly the heart, the
//! front only grows, THE GATE and the beaches stay clean at any hour, and
//! each wipe's outbreaks are its own.

use sim_core::blight::{self, Depth, Plan};
use sim_core::terrain;

const SEEDS: [u64; 3] = [20260731, 0x0047_4154_4553, 0xDEAD_BEEF];
const SALTS: [u32; 3] = [0, 0x1234_5679, 0xDEAD_0001];
const HOUR: u64 = blight::TICKS_PER_HOUR;

fn grid(step: f32, mut f: impl FnMut(f32, f32)) {
    let n = (terrain::ISLAND_SIZE / step) as i32;
    for iz in 0..n {
        for ix in 0..n {
            f((ix as f32 + 0.5) * step, (iz as f32 + 0.5) * step);
        }
    }
}

fn at(now: &blight::Now, seed: u64, x: f32, z: f32) -> Depth {
    blight::depth(now, seed, x, z, terrain::height(seed, x, z))
}

/// The load-bearing promise: at hour 0 in tier A the spread IS the heart,
/// bit for bit (held at 0, where the heart's fade overshoots by an ulp), so
/// every test and fixture written against `terrain::fel` still holds on wipe
/// day.
#[test]
fn wipe_day_is_the_heart_exactly() {
    for seed in SEEDS {
        let haven = terrain::haven(seed);
        let plan = Plan::new(seed, 0x5EED, &haven);
        let now = blight::now(&plan, &haven, 0, 0);
        assert_eq!(now.front, blight::F0);
        grid(24.0, |x, z| {
            let d = at(&now, seed, x, z);
            assert_eq!(
                d.fel.to_bits(),
                terrain::fel(seed, x, z).max(0.0).to_bits(),
                "seed {seed:#x}: wipe day at ({x},{z}) is not the heart"
            );
            assert_eq!(d.hot, 0.0, "seed {seed:#x}: a hot spot burns on wipe day");
        });
    }
}

#[test]
fn the_front_never_falls_back_and_every_tier_climb_moves_it() {
    let mut last = 0.0;
    for h in 0..400 {
        let f = blight::front(h as f32, 0);
        assert!(f >= last, "the front fell back at hour {h}");
        last = f;
        for t in 0..2u8 {
            assert!(
                blight::front(h as f32, t + 1) > blight::front(h as f32, t),
                "hour {h}: tier {} to {} did not surge",
                t,
                t + 1
            );
        }
    }
    // The clock reaches the floats through milli-hours: a week of ticks is
    // past f32's exact integers, and the hour must not wobble.
    assert_eq!(blight::hours(168 * HOUR), 168.0);
    assert_eq!(
        blight::front(400.0, 0),
        blight::front(blight::START_H + blight::GROW_H, 0)
    );
}

/// THE GATE holds and the coast stays clean, at every hour, tier and salt.
#[test]
fn the_town_and_the_beaches_stay_clean() {
    for seed in SEEDS {
        let haven = terrain::haven(seed);
        let (tx, tz) = blight::ward_centre(&haven);
        // The town's ring and the island's beaches, found once.
        let mut points = Vec::new();
        grid(16.0, |x, z| {
            let (dx, dz) = (x - tx, z - tz);
            let near_town = dx * dx + dz * dz < blight::WARD_IN_M * blight::WARD_IN_M;
            let ground = terrain::height(seed, x, z);
            if near_town || ground < terrain::BEACH_MAX_H {
                points.push((x, z, ground, near_town));
            }
        });
        for salt in SALTS {
            let plan = Plan::new(seed, salt, &haven);
            for (h, tier) in [(48u64, 1u8), (120, 1), (168, 2), (400, 2)] {
                let now = blight::now(&plan, &haven, h * HOUR, tier);
                for &(x, z, ground, near_town) in &points {
                    let beach = ground < terrain::BEACH_MAX_H;
                    let d = blight::depth(&now, seed, x, z, ground);
                    let heart = terrain::fel(seed, x, z).max(0.0);
                    assert_eq!(
                        d.fel, heart,
                        "seed {seed:#x} salt {salt:#x} hour {h}: the spread reached ({x},{z}) \
                         (town {near_town}, beach {beach})"
                    );
                    if near_town {
                        assert_eq!(d.hot, 0.0, "a hot spot inside the town's ward");
                    }
                }
            }
        }
    }
}

#[test]
fn each_wipe_draws_its_own_outbreaks() {
    for seed in SEEDS {
        let haven = terrain::haven(seed);
        let a = Plan::new(seed, 7, &haven);
        assert_eq!(
            a,
            Plan::new(seed, 7, &haven),
            "the same salt must draw the same sites"
        );
        assert!(
            a.n >= 3,
            "seed {seed:#x}: only {} outbreaks found room",
            a.n
        );
        let b = Plan::new(seed, 8, &haven);
        assert_ne!(a, b, "seed {seed:#x}: two salts drew the same outbreaks");
        let (tx, tz) = blight::ward_centre(&haven);
        for s in a.sites.iter().take(a.n as usize) {
            let (dx, dz) = (s.x - tx, s.z - tz);
            assert!(
                (dx * dx + dz * dz).sqrt() > blight::WARD_OUT_M + s.r,
                "an outbreak inside the town's ward"
            );
            assert!(s.start_h > 0.0 && s.r >= blight::OUTBREAK_R_M.0);
        }
    }
}

/// It grows: more of the island is green late than early, and an outbreak
/// is green once it has begun.
#[test]
fn the_island_greens_as_the_wipe_ages() {
    let seed = SEEDS[0];
    let haven = terrain::haven(seed);
    let plan = Plan::new(seed, 0xABCD, &haven);
    let share = |h: u64, tier: u8| {
        let now = blight::now(&plan, &haven, h * HOUR, tier);
        let (mut land, mut green) = (0u32, 0u32);
        grid(32.0, |x, z| {
            let g = terrain::height(seed, x, z);
            if g < terrain::LAND_MIN_H {
                return;
            }
            land += 1;
            if blight::depth(&now, seed, x, z, g).fel >= 0.5 {
                green += 1;
            }
        });
        green as f32 / land as f32
    };
    let (d0, d2, d7) = (share(0, 0), share(48, 1), share(168, 1));
    assert!(d0 < d2 && d2 < d7, "the green did not grow: {d0} {d2} {d7}");
    assert!(d7 > 2.0 * d0, "a week took only {d0} to {d7} of the land");
    let s = plan.sites[0];
    let now = blight::now(
        &plan,
        &haven,
        ((s.start_h + blight::OUTBREAK_GROW_H) as u64 + 1) * HOUR,
        1,
    );
    let g = terrain::height(seed, s.x, s.z);
    if g > terrain::BEACH_MAX_H + blight::COAST_M {
        assert!(
            blight::depth(&now, seed, s.x, s.z, g).fel > 0.5,
            "a grown outbreak's core is not green"
        );
    }
}
