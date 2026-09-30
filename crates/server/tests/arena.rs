//! The arena (lane A): the agent against a scripted attacker, in lockstep
//! against `ShardCore` with the shipped content, the inline scripted mind
//! and a synthetic clock, so a run is a function of its inputs.
//!
//! The attacker is test code and may read the world: a rusher that walks
//! straight at the agent with perfect yaw and holds primary once in reach.
//! What is gated is that the agent fights back — it lands blows instead of
//! only backing away — and that it does so as a player would: its frame
//! loop never allocates, and its view never turns faster than its hands'
//! preset allows.

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
        }
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
        let (view, bot) = (&mut self.view, &mut self.bot);
        self.shard.tick_bare(&self.stats, |lane, slot, bytes| {
            if slot != 0 {
                return true;
            }
            match lane {
                Lane::Snapshot => {
                    view.apply(bytes).unwrap();
                }
                Lane::Event => bot.event(bytes).unwrap(),
            }
            true
        });
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
    let haven = sim_core::terrain::haven(SEED);
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
    a.stage(RUSHER, |p| {
        p.body = Body::at(SEED, &haven, x, z);
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
