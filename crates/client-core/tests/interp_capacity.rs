//! The interpolator holds what the world can hold, not what one datagram
//! can carry.
//!
//! `MAX_SNAPSHOT_ENTITIES` is a **per-snapshot wire count** — how many
//! records one datagram's 7-bit count field and 1,100 B budget will carry.
//! `Interp` was sized on it and filled *cumulatively*: a zero-state
//! snapshot clears the table, and every snapshot after it adds whatever
//! ids it names. The set of ids a client has seen since its last
//! zero-state is therefore the **union** of many snapshots, and that union
//! is bounded by the world (`MAX_PLAYERS + MAX_MOBS`), never by one wire
//! record count.
//!
//! The failure that shape produces is silent and permanent, which is why
//! it is gated here rather than trusted to review: `Interp::push` returns
//! on a full table, `ClientView` does not, so the 65th distinct id lands
//! in the view and never in the interpolation history — and the renderer
//! drives spawning solely off `interp.ids()` (`render/bodies.rs`,
//! `render/mobs.rs`). The body is on the client's map, inside AOI, and is
//! never drawn until something forces a resync.
//!
//! The assertion is the invariant itself and not a pinned number: **every
//! entity the view holds has interpolation history**, so a body the client
//! knows about is a body the client can draw. Measured red at 64 vs 80.

use client_core::core::{ClientCore, Ingest};
use protocol::{EntityState, SnapshotEncoder, SnapshotHeader};
use sim_core::limits::{DATAGRAM_BUDGET_BYTES, MAX_PLAYERS, MAX_SNAPSHOT_ENTITIES};

/// The player this core is. Deliberately outside both id runs below: the
/// own entity is reconciled by the predictor and never pushed into the
/// interpolator, so it would make the two counts differ for a reason that
/// is not the defect.
const ME: u32 = 0xDEAD;

/// Two runs of ids, both distinct from `ME` and from each other. 40 + 40
/// straddles `MAX_SNAPSHOT_ENTITIES` (64) while each run alone fits it —
/// so nothing here tests the wire's own cap, only the client's cumulative
/// table.
const RUN: u32 = 40;

fn ent(id: u32) -> EntityState {
    EntityState {
        id,
        // Spread along x so no two records are identical; well inside the
        // island and inside every range `check_ranges` enforces.
        qx: 30_000 + id as i32 * 10,
        qy: 500,
        qz: 30_000,
        qvy: 0,
        grounded: true,
        sleeping: false,
        dead: false,
        yaw: 0,
        pitch: 0,
        // Empty hand: this suite is about the interpolator's table, and a
        // held item would put a second reason for two records to differ
        // into a file whose whole subject is the first.
        held: None,
        lit: false,
    }
}

/// Encode one snapshot the way the server's fill loop does — the budget IS
/// the buffer, so this is exactly the datagram a shard would put on the
/// wire, and the test fails loudly rather than silently shrinking if a run
/// stops fitting.
fn snapshot(
    tick: u32,
    baseline_age: u8,
    ids: std::ops::Range<u32>,
    baseline: &[EntityState],
    buf: &mut [u8; DATAGRAM_BUDGET_BYTES],
) -> usize {
    let header = SnapshotHeader {
        tick,
        baseline_age,
        last_executed_seq: 0,
        nudge: protocol::Nudge::Ok,
    };
    let mut enc = SnapshotEncoder::begin(buf, &header, baseline).expect("begin");
    for id in ids {
        enc.add_entity(&ent(id))
            .unwrap_or_else(|e| panic!("entity {id} did not fit the budget: {e:?}"));
    }
    enc.finish().expect("finish")
}

#[test]
fn the_interpolator_holds_every_entity_the_view_holds() {
    let mut core = ClientCore::new(1, ME, 0);

    // A zero-state snapshot: the authoritative restart of the entity set.
    let mut buf = [0u8; DATAGRAM_BUDGET_BYTES];
    let len = snapshot(100, 0, 1..1 + RUN, &[], &mut buf);
    assert_eq!(core.on_datagram(&buf[..len]), Ingest::Applied);
    assert_eq!(core.view.entities.len(), RUN as usize);
    assert_eq!(core.interp.ids().count(), RUN as usize);

    // A delta against it naming 40 *different* ids — peers that walked
    // into AOI. The view keeps the first 40 (a delta never clears the
    // map), so the client now knows 80 bodies.
    let baseline: Vec<EntityState> = (1..1 + RUN).map(ent).collect();
    let mut buf2 = [0u8; DATAGRAM_BUDGET_BYTES];
    let len2 = snapshot(102, 2, 1 + RUN..1 + 2 * RUN, &baseline, &mut buf2);
    assert_eq!(core.on_datagram(&buf2[..len2]), Ingest::AppliedDelta);
    assert_eq!(
        core.view.entities.len(),
        2 * RUN as usize,
        "the view must accumulate both runs"
    );

    assert_eq!(
        core.interp.ids().count(),
        core.view.entities.len(),
        "an entity the client knows about with no interpolation history is \
         a body inside AOI that is never drawn — {} of {} lost, because the \
         table is sized on the per-snapshot wire count \
         (MAX_SNAPSHOT_ENTITIES = {MAX_SNAPSHOT_ENTITIES}) rather than on \
         what the world can hold",
        core.view.entities.len() - core.interp.ids().count(),
        core.view.entities.len()
    );

    // And every one of them samples — `ids()` reporting a slot is not the
    // same claim as the history in it being usable.
    let mut r = client_core::interp::RemoteState::default();
    for (id, _) in core.view.entities.iter() {
        assert!(core.interp.sample(*id, 102.0, &mut r), "no sample for {id}");
    }
    assert_eq!(core.interp.drops, 0, "nothing may be dropped at this size");
}

/// The table is sized on the world, so it cannot be the binding constraint
/// even if the wire's cap is later raised: a shard's whole population plus
/// its whole animal roster fits.
#[test]
fn the_table_is_sized_on_the_world_and_says_so() {
    assert_eq!(
        client_core::interp::INTERP_SLOTS,
        MAX_PLAYERS + sim_core::limits::MAX_MOBS
    );
    const {
        assert!(
            client_core::interp::INTERP_SLOTS > MAX_SNAPSHOT_ENTITIES,
            "sizing the interpolator on the wire's per-snapshot count is the \
             defect this file exists for"
        )
    };

    // The drop counter is the other half of wall 4's "stated overflow
    // policy": past the world's own size there is nothing left to hold,
    // and the refusal must be counted rather than silent.
    let mut it = client_core::interp::Interp::new();
    for id in 0..client_core::interp::INTERP_SLOTS as u32 + 3 {
        it.push(10, &ent(id + 1));
    }
    assert_eq!(it.ids().count(), client_core::interp::INTERP_SLOTS);
    assert_eq!(it.drops, 3, "the refusal must be counted, not silent");
}

/// The interpolation delay has ONE home, and it is `sim_core::limits`.
///
/// `interp.rs` carried its own `f64` spelling of it, hand-typed as `4.0`,
/// until lag compensation needed the same number on the *server*
/// — to say how stale a client's aim is, which is
/// `(T - S) + INTERP_DELAY_TICKS - REWIND_ACK_BIAS_TICKS`
/// (`findings/lagcomp-design-20260818.md` §2.2). Two crates typing one
/// constant is the `props.js` drift `CLAUDE.md` opens with.
///
/// The mirror was **deleted rather than gated**, which is the stronger fix
/// and was forced by a gate that already existed: `ci/knob_registry.mjs`
/// pins a registry claim against the constant a source file declares and
/// cannot evaluate arithmetic, so even a spelling that referred to `limits`
/// and held no second *value* of its own failed loudly. Two
/// declarations of one knob is a shape that gate refuses however honest the
/// second one is, and it was right to.
///
/// So what is left to check is not equality between two constants; it is
/// that the derivation still holds and that the client still reads the one
/// declaration.
#[test]
fn the_interp_delay_has_one_home_and_the_client_reads_it() {
    // The derivation the number came from (NETCODE.md §3's shipped rule):
    // 2 x the snapshot interval. If the interval moves, this is the argument
    // that has to be remade rather than the constant quietly kept.
    assert_eq!(
        u64::from(sim_core::limits::INTERP_DELAY_TICKS),
        sim_core::limits::SNAPSHOT_INTERVAL_TICKS * 2
    );

    // A re-typed literal anywhere in `client-core` would pass every other
    // test in this crate — the interpolator would still blend, just at a
    // delay the server no longer assumes — so the grep IS the assertion.
    const INTERP_SRC: &str = include_str!("../src/interp.rs");
    const CORE_SRC: &str = include_str!("../src/core.rs");

    // ⚠ Skip comment lines. The first draft was `contains("const
    // INTERP_DELAY_TICKS")` and it went red on this file's own module
    // header, which quotes the declaration it is describing the removal of —
    // a needle that matches prose is the vacuous-grep shape that turns up in
    // this tree about once a month, and it fails in whichever direction the
    // prose happens to sit. A declaration is a line of code.
    let redeclared = INTERP_SRC.lines().any(|l| {
        let l = l.trim_start();
        !l.starts_with("//") && l.contains("const INTERP_DELAY_TICKS")
    });
    assert!(
        !redeclared,
        "interp.rs re-declared the interpolation delay — it lives in \
         sim_core::limits, and a second declaration is the drift this test \
         exists for"
    );
    assert!(
        CORE_SRC.contains("sim_core::limits::INTERP_DELAY_TICKS"),
        "the render clock stopped reading the sim's delay; if that reader \
         moved, move this assertion with it rather than deleting it"
    );
}
