//! The animal brain — the reference game's AI brain, ripped
//! (`reference/ANIMALS.md` §2, §7 and §9.5).
//!
//! **What the reference has.** Its June 2021 rework put every NPC on one
//! brain (`BaseAIBrain`) and retired three older AI systems. The brain is a
//! state machine: each state (`BasicAIState`) has an enter, a think that
//! reports running, finished or error, and a leave. States are wired by an
//! *AI design*, a data file listing, per state, an ordered set of events
//! (`AIEventType`: timer, player detected, target lost, in attack range,
//! health below, state finished, state error, and so on). The first event
//! that fires and links somewhere switches state, and events chain with
//! `And`. Senses (`AIBrainSenses`) fill a memory (`SimpleAIMemory`) every
//! half second: a range, a listen range, a vision cone, and
//! `ignoreNonVisionSneakers` — "ignore crouching, out-of-FOV entities", the
//! blind spot a hunter sneaks up in. A navigator the states drive is a
//! separate component (`nav.rs` here). The 2024 wolf is the second
//! generation (`Rust.Ai.Gen2`: an FSM whose transitions sit on the states,
//! several switches allowed per tick, and a sense component in which
//! crouching halves the range and sprinting makes it 130%). Its pack answers
//! only its own members' howls, and it circles, charges and backs off fire.
//!
//! **What this is.** The same machine at the sim's scale. [`AiState`] is the
//! state list, [`Cond`] the event list, and a [`Design`] is the AI design:
//! for each state, the rules that leave it, tried in order, first match
//! wins, every condition in a rule ANDed. A species picks a design
//! ([`design_for`]) and every number the states spend comes from its
//! `MobDef` row, so the pig and the wolf differ in data as they always did
//! here, and a new behaviour is a rule in a table before it is a branch.
//!
//! **One think a slot every `MOB_THINK_TICKS`, phase-offset** (`limits.rs`):
//! the reference's half-second sense cadence. A think senses, tries the
//! current state's rules, and runs the state; a state that finishes or
//! fails on the spot is re-ruled in the same think, up to `MAX_SWITCHES`
//! (Gen2's own cap is three a tick). Between thinks the body only walks what
//! the last think left (`mob::step`). Every input is sim state (player bodies
//! and buttons, the roster, the clock) and every draw is a `cell_hash` of the
//! slot and the tick, so a replay thinks the same thoughts on the same ticks.

use crate::input::{BTN_CROUCH, BTN_SPRINT};
use crate::limits::{MAX_MOBS, MAX_PLAYERS, MOB_THINK_TICKS, MOB_WAKE_CM};
use crate::mob::{Bite, Bites, Mob, MobDef};
use crate::movement::POS_XZ_Q;
use crate::nav::{dist2_cm, yaw_toward, Ground, Nav, Plan};
use crate::rng::cell_hash;
use crate::terrain;
use crate::world::Player;
use crate::yaw_lut::yaw_dir;

/// What the brain is doing. The reference's `AIState` names, the subset an
/// animal uses.
#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AiState {
    /// Standing: grazing, looking about, waiting out a timer.
    #[default]
    Idle = 0,
    /// Walking to a point near home.
    Roam = 1,
    /// Running at the target along a path.
    Chase = 2,
    /// In reach: standing, facing, biting.
    Attack = 3,
    /// Running from the target to a point away from it.
    Flee = 4,
    /// Walking back inside the leash.
    NavigateHome = 5,
    /// Circling the target at a distance: waiting for room to close in,
    /// or for a target it cannot reach to come down.
    Orbit = 6,
    /// A guard's rounds: walking a ring of points around its post.
    Patrol = 7,
    /// Lying down at night: still, and noticing at half range.
    Sleep = 8,
    /// Going to look at something heard (the reference's `MoveTowards`,
    /// fed from its position memory).
    MoveTowards = 9,
}

pub const AI_STATES: usize = 10;

/// State switches one think may make — a state that finishes or fails on
/// entry is re-ruled at once rather than standing a half-second.
pub const MAX_SWITCHES: usize = 3;

/// How a state's last think went — the reference's `StateStatus`.
pub const RUNNING: u8 = 0;
pub const FINISHED: u8 = 1;
pub const FAILED: u8 = 2;

/// `Mob::target` when nothing is remembered.
pub const NO_TARGET: u8 = u8::MAX;

/// One condition a rule can test — the reference's `AIEvent` types.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cond {
    /// The state's timer ran out (`TimerAIEvent`).
    Timer,
    /// The state reported done: it arrived (`StateFinishedAIEvent`).
    Finished,
    /// The state could not go on: no route, or stuck (`StateErrorAIEvent`).
    Failed,
    /// A target is remembered (`PlayerDetected` / `TargetDetected`).
    Target,
    /// Nothing is (`TargetLost`).
    NoTarget,
    /// Health at or over the courage floor.
    Brave,
    /// Under it (`HealthBelow`).
    Afraid,
    /// The target is inside bite reach (`InAttackRange`).
    InReach,
    /// The target has left bite reach by a margin, so a stand does not
    /// flicker back into a chase on a step.
    OutOfReach,
    /// Past the leash, or on the beach (the inverse of `InRangeOfHome`).
    FarFromHome,
    /// This many pack-mates are already biting the target: wait your turn.
    Crowded,
    /// The target holds a lit torch inside the species' fear radius.
    Fire,
    /// Every route to the target has failed `GIVE_UP_TRIES` times running.
    GaveUp,
    /// After dusk (`world::is_night`).
    Night,
    /// A draw, percent — the reference's `Chance` event, hashed off the
    /// slot and the think so a replay rolls the same.
    Chance(u8),
    /// A noise is remembered (`noise.rs`; the reference's
    /// `OnPositionMemorySet`).
    Noise,
    /// None is.
    Quiet,
    /// Struck before it had noticed anyone (`Mob::ambushed`).
    Ambushed,
    /// A pack-mate inside the call range was struck within the last think.
    MateStruck,
}

/// A rule: when every condition holds, go to `to`. `forget` drops the
/// target and calms the senses on the way — how an animal gives up.
#[derive(Clone, Copy, Debug)]
pub struct Rule {
    pub when: &'static [Cond],
    pub to: AiState,
    pub forget: bool,
}

const fn go(when: &'static [Cond], to: AiState) -> Rule {
    Rule {
        when,
        to,
        forget: false,
    }
}

const fn give_up(when: &'static [Cond], to: AiState) -> Rule {
    Rule {
        when,
        to,
        forget: true,
    }
}

/// An AI design: for each state (indexed by `AiState as usize`), the rules
/// that leave it.
pub struct Design {
    pub rules: [&'static [Rule]; AI_STATES],
}

use AiState::*;
use Cond::*;

/// The boar (their boar and ours): grazes and roams its patch, beds down
/// some nights, charges whatever crowds it while whole and runs once hurt
/// under half. It never circles and never waits a target out — a boar that
/// cannot reach you loses interest.
pub static BOAR: Design = Design {
    rules: [
        // Idle
        &[
            go(&[Target, Afraid], Flee),
            go(&[Target, Brave], Chase),
            go(&[Noise], Flee),
            go(&[FarFromHome], NavigateHome),
            go(&[Timer, Night, Chance(40)], Sleep),
            go(&[Timer], Roam),
        ],
        // Roam
        &[
            go(&[Target, Afraid], Flee),
            go(&[Target, Brave], Chase),
            go(&[Noise], Flee),
            go(&[FarFromHome], NavigateHome),
            go(&[Finished], Idle),
            go(&[Failed], Idle),
            go(&[Timer], Idle),
        ],
        // Chase
        &[
            go(&[Afraid], Flee),
            go(&[NoTarget], Idle),
            go(&[InReach], Attack),
            give_up(&[Failed], Idle),
        ],
        // Attack
        &[
            go(&[Afraid], Flee),
            go(&[NoTarget], Idle),
            go(&[OutOfReach], Chase),
        ],
        // Flee: from the target, or from the noise when there is none.
        &[go(&[NoTarget, Quiet], Idle)],
        // NavigateHome
        &[
            go(&[Target, Afraid], Flee),
            go(&[Target, Brave], Chase),
            go(&[Noise], Flee),
            go(&[Finished], Idle),
            go(&[Failed], Idle),
        ],
        // Orbit
        &[go(&[NoTarget], Idle), go(&[Timer], Chase)],
        // Patrol
        &[
            go(&[Target, Afraid], Flee),
            go(&[Target, Brave], Chase),
            go(&[Noise], Flee),
            go(&[Finished], Idle),
            go(&[Failed], Idle),
        ],
        // Sleep: a shot wakes it running.
        &[
            go(&[Target, Afraid], Flee),
            go(&[Target, Brave], Chase),
            go(&[Noise], Flee),
            go(&[Timer], Idle),
        ],
        // MoveTowards: prey never goes to look; the row is the table's.
        &[
            go(&[Target], Chase),
            go(&[Timer], Idle),
            go(&[Finished], Idle),
        ],
    ],
};

/// The wolf, after the reference's 2024 rework: a pack hunter that runs you
/// down, mixes charging with circling (no more than `PACK_BITERS` close in
/// at once), circles a lit torch instead of biting through it, waits you
/// out on a rock until it has failed `GIVE_UP_TRIES` routes to you, and
/// comes to see what a gunshot was.
pub static WOLF: Design = Design {
    rules: [
        // Idle
        &[
            go(&[Target, Afraid], Flee),
            go(&[Target, Ambushed], Flee),
            go(&[Target, Fire], Orbit),
            go(&[Target], Chase),
            go(&[FarFromHome], NavigateHome),
            go(&[Noise], MoveTowards),
            go(&[Timer], Roam),
        ],
        // Roam
        &[
            go(&[Target, Afraid], Flee),
            go(&[Target, Ambushed], Flee),
            go(&[Target, Fire], Orbit),
            go(&[Target], Chase),
            go(&[FarFromHome], NavigateHome),
            go(&[Noise], MoveTowards),
            go(&[Finished], Idle),
            go(&[Failed], Idle),
            go(&[Timer], Idle),
        ],
        // Chase
        &[
            go(&[Afraid], Flee),
            go(&[NoTarget], Idle),
            give_up(&[GaveUp], NavigateHome),
            go(&[MateStruck], Orbit),
            go(&[Fire], Orbit),
            go(&[InReach, Crowded], Orbit),
            go(&[InReach], Attack),
            go(&[Failed], Orbit),
        ],
        // Attack
        &[
            go(&[Afraid], Flee),
            go(&[NoTarget], Idle),
            go(&[Fire], Orbit),
            go(&[OutOfReach], Chase),
        ],
        // Flee: a retreat after an ambush ends on its timer, in a charge.
        &[go(&[NoTarget], Idle), go(&[Timer], Chase)],
        // NavigateHome
        &[
            go(&[Target, Afraid], Flee),
            go(&[Target, Ambushed], Flee),
            go(&[Target, Fire], Orbit),
            go(&[Target], Chase),
            go(&[Finished], Idle),
            go(&[Failed], Idle),
        ],
        // Orbit
        &[
            go(&[Afraid], Flee),
            go(&[NoTarget], Idle),
            give_up(&[GaveUp], NavigateHome),
            go(&[Timer], Chase),
        ],
        // Patrol
        &[
            go(&[Target, Afraid], Flee),
            go(&[Target, Ambushed], Flee),
            go(&[Target], Chase),
            go(&[Finished], Idle),
            go(&[Failed], Idle),
        ],
        // Sleep: a hunter does not bed down at night; the row is the table's.
        &[go(&[Target], Chase), go(&[Timer], Idle)],
        // MoveTowards: going to look, until it finds someone or gets there.
        &[
            go(&[Target, Afraid], Flee),
            go(&[Target, Ambushed], Flee),
            go(&[Target, Fire], Orbit),
            go(&[Target], Chase),
            go(&[Finished], Idle),
            go(&[Failed], Idle),
            go(&[Timer], Idle),
        ],
    ],
};

/// A wolf kept at a post (`mob::guard_site_of`): the wolf's fight, with
/// rounds instead of a roam and the post instead of the hills afterwards —
/// the reference's monument NPC walking its move points.
pub static GUARD: Design = Design {
    rules: [
        // Idle
        &[
            go(&[Target, Afraid], Flee),
            go(&[Target, Ambushed], Flee),
            go(&[Target, Fire], Orbit),
            go(&[Target], Chase),
            go(&[FarFromHome], NavigateHome),
            go(&[Noise], MoveTowards),
            go(&[Timer], Patrol),
        ],
        // Roam: a guard never roams; if it ever did, it goes back on rounds.
        &[
            go(&[Target], Chase),
            go(&[Timer], Patrol),
            go(&[Finished], Idle),
        ],
        // Chase
        &[
            go(&[Afraid], Flee),
            go(&[NoTarget], NavigateHome),
            give_up(&[GaveUp], NavigateHome),
            go(&[MateStruck], Orbit),
            go(&[Fire], Orbit),
            go(&[InReach, Crowded], Orbit),
            go(&[InReach], Attack),
            go(&[Failed], Orbit),
        ],
        // Attack
        &[
            go(&[Afraid], Flee),
            go(&[NoTarget], NavigateHome),
            go(&[Fire], Orbit),
            go(&[OutOfReach], Chase),
        ],
        // Flee: a retreat after an ambush ends on its timer, in a charge.
        &[go(&[NoTarget], NavigateHome), go(&[Timer], Chase)],
        // NavigateHome
        &[
            go(&[Target, Afraid], Flee),
            go(&[Target, Ambushed], Flee),
            go(&[Target, Fire], Orbit),
            go(&[Target], Chase),
            go(&[Finished], Idle),
            go(&[Failed], Idle),
        ],
        // Orbit
        &[
            go(&[Afraid], Flee),
            go(&[NoTarget], NavigateHome),
            give_up(&[GaveUp], NavigateHome),
            go(&[Timer], Chase),
        ],
        // Patrol
        &[
            go(&[Target, Afraid], Flee),
            go(&[Target, Ambushed], Flee),
            go(&[Target, Fire], Orbit),
            go(&[Target], Chase),
            go(&[FarFromHome], NavigateHome),
            go(&[Noise], MoveTowards),
            go(&[Finished], Idle),
            go(&[Failed], Idle),
        ],
        // Sleep: a guard does not sleep at its post.
        &[go(&[Target], Chase), go(&[Timer], Idle)],
        // MoveTowards: to the edge of its post, toward the noise.
        &[
            go(&[Target, Afraid], Flee),
            go(&[Target, Ambushed], Flee),
            go(&[Target, Fire], Orbit),
            go(&[Target], Chase),
            go(&[Finished], Idle),
            go(&[Failed], Idle),
            go(&[Timer], Idle),
        ],
    ],
};

/// Which design a slot runs. Pure in the slot and the species, like
/// `kind_of`: a guard is a wolf at a post, and a pack animal is one whose
/// content row says it answers its pack.
pub fn design_for(slot: usize, def: &MobDef) -> &'static Design {
    if crate::mob::guard_site_of(slot).is_some() {
        &GUARD
    } else if def.pack_cm > 0 {
        &WOLF
    } else {
        &BOAR
    }
}

/// At most this many animals bite one target at once; the rest of a pack
/// circles (`Cond::Crowded`). **Ours, not theirs**: the reference's wolves
/// visibly mix charging with circling and reposition between attacks, but
/// no published rule caps the count. Two is the cap that makes the mix
/// happen here.
pub const PACK_BITERS: usize = 2;

/// Failed routes to one target before the animal gives up on it.
pub const GIVE_UP_TRIES: u8 = 3;

/// How long a give-up calms the senses: only an attacker is noticed until
/// it runs out. Twenty seconds.
const CALM_TICKS: u64 = 600;

/// Idle stands for 1–4 s; a guard at a patrol point for 3–8 s.
const IDLE_MIN_TICKS: u64 = 30;
const IDLE_SPAN_TICKS: u64 = 90;
const POST_MIN_TICKS: u64 = 90;
const POST_SPAN_TICKS: u64 = 150;
/// A roam leg gives up after this long, arrived or not. Twenty seconds.
const ROAM_MAX_TICKS: u64 = 600;
/// One orbit lasts four seconds before the brain tries the chase again.
const ORBIT_TICKS: u64 = 120;
/// A night's lie-down is 20–40 s, then up to look about.
const SLEEP_MIN_TICKS: u64 = 600;
const SLEEP_SPAN_TICKS: u64 = 600;
/// Under this percent of max hp an animal limps: walk speed, no sprint (the
/// reference caps a badly hurt animal's top speed below 10%).
const LIMP_PCT: u32 = 10;
/// A pack animal under this percent of max hp does not answer a call.
const ANSWER_PCT: u32 = 50;
/// A torch-bearer closer than this is bitten anyway: the reference's wolves
/// feint in at a player who crowds them with fire.
const FIRE_FEINT_CM: i64 = 250;
/// A target this far under the waterline is swimming, and a wolf gives up
/// on a swimmer at once.
const SWIM_DEPTH_M: f32 = 0.5;
/// Out of combat this long — not struck, nobody remembered — an animal
/// starts to heal (the reference's reworked wolf regenerates "after a long
/// time out of combat"). A minute, then a fortieth of its hp every think:
/// twenty seconds from a scratch to whole.
const HEAL_AFTER_TICKS: u64 = 1_800;
const HEAL_PARTS: u16 = 40;
/// How long an ambushed pack animal backs off before it turns and comes
/// back with whoever answered. Three seconds.
const RETREAT_TICKS: u64 = 90;
/// A wolf howls for its pack at most this often. Twenty seconds: one call
/// per hunt, not one per sighting.
const HOWL_COOLDOWN_TICKS: u64 = 600;
/// How long a heard noise is remembered — how long prey keeps running from
/// the spot and a hunter keeps meaning to look. Five seconds.
const NOISE_MEMORY_TICKS: u64 = 150;
/// A look lasts at most this long before it is given up. Twenty seconds.
const LOOK_MAX_TICKS: u64 = 600;
/// A hunter going to look stops inside this much of its leash: a guard
/// looks from the edge of its post and does not leave it.
const LOOK_LEASH_PCT: f32 = 0.9;
/// A pack-mate roams within this ring round its leader.
const PACK_ROAM_MIN_M: f32 = 3.0;
const PACK_ROAM_SPAN_M: f32 = 7.0;

/// A roam leg is 6–20 m, and lands inside 80% of the leash so that a route
/// around something near the edge does not carry the animal over it.
const ROAM_MIN_M: f32 = 6.0;
const ROAM_SPAN_M: f32 = 14.0;
const ROAM_LEASH_PCT: f32 = 0.8;
const ROAM_TRIES: i32 = 6;
/// A flight is to a point this far away from the threat.
const FLEE_M: f32 = 20.0;
/// A guard's rounds: `PATROL_LEGS` points on a ring this fraction of its
/// site leash around its home.
const PATROL_LEGS: u8 = 4;
const PATROL_RING_PCT: f32 = 0.5;
/// Arrived at home once within this.
const HOME_STOP_CM: u16 = 300;
/// Arrived at a roam or patrol point once within this.
const WALK_STOP_CM: u16 = 100;
/// A chase re-plans when the target has moved this far from where the
/// path was aimed.
const REPATH_CM: i64 = 150;
/// A partial route shorter than this is no progress: the animal is already
/// as close as the ground lets it get.
const PROGRESS_CM: i64 = 200;
/// Circling radius; a circling animal closer than the inner fraction of it
/// backs off first.
const ORBIT_CM: f32 = 700.0;
const ORBIT_STEP: u16 = 12 << 8;
/// Anything this close is noticed however it moves and wherever it
/// stands: you do not get to crouch on top of a pig.
const BUMP_CM: i64 = 100;
/// How far above its own feet a bite reaches, and how far below,
/// centimetres (y quanta are 1 cm). A player on a boulder is out of reach
/// of a wolf at its foot; one on the beach above a wolf in the shallows is
/// not.
const BITE_RISE_CM: i32 = 150;
const BITE_DROP_CM: i32 = 200;
/// A walker that moved less than this fraction of what its gait should
/// have carried it in one think is stuck; two such thinks is an error.
const STUCK_PCT: f32 = 0.25;
const STUCK_THINKS: u8 = 2;

/// Noise channels — disjoint from `mob.rs`'s 112 and 116 and every terrain
/// channel.
const CH_IDLE: u32 = 114;
const CH_ROAM: u32 = 115;
const CH_CHANCE: u32 = 117;

/// A peer as the brain sees it. The roster is snapshotted once at the top
/// of the tick and each slot's entry is refreshed as soon as it has thought,
/// so a pack-mate thinking later in the same tick sees the decision (the
/// bite cap is exact) and nothing reads a body mid-step.
#[derive(Clone, Copy, Debug, Default)]
pub struct Peer {
    pub live: bool,
    pub kind: u8,
    pub state: AiState,
    pub target: u8,
    pub qx: i32,
    pub qz: i32,
    pub hurt_at: u64,
}

impl Peer {
    pub fn of(m: &Mob, tick: u64) -> Self {
        Self {
            live: m.alive && m.awake && m.hp > 0,
            kind: m.kind,
            state: m.state,
            target: if m.roused_until > tick {
                m.target
            } else {
                NO_TARGET
            },
            qx: m.body.qx,
            qz: m.body.qz,
            hurt_at: m.hurt_at,
        }
    }
}

/// Snapshot the roster for this tick's thinks.
pub fn peers(mobs: &[Mob; MAX_MOBS], tick: u64) -> [Peer; MAX_MOBS] {
    let mut out = [Peer::default(); MAX_MOBS];
    for (p, m) in out.iter_mut().zip(mobs.iter()) {
        *p = Peer::of(m, tick);
    }
    out
}

/// Everything one think reads, apart from the animal itself.
pub struct Ctx<'c, 'h, 'o> {
    pub tick: u64,
    /// The hour the day clock reads, `/time` included (`weather::day_tick`).
    pub day_tick: u64,
    /// How far this weather lets an animal notice, per mille of a clear
    /// day's reach (`weather::sense_pm`): fog and rain shrink it.
    pub sense_pm: u32,
    pub players: &'c [Player; MAX_PLAYERS],
    /// Which players hold a lit torch this tick (`light::is_lit`).
    pub lit: &'c [bool; MAX_PLAYERS],
    /// What there is to hear (`noise.rs`).
    pub noises: &'c crate::noise::Noises,
    /// The tick's pack calls, for `world::tick` to announce.
    pub howls: &'c mut crate::mob::Howls,
    pub peers: &'c [Peer; MAX_MOBS],
    pub nav: &'c mut Nav,
    pub ground: &'c mut Ground<'h, 'o>,
}

/// One think for the animal in `slot`.
pub fn think(ctx: &mut Ctx, slot: usize, def: &MobDef, mob: &mut Mob, bites: &mut Bites) {
    let tick = ctx.tick;
    // Dormancy first, exactly as it always was: anyone who could see this
    // animal keeps it awake, the dead on their death screen included.
    mob.awake = nearest_watcher(ctx.players, mob) <= MOB_WAKE_CM * MOB_WAKE_CM;
    if !mob.awake {
        // Asleep stops the body where it stands and starts the next waking
        // from rest, so nothing wakes mid-stride along a stale route.
        mob.gait = 0;
        mob.path.clear();
        mob.state = Idle;
        mob.status = RUNNING;
        return;
    }

    track_stuck(mob);
    sense(ctx, slot, def, mob);
    heal(ctx, def, mob);

    // Rule on what the last think left, then run the state; a state that
    // finishes or fails on the spot is re-ruled at once, up to
    // `MAX_SWITCHES`.
    let design = design_for(slot, def);
    let (mut switches, mut ran) = (0, false);
    loop {
        let fired = design.rules[mob.state as usize]
            .iter()
            .find(|r| r.when.iter().all(|&c| holds(ctx, slot, def, mob, c)))
            .copied();
        if let Some(rule) = fired {
            if rule.forget {
                mob.target = NO_TARGET;
                mob.roused_until = tick;
                mob.calm_until = tick + CALM_TICKS;
                mob.tries = 0;
                mob.ambushed = false;
            }
            enter(ctx, slot, def, mob, rule.to);
            switches += 1;
        } else if ran {
            break;
        }
        mob.status = run(ctx, slot, def, mob, bites);
        ran = true;
        if mob.status == RUNNING || switches >= MAX_SWITCHES {
            break;
        }
    }
}

/// Is the condition true for this animal, now?
fn holds(ctx: &Ctx, slot: usize, def: &MobDef, mob: &Mob, c: Cond) -> bool {
    let tick = ctx.tick;
    match c {
        Timer => tick >= mob.state_until,
        Finished => mob.status == FINISHED,
        Failed => mob.status == FAILED,
        Target => has_target(ctx, mob),
        NoTarget => !has_target(ctx, mob),
        Brave => brave(def, mob),
        Afraid => !brave(def, mob),
        InReach => in_reach(ctx, def, mob, 100),
        OutOfReach => !in_reach(ctx, def, mob, 125),
        FarFromHome => far_from_home(ctx.ground.seed, slot, def, mob),
        Crowded => crowded(ctx, slot, mob),
        Fire => fire(ctx, def, mob),
        GaveUp => mob.tries >= GIVE_UP_TRIES,
        Night => crate::world::is_night(ctx.day_tick),
        Noise => mob.poi_until > tick,
        Quiet => mob.poi_until <= tick,
        Ambushed => mob.ambushed,
        MateStruck => mate_struck(ctx, slot, def, mob),
        Chance(pct) => draw(ctx.ground.seed, slot, tick, CH_CHANCE) % 100 < pct as u64,
    }
}

/// Courage: at or over `brave_pct` of max hp a roused animal stands and
/// fights. A species that cannot bite is never brave.
fn brave(def: &MobDef, mob: &Mob) -> bool {
    def.attack > 0 && (mob.hp as u32) * 100 >= (def.hp as u32) * (def.brave_pct as u32)
}

fn has_target(ctx: &Ctx, mob: &Mob) -> bool {
    mob.target != NO_TARGET && mob.roused_until > ctx.tick && valid_target(ctx.players, mob.target)
}

fn valid_target(players: &[Player; MAX_PLAYERS], t: u8) -> bool {
    players
        .get(t as usize)
        .is_some_and(|p| p.active && !p.sleeping && !p.dead)
}

fn target_d2(ctx: &Ctx, mob: &Mob) -> Option<i64> {
    if !has_target(ctx, mob) {
        return None;
    }
    let p = &ctx.players[mob.target as usize];
    Some(dist2_cm(p.body.qx - mob.body.qx, p.body.qz - mob.body.qz))
}

/// Bite reach squared, scaled by `pct` percent.
fn reach2(def: &MobDef, pct: i64) -> i64 {
    let r = def.attack_range_cm * pct / 100;
    r * r
}

/// The target can be bitten from here: inside `pct` percent of reach on
/// the plane, within `BITE_RISE_CM` of the same height, and with no wall
/// between. The height is what makes a boulder an escape and the wall what
/// makes a base one — both were bites through geometry before routes.
fn in_reach(ctx: &Ctx, def: &MobDef, mob: &Mob, pct: i64) -> bool {
    let Some(d2) = target_d2(ctx, mob) else {
        return false;
    };
    if d2 > reach2(def, pct) {
        return false;
    }
    let p = &ctx.players[mob.target as usize].body;
    let rise = p.qy - mob.body.qy;
    if !(-BITE_DROP_CM..=BITE_RISE_CM).contains(&rise) {
        return false;
    }
    let y = mob.body.qy as f32 * crate::movement::POS_Y_Q;
    !crate::collide::blocked(
        ctx.ground.seed,
        ctx.ground.haven,
        ctx.ground.cols,
        mob.body.qx as f32 * POS_XZ_Q,
        mob.body.qz as f32 * POS_XZ_Q,
        p.qx as f32 * POS_XZ_Q,
        p.qz as f32 * POS_XZ_Q,
        y,
    )
}

/// The leash, both halves: past the radius from home, or standing on the
/// beach. A guard's radius is its site's and its floor the land line
/// (`mob::guard_leash_cm` has why).
fn far_from_home(seed: u64, slot: usize, def: &MobDef, mob: &Mob) -> bool {
    let (leash_cm, floor) = leash(slot, def);
    let d2 = dist2_cm(mob.home_qx - mob.body.qx, mob.home_qz - mob.body.qz);
    if d2 > leash_cm * leash_cm {
        return true;
    }
    let x = mob.body.qx as f32 * POS_XZ_Q;
    let z = mob.body.qz as f32 * POS_XZ_Q;
    terrain::height(seed, x, z) <= floor
}

fn leash(slot: usize, def: &MobDef) -> (i64, f32) {
    match crate::mob::guard_leash_cm(slot) {
        Some(cm) => (cm, terrain::LAND_MIN_H),
        None => (def.roam_cm, terrain::BEACH_MAX_H),
    }
}

/// `PACK_BITERS` others already biting this animal's target.
fn crowded(ctx: &Ctx, slot: usize, mob: &Mob) -> bool {
    if mob.target == NO_TARGET {
        return false;
    }
    let biting = ctx
        .peers
        .iter()
        .enumerate()
        .filter(|&(i, p)| i != slot && p.live && p.state == Attack && p.target == mob.target)
        .count();
    biting >= PACK_BITERS
}

/// The target holds a lit torch inside this species' fear radius — and is
/// not crowding the animal with it, which gets them bitten anyway.
fn fire(ctx: &Ctx, def: &MobDef, mob: &Mob) -> bool {
    if def.fire_fear_cm <= 0 || !has_target(ctx, mob) || !ctx.lit[mob.target as usize] {
        return false;
    }
    target_d2(ctx, mob).is_some_and(|d2| {
        d2 <= def.fire_fear_cm * def.fire_fear_cm && d2 > FIRE_FEINT_CM * FIRE_FEINT_CM
    })
}

/// A pack-mate inside the call range was struck within the last think —
/// the reference's "a pack-mate getting hit makes the others stop charging".
fn mate_struck(ctx: &Ctx, slot: usize, def: &MobDef, mob: &Mob) -> bool {
    let Some(pack) = crate::mob::pack_of(slot) else {
        return false;
    };
    ctx.peers.iter().enumerate().any(|(i, p)| {
        i != slot
            && p.live
            && p.hurt_at > 0
            && ctx.tick.saturating_sub(p.hurt_at) <= MOB_THINK_TICKS
            && crate::mob::pack_of(i) == Some(pack)
            && dist2_cm(p.qx - mob.body.qx, p.qz - mob.body.qz) <= def.pack_cm * def.pack_cm
    })
}

/// Howl for the pack, if this is a pack animal and it has not howled
/// within `HOWL_COOLDOWN_TICKS`.
fn howl(ctx: &mut Ctx, slot: usize, def: &MobDef, mob: &mut Mob) {
    let tick = ctx.tick;
    if def.pack_cm > 0
        && crate::mob::pack_of(slot).is_some()
        && (mob.howled_at == 0 || tick >= mob.howled_at.saturating_add(HOWL_COOLDOWN_TICKS))
    {
        mob.howled_at = tick;
        ctx.howls.push(slot as u8);
    }
}

/// The gait a running state runs at: the species' fast gait, or its walk
/// once it is limping.
fn run_gait(def: &MobDef, mob: &Mob) -> i8 {
    if limping(def, mob) {
        def.gait.min(127) as i8
    } else {
        def.flee_gait.min(127) as i8
    }
}

/// Under `LIMP_PCT` of max hp.
pub fn limping(def: &MobDef, mob: &Mob) -> bool {
    (mob.hp as u32) * 100 < (def.hp as u32) * LIMP_PCT
}

/// The nearest body anyone is driving, distance² in cm². The dormancy
/// predicate: sleepers keep nothing awake, the dead on their death screen do.
fn nearest_watcher(players: &[Player; MAX_PLAYERS], mob: &Mob) -> i64 {
    let mut best = i64::MAX;
    for p in players.iter() {
        if !p.active || p.sleeping {
            continue;
        }
        best = best.min(dist2_cm(p.body.qx - mob.body.qx, p.body.qz - mob.body.qz));
    }
    best
}

/// Senses (`AIBrainSenses`, one pass a think) into memory.
///
/// Within the notice radius (`MobDef::spook_at`, which is the hour's, and
/// the weather's `sense_pm` of it) a
/// player walking or standing is noticed from any bearing — hearing, scent,
/// the reference's listen range — and one sprinting at 130% of it. A
/// crouched player is silent: seen only inside the sight cone and only at
/// half the radius (the reference wolf's two numbers), and outside the cone
/// not at all until close enough to touch. That last clause is the
/// reference's `ignoreNonVisionSneakers` and the whole of how a hunter gets
/// behind a boar (`ANIMALS.md` §9.5 item 6). Asleep, it all halves.
///
/// Memory is `target` plus `roused_until`, refreshed while the target is
/// noticed and left to run out when it is not — so a chase outlasts the
/// last sighting by `flee_ticks` and an hour changing mid-chase does not
/// call it off. The current target is kept over a nearer stranger (a
/// charge does not swap victims on a step), and a pack animal that notices
/// nobody itself answers a pack-mate inside `pack_cm` that is already on
/// someone.
fn sense(ctx: &mut Ctx, slot: usize, def: &MobDef, mob: &mut Mob) {
    let tick = ctx.tick;
    if mob.target != NO_TARGET && !valid_target(ctx.players, mob.target) {
        mob.target = NO_TARGET;
    }
    let calm = tick < mob.calm_until;
    // The hour's radius (`/time` moves the hour), shrunk by fog and rain
    // the way the dark already shrinks it (weather v0). Asleep, everything
    // is noticed at half of that.
    let awake_r = def.spook_at(ctx.day_tick) * ctx.sense_pm as i64 / 1000;
    let r = if mob.state == Sleep {
        awake_r / 2
    } else {
        awake_r
    };
    let (fx, fz) = yaw_dir(mob.yaw);
    let mut best: Option<(i64, u8)> = None;
    for (i, p) in ctx.players.iter().enumerate() {
        if calm || !p.active || p.sleeping || p.dead {
            continue;
        }
        let (dqx, dqz) = (p.body.qx - mob.body.qx, p.body.qz - mob.body.qz);
        let d2 = dist2_cm(dqx, dqz);
        let buttons = p.frame.buttons;
        let moving = p.frame.move_z != 0 || p.frame.move_x != 0;
        let radius = if buttons & BTN_CROUCH != 0 {
            let (dx, dz) = (dqx as f32, dqz as f32);
            let cone = def.sight_dot_pm as f32 * 0.001;
            let seen = fx * dx + fz * dz >= cone * (dx * dx + dz * dz).sqrt();
            if seen {
                r / 2
            } else {
                BUMP_CM
            }
        } else if buttons & BTN_SPRINT != 0 && moving {
            r * 13 / 10
        } else {
            r
        };
        if d2 > radius.max(BUMP_CM) * radius.max(BUMP_CM) {
            continue;
        }
        // The one already being chased wins ties with anyone up to twice
        // as close: a charge does not swap victims on a step.
        let score = if i as u8 == mob.target { d2 / 4 } else { d2 };
        if best.is_none_or(|(s, _)| score < s) {
            best = Some((score, i as u8));
        }
    }
    // A pack animal that found someone itself, fresh, calls its pack —
    // the reference wolf's howl. An answer to a call is not a call, and a
    // wolf howls at most once a `HOWL_COOLDOWN_TICKS`.
    if best.is_some() && !has_target(ctx, mob) {
        howl(ctx, slot, def, mob);
    }
    if best.is_none() && !calm && def.pack_cm > 0 && !has_target(ctx, mob) {
        best = pack_call(ctx, slot, def, mob);
    }
    // Hearing: the newest noise this animal is inside the radius of goes
    // into position memory. A sulk hears nothing, as it sees nothing.
    if !calm {
        if let Some(n) = ctx.noises.heard(tick, mob.body.qx, mob.body.qz) {
            mob.poi_qx = n.qx;
            mob.poi_qz = n.qz;
            mob.poi_until = tick + NOISE_MEMORY_TICKS;
        }
    }
    if let Some((_, t)) = best {
        if t != mob.target {
            mob.tries = 0;
        }
        mob.target = t;
        mob.roused_until = tick + def.flee_ticks as u64;
    }
}

/// A pack-mate on a target inside `pack_cm`: the answer to its call, the
/// reference wolf's howl. Only its own pack's howl, only from a mate that is
/// actually running someone down or biting (a wolf circling prey it cannot
/// reach does not howl for it), and a badly hurt wolf does not answer.
fn pack_call(ctx: &Ctx, slot: usize, def: &MobDef, mob: &Mob) -> Option<(i64, u8)> {
    let pack = crate::mob::pack_of(slot)?;
    if (mob.hp as u32) * 100 < (def.hp as u32) * ANSWER_PCT {
        return None;
    }
    let mut best: Option<(i64, u8)> = None;
    for (i, p) in ctx.peers.iter().enumerate() {
        if i == slot || !p.live || p.kind != mob.kind || p.target == NO_TARGET {
            continue;
        }
        if crate::mob::pack_of(i) != Some(pack) || !matches!(p.state, Chase | Attack | Flee) {
            continue;
        }
        if !valid_target(ctx.players, p.target) {
            continue;
        }
        let d2 = dist2_cm(p.qx - mob.body.qx, p.qz - mob.body.qz);
        if d2 > def.pack_cm * def.pack_cm {
            continue;
        }
        if best.is_none_or(|(s, _)| d2 < s) {
            best = Some((d2, p.target));
        }
    }
    best
}

/// Healing out of combat (`HEAL_AFTER_TICKS`): a wounded animal that got
/// away comes back whole, and one still being hunted does not.
fn heal(ctx: &Ctx, def: &MobDef, mob: &mut Mob) {
    if mob.hp >= def.hp || has_target(ctx, mob) {
        return;
    }
    if ctx.tick < mob.hurt_at.saturating_add(HEAL_AFTER_TICKS) {
        return;
    }
    mob.hp = (mob.hp + (def.hp / HEAL_PARTS).max(1)).min(def.hp);
}

/// Stuck detection: a walker that has barely moved across a think while
/// its gait said it should have. Counted here, spent by the walking states.
fn track_stuck(mob: &mut Mob) {
    let moved2 = dist2_cm(mob.body.qx - mob.last_qx, mob.body.qz - mob.last_qz);
    mob.last_qx = mob.body.qx;
    mob.last_qz = mob.body.qz;
    if mob.gait <= 0 || !mob.path.active() {
        mob.stuck = 0;
        return;
    }
    let top = if matches!(mob.state, Chase | Flee) {
        crate::movement::SPRINT_SPEED
    } else {
        crate::movement::WALK_SPEED
    };
    let expect_cm =
        top * (mob.gait as f32 / 127.0) * MOB_THINK_TICKS as f32 * crate::movement::DT * 100.0;
    let floor = (expect_cm * STUCK_PCT) as i64;
    if moved2 < floor * floor {
        mob.stuck = mob.stuck.saturating_add(1);
    } else {
        mob.stuck = 0;
    }
}

/// Switch state: the reference's `StateLeave` then `StateEnter`. Leaving
/// is only ever "drop the route", so it is folded in here.
fn enter(ctx: &mut Ctx, slot: usize, def: &MobDef, mob: &mut Mob, to: AiState) {
    let tick = ctx.tick;
    mob.state = to;
    mob.status = RUNNING;
    mob.stuck = 0;
    mob.path.clear();
    mob.state_until = u64::MAX;
    match to {
        Idle => {
            mob.gait = 0;
            let (lo, span) = if crate::mob::guard_site_of(slot).is_some() {
                (POST_MIN_TICKS, POST_SPAN_TICKS)
            } else {
                (IDLE_MIN_TICKS, IDLE_SPAN_TICKS)
            };
            mob.state_until = tick + lo + draw(ctx.ground.seed, slot, tick, CH_IDLE) % span;
        }
        Roam => {
            mob.state_until = tick + ROAM_MAX_TICKS;
            if let Some((x, z)) = roam_point(ctx, slot, def, mob) {
                walk_to(ctx, mob, x, z, WALK_STOP_CM);
            }
        }
        Patrol => {
            mob.leg = (mob.leg + 1) % PATROL_LEGS;
            let (x, z) = patrol_point(slot, def, mob);
            walk_to(ctx, mob, x, z, WALK_STOP_CM);
        }
        NavigateHome => {
            let (x, z) = home_xz(mob);
            walk_to(ctx, mob, x, z, HOME_STOP_CM);
        }
        Orbit => {
            mob.state_until = tick + ORBIT_TICKS;
        }
        MoveTowards => {
            mob.state_until = tick + LOOK_MAX_TICKS;
            let (x, z) = look_point(slot, def, mob);
            walk_to(ctx, mob, x, z, WALK_STOP_CM);
        }
        Sleep => {
            mob.gait = 0;
            mob.state_until = tick
                + SLEEP_MIN_TICKS
                + draw(ctx.ground.seed, slot, tick, CH_IDLE) % SLEEP_SPAN_TICKS;
        }
        Flee => {
            // Ambushed: back off for a moment and call the pack, then the
            // retreat's timer turns it round into the charge.
            if mob.ambushed && def.pack_cm > 0 {
                mob.state_until = tick + RETREAT_TICKS;
                howl(ctx, slot, def, mob);
            }
        }
        Chase | Attack => mob.ambushed = false,
    }
}

/// The state's think (`StateThink`): what it does this half-second, and
/// whether it is running, finished or failed.
fn run(ctx: &mut Ctx, slot: usize, def: &MobDef, mob: &mut Mob, bites: &mut Bites) -> u8 {
    match mob.state {
        Idle | Sleep => {
            mob.gait = 0;
            RUNNING
        }
        Roam | Patrol | NavigateHome | MoveTowards => walking(ctx, slot, def, mob),
        Chase => chase(ctx, def, mob),
        Attack => attack(ctx, slot, def, mob, bites),
        Flee => flee(ctx, def, mob),
        Orbit => orbit(ctx, slot, def, mob),
    }
}

/// Roam, Patrol and NavigateHome all walk a route to a point and stop.
fn walking(ctx: &mut Ctx, slot: usize, def: &MobDef, mob: &mut Mob) -> u8 {
    if mob.stuck >= STUCK_THINKS {
        mob.gait = 0;
        mob.path.clear();
        return FAILED;
    }
    if mob.path.arrived && !mob.path.partial {
        mob.gait = 0;
        if mob.state == MoveTowards {
            // Looked: the noise is spent, or the look would start again.
            mob.poi_until = ctx.tick;
        }
        return FINISHED;
    }
    if !mob.path.active() {
        // Arrived at the end of a partial route, or the plan was deferred
        // or never made: plan (again) from here.
        let (x, z, stop) = match mob.state {
            NavigateHome => {
                let (x, z) = home_xz(mob);
                (x, z, HOME_STOP_CM)
            }
            Patrol => {
                let (x, z) = patrol_point(slot, def, mob);
                (x, z, WALK_STOP_CM)
            }
            MoveTowards => {
                let (x, z) = look_point(slot, def, mob);
                (x, z, WALK_STOP_CM)
            }
            _ => {
                if mob.path.len == 0 {
                    // A roam with nowhere to go is a short one.
                    mob.gait = 0;
                    return FAILED;
                }
                let (x, z) = (
                    mob.path.goal_qx as f32 * POS_XZ_Q,
                    mob.path.goal_qz as f32 * POS_XZ_Q,
                );
                (x, z, WALK_STOP_CM)
            }
        };
        if !walk_to(ctx, mob, x, z, stop) {
            // No route home is the leash's old answer: steer straight at
            // it and let the capsule find the way it can.
            if mob.state == NavigateHome {
                let (hx, hz) = home_xz(mob);
                head(mob, hx, hz);
                mob.gait = def.gait.min(127) as i8;
                return RUNNING;
            }
            mob.gait = 0;
            return FAILED;
        }
    }
    // A look is a jog (the fast gait without the sprint); every other walk
    // is a walk.
    mob.gait = if mob.state == MoveTowards {
        def.flee_gait.min(127) as i8
    } else {
        def.gait.min(127) as i8
    };
    RUNNING
}

/// Run at the target along a route re-aimed as it moves. A target no
/// route reaches — past a wall, up a boulder — is a failed try, and the
/// design decides what a failure means (a boar loses interest; a wolf
/// circles and tries again).
fn chase(ctx: &mut Ctx, def: &MobDef, mob: &mut Mob) -> u8 {
    let Some(d2) = target_d2(ctx, mob) else {
        mob.gait = 0;
        return RUNNING;
    };
    let p = ctx.players[mob.target as usize].body;
    let (tx, tz) = (p.qx as f32 * POS_XZ_Q, p.qz as f32 * POS_XZ_Q);
    mob.gait = run_gait(def, mob);
    // A swimmer is given up on at once: nothing here swims after anyone.
    if terrain::ground(ctx.ground.seed, ctx.ground.haven, tx, tz)
        <= terrain::SEA_LEVEL - SWIM_DEPTH_M
    {
        mob.path.clear();
        mob.gait = 0;
        mob.tries = GIVE_UP_TRIES;
        return FAILED;
    }
    let stop = (def.attack_range_cm * 4 / 5).clamp(50, u16::MAX as i64) as u16;
    if d2 <= (stop as i64) * (stop as i64) {
        mob.path.clear();
        mob.gait = 0;
        head(mob, tx, tz);
        if in_reach(ctx, def, mob, 100) {
            // There: stand, and the next think takes the bite.
            return RUNNING;
        }
        // At its feet and still out of reach: up something, or through a
        // wall. Standing here would be forever.
        return fail(mob);
    }
    if mob.stuck >= STUCK_THINKS {
        mob.stuck = 0;
        mob.path.clear();
        return fail(mob);
    }
    // Re-aim when the target has walked off the end of the route. A route
    // that stops short is walked out first and re-planned from its end:
    // planning it again every think would buy the same cells again.
    let aimed = dist2_cm(p.qx - mob.path.goal_qx, p.qz - mob.path.goal_qz);
    if mob.path.active() && (mob.path.partial || aimed <= REPATH_CM * REPATH_CM) {
        return RUNNING;
    }
    match plan(ctx, mob, tx, tz, stop) {
        Plan::Direct | Plan::Found | Plan::Toward => RUNNING,
        Plan::Partial | Plan::Unreachable => {
            // The route stops short. Walk it while it is a real step closer;
            // once the animal is as close as the ground lets it get (the
            // foot of the rock, the outside of the wall) and still not in
            // reach — `InReach` is ruled before this runs — the try is spent.
            let step = dist2_cm(
                mob.path.goal_qx - mob.body.qx,
                mob.path.goal_qz - mob.body.qz,
            );
            if step > PROGRESS_CM * PROGRESS_CM {
                RUNNING
            } else {
                mob.path.clear();
                mob.gait = 0;
                fail(mob)
            }
        }
        Plan::NoWay => fail(mob),
        Plan::Deferred => {
            if !mob.path.active() {
                // No budget and no route: straight at them this think,
                // which is all the animal ever did before routes.
                head(mob, tx, tz);
            }
            RUNNING
        }
    }
}

/// A try spent on a target this animal cannot reach.
fn fail(mob: &mut Mob) -> u8 {
    mob.tries = mob.tries.saturating_add(1);
    FAILED
}

/// Stand, face, bite. The bite is phase-locked to `attack_ticks` (a
/// cooldown with no state), recorded for `world::tick` to land after the
/// whole roster has stepped.
fn attack(ctx: &mut Ctx, slot: usize, def: &MobDef, mob: &mut Mob, bites: &mut Bites) -> u8 {
    mob.gait = 0;
    mob.path.clear();
    let Some(d2) = target_d2(ctx, mob) else {
        return RUNNING;
    };
    let victim = mob.target as usize;
    let p = ctx.players[victim].body;
    head(mob, p.qx as f32 * POS_XZ_Q, p.qz as f32 * POS_XZ_Q);
    let period = def.attack_ticks.max(1) as u64;
    if def.attack > 0
        && d2 <= reach2(def, 100)
        && ctx.players[victim].hp > 0
        && ctx.tick % period == (slot as u64) % period
    {
        bites.push(Bite {
            mob_slot: slot as u8,
            victim: victim as u8,
            damage: def.attack,
            // A sentence's worth of precision for the death screen, not a
            // sim quantity; f32 sqrt is on wall 1's list.
            range_cm: ((d2 as f32).sqrt()) as u16,
        });
    }
    RUNNING
}

/// Run to a point away from the threat — the target, or with none the
/// noise that spooked it — and again from there while it is still
/// remembered. With no point to run to, straight away.
fn flee(ctx: &mut Ctx, def: &MobDef, mob: &mut Mob) -> u8 {
    mob.gait = run_gait(def, mob);
    let from = if has_target(ctx, mob) {
        let p = ctx.players[mob.target as usize].body;
        Some((p.qx, p.qz))
    } else if mob.poi_until > ctx.tick {
        Some((mob.poi_qx, mob.poi_qz))
    } else {
        None
    };
    let Some((tqx, tqz)) = from else {
        return RUNNING;
    };
    if mob.path.active() && mob.stuck < STUCK_THINKS {
        return RUNNING;
    }
    mob.stuck = 0;
    let (ax, az) = (
        (mob.body.qx - tqx) as f32 * POS_XZ_Q,
        (mob.body.qz - tqz) as f32 * POS_XZ_Q,
    );
    let away = yaw_toward(ax, az, mob.yaw.wrapping_add(0x8000));
    let (x, z) = (mob.body.qx as f32 * POS_XZ_Q, mob.body.qz as f32 * POS_XZ_Q);
    // Straight away first, then fanning out a quarter-turn either side.
    for k in [0i32, 16, -16, 32, -32, 48, -48, 64, -64] {
        let yaw = away.wrapping_add((k << 8) as u16);
        let (dx, dz) = yaw_dir(yaw);
        let (px, pz) = (x + dx * FLEE_M, z + dz * FLEE_M);
        if !ctx.nav.standable(ctx.ground, px, pz) {
            continue;
        }
        if plan(ctx, mob, px, pz, WALK_STOP_CM).moving() {
            return RUNNING;
        }
    }
    mob.path.clear();
    mob.want_yaw = away;
    RUNNING
}

/// Circle the target at a distance, stepping round it a little each think
/// (the reference wolf's stalk). Closer than the circle, back off first; a
/// lit torch widens it to the fear radius. No route: circling is steering,
/// and the capsule slides along whatever is in the way.
fn orbit(ctx: &mut Ctx, slot: usize, def: &MobDef, mob: &mut Mob) -> u8 {
    mob.path.clear();
    let Some(d2) = target_d2(ctx, mob) else {
        mob.gait = 0;
        return RUNNING;
    };
    let p = ctx.players[mob.target as usize].body;
    let r = if fire(ctx, def, mob) {
        def.fire_fear_cm as f32 + 200.0
    } else {
        ORBIT_CM
    };
    let (ox, oz) = (
        (mob.body.qx - p.qx) as f32 * POS_XZ_Q,
        (mob.body.qz - p.qz) as f32 * POS_XZ_Q,
    );
    let bearing = yaw_toward(ox, oz, mob.yaw);
    // Half the slots circle one way and half the other, so a pack spreads.
    let step = if slot.is_multiple_of(2) {
        ORBIT_STEP
    } else {
        0u16.wrapping_sub(ORBIT_STEP)
    };
    let (px, pz) = (p.qx as f32 * POS_XZ_Q, p.qz as f32 * POS_XZ_Q);
    let (dx, dz) = yaw_dir(bearing.wrapping_add(step));
    let rm = r * 0.01;
    let too_close = (d2 as f32) < (r * 0.6) * (r * 0.6);
    let (gx, gz) = if too_close {
        let (bx, bz) = yaw_dir(bearing);
        (px + bx * rm, pz + bz * rm)
    } else {
        (px + dx * rm, pz + dz * rm)
    };
    head(mob, gx, gz);
    mob.gait = run_gait(def, mob);
    RUNNING
}

/// Plan a route for this animal, gait untouched.
fn plan(ctx: &mut Ctx, mob: &mut Mob, x: f32, z: f32, stop: u16) -> Plan {
    let (bx, by, bz) = (
        mob.body.qx as f32 * POS_XZ_Q,
        mob.body.qy as f32 * crate::movement::POS_Y_Q,
        mob.body.qz as f32 * POS_XZ_Q,
    );
    ctx.nav
        .plan(ctx.ground, bx, by, bz, x, z, stop, &mut mob.path)
}

/// Plan a walk; false when there is no route at all (a deferred plan is
/// not a failure — it comes back next think).
fn walk_to(ctx: &mut Ctx, mob: &mut Mob, x: f32, z: f32, stop: u16) -> bool {
    !matches!(plan(ctx, mob, x, z, stop), Plan::NoWay)
}

/// Point the no-route heading at a spot.
fn head(mob: &mut Mob, x: f32, z: f32) {
    let (bx, bz) = (mob.body.qx as f32 * POS_XZ_Q, mob.body.qz as f32 * POS_XZ_Q);
    mob.want_yaw = yaw_toward(x - bx, z - bz, mob.yaw);
}

fn home_xz(mob: &Mob) -> (f32, f32) {
    (mob.home_qx as f32 * POS_XZ_Q, mob.home_qz as f32 * POS_XZ_Q)
}

/// A roam point (the reference's best-roam-position pick): a few draws in a
/// ring around where the animal stands — around the way home once it is
/// past half its leash — kept inside `ROAM_LEASH_PCT` of the leash and on
/// ground a probe says it can stand on.
fn roam_point(ctx: &mut Ctx, slot: usize, def: &MobDef, mob: &Mob) -> Option<(f32, f32)> {
    let (leash_cm, _) = leash(slot, def);
    let (x, z) = (mob.body.qx as f32 * POS_XZ_Q, mob.body.qz as f32 * POS_XZ_Q);
    let (hx, hz) = home_xz(mob);
    let leash_m = leash_cm as f32 * 0.01;
    let from_home2 = (x - hx) * (x - hx) + (z - hz) * (z - hz);
    // A pack-mate roams round its leader while the leader is up and about
    // (the reference's grouped animals roam to a ring round the leader).
    let leader = crate::mob::pack_leader_of(slot)
        .filter(|&l| l != slot && ctx.peers[l].live && def.pack_cm > 0);
    let (cx, cz, lo, span) = if let Some(l) = leader {
        let p = ctx.peers[l];
        (
            p.qx as f32 * POS_XZ_Q,
            p.qz as f32 * POS_XZ_Q,
            PACK_ROAM_MIN_M,
            PACK_ROAM_SPAN_M,
        )
    } else if from_home2 > (leash_m * 0.5) * (leash_m * 0.5) {
        ((x + hx) * 0.5, (z + hz) * 0.5, ROAM_MIN_M, ROAM_SPAN_M)
    } else {
        (x, z, ROAM_MIN_M, ROAM_SPAN_M)
    };
    let lim = leash_m * ROAM_LEASH_PCT;
    for attempt in 0..ROAM_TRIES {
        let h = cell_hash(
            ctx.ground.seed,
            slot as i32,
            ((ctx.tick / MOB_THINK_TICKS) as i32).wrapping_mul(8) + attempt,
            CH_ROAM,
        );
        let (dx, dz) = yaw_dir(((h & 0xFF) as u16) << 8);
        let dist = (lo + ((h >> 8) & 0xFFFF) as f32 / 65536.0 * span).min(lim);
        let (px, pz) = (cx + dx * dist, cz + dz * dist);
        if (px - hx) * (px - hx) + (pz - hz) * (pz - hz) > lim * lim {
            continue;
        }
        if !ctx.nav.standable(ctx.ground, px, pz) {
            continue;
        }
        return Some((px, pz));
    }
    None
}

/// Where a look goes: the remembered noise, pulled in to `LOOK_LEASH_PCT`
/// of the leash round home so a hunter never walks off its range to look
/// (and a guard looks from the edge of its post).
fn look_point(slot: usize, def: &MobDef, mob: &Mob) -> (f32, f32) {
    let (leash_cm, _) = leash(slot, def);
    let lim = leash_cm as f32 * 0.01 * LOOK_LEASH_PCT;
    let (hx, hz) = home_xz(mob);
    let (px, pz) = (mob.poi_qx as f32 * POS_XZ_Q, mob.poi_qz as f32 * POS_XZ_Q);
    let (dx, dz) = (px - hx, pz - hz);
    let d2 = dx * dx + dz * dz;
    if d2 <= lim * lim {
        return (px, pz);
    }
    let k = lim / d2.sqrt();
    (hx + dx * k, hz + dz * k)
}

/// Patrol point `mob.leg`: evenly round a ring about home, the ring's phase
/// set by the slot so two guards on one post walk different rounds.
fn patrol_point(slot: usize, def: &MobDef, mob: &Mob) -> (f32, f32) {
    let (leash_cm, _) = leash(slot, def);
    let r = leash_cm as f32 * 0.01 * PATROL_RING_PCT;
    let phase = ((slot as u16).wrapping_mul(37) << 8)
        .wrapping_add(((mob.leg as u16) * (256 / PATROL_LEGS as u16)) << 8);
    let (dx, dz) = yaw_dir(phase);
    let (hx, hz) = home_xz(mob);
    (hx + dx * r, hz + dz * r)
}

fn draw(seed: u64, slot: usize, tick: u64, ch: u32) -> u64 {
    cell_hash(seed, slot as i32, (tick / MOB_THINK_TICKS) as i32, ch)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build::{self, BuildContent, BUILD_CELL_M, LOC_EDGE_XLO, LOC_EDGE_ZLO, LOC_PLANE};
    use crate::combat::CombatContent;
    use crate::input::InputFrame;
    use crate::mob::MobContent;
    use crate::movement::Body;
    use crate::world::{Command, World};

    /// A build cell whose own ground and every neighbour's takes a
    /// foundation, searched outward from the island's middle.
    fn flat_cell(w: &World) -> (u16, u16) {
        for r in 0..128i32 {
            for dz in -r..=r {
                for dx in -r..=r {
                    if dx.abs() != r && dz.abs() != r {
                        continue;
                    }
                    let (cx, cz) = (341 + dx, 341 + dz);
                    let ok = (-2..=2).all(|ez| {
                        (-2..=2).all(|ex| {
                            let x = ((cx + ex) as f32 + 0.5) * BUILD_CELL_M;
                            let z = ((cz + ez) as f32 + 0.5) * BUILD_CELL_M;
                            build::foundation_terrain_ok(w.seed, &w.haven, x, z)
                        })
                    });
                    if ok {
                        return (cx as u16, cz as u16);
                    }
                }
            }
        }
        panic!("no flat build cell");
    }

    /// **A wolf that cannot reach you circles, and then gives up.** The
    /// player stands in a walled 3 m box; a wolf 12 m off hears them. It
    /// must never bite through a wall, it must circle at least once while it
    /// tries, and inside `GIVE_UP_TRIES` failed routes it must drop the
    /// target and head home with its senses calmed.
    #[test]
    fn a_wolf_circles_a_player_it_cannot_reach_and_then_gives_up() {
        let mut w = World::new(11);
        // Clear skies held: fog would shrink the 12 m it has to hear across.
        w.env = crate::weather::Env::CLEAR;
        w.combat = CombatContent::probe_fixture();
        w.mob = MobContent::probe_fixture();
        let bc = BuildContent::probe_fixture();
        let (cx, cz) = flat_cell(&w);
        w.pieces.insert_for_test(cx, cz, 0, LOC_PLANE, 0, &bc);
        w.pieces.insert_for_test(cx, cz, 0, LOC_EDGE_XLO, 1, &bc);
        w.pieces.insert_for_test(cx, cz, 0, LOC_EDGE_ZLO, 1, &bc);
        w.pieces
            .insert_for_test(cx + 1, cz, 0, LOC_EDGE_XLO, 1, &bc);
        w.pieces
            .insert_for_test(cx, cz + 1, 0, LOC_EDGE_ZLO, 1, &bc);
        let (px, pz) = (
            (cx as f32 + 0.5) * BUILD_CELL_M,
            (cz as f32 + 0.5) * BUILD_CELL_M,
        );
        w.dev_spawn = Some((px, pz));
        w.tick(&[Command::Join { id: 1 }]);

        let slot = (0..MAX_MOBS)
            .find(|&s| crate::mob::pack_leader_of(s) == Some(s) && w.mobs.m[s].alive)
            .expect("a free wolf");
        for (i, m) in w.mobs.m.iter_mut().enumerate() {
            if i != slot {
                m.alive = false;
            }
        }
        let haven = w.haven;
        let m = &mut w.mobs.m[slot];
        m.body = Body::at(11, &haven, px + 12.0, pz + 1.0);
        m.home_qx = m.body.qx;
        m.home_qz = m.body.qz;
        m.path.clear();
        m.state = AiState::Idle;
        m.state_until = u64::MAX;

        let full = w.players[0].hp;
        assert!(full > 0);
        let (mut circled, mut gave_up) = (false, false);
        for seq in 0..1_500u16 {
            let frame = InputFrame {
                seq,
                ..InputFrame::default()
            };
            w.tick(&[Command::Input {
                id: 1,
                frame,
                favour: 0,
            }]);
            let m = &w.mobs.m[slot];
            assert_eq!(
                w.players[0].hp, full,
                "the wolf bit through a wall (state {:?})",
                m.state
            );
            circled |= m.state == AiState::Orbit;
            if m.calm_until > w.tick && m.target == NO_TARGET {
                gave_up = true;
                assert!(
                    matches!(
                        m.state,
                        AiState::NavigateHome | AiState::Idle | AiState::Roam
                    ),
                    "a wolf that gave up is {:?}",
                    m.state
                );
                break;
            }
        }
        assert!(circled, "the wolf never circled the box");
        assert!(
            gave_up,
            "the wolf never gave up on a player it cannot reach"
        );
    }
}
