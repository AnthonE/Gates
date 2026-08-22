//! The freehand aim rule: whether a placement declines the plate latch is a
//! function of WHERE IN THE CELL the crosshair landed, not of a key
//! (freehand placement v0, `DECISIONS.md` 2026-08-22).
//!
//! **Why an aim and not a modifier.** The reference has no freehand button —
//! placement there is continuous and a piece is attracted to a socket when
//! you aim near one, so you get a freehand piece by aiming where no socket
//! catches it (`reference/BUILDING.md` §7c.3, and it is why their own guides
//! call the technique "tricky and non-intuitive"). Ours is address-based, so
//! there is no "near": `Place` carries a cell and `plate_for` latches on
//! exact adjacency. What ports is that `aim_from_look` still produces a real
//! `(f32, f32)` and `target_at` quantizes it away — the sub-cell remainder is
//! the input the model was missing.
//!
//! This gates the client half. `sim-core/tests/plate.rs` gates what the bit
//! then MEANS; neither can stand in for the other, because a correct rule
//! wired to an aim nobody computes is a feature nobody can reach.

use client::ui::place::{freehand_from_aim, Site, Target, SNAP_BAND_M};
use sim_core::build::{place, BuildContent, Pieces, BUILD_CELL_M, LOC_PLANE};
use sim_core::deploy::Deploys;
use sim_core::gather::ItemStack;
use sim_core::limits::INV_SLOTS;
use sim_core::movement::Body;
use sim_core::terrain;
use sim_core::world::{EventQueue, Player};

const SEED: u64 = 20260731;
const CX: u16 = 341;
const CZ: u16 = 341;

fn hv() -> &'static terrain::Haven {
    use std::sync::OnceLock;
    static H: OnceLock<terrain::Haven> = OnceLock::new();
    H.get_or_init(|| terrain::haven(SEED))
}

/// A store holding one twig foundation at each listed cell, placed through
/// the sim's own verb so the column index is the one the game would have.
fn store_with(cells: &[(u16, u16)]) -> (Pieces, BuildContent) {
    let bc = BuildContent::probe_fixture();
    let mut pieces = Pieces::new();
    let deploys = Deploys::new();
    for &(cx, cz) in cells {
        let (ax, az) = sim_core::build::anchor(cx, cz, LOC_PLANE);
        let mut p = Player {
            id: 7,
            active: true,
            body: Body::at(SEED, hv(), ax, az),
            ..Player::default()
        };
        for (i, slot) in p.inv.iter_mut().enumerate().take(2) {
            *slot = ItemStack {
                item: i as u16,
                count: 99,
                cond: 0,
            };
        }
        let mut ev = EventQueue::default();
        place(
            SEED,
            hv(),
            &bc,
            &deploys,
            &mut pieces,
            &mut p,
            0,
            0,
            cx,
            cz,
            0,
            LOC_PLANE,
            false,
            &mut ev,
        );
    }
    (pieces, bc)
}

fn site<'a>(pieces: &'a Pieces, bc: &'a BuildContent, inv: &'a [ItemStack; INV_SLOTS]) -> Site<'a> {
    Site {
        seed: SEED,
        haven: hv(),
        at: (0.0, 0.0),
        taken: pieces.entries(),
        cols: pieces.cols(),
        content: bc,
        inv,
    }
}

fn target(cx: u16, cz: u16) -> Target {
    Target {
        cx,
        cz,
        level: 0,
        loc: LOC_PLANE,
    }
}

/// Sweeping the crosshair across ONE cell flips the bit, and it flips in the
/// direction the mechanic claims: near the built neighbour it snaps, at the
/// far edge it goes freehand.
///
/// **The sweep is the assertion, not two sampled points.** A rule that
/// returned a constant would pass any single-point check written to match it;
/// what cannot be faked is that the flip happens exactly once, at
/// `SNAP_BAND_M`, walking away from the neighbour.
///
/// ⚠ **What this deliberately cannot police is the constant's VALUE.** It
/// measures the flip against `SNAP_BAND_M`, so narrowing the band to a third
/// moves the constant and the assertion together and this test stays green —
/// run and confirmed, 2026-08-22. That is correct, because the band is a knob
/// (`DECISIONS.md` §open) and not a law; what IS a law is the consequence the
/// two-thirds value has, and `a_cell_between_two_columns_cannot_decline`
/// holds that one with an explicit assert on the overlap. Same-mutant
/// coverage is split between the two on purpose — noted here because a
/// reader who saw only this test would over-trust it.
#[test]
fn the_bit_flips_where_the_snap_band_ends() {
    // Foundation at CX; the empty cell to its +x is where we aim.
    let (pieces, bc) = store_with(&[(CX, CZ)]);
    let inv = [ItemStack::default(); INV_SLOTS];
    let s = site(&pieces, &bc, &inv);
    let t = target(CX + 1, CZ);

    let x0 = (CX + 1) as f32 * BUILD_CELL_M;
    let z_mid = CZ as f32 * BUILD_CELL_M + BUILD_CELL_M * 0.5;

    // Walk from the shared edge to the far edge in 100 steps.
    let n = 100;
    let mut flips = 0;
    let mut prev: Option<bool> = None;
    let mut first_free_at = f32::MAX;
    for i in 0..=n {
        let d = BUILD_CELL_M * (i as f32) / (n as f32); // distance from the shared edge
        let free = freehand_from_aim(&s, t, (x0 + d, z_mid));
        if free && d < first_free_at {
            first_free_at = d;
        }
        if let Some(p) = prev {
            if p != free {
                flips += 1;
            }
        }
        prev = Some(free);
    }
    assert_eq!(
        flips, 1,
        "the bit flipped {flips} times across one cell; it must flip once"
    );
    assert!(
        (first_free_at - SNAP_BAND_M).abs() <= BUILD_CELL_M / n as f32 * 1.5,
        "the flip landed at {first_free_at} m from the edge, not at SNAP_BAND_M {SNAP_BAND_M} m"
    );
    // And the two ends say what the mechanic says they say.
    assert!(
        !freehand_from_aim(&s, t, (x0 + 0.05, z_mid)),
        "aiming at the neighbour's edge did not snap"
    );
    assert!(
        freehand_from_aim(&s, t, (x0 + BUILD_CELL_M - 0.05, z_mid)),
        "aiming at the far edge did not go freehand"
    );
}

/// **No built neighbour, no bit.** Over open ground there is no latch to
/// decline, so the answer is false everywhere in the cell — a bit that
/// flickered with the crosshair out here would be noise on the wire and in
/// every replay that carries it.
#[test]
fn open_ground_never_sets_the_bit() {
    let (pieces, bc) = store_with(&[]);
    let inv = [ItemStack::default(); INV_SLOTS];
    let s = site(&pieces, &bc, &inv);
    let t = target(CX + 1, CZ);
    let x0 = (CX + 1) as f32 * BUILD_CELL_M;
    let z0 = CZ as f32 * BUILD_CELL_M;
    for i in 0..=20 {
        for j in 0..=20 {
            let a = (
                x0 + BUILD_CELL_M * i as f32 / 20.0,
                z0 + BUILD_CELL_M * j as f32 / 20.0,
            );
            assert!(
                !freehand_from_aim(&s, t, a),
                "an empty neighbourhood set the freehand bit at {a:?}"
            );
        }
    }
}

/// **A cell wedged between two built columns can never go freehand.** The
/// band is two thirds of the cell measured from each side, so the two
/// overlap and no point clears both. Deliberate, and asserted rather than
/// left to be discovered: an interior cell of a base is the one place a
/// second floor height has nothing to mean.
#[test]
fn a_cell_between_two_columns_cannot_decline() {
    let (pieces, bc) = store_with(&[(CX, CZ), (CX + 2, CZ)]);
    let inv = [ItemStack::default(); INV_SLOTS];
    let s = site(&pieces, &bc, &inv);
    // Both neighbours must actually be in the index, or this proves nothing.
    assert!(
        pieces.cols().plate(CX, CZ).is_some() && pieces.cols().plate(CX + 2, CZ).is_some(),
        "the fixture needs both flanking columns built"
    );
    let t = target(CX + 1, CZ);
    let x0 = (CX + 1) as f32 * BUILD_CELL_M;
    let z_mid = CZ as f32 * BUILD_CELL_M + BUILD_CELL_M * 0.5;
    for i in 0..=100 {
        let a = (x0 + BUILD_CELL_M * i as f32 / 100.0, z_mid);
        assert!(
            !freehand_from_aim(&s, t, a),
            "a cell between two built columns went freehand at {a:?}"
        );
    }
    // A `const` block rather than a runtime assert, which is what clippy
    // asked for and is the better gate anyway: narrowing the band below half
    // a cell now fails to COMPILE, which is louder than a red test and
    // reaches whoever moves the knob rather than whoever next runs the suite.
    const {
        assert!(
            SNAP_BAND_M * 2.0 > BUILD_CELL_M,
            "the two bands no longer overlap, so this test's whole claim is false"
        )
    };
}
