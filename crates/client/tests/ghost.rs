//! The build ghost tells the truth about the piece it becomes.
//!
//! **What this asserts, and what it deliberately does not.** `render/ghost.rs`
//! and `render/structures.rs` both draw a doorway — since 2026-08-09 from ONE
//! emit site, `structures::shape_parts` (`NOW.md` §0u item 1). Asserting "the
//! ghost's table equals the kit's table" would be the byte-golden hole
//! `CLAUDE.md`'s trap list names — both could be wrong together and stay
//! green, which is how 27 Oxide payload fixes shipped under a pinned
//! `MSILHash`. Sharing the emit closes the drift *between* them; these tests
//! close the other half by checking the one table against the sim.
//!
//! So the assertion is against the **sim**: the posts the client draws stand
//! where `sim_core::collide::doorway_solid_at` says a player is blocked, and
//! the gap is where it says they are not. `RENDER.md` §8 states the stake — the
//! opening is 1.2 m x 2.1 m *because* that is what `edge_hit` refuses, and
//! "draw it elsewhere and the frame lies about where a player can walk".
//!
//! Headless, no GPU, no socket; it links `render` for the shared constants.

use client::render::structures::{
    door_opening_w, door_post_gap, shape_parts, LINTEL_DROP_M, LINTEL_H_M, MAX_PARTS, N_SHAPES,
    SEAM_M,
};
use sim_core::build::{BUILD_CELL_M, LEVEL_H_M};
use sim_core::collide::{
    doorway_solid_at, frame_solid_at, window_solid_at, DOOR_POST_W_M, FRAME_RIM_M, WINDOW_HEAD_M,
    WINDOW_SILL_M,
};

/// Where the two drawn posts span, as `t` along the edge.
fn drawn_posts() -> [(f32, f32); 2] {
    let gap = door_post_gap();
    let mid = BUILD_CELL_M * 0.5;
    let half = DOOR_POST_W_M * 0.5;
    [
        (mid - gap - half, mid - gap + half),
        (mid + gap - half, mid + gap + half),
    ]
}

fn drawn_solid_at(t: f32) -> bool {
    drawn_posts().iter().any(|(a, b)| t >= *a && t <= *b)
}

/// **The measured deviation, pinned.** The drawn posts are inset from the
/// cell boundary by half the cosmetic seam — `SEAM_M / 2` = 2 cm — because
/// `door_post_gap` is computed over `BUILD_CELL_M - SEAM_M` so adjacent edge
/// pieces do not z-fight. The sim blocks to the boundary itself.
///
/// This is a deliberate cosmetic offset, not a defect, and it is pinned here
/// rather than tolerated silently: at 2 cm no player can see or exploit it,
/// and if it ever grows this test says so before anyone has to notice a
/// phantom post in play.
const EXPECTED_INSET_M: f32 = SEAM_M * 0.5;

#[test]
fn the_posts_are_inset_by_exactly_half_the_seam_and_no_more() {
    let [(a0, a1), (b0, b1)] = drawn_posts();
    // The outer face of each post against the cell's two ends.
    assert!(
        (a0 - EXPECTED_INSET_M).abs() < 1e-4,
        "near post starts at {a0}, expected {EXPECTED_INSET_M}"
    );
    assert!(
        ((BUILD_CELL_M - b1) - EXPECTED_INSET_M).abs() < 1e-4,
        "far post ends {} from the boundary, expected {EXPECTED_INSET_M}",
        BUILD_CELL_M - b1
    );
    // The inner faces, against the band the sim stops blocking at.
    assert!(
        (a1 - (DOOR_POST_W_M + EXPECTED_INSET_M)).abs() < 1e-4,
        "near post ends at {a1}"
    );
    assert!(
        (b0 - (BUILD_CELL_M - DOOR_POST_W_M - EXPECTED_INSET_M)).abs() < 1e-4,
        "far post starts at {b0}"
    );
    // A const assertion, because the bound is on a constant: clippy is right
    // that a runtime `assert!` over two literals is not a test. It still
    // belongs here rather than beside the constant — this is the file that
    // explains why 2 cm is tolerable and 5 cm is not.
    const {
        assert!(
            EXPECTED_INSET_M < 0.05,
            "the drawn/blocked disagreement has grown past what a player \
             cannot see: a post they can walk through"
        )
    };
}

/// Everywhere except within the seam of a boundary, the drawn doorway and the
/// sim agree about solid and open.
#[test]
fn no_sampled_point_disagrees_with_the_sim() {
    // The four boundaries the seam sits astride.
    let seams = [
        0.0,
        DOOR_POST_W_M,
        BUILD_CELL_M - DOOR_POST_W_M,
        BUILD_CELL_M,
    ];
    let mut checked = 0;
    let mut skipped = 0;
    for i in 0..=600 {
        let t = BUILD_CELL_M * (i as f32 / 600.0);
        if seams.iter().any(|s| (t - s).abs() <= SEAM_M) {
            skipped += 1;
            continue;
        }
        assert_eq!(
            drawn_solid_at(t),
            doorway_solid_at(t),
            "at t={t}: drawn-solid={} but sim-blocks={}",
            drawn_solid_at(t),
            doorway_solid_at(t)
        );
        checked += 1;
    }
    // The suite must not pass by checking nothing, and must not pass by
    // skipping everything (`CLAUDE.md`: a pass it did not earn is the worst
    // bug class). Both halves are asserted.
    assert!(checked > 500, "only {checked} samples were compared");
    assert!(skipped < 60, "{skipped} samples were skipped as seam");
}

/// The middle of the opening is walkable and the middle of each post is not —
/// the two claims a player actually cares about, stated without tolerance.
#[test]
fn the_opening_is_open_and_the_posts_are_solid() {
    let mid = BUILD_CELL_M * 0.5;
    assert!(!doorway_solid_at(mid), "the doorway's opening is blocked");
    assert!(!drawn_solid_at(mid), "a post is drawn in the opening");
    let gap = door_post_gap();
    for c in [mid - gap, mid + gap] {
        assert!(doorway_solid_at(c), "the sim lets a player through a post");
        assert!(drawn_solid_at(c), "no post is drawn where the sim blocks");
    }
}

/// The lintel's underside is the top of the opening, and a player is 1.8 m.
#[test]
fn the_lintel_clears_a_standing_player() {
    let centre = LEVEL_H_M * 0.5 + (LEVEL_H_M * 0.5 - LINTEL_DROP_M);
    let underside = centre - LINTEL_H_M * 0.5;
    assert!(
        underside >= 1.8,
        "the doorway is {underside} m clear — a 1.8 m player cannot walk through"
    );
    // ...and it does not float: the lintel's top is the piece's own top.
    let top = centre + LINTEL_H_M * 0.5;
    assert!(
        (top - LEVEL_H_M).abs() < 1e-4,
        "the lintel's top is {top}, not the piece's {LEVEL_H_M}"
    );
}

/// The lintel spans exactly the gap the posts leave — not a third number that
/// happens to look right.
#[test]
fn the_lintel_spans_exactly_what_the_posts_leave() {
    let between = 2.0 * door_post_gap() - DOOR_POST_W_M;
    assert!(
        (door_opening_w() - between).abs() < 1e-4,
        "lintel spans {} but the posts leave {between}",
        door_opening_w()
    );
}

// ---------------------------------------------------------------------------
// The shared parts table's own emit (`structures::shape_parts`).
//
// The tests above pin the CONSTANTS; these read what the table actually
// EMITS — the same parts the ghost scales a unit cube to and `spawn_piece`
// builds meshes from, from the one function both call. They exist because
// the constants were never the hole: on 2026-08-09 the ghost and the piece
// shared every number and the ghost still drew the doorway's lintel at waist
// height (centre 1.05 m against the piece's 2.55 m) and previewed stairs as
// a level plate against the piece's pitched ramp, because which parts exist
// and where they go was a second, ungated copy. `no_sampled_point_…` above
// could not see either, since `drawn_posts` restates the layout by hand.

/// Where the table's doorway posts span, as `t` along the edge — read off the
/// emitted parts rather than restated.
fn table_posts() -> Vec<(f32, f32)> {
    let (parts, n) = shape_parts(sim_core::build::SHAPE_DOORWAY);
    let mid = BUILD_CELL_M * 0.5;
    parts[..n]
        .iter()
        .filter(|p| (p.size.y - LEVEL_H_M).abs() < 1e-4) // full height = a post
        .map(|p| {
            let c = mid + p.offset.z;
            (c - p.size.z * 0.5, c + p.size.z * 0.5)
        })
        .collect()
}

/// The emitted doorway agrees with the sim about solid and open, sampled the
/// way `no_sampled_point_disagrees_with_the_sim` samples — but off the
/// table's emit instead of a hand-restated layout.
#[test]
fn the_tables_doorway_is_the_sims_doorway() {
    let posts = table_posts();
    assert_eq!(posts.len(), 2, "a doorway emits exactly two posts");
    let solid = |t: f32| posts.iter().any(|(a, b)| t >= *a && t <= *b);
    let seams = [
        0.0,
        DOOR_POST_W_M,
        BUILD_CELL_M - DOOR_POST_W_M,
        BUILD_CELL_M,
    ];
    let mut checked = 0;
    for i in 0..=600 {
        let t = BUILD_CELL_M * (i as f32 / 600.0);
        if seams.iter().any(|s| (t - s).abs() <= SEAM_M) {
            continue;
        }
        assert_eq!(
            solid(t),
            doorway_solid_at(t),
            "at t={t}: table-solid={} but sim-blocks={}",
            solid(t),
            doorway_solid_at(t)
        );
        checked += 1;
    }
    assert!(checked > 500, "only {checked} samples were compared");
}

/// The emitted lintel caps the opening: its underside is the 2.1 m the
/// opening derivation on `LINTEL_H_M` states, a 1.8 m player clears it, and
/// its top is the piece's own top. **This is the assertion the old ghost
/// failed** — its copy hung the lintel at waist height, underside 0.6 m.
#[test]
fn the_tables_lintel_caps_the_opening() {
    let (parts, n) = shape_parts(sim_core::build::SHAPE_DOORWAY);
    let lintels: Vec<_> = parts[..n]
        .iter()
        .filter(|p| (p.size.y - LINTEL_H_M).abs() < 1e-4)
        .collect();
    assert_eq!(lintels.len(), 1, "a doorway emits exactly one lintel");
    let lintel = lintels[0];
    let underside = lintel.offset.y - lintel.size.y * 0.5;
    assert!(
        (underside - (LEVEL_H_M - LINTEL_DROP_M - LINTEL_H_M * 0.5)).abs() < 1e-4,
        "the lintel's underside is {underside}, not the derived opening top"
    );
    assert!(
        underside >= 1.8,
        "the doorway is {underside} m clear — a 1.8 m player cannot walk through"
    );
    let top = lintel.offset.y + lintel.size.y * 0.5;
    assert!(
        (top - LEVEL_H_M).abs() < 1e-4,
        "the lintel's top is {top}, not the piece's {LEVEL_H_M}"
    );
    assert!(
        (lintel.size.z - door_opening_w()).abs() < 1e-4,
        "the lintel spans {}, not the opening's {}",
        lintel.size.z,
        door_opening_w()
    );
}

/// Drawn solidity of an edge shape's emitted parts at `(t, y)` — `t` metres
/// along the edge, `y` metres above the storey base. The window and frame
/// gates sample this against the sim's own `*_solid_at`, which is the
/// doorway pair's discipline extended to the axis those two shapes
/// actually vary on: height.
fn table_solid_at(shape: u8, t: f32, y: f32) -> bool {
    let (parts, n) = shape_parts(shape);
    let mid = BUILD_CELL_M * 0.5;
    parts[..n].iter().any(|p| {
        let c = mid + p.offset.z;
        t >= c - p.size.z * 0.5
            && t <= c + p.size.z * 0.5
            && y >= p.offset.y - p.size.y * 0.5
            && y <= p.offset.y + p.size.y * 0.5
    })
}

/// The emitted window agrees with `collide::window_solid_at` everywhere
/// but the seam bands: sill solid, header solid, jambs solid, aperture
/// open. This is the gate `collide.rs` promises in `window_solid_at`'s
/// own doc — the drawn hole IS the hole an arrow threads.
#[test]
fn the_tables_window_is_the_sims_window() {
    let t_seams = [
        0.0,
        DOOR_POST_W_M,
        BUILD_CELL_M - DOOR_POST_W_M,
        BUILD_CELL_M,
    ];
    let y_seams = [WINDOW_SILL_M, WINDOW_HEAD_M];
    let mut checked = 0;
    for i in 0..=150 {
        let t = BUILD_CELL_M * (i as f32 / 150.0);
        if t_seams.iter().any(|s| (t - s).abs() <= SEAM_M) {
            continue;
        }
        for j in 0..150 {
            let y = LEVEL_H_M * (j as f32 + 0.5) / 150.0;
            if y_seams.iter().any(|s| (y - s).abs() <= SEAM_M) {
                continue;
            }
            assert_eq!(
                table_solid_at(sim_core::build::SHAPE_WINDOW, t, y),
                window_solid_at(t, y),
                "at (t={t}, y={y}): drawn-solid={} but sim-solid={}",
                table_solid_at(sim_core::build::SHAPE_WINDOW, t, y),
                window_solid_at(t, y)
            );
            checked += 1;
        }
    }
    assert!(checked > 15_000, "only {checked} samples were compared");
}

/// The emitted frame agrees with `collide::frame_solid_at`: rim jambs and
/// top beam solid, the rest of the edge open — to bodies and arrows both.
#[test]
fn the_tables_frame_is_the_sims_frame() {
    let t_seams = [0.0, FRAME_RIM_M, BUILD_CELL_M - FRAME_RIM_M, BUILD_CELL_M];
    let mut checked = 0;
    for i in 0..=150 {
        let t = BUILD_CELL_M * (i as f32 / 150.0);
        if t_seams.iter().any(|s| (t - s).abs() <= SEAM_M) {
            continue;
        }
        for j in 0..150 {
            let y = LEVEL_H_M * (j as f32 + 0.5) / 150.0;
            if (y - (LEVEL_H_M - FRAME_RIM_M)).abs() <= SEAM_M {
                continue;
            }
            assert_eq!(
                table_solid_at(sim_core::build::SHAPE_FRAME, t, y),
                frame_solid_at(t, y),
                "at (t={t}, y={y}): drawn-solid={} but sim-solid={}",
                table_solid_at(sim_core::build::SHAPE_FRAME, t, y),
                frame_solid_at(t, y)
            );
            checked += 1;
        }
    }
    assert!(checked > 15_000, "only {checked} samples were compared");
}

/// Every shape the wire can name emits something drawable: at least one part,
/// every part a real box, nothing above the piece's own top. Bounds, not
/// layout — the layout tests are the two above.
#[test]
fn every_shape_emits_a_drawable_part_set() {
    for shape in 0..=(N_SHAPES as u8) {
        let (parts, n) = shape_parts(shape);
        assert!(
            (1..=MAX_PARTS).contains(&n),
            "shape {shape} emits {n} parts"
        );
        for part in &parts[..n] {
            assert!(
                part.size.x > 0.0 && part.size.y > 0.0 && part.size.z > 0.0,
                "shape {shape} emits a degenerate box {:?}",
                part.size
            );
            let top = part.offset.y + part.size.y * 0.5;
            assert!(
                top <= LEVEL_H_M + 1e-4,
                "shape {shape} draws to {top}, above its own level"
            );
        }
    }
}

/// The stairs preview is pitched like the piece: the one part that carries a
/// non-zero pitch is the ramp, and every other shape is level. The old ghost
/// previewed the ramp as a horizontal plate — a preview of a different piece.
#[test]
fn only_the_stairs_carry_a_pitch_and_they_do_carry_one() {
    for shape in 0..(N_SHAPES as u8) {
        let (parts, n) = shape_parts(shape);
        for part in &parts[..n] {
            if shape == sim_core::build::SHAPE_STAIRS {
                assert!(
                    part.x_rot != 0.0,
                    "the stairs ramp is emitted level — the preview lies about the piece"
                );
            } else {
                assert_eq!(part.x_rot, 0.0, "shape {shape} is pitched and must not be");
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The deploy verdict against the sim's own refusal ladder (`NOW.md` §0u
// item 2).
//
// The build-ghost tests above check the client's DRAWING against the sim's
// collision; these check the client's WHETHER against the sim's refusals —
// `place::deploy_verdict` and `deploy::place_deploy` driven on the same
// constructed state, asserting agreement. The client side reads the stores'
// own `entries()` slices, which is byte-for-byte what the wire's mirror
// would hold (minus `owner`, which the wire never carries — the reason the
// bag cap is in the neutral set below).
//
// Two directions are pinned, and the second is the sharper one:
// - every MIRRORED refusal goes red with the refusal table's own sentence;
// - every refusal the mirror cannot see — the claim, the overlap, the caps —
//   stays `Unknown`, so a mutant that starts colouring them (off state the
//   client does not really hold) goes red here.

use client::render::structures::{deploy_size, deploy_transform, level_base_y};
use client::ui::place::{deploy_verdict, DeploySite, DeployVerdict, Target};
use sim_core::build::{BuildContent, Pieces, LOC_EDGE_XLO, LOC_PLANE};
use sim_core::deploy::{
    place_deploy, DeployContent, DeployDef, Deploys, ARCH_BOX, ARCH_DOOR, BAG_CAP, PLACE_GROUND,
    REFUSE_D_BAG_CAP, REFUSE_D_CLAIM, REFUSE_D_COST, REFUSE_D_HAS_LOCK, REFUSE_D_KIND,
    REFUSE_D_OVERLAP, REFUSE_D_REACH, REFUSE_D_SPOT, REFUSE_D_SUPPORT, REFUSE_D_TERRAIN,
};
use sim_core::gather::ItemStack;
use sim_core::movement::{Body, POS_XZ_Q};
use sim_core::world::{EventQueue, Player, EV_DEPLOY_REFUSED, EV_PIECE_PLACED};

/// `deploy.rs`'s own fixture seed and cells — buildable ground, proven by
/// the sim's own tests on the same numbers.
const SEED: u64 = 20260731;
const CX: u16 = 341;
const CZ: u16 = 341;

/// Fixture rows of `DeployContent::probe_fixture`, named.
const ROW_HEARTH: u16 = 0; // foundation class
const ROW_DOOR: u16 = 2; // doorway class
const ROW_BAG: u16 = 3; // ground class
const ROW_LOCK: u16 = 5; // door class

struct Rig {
    dc: DeployContent,
    bc: BuildContent,
    pieces: Pieces,
    deploys: Deploys,
}

impl Rig {
    fn new() -> Self {
        Self {
            dc: DeployContent::probe_fixture(),
            bc: BuildContent::probe_fixture(),
            pieces: Pieces::new(),
            deploys: Deploys::new(),
        }
    }

    /// Drive the SIM's own verb. `Err(reason)` on a refusal, `Ok` on a
    /// placement (a lock's success announces `EV_DOOR` rather than
    /// `EV_DEPLOY_PLACED`, so the discriminator is the refusal, not the ack).
    fn sim(&mut self, p: &mut Player, row: u16, t: Target) -> Result<(), u32> {
        let mut ev = EventQueue::default();
        place_deploy(
            SEED,
            &self.dc,
            &self.bc,
            &mut self.pieces,
            &mut self.deploys,
            p,
            0,
            row,
            t.cx,
            t.cz,
            t.level,
            t.loc,
            &mut ev,
        );
        match ev.entries().iter().find(|e| e.code == EV_DEPLOY_REFUSED) {
            Some(e) => Err(e.b),
            None => Ok(()),
        }
    }

    /// The CLIENT's verdict on the same state, through the mirror's shape.
    fn client(&self, p: &Player, row: u16, t: Target) -> DeployVerdict {
        deploy_verdict(
            t,
            row as u8,
            &DeploySite {
                seed: SEED,
                at: (p.body.qx as f32 * POS_XZ_Q, p.body.qz as f32 * POS_XZ_Q),
                pieces: self.pieces.entries(),
                piece_defs: &self.bc,
                piece_have: self.bc.piece_count,
                deploys: self.deploys.entries(),
                deploy_defs: &self.dc,
                deploy_have: self.dc.def_count,
                inv: &p.inv,
            },
        )
    }

    /// Both sides on one state: the client computed FIRST (a sim success
    /// mutates the stores), then the sim, then the agreement asserts.
    fn agree_no(&mut self, p: &mut Player, row: u16, t: Target, code: u32, sentence: &str) {
        let said = self.client(p, row, t);
        let sim = self.sim(p, row, t);
        assert_eq!(sim, Err(code), "the sim did not refuse with code {code}");
        assert_eq!(
            said,
            DeployVerdict::No(client::ui::refusals::DEPLOY[code as usize]),
            "client and sim disagree for code {code}"
        );
        assert_eq!(said.why(), sentence, "the sentence drifted from the table");
    }

    /// A placement the sim takes must NEVER have been red — the module's
    /// forbidden direction — and can only ever have been `Unknown`.
    fn agree_ok(&mut self, p: &mut Player, row: u16, t: Target) {
        let said = self.client(p, row, t);
        assert_eq!(
            said,
            DeployVerdict::Unknown,
            "red (or worse) on a placement the sim accepts"
        );
        assert_eq!(self.sim(p, row, t), Ok(()), "the fixture placement failed");
    }

    /// A refusal the mirror cannot see must stay neutral: the sim says no,
    /// the client says `Unknown` — a mutant that colours it goes red here.
    fn agree_unknown(&mut self, p: &mut Player, row: u16, t: Target, code: u32) {
        let said = self.client(p, row, t);
        let sim = self.sim(p, row, t);
        assert_eq!(sim, Err(code), "the sim did not refuse with code {code}");
        assert_eq!(
            said,
            DeployVerdict::Unknown,
            "code {code} is deliberately unknowable client-side and must stay neutral"
        );
    }

    /// A foundation piece at the cell, by the sim's own build verb.
    fn founded(&mut self, p: &mut Player, cx: u16, cz: u16) {
        self.piece(p, 0, cx, cz, sim_core::build::LOC_PLANE);
    }

    fn piece(&mut self, p: &mut Player, row: u16, cx: u16, cz: u16, loc: u8) {
        let mut ev = EventQueue::default();
        sim_core::build::place(
            SEED,
            &self.bc,
            &self.deploys,
            &mut self.pieces,
            p,
            0,
            row,
            cx,
            cz,
            0,
            loc,
            &mut ev,
        );
        let last = ev.entries().last().expect("place answers");
        assert_eq!(
            last.code, EV_PIECE_PLACED,
            "fixture piece refused: {}",
            last.b
        );
    }
}

fn player_at_cell(cx: u16, cz: u16, items: &[(u16, u16)]) -> Player {
    let mut p = Player {
        id: 7,
        active: true,
        body: Body::at(
            SEED,
            (cx as f32 + 0.5) * BUILD_CELL_M,
            (cz as f32 + 0.5) * BUILD_CELL_M,
        ),
        ..Player::default()
    };
    for (i, &(item, count)) in items.iter().enumerate() {
        p.inv[i] = ItemStack { item, count };
    }
    p
}

/// A hand stocked for everything the fixture can place: build materials
/// (0, 1), the hearth (2), the door (4), the bag (5) and the lock (7).
fn rich(cx: u16, cz: u16) -> Player {
    player_at_cell(cx, cz, &[(0, 99), (1, 99), (2, 9), (4, 9), (5, 20), (7, 9)])
}

fn at(cx: u16, cz: u16, level: u8, loc: u8) -> Target {
    Target { cx, cz, level, loc }
}

#[test]
fn a_taken_spot_is_red_on_both_sides() {
    let mut rig = Rig::new();
    let mut p = rich(CX, CZ);
    // The first bag places — and was never red.
    rig.agree_ok(&mut p, ROW_BAG, at(CX, CZ, 0, LOC_PLANE));
    // The second refuses the occupied address, both sides.
    rig.agree_no(
        &mut p,
        ROW_BAG,
        at(CX, CZ, 0, LOC_PLANE),
        REFUSE_D_SPOT,
        "spot taken",
    );
    // And the loc rule is the same function on both sides: a door named at
    // the cell body is not a slot a doorway-class deployable occupies.
    rig.agree_no(
        &mut p,
        ROW_DOOR,
        at(CX + 1, CZ, 0, LOC_PLANE),
        REFUSE_D_SPOT,
        "spot taken",
    );
}

#[test]
fn out_of_reach_is_red_on_both_sides() {
    let mut rig = Rig::new();
    let mut p = rich(CX, CZ);
    // Five cells is 15 m of cell-centre distance against a 5 m reach.
    rig.agree_no(
        &mut p,
        ROW_BAG,
        at(CX + 5, CZ, 0, LOC_PLANE),
        REFUSE_D_REACH,
        "out of reach",
    );
}

#[test]
fn needs_support_is_red_on_both_sides() {
    let mut rig = Rig::new();
    let mut p = rich(CX, CZ);
    // A hearth (foundation class) on bare terrain.
    rig.agree_no(
        &mut p,
        ROW_HEARTH,
        at(CX, CZ, 0, LOC_PLANE),
        REFUSE_D_SUPPORT,
        "needs support",
    );
    // A door at an empty edge.
    rig.agree_no(
        &mut p,
        ROW_DOOR,
        at(CX, CZ, 0, LOC_EDGE_XLO),
        REFUSE_D_SUPPORT,
        "needs support",
    );
    // A wall is not a doorway — the shape check runs against the piece-def
    // table on both sides.
    rig.founded(&mut p, CX, CZ);
    rig.piece(&mut p, 1, CX, CZ, LOC_EDGE_XLO); // row 1 = wood wall
    rig.agree_no(
        &mut p,
        ROW_DOOR,
        at(CX, CZ, 0, LOC_EDGE_XLO),
        REFUSE_D_SUPPORT,
        "needs support",
    );
    // A lock at an address holding nothing it bolts to.
    rig.agree_no(
        &mut p,
        ROW_LOCK,
        at(CX + 1, CZ, 0, LOC_PLANE),
        REFUSE_D_SUPPORT,
        "needs support",
    );
}

#[test]
fn bad_ground_is_red_on_both_sides() {
    let mut rig = Rig::new();
    let mut p = rich(CX, CZ);
    // Ground class refuses a cell whose body holds a piece.
    rig.founded(&mut p, CX, CZ);
    rig.agree_no(
        &mut p,
        ROW_BAG,
        at(CX, CZ, 0, LOC_PLANE),
        REFUSE_D_TERRAIN,
        "bad ground",
    );
    // ...and a cell whose terrain a foundation would refuse — found by the
    // sim's own predicate, because this test must not guess where the sea
    // is. Walking out from the island's build area, the first unbuildable
    // cell is the shared answer.
    let unbuildable = (2..CX)
        .map(|k| (CX - k, CZ))
        .find(|&(cx, cz)| {
            let (ax, az) = sim_core::deploy::cell_center(cx, cz);
            !sim_core::build::foundation_terrain_ok(SEED, ax, az)
        })
        .expect("an island has an edge");
    let mut swimmer = rich(unbuildable.0, unbuildable.1);
    rig.agree_no(
        &mut swimmer,
        ROW_BAG,
        at(unbuildable.0, unbuildable.1, 0, LOC_PLANE),
        REFUSE_D_TERRAIN,
        "bad ground",
    );
}

#[test]
fn an_empty_pocket_is_red_on_both_sides() {
    let mut rig = Rig::new();
    // No items at all: the bag's own item is not in the hand.
    let mut poor = player_at_cell(CX, CZ, &[]);
    rig.agree_no(
        &mut poor,
        ROW_BAG,
        at(CX, CZ, 0, LOC_PLANE),
        REFUSE_D_COST,
        "item not in inventory",
    );
    // The lock's cost arm is its own path in the sim; same agreement.
    let mut rich_hand = rich(CX, CZ);
    rig.founded(&mut rich_hand, CX, CZ);
    rig.piece(&mut rich_hand, 3, CX, CZ, LOC_EDGE_XLO); // row 3 = doorway
    rig.agree_ok(&mut rich_hand, ROW_DOOR, at(CX, CZ, 0, LOC_EDGE_XLO));
    let mut no_lock = player_at_cell(CX, CZ, &[(0, 9)]);
    rig.agree_no(
        &mut no_lock,
        ROW_LOCK,
        at(CX, CZ, 0, LOC_EDGE_XLO),
        REFUSE_D_COST,
        "item not in inventory",
    );
}

#[test]
fn a_bad_row_is_red_on_both_sides() {
    let mut rig = Rig::new();
    let mut p = rich(CX, CZ);
    // Past the table.
    rig.agree_no(
        &mut p,
        9,
        at(CX, CZ, 0, LOC_PLANE),
        REFUSE_D_KIND,
        "no such deployable",
    );
    // A declared row with no hp is the inert row; the sim refuses it and
    // the client sees the same field.
    rig.dc.defs[6] = DeployDef {
        arch: ARCH_BOX,
        placement: PLACE_GROUND,
        hp: 0,
        item: 9,
        n_costs: 0,
        costs: [(0, 0); sim_core::limits::MAX_DEPLOY_COSTS],
    };
    rig.dc.def_count = 7;
    p.inv[8] = ItemStack { item: 9, count: 1 };
    rig.agree_no(
        &mut p,
        6,
        at(CX, CZ, 0, LOC_PLANE),
        REFUSE_D_KIND,
        "no such deployable",
    );
}

#[test]
fn a_second_lock_is_red_on_both_sides() {
    let mut rig = Rig::new();
    let mut p = rich(CX, CZ);
    rig.founded(&mut p, CX, CZ);
    rig.piece(&mut p, 3, CX, CZ, LOC_EDGE_XLO); // doorway
    rig.agree_ok(&mut p, ROW_DOOR, at(CX, CZ, 0, LOC_EDGE_XLO));
    // The first lock bolts on (its success is EV_DOOR, not a placed ack).
    rig.agree_ok(&mut p, ROW_LOCK, at(CX, CZ, 0, LOC_EDGE_XLO));
    // The second is refused by the lock store — mirrored client-side as the
    // `has_lock` bit the wire keeps in lockstep with it.
    rig.agree_no(
        &mut p,
        ROW_LOCK,
        at(CX, CZ, 0, LOC_EDGE_XLO),
        REFUSE_D_HAS_LOCK,
        "that door already has a lock",
    );
}

/// **The deliberately-unknown reasons stay neutral.** The claim needs crew
/// lists the wire never carries; the overlap needs the claim walk over the
/// sim's own stores; the bag cap counts by an owner field the wire never
/// carries. For each: the sim refuses, and the client's verdict must be
/// `Unknown` — a mutant that colours any of them goes red here, because the
/// only way to colour them is to guess off state the mirror does not hold.
#[test]
fn the_unknowable_refusals_stay_neutral() {
    let mut rig = Rig::new();
    let mut own = rich(CX, CZ);
    rig.founded(&mut own, CX, CZ);
    rig.agree_ok(&mut own, ROW_HEARTH, at(CX, CZ, 0, LOC_PLANE));

    // REFUSE_D_CLAIM: a stranger inside the hearth's claim.
    let mut foe = rich(CX + 2, CZ);
    foe.id = 8;
    rig.agree_unknown(
        &mut foe,
        ROW_BAG,
        at(CX + 2, CZ, 0, LOC_PLANE),
        REFUSE_D_CLAIM,
    );

    // REFUSE_D_OVERLAP: a second hearth inside the first's claim, own crew
    // or not.
    rig.piece(&mut own, 0, CX + 1, CZ, LOC_PLANE);
    rig.agree_unknown(
        &mut own,
        ROW_HEARTH,
        at(CX + 1, CZ, 0, LOC_PLANE),
        REFUSE_D_OVERLAP,
    );
}

#[test]
fn the_bag_cap_stays_neutral() {
    // Its own rig: no hearth, so the claim never preempts the cap.
    let mut rig = Rig::new();
    let mut p = player_at_cell(CX, CZ, &[(5, 20)]);
    for k in 0..BAG_CAP as u16 {
        p.body = Body::at(
            SEED,
            (CX + k) as f32 * BUILD_CELL_M + 1.5,
            CZ as f32 * BUILD_CELL_M + 1.5,
        );
        rig.agree_ok(&mut p, ROW_BAG, at(CX + k, CZ, 0, LOC_PLANE));
    }
    let k = BAG_CAP as u16;
    p.body = Body::at(
        SEED,
        (CX + k) as f32 * BUILD_CELL_M + 1.5,
        CZ as f32 * BUILD_CELL_M + 1.5,
    );
    rig.agree_unknown(
        &mut p,
        ROW_BAG,
        at(CX + k, CZ, 0, LOC_PLANE),
        REFUSE_D_BAG_CAP,
    );
}

// ---------------------------------------------------------------------------
// The door in its edge (`NOW.md` §0u item 3): the ghost's pose comes from
// `structures::deploy_transform` — the one emit site the standing deployable
// uses — so what these pin is that emit site, and the seam pins the rest.

/// A closed door at a doorway edge stands ON the edge — its centre on the
/// cell boundary, half its height above the level base — not on the cell
/// body where the old ghost previewed it.
#[test]
fn the_door_ghost_stands_in_the_doorway_edge() {
    let size = deploy_size(ARCH_DOOR as usize);
    let base = level_base_y(SEED, CX, CZ, 0);

    let xlo = deploy_transform(SEED, (CX, CZ, 0, LOC_EDGE_XLO), ARCH_DOOR, false);
    assert!(
        (xlo.translation.x - CX as f32 * BUILD_CELL_M).abs() < 1e-4,
        "a low-x door's centre is not on the low-x boundary"
    );
    assert!(
        (xlo.translation.z - (CZ as f32 + 0.5) * BUILD_CELL_M).abs() < 1e-4,
        "a low-x door is not centred along its edge"
    );
    assert!(
        (xlo.translation.y - (base + size.y * 0.5)).abs() < 1e-4,
        "the door does not stand on the level base"
    );

    // The low-z edge is the same door under the quarter-turn — the edge
    // canonicalisation the grid uses everywhere else.
    let zlo = deploy_transform(
        SEED,
        (CX, CZ, 0, sim_core::build::LOC_EDGE_ZLO),
        ARCH_DOOR,
        false,
    );
    assert!(
        (zlo.translation.z - CZ as f32 * BUILD_CELL_M).abs() < 1e-4,
        "a low-z door's centre is not on the low-z boundary"
    );
    let turned = zlo.rotation * bevy::math::Vec3::X;
    assert!(
        (turned.z + 1.0).abs() < 1e-4,
        "a low-z door does not carry the quarter-turn"
    );

    // A body deployable keeps the cell body: the split is the door's alone.
    let body = deploy_transform(SEED, (CX, CZ, 0, LOC_PLANE), ARCH_BOX, false);
    assert!(
        (body.translation.x - (CX as f32 + 0.5) * BUILD_CELL_M).abs() < 1e-4,
        "a box left the cell centre"
    );

    // And an open door is drawn elsewhere than a closed one — the leaf
    // swings, exactly as the sim's collision reads it.
    let open = deploy_transform(SEED, (CX, CZ, 0, LOC_EDGE_XLO), ARCH_DOOR, true);
    assert!(
        (open.translation - xlo.translation).length() > 0.1,
        "an open door is drawn shut"
    );
}

/// The door's box fits the opening the sim actually leaves — width inside
/// the posts' clear span, top at or under the lintel's underside. The
/// arithmetic gate `CLAUDE.md` allows: the mesh fits the volume the sim
/// blocks out.
#[test]
fn the_door_fits_the_opening_the_sim_leaves() {
    let size = deploy_size(ARCH_DOOR as usize);
    assert!(
        size.z <= door_opening_w() + 1e-4,
        "the door leaf ({}) is wider than the opening ({})",
        size.z,
        door_opening_w()
    );
    let opening_top = LEVEL_H_M - LINTEL_DROP_M - LINTEL_H_M * 0.5;
    assert!(
        size.y <= opening_top + 1e-4,
        "the door leaf ({}) is taller than the opening ({opening_top})",
        size.y
    );
}
