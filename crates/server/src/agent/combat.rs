//! The fight reflex: what runs over whatever goal is in hand when another
//! body turns on this one, or when an opening is worth taking. It never
//! waits on the mind.
//!
//! A controller with a handful of modes:
//! - **Idle**: nothing to answer.
//! - **Guard**: a blow landed and the eyes have not found who struck it:
//!   face the blow's bearing and walk back from it until they do, or for
//!   a while. Shot at by a body not yet seen (a round cracking past, an
//!   arrow landing beside me after a report, a blow from nobody in sight),
//!   run weaving for cover from where the report came, eyes that way, and
//!   answer whoever the eyes find there ([`Combat::listen`]).
//! - **Engage**: a fight with one body, swung ([`Combat::melee`]) or
//!   shot ([`Combat::shoot`]).
//! - **Escape**: losing, or told to run: back away along a route, weaving,
//!   eyes on the danger, until nobody follows.
//! - **Loot**: the foe is dead: take its bag and pick up the arrows
//!   about, if nobody else is near; the orchestrator sends the verbs.
//! - **Heal**: not a frame of its own: the orchestrator bandages while
//!   nobody is near enough and in sight ([`safe`]).
//! - **Downed**: the body is down; the orchestrator crawls to cover.
//!
//! A fight is fought with whatever suits the range: a swung weapon close
//! in, and beyond a few metres a bow, crossbow or gun with rounds for it
//! ([`Combat::shoot`]). An arrow is aimed ahead of a moving body
//! (`super::aim`); a bullet at a player's pose on my screen (the server
//! rewinds players to it), and ahead of an animal (it does not rewind
//! them). Under fire from
//! further than a charge, a body with nothing to shoot back runs for cover
//! (`super::cover`), weaving; one merely aimed at sidesteps.
//!
//! Whether to fight is a sum a player does on the wiki's numbers: my hits
//! to kill them against theirs to kill me, what they are holding, how hurt
//! they are (my own hit markers), and who else is near. The temperament
//! says what to do with the answer: a passive body always runs, a
//! defensive one only answers attacks, an opportunist also starts a fight
//! it has a clear edge in, and a kill-on-sight one starts any it is not
//! losing. Everything it knows of other bodies comes from [`Tracks`].
//!
//! **A bred brain may make the call instead** ([`Combat::set_tactics`],
//! `jev-bot --tactics ga1:…`): a small net (`sim_core::genome`) bred in the
//! arena (`examples/zoo.rs --agent`) that reads the same sum and the same
//! tracks and says fight, fight close, fight at range, or run, where the
//! sum's `losing` and the range's hand used to. It decides nothing else:
//! a passive body still always runs, an unarmed one runs, the safe zone is
//! still never fought, and the eyes and hands are the human ones they
//! were. With no brain set, nothing here changes.

use super::hands::part_height;
use super::intent::{yaw_toward, Intent, Look};
use super::route::{into_deeper_water, Route, Step};
use super::tracks::{clear_line, eye_m, Band, Heard, Sight};
use super::tracks::{Sound, Species, Track, Tracks, AIM_MISS_M, CLOSING_MPS};
use super::wiki::{Book, Page};
use super::{aim, cover};
use client_core::core::ClientCore;
use protocol::{EntityState, BAG_KIND_PACK};
use sim_core::backpack::LOOT_REACH_M;
use sim_core::collide::{Part, CAPSULE_RADIUS_M};
use sim_core::gather::{STRIKE_DELAY_TICKS, SWING_INTERVAL_TICKS};
use sim_core::input::{InputFrame, BTN_AIM, BTN_CROUCH, BTN_JUMP, BTN_PRIMARY, BTN_SPRINT};
use sim_core::limits::{MAX_ITEM_DEFS, TICK_HZ};
use sim_core::movement::{POS_XZ_Q, POS_Y_Q, SPRINT_SPEED, WALK_SPEED};
use sim_core::ranged::{ARROW_EYE_MM, MM_PER_M};
use sim_core::rng::Pcg32;
use sim_core::town::Town;
use sim_core::wound::WOUNDED_HP;
use sim_core::{pitch_dir, yaw_dir};

/// A retreat backs away this long after its last renewal.
pub const FLEE_TICKS: u32 = 6 * TICK_HZ;
/// A retreat routes to a point this far along its bearing, and on again
/// from wherever it arrives.
pub const FLEE_M: f32 = 24.0;
/// A pursuer this close renews a retreat whatever it is doing.
pub const PURSUER_NEAR_M: f32 = 32.0;
/// A goal paused by a fight shorter than this picks up where it left off;
/// a longer one ends it.
pub const RESUME_TICKS: u32 = 20 * TICK_HZ;
/// Other bodies this close count against the odds, and an opportunist
/// takes no fight with a second one about.
pub const CROWD_M: f32 = 30.0;
/// An opening is looked for no further off than this.
pub const START_M: f32 = 30.0;
/// A foe chased further than this, uncommanded, is let go.
pub const CHASE_M: f32 = 45.0;
/// A foe out of sight this long has left the fight.
pub const LOST_TICKS: u32 = 3 * TICK_HZ;
/// An answered fight with no edge is let go once the foe has not
/// threatened for this long.
pub const CALM_TICKS: u32 = 6 * TICK_HZ;
/// A blow, swing or shot this recent is an attack under way.
pub const ALARM_TICKS: u32 = 3 * TICK_HZ;
/// An animal I swung or fired at within this long that leaves the snapshot
/// this near, having stood in sight, fell (`Foe::seen`): an arrow's flight
/// and the snapshot's delay both fit, and one that walks out of the
/// snapshot's reach is far further off.
const VANISHED_KILL_TICKS: u32 = 2 * TICK_HZ;
const VANISHED_KILL_M: f32 = 60.0;
/// A blow's author is looked for within this far past their reach: the
/// ground a body covers while the eyes come round and find it.
pub const ATTACKER_SLACK_M: f32 = 6.0;
/// A swing seen but not felt is at me only if it could have landed: I
/// stand within its reach plus this (a pose a few ticks old), and no
/// further off its line than a body's width plus [`SWING_MISS_M`]. Kept
/// under the margin a harvest swing clears bodies by, so a body chopping
/// beside me is not an attack.
pub const SWING_SLACK_M: f32 = 0.5;
pub const SWING_MISS_M: f32 = 0.1;
/// A foe not closed on by [`CLOSE_M`] in this long, while trying, cannot be
/// reached: the fight is let go, and the body is not picked again for
/// [`SHUN_TICKS`].
pub const UNREACHABLE_TICKS: u32 = 3 * TICK_HZ;
pub const CLOSE_M: f32 = 0.5;
pub const SHUN_TICKS: u32 = 30 * TICK_HZ;
/// An animal fought this long with not one hit landing on it (wedged on a
/// lip, under a rock, or standing where every blow and arrow misses) is let
/// go and shunned: the reach clock never runs while I stand in range, and a
/// hunt that never ends holds every decision back with it.
pub const NO_HARM_TICKS: u32 = 20 * TICK_HZ;
/// Bodies shunned at once: a raid's few players, each found out of reach,
/// are not taken up again in turn.
pub const SHUN_MAX: usize = 4;
/// A wolf closing inside this is already attacking.
pub const WOLF_ALARM_M: f32 = 10.0;
/// A player with a swung weapon out, facing me and stepping in on me from
/// within this past its reach, is already attacking once it has kept at
/// it this long: about the time it takes to tell someone coming at me
/// from someone passing on their way to a tree behind me. A person swings
/// at a charge as it comes into reach, not after its first blow lands.
pub const MENACE_M: f32 = 2.0;
pub const MENACE_TICKS: u32 = 6;
/// ...its eyes no further off my head than this, and with no swing seen
/// from it this long.
const FACE_MISS_M: f32 = 0.35;
pub const GATHERING_TICKS: u32 = 10 * TICK_HZ;
/// A player with something in hand this near is watched while the goal
/// runs: for this long after it is first spotted (sized up), and for as
/// long as it keeps coming my way.
pub const WARY_M: f32 = 15.0;
pub const SIZE_UP_TICKS: u32 = 3 * TICK_HZ / 2;
/// Footsteps or a shot heard this recently from where nobody was seen
/// lately (within [`GLANCE_CONE`] of the sound) turn the eyes that way, for
/// one look this long; sounds from that way are then let be for a while,
/// found or not (someone walking behind a wall is heard and never seen).
pub const GLANCE_TICKS: u32 = TICK_HZ / 2;
pub const GLANCE_KNOWN_TICKS: u32 = 3 * TICK_HZ;
pub const GLANCE_LOOK_TICKS: u32 = TICK_HZ;
pub const GLANCE_REST_TICKS: u32 = 8 * TICK_HZ;
const GLANCE_CONE: u16 = 0x2000;
/// A report heard no longer than this before an arrow or a round landed
/// beside me, or a blow landed, is the shot that did it: an arrow from as
/// far as a bow is heard is under a second in the air.
pub const PAIR_TICKS: u32 = 3 * TICK_HZ / 2;
/// Shot at and missed, the legs wait this long before they answer it: a
/// person's startle. A blow that lands is answered at once, as it always
/// was, and a shot while already running from fire keeps it running.
pub const STARTLE_TICKS: u32 = TICK_HZ / 4;
/// Turned toward a blow this long ago and nobody found who struck it: it
/// came from further off than a swing, and is run from as a shot is. About
/// what slow hands take to come round and the eyes to pick a body out.
pub const LOOKED_TICKS: u32 = TICK_HZ;
/// Where cover is taken from until the shooter is seen: this far along the
/// report's bearing, by how far off it sounded.
pub const NEAR_GUESS_M: f32 = 12.0;
pub const MID_GUESS_M: f32 = 40.0;
pub const FAR_GUESS_M: f32 = 90.0;
/// The body a guard under fire runs from before the eyes have it (no
/// entity is ever this id: `client_core::core::NO_VICTIM`).
const UNSEEN: u32 = u32::MAX;
/// A player holding a bow on me and standing still for this long has
/// stopped to shoot: the draw a person sees (the wire carries what is in
/// hand and where it looks, not whether it is drawn). Someone walking my
/// way with a bow in hand is not that.
pub const DRAWN_TICKS: u32 = TICK_HZ / 2;
pub const STILL_MPS: f32 = 0.5;
/// A shooter's aim counts as at me off its line by up to this share of the
/// range (~8°), and over me by up to this share (~14°, the arc of an
/// arrow at 30 m).
const LEAD_SPREAD: f32 = 0.15;
const ARC_RISE: f32 = 0.25;
/// A shooter further off than this is not charged with a melee weapon.
pub const CHARGE_M: f32 = 12.0;
/// Gaps wider than this are closed by route; nearer, straight in.
pub const ROUTE_M: f32 = 8.0;
/// Stand this far inside my own reach to swing, and this far outside
/// theirs to wait.
pub const HOLD_M: f32 = 0.3;
pub const SAFE_M: f32 = 0.5;
/// A spent swing waits this far outside their reach: their screen shows me
/// a playout ago, a step nearer when I have been backing off from a body
/// walking in, and the lunge back in needs a start.
pub const WAIT_M: f32 = 0.9;
/// The strafe reverses no sooner than this, and at most this much later.
pub const STRAFE_MIN_TICKS: u32 = 18;
pub const STRAFE_SPAN_TICKS: u32 = 24;
/// Their swing leaves time to step in, swing and step out when it is at
/// least this far from being ready again.
pub const STEP_IN_TICKS: u32 = 12;
/// Reaches are content centimetres summed in metres: a tie is a tie.
const REACH_EPS_M: f32 = 0.005;
/// While their swing is ready, a bait: in at a walk to this far inside
/// their reach, then out at a run to this far past it. A blow lands
/// `STRIKE_DELAY_TICKS` after it is swung, so one swung at the dip lands
/// on a body already on its way out, and their swing is a cadence spent.
pub const BAIT_IN_M: f32 = 0.15;
pub const BAIT_OUT_M: f32 = 0.6;
/// A body that will not swing at a bait this long is fought blow for blow.
pub const BAIT_MAX_TICKS: u32 = 2 * TICK_HZ;
/// Out of their reach while their swing comes back no more than this after
/// mine.
pub const STEP_OUT_TICKS: u32 = 4;
/// A ready swing is carried in at a run from no further than this past
/// my reach.
pub const LUNGE_M: f32 = 3.0;
/// A swing this near to ready, and ahead of theirs, is already on its way
/// in: it arrives in reach as it comes back, not a step after, and keeps
/// its lead. With none (the two come back together) it waits outside
/// their reach and lunges when ready: whoever moves in is nearer than the
/// other's screen shows, so the lunge lands first.
pub const LEAD_IN_TICKS: u32 = 6;
/// The button goes down this long before my swing is back, held: the
/// server swings the tick it is ready, and my count of it is an echo late.
pub const PRESS_EARLY_TICKS: u32 = 2;
/// The sideways share of a stride while in reach.
const STRAFE_SHARE: f32 = 0.6;
/// An escape weaves this far either side of its bearing (~22°).
const WEAVE: u16 = 0x1000;
/// A blow's author stands within this of its announced bearing (~62°; a
/// hurt sector is 45° wide).
const HURT_CONE: u16 = 0x2c00;
/// A thrown or shot weapon's reach when the wiki gives none, metres.
const FAR_M: f32 = 80.0;
/// With both to hand: the swung weapon inside this, the shooting one
/// beyond [`RANGED_OUT_M`], and between the two whichever is out already.
pub const MELEE_IN_M: f32 = 3.5;
pub const RANGED_OUT_M: f32 = 5.5;
/// An arrow is loosed at a body no further than this: past it the arc
/// and the lead are guesses.
pub const ARROW_MAX_M: f32 = 32.0;
/// A gun is fired within this share of its reach.
pub const HITSCAN_SHARE: f32 = 0.8;
/// A drawn bow looses this long after the draw is full by my count: the
/// press reaches the server a tick or so after it left.
pub const DRAW_SLACK_TICKS: u32 = 2;
/// Prey is stalked, crouched, to this far and shot from there: outside
/// what a pig hears of a crouched body in front of it.
pub const HUNT_SHOT_M: f32 = 16.0;
/// A player is shot at from no further than this, where a lead is short
/// enough to hold across a strafe; and at the head from inside this.
///
/// 14 → 26 when arrows took Rust's 50 m/s and two-hit kills: closing to
/// 14 m under a foe's flatter arrows cost the arena's 30 m bouts all four,
/// and from 26 m a shaft is half a second in the air.
pub const DUEL_M: f32 = 26.0;
pub const HEAD_SHOT_M: f32 = 12.0;
/// A foe that turned back across my line of sight this recently, crossing
/// it at least this fast, is weaving, and an arrow waits for its next turn:
/// no sooner after it than the eyes and the hands take to follow it, and
/// no later than this.
pub const WEAVE_TICKS: u32 = 2 * TICK_HZ;
pub const WEAVE_MPS: f32 = 1.0;
pub const WEAVE_SETTLE_TICKS: u32 = 7;
pub const WEAVE_FIRE_TICKS: u32 = 16;
/// A swung weapon closing inside this is backed away from while shooting.
pub const KITE_M: f32 = 8.0;

/// A bred brain's choices for a fight ([`Combat::set_tactics`]): fight with
/// the hand the range calls for (the hand-written choice), fight close,
/// fight at range, or run.
pub const TACTIC_FIGHT: u8 = 0;
pub const TACTIC_MELEE: u8 = 1;
pub const TACTIC_RANGED: u8 = 2;
pub const TACTIC_RUN: u8 = 3;
pub const TACTICS: usize = 4;
/// What it reads ([`tactics_features`]) and how wide it thinks.
pub const TACTICS_IN: usize = 24;
pub const TACTICS_HIDDEN: usize = 16;
/// The text a bred tactics genome starts with: shape 24-16-4 reading
/// [`tactics_features`]. A change to either is a new prefix.
pub const TACTICS_PREFIX: &[u8; 4] = b"ga1:";
/// A choice holds this long before the brain is asked again: half a
/// second, so it cannot flicker between a charge and a retreat.
pub const TACTIC_HOLD_TICKS: u32 = 15;

/// The bred brain every agent fights wolves with unless told otherwise
/// (`jev-bot --tactics`): `examples/zoo.rs --agent --seed 2`, four
/// mutations kept in 100 generations. On 192 fights no stage of breeding
/// saw (`--held-from 3001`), against the rule-table wolf it went down in 5%
/// of fights to the sum's 9% and lost 31% of its hp to the sum's 49%;
/// against the bred wolf it won 34% to 25%. Against people it is never
/// asked ([`Combat::tactic`]).
pub const DEFAULT_TACTICS: &str = "ga1:_kg9waqK-QGQkHgY";

pub fn default_tactics() -> sim_core::genome::Genome {
    sim_core::genome::Genome::decode(TACTICS_PREFIX, DEFAULT_TACTICS.as_bytes())
        .expect("DEFAULT_TACTICS is a ga1: genome")
}

/// The tactics brain's shape.
pub fn tactics_shape() -> sim_core::genome::Shape {
    sim_core::genome::Shape::new(TACTICS_IN, TACTICS_HIDDEN, TACTICS)
        .expect("the tactics net is inside genome.rs's caps")
}
/// A body aiming something that shoots at me from within this is dodged,
/// for this long at a stretch, and then not again for this long unless it
/// shoots: two bodies each dodging the other's aim would otherwise dance
/// with their goals waiting for as long as they stay in sight.
pub const EVADE_M: f32 = 40.0;
pub const DODGE_MAX_TICKS: u32 = 3 * TICK_HZ;
pub const DODGE_REST_TICKS: u32 = 10 * TICK_HZ;
/// Cover is looked for this far off, and thought again about this often.
pub const COVER_M: f32 = 24.0;
pub const COVER_RETHINK_TICKS: u32 = 3 * TICK_HZ;
/// Meds wait while a hostile body in sight stands this close.
pub const HEAL_SAFE_M: f32 = 15.0;
/// After a win: the bag is looked for this near where the foe fell, for
/// this long; the loot is given this long in all.
pub const BAG_MATCH_M: f32 = 4.0;
pub const BAG_WAIT_TICKS: u32 = 2 * TICK_HZ;
pub const LOOT_TICKS: u32 = 25 * TICK_HZ;
/// Stand this far inside the reach a take is judged at, and press again
/// after this long when nothing came of it; a stack left after this many
/// presses stays where it is.
pub const LOOT_STAND_M: f32 = LOOT_REACH_M - 2.0;
pub const VERB_RETRY_TICKS: u32 = TICK_HZ;
pub const LOOT_TRIES: u8 = 3;
/// Arrows lying this near are picked up after a win, up to this many.
pub const ARROW_SEEK_M: f32 = 25.0;
pub const PICKUPS_MAX: u8 = 12;
/// While its base is being struck, a player swinging or shooting this near
/// it is the one doing it: the base's footprint and a reach beyond.
pub const RAID_M: f32 = 12.0;
/// Eye height above the feet standing, metres (where a swing leaves from:
/// a melee fight never presses crouch). A shot's eye is its stance's
/// ([`eye_m`]).
const EYE_M: f32 = ARROW_EYE_MM as f32 / MM_PER_M;

/// How ready this body is to start a fight (`--temperament`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Temperament {
    /// Runs from every fight.
    Passive,
    /// Answers attacks; never starts one.
    Defensive,
    /// Answers attacks; starts one only with a clear edge.
    #[default]
    Opportunist,
    /// Starts any fight it is not losing.
    Kos,
}

impl Temperament {
    pub const ALL: [Temperament; 4] = [
        Temperament::Passive,
        Temperament::Defensive,
        Temperament::Opportunist,
        Temperament::Kos,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Temperament::Passive => "passive",
            Temperament::Defensive => "defensive",
            Temperament::Opportunist => "opportunist",
            Temperament::Kos => "kos",
        }
    }

    pub fn parse(name: &str) -> Option<Temperament> {
        Self::ALL.into_iter().find(|t| t.name() == name)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Idle,
    Guard,
    Engage,
    Escape,
    Heal,
    Loot,
    Downed,
}

/// How a fight ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum End {
    /// The foe died.
    Won,
    /// Got away from a fight it was losing.
    Escaped,
    /// Nobody won: the foe left, stopped, or was never found.
    Parted,
}

/// A retreat in progress.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Retreat {
    /// Tick of the last renewal.
    pub start: u32,
    /// The world bearing it backs away along.
    pub away: u16,
    /// Where the route is taking it, metres; set on the first frame.
    to: Option<[f32; 2]>,
}

/// The one body a fight is with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Foe {
    pub id: u32,
    pub since: u32,
    /// Last tick it struck, swung or shot at me.
    pub threat: u32,
    /// Last tick it actually swung or shot at me or a blow landed: what
    /// keeps a fight in the safe zone answered.
    pub struck: Option<u32>,
    /// Told to (a fight or hunt goal): no temperament asked.
    pub commanded: bool,
    /// My hit markers on an animal when this fight began: it heals
    /// between fights, so only what landed since counts against it.
    pub dealt_from: Option<u32>,
    /// Where it last stood in sight, and when I last swung or fired at it:
    /// an animal that drops out of the snapshot right after, near and in
    /// plain sight, fell (`VANISHED_KILL_TICKS`).
    pub seen: Option<[f32; 3]>,
    pub fired_at: Option<u32>,
    /// An animal: the most my markers have dealt it this fight, and the
    /// tick that grew last (`NO_HARM_TICKS`).
    pub harmed: (u32, u32),
}

/// Shot at by a body not yet seen: when it was heard, which way the
/// report came from (a world wire yaw toward it) and how far off it
/// sounded, and whether the reflex has taken it up yet (a miss waits out
/// the startle).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Fire {
    at: u32,
    toward: u16,
    band: Band,
    taken: bool,
}

/// What the reflex made of this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Assess {
    /// Nothing to answer: run the goal.
    Calm,
    /// Answering: this frame belongs to the fight, and the goal waits.
    Fight(Intent),
    /// The fight ended this frame; a retreat's bearing, if it was one.
    Over { away: Option<u16>, end: End },
}

/// What the body brings to a fight this frame.
#[derive(Clone, Copy, Debug)]
pub struct Kit<'a> {
    pub book: &'a Book,
    /// The best swung weapon on the belt: slot and wire item.
    pub melee: Option<(u8, u16)>,
    /// The best shooting weapon on the belt that has something to shoot:
    /// slot and wire item.
    pub ranged: Option<(u8, u16)>,
    /// Rounds for it in the pack, and loaded in it when it is in hand and
    /// the readout is known (`ClientCore::mag`).
    pub rounds: u32,
    pub loaded: Option<u16>,
    pub hp: u16,
    pub hp_max: u16,
    /// The hands were on their target last frame, and had been for a few.
    pub on_target: bool,
    pub settled: bool,
    /// Ticks from the pose my screen shows another body in to my next
    /// input landing on the server: the playout plus the input's round
    /// trip. What an arrow, which is judged live, has to lead by.
    pub lead_ticks: u32,
    /// The action lane is free this frame: a take pressed now is sent.
    pub lane_free: bool,
    /// Where its own base stands, while blows, breaks or blasts there are
    /// fresh (`Home::raided`): a player at work beside it is attacking me.
    pub raided: Option<[f32; 2]>,
    /// Standing inside its own base: its walls are its cover, and running
    /// out of them is no escape. A fight met here is stood.
    pub home_ground: bool,
    /// The town on the map (`Haven::town`): inside its safe zone no player
    /// hurts another, so no fight is started there or into it, and one is
    /// only answered while it strikes.
    pub town: Town,
}

/// A verb the reflex wants sent; the orchestrator owns the action lane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verb {
    /// Empty the nearest bag in reach.
    Loot,
    /// Take the nearest loose stack in reach.
    Pickup,
}

/// Which hand the fight is in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Arm {
    #[default]
    Melee,
    Ranged,
}

/// After a win: where the foe fell, and how the taking is going.
#[derive(Clone, Copy, Debug)]
struct Spoils {
    at: [f32; 2],
    since: u32,
    /// Still after the bag (else the arrows).
    bag: bool,
    /// The stack pressed for, when, and how many presses it has had.
    sent: Option<(u32, u32, u8)>,
    /// A stack given up on.
    skip: Option<u32>,
    picks: u8,
    /// The foe was a player: its bag is a pack, not a carcass (wire v84
    /// `WireBag::kind`), so a pig killed beside it is not looted for it.
    pack: bool,
}

/// The sum a player does before and during a fight.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Odds {
    /// Blows each side needs.
    pub mine: u32,
    pub theirs: u32,
    /// Centre-to-centre distance a swing lands from, metres.
    pub my_reach: f32,
    pub their_reach: f32,
    /// Ticks between their blows.
    pub their_cadence: u32,
    pub armed: bool,
    pub ranged: bool,
    /// Something makes this fight clearly mine.
    pub edge: bool,
    /// Staying is dying.
    pub losing: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct CombatStats {
    pub guards: u64,
    pub engages: u64,
    pub escapes: u64,
    pub won: u64,
    pub escaped: u64,
    pub parted: u64,
    /// Shots I loosed (my own echoes), and PRIMARY frames asked to loose.
    pub shots: u64,
    /// Sidesteps from a body aiming at me.
    pub evades: u64,
    /// Escapes that found cover to run to.
    pub covers: u64,
    /// Guards begun under fire from a body not yet seen: a round cracking
    /// past, an arrow landing beside it after a report, a blow with a
    /// report to match or from nobody in sight.
    pub shot_at: u64,
    /// Bags and stacks asked for after a win.
    pub loots: u64,
    pub pickups: u64,
}

impl CombatStats {
    /// Retreats of any kind begun.
    pub fn retreats(&self) -> u64 {
        self.guards + self.escapes
    }
}

pub struct Combat {
    temperament: Temperament,
    mode: Mode,
    retreat: Option<Retreat>,
    foe: Option<Foe>,
    /// The last blow taken: tick and world bearing toward its source.
    hurt: Option<(u32, u16)>,
    /// The last shot at me from a body not yet seen.
    fire: Option<Fire>,
    /// How many sounds the ears had heard by the last frame
    /// ([`Tracks::heard_since`]).
    ears: u64,
    /// Strafe side (+1 right, −1 left) and when it next reverses.
    strafe: (i8, u32),
    /// When my own next swing is ready, from the last one it felt.
    ready_at: u32,
    /// When the swing it felt lands: until then the arm is on its way in,
    /// and backing off or sidestepping takes the blow out of reach.
    strike_at: u32,
    /// The last frame that pressed with the swing ready: the server takes
    /// the swing as it runs, so the frames after it are the wind-up's.
    pressed_at: u32,
    /// Baiting a ready swing: at which body, since when, and whether the
    /// step is on its way back out.
    bait: Option<(u32, u32, bool)>,
    /// Trying to close on the foe and not getting nearer: since when, and
    /// the nearest it has been since.
    stuck: Option<(u32, f32)>,
    /// Bodies found unreachable, and until when each is not picked again.
    shunned: [Option<(u32, u32)>; SHUN_MAX],
    last_end: Option<End>,
    /// Which weapon the fight is in, for the band where either would do.
    arm: Arm,
    /// A bow's draw: since when, and in which slot.
    draw: Option<(u32, u8)>,
    /// When my next shot is ready, from the last one echoed, and the gap a
    /// shot in hand leaves.
    shot_ready: u32,
    fire_gap: u32,
    /// What the escape runs from: its body, where it last stood, and
    /// whether it shoots.
    threat: Option<(u32, [f32; 2], bool)>,
    /// Cover the escape is making for, and when it was last looked for
    /// (found or not: the scan is not cheap).
    cover: Option<[f32; 2]>,
    cover_tried: Option<u32>,
    /// A body aiming at me, being sidestepped: until when, and since when.
    dodge: Option<(u32, u32, u32)>,
    /// A body dodged its full stretch, and until when it is not again.
    dodged: Option<(u32, u32)>,
    spoils: Option<Spoils>,
    verb: Option<Verb>,
    /// The side a foe was last seen stepping across my line of sight,
    /// which foe, and when it last turned back.
    lat: (i8, u32, Option<u32>),
    /// My own inputs the server has not run yet as of my newest snapshot.
    pub strides: Strides,
    /// The body stepping in on me, and since when.
    menace: Option<(u32, u32)>,
    /// The body standing with a bow on me ([`drawing`]), and since when.
    drawer: Option<(u32, u32)>,
    /// The last look round at footsteps: its bearing, and when it began.
    glance: Option<(u16, u32)>,
    /// Since when a shot at a weaving player has been ready but for the
    /// moment in its weave.
    held_shot: Option<u32>,
    /// A bred brain making the fight-or-run call ([`Combat::set_tactics`]).
    tactics: Option<Box<sim_core::genome::Net>>,
    /// Its last choice: the choice, since when, and against which body.
    tactic: Option<(u8, u32, u32)>,
    rng: Pcg32,
    pub stats: CombatStats,
}

/// Ticks of my own inputs remembered for [`Strides`].
const STRIDE_RING: usize = 16;

/// Where my own inputs have taken my body that my snapshots do not show
/// yet: the human client's prediction, on flat ground and without the
/// walls (`movement::step`'s wish and speed). A trip of input is a step or
/// two at a run, the difference between a blow that lands and one that
/// falls short.
#[derive(Clone, Copy, Debug)]
pub struct Strides {
    /// Each sent input's ground displacement, by its sequence.
    ring: [(u16, [f32; 2]); STRIDE_RING],
    /// The sequence the server last ran, as my newest snapshot says, and
    /// the one this frame goes out as.
    ran: u16,
    seq: u16,
}

impl Default for Strides {
    fn default() -> Self {
        Self {
            ring: [(0, [0.0; 2]); STRIDE_RING],
            ran: 0,
            seq: 0,
        }
    }
}

impl Strides {
    /// A frame begins: the snapshot says the server ran `ran`; this one
    /// goes out as `seq`.
    pub fn begin(&mut self, ran: u16, seq: u16) {
        self.ran = ran;
        self.seq = seq;
    }

    /// The frame that went out.
    pub fn sent(&mut self, f: &InputFrame) {
        let (fx, fz) = yaw_dir(f.yaw);
        let (rx, rz) = (fz, -fx);
        let (mf, ms) = (f32::from(f.move_z) / 127.0, f32::from(f.move_x) / 127.0);
        let (mut wx, mut wz) = (fx * mf + rx * ms, fz * mf + rz * ms);
        let len2 = wx * wx + wz * wz;
        if len2 > 1.0 {
            let inv = 1.0 / len2.sqrt();
            wx *= inv;
            wz *= inv;
        }
        let speed = if f.buttons & BTN_SPRINT != 0 && f.buttons & BTN_AIM == 0 {
            SPRINT_SPEED
        } else {
            WALK_SPEED
        };
        let k = speed / TICK_HZ as f32;
        self.ring[usize::from(f.seq) % STRIDE_RING] = (f.seq, [wx * k, wz * k]);
    }

    /// Ticks from my newest snapshot to this frame's input running: the
    /// inputs still in flight, and this one.
    pub fn trip(&self) -> u32 {
        u32::from(self.seq.wrapping_sub(self.ran)).clamp(1, STRIDE_RING as u32)
    }

    /// `p` (from my newest snapshot) carried on by the inputs sent since
    /// it, which the server runs before this frame's.
    pub fn ahead_of(&self, p: [f32; 3]) -> [f32; 3] {
        let pending = self.seq.wrapping_sub(self.ran).saturating_sub(1);
        let mut out = p;
        for k in 1..=pending.min(STRIDE_RING as u16 - 1) {
            let s = self.ran.wrapping_add(k);
            let (was, d) = self.ring[usize::from(s) % STRIDE_RING];
            if was == s {
                out[0] += d[0];
                out[2] += d[1];
            }
        }
        out
    }
}

impl Default for Combat {
    fn default() -> Self {
        Self::new(Temperament::default())
    }
}

impl Combat {
    pub fn new(temperament: Temperament) -> Self {
        Self {
            temperament,
            mode: Mode::Idle,
            retreat: None,
            foe: None,
            hurt: None,
            fire: None,
            ears: 0,
            strafe: (1, 0),
            ready_at: 0,
            strike_at: 0,
            pressed_at: 0,
            bait: None,
            stuck: None,
            shunned: [None; SHUN_MAX],
            last_end: None,
            arm: Arm::Melee,
            draw: None,
            shot_ready: 0,
            fire_gap: 1,
            threat: None,
            cover: None,
            cover_tried: None,
            dodge: None,
            dodged: None,
            spoils: None,
            verb: None,
            lat: (0, 0, None),
            strides: Strides::default(),
            menace: None,
            drawer: None,
            glance: None,
            held_shot: None,
            tactics: None,
            tactic: None,
            rng: Pcg32::new(0, 0),
            stats: CombatStats::default(),
        }
    }

    /// Let a bred brain make the fight-or-run call and choose the hand
    /// against wolves ([`tactics_shape`]); `None` puts the sum back.
    pub fn set_tactics(&mut self, net: Option<sim_core::genome::Net>) {
        self.tactics = net.map(Box::new);
        self.tactic = None;
    }

    /// The bred brain's choice against `t`, held [`TACTIC_HOLD_TICKS`];
    /// `None` with no brain set.
    #[allow(clippy::too_many_arguments)]
    fn tactic(
        &mut self,
        t: &Track,
        odds: &Odds,
        kit: &Kit,
        others: u32,
        me: [f32; 3],
        d: f32,
        tick: u32,
    ) -> Option<u8> {
        // Wolves only: the brain is bred and judged against packs and the
        // arena's stand-in players (`examples/zoo.rs --agent`), and held out
        // it beat the sum against packs and lost to it against people, two
        // runs of two. Against a person, or prey it never met, the sum makes
        // the call, so every ceiling the arena holds the sum to still holds.
        if t.species != Species::Wolf {
            return None;
        }
        let net = self.tactics.as_ref()?;
        if let Some((c, since, id)) = self.tactic {
            if id == t.id && tick.wrapping_sub(since) < TACTIC_HOLD_TICKS {
                return Some(c);
            }
        }
        let struck = self
            .hurt
            .is_some_and(|(at, _)| tick.wrapping_sub(at) < ALARM_TICKS);
        let c = net.choose(&tactics_features(t, odds, kit, others, me, d, struck, tick));
        self.tactic = Some((c, tick, t.id));
        Some(c)
    }

    /// The hand the bred brain chose for this foe, when it chose one the
    /// belt can use; a dry gun is still never chosen in someone's face.
    fn chosen_arm(&self, foe: u32, kit: &Kit, d: f32) -> Option<Arm> {
        match self.tactic.filter(|&(_, _, id)| id == foe)?.0 {
            TACTIC_MELEE if kit.melee.is_some() => Some(Arm::Melee),
            TACTIC_RANGED if kit.ranged.is_some() && !(kit.loaded == Some(0) && d < KITE_M) => {
                Some(Arm::Ranged)
            }
            _ => None,
        }
    }

    /// A new session: the strafe rhythm is drawn from this body's stream.
    pub fn reset(&mut self, seed: u64, player: u32) {
        self.rng = Pcg32::new(seed ^ 0x4649_4748, u64::from(player));
        self.forget();
    }

    /// A new body, or one that went down: nothing it was fighting or
    /// running from is in front of it now.
    pub fn forget(&mut self) {
        self.tactic = None;
        self.mode = Mode::Idle;
        self.retreat = None;
        self.foe = None;
        self.hurt = None;
        self.fire = None;
        self.stuck = None;
        self.shunned = [None; SHUN_MAX];
        self.draw = None;
        self.threat = None;
        self.cover = None;
        self.cover_tried = None;
        self.dodge = None;
        self.dodged = None;
        self.spoils = None;
        self.verb = None;
        self.drawer = None;
        self.glance = None;
        self.held_shot = None;
    }

    /// Down: the orchestrator crawls; the reflex waits for the body.
    pub fn down(&mut self) {
        self.forget();
        self.mode = Mode::Downed;
    }

    pub fn temperament(&self) -> Temperament {
        self.temperament
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn foe(&self) -> Option<Foe> {
        self.foe
    }

    /// How the last fight ended.
    pub fn last_end(&self) -> Option<End> {
        self.last_end
    }

    /// The retreat running now (guarding or escaping), as (last renewal,
    /// bearing).
    pub fn retreat(&self) -> Option<(u32, u16)> {
        self.retreat.map(|r| (r.start, r.away))
    }

    /// In a fight: a frame now goes to the reflex, not the goal. Taking
    /// the loser's bag is part of it.
    pub fn engaged(&self) -> bool {
        matches!(
            self.mode,
            Mode::Guard | Mode::Engage | Mode::Escape | Mode::Loot
        )
    }

    /// The verb this frame's reflex wants sent, once.
    pub fn take_verb(&mut self) -> Option<Verb> {
        self.verb.take()
    }

    /// My own shot was echoed: the next is a weapon's cadence away.
    pub fn shot(&mut self, tick: u32) {
        self.stats.shots += 1;
        self.shot_ready = tick.wrapping_add(self.fire_gap);
    }

    /// A blow landed, from this bearing's opposite: the hit indicator a
    /// human gets, never a position. An alarm: guard toward it until its
    /// author is found; a running retreat takes the new bearing.
    pub fn on_hurt(&mut self, tick: u32, away: u16) {
        self.hurt = Some((tick, away.wrapping_add(1 << 15)));
        match self.mode {
            Mode::Engage => {}
            Mode::Escape => self.run(Mode::Escape, tick, away),
            _ => self.run(Mode::Guard, tick, away),
        }
    }

    /// Run from a body the eyes found (a flee goal).
    pub fn flee(&mut self, tick: u32, away: u16) {
        self.run(Mode::Escape, tick, away);
    }

    /// My own arm moved (the server's swing echo, at a tree or a body): the
    /// next swing is a cadence away. A player feels the swing, and knows
    /// the rhythm.
    pub fn swung(&mut self, tick: u32) {
        self.ready_at = tick.wrapping_add(SWING_INTERVAL_TICKS as u32);
        self.strike_at = tick.wrapping_add(STRIKE_DELAY_TICKS as u32);
    }

    /// A body a fight was let go with because it could not be reached:
    /// not worth picking again yet.
    pub fn shuns(&self, id: u32, tick: u32) -> bool {
        self.shunned
            .iter()
            .flatten()
            .any(|&(who, until)| who == id && tick.wrapping_sub(until) >= 1 << 31)
    }

    /// Let `id` be for [`SHUN_TICKS`]: in its own place, a lapsed one's, or
    /// the one that lapses soonest.
    fn shun(&mut self, id: u32, tick: u32) {
        let list = &self.shunned;
        let slot = list
            .iter()
            .position(|e| e.is_some_and(|(who, _)| who == id))
            .or_else(|| {
                list.iter()
                    .position(|e| e.is_none_or(|(_, until)| tick.wrapping_sub(until) < 1 << 31))
            })
            .or_else(|| {
                (0..SHUN_MAX).min_by_key(|&i| list[i].map_or(0, |(_, u)| u.wrapping_sub(tick)))
            })
            .unwrap_or(0);
        self.shunned[slot] = Some((id, tick.wrapping_add(SHUN_TICKS)));
    }

    /// Fight this body because the mind said so (a fight or hunt goal).
    pub fn command(&mut self, id: u32, tick: u32) {
        self.engage(id, tick, true);
    }

    fn run(&mut self, mode: Mode, tick: u32, away: u16) {
        if self.mode != mode {
            match mode {
                Mode::Guard => self.stats.guards += 1,
                _ => self.stats.escapes += 1,
            }
        }
        self.mode = mode;
        self.foe = None;
        self.retreat = Some(Retreat {
            start: tick,
            away,
            to: None,
        });
    }

    fn engage(&mut self, id: u32, tick: u32, commanded: bool) {
        if self.foe.map(|f| f.id) != Some(id) {
            self.stats.engages += 1;
            self.foe = Some(Foe {
                id,
                since: tick,
                threat: tick,
                // A blow just taken is what an answered fight answers.
                struck: self
                    .hurt
                    .filter(|(at, _)| tick.wrapping_sub(*at) < ALARM_TICKS)
                    .map(|(at, _)| at),
                commanded,
                dealt_from: None,
                seen: None,
                fired_at: None,
                harmed: (0, tick),
            });
            self.stuck = None;
        }
        if let Some(f) = self.foe.as_mut() {
            f.commanded |= commanded;
        }
        self.mode = Mode::Engage;
        self.retreat = None;
        self.threat = None;
    }

    fn over(&mut self, away: Option<u16>, end: End) -> Assess {
        match end {
            End::Won => self.stats.won += 1,
            End::Escaped => self.stats.escaped += 1,
            End::Parted => self.stats.parted += 1,
        }
        self.mode = Mode::Idle;
        self.retreat = None;
        self.foe = None;
        self.stuck = None;
        self.bait = None;
        self.draw = None;
        self.held_shot = None;
        self.threat = None;
        self.cover = None;
        self.cover_tried = None;
        self.spoils = None;
        self.fire = self.fire.filter(|f| !f.taken);
        self.last_end = Some(end);
        Assess::Over { away, end }
    }

    /// One frame of the reflex.
    pub fn assess(
        &mut self,
        core: &mut ClientCore,
        body: &EntityState,
        tracks: &Tracks,
        route: &mut Route,
        kit: &Kit,
        tick: u32,
    ) -> Assess {
        let me = pos(body);
        self.verb = None;
        self.size_up(tracks, kit.book, me, tick);
        self.listen(tracks, kit, me, tick);
        self.take_fire(me, tick);
        match self.mode {
            Mode::Idle | Mode::Downed | Mode::Heal => {
                self.mode = Mode::Idle;
                if let Some(id) = self.attacker(tracks, kit, me, tick) {
                    self.dodge = None;
                    return self.answer(core, body, tracks, route, kit, id, tick);
                }
                if let Some(id) = self.opening(tracks, kit, me, tick) {
                    self.dodge = None;
                    self.engage(id, tick, false);
                    return self.fight(core, body, tracks, route, kit, tick);
                }
                self.sidestep(tracks, kit.book, me, tick)
            }
            Mode::Loot => {
                if let Some(id) = self.attacker(tracks, kit, me, tick) {
                    self.spoils = None;
                    return self.answer(core, body, tracks, route, kit, id, tick);
                }
                self.loot(core, body, tracks, route, kit, tick)
            }
            Mode::Guard => {
                if let Some(id) = self.attacker(tracks, kit, me, tick) {
                    return self.answer(core, body, tracks, route, kit, id, tick);
                }
                // Turned to the blow and nobody there who struck it: it
                // came from further off than a swing reaches.
                let unfound = self.hurt.filter(|&(at, _)| {
                    let age = tick.wrapping_sub(at);
                    (LOOKED_TICKS..ALARM_TICKS).contains(&age)
                });
                if let Some((_, toward)) = unfound.filter(|_| !self.under_fire()) {
                    self.cover_from(me, toward, Band::Mid, false, tick);
                }
                let fired = self.under_fire();
                self.back_away(core, body, tracks, route, tick, fired)
            }
            Mode::Escape => self.back_away(core, body, tracks, route, tick, true),
            Mode::Engage => self.fight(core, body, tracks, route, kit, tick),
        }
    }

    /// Run from this body: away from it, and to cover when it shoots.
    fn flee_from(&mut self, t: &Track, ranged: bool, me: [f32; 3], tick: u32) {
        self.run(
            Mode::Escape,
            tick,
            yaw_toward(me[0] - t.pos[0], me[2] - t.pos[2]),
        );
        self.threat = Some((t.id, [t.pos[0], t.pos[2]], ranged));
        self.cover = None;
        self.cover_tried = None;
        self.draw = None;
    }

    /// What the ears make of the sounds heard since the last frame. A round
    /// cracking past, or an arrow or a round landing beside me just after a
    /// report, is a shot at me from where the report came, and so is a blow
    /// that lands with a report to match its side: a person knows that much,
    /// and roughly from where, before they have seen anyone. Not in the safe
    /// zone, where no player's shot can hurt me (a blow still counts), and
    /// never from a teammate, whose reports the ears do not listen for
    /// (`Tracks::on_shot`). What was heard while the body could not answer
    /// (down, asleep) is history by the time it can.
    fn listen(&mut self, tracks: &Tracks, kit: &Kit, me: [f32; 3], tick: u32) {
        let mark = self.ears;
        self.ears = tracks.stats.heard;
        let safe = in_zone(kit, me);
        for h in tracks.heard_since(mark) {
            if tick.wrapping_sub(h.tick) >= ALARM_TICKS {
                continue;
            }
            let from = match h.sound {
                Sound::Whiz if !safe => Some(*h),
                Sound::Impact if !safe => report(tracks, h, None),
                Sound::Hurt => report(tracks, h, Some(h.bearing)),
                _ => None,
            };
            if let Some(r) = from {
                self.fire = Some(Fire {
                    at: h.tick,
                    toward: r.bearing,
                    band: r.band,
                    taken: false,
                });
            }
        }
    }

    /// Take up a shot at me once the startle has passed, at once if a blow
    /// landed with it or the body is already running from fire: the guard
    /// runs for cover from where the report came. A fight under way goes
    /// on, and a run from a body seen runs on.
    fn take_fire(&mut self, me: [f32; 3], tick: u32) {
        let Some(f) = self.fire.filter(|f| !f.taken) else {
            return;
        };
        let blow = self
            .hurt
            .is_some_and(|(at, _)| tick.wrapping_sub(at) < ALARM_TICKS);
        if !blow && !self.under_fire() && tick.wrapping_sub(f.at) < STARTLE_TICKS {
            return;
        }
        self.fire = Some(Fire { taken: true, ..f });
        match self.mode {
            Mode::Engage => {}
            Mode::Escape => {
                if let Some(r) = self.retreat.as_mut() {
                    r.start = tick;
                }
            }
            _ => self.cover_from(me, f.toward, f.band, blow, tick),
        }
    }

    /// Guard from a shooter not yet seen, `toward` that way and about as
    /// far as `band` sounded: back away from it, weaving to cover, eyes on
    /// where it must be. A new side, or a blow taken on the way, looks for
    /// cover again.
    fn cover_from(&mut self, me: [f32; 3], toward: u16, band: Band, blow: bool, tick: u32) {
        let m = match band {
            Band::Near => NEAR_GUESS_M,
            Band::Mid => MID_GUESS_M,
            Band::Far => FAR_GUESS_M,
        };
        let (dx, dz) = yaw_dir(toward);
        let at = [me[0] + dx * m, me[2] + dz * m];
        let moved = match self.threat {
            Some((UNSEEN, was, true)) if self.under_fire() => {
                off(yaw_toward(was[0] - me[0], was[1] - me[2]), toward) > GLANCE_CONE
            }
            _ => {
                self.stats.shot_at += 1;
                true
            }
        };
        if moved || blow {
            self.cover = None;
            self.cover_tried = None;
        }
        self.run(Mode::Guard, tick, toward.wrapping_add(1 << 15));
        self.threat = Some((UNSEEN, at, true));
        self.draw = None;
    }

    /// Guarding from a shooter not yet seen.
    fn under_fire(&self) -> bool {
        self.mode == Mode::Guard && self.threat.is_some_and(|(_, _, ranged)| ranged)
    }

    /// A stranger worth keeping an eye on while the goal runs: a player
    /// with something in hand, near, just spotted (sized up for a moment)
    /// or coming my way. A person does not turn their back on someone
    /// walking up to them with a rock; the charge is then seen, and met.
    pub fn watch(&mut self, tracks: &Tracks, book: &Book, me: [f32; 3], tick: u32) -> Option<Look> {
        let my_vel = tracks.own().map_or([0.0; 3], |o| o.vel);
        let mut best: Option<(f32, u32)> = None;
        for t in tracks.seen() {
            if !t.active() || t.species != Species::Player || t.wounded {
                continue;
            }
            let (reach, ranged) = their_reach(t, book);
            let d = flat(me, t.pos);
            if (reach <= 0.0 && !ranged) || d > WARY_M || best.is_some_and(|(b, _)| d >= b) {
                continue;
            }
            // Glimpsed at the edge of the view and gone: the eyes go back
            // to where it was. Once sized up, one that keeps its distance
            // is let be, staring or not: the goal goes on, and its next
            // step in is seen. One walking beside me to the same tree is
            // not stepping in.
            let fresh = tick.wrapping_sub(t.first_seen) < SIZE_UP_TICKS;
            if fresh || (t.visible && nearing(t, me, my_vel, d)) {
                best = Some((d, t.id));
            }
        }
        if let Some((_, id)) = best {
            return Some(Look::Body {
                id,
                part: Part::Chest.bits(),
            });
        }
        // Steps behind me or off to the side, or a shot, from someone I
        // have not seen about: a look round.
        let step = tracks
            .heard()
            .filter(|h| {
                matches!(h.sound, Sound::Step | Sound::Shot)
                    && tick.wrapping_sub(h.tick) < GLANCE_TICKS
            })
            .last()?;
        let near = |a: u16, b: u16| (a.wrapping_sub(b) as i16).unsigned_abs() <= GLANCE_CONE;
        let known = tracks
            .recent(tick, GLANCE_KNOWN_TICKS)
            .any(|t| near(yaw_toward(t.pos[0] - me[0], t.pos[2] - me[2]), step.bearing));
        if known {
            return None;
        }
        match self.glance {
            // Looked that way already: one more look while this one lasts,
            // then the steps are let be for a while.
            Some((b, since)) if near(b, step.bearing) => {
                let age = tick.wrapping_sub(since);
                if age < GLANCE_LOOK_TICKS {
                    return Some(Look::Heading(step.bearing));
                }
                if age < GLANCE_REST_TICKS {
                    return None;
                }
            }
            _ => {}
        }
        self.glance = Some((step.bearing, tick));
        Some(Look::Heading(step.bearing))
    }

    /// Keep count of the nearest body stepping in on me with something
    /// swung in hand ([`rushing`]), and of the nearest standing with a bow
    /// on me ([`drawing`]), and since when each has been.
    fn size_up(&mut self, tracks: &Tracks, book: &Book, me: [f32; 3], tick: u32) {
        let my_vel = tracks.own().map_or([0.0; 3], |o| o.vel);
        let low = me_low(tracks);
        let mut near: Option<(f32, u32)> = None;
        let mut bow: Option<(f32, u32)> = None;
        for t in tracks.seen() {
            if t.visible && drawing(t, me, low, book) {
                let d = flat(me, t.pos);
                if bow.is_none_or(|(b, _)| d < b) {
                    bow = Some((d, t.id));
                }
            }
            // Seen swinging lately with none of it at me: busy at the trees
            // and the rocks, walking to the next, not coming for me. Its
            // next swing at me is answered as any swing is.
            let busy = t
                .last_swing
                .is_some_and(|at| tick.wrapping_sub(at) < GATHERING_TICKS);
            if !t.visible || !t.active() || busy {
                continue;
            }
            let d = flat(me, t.pos);
            let (reach, ranged) = their_reach(t, book);
            if rushing(t, me, low, my_vel, d, reach, ranged) && near.is_none_or(|(b, _)| d < b) {
                near = Some((d, t.id));
            }
        }
        let held = |was: Option<(u32, u32)>, now: Option<(f32, u32)>| {
            now.map(|(_, id)| match was {
                Some((who, since)) if who == id => (id, since),
                _ => (id, tick),
            })
        };
        self.menace = held(self.menace, near);
        self.drawer = held(self.drawer, bow);
    }

    /// A body aiming something that shoots at me, not yet shooting: step
    /// side to side while it does, eyes on it, and let the goal wait.
    fn sidestep(&mut self, tracks: &Tracks, book: &Book, me: [f32; 3], tick: u32) -> Assess {
        let rested = |until: u32| tick.wrapping_sub(until) < 1 << 31;
        if self.dodged.is_some_and(|(_, until)| rested(until)) {
            self.dodged = None;
        }
        let mut aimer: Option<(f32, &Track)> = None;
        let low = me_low(tracks);
        for t in tracks.seen() {
            if !t.visible || !t.active() || t.species != Species::Player || !aimed(t, me, low, book)
            {
                continue;
            }
            // Dodged its stretch already: a shot from it is an attack
            // ([`Combat::attacker`]); a held aim alone is not, for a while.
            if self.dodged.is_some_and(|(id, _)| id == t.id) {
                continue;
            }
            let fires = t.held.is_some_and(|h| book.page(h).fires());
            let d = flat(me, t.pos);
            if fires && d <= EVADE_M && aimer.is_none_or(|(b, _)| d < b) {
                aimer = Some((d, t));
            }
        }
        if let Some((_, t)) = aimer {
            let since = match self.dodge {
                Some((id, _, since)) if id == t.id => since,
                _ => {
                    self.stats.evades += 1;
                    tick
                }
            };
            self.dodge = Some((t.id, tick.wrapping_add(TICK_HZ), since));
        }
        let Some((id, until, since)) = self.dodge else {
            return Assess::Calm;
        };
        let Some(t) = tracks.get(id).filter(|t| t.active()) else {
            self.dodge = None;
            return Assess::Calm;
        };
        if rested(until) {
            self.dodge = None;
            return Assess::Calm;
        }
        if tick.wrapping_sub(since) >= DODGE_MAX_TICKS {
            self.dodge = None;
            self.dodged = Some((id, tick.wrapping_add(DODGE_REST_TICKS)));
            return Assess::Calm;
        }
        self.rhythm(tick);
        let toward = yaw_toward(t.pos[0] - me[0], t.pos[2] - me[2]);
        let side = if self.strafe.0 > 0 {
            1u16 << 14
        } else {
            3u16 << 14
        };
        Assess::Fight(Intent {
            look: Look::Body {
                id,
                part: Part::Chest.bits(),
            },
            travel: Some(toward.wrapping_add(side)),
            buttons: BTN_SPRINT,
            ..Intent::IDLE
        })
    }

    /// The foe has turned back across my line of sight lately: it is
    /// stepping side to side, not running somewhere. How long ago it last
    /// turned.
    fn weaving(&mut self, t: &Track, toward: u16, tick: u32) -> Option<u32> {
        let (ux, uz) = yaw_dir(toward);
        let across = t.vel[0] * uz - t.vel[2] * ux;
        let side: i8 = if across > WEAVE_MPS {
            1
        } else if across < -WEAVE_MPS {
            -1
        } else {
            0
        };
        let (was, id, _) = self.lat;
        if id != t.id {
            self.lat = (side, t.id, None);
            return None;
        }
        if side != 0 && was != 0 && side != was {
            self.lat = (side, id, Some(tick));
        } else if side != 0 {
            self.lat.0 = side;
        }
        self.lat
            .2
            .map(|at| tick.wrapping_sub(at))
            .filter(|&since| since < WEAVE_TICKS)
    }

    /// The strafe's side, reversed on a person's uneven rhythm.
    fn rhythm(&mut self, tick: u32) {
        if tick.wrapping_sub(self.strafe.1) < 1 << 31 {
            let span = self.rng.next_bounded(STRAFE_SPAN_TICKS + 1);
            self.strafe = (-self.strafe.0, tick + STRAFE_MIN_TICKS + span);
        }
    }

    /// Attacked by this body: fight back, unless this body never fights or
    /// the fight is already lost.
    #[allow(clippy::too_many_arguments)]
    fn answer(
        &mut self,
        core: &mut ClientCore,
        body: &EntityState,
        tracks: &Tracks,
        route: &mut Route,
        kit: &Kit,
        id: u32,
        tick: u32,
    ) -> Assess {
        let Some(t) = tracks.get(id).copied() else {
            return Assess::Calm;
        };
        let me = pos(body);
        let others = crowd(tracks, id, me, tick);
        let odds = odds(&t, kit, others);
        let d = flat(me, t.pos);
        // Shot at from past a charge with nothing to shoot back: cover. At
        // home the walls are the cover, and the fight is stood unless it is
        // all but lost.
        let out_shot = odds.ranged && d > CHARGE_M && kit.ranged.is_none();
        let stand = kit.home_ground && u32::from(kit.hp) * 3 > u32::from(kit.hp_max);
        // Staying is dying: the sum's word, or a bred brain's.
        let run = match self.tactic(&t, &odds, kit, others, me, d, tick) {
            Some(c) => c == TACTIC_RUN,
            None => odds.losing,
        };
        if !stand && (self.temperament == Temperament::Passive || run || out_shot) {
            self.flee_from(&t, odds.ranged, me, tick);
            return self.back_away(core, body, tracks, route, tick, true);
        }
        self.engage(id, tick, false);
        self.fight(core, body, tracks, route, kit, tick)
    }

    /// The body that is attacking me now, if the eyes have it: one that
    /// swung at me from where the swing could land, or shot at me, just
    /// now; one standing where a blow came from; or a wolf coming in. A
    /// body that has stopped with a bow on me is one too to a body that
    /// starts fights, when it can answer it: with a shot of its own, or at
    /// a run from inside a charge. Otherwise it is sidestepped, and a bow
    /// merely carried my way is nothing until it shoots.
    fn attacker(&self, tracks: &Tracks, kit: &Kit, me: [f32; 3], tick: u32) -> Option<u32> {
        let book = kit.book;
        let recent = |at: Option<u32>| at.is_some_and(|a| tick.wrapping_sub(a) < ALARM_TICKS);
        let blow = self
            .hurt
            .filter(|(at, _)| tick.wrapping_sub(*at) < ALARM_TICKS);
        let fire = self
            .fire
            .filter(|f| f.taken && tick.wrapping_sub(f.at) < FLEE_TICKS);
        let mut best: Option<(f32, u32)> = None;
        let low = me_low(tracks);
        for t in tracks.seen() {
            if !t.visible || !t.active() {
                continue;
            }
            let d = flat(me, t.pos);
            let (reach, ranged) = their_reach(t, book);
            let swung = recent(t.last_swing) && swing_reaches(t, me, reach);
            let shot = recent(t.last_shot) && aimed(t, me, low, book);
            let struck = blow.is_some_and(|(_, toward)| {
                let bearing = yaw_toward(t.pos[0] - me[0], t.pos[2] - me[2]);
                let off = (bearing.wrapping_sub(toward) as i16).unsigned_abs();
                (d <= reach.max(1.0) + ATTACKER_SLACK_M || (ranged && aimed(t, me, low, book)))
                    && (off <= HURT_CONE || d < 1.0)
            });
            // Found where a shot at me came from, with something that
            // shoots it that far pointed my way: the shooter.
            let fired = fire.is_some_and(|f| {
                let bearing = yaw_toward(t.pos[0] - me[0], t.pos[2] - me[2]);
                ranged
                    && d <= reach
                    && off(bearing, f.toward) <= HURT_CONE
                    && aimed(t, me, low, book)
            });
            let charging = t.species == Species::Wolf && d < WOLF_ALARM_M && closing(t, me, d);
            // One I could not get at is let be: it is the base it is after.
            let raiding = raiding(t, kit, me, tick) && !self.shuns(t.id, tick);
            // Coming on at me for a moment, not a stride that happens to
            // point my way on its way past.
            let rushing = self
                .menace
                .is_some_and(|(id, since)| id == t.id && tick.wrapping_sub(since) >= MENACE_TICKS);
            let drawn = matches!(
                self.temperament,
                Temperament::Opportunist | Temperament::Kos
            ) && d <= EVADE_M
                && (kit.ranged.is_some() || (kit.melee.is_some() && d <= CHARGE_M))
                && self.drawer.is_some_and(|(id, since)| {
                    id == t.id && tick.wrapping_sub(since) >= DRAWN_TICKS
                });
            // In the safe zone (either of us) nothing is a fight: THE GATE's
            // sentries answer whoever attacked (`sim_core::sentry`), and a
            // blow back would make this body hostile under the same guns.
            let zone = t.species == Species::Player && (in_zone(kit, me) || in_zone(kit, t.pos));
            let provoked = rushing || drawn || raiding;
            if ((!zone && (swung || shot || struck || fired || provoked)) || charging)
                && best.is_none_or(|(b, _)| d < b)
            {
                best = Some((d, t.id));
            }
        }
        best.map(|(_, id)| id)
    }

    /// A fight worth starting, as the temperament sees it: the nearest
    /// player in sight this body has a clear edge on (an opportunist, with
    /// nobody else about and health to spare), or is not losing to (kill
    /// on sight).
    fn opening(&self, tracks: &Tracks, kit: &Kit, me: [f32; 3], tick: u32) -> Option<u32> {
        let picky = match self.temperament {
            Temperament::Passive | Temperament::Defensive => return None,
            Temperament::Opportunist => true,
            Temperament::Kos => false,
        };
        kit.melee?;
        if picky && u32::from(kit.hp) * 2 <= u32::from(kit.hp_max) {
            return None;
        }
        // No fight is started from the safe zone, nor on anyone in it.
        if in_zone(kit, me) {
            return None;
        }
        let mut best: Option<(f32, u32)> = None;
        for t in tracks.seen() {
            if !t.visible
                || !t.active()
                || t.species != Species::Player
                || self.shuns(t.id, tick)
                || in_zone(kit, t.pos)
            {
                continue;
            }
            let d = flat(me, t.pos);
            if d > START_M || best.is_some_and(|(b, _)| d >= b) {
                continue;
            }
            // Busy at the trees and the rocks beside me: someone sharing
            // the ground, not an opening, whatever it holds against mine.
            let working = t
                .last_swing
                .is_some_and(|at| tick.wrapping_sub(at) < GATHERING_TICKS);
            if picky && working && !t.wounded {
                continue;
            }
            let others = crowd(tracks, t.id, me, tick);
            let odds = odds(t, kit, others);
            let take = if picky {
                odds.edge && !odds.losing && others == 0 && !odds.ranged
            } else {
                !odds.losing
            };
            if take {
                best = Some((d, t.id));
            }
        }
        best.map(|(_, id)| id)
    }

    /// One frame of an engagement: judge it, then fight it.
    fn fight(
        &mut self,
        core: &mut ClientCore,
        body: &EntityState,
        tracks: &Tracks,
        route: &mut Route,
        kit: &Kit,
        tick: u32,
    ) -> Assess {
        let Some(mut foe) = self.foe else {
            return self.over(None, End::Parted);
        };
        let me = pos(body);
        // An animal's track outlives its body: remembered out of sight, it
        // would read as one that ran off, not one that fell.
        let tracked = tracks
            .get(foe.id)
            .copied()
            .filter(|_| Species::of(foe.id) == Species::Player || !tracks.gone(foe.id));
        let Some(mut t) = tracked else {
            // Gone from the snapshot the moment after my round landed, in
            // sight and near: it fell, and its carcass took it out of the
            // snapshot. The markers cannot say so when the hurt began in an
            // earlier fight it had no time to heal from (`Foe::dealt_from`).
            let fell = foe.seen.filter(|at| {
                Species::of(foe.id) != Species::Player
                    && flat(me, *at) <= VANISHED_KILL_M
                    && foe
                        .fired_at
                        .is_some_and(|f| tick.wrapping_sub(f) <= VANISHED_KILL_TICKS)
            });
            if let Some(at) = fell {
                return self.fell(core, body, tracks, route, kit, at, false, tick);
            }
            return self.over(None, End::Parted);
        };
        if t.visible {
            foe.seen = Some(t.pos);
        }
        if t.species != Species::Player {
            // An animal hurt in an earlier fight has healed since: only
            // my markers from this one say how much it has left.
            let from = *foe.dealt_from.get_or_insert(t.dealt);
            t.dealt = t.dealt.saturating_sub(from);
        }
        // A body lies down dead; an animal drops where my markers say it
        // had no more to give (its carcass leaves the snapshot).
        let beast_hp = match t.species {
            Species::Player => u32::MAX,
            Species::Pig => u32::from(kit.book.pig().hp),
            Species::Wolf => u32::from(kit.book.wolf().hp),
        };
        if t.dead || t.dealt >= beast_hp.max(1) {
            let pack = t.species == Species::Player;
            return self.fell(core, body, tracks, route, kit, t.pos, pack, tick);
        }
        if t.species != Species::Player {
            if t.dealt > foe.harmed.0 {
                foe.harmed = (t.dealt, tick);
            } else if tick.wrapping_sub(foe.harmed.1) >= NO_HARM_TICKS {
                // A standoff, not a fight: let it be and decide again.
                self.foe = Some(foe);
                self.shun(foe.id, tick);
                let away = yaw_toward(me[0] - t.pos[0], me[2] - t.pos[2]);
                return self.over(Some(away), End::Parted);
            }
        }
        let away = yaw_toward(me[0] - t.pos[0], me[2] - t.pos[2]);
        let lost = !t.visible && tick.wrapping_sub(t.last_seen) >= LOST_TICKS;
        if lost || t.sleeping {
            // Gone from sight: still being hit means it is still here.
            if self
                .hurt
                .is_some_and(|(at, _)| tick.wrapping_sub(at) < ALARM_TICKS)
            {
                self.run(Mode::Guard, tick, away);
                return self.back_away(core, body, tracks, route, tick, false);
            }
            return self.over(None, End::Parted);
        }
        let d = flat(me, t.pos);
        // Anything aimed at me from this body keeps the fight live.
        let recent = |at: Option<u32>| at.is_some_and(|a| tick.wrapping_sub(a) < ALARM_TICKS);
        let (reach, _) = their_reach(&t, kit.book);
        let struck = (recent(t.last_swing) && swing_reaches(&t, me, reach))
            || (recent(t.last_shot) && aimed(&t, me, me_low(tracks), kit.book))
            || self
                .hurt
                .is_some_and(|(at, _)| tick.wrapping_sub(at) < ALARM_TICKS && at > foe.threat);
        if struck {
            foe.struck = Some(tick);
        }
        if struck
            || self.drawer.is_some_and(|(id, _)| id == t.id)
            || self.menace.is_some_and(|(id, _)| id == t.id)
            || raiding(&t, kit, me, tick)
        {
            foe.threat = tick;
        }
        self.foe = Some(foe);
        // The safe zone: a player in it, or me in it, is never fought —
        // the town's sentries shoot the one who struck, and striking back
        // would put this body under them too. The fight is let go, never
        // carried or shot into the town.
        if t.species == Species::Player && (in_zone(kit, me) || in_zone(kit, t.pos)) {
            return self.over(Some(away), End::Parted);
        }
        let others = crowd(tracks, foe.id, me, tick);
        let odds = odds(&t, kit, others);
        let unarmed = kit.melee.is_none() && kit.ranged.is_none();
        let run = match self.tactic(&t, &odds, kit, others, me, d, tick) {
            Some(c) => c == TACTIC_RUN,
            None => odds.losing,
        };
        if unarmed || run || (odds.ranged && d > CHARGE_M && kit.ranged.is_none()) {
            self.flee_from(&t, odds.ranged, me, tick);
            return self.back_away(core, body, tracks, route, tick, true);
        }
        if !foe.commanded {
            let calm = tick.wrapping_sub(foe.threat) >= CALM_TICKS;
            if (calm && !odds.edge) || d > CHASE_M {
                return self.over(Some(away), End::Parted);
            }
        }
        // Where my body is by the time this frame's input runs: my blow
        // and my shot leave from there, not from my last snapshot.
        let here = self.strides.ahead_of(me);
        // An animal running from me is shot, never chased with the club: a
        // pig runs as fast as I do (and a wolf faster), so a chase on foot
        // never closes and ends as an unreachable foe.
        let running = t.species != Species::Player && opening(&t, me, d);
        let mut arm = self.arm_for(kit, d, running);
        if let Some(chosen) = self.chosen_arm(t.id, kit, d) {
            arm = chosen;
            self.arm = chosen;
        }
        let intent = match arm {
            Arm::Melee => {
                self.draw = None;
                self.melee(core, body, here, route, &t, &odds, kit, tick)
            }
            Arm::Ranged => self.shoot(core, body, here, route, &t, &odds, kit, tick),
        };
        if intent.buttons & BTN_PRIMARY != 0 {
            if let Some(f) = self.foe.as_mut() {
                f.fired_at = Some(tick);
            }
        }
        if self
            .stuck
            .is_some_and(|(since, _)| tick.wrapping_sub(since) >= UNREACHABLE_TICKS)
        {
            // No way to them: standing and staring is no fight. A goal
            // that asked for it hears the foe was not found.
            self.shun(foe.id, tick);
            return self.over(Some(away), End::Parted);
        }
        Assess::Fight(intent)
    }

    /// The foe fell at `at`: on to its bag (a player's pack when `pack`).
    #[allow(clippy::too_many_arguments)]
    fn fell(
        &mut self,
        core: &mut ClientCore,
        body: &EntityState,
        tracks: &Tracks,
        route: &mut Route,
        kit: &Kit,
        at: [f32; 3],
        pack: bool,
        tick: u32,
    ) -> Assess {
        self.spoils = Some(Spoils {
            at: [at[0], at[2]],
            since: tick,
            bag: true,
            sent: None,
            skip: None,
            picks: 0,
            pack,
        });
        self.mode = Mode::Loot;
        self.foe = None;
        self.stuck = None;
        self.draw = None;
        self.loot(core, body, tracks, route, kit, tick)
    }

    /// Trying to close on the foe from `d`: the clock on an unreachable foe
    /// runs until it gets nearer.
    fn closing_from(&mut self, d: f32, tick: u32) {
        self.stuck = match self.stuck {
            Some((since, best)) if d > best - CLOSE_M => Some((since, best)),
            _ => Some((tick, d)),
        };
    }

    /// The weapon for this range: the swung one close in, the shooting
    /// one further out, and between the two whichever is out already; a
    /// dry gun is swapped for the club rather than reloaded in someone's
    /// face.
    fn arm_for(&mut self, kit: &Kit, d: f32, running: bool) -> Arm {
        self.arm = match (kit.melee.is_some(), kit.ranged.is_some()) {
            (_, false) => Arm::Melee,
            (false, true) => Arm::Ranged,
            (true, true) if kit.loaded == Some(0) && d < KITE_M => Arm::Melee,
            (true, true) if running => Arm::Ranged,
            (true, true) if d <= MELEE_IN_M => Arm::Melee,
            (true, true) if d >= RANGED_OUT_M => Arm::Ranged,
            (true, true) => self.arm,
        };
        self.arm
    }

    /// A shot: bow, crossbow or gun. Get the body in sight and in range
    /// (a prey animal crouched, at a walk, to where it will not hear);
    /// then hold the draw, lead the body by where it will be when the
    /// round arrives (a bullet at a player goes where my screen shows
    /// it), and loose
    /// only once the hands have settled on it. Against a player keep
    /// stepping side to side, and away from a swung weapon closing in.
    #[allow(clippy::too_many_arguments)]
    fn shoot(
        &mut self,
        core: &mut ClientCore,
        body: &EntityState,
        me: [f32; 3],
        route: &mut Route,
        t: &Track,
        odds: &Odds,
        kit: &Kit,
        tick: u32,
    ) -> Intent {
        let Some((slot, item)) = kit.ranged else {
            return Intent::IDLE;
        };
        let r = kit.book.page(item).ranged;
        let (dx, dz) = (t.pos[0] - me[0], t.pos[2] - me[2]);
        let d = dx.hypot(dz);
        let toward = yaw_toward(dx, dz);
        let reach = if r.hitscan {
            r.range_mm as f32 / 1000.0 * HITSCAN_SHARE
        } else {
            ARROW_MAX_M
        };
        // Prey that has not been touched or turned on me hears a walking
        // body a long way off, and a crouched one in front of it hardly.
        let stalking = t.species != Species::Player
            && t.dealt == 0
            && !self
                .hurt
                .is_some_and(|(at, _)| tick.wrapping_sub(at) < ALARM_TICKS)
            && !closing(t, me, d);
        let at = [t.pos[0], t.pos[1] + aim_height(t, kit.book, d), t.pos[2]];
        let want = if stalking {
            HUNT_SHOT_M
        } else if t.species == Species::Player {
            (reach * 0.75).min(DUEL_M)
        } else {
            reach * 0.75
        };
        // The stalk is crouched (v83: a lower eye, as the sim fires from),
        // but not at the shot when the crouch is what hides the prey: a
        // crouched eye that cannot see the aim point stands up to loose,
        // and to find it again.
        let crouch = stalking
            && (d > want.min(reach) || {
                let low = [me[0], me[1] + eye_m(true), me[2]];
                let haven = *core.island().1.haven;
                clear_line(core, &haven, low, at, f32::INFINITY) == Sight::Clear
            });
        // Where the sim will fire this frame's shot from: the stance's eye,
        // which a crouch held off the ground does not lower.
        let eye = [me[0], me[1] + eye_m(crouch && body.grounded), me[2]];
        // A round that flies (`sim_core::bullet`) is led like an arrow, by
        // its flight; against a player only by its flight, because the shard
        // judges the whole flight where my screen shows them.
        let (look, solved) = if r.hitscan && r.speed_mmpt > 0 {
            let ahead_ticks = match t.species {
                Species::Player => 0.0,
                _ => kit.lead_ticks as f32,
            };
            match aim::lead(eye, at, t.vel, ahead_ticks, r.speed_mmpt, r.drop_mmpt2) {
                Some((p, _)) => (Look::Point(p), true),
                None => (Look::Point(at), false),
            }
        } else if r.hitscan {
            let look = match t.species {
                Species::Player => Look::Body {
                    id: t.id,
                    part: Part::Chest.bits(),
                },
                // The server rewinds players for a bullet, not animals: it
                // lands where the animal is when my press arrives.
                _ => Look::Point(ahead(t, at, kit.lead_ticks)),
            };
            (look, true)
        } else {
            match aim::lead(
                eye,
                at,
                t.vel,
                kit.lead_ticks as f32,
                r.speed_mmpt,
                r.drop_mmpt2,
            ) {
                Some((p, _)) => (Look::Point(p), true),
                None => (Look::Point(at), false),
            }
        };
        let base = Intent {
            look,
            sel: Some(slot),
            ..Intent::IDLE
        };
        if !t.visible || !solved || d > reach || (stalking && d > want) {
            // Into sight and range first, by a route; the draw waits.
            self.draw = None;
            let step = route.to(
                core,
                body,
                [t.pos[0], t.pos[2]],
                want * 0.8,
                !stalking,
                tick,
            );
            let (travel, jump) = match step {
                Step::Walk { yaw, jump, .. } => {
                    self.stuck = None;
                    (Some(yaw), jump)
                }
                Step::Wait => (Some(toward), false),
                // As near as the ground goes and still no shot (water or
                // a wall between, or too steep an arc): the same clock as
                // a foe out of a swing's reach.
                Step::Arrived | Step::Blocked => {
                    self.closing_from(d, tick);
                    (None, false)
                }
            };
            let mut buttons = match (stalking, crouch) {
                (false, _) => BTN_SPRINT,
                (true, true) => BTN_CROUCH,
                (true, false) => 0,
            };
            if jump {
                buttons |= BTN_JUMP;
            }
            return Intent {
                travel,
                buttons,
                ..base
            };
        }
        self.stuck = None;
        let mut buttons = 0;
        if crouch {
            buttons |= BTN_CROUCH;
        }
        if r.draw_ticks > 0 {
            buttons |= BTN_AIM;
            if self.draw.is_none_or(|(_, s)| s != slot) {
                self.draw = Some((tick, slot));
            }
        } else {
            self.draw = None;
        }
        let drawn = self.draw.map_or(r.draw_ticks == 0, |(since, _)| {
            tick.wrapping_sub(since) >= u32::from(r.draw_ticks) + DRAW_SLACK_TICKS
        });
        let ready = tick.wrapping_sub(self.shot_ready) < 1 << 31;
        let loaded = match kit.loaded {
            Some(n) => n > 0,
            None => kit.rounds > 0 || r.magazine > 0,
        };
        // A body stepping side to side is shot just after it turns back,
        // once the aim has caught up with it: it holds a stride for a
        // moment, and an arrow loosed late in one lands where it turned.
        // One that turns back faster than that never gives the moment: the
        // shot waits no longer than a moment's span, then goes at it anyway.
        let turned = self.weaving(t, toward, tick);
        let window =
            turned.is_none_or(|since| (WEAVE_SETTLE_TICKS..=WEAVE_FIRE_TICKS).contains(&since));
        let poised = drawn && ready && loaded && kit.settled && t.species == Species::Player;
        if !poised || window {
            self.held_shot = None;
        } else if self.held_shot.is_none() {
            self.held_shot = Some(tick);
        }
        let waited = self
            .held_shot
            .is_some_and(|since| tick.wrapping_sub(since) >= WEAVE_FIRE_TICKS);
        let steady = if r.hitscan {
            kit.on_target
        } else {
            kit.settled && (t.species != Species::Player || window || waited)
        };
        if drawn && ready && loaded && steady {
            buttons |= BTN_PRIMARY;
            self.fire_gap = u32::from(r.rate_ticks).max(1);
        }
        // Footwork. A swung weapon closing in is backed away from; a
        // player is never given a still body to shoot at; prey is shot
        // from where the stalk stopped.
        let club = t.species != Species::Pig && odds.armed && !odds.ranged && !t.wounded;
        let radial: f32 = if club && d < KITE_M {
            -1.0
        } else if d > want {
            1.0
        } else {
            0.0
        };
        let side = if t.species == Species::Player {
            self.rhythm(tick);
            f32::from(self.strafe.0)
        } else {
            0.0
        };
        let (ux, uz) = yaw_dir(toward);
        let (rx, rz) = (uz, -ux);
        let (vx, vz) = (radial * ux + side * rx, radial * uz + side * rz);
        let travel = (vx != 0.0 || vz != 0.0).then(|| yaw_toward(vx, vz));
        if radial < 0.0 && r.draw_ticks == 0 {
            buttons |= BTN_SPRINT;
        }
        // A gun is fired down its sights unless the legs are running (an
        // aim walks): the hip adds its whole cone to the aim's
        // (`combat::GunDef::cone`), the revolver's 2° on a 0.75°, and from
        // the hip half a cylinder missed a pig at twelve metres.
        if r.hitscan && buttons & BTN_SPRINT == 0 {
            buttons |= BTN_AIM;
        }
        Intent {
            travel,
            buttons,
            ..base
        }
    }

    /// After a win, while nobody else is near: walk to the bag where the
    /// foe fell and empty it, then pick up the arrows lying about. The
    /// orchestrator sends the verbs this asks for ([`Combat::take_verb`]).
    fn loot(
        &mut self,
        core: &mut ClientCore,
        body: &EntityState,
        tracks: &Tracks,
        route: &mut Route,
        kit: &Kit,
        tick: u32,
    ) -> Assess {
        let Some(mut s) = self.spoils else {
            return self.over(None, End::Won);
        };
        let me = pos(body);
        let here = [me[0], me[2]];
        if tick.wrapping_sub(s.since) >= LOOT_TICKS || !safe(tracks, me, tick) {
            self.spoils = None;
            return self.over(None, End::Won);
        }
        // What to take next: the bag, else the nearest arrow on the ground.
        let mut target: Option<(u32, [f32; 2], f32, Verb)> = None;
        if s.bag {
            let mut best: Option<(f32, u32, [f32; 2])> = None;
            for b in core.bags.entries() {
                if (b.kind == BAG_KIND_PACK) != s.pack {
                    continue;
                }
                let p = [b.qx as f32 * POS_XZ_Q, b.qz as f32 * POS_XZ_Q];
                let d = ground_dist(here, p);
                if ground_dist(s.at, p) <= BAG_MATCH_M && best.is_none_or(|(bd, ..)| d < bd) {
                    best = Some((d, b.id, p));
                }
            }
            match best {
                Some((_, id, p)) if !spent(s.sent, id, tick) => {
                    target = Some((id, p, LOOT_STAND_M, Verb::Loot));
                }
                None if tick.wrapping_sub(s.since) < BAG_WAIT_TICKS => {
                    // The bag's word comes on the event lane; look where
                    // the body fell while it does.
                    self.spoils = Some(s);
                    return Assess::Fight(Intent {
                        look: Look::Point([s.at[0], me[1], s.at[1]]),
                        ..Intent::IDLE
                    });
                }
                _ => {
                    s.bag = false;
                    s.sent = None;
                }
            }
        }
        if target.is_none() && s.picks < PICKUPS_MAX {
            let mut best: Option<(f32, u32, [f32; 2])> = None;
            for g in core.ground_items() {
                let p = [g.qx as f32 * POS_XZ_Q, g.qz as f32 * POS_XZ_Q];
                let d = ground_dist(here, p);
                if Some(g.id) == s.skip
                    || d > ARROW_SEEK_M
                    || ground_dist(s.at, p) > ARROW_SEEK_M
                    || !is_round(kit.book, g.item)
                    || best.is_some_and(|(bd, ..)| d >= bd)
                {
                    continue;
                }
                best = Some((d, g.id, p));
            }
            if let Some((_, id, p)) = best {
                if spent(s.sent, id, tick) {
                    // Pressed for and never taken: leave it.
                    s.skip = Some(id);
                    s.sent = None;
                } else {
                    target = Some((id, p, LOOT_STAND_M, Verb::Pickup));
                }
            }
        }
        let Some((id, p, stand, verb)) = target else {
            self.spoils = None;
            return self.over(None, End::Won);
        };
        let d = ground_dist(here, p);
        let look = Look::Point([p[0], me[1], p[1]]);
        if d > stand {
            let step = route.to(core, body, p, stand - 0.5, false, tick);
            self.spoils = Some(s);
            return Assess::Fight(match step {
                Step::Walk { yaw, jump, .. } => Intent {
                    look,
                    travel: Some(yaw),
                    buttons: if jump { BTN_JUMP } else { 0 },
                    ..Intent::IDLE
                },
                _ => Intent {
                    look,
                    travel: Some(yaw_toward(p[0] - here[0], p[1] - here[1])),
                    ..Intent::IDLE
                },
            });
        }
        let due = match s.sent {
            Some((at, was, _)) if was == id => tick.wrapping_sub(at) >= VERB_RETRY_TICKS,
            _ => true,
        };
        if due && kit.lane_free {
            let n = match s.sent {
                Some((_, was, n)) if was == id => n + 1,
                _ => 1,
            };
            s.sent = Some((tick, id, n));
            self.verb = Some(verb);
            match verb {
                Verb::Loot => self.stats.loots += 1,
                Verb::Pickup => {
                    self.stats.pickups += 1;
                    if n == 1 {
                        s.picks += 1;
                    }
                }
            }
        }
        self.spoils = Some(s);
        Assess::Fight(Intent {
            look,
            ..Intent::IDLE
        })
    }

    /// Melee footwork. Close the gap; where my reach is the longer, hold
    /// between the two reaches and let them walk onto it; where it is not,
    /// wait just outside theirs and step in right after they swing (a
    /// swing is a cadence long); always strafe, reversing no faster than a
    /// person's rhythm. Primary only in reach and on target.
    #[allow(clippy::too_many_arguments)]
    fn melee(
        &mut self,
        core: &mut ClientCore,
        body: &EntityState,
        me: [f32; 3],
        route: &mut Route,
        t: &Track,
        odds: &Odds,
        kit: &Kit,
        tick: u32,
    ) -> Intent {
        let (dx, dz) = (t.pos[0] - me[0], t.pos[2] - me[2]);
        let d = (dx * dx + dz * dz).sqrt();
        let toward = yaw_toward(dx, dz);
        let look = match t.species {
            Species::Player => Look::Body {
                id: t.id,
                part: if t.wounded {
                    Part::Chest.bits()
                } else {
                    Part::Head.bits()
                },
            },
            Species::Pig | Species::Wolf => {
                // Down at its back, on the near side: a swing from the eye
                // reaches an animal only through the top of it.
                let beast = beast(kit.book, t.species);
                let (r, h) = (
                    f32::from(beast.radius_cm) * 0.01,
                    f32::from(beast.height_cm) * 0.01,
                );
                // A swing at an animal is judged where it stands when the
                // press arrives, not where my screen shows it.
                let k = if d > 0.0 { r * 0.5 / d } else { 0.0 };
                let back = [t.pos[0] - dx * k, t.pos[1] + h * 0.9, t.pos[2] - dz * k];
                Look::Point(ahead(t, back, kit.lead_ticks))
            }
        };
        let base = Intent {
            look,
            sel: kit.melee.map(|(slot, _)| slot),
            ..Intent::IDLE
        };
        if !t.visible || d > odds.my_reach + ROUTE_M {
            // Out of reach by a distance: by a route, running.
            let step = route.to(core, body, [t.pos[0], t.pos[2]], odds.my_reach, true, tick);
            let (travel, jump) = match step {
                Step::Walk { yaw, jump, .. } => {
                    // The route is on its way; it says so when it is not.
                    self.stuck = None;
                    (Some(yaw), jump)
                }
                Step::Wait => (Some(toward), false),
                Step::Arrived | Step::Blocked => {
                    self.closing_from(d, tick);
                    (None, false)
                }
            };
            return Intent {
                travel,
                buttons: if jump {
                    BTN_SPRINT | BTN_JUMP
                } else {
                    BTN_SPRINT
                },
                ..base
            };
        }
        let safe = odds.their_reach + SAFE_M;
        // My clock and theirs are both in my snapshots' ticks; a press
        // made now lands a trip later, so mine counts as ready a trip early.
        let trip = self.strides.trip();
        let mine_in = if tick.wrapping_sub(self.ready_at) < 1 << 31 {
            0
        } else {
            self.ready_at.wrapping_sub(tick).saturating_sub(trip)
        };
        let ready = mine_in == 0;
        // Their next swing: a cadence on from the last one I saw, or felt.
        let felt = self
            .hurt
            .map(|(at, _)| at)
            .filter(|_| d <= safe + ATTACKER_SLACK_M);
        let last = match (t.last_swing, felt) {
            (Some(a), Some(b)) => Some(if b.wrapping_sub(a) < 1 << 31 { b } else { a }),
            (a, b) => a.or(b),
        };
        let theirs_in = last.map_or(0, |at| {
            let due = at.wrapping_add(odds.their_cadence.min(u32::from(u16::MAX)));
            if due.wrapping_sub(tick) < 1 << 31 {
                due.wrapping_sub(tick)
            } else {
                0
            }
        });
        let feared = odds.armed && !odds.ranged && !t.wounded;
        // Where they are by now, not where my screen shows them: a body
        // walking in is a playout and an input's trip nearer than that, and
        // their swing is judged on where I am now. Keeping out of reach
        // goes by this; my own swing by the screen.
        let lag = kit.lead_ticks as f32 / TICK_HZ as f32;
        let coming = if d > 0.0 {
            -(t.vel[0] * dx + t.vel[2] * dz) / d
        } else {
            0.0
        };
        let d_now = (d - coming.max(0.0) * lag).max(0.0);
        // And the gap their swing is judged on: where they are when it
        // lands against where their screen had me, as late as mine has
        // them. A body walking in has come that much nearer again.
        let d_theirs = (d_now - coming.max(0.0) * lag).max(0.0);
        // Distances here are to the pose on my screen, the one the server
        // judges my swing against. Whoever is moving in when two swings
        // meet has the better of it: the other's picture of them is a
        // playout behind, further off than they are. So a ready swing is
        // carried in at a run, straight (a sidestep there only spoils the
        // aim), and a spent one is taken back out of their reach at a run.
        // My own blow on its way in: hold the ground it was swung from,
        // from the frame that took it (the press with the swing ready) to
        // the last one that runs before it lands, echoed or not.
        let striking = tick.wrapping_sub(self.pressed_at) <= STRIKE_DELAY_TICKS as u32
            || self.strike_at.wrapping_sub(tick.wrapping_add(trip)) < 1 << 31;
        // Theirs on its way in: a swing is not spent when it is taken but
        // when it lands, `STRIKE_DELAY_TICKS` later, so that is the moment
        // to be out of its reach — not the moment to step in.
        let incoming = feared
            && t.last_swing
                .is_some_and(|at| tick.wrapping_sub(at) <= STRIKE_DELAY_TICKS as u32);
        // Theirs ready: bait it rather than trade into it, for as long as
        // they keep biting. Theirs spent far enough out: step in on it.
        if theirs_in > 0 || self.bait.is_some_and(|(id, ..)| id != t.id) {
            self.bait = None;
        }
        let bait_over = self
            .bait
            .is_some_and(|(_, since, _)| tick.wrapping_sub(since) >= BAIT_MAX_TICKS);
        let punish = theirs_in > mine_in + trip + STEP_IN_TICKS;
        let mut baiting = false;
        // Which gap a stand-off is kept on: mine to the screen, or theirs
        // to where their blow would be judged.
        let mut theirs_side = false;
        let (want, lunge) = if striking {
            (d.min(odds.my_reach - HOLD_M), false)
        } else if incoming {
            (odds.their_reach + WAIT_M, false)
        } else if !feared {
            // Nothing to fear up close: in, and swing.
            (odds.my_reach - HOLD_M, true)
        } else if (ready || mine_in <= LEAD_IN_TICKS) && (punish || bait_over) {
            // Mine is ready, or will be as I get there, and theirs is spent
            // (or will not be): in, and swing.
            (odds.my_reach - HOLD_M, d <= odds.my_reach + LUNGE_M)
        } else if theirs_in == 0 && !bait_over && t.species == Species::Player {
            let (id, since, out) = self.bait.unwrap_or((t.id, tick, false));
            let out = if out {
                d_theirs < odds.their_reach + BAIT_OUT_M
            } else {
                d_theirs <= odds.their_reach - BAIT_IN_M
            };
            self.bait = Some((id, since, out));
            baiting = true;
            if out {
                (odds.their_reach + BAIT_OUT_M, false)
            } else {
                (odds.their_reach - BAIT_IN_M, false)
            }
        } else if odds.my_reach - HOLD_M >= safe - REACH_EPS_M {
            // Out-ranged by me, a spear against a rock, and their swing
            // spent: hold between the two reaches and let them walk onto
            // the point.
            theirs_side = true;
            (odds.my_reach - HOLD_M, false)
        } else if theirs_in <= mine_in + STEP_OUT_TICKS {
            // Theirs comes back first, or near enough: out of their reach
            // until mine is ready.
            (odds.their_reach + WAIT_M, false)
        } else {
            // Mine comes back well before theirs: stay where the next
            // swing is short.
            (d.max(odds.my_reach), false)
        };
        // Coming in is judged on the screen; staying out on where they are.
        let judged = if baiting || theirs_side {
            d_theirs
        } else if want >= safe {
            d_now
        } else {
            d
        };
        let radial: f32 = if baiting {
            // The bait goes all the way in and all the way out.
            if want > odds.their_reach {
                -1.0
            } else {
                1.0
            }
        } else if judged > want + 0.25 {
            1.0
        } else if judged < want - 0.25 && !striking {
            // Never back out of my own blow on its way in.
            -1.0
        } else {
            0.0
        };
        if radial > 0.0 && d > odds.my_reach {
            self.closing_from(d, tick);
        } else {
            self.stuck = None;
        }
        self.rhythm(tick);
        // Less sideways the closer: circling a body at arm's length turns
        // the bearing to it faster than a person's eyes follow.
        let side = if lunge || striking || baiting || radial < 0.0 {
            0.0
        } else {
            f32::from(self.strafe.0) * STRAFE_SHARE * ((d - 0.5) / 2.5).clamp(0.2, 1.0)
        };
        let (ux, uz) = yaw_dir(toward);
        // World right of the bearing (`yaw_dir` of a quarter turn on).
        let (rx, rz) = (uz, -ux);
        let (vx, vz) = (radial * ux + side * rx, radial * uz + side * rz);
        let mut buttons = 0;
        // Run in to close, and run out: backing off at a walk, a body that
        // walks after me never leaves its reach.
        if radial < 0.0 || (radial > 0.0 && !baiting && (d > odds.my_reach || lunge)) {
            buttons |= BTN_SPRINT;
        }
        // The blow lands `STRIKE_DELAY_TICKS` after the press, where they
        // will be by then: swung at a body walking in, it meets them. A
        // player is judged as my screen shows them; an animal where it
        // truly is, a playout and a trip on from that.
        let ahead_ticks = if t.species == Species::Player {
            STRIKE_DELAY_TICKS as u32
        } else {
            STRIKE_DELAY_TICKS as u32 + kit.lead_ticks
        };
        let at_strike = (d - coming * ahead_ticks as f32 / TICK_HZ as f32).max(0.0);
        let in_reach = at_strike <= odds.my_reach - 0.05;
        // Not at a bait: a blow swung on the dip lands on the way back out.
        if in_reach
            && !baiting
            && mine_in <= PRESS_EARLY_TICKS
            && kit.on_target
            && kit.melee.is_some()
        {
            buttons |= BTN_PRIMARY;
            if ready && !striking {
                self.pressed_at = tick;
            }
        }
        Intent {
            travel: (vx != 0.0 || vz != 0.0).then(|| yaw_toward(vx, vz)),
            buttons,
            ..base
        }
    }

    /// Guard (walking, steady) or escape (running, weaving): face back
    /// toward the danger and back away from it along a route, renewed while
    /// a pursuer is in sight. A guard walks so the body it is looking for
    /// is still near when the eyes find it.
    fn back_away(
        &mut self,
        core: &mut ClientCore,
        body: &EntityState,
        tracks: &Tracks,
        route: &mut Route,
        tick: u32,
        weave: bool,
    ) -> Assess {
        let Some(mut r) = self.retreat else {
            return self.over(None, End::Parted);
        };
        if tracks.pursuer_in_sight(PURSUER_NEAR_M) {
            r.start = tick;
        }
        if tick.wrapping_sub(r.start) >= FLEE_TICKS {
            let end = if self.mode == Mode::Escape {
                End::Escaped
            } else {
                End::Parted
            };
            return self.over(Some(r.away), end);
        }
        let (x, z) = (body.qx as f32 * POS_XZ_Q, body.qz as f32 * POS_XZ_Q);
        // Under fire, a trunk or a rock between us beats open ground.
        if let Some((id, at, true)) = self.threat.filter(|_| weave) {
            let seen = tracks
                .get(id)
                .filter(|t| t.visible)
                .map(|t| [t.pos[0], t.pos[2]]);
            let at = seen.unwrap_or(at);
            self.threat = Some((id, at, true));
            // A scan that found nothing is not run again every frame.
            let rethink = self
                .cover_tried
                .is_none_or(|at| tick.wrapping_sub(at) >= COVER_RETHINK_TICKS);
            // Still in its sight where the cover was: somewhere else.
            if rethink && (self.cover.is_none() || seen.is_some()) {
                let had = self.cover;
                self.cover = cover::find(core, [x, z], at, COVER_M, had, None);
                self.cover_tried = Some(tick);
                if had.is_none() && self.cover.is_some() {
                    self.stats.covers += 1;
                }
            }
            if let Some(spot) = self.cover {
                self.retreat = Some(r);
                let face = Look::Heading(yaw_toward(at[0] - x, at[1] - z));
                return Assess::Fight(match route.to(core, body, spot, 0.6, true, tick) {
                    Step::Walk { yaw, jump, .. } => {
                        // Not in a straight line while they shoot.
                        self.rhythm(tick);
                        let weave = if self.strafe.0 > 0 {
                            yaw.wrapping_add(WEAVE)
                        } else {
                            yaw.wrapping_sub(WEAVE)
                        };
                        Intent {
                            look: face,
                            travel: Some(weave),
                            buttons: if jump {
                                BTN_SPRINT | BTN_JUMP
                            } else {
                                BTN_SPRINT
                            },
                            ..Intent::IDLE
                        }
                    }
                    Step::Wait => Intent {
                        look: face,
                        travel: Some(yaw_toward(spot[0] - x, spot[1] - z)),
                        buttons: BTN_SPRINT,
                        ..Intent::IDLE
                    },
                    // There, or as near as the ground allows: low, still,
                    // watching the way they come.
                    Step::Arrived | Step::Blocked => Intent {
                        look: face,
                        buttons: BTN_CROUCH,
                        ..Intent::IDLE
                    },
                });
            }
        }
        if into_deeper_water(core, body, r.away) {
            r.away = r.away.wrapping_add(1 << 14);
            r.to = None;
        }
        let ahead = |away: u16| {
            let (dx, dz) = yaw_dir(away);
            [x + dx * FLEE_M, z + dz * FLEE_M]
        };
        let to = *r.to.get_or_insert_with(|| ahead(r.away));
        let mut step = route.to(core, body, to, 2.0, true, tick);
        if step == Step::Arrived {
            // Still in danger and out of road: on along the same bearing.
            let on = ahead(r.away);
            r.to = Some(on);
            step = route.to(core, body, on, 2.0, true, tick);
        }
        self.retreat = Some(r);
        // Keep the danger in view while retreating: turn to face back and
        // back away along the route, whatever the view is doing.
        // A lip the route is fighting is jumped, as on any other walk.
        let (mut travel, jump) = match step {
            Step::Walk { yaw, jump, .. } => (yaw, jump),
            Step::Arrived | Step::Blocked | Step::Wait => (r.away, false),
        };
        if weave {
            if tick.wrapping_sub(self.strafe.1) < 1 << 31 {
                let span = self.rng.next_bounded(STRAFE_SPAN_TICKS + 1);
                self.strafe = (-self.strafe.0, tick + STRAFE_MIN_TICKS + span);
            }
            travel = if self.strafe.0 > 0 {
                travel.wrapping_add(WEAVE)
            } else {
                travel.wrapping_sub(WEAVE)
            };
        }
        let run = if weave { BTN_SPRINT } else { 0 };
        Assess::Fight(Intent {
            look: Look::Heading(r.away.wrapping_add(1 << 15)),
            travel: Some(travel),
            buttons: if jump { run | BTN_JUMP } else { run },
            ..Intent::IDLE
        })
    }
}

/// The report heard last before `h` (or with it), no longer than
/// [`PAIR_TICKS`] before, and within [`HURT_CONE`] of `side` if one is
/// given: the shot that landed it.
fn report(tracks: &Tracks, h: &Heard, side: Option<u16>) -> Option<Heard> {
    tracks
        .heard()
        .filter(|r| {
            matches!(r.sound, Sound::Shot | Sound::Whiz)
                && h.tick.wrapping_sub(r.tick) <= PAIR_TICKS
                && side.is_none_or(|s| off(r.bearing, s) <= HURT_CONE)
        })
        .last()
        .copied()
}

/// How far apart two wire yaws are, either way round.
fn off(a: u16, b: u16) -> u16 {
    (a.wrapping_sub(b) as i16).unsigned_abs()
}

/// Inside the town's safe zone (`town::safe`, a map fact).
fn in_zone(kit: &Kit, p: [f32; 3]) -> bool {
    sim_core::town::safe(&kit.town, p[0], p[2])
}

fn pos(body: &EntityState) -> [f32; 3] {
    [
        body.qx as f32 * POS_XZ_Q,
        body.qy as f32 * POS_Y_Q,
        body.qz as f32 * POS_XZ_Q,
    ]
}

fn beast(book: &Book, species: Species) -> &super::wiki::Beast {
    if species == Species::Wolf {
        book.wolf()
    } else {
        book.pig()
    }
}

/// How far from an animal's middle a swing of `reach` metres from the eye
/// lands on it: the ray goes in through its back, so the drop from my eye
/// to its top comes off the reach, and the near half of it is in range.
fn beast_reach(reach: f32, beast: &super::wiki::Beast) -> f32 {
    let (r, h) = (
        f32::from(beast.radius_cm) * 0.01,
        f32::from(beast.height_cm) * 0.01,
    );
    let drop = EYE_M - h;
    (reach * reach - drop * drop).max(0.0).sqrt() + r * 0.8
}

/// Where a point on an animal will be when my next press lands: the
/// server judges shots and swings at animals live, with no rewind. A
/// player's pose is the one on my screen, as the server rewinds to it.
fn ahead(t: &Track, p: [f32; 3], lead_ticks: u32) -> [f32; 3] {
    if t.species == Species::Player {
        return p;
    }
    let k = lead_ticks as f32 / TICK_HZ as f32;
    [p[0] + t.vel[0] * k, p[1], p[2] + t.vel[2] * k]
}

fn ground_dist(a: [f32; 2], b: [f32; 2]) -> f32 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

/// A stack pressed for its full count of tries, the last a while ago, and
/// still there: it is not coming.
fn spent(sent: Option<(u32, u32, u8)>, id: u32, tick: u32) -> bool {
    sent.is_some_and(|(at, was, n)| {
        was == id && n >= LOOT_TRIES && tick.wrapping_sub(at) >= VERB_RETRY_TICKS
    })
}

/// Something a weapon on the wiki shoots.
fn is_round(book: &Book, item: u16) -> bool {
    (0..MAX_ITEM_DEFS as u16).any(|w| book.page(w).ranged.round == item)
}

/// Where on a body a shot is aimed, above its feet: a player's head from
/// near enough that the hands hold it (two arrows kill where three to the
/// chest would), its chest further off; an animal's middle.
fn aim_height(t: &Track, book: &Book, d: f32) -> f32 {
    match t.species {
        Species::Player if d <= HEAD_SHOT_M => part_height(Part::Head.bits(), t.crouched),
        Species::Player => part_height(Part::Chest.bits(), t.crouched),
        Species::Pig => f32::from(book.pig().height_cm) * 0.005,
        Species::Wolf => f32::from(book.wolf().height_cm) * 0.005,
    }
}

/// A player swinging or shooting beside my base while it is being struck:
/// the raid is an attack on me, answered as one from within a chase of it.
/// One further off is too far to close on before the fight is let go; the
/// way home is the mind's.
fn raiding(t: &Track, kit: &Kit, me: [f32; 3], tick: u32) -> bool {
    let recent = |at: Option<u32>| at.is_some_and(|a| tick.wrapping_sub(a) < ALARM_TICKS);
    kit.raided.is_some_and(|[hx, hz]| {
        t.species == Species::Player
            && (recent(t.last_swing) || recent(t.last_shot))
            && (t.pos[0] - hx).hypot(t.pos[2] - hz) <= RAID_M
            && flat(me, t.pos) <= CHASE_M
    })
}

/// Nobody who could hurt me near enough and in sight to stop a bandage:
/// no player or wolf in sight within [`HEAL_SAFE_M`]. One further off, or
/// out of sight behind cover, gives the seconds a med takes.
pub fn safe(tracks: &Tracks, me: [f32; 3], tick: u32) -> bool {
    !tracks
        .recent(tick, TICK_HZ)
        .any(|t| t.visible && t.species != Species::Pig && flat(me, t.pos) <= HEAL_SAFE_M)
}

/// Am I crouched, as the server last applied it (my own snapshot, v83):
/// where a foe's look ray finds my head and chest.
fn me_low(tracks: &Tracks) -> bool {
    tracks.own().is_some_and(|o| o.crouched)
}

fn flat(a: [f32; 3], b: [f32; 3]) -> f32 {
    let (dx, dz) = (a[0] - b[0], a[2] - b[2]);
    (dx * dx + dz * dz).sqrt()
}

/// Could this player's last swing have landed on me: am I ahead of it,
/// within `reach` (centre to centre) and across its line by no more than a
/// body? The look ray alone says only where it faces; a body chopping a
/// tree faces the tree, and whoever stands a step off that line with it.
fn swing_reaches(t: &Track, me: [f32; 3], reach: f32) -> bool {
    if t.species != Species::Player || reach <= 0.0 {
        return false;
    }
    let (fx, fz) = yaw_dir(t.yaw);
    let (dx, dz) = (me[0] - t.pos[0], me[2] - t.pos[2]);
    let along = dx * fx + dz * fz;
    let across = (dx * fz - dz * fx).abs();
    along > 0.0 && along <= reach + SWING_SLACK_M && across <= CAPSULE_RADIUS_M + SWING_MISS_M
}

fn closing(t: &Track, me: [f32; 3], d: f32) -> bool {
    closing_at(t, me, d, CLOSING_MPS)
}

/// Looking me in the face, not past me at the tree behind: its look ray
/// passes within a body's width of my head.
fn eyes_on_me(t: &Track, me: [f32; 3], low: bool) -> bool {
    let (fx, fz) = yaw_dir(t.yaw);
    let (h, v) = pitch_dir(t.pitch);
    let dir = [fx * h, v, fz * h];
    let to = [
        me[0] - t.pos[0],
        me[1] + part_height(Part::Head.bits(), low) - (t.pos[1] + eye_m(t.crouched)),
        me[2] - t.pos[2],
    ];
    let along = to[0] * dir[0] + to[1] * dir[1] + to[2] * dir[2];
    let len2 = to[0] * to[0] + to[1] * to[1] + to[2] * to[2];
    along > 0.0 && len2 - along * along <= FACE_MISS_M * FACE_MISS_M
}

/// Pointed my way: the look ray passes near me ([`Track::aiming_at_me`]),
/// or, for something that shoots, near enough that a lead and an arc would
/// put a round on me: off its line by no more than a walker's lead at that
/// range, and over me by no more than an arrow's arc. A drawn bow is seen
/// pointing a body's way; the few degrees of its arc are not.
fn aimed(t: &Track, me: [f32; 3], low: bool, book: &Book) -> bool {
    if t.aiming_at_me {
        return true;
    }
    if t.species != Species::Player || !t.held.is_some_and(|h| book.page(h).fires()) {
        return false;
    }
    let (fx, fz) = yaw_dir(t.yaw);
    let (dx, dz) = (me[0] - t.pos[0], me[2] - t.pos[2]);
    let along = dx * fx + dz * fz;
    if along <= 0.0 {
        return false;
    }
    let across = (dx * fz - dz * fx).abs();
    if across > AIM_MISS_M.max(along * LEAD_SPREAD) {
        return false;
    }
    let (h, v) = pitch_dir(t.pitch);
    // How far over my chest the look ray passes, where it passes me.
    let rise = t.pos[1] + eye_m(t.crouched) + v / h.max(0.05) * along
        - (me[1] + part_height(Part::Chest.bits(), low));
    rise >= -AIM_MISS_M && rise <= AIM_MISS_M + along * ARC_RISE
}

/// A player standing still with something that shoots pointed my way
/// ([`aimed`]): stopped to draw on me, not walking past or toward me.
fn drawing(t: &Track, me: [f32; 3], low: bool, book: &Book) -> bool {
    t.species == Species::Player
        && t.active()
        && !t.wounded
        && t.held.is_some_and(|h| book.page(h).fires())
        && t.vel[0] * t.vel[0] + t.vel[2] * t.vel[2] <= STILL_MPS * STILL_MPS
        && aimed(t, me, low, book)
}

/// A player with a swung weapon out, facing me, stepping in on me from
/// just past its reach: a charge, answered before it lands.
/// It is coming at me, and the gap between us is closing: a body running
/// beside me toward the same tree is not coming at me however fast it
/// runs, and neither is one I am walking up to.
fn rushing(
    t: &Track,
    me: [f32; 3],
    low: bool,
    my_vel: [f32; 3],
    d: f32,
    reach: f32,
    ranged: bool,
) -> bool {
    let (dx, dz) = (me[0] - t.pos[0], me[2] - t.pos[2]);
    let gap = (t.vel[0] - my_vel[0]) * dx + (t.vel[2] - my_vel[2]) * dz;
    t.species == Species::Player
        && !ranged
        && reach > 0.0
        && !t.wounded
        && eyes_on_me(t, me, low)
        && d <= reach + MENACE_M
        && gap > 0.0
        && gap >= CLOSING_MPS * d
        && closing_at(t, me, d, CLOSING_MPS)
}

/// The gap between this body and me is shrinking by its own steps at a
/// walk's half or faster: it is coming my way, not keeping pace beside me.
fn nearing(t: &Track, me: [f32; 3], my_vel: [f32; 3], d: f32) -> bool {
    let (dx, dz) = (me[0] - t.pos[0], me[2] - t.pos[2]);
    let gap = (t.vel[0] - my_vel[0]) * dx + (t.vel[2] - my_vel[2]) * dz;
    gap > 0.0 && gap >= CLOSING_MPS * d && closing(t, me, d)
}

/// This body is coming at me at `mps` or faster.
/// Running from me: its speed away from me at least [`CLOSING_MPS`].
fn opening(t: &Track, me: [f32; 3], d: f32) -> bool {
    let (dx, dz) = (me[0] - t.pos[0], me[2] - t.pos[2]);
    let away = -(t.vel[0] * dx + t.vel[2] * dz);
    away > 0.0 && away >= CLOSING_MPS * d
}

fn closing_at(t: &Track, me: [f32; 3], d: f32, mps: f32) -> bool {
    let (dx, dz) = (me[0] - t.pos[0], me[2] - t.pos[2]);
    // Standing still on top of me is not coming at me.
    let toward = t.vel[0] * dx + t.vel[2] * dz;
    toward > 0.0 && toward >= mps * d
}

/// How far a body's blow reaches me, centre to centre, and whether it
/// shoots: its held weapon's page, or its species' bite.
fn their_reach(t: &Track, book: &Book) -> (f32, bool) {
    match t.species {
        Species::Player => {
            let page = t.held.map_or(&Page::EMPTY, |h| book.page(h));
            if page.fires() {
                let far = page.ranged.range_mm as f32 / 1000.0;
                (if far > 0.0 { far } else { FAR_M }, true)
            } else if page.swings() {
                (
                    f32::from(page.melee.reach_cm) * 0.01 + CAPSULE_RADIUS_M,
                    false,
                )
            } else {
                (0.0, false)
            }
        }
        Species::Wolf => (f32::from(book.wolf().reach_cm) * 0.01, false),
        Species::Pig => (f32::from(book.pig().reach_cm) * 0.01, false),
    }
}

/// Other bodies about that could join in: players and wolves up and seen
/// in the last two seconds, within [`CROWD_M`], besides `foe`.
fn crowd(tracks: &Tracks, foe: u32, me: [f32; 3], tick: u32) -> u32 {
    tracks
        .recent(tick, 2 * TICK_HZ)
        .filter(|t| t.id != foe && t.species != Species::Pig && flat(me, t.pos) <= CROWD_M)
        .count() as u32
}

/// What a bred tactics brain reads of a fight: [`TACTICS_IN`] numbers of
/// about unit size, all of them things the sum and the eyes already had.
///
/// | # | input |
/// |---|---|
/// | 0 | distance, tens of metres, at most 4 |
/// | 1, 2 | my hp share, theirs (from my hit markers; wounded is nearly none) |
/// | 3, 4 | blows each way to a kill, per 10, at most 2 |
/// | 5, 6 | reach each way, per 5 m, at most 4 |
/// | 7 | their cadence, per 2 s, at most 2 |
/// | 8, 9 | they are armed, they shoot |
/// | 10, 11 | the sum's verdicts: an edge, losing |
/// | 12, 13, 14 | a player, a wolf, a pig |
/// | 15 | others near, per 3 |
/// | 16, 17 | I can swing, I can shoot now (a bow with rounds, not dry) |
/// | 18 | on my home ground |
/// | 19 | I was struck just now |
/// | 20 | they are aiming at me |
/// | 21 | they swung just now |
/// | 22 | they are closing on me |
/// | 23 | they are down (wounded) |
#[allow(clippy::too_many_arguments)]
pub fn tactics_features(
    t: &Track,
    odds: &Odds,
    kit: &Kit,
    others: u32,
    me: [f32; 3],
    d: f32,
    struck: bool,
    tick: u32,
) -> [f32; TACTICS_IN] {
    let bit = |b: bool| if b { 1.0 } else { 0.0 };
    let hp0 = match t.species {
        Species::Player => u32::from(kit.hp_max.max(1)),
        Species::Wolf => u32::from(kit.book.wolf().hp.max(1)),
        Species::Pig => u32::from(kit.book.pig().hp.max(1)),
    };
    let blows = |n: u32| (n.min(20) as f32 / 10.0).min(2.0);
    let mut x = [0.0f32; TACTICS_IN];
    x[0] = (d / 10.0).min(4.0);
    x[1] = f32::from(kit.hp) / f32::from(kit.hp_max.max(1));
    x[2] = if t.wounded {
        0.05
    } else {
        1.0 - (t.dealt.min(hp0) as f32 / hp0 as f32)
    };
    x[3] = blows(odds.mine);
    x[4] = blows(odds.theirs);
    x[5] = (odds.my_reach / 5.0).min(4.0);
    x[6] = (odds.their_reach / 5.0).min(4.0);
    x[7] = (odds.their_cadence.min(2 * TICK_HZ) as f32 / TICK_HZ as f32).min(2.0);
    x[8] = bit(odds.armed);
    x[9] = bit(odds.ranged);
    x[10] = bit(odds.edge);
    x[11] = bit(odds.losing);
    x[12] = bit(t.species == Species::Player);
    x[13] = bit(t.species == Species::Wolf);
    x[14] = bit(t.species == Species::Pig);
    x[15] = others.min(6) as f32 / 3.0;
    x[16] = bit(kit.melee.is_some());
    x[17] = bit(kit.ranged.is_some() && kit.rounds > 0 && kit.loaded != Some(0));
    x[18] = bit(kit.home_ground);
    x[19] = bit(struck);
    x[20] = bit(t.aiming_at_me);
    x[21] = bit(t
        .last_swing
        .is_some_and(|at| tick.wrapping_sub(at) < ALARM_TICKS));
    x[22] = bit(closing(t, me, d));
    x[23] = bit(t.wounded);
    x
}

/// The sum, on the wiki's numbers: blows to kill each way, reaches, and
/// what makes the fight clearly mine or clearly lost.
pub fn odds(t: &Track, kit: &Kit, others: u32) -> Odds {
    let book = kit.book;
    let hp0 = match t.species {
        Species::Player if kit.hp_max > 0 => u32::from(kit.hp_max),
        Species::Player => 100,
        Species::Wolf => u32::from(book.wolf().hp),
        Species::Pig => u32::from(book.pig().hp),
    };
    let left = if t.wounded {
        u32::from(WOUNDED_HP)
    } else {
        hp0.saturating_sub(t.dealt).max(1)
    };
    let mine_page = kit.melee.map_or(&Page::EMPTY, |(_, item)| book.page(item));
    // A level swing from the eye crosses a standing body's head band: at
    // melee range between two people, a blow is a head blow.
    let head = |page: &Page| u32::from(page.melee.head_pct.max(100));
    let (my_damage, my_reach, my_cadence) = match (kit.melee, kit.ranged) {
        // Nothing to swing: the sum is on what I shoot, aimed at the
        // chest, a shot a draw and a cadence apart.
        (None, Some((_, item))) => {
            let r = book.page(item).ranged;
            let far = if r.hitscan {
                r.range_mm as f32 / 1000.0
            } else {
                ARROW_MAX_M
            };
            (
                u32::from(r.damage),
                far,
                (u32::from(r.rate_ticks) + u32::from(r.draw_ticks)).max(1),
            )
        }
        _ => (
            u32::from(mine_page.melee.damage)
                * if t.species == Species::Player && !t.wounded {
                    head(mine_page)
                } else {
                    100
                }
                / 100,
            {
                let reach = f32::from(mine_page.melee.reach_cm) * 0.01;
                match t.species {
                    Species::Player => reach + CAPSULE_RADIUS_M,
                    _ => beast_reach(reach, beast(book, t.species)),
                }
            },
            u32::from(mine_page.melee.cadence_ticks).max(1),
        ),
    };
    let (their_damage, their_cadence) = if t.wounded {
        (0, u32::MAX)
    } else {
        match t.species {
            Species::Player => {
                let page = t.held.map_or(&Page::EMPTY, |h| book.page(h));
                if page.fires() {
                    let gap = u32::from(page.ranged.rate_ticks) + u32::from(page.ranged.draw_ticks);
                    (u32::from(page.ranged.damage), gap.max(1))
                } else if page.swings() {
                    (
                        u32::from(page.melee.damage) * head(page) / 100,
                        u32::from(page.melee.cadence_ticks).max(1),
                    )
                } else {
                    (0, u32::MAX)
                }
            }
            Species::Wolf => (
                u32::from(book.wolf().bite),
                u32::from(book.wolf().bite_ticks).max(1),
            ),
            Species::Pig => (
                u32::from(book.pig().bite),
                u32::from(book.pig().bite_ticks).max(1),
            ),
        }
    };
    let (their_reach, ranged) = if t.wounded {
        (0.0, false)
    } else {
        their_reach(t, book)
    };
    let hits = |hp: u32, dmg: u32| if dmg == 0 { u32::MAX } else { hp.div_ceil(dmg) };
    let mine = hits(left, my_damage);
    let my_hp = u32::from(kit.hp).max(1);
    let theirs = hits(my_hp, their_damage);
    let armed = their_damage > 0;
    // Ticks each side needs, the first blow free.
    let my_time = mine.saturating_sub(1).saturating_mul(my_cadence);
    let their_time = theirs.saturating_sub(1).saturating_mul(their_cadence);
    let out_reach = my_reach > their_reach + SAFE_M && !ranged;
    let hurt_bad = u32::from(kit.hp) * 2 < u32::from(kit.hp_max.max(1));
    let edge = my_damage > 0
        && (!armed
            || t.wounded
            || left * 2 <= hp0
            || out_reach
            || my_time.saturating_mul(2) < their_time);
    let losing = my_damage == 0
        || (armed
            && ((theirs <= 1 && mine > 1)
                || (hurt_bad && their_time.saturating_add(my_cadence) < my_time)
                || (others > 0 && u32::from(kit.hp) * 10 < u32::from(kit.hp_max.max(1)) * 7)));
    Odds {
        mine,
        theirs,
        my_reach,
        their_reach,
        their_cadence,
        armed,
        ranged,
        edge,
        losing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::wiki::Rules;

    fn book() -> Book {
        let content = content::Content::from_sources(&crate::agent::wiki::SOURCES).unwrap();
        let catalog = crate::net::bake_all(&content).unwrap().catalog;
        let mut book = Book::EMPTY;
        book.learn(&Rules::from_content(&content).unwrap(), &catalog);
        book
    }

    fn wire(name: &str) -> u16 {
        let content = content::Content::from_sources(&crate::agent::wiki::SOURCES).unwrap();
        let item = content.items.iter().find(|i| i.name == name).unwrap();
        content.item_index(&item.id).unwrap()
    }

    fn player(held: Option<u16>) -> Track {
        Track {
            id: 7,
            species: Species::Player,
            first_seen: 0,
            last_seen: 0,
            pos: [0.0; 3],
            vel: [0.0; 3],
            yaw: 0,
            pitch: 128,
            held,
            lit: false,
            wounded: false,
            dead: false,
            sleeping: false,
            crouched: false,
            dealt: 0,
            last_swing: None,
            last_shot: None,
            aiming_at_me: false,
            visible: true,
        }
    }

    /// The sums a player does: a spear out-ranges a rock, an unarmed or
    /// badly hurt body is an edge, the next blow downing me is a lost
    /// fight, and a second body about is one to leave when hurt.
    #[test]
    fn the_odds_read_the_wiki() {
        let book = book();
        let spear = wire("Wooden Spear");
        // The short tool: the bat reaches 1.35 m since reach follows the
        // drawn weapon, too near a spear's 1.75 for an edge on reach alone.
        let rock = wire("Stone Hatchet");
        let kit = |item: u16, hp: u16| Kit {
            book: &book,
            melee: Some((0, item)),
            ranged: None,
            rounds: 0,
            loaded: None,
            hp,
            hp_max: 100,
            on_target: true,
            settled: true,
            lead_ticks: 4,
            lane_free: true,
            raided: None,
            home_ground: false,
            town: Town::NONE,
        };
        let with_spear = kit(spear, 100);
        let o = odds(&player(Some(rock)), &with_spear, 0);
        assert!(o.my_reach > o.their_reach + SAFE_M, "{o:?}");
        assert!(o.edge && o.armed && !o.losing && !o.ranged, "{o:?}");
        let even = odds(&player(Some(rock)), &kit(rock, 100), 0);
        assert!(!even.edge && !even.losing, "{even:?}");
        assert!(odds(&player(None), &kit(rock, 100), 0).edge, "unarmed");
        let mut hurt = player(Some(rock));
        hurt.dealt = 60;
        assert!(odds(&hurt, &kit(rock, 100), 0).edge, "wounded");
        assert!(
            odds(&player(Some(rock)), &kit(rock, 15), 0).losing,
            "one blow from down"
        );
        assert!(
            odds(&player(Some(rock)), &kit(rock, 60), 1).losing,
            "outnumbered, hurt"
        );
        assert!(!odds(&player(Some(rock)), &kit(rock, 100), 1).losing);
        let bow = odds(&player(Some(wire("Hunting Bow"))), &kit(rock, 100), 0);
        assert!(bow.ranged && bow.armed, "{bow:?}");
        let none = Kit {
            melee: None,
            ..kit(rock, 100)
        };
        assert!(
            odds(&player(Some(rock)), &none, 0).losing,
            "nothing to swing"
        );
    }

    /// A player swinging beside my base while it is being struck is an
    /// attack on me, from wherever I see it; not before the base is struck,
    /// not one further off it, and not one I already found I cannot reach.
    #[test]
    fn a_raid_on_my_base_is_an_attack_on_me() {
        let book = book();
        let raider = 7;
        let mut tracks = Tracks::new();
        tracks.stand(raider, [0.0, 0.0, 8.0], true);
        tracks.on_swing(raider, 5);
        let kit = |raided: Option<[f32; 2]>| Kit {
            book: &book,
            melee: Some((0, wire("Wooden Spear"))),
            ranged: None,
            rounds: 0,
            loaded: None,
            hp: 100,
            hp_max: 100,
            on_target: false,
            settled: false,
            lead_ticks: 4,
            lane_free: true,
            raided,
            home_ground: false,
            town: Town::NONE,
        };
        let me = [0.0, 0.0, 30.0];
        let mut combat = Combat::new(Temperament::Opportunist);
        assert_eq!(combat.attacker(&tracks, &kit(None), me, 10), None);
        let home = Some([0.0, 0.0]);
        assert_eq!(combat.attacker(&tracks, &kit(home), me, 10), Some(raider));
        assert_eq!(
            combat.attacker(&tracks, &kit(home), me, 5 + ALARM_TICKS),
            None,
            "a swing long ago"
        );
        let far = Some([0.0, -RAID_M]);
        assert_eq!(
            combat.attacker(&tracks, &kit(far), me, 10),
            None,
            "not at my base"
        );
        combat.shun(raider, 10);
        assert_eq!(
            combat.attacker(&tracks, &kit(home), me, 10),
            None,
            "out of reach"
        );
        // A second raider out of reach does not free the first to be taken
        // up again; past the list's room the soonest to lapse goes.
        combat.shun(8, 11);
        assert!(combat.shuns(raider, 12) && combat.shuns(8, 12));
        for id in 20..20 + SHUN_MAX as u32 - 1 {
            combat.shun(id, 12);
        }
        assert!(!combat.shuns(raider, 13) && combat.shuns(8, 13));
        assert!(!combat.shuns(8, 11 + SHUN_TICKS), "lapsed");
    }

    /// A raid seen from past a chase is not taken up only to be let go
    /// the same frame, over and over; from within one it is a fight, taken
    /// up once.
    #[test]
    fn a_raid_past_a_chase_is_not_taken_up_frame_after_frame() {
        let book = book();
        let raider = 7;
        let mut tracks = Tracks::new();
        tracks.stand(raider, [0.0, 0.0, 8.0], true);
        tracks.on_swing(raider, 5);
        let kit = Kit {
            book: &book,
            melee: Some((0, wire("Wooden Spear"))),
            ranged: None,
            rounds: 0,
            loaded: None,
            hp: 100,
            hp_max: 100,
            on_target: false,
            settled: false,
            lead_ticks: 4,
            lane_free: true,
            raided: Some([0.0, 0.0]),
            home_ground: false,
            town: Town::NONE,
        };
        let mut core = Box::new(ClientCore::new(1, 1, 0));
        let mut route = Route::new();
        let at = |z: f32| EntityState {
            qz: (z / POS_XZ_Q) as i32,
            ..EntityState::default()
        };
        let mut combat = Combat::new(Temperament::Opportunist);
        let far = at(8.0 + CHASE_M + 5.0);
        for tick in 6..6 + ALARM_TICKS / 2 {
            combat.assess(&mut core, &far, &tracks, &mut route, &kit, tick);
        }
        assert_eq!((combat.stats.engages, combat.stats.parted), (0, 0));
        let near = at(30.0);
        for tick in 6..6 + ALARM_TICKS / 2 {
            combat.assess(&mut core, &near, &tracks, &mut route, &kit, tick);
        }
        assert_eq!((combat.stats.engages, combat.stats.parted), (1, 0));
    }

    /// Inside its own base even a body that runs from every fight stands
    /// and meets the raider: its walls are the cover, and out of them is
    /// no escape. Outside, it runs.
    #[test]
    fn a_raid_met_on_home_ground_is_stood() {
        let book = book();
        let raider = 7;
        let mut tracks = Tracks::new();
        tracks.stand(raider, [0.0, 0.0, 8.0], true);
        tracks.on_swing(raider, 5);
        let kit = |home_ground: bool| Kit {
            book: &book,
            melee: Some((0, wire("Wooden Spear"))),
            ranged: None,
            rounds: 0,
            loaded: None,
            hp: 100,
            hp_max: 100,
            on_target: false,
            settled: false,
            lead_ticks: 4,
            lane_free: true,
            raided: Some([0.0, 0.0]),
            home_ground,
            town: Town::NONE,
        };
        let mut core = Box::new(ClientCore::new(1, 1, 0));
        let mut route = Route::new();
        let me = EntityState {
            qz: (3.0 / POS_XZ_Q) as i32,
            ..EntityState::default()
        };
        let mut combat = Combat::new(Temperament::Passive);
        combat.assess(&mut core, &me, &tracks, &mut route, &kit(false), 6);
        assert_eq!((combat.stats.escapes, combat.stats.engages), (1, 0));
        let mut combat = Combat::new(Temperament::Passive);
        combat.assess(&mut core, &me, &tracks, &mut route, &kit(true), 6);
        assert_eq!((combat.stats.escapes, combat.stats.engages), (0, 1));
    }

    /// No fight is started in the town's safe zone: not on a player
    /// standing in it, not from inside it. A swing at me there is still
    /// answered.
    #[test]
    fn no_fight_is_started_in_the_safe_zone() {
        let book = book();
        let town = Town {
            live: true,
            ..Town::NONE
        };
        let kit = Kit {
            book: &book,
            melee: Some((0, wire("Wooden Spear"))),
            ranged: None,
            rounds: 0,
            loaded: None,
            hp: 100,
            hp_max: 100,
            on_target: false,
            settled: false,
            lead_ticks: 4,
            lane_free: true,
            raided: None,
            home_ground: false,
            town,
        };
        let edge = sim_core::town::SAFE_HALF_M;
        let combat = Combat::new(Temperament::Kos);
        // Outside, an unarmed player 4 m off is an opening.
        let mut tracks = Tracks::new();
        tracks.stand(7, [0.0, 0.0, edge + 7.0], true);
        let me = [0.0, 0.0, edge + 3.0];
        assert_eq!(combat.opening(&tracks, &kit, me, 10), Some(7));
        // The same player just inside the zone is not.
        let mut tracks = Tracks::new();
        tracks.stand(7, [0.0, 0.0, edge - 1.0], true);
        assert_eq!(combat.opening(&tracks, &kit, me, 10), None);
        // Nor is anyone, seen from inside it.
        let mut tracks = Tracks::new();
        tracks.stand(7, [0.0, 0.0, edge + 3.0], true);
        let inside = [0.0, 0.0, edge - 1.0];
        assert_eq!(combat.opening(&tracks, &kit, inside, 10), None);
        assert_eq!(combat.attacker(&tracks, &kit, inside, 10), None);
        // Not even a blow from beside me there: THE GATE's sentries answer
        // the one who struck, and striking back would put me under them.
        let mut combat = combat;
        let mut tracks = Tracks::new();
        tracks.stand(7, [0.0, 0.0, edge], true);
        combat.on_hurt(9, yaw_toward(0.0, -1.0));
        let struck = [0.0, 0.0, edge - 1.0];
        assert_eq!(combat.attacker(&tracks, &kit, struck, 10), None);
        // The same blow out past the zone is an attack.
        let mut tracks = Tracks::new();
        tracks.stand(7, [0.0, 0.0, edge + 8.0], true);
        let out = [0.0, 0.0, edge + 7.0];
        assert_eq!(combat.attacker(&tracks, &kit, out, 10), Some(7));
    }

    fn spear_kit(book: &Book, town: Town) -> Kit<'_> {
        Kit {
            book,
            melee: Some((0, wire("Wooden Spear"))),
            ranged: None,
            rounds: 0,
            loaded: None,
            hp: 100,
            hp_max: 100,
            on_target: false,
            settled: false,
            lead_ticks: 4,
            lane_free: true,
            raided: None,
            home_ground: false,
            town,
        }
    }

    /// Shot at by a body it has not seen, it does what a person does:
    /// startled a moment, then off for cover from where the report came,
    /// at a run, eyes that way. Whoever the eyes then find there with a gun
    /// pointed at it is the one it answers; a body there pointing it
    /// somewhere else is not. A shot that cannot hurt it (in the safe zone)
    /// is no alarm.
    #[test]
    fn a_round_cracking_past_sends_it_to_cover_and_finds_the_shooter() {
        let book = book();
        let kit = spear_kit(&book, Town::NONE);
        let (west, east, north) = (
            yaw_toward(-1.0, 0.0),
            yaw_toward(1.0, 0.0),
            yaw_toward(0.0, 1.0),
        );
        let me = [0.0; 3];
        let shooter = 7;
        let mut tracks = Tracks::new();
        tracks.reset(1, 1);
        tracks.place(me);
        // Out of sight 30 m east, a revolver fired along a line by my chest.
        tracks.stand(shooter, [30.0, 0.0, 0.0], false);
        tracks.on_shot(shooter, (west, 128), 0, 500, 10);
        let mut core = Box::new(ClientCore::new(1, 1, 0));
        let mut route = Route::new();
        let body = EntityState::default();
        let mut combat = Combat::new(Temperament::Defensive);
        for tick in 10..10 + STARTLE_TICKS {
            let a = combat.assess(&mut core, &body, &tracks, &mut route, &kit, tick);
            assert_eq!(a, Assess::Calm, "startled, at {tick}");
        }
        let tick = 10 + STARTLE_TICKS;
        let a = combat.assess(&mut core, &body, &tracks, &mut route, &kit, tick);
        let Assess::Fight(intent) = a else {
            panic!("no answer to the shot: {a:?}");
        };
        assert!(combat.under_fire(), "{:?}", combat.mode());
        assert_eq!(combat.stats.shot_at, 1);
        let Look::Heading(eyes) = intent.look else {
            panic!("eyes {:?}", intent.look);
        };
        assert!(off(eyes, east) <= GLANCE_CONE, "looking {eyes}");
        let (_, away) = combat.retreat().unwrap();
        assert!(off(away, west) <= GLANCE_CONE, "backing off along {away}");
        // A second round on the way does not count a second alarm.
        tracks.on_shot(shooter, (west, 128), 0, 500, tick + 5);
        combat.assess(&mut core, &body, &tracks, &mut route, &kit, tick + 5);
        assert_eq!(combat.stats.shot_at, 1);
        // In sight now, revolver up and on me: the one shooting.
        tracks.stand(shooter, [30.0, 0.0, 0.0], true);
        let t = tracks.track_mut(shooter).unwrap();
        t.held = Some(wire("Revolver"));
        t.yaw = west;
        assert_eq!(combat.attacker(&tracks, &kit, me, tick + 6), Some(shooter));
        // Pointed up the beach instead: someone else's fight.
        tracks.track_mut(shooter).unwrap().yaw = north;
        assert_eq!(combat.attacker(&tracks, &kit, me, tick + 6), None);

        // The same round in the safe zone is no alarm.
        let town = Town {
            live: true,
            ..Town::NONE
        };
        let kit = spear_kit(&book, town);
        let mut combat = Combat::new(Temperament::Defensive);
        for tick in 20..20 + 2 * STARTLE_TICKS {
            combat.assess(&mut core, &body, &tracks, &mut route, &kit, tick);
        }
        assert_eq!((combat.mode(), combat.stats.shot_at), (Mode::Idle, 0));
    }

    /// An arrow landing beside me just after a twang is a shot at me from
    /// the twang's side; with no report it says nothing of where. A blow
    /// with a report from its side is a shot, taken up at once; a blow from
    /// a side nothing was heard from is a swing turned toward first, and
    /// run from as a shot only once the eyes have looked and found nobody.
    #[test]
    fn an_arrow_beside_me_or_a_blow_from_nobody_is_run_from_as_a_shot() {
        let book = book();
        let kit = spear_kit(&book, Town::NONE);
        let (north, south) = (yaw_toward(0.0, 1.0), yaw_toward(0.0, -1.0));
        let me = [0.0; 3];
        let mut core = Box::new(ClientCore::new(1, 1, 0));
        let mut route = Route::new();
        let body = EntityState::default();
        let fresh = || {
            let mut tracks = Tracks::new();
            tracks.reset(1, 1);
            tracks.place(me);
            tracks.stand(7, [0.0, 0.0, 25.0], false);
            tracks
        };
        // A twang from the north, then the arrow in the dirt beside me.
        let mut tracks = fresh();
        tracks.on_shot(7, (south, 140), 900, 10, 10);
        tracks.on_impact([1.0, 0.0, 1.0], 25);
        let mut combat = Combat::new(Temperament::Opportunist);
        for tick in 25..25 + STARTLE_TICKS + 1 {
            combat.assess(&mut core, &body, &tracks, &mut route, &kit, tick);
        }
        assert!(combat.under_fire());
        let (_, away) = combat.retreat().unwrap();
        assert!(off(away, south) <= GLANCE_CONE);
        // The arrow alone: nothing to say from where.
        let mut tracks = fresh();
        tracks.on_impact([1.0, 0.0, 1.0], 25);
        let mut combat = Combat::new(Temperament::Opportunist);
        for tick in 25..25 + 2 * STARTLE_TICKS {
            combat.assess(&mut core, &body, &tracks, &mut route, &kit, tick);
        }
        assert_eq!(combat.mode(), Mode::Idle);

        // The twang, then the arrow in me: at once, from the twang's side.
        let mut tracks = fresh();
        tracks.on_shot(7, (south, 140), 900, 10, 10);
        tracks.on_hurt(north, 25);
        let mut combat = Combat::new(Temperament::Opportunist);
        combat.on_hurt(25, south);
        combat.assess(&mut core, &body, &tracks, &mut route, &kit, 25);
        assert!(combat.under_fire());
        assert_eq!(combat.stats.shot_at, 1);
        // A blow from the south with the twang in the north: a swing at my
        // back, turned toward and walked back from...
        let mut tracks = fresh();
        tracks.on_shot(7, (south, 140), 900, 10, 10);
        tracks.on_hurt(south, 25);
        let mut combat = Combat::new(Temperament::Opportunist);
        combat.on_hurt(25, north);
        let a = combat.assess(&mut core, &body, &tracks, &mut route, &kit, 25);
        assert_eq!(combat.mode(), Mode::Guard);
        assert!(!combat.under_fire());
        let Assess::Fight(walk) = a else {
            panic!("{a:?}");
        };
        assert_eq!(walk.buttons & BTN_SPRINT, 0, "walked back, eyes round");
        // ...until the eyes have looked and found nobody there: then it
        // came from further off, and is run from to cover.
        for tick in 26..25 + LOOKED_TICKS {
            combat.assess(&mut core, &body, &tracks, &mut route, &kit, tick);
            assert!(!combat.under_fire(), "at {tick}");
        }
        combat.assess(
            &mut core,
            &body,
            &tracks,
            &mut route,
            &kit,
            25 + LOOKED_TICKS,
        );
        assert!(combat.under_fire());
        let (_, away) = combat.retreat().unwrap();
        assert!(off(away, north) <= GLANCE_CONE);
    }

    /// A shot heard from where nobody was seen turns the eyes there while
    /// the goal runs, once, as footsteps do; it is not an alarm.
    #[test]
    fn a_shot_heard_elsewhere_is_looked_at_not_run_from() {
        let book = book();
        let (west, north) = (yaw_toward(-1.0, 0.0), yaw_toward(0.0, 1.0));
        let mut tracks = Tracks::new();
        tracks.reset(1, 1);
        tracks.place([0.0; 3]);
        // Seen at the start, and not since: nobody known about there.
        tracks.stand(7, [50.0, 0.0, 0.0], false);
        let at = 1 + GLANCE_KNOWN_TICKS;
        tracks.on_shot(7, (north, 128), 0, 500, at);
        let mut combat = Combat::new(Temperament::Defensive);
        let look = combat.watch(&tracks, &book, [0.0; 3], at + 1);
        let Some(Look::Heading(y)) = look else {
            panic!("{look:?}");
        };
        assert!(off(y, west.wrapping_add(1 << 15)) <= GLANCE_CONE, "{y}");
        assert_eq!(
            combat.watch(&tracks, &book, [0.0; 3], at + GLANCE_TICKS),
            None
        );
        let kit = spear_kit(&book, Town::NONE);
        let mut core = Box::new(ClientCore::new(1, 1, 0));
        let mut route = Route::new();
        for tick in at..at + 2 * STARTLE_TICKS {
            let a = combat.assess(
                &mut core,
                &EntityState::default(),
                &tracks,
                &mut route,
                &kit,
                tick,
            );
            assert_eq!(a, Assess::Calm);
        }
    }

    /// A swing seen is at me only where it could land: ahead of it, in
    /// its reach, and on its line, not a step off it or a tree away.
    #[test]
    fn a_swing_is_at_me_only_where_it_could_land() {
        let mut t = player(None);
        t.yaw = yaw_toward(0.0, 1.0);
        let reach = 1.4;
        assert!(swing_reaches(&t, [0.0, 0.0, 1.2], reach));
        assert!(swing_reaches(&t, [0.4, 0.0, 1.2], reach), "the arm");
        assert!(!swing_reaches(&t, [0.8, 0.0, 1.2], reach), "a step aside");
        assert!(!swing_reaches(&t, [0.0, 0.0, 3.0], reach), "out of reach");
        assert!(!swing_reaches(&t, [0.0, 0.0, -1.0], reach), "behind it");
        t.species = Species::Wolf;
        assert!(!swing_reaches(&t, [0.0, 0.0, 1.2], reach), "not a swing");
    }

    #[test]
    fn temperaments_parse_by_name() {
        for t in Temperament::ALL {
            assert_eq!(Temperament::parse(t.name()), Some(t));
        }
        assert_eq!(Temperament::default(), Temperament::Opportunist);
        assert_eq!(Temperament::parse("berserk"), None);
    }

    /// My own inputs in flight carry my body on from my last snapshot:
    /// each at its stride (a run, a walk with a bow drawn), and only those
    /// the server has not yet run.
    #[test]
    fn strides_in_flight_carry_my_body_on() {
        let mut st = Strides::default();
        let run = InputFrame {
            seq: 10,
            yaw: 0,
            move_z: 127,
            buttons: BTN_SPRINT,
            ..InputFrame::default()
        };
        st.sent(&run);
        st.sent(&InputFrame {
            seq: 11,
            buttons: BTN_SPRINT | BTN_AIM,
            ..run
        });
        // The server ran 9: 10 and 11 are in flight, 12 goes out now.
        st.begin(9, 12);
        assert_eq!(st.trip(), 3);
        let p = st.ahead_of([1.0, 2.0, 3.0]);
        let (fx, fz) = yaw_dir(0);
        let k = (SPRINT_SPEED + WALK_SPEED) / TICK_HZ as f32;
        assert!((p[0] - (1.0 + fx * k)).abs() < 1e-4, "{p:?}");
        assert!((p[2] - (3.0 + fz * k)).abs() < 1e-4, "{p:?}");
        assert_eq!(p[1], 2.0);
        // Once the snapshot shows 11 ran, nothing is in flight.
        st.begin(11, 12);
        assert_eq!(st.ahead_of([1.0, 2.0, 3.0]), [1.0, 2.0, 3.0]);
    }

    /// A drawn bow pointed my way counts as aimed at me with the lead and
    /// the arc an arrow needs; a club held the same way does not, and
    /// neither does a bow pointed well wide.
    #[test]
    fn a_bow_is_aimed_at_me_through_its_lead_and_arc() {
        let book = book();
        let (bow, rock) = (wire("Hunting Bow"), wire("Bat"));
        let me = [0.0, 0.0, 25.0];
        let mut t = player(Some(bow));
        // 25 m off, the look 2 m to my side and raised for the arc.
        t.yaw = yaw_toward(2.0, 25.0);
        t.pitch = super::super::intent::pitch_toward(3.0, 25.0);
        assert!(aimed(&t, me, false, &book));
        t.held = Some(rock);
        assert!(!aimed(&t, me, false, &book), "a rock does not lead");
        t.held = Some(bow);
        t.yaw = yaw_toward(10.0, 25.0);
        assert!(!aimed(&t, me, false, &book), "wide of me");
    }
}
