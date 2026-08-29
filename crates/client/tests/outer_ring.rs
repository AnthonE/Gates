//! Gate: the tree-only outer ring is one hull per tree, planted on the ground
//! that is actually drawn, and it fits the frame's triangle budget.
//!
//! **The ring this widens was sized by a cost that no longer exists.**
//! `NEAR_RADIUS` is 2 — every prop on the island stopped between 128 m and
//! 192 m — and it was chosen when a tree was a 5,900-triangle bark mesh plus
//! an alpha-masked canopy. `tree::impostor_of` made a whole tree one
//! 105-triangle hull, ~55× cheaper, and the radius never moved: the far
//! two-thirds of every wide frame stayed a painted heightfield with nothing
//! standing on it.
//!
//! Three things here that a mesh gate cannot see, in the order they would
//! silently go wrong:
//!
//! 1. **The spawn shape.** A near tree is four entities and an outer tree must
//!    be one. Reusing `spawn_slot` out here would quadruple the entity count
//!    for an identical picture, and every existing tree gate would stay green
//!    because not one triangle would change. `CLAUDE.md`: a spawn is not
//!    type-checked, so a bundle is a claim you have to run.
//! 2. **The ground.** Outside the near ring the surface a player sees is the
//!    8 m far mesh sitting `FAR_DROP` below the real heightfield, so planting
//!    on `scatter`'s own `slot.y` floats every tree over a valley and buries
//!    it on a ridge — measured at 0.630 m worst on the shipped seed, a tenth
//!    of a 6.6 m conifer hanging in the air, at exactly the range where that
//!    is the most obvious thing in the frame.
//! 3. **The budget.** The whole argument for the wider ring is that a hull is
//!    cheap. If the hull ever stopped being cheap — or if an outer tree
//!    quietly acquired the near mesh — the picture would look the same at a
//!    silhouette and cost four times as much.
//!
//! Headless: `MinimalPlugins` plus the asset plugin, the fixture
//! `tests/tree_lod.rs` and `tests/fell.rs` already share.

#![cfg(feature = "render")]

use bevy::asset::AssetPlugin;
use bevy::camera::visibility::VisibilityRange;
use bevy::prelude::*;
use client::render::props::{
    assets, spawn_outer_tree, FellPart, Fellable, PropAssets, Topple, OUTER_CHUNKS, OUTER_RADIUS,
    SINK_M,
};
use client::render::terrain_mesh::{far_ground_y, FAR_DROP, FAR_STEP, NEAR_RADIUS};
use client::render::textures::{MapSet, PropMaps};
use client::render::tree::IMPOSTOR_MAX_TRIS;
use client::render::WorldId;
use sim_core::terrain::{self, Occupant, Slot};

/// The island the shard ships and every frame this project judges is shot on.
const SEED: u64 = 20260731;

const KEY: u32 = (137u32 << 16) | 42;
const YAW: u8 = 137;

/// Trees inside the NEAR ring at the measured p90, from `tests/tree.rs`.
/// Restated rather than imported because that suite's constant is private to
/// it; the budget arithmetic below is meaningless without it.
const RING_TREES_P90: usize = 328;

/// The outer ring has to be OUTSIDE the near one. Both are consts, so this is a
/// compile error rather than a test failure — and clippy is right that an
/// `assert!` over two constants is the weaker form of the same claim.
const _: () = assert!(
    OUTER_RADIUS > NEAR_RADIUS,
    "OUTER_RADIUS is inside NEAR_RADIUS — the outer ring would be a set of \
     chunks the near ring already owns"
);

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
    let maps = PropMaps {
        rock: MapSet::default(),
        bark: MapSet::default(),
        wood: MapSet::default(),
        stone: MapSet::default(),
        metal: MapSet::default(),
    };
    let a = assets(&mut meshes, &mut materials, &mut images, &maps);
    world.insert_resource(meshes);
    world.insert_resource(materials);
    world.insert_resource(images);
    (app, a)
}

fn spawned(app: &mut App, a: &PropAssets, s: &Slot, w: &WorldId) -> Vec<Entity> {
    let parent = app
        .world_mut()
        .spawn((Transform::IDENTITY, Visibility::default()))
        .id();
    {
        let mut commands = app.world_mut().commands();
        spawn_outer_tree(&mut commands, parent, a, s, KEY, w);
    }
    app.world_mut().flush();
    app.world()
        .entity(parent)
        .get::<Children>()
        .map(|c| c.iter().collect::<Vec<_>>())
        .unwrap_or_default()
}

/// **One entity, and it is the hull.**
///
/// Proven red two ways: calling `spawn_slot` here instead spawns four and
/// fails the count; swapping `a.impostor_mesh` for `a.pine_mesh` fails the
/// handle.
#[test]
fn an_outer_tree_is_one_hull_and_nothing_else() {
    let (mut app, a) = fixture();
    let w = WorldId::new(SEED);
    let s = Slot {
        occupant: Occupant::Tree,
        x: 512.0,
        y: 12.5,
        z: 704.0,
        yaw: YAW,
        scale: 1.05,
    };
    let kids = spawned(&mut app, &a, &s, &w);

    assert_eq!(
        kids.len(),
        1,
        "an outer tree spawned {} entities — the whole point of the ring is \
         that a tree past the swap distance is ONE hull, where a near tree is \
         four (trunk, canopy, hidden stump, hull)",
        kids.len()
    );

    let e = app.world().entity(kids[0]);
    let variant = YAW as usize % a.pine_variants();
    assert_eq!(
        e.get::<Mesh3d>().map(|m| m.0.clone()),
        Some(a.impostor_mesh(variant).clone()),
        "an outer tree is not carrying the impostor hull — if this is the near \
         bark mesh the frame looks identical at 300 m and costs 56× the \
         triangles"
    );

    // No band component: out here the swap distance is behind us by
    // construction, so a range could only ever hide the one mesh there is.
    assert!(
        e.get::<VisibilityRange>().is_none(),
        "an outer hull carries a VisibilityRange — there is no second LOD to \
         cross-fade to out here, so this can only cull the tree"
    );
    // No topple: `Vanish` never reads one, and a `Topple` here would be
    // iterated every frame forever to decide it is not animating.
    assert!(
        e.get::<Topple>().is_none(),
        "an outer hull carries a Topple it can never use"
    );

    // It DOES carry the harvested bit, and `Vanish` is the right part.
    let f = e
        .get::<Fellable>()
        .expect("an outer tree with no Fellable stands after it is cut");
    assert_eq!(f.key, KEY);
    assert!(
        matches!(f.part, FellPart::Vanish),
        "an outer hull must be FellPart::Vanish — Trunk means *speaks for this \
         slot* to the fell cue and the ambience cover count, and a second \
         speaker doubles both"
    );
}

/// **The tree stands on the ground that is drawn, not on the heightfield.**
///
/// The two differ by `FAR_DROP` plus the whole error of an 8 m linear chord
/// across real terrain. Proven red by planting at `slot.y`.
#[test]
fn an_outer_tree_stands_on_the_far_mesh() {
    let (mut app, a) = fixture();
    let w = WorldId::new(SEED);

    // Sweep the island and take the worst disagreement between the exact
    // heightfield and the surface the far mesh actually draws. That gap is the
    // error a naive `slot.y` would plant with, and the same sweep picks the
    // point the placement assert below runs at — see the note under it about
    // searching rather than hand-picking.
    //
    // ⚠ **This walked ONE hand-chosen line until 2026-08-26, and one line is
    // not the island** — `CLAUDE.md`'s own trap, twice paid for. It cost a
    // false red: worldgen shape v1 (`DECISIONS.md` §open) made `remap` C¹, and
    // an 8 m chord across a curve with no creases in it tracks the ground
    // *better*, so that line's worst fell 0.630 → 0.472 and tripped the
    // non-vacuity floor below — while the island's worst went the other way,
    // **2.416 → 2.939 m**, exactly as adding relief should. The line said the
    // gate had stopped measuring anything at the moment it started measuring
    // more. A 4 m sweep of the world square costs well under a second here and
    // leaves the floor six times clear instead of 6% clear.
    let (mut worst, mut px, mut pz) = (0.0f32, 0.0f32, 0.0f32);
    let mut sz = 4.0f32;
    while sz < terrain::ISLAND_SIZE {
        let mut sx = 4.0f32;
        while sx < terrain::ISLAND_SIZE {
            let exact = terrain::ground(w.seed, &w.haven, sx, sz);
            // Land only: a tree is not planted in the sea, so a disagreement
            // out there is not the error this gate is about.
            if exact > 0.5 {
                let d = (exact - far_ground_y(w.seed, &w.haven, sx, sz)).abs();
                if d > worst {
                    (worst, px, pz) = (d, sx, sz);
                }
            }
            sx += 4.0;
        }
        sz += 4.0;
    }
    assert!(
        worst > 0.5,
        "the far mesh and the heightfield differ by at most {worst:.3} m \
         anywhere on the island — if that is really under half a metre this \
         gate is measuring nothing and the placement fix is unnecessary"
    );

    // **Pick the test point by SEARCH, not by hand.** The first draft of this
    // gate hard-coded a coordinate, compared against `far_ground_y` while the
    // spawn writes `far_ground_y - SINK_M`, and used a 0.1 m tolerance — so it
    // passed on the correct code for the wrong reason (0.06 < 0.1) and PASSED
    // UNDER ITS OWN MUTANT too, because at that particular point the two
    // grounds happened to agree inside the slop. A test that shares a tolerance
    // with the effect it is measuring is measuring the tolerance.
    let gap = worst;
    assert!(
        gap > 0.4,
        "the most discriminating point on the island separates the two grounds \
         by only {gap:.3} m — the assert below would not be able to tell them \
         apart"
    );

    let s = Slot {
        occupant: Occupant::Tree,
        x: px,
        y: terrain::ground(w.seed, &w.haven, px, pz),
        z: pz,
        yaw: YAW,
        scale: 1.0,
    };
    let kids = spawned(&mut app, &a, &s, &w);
    let got = app
        .world()
        .entity(kids[0])
        .get::<Transform>()
        .copied()
        .expect("no transform")
        .translation
        .y;

    // EXACT, and against what the spawn actually writes — the drawn ground
    // minus the shared sink every prop takes.
    let want = far_ground_y(w.seed, &w.haven, s.x, s.z) - SINK_M;
    assert!(
        (got - want).abs() < 1e-4,
        "outer tree planted at y={got:.4}; the far mesh draws {:.4} there and \
         the sink is {SINK_M}, so it belongs at {want:.4}",
        want + SINK_M
    );
    // …and demonstrably NOT where the naive placement would have put it, which
    // is the half the first draft could not see.
    let naive = s.y - SINK_M;
    assert!(
        (got - naive).abs() > 0.4,
        "the spawned y {got:.4} is within {:.4} m of the heightfield placement \
         {naive:.4} — at this point the two grounds are {gap:.3} m apart, so \
         this gate cannot distinguish the fix from the bug",
        (got - naive).abs()
    );
}

/// `far_ground_y` agrees EXACTLY with the mesh's own vertices at the lattice
/// points — the only place the two can be compared without rebuilding it.
#[test]
fn the_far_height_matches_the_mesh_at_its_own_vertices() {
    let w = WorldId::new(SEED);
    for iz in 0..12 {
        for ix in 0..12 {
            let x = (ix as f32 + 40.0) * FAR_STEP;
            let z = (iz as f32 + 55.0) * FAR_STEP;
            // What `heightfield` writes into a vertex: the ground tap, dropped.
            let vertex = terrain::ground(w.seed, &w.haven, x, z) - FAR_DROP;
            let ours = far_ground_y(w.seed, &w.haven, x, z);
            assert!(
                (vertex - ours).abs() < 1e-4,
                "at a lattice corner ({x}, {z}) the mesh has {vertex:.5} and \
                 far_ground_y says {ours:.5} — these must be the same tap"
            );
        }
    }
}

/// **The budget.** The whole argument for the wider ring is that a hull is
/// cheap; this is that argument as arithmetic, so it goes red if either half
/// of it stops being true.
#[test]
fn the_outer_ring_fits_the_frame() {
    // Area scaling off the near ring's own measured tree count. The outer ring
    // is every chunk of the 11×11 block that the 5×5 does not already hold.
    let near_chunks = ((2 * NEAR_RADIUS + 1) * (2 * NEAR_RADIUS + 1)) as usize;
    assert_eq!(
        OUTER_CHUNKS,
        ((2 * OUTER_RADIUS + 1) * (2 * OUTER_RADIUS + 1)) as usize - near_chunks
    );
    let outer_trees = RING_TREES_P90 * OUTER_CHUNKS / near_chunks;
    let outer_tris = outer_trees * IMPOSTOR_MAX_TRIS;

    // `DESIGN.md` §9's whole-frame ceiling, and the near ring's own share of
    // it after the LOD landed (`RENDER.md`: 1.94 M → 510 k).
    const FRAME_TRIS: usize = 1_500_000;
    const NEAR_TREE_TRIS: usize = 510_000;
    assert!(
        outer_tris + NEAR_TREE_TRIS < FRAME_TRIS / 2,
        "the outer ring adds {outer_trees} trees at {IMPOSTOR_MAX_TRIS} tris = \
         {outer_tris}, and with the near ring's {NEAR_TREE_TRIS} that is more \
         than half the frame's {FRAME_TRIS} — trees are not the only thing in \
         a frame, so half is the share this ring may take"
    );

    // One entity each, which is what makes the count affordable at all.
    assert!(
        outer_trees < 2_000,
        "{outer_trees} outer trees is {outer_trees} entities Bevy sweeps for \
         visibility every frame; past ~2,000 this wants instancing rather than \
         a wider radius"
    );
}
