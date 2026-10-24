//! The local controller behind an agent player: skills that carry out one
//! goal (`mind::Goal`) with ordinary player inputs and actions, over the
//! human client's decoded state. A decision source chooses goals; this file
//! executes them and says how they went. Geometry is shared with the
//! renderer/sim and read from the same seed, slot deltas and pieces the
//! client holds, never from a server `World`.
//!
//! Reflexes run under every goal and need no decision: answer the death
//! screen with the respawn verb, crawl away while wounded, and fight or
//! run (`agent::combat`), which pauses the goal rather than ending it,
//! then take the loser's bag; bandage when hurt and nobody is near; keep
//! a gun loaded.
//! Loot runs (`agent::loot`) smash barrels and pick up what falls out, and
//! empty crates through their panel; a recipe made at a station is crafted
//! at its own workbench or furnace, walking home to it first. The base gets
//! code locks armed with its own code, metal doors and a second bench, and
//! the gear after it is learned and made there (`agent::build`); better
//! armour is worn and better arms belted between goals. On a team it
//! learns the team's chest by seeing a teammate at it, and fills it
//! (`agent::supply`): what the pack carries, or a supply run's gathering.
//! The only verbs sent are the ones a human client sends: input frames,
//! and `Respawn`, `Craft`, `Consume`, `Drink`, `Move`, `Reload`, `Loot`,
//! `Pickup`, `Deploy`, `Place`, `Upgrade`, `Use`, `Container`, `Feed`,
//! `Access`, `Demolish`, `Unlock` and `Research` actions
//! (`crates/server/tests/agent_walls.rs` holds that to the client).

use crate::agent::build::{craft_fits, queue_wait, queued, Act, Builder, Region, Stations, Way};
use crate::agent::combat::{self, Assess, Combat, End, Kit, Mode, Temperament, Verb, RESUME_TICKS};
use crate::agent::cover;
use crate::agent::defend::{self, DefendJob, DefendStats, LetBe, Ward};
use crate::agent::doors::{self, Leave, Through};
use crate::agent::gate::{self, TradeJob, Visit, WorkJob};
use crate::agent::hands::{Hands, Skill};
use crate::agent::home::{self, BagJob, Do, Home, RecoverJob};
use crate::agent::intent::{pitch_toward, yaw_toward, Intent, Look};
use crate::agent::loadout::{Loadout, Role, BELT};
use crate::agent::lock::{LockCode, LockSecret};
use crate::agent::loot::{self, Lid, Loot, LootJob, Prize, Spot};
use crate::agent::lore::{self, ExitJob, ReadJob, SolveJob, SwipeJob, Visit as LoreVisit};
use crate::agent::oven::{self, DeviceJob, Keep, OvenStats, Tend, Work};
use crate::agent::plan::{raw_needs, RAW_ROWS};
use crate::agent::raid::{self, Bases, Means, Raid, RaidJob};
use crate::agent::route::{into_deeper_water, Frontier, Route, Step};
use crate::agent::site::Seen;
use crate::agent::society::{self, Hand, Kinship, Patrol, Society};
use crate::agent::stash::{self, Chore, Ledger, StashJob, Transfer};
use crate::agent::supply::{self, Deliver, HomeJob, TeamHome, Trip, WANT_ROWS};
use crate::agent::tracks::{self, Sight, Species, Tracks};
use crate::agent::wedge::{self, Wedge};
use crate::agent::wiki::{Book, Class, Rules};
use crate::botclient::BotDriver;
use crate::mind::{
    Arms, BodyState, Choice, Distance, Goal, Heard, HeardLine, History, HomeSense, HomeState, Mind,
    Name, Outcome, Place, Range, Report, Say, Sighting, SourceKind, Summary, Threat, Trigger, Why,
    SUMMARY_CRAFTS, SUMMARY_ITEMS, SUMMARY_THREATS,
};
use crate::pace::Pace;
use client_core::core::{
    ClientCore, APPLIED2_BAGS, APPLIED2_CHARGE, APPLIED2_CONT, APPLIED2_MOVE,
    APPLIED2_OWN_STRUCT_HIT, APPLIED_DRANK, APPLIED_RESPAWN, APPLIED_STOCK, APPLIED_STRUCT_HIT,
    APPLIED_VITALS,
};
use client_core::view::ClientView;
use protocol::{EntityState, Welcome, WireError, MAX_STREAM_MSG_BYTES};
use sim_core::build::{LOC_PLANE, MAT_TWIG};
use sim_core::craft::{STATION_FURNACE, STATION_NONE};
use sim_core::deploy::{box_key, cell_center, ACCESS_OP_CREW_JOIN, ACCESS_OP_ENTER, BAG_CAP};
use sim_core::footprint::Pose;
use sim_core::gather::{cell_key, REACH_M};
use sim_core::input::{InputFrame, BTN_CROUCH, BTN_PRIMARY};
use sim_core::inventory::{CONT_BOX, CONT_SELF, CONT_WEAR, CONT_WORLD};
use sim_core::limits::{
    BOX_SLOTS, CRAFT_COUNT_MAX, CRAFT_QUEUE, HOTBAR_SLOTS, INV_SLOTS, MAX_ITEM_DEFS, TICK_HZ,
};
use sim_core::melee;
use sim_core::movement::{Body, POS_XZ_Q, POS_Y_Q};
use sim_core::ranged::{IMPACT_ARROW, IMPACT_BLAST, IMPACT_BULLET, MM_PER_M};
use sim_core::survival::{DRINK_REACH_M, REFUSE_C_FULL, REFUSE_C_NOT_FOOD};
use sim_core::terrain::{self, Haven, Occupant, Slot, CELL_SIZE};
use sim_core::{pitch_dir, yaw_dir};
use std::time::Instant;

// Experiment defaults, DECISIONS.md §open. One cell is examined per input
// frame, so neither the search nor its storage grows with island size.
pub const SIGHT_M: f32 = 32.0;
pub const SIGHT_CELLS: i32 = (SIGHT_M / CELL_SIZE) as i32;
pub const NO_PROGRESS_TICKS: u32 = 3 * TICK_HZ;
pub const RECOVER_TICKS: u32 = TICK_HZ;
pub use crate::agent::combat::FLEE_TICKS;
const SIGHT_WIDTH: i32 = 2 * SIGHT_CELLS + 1;

// Survivor v0 defaults (DECISIONS.md §open "Jev survivor v0"), proposed.
/// A gather or water search that finds nothing in this long fails.
pub const SEARCH_GOAL_SECS: u32 = 20;
/// An explore goal walks this long, then asks again.
pub const EXPLORE_GOAL_SECS: u32 = 20;
/// A wait goal stands still this long.
pub const WAIT_GOAL_SECS: u32 = 5;
/// A follow goal stays with its teammate this long, then asks again.
pub const FOLLOW_GOAL_SECS: u32 = 90;
/// A teammate seen this recently is where its track says, height included.
pub const FOLLOW_SEEN_TICKS: u32 = 10 * TICK_HZ;
/// A teammate this much higher or lower stands on another floor (a half
/// storey is 1.5 m).
pub const FOLLOW_APART_M: f32 = 1.0;
/// How close a follower keeps to its teammate, and the distance past which
/// it runs to catch up.
pub const FOLLOW_NEAR_M: f32 = 4.0;
pub const FOLLOW_RUN_M: f32 = 12.0;
/// A stranger who swung or shot this recently, this near a teammate, is
/// going for them.
pub const GUARD_RECALL_SECS: u32 = 3;
pub const GUARD_M: f32 = 15.0;
/// A follow with no way to its teammate goes through a door this near.
pub const FOLLOW_DOOR_M: f32 = 6.0;
/// Living in the team's base, the walk home is offered farther than this
/// from it...
pub const SHARED_HOME_NEAR_M: u8 = 8;
/// A bag for the team's base that came to nothing (no way onto the
/// cupboard's crew, no room on the floor) is not tried again for this
/// long, unless a teammate gives a new code.
pub const TEAM_BAG_RETRY_TICKS: u32 = 10 * 60 * TICK_HZ;
/// ...and its defence ends once nobody has been about it this long.
pub const SHARED_GUARD_SECS: u32 = 30;
/// A trade at THE GATE or a feed at a work, the walk there included, that
/// has not finished in this long is stuck; one that came to nothing is not
/// offered again for [`VISIT_RETRY_TICKS`].
pub const VISIT_GOAL_SECS: u32 = 360;
pub const VISIT_RETRY_TICKS: u32 = 120 * TICK_HZ;
/// A team invite's accept that has not gone out in this long is stuck.
pub const JOIN_GOAL_SECS: u32 = 5;
/// How near a person must stand for `invite`: the human client's reach
/// for N (`ASSIST_REACH_M`), inside the shard's own 7 m.
pub const INVITE_REACH_M: f32 = 3.0;
/// A give (`Goal::Give`) is offered while its friend or a teammate stands
/// this near: the give's own reach (`sim_core::assist::ASSIST_REACH_M`), the
/// same as an invite's. It runs at most this long, seconds.
pub const GIVE_REACH_M: f32 = INVITE_REACH_M;
pub const GIVE_GOAL_SECS: u32 = 60;
/// An authorize (`Goal::Authorize`): the walk in through the team's doors to
/// its cupboard and the press there, at most this long, seconds; one that
/// came to nothing is not offered again for a while.
pub const AUTHORIZE_GOAL_SECS: u32 = 150;
pub const AUTHORIZE_RETRY_TICKS: u32 = 2 * 60 * TICK_HZ;
/// A meet (`--friend`): the walk to THE GATE and the wait there, at most
/// this long before it asks again, standing this near the stall.
pub const MEET_GOAL_SECS: u32 = 180;
pub const MEET_STOP_M: f32 = 6.0;
/// A trip to a named place (`Goal::Go`) ends this near it, inside the
/// summary's "here"; one not there in this long is stuck (half the island
/// is a few minutes' walk, and the watchdog catches a body that stands).
pub const GO_STOP_M: f32 = 8.0;
pub const GO_GOAL_SECS: u32 = 600;
/// A walk anywhere (`Goal::Walk`) is done this near its point: close enough
/// that a tree or a rock on the very spot does not fail it.
pub const WALK_STOP_M: f32 = 4.0;
/// Before a team says where its friend is, a friend who joined or was seen
/// this recently counts as on the island. A shard keeps a tag after its
/// player leaves, so an older one says nothing.
pub const FRIEND_AWAY_SECS: u32 = 30 * 60;
/// An invite makes its sender the leader of a team of one (as in Rust).
/// Alone on its team this long, past the invite's own minute, it leaves,
/// so the next person's N finds it free.
pub const ALONE_TEAM_SECS: u32 = 65;
/// A supply run gathers this long, then takes what it has to the chest.
pub const SUPPLY_GATHER_SECS: u32 = 180;
/// A walk to the team's chest, through its doors, and out again that has
/// not finished in this long is stuck.
pub const DELIVER_GOAL_SECS: u32 = 180;
/// A build goal that has not finished its milestone in this long is stuck
/// somewhere the builder's own checks missed; it ends, and the mind chooses
/// again. A milestone's crafts and ops take a few minutes.
pub const BUILD_GOAL_SECS: u32 = 600;
/// A visit home (the walk in, the feed, the box) that has not finished in
/// this long is stuck.
pub const STASH_GOAL_SECS: u32 = 120;
/// A walk home through the doors that has not finished in this long is
/// stuck.
pub const GO_HOME_GOAL_SECS: u32 = 120;
/// After a walk home came to nothing (its own lock would not take the
/// code, say), this long before the next.
pub const GO_HOME_RETRY_TICKS: u32 = 60 * TICK_HZ;
/// A walk home (or a visit to the box) that could not reach home is not
/// offered again while the body stays within this of where it failed, for
/// up to [`HOME_STUCK_TICKS`]: from the same spot it fails the same way,
/// in a second, every time it is offered (a report, 2026-10-19).
pub const HOME_STUCK_NEAR_M: f32 = 16.0;
pub const HOME_STUCK_TICKS: u32 = 5 * 60 * TICK_HZ;
/// After a walk to its stations came to nothing, this long before a craft
/// that needs one is offered again: from where it stands it fails the same
/// way at once, and the policy offered it again a second later, for half
/// an hour (Flint, 2026-10-19: 2,135 of them).
pub const STATION_RETRY_TICKS: u32 = 60 * TICK_HZ;
/// A craft's walk to its stations that has not got there in this long is
/// stuck.
pub const CRAFT_WALK_SECS: u32 = 180;
/// On the stand spot: this near it.
const STAND_ON_M: f32 = 0.5;
/// After a build goal could not reach its work (a spot nothing routes to,
/// a passage that will not open) or failed as it began, this long before
/// the next.
pub const BUILD_RETRY_TICKS: u32 = 60 * TICK_HZ;
/// A goal that has failed the moment it began twice running is off the
/// list this long after each such failure: the same summary would choose
/// it again at once and it would fail the same way, a try a second, and
/// the body would stand there doing it for an hour (a heal with no med it
/// could take, a craft whose bench it could not reach: Flint, 2026-10-19).
pub const FLOP_RETRY_TICKS: u32 = 15 * TICK_HZ;
/// How many goals' runs of such failures are kept at once.
const FLOPS: usize = 4;
/// A loot run reports back after this long.
pub const LOOT_GOAL_SECS: u32 = 240;
/// A cook at a fire that has not finished in this long is stuck. A camp
/// fire's grill is one slot, so a full stack of raw meat (20 × 20 s) cooks
/// for 400 s on its own.
pub const COOK_GOAL_SECS: u32 = 480;
/// THE GATE's research table is walked to until it is this near, well
/// inside the deployables' sight (`AOI_ENTER_CM`); this near and not
/// standing, there is none to use.
pub const TABLE_SIGHT_M: f32 = 40.0;
/// A defence (the walk home, the wait, the repairs) or a raid ends by then.
pub const DEFEND_GOAL_SECS: u32 = 300;
pub const RAID_GOAL_SECS: u32 = 300;
/// An arrow or a bullet landing within this of my own shot is my own.
pub const OWN_SHOT_TICKS: u32 = 2 * TICK_HZ;
/// A raid that came to nothing is not offered again for this long.
pub const RAID_HELD_TICKS: u32 = 60 * TICK_HZ;
/// At a place on the map, how long the eyes look round for its crates.
pub const LOOK_ROUND_TICKS: u32 = 8 * TICK_HZ;
/// Swings at one barrel before it is left: three break it.
pub const SMASH_TICKS: u32 = 12 * TICK_HZ;
/// After a barrel breaks, how long its stacks are waited for, and how far
/// round it they are looked for.
pub const GROUND_WAIT_TICKS: u32 = 3 * TICK_HZ / 2;
pub const PICK_AROUND_M: f32 = 3.5;
/// A stack is picked up from this near (the take reaches the nearest one
/// within `LOOT_REACH_M`, so near makes it this one), and pressed for again
/// this often, this many times.
pub const PICK_STAND_M: f32 = 1.0;
pub const PICK_RETRY_TICKS: u32 = TICK_HZ;
pub const PICK_TRIES: u8 = 3;
/// A crate is opened from this near.
pub const OPEN_STAND_M: f32 = 2.5;
/// A tree under this share of its adult size, per mille, is not worth the
/// walk ([`Target::gone`]).
pub const SKIP_TREE_PM: u16 = 500;
/// Presses at one crate that went unanswered or were refused before it
/// is left.
pub const LID_TRIES: u8 = 3;
/// After a move is answered, the panel catches up this soon.
pub const LID_SETTLE_TICKS: u32 = 3;
/// From this long before the session's end the body goes home, shuts its
/// doors and stands inside, so what sleeps there is not free loot.
pub const LOG_OFF_SECS: u32 = 90;
/// An action's answer must arrive within this long (after any craft time).
pub const VERDICT_SECS: u32 = 3;
/// A craft queued behind other jobs is waited for this long at most; past
/// it, a job the queue shows is left to pay out while the body goes on.
pub const QUEUE_WAIT_SECS: u32 = 20;
/// A furnace batch smelts at most this long, so a craft asked after it is
/// not stuck behind it for minutes.
pub const SMELT_BATCH_SECS: u32 = 60;
/// An exploring walk counts a map cell reached this close to its centre.
pub const FRONTIER_STOP_M: f32 = 8.0;
/// An idle body looks somewhere else this often.
pub const GLANCE_TICKS: u32 = 3 * TICK_HZ / 2;
/// Where an idle glance goes, relative to where the idle began: either
/// side just past the cone's edge, over the shoulder, and back.
const GLANCE_OFFSETS: [u16; 4] = [0x2e00, 0u16.wrapping_sub(0x2e00), 0x6a00, 0];
/// Eat or drink until the meter reaches this percentage.
pub const METER_TARGET_PCT: u32 = 80;
/// The sea, the water pass's dear drink (`balance.toml`: half a lake's
/// water for 3 hp a mouthful): only below this, only with this much health,
/// and only up to this. A lake fills to [`METER_TARGET_PCT`] for nothing.
pub const SEA_DRINK_BELOW_PCT: u32 = 35;
pub const SEA_MIN_HP_PCT: u32 = 50;
pub const SEA_TARGET_PCT: u32 = 55;
/// Targets abandoned before a gather goal fails as stuck.
pub const MAX_ABANDONS: u32 = 3;
/// Distances probed for open water on eight bearings, once a second.
pub const WATER_PROBE_M: [f32; 4] = [8.0, 16.0, 24.0, 32.0];
/// How long a resource seen in view is remembered once it leaves it: long
/// enough for a supply run's trip home and back, so the next run walks
/// back to the rocks it was working rather than searching afresh.
pub const RECALL_SECS: u32 = 300;
/// A remembered resource this far off or nearer counts as known (the
/// census, the gather goal's walk back to it); farther, it is forgotten.
/// At the walk's 3 m/s this is about 80 s of walking.
pub const RECALL_WALK_M: f32 = 240.0;
/// The walk back to a remembered resource may take this long before the
/// gather goal gives it up. The search's own [`SEARCH_GOAL_SECS`] does not
/// cut it short: it is 60 m of walking, and a node remembered farther off
/// than that was never reached (#362).
pub const RECALL_WALK_SECS: u32 = 120;
/// Sight-window cells examined per call: one per game tick elapsed since
/// the last call, at most this many. A controller sampled once per slow
/// rendered frame still sweeps at the game's pace, and a call's work stays
/// bounded (each cell is at most one sight ray).
pub const SCAN_CELLS_MAX: u32 = 8;
/// Bodies seen this recently count in the census a request carries.
pub const BODY_RECALL_SECS: u32 = 2;
/// A flee goal runs from the nearest body seen this recently.
pub const THREAT_RECALL_SECS: u32 = 10;
/// A body standing on the swing's own ray this close would take the blow
/// meant for a node: the swing waits.
pub const BYSTANDER_RAY_M: f32 = 3.0;
/// The ray is held off a body by this much more than its radius: a pose
/// that moved since the frame drawn, and hands that wobble. Wider than the
/// margin a watcher calls a seen swing an attack by (`combat::SWING_MISS_M`).
pub const BYSTANDER_MISS_M: f32 = 0.3;
/// A player standing this near a node has it.
pub const NODE_TAKEN_M: f32 = 3.0;
/// A heal goal uses meds until health reaches this percentage, counting
/// what the meds already used are still delivering.
pub const HEAL_TARGET_PCT: u32 = 90;
/// A health bar that has not risen for this long has no heal coming (the
/// slowest med fills a point every few ticks).
pub const HEAL_STALL_TICKS: u32 = TICK_HZ;
/// Outside a heal goal, a body under this percentage of its health
/// bandages itself (to [`HEAL_TARGET_PCT`], counting what is still coming)
/// whenever nobody is near (`combat::safe`), the way a player does after a
/// fight without stopping to think about it.
pub const REFLEX_HEAL_PCT: u32 = 80;
/// A downed body crawls to cover no further than this.
pub const CRAWL_COVER_M: f32 = 10.0;
/// A reload is not asked for again sooner than this after the last.
pub const RELOAD_RETRY_TICKS: u32 = TICK_HZ;
/// Armour put on or a better weapon belted at most this often: the mirror
/// shows the last move first.
pub const DRESS_TICKS: u32 = TICK_HZ;

/// Tool ladders, best first, by catalog name — the player's knowledge of
/// which tool fells a tree and which breaks rock. Never indices; yields,
/// wear and stack sizes stay content's.
pub const TREE_TOOLS: [&str; 3] = ["Metal Hatchet", "Stone Hatchet", "Bat"];
pub const NODE_TOOLS: [&str; 3] = ["Metal Pickaxe", "Stone Pickaxe", "Bat"];

const _: () = assert!(
    MAX_ITEM_DEFS <= ItemBits::CAP,
    "FoodBook and yield sets are ItemBits masks"
);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Waiting,
    Dead,
    Wounded,
    Sleeping,
    Stale,
    Deciding,
    Paused,
    Exploring,
    Approaching,
    Harvesting,
    Recovering,
    Fleeing,
    Fighting,
    Looting,
    Healing,
    Crafting,
    Equipping,
    Eating,
    Drinking,
    SeekingWater,
    Resting,
    Bagging,
    Returning,
    Building,
    GoingHome,
    Leaving,
    Stashing,
    Cooking,
    Recycling,
    Defending,
    Repairing,
    Raiding,
    Following,
    Delivering,
    Trading,
    /// At a stone, a lock or a card door (`agent::lore`).
    Ancients,
    /// Round the ring of posts about home (`agent::society`).
    Patrolling,
    LoggingOff,
}

impl Phase {
    pub fn text(self) -> &'static str {
        match self {
            Phase::Waiting => "Getting ready",
            Phase::Dead => "Dead: answering the respawn screen",
            Phase::Wounded => "Wounded: crawling",
            Phase::Sleeping => "Sleeping",
            Phase::Stale => "Waiting for the shard",
            Phase::Deciding => "Choosing the next goal",
            Phase::Paused => "Paused: model spend cap reached",
            Phase::Exploring => "Exploring",
            Phase::Approaching => "Approaching a target",
            Phase::Harvesting => "Harvesting",
            Phase::Recovering => "Trying another route",
            Phase::Fleeing => "Retreating from danger",
            Phase::Fighting => "Fighting",
            Phase::Looting => "Looting",
            Phase::Healing => "Healing",
            Phase::Crafting => "Crafting",
            Phase::Equipping => "Moving a tool to the belt",
            Phase::Eating => "Eating",
            Phase::Drinking => "Drinking",
            Phase::SeekingWater => "Walking to water",
            Phase::Resting => "Waiting",
            Phase::Bagging => "Putting a sleeping bag down",
            Phase::Returning => "Walking back to the death backpack",
            Phase::Building => "Building the base",
            Phase::GoingHome => "Going home",
            Phase::Leaving => "Leaving the base through its doors",
            Phase::Stashing => "At home: the cupboard and the box",
            Phase::Cooking => "Cooking at a fire",
            Phase::Recycling => "Recycling salvage",
            Phase::Defending => "Home under attack: inside, defending it",
            Phase::Repairing => "Mending the base with the hammer",
            Phase::Raiding => "Raiding a base",
            Phase::Following => "With its teammate",
            Phase::Delivering => "Seeing to the team's boxes",
            Phase::Trading => "At the gate or a work",
            Phase::Ancients => "At a stone, a lock or a card door",
            Phase::Patrolling => "Walking the ring round home",
            Phase::LoggingOff => "Home for the log-off, doors shut",
        }
    }
}

/// Which resource a gather goal wants.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Wood = 0,
    Stone = 1,
    Ore = 2,
    /// The berry bush: food and water.
    Forage = 3,
    /// Hemp: cloth. Its own kind, so a thirsty body walks to berries and
    /// not to whichever plant is nearest.
    Hemp = 4,
}

/// How many [`Kind`]s there are — the length of every per-kind table.
const KINDS: usize = 5;

impl Kind {
    fn of(occupant: Occupant) -> Option<Kind> {
        match occupant {
            Occupant::Tree => Some(Kind::Wood),
            Occupant::StoneNode => Some(Kind::Stone),
            Occupant::MetalNode | Occupant::SulfurNode => Some(Kind::Ore),
            Occupant::BerryBush => Some(Kind::Forage),
            Occupant::Hemp => Some(Kind::Hemp),
            _ => None,
        }
    }

    fn of_goal(goal: Goal) -> Option<Kind> {
        match goal {
            Goal::GatherWood | Goal::SupplyWood => Some(Kind::Wood),
            Goal::GatherStone | Goal::SupplyStone => Some(Kind::Stone),
            Goal::GatherOre | Goal::SupplyOre => Some(Kind::Ore),
            Goal::Forage => Some(Kind::Forage),
            Goal::GatherHemp | Goal::SupplyHemp => Some(Kind::Hemp),
            _ => None,
        }
    }

    /// Picked by hand with `E` rather than swung at.
    fn picked(self) -> bool {
        matches!(self, Kind::Forage | Kind::Hemp)
    }
}

/// A set of item indices, one bit each: 256 of them, so the whole catalog
/// (`MAX_ITEM_DEFS`) fits. An index past the set is never in it.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct ItemBits([u128; 2]);

impl ItemBits {
    pub const CAP: usize = 256;

    /// The set holding `item` alone (empty when `item` is past the set).
    pub fn bit(item: u16) -> Self {
        let mut b = Self::default();
        b.set(item);
        b
    }

    pub fn set(&mut self, item: u16) {
        let i = usize::from(item);
        if i < Self::CAP {
            self.0[i / 128] |= 1 << (i % 128);
        }
    }

    pub fn has(&self, item: u16) -> bool {
        let i = usize::from(item);
        i < Self::CAP && self.0[i / 128] & 1 << (i % 128) != 0
    }

    pub fn or(&mut self, other: Self) {
        self.0[0] |= other.0[0];
        self.0[1] |= other.0[1];
    }

    pub fn any(&self) -> bool {
        self.0 != [0; 2]
    }
}

/// What the eat verb has taught this body, per item index: the sim's own
/// verdicts, the way a player learns by pressing eat. No food table.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FoodBook {
    pub not_food: ItemBits,
    pub feeds: ItemBits,
    pub waters: ItemBits,
    pub tried: ItemBits,
}

impl FoodBook {
    /// Worth trying to eat: known to feed, or never tried and not refused.
    pub fn may_feed(&self, item: u16) -> bool {
        usize::from(item) < ItemBits::CAP
            && (self.feeds.has(item) || (!self.tried.has(item) && !self.not_food.has(item)))
    }

    pub fn waters_known(&self, item: u16) -> bool {
        self.waters.has(item)
    }
}

/// What perception last published. Built by the per-frame scans from the
/// client's own view cone, line of sight and received deltas.
#[derive(Clone, Copy, Debug, Default)]
pub struct Senses {
    pub trees: Sighting,
    pub stone_nodes: Sighting,
    pub ore_nodes: Sighting,
    /// Berry bushes in view; hemp is `hemp`.
    pub bushes: Sighting,
    pub hemp: Sighting,
    pub players: Sighting,
    /// The players that have not declared themselves agents.
    pub humans: Sighting,
    pub animals: Sighting,
    pub water: Sighting,
    /// The water in `water` is the sea's: salt, a dear drink (`water_in_reach`).
    pub water_salt: bool,
    /// Bodies in sight that could hurt this one, nearest first.
    pub threats: [Threat; SUMMARY_THREATS],
    pub threats_len: u8,
    /// The backpack from its last death, while it stands.
    pub backpack: Sighting,
    /// A person or a wolf seen within `THREAT_RECALL_SECS`.
    pub hostile: bool,
    /// The same, not counting people inside the town's safe zone, where
    /// the gate's guns keep the peace: what THE GATE's own goals (trades,
    /// research, a meeting there) go by. Players crowd the gate, and its
    /// trades vanished whenever one was in view (an agent, 2026-10-18).
    pub hostile_afield: bool,
    /// Barrels and crates it knows of and has not emptied, and the nearest
    /// place on the map that keeps crates.
    pub loot: Sighting,
    pub loot_place: Option<Place>,
    /// Other people's bases it has seen that are worth raiding now, by its
    /// temperament and with what it carries (`Bases::targets`).
    pub raid: Sighting,
    /// A stranger is swinging or shooting near a teammate (wire v108).
    pub mate_attacked: bool,
    /// The team's base, where its eyes saw a teammate at a box.
    pub team_home: Sighting,
    /// A person it could invite to its team stands within reach.
    pub invitee: bool,
    /// Its friend or a teammate, up and in sight, stands in hand reach
    /// ([`GIVE_REACH_M`]): someone a give would go to.
    pub give_to: bool,
    /// Its friend (`--friend`), and where its eyes have them.
    pub friend: crate::mind::FriendSense,
    pub friend_at: Option<[f32; 2]>,
    /// Its society (`agent::society`), and whether a walk to THE GATE to
    /// bring its crew together is worth offering.
    pub society: crate::mind::SocietySense,
    pub society_meet: bool,
}

/// What this body remembers of its own recent history, all of it learned
/// from messages it received: how its last goal went, blows and deaths
/// since it last asked, and what the eat verb taught it.
#[derive(Clone, Copy, Debug, Default)]
pub struct Memory {
    pub last: Option<Report>,
    pub hits: u16,
    pub deaths: u32,
    pub respawns: u32,
    pub trigger: Trigger,
    /// How the last fight went.
    pub fight: Option<Why>,
    pub food: FoodBook,
    /// Items each resource kind has been seen to pay (gather receipts),
    /// as masks over item indices: what "room for it" means.
    pub yields: [ItemBits; KINDS],
    /// Sleeping bags it knows it has down.
    pub bags: u8,
    /// A bag failed to go down near here a moment ago (`Home::bag_held`).
    pub bag_held: bool,
    /// The death backpack is not worth the walk (`Home::recover_held`).
    pub recover_held: bool,
    /// The base's next milestone and what it needs (`Builder::survey`).
    pub base: crate::agent::build::Survey,
    /// Where home is from here.
    pub home: HomeSense,
    /// Chores at home, once a second: the box holds what the base needs,
    /// the cupboard wants feeding, the pack has things to put away.
    pub take_out: bool,
    pub feed: bool,
    pub put_away: bool,
    /// A visit to the box failed a moment ago (`Home::stash_held`).
    pub stash_held: bool,
    /// A walk home failed a moment ago (`GO_HOME_RETRY_TICKS`), or could
    /// not reach home from about here (`HOME_STUCK_NEAR_M`).
    pub home_held: bool,
    /// A walk to its stations came to nothing a moment ago
    /// (`STATION_RETRY_TICKS`): no craft that needs one is offered.
    pub station_held: bool,
    /// What the box held when it was last open.
    pub stored: [(u16, u32); stash::STORED_ROWS],
    pub stored_len: u8,
    /// Its own workbench and furnace (`Builder::stations`).
    pub stations: Stations,
    /// What the next milestone's shortfalls come down to
    /// (`agent::plan::raw_needs`).
    pub raw: [(u16, u32); RAW_ROWS],
    pub raw_len: u8,
    /// A loot run came to nothing a moment ago (`Loot::held`).
    pub loot_held: bool,
    /// Raw meat, fuel and a fire to cook on (`agent::oven::can_tend`), and
    /// no cook came to nothing a moment ago.
    pub cook: bool,
    /// Salvage the base does not want whole and a recycler to take it
    /// apart at, and no recycle came to nothing a moment ago.
    pub recycle: bool,
    /// A recycler stands within a recycle's walk, its own or anybody's.
    pub recycler_near: bool,
    /// A found thing to research and the junk for it, or paper to read
    /// (`oven::research`), and no research came to nothing a moment ago.
    pub research: bool,
    /// Something of its base is damaged and can be mended
    /// (`defend::mendable`), seen from near it (`defend::MEND_SIGHT_M`);
    /// a defence came to nothing a moment ago.
    pub damaged: bool,
    /// Its doors, those locked, and the key locks among them
    /// (`Builder::doors`), looked at once a second.
    pub doors: (u8, u8, u8),
    pub defend_held: bool,
    /// Its temperament raids; a raid came to nothing a moment ago.
    pub raids: bool,
    pub raid_held: bool,
    /// It knows a home to walk a patrol round (`agent::society`).
    pub patrol_home: bool,
    /// It carries enough for the base to hand its founder, who is near or
    /// says where it is; no hand-over came to nothing a moment ago.
    pub hand: bool,
    /// Its crew's vending machine stands, and keeping it is worth a walk:
    /// one to put down, wares to stock it with, or takings due a look
    /// (`agent::shop`).
    pub shop_stands: bool,
    pub shop: bool,
    /// A blow, blast or shot at the base fresh enough to call it back
    /// (`defend::DEFEND_ALARM_TICKS`).
    pub alarm: bool,
    /// Chat its client received since it last asked (`EventMsg::Chat`).
    /// `chat` because `tracks` already hears sounds.
    pub chat: Heard,
    /// Someone else spoke locally since it last asked: an external source
    /// is asked early (`Trigger::Heard`).
    pub chat_wake: bool,
    /// Its team changed since it last asked: an invite came, or someone
    /// joined or left. Any source is asked early (`Trigger::Team`).
    pub team_wake: bool,
    /// It lives in the team's base (it knows one, and has no base of its
    /// own); the pack carries something for its boxes, or they hold
    /// something the pack lacks; a trip there failed a moment ago; what
    /// they held when last open; it is under attack; a teammate gave the
    /// team's code.
    pub shared: bool,
    pub deliver: bool,
    pub deliver_held: bool,
    /// A supply run found the team's boxes full a while ago: what another
    /// would gather has nowhere to go.
    pub box_full_held: bool,
    pub home_holds: [(u16, u32); crate::mind::SUMMARY_HOME_HOLDS],
    pub home_holds_len: u8,
    pub home_boxes: u8,
    pub home_alarm: bool,
    pub home_needs: bool,
    pub team_code: bool,
    /// Living there, its bag goes in the team's base: none of its own is
    /// down there yet, and no try came to nothing lately. It is on the
    /// cupboard's crew; it has a bag down there.
    pub team_bag: bool,
    pub crewed: bool,
    pub bag_home: bool,
    /// The team's cupboard is known and it is not on the crew, a lock on it
    /// has the team's code to answer it (or it has none), and no authorize
    /// came to nothing a moment ago ([`AUTHORIZE_RETRY_TICKS`]).
    pub authorize: bool,
    /// A trade at THE GATE, or a feed at a work, came to nothing a moment
    /// ago.
    pub trade_held: bool,
    pub work_held: bool,
    /// A read, a solve, a swipe came to nothing a moment ago.
    pub lore_held: [bool; 3],
    /// Its mind is sent `places` (an external one), so it is offered trips
    /// to them (`Goal::Go`); the place a trip came to nothing a moment ago.
    pub travels: bool,
    pub go_held: Option<Name>,
}

#[derive(Clone, Copy, Debug, Default)]
struct Sweep {
    cells: [Sighting; KINDS],
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SurvivorStats {
    pub phase: Phase,
    /// Goals the watchdog ended because the body would not move
    /// (`agent::wedge`).
    pub wedges: u64,
    pub deaths: u64,
    pub respawns: u64,
    pub respawn_asks: u64,
    /// Retreats begun: guarding from an unseen blow, or escaping.
    pub retreats: u64,
    /// Blows taken, and blows landed on bodies (my hit markers).
    pub hurts: u64,
    pub landed: u64,
    pub goals_done: u64,
    pub goals_failed: u64,
    pub goals_interrupted: u64,
    /// Goals a short fight paused and that then carried on.
    pub goals_resumed: u64,
    /// Answers from the mind held until a fight ended.
    pub deferred: u64,
    /// Held answers dropped when the fight ended: too old by then, the
    /// fight too long, or the goal no longer on offer.
    pub deferred_dropped: u64,
    pub targets_seen: u64,
    pub targets_completed: u64,
    pub targets_abandoned: u64,
    /// Nodes left to a body with a lower id standing at them.
    pub gave_way: u64,
    pub gather_awards: u64,
    pub refusals: u64,
    pub crafted: u64,
    pub eaten: u64,
    pub drinks: u64,
    pub equips: u64,
    /// Meds taken outside a heal goal, and reloads asked for.
    pub reflex_heals: u64,
    pub reloads: u64,
    /// Armour put on and weapons, meds or tools belted between goals.
    pub dressed: u64,
    /// Outclassed kit thrown away between goals (`Survivor::discard`).
    pub discarded: u64,
    pub actions: u64,
    /// Frames an action waited in the hand for its kind's pace.
    pub paced: u64,
    pub unencodable: u64,
    /// Chat (an external source only): lines it said, its own lines heard
    /// back, other bodies' lines heard, lines the chat rule refused, and
    /// unsent lines a newer one replaced.
    pub said: u64,
    pub echoed: u64,
    pub heard: u64,
    pub say_refused: u64,
    pub say_dropped: u64,
}

/// How a survivor is set up beyond its mind.
#[derive(Clone, Copy, Debug, Default)]
pub struct SurvivorOpts {
    /// How good its hands are (`--skill`).
    pub skill: Skill,
    /// How ready it is to start a fight (`--temperament`).
    pub temperament: Temperament,
    /// Where the code on its locks comes from; `None` makes one up for
    /// this process alone.
    pub lock: Option<LockSecret>,
    /// The one wallet it plays with (`--friend`): invited first, never
    /// fought, met at THE GATE (`Goal::Meet`).
    pub friend: Option<protocol::Address>,
    /// The clan it lives in (`jev-bot --society`, `agent::society`): its
    /// job, its crew and its guild.
    pub society: Option<Society>,
    /// Its doors wait for code locks instead of taking key locks with the
    /// doors: others come in by its code. Always so in a society.
    pub code_doors: bool,
    /// A bred brain for the fight-or-run call (`jev-bot --tactics ga1:…`,
    /// `agent::combat::set_tactics`); `None` keeps the sum's.
    pub tactics: Option<sim_core::genome::Genome>,
}

#[derive(Clone, Copy, Debug)]
struct Target {
    cx: u16,
    cz: u16,
    slot: Slot,
}

impl Target {
    fn key(self) -> u32 {
        cell_key(self.cx, self.cz)
    }

    /// The slot at the size it stands now: a young or regrowing tree is
    /// struck at its own size (tree ages v0, `occupy::grown`), so it is
    /// aimed at and seen at that size, or the swing passes over a sapling.
    fn live(self, core: &ClientCore) -> Slot {
        if self.slot.occupant != Occupant::Tree {
            return self.slot;
        }
        match self.size_pm(core) {
            0 | 1000.. => self.slot,
            pm => sim_core::occupy::grown(&self.slot, pm),
        }
    }

    /// Nothing to gather here: harvested, a picked plant still growing
    /// back (`gather::pick` refuses a sprout until it is grown), or a tree
    /// not yet half grown — a sapling or a young wild tree is a swing or
    /// two of wood, and a player walks past it to a grown one.
    fn gone(self, core: &ClientCore) -> bool {
        core.harvested.contains(self.key())
            || (sim_core::gather::pickable(self.slot.occupant)
                && core.harvested.growth(self.key()).is_some())
            || (self.slot.occupant == Occupant::Tree && self.size_pm(core) < SKIP_TREE_PM)
    }

    /// How much of a tree stands, per mille of its adult size.
    fn size_pm(self, core: &ClientCore) -> u16 {
        use sim_core::occupy::Harvested;
        core.harvested
            .tree_pm(core.predict.seed(), self.cx, self.cz)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pending {
    Respawn,
    Craft { item: u16 },
    Equip,
    Consume { item: u16, food: u16, water: u16 },
    Drink,
}

/// Health still on its way from meds used, read off the bar the way a
/// player reads it: the server folds what is left of one bandage into the
/// next (`survival.rs`), so the pool is what counts, not the last dose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Dose {
    /// Health when last looked at.
    hp: u16,
    /// Heal still to arrive, the med awaiting its answer included.
    rem: u16,
    /// That med's heal, taken back if it is refused.
    sent: u16,
    /// When the bar last rose, or a med went in.
    rose: u32,
}

/// How a step of taking meds went.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Dosed {
    /// An answer, or the bar, is still coming.
    Waiting,
    /// A med went in.
    Took,
    /// Health is at the mark.
    Enough,
    NoMeds,
    Failed(Why),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Verdict {
    Ok,
    Full,
    Refused,
    NoWater,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CraftStep {
    Start,
    Sent { ticks: u32, recipe: u16 },
    Equip,
    Equipped,
}

#[derive(Clone, Copy, Debug)]
struct Active {
    goal: Goal,
    started: u32,
    /// A gather leg began here: the goal's start, or, on a supply run, the
    /// last node's fall.
    leg: u32,
    /// A supply run has turned for the team's chest.
    delivering: bool,
    asked: u32,
    gained: u32,
    abandons: u32,
    craft: CraftStep,
    fled: bool,
    /// A fight took the frames from this tick on; the goal waits.
    paused: Option<u32>,
}

/// What a hand-over hands (`Survivor::hand_to`): the base's raw stuff to its
/// founder (`HandOver`), or every stack of one item (`Give`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Handing {
    Base,
    Item(u16),
}

/// An authorize under way ([`Survivor::authorize`]): in through the team's
/// doors to its cupboard, the code at its lock (once) or the crew's join at
/// a bare one, and its answer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum AuthLeg {
    #[default]
    In,
    /// At the cupboard: what it takes to get on the crew.
    Crew,
    /// The crew's join went out at this tick; the answer is awaited.
    Sent(u32),
}

#[derive(Clone, Copy, Debug, Default)]
struct Auth {
    leg: AuthLeg,
    /// The team's code went in at the cupboard's lock this authorize.
    knocked: bool,
    /// A deploy refusal answered the join in flight (`REFUSE_D_OWNER`: the
    /// crew does not admit it).
    refused: bool,
    /// Since when the eyes have been on the cupboard, and where.
    held: Option<(u32, [f32; 3])>,
}

/// A bag on its way to the team's base ([`Survivor::bag_in`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BagLeg {
    /// Where it stands, on bare ground.
    Here,
    /// In through the team's doors to its cupboard.
    In,
    /// The cupboard's crew.
    Crew,
    /// The base's floor.
    Floor,
}

pub struct Survivor {
    pub mind: Mind,
    pub stats: SurvivorStats,
    pub history: History,
    /// Units received per item index, across every life.
    pub gathered: [u32; MAX_ITEM_DEFS],
    core: Option<Box<ClientCore>>,
    haven: Option<Haven>,
    /// A haven already solved for a seed ([`Survivor::know_haven`]): the
    /// next welcome on that seed takes it instead of solving its own.
    known_haven: Option<(u64, Haven)>,
    senses: Senses,
    memory: Memory,
    sweep: Sweep,
    /// Other bodies, as eyes and ears found them. Boxed once here: the
    /// interpolation history behind it is the frame path's largest buffer.
    tracks: Box<Tracks>,
    /// The only writer of the view: every skill's look goes through them.
    hands: Hands,
    /// The legs' way somewhere, and the coarse map of where they have been.
    route: Route,
    frontier: Frontier,
    /// The fight reflex; it takes the frame from the goal while it runs.
    combat: Combat,
    /// The belt policy's ladders, by wire id once the catalog is in.
    loadout: Loadout,
    dose: Option<Dose>,
    /// The med awaiting its answer was the reflex's, not a goal's.
    medic: bool,
    /// The slot the last frame held, and how many ticks my input takes to
    /// be executed (the frame's sequence against the one the server last
    /// said it ran).
    sel: u8,
    ack_age: u32,
    /// When a reload was last asked for.
    reload_at: Option<u32>,
    /// When armour or the belt was last seen to.
    dressed_at: Option<u32>,
    /// The tick a bandage this bot is wrapping on has landed by (its
    /// `use_ticks` and a second for the lane), while one is.
    wrap_until: Option<u32>,
    /// Where a downed body is crawling to, if anywhere, and when that was
    /// looked for: a scan that found nothing waits before the next.
    crawl_to: Option<(Option<[f32; 2]>, u32)>,
    /// Its bags and death backpack, the skills that use them, and the
    /// buildings it has seen.
    home: Home,
    bag_job: BagJob,
    /// Where a bag for the team's base is on its way: in, onto the
    /// cupboard's crew, the floor. The code went in at the cupboard on this
    /// trip; a trip there came to nothing at this tick.
    bag_leg: BagLeg,
    bag_knocked: bool,
    team_bag_failed: Option<u32>,
    recover_job: RecoverJob,
    seen: Seen,
    /// The base it is building, and the walk through its doors.
    builder: Builder,
    /// What the code on its locks is made from (never printed).
    lock: LockSecret,
    /// A visit home, and what the box held when last open.
    stash_job: StashJob,
    ledger: Ledger,
    /// The barrels and crates it knows of, and the loot run in hand.
    loot: Loot,
    loot_job: LootJob,
    /// A cook at a fire or a recycle at a recycler, the fire pit and the
    /// recycler it put down last, what they have come to, and when a cook
    /// or a recycle last came to nothing.
    oven_job: DeviceJob,
    fire: Option<(u16, u16)>,
    recycler: Option<(u16, u16)>,
    /// The crew's vending machine it put down, and when it last kept it
    /// (`agent::shop`).
    shop: Option<(u16, u16)>,
    shop_kept: Option<u32>,
    pub oven_stats: OvenStats,
    cook_failed: Option<u32>,
    recycle_failed: Option<u32>,
    research_failed: Option<u32>,
    /// A research goal's read of the paper is queued: the goal ends once
    /// it has gone out (an ended goal's outbox is dropped).
    paper_read: bool,
    /// This research's sample is out of the pack and its paper not in it:
    /// in the table, or paper there. A goal that ends so leaves it
    /// `research_left`, and the next research goes back for it.
    sample_in_table: Option<u16>,
    research_left: Option<u16>,
    shop_failed: Option<u32>,
    /// When a walk home last came to nothing.
    home_failed: Option<u32>,
    /// When and where a walk home or a visit to the box last could not
    /// reach home (`HOME_STUCK_NEAR_M`).
    home_stuck: Option<(u32, [f32; 2])>,
    /// When a walk to its stations last came to nothing.
    station_failed: Option<u32>,
    /// Where the body stood this frame.
    here: [f32; 2],
    /// When a build goal last could not reach its work.
    build_failed: Option<u32>,
    /// Goals that failed the moment they began: when last, and how many
    /// times running.
    flops: [Option<(Goal, u32, u8)>; FLOPS],
    /// A defence of home, and what defences came to.
    defend_job: DefendJob,
    pub defend_stats: DefendStats,
    defend_failed: Option<u32>,
    /// What defences gave up on, let be by the next ones a while.
    defend_let_be: LetBe,
    /// Other people's bases as its eyes found them, and the raid in hand.
    bases: Bases,
    raid_job: RaidJob,
    raid_failed: Option<u32>,
    /// The team's base as its eyes found it, the trip into it, what the
    /// pack lacks that its boxes hold, when a trip last came to nothing,
    /// when it was last under attack, and the code a teammate gave.
    shared: TeamHome,
    home_job: HomeJob,
    wants: [(u16, u32); WANT_ROWS],
    wants_len: u8,
    /// Out of a teammate's base, and through the doors a teammate went by.
    leave: Leave,
    pub delivered: u64,
    pub took: u64,
    deliver_failed: Option<u32>,
    /// A supply run last found the team's boxes full, then: no run for a
    /// while ([`supply::BOX_FULL_RETRY_TICKS`]).
    box_full: Option<u32>,
    home_alarm: Option<u32>,
    team_code: Option<LockCode>,
    /// A trade at THE GATE and a feed at a work, and when one last came to
    /// nothing.
    trade_job: TradeJob,
    work_job: WorkJob,
    trade_failed: Option<u32>,
    work_failed: Option<u32>,
    /// The place a trip (`Goal::Go`) last came to nothing at, and when.
    go_failed: Option<(Name, u32)>,
    /// Where the walk under way (`Goal::Walk`) is going, fixed when it
    /// began (the goal's start tick and the point), so a turn on the way
    /// does not move it.
    walk_to: Option<(u32, [f32; 2])>,
    /// A stone read, a lock solved, a card door opened, the way out of a
    /// card room (`agent::lore`), and when a read, a solve or a swipe last
    /// came to nothing (in that order).
    read_job: ReadJob,
    solve_job: SolveJob,
    swipe_job: SwipeJob,
    exit_job: ExitJob,
    lore_failed: [Option<u32>; 3],
    /// When this body last shot: an impact right after is its own.
    shot_at: Option<u32>,
    /// Salvage a recycle leaves whole (`Builder::kept_whole`), once a
    /// second.
    keep: Keep,
    /// The session ends this many ticks after the welcome (`set_deadline`),
    /// and the tick that is, once welcomed.
    deadline_after: Option<u32>,
    deadline: Option<u32>,
    welcomed: Option<u32>,
    /// An answer that arrived mid-fight, and when; judged once the fight
    /// is over.
    deferred: Option<(Choice, Instant)>,
    /// An idle look round: since when, from which heading, which offset.
    glance: Option<(u32, u16, u8)>,
    water_at: Option<u32>,
    water_yaw: Option<u16>,
    /// The nearest open water the last probe found, metres.
    water_point: Option<[f32; 2]>,
    /// The water a drink goal's seek is walking to.
    seek: Option<[f32; 2]>,
    scan: i32,
    scanned_at: Option<u32>,
    /// The nearest resource of each kind last seen, and when.
    recall: [Option<(Target, u32)>; KINDS],
    /// A sweep of the sight window has completed since the body appeared.
    sensed: bool,
    seen_tick: Option<u32>,
    seen_at: Instant,
    goal: Option<Active>,
    target: Option<Target>,
    target_gain: u32,
    skipped: Option<u32>,
    progress_tick: u32,
    best_distance: f32,
    recovery: Option<(u32, u16)>,
    last_hurt: Option<(u32, u16)>,
    heading: Option<u16>,
    outbox: Option<([u8; MAX_STREAM_MSG_BYTES], usize)>,
    /// The lines to say (`BotDriver::say`), one per chat refill, kept on
    /// this side so a line is never sent into the limiter to be dropped
    /// there.
    say_line: Option<crate::mind::Lines>,
    /// Where the line above goes (`Say`): a team line with no team to hear
    /// it goes to whoever is near.
    say_to: protocol::Channel,
    /// A line its voice could not say, in chat-sized pieces, typed to whoever
    /// is near once `say_line` is clear.
    voice_typed: std::collections::VecDeque<protocol::ChatText>,
    /// Its team has someone besides itself to hear the team channel.
    has_mates: bool,
    /// The stranger going for a teammate, nearest them (`perceive`).
    guard: Option<u32>,
    /// The watchdog over a running goal, and the back-off after it trips:
    /// since when, and which way (`agent::wedge`).
    wedge: Wedge,
    backoff: Option<(u32, u16)>,
    /// Who `invite` would invite: a person up and within
    /// [`INVITE_REACH_M`], not an agent, not on its team.
    invitee: Option<u32>,
    /// Who a give would go to now: its friend, else the nearest teammate,
    /// up and in sight within [`GIVE_REACH_M`]; and who the give under way
    /// is handing to.
    give_to: Option<u32>,
    hand_target: u32,
    /// An authorize under way, and when the last came to nothing.
    auth: Auth,
    auth_failed: Option<u32>,
    /// Since when it has led a team of one, if it does.
    alone_since: Option<u32>,
    /// Its friend (`--friend`): their wallet, the player id the shard gave
    /// them, since when that id was known, and when its eyes last had them.
    friend: Option<protocol::Address>,
    friend_id: u32,
    friend_since: u32,
    friend_seen: u32,
    /// Its society (`agent::society`): who it is in it, who the rest are
    /// on this island, the nearest crewmate in sight not yet on its team
    /// (id, where, how far), and the patrol under way.
    society: Option<Society>,
    kin: Kinship,
    crew_near: Option<(u32, [f32; 2], f32)>,
    patrol: Patrol,
    hand: Hand,
    hand_failed: Option<u32>,
    patrol_failed: Option<u32>,
    /// The team's invite and size when last looked at, to wake on a change.
    team_seen: (u32, u8),
    /// What the request out was told it had heard, and the hits it had
    /// taken: a newer look at it keeps them beside what came since.
    asked_heard: Heard,
    asked_hits: u16,
    say_at: u32,
    /// The server's per-kind pace, kept on this side the way an honest
    /// client keeps it: an early action waits in the outbox rather than in
    /// the server's hand, where it would block the next one.
    pace: Pace,
    /// The game's rules from the shipped content, built at the first
    /// welcome, and the same rules keyed by wire item once the catalog is in.
    rules: Option<Box<Rules>>,
    book: Book,
    awaiting: Option<(Pending, u32)>,
    verdict: Option<Verdict>,
    learning: Option<(u16, u16, u16)>,
    /// Its ears and voice, when it has them (`--voice`, `agent_voice.rs`).
    pub voice: Option<Box<crate::agent_voice::AgentVoice>>,
    /// Bug reports its mind wrote, waiting out the gap (`bugs.rs`).
    bugs: crate::bugs::Tray,
}

impl Survivor {
    pub fn new(mind: Mind) -> Self {
        Self::with(mind, SurvivorOpts::default())
    }

    /// Hand over `terrain::haven(seed)` already solved, for a process that
    /// seats many agents on one seed: the solve costs most of a second, and
    /// it is a pure function of the seed.
    pub fn know_haven(&mut self, seed: u64, haven: Haven) {
        self.known_haven = Some((seed, haven));
    }

    /// Put its clocks on `now`, for a driver that keeps its own (the
    /// lockstep island): `welcome` and the mind read the wall clock, which
    /// a synthetic clock started earlier would trail.
    pub fn set_clock(&mut self, now: Instant) {
        self.seen_at = now;
        self.mind.set_clock(now);
    }

    pub fn with(mind: Mind, opts: SurvivorOpts) -> Self {
        Self {
            mind,
            stats: SurvivorStats::default(),
            history: History::default(),
            gathered: [0; MAX_ITEM_DEFS],
            core: None,
            haven: None,
            known_haven: None,
            senses: Senses::default(),
            memory: Memory::default(),
            sweep: Sweep::default(),
            tracks: Box::new(Tracks::new()),
            hands: Hands::new(opts.skill),
            route: Route::new(),
            frontier: Frontier::new(),
            combat: {
                let mut c = Combat::new(opts.temperament);
                c.set_tactics(opts.tactics.map(|g| {
                    sim_core::genome::Net::build(crate::agent::combat::tactics_shape(), &g)
                }));
                c
            },
            loadout: Loadout::new(),
            dose: None,
            medic: false,
            sel: 0,
            ack_age: 1,
            reload_at: None,
            dressed_at: None,
            wrap_until: None,
            crawl_to: None,
            home: Home::new(),
            bag_job: BagJob::default(),
            bag_leg: BagLeg::Here,
            bag_knocked: false,
            team_bag_failed: None,
            recover_job: RecoverJob::default(),
            seen: Seen::new(),
            builder: Builder::new().keyed(!opts.code_doors && opts.society.is_none()),
            lock: opts
                .lock
                .unwrap_or_else(|| LockSecret::Code(LockCode::random())),
            stash_job: StashJob::default(),
            ledger: Ledger::EMPTY,
            loot: Loot::new(),
            loot_job: LootJob::default(),
            oven_job: DeviceJob::new(Work::Cook),
            fire: None,
            recycler: None,
            shop: None,
            shop_kept: None,
            oven_stats: OvenStats::default(),
            cook_failed: None,
            recycle_failed: None,
            research_failed: None,
            paper_read: false,
            sample_in_table: None,
            research_left: None,
            shop_failed: None,
            home_failed: None,
            home_stuck: None,
            station_failed: None,
            here: [0.0; 2],
            build_failed: None,
            flops: [None; FLOPS],
            defend_job: DefendJob::default(),
            defend_stats: DefendStats::default(),
            defend_failed: None,
            defend_let_be: LetBe::default(),
            bases: Bases::new(),
            raid_job: RaidJob::default(),
            raid_failed: None,
            shared: TeamHome::default(),
            home_job: HomeJob::default(),
            wants: [(0, 0); WANT_ROWS],
            wants_len: 0,
            leave: Leave::default(),
            delivered: 0,
            took: 0,
            deliver_failed: None,
            box_full: None,
            home_alarm: None,
            team_code: None,
            trade_job: TradeJob::default(),
            work_job: WorkJob::default(),
            go_failed: None,
            walk_to: None,
            trade_failed: None,
            work_failed: None,
            read_job: ReadJob::default(),
            solve_job: SolveJob::default(),
            swipe_job: SwipeJob::default(),
            exit_job: ExitJob::default(),
            lore_failed: [None; 3],
            shot_at: None,
            keep: Keep::NONE,
            deadline_after: None,
            deadline: None,
            welcomed: None,
            deferred: None,
            glance: None,
            water_at: None,
            water_yaw: None,
            water_point: None,
            seek: None,
            scan: 0,
            scanned_at: None,
            recall: [None; KINDS],
            sensed: false,
            seen_tick: None,
            seen_at: Instant::now(),
            goal: None,
            target: None,
            target_gain: 0,
            skipped: None,
            progress_tick: 0,
            best_distance: f32::INFINITY,
            recovery: None,
            last_hurt: None,
            heading: None,
            outbox: None,
            say_line: None,
            say_to: protocol::Channel::Local,
            voice_typed: std::collections::VecDeque::new(),
            has_mates: false,
            guard: None,
            wedge: Wedge::default(),
            backoff: None,
            invitee: None,
            give_to: None,
            hand_target: 0,
            auth: Auth::default(),
            auth_failed: None,
            alone_since: None,
            friend: opts.friend,
            friend_id: 0,
            friend_since: 0,
            friend_seen: 0,
            society: opts.society,
            kin: Kinship::default(),
            crew_near: None,
            patrol: Patrol::default(),
            hand: Hand::default(),
            hand_failed: None,
            patrol_failed: None,
            team_seen: (0, 0),
            asked_heard: Heard::EMPTY,
            asked_hits: 0,
            say_at: 0,
            pace: Pace::default(),
            rules: None,
            book: Book::EMPTY,
            awaiting: None,
            verdict: None,
            learning: None,
            voice: None,
            bugs: crate::bugs::Tray::default(),
        }
    }

    /// The client state this controller reads, once the welcome arrived.
    pub fn core(&self) -> Option<&ClientCore> {
        self.core.as_deref()
    }

    /// The mind back, for another body (`bench.rs` plays one mind through
    /// every run).
    pub fn into_mind(self) -> Mind {
        self.mind
    }

    pub fn goal(&self) -> Option<Goal> {
        self.goal.map(|a| a.goal)
    }

    /// Server ticks the current goal has run.
    pub fn goal_ticks(&self) -> Option<u32> {
        let now = self.seen_tick.unwrap_or_default();
        self.goal.map(|a| now.wrapping_sub(a.started))
    }

    pub fn senses(&self) -> &Senses {
        &self.senses
    }

    /// The team's base as its eyes found it.
    pub fn shared(&self) -> &TeamHome {
        &self.shared
    }

    /// Ziggurat levers pulled to get out of a card room.
    pub fn levers(&self) -> u64 {
        self.exit_job.pulls
    }

    pub fn memory(&self) -> &Memory {
        &self.memory
    }

    /// What this body's eyes and ears know of other bodies.
    pub fn tracks(&self) -> &Tracks {
        &self.tracks
    }

    /// The hands that turn every look into a view.
    pub fn hands(&self) -> &Hands {
        &self.hands
    }

    /// The legs' routes, and what they cost.
    pub fn route(&self) -> &Route {
        &self.route
    }

    /// The fight reflex.
    pub fn combat(&self) -> &Combat {
        &self.combat
    }

    /// Give its fights a bred brain, or take it away
    /// (`Combat::set_tactics`): what `SurvivorOpts::tactics` does at birth,
    /// for a net that no genome describes (a test's hand-set one).
    pub fn set_tactics(&mut self, net: Option<sim_core::genome::Net>) {
        self.combat.set_tactics(net);
    }

    /// Its bags, and what waking and recovering have come to.
    pub fn home(&self) -> &Home {
        &self.home
    }

    /// Other people's building, as far as its eyes found it.
    pub fn seen(&self) -> &Seen {
        &self.seen
    }

    /// Its own base: the plot, what stands, what the next part needs.
    pub fn builder(&self) -> &Builder {
        &self.builder
    }

    /// What its box held when the panel was last open.
    pub fn ledger(&self) -> &Ledger {
        &self.ledger
    }

    /// The barrels and crates it knows of, and what its loot runs took.
    pub fn loot(&self) -> &Loot {
        &self.loot
    }

    /// Other people's bases, as its eyes found them.
    pub fn bases(&self) -> &Bases {
        &self.bases
    }

    /// The session ends `ticks` server ticks after the welcome (`jev-bot
    /// --seconds`); `None` for no end. From [`LOG_OFF_SECS`] before it the
    /// body goes home, shuts its doors and stands inside. Counted from the
    /// first welcome, so a reconnect does not push it back.
    pub fn set_deadline(&mut self, ticks: Option<u32>) {
        self.deadline_after = ticks;
        self.deadline = ticks.zip(self.welcomed).map(|(t, w)| w.saturating_add(t));
    }

    /// A new shift (`jev-bot --shift`): the next session ends `ticks` after
    /// its own welcome, not the first one's, and this body goes home for it
    /// the way it does at a run's end.
    pub fn new_shift(&mut self, ticks: u32) {
        self.welcomed = None;
        self.set_deadline(Some(ticks));
    }

    /// Its society's job, if it is in one.
    pub fn role(&self) -> Option<society::Role> {
        self.society.map(|s| s.role)
    }

    /// Sample other bodies this many ticks behind the newest snapshot: the
    /// session's playout delay, where one runs (`sim_core::limits::
    /// INTERP_DELAY_TICKS` until told).
    pub fn set_playout(&mut self, ticks: u8) {
        self.tracks.set_playout(ticks);
    }

    /// What this body knows of the game's rules, by wire item id; empty
    /// until the catalog has arrived.
    pub fn wiki(&self) -> &Book {
        &self.book
    }

    /// Units of the named item this body has received in all its lives.
    pub fn gathered_of(&self, name: &str) -> u32 {
        let Some(core) = self.core.as_deref() else {
            return 0;
        };
        (0..usize::from(core.catalog.count).min(MAX_ITEM_DEFS))
            .filter(|&i| core.catalog.name(i) == name.as_bytes())
            .map(|i| self.gathered[i])
            .sum()
    }

    /// Units of the named item in the pack now.
    pub fn held(&self, name: &str) -> u32 {
        self.core
            .as_deref()
            .map_or(0, |core| amount(core, name.as_bytes()))
    }

    /// The summary this body would send now; pure, for display and tests.
    pub fn summary(&self, view: &ClientView, player: u32) -> Option<Summary> {
        let core = self.core.as_deref()?;
        Some(self.summarize(core, view, player))
    }

    /// `observe`, plus the goal this body is running now.
    fn summarize(&self, core: &ClientCore, view: &ClientView, player: u32) -> Summary {
        let mut s = observe(
            core,
            view,
            player,
            &self.senses,
            &self.memory,
            &self.book,
            &self.loadout,
        );
        let tick = view.newest_applied.unwrap_or_default();
        for &(goal, at, runs) in self.flops.iter().flatten() {
            if runs >= 2 && tick.wrapping_sub(at) < FLOP_RETRY_TICKS {
                s.withdraw(goal);
            }
        }
        s.current = self.goal.map(|a| Report {
            goal: a.goal,
            outcome: Outcome::Running,
            gained: a.gained,
            secs: tick.wrapping_sub(a.started) / TICK_HZ,
        });
        s
    }

    pub fn frame_at(
        &mut self,
        view: &ClientView,
        player: u32,
        seq: u16,
        now: Instant,
    ) -> InputFrame {
        let Some(mut core) = self.core.take() else {
            return InputFrame {
                seq,
                pitch: 128,
                ..InputFrame::default()
            };
        };
        let frame = self.frame_with(&mut core, view, player, seq, now);
        self.core = Some(core);
        frame
    }

    /// Hold a bug report its mind wrote (`bugs.rs`) with what the body
    /// knows and the mind is never shown: the seed and where it stands.
    fn hold_bug(
        &mut self,
        note: crate::bugs::Note,
        core: &mut ClientCore,
        view: &ClientView,
        player: u32,
        body: &EntityState,
    ) {
        let (seed, _) = core.island();
        let summary = self.summarize(core, view, player);
        let mut seen = crate::external::External::observation(0, &summary);
        if let Some(o) = seen.as_object_mut() {
            o.remove("id");
            o.remove("protocol");
        }
        let facts = crate::bugs::Facts {
            seed,
            tick: view.newest_applied.unwrap_or_default(),
            pos: [
                body.qx as f32 * POS_XZ_Q,
                body.qy as f32 * POS_Y_Q,
                body.qz as f32 * POS_XZ_Q,
            ],
            player_id: player,
            vitals: ::client::report::Vitals {
                hp: core.hp,
                hp_max: core.hp_max,
                food: core.food,
                water: core.water,
                dead: core.dead || body.dead,
            },
            goal: summary.current.map(|r| r.goal.label().as_str().to_owned()),
            last: summary
                .last
                .map(|r| format!("{} ({})", r.goal.label().as_str(), r.outcome.word())),
            seen,
        };
        self.bugs.hold(note, facts);
    }

    /// Stop what this body was doing: down, dead, a stale link or a new
    /// life. An answer held for the old situation goes with it.
    fn halt(&mut self) {
        self.target = None;
        self.recovery = None;
        self.combat.forget();
        self.deferred = None;
        self.builder.halt();
    }

    /// Would switching goals now lose nothing: no op of a long skill
    /// waiting on its answer, no door left open mid-walk.
    fn at_checkpoint(&self) -> bool {
        self.goal.is_none()
            || (self.builder.at_checkpoint()
                && self.stash_job.at_checkpoint()
                && self.oven_job.at_checkpoint()
                && self.defend_job.at_checkpoint()
                && self.raid_job.at_checkpoint())
    }

    /// The session's end is near and there is a home to sleep in: its
    /// own, or (in a society) the crew's base it lives in.
    fn log_off_due(&self, core: &ClientCore, tick: u32) -> bool {
        self.deadline
            .is_some_and(|end| tick.saturating_add(LOG_OFF_SECS * TICK_HZ) >= end)
            && ((self.builder.survey().hearth && self.builder.walled(core))
                || (self.society.is_some() && self.shared_home()))
    }

    /// The log-off: the goal in hand ends at its next checkpoint (a walk
    /// home already under way carries on), then home and in through the
    /// doors, shutting them behind, and stand there. The mind is not asked
    /// meanwhile; a fight is still answered, before this.
    fn log_off(&mut self, core: &mut ClientCore, body: &EntityState, tick: u32) -> Intent {
        match self.goal {
            Some(a) if a.goal == Goal::GoHome || !self.at_checkpoint() => {
                return self.run_goal(core, body, tick);
            }
            Some(_) => self.end_goal(tick, Outcome::Interrupted(Why::LogOff)),
            None => {}
        }
        // A crewmate sleeps in the crew's base, by its first box.
        if self.shared_home() {
            let at_home = self.shared.anchor().is_some_and(|[hx, hz]| {
                let (x, z) = (body.qx as f32 * POS_XZ_Q, body.qz as f32 * POS_XZ_Q);
                (hx - x).hypot(hz - z) <= supply::STAY_NEAR_M + 1.5
            });
            if at_home || self.home_held(tick) {
                self.stats.phase = Phase::LoggingOff;
                return Intent::IDLE;
            }
            self.begin(Goal::GoHome, tick);
            return self.run_goal(core, body, tick);
        }
        // The walk home came to nothing a moment ago: the session ends
        // where it stands rather than at a door that will not open.
        if self.home_held(tick) && self.builder.region(body) == Region::Outside {
            self.stats.phase = Phase::LoggingOff;
            // Never asleep in the town: its safe zone puts sleepers out.
            return out_of_town(core, body).unwrap_or(Intent::IDLE);
        }
        let settled = self.builder.region(body) == Region::Room && !self.builder.passing();
        if settled && !self.builder.door_open(core) {
            self.stats.phase = Phase::LoggingOff;
            return Intent::IDLE;
        }
        // A door stands open behind it (somebody else's hand): out through
        // both and back in, which shuts each behind it.
        if settled || self.builder.leaving() {
            if let Some(intent) = self.leave(core, body, tick) {
                return intent;
            }
        }
        self.begin(Goal::GoHome, tick);
        self.run_goal(core, body, tick)
    }

    /// One frame, with the client state kept the way the human client
    /// keeps it: regrowth measured at the newest tick (the client's own
    /// `advance` does this, and the agent never calls it), the body where
    /// the snapshot has it, and the frame
    /// sent recorded as the live input, which readers such as
    /// `ClientCore::mag` key on. Whatever the skills decided, the hands
    /// turn into the frame.
    fn frame_with(
        &mut self,
        core: &mut ClientCore,
        view: &ClientView,
        player: u32,
        seq: u16,
        now: Instant,
    ) -> InputFrame {
        if let Some(tick) = view.newest_applied {
            core.harvested.set_now(tick);
        }
        // Ticks between this input leaving and the server running it, as
        // the acks show: part of what an arrow leads a body by.
        self.ack_age = u32::from(seq.wrapping_sub(view.last_executed_seq)).clamp(1, 15);
        self.combat.strides.begin(view.last_executed_seq, seq);
        // No prediction runs here: the snapshot's body is the body, the way
        // a spectator's client takes it. Readers keyed on the predicted
        // body (the own-backpack tag at a death) then see where it stands.
        if let Some(body) = view.get(player) {
            core.predict.adopt_authoritative(body);
        }
        let intent = self.decide(core, view, player, now);
        let body = view.get(player);
        // A skill that names no slot keeps the one in hand.
        let base = InputFrame {
            seq,
            yaw: body.map_or(0, |b| b.yaw),
            pitch: 128,
            sel: self.sel,
            ..InputFrame::default()
        };
        // Aimed from where my body is by the time this input runs, from
        // the eye of the stance this frame presses.
        let eye = self
            .combat
            .strides
            .ahead_of(body.map_or([0.0; 3], |b| pressed_eye(b, intent.buttons)));
        let mut f = self.hands.drive(&intent, &self.tracks, eye, base);
        // In the water (the water pass) an agent swims at the surface and
        // never dives: a look down at what it walks toward would steer it
        // under, and nothing here counts its breath. Under anyway — knocked
        // off a ledge, say — it comes up.
        if let Some(b) = body {
            let feet = b.qy as f32 * sim_core::movement::POS_Y_Q;
            if sim_core::movement::swimming_at(feet) {
                f.pitch = f.pitch.max(crate::agent::intent::LEVEL_PITCH);
                f.buttons &= !sim_core::input::BTN_CROUCH;
                if sim_core::movement::mouth_under(feet) {
                    f.buttons |= sim_core::input::BTN_JUMP;
                }
            }
        }
        // A bandage being wrapped on keeps the hand that began it up: a
        // switch would put it away unspent (`survival::tick_use`).
        if self.wrapping(view.newest_applied.unwrap_or_default()) {
            f.sel = self.sel;
        }
        self.combat.strides.sent(&f);
        core.set_input(f.buttons, f.yaw, f.pitch, f.move_x, f.move_z, f.sel);
        self.sel = f.sel;
        f
    }

    /// The frame's arbitration, in order: eyes and ears; the body's own
    /// state (dead, asleep, down); the mind's answer; the fight reflex,
    /// which pauses the goal while it runs; the goal; then the overlays.
    /// The hands come after, in `frame_with`.
    fn decide(
        &mut self,
        core: &mut ClientCore,
        view: &ClientView,
        player: u32,
        now: Instant,
    ) -> Intent {
        if view.newest_applied.is_some() && view.newest_applied != self.seen_tick {
            self.seen_tick = view.newest_applied;
            self.seen_at = now;
        }
        self.tracks.feed(view, player);
        self.route.begin_tick();
        let Some(body) = view.get(player).copied() else {
            self.stats.phase = Phase::Waiting;
            return Intent::IDLE;
        };
        let tick = view.newest_applied.unwrap_or_default();
        if now.saturating_duration_since(self.seen_at) >= self.mind.config().timeout {
            self.halt();
            self.stats.phase = Phase::Stale;
            return Intent::IDLE;
        }
        if !catalog_ready(core) {
            self.stats.phase = Phase::Waiting;
            return Intent::IDLE;
        }
        if !self.book.ready() {
            if let Some(rules) = self.rules.as_deref() {
                self.book.learn(rules, &core.catalog);
                self.loadout.learn(rules, &self.book);
            }
        }
        // Bodies come from the tracks alone: seen through the cone and a
        // held line of sight, and placed where they were last seen.
        if let Some(haven) = self.haven {
            self.tracks.perceive(core, &haven, tick);
            let (seed, _) = core.island();
            let (home, builder) = (&self.home, &self.builder);
            self.seen.look(
                core,
                seed,
                &haven,
                eye_point(&body),
                body.yaw,
                tick,
                |cx, cz, l, loc| home.owns(cx, cz, l, loc) || builder.owns(cx, cz, l, loc),
            );
            // Bases worth a raid, and who is about them.
            self.bases
                .look(core, &haven, &body, tick, |cx, cz, l, loc| {
                    home.owns(cx, cz, l, loc) || builder.owns(cx, cz, l, loc)
                });
            self.bases.bodies(&self.tracks, tick);
            // The team's base: a box a teammate was seen standing at, and
            // what stands near it.
            self.shared
                .look(core, &haven, &body, &self.tracks, tick, |cx, cz, l, loc| {
                    home.owns(cx, cz, l, loc) || builder.owns(cx, cz, l, loc)
                });
        }
        self.mind.expire(now);
        if let Some(note) = self.mind.take_note() {
            self.hold_bug(note, core, view, player, &body);
        }
        if let Some((note, facts)) = self.bugs.due(now) {
            eprintln!("{}", crate::bugs::file(&note, facts));
        }
        if body.dead || core.dead {
            // The death screen's answer: one of its own ready bags, unless
            // home is where it was just fought over; else the beach.
            // Asked on the event lane's own fact: a snapshot can still show
            // the corpse for a frame after the wake has landed.
            self.lose(tick, Why::Died);
            self.halt();
            let due = core.dead
                && match self.awaiting {
                    Some((Pending::Respawn, since)) => {
                        tick.wrapping_sub(since) >= VERDICT_SECS * TICK_HZ
                    }
                    _ => true,
                };
            let fell = [body.qx as f32 * POS_XZ_Q, body.qz as f32 * POS_XZ_Q];
            let on_bag = self.home.wake_on_bag(core, tick, fell);
            if due && self.queue(|buf| protocol::encode_action_respawn(on_bag, buf)) {
                self.awaiting = Some((Pending::Respawn, tick));
                self.stats.respawn_asks += 1;
                self.home.woke(on_bag);
            }
            self.stats.phase = Phase::Dead;
            return Intent::IDLE;
        }
        if body.sleeping {
            self.stats.phase = Phase::Sleeping;
            return Intent::IDLE;
        }
        if body.wounded || core.wounded {
            // Down: crawl away from the last blow's bearing while it is
            // fresh, otherwise lie still. Nothing else is possible here.
            self.lose(tick, Why::Wounded);
            self.halt();
            self.combat.down();
            self.stats.phase = Phase::Wounded;
            return self.crawl(core, &body, tick);
        }
        self.crawl_to = None;
        // Answers are taken only by a body that can act on them; one that
        // arrives while it is down waits in the ring and is judged fresh
        // or late when it is read. Mid-fight, or mid-op in a long skill, a
        // routine answer waits for the fight to end or the checkpoint; a
        // newer one, held or adopted, supersedes it.
        let logging_off = self.log_off_due(core, tick);
        if logging_off {
            // Home is where this session ends: nothing held is still owed.
            self.deferred = None;
        }
        if let Some(choice) = self.mind.poll(now) {
            // Speech is not a goal: a fight that holds the goal does not
            // hold the line.
            self.take_say(choice.say);
            // A code a teammate gave is kept whatever becomes of the goal.
            if let Some(code) = choice.code {
                // A new code is a new chance at the cupboard.
                if self.team_code != Some(code) {
                    self.team_bag_failed = None;
                    self.auth_failed = None;
                }
                self.team_code = Some(code);
            }
            let busy = self.combat.engaged() || !self.at_checkpoint();
            if logging_off {
                self.stats.deferred_dropped += 1;
            } else if busy && !urgent(choice.goal, self.memory.alarm) {
                self.deferred = Some((choice, now));
                self.stats.deferred += 1;
            } else {
                self.deferred = None;
                let running = self.combat.engaged();
                self.adopt(choice, tick);
                // Told to run mid-fight: the run is the fight's answer now.
                if running && choice.goal == Goal::Flee {
                    self.start_flee(&body, tick);
                }
            }
        }
        self.perceive(core, &body, tick);
        let x = body.qx as f32 * POS_XZ_Q;
        let z = body.qz as f32 * POS_XZ_Q;
        self.here = [x, z];
        self.frontier.visit(Frontier::cell_of(x, z));
        let (ranged, rounds, loaded) = self.shooter(core);
        let kit = Kit {
            book: &self.book,
            melee: self.loadout.on_belt(core, Role::Melee),
            ranged,
            rounds,
            loaded,
            hp: core.hp,
            hp_max: core.hp_max,
            on_target: self.hands.on_target(),
            settled: self.hands.settled(),
            lead_ticks: u32::from(self.tracks.playout()) + self.ack_age,
            lane_free: self.outbox.is_none(),
            raided: self.home.raided(tick, combat::ALARM_TICKS),
            home_ground: self.builder.region(&body) != Region::Outside,
            town: core.haven().town,
        };
        let assessed = self
            .combat
            .assess(core, &body, &self.tracks, &mut self.route, &kit, tick);
        self.stats.retreats = self.combat.stats.retreats();
        if let Some(verb) = self.combat.take_verb() {
            // The reflex's take: pressed only while the lane is free
            // (`Kit::lane_free`), so it counts as a try.
            let _ = match verb {
                Verb::Loot => self.queue(protocol::encode_action_loot),
                Verb::Pickup => self.queue(|b| protocol::encode_action_pickup(0, b)),
            };
        }
        let me = [
            body.qx as f32 * POS_XZ_Q,
            body.qy as f32 * POS_Y_Q,
            body.qz as f32 * POS_XZ_Q,
        ];
        let calm = combat::safe(&self.tracks, me, tick);
        if self.combat.mode() != Mode::Engage {
            self.medic(core, tick, calm);
        }
        self.reload(core, tick, calm && !self.combat.engaged());
        self.dress(core, tick, calm && !self.combat.engaged());
        match assessed {
            Assess::Fight(intent) => {
                self.pause_goal(tick);
                self.glance = None;
                self.stats.phase = match self.combat.mode() {
                    Mode::Engage => Phase::Fighting,
                    Mode::Loot => Phase::Looting,
                    _ => Phase::Fleeing,
                };
                return intent;
            }
            Assess::Over { away, end } => {
                // We were looking back at the danger. Resume travelling
                // away, rather than turning the retreat into a return trip.
                if let Some(away) = away {
                    self.heading = Some(away);
                }
                match end {
                    End::Won => self.memory.fight = Some(Why::Won),
                    End::Escaped => self.memory.fight = Some(Why::Escaped),
                    End::Parted => {}
                }
                self.after_fight(core, view, player, tick, now);
            }
            Assess::Calm => {
                let held = self.deferred.is_some() && self.at_checkpoint();
                if held || self.goal.is_some_and(|a| a.paused.is_some()) {
                    self.after_fight(core, view, player, tick, now);
                }
            }
        }
        if logging_off {
            let intent = self.log_off(core, &body, tick);
            return self.overlay(intent, tick);
        }
        // Its team changed while a decision is out (an invite came): the
        // mind's asker reads it on that decision, not the next.
        if self.memory.team_wake && self.mind.refreshes() {
            self.refresh(core, view, player);
        }
        let intent = match self.goal {
            None => {
                // Look before choosing: the first sweep of the sight window
                // after appearing takes under three seconds.
                if !self.sensed {
                    self.stats.phase = Phase::Waiting;
                } else {
                    self.request(core, view, player, tick, now);
                    self.stats.phase = if self.mind.mode(now) == crate::mind::Mode::Paused {
                        Phase::Paused
                    } else {
                        Phase::Deciding
                    };
                }
                Intent::IDLE
            }
            Some(active) => {
                if tick.wrapping_sub(active.asked) >= heartbeat_ticks(&self.mind)
                    && !self.mind.pending()
                {
                    self.memory.trigger = Trigger::Heartbeat;
                    if self.request(core, view, player, tick, now) {
                        if let Some(a) = self.goal.as_mut() {
                            a.asked = tick;
                        }
                    }
                } else if self.memory.chat_wake
                    && matches!(
                        self.mind.kind(),
                        SourceKind::External | SourceKind::Scripted
                    )
                    && !self.mind.pending()
                {
                    // Somebody near spoke: an external source may answer
                    // now rather than at the heartbeat, and the scripted
                    // policy hears its team's orders (`agent::orders`). The
                    // mind's floor, backoff and spend guard still decide
                    // whether it is asked.
                    self.memory.trigger = Trigger::Heard;
                    if self.request(core, view, player, tick, now) {
                        if let Some(a) = self.goal.as_mut() {
                            a.asked = tick;
                        }
                    }
                } else if self.memory.team_wake && !self.mind.pending() {
                    // An invite, or a teammate come or gone: worth a
                    // fresh look now rather than at the heartbeat.
                    self.memory.trigger = Trigger::Team;
                    if self.request(core, view, player, tick, now) {
                        if let Some(a) = self.goal.as_mut() {
                            a.asked = tick;
                        }
                    }
                }
                self.run_goal(core, &body, tick)
            }
        };
        let intent = self.unwedge(intent, &body, tick);
        // A stranger near is kept in view while the goal runs, and no
        // swing goes where the eyes are not.
        let intent = match self.combat.watch(&self.tracks, &self.book, me, tick) {
            Some(look) => Intent {
                look,
                buttons: intent.buttons & !BTN_PRIMARY,
                ..intent
            },
            None => intent,
        };
        self.overlay(intent, tick)
    }

    /// The watchdog (`agent::wedge`): a goal that has been trying to move
    /// the body for half a minute without it leaving a metre's box is stuck,
    /// whatever its skill thinks. It ends, the body backs off with a jump,
    /// and the next choice starts from somewhere new. Alternate trips back
    /// off to alternate sides.
    fn unwedge(&mut self, intent: Intent, body: &EntityState, tick: u32) -> Intent {
        if let Some((at, away)) = self.backoff {
            if tick.wrapping_sub(at) < wedge::BACK_OFF_TICKS {
                return Intent {
                    travel: Some(away),
                    ..Intent::IDLE
                };
            }
            self.backoff = None;
        }
        if !self.goal.is_some_and(|a| a.paused.is_none()) {
            return intent;
        }
        let trying = intent.travel.is_some() || intent.move_x != 0 || intent.move_z != 0;
        let at = [body.qx as f32 * POS_XZ_Q, body.qz as f32 * POS_XZ_Q];
        if !self.wedge.observe(tick, at, trying) {
            return intent;
        }
        self.stats.wedges += 1;
        self.end_goal(tick, Outcome::Failed(Why::Stuck));
        self.route.reset();
        let side = if self.stats.wedges.is_multiple_of(2) {
            0x6000
        } else {
            0xA000
        };
        let away = self.hands.view().0.wrapping_add(side);
        self.backoff = Some((tick, away));
        Intent {
            travel: Some(away),
            buttons: sim_core::input::BTN_JUMP,
            ..Intent::IDLE
        }
    }

    /// Down or dead: the goal ends there, and a fight in hand was lost.
    fn lose(&mut self, tick: u32, why: Why) {
        let fighting =
            self.combat.engaged() || matches!(self.goal(), Some(Goal::Fight | Goal::Hunt));
        if fighting {
            self.memory.fight = Some(Why::Lost);
        }
        let outcome = match self.goal() {
            Some(Goal::Fight | Goal::Hunt) => Outcome::Failed(Why::Lost),
            _ => Outcome::Interrupted(why),
        };
        self.end_goal(tick, outcome);
    }

    /// A fight took this frame: the goal waits, where it was.
    fn pause_goal(&mut self, tick: u32) {
        self.wedge.reset();
        if let Some(a) = self.goal.as_mut() {
            if a.paused.is_none() {
                a.paused = Some(tick);
            }
        }
        self.recovery = None;
    }

    /// The fight is over. A short one leaves the goal to carry on, if it is
    /// still on offer; a long one has changed the situation enough that
    /// the mind should choose again. An answer held through the fight is
    /// judged the way the mind judges its own: adopted over the paused goal
    /// only if it is still fresh, the fight was short and it is on offer.
    fn after_fight(
        &mut self,
        core: &ClientCore,
        view: &ClientView,
        player: u32,
        tick: u32,
        now: Instant,
    ) {
        let paused = self.goal.and_then(|a| a.paused.map(|at| (a.goal, at)));
        let fought = paused.map(|(_, at)| tick.wrapping_sub(at));
        let held = self.deferred.take().and_then(|(choice, since)| {
            let fresh = now.saturating_duration_since(since) < self.mind.config().timeout;
            let short = fought.is_none_or(|f| f < RESUME_TICKS);
            let keep = fresh && short && self.summarize(core, view, player).offers(choice.goal);
            if !keep {
                self.stats.deferred_dropped += 1;
            }
            keep.then_some(choice)
        });
        if let (Some((goal, _)), Some(fought)) = (paused, fought) {
            if let Some(a) = self.goal.as_mut() {
                a.paused = None;
                // The goal's own clocks do not count the fight.
                a.started = a.started.wrapping_add(fought);
            }
            if let Some((pending, since)) = self.awaiting {
                if pending != Pending::Respawn {
                    self.awaiting = Some((pending, since.wrapping_add(fought)));
                }
            }
            self.progress_tick = tick;
            self.best_distance = f32::INFINITY;
            // A held answer naming another goal replaces this one below;
            // one naming this goal is this goal carrying on.
            if held.is_none_or(|c| c.goal == goal) {
                // A flee, fight or hunt goal's fight was the goal; it
                // finishes itself with how it went.
                let keep = matches!(goal, Goal::Flee | Goal::Fight | Goal::Hunt)
                    || (fought < RESUME_TICKS && self.summarize(core, view, player).offers(goal));
                if keep {
                    self.stats.goals_resumed += 1;
                } else {
                    self.end_goal(tick, Outcome::Interrupted(Why::Fight));
                    self.memory.trigger = Trigger::Fought;
                }
            }
        }
        if let Some(choice) = held {
            self.adopt(choice, tick);
        }
    }

    /// What runs over an idle frame: a body standing at a bench or waiting
    /// on an answer still looks about, the way a person does, which is
    /// also how its eyes find what is beside and behind it.
    fn overlay(&mut self, intent: Intent, tick: u32) -> Intent {
        let idle = intent.look == Look::Keep
            && intent.travel.is_none()
            && intent.move_x == 0
            && intent.move_z == 0
            && intent.buttons == 0;
        if !idle {
            self.glance = None;
            return intent;
        }
        let (from, step) = match self.glance {
            Some((at, from, step)) if tick.wrapping_sub(at) < GLANCE_TICKS => (from, step),
            Some((_, from, step)) => {
                let step = (step + 1) % GLANCE_OFFSETS.len() as u8;
                self.glance = Some((tick, from, step));
                (from, step)
            }
            None => {
                let from = self.hands.view().0;
                self.glance = Some((tick, from, 0));
                (from, 0)
            }
        };
        Intent {
            look: Look::Heading(from.wrapping_add(GLANCE_OFFSETS[usize::from(step)])),
            ..intent
        }
    }

    /// Ask for a goal. True when a request went out.
    fn request(
        &mut self,
        core: &ClientCore,
        view: &ClientView,
        player: u32,
        tick: u32,
        now: Instant,
    ) -> bool {
        let summary = self.summarize(core, view, player);
        if summary.options_len == 0 || !self.mind.ask(now, &summary) {
            return false;
        }
        self.asked_heard = summary.heard;
        self.asked_hits = summary.hits;
        self.memory.hits = 0;
        self.memory.chat.clear();
        self.memory.chat_wake = false;
        self.memory.team_wake = false;
        if let Some(a) = self.goal.as_mut() {
            a.asked = tick;
        }
        true
    }

    /// A newer look at the decision out, for a mind that shows it to its
    /// asker (`Mind::refresh`): its team changed while the mind thinks. An
    /// agent that takes a minute over a decision reads the invite on the
    /// one it is reading, not on the next, by when the inviter may have
    /// given up. What it had heard and the hits it had taken stay on it
    /// beside what came since, and are not told again on the next. One that
    /// does not go is tried again next frame.
    fn refresh(&mut self, core: &ClientCore, view: &ClientView, player: u32) {
        self.memory.trigger = Trigger::Team;
        let mut s = self.summarize(core, view, player);
        let mut heard = self.asked_heard;
        for &line in s.heard.lines() {
            heard.push(line);
        }
        s.heard = heard;
        s.hits = s.hits.saturating_add(self.asked_hits);
        if s.options_len == 0 || !self.mind.refresh(&s) {
            return;
        }
        self.asked_heard = s.heard;
        self.asked_hits = s.hits;
        self.memory.hits = 0;
        self.memory.chat.clear();
        self.memory.chat_wake = false;
        self.memory.team_wake = false;
    }

    /// An answer's line, kept for [`BotDriver::say`]. An external source
    /// speaks, and the scripted policy answers its team's orders; Jev
    /// never does. A newer line replaces one not yet sent.
    fn take_say(&mut self, say: Say) {
        if !matches!(
            self.mind.kind(),
            SourceKind::External | SourceKind::Scripted
        ) {
            return;
        }
        match say {
            Say::Quiet => {}
            Say::Refused => self.stats.say_refused += 1,
            Say::Line(line) | Say::Team(line) | Say::Global(line) => {
                self.say_to = match say {
                    Say::Team(_) => protocol::Channel::Team,
                    Say::Global(_) => protocol::Channel::Global,
                    _ => protocol::Channel::Local,
                };
                if self.say_line.replace(line).is_some() {
                    self.stats.say_dropped += 1;
                }
            }
        }
    }

    fn adopt(&mut self, choice: Choice, tick: u32) {
        if let Some(active) = self.goal {
            if active.goal == choice.goal {
                return;
            }
            self.end_goal(tick, Outcome::Interrupted(Why::Replaced));
        }
        self.begin(choice.goal, tick);
    }

    /// Start a goal from scratch.
    fn begin(&mut self, goal: Goal, tick: u32) {
        self.wedge.reset();
        if self.medic {
            // The reflex's bandage still in flight answers to no goal: its
            // toast is not the new goal's to count.
            self.medic = false;
            if matches!(self.awaiting, Some((Pending::Consume { .. }, _))) {
                self.awaiting = None;
            }
        }
        self.goal = Some(Active {
            goal,
            started: tick,
            leg: tick,
            delivering: false,
            asked: tick,
            gained: 0,
            abandons: 0,
            craft: CraftStep::Start,
            fled: false,
            paused: None,
        });
        self.target = None;
        self.recovery = None;
        self.seek = None;
        self.verdict = None;
        if matches!(goal, Goal::HandOver | Goal::Give(_)) {
            self.hand = Hand::default();
        }
        // A give goes to whoever was in hand reach when it was chosen.
        if matches!(goal, Goal::Give(_)) {
            self.hand_target = self.give_to.unwrap_or(0);
        }
        self.auth = Auth::default();
        self.bag_job = BagJob::default();
        self.bag_leg = BagLeg::Here;
        self.bag_knocked = false;
        self.recover_job = RecoverJob::default();
        self.stash_job = StashJob::default();
        self.loot_job = LootJob::default();
        self.home_job = HomeJob::new(match goal {
            Goal::GoHome | Goal::Defend => Trip::Stay,
            _ => Trip::Stock,
        });
        // Living in the team's base, its bag goes down in there: in to the
        // cupboard (or the first box, if it knows none), then the floor.
        if goal == Goal::Bag && self.memory.team_bag {
            if let Some(home) = self.shared.anchor() {
                self.bag_leg = BagLeg::In;
                self.bag_job = BagJob::on_floor(home);
                self.home_job = HomeJob::new(match self.shared.hearth() {
                    Some(h) => {
                        let (x, z) = cell_center(h.cx, h.cz);
                        Trip::Point([x, z])
                    }
                    None => Trip::Stay,
                });
            }
        }
        // An authorize walks in through the team's doors to its cupboard.
        if goal == Goal::Authorize {
            if let Some(h) = self.shared.hearth() {
                let (x, z) = cell_center(h.cx, h.cz);
                self.home_job = HomeJob::new(Trip::Point([x, z]));
            }
        }
        self.leave.doors.clear();
        self.trade_job.reset();
        self.work_job = WorkJob::default();
        self.read_job.reset();
        self.solve_job.reset();
        self.swipe_job.reset();
        self.paper_read = false;
        self.sample_in_table = None;
        self.oven_job = DeviceJob::new(match goal {
            Goal::Recycle => Work::Recycle,
            Goal::KeepShop => Work::Shop,
            _ => Work::Cook,
        });
        if goal == Goal::Loot {
            self.loot.stats.runs += 1;
        }
        // A defence or raid cut short is over all the same: what the one
        // gave up on is let be, and the other's base is let be a while
        // rather than priced again from a face already blown.
        self.defend_let_be.remember(&self.defend_job, tick);
        self.defend_job = DefendJob::new(self.defend_let_be);
        self.raid_let_be(tick);
        if goal == Goal::Defend {
            self.defend_stats.defences += 1;
            if self.memory.alarm {
                self.defend_stats.alarmed += 1;
            }
        }
        self.builder.halt();
    }

    fn end_goal(&mut self, tick: u32, outcome: Outcome) {
        self.wedge.reset();
        let Some(active) = self.goal.take() else {
            return;
        };
        let report = Report {
            goal: active.goal,
            outcome,
            gained: active.gained,
            secs: tick.wrapping_sub(active.started) / TICK_HZ,
        };
        self.memory.last = Some(report);
        self.history.push(report);
        if active.goal == Goal::Research {
            self.research_left = match outcome {
                Outcome::Done => None,
                _ => self.sample_in_table.take(),
            };
            // Not offered again before the hold is read, a second from now.
            if matches!(outcome, Outcome::Failed(_)) {
                self.memory.research = false;
            }
        }
        if active.goal == Goal::GoHome && matches!(outcome, Outcome::Failed(_)) {
            self.home_failed = Some(tick);
        }
        if let (Goal::Go(place), Outcome::Failed(_)) = (active.goal, outcome) {
            self.go_failed = Some((place, tick));
            self.memory.go_held = Some(place);
        }
        if matches!(active.goal, Goal::GoHome | Goal::Stash)
            && outcome == Outcome::Failed(Why::Stuck)
        {
            self.home_stuck = Some((tick, self.here));
        }
        // A build that failed the moment it began would fail the same way
        // the next second; one that ran out of materials partway goes for
        // more and is back when it has them.
        if active.goal == Goal::Build
            && (outcome == Outcome::Failed(Why::Stuck)
                || matches!(outcome, Outcome::Failed(_)) && report.secs == 0)
        {
            self.build_failed = Some(tick);
        }
        // Any other goal failing as it began, again, sits out a few seconds;
        // once is often the moment changing under it (a pack just filled).
        // Not flight or a fight: what they answer is still there.
        let same = (0..FLOPS).find(|&i| self.flops[i].is_some_and(|(g, ..)| g == active.goal));
        if matches!(outcome, Outcome::Failed(_))
            && report.secs == 0
            && !matches!(active.goal, Goal::Flee | Goal::Fight)
        {
            let runs = same.and_then(|i| self.flops[i]).map_or(0, |(.., n)| n);
            let oldest = (0..FLOPS).min_by_key(|&i| self.flops[i].map_or(0, |(_, at, _)| at));
            if let Some(i) = same.or(oldest) {
                self.flops[i] = Some((active.goal, tick, runs.saturating_add(1)));
            }
        } else if let Some(i) = same {
            self.flops[i] = None;
        }
        self.memory.trigger = match outcome {
            Outcome::Done | Outcome::Running => {
                self.stats.goals_done += 1;
                Trigger::Completed
            }
            Outcome::Failed(_) => {
                self.stats.goals_failed += 1;
                Trigger::Failed
            }
            Outcome::Interrupted(_) => {
                self.stats.goals_interrupted += 1;
                Trigger::Interrupted
            }
        };
        self.target = None;
        self.recovery = None;
        self.builder.halt();
        if !matches!(self.awaiting, Some((Pending::Respawn, _))) {
            self.awaiting = None;
            // An action still waiting on its pace was this goal's; the
            // next goal asks for its own.
            self.outbox = None;
        }
        self.verdict = None;
    }

    /// Hold one encoded action for the next tick's action slot.
    fn queue(&mut self, encode: impl FnOnce(&mut [u8]) -> Result<usize, WireError>) -> bool {
        if self.outbox.is_some() {
            return false;
        }
        let mut buf = [0u8; MAX_STREAM_MSG_BYTES];
        match encode(&mut buf) {
            Ok(len) => {
                self.outbox = Some((buf, len));
                true
            }
            Err(_) => {
                self.stats.unencodable += 1;
                false
            }
        }
    }

    fn run_goal(&mut self, core: &mut ClientCore, body: &EntityState, tick: u32) -> Intent {
        let Some(active) = self.goal else {
            return Intent::IDLE;
        };
        let elapsed = tick.wrapping_sub(active.started);
        // A goal that walks the island starts from outside: out through the
        // doors first, shutting them behind. A fight is out there when its
        // quarry is, or when none is in sight from in here; one in the room
        // with me is fought where it stands, doors shut.
        let walks = match active.goal {
            Goal::Explore
            | Goal::GatherWood
            | Goal::GatherStone
            | Goal::GatherOre
            | Goal::Forage
            | Goal::GatherHemp
            | Goal::Drink
            | Goal::Recover
            | Goal::Bag
            | Goal::Loot
            | Goal::Cook
            | Goal::Recycle
            | Goal::Research
            | Goal::KeepShop
            | Goal::Raid
            | Goal::Follow
            | Goal::Meet
            | Goal::Patrol
            | Goal::HandOver
            | Goal::Deliver
            | Goal::Authorize
            | Goal::SupplyWood
            | Goal::SupplyStone
            | Goal::SupplyOre
            | Goal::SupplyHemp
            | Goal::Buy(_)
            | Goal::Sell(_)
            | Goal::FeedWork
            | Goal::ReadStone
            | Goal::SolveLock
            | Goal::SwipeCard
            | Goal::Go(_)
            | Goal::Walk(_) => true,
            Goal::Fight | Goal::Hunt => self
                .quarry(body, active.goal == Goal::Hunt, tick)
                .is_none_or(|(_, [x, _, z])| self.builder.region_of([x, z]) == Region::Outside),
            _ => false,
        };
        // Up on a base's floor: down the stairs first (`agent::floors`), so
        // the doors and the walk out start from the ground they were made
        // for. A follow and a delivery go where their own legs take them.
        if walks
            && !matches!(active.goal, Goal::Follow | Goal::Deliver)
            && !active.delivering
            && (self.route.descending() || self.route.upstairs(core, body))
        {
            match self.route.down(core, body, tick) {
                step @ Step::Walk { .. } => {
                    self.stats.phase = Phase::Leaving;
                    return step.walk().unwrap_or(Intent::IDLE);
                }
                Step::Wait => return Intent::IDLE,
                Step::Arrived | Step::Blocked => {}
            }
        }
        if walks && (self.builder.passing() || self.builder.must_exit(core, body)) {
            if let Some(intent) = self.leave(core, body, tick) {
                return intent;
            }
        }
        // Out of a teammate's base too, by its nearest door. A delivery and
        // an authorize work its own doors, and a follow goes where its
        // teammate goes.
        let own_doors = matches!(active.goal, Goal::Follow | Goal::Deliver | Goal::Authorize)
            || active.delivering
            || self.bag_leg != BagLeg::Here;
        if walks && !own_doors && self.builder.region(body) == Region::Outside {
            if let Some(intent) = self.leave_mates(core, body, tick) {
                return intent;
            }
        }
        // Out of a ziggurat card room first, by its lever if the door has
        // shut, and down off the terraces. A loot run takes the room's
        // crates where it stands; a swipe under way walks its own way, and
        // one starting up on its own door's ledge starts from there.
        let swipe = active.goal == Goal::SwipeCard;
        if walks && active.goal != Goal::Loot && !(swipe && self.swipe_job.under_way()) {
            let (x, y, z) = (
                body.qx as f32 * POS_XZ_Q,
                body.qy as f32 * POS_Y_Q,
                body.qz as f32 * POS_XZ_Q,
            );
            let climbing = swipe
                && lore::swipeable(core)
                    .is_some_and(|(d, _)| lore::on_path(&core.haven().ziggurat, d, x, y, z));
            if let Some(intent) = self.exit_room(core, body, !climbing, tick) {
                return intent;
            }
        }
        match active.goal {
            Goal::Explore => {
                if elapsed >= EXPLORE_GOAL_SECS * TICK_HZ {
                    self.end_goal(tick, Outcome::Done);
                    return Intent::IDLE;
                }
                self.stats.phase = Phase::Exploring;
                self.wander(core, body, tick)
            }
            Goal::Wait => {
                if elapsed >= WAIT_GOAL_SECS * TICK_HZ {
                    self.end_goal(tick, Outcome::Done);
                }
                self.stats.phase = Phase::Resting;
                Intent::IDLE
            }
            Goal::Flee => {
                if active.fled || !self.start_flee(body, tick) {
                    self.end_goal(tick, Outcome::Done);
                }
                Intent::IDLE
            }
            Goal::Craft(name) => self.craft(core, body, name, tick),
            Goal::Loot => self.loot_run(core, body, tick),
            Goal::Eat => {
                self.eat(core, tick);
                Intent::IDLE
            }
            Goal::Heal => {
                self.heal(core, tick);
                Intent::IDLE
            }
            Goal::Fight | Goal::Hunt => {
                if active.fled {
                    // The fight has been and gone: say how it went.
                    let outcome = match self.combat.last_end() {
                        Some(End::Won) => Outcome::Done,
                        Some(End::Escaped) => Outcome::Failed(Why::Escaped),
                        _ => Outcome::Failed(Why::NotFound),
                    };
                    self.end_goal(tick, outcome);
                } else if !self.start_fight(core, body, active.goal == Goal::Hunt, tick) {
                    self.end_goal(tick, Outcome::Failed(Why::NotFound));
                }
                Intent::IDLE
            }
            Goal::Drink => self.drink(core, body, tick),
            Goal::Bag => self.place_bag(core, body, tick),
            Goal::Recover => self.recover(core, body, tick),
            Goal::Build if elapsed >= BUILD_GOAL_SECS * TICK_HZ => {
                self.end_goal(tick, Outcome::Failed(Why::Stuck));
                Intent::IDLE
            }
            Goal::Build => self.build_home(core, body, tick),
            Goal::GoHome if elapsed >= GO_HOME_GOAL_SECS * TICK_HZ => {
                self.end_goal(tick, Outcome::Failed(Why::Stuck));
                Intent::IDLE
            }
            Goal::GoHome if self.shared_home() => self.go_home_shared(core, body, tick),
            Goal::GoHome => self.go_home(core, body, tick),
            Goal::Stash if elapsed >= STASH_GOAL_SECS * TICK_HZ => {
                self.home.stash_failed(tick);
                self.end_goal(tick, Outcome::Failed(Why::Stuck));
                Intent::IDLE
            }
            Goal::Stash => self.stash(core, body, tick),
            Goal::Cook | Goal::Recycle | Goal::KeepShop if elapsed >= COOK_GOAL_SECS * TICK_HZ => {
                self.oven_failed(tick);
                self.end_goal(tick, Outcome::Failed(Why::Stuck));
                Intent::IDLE
            }
            Goal::Cook | Goal::Recycle | Goal::KeepShop => self.tend(core, body, tick),
            Goal::Research if elapsed >= VISIT_GOAL_SECS * TICK_HZ => {
                self.research_failed = Some(tick);
                self.end_goal(tick, Outcome::Failed(Why::Stuck));
                Intent::IDLE
            }
            Goal::Research => self.research(core, body, tick),
            Goal::Defend if elapsed >= DEFEND_GOAL_SECS * TICK_HZ => {
                self.defend_failed = Some(tick);
                self.end_goal(tick, Outcome::Failed(Why::Stuck));
                Intent::IDLE
            }
            Goal::Defend if self.shared_home() => self.defend_shared(core, body, tick),
            Goal::Defend => self.defend(core, body, tick),
            Goal::Raid if elapsed >= RAID_GOAL_SECS * TICK_HZ => {
                self.raid_over(Outcome::Failed(Why::Stuck), tick);
                Intent::IDLE
            }
            Goal::Raid => self.raid(core, body, tick),
            Goal::JoinTeam => {
                let team = *core.team();
                if team.count > 0 {
                    self.end_goal(tick, Outcome::Done);
                } else if team.invite == 0 && active.fled {
                    // The yes went, and the invite is gone with no team:
                    // the shard said no (its line says why).
                    self.end_goal(tick, Outcome::Failed(Why::Refused));
                } else if team.invite == 0 {
                    self.end_goal(tick, Outcome::Failed(Why::NoInvite));
                } else if elapsed >= JOIN_GOAL_SECS * TICK_HZ {
                    self.end_goal(tick, Outcome::Failed(Why::NoAnswer));
                } else if !active.fled
                    && self.queue(|b| {
                        protocol::encode_action_team(protocol::TEAM_OP_ACCEPT, team.invite, b)
                    })
                {
                    // `fled` is the one-shot half done: the yes is sent,
                    // and the goal waits for the team view to say so.
                    if let Some(a) = self.goal.as_mut() {
                        a.fled = true;
                    }
                }
                self.stats.phase = Phase::Waiting;
                Intent::IDLE
            }
            Goal::Invite if elapsed >= JOIN_GOAL_SECS * TICK_HZ => {
                self.end_goal(tick, Outcome::Failed(Why::Stuck));
                Intent::IDLE
            }
            Goal::Invite => {
                // The invite a player sends with N. `fled` is the one-shot
                // half done: once it has gone out the goal is done, and the
                // yes is theirs (the team view says when it comes).
                if active.fled {
                    if self.outbox.is_none() {
                        self.end_goal(tick, Outcome::Done);
                    }
                } else if let Some(target) = self.invitee {
                    if self.queue(|b| {
                        protocol::encode_action_team(protocol::TEAM_OP_INVITE, target, b)
                    }) {
                        if let Some(a) = self.goal.as_mut() {
                            a.fled = true;
                        }
                    }
                } else {
                    self.end_goal(tick, Outcome::Failed(Why::NotFound));
                }
                self.stats.phase = Phase::Waiting;
                Intent::IDLE
            }
            Goal::Follow if elapsed >= FOLLOW_GOAL_SECS * TICK_HZ => {
                self.end_goal(tick, Outcome::Done);
                Intent::IDLE
            }
            Goal::Follow => self.follow(core, body, tick),
            Goal::Meet if elapsed >= MEET_GOAL_SECS * TICK_HZ => {
                self.end_goal(tick, Outcome::Done);
                Intent::IDLE
            }
            Goal::Meet => self.meet(core, body, tick),
            Goal::Go(_) if elapsed >= GO_GOAL_SECS * TICK_HZ => {
                self.end_goal(tick, Outcome::Failed(Why::Stuck));
                Intent::IDLE
            }
            Goal::Go(place) => self.travel(core, body, place, tick),
            Goal::Walk(_) if elapsed >= GO_GOAL_SECS * TICK_HZ => {
                self.end_goal(tick, Outcome::Failed(Why::Stuck));
                Intent::IDLE
            }
            Goal::Walk(leg) => self.walk_leg(core, body, leg, active.started, tick),
            Goal::Patrol if elapsed >= society::PATROL_GOAL_SECS * TICK_HZ => {
                self.end_goal(tick, Outcome::Done);
                Intent::IDLE
            }
            Goal::Patrol => self.patrol(core, body, tick),
            Goal::HandOver if elapsed >= society::HAND_GOAL_SECS * TICK_HZ => {
                self.hand_failed = Some(tick);
                self.end_goal(tick, Outcome::Failed(Why::Stuck));
                Intent::IDLE
            }
            Goal::HandOver => self.hand_over(core, body, tick),
            Goal::Give(_) if elapsed >= GIVE_GOAL_SECS * TICK_HZ => {
                self.end_goal(tick, Outcome::Failed(Why::Stuck));
                Intent::IDLE
            }
            Goal::Give(name) => self.give(core, body, name, tick),
            Goal::Authorize if elapsed >= AUTHORIZE_GOAL_SECS * TICK_HZ => {
                self.auth_failed = Some(tick);
                self.home_job = HomeJob::default();
                self.end_goal(tick, Outcome::Failed(Why::Stuck));
                Intent::IDLE
            }
            Goal::Authorize => self.authorize(core, body, tick),
            Goal::Deliver if elapsed >= DELIVER_GOAL_SECS * TICK_HZ => {
                self.deliver_over(tick, Outcome::Failed(Why::Stuck));
                Intent::IDLE
            }
            Goal::Deliver => self.deliver(core, body, tick),
            Goal::SupplyWood | Goal::SupplyStone | Goal::SupplyOre | Goal::SupplyHemp => {
                self.supply(core, body, tick)
            }
            Goal::Buy(_) | Goal::Sell(_) | Goal::FeedWork
                if elapsed >= VISIT_GOAL_SECS * TICK_HZ =>
            {
                self.visit_over(active.goal, Outcome::Failed(Why::Stuck), tick);
                Intent::IDLE
            }
            Goal::Buy(name) => self.trade(core, body, name, false, tick),
            Goal::Sell(name) => self.trade(core, body, name, true, tick),
            Goal::FeedWork => self.feed_work(core, body, tick),
            Goal::ReadStone | Goal::SolveLock | Goal::SwipeCard
                if elapsed >= VISIT_GOAL_SECS * TICK_HZ =>
            {
                self.lore_over(active.goal, Outcome::Failed(Why::Stuck), tick);
                Intent::IDLE
            }
            Goal::ReadStone | Goal::SolveLock | Goal::SwipeCard => {
                self.lore_visit(core, body, active.goal, tick)
            }
            goal => match Kind::of_goal(goal) {
                Some(kind) => self.gather(core, body, tick, kind),
                None => Intent::IDLE,
            },
        }
    }

    /// A trip (`Goal::Go`): to the named place by the route, done once
    /// within [`GO_STOP_M`] of it. A place it no longer knows is not found;
    /// a way the route cannot find is stuck.
    fn travel(
        &mut self,
        core: &mut ClientCore,
        body: &EntityState,
        place: Name,
        tick: u32,
    ) -> Intent {
        let Some(to) = gate::place(core, place.as_str()) else {
            self.end_goal(tick, Outcome::Failed(Why::NotFound));
            return Intent::IDLE;
        };
        match gate::walk(core, body, &mut self.route, to, GO_STOP_M, tick) {
            Some(Ok(intent)) => {
                self.stats.phase = Phase::Approaching;
                intent
            }
            Some(Err(why)) => {
                self.end_goal(tick, Outcome::Failed(why));
                Intent::IDLE
            }
            None => {
                self.end_goal(tick, Outcome::Done);
                Intent::IDLE
            }
        }
    }

    /// A walk anywhere (`Goal::Walk`): the point `leg` names from where the
    /// body stood and faced when it began, kept inside the island's border,
    /// walked to by the route (the long way round included, `agent::far`)
    /// and done within [`WALK_STOP_M`] of it.
    fn walk_leg(
        &mut self,
        core: &mut ClientCore,
        body: &EntityState,
        leg: crate::mind::Walk,
        started: u32,
        tick: u32,
    ) -> Intent {
        let to = match self.walk_to {
            Some((at, to)) if at == started => to,
            _ => {
                let to = walk_target(body, leg);
                self.walk_to = Some((started, to));
                to
            }
        };
        match gate::walk(core, body, &mut self.route, to, WALK_STOP_M, tick) {
            Some(Ok(intent)) => {
                self.stats.phase = Phase::Approaching;
                intent
            }
            Some(Err(why)) => {
                self.end_goal(tick, Outcome::Failed(why));
                Intent::IDLE
            }
            None => {
                self.end_goal(tick, Outcome::Done);
                Intent::IDLE
            }
        }
    }

    /// Meet (`--friend`): to THE GATE, and wait there until its friend is
    /// in reach of an invite or on its team; then it is done, and the next
    /// decision can invite them. Gone from the island, the meet is over.
    ///
    /// A society's meet (`agent::society`) is its crew coming together
    /// there: the founder waits until a crewmate is in reach of its invite
    /// or the whole crew is on its team; a crewmate walks up to its founder
    /// once it sees them, and is done when the invite comes.
    fn meet(&mut self, core: &mut ClientCore, body: &EntityState, tick: u32) -> Intent {
        if let Some(soc) = self.society {
            let sense = self.senses.society;
            let over = if soc.founder() {
                sense.crew_missing == 0 || sense.crew_reachable
            } else {
                sense.founded || sense.crew_invited_me
            };
            if over {
                self.end_goal(tick, Outcome::Done);
                return Intent::IDLE;
            }
            if !soc.founder() {
                let (x, z) = (body.qx as f32 * POS_XZ_Q, body.qz as f32 * POS_XZ_Q);
                // Its founder: in sight, else where its founder last said
                // it is (a crewmate come late finds the crew at work).
                let founder = self
                    .tracks
                    .get(self.kin.founder())
                    .filter(|t| t.visible && t.active())
                    .map(|t| [t.pos[0], t.pos[2]])
                    .filter(|at| (at[0] - x).hypot(at[1] - z) <= society::GATHER_M)
                    .or_else(|| soc.board.and_then(|b| b.founder(soc.crew_no)));
                if let Some(at) = founder {
                    self.stats.phase = Phase::Approaching;
                    return match self
                        .route
                        .to(core, body, at, society::GATHER_STOP_M, false, tick)
                    {
                        step @ Step::Walk { yaw, .. } => step.walk().unwrap_or(Intent::walk(yaw)),
                        // No plan yet this frame: straight at them meanwhile,
                        // as the walk to THE GATE does.
                        Step::Wait => Intent::walk(yaw_toward(at[0] - x, at[1] - z)),
                        Step::Arrived | Step::Blocked => {
                            let y = body.qy as f32 * POS_Y_Q + tracks::eye_m(false);
                            Intent {
                                look: Look::Point([at[0], y, at[1]]),
                                ..Intent::IDLE
                            }
                        }
                    };
                }
            }
        } else {
            let f = self.senses.friend;
            if !f.online || f.on_team || f.reachable || f.invited_me {
                self.end_goal(tick, Outcome::Done);
                return Intent::IDLE;
            }
        }
        match crate::agent::gate::approach(core, body, &mut self.route, MEET_STOP_M, tick) {
            Some(Ok(intent)) => {
                self.stats.phase = Phase::Approaching;
                intent
            }
            Some(Err(why)) => {
                self.end_goal(tick, Outcome::Failed(why));
                Intent::IDLE
            }
            None => {
                self.stats.phase = Phase::Waiting;
                Intent::IDLE
            }
        }
    }

    /// What it knows of its friend now (`--friend`), for the summary and
    /// the meet. Allocation-free: this runs on the frame path.
    fn friend_sense(
        &self,
        core: &ClientCore,
        team: &protocol::TeamView,
        tick: u32,
    ) -> crate::mind::FriendSense {
        let Some(address) = self.friend else {
            return crate::mind::FriendSense::NONE;
        };
        let id = self.friend_id;
        let tag = (id != 0).then(|| core.tag(id)).flatten();
        let member = team.members().iter().find(|m| id != 0 && m.id == id);
        let recent =
            tick.wrapping_sub(self.friend_since.max(self.friend_seen)) < FRIEND_AWAY_SECS * TICK_HZ;
        crate::mind::FriendSense {
            set: true,
            who: crate::mind::Who {
                id,
                name: tag.map_or(protocol::Name::EMPTY, |t| t.name),
                address,
            },
            online: match member {
                Some(m) => m.state == protocol::TEAM_AWAKE,
                None => id != 0 && recent,
            },
            on_team: member.is_some(),
            in_sight: crate::mind::Sighting::NONE,
            reachable: id != 0 && self.invitee == Some(id),
            invited_me: id != 0 && team.invite == id && team.count == 0,
        }
    }

    /// Its society (`agent::society`) as its eyes and the team view have
    /// it: the nearest crewmate in sight not yet on its team, the walk to
    /// THE GATE that would bring the crew together, who is in the ward at
    /// home, and — for a founder with room on its team — who an invite
    /// goes to: a crewmate in reach first, else an ally. Allocation-free.
    fn sense_society(
        &mut self,
        body: &EntityState,
        team: &protocol::TeamView,
        room: bool,
        tick: u32,
    ) -> Option<u32> {
        let Some(soc) = self.society else {
            self.crew_near = None;
            self.senses.society = crate::mind::SocietySense::NONE;
            self.senses.society_meet = false;
            return None;
        };
        let (bx, bz) = (body.qx as f32 * POS_XZ_Q, body.qz as f32 * POS_XZ_Q);
        let mut crew_near: Option<(u32, [f32; 2], f32)> = None;
        let mut invite: Option<(bool, f32, u32)> = None;
        let mut allies_in_sight = Sighting::default();
        for t in self.tracks.all() {
            if t.species != Species::Player || !t.visible || !t.active() {
                continue;
            }
            let crew = self.kin.is_crew(t.id);
            if !crew && !self.kin.is_ally(t.id) {
                continue;
            }
            let d = (t.pos[0] - bx).hypot(t.pos[2] - bz);
            if !crew {
                let (_, b) = relative(body, t.pos[0], t.pos[2]);
                allies_in_sight.add(d, b);
            }
            if team.has(t.id) || t.wounded {
                continue;
            }
            if crew && d <= society::GATHER_M && crew_near.is_none_or(|c| d < c.2) {
                crew_near = Some((t.id, [t.pos[0], t.pos[2]], d));
            }
            if soc.founder()
                && room
                && d <= INVITE_REACH_M
                && invite.is_none_or(|(c, near, _)| (crew && !c) || (crew == c && d < near))
            {
                invite = Some((crew, d, t.id));
            }
        }
        self.crew_near = crew_near;
        let me = usize::from(soc.me);
        let (mut on_team, mut missing) = (0u8, 0u8);
        for (i, &id) in self.kin.crew[..usize::from(soc.crew_len)]
            .iter()
            .enumerate()
        {
            if i == me || id == 0 {
                continue;
            }
            if team.has(id) {
                on_team += 1;
            } else {
                missing += 1;
            }
        }
        let founded = if soc.founder() {
            on_team > 0
        } else {
            self.kin.founder() != 0 && team.has(self.kin.founder())
        };
        let invited_me = team.count == 0 && team.invite != 0 && self.kin.is_crew(team.invite);
        let founding = soc.founder()
            && self
                .welcomed
                .is_some_and(|w| tick.wrapping_sub(w) < society::FOUNDING_SECS * TICK_HZ);
        let mut near = Sighting::default();
        if let Some((_, [x, z], _)) = crew_near {
            let (d, b) = relative(body, x, z);
            near.add(d, b);
        }
        let mut intruders = Sighting::default();
        if let Some(centre) = self.patrol_centre() {
            for t in self.tracks.seen() {
                if society::intruder(t, centre, self.armed_track(t), tick) {
                    let (d, b) = relative(body, t.pos[0], t.pos[2]);
                    intruders.add(d, b);
                }
            }
        }
        self.senses.society = crate::mind::SocietySense {
            set: true,
            role: soc.role,
            crew: soc.crew_len.saturating_sub(1),
            crew_on_team: on_team,
            crew_missing: missing,
            crew_near: near,
            crew_reachable: invite.is_some_and(|(crew, ..)| crew),
            crew_invited_me: invited_me,
            founded,
            founding,
            allies: soc.allies_len,
            allies_in_sight,
            intruders,
            shop: self.memory.shop_stands,
        };
        // The walk to THE GATE: a founder while some of its crew is not on
        // its team and it is still founding (later a crewmate walks to
        // where it says it is), a crewmate until its founder's invite comes.
        self.senses.society_meet = if soc.founder() {
            missing > 0 && founding
        } else {
            !founded && !invited_me
        };
        invite.map(|(_, _, id)| id)
    }

    /// What a body in sight is holding, as a guard sizes up a visitor: a
    /// weapon (melee, a bow or a gun, a satchel), not a tool.
    fn armed_track(&self, t: &crate::agent::tracks::Track) -> bool {
        matches!(
            t.held.map(|h| self.book.page(h).class),
            Some(Class::Melee | Class::Ranged | Class::Throw)
        )
    }

    /// Home's middle, for a patrol and its ward: the team's base it lives
    /// in, else its own once a cupboard stands there.
    fn patrol_centre(&self) -> Option<[f32; 2]> {
        if self.shared_home() {
            return self.shared.anchor();
        }
        self.builder
            .stand()
            .filter(|_| self.builder.survey().hearth)
    }

    /// The nearest intruder in the ward round `centre` the fight reflex has
    /// not just let go (`society::intruder`).
    fn intruder(&self, body: &EntityState, centre: [f32; 2], tick: u32) -> Option<u32> {
        let (bx, bz) = (body.qx as f32 * POS_XZ_Q, body.qz as f32 * POS_XZ_Q);
        let mut nearest: Option<(f32, u32)> = None;
        for t in self.tracks.seen() {
            if !society::intruder(t, centre, self.armed_track(t), tick)
                || self.combat.shuns(t.id, tick)
            {
                continue;
            }
            let d = (t.pos[0] - bx).hypot(t.pos[2] - bz);
            if nearest.is_none_or(|n| d < n.0) {
                nearest = Some((d, t.id));
            }
        }
        nearest.map(|(_, id)| id)
    }

    /// The guild's board (`society::Board`), once a second. Every member
    /// says which body is its own; a founder says where it is and which box
    /// is its crew's; the senior founder says
    /// where its base stands, and every other founder chooses its plot a
    /// step from there, so the guild's bases stand together. A crewmate on
    /// its founder's team takes its word for which box is home.
    fn post_board(&mut self, core: &ClientCore, body: &EntityState) {
        let Some(soc) = self.society else {
            return;
        };
        let Some(board) = soc.board else {
            return;
        };
        board.set_member(soc.post(), core.player_id);
        if soc.founder() {
            board.set_founder(
                soc.crew_no,
                [body.qx as f32 * POS_XZ_Q, body.qz as f32 * POS_XZ_Q],
            );
            board.set_chest(soc.crew_no, self.builder.box_addr(core));
            if soc.senior() {
                if let Some(at) = self.builder.stand() {
                    board.set_seat(at);
                }
            } else {
                self.builder.set_seat(
                    board
                        .seat()
                        .map(|seat| society::seat_for(soc.crew_no, seat)),
                );
            }
        } else if self.senses.society.founded {
            if let Some(at) = board.chest(soc.crew_no) {
                self.shared.tell(core, at);
            }
        }
    }

    /// Units of what a crewmate hands its founder that the pack carries.
    fn hand_units(core: &ClientCore) -> u32 {
        society::HAND_ITEMS
            .iter()
            .filter_map(|name| crate::agent::build::item_named(core, name))
            .map(|item| count_item(core, item))
            .sum()
    }

    /// Where its founder is, and whether it is in sight: in sight, else
    /// where its eyes last had them a moment ago, else where the founder
    /// says it is.
    fn founder_at(&self, tick: u32) -> Option<([f32; 3], bool)> {
        let soc = self.society?;
        if let Some(t) = self.tracks.get(self.kin.founder()).filter(|t| {
            t.active() && (t.visible || tick.wrapping_sub(t.last_seen) < FOLLOW_SEEN_TICKS)
        }) {
            return Some((t.pos, t.visible));
        }
        let [x, z] = soc.board?.founder(soc.crew_no)?;
        Some(([x, 0.0, z], false))
    }

    /// A hand-over is worth a walk: a crewmate with a founder (not itself)
    /// on its team or about to be, near or saying where it is, the pack
    /// carrying [`society::HAND_AT`] of what the base takes, and no
    /// hand-over came to nothing a moment ago.
    fn hand_due(&self, core: &ClientCore, body: &EntityState, tick: u32) -> bool {
        if self.society.is_none_or(|s| s.founder()) {
            return false;
        }
        if !self.senses.society.founded
            || self
                .hand_failed
                .is_some_and(|at| tick.wrapping_sub(at) < society::HAND_RETRY_SECS * TICK_HZ)
            || Self::hand_units(core) < society::HAND_AT
        {
            return false;
        }
        let (x, z) = (body.qx as f32 * POS_XZ_Q, body.qz as f32 * POS_XZ_Q);
        self.founder_at(tick).is_some_and(|(p, _)| {
            (p[0] - x).hypot(p[2] - z) <= society::HAND_WALK_M
                && !doors::walled_between(core, [x, z], [p[0], p[2]])
        })
    }

    /// Hand over (`agent::society`): the base's raw stuff to its founder.
    fn hand_over(&mut self, core: &mut ClientCore, body: &EntityState, tick: u32) -> Intent {
        let founder = self.kin.founder();
        let to = self
            .founder_at(tick)
            .filter(|_| founder != 0)
            .map(|(at, seen)| (founder, at, seen));
        self.hand_to(core, body, to, Handing::Base, tick)
    }

    /// A give (`Goal::Give`): every stack of the named item the pack
    /// carries, to the friend or teammate who stood in hand reach when it
    /// was chosen.
    fn give(&mut self, core: &mut ClientCore, body: &EntityState, name: Name, tick: u32) -> Intent {
        let Some(item) = crate::agent::build::item_named(core, name.as_str()) else {
            self.end_goal(tick, Outcome::Failed(Why::MissingInputs));
            return Intent::IDLE;
        };
        let id = self.hand_target;
        let to = (id != 0)
            .then(|| self.mate_at(id, tick))
            .flatten()
            .map(|(at, seen)| (id, at, seen));
        self.hand_to(core, body, to, Handing::Item(item), tick)
    }

    /// Where a player it hands to stands, by its own eyes: in sight, else
    /// where they were a moment ago. Down, dead or asleep, nobody takes a
    /// give (the sim's give refuses them).
    fn mate_at(&self, id: u32, tick: u32) -> Option<([f32; 3], bool)> {
        self.tracks
            .get(id)
            .filter(|t| {
                t.active()
                    && !t.dead
                    && !t.wounded
                    && !t.sleeping
                    && (t.visible || tick.wrapping_sub(t.last_seen) < FOLLOW_SEEN_TICKS)
            })
            .map(|t| (t.pos, t.visible))
    }

    /// Hand to `to` (who, where, and whether its eyes have them): up to
    /// them, eyes on them at arm's length, and what `what` names out of the
    /// pack into theirs a stack at a time — the give a player makes with a
    /// stack and a look (`encode_action_give`). What left the
    /// slot says a give landed; a pack with no room says so, and the
    /// hand-over is over. A hand-over to its founder that came to nothing
    /// is not offered again for a while (`hand_failed`).
    fn hand_to(
        &mut self,
        core: &mut ClientCore,
        body: &EntityState,
        to: Option<(u32, [f32; 3], bool)>,
        what: Handing,
        tick: u32,
    ) -> Intent {
        let (x, z) = (body.qx as f32 * POS_XZ_Q, body.qz as f32 * POS_XZ_Q);
        let base = what == Handing::Base;
        let Some((id, at, seen)) = to else {
            if base {
                self.hand_failed = Some(tick);
            }
            self.end_goal(tick, Outcome::Failed(Why::NotFound));
            return Intent::IDLE;
        };
        // A give in flight.
        if let Some((slot, before, sent, seq)) = self.hand.sent {
            let now = core.inv[usize::from(slot)].count;
            if now < before {
                self.hand.sent = None;
                self.hand.tries = 0;
                self.hand.given += u32::from(before - now);
                if let Some(a) = self.goal.as_mut() {
                    a.gained = a.gained.saturating_add(u32::from(before - now));
                }
            } else if core.move_seq != seq
                && u32::from(core.last_move_refused) == sim_core::inventory::REFUSE_M_GIVE
            {
                // Their pack is full: what was handed is theirs, the rest
                // keeps.
                self.hand.sent = None;
                let outcome = if self.hand.given > 0 {
                    Outcome::Done
                } else {
                    if base {
                        self.hand_failed = Some(tick);
                    }
                    Outcome::Failed(Why::BoxFull)
                };
                self.end_goal(tick, outcome);
                return Intent::IDLE;
            } else if tick.wrapping_sub(sent) >= VERDICT_SECS * TICK_HZ {
                self.hand.sent = None;
                self.hand.tries += 1;
                if self.hand.tries >= society::HAND_TRIES {
                    if base {
                        self.hand_failed = Some(tick);
                    }
                    self.end_goal(tick, Outcome::Failed(Why::Stuck));
                    return Intent::IDLE;
                }
            }
        }
        // Behind its walls the founder is not to be followed in: a crewmate
        // in the airlock works its doors under the founder's own walk. Nor
        // is anyone a give is for: they were in reach when it was chosen.
        if doors::walled_between(core, [x, z], [at[0], at[2]]) {
            if base {
                self.hand_failed = Some(tick);
            }
            let outcome = if self.hand.given > 0 {
                Outcome::Done
            } else {
                Outcome::Failed(Why::Locked)
            };
            self.end_goal(tick, outcome);
            return Intent::IDLE;
        }
        let d = (at[0] - x).hypot(at[2] - z);
        // Where the founder only says it is, its height is not known: eyes
        // level, to find it.
        let eye_y = if seen || at[1] != 0.0 {
            at[1] + 1.0
        } else {
            body.qy as f32 * POS_Y_Q + tracks::eye_m(false)
        };
        let face = Intent {
            look: Look::Point([at[0], eye_y, at[2]]),
            ..Intent::IDLE
        };
        if d > society::HAND_STAND_M {
            self.stats.phase = Phase::Approaching;
            self.hand.aimed = None;
            let hurry = d > FOLLOW_RUN_M;
            return match self.route.to(
                core,
                body,
                [at[0], at[2]],
                society::HAND_STAND_M - 0.4,
                hurry,
                tick,
            ) {
                step @ Step::Walk { yaw, .. } => step.walk().unwrap_or(Intent::walk(yaw)),
                // No plan yet, close but not close enough, or no way: straight
                // at them, as a player walks up to a friend.
                Step::Wait | Step::Arrived | Step::Blocked => {
                    Intent::walk(yaw_toward(at[0] - x, at[2] - z))
                }
            };
        }
        if !seen && at[1] == 0.0 {
            // Where it was said to be, and not here: looked round for there.
            self.stats.phase = Phase::Approaching;
            return face;
        }
        self.stats.phase = Phase::Delivering;
        let look = face;
        if self.hand.sent.is_some() {
            return look;
        }
        // The next stack, biggest first: of the base's raw stuff, or of the
        // item a give names.
        let mut next: Option<(u8, u16)> = None;
        let mut biggest = |item: u16| {
            for (i, st) in core.inv[..INV_SLOTS].iter().enumerate() {
                if st.count > 0 && st.item == item && next.is_none_or(|(_, c)| st.count > c) {
                    next = Some((i as u8, st.count));
                }
            }
        };
        match what {
            Handing::Base => {
                for name in society::HAND_ITEMS {
                    if let Some(item) = crate::agent::build::item_named(core, name) {
                        biggest(item);
                    }
                }
            }
            Handing::Item(item) => biggest(item),
        }
        let Some((slot, count)) = next else {
            // All of it handed. A give with none of it to hand was not one.
            let outcome = if base || self.hand.given > 0 {
                Outcome::Done
            } else {
                Outcome::Failed(Why::MissingInputs)
            };
            self.end_goal(tick, outcome);
            return Intent::IDLE;
        };
        // The eyes settle on them first, as a player's would.
        let aimed = *self.hand.aimed.get_or_insert(tick);
        if tick.wrapping_sub(aimed) >= home::HOLD_TICKS
            && self.hands.settled()
            && self.queue(|b| protocol::encode_action_give(slot, count, id, b))
        {
            self.hand.sent = Some((slot, count, tick, core.move_seq));
            self.hand.aimed = None;
        }
        look
    }

    /// Patrol (`agent::society`): round the ring of posts about home, a
    /// stand at each looking out, and whoever comes into the ward armed or
    /// fighting is taken on — the fight reflex runs it, and the walk goes
    /// on when it is over. A visitor with empty hands is let be.
    fn patrol(&mut self, core: &mut ClientCore, body: &EntityState, tick: u32) -> Intent {
        let Some(centre) = self.patrol_centre() else {
            self.end_goal(tick, Outcome::Failed(Why::NotFound));
            return Intent::IDLE;
        };
        if let Some(id) = self.intruder(body, centre, tick) {
            if self.loadout.on_belt(core, Role::Melee).is_some() || self.shooter(core).0.is_some() {
                self.combat.command(id, tick);
            }
        }
        self.stats.phase = Phase::Patrolling;
        let (x, z) = (body.qx as f32 * POS_XZ_Q, body.qz as f32 * POS_XZ_Q);
        let k = match self.patrol.post {
            Some(k) => usize::from(k),
            None => {
                let k = society::nearest_post(centre, [x, z]);
                self.patrol.post = Some(k as u8);
                k
            }
        };
        let at = society::post(centre, k);
        if let Some(from) = self.patrol.dwell_from {
            if tick.wrapping_sub(from) >= society::PATROL_DWELL_TICKS {
                self.patrol.next();
                return Intent::IDLE;
            }
            // At the post, eyes out past it: the way somebody comes from.
            let out = [2.0 * at[0] - centre[0], 2.0 * at[1] - centre[1]];
            let y = body.qy as f32 * POS_Y_Q + tracks::eye_m(false);
            return Intent {
                look: Look::Point([out[0], y, out[1]]),
                ..Intent::IDLE
            };
        }
        match self
            .route
            .to(core, body, at, society::PATROL_STOP_M, false, tick)
        {
            step @ Step::Walk { yaw, .. } => {
                self.heading = Some(yaw);
                step.walk().unwrap_or(Intent::walk(yaw))
            }
            Step::Arrived => {
                self.patrol.misses = 0;
                self.patrol.dwell_from = Some(tick);
                Intent::IDLE
            }
            Step::Wait => Intent::IDLE,
            Step::Blocked => {
                self.patrol.misses += 1;
                if usize::from(self.patrol.misses) >= society::PATROL_POSTS.len() {
                    self.patrol_failed = Some(tick);
                    self.end_goal(tick, Outcome::Failed(Why::Stuck));
                } else {
                    self.patrol.next();
                }
                Intent::IDLE
            }
        }
    }

    /// Follow: stay within reach of the nearest awake teammate (the leader
    /// first), running to catch up, and fight whoever is going for them.
    /// Where they are comes from what its client was sent: its own eyes
    /// when they are in sight, else the team view, which a human teammate
    /// is sent too (wire v108).
    fn follow(&mut self, core: &mut ClientCore, body: &EntityState, tick: u32) -> Intent {
        let (x, z) = (body.qx as f32 * POS_XZ_Q, body.qz as f32 * POS_XZ_Q);
        let team = *core.team();
        let mut mate: Option<(bool, f32, [f32; 2], Option<f32>)> = None;
        for m in team.members() {
            if m.id == core.player_id || m.state != protocol::TEAM_AWAKE {
                continue;
            }
            let track = self.tracks.get(m.id);
            let at = track
                .filter(|t| t.visible)
                .map_or([f32::from(m.x_m) + 0.5, f32::from(m.z_m) + 0.5], |t| {
                    [t.pos[0], t.pos[2]]
                });
            // How high they stand, as last seen: the team view has no
            // height, and a floor above is where they are when it says
            // they are near but the eyes found them up there.
            let y = track
                .filter(|t| tick.wrapping_sub(t.last_seen) < FOLLOW_SEEN_TICKS)
                .map(|t| t.pos[1]);
            let d = (at[0] - x).hypot(at[1] - z);
            let lead = m.id == team.leader;
            if mate.is_none_or(|(l, md, ..)| (lead && !l) || (lead == l && d < md)) {
                mate = Some((lead, d, at, y));
            }
        }
        let Some((_, d, at, mate_y)) = mate else {
            self.end_goal(tick, Outcome::Failed(Why::NotFound));
            return Intent::IDLE;
        };
        // A crewmate (`agent::society`) only keeps up: within a stone's throw
        // of its founder it goes back to work, rather than standing on the
        // ground the base is going up on.
        if self.society.is_some() && d <= society::KEEP_UP_M && self.guard.is_none() {
            self.end_goal(tick, Outcome::Done);
            return Intent::IDLE;
        }
        // Someone is going for them: the fight reflex takes it from here,
        // and the goal resumes when it is over.
        // One it just let go is left alone, as the fight goal leaves it.
        if let Some(id) = self.guard.filter(|&id| !self.combat.shuns(id, tick)) {
            if self.loadout.on_belt(core, Role::Melee).is_some() || self.shooter(core).0.is_some() {
                self.combat.command(id, tick);
            }
        }
        self.stats.phase = Phase::Following;
        // A door they went through: through it after them first.
        if let Some(haven) = self.haven {
            let (seed, _) = core.island();
            if let Some(step) = self.leave.going(
                core,
                seed,
                &haven,
                body,
                &self.hands,
                &mut self.route,
                self.team_code,
                tick,
            ) {
                return self.leave_step(step, tick);
            }
        }
        // Another floor than theirs: the stairs (`agent::floors`), not the
        // doors below them.
        let my_y = body.qy as f32 * POS_Y_Q;
        let apart = mate_y.is_some_and(|y| (y - my_y).abs() > FOLLOW_APART_M);
        // Near them, but a wall or a shut door between: through the door.
        let walled = !apart && d <= FOLLOW_RUN_M && doors::walled_between(core, [x, z], at);
        if walled && self.leave.after(core, body, FOLLOW_DOOR_M, tick) {
            return Intent::IDLE;
        }
        // With them: eyes on them, as a player keeps them in view (and so
        // sees what they are standing at). Never halfway up a flight.
        if d <= FOLLOW_NEAR_M && !walled && !apart && !self.route.climbing() {
            let y = body.qy as f32 * POS_Y_Q + tracks::eye_m(false);
            return Intent {
                look: Look::Point([at[0], y, at[1]]),
                ..Intent::IDLE
            };
        }
        let hurry = d > FOLLOW_RUN_M && !apart;
        let off_ground = apart || self.route.upstairs(core, body);
        match self.route.to_at(
            core,
            body,
            at,
            mate_y.filter(|_| off_ground),
            FOLLOW_NEAR_M - 1.0,
            hurry,
            tick,
        ) {
            step @ Step::Walk { yaw, .. } => {
                self.heading = Some(yaw);
                step.walk().unwrap_or(Intent::walk(yaw))
            }
            Step::Arrived => Intent::IDLE,
            // No way to them: a door between, as like as not.
            Step::Blocked if self.leave.after(core, body, FOLLOW_DOOR_M, tick) => Intent::IDLE,
            // Up on a floor, or below theirs, a straight walk goes off an
            // edge or into a wall: keep them in view and wait for a way.
            Step::Blocked | Step::Wait if off_ground => Intent {
                look: Look::Point([at[0], mate_y.unwrap_or(my_y) + tracks::eye_m(false), at[1]]),
                ..Intent::IDLE
            },
            Step::Blocked | Step::Wait => {
                let yaw = yaw_toward(at[0] - x, at[1] - z);
                self.walk_on(core, body, yaw)
            }
        }
    }

    fn start_flee(&mut self, body: &EntityState, tick: u32) -> bool {
        let bx = body.qx as f32 * POS_XZ_Q;
        let bz = body.qz as f32 * POS_XZ_Q;
        let mut nearest: Option<(f32, f32, f32)> = None;
        for t in self.tracks.recent(tick, THREAT_RECALL_SECS * TICK_HZ) {
            let d = (t.pos[0] - bx).hypot(t.pos[2] - bz);
            if nearest.is_none_or(|n| d < n.0) {
                nearest = Some((d, t.pos[0], t.pos[2]));
            }
        }
        let Some((_, x, z)) = nearest else {
            return false;
        };
        let away = yaw_toward(bx - x, bz - z);
        self.combat.flee(tick, away);
        self.stats.retreats = self.combat.stats.retreats();
        if let Some(a) = self.goal.as_mut() {
            a.fled = true;
        }
        true
    }

    /// Take on the nearest body in sight of the kind the goal names (a
    /// player to fight, an animal to hunt), with a weapon on the belt. The
    /// reflex runs the fight from the next frame; the goal waits for it.
    fn start_fight(
        &mut self,
        core: &ClientCore,
        body: &EntityState,
        hunt: bool,
        tick: u32,
    ) -> bool {
        if self.loadout.on_belt(core, Role::Melee).is_none() && self.shooter(core).0.is_none() {
            return false;
        }
        let Some((id, _)) = self.quarry(body, hunt, tick) else {
            return false;
        };
        self.combat.command(id, tick);
        if let Some(a) = self.goal.as_mut() {
            a.fled = true;
        }
        true
    }

    /// The nearest body in sight of the kind a fight or hunt goal names,
    /// and where it stands.
    fn quarry(&self, body: &EntityState, hunt: bool, tick: u32) -> Option<(u32, [f32; 3])> {
        let (bx, bz) = (body.qx as f32 * POS_XZ_Q, body.qz as f32 * POS_XZ_Q);
        let mut nearest: Option<(f32, u32, [f32; 3])> = None;
        for t in self.tracks.seen() {
            if !t.visible
                || !t.active()
                || (t.species == Species::Player) == hunt
                || self.combat.shuns(t.id, tick)
            {
                continue;
            }
            let d = (t.pos[0] - bx).hypot(t.pos[2] - bz);
            if nearest.is_none_or(|n| d < n.0) {
                nearest = Some((d, t.id, t.pos));
            }
        }
        nearest.map(|(_, id, pos)| (id, pos))
    }

    /// Explore: walk to the nearest part of the map this body has not been
    /// to, ahead of it by preference, by a route round what stands in the
    /// way. A cell reached or found unreachable is crossed off.
    fn wander(&mut self, core: &mut ClientCore, body: &EntityState, tick: u32) -> Intent {
        let heading = *self.heading.get_or_insert(body.yaw);
        let (x, z) = (body.qx as f32 * POS_XZ_Q, body.qz as f32 * POS_XZ_Q);
        let target = match self.frontier.target {
            Some(t) => t,
            None => {
                let haven = self.haven.expect("connected haven");
                let (seed, _) = core.island();
                match self.frontier.pick(seed, &haven, x, z, heading) {
                    Some(t) => t,
                    None => {
                        // Everything near is walked: start the map afresh
                        // and keep going meanwhile.
                        self.frontier.clear();
                        self.frontier.visit(Frontier::cell_of(x, z));
                        return self.walk_on(core, body, heading);
                    }
                }
            }
        };
        let goal = Frontier::centre(target);
        match self.route.to(core, body, goal, FRONTIER_STOP_M, true, tick) {
            step @ Step::Walk { yaw, .. } => {
                if into_deeper_water(core, body, yaw) {
                    // Not worth a swim: somewhere else.
                    self.frontier.visit(target);
                    return self.walk_on(core, body, yaw.wrapping_add(1 << 14));
                }
                self.heading = Some(yaw);
                step.walk().unwrap_or(Intent::walk(yaw))
            }
            Step::Arrived | Step::Blocked => {
                self.frontier.visit(target);
                self.walk_on(core, body, heading)
            }
            Step::Wait => {
                let yaw = yaw_toward(goal[0] - x, goal[1] - z);
                self.walk_on(core, body, yaw)
            }
        }
    }

    /// Walk straight along a heading, turning off it rather than wading
    /// deeper: what the legs do while no route is in hand.
    fn walk_on(&mut self, core: &mut ClientCore, body: &EntityState, mut heading: u16) -> Intent {
        if into_deeper_water(core, body, heading) {
            heading = heading.wrapping_add(1 << 14);
        }
        self.heading = Some(heading);
        Intent::walk(heading)
    }

    /// A gather leg is over. A supply run goes on to the next node while
    /// its time lasts, and turns for the team's chest with what it has
    /// when the pack is full or there is no more to be had; any other
    /// gather goal ends.
    fn gather_over(&mut self, tick: u32, outcome: Outcome) {
        if let Some(a) = self.goal.as_mut().filter(|a| a.goal.supplies()) {
            match outcome {
                Outcome::Done => {
                    a.leg = tick;
                    a.abandons = 0;
                    return;
                }
                Outcome::Failed(_) if a.gained > 0 => {
                    a.delivering = true;
                    self.home_job = HomeJob::new(Trip::Stock);
                    self.target = None;
                    return;
                }
                _ => {}
            }
        }
        self.end_goal(tick, outcome);
    }

    /// A supply run: gather the kind for [`SUPPLY_GATHER_SECS`], then take
    /// it to the team's chest; done once it is in.
    fn supply(&mut self, core: &mut ClientCore, body: &EntityState, tick: u32) -> Intent {
        let Some(active) = self.goal else {
            return Intent::IDLE;
        };
        let Some(kind) = Kind::of_goal(active.goal) else {
            return Intent::IDLE;
        };
        let elapsed = tick.wrapping_sub(active.started);
        if !active.delivering {
            if elapsed < SUPPLY_GATHER_SECS * TICK_HZ {
                return self.gather(core, body, tick, kind);
            }
            if active.gained == 0 {
                self.end_goal(tick, Outcome::Failed(Why::NotFound));
                return Intent::IDLE;
            }
            self.gather_over(tick, Outcome::Failed(Why::Stuck));
        }
        if elapsed >= (SUPPLY_GATHER_SECS + DELIVER_GOAL_SECS) * TICK_HZ {
            self.deliver_over(tick, Outcome::Failed(Why::Stuck));
            return Intent::IDLE;
        }
        self.deliver(core, body, tick)
    }

    /// To the team's chest with the pack (`agent::supply`): through the
    /// doors in the way, everything past the belt put in, and out again.
    fn deliver(&mut self, core: &mut ClientCore, body: &EntityState, tick: u32) -> Intent {
        self.stats.phase = Phase::Delivering;
        let (intent, over) = self.home_step(core, body, tick);
        if let Some(outcome) = over {
            self.deliver_over(tick, outcome);
        }
        intent
    }

    /// Authorize (`Goal::Authorize`), Rust's tool-cupboard authorization:
    /// in through the team's doors to its cupboard (the trip a bag bound
    /// there takes, [`Self::bag_in`]), then onto its crew by the keys a
    /// player presses there. A lock on it: the team's code at its keypad,
    /// entered by the trip's own door walk — the right code is the whole
    /// invitation (`deploy::hearth_admits`), and the lock puts the hand it
    /// remembers on the crew in the same act, as the human client's `L`
    /// opens the pad there and sends no crew op. A bare one: `L` at it, the
    /// crew's join (`ACCESS_OP_CREW_JOIN`), which a crew that does not admit
    /// it refuses (`REFUSE_D_OWNER`). Done once the cupboard says it is on
    /// the crew (`EV_AUTH`, [`TeamHome::crewed`]).
    fn authorize(&mut self, core: &mut ClientCore, body: &EntityState, tick: u32) -> Intent {
        let Some(hearth) = self.shared.hearth() else {
            return self.auth_over(tick, Outcome::Failed(Why::NotFound));
        };
        if self.shared.crewed() {
            if let Some(a) = self.goal.as_mut() {
                a.gained = 1;
            }
            return self.auth_over(tick, Outcome::Done);
        }
        let (x, z) = (body.qx as f32 * POS_XZ_Q, body.qz as f32 * POS_XZ_Q);
        match self.auth.leg {
            AuthLeg::In => {
                self.stats.phase = Phase::GoingHome;
                let (intent, over) = self.home_step(core, body, tick);
                match over {
                    None => intent,
                    Some(Outcome::Failed(why)) => self.auth_over(tick, Outcome::Failed(why)),
                    Some(_) => {
                        self.auth.leg = AuthLeg::Crew;
                        Intent::IDLE
                    }
                }
            }
            AuthLeg::Crew if self.shared.hearth_locked(core) => {
                // The code went in once and did not put it on the crew (a
                // wrong code, or a full crew), or it has none to enter.
                if self.auth.knocked || self.team_code.is_none() {
                    return self.auth_over(tick, Outcome::Failed(Why::Locked));
                }
                self.home_job.doors.knock_at(hearth);
                self.auth.knocked = true;
                self.auth.leg = AuthLeg::In;
                Intent::IDLE
            }
            AuthLeg::Crew => {
                self.stats.phase = Phase::Waiting;
                let Some(haven) = self.haven else {
                    return Intent::IDLE;
                };
                let (seed, _) = core.island();
                let (since, point) = match self.auth.held {
                    Some(held) => held,
                    None => {
                        let point =
                            crate::agent::build::aim_point(core, seed, &haven, hearth, x, z);
                        self.auth.held = Some((tick, point));
                        (tick, point)
                    }
                };
                let waited = tick.wrapping_sub(since);
                // The eyes settle on it, and the press goes only where the
                // human client's `L` would take this cupboard.
                if waited >= home::HOLD_TICKS
                    && self.hands.settled()
                    && crate::agent::build::e_picks_by(
                        core,
                        x,
                        z,
                        self.hands.view().0,
                        hearth,
                        crate::agent::build::PRESS_SLACK_M,
                    )
                    && self.queue(|b| {
                        protocol::encode_action_access(
                            hearth.cx,
                            hearth.cz,
                            hearth.level,
                            hearth.loc,
                            ACCESS_OP_CREW_JOIN,
                            sim_core::lock::CODE_NONE,
                            b,
                        )
                    })
                {
                    self.auth.leg = AuthLeg::Sent(tick);
                    self.auth.refused = false;
                } else if waited >= VERDICT_SECS * TICK_HZ {
                    // Settled on it this long and `L` would still take
                    // something else.
                    return self.auth_over(tick, Outcome::Failed(Why::Blocked));
                }
                Intent {
                    look: Look::Point(point),
                    ..Intent::IDLE
                }
            }
            AuthLeg::Sent(at) => {
                if self.auth.refused {
                    return self.auth_over(tick, Outcome::Failed(Why::Refused));
                }
                if tick.wrapping_sub(at) >= VERDICT_SECS * TICK_HZ {
                    return self.auth_over(tick, Outcome::Failed(Why::NoAnswer));
                }
                self.stats.phase = Phase::Waiting;
                match self.auth.held {
                    Some((_, point)) => Intent {
                        look: Look::Point(point),
                        ..Intent::IDLE
                    },
                    None => Intent::IDLE,
                }
            }
        }
    }

    /// An authorize is over; one that came to nothing is not offered again
    /// for a while ([`AUTHORIZE_RETRY_TICKS`]).
    fn auth_over(&mut self, tick: u32, outcome: Outcome) -> Intent {
        if matches!(outcome, Outcome::Failed(_)) {
            self.auth_failed = Some(tick);
        }
        self.auth = Auth::default();
        self.home_job = HomeJob::default();
        self.end_goal(tick, outcome);
        Intent::IDLE
    }

    /// One step of the trip into the team's base (`agent::supply`):
    /// through its doors, its boxes seen to, and out; or there to stay.
    /// `Some` once the trip is over.
    fn home_step(
        &mut self,
        core: &mut ClientCore,
        body: &EntityState,
        tick: u32,
    ) -> (Intent, Option<Outcome>) {
        let Some(haven) = self.haven else {
            return (Intent::IDLE, None);
        };
        if self.shared.anchor().is_none() {
            return (Intent::IDLE, Some(Outcome::Failed(Why::NotFound)));
        }
        let (seed, _) = core.island();
        let wants = self.wants;
        let step = self.home_job.step(
            core,
            seed,
            &haven,
            body,
            &self.hands,
            &mut self.route,
            &self.shared,
            &wants[..usize::from(self.wants_len)],
            self.team_code,
            &self.book,
            tick,
        );
        let (sent, intent) = match step {
            Deliver::Door(ref door) => self.door_verb(door),
            Deliver::Open { key, intent } => (
                self.queue(|buf| protocol::encode_action_container(CONT_BOX, key, buf)),
                intent,
            ),
            Deliver::Put {
                key,
                from,
                to,
                count,
                intent,
            } => (
                self.queue(|buf| {
                    protocol::encode_action_move(key, CONT_SELF, from, CONT_BOX, to, count, buf)
                }),
                intent,
            ),
            Deliver::Take {
                key,
                from,
                to,
                count,
                intent,
            } => (
                self.queue(|buf| {
                    protocol::encode_action_move(key, CONT_BOX, from, CONT_SELF, to, count, buf)
                }),
                intent,
            ),
            Deliver::Close(intent) => (
                self.queue(|buf| protocol::encode_action_container(CONT_SELF, 0, buf)),
                intent,
            ),
            // The shut panel may still wait on its pace.
            Deliver::Done => {
                return (Intent::IDLE, self.outbox.is_none().then_some(Outcome::Done));
            }
            Deliver::Fail(why) => return (Intent::IDLE, Some(Outcome::Failed(why))),
        };
        if sent {
            self.home_job.sent(&step, tick);
        }
        (intent, None)
    }

    /// A trade at THE GATE (`agent::gate`): to the stall's counter, the
    /// vend verb, its answer.
    fn trade(
        &mut self,
        core: &mut ClientCore,
        body: &EntityState,
        name: Name,
        sell: bool,
        tick: u32,
    ) -> Intent {
        self.stats.phase = Phase::Trading;
        let goal = if sell {
            Goal::Sell(name)
        } else {
            Goal::Buy(name)
        };
        let Some(item) = crate::agent::build::item_named(core, name.as_str()) else {
            self.visit_over(goal, Outcome::Failed(Why::NoRecipe), tick);
            return Intent::IDLE;
        };
        match self
            .trade_job
            .step(core, body, &mut self.route, item, sell, tick)
        {
            Visit::Go(intent) => intent,
            Visit::Vend { offer, times } => {
                if self.queue(|buf| protocol::encode_action_vend(offer, times, buf)) {
                    self.trade_job.sent(tick);
                }
                Intent::IDLE
            }
            Visit::Done => {
                self.visit_over(goal, Outcome::Done, tick);
                Intent::IDLE
            }
            Visit::Fail(why) => {
                self.visit_over(goal, Outcome::Failed(why), tick);
                Intent::IDLE
            }
            Visit::Arc { .. } => Intent::IDLE,
        }
    }

    /// A feed at a work (`agent::gate`): to the nearest work that takes
    /// what the pack holds, everything it takes put in.
    fn feed_work(&mut self, core: &mut ClientCore, body: &EntityState, tick: u32) -> Intent {
        self.stats.phase = Phase::Trading;
        let Some(haven) = self.haven else {
            return Intent::IDLE;
        };
        match self
            .work_job
            .step(core, &haven, body, &mut self.route, tick)
        {
            Visit::Go(intent) => intent,
            Visit::Arc { op, target, arg } => {
                if self.queue(|buf| protocol::encode_action_arc(op, target, arg, buf)) {
                    self.work_job.sent(core, target, op, tick);
                }
                Intent::IDLE
            }
            Visit::Done => {
                if let Some(a) = self.goal.as_mut() {
                    a.gained = self.work_job.gave;
                }
                self.visit_over(Goal::FeedWork, Outcome::Done, tick);
                Intent::IDLE
            }
            Visit::Fail(why) => {
                self.visit_over(Goal::FeedWork, Outcome::Failed(why), tick);
                Intent::IDLE
            }
            Visit::Vend { .. } => Intent::IDLE,
        }
    }

    /// A trade or a feed is over; one that failed is held a while.
    fn visit_over(&mut self, goal: Goal, outcome: Outcome, tick: u32) {
        if matches!(outcome, Outcome::Failed(_)) {
            if goal == Goal::FeedWork {
                self.work_failed = Some(tick);
            } else {
                self.trade_failed = Some(tick);
            }
        }
        self.end_goal(tick, outcome);
    }

    /// A stone read, a lock solved or a card door opened (`agent::lore`):
    /// the walk there, the press, its answer.
    fn lore_visit(
        &mut self,
        core: &mut ClientCore,
        body: &EntityState,
        goal: Goal,
        tick: u32,
    ) -> Intent {
        self.stats.phase = Phase::Ancients;
        let step = match goal {
            Goal::ReadStone => self.read_job.step(core, body, &mut self.route, tick),
            Goal::SolveLock => self.solve_job.step(core, body, &mut self.route, tick),
            _ => self.swipe_job.step(core, body, &mut self.route, tick),
        };
        match step {
            LoreVisit::Go(intent) => intent,
            LoreVisit::Arc { op, target, arg } => {
                if self.queue(|buf| protocol::encode_action_arc(op, target, arg, buf)) {
                    if goal == Goal::SolveLock {
                        self.solve_job.sent(core, arg, tick);
                    } else {
                        self.read_job.sent(tick);
                    }
                }
                Intent::IDLE
            }
            LoreVisit::Swipe(door) => {
                if self.queue(|buf| protocol::encode_action_swipe(door, buf)) {
                    self.swipe_job.sent(tick);
                }
                Intent::IDLE
            }
            LoreVisit::Done => {
                if let Some(a) = self.goal.as_mut() {
                    a.gained = a.gained.saturating_add(1);
                }
                self.lore_over(goal, Outcome::Done, tick);
                Intent::IDLE
            }
            LoreVisit::Fail(why) => {
                self.lore_over(goal, Outcome::Failed(why), tick);
                Intent::IDLE
            }
        }
    }

    /// A read, a solve or a swipe is over; one that failed is held a while.
    fn lore_over(&mut self, goal: Goal, outcome: Outcome, tick: u32) {
        if matches!(outcome, Outcome::Failed(_)) {
            let i = match goal {
                Goal::ReadStone => 0,
                Goal::SolveLock => 1,
                _ => 2,
            };
            self.lore_failed[i] = Some(tick);
        }
        self.end_goal(tick, outcome);
    }

    /// The way out of the ziggurat card room it stands in (`agent::lore::
    /// ExitJob`); `None` once out, in none, or after a way out came to
    /// nothing a moment ago.
    fn exit_room(
        &mut self,
        core: &mut ClientCore,
        body: &EntityState,
        descend: bool,
        tick: u32,
    ) -> Option<Intent> {
        let haven = self.haven?;
        if self
            .exit_job
            .failed
            .is_some_and(|at| tick.wrapping_sub(at) < doors::LEAVE_RETRY_TICKS)
        {
            return None;
        }
        match self
            .exit_job
            .step(core, &haven, body, &mut self.route, descend, tick)?
        {
            LoreVisit::Go(intent) => {
                self.stats.phase = Phase::Leaving;
                Some(intent)
            }
            LoreVisit::Swipe(door) => {
                if self.queue(|buf| protocol::encode_action_swipe(door, buf)) {
                    self.exit_job.sent(tick);
                }
                Some(Intent::IDLE)
            }
            LoreVisit::Fail(_) => {
                self.exit_job.failed = Some(tick);
                None
            }
            LoreVisit::Done | LoreVisit::Arc { .. } => None,
        }
    }

    /// A point within the team's base.
    fn near_shared(&self, (x, z): (f32, f32)) -> bool {
        self.shared
            .anchor()
            .is_some_and(|[hx, hz]| (x - hx).hypot(z - hz) <= supply::HOME_M)
    }

    /// It has no base of its own and knows the team's: it lives there.
    fn shared_home(&self) -> bool {
        !self.builder.survey().hearth && self.shared.anchor().is_some()
    }

    /// Its own stations, and where it has none of a kind, the team's.
    fn stations(&self, core: &ClientCore) -> Stations {
        let own = self.builder.stations(core);
        let team = self.shared.stations();
        Stations {
            bench: own.bench.or(team.bench),
            bench2: own.bench2.or(team.bench2),
            furnace: own.furnace.or(team.furnace),
            tier: own.tier.max(team.tier),
        }
    }

    /// The walk home to the team's base, to stay.
    fn go_home_shared(&mut self, core: &mut ClientCore, body: &EntityState, tick: u32) -> Intent {
        self.stats.phase = Phase::GoingHome;
        let (intent, over) = self.home_step(core, body, tick);
        if let Some(outcome) = over {
            self.end_goal(tick, outcome);
        }
        intent
    }

    /// The team's base is under attack: home, and whoever is about it and
    /// not on the team is fought, on the way in or once there, until it has
    /// been quiet a while.
    fn defend_shared(&mut self, core: &mut ClientCore, body: &EntityState, tick: u32) -> Intent {
        self.stats.phase = Phase::Defending;
        let Some(active) = self.goal else {
            return Intent::IDLE;
        };
        let Some(home) = self.shared.anchor() else {
            self.end_goal(tick, Outcome::Failed(Why::NotFound));
            return Intent::IDLE;
        };
        // Whoever it sees at the base is taken on wherever it is: from
        // inside, the walls hide the one hacking at them.
        let about = self.guard_shared(core, home, tick);
        // `fled` marks the arrival, `leg` the last time anyone was about.
        if !active.fled {
            let (intent, over) = self.home_step(core, body, tick);
            match over {
                None => return intent,
                Some(Outcome::Failed(why)) => {
                    self.defend_failed = Some(tick);
                    self.end_goal(tick, Outcome::Failed(why));
                    return Intent::IDLE;
                }
                Some(_) => {
                    if let Some(a) = self.goal.as_mut() {
                        a.fled = true;
                        a.leg = tick;
                    }
                }
            }
        }
        if !about && tick.wrapping_sub(active.leg) >= SHARED_GUARD_SECS * TICK_HZ {
            self.home_alarm = None;
            self.memory.home_alarm = false;
            self.end_goal(tick, Outcome::Done);
        }
        Intent::IDLE
    }

    /// The nearest stranger in sight within the team's base or near it is
    /// fought; true when there was one.
    fn guard_shared(&mut self, core: &ClientCore, home: [f32; 2], tick: u32) -> bool {
        let mut stranger: Option<(f32, u32)> = None;
        for t in self.tracks.seen() {
            if !t.visible
                || !t.active()
                || t.dead
                || t.species != Species::Player
                || t.id == core.player_id
                || core.is_teammate(t.id)
            {
                continue;
            }
            let d = (t.pos[0] - home[0]).hypot(t.pos[2] - home[1]);
            if d <= supply::HOME_M + GUARD_M && stranger.is_none_or(|s| d < s.0) {
                stranger = Some((d, t.id));
            }
        }
        let Some((_, id)) = stranger else {
            return false;
        };
        if !self.combat.shuns(id, tick) {
            self.combat.command(id, tick);
        }
        if let Some(a) = self.goal.as_mut() {
            a.leg = tick;
        }
        true
    }

    /// Send what a walk through a door asked for: a use, or the team's
    /// code at a lock. True when it went out.
    fn door_verb(&mut self, step: &Through) -> (bool, Intent) {
        match *step {
            Through::Go(intent) => (false, intent),
            Through::Use(at, intent) => (
                self.queue(|buf| protocol::encode_action_use(at.cx, at.cz, at.level, at.loc, buf)),
                intent,
            ),
            // The team's code goes from its keeping onto the wire, the
            // keypad's way, and nowhere else.
            Through::Enter(at, code, intent) => (
                self.queue(|buf| {
                    protocol::encode_action_access(
                        at.cx,
                        at.cz,
                        at.level,
                        at.loc,
                        ACCESS_OP_ENTER,
                        code.wire(),
                        buf,
                    )
                }),
                intent,
            ),
            _ => (false, Intent::IDLE),
        }
    }

    /// A step of [`Leave`]'s door, sent.
    fn leave_step(&mut self, step: Through, tick: u32) -> Intent {
        let (sent, intent) = self.door_verb(&step);
        if sent {
            self.leave.doors.sent(&step, tick);
        }
        intent
    }

    /// Walled in on a teammate's floor before a walk: out by the nearest
    /// door, shutting it behind. `None` once out, or with no door to go by.
    fn leave_mates(
        &mut self,
        core: &mut ClientCore,
        body: &EntityState,
        tick: u32,
    ) -> Option<Intent> {
        let haven = self.haven?;
        let (seed, _) = core.island();
        let step = self.leave.out(
            core,
            seed,
            &haven,
            body,
            &self.hands,
            &mut self.route,
            self.team_code,
            tick,
        )?;
        self.stats.phase = Phase::Leaving;
        Some(self.leave_step(step, tick))
    }

    /// The trip to the chest is over: what went in is counted, a delivery
    /// on its own is gained by it, and one that failed is not offered again
    /// for a while.
    fn deliver_over(&mut self, tick: u32, outcome: Outcome) {
        let (put, took) = (self.home_job.put, self.home_job.took);
        self.delivered += u64::from(put);
        self.took += u64::from(took);
        if let Some(a) = self.goal.as_mut().filter(|a| a.goal == Goal::Deliver) {
            a.gained = put + took;
        }
        if matches!(outcome, Outcome::Failed(_)) {
            self.deliver_failed = Some(tick);
        }
        if outcome == Outcome::Failed(Why::BoxFull) && self.goal.is_some_and(|a| a.goal.supplies())
        {
            self.box_full = Some(tick);
        }
        self.home_job = HomeJob::default();
        self.end_goal(tick, outcome);
    }

    fn gather(
        &mut self,
        core: &mut ClientCore,
        body: &EntityState,
        tick: u32,
        kind: Kind,
    ) -> Intent {
        let Some(active) = self.goal else {
            return Intent::IDLE;
        };
        let Some(sel) = best_tool(core, kind) else {
            self.gather_over(tick, Outcome::Failed(Why::NoTool));
            return Intent::IDLE;
        };
        let hold = Intent {
            sel: Some(sel),
            ..Intent::IDLE
        };
        if !room_for(core, self.memory.yields[kind as usize]) {
            self.gather_over(tick, Outcome::Failed(Why::PackFull));
            return hold;
        }
        if let Some((start, yaw)) = self.recovery {
            if tick.wrapping_sub(start) < RECOVER_TICKS {
                self.stats.phase = Phase::Recovering;
                return Intent {
                    sel: Some(sel),
                    ..Intent::walk(yaw)
                };
            }
            self.recovery = None;
        }
        if let Some(target) = self.target {
            if target.gone(core) {
                self.target = None;
                if self.target_gain > 0 {
                    self.stats.targets_completed += 1;
                    self.gather_over(tick, Outcome::Done);
                    return hold;
                }
            }
        }
        let Some(target) = self.target else {
            let searched = tick.wrapping_sub(active.leg);
            // Head back toward the nearest one remembered: facing it puts
            // it in the cone, where the scan acquires it like any other.
            // The walk there has its own time: the search's is for looking
            // round, and is gone before a node 100 m off is in sight.
            if let Some((seen, at)) = self.recall[kind as usize] {
                let fresh = tick.wrapping_sub(at) < RECALL_SECS * TICK_HZ
                    && !seen.gone(core)
                    && Some(seen.key()) != self.skipped;
                let (yaw, _, distance) = aim(body, &seen.slot);
                let step = if fresh
                    && distance > REACH_M
                    && distance <= RECALL_WALK_M
                    && searched < RECALL_WALK_SECS * TICK_HZ
                {
                    self.route
                        .to(core, body, [seen.slot.x, seen.slot.z], REACH_M, true, tick)
                } else {
                    Step::Arrived
                };
                // Walking the route, or straight at it until a route is
                // planned; there, blocked or gone ends the walk.
                let walking = match step {
                    Step::Walk { yaw, .. } => Some(yaw),
                    Step::Wait => Some(yaw),
                    Step::Arrived | Step::Blocked => None,
                };
                if let Some(yaw) = walking {
                    self.stats.phase = Phase::Approaching;
                    self.heading = Some(yaw);
                    let mut walk = Intent {
                        sel: Some(sel),
                        ..step.walk().unwrap_or(Intent::walk(yaw))
                    };
                    if into_deeper_water(core, body, yaw) {
                        walk.travel = None;
                    }
                    return walk;
                }
                // Arrived and still not acquired, or gone: forget it.
                self.recall[kind as usize] = None;
            }
            if searched >= SEARCH_GOAL_SECS * TICK_HZ {
                self.gather_over(tick, Outcome::Failed(Why::NotFound));
                return hold;
            }
            self.stats.phase = Phase::Exploring;
            return Intent {
                sel: Some(sel),
                ..self.wander(core, body, tick)
            };
        };
        let slot = target.live(core);
        let (yaw, pitch, distance) = aim(body, &slot);
        let mut intent = Intent {
            look: Look::Point(aim_point(body, &slot)),
            ..hold
        };
        if distance + POS_XZ_Q < self.best_distance {
            self.best_distance = distance;
            self.progress_tick = tick;
        }
        if tick.wrapping_sub(self.progress_tick) >= NO_PROGRESS_TICKS {
            self.skipped = Some(target.key());
            self.target = None;
            self.stats.targets_abandoned += 1;
            self.recovery = Some((tick, body.yaw.wrapping_add(1 << 14)));
            let abandons = self.goal.as_mut().map_or(0, |a| {
                a.abandons += 1;
                a.abandons
            });
            if abandons >= MAX_ABANDONS {
                self.gather_over(tick, Outcome::Failed(Why::Stuck));
            }
            return intent;
        }
        if kind.picked() && distance <= REACH_M && pick_reaches(core, body, yaw, pitch, target) {
            // A plant is picked with `E`, not swung at: the human client's
            // `[E] PICK`, sent once the view the hands hold is on it.
            self.stats.phase = Phase::Harvesting;
            let (held_yaw, held_pitch) = self.hands.view();
            let haven = self.haven.expect("connected haven");
            if visible(core, &haven, body, target)
                && pick_reaches(core, body, held_yaw, held_pitch, target)
            {
                let cell = target.key();
                self.queue(|buf| protocol::encode_action_pick(cell, buf));
            }
        } else if !kind.picked() && swing_reaches(core, body, yaw, pitch, target) {
            // Holding primary is the human harvesting verb. A harvest swing
            // never lands on a body: one standing on the swing's own ray
            // holds it, wherever else bodies stand round the node.
            self.stats.phase = Phase::Harvesting;
            let (held_yaw, held_pitch) = self.hands.view();
            let bystander = self.body_on_ray(body, held_yaw);
            // Of two bodies in each other's way at a node, the lower id
            // keeps it and the other gives way; otherwise both would stand
            // there waiting.
            if bystander.is_some_and(|other| other < body.id) {
                self.skipped = Some(target.key());
                self.target = None;
                self.stats.gave_way += 1;
                self.recovery = Some((tick, body.yaw.wrapping_add(1 << 14)));
                return intent;
            }
            let haven = self.haven.expect("connected haven");
            // Swing only once the view the hands hold is on the node too:
            // standing in reach is not yet aiming at it.
            if bystander.is_none()
                && visible(core, &haven, body, target)
                && swing_reaches(core, body, held_yaw, held_pitch, target)
            {
                intent.buttons = BTN_PRIMARY;
            }
        } else {
            // Someone already at it: another node, rather than walking up
            // to a stranger with a rock in hand.
            if self.someone_at(body, &target.slot, tick) {
                self.skipped = Some(target.key());
                self.target = None;
                self.stats.gave_way += 1;
                return intent;
            }
            self.stats.phase = Phase::Approaching;
            // By a route round what stands between; straight on once the
            // route has nothing better to say.
            match self.route.to(
                core,
                body,
                [target.slot.x, target.slot.z],
                REACH_M,
                false,
                tick,
            ) {
                Step::Walk { yaw, jump, .. } => {
                    intent.travel = Some(yaw);
                    if jump {
                        intent.buttons |= sim_core::input::BTN_JUMP;
                    }
                }
                _ => intent.move_z = 127,
            }
        }
        intent
    }

    /// A player seen lately standing at this node, nearer it than I am.
    fn someone_at(&self, body: &EntityState, slot: &Slot, tick: u32) -> bool {
        let (x, z) = (body.qx as f32 * POS_XZ_Q, body.qz as f32 * POS_XZ_Q);
        let mine = (slot.x - x).hypot(slot.z - z);
        self.tracks.recent(tick, TICK_HZ).any(|t| {
            let theirs = (slot.x - t.pos[0]).hypot(slot.z - t.pos[2]);
            t.species == Species::Player
                && t.id != body.id
                && theirs <= NODE_TAKEN_M
                && theirs < mine
        })
    }

    /// The lowest id among the bodies in view standing across a swing
    /// along this bearing, within [`BYSTANDER_RAY_M`]: where my screen
    /// draws them (the pose the server judges the swing against), and
    /// bodies just come into view as well as those long in sight.
    fn body_on_ray(&self, body: &EntityState, yaw: u16) -> Option<u32> {
        let (fx, fz) = yaw_dir(yaw);
        let (x, z) = (body.qx as f32 * POS_XZ_Q, body.qz as f32 * POS_XZ_Q);
        let on_ray = |species: Species, at: [f32; 3]| {
            let (dx, dz) = (at[0] - x, at[2] - z);
            let along = dx * fx + dz * fz;
            let across = (dx * fz - dz * fx).abs();
            let radius = match species {
                Species::Player => sim_core::collide::CAPSULE_RADIUS_M,
                Species::Pig => f32::from(self.book.pig().radius_cm) * 0.01,
                Species::Wolf => f32::from(self.book.wolf().radius_cm) * 0.01,
            };
            // A body pressed against mine is in the way of any swing,
            // whichever way it faces: the swing's ray leaves from inside
            // its capsule, and where my screen draws it lags where it is.
            let touch = radius + sim_core::collide::CAPSULE_RADIUS_M + BYSTANDER_MISS_M;
            let pressed = dx * dx + dz * dz <= touch * touch;
            pressed
                || (along > -radius
                    && along <= BYSTANDER_RAY_M + radius
                    && across <= radius + BYSTANDER_MISS_M)
        };
        let mut lowest: Option<u32> = None;
        for t in self.tracks.all() {
            if !t.visible || t.dead || t.id == body.id {
                continue;
            }
            let at = self
                .tracks
                .aim_pose(t.id)
                .map_or(t.pos, |s| [s.x, s.y, s.z]);
            if on_ray(t.species, at) {
                lowest = Some(lowest.map_or(t.id, |b| b.min(t.id)));
            }
        }
        for (id, species, at) in self.tracks.glimpses() {
            if id != body.id && on_ray(species, at) {
                lowest = Some(lowest.map_or(id, |b| b.min(id)));
            }
        }
        lowest
    }

    fn take_verdict(&mut self, tick: u32, budget: u32) -> Result<Option<Verdict>, ()> {
        let Some((_, since)) = self.awaiting else {
            return Ok(None);
        };
        if let Some(v) = self.verdict.take() {
            self.awaiting = None;
            return Ok(Some(v));
        }
        if tick.wrapping_sub(since) >= budget {
            self.awaiting = None;
            return Err(());
        }
        Ok(None)
    }

    /// Craft one of the named item (a batch, at a furnace), at one of its
    /// own stations where the recipe needs one, walking home to it first.
    fn craft(
        &mut self,
        core: &mut ClientCore,
        body: &EntityState,
        name: Name,
        tick: u32,
    ) -> Intent {
        let Some(active) = self.goal else {
            return Intent::IDLE;
        };
        self.stats.phase = Phase::Crafting;
        match active.craft {
            CraftStep::Start => {
                let has = self.stations(core);
                let Some((recipe, item, ticks, station)) = resolve_recipe(core, name, &has) else {
                    self.end_goal(tick, Outcome::Failed(Why::NoRecipe));
                    return Intent::IDLE;
                };
                if !inputs_ok(core, recipe) {
                    self.end_goal(tick, Outcome::Failed(Why::MissingInputs));
                    return Intent::IDLE;
                }
                // A full pack drops what is made at the crafter's feet, and
                // the inputs are spent either way.
                if !room_for(core, ItemBits::bit(item))
                    && !craft_fits(core, recipe, batch(core, recipe))
                {
                    self.end_goal(tick, Outcome::Failed(Why::PackFull));
                    return Intent::IDLE;
                }
                // A full queue refuses whatever is asked.
                if usize::from(core.jobs_count) >= CRAFT_QUEUE {
                    self.end_goal(tick, Outcome::Failed(Why::Refused));
                    return Intent::IDLE;
                }
                let here = [body.qx as f32 * POS_XZ_Q, body.qz as f32 * POS_XZ_Q];
                if !has.in_reach(station, here) {
                    return self.walk_to_station(core, body, station, tick);
                }
                let count = batch(core, recipe);
                // Behind other jobs the first unit starts when they are
                // done: waited for, up to a point.
                let ahead = queue_wait(core).min(QUEUE_WAIT_SECS * TICK_HZ);
                if self.queue(|buf| protocol::encode_action_craft(recipe, count, 0, buf)) {
                    self.awaiting = Some((Pending::Craft { item }, tick));
                    self.verdict = None;
                    self.set_craft(CraftStep::Sent {
                        ticks: ticks + ahead,
                        recipe,
                    });
                }
            }
            CraftStep::Sent { ticks, recipe } => {
                match self.take_verdict(tick, ticks + VERDICT_SECS * TICK_HZ) {
                    // Queued behind a long job: it pays out on its own, and
                    // the inputs are already spent on it.
                    Err(()) if queued(core, recipe) => self.end_goal(tick, Outcome::Done),
                    Err(()) => self.end_goal(tick, Outcome::Failed(Why::NoAnswer)),
                    Ok(Some(Verdict::Ok)) => {
                        // The first unit is in; a batch pays the rest out
                        // while the body goes on.
                        if let Some(a) = self.goal.as_mut() {
                            a.gained += 1;
                        }
                        self.set_craft(CraftStep::Equip);
                    }
                    Ok(Some(_)) => self.end_goal(tick, Outcome::Failed(Why::Refused)),
                    Ok(None) => {}
                }
            }
            CraftStep::Equip => {
                // A better tool, weapon or med belongs on the belt, where a
                // key selects it (`agent::loadout`'s belt policy).
                let item = (0..usize::from(core.catalog.count).min(MAX_ITEM_DEFS))
                    .find(|&i| core.catalog.name(i) == name.as_bytes());
                match item.and_then(|i| self.loadout.belt_move(core, i as u16)) {
                    Some((from, to, count)) => {
                        if self.queue(|buf| {
                            protocol::encode_action_move(
                                0, CONT_SELF, from, CONT_SELF, to, count, buf,
                            )
                        }) {
                            self.stats.phase = Phase::Equipping;
                            self.awaiting = Some((Pending::Equip, tick));
                            self.verdict = None;
                            self.set_craft(CraftStep::Equipped);
                        }
                    }
                    None => self.end_goal(tick, Outcome::Done),
                }
            }
            CraftStep::Equipped => {
                self.stats.phase = Phase::Equipping;
                match self.take_verdict(tick, VERDICT_SECS * TICK_HZ) {
                    Ok(Some(Verdict::Ok)) => {
                        self.stats.equips += 1;
                        self.end_goal(tick, Outcome::Done);
                    }
                    // The tool exists either way; the next swing still
                    // finds it if it reached the belt.
                    Ok(Some(_)) | Err(()) => self.end_goal(tick, Outcome::Done),
                    Ok(None) => {}
                }
            }
        }
        Intent::IDLE
    }

    /// Home to its stations: in through the doors to the stand spot, which
    /// its workbench and furnace are both in reach of. A walk that comes to
    /// nothing holds every craft that needs a station a while
    /// (`STATION_RETRY_TICKS`); one that has not got there in
    /// `CRAFT_WALK_SECS`, or got to the stand spot and is still out of the
    /// station's reach, has come to nothing.
    fn walk_to_station(
        &mut self,
        core: &mut ClientCore,
        body: &EntityState,
        station: u8,
        tick: u32,
    ) -> Intent {
        self.stats.phase = Phase::GoingHome;
        if self
            .goal
            .is_some_and(|a| tick.wrapping_sub(a.started) >= CRAFT_WALK_SECS * TICK_HZ)
        {
            return self.station_over(tick, Why::Stuck);
        }
        // None of its own: the team's, in through the team's doors.
        if self.builder.stations(core).spot(station).is_none() {
            if let Some(spot) = self.shared.stations().spot(station) {
                if self.home_job.trip() != Trip::Point(spot) {
                    self.home_job = HomeJob::new(Trip::Point(spot));
                }
                let (intent, over) = self.home_step(core, body, tick);
                return match over {
                    Some(Outcome::Failed(why)) => self.station_over(tick, why),
                    // There, and still out of the station's reach.
                    Some(_) => self.station_over(tick, Why::Stuck),
                    None => intent,
                };
            }
        }
        let (Some(haven), Some(stand)) = (self.haven, self.builder.stand()) else {
            return self.station_over(tick, Why::NoRecipe);
        };
        let (seed, _) = core.island();
        if self.builder.walled(core) && self.builder.region(body) != Region::Room {
            let act = self.builder.pass(
                core,
                seed,
                &haven,
                body,
                &self.hands,
                &mut self.route,
                Way::In,
                tick,
            );
            match act {
                Act::Done => {}
                Act::Fail(why) => return self.station_over(tick, why),
                act => return self.act(act, tick),
            }
        }
        let (x, z) = (body.qx as f32 * POS_XZ_Q, body.qz as f32 * POS_XZ_Q);
        match self.route.to(core, body, stand, 1.0, false, tick) {
            step @ Step::Walk { .. } => step.walk().unwrap_or(Intent::IDLE),
            Step::Blocked => self.station_over(tick, Why::Stuck),
            // On the stand spot and still out of the station's reach: the
            // station is not where the stand spot reaches, and walking at
            // the spot would go on for ever.
            Step::Arrived if (stand[0] - x).hypot(stand[1] - z) <= STAND_ON_M => {
                self.station_over(tick, Why::Stuck)
            }
            Step::Arrived | Step::Wait => Intent::walk(yaw_toward(stand[0] - x, stand[1] - z)),
        }
    }

    /// The walk to its stations came to nothing.
    fn station_over(&mut self, tick: u32, why: Why) -> Intent {
        self.station_failed = Some(tick);
        self.end_goal(tick, Outcome::Failed(why));
        Intent::IDLE
    }

    /// A loot run (`agent::loot`): the nearest barrel or crate it knows of,
    /// the next one near it, and so on; with none known, the nearest place
    /// on the map that keeps crates, and a look round there. A run ends
    /// when the pack is full, nothing more is near, or it has taken
    /// [`loot::MAX_STOPS`] containers.
    fn loot_run(&mut self, core: &mut ClientCore, body: &EntityState, tick: u32) -> Intent {
        self.stats.phase = Phase::Looting;
        let (Some(active), Some(haven)) = (self.goal, self.haven) else {
            return Intent::IDLE;
        };
        if tick.wrapping_sub(active.started) >= LOOT_GOAL_SECS * TICK_HZ
            || self.loot_job.stops >= loot::MAX_STOPS
        {
            // What it was still working on when time ran out is not picked
            // first again on the next run.
            if self.loot_job.smashed.is_none() {
                if let Some(spot) = self.loot_job.target.take() {
                    self.loot.emptied(spot.key(), tick);
                }
            }
            return self.loot_over(tick);
        }
        if let Some((spot, at)) = self.loot_job.smashed {
            return self.loot_pickup(core, body, spot, at, tick);
        }
        if let Some(spot) = self.loot_job.target {
            return match spot.prize {
                Prize::Barrel => self.loot_barrel(core, body, spot, tick),
                Prize::Crate => self.loot_crate(core, body, spot, tick),
            };
        }
        if loot::free_slots(core) == 0 {
            return self.loot_over(tick);
        }
        let here = [body.qx as f32 * POS_XZ_Q, body.qz as f32 * POS_XZ_Q];
        // The next container: on the first stop any it knows of on the
        // island's near side, after that only one near the last.
        let within = if self.loot_job.stops == 0 && self.loot_job.place.is_none() {
            loot::TRIP_M
        } else {
            loot::NEXT_SPOT_M
        };
        if let Some(spot) = self.loot.nearest(core, here, within, tick) {
            self.loot_job.target = Some(spot);
            self.loot_job.arrived = None;
            return Intent::IDLE;
        }
        if self.loot_job.stops > 0 && self.loot_job.place.is_none() {
            // Nothing more near what it took.
            return self.loot_over(tick);
        }
        // None known: to the map.
        let Some((place, at)) = self
            .loot_job
            .place
            .or_else(|| self.loot.place(&haven, here, tick))
        else {
            return self.loot_over(tick);
        };
        self.loot_job.place = Some((place, at));
        let d = (at[0] - here[0]).hypot(at[1] - here[1]);
        if d <= loot::PLACE_NEAR_M || self.loot_job.arrived.is_some() {
            // There: a slow turn round finds what stands about.
            let since = *self.loot_job.arrived.get_or_insert(tick);
            if tick.wrapping_sub(since) >= LOOK_ROUND_TICKS {
                self.loot.visited(place, tick);
                self.loot_job.place = None;
                self.loot_job.arrived = None;
                return self.loot_over(tick);
            }
            let turn = (tick.wrapping_sub(since) * 65536 / LOOK_ROUND_TICKS) as u16;
            return Intent {
                look: Look::Heading(body.yaw.wrapping_add(turn.min(0x3000))),
                ..Intent::IDLE
            };
        }
        match self
            .route
            .to(core, body, at, loot::PLACE_NEAR_M * 0.5, true, tick)
        {
            step @ Step::Walk { yaw, .. } => {
                if into_deeper_water(core, body, yaw) {
                    self.loot.visited(place, tick);
                    self.loot_job.place = None;
                    return Intent::IDLE;
                }
                step.walk().unwrap_or(Intent::walk(yaw))
            }
            Step::Blocked => {
                // No way there: somewhere else, another time.
                self.loot.visited(place, tick);
                self.loot_job.place = None;
                Intent::IDLE
            }
            Step::Arrived => {
                self.loot_job.arrived = Some(tick);
                Intent::IDLE
            }
            Step::Wait => self.walk_on(core, body, yaw_toward(at[0] - here[0], at[1] - here[1])),
        }
    }

    /// The run is over: what it took comes home with it. An open panel is
    /// shut on the way out.
    fn loot_over(&mut self, tick: u32) -> Intent {
        if matches!(
            self.loot_job.lid,
            Some(Lid::Open | Lid::Moving(..) | Lid::Settling(_))
        ) {
            if !self.queue(|buf| protocol::encode_action_container(CONT_SELF, 0, buf)) {
                // The hand is busy: the close goes next frame.
                return Intent::IDLE;
            }
            self.loot_job.lid = None;
        }
        let gained = self.goal.map_or(0, |a| a.gained);
        if gained > 0 {
            self.end_goal(tick, Outcome::Done);
        } else {
            self.loot.failed(tick);
            self.end_goal(tick, Outcome::Failed(Why::NotFound));
        }
        Intent::IDLE
    }

    /// Walk to a container; `Some` while the walk goes on. A walk that
    /// stops getting nearer gives the container up for this run.
    fn loot_approach(
        &mut self,
        core: &mut ClientCore,
        body: &EntityState,
        spot: Spot,
        stand: f32,
        look: Intent,
        tick: u32,
    ) -> Option<Intent> {
        let here = [body.qx as f32 * POS_XZ_Q, body.qz as f32 * POS_XZ_Q];
        let d = (spot.slot.x - here[0]).hypot(spot.slot.z - here[1]);
        if d <= stand {
            self.loot_job.best = None;
            return None;
        }
        match self.loot_job.best {
            Some((best, since)) if d + POS_XZ_Q >= best => {
                if tick.wrapping_sub(since) >= 2 * NO_PROGRESS_TICKS {
                    self.loot.emptied(spot.key(), tick);
                    self.loot_job.next();
                    return Some(look);
                }
            }
            _ => self.loot_job.best = Some((d, tick)),
        }
        let mut intent = look;
        match self
            .route
            .to(core, body, spot.at(), stand * 0.8, false, tick)
        {
            Step::Walk { yaw, jump, .. } => {
                intent.travel = Some(yaw);
                if jump {
                    intent.buttons |= sim_core::input::BTN_JUMP;
                }
            }
            _ => intent.travel = Some(yaw_toward(spot.slot.x - here[0], spot.slot.z - here[1])),
        }
        Some(intent)
    }

    /// A barrel: swung at like a node with whatever is in hand until it
    /// breaks, then its stacks are picked up.
    fn loot_barrel(
        &mut self,
        core: &mut ClientCore,
        body: &EntityState,
        spot: Spot,
        tick: u32,
    ) -> Intent {
        let target = Target {
            cx: spot.cx,
            cz: spot.cz,
            slot: spot.slot,
        };
        if core.harvested.contains(target.key()) {
            // Broken, by these swings or somebody's: what lies there.
            self.loot.stats.barrels += 1;
            self.loot.emptied(target.key(), tick);
            self.loot_job.smashed = Some((spot, tick));
            self.loot_job.swinging = None;
            return Intent::IDLE;
        }
        let sel = self
            .loadout
            .on_belt(core, Role::Melee)
            .map(|(slot, _)| slot)
            .or_else(|| best_tool(core, Kind::Wood))
            .or_else(|| best_tool(core, Kind::Stone));
        let mut intent = Intent {
            look: Look::Point(aim_point(body, &spot.slot)),
            sel,
            ..Intent::IDLE
        };
        let (yaw, pitch, _) = aim(body, &spot.slot);
        if !swing_reaches(core, body, yaw, pitch, target) {
            self.loot_job.swinging = None;
            if let Some(walk) = self.loot_approach(core, body, spot, REACH_M, intent, tick) {
                return walk;
            }
            // Near it, and still no swing lands: a step in, for a while.
            let since = *self.loot_job.close.get_or_insert(tick);
            if tick.wrapping_sub(since) >= SMASH_TICKS {
                self.loot.emptied(target.key(), tick);
                self.loot_job.next();
            }
            return Intent {
                move_z: 127,
                ..intent
            };
        }
        self.loot_job.best = None;
        let since = *self.loot_job.swinging.get_or_insert(tick);
        if tick.wrapping_sub(since) >= SMASH_TICKS {
            self.loot.emptied(target.key(), tick);
            self.loot_job.next();
            return intent;
        }
        // Swing only with the view the hands hold on it, and nobody on the
        // swing's ray.
        let Some(haven) = self.haven else {
            return intent;
        };
        let (held_yaw, held_pitch) = self.hands.view();
        if self.body_on_ray(body, held_yaw).is_none()
            && visible(core, &haven, body, target)
            && swing_reaches(core, body, held_yaw, held_pitch, target)
        {
            intent.buttons = BTN_PRIMARY;
        }
        intent
    }

    /// What a broken barrel scattered: the nearest stack round it, walked
    /// up to and taken, one press each, until none is left.
    fn loot_pickup(
        &mut self,
        core: &mut ClientCore,
        body: &EntityState,
        spot: Spot,
        smashed: u32,
        tick: u32,
    ) -> Intent {
        let here = [body.qx as f32 * POS_XZ_Q, body.qz as f32 * POS_XZ_Q];
        // The stack pressed for is gone: taken.
        if let Some((id, at, n)) = self.loot_job.pick {
            if core.ground_items().iter().all(|g| g.id != id) {
                self.loot_job.pick = None;
                self.loot.stats.pickups += 1;
            } else if n >= PICK_TRIES && tick.wrapping_sub(at) >= PICK_RETRY_TICKS {
                self.loot_job.skip_item(id);
            }
        }
        let around = spot.at();
        let mut best: Option<(f32, u32, [f32; 3])> = None;
        for g in core.ground_items() {
            let p = [
                g.qx as f32 * POS_XZ_Q,
                g.qy as f32 * POS_Y_Q,
                g.qz as f32 * POS_XZ_Q,
            ];
            let d = (p[0] - here[0]).hypot(p[2] - here[1]);
            if (p[0] - around[0]).hypot(p[2] - around[1]) > PICK_AROUND_M
                || self.loot_job.skips(g.id)
                || best.is_some_and(|(b, ..)| d >= b)
            {
                continue;
            }
            if !loot::room_for(core, g.item) {
                // No room for it: left lying.
                self.loot_job.skip_item(g.id);
                continue;
            }
            best = Some((d, g.id, p));
        }
        let Some((d, id, p)) = best else {
            if tick.wrapping_sub(smashed) < GROUND_WAIT_TICKS {
                // The stacks' word comes after the barrel's.
                return Intent {
                    look: Look::Point([around[0], spot.slot.y, around[1]]),
                    ..Intent::IDLE
                };
            }
            self.loot_job.next();
            return Intent::IDLE;
        };
        let look = Intent {
            look: Look::Point(p),
            ..Intent::IDLE
        };
        if d > PICK_STAND_M {
            let mut intent = look;
            intent.travel = Some(
                match self
                    .route
                    .to(core, body, [p[0], p[2]], PICK_STAND_M * 0.8, false, tick)
                {
                    Step::Walk { yaw, .. } => yaw,
                    _ => yaw_toward(p[0] - here[0], p[2] - here[1]),
                },
            );
            return intent;
        }
        let (due, n) = match self.loot_job.pick {
            Some((was, at, n)) if was == id => (tick.wrapping_sub(at) >= PICK_RETRY_TICKS, n + 1),
            _ => (true, 1),
        };
        if due && self.queue(|b| protocol::encode_action_pickup(id, b)) {
            self.loot_job.pick = Some((id, tick, n));
        }
        look
    }

    /// A crate or a cache: eyes on it, `E` to open it, every stack moved
    /// into the pack with its panel open, then shut.
    fn loot_crate(
        &mut self,
        core: &mut ClientCore,
        body: &EntityState,
        spot: Spot,
        tick: u32,
    ) -> Intent {
        let key = spot.key();
        let look = Intent {
            look: Look::Point(aim_point(body, &spot.slot)),
            ..Intent::IDLE
        };
        let open = core.cont_kind == CONT_WORLD && core.cont_handle == key;
        let late = |at: u32| tick.wrapping_sub(at) >= VERDICT_SECS * TICK_HZ;
        match self.loot_job.lid {
            None | Some(Lid::Aiming(_)) => {
                if let Some(walk) = self.loot_approach(core, body, spot, OPEN_STAND_M, look, tick) {
                    self.loot_job.lid = None;
                    return walk;
                }
                // A storey up or down (the ziggurat's rooms stand over one
                // another, and the eyes' line can miss a floor): not this
                // one's to open from here.
                if (spot.slot.y - body.qy as f32 * POS_Y_Q).abs() > loot::SAME_FLOOR_M {
                    self.loot.emptied(key, tick);
                    self.loot_job.next();
                    return Intent::IDLE;
                }
                let since = match self.loot_job.lid {
                    Some(Lid::Aiming(at)) => at,
                    _ => {
                        self.loot_job.lid = Some(Lid::Aiming(tick));
                        tick
                    }
                };
                // Only what the human's `E` would open from here: a crate
                // round a corner, or one a wall stands in front of, is not
                // taken on trust because the walk ended near it.
                let (yaw, pitch) = self.hands.view();
                let picks = open_reaches(core, body, yaw, pitch, key);
                if tick.wrapping_sub(since) >= home::HOLD_TICKS
                    && self.hands.settled()
                    && picks
                    && self.queue(|buf| protocol::encode_action_container(CONT_WORLD, key, buf))
                {
                    self.loot_job.lid = Some(Lid::Opening(tick));
                    self.loot_job.fresh = false;
                    self.loot.stats.opened += 1;
                } else if !picks && late(since) {
                    // Aimed this long and `E` still would not take it: a
                    // try spent, and the walk up again.
                    self.loot_job.tries += 1;
                    self.loot_job.lid = None;
                    if self.loot_job.tries >= LID_TRIES {
                        self.loot.emptied(key, tick);
                        self.loot_job.next();
                    }
                }
                return look;
            }
            Some(Lid::Opening(at)) => {
                if self.loot_job.fresh && open {
                    self.loot_job.lid = Some(Lid::Open);
                } else if late(at) {
                    self.loot_job.tries += 1;
                    self.loot_job.lid = None;
                    if self.loot_job.tries >= LID_TRIES {
                        self.loot.emptied(key, tick);
                        self.loot_job.next();
                    }
                    return look;
                } else {
                    return look;
                }
            }
            Some(Lid::Moving(at, count)) => match self.loot_job.moved.take() {
                Some(refused) => {
                    if refused {
                        self.loot_job.tries += 1;
                    } else {
                        self.loot_job.taken += u32::from(count);
                        if let Some(a) = self.goal.as_mut() {
                            a.gained = a.gained.saturating_add(u32::from(count));
                        }
                    }
                    self.loot_job.lid = Some(Lid::Settling(tick));
                    return look;
                }
                None if late(at) => {
                    self.loot_job.tries += 1;
                    self.loot_job.lid = Some(Lid::Open);
                }
                None => return look,
            },
            Some(Lid::Settling(at)) => {
                if tick.wrapping_sub(at) < LID_SETTLE_TICKS {
                    return look;
                }
                self.loot_job.lid = Some(Lid::Open);
            }
            Some(Lid::Open) => {}
        }
        if !open {
            // Shut by the server: out of reach, or gone.
            self.loot.emptied(key, tick);
            self.loot_job.next();
            return look;
        }
        let next = (self.loot_job.tries < LID_TRIES)
            .then(|| loot::take_plan(core))
            .flatten();
        match next {
            Some((from, to, count)) => {
                if self.queue(|buf| {
                    protocol::encode_action_move(key, CONT_WORLD, from, CONT_SELF, to, count, buf)
                }) {
                    self.loot_job.lid = Some(Lid::Moving(tick, count));
                    self.loot_job.moved = None;
                    self.loot.stats.moves += 1;
                }
            }
            None => {
                // Empty, or no room for the rest: shut it.
                if self.queue(|buf| protocol::encode_action_container(CONT_SELF, 0, buf)) {
                    self.loot.emptied(key, tick);
                    self.loot_job.next();
                }
            }
        }
        look
    }

    fn set_craft(&mut self, step: CraftStep) {
        if let Some(a) = self.goal.as_mut() {
            a.craft = step;
        }
    }

    fn eat(&mut self, core: &mut ClientCore, tick: u32) {
        self.stats.phase = Phase::Eating;
        match self.take_verdict(tick, VERDICT_SECS * TICK_HZ) {
            Err(()) => {
                self.end_goal(tick, Outcome::Failed(Why::NoAnswer));
                return;
            }
            Ok(Some(Verdict::Ok)) => {
                if let Some(a) = self.goal.as_mut() {
                    a.gained += 1;
                }
            }
            Ok(Some(Verdict::Full)) => {
                self.end_goal(tick, Outcome::Done);
                return;
            }
            Ok(Some(_)) => {}
            Ok(None) if self.awaiting.is_some() => return,
            Ok(None) => {}
        }
        if core.max_food == 0 || pct(core.food, core.max_food) >= METER_TARGET_PCT {
            self.end_goal(tick, Outcome::Done);
            return;
        }
        let Some(slot) = food_slot(core, &self.memory.food, false) else {
            let gained = self.goal.map_or(0, |a| a.gained);
            let outcome = if gained > 0 {
                Outcome::Done
            } else {
                Outcome::Failed(Why::NoFood)
            };
            self.end_goal(tick, outcome);
            return;
        };
        self.consume(core, slot, tick);
    }

    /// Use meds until health, with what those used are still delivering,
    /// reaches [`HEAL_TARGET_PCT`]: a bandage heals over seconds, and a
    /// second one only stretches the first (`survival.rs`). While the bar
    /// is still filling to the target, wait on it rather than waste one.
    fn heal(&mut self, core: &mut ClientCore, tick: u32) {
        self.stats.phase = Phase::Healing;
        self.medic = false;
        match self.dose(core, tick, HEAL_TARGET_PCT, false) {
            Dosed::Failed(why) => self.end_goal(tick, Outcome::Failed(why)),
            Dosed::Enough => self.end_goal(tick, Outcome::Done),
            Dosed::NoMeds => {
                let outcome = if self.goal.is_some_and(|a| a.gained > 0) {
                    Outcome::Done
                } else {
                    Outcome::Failed(Why::NoMeds)
                };
                self.end_goal(tick, outcome);
            }
            Dosed::Waiting | Dosed::Took => {}
        }
    }

    /// One step of taking meds toward `target_pct` of health: judge the
    /// last one's answer, wait on the bar while what was taken is still
    /// coming, else take the best med there is — or, for a `reflex` top-up
    /// at half health or better, the least, and never the best of them (a
    /// medkit is for a wound, not a scratch).
    fn dose(&mut self, core: &ClientCore, tick: u32, target_pct: u32, reflex: bool) -> Dosed {
        self.watch_dose(core, tick);
        // A bandage answers when its wrap lands (`survival::begin_use`).
        let wrap = match self.awaiting {
            Some((Pending::Consume { item, .. }, _)) => {
                u32::from(core.catalog.row(item as usize).use_ticks)
            }
            _ => 0,
        };
        let verdict = self.take_verdict(tick, VERDICT_SECS * TICK_HZ + wrap);
        match verdict {
            Ok(Some(Verdict::Ok)) => {
                if let Some(a) = self.goal.as_mut().filter(|a| a.goal == Goal::Heal) {
                    a.gained += 1;
                }
                if let Some(d) = self.dose.as_mut() {
                    d.sent = 0;
                    d.rose = tick;
                }
            }
            Err(()) | Ok(Some(_)) => {
                // Not taken, or not known to be: that med is not coming.
                if let Some(d) = self.dose.as_mut() {
                    d.rem = d.rem.saturating_sub(d.sent);
                    d.sent = 0;
                }
                return Dosed::Failed(if verdict.is_err() {
                    Why::NoAnswer
                } else {
                    Why::Refused
                });
            }
            Ok(None) if self.awaiting.is_some() => return Dosed::Waiting,
            Ok(None) => {}
        }
        let max = u32::from(core.hp_max) * target_pct;
        if core.hp_max == 0 || u32::from(core.hp) * 100 >= max {
            return Dosed::Enough;
        }
        let owed = self.dose.map_or(0, |d| u32::from(d.hp) + u32::from(d.rem));
        if owed * 100 >= max {
            return Dosed::Waiting;
        }
        let pick = if reflex && u32::from(core.hp) * 2 >= u32::from(core.hp_max) {
            self.loadout
                .least(core, Role::Meds)
                .filter(|&(_, _, best)| !best)
                .map(|(slot, item, _)| (slot, item))
        } else {
            self.loadout.best(core, Role::Meds)
        };
        let Some((slot, item)) = pick else {
            return Dosed::NoMeds;
        };
        let heal = self.book.page(item).heal;
        self.consume(core, slot, tick);
        if !matches!(self.awaiting, Some((Pending::Consume { .. }, _))) {
            return Dosed::Waiting;
        }
        let d = self.dose.get_or_insert(Dose {
            hp: core.hp,
            rem: 0,
            sent: 0,
            rose: tick,
        });
        d.rem = d.rem.saturating_add(heal);
        d.sent = heal;
        d.rose = tick;
        Dosed::Took
    }

    /// The bandage a player puts on between everything else: below
    /// [`REFLEX_HEAL_PCT`], with nobody near enough and in sight, under
    /// any goal that is not itself using the action lane's answers; up to
    /// [`HEAL_TARGET_PCT`] with what is still coming counted.
    fn medic(&mut self, core: &ClientCore, tick: u32, calm: bool) {
        let owns_lane = matches!(
            self.goal(),
            Some(Goal::Heal | Goal::Eat | Goal::Drink | Goal::Craft(_))
        );
        if owns_lane {
            self.medic = false;
            return;
        }
        let mine = self.medic && matches!(self.awaiting, Some((Pending::Consume { .. }, _)));
        if !mine && (self.awaiting.is_some() || self.outbox.is_some()) {
            return;
        }
        let low = u32::from(core.hp) * 100 < u32::from(core.hp_max) * REFLEX_HEAL_PCT;
        if !mine && (!calm || !low) {
            // Nothing to judge and nothing to take; the pool still drains.
            self.watch_dose(core, tick);
            return;
        }
        match self.dose(core, tick, HEAL_TARGET_PCT, true) {
            Dosed::Took => {
                self.medic = true;
                self.stats.reflex_heals += 1;
            }
            Dosed::Waiting if self.awaiting.is_some() => {}
            _ => self.medic = false,
        }
    }

    /// Keep the gun in hand loaded: at once when it runs dry, and topped
    /// up when things are quiet. Blind, as a player presses the key: the
    /// server says no to a full cylinder or an empty pack.
    fn reload(&mut self, core: &ClientCore, tick: u32, quiet: bool) {
        let held = core.inv[usize::from(self.sel)];
        if held.count == 0 {
            return;
        }
        let r = self.book.page(held.item).ranged;
        if r.magazine == 0 || r.round == sim_core::gather::NO_ITEM {
            return;
        }
        let (loaded, _) = core.mag();
        let dry = loaded == 0;
        let short = loaded < r.magazine;
        let spare = count_item(core, r.round) > 0;
        let due = self.reload_at.is_none_or(|at| {
            tick.wrapping_sub(at) >= u32::from(r.reload_ticks) + RELOAD_RETRY_TICKS
        });
        if spare && due && (dry || (quiet && short)) && self.queue(protocol::encode_action_reload) {
            self.reload_at = Some(tick);
            self.stats.reloads += 1;
        }
    }

    /// A bandage of mine is being wrapped on (`survival::begin_use`), by
    /// its own clock: a goal that starts or ends mid-wrap drops the answer
    /// it was waiting on, and the hands must still hold still.
    fn wrapping(&self, tick: u32) -> bool {
        self.wrap_until
            .is_some_and(|until| (until.wrapping_sub(tick) as i32) > 0)
    }

    /// Kit up between everything else, the way a player does when nothing
    /// presses: wear the best armour the pack holds for each slot (a move
    /// into the wear container, by the catalog's wear slot), and put a
    /// better weapon, med or tool on the belt (`agent::loadout`'s belt
    /// policy). One move a second at most, while the action lane is free,
    /// nobody is about, and the goal in hand is not one waiting on a move's
    /// answer of its own.
    fn dress(&mut self, core: &ClientCore, tick: u32, calm: bool) {
        // Not under a wrap: moving the stack would put it away unspent.
        if self.wrapping(tick) {
            return;
        }
        // Mid-walk through the doors a use or a code may go out any frame,
        // and a move in the outbox would take its place.
        let quiet = !self.builder.passing()
            && match self.goal() {
                None
                | Some(
                    Goal::Explore
                    | Goal::GatherWood
                    | Goal::GatherStone
                    | Goal::GatherOre
                    | Goal::Forage
                    | Goal::GatherHemp
                    | Goal::Wait
                    | Goal::GoHome,
                ) => true,
                Some(Goal::Build) => self.builder.at_checkpoint(),
                _ => false,
            };
        let due = self
            .dressed_at
            .is_none_or(|at| tick.wrapping_sub(at) >= DRESS_TICKS);
        if !calm
            || !quiet
            || !due
            || self.awaiting.is_some()
            || self.outbox.is_some()
            || !self.loadout.ready()
        {
            return;
        }
        let (from, to_kind, to, count) = match wear_move(core) {
            Some((from, to)) => (from, CONT_WEAR, to, 1),
            None => {
                let belt = BELT.iter().find_map(|&role| {
                    let (slot, item) = self.loadout.best(core, role)?;
                    (slot >= HOTBAR_SLOTS)
                        .then(|| self.loadout.belt_move(core, item))
                        .flatten()
                });
                let Some((from, to, count)) = belt else {
                    self.discard(core, tick);
                    return;
                };
                (from, CONT_SELF, to, count)
            }
        };
        if self
            .queue(|buf| protocol::encode_action_move(0, CONT_SELF, from, to_kind, to, count, buf))
        {
            self.dressed_at = Some(tick);
            self.stats.dressed += 1;
        }
    }

    /// Throw away kit the pack has outgrown (`loadout::outclassed`): a
    /// stone hatchet beside a metal one, a bow beside a crossbow. Kept, it
    /// fills the pack and then the box, and the next thing the base makes
    /// has nowhere to go: the second bench's gunpowder waited on a slot for
    /// good (2026-10-22). Last slot first, so the belt goes last.
    fn discard(&mut self, core: &ClientCore, tick: u32) {
        let name =
            |item: u16| std::str::from_utf8(core.catalog.name(usize::from(item))).unwrap_or("");
        let held = |better: &str| {
            core.inv[..INV_SLOTS]
                .iter()
                .any(|s| s.count > 0 && name(s.item) == better)
        };
        let Some(slot) = (0..INV_SLOTS).rev().find(|&i| {
            let s = core.inv[i];
            s.count > 0 && crate::agent::loadout::outclassed(name(s.item), held)
        }) else {
            return;
        };
        let count = core.inv[slot].count;
        if self.queue(|buf| protocol::encode_action_drop(slot as u8, count, buf)) {
            self.dressed_at = Some(tick);
            self.stats.discarded += 1;
        }
    }

    /// The shooting weapon for a fight: the best on the belt with rounds
    /// in the pack (or in it, for the gun in hand), how many rounds, and
    /// what is loaded when the readout is for it.
    fn shooter(&self, core: &ClientCore) -> (Option<(u8, u16)>, u32, Option<u16>) {
        let mut best: Option<(usize, u8, u16, u32, Option<u16>)> = None;
        for slot in 0..HOTBAR_SLOTS {
            let s = core.inv[slot];
            if s.count == 0 {
                continue;
            }
            let Some(rank) = self.loadout.rank(Role::Ranged, s.item) else {
                continue;
            };
            let r = self.book.page(s.item).ranged;
            let rounds = if r.round == sim_core::gather::NO_ITEM {
                0
            } else {
                count_item(core, r.round)
            };
            let (mag, cap) = core.mag();
            let loaded = (slot == usize::from(self.sel) && cap > 0).then_some(mag);
            if rounds == 0 && loaded.is_none_or(|n| n == 0) {
                continue;
            }
            if best.is_none_or(|b| rank < b.0) {
                best = Some((rank, slot as u8, s.item, rounds, loaded));
            }
        }
        match best {
            Some((_, slot, item, rounds, loaded)) => (Some((slot, item)), rounds, loaded),
            None => (None, 0, None),
        }
    }

    /// Down: crawl for the nearest trunk or rock away from the blow while
    /// it is fresh, else straight away from it; once there, or with the
    /// blow long past, lie still.
    fn crawl(&mut self, core: &mut ClientCore, body: &EntityState, tick: u32) -> Intent {
        let Some((_, away)) = self
            .last_hurt
            .filter(|(at, _)| tick.wrapping_sub(*at) < FLEE_TICKS)
        else {
            return Intent::IDLE;
        };
        let me = [body.qx as f32 * POS_XZ_Q, body.qz as f32 * POS_XZ_Q];
        let due = self.crawl_to.is_none_or(|(spot, at)| {
            spot.is_none() && tick.wrapping_sub(at) >= combat::COVER_RETHINK_TICKS
        });
        if due {
            // The blow's author is somewhere back along its bearing. Only
            // cover that lies away from the blow, not past it.
            let (bx, bz) = yaw_dir(away.wrapping_add(1 << 15));
            let threat = [me[0] + bx * 6.0, me[1] + bz * 6.0];
            let spot = cover::find(core, me, threat, CRAWL_COVER_M, None, Some(away));
            self.crawl_to = Some((spot, tick));
        }
        match self.crawl_to.and_then(|(spot, _)| spot) {
            Some(spot) => match self.route.to(core, body, spot, 0.5, false, tick) {
                Step::Walk { yaw, .. } => Intent::walk(yaw),
                Step::Wait => Intent::walk(yaw_toward(spot[0] - me[0], spot[1] - me[1])),
                Step::Arrived | Step::Blocked => Intent::IDLE,
            },
            None => Intent::walk(away),
        }
    }

    /// Read the health bar against the heal pool: what it rose by came out
    /// of the pool. A full bar, an empty pool or a bar that stopped rising
    /// has nothing more coming.
    fn watch_dose(&mut self, core: &ClientCore, tick: u32) {
        let asking = matches!(self.awaiting, Some((Pending::Consume { .. }, _)));
        let Some(d) = self.dose.as_mut() else {
            return;
        };
        if !asking {
            // Its goal ended before the answer came: count it as taken,
            // and let a still bar say otherwise.
            d.sent = 0;
        }
        if core.hp > d.hp {
            d.rem = d.rem.saturating_sub(core.hp - d.hp);
            d.rose = tick;
        }
        d.hp = core.hp;
        let done =
            d.rem == 0 || core.hp >= core.hp_max || tick.wrapping_sub(d.rose) >= HEAL_STALL_TICKS;
        if d.sent == 0 && done {
            self.dose = None;
        }
    }

    fn consume(&mut self, core: &ClientCore, slot: usize, tick: u32) {
        let item = core.inv[slot].item;
        if self.queue(|buf| protocol::encode_action_consume(slot as u8, buf)) {
            let wrap = u32::from(core.catalog.row(item as usize).use_ticks);
            if wrap > 0 {
                self.wrap_until = Some(tick.wrapping_add(wrap + TICK_HZ));
            }
            self.awaiting = Some((
                Pending::Consume {
                    item,
                    food: core.food,
                    water: core.water,
                },
                tick,
            ));
            self.verdict = None;
            self.memory.food.tried.set(item);
        }
    }

    fn drink(&mut self, core: &mut ClientCore, body: &EntityState, tick: u32) -> Intent {
        let Some(active) = self.goal else {
            return Intent::IDLE;
        };
        self.stats.phase = Phase::Drinking;
        match self.take_verdict(tick, VERDICT_SECS * TICK_HZ) {
            Err(()) => {
                self.end_goal(tick, Outcome::Failed(Why::NoAnswer));
                return Intent::IDLE;
            }
            Ok(Some(Verdict::Ok)) => {
                if let Some(a) = self.goal.as_mut() {
                    a.gained += 1;
                }
            }
            Ok(Some(Verdict::Full)) => {
                self.end_goal(tick, Outcome::Done);
                return Intent::IDLE;
            }
            Ok(Some(_)) => {}
            Ok(None) if self.awaiting.is_some() => return Intent::IDLE,
            Ok(None) => {}
        }
        if core.max_water == 0 || pct(core.water, core.max_water) >= METER_TARGET_PCT {
            self.end_goal(tick, Outcome::Done);
            return Intent::IDLE;
        }
        // Food the eat verb has seen restore water comes first: it is free.
        if let Some(slot) = food_slot(core, &self.memory.food, true) {
            self.consume(core, slot, tick);
            return Intent::IDLE;
        }
        // No water to drink, or too hurt for salt water: a food never tried
        // is tasted (`taste_slot`), which costs no health.
        let x = body.qx as f32 * POS_XZ_Q;
        let z = body.qz as f32 * POS_XZ_Q;
        let (seed, _) = core.island();
        let reach = water_in_reach(seed, x, z);
        // A lake in sight beats the sea at hand: the walk is free, the sea's
        // every mouthful costs health (the water pass).
        let lake_seen = self.water_yaw.is_some() && !self.senses.water_salt;
        let here = reach == Some(false) || (reach == Some(true) && !lake_seen);
        let salt = if here {
            reach == Some(true)
        } else {
            self.senses.water_salt
        };
        let hurt = salt && core.hp_max > 0 && pct(core.hp, core.hp_max) <= SEA_MIN_HP_PCT;
        if hurt || (reach.is_none() && self.water_yaw.is_none()) {
            if let Some(slot) = taste_slot(core, &self.memory.food) {
                self.consume(core, slot, tick);
                return Intent::IDLE;
            }
        }
        if hurt {
            let outcome = if active.gained > 0 {
                Outcome::Done
            } else {
                Outcome::Failed(Why::TooHurt)
            };
            self.end_goal(tick, outcome);
            return Intent::IDLE;
        }
        // The sea is drunk only back up to half, not to the lake's mark.
        if salt && pct(core.water, core.max_water) >= SEA_TARGET_PCT {
            self.end_goal(tick, Outcome::Done);
            return Intent::IDLE;
        }
        if here {
            if self.queue(protocol::encode_action_drink) {
                self.awaiting = Some((Pending::Drink, tick));
                self.verdict = None;
            }
            return Intent::IDLE;
        }
        // Out of water sources: whatever was drunk still counts as done.
        let dry = if active.gained > 0 {
            Outcome::Done
        } else {
            Outcome::Failed(Why::NoWater)
        };
        let Some(yaw) = self.water_yaw else {
            self.end_goal(tick, dry);
            return Intent::IDLE;
        };
        if tick.wrapping_sub(active.started) >= SEARCH_GOAL_SECS * TICK_HZ {
            self.end_goal(tick, dry);
            return Intent::IDLE;
        }
        self.stats.phase = Phase::SeekingWater;
        // The water seen when the seek began, by a route there: the
        // probe's next answer is relative to wherever the walk has got to.
        let Some(to) = self.seek.or(self.water_point) else {
            return Intent::walk(yaw);
        };
        self.seek = Some(to);
        match self.route.to(core, body, to, 1.0, false, tick) {
            step @ Step::Walk { .. } => step.walk().unwrap_or(Intent::walk(yaw)),
            _ => {
                // There, or no way there, and still not in reach: the
                // bearing the probe gives now.
                self.seek = None;
                Intent::walk(yaw)
            }
        }
    }

    /// Put a sleeping bag down near here (`agent::home::BagJob`): on the
    /// belt, in hand, eyes on the spot, then the deploy verb.
    fn place_bag(&mut self, core: &mut ClientCore, body: &EntityState, tick: u32) -> Intent {
        self.stats.phase = Phase::Bagging;
        if let Some(intent) = self.bag_in(core, body, tick) {
            return intent;
        }
        let Some(haven) = self.haven else {
            return Intent::IDLE;
        };
        let (seed, _) = core.island();
        let step = self
            .bag_job
            .step(core, seed, &haven, body, &self.hands, &mut self.home, tick);
        match step {
            Do::Go(intent) => intent,
            Do::Belt { from, to, count } => {
                if self.queue(|buf| {
                    protocol::encode_action_move(0, CONT_SELF, from, CONT_SELF, to, count, buf)
                }) {
                    self.bag_job.belt_sent(tick);
                }
                Intent::IDLE
            }
            Do::Deploy {
                row,
                cx,
                cz,
                loc,
                pose,
                intent,
            } => {
                if self.queue(|buf| {
                    protocol::encode_action_deploy(row, cx, cz, 0, LOC_PLANE, pose, buf)
                }) {
                    self.home.asked(cx, cz, 0, loc);
                    self.bag_job.deploy_sent(tick);
                }
                intent
            }
            Do::Done => {
                if let Some(a) = self.goal.as_mut() {
                    a.gained += 1;
                }
                self.end_goal(tick, Outcome::Done);
                Intent::IDLE
            }
            Do::Fail(why) => {
                if self.bag_leg != BagLeg::Here {
                    self.team_bag_failed = Some(tick);
                }
                self.end_goal(tick, Outcome::Failed(why));
                Intent::IDLE
            }
            Do::Loot(_) => Intent::IDLE,
        }
    }

    /// The way in for a bag bound for the team's base: through its doors to
    /// its cupboard, and onto the cupboard's crew with the team's code when
    /// it has a lock and the crew does not have it yet (the right code is
    /// the whole invitation, `deploy::hearth_admits`); a base it is not on
    /// the crew of refuses a bag. `None` once the floor is the next thing.
    fn bag_in(&mut self, core: &mut ClientCore, body: &EntityState, tick: u32) -> Option<Intent> {
        match self.bag_leg {
            BagLeg::Here | BagLeg::Floor => None,
            BagLeg::In => {
                let (intent, over) = self.home_step(core, body, tick);
                match over {
                    None => Some(intent),
                    Some(Outcome::Failed(why)) => {
                        self.team_bag_failed = Some(tick);
                        self.end_goal(tick, Outcome::Failed(why));
                        Some(Intent::IDLE)
                    }
                    Some(_) => {
                        self.bag_leg = BagLeg::Crew;
                        Some(Intent::IDLE)
                    }
                }
            }
            BagLeg::Crew => {
                let knock = !self.shared.crewed()
                    && !self.bag_knocked
                    && self.team_code.is_some()
                    && self.shared.hearth_locked(core);
                match self.shared.hearth().filter(|_| knock) {
                    Some(hearth) => {
                        // The code goes in at its keypad, the trip's own
                        // door walk's way, then the trip is there again.
                        self.home_job.doors.knock_at(hearth);
                        self.bag_knocked = true;
                        self.bag_leg = BagLeg::In;
                    }
                    // Not on the crew, and the code did not put it there
                    // (or there is no lock to enter one at): the floor would
                    // refuse the bag, the claim's, so it is not sent.
                    None if self.shared.hearth().is_some()
                        && !self.shared.crewed()
                        && (self.bag_knocked || self.shared.hearth_unlocked(core)) =>
                    {
                        self.team_bag_failed = Some(tick);
                        self.end_goal(tick, Outcome::Failed(Why::Locked));
                    }
                    // On the crew, or no way to know: the floor says.
                    None => self.bag_leg = BagLeg::Floor,
                }
                Some(Intent::IDLE)
            }
        }
    }

    /// Walk back to the backpack from the last death and loot it
    /// (`agent::home::RecoverJob`); anyone dangerous in view ends it.
    fn recover(&mut self, core: &mut ClientCore, body: &EntityState, tick: u32) -> Intent {
        self.stats.phase = Phase::Returning;
        let gained = self.goal.map_or(0, |a| a.gained);
        let step = self.recover_job.step(
            core,
            body,
            &mut self.route,
            &self.hands,
            &mut self.home,
            self.senses.hostile,
            gained,
            tick,
        );
        match step {
            Do::Go(intent) => intent,
            Do::Loot(intent) => {
                if self.queue(protocol::encode_action_loot) {
                    self.recover_job.loot_sent(tick, gained);
                    self.home.stats.loots += 1;
                }
                intent
            }
            Do::Done => {
                self.home.stats.recovered += 1;
                self.end_goal(tick, Outcome::Done);
                Intent::IDLE
            }
            Do::Fail(why) => {
                self.end_goal(tick, Outcome::Failed(why));
                Intent::IDLE
            }
            Do::Belt { .. } | Do::Deploy { .. } => Intent::IDLE,
        }
    }

    /// Work on the base (`agent::build::Builder`): the plot, then the next
    /// milestone's ops from the stand spot in the core, one verdict each.
    fn build_home(&mut self, core: &mut ClientCore, body: &EntityState, tick: u32) -> Intent {
        self.stats.phase = Phase::Building;
        let Some(haven) = self.haven else {
            return Intent::IDLE;
        };
        let (seed, _) = core.island();
        let at = |r: Option<(Target, u32)>| r.map(|(t, _)| [t.slot.x, t.slot.z]);
        let (wood, stone) = (
            at(self.recall[Kind::Wood as usize]),
            at(self.recall[Kind::Stone as usize]),
        );
        let act = self.builder.step(
            core,
            seed,
            &haven,
            body,
            &self.hands,
            &mut self.route,
            &self.seen,
            &self.home,
            wood,
            stone,
            tick,
        );
        self.act(act, tick)
    }

    /// Walk home and in through its doors.
    fn go_home(&mut self, core: &mut ClientCore, body: &EntityState, tick: u32) -> Intent {
        self.stats.phase = Phase::GoingHome;
        let Some(haven) = self.haven else {
            return Intent::IDLE;
        };
        let (seed, _) = core.island();
        let act = self.builder.pass(
            core,
            seed,
            &haven,
            body,
            &self.hands,
            &mut self.route,
            Way::In,
            tick,
        );
        self.act(act, tick)
    }

    /// A visit home (`agent::stash::StashJob`): in through the doors, the
    /// cupboard fed if it runs low, the box opened and the pack sorted into
    /// it under the belt loadout, the panel shut.
    fn stash(&mut self, core: &mut ClientCore, body: &EntityState, tick: u32) -> Intent {
        self.stats.phase = Phase::Stashing;
        let Some(haven) = self.haven else {
            return Intent::IDLE;
        };
        let (seed, _) = core.island();
        let chore = self.stash_job.step(
            core,
            seed,
            &haven,
            body,
            &self.hands,
            &mut self.route,
            &mut self.builder,
            &self.home,
            &self.book,
            tick,
        );
        let (sent, intent) = match chore {
            Chore::Walk(act) => return self.act(act, tick),
            Chore::Go(intent) => return intent,
            Chore::Feed { at, intent } => (
                self.queue(|buf| protocol::encode_action_feed(at.cx, at.cz, at.level, buf)),
                intent,
            ),
            Chore::Open { key, intent } => (
                self.queue(|buf| protocol::encode_action_container(CONT_BOX, key, buf)),
                intent,
            ),
            Chore::Move {
                key,
                transfer,
                intent,
            } => {
                let (cont, from_kind, from, to_kind, to, count) = match transfer {
                    Transfer::Take { from, to, count } => {
                        (key, CONT_BOX, from, CONT_SELF, to, count)
                    }
                    Transfer::Put { from, to, count } => {
                        (key, CONT_SELF, from, CONT_BOX, to, count)
                    }
                    Transfer::Belt { from, to, count } => {
                        (0, CONT_SELF, from, CONT_SELF, to, count)
                    }
                };
                (
                    self.queue(|buf| {
                        protocol::encode_action_move(cont, from_kind, from, to_kind, to, count, buf)
                    }),
                    intent,
                )
            }
            Chore::Close(intent) => (
                self.queue(|buf| protocol::encode_action_container(CONT_SELF, 0, buf)),
                intent,
            ),
            Chore::Done => {
                // The shut panel (or a last move) may still wait on its
                // pace; ending the goal now would drop it.
                if self.outbox.is_none() {
                    self.stash_done(tick);
                }
                return Intent::IDLE;
            }
            Chore::Fail(why) => {
                if why == Why::Blocked {
                    self.home.stash_blocked(tick);
                } else {
                    self.home.stash_failed(tick);
                }
                self.end_goal(tick, Outcome::Failed(why));
                return Intent::IDLE;
            }
        };
        if sent {
            self.stash_job.sent(tick);
        }
        intent
    }

    /// The device a work is done at that is worth walking to from here:
    /// its own fire near, or a recycler within a longer walk, its own first.
    /// Any recycler standing will do: the one it put down is forgotten with
    /// the session, and taking it for gone made it build another. A
    /// research table is THE GATE's, once in sight: a table in somebody's
    /// base is behind their walls, and what is in it is theirs.
    fn device_near(&self, core: &ClientCore, work: Work, x: f32, z: f32) -> Option<(u16, u16)> {
        let (own, within) = match work {
            Work::Cook => (self.fire, oven::KEEP_M),
            Work::Shop => return self.shop_machine(core),
            Work::Research { .. } => (oven::town_table(core), f32::MAX),
            Work::Recycle => (self.recycler, oven::SEEN_NEAR_M),
        };
        oven::nearest(core, work, own.into_iter(), x, z, within).or_else(|| {
            (work == Work::Recycle)
                .then(|| oven::nearest(core, work, oven::standing(core, work), x, z, within))
                .flatten()
        })
    }

    /// Where the crew's vending machine goes, with home's middle known.
    fn shop_spot(&self, core: &ClientCore) -> Option<[f32; 2]> {
        self.patrol_centre()
            .map(|home| crate::agent::shop::spot(core, home))
    }

    /// The crew's vending machine: the one it put down, else one standing
    /// at the crew's spot (a crewmate's, or its own from before a
    /// reconnect; its view says whether this body may keep it).
    fn shop_machine(&self, core: &ClientCore) -> Option<(u16, u16)> {
        let [sx, sz] = self.shop_spot(core)?;
        if let Some(own) = self.shop.filter(|&d| oven::stands(core, Work::Shop, d)) {
            return Some(own);
        }
        let cells = core
            .deploys
            .entries()
            .iter()
            .filter(|d| d.level == 0 && d.loc == LOC_PLANE)
            .map(|d| (d.cx, d.cz));
        oven::nearest(
            core,
            Work::Shop,
            cells,
            sx,
            sz,
            crate::agent::shop::SPOT_NEAR_M,
        )
    }

    /// Is a walk home not worth another try yet?
    fn home_held(&self, tick: u32) -> bool {
        self.home_failed
            .is_some_and(|at| tick.wrapping_sub(at) < GO_HOME_RETRY_TICKS)
    }

    /// Did a walk home, or a visit to the box, fail to reach home from
    /// about here a moment ago?
    fn home_unreachable(&self, tick: u32) -> bool {
        self.home_stuck.is_some_and(|(at, [x, z])| {
            let near = (x - self.here[0]).hypot(z - self.here[1]) <= HOME_STUCK_NEAR_M;
            tick.wrapping_sub(at)
                < if near {
                    HOME_STUCK_TICKS
                } else {
                    GO_HOME_RETRY_TICKS
                }
        })
    }

    fn oven_failed(&mut self, tick: u32) {
        match self.oven_job.work() {
            Work::Cook => self.cook_failed = Some(tick),
            Work::Shop => self.shop_failed = Some(tick),
            Work::Research { .. } => self.research_failed = Some(tick),
            Work::Recycle => self.recycle_failed = Some(tick),
        }
    }

    /// A research (`agent::oven`, `Work::Research`): paper it holds and
    /// has not read is read, here; else the cheapest found thing it has no
    /// blueprint for goes to THE GATE's research table with its junk, the
    /// paper is taken off and read.
    fn research(&mut self, core: &mut ClientCore, body: &EntityState, tick: u32) -> Intent {
        self.stats.phase = Phase::Recycling;
        if self.paper_read {
            if self.outbox.is_none() {
                self.end_goal(tick, Outcome::Done);
            }
            return Intent::IDLE;
        }
        if !matches!(self.oven_job.work(), Work::Research { .. }) {
            if let Some(slot) = oven::unread(core) {
                self.paper_read = self.queue(|buf| protocol::encode_action_research(slot, buf));
                return Intent::IDLE;
            }
            let tier = self.builder.stations(core).tier;
            let left = self
                .research_left
                .take()
                .and_then(|sample| oven::research_of(core, sample));
            let Some(work) = left.or_else(|| oven::research(core, tier)) else {
                self.research_failed = Some(tick);
                self.end_goal(tick, Outcome::Failed(Why::MissingInputs));
                return Intent::IDLE;
            };
            self.oven_job = DeviceJob::new(work);
        }
        if let Work::Research { sample, .. } = self.oven_job.work() {
            self.sample_in_table = (count_item(core, sample) == 0
                && oven::paper_of(core, sample).is_none())
            .then_some(sample);
        }
        // To the table until it is in sight (the deployables' interest),
        // then the job walks to it and works it.
        let (x, z) = (body.qx as f32 * POS_XZ_Q, body.qz as f32 * POS_XZ_Q);
        if self.device_near(core, self.oven_job.work(), x, z).is_none() {
            let far = oven::town_table(core)
                .map(|(cx, cz)| cell_center(cx, cz))
                .filter(|&(mx, mz)| (mx - x).hypot(mz - z) > TABLE_SIGHT_M);
            let Some((mx, mz)) = far else {
                // At THE GATE, and no table there to be seen; or no town.
                self.research_failed = Some(tick);
                self.end_goal(tick, Outcome::Failed(Why::NotFound));
                return Intent::IDLE;
            };
            return match self
                .route
                .to(core, body, [mx, mz], TABLE_SIGHT_M - 2.0, false, tick)
            {
                step @ Step::Walk { yaw, .. } => step.walk().unwrap_or(Intent::walk(yaw)),
                Step::Wait | Step::Arrived => Intent::walk(yaw_toward(mx - x, mz - z)),
                Step::Blocked => {
                    self.research_failed = Some(tick);
                    self.end_goal(tick, Outcome::Failed(Why::Stuck));
                    Intent::IDLE
                }
            };
        }
        self.tend(core, body, tick)
    }

    /// A cook at a fire or a recycle at a recycler (`agent::oven`): its own
    /// device if one stands near, else one put down here (a fire pit
    /// crafted first; a recycler is made at the bench by the craft goal),
    /// what it works on laid in, switched on, what it makes taken off, then
    /// switched off and the panel shut.
    fn tend(&mut self, core: &mut ClientCore, body: &EntityState, tick: u32) -> Intent {
        let work = self.oven_job.work();
        self.stats.phase = match work {
            Work::Cook => Phase::Cooking,
            Work::Shop => Phase::Trading,
            _ => Phase::Recycling,
        };
        let Some(haven) = self.haven else {
            return Intent::IDLE;
        };
        let (seed, _) = core.island();
        let (x, z) = (body.qx as f32 * POS_XZ_Q, body.qz as f32 * POS_XZ_Q);
        let go_to = self.device_near(core, work, x, z);
        if work == Work::Shop {
            // What the base keeps is never sold.
            let mut keep = Keep::NONE;
            for (name, units) in crate::agent::shop::RESERVE {
                if let Some(item) = crate::agent::build::item_named(core, name) {
                    keep.add(item, units);
                }
            }
            self.oven_job.keep(keep);
            // No machine yet: to the crew's spot to put it down there.
            if go_to.is_none() && self.oven_job.placed().is_none() {
                if let Some(spot) = self.shop_spot(core) {
                    if (spot[0] - x).hypot(spot[1] - z) > 1.5 {
                        return match self.route.to(core, body, spot, 1.0, false, tick) {
                            step @ Step::Walk { yaw, .. } => {
                                step.walk().unwrap_or(Intent::walk(yaw))
                            }
                            Step::Wait => Intent::walk(yaw_toward(spot[0] - x, spot[1] - z)),
                            Step::Arrived => Intent::IDLE,
                            Step::Blocked => {
                                self.oven_failed(tick);
                                self.end_goal(tick, Outcome::Failed(Why::Stuck));
                                Intent::IDLE
                            }
                        };
                    }
                }
            }
        } else {
            self.oven_job.keep(self.keep);
        }
        let builder = &self.builder;
        let step = self.oven_job.step(
            core,
            seed,
            &haven,
            body,
            &self.hands,
            &mut self.route,
            &self.book,
            go_to,
            |cx, cz| builder.near_plot(cx, cz),
            &mut self.oven_stats,
            tick,
        );
        if let Some(at) = self.oven_job.placed() {
            match work {
                Work::Cook => self.fire = Some(at),
                Work::Shop => self.shop = Some(at),
                _ => self.recycler = Some(at),
            }
        }
        let (sent, intent) = match step {
            Tend::Go(intent) => return intent,
            Tend::Craft { recipe } => (
                self.queue(|buf| protocol::encode_action_craft(recipe, 1, 0, buf)),
                Intent::IDLE,
            ),
            Tend::Belt { from, to, count } => (
                self.queue(|buf| {
                    protocol::encode_action_move(0, CONT_SELF, from, CONT_SELF, to, count, buf)
                }),
                Intent::IDLE,
            ),
            Tend::Deploy {
                row,
                cx,
                cz,
                intent,
            } => (
                self.queue(|buf| {
                    protocol::encode_action_deploy(row, cx, cz, 0, LOC_PLANE, Pose::CENTRE, buf)
                }),
                intent,
            ),
            Tend::Open { key, intent } => (
                self.queue(|buf| protocol::encode_action_container(CONT_BOX, key, buf)),
                intent,
            ),
            Tend::Move {
                key,
                into,
                from,
                to,
                count,
                intent,
            } => {
                let (from_kind, to_kind) = if into {
                    (CONT_SELF, CONT_BOX)
                } else {
                    (CONT_BOX, CONT_SELF)
                };
                (
                    self.queue(|buf| {
                        protocol::encode_action_move(key, from_kind, from, to_kind, to, count, buf)
                    }),
                    intent,
                )
            }
            Tend::Switch { at, intent } => (
                self.queue(|buf| protocol::encode_action_use(at.cx, at.cz, at.level, at.loc, buf)),
                intent,
            ),
            Tend::Offer { key, act, intent } => (
                self.queue(|buf| protocol::encode_action_shop(key, act, buf)),
                intent,
            ),
            Tend::Close(intent) => (
                self.queue(|buf| protocol::encode_action_container(CONT_SELF, 0, buf)),
                intent,
            ),
            Tend::Done => {
                // The shut panel may still wait on its pace.
                if self.outbox.is_none() {
                    if work == Work::Shop {
                        self.shop_kept = Some(tick);
                    }
                    // The paper a research made, read where it stands.
                    if let Some(slot) = oven::unread(core).filter(|_| work != Work::Recycle) {
                        if self.queue(|buf| protocol::encode_action_research(slot, buf)) {
                            self.paper_read = true;
                            return Intent::IDLE;
                        }
                    }
                    self.end_goal(tick, Outcome::Done);
                }
                return Intent::IDLE;
            }
            Tend::Fail(why) => {
                self.oven_failed(tick);
                self.end_goal(tick, Outcome::Failed(why));
                return Intent::IDLE;
            }
        };
        if sent {
            self.oven_job.sent(tick);
        }
        intent
    }

    /// Home under attack (`agent::defend`): in through its doors, the
    /// fight answered from inside, then what was damaged mended with the
    /// hammer in hand and the doors shut behind.
    fn defend(&mut self, core: &mut ClientCore, body: &EntityState, tick: u32) -> Intent {
        let Some(haven) = self.haven else {
            return Intent::IDLE;
        };
        let (seed, _) = core.island();
        let ward = self.defend_job.step(
            core,
            seed,
            &haven,
            body,
            &self.hands,
            &mut self.route,
            &mut self.builder,
            &self.home,
            self.senses.hostile,
            &mut self.defend_stats,
            tick,
        );
        self.stats.phase = match ward {
            Ward::Repair { .. } | Ward::Belt { .. } | Ward::Craft { .. } => Phase::Repairing,
            _ => Phase::Defending,
        };
        let (sent, intent) = match ward {
            Ward::Go(intent) => return intent,
            Ward::Walk(act) => return self.act(act, tick),
            Ward::Craft { recipe } => (
                self.queue(|buf| protocol::encode_action_craft(recipe, 1, 0, buf)),
                Intent::IDLE,
            ),
            Ward::Belt { from, to, count } => (
                self.queue(|buf| {
                    protocol::encode_action_move(0, CONT_SELF, from, CONT_SELF, to, count, buf)
                }),
                Intent::IDLE,
            ),
            Ward::Repair { deploy, at, intent } => (
                self.queue(|buf| {
                    protocol::encode_action_repair(deploy, at.cx, at.cz, at.level, at.loc, buf)
                }),
                intent,
            ),
            Ward::Done => {
                if self.outbox.is_none() {
                    if self.defend_job.gave_up() {
                        self.defend_failed = Some(tick);
                    }
                    // Seen through: only a new fight calls it back, and
                    // the next offer waits for a fresh look at the walls.
                    self.home.settle();
                    self.memory.alarm = false;
                    self.memory.damaged = false;
                    self.defend_let_be.remember(&self.defend_job, tick);
                    if let Some(a) = self.goal.as_mut() {
                        a.gained = a.gained.saturating_add(self.defend_job.mended);
                    }
                    self.end_goal(tick, Outcome::Done);
                }
                return Intent::IDLE;
            }
            Ward::Fail(why) => {
                self.defend_failed = Some(tick);
                self.defend_let_be.remember(&self.defend_job, tick);
                self.end_goal(tick, Outcome::Failed(why));
                return Intent::IDLE;
            }
        };
        if sent {
            self.defend_job.sent(tick, &ward);
        }
        intent
    }

    /// What this body would raid with: the satchels in the pack, and the
    /// weapon that takes most off a structure per blow.
    fn means(&self, core: &ClientCore) -> Means {
        let book = &self.book;
        let satchel = crate::agent::build::item_named(core, raid::SATCHEL_ITEM)
            .filter(|&item| book.page(item).throw.structure > 0)
            .map(|item| (item, book.page(item).throw));
        let mut melee: Option<(u16, u16, u16)> = None;
        for st in core.inv[..INV_SLOTS].iter().filter(|st| st.count > 0) {
            let m = book.page(st.item).melee;
            if m.damage > 0 && m.structure > 0 && melee.is_none_or(|(_, s, _)| m.structure > s) {
                melee = Some((st.item, m.structure, m.cadence_ticks));
            }
        }
        Means {
            satchel,
            satchels: satchel.map_or(0, |(item, _)| count_item(core, item)),
            melee,
        }
    }

    /// A raid (`agent::raid`): to the weakest face of a base worth it, in
    /// with satchels or blows, the boxes emptied into the pack.
    fn raid(&mut self, core: &mut ClientCore, body: &EntityState, tick: u32) -> Intent {
        self.stats.phase = Phase::Raiding;
        let Some(haven) = self.haven else {
            return Intent::IDLE;
        };
        let (seed, _) = core.island();
        let means = self.means(core);
        let step = self.raid_job.step(
            core,
            seed,
            &haven,
            body,
            &self.hands,
            &mut self.route,
            &mut self.bases,
            &means,
            self.combat.temperament(),
            tick,
        );
        let (sent, intent) = match step {
            Raid::Go(intent) => return intent,
            Raid::Belt { from, to, count } => (
                self.queue(|buf| {
                    protocol::encode_action_move(0, CONT_SELF, from, CONT_SELF, to, count, buf)
                }),
                Intent::IDLE,
            ),
            Raid::Throw { deploy, at, intent } => (
                self.queue(|buf| {
                    protocol::encode_action_throw(deploy, at.cx, at.cz, at.level, at.loc, buf)
                }),
                intent,
            ),
            Raid::Open { key, intent } => (
                self.queue(|buf| protocol::encode_action_container(CONT_BOX, key, buf)),
                intent,
            ),
            Raid::Take {
                key,
                from,
                to,
                count,
                intent,
            } => (
                self.queue(|buf| {
                    protocol::encode_action_move(key, CONT_BOX, from, CONT_SELF, to, count, buf)
                }),
                intent,
            ),
            Raid::Close(intent) => (
                self.queue(|buf| protocol::encode_action_container(CONT_SELF, 0, buf)),
                intent,
            ),
            Raid::Done => {
                // The shut panel may still wait on its pace.
                if self.outbox.is_none() {
                    self.raid_over(Outcome::Done, tick);
                }
                return Intent::IDLE;
            }
            Raid::Fail(why) => {
                self.raid_over(Outcome::Failed(why), tick);
                return Intent::IDLE;
            }
        };
        if sent {
            self.raid_job.sent(&mut self.bases, tick, &step);
        }
        intent
    }

    /// The raid is over: what it took is the goal's gain.
    fn raid_over(&mut self, outcome: Outcome, tick: u32) {
        if let Some(a) = self.goal.as_mut() {
            a.gained = a.gained.max(self.raid_job.taken);
        }
        if matches!(outcome, Outcome::Failed(_)) {
            self.raid_failed = Some(tick);
        }
        self.raid_let_be(tick);
        self.end_goal(tick, outcome);
    }

    /// A raid ended, however it ended: its base is let be a while.
    fn raid_let_be(&mut self, tick: u32) {
        if let Some((i, ..)) = self.raid_job.target() {
            self.bases.tried(i, tick);
        }
        self.raid_job = RaidJob::default();
    }

    fn stash_done(&mut self, tick: u32) {
        if self.stash_job.gave_up() {
            self.home.stash_failed(tick);
        }
        self.end_goal(tick, Outcome::Done);
    }

    /// Out through its own doors before a walk into the island; `None` once
    /// outside with the doors shut behind.
    fn leave(&mut self, core: &mut ClientCore, body: &EntityState, tick: u32) -> Option<Intent> {
        let haven = self.haven?;
        let (seed, _) = core.island();
        let act = self.builder.pass(
            core,
            seed,
            &haven,
            body,
            &self.hands,
            &mut self.route,
            Way::Out,
            tick,
        );
        match act {
            Act::Done => None,
            act => {
                self.stats.phase = Phase::Leaving;
                Some(self.act(act, tick))
            }
        }
    }

    /// Send what the builder asked for; a finished or failed job ends the
    /// goal.
    fn act(&mut self, act: Act, tick: u32) -> Intent {
        let sent = match act {
            Act::Go(intent) => return intent,
            Act::Craft { recipe, count } => {
                self.queue(|buf| protocol::encode_action_craft(recipe, count, 0, buf))
            }
            Act::Belt { from, to, count } => self.queue(|buf| {
                protocol::encode_action_move(0, CONT_SELF, from, CONT_SELF, to, count, buf)
            }),
            Act::Place { row, at, .. } => self.queue(|buf| {
                protocol::encode_action_place(row, at.cx, at.cz, at.level, at.loc, false, 0, buf)
            }),
            Act::Deploy {
                row, at, pose, bag, ..
            } => {
                let sent = self.queue(|buf| {
                    protocol::encode_action_deploy(row, at.cx, at.cz, at.level, at.loc, pose, buf)
                });
                if sent && bag {
                    // Home keeps the list of its bags.
                    self.home.asked(at.cx, at.cz, at.level, at.loc);
                }
                sent
            }
            Act::Upgrade { at, material, .. } => self.queue(|buf| {
                protocol::encode_action_upgrade(at.cx, at.cz, at.level, at.loc, material, buf)
            }),
            Act::Use { at, .. } => {
                self.queue(|buf| protocol::encode_action_use(at.cx, at.cz, at.level, at.loc, buf))
            }
            // The code goes straight from the builder's keeping onto the
            // wire, the keypad's way, and nowhere else.
            Act::Access { at, op, code, .. } => self.queue(|buf| {
                protocol::encode_action_access(at.cx, at.cz, at.level, at.loc, op, code.wire(), buf)
            }),
            Act::Demolish { at, .. } => self.queue(|buf| {
                protocol::encode_action_demolish(true, at.cx, at.cz, at.level, at.loc, buf)
            }),
            Act::Unlock { recipe } => self.queue(|buf| protocol::encode_action_unlock(recipe, buf)),
            Act::Read { slot } => self.queue(|buf| protocol::encode_action_research(slot, buf)),
            Act::Done => {
                if let Some(a) = self.goal.as_mut() {
                    a.gained += 1;
                }
                self.end_goal(tick, Outcome::Done);
                return Intent::IDLE;
            }
            Act::Fail(why) => {
                self.end_goal(tick, Outcome::Failed(why));
                return Intent::IDLE;
            }
        };
        if sent {
            self.builder.sent(tick);
        }
        match act {
            Act::Place { intent, .. }
            | Act::Deploy { intent, .. }
            | Act::Upgrade { intent, .. }
            | Act::Use { intent, .. }
            | Act::Access { intent, .. }
            | Act::Demolish { intent, .. } => intent,
            _ => Intent::IDLE,
        }
    }

    /// Cells of the sight window, the eyes on other bodies and — once a
    /// second — the water probe. Publishes the census when a sweep
    /// completes.
    fn perceive(&mut self, core: &mut ClientCore, body: &EntityState, tick: u32) {
        let Some(haven) = self.haven else {
            return;
        };
        let cells = self
            .scanned_at
            .map_or(1, |at| tick.wrapping_sub(at).clamp(1, SCAN_CELLS_MAX));
        self.scanned_at = Some(tick);
        for _ in 0..cells {
            self.scan_cell(core, body, &haven, tick);
        }
        let mut bodies = [Sighting::default(); 2];
        let mut humans = Sighting::default();
        // The nearest bodies that could hurt this one, as a player would
        // size them up: what is in their hands, which way, how hurt.
        let mut threats = [(f32::INFINITY, Threat::default()); SUMMARY_THREATS];
        for t in self.tracks.recent(tick, BODY_RECALL_SECS * TICK_HZ) {
            let (d, b) = relative(body, t.pos[0], t.pos[2]);
            bodies[usize::from(t.species != Species::Player)].add(d, b);
            if t.species == Species::Player && !core.is_agent(t.id) {
                humans.add(d, b);
            }
            let (arms, hp) = match t.species {
                Species::Pig => continue,
                Species::Wolf => (Arms::Teeth, u32::from(self.book.wolf().hp)),
                Species::Player => (
                    match t.held.map(|h| self.book.page(h).class) {
                        Some(Class::Melee) => Arms::Melee,
                        Some(Class::Tool) => Arms::Tool,
                        Some(Class::Ranged) => Arms::Ranged,
                        Some(Class::Throw) => Arms::Explosive,
                        _ => Arms::Unarmed,
                    },
                    u32::from(core.hp_max),
                ),
            };
            let threat = Threat {
                arms,
                range: Range::of(d),
                bearing: b,
                wounded: t.wounded || (t.dealt > 0 && t.dealt * 2 >= hp),
                aiming_at_me: t.aiming_at_me,
            };
            if let Some(i) = threats.iter().position(|(m, _)| d < *m) {
                threats.copy_within(i..SUMMARY_THREATS - 1, i + 1);
                threats[i] = (d, threat);
            }
        }
        [self.senses.players, self.senses.animals] = bodies;
        self.senses.humans = humans;
        self.senses.threats_len = threats.iter().filter(|(d, _)| d.is_finite()).count() as u8;
        self.senses.threats = threats.map(|(_, t)| t);
        self.senses.hostile = self
            .tracks
            .recent(tick, THREAT_RECALL_SECS * TICK_HZ)
            .any(|t| t.species != Species::Pig);
        let town = core.haven().town;
        self.senses.hostile_afield =
            self.tracks
                .recent(tick, THREAT_RECALL_SECS * TICK_HZ)
                .any(|t| match t.species {
                    Species::Pig => false,
                    Species::Wolf => true,
                    Species::Player => !sim_core::town::safe(&town, t.pos[0], t.pos[2]),
                });
        // Its team (wire v108): nobody's quarry, and a stranger swinging or
        // shooting near one of them is who `follow` goes for.
        let team = *core.team();
        self.tracks.befriend(&team);
        // Its society: who its crew and guild are on this island, by their
        // nametags; never a quarry, whatever team they are on.
        if let Some(soc) = self.society {
            self.kin.refresh(&soc, core, tick);
            for id in self.kin.ids() {
                self.tracks.spare(id);
            }
        }
        // Whoever is asking it onto their team, face to face, is not a
        // threat for asking: spared while the invite waits. Not a truce:
        // seen swinging or shooting lately, they are sized up like anyone.
        if team.count == 0 && team.invite != 0 {
            let recall = THREAT_RECALL_SECS * TICK_HZ;
            let peaceful = self.tracks.get(team.invite).is_none_or(|t| {
                [t.last_swing, t.last_shot]
                    .into_iter()
                    .flatten()
                    .all(|when| tick.wrapping_sub(when) >= recall)
            });
            if peaceful {
                self.tracks.spare(team.invite);
            }
        }
        let mut guard: Option<(f32, u32)> = None;
        let recall = GUARD_RECALL_SECS * TICK_HZ;
        for m in team.members() {
            if m.id == core.player_id || m.state != protocol::TEAM_AWAKE {
                continue;
            }
            let at = self
                .tracks
                .get(m.id)
                .filter(|t| t.visible)
                .map_or([f32::from(m.x_m) + 0.5, f32::from(m.z_m) + 0.5], |t| {
                    [t.pos[0], t.pos[2]]
                });
            for t in self.tracks.recent(tick, recall) {
                let fought = [t.last_swing, t.last_shot]
                    .into_iter()
                    .flatten()
                    .any(|when| tick.wrapping_sub(when) < recall);
                let d = (t.pos[0] - at[0]).hypot(t.pos[2] - at[1]);
                if t.species != Species::Pig
                    && t.visible
                    && fought
                    && d <= GUARD_M
                    && guard.is_none_or(|g| d < g.0)
                {
                    guard = Some((d, t.id));
                }
            }
        }
        self.guard = guard.map(|(_, id)| id);
        self.senses.mate_attacked = self.guard.is_some();
        // Someone to invite: a person up and in sight within reach, not an
        // agent and not on its team, while it has no team or leads one
        // with room. The nearest, as a player's N would pick.
        let room = team.count == 0
            || (team.leader == core.player_id && usize::from(team.count) < protocol::TEAM_MAX);
        // Its friend (`--friend`): who the shard says has that wallet now.
        // A new id (they came back) starts their clock again.
        if let Some(address) = self.friend {
            let id = core.tag_of_address(&address).map_or(0, |t| t.id);
            if id != self.friend_id {
                self.friend_id = id;
                self.friend_since = tick;
            }
            self.tracks.spare(id);
        }
        let mut invitee: Option<(f32, u32)> = None;
        // A society invites its own crew and guild, never a passer-by.
        if let (true, Some(own)) = (room && self.society.is_none(), self.tracks.own()) {
            for t in self.tracks.recent(tick, TICK_HZ) {
                if t.species != Species::Player
                    || !t.visible
                    || t.wounded
                    || team.has(t.id)
                    || core.is_agent(t.id)
                {
                    continue;
                }
                let d = (t.pos[0] - own.pos[0]).hypot(t.pos[2] - own.pos[2]);
                if d <= INVITE_REACH_M && invitee.is_none_or(|i| d < i.0) {
                    invitee = Some((d, t.id));
                }
            }
        }
        // The friend first, in reach: not the nearest stranger beside them.
        let own_pos = self.tracks.own().map(|o| [o.pos[0], o.pos[2]]);
        let mut friend_at = None;
        if self.friend_id != 0 {
            if let Some(t) = self
                .tracks
                .recent(tick, TICK_HZ)
                .find(|t| t.id == self.friend_id && t.visible)
            {
                self.friend_seen = tick;
                friend_at = Some([t.pos[0], t.pos[2]]);
                let in_reach = own_pos
                    .is_some_and(|o| (t.pos[0] - o[0]).hypot(t.pos[2] - o[1]) <= INVITE_REACH_M);
                if room && in_reach && !t.wounded && !team.has(t.id) {
                    invitee = Some((0.0, t.id));
                }
            }
        }
        if let Some(id) = self.sense_society(body, &team, room, tick) {
            invitee = Some((0.0, id));
        }
        self.invitee = invitee.map(|(_, id)| id);
        self.senses.invitee = self.invitee.is_some();
        self.senses.friend_at = friend_at;
        self.senses.friend = self.friend_sense(core, &team, tick);
        // Someone a give would go to: its friend first, else the nearest
        // teammate, up, awake and in sight within hand reach.
        // Teammates and its friend are spared from `recent`, so all of them.
        let mut give_to: Option<(bool, f32, u32)> = None;
        if let Some(o) = own_pos {
            for t in self.tracks.all() {
                let friend = self.friend_id != 0 && t.id == self.friend_id;
                if t.species != Species::Player
                    || !t.visible
                    || !t.active()
                    || t.wounded
                    || t.dead
                    || t.sleeping
                    || t.id == core.player_id
                    || !(friend || team.has(t.id))
                {
                    continue;
                }
                let d = (t.pos[0] - o[0]).hypot(t.pos[2] - o[1]);
                if d <= GIVE_REACH_M
                    && give_to
                        .is_none_or(|(f, near, _)| (friend && !f) || (friend == f && d < near))
                {
                    give_to = Some((friend, d, t.id));
                }
            }
        }
        self.give_to = give_to.map(|(.., id)| id);
        self.senses.give_to = self.give_to.is_some();
        if team.count == 1 && team.leader == core.player_id {
            let since = *self.alone_since.get_or_insert(tick);
            if tick.wrapping_sub(since) >= ALONE_TEAM_SECS * TICK_HZ
                && self.queue(|b| protocol::encode_action_team(protocol::TEAM_OP_LEAVE, 0, b))
            {
                self.alone_since = None;
            }
        } else {
            self.alone_since = None;
        }
        let seen = (team.invite, team.count);
        if seen != self.team_seen {
            self.team_seen = seen;
            self.memory.team_wake = true;
        }
        let mut backpack = Sighting::default();
        if let Some([x, _, z]) = home::death_bag(core) {
            let (d, b) = relative(body, x, z);
            backpack.add(d, b);
        }
        self.senses.backpack = backpack;
        self.memory.bags = self.home.bags_known();
        let at = [body.qx as f32 * POS_XZ_Q, body.qz as f32 * POS_XZ_Q];
        self.memory.bag_held = self.home.bag_held(at, tick);
        self.memory.recover_held = self.home.recover_held(core);
        let survey = *self.builder.survey();
        self.memory.base = survey;
        // A build that could not reach its work, or failed as it began, is
        // not offered again at once: a second a try, it would be all the body
        // did (two village founders, an hour each, 2026-10-17).
        if self
            .build_failed
            .is_some_and(|at| tick.wrapping_sub(at) < BUILD_RETRY_TICKS)
        {
            self.memory.base.ready = false;
        }
        // Living in the team's base, it builds none of its own; nor does a
        // crewmate who is not its crew's founder, before it is shown home.
        if self.shared_home() || self.society.is_some_and(|soc| !soc.founder()) {
            self.memory.base.ready = false;
        }
        self.home.set_base(self.builder.stand());
        let unreachable = self.home_unreachable(tick);
        self.memory.stash_held = self.home.stash_held(tick) || unreachable;
        self.memory.home_held = self.home_held(tick) || unreachable;
        self.memory.station_held = self
            .station_failed
            .is_some_and(|at| tick.wrapping_sub(at) < STATION_RETRY_TICKS);
        self.memory.home = match self.builder.stand() {
            None => HomeSense::default(),
            Some([hx, hz]) => {
                let (d, bearing) = relative(body, hx, hz);
                let state = if !survey.hearth {
                    HomeState::Plot
                } else if self.builder.region(body) != Region::Outside {
                    HomeState::Inside
                } else {
                    HomeState::Built
                };
                HomeSense {
                    state,
                    distance: Distance::of(d),
                    bearing,
                    metres: d.clamp(0.0, f32::from(u16::MAX)) as u16,
                    attacked: self.home.alarmed(tick, home::HOME_ALARM_TICKS),
                    damaged: self.memory.damaged,
                    doors: self.memory.doors.0,
                    doors_locked: self.memory.doors.1,
                    key_locks: self.memory.doors.2,
                    unreachable,
                }
            }
        };
        if self
            .water_at
            .is_none_or(|at| tick.wrapping_sub(at) >= TICK_HZ)
        {
            self.water_at = Some(tick);
            self.memory.doors = self.builder.doors(core);
            // A new session knows no plot: the base from before is taken up
            // again where its own bag lies in the airlock, not taken for a
            // stranger's and walked away from. A crewmate's bag lies in its
            // founder's base, which is not its own to build.
            let adopted = self.builder.stats.adopted;
            if self.builder.plan().is_none() && self.society.is_none_or(|soc| soc.founder()) {
                if let Some(plan) = Builder::find_home(core, self.home.own_bags()) {
                    self.builder.adopt(core, plan);
                }
            }
            if self.builder.stats.adopted != adopted {
                // What it saw of its own walls before it knew them, it took
                // for somebody else's building, a raid's among them.
                self.seen.clear();
                self.bases.clear();
            }
            // Once a second too: what stands of the base, what the next
            // part of it needs, and the chores waiting at home.
            let chest = self.builder.box_addr(core);
            if chest.is_none() {
                self.ledger.clear();
            }
            self.builder.survey_now(core, &self.ledger);
            self.chores(core, chest.is_some(), tick);
            // Its stations, what the next milestone comes down to at them,
            // and where loot is: what it has seen, and the map.
            let stations = self.stations(core);
            let mut raw = [(0, 0); RAW_ROWS];
            self.memory.raw_len =
                raw_needs(core, &stations, self.builder.survey().needs(), &mut raw) as u8;
            self.memory.raw = raw;
            self.memory.stations = stations;
            self.memory.loot_held = self.loot.held(tick);
            // Home's damage, and the alarm that calls it back; bases worth
            // a raid, by its temperament and what it carries.
            let here = [body.qx as f32 * POS_XZ_Q, body.qz as f32 * POS_XZ_Q];
            let in_sight = self
                .builder
                .stand()
                .is_some_and(|[x, z]| (x - here[0]).hypot(z - here[1]) <= defend::MEND_SIGHT_M);
            self.memory.damaged = self.builder.survey().hearth
                && in_sight
                && defend::mendable(core, &self.builder, &self.defend_let_be, tick);
            self.memory.defend_held = self
                .defend_failed
                .is_some_and(|at| tick.wrapping_sub(at) < defend::DEFEND_RETRY_TICKS);
            self.memory.alarm = self.home.alarmed(tick, defend::DEFEND_ALARM_TICKS);
            let temperament = self.combat.temperament();
            // A society keeps the peace it asks of others: it raids nobody.
            self.memory.raids = self.society.is_none()
                && matches!(temperament, Temperament::Opportunist | Temperament::Kos);
            // A patrol that could not reach its posts is not offered again
            // at once: a body walled in would choose it every second.
            self.memory.patrol_home = self.patrol_centre().is_some()
                && self
                    .patrol_failed
                    .is_none_or(|at| tick.wrapping_sub(at) >= society::PATROL_RETRY_SECS * TICK_HZ);
            self.post_board(core, body);
            self.memory.raid_held = self
                .raid_failed
                .is_some_and(|at| tick.wrapping_sub(at) < RAID_HELD_TICKS);
            let means = self.means(core);
            let (n, nearest) = self.bases.targets(&means, temperament, here, tick);
            let mut raid = Sighting::default();
            if let Some([x, z]) = nearest {
                let (d, b) = relative(body, x, z);
                raid.add(d, b);
                raid.count = n;
            }
            self.senses.raid = raid;
            // The team's base: where it is, what its boxes hold, what the
            // pack has for them and lacks from them, and whether it is
            // under attack.
            let mut home = Sighting::default();
            if let Some([hx, hz]) = self.shared.anchor() {
                let (d, b) = relative(body, hx, hz);
                home.add(d, b);
            }
            self.senses.team_home = home;
            let mut rows = [(0u16, 0u32); crate::mind::SUMMARY_HOME_HOLDS];
            self.memory.home_holds_len = self.shared.totals(&mut rows) as u8;
            self.memory.home_holds = rows;
            self.memory.home_boxes = self.shared.box_count() as u8;
            self.memory.shared = self.shared_home();
            let mut wants = [(0u16, 0u32); WANT_ROWS];
            self.wants_len = if self.book.ready() {
                self.shared.wants(core, &self.book, &mut wants) as u8
            } else {
                0
            };
            self.wants = wants;
            let wants = &self.wants[..usize::from(self.wants_len)];
            self.memory.home_needs = self.shared.has_wanted(core, wants);
            self.memory.deliver = self.shared.anchor().is_some()
                && self.book.ready()
                && (self.shared.has_delivery(core, &self.book, wants) || self.memory.home_needs);
            self.memory.deliver_held = self
                .deliver_failed
                .is_some_and(|at| tick.wrapping_sub(at) < supply::DELIVER_RETRY_TICKS);
            self.memory.box_full_held = self
                .box_full
                .is_some_and(|at| tick.wrapping_sub(at) < supply::BOX_FULL_RETRY_TICKS);
            self.memory.home_alarm = self
                .home_alarm
                .is_some_and(|at| tick.wrapping_sub(at) < home::HOME_ALARM_TICKS);
            self.memory.team_code = self.team_code.is_some();
            self.memory.crewed = self.shared.crewed();
            // An authorize: on a team, the team's cupboard known (not its
            // own, whose crew it joined by placing it) and not on its crew,
            // and a way onto it — the team's code for a lock on it, or a bare
            // one, whose crew answers yes or no.
            let own_hearth = self.builder.hearth_addr(core);
            self.memory.authorize = core.team().count > 0
                && self.shared.hearth().is_some_and(|h| own_hearth != Some(h))
                && !self.shared.crewed()
                && (self.team_code.is_some() || self.shared.hearth_unlocked(core))
                && self
                    .auth_failed
                    .is_none_or(|at| tick.wrapping_sub(at) >= AUTHORIZE_RETRY_TICKS);
            self.memory.bag_home = self
                .shared
                .anchor()
                .is_some_and(|at| self.home.bag_near(at, supply::HOME_M));
            self.memory.team_bag = self.memory.shared
                && !self.memory.bag_home
                && self.shared.may_bag(core, self.team_code.is_some())
                && self
                    .team_bag_failed
                    .is_none_or(|at| tick.wrapping_sub(at) >= TEAM_BAG_RETRY_TICKS);
            let held =
                |at: Option<u32>| at.is_some_and(|at| tick.wrapping_sub(at) < VISIT_RETRY_TICKS);
            self.memory.trade_held = held(self.trade_failed);
            self.memory.work_held = held(self.work_failed);
            self.memory.lore_held = self.lore_failed.map(held);
            self.memory.go_held = self
                .go_failed
                .filter(|&(_, at)| held(Some(at)))
                .map(|(name, _)| name);
            self.memory.travels = self.mind.kind() == SourceKind::External;
            let (x, z) = (body.qx as f32 * POS_XZ_Q, body.qz as f32 * POS_XZ_Q);
            let fire = self.device_near(core, Work::Cook, x, z);
            // Mid-cook the meat is on the grill rather than in the pack, and a
            // camp fire's one grill slot cooks it piece by piece: the work is
            // still on offer until the last piece is done.
            let cooking = self.goal() == Some(Goal::Cook)
                && core.cont_kind == CONT_BOX
                && oven::should_run(Work::Cook, core, &self.book, &core.cont[..BOX_SLOTS]);
            self.memory.cook = (cooking
                || oven::can_tend(core, &self.book, Work::Cook, fire.is_some(), &Keep::NONE))
                && self
                    .cook_failed
                    .is_none_or(|at| tick.wrapping_sub(at) >= oven::RETRY_TICKS);
            // Salvage is taken apart unless the base wants it whole: what
            // the rest of the milestone costs, or the gear still to be made
            // takes.
            let recycler = self.device_near(core, Work::Recycle, x, z);
            self.memory.recycler_near = recycler.is_some();
            let book = &self.book;
            let mut keep = self
                .builder
                .kept_whole(core, |item| book.page(item).recycles);
            for &(item, units) in self.builder.survey().bill() {
                if book.page(item).recycles {
                    keep.at_least(item, units);
                }
            }
            self.keep = keep;
            self.memory.recycle =
                oven::can_tend(core, book, Work::Recycle, recycler.is_some(), &keep)
                    && self
                        .recycle_failed
                        .is_none_or(|at| tick.wrapping_sub(at) >= oven::RETRY_TICKS);
            // A research: paper to read, or a found thing and the junk for
            // it (or what an earlier one left in the table) while THE GATE's
            // table is there to take them to. Mid-research the sample is in
            // the table rather than the pack: still on offer until it is done,
            // or the next decision walks off and leaves the paper there.
            let tier = self.builder.stations(core).tier;
            let researching = self.goal() == Some(Goal::Research)
                && (self.paper_read || matches!(self.oven_job.work(), Work::Research { .. }));
            let left = self
                .research_left
                .is_some_and(|sample| oven::research_of(core, sample).is_some());
            let table = oven::town_table(core).is_some();
            self.memory.research = researching
                || ((oven::unread(core).is_some()
                    || (table && (left || oven::research(core, tier).is_some())))
                    && self
                        .research_failed
                        .is_none_or(|at| tick.wrapping_sub(at) >= VISIT_RETRY_TICKS));
            // The crew's shop (`agent::shop`), its founder's to keep: a
            // machine carried and none standing is put down; one standing is
            // stocked when the pack has lots of a ware past what the base
            // keeps, and looked in on for its takings now and then.
            let machine = self.society.and_then(|_| self.shop_machine(core));
            self.memory.shop_stands = machine.is_some();
            let carried_machine = oven::device_row(core, crate::agent::shop::SHOP_ITEM)
                .is_some_and(|(_, item)| count_item(core, item) > 0);
            let placing = machine.is_none() && carried_machine && self.shop_spot(core).is_some();
            let stocking = machine.is_some()
                && (crate::agent::shop::lots_carried(core) >= crate::agent::shop::STOCK_LOTS
                    || self.shop_kept.is_none_or(|at| {
                        tick.wrapping_sub(at) >= crate::agent::shop::VISIT_SECS * TICK_HZ
                    }));
            self.memory.hand = self.hand_due(core, body, tick);
            self.memory.shop = self.society.is_some_and(|s| s.founder())
                && (placing || stocking)
                && self
                    .shop_failed
                    .is_none_or(|at| tick.wrapping_sub(at) >= oven::RETRY_TICKS);
            let here = [body.qx as f32 * POS_XZ_Q, body.qz as f32 * POS_XZ_Q];
            let mut known = Sighting::default();
            // What a loot run would set off for (`loot::TRIP_M`): one known
            // farther off was offered, and the run ended at once on it.
            for spot in self.loot.live(core, tick).filter(|s| !s.roadside()) {
                let (d, b) = relative(body, spot.slot.x, spot.slot.z);
                if d <= loot::TRIP_M {
                    known.add(d, b);
                }
            }
            self.senses.loot = known;
            self.senses.loot_place = self.loot.place(&haven, here, tick).map(|(_, [x, z])| {
                let (d, bearing) = relative(body, x, z);
                Place::at(d, bearing)
            });
            let (seed, _) = core.island();
            let x = body.qx as f32 * POS_XZ_Q;
            let z = body.qz as f32 * POS_XZ_Q;
            // The nearest water each way, and a lake before the sea.
            let mut best: Option<(f32, u16, bool)> = None;
            for dir in 0..8u16 {
                let yaw = dir << 13;
                let (fx, fz) = yaw_dir(yaw);
                let found = WATER_PROBE_M.iter().find_map(|&m| {
                    let (h, open) = terrain::height_open(seed, x + fx * m, z + fz * m);
                    (h < terrain::SEA_LEVEL).then_some((m, open >= 0.5))
                });
                if let Some((m, salt)) = found {
                    if best.is_none_or(|b| (salt, m) < (b.2, b.0)) {
                        best = Some((m, yaw, salt));
                    }
                }
            }
            let mut water = Sighting::default();
            if let Some((m, yaw, _)) = best {
                let (fx, fz) = yaw_dir(yaw);
                let (_, b) = relative(body, x + fx * m, z + fz * m);
                water.add(m, b);
            }
            self.senses.water = water;
            self.senses.water_salt = best.is_some_and(|b| b.2);
            self.water_yaw = best.map(|b| b.1);
            self.water_point = best.map(|(m, yaw, _)| {
                let (fx, fz) = yaw_dir(yaw);
                [x + fx * m, z + fz * m]
            });
        }
    }

    /// What a visit home would do now: take out what the base needs, feed
    /// the cupboard, put things away. What the box holds is the ledger's
    /// word, an empty box until its panel has been seen.
    fn chores(&mut self, core: &ClientCore, chest: bool, tick: u32) {
        let survey = self.builder.survey();
        let grades = self.builder.charged();
        self.memory.feed = survey.hearth
            && self.builder.hearth_addr(core).is_some()
            && stash::feed_due(core, &self.home, &self.ledger, grades, tick);
        // Only a move the visit would make calls it home: a take the pack
        // has no room for is no chore.
        let book = &self.book;
        let mut bill = [(0, 0); stash::VISIT_ROWS];
        let n = stash::visit_bill(survey.bill(), &self.home, grades, tick, &mut bill);
        let has = self.builder.stations(core);
        let next = (chest && book.ready())
            .then(|| {
                stash::plan(
                    core,
                    |item| stash::keeps(core, book, &has, item),
                    &bill[..n],
                    self.ledger.slots(),
                )
            })
            .flatten();
        self.memory.take_out = survey.take_out && matches!(next, Some(Transfer::Take { .. }));
        self.memory.put_away = matches!(next, Some(Transfer::Put { .. }));
        let mut rows = [(0u16, 0u32); stash::STORED_ROWS];
        self.memory.stored_len = self.ledger.totals(&mut rows) as u8;
        self.memory.stored = rows;
    }

    /// Examine the next cell of the sight window; publish the census when a
    /// sweep completes.
    fn scan_cell(&mut self, core: &mut ClientCore, body: &EntityState, haven: &Haven, tick: u32) {
        let haven = *haven;
        let dx = self.scan % SIGHT_WIDTH - SIGHT_CELLS;
        let dz = self.scan / SIGHT_WIDTH - SIGHT_CELLS;
        self.scan = (self.scan + 1) % (SIGHT_WIDTH * SIGHT_WIDTH);
        let cx = (body.qx as f32 * POS_XZ_Q / CELL_SIZE).floor() as i32 + dx;
        let cz = (body.qz as f32 * POS_XZ_Q / CELL_SIZE).floor() as i32 + dz;
        if (0..terrain::CELLS_PER_SIDE).contains(&cx) && (0..terrain::CELLS_PER_SIDE).contains(&cz)
        {
            let (seed, island) = core.island();
            let slot = island.cache.slot(seed, island.table, island.haven, cx, cz);
            let target = Target {
                cx: cx as u16,
                cz: cz as u16,
                slot,
            };
            if let Some(kind) = Kind::of(slot.occupant) {
                if !target.gone(core) && in_view(body, &slot) && visible(core, &haven, body, target)
                {
                    let (d, b) = relative(body, slot.x, slot.z);
                    // The census counts what a gather goal would go for:
                    // not the node it gave up on a moment ago.
                    if Some(target.key()) != self.skipped {
                        self.sweep.cells[kind as usize].add(d, b);
                    }
                    let nearer = self.recall[kind as usize].is_none_or(|(old, at)| {
                        tick.wrapping_sub(at) >= RECALL_SECS * TICK_HZ
                            || old.gone(core)
                            || relative(body, old.slot.x, old.slot.z).0 >= d
                    });
                    if nearer && Some(target.key()) != self.skipped {
                        self.recall[kind as usize] = Some((target, tick));
                    }
                    // A goal a fight paused picks its next target when it
                    // resumes, from where the fight left the body.
                    let wanted = self
                        .goal
                        .filter(|a| a.paused.is_none())
                        .and_then(|a| Kind::of_goal(a.goal))
                        == Some(kind);
                    if wanted
                        && self.target.is_none()
                        && self.recovery.is_none()
                        && Some(target.key()) != self.skipped
                    {
                        self.target = Some(target);
                        self.target_gain = 0;
                        self.progress_tick = tick;
                        self.best_distance = f32::INFINITY;
                        self.stats.targets_seen += 1;
                    }
                }
            }
            // A barrel still standing, or a crate, in the cone with a
            // clear line: somewhere loot is.
            if let Some(prize) = Prize::of(slot.occupant) {
                if (prize == Prize::Crate || !core.harvested.contains(target.key()))
                    && in_view(body, &slot)
                    && visible(core, &haven, body, target)
                {
                    self.loot.saw(Spot {
                        cx: target.cx,
                        cz: target.cz,
                        slot,
                        prize,
                        seen: tick,
                    });
                }
            }
        }
        if self.scan == 0 {
            // In view now, or seen within the recall window and still
            // standing: a player remembers the rock they just turned from,
            // and the gather goal walks back to it (`RECALL_WALK_M`).
            let mut cells = self.sweep.cells;
            for (kind, cell) in cells.iter_mut().enumerate() {
                if cell.count > 0 {
                    continue;
                }
                if let Some((t, at)) = self.recall[kind] {
                    let (d, b) = relative(body, t.slot.x, t.slot.z);
                    if tick.wrapping_sub(at) < RECALL_SECS * TICK_HZ
                        && !t.gone(core)
                        && Some(t.key()) != self.skipped
                        && d <= RECALL_WALK_M
                    {
                        cell.add(d, b);
                    }
                }
            }
            let [trees, stone, ore, bushes, hemp] = cells;
            self.senses.trees = trees;
            self.senses.stone_nodes = stone;
            self.senses.ore_nodes = ore;
            self.senses.bushes = bushes;
            self.senses.hemp = hemp;
            self.sweep = Sweep::default();
            self.sensed = true;
        }
    }

    /// Apply one reliable message and take the facts this controller owns.
    fn event_with(&mut self, core: &mut ClientCore, bytes: &[u8]) -> Result<(), WireError> {
        let flags = core.on_stream(bytes)?;
        let applied2 = core.applied2();
        let tick = self.seen_tick.unwrap_or_default();
        while let Some((item, added)) = core.pop_toast() {
            self.stats.gather_awards += 1;
            if let Some(g) = self.gathered.get_mut(item as usize) {
                *g = g.saturating_add(u32::from(added));
            }
            if let Some(a) = self.goal.as_mut() {
                a.gained = a.gained.saturating_add(u32::from(added));
                if let Some(kind) = Kind::of_goal(a.goal) {
                    self.memory.yields[kind as usize].set(item);
                }
            }
            if self.target.is_some() {
                self.target_gain = self.target_gain.saturating_add(u32::from(added));
                self.progress_tick = tick;
            }
        }
        while core.pop_gather_refusal().is_some() {
            self.stats.refusals += 1;
            if let Some(t) = self.target.take() {
                self.skipped = Some(t.key());
            }
        }
        while let Some((sector, damage, _)) = core.pop_hurt() {
            if damage == 0 {
                continue;
            }
            let toward =
                (u32::from(sector) * 65536 / u32::from(sim_core::combat::HURT_SECTORS)) as u16;
            // Compass bearings turn toward -X; wire yaw turns toward +X.
            self.tracks.on_hurt(0u16.wrapping_sub(toward), tick);
            let away = 0u16.wrapping_sub(toward).wrapping_add(1 << 15);
            // The alarm: the reflex turns to find who struck it.
            self.combat.on_hurt(tick, away);
            self.last_hurt = Some((tick, away));
            self.recovery = None;
            if let Some(target) = self.target.take() {
                self.skipped = Some(target.key());
                self.stats.targets_abandoned += 1;
            }
            self.stats.hurts += 1;
            self.memory.hits = self.memory.hits.saturating_add(1);
            if let Some(own) = self.tracks.own() {
                self.home.on_hurt([own.pos[0], own.pos[2]], tick);
            }
        }
        // What is said around it: filed for the next request, its own
        // line coming back included; another body's local line asks early.
        self.has_mates = core.team().count > 1;
        while let Some((from, channel, text)) = core.pop_chat() {
            let me = from == core.player_id;
            if me {
                self.stats.echoed += 1;
            } else {
                self.stats.heard += 1;
                self.memory.chat_wake |= !channel.is_global();
            }
            let tag = core.tag(from);
            self.memory.chat.push(HeardLine {
                from,
                name: tag.map_or(protocol::Name::EMPTY, |t| t.name),
                address: tag.map_or(protocol::Address::GUEST, |t| t.address),
                agent: core.is_agent(from),
                channel,
                me,
                text,
                voice: false,
                more: false,
            });
        }
        while let Some(victim) = core.pop_death() {
            self.tracks.on_death(victim, tick);
            if victim == core.player_id {
                self.stats.deaths += 1;
                self.memory.deaths = self.memory.deaths.saturating_add(1);
                if let Some(own) = self.tracks.own() {
                    self.home.on_hurt([own.pos[0], own.pos[2]], tick);
                }
            }
        }
        // What the eyes and ears make of the fight around this body: the
        // tracks take facts, never the rings.
        while let Some((shooter, yaw, pitch, speed, low, _fine)) = core.pop_shot() {
            if shooter == core.player_id {
                self.combat.shot(tick);
                self.shot_at = Some(tick);
            }
            self.tracks.on_shot(shooter, (yaw, pitch), speed, low, tick);
        }
        // A reload the server turned down (busy, full, nothing to load):
        // the next ask waits out its own retry either way.
        while core.pop_reload_refusal().is_some() {
            self.stats.refusals += 1;
        }
        while let Some(swinger) = core.pop_swing() {
            if swinger == core.player_id {
                self.combat.swung(tick);
            }
            self.tracks.on_swing(swinger, tick);
        }
        while let Some(i) = core.pop_impact() {
            let at = [
                i.qx as f32 * POS_XZ_Q,
                i.qy as f32 * POS_Y_Q,
                i.qz as f32 * POS_XZ_Q,
            ];
            self.tracks.on_impact(at, tick);
            if i.kind == IMPACT_BLAST {
                self.home.on_blast([at[0], at[2]], tick);
                if self.near_shared((at[0], at[2])) {
                    self.home_alarm = Some(tick);
                }
            }
            // Arrows and bullets landing at home that are not its own.
            let mine = self
                .shot_at
                .is_some_and(|t| tick.wrapping_sub(t) <= OWN_SHOT_TICKS);
            if matches!(i.kind, IMPACT_ARROW | IMPACT_BULLET) && !mine {
                self.home.on_shot([at[0], at[2]], tick);
            }
        }
        while let Some(hit) = core.pop_hit() {
            // A blow on a body; a wall's marker is not one.
            if hit.victim != client_core::core::NO_VICTIM {
                self.stats.landed += 1;
            }
            self.tracks.on_hit(hit.victim, hit.damage);
        }
        // Its own base struck, or broken (not rotting: `Home::on_removed`
        // weighs that against the cupboard's stock): home under attack.
        // A `StructHit` never sets `APPLIED_HIT` (wire v77); a repair
        // latches the piece at its whole hp, so hp short of it is a blow.
        if flags & APPLIED_STRUCT_HIT != 0 && applied2 & APPLIED2_OWN_STRUCT_HIT == 0 {
            let (cx, cz, level, loc, left, max) = core.struct_hit;
            if left != max && self.builder.owns(cx, cz, level, loc) {
                self.home.on_struck(tick);
            } else if left != max && self.near_shared(sim_core::build::anchor(cx, cz, loc)) {
                self.home_alarm = Some(tick);
            }
        }
        while let Some(r) = core.pop_removed() {
            let twig = !r.deploy
                && u16::from(r.row) < core.piece_defs_have
                && core.piece_defs.pieces[usize::from(r.row)].material == MAT_TWIG;
            if !twig && self.builder.owns(r.cx, r.cz, r.level, r.loc) {
                self.home.on_removed(tick, self.builder.charged());
            }
            if r.deploy
                && self
                    .builder
                    .hearth_spot()
                    .is_some_and(|h| (h.cx, h.cz, h.level, h.loc) == (r.cx, r.cz, r.level, r.loc))
            {
                self.home.lost_hearth();
            }
        }
        // Its own deploys' answers: home takes the facts. (The own-bag
        // list comes in below, `APPLIED2_BAGS`: at the join, as its bags go
        // down or come up, on a wake and at each death.)
        while let Some((cx, cz, level, loc, deploy)) = core.pop_placed() {
            self.home.on_placed(cx, cz, level, loc, deploy);
            self.builder.on_placed(cx, cz, level, loc, deploy);
            self.oven_job.on_placed(cx, cz, level, loc, deploy);
        }
        while let Some(reason) = core.pop_deploy_refusal() {
            self.stats.refusals += 1;
            // The crew's join in flight was refused: the crew does not
            // admit it (`REFUSE_D_OWNER`), or has no room.
            if matches!(self.auth.leg, AuthLeg::Sent(_)) {
                self.auth.refused = true;
            }
            self.home.on_refused(reason);
            self.builder.on_refused(true, reason);
            self.stash_job.on_refused();
            self.oven_job.on_refused(reason);
            self.home_job.on_refused(reason);
            self.leave.doors.on_refused(reason);
        }
        // Its own cupboard's stock readout: a feed's reply, or the crew
        // vital's push while it stands in the claim. Either is a reading;
        // only a reply answers a feed (`StashJob::on_stock`).
        if flags & APPLIED_STOCK != 0 {
            let (cx, cz, level) = core.stock_addr;
            if self
                .builder
                .hearth_addr(core)
                .is_some_and(|h| (h.cx, h.cz, h.level) == (cx, cz, level))
            {
                self.home.on_stock(core, tick, self.builder.charged());
                self.stash_job.on_stock(core.stock_grew);
            }
        }
        // Its own box's panel: what it shows is what the box holds.
        if applied2 & APPLIED2_CONT != 0 {
            if let Some(b) = self.builder.box_addr(core) {
                if self
                    .ledger
                    .on_cont(core, box_key(b.cx, b.cz, b.level, b.loc))
                {
                    self.stash_job.on_panel();
                }
            }
        }
        if applied2 & APPLIED2_MOVE != 0 {
            self.raid_job.on_moved(core.last_move_refused != 0);
            self.stash_job.on_moved(core.last_move_refused != 0, tick);
            self.loot_job.on_moved(core.last_move_refused != 0);
            self.oven_job.on_moved(core.last_move_refused != 0, tick);
            self.home_job.on_moved(core.last_move_refused != 0);
        }
        // A fire's or a recycler's panel: the session's open was answered.
        if applied2 & APPLIED2_CONT != 0 && core.cont_kind == CONT_BOX {
            self.oven_job.on_panel(core.cont_handle);
            self.raid_job.on_panel(core.cont_handle);
            self.home_job.on_panel(core.cont_handle);
            self.shared.on_panel(core);
        }
        // A charge on a wall somewhere: the raid's plant, if it is at the
        // face the raid is working.
        if applied2 & APPLIED2_CHARGE != 0 {
            let (cx, cz, level, loc, ..) = core.charge_placed;
            self.raid_job
                .on_charge(core.charge_deploy, cx, cz, level, loc);
        }
        // A crate's panel: the loot run's open was answered.
        if applied2 & APPLIED2_CONT != 0 && core.cont_kind == CONT_WORLD {
            self.loot_job.on_panel(core.cont_handle);
        }
        while let Some(reason) = core.pop_build_refusal() {
            self.stats.refusals += 1;
            self.defend_job.on_refused(reason);
            self.raid_job.on_refused(reason);
            self.builder.on_refused(false, reason);
        }
        // A lock let it through (its own code, entered at its own door);
        // a blueprint learned or refused.
        while let Some((cx, cz, level, loc, grant)) = core.pop_auth() {
            self.shared.on_auth(cx, cz, level, loc, grant);
            self.builder.on_auth(cx, cz, level, loc);
            self.home_job.on_auth(cx, cz, level, loc);
            self.leave.doors.on_auth(cx, cz, level, loc);
        }
        // THE GATE's answer to a trade, and a work's refusal of a feed.
        while let Some((refused, ..)) = core.pop_vend() {
            self.trade_job.on_vend(refused);
        }
        while let Some((_, op, target)) = core.pop_arc_refused() {
            self.stats.refusals += 1;
            self.work_job.on_refused();
            self.read_job.on_refused(op, target);
            self.solve_job.on_refused(op, target);
        }
        // A stone's words, a lock solved anywhere, a card refused.
        while let Some((kind, index, _)) = core.pop_arc_text() {
            self.read_job.on_text(kind, index);
        }
        while let Some((mech, by)) = core.pop_mech_solved() {
            self.solve_job.on_solved(mech, by, core.player_id);
        }
        while let Some((code, door)) = core.pop_swipe_refused() {
            self.stats.refusals += 1;
            self.swipe_job.on_refused(code, door);
        }
        while core.pop_work_event().is_some() {}
        while core.pop_research_toast().is_some() {}
        while let Some(reason) = core.pop_research_refusal() {
            self.stats.refusals += 1;
            self.builder.on_research_refused(reason);
        }
        if applied2 & APPLIED2_BAGS != 0 {
            self.home.on_bags(core.own_bags());
        }
        if flags & APPLIED_RESPAWN != 0 && !core.dead {
            self.stats.respawns += 1;
            self.memory.respawns = self.memory.respawns.saturating_add(1);
            self.memory.trigger = Trigger::Respawned;
            if matches!(self.awaiting, Some((Pending::Respawn, _))) {
                self.awaiting = None;
            }
            // A second respawn ask may still be held; a live body needs none.
            self.outbox = None;
            // A new body on a new beach: what the old one saw is elsewhere.
            self.recall = [None; KINDS];
            self.tracks.forget();
            self.hands.forget();
            // The map this body walked stays learned; the way it was
            // walking starts from the new beach.
            self.route.reset();
            self.frontier.target = None;
            self.seek = None;
            self.sensed = false;
            self.halt();
            self.heading = None;
            self.last_hurt = None;
            self.crawl_to = None;
            self.reload_at = None;
        }
        while let Some((item, added)) = core.pop_craft_toast() {
            self.stats.crafted += u64::from(added);
            if let Some((Pending::Craft { item: want }, _)) = self.awaiting {
                if want == item {
                    self.verdict = Some(Verdict::Ok);
                }
            }
        }
        while core.pop_craft_refusal().is_some() {
            self.stats.refusals += 1;
            self.builder.on_craft_refused();
            if matches!(self.awaiting, Some((Pending::Craft { .. }, _))) {
                self.verdict = Some(Verdict::Refused);
            }
        }
        while let Some((item, _slot)) = core.pop_consume_toast() {
            self.stats.eaten += 1;
            // Only the toast for what was asked for: a late one for an
            // earlier consume is not this one's answer.
            if let Some((
                Pending::Consume {
                    item: want,
                    food,
                    water,
                },
                _,
            )) = self.awaiting
            {
                if want == item {
                    self.learning = Some((item, food, water));
                    self.verdict = Some(Verdict::Ok);
                }
            }
        }
        while let Some(reason) = core.pop_consume_refusal() {
            self.stats.refusals += 1;
            match self.awaiting {
                Some((Pending::Consume { item, .. }, _)) => {
                    if u32::from(reason) == REFUSE_C_NOT_FOOD {
                        self.memory.food.not_food.set(item);
                        self.verdict = Some(Verdict::Refused);
                    } else if u32::from(reason) == REFUSE_C_FULL {
                        self.verdict = Some(Verdict::Full);
                    } else {
                        self.verdict = Some(Verdict::Refused);
                    }
                }
                Some((Pending::Drink, _)) => {
                    self.verdict = Some(if u32::from(reason) == REFUSE_C_FULL {
                        Verdict::Full
                    } else {
                        Verdict::NoWater
                    });
                }
                _ => {}
            }
        }
        // A vessel filled or poured rides the drink's flag too; only a
        // mouthful is a drink.
        if flags & APPLIED_DRANK != 0 && core.last_vessel.is_none() {
            self.stats.drinks += 1;
            if matches!(self.awaiting, Some((Pending::Drink, _))) {
                self.verdict = Some(Verdict::Ok);
            }
        }
        if flags & APPLIED_VITALS != 0 {
            if let Some((item, food, water)) = self.learning.take() {
                if core.food > food {
                    self.memory.food.feeds.set(item);
                }
                if core.water > water {
                    self.memory.food.waters.set(item);
                }
                if core.food <= food && core.water <= water {
                    // Consumed but restored neither meter: a bandage, say.
                    // Not food for this purpose, whatever else it does.
                    self.memory.food.not_food.set(item);
                }
            }
        }
        if applied2 & APPLIED2_MOVE != 0 && matches!(self.awaiting, Some((Pending::Equip, _))) {
            self.verdict = Some(if core.last_move_refused == 0 {
                Verdict::Ok
            } else {
                Verdict::Refused
            });
        }
        Ok(())
    }
}

impl BotDriver for Survivor {
    fn welcome(&mut self, welcome: &Welcome) {
        // A goal still in hand is the last session's (a shift that ended
        // mid-walk home, a dropped link). Kept, its clock would count the
        // time away: the log-off walk home a shift's end cut short failed
        // as stuck at 1900 s the moment the next shift began (#363). It
        // ends where that session left it.
        if self.goal.is_some() {
            let last = self.seen_tick.unwrap_or(welcome.tick);
            self.end_goal(last, Outcome::Interrupted(Why::Session));
        }
        let mut core = Box::new(match self.known_haven {
            Some((seed, haven)) if seed == welcome.seed => {
                ClientCore::with_haven(welcome.seed, welcome.player_id, welcome.tick, haven)
            }
            _ => ClientCore::new(welcome.seed, welcome.player_id, welcome.tick),
        });
        self.haven = Some(*core.island().1.haven);
        self.core = Some(core);
        self.tracks.reset(welcome.seed, welcome.player_id);
        self.hands.reset(welcome.seed, welcome.player_id);
        self.route.reset();
        self.frontier.clear();
        self.combat.reset(welcome.seed, welcome.player_id);
        self.home.reset();
        self.seen.clear();
        self.builder.reset();
        self.builder.set_code(self.lock.code(welcome.seed));
        self.bag_job = BagJob::default();
        self.bag_leg = BagLeg::Here;
        self.team_bag_failed = None;
        self.recover_job = RecoverJob::default();
        self.stash_job = StashJob::default();
        self.ledger.clear();
        self.loot.clear();
        self.loot_job = LootJob::default();
        self.bases.clear();
        self.defend_job = DefendJob::default();
        self.defend_let_be = LetBe::default();
        self.raid_job = RaidJob::default();
        self.shared = TeamHome::default();
        self.home_job = HomeJob::default();
        self.home_alarm = None;
        self.leave = Leave::default();
        self.deliver_failed = None;
        self.box_full = None;
        // A crewmate lives behind its founder's doors: the crew's code is
        // what its clan would have told it, never printed.
        self.team_code = self
            .society
            .filter(|soc| !soc.founder())
            .and_then(|soc| soc.code)
            .map(|code| code.code(welcome.seed));
        self.kin.clear();
        self.crew_near = None;
        self.patrol = Patrol::default();
        self.hand = Hand::default();
        self.shot_at = None;
        self.oven_job = DeviceJob::new(Work::Cook);
        self.fire = None;
        self.recycler = None;
        self.shop = None;
        if self.welcomed.is_none() {
            self.welcomed = Some(welcome.tick);
            self.set_deadline(self.deadline_after);
        }
        self.deferred = None;
        self.glance = None;
        // Off the frame path: parsing the content allocates. A body that
        // cannot read it plays on without the wiki.
        if self.rules.is_none() {
            self.rules = Rules::shipped().ok();
        }
        self.book = Book::EMPTY;
        self.pace = Pace::default();
        self.outbox = None;
        self.say_line = None;
        self.voice_typed.clear();
        self.say_at = 0;
        self.memory.chat.clear();
        self.memory.chat_wake = false;
        self.memory.team_wake = false;
        self.team_seen = (0, 0);
        self.guard = None;
        self.invitee = None;
        self.alone_since = None;
        self.seen_tick = Some(welcome.tick);
        self.seen_at = Instant::now();
    }

    fn event(&mut self, bytes: &[u8]) -> Result<(), WireError> {
        let Some(mut core) = self.core.take() else {
            return Ok(());
        };
        let result = self.event_with(&mut core, bytes);
        self.core = Some(core);
        result
    }

    fn frame(&mut self, view: &ClientView, player: u32, seq: u16) -> InputFrame {
        self.frame_at(view, player, seq, Instant::now())
    }

    fn action(&mut self, out: &mut [u8]) -> Option<usize> {
        let (buf, len) = self.outbox.as_ref()?;
        // The tick this action reaches the server's lane on, as far as this
        // client can tell; the gaps are what matter, and they hold.
        let at = u64::from(self.seen_tick.unwrap_or_default()) + 1;
        match protocol::decode_action(&buf[..*len]) {
            Ok(msg) if !self.pace.go(&msg, at) => {
                self.stats.paced += 1;
                return None;
            }
            _ => {}
        }
        let (buf, len) = self.outbox.take()?;
        if out.len() < len {
            self.stats.unencodable += 1;
            return None;
        }
        out[..len].copy_from_slice(&buf[..len]);
        self.stats.actions += 1;
        Some(len)
    }

    fn voice_in(&mut self, frame: &protocol::VoiceFrame) {
        if let Some(v) = self.voice.as_mut() {
            v.hear(frame, Instant::now());
        }
    }

    fn voice_out(&mut self, out: &mut [protocol::VoiceFrame]) -> usize {
        let Some(v) = self.voice.as_mut() else {
            return 0;
        };
        let now = Instant::now();
        let (memory, stats, core) = (&mut self.memory, &mut self.stats, self.core.as_deref());
        // What the players near it said out loud, filed like a line they
        // typed — who said it included, so its friend's words are its
        // friend's — and asked about early, as a local line is.
        v.tick(now, |from, text| {
            stats.heard += 1;
            memory.chat_wake = true;
            let tag = core.and_then(|c| c.tag(from));
            for (text, more) in crate::agent_voice::pieces(text) {
                memory.chat.push(HeardLine {
                    from,
                    name: tag.map_or(protocol::Name::EMPTY, |t| t.name),
                    address: tag.map_or(protocol::Address::GUEST, |t| t.address),
                    agent: core.is_some_and(|c| c.is_agent(from)),
                    channel: protocol::Channel::Local,
                    me: false,
                    text,
                    voice: true,
                    more,
                });
            }
        });
        if let Some(text) = v.take_unsaid() {
            // The newest line it could not say replaces any still being typed.
            self.voice_typed.clear();
            self.voice_typed.extend(
                crate::agent_voice::pieces(&text)
                    .into_iter()
                    .map(|(t, _)| t),
            );
        }
        v.due(now, out)
    }

    fn say(&mut self, out: &mut [u8]) -> Option<usize> {
        let tick = self.seen_tick?;
        if tick < self.say_at {
            return None;
        }
        // A body speaks where it asked to: to everyone, to the bodies near
        // it, or to its team when it has one. What its voice could not say
        // goes to the bodies near it, who would have heard it.
        let (line, channel) = match self.say_line.take() {
            Some(lines) => (
                {
                    self.say_line = lines.rest();
                    lines.first
                },
                match self.say_to {
                    protocol::Channel::Team if !self.has_mates => protocol::Channel::Local,
                    to => to,
                },
            ),
            None => (self.voice_typed.pop_front()?, protocol::Channel::Local),
        };
        match protocol::encode_chat(line.as_bytes(), channel, out) {
            Ok(len) => {
                let refill = protocol::chat::CHAT_REFILL_MS * u64::from(sim_core::limits::TICK_HZ);
                self.say_at = tick.saturating_add((refill / 1000) as u32);
                self.stats.said += 1;
                Some(len)
            }
            Err(_) => {
                self.stats.say_refused += 1;
                None
            }
        }
    }
}

/// An answer that cannot wait for a fight to end: running, fighting,
/// healing and going home to defend it while it is fought over (`alarm`)
/// are part of it. Mending what a quiet base lost can wait.
fn urgent(goal: Goal, alarm: bool) -> bool {
    match goal {
        Goal::Flee | Goal::Fight | Goal::Heal => true,
        Goal::Defend => alarm,
        _ => false,
    }
}

fn heartbeat_ticks(mind: &Mind) -> u32 {
    let secs = mind
        .config()
        .heartbeat
        .as_secs()
        .min(u64::from(u32::MAX / TICK_HZ));
    secs as u32 * TICK_HZ
}

fn pct(value: u16, max: u16) -> u32 {
    if max == 0 {
        100
    } else {
        u32::from(value) * 100 / u32::from(max)
    }
}

fn catalog_ready(core: &ClientCore) -> bool {
    let n = usize::from(core.catalog.count).min(MAX_ITEM_DEFS);
    n > 0 && (0..n).all(|i| core.catalog.lens[i] > 0)
}

fn amount(core: &ClientCore, name: &[u8]) -> u32 {
    core.inv
        .iter()
        .filter(|s| s.count > 0 && core.catalog.name(s.item as usize) == name)
        .map(|s| u32::from(s.count))
        .sum()
}

fn count_item(core: &ClientCore, item: u16) -> u32 {
    core.inv
        .iter()
        .filter(|s| s.count > 0 && s.item == item)
        .map(|s| u32::from(s.count))
        .sum()
}

/// The belt slot holding the best working tool for this resource. Forage
/// is picked by hand whatever is held, so any slot will do.
pub fn best_tool(core: &ClientCore, kind: Kind) -> Option<u8> {
    let ladder: &[&str] = match kind {
        Kind::Wood => &TREE_TOOLS,
        Kind::Stone | Kind::Ore => &NODE_TOOLS,
        Kind::Forage | Kind::Hemp => return Some(best_tool(core, Kind::Stone).unwrap_or(0)),
    };
    for name in ladder {
        for (slot, stack) in core.inv[..HOTBAR_SLOTS].iter().enumerate() {
            if stack.count > 0
                && core.catalog.name(stack.item as usize) == name.as_bytes()
                && (core.catalog.row(stack.item as usize).cond_max == 0 || stack.cond > 0)
            {
                return Some(slot as u8);
            }
        }
    }
    None
}

/// A craft of a tool or weapon that would be a second one: a working one
/// of it is carried, or a better one on every tool ladder it stands on
/// ([`TREE_TOOLS`], [`NODE_TOOLS`]), or one that outclasses it
/// (`loadout::outclassed`). A player looks in the pack before crafting
/// (an agent was offered the stone hatchet and pickaxe it carried,
/// 2026-10-18). One worn out does not count: making another is the repair.
fn needless_gear(core: &ClientCore, book: &Book, item: u16, name: &str) -> bool {
    let gear = matches!(
        book.page(item).class,
        Class::Melee | Class::Ranged | Class::Tool
    ) && core.catalog.row(usize::from(item)).stack_max <= 1;
    if !gear {
        return false;
    }
    let working = |n: &str| {
        core.inv.iter().any(|st| {
            st.count > 0
                && core.catalog.name(usize::from(st.item)) == n.as_bytes()
                && (core.catalog.row(usize::from(st.item)).cond_max == 0 || st.cond > 0)
        })
    };
    if working(name) {
        return true;
    }
    let mut rungs = [TREE_TOOLS, NODE_TOOLS]
        .into_iter()
        .filter_map(|ladder| Some((ladder, ladder.iter().position(|t| *t == name)?)))
        .peekable();
    let laddered = rungs.peek().is_some();
    (laddered && rungs.all(|(ladder, rank)| ladder[..rank].iter().any(|t| working(t))))
        || crate::agent::loadout::outclassed(name, working)
}

/// The move that puts on better armour than is worn, if the pack holds
/// some: `(pack slot, wear slot)`, by the catalog's wear slot and armour
/// share, the best piece for each slot first.
fn wear_move(core: &ClientCore) -> Option<(u8, u8)> {
    let mut best: Option<(u8, u8, u8)> = None;
    for (i, s) in core.inv[..INV_SLOTS].iter().enumerate() {
        if s.count == 0 {
            continue;
        }
        let row = core.catalog.row(usize::from(s.item));
        let Some(slot) = usize::from(row.wear_slot).checked_sub(1) else {
            continue;
        };
        let Some(worn) = core.worn.get(slot) else {
            continue;
        };
        let have = if worn.count > 0 {
            core.catalog.row(usize::from(worn.item)).armor_pct
        } else {
            0
        };
        let gain = row.armor_pct;
        if gain > have && best.is_none_or(|(.., g)| gain > g) {
            best = Some((i as u8, slot as u8, gain));
        }
    }
    best.map(|(from, to, _)| (from, to))
}

/// Out of the town's safe zone, if the body stands in it: along the
/// market street to the nearer gate and on past the zone's edge, where the
/// town puts its sleepers. The town is on the map.
fn out_of_town(core: &ClientCore, body: &EntityState) -> Option<Intent> {
    let town = core.haven().town;
    let (x, z) = (body.qx as f32 * POS_XZ_Q, body.qz as f32 * POS_XZ_Q);
    if !sim_core::town::safe(&town, x, z) {
        return None;
    }
    let out = sim_core::town::SAFE_HALF_M + 8.0;
    let gate = |side: f32| sim_core::kit::to_world(&town.placed(), 0.0, side * out);
    let (a, b) = (gate(1.0), gate(-1.0));
    let d2 = |p: (f32, f32)| (p.0 - x) * (p.0 - x) + (p.1 - z) * (p.1 - z);
    let (gx, gz) = if d2(a) <= d2(b) { a } else { b };
    Some(Intent::walk(yaw_toward(gx - x, gz - z)))
}

/// Room for a node's yield: an empty slot, or a stack of something this
/// kind has been seen to pay that is not yet at its ceiling.
fn room_for(core: &ClientCore, yields: ItemBits) -> bool {
    core.inv.iter().any(|s| {
        s.count == 0
            || (yields.has(s.item) && s.count < core.catalog.row(s.item as usize).stack_max)
    })
}

/// A recipe this player can use for the named output: no station or one of
/// its own standing, and any blueprint already known. `(recipe, output
/// item, ticks per unit, station)`.
fn resolve_recipe(core: &ClientCore, name: Name, has: &Stations) -> Option<(u16, u16, u32, u8)> {
    if core.recipes_have < core.recipes.recipe_count {
        return None;
    }
    let known = core.known();
    (0..usize::from(core.recipes.recipe_count).min(core.recipes.recipes.len())).find_map(|r| {
        let def = core.recipes.recipes[r];
        let usable = def.out_count > 0
            && has.usable(def.station)
            && (!def.blueprint || sim_core::research::knows(known, r as u16))
            && core.catalog.name(def.output as usize) == name.as_bytes();
        usable.then_some((r as u16, def.output, def.ticks, def.station))
    })
}

/// How many to queue in one craft: one, or at a furnace a batch, which
/// smelts on while the body walks off (the queue is the server's), as
/// many as the inputs, the pack's room for the output and
/// `SMELT_BATCH_SECS` allow.
fn batch(core: &ClientCore, recipe: u16) -> u16 {
    let Some(def) = core.recipes.recipes.get(usize::from(recipe)) else {
        return 1;
    };
    if def.station != STATION_FURNACE || def.out_count == 0 {
        return 1;
    }
    let inputs = def.inputs[..usize::from(def.n_inputs).min(def.inputs.len())]
        .iter()
        .filter(|&&(_, per)| per > 0)
        .map(|&(item, per)| count_item(core, item) / u32::from(per))
        .min()
        .unwrap_or(0);
    let max = u32::from(core.catalog.row(usize::from(def.output)).stack_max.max(1));
    let room: u32 = core.inv[..INV_SLOTS]
        .iter()
        .map(|s| match s.count {
            0 => max,
            n if s.item == def.output => max.saturating_sub(u32::from(n)),
            _ => 0,
        })
        .sum();
    inputs
        .min(room / u32::from(def.out_count))
        .min(SMELT_BATCH_SECS * TICK_HZ / def.ticks.max(1))
        .min(u32::from(CRAFT_COUNT_MAX))
        .max(1) as u16
}

fn inputs_ok(core: &ClientCore, recipe: u16) -> bool {
    let Some(def) = core.recipes.recipes.get(recipe as usize) else {
        return false;
    };
    def.inputs[..usize::from(def.n_inputs).min(def.inputs.len())]
        .iter()
        .all(|&(item, need)| count_item(core, item) >= u32::from(need))
}

/// A slot worth eating: one known to restore water (`thirst`), or one known
/// to feed, else one never tried that the eat verb has not refused; the
/// most filling first, by what the catalog says each is worth (cooked meat
/// before mushrooms).
fn food_slot(core: &ClientCore, book: &FoodBook, thirst: bool) -> Option<usize> {
    let slots = || (0..INV_SLOTS).filter(|&i| core.inv[i].count > 0);
    if thirst {
        return slots().find(|&i| book.waters_known(core.inv[i].item));
    }
    let worth = |i: &usize| core.catalog.row(usize::from(core.inv[*i].item)).food;
    // The first of the most filling, so ties keep the slot order.
    let best = |it: &mut dyn Iterator<Item = usize>| {
        it.fold(None, |best: Option<usize>, i| match best {
            Some(b) if worth(&b) >= worth(&i) => Some(b),
            _ => Some(i),
        })
    };
    best(&mut slots().filter(|&i| book.feeds.has(core.inv[i].item)))
        .or_else(|| best(&mut slots().filter(|&i| book.may_feed(core.inv[i].item))))
}

/// A food in the pack never tried: what a thirsty body with no water in
/// sight tastes, since eating is how it learns which of its food carries
/// water. A forager full of berries it had never tasted let its water run
/// down once mushrooms stopped coming off every tree it chopped (#356).
fn taste_slot(core: &ClientCore, book: &FoodBook) -> Option<usize> {
    (0..INV_SLOTS).find(|&i| {
        let s = core.inv[i];
        s.count > 0
            && usize::from(s.item) < ItemBits::CAP
            && !book.tried.has(s.item)
            && !book.not_food.has(s.item)
            && core.catalog.row(usize::from(s.item)).food > 0
    })
}

/// The sim's own drink test: five taps on the heightfield at the reach.
fn water_in_reach(seed: u64, x: f32, z: f32) -> Option<bool> {
    let r = DRINK_REACH_M;
    let (mut wet, mut fresh) = (false, false);
    for (dx, dz) in [(0.0, 0.0), (r, 0.0), (-r, 0.0), (0.0, r), (0.0, -r)] {
        let (h, open) = terrain::height_open(seed, x + dx, z + dz);
        let here = h < terrain::SEA_LEVEL;
        wet |= here;
        fresh |= here && open < 0.5;
    }
    wet.then_some(!fresh)
}

/// The observation encoder (PLAYERS.md): a pure function of what this
/// client received — its decoded state, its own view, and what its own
/// perception and history derived from them. No `World`, no seed in the
/// output, no absolute position, no other body's identity.
pub fn observe(
    core: &ClientCore,
    view: &ClientView,
    player: u32,
    senses: &Senses,
    memory: &Memory,
    book: &Book,
    loadout: &Loadout,
) -> Summary {
    let mut s = Summary::EMPTY;
    s.tick = view.newest_applied.unwrap_or_default();
    let body = view.get(player);
    s.body = if core.dead || body.is_some_and(|b| b.dead) {
        BodyState::Dead
    } else if core.wounded || body.is_some_and(|b| b.wounded) {
        BodyState::Wounded
    } else {
        BodyState::Alive
    };
    (s.hp, s.hp_max) = (core.hp, core.hp_max);
    (s.food, s.food_max) = (core.food, core.max_food);
    (s.water, s.water_max) = (core.water, core.max_water);
    s.free_slots = core.inv.iter().filter(|st| st.count == 0).count() as u8;
    // What the playbook counts by name goes in first: a pack of many kinds
    // must not push its wood, its stone or its tools off the list's end.
    let counted = |name: &Name| {
        let name = name.as_str();
        matches!(
            name,
            "Wood" | "Stone" | "Cloth" | crate::mind::BAG_ITEM | crate::mind::SATCHEL_ITEM
        ) || TREE_TOOLS.contains(&name)
            || NODE_TOOLS.contains(&name)
            || crate::mind::LOOTED.contains(&name)
            || crate::mind::SMELTS.iter().any(|&(ore, _)| ore == name)
            || crate::agent::loadout::ARM_UP
                .iter()
                .any(|&(item, ..)| item == name)
    };
    for first in [true, false] {
        for stack in core.inv.iter().filter(|st| st.count > 0) {
            let Some(name) = Name::new(core.catalog.name(stack.item as usize)) else {
                continue;
            };
            if counted(&name) != first {
                continue;
            }
            let n = s.items_len as usize;
            if let Some(entry) = s.items[..n].iter_mut().find(|(e, _)| *e == name) {
                entry.1 = entry.1.saturating_add(u32::from(stack.count));
            } else if n < SUMMARY_ITEMS {
                s.items[n] = (name, u32::from(stack.count));
                s.items_len += 1;
            }
        }
    }
    if core.recipes_have >= core.recipes.recipe_count {
        let known = core.known();
        // Tools first, then what the base needs and what its furnace makes
        // of the pack, then arms (weapons, their rounds, meds), then the
        // sleeping bag: the list is bounded, and with a full pack more than
        // `SUMMARY_CRAFTS` recipes can be craftable at once — the better
        // tool, the first spear and what the playbook wants next are the
        // ones that must not fall off the end. A recipe made at one of its
        // own stations is craftable wherever it stands: the craft walks
        // there.
        let arm = |item: u16| {
            let page = book.page(item);
            matches!(
                page.class,
                Class::Melee | Class::Ranged | Class::Med | Class::Throw
            ) || (0..MAX_ITEM_DEFS as u16).any(|w| book.page(w).ranged.round == item)
        };
        let needed = |item: u16, name: &Name| {
            crate::mind::SMELTS
                .iter()
                .any(|&(_, out)| out == name.as_str())
                || memory.base.needs().iter().any(|&(i, _)| i == item)
        };
        let rank = |item: u16, name: &Name| {
            if TREE_TOOLS.contains(&name.as_str()) || NODE_TOOLS.contains(&name.as_str()) {
                0
            } else if needed(item, name) {
                1
            } else if arm(item) {
                2
            } else if name.as_str() == crate::mind::BAG_ITEM {
                3
            } else {
                4
            }
        };
        for pass in 0..5 {
            for r in 0..usize::from(core.recipes.recipe_count).min(core.recipes.recipes.len()) {
                let def = core.recipes.recipes[r];
                // What the queue already holds is coming; a full queue
                // takes nothing more.
                if def.out_count == 0
                    || usize::from(core.jobs_count) >= CRAFT_QUEUE
                    || queued(core, r as u16)
                    || !memory.stations.usable(def.station)
                    || (def.station != STATION_NONE && memory.station_held)
                    || (def.blueprint && !sim_core::research::knows(known, r as u16))
                    || !inputs_ok(core, r as u16)
                    || !(room_for(core, ItemBits::bit(def.output)) || craft_fits(core, r as u16, 1))
                {
                    continue;
                }
                let Some(name) = Name::new(core.catalog.name(def.output as usize)) else {
                    continue;
                };
                // Satchels are for a temperament that raids.
                if name.as_str() == crate::mind::SATCHEL_ITEM && !memory.raids {
                    continue;
                }
                // A tool already carried, and not worn out, is not made twice.
                let tool = rank(def.output, &name) == 0;
                if tool
                    && core
                        .inv
                        .iter()
                        .any(|st| st.count > 0 && st.item == def.output && st.cond > 0)
                {
                    continue;
                }
                let n = s.craftable_len as usize;
                if rank(def.output, &name) == pass
                    && n < SUMMARY_CRAFTS
                    && !s.craftable[..n].contains(&name)
                    && !needless_gear(core, book, def.output, name.as_str())
                {
                    s.craftable[n] = name;
                    s.craftable_len += 1;
                }
            }
        }
    }
    s.trees = senses.trees;
    s.stone_nodes = senses.stone_nodes;
    s.ore_nodes = senses.ore_nodes;
    s.bushes = senses.bushes;
    s.hemp = senses.hemp;
    s.players = senses.players;
    s.humans = senses.humans;
    s.animals = senses.animals;
    s.water_near = senses.water;
    s.threats = senses.threats;
    s.threats_len = senses.threats_len;
    s.last_fight = memory.fight;
    s.last = memory.last;
    s.hits = memory.hits;
    s.heard = memory.chat;
    s.deaths = memory.deaths;
    s.respawns = memory.respawns;
    s.bags = memory.bags;
    s.backpack = senses.backpack;
    s.home = memory.home;
    s.bag_ready = core.any_bag_ready();
    s.night = crate::agent::tracks::night(s.tick, &core.env);
    s.milestone = memory.base.milestone;
    for &(item, units) in memory.base.needs() {
        let n = usize::from(s.needs_len);
        if let (Some(name), true) = (
            Name::new(core.catalog.name(usize::from(item))),
            n < s.needs.len(),
        ) {
            s.needs[n] = (name, units);
            s.needs_len += 1;
        }
    }
    for &(item, units) in &memory.raw[..usize::from(memory.raw_len)] {
        let n = usize::from(s.raw_len);
        if let (Some(name), true) = (
            Name::new(core.catalog.name(usize::from(item))),
            n < s.raw.len(),
        ) {
            s.raw[n] = (name, units);
            s.raw_len += 1;
        }
    }
    s.bench = memory.stations.bench.is_some();
    s.furnace = memory.stations.furnace.is_some();
    s.loot = senses.loot;
    s.loot_place = senses.loot_place;
    s.raid = senses.raid;
    for &(item, units) in &memory.stored[..usize::from(memory.stored_len)] {
        let n = usize::from(s.stored_len);
        if let (Some(name), true) = (
            Name::new(core.catalog.name(usize::from(item))),
            n < s.stored.len(),
        ) {
            s.stored[n] = (name, units);
            s.stored_len += 1;
        }
    }
    (s.take_out, s.feed) = (memory.take_out, memory.feed);
    s.recycler_near = memory.recycler_near;
    s.trigger = memory.trigger;
    // Its team (wire v108), from the team view its client was sent.
    let who = |id: u32| {
        let tag = core.tag(id);
        crate::mind::Who {
            id,
            name: tag.map_or(protocol::Name::EMPTY, |t| t.name),
            address: tag.map_or(protocol::Address::GUEST, |t| t.address),
        }
    };
    let team = core.team();
    s.team.leader = team.count > 0 && team.leader == player;
    s.team.invited = team.invite != 0 && team.count == 0;
    if s.team.invited {
        s.team.invite_from = who(team.invite);
    }
    for m in team.members() {
        if m.id == player {
            continue;
        }
        s.team.mates += 1;
        s.team.who[usize::from(s.team.who_len)] = who(m.id);
        s.team.who_len += 1;
        match m.state {
            protocol::TEAM_AWAKE => match body {
                Some(b) => {
                    let (d, bearing) = relative(b, f32::from(m.x_m) + 0.5, f32::from(m.z_m) + 0.5);
                    s.team.awake.add(d, bearing);
                }
                None => s.team.awake.count += 1,
            },
            protocol::TEAM_DEAD => s.team.down += 1,
            _ => {}
        }
    }
    s.friend = senses.friend;
    s.society = senses.society;
    if let (Some(b), Some([fx, fz])) = (body, senses.friend_at) {
        let (d, bearing) = relative(b, fx, fz);
        s.friend.in_sight.add(d, bearing);
    }
    s.team.mate_attacked = senses.mate_attacked;
    s.team.home = senses.team_home;
    s.team.home_boxes = memory.home_boxes;
    s.team.home_attacked = memory.home_alarm;
    s.team.lives_there = memory.shared;
    s.team.home_has_needs = memory.home_needs;
    s.team.code = memory.team_code;
    s.team.on_cupboard = memory.crewed;
    s.team.bag_there = memory.bag_home;
    // THE GATE and the works, from the tables its client was sent.
    if let (Some(b), Some([gx, gz])) = (body, gate::gate(core)) {
        let (d, bearing) = relative(b, gx, gz);
        s.gate = Some(Place::at(d, bearing));
    }
    let named = |item: u16| Name::new(core.catalog.name(usize::from(item))).unwrap_or(Name::EMPTY);
    for (_, w) in core.arc.known() {
        let n = usize::from(s.works_len);
        if n >= s.works.len() {
            break;
        }
        let mut sense = crate::mind::WorkSense {
            name: Name::new(w.name().as_bytes()).unwrap_or(Name::EMPTY),
            state: w.state,
            fuel: named(w.def.fuel),
            fuel_room: w.def.fuel_max.saturating_sub(w.fuel),
            unlocks: Name::new(w.floor_name().as_bytes()).unwrap_or(Name::EMPTY),
            ..crate::mind::WorkSense::EMPTY
        };
        if let (Some(b), Some((x, _, z))) = (body, sim_core::spot::world(core.haven(), &w.def.spot))
        {
            let (d, bearing) = relative(b, x, z);
            sense.at = Some(Place::at(d, bearing));
        }
        let mut needs = [(0u16, 0u32); sim_core::limits::MAX_WORK_INPUTS];
        let m = gate::work_needs(w, &mut needs);
        for (i, &(item, left)) in needs[..m].iter().enumerate() {
            sense.needs[i] = (named(item), left);
        }
        sense.needs_len = m as u8;
        s.works[n] = sense;
        s.works_len += 1;
    }
    // The stones, their locks and the ziggurat's card doors.
    let place = |x: f32, z: f32| {
        body.map(|b| {
            let (d, bearing) = relative(b, x, z);
            Place::at(d, bearing)
        })
    };
    (s.stones_read, s.stones_known) = lore::stones(core);
    if let Some(b) = body {
        let (x, z) = (b.qx as f32 * POS_XZ_Q, b.qz as f32 * POS_XZ_Q);
        s.unread_stone =
            lore::unread(core, x, z, f32::INFINITY).and_then(|(_, at)| place(at[0], at[2]));
    }
    s.glyphs = core.lore.glyphs.count_ones() as u8;
    s.alphabet = core.lore.alphabet_len;
    for (m, mech) in core.lore.known_mechs() {
        let n = usize::from(s.locks_len);
        if n >= s.locks.len() {
            break;
        }
        let middle = usize::from(mech.n_dials.saturating_sub(1) / 2);
        s.locks[n] = crate::mind::LockSense {
            name: Name::new(mech.name().as_bytes()).unwrap_or(Name::EMPTY),
            at: lore::dial_at(core, m, middle).and_then(|p| place(p[0], p[2])),
            dials: mech.n_dials,
            resting: mech.resting,
            answer_known: lore::answer(&core.lore, m).is_some(),
        };
        s.locks_len += 1;
    }
    let zig = core.haven().ziggurat;
    for (d, card) in lore::CARD_NAMES.iter().enumerate().filter(|_| zig.live) {
        let reader = sim_core::monument::reader_world(&zig, d, false);
        s.card_doors[d] = crate::mind::CardDoorSense {
            card: Name::new(card.as_bytes()).unwrap_or(Name::EMPTY),
            at: reader.and_then(|(x, _, z)| place(x, z)),
            have_card: crate::agent::build::item_named(core, card)
                .is_some_and(|item| count_item(core, item) > 0),
            open: lore::door_open(core, d),
        };
        s.card_doors_len = d as u8 + 1;
    }
    for &(item, units) in &memory.home_holds[..usize::from(memory.home_holds_len)] {
        let n = usize::from(s.home_holds_len);
        if let (Some(name), true) = (
            Name::new(core.catalog.name(usize::from(item))),
            n < s.home_holds.len(),
        ) {
            s.home_holds[n] = (name, units);
            s.home_holds_len += 1;
        }
    }
    if s.body != BodyState::Alive {
        return s;
    }
    // A gather goal needs a tool for the kind and room for what it pays: a
    // full pack takes the offer away rather than failing every second.
    s.offer(Goal::Explore);
    let fits = |kind: Kind| room_for(core, memory.yields[kind as usize]);
    if best_tool(core, Kind::Wood).is_some() && fits(Kind::Wood) {
        s.offer(Goal::GatherWood);
    }
    if best_tool(core, Kind::Stone).is_some() {
        if fits(Kind::Stone) {
            s.offer(Goal::GatherStone);
        }
        if fits(Kind::Ore) {
            s.offer(Goal::GatherOre);
        }
    }
    if fits(Kind::Forage) {
        s.offer(Goal::Forage);
    }
    if fits(Kind::Hemp) {
        s.offer(Goal::GatherHemp);
    }
    // Eat and drink are offered only below the level they fill to, and
    // only where they can work: something worth eating in the pack; a food
    // the eat verb has seen restore water, or open water in sight: a lake,
    // or the sea when thirsty enough and healthy enough to pay for it.
    if core.max_food > 0
        && pct(core.food, core.max_food) < METER_TARGET_PCT
        && food_slot(core, &memory.food, false).is_some()
    {
        s.offer(Goal::Eat);
    }
    let sea = senses.water.count > 0
        && (!senses.water_salt
            || (pct(core.water, core.max_water.max(1)) < SEA_DRINK_BELOW_PCT
                && (core.hp_max == 0 || pct(core.hp, core.hp_max) > SEA_MIN_HP_PCT)));
    if core.max_water > 0
        && pct(core.water, core.max_water) < METER_TARGET_PCT
        && (sea
            || food_slot(core, &memory.food, true).is_some()
            || taste_slot(core, &memory.food).is_some())
    {
        s.offer(Goal::Drink);
    }
    if s.players.count > 0 || s.animals.count > 0 {
        s.offer(Goal::Flee);
    }
    // A fight or a hunt needs something to swing or shoot on the belt and
    // someone in sight; a heal, a med in the pack and health to restore.
    let armed = core.inv[..HOTBAR_SLOTS].iter().any(|st| {
        let page = book.page(st.item);
        st.count > 0
            && (page.swings()
                || (page.fires()
                    && page.ranged.round != sim_core::gather::NO_ITEM
                    && count_item(core, page.ranged.round) > 0))
    });
    // What `dose` can take, by the same ladder: a med it cannot pick would
    // offer a heal that fails at once and is chosen again, every tick.
    let meds = loadout.best(core, Role::Meds).is_some();
    let player = senses.threats[..usize::from(senses.threats_len)]
        .iter()
        .any(|t| t.arms != Arms::Teeth);
    if armed && player {
        s.offer(Goal::Fight);
    }
    if armed && s.animals.count > 0 {
        s.offer(Goal::Hunt);
    }
    if meds && core.hp < core.hp_max && pct(core.hp, core.hp_max) < HEAL_TARGET_PCT {
        s.offer(Goal::Heal);
    }
    s.offer(Goal::Wait);
    // A bag in the pack and room for one more of its own down; the way
    // back to the last death's backpack while it is near and nobody who
    // could start the fight again is in view.
    if usize::from(memory.bags) < BAG_CAP
        && !memory.bag_held
        && !(memory.shared && memory.bag_home)
        && home::bag_row(core).is_some_and(|(_, item)| count_item(core, item) > 0)
    {
        s.offer(Goal::Bag);
    }
    if senses.backpack.count > 0
        && f32::from(senses.backpack.nearest_m) <= home::RECOVER_M
        && !senses.hostile
        && !memory.recover_held
    {
        s.offer(Goal::Recover);
    }
    // The base: the next milestone while some op of it can go now; the way
    // home once a cupboard stands there and this body is not inside.
    if memory.base.ready {
        s.offer(Goal::Build);
    }
    if memory.home.state == HomeState::Built && !memory.home_held {
        s.offer(Goal::GoHome);
    }
    // A loot run: a barrel or crate it knows of, or a place on the map
    // that keeps them, room in the pack, and nobody dangerous in view.
    if (senses.loot.count > 0 || senses.loot_place.is_some())
        && s.free_slots >= loot::MIN_FREE_SLOTS
        && !senses.hostile
        && !memory.loot_held
    {
        s.offer(Goal::Loot);
    }
    // A cook, with raw meat, fuel and a fire to cook on, nobody dangerous
    // about.
    if memory.cook && !senses.hostile {
        s.offer(Goal::Cook);
    }
    if memory.recycle && !senses.hostile {
        s.offer(Goal::Recycle);
    }
    // The gate's guns shoot whoever fought a player lately.
    if memory.research && !senses.hostile_afield && core.hostile_left() == 0 {
        s.offer(Goal::Research);
    }
    if memory.shop && !senses.hostile {
        s.offer(Goal::KeepShop);
    }
    // A visit home while there is something to see to there.
    if matches!(memory.home.state, HomeState::Built | HomeState::Inside)
        && !memory.stash_held
        && (memory.take_out || memory.feed || memory.put_away)
    {
        s.offer(Goal::Stash);
    }
    // Home called back to (an alarm while out) or damaged and mendable,
    // and no defence came to nothing a moment ago.
    let built = matches!(memory.home.state, HomeState::Built | HomeState::Inside);
    if built
        && !memory.defend_held
        && ((memory.alarm && memory.home.state == HomeState::Built) || memory.damaged)
    {
        s.offer(Goal::Defend);
    }
    // A raid: a base worth it by its temperament and its means, room for
    // what it holds, and nobody dangerous in view.
    if memory.raids
        && senses.raid.count > 0
        && !senses.hostile
        && !memory.raid_held
        && s.free_slots >= loot::MIN_FREE_SLOTS
    {
        s.offer(Goal::Raid);
    }
    // In a society only a crewmate's invite is said yes to.
    if s.team.invited && (!s.society.set || s.society.crew_invited_me) {
        s.offer(Goal::JoinTeam);
    }
    if senses.invitee {
        s.offer(Goal::Invite);
    }
    if s.team.awake.count > 0 {
        s.offer(Goal::Follow);
    }
    // Its friend on the island, not on its team and not yet in reach: the
    // walk to THE GATE, where they know to find it.
    let f = &s.friend;
    if f.online
        && !f.on_team
        && !f.reachable
        && !f.invited_me
        && s.gate.is_some()
        && !senses.hostile_afield
    {
        s.offer(Goal::Meet);
    }
    // Its society's crew, coming together there (`agent::society`).
    if senses.society_meet && s.gate.is_some() && !senses.hostile_afield {
        s.offer(Goal::Meet);
    }
    // A home to walk a ring round, and something in hand to meet an
    // intruder with.
    if armed && memory.patrol_home {
        s.offer(Goal::Patrol);
    }
    if memory.hand && !senses.hostile {
        s.offer(Goal::HandOver);
    }
    // Onto the team's cupboard's crew (Rust's authorize), while it is not.
    if memory.authorize && !senses.hostile {
        s.offer(Goal::Authorize);
    }
    // The team's base, seen with a teammate at one of its boxes: a trip
    // there to stock it and take what the pack lacks, or a supply run for
    // it of a kind a gather goal could go for now; nobody dangerous in
    // view, and no trip there came to nothing a moment ago. Living there,
    // home is there too: the walk home, and its defence.
    if memory.shared {
        if s.team.home.count > 0 && s.team.home.nearest_m > SHARED_HOME_NEAR_M && !memory.home_held
        {
            s.offer(Goal::GoHome);
        }
        if memory.home_alarm && !memory.defend_held {
            s.offer(Goal::Defend);
        }
    }
    if s.team.home.count > 0 && !senses.hostile && !memory.deliver_held {
        if memory.deliver {
            s.offer(Goal::Deliver);
        }
        for (gather, run) in [
            (Goal::GatherWood, Goal::SupplyWood),
            (Goal::GatherStone, Goal::SupplyStone),
            (Goal::GatherOre, Goal::SupplyOre),
            (Goal::GatherHemp, Goal::SupplyHemp),
        ] {
            if s.offers(gather) && !memory.box_full_held {
                s.offer(run);
            }
        }
    }
    // THE GATE's trades, while it is not marked hostile (its guns shoot),
    // and a feed for the nearest work that takes what the pack holds.
    // Strangers at the gate itself do not take them away: it is where
    // everybody trades.
    if !senses.hostile_afield && core.hostile_left() == 0 && s.gate.is_some() && !memory.trade_held
    {
        let mut rows = [(0u16, false); gate::TRADE_ROWS];
        let wants = memory
            .base
            .needs()
            .iter()
            .chain(&memory.raw[..usize::from(memory.raw_len)]);
        let n = gate::trades(core, wants.map(|&(item, _)| item), &mut rows);
        for &(item, sell) in &rows[..n] {
            if let Some(name) = Name::new(core.catalog.name(usize::from(item))) {
                s.offer(if sell {
                    Goal::Sell(name)
                } else {
                    Goal::Buy(name)
                });
            }
        }
    }
    if let Some(b) = body {
        let (x, z) = (b.qx as f32 * POS_XZ_Q, b.qz as f32 * POS_XZ_Q);
        if !senses.hostile
            && !memory.work_held
            && gate::pick_work(core, core.haven(), x, z).is_some()
        {
            s.offer(Goal::FeedWork);
        }
        // The ancients: a stone near it has not read, a lock whose answer
        // it read, a card door it holds the card for; nobody dangerous in
        // view, and none of them came to nothing a moment ago.
        if !senses.hostile {
            if !memory.lore_held[0] && lore::unread(core, x, z, lore::READ_TRIP_M).is_some() {
                s.offer(Goal::ReadStone);
            }
            if !memory.lore_held[1] && lore::solvable(core, x, z).is_some() {
                s.offer(Goal::SolveLock);
            }
            if !memory.lore_held[2] && lore::swipeable(core).is_some() {
                s.offer(Goal::SwipeCard);
            }
        }
    }
    // Trips to the places an external mind is told of: THE GATE (not while
    // its guns would shoot), each work and each lock it knows, while it is
    // not there already and no trip there came to nothing a moment ago.
    // Strangers in view do not take them away: a person walks past people.
    if memory.travels {
        let gate = s
            .gate
            .filter(|_| core.hostile_left() == 0)
            .and_then(|at| Some((Name::new(gate::GATE_NAME.as_bytes())?, at)));
        let (works, locks) = (s.works, s.locks);
        let trips = gate
            .into_iter()
            .chain(
                works[..usize::from(s.works_len)]
                    .iter()
                    .filter_map(|w| Some((w.name, w.at?))),
            )
            .chain(
                locks[..usize::from(s.locks_len)]
                    .iter()
                    .filter_map(|l| Some((l.name, l.at?))),
            );
        for (name, at) in trips {
            if name != Name::EMPTY && at.distance != Distance::Here && memory.go_held != Some(name)
            {
                s.offer(Goal::Go(name));
            }
        }
        // And anywhere at all, by bearing and metres.
        s.offer(Goal::Walk(crate::mind::Walk::ANY));
        // A give of anything the pack holds, while its friend or a teammate
        // stands in hand reach: a mind told who is about decides what.
        if senses.give_to {
            for i in 0..usize::from(s.items_len) {
                s.offer(Goal::Give(s.items[i].0));
            }
        }
    }
    for i in 0..s.craftable_len as usize {
        s.offer(Goal::Craft(s.craftable[i]));
    }
    s
}

fn as_body(body: EntityState) -> Body {
    Body {
        qx: body.qx,
        qy: body.qy,
        qz: body.qz,
        qvy: body.qvy,
        grounded: body.grounded,
    }
}

fn eye_point(body: &EntityState) -> [f32; 3] {
    let (x, y, z) = eye(body);
    [x, y, z]
}

/// Where this body's eyes were in its snapshot: the stance the server
/// applied (wire v83), which is what the eyes saw from.
fn eye(body: &EntityState) -> (f32, f32, f32) {
    (
        body.qx as f32 * POS_XZ_Q,
        body.qy as f32 * POS_Y_Q + tracks::eye_m(body.crouched),
        body.qz as f32 * POS_XZ_Q,
    )
}

/// The eye a swing or a shot sent with these buttons leaves from: the
/// sim's `Player::crouched` read off the frame about to go out (crouch
/// pressed, on the ground, upright), not the snapshot's, which lags it.
fn pressed_eye(body: &EntityState, buttons: u8) -> [f32; 3] {
    let crouched = buttons & BTN_CROUCH != 0 && body.grounded && !body.wounded && !body.dead;
    [
        body.qx as f32 * POS_XZ_Q,
        body.qy as f32 * POS_Y_Q + tracks::eye_m(crouched),
        body.qz as f32 * POS_XZ_Q,
    ]
}

/// The stance a gathering swing leaves from. Only a fight presses
/// `BTN_CROUCH` (the stalk and the hide in `agent::combat`); every skill
/// that swings at a node, a piece or a door presses none, and the sim
/// takes the stance off the frame the swing rides in, so the swing is a
/// standing one even while the snapshot still shows the last fight's
/// crouch.
const SWING_CROUCHED: bool = false;

/// A standing-or-crouched eye's height above the feet, for a swing.
fn swing_eye_m() -> f32 {
    tracks::eye_m(SWING_CROUCHED)
}

/// Distance and relative bearing (index into `mind::BEARINGS`, clockwise
/// from ahead) of a point from this body's facing.
/// The point a walk leg names: `leg.metres` from the body toward
/// `BEARINGS[leg.bearing]` of its facing (`relative`'s sectors, turned
/// back into a direction), clamped inside the island's border.
fn walk_target(body: &EntityState, leg: crate::mind::Walk) -> [f32; 2] {
    let (x, z) = (body.qx as f32 * POS_XZ_Q, body.qz as f32 * POS_XZ_Q);
    let (dx, dz) = yaw_dir(body.yaw.wrapping_add(u16::from(leg.bearing % 8) << 13));
    let m = f32::from(leg.metres);
    let edge = sim_core::movement::BORDER_MARGIN + WALK_STOP_M;
    let far = sim_core::terrain::ISLAND_SIZE - edge;
    [(x + dx * m).clamp(edge, far), (z + dz * m).clamp(edge, far)]
}

fn relative(body: &EntityState, x: f32, z: f32) -> (f32, u8) {
    let dx = x - body.qx as f32 * POS_XZ_Q;
    let dz = z - body.qz as f32 * POS_XZ_Q;
    let distance = dx.hypot(dz);
    let (fx, fz) = yaw_dir(body.yaw);
    let (rx, rz) = yaw_dir(body.yaw.wrapping_add(1 << 14));
    let ahead = dx * fx + dz * fz;
    let right = dx * rx + dz * rz;
    let angle = right.atan2(ahead).rem_euclid(std::f32::consts::TAU);
    let sector = ((angle / (std::f32::consts::TAU / 8.0)).round() as i32).rem_euclid(8);
    (distance, sector as u8)
}

fn in_view(body: &EntityState, slot: &Slot) -> bool {
    in_cone(body, slot.x, slot.z)
}

fn in_cone(body: &EntityState, x: f32, z: f32) -> bool {
    let dx = x - body.qx as f32 * POS_XZ_Q;
    let dz = z - body.qz as f32 * POS_XZ_Q;
    let distance = dx.hypot(dz);
    let (fx, fz) = yaw_dir(body.yaw);
    distance <= SIGHT_M && dx * fx + dz * fz >= distance * std::f32::consts::FRAC_1_SQRT_2
}

/// Where on a node a swing aims: its centre line, at eye height where the
/// trunk or rock reaches it.
fn aim_point(body: &EntityState, slot: &Slot) -> [f32; 3] {
    let eye = body.qy as f32 * POS_Y_Q + swing_eye_m();
    let top = terrain::occupant_volume(slot.occupant).1 * slot.scale;
    let y = eye.clamp(
        slot.y + melee::MELEE_PROBE_M,
        (slot.y + top - melee::MELEE_PROBE_M).max(slot.y + melee::MELEE_PROBE_M),
    );
    [slot.x, y, slot.z]
}

/// The view that faces a node's aim point, and its horizontal distance.
fn aim(body: &EntityState, slot: &Slot) -> (u16, u8, f32) {
    let [x, y, z] = aim_point(body, slot);
    let (dx, dz) = (x - body.qx as f32 * POS_XZ_Q, z - body.qz as f32 * POS_XZ_Q);
    let distance = dx.hypot(dz);
    let eye = body.qy as f32 * POS_Y_Q + swing_eye_m();
    (
        yaw_toward(dx, dz),
        pitch_toward(y - eye, distance),
        distance,
    )
}

/// A swing along this view from where the body stands hits the target
/// node first: the sim's own melee ray.
fn swing_reaches(
    core: &mut ClientCore,
    body: &EntityState,
    yaw: u16,
    pitch: u8,
    target: Target,
) -> bool {
    let ray = melee::ray(
        &as_body(*body),
        SWING_CROUCHED,
        yaw,
        pitch,
        REACH_M * MM_PER_M,
    );
    let (seed, mut island) = core.island();
    melee::node_cast(seed, &mut island, &ray)
        .is_some_and(|hit| hit.cx == target.cx && hit.cz == target.cz)
}

/// A pick along this view from where the body stands reaches the target
/// plant first: the cast the human client's `[E] PICK` prompt makes.
fn pick_reaches(
    core: &mut ClientCore,
    body: &EntityState,
    yaw: u16,
    pitch: u8,
    target: Target,
) -> bool {
    let ray = melee::ray(
        &as_body(*body),
        SWING_CROUCHED,
        yaw,
        pitch,
        sim_core::gather::PICK_REACH_M * MM_PER_M,
    );
    let (seed, mut island) = core.island();
    melee::pick_cast(seed, &mut island, &ray)
        .is_some_and(|hit| hit.cx == target.cx && hit.cz == target.cz)
}

/// `E` from where the body stands, along this view, opens the world
/// container keyed `key`: the cast the human client's `[E] OPEN` prompt
/// makes (`client::ui::interact::resolve_open`), at the arm a container is
/// opened from.
fn open_reaches(core: &mut ClientCore, body: &EntityState, yaw: u16, pitch: u8, key: u32) -> bool {
    let ray = melee::ray(
        &as_body(*body),
        core.crouched(),
        yaw,
        pitch,
        sim_core::backpack::LOOT_REACH_M * MM_PER_M,
    );
    let hit = {
        let (seed, mut island) = core.island();
        melee::occupant_cast(seed, &mut island, &ray, melee::openable)
    };
    let Some(hit) = hit.filter(|hit| cell_key(hit.cx, hit.cz) == key) else {
        return false;
    };
    // And the shard's hand, as the human prompt asks it (`resolve_open`):
    // the same storey and a clear line, so a crate behind stone is not
    // pressed at until the verdict times out.
    let b = as_body(*body);
    sim_core::reach::clear_from(
        core.predict.seed(),
        core.haven(),
        core.pieces.cols(),
        core.card_doors,
        sim_core::reach::eye_of(&b, core.crouched()),
        b.qy as f32 * POS_Y_Q,
        sim_core::reach::Target::unopened(&hit.slot),
    )
}

/// Conservative line of sight to the near face of the target. Uses the
/// same terrain, occupant and built-volume queries as a player's projectile.
fn visible(core: &mut ClientCore, haven: &Haven, body: &EntityState, target: Target) -> bool {
    let slot = target.live(core);
    let (_, pitch, distance) = aim(body, &slot);
    if distance > SIGHT_M || distance <= 0.0 {
        return false;
    }
    let eye = body.qy as f32 * POS_Y_Q + swing_eye_m();
    let radius = terrain::occupant_volume(slot.occupant).0 * slot.scale;
    let stop = (distance - radius - melee::MELEE_PROBE_M).max(0.0);
    let dx = (target.slot.x - body.qx as f32 * POS_XZ_Q) / distance;
    let dz = (target.slot.z - body.qz as f32 * POS_XZ_Q) / distance;
    let (horizontal, vertical) = pitch_dir(pitch);
    let dy = if horizontal > 0.0 {
        vertical / horizontal
    } else {
        0.0
    };
    let origin = (body.qx as f32 * POS_XZ_Q, body.qz as f32 * POS_XZ_Q);
    tracks::clear_line(
        core,
        haven,
        [origin.0, eye, origin.1],
        [origin.0 + dx * stop, eye + dy * stop, origin.1 + dz * stop],
        SIGHT_M,
    ) == Sight::Clear
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mind::{MindConfig, Scripted};
    use protocol::{InvSlot, ItemRow};
    use sim_core::gather::ItemStack;
    use sim_core::movement::{quant_xz, quant_y};

    const SEED: u64 = 20260731;

    fn survivor() -> Survivor {
        Survivor::new(Mind::inline(Scripted::default(), MindConfig::default()).unwrap())
    }

    /// The catalog outgrew one u128 (127 rows filled it): ids on both
    /// sides of the word boundary and the catalog's last row each land in
    /// their own bit, and nothing past the set is ever in it.
    #[test]
    fn item_bits_hold_ids_across_the_word_boundary() {
        let ids = [0u16, 127, 128, MAX_ITEM_DEFS as u16 - 1];
        assert_eq!(MAX_ITEM_DEFS, 160);
        let mut all = ItemBits::default();
        assert!(!all.any());
        for &id in &ids {
            let one = ItemBits::bit(id);
            assert!(one.any() && one.has(id), "id {id}");
            for &other in &ids {
                assert_eq!(one.has(other), other == id, "id {id} vs {other}");
            }
            all.or(one);
        }
        assert!(ids.iter().all(|&id| all.has(id)));
        assert!(!all.has(1) && !all.has(129) && !all.has(MAX_ITEM_DEFS as u16));
        assert!(!ItemBits::bit(ItemBits::CAP as u16).any());
        assert!(!all.has(u16::MAX));
    }

    /// A welcomed controller with a small catalog: Rock (3), Wood (5) and,
    /// deliberately off the production rows, food at 7 and a hatchet at 9.
    fn fixture() -> (Survivor, ClientView, Target) {
        let mut bot = survivor();
        bot.welcome(&Welcome {
            seed: SEED,
            player_id: 1,
            tick: 0,
            dev: true,
            blight_salt: 0,
        });
        let core = bot.core.as_mut().unwrap();
        core.catalog.count = 10;
        for i in 0..10 {
            let name = format!("Thing {i}");
            core.catalog
                .set(
                    i,
                    name.as_bytes(),
                    ItemRow {
                        stack_max: 100,
                        ..ItemRow::EMPTY
                    },
                )
                .unwrap();
        }
        core.catalog
            .set(
                3,
                b"Bat",
                ItemRow {
                    cond_max: 100,
                    stack_max: 1,
                    ..ItemRow::EMPTY
                },
            )
            .unwrap();
        core.catalog
            .set(
                5,
                b"Wood",
                ItemRow {
                    stack_max: 1000,
                    ..ItemRow::EMPTY
                },
            )
            .unwrap();
        core.catalog
            .set(
                9,
                b"Stone Hatchet",
                ItemRow {
                    cond_max: 100,
                    stack_max: 1,
                    ..ItemRow::EMPTY
                },
            )
            .unwrap();
        core.max_food = 500;
        core.food = 500;
        core.max_water = 250;
        core.water = 250;
        core.hp = 100;
        core.hp_max = 100;
        // Neither row nor hotbar slot matches the production spawn kit.
        core.inv[4] = ItemStack {
            item: 3,
            count: 1,
            cond: 100,
            skin: 0,
        };
        for cz in 40..216 {
            for cx in 40..216 {
                let (seed, island) = core.island();
                let slot = island.cache.slot(seed, island.table, island.haven, cx, cz);
                if slot.occupant != Occupant::Tree {
                    continue;
                }
                let y = terrain::ground(seed, bot.haven.as_ref().unwrap(), slot.x, slot.z - 5.0);
                if y < 1.0 || (y - slot.y).abs() > 0.3 {
                    continue;
                }
                let target = Target {
                    cx: cx as u16,
                    cz: cz as u16,
                    slot,
                };
                let body = EntityState {
                    id: 1,
                    qx: quant_xz(slot.x),
                    qy: quant_y(y),
                    qz: quant_xz(slot.z - 5.0),
                    grounded: true,
                    ..EntityState::default()
                };
                if !visible(core, bot.haven.as_ref().unwrap(), &body, target) {
                    continue;
                }
                let mut view = ClientView::new();
                view.newest_applied = Some(1);
                view.entities.push((1, body));
                return (bot, view, target);
            }
        }
        panic!("no visible tree fixture");
    }

    fn goal(bot: &mut Survivor, goal: Goal, tick: u32) {
        bot.adopt(
            Choice {
                goal,
                confidence: 1.0,
                reason: crate::mind::Reason::EMPTY,
                input_tokens: 0,
                output_tokens: 0,
                say: crate::mind::Say::Quiet,
                code: None,
            },
            tick,
        );
    }

    fn event(bot: &mut Survivor, n: usize, buf: &[u8]) {
        bot.event(&buf[..n]).unwrap();
    }

    #[test]
    fn perception_rejects_behind_distant_harvested_and_wall_occluded_trees() {
        let (mut bot, view, target) = fixture();
        let body = view.get(1).unwrap();
        assert!(in_view(body, &target.slot));
        let mut back = *body;
        back.yaw = 1 << 15;
        assert!(!in_view(&back, &target.slot));
        let mut far = *body;
        far.qz -= quant_xz(SIGHT_M);
        assert!(!in_view(&far, &target.slot));
        let core = bot.core.as_mut().unwrap();
        let haven = bot.haven.as_ref().unwrap();
        assert!(visible(core, haven, body, target));
        use sim_core::build::{BuildContent, PieceRec, BUILD_CELL_M, LOC_EDGE_ZLO, SHAPE_WALL};
        core.piece_defs = BuildContent::probe_fixture();
        core.piece_defs_have = core.piece_defs.piece_count;
        let row = core
            .piece_defs
            .pieces
            .iter()
            .position(|p| p.shape == SHAPE_WALL)
            .unwrap() as u8;
        let cx = (target.slot.x / BUILD_CELL_M).floor() as u16;
        let cz = ((body.qz as f32 * POS_XZ_Q / BUILD_CELL_M).floor() as u16) + 1;
        let rec = PieceRec {
            cx,
            cz,
            row,
            loc: LOC_EDGE_ZLO,
            hp: 100,
            ..PieceRec::default()
        };
        let mut buf = [0u8; protocol::event::MAX_EVENT_MSG_BYTES];
        let n = protocol::event::encode_event_piece_sync(
            true,
            &[(rec, sim_core::build::grid_frame(SEED, haven, cx, cz))],
            &mut buf,
        )
        .unwrap();
        core.on_stream(&buf[..n]).unwrap();
        assert!(
            !visible(core, haven, body, target),
            "a built wall revealed the tree behind it"
        );
        let n = protocol::event::encode_event_piece_sync(true, &[], &mut buf).unwrap();
        core.on_stream(&buf[..n]).unwrap();
        assert!(visible(core, haven, body, target));
        let n = protocol::event::encode_event_slot_change(true, target.cx, target.cz, &mut buf)
            .unwrap();
        event(&mut bot, n, &buf);
        goal(&mut bot, Goal::GatherWood, 1);
        let now = Instant::now();
        for _ in 0..SIGHT_WIDTH * SIGHT_WIDTH {
            bot.frame_at(&view, 1, 1, now);
            assert!(
                bot.target.is_none_or(|t| t.key() != target.key()),
                "selected a harvested tree"
            );
        }
    }

    #[test]
    fn a_full_sweep_publishes_what_is_in_view_and_nothing_behind() {
        let (mut bot, mut view, _) = fixture();
        goal(&mut bot, Goal::Wait, 1);
        let now = Instant::now();
        for _ in 0..SIGHT_WIDTH * SIGHT_WIDTH {
            bot.frame_at(&view, 1, 1, now);
        }
        let ahead = bot.senses.trees;
        assert!(ahead.count > 0 && ahead.nearest_m <= 6, "{ahead:?}");
        // Whatever is published lies inside the 90-degree cone: ahead,
        // ahead-right or ahead-left, never beside or behind.
        for yaw in [1u16 << 14, 1 << 15, 3 << 14] {
            view.entities[0].1.yaw = yaw;
            for _ in 0..SIGHT_WIDTH * SIGHT_WIDTH {
                bot.frame_at(&view, 1, 1, now);
            }
            let seen = bot.senses.trees;
            assert!(
                seen.count == 0 || [7, 0, 1].contains(&seen.bearing),
                "{seen:?}"
            );
        }
    }

    #[test]
    fn stalled_approach_abandons_the_target_instead_of_walking_forever() {
        let (mut bot, mut view, target) = fixture();
        goal(&mut bot, Goal::GatherWood, 1);
        bot.target = Some(target);
        let now = Instant::now();
        assert_ne!(bot.frame_at(&view, 1, 1, now).move_z, 0);
        view.newest_applied = Some(1 + NO_PROGRESS_TICKS);
        bot.frame_at(&view, 1, 2, now);
        assert!(bot.target.is_none());
        assert_eq!(bot.skipped, Some(target.key()));
        // The side-step walks sideways from its first frame, not on into
        // what stopped it while the view comes round.
        let (sx, sz) = yaw_dir(view.get(1).unwrap().yaw.wrapping_add(1 << 14));
        for seq in 3..10 {
            let frame = bot.frame_at(&view, 1, seq, now);
            assert_eq!(bot.stats.phase, Phase::Recovering);
            let (wx, wz) = walked(&frame);
            assert!(wx * sx + wz * sz > 120.0, "({wx}, {wz})");
            assert_eq!(frame.buttons, 0);
        }
    }

    /// A node remembered farther off than the search's 20 s of walking
    /// (60 m): the gather goal walks back to it, where it used to fail
    /// "none in sight" with the node in its own census (#362).
    #[test]
    fn a_gather_walks_back_to_a_node_remembered_beyond_the_search() {
        let (mut bot, mut view, _) = fixture();
        let haven = *bot.haven.as_ref().unwrap();
        let core = bot.core.as_mut().unwrap();
        let mut slot_at = |cx: i32, cz: i32| {
            let (seed, island) = core.island();
            island.cache.slot(seed, island.table, island.haven, cx, cz)
        };
        // An ore node, and dry ground 100 m from it with no ore in sight.
        let r = SIGHT_CELLS + 1;
        let (target, (x, y, z)) = (40..216)
            .flat_map(|cz| (40..216).map(move |cx| (cx, cz)))
            .find_map(|(cx, cz)| {
                let slot = slot_at(cx, cz);
                if Kind::of(slot.occupant) != Some(Kind::Ore) {
                    return None;
                }
                let spot = (0..16).find_map(|i| {
                    let a = i as f32 * std::f32::consts::TAU / 16.0;
                    let (x, z) = (slot.x + 100.0 * a.sin(), slot.z + 100.0 * a.cos());
                    let y = terrain::ground(SEED, &haven, x, z);
                    let (bx, bz) = (
                        (x / CELL_SIZE).floor() as i32,
                        (z / CELL_SIZE).floor() as i32,
                    );
                    let bare = (-r..=r).all(|dz| {
                        (-r..=r).all(|dx| {
                            Kind::of(slot_at(bx + dx, bz + dz).occupant) != Some(Kind::Ore)
                        })
                    });
                    (y > 1.0 && bare).then_some((x, y, z))
                })?;
                let target = Target {
                    cx: cx as u16,
                    cz: cz as u16,
                    slot,
                };
                Some((target, spot))
            })
            .expect("an ore node with bare ground 100 m off");
        view.entities[0].1 = EntityState {
            id: 1,
            qx: quant_xz(x),
            qy: quant_y(y),
            qz: quant_xz(z),
            grounded: true,
            ..EntityState::default()
        };
        let t0 = 10;
        view.newest_applied = Some(t0);
        goal(&mut bot, Goal::GatherOre, t0);
        bot.recall[Kind::Ore as usize] = Some((target, t0));
        let now = Instant::now();
        let until = t0 + (SEARCH_GOAL_SECS + 5) * TICK_HZ;
        for (seq, tick) in (t0..until).step_by(SCAN_CELLS_MAX as usize).enumerate() {
            view.newest_applied = Some(tick);
            bot.frame_at(&view, 1, seq as u16 + 1, now);
        }
        assert!(
            bot.goal.is_some_and(|a| a.goal == Goal::GatherOre),
            "the walk back was cut short: {:?}",
            bot.memory.last
        );
        assert!(
            bot.history
                .iter()
                .all(|r| r.outcome != Outcome::Failed(Why::NotFound)),
            "it gave up the search with the node remembered: {:?}",
            bot.memory.last
        );
        assert_eq!(bot.stats.phase, Phase::Approaching);
        assert!(bot.senses.ore_nodes.count > 0, "the census knows of it");
    }

    /// A harvest swing never lands on a body: one standing across the
    /// swing's ray holds it, one beside the node does not. (Until lane A,
    /// any body within 4 m held every swing, so two bodies at one tree
    /// stood waiting; this test pinned that.)
    #[test]
    fn a_body_on_the_swing_ray_holds_the_harvest_swing() {
        let (mut bot, mut view, target) = fixture();
        // The bystanders here are unarmed: an opportunist would take that
        // opening, and this test is about the harvest.
        bot.combat = Combat::new(Temperament::Defensive);
        let now = Instant::now();
        let radius = terrain::occupant_volume(Occupant::Tree).0 * target.slot.scale;
        let body = &mut view.entities[0].1;
        body.qz = quant_xz(target.slot.z - radius - REACH_M / 2.0);
        body.qy = quant_y(terrain::ground(
            SEED,
            bot.haven.as_ref().unwrap(),
            body.qx as f32 * POS_XZ_Q,
            body.qz as f32 * POS_XZ_Q,
        ));
        let me = *view.get(1).unwrap();
        let (x, y, z) = (
            me.qx as f32 * POS_XZ_Q,
            me.qy as f32 * POS_Y_Q,
            me.qz as f32 * POS_XZ_Q,
        );
        goal(&mut bot, Goal::GatherWood, 1);
        bot.target = Some(target);
        assert_eq!(bot.frame_at(&view, 1, 1, now).buttons, BTN_PRIMARY);
        // Beside the node, off the ray: the swing goes on.
        bot.tracks.stand(2, [x + REACH_M, y, z], true);
        assert_eq!(bot.frame_at(&view, 1, 2, now).buttons, BTN_PRIMARY);
        // On the ray, within reach of a swing: held, the node kept.
        bot.tracks
            .stand(2, [x, y, z + REACH_M + radius + 0.5], true);
        assert_eq!(bot.frame_at(&view, 1, 3, now).buttons, 0);
        assert!(bot.target.is_some());
        // Out of sight is out of mind: the eyes decide, not the snapshot.
        bot.tracks
            .stand(2, [x, y, z + REACH_M + radius + 0.5], false);
        assert_eq!(bot.frame_at(&view, 1, 4, now).buttons, BTN_PRIMARY);
        // A body with a lower id on the ray keeps the node: this one gives
        // way and does not retry it straight away, and it is not counted
        // as stuck.
        bot.tracks
            .stand(0, [x, y, z + REACH_M + radius + 0.5], true);
        assert_eq!(bot.frame_at(&view, 1, 5, now).buttons, 0);
        assert!(bot.target.is_none());
        assert_eq!(bot.skipped, Some(target.key()));
        assert_eq!(bot.stats.gave_way, 1);
        assert_eq!(bot.goal.map(|a| a.abandons), Some(0));
    }

    #[test]
    fn stale_snapshots_wounds_and_broken_tools_stop_work() {
        let (mut bot, mut view, target) = fixture();
        let now = Instant::now();
        goal(&mut bot, Goal::GatherWood, 1);
        bot.target = Some(target);
        bot.frame_at(&view, 1, 1, now);
        let timeout = bot.mind.config().timeout;
        let frame = bot.frame_at(&view, 1, 2, now + timeout);
        assert_eq!((frame.move_z, frame.buttons), (0, 0));
        assert_eq!(bot.stats.phase, Phase::Stale);
        view.newest_applied = Some(3);
        view.entities[0].1.wounded = true;
        let frame = bot.frame_at(&view, 1, 3, now + timeout);
        assert_eq!(
            (frame.move_z, frame.buttons),
            (0, 0),
            "no recent blow: lie still"
        );
        assert_eq!(bot.stats.phase, Phase::Wounded);
        assert!(bot.goal.is_none(), "a wound interrupts the goal");
        view.entities[0].1.wounded = false;
        goal(&mut bot, Goal::GatherWood, 3);
        bot.core.as_mut().unwrap().inv[4].cond = 0;
        bot.frame_at(&view, 1, 4, now + timeout);
        assert!(bot.goal.is_none());
        assert_eq!(
            bot.memory.last.unwrap().outcome,
            Outcome::Failed(Why::NoTool)
        );
    }

    #[test]
    fn death_answers_the_respawn_screen_and_the_wake_resumes_play() {
        let (mut bot, mut view, _) = fixture();
        let now = Instant::now();
        goal(&mut bot, Goal::Explore, 1);
        let mut buf = [0u8; protocol::event::MAX_EVENT_MSG_BYTES];
        let n = protocol::event::encode_event_death(1, 0, 0, 0, 0, 0, &mut buf).unwrap();
        event(&mut bot, n, &buf);
        assert!(bot.core.as_ref().unwrap().dead);
        view.entities[0].1.dead = true;
        let mut out = [0u8; MAX_STREAM_MSG_BYTES];
        let frame = bot.frame_at(&view, 1, 2, now);
        assert_eq!((frame.move_z, frame.buttons), (0, 0));
        assert_eq!(bot.stats.phase, Phase::Dead);
        let len = bot.action(&mut out).expect("the respawn verb");
        assert!(matches!(
            protocol::decode_action(&out[..len]),
            Ok(protocol::ActionMsg::Respawn { on_bag: false })
        ));
        assert_eq!(
            bot.memory.last.unwrap().outcome,
            Outcome::Interrupted(Why::Died)
        );
        // No second ask while the first is fresh; one after the deadline.
        view.newest_applied = Some(10);
        bot.frame_at(&view, 1, 3, now);
        assert!(bot.action(&mut out).is_none());
        view.newest_applied = Some(2 + VERDICT_SECS * TICK_HZ);
        bot.frame_at(&view, 1, 4, now);
        assert!(
            bot.action(&mut out).is_some(),
            "a lost answer is asked again"
        );
        let n = protocol::event::encode_event_respawn(false, &mut buf).unwrap();
        event(&mut bot, n, &buf);
        view.entities[0].1.dead = false;
        assert_eq!((bot.stats.deaths, bot.stats.respawns), (1, 1));
        assert_eq!(bot.memory.trigger, Trigger::Respawned);
        // A new body looks around for one sweep, then chooses again.
        let asked = bot.mind.stats.requests;
        for _ in 0..SIGHT_WIDTH * SIGHT_WIDTH + 1 {
            bot.frame_at(&view, 1, 5, now);
        }
        assert!(bot.mind.stats.requests > asked, "play resumes");
        assert_ne!(bot.stats.phase, Phase::Dead);

        // The screen lists a ready bag of its own (it comes before the
        // death on the event lane): the second death wakes there.
        let body = *view.get(1).unwrap();
        let cell = |q: i32| sim_core::build::build_cell_of(q as f32 * POS_XZ_Q) as u16;
        let bag = sim_core::deploy::BagAnchor {
            cx: cell(body.qx),
            cz: cell(body.qz),
            level: 0,
            ready: true,
            wait_s: 0,
        };
        let die = |bot: &mut Survivor, view: &mut ClientView, seq: u16| {
            let mut buf = [0u8; protocol::event::MAX_EVENT_MSG_BYTES];
            let n = protocol::event::encode_event_bags(&[bag], &mut buf).unwrap();
            event(bot, n, &buf);
            let n = protocol::event::encode_event_death(1, 0, 0, 0, 0, 0, &mut buf).unwrap();
            event(bot, n, &buf);
            view.entities[0].1.dead = true;
            view.newest_applied = Some(view.newest_applied.unwrap() + 1);
            bot.frame_at(view, 1, seq, now);
            let mut out = [0u8; MAX_STREAM_MSG_BYTES];
            let len = bot.action(&mut out).expect("the respawn verb");
            let n = protocol::event::encode_event_respawn(false, &mut buf).unwrap();
            event(bot, n, &buf);
            view.entities[0].1.dead = false;
            protocol::decode_action(&out[..len]).unwrap()
        };
        assert_eq!(bot.home.bags(), 0, "no bag of its own yet");
        assert!(matches!(
            die(&mut bot, &mut view, 6),
            protocol::ActionMsg::Respawn { on_bag: true }
        ));
        assert_eq!(bot.home.bags(), 1, "the screen's list is what it owns");
        // Hurt beside that bag a moment ago (the blow arrives as a person
        // feels it, with the eyes on where it stands): home is being
        // fought over, and the beach is the way back in.
        let me = *view.get(1).unwrap();
        let tick = view.newest_applied.unwrap() + 1;
        keyframe(&mut view, tick, &[me]);
        bot.frame_at(&view, 1, 7, now);
        assert!(!bot.home.under_attack(view.newest_applied.unwrap()));
        let n = protocol::event::encode_event_hurt(0, 15, false, &mut buf).unwrap();
        event(&mut bot, n, &buf);
        assert!(bot.home.under_attack(view.newest_applied.unwrap()));
        assert!(matches!(
            die(&mut bot, &mut view, 8),
            protocol::ActionMsg::Respawn { on_bag: false }
        ));
        assert_eq!(
            (bot.home.stats.wakes_on_bag, bot.home.stats.wakes_on_beach),
            (1, 3)
        );
    }

    #[test]
    fn a_stranger_striking_its_base_raises_the_alarm_and_a_repair_does_not() {
        let (mut bot, _, _) = fixture();
        let plan = sim_core::bots::BasePlan::new(0, 100, 100);
        bot.builder.set_plan(plan);
        let mut buf = [0u8; protocol::event::MAX_EVENT_MSG_BYTES];
        let loc = sim_core::build::LOC_EDGE_XLO;
        // Mended to its whole hp: nobody struck anything.
        let n = protocol::event::encode_event_piece_repaired(
            false, 101, 100, 0, loc, 0, 5, 250, &mut buf,
        )
        .unwrap();
        event(&mut bot, n, &buf);
        assert!(!bot.home.under_attack(0));
        // Somebody's wall off the plot.
        let n =
            protocol::event::encode_event_struct_hit(false, 90, 90, 0, loc, 0, 10, 240, &mut buf)
                .unwrap();
        event(&mut bot, n, &buf);
        assert!(!bot.home.under_attack(0));
        // Its own wall, struck by somebody else.
        let n =
            protocol::event::encode_event_struct_hit(false, 101, 100, 0, loc, 0, 10, 240, &mut buf)
                .unwrap();
        event(&mut bot, n, &buf);
        assert!(bot.home.under_attack(0), "a blow on its own wall");
    }

    #[test]
    fn a_bag_that_failed_is_not_chosen_again_where_it_failed() {
        let (mut bot, mut view, _) = fixture();
        let now = Instant::now();
        let core = bot.core.as_mut().unwrap();
        core.deploy_defs.defs[0] = sim_core::deploy::DeployDef {
            arch: sim_core::deploy::ARCH_BAG,
            hp: 10,
            item: 7,
            ..sim_core::deploy::DeployDef::INERT
        };
        (core.deploy_defs.def_count, core.deploy_defs_have) = (1, 1);
        core.inv[HOTBAR_SLOTS + 2] = ItemStack {
            item: 7,
            count: 1,
            ..ItemStack::default()
        };
        view.newest_applied = Some(1);
        bot.frame_at(&view, 1, 1, now);
        assert!(bot.summary(&view, 1).unwrap().offers(Goal::Bag));
        // The move to the belt goes unanswered: the goal fails here...
        goal(&mut bot, Goal::Bag, 1);
        bot.frame_at(&view, 1, 2, now);
        view.newest_applied = Some(2 + VERDICT_SECS * TICK_HZ);
        bot.frame_at(&view, 1, 3, now);
        assert_eq!(
            bot.memory.last.unwrap().outcome,
            Outcome::Failed(Why::NoAnswer)
        );
        // ...and is not offered again on the same spot, only after a walk.
        view.newest_applied = Some(3 + VERDICT_SECS * TICK_HZ);
        bot.frame_at(&view, 1, 4, now);
        assert!(!bot.summary(&view, 1).unwrap().offers(Goal::Bag));
        view.entities[0].1.qx += ((home::BAG_RETRY_M + 2.0) / POS_XZ_Q) as i32;
        view.newest_applied = Some(4 + VERDICT_SECS * TICK_HZ);
        bot.frame_at(&view, 1, 5, now);
        assert!(bot.summary(&view, 1).unwrap().offers(Goal::Bag));
    }

    #[test]
    fn crafting_resolves_by_name_equips_the_tool_and_reports_it() {
        let (mut bot, view, _) = fixture();
        let now = Instant::now();
        let core = bot.core.as_mut().unwrap();
        // Recipe 2 makes the hatchet from 20 wood; nothing hard-codes 2.
        core.recipes.recipe_count = 3;
        core.recipes_have = 3;
        core.recipes.recipes[2] = sim_core::craft::RecipeDef {
            output: 9,
            out_count: 1,
            ticks: 10,
            station: STATION_NONE,
            blueprint: false,
            unlock: 0,
            n_inputs: 1,
            inputs: [(5, 20), (0, 0), (0, 0), (0, 0)],
        };
        for slot in [0, 1, 2, 3, 5] {
            core.inv[slot] = ItemStack {
                item: 5,
                count: 1,
                cond: 0,
                skin: 0,
            };
        }
        core.inv[7] = ItemStack {
            item: 5,
            count: 30,
            cond: 0,
            skin: 0,
        };
        let hatchet = Name::new(b"Stone Hatchet").unwrap();
        let summary = bot.summary(&view, 1).unwrap();
        assert!(summary.craftable().contains(&hatchet));
        assert!(summary.offers(Goal::Craft(hatchet)));
        goal(&mut bot, Goal::Craft(hatchet), 1);
        bot.frame_at(&view, 1, 1, now);
        let mut out = [0u8; MAX_STREAM_MSG_BYTES];
        let len = bot.action(&mut out).unwrap();
        assert!(matches!(
            protocol::decode_action(&out[..len]),
            Ok(protocol::ActionMsg::Craft {
                recipe: 2,
                count: 1,
                ..
            })
        ));
        let mut buf = [0u8; protocol::event::MAX_EVENT_MSG_BYTES];
        let n = protocol::event::encode_event_craft_done(9, 1, 0, &mut buf).unwrap();
        event(&mut bot, n, &buf);
        let n = protocol::event::encode_event_inv(
            &[InvSlot {
                slot: 8,
                stack: ItemStack {
                    item: 9,
                    count: 1,
                    cond: 100,
                    skin: 0,
                },
            }],
            &mut buf,
        )
        .unwrap();
        event(&mut bot, n, &buf);
        bot.frame_at(&view, 1, 2, now);
        bot.frame_at(&view, 1, 3, now);
        let len = bot.action(&mut out).expect("equip move");
        match protocol::decode_action(&out[..len]).unwrap() {
            protocol::ActionMsg::Move {
                from_kind,
                from_slot,
                to_kind,
                to_slot,
                count,
                ..
            } => {
                assert_eq!((from_kind, to_kind), (CONT_SELF, CONT_SELF));
                assert_eq!((from_slot, count), (8, 1));
                assert!(
                    (to_slot as usize) < HOTBAR_SLOTS && to_slot != 4,
                    "not onto the rock"
                );
            }
            other => panic!("expected a move, got {other:?}"),
        }
        let n = protocol::event::encode_event_moved(CONT_SELF, 8, CONT_SELF, 5, 1, 9, &mut buf)
            .unwrap();
        event(&mut bot, n, &buf);
        bot.frame_at(&view, 1, 4, now);
        let report = bot.memory.last.unwrap();
        assert_eq!((report.outcome, report.gained), (Outcome::Done, 1));
        assert_eq!(bot.stats.equips, 1);
        // A name with no usable recipe fails without sending anything.
        goal(
            &mut bot,
            Goal::Craft(Name::new(b"Metal Hatchet").unwrap()),
            5,
        );
        bot.frame_at(&view, 1, 5, now);
        assert!(bot.action(&mut out).is_none());
        assert_eq!(
            bot.memory.last.unwrap().outcome,
            Outcome::Failed(Why::NoRecipe)
        );
    }

    /// A craft queued behind a long smelt is not offered again, and its
    /// slow answer is not a failure while the queue shows it coming.
    #[test]
    fn a_craft_behind_a_long_batch_is_coming_not_lost() {
        let (mut bot, mut view, _) = fixture();
        let now = Instant::now();
        let core = bot.core.as_mut().unwrap();
        core.recipes.recipe_count = 3;
        core.recipes_have = 3;
        core.recipes.recipes[2] = sim_core::craft::RecipeDef {
            output: 9,
            out_count: 1,
            ticks: 10,
            station: STATION_NONE,
            blueprint: false,
            unlock: 0,
            n_inputs: 1,
            inputs: [(5, 20), (0, 0), (0, 0), (0, 0)],
        };
        core.recipes.recipes[1].ticks = 2 * TICK_HZ;
        core.inv[7] = ItemStack {
            item: 5,
            count: 30,
            cond: 0,
            skin: 0,
        };
        // A smelt of fifty units is in the queue ahead.
        core.jobs[0] = (1, 50);
        core.jobs_count = 1;
        core.craft_eta_ticks = 2 * TICK_HZ as u16;
        let hatchet = Name::new(b"Stone Hatchet").unwrap();
        assert!(bot.summary(&view, 1).unwrap().offers(Goal::Craft(hatchet)));
        view.newest_applied = Some(1);
        goal(&mut bot, Goal::Craft(hatchet), 1);
        bot.frame_at(&view, 1, 1, now);
        let mut out = [0u8; MAX_STREAM_MSG_BYTES];
        assert!(bot.action(&mut out).is_some(), "the craft went out");
        let core = bot.core.as_mut().unwrap();
        core.jobs[1] = (2, 1);
        core.jobs_count = 2;
        assert!(
            !bot.summary(&view, 1).unwrap().offers(Goal::Craft(hatchet)),
            "what the queue holds is not offered again"
        );
        // Past the plain window it still waits: the jobs ahead take longer.
        view.newest_applied = Some(2 + 10 + VERDICT_SECS * TICK_HZ);
        bot.frame_at(&view, 1, 2, now);
        assert_eq!(bot.goal(), Some(Goal::Craft(hatchet)));
        // Past the longest wait, it is left to pay out on its own.
        view.newest_applied = Some(2 + 10 + (QUEUE_WAIT_SECS + VERDICT_SECS) * TICK_HZ);
        bot.frame_at(&view, 1, 3, now);
        assert_eq!(bot.memory.last.unwrap().outcome, Outcome::Done);
        // A full queue takes nothing more.
        let core = bot.core.as_mut().unwrap();
        core.jobs = [(1, 50), (1, 50), (1, 50), (1, 50)];
        core.jobs_count = CRAFT_QUEUE as u8;
        assert!(!bot.summary(&view, 1).unwrap().offers(Goal::Craft(hatchet)));
    }

    /// A goal that fails as it begins twice running is off the list for a
    /// few seconds, so the next choice is something else; one that failed
    /// after trying a while is offered again at once.
    #[test]
    fn a_goal_that_fails_as_it_begins_sits_out_a_few_seconds() {
        let (mut bot, mut view, _) = fixture();
        let flop = |bot: &mut Survivor, at: u32| {
            goal(bot, Goal::Explore, at);
            bot.end_goal(at, Outcome::Failed(Why::Stuck));
        };
        let offered =
            |bot: &Survivor, view: &ClientView| bot.summary(view, 1).unwrap().offers(Goal::Explore);
        view.newest_applied = Some(100);
        flop(&mut bot, 100);
        assert!(offered(&bot, &view), "once may be the moment changing");
        flop(&mut bot, 100);
        assert!(!offered(&bot, &view), "twice running is a loop");
        view.newest_applied = Some(100 + FLOP_RETRY_TICKS);
        assert!(offered(&bot, &view));
        flop(&mut bot, 100 + FLOP_RETRY_TICKS);
        assert!(!offered(&bot, &view), "still the same loop");
        // A run that tried for a while ends the loop.
        let at = 100 + FLOP_RETRY_TICKS;
        goal(&mut bot, Goal::Explore, at);
        bot.end_goal(at + 5 * TICK_HZ, Outcome::Failed(Why::Stuck));
        assert!(offered(&bot, &view));
    }

    #[test]
    fn a_craftable_tool_is_never_crowded_off_the_bounded_list() {
        let (mut bot, view, _) = fixture();
        let core = bot.core.as_mut().unwrap();
        // Every recipe row but the last makes a different plain thing from
        // one wood; the last makes the hatchet. More names than the list
        // holds, so a list filled in recipe order would drop the hatchet.
        let rows = SUMMARY_CRAFTS + 3;
        core.catalog.count = 10 + rows as u16;
        for i in 10..10 + rows {
            let name = format!("Plain Thing {i}");
            let row = ItemRow {
                stack_max: 10,
                ..ItemRow::EMPTY
            };
            core.catalog.set(i, name.as_bytes(), row).unwrap();
        }
        core.recipes.recipe_count = rows as u16;
        core.recipes_have = rows as u16;
        for r in 0..rows {
            let output = if r + 1 == rows { 9 } else { 10 + r as u16 };
            core.recipes.recipes[r] = sim_core::craft::RecipeDef {
                output,
                out_count: 1,
                ticks: 1,
                station: STATION_NONE,
                blueprint: false,
                unlock: 0,
                n_inputs: 1,
                inputs: [(5, 1), (0, 0), (0, 0), (0, 0)],
            };
        }
        core.inv[7] = ItemStack {
            item: 5,
            count: 10,
            cond: 0,
            skin: 0,
        };
        let summary = bot.summary(&view, 1).unwrap();
        assert_eq!(summary.craftable()[0].as_str(), "Stone Hatchet");
        assert!(summary.offers(Goal::Craft(Name::new(b"Stone Hatchet").unwrap())));
    }

    /// Issue #387: a tool already in the pack is not offered again; a worn
    /// out one is.
    #[test]
    fn a_tool_already_carried_is_not_offered_again() {
        let (mut bot, view, _) = fixture();
        let hatchet = Goal::Craft(Name::new(b"Stone Hatchet").unwrap());
        let core = bot.core.as_mut().unwrap();
        core.recipes.recipe_count = 3;
        core.recipes_have = 3;
        core.recipes.recipes[2] = sim_core::craft::RecipeDef {
            output: 9,
            out_count: 1,
            ticks: 10,
            station: STATION_NONE,
            blueprint: false,
            unlock: 0,
            n_inputs: 1,
            inputs: [(5, 20), (0, 0), (0, 0), (0, 0)],
        };
        core.inv[7] = ItemStack {
            item: 5,
            count: 30,
            cond: 0,
            skin: 0,
        };
        assert!(bot.summary(&view, 1).unwrap().offers(hatchet));
        let core = bot.core.as_mut().unwrap();
        core.inv[8] = ItemStack {
            item: 9,
            count: 1,
            cond: 5_000,
            skin: 0,
        };
        assert!(!bot.summary(&view, 1).unwrap().offers(hatchet));
        bot.core.as_mut().unwrap().inv[8].cond = 0;
        assert!(bot.summary(&view, 1).unwrap().offers(hatchet));
    }

    #[test]
    fn a_tool_or_weapon_carried_is_not_offered_again_until_worn_out() {
        // Report 3c435c78: an agent was offered the stone hatchet and
        // pickaxe it carried.
        let (mut bot, view, _) = fixture();
        let core = bot.core.as_mut().unwrap();
        core.recipes.recipe_count = 2;
        core.recipes_have = 2;
        for (r, output) in [(0, 9u16), (1, 3)] {
            core.recipes.recipes[r] = sim_core::craft::RecipeDef {
                output,
                out_count: 1,
                ticks: 1,
                station: STATION_NONE,
                blueprint: false,
                unlock: 0,
                n_inputs: 1,
                inputs: [(5, 1), (0, 0), (0, 0), (0, 0)],
            };
        }
        core.inv[7] = ItemStack {
            item: 5,
            count: 10,
            cond: 0,
            skin: 0,
        };
        let rules = Rules::shipped().unwrap();
        bot.book.learn(&rules, &bot.core.as_ref().unwrap().catalog);
        let (hatchet, bat) = (
            Name::new(b"Stone Hatchet").unwrap(),
            Name::new(b"Bat").unwrap(),
        );
        let s = bot.summary(&view, 1).unwrap();
        assert!(s.offers(Goal::Craft(hatchet)));
        assert!(!s.craftable().contains(&bat), "the bat it carries");
        bot.core.as_mut().unwrap().inv[8] = ItemStack {
            item: 9,
            count: 1,
            cond: 100,
            skin: 0,
        };
        assert!(!bot.summary(&view, 1).unwrap().offers(Goal::Craft(hatchet)));
        // Worn out, making another is the repair.
        bot.core.as_mut().unwrap().inv[8].cond = 0;
        assert!(bot.summary(&view, 1).unwrap().offers(Goal::Craft(hatchet)));
    }

    #[test]
    fn an_external_mind_may_walk_to_the_gate_and_stop_there() {
        // Report 215046-e68f8c35: no goal walked to THE GATE but a meet
        // with a friend, so an agent walked there by trades.
        let (mut bot, mut view, _) = fixture();
        let now = Instant::now();
        let trip = Goal::Go(Name::new(gate::GATE_NAME.as_bytes()).unwrap());
        assert_eq!(trip.label().as_str(), "go:THE GATE");
        assert_eq!(Goal::named("go:THE GATE"), Some(trip));
        let s = bot.summary(&view, 1).unwrap();
        assert!(s.gate.is_some_and(|p| p.distance != Distance::Here));
        assert!(!s.offers(trip), "only a mind that is told of places");
        bot.memory.travels = true;
        assert!(bot.summary(&view, 1).unwrap().offers(trip));
        goal(&mut bot, trip, 1);
        bot.frame_at(&view, 1, 1, now);
        assert_eq!(bot.goal(), Some(trip));
        assert_eq!(bot.stats.phase, Phase::Approaching);
        // Beside the stall it is there: done, and no longer offered.
        let [gx, gz] = gate::gate(bot.core.as_ref().unwrap()).unwrap();
        let haven = *bot.haven.as_ref().unwrap();
        let start = view.entities[0].1;
        let body = &mut view.entities[0].1;
        body.qx = quant_xz(gx + 3.0);
        body.qz = quant_xz(gz);
        body.qy = quant_y(terrain::ground(SEED, &haven, gx + 3.0, gz));
        bot.frame_at(&view, 1, 2, now);
        let report = bot.memory.last.unwrap();
        assert_eq!((report.goal, report.outcome), (trip, Outcome::Done));
        // (The frames read the fixture's scripted mind: it is told again.)
        bot.memory.travels = true;
        assert!(!bot.summary(&view, 1).unwrap().offers(trip), "it is here");
        // A trip that came to nothing is not offered again at once.
        view.entities[0].1 = start;
        assert!(bot.summary(&view, 1).unwrap().offers(trip));
        goal(&mut bot, trip, 3);
        bot.end_goal(3, Outcome::Failed(Why::Stuck));
        assert!(!bot.summary(&view, 1).unwrap().offers(trip));
    }

    /// Walk anywhere (operator, 2026-10-19): one template on offer to a mind
    /// told of places; any filled-in walk answers to it, a bad one does not;
    /// the point is the bearing from where the body faced when it began,
    /// and the walk is done beside it.
    #[test]
    fn an_external_mind_may_walk_anywhere_by_bearing_and_metres() {
        use crate::mind::Walk;
        let (mut bot, mut view, _) = fixture();
        let now = Instant::now();
        let any = Goal::Walk(Walk::ANY);
        assert_eq!(any.label().as_str(), "walk:<bearing>:<metres>");
        let leg = Goal::named("walk:right:40").unwrap();
        assert_eq!(
            leg,
            Goal::Walk(Walk {
                bearing: 2,
                metres: 40
            })
        );
        assert_eq!(leg.label().as_str(), "walk:right:40");
        assert!(
            !bot.summary(&view, 1).unwrap().offers(leg),
            "only a mind told of places"
        );
        bot.memory.travels = true;
        let s = bot.summary(&view, 1).unwrap();
        assert!(s.offers(any) && s.offers(leg));
        assert_eq!(Goal::parse("walk:right:40", s.options()), Some(leg));
        for bad in [
            "walk:up:40",
            "walk:right:5",
            "walk:right:900",
            "walk:<bearing>:<metres>",
        ] {
            assert_eq!(Goal::parse(bad, s.options()), None, "{bad}");
        }
        // The point: 40 m to the body's right, whichever way it faces.
        let body = view.entities[0].1;
        let [tx, tz] = walk_target(
            &body,
            Walk {
                bearing: 2,
                metres: 40,
            },
        );
        let (x, z) = (body.qx as f32 * POS_XZ_Q, body.qz as f32 * POS_XZ_Q);
        let (rx, rz) = yaw_dir(body.yaw.wrapping_add(1 << 14));
        assert!(((tx - x) - rx * 40.0).abs() < 0.01 && ((tz - z) - rz * 40.0).abs() < 0.01);
        let [ex, ez] = walk_target(
            &body,
            Walk {
                bearing: 0,
                metres: 600,
            },
        );
        assert!(
            (0.0..=terrain::ISLAND_SIZE).contains(&ex)
                && (0.0..=terrain::ISLAND_SIZE).contains(&ez)
        );
        goal(&mut bot, leg, 1);
        bot.frame_at(&view, 1, 1, now);
        assert_eq!(bot.goal(), Some(leg));
        assert_eq!(bot.stats.phase, Phase::Approaching);
        // Turned on the way, the point stays where it was; beside it, done.
        let haven = *bot.haven.as_ref().unwrap();
        let b = &mut view.entities[0].1;
        b.yaw = b.yaw.wrapping_add(1 << 15);
        b.qx = quant_xz(tx + 1.0);
        b.qz = quant_xz(tz);
        b.qy = quant_y(terrain::ground(SEED, &haven, tx + 1.0, tz));
        bot.frame_at(&view, 1, 2, now);
        let report = bot.memory.last.unwrap();
        assert_eq!((report.goal, report.outcome), (leg, Outcome::Done));
    }

    #[test]
    fn eating_learns_food_from_the_verdicts_alone() {
        let (mut bot, mut view, _) = fixture();
        let now = Instant::now();
        let core = bot.core.as_mut().unwrap();
        core.food = 100;
        core.inv[0] = ItemStack {
            item: 6,
            count: 1,
            cond: 0,
            skin: 0,
        };
        core.inv[1] = ItemStack {
            item: 7,
            count: 3,
            cond: 0,
            skin: 0,
        };
        let summary = bot.summary(&view, 1).unwrap();
        assert!(summary.offers(Goal::Eat), "untried items may be food");
        goal(&mut bot, Goal::Eat, 1);
        let mut out = [0u8; MAX_STREAM_MSG_BYTES];
        let mut buf = [0u8; protocol::event::MAX_EVENT_MSG_BYTES];
        bot.frame_at(&view, 1, 1, now);
        let len = bot.action(&mut out).unwrap();
        assert!(matches!(
            protocol::decode_action(&out[..len]),
            Ok(protocol::ActionMsg::Consume { slot: 0 })
        ));
        let n = protocol::event::encode_event_consume_refused(REFUSE_C_NOT_FOOD as u8, &mut buf)
            .unwrap();
        event(&mut bot, n, &buf);
        // A mouthful a second, as the server paces it: the next bite waits
        // in the hand rather than in the server's.
        bot.frame_at(&view, 1, 2, now);
        assert!(bot.action(&mut out).is_none(), "a second bite in one tick");
        let gap = crate::pace::gap(crate::pace::Kind::Mouth) as u32;
        view.newest_applied = Some(view.newest_applied.unwrap_or_default() + gap);
        bot.frame_at(&view, 1, 3, now);
        let len = bot.action(&mut out).unwrap();
        assert!(matches!(
            protocol::decode_action(&out[..len]),
            Ok(protocol::ActionMsg::Consume { slot: 1 })
        ));
        let n = protocol::event::encode_event_consumed(7, 1, &mut buf).unwrap();
        event(&mut bot, n, &buf);
        let n = protocol::event::encode_event_vitals(110, 250, 500, 250, &mut buf).unwrap();
        event(&mut bot, n, &buf);
        let book = bot.memory.food;
        assert!(book.not_food.has(6) && book.feeds.has(7));
        assert!(!book.may_feed(6) && book.may_feed(7));
        assert!(!book.waters_known(7));
    }

    /// A beach cell, out from the middle: dry here, the water `salt` (the
    /// sea) or not (a lake) within the drink reach.
    fn shore(haven: &Haven, salt: bool) -> (f32, f32) {
        let centre = terrain::ISLAND_SIZE * 0.5;
        for (dx, dz) in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
            for step in 0..1020 {
                let (x, z) = (
                    centre + dx * step as f32 * 2.0,
                    centre + dz * step as f32 * 2.0,
                );
                if terrain::ground(SEED, haven, x, z) > 0.2
                    && water_in_reach(SEED, x, z) == Some(salt)
                {
                    return (x, z);
                }
            }
        }
        panic!("the island has no such shore");
    }

    fn stand(view: &mut ClientView, haven: &Haven, (x, z): (f32, f32)) {
        let body = &mut view.entities[0].1;
        body.qx = quant_xz(x);
        body.qz = quant_xz(z);
        body.qy = quant_y(terrain::ground(SEED, haven, x, z));
    }

    #[test]
    fn drinking_walks_to_seen_water_and_uses_the_drink_verb_there() {
        let (mut bot, mut view, _) = fixture();
        let now = Instant::now();
        let haven = *bot.haven.as_ref().unwrap();
        stand(&mut view, &haven, shore(&haven, true));
        bot.core.as_mut().unwrap().water = 50;
        goal(&mut bot, Goal::Drink, 1);
        bot.frame_at(&view, 1, 1, now);
        let mut out = [0u8; MAX_STREAM_MSG_BYTES];
        let len = bot.action(&mut out).unwrap();
        assert!(matches!(
            protocol::decode_action(&out[..len]),
            Ok(protocol::ActionMsg::Drink)
        ));
        let mut buf = [0u8; protocol::event::MAX_EVENT_MSG_BYTES];
        let n = protocol::event::encode_event_drank(25, 2, &mut buf).unwrap();
        event(&mut bot, n, &buf);
        assert_eq!(bot.stats.drinks, 1);
        let n = protocol::event::encode_event_vitals(500, 250, 500, 250, &mut buf).unwrap();
        event(&mut bot, n, &buf);
        bot.frame_at(&view, 1, 2, now);
        let report = bot.memory.last.unwrap();
        assert_eq!(
            (report.goal, report.outcome, report.gained),
            (Goal::Drink, Outcome::Done, 1)
        );
        assert!(bot.senses.water.count > 0, "the probe saw the sea");
    }

    /// The sea costs health a mouthful (the water pass), so it is drunk only
    /// thirsty and healthy: not at half a meter, not on half health.
    #[test]
    fn the_sea_waits_for_thirst_and_health() {
        let (mut bot, mut view, _) = fixture();
        let now = Instant::now();
        let haven = *bot.haven.as_ref().unwrap();
        stand(&mut view, &haven, shore(&haven, true));
        let set = |bot: &mut Survivor, water: u16, hp: u16| {
            let core = bot.core.as_mut().unwrap();
            (core.max_water, core.water, core.hp_max, core.hp) = (100, water, 100, hp);
        };
        set(&mut bot, 45, 100);
        bot.frame_at(&view, 1, 1, now);
        assert!(
            bot.senses.water.count > 0 && bot.senses.water_salt,
            "the probe saw the sea"
        );
        let summary = bot.summary(&view, 1).unwrap();
        assert!(!summary.offers(Goal::Drink), "the sea at 45% water");
        set(&mut bot, 20, 100);
        assert!(
            bot.summary(&view, 1).unwrap().offers(Goal::Drink),
            "thirsty"
        );
        set(&mut bot, 20, 40);
        assert!(
            !bot.summary(&view, 1).unwrap().offers(Goal::Drink),
            "too hurt for salt"
        );
    }

    /// Where a frame walks, in the world: its axes turned by its view.
    fn walked(frame: &InputFrame) -> (f32, f32) {
        let (fx, fz) = yaw_dir(frame.yaw);
        let (ahead, right) = (f32::from(frame.move_z), f32::from(frame.move_x));
        (fx * ahead + fz * right, fz * ahead - fx * right)
    }

    /// A blow pauses the goal, never ends it: the body backs away from every
    /// announced bearing with the goal waiting, a short fight hands the goal
    /// back where it was, and one that outlasts `RESUME_TICKS` ends it as
    /// fought, so the mind chooses afresh. (Until the frame arbitration, a
    /// hit ended the goal outright; this test pinned that.)
    #[test]
    fn damage_pauses_work_and_escapes_away_from_every_announced_bearing() {
        use sim_core::input::BTN_JUMP;
        let (mut bot, mut view, target) = fixture();
        let now = Instant::now();
        let cap = bot.hands().skill().max_turn();
        let mut buf = [0u8; protocol::event::MAX_EVENT_MSG_BYTES];
        let mut last = view.get(1).unwrap().yaw;
        for sector in 0..sim_core::combat::HURT_SECTORS {
            view.newest_applied = Some(1 + u32::from(sector));
            goal(&mut bot, Goal::GatherWood, 1 + u32::from(sector));
            bot.target = Some(target);
            let n = protocol::event::encode_event_hurt(sector, 15, false, &mut buf).unwrap();
            event(&mut bot, n, &buf);
            // From the first frame the body backs away from the blow, while
            // the hands are still turning to face where it came from.
            let (_, away) = bot.combat.retreat().unwrap();
            let (ax, az) = yaw_dir(away);
            let mut frame = bot.frame_at(&view, 1, 2, now);
            for _ in 0..40 {
                assert_eq!(bot.stats.phase, Phase::Fleeing);
                // A guard walks back while it looks for the attacker, and
                // one caught on a lip may jump. (It sprinted until lane A:
                // the body it looks for must still be near when found.)
                assert_eq!(frame.buttons & !BTN_JUMP, 0, "a guard must release primary");
                assert!(bot.target.is_none());
                assert!(
                    bot.goal.is_some_and(|a| a.paused.is_some()),
                    "a hit pauses the goal"
                );
                let (wx, wz) = walked(&frame);
                assert!(wx * ax + wz * az > 100.0, "walked toward the blow");
                assert!((frame.yaw.wrapping_sub(last) as i16).unsigned_abs() <= cap);
                last = frame.yaw;
                if bot.hands().settled() {
                    break;
                }
                frame = bot.frame_at(&view, 1, 2, now);
            }
            // Turned, it faces the blow's bearing and backs away.
            assert!(frame.move_z < 0);
            let (fx, fz) = yaw_dir(frame.yaw);
            assert_eq!(
                sim_core::combat::bearing_sector((fx * 1000000.0) as i64, (fz * 1000000.0) as i64),
                sector
            );
        }
        // A short fight: the goal carries on, uninterrupted.
        let (start, _) = bot.combat.retreat().unwrap();
        view.newest_applied = Some(start + FLEE_TICKS);
        let frame = bot.frame_at(&view, 1, 4, now);
        assert_ne!(bot.stats.phase, Phase::Fleeing);
        assert_eq!(frame.buttons & BTN_PRIMARY, 0);
        assert!(bot
            .goal
            .is_some_and(|a| a.goal == Goal::GatherWood && a.paused.is_none()));
        assert_eq!(bot.stats.goals_interrupted, 0);
        assert_eq!(bot.stats.goals_resumed, 1);

        // A long one: blows every five seconds for half a minute.
        let t0 = start + FLEE_TICKS + 1;
        for k in 0..7 {
            view.newest_applied = Some(t0 + k * 5 * TICK_HZ);
            bot.frame_at(&view, 1, 5, now);
            if k > 0 {
                assert_eq!(bot.stats.phase, Phase::Fleeing);
                assert!(bot.goal.is_some_and(|a| a.paused.is_some()));
            }
            let n = protocol::event::encode_event_hurt(0, 15, false, &mut buf).unwrap();
            event(&mut bot, n, &buf);
        }
        // An answer chosen before all that is stale by the end of it.
        bot.deferred = Some((
            Choice {
                goal: Goal::Wait,
                confidence: 1.0,
                reason: crate::mind::Reason::EMPTY,
                input_tokens: 0,
                output_tokens: 0,
                say: crate::mind::Say::Quiet,
                code: None,
            },
            now,
        ));
        let (start, _) = bot.combat.retreat().unwrap();
        view.newest_applied = Some(start + FLEE_TICKS);
        bot.frame_at(&view, 1, 6, now);
        let last = bot.memory.last.unwrap();
        assert_eq!(
            (last.goal, last.outcome),
            (Goal::GatherWood, Outcome::Interrupted(Why::Fight))
        );
        assert_eq!(bot.memory.trigger, Trigger::Fought);
        assert_eq!(bot.stats.goals_resumed, 1);
        assert_eq!(bot.stats.deferred_dropped, 1);
        assert_eq!(bot.goal(), None, "the mind chooses again");
    }

    /// An answer that lands mid-fight waits for the fight to end and is
    /// then judged the way the mind judges its own: adopted only while it
    /// is fresh and on offer, and never carried into a new situation.
    /// (Until the review, a held answer was adopted as it was.)
    #[test]
    fn an_answer_mid_fight_is_held_and_judged_when_the_fight_ends() {
        let (mut bot, mut view, _) = fixture();
        let now = Instant::now();
        let mut buf = [0u8; protocol::event::MAX_EVENT_MSG_BYTES];
        let mut hurt = |bot: &mut Survivor| {
            let n = protocol::event::encode_event_hurt(2, 15, false, &mut buf).unwrap();
            event(bot, n, &buf);
        };
        let choice = |goal| Choice {
            goal,
            confidence: 1.0,
            reason: crate::mind::Reason::EMPTY,
            input_tokens: 0,
            output_tokens: 0,
            say: crate::mind::Say::Quiet,
            code: None,
        };
        goal(&mut bot, Goal::GatherWood, 1);
        hurt(&mut bot);
        bot.frame_at(&view, 1, 1, now);
        assert!(bot.combat.engaged());
        // Ask while backing away: told it is thirsty with water on offer,
        // the scripted mind answers drink, and the answer lands mid-fight.
        let mut summary = bot.summary(&view, 1).unwrap();
        summary.water = 1;
        summary.current = None;
        summary.offer(Goal::Drink);
        assert!(bot.mind.ask(now, &summary));
        bot.frame_at(&view, 1, 2, now);
        assert_eq!(bot.stats.deferred, 1);
        assert!(bot.deferred.is_some_and(|(c, _)| c.goal == Goal::Drink));
        assert_eq!(bot.goal(), Some(Goal::GatherWood), "held, not adopted");
        assert_eq!(bot.stats.phase, Phase::Fleeing);
        // The fight ends with the water meter full: drink is not on offer,
        // so the answer is dropped and the paused goal carries on.
        let (start, _) = bot.combat.retreat().unwrap();
        view.newest_applied = Some(start + FLEE_TICKS);
        bot.frame_at(&view, 1, 3, now);
        assert!(bot.deferred.is_none());
        assert_eq!(bot.stats.deferred_dropped, 1);
        assert_eq!(bot.goal(), Some(Goal::GatherWood));
        assert_eq!(bot.stats.goals_resumed, 1);

        // Fresh and on offer after a short fight: adopted over the goal.
        let tick = start + FLEE_TICKS + 1;
        view.newest_applied = Some(tick);
        hurt(&mut bot);
        bot.frame_at(&view, 1, 4, now);
        bot.deferred = Some((choice(Goal::Explore), now));
        let (start, _) = bot.combat.retreat().unwrap();
        view.newest_applied = Some(start + FLEE_TICKS);
        bot.frame_at(&view, 1, 5, now);
        assert_eq!(bot.goal(), Some(Goal::Explore));
        assert_eq!(
            bot.memory.last.map(|r| (r.goal, r.outcome)),
            Some((Goal::GatherWood, Outcome::Interrupted(Why::Replaced)))
        );

        // Older than the mind's own timeout by the end: dropped.
        view.newest_applied = Some(start + FLEE_TICKS + 1);
        hurt(&mut bot);
        bot.frame_at(&view, 1, 6, now);
        bot.deferred = Some((choice(Goal::Wait), now));
        let (start, _) = bot.combat.retreat().unwrap();
        view.newest_applied = Some(start + FLEE_TICKS);
        let late = now + bot.mind.config().timeout;
        bot.frame_at(&view, 1, 7, late);
        assert_eq!(bot.goal(), Some(Goal::Explore));
        assert_eq!(bot.stats.deferred_dropped, 2);

        // A body that goes down takes no answer from its old life along.
        view.newest_applied = Some(start + FLEE_TICKS + 1);
        hurt(&mut bot);
        bot.frame_at(&view, 1, 8, late);
        bot.deferred = Some((choice(Goal::Wait), late));
        view.entities[0].1.wounded = true;
        bot.frame_at(&view, 1, 9, late);
        assert!(bot.deferred.is_none());
        assert_eq!(bot.stats.phase, Phase::Wounded);
    }

    /// An idle body looks about: while it waits, the view leaves where it
    /// was, to either side and over the shoulder, at the hands' pace, and
    /// never moves the body.
    #[test]
    fn an_idle_body_glances_around() {
        let (mut bot, mut view, _) = fixture();
        let now = Instant::now();
        goal(&mut bot, Goal::Wait, 1);
        let start = view.get(1).unwrap().yaw;
        let cap = bot.hands().skill().max_turn();
        let (mut last, mut widest) = (start, 0u16);
        for tick in 1..4 * GLANCE_TICKS {
            view.newest_applied = Some(tick);
            goal(&mut bot, Goal::Wait, tick);
            let frame = bot.frame_at(&view, 1, tick as u16, now);
            assert_eq!((frame.move_x, frame.move_z, frame.buttons), (0, 0, 0));
            assert!((frame.yaw.wrapping_sub(last) as i16).unsigned_abs() <= cap);
            last = frame.yaw;
            widest = widest.max((frame.yaw.wrapping_sub(start) as i16).unsigned_abs());
        }
        // Over the shoulder: well past the cone's edge.
        assert!(widest >= 0x6000, "looked at most {widest} away");
    }

    #[test]
    fn a_wounded_body_crawls_away_from_a_fresh_blow() {
        let (mut bot, mut view, _) = fixture();
        let now = Instant::now();
        let mut buf = [0u8; protocol::event::MAX_EVENT_MSG_BYTES];
        let n = protocol::event::encode_event_hurt(4, 30, false, &mut buf).unwrap();
        event(&mut bot, n, &buf);
        view.entities[0].1.wounded = true;
        let (_, away) = bot.last_hurt.unwrap();
        let mut frame = bot.frame_at(&view, 1, 2, now);
        // The hands turn the body round at their own speed while it crawls
        // away from the first frame, never toward the blow: straight
        // away, or to the nearest cover that lies that way.
        let (ax, az) = yaw_dir(away);
        // 60° in yaw units: the cone `cover::AHEAD_COS` looks for cover in.
        const COVER_CONE: u16 = 0x2AAB;
        let mut last = frame.yaw.wrapping_add(1);
        for _ in 0..40 {
            assert_eq!(bot.stats.phase, Phase::Wounded);
            let (wx, wz) = walked(&frame);
            // Inside the cone cover is looked for in (`cover::AHEAD_COS`,
            // 60°): straight away, or toward cover that lies that way. It
            // read 120 (19°) until the nearest trunk to this fixture moved.
            assert!(
                wx * ax + wz * az > 127.0 * cover::AHEAD_COS,
                "crawled ({wx}, {wz})"
            );
            assert_eq!(frame.buttons, 0, "no swing, no sprint");
            let off = (frame.yaw.wrapping_sub(away) as i16).unsigned_abs();
            if frame.yaw == last && off <= COVER_CONE {
                break;
            }
            last = frame.yaw;
            frame = bot.frame_at(&view, 1, 2, now);
        }
        let off = (frame.yaw.wrapping_sub(away) as i16).unsigned_abs();
        assert!(off <= COVER_CONE, "settled {off} off the way away");
        // Found once, cover is kept; none found is looked for again only
        // after a while.
        let Some((_, at)) = bot.crawl_to else {
            panic!("never looked for cover");
        };
        assert_eq!(at, 1, "looked for cover again every frame");
    }

    #[test]
    fn receipts_count_as_gathered_and_the_pack_limits_what_is_worth_gathering() {
        let (mut bot, view, _) = fixture();
        goal(&mut bot, Goal::GatherWood, 1);
        let now = Instant::now();
        bot.frame_at(&view, 1, 1, now);
        let mut buf = [0u8; protocol::event::MAX_EVENT_MSG_BYTES];
        let n = protocol::event::encode_event_gather(5, 25, 0, &mut buf).unwrap();
        event(&mut bot, n, &buf);
        assert_eq!(bot.gathered_of("Wood"), 25);
        assert_eq!(bot.stats.gather_awards, 1);
        assert!(bot.memory.yields[Kind::Wood as usize].any());
        let core = bot.core.as_mut().unwrap();
        core.inv.fill(ItemStack {
            item: 6,
            count: 100,
            cond: 0,
            skin: 0,
        });
        core.inv[4] = ItemStack {
            item: 3,
            count: 1,
            cond: 100,
            skin: 0,
        };
        let summary = bot.summary(&view, 1).unwrap();
        assert!(
            !summary.offers(Goal::GatherWood),
            "a full pack takes the offer away"
        );
        bot.frame_at(&view, 1, 2, now);
        assert_eq!(
            bot.memory.last.unwrap().outcome,
            Outcome::Failed(Why::PackFull)
        );
        // A stack of the yield with room is still room.
        let core = bot.core.as_mut().unwrap();
        core.inv[7] = ItemStack {
            item: 5,
            count: 10,
            cond: 0,
            skin: 0,
        };
        assert!(room_for(core, bot.memory.yields[Kind::Wood as usize]));
        assert!(bot.summary(&view, 1).unwrap().offers(Goal::GatherWood));
    }

    /// Apply one zero-state snapshot holding `bodies`, as the shard sends.
    fn keyframe(view: &mut ClientView, tick: u32, bodies: &[EntityState]) {
        let mut buf = [0u8; sim_core::limits::DATAGRAM_BUDGET_BYTES];
        let header = protocol::SnapshotHeader {
            tick,
            baseline_age: 0,
            last_executed_seq: 0,
            nudge: protocol::Nudge::Ok,
            buffered_depth: 0,
            repeat_count: 0,
        };
        let n = protocol::encode_snapshot(&header, &[], bodies, &[], &mut buf).unwrap();
        view.apply(&buf[..n]).unwrap();
    }

    /// A player standing on the ground at `(x, z)`.
    fn stander(id: u32, haven: &Haven, x: f32, z: f32) -> EntityState {
        EntityState {
            id,
            qx: quant_xz(x),
            qy: quant_y(terrain::ground(SEED, haven, x, z)),
            qz: quant_xz(z),
            grounded: true,
            pitch: 128,
            ..EntityState::default()
        }
    }

    #[test]
    fn only_bodies_seen_in_the_cone_with_a_held_sight_line_become_tracks() {
        let (mut bot, view, _) = fixture();
        let me = *view.get(1).unwrap();
        let haven = *bot.haven.as_ref().unwrap();
        let (mx, mz) = (me.qx as f32 * POS_XZ_Q, me.qz as f32 * POS_XZ_Q);
        let eye = eye_point(&me);
        let core = bot.core.as_mut().unwrap();
        // A facing with a body the ground hides well inside the day's range
        // (120 m scaled by the weather), and one in the open close by in
        // the same cone.
        let mut pick = None;
        'search: for r in 8..40 {
            for k in 0..32u16 {
                let yaw = k << 11;
                let (fx, fz) = yaw_dir(yaw);
                let (hx, hz) = (mx + fx * r as f32 * 2.0, mz + fz * r as f32 * 2.0);
                let b = stander(0, &haven, hx, hz);
                let to = [hx, b.qy as f32 * POS_Y_Q + 1.4, hz];
                if tracks::clear_line(core, &haven, eye, to, tracks::PLAYER_SIGHT_M)
                    != Sight::Terrain
                {
                    continue;
                }
                for step in 4..12 {
                    for side in -3..=3 {
                        let (f, sd) = (step as f32 * 2.0, side as f32 * 2.0);
                        let (x, z) = (mx + fx * f + fz * sd, mz + fz * f - fx * sd);
                        let b = stander(0, &haven, x, z);
                        let to = [x, b.qy as f32 * POS_Y_Q + 1.4, z];
                        if tracks::clear_line(core, &haven, eye, to, tracks::PLAYER_SIGHT_M)
                            == Sight::Clear
                        {
                            pick = Some((yaw, (x, z), (hx, hz)));
                            break 'search;
                        }
                    }
                }
            }
        }
        let (yaw, (ox, oz), (hx, hz)) = pick.expect("a hidden spot and an open one ahead");
        let (fx, fz) = yaw_dir(yaw);
        let mut me = me;
        me.yaw = yaw;
        let bodies = [
            me,
            stander(2, &haven, ox, oz),
            stander(3, &haven, hx, hz),
            // Behind, close and in the open: outside the cone.
            stander(4, &haven, mx - fx * 6.0, mz - fz * 6.0),
        ];
        let mut view = ClientView::new();
        goal(&mut bot, Goal::Wait, 1);
        let now = Instant::now();
        for tick in 2..40u32 {
            keyframe(&mut view, tick, &bodies);
            bot.frame_at(&view, 1, tick as u16, now);
            if tick == 2 {
                assert!(bot.tracks.get(2).is_none(), "a glimpse is not a sighting");
            }
            assert!(bot.tracks.get(3).is_none(), "seen through the ground");
            assert!(bot.tracks.get(4).is_none(), "seen behind its back");
        }
        assert!(
            bot.tracks.stats.coarse_rejects > 0,
            "the hidden body was in range and a sight line was cast at it"
        );
        let seen = bot.tracks.get(2).expect("the body in the open is seen");
        assert!(seen.visible && seen.species == Species::Player);
        assert_eq!(bot.tracks.seen().count(), 1);
        assert_eq!(bot.senses.players.count, 1);
        // A shot from behind is heard as a sound from behind, not a body.
        let mut buf = [0u8; protocol::event::MAX_EVENT_MSG_BYTES];
        let n = protocol::event::encode_event_shot(4, 0, 128, 0, 900, 10, &mut buf).unwrap();
        event(&mut bot, n, &buf);
        let heard = *bot.tracks.heard().last().expect("the shot was heard");
        assert_eq!(heard.sound, tracks::Sound::Shot);
        assert_eq!(heard.band, tracks::Band::Near);
        let off = heard.bearing.wrapping_sub(yaw.wrapping_add(1 << 15)) as i16;
        assert!(i32::from(off).abs() <= 2 * tracks::HEAR_BEARING_NOISE as i32);
        assert!(bot.tracks.get(4).is_none());
        // Turned away, the body is remembered where it stood, out of sight.
        let mut turned = bodies;
        turned[0].yaw = yaw.wrapping_add(1 << 15);
        for tick in 40..50u32 {
            keyframe(&mut view, tick, &turned);
            bot.frame_at(&view, 1, tick as u16, now);
        }
        let kept = bot.tracks.get(2).expect("remembered");
        assert!(!kept.visible);
        assert!(bot.tracks.aim_pose(2).is_none(), "nobody aims at a memory");
    }

    /// #363: a walk home a shift's end cut short was still the goal when
    /// the next shift began half an hour later, and its clock, counting the
    /// time away, failed it as stuck at once ("go_home failed after 1900
    /// s"). A new session ends what the last one left running, at the time
    /// it really ran.
    #[test]
    fn a_goal_the_last_session_left_running_ends_with_that_session() {
        let (mut bot, _, _) = fixture();
        goal(&mut bot, Goal::GoHome, 100);
        bot.seen_tick = Some(100 + 30 * TICK_HZ);
        bot.welcome(&Welcome {
            seed: SEED,
            player_id: 1,
            tick: 100 + 1900 * TICK_HZ,
            dev: true,
            blight_salt: 0,
        });
        assert_eq!(bot.goal(), None, "nothing carried over");
        let last = bot.memory.last.expect("the walk's report");
        assert_eq!(last.goal, Goal::GoHome);
        assert_eq!(last.outcome, Outcome::Interrupted(Why::Session));
        assert_eq!(last.secs, 30, "the time it ran, not the time away");
        assert!(
            !bot.home_held(100 + 1900 * TICK_HZ),
            "a cut-short walk is not a failed one"
        );
    }

    /// A walk home that could not reach home is not offered again from
    /// about the same spot (a report, 2026-10-19: offered, failed in 0 s,
    /// offered again a minute later, the body starving), and the summary
    /// says why; a walk to the stations that came to nothing holds the
    /// crafts that need one (Flint, the same day: 2,135 in half an hour).
    #[test]
    fn a_walk_that_could_not_get_there_is_not_offered_again_from_the_same_spot() {
        let (mut bot, view, _) = fixture();
        bot.here = [100.0, 100.0];
        bot.home_stuck = Some((1000, [100.0, 100.0]));
        assert!(
            bot.home_unreachable(1000 + GO_HOME_RETRY_TICKS + 1),
            "held on the spot"
        );
        assert!(
            !bot.home_unreachable(1000 + HOME_STUCK_TICKS),
            "not for ever"
        );
        bot.here = [100.0 + HOME_STUCK_NEAR_M + 1.0, 100.0];
        assert!(
            !bot.home_unreachable(1000 + GO_HOME_RETRY_TICKS + 1),
            "moved off"
        );
        assert!(
            bot.home_unreachable(1000 + GO_HOME_RETRY_TICKS - 1),
            "the minute anywhere"
        );
        bot.memory.home = HomeSense {
            state: HomeState::Built,
            unreachable: true,
            ..HomeSense::default()
        };
        let v = bot.summary(&view, 1).unwrap().to_json();
        assert!(v["home"]["no_way_from_here"].is_string(), "{}", v["home"]);

        // A hatchet made at the bench, from wood in the pack.
        let core = bot.core.as_mut().unwrap();
        core.recipes.recipe_count = 3;
        core.recipes_have = 3;
        core.recipes.recipes[2] = sim_core::craft::RecipeDef {
            output: 9,
            out_count: 1,
            ticks: 10,
            station: sim_core::craft::STATION_WORKBENCH1,
            blueprint: false,
            unlock: 0,
            n_inputs: 1,
            inputs: [(5, 20), (0, 0), (0, 0), (0, 0)],
        };
        core.inv[7] = ItemStack {
            item: 5,
            count: 30,
            cond: 0,
            skin: 0,
        };
        bot.memory.stations = Stations {
            bench: Some([0.0, 0.0]),
            tier: 1,
            ..Stations::NONE
        };
        let hatchet = Goal::Craft(Name::new(b"Stone Hatchet").unwrap());
        assert!(bot.summary(&view, 1).unwrap().offers(hatchet));
        bot.memory.station_held = true;
        assert!(!bot.summary(&view, 1).unwrap().offers(hatchet));
    }

    /// A turn built from a real view of the island, with a home, a station
    /// and a full pack, matches the published schema
    /// (`schema/agent-turn-1.schema.json`) as the empty one does.
    #[test]
    fn a_turn_from_a_real_view_matches_the_published_schema() {
        let (mut bot, view, _) = fixture();
        bot.memory.home = HomeSense {
            state: HomeState::Built,
            unreachable: true,
            ..HomeSense::default()
        };
        bot.memory.stations = Stations {
            bench: Some([0.0, 0.0]),
            tier: 1,
            ..Stations::NONE
        };
        bot.core.as_mut().unwrap().inv[7] = ItemStack {
            item: 5,
            count: 30,
            cond: 0,
            skin: 0,
        };
        let turn = crate::external::External::observation(1, &bot.summary(&view, 1).unwrap());
        let errs = crate::external::turn_schema::errors(&turn);
        assert!(errs.is_empty(), "{errs:#?}\n{turn:#}");
    }

    #[test]
    fn the_observation_is_a_pure_function_of_received_state() {
        let (bot, mut view, _) = fixture();
        let a = bot.summary(&view, 1).unwrap().to_json();
        assert_eq!(a, bot.summary(&view, 1).unwrap().to_json(), "deterministic");
        // A body that is in the snapshot but not seen changes nothing: the
        // encoder reads sightings, never the entity list.
        let mut other = *view.get(1).unwrap();
        other.id = 2;
        other.qx += quant_xz(3.0);
        view.entities.push((2, other));
        assert_eq!(a, bot.summary(&view, 1).unwrap().to_json());
        let text = a.to_string();
        for banned in ["seed", "20260731", "qx", "player_id"] {
            assert!(!text.contains(banned), "{banned} in {text}");
        }
    }

    /// An external mind that holds its answer for a newer look at the
    /// decision and says yes to the invite on it. The decision it was
    /// asked first has no invite on it and offers a flight from the
    /// stranger; the newer look must offer the yes and no flight.
    struct YesOnTheNewerLook;

    impl crate::mind::DecisionSource for YesOnTheNewerLook {
        fn kind(&self) -> SourceKind {
            SourceKind::External
        }

        fn decide(&mut self, _: &Summary) -> Result<Choice, String> {
            Err("no newer look came".into())
        }

        fn decide_fresh(
            &mut self,
            first: &Summary,
            fresher: &mut dyn FnMut() -> Option<Summary>,
        ) -> Result<Choice, String> {
            if first.offers(Goal::JoinTeam) || !first.offers(Goal::Flee) {
                return Err("the first look was not the stale one".into());
            }
            let until = Instant::now() + std::time::Duration::from_secs(10);
            while Instant::now() < until {
                if let Some(newer) = fresher() {
                    if !newer.offers(Goal::JoinTeam) || newer.offers(Goal::Flee) {
                        return Err("the newer look did not show the invite".into());
                    }
                    return Ok(Choice {
                        goal: Goal::JoinTeam,
                        confidence: 1.0,
                        reason: crate::mind::Reason::EMPTY,
                        input_tokens: 0,
                        output_tokens: 0,
                        say: crate::mind::Say::Quiet,
                        code: None,
                    });
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            Err("no newer look came".into())
        }
    }

    /// 2026-10-19 (Moss): a harness sat over a minute on each decision; a
    /// person beside it invited it meanwhile. It read the decision asked
    /// before the invite (no join_team, flight and fight from the inviter),
    /// and the yes it gave a decision later came after the invite lapsed.
    /// Now the invite sends a newer look at the decision out, under the
    /// same request: the inviter is no threat on it, the yes is on offer,
    /// and the yes goes to the shard. An inviter seen swinging is sized up
    /// again like anyone.
    #[test]
    fn an_invite_while_the_mind_thinks_is_on_the_decision_it_is_reading() {
        let (mut bot, view, _) = fixture();
        let me = *view.get(1).unwrap();
        let haven = *bot.haven.as_ref().unwrap();
        let (mx, mz) = (me.qx as f32 * POS_XZ_Q, me.qz as f32 * POS_XZ_Q);
        let eye = eye_point(&me);
        let core = bot.core.as_mut().unwrap();
        // A person in the open a few metres ahead, as the inviter stood.
        let mut pick = None;
        'search: for k in 0..32u16 {
            let yaw = k << 11;
            let (fx, fz) = yaw_dir(yaw);
            for step in 2..6 {
                let (x, z) = (mx + fx * step as f32, mz + fz * step as f32);
                let b = stander(0, &haven, x, z);
                let to = [x, b.qy as f32 * POS_Y_Q + 1.4, z];
                if tracks::clear_line(core, &haven, eye, to, tracks::PLAYER_SIGHT_M) == Sight::Clear
                {
                    pick = Some((yaw, x, z));
                    break 'search;
                }
            }
        }
        let (yaw, x, z) = pick.expect("an open spot ahead");
        let mut me = me;
        me.yaw = yaw;
        let bodies = [me, stander(2, &haven, x, z)];
        let mut view = ClientView::new();
        goal(&mut bot, Goal::Wait, 1);
        let now = Instant::now();
        let mut tick = 2u32;
        let mut frame = |bot: &mut Survivor, view: &mut ClientView| {
            keyframe(view, tick, &bodies);
            bot.frame_at(view, 1, tick as u16, now);
            tick += 1;
        };
        for _ in 0..60 {
            if bot.tracks.get(2).is_some() {
                break;
            }
            frame(&mut bot, &mut view);
        }
        frame(&mut bot, &mut view);
        assert_eq!(bot.senses.players.count, 1, "the person is seen");
        // The decision goes out with the person a stranger, and its mind
        // takes its time over it.
        bot.mind = Mind::new(YesOnTheNewerLook, MindConfig::default()).unwrap();
        let first = bot.summary(&view, 1).unwrap();
        assert!(first.offers(Goal::Flee) && !first.offers(Goal::JoinTeam));
        assert!(bot.mind.ask(Instant::now(), &first));
        // The person invites it.
        let mut buf = [0u8; protocol::event::MAX_EVENT_MSG_BYTES];
        let invite = protocol::TeamView {
            invite: 2,
            ..protocol::TeamView::NONE
        };
        let n = protocol::event::encode_event_team(&invite, &mut buf).unwrap();
        event(&mut bot, n, &buf);
        for _ in 0..2000 {
            frame(&mut bot, &mut view);
            if bot.goal() == Some(Goal::JoinTeam) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert_eq!(
            bot.goal(),
            Some(Goal::JoinTeam),
            "the yes on the newer look was taken: {:?}",
            bot.mind.stats
        );
        let (out, len) = bot.outbox.expect("the yes is on its way");
        assert!(matches!(
            protocol::decode_action(&out[..len]),
            Ok(protocol::ActionMsg::Team {
                op: protocol::TEAM_OP_ACCEPT,
                target: 2
            })
        ));
        assert_eq!(bot.senses.players.count, 0, "the inviter is no threat");
        // A swing from the inviter: sized up like anyone again.
        let n = protocol::event::encode_event_swing(2, &mut buf).unwrap();
        event(&mut bot, n, &buf);
        frame(&mut bot, &mut view);
        frame(&mut bot, &mut view);
        assert_eq!(
            bot.senses.players.count, 1,
            "an inviter who swings is watched"
        );
    }
}
