//! Gate: what a blow throws, and that the particle pools stay inside their
//! bounds. Arithmetic on the pools and the table — no `World`, no GPU. The
//! pool's own laws (a spark's width, its bounce, the sort) are the shared
//! crate's, tested in `crates/fx/tests`; these are Gates' half.

#![cfg(feature = "render")]

use bevy::math::Vec3;
use bevy::mesh::{Mesh, VertexAttributeValues};
use client::render::fx::gun::{cosmetic_mark, first_in_line, seg_dist};
use client::render::fx::pool::{pool_mesh, Cam, MeshWrite, Pool};
use client::render::fx::table::{effect, emit, lod, scaled, Layer};
use client::render::impact::{Matter, Weapon};

const WEAPONS: [Weapon; 3] = [Weapon::Melee, Weapon::Arrow, Weapon::Bullet];

/// Wall 4 on a client path: a firefight cannot grow a pool, and a full pool
/// recycles its oldest rather than dropping the newest blow.
#[test]
fn a_pool_is_bounded_and_recycles() {
    let mut p = Pool::new(64, false);
    for _ in 0..40 {
        emit(
            &mut p,
            Layer::Sparks,
            12,
            Vec3::ZERO,
            Vec3::Y,
            Matter::Metal,
        );
    }
    assert_eq!(p.live(), 64);
    assert_eq!(p.spawned, 480);
    assert_eq!(p.stolen, 480 - 64, "every overflow is counted");
    p.clear();
    assert_eq!(p.live(), 0);
}

/// Wood and soil never spark, whatever struck them; metal always does; a
/// body bleeds and throws no chips; water splashes and marks nothing solid.
#[test]
fn only_hard_matter_sparks() {
    for w in WEAPONS {
        for m in [
            Matter::Wood,
            Matter::Dirt,
            Matter::Sand,
            Matter::Grass,
            Matter::Plant,
        ] {
            assert_eq!(
                effect(w, m).count(Layer::Sparks),
                0,
                "{w:?} on {m:?} sparks"
            );
        }
        assert!(
            effect(w, Matter::Metal).count(Layer::Sparks) > 0,
            "{w:?} on metal"
        );
        let blood = effect(w, Matter::Flesh);
        assert!(blood.count(Layer::BloodMist) > 0 && blood.count(Layer::BloodDrops) > 0);
        assert_eq!(blood.chips, 0);
        let water = effect(w, Matter::Water);
        assert!(water.count(Layer::Droplets) > 0 && water.count(Layer::Ring) > 0);
        assert_eq!(water.count(Layer::Dust), 0);
        assert_eq!(water.chips, 0);
        assert!(effect(w, Matter::Plant).count(Layer::Leaves) > 0);
    }
    // Metal showers; stone only flashes a few.
    assert!(
        effect(Weapon::Bullet, Matter::Metal).count(Layer::Sparks)
            > effect(Weapon::Bullet, Matter::Stone).count(Layer::Sparks)
    );
    // Soil is a dust cloud and flying grit.
    let soil = effect(Weapon::Bullet, Matter::Dirt);
    assert!(soil.count(Layer::Dust) > 0 && soil.count(Layer::Grit) > 0);
}

/// Less with distance, nothing past 100 m, and a layer in range is never
/// rounded to nothing.
#[test]
fn particles_thin_with_distance() {
    let mut last = f32::MAX;
    for d in [0.0, 10.0, 30.0, 60.0, 99.0, 101.0, 500.0] {
        let l = lod(d);
        assert!(l <= last, "lod rose with distance at {d} m");
        last = l;
    }
    assert_eq!(lod(101.0), 0.0);
    assert_eq!(scaled(1, lod(90.0)), 1);
    assert_eq!(scaled(12, 1.0), 12);
    assert_eq!(scaled(0, 1.0), 0);
}

/// The mesh is written in place: live particles are quads, dead ones are
/// collapsed, and an empty pool writes once more to clear and then stops.
#[test]
fn the_mesh_collapses_the_dead_and_goes_quiet() {
    let mut mesh: Mesh = pool_mesh(16);
    let mut p = Pool::new(16, true);
    let eye = Vec3::new(0.0, 1.6, 5.0);
    let cam = Cam {
        pos: eye.into(),
        right: Vec3::X.into(),
        up: Vec3::Y.into(),
    };
    emit(&mut p, Layer::Dust, 3, Vec3::ZERO, Vec3::Y, Matter::Sand);
    assert!(p.needs_write());
    assert!(p.write(&mut mesh, &cam, eye));
    let positions = |m: &Mesh| match m.attribute(Mesh::ATTRIBUTE_POSITION) {
        Some(VertexAttributeValues::Float32x3(v)) => v.clone(),
        _ => panic!("no positions"),
    };
    let quads = positions(&mesh)
        .chunks(4)
        .filter(|q| q.iter().any(|v| *v != [0.0; 3]))
        .count();
    assert_eq!(quads, 3);
    for _ in 0..200 {
        p.step(0.05);
    }
    assert_eq!(p.live(), 0);
    assert!(p.needs_write(), "the dead must be cleared once");
    assert!(p.write(&mut mesh, &cam, eye));
    assert!(positions(&mesh).iter().all(|v| *v == [0.0; 3]));
    assert!(!p.needs_write(), "and then the mesh is left alone");
}

/// The generated sprite sheet's level 0, pinned: the sheet moved into the
/// shared `fx` crate byte for byte, and a change to a sprite is a change to
/// every effect that draws it (look at the strips, `ci/fx.sh`, then repin).
#[test]
fn the_sprite_sheet_is_unchanged() {
    let (px, w, h) = client::render::fx::atlas::sprite_pixels();
    assert_eq!((w, h), (256, 256));
    let mut d: u64 = 0xcbf2_9ce4_8422_2325;
    for b in px {
        d = (d ^ b as u64).wrapping_mul(0x100_0000_01b3);
    }
    assert_eq!(d, 0xcfa8_8247_627f_258f, "sprite sheet digest {d:#018x}");
}

/// A client draws a miss exactly where the shard did not: the beam stopped on
/// something, past the shard's mark walk, and no body stood in its line.
#[test]
fn the_client_marks_only_what_the_shard_did_not() {
    use sim_core::limits::MAX_HITSCAN_MARK_SAMPLES as K;
    assert!(
        cosmetic_mark(K + 1, true, false),
        "a far miss is the client's"
    );
    assert!(!cosmetic_mark(K, true, false), "the shard marked this one");
    assert!(!cosmetic_mark(K + 50, false, false), "it reached nothing");
    assert!(!cosmetic_mark(K + 50, true, true), "a body took it");
}

/// The body-in-line test's geometry: a point beside the segment is its
/// perpendicular distance away; past an end, the distance to that end.
#[test]
fn distance_to_a_shot_line() {
    let (a, b) = (Vec3::ZERO, Vec3::new(10.0, 0.0, 0.0));
    assert!((seg_dist(Vec3::new(5.0, 0.4, 0.0), a, b) - 0.4).abs() < 1e-5);
    assert!((seg_dist(Vec3::new(12.0, 0.0, 0.0), a, b) - 2.0).abs() < 1e-5);
    assert!((seg_dist(Vec3::new(-3.0, 4.0, 0.0), a, b) - 5.0).abs() < 1e-5);
}

/// A round stops at the first body or animal in its line — the tracer ends
/// there rather than flying on through, and nothing behind it is marked.
#[test]
fn a_shot_stops_at_the_first_thing_standing_in_its_line() {
    let (a, b) = (Vec3::ZERO, Vec3::new(20.0, 0.0, 0.0));
    let near = (Vec3::new(6.0, 0.3, 0.0), 0.5);
    let far = (Vec3::new(12.0, 0.0, 0.2), 0.5);
    let off = (Vec3::new(3.0, 0.0, 2.0), 0.5);
    let t = first_in_line(a, b, [far, off, near].into_iter()).expect("a target in line");
    assert!(
        (t - 6.0).abs() < 1e-5,
        "stopped at {t}, not the nearer target"
    );
    assert_eq!(
        first_in_line(a, b, [off].into_iter()),
        None,
        "a target off the line"
    );
    let behind = (Vec3::new(-2.0, 0.0, 0.0), 0.5);
    assert_eq!(
        first_in_line(a, b, [behind].into_iter()),
        None,
        "a target behind the muzzle"
    );
}
