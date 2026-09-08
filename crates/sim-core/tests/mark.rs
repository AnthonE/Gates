//! `test_mark` — the gate for **where a melee swing leaves its scuff on a
//! built piece**.
//!
//! A hatchet swung at a tree has marked the bark since 2026-08-25 and a
//! hatchet swung at a wall since 2026-08-28 (`NOW.md` §0mk item 1: the shard
//! took the right hp off the right record and the raider could not see it).
//! Until melee aim v1 the wall's mark was `combat::piece_mark` — a centroid
//! on the piece's surface nearest the raider's *stance*, because the planar
//! pick had no point of contact to offer. This file held that function's ten
//! `loc` shapes to the piece.
//!
//! **Since 2026-09-05 there is a point of contact.** A swing is a ray from
//! the eye along the look direction (`sim-core/src/melee.rs`), the world is
//! walked by the same `ranged::world_stop` an arrow and a bullet are walked
//! by, and the mark is the ray's own stop — the sample at which the swing met
//! the plank, up to one `ARROW_STEP_MM` inside its face, exactly where an
//! arrow's mark sits and where `render/decal.rs` already draws one facing the
//! shooter. So the claim this file makes changed shape: not *the centroid is
//! on the piece* but **the mark is where you aimed, and it moves when you
//! do**. Three things follow and each is checked from outside the code that
//! produces them:
//!
//! 1. **On the face.** The mark's x is within the probe's forgiveness in
//!    front of the wall's plane and within one sample behind it; a mark on
//!    the cell centre, or at the raider's feet, is off by metres.
//! 2. **At the aim's height and bearing.** Level, the mark is at eye height
//!    on the raider's own z; pitched down it drops, pitched up it climbs,
//!    yawed it slides along the face — and stays inside the storey the wall
//!    spans. This is the assertion the stance centroid could never satisfy,
//!    and the whole reason the operator's *"proper video game"* was owed.
//! 3. **Over the wall is over the wall.** A swing pitched above the storey
//!    marks nothing built and charges nothing; one pitched at the ground
//!    marks the ground. The old scan hit the wall from any stance in the
//!    cone whatever the pitch, because there was no pitch.
//!
//! `EV_IMPACT` is not in `state_hash` (replay green), moved no wire byte
//! (golden green), and all three fields are `u32` (clippy green) — so, as
//! before, nothing but this file can see the *value*. `tests/event_roles.rs`
//! still checks which field is which.

use sim_core::build::{
    foundation_terrain_ok, BuildContent, BUILD_CELL_M, LEVEL_H_M, LOC_EDGE_XLO, LOC_PLANE,
};
use sim_core::collide::WALL_THICKNESS_M;
use sim_core::combat::CombatContent;
use sim_core::deploy::DeployContent;
use sim_core::gather::{GatherContent, ItemStack};
use sim_core::input::{InputFrame, BTN_PRIMARY};
use sim_core::limits::ARROW_STEP_MM;
use sim_core::melee::MELEE_PROBE_M;
use sim_core::movement::{Body, POS_XZ_Q, POS_Y_Q};
use sim_core::ranged::{ARROW_EYE_MM, SURF_BUILT, SURF_GROUND};
use sim_core::world::{Command, SimEvent, World, EV_IMPACT, EV_STRUCT_HIT, EV_SWING};

const SEED: u64 = 20260802;
/// The raider's network id — `event_roles.rs`' `BUILDER`, and the same
/// player builds and swings here so the wall's soft side faces them.
const RAIDER: u32 = 4;
/// `BuildContent::probe_fixture`'s rows: 0 is the foundation, 1 the wall.
const PIECE_FOUNDATION: u16 = 0;
const PIECE_WALL: u16 = 1;
const GROUND: u8 = 0;
/// `CombatContent::probe_fixture`'s item 0: 34 body, 34 structure, 2 m.
const CLUB: u16 = 0;
const CLUB_STRUCTURE: u32 = 34;
/// `BuildContent::probe_fixture`'s wall hp.
const WALL_HP: u32 = 100;
/// Wire yaw facing +X — LUT index 64 of 256 (`yaw_lut.rs`: index 0 is +Z,
/// increasing index rotates toward +X). The raider stands west of the wall
/// and looks east at it.
const YAW_PLUS_X: u16 = 64 << 8;
/// How far west of the wall's face the raider stands, metres — inside the
/// club's 2 m reach with room for the pitched cases to still land.
const STANCE_M: f32 = 1.5;
const EYE_M: f32 = ARROW_EYE_MM as f32 / 1000.0;
/// The sampler's spacing: the most a mark can sit *inside* the face.
const SAMPLE_STEP_M: f32 = ARROW_STEP_MM as f32 / 1000.0;
const MAX_STEPS: u32 = 200;

fn hv(seed: u64) -> &'static sim_core::terrain::Haven {
    use std::cell::RefCell;
    thread_local! {
        static CACHE: RefCell<Vec<(u64, &'static sim_core::terrain::Haven)>> =
            const { RefCell::new(Vec::new()) };
    }
    let hit = CACHE.with(|c| c.borrow().iter().find(|(s, _)| *s == seed).map(|&(_, h)| h));
    if let Some(h) = hit {
        return h;
    }
    let h: &'static sim_core::terrain::Haven = Box::leak(Box::new(sim_core::terrain::haven(seed)));
    CACHE.with(|c| c.borrow_mut().push((seed, h)));
    h
}

/// Ask the sim's own rule for a buildable cell rather than typing a
/// coordinate — `chip.rs`' helper, and the reason worldgen moved under a
/// hand-typed cell twice.
///
/// **And a quiet one**: nothing the scatter put down within a swing's reach
/// of the raider's stance. A swing is a ray, and a bush or a rock standing
/// beside the wall is what the ray meets when the yawed cases below turn
/// eleven degrees — the first draft lost its `right lands` case to exactly
/// that, a scatter occupant it had never been told about.
fn buildable_cell(seed: u64) -> (u16, u16) {
    let table = sim_core::terrain::ScatterTable::alpha_default();
    for r in 0..64i32 {
        for dz in -r..=r {
            for dx in -r..=r {
                if dx.abs() != r && dz.abs() != r {
                    continue;
                }
                let cx = (170 + dx).clamp(0, 1023) as u16;
                let cz = (170 + dz).clamp(0, 1023) as u16;
                if cx == cz {
                    continue;
                }
                let (x, z) = (
                    (cx as f32 + 0.5) * BUILD_CELL_M,
                    (cz as f32 + 0.5) * BUILD_CELL_M,
                );
                if !foundation_terrain_ok(seed, hv(seed), x, z) {
                    continue;
                }
                // The stance and everything within a long swing of it.
                let (sx, sz) = (cx as f32 * BUILD_CELL_M - STANCE_M, z);
                let (pcx, pcz) = (
                    (sx / sim_core::terrain::CELL_SIZE) as i32,
                    (sz / sim_core::terrain::CELL_SIZE) as i32,
                );
                let mut quiet = true;
                for ddz in -1..=1i32 {
                    for ddx in -1..=1i32 {
                        let s = sim_core::terrain::scatter(
                            seed,
                            &table,
                            hv(seed),
                            pcx + ddx,
                            pcz + ddz,
                        );
                        let (r_m, _) = sim_core::melee::swing_volume(s.occupant);
                        if r_m <= 0.0 {
                            continue;
                        }
                        let d2 = (s.x - sx) * (s.x - sx) + (s.z - sz) * (s.z - sz);
                        if d2 < (3.5 + r_m) * (3.5 + r_m) {
                            quiet = false;
                        }
                    }
                }
                if quiet {
                    return (cx, cz);
                }
            }
        }
    }
    panic!("no quiet buildable cell within 64 cells — the generator changed under this test");
}

/// The pitch byte pointing closest along `(run, rise)`, off the sim's own
/// LUT — no trig, because the crate's clippy walls bind this suite.
fn pitch_toward(rise: f32, run: f32) -> u8 {
    let len = (rise * rise + run * run).sqrt();
    let mut best = 128u8;
    let mut best_dot = f32::MIN;
    for b in 0..=255u8 {
        let (ch, sv) = sim_core::pitch_dir(b);
        let dot = (ch * run + sv * rise) / len;
        if dot > best_dot {
            best_dot = dot;
            best = b;
        }
    }
    best
}

fn place(w: &mut World, row: u16, cx: u16, cz: u16, level: u8, loc: u8) {
    let before = w.pieces.len();
    w.tick(&[Command::Place {
        id: RAIDER,
        row,
        cx,
        cz,
        level,
        loc,
        freehand: false,
        // The band asked for (foundation height v0): 0 is the column's own
        // ground where this fixture starts one, and ignored where it builds
        // into a column that already stands — the v61 behaviour either way.
        plate: 0,
    }]);
    assert_eq!(
        w.pieces.len(),
        before + 1,
        "piece row {row} did not place at ({cx}, {cz}) level {level} loc {loc} — \
         the fixture, not the mechanic"
    );
}

/// Stand the wall back up if the swings so far have felled it. Three club
/// hits (`CLUB_STRUCTURE` × 3 ≥ `WALL_HP`) take it down, and the mark test
/// below swings four times at one wall: its fourth mark was measured
/// against a wall that no longer existed, and read as a swing that marked
/// nothing. Placed from the same stance, so the soft side stays toward the
/// raider and the damage assertions keep meaning what they say.
fn rewall(w: &mut World, cx: u16, cz: u16) {
    if w.pieces.find(cx, cz, GROUND, LOC_EDGE_XLO).is_none() {
        place(w, PIECE_WALL, cx, cz, GROUND, LOC_EDGE_XLO);
    }
}

/// A world with a foundation and one wall on its west edge, and a club-armed
/// raider standing [`STANCE_M`] west of that face on the ground there — not
/// pinned to the slab, because every `tick` steps the body under gravity and
/// a pinned raider 1.5 m outside the foundation falls between two swings,
/// which moved the eye the second mark was measured against. The wall's
/// storey starts at the column's plate, `PIECE_LIFT_M` over the cell's
/// terrain, so an eye 1.6 m over the ground beside it is inside the span.
///
/// **Takes `&mut World` and never returns one** — `event_roles.rs`' measured
/// rule: a `World` is ~440 kB and moving one out of a frame puts two in a
/// debug test thread's 2 MiB stack.
///
/// **The wall is placed from the stance**, `chip.rs`' rule and its reason:
/// hard/soft v0 puts a placement's soft side toward the placer, so a wall
/// built from inside and swung at from outside pays `HARD_SIDE_STRUCTURE`
/// and every damage assertion silently becomes a test of the side rule.
///
/// Returns the wall's cell and the x of its face.
fn walled(w: &mut World) -> ((u16, u16), f32) {
    w.gather = GatherContent::probe_fixture();
    w.combat = CombatContent::probe_fixture();
    w.build = BuildContent::probe_fixture();
    w.deploy = DeployContent::probe_fixture();
    w.tick(&[Command::Join { id: RAIDER }]);
    let (cx, cz) = buildable_cell(SEED);
    let x0 = cx as f32 * BUILD_CELL_M;
    let z_mid = (cz as f32 + 0.5) * BUILD_CELL_M;
    w.players[0].body = Body::at(SEED, hv(SEED), x0 - STANCE_M, z_mid);
    for (slot, item) in [(0usize, CLUB), (1, 1), (2, 2), (3, 4)] {
        w.players[0].inv[slot] = ItemStack {
            item,
            count: 200,
            cond: 0,
        };
    }
    place(w, PIECE_FOUNDATION, cx, cz, GROUND, LOC_PLANE);
    place(w, PIECE_WALL, cx, cz, GROUND, LOC_EDGE_XLO);
    // Let the body settle onto its ground before anything is measured
    // against its eye.
    for _ in 0..8 {
        w.tick(&[]);
    }
    let base = sim_core::build::column_floor_y(
        SEED,
        hv(SEED),
        cx,
        cz,
        w.pieces.cols().plate(cx, cz).unwrap_or(0),
    );
    let (eye_y, _) = eye(w);
    assert!(
        eye_y > base && eye_y < base + LEVEL_H_M,
        "fixture: the raider's eye ({eye_y:.2}) is outside the storey the wall \
         spans ({base:.2}..{:.2}) — pick another cell",
        base + LEVEL_H_M
    );
    ((cx, cz), x0)
}

/// Hold the primary button at `(yaw, pitch)` until a swing is taken, and
/// hand back that tick's events. Cadence, not the button, paces it.
fn swing(w: &mut World, yaw: u16, pitch: u8) -> Vec<SimEvent> {
    let mut seq = w.players[0].frame.seq;
    for _ in 0..MAX_STEPS {
        seq = seq.wrapping_add(1);
        w.tick(&[Command::Input {
            id: RAIDER,
            frame: InputFrame {
                seq,
                buttons: BTN_PRIMARY,
                yaw,
                pitch,
                move_x: 0,
                move_z: 0,
                sel: 0,
            },
            favour: 0,
        }]);
        if w.events.entries().iter().any(|e| e.code == EV_SWING) {
            return w.events.entries().to_vec();
        }
    }
    panic!("no swing was taken in {MAX_STEPS} ticks");
}

/// The one `EV_IMPACT` in a swing's events, unpacked to metres with its
/// surface class — or `None` for a swing that marked nothing.
fn impact(events: &[SimEvent]) -> Option<(u8, f32, f32, f32)> {
    let hits: Vec<_> = events.iter().filter(|e| e.code == EV_IMPACT).collect();
    assert!(hits.len() <= 1, "one swing left {} marks", hits.len());
    hits.first().map(|e| {
        (
            (e.a >> 24) as u8,
            (e.a & 0x00ff_ffff) as i32 as f32 * POS_XZ_Q,
            e.c as i32 as f32 * POS_Y_Q,
            e.b as i32 as f32 * POS_XZ_Q,
        )
    })
}

fn struct_hits(events: &[SimEvent]) -> usize {
    events.iter().filter(|e| e.code == EV_STRUCT_HIT).count()
}

/// The eye's height and z for the seated raider.
fn eye(w: &World) -> (f32, f32) {
    let b = w.players[0].body;
    (b.qy as f32 * POS_Y_Q + EYE_M, b.qz as f32 * POS_XZ_Q)
}

/// The mark is on the wall: the sampler stops on the first tap whose probe
/// reaches the slab, so the point sits at most the wall's half-thickness
/// plus the probe in front of the plane, and at most one sample behind it —
/// exactly where an arrow's mark sits (`tests/shoot.rs`).
fn assert_on_face(mx: f32, x0: f32, what: &str) {
    let lo = x0 - WALL_THICKNESS_M * 0.5 - MELEE_PROBE_M - 2.0 * POS_XZ_Q;
    let hi = x0 + SAMPLE_STEP_M + 2.0 * POS_XZ_Q;
    assert!(
        mx >= lo && mx <= hi,
        "{what}: mark x {mx:.3} is not on the wall's face at x {x0:.3} \
         (allowed {lo:.3}..{hi:.3})"
    );
}

#[test]
fn a_level_swing_marks_the_wall_at_eye_height_on_its_own_line() {
    let mut w = World::new(SEED);
    let (_, x0) = walled(&mut w);
    let (eye_y, eye_z) = eye(&w);
    let ev = swing(&mut w, YAW_PLUS_X, 128);
    let (surf, mx, my, mz) = impact(&ev).expect("a swing at a wall 1.5 m away leaves a mark");
    assert_eq!(
        surf, SURF_BUILT,
        "a struck plank is the built surface, not the ground"
    );
    assert_on_face(mx, x0, "level");
    assert!(
        (my - eye_y).max(eye_y - my) <= 2.0 * POS_Y_Q,
        "level swing marked at y {my:.3}, eye at {eye_y:.3}"
    );
    assert!(
        (mz - eye_z).max(eye_z - mz) <= 2.0 * POS_XZ_Q,
        "a swing straight along +x marked at z {mz:.3}, raider at {eye_z:.3}"
    );
    // And it was charged, at the soft-side price: the mark is a fact about
    // the same blow `World::chip` billed.
    let hit = ev
        .iter()
        .find(|e| e.code == EV_STRUCT_HIT)
        .expect("the swing that marked the wall also charged it");
    // `EV_STRUCT_HIT.c = damage << 16 | hp left` (`world.rs`).
    assert_eq!(
        hit.c >> 16,
        CLUB_STRUCTURE,
        "the soft side pays the club's whole structure column"
    );
    assert_eq!(
        hit.c & 0xffff,
        WALL_HP - CLUB_STRUCTURE,
        "and the wall keeps the rest"
    );
}

#[test]
fn the_mark_follows_the_aim_down_up_and_across() {
    let mut w = World::new(SEED);
    let ((cx, cz), x0) = walled(&mut w);
    let base = sim_core::build::column_floor_y(
        SEED,
        hv(SEED),
        cx,
        cz,
        w.pieces.cols().plate(cx, cz).unwrap_or(0),
    );

    // The eye is read before EACH swing: nothing moves the body between
    // them, and reading it once and trusting it is how the first draft of
    // this test measured a mark against an eye that had since fallen.
    let down = pitch_toward(-0.7, STANCE_M);
    let up = pitch_toward(0.7, STANCE_M);
    let (eye_y, _) = eye(&w);
    let (_, mx_down, y_down, _) = impact(&swing(&mut w, YAW_PLUS_X, down)).expect("down lands");
    assert_on_face(mx_down, x0, "down");
    assert!(
        y_down < eye_y - 0.4 && y_down >= base - 2.0 * POS_Y_Q,
        "aimed 25° down the mark sits at {y_down:.3} (eye {eye_y:.3}, storey base {base:.3})"
    );
    let (eye_y, eye_z) = eye(&w);
    rewall(&mut w, cx, cz);
    let (_, mx_up, y_up, _) = impact(&swing(&mut w, YAW_PLUS_X, up)).expect("up lands");
    assert!(
        y_up > eye_y + 0.4 && y_up <= base + LEVEL_H_M + 2.0 * POS_Y_Q,
        "aimed 25° up the mark sits at {y_up:.3} (eye {eye_y:.3}, storey top {:.3})",
        base + LEVEL_H_M
    );
    assert_on_face(mx_up, x0, "up");

    // Yawed eight LUT steps (11°) either side: the mark slides along the
    // face, on the side the yaw turned to. Index 0 is +Z and increasing
    // index rotates toward +X, so a smaller index than +X leans toward +Z.
    let left = YAW_PLUS_X.wrapping_sub(8 << 8);
    let right = YAW_PLUS_X.wrapping_add(8 << 8);
    rewall(&mut w, cx, cz);
    let (_, mx_l, _, z_l) = impact(&swing(&mut w, left, 128)).expect("left lands");
    rewall(&mut w, cx, cz);
    let (_, mx_r, _, z_r) = impact(&swing(&mut w, right, 128)).expect("right lands");
    assert_on_face(mx_l, x0, "yawed toward +z");
    assert_on_face(mx_r, x0, "yawed toward -z");
    assert!(
        z_l > eye_z + 0.15 && z_r < eye_z - 0.15,
        "yawing moved the mark to z {z_l:.3} and {z_r:.3} around {eye_z:.3} — \
         it should slide along the face with the aim"
    );
    // Still inside the wall's own cell along its face.
    let z0 = cz as f32 * BUILD_CELL_M;
    for (z, what) in [(z_l, "left"), (z_r, "right")] {
        assert!(
            z >= z0 - 2.0 * POS_XZ_Q && z <= z0 + BUILD_CELL_M + 2.0 * POS_XZ_Q,
            "{what}: mark z {z:.3} is off the wall's span {z0:.3}..{:.3}",
            z0 + BUILD_CELL_M
        );
    }
}

#[test]
fn over_the_wall_marks_nothing_built_and_the_ground_is_the_ground() {
    let mut w = World::new(SEED);
    let ((cx, cz), _) = walled(&mut w);
    let hp_before = w
        .pieces
        .find(cx, cz, GROUND, LOC_EDGE_XLO)
        .expect("the wall stands")
        .hp;

    // 60° up from 1.5 m away clears a 3 m storey from a 1.6 m eye by a
    // metre — and a 2 m reach ends in the air.
    let over = pitch_toward(2.6, STANCE_M);
    let ev = swing(&mut w, YAW_PLUS_X, over);
    assert!(
        impact(&ev).is_none_or(|(surf, ..)| surf != SURF_BUILT),
        "a swing pitched over the wall marked it"
    );
    assert_eq!(
        struct_hits(&ev),
        0,
        "a swing pitched over the wall charged it"
    );

    // Straight down: the dirt at the feet, and the wall is untouched.
    let ev = swing(&mut w, YAW_PLUS_X, 0);
    let (surf, ..) = impact(&ev).expect("a swing straight down meets the ground");
    assert_eq!(surf, SURF_GROUND, "a swing at the ground marks the ground");
    assert_eq!(struct_hits(&ev), 0);
    assert_eq!(
        w.pieces
            .find(cx, cz, GROUND, LOC_EDGE_XLO)
            .expect("the wall stands")
            .hp,
        hp_before,
        "the wall lost hp to swings that never met it"
    );
}
