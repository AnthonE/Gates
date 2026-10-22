//! Gate: the sea's snap carries the last sweep's answers without changing them.
//!
//! `water::stream` re-taps the ground when the eye crosses a [`SNAP_M`] cell —
//! 5,929 `terrain::height` taps plus four more per shoreline vertex through
//! `terrain::slope`, ~4.5 ms, about 0.6 times a second at a walk. Most of it
//! is a repeat: [`SNAP_M`] is an exact multiple of [`STEP_M`], so a one-cell
//! step along one axis slides every core coordinate onto the coordinate four
//! slots away and the world position is the same `f32` — the same bits, not a
//! near value.
//!
//! `NOW.md` §0pf item 2 says the opposite ("no half-lattice left to share"),
//! and it is right about the SKIRT, whose coordinates are geometric. This
//! suite is the evidence for the half it is wrong about, and it is written the
//! only way that claim can be checked: **walk the grid and compare it against
//! one built from scratch at the same place, value by value on `to_bits()`.**
//!
//! Four caches and a rate, five snap directions, and the diagonal — which the
//! carry must REFUSE, because a diagonal slides both axes and a vertex would
//! have to read a slot that moved in a different row.
//!
//! No window, no GPU, no socket: `MinimalPlugins` plus the asset plugin,
//! `tests/music.rs`'s posture.
//!
//! **Every claim was watched going red under its own mutant**, and one row of
//! the table is GREEN on purpose:
//!
//! | mutant | result |
//! |---|---|
//! | the `spacing[j] == spacing[i]` condition dropped from `carry_of` | RED (5 of 8) |
//! | `reach` slid with nothing else — `depth` moves, the rate does not | RED (5 of 8) |
//! | the diagonal let through as an x snap | RED (the diagonal test, exactly) |
//! | the `coords[j] == coords[i] + off` identity dropped | GREEN (see below) |
//! | the index shift derived wrong (`SNAP_M / STEP_M` read as 2, not 4) | RED — **and only because of §5** |
//!
//! **The last two rows are the useful ones and both were surprises.**
//!
//! The coordinate identity is not redundant, it is *unfalsifiable by a
//! correctness gate on this table*: the spacing condition and the `DEPTH_R_M`
//! bound already confine the carried range to the uniform core, where the
//! identity holds by construction. It stays because it is the claim that
//! actually matters — the world position is the same `f32` — while `spacing`
//! is a proxy that happens to imply it here and need not for a table nobody
//! has written yet.
//!
//! The wrong-shift mutant was *predicted* red by that reasoning and measured
//! GREEN against §1–§4, which is the more interesting result. A wrong shift
//! makes `carry_of` refuse every index, so the sweep rebuilds, and a rebuild
//! is CORRECT — every comparison passes. The check does its job perfectly and
//! a value comparison cannot see it doing anything. Worse: **an implementation
//! that simply never carried would pass all eight of those tests.**
//!
//! §5 is what closes it — `Sea::carried`, a count of vertices the sweep took
//! from the last one, asserted as a floor on a one-cell snap and as zero on a
//! diagonal. With it, the wrong shift is red on
//! `a_one_cell_snap_carries_most_of_the_grid` and nothing else, which is
//! exactly right: the output never was wrong. It is a COUNT and not a time,
//! so the no-clock rule is untouched. The whole section exists because a
//! mutant that was supposed to prove a point disproved it instead.

use bevy::asset::AssetPlugin;
use bevy::prelude::*;

use client::render::water::{self, Sea, SNAP_M};
use client::render::{Eye, WorldId};

const SEED: u64 = 20260731;

/// A shoreline eye: the case with the most to lose, because `shore` is the
/// cache that costs a `terrain::slope` (four more `height` taps) and it only
/// exists in a band around the coast. An inland eye tests the cheap half.
fn shore_eye() -> Vec3 {
    // Walk out from the island centre along +X until the ground drops under
    // the sea, then step back into the surf band. Found, never written down:
    // where a seed's coastline is is the seed's business.
    let haven = sim_core::terrain::haven(SEED);
    let mut x = 2048.0f32;
    while x < 2048.0 {
        if sim_core::terrain::ground(SEED, &haven, x, 2048.0) < 0.0 {
            return Vec3::new(x - 4.0, 2.0, 2048.0);
        }
        x += 4.0;
    }
    Vec3::new(2048.0, 2.0, 2048.0)
}

fn app(at: Vec3) -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()));
    app.init_asset::<Mesh>();
    app.init_asset::<Image>();
    app.init_asset::<water::WaterMaterial>();
    app.insert_resource(WorldId::new(SEED));
    app.init_resource::<Sea>();
    app.insert_resource(Eye {
        pos: at,
        ..default()
    });
    app.add_systems(Startup, water::setup);
    app.add_systems(Update, water::stream);
    app.update();
    app
}

fn same(a: &App, b: &App, what: &str) {
    let (sa, sb) = (a.world().resource::<Sea>(), b.world().resource::<Sea>());
    assert_eq!(
        sa.cell(),
        sb.cell(),
        "{what}: the two grids are not co-located"
    );
    let n = sa.n() * sa.n();
    assert!(n > 1000, "{what}: the grid is {n} vertices");
    for i in 0..n {
        assert_eq!(
            sa.depth()[i].to_bits(),
            sb.depth()[i].to_bits(),
            "{what}: depth[{i}] carried {} against a fresh {}",
            sa.depth()[i],
            sb.depth()[i]
        );
        assert_eq!(
            sa.shoal()[i].to_bits(),
            sb.shoal()[i].to_bits(),
            "{what}: shoal[{i}]"
        );
        assert_eq!(
            sa.shore()[i].to_bits(),
            sb.shore()[i].to_bits(),
            "{what}: shore[{i}] carried {} against a fresh {}",
            sa.shore()[i],
            sb.shore()[i]
        );
        assert_eq!(
            sa.reach()[i].to_bits(),
            sb.reach()[i].to_bits(),
            "{what}: reach[{i}] carried {} against a fresh {}",
            sa.reach()[i],
            sb.reach()[i]
        );
        for k in 0..4 {
            assert_eq!(
                sa.base()[i][k].to_bits(),
                sb.base()[i][k].to_bits(),
                "{what}: base[{i}][{k}]"
            );
        }
    }
}

/// Walk `steps` snaps along `(dx, dz)`, one at a time, and compare against a
/// grid built from scratch at the destination.
fn walked_equals_fresh(dx: i32, dz: i32, steps: i32, what: &str) {
    let start = shore_eye();
    let mut walked = app(start);
    for k in 1..=steps {
        walked.world_mut().resource_mut::<Eye>().pos = start
            + Vec3::new(
                dx as f32 * SNAP_M * k as f32,
                0.0,
                dz as f32 * SNAP_M * k as f32,
            );
        walked.update();
    }
    let end = start
        + Vec3::new(
            dx as f32 * SNAP_M * steps as f32,
            0.0,
            dz as f32 * SNAP_M * steps as f32,
        );
    let fresh = app(end);
    same(&walked, &fresh, what);
}

#[test]
fn walking_east_carries_and_changes_nothing() {
    walked_equals_fresh(1, 0, 6, "east");
}

#[test]
fn walking_west_carries_and_changes_nothing() {
    walked_equals_fresh(-1, 0, 6, "west");
}

#[test]
fn walking_north_carries_and_changes_nothing() {
    walked_equals_fresh(0, 1, 6, "north");
}

#[test]
fn walking_south_carries_and_changes_nothing() {
    walked_equals_fresh(0, -1, 6, "south");
}

/// A diagonal slides both axes at once, so no vertex can read its answer out
/// of a shifted row. The carry has to refuse it and rebuild — and the only way
/// to see that from outside is that the result is still right.
#[test]
fn a_diagonal_snap_rebuilds_rather_than_carrying() {
    walked_equals_fresh(1, 1, 4, "diagonal");
}

/// A multi-cell jump — a teleport, a respawn, a spawn on the far shore. The
/// shift would be 8 slots and most of the core would still admit it, which is
/// exactly why it has to be checked rather than assumed to be one cell.
#[test]
fn a_two_cell_jump_carries_and_changes_nothing() {
    let start = shore_eye();
    let mut jumped = app(start);
    jumped.world_mut().resource_mut::<Eye>().pos = start + Vec3::new(SNAP_M * 2.0, 0.0, 0.0);
    jumped.update();
    let fresh = app(start + Vec3::new(SNAP_M * 2.0, 0.0, 0.0));
    same(&jumped, &fresh, "two-cell jump");
}

/// A jump past the whole core: nothing can be carried and the sweep must be
/// complete rather than partial.
#[test]
fn a_jump_past_the_core_rebuilds_completely() {
    let start = shore_eye();
    let mut jumped = app(start);
    jumped.world_mut().resource_mut::<Eye>().pos = start + Vec3::new(SNAP_M * 40.0, 0.0, 0.0);
    jumped.update();
    let fresh = app(start + Vec3::new(SNAP_M * 40.0, 0.0, 0.0));
    same(&jumped, &fresh, "long jump");
}

// ── 5. The carry actually happens ──────────────────────────────────────────

/// Everything above compares a walked grid with a fresh one, and a REBUILD
/// satisfies all of it — so without this the whole suite is green on a
/// `carry_of` that returns `None` every time, which is exactly what a wrong
/// index shift produces (see the header's mutant table).
///
/// `Sea::carried` is therefore not a diagnostic; it is the only observable
/// difference between the optimization working and the optimization being
/// absent. Asserted as a COUNT of vertices, never as elapsed time.
#[test]
fn a_one_cell_snap_carries_most_of_the_grid() {
    let start = shore_eye();
    let mut app = app(start);
    let n = app.world().resource::<Sea>().n();

    app.world_mut().resource_mut::<Eye>().pos = start + Vec3::new(SNAP_M, 0.0, 0.0);
    app.update();
    let carried = app.world().resource::<Sea>().carried();
    // The uniform core is 65 of the 89 coordinates and a one-cell step gives
    // up four of them at the leading edge, so a healthy carry is ~61 columns
    // of full rows. Half the grid is a floor with room for the table to
    // change shape; zero is the failure this test exists for.
    assert!(
        carried >= n * n / 2,
        "a one-cell snap carried {carried} of {} vertices — under half means the \
         shift is being refused and every snap is a full rebuild, which is \
         CORRECT and is not what this code is for",
        n * n
    );

    // And the other axis, since the two take different paths through `slide`
    // (a run per row against one contiguous block).
    app.world_mut().resource_mut::<Eye>().pos = start + Vec3::new(SNAP_M, 0.0, SNAP_M);
    app.update();
    let carried_z = app.world().resource::<Sea>().carried();
    assert!(
        carried_z >= n * n / 2,
        "a one-cell snap along z carried {carried_z} of {}",
        n * n
    );
}

/// The refusals are refusals. A diagonal and a jump past the core must carry
/// NOTHING — the assertion that stops §5 above from being satisfied by a
/// carry that ignores its own preconditions.
#[test]
fn a_diagonal_and_a_long_jump_carry_nothing() {
    let start = shore_eye();
    let mut app = app(start);

    app.world_mut().resource_mut::<Eye>().pos = start + Vec3::new(SNAP_M, 0.0, SNAP_M);
    app.update();
    assert_eq!(
        app.world().resource::<Sea>().carried(),
        0,
        "a diagonal snap carried something — no vertex can read its answer out \
         of a row that also moved"
    );

    app.world_mut().resource_mut::<Eye>().pos = start + Vec3::new(SNAP_M * 40.0, 0.0, 0.0);
    app.update();
    assert_eq!(
        app.world().resource::<Sea>().carried(),
        0,
        "a jump past the whole core carried something"
    );
}

/// And inland, where `shore` is empty and `depth` is the sentinel — the case
/// where a carry could look right for the wrong reason.
#[test]
fn walking_inland_carries_and_changes_nothing() {
    let start = Vec3::new(2048.0, 20.0, 2048.0);
    let mut walked = app(start);
    for k in 1..=5 {
        walked.world_mut().resource_mut::<Eye>().pos =
            start + Vec3::new(SNAP_M * k as f32, 0.0, 0.0);
        walked.update();
    }
    let fresh = app(start + Vec3::new(SNAP_M * 5.0, 0.0, 0.0));
    same(&walked, &fresh, "inland east");
}

// ---------------------------------------------------------------------------
// The surface the vertex stage draws (`water.wgsl`).
// ---------------------------------------------------------------------------

/// An eye on a real beach: walk in from the open sea east of the island,
/// along its middle, to the first dry ground — the coast, never a lake — and
/// stand a few metres out in the water, so the core holds swell, breakers and
/// the shore wash at once.
fn beach_eye() -> Vec3 {
    let haven = sim_core::terrain::haven(SEED);
    let z = sim_core::terrain::ISLAND_SIZE * 0.5;
    let mut x = sim_core::terrain::ISLAND_SIZE;
    while x > z {
        if sim_core::terrain::ground(SEED, &haven, x, z) >= sim_core::terrain::SEA_LEVEL {
            return Vec3::new(x + 8.0, 2.0, z);
        }
        x -= 4.0;
    }
    panic!("no coast east of the middle of seed {SEED}");
}

/// **The GPU's surface is the CPU's.** `water::surface_local` is the vertex
/// stage line for line — local offsets, phases folded around the centre in
/// `f64`, every angle reduced in cycles — and `water::surface_at` is the sea
/// as the CPU drew it before the port, at world coordinates. Over every vertex
/// of a real coast, mid-phase in a rough sea, they agree; and the vertices
/// that carry the swell, the breakers and the wash are all among them, so the
/// agreement is not three empty sets.
///
/// The foam is the fragment's (`water::foam_px`, at a pixel of no width), off
/// the vertex stage's outputs. Exempt from it: the instant a bore's crest
/// passes, where the trail restarts at 1 and two arithmetics may sit either
/// side of it, and the `SURF_FRONT_M` before it, which the GPU softens and
/// the CPU never did.
#[test]
fn the_vertex_stage_draws_the_cpu_surface() {
    let app = app(beach_eye());
    let sea = app.world().resource::<Sea>();
    let centre = sea.centre().expect("the sweep ran");
    let coords = sea.coords();
    let n = coords.len();
    let state = water::SeaState::at(0.6);
    let phase = [0.3, 2.9, 4.4, 5.9];
    let surf_phase = 1.7f32;
    let folded = water::fold_phases(&phase, centre);
    let surf_cycles = surf_phase / std::f32::consts::TAU;
    let (mut swell, mut bores, mut wash) = (0usize, 0usize, 0usize);
    for iz in 0..n {
        for ix in 0..n {
            let i = iz * n + ix;
            let v = sea.statics(ix, iz);
            let want = water::surface_at(
                centre.x + coords[ix],
                centre.y + coords[iz],
                &phase,
                surf_phase,
                &state,
                sea.spacing_at(ix, iz),
                sea.shoal()[i],
                &v,
            );
            let lx = coords[ix];
            let lz = coords[iz];
            let out = water::surface_local(lx, lz, &folded, surf_cycles, &state, &v);
            let s0 = water::surge_sin(lx, lz, &folded);
            let foam = water::foam_px(out.caps, v.wash, out.cs, out.b, surf_cycles, s0, 0.0);
            let g = out.g_swell + out.g_breaker;
            // The bore's phase since its crest, in cycles: the CPU's trail
            // restarts there, and the per-pixel front softens the last
            // `SURF_FRONT_M` before it, which the CPU never did.
            let lead = water::SURF_FRONT_M / water::SURF_LEN_M;
            let since = {
                let c = out.cs + surf_cycles - 0.25;
                c - c.floor()
            };
            let at_crest = v.surf_dir != [0.0, 0.0] && !(1e-4..=1.0 - lead - 1e-4).contains(&since);
            for (what, a, b) in [
                ("height", out.h, want.h),
                ("gradient x", g.x, want.gx),
                ("gradient z", g.y, want.gz),
                ("foam", foam, want.foam),
            ] {
                if what == "foam" && at_crest {
                    continue;
                }
                assert!(
                    (a - b).abs() < 1e-3,
                    "vertex ({ix}, {iz}) {what}: the GPU draws {a}, the CPU drew {b}"
                );
            }
            // A pixel wider than a breaker settles to the trail's mean.
            if out.b > 0.0 {
                let wide = water::foam_px(0.0, 0.0, out.cs, out.b, surf_cycles, s0, 0.7);
                let mean = water::SURF_TRAIL_RAD / std::f32::consts::TAU
                    * (1.0 - (-std::f32::consts::TAU / water::SURF_TRAIL_RAD).exp());
                assert!(
                    (wide - (out.b * mean).min(1.0)).abs() < 1e-5,
                    "vertex ({ix}, {iz}): a coarse pixel's bore is {wide}, not the mean"
                );
            }
            swell += usize::from(v.swell.iter().any(|w| *w > 0.0));
            bores += usize::from(v.surf_amp > 0.0);
            wash += usize::from(v.wash > 0.0);
        }
    }
    assert!(
        swell > 1000 && bores > 10 && wash > 10,
        "the coast did not exercise every term: {swell} swell, {bores} bores, {wash} wash"
    );
}

/// **The mesh holds what the vertex stage reads**, at every vertex: the local
/// offset as the position (X from `ix`, Z from `iz` — transposed, the swell
/// would run across its own gradient), and the per-snap inputs where its
/// `@location`s expect them. The entity stands at the centre.
#[test]
fn the_mesh_carries_the_vertex_statics() {
    use bevy::mesh::VertexAttributeValues as V;
    let mut app = app(beach_eye());
    let sea = app.world().resource::<Sea>();
    let centre = sea.centre().expect("the sweep ran");
    let n = sea.n();
    let coords = sea.coords().to_vec();
    let statics: Vec<_> = (0..n * n).map(|i| sea.statics(i % n, i / n)).collect();
    let mesh = app
        .world()
        .resource::<Assets<Mesh>>()
        .get(sea.mesh().expect("built"))
        .expect("the sea's mesh")
        .clone();
    let Some(V::Float32x3(pos)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else {
        panic!("no positions")
    };
    let Some(V::Float32x2(uv_b)) = mesh.attribute(Mesh::ATTRIBUTE_UV_1) else {
        panic!("no UV_1")
    };
    let Some(V::Float32x4(swell)) = mesh.attribute(water::ATTRIBUTE_SWELL) else {
        panic!("no swell shares")
    };
    let Some(V::Float32x4(surf)) = mesh.attribute(water::ATTRIBUTE_SURF) else {
        panic!("no breakers")
    };
    let Some(V::Float32(reach)) = mesh.attribute(water::ATTRIBUTE_REACH) else {
        panic!("no reach")
    };
    for (i, v) in statics.iter().enumerate() {
        let (ix, iz) = (i % n, i / n);
        assert_eq!(
            pos[i],
            [coords[ix], sim_core::terrain::SEA_LEVEL, coords[iz]],
            "position {i}"
        );
        assert_eq!(uv_b[i], [v.wash, v.depth], "UV_1 {i}");
        assert_eq!(swell[i], v.swell, "swell shares {i}");
        assert_eq!(
            surf[i],
            [v.surf_dist, v.surf_dir[0], v.surf_dir[1], v.surf_amp],
            "breakers {i}"
        );
        assert_eq!(reach[i], v.reach, "reach {i}");
    }
    let tf = app
        .world_mut()
        .query_filtered::<&Transform, With<water::SeaSurface>>()
        .single(app.world())
        .expect("one sea");
    assert_eq!(tf.translation, Vec3::new(centre.x, 0.0, centre.y));
}

/// **The uniform folds the phases around the centre the mesh is drawn at**,
/// in the frame that moved it: `animate` runs after `stream`, and on a snap
/// the material's centre, the entity's translation and the folded phases all
/// name the new cell. One frame behind and the whole swell would jump by up
/// to a wavelength on every snap.
#[test]
fn the_uniform_folds_around_the_drawn_centre() {
    let mut app = app(beach_eye());
    app.add_systems(Update, water::animate.after(water::stream));
    app.update();
    app.world_mut().resource_mut::<Eye>().pos += Vec3::new(SNAP_M, 0.0, 0.0);
    app.update();
    let sea = app.world().resource::<Sea>();
    let centre = sea.centre().expect("the sweep ran");
    let folded = water::fold_phases(&sea.phases().0, centre);
    let params = app
        .world()
        .resource::<Assets<water::WaterMaterial>>()
        .iter()
        .next()
        .expect("the sea's material")
        .1
        .extension
        .params;
    let tf = *app
        .world_mut()
        .query_filtered::<&Transform, With<water::SeaSurface>>()
        .single(app.world())
        .expect("one sea");
    assert_eq!(Vec2::new(params.field.x, params.field.y), centre);
    assert_eq!(Vec2::new(tf.translation.x, tf.translation.z), centre);
    assert_eq!(params.phase.to_array(), folded);
}
