//! The local controller behind an agent player: skills that carry out one
//! goal (`mind::Goal`) with ordinary player inputs and actions, over the
//! human client's decoded state. A decision source chooses goals; this file
//! executes them and says how they went. Geometry is shared with the
//! renderer/sim and read from the same seed, slot deltas and pieces the
//! client holds, never from a server `World`.
//!
//! Reflexes run under every goal and need no decision: answer the death
//! screen with the respawn verb, crawl away while wounded, and back away
//! from a hit. The only verbs sent are the ones a human client sends:
//! input frames, and `Respawn`, `Craft`, `Consume`, `Drink` and `Move`
//! actions (`crates/server/tests/agent_walls.rs` holds that to the client).

use crate::botclient::BotDriver;
use crate::mind::{
    BodyState, Choice, Goal, History, Mind, Name, Outcome, Report, Sighting, Summary, Trigger,
    Why, SUMMARY_CRAFTS, SUMMARY_ITEMS,
};
use client_core::core::{
    ClientCore, APPLIED2_MOVE, APPLIED_DRANK, APPLIED_RESPAWN, APPLIED_VITALS,
};
use client_core::view::ClientView;
use protocol::{EntityState, Welcome, WireError, MAX_STREAM_MSG_BYTES};
use sim_core::collide;
use sim_core::craft::STATION_NONE;
use sim_core::gather::{cell_key, REACH_M};
use sim_core::input::{InputFrame, BTN_PRIMARY, BTN_SPRINT};
use sim_core::inventory::CONT_SELF;
use sim_core::limits::{ARROW_STEP_MM, HOTBAR_SLOTS, INV_SLOTS, MAX_ITEM_DEFS, TICK_HZ};
use sim_core::melee;
use sim_core::movement::{Body, POS_XZ_Q, POS_Y_Q, WADE_GROUND_MAX};
use sim_core::ranged::{ARROW_EYE_MM, MM_PER_M};
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
pub const FLEE_TICKS: u32 = 6 * TICK_HZ;
const SIGHT_WIDTH: i32 = 2 * SIGHT_CELLS + 1;

// Survivor v0 defaults (DECISIONS.md §open "Jev survivor v0"), proposed.
/// A gather or water search that finds nothing in this long fails.
pub const SEARCH_GOAL_SECS: u32 = 20;
/// An explore goal walks this long, then asks again.
pub const EXPLORE_GOAL_SECS: u32 = 20;
/// A wait goal stands still this long.
pub const WAIT_GOAL_SECS: u32 = 5;
/// An action's answer must arrive within this long (after any craft time).
pub const VERDICT_SECS: u32 = 3;
/// A wandering walk changes heading this often.
pub const WANDER_TURN_SECS: u32 = 10;
/// Less progress than this in a second is a stalled walk.
pub const WANDER_STALL_M: f32 = 0.5;
/// Eat or drink until the meter reaches this percentage.
pub const METER_TARGET_PCT: u32 = 80;
/// Stop drinking sea water at or below this share of health.
pub const DRINK_MIN_HP_PCT: u32 = 20;
/// Targets abandoned before a gather goal fails as stuck.
pub const MAX_ABANDONS: u32 = 3;
/// Distances probed for open water on eight bearings, once a second.
pub const WATER_PROBE_M: [f32; 4] = [8.0, 16.0, 24.0, 32.0];
/// How long a resource seen in view is remembered once it leaves it.
pub const RECALL_SECS: u32 = 60;

/// Tool ladders, best first, by catalog name — the player's knowledge of
/// which tool fells a tree and which breaks rock. Never indices; yields,
/// wear and stack sizes stay content's.
pub const TREE_TOOLS: [&str; 3] = ["Metal Hatchet", "Stone Hatchet", "Rock"];
pub const NODE_TOOLS: [&str; 3] = ["Metal Pickaxe", "Stone Pickaxe", "Rock"];

const _: () = assert!(MAX_ITEM_DEFS <= 64, "FoodBook and yield sets are u64 masks");

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
    Crafting,
    Equipping,
    Eating,
    Drinking,
    SeekingWater,
    Resting,
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
            Phase::Crafting => "Crafting",
            Phase::Equipping => "Moving a tool to the belt",
            Phase::Eating => "Eating",
            Phase::Drinking => "Drinking",
            Phase::SeekingWater => "Walking to water",
            Phase::Resting => "Waiting",
        }
    }
}

/// Which resource a gather goal wants.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Wood = 0,
    Stone = 1,
    Ore = 2,
    Forage = 3,
}

impl Kind {
    fn of(occupant: Occupant) -> Option<Kind> {
        match occupant {
            Occupant::Tree => Some(Kind::Wood),
            Occupant::StoneNode => Some(Kind::Stone),
            Occupant::MetalNode | Occupant::SulfurNode => Some(Kind::Ore),
            Occupant::Bush => Some(Kind::Forage),
            _ => None,
        }
    }

    fn of_goal(goal: Goal) -> Option<Kind> {
        match goal {
            Goal::GatherWood => Some(Kind::Wood),
            Goal::GatherStone => Some(Kind::Stone),
            Goal::GatherOre => Some(Kind::Ore),
            Goal::Forage => Some(Kind::Forage),
            _ => None,
        }
    }
}

/// What the eat verb has taught this body, per item index: the sim's own
/// verdicts, the way a player learns by pressing eat. No food table.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FoodBook {
    pub not_food: u64,
    pub feeds: u64,
    pub waters: u64,
    pub tried: u64,
}

impl FoodBook {
    fn bit(item: u16) -> u64 {
        if (item as usize) < 64 {
            1 << item
        } else {
            0
        }
    }

    /// Worth trying to eat: known to feed, or never tried and not refused.
    pub fn may_feed(&self, item: u16) -> bool {
        let b = Self::bit(item);
        b != 0 && (self.feeds & b != 0 || (self.tried & b == 0 && self.not_food & b == 0))
    }

    pub fn waters_known(&self, item: u16) -> bool {
        self.waters & Self::bit(item) != 0
    }
}

/// What perception last published. Built by the per-frame scans from the
/// client's own view cone, line of sight and received deltas.
#[derive(Clone, Copy, Debug, Default)]
pub struct Senses {
    pub trees: Sighting,
    pub stone_nodes: Sighting,
    pub ore_nodes: Sighting,
    pub bushes: Sighting,
    pub players: Sighting,
    pub animals: Sighting,
    pub water: Sighting,
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
    pub food: FoodBook,
    /// Items each resource kind has been seen to pay (gather receipts),
    /// as masks over item indices: what "room for it" means.
    pub yields: [u64; 4],
}

#[derive(Clone, Copy, Debug, Default)]
struct Sweep {
    cells: [Sighting; 4],
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SurvivorStats {
    pub phase: Phase,
    pub deaths: u64,
    pub respawns: u64,
    pub respawn_asks: u64,
    pub retreats: u64,
    pub goals_done: u64,
    pub goals_failed: u64,
    pub goals_interrupted: u64,
    pub targets_seen: u64,
    pub targets_completed: u64,
    pub targets_abandoned: u64,
    pub gather_awards: u64,
    pub refusals: u64,
    pub crafted: u64,
    pub eaten: u64,
    pub drinks: u64,
    pub equips: u64,
    pub actions: u64,
    pub unencodable: u64,
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pending {
    Respawn,
    Craft { item: u16 },
    Equip,
    Consume { item: u16, food: u16, water: u16 },
    Drink,
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
    Sent { ticks: u32 },
    Equip,
    Equipped,
}

#[derive(Clone, Copy, Debug)]
struct Active {
    goal: Goal,
    started: u32,
    asked: u32,
    gained: u32,
    abandons: u32,
    craft: CraftStep,
    fled: bool,
}

pub struct Survivor {
    pub mind: Mind,
    pub stats: SurvivorStats,
    pub history: History,
    /// Units received per item index, across every life.
    pub gathered: [u32; MAX_ITEM_DEFS],
    core: Option<Box<ClientCore>>,
    haven: Option<Haven>,
    senses: Senses,
    memory: Memory,
    sweep: Sweep,
    bodies: [Sighting; 2],
    body_threat: Option<(f32, f32, f32)>,
    threat: Option<(f32, f32)>,
    water_at: Option<u32>,
    water_yaw: Option<u16>,
    scan: i32,
    ent_scan: usize,
    /// The nearest resource of each kind last seen, and when.
    recall: [Option<(Target, u32)>; 4],
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
    retreat: Option<(u32, u16)>,
    last_hurt: Option<(u32, u16)>,
    interrupt: Option<Why>,
    heading: Option<u16>,
    wander_mark: Option<(u32, i32, i32)>,
    wander_turn: u32,
    outbox: Option<([u8; MAX_STREAM_MSG_BYTES], usize)>,
    awaiting: Option<(Pending, u32)>,
    verdict: Option<Verdict>,
    learning: Option<(u16, u16, u16)>,
}

impl Survivor {
    pub fn new(mind: Mind) -> Self {
        Self {
            mind,
            stats: SurvivorStats::default(),
            history: History::default(),
            gathered: [0; MAX_ITEM_DEFS],
            core: None,
            haven: None,
            senses: Senses::default(),
            memory: Memory::default(),
            sweep: Sweep::default(),
            bodies: [Sighting::default(); 2],
            body_threat: None,
            threat: None,
            water_at: None,
            water_yaw: None,
            scan: 0,
            ent_scan: 0,
            recall: [None; 4],
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
            retreat: None,
            last_hurt: None,
            interrupt: None,
            heading: None,
            wander_mark: None,
            wander_turn: 0,
            outbox: None,
            awaiting: None,
            verdict: None,
            learning: None,
        }
    }

    /// The client state this controller reads, once the welcome arrived.
    pub fn core(&self) -> Option<&ClientCore> {
        self.core.as_deref()
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

    pub fn memory(&self) -> &Memory {
        &self.memory
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
        self.core.as_deref().map_or(0, |core| amount(core, name.as_bytes()))
    }

    /// The summary this body would send now; pure, for display and tests.
    pub fn summary(&self, view: &ClientView, player: u32) -> Option<Summary> {
        let core = self.core.as_deref()?;
        Some(observe(core, view, player, &self.senses, &self.memory))
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

    fn halt(&mut self) {
        self.target = None;
        self.recovery = None;
        self.retreat = None;
    }

    fn frame_with(
        &mut self,
        core: &mut ClientCore,
        view: &ClientView,
        player: u32,
        seq: u16,
        now: Instant,
    ) -> InputFrame {
        let mut frame = InputFrame {
            seq,
            pitch: 128,
            ..InputFrame::default()
        };
        if view.newest_applied.is_some() && view.newest_applied != self.seen_tick {
            self.seen_tick = view.newest_applied;
            self.seen_at = now;
        }
        let Some(body) = view.get(player).copied() else {
            self.stats.phase = Phase::Waiting;
            return frame;
        };
        frame.yaw = body.yaw;
        let tick = view.newest_applied.unwrap_or_default();
        if now.saturating_duration_since(self.seen_at) >= self.mind.config().timeout {
            self.halt();
            self.stats.phase = Phase::Stale;
            return frame;
        }
        if !catalog_ready(core) {
            self.stats.phase = Phase::Waiting;
            return frame;
        }
        self.mind.expire(now);
        if body.dead || core.dead {
            // The death screen's one answer. A beach, never a bag: this
            // body places none, and a bag is where it just lost a fight.
            // Asked on the event lane's own fact: a snapshot can still show
            // the corpse for a frame after the wake has landed.
            self.end_goal(tick, Outcome::Interrupted(Why::Died));
            self.halt();
            let due = core.dead
                && match self.awaiting {
                    Some((Pending::Respawn, since)) => {
                        tick.wrapping_sub(since) >= VERDICT_SECS * TICK_HZ
                    }
                    _ => true,
                };
            if due && self.queue(|buf| protocol::encode_action_respawn(false, buf)) {
                self.awaiting = Some((Pending::Respawn, tick));
                self.stats.respawn_asks += 1;
            }
            self.stats.phase = Phase::Dead;
            return frame;
        }
        if body.sleeping {
            self.stats.phase = Phase::Sleeping;
            return frame;
        }
        if body.wounded || core.wounded {
            // Down: crawl away from the last blow's bearing while it is
            // fresh, otherwise lie still. Nothing else is possible here.
            self.end_goal(tick, Outcome::Interrupted(Why::Wounded));
            self.halt();
            if let Some((at, away)) = self.last_hurt {
                if tick.wrapping_sub(at) < FLEE_TICKS {
                    frame.yaw = away;
                    frame.move_z = 127;
                }
            }
            self.stats.phase = Phase::Wounded;
            return frame;
        }
        if let Some(why) = self.interrupt.take() {
            if self.goal.is_some_and(|a| a.goal != Goal::Flee) {
                self.end_goal(tick, Outcome::Interrupted(why));
            }
        }
        // Answers are taken only by a body that can act on them; one that
        // arrives while it is down waits in the ring and is judged fresh
        // or late when it is read.
        if let Some(choice) = self.mind.poll(now) {
            self.adopt(choice, tick);
        }
        self.perceive(core, view, &body, player, tick);
        if let Some(retreat) = self.retreat_frame(core, view, &body, tick, frame) {
            return retreat;
        }
        let Some(active) = self.goal else {
            // Look before choosing: the first sweep of the sight window
            // after appearing takes under three seconds.
            if !self.sensed {
                self.stats.phase = Phase::Waiting;
                return frame;
            }
            self.request(core, view, player, tick, now);
            self.stats.phase = if self.mind.mode(now) == crate::mind::Mode::Paused {
                Phase::Paused
            } else {
                Phase::Deciding
            };
            return frame;
        };
        if tick.wrapping_sub(active.asked) >= heartbeat_ticks(&self.mind) && !self.mind.pending() {
            self.memory.trigger = Trigger::Heartbeat;
            if self.request(core, view, player, tick, now) {
                if let Some(a) = self.goal.as_mut() {
                    a.asked = tick;
                }
            }
        }
        self.run_goal(core, view, &body, tick, frame)
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
        let summary = observe(core, view, player, &self.senses, &self.memory);
        if summary.options_len == 0 || !self.mind.ask(now, &summary) {
            return false;
        }
        self.memory.hits = 0;
        if let Some(a) = self.goal.as_mut() {
            a.asked = tick;
        }
        true
    }

    fn adopt(&mut self, choice: Choice, tick: u32) {
        if let Some(active) = self.goal {
            if active.goal == choice.goal {
                return;
            }
            self.end_goal(tick, Outcome::Interrupted(Why::Replaced));
        }
        self.goal = Some(Active {
            goal: choice.goal,
            started: tick,
            asked: tick,
            gained: 0,
            abandons: 0,
            craft: CraftStep::Start,
            fled: false,
        });
        self.target = None;
        self.recovery = None;
        self.wander_mark = None;
        self.wander_turn = tick;
        self.verdict = None;
    }

    fn end_goal(&mut self, tick: u32, outcome: Outcome) {
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
        if !matches!(self.awaiting, Some((Pending::Respawn, _))) {
            self.awaiting = None;
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

    fn run_goal(
        &mut self,
        core: &mut ClientCore,
        view: &ClientView,
        body: &EntityState,
        tick: u32,
        frame: InputFrame,
    ) -> InputFrame {
        let Some(active) = self.goal else {
            return frame;
        };
        let elapsed = tick.wrapping_sub(active.started);
        match active.goal {
            Goal::Explore => {
                if elapsed >= EXPLORE_GOAL_SECS * TICK_HZ {
                    self.end_goal(tick, Outcome::Done);
                    return frame;
                }
                self.stats.phase = Phase::Exploring;
                self.wander(core, body, tick, frame)
            }
            Goal::Wait => {
                if elapsed >= WAIT_GOAL_SECS * TICK_HZ {
                    self.end_goal(tick, Outcome::Done);
                }
                self.stats.phase = Phase::Resting;
                frame
            }
            Goal::Flee => {
                if active.fled || !self.start_flee(body, tick) {
                    self.end_goal(tick, Outcome::Done);
                }
                frame
            }
            Goal::Craft(name) => self.craft(core, name, tick, frame),
            Goal::Eat => self.eat(core, tick, frame),
            Goal::Drink => self.drink(core, body, tick, frame),
            goal => match Kind::of_goal(goal) {
                Some(kind) => self.gather(core, view, body, tick, kind, frame),
                None => frame,
            },
        }
    }

    fn start_flee(&mut self, body: &EntityState, tick: u32) -> bool {
        let Some((x, z)) = self.threat else {
            return false;
        };
        let bx = body.qx as f32 * POS_XZ_Q;
        let bz = body.qz as f32 * POS_XZ_Q;
        let away = yaw_toward(bx - x, bz - z);
        self.retreat = Some((tick, away));
        self.stats.retreats += 1;
        if let Some(a) = self.goal.as_mut() {
            a.fled = true;
        }
        true
    }

    fn retreat_frame(
        &mut self,
        core: &mut ClientCore,
        view: &ClientView,
        body: &EntityState,
        tick: u32,
        mut frame: InputFrame,
    ) -> Option<InputFrame> {
        let (mut start, mut yaw) = self.retreat?;
        // A received damage bearing is the human's hit indicator, not an
        // opponent position; every fresh hit renews the bounded retreat.
        if visible_pursuer(core, self.haven.as_ref()?, body, view) {
            start = tick;
            self.retreat = Some((start, yaw));
        }
        if tick.wrapping_sub(start) < FLEE_TICKS {
            if into_deeper_water(core, body, yaw) {
                yaw = yaw.wrapping_add(1 << 14);
                self.retreat = Some((start, yaw));
            }
            self.stats.phase = Phase::Fleeing;
            // Keep the pursuer in view while retreating through ordinary
            // backwards movement. No unseen-body distance check is used.
            frame.yaw = yaw.wrapping_add(1 << 15);
            frame.move_z = -127;
            frame.buttons = BTN_SPRINT;
            return Some(frame);
        }
        self.retreat = None;
        // We were looking backwards at the pursuer. Resume travelling
        // away, rather than turning the retreat into a return trip.
        self.heading = Some(yaw);
        None
    }

    /// Walk somewhere new: straight, turning on a stall, on deeper water
    /// ahead, and on a fixed cadence, so a search covers ground.
    fn wander(
        &mut self,
        core: &mut ClientCore,
        body: &EntityState,
        tick: u32,
        mut frame: InputFrame,
    ) -> InputFrame {
        let mut heading = *self.heading.get_or_insert(body.yaw);
        match self.wander_mark {
            Some((at, qx, qz)) if tick.wrapping_sub(at) >= TICK_HZ => {
                let moved = ((body.qx - qx) as f32 * POS_XZ_Q).hypot((body.qz - qz) as f32 * POS_XZ_Q);
                if moved < WANDER_STALL_M {
                    heading = heading.wrapping_add(1 << 14);
                }
                self.wander_mark = Some((tick, body.qx, body.qz));
            }
            None => self.wander_mark = Some((tick, body.qx, body.qz)),
            _ => {}
        }
        if tick.wrapping_sub(self.wander_turn) >= WANDER_TURN_SECS * TICK_HZ {
            self.wander_turn = tick;
            // A deterministic eighth or quarter turn either way.
            let turn: [u16; 4] = [0u16.wrapping_sub(1 << 14), 0u16.wrapping_sub(1 << 13), 1 << 13, 1 << 14];
            heading = heading.wrapping_add(turn[(tick.wrapping_mul(2_654_435_761) >> 30) as usize]);
        }
        if into_deeper_water(core, body, heading) {
            heading = heading.wrapping_add(1 << 14);
        }
        self.heading = Some(heading);
        frame.yaw = heading;
        frame.move_z = 127;
        frame
    }

    fn gather(
        &mut self,
        core: &mut ClientCore,
        view: &ClientView,
        body: &EntityState,
        tick: u32,
        kind: Kind,
        mut frame: InputFrame,
    ) -> InputFrame {
        let Some(active) = self.goal else {
            return frame;
        };
        let Some(sel) = best_tool(core, kind) else {
            self.end_goal(tick, Outcome::Failed(Why::NoTool));
            return frame;
        };
        frame.sel = sel;
        if !room_for(core, self.memory.yields[kind as usize]) {
            self.end_goal(tick, Outcome::Failed(Why::PackFull));
            return frame;
        }
        if let Some((start, yaw)) = self.recovery {
            if tick.wrapping_sub(start) < RECOVER_TICKS {
                self.stats.phase = Phase::Recovering;
                frame.yaw = yaw;
                frame.move_z = 127;
                return frame;
            }
            self.recovery = None;
        }
        if let Some(target) = self.target {
            if core.harvested.contains(target.key()) {
                self.target = None;
                if self.target_gain > 0 {
                    self.stats.targets_completed += 1;
                    self.end_goal(tick, Outcome::Done);
                    return frame;
                }
            }
        }
        let Some(target) = self.target else {
            if tick.wrapping_sub(active.started) >= SEARCH_GOAL_SECS * TICK_HZ {
                self.end_goal(tick, Outcome::Failed(Why::NotFound));
                return frame;
            }
            // Head back toward the nearest one remembered: facing it puts
            // it in the cone, where the scan acquires it like any other.
            if let Some((seen, at)) = self.recall[kind as usize] {
                let fresh = tick.wrapping_sub(at) < RECALL_SECS * TICK_HZ
                    && !core.harvested.contains(seen.key())
                    && Some(seen.key()) != self.skipped;
                let (yaw, _, distance) = aim(body, &seen.slot);
                if fresh && distance > REACH_M {
                    self.stats.phase = Phase::Approaching;
                    self.heading = Some(yaw);
                    frame.yaw = yaw;
                    frame.move_z = 127;
                    if into_deeper_water(core, body, yaw) {
                        frame.move_z = 0;
                    }
                    return frame;
                }
                // Arrived and still not acquired, or gone: forget it.
                self.recall[kind as usize] = None;
            }
            self.stats.phase = Phase::Exploring;
            let walk = self.wander(core, body, tick, frame);
            return InputFrame { sel, ..walk };
        };
        let (yaw, pitch, distance) = aim(body, &target.slot);
        frame.yaw = yaw;
        frame.pitch = pitch;
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
                self.end_goal(tick, Outcome::Failed(Why::Stuck));
            }
            return frame;
        }
        let ray = melee::ray(&as_body(*body), yaw, pitch, REACH_M * MM_PER_M);
        let (seed, mut island) = core.island();
        let reached = melee::node_cast(seed, &mut island, &ray)
            .is_some_and(|hit| hit.cx == target.cx && hit.cz == target.cz);
        if reached {
            // Holding primary is the human harvesting verb. Keep a bystander
            // out of the swing: this skill never deliberately attacks a body.
            self.stats.phase = Phase::Harvesting;
            let bystander = view.entities.iter().any(|(id, other)| {
                *id != body.id
                    && !other.dead
                    && ((other.qx - body.qx) as f32 * POS_XZ_Q)
                        .hypot((other.qz - body.qz) as f32 * POS_XZ_Q)
                        <= 2.0 * REACH_M
            });
            let haven = self.haven.expect("connected haven");
            if !bystander && visible(core, &haven, body, target) {
                frame.buttons = BTN_PRIMARY;
            }
        } else {
            self.stats.phase = Phase::Approaching;
            frame.move_z = 127;
        }
        frame
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

    fn craft(&mut self, core: &mut ClientCore, name: Name, tick: u32, frame: InputFrame) -> InputFrame {
        let Some(active) = self.goal else {
            return frame;
        };
        self.stats.phase = Phase::Crafting;
        match active.craft {
            CraftStep::Start => {
                let Some((recipe, item, ticks)) = resolve_recipe(core, name) else {
                    self.end_goal(tick, Outcome::Failed(Why::NoRecipe));
                    return frame;
                };
                if !inputs_ok(core, recipe) {
                    self.end_goal(tick, Outcome::Failed(Why::MissingInputs));
                    return frame;
                }
                if self.queue(|buf| protocol::encode_action_craft(recipe, 1, buf)) {
                    self.awaiting = Some((Pending::Craft { item }, tick));
                    self.verdict = None;
                    self.set_craft(CraftStep::Sent { ticks });
                }
            }
            CraftStep::Sent { ticks } => {
                match self.take_verdict(tick, ticks + VERDICT_SECS * TICK_HZ) {
                    Err(()) => self.end_goal(tick, Outcome::Failed(Why::NoAnswer)),
                    Ok(Some(Verdict::Ok)) => {
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
                // A better tool belongs on the belt, where a swing uses it.
                let tool = TREE_TOOLS.contains(&name.as_str()) || NODE_TOOLS.contains(&name.as_str());
                match (tool, equip_move(core, name)) {
                    (true, Some((from, to, count))) => {
                        if self.queue(|buf| {
                            protocol::encode_action_move(0, CONT_SELF, from, CONT_SELF, to, count, buf)
                        }) {
                            self.stats.phase = Phase::Equipping;
                            self.awaiting = Some((Pending::Equip, tick));
                            self.verdict = None;
                            self.set_craft(CraftStep::Equipped);
                        }
                    }
                    _ => self.end_goal(tick, Outcome::Done),
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
        frame
    }

    fn set_craft(&mut self, step: CraftStep) {
        if let Some(a) = self.goal.as_mut() {
            a.craft = step;
        }
    }

    fn eat(&mut self, core: &mut ClientCore, tick: u32, frame: InputFrame) -> InputFrame {
        self.stats.phase = Phase::Eating;
        match self.take_verdict(tick, VERDICT_SECS * TICK_HZ) {
            Err(()) => {
                self.end_goal(tick, Outcome::Failed(Why::NoAnswer));
                return frame;
            }
            Ok(Some(Verdict::Ok)) => {
                if let Some(a) = self.goal.as_mut() {
                    a.gained += 1;
                }
            }
            Ok(Some(Verdict::Full)) => {
                self.end_goal(tick, Outcome::Done);
                return frame;
            }
            Ok(Some(_)) => {}
            Ok(None) if self.awaiting.is_some() => return frame,
            Ok(None) => {}
        }
        if core.max_food == 0 || pct(core.food, core.max_food) >= METER_TARGET_PCT {
            self.end_goal(tick, Outcome::Done);
            return frame;
        }
        let Some(slot) = food_slot(core, &self.memory.food, false) else {
            let gained = self.goal.map_or(0, |a| a.gained);
            let outcome = if gained > 0 {
                Outcome::Done
            } else {
                Outcome::Failed(Why::NoFood)
            };
            self.end_goal(tick, outcome);
            return frame;
        };
        self.consume(core, slot, tick);
        frame
    }

    fn consume(&mut self, core: &ClientCore, slot: usize, tick: u32) {
        let item = core.inv[slot].item;
        if self.queue(|buf| protocol::encode_action_consume(slot as u8, buf)) {
            self.awaiting = Some((
                Pending::Consume {
                    item,
                    food: core.food,
                    water: core.water,
                },
                tick,
            ));
            self.verdict = None;
            self.memory.food.tried |= FoodBook::bit(item);
        }
    }

    fn drink(
        &mut self,
        core: &mut ClientCore,
        body: &EntityState,
        tick: u32,
        mut frame: InputFrame,
    ) -> InputFrame {
        let Some(active) = self.goal else {
            return frame;
        };
        self.stats.phase = Phase::Drinking;
        match self.take_verdict(tick, VERDICT_SECS * TICK_HZ) {
            Err(()) => {
                self.end_goal(tick, Outcome::Failed(Why::NoAnswer));
                return frame;
            }
            Ok(Some(Verdict::Ok)) => {
                if let Some(a) = self.goal.as_mut() {
                    a.gained += 1;
                }
            }
            Ok(Some(Verdict::Full)) => {
                self.end_goal(tick, Outcome::Done);
                return frame;
            }
            Ok(Some(_)) => {}
            Ok(None) if self.awaiting.is_some() => return frame,
            Ok(None) => {}
        }
        if core.max_water == 0 || pct(core.water, core.max_water) >= METER_TARGET_PCT {
            self.end_goal(tick, Outcome::Done);
            return frame;
        }
        // Food the eat verb has seen restore water comes first: it is free.
        if let Some(slot) = food_slot(core, &self.memory.food, true) {
            self.consume(core, slot, tick);
            return frame;
        }
        if core.hp_max > 0 && pct(core.hp, core.hp_max) <= DRINK_MIN_HP_PCT {
            let outcome = if active.gained > 0 {
                Outcome::Done
            } else {
                Outcome::Failed(Why::TooHurt)
            };
            self.end_goal(tick, outcome);
            return frame;
        }
        let x = body.qx as f32 * POS_XZ_Q;
        let z = body.qz as f32 * POS_XZ_Q;
        let (seed, _) = core.island();
        if water_in_reach(seed, x, z) {
            if self.queue(protocol::encode_action_drink) {
                self.awaiting = Some((Pending::Drink, tick));
                self.verdict = None;
            }
            return frame;
        }
        let Some(yaw) = self.water_yaw else {
            self.end_goal(tick, Outcome::Failed(Why::NoWater));
            return frame;
        };
        if tick.wrapping_sub(active.started) >= SEARCH_GOAL_SECS * TICK_HZ {
            self.end_goal(tick, Outcome::Failed(Why::NoWater));
            return frame;
        }
        self.stats.phase = Phase::SeekingWater;
        frame.yaw = yaw;
        frame.move_z = 127;
        frame
    }

    /// One cell of the sight window, one body and — once a second — the
    /// water probe. Publishes the census when a sweep completes.
    fn perceive(
        &mut self,
        core: &mut ClientCore,
        view: &ClientView,
        body: &EntityState,
        player: u32,
        tick: u32,
    ) {
        let Some(haven) = self.haven else {
            return;
        };
        let dx = self.scan % SIGHT_WIDTH - SIGHT_CELLS;
        let dz = self.scan / SIGHT_WIDTH - SIGHT_CELLS;
        self.scan = (self.scan + 1) % (SIGHT_WIDTH * SIGHT_WIDTH);
        let cx = (body.qx as f32 * POS_XZ_Q / CELL_SIZE).floor() as i32 + dx;
        let cz = (body.qz as f32 * POS_XZ_Q / CELL_SIZE).floor() as i32 + dz;
        if (0..terrain::CELLS_PER_SIDE).contains(&cx) && (0..terrain::CELLS_PER_SIDE).contains(&cz) {
            let (seed, island) = core.island();
            let slot = island.cache.slot(seed, island.table, island.haven, cx, cz);
            let target = Target {
                cx: cx as u16,
                cz: cz as u16,
                slot,
            };
            if let Some(kind) = Kind::of(slot.occupant) {
                if !core.harvested.contains(target.key())
                    && in_view(body, &slot)
                    && visible(core, &haven, body, target)
                {
                    let (d, b) = relative(body, slot.x, slot.z);
                    self.sweep.cells[kind as usize].add(d, b);
                    let nearer = self.recall[kind as usize].is_none_or(|(old, at)| {
                        tick.wrapping_sub(at) >= RECALL_SECS * TICK_HZ
                            || core.harvested.contains(old.key())
                            || relative(body, old.slot.x, old.slot.z).0 >= d
                    });
                    if nearer && Some(target.key()) != self.skipped {
                        self.recall[kind as usize] = Some((target, tick));
                    }
                    let wanted = self.goal.and_then(|a| Kind::of_goal(a.goal)) == Some(kind);
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
        }
        if self.scan == 0 {
            // In view now, or seen within the recall window and still
            // standing: a player remembers the rock they just turned from.
            let mut cells = self.sweep.cells;
            for (kind, cell) in cells.iter_mut().enumerate() {
                if cell.count > 0 {
                    continue;
                }
                if let Some((t, at)) = self.recall[kind] {
                    if tick.wrapping_sub(at) < RECALL_SECS * TICK_HZ
                        && !core.harvested.contains(t.key())
                    {
                        let (d, b) = relative(body, t.slot.x, t.slot.z);
                        cell.add(d, b);
                    }
                }
            }
            let [trees, stone, ore, bushes] = cells;
            self.senses.trees = trees;
            self.senses.stone_nodes = stone;
            self.senses.ore_nodes = ore;
            self.senses.bushes = bushes;
            self.sweep = Sweep::default();
            self.sensed = true;
        }
        // Bodies, one per frame, with the same cone and cover checks.
        let n = view.entities.len();
        if n > 0 {
            let i = self.ent_scan % n;
            self.ent_scan = self.ent_scan.wrapping_add(1);
            let (id, other) = view.entities[i];
            if id != player && !other.dead && !other.sleeping {
                let x = other.qx as f32 * POS_XZ_Q;
                let z = other.qz as f32 * POS_XZ_Q;
                if in_cone(body, x, z)
                    && clear_line(
                        core,
                        &haven,
                        eye(body),
                        (x, other.qy as f32 * POS_Y_Q + collide::CAPSULE_RADIUS_M, z),
                    )
                {
                    let (d, b) = relative(body, x, z);
                    let animal = sim_core::mob::slot_of_id(id).is_some();
                    self.bodies[usize::from(animal)].add(d, b);
                    if self.body_threat.is_none_or(|t| d < t.0) {
                        self.body_threat = Some((d, x, z));
                    }
                }
            }
            if i + 1 >= n {
                self.senses.players = self.bodies[0];
                self.senses.animals = self.bodies[1];
                self.threat = self.body_threat.map(|(_, x, z)| (x, z));
                self.bodies = [Sighting::default(); 2];
                self.body_threat = None;
            }
        } else {
            self.senses.players = Sighting::default();
            self.senses.animals = Sighting::default();
            self.threat = None;
        }
        if self.water_at.is_none_or(|at| tick.wrapping_sub(at) >= TICK_HZ) {
            self.water_at = Some(tick);
            let (seed, _) = core.island();
            let x = body.qx as f32 * POS_XZ_Q;
            let z = body.qz as f32 * POS_XZ_Q;
            let mut best: Option<(f32, u16)> = None;
            for dir in 0..8u16 {
                let yaw = dir << 13;
                let (fx, fz) = yaw_dir(yaw);
                if let Some(&m) = WATER_PROBE_M
                    .iter()
                    .find(|&&m| terrain::height(seed, x + fx * m, z + fz * m) < terrain::SEA_LEVEL)
                {
                    if best.is_none_or(|b| m < b.0) {
                        best = Some((m, yaw));
                    }
                }
            }
            let mut water = Sighting::default();
            if let Some((m, yaw)) = best {
                let (fx, fz) = yaw_dir(yaw);
                let (_, b) = relative(body, x + fx * m, z + fz * m);
                water.add(m, b);
            }
            self.senses.water = water;
            self.water_yaw = best.map(|b| b.1);
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
                    self.memory.yields[kind as usize] |= FoodBook::bit(item);
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
        while let Some((sector, damage)) = core.pop_hurt() {
            if damage == 0 {
                continue;
            }
            let toward =
                (u32::from(sector) * 65536 / u32::from(sim_core::combat::HURT_SECTORS)) as u16;
            // Compass bearings turn toward -X; wire yaw turns toward +X.
            let away = 0u16.wrapping_sub(toward).wrapping_add(1 << 15);
            self.retreat = Some((tick, away));
            self.last_hurt = Some((tick, away));
            self.recovery = None;
            if let Some(target) = self.target.take() {
                self.skipped = Some(target.key());
                self.stats.targets_abandoned += 1;
            }
            self.stats.retreats += 1;
            self.memory.hits = self.memory.hits.saturating_add(1);
            self.interrupt = Some(Why::Hit);
        }
        while let Some(victim) = core.pop_death() {
            if victim == core.player_id {
                self.stats.deaths += 1;
                self.memory.deaths = self.memory.deaths.saturating_add(1);
            }
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
            self.recall = [None; 4];
            self.sensed = false;
            self.halt();
            self.heading = None;
            self.last_hurt = None;
            self.interrupt = None;
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
            if matches!(self.awaiting, Some((Pending::Craft { .. }, _))) {
                self.verdict = Some(Verdict::Refused);
            }
        }
        while let Some((item, _slot)) = core.pop_consume_toast() {
            self.stats.eaten += 1;
            if let Some((Pending::Consume { food, water, .. }, _)) = self.awaiting {
                self.learning = Some((item, food, water));
                self.verdict = Some(Verdict::Ok);
            }
        }
        while let Some(reason) = core.pop_consume_refusal() {
            self.stats.refusals += 1;
            match self.awaiting {
                Some((Pending::Consume { item, .. }, _)) => {
                    let bit = FoodBook::bit(item);
                    if u32::from(reason) == REFUSE_C_NOT_FOOD {
                        self.memory.food.not_food |= bit;
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
        if flags & APPLIED_DRANK != 0 {
            self.stats.drinks += 1;
            if matches!(self.awaiting, Some((Pending::Drink, _))) {
                self.verdict = Some(Verdict::Ok);
            }
        }
        if flags & APPLIED_VITALS != 0 {
            if let Some((item, food, water)) = self.learning.take() {
                let bit = FoodBook::bit(item);
                if core.food > food {
                    self.memory.food.feeds |= bit;
                }
                if core.water > water {
                    self.memory.food.waters |= bit;
                }
                if core.food <= food && core.water <= water {
                    // Consumed but restored neither meter: a bandage, say.
                    // Not food for this purpose, whatever else it does.
                    self.memory.food.not_food |= bit;
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
        let mut core = Box::new(ClientCore::new(
            welcome.seed,
            welcome.player_id,
            welcome.tick,
        ));
        self.haven = Some(*core.island().1.haven);
        self.core = Some(core);
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
        let (buf, len) = self.outbox.take()?;
        if out.len() < len {
            self.stats.unencodable += 1;
            return None;
        }
        out[..len].copy_from_slice(&buf[..len]);
        self.stats.actions += 1;
        Some(len)
    }
}

fn heartbeat_ticks(mind: &Mind) -> u32 {
    let secs = mind.config().heartbeat.as_secs().min(u64::from(u32::MAX / TICK_HZ));
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
        Kind::Forage => return Some(best_tool(core, Kind::Stone).unwrap_or(0)),
    };
    for name in ladder {
        for (slot, stack) in core.inv[..HOTBAR_SLOTS].iter().enumerate() {
            if stack.count > 0
                && core.catalog.name(stack.item as usize) == name.as_bytes()
                && (core.catalog.rows[stack.item as usize].cond_max == 0 || stack.cond > 0)
            {
                return Some(slot as u8);
            }
        }
    }
    None
}

/// Room for a node's yield: an empty slot, or a stack of something this
/// kind has been seen to pay that is not yet at its ceiling.
fn room_for(core: &ClientCore, yields: u64) -> bool {
    core.inv.iter().any(|s| {
        s.count == 0
            || (yields & FoodBook::bit(s.item) != 0
                && s.count < core.catalog.rows[s.item as usize].stack_max)
    })
}

/// A recipe this player can use for the named output: no station, and any
/// blueprint already known. `(recipe, output item, ticks per unit)`.
fn resolve_recipe(core: &ClientCore, name: Name) -> Option<(u16, u16, u32)> {
    if core.recipes_have < core.recipes.recipe_count {
        return None;
    }
    let known = core.known();
    (0..usize::from(core.recipes.recipe_count).min(core.recipes.recipes.len())).find_map(|r| {
        let def = core.recipes.recipes[r];
        let usable = def.out_count > 0
            && def.station == STATION_NONE
            && (!def.blueprint || (r < 64 && known & (1 << r) != 0))
            && core.catalog.name(def.output as usize) == name.as_bytes();
        usable.then_some((r as u16, def.output, def.ticks))
    })
}

fn inputs_ok(core: &ClientCore, recipe: u16) -> bool {
    let def = core.recipes.recipes[recipe as usize];
    def.inputs[..usize::from(def.n_inputs)]
        .iter()
        .all(|&(item, need)| count_item(core, item) >= u32::from(need))
}

/// The whole-stack move that puts the named tool on the belt, if it is in
/// the pack and not already there: an empty belt slot, else the last belt
/// slot not holding a tool (the two stacks swap).
fn equip_move(core: &ClientCore, name: Name) -> Option<(u8, u8, u16)> {
    let named = |s: &sim_core::gather::ItemStack| {
        s.count > 0 && core.catalog.name(s.item as usize) == name.as_bytes()
    };
    if core.inv[..HOTBAR_SLOTS].iter().any(named) {
        return None;
    }
    let from = (HOTBAR_SLOTS..INV_SLOTS).find(|&i| named(&core.inv[i]))?;
    let is_tool = |s: &sim_core::gather::ItemStack| {
        let n = core.catalog.name(s.item as usize);
        s.count > 0
            && TREE_TOOLS
                .iter()
                .chain(NODE_TOOLS.iter())
                .any(|t| t.as_bytes() == n)
    };
    let to = (0..HOTBAR_SLOTS)
        .find(|&i| core.inv[i].count == 0)
        .or_else(|| (0..HOTBAR_SLOTS).rev().find(|&i| !is_tool(&core.inv[i])))?;
    Some((from as u8, to as u8, core.inv[from].count))
}

/// A slot worth eating: one known to restore water (`thirst`), or one known
/// to feed, else one never tried that the eat verb has not refused.
fn food_slot(core: &ClientCore, book: &FoodBook, thirst: bool) -> Option<usize> {
    let slots = || (0..INV_SLOTS).filter(|&i| core.inv[i].count > 0);
    if thirst {
        return slots().find(|&i| book.waters_known(core.inv[i].item));
    }
    slots()
        .find(|&i| book.feeds & FoodBook::bit(core.inv[i].item) != 0)
        .or_else(|| slots().find(|&i| book.may_feed(core.inv[i].item)))
}

/// The sim's own drink test: five taps on the heightfield at the reach.
fn water_in_reach(seed: u64, x: f32, z: f32) -> bool {
    let r = DRINK_REACH_M;
    [(0.0, 0.0), (r, 0.0), (-r, 0.0), (0.0, r), (0.0, -r)]
        .iter()
        .any(|(dx, dz)| terrain::height(seed, x + dx, z + dz) < terrain::SEA_LEVEL)
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
    let mut free = 0u8;
    for stack in core.inv.iter() {
        if stack.count == 0 {
            free += 1;
            continue;
        }
        let Some(name) = Name::new(core.catalog.name(stack.item as usize)) else {
            continue;
        };
        let n = s.items_len as usize;
        if let Some(entry) = s.items[..n].iter_mut().find(|(e, _)| *e == name) {
            entry.1 = entry.1.saturating_add(u32::from(stack.count));
        } else if n < SUMMARY_ITEMS {
            s.items[n] = (name, u32::from(stack.count));
            s.items_len += 1;
        }
    }
    s.free_slots = free;
    if core.recipes_have >= core.recipes.recipe_count {
        let known = core.known();
        for r in 0..usize::from(core.recipes.recipe_count).min(core.recipes.recipes.len()) {
            let def = core.recipes.recipes[r];
            if def.out_count == 0
                || def.station != STATION_NONE
                || (def.blueprint && (r >= 64 || known & (1 << r) == 0))
                || !inputs_ok(core, r as u16)
            {
                continue;
            }
            let Some(name) = Name::new(core.catalog.name(def.output as usize)) else {
                continue;
            };
            let n = s.craftable_len as usize;
            if n < SUMMARY_CRAFTS && !s.craftable[..n].contains(&name) {
                s.craftable[n] = name;
                s.craftable_len += 1;
            }
        }
    }
    s.trees = senses.trees;
    s.stone_nodes = senses.stone_nodes;
    s.ore_nodes = senses.ore_nodes;
    s.bushes = senses.bushes;
    s.players = senses.players;
    s.animals = senses.animals;
    s.water_near = senses.water;
    s.last = memory.last;
    s.hits = memory.hits;
    s.deaths = memory.deaths;
    s.respawns = memory.respawns;
    s.trigger = memory.trigger;
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
    // Eat and drink are offered only below the level they fill to, and
    // only where they can work: something worth eating in the pack; a food
    // the eat verb has seen restore water, or open water in sight and the
    // health to pay for salt water.
    if core.max_food > 0
        && pct(core.food, core.max_food) < METER_TARGET_PCT
        && food_slot(core, &memory.food, false).is_some()
    {
        s.offer(Goal::Eat);
    }
    let sea = senses.water.count > 0
        && (core.hp_max == 0 || pct(core.hp, core.hp_max) > DRINK_MIN_HP_PCT);
    if core.max_water > 0
        && pct(core.water, core.max_water) < METER_TARGET_PCT
        && (sea || food_slot(core, &memory.food, true).is_some())
    {
        s.offer(Goal::Drink);
    }
    if s.players.count > 0 || s.animals.count > 0 {
        s.offer(Goal::Flee);
    }
    s.offer(Goal::Wait);
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

fn eye(body: &EntityState) -> (f32, f32, f32) {
    (
        body.qx as f32 * POS_XZ_Q,
        body.qy as f32 * POS_Y_Q + ARROW_EYE_MM as f32 / MM_PER_M,
        body.qz as f32 * POS_XZ_Q,
    )
}

/// The wire yaw that faces along `(dx, dz)`, on the 256-step grid.
fn yaw_toward(dx: f32, dz: f32) -> u16 {
    (((dx.atan2(dz) / std::f32::consts::TAU * 256.0).round() as i32).rem_euclid(256) as u16) << 8
}

/// Distance and relative bearing (index into `mind::BEARINGS`, clockwise
/// from ahead) of a point from this body's facing.
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

fn into_deeper_water(core: &mut ClientCore, body: &EntityState, yaw: u16) -> bool {
    let x = body.qx as f32 * POS_XZ_Q;
    let z = body.qz as f32 * POS_XZ_Q;
    let (dx, dz) = yaw_dir(yaw);
    let (seed, island) = core.island();
    let here = terrain::ground(seed, island.haven, x, z);
    let ahead = terrain::ground(seed, island.haven, x + dx * REACH_M, z + dz * REACH_M);
    ahead <= WADE_GROUND_MAX && ahead < here
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

fn aim(body: &EntityState, slot: &Slot) -> (u16, u8, f32) {
    let dx = slot.x - body.qx as f32 * POS_XZ_Q;
    let dz = slot.z - body.qz as f32 * POS_XZ_Q;
    let distance = dx.hypot(dz);
    let eye = body.qy as f32 * POS_Y_Q + ARROW_EYE_MM as f32 / MM_PER_M;
    let top = terrain::occupant_volume(slot.occupant).1 * slot.scale;
    let y = eye.clamp(
        slot.y + melee::MELEE_PROBE_M,
        (slot.y + top - melee::MELEE_PROBE_M).max(slot.y + melee::MELEE_PROBE_M),
    );
    let yaw = yaw_toward(dx, dz);
    let pitch = (((y - eye).atan2(distance) / std::f32::consts::PI + 0.5) * 255.0).round() as u8;
    (yaw, pitch, distance)
}

/// Conservative line of sight to the near face of the target. Uses the
/// same terrain, occupant and built-volume queries as a player's projectile.
fn visible(core: &mut ClientCore, haven: &Haven, body: &EntityState, target: Target) -> bool {
    let (_, pitch, distance) = aim(body, &target.slot);
    if distance > SIGHT_M || distance <= 0.0 {
        return false;
    }
    let eye = body.qy as f32 * POS_Y_Q + ARROW_EYE_MM as f32 / MM_PER_M;
    let radius = terrain::occupant_volume(target.slot.occupant).0 * target.slot.scale;
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
    clear_line(
        core,
        haven,
        (origin.0, eye, origin.1),
        (origin.0 + dx * stop, eye + dy * stop, origin.1 + dz * stop),
    )
}

/// Check just the nearest candidate in the current view cone: at most one
/// extra sight ray per frame. An occluded nearer body can hide a farther one,
/// which is conservative; no body behind cover prolongs the retreat.
fn visible_pursuer(
    core: &mut ClientCore,
    haven: &Haven,
    body: &EntityState,
    view: &ClientView,
) -> bool {
    let mut nearest = None;
    let mut distance = f32::INFINITY;
    for (_, other) in &view.entities {
        if other.id == body.id || other.dead || other.sleeping {
            continue;
        }
        let x = other.qx as f32 * POS_XZ_Q;
        let z = other.qz as f32 * POS_XZ_Q;
        let d = (x - body.qx as f32 * POS_XZ_Q).hypot(z - body.qz as f32 * POS_XZ_Q);
        if d < distance && in_cone(body, x, z) {
            distance = d;
            nearest = Some((x, other.qy as f32 * POS_Y_Q + collide::CAPSULE_RADIUS_M, z));
        }
    }
    nearest.is_some_and(|to| clear_line(core, haven, eye(body), to))
}

fn clear_line(
    core: &mut ClientCore,
    haven: &Haven,
    from: (f32, f32, f32),
    to: (f32, f32, f32),
) -> bool {
    let delta = (to.0 - from.0, to.1 - from.1, to.2 - from.2);
    let length = delta.0.hypot(delta.1).hypot(delta.2);
    if length > SIGHT_M {
        return false;
    }
    let steps = (length * MM_PER_M / ARROW_STEP_MM as f32).ceil() as usize;
    let point = |i: usize| {
        let t = i as f32 / steps.max(1) as f32;
        (
            from.0 + delta.0 * t,
            from.1 + delta.1 * t,
            from.2 + delta.2 * t,
        )
    };
    let (seed, mut island) = core.island();
    for i in 1..=steps {
        let (x, y, z) = point(i);
        if y <= terrain::ground(seed, haven, x, z) || island.blocks_volume(seed, x, z, y, 0.0, 0.0)
        {
            return false;
        }
    }
    let mut prev = (from.0, from.2);
    for i in 1..=steps {
        let (x, y, z) = point(i);
        if collide::shot_blocked(
            seed,
            haven,
            core.pieces.cols(),
            prev.0,
            prev.1,
            x,
            z,
            y,
            0.0,
        ) || collide::deploy_stop(seed, haven, core.pieces.cols(), x, z, y, 0.0).is_some()
        {
            return false;
        }
        prev = (x, z);
    }
    true
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

    /// A welcomed controller with a small catalog: Rock (3), Wood (5) and,
    /// deliberately off the production rows, food at 7 and a hatchet at 9.
    fn fixture() -> (Survivor, ClientView, Target) {
        let mut bot = survivor();
        bot.welcome(&Welcome {
            seed: SEED,
            player_id: 1,
            tick: 0,
            dev: true,
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
                b"Rock",
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
        let n = protocol::event::encode_event_piece_sync(true, &[rec], &mut buf).unwrap();
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
            assert!(seen.count == 0 || [7, 0, 1].contains(&seen.bearing), "{seen:?}");
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
        let frame = bot.frame_at(&view, 1, 3, now);
        assert_eq!(bot.stats.phase, Phase::Recovering);
        assert_ne!(frame.move_z, 0);
        assert_eq!(frame.buttons, 0);
    }

    #[test]
    fn a_nearby_body_interrupts_the_harvest_swing() {
        let (mut bot, mut view, target) = fixture();
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
        goal(&mut bot, Goal::GatherWood, 1);
        bot.target = Some(target);
        assert_eq!(bot.frame_at(&view, 1, 1, now).buttons, BTN_PRIMARY);
        let mut bystander = *view.get(1).unwrap();
        bystander.id = 2;
        bystander.qx += quant_xz(REACH_M);
        view.entities.push((2, bystander));
        assert_eq!(bot.frame_at(&view, 1, 2, now).buttons, 0);
        view.entities.pop();
        assert_eq!(bot.frame_at(&view, 1, 3, now).buttons, BTN_PRIMARY);
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
        assert_eq!((frame.move_z, frame.buttons), (0, 0), "no recent blow: lie still");
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
        let n = protocol::event::encode_event_death(1, 0, 0, 0, 0, &mut buf).unwrap();
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
        assert!(bot.action(&mut out).is_some(), "a lost answer is asked again");
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
            n_inputs: 1,
            inputs: [(5, 20), (0, 0), (0, 0), (0, 0)],
        };
        for slot in [0, 1, 2, 3, 5] {
            core.inv[slot] = ItemStack {
                item: 5,
                count: 1,
                cond: 0,
            };
        }
        core.inv[7] = ItemStack {
            item: 5,
            count: 30,
            cond: 0,
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
            Ok(protocol::ActionMsg::Craft { recipe: 2, count: 1 })
        ));
        let mut buf = [0u8; protocol::event::MAX_EVENT_MSG_BYTES];
        let n = protocol::event::encode_event_craft_done(9, 1, &mut buf).unwrap();
        event(&mut bot, n, &buf);
        let n = protocol::event::encode_event_inv(
            &[InvSlot {
                slot: 8,
                stack: ItemStack {
                    item: 9,
                    count: 1,
                    cond: 100,
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
                assert!((to_slot as usize) < HOTBAR_SLOTS && to_slot != 4, "not onto the rock");
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
        goal(&mut bot, Goal::Craft(Name::new(b"Metal Hatchet").unwrap()), 5);
        bot.frame_at(&view, 1, 5, now);
        assert!(bot.action(&mut out).is_none());
        assert_eq!(bot.memory.last.unwrap().outcome, Outcome::Failed(Why::NoRecipe));
    }

    #[test]
    fn eating_learns_food_from_the_verdicts_alone() {
        let (mut bot, view, _) = fixture();
        let now = Instant::now();
        let core = bot.core.as_mut().unwrap();
        core.food = 100;
        core.inv[0] = ItemStack {
            item: 6,
            count: 1,
            cond: 0,
        };
        core.inv[1] = ItemStack {
            item: 7,
            count: 3,
            cond: 0,
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
        bot.frame_at(&view, 1, 2, now);
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
        assert!(book.not_food & 1 << 6 != 0 && book.feeds & 1 << 7 != 0);
        assert!(!book.may_feed(6) && book.may_feed(7));
        assert!(!book.waters_known(7));
    }

    #[test]
    fn drinking_walks_to_seen_water_and_uses_the_drink_verb_there() {
        let (mut bot, mut view, _) = fixture();
        let now = Instant::now();
        // Find a beach cell: dry here, open water within the drink reach.
        let haven = *bot.haven.as_ref().unwrap();
        let centre = terrain::ISLAND_SIZE * 0.5;
        let mut shore = None;
        'search: for (dx, dz) in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
            for step in 0..600 {
                let (x, z) = (centre + dx * step as f32 * 2.0, centre + dz * step as f32 * 2.0);
                if terrain::ground(SEED, &haven, x, z) > 0.2 && water_in_reach(SEED, x, z) {
                    shore = Some((x, z));
                    break 'search;
                }
            }
        }
        let (x, z) = shore.expect("the island has a shore");
        let body = &mut view.entities[0].1;
        body.qx = quant_xz(x);
        body.qz = quant_xz(z);
        body.qy = quant_y(terrain::ground(SEED, &haven, x, z));
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
        assert_eq!((report.goal, report.outcome, report.gained), (Goal::Drink, Outcome::Done, 1));
        assert!(bot.senses.water.count > 0, "the probe saw the sea");
    }

    #[test]
    fn damage_interrupts_work_and_escapes_away_from_every_announced_bearing() {
        let (mut bot, mut view, target) = fixture();
        let now = Instant::now();
        let mut buf = [0u8; protocol::event::MAX_EVENT_MSG_BYTES];
        for sector in 0..sim_core::combat::HURT_SECTORS {
            view.newest_applied = Some(1 + u32::from(sector));
            goal(&mut bot, Goal::GatherWood, 1 + u32::from(sector));
            bot.target = Some(target);
            let n = protocol::event::encode_event_hurt(sector, 15, &mut buf).unwrap();
            event(&mut bot, n, &buf);
            let frame = bot.frame_at(&view, 1, 2, now);
            assert_eq!(bot.stats.phase, Phase::Fleeing);
            assert_eq!(frame.buttons, BTN_SPRINT, "a retreat must release primary");
            assert!(frame.move_z < 0 && bot.target.is_none());
            assert!(bot.goal.is_none(), "a hit interrupts the goal");
            let (fx, fz) = yaw_dir(frame.yaw);
            assert_eq!(
                sim_core::combat::bearing_sector((fx * 1000000.0) as i64, (fz * 1000000.0) as i64),
                sector
            );
        }
        let (start, _) = bot.retreat.unwrap();
        view.newest_applied = Some(start + FLEE_TICKS);
        let frame = bot.frame_at(&view, 1, 4, now);
        assert_ne!(bot.stats.phase, Phase::Fleeing);
        assert_eq!(frame.buttons, 0);
        assert_eq!(
            bot.memory.last.unwrap().outcome,
            Outcome::Interrupted(Why::Hit)
        );
    }

    #[test]
    fn a_wounded_body_crawls_away_from_a_fresh_blow() {
        let (mut bot, mut view, _) = fixture();
        let now = Instant::now();
        let mut buf = [0u8; protocol::event::MAX_EVENT_MSG_BYTES];
        let n = protocol::event::encode_event_hurt(4, 30, &mut buf).unwrap();
        event(&mut bot, n, &buf);
        view.entities[0].1.wounded = true;
        let frame = bot.frame_at(&view, 1, 2, now);
        assert_eq!(bot.stats.phase, Phase::Wounded);
        assert_eq!(frame.move_z, 127, "crawl");
        assert_eq!(frame.buttons, 0, "no swing, no sprint");
        let (_, away) = bot.last_hurt.unwrap();
        assert_eq!(frame.yaw, away);
    }

    #[test]
    fn receipts_count_as_gathered_and_the_pack_limits_what_is_worth_gathering() {
        let (mut bot, view, _) = fixture();
        goal(&mut bot, Goal::GatherWood, 1);
        let now = Instant::now();
        bot.frame_at(&view, 1, 1, now);
        let mut buf = [0u8; protocol::event::MAX_EVENT_MSG_BYTES];
        let n = protocol::event::encode_event_gather(5, 25, &mut buf).unwrap();
        event(&mut bot, n, &buf);
        assert_eq!(bot.gathered_of("Wood"), 25);
        assert_eq!(bot.stats.gather_awards, 1);
        assert_ne!(bot.memory.yields[Kind::Wood as usize], 0);
        let core = bot.core.as_mut().unwrap();
        core.inv.fill(ItemStack {
            item: 6,
            count: 100,
            cond: 0,
        });
        core.inv[4] = ItemStack {
            item: 3,
            count: 1,
            cond: 100,
        };
        let summary = bot.summary(&view, 1).unwrap();
        assert!(
            !summary.offers(Goal::GatherWood),
            "a full pack takes the offer away"
        );
        bot.frame_at(&view, 1, 2, now);
        assert_eq!(bot.memory.last.unwrap().outcome, Outcome::Failed(Why::PackFull));
        // A stack of the yield with room is still room.
        let core = bot.core.as_mut().unwrap();
        core.inv[7] = ItemStack {
            item: 5,
            count: 10,
            cond: 0,
        };
        assert!(room_for(core, bot.memory.yields[Kind::Wood as usize]));
        assert!(bot.summary(&view, 1).unwrap().offers(Goal::GatherWood));
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
}
