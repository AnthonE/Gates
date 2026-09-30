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
    haven: sim_core::terrain::Haven,
}

impl Arena {
    fn new(temperament: Temperament) -> Self {
        let content = content();
        let mut bot = Survivor::with(
            Mind::inline(Scripted::default(), MindConfig::default()).unwrap(),
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
            shard: shard(&content, scene(), false, ID),
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
