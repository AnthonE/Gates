//! PLAYERS.md wall 4 — **determinism holds with agents in the loop.** An
//! agent client is an input source like any other, and the sim must not
//! learn it exists: no clock, no I/O and no allocation enters the tick
//! because a player is a model.
//!
//! The fixture is a scripted agent-input source shaped like the real one
//! (`crates/server/src/explorer.rs`): each agent reads only what a client
//! is told about its own body — its position, its meters, its inventory,
//! its death screen — and answers with the agent layer's verbs as ordinary
//! commands: movement, look, swing, sprint, jump and hotbar frames, plus
//! `Craft`, `Consume`, `Drink` and `Respawn`, at most one action per tick.
//! `Command` has no field that could say "this came from a model", which
//! is the wall's structural half; this file is its measured half:
//!
//! 1. the same agents on the same seed produce the same hash every tick,
//!    twice (`test_replay`'s contract, with agents deciding live);
//! 2. the recorded command log, replayed into a fresh world, reproduces
//!    every one of those hashes — an agent's input is a WAL like any other;
//! 3. inside `World::tick`, after warmup, the heap is never touched
//!    (`test_alloc_zero`'s contract, with agents in the loop);
//! 4. every agent verb actually landed inside the counted window, so the
//!    first three are not satisfied by a fixture that did nothing.
//!
//! The server-side half — that these are the only verbs the real agent
//! sends, and that its observation is a pure function of received state —
//! is `crates/server/tests/agent_walls.rs`.

use sim_core::combat::CombatContent;
use sim_core::craft::CraftContent;
use sim_core::gather::{GatherContent, ItemStack};
use sim_core::input::{InputFrame, BTN_JUMP, BTN_PRIMARY, BTN_SPRINT};
use sim_core::limits::TICK_HZ;
use sim_core::survival::SurvivalContent;
use sim_core::world::{
    Command, Player, World, EV_CONSUMED, EV_CRAFT_DONE, EV_DEATH, EV_DRANK, EV_RESPAWN,
};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

// Per-thread, so parallel tests in this binary cannot count each other.
thread_local! { static ALLOCS: Cell<Option<u64>> = const { Cell::new(None) }; }

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

const SEED: u64 = 0xA6E7_7001;
const AGENTS: u32 = 4;
const WARMUP: u32 = 30;
/// Thirty seconds: the survival fixture empties water in 8 s and food in
/// 10 s, so the meters, the verbs that answer them and the clock's own
/// death all fall inside the window.
const TICKS: u32 = 30 * TICK_HZ;

fn world() -> World {
    let mut w = World::new(SEED);
    w.gather = GatherContent::probe_fixture();
    w.craft = CraftContent::probe_fixture();
    w.combat = CombatContent::probe_fixture();
    w.survival = SurvivalContent::probe_fixture();
    w
}

/// A standable point with sea inside the drink reach, scanned off the
/// heightfield (`alloc_zero.rs` says why a typed coast goes stale).
fn shoreline() -> (f32, f32) {
    let r = sim_core::survival::DRINK_REACH_M;
    let sea = sim_core::terrain::SEA_LEVEL;
    let h = |x: f32, z: f32| sim_core::terrain::height(SEED, x, z);
    let mut x = 0.0f32;
    while x < sim_core::terrain::ISLAND_SIZE {
        let mut z = 0.0f32;
        while z < sim_core::terrain::ISLAND_SIZE {
            if (sea..sim_core::terrain::BEACH_MAX_H).contains(&h(x, z))
                && (h(x + r, z) < sea
                    || h(x - r, z) < sea
                    || h(x, z + r) < sea
                    || h(x, z - r) < sea)
            {
                return (x, z);
            }
            z += 4.0;
        }
        x += 4.0;
    }
    panic!("this island has no coast — the generator changed under this gate");
}

/// Join the agents and stage the scene: fixture arrangements, the way the
/// wire tests grant server-side state. Agent 1 carries fixture food, which
/// is also the fixture recipe's input; agent 2 stands at the sea; agent 4
/// starts dry, so the clock kills it and its death screen must be answered.
fn stage(w: &mut World, haven: &sim_core::terrain::Haven, shore: (f32, f32)) {
    let joins: [Command; AGENTS as usize] =
        core::array::from_fn(|i| Command::Join { id: i as u32 + 1 });
    w.tick(&joins);
    w.players[0].inv[10] = ItemStack {
        item: 0,
        count: 90,
        cond: 0,
        skin: 0,
    };
    w.players[1].body = sim_core::movement::Body::at(SEED, haven, shore.0, shore.1);
    w.players[3].food = 0;
    w.players[3].water = 0;
}

/// What a client is told about its own body, and nothing else.
#[derive(Clone, Copy)]
struct Own {
    qx: i32,
    qz: i32,
    dead: bool,
    food: u16,
    water: u16,
    max_food: u16,
    max_water: u16,
    food_slot: Option<u8>,
    craftable: bool,
}

fn own(p: &Player, sc: &SurvivalContent, cc: &CraftContent) -> Own {
    let food_slot = p
        .inv
        .iter()
        .position(|s| s.count > 0 && sc.row(s.item).is_some())
        .map(|i| i as u8);
    let r = cc.recipes[0];
    let craftable = r.inputs[..usize::from(r.n_inputs)]
        .iter()
        .all(|&(item, need)| {
            p.inv
                .iter()
                .filter(|s| s.count > 0 && s.item == item)
                .map(|s| u32::from(s.count))
                .sum::<u32>()
                >= u32::from(need)
        });
    Own {
        qx: p.body.qx,
        qz: p.body.qz,
        dead: p.dead,
        food: p.food,
        water: p.water,
        max_food: sc.max_food,
        max_water: sc.max_water,
        food_slot,
        craftable,
    }
}

/// The scripted agent: a goal layer (answer the death screen, drink, eat,
/// craft) over a skill layer (walk, turn on a stall, swing, sprint, jump).
struct Agent {
    id: u32,
    yaw: u16,
    last: (i32, i32),
}

impl Agent {
    fn decide(&mut self, me: Own, t: u32, out: &mut [Command; 2 * AGENTS as usize], n: &mut usize) {
        let mut push = |c: Command| {
            out[*n] = c;
            *n += 1;
        };
        let still = InputFrame {
            seq: t as u16,
            pitch: 128,
            ..InputFrame::default()
        };
        if me.dead {
            push(Command::Respawn {
                id: self.id,
                on_bag: false,
            });
            push(Command::Input {
                id: self.id,
                frame: still,
                favour: 0,
            });
            return;
        }
        // One action at most, as the shard's action slot allows.
        let low = |v: u16, max: u16| u32::from(v) * 100 < u32::from(max) * 40;
        if low(me.water, me.max_water) && t.is_multiple_of(15) {
            push(Command::Drink { id: self.id });
        } else if low(me.food, me.max_food) && me.food_slot.is_some() {
            push(Command::Consume {
                id: self.id,
                slot: me.food_slot.unwrap_or(0),
            });
        } else if me.craftable && t.is_multiple_of(90) {
            push(Command::Craft {
                id: self.id,
                recipe: 0,
                count: 1,
                skin: 0,
            });
        }
        // A stalled second turns a quarter, like the controller's wander.
        if t.is_multiple_of(TICK_HZ) {
            if (me.qx, me.qz) == self.last {
                self.yaw = self.yaw.wrapping_add(1 << 14);
            }
            self.last = (me.qx, me.qz);
        }
        let mut buttons = 0;
        if (t / TICK_HZ) % 2 == 1 {
            buttons |= BTN_PRIMARY;
        }
        if t.is_multiple_of(7) {
            buttons |= BTN_SPRINT;
        }
        if t.is_multiple_of(97) {
            buttons |= BTN_JUMP;
        }
        push(Command::Input {
            id: self.id,
            frame: InputFrame {
                seq: t as u16,
                buttons,
                yaw: self.yaw,
                pitch: 128,
                move_z: if self.id == 2 { 0 } else { 127 },
                sel: 0,
                ..InputFrame::default()
            },
            favour: 0,
        });
    }
}

#[derive(Default, Debug, PartialEq, Eq)]
struct Tally {
    consumed: u32,
    drank: u32,
    crafted: u32,
    deaths: u32,
    respawns: u32,
}

struct Run {
    hashes: Vec<u64>,
    log: Vec<Vec<Command>>,
    tally: Tally,
    heap_ops: u64,
}

fn tally(w: &World, t: &mut Tally) {
    for ev in w.events.entries() {
        match ev.code {
            EV_CONSUMED => t.consumed += 1,
            EV_DRANK => t.drank += 1,
            EV_CRAFT_DONE => t.crafted += 1,
            EV_DEATH if ev.a <= AGENTS => t.deaths += 1,
            EV_RESPAWN if ev.a <= AGENTS => t.respawns += 1,
            _ => {}
        }
    }
}

/// Agents decide live, between ticks, from their own records; only the
/// tick itself is counted.
fn run_agents(haven: &sim_core::terrain::Haven, shore: (f32, f32)) -> Run {
    let mut w = world();
    stage(&mut w, haven, shore);
    let mut agents: Vec<Agent> = (1..=AGENTS)
        .map(|id| Agent {
            id,
            yaw: (id as u16).wrapping_mul(16_411),
            last: (0, 0),
        })
        .collect();
    let mut run = Run {
        hashes: Vec::with_capacity(TICKS as usize),
        log: Vec::with_capacity(TICKS as usize),
        tally: Tally::default(),
        heap_ops: 0,
    };
    let sc = w.survival;
    let cc = w.craft;
    for t in 0..TICKS {
        let mut cmds = [Command::Leave { id: 0 }; 2 * AGENTS as usize];
        let mut n = 0;
        for agent in agents.iter_mut() {
            let me = own(&w.players[agent.id as usize - 1], &sc, &cc);
            agent.decide(me, t, &mut cmds, &mut n);
        }
        let counted = t >= WARMUP;
        if counted {
            ALLOCS.with(|c| c.set(Some(0)));
        }
        w.tick(&cmds[..n]);
        if counted {
            run.heap_ops += ALLOCS.with(|c| c.replace(None).unwrap_or(0));
            tally(&w, &mut run.tally);
        }
        run.hashes.push(w.state_hash());
        run.log.push(cmds[..n].to_vec());
    }
    run
}

#[test]
fn agent_input_replays_exactly_and_never_allocates_in_the_tick() {
    let haven = sim_core::terrain::haven(SEED);
    let shore = shoreline();
    let a = run_agents(&haven, shore);
    assert_eq!(
        a.heap_ops, 0,
        "agent-origin commands reached the allocator inside World::tick"
    );
    // Every verb landed inside the counted window, or the two contracts
    // above were proven over a fixture that did nothing.
    assert!(a.tally.consumed > 0, "no eat landed: {:?}", a.tally);
    assert!(a.tally.drank > 0, "no drink landed: {:?}", a.tally);
    assert!(a.tally.crafted > 0, "no craft finished: {:?}", a.tally);
    assert!(
        a.tally.deaths > 0 && a.tally.respawns > 0,
        "no death screen was answered: {:?}",
        a.tally
    );

    // 1. The same agents, deciding live again: every stamp agrees.
    let b = run_agents(&haven, shore);
    assert_eq!(a.hashes, b.hashes, "live agents diverged from themselves");
    assert_eq!(a.tally, b.tally);

    // 2. The recorded log is a WAL: replayed with no agent at all, it
    //    rebuilds the same world tick for tick.
    let mut w = world();
    stage(&mut w, &haven, shore);
    for (t, cmds) in a.log.iter().enumerate() {
        w.tick(cmds);
        assert_eq!(
            w.state_hash(),
            a.hashes[t],
            "replaying the agents' log diverged at tick {t}"
        );
    }
}

#[test]
fn an_agent_frame_is_an_ordinary_frame() {
    // The fixture's frames use only the buttons a human's keys produce for
    // these verbs, and a hotbar slot a human can select: the sim receives
    // nothing a person could not send.
    let haven = sim_core::terrain::haven(SEED);
    let run = run_agents(&haven, shoreline());
    for cmds in &run.log {
        for c in cmds {
            match c {
                Command::Input { frame, favour, .. } => {
                    assert_eq!(frame.buttons & !(BTN_PRIMARY | BTN_SPRINT | BTN_JUMP), 0);
                    assert!(usize::from(frame.sel) < sim_core::limits::HOTBAR_SLOTS);
                    assert_eq!(*favour, 0);
                }
                Command::Craft { .. }
                | Command::Consume { .. }
                | Command::Drink { .. }
                | Command::Respawn { .. } => {}
                other => panic!("the agent fixture sent a non-agent verb: {other:?}"),
            }
        }
    }
}
