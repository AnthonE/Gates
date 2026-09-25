//! PLAYERS.md walls 1 and 4 against the real agent (`explorer::Survivor`).
//!
//! **Wall 1 — agent verbs are a subset of human verbs.** Every action the
//! agent layer can encode is one the human client encodes (read off both
//! sources, not a hand-kept list), every action it actually sent in a long
//! run decodes as an ordinary `ActionMsg`, every frame uses only buttons a
//! player's keys produce, and the observation encoder reads no `World`.
//!
//! **Wall 4 — determinism with agents in the loop**, through the shard's
//! own path: the agent plays a whole life cycle against `ShardCore` in
//! lockstep (shipped content, no sockets, a synthetic clock and the inline
//! scripted mind, so the run is a function of its inputs), a second shard
//! fed the same bytes stays hash-identical every tick, and the agent's own
//! frame loop never touches the allocator. The sim-level half, with the
//! counting allocator around `World::tick`, is
//! `crates/sim-core/tests/agent_input.rs`.
//!
//! The run is also the survival acceptance test: gather wood and stone,
//! craft a tool by name and move it to the belt, drink and eat, die, answer
//! the death screen in-game, and play on.

use protocol::{decode_action, decode_input, encode_input, ActionMsg, InputDatagram};
use server::core::{Lane, ShardCore};
use server::explorer::Survivor;
use server::mind::{Mind, MindConfig, Outcome, Scripted, Why};
use server::stats::ShardStats;
use server::view::ClientView;
use sim_core::input::{BTN_JUMP, BTN_PRIMARY, BTN_SPRINT};
use sim_core::limits::{HOTBAR_SLOTS, TICK_HZ};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::collections::BTreeSet;
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

const SEED: u64 = 20260731;
const ID: u32 = 256;
/// A second, scripted body the harness can use to deliver a blow — the
/// test's hazard, never an agent.
const PUPPET: u32 = 257;

fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The `encode_action_*` verbs a source file calls, read off the source.
fn verbs_in(dir: &std::path::Path, only: Option<&str>) -> BTreeSet<String> {
    fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }
    let mut files = Vec::new();
    walk(dir, &mut files);
    let mut verbs = BTreeSet::new();
    for file in files {
        if only.is_some_and(|name| !file.ends_with(name)) {
            continue;
        }
        let text = std::fs::read_to_string(&file).unwrap();
        // Test modules may encode anything; the shipped code is what binds.
        let text = text.split("#[cfg(test)]").next().unwrap_or_default();
        for piece in text.split("encode_action_").skip(1) {
            let verb: String = piece
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            if !verb.is_empty() {
                verbs.insert(verb);
            }
        }
    }
    verbs
}

#[test]
fn the_agent_encodes_only_verbs_the_human_client_encodes() {
    let human = verbs_in(&root().join("crates/client/src"), None);
    let agent = verbs_in(&root().join("crates/server/src"), Some("explorer.rs"));
    let expected: BTreeSet<String> = ["craft", "consume", "drink", "move", "respawn"]
        .into_iter()
        .map(String::from)
        .collect();
    assert_eq!(
        agent, expected,
        "the agent's verb set changed; update PLAYERS.md"
    );
    assert!(
        agent.is_subset(&human),
        "the agent encodes {:?}, which no human client call site encodes",
        agent.difference(&human).collect::<Vec<_>>()
    );
}

#[test]
fn the_observation_encoder_and_the_mind_read_no_world() {
    for file in ["explorer.rs", "mind.rs", "jev.rs", "external.rs"] {
        let text = std::fs::read_to_string(root().join("crates/server/src").join(file)).unwrap();
        let shipped = text.split("#[cfg(test)]").next().unwrap_or_default();
        for banned in ["sim_core::world", "World::", ".world.", "ShardCore"] {
            assert!(
                !shipped.contains(banned),
                "{file} names {banned}: the agent may only read what its client received"
            );
        }
    }
}

/// A level standing point near a stone node with a tree close by, so one
/// short walk sees both. Scene placement belongs to the test; the agent
/// gets the ordinary welcome and nothing else.
fn scene() -> (f32, f32) {
    use sim_core::terrain::{self, Occupant, ScatterTable};
    let haven = terrain::haven(SEED);
    let table = ScatterTable::alpha_default();
    for cz in 20..236 {
        for cx in 20..236 {
            let stone = terrain::scatter(SEED, &table, &haven, cx, cz);
            if stone.occupant != Occupant::StoneNode {
                continue;
            }
            let tree = (-3..=3).any(|dz| {
                (-3..=3).any(|dx| {
                    terrain::scatter(SEED, &table, &haven, cx + dx, cz + dz).occupant
                        == Occupant::Tree
                })
            });
            let at = (stone.x, stone.z - 6.0);
            let floor = terrain::ground(SEED, &haven, at.0, at.1);
            if tree && floor > 1.0 && (floor - stone.y).abs() < 0.5 {
                return at;
            }
        }
    }
    panic!("fixture seed has no stone node beside a tree");
}

/// A dry standing point with open sea inside the drink reach.
fn shore() -> (f32, f32) {
    use sim_core::terrain;
    let haven = terrain::haven(SEED);
    let reach = sim_core::survival::DRINK_REACH_M;
    let c = terrain::ISLAND_SIZE * 0.5;
    for (dx, dz) in [(1.0f32, 0.0f32), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
        for step in 0..600 {
            let (x, z) = (c + dx * step as f32 * 2.0, c + dz * step as f32 * 2.0);
            let wet = [
                (0.0, 0.0),
                (reach, 0.0),
                (-reach, 0.0),
                (0.0, reach),
                (0.0, -reach),
            ]
            .iter()
            .any(|(ox, oz)| terrain::height(SEED, x + ox, z + oz) < terrain::SEA_LEVEL);
            if wet && terrain::ground(SEED, &haven, x, z) > 0.3 {
                return (x, z);
            }
        }
    }
    panic!("fixture seed has no shore");
}

fn shard(content: &content::Content, at: (f32, f32), wildlife: bool) -> Box<ShardCore> {
    let t = server::net::bake_all(content).unwrap();
    let mut core = Box::new(ShardCore::new(SEED));
    // Every table a real shard installs. Wildlife only where a test says
    // so: elsewhere it would make an acceptance run a test of the animals.
    core.world.gather = t.gather;
    core.world.craft = t.craft;
    core.world.build = t.build;
    core.world.deploy = t.deploy;
    core.world.combat = t.combat;
    core.world.backpack = t.backpack;
    core.world.survival = t.survival;
    core.world.cook = t.cook;
    core.world.spawn_kit = t.spawn_kit;
    core.world.loot = t.loot;
    core.world.research = t.research;
    if wildlife {
        core.world.mob = t.mobs;
    }
    core.catalog = t.catalog;
    core.world.dev_spawn = Some(at);
    assert!(core.connect(0, ID));
    core
}

struct Harness {
    shard: Box<ShardCore>,
    replay: Box<ShardCore>,
    stats: ShardStats,
    view: ClientView,
    bot: Survivor,
    t0: Instant,
    tick: u32,
    heap_ops: usize,
    verbs: BTreeSet<&'static str>,
    buttons: u8,
    /// While set, the puppet stands on the bot swinging this weapon.
    puppet: Option<sim_core::gather::ItemStack>,
    spear: sim_core::gather::ItemStack,
}

impl Harness {
    fn new(wildlife: bool) -> Self {
        let content = content::Content::load_dir(&root().join("content")).unwrap();
        let at = scene();
        let spear = content.item_index("item.spear_wood").unwrap();
        let catalog = server::net::bake_all(&content).unwrap().catalog;
        let spear = sim_core::gather::ItemStack {
            item: spear,
            count: 1,
            cond: catalog.cond_max(spear as usize),
            skin: 0,
        };
        let mut bot =
            Survivor::new(Mind::inline(Scripted::default(), MindConfig::default()).unwrap());
        use server::botclient::BotDriver;
        bot.welcome(&protocol::Welcome {
            seed: SEED,
            player_id: ID,
            tick: 0,
            dev: true,
        });
        Self {
            shard: shard(&content, at, wildlife),
            replay: shard(&content, at, wildlife),
            stats: ShardStats::default(),
            view: ClientView::new(),
            bot,
            t0: Instant::now(),
            tick: 0,
            heap_ops: 0,
            verbs: BTreeSet::new(),
            buttons: 0,
            puppet: None,
            spear,
        }
    }

    /// Join the puppet in both shards; it swings only while `puppet` is set.
    fn with_puppet(&mut self) {
        for core in [&mut self.shard, &mut self.replay] {
            assert!(core.connect(1, PUPPET));
        }
    }

    /// Point-blank, like `alloc_zero.rs`'s duel: the puppet is stood on the
    /// bot each tick and holds primary, so the blow lands wherever the bot
    /// crawls. Scene staging, applied to both shards alike.
    fn puppet_step(&mut self, weapon: sim_core::gather::ItemStack) {
        let body = self
            .shard
            .world
            .players
            .iter()
            .find(|p| p.active && p.id == ID)
            .unwrap()
            .body;
        for core in [&mut self.shard, &mut self.replay] {
            // The join lands on the tick after `connect`.
            let Some(p) = core
                .world
                .players
                .iter_mut()
                .find(|p| p.active && p.id == PUPPET)
            else {
                return;
            };
            p.body = body;
            p.inv[0] = weapon;
        }
        let mut dg = InputDatagram::new(0, 0, sim_core::limits::INTERP_DELAY_TICKS);
        dg.push(sim_core::input::InputFrame {
            seq: self.tick as u16,
            buttons: BTN_PRIMARY,
            pitch: 128,
            ..Default::default()
        })
        .unwrap();
        let mut bytes = [0u8; sim_core::limits::DATAGRAM_BUDGET_BYTES];
        let len = encode_input(&dg, &mut bytes).unwrap();
        let decoded = decode_input(&bytes[..len]).unwrap();
        self.shard.push_input(1, &decoded);
        self.replay.push_input(1, &decoded);
    }

    /// Stage the same server-side scene change in both shards.
    fn stage(&mut self, f: impl Fn(&mut sim_core::world::Player)) {
        for core in [&mut self.shard, &mut self.replay] {
            let p = core
                .world
                .players
                .iter_mut()
                .find(|p| p.active && p.id == ID)
                .unwrap();
            f(p);
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
        self.buttons |= frame.buttons;
        assert!(usize::from(frame.sel) < HOTBAR_SLOTS);
        let (ack, ack_bits) = self.view.ack_fields();
        let mut dg = InputDatagram::new(ack, ack_bits, sim_core::limits::INTERP_DELAY_TICKS);
        dg.push(frame).unwrap();
        let mut bytes = [0u8; sim_core::limits::DATAGRAM_BUDGET_BYTES];
        let len = encode_input(&dg, &mut bytes).unwrap();
        let decoded = decode_input(&bytes[..len]).unwrap();
        self.shard.push_input(0, &decoded);
        self.replay.push_input(0, &decoded);
        if let Some(len) = action {
            let msg = decode_action(&act[..len]).expect("agent actions decode as player actions");
            self.verbs.insert(match msg {
                ActionMsg::Respawn { .. } => "respawn",
                ActionMsg::Craft { .. } => "craft",
                ActionMsg::Consume { .. } => "consume",
                ActionMsg::Drink => "drink",
                ActionMsg::Move { .. } => "move",
                other => panic!("the agent sent a verb outside its set: {other:?}"),
            });
            assert!(self.shard.wants_action(0) && self.replay.wants_action(0));
            self.shard.push_action(0, msg);
            let again = decode_action(&act[..len]).unwrap();
            self.replay.push_action(0, again);
        }
        if let Some(weapon) = self.puppet {
            self.puppet_step(weapon);
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
        self.replay.tick_bare(&self.stats, |_, _, _| true);
        assert_eq!(
            self.shard.world.state_hash(),
            self.replay.world.state_hash(),
            "the same agent bytes diverged two shards at tick {}",
            self.tick
        );
        self.tick += 1;
    }

    fn until(&mut self, ticks: u32, done: impl Fn(&Survivor) -> bool) -> bool {
        for _ in 0..ticks {
            if done(&self.bot) {
                return true;
            }
            self.step();
        }
        done(&self.bot)
    }

    fn explain(&self) -> String {
        format!(
            "tick {} stats {:?} mind {:?} goals {:?}",
            self.tick,
            self.bot.stats,
            self.bot.mind.stats,
            self.bot.history.iter().collect::<Vec<_>>()
        )
    }
}

#[test]
fn a_survivor_plays_a_whole_life_and_the_next_one_in_lockstep() {
    let mut h = Harness::new(false);

    // 1. Gather both resources, then craft a better tool by name.
    let crafted = h.until(12_000, |b| {
        b.stats.crafted >= 1 && b.gathered_of("Wood") > 0 && b.gathered_of("Stone") > 0
    });
    assert!(crafted, "no wood, stone and craft: {}", h.explain());
    let core = h.bot.core().unwrap();
    let belt: Vec<&[u8]> = core.inv[..HOTBAR_SLOTS]
        .iter()
        .filter(|s| s.count > 0)
        .map(|s| core.catalog.name(s.item as usize))
        .collect();
    assert!(
        belt.contains(&b"Stone Hatchet".as_slice()) || belt.contains(&b"Stone Pickaxe".as_slice()),
        "the crafted tool is not on the belt: {}",
        h.explain()
    );

    // 2. Thirsty and hungry at the shore: drink the sea, learn food.
    let (x, z) = shore();
    let haven = sim_core::terrain::haven(SEED);
    h.stage(|p| {
        p.body = sim_core::movement::Body::at(SEED, &haven, x, z);
        p.food = 100;
        p.water = 40;
    });
    let fed = h.until(9_000, |b| b.stats.drinks >= 1 && b.stats.eaten >= 1);
    assert!(fed, "no drink and meal: {}", h.explain());
    assert_ne!(h.bot.memory().food.feeds, 0, "a meal teaches what feeds");

    // 3. Go down, die, answer the death screen in-game, and play on. A
    //    starved body no longer serves: the survivor forages and eats its
    //    way out even at 1 hp. So a second body stands on it swinging a
    //    spear — the first blow lays it down, the next one kills.
    h.with_puppet();
    h.puppet = Some(h.spear);
    let downed = h.until(3_000, |b| b.core().is_some_and(|c| c.wounded || c.dead));
    assert!(downed, "the blow did not land: {}", h.explain());
    let reborn = h.until(3_000, |b| b.stats.deaths >= 1 && b.stats.respawns >= 1);
    h.puppet = None;
    assert!(reborn, "no in-game respawn: {}", h.explain());
    let died = h
        .bot
        .history
        .iter()
        .any(|r| r.outcome == Outcome::Interrupted(Why::Died));
    let decided = h.bot.mind.stats.decisions;
    let settled = h.bot.stats.goals_done + h.bot.stats.goals_failed;
    let played_on = h.until(3_000, |b| {
        b.mind.stats.decisions > decided && b.stats.goals_done + b.stats.goals_failed > settled
    });
    assert!(died || h.bot.stats.deaths > 0);
    assert!(played_on, "no goal after the respawn: {}", h.explain());
    let me = h.view.get(ID).copied().unwrap();
    assert!(!me.dead && !me.wounded, "a live body after the wake");

    // Wall 1, as sent: every verb in the set, and nothing else.
    let expected: BTreeSet<&str> = ["craft", "consume", "drink", "move", "respawn"]
        .into_iter()
        .collect();
    assert_eq!(h.verbs, expected, "{}", h.explain());
    assert_eq!(
        h.buttons & !(BTN_PRIMARY | BTN_SPRINT | BTN_JUMP),
        0,
        "only the buttons a player's keys produce"
    );
    // Wall 4, the agent's half: its frame loop never allocated.
    assert_eq!(
        h.heap_ops, 0,
        "the agent's frame loop touched the allocator"
    );
    println!("{}", h.explain());
    // Far fewer decisions than ticks: goals, not steps.
    assert!(
        h.bot.mind.stats.requests * u64::from(TICK_HZ) <= u64::from(h.tick),
        "more than one request a second: {}",
        h.explain()
    );
}

/// Half an hour of play, the shipped clock, no staging and no animals: the
/// survivor must keep itself fed and watered on its own. This is the gate
/// that would have caught an inland body that never learned which food
/// carries water (it carried 130 mushrooms and let water fall to 17%).
#[test]
fn half_an_hour_alone_keeps_food_and_water_up() {
    let mut h = Harness::new(false);
    let (mut food, mut water) = (u16::MAX, u16::MAX);
    for _ in 0..30 * 60 * TICK_HZ {
        h.step();
        let core = h.bot.core().unwrap();
        if core.max_food > 0 {
            food = food.min(core.food);
            water = water.min(core.water);
        }
    }
    let core = h.bot.core().unwrap();
    println!("minimum food {food}, water {water}; {}", h.explain());
    assert_eq!(h.bot.stats.deaths, 0, "{}", h.explain());
    // 30 %: the healthy run bottoms out near 39 % water and 59 % food; a
    // body that never learns which food carries water is near 21 % here.
    assert!(
        u32::from(food) * 10 >= u32::from(core.max_food) * 3,
        "food fell to {food}"
    );
    assert!(
        u32::from(water) * 10 >= u32::from(core.max_water) * 3,
        "water fell to {water}"
    );
    assert!(h.bot.gathered_of("Wood") > 0 && h.bot.gathered_of("Stone") > 0);
    assert!(
        h.bot.stats.crafted >= 2 && h.bot.stats.eaten > 0,
        "{}",
        h.explain()
    );
    assert!(h.bot.mind.stats.requests * u64::from(TICK_HZ) <= u64::from(h.tick));
    assert_eq!(h.heap_ops, 0);
}

/// Half an hour with the island's animals. Whether they kill is theirs to
/// decide; what is gated is that every death is answered in-game within a
/// few seconds and the body plays on afterwards.
#[test]
fn half_an_hour_with_wildlife_answers_every_death_in_game() {
    let mut h = Harness::new(true);
    let (mut dead_for, mut longest) = (0u32, 0u32);
    let mut decisions_at_wake = None;
    for _ in 0..30 * 60 * TICK_HZ {
        let respawns = h.bot.stats.respawns;
        h.step();
        let dead = h.bot.core().is_some_and(|c| c.dead);
        dead_for = if dead { dead_for + 1 } else { 0 };
        longest = longest.max(dead_for);
        if h.bot.stats.respawns > respawns {
            decisions_at_wake = Some(h.bot.mind.stats.decisions);
        }
    }
    println!("longest time dead {longest} ticks; {}", h.explain());
    let dead_now = h.bot.core().is_some_and(|c| c.dead);
    assert_eq!(
        h.bot.stats.respawns + u64::from(dead_now),
        h.bot.stats.deaths,
        "every death is answered: {}",
        h.explain()
    );
    assert!(
        longest <= 2 * TICK_HZ,
        "a death screen stood {longest} ticks"
    );
    if let Some(at_wake) = decisions_at_wake {
        assert!(
            h.bot.mind.stats.decisions > at_wake,
            "no decision after the last wake"
        );
    }
    assert!(h.bot.mind.stats.requests * u64::from(TICK_HZ) <= u64::from(h.tick));
    assert_eq!(h.heap_ops, 0);
}
