//! The fight reflex: what runs over whatever goal is in hand when another
//! body turns on this one, or when an opening is worth taking. It never
//! waits on the mind.
//!
//! A controller with a handful of modes:
//! - **Idle**: nothing to answer.
//! - **Guard**: a blow landed and the eyes have not found who struck it:
//!   face the blow's bearing and walk back from it until they do, or for
//!   a while.
//! - **Engage**: a melee fight with one body (see [`Combat::melee`]).
//! - **Escape**: losing, or told to run: back away along a route, weaving,
//!   eyes on the danger, until nobody follows.
//! - **Heal**, **Loot**: after a fight; not driven here yet (the heal
//!   goal and a later lane do those).
//! - **Downed**: the body is down; the orchestrator crawls.
//!
//! Whether to fight is a sum a player does on the wiki's numbers: my hits
//! to kill them against theirs to kill me, what they are holding, how hurt
//! they are (my own hit markers), and who else is near. The temperament
//! says what to do with the answer: a passive body always runs, a
//! defensive one only answers attacks, an opportunist also starts a fight
//! it has a clear edge in, and a kill-on-sight one starts any it is not
//! losing. Everything it knows of other bodies comes from [`Tracks`].

use super::intent::{yaw_toward, Intent, Look};
use super::route::{into_deeper_water, Route, Step};
use super::tracks::{Species, Track, Tracks, CLOSING_MPS};
use super::wiki::{Book, Page};
use client_core::core::ClientCore;
use protocol::EntityState;
use sim_core::collide::{Part, CAPSULE_RADIUS_M};
use sim_core::gather::SWING_INTERVAL_TICKS;
use sim_core::input::{BTN_JUMP, BTN_PRIMARY, BTN_SPRINT};
use sim_core::limits::TICK_HZ;
use sim_core::movement::{POS_XZ_Q, POS_Y_Q};
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
    pub hp: u16,
    pub hp_max: u16,
    /// The hands were on their target last frame.
    pub on_target: bool,
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
    last_end: Option<End>,
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
            last_end: None,
            rng: Pcg32::new(0, 0),
            stats: CombatStats {
                guards: 0,
                engages: 0,
                escapes: 0,
                won: 0,
                escaped: 0,
                parted: 0,
            },
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

    /// In a fight: a frame now goes to the reflex, not the goal.
    pub fn engaged(&self) -> bool {
        matches!(self.mode, Mode::Guard | Mode::Engage | Mode::Escape)
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
        match self.mode {
            Mode::Idle | Mode::Downed | Mode::Heal | Mode::Loot => {
                self.mode = Mode::Idle;
                if let Some(id) = self.attacker(tracks, kit.book, me, tick) {
                    return self.answer(core, body, tracks, route, kit, id, tick);
                }
                if let Some(id) = self.opening(tracks, kit, me, tick) {
                    self.engage(id, tick, false);
                    return self.fight(core, body, tracks, route, kit, tick);
                }
                Assess::Calm
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
        let out_shot = odds.ranged && d > CHARGE_M;
        if self.temperament == Temperament::Passive || odds.losing || out_shot {
            self.run(
                Mode::Escape,
                tick,
                yaw_toward(me[0] - t.pos[0], me[2] - t.pos[2]),
            );
            return self.back_away(core, body, tracks, route, tick, true);
        }
        self.engage(id, tick, false);
        self.fight(core, body, tracks, route, kit, tick)
    }

    /// The body that is attacking me now, if the eyes have it: one that
    /// swung or shot at me just now, one standing where a blow came from,
    /// or a wolf coming in.
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
            let swung = recent(t.last_swing) && t.aiming_at_me && d <= reach + ATTACKER_SLACK_M;
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
            if !t.visible || !t.active() || t.species != Species::Player {
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
        if t.dead {
            return self.over(None, End::Won);
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
        if (t.aiming_at_me && (recent(t.last_swing) || recent(t.last_shot)))
            || self
                .hurt
                .is_some_and(|(at, _)| tick.wrapping_sub(at) < ALARM_TICKS && at > foe.threat)
        {
            foe.threat = tick;
        }
        self.foe = Some(foe);
        let odds = odds(&t, kit, crowd(tracks, foe.id, me, tick));
        if kit.melee.is_none() || odds.losing || (odds.ranged && d > CHARGE_M) {
            self.run(Mode::Escape, tick, away);
            return self.back_away(core, body, tracks, route, tick, true);
        }
        if !foe.commanded {
            let calm = tick.wrapping_sub(foe.threat) >= CALM_TICKS;
            if (calm && !odds.edge) || d > CHASE_M {
                return self.over(Some(away), End::Parted);
            }
        }
        Assess::Fight(self.melee(core, body, route, &t, &odds, kit, tick))
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
                let beast = if t.species == Species::Wolf {
                    kit.book.wolf()
                } else {
                    kit.book.pig()
                };
                let h = f32::from(beast.height_cm) * 0.01;
                Look::Point([t.pos[0], t.pos[1] + h * 0.5, t.pos[2]])
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
                Step::Walk { yaw, jump, .. } => (Some(yaw), jump),
                Step::Wait => (Some(toward), false),
                Step::Arrived | Step::Blocked => (None, false),
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

fn flat(a: [f32; 3], b: [f32; 3]) -> f32 {
    let (dx, dz) = (a[0] - b[0], a[2] - b[2]);
    (dx * dx + dz * dz).sqrt()
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
    let (hp0, my_radius) = match t.species {
        Species::Player => (
            if kit.hp_max > 0 {
                u32::from(kit.hp_max)
            } else {
                100
            },
            CAPSULE_RADIUS_M,
        ),
        Species::Wolf => (
            u32::from(book.wolf().hp),
            f32::from(book.wolf().radius_cm) * 0.01,
        ),
        Species::Pig => (
            u32::from(book.pig().hp),
            f32::from(book.pig().radius_cm) * 0.01,
        ),
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
    let my_damage = u32::from(mine_page.melee.damage)
        * if t.species == Species::Player && !t.wounded {
            head(mine_page)
        } else {
            1
        };
    let my_reach = f32::from(mine_page.melee.reach_cm) * 0.01 + my_radius;
    let my_cadence = u32::from(mine_page.melee.cadence_ticks).max(1);
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
            hp,
            hp_max: 100,
            on_target: true,
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

    #[test]
    fn temperaments_parse_by_name() {
        for t in Temperament::ALL {
            assert_eq!(Temperament::parse(t.name()), Some(t));
        }
        assert_eq!(Temperament::default(), Temperament::Opportunist);
        assert_eq!(Temperament::parse("berserk"), None);
    }
}
