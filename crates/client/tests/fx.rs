//! Gate: what a blow throws, and that the particle pools stay inside their
//! bounds. Arithmetic on the pools and the table — no `World`, no GPU.

#![cfg(feature = "render")]

use bevy::math::Vec3;
use bevy::mesh::{Mesh, VertexAttributeValues};
use client::render::fx::gun::{cosmetic_mark, seg_dist};
use client::render::fx::pool::{pool_mesh, Cam, Orient, Particle, Pool, MIN_PX};
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

/// The quad's extent across its streak (corner 0 to corner 3).
fn quad_width(p: &Particle, cam: &Cam) -> f32 {
    let (pos, _, _) = Pool::quad(p, cam, Vec3::ZERO).expect("live");
    (Vec3::from(pos[3]) - Vec3::from(pos[0])).length()
}

/// A spark far away is still at least ~1.5 px wide — the web draws no bloom
/// to widen it.
#[test]
fn a_distant_spark_does_not_vanish() {
    let cam = Cam {
        pos: Vec3::ZERO,
        right: Vec3::X,
        up: Vec3::Y,
    };
    let p = Particle {
        left: 0.3,
        life: 0.3,
        pos: Vec3::new(0.0, 0.0, -60.0),
        vel: Vec3::new(5.0, 0.0, 0.0),
        size0: 0.006,
        size1: 0.006,
        c0: [1.0; 4],
        c1: [1.0; 4],
        orient: Orient::Stretch,
        stretch: 0.016,
        ..Particle::default()
    };
    let w = quad_width(&p, &cam);
    assert!(
        w >= 2.0 * 60.0 * MIN_PX * 0.99,
        "a spark at 60 m draws {w} m wide"
    );
}

/// A spark thrown into the surface it came off bounces back out of it.
#[test]
fn a_spark_does_not_fall_through_its_wall() {
    let mut p = Pool::new(8, false);
    p.spawn(Particle {
        left: 1.0,
        life: 1.0,
        pos: Vec3::new(0.0, 1.0, 0.05),
        vel: Vec3::new(0.0, 0.0, -6.0),
        gravity: 9.81,
        plane_p: Vec3::ZERO,
        plane_n: Vec3::Z,
        ..Particle::default()
    });
    for _ in 0..30 {
        p.step(1.0 / 60.0);
    }
    let s = p.particles().next().expect("still alive");
    assert!(
        s.pos.z >= -1e-4,
        "the spark went through the wall: z = {}",
        s.pos.z
    );
}

/// The mesh is written in place: live particles are quads, dead ones are
/// collapsed, and an empty pool writes once more to clear and then stops.
#[test]
fn the_mesh_collapses_the_dead_and_goes_quiet() {
    let mut mesh: Mesh = pool_mesh(16);
    let mut p = Pool::new(16, true);
    let cam = Cam {
        pos: Vec3::new(0.0, 1.6, 5.0),
        right: Vec3::X,
        up: Vec3::Y,
    };
    emit(&mut p, Layer::Dust, 3, Vec3::ZERO, Vec3::Y, Matter::Sand);
    assert!(p.needs_write());
    assert!(p.write(&mut mesh, &cam, cam.pos));
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
    assert!(p.write(&mut mesh, &cam, cam.pos));
    assert!(positions(&mesh).iter().all(|v| *v == [0.0; 3]));
    assert!(!p.needs_write(), "and then the mesh is left alone");
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
