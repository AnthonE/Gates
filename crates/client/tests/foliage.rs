//! The foliage material's CPU half (`render/foliage.rs`): the wrapping trail
//! map, and the few numbers that must agree with each other for the shader
//! to look right. Renderer tier: links Bevy for `Mesh`, opens no window.

#![cfg(feature = "render")]

use bevy::mesh::VertexAttributeValues;
use bevy::prelude::*;
use client::render::clutter::{element_mesh, CLUTTER_RING};
use client::render::foliage::{
    Foliages, Kind, Trample, GRASS_FADE_END_M, GRASS_FADE_START_M, TRAIL_S, TRAMPLE_M,
};
use sim_core::terrain::{Clutter, ClutterElem, CLUTTER_TILE_M};

#[test]
fn a_trail_is_where_it_was_walked_and_springs_back() {
    let mut t = Trample::default();
    let here = Vec2::new(1501.3, 603.7);
    t.recentre(here);
    for _ in 0..30 {
        t.stamp(here, 1.0 / 30.0);
    }
    assert!(t.at(here) > 0.5, "a second's standing flattens the grass");
    assert_eq!(t.at(here + Vec2::new(3.0, 0.0)), 0.0, "and only there");
    t.decay(TRAIL_S);
    assert_eq!(t.at(here), 0.0, "and it springs back inside TRAIL_S");
}

/// The map wraps: walking away reuses the slots the old trail sat in, and
/// a slot must be cleared when its world cell changes, or an old trail
/// reappears a window-width away from where it was walked.
#[test]
fn a_trail_left_behind_does_not_reappear_elsewhere() {
    let mut t = Trample::default();
    let a = Vec2::new(100.0, 100.0);
    t.recentre(a);
    t.stamp(a, 1.0);
    assert!(t.at(a) > 0.0);
    // One window away, the slot `a` used now holds a different world cell.
    let b = a + Vec2::new(TRAMPLE_M, 0.0);
    t.recentre(b);
    assert_eq!(t.at(b), 0.0, "the old trail aliased onto the new window");
    // Back again: `a`'s cell was cleared when it left, so nothing is there.
    t.recentre(a);
    assert_eq!(t.at(a), 0.0, "a trail survived its slot being reused");
}

/// A trail inside the window survives the window following the walker.
#[test]
fn a_trail_survives_a_short_walk() {
    let mut t = Trample::default();
    let a = Vec2::new(-40.0, 12.0);
    t.recentre(a);
    t.stamp(a, 1.0);
    let v = t.at(a);
    t.recentre(a + Vec2::new(5.0, -3.0));
    assert_eq!(t.at(a), v);
}

/// The trunk and the canopy are two meshes of one tree; if they swing by
/// different laws the needles leave their branches.
#[test]
fn a_tree_sways_as_one() {
    let bark = Kind::Bark.params().sway.truncate();
    assert_eq!(Kind::Needle.params().sway.truncate(), bark);
    assert_eq!(Kind::Leaf.params().sway.truncate(), bark);
    assert_eq!(Kind::Bark.params().sway.w, 0.0, "a trunk does not flutter");
}

/// A plant's leaves and its wood are two meshes too: the same bend and the
/// same parting, or the berries and the stalk come out of the leaves when the
/// wind blows or somebody pushes through.
#[test]
fn a_plant_sways_and_parts_as_one() {
    for (leaf, wood) in [
        (Kind::BushLeaf, Kind::Stem),
        (Kind::HempLeaf, Kind::HempStem),
    ] {
        let (l, w) = (leaf.params(), wood.params());
        assert_eq!(l.sway.truncate(), w.sway.truncate(), "{leaf:?} / {wood:?}");
        assert_eq!(l.misc, w.misc, "{leaf:?} / {wood:?}");
        assert!(l.misc.y > 0.0, "{leaf:?} does not part for a body");
        assert_eq!(w.sway.w, 0.0, "{wood:?} flutters");
    }
    // Hemp is the soft one.
    assert!(Kind::HempLeaf.params().sway.x > Kind::BushLeaf.params().sway.x);
    // A tree and the grass do not part (the grass has its trails).
    for k in [Kind::Grass, Kind::Bark, Kind::Needle, Kind::Leaf] {
        assert_eq!(k.params().misc.y, 0.0, "{k:?}");
    }
}

/// The grass must be gone before the ring's nearest edge, or a tile is seen
/// streaming in and out — the popping square the fade exists to remove.
#[test]
fn the_grass_has_faded_before_the_ring_ends() {
    const NEAREST_EDGE: f32 = CLUTTER_RING as f32 * CLUTTER_TILE_M;
    const { assert!(GRASS_FADE_START_M < GRASS_FADE_END_M) };
    const { assert!(GRASS_FADE_END_M < NEAREST_EDGE) };
}

/// A grass card tells the shader how high each corner stands above its root
/// (`UV_1.x`): 0 at the root, so the root never moves, and the card's height
/// at the tip.
#[test]
fn a_tuft_knows_its_height() {
    let m = element_mesh(&ClutterElem {
        kind: Clutter::Tuft,
        x: 10.0,
        y: 2.0,
        z: -4.0,
        yaw: 40,
        scale: 1.0,
    });
    let Some(VertexAttributeValues::Float32x2(uv1)) = m.attribute(Mesh::ATTRIBUTE_UV_1.id) else {
        panic!("a grass card carries UV_1");
    };
    let Some(VertexAttributeValues::Float32x3(pos)) = m.attribute(Mesh::ATTRIBUTE_POSITION.id)
    else {
        panic!("positions");
    };
    let low = pos.iter().map(|p| p[1]).fold(f32::INFINITY, f32::min);
    for (p, h) in pos.iter().zip(uv1) {
        assert!((h[0] - (p[1] - low)).abs() < 1e-4, "{h:?} at {p:?}");
        assert!((0.0..1.0).contains(&h[1]));
    }
}

/// Transmission is paid for: Bevy takes `t` out of the lit side to light
/// the back, so the twin's base colour is raised by `1 / (1 − t)` and the
/// lit side draws exactly as the `StandardMaterial` it was made from.
#[test]
fn transmission_does_not_darken_the_lit_side() {
    let f = Foliages::new(Handle::default(), Handle::default());
    for kind in [
        Kind::Grass,
        Kind::BushLeaf,
        Kind::HempLeaf,
        Kind::Stem,
        Kind::HempStem,
        Kind::Needle,
        Kind::Leaf,
        Kind::Bark,
    ] {
        let base = StandardMaterial {
            base_color: Color::linear_rgb(0.7, 0.7, 0.7),
            ..default()
        };
        let m = f.make(base, kind);
        let t = kind.transmission();
        assert_eq!(m.base.diffuse_transmission, t, "{kind:?}");
        let lit = m.base.base_color.to_linear().red * (1.0 - t);
        assert!((lit - 0.7).abs() < 1e-5, "{kind:?}: lit side {lit}");
        assert_eq!(m.extension.params, kind.params());
    }
}
