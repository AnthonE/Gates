//! **A swing is a ray along the look**, and this is the suite that says so
//! (melee aim v1, 2026-09-05; operator: *"we need to step back and make this
//! a PROPER VIDEO GAME"*).
//!
//! What it replaces: until this landed the sim aimed a swing on a planar
//! 30° cone with a ±3 m vertical window and **no pitch at all** — the wire
//! carried a pitch byte, `ranged` had used it since the bow, and
//! `gather::swing` and `combat::strike` both threw it away. So a spear drawn
//! at the sky killed the man in front of you, a level swing took a rock at
//! your knees, and the first-person picture and the server disagreed about
//! everything but heading. `melee::cast` is one cast along the look now, and
//! whatever it enters first is what was hit.
//!
//! **The claims here are the ones construction cannot make for you.**
//! `tests/gather.rs` holds the node arm, `tests/combat.rs` the body arm,
//! `tests/mark.rs` the world arm and `tests/mob.rs` the animals; each of
//! those aims at the thing it is about and would stay green under a cast
//! that quietly ignored pitch, because every one of them now looks straight
//! at its target. This file is the negative space: the aims that must MISS,
//! the ladder a swing now has, the wall that stops one, and the arm that is
//! measured along the ray rather than across the ground.
//!
//! Mutants run against it, all red:
//!
//! | mutant | caught by |
//! |---|---|
//! | `melee::ray` ignores `pitch` (level always) | the sky, the ladder, the shins |
//! | `cast` skips the world arm | the wall |
//! | the world arm only wins when nothing else was found | the wall |
//! | reach compared on the planar run, not `t` | the short arm |
//! | `part_damage` dropped from `strike_body` | the ladder |
//! | `World::chip`'s door tie deleted | the door |
//! | the door tie widened to every `loc` | `tests/chip.rs`' bench |

use sim_core::build::{BuildContent, BUILD_CELL_M, LOC_EDGE_XLO, LOC_PLANE};
use sim_core::collide::Part;
use sim_core::combat::CombatContent;
use sim_core::deploy::DeployContent;
use sim_core::fmath::fabs;
use sim_core::gather::{GatherContent, ItemStack};
use sim_core::input::{InputFrame, BTN_PRIMARY};
use sim_core::movement::{Body, POS_XZ_Q, POS_Y_Q};
use sim_core::world::{Command, World, EV_STRUCT_HIT, EV_SWING};
use sim_core::yaw_dir;

const SEED: u64 = 20260802;
/// `CombatContent::probe_fixture`'s item 0: 34 body, 34 structure, 2 m
/// reach, ×2 on the head and 50% on a limb — the shipped ladder's shape.
const SPEAR: u16 = 0;
const SPEAR_DAMAGE: u16 = 34;
const SPEAR_REACH_M: f32 = 2.0;
/// `BuildContent::probe_fixture`: row 0 is the foundation, row 1 the wall,
/// row 3 the doorway. `DeployContent::probe_fixture`: row 2 is the door.
const PIECE_FOUNDATION: u16 = 0;
const PIECE_WALL: u16 = 1;
const PIECE_DOORWAY: u16 = 3;
const DEPLOY_DOOR: u16 = 2;
const GROUND: u8 = 0;
/// Wire yaw due +X (`yaw_lut.rs`: index 0 is +Z, increasing rotates to +X).
const YAW_PLUS_X: u16 = 64 << 8;
/// Level look. Wire pitch 0 is straight DOWN and 255 straight up.
const LEVEL: u8 = 128;
const STRAIGHT_UP: u8 = 255;
const EYE_M: f32 = sim_core::ranged::ARROW_EYE_MM as f32 / 1000.0;
/// The capsule the sim gives a body (`collide`), restated here because the
/// aims below are written in terms of it: crown, chest and shin.
const CAPSULE_H_M: f32 = 1.7;
const HEAD_BAND_M: f32 = 0.25;
const LIMB_BAND_M: f32 = 0.85;
/// How far either side of the built edge the two stand — 1.9 m apart, inside
/// the fixture spear's 2 m reach with room for the ground to tilt.
const HALF_GAP_M: f32 = 0.95;
/// Ticks a fixture will hold the button waiting for the cadence.
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

/// Two armed players on the spawn pad, which the selector keeps clear of
/// scatter for 4 m — so nothing the world put down can absorb a swing that
/// these tests mean to reach a body.
///
/// **Takes and returns a `Box`** (`event_roles.rs`' measured rule): a
/// `World` is ~440 kB and two in a debug test frame overflow the thread's
/// 2 MiB stack.
fn duel() -> Box<World> {
    let mut w = Box::new(World::new(SEED));
    w.gather = GatherContent::probe_fixture();
    w.combat = CombatContent::probe_fixture();
    w.build = BuildContent::probe_fixture();
    w.deploy = DeployContent::probe_fixture();
    w.dev_spawn = Some(w.spawn_pos(1));
    w.tick(&[Command::Join { id: 1 }, Command::Join { id: 2 }]);
    for p in w.players.iter_mut().take(2) {
        // Wood and stone for the build verbs.
        p.inv[1] = ItemStack {
            item: 0,
            count: 500,
            cond: 0,
        };
        p.inv[2] = ItemStack {
            item: 1,
            count: 500,
            cond: 0,
        };
        p.inv[3] = ItemStack {
            item: 4,
            count: 10,
            cond: 0,
        };
    }
    arm(&mut w);
    w
}

/// Put the fixture's weapon back in the swinging hand (hotbar slot 0, which
/// is what `sel: 0` selects).
///
/// **Called after every placement, and that is not belt-and-braces.** The
/// fixture's melee item 0 and the fixture's building material are the same
/// item id, so `Command::Place` spends the weapon out of the hand — the
/// first draft of this file built a wall between the two men, swung, and
/// measured an unarmed swing that reached nothing, which reads exactly like
/// a wall that stopped the blow and was not charged for it.
fn arm(w: &mut World) {
    for p in w.players.iter_mut().take(2) {
        p.inv[0] = ItemStack {
            item: SPEAR,
            count: 1,
            cond: 0,
        };
    }
}

/// Feet of player `i`, metres.
fn feet(w: &World, i: usize) -> (f32, f32, f32) {
    let b = w.players[i].body;
    (
        b.qx as f32 * POS_XZ_Q,
        b.qy as f32 * POS_Y_Q,
        b.qz as f32 * POS_XZ_Q,
    )
}

/// Stand player `i` `dist` metres due +X of player 0.
fn stand_east_of_attacker(w: &mut World, i: usize, dist: f32) {
    let (ax, _, az) = feet(w, 0);
    w.players[i].body = Body::at(SEED, hv(SEED), ax + dist, az);
}

/// The pitch byte pointing closest along `(run, rise)` — a scan of the sim's
/// own LUT, because the crate's clippy walls bind this suite and there is no
/// `atan2` to be had.
fn pitch_toward(rise: f32, run: f32) -> u8 {
    let len = (rise * rise + run * run).sqrt();
    let mut best = LEVEL;
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

/// The pitch that looks from player 0's eye at a point `up` metres above
/// player 1's feet.
fn look_at_height(w: &World, up: f32) -> u8 {
    let (ax, ay, az) = feet(w, 0);
    let (vx, vy, vz) = feet(w, 1);
    let run = ((vx - ax) * (vx - ax) + (vz - az) * (vz - az)).sqrt();
    pitch_toward(vy + up - (ay + EYE_M), run)
}

/// Hold the primary button at `(yaw, pitch)` until **one** swing is taken,
/// and return what player 1 lost on that tick.
///
/// One, not a window: cadence paces the swings, and a window of
/// `SWING_INTERVAL_TICKS + 1` lands two — which is how the first draft of
/// this file killed the victim with a headshot pair and then measured the
/// next aim against an empty spot on the beach.
fn swing_once(w: &mut World, yaw: u16, pitch: u8) -> Swung {
    let before = w.players[1].hp;
    let mut seq = w.players[0].frame.seq;
    for _ in 0..MAX_STEPS {
        seq = seq.wrapping_add(1);
        w.tick(&[Command::Input {
            id: 1,
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
        let mut swung = false;
        let mut structure = 0;
        for e in w.events.entries() {
            swung |= e.code == EV_SWING;
            if e.code == EV_STRUCT_HIT {
                structure += e.c >> 16;
            }
        }
        if swung {
            return Swung {
                hp: before.saturating_sub(w.players[1].hp),
                structure,
            };
        }
    }
    panic!("no swing was taken in {MAX_STEPS} ticks");
}

/// What one swing did.
struct Swung {
    /// Hit points player 1 lost.
    hp: u16,
    /// Structure damage announced on `EV_STRUCT_HIT` (`c` packs
    /// `damage << 16 | hp_left`).
    structure: u32,
}

/// One swing's damage to the body.
fn swing_at(w: &mut World, yaw: u16, pitch: u8) -> u16 {
    swing_once(w, yaw, pitch).hp
}

/// One swing's damage to whatever built thing it met.
fn swing_for_structure(w: &mut World, yaw: u16, pitch: u8) -> u32 {
    swing_once(w, yaw, pitch).structure
}

/// Heal player 1 back to full, so consecutive aims in one test each measure
/// their own swing rather than a running total.
fn heal(w: &mut World) {
    let full = w.combat.player_hp;
    w.players[1].hp = full;
}

/// Seat the two players either side of a cell edge that will actually hold
/// a foundation, and hand back that cell.
///
/// **Scanned, not typed** (`tests/mark.rs`' rule and its reason): the spawn
/// pad is a beach and a foundation wants 1.5 m of level ground, so a fixture
/// that builds where the selector happens to seat a body is a fixture that
/// goes red the next time worldgen moves. It also asks for a cell nothing
/// scattered stands near, because a swing is a ray and a bush beside the
/// wall is what the ray would meet instead.
///
/// Player 0 stands `HALF_GAP_M` west of the edge and player 1 the same to
/// its east, so they are `2 × HALF_GAP_M` apart — inside the spear's reach,
/// with the edge exactly between them.
fn two_at_a_buildable_edge(w: &mut World) -> (u16, u16) {
    let table = sim_core::terrain::ScatterTable::alpha_default();
    for r in 0..64i32 {
        for dz in -r..=r {
            for dx in -r..=r {
                if dx.abs() != r && dz.abs() != r {
                    continue;
                }
                let cx = (170 + dx).clamp(0, 1023);
                let cz = (170 + dz).clamp(0, 1023);
                let x0 = cx as f32 * BUILD_CELL_M;
                let z_mid = (cz as f32 + 0.5) * BUILD_CELL_M;
                let centre = (x0 + BUILD_CELL_M * 0.5, z_mid);
                if !sim_core::build::foundation_terrain_ok(SEED, hv(SEED), centre.0, centre.1) {
                    continue;
                }
                // Nothing scattered within a swing of either stance.
                let mut quiet = true;
                for stance in [x0 - HALF_GAP_M, x0 + HALF_GAP_M] {
                    let pcx = (stance / sim_core::terrain::CELL_SIZE) as i32;
                    let pcz = (z_mid / sim_core::terrain::CELL_SIZE) as i32;
                    for ddz in -1..=1i32 {
                        for ddx in -1..=1i32 {
                            let sl = sim_core::terrain::scatter(
                                SEED,
                                &table,
                                hv(SEED),
                                pcx + ddx,
                                pcz + ddz,
                            );
                            let (rad, _) = sim_core::melee::swing_volume(sl.occupant);
                            if rad <= 0.0 {
                                continue;
                            }
                            let d2 =
                                (sl.x - stance) * (sl.x - stance) + (sl.z - z_mid) * (sl.z - z_mid);
                            if d2 < (3.0 + rad) * (3.0 + rad) {
                                quiet = false;
                            }
                        }
                    }
                }
                if !quiet {
                    continue;
                }
                w.players[0].body = Body::at(SEED, hv(SEED), x0 - HALF_GAP_M, z_mid);
                w.players[1].body = Body::at(SEED, hv(SEED), x0 + HALF_GAP_M, z_mid);
                // Let both settle onto their ground before an eye is read.
                for _ in 0..8 {
                    w.tick(&[]);
                }
                return (cx as u16, cz as u16);
            }
        }
    }
    panic!(
        "no quiet buildable cell within 64 of (170, 170) — the generator changed under this test"
    );
}

/// Stand `row` at `loc` on the scanned cell, over a foundation, placed by
/// player 0 — hard/soft v0 puts the soft side toward the placer, so the
/// swinger is at the face the fixture means.
fn place_at(w: &mut World, cx: u16, cz: u16, row: u16, loc: u8) {
    for (r, l) in [(PIECE_FOUNDATION, LOC_PLANE), (row, loc)] {
        let before = w.pieces.len();
        w.tick(&[Command::Place {
            id: 1,
            row: r,
            cx,
            cz,
            level: GROUND,
            loc: l,
            freehand: false,
        }]);
        assert_eq!(
            w.pieces.len(),
            before + 1,
            "fixture: piece row {r} did not stand at ({cx}, {cz}) loc {l}"
        );
    }
    arm(w);
}

// ---------------------------------------------------------------------------
// 1 · Pitch exists.
// ---------------------------------------------------------------------------

/// **A swing at the sky hits the sky.** The whole reason this slice exists:
/// the same body, the same heading, the same reach — and the only thing that
/// changes is where the player is looking.
///
/// Both misses are named because they fail differently: straight up leaves
/// the world entirely, and straight down stops on the ground at the swinger's
/// own feet (`melee::cast`'s world arm), which is a *stop* rather than an
/// absence and is the one a "did the ray reach anything" test would miss.
#[test]
fn a_swing_at_the_sky_does_not_kill_the_man_in_front_of_you() {
    let mut w = duel();
    stand_east_of_attacker(&mut w, 1, 1.2);
    let chest = look_at_height(&w, 1.0);

    assert_eq!(
        swing_at(&mut w, YAW_PLUS_X, STRAIGHT_UP),
        0,
        "a spear drawn at the sky took hp off a body on the ground"
    );
    assert_eq!(
        swing_at(&mut w, YAW_PLUS_X, 0),
        0,
        "a spear driven into the dirt took hp off a body standing over it"
    );
    assert_eq!(
        swing_at(&mut w, YAW_PLUS_X, chest),
        SPEAR_DAMAGE,
        "…and looking at the man is what hits him"
    );
}

/// **Where you look on a body is what you hit**, which a swing could not ask
/// before: `part_crossed` was the bow's alone, because a planar cone has no
/// height to compare against a head band. Three aims at one body, one metre
/// apart in altitude, and the fixture's own ladder (×2 head, 50% limb).
///
/// The bands are `collide::{HEAD_BAND_M, LIMB_BAND_M}` and the aims are
/// computed from them rather than typed: crown-minus-a-hand, mid-chest, and
/// mid-shin.
#[test]
fn the_part_ladder_is_the_swings_too() {
    let mut w = duel();
    stand_east_of_attacker(&mut w, 1, 1.0);

    let head = look_at_height(&w, CAPSULE_H_M - HEAD_BAND_M * 0.5);
    let chest = look_at_height(&w, (LIMB_BAND_M + CAPSULE_H_M - HEAD_BAND_M) * 0.5);
    // Low enough that the whole span through the body stays under the limb
    // band: §7's rule is the MOST significant part a span touches, so an aim
    // at mid-shin from a 1.6 m eye a metre away still scores the chest —
    // the span runs shin-to-sternum. This one leaves under his knees.
    let shin = look_at_height(&w, 0.15);
    assert!(
        head > chest && chest > shin,
        "fixture: the three aims are not ordered (head {head}, chest {chest}, shin {shin})"
    );

    let took_head = swing_at(&mut w, YAW_PLUS_X, head);
    heal(&mut w);
    let took_chest = swing_at(&mut w, YAW_PLUS_X, chest);
    heal(&mut w);
    let took_shin = swing_at(&mut w, YAW_PLUS_X, shin);

    assert_eq!(
        took_chest, SPEAR_DAMAGE,
        "the chest is the identity rung and must be the row's own damage"
    );
    // Through the sim's own rung table rather than arithmetic of our own:
    // what is under test here is which RUNG the aim selected, and
    // `tests/headshot.rs` is where the multipliers are gated.
    let rung = |part| sim_core::combat::part_damage(SPEAR_DAMAGE, part, 2, 50);
    assert_eq!(
        took_head,
        rung(Part::Head),
        "a spear to the head must pay the row's ×2 (took {took_head})"
    );
    assert_eq!(
        took_shin,
        rung(Part::Limb),
        "a spear to the shin must pay the row's 50% (took {took_shin})"
    );
}

/// **The arm is measured along the ray, not across the ground.** A man 1.9 m
/// away is inside a 2 m reach while you look at his chest and outside it
/// while you look at his feet, because his feet are further away than he is
/// — 2.47 m of spear to reach 1.9 m of ground.
///
/// This is the assertion a planar reach cannot make, and the one that says
/// which of the two numbers `reach_cm` is.
#[test]
fn the_reach_is_spent_along_the_look_not_across_the_ground() {
    let mut w = duel();
    let run = SPEAR_REACH_M * 0.95;
    stand_east_of_attacker(&mut w, 1, run);

    let chest = look_at_height(&w, 1.0);
    let boots = look_at_height(&w, 0.05);
    let drop = EYE_M - 0.05;
    let slant = (run * run + drop * drop).sqrt();
    assert!(
        slant > SPEAR_REACH_M,
        "fixture: the boots are {slant:.2} m away, inside the {SPEAR_REACH_M} m \
         reach — this test needs them outside it"
    );

    assert_eq!(
        swing_at(&mut w, YAW_PLUS_X, chest),
        SPEAR_DAMAGE,
        "his chest is {run:.2} m away and the arm is {SPEAR_REACH_M} m"
    );
    heal(&mut w);
    assert_eq!(
        swing_at(&mut w, YAW_PLUS_X, boots),
        0,
        "his boots are {slant:.2} m of spear away and the arm is {SPEAR_REACH_M} m"
    );
}

/// **A short arm is short ALONG the ray**, which is the half the test above
/// cannot separate: there the arm runs out because the ray itself is only as
/// long as the longest reach, so a planar reach test measures the same miss
/// (it does — that mutant survives on that case alone, which is why this one
/// exists).
///
/// Here the ray is long enough to arrive and the *hand* is what refuses. The
/// row's reach is cut to a metre and one body stands 1.35 m off, so his near
/// side is 0.95 m of ground away — and what that costs in arm depends
/// entirely on the look: level with his crown it is 0.95 m of spear, and
/// angled down at his chest it is 1.10 m of spear over the same ground.
#[test]
fn a_short_arm_is_spent_faster_by_looking_down() {
    const SHORT_REACH_M: f32 = 1.0;
    let mut w = duel();
    // A fixture arrangement, not a content edit: every probe row reaches
    // 2 m, and this case needs a reach the ray outlives.
    w.combat.melee[SPEAR as usize].reach_cm = (SHORT_REACH_M * 100.0) as u16;
    stand_east_of_attacker(&mut w, 1, 1.35);

    let flat = look_at_height(&w, 1.55);
    let down = look_at_height(&w, 0.8);
    // The two arm-lengths this case turns on, rebuilt from the sim's own
    // pitch LUT and the capsule's radius: how much spear each look spends
    // reaching the same near side. Asserted rather than trusted, because
    // the pad is not a table and a tilt under the fixture would otherwise
    // turn this into a test of nothing.
    let (ax, ay, az) = feet(&w, 0);
    let (vx, _, vz) = feet(&w, 1);
    let run = ((vx - ax) * (vx - ax) + (vz - az) * (vz - az)).sqrt()
        - sim_core::collide::CAPSULE_RADIUS_M;
    let spear_for = |pitch: u8| run / sim_core::pitch_dir(pitch).0;
    let _ = ay;
    assert!(
        spear_for(flat) < SHORT_REACH_M && spear_for(down) > SHORT_REACH_M,
        "fixture: the two looks spend {:.2} m and {:.2} m of a {SHORT_REACH_M} m \
         arm on the same {run:.2} m of ground — they must straddle it",
        spear_for(flat),
        spear_for(down)
    );

    assert!(
        swing_at(&mut w, YAW_PLUS_X, flat) > 0,
        "looking level along a {SHORT_REACH_M} m arm reaches his near side \
         {run:.2} m away and must land"
    );
    heal(&mut w);
    assert_eq!(
        swing_at(&mut w, YAW_PLUS_X, down),
        0,
        "aimed down at his chest the same arm spends {:.2} m on the same \
         {run:.2} m of ground, and must not reach",
        spear_for(down)
    );
}

// ---------------------------------------------------------------------------
// 2 · Whatever the ray meets first.
// ---------------------------------------------------------------------------

/// **A wall between two men stops the swing**, and the wall takes it instead.
///
/// One world and a before/after, so the control is the same two bodies on the
/// same ground: swing, watch hp fall, build the wall between them, swing
/// again at the same aim. `melee::cast` has no target order any more — the
/// wall wins because it is nearer along the ray, and it is what gets billed.
#[test]
fn a_wall_between_two_men_takes_the_swing_and_the_man_does_not() {
    let mut w = duel();
    let (cx, cz) = two_at_a_buildable_edge(&mut w);
    let chest = look_at_height(&w, 1.0);
    assert_eq!(
        swing_at(&mut w, YAW_PLUS_X, chest),
        SPEAR_DAMAGE,
        "fixture: the two must be able to reach each other before the wall goes up"
    );
    heal(&mut w);

    place_at(&mut w, cx, cz, PIECE_WALL, LOC_EDGE_XLO);
    let hp0 = w
        .pieces
        .find(cx, cz, GROUND, LOC_EDGE_XLO)
        .expect("the wall stands")
        .hp;

    let blow = swing_once(&mut w, YAW_PLUS_X, chest);
    assert_eq!(
        blow.hp, 0,
        "the swing went through the wall and hit the man behind it"
    );
    assert!(
        blow.structure > 0,
        "the swing hit nothing at all — the wall must be what stopped it"
    );
    let hp1 = w
        .pieces
        .find(cx, cz, GROUND, LOC_EDGE_XLO)
        .expect("the wall survives one blow")
        .hp;
    assert!(
        hp1 < hp0,
        "the wall stopped the swing and was not charged for it ({hp0} -> {hp1})"
    );
}

/// **A door in its doorway takes the blow, not the frame it hangs in.**
///
/// `collide::shot_stop` answers a shut door by blocking the whole edge and
/// returning the EDGE's address — the doorway piece's — so the naive read
/// bills the frame and leaves the door a vault. `World::chip` settles the tie
/// on the edge address, once, for a swing and a shot alike (it was
/// `combat::raid`'s by hand, for the swing only, until melee aim v1).
///
/// The narrowness is the other half: the tie is edges only, because a bench
/// is a deployable at its cell's PLANE address and a shot past it lands on
/// that same address — `tests/chip.rs`'s workbench case is the gate for that
/// direction, and widening this to every `loc` reddens it.
#[test]
fn a_door_in_its_doorway_takes_the_swing_and_the_frame_does_not() {
    let mut w = duel();
    let (cx, cz) = two_at_a_buildable_edge(&mut w);
    let chest = look_at_height(&w, 1.0);
    place_at(&mut w, cx, cz, PIECE_DOORWAY, LOC_EDGE_XLO);
    w.tick(&[Command::PlaceDeploy {
        id: 1,
        row: DEPLOY_DOOR,
        cx,
        cz,
        level: GROUND,
        loc: LOC_EDGE_XLO,
    }]);
    arm(&mut w);
    let i = w
        .deploys
        .entries()
        .iter()
        .position(|r| r.cx == cx && r.cz == cz && r.level == GROUND && r.loc == LOC_EDGE_XLO)
        .expect("fixture: the door hangs in the doorway");
    let door0 = w.deploys.entries()[i].hp;
    let frame0 = w
        .pieces
        .find(cx, cz, GROUND, LOC_EDGE_XLO)
        .expect("the doorway stands")
        .hp;

    let took = swing_for_structure(&mut w, YAW_PLUS_X, chest);
    assert!(took > 0, "the swing at a shut door landed on nothing");

    let door1 = w.deploys.entries()[i].hp;
    let frame1 = w
        .pieces
        .find(cx, cz, GROUND, LOC_EDGE_XLO)
        .expect("the doorway survives")
        .hp;
    assert!(
        door1 < door0,
        "the door took nothing ({door0} -> {door1}) — the blow was billed to \
         the frame it hangs in, which makes a door a vault"
    );
    assert_eq!(
        frame1, frame0,
        "the doorway was charged for a blow that landed on the door in it"
    );
}

/// **An empty hand breaks nothing**, and the swing still resolves. The
/// structure arm is gated on the held row rather than on the pick, so a
/// bare-handed player standing at a wall swings, reaches it, and takes
/// nothing off it — which is what stops a naked spawn from demolishing a
/// base with their fists.
#[test]
fn a_bare_hand_reaches_the_wall_and_cannot_break_it() {
    let mut w = duel();
    let (cx, cz) = two_at_a_buildable_edge(&mut w);
    let chest = look_at_height(&w, 1.0);
    place_at(&mut w, cx, cz, PIECE_WALL, LOC_EDGE_XLO);
    let hp0 = w
        .pieces
        .find(cx, cz, GROUND, LOC_EDGE_XLO)
        .expect("the wall stands")
        .hp;
    // Empty the swinging hand. Slot 0 is what `sel: 0` selects.
    w.players[0].inv[0] = ItemStack::default();

    assert_eq!(
        swing_for_structure(&mut w, YAW_PLUS_X, chest),
        0,
        "a bare hand chipped a wall"
    );
    assert_eq!(
        w.pieces
            .find(cx, cz, GROUND, LOC_EDGE_XLO)
            .expect("the wall stands")
            .hp,
        hp0,
        "a bare hand took hp off a wall"
    );
}

/// **Facing away is a whiff whatever the pitch.** The cheapest of these and
/// the one that would survive a cast that lost its yaw: three pitches over
/// the same body, all behind the swinger.
#[test]
fn nothing_behind_you_is_reachable_at_any_pitch() {
    let mut w = duel();
    stand_east_of_attacker(&mut w, 1, 1.2);
    let chest = look_at_height(&w, 1.0);
    let away = YAW_PLUS_X.wrapping_add(128 << 8);
    for pitch in [0u8, chest, LEVEL, STRAIGHT_UP] {
        assert_eq!(
            swing_at(&mut w, away, pitch),
            0,
            "a swing facing away at pitch {pitch} hit the man behind the swinger"
        );
    }
    // …and the fixture is live: turning round lands it.
    assert_eq!(swing_at(&mut w, YAW_PLUS_X, chest), SPEAR_DAMAGE);
}

/// The yaw LUT and the pitch LUT agree about what "level, due +X" means —
/// the one place this suite's two constants meet the sim's own tables, so a
/// LUT edit reddens here rather than silently re-aiming every test above.
#[test]
fn the_two_luts_are_the_ones_the_tests_assume() {
    let (fx, fz) = yaw_dir(YAW_PLUS_X);
    assert!(
        fx > 0.999 && fabs(fz) < 0.001,
        "yaw {YAW_PLUS_X:#x} is not +X"
    );
    let (ch, sv) = sim_core::pitch_dir(LEVEL);
    assert!(ch > 0.999 && fabs(sv) < 0.01, "pitch {LEVEL} is not level");
    let (_, up) = sim_core::pitch_dir(STRAIGHT_UP);
    assert!(up > 0.999, "pitch {STRAIGHT_UP} is not straight up");
    let (_, down) = sim_core::pitch_dir(0);
    assert!(down < -0.999, "pitch 0 is not straight down");
}
