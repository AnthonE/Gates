//! `frame_cost` — what the client's streaming frame spends, on the CPU.
//!
//! Not a gate and never will be: it reports elapsed time, and a gate that
//! waits on a clock is not a gate on this box (`CLAUDE.md`). It is the
//! instrument behind `findings/client-frame-20260819.md`, and it exists
//! because that note's one rule is that every claim carries the command that
//! produced it — a number nobody can re-run is prose.
//!
//! ```text
//! cargo run --release -p client --features render --example frame_cost
//! ```
//!
//! Four things, in the order a frame meets them:
//!
//!  - **`terrain::height`**, plain and through a `terrain::Lattice`. The plain
//!    form is deliberately unchanged by the memo — every public entry point
//!    still hashes every corner every time, and the memo is a second entry
//!    point a caller opts into — so a run where these two are equal means the
//!    memo is not being reached, not that it is free. Then again on a range
//!    flank, where stage 4d (the interior ranges) is paid.
//!  - **`heightfield`**, both rings. The far mesh is the one that used to be a
//!    ~190 ms frame; it runs on `AsyncComputeTaskPool` now, so what this
//!    prints is the pool's cost rather than the frame's.
//!  - **`clutter_fill` + `skirt_fill`**, one tile and one ring. The client
//!    fills one tile a frame, so the per-tile row is the frame's. Also on a
//!    range summit tile and a range flank tile.
//!  - **`water::stream`**, as a system on a bare `App`: a cold sweep (nothing
//!    to carry) against a one-cell snap (most of the core carried).
//!
//! Medians are not reported — the MINIMUM of several runs is, which is the
//! honest statistic on a box that shares cores with a build: the fastest run
//! is the one least interrupted, and a mean here measures the neighbours.

use std::hint::black_box;
use std::time::Instant;

use bevy::asset::AssetPlugin;
use bevy::prelude::*;

use client::render::terrain_mesh::{self, CHUNK_M, FAR_DROP, FAR_N, FAR_STEP, NEAR_N};
use client::render::water::{self, Sea, SNAP_M};
use client::render::{Eye, WorldId};
use sim_core::terrain::{self, ScatterTable, CLUTTER_NONE, CLUTTER_PER_TILE, SKIRT_PER_TILE};

const SEED: u64 = 20_260_731;

fn best(reps: u32, mut f: impl FnMut()) -> f64 {
    f();
    let mut b = f64::MAX;
    for _ in 0..reps {
        let t = Instant::now();
        f();
        b = b.min(t.elapsed().as_nanos() as f64);
    }
    b
}

fn row(label: &str, ns: f64) {
    println!("  {label:<46} {:9.3} ms", ns / 1e6);
}

fn tile_origin(t: (i32, i32)) -> (f32, f32) {
    (
        t.0 as f32 * terrain::CLUTTER_TILE_M,
        t.1 as f32 * terrain::CLUTTER_TILE_M,
    )
}

/// The clutter tiles the timings stand on: the range's highest summit, and
/// the flank tile whose lift is nearest 20 m. Scanned at tile centres.
fn range_tiles() -> ((i32, i32), (i32, i32)) {
    let n = (2048.0 / terrain::CLUTTER_TILE_M) as i32;
    let (mut summit, mut top) = ((0, 0), f32::MIN);
    let (mut flank, mut miss) = ((0, 0), f32::MAX);
    for tz in 0..n {
        for tx in 0..n {
            let (ox, oz) = tile_origin((tx, tz));
            let lift = terrain::massif_lift_at(SEED, ox + 8.0, oz + 8.0);
            if lift > top {
                (summit, top) = ((tx, tz), lift);
            }
            if lift > 0.0 && (lift - 20.0).abs() < miss {
                (flank, miss) = ((tx, tz), (lift - 20.0).abs());
            }
        }
    }
    (summit, flank)
}

fn main() {
    println!("frame_cost — release, minimum of several runs, CPU only\n");
    let haven = terrain::haven(SEED);
    let table = ScatterTable::alpha_default();

    // ── terrain::height, with and without a memo ──────────────────────────
    println!("terrain::height, 100k taps on a walking line");
    let plain = best(4, || {
        let mut a = 0.0f32;
        let mut c = 0.0f32;
        for _ in 0..100_000 {
            c += 0.001;
            a += terrain::height(SEED, 1024.0 + c, 1024.0);
        }
        black_box(a);
    });
    let memo = best(4, || {
        let mut lat = terrain::Lattice::new();
        let mut a = 0.0f32;
        let mut c = 0.0f32;
        for _ in 0..100_000 {
            c += 0.001;
            a += terrain::height_memo(&mut lat, SEED, 1024.0 + c, 1024.0);
        }
        black_box(a);
    });
    row("plain (unchanged on purpose)", plain);
    row("through a Lattice", memo);
    println!("  {:<46} {:9.2}x\n", "ratio", plain / memo);

    // ── the same, on the interior ranges ──────────────────────────────────
    //
    // The walk above sits at the island centre, inside `MASSIF_R_IN`, so it
    // never pays stage 4d. Two tiles are found rather than written down: the
    // highest summit, and a flank (lift nearest 20 m), which pays the ridged
    // term AND the gully filter.
    let (summit, flank) = range_tiles();
    let (fx, fz) = tile_origin(flank);
    println!("terrain::height on a range flank, 100k taps from ({fx:.0}, {fz:.0})");
    let plain_r = best(4, || {
        let mut a = 0.0f32;
        let mut c = 0.0f32;
        for _ in 0..100_000 {
            c += 0.001;
            a += terrain::height(SEED, fx + c, fz);
        }
        black_box(a);
    });
    let memo_r = best(4, || {
        let mut lat = terrain::Lattice::new();
        let mut a = 0.0f32;
        let mut c = 0.0f32;
        for _ in 0..100_000 {
            c += 0.001;
            a += terrain::height_memo(&mut lat, SEED, fx + c, fz);
        }
        black_box(a);
    });
    row("plain", plain_r);
    row("through a Lattice", memo_r);
    println!(
        "  {:<46} {:9.2}x\n",
        "range / centre, through a Lattice",
        memo_r / memo
    );

    // ── the ground mesh ───────────────────────────────────────────────────
    println!("terrain_mesh::heightfield — now on AsyncComputeTaskPool");
    let step = CHUNK_M / (NEAR_N - 1) as f32;
    row(
        "near chunk 65^2 @1 m (one per streaming frame)",
        best(6, || {
            black_box(terrain_mesh::heightfield(
                SEED, &haven, 1024.0, 1024.0, NEAR_N, step, 0.0,
            ));
        }),
    );
    row(
        "far mesh 257^2 @8 m (once, at load)",
        best(2, || {
            black_box(terrain_mesh::heightfield(
                SEED, &haven, 0.0, 0.0, FAR_N, FAR_STEP, FAR_DROP,
            ));
        }),
    );
    println!();

    // ── the ground population ─────────────────────────────────────────────
    println!("sim_core::terrain — the clutter ring");
    let mut buf = vec![CLUTTER_NONE; CLUTTER_PER_TILE];
    let mut sbuf = vec![CLUTTER_NONE; SKIRT_PER_TILE];
    let mut tbuf = vec![CLUTTER_NONE; terrain::CLUTTER_TILE_CAP];
    let tx = (1024.0 / terrain::CLUTTER_TILE_M) as i32;
    let n = terrain::clutter_fill(SEED, &haven, tx, tx, &mut buf);
    println!("  (tile ({tx},{tx}) holds {n} elements)");
    row(
        "clutter_fill, one tile (one per frame)",
        best(20, || {
            black_box(terrain::clutter_fill(SEED, &haven, tx, tx, &mut buf));
        }),
    );
    row(
        "skirt_fill, one tile",
        best(60, || {
            black_box(terrain::skirt_fill(SEED, &table, &haven, tx, tx, &mut sbuf));
        }),
    );
    row(
        "both, over the whole 5x5 ring",
        best(4, || {
            for j in -2..=2i32 {
                for i in -2..=2i32 {
                    black_box(terrain::clutter_fill(
                        SEED,
                        &haven,
                        tx + i,
                        tx + j,
                        &mut buf,
                    ));
                    black_box(terrain::skirt_fill(
                        SEED,
                        &table,
                        &haven,
                        tx + i,
                        tx + j,
                        &mut sbuf,
                    ));
                }
            }
        }),
    );
    for (label, t) in [("summit", summit), ("flank", flank)] {
        let (ox, oz) = tile_origin(t);
        let lift = terrain::massif_lift_at(SEED, ox + 8.0, oz + 8.0);
        println!(
            "  (range {label} tile ({},{}) at ({ox:.0}, {oz:.0}), lift {lift:.1} m)",
            t.0, t.1
        );
        row(
            &format!("clutter_fill, one {label} tile"),
            best(20, || {
                black_box(terrain::clutter_fill(SEED, &haven, t.0, t.1, &mut buf));
            }),
        );
        row(
            &format!("skirt_fill, one {label} tile"),
            best(60, || {
                black_box(terrain::skirt_fill(
                    SEED, &table, &haven, t.0, t.1, &mut sbuf,
                ));
            }),
        );
        row(
            &format!("clutter_tile_fill_memo, one {label} tile (the client's)"),
            best(20, || {
                let mut lat = terrain::Lattice::new();
                black_box(terrain::clutter_tile_fill_memo(
                    &mut lat, SEED, &table, &haven, t.0, t.1, &mut tbuf,
                ));
            }),
        );
    }
    println!();

    // ── the sea, as a system ──────────────────────────────────────────────
    //
    // A shoreline eye, found rather than written down: `shore` is the cache
    // that costs a `terrain::slope` and it only exists in a band around the
    // coast, so an inland run measures the cheap half of the sweep.
    let mut sx = 1024.0f32;
    while sx < 2048.0 && terrain::ground(SEED, &haven, sx, 1024.0) >= 0.0 {
        sx += 4.0;
    }
    let eye0 = Vec3::new(sx - 8.0, 2.0, 1024.0);
    println!("water::stream — a bare App at a shoreline eye ({sx:.0} m)");

    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()));
    app.init_asset::<Mesh>();
    app.init_asset::<Image>();
    app.init_asset::<StandardMaterial>();
    app.insert_resource(WorldId::new(SEED));
    app.init_resource::<Sea>();
    app.insert_resource(Eye {
        pos: eye0,
        ..default()
    });
    app.add_systems(Startup, water::setup);
    app.add_systems(Update, water::stream);
    app.update();

    // Cold: a jump far enough that nothing can be carried.
    let mut jump = 0.0f32;
    let cold = best(4, || {
        jump += SNAP_M * 200.0;
        app.world_mut().resource_mut::<Eye>().pos = eye0 + Vec3::new(jump, 0.0, 0.0);
        app.update();
    });
    // Warm: one cell along one axis, which is what walking does.
    app.world_mut().resource_mut::<Eye>().pos = eye0;
    app.update();
    let mut walk = 0.0f32;
    let warm = best(8, || {
        walk += SNAP_M;
        app.world_mut().resource_mut::<Eye>().pos = eye0 + Vec3::new(walk, 0.0, 0.0);
        app.update();
    });
    row("a cold sweep (teleport — nothing carried)", cold);
    row("a one-cell snap (walking — the core carried)", warm);
    println!("  {:<46} {:9.2}x", "ratio", cold / warm);

    // ── water::animate — what a frame that moves NOTHING still pays ────────
    //
    // `NOW.md` §0pf item 2 says the sea clones ~677 KiB into the render world
    // every frame: `Assets::get_mut` marks the mesh modified, and a mesh with
    // `MAIN_WORLD` usage is deep-cloned on every modification. That number was
    // quoted rather than derived, and the whole point of this file is that a
    // finding can be re-run — so it is measured here, off the mesh the sweep
    // above actually built.
    println!();
    println!("water::animate — the per-frame cost of a sea that is not moving");
    app.add_systems(Update, water::animate);
    app.insert_resource(Time::<()>::default());
    let anim = best(16, || {
        app.update();
    });
    let (verts, bytes) = {
        let sea = app.world().resource::<Sea>();
        let meshes = app.world().resource::<Assets<Mesh>>();
        match sea.mesh().and_then(|h| meshes.get(h)) {
            Some(m) => {
                let v = m.count_vertices();
                // What one modification hands the extractor: every attribute
                // buffer plus the index buffer.
                let per_vertex = m
                    .attributes()
                    .map(|(_, values)| values.get_bytes().len())
                    .sum::<usize>();
                let idx = m.indices().map_or(0, |i| i.len() * 4);
                (v, per_vertex + idx)
            }
            None => (0, 0),
        }
    };
    println!("  {:<46} {:9}", "the sea's vertices", verts);
    println!(
        "  {:<46} {:8.1} KiB",
        "…and the bytes one modification re-clones",
        bytes as f32 / 1024.0
    );
    row("stream + animate on a still frame", anim);
}
