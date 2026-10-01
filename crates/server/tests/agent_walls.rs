//! PLAYERS.md walls 1 and 4 against the real agent (`explorer::Survivor`).
//!
//! **Wall 1 — agent verbs are a subset of human verbs.** Every action the
//! agent layer can encode is one the human client encodes (read off both
//! sources, not a hand-kept list), every action it actually sent in a long
//! run decodes as an ordinary `ActionMsg`, every frame uses only buttons a
//! player's keys produce and turns the view no faster than the hands'
//! preset allows, and the observation encoder reads no `World`.
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

mod common;

use common::{root, scene, SEED};
use protocol::{decode_action, decode_input, encode_input, ActionMsg, InputDatagram};
use server::core::{Lane, ShardCore};
use server::explorer::Survivor;
use server::mind::{Goal, Mind, MindConfig, Outcome, Scripted, Why};
use server::stats::ShardStats;
use server::view::ClientView;
use sim_core::input::{
    BTN_AIM, BTN_ASSIST, BTN_CROUCH, BTN_JUMP, BTN_LIGHT, BTN_PRIMARY, BTN_SPRINT,
};
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

const ID: u32 = 256;
/// A second, scripted body the harness can use to deliver a blow — the
/// test's hazard, never an agent.
const PUPPET: u32 = 257;

/// The verbs the agent may send, in `encode_action_*` spelling. The one
/// list to extend when a lane gives the agent a new verb: the source grep,
/// the lockstep run and PLAYERS.md all answer to it.
const EXPECTED_VERBS: [&str; 14] = [
    "craft",
    "consume",
    "drink",
    "move",
    "respawn",
    "reload",
    "loot",
    "pickup",
    "deploy",
    "place",
    "upgrade",
    "use",
    "container",
    "feed",
];

/// The verbs a lockstep life of gathering, crafting, eating, drinking and
/// dying sends every time. The rest of the set comes after a won fight or
/// with a gun in hand, which the arena exercises (`tests/arena.rs`: the
/// bow hunt loots and picks up, the revolver reloads), or from the bag
/// lifecycle's and the builder's runs below.
const LIFE_VERBS: [&str; 5] = ["craft", "consume", "drink", "move", "respawn"];

/// The buttons the agent presses today. Always within what the human
/// client sends (`human_buttons`); widened here when a lane presses more.
const EXPECTED_BUTTONS: u8 = BTN_PRIMARY | BTN_SPRINT | BTN_JUMP | BTN_AIM | BTN_CROUCH;

fn expected_verbs() -> BTreeSet<String> {
    EXPECTED_VERBS.into_iter().map(String::from).collect()
}

/// Every `.rs` file under `dir`, recursively.
fn rs_files(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
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
    files.sort();
    files
}

/// The agent layer's source: the orchestrator and everything under
/// `src/agent/`, so a new skill file cannot escape the walls.
fn agent_sources() -> Vec<std::path::PathBuf> {
    let src = root().join("crates/server/src");
    let mut files = vec![src.join("explorer.rs")];
    files.extend(rs_files(&src.join("agent")));
    files
}

/// A file's shipped text: every `#[cfg(test)]` item cut out, since test
/// code may name anything. Only the gated items go, not the rest of the
/// file after the first one, so a test-only const near the top cannot hide
/// the code below it from the walls.
fn shipped(file: &std::path::Path) -> String {
    strip_test_items(&std::fs::read_to_string(file).unwrap())
}

fn strip_test_items(text: &str) -> String {
    const GATE: &str = "#[cfg(test)]";
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(GATE) {
        out.push_str(&rest[..at]);
        rest = &rest[at + GATE.len()..];
        rest = &rest[item_len(rest)..];
    }
    out.push_str(rest);
    out
}

/// The length of the item at the start of `text`: up to its `;` or the
/// brace that closes its body, skipping strings, chars and comments so a
/// brace inside one does not end the item early.
fn item_len(text: &str) -> usize {
    let bytes = text.as_bytes();
    let mut depth = 0usize;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                i += text[i..].find('\n').unwrap_or(text.len() - i);
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i += text[i..].find("*/").map_or(text.len() - i, |e| e + 1);
            }
            b'r' if matches!(bytes.get(i + 1), Some(b'"' | b'#'))
                && !bytes[i.saturating_sub(1)].is_ascii_alphanumeric() =>
            {
                let hashes = bytes[i + 1..].iter().take_while(|&&b| b == b'#').count();
                let close = format!("\"{}", "#".repeat(hashes));
                let open = i + 1 + hashes + 1;
                i = open + text[open..].find(&close).unwrap_or(text.len() - open) + close.len() - 1;
            }
            b'"' => {
                i += 1;
                while i < bytes.len() && bytes[i] != b'"' {
                    i += if bytes[i] == b'\\' { 2 } else { 1 };
                }
            }
            b'\'' if bytes.get(i + 2) == Some(&b'\'') => i += 2,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return i + 1;
                }
            }
            b';' if depth == 0 => return i + 1,
            _ => {}
        }
        i += 1;
    }
    text.len()
}

/// The `encode_action_*` verbs these files call, read off the source.
fn verbs_in(files: &[std::path::PathBuf]) -> BTreeSet<String> {
    let mut verbs = BTreeSet::new();
    for file in files {
        let text = shipped(file);
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

/// A decoded action's verb, in `encode_action_*` spelling. Exhaustive, so
/// a new wire verb is a compile error here rather than a silent pass.
fn verb_of(msg: &ActionMsg) -> &'static str {
    match msg {
        ActionMsg::Assist { .. } => "assist",
        ActionMsg::Craft { .. } => "craft",
        ActionMsg::Reskin { .. } => "reskin",
        ActionMsg::SkinsRefresh => "skins_refresh",
        ActionMsg::CraftCancel { .. } => "cancel",
        ActionMsg::Research { .. } => "research",
        ActionMsg::Unlock { .. } => "unlock",
        ActionMsg::Consume { .. } => "consume",
        ActionMsg::Drink => "drink",
        ActionMsg::Move { .. } => "move",
        ActionMsg::Container { .. } => "container",
        ActionMsg::Respawn { .. } => "respawn",
        ActionMsg::Reload => "reload",
        ActionMsg::Loot => "loot",
        ActionMsg::Pickup => "pickup",
        ActionMsg::Place { .. } => "place",
        ActionMsg::Deploy { .. } => "deploy",
        ActionMsg::Feed { .. } => "feed",
        ActionMsg::Use { .. } => "use",
        ActionMsg::Repair { .. } => "repair",
        ActionMsg::Throw { .. } => "throw",
        ActionMsg::Access { .. } => "access",
        ActionMsg::Demolish { .. } => "demolish",
        ActionMsg::Rotate { .. } => "rotate",
        ActionMsg::Upgrade { .. } => "upgrade",
    }
}

/// The input buttons the human client sends: every `BTN_*` it names,
/// read off its source and mapped to the wire bits.
fn human_buttons() -> u8 {
    let mut mask = 0;
    for file in rs_files(&root().join("crates/client/src")) {
        let text = shipped(&file);
        let mut rest = text.as_str();
        while let Some(at) = rest.find("BTN_") {
            let before = rest[..at].chars().next_back();
            let name: String = rest[at..]
                .chars()
                .take_while(|c| c.is_ascii_uppercase() || *c == '_')
                .collect();
            rest = &rest[at + 4..];
            // Part of a longer identifier, or prose about `BTN_*` as a set.
            if before.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_') || name == "BTN_" {
                continue;
            }
            mask |= match name.as_str() {
                "BTN_SPRINT" => BTN_SPRINT,
                "BTN_CROUCH" => BTN_CROUCH,
                "BTN_PRIMARY" => BTN_PRIMARY,
                "BTN_JUMP" => BTN_JUMP,
                "BTN_LIGHT" => BTN_LIGHT,
                "BTN_ASSIST" => BTN_ASSIST,
                "BTN_AIM" => BTN_AIM,
                other => panic!("{}: map {other} to its sim_core::input bit", file.display()),
            };
        }
    }
    mask
}

#[test]
fn the_agent_encodes_only_verbs_the_human_client_encodes() {
    let human = verbs_in(&rs_files(&root().join("crates/client/src")));
    let agent = verbs_in(&agent_sources());
    assert_eq!(
        agent,
        expected_verbs(),
        "the agent's verb set changed; update EXPECTED_VERBS and PLAYERS.md"
    );
    assert!(
        agent.is_subset(&human),
        "the agent encodes {:?}, which no human client call site encodes",
        agent.difference(&human).collect::<Vec<_>>()
    );
}

#[test]
fn the_agent_presses_only_buttons_the_human_client_sends() {
    let human = human_buttons();
    assert_ne!(human & BTN_PRIMARY, 0, "the client source was not read");
    assert_eq!(
        EXPECTED_BUTTONS & !human,
        0,
        "the agent may press a button no human client sends"
    );
}

/// The walls read `shipped` text, so a cut that eats real code blinds them
/// without failing. jev.rs gates a const near the top, well above its test
/// module: its request code must survive, and no test code may.
#[test]
fn shipped_text_cuts_only_the_test_items() {
    let src = root().join("crates/server/src");
    let jev = shipped(&src.join("jev.rs"));
    for kept in [
        "pub struct Jev",
        "impl Jev",
        "INSTRUCTIONS",
        "MAX_RESPONSE_BYTES",
    ] {
        assert!(jev.contains(kept), "shipped jev.rs lost {kept}");
    }
    assert!(!jev.contains("REQUEST_BYTES_MAX"), "a gated const survived");
    let mut files = agent_sources();
    files.extend(["mind.rs", "jev.rs", "external.rs"].map(|f| src.join(f)));
    for file in files {
        let text = shipped(&file);
        for test_only in ["#[cfg(test)]", "#[test]", "mod tests"] {
            assert!(
                !text.contains(test_only),
                "{} keeps {test_only}",
                file.display()
            );
        }
    }
    let strip = strip_test_items(
        "a\n#[cfg(test)]\nconst X: char = '}';\nb\n#[cfg(test)]\nmod t { fn f() { let _ = \"}\"; } }\nc",
    );
    assert_eq!(strip.split_whitespace().collect::<String>(), "abc");
}

#[test]
fn the_observation_encoder_and_the_mind_read_no_world() {
    let src = root().join("crates/server/src");
    let mut files = agent_sources();
    files.extend(["mind.rs", "jev.rs", "external.rs"].map(|f| src.join(f)));
    for file in files {
        let shipped = shipped(&file);
        for banned in ["sim_core::world", "World::", ".world.", "ShardCore"] {
            assert!(
                !shipped.contains(banned),
                "{} names {banned}: the agent may only read what its client received",
                file.display()
            );
        }
    }
}

/// A dry standing point with open sea inside the drink reach.
fn shore() -> (f32, f32) {
    use sim_core::terrain;
    let haven = terrain::haven(SEED);
    let reach = sim_core::survival::DRINK_REACH_M;
    let c = terrain::ISLAND_SIZE * 0.5;
    for (dx, dz) in [(1.0f32, 0.0f32), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
        for step in 0..(c as usize) {
            let (x, z) = (c + dx * step as f32, c + dz * step as f32);
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

/// The shared lockstep shard (`common::shard`), the agent joined as [`ID`].
/// Wildlife only where a test says so: elsewhere it would make an
/// acceptance run a test of the animals.
fn shard(content: &content::Content, at: (f32, f32), wildlife: bool) -> Box<ShardCore> {
    common::shard(content, at, wildlife, ID)
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
    /// Which body is the puppet, and on which slot.
    puppet_id: u32,
    puppet_slot: usize,
    spear: sim_core::gather::ItemStack,
    /// The last yaw sent in this life, and the largest turn between two
    /// frames: the hands wall.
    last_yaw: Option<u16>,
    life: u64,
    max_turn: u16,
    /// What the human UI makes a player hold for the verbs that need it:
    /// the building plan to place, the hammer to upgrade or repair.
    plan: u16,
    hammer: u16,
    /// Held-item checks passed at send time.
    held_checks: u32,
    /// Door uses that were the human client's `E` pick when sent.
    use_checks: u32,
    /// Box opens and cupboard feeds that were the `E` pick when sent, and
    /// box moves sent with that box's panel open.
    pick_checks: u32,
    panel_checks: u32,
}

impl Harness {
    fn new(wildlife: bool) -> Self {
        Self::at(wildlife, scene())
    }

    fn at(wildlife: bool, at: (f32, f32)) -> Self {
        let content = content::Content::load_dir(&root().join("content")).unwrap();
        let spear = content.item_index("item.spear_wood").unwrap();
        let catalog = server::net::bake_all(&content).unwrap().catalog;
        let spear = sim_core::gather::ItemStack {
            item: spear,
            count: 1,
            cond: catalog.cond_max(spear as usize),
            skin: 0,
        };
        let plan = content.item_index("item.building_plan").unwrap();
        let hammer = content.item_index("item.hammer").unwrap();
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
            puppet_id: PUPPET,
            puppet_slot: 1,
            spear,
            last_yaw: None,
            life: 0,
            max_turn: 0,
            plan,
            hammer,
            held_checks: 0,
            use_checks: 0,
            pick_checks: 0,
            panel_checks: 0,
        }
    }

    /// The item the human client makes a player hold to send this verb,
    /// if any: the deployable itself, the plan, the hammer.
    fn required_item(&self, msg: &ActionMsg) -> Option<u16> {
        match *msg {
            ActionMsg::Deploy { row, .. } => {
                Some(self.shard.world.deploy.defs[usize::from(row)].item)
            }
            ActionMsg::Place { .. } => Some(self.plan),
            ActionMsg::Upgrade { .. } | ActionMsg::Repair { .. } => Some(self.hammer),
            _ => None,
        }
    }

    /// The bot's body in the live shard.
    fn me(&self) -> &sim_core::world::Player {
        self.shard
            .world
            .players
            .iter()
            .find(|p| p.active && p.id == ID)
            .unwrap()
    }

    /// Stage the puppet's body in both shards (scene staging, never the
    /// agent's).
    fn stage_puppet(&mut self, f: impl Fn(&mut sim_core::world::Player)) {
        for core in [&mut self.shard, &mut self.replay] {
            if let Some(p) = core
                .world
                .players
                .iter_mut()
                .find(|p| p.active && p.id == PUPPET)
            {
                f(p);
            }
        }
    }

    /// Join the puppet in both shards; it swings only while `puppet` is set.
    fn with_puppet(&mut self) {
        for core in [&mut self.shard, &mut self.replay] {
            assert!(core.connect(self.puppet_slot, self.puppet_id));
        }
    }

    /// Point-blank, like `alloc_zero.rs`'s duel: the puppet is stood on the
    /// bot each tick and holds primary, so the blow lands wherever the bot
    /// crawls. While it is staged it is also kept whole: the bot fights
    /// back now, and the hazard must outlast it. Scene staging, applied to
    /// both shards alike.
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
                .find(|p| p.active && p.id == self.puppet_id)
            else {
                return;
            };
            p.body = body;
            p.inv[0] = weapon;
            p.hp = p.hp_max;
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
        self.shard.push_input(self.puppet_slot, &decoded);
        self.replay.push_input(self.puppet_slot, &decoded);
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
        // Wall 1, the hands' half: the view turns no faster than the
        // preset's hands. A new life starts from a fresh view, so its first
        // frame is not a turn.
        if self.life != self.bot.stats.respawns {
            self.life = self.bot.stats.respawns;
            self.last_yaw = None;
        }
        if let Some(last) = self.last_yaw {
            let turn = (frame.yaw.wrapping_sub(last) as i16).unsigned_abs();
            self.max_turn = self.max_turn.max(turn);
            let cap = self.bot.hands().skill().max_turn();
            assert!(
                turn <= cap,
                "the view turned {turn} in one frame at tick {}, over the cap {cap}",
                self.tick
            );
        }
        self.last_yaw = Some(frame.yaw);
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
            let verb = verb_of(&msg);
            assert!(
                EXPECTED_VERBS.contains(&verb),
                "the agent sent a verb outside its set: {msg:?}"
            );
            self.verbs.insert(verb);
            // Wall 1, the hands' other half: a verb the human UI sends only
            // with something in hand goes with it in hand, in the world's
            // own selected slot at the moment it is sent.
            if let Some(need) = self.required_item(&msg) {
                let held = sim_core::combat::held_item(self.me());
                assert_eq!(
                    held, need,
                    "the agent sent {msg:?} holding item {held}, not {need}, at tick {}",
                    self.tick
                );
                self.held_checks += 1;
            }
            // Wall 1, the eyes' half: a use, a box opened, a cupboard fed
            // goes to what the human client's `E` would pick from this
            // body, facing this frame's bearing; a move into or out of a
            // box goes with that box's panel open.
            use client::ui::interact::Verb;
            use sim_core::inventory::CONT_BOX;
            match msg {
                ActionMsg::Use { cx, cz, level, loc } => {
                    let pick = self.e_pick(frame.yaw);
                    assert_eq!(
                        (pick.verb, pick.cx, pick.cz, pick.level, pick.loc),
                        (Verb::Door, cx, cz, level, loc),
                        "the agent used a door `E` would not pick, at tick {}",
                        self.tick
                    );
                    self.use_checks += 1;
                }
                ActionMsg::Container {
                    kind: CONT_BOX,
                    cont,
                } => {
                    let pick = self.e_pick(frame.yaw);
                    assert_eq!(
                        (pick.verb, pick.handle),
                        (Verb::Box, cont),
                        "the agent opened a box `E` would not pick, at tick {}",
                        self.tick
                    );
                    self.pick_checks += 1;
                }
                ActionMsg::Feed { cx, cz, level } => {
                    let pick = self.e_pick(frame.yaw);
                    assert_eq!(
                        (pick.verb, pick.cx, pick.cz, pick.level),
                        (Verb::Hearth, cx, cz, level),
                        "the agent fed a cupboard `E` would not pick, at tick {}",
                        self.tick
                    );
                    self.pick_checks += 1;
                }
                ActionMsg::Move {
                    cont,
                    from_kind,
                    to_kind,
                    ..
                } if from_kind == CONT_BOX || to_kind == CONT_BOX => {
                    let core = self.bot.core().unwrap();
                    assert_eq!(
                        (core.cont_kind, core.cont_handle),
                        (CONT_BOX, cont),
                        "the agent moved a box's stack without its panel open, at tick {}",
                        self.tick
                    );
                    self.panel_checks += 1;
                }
                _ => {}
            }
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

    /// What the human client's `E` would pick from the bot's body, facing
    /// wire yaw `yaw`, over what its client holds.
    fn e_pick(&self, yaw: u16) -> client::ui::interact::Pick {
        use client::ui::interact::{resolve, Aim};
        let core = self.bot.core().unwrap();
        let me = self.view.get(ID).copied().unwrap();
        let q = sim_core::movement::POS_XZ_Q;
        let (fx, fz) = sim_core::yaw_dir(yaw);
        resolve(
            Aim::new(me.qx as f32 * q, me.qz as f32 * q, fx, fz),
            core.deploys.entries(),
            &core.deploy_defs,
            core.deploy_defs_have,
            core.bags.entries(),
        )
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
            "tick {} widest turn {} stats {:?} route {:?} home {:?} mind {:?} goals {:?}",
            self.tick,
            self.max_turn,
            self.bot.stats,
            self.bot.route().stats,
            self.bot.home().stats,
            self.bot.mind.stats,
            self.bot.history.iter().collect::<Vec<_>>()
        )
    }
}

#[test]
fn a_survivor_plays_a_whole_life_and_the_next_one_in_lockstep() {
    let mut h = Harness::new(false);

    // 1. Gather both resources, then craft a better tool by name. The
    //    craft goal ends once the tool is on the belt, whether it landed
    //    there or had to be moved.
    let crafted = h.until(12_000, |b| {
        b.stats.crafted >= 1
            && b.gathered_of("Wood") > 0
            && b.gathered_of("Stone") > 0
            && !matches!(b.goal(), Some(Goal::Craft(_)))
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

    // 3. A blow it survives, mid-goal: the fight reflex takes the frames —
    //    it turns on the body that struck it and fights back — the goal
    //    waits and is then handed back or ended, all under the counting
    //    allocator. (Until lane A the reflex only backed away.)
    let busy = h.until(3_000, |b| b.goal().is_some() && !b.combat().engaged());
    assert!(busy, "no goal to pause: {}", h.explain());
    let settled = |b: &Survivor| b.stats.goals_resumed + b.stats.goals_interrupted;
    let before = settled(&h.bot);
    let hurts = h.bot.stats.hurts;
    h.stage(|p| p.hp = p.hp_max);
    h.with_puppet();
    h.puppet = Some(h.spear);
    let hit = h.until(600, |b| b.stats.hurts > hurts);
    h.puppet = None;
    assert!(hit, "the blow did not land: {}", h.explain());
    assert!(!h.bot.core().unwrap().wounded, "one blow must not down it");
    let over = h.until(60 * TICK_HZ, |b| {
        !b.combat().engaged() && settled(b) > before
    });
    assert!(
        over,
        "the fight never ended, or the paused goal was neither resumed nor ended: {}",
        h.explain()
    );
    assert!(
        h.bot.stats.landed > 0,
        "it never struck back: {}",
        h.explain()
    );

    // 4. Go down, die, answer the death screen in-game, and play on. A
    //    starved body no longer serves: the survivor forages and eats its
    //    way out even at 1 hp. So a second body stands on it swinging a
    //    spear until it lays it down, and then kills it. A fresh one: the
    //    first may not have survived the answer to its blow.
    h.puppet_id = PUPPET + 1;
    h.puppet_slot = 2;
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

    // Wall 1, as sent: every verb a life needs, and nothing outside the
    // set (the arena and the bag and builder runs below send the rest).
    let expected: BTreeSet<&str> = EXPECTED_VERBS.into_iter().collect();
    let life: BTreeSet<&str> = LIFE_VERBS.into_iter().collect();
    assert!(
        h.verbs.is_subset(&expected) && life.is_subset(&h.verbs),
        "{:?}: {}",
        h.verbs,
        h.explain()
    );
    assert_eq!(
        h.buttons & !(EXPECTED_BUTTONS & human_buttons()),
        0,
        "only the buttons a player's keys produce"
    );
    // Wall 4, the agent's half: its frame loop never allocated.
    assert_eq!(
        h.heap_ops, 0,
        "the agent's frame loop touched the allocator"
    );
    // Wall 1's turn cap held every frame (`step`), and the hands did turn.
    assert!(h.max_turn > 0, "{}", h.explain());
    // The wiki read the shipped rules and the wire's catalog agreed.
    let wiki = h.bot.wiki();
    assert!(wiki.ready() && wiki.unknown == 0 && wiki.disagreements == 0);
    println!("{}", h.explain());
    // Far fewer decisions than ticks: goals, not steps.
    assert!(
        h.bot.mind.stats.requests * u64::from(TICK_HZ) <= u64::from(h.tick),
        "more than one request a second: {}",
        h.explain()
    );
}

/// Somewhere to come back to, in lockstep: with stone tools and cloth in
/// the pack the playbook crafts a sleeping bag and puts it down on bare
/// ground, holding it the way the human client makes a player; killed a
/// walk away, it wakes on that bag, walks back and loots its backpack.
#[test]
fn a_survivor_wakes_on_its_bag_and_walks_back_for_its_backpack() {
    let mut h = Harness::new(false);
    let content = content::Content::load_dir(&root().join("content")).unwrap();
    let catalog = server::net::bake_all(&content).unwrap().catalog;
    let stack = |id: &str, count: u16| {
        let item = content.item_index(id).unwrap();
        sim_core::gather::ItemStack {
            item,
            count,
            cond: catalog.cond_max(item as usize),
            skin: 0,
        }
    };
    let (hatchet, pickaxe) = (
        stack("item.hatchet_stone", 1),
        stack("item.pickaxe_stone", 1),
    );
    let cloth = stack("item.cloth", 40);
    // The join lands on the tick after `connect`.
    h.until(5, |_| false);
    h.stage(|p| {
        p.inv[1] = hatchet;
        p.inv[2] = pickaxe;
        p.inv[12] = cloth;
    });

    // 1. Craft the bag and put it down.
    let placed = h.until(9_000, |b| b.home().stats.bags_placed >= 1);
    assert!(placed, "no bag down: {}", h.explain());
    let bags: Vec<_> = h
        .shard
        .world
        .deploys
        .entries()
        .iter()
        .filter(|d| {
            d.owner == ID
                && h.shard.world.deploy.defs[usize::from(d.row)].arch == sim_core::deploy::ARCH_BAG
        })
        .copied()
        .collect();
    assert_eq!(bags.len(), 1, "{}", h.explain());
    assert!(
        h.held_checks >= 1,
        "the deploy went out with the bag in hand"
    );
    let (bx, bz) = sim_core::deploy::cell_center(bags[0].cx, bags[0].cz);

    // 2. A walk away from it (well outside the alarm around home, inside
    //    the recovery range), a puppet lays it down and kills it.
    let haven = sim_core::terrain::haven(SEED);
    let away = [
        (70.0f32, 0.0f32),
        (-70.0, 0.0),
        (0.0, 70.0),
        (0.0, -70.0),
        (50.0, 50.0),
    ]
    .into_iter()
    .map(|(dx, dz)| (bx + dx, bz + dz))
    .find(|&(x, z)| sim_core::terrain::ground(SEED, &haven, x, z) > 1.0)
    .expect("dry ground a walk from the bag");
    h.stage(|p| p.body = sim_core::movement::Body::at(SEED, &haven, away.0, away.1));
    h.with_puppet();
    h.puppet = Some(h.spear);
    let died = h.until(3_000, |b| b.stats.deaths >= 1);
    h.puppet = None;
    assert!(died, "no death: {}", h.explain());
    // The killer leaves: nobody dangerous stands between it and its pack.
    let far = sim_core::movement::Body::at(SEED, &haven, bx + 400.0, bz + 400.0);
    h.stage_puppet(|p| p.body = far);
    let woke = h.until(3_000, |b| b.stats.respawns >= 1);
    assert!(woke, "no wake: {}", h.explain());
    assert_eq!(h.bot.home().stats.wakes_on_bag, 1, "{}", h.explain());
    h.step();
    let me = h.me().body;
    let (x, z) = (
        me.qx as f32 * sim_core::movement::POS_XZ_Q,
        me.qz as f32 * sim_core::movement::POS_XZ_Q,
    );
    assert!(
        (x - bx).hypot(z - bz) < 4.0,
        "woke at ({x}, {z}), not at the bag ({bx}, {bz})"
    );
    assert_ne!(
        h.bot.core().unwrap().own_bag,
        0,
        "the death backpack is tagged"
    );

    // 3. Back to the backpack, and everything in it comes home.
    let back = h.until(4_000, |b| b.home().stats.recovered >= 1);
    assert!(back, "no recovery: {}", h.explain());
    let core = h.bot.core().unwrap();
    assert_eq!(core.own_bag, 0, "the backpack was emptied");
    assert!(
        core.inv
            .iter()
            .any(|s| s.count > 0 && s.item == hatchet.item),
        "the hatchet came back: {}",
        h.explain()
    );
    for verb in ["craft", "deploy", "respawn", "loot"] {
        assert!(h.verbs.contains(verb), "{verb} never sent: {:?}", h.verbs);
    }
    let expected: BTreeSet<&str> = EXPECTED_VERBS.into_iter().collect();
    assert!(h.verbs.is_subset(&expected));
    assert_eq!(
        h.heap_ops, 0,
        "the agent's frame loop touched the allocator"
    );
    println!("{}", h.explain());
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
    // With stone tools in hand the playbook forages the cloth for a
    // sleeping bag, crafts it and puts it down, unstaged; then it gathers
    // for a base and builds one, whose second milestone puts a bag inside.
    let bags = h.bot.home().stats.bags_placed;
    assert!((1..=2).contains(&bags), "{bags} bags: {}", h.explain());
    assert!(
        h.bot.builder().survey().hearth,
        "no cupboard in half an hour: {:?} {}",
        h.bot.builder().stats,
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

/// A home, in lockstep. With stone tools and the wood, stone and cloth for
/// the first three milestones staged in the pack, the playbook puts a bag
/// down, chooses a plot and builds on it from inside: the cupboard and the
/// twig shell, the wooden doors, a bag and a box inside, then the core
/// graded to stone. Every piece goes down with the plan in hand, every
/// grade with the hammer, every deployable itself in hand. Then it lives
/// there: feeds the cupboard and puts its loot in the box, walks out
/// through its own doors for wood and shuts them behind, comes back in
/// through them to build, and when the session is ending goes home and
/// stands inside behind shut doors. What it built then keeps a stranger
/// out, the way `tests/base.rs` uses a base: a shut door is a wall, and
/// nobody else builds inside its claim.
#[test]
fn a_survivor_builds_its_starter_and_the_base_keeps_strangers_out() {
    use server::agent::build::{Milestone, Region};
    use sim_core::build::{BUILD_CELL_M, LOC_EDGE_XLO, LOC_EDGE_ZLO, LOC_PLANE, MAT_STONE};
    use sim_core::deploy::{ARCH_BAG, ARCH_BOX, ARCH_DOOR, ARCH_HEARTH};

    let mut h = Harness::new(false);
    let content = content::Content::load_dir(&root().join("content")).unwrap();
    let catalog = server::net::bake_all(&content).unwrap().catalog;
    let stack = |id: &str, count: u16| {
        let item = content.item_index(id).unwrap();
        sim_core::gather::ItemStack {
            item,
            count,
            cond: catalog.cond_max(item as usize),
            skin: 0,
        }
    };
    // The join lands on the tick after `connect`.
    h.until(5, |_| false);
    let kit = [
        stack("item.wood", 1000),
        stack("item.wood", 1000),
        stack("item.wood", 400),
        stack("item.stone", 1000),
        stack("item.stone", 1000),
        stack("item.stone", 1000),
        stack("item.cloth", 60),
    ];
    let (hatchet, pickaxe) = (
        stack("item.hatchet_stone", 1),
        stack("item.pickaxe_stone", 1),
    );
    h.stage(|p| {
        p.inv[1] = hatchet;
        p.inv[2] = pickaxe;
        for (i, s) in kit.iter().enumerate() {
            p.inv[10 + i] = *s;
        }
    });

    // 1. The shell, the doors, the bag and box inside, the stone core.
    let built = h.until(24_000, |b| {
        b.builder().survey().milestone >= Milestone::Upstairs
    });
    if !built {
        let me = h.view.get(ID).copied().unwrap();
        let plan = h.bot.builder().plan().unwrap();
        eprintln!(
            "DEBUG rel ({}, {}) builder {:?}",
            me.qx as f32 * sim_core::movement::POS_XZ_Q - f32::from(plan.cx + 1) * 3.0,
            me.qz as f32 * sim_core::movement::POS_XZ_Q - f32::from(plan.cz + 1) * 3.0,
            h.bot.builder()
        );
    }
    assert!(
        built,
        "the base stopped at {:?}: {:?} {}",
        h.bot.builder().survey(),
        h.bot.builder().stats,
        h.explain()
    );
    let plan = h.bot.builder().plan().expect("a plot");
    let (cx, cz) = (plan.cx, plan.cz);
    {
        let w = &h.shard.world;
        let arch = |x: u16, z: u16, level: u8, loc: u8| {
            w.deploys
                .find(x, z, level, loc)
                .filter(|d| d.owner == ID)
                .map(|d| w.deploy.defs[usize::from(d.row)].arch)
        };
        assert_eq!(
            arch(cx, cz, 0, LOC_PLANE),
            Some(ARCH_HEARTH),
            "the cupboard"
        );
        assert_eq!(
            arch(cx + 1, cz + 1, 0, LOC_EDGE_XLO),
            Some(ARCH_DOOR),
            "front door"
        );
        assert_eq!(
            arch(cx + 1, cz + 1, 0, LOC_EDGE_ZLO),
            Some(ARCH_DOOR),
            "inner door"
        );
        assert_eq!(
            arch(cx + 1, cz, 0, LOC_PLANE),
            Some(ARCH_BOX),
            "the box inside"
        );
        assert_eq!(
            arch(cx + 1, cz + 1, 0, LOC_PLANE),
            Some(ARCH_BAG),
            "the bag inside"
        );
        let wood_door = content.item_index("item.door_wood").unwrap();
        for loc in [LOC_EDGE_XLO, LOC_EDGE_ZLO] {
            let d = w.deploys.find(cx + 1, cz + 1, 0, loc).unwrap();
            assert_eq!(w.deploy.defs[usize::from(d.row)].item, wood_door);
        }
        // The shell, and the core's walls in stone.
        for (x, z, loc) in [
            (cx, cz, LOC_EDGE_XLO),
            (cx, cz, LOC_EDGE_ZLO),
            (cx, cz + 1, LOC_EDGE_ZLO),
            (cx + 1, cz, LOC_EDGE_ZLO),
            (cx + 2, cz, LOC_EDGE_XLO),
        ] {
            let rec = w.pieces.find(x, z, 0, loc).expect("a shell wall");
            assert_eq!(
                w.build.pieces[rec.row as usize].material, MAT_STONE,
                "wall at {x},{z} loc {loc}"
            );
        }
    }
    let stats = h.bot.builder().stats;
    assert!(
        stats.placed >= 11 && stats.deployed >= 5 && stats.graded >= 11,
        "{stats:?}"
    );
    assert!(
        h.held_checks as u64 >= stats.placed + stats.graded + stats.deployed,
        "every place, grade and deploy went out with its item in hand: {} checks, {stats:?}",
        h.held_checks
    );

    // 1b. Home with loot in the pack: the stone core stands, so the
    //     cupboard pays upkeep and is fed, and the ore goes in the box while
    //     the belt loadout (the tools) and what the storey above needs stay
    //     on the body. Every open and feed is the `E` pick, every move made
    //     with the box's panel open.
    let ore = stack("item.metal_ore", 300);
    h.stage(|p| {
        if let Some(free) = p.inv.iter_mut().skip(HOTBAR_SLOTS).find(|x| x.count == 0) {
            *free = ore;
        }
    });
    let units = |stacks: &[sim_core::gather::ItemStack], item: u16| {
        stacks
            .iter()
            .filter(|s| s.count > 0 && s.item == item)
            .map(|s| u32::from(s.count))
            .sum::<u32>()
    };
    let mut stashed = false;
    for _ in 0..3 * 60 * TICK_HZ {
        h.step();
        let w = &h.shard.world;
        let fed = w
            .deploys
            .hearths()
            .iter()
            .any(|r| (r.cx, r.cz) == (cx, cz) && r.stock.iter().any(|&u| u > 0));
        let boxed = w
            .deploys
            .boxes()
            .iter()
            .find(|b| (b.cx, b.cz, b.level) == (cx + 1, cz, 0))
            .map_or(0, |b| units(&b.items, ore.item));
        if fed
            && boxed == 300
            && units(&h.me().inv, ore.item) == 0
            && h.bot.goal() != Some(Goal::Stash)
        {
            stashed = true;
            break;
        }
    }
    assert!(
        stashed,
        "the loot never went in the box, or the cupboard was never fed: {:?} {:?} {}",
        h.bot.home().upkeep(),
        h.bot.ledger(),
        h.explain()
    );
    for tool in [hatchet, pickaxe] {
        assert!(units(&h.me().inv, tool.item) > 0, "a tool left the body");
    }
    assert_eq!(h.bot.ledger().units(ore.item), 300, "what the panel showed");
    assert!(h.bot.home().upkeep().is_some(), "the feed's reply was kept");
    for verb in ["container", "feed", "move"] {
        assert!(h.verbs.contains(verb), "{verb} never sent: {:?}", h.verbs);
    }
    assert!(h.pick_checks >= 2 && h.panel_checks >= 1);

    // 2. Short of wood for the storey above: out through its own doors,
    //    shut behind it.
    let mut outside = false;
    for _ in 0..9_000 {
        h.step();
        let me = h.view.get(ID).copied().unwrap();
        let shut = |loc: u8| {
            h.shard
                .world
                .deploys
                .find(cx + 1, cz + 1, 0, loc)
                .is_some_and(|d| !d.open)
        };
        if h.bot.builder().region(&me) == Region::Outside
            && !h.bot.builder().passing()
            && h.bot.builder().stats.uses >= 4
            && shut(LOC_EDGE_XLO)
            && shut(LOC_EDGE_ZLO)
        {
            outside = true;
            break;
        }
    }
    assert!(
        outside,
        "never left through the doors and shut them: {:?} {}",
        h.bot.builder().stats,
        h.explain()
    );
    assert!(u64::from(h.use_checks) >= h.bot.builder().stats.uses);
    for verb in ["place", "upgrade", "deploy", "use", "craft"] {
        assert!(h.verbs.contains(verb), "{verb} never sent: {:?}", h.verbs);
    }
    let expected: BTreeSet<&str> = EXPECTED_VERBS.into_iter().collect();
    assert!(h.verbs.is_subset(&expected));
    assert_eq!(
        h.heap_ops, 0,
        "the agent's frame loop touched the allocator"
    );
    println!("{:?} {}", h.bot.builder().stats, h.explain());

    // 2b. The wood and stone for the storey above: back in through its own
    //     doors to build it, both shut behind. Once on the stand spot the
    //     walk is over, so the build goal is at a checkpoint between ops
    //     and an answer held for one (hunger, here) is taken there.
    let (wood, stone, berries) = (
        stack("item.wood", 1000),
        stack("item.stone", 1000),
        stack("item.berries", 5),
    );
    h.stage(|p| {
        for s in [wood, wood, stone, berries] {
            if let Some(free) = p.inv.iter_mut().skip(HOTBAR_SLOTS).find(|x| x.count == 0) {
                *free = s;
            }
        }
    });
    let uses = h.bot.builder().stats.uses;
    let mut inside = false;
    for _ in 0..9_000 {
        h.step();
        let me = h.view.get(ID).copied().unwrap();
        let b = h.bot.builder();
        if h.bot.goal() == Some(Goal::Build)
            && b.stats.uses >= uses + 4
            && b.region(&me) == Region::Room
            && !b.passing()
            && b.at_checkpoint()
        {
            inside = true;
            break;
        }
    }
    assert!(
        inside,
        "never back in to build, or the walk in never ended: {:?} {}",
        h.bot.builder().stats,
        h.explain()
    );
    h.stage(|p| p.food = 5);
    let fed = h.until(3 * 30 * TICK_HZ, |b| b.goal() == Some(Goal::Eat));
    assert!(fed, "hunger mid-build was never answered: {}", h.explain());
    assert_eq!(h.heap_ops, 0);

    // 2c. The session is ending and the body is outside: home, in through
    //     both doors, shut behind it, and it stands inside without asking
    //     the mind for anything more, so its sleeper is behind a door.
    let at_rest = h.until(60 * TICK_HZ, |b| {
        b.builder().at_checkpoint() && !b.builder().passing() && b.goal() != Some(Goal::Eat)
    });
    assert!(at_rest, "never at a checkpoint: {}", h.explain());
    let out = sim_core::movement::Body::at(
        SEED,
        &h.shard.world.haven,
        f32::from(cx + 1) * BUILD_CELL_M - 1.5,
        f32::from(cz + 1) * BUILD_CELL_M + 1.5,
    );
    h.stage(move |p| p.body = out);
    h.step();
    assert_eq!(
        h.bot.builder().region(&h.view.get(ID).copied().unwrap()),
        Region::Outside
    );
    h.bot.set_deadline(Some(h.tick + 60 * TICK_HZ));
    let uses = h.bot.builder().stats.uses;
    let shut = |h: &Harness, loc: u8| {
        h.shard
            .world
            .deploys
            .find(cx + 1, cz + 1, 0, loc)
            .is_some_and(|d| !d.open)
    };
    let mut home = false;
    for _ in 0..50 * TICK_HZ {
        h.step();
        let me = h.view.get(ID).copied().unwrap();
        if h.bot.stats.phase == server::explorer::Phase::LoggingOff
            && h.bot.builder().region(&me) == Region::Room
            && shut(&h, LOC_EDGE_XLO)
            && shut(&h, LOC_EDGE_ZLO)
        {
            home = true;
            break;
        }
    }
    assert!(
        home,
        "not home behind shut doors for the log-off: {:?} {}",
        h.bot.builder().stats,
        h.explain()
    );
    assert!(
        h.bot.builder().stats.uses >= uses + 4,
        "in through both doors"
    );
    let asked = h.bot.mind.stats.requests;
    for _ in 0..10 * TICK_HZ {
        h.step();
    }
    let me = h.view.get(ID).copied().unwrap();
    assert_eq!(h.bot.stats.phase, server::explorer::Phase::LoggingOff);
    assert_eq!(h.bot.builder().region(&me), Region::Room);
    assert!(shut(&h, LOC_EDGE_XLO) && shut(&h, LOC_EDGE_ZLO));
    assert_eq!(h.bot.mind.stats.requests, asked, "the mind is not asked");
    assert_eq!(h.heap_ops, 0);

    // 2d. Somebody opens its front door from outside (doors go up with no
    //     lock until lane C, so a plain use opens them): it walks out
    //     through both doors and back in, shutting each behind it.
    h.with_puppet();
    h.step();
    let (fx, fz) = (
        f32::from(cx + 1) * BUILD_CELL_M,
        f32::from(cz + 1) * BUILD_CELL_M,
    );
    let at_door = sim_core::movement::Body::at(SEED, &h.shard.world.haven, fx - 1.0, fz + 1.5);
    h.stage_puppet(move |p| p.body = at_door);
    for core in [&mut h.shard, &mut h.replay] {
        core.push_action(
            1,
            ActionMsg::Use {
                cx: cx + 1,
                cz: cz + 1,
                level: 0,
                loc: LOC_EDGE_XLO,
            },
        );
    }
    let opened = h.until(TICK_HZ, |_| false) || !shut(&h, LOC_EDGE_XLO);
    assert!(opened, "the puppet never opened the front door");
    // Out of the way again.
    let away = sim_core::movement::Body::at(SEED, &h.shard.world.haven, fx - 25.0, fz + 25.0);
    h.stage_puppet(move |p| p.body = away);
    let uses = h.bot.builder().stats.uses;
    let mut resealed = false;
    for _ in 0..60 * TICK_HZ {
        h.step();
        let me = h.view.get(ID).copied().unwrap();
        if h.bot.builder().stats.uses >= uses + 4
            && h.bot.stats.phase == server::explorer::Phase::LoggingOff
            && h.bot.builder().region(&me) == Region::Room
            && shut(&h, LOC_EDGE_XLO)
            && shut(&h, LOC_EDGE_ZLO)
        {
            resealed = true;
            break;
        }
    }
    assert!(
        resealed,
        "the door left open was never shut again: {:?} {}",
        h.bot.builder().stats,
        h.explain()
    );
    assert_eq!(h.bot.mind.stats.requests, asked, "the mind is not asked");
    assert_eq!(h.heap_ops, 0);

    // 2e. Somebody swings a spear at its shut front door from outside. The
    //     blow reaches it as any player hears one, the wall's own broadcast
    //     on the event lane: home is under attack, and the summary says so.
    let tick = h.tick;
    assert!(!h.bot.home().under_attack(tick));
    let spear = h.spear;
    let at_door = sim_core::movement::Body::at(SEED, &h.shard.world.haven, fx - 1.0, fz + 1.5);
    h.stage_puppet(move |p| {
        p.body = at_door;
        p.inv[0] = spear;
    });
    let struck = |h: &Harness| {
        h.shard
            .world
            .deploys
            .find(cx + 1, cz + 1, 0, LOC_EDGE_XLO)
            .is_some_and(|d| d.hp < h.shard.world.deploy.defs[usize::from(d.row)].hp)
    };
    let mut alarmed = false;
    for _ in 0..5 * TICK_HZ {
        // Facing +X, level, primary held: into the door.
        let mut dg = InputDatagram::new(0, 0, sim_core::limits::INTERP_DELAY_TICKS);
        dg.push(sim_core::input::InputFrame {
            seq: h.tick as u16,
            buttons: BTN_PRIMARY,
            yaw: 1 << 14,
            pitch: 128,
            ..Default::default()
        })
        .unwrap();
        let mut bytes = [0u8; sim_core::limits::DATAGRAM_BUDGET_BYTES];
        let len = encode_input(&dg, &mut bytes).unwrap();
        let decoded = decode_input(&bytes[..len]).unwrap();
        h.shard.push_input(1, &decoded);
        h.replay.push_input(1, &decoded);
        h.step();
        if struck(&h) && h.bot.home().under_attack(h.tick) {
            alarmed = true;
            break;
        }
    }
    assert!(
        struck(&h),
        "the puppet's spear never reached the door: {}",
        h.explain()
    );
    assert!(alarmed, "a blow on its own door raised no alarm");
    h.step();
    assert!(h.bot.summary(&h.view, ID).unwrap().home.attacked);
    let away = sim_core::movement::Body::at(SEED, &h.shard.world.haven, fx - 25.0, fz + 25.0);
    h.stage_puppet(move |p| p.body = away);
    assert_eq!(h.heap_ops, 0);

    // 3. Its base, used by somebody else, with its owner asleep inside. The
    //    lockstep is over, so the live world may be driven directly now.
    let w = &mut h.shard.world;
    let (ax, az) = (
        f32::from(cx + 1) * BUILD_CELL_M,
        f32::from(cz + 1) * BUILD_CELL_M,
    );
    // Outside the shut front door, walking at it: a wall.
    let mut body = sim_core::movement::Body::at(SEED, &w.haven, ax - 1.2, az + 1.5);
    let mut scratch = sim_core::occupy::Scratch::barren();
    for _ in 0..60 {
        sim_core::movement::step(
            SEED,
            &w.haven,
            w.pieces.cols(),
            &mut scratch.occupants(),
            &mut body,
            &sim_core::input::InputFrame {
                move_x: 127,
                ..Default::default()
            },
        );
    }
    let x = body.qx as f32 * sim_core::movement::POS_XZ_Q;
    assert!(x < ax + 0.5, "walked through the shut front door to x {x}");
    // A stranger two cells south of the front door, with a foundation's
    // wood in hand: refused for the claim.
    const STRANGER: u32 = 300;
    w.tick(&[sim_core::world::Command::Join { id: STRANGER }]);
    let slot = w
        .players
        .iter()
        .position(|p| p.active && p.id == STRANGER)
        .expect("the stranger joined");
    w.players[slot].body = sim_core::movement::Body::at(
        SEED,
        &w.haven,
        f32::from(cx + 1) * BUILD_CELL_M + 1.5,
        f32::from(cz + 3) * BUILD_CELL_M + 1.5,
    );
    w.players[slot].inv[0] = stack("item.wood", 1000);
    let foundation = server::population::base_rows(&content).unwrap().foundation;
    w.tick(&[sim_core::world::Command::Place {
        id: STRANGER,
        row: foundation,
        cx: cx + 1,
        cz: cz + 3,
        level: 0,
        loc: LOC_PLANE,
        freehand: false,
        plate: 0,
    }]);
    assert!(
        w.events
            .entries()
            .iter()
            .any(|e| e.code == sim_core::world::EV_BUILD_REFUSED
                && e.a == STRANGER
                && e.b == sim_core::build::REFUSE_B_CLAIM),
        "a stranger built inside the base's claim"
    );
    assert!(w.pieces.find(cx + 1, cz + 3, 0, LOC_PLANE).is_none());
}
