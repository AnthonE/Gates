//! Deterministic synthetic players: the input source for `test_alloc_zero`,
//! `test_replay`, `test_parity_wasm`, and later the server's `bots` bin.
//! Lives in sim-core so native and wasm drive byte-identical inputs.
//!
//! Two profiles now, and they are deliberately separate functions rather
//! than one with a mode flag. `bot_frame` produces **input frames** — a
//! walk, a swing, a jump — and its rng stream is load-bearing for three
//! digest gates (see `JUMP_PERIOD`). `raid_step` produces **commands** —
//! build, bolt, arm, plant, guess, take — and spends its own stream on
//! its own `Pcg32`, so nothing here moves a digest that existed before it.

use crate::build::{LOC_EDGE_XLO, LOC_EDGE_ZLO, LOC_PLANE};
use crate::deploy::{box_key, ACCESS_OP_ENTER, ACCESS_OP_SET_CODE, ACCESS_OP_TAKE};
use crate::input::{InputFrame, BTN_JUMP, BTN_PRIMARY, BTN_SPRINT};
use crate::inventory::{CONT_BOX, CONT_SELF};
use crate::lock::CODE_MAX;
use crate::rng::Pcg32;
use crate::world::Command;

/// One jump every `JUMP_PERIOD` frames, and the reason it is a period rather
/// than a roll.
///
/// Every other button here spends an `rng` draw, so adding a fourth would
/// shift the stream and move `parity`, `combat`, `bags`, `test_replay` and
/// `test_alloc_zero` all at once — leaving no way to tell a digest that moved
/// because bodies now leave the ground from a digest that moved because every
/// bot re-rolled its whole life. Keyed off `seq` instead, the stream is
/// byte-identical to before this bit existed and the digests move for exactly
/// one reason.
///
/// 128 frames is ~4.3 s at 30 Hz against a ~0.7 s arc (`2·JUMP_SPEED/GRAVITY`
/// = 21 ticks), so a bot is airborne roughly a sixth of the time: enough that
/// launch, flight and landing are all permanently on the parity surface,
/// little enough that it does not hollow out the coverage the other probes
/// depend on — `probe_parity`'s bots still have to stand still enough to fill
/// inventories, stand pieces and reach the upgrade rung. It also divides
/// `PARITY_TICKS`' 16-tick window at `seq` 0, so all 10,000 of that probe's
/// sequences carry a launch even though none of them is long enough to carry
/// a landing; `probe_combat` (256) and `probe_bags` (600) carry whole arcs.
const JUMP_PERIOD: u16 = 128;

/// Random-walk input frame: `yaw` drifts around the previous heading,
/// mostly-forward movement, bursts of sprint and strafe, and the primary
/// button held about a third of the time — bots swing at whatever they
/// wander past, so the alloc/replay/parity gates walk the gather path
/// too. They jump on a fixed period for the same reason, so those gates
/// walk the vertical too. Allocation-free.
pub fn bot_frame(rng: &mut Pcg32, prev_yaw: u16, seq: u16) -> InputFrame {
    let yaw_step = rng.next_bounded(8192) as i32 - 4096;
    let yaw = (prev_yaw as i32).wrapping_add(yaw_step) as u16;
    let forward = 40 + rng.next_bounded(88) as i32; // 40..=127, keeps moving
    let strafe = rng.next_bounded(255) as i32 - 127;
    let mut buttons = if rng.next_bounded(4) == 0 {
        BTN_SPRINT
    } else {
        0
    };
    if rng.next_bounded(3) == 0 {
        buttons |= BTN_PRIMARY;
    }
    // Deliberately after the draws and deliberately not one: see JUMP_PERIOD.
    if seq.is_multiple_of(JUMP_PERIOD) {
        buttons |= BTN_JUMP;
    }
    InputFrame {
        seq,
        buttons,
        yaw,
        pitch: rng.next_bounded(256) as u8,
        move_x: strafe as i8,
        move_z: forward as i8,
        // Wander the hotbar too, so held-item selection is inside the
        // alloc/replay/parity surface.
        sel: rng.next_bounded(6) as u8,
    }
}

/// Which rows of a baked table a raid script drives.
///
/// The profile carries no row numbers of its own, and that is wall 7 and
/// not tidiness: a raider that knew what a wall costs would be balance
/// living in code. The caller names what a foundation, a wall, a
/// container, a lock and the held throwable are in the table *it* baked,
/// and the script is the same script against `probe_fixture` or
/// `content/`.
#[derive(Clone, Copy)]
pub struct RaidRows {
    /// Building-piece row for a foundation, placed at `LOC_PLANE`.
    pub foundation: u16,
    /// Building-piece row for a wall, placed at the two edge locs.
    pub wall: u16,
    /// Deployable row for a lockable container.
    pub container: u16,
    /// Deployable row for a code lock.
    pub lock: u16,
    /// Hotbar slot the throwable sits in. The plant verb reads the **held**
    /// item (`charge::place`), so a raider has to select before it can
    /// plant — which is why step 0 of the attacker's cycle is an input
    /// frame and not a throw.
    pub charge_slot: u8,
    /// Inventory slot the goods to bank sit in.
    pub goods_slot: u8,
    /// The code the owner arms its lock with. The attacker never gets it:
    /// it guesses, which is the point of the verb.
    pub code: u16,
}

/// One synthetic player's raid script: who it is, which plot it is on, and
/// which side of the wall it stands.
///
/// A plot is **one cell with two players on it** rather than a pair of
/// neighbouring bases, because the verbs under test are all reach-checked
/// and a raider out of reach exercises nothing but `REFUSE_B_REACH`. The
/// owner builds and locks; the attacker plants, guesses and takes.
#[derive(Clone, Copy)]
pub struct RaidPlan {
    pub id: u32,
    pub cx: u16,
    pub cz: u16,
    /// `true` = the raider, `false` = the owner whose base it is.
    pub attacker: bool,
    /// How many steps this plan has taken; the cycle is `RAID_CYCLE` long.
    pub step: u16,
    /// Input sequence, so a selection frame is not mistaken for a replay
    /// of the last one.
    pub seq: u16,
}

impl RaidPlan {
    pub const fn new(id: u32, cx: u16, cz: u16, attacker: bool) -> Self {
        Self {
            id,
            cx,
            cz,
            attacker,
            step: 0,
            seq: 0,
        }
    }
}

/// Steps in either side's cycle. Both are the same length so a plot's two
/// players stay in phase with each other across a long run — the owner is
/// rebuilding what the attacker blew up on a fixed offset rather than a
/// drifting one, which is what makes a storm a storm and not a race the
/// builder always loses.
pub const RAID_CYCLE: u16 = 10;

/// The next command this plan wants to issue, and the profile the judge's
/// gap 1 asked for: *build, lock, satchel a neighbour's wall, loot the
/// box.*
///
/// One command per call, allocation-free, no floats, no clock — the caller
/// owns the cadence. Every step is a **claim**, never a fact: the sim is
/// the verdict, and roughly half of what this emits is meant to be refused
/// (a stranger's code, a stranger's box, a foundation on somebody else's
/// claim). That is deliberate. The refusal paths are the half of wall 4
/// nothing drove at population, because `bot_frame` has never sent an
/// action of any kind — the 100-bot soak walks and swings and touches not
/// one of the ~20 action verbs.
pub fn raid_step(plan: &mut RaidPlan, rng: &mut Pcg32, rows: RaidRows) -> Command {
    let step = plan.step % RAID_CYCLE;
    plan.step = plan.step.wrapping_add(1);
    plan.seq = plan.seq.wrapping_add(1);
    let id = plan.id;
    let cx = plan.cx;
    let cz = plan.cz;
    // One draw per call whichever branch runs, so the two sides of a plot
    // spend the stream at the same rate.
    let edge = if rng.next_bounded(2) == 0 {
        LOC_EDGE_XLO
    } else {
        LOC_EDGE_ZLO
    };
    if plan.attacker {
        match step {
            0 => Command::Input {
                id,
                frame: InputFrame {
                    seq: plan.seq,
                    sel: rows.charge_slot,
                    ..InputFrame::default()
                },
                favour: 0,
            },
            1 => Command::Throw {
                id,
                deploy: false,
                cx,
                cz,
                level: 0,
                loc: edge,
            },
            // A guess, not the code. `lock.rs`'s shock/lockout ladder is
            // the thing being driven here, and it only runs on a wrong one.
            2 => Command::Access {
                id,
                cx,
                cz,
                level: 0,
                loc: LOC_PLANE,
                op: ACCESS_OP_ENTER,
                code: (rng.next_bounded(CODE_MAX as u32 + 1)) as u16,
            },
            3 => Command::Move {
                id,
                cont: box_key(cx, cz, 0),
                from_kind: CONT_BOX,
                from_slot: 0,
                to_kind: CONT_SELF,
                to_slot: 10,
                count: 5,
            },
            // On somebody else's claim, so the privilege walk answers.
            4 => Command::Place {
                id,
                row: rows.foundation,
                cx,
                cz,
                level: 0,
                loc: LOC_PLANE,
                freehand: false,
                plate: 0,
            },
            5 => Command::Throw {
                id,
                deploy: false,
                cx,
                cz,
                level: 0,
                loc: LOC_PLANE,
            },
            6 => Command::Demolish {
                id,
                deploy: false,
                cx,
                cz,
                level: 0,
                loc: edge,
            },
            7 => Command::Loot { id },
            // Take the lock off somebody else's box — the pickup rule from
            // the wrong side of it.
            8 => Command::Access {
                id,
                cx,
                cz,
                level: 0,
                loc: LOC_PLANE,
                op: ACCESS_OP_TAKE,
                code: 0,
            },
            // Mend the wall it is trying to breach. Absurd on purpose:
            // `Repair` is a client-driven path with a cost, and a verb
            // nobody sends is a verb no storm covers.
            _ => Command::Repair {
                id,
                deploy: false,
                cx,
                cz,
                level: 0,
                loc: edge,
            },
        }
    } else {
        match step {
            0 => Command::Place {
                id,
                row: rows.foundation,
                cx,
                cz,
                level: 0,
                loc: LOC_PLANE,
                freehand: false,
                plate: 0,
            },
            1 => Command::Place {
                id,
                row: rows.wall,
                cx,
                cz,
                level: 0,
                loc: LOC_EDGE_XLO,
                freehand: false,
                plate: 0,
            },
            2 => Command::Place {
                id,
                row: rows.wall,
                cx,
                cz,
                level: 0,
                loc: LOC_EDGE_ZLO,
                freehand: false,
                plate: 0,
            },
            3 => Command::PlaceDeploy {
                id,
                row: rows.container,
                cx,
                cz,
                level: 0,
                loc: LOC_PLANE,
            },
            4 => Command::PlaceDeploy {
                id,
                row: rows.lock,
                cx,
                cz,
                level: 0,
                loc: LOC_PLANE,
            },
            5 => Command::Access {
                id,
                cx,
                cz,
                level: 0,
                loc: LOC_PLANE,
                op: ACCESS_OP_SET_CODE,
                code: rows.code,
            },
            6 => Command::Move {
                id,
                cont: box_key(cx, cz, 0),
                from_kind: CONT_SELF,
                from_slot: rows.goods_slot,
                to_kind: CONT_BOX,
                to_slot: 0,
                count: 5,
            },
            // The owner opening its own box. The **correct** code, and the
            // only step on either side that produces one: `EV_AUTH` is
            // announced for a grant and never for a guess, so without this
            // the lock's success path is not in the storm at all.
            7 => Command::Access {
                id,
                cx,
                cz,
                level: 0,
                loc: LOC_PLANE,
                op: ACCESS_OP_ENTER,
                code: rows.code,
            },
            8 => Command::Repair {
                id,
                deploy: false,
                cx,
                cz,
                level: 0,
                loc: edge,
            },
            // And banking works both ways: the withdrawal is the same
            // splice point the reference's item-move bug lived on
            // (`CLAUDE.md` trap list), from the authorized side.
            _ => Command::Move {
                id,
                cont: box_key(cx, cz, 0),
                from_kind: CONT_BOX,
                from_slot: 0,
                to_kind: CONT_SELF,
                to_slot: 11,
                count: 5,
            },
        }
    }
}

/// Steps in a duellist's cycle: **pull, then feed.**
///
/// Two, and the count is a measurement rather than a preference. The first
/// draft held the trigger on three steps out of four and the gun never
/// fired once in four hundred ticks — because a pull on an empty magazine
/// is not free. `ranged::hitscan` pays the weapon's cadence *before* it
/// looks for a round (`ranged.rs`, the dry click), so every pull on a
/// spent cylinder pushes `next_swing` another `rate_ticks` out, and
/// `reload` refuses anything inside that window as `REFUSE_RL_BUSY`. Pull
/// more often than the gun can fire and the reload is starved forever:
/// the storm measured 96 deaths, all of them from the knife duels, with
/// `EV_SHOT` and `EV_RELOAD` never once announced and `EV_RELOAD_REFUSED`
/// the only thing the firearms said.
///
/// One pull and one feed per tick fixes it without a clock, because the
/// two verbs resolve at different points of the tick: `Command::Reload` is
/// applied in the command phase and the shot resolves after the player
/// loop, so the feed always gets the first look at a cadence that has just
/// expired. The gun then settles into its own rhythm — six shots at
/// `rate_ticks`, one dry click, one `rate_ticks` of quiet while the pulls
/// bounce off the cadence gate, and the reload lands on the tick that
/// window closes.
pub const BRAWL_CYCLE: u16 = 2;

/// One synthetic duellist: who it is, what it holds, and which way it
/// faces.
///
/// There is no `cx`/`cz` here, unlike [`RaidPlan`], because a duel is not
/// a place — it is a **bearing**. The partner stands straight ahead at a
/// fixed separation, so `yaw` is the whole of the geometry: a melee swing
/// wants the victim inside a 30° cone (`combat::strike`) and a bullet
/// wants it on the ray (`ranged::hitscan`), and one heading satisfies
/// both. The caller owns the seating; this owns the intent.
#[derive(Clone, Copy)]
pub struct BrawlPlan {
    pub id: u32,
    /// Hotbar slot the weapon sits in. Both verbs read the **held** item
    /// (`combat::held_melee`, `combat::held_ranged`), so a duellist that
    /// never selects never fights.
    pub weapon_slot: u8,
    /// Heading. The partner is straight ahead of it, and carries this
    /// yaw flipped a half turn.
    pub yaw: u16,
    /// `true` = carries the firearm and has to feed it; `false` = melee
    /// only, and its whole cycle is the one button.
    pub shooter: bool,
    /// How many steps this plan has taken; the cycle is `BRAWL_CYCLE`.
    pub step: u16,
    /// Input sequence, so two frames in one tick are not one frame
    /// replayed.
    pub seq: u16,
}

impl BrawlPlan {
    pub const fn new(id: u32, weapon_slot: u8, yaw: u16, shooter: bool) -> Self {
        Self {
            id,
            weapon_slot,
            yaw,
            shooter,
            step: 0,
            seq: 0,
        }
    }
}

/// The next command this duellist wants to issue: **hold the trigger, feed
/// the gun, and get up when you are put down.**
///
/// The third of those is why `down` is a parameter rather than something
/// this function could work out. A profile is a *client*, and a client
/// knows exactly one thing about its own death: the screen is up. So it
/// presses respawn, and the sim decides whether that is allowed — the
/// same claim-not-fact discipline every step of [`raid_step`] follows.
/// Without it a storm with damage in it is a graveyard by tick 120, which
/// is the reason `tests/raid_storm.rs` set its throwable's damage to zero
/// and left this whole family un-driven.
///
/// One command per call, allocation-free, no floats, no clock. One `rng`
/// draw per call whichever branch runs, so two duellists spend the stream
/// at the same rate — `raid_step`'s rule, for `raid_step`'s reason.
pub fn brawl_step(plan: &mut BrawlPlan, rng: &mut Pcg32, down: bool) -> Command {
    let step = plan.step % BRAWL_CYCLE;
    plan.step = plan.step.wrapping_add(1);
    plan.seq = plan.seq.wrapping_add(1);
    let id = plan.id;
    // Lag compensation's own input, and the only place this profile is
    // random. `world::apply` clamps a request to `Rewind::max_back()`, so
    // every value here is legal by construction and the draw is a
    // *rewind depth* rather than a knob: a duel where every body asks for
    // a different one puts `Rewind::pose_at` under a hundred distinct
    // lookups a tick instead of the one a unit fixture spends.
    let favour = rng.next_bounded(crate::rewind::Rewind::max_back() as u32 + 1) as u8;
    if down {
        // Not `on_bag`: a bag is a placed deployable and this fixture
        // places nothing, so asking for one would be a refusal dressed as
        // a verb. The spawn ring answers instead.
        return Command::Respawn { id, on_bag: false };
    }
    // The odd step is the feed, and only a shooter has anything to feed.
    // A melee fighter spends it on the button as well, which is not
    // filler: the cadence is 38 ticks (`gather::SWING_INTERVAL_TICKS`)
    // and the cycle is one tick long, so most presses are *absorbed* —
    // and a storm that never pressed a button it was not allowed to
    // press would not drive the cadence gate at all.
    if plan.shooter && step == BRAWL_CYCLE - 1 {
        return Command::Reload { id };
    }
    Command::Input {
        id,
        frame: InputFrame {
            seq: plan.seq,
            buttons: BTN_PRIMARY,
            yaw: plan.yaw,
            // Level, the value every melee suite in this repo aims with.
            pitch: 128,
            move_x: 0,
            move_z: 0,
            sel: plan.weapon_slot,
        },
        favour,
    }
}
