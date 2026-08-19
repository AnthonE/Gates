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
    let mut x = 1024.0f32;
    while x < 2048.0 {
        if sim_core::terrain::ground(SEED, &haven, x, 1024.0) < 0.0 {
            return Vec3::new(x - 4.0, 2.0, 1024.0);
        }
        x += 4.0;
    }
    Vec3::new(1024.0, 2.0, 1024.0)
}

fn app(at: Vec3) -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()));
    app.init_asset::<Mesh>();
    app.init_asset::<Image>();
    app.init_asset::<StandardMaterial>();
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
    let start = Vec3::new(1024.0, 20.0, 1024.0);
    let mut walked = app(start);
    for k in 1..=5 {
        walked.world_mut().resource_mut::<Eye>().pos =
            start + Vec3::new(SNAP_M * k as f32, 0.0, 0.0);
        walked.update();
    }
    let fresh = app(start + Vec3::new(SNAP_M * 5.0, 0.0, 0.0));
    same(&walked, &fresh, "inland east");
}
