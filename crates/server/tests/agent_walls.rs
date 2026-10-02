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

use common::{root, scene, shore, SEED};
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
const EXPECTED_VERBS: [&str; 21] = [
    "craft",
    "consume",
    "drink",
    "move",
    "respawn",
    "reload",
    "loot",
    "pickup",
    "pick",
    "deploy",
    "place",
    "upgrade",
    "use",
    "container",
    "feed",
    "access",
    "demolish",
    "unlock",
    "research",
    "repair",
    "throw",
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
        ActionMsg::Vend { .. } => "vend",
        ActionMsg::Swipe { .. } => "swipe",
        ActionMsg::Pick { .. } => "pick",
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
    satchel: u16,
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
        let mind = Mind::inline(Scripted::default(), MindConfig::default()).unwrap();
        Self::with_mind(wildlife, at, mind)
    }

    fn with_mind(wildlife: bool, at: (f32, f32), mind: Mind) -> Self {
        Self::with_opts(
            wildlife,
            at,
            mind,
            server::explorer::SurvivorOpts::default(),
        )
    }

    fn with_opts(
        wildlife: bool,
        at: (f32, f32),
        mind: Mind,
        opts: server::explorer::SurvivorOpts,
    ) -> Self {
        let content = common::content();
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
        let satchel = content.item_index("item.satchel_charge").unwrap();
        let mut bot = Survivor::with(mind, opts);
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
            satchel,
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
            ActionMsg::Throw { .. } => Some(self.satchel),
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
            use sim_core::inventory::{CONT_BOX, CONT_WORLD};
            match msg {
                // A door swung, or a fire, a recycler or a research table
                // switched (`C`, on the same pick).
                ActionMsg::Use { cx, cz, level, loc } => {
                    let pick = self.e_pick(frame.yaw);
                    let switch = matches!(pick.verb, Verb::Fire | Verb::Recycler | Verb::Research);
                    assert!(
                        (pick.verb == Verb::Door || switch)
                            && (pick.cx, pick.cz, pick.level) == (cx, cz, level)
                            && (switch || pick.loc == loc),
                        "the agent used {cx},{cz},{level},{loc}, which `E` would not pick \
                         ({pick:?}), at tick {}",
                        self.tick
                    );
                    self.use_checks += 1;
                }
                ActionMsg::Container {
                    kind: CONT_BOX,
                    cont,
                } => {
                    let pick = self.e_pick(frame.yaw);
                    assert!(
                        matches!(
                            pick.verb,
                            Verb::Box | Verb::Fire | Verb::Recycler | Verb::Research
                        ) && pick.handle == cont,
                        "the agent opened {cont:#x}, which `E` would not pick ({pick:?}), at \
                         tick {}",
                        self.tick
                    );
                    self.pick_checks += 1;
                }
                // The keypad speaks to the lock `L` opened it on: the one
                // `E` picks, on something a lock bolts to.
                ActionMsg::Access {
                    cx, cz, level, loc, ..
                } => {
                    let pick = self.e_pick(frame.yaw);
                    assert!(
                        sim_core::deploy::lockable(pick.arch)
                            && pick.has_lock
                            && (pick.cx, pick.cz, pick.level, pick.loc) == (cx, cz, level, loc),
                        "the agent spoke to the lock at {cx},{cz},{level},{loc}, not the one `L` \
                         would open ({pick:?}), at tick {}",
                        self.tick
                    );
                    self.pick_checks += 1;
                }
                // Backspace takes the nearest structure to the feet; so do
                // `R` (repair) and `X` (plant the charge in hand).
                ActionMsg::Demolish {
                    deploy,
                    cx,
                    cz,
                    level,
                    loc,
                }
                | ActionMsg::Repair {
                    deploy,
                    cx,
                    cz,
                    level,
                    loc,
                }
                | ActionMsg::Throw {
                    deploy,
                    cx,
                    cz,
                    level,
                    loc,
                } => {
                    let near = self.nearest_structure();
                    assert!(
                        near.is_some_and(|t| t.store.is_deploy() == deploy
                            && (t.cx, t.cz, t.level, t.loc) == (cx, cz, level, loc)),
                        "the agent worked {cx},{cz},{level},{loc}, not the nearest \
                         structure ({near:?}), at tick {}",
                        self.tick
                    );
                    self.pick_checks += 1;
                }
                // The tree is bought from the panel `E` on a bench opens,
                // at that bench's rung: the node's tier or above.
                ActionMsg::Unlock { recipe } => {
                    let pick = self.e_pick(frame.yaw);
                    let station = self.shard.world.craft.recipes[usize::from(recipe)].station;
                    let tier = sim_core::research::node_tier(station);
                    assert!(
                        pick.verb == Verb::TechTree
                            && sim_core::deploy::bench_tier(pick.arch) >= tier,
                        "the agent bought recipe {recipe} (tier {tier}) with no bench of that                          rung under `E` ({pick:?}), at tick {}",
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
                // A crate: the one the human's `E` would open, cast from
                // this body along this frame's look; emptied with its panel
                // open.
                ActionMsg::Container {
                    kind: CONT_WORLD,
                    cont,
                } => {
                    let pick = self.open_pick(frame.yaw, frame.pitch);
                    assert!(
                        pick.occupant != 0 && sim_core::gather::cell_key(pick.cx, pick.cz) == cont,
                        "the agent opened {cont:#x}, not the crate `E` would pick ({pick:?}), \
                         at tick {}",
                        self.tick
                    );
                    self.pick_checks += 1;
                }
                ActionMsg::Move {
                    cont, from_kind, ..
                } if from_kind == CONT_WORLD => {
                    let core = self.bot.core().unwrap();
                    assert_eq!(
                        (core.cont_kind, core.cont_handle),
                        (CONT_WORLD, cont),
                        "the agent moved a crate's stack without its panel open, at tick {}",
                        self.tick
                    );
                    self.panel_checks += 1;
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

    /// What the human client's take-down key (`Backspace`) would take: the
    /// nearest structure to the bot's feet, over its client's mirror.
    fn nearest_structure(&self) -> Option<client::ui::structure::Target> {
        let core = self.bot.core().unwrap();
        let me = self.view.get(ID).copied().unwrap();
        let q = sim_core::movement::POS_XZ_Q;
        client::ui::structure::nearest(
            (me.qx as f32 * q, me.qz as f32 * q),
            core.pieces.entries(),
            &core.piece_defs,
            core.piece_defs_have,
            core.deploys.entries(),
            &core.deploy_defs,
            core.deploy_defs_have,
        )
    }

    /// What the human client's `E` would open in the scatter (a crate or a
    /// cache) from this body, looking along `yaw` and `pitch`.
    fn open_pick(&self, yaw: u16, pitch: u8) -> client::ui::interact::SwingPick {
        use client::ui::interact::{resolve_open, Island, SwingAim};
        let me = self.view.get(ID).copied().unwrap();
        let w = &self.shard.world;
        let mut cache = sim_core::occupy::SlotCache::new();
        resolve_open(
            SwingAim {
                x: me.qx as f32 * sim_core::movement::POS_XZ_Q,
                y: me.qy as f32 * sim_core::movement::POS_Y_Q,
                z: me.qz as f32 * sim_core::movement::POS_XZ_Q,
                yaw,
                pitch,
                // The human client's own stance read (v83).
                crouched: self.bot.core().unwrap().crouched(),
            },
            &mut Island {
                seed: SEED,
                doors: w.card_door_bits,
                table: &w.scatter,
                haven: &w.haven,
                harvested: &w.slot_lives,
                cache: &mut cache,
            },
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
    let content = common::content();
    let stack = |id: &str, count: u16| sim_core::gather::ItemStack {
        count,
        ..common::stack(&content, id)
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
    let content = common::content();
    let stack = |id: &str, count: u16| sim_core::gather::ItemStack {
        count,
        ..common::stack(&content, id)
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

/// The test's own decision source: a loot run whenever one is on offer and
/// a barrel or crate is in its eyes' memory, else stand and look round.
struct LootWhenSeen;

impl server::mind::DecisionSource for LootWhenSeen {
    fn kind(&self) -> server::mind::SourceKind {
        server::mind::SourceKind::Scripted
    }

    fn decide(&mut self, s: &server::mind::Summary) -> Result<server::mind::Choice, String> {
        let goal = if s.offers(Goal::Loot) && s.loot.count > 0 {
            Goal::Loot
        } else {
            Goal::Wait
        };
        Ok(server::mind::Choice {
            goal,
            confidence: 1.0,
            reason: server::mind::Reason::EMPTY,
            input_tokens: 0,
            output_tokens: 0,
        })
    }
}

/// Units of `item` in a player's pack and belt.
fn units_of(p: &sim_core::world::Player, item: u16) -> u32 {
    p.inv
        .iter()
        .filter(|s| s.count > 0 && s.item == item)
        .map(|s| u32::from(s.count))
        .sum()
}

#[test]
fn a_survivor_breaks_a_barrel_and_picks_up_what_falls_out() {
    use sim_core::terrain::Occupant;
    let ((cx, cz), at) = common::container(Occupant::BarrelSlot);
    let mind = Mind::inline(LootWhenSeen, MindConfig::default()).unwrap();
    let mut h = Harness::with_mind(false, at, mind);
    let content = common::content();
    let junk = content.item_index("item.junk").unwrap();
    let hatchet = common::stack(&content, "item.hatchet_stone");
    // The join lands on the tick after `connect`.
    h.until(5, |_| false);
    h.stage(|p| p.inv[1] = hatchet);

    let key = sim_core::gather::cell_key(cx, cz);
    let broke = h.until(60 * TICK_HZ, |b| {
        b.loot().stats.barrels >= 1 && b.core().unwrap().harvested.contains(key)
    });
    assert!(
        broke,
        "the barrel never broke: {:?} {}",
        h.bot.loot().stats,
        h.explain()
    );
    // What fell out is picked up, a stack a press: the barrel's two junk
    // are guaranteed, whatever else it rolled.
    let taken = h.until(30 * TICK_HZ, |b| {
        b.goal() != Some(Goal::Loot) && b.loot().stats.pickups >= 1
    });
    assert!(
        taken,
        "nothing was picked up: {:?} {}",
        h.bot.loot().stats,
        h.explain()
    );
    assert!(
        units_of(h.me(), junk) >= 2,
        "the barrel's junk is in the pack: {}",
        h.explain()
    );
    let left = h
        .shard
        .world
        .ground_items
        .entries()
        .iter()
        .filter(|g| {
            let q = sim_core::movement::POS_XZ_Q;
            let slot = sim_core::terrain::scatter(
                SEED,
                &h.shard.world.scatter,
                &h.shard.world.haven,
                i32::from(cx),
                i32::from(cz),
            );
            (g.qx as f32 * q - slot.x).hypot(g.qz as f32 * q - slot.z) < 4.0
        })
        .count();
    assert_eq!(left, 0, "stacks were left lying by the barrel");
    let run = h.bot.history.iter().find(|r| r.goal == Goal::Loot).unwrap();
    assert_eq!(run.outcome, Outcome::Done, "{}", h.explain());
    assert!(h.verbs.contains("pickup"), "never picked up: {:?}", h.verbs);
    assert_eq!(
        h.heap_ops, 0,
        "the agent's frame loop touched the allocator"
    );
}

#[test]
fn a_survivor_empties_a_crate_through_its_panel() {
    use sim_core::terrain::Occupant;
    let ((cx, cz), at) = common::container(Occupant::CrateSlot);
    let mind = Mind::inline(LootWhenSeen, MindConfig::default()).unwrap();
    let mut h = Harness::with_mind(false, at, mind);
    let content = common::content();
    let junk = content.item_index("item.junk").unwrap();
    h.until(5, |_| false);

    let done = h.until(60 * TICK_HZ, |b| {
        b.loot().stats.emptied >= 1 && b.goal() != Some(Goal::Loot) && b.loot().stats.moves >= 1
    });
    assert!(
        done,
        "the crate was not emptied: {:?} {}",
        h.bot.loot().stats,
        h.explain()
    );
    // The junk every crate and cache pays (five at the least).
    assert!(units_of(h.me(), junk) >= 5, "{}", h.explain());
    // What it opened (this one, or another standing beside it) it left
    // empty.
    let w = &h.shard.world;
    let opened = w.world_conts.entries();
    assert!(!opened.is_empty(), "no crate was opened");
    assert!(
        opened.iter().all(|c| c.is_empty()),
        "a crate was left with loot in it"
    );
    assert!(opened
        .iter()
        .all(|c| (i32::from(c.cx) - i32::from(cx)).abs() <= 8
            && (i32::from(c.cz) - i32::from(cz)).abs() <= 8));
    assert!(h.pick_checks >= 1 && h.panel_checks >= 1);
    for verb in ["container", "move"] {
        assert!(h.verbs.contains(verb), "{verb} never sent: {:?}", h.verbs);
    }
    assert_eq!(
        h.heap_ops, 0,
        "the agent's frame loop touched the allocator"
    );
}

/// The starter stands, the pack holds what the stations cost: the
/// workbench goes on a stone foundation behind the core, the furnace is
/// crafted at it and goes on the floor over the cupboard, and ore staged
/// afterwards is smelted there into fragments, a batch at a time.
#[test]
fn a_survivor_puts_down_its_bench_and_furnace_and_smelts_ore() {
    use server::agent::build::{Milestone, YARD};
    use sim_core::build::LOC_PLANE;
    use sim_core::deploy::{ARCH_FURNACE, ARCH_WORKBENCH};

    let mut h = Harness::new(false);
    let content = common::content();
    let stack = |id: &str, count: u16| sim_core::gather::ItemStack {
        count,
        ..common::stack(&content, id)
    };
    h.until(5, |_| false);
    let kit = [
        stack("item.wood", 1000),
        stack("item.wood", 1000),
        stack("item.wood", 1000),
        stack("item.wood", 1000),
        stack("item.wood", 1000),
        stack("item.stone", 1000),
        stack("item.stone", 1000),
        stack("item.stone", 1000),
        stack("item.stone", 1000),
        stack("item.stone", 1000),
        stack("item.stone", 1000),
        stack("item.stone", 1000),
        stack("item.cloth", 60),
        stack("item.metal_frags", 100),
        stack("item.lowgrade", 50),
    ];
    let (hatchet, pickaxe) = (
        stack("item.hatchet_stone", 1),
        stack("item.pickaxe_stone", 1),
    );
    h.stage(|p| {
        p.inv[1] = hatchet;
        p.inv[2] = pickaxe;
        for (i, s) in kit.iter().enumerate() {
            p.inv[HOTBAR_SLOTS + i] = *s;
        }
    });

    // 1. The starter, then the bench, then the furnace crafted at it.
    let stations = h.until(60_000, |b| {
        b.builder().survey().milestone >= Milestone::Done
            || b.core()
                .is_some_and(|c| b.builder().stations(c).furnace.is_some())
    });
    assert!(
        stations,
        "the stations never stood: {:?} {:?} {}",
        h.bot.builder().survey(),
        h.bot.builder().stats,
        h.explain()
    );
    let plan = h.bot.builder().plan().expect("a plot");
    let (cx, cz) = (plan.cx, plan.cz);
    {
        let w = &h.shard.world;
        let arch = |x: u16, z: u16, level: u8| {
            w.deploys
                .find(x, z, level, LOC_PLANE)
                .filter(|d| d.owner == ID)
                .map(|d| w.deploy.defs[usize::from(d.row)].arch)
        };
        assert_eq!(arch(cx, cz, 1), Some(ARCH_FURNACE), "the furnace upstairs");
        let (yx, yz) = (
            cx.checked_add_signed(i16::from(YARD.0)).unwrap(),
            cz.checked_add_signed(i16::from(YARD.1)).unwrap(),
        );
        assert_eq!(arch(yx, yz, 0), Some(ARCH_WORKBENCH), "the bench behind");
    }
    assert!(
        h.held_checks as u64 >= h.bot.builder().stats.deployed,
        "every deploy went out with its item in hand"
    );

    // 2. Ore in the pack, once the body is off somewhere out of the
    //    furnace's reach: home to it, and fragments come of it.
    let stand = h.bot.builder().stand().expect("a stand spot");
    let away = h.until(5 * 60 * TICK_HZ, |b| {
        b.core().is_some_and(|c| {
            let me = c.predict.render_position();
            (me[0] - stand[0]).hypot(me[2] - stand[1]) > 2.0 * sim_core::craft::STATION_RADIUS_M
        })
    });
    assert!(away, "the body never left its base: {}", h.explain());
    let frags = content.item_index("item.metal_frags").unwrap();
    let before = units_of(h.me(), frags);
    let ore = stack("item.metal_ore", 60);
    h.stage(|p| {
        if let Some(free) = p.inv.iter_mut().skip(HOTBAR_SLOTS).find(|x| x.count == 0) {
            *free = ore;
        }
    });
    let smelted = h.until(5 * 60 * TICK_HZ, |b| {
        b.held("Metal Fragments") >= before + 40
    });
    assert!(
        smelted,
        "the ore was not smelted: {} fragments, {} before {}",
        units_of(h.me(), frags),
        before,
        h.explain()
    );
    assert!(
        units_of(h.me(), frags) >= before + 40,
        "fragments the body holds: {}",
        h.explain()
    );
    assert!(
        h.bot
            .history
            .iter()
            .any(|r| r.goal.label().as_str() == "craft:Metal Fragments"
                && r.outcome == Outcome::Done),
        "{}",
        h.explain()
    );
    for verb in ["craft", "deploy", "place"] {
        assert!(h.verbs.contains(verb), "{verb} never sent: {:?}", h.verbs);
    }
    assert_eq!(
        h.heap_ops, 0,
        "the agent's frame loop touched the allocator"
    );
    println!("{:?} {}", h.bot.builder().stats, h.explain());
}

/// The starter and its stations stand and the pack holds the fragments for
/// three locks and metal doors: code locks go on both doors and the
/// cupboard, armed with its code, and the front door is swapped for metal
/// (part of the milestone; a lock already on it comes off with it and goes
/// on again).
/// A stranger at the front door is refused, the wrong code shocks him and
/// does not let him in. Somebody who knows the code resets the lock's list
/// to himself alone: the owner, refused at its own door, enters its code
/// and walks in.
#[test]
fn a_survivor_locks_its_doors_and_a_stranger_cannot_open_them() {
    use server::agent::build::Region;
    use server::agent::lock::{LockCode, LockSecret};
    use sim_core::build::{BUILD_CELL_M, LOC_EDGE_XLO, LOC_EDGE_ZLO, LOC_PLANE};
    use sim_core::deploy::{ACCESS_OP_ENTER, ACCESS_OP_SET_CODE};

    let code = LockCode::derive(b"agent walls", "jev");
    let mind = Mind::inline(Scripted::default(), MindConfig::default()).unwrap();
    // It answers a blow but starts nothing: the stranger at its door below
    // is unarmed, and an opportunist would not leave him standing.
    let opts = server::explorer::SurvivorOpts {
        lock: Some(LockSecret::Code(code)),
        temperament: server::agent::combat::Temperament::Defensive,
        ..Default::default()
    };
    let mut h = Harness::with_opts(false, scene(), mind, opts);
    let content = common::content();
    let stack = |id: &str, count: u16| sim_core::gather::ItemStack {
        count,
        ..common::stack(&content, id)
    };
    h.until(5, |_| false);
    let mut kit = vec![stack("item.wood", 1000); 6];
    kit.extend([stack("item.stone", 1000); 7]);
    kit.extend([
        stack("item.cloth", 60),
        stack("item.metal_frags", 1000),
        stack("item.metal_frags", 1000),
        stack("item.lowgrade", 50),
    ]);
    let (hatchet, pickaxe) = (
        stack("item.hatchet_stone", 1),
        stack("item.pickaxe_stone", 1),
    );
    h.stage(|p| {
        p.inv[1] = hatchet;
        p.inv[2] = pickaxe;
        for (i, s) in kit.iter().enumerate() {
            p.inv[HOTBAR_SLOTS + i] = *s;
        }
    });

    // 1. The starter, its stations, the locks armed, the doors in metal.
    let lock_at = |h: &Harness, x: u16, z: u16, loc: u8| {
        h.shard
            .world
            .deploys
            .locks()
            .iter()
            .find(|l| (l.cx, l.cz, l.level, l.loc) == (x, z, 0, loc))
            .copied()
    };
    let metal = content.item_index("item.door_metal").unwrap();
    let door_item = |h: &Harness, x: u16, z: u16, loc: u8| {
        let w = &h.shard.world;
        w.deploys
            .find(x, z, 0, loc)
            .map(|d| w.deploy.defs[usize::from(d.row)].item)
    };
    let mut done = false;
    for _ in 0..120_000 {
        h.step();
        let Some(plan) = h.bot.builder().plan() else {
            continue;
        };
        let (cx, cz) = (plan.cx, plan.cz);
        let armed = [
            (cx + 1, cz + 1, LOC_EDGE_XLO),
            (cx + 1, cz + 1, LOC_EDGE_ZLO),
            (cx, cz, LOC_PLANE),
        ]
        .iter()
        .all(|&(x, z, loc)| lock_at(&h, x, z, loc).is_some_and(|l| l.locked));
        // The front door in metal at the least: the inner one waits for the
        // spare once the gear has had its fragments.
        let metal_door = door_item(&h, cx + 1, cz + 1, LOC_EDGE_XLO) == Some(metal);
        if armed && metal_door && h.bot.builder().at_checkpoint() {
            done = true;
            break;
        }
    }
    if !done {
        if let Some(plan) = h.bot.builder().plan() {
            let (cx, cz) = (plan.cx, plan.cz);
            for (x, z, loc) in [
                (cx + 1, cz + 1, LOC_EDGE_XLO),
                (cx + 1, cz + 1, LOC_EDGE_ZLO),
                (cx, cz, LOC_PLANE),
            ] {
                eprintln!(
                    "at {loc}: {:?} item {:?} lock {:?}",
                    h.shard.world.deploys.find(x, z, 0, loc),
                    door_item(&h, x, z, loc),
                    lock_at(&h, x, z, loc).map(|l| (l.locked, l.auth.contains(ID)))
                );
            }
        }
    }
    assert!(
        done,
        "the locks and metal doors never stood: {:?} {:?} {}",
        h.bot.builder().survey(),
        h.bot.builder().stats,
        h.explain()
    );
    let plan = h.bot.builder().plan().unwrap();
    let (cx, cz) = (plan.cx, plan.cz);
    for (x, z, loc) in [
        (cx + 1, cz + 1, LOC_EDGE_XLO),
        (cx + 1, cz + 1, LOC_EDGE_ZLO),
        (cx, cz, LOC_PLANE),
    ] {
        let l = lock_at(&h, x, z, loc).unwrap();
        assert_eq!(l.code, code.wire(), "its own code on the lock at {loc}");
        assert!(
            l.locked && l.auth.contains(ID),
            "armed, and it is known there"
        );
    }
    let stats = h.bot.builder().stats;
    assert!(
        stats.locks >= 3 && stats.codes >= 3 && stats.swapped >= 1,
        "three locks armed and the front door swapped: {stats:?}"
    );
    for verb in ["access", "demolish", "deploy"] {
        assert!(h.verbs.contains(verb), "{verb} never sent: {:?}", h.verbs);
    }
    assert!(
        h.held_checks as u64 >= stats.deployed,
        "every lock and door went out with its item in hand"
    );
    assert_eq!(
        h.heap_ops, 0,
        "the agent's frame loop touched the allocator"
    );

    // 2. A stranger outside the front door: the door does not swing for
    //    him, and a wrong code shocks him and lets nobody in. The owner is
    //    logging off in its room meanwhile, doors shut: none of its walks
    //    through them.
    h.bot.set_deadline(Some(0));
    assert!(
        h.until(90 * TICK_HZ, |b| b.stats.phase
            == server::explorer::Phase::LoggingOff),
        "it never settled in its room: {}",
        h.explain()
    );
    let shut = |h: &Harness| {
        h.shard
            .world
            .deploys
            .find(cx + 1, cz + 1, 0, LOC_EDGE_XLO)
            .is_some_and(|d| !d.open)
    };
    h.with_puppet();
    h.step();
    let (fx, fz) = (
        f32::from(cx + 1) * BUILD_CELL_M,
        f32::from(cz + 1) * BUILD_CELL_M,
    );
    let at_door = sim_core::movement::Body::at(SEED, &h.shard.world.haven, fx - 1.0, fz + 1.5);
    h.stage_puppet(move |p| p.body = at_door);
    let front = (cx + 1, cz + 1, 0u8, LOC_EDGE_XLO);
    let puppet_does = |h: &mut Harness, act: ActionMsg| {
        // One action at a time, as the lane takes them.
        assert!(h.until(5 * TICK_HZ, |_| false) || h.shard.wants_action(1));
        for core in [&mut h.shard, &mut h.replay] {
            core.push_action(1, act);
        }
        h.until(TICK_HZ / 2, |_| false);
    };
    assert!(shut(&h), "the front door stands shut");
    puppet_does(
        &mut h,
        ActionMsg::Use {
            cx: front.0,
            cz: front.1,
            level: front.2,
            loc: front.3,
        },
    );
    assert!(shut(&h), "a stranger opened the locked front door");
    let wrong = (code.wire() + 1) % (sim_core::lock::CODE_MAX + 1);
    let hp = |h: &Harness| {
        h.shard
            .world
            .players
            .iter()
            .find(|p| p.active && p.id == PUPPET)
            .map(|p| p.hp)
    };
    let before = hp(&h);
    puppet_does(
        &mut h,
        ActionMsg::Access {
            cx: front.0,
            cz: front.1,
            level: front.2,
            loc: front.3,
            op: ACCESS_OP_ENTER,
            code: wrong,
        },
    );
    assert!(hp(&h) < before, "a wrong code shocks");
    assert!(!h
        .shard
        .world
        .deploys
        .lock_passes(front.0, front.1, front.2, front.3, PUPPET));
    puppet_does(
        &mut h,
        ActionMsg::Use {
            cx: front.0,
            cz: front.1,
            level: front.2,
            loc: front.3,
        },
    );
    assert!(shut(&h), "the wrong code let a stranger in");

    // 3. Somebody who knows the code sets it again: the lock forgets
    //    everyone else. The owner, sent home from outside, is refused at its
    //    own front door, enters its code and is let through.
    for op in [ACCESS_OP_ENTER, ACCESS_OP_SET_CODE] {
        puppet_does(
            &mut h,
            ActionMsg::Access {
                cx: front.0,
                cz: front.1,
                level: front.2,
                loc: front.3,
                op,
                code: code.wire(),
            },
        );
    }
    assert!(!h
        .shard
        .world
        .deploys
        .lock_passes(front.0, front.1, front.2, front.3, ID));
    let away = sim_core::movement::Body::at(SEED, &h.shard.world.haven, fx - 25.0, fz + 25.0);
    h.stage_puppet(move |p| p.body = away);
    let out = sim_core::movement::Body::at(SEED, &h.shard.world.haven, fx - 1.5, fz + 1.5);
    h.stage(move |p| p.body = out);
    h.step();
    h.bot.set_deadline(Some(h.tick + 60 * TICK_HZ));
    let entered = h.bot.builder().stats.entered;
    let mut home = false;
    for _ in 0..50 * TICK_HZ {
        h.step();
        let me = h.view.get(ID).copied().unwrap();
        if h.bot.builder().stats.entered > entered
            && h.bot.builder().region(&me) == Region::Room
            && shut(&h)
        {
            home = true;
            break;
        }
    }
    assert!(
        home,
        "the owner never got back in through its own locked door: {:?} {}",
        h.bot.builder().stats,
        h.explain()
    );
    assert!(h
        .shard
        .world
        .deploys
        .lock_passes(front.0, front.1, front.2, front.3, ID));

    // 4. The code changed under it: its own code is wrong now. Sent home
    //    from outside, it tries the code a bounded number of times, gives
    //    the walk up and does not stand there shocking itself into the
    //    lockout.
    let near = sim_core::movement::Body::at(SEED, &h.shard.world.haven, fx - 1.0, fz + 1.5);
    h.stage_puppet(move |p| p.body = near);
    h.step();
    let changed = (code.wire() + 7) % (sim_core::lock::CODE_MAX + 1);
    for (op, code) in [
        (ACCESS_OP_ENTER, code.wire()),
        (ACCESS_OP_SET_CODE, changed),
    ] {
        puppet_does(
            &mut h,
            ActionMsg::Access {
                cx: front.0,
                cz: front.1,
                level: front.2,
                loc: front.3,
                op,
                code,
            },
        );
    }
    assert!(!h
        .shard
        .world
        .deploys
        .lock_passes(front.0, front.1, front.2, front.3, ID));
    h.stage_puppet(move |p| p.body = away);
    h.stage(move |p| p.body = out);
    h.step();
    let mut gave_up = false;
    for _ in 0..90 * TICK_HZ {
        h.step();
        gave_up |=
            h.bot.memory().last.is_some_and(|r| {
                r.goal == Goal::GoHome && r.outcome == Outcome::Failed(Why::Refused)
            });
    }
    // A few codes a walk, and a minute between walks: two walks in the
    // ninety seconds, and the keypad never shut.
    let l = lock_at(&h, front.0, front.1, front.3).unwrap();
    assert!(
        gave_up && l.misses <= 2 * server::agent::build::MAX_FAILS && l.shut_until == 0,
        "the walk home at a lock that no longer takes its code never gave up ({} misses): \
         {:?} {}",
        l.misses,
        h.bot.builder().stats,
        h.explain()
    );
    assert!(u64::from(h.use_checks) >= h.bot.builder().stats.uses);
    let expected: BTreeSet<&str> = EXPECTED_VERBS.into_iter().collect();
    assert!(h.verbs.is_subset(&expected));
    assert_eq!(
        h.heap_ops, 0,
        "the agent's frame loop touched the allocator"
    );
    println!("{:?} {}", h.bot.builder().stats, h.explain());
}

/// Past the base, the gear: with junk, cloth, rope and fragments staged, it
/// learns the metal tools and the medkit at its bench's tree (the unlock
/// verb, paid in junk), makes the metal tools, a crossbow and metal arrows,
/// burlap and medkits there, wears the burlap (a move into the wear slots
/// by the catalog's slot) and puts the better arms and the medkits on its
/// belt between goals.
#[test]
fn a_survivor_learns_makes_and_wears_its_gear() {
    let mind = Mind::inline(Scripted::default(), MindConfig::default()).unwrap();
    let mut h = Harness::with_mind(false, scene(), mind);
    let content = common::content();
    let stack = |id: &str, count: u16| sim_core::gather::ItemStack {
        count,
        ..common::stack(&content, id)
    };
    h.until(5, |_| false);
    let mut kit = vec![stack("item.wood", 1000); 6];
    kit.extend([stack("item.stone", 1000); 6]);
    kit.extend([
        stack("item.cloth", 400),
        stack("item.metal_frags", 1000),
        stack("item.metal_frags", 500),
        stack("item.lowgrade", 100),
        stack("item.junk", 200),
        stack("item.rope", 4),
    ]);
    let (hatchet, pickaxe) = (
        stack("item.hatchet_stone", 1),
        stack("item.pickaxe_stone", 1),
    );
    h.stage(|p| {
        p.inv[1] = hatchet;
        p.inv[2] = pickaxe;
        for (i, s) in kit.iter().enumerate() {
            p.inv[HOTBAR_SLOTS + i] = *s;
        }
    });
    let id = |name: &str| content.item_index(name).unwrap();
    let (hood, tunic, medkit) = (
        id("item.armor_burlap_head"),
        id("item.armor_burlap_body"),
        id("item.medkit"),
    );
    let dressed = h.until(200_000, |b| {
        let Some(core) = b.core() else {
            return false;
        };
        let worn = |item| core.worn.iter().any(|s| s.count > 0 && s.item == item);
        let belted = |item| {
            core.inv[..HOTBAR_SLOTS]
                .iter()
                .any(|s| s.count > 0 && s.item == item)
        };
        worn(hood) && worn(tunic) && belted(medkit)
    });
    assert!(
        dressed,
        "never in burlap with a medkit on the belt: {:?} {:?} {:?} {}",
        h.bot.builder(),
        h.bot.builder().survey(),
        h.bot.builder().stats,
        h.explain()
    );
    // Learned at the tree, not found: the blueprints are known.
    let recipes = server::net::bake_all(&content).unwrap().craft;
    let recipe_of = |item: u16| {
        (0..usize::from(recipes.recipe_count))
            .find(|&r| recipes.recipes[r].output == item)
            .unwrap() as u16
    };
    let me = h.me();
    for item in ["item.hatchet_metal", "item.pickaxe_metal", "item.medkit"] {
        assert!(
            sim_core::research::knows(me.known, recipe_of(id(item))),
            "{item} not learned"
        );
    }
    // Worn by the wear slots, and what it made is on the body: the metal
    // tools and the crossbow on the belt, its arrows in the pack.
    let worn = |item| me.worn.iter().any(|s| s.count > 0 && s.item == item);
    assert!(worn(hood) && worn(tunic));
    let belted = |item: &str| {
        me.inv[..HOTBAR_SLOTS]
            .iter()
            .any(|s| s.count > 0 && s.item == id(item))
    };
    for item in [
        "item.hatchet_metal",
        "item.pickaxe_metal",
        "item.crossbow",
        "item.medkit",
    ] {
        assert!(belted(item), "{item} not on the belt: {}", h.explain());
    }
    assert!(
        units_of(me, id("item.arrow_metal")) >= 20,
        "{}",
        h.explain()
    );
    let stats = h.bot.builder().stats;
    assert!(stats.learned >= 3 && stats.made >= 5, "{stats:?}");
    assert!(h.bot.stats.dressed >= 2, "{:?}", h.bot.stats);
    for verb in ["unlock", "craft", "move"] {
        assert!(h.verbs.contains(verb), "{verb} never sent: {:?}", h.verbs);
    }
    let expected: BTreeSet<&str> = EXPECTED_VERBS.into_iter().collect();
    assert!(h.verbs.is_subset(&expected));
    assert_eq!(
        h.heap_ops, 0,
        "the agent's frame loop touched the allocator"
    );
    println!("{stats:?} {}", h.explain());
}

/// The gear past the first bench, with what it costs staged: the second
/// bench on its stone foundation behind the core, gunpowder made at it,
/// the pistol round, the revolver and the roadsign vest learned at that
/// bench's tree (tier two: `E` on the second bench, which the harness
/// checks at every unlock), made, and the vest worn.
#[test]
fn a_survivor_builds_its_second_bench_and_learns_the_revolver_and_vest() {
    use server::agent::build::{Milestone, ANNEX};
    use sim_core::build::LOC_PLANE;
    use sim_core::deploy::ARCH_WORKBENCH2;

    let mind = Mind::inline(Scripted::default(), MindConfig::default()).unwrap();
    let mut h = Harness::with_mind(false, scene(), mind);
    let content = common::content();
    let stack = |id: &str, count: u16| sim_core::gather::ItemStack {
        count,
        ..common::stack(&content, id)
    };
    h.until(5, |_| false);
    let mut kit = vec![stack("item.wood", 1000); 6];
    kit.extend([stack("item.stone", 1000); 6]);
    kit.extend([
        stack("item.cloth", 400),
        stack("item.metal_frags", 1000),
        stack("item.metal_frags", 1000),
        stack("item.metal_frags", 1000),
        stack("item.lowgrade", 100),
        stack("item.junk", 1000),
        stack("item.rope", 4),
        stack("item.gears", 4),
        stack("item.tarp", 4),
        stack("item.charcoal", 200),
        stack("item.sulfur", 150),
    ]);
    let (hatchet, pickaxe) = (
        stack("item.hatchet_stone", 1),
        stack("item.pickaxe_stone", 1),
    );
    h.stage(|p| {
        p.inv[1] = hatchet;
        p.inv[2] = pickaxe;
        for (i, s) in kit.iter().enumerate() {
            p.inv[HOTBAR_SLOTS + i] = *s;
        }
    });
    let id = |name: &str| content.item_index(name).unwrap();
    let vest = id("item.armor_roadsign_body");
    let done = h.until(400_000, |b| {
        b.builder().survey().milestone == Milestone::Done
            && b.core()
                .is_some_and(|c| c.worn.iter().any(|s| s.count > 0 && s.item == vest))
    });
    assert!(
        done,
        "never past the roadsign vest: {:?} {:?} {}",
        h.bot.builder().survey(),
        h.bot.builder().stats,
        h.explain()
    );
    let plan = h.bot.builder().plan().unwrap();
    let (ax, az) = (
        plan.cx.checked_add_signed(i16::from(ANNEX.0)).unwrap(),
        plan.cz.checked_add_signed(i16::from(ANNEX.1)).unwrap(),
    );
    {
        let w = &h.shard.world;
        let bench2 = w
            .deploys
            .find(ax, az, 0, LOC_PLANE)
            .filter(|d| d.owner == ID)
            .map(|d| w.deploy.defs[usize::from(d.row)].arch);
        assert_eq!(bench2, Some(ARCH_WORKBENCH2), "the second bench behind");
    }
    let recipes = server::net::bake_all(&content).unwrap().craft;
    let recipe_of = |item: u16| {
        (0..usize::from(recipes.recipe_count))
            .find(|&r| recipes.recipes[r].output == item)
            .unwrap() as u16
    };
    let me = h.me();
    for item in [
        "item.pistol_ammo",
        "item.revolver",
        "item.armor_roadsign_body",
    ] {
        assert!(
            sim_core::research::knows(me.known, recipe_of(id(item))),
            "{item} not learned"
        );
    }
    assert!(
        units_of(me, id("item.revolver")) >= 1 && units_of(me, id("item.pistol_ammo")) >= 1,
        "{}",
        h.explain()
    );
    let stats = h.bot.builder().stats;
    assert!(stats.learned >= 6, "{stats:?}");
    assert!(
        h.pick_checks as u64 >= stats.learned,
        "every unlock went out with its bench under `E`"
    );
    let expected: BTreeSet<&str> = EXPECTED_VERBS.into_iter().collect();
    assert!(h.verbs.is_subset(&expected));
    assert_eq!(
        h.heap_ops, 0,
        "the agent's frame loop touched the allocator"
    );
    println!("{stats:?} {}", h.explain());
}

/// Gears, rope and tarp with a recycler in the pack: it puts the recycler
/// down, feeds it through its panel, switches it on, and takes off the
/// junk, fragments and cloth they come apart into. The gear its base has
/// still to make (the crossbow's rope, the revolver's gears, the vest's
/// tarp) is left whole.
#[test]
fn a_survivor_recycles_its_salvage() {
    let mut h = Harness::new(false);
    let content = common::content();
    let stack = |id: &str, count: u16| sim_core::gather::ItemStack {
        count,
        ..common::stack(&content, id)
    };
    h.until(5, |_| false);
    let staged = [
        stack("item.recycler", 1),
        stack("item.gears", 5),
        stack("item.rope", 3),
        stack("item.tarp", 3),
    ];
    h.stage(|p| {
        for (i, s) in staged.iter().enumerate() {
            p.inv[HOTBAR_SLOTS + i] = *s;
        }
    });
    let id = |name: &str| content.item_index(name).unwrap();
    let done = h.until(3 * 60 * TICK_HZ, |b| {
        b.oven_stats.recycled >= 3 && b.goal() != Some(Goal::Recycle)
    });
    assert!(
        done,
        "nothing was recycled: {:?} {}",
        h.bot.oven_stats,
        h.explain()
    );
    let me = h.me();
    // Two of each kept for the gear, the rest taken apart.
    assert_eq!(units_of(me, id("item.gears")), 2);
    assert_eq!(units_of(me, id("item.rope")), 2);
    assert_eq!(units_of(me, id("item.tarp")), 2);
    assert_eq!(units_of(me, id("item.junk")), 3 * 12);
    assert_eq!(units_of(me, id("item.metal_frags")), 3 * 15);
    assert_eq!(units_of(me, id("item.cloth")), 18 + 60);
    // Nothing left to take apart: it is not sent back to the recycler.
    assert!(!h.until(30 * TICK_HZ, |b| b.goal() == Some(Goal::Recycle)));
    assert_eq!(h.bot.oven_stats.placed, 1, "{:?}", h.bot.oven_stats);
    for verb in ["deploy", "container", "move", "use"] {
        assert!(h.verbs.contains(verb), "{verb} never sent: {:?}", h.verbs);
    }
    assert!(h.pick_checks >= 1 && h.panel_checks >= 5 && h.use_checks >= 2);
    let expected: BTreeSet<&str> = EXPECTED_VERBS.into_iter().collect();
    assert!(h.verbs.is_subset(&expected));
    assert_eq!(
        h.heap_ops, 0,
        "the agent's frame loop touched the allocator"
    );
    println!("{:?} {}", h.bot.oven_stats, h.explain());
}

/// Raw meat in the pack is cooked before it is eaten: a fire pit crafted
/// and put down in front of it, opened (`E` on the fire), wood and a piece
/// of meat a slot laid in with its panel open, lit (`C` on the same fire),
/// each piece taken off as soon as it is done, the fire put out. Hungry,
/// it eats the cooked meat.
#[test]
fn a_survivor_cooks_its_meat_and_eats_it() {
    let mut h = Harness::new(false);
    let content = common::content();
    let stack = |id: &str, count: u16| sim_core::gather::ItemStack {
        count,
        ..common::stack(&content, id)
    };
    h.until(5, |_| false);
    let (meat, wood) = (stack("item.raw_meat", 10), stack("item.wood", 400));
    h.stage(|p| {
        p.inv[HOTBAR_SLOTS] = meat;
        p.inv[HOTBAR_SLOTS + 1] = wood;
    });
    let id = |name: &str| content.item_index(name).unwrap();
    let (raw, cooked, burnt) = (
        id("item.raw_meat"),
        id("item.cooked_meat"),
        id("item.burnt_meat"),
    );
    let done = h.until(4 * 60 * TICK_HZ, |b| {
        b.oven_stats.cooked >= 10 && b.goal() != Some(Goal::Cook)
    });
    assert!(
        done,
        "the meat was not cooked: {:?} {}",
        h.bot.oven_stats,
        h.explain()
    );
    let me = h.me();
    assert_eq!(units_of(me, raw), 0, "all of it went on the fire");
    assert_eq!(units_of(me, cooked), 10, "every piece taken off cooked");
    assert_eq!(units_of(me, burnt), 0, "none left on to burn");
    assert_eq!(h.bot.oven_stats.placed, 1, "{:?}", h.bot.oven_stats);
    // Its fire stands, put out, and holds no meat.
    let w = &h.shard.world;
    let fire_row = w
        .deploy
        .defs
        .iter()
        .position(|d| d.item == id("item.fire_pit"))
        .unwrap();
    let fire = w
        .deploys
        .entries()
        .iter()
        .find(|d| usize::from(d.row) == fire_row && d.owner == ID)
        .copied()
        .expect("its fire pit");
    let key = sim_core::deploy::box_key(fire.cx, fire.cz, 0);
    let held = w
        .deploys
        .boxes()
        .iter()
        .find(|b| sim_core::deploy::box_key(b.cx, b.cz, b.level) == key)
        .map(|b| b.items)
        .unwrap();
    assert!(held
        .iter()
        .all(|s| s.count == 0 || (s.item != raw && s.item != cooked)));
    for verb in ["craft", "deploy", "container", "move", "use"] {
        assert!(h.verbs.contains(verb), "{verb} never sent: {:?}", h.verbs);
    }
    assert!(h.pick_checks >= 1 && h.panel_checks >= 10 && h.use_checks >= 2);
    // Hungry: the cooked meat is what it eats.
    h.stage(|p| p.food = 50);
    let ate = h.until(60 * TICK_HZ, |b| b.stats.eaten > 0);
    assert!(ate, "never ate: {}", h.explain());
    assert!(units_of(h.me(), cooked) < 10, "it ate the cooked meat");
    let expected: BTreeSet<&str> = EXPECTED_VERBS.into_iter().collect();
    assert!(h.verbs.is_subset(&expected));
    assert_eq!(
        h.heap_ops, 0,
        "the agent's frame loop touched the allocator"
    );
    println!("{:?} {}", h.bot.oven_stats, h.explain());
}

impl Harness {
    /// The puppet stands at (`x`, `z`) facing `yaw` with `weapon` in hand,
    /// swinging it this tick if `swing`. Scene staging in both shards.
    fn puppet_at(
        &mut self,
        x: f32,
        z: f32,
        yaw: u16,
        weapon: sim_core::gather::ItemStack,
        swing: bool,
    ) {
        for core in [&mut self.shard, &mut self.replay] {
            let haven = core.world.haven;
            let Some(p) = core
                .world
                .players
                .iter_mut()
                .find(|p| p.active && p.id == self.puppet_id)
            else {
                return;
            };
            p.body = sim_core::movement::Body::at(SEED, &haven, x, z);
            p.inv[0] = weapon;
        }
        let mut dg = InputDatagram::new(0, 0, sim_core::limits::INTERP_DELAY_TICKS);
        dg.push(sim_core::input::InputFrame {
            seq: self.tick as u16,
            buttons: if swing { BTN_PRIMARY } else { 0 },
            yaw,
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

    /// The puppet sends this action, once its lane takes one; the agent
    /// plays on meanwhile.
    fn puppet_act(&mut self, msg: ActionMsg) {
        for _ in 0..10 * TICK_HZ {
            if self.shard.wants_action(self.puppet_slot)
                && self.replay.wants_action(self.puppet_slot)
            {
                break;
            }
            self.step();
        }
        let bytes = {
            let mut buf = [0u8; protocol::MAX_STREAM_MSG_BYTES];
            let len = encode_puppet(&msg, &mut buf);
            buf[..len].to_vec()
        };
        self.shard
            .push_action(self.puppet_slot, decode_action(&bytes).unwrap());
        self.replay
            .push_action(self.puppet_slot, decode_action(&bytes).unwrap());
        // Its own pace, as a person's hands have one.
        for _ in 0..8 {
            self.step();
        }
    }
}

/// The wire bytes of the puppet's building actions.
fn encode_puppet(msg: &ActionMsg, buf: &mut [u8]) -> usize {
    match *msg {
        ActionMsg::Place {
            row,
            cx,
            cz,
            level,
            loc,
            ..
        } => protocol::encode_action_place(row, cx, cz, level, loc, false, 0, buf),
        ActionMsg::Deploy {
            row,
            cx,
            cz,
            level,
            loc,
        } => protocol::encode_action_deploy(row, cx, cz, level, loc, buf),
        ActionMsg::Upgrade {
            cx,
            cz,
            level,
            loc,
            material,
        } => protocol::encode_action_upgrade(cx, cz, level, loc, material, buf),
        _ => panic!("the puppet builds, and only builds: {msg:?}"),
    }
    .unwrap()
}

/// The defence test's decision source: home when it is called back or
/// damaged; the base's first two milestones; otherwise off exploring.
struct HomeGuard;

impl server::mind::DecisionSource for HomeGuard {
    fn kind(&self) -> server::mind::SourceKind {
        server::mind::SourceKind::Scripted
    }

    fn decide(&mut self, s: &server::mind::Summary) -> Result<server::mind::Choice, String> {
        use server::mind::Milestone;
        let goal = if s.offers(Goal::Defend) {
            Goal::Defend
        } else if s.milestone <= Milestone::Doors && s.offers(Goal::Build) {
            Goal::Build
        } else {
            Goal::Explore
        };
        Ok(server::mind::Choice {
            goal,
            confidence: 1.0,
            reason: server::mind::Reason::EMPTY,
            input_tokens: 0,
            output_tokens: 0,
        })
    }
}

/// Its base built to the doors, the body off exploring, a stranger takes a
/// hatchet to a wall of its core from outside and goes. The alarm calls it
/// home: in through its doors, a wait for quiet, then a hammer crafted and
/// in hand and the wall mended from where `R` takes it, and the doors shut
/// behind it.
#[test]
fn a_survivor_comes_home_and_mends_the_wall_a_stranger_struck() {
    use server::agent::build::{Milestone, Region};
    use sim_core::build::{BUILD_CELL_M, LOC_EDGE_XLO, LOC_EDGE_ZLO};

    let mind = Mind::inline(HomeGuard, MindConfig::default()).unwrap();
    let mut h = Harness::with_mind(false, scene(), mind);
    let content = common::content();
    let stack = |id: &str, count: u16| sim_core::gather::ItemStack {
        count,
        ..common::stack(&content, id)
    };
    h.until(5, |_| false);
    let kit = [
        stack("item.wood", 1000),
        stack("item.wood", 1000),
        stack("item.wood", 1000),
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
            p.inv[HOTBAR_SLOTS + i] = *s;
        }
    });

    // 1. The shell, the doors, the bag and the box.
    let built = h.until(20_000, |b| {
        b.builder().survey().milestone > Milestone::Doors
    });
    assert!(
        built,
        "the base stopped at {:?}: {}",
        h.bot.builder().survey(),
        h.explain()
    );
    let plan = h.bot.builder().plan().expect("a plot");
    let stand = h.bot.builder().stand().unwrap();
    // 2. Off exploring, away from home.
    let q = sim_core::movement::POS_XZ_Q;
    let away = |h: &Harness| {
        let me = h.view.get(ID).copied().unwrap();
        (me.qx as f32 * q - stand[0]).hypot(me.qz as f32 * q - stand[1])
    };
    let mut gone = false;
    for _ in 0..6_000 {
        h.step();
        if away(&h) > 25.0 && h.bot.goal() == Some(Goal::Explore) {
            gone = true;
            break;
        }
    }
    assert!(gone, "it never left home: {}", h.explain());

    // 3. The stranger strikes the south wall of the core from outside.
    h.with_puppet();
    h.until(2, |_| false);
    let wall = (plan.cx, plan.cz, 0u8, LOC_EDGE_ZLO);
    let hp = |h: &Harness| {
        h.shard
            .world
            .pieces
            .find(wall.0, wall.1, wall.2, wall.3)
            .map(|r| r.hp)
    };
    let full = hp(&h).expect("the wall stands");
    let (wx, wz) = (
        f32::from(plan.cx) * BUILD_CELL_M + 1.5,
        f32::from(plan.cz) * BUILD_CELL_M - 0.8,
    );
    for _ in 0..4 * TICK_HZ {
        h.puppet_at(wx, wz, 0, hatchet, true);
        h.step();
    }
    let struck = hp(&h).expect("the wall stands");
    assert!(struck < full, "the stranger's blows never landed");
    // ...and goes.
    h.puppet_at(wx + 300.0, wz - 300.0, 0, hatchet, false);
    let (repairs, alarmed) = (h.bot.defend_stats.repairs, h.bot.defend_stats.alarmed);

    // 4. Home, quiet, the wall mended, the doors shut. Too far off to see
    // the wall's damage: what calls it home is the alarm.
    let called = h.until(60 * TICK_HZ, |b| b.goal() == Some(Goal::Defend));
    assert!(called, "the alarm never called it home: {}", h.explain());
    assert!(
        away(&h) > server::agent::defend::MEND_SIGHT_M,
        "called home from where it could see the wall"
    );
    assert_eq!(h.bot.defend_stats.alarmed, alarmed + 1, "not on the alarm");
    let mended = h.until(DEFEND_TICKS, |b| {
        b.defend_stats.repairs > repairs && b.goal() != Some(Goal::Defend)
    });
    assert!(
        mended,
        "the wall was never mended: {:?} {:?} {}",
        h.bot.defend_stats,
        hp(&h),
        h.explain()
    );
    assert_eq!(hp(&h), Some(full), "mended whole");
    assert!(h.verbs.contains("repair") && h.verbs.contains("use"));
    let w = &h.shard.world;
    for (x, z, loc) in [
        (plan.cx + 1, plan.cz + 1, LOC_EDGE_XLO),
        (plan.cx + 1, plan.cz + 1, LOC_EDGE_ZLO),
    ] {
        let door = w.deploys.find(x, z, 0, loc).expect("its doors stand");
        assert!(!door.open, "a door left open at {x},{z},{loc}");
    }
    let me = h.view.get(ID).copied().unwrap();
    assert_ne!(h.bot.builder().region(&me), Region::Outside, "home, inside");
    assert!(h.held_checks >= 1 && h.pick_checks >= 1);
    // Seen through: the alarm that called it is spent, and it does not
    // walk out and back in again while that alarm would still be fresh.
    let defences = h.bot.defend_stats.defences;
    h.until(server::agent::defend::DEFEND_ALARM_TICKS, |_| false);
    assert_eq!(h.bot.defend_stats.defences, defences, "{}", h.explain());
    let expected: BTreeSet<&str> = EXPECTED_VERBS.into_iter().collect();
    assert!(h.verbs.is_subset(&expected));
    assert_eq!(
        h.heap_ops, 0,
        "the agent's frame loop touched the allocator"
    );
    println!("{:?} {}", h.bot.defend_stats, h.explain());
}

/// How long a defence may take in the lockstep run: the walk home, the
/// wait for quiet, a hammer, the repair, the doors.
const DEFEND_TICKS: u32 = 4 * 60 * TICK_HZ;

/// The raid test's decision source: a raid whenever one is on offer, else
/// stand and look round.
struct RaidWhenOffered;

impl server::mind::DecisionSource for RaidWhenOffered {
    fn kind(&self) -> server::mind::SourceKind {
        server::mind::SourceKind::Scripted
    }

    fn decide(&mut self, s: &server::mind::Summary) -> Result<server::mind::Choice, String> {
        let goal = if s.offers(Goal::Raid) {
            Goal::Raid
        } else {
            Goal::Wait
        };
        Ok(server::mind::Choice {
            goal,
            confidence: 1.0,
            reason: server::mind::Reason::EMPTY,
            input_tokens: 0,
            output_tokens: 0,
        })
    }
}

/// A stranger's base four cells north of `at`: two cells, a wall round
/// them (floor and walls in stone if `stone`, else left in twig), a wooden
/// door in the doorway of the south face, the face towards `at`, a
/// cupboard, and a box of 300 fragments. Built through the puppet's own
/// actions, the box filled and the stranger sent far away by scene
/// staging. Returns the door's cell.
fn stage_strangers_base(h: &mut Harness, at: (f32, f32), stone: bool) -> (u16, u16) {
    use server::population::base_rows;
    use sim_core::build::{build_cell_of, LOC_EDGE_XLO, LOC_EDGE_ZLO, LOC_PLANE, MAT_STONE};

    let content = common::content();
    let stack = |id: &str, count: u16| sim_core::gather::ItemStack {
        count,
        ..common::stack(&content, id)
    };
    let rows = base_rows(&content).unwrap();
    let (bx, bz) = (build_cell_of(at.0) as u16, build_cell_of(at.1) as u16 + 4);
    h.with_puppet();
    h.until(2, |_| false);
    let builder_at = (f32::from(bx) * 3.0 + 3.0, f32::from(bz) * 3.0 + 1.5);
    let pocket = [
        stack("item.wood", 1000),
        stack("item.wood", 1000),
        stack("item.stone", 1000),
        stack("item.stone", 1000),
        stack("item.stone", 1000),
        stack("item.stone", 1000),
        stack("item.hearth", 1),
        stack("item.door_wood", 1),
        stack("item.box_small", 1),
    ];
    for core in [&mut h.shard, &mut h.replay] {
        let haven = core.world.haven;
        let p = core
            .world
            .players
            .iter_mut()
            .find(|p| p.active && p.id == PUPPET)
            .unwrap();
        p.body = sim_core::movement::Body::at(SEED, &haven, builder_at.0, builder_at.1);
        for (i, s) in pocket.iter().enumerate() {
            p.inv[HOTBAR_SLOTS + i] = *s;
        }
    }
    let place = |row: u16, cx: u16, cz: u16, loc: u8| ActionMsg::Place {
        row,
        cx,
        cz,
        level: 0,
        loc,
        freehand: false,
        plate: 0,
    };
    let deploy = |row: u16, cx: u16, cz: u16, loc: u8| ActionMsg::Deploy {
        row,
        cx,
        cz,
        level: 0,
        loc,
    };
    let walls = [
        (bx, bz, LOC_EDGE_XLO, rows.wall),
        (bx + 2, bz, LOC_EDGE_XLO, rows.wall),
        (bx + 1, bz, LOC_EDGE_ZLO, rows.wall),
        (bx, bz + 1, LOC_EDGE_ZLO, rows.wall),
        (bx + 1, bz + 1, LOC_EDGE_ZLO, rows.wall),
        (bx, bz, LOC_EDGE_ZLO, rows.doorway),
    ];
    let mut acts = vec![
        place(rows.foundation, bx, bz, LOC_PLANE),
        place(rows.foundation, bx + 1, bz, LOC_PLANE),
        deploy(rows.hearth, bx, bz, LOC_PLANE),
    ];
    for &(cx, cz, loc, row) in &walls {
        acts.push(place(row, cx, cz, loc));
    }
    if stone {
        // The floor too: a twig foundation falls to the first blast beside
        // it and takes the doorway and the door down with it.
        let floor = [(bx, bz, LOC_PLANE, 0), (bx + 1, bz, LOC_PLANE, 0)];
        for &(cx, cz, loc, _) in floor.iter().chain(&walls) {
            acts.push(ActionMsg::Upgrade {
                cx,
                cz,
                level: 0,
                loc,
                material: MAT_STONE,
            });
        }
    }
    acts.push(deploy(rows.door, bx, bz, LOC_EDGE_ZLO));
    acts.push(deploy(rows.container, bx + 1, bz, LOC_PLANE));
    for a in acts {
        h.puppet_act(a);
    }
    let loot = stack("item.metal_frags", 300);
    for core in [&mut h.shard, &mut h.replay] {
        let w = &mut core.world;
        for &(cx, cz, loc, _) in &walls {
            assert!(
                w.pieces.find(cx, cz, 0, loc).is_some(),
                "a wall at {cx},{cz},{loc}"
            );
        }
        assert!(
            w.deploys.find(bx, bz, 0, LOC_EDGE_ZLO).is_some(),
            "the door"
        );
        let key = sim_core::deploy::box_key(bx + 1, bz, 0);
        let i = w.deploys.box_index(key).expect("the box");
        w.deploys.set_box_slot(i, 0, loot);
        // Nobody home.
        let haven = w.haven;
        let p = w
            .players
            .iter_mut()
            .find(|p| p.active && p.id == PUPPET)
            .unwrap();
        p.body = sim_core::movement::Body::at(SEED, &haven, at.0 + 300.0, at.1 - 300.0);
    }
    (bx, bz)
}

/// A stranger's base in front of it: two cells in stone, a wooden door on
/// the face towards it, a box of fragments inside, nobody home. With three
/// satchels in the pack it sees the door is the weak face (two satchels by
/// its hp), walks up to it, plants them one at a time, each in hand, from
/// where `X` takes the door, stands clear of each blast, walks in through
/// the doorway, opens the box with `E` and takes what it holds.
#[test]
fn a_raider_blows_the_door_of_a_stranger_s_base_and_empties_its_box() {
    use sim_core::build::LOC_EDGE_ZLO;

    let at = common::clearing(3);
    let mind = Mind::inline(RaidWhenOffered, MindConfig::default()).unwrap();
    let mut h = Harness::with_mind(false, at, mind);
    let content = common::content();
    h.until(5, |_| false);
    let satchels = sim_core::gather::ItemStack {
        count: 3,
        ..common::stack(&content, "item.satchel_charge")
    };
    h.stage(|p| p.inv[HOTBAR_SLOTS] = satchels);
    let (bx, bz) = stage_strangers_base(&mut h, at, true);
    let frags = content.item_index("item.metal_frags").unwrap();
    assert_eq!(units_of(h.me(), frags), 0);

    // The raid: on offer once the door is seen, then through it.
    let raided = h.until(3 * 60 * TICK_HZ, |b| {
        b.history
            .iter()
            .any(|r| r.goal == Goal::Raid && r.outcome == Outcome::Done)
    });

    assert!(
        raided,
        "no raid finished: {:?} {}",
        h.bot.bases().stats,
        h.explain()
    );
    let w = &h.shard.world;
    assert!(
        w.deploys.find(bx, bz, 0, LOC_EDGE_ZLO).is_none(),
        "the door is down"
    );
    assert_eq!(units_of(h.me(), frags), 300, "the box's fragments taken");
    // The door's 200 hp is two satchels of 125: both planted, both
    // counted, and the third still in the pack.
    let stats = h.bot.bases().stats;
    assert!(
        stats.charges == 2 && stats.breached == 1 && stats.boxes >= 1,
        "{stats:?}"
    );
    let satchel = content.item_index("item.satchel_charge").unwrap();
    assert_eq!(units_of(h.me(), satchel), 1, "one satchel left");
    for verb in ["move", "throw", "container"] {
        assert!(h.verbs.contains(verb), "{verb} never sent: {:?}", h.verbs);
    }
    assert!(h.held_checks >= 1 && h.pick_checks >= 2 && h.panel_checks >= 1);
    let expected: BTreeSet<&str> = EXPECTED_VERBS.into_iter().collect();
    assert!(h.verbs.is_subset(&expected));
    assert_eq!(
        h.heap_ops, 0,
        "the agent's frame loop touched the allocator"
    );
    println!("{stats:?} {}", h.explain());
}

/// The same base left in twig, and nothing in the pack to blow it with:
/// the numbers say a twig wall comes down to a few blows of a hatchet, even
/// on its hard face, so it takes the hatchet to the wall, walks in through
/// the gap and empties the box.
#[test]
fn a_raider_hacks_through_a_twig_wall_when_the_numbers_say_so() {
    let at = common::clearing(3);
    let mind = Mind::inline(RaidWhenOffered, MindConfig::default()).unwrap();
    let mut h = Harness::with_mind(false, at, mind);
    let content = common::content();
    h.until(5, |_| false);
    let hatchet = common::stack(&content, "item.hatchet_stone");
    h.stage(|p| p.inv[1] = hatchet);
    stage_strangers_base(&mut h, at, false);
    let frags = content.item_index("item.metal_frags").unwrap();
    let raided = h.until(3 * 60 * TICK_HZ, |b| {
        b.history
            .iter()
            .any(|r| r.goal == Goal::Raid && r.outcome == Outcome::Done)
    });
    assert!(
        raided,
        "no raid finished: {:?} {}",
        h.bot.bases().stats,
        h.explain()
    );
    assert_eq!(units_of(h.me(), frags), 300, "the box's fragments taken");
    let stats = h.bot.bases().stats;
    assert!(stats.charges == 0 && stats.breached >= 1, "{stats:?}");
    assert!(
        h.bot.bases().iter().any(|b| b.open.is_some()),
        "the hole is remembered as the way back in"
    );
    assert!(!h.verbs.contains("throw"));
    let expected: BTreeSet<&str> = EXPECTED_VERBS.into_iter().collect();
    assert!(h.verbs.is_subset(&expected));
    assert_eq!(
        h.heap_ops, 0,
        "the agent's frame loop touched the allocator"
    );
    println!("{stats:?} {}", h.explain());
}
