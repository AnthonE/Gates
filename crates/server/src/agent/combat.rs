//! The fight reflex: what runs over whatever goal is in hand when another
//! body turns on this one, or when an opening is worth taking. It never
//! waits on the mind.
//!
//! A controller with a handful of modes:
//! - **Idle**: nothing to answer.
//! - **Guard**: a blow landed and the eyes have not found who struck it:
//!   face the blow's bearing and walk back from it until they do, or for
//!   a while.
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
//! (`super::aim`); a bullet at the pose on my screen. Under fire from
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

use super::hands::part_height;
use super::intent::{yaw_toward, Intent, Look};
use super::route::{into_deeper_water, Route, Step};
use super::tracks::{Species, Track, Tracks, CLOSING_MPS};
use super::wiki::{Book, Page};
use super::{aim, cover};
use client_core::core::ClientCore;
use protocol::EntityState;
use sim_core::backpack::LOOT_REACH_M;
use sim_core::collide::{Part, CAPSULE_RADIUS_M};
use sim_core::gather::SWING_INTERVAL_TICKS;
use sim_core::input::{BTN_AIM, BTN_CROUCH, BTN_JUMP, BTN_PRIMARY, BTN_SPRINT};
use sim_core::limits::{MAX_ITEM_DEFS, TICK_HZ};
use sim_core::movement::{POS_XZ_Q, POS_Y_Q};
use sim_core::ranged::{ARROW_EYE_MM, MM_PER_M};
use sim_core::rng::Pcg32;
use sim_core::wound::WOUNDED_HP;
use sim_core::yaw_dir;

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
/// A wolf closing inside this is already attacking.
pub const WOLF_ALARM_M: f32 = 10.0;
/// A shooter further off than this is not charged with a melee weapon.
pub const CHARGE_M: f32 = 12.0;
/// Gaps wider than this are closed by route; nearer, straight in.
pub const ROUTE_M: f32 = 8.0;
/// Stand this far inside my own reach to swing, and this far outside
/// theirs to wait.
pub const HOLD_M: f32 = 0.3;
pub const SAFE_M: f32 = 0.5;
/// The strafe reverses no sooner than this, and at most this much later.
pub const STRAFE_MIN_TICKS: u32 = 18;
pub const STRAFE_SPAN_TICKS: u32 = 24;
/// Their swing leaves time to step in, swing and step out when it is at
/// least this far from being ready again.
pub const STEP_IN_TICKS: u32 = 12;
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
/// A swung weapon closing inside this is backed away from while shooting.
pub const KITE_M: f32 = 8.0;
/// A body aiming something that shoots at me from within this is dodged.
pub const EVADE_M: f32 = 40.0;
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
/// Eye height above the feet, metres (where a shot leaves from).
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Foe {
    pub id: u32,
    pub since: u32,
    /// Last tick it struck, swung or shot at me.
    pub threat: u32,
    /// Told to (a fight or hunt goal): no temperament asked.
    pub commanded: bool,
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
    /// Strafe side (+1 right, −1 left) and when it next reverses.
    strafe: (i8, u32),
    /// When my own next swing is ready, from the last one it felt.
    ready_at: u32,
    /// Trying to close on the foe and not getting nearer: since when, and
    /// the nearest it has been since.
    stuck: Option<(u32, f32)>,
    /// A body found unreachable, and until when it is not picked again.
    shun: Option<(u32, u32)>,
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
    /// Cover the escape is making for, and when it was chosen.
    cover: Option<([f32; 2], u32)>,
    /// A body aiming at me, being sidestepped, until when.
    dodge: Option<(u32, u32)>,
    spoils: Option<Spoils>,
    verb: Option<Verb>,
    rng: Pcg32,
    pub stats: CombatStats,
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
            strafe: (1, 0),
            ready_at: 0,
            stuck: None,
            shun: None,
            last_end: None,
            arm: Arm::Melee,
            draw: None,
            shot_ready: 0,
            fire_gap: 1,
            threat: None,
            cover: None,
            dodge: None,
            spoils: None,
            verb: None,
            rng: Pcg32::new(0, 0),
            stats: CombatStats::default(),
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
        self.mode = Mode::Idle;
        self.retreat = None;
        self.foe = None;
        self.hurt = None;
        self.stuck = None;
        self.shun = None;
        self.draw = None;
        self.threat = None;
        self.cover = None;
        self.dodge = None;
        self.spoils = None;
        self.verb = None;
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
    }

    /// A body a fight was let go with because it could not be reached:
    /// not worth picking again yet.
    pub fn shuns(&self, id: u32, tick: u32) -> bool {
        self.shun
            .is_some_and(|(who, until)| who == id && tick.wrapping_sub(until) >= 1 << 31)
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
                commanded,
            });
            self.stuck = None;
        }
        if let Some(f) = self.foe.as_mut() {
            f.commanded |= commanded;
        }
        self.mode = Mode::Engage;
        self.retreat = None;
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
        self.draw = None;
        self.threat = None;
        self.cover = None;
        self.spoils = None;
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
        match self.mode {
            Mode::Idle | Mode::Downed | Mode::Heal => {
                self.mode = Mode::Idle;
                if let Some(id) = self.attacker(tracks, kit.book, me, tick) {
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
                if let Some(id) = self.attacker(tracks, kit.book, me, tick) {
                    self.spoils = None;
                    return self.answer(core, body, tracks, route, kit, id, tick);
                }
                self.loot(core, body, tracks, route, kit, tick)
            }
            Mode::Guard => {
                if let Some(id) = self.attacker(tracks, kit.book, me, tick) {
                    return self.answer(core, body, tracks, route, kit, id, tick);
                }
                self.back_away(core, body, tracks, route, tick, false)
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
        self.draw = None;
    }

    /// A body aiming something that shoots at me, not yet shooting: step
    /// side to side while it does, eyes on it, and let the goal wait.
    fn sidestep(&mut self, tracks: &Tracks, book: &Book, me: [f32; 3], tick: u32) -> Assess {
        let mut aimer: Option<(f32, &Track)> = None;
        for t in tracks.seen() {
            if !t.visible || !t.active() || t.species != Species::Player || !t.aiming_at_me {
                continue;
            }
            let fires = t.held.is_some_and(|h| book.page(h).fires());
            let d = flat(me, t.pos);
            if fires && d <= EVADE_M && aimer.is_none_or(|(b, _)| d < b) {
                aimer = Some((d, t));
            }
        }
        if let Some((_, t)) = aimer {
            if self.dodge.is_none_or(|(id, _)| id != t.id) {
                self.stats.evades += 1;
            }
            self.dodge = Some((t.id, tick.wrapping_add(TICK_HZ)));
        }
        let Some((id, until)) = self.dodge else {
            return Assess::Calm;
        };
        let Some(t) = tracks.get(id).filter(|t| t.active()) else {
            self.dodge = None;
            return Assess::Calm;
        };
        if tick.wrapping_sub(until) < 1 << 31 {
            self.dodge = None;
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
        let odds = odds(&t, kit, crowd(tracks, id, me, tick));
        let d = flat(me, t.pos);
        // Shot at from past a charge with nothing to shoot back: cover.
        let out_shot = odds.ranged && d > CHARGE_M && kit.ranged.is_none();
        if self.temperament == Temperament::Passive || odds.losing || out_shot {
            self.flee_from(&t, odds.ranged, me, tick);
            return self.back_away(core, body, tracks, route, tick, true);
        }
        self.engage(id, tick, false);
        self.fight(core, body, tracks, route, kit, tick)
    }

    /// The body that is attacking me now, if the eyes have it: one that
    /// swung at me from where the swing could land, or shot at me, just
    /// now; one standing where a blow came from; or a wolf coming in.
    fn attacker(&self, tracks: &Tracks, book: &Book, me: [f32; 3], tick: u32) -> Option<u32> {
        let recent = |at: Option<u32>| at.is_some_and(|a| tick.wrapping_sub(a) < ALARM_TICKS);
        let blow = self
            .hurt
            .filter(|(at, _)| tick.wrapping_sub(*at) < ALARM_TICKS);
        let mut best: Option<(f32, u32)> = None;
        for t in tracks.seen() {
            if !t.visible || !t.active() {
                continue;
            }
            let d = flat(me, t.pos);
            let (reach, ranged) = their_reach(t, book);
            let swung = recent(t.last_swing) && swing_reaches(t, me, reach);
            let shot = recent(t.last_shot) && t.aiming_at_me;
            let struck = blow.is_some_and(|(_, toward)| {
                let bearing = yaw_toward(t.pos[0] - me[0], t.pos[2] - me[2]);
                let off = (bearing.wrapping_sub(toward) as i16).unsigned_abs();
                (d <= reach.max(1.0) + ATTACKER_SLACK_M || (ranged && t.aiming_at_me))
                    && (off <= HURT_CONE || d < 1.0)
            });
            let charging = t.species == Species::Wolf && d < WOLF_ALARM_M && closing(t, me, d);
            if (swung || shot || struck || charging) && best.is_none_or(|(b, _)| d < b) {
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
        let mut best: Option<(f32, u32)> = None;
        for t in tracks.seen() {
            if !t.visible || !t.active() || t.species != Species::Player || self.shuns(t.id, tick) {
                continue;
            }
            let d = flat(me, t.pos);
            if d > START_M || best.is_some_and(|(b, _)| d >= b) {
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
        let Some(t) = tracks.get(foe.id).copied() else {
            return self.over(None, End::Parted);
        };
        // A body lies down dead; an animal drops where my markers say it
        // had no more to give (its carcass leaves the snapshot).
        let beast_hp = match t.species {
            Species::Player => u32::MAX,
            Species::Pig => u32::from(kit.book.pig().hp),
            Species::Wolf => u32::from(kit.book.wolf().hp),
        };
        if t.dead || t.dealt >= beast_hp.max(1) {
            self.spoils = Some(Spoils {
                at: [t.pos[0], t.pos[2]],
                since: tick,
                bag: true,
                sent: None,
                skip: None,
                picks: 0,
            });
            self.mode = Mode::Loot;
            self.foe = None;
            self.stuck = None;
            self.draw = None;
            return self.loot(core, body, tracks, route, kit, tick);
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
        if (recent(t.last_swing) && swing_reaches(&t, me, reach))
            || (recent(t.last_shot) && t.aiming_at_me)
            || self
                .hurt
                .is_some_and(|(at, _)| tick.wrapping_sub(at) < ALARM_TICKS && at > foe.threat)
        {
            foe.threat = tick;
        }
        self.foe = Some(foe);
        let odds = odds(&t, kit, crowd(tracks, foe.id, me, tick));
        let unarmed = kit.melee.is_none() && kit.ranged.is_none();
        if unarmed || odds.losing || (odds.ranged && d > CHARGE_M && kit.ranged.is_none()) {
            self.flee_from(&t, odds.ranged, me, tick);
            return self.back_away(core, body, tracks, route, tick, true);
        }
        if !foe.commanded {
            let calm = tick.wrapping_sub(foe.threat) >= CALM_TICKS;
            if (calm && !odds.edge) || d > CHASE_M {
                return self.over(Some(away), End::Parted);
            }
        }
        let intent = match self.arm_for(kit, d) {
            Arm::Melee => {
                self.draw = None;
                self.melee(core, body, route, &t, &odds, kit, tick)
            }
            Arm::Ranged => {
                self.stuck = None;
                self.shoot(core, body, route, &t, &odds, kit, tick)
            }
        };
        if self
            .stuck
            .is_some_and(|(since, _)| tick.wrapping_sub(since) >= UNREACHABLE_TICKS)
        {
            // No way to them: standing and staring is no fight. A goal
            // that asked for it hears the foe was not found.
            self.shun = Some((foe.id, tick.wrapping_add(SHUN_TICKS)));
            return self.over(Some(away), End::Parted);
        }
        Assess::Fight(intent)
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
    fn arm_for(&mut self, kit: &Kit, d: f32) -> Arm {
        self.arm = match (kit.melee.is_some(), kit.ranged.is_some()) {
            (_, false) => Arm::Melee,
            (false, true) => Arm::Ranged,
            (true, true) if kit.loaded == Some(0) && d < KITE_M => Arm::Melee,
            (true, true) if d <= MELEE_IN_M => Arm::Melee,
            (true, true) if d >= RANGED_OUT_M => Arm::Ranged,
            (true, true) => self.arm,
        };
        self.arm
    }

    /// A shot: bow, crossbow or gun. Get the body in sight and in range
    /// (a prey animal crouched, at a walk, to where it will not hear);
    /// then hold the draw, lead the body by where it will be when the
    /// round arrives (a bullet goes where my screen shows it), and loose
    /// only once the hands have settled on it. Against a player keep
    /// stepping side to side, and away from a swung weapon closing in.
    #[allow(clippy::too_many_arguments)]
    fn shoot(
        &mut self,
        core: &mut ClientCore,
        body: &EntityState,
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
        let me = pos(body);
        let eye = [me[0], me[1] + EYE_M, me[2]];
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
        let at = [t.pos[0], t.pos[1] + aim_height(t, kit.book), t.pos[2]];
        let (look, solved) = if r.hitscan {
            let look = match t.species {
                Species::Player => Look::Body {
                    id: t.id,
                    part: Part::Chest.bits(),
                },
                _ => Look::Point(at),
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
        let want = if stalking { HUNT_SHOT_M } else { reach * 0.75 };
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
                Step::Walk { yaw, jump, .. } => (Some(yaw), jump),
                Step::Wait => (Some(toward), false),
                Step::Arrived | Step::Blocked => (None, false),
            };
            let mut buttons = if stalking { BTN_CROUCH } else { BTN_SPRINT };
            if jump {
                buttons |= BTN_JUMP;
            }
            return Intent {
                travel,
                buttons,
                ..base
            };
        }
        let mut buttons = 0;
        if stalking {
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
        let steady = if r.hitscan {
            kit.on_target
        } else {
            kit.settled
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
        if due {
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
        route: &mut Route,
        t: &Track,
        odds: &Odds,
        kit: &Kit,
        tick: u32,
    ) -> Intent {
        let me = pos(body);
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
                let k = if d > 0.0 { r * 0.5 / d } else { 0.0 };
                Look::Point([t.pos[0] - dx * k, t.pos[1] + h * 0.9, t.pos[2] - dz * k])
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
        let cooling = t.last_swing.is_some_and(|at| {
            tick.wrapping_sub(at) + STEP_IN_TICKS < odds.their_cadence.max(STEP_IN_TICKS)
        });
        let ready = tick.wrapping_sub(self.ready_at) < 1 << 31;
        // Distances here are to the pose on my screen, the one the server
        // judges my swing against: a body walking in is nearer than that,
        // and one backing off is judged nearer to it. So a ready swing
        // steps in rather than waiting at the edge.
        let want = if !odds.armed || odds.ranged || t.wounded {
            // Nothing to fear up close: in, and swing.
            odds.my_reach - HOLD_M
        } else if odds.my_reach - HOLD_M > safe {
            // Out-ranged by me, a spear against a rock: hold between the
            // two reaches and let them walk onto the point.
            odds.my_reach - HOLD_M
        } else if ready {
            // Mine is ready: in, and swing, their swing spent or not.
            odds.my_reach - HOLD_M
        } else if !cooling {
            // Mine is spent and theirs is not: out of their reach.
            safe
        } else {
            // Both spent: stay where the next swing is short.
            d
        };
        let radial: f32 = if d > want + 0.25 {
            1.0
        } else if d < want - 0.25 {
            -1.0
        } else {
            0.0
        };
        if radial > 0.0 && d > odds.my_reach {
            self.closing_from(d, tick);
        } else {
            self.stuck = None;
        }
        if tick.wrapping_sub(self.strafe.1) < 1 << 31 {
            let span = self.rng.next_bounded(STRAFE_SPAN_TICKS + 1);
            self.strafe = (-self.strafe.0, tick + STRAFE_MIN_TICKS + span);
        }
        // Less sideways the closer: circling a body at arm's length turns
        // the bearing to it faster than a person's eyes follow.
        let side = f32::from(self.strafe.0) * STRAFE_SHARE * ((d - 0.5) / 2.5).clamp(0.2, 1.0);
        let (ux, uz) = yaw_dir(toward);
        // World right of the bearing (`yaw_dir` of a quarter turn on).
        let (rx, rz) = (uz, -ux);
        let (vx, vz) = (radial * ux + side * rx, radial * uz + side * rz);
        let mut buttons = 0;
        // Run in to close, and run out: backing off at a walk, a body that
        // walks after me never leaves its reach.
        if radial < 0.0 || (radial > 0.0 && d > odds.my_reach) {
            buttons |= BTN_SPRINT;
        }
        let in_reach = d <= odds.my_reach - 0.05;
        if in_reach && ready && kit.on_target && kit.melee.is_some() {
            buttons |= BTN_PRIMARY;
        }
        Intent {
            travel: Some(yaw_toward(vx, vz)),
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
            let rethink = self
                .cover
                .is_none_or(|(_, since)| tick.wrapping_sub(since) >= COVER_RETHINK_TICKS);
            // Still in its sight where the cover was: somewhere else.
            if rethink && (self.cover.is_none() || seen.is_some()) {
                let had = self.cover.map(|(c, _)| c);
                self.cover = cover::find(core, [x, z], at, COVER_M, had).map(|c| (c, tick));
                if had.is_none() && self.cover.is_some() {
                    self.stats.covers += 1;
                }
            }
            if let Some((spot, _)) = self.cover {
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

/// Where on a body a shot is aimed, above its feet: a player's chest, an
/// animal's middle.
fn aim_height(t: &Track, book: &Book) -> f32 {
    match t.species {
        Species::Player => part_height(Part::Chest.bits()),
        Species::Pig => f32::from(book.pig().height_cm) * 0.005,
        Species::Wolf => f32::from(book.wolf().height_cm) * 0.005,
    }
}

/// Nobody who could hurt me near enough and in sight to stop a bandage:
/// no player or wolf in sight within [`HEAL_SAFE_M`]. One further off, or
/// out of sight behind cover, gives the seconds a med takes.
pub fn safe(tracks: &Tracks, me: [f32; 3], tick: u32) -> bool {
    !tracks
        .recent(tick, TICK_HZ)
        .any(|t| t.visible && t.species != Species::Pig && flat(me, t.pos) <= HEAL_SAFE_M)
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
    let (dx, dz) = (me[0] - t.pos[0], me[2] - t.pos[2]);
    t.vel[0] * dx + t.vel[2] * dz >= CLOSING_MPS * d
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
    let head = |page: &Page| u32::from(page.melee.headshot_mult.max(1));
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
                    1
                },
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
                        u32::from(page.melee.damage) * head(page),
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
            || my_time.saturating_mul(2) <= their_time);
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
        let rock = wire("Rock");
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
}
