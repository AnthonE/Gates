//! Gate: the tree count cap — `reference/FORESTS.md` §8 gate 7, "the frame
//! budget as a cap, not a print".
//!
//! `TREE_LOD_SWAP_M` bounds a DISTANCE, and what a distance holds is the
//! forest's business: forest density v1 took the Forest biome from ~39 to
//! ~94 stems/ha (stands at ~134), so the 80 m disc plus its fade holds up to
//! ~360 trees at 5,900 triangles each — 2.1 M before a hull, against a 1.5 M
//! frame. `tree::cap_swap` is the answer: when more than `TREE_LOD_CAP` trees
//! would draw their near pair it pulls the swap in to the distance that holds
//! exactly that many, and lets it back out to the tier when fewer would.
//!
//! Five things a wrong version does silently, because a wrong band is only
//! a frame that costs more or a hull that stands closer:
//!
//!  1. in a stand denser than the cap, the drawn set is exactly the cap;
//!  2. it is the NEAREST trees that keep their pair, not any other set;
//!  3. where fewer trees than the cap are in reach, the tier's distance is
//!     untouched — the cap is not a second, lower swap;
//!  4. the bands go back OUT when the eye leaves the stand, and the tier
//!     survives a contraction (it is what the bands go back out to);
//!  5. the swap has a floor at any density this grid can produce.
//!
//! Then the real streamer: the shipped island's densest eye (from
//! `sim-core/examples/ring_census.rs`), through `props::stream`,
//! `cap_swap` and `quality::reband_trees` together, with the cap proven to
//! bind there and every near part carrying the bands the cap wrote.
//!
//! Headless — `MinimalPlugins`, no GPU. The synthetic half spawns
//! `GlobalTransform`s directly (the system reads them, as `swap_by_distance`
//! does); the real half adds `TransformPlugin` so streamed parts get theirs.

#![cfg(feature = "render")]

use bevy::asset::AssetPlugin;
use bevy::prelude::*;

use client::render::props::{self, FellPart, Fellable, PropRing};
use client::render::quality;
use client::render::textures::{MapSet, PropMaps};
use client::render::tree::{
    self, TreeLod, TREE_LOD_CAP, TREE_LOD_CAP_STEP_M, TREE_LOD_FADE_M, TREE_LOD_SWAP_M,
};
use client::render::{Eye, WorldId};
use sim_core::terrain::CELL_SIZE;

/// The island the shard ships and every frame this project judges is shot on.
const SEED: u64 = 20260731;

/// The densest eye on that island at the 80 m swap, from
/// `cargo run --release -p sim-core --example ring_census`: 246 trees inside
/// 80 m, 357 inside the fade. Over the cap by a wide margin, which is what
/// makes it the fixture and not a pin.
const DENSEST_EYE: (f32, f32) = (676.0, 292.0);

/// Frames the near ring is given to build around a placed eye — one chunk a
/// frame, 25 chunks, with room. A count, not a clock.
const SETTLE_FRAMES: usize = 200;

fn synthetic(eye: Vec3) -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.init_resource::<TreeLod>();
    app.insert_resource(Eye {
        pos: eye,
        placed: true,
        ..default()
    });
    app.add_systems(Update, tree::cap_swap);
    app
}

fn trunk_at(app: &mut App, x: f32, z: f32) -> Entity {
    app.world_mut()
        .spawn((
            Fellable {
                key: 1,
                variant: 0,
                base_y: 0.0,
                yaw: 0.0,
                part: FellPart::Trunk,
                felled: false,
            },
            GlobalTransform::from_xyz(x, 0.0, z),
            Visibility::Inherited,
        ))
        .id()
}

/// A square lattice of trunks around the origin at `pitch` metres, `n` on a
/// side — a stand of known density, so the expected distance of the cap's
/// edge is arithmetic.
fn stand(app: &mut App, n: i32, pitch: f32) -> Vec<(Entity, f32)> {
    let mut out = Vec::new();
    for iz in -n..=n {
        for ix in -n..=n {
            let (x, z) = (ix as f32 * pitch, iz as f32 * pitch);
            let e = trunk_at(app, x, z);
            out.push((e, (x * x + z * z).sqrt()));
        }
    }
    out
}

fn swap(app: &App) -> f32 {
    app.world().resource::<TreeLod>().swap_m()
}

/// How many of the given trunks would draw any near geometry under the
/// current bands: inside the swap plus the fade.
fn drawn(app: &App, trees: &[(Entity, f32)]) -> usize {
    let reach = swap(app) + TREE_LOD_FADE_M;
    trees.iter().filter(|(_, d)| *d < reach).count()
}

// ── 1 + 2. A stand over the cap draws exactly the cap, nearest first ────────

#[test]
fn a_stand_over_the_cap_draws_exactly_the_cap_and_the_nearest() {
    let mut app = synthetic(Vec3::ZERO);
    // 8 m pitch is the grid's own ceiling (one tree a cell); 41² = 1,681
    // trunks, ~800 inside the 95 m reach — four times the cap.
    let mut trees = stand(&mut app, 20, CELL_SIZE);
    app.update();

    let s = swap(&app);
    assert!(
        s < TREE_LOD_SWAP_M,
        "the cap did not bind: the swap is still the tier's {TREE_LOD_SWAP_M} m in a \
         stand that puts ~800 trees inside the reach"
    );
    let n = drawn(&app, &trees);
    assert!(
        n <= TREE_LOD_CAP,
        "{n} trees draw their near pair under a swap of {s} m — over TREE_LOD_CAP \
         {TREE_LOD_CAP}"
    );
    // Not merely under: the cap is where the swap LANDS, so the drawn set is
    // the cap itself, less the step's quantization (which can only drop a
    // ring of trees, never admit one).
    let step_ring = ((s + TREE_LOD_FADE_M + TREE_LOD_CAP_STEP_M).powi(2)
        - (s + TREE_LOD_FADE_M).powi(2))
        * std::f32::consts::PI
        / (CELL_SIZE * CELL_SIZE);
    assert!(
        n as f32 >= TREE_LOD_CAP as f32 - step_ring - 1.0,
        "only {n} trees draw under a swap of {s} m — the cap is {TREE_LOD_CAP} and the \
         step can drop at most ~{step_ring:.0} of them, so the swap landed too low"
    );

    // The NEAREST keep their pair: every drawn tree is nearer than every
    // undrawn one. Sort by distance; the first `n` must be the drawn set.
    trees.sort_by(|a, b| a.1.total_cmp(&b.1));
    let reach = s + TREE_LOD_FADE_M;
    for (i, (_, d)) in trees.iter().enumerate() {
        assert_eq!(
            *d < reach,
            i < n,
            "tree {i} at {d:.2} m is {} under a reach of {reach} m, but the {n} \
             nearest are the drawn set",
            if *d < reach { "drawn" } else { "not drawn" }
        );
    }
}

// ── 3. Under the cap, the tier's distance is untouched ──────────────────────

#[test]
fn a_stand_under_the_cap_keeps_the_tiers_swap() {
    let mut app = synthetic(Vec3::ZERO);
    // 16 m pitch: 13² = 169 trunks in total, all of them inside the reach and
    // fewer than the cap.
    let trees = stand(&mut app, 6, 16.0);
    assert!(trees.len() < TREE_LOD_CAP);
    let lod_before = app.world().resource::<TreeLod>().clone();
    app.update();
    assert!(
        (swap(&app) - TREE_LOD_SWAP_M).abs() < 1e-6,
        "the swap moved to {} m with only {} trees in reach — the cap is not a \
         second, lower swap",
        swap(&app),
        trees.len()
    );
    assert!(
        *app.world().resource::<TreeLod>() == lod_before,
        "the resource was rewritten with nothing to change — every write costs a \
         reband of the whole ring"
    );
}

// ── 4. Out again when the stand is left, and the tier survives ──────────────

#[test]
fn the_swap_goes_back_out_when_the_eye_leaves_the_stand() {
    let mut app = synthetic(Vec3::ZERO);
    stand(&mut app, 20, CELL_SIZE);
    app.update();
    let contracted = swap(&app);
    assert!(
        contracted < TREE_LOD_SWAP_M,
        "fixture: the cap must bind first"
    );
    assert!(
        (app.world().resource::<TreeLod>().tier_m - TREE_LOD_SWAP_M).abs() < 1e-6,
        "the tier moved with the bands — it is what the bands go back out to"
    );

    // Walk out of the stand: 41 × 8 m is 328 m across, so 400 m away is
    // clear of it and of the reach.
    app.world_mut().resource_mut::<Eye>().pos = Vec3::new(400.0, 0.0, 0.0);
    app.update();
    assert!(
        (swap(&app) - TREE_LOD_SWAP_M).abs() < 1e-6,
        "out of the stand the swap is {} m, not the tier's {TREE_LOD_SWAP_M}",
        swap(&app)
    );

    // And a tier change under a contraction resets to the new tier, which
    // the cap then pulls in again if the stand still needs it.
    app.world_mut().resource_mut::<Eye>().pos = Vec3::ZERO;
    app.update();
    assert!(swap(&app) < TREE_LOD_SWAP_M);
    *app.world_mut().resource_mut::<TreeLod>() = TreeLod::at(55.0);
    app.update();
    let lod = app.world().resource::<TreeLod>();
    assert!((lod.tier_m - 55.0).abs() < 1e-6);
    assert!(
        lod.swap_m() <= 55.0,
        "the bands sit at {} m, above a tier of 55 m",
        lod.swap_m()
    );
}

// ── 5. The floor, at any density this grid can produce ──────────────────────

#[test]
fn the_swap_has_a_floor_at_the_grids_own_ceiling() {
    // One tree a cell, every cell — 156.25 stems/ha, which nothing on this
    // grid exceeds. The cap fills at the radius holding TREE_LOD_CAP of them.
    let expect = (TREE_LOD_CAP as f32 * CELL_SIZE * CELL_SIZE / std::f32::consts::PI).sqrt()
        - TREE_LOD_FADE_M;
    let mut app = synthetic(Vec3::ZERO);
    stand(&mut app, 20, CELL_SIZE);
    app.update();
    let s = swap(&app);
    println!("at the grid's ceiling the swap lands at {s} m (arithmetic {expect:.1} m)");
    assert!(
        (s - expect).abs() <= TREE_LOD_CAP_STEP_M + CELL_SIZE * 0.5,
        "the swap landed at {s} m; a stand of one tree a cell holds the cap by \
         {expect:.1} m"
    );
    // The number that matters to a player: never a hull inside this. 44 m
    // by the arithmetic above at the grid's absolute ceiling, which no stand
    // the shipped field makes reaches (those sit at ~134/ha, ~50 m).
    assert!(
        s >= 40.0,
        "the floor is {s} m — a 14 m tree becomes a hull inside 40 m, which is \
         under the distance `NOW.md` §0t's hull item was written against"
    );
}

// ── The real streamer, at the island's densest eye ──────────────────────────

fn real_at(x_m: f32, z_m: f32) -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin::default(),
        bevy::transform::TransformPlugin,
    ));
    app.init_asset::<Mesh>();
    app.init_asset::<StandardMaterial>();
    app.init_asset::<Image>();
    app.insert_resource(PropMaps {
        rock: MapSet::default(),
        bark: MapSet::default(),
        wood: MapSet::default(),
        stone: MapSet::default(),
        metal: MapSet::default(),
    });
    app.insert_resource(WorldId::new(SEED));
    app.init_resource::<TreeLod>();
    app.init_resource::<PropRing>();
    app.insert_resource(Eye {
        pos: Vec3::new(x_m, 20.0, z_m),
        placed: true,
        ..default()
    });
    // The shipped order: stream, then the cap, then the reband that applies
    // what the cap wrote (`render/mod.rs`).
    app.add_systems(
        Update,
        (
            props::stream,
            tree::cap_swap.after(props::stream),
            quality::reband_trees.after(tree::cap_swap),
        ),
    );
    app
}

#[test]
fn the_shipped_islands_densest_eye_is_held_to_the_cap_by_the_real_streamer() {
    let mut app = real_at(DENSEST_EYE.0, DENSEST_EYE.1);
    for _ in 0..SETTLE_FRAMES {
        app.update();
    }
    let eye = app.world().resource::<Eye>().pos;
    let lod = app.world().resource::<TreeLod>().clone();
    let reach = lod.swap_m() + TREE_LOD_FADE_M;

    // Count what the census counted, off the streamed trunks.
    let mut trunks = 0usize;
    let mut in_tier_reach = 0usize;
    let mut drawn = 0usize;
    let tier_reach = TREE_LOD_SWAP_M + TREE_LOD_FADE_M;
    let mut q = app.world_mut().query::<(
        &Fellable,
        &GlobalTransform,
        Option<&bevy::camera::visibility::VisibilityRange>,
    )>();
    for (f, gt, range) in q.iter(app.world()) {
        if f.part != FellPart::Trunk {
            continue;
        }
        trunks += 1;
        let d = (gt.translation() - eye).length();
        in_tier_reach += usize::from(d < tier_reach);
        drawn += usize::from(d < reach);
        // Every near part carries the bands the cap wrote — `reband_trees`
        // applied the contraction to the trees already standing.
        let range = range.expect("a trunk carries a VisibilityRange on the desktop");
        assert_eq!(
            range.end_margin.start,
            lod.swap_m(),
            "a trunk's band starts its fade at {} m while the cap put the swap at {} m",
            range.end_margin.start,
            lod.swap_m()
        );
    }
    println!(
        "densest eye: {trunks} trunks in the ring, {in_tier_reach} inside the tier's \
         {tier_reach} m, {drawn} drawn under the cap's {} m swap",
        lod.swap_m()
    );
    // The fixture is what the census said it is, or this gate is measuring
    // the wrong place: over the cap at the tier's own distance.
    assert!(
        in_tier_reach > TREE_LOD_CAP,
        "only {in_tier_reach} trunks inside the tier's reach at the eye the census \
         called densest — re-run `ring_census` and move DENSEST_EYE"
    );
    assert!(
        lod.swap_m() < TREE_LOD_SWAP_M,
        "the cap did not bind on the real streamer at the island's densest eye"
    );
    assert!(
        drawn <= TREE_LOD_CAP,
        "{drawn} trees draw their near pair at the densest eye — over TREE_LOD_CAP \
         {TREE_LOD_CAP}"
    );
    assert!(
        lod.swap_m() >= 40.0,
        "the swap landed at {} m on the real island — under the 40 m floor",
        lod.swap_m()
    );
}
