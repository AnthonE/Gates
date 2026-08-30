//! `test_jump` — the gate for the vertical verb.
//!
//! Gravity has been in `movement.rs` since M0 and nothing could leave the
//! ground, so every assertion here is about a path that did not exist before
//! this file. What makes it able to fail is that **the two bounds are derived
//! from other constants, never written down twice**: an apex is checked
//! against `STEP_UP` and `build::LEVEL_H_M`, not against 1.22. Retune
//! `JUMP_SPEED` and this stays green while it still means something; move
//! `STEP_UP` or the storey without re-deriving the impulse and it reddens,
//! which is the whole point of a knob having a derivation rather than a
//! value.
//!
//! Four properties, in the order they matter:
//!
//! 1. **It does something walking cannot.** The capsule already steps
//!    `STEP_UP` (0.6 m) for free, so an apex at or under it is an animation,
//!    not a verb. This is the assertion that would have caught a jump tuned
//!    to look right in a client nobody can run yet.
//! 2. **It never clears a storey.** `LEVEL_H_M` is 3.0 m and a wall that can
//!    be hopped is not a wall — the whole build/raid premise, the thing
//!    `build::repair` and `deploy::decay_of` and upkeep are all defending,
//!    rests on this staying true. It is checked against the constant so that
//!    raising the impulse past a wall is a red gate rather than a playtest
//!    discovery.
//! 3. **A held button is a hop, never a climb.** `grounded` is the only gate
//!    on the launch and it is false for the whole arc, so the press that
//!    would double-jump simply is not grounded. Asserted by holding the
//!    button down for the entire flight and counting launches.
//! 4. **The arc closes.** It comes back down, re-grounds, and `qvy` returns
//!    to rest — a jump that left a body drifting would be a determinism
//!    problem long before it was a gameplay one.
//!
//! Plus the coverage claim itself, asserted rather than asserted-about
//! (`bots_actually_leave_the_ground`). `bot_frame` is the single input source
//! for `test_alloc_zero`, `test_replay` and all three parity probes, so
//! "jump is on those surfaces" is a real claim — and it is exactly the claim
//! that reads as true while being false if the period, the grounded gate or
//! the button bit quietly stopped agreeing. `probe_bags` folds a `wakes`
//! count beside its digest for this reason; this is the same discipline for
//! the same reason.

use sim_core::build::LEVEL_H_M;
use sim_core::fmath::fabs;
use sim_core::input::{InputFrame, BTN_JUMP, BTN_PRIMARY};
use sim_core::movement::{quant_y, Body, GRAVITY, JUMP_SPEED, POS_Y_Q, STEP_UP, VEL_Q, WALK_SPEED};
use sim_core::world::{Command, World};

/// A seed and a spot the rest of the suite already trusts to be walkable:
/// the spawn ring's own point for id 1, which the selector guarantees is
/// beach, flat enough to build on, and 4 m clear of scatter. A jump wants
/// exactly that — flat ground with nothing overhead — and taking it from the
/// sim rather than picking coordinates means a worldgen change cannot quietly
/// move this test onto a cliff.
const SEED: u64 = 20260805;

/// Ticks to run an arc out. `2·JUMP_SPEED/GRAVITY` is 0.7 s = 21 ticks, so
/// this is ~5× the flight time — a bound, not a prediction. Sim ticks, which
/// are deterministic state; `CLAUDE.md`'s no-wall-clock rule is untouched.
const ARC_TICKS: u32 = 100;

fn frame(buttons: u8) -> InputFrame {
    InputFrame {
        seq: 0,
        buttons,
        yaw: 0,
        pitch: 0,
        move_x: 0,
        move_z: 0,
        sel: 0,
    }
}

/// One body standing still on the spawn ring, already settled onto the
/// ground. Settling matters: `Body::at` asserts `grounded` before a step has
/// ever run, and a launch measured from an unsettled body would be measuring
/// the settle.
fn standing_world() -> World {
    let mut world = World::new(SEED);
    world.dev_spawn = Some(world.spawn_pos(1));
    world.tick(&[Command::Join { id: 1 }]);
    for _ in 0..30 {
        world.tick(&[Command::Input {
            id: 1,
            frame: frame(0),
            favour: 0,
        }]);
    }
    world
}

fn body_of(world: &World) -> Body {
    world.players[0].body
}

fn y_of(world: &World) -> f32 {
    body_of(world).qy as f32 * POS_Y_Q
}

/// Drive `ticks` ticks with `buttons` held, returning (apex above the start,
/// number of launches seen, whether the body finished grounded).
///
/// A launch is counted as a tick where the body was grounded going in and is
/// airborne with `qvy > 0` coming out — the transition, not the state, so
/// holding the button through a whole arc counts one and not twenty.
fn run(world: &mut World, buttons: u8, ticks: u32) -> (f32, u32, bool) {
    run_with(world, ticks, |_| buttons)
}

/// As `run`, but the button is decided per tick — `run_with(w, n, |t| ...)`.
/// A *tap* (press on tick 0, release after) is the only way to measure one
/// clean arc: a held button is a hop on every landing by design, so holding
/// it measures a sequence of arcs and their apexes all at once.
fn run_with(world: &mut World, ticks: u32, buttons_at: impl Fn(u32) -> u8) -> (f32, u32, bool) {
    let start_y = y_of(world);
    let mut apex = 0.0f32;
    let mut launches = 0u32;
    for t in 0..ticks {
        let was_grounded = body_of(world).grounded;
        world.tick(&[Command::Input {
            id: 1,
            frame: frame(buttons_at(t)),
            favour: 0,
        }]);
        let b = body_of(world);
        if was_grounded && !b.grounded && b.qvy > 0 {
            launches += 1;
        }
        let rise = y_of(world) - start_y;
        if rise > apex {
            apex = rise;
        }
    }
    (apex, launches, body_of(world).grounded)
}

/// Property 1 and 2: the apex sits strictly between what a step already buys
/// and what a storey must never give away.
#[test]
fn the_apex_clears_a_step_and_never_a_storey() {
    let mut world = standing_world();
    // A tap, not a hold: one press, then the button is released for the rest
    // of the flight. Holding it is a hop on every landing (asserted as its
    // own property below), which would make "the apex" mean the highest of
    // several arcs rather than the height of one.
    let (apex, launches, grounded) =
        run_with(&mut world, ARC_TICKS, |t| if t == 0 { BTN_JUMP } else { 0 });

    assert_eq!(
        launches, 1,
        "one press should be exactly one launch — saw {launches}"
    );
    assert!(
        apex > STEP_UP,
        "the jump peaked at {apex:.3} m, which is at or under STEP_UP \
         ({STEP_UP} m) — the capsule already climbs that for free, so this \
         impulse buys the player nothing a walk did not already have"
    );
    assert!(
        apex < LEVEL_H_M,
        "the jump peaked at {apex:.3} m against a storey of {LEVEL_H_M} m — \
         a piece that can be hopped is not a wall, and the build/raid loop \
         rests on that being impossible"
    );
    assert!(
        grounded,
        "the body never came back down within {ARC_TICKS} ticks"
    );

    // The analytic apex, as a regression band rather than a second copy of
    // the constant: v²/2g is what the continuous arc reaches and the discrete
    // integrator lands a little under it (the first tick pays a full step of
    // gravity before it moves). A band, because the exact figure is the
    // integrator's business and pinning it would make DT a wire constant.
    let analytic = JUMP_SPEED * JUMP_SPEED / (2.0 * GRAVITY);
    assert!(
        apex <= analytic && apex > analytic - 0.15,
        "apex {apex:.3} m is outside the integrator band for JUMP_SPEED \
         {JUMP_SPEED} m/s (analytic {analytic:.3} m) — the arc is no longer \
         the ballistic one these bounds were derived for"
    );
}

/// Property 3: the grounded gate, stated as the thing it prevents.
#[test]
fn a_held_button_cannot_climb() {
    let mut world = standing_world();
    let start_y = y_of(&world);

    // One arc's worth of ticks with the button never released. If `grounded`
    // were not the gate — if the launch keyed off the button alone — the body
    // would gain JUMP_SPEED every tick and be ~14 m up by here.
    let flight = (2.0 * JUMP_SPEED / GRAVITY / sim_core::movement::DT) as u32;
    let (apex, launches, _) = run(&mut world, BTN_JUMP, flight);
    assert_eq!(
        launches, 1,
        "holding jump through one flight time launched {launches} times — \
         the grounded gate is not holding, and a player can climb the sky"
    );
    assert!(
        apex < LEVEL_H_M,
        "a held button reached {apex:.3} m inside one flight time"
    );

    // And the arc still closes with the button down: landing re-arms
    // `grounded`, which is a hop, not a hover.
    let (_, _, grounded) = run(&mut world, BTN_JUMP, ARC_TICKS);
    assert!(grounded || y_of(&world) - start_y < LEVEL_H_M);

    // The other half of the same rule, stated so it is a decision and not an
    // accident: a held button DOES re-hop once the body lands. Over
    // `ARC_TICKS` at a ~21-tick flight time that is several launches, and it
    // is the genre's behaviour — it buys no speed here, because this capsule
    // has the same horizontal speed airborne as grounded, so bunny-hopping
    // is a way to look busy rather than a way to travel. If a later pass adds
    // an air-control penalty this assertion is where that decision surfaces.
    let mut held = standing_world();
    let (apex, launches, _) = run(&mut held, BTN_JUMP, ARC_TICKS);
    assert!(
        launches > 1,
        "a button held for {ARC_TICKS} ticks launched {launches} time(s) — \
         landing is supposed to re-arm the jump, and a player who has to \
         release and re-press to hop again has a different verb than this"
    );
    assert!(
        apex < LEVEL_H_M,
        "repeated hops reached {apex:.3} m — hops must not accumulate height"
    );
}

/// Property 4, and the regression half: without the bit, nothing moves. This
/// is what says the change is a new path rather than a change to the old one.
#[test]
fn without_the_bit_the_body_never_leaves_the_ground() {
    let mut world = standing_world();
    let settled = body_of(&world);
    let (apex, launches, grounded) = run(&mut world, 0, ARC_TICKS);
    assert_eq!(launches, 0, "a body with no jump bit left the ground");
    assert!(
        apex <= POS_Y_Q,
        "a standing body rose {apex:.3} m with no input"
    );
    assert!(grounded, "a standing body stopped being grounded");
    assert_eq!(
        body_of(&world).qvy,
        0,
        "a standing body holds a nonzero vertical velocity"
    );

    // Every other button is inert vertically, including the one that shares
    // its half of the field. A swing is not a hop.
    let mut world = standing_world();
    let (apex, launches, _) = run(&mut world, BTN_PRIMARY, ARC_TICKS);
    assert_eq!(launches, 0, "the primary button launched a jump");
    assert!(apex <= POS_Y_Q, "the primary button raised the body");
    assert_eq!(
        settled.qy,
        body_of(&world).qy,
        "swinging moved the body vertically"
    );
}

/// A corpse does not jump. `world.rs` rebuilds a dead player's frame with
/// `buttons` zeroed, which already covered sprint and the swing; this asserts
/// the new bit is inside that guarantee rather than beside it.
#[test]
fn the_dead_do_not_jump() {
    let mut world = standing_world();
    world.combat = sim_core::combat::CombatContent::probe_fixture();
    world.players[0].hp = 0;
    world.players[0].dead = true;
    let before = body_of(&world);
    let (apex, launches, _) = run(&mut world, BTN_JUMP, ARC_TICKS);
    assert_eq!(launches, 0, "a dead body launched a jump");
    assert!(apex <= POS_Y_Q, "a dead body rose {apex:.3} m");
    assert_eq!(before.qy, body_of(&world).qy, "a corpse moved vertically");
}

/// The coverage claim, asserted rather than asserted-about.
///
/// `bot_frame` is the input source for `test_alloc_zero`, `test_replay`,
/// `probe_parity`, `probe_combat` and `probe_bags`. Saying "jump rides those
/// gates" is only true while the period, the grounded gate and the bit still
/// agree with each other — and a claim like that reads exactly the same when
/// it has quietly stopped being true, which is the shape of a pass nobody
/// earned. So: drive real bots through a real world and require that bodies
/// actually left the ground.
#[test]
fn bots_actually_leave_the_ground() {
    use sim_core::bots::bot_frame;
    use sim_core::rng::Pcg32;

    let mut world = World::new(SEED);
    world.dev_spawn = Some(world.spawn_pos(1));
    world.tick(&[Command::Join { id: 1 }, Command::Join { id: 2 }]);
    let mut rng = Pcg32::new(SEED, 7);
    let mut yaws = [0u16; 2];
    let mut airborne_ticks = 0u32;
    let mut launches = 0u32;
    let mut peak = 0.0f32;

    // Two full jump periods, so the count cannot be an artefact of where the
    // window happened to start.
    for t in 0..300u32 {
        let f1 = bot_frame(&mut rng, yaws[0], t as u16);
        let f2 = bot_frame(&mut rng, yaws[1], t as u16);
        yaws = [f1.yaw, f2.yaw];
        let was = world.players[0].body.grounded;
        world.tick(&[
            Command::Input {
                id: 1,
                frame: f1,
                favour: 0,
            },
            Command::Input {
                id: 2,
                frame: f2,
                favour: 0,
            },
        ]);
        let b = world.players[0].body;
        if was && !b.grounded && b.qvy > 0 {
            launches += 1;
        }
        if !b.grounded {
            airborne_ticks += 1;
        }
        let over = b.qy as f32 * POS_Y_Q
            - sim_core::terrain::height(
                SEED,
                b.qx as f32 * sim_core::movement::POS_XZ_Q,
                b.qz as f32 * sim_core::movement::POS_XZ_Q,
            );
        if over > peak {
            peak = over;
        }
    }

    assert!(
        launches >= 2,
        "over 300 ticks at one jump per 128 frames a bot launched {launches} \
         times — the alloc/replay/parity gates are not walking the vertical, \
         whatever this file's header claims"
    );
    assert!(
        airborne_ticks > 0,
        "bots launched but were never observed airborne"
    );
    assert!(
        peak > STEP_UP,
        "a bot's highest point over its own terrain was {peak:.3} m, under \
         STEP_UP — the bots are stepping, not jumping"
    );
}

/// The knob's derivation, checked as arithmetic rather than trusted as prose.
///
/// `movement.rs` states `JUMP_SPEED` is derived from `STEP_UP` and
/// `LEVEL_H_M`. That derivation is the reason the number is defensible
/// instead of invented, and a comment cannot hold it — this can. It is also
/// the assertion that reddens if a later pass retunes the impulse for feel
/// and pushes it through a wall.
#[test]
fn the_impulse_is_derived_from_the_constants_it_sits_between() {
    let apex = JUMP_SPEED * JUMP_SPEED / (2.0 * GRAVITY);
    assert!(
        apex > STEP_UP * 1.5,
        "JUMP_SPEED gives a {apex:.3} m apex against a {STEP_UP} m free \
         step — too close to be a distinct verb"
    );
    assert!(
        apex < LEVEL_H_M * 0.5,
        "JUMP_SPEED gives a {apex:.3} m apex against a {LEVEL_H_M} m storey \
         — jump must stay clearly under half a storey so that no combination \
         of a step-up onto a piece and a jump ever tops one"
    );
    // Quantization: the impulse has to survive the wire's own velocity
    // quantum, or the value the server sims on is not the value it sends
    // (NETCODE.md §3). One quantum is 1 cm/s and JUMP_SPEED is a whole
    // number of them.
    let q = (JUMP_SPEED / VEL_Q + 0.5) as i32;
    assert!(
        fabs((q as f32 * VEL_Q) - JUMP_SPEED) < VEL_Q * 0.01,
        "JUMP_SPEED {JUMP_SPEED} is not a whole number of {VEL_Q} m/s velocity \
         quanta, so the launch the server sims is not the launch it transmits"
    );
    // And the apex has to survive the position quantum with room to spare,
    // or the arc a client draws is a staircase.
    assert!(
        quant_y(apex) > 100,
        "the apex is under 100 vertical quanta — too coarse to interpolate"
    );
    // Sanity on the pair that makes flight time: a jump must last long enough
    // to be steerable at walking pace, or it is a twitch.
    let flight_s = 2.0 * JUMP_SPEED / GRAVITY;
    assert!(
        flight_s * WALK_SPEED > 1.0,
        "a walking jump covers {:.2} m of ground — too short to clear a gap",
        flight_s * WALK_SPEED
    );
}
