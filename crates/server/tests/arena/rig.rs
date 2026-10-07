//! The arena's rig: the agent (slot 0) in lockstep against `ShardCore`
//! with the shipped content, the inline scripted mind and a synthetic
//! clock, so a run is a function of its inputs; and whoever stands across
//! from it on slot 1.
//!
//! Across from it sits one of:
//! - a **seat**: a test-only proxy that reads the world directly and is the
//!   yardstick, not an agent ([`Seat`]): no reaction time, perfect aim at
//!   the agent's pose as its own client draws it (the pose the server
//!   rewinds a blow to), and a perfect lead on the agent's true motion for
//!   an arrow (judged live);
//! - a second agent ([`Peer`]), or a scripted prop (a chopper beside it, an
//!   archer standing still, a body held a fixed way off).
//!
//! The agent's link can be given a round trip ([`Arena::latency`]): its
//! snapshots and events arrive late and its inputs land late, through
//! delay queues outside the counted frame, so lag compensation sees the
//! same acks it would over a wire.

use super::common::{clearing, content, scene, shard_with, stack, SEED};
use protocol::{decode_action, decode_input, encode_input, ActionMsg, InputDatagram};
use server::agent::combat::{Mode, Temperament};
use server::agent::hands::Preset;
use server::agent::intent::{pitch_toward, yaw_toward};
use server::core::{Lane, ShardCore};
use server::explorer::{Survivor, SurvivorOpts};
use server::mind::{Mind, MindConfig, Scripted};
use server::stats::ShardStats;
use server::view::ClientView;
use sim_core::collide::{CAPSULE_HEIGHT_M, CAPSULE_RADIUS_M, HEAD_BAND_M};
use sim_core::input::{InputFrame, BTN_AIM, BTN_PRIMARY, BTN_SPRINT};
use sim_core::limits::{HOTBAR_SLOTS, TICK_HZ};
use sim_core::movement::{Body, POS_XZ_Q, POS_Y_Q};
use sim_core::ranged::{ARROW_EYE_MM, MM_PER_M};
use sim_core::rng::Pcg32;
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

thread_local! { static ALLOCS: Cell<Option<usize>> = const { Cell::new(None) }; }
struct CountAlloc;
fn note() {
    let _ = ALLOCS.try_with(|count| {
        if let Some(n) = count.get() {
            count.set(Some(n + 1));
        }
    });
}
unsafe impl GlobalAlloc for CountAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note();
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        note();
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        note();
        unsafe { System.realloc(ptr, layout, size) }
    }
}
#[global_allocator]
static ALLOCATOR: CountAlloc = CountAlloc;

pub const ID: u32 = 256;
pub const RUSHER: u32 = 257;

/// Eye height over the feet, metres: where swings and shots leave from.
const EYE_M: f32 = ARROW_EYE_MM as f32 / MM_PER_M;
/// The middle of the head band over the feet.
const HEAD_M: f32 = CAPSULE_HEIGHT_M - HEAD_BAND_M * 0.5;
const CHEST_M: f32 = 1.2;
/// Ticks of history kept of the agent's pose.
const TRAIL: usize = 32;
/// How far behind the present a seat sees the agent, ticks: the playout a
/// client draws other bodies at, which the server rewinds a blow by when
/// the input acks the newest tick (`server::stats::favour_for`).
pub const SEAT_FAVOUR: u32 = sim_core::limits::INTERP_DELAY_TICKS as u32;
/// A placed seat waits at most this long for the agent to see it.
const SIGHT_WAIT_TICKS: u32 = 5 * TICK_HZ;
const NOTICE_TICKS: u32 = TICK_HZ;
/// A seat swings this far inside the reach the server judges, centre to
/// centre: a hair short of the edge, as a player's own sense of it is.
const REACH_MARGIN_M: f32 = 0.1;
/// An agent's shot loosed from at least this far off counts as a long one
/// ([`Record::far_shots`]).
pub const FAR_SHOT_M: f32 = 25.0;

/// Who sits across from the agent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Seat {
    /// Runs straight in and strikes the moment it is in reach.
    Rusher,
    /// Keeps its spacing: at its own reach's edge with a spear, 20 to 30 m
    /// off with a bow, backing off whatever closes.
    Kiter,
    /// Comes in, then circles side to side while it strikes.
    Strafer,
    /// Knows everything: both swing clocks, both reaches to the
    /// centimetre. In at a run with its swing ready, out of the agent's
    /// reach with it spent; with a bow, the head every time.
    Aimbot,
}

impl Seat {
    pub const ALL: [Seat; 4] = [Seat::Rusher, Seat::Kiter, Seat::Strafer, Seat::Aimbot];

    pub fn name(self) -> &'static str {
        match self {
            Seat::Rusher => "rusher",
            Seat::Kiter => "kiter",
            Seat::Strafer => "strafer",
            Seat::Aimbot => "aimbot",
        }
    }
}

/// What a seat holds, read off the world's tables.
#[derive(Clone, Copy, Debug)]
enum Arms {
    None,
    /// Where a swing lands from, centre to centre, metres.
    Melee {
        reach_m: f32,
    },
    /// Arrow speed and drop, the sim's units.
    Bow {
        speed: u16,
        drop: u16,
    },
}

/// A seat in play: its arms and its rhythm.
#[derive(Clone, Copy)]
pub struct Proxy {
    pub seat: Seat,
    arms: Arms,
    rng: Pcg32,
    /// Strafe side and when it next flips.
    side: f32,
    flip_at: u64,
}

/// A second agent on slot 1, as body [`RUSHER`].
pub struct Peer {
    pub bot: Survivor,
    pub view: ClientView,
}

/// A body standing still beside the agent, holding primary, its look
/// passing a step to one side of it.
#[derive(Clone, Copy)]
pub struct Chopper {
    pub yaw: u16,
}

/// The agent's link: ticks its inputs take to land, and its snapshots and
/// events to arrive.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Latency {
    pub up: u32,
    pub down: u32,
}

impl Latency {
    pub const NONE: Latency = Latency { up: 0, down: 0 };
    /// About 100 ms round trip: a tick each way, and the tick the lockstep
    /// itself takes between a snapshot and the input made from it. More
    /// and the rewind's 233 ms cap (`REWIND_MAX_TICKS`, playout included)
    /// starts to bite.
    pub const RTT_100MS: Latency = Latency { up: 1, down: 1 };

    /// The round trip on top of the lockstep's own tick, ms.
    pub fn ms(self) -> u32 {
        if self == Latency::NONE {
            return 0;
        }
        (self.up + self.down + 1) * 1000 / TICK_HZ
    }
}

pub struct Arena {
    pub shard: Box<ShardCore>,
    pub stats: ShardStats,
    pub view: ClientView,
    pub bot: Survivor,
    /// The agent's body id, and the one across from it on slot 1.
    pub me: u32,
    pub foe: u32,
    t0: Instant,
    pub tick: u32,
    pub heap_ops: usize,
    last_yaw: Option<u16>,
    pub max_turn: u16,
    pub engaged_ticks: u32,
    pub seat: Option<Proxy>,
    pub peer: Option<Peer>,
    /// The other body is put back this far off the agent along this world
    /// bearing every tick: however the agent walks, it never gets nearer.
    pub keep_off: Option<(f32, u16)>,
    pub chopper: Option<Chopper>,
    /// The other body stands where it is with a bow up, eyes on the
    /// agent's chest; drawing and loosing when `true`.
    pub archer: Option<bool>,
    /// The other body walks straight at the agent, eyes level, whatever it
    /// holds out but never raised, and stops this far off.
    pub walker: Option<f32>,
    /// The agent's body is put back here every tick: it shoots from where
    /// it stands, however it tries to walk.
    pub pin: Option<Body>,
    pub haven: sim_core::terrain::Haven,
    pub latency: Latency,
    uplink: VecDeque<(u32, InputFrame, (u16, u32))>,
    actions: VecDeque<(u32, ActionMsg)>,
    downlink: VecDeque<(u32, Lane, Vec<u8>)>,
    /// Where the agent stood at the end of each recent tick: what a seat
    /// sees of it, [`SEAT_FAVOUR`] ticks late.
    trail: [[f32; 3]; TRAIL],
    /// What slot 1's client has been sent: a seat acks it as a client does.
    seat_view: ClientView,
    foe_joined: bool,
}

impl Arena {
    pub fn new(temperament: Temperament) -> Self {
        let mind = Mind::inline(Scripted::default(), MindConfig::default()).unwrap();
        Self::with(temperament, mind, false)
    }

    /// With a mind of the test's choosing, and animals if asked for.
    pub fn with(temperament: Temperament, mind: Mind, wildlife: bool) -> Self {
        Self::seated(
            SurvivorOpts {
                temperament,
                ..SurvivorOpts::default()
            },
            mind,
            wildlife,
            ID,
            scene(),
            false,
        )
    }

    /// The agent with these options, as body `me` spawned at `at`; slot 1
    /// is `me + 1`. With `foe_first`, slot 1 joins ahead of the agent (and
    /// is put far off until it is placed): the world steps it first, so
    /// when both swing on one tick the agent's lands second. A bout is
    /// not won on the order of the rows.
    pub fn seated(
        opts: SurvivorOpts,
        mind: Mind,
        wildlife: bool,
        me: u32,
        at: (f32, f32),
        foe_first: bool,
    ) -> Self {
        let content = content();
        let mut bot = Survivor::with(mind, opts);
        use server::botclient::BotDriver;
        bot.welcome(&protocol::Welcome {
            seed: SEED,
            player_id: me,
            tick: 0,
            dev: true,
        });
        let stats = ShardStats::default();
        let shard = if foe_first {
            // Slot 1 joins a tick ahead, far off where the agent will not
            // see it until it is placed.
            let mut core = shard_with(&content, (at.0 + 150.0, at.1), wildlife, &[(1, me + 1)]);
            core.tick_bare(&stats, |_, _, _| true);
            core.world.dev_spawn = Some(at);
            assert!(core.connect(0, me));
            core
        } else {
            shard_with(&content, at, wildlife, &[(0, me)])
        };
        Self {
            shard,
            stats,
            view: ClientView::new(),
            bot,
            me,
            foe: me + 1,
            t0: Instant::now(),
            tick: 0,
            heap_ops: 0,
            last_yaw: None,
            max_turn: 0,
            engaged_ticks: 0,
            seat: None,
            peer: None,
            keep_off: None,
            chopper: None,
            archer: None,
            walker: None,
            pin: None,
            haven: sim_core::terrain::haven(SEED),
            latency: Latency::NONE,
            uplink: VecDeque::new(),
            actions: VecDeque::new(),
            downlink: VecDeque::new(),
            trail: [[0.0; 3]; TRAIL],
            seat_view: ClientView::new(),
            foe_joined: foe_first,
        }
    }

    /// A second agent joins beside the first, on slot 1.
    pub fn with_peer(&mut self, temperament: Temperament) {
        use server::botclient::BotDriver;
        let mut bot = Survivor::with(
            Mind::inline(Scripted::default(), MindConfig::default()).unwrap(),
            SurvivorOpts {
                temperament,
                ..SurvivorOpts::default()
            },
        );
        bot.welcome(&protocol::Welcome {
            seed: SEED,
            player_id: self.foe,
            tick: self.tick,
            dev: true,
        });
        assert!(self.shard.connect(1, self.foe));
        self.peer = Some(Peer {
            bot,
            view: ClientView::new(),
        });
    }

    /// Slot 1 joins, for a seat or a prop (if it has not already).
    pub fn connect_foe(&mut self) {
        if !self.foe_joined {
            assert!(self.shard.connect(1, self.foe));
            self.foe_joined = true;
        }
    }

    pub fn player(&self, id: u32) -> &sim_core::world::Player {
        self.shard
            .world
            .players
            .iter()
            .find(|p| p.active && p.id == id)
            .unwrap()
    }

    pub fn stage(&mut self, id: u32, f: impl Fn(&mut sim_core::world::Player)) {
        let p = self
            .shard
            .world
            .players
            .iter_mut()
            .find(|p| p.active && p.id == id)
            .unwrap();
        f(p);
    }

    /// Put slot 1 in a seat, holding whatever it holds now.
    pub fn sit(&mut self, seat: Seat, seed: u64) {
        let held = sim_core::combat::held_item(self.player(self.foe));
        let cc = &self.shard.world.combat;
        let arms = if let Some(m) = cc.held_melee(held) {
            Arms::Melee {
                reach_m: f32::from(m.reach_cm) * 0.01 + CAPSULE_RADIUS_M - REACH_MARGIN_M,
            }
        } else if let Some(r) = cc.held_ranged(held) {
            let a = cc.ammo_def(r.ammo[0]).unwrap_or_default();
            Arms::Bow {
                speed: a.speed_mmpt,
                drop: a.drop_mmpt2,
            }
        } else {
            Arms::None
        };
        self.seat = Some(Proxy {
            seat,
            arms,
            rng: Pcg32::new(seed ^ 0x5345_4154, u64::from(self.foe)),
            side: 1.0,
            flip_at: 0,
        });
    }

    fn push(&mut self, slot: usize, frame: InputFrame, ack: (u16, u32)) {
        let mut dg = InputDatagram::new(ack.0, ack.1, sim_core::limits::INTERP_DELAY_TICKS);
        dg.push(frame).unwrap();
        let mut bytes = [0u8; sim_core::limits::DATAGRAM_BUDGET_BYTES];
        let len = encode_input(&dg, &mut bytes).unwrap();
        self.shard
            .push_input(slot, &decode_input(&bytes[..len]).unwrap());
    }

    fn pos(&self, id: u32) -> [f32; 3] {
        let b = self.player(id).body;
        [
            b.qx as f32 * POS_XZ_Q,
            b.qy as f32 * POS_Y_Q,
            b.qz as f32 * POS_XZ_Q,
        ]
    }

    /// Centre-to-centre ground distance between the agent and slot 1.
    pub fn gap(&self) -> f32 {
        let (a, b) = (self.pos(self.me), self.pos(self.foe));
        (a[0] - b[0]).hypot(a[2] - b[2])
    }

    /// The seat's frame: read the world, and act on it perfectly.
    /// Where the agent stood at the end of the tick `back` before the
    /// newest snapshot the seat's client holds (stamped with the tick about
    /// to run): with that snapshot acked, the server rewinds the seat's
    /// blow by the playout alone, to exactly this pose.
    fn seen(&self, back: u32) -> [f32; 3] {
        let newest = self
            .seat_view
            .newest_applied
            .unwrap_or(self.shard.world.tick as u32);
        self.trail[newest.wrapping_sub(back) as usize % TRAIL]
    }

    fn seat_frame(&mut self, mut s: Proxy) -> (InputFrame, Proxy) {
        // The seat sees the agent through the same playout the agent sees
        // it through, and the server rewinds its blows to that pose.
        let (me, them) = (self.pos(self.foe), self.seen(SEAT_FAVOUR));
        let (dx, dz) = (them[0] - me[0], them[2] - me[2]);
        let d = dx.hypot(dz);
        let toward = yaw_toward(dx, dz);
        let tick = self.shard.world.tick;
        let mine = self.player(self.foe);
        let agent = self.player(self.me);
        let ready = mine.next_swing <= tick + 1;
        // The agent's own swing and its reach, as the world has them.
        let agent_held = sim_core::combat::held_item(agent);
        let agent_reach = self
            .shard
            .world
            .combat
            .held_melee(agent_held)
            .map_or(0.0, |m| f32::from(m.reach_cm) * 0.01 + CAPSULE_RADIUS_M);
        let agent_shoots = self.shard.world.combat.held_ranged(agent_held).is_some();
        let agent_ready = agent.next_swing <= tick + 2;
        if tick >= s.flip_at {
            s.side = -s.side;
            s.flip_at = tick + 15 + u64::from(s.rng.next_bounded(21));
        }
        let aim = |at_y: f32| pitch_toward(them[1] + at_y - (me[1] + EYE_M), d);
        let mut f = InputFrame {
            seq: self.tick as u16,
            yaw: toward,
            pitch: aim(HEAD_M),
            sel: 0,
            ..Default::default()
        };
        // Forward and right, in the seat's own view (it faces the agent).
        let (mut fwd, mut right, mut run) = (0.0f32, 0.0f32, false);
        match s.arms {
            Arms::None => {
                fwd = if d > 0.9 { 1.0 } else { 0.0 };
                run = d > 3.0;
            }
            Arms::Melee { reach_m } => {
                // The Aimbot's sense of its reach has no margin.
                let reach_m = if s.seat == Seat::Aimbot {
                    reach_m + REACH_MARGIN_M - 0.02
                } else {
                    reach_m
                };
                let in_reach = d <= reach_m;
                match s.seat {
                    Seat::Rusher => {
                        fwd = if d > 0.9 { 1.0 } else { 0.0 };
                        run = d > 3.0;
                    }
                    Seat::Strafer => {
                        if d > reach_m + 1.0 {
                            fwd = 1.0;
                            run = d > 3.0;
                        } else {
                            fwd = if d > reach_m - 0.2 { 1.0 } else { 0.0 };
                            right = s.side;
                        }
                    }
                    Seat::Kiter => {
                        // At its own reach's edge; out of theirs when it can
                        // be, and back from anything closer.
                        let hold = reach_m - 0.1;
                        let floor = (agent_reach + 0.3).min(hold - 0.2);
                        if d > hold {
                            fwd = 1.0;
                            run = d > 3.0;
                        } else if d < floor {
                            fwd = -1.0;
                            run = true;
                        }
                        right = s.side * 0.5;
                    }
                    Seat::Aimbot => {
                        // Knows both clocks and both reaches to the
                        // centimetre. Its swing ready: straight in at a run
                        // (whoever moves in is nearer than the other's
                        // screen shows). Spent: back out of the agent's
                        // reach at a run while the agent's is ready, and
                        // wait at its edge while neither is.
                        let safe = agent_reach + 0.3;
                        if ready || agent_reach == 0.0 {
                            fwd = if d > 0.9 { 1.0 } else { 0.0 };
                            run = true;
                        } else if agent_ready && d < safe {
                            fwd = -1.0;
                            run = true;
                        } else if d > safe + 0.3 {
                            fwd = 1.0;
                        } else if d < safe {
                            fwd = -1.0;
                        }
                    }
                }
                if in_reach && ready {
                    f.buttons |= BTN_PRIMARY;
                }
            }
            Arms::Bow { speed, drop } => {
                let before = self.seen(SEAT_FAVOUR + 1);
                // Perfect lead on the agent's true motion; the Aimbot takes
                // the head, the rest the chest.
                let vel = [
                    (them[0] - before[0]) * TICK_HZ as f32,
                    0.0,
                    (them[2] - before[2]) * TICK_HZ as f32,
                ];
                let part = if s.seat == Seat::Aimbot {
                    HEAD_M
                } else {
                    CHEST_M
                };
                let eye = [me[0], me[1] + EYE_M, me[2]];
                let at = [them[0], them[1] + part, them[2]];
                // An arrow is judged live: from the pose seen, on by the
                // ticks it is behind the present and the tick to loose.
                let lead = (SEAT_FAVOUR + 1) as f32;
                let solved = server::agent::aim::lead(eye, at, vel, lead, speed, drop);
                if let Some((p, _)) = solved {
                    let (px, pz) = (p[0] - eye[0], p[2] - eye[2]);
                    f.yaw = yaw_toward(px, pz);
                    f.pitch = pitch_toward(p[1] - eye[1], px.hypot(pz));
                }
                let (near, far) = match s.seat {
                    Seat::Kiter => (20.0, 30.0),
                    Seat::Rusher => (0.0, 6.0),
                    Seat::Strafer | Seat::Aimbot => (8.0, 30.0),
                };
                let close_in = agent_reach > 0.0 && !agent_shoots && d < 12.0;
                if d > far + 10.0 {
                    fwd = 1.0;
                    run = true;
                } else if s.seat == Seat::Kiter && close_in {
                    // A club closing on a bow: turn and run, and shoot
                    // again from further off.
                    fwd = -1.0;
                    run = true;
                } else {
                    f.buttons |= BTN_AIM;
                    if d > far {
                        fwd = 1.0;
                    } else if d < near {
                        fwd = -1.0;
                    }
                    if s.seat != Seat::Rusher {
                        right = s.side;
                    }
                    if solved.is_some() && ready {
                        f.buttons |= BTN_PRIMARY;
                    }
                }
            }
        }
        if run {
            f.buttons |= BTN_SPRINT;
        }
        let len = fwd.hypot(right).max(1.0);
        f.move_z = (fwd / len * 127.0) as i8;
        f.move_x = (right / len * 127.0) as i8;
        (f, s)
    }

    pub fn step(&mut self) {
        use server::botclient::BotDriver;
        let now = self.t0 + Duration::from_secs_f64(f64::from(self.tick) / f64::from(TICK_HZ));
        // What the link has carried to the agent by now.
        while self
            .downlink
            .front()
            .is_some_and(|(due, ..)| *due <= self.tick)
        {
            let (_, lane, bytes) = self.downlink.pop_front().unwrap();
            match lane {
                Lane::Snapshot => {
                    self.view.apply(&bytes).unwrap();
                }
                Lane::Event => self.bot.event(&bytes).unwrap(),
            }
        }
        let mut act = [0u8; protocol::MAX_STREAM_MSG_BYTES];
        ALLOCS.with(|c| c.set(Some(0)));
        let frame = self
            .bot
            .frame_at(&self.view, self.me, self.tick as u16, now);
        let action = self.bot.action(&mut act);
        self.heap_ops += ALLOCS.with(|c| c.replace(None).unwrap());
        assert!(usize::from(frame.sel) < HOTBAR_SLOTS);
        if let Some(last) = self.last_yaw {
            let turn = (frame.yaw.wrapping_sub(last) as i16).unsigned_abs();
            self.max_turn = self.max_turn.max(turn);
            assert!(
                turn <= self.bot.hands().skill().max_turn(),
                "the view turned {turn} in one frame at tick {}",
                self.tick
            );
        }
        self.last_yaw = Some(frame.yaw);
        if self.bot.combat().mode() == Mode::Engage {
            self.engaged_ticks += 1;
        }
        let ack = self.view.ack_fields();
        let due = self.tick + self.latency.up;
        self.uplink.push_back((due, frame, ack));
        if let Some(len) = action {
            let msg = decode_action(&act[..len]).unwrap();
            if self.latency == Latency::NONE {
                assert!(self.shard.wants_action(0));
            }
            self.actions.push_back((due, msg));
        }
        while self
            .uplink
            .front()
            .is_some_and(|(due, ..)| *due <= self.tick)
        {
            let (_, frame, ack) = self.uplink.pop_front().unwrap();
            self.push(0, frame, ack);
        }
        if self
            .actions
            .front()
            .is_some_and(|(due, _)| *due <= self.tick)
            && self.shard.wants_action(0)
        {
            let (_, msg) = self.actions.pop_front().unwrap();
            self.shard.push_action(0, msg);
        }
        if let Some(s) = self.seat {
            let (frame, s) = self.seat_frame(s);
            self.seat = Some(s);
            let ack = self.seat_view.ack_fields();
            self.push(1, frame, ack);
        }
        if let Some(c) = self.chopper {
            let frame = InputFrame {
                seq: self.tick as u16,
                yaw: c.yaw,
                pitch: 128,
                buttons: BTN_PRIMARY,
                sel: 0,
                ..Default::default()
            };
            self.push(1, frame, (0, 0));
        }
        if let Some(shoot) = self.archer {
            let (me, them) = (self.player(self.foe).body, self.player(self.me).body);
            let dx = (them.qx - me.qx) as f32 * POS_XZ_Q;
            let dz = (them.qz - me.qz) as f32 * POS_XZ_Q;
            let dy = (them.qy - me.qy) as f32 * POS_Y_Q - 0.4;
            let frame = InputFrame {
                seq: self.tick as u16,
                yaw: yaw_toward(dx, dz),
                pitch: pitch_toward(dy, dx.hypot(dz)),
                buttons: if shoot { BTN_AIM | BTN_PRIMARY } else { 0 },
                sel: 0,
                ..Default::default()
            };
            self.push(1, frame, (0, 0));
        }
        if let Some(stop) = self.walker {
            let (me, them) = (self.pos(self.foe), self.pos(self.me));
            let (dx, dz) = (them[0] - me[0], them[2] - me[2]);
            let frame = InputFrame {
                seq: self.tick as u16,
                yaw: yaw_toward(dx, dz),
                pitch: server::agent::intent::LEVEL_PITCH,
                move_z: if dx.hypot(dz) > stop { 127 } else { 0 },
                sel: 0,
                ..Default::default()
            };
            self.push(1, frame, (0, 0));
        }
        if let Some(mut peer) = self.peer.take() {
            ALLOCS.with(|c| c.set(Some(0)));
            let frame = peer
                .bot
                .frame_at(&peer.view, self.foe, self.tick as u16, now);
            let action = peer.bot.action(&mut act);
            self.heap_ops += ALLOCS.with(|c| c.replace(None).unwrap());
            let ack = peer.view.ack_fields();
            self.push(1, frame, ack);
            if let Some(len) = action {
                let msg = decode_action(&act[..len]).unwrap();
                assert!(self.shard.wants_action(1));
                self.shard.push_action(1, msg);
            }
            self.peer = Some(peer);
        }
        let due = self.tick + 1 + self.latency.down;
        let (downlink, peer, seat_view) = (&mut self.downlink, &mut self.peer, &mut self.seat_view);
        self.shard.tick_bare(&self.stats, |lane, slot, bytes| {
            match (slot, peer.as_mut()) {
                (0, _) => downlink.push_back((due, lane, bytes.to_vec())),
                (1, None) if lane == Lane::Snapshot => {
                    seat_view.apply(bytes).unwrap();
                }
                (1, Some(p)) => match lane {
                    Lane::Snapshot => {
                        p.view.apply(bytes).unwrap();
                    }
                    Lane::Event => p.bot.event(bytes).unwrap(),
                },
                _ => {}
            }
            true
        });
        if let Some(p) = self
            .shard
            .world
            .players
            .iter()
            .find(|p| p.active && p.id == self.me)
        {
            // By the tick that just ran, as the rewind ring keeps it.
            let done = self.shard.world.tick.wrapping_sub(1);
            self.trail[done as usize % TRAIL] = [
                p.body.qx as f32 * POS_XZ_Q,
                p.body.qy as f32 * POS_Y_Q,
                p.body.qz as f32 * POS_XZ_Q,
            ];
        }
        if let Some(body) = self.pin {
            let me = self.me;
            self.stage(me, |p| p.body = body);
        }
        if let Some((off, bearing)) = self.keep_off {
            let at = self.player(self.me).body;
            let (fx, fz) = sim_core::yaw_dir(bearing);
            let (x, z) = (
                at.qx as f32 * POS_XZ_Q + fx * off,
                at.qz as f32 * POS_XZ_Q + fz * off,
            );
            let body = Body::at(SEED, &self.haven, x, z);
            let foe = self.foe;
            self.stage(foe, |p| p.body = body);
        }
        self.tick += 1;
    }

    /// The agent just loosed: how far its view was from the one a perfect
    /// lead on the foe's true motion would have taken (debugging).
    fn trace_shot(&self, foe_was: [f32; 3]) {
        let (me, it) = (self.pos(self.me), self.pos(self.foe));
        let f = self.player(self.me).frame;
        let vel = [
            (it[0] - foe_was[0]) * TICK_HZ as f32,
            0.0,
            (it[2] - foe_was[2]) * TICK_HZ as f32,
        ];
        let cc = &self.shard.world.combat;
        let held = sim_core::combat::held_item(self.player(self.me));
        let Some(r) = cc.held_ranged(held) else {
            return;
        };
        let ammo = cc.ammo_def(r.ammo[0]).unwrap_or_default();
        let eye = [me[0], me[1] + EYE_M, me[2]];
        for (name, part) in [("head", HEAD_M), ("chest", CHEST_M)] {
            let at = [it[0], it[1] + part, it[2]];
            if let Some((p, _)) =
                server::agent::aim::lead(eye, at, vel, 1.0, ammo.speed_mmpt, ammo.drop_mmpt2)
            {
                let (px, pz) = (p[0] - eye[0], p[2] - eye[2]);
                let yaw = yaw_toward(px, pz);
                let pitch = pitch_toward(p[1] - eye[1], px.hypot(pz));
                let dy = f32::from(f.yaw.wrapping_sub(yaw) as i16) * 360.0 / 65536.0;
                let dp = (f32::from(f.pitch) - f32::from(pitch)) * 180.0 / 255.0;
                println!(
                    "      SHOT at {name}: yaw off {dy:+.1} deg, pitch off {dp:+.1} deg, foe vel {:.1},{:.1}",
                    vel[0], vel[2]
                );
            }
        }
    }

    pub fn until(&mut self, ticks: u32, done: impl Fn(&Self) -> bool) -> bool {
        for _ in 0..ticks {
            if done(self) {
                return true;
            }
            self.step();
        }
        done(self)
    }

    pub fn explain(&self) -> String {
        format!(
            "tick {} engaged {} ticks, widest turn {}, combat {:?}, stats {:?}",
            self.tick,
            self.engaged_ticks,
            self.max_turn,
            self.bot.combat().stats,
            self.bot.stats
        )
    }

    /// Slot 1's body is on the agent's screen: inside the view cone the
    /// eyes attend to (or anywhere across it at arm's length), in range,
    /// with nothing but the clearing between. The world's truth, for the
    /// reaction gate.
    pub fn in_view(&self) -> bool {
        use server::agent::tracks::{ARMS_LENGTH_M, SCREEN_HALF_COS};
        let (me, it) = (self.pos(self.me), self.pos(self.foe));
        let (dx, dz) = (it[0] - me[0], it[2] - me[2]);
        let d = dx.hypot(dz);
        let (fx, fz) = sim_core::yaw_dir(self.player(self.me).frame.yaw);
        let ahead = dx * fx + dz * fz;
        let cone = if d < ARMS_LENGTH_M {
            SCREEN_HALF_COS
        } else {
            std::f32::consts::FRAC_1_SQRT_2
        };
        d < 120.0 && ahead >= d * cone
    }

    /// Either body is down or dead.
    pub fn decided(&self) -> bool {
        let (me, it) = (self.player(self.me), self.player(self.foe));
        me.wounded || me.dead || it.wounded || it.dead
    }
}

/// What one side of a bout holds: an item, and rounds for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Gear {
    pub item: &'static str,
    pub rounds: Option<(&'static str, u16)>,
}

impl Gear {
    pub const ROCK: Gear = Gear {
        item: "item.bat",
        rounds: None,
    };
    pub const SPEAR: Gear = Gear {
        item: "item.spear_wood",
        rounds: None,
    };
    pub const BOW: Gear = Gear {
        item: "item.bow",
        rounds: Some(("item.arrow_wood", 40)),
    };

    pub fn name(self) -> &'static str {
        match self.item {
            "item.bat" => "rock",
            "item.spear_wood" => "spear",
            "item.bow" => "bow",
            other => other,
        }
    }

    /// Into slot 0, rounds in the pack; everything else emptied.
    fn arm(self, content: &content::Content, p: &mut sim_core::world::Player) {
        for s in p.inv.iter_mut() {
            *s = Default::default();
        }
        p.inv[0] = stack(content, self.item);
        if let Some((round, n)) = self.rounds {
            let mut r = stack(content, round);
            r.count = n;
            p.inv[HOTBAR_SLOTS + 3] = r;
        }
    }
}

/// One bout's setting.
#[derive(Clone, Copy, Debug)]
pub struct Setup {
    pub seat: Seat,
    pub mine: Gear,
    pub theirs: Gear,
    /// How far off the seat starts, metres, and at what bearing off the
    /// agent's facing (0 ahead, 0x8000 behind).
    pub dist_m: f32,
    pub off: u16,
    pub preset: Preset,
    pub temperament: Temperament,
    pub latency: Latency,
    /// Picks the agent's body id (its hands' and its reflex's streams),
    /// the seat's rhythm, and a few degrees on the approach.
    pub seed: u32,
    pub limit_ticks: u32,
    /// Print every tick of the bout (debugging a scenario).
    pub trace: bool,
    /// The seat comes on the moment it is placed: no waiting to be seen.
    pub ambush: bool,
    /// The agent is held where it stands ([`Arena::pin`]).
    pub pinned: bool,
}

impl Setup {
    pub fn new(seat: Seat, mine: Gear, theirs: Gear, dist_m: f32, seed: u32) -> Self {
        Self {
            seat,
            mine,
            theirs,
            dist_m,
            off: 0,
            preset: Preset::Good,
            temperament: Temperament::Opportunist,
            latency: Latency::NONE,
            seed,
            limit_ticks: 60 * TICK_HZ,
            trace: false,
            ambush: false,
            pinned: false,
        }
    }
}

/// How one bout went, as the world tells it.
#[derive(Clone, Copy, Debug, Default)]
pub struct Record {
    pub won: bool,
    pub lost: bool,
    /// Ticks from the seat taking its place to the end.
    pub ticks: u32,
    /// The agent's swings and shots (cadences paid), and those that drew
    /// blood; the same for the seat.
    pub attacks: u32,
    pub hits: u32,
    pub their_attacks: u32,
    pub their_hits: u32,
    /// The agent's shots loosed from [`FAR_SHOT_M`] or further, and those
    /// of its hits that came from one.
    pub far_shots: u32,
    pub far_hits: u32,
    /// Ticks from the seat first standing on the agent's screen
    /// ([`Arena::in_view`]) to the agent's first blow that landed.
    pub sight_to_hit: Option<u32>,
    /// The fewest ticks the agent's hands could need to answer a body.
    pub react_floor: u32,
    pub heap_ops: usize,
    pub max_turn: u16,
    pub turn_cap: u16,
    pub hash: u64,
}

/// Where bouts are fought: a clearing some 50 m across, nothing between the two.
pub fn arena_ground() -> (f32, f32) {
    static AT: std::sync::OnceLock<(f32, f32)> = std::sync::OnceLock::new();
    *AT.get_or_init(|| clearing(3))
}

/// A mind that stands about, looking around: the arena's agent is there to
/// fight, not to wander off after wood while the seat chases it.
pub struct Sentry;

impl server::mind::DecisionSource for Sentry {
    fn kind(&self) -> server::mind::SourceKind {
        server::mind::SourceKind::Scripted
    }

    fn decide(&mut self, _: &server::mind::Summary) -> Result<server::mind::Choice, String> {
        Ok(server::mind::Choice {
            goal: server::mind::Goal::Wait,
            confidence: 1.0,
            reason: server::mind::Reason::from_text("arena: stand about"),
            input_tokens: 0,
            output_tokens: 0,
        })
    }
}

/// One bout: the agent settles in, both sides are armed, the seat is put
/// `dist_m` off and plays until one side is down or time is up.
pub fn bout(s: Setup) -> (Arena, Record) {
    let content = content();
    let me = ID + 2 * s.seed;
    let skill = s.preset.skill();
    let mind = Mind::inline(Sentry, MindConfig::default()).unwrap();
    let opts = SurvivorOpts {
        skill,
        temperament: s.temperament,
        ..SurvivorOpts::default()
    };
    let mut a = Arena::seated(opts, mind, false, me, arena_ground(), true);
    a.latency = s.latency;
    assert!(a.until(900, |a| a.bot.goal().is_some()), "{}", a.explain());
    a.stage(me, |p| {
        s.mine.arm(&content, p);
        p.hp = p.hp_max;
    });
    a.connect_foe();
    a.step();
    let (at, facing) = (a.player(me).body, a.player(me).frame.yaw);
    // A few degrees either way, by seed.
    let wobble = ((s.seed * 0x9e37) % 0x1000) as u16;
    let (fx, fz) = sim_core::yaw_dir(
        facing
            .wrapping_add(s.off)
            .wrapping_add(wobble)
            .wrapping_sub(0x800),
    );
    let (x, z) = (
        at.qx as f32 * POS_XZ_Q + fx * s.dist_m,
        at.qz as f32 * POS_XZ_Q + fz * s.dist_m,
    );
    let body = Body::at(SEED, &a.haven, x, z);
    let foe = a.foe;
    // Facing the agent: two people who are about to fight have seen each
    // other.
    let facing_me = yaw_toward(at.qx as f32 * POS_XZ_Q - x, at.qz as f32 * POS_XZ_Q - z);
    a.stage(foe, |p| {
        s.theirs.arm(&content, p);
        p.body = body;
        p.hp = p.hp_max;
        p.frame.yaw = facing_me;
        p.frame.pitch = server::agent::intent::LEVEL_PITCH;
    });
    // First sight is the world's fact, not the agent's word: the first
    // tick the seat stands on its screen.
    let mut sighted = a.in_view().then_some(a.tick);
    if !s.ambush {
        // The seat stands where it was put until the agent has had it in
        // sight (or for a few seconds): a fight begins with one side
        // seeing the other, not with a blow out of nowhere...
        let seen = |a: &Arena| a.bot.tracks().get(foe).is_some_and(|t| t.visible);
        for _ in 0..SIGHT_WAIT_TICKS {
            if seen(&a) {
                break;
            }
            a.step();
            sighted = sighted.or(a.in_view().then_some(a.tick));
        }
        // ...and a moment more, as two people who have seen each other:
        // the floor is a duel, not an ambush.
        for _ in 0..NOTICE_TICKS {
            a.step();
            sighted = sighted.or(a.in_view().then_some(a.tick));
        }
    }
    a.sit(s.seat, u64::from(s.seed));
    if s.pinned {
        a.pin = Some(a.player(me).body);
    }
    let shoots = a
        .shard
        .world
        .combat
        .held_ranged(sim_core::combat::held_item(a.player(me)))
        .is_some();
    // The range the agent's last shot left from.
    let mut shot_from = 0.0f32;
    let (start, hp0) = (a.tick, a.player(me).hp);
    let mut r = Record {
        react_floor: skill.react_frames,
        turn_cap: skill.max_turn(),
        ..Record::default()
    };
    let (mut my_swing, mut their_swing) = (a.player(me).next_swing, a.player(foe).next_swing);
    let (mut my_hp, mut their_hp) = (hp0, a.player(foe).hp);
    let mut foe_was = a.pos(foe);
    while a.tick - start < s.limit_ticks && !a.decided() {
        foe_was = if s.trace { a.pos(foe) } else { foe_was };
        a.step();
        sighted = sighted.or(a.in_view().then_some(a.tick));
        let (p, q) = (a.player(me), a.player(foe));
        // A cadence paid: a swing or a shot (a relaxed bow creeps by one).
        if p.next_swing > my_swing + 8 {
            r.attacks += 1;
            if shoots {
                shot_from = a.gap();
                r.far_shots += u32::from(shot_from >= FAR_SHOT_M);
            }
            if s.trace {
                a.trace_shot(foe_was);
            }
        }
        if q.next_swing > their_swing + 8 {
            r.their_attacks += 1;
        }
        if q.hp < their_hp {
            r.hits += 1;
            r.far_hits += u32::from(shoots && shot_from >= FAR_SHOT_M);
            if r.sight_to_hit.is_none() {
                // A blow at a body never on screen counts as no reaction at all.
                r.sight_to_hit = Some(sighted.map_or(0, |at| a.tick - at));
            }
        }
        if p.hp < my_hp {
            r.their_hits += 1;
        }
        if s.trace {
            let h = a.bot.hands();
            println!(
                "t{:4} d{:5.2} hp {:3}/{:3} {:?} btn {:02x}/{:02x} mv {:4},{:4} yaw {:5} on {} err {:4.1} sw {}/{}",
                a.tick - start,
                a.gap(),
                p.hp,
                q.hp,
                a.bot.combat().mode(),
                p.frame.buttons,
                q.frame.buttons,
                p.frame.move_x,
                p.frame.move_z,
                p.frame.yaw >> 8,
                h.on_target(),
                h.aim_error_deg(),
                p.next_swing.saturating_sub(a.shard.world.tick),
                q.next_swing.saturating_sub(a.shard.world.tick),
            );
            let inview = a
                .view
                .newest()
                .is_some_and(|n| n.entities().iter().any(|e| e.id == foe));
            let (pa, pb) = (a.pos(me), a.pos(foe));
            let rel = yaw_toward(pb[0] - pa[0], pb[2] - pa[2]).wrapping_sub(p.frame.yaw) as i16;
            println!(
                "      in snapshot {inview} rel {:.0} deg {:?} ack age {} playout {}",
                f32::from(rel) * 360.0 / 65536.0,
                a.bot.tracks().stats,
                (a.tick as u16).wrapping_sub(a.view.last_executed_seq),
                a.bot.tracks().playout()
            );
            if a.shard
                .world
                .combat
                .held_ranged(sim_core::combat::held_item(p))
                .is_some()
            {
                a.trace_shot(foe_was);
            }
            if let Some(t) = a.bot.tracks().get(foe) {
                println!(
                    "      track vis {} aim {} vel {:.1},{:.1} held {:?} swing {:?}",
                    t.visible, t.aiming_at_me, t.vel[0], t.vel[2], t.held, t.last_swing
                );
            }
        }
        (my_swing, their_swing, my_hp, their_hp) = (p.next_swing, q.next_swing, p.hp, q.hp);
    }
    a.seat = None;
    let (p, q) = (a.player(me), a.player(foe));
    r.won = q.wounded || q.dead;
    r.lost = !r.won && (p.wounded || p.dead);
    r.ticks = a.tick - start;
    r.heap_ops = a.heap_ops;
    r.max_turn = a.max_turn;
    r.hash = a.shard.world.state_hash();
    (a, r)
}

/// Totals over a set of bouts.
#[derive(Clone, Copy, Debug, Default)]
pub struct Tally {
    pub bouts: u32,
    pub wins: u32,
    pub losses: u32,
    pub attacks: u32,
    pub hits: u32,
    pub their_attacks: u32,
    pub their_hits: u32,
    pub kill_ticks: u32,
    pub far_shots: u32,
    pub far_hits: u32,
}

impl Tally {
    pub fn add(&mut self, r: &Record) {
        self.bouts += 1;
        self.wins += u32::from(r.won);
        self.losses += u32::from(r.lost);
        self.attacks += r.attacks;
        self.hits += r.hits;
        self.their_attacks += r.their_attacks;
        self.their_hits += r.their_hits;
        self.far_shots += r.far_shots;
        self.far_hits += r.far_hits;
        if r.won {
            self.kill_ticks += r.ticks;
        }
    }

    pub fn win_pct(&self) -> u32 {
        self.wins * 100 / self.bouts.max(1)
    }

    pub fn hit_pct(&self) -> u32 {
        self.hits * 100 / self.attacks.max(1)
    }

    /// Of the shots loosed from [`FAR_SHOT_M`] or further.
    pub fn far_hit_pct(&self) -> u32 {
        self.far_hits * 100 / self.far_shots.max(1)
    }

    pub fn their_hit_pct(&self) -> u32 {
        self.their_hits * 100 / self.their_attacks.max(1)
    }

    pub fn ttk_s(&self) -> f32 {
        if self.wins == 0 {
            return f32::NAN;
        }
        self.kill_ticks as f32 / self.wins as f32 / TICK_HZ as f32
    }
}

/// A results table row: scenario, wins/losses of bouts, the agent's hit
/// rate (and the seat's), average time to kill.
pub fn row(label: &str, t: &Tally) -> String {
    format!(
        "{label:<36} {:>2}/{:<2} W {:>2} L {:>3}% win  hit {:>3}% ({:>2}/{:<2}) seat {:>3}%  ttk {:>5.1}s",
        t.wins,
        t.bouts,
        t.losses,
        t.win_pct(),
        t.hit_pct(),
        t.hits,
        t.attacks,
        t.their_hit_pct(),
        t.ttk_s()
    )
}

/// The label a bout goes under in the table.
pub fn label(s: &Setup) -> String {
    format!(
        "{} v {} {} {:.0}m {} {}ms{}{}",
        s.mine.name(),
        s.theirs.name(),
        s.seat.name(),
        s.dist_m,
        s.preset.name(),
        s.latency.ms(),
        if s.ambush { " ambush" } else { "" },
        if s.pinned { " pinned" } else { "" }
    )
}

/// Run the bouts on every core, in order of the setups given.
pub fn sweep(setups: &[Setup]) -> Vec<Record> {
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let next = std::sync::atomic::AtomicUsize::new(0);
    let mut out = vec![Record::default(); setups.len()];
    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                scope.spawn(|| {
                    let mut done = Vec::new();
                    loop {
                        let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(s) = setups.get(i) else {
                            return done;
                        };
                        done.push((i, bout(*s).1));
                    }
                })
            })
            .collect();
        for w in workers {
            for (i, r) in w.join().unwrap() {
                out[i] = r;
            }
        }
    });
    out
}

/// Print the table, one row per label in first-seen order; the totals by
/// label come back for the gates.
pub fn table(title: &str, setups: &[Setup], records: &[Record]) -> Vec<(String, Tally)> {
    let mut rows: Vec<(String, Tally)> = Vec::new();
    for (s, r) in setups.iter().zip(records) {
        let l = label(s);
        match rows.iter_mut().find(|(k, _)| *k == l) {
            Some((_, t)) => t.add(r),
            None => {
                let mut t = Tally::default();
                t.add(r);
                rows.push((l, t));
            }
        }
    }
    println!("\n== {title} ==");
    for (l, t) in &rows {
        println!("{}", row(l, t));
    }
    rows
}
