//! The arena (lane A): the agent against a scripted attacker, in lockstep
//! against `ShardCore` with the shipped content, the inline scripted mind
//! and a synthetic clock, so a run is a function of its inputs.
//!
//! The attacker is test code and may read the world: a rusher that walks
//! straight at the agent with perfect yaw and holds primary once in reach.
//! What is gated is that the agent fights back — it lands blows instead of
//! only backing away — and that it does so as a player would: its frame
//! loop never allocates, and its view never turns faster than its hands'
//! preset allows. And that it does not fight what is not a fight: a second
//! agent harvesting beside it, or a body it has no way to reach.

mod common;

use common::{content, scene, shard, stack, SEED};
use protocol::{decode_action, decode_input, encode_input, InputDatagram};
use server::agent::combat::{Mode, Temperament};
use server::agent::intent::yaw_toward;
use server::core::{Lane, ShardCore};
use server::explorer::{Survivor, SurvivorOpts};
use server::mind::{Mind, MindConfig, Scripted};
use server::stats::ShardStats;
use server::view::ClientView;
use sim_core::input::{InputFrame, BTN_PRIMARY, BTN_SPRINT};
use sim_core::limits::{HOTBAR_SLOTS, TICK_HZ};
use sim_core::movement::{Body, POS_XZ_Q};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
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

const ID: u32 = 256;
const RUSHER: u32 = 257;

/// A body that walks straight at the agent and swings once in reach.
#[derive(Clone, Copy)]
struct Rusher {
    /// Centre to centre, where its swing lands.
    reach_m: f32,
}

/// A body standing still beside the agent, holding primary, its look
/// passing a step to one side of it: chopping at the air beside it, the
/// way a body chops a tree next to it.
#[derive(Clone, Copy)]
struct Chopper {
    yaw: u16,
}

/// A second agent on slot 1, as body [`RUSHER`].
struct Peer {
    bot: Survivor,
    view: ClientView,
}

struct Arena {
    shard: Box<ShardCore>,
    stats: ShardStats,
    view: ClientView,
    bot: Survivor,
    t0: Instant,
    tick: u32,
    heap_ops: usize,
    last_yaw: Option<u16>,
    max_turn: u16,
    engaged_ticks: u32,
    rusher: Option<Rusher>,
    peer: Option<Peer>,
    /// The other body is put back this far off the agent along this world
    /// bearing every tick: however the agent walks, it never gets nearer.
    keep_off: Option<(f32, u16)>,
    chopper: Option<Chopper>,
    /// The other body stands where it is with a bow up, eyes on the
    /// agent's chest; drawing and loosing when `true`.
    archer: Option<bool>,
    haven: sim_core::terrain::Haven,
}

impl Arena {
    fn new(temperament: Temperament) -> Self {
        let mind = Mind::inline(Scripted::default(), MindConfig::default()).unwrap();
        Self::with(temperament, mind, false)
    }

    /// With a mind of the test's choosing, and animals if asked for.
    fn with(temperament: Temperament, mind: Mind, wildlife: bool) -> Self {
        let content = content();
        let mut bot = Survivor::with(
            mind,
            SurvivorOpts {
                temperament,
                ..SurvivorOpts::default()
            },
        );
        use server::botclient::BotDriver;
        bot.welcome(&protocol::Welcome {
            seed: SEED,
            player_id: ID,
            tick: 0,
            dev: true,
        });
        Self {
            shard: shard(&content, scene(), wildlife, ID),
            stats: ShardStats::default(),
            view: ClientView::new(),
            bot,
            t0: Instant::now(),
            tick: 0,
            heap_ops: 0,
            last_yaw: None,
            max_turn: 0,
            engaged_ticks: 0,
            rusher: None,
            peer: None,
            keep_off: None,
            chopper: None,
            archer: None,
            haven: sim_core::terrain::haven(SEED),
        }
    }

    /// A second agent joins beside the first, on slot 1.
    fn with_peer(&mut self, temperament: Temperament) {
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
            player_id: RUSHER,
            tick: self.tick,
            dev: true,
        });
        assert!(self.shard.connect(1, RUSHER));
        self.peer = Some(Peer {
            bot,
            view: ClientView::new(),
        });
    }

    fn player(&self, id: u32) -> &sim_core::world::Player {
        self.shard
            .world
            .players
            .iter()
            .find(|p| p.active && p.id == id)
            .unwrap()
    }

    fn stage(&mut self, id: u32, f: impl Fn(&mut sim_core::world::Player)) {
        let p = self
            .shard
            .world
            .players
            .iter_mut()
            .find(|p| p.active && p.id == id)
            .unwrap();
        f(p);
    }

    fn push(&mut self, slot: usize, frame: InputFrame, ack: (u16, u32)) {
        let mut dg = InputDatagram::new(ack.0, ack.1, sim_core::limits::INTERP_DELAY_TICKS);
        dg.push(frame).unwrap();
        let mut bytes = [0u8; sim_core::limits::DATAGRAM_BUDGET_BYTES];
        let len = encode_input(&dg, &mut bytes).unwrap();
        self.shard
            .push_input(slot, &decode_input(&bytes[..len]).unwrap());
    }

    /// The rusher's frame: face the agent, walk in, swing in reach.
    fn rusher_frame(&self, r: Rusher) -> InputFrame {
        let (me, them) = (self.player(RUSHER).body, self.player(ID).body);
        let dx = (them.qx - me.qx) as f32 * POS_XZ_Q;
        let dz = (them.qz - me.qz) as f32 * POS_XZ_Q;
        let d = dx.hypot(dz);
        let mut buttons = 0;
        if d > 3.0 {
            buttons |= BTN_SPRINT;
        }
        if d <= r.reach_m {
            buttons |= BTN_PRIMARY;
        }
        InputFrame {
            seq: self.tick as u16,
            yaw: yaw_toward(dx, dz),
            pitch: 128,
            move_z: if d > 0.9 { 127 } else { 0 },
            buttons,
            sel: 0,
            ..Default::default()
        }
    }

    fn step(&mut self) {
        use server::botclient::BotDriver;
        let now = self.t0 + Duration::from_secs_f64(f64::from(self.tick) / f64::from(TICK_HZ));
        let mut act = [0u8; protocol::MAX_STREAM_MSG_BYTES];
        ALLOCS.with(|c| c.set(Some(0)));
        let frame = self.bot.frame_at(&self.view, ID, self.tick as u16, now);
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
        self.push(0, frame, ack);
        if let Some(len) = action {
            let msg = decode_action(&act[..len]).unwrap();
            assert!(self.shard.wants_action(0));
            self.shard.push_action(0, msg);
        }
        if let Some(r) = self.rusher {
            let frame = self.rusher_frame(r);
            self.push(1, frame, (0, 0));
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
            let (me, them) = (self.player(RUSHER).body, self.player(ID).body);
            let dx = (them.qx - me.qx) as f32 * POS_XZ_Q;
            let dz = (them.qz - me.qz) as f32 * POS_XZ_Q;
            let dy = (them.qy - me.qy) as f32 * sim_core::movement::POS_Y_Q - 0.4;
            let frame = InputFrame {
                seq: self.tick as u16,
                yaw: yaw_toward(dx, dz),
                pitch: server::agent::intent::pitch_toward(dy, dx.hypot(dz)),
                buttons: if shoot {
                    sim_core::input::BTN_AIM | BTN_PRIMARY
                } else {
                    0
                },
                sel: 0,
                ..Default::default()
            };
            self.push(1, frame, (0, 0));
        }
        if let Some(mut peer) = self.peer.take() {
            ALLOCS.with(|c| c.set(Some(0)));
            let frame = peer.bot.frame_at(&peer.view, RUSHER, self.tick as u16, now);
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
        let (view, bot, peer) = (&mut self.view, &mut self.bot, &mut self.peer);
        self.shard.tick_bare(&self.stats, |lane, slot, bytes| {
            let (view, bot) = match (slot, peer.as_mut()) {
                (0, _) => (&mut *view, &mut *bot),
                (1, Some(p)) => (&mut p.view, &mut p.bot),
                _ => return true,
            };
            match lane {
                Lane::Snapshot => {
                    view.apply(bytes).unwrap();
                }
                Lane::Event => bot.event(bytes).unwrap(),
            }
            true
        });
        if let Some((off, bearing)) = self.keep_off {
            let at = self.player(ID).body;
            let (fx, fz) = sim_core::yaw_dir(bearing);
            let (x, z) = (
                at.qx as f32 * POS_XZ_Q + fx * off,
                at.qz as f32 * POS_XZ_Q + fz * off,
            );
            let body = Body::at(SEED, &self.haven, x, z);
            self.stage(RUSHER, |p| p.body = body);
        }
        self.tick += 1;
    }

    fn until(&mut self, ticks: u32, done: impl Fn(&Self) -> bool) -> bool {
        for _ in 0..ticks {
            if done(self) {
                return true;
            }
            self.step();
        }
        done(self)
    }

    fn explain(&self) -> String {
        format!(
            "tick {} engaged {} ticks, widest turn {}, combat {:?}, stats {:?}",
            self.tick,
            self.engaged_ticks,
            self.max_turn,
            self.bot.combat().stats,
            self.bot.stats
        )
    }
}

/// One bout: the agent holds `mine`; another body holding `theirs` (if
/// anything) stands 8 m off, in front of it or behind, and rushes it or
/// stands still. Returns the arena when either side is down or a minute
/// has passed.
fn bout(
    temperament: Temperament,
    mine: &str,
    theirs: Option<&str>,
    ahead: bool,
    rush: bool,
) -> Arena {
    let content = content();
    let mut a = Arena::new(temperament);
    // The agent settles in: it has looked around and chosen a goal.
    assert!(a.until(900, |a| a.bot.goal().is_some()), "{}", a.explain());
    let weapon = stack(&content, mine);
    a.stage(ID, |p| {
        p.inv[0] = weapon;
        p.hp = p.hp_max;
    });
    assert!(a.shard.connect(1, RUSHER));
    a.step();
    let (at, facing) = (a.player(ID).body, a.player(ID).frame.yaw);
    let (fx, fz) = sim_core::yaw_dir(facing);
    let off = if ahead { 8.0 } else { -8.0 };
    let (x, z) = (
        at.qx as f32 * POS_XZ_Q + fx * off,
        at.qz as f32 * POS_XZ_Q + fz * off,
    );
    let theirs = theirs.map(|id| stack(&content, id));
    let reach_cm = theirs
        .and_then(|s| a.shard.world.combat.held_melee(s.item))
        .map_or(0, |m| m.reach_cm);
    let body = Body::at(SEED, &a.haven, x, z);
    a.stage(RUSHER, |p| {
        p.body = body;
        p.inv[0] = theirs.unwrap_or_default();
    });
    a.rusher = rush.then_some(Rusher {
        reach_m: f32::from(reach_cm) * 0.01 + sim_core::collide::CAPSULE_RADIUS_M - 0.1,
    });
    a.until(60 * TICK_HZ, |a| {
        let (me, it) = (a.player(ID), a.player(RUSHER));
        me.wounded || me.dead || it.wounded || it.dead
    });
    a.rusher = None;
    a
}

/// Rushed at equal gear, the agent fights back: it engages, lands blows,
/// and does it with a person's hands and without allocating.
#[test]
fn a_rushed_survivor_fights_back_at_equal_gear() {
    let a = bout(
        Temperament::Opportunist,
        "item.rock",
        Some("item.rock"),
        false,
        true,
    );
    println!("rock v rock: {}", a.explain());
    let (me, it) = (a.player(ID), a.player(RUSHER));
    println!(
        "agent hp {} wounded {} dead {}; rusher hp {} wounded {} dead {}",
        me.hp, me.wounded, me.dead, it.hp, it.wounded, it.dead
    );
    assert!(a.bot.combat().stats.engages > 0, "{}", a.explain());
    assert!(a.bot.stats.landed > 0, "it only fled: {}", a.explain());
    assert!(a.engaged_ticks > 0);
    // Even gear, and the rusher never stops: losing, it runs. Whether a
    // rusher as fast as it catches it in the end is the ground's call.
    assert!(a.bot.combat().stats.escapes > 0, "{}", a.explain());
    assert_eq!(
        a.heap_ops, 0,
        "the agent's frame loop touched the allocator"
    );
    assert!(a.max_turn > 0);
}

/// With the longer reach, the agent beats a rock.
#[test]
fn a_spear_beats_a_rock() {
    let a = bout(
        Temperament::Opportunist,
        "item.spear_wood",
        Some("item.rock"),
        true,
        true,
    );
    println!("spear v rock: {}", a.explain());
    let (me, it) = (a.player(ID), a.player(RUSHER));
    println!(
        "agent hp {} wounded {} dead {}; rusher hp {} wounded {} dead {}",
        me.hp, me.wounded, me.dead, it.hp, it.wounded, it.dead
    );
    assert!(a.bot.stats.landed > 0, "{}", a.explain());
    assert!(
        it.wounded || it.dead,
        "the rusher is still up: {}",
        a.explain()
    );
    assert!(!me.wounded && !me.dead, "{}", a.explain());
    assert_eq!(a.heap_ops, 0);
}

/// An unarmed body standing in front of it is an opening an opportunist
/// takes, and a defensive body leaves alone.
#[test]
fn an_opportunist_takes_an_opening_a_defender_does_not() {
    let a = bout(Temperament::Opportunist, "item.rock", None, true, false);
    println!("opportunist: {}", a.explain());
    let it = a.player(RUSHER);
    assert!(a.bot.stats.landed > 0, "{}", a.explain());
    assert!(it.wounded || it.dead, "{}", a.explain());
    assert_eq!(a.bot.stats.hurts, 0);
    assert_eq!(a.heap_ops, 0);
    let a = {
        let mut a = bout(Temperament::Defensive, "item.rock", None, true, false);
        a.until(20 * TICK_HZ, |_| false);
        a
    };
    println!("defender: {}", a.explain());
    assert_eq!(a.bot.combat().stats.engages, 0, "{}", a.explain());
    assert_eq!(a.bot.stats.landed, 0);
}

/// Two agents at one spawn, both armed, go about their gathering side by
/// side for a minute and a half: a harvest swing beside a body is not an
/// attack on it, and neither turns on the other.
#[test]
fn two_gatherers_side_by_side_do_not_fight() {
    use server::explorer::Phase;
    let mut a = Arena::new(Temperament::Opportunist);
    a.with_peer(Temperament::Opportunist);
    let mut closest = f32::MAX;
    for _ in 0..90 * TICK_HZ {
        a.step();
        let peer = &a.peer.as_ref().unwrap().bot;
        if a.bot.stats.phase == Phase::Harvesting && peer.stats.phase == Phase::Harvesting {
            let (me, it) = (a.player(ID).body, a.player(RUSHER).body);
            let d = ((me.qx - it.qx) as f32 * POS_XZ_Q).hypot((me.qz - it.qz) as f32 * POS_XZ_Q);
            closest = closest.min(d);
        }
    }
    let peer = &a.peer.as_ref().unwrap().bot;
    println!(
        "closest while both harvest {closest:.2} m\nagent: {}\npeer: {:?} {:?}",
        a.explain(),
        peer.combat().stats,
        peer.stats
    );
    assert!(a.bot.stats.gather_awards > 0 && peer.stats.gather_awards > 0);
    for (who, bot) in [("agent", &a.bot), ("peer", peer)] {
        assert_eq!(bot.combat().stats.engages, 0, "{who} picked a fight");
        assert_eq!(bot.stats.hurts, 0, "{who} was struck");
        assert_eq!(bot.stats.landed, 0, "{who} struck a body");
    }
    assert_eq!(a.heap_ops, 0);
}

/// An unarmed body is an opening an opportunist takes; one it never gets
/// any nearer to (the test keeps it a fixed way off, however the agent
/// walks) is let go in seconds and not taken again straight away.
#[test]
fn a_foe_that_cannot_be_reached_is_let_go() {
    let content = content();
    let mut a = Arena::new(Temperament::Opportunist);
    assert!(a.until(900, |a| a.bot.goal().is_some()), "{}", a.explain());
    let rock = stack(&content, "item.rock");
    a.stage(ID, |p| {
        p.inv[0] = rock;
        p.hp = p.hp_max;
    });
    assert!(a.shard.connect(1, RUSHER));
    a.step();
    a.stage(RUSHER, |p| p.inv[0] = Default::default());
    let facing = a.player(ID).frame.yaw;
    a.keep_off = Some((6.0, facing));
    a.step();
    assert!(
        a.until(10 * TICK_HZ, |a| a.bot.combat().stats.engages > 0),
        "never took the opening: {}",
        a.explain()
    );
    a.until(20 * TICK_HZ, |_| false);
    println!("unreachable: {}", a.explain());
    let stats = a.bot.combat().stats;
    assert_eq!(stats.engages, 1, "it went back for it: {}", a.explain());
    assert!(stats.parted > 0, "{}", a.explain());
    assert_ne!(a.bot.combat().mode(), Mode::Engage, "{}", a.explain());
    assert!(
        a.engaged_ticks < 6 * TICK_HZ,
        "stood there {} ticks",
        a.engaged_ticks
    );
    assert_eq!(a.bot.stats.landed, 0);
    assert_eq!(a.heap_ops, 0);
}

/// A body swinging a rock just ahead of the agent while it harvests, its
/// look passing a step to one side of it (the swing a body chopping beside
/// it makes), is not attacking it: the agent keeps at its work. Beyond
/// the margin a harvest swing clears a body by, and inside the metre a
/// look ray was once called "at me" by.
#[test]
fn a_swing_beside_me_is_not_an_attack() {
    use server::explorer::Phase;
    const AHEAD_M: f32 = 1.0;
    const RIGHT_M: f32 = 1.0;
    const ASIDE_M: f32 = 0.8;
    let content = content();
    let mut a = Arena::new(Temperament::Defensive);
    let rock = stack(&content, "item.rock");
    assert!(a.shard.connect(1, RUSHER));
    a.step();
    a.stage(RUSHER, |p| p.inv[0] = rock);
    assert!(
        a.until(60 * TICK_HZ, |a| a.bot.stats.phase == Phase::Harvesting),
        "{}",
        a.explain()
    );
    // Ahead and to the right of it, looking back past its right side.
    let me = a.player(ID);
    let (x, z) = (me.body.qx as f32 * POS_XZ_Q, me.body.qz as f32 * POS_XZ_Q);
    let (fx, fz) = sim_core::yaw_dir(me.frame.yaw);
    let (rx, rz) = (fz, -fx);
    let (cx, cz) = (
        x + fx * AHEAD_M + rx * RIGHT_M,
        z + fz * AHEAD_M + rz * RIGHT_M,
    );
    let gap = (x - cx).hypot(z - cz);
    let (ux, uz) = ((x - cx) / gap, (z - cz) / gap);
    let (tx, tz) = (x - uz * ASIDE_M, z + ux * ASIDE_M);
    let body = Body::at(SEED, &a.haven, cx, cz);
    a.stage(RUSHER, |p| p.body = body);
    a.chopper = Some(Chopper {
        yaw: yaw_toward(tx - cx, tz - cz),
    });
    let mut beside = 0;
    for _ in 0..10 * TICK_HZ {
        a.step();
        if a.bot.stats.phase != Phase::Harvesting {
            break;
        }
        beside += 1;
    }
    a.chopper = None;
    println!("chopper: {beside} ticks beside it; {}", a.explain());
    assert!(beside >= 2 * TICK_HZ, "left its work: {}", a.explain());
    assert_eq!(a.bot.stats.hurts, 0, "the swing landed: {}", a.explain());
    assert_eq!(
        a.bot.combat().stats.engages,
        0,
        "took a swing beside it for an attack: {}",
        a.explain()
    );
    assert_eq!(a.heap_ops, 0);
}

/// Hurt with nobody about, it heals, and counts what the bandages already
/// used are still delivering (the server folds one's remainder into the
/// next): from 40 hp it takes three of five, not the lot, to reach 90 %.
#[test]
fn a_heal_counts_the_bandages_still_working() {
    let content = content();
    let mut a = Arena::new(Temperament::Opportunist);
    assert!(a.until(900, |a| a.bot.goal().is_some()), "{}", a.explain());
    let mut bandages = stack(&content, "item.bandage");
    bandages.count = 5;
    let slot = sim_core::limits::HOTBAR_SLOTS + 2;
    a.stage(ID, |p| {
        p.inv[slot] = bandages;
        p.hp = 40;
    });
    let full = |a: &Arena| {
        let p = a.player(ID);
        u32::from(p.hp) * 100 >= u32::from(p.hp_max) * 90 && p.heal_rem == 0
    };
    assert!(a.until(40 * TICK_HZ, full), "{}", a.explain());
    let left = a.player(ID).inv[slot].count;
    println!(
        "healed to {} with {left} left: {}",
        a.player(ID).hp,
        a.explain()
    );
    assert_eq!(left, 2, "used {} bandages", 5 - left);
    assert_eq!(a.heap_ops, 0);
}

/// A mind that heals whenever healing is on offer and otherwise waits.
struct Healer;

impl server::mind::DecisionSource for Healer {
    fn kind(&self) -> server::mind::SourceKind {
        server::mind::SourceKind::Scripted
    }

    fn decide(&mut self, s: &server::mind::Summary) -> Result<server::mind::Choice, String> {
        use server::mind::Goal;
        let goal = if s.offers(Goal::Heal) {
            Goal::Heal
        } else {
            Goal::Wait
        };
        Ok(server::mind::Choice {
            goal,
            confidence: 1.0,
            reason: server::mind::Reason::from_text("test: heal when offered"),
            input_tokens: 0,
            output_tokens: 0,
        })
    }
}

/// The heal goal on its own, above where the reflex bandages: told to
/// heal at 85 %, one bandage takes it past 90 % and the goal ends done
/// with that one counted, the rest left in the pack.
#[test]
fn a_heal_goal_uses_what_it_needs_and_ends_done() {
    use server::mind::{Goal, Outcome};
    let content = content();
    let cfg = MindConfig {
        heartbeat: Duration::from_secs(1),
        per_hour: 100_000,
        per_day: 100_000,
        ..MindConfig::default()
    };
    let mut a = Arena::with(
        Temperament::Opportunist,
        Mind::inline(Healer, cfg).unwrap(),
        false,
    );
    assert!(a.until(900, |a| a.bot.goal().is_some()), "{}", a.explain());
    let mut bandages = stack(&content, "item.bandage");
    bandages.count = 5;
    let slot = HOTBAR_SLOTS + 2;
    a.stage(ID, |p| {
        p.inv[slot] = bandages;
        p.hp = p.hp_max * 85 / 100;
    });
    let healed = |a: &Arena| {
        a.bot
            .memory()
            .last
            .is_some_and(|r| r.goal == Goal::Heal && r.outcome != Outcome::Running)
    };
    assert!(a.until(30 * TICK_HZ, healed), "{}", a.explain());
    let report = a.bot.memory().last.unwrap();
    let p = a.player(ID);
    println!(
        "heal goal: {report:?}, {} hp, {} left; {}",
        p.hp,
        p.inv[slot].count,
        a.explain()
    );
    assert_eq!(report.outcome, Outcome::Done, "{}", a.explain());
    assert_eq!(report.gained, 1, "{}", a.explain());
    assert_eq!(p.inv[slot].count, 4);
    assert!(u32::from(p.hp) * 100 >= u32::from(p.hp_max) * 90);
    assert_eq!(a.bot.stats.reflex_heals, 0, "the reflex took it");
    assert_eq!(a.heap_ops, 0);
}

/// A mind that hunts whatever animal is on offer and otherwise waits,
/// looking about: the test's way of pointing the agent at the pig.
struct Hunter;

impl server::mind::DecisionSource for Hunter {
    fn kind(&self) -> server::mind::SourceKind {
        server::mind::SourceKind::Scripted
    }

    fn decide(&mut self, s: &server::mind::Summary) -> Result<server::mind::Choice, String> {
        use server::mind::Goal;
        let goal = if s.offers(Goal::Hunt) {
            Goal::Hunt
        } else {
            Goal::Wait
        };
        Ok(server::mind::Choice {
            goal,
            confidence: 1.0,
            reason: server::mind::Reason::from_text("test: hunt what is in view"),
            input_tokens: 0,
            output_tokens: 0,
        })
    }
}

/// A hunter on the island with one pig: every other animal slot is
/// emptied, and the pig stands `off` metres ahead of the agent, grazing.
/// The agent holds a rock (slot 0) and `weapon` (slot 1), with `rounds` of
/// `round` in the pack.
fn hunt(weapon: &str, round: &str, rounds: u16, off: f32) -> (Arena, usize) {
    let content = content();
    // Asked every second, so a pig glimpsed while looking about is hunted
    // while it is still fresh in the mind's census.
    let cfg = MindConfig {
        heartbeat: Duration::from_secs(1),
        per_hour: 100_000,
        per_day: 100_000,
        ..MindConfig::default()
    };
    let mind = Mind::inline(Hunter, cfg).unwrap();
    let mut a = Arena::with(Temperament::Opportunist, mind, true);
    a.step();
    for m in a.shard.world.mobs.m.iter_mut() {
        m.alive = false;
        m.homed = false;
    }
    let pig = (0..sim_core::limits::MAX_MOBS)
        .find(|&s| sim_core::mob::kind_of(s) == sim_core::mob::MOB_PIG)
        .unwrap();
    assert!(a.until(900, |a| a.bot.goal().is_some()), "{}", a.explain());
    let (rock, gun) = (stack(&content, "item.rock"), stack(&content, weapon));
    let mut ammo = stack(&content, round);
    ammo.count = rounds;
    a.stage(ID, |p| {
        p.inv[0] = rock;
        p.inv[1] = gun;
        p.inv[HOTBAR_SLOTS + 3] = ammo;
        p.hp = p.hp_max;
    });
    let me = a.player(ID);
    let (fx, fz) = sim_core::yaw_dir(me.frame.yaw);
    let (x, z) = (
        me.body.qx as f32 * POS_XZ_Q + fx * off,
        me.body.qz as f32 * POS_XZ_Q + fz * off,
    );
    let body = Body::at(SEED, &a.haven, x, z);
    let hp = a.shard.world.mob.def(sim_core::mob::MOB_PIG).hp;
    let m = &mut a.shard.world.mobs.m[pig];
    m.homed = true;
    m.alive = true;
    m.hp = hp;
    m.body = body;
    m.home_qx = body.qx;
    m.home_qz = body.qz;
    m.respawn_at = u64::MAX;
    (a, pig)
}

fn count_of(a: &Arena, id: &str) -> u32 {
    let item = content().item_index(id).unwrap();
    a.player(ID)
        .inv
        .iter()
        .filter(|s| s.count > 0 && s.item == item)
        .map(|s| u32::from(s.count))
        .sum()
}

/// With a bow and arrows, the agent stalks a pig crouched, shoots it dead
/// from range (drawing, leading, loosing only once settled), then walks
/// to the carcass bag and loots it, and picks up the arrows lying about.
#[test]
fn the_agent_kills_a_pig_with_a_bow_and_loots_it() {
    use sim_core::input::{BTN_AIM, BTN_CROUCH};
    let (mut a, pig) = hunt("item.bow", "item.arrow_wood", 20, 24.0);
    let mut pressed = 0u8;
    // The rock is in slot 0: a kill with it would be no bow kill.
    let mut clubbed = 0u32;
    for _ in 0..90 * TICK_HZ {
        if !a.shard.world.mobs.m[pig].alive {
            break;
        }
        let f = a.player(ID).frame;
        pressed |= f.buttons;
        if f.buttons & BTN_PRIMARY != 0 && f.sel == 0 {
            clubbed += 1;
        }
        a.step();
    }
    assert!(
        !a.shard.world.mobs.m[pig].alive,
        "the pig lives: {}",
        a.explain()
    );
    let arrows = count_of(&a, "item.arrow_wood");
    let looted = a.until(30 * TICK_HZ, |a| {
        count_of(a, "item.raw_meat") > 0 && !a.bot.combat().engaged()
    });
    let c = a.bot.combat().stats;
    println!(
        "bow hunt: arrows {arrows} after the kill, {} after the pickups; {}",
        count_of(&a, "item.arrow_wood"),
        a.explain()
    );
    assert!(pressed & BTN_AIM != 0, "never drew the bow");
    assert_eq!(clubbed, 0, "swung the rock at the pig: {}", a.explain());
    assert!(pressed & BTN_CROUCH != 0, "never crouched to stalk");
    assert!(c.shots >= 1 && c.won >= 1, "{}", a.explain());
    assert!(looted, "never looted the carcass: {}", a.explain());
    assert!(c.loots >= 1 && c.pickups >= 1, "{}", a.explain());
    assert!(
        count_of(&a, "item.arrow_wood") > arrows,
        "no arrow picked back up: {}",
        a.explain()
    );
    assert_eq!(a.heap_ops, 0);
}

/// With a revolver (bought dry, as it comes off the bench), the agent
/// loads it before it can shoot, kills the pig with it, and tops the
/// cylinder up once things are quiet.
#[test]
fn a_dry_revolver_is_loaded_fired_and_topped_up() {
    let (mut a, pig) = hunt("item.revolver", "item.pistol_ammo", 24, 20.0);
    let gun = content().item_index("item.revolver").unwrap();
    let def = a.shard.world.combat.held_ranged(gun).unwrap();
    let mag = |a: &Arena| a.player(ID).mag[usize::from(def.mag_slot)];
    assert_eq!(mag(&a), 0, "the revolver comes dry");
    // The server's own count: loaded (accepted, not just asked for)
    // before the first shot.
    let mut loaded_first = None;
    for _ in 0..90 * TICK_HZ {
        if !a.shard.world.mobs.m[pig].alive {
            break;
        }
        if mag(&a) > 0 && loaded_first.is_none() {
            loaded_first = Some(a.bot.combat().stats.shots);
        }
        a.step();
    }
    assert!(
        !a.shard.world.mobs.m[pig].alive,
        "the pig lives: {}",
        a.explain()
    );
    assert_eq!(loaded_first, Some(0), "no load before the first shot");
    let after_kill = mag(&a);
    a.until(20 * TICK_HZ, |_| false);
    let c = a.bot.combat().stats;
    println!(
        "revolver: mag {after_kill} after the kill, {} now, rounds {}; {}",
        mag(&a),
        count_of(&a, "item.pistol_ammo"),
        a.explain()
    );
    assert!(c.shots >= 1 && c.won >= 1, "{}", a.explain());
    assert!(after_kill < def.magazine, "no shot spent: {}", a.explain());
    assert_eq!(mag(&a), def.magazine, "never topped up: {}", a.explain());
    assert_eq!(a.heap_ops, 0);
}

/// Hurt in a fight it wins, the agent bandages itself once the foe is dead
/// and its bag taken: under whatever goal it had, with no heal goal asked
/// for. The bandages reach its pack only once the fight is under way, so
/// none goes on before it.
#[test]
fn the_agent_heals_after_a_fight() {
    let content = content();
    let mut a = Arena::new(Temperament::Opportunist);
    assert!(a.until(900, |a| a.bot.goal().is_some()), "{}", a.explain());
    let spear = stack(&content, "item.spear_wood");
    a.stage(ID, |p| {
        p.inv[0] = spear;
        p.hp = p.hp_max;
    });
    assert!(a.shard.connect(1, RUSHER));
    a.step();
    let (at, facing) = (a.player(ID).body, a.player(ID).frame.yaw);
    let (fx, fz) = sim_core::yaw_dir(facing);
    let (x, z) = (
        at.qx as f32 * POS_XZ_Q + fx * 8.0,
        at.qz as f32 * POS_XZ_Q + fz * 8.0,
    );
    let rock = stack(&content, "item.rock");
    let body = Body::at(SEED, &a.haven, x, z);
    a.stage(RUSHER, |p| {
        p.body = body;
        p.inv[0] = rock;
    });
    let reach_cm = a.shard.world.combat.held_melee(rock.item).unwrap().reach_cm;
    a.rusher = Some(Rusher {
        reach_m: f32::from(reach_cm) * 0.01 + sim_core::collide::CAPSULE_RADIUS_M - 0.1,
    });
    assert!(
        a.until(20 * TICK_HZ, |a| a.bot.combat().mode() == Mode::Engage),
        "{}",
        a.explain()
    );
    let slot = HOTBAR_SLOTS + 2;
    let mut bandages = stack(&content, "item.bandage");
    bandages.count = 3;
    a.stage(ID, |p| p.inv[slot] = bandages);
    let won = a.until(60 * TICK_HZ, |a| {
        a.player(RUSHER).dead || a.player(ID).wounded || a.player(ID).dead
    });
    a.rusher = None;
    assert!(won && a.player(RUSHER).dead, "{}", a.explain());
    assert_eq!(a.player(ID).inv[slot].count, 3, "a bandage mid-fight");
    let over = a.until(30 * TICK_HZ, |a| !a.bot.combat().engaged());
    let hp = a.player(ID).hp;
    assert!(over, "{}", a.explain());
    assert!(
        u32::from(hp) * 100 < u32::from(a.player(ID).hp_max) * 80,
        "not hurt enough to need a bandage: {hp}"
    );
    let healed = a.until(20 * TICK_HZ, |a| {
        let p = a.player(ID);
        p.hp > hp && p.inv[slot].count < 3
    });
    println!(
        "heal: {hp} hp after the fight, {} now, {} bandages left; {}",
        a.player(ID).hp,
        a.player(ID).inv[slot].count,
        a.explain()
    );
    assert!(healed, "no bandage after the fight: {}", a.explain());
    assert!(a.bot.stats.reflex_heals >= 1, "{}", a.explain());
    assert_eq!(a.heap_ops, 0);
}

/// A body aiming a bow at the agent from 25 m is sidestepped, eyes on it;
/// once it looses arrows, the agent (nothing to shoot back with) runs for
/// a trunk or a rock between them, weaving, rather than across open ground.
#[test]
fn an_archer_is_sidestepped_then_run_from_to_cover() {
    let content = content();
    // A body standing about (the hunter's mind waits with nothing to
    // hunt), so the test is about the archer and not about exploring.
    let mind = Mind::inline(Hunter, MindConfig::default()).unwrap();
    let mut a = Arena::with(Temperament::Opportunist, mind, false);
    // The archer joins with the agent, far off with its bow already up:
    // first seen holding it.
    assert!(a.shard.connect(1, RUSHER));
    a.step();
    let at = a.player(ID).body;
    let bow = stack(&content, "item.bow");
    let far = Body::at(
        SEED,
        &a.haven,
        at.qx as f32 * POS_XZ_Q + 150.0,
        at.qz as f32 * POS_XZ_Q,
    );
    a.stage(RUSHER, |p| {
        p.inv[0] = bow;
        p.body = far;
    });
    assert!(a.until(900, |a| a.bot.goal().is_some()), "{}", a.explain());
    let spear = stack(&content, "item.spear_wood");
    a.stage(ID, |p| {
        p.inv[0] = spear;
        p.hp = p.hp_max;
    });
    a.step();
    let (at, facing) = (a.player(ID).body, a.player(ID).frame.yaw);
    let (fx, fz) = sim_core::yaw_dir(facing);
    let (x, z) = (
        at.qx as f32 * POS_XZ_Q + fx * 18.0,
        at.qz as f32 * POS_XZ_Q + fz * 18.0,
    );
    let body = Body::at(SEED, &a.haven, x, z);
    a.stage(RUSHER, |p| p.body = body);
    a.archer = Some(false);
    let dodged = a.until(10 * TICK_HZ, |a| a.bot.combat().stats.evades > 0);
    assert!(dodged, "never sidestepped the bow: {}", a.explain());
    // Side to side: the path is long, the ground covered short.
    let mut moved = 0.0;
    for _ in 0..2 * TICK_HZ {
        let from = a.player(ID).body;
        a.step();
        let to = a.player(ID).body;
        moved += ((to.qx - from.qx) as f32 * POS_XZ_Q).hypot((to.qz - from.qz) as f32 * POS_XZ_Q);
    }
    assert!(moved > 4.0, "stood still under a drawn bow: {moved:.2} m");
    assert_eq!(a.bot.combat().stats.engages, 0, "{}", a.explain());
    assert_eq!(a.bot.combat().mode(), Mode::Idle, "a dodge is not a fight");
    let mut arrows = stack(&content, "item.arrow_wood");
    arrows.count = 50;
    a.stage(RUSHER, |p| p.inv[HOTBAR_SLOTS] = arrows);
    a.archer = Some(true);
    let ran = a.until(20 * TICK_HZ, |a| a.bot.combat().stats.covers > 0);
    a.until(5 * TICK_HZ, |_| false);
    a.archer = None;
    println!("archer: {}", a.explain());
    assert!(ran, "never made for cover: {}", a.explain());
    assert!(a.bot.combat().stats.escapes > 0, "{}", a.explain());
    assert_eq!(a.heap_ops, 0);
}
