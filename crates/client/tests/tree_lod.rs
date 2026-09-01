//! Gate: a spawned tree carries the level of detail it is supposed to carry.
//!
//! **Every other tree gate in this crate measures a mesh, and a mesh cannot
//! see this defect.** `tests/tree.rs` proves the impostor is small, that it
//! fits inside the tree it replaces, and that the ring's triangle count falls
//! under `DESIGN.md` §9's ceiling once the far band is a hull. All of that
//! stays true — and every one of those assertions still passes — if
//! `spawn_slot` forgets the [`VisibilityRange`] on one of the three parts. The
//! forest would then be MORE expensive than before the LOD landed: a fourth
//! entity drawn on top of the first three, at every distance, in the depth
//! prepass, the normal prepass, the main pass and all four shadow cascades.
//!
//! `CLAUDE.md`'s trap list has the general shape of it — *"a spawn is not
//! type-checked, so a bundle assembled out of two helpers is a claim you have
//! to run"* — and the loop has already paid for the other end of it, where a
//! duplicate component in one bundle was a runtime panic no gate could see.
//! So this suite runs the real `spawn_slot` against a real `World` and reads
//! back what reached the ECS.
//!
//! No window, no GPU, no socket: `MinimalPlugins` plus the asset plugin, the
//! same fixture `tests/fell.rs` uses.

#![cfg(feature = "render")]

use bevy::asset::AssetPlugin;
use bevy::camera::visibility::VisibilityRange;
use bevy::prelude::*;
use client::render::props::{assets, spawn_slot, FellPart, Fellable, PropAssets};
use client::render::textures::{MapSet, PropMaps};
use client::render::tree::{TreeLod, TREE_LOD_FADE_M, TREE_LOD_SWAP_M};
use sim_core::terrain::{Occupant, Slot};

/// The cell this fixture's slot claims. Any key works — the LOD does not read
/// it — but a felled tree does, so it is the shape `gather::cell_key` packs.
const KEY: u32 = (137u32 << 16) | 42;

/// A yaw that is neither 0 nor a multiple of the pool size, so `spawn_slot`'s
/// `variant` is a real choice rather than always the first mesh.
const YAW: u8 = 137;

fn fixture() -> (App, PropAssets) {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()));
    app.init_asset::<Mesh>();
    app.init_asset::<StandardMaterial>();
    app.init_asset::<Image>();

    let world = app.world_mut();
    let mut meshes = world.remove_resource::<Assets<Mesh>>().unwrap();
    let mut materials = world.remove_resource::<Assets<StandardMaterial>>().unwrap();
    let mut images = world.remove_resource::<Assets<Image>>().unwrap();
    // Handles, never resolved — see `tests/fell.rs` for why that exercises
    // the same code path a loaded map would.
    let maps = PropMaps {
        rock: MapSet::default(),
        bark: MapSet::default(),
        wood: MapSet::default(),
        stone: MapSet::default(),
        metal: MapSet::default(),
    };
    let a = assets(
        &mut meshes,
        &mut materials,
        &mut images,
        &maps,
        // Unresolved, like every `MapSet::default()` above it: this tier has no
        // filesystem and a material clones the handle either way.
        Handle::default(),
        client::render::props::SiteModels::default(),
    );
    world.insert_resource(meshes);
    world.insert_resource(materials);
    world.insert_resource(images);
    (app, a)
}

fn slot(occupant: Occupant) -> Slot {
    Slot {
        occupant,
        x: 14.0,
        y: 12.5,
        z: -7.25,
        yaw: YAW,
        scale: 1.05,
    }
}

/// Spawn one slot through the real `spawn_slot` and hand back its children.
fn spawned(app: &mut App, a: &PropAssets, s: &Slot) -> (Entity, Vec<Entity>) {
    let parent = app
        .world_mut()
        .spawn((Transform::IDENTITY, Visibility::default()))
        .id();
    {
        let mut commands = app.world_mut().commands();
        spawn_slot(&mut commands, parent, a, s, KEY, &TreeLod::default());
    }
    app.world_mut().flush();
    let kids = app
        .world()
        .entity(parent)
        .get::<Children>()
        .map(|c| c.iter().collect::<Vec<_>>())
        .unwrap_or_default();
    (parent, kids)
}

#[test]
fn a_spawned_tree_carries_both_lod_bands() {
    let (mut app, a) = fixture();
    let (_, kids) = spawned(&mut app, &a, &slot(Occupant::Tree));

    // Trunk, stump, canopy, hull. The count is asserted so that adding a part
    // without deciding which band it belongs to fails here rather than
    // shipping an unranged mesh nobody notices.
    assert_eq!(
        kids.len(),
        4,
        "a tree spawned {} parts — trunk, stump, canopy and far hull is four, \
         and a new one needs a VisibilityRange decision before it ships",
        kids.len()
    );

    let lod = TreeLod::default();
    let (near, far) = (&lod.near, &lod.far);
    let variant = YAW as usize % a.pine_variants();
    let want_far = a.impostor_mesh(variant).clone();
    let want_trunk = a.pine_mesh(variant).clone();
    let want_canopy = a.needle_mesh(variant).clone();
    let want_stump = a.stump_mesh().clone();

    let mut seen_trunk = false;
    let mut seen_canopy = false;
    let mut seen_hull = false;
    let mut seen_stump = false;
    for k in kids {
        let e = app.world().entity(k);
        let mesh = e
            .get::<Mesh3d>()
            .expect("every tree part draws a mesh")
            .0
            .clone();
        let range = e.get::<VisibilityRange>();
        if mesh == want_far {
            seen_hull = true;
            assert!(
                range == Some(far),
                "the far hull must carry the far band ({:?}..{:?}), or it \
                 draws at every distance and the LOD is a fourth tree",
                far.start_margin,
                far.end_margin
            );
        } else if mesh == want_trunk {
            seen_trunk = true;
            assert!(
                range == Some(near),
                "the trunk must carry the near band ({:?}..{:?}), or 6 k \
                 triangles survive past the swap and the hull is drawn on top \
                 of them",
                near.start_margin,
                near.end_margin
            );
        } else if mesh == want_canopy {
            seen_canopy = true;
            assert!(
                range == Some(near),
                "the canopy must carry the near band — it is the alpha-masked \
                 half, which is the expensive one in every pass"
            );
        } else if mesh == want_stump {
            seen_stump = true;
            assert!(
                range.is_none(),
                "the stump is deliberately unranged: it is a few dozen \
                 triangles and it only exists after a cut. If that changed, \
                 change this line with it rather than around it"
            );
        } else {
            panic!("a tree spawned a mesh that is none of its four parts");
        }
    }
    assert!(
        seen_trunk && seen_canopy && seen_hull && seen_stump,
        "trunk {seen_trunk}, canopy {seen_canopy}, hull {seen_hull}, stump \
         {seen_stump} — every part must be present exactly once"
    );
}

/// The hull falls with the tree, because it IS the tree at that distance.
#[test]
fn the_hull_topples_as_the_trunk_does() {
    let (mut app, a) = fixture();
    let (_, kids) = spawned(&mut app, &a, &slot(Occupant::Tree));
    let want_far = a.impostor_mesh(YAW as usize % a.pine_variants()).clone();

    let hull = kids
        .iter()
        .copied()
        .find(|k| {
            app.world()
                .entity(*k)
                .get::<Mesh3d>()
                .is_some_and(|m| m.0 == want_far)
        })
        .expect("the tree spawned no hull");
    let e = app.world().entity(hull);
    let f = e
        .get::<Fellable>()
        .expect("the hull must be fellable, or a chopped tree stands at range");
    assert!(
        e.contains::<client::render::props::Topple>(),
        "the hull must carry a Topple — apply_fell leaves the pose alone when \
         one is missing, so a felled far tree would vanish instead of falling"
    );
    assert_eq!(
        f.key, KEY,
        "the hull must answer to the same cell as the tree"
    );

    // Same pivot and same bearing as the trunk, so the two fall as one thing:
    // a second bearing or a second pivot is a tree that breaks in half across
    // the swap distance.
    let trunk = kids
        .iter()
        .copied()
        .find(|k| {
            app.world()
                .entity(*k)
                .get::<Mesh3d>()
                .is_some_and(|m| m.0 == *a.pine_mesh(YAW as usize % a.pine_variants()))
        })
        .expect("the tree spawned no trunk");
    let t = app.world().entity(trunk).get::<Fellable>().unwrap();
    assert_eq!(
        (f.base_y, f.yaw),
        (t.base_y, t.yaw),
        "the hull and the trunk must fall from the same pivot on the same \
         bearing"
    );
    // …and a DIFFERENT role, which is the other half and the one that is easy
    // to get wrong. See below.
    assert_eq!(
        f.part,
        FellPart::Far,
        "the hull must be its own part rather than a second trunk"
    );
}

/// Exactly one part of a tree speaks for the slot.
///
/// **This is the gate for a bug the first cut of this slice shipped.** The
/// hull was given `FellPart::Trunk` on the reasonable-sounding grounds that it
/// falls like one — and `render/audio.rs` reads that variant twice to mean
/// *this entity speaks for the whole slot*: once for the fell cue, which would
/// then play `TreeFall` twice at one position on one frame, and once for the
/// ambience bed's cover count, whose `COVER_FULL` was re-derived 14 → 7 the
/// last time a tree quietly gained an entity. Both are audible-only defects
/// that every visual and arithmetic gate in this repo passes straight over.
///
/// Stated here rather than in `tests/sound.rs` because it is a property of the
/// SPAWN — how many one-per-slot parts a tree has — and the sound suite would
/// have to build a forest to see it.
#[test]
fn exactly_one_part_of_a_tree_speaks_for_the_slot() {
    let (mut app, a) = fixture();
    let (_, kids) = spawned(&mut app, &a, &slot(Occupant::Tree));
    let speakers = kids
        .iter()
        .filter_map(|k| app.world().entity(*k).get::<Fellable>())
        .filter(|f| matches!(f.part, FellPart::Trunk | FellPart::Vanish))
        .count();
    assert_eq!(
        speakers, 1,
        "{speakers} parts of this tree carry a one-per-slot role — \
         `audio::fell` would play {speakers} TreeFall cues for one chop and \
         `audio::bed` would count this tree {speakers} times toward cover"
    );
}

/// Nothing else in the scatter got a range by accident.
#[test]
fn only_a_tree_carries_a_visibility_range() {
    let (mut app, a) = fixture();
    for occupant in [
        Occupant::Rock,
        Occupant::StoneNode,
        Occupant::Bush,
        Occupant::BarrelSlot,
    ] {
        let (_, kids) = spawned(&mut app, &a, &slot(occupant));
        for k in kids {
            assert!(
                app.world().entity(k).get::<VisibilityRange>().is_none(),
                "{occupant:?} carries a VisibilityRange — the tree's bands are \
                 measured against a 6 k-triangle conifer, and a 1,280-triangle \
                 boulder swapping to nothing at {TREE_LOD_SWAP_M} m is a hole \
                 in the frame, not a saving"
            );
        }
    }
}

/// The fade band is wide enough to be a fade.
///
/// A zero-width margin is what `VisibilityRange::abrupt` builds and it is a
/// legal component, so this cannot be caught by construction: the swap would
/// simply pop, which is exactly the artifact the crossfade exists to prevent
/// and exactly the kind of thing no arithmetic gate in this repo would notice.
#[test]
fn the_swap_actually_crossfades() {
    let lod = TreeLod::default();
    let (near, far) = (&lod.near, &lod.far);
    // Bevy's own predicate rather than a read of `TREE_LOD_FADE_M`: the
    // component is what the renderer acts on, and a fade constant that never
    // reaches it is the failure this is looking for.
    assert!(
        !near.is_abrupt() && !far.is_abrupt(),
        "one of the two bands is abrupt — the tree would pop between LODs \
         rather than dithering across {TREE_LOD_FADE_M} m"
    );
}
