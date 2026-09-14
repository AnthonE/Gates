//! Gate: when the player walks into a chunk, the outer ring's hulls leave it.
//!
//! **The prop streamer is two rings with two owners, and every gate on it
//! stood still.** `props::stream` keeps a 5×5 near ring of full-detail trees
//! (trunk, canopy, hidden stump, and a hull `VisibilityRange` opens past the
//! swap distance) inside a wider outer ring of hull-only trees that carry
//! no range at all — out there nothing is ever shown in a hull's place, so
//! it is simply visible. That is correct exactly as long as no chunk is in
//! both rings.
//!
//! From the outer ring's landing until 2026-09-13 the outer ring's retain
//! asked one question — *is this chunk still inside my radius* — which every
//! chunk the player walks INTO answers yes. So the near ring built the real
//! tree on top and the outer hull stayed: an opaque faceted dome drawn over
//! every tree in every chunk entered after spawn, at any distance, on both
//! targets. `tests/tree_lod.rs` (one spawn), `tests/outer_ring.rs` (one outer
//! tree), `tests/tree_swap.rs` (the browser's swap) and the `--capture` probe
//! were all green over it — the probe stands where it spawned, and at spawn
//! the near ring is built first and the outer ring skips its keys, so no
//! frame from there could contain the defect. The operator, who had walked
//! to a base, was the first to see it, and the first reading of that frame
//! blamed the browser's swap distance and moved a knob. A hull at arm's
//! length is inside any swap distance.
//!
//! So this suite does the one thing the others did not: it drives the REAL
//! `props::stream` with an eye that moves three chunks, lets both rings
//! settle on the one-chunk-per-frame budget, and asserts that no chunk is
//! held by both rings and no cell carries two hulls. Proven red under the
//! old retain: 15 chunks in both rings, every tree in them doubled.
//!
//! Headless: `MinimalPlugins` plus the asset plugin, the fixture
//! `tests/tree_lod.rs` and `tests/outer_ring.rs` share. The asset server is
//! asked for files this tier does not have and hands back handles that never
//! resolve, which is what every other fixture in this directory relies on.

#![cfg(feature = "render")]

use std::collections::HashMap;

use bevy::asset::AssetPlugin;
use bevy::prelude::*;
use client::render::props::{self, FellPart, Fellable, PropRing, OUTER_CHUNKS, OUTER_RADIUS};
use client::render::terrain_mesh::{CHUNK_M, NEAR_RADIUS};
use client::render::textures::{MapSet, PropMaps};
use client::render::tree::TreeLod;
use client::render::{Eye, WorldId};

/// The island the shard ships and every frame this project judges is shot on.
const SEED: u64 = 20260731;

/// Chunks in a full near ring.
const NEAR_CHUNKS: usize = ((2 * NEAR_RADIUS + 1) * (2 * NEAR_RADIUS + 1)) as usize;

/// Frames both rings are given to settle after the eye moves. The streamer
/// builds or drops at most one chunk per frame, so a full re-centre is a
/// bounded number of frames; this is that bound with room, and a frame count
/// rather than a clock for the reason every budget in this crate is one.
const SETTLE_FRAMES: usize = 600;

fn app_at(x_m: f32, z_m: f32) -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()));
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
    app.add_systems(Update, props::stream);
    app
}

fn move_eye(app: &mut App, x_m: f32, z_m: f32) {
    app.world_mut().resource_mut::<Eye>().pos = Vec3::new(x_m, 20.0, z_m);
}

/// Run frames until both rings hold exactly the chunks the eye's position
/// says they should, or the budget is spent.
///
/// **By KEY, not by count.** The first draft returned on `len() == 25 &&
/// outer_len() == 96`, and under the old retain that was satisfied one frame
/// into the walk: the near ring drops one chunk and builds one per frame, so
/// its count never moves, and the outer ring's count does not move until its
/// own retain starts. The mutant then read 2 overlapping chunks where the
/// defect was 15 — the harness had certified a ring that held the right
/// NUMBER of the wrong chunks. `CLAUDE.md`'s relief.rs entry is the same
/// shape: consistency proves the method is constant, never that it is aimed
/// at the right thing.
fn settle(app: &mut App) -> usize {
    for frame in 0..SETTLE_FRAMES {
        app.update();
        if rings_match_eye(app) {
            // One more, so a retain that fires after the last build has run.
            app.update();
            return frame + 2;
        }
    }
    SETTLE_FRAMES
}

/// Both rings hold exactly the chunk keys the eye's chunk implies: every key
/// inside the near radius is near and not outer, every key in the annulus is
/// outer and not near, and nothing else is held.
fn rings_match_eye(app: &App) -> bool {
    let eye = app.world().resource::<Eye>().pos;
    let (cx, cz) = chunk_of(eye.x, eye.z);
    let ring = app.world().resource::<PropRing>();
    if ring.len() != NEAR_CHUNKS || ring.outer_len() != OUTER_CHUNKS {
        return false;
    }
    for dz in -OUTER_RADIUS..=OUTER_RADIUS {
        for dx in -OUTER_RADIUS..=OUTER_RADIUS {
            let key = (cx + dx, cz + dz);
            let near = dx.abs() <= NEAR_RADIUS && dz.abs() <= NEAR_RADIUS;
            if near != ring.near_holds(key) || near == ring.outer_holds(key) {
                return false;
            }
        }
    }
    true
}

/// Every cell key that has a hull from the near ring (`Far`) AND a hull from
/// the outer ring (`Vanish` on a tree). A node or a bush is `Vanish` too, but
/// only the near ring spawns those and it never spawns a `Far` for them, so a
/// key carrying both parts can only be a tree drawn by both rings.
fn doubled(app: &mut App) -> Vec<u32> {
    let mut far: HashMap<u32, usize> = HashMap::new();
    let mut vanish: HashMap<u32, usize> = HashMap::new();
    let mut q = app.world_mut().query::<&Fellable>();
    for f in q.iter(app.world()) {
        match f.part {
            FellPart::Far => *far.entry(f.key).or_default() += 1,
            FellPart::Vanish => *vanish.entry(f.key).or_default() += 1,
            _ => {}
        }
    }
    let mut keys: Vec<u32> = far
        .keys()
        .filter(|k| vanish.contains_key(k))
        .copied()
        .collect();
    keys.sort_unstable();
    keys
}

fn chunk_of(x_m: f32, z_m: f32) -> (i32, i32) {
    (
        (x_m / CHUNK_M).floor() as i32,
        (z_m / CHUNK_M).floor() as i32,
    )
}

// ── The walk ────────────────────────────────────────────────────────────────

/// Stand, let both rings fill, walk three chunks east, let them settle: no
/// chunk in both rings, no tree with two hulls.
///
/// Three chunks is the distance that makes the defect largest — every column
/// the near ring gains was the outer ring's — and it is a stroll: 192 m, the
/// walk from any spawn to the first place a player would put a foundation.
#[test]
fn walking_into_the_treeline_takes_the_hulls_down() {
    // The island's middle, so both rings are entirely on land and full of
    // trees; the exact spot is not the claim.
    let (x0, z0) = (1024.0, 1024.0);
    let mut app = app_at(x0, z0);
    let frames = settle(&mut app);
    {
        let ring = app.world().resource::<PropRing>();
        assert_eq!(
            ring.len(),
            NEAR_CHUNKS,
            "near ring did not fill in {frames} frames"
        );
        assert_eq!(
            ring.outer_len(),
            OUTER_CHUNKS,
            "outer ring did not fill in {frames} frames"
        );
        assert_eq!(
            ring.overlap(),
            0,
            "the rings overlap before anyone has moved"
        );
    }
    assert!(
        doubled(&mut app).is_empty(),
        "a tree has two hulls before anyone has moved"
    );

    // Walk three chunks east. The near ring's new eastern columns were the
    // outer ring's; the outer ring has to let them go.
    let x1 = x0 + 3.0 * CHUNK_M;
    move_eye(&mut app, x1, z0);
    let frames = settle(&mut app);
    let ring = app.world().resource::<PropRing>();
    assert_eq!(
        ring.len(),
        NEAR_CHUNKS,
        "near ring did not re-fill in {frames} frames"
    );
    assert_eq!(
        ring.outer_len(),
        OUTER_CHUNKS,
        "outer ring did not re-fill in {frames} frames"
    );
    // The two summary claims first, so a failure names the size of the
    // defect rather than the first chunk it happened to visit.
    let overlap = ring.overlap();
    assert_eq!(
        overlap, 0,
        "{overlap} chunk(s) are held by both rings after a 3-chunk walk — the \
         outer ring kept the hulls in chunks the near ring now owns"
    );
    let (cx, cz) = chunk_of(x1, z0);
    // The column the eye now stands in was three chunks outside the old near
    // ring: it is near now, and it must not be outer as well.
    for dz in -NEAR_RADIUS..=NEAR_RADIUS {
        let key = (cx, cz + dz);
        assert!(
            ring.near_holds(key),
            "chunk {key:?} under the eye is not near"
        );
        assert!(
            !ring.outer_holds(key),
            "chunk {key:?} under the eye is still outer"
        );
    }
    let twice = doubled(&mut app);
    assert!(
        twice.is_empty(),
        "{} tree(s) carry a near hull and an outer hull at once (cells {:?}…) — \
         the picture is an opaque dome over every one of them",
        twice.len(),
        &twice[..twice.len().min(6)]
    );
}

/// Walk back: the chunks the near ring gives up become the outer ring's
/// again, and only the outer ring's.
#[test]
fn walking_back_out_hands_the_chunks_over_the_other_way() {
    let (x0, z0) = (1024.0, 1024.0);
    let mut app = app_at(x0, z0);
    settle(&mut app);
    let x1 = x0 + 3.0 * CHUNK_M;
    move_eye(&mut app, x1, z0);
    settle(&mut app);
    move_eye(&mut app, x0, z0);
    let frames = settle(&mut app);
    let ring = app.world().resource::<PropRing>();
    assert_eq!(
        ring.len(),
        NEAR_CHUNKS,
        "near ring did not re-fill in {frames} frames"
    );
    assert_eq!(
        ring.outer_len(),
        OUTER_CHUNKS,
        "outer ring did not re-fill in {frames} frames"
    );
    assert_eq!(ring.overlap(), 0);
    // The column the eye stood in during the walk is outer again and not near.
    let (cx, cz) = chunk_of(x1, z0);
    for dz in -NEAR_RADIUS..=NEAR_RADIUS {
        let key = (cx, cz + dz);
        assert!(
            !ring.near_holds(key),
            "chunk {key:?} left behind is still near"
        );
        assert!(
            ring.outer_holds(key),
            "chunk {key:?} left behind is not outer"
        );
    }
    assert!(doubled(&mut app).is_empty());
}

/// The outer ring never holds a chunk inside the near radius, on any frame of
/// the walk — not only once settled. The hand-off happens the frame the near
/// chunk is built, so there is no frame in which a player sees both.
#[test]
fn no_frame_of_the_walk_shows_both() {
    let (x0, z0) = (1024.0, 1024.0);
    let mut app = app_at(x0, z0);
    settle(&mut app);
    // One chunk per step, the way a walk actually crosses chunk edges.
    for step in 1..=3 {
        move_eye(&mut app, x0 + step as f32 * CHUNK_M, z0);
        for frame in 0..SETTLE_FRAMES {
            app.update();
            let ring = app.world().resource::<PropRing>();
            assert_eq!(
                ring.overlap(),
                0,
                "step {step}, frame {frame}: a chunk is in both rings"
            );
            if ring.len() == NEAR_CHUNKS && ring.outer_len() == OUTER_CHUNKS {
                break;
            }
        }
    }
    assert!(doubled(&mut app).is_empty());
}
