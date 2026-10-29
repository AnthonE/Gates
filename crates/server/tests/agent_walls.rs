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
const EXPECTED_VERBS: [&str; 28] = [
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
    "team",
    "vend",
    "arc",
    "swipe",
    "shop",
    "give",
    "drop",
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
        ActionMsg::Team { .. } => "team",
        ActionMsg::Craft { .. } => "craft",
        ActionMsg::Reskin { .. } => "reskin",
        ActionMsg::SkinsRefresh => "skins_refresh",
        ActionMsg::CraftCancel { .. } => "cancel",
        ActionMsg::CraftFastTrack { .. } => "fasttrack",
        ActionMsg::Research { .. } => "research",
        ActionMsg::Unlock { .. } => "unlock",
        ActionMsg::Consume { .. } => "consume",
        ActionMsg::Drink => "drink",
        ActionMsg::Move { .. } => "move",
        ActionMsg::Container { .. } => "container",
        ActionMsg::Respawn { .. } => "respawn",
        ActionMsg::RespawnGate => "respawn_gate",
        ActionMsg::Reload => "reload",
        ActionMsg::Unload => "unload",
        ActionMsg::Loot => "loot",
        ActionMsg::Pickup { .. } => "pickup",
        ActionMsg::Place { .. } => "place",
        ActionMsg::PlaceAt { .. } => "place_at",
        ActionMsg::PlaceFree { .. } => "place_free",
        ActionMsg::Deploy { .. } => "deploy",
        ActionMsg::Feed { .. } => "feed",
        ActionMsg::TakeStock { .. } => "take_stock",
        ActionMsg::Use { .. } => "use",
        ActionMsg::Repair { .. } => "repair",
        ActionMsg::Throw { .. } => "throw",
        ActionMsg::Access { .. } => "access",
        ActionMsg::Demolish { .. } => "demolish",
        ActionMsg::Rotate { .. } => "rotate",
        ActionMsg::Upgrade { .. } => "upgrade",
        ActionMsg::Vend { .. } => "vend",
        ActionMsg::Shop { .. } => "shop",
        ActionMsg::Wheel { .. } => "wheel",
        ActionMsg::Slots { .. } => "slots",
        ActionMsg::Sit { .. } => "sit",
        ActionMsg::Swipe { .. } => "swipe",
        ActionMsg::Pick { .. } => "pick",
        ActionMsg::Plant { .. } => "plant",
        ActionMsg::Tend { .. } => "tend",
        ActionMsg::Arc { .. } => "arc",
        ActionMsg::Drop { .. } => "drop",
        ActionMsg::Treat { .. } => "treat",
        ActionMsg::Give { .. } => "give",
        ActionMsg::RespawnAt { .. } => "respawn",
        ActionMsg::Attach { .. } => "attach",
        ActionMsg::Detach { .. } => "detach",
        ActionMsg::Wire { .. } => "wire",
        ActionMsg::Unwire { .. } => "unwire",
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

/// The `ACCESS_OP_*` names these files use in code (comments cut), read
/// off the source: which ops a side can put in an access verb.
fn access_ops_in(files: &[std::path::PathBuf]) -> BTreeSet<String> {
    let mut ops = BTreeSet::new();
    for file in files {
        for line in shipped(file).lines() {
            let code = line.split("//").next().unwrap_or("");
            for piece in code.split("ACCESS_OP_").skip(1) {
                let name: String = piece
                    .chars()
                    .take_while(|c| c.is_ascii_uppercase() || *c == '_')
                    .collect();
                if !name.is_empty() {
                    ops.insert(name);
                }
            }
        }
    }
    ops
}

/// The access ops the agent sends, `ACCESS_OP_` cut: the code at a lock,
/// the code on its own lock, and the crew's join at a bare cupboard
/// (`authorize`). The one list to extend when a lane gives it another.
const EXPECTED_ACCESS_OPS: [&str; 3] = ["CREW_JOIN", "ENTER", "SET_CODE"];

/// Wall 1 for locks and cupboards (the social verb `authorize`): every op
/// the agent puts in an access verb is one the human client's keypad or its
/// `L`/`K` keys send, read off both sources. The verbs that carry them,
/// `access` and `give`, are human verbs in [`EXPECTED_VERBS`].
#[test]
fn the_agent_sends_only_access_ops_the_human_client_sends() {
    assert!(EXPECTED_VERBS.contains(&"access") && EXPECTED_VERBS.contains(&"give"));
    let human = access_ops_in(&rs_files(&root().join("crates/client/src")));
    assert!(
        human.contains("ENTER") && human.contains("CREW_JOIN"),
        "the client source was not read: {human:?}"
    );
    let agent = access_ops_in(&agent_sources());
    assert_eq!(
        agent,
        EXPECTED_ACCESS_OPS.into_iter().map(String::from).collect(),
        "the agent's access ops changed; update EXPECTED_ACCESS_OPS and PLAYERS.md"
    );
    assert!(
        agent.is_subset(&human),
        "the agent sends access ops {:?}, which no human client call site sends",
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

/// Wall 1 for the voice: the agent speaks only through the encoder the
/// human chat box calls, and nothing it, its mind or the external source
/// ships names a server-side way to put words in the room.
#[test]
fn the_agent_speaks_only_through_the_human_clients_chat_encoder() {
    let human: String = rs_files(&root().join("crates/client/src"))
        .iter()
        .map(|f| shipped(f))
        .collect();
    assert!(
        human.contains("encode_chat("),
        "the human chat box was not read"
    );
    let agent: String = agent_sources().iter().map(|f| shipped(f)).collect();
    assert!(
        agent.contains("encode_chat("),
        "the agent speaks some other way"
    );
    let src = root().join("crates/server/src");
    let mut files = agent_sources();
    files.extend(["mind.rs", "jev.rs", "external.rs"].map(|f| src.join(f)));
    for file in files {
        let shipped = shipped(&file);
        for banned in ["encode_event_chat", "push_chat", "say_server", "KIND_CHAT"] {
            assert!(
                !shipped.contains(banned),
                "{} names {banned}: an agent says a line the way a player types one",
                file.display()
            );
        }
    }
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
    /// Chat lines the puppet's client was sent: (speaker, global, text).
    puppet_chat: Vec<(u32, bool, protocol::ChatText)>,
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
            ..sim_core::gather::ItemStack::EMPTY
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
            blight_salt: 0,
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
            puppet_chat: Vec::new(),
        }
    }

    /// The puppet types a line into its chat box, in both shards.
    fn puppet_says(&mut self, text: &str) {
        self.puppet_says_on(protocol::Channel::Local, text);
    }

    /// The same on a channel: `/t` is [`protocol::Channel::Team`].
    fn puppet_says_on(&mut self, channel: protocol::Channel, text: &str) {
        let msg = protocol::ChatMsg {
            channel,
            text: protocol::ChatText::sanitize(text.as_bytes()).unwrap(),
        };
        self.shard.push_chat(self.puppet_slot, msg);
        self.replay.push_chat(self.puppet_slot, msg);
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
    /// Light every work (`ARC.md` F1) in both shards, as a world past its
    /// fallback hours would be: what a test that stages gunpowder needs,
    /// now that the world gates it.
    fn light_works(&mut self) {
        for core in [&mut self.shard, &mut self.replay] {
            let w = &mut core.world;
            for k in 0..w.works_def.count as usize {
                w.works.w[k].state = sim_core::works::WORK_LIT;
            }
            let def = w.works_def;
            w.works.refresh(&def);
        }
    }

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
        let mut said = [0u8; protocol::MAX_STREAM_MSG_BYTES];
        let frame = self.bot.frame_at(&self.view, ID, self.tick as u16, now);
        let action = self.bot.action(&mut act);
        let say = self.bot.say(&mut said);
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
                // `E` picks, on something a lock bolts to. A crew op is `L`
                // at a bare cupboard, sent at once with no pad
                // (`render/verbs.rs` `access_aimed`): the cupboard `E` picks,
                // with no lock on it.
                ActionMsg::Access {
                    cx,
                    cz,
                    level,
                    loc,
                    op,
                    ..
                } => {
                    let pick = self.e_pick(frame.yaw);
                    let at = (pick.cx, pick.cz, pick.level, pick.loc) == (cx, cz, level, loc);
                    let human = if sim_core::deploy::op_is_crew(op) {
                        pick.verb == Verb::Hearth && !pick.has_lock && at
                    } else {
                        sim_core::deploy::lockable(pick.arch) && pick.has_lock && at
                    };
                    assert!(
                        human,
                        "the agent sent access op {op} to {cx},{cz},{level},{loc}, which `L` \
                         would not ({pick:?}), at tick {}",
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
                // A stone read and a dial turned are what `E` would take
                // from where the body stands (the nearest in reach, a dial
                // first); so is a card reader's or a lever's swipe.
                ActionMsg::Arc { op, target, arg }
                    if op == sim_core::lore::OP_READ || op == sim_core::mech::OP_TURN =>
                {
                    let core = self.bot.core().unwrap();
                    let me = self.view.get(ID).copied().unwrap();
                    let pick = client::ui::interact::resolve_lore(
                        me.qx as f32 * sim_core::movement::POS_XZ_Q,
                        me.qy as f32 * sim_core::movement::POS_Y_Q,
                        me.qz as f32 * sim_core::movement::POS_XZ_Q,
                        core.haven(),
                        &core.lore,
                    );
                    let want = if op == sim_core::lore::OP_READ {
                        (Verb::Read, u32::from(target))
                    } else {
                        (Verb::Turn, (u32::from(target) << 8) | u32::from(arg))
                    };
                    assert_eq!(
                        (pick.verb, pick.handle),
                        want,
                        "the agent pressed {msg:?}, which `E` would not take ({pick:?}), at tick {}",
                        self.tick
                    );
                    self.pick_checks += 1;
                }
                ActionMsg::Swipe { door } => {
                    let core = self.bot.core().unwrap();
                    let me = self.view.get(ID).copied().unwrap();
                    let pick = client::ui::interact::resolve_swipe(
                        me.qx as f32 * sim_core::movement::POS_XZ_Q,
                        me.qy as f32 * sim_core::movement::POS_Y_Q,
                        me.qz as f32 * sim_core::movement::POS_XZ_Q,
                        &core.haven().ziggurat,
                    );
                    assert!(
                        pick.verb == Verb::Swipe && pick.handle == u32::from(door),
                        "the agent swiped at door {door}, which `E` would not take ({pick:?}), at \
                         tick {}",
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
        // Wall 1, the voice: a line goes out through the human chat box's
        // encoder, local, and the shard reads it as any player's line.
        if let Some(len) = say {
            let msg =
                protocol::decode_chat(&said[..len]).expect("agent chat decodes as a player's");
            assert!(
                !msg.channel.is_global(),
                "an agent speaks only to the bodies near it or its team"
            );
            self.shard.push_chat(0, msg);
            self.replay.push_chat(0, msg);
        }
        if let Some(weapon) = self.puppet {
            self.puppet_step(weapon);
        }
        let (view, bot) = (&mut self.view, &mut self.bot);
        let (puppet_slot, puppet_chat) = (self.puppet_slot, &mut self.puppet_chat);
        self.shard.tick_bare(&self.stats, |lane, slot, bytes| {
            if slot == puppet_slot && lane == Lane::Event {
                if let Ok(protocol::EventMsg::Chat {
                    from,
                    channel,
                    text,
                }) = protocol::decode_event(bytes)
                {
                    puppet_chat.push((from, channel.is_global(), text));
                }
            }
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
            w.pieces.cols(),
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
    // The second scene: on this seed the first one's stone is never found
    // in the 12 000 ticks below, where the next five each craft inside 5 000.
    let at = common::scenes().nth(1).expect("a second scene");
    let mut h = Harness::at(false, at);
    // A belt already full of food, so the crafted tool lands in the pack and
    // has to be moved: what a long gather used to leave on the belt by
    // itself, until the world put stone nearer the spawn.
    h.until(5, |_| false);
    let shrooms = sim_core::gather::ItemStack {
        count: 10,
        ..common::stack(&common::content(), "item.mushrooms")
    };
    h.stage(|p| {
        for s in p.inv[..HOTBAR_SLOTS].iter_mut().filter(|s| s.count == 0) {
            *s = shrooms;
        }
    });

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
    assert!(h.bot.memory().food.feeds.any(), "a meal teaches what feeds");

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
    // 30 %: a body that never learns which food carries water fell near
    // 21 % here. At Rust's idle drain (#383) the healthy run bottoms out
    // near 74 % water and 80 % food.
    assert!(
        u32::from(food) * 10 >= u32::from(core.max_food) * 3,
        "food fell to {food}"
    );
    assert!(
        u32::from(water) * 10 >= u32::from(core.max_water) * 3,
        "water fell to {water}"
    );
    assert!(h.bot.gathered_of("Wood") > 0 && h.bot.gathered_of("Stone") > 0);
    // Not `eaten`: at Rust's idle drain half an hour never makes it hungry.
    assert!(h.bot.stats.crafted >= 2, "{}", h.explain());
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

    // 1. The shell, the doors and their key locks, the bag and box inside,
    // the roof, the stone core.
    let built = h.until(30_000, |b| {
        b.builder().survey().milestone > Milestone::Stone
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
            // Locked from the day it was hung, as a player's is.
            assert!(
                w.deploys
                    .locks()
                    .iter()
                    .any(|l| (l.cx, l.cz, l.level, l.loc) == (cx + 1, cz + 1, 0, loc)
                        && l.is_key()
                        && l.locked),
                "a key lock on the door at loc {loc}"
            );
        }
        // Roofed over the core and the stairwell, and the airlock.
        for (x, z, level, loc) in [
            (cx, cz, 2, LOC_PLANE),
            (cx + 1, cz, 2, LOC_PLANE),
            (cx + 1, cz + 1, 1, LOC_PLANE),
        ] {
            assert!(
                w.pieces.find(x, z, level, loc).is_some(),
                "a roof over {x},{z} at level {level}"
            );
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

    // 2d. Somebody tries its front door from outside: the key lock that
    //     went on with it (`build::KEY_LOCK_ITEM`) keeps them out, and the
    //     body asleep inside is not disturbed.
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
    for _ in 0..2 * TICK_HZ {
        h.step();
        assert!(shut(&h, LOC_EDGE_XLO), "a stranger opened its locked door");
    }
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
            say: server::mind::Say::Quiet,
            code: None,
        })
    }
}

/// The test's talking source, of the external kind the body lets speak: it
/// greets on its first decision and answers once when it has heard the
/// puppet. It notes what it heard where the test can read it, and decides
/// without allocating, inside the harness's counted window.
struct Talker {
    greeted: bool,
    answered: bool,
    /// Bit 0: heard the puppet's line, labelled and local. Bit 1: asked
    /// because of it (`Trigger::Heard`). Bit 2: heard its own line back.
    seen: std::sync::Arc<std::sync::atomic::AtomicU32>,
}

impl server::mind::DecisionSource for Talker {
    fn kind(&self) -> server::mind::SourceKind {
        server::mind::SourceKind::External
    }

    fn decide(&mut self, s: &server::mind::Summary) -> Result<server::mind::Choice, String> {
        use server::mind::{Say, Trigger};
        use std::sync::atomic::Ordering;
        let mut seen = 0;
        for l in s.heard.lines() {
            if l.me {
                seen |= 4;
            } else if l.from == PUPPET
                && l.channel == protocol::Channel::Local
                && !l.agent
                && l.text.as_bytes() == b"hi bot"
            {
                seen |= 1;
            }
        }
        if seen & 1 != 0 && s.trigger == Trigger::Heard {
            seen |= 2;
        }
        self.seen.fetch_or(seen, Ordering::Relaxed);
        let line = |t: &[u8]| Say::Line(protocol::ChatText::sanitize(t).unwrap().into());
        let say = if !self.greeted {
            self.greeted = true;
            line(b"hello")
        } else if seen & 1 != 0 && !self.answered {
            self.answered = true;
            line(b"hi puppet")
        } else {
            Say::Quiet
        };
        let goal = if s.offers(Goal::Wait) {
            Goal::Wait
        } else {
            s.options()[0]
        };
        Ok(server::mind::Choice {
            goal,
            confidence: 1.0,
            reason: server::mind::Reason::EMPTY,
            input_tokens: 0,
            output_tokens: 0,
            say,
            code: None,
        })
    }
}

/// Agents meet humans: the body says its line through the chat box's
/// encoder, a player beside it hears it from the agent, the player's answer
/// reaches the source labelled as a player's chat log labels it and wakes
/// it early, and the answer to that is heard too — local throughout, the
/// two shards hash-identical and the frame path allocation-free.
#[test]
fn a_survivor_speaks_locally_and_hears_who_spoke() {
    let seen = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
    let talker = Talker {
        greeted: false,
        answered: false,
        seen: seen.clone(),
    };
    let mind = Mind::inline(talker, MindConfig::default()).unwrap();
    let mut h = Harness::with_mind(false, scene(), mind);
    h.with_puppet();
    h.step();
    let body = h.me().body;
    h.stage_puppet(|p| p.body = body);
    let greeted = |h: &Harness| h.puppet_chat.iter().any(|c| c.0 == ID && !c.1);
    for _ in 0..600 {
        if greeted(&h) {
            break;
        }
        h.step();
    }
    assert!(
        greeted(&h),
        "the puppet never heard the agent: {}",
        h.explain()
    );
    assert_eq!(h.puppet_chat[0].2.as_bytes(), b"hello");
    h.puppet_says("hi bot");
    let answered = |h: &Harness| h.puppet_chat.iter().filter(|c| c.0 == ID).count() >= 2;
    for _ in 0..900 {
        if answered(&h) {
            break;
        }
        h.step();
    }
    assert!(answered(&h), "the agent never answered: {}", h.explain());
    let bits = seen.load(std::sync::atomic::Ordering::Relaxed);
    assert_eq!(bits & 1, 1, "the source never heard the puppet's line");
    assert_eq!(bits & 2, 2, "a line nearby did not wake the source early");
    assert_eq!(bits & 4, 4, "its own line never came back to it");
    assert!(h.puppet_chat.iter().all(|c| !c.1), "every line was local");
    let s = h.bot.stats;
    assert_eq!((s.said, s.say_refused, s.say_dropped), (2, 0, 0));
    assert!(s.heard >= 1 && s.echoed >= 1, "{s:?}");
    assert_eq!(h.heap_ops, 0, "the frame path allocated");
}

/// Teams (wire v108): the puppet invites the agent from beside it, the
/// scripted mind says yes through the human client's own team verb, and
/// once the puppet is forty metres off the agent goes to it — the
/// teammate's place from the team view a human teammate is sent too. Both
/// shards hash-identical, the frame path allocation-free.
#[test]
fn a_survivor_joins_a_team_and_keeps_up_with_its_teammate() {
    let mut h = Harness::new(false);
    h.with_puppet();
    h.step();
    let body = h.me().body;
    h.stage_puppet(|p| p.body = body);
    h.puppet_act(ActionMsg::Team {
        op: protocol::TEAM_OP_INVITE,
        target: ID,
    });
    let joined = |h: &Harness| h.shard.teams.team_of(ID).len() == 2;
    for _ in 0..30 * TICK_HZ {
        if joined(&h) {
            break;
        }
        h.step();
    }
    assert!(joined(&h), "the agent never joined: {}", h.explain());
    assert!(h.verbs.contains("team"), "it joined by its own verb");
    // Forty metres east, out of the way of anything it was doing.
    h.stage_puppet(|p| p.body.qx += 4_000 / 3);
    let apart = |h: &Harness| {
        let me = h.me().body;
        let mate = h
            .shard
            .world
            .players
            .iter()
            .find(|p| p.active && p.id == PUPPET)
            .unwrap()
            .body;
        ((me.qx - mate.qx) as f32 * 0.03).hypot((me.qz - mate.qz) as f32 * 0.03)
    };
    for _ in 0..120 * TICK_HZ {
        if apart(&h) <= server::explorer::FOLLOW_NEAR_M + 2.0 {
            break;
        }
        h.step();
    }
    assert!(
        apart(&h) <= server::explorer::FOLLOW_NEAR_M + 2.0,
        "the agent never caught up ({:.1} m): {}",
        apart(&h),
        h.explain()
    );
    assert_eq!(
        h.bot.stats.phase,
        server::explorer::Phase::Following,
        "it came by following, not by chance"
    );
    assert!(h
        .bot
        .history
        .iter()
        .any(|r| r.goal == Goal::JoinTeam && r.outcome == server::mind::Outcome::Done));
    assert_eq!(h.heap_ops, 0, "the frame path allocated");
}

/// A society's crewmate (`agent::society`): its founder — the puppet, an
/// agent, known by the post it holds on the guild's board — invites it and
/// it says yes, as it would to nobody else; then, carrying wood, it walks up
/// to its founder and hands the wood over with the human client's give, a
/// stack at a time, until the pack holds none.
#[test]
fn a_crewmate_joins_its_founder_and_hands_over_its_wood() {
    use server::agent::society::{post_of, Board, Kin, Role, Society, MAX_ALLIES};

    let board: &'static Board = Box::leak(Box::new(Board::new()));
    board.set_member(post_of(0, 0), PUPPET);
    let mut crew = [Kin::NONE; protocol::TEAM_MAX];
    crew[0] = Kin::member(protocol::Name::EMPTY, None, 0, 0);
    crew[1] = Kin::member(protocol::Name::new("me").unwrap(), None, 0, 1);
    let opts = server::explorer::SurvivorOpts {
        society: Some(Society {
            role: Role::Farmer,
            me: 1,
            crew,
            crew_len: 2,
            allies: [Kin::NONE; MAX_ALLIES],
            allies_len: 0,
            code: None,
            crew_no: 0,
            board: Some(board),
        }),
        ..Default::default()
    };
    let mind = Mind::inline(
        Prefer(&[Goal::JoinTeam, Goal::HandOver], None),
        MindConfig::default(),
    )
    .unwrap();
    let mut h = Harness::with_opts(false, common::clearing(3), mind, opts);
    h.with_puppet();
    for core in [&mut h.shard, &mut h.replay] {
        core.note_agent(1, true);
    }
    h.step();
    let body = h.me().body;
    h.stage_puppet(|p| p.body = body);
    h.puppet_act(ActionMsg::Team {
        op: protocol::TEAM_OP_INVITE,
        target: ID,
    });
    let joined = |h: &Harness| h.shard.teams.team_of(ID).len() == 2;
    for _ in 0..30 * TICK_HZ {
        if joined(&h) {
            break;
        }
        h.step();
    }
    assert!(joined(&h), "the crewmate never joined: {}", h.explain());

    // Its founder walks eight metres off; it is given a run's worth of wood.
    let content = common::content();
    let wood = |count: u16| sim_core::gather::ItemStack {
        count,
        ..common::stack(&content, "item.wood")
    };
    let wood_item = wood(1).item;
    h.stage_puppet(|p| p.body.qx += 800 / 3);
    h.stage(move |p| {
        p.inv[HOTBAR_SLOTS + 5] = wood(1000);
        p.inv[HOTBAR_SLOTS + 6] = wood(500);
    });
    let handed = h.until(90 * TICK_HZ, |b| {
        b.history.iter().any(|r| r.goal == Goal::HandOver)
    });
    assert!(handed, "no hand-over ended: {}", h.explain());
    let report = h
        .bot
        .history
        .iter()
        .find(|r| r.goal == Goal::HandOver)
        .unwrap();
    assert_eq!(report.outcome, Outcome::Done, "{report:?}: {}", h.explain());
    let held = |p: &sim_core::world::Player| -> u32 {
        p.inv
            .iter()
            .filter(|s| s.count > 0 && s.item == wood_item)
            .map(|s| u32::from(s.count))
            .sum()
    };
    let founder = h
        .shard
        .world
        .players
        .iter()
        .find(|p| p.active && p.id == PUPPET)
        .unwrap();
    assert_eq!(held(founder), 1500, "the founder holds the wood");
    assert_eq!(held(h.me()), 0, "the pack is empty of it");
    assert_eq!(report.gained, 1500);
    assert!(h.verbs.contains("give"), "{:?}", h.verbs);
    assert_eq!(h.heap_ops, 0, "the frame path allocated");
}

/// The give test's decision source, an external one (a give is offered to
/// the minds `go:` is): yes to a team, then `give:Wood` while it is on
/// offer, else stand.
struct GiveWood;

impl server::mind::DecisionSource for GiveWood {
    fn kind(&self) -> server::mind::SourceKind {
        server::mind::SourceKind::External
    }

    fn decide(&mut self, s: &server::mind::Summary) -> Result<server::mind::Choice, String> {
        let wood = Goal::Give(server::mind::Name::new(b"Wood").unwrap());
        let goal = [Goal::JoinTeam, wood]
            .into_iter()
            .find(|g| s.offers(*g))
            .unwrap_or(Goal::Wait);
        Ok(server::mind::Choice {
            goal,
            confidence: 1.0,
            reason: server::mind::Reason::EMPTY,
            input_tokens: 0,
            output_tokens: 0,
            say: server::mind::Say::Quiet,
            code: None,
        })
    }
}

/// `give`, the social verb (PLAYERS.md): on a team, its teammate standing
/// in hand reach, an external mind is offered `give:Wood` and takes it, and
/// the agent hands every stack of wood it carries into the teammate's pack
/// with the human client's give — `TRUST_GIVE` in the ledger, the teammate
/// the counterparty. Both shards hash-identical, the frame path
/// allocation-free.
#[test]
fn a_teammate_gives_its_wood_to_the_teammate_in_reach() {
    use sim_core::movement::POS_XZ_Q;
    use sim_core::world::TRUST_GIVE;

    let mind = Mind::inline(GiveWood, MindConfig::default()).unwrap();
    let mut h = Harness::with_mind(false, common::clearing(3), mind);
    h.with_puppet();
    h.step();
    let body = h.me().body;
    h.stage_puppet(|p| p.body = body);
    h.puppet_act(ActionMsg::Team {
        op: protocol::TEAM_OP_INVITE,
        target: ID,
    });
    let joined = |h: &Harness| h.shard.teams.team_of(ID).len() == 2;
    for _ in 0..30 * TICK_HZ {
        if joined(&h) {
            break;
        }
        h.step();
    }
    assert!(joined(&h), "the agent never joined: {}", h.explain());

    // Wood in the pack, two stacks; the teammate two metres in front of it.
    let content = common::content();
    let wood = |count: u16| sim_core::gather::ItemStack {
        count,
        ..common::stack(&content, "item.wood")
    };
    let wood_item = wood(1).item;
    h.stage(move |p| {
        p.inv[HOTBAR_SLOTS + 5] = wood(300);
        p.inv[HOTBAR_SLOTS + 6] = wood(100);
    });
    let me = h.me().body;
    let (fx, fz) = sim_core::yaw_dir(h.bot.hands().view().0);
    let haven = h.shard.world.haven;
    let (x, z) = (me.qx as f32 * POS_XZ_Q, me.qz as f32 * POS_XZ_Q);
    let mate = sim_core::movement::Body::at(SEED, &haven, x + fx * 2.0, z + fz * 2.0);
    h.stage_puppet(move |p| p.body = mate);
    let gave = |h: &Harness| {
        h.bot
            .history
            .iter()
            .any(|r| matches!(r.goal, Goal::Give(_)))
    };
    let mut logged = false;
    for _ in 0..60 * TICK_HZ {
        if gave(&h) {
            break;
        }
        h.step();
        logged |= h
            .shard
            .world
            .trust
            .rows()
            .iter()
            .any(|r| r.actor == ID && r.counterparty == PUPPET && r.verb == TRUST_GIVE);
    }
    assert!(gave(&h), "no give ended: {}", h.explain());
    let report = h
        .bot
        .history
        .iter()
        .find(|r| matches!(r.goal, Goal::Give(_)))
        .unwrap();
    assert_eq!(
        report.goal,
        Goal::Give(server::mind::Name::new(b"Wood").unwrap())
    );
    assert_eq!(report.outcome, Outcome::Done, "{report:?}: {}", h.explain());
    assert_eq!(report.gained, 400);
    let held = |p: &sim_core::world::Player| -> u32 {
        p.inv
            .iter()
            .filter(|s| s.count > 0 && s.item == wood_item)
            .map(|s| u32::from(s.count))
            .sum()
    };
    let mate = h
        .shard
        .world
        .players
        .iter()
        .find(|p| p.active && p.id == PUPPET)
        .unwrap();
    assert_eq!(held(mate), 400, "the teammate holds the wood");
    assert_eq!(held(h.me()), 0, "the pack is empty of it");
    assert!(logged, "the ledger kept no give from it to the teammate");
    assert!(h.verbs.contains("give"), "{:?}", h.verbs);
    assert_eq!(h.heap_ops, 0, "the frame path allocated");
}

/// The authorize tests' decision source: yes to a team, then `authorize`
/// once the test says (`.0`) and it is on offer, else stand; every answer
/// carries the code a teammate gave (`.1`), if one was.
struct AuthorizeWhenSaid(
    std::sync::Arc<std::sync::atomic::AtomicBool>,
    Option<server::agent::lock::LockCode>,
);

impl server::mind::DecisionSource for AuthorizeWhenSaid {
    fn kind(&self) -> server::mind::SourceKind {
        server::mind::SourceKind::Scripted
    }

    fn decide(&mut self, s: &server::mind::Summary) -> Result<server::mind::Choice, String> {
        let said = self.0.load(std::sync::atomic::Ordering::Relaxed);
        let goal = if s.offers(Goal::JoinTeam) {
            Goal::JoinTeam
        } else if said && s.offers(Goal::Authorize) {
            Goal::Authorize
        } else {
            Goal::Wait
        };
        Ok(server::mind::Choice {
            goal,
            confidence: 1.0,
            reason: server::mind::Reason::EMPTY,
            input_tokens: 0,
            output_tokens: 0,
            say: server::mind::Say::Quiet,
            code: self.1,
        })
    }
}

/// Run until an authorize ends, watching the ledger for a `TRUST_AUTH` from
/// the agent to the teammate. The report, and whether one was kept.
fn authorize_until_over(h: &mut Harness) -> (server::mind::Report, bool) {
    use sim_core::world::TRUST_AUTH;
    let over = |h: &Harness| h.bot.history.iter().any(|r| r.goal == Goal::Authorize);
    let mut logged = false;
    for _ in 0..150 * TICK_HZ {
        if over(h) {
            break;
        }
        h.step();
        logged |= h
            .shard
            .world
            .trust
            .rows()
            .iter()
            .any(|r| r.actor == ID && r.counterparty == PUPPET && r.verb == TRUST_AUTH);
    }
    assert!(over(h), "no authorize ended: {}", h.explain());
    let report = h
        .bot
        .history
        .iter()
        .find(|r| r.goal == Goal::Authorize)
        .unwrap();
    (report, logged)
}

/// `authorize`, the social verb (Rust's tool-cupboard auth): living in its
/// teammate's base, given the team's code, with a code lock bolted on the
/// cupboard, the agent walks back in from outside through the shut door and
/// enters the code at the cupboard's keypad — the human client's `L`, the
/// pad, the try — which puts it on the crew; `TRUST_AUTH` in the ledger,
/// the teammate the counterparty. Both shards hash-identical, the frame
/// path allocation-free.
#[test]
fn a_teammate_given_the_code_authorizes_on_the_team_s_cupboard() {
    use server::agent::lock::LockCode;
    use sim_core::build::{BUILD_CELL_M, LOC_PLANE};
    use sim_core::deploy::ACCESS_OP_SET_CODE;

    let given = 2580;
    let at = common::clearing(3);
    let said = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mind = Mind::inline(
        AuthorizeWhenSaid(said.clone(), Some(LockCode::given(given).unwrap())),
        MindConfig::default(),
    )
    .unwrap();
    let mut h = Harness::with_mind(false, at, mind);
    let content = common::content();
    let rows = server::population::base_rows(&content).unwrap();
    let (bx, bz) = move_in(&mut h, at);
    let (bxf, bzf) = (f32::from(bx) * BUILD_CELL_M, f32::from(bz) * BUILD_CELL_M);
    let haven = h.shard.world.haven;
    let place = move |x: f32, z: f32| sim_core::movement::Body::at(SEED, &haven, x, z);
    assert!(
        h.bot.shared().hearth().is_some(),
        "the cupboard was never seen: {}",
        h.explain()
    );
    // The teammate bolts a code lock onto the cupboard, sets the code, goes.
    let lock = sim_core::gather::ItemStack {
        count: 1,
        ..common::stack(&content, "item.lock_code")
    };
    let mate = place(bxf + 1.5, bzf + 2.2);
    h.stage_puppet(move |p| {
        p.body = mate;
        p.inv[HOTBAR_SLOTS] = lock;
    });
    h.puppet_act(ActionMsg::Deploy {
        row: rows.lock,
        cx: bx,
        cz: bz,
        level: 0,
        loc: LOC_PLANE,
        pose: sim_core::footprint::Pose::CENTRE,
    });
    h.puppet_act(ActionMsg::Access {
        cx: bx,
        cz: bz,
        level: 0,
        loc: LOC_PLANE,
        op: ACCESS_OP_SET_CODE,
        code: given,
    });
    let crew = |h: &Harness| {
        h.shard
            .world
            .deploys
            .hearths()
            .iter()
            .any(|r| (r.cx, r.cz) == (bx, bz) && r.crew.contains(ID))
    };
    assert!(!crew(&h), "the agent is not on the crew yet");
    let away = place(at.0 + 300.0, at.1 - 300.0);
    h.stage_puppet(move |p| p.body = away);
    let out = place(bxf + 1.5, bzf - 8.0);
    h.stage(move |p| p.body = out);
    h.until(TICK_HZ, |_| false);
    assert!(
        h.bot.summary(&h.view, ID).unwrap().offers(Goal::Authorize),
        "authorize is offered: {}",
        h.explain()
    );

    said.store(true, std::sync::atomic::Ordering::Relaxed);
    let (report, logged) = authorize_until_over(&mut h);
    assert_eq!(report.outcome, Outcome::Done, "{report:?}: {}", h.explain());
    assert!(crew(&h), "the code put it on the crew");
    assert!(h.bot.shared().crewed());
    assert!(logged, "the ledger kept no authorization by it");
    for verb in ["use", "access"] {
        assert!(h.verbs.contains(verb), "{verb} never sent: {:?}", h.verbs);
    }
    // On the crew, it is not offered again.
    h.until(2 * TICK_HZ, |_| false);
    assert!(!h.bot.summary(&h.view, ID).unwrap().offers(Goal::Authorize));
    let expected: BTreeSet<&str> = EXPECTED_VERBS.into_iter().collect();
    assert!(h.verbs.is_subset(&expected));
    assert_eq!(h.heap_ops, 0, "the frame path allocated");
}

/// `authorize` at a bare cupboard whose crew does not admit it (its owner
/// is on it, and no lock remembers the agent): the crew's join, `L` at a
/// cupboard with no lock, is refused with the cupboard's own word
/// (`REFUSE_D_OWNER`), the authorize fails as refused, nobody's crew moves,
/// and it is not offered again for a while.
#[test]
fn an_authorize_at_a_cupboard_whose_crew_does_not_admit_it_is_refused() {
    let at = common::clearing(3);
    let said = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mind = Mind::inline(AuthorizeWhenSaid(said.clone(), None), MindConfig::default()).unwrap();
    let mut h = Harness::with_mind(false, at, mind);
    let (bx, bz) = move_in(&mut h, at);
    assert!(
        h.bot.shared().hearth().is_some(),
        "the cupboard was never seen: {}",
        h.explain()
    );
    let crew = |h: &Harness| {
        h.shard
            .world
            .deploys
            .hearths()
            .iter()
            .find(|r| (r.cx, r.cz) == (bx, bz))
            .map(|r| (r.crew.contains(ID), r.crew.contains(PUPPET)))
    };
    assert_eq!(crew(&h), Some((false, true)), "the owner alone is on it");

    said.store(true, std::sync::atomic::Ordering::Relaxed);
    let (report, logged) = authorize_until_over(&mut h);
    assert_eq!(
        report.outcome,
        Outcome::Failed(Why::Refused),
        "{report:?}: {}",
        h.explain()
    );
    assert_eq!(crew(&h), Some((false, true)), "nobody's crew moved");
    assert!(!logged, "a refused join entrusted nothing");
    assert!(h.verbs.contains("access"), "{:?}", h.verbs);
    assert!(!h.bot.shared().crewed());
    h.until(2 * TICK_HZ, |_| false);
    assert!(
        !h.bot.summary(&h.view, ID).unwrap().offers(Goal::Authorize),
        "a refused authorize is held a while"
    );
    assert_eq!(h.heap_ops, 0, "the frame path allocated");
}

/// The free bot as a teammate, end to end through the real scripted
/// policy: a person beside it asks to team up in local chat and it
/// invites them by the human client's own verb; they say yes; on team chat
/// it answers `help` with its orders, and `follow me` makes it stay with
/// them when they walk off, nearer than the scripted catch-up would on its
/// own. Both shards hash-identical, the frame path allocation-free.
#[test]
fn a_scripted_teammate_invites_who_asks_and_takes_orders() {
    let mut h = Harness::new(false);
    h.with_puppet();
    h.step();
    let body = h.me().body;
    h.stage_puppet(|p| p.body = body);
    h.puppet_says("team?");
    let invited = |h: &Harness| h.shard.teams.team_of(ID).len() == 1;
    for _ in 0..30 * TICK_HZ {
        if invited(&h) {
            break;
        }
        h.step();
    }
    assert!(invited(&h), "the agent never invited: {}", h.explain());
    assert!(h.verbs.contains("team"), "it invited by its own verb");
    h.puppet_act(ActionMsg::Team {
        op: protocol::TEAM_OP_ACCEPT,
        target: ID,
    });
    let joined = |h: &Harness| h.shard.teams.team_of(ID).len() == 2;
    for _ in 0..5 * TICK_HZ {
        if joined(&h) {
            break;
        }
        h.step();
    }
    assert!(joined(&h), "the puppet never joined: {}", h.explain());
    assert!(h
        .bot
        .history
        .iter()
        .any(|r| r.goal == Goal::Invite && r.outcome == server::mind::Outcome::Done));
    assert!(h
        .puppet_chat
        .iter()
        .any(|c| c.0 == ID && c.2.as_bytes() == b"invite sent: press Y"));

    h.puppet_says_on(protocol::Channel::Team, "help");
    let helped = |h: &Harness| {
        h.puppet_chat
            .iter()
            .any(|c| c.0 == ID && c.2.as_bytes() == server::agent::orders::HELP.as_bytes())
    };
    for _ in 0..20 * TICK_HZ {
        if helped(&h) {
            break;
        }
        h.step();
    }
    assert!(helped(&h), "help was never answered: {}", h.explain());

    h.puppet_says_on(protocol::Channel::Team, "follow me");
    for _ in 0..10 * TICK_HZ {
        h.step();
    }
    // Fifteen metres east: inside the scripted catch-up's 25 m, so only
    // the order brings it.
    h.stage_puppet(|p| p.body.qx += 1_500 / 3);
    let apart = |h: &Harness| {
        let me = h.me().body;
        let mate = h
            .shard
            .world
            .players
            .iter()
            .find(|p| p.active && p.id == PUPPET)
            .unwrap()
            .body;
        ((me.qx - mate.qx) as f32 * 0.03).hypot((me.qz - mate.qz) as f32 * 0.03)
    };
    for _ in 0..90 * TICK_HZ {
        if apart(&h) <= server::explorer::FOLLOW_NEAR_M + 2.0 {
            break;
        }
        h.step();
    }
    assert!(
        apart(&h) <= server::explorer::FOLLOW_NEAR_M + 2.0,
        "the agent never followed ({:.1} m): {}",
        apart(&h),
        h.explain()
    );
    assert!(h
        .puppet_chat
        .iter()
        .any(|c| c.0 == ID && c.2.as_bytes() == b"following you"));
    assert_eq!(h.heap_ops, 0, "the frame path allocated");
}

/// The team-chest tests' decision source: yes to a team; a wood run for
/// the team's chest if it is sent on one (`.1`), else the pack taken there
/// once it carries wood; else stand and look round. Every answer carries
/// the code the teammate gave, if one was.
struct ChestWhenOffered(Option<server::agent::lock::LockCode>, bool);

impl server::mind::DecisionSource for ChestWhenOffered {
    fn kind(&self) -> server::mind::SourceKind {
        server::mind::SourceKind::Scripted
    }

    fn decide(&mut self, s: &server::mind::Summary) -> Result<server::mind::Choice, String> {
        let goal = if s.offers(Goal::JoinTeam) {
            Goal::JoinTeam
        } else if self.1 && s.offers(Goal::SupplyWood) {
            Goal::SupplyWood
        } else if !self.1 && s.offers(Goal::Deliver) && s.count_of("Wood") > 0 {
            Goal::Deliver
        } else {
            Goal::Wait
        };
        Ok(server::mind::Choice {
            goal,
            confidence: 1.0,
            reason: server::mind::Reason::EMPTY,
            input_tokens: 0,
            output_tokens: 0,
            say: server::mind::Say::Quiet,
            code: self.0,
        })
    }
}

/// The puppet is a teammate with a base: a door, a cupboard, a box inside.
/// It stands at its box while the agent is in there with it, which is how
/// the agent learns the team's chest. Then the agent is outside, the door
/// shut, 500 wood in its pack: it walks back, finds no way in but the door,
/// opens it with `E`, walks through and shuts it, puts the wood in the box
/// through its panel, and leaves the way it came, shutting the door behind
/// it. With `locked`, the teammate has put a code lock on the door, and the
/// agent enters the code it was given when the lock refuses it. With
/// `supply`, it is left inside with a hatchet instead of the wood, and is
/// sent on a wood run: walled in, it goes out by the door first, chops what
/// it finds for the run's minutes, then fills the chest the same way.
fn a_teammate_fills_the_team_chest(locked: bool, supply: bool) {
    use server::agent::lock::LockCode;
    use sim_core::build::{BUILD_CELL_M, LOC_EDGE_ZLO};
    use sim_core::deploy::ACCESS_OP_SET_CODE;

    let given = 2468;
    let code = locked.then(|| LockCode::given(given).unwrap());
    let at = common::clearing(3);
    let mind = Mind::inline(ChestWhenOffered(code, supply), MindConfig::default()).unwrap();
    let mut h = Harness::with_mind(false, at, mind);
    let content = common::content();
    h.until(5, |_| false);
    let (bx, bz) = stage_strangers_base(&mut h, at, false);
    let run = if supply {
        Goal::SupplyWood
    } else {
        Goal::Deliver
    };
    let rows = server::population::base_rows(&content).unwrap();
    let (bxf, bzf) = (f32::from(bx) * BUILD_CELL_M, f32::from(bz) * BUILD_CELL_M);
    let haven = h.shard.world.haven;
    let place = move |x: f32, z: f32| sim_core::movement::Body::at(SEED, &haven, x, z);

    // A team: the puppet invites the agent, face to face.
    let body = h.me().body;
    h.stage_puppet(|p| p.body = body);
    h.puppet_act(ActionMsg::Team {
        op: protocol::TEAM_OP_INVITE,
        target: ID,
    });
    let joined = |h: &Harness| h.shard.teams.team_of(ID).len() == 2;
    for _ in 0..30 * TICK_HZ {
        if joined(&h) {
            break;
        }
        h.step();
    }
    assert!(joined(&h), "the agent never joined: {}", h.explain());

    // Both inside, the teammate at its box, the agent across the box from
    // it, looking round as it waits.
    let (mate, me) = (
        place(bxf + BUILD_CELL_M + 2.4, bzf + 2.4),
        place(bxf + BUILD_CELL_M + 0.6, bzf + 0.6),
    );
    h.stage_puppet(move |p| p.body = mate);
    h.stage(move |p| p.body = me);
    let learnt = h.until(30 * TICK_HZ, |b| b.senses().team_home.count > 0);
    assert!(learnt, "the chest was never learnt: {}", h.explain());

    // The teammate locks its door, if it is to be locked, and goes; the
    // agent is outside, the door shut, wood in its pack.
    if locked {
        // A lock is worked within a hand's reach of its door.
        let by_the_door = place(bxf + 1.5, bzf + 1.0);
        h.stage_puppet(move |p| p.body = by_the_door);
        let lock = sim_core::gather::ItemStack {
            count: 1,
            ..common::stack(&content, "item.lock_code")
        };
        h.stage_puppet(move |p| p.inv[HOTBAR_SLOTS] = lock);
        h.puppet_act(ActionMsg::Deploy {
            row: rows.lock,
            cx: bx,
            cz: bz,
            level: 0,
            loc: LOC_EDGE_ZLO,
            pose: sim_core::footprint::Pose::CENTRE,
        });
        h.puppet_act(ActionMsg::Access {
            cx: bx,
            cz: bz,
            level: 0,
            loc: LOC_EDGE_ZLO,
            op: ACCESS_OP_SET_CODE,
            code: given,
        });
        assert!(
            !h.shard
                .world
                .deploys
                .lock_passes(bx, bz, 0, LOC_EDGE_ZLO, ID),
            "the lock does not know the agent yet"
        );
    }
    let (away, out) = (
        place(at.0 + 300.0, at.1 - 300.0),
        place(bxf + 1.5, bzf - 6.0),
    );
    h.stage_puppet(move |p| p.body = away);
    let wood = sim_core::gather::ItemStack {
        count: if supply { 0 } else { 500 },
        ..common::stack(&content, "item.wood")
    };
    let hatchet = sim_core::gather::ItemStack {
        count: u16::from(supply),
        ..common::stack(&content, "item.hatchet_stone")
    };
    let wood_item = wood.item;
    let slot = h
        .me()
        .inv
        .iter()
        .enumerate()
        .skip(HOTBAR_SLOTS)
        .find(|(_, s)| s.count == 0)
        .map(|(i, _)| i)
        .unwrap();
    h.stage(move |p| {
        if !supply {
            p.body = out;
        }
        p.inv[slot] = wood;
        p.inv[1] = hatchet;
    });
    let shut = |h: &Harness| {
        h.shard
            .world
            .deploys
            .find(bx, bz, 0, LOC_EDGE_ZLO)
            .is_some_and(|d| !d.open)
    };
    assert!(shut(&h), "the door stands shut");

    let secs = if supply {
        server::explorer::SUPPLY_GATHER_SECS + 150
    } else {
        150
    };
    let delivered = h.until(secs * TICK_HZ, |b| b.history.iter().any(|r| r.goal == run));
    assert!(delivered, "no delivery ended: {}", h.explain());
    let report = h.bot.history.iter().find(|r| r.goal == run).unwrap();
    assert_eq!(report.outcome, Outcome::Done, "{report:?}: {}", h.explain());
    let w = &h.shard.world;
    let key = sim_core::deploy::box_key(bx + 1, bz, 0, 0);
    let i = w.deploys.box_index(key).expect("the box");
    let boxed: u32 = (0..sim_core::limits::BOX_SLOTS)
        .map(|s| w.deploys.box_slot(i, s))
        .filter(|s| s.count > 0 && s.item == wood_item)
        .map(|s| u32::from(s.count))
        .sum();
    if supply {
        // What it chopped went in, the wood and whatever else a tree pays.
        assert!(boxed > 0, "nothing chopped went in: {}", h.explain());
        assert!(report.gained >= boxed, "{report:?}");
        assert!(h.bot.delivered >= u64::from(boxed));
    } else {
        assert_eq!(boxed, 500, "the wood is in the box");
        assert_eq!(report.gained, 500);
    }
    // All of it out of the pack, unless the box filled first.
    let full = (0..sim_core::limits::BOX_SLOTS).all(|s| w.deploys.box_slot(i, s).count > 0);
    assert!(
        units_of(h.me(), wood_item) == 0 || full,
        "wood left in the pack and room in the box: boxed {boxed}, {report:?}, {}",
        h.explain()
    );
    assert!(shut(&h), "the door was shut behind it");
    assert!(
        (h.me().body.qz as f32 * 0.03) < bzf,
        "it left the way it came"
    );
    let mut verbs = vec!["use", "container", "move"];
    if locked {
        verbs.push("access");
        assert!(h
            .shard
            .world
            .deploys
            .lock_passes(bx, bz, 0, LOC_EDGE_ZLO, ID));
    }
    for verb in verbs {
        assert!(h.verbs.contains(verb), "{verb} never sent: {:?}", h.verbs);
    }
    assert!(h.use_checks >= 2 && h.pick_checks >= 1 && h.panel_checks >= 1);
    let expected: BTreeSet<&str> = EXPECTED_VERBS.into_iter().collect();
    assert!(h.verbs.is_subset(&expected));
    assert_eq!(h.heap_ops, 0, "the frame path allocated");
}

#[test]
fn a_teammate_fills_the_team_chest_through_its_door() {
    a_teammate_fills_the_team_chest(false, false);
}

#[test]
fn a_teammate_given_the_code_fills_the_chest_behind_a_lock() {
    a_teammate_fills_the_team_chest(true, false);
}

#[test]
fn a_teammate_sent_for_wood_chops_it_and_fills_the_chest() {
    a_teammate_fills_the_team_chest(false, true);
}

/// The starter's ground floor as a founder leaves it (`sim_core::bots::
/// STARTER`, its box where the agent's builder puts its own, `agent::build`
/// `EXTRAS`): the core and the stair cell one room, the triangle airlock
/// off the stair cell with its long side a diagonal wall, the front door
/// on the airlock's west edge and the inner door between it and the stair
/// cell, both shut; a cupboard in the core and a box in the stair cell.
/// Returns the plot cell; the puppet is left in the core.
fn stage_airlock_base(h: &mut Harness, near: (f32, f32)) -> (u16, u16) {
    use server::population::base_rows;
    use sim_core::bots::{
        BasePlan, AIRLOCK_STAND_M, INNER_STAND_M, STARTER_FOOTPRINT, STARTER_STAND_M,
    };
    use sim_core::build::{
        anchor, build_cell_of, foundation_terrain_ok, terrain_band, BUILD_CELL_M, LOC_EDGE_XLO,
        LOC_EDGE_ZLO, LOC_PLANE,
    };

    let haven = h.shard.world.haven;
    let buildable = |cx: u16, cz: u16| {
        let (ax, az) = anchor(cx, cz, LOC_PLANE);
        foundation_terrain_ok(SEED, &haven, ax, az)
            && !sim_core::terrain::build_reserved(&haven, ax, az, BUILD_CELL_M * 1.5)
    };
    let (c0, r0) = (build_cell_of(near.0) as i32, build_cell_of(near.1) as i32);
    let mut plot = None;
    'search: for r in 0..64i32 {
        for dz in -r..=r {
            for dx in -r..=r {
                if dx.abs() != r && dz.abs() != r {
                    continue;
                }
                let (cx, cz) = ((c0 + dx) as u16, (r0 + dz) as u16);
                let band = terrain_band(SEED, &haven, cx, cz);
                // Even ground under the base and a cell round it, so the
                // walk round to the front door is open ground.
                let even = (-1..=3).all(|ox| {
                    (-1..=2).all(|oz| {
                        let (x, z) = ((cx as i32 + ox) as u16, (cz as i32 + oz) as u16);
                        (terrain_band(SEED, &haven, x, z) - band).abs() <= 1
                    })
                });
                let plan = BasePlan::new(0, cx, cz);
                // Beside the scene, never on it: the agent stands at `near`,
                // and a wall laid through a body is refused (`REFUSE_B_SPOT`).
                let off_the_agent =
                    |x: u16, z: u16| (i32::from(x) - c0).abs() > 1 || (i32::from(z) - r0).abs() > 1;
                if even
                    && STARTER_FOOTPRINT.iter().all(|&(dx, dz)| {
                        let (x, z) = plan.cell(dx, dz);
                        buildable(x, z) && off_the_agent(x, z)
                    })
                {
                    plot = Some((cx, cz));
                    break 'search;
                }
            }
        }
    }
    let (bx, bz) = plot.expect("an even plot near the scene");
    let content = common::content();
    let stack = |id: &str, count: u16| sim_core::gather::ItemStack {
        count,
        ..common::stack(&content, id)
    };
    let rows = base_rows(&content).unwrap();
    h.with_puppet();
    h.until(2, |_| false);
    let (x0, z0) = (f32::from(bx) * BUILD_CELL_M, f32::from(bz) * BUILD_CELL_M);
    let stand = (x0 + STARTER_STAND_M.0, z0 + STARTER_STAND_M.1);
    let inner = (x0 + INNER_STAND_M.0, z0 + INNER_STAND_M.1);
    let pocket = [
        stack("item.wood", 1000),
        stack("item.wood", 1000),
        stack("item.wood", 1000),
        stack("item.hearth", 1),
        stack("item.door_wood", 2),
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
        p.body = sim_core::movement::Body::at(SEED, &haven, stand.0, stand.1);
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
        pose: sim_core::footprint::Pose::CENTRE,
    };
    let shell = [
        (bx, bz, LOC_EDGE_XLO, rows.wall),
        (bx, bz, LOC_EDGE_ZLO, rows.wall),
        (bx, bz + 1, LOC_EDGE_ZLO, rows.wall),
        (bx + 1, bz, LOC_EDGE_ZLO, rows.wall),
        (bx + 2, bz, LOC_EDGE_XLO, rows.wall),
        (bx + 1, bz + 1, LOC_EDGE_ZLO, rows.doorway),
        (bx + 1, bz + 1, LOC_EDGE_XLO, rows.doorway),
        (bx + 2, bz + 1, LOC_EDGE_XLO, rows.wall),
        (bx + 1, bz + 2, LOC_EDGE_ZLO, rows.wall),
    ];
    let mut acts = vec![
        place(rows.foundation, bx, bz, LOC_PLANE),
        deploy(rows.hearth, bx, bz, LOC_PLANE),
        place(rows.foundation, bx + 1, bz, LOC_PLANE),
        place(rows.foundation, bx + 1, bz + 1, LOC_PLANE),
    ];
    // The airlock's outer walls are past reach from the core: laid from
    // the room (`bots::stand_m`).
    let (core_shell, outer) = shell.split_at(shell.len() - 2);
    for &(cx, cz, loc, row) in core_shell {
        acts.push(place(row, cx, cz, loc));
    }
    for a in acts.drain(..) {
        h.puppet_act(a);
    }
    for (spot, walls) in [(inner, outer), (stand, &[][..])] {
        for core in [&mut h.shard, &mut h.replay] {
            let haven = core.world.haven;
            let p = core
                .world
                .players
                .iter_mut()
                .find(|p| p.active && p.id == PUPPET)
                .unwrap();
            p.body = sim_core::movement::Body::at(SEED, &haven, spot.0, spot.1);
        }
        for &(cx, cz, loc, row) in walls {
            h.puppet_act(place(row, cx, cz, loc));
        }
    }
    // Each door hung from beside it, in sight of its doorway (a hand places
    // only what it sees: `bots::stand_m`), the front from the airlock and
    // the inner from the room; the box from the core, the puppet left there.
    let airlock = (x0 + AIRLOCK_STAND_M.0, z0 + AIRLOCK_STAND_M.1);
    for (spot, act) in [
        (airlock, deploy(rows.door, bx + 1, bz + 1, LOC_EDGE_XLO)),
        (inner, deploy(rows.door, bx + 1, bz + 1, LOC_EDGE_ZLO)),
        (stand, deploy(rows.container, bx + 1, bz, LOC_PLANE)),
    ] {
        for core in [&mut h.shard, &mut h.replay] {
            let haven = core.world.haven;
            let p = core
                .world
                .players
                .iter_mut()
                .find(|p| p.active && p.id == PUPPET)
                .unwrap();
            p.body = sim_core::movement::Body::at(SEED, &haven, spot.0, spot.1);
        }
        h.puppet_act(act);
        // Taken where it was sent from, before the body moves on.
        for _ in 0..10 * TICK_HZ {
            if h.shard.wants_action(h.puppet_slot) && h.replay.wants_action(h.puppet_slot) {
                break;
            }
            h.step();
        }
    }
    for core in [&mut h.shard, &mut h.replay] {
        let w = &core.world;
        for &(cx, cz, loc, _) in &shell {
            assert!(
                w.pieces.find(cx, cz, 0, loc).is_some(),
                "a wall at {cx},{cz},{loc}"
            );
        }
        for loc in [LOC_EDGE_XLO, LOC_EDGE_ZLO] {
            assert!(
                w.deploys
                    .find(bx + 1, bz + 1, 0, loc)
                    .is_some_and(|d| !d.open),
                "a shut door at {loc}"
            );
        }
        assert!(w
            .deploys
            .box_index(sim_core::deploy::box_key(bx + 1, bz, 0, 0))
            .is_some());
    }
    (bx, bz)
}

/// #364: the deliveries of a crew whose founder built the starter failed
/// nearly every time ("could not reach a target", a minute and more in,
/// home 3 m away). Arriving round the back of the base, the route ends by
/// the stair cell's wall, where the airlock's inner door is the nearest
/// door to the box: it cannot be got to from out there (the airlock's
/// diagonal wall is in the way), and given up from outside it stayed given
/// up for the whole trip, so once in through the front door the only door
/// left was the one just come through. Given up doors are tried again from
/// the far side of a door, and one that a wall or a shut door stands in
/// front of is given up at once rather than walked into.
#[test]
fn a_teammate_fills_the_team_chest_through_an_airlock_from_behind_the_base() {
    use sim_core::build::BUILD_CELL_M;

    let at = common::clearing(3);
    let mind = Mind::inline(ChestWhenOffered(None, false), MindConfig::default()).unwrap();
    let mut h = Harness::with_mind(false, at, mind);
    let content = common::content();
    h.until(5, |_| false);
    let (bx, bz) = stage_airlock_base(&mut h, at);
    let (x0, z0) = (f32::from(bx) * BUILD_CELL_M, f32::from(bz) * BUILD_CELL_M);
    let haven = h.shard.world.haven;
    let place = move |x: f32, z: f32| sim_core::movement::Body::at(SEED, &haven, x, z);

    // A team, face to face, then both in the stair cell, the teammate at
    // its box: the agent learns the team's chest.
    let body = h
        .shard
        .world
        .players
        .iter()
        .find(|p| p.id == PUPPET)
        .unwrap()
        .body;
    h.stage(move |p| p.body = body);
    h.puppet_act(ActionMsg::Team {
        op: protocol::TEAM_OP_INVITE,
        target: ID,
    });
    let joined = |h: &Harness| h.shard.teams.team_of(ID).len() == 2;
    for _ in 0..30 * TICK_HZ {
        if joined(&h) {
            break;
        }
        h.step();
    }
    assert!(joined(&h), "the agent never joined: {}", h.explain());
    let (mate, me) = (
        place(x0 + BUILD_CELL_M + 2.4, z0 + 2.4),
        place(x0 + BUILD_CELL_M + 0.6, z0 + 0.6),
    );
    h.stage_puppet(move |p| p.body = mate);
    h.stage(move |p| p.body = me);
    let learnt = h.until(30 * TICK_HZ, |b| b.senses().team_home.count > 0);
    assert!(learnt, "the chest was never learnt: {}", h.explain());

    // Nobody home; the agent round the back of the base, east of the stair
    // cell, with wood to put away.
    let away = place(at.0 + 300.0, at.1 - 300.0);
    h.stage_puppet(move |p| p.body = away);
    let wood = sim_core::gather::ItemStack {
        count: 500,
        ..common::stack(&content, "item.wood")
    };
    let wood_item = wood.item;
    let slot = h
        .me()
        .inv
        .iter()
        .enumerate()
        .skip(HOTBAR_SLOTS)
        .find(|(_, s)| s.count == 0)
        .map(|(i, _)| i)
        .unwrap();
    let behind = place(x0 + 3.0 * BUILD_CELL_M + 2.0, z0 + 1.5);
    h.stage(move |p| {
        p.body = behind;
        p.inv[slot] = wood;
    });

    let delivered = h.until(150 * TICK_HZ, |b| {
        b.history.iter().any(|r| r.goal == Goal::Deliver)
    });
    assert!(delivered, "no delivery ended: {}", h.explain());
    let report = h
        .bot
        .history
        .iter()
        .find(|r| r.goal == Goal::Deliver)
        .unwrap();
    assert_eq!(report.outcome, Outcome::Done, "{report:?}: {}", h.explain());
    // Round the base, in by both doors, the box, out by both: no stage of
    // a door walked into for its whole time on the way (46 s when written).
    assert!(report.secs < 60, "{report:?}: {}", h.explain());
    let w = &h.shard.world;
    let i = w
        .deploys
        .box_index(sim_core::deploy::box_key(bx + 1, bz, 0, 0))
        .expect("the box");
    let boxed: u32 = (0..sim_core::limits::BOX_SLOTS)
        .map(|s| w.deploys.box_slot(i, s))
        .filter(|s| s.count > 0 && s.item == wood_item)
        .map(|s| u32::from(s.count))
        .sum();
    assert_eq!(boxed, 500, "the wood is in the box: {}", h.explain());
    assert_eq!(h.heap_ops, 0, "the frame path allocated");
}

/// The follow test's decision source: yes to a team, then with them.
struct FollowWhenOffered;

impl server::mind::DecisionSource for FollowWhenOffered {
    fn kind(&self) -> server::mind::SourceKind {
        server::mind::SourceKind::Scripted
    }

    fn decide(&mut self, s: &server::mind::Summary) -> Result<server::mind::Choice, String> {
        let goal = [Goal::JoinTeam, Goal::Follow]
            .into_iter()
            .find(|g| s.offers(*g))
            .unwrap_or(Goal::Wait);
        Ok(server::mind::Choice {
            goal,
            confidence: 1.0,
            reason: server::mind::Reason::EMPTY,
            input_tokens: 0,
            output_tokens: 0,
            say: server::mind::Say::Quiet,
            code: None,
        })
    }
}

/// Its teammate has gone home and shut the door behind them. Following,
/// the agent finds no way to them but the door: it opens it with `E`, walks
/// in, shuts it behind, and is with them.
#[test]
fn a_teammate_follows_through_a_shut_door() {
    use sim_core::build::{BUILD_CELL_M, LOC_EDGE_ZLO};

    let at = common::clearing(3);
    let mind = Mind::inline(FollowWhenOffered, MindConfig::default()).unwrap();
    let mut h = Harness::with_mind(false, at, mind);
    h.until(5, |_| false);
    let (bx, bz) = stage_strangers_base(&mut h, at, false);
    let (bxf, bzf) = (f32::from(bx) * BUILD_CELL_M, f32::from(bz) * BUILD_CELL_M);
    let haven = h.shard.world.haven;
    let place = move |x: f32, z: f32| sim_core::movement::Body::at(SEED, &haven, x, z);
    let body = h.me().body;
    h.stage_puppet(|p| p.body = body);
    h.puppet_act(ActionMsg::Team {
        op: protocol::TEAM_OP_INVITE,
        target: ID,
    });
    let joined = |h: &Harness| h.shard.teams.team_of(ID).len() == 2;
    for _ in 0..30 * TICK_HZ {
        if joined(&h) {
            break;
        }
        h.step();
    }
    assert!(joined(&h), "the agent never joined: {}", h.explain());
    let (mate, out) = (
        place(bxf + BUILD_CELL_M + 0.6, bzf + 0.6),
        place(bxf + 1.5, bzf - 4.0),
    );
    h.stage_puppet(move |p| p.body = mate);
    h.stage(move |p| p.body = out);
    let shut = |h: &Harness| {
        h.shard
            .world
            .deploys
            .find(bx, bz, 0, LOC_EDGE_ZLO)
            .is_some_and(|d| !d.open)
    };
    assert!(shut(&h));
    let near = |h: &Harness| {
        let me = h.me().body;
        let (x, z) = (me.qx as f32 * 0.03, me.qz as f32 * 0.03);
        z > bzf + 0.3
            && (x - mate_x(bxf)).hypot(z - (bzf + 0.6)) <= server::explorer::FOLLOW_NEAR_M + 1.0
    };
    fn mate_x(bxf: f32) -> f32 {
        bxf + BUILD_CELL_M + 0.6
    }
    for _ in 0..60 * TICK_HZ {
        if near(&h) && shut(&h) {
            break;
        }
        h.step();
    }
    assert!(near(&h), "never got in to its teammate: {}", h.explain());
    assert!(shut(&h), "the door was left open");
    assert!(h.verbs.contains("use"), "{:?}", h.verbs);
    assert_eq!(h.heap_ops, 0, "the frame path allocated");
}

/// Two of the team's boxes: the one inside holding fragments, one outside
/// holding cloth and a stone hatchet. Shown both, the agent comes home with
/// wood and cloth and no hatchet: the wood goes in with the fragments, the
/// cloth with the cloth, and it takes the hatchet out.
#[test]
fn a_teammate_sorts_its_pack_into_the_team_s_boxes() {
    use sim_core::build::{BUILD_CELL_M, LOC_EDGE_ZLO, LOC_PLANE};

    let at = common::clearing(3);
    let mind = Mind::inline(ChestWhenOffered(None, false), MindConfig::default()).unwrap();
    let mut h = Harness::with_mind(false, at, mind);
    let content = common::content();
    h.until(5, |_| false);
    let (bx, bz) = stage_strangers_base(&mut h, at, false);
    let rows = server::population::base_rows(&content).unwrap();
    let (bxf, bzf) = (f32::from(bx) * BUILD_CELL_M, f32::from(bz) * BUILD_CELL_M);
    let haven = h.shard.world.haven;
    let place = move |x: f32, z: f32| sim_core::movement::Body::at(SEED, &haven, x, z);
    let stack = |id: &str, count: u16| sim_core::gather::ItemStack {
        count,
        ..common::stack(&content, id)
    };
    let (cloth, hatchet, wood) = (
        stack("item.cloth", 10),
        stack("item.hatchet_stone", 1),
        stack("item.wood", 500),
    );
    // The second box, outside the east cell's south wall.
    let (ox, oz) = (bx + 1, bz - 1);
    let spare_box = stack("item.box_small", 1);
    h.stage_puppet(move |p| {
        p.body = place(bxf + 4.5, bzf - 3.5);
        p.inv[HOTBAR_SLOTS] = spare_box;
    });
    h.puppet_act(ActionMsg::Deploy {
        row: rows.container,
        cx: ox,
        cz: oz,
        level: 0,
        loc: LOC_PLANE,
        pose: sim_core::footprint::Pose::CENTRE,
    });
    let outside = sim_core::deploy::box_key(ox, oz, 0, LOC_PLANE);
    for core in [&mut h.shard, &mut h.replay] {
        let w = &mut core.world;
        let i = w.deploys.box_index(outside).expect("the outside box");
        w.deploys.set_box_slot(i, 0, cloth);
        w.deploys.set_box_slot(i, 1, hatchet);
    }

    // A team, and its base shown: the teammate at the box inside, then
    // the agent outside with the other box in view.
    let body = h.me().body;
    h.stage_puppet(|p| p.body = body);
    h.puppet_act(ActionMsg::Team {
        op: protocol::TEAM_OP_INVITE,
        target: ID,
    });
    let joined = |h: &Harness| h.shard.teams.team_of(ID).len() == 2;
    for _ in 0..30 * TICK_HZ {
        if joined(&h) {
            break;
        }
        h.step();
    }
    assert!(joined(&h), "the agent never joined: {}", h.explain());
    let (mate, me) = (
        place(bxf + BUILD_CELL_M + 2.4, bzf + 2.4),
        place(bxf + BUILD_CELL_M + 0.6, bzf + 0.6),
    );
    h.stage_puppet(move |p| p.body = mate);
    h.stage(move |p| p.body = me);
    assert!(
        h.until(30 * TICK_HZ, |b| b.shared().box_count() >= 1),
        "the inside box was never learnt: {}",
        h.explain()
    );
    let (away, west) = (
        place(at.0 + 300.0, at.1 - 300.0),
        place(bxf - 0.5, bzf - 1.5),
    );
    h.stage_puppet(move |p| p.body = away);
    h.stage(move |p| p.body = west);
    assert!(
        h.until(30 * TICK_HZ, |b| b.shared().box_count() >= 2),
        "the outside box was never learnt: {}",
        h.explain()
    );

    // Home with wood and cloth.
    let slots: Vec<usize> = h
        .me()
        .inv
        .iter()
        .enumerate()
        .skip(HOTBAR_SLOTS)
        .filter(|(_, s)| s.count == 0)
        .map(|(i, _)| i)
        .take(2)
        .collect();
    let (a, b) = (slots[0], slots[1]);
    let more_cloth = stack("item.cloth", 50);
    h.stage(move |p| {
        p.inv[a] = wood;
        p.inv[b] = more_cloth;
    });
    let done = h.until(150 * TICK_HZ, |b| {
        b.history.iter().any(|r| r.goal == Goal::Deliver)
    });
    assert!(done, "no trip home ended: {}", h.explain());
    let report = h
        .bot
        .history
        .iter()
        .find(|r| r.goal == Goal::Deliver)
        .unwrap();
    assert_eq!(report.outcome, Outcome::Done, "{report:?}: {}", h.explain());
    let w = &h.shard.world;
    let held = |key: u32, item: u16| -> u32 {
        let i = w.deploys.box_index(key).unwrap();
        (0..sim_core::limits::BOX_SLOTS)
            .map(|s| w.deploys.box_slot(i, s))
            .filter(|s| s.count > 0 && s.item == item)
            .map(|s| u32::from(s.count))
            .sum()
    };
    let inside = sim_core::deploy::box_key(bx + 1, bz, 0, LOC_PLANE);
    assert_eq!(held(inside, wood.item), 500, "the wood with the fragments");
    assert_eq!(held(outside, wood.item), 0);
    assert_eq!(held(outside, cloth.item), 60, "the cloth with the cloth");
    assert_eq!(held(inside, cloth.item), 0);
    assert_eq!(held(outside, hatchet.item), 0, "the hatchet taken out");
    assert_eq!(units_of(h.me(), hatchet.item), 1);
    assert!(
        h.shard
            .world
            .deploys
            .find(bx, bz, 0, LOC_EDGE_ZLO)
            .is_some_and(|d| !d.open),
        "the door was shut behind it"
    );
    assert_eq!(h.heap_ops, 0, "the frame path allocated");
}

/// A decision source for the team's-base tests: the first goal of `.0` on
/// offer, else stand and look round; every answer carries the code the
/// teammate gave (`.1`), if one was.
struct Prefer(&'static [Goal], Option<server::agent::lock::LockCode>);

impl server::mind::DecisionSource for Prefer {
    fn kind(&self) -> server::mind::SourceKind {
        server::mind::SourceKind::Scripted
    }

    fn decide(&mut self, s: &server::mind::Summary) -> Result<server::mind::Choice, String> {
        let goal = self
            .0
            .iter()
            .copied()
            .find(|g| s.offers(*g))
            .unwrap_or(Goal::Wait);
        Ok(server::mind::Choice {
            goal,
            confidence: 1.0,
            reason: server::mind::Reason::EMPTY,
            input_tokens: 0,
            output_tokens: 0,
            say: server::mind::Say::Quiet,
            code: self.1,
        })
    }
}

/// A third body, never on the team: the stranger at the team's base.
const STRANGER: u32 = 258;
const STRANGER_SLOT: usize = 2;

impl Harness {
    /// The stranger joins, in both shards.
    fn with_stranger(&mut self) {
        for core in [&mut self.shard, &mut self.replay] {
            assert!(core.connect(STRANGER_SLOT, STRANGER));
        }
        self.until(2, |_| false);
    }

    /// `f` with the puppet's helpers driving the stranger.
    fn as_stranger(&mut self, f: impl FnOnce(&mut Self)) {
        let was = (self.puppet_id, self.puppet_slot);
        (self.puppet_id, self.puppet_slot) = (STRANGER, STRANGER_SLOT);
        f(self);
        (self.puppet_id, self.puppet_slot) = was;
    }
}

/// The agent moves in with its teammate: the teammate's base (a door, a
/// cupboard, a box) staged north of `at`, the invite taken face to face,
/// and the teammate at their box with the agent inside to see it, which
/// makes the base its home. The teammate then goes far off. Returns the
/// door's cell.
fn move_in(h: &mut Harness, at: (f32, f32)) -> (u16, u16) {
    use sim_core::build::BUILD_CELL_M;

    h.until(5, |_| false);
    let (bx, bz) = stage_strangers_base(h, at, false);
    let (bxf, bzf) = (f32::from(bx) * BUILD_CELL_M, f32::from(bz) * BUILD_CELL_M);
    let haven = h.shard.world.haven;
    let place = move |x: f32, z: f32| sim_core::movement::Body::at(SEED, &haven, x, z);
    let body = h.me().body;
    h.stage_puppet(|p| p.body = body);
    h.puppet_act(ActionMsg::Team {
        op: protocol::TEAM_OP_INVITE,
        target: ID,
    });
    let joined = |h: &Harness| h.shard.teams.team_of(ID).len() == 2;
    for _ in 0..30 * TICK_HZ {
        if joined(h) {
            break;
        }
        h.step();
    }
    assert!(joined(h), "the agent never joined: {}", h.explain());
    let (mate, me) = (
        place(bxf + BUILD_CELL_M + 2.4, bzf + 2.4),
        place(bxf + BUILD_CELL_M + 0.6, bzf + 0.6),
    );
    h.stage_puppet(move |p| p.body = mate);
    h.stage(move |p| p.body = me);
    assert!(
        h.until(30 * TICK_HZ, |b| b.shared().box_count() >= 1),
        "the team's box was never learnt: {}",
        h.explain()
    );
    // Long enough to have looked round the rest of it.
    h.until(5 * TICK_HZ, |_| false);
    let away = place(at.0 + 300.0, at.1 - 300.0);
    h.stage_puppet(move |p| p.body = away);
    (bx, bz)
}

/// Where the agent's feet are, metres.
fn feet(h: &Harness) -> [f32; 2] {
    let me = h.me().body;
    [
        me.qx as f32 * sim_core::movement::POS_XZ_Q,
        me.qz as f32 * sim_core::movement::POS_XZ_Q,
    ]
}

/// Inside the staged base's two cells.
fn inside_base(p: [f32; 2], (bx, bz): (u16, u16)) -> bool {
    use sim_core::build::BUILD_CELL_M;
    let (x0, z0) = (f32::from(bx) * BUILD_CELL_M, f32::from(bz) * BUILD_CELL_M);
    p[0] > x0 + 0.2
        && p[0] < x0 + 2.0 * BUILD_CELL_M - 0.2
        && p[1] > z0 + 0.2
        && p[1] < z0 + BUILD_CELL_M - 0.2
}

/// Living in its teammate's base (none of its own), the agent sent home
/// from twenty metres out walks back, finds no way in but the door, opens
/// it with `E`, walks in, shuts it behind, and stays by the box.
#[test]
fn a_teammate_living_in_the_team_s_base_goes_home_to_it() {
    use sim_core::build::{BUILD_CELL_M, LOC_EDGE_ZLO};

    let at = common::clearing(3);
    let mind = Mind::inline(
        Prefer(&[Goal::JoinTeam, Goal::GoHome], None),
        MindConfig::default(),
    )
    .unwrap();
    let mut h = Harness::with_mind(false, at, mind);
    let (bx, bz) = move_in(&mut h, at);
    assert!(h.bot.memory().shared, "it lives in the team's base");
    let (bxf, bzf) = (f32::from(bx) * BUILD_CELL_M, f32::from(bz) * BUILD_CELL_M);
    let haven = h.shard.world.haven;
    let out = sim_core::movement::Body::at(SEED, &haven, bxf + 1.5, bzf - 20.0);
    h.stage(move |p| p.body = out);
    let shut = |h: &Harness| {
        h.shard
            .world
            .deploys
            .find(bx, bz, 0, LOC_EDGE_ZLO)
            .is_some_and(|d| !d.open)
    };
    assert!(shut(&h));
    let home = h.until(120 * TICK_HZ, |b| {
        b.history.iter().any(|r| r.goal == Goal::GoHome)
    });
    assert!(home, "no walk home ended: {}", h.explain());
    let report = h
        .bot
        .history
        .iter()
        .find(|r| r.goal == Goal::GoHome)
        .unwrap();
    assert_eq!(report.outcome, Outcome::Done, "{report:?}: {}", h.explain());
    assert!(
        inside_base(feet(&h), (bx, bz)),
        "home, inside: {:?} {}",
        feet(&h),
        h.explain()
    );
    let first = h.bot.shared().first().unwrap();
    let (cx, cz) = sim_core::deploy::cell_center(first.cx, first.cz);
    let p = feet(&h);
    assert!((p[0] - cx).hypot(p[1] - cz) <= server::agent::supply::STAY_NEAR_M + 0.5);
    assert!(shut(&h), "the door was shut behind it");
    assert!(h.verbs.contains("use"), "{:?}", h.verbs);
    // Home, it is not sent home again.
    h.until(10 * TICK_HZ, |_| false);
    let walks = h
        .bot
        .history
        .iter()
        .filter(|r| r.goal == Goal::GoHome)
        .count();
    assert_eq!(walks, 1, "{}", h.explain());
    assert_ne!(h.bot.goal(), Some(Goal::GoHome));
    assert_eq!(h.heap_ops, 0, "the frame path allocated");
}

/// Living in its teammate's base, the agent stands on the far side of it
/// from where a stranger hacks at the base's south wall, the base between
/// them. The blows, not its eyes, call it home: it comes round to the door,
/// takes the stranger on, and once it has been quiet a while, the defence
/// is over.
#[test]
fn a_teammate_defends_the_team_s_base_when_a_stranger_strikes_it() {
    use sim_core::build::{BUILD_CELL_M, LOC_EDGE_ZLO};

    let at = common::clearing(3);
    let mind = Mind::inline(
        Prefer(&[Goal::JoinTeam, Goal::Defend], None),
        MindConfig::default(),
    )
    .unwrap();
    let mut h = Harness::with_mind(false, at, mind);
    let content = common::content();
    let (bx, bz) = move_in(&mut h, at);
    let (bxf, bzf) = (f32::from(bx) * BUILD_CELL_M, f32::from(bz) * BUILD_CELL_M);
    let haven = h.shard.world.haven;
    // North of the base, a spear on the belt.
    let spear = h.spear;
    let out = sim_core::movement::Body::at(SEED, &haven, bxf + 4.5, bzf + 18.0);
    h.stage(move |p| {
        p.body = out;
        p.inv[0] = spear;
    });
    h.until(3 * TICK_HZ, |_| false);

    // The stranger hacks at the east cell's south wall from outside.
    h.with_stranger();
    let hatchet = sim_core::gather::ItemStack {
        count: 1,
        ..common::stack(&content, "item.hatchet_stone")
    };
    let wall = (bx + 1, bz, 0u8, LOC_EDGE_ZLO);
    let hp = |h: &Harness| {
        h.shard
            .world
            .pieces
            .find(wall.0, wall.1, wall.2, wall.3)
            .map(|r| r.hp)
    };
    let full = hp(&h).expect("the wall stands");
    let (wx, wz) = (bxf + BUILD_CELL_M + 1.5, bzf - 0.8);
    let seen = |h: &Harness| h.bot.tracks().get(STRANGER).is_some_and(|t| t.visible);
    let mut called_blind = false;
    for _ in 0..20 * TICK_HZ {
        h.as_stranger(|h| h.puppet_at(wx, wz, 0, hatchet, true));
        h.step();
        if h.bot.goal() == Some(Goal::Defend) {
            called_blind = !seen(&h);
            break;
        }
    }
    assert!(
        hp(&h).expect("the wall stands") < full,
        "the blows never landed"
    );
    assert!(
        h.bot.goal() == Some(Goal::Defend),
        "the blows never called it home: {}",
        h.explain()
    );
    assert!(called_blind, "it was called by its eyes, not the blows");
    // The stranger hacks on; the agent comes round and takes them on.
    let engages = h.bot.combat().stats.engages;
    let fought = (0..60 * TICK_HZ).any(|_| {
        h.as_stranger(|h| h.puppet_at(wx, wz, 0, hatchet, true));
        h.step();
        h.bot.combat().stats.engages > engages
    });
    assert!(fought, "it never took the stranger on: {}", h.explain());
    assert_eq!(h.bot.combat().foe().map(|f| f.id), Some(STRANGER));
    let me = feet(&h);
    assert!(
        (me[0] - wx).hypot(me[1] - wz) <= server::agent::supply::HOME_M,
        "it came home to fight: {me:?}"
    );
    // The fight is the reflex's from here; once it is over and the base
    // has been quiet a while, the defence ends.
    let over = h.until((server::explorer::SHARED_GUARD_SECS + 120) * TICK_HZ, |b| {
        b.history.iter().any(|r| r.goal == Goal::Defend)
    });
    assert!(over, "the defence never ended: {}", h.explain());
    let report = h
        .bot
        .history
        .iter()
        .find(|r| r.goal == Goal::Defend)
        .unwrap();
    assert_eq!(report.outcome, Outcome::Done, "{report:?}: {}", h.explain());
    assert_eq!(h.heap_ops, 0, "the frame path allocated");
}

/// Living in its teammate's base, the agent carries a sleeping bag and
/// has the code the teammate gave on team chat. The teammate's cupboard
/// has a code lock on it with that code, so the base is theirs alone: a
/// bag would be refused in it. The agent walks in through the door, enters
/// the code at the cupboard's keypad (which puts it on the crew, the
/// keypad's way), and puts its bag down on the floor inside, clear of the
/// doorway.
#[test]
fn a_teammate_given_the_code_puts_its_bag_down_in_the_team_s_base() {
    use server::agent::lock::LockCode;
    use sim_core::build::{BUILD_CELL_M, LOC_EDGE_ZLO, LOC_PLANE};
    use sim_core::deploy::{ACCESS_OP_SET_CODE, ARCH_BAG};

    let given = 1357;
    let at = common::clearing(3);
    let mind = Mind::inline(
        Prefer(
            &[Goal::JoinTeam, Goal::Bag],
            Some(LockCode::given(given).unwrap()),
        ),
        MindConfig::default(),
    )
    .unwrap();
    let mut h = Harness::with_mind(false, at, mind);
    let content = common::content();
    let rows = server::population::base_rows(&content).unwrap();
    // No bag yet: the move-in comes first.
    let (bx, bz) = move_in(&mut h, at);
    let (bxf, bzf) = (f32::from(bx) * BUILD_CELL_M, f32::from(bz) * BUILD_CELL_M);
    let haven = h.shard.world.haven;
    let place = move |x: f32, z: f32| sim_core::movement::Body::at(SEED, &haven, x, z);
    assert!(
        h.bot.shared().hearth().is_some(),
        "the cupboard was never seen: {}",
        h.explain()
    );
    // The teammate comes back, bolts a lock onto the cupboard, sets the
    // code, and goes.
    let lock = sim_core::gather::ItemStack {
        count: 1,
        ..common::stack(&content, "item.lock_code")
    };
    let mate = place(bxf + 1.5, bzf + 2.2);
    h.stage_puppet(move |p| {
        p.body = mate;
        p.inv[HOTBAR_SLOTS] = lock;
    });
    h.puppet_act(ActionMsg::Deploy {
        row: rows.lock,
        cx: bx,
        cz: bz,
        level: 0,
        loc: LOC_PLANE,
        pose: sim_core::footprint::Pose::CENTRE,
    });
    h.puppet_act(ActionMsg::Access {
        cx: bx,
        cz: bz,
        level: 0,
        loc: LOC_PLANE,
        op: ACCESS_OP_SET_CODE,
        code: given,
    });
    let crew = |h: &Harness| {
        h.shard
            .world
            .deploys
            .hearths()
            .iter()
            .any(|r| (r.cx, r.cz) == (bx, bz) && r.crew.contains(ID))
    };
    assert!(
        h.shard
            .world
            .deploys
            .find(bx, bz, 0, LOC_PLANE)
            .is_some_and(|d| d.has_lock),
        "the cupboard's lock"
    );
    assert!(!crew(&h), "the agent is not on the crew yet");
    let away = place(at.0 + 300.0, at.1 - 300.0);
    h.stage_puppet(move |p| p.body = away);
    // Outside, a bag in the pack.
    let bag = sim_core::gather::ItemStack {
        count: 1,
        ..common::stack(&content, "item.sleeping_bag")
    };
    let out = place(bxf + 1.5, bzf - 8.0);
    h.stage(move |p| {
        p.body = out;
        p.inv[HOTBAR_SLOTS + 2] = bag;
    });
    let ended = h.until(150 * TICK_HZ, |b| {
        b.history.iter().any(|r| r.goal == Goal::Bag)
    });
    assert!(ended, "no bag goal ended: {}", h.explain());
    let report = h.bot.history.iter().find(|r| r.goal == Goal::Bag).unwrap();
    assert_eq!(report.outcome, Outcome::Done, "{report:?}: {}", h.explain());
    assert!(crew(&h), "the code put it on the crew");
    assert!(h.bot.shared().crewed());
    let w = &h.shard.world;
    let bags: Vec<_> = w
        .deploys
        .entries()
        .iter()
        .filter(|d| d.owner == ID && w.deploy.defs[usize::from(d.row)].arch == ARCH_BAG)
        .copied()
        .collect();
    assert_eq!(bags.len(), 1, "{}", h.explain());
    let (x, z) = bags[0].xz();
    assert!(
        inside_base([x, z], (bx, bz)),
        "the bag lies inside the base: {x},{z}"
    );
    let door = sim_core::build::anchor(bx, bz, LOC_EDGE_ZLO);
    assert!(
        (x - door.0).hypot(z - door.1) >= server::agent::home::FLOOR_DOOR_CLEAR_M - 0.01,
        "clear of the doorway"
    );
    for verb in ["use", "access", "deploy"] {
        assert!(h.verbs.contains(verb), "{verb} never sent: {:?}", h.verbs);
    }
    assert!(
        h.shard
            .world
            .deploys
            .find(bx, bz, 0, LOC_EDGE_ZLO)
            .is_some_and(|d| !d.open),
        "the door was shut behind it"
    );
    // One is enough: it is not offered another there.
    h.until(2 * TICK_HZ, |_| false);
    assert!(h.bot.memory().bag_home);
    let view = &h.view;
    assert!(!h.bot.summary(view, ID).unwrap().offers(Goal::Bag));
    assert_eq!(h.heap_ops, 0, "the frame path allocated");
}

/// The public places' decision source: sell wood at THE GATE, or feed a
/// work, whichever is on offer, else stand.
struct VisitWhenOffered;

impl server::mind::DecisionSource for VisitWhenOffered {
    fn kind(&self) -> server::mind::SourceKind {
        server::mind::SourceKind::Scripted
    }

    fn decide(&mut self, s: &server::mind::Summary) -> Result<server::mind::Choice, String> {
        let wood = Goal::Sell(server::mind::Name::new(b"Wood").unwrap());
        let goal = [wood, Goal::FeedWork, Goal::Research]
            .into_iter()
            .find(|g| s.offers(*g))
            .unwrap_or(Goal::Wait);
        Ok(server::mind::Choice {
            goal,
            confidence: 1.0,
            reason: server::mind::Reason::EMPTY,
            input_tokens: 0,
            output_tokens: 0,
            say: server::mind::Say::Quiet,
            code: None,
        })
    }
}

/// THE GATE's SALVAGE stall buys wood for junk: carrying 3000, the agent
/// walks to its counter and sells all but two lots of a thousand.
#[test]
fn a_survivor_sells_its_surplus_wood_at_the_gate() {
    let mind = Mind::inline(VisitWhenOffered, MindConfig::default()).unwrap();
    let haven = sim_core::terrain::haven(SEED);
    assert!(haven.town.live, "the fixture seed has a town");
    let content = common::content();
    let salvage = (0..sim_core::limits::MAX_VENDORS)
        .find(|&k| {
            let t = server::net::bake_all(&content).unwrap();
            t.vendor_names.get(k).is_some_and(|n| n == "SALVAGE")
        })
        .unwrap();
    let (kx, kz) = sim_core::town::kiosk_world(&haven.town, salvage).unwrap();
    // Twenty metres off the counter, out in the street.
    let at = (kx - 20.0, kz);
    let mut h = Harness::with_mind(false, at, mind);
    h.until(5, |_| false);
    let wood = sim_core::gather::ItemStack {
        count: 1000,
        ..common::stack(&content, "item.wood")
    };
    h.stage(move |p| {
        p.body = sim_core::movement::Body::at(SEED, &haven, at.0, at.1);
        for i in 0..3 {
            p.inv[HOTBAR_SLOTS + i] = wood;
        }
    });
    let junk = content.item_index("item.junk").unwrap();
    let sold = h.until(90 * TICK_HZ, |b| {
        b.history
            .iter()
            .any(|r| matches!(r.goal, Goal::Sell(_)) && r.outcome == Outcome::Done)
    });
    assert!(sold, "no sale: {}", h.explain());
    assert_eq!(units_of(h.me(), wood.item), 2000, "two lots kept");
    assert_eq!(units_of(h.me(), junk), 4, "a thousand wood is four junk");
    assert!(h.verbs.contains("vend"), "{:?}", h.verbs);
    assert_eq!(h.heap_ops, 0, "the frame path allocated");
}

/// THE GATE's research table makes paper of what an agent found: carrying a
/// revolver it has no blueprint for (its tree parent unlearned) and junk,
/// the agent walks to the table, puts in the revolver and its price,
/// switches it on, takes the paper off and reads it. From the south, round
/// the workshop: long enough that the next decision falls mid-research,
/// with the revolver in the table rather than the pack. Taken for done,
/// that decision walked off and left the paper in the table, shut to
/// everyone after (2026-10-17).
#[test]
fn a_survivor_researches_what_it_found_at_the_gate_s_table() {
    let mind = Mind::inline(VisitWhenOffered, MindConfig::default()).unwrap();
    let haven = sim_core::terrain::haven(SEED);
    assert!(haven.town.live, "the fixture seed has a town");
    let (_, tx, tz) = (0..sim_core::town::STATIONS.len())
        .filter_map(|k| sim_core::town::station_world(&haven.town, k))
        .find(|&(arch, ..)| arch == "research")
        .unwrap();
    let at = (tx, tz - 25.0);
    let mut h = Harness::with_mind(false, at, mind);
    for core in [&mut h.shard, &mut h.replay] {
        core.world.seed_authored();
    }
    h.until(5, |_| false);
    let content = common::content();
    let revolver = common::stack(&content, "item.revolver");
    let junk = sim_core::gather::ItemStack {
        count: 100,
        ..common::stack(&content, "item.junk")
    };
    h.stage(move |p| {
        p.body = sim_core::movement::Body::at(SEED, &haven, at.0, at.1);
        p.inv[HOTBAR_SLOTS] = revolver;
        p.inv[HOTBAR_SLOTS + 1] = junk;
    });
    let done = h.until(120 * TICK_HZ, |b| {
        b.history
            .iter()
            .any(|r| r.goal == Goal::Research && r.outcome == Outcome::Done)
    });
    assert!(done, "no research: {:?} {}", h.bot.oven_stats, h.explain());
    let research = h.shard.world.research;
    let row = *research.row_for(revolver.item).unwrap();
    // The read goes out as the goal ends; its answer, a moment later.
    h.until(3 * TICK_HZ, |_| false);
    let me = h.me();
    assert!(
        sim_core::research::knows(me.known, row.recipe),
        "the paper was not read: {:?} {:?} {:?}",
        h.bot.oven_stats,
        h.verbs,
        me.inv.iter().filter(|s| s.count > 0).collect::<Vec<_>>()
    );
    assert_eq!(units_of(me, revolver.item), 0, "the revolver became paper");
    assert_eq!(
        units_of(me, research.blueprint),
        0,
        "and the paper was read"
    );
    assert_eq!(units_of(me, junk.item), 100 - u32::from(row.cost));
    assert_eq!(h.bot.oven_stats.papers, 1);
    assert_eq!(h.bot.oven_stats.placed, 0, "THE GATE's table, not its own");
    for verb in ["container", "move", "use", "research"] {
        assert!(h.verbs.contains(verb), "{verb} never sent: {:?}", h.verbs);
    }
    assert_eq!(h.heap_ops, 0, "the frame path allocated");
}

/// An open work takes its quota at its terminal: carrying what its first
/// line takes, the agent walks there and puts it in.
#[test]
fn a_survivor_feeds_an_open_work() {
    let mind = Mind::inline(VisitWhenOffered, MindConfig::default()).unwrap();
    let haven = sim_core::terrain::haven(SEED);
    let content = common::content();
    let works = server::net::bake_all(&content).unwrap().works;
    // The first work whose terminal stands somewhere.
    let (k, (sx, _, sz)) = (0..usize::from(works.count))
        .find_map(|k| {
            let def = works.get(k)?;
            sim_core::spot::world(&haven, &def.spot).map(|p| (k, p))
        })
        .expect("a work with a terminal");
    let def = *works.get(k).unwrap();
    let input = def.inputs[0];
    let at = (sx + 12.0, sz);
    let mut h = Harness::with_mind(false, at, mind);
    h.until(5, |_| false);
    for core in [&mut h.shard, &mut h.replay] {
        let w = &mut core.world;
        w.works.w[k].state = sim_core::works::WORK_OPEN;
        let def = w.works_def;
        w.works.refresh(&def);
    }
    let catalog = server::net::bake_all(&content).unwrap().catalog;
    let give = sim_core::gather::ItemStack {
        item: input.item,
        count: 50.min(input.need as u16).max(1),
        cond: catalog.cond_max(input.item as usize),
        skin: 0,
        ..sim_core::gather::ItemStack::EMPTY
    };
    h.stage(move |p| {
        p.body = sim_core::movement::Body::at(SEED, &haven, at.0, at.1);
        p.inv[HOTBAR_SLOTS] = give;
    });
    let fed = h.until(90 * TICK_HZ, |b| {
        b.history
            .iter()
            .any(|r| r.goal == Goal::FeedWork && r.outcome == Outcome::Done)
    });
    assert!(fed, "no feed: {}", h.explain());
    assert_eq!(
        h.shard.world.works.w[k].got[0],
        u32::from(give.count),
        "it went in"
    );
    assert_eq!(units_of(h.me(), input.item), 0);
    assert!(h.verbs.contains("arc"), "{:?}", h.verbs);
    assert_eq!(h.heap_ops, 0, "the frame path allocated");
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
/// crafted at it and goes on one of its own behind the stair cell (each put
/// down from behind it, where the hand sees it), and ore staged afterwards
/// is smelted there into fragments, a batch at a time.
#[test]
fn a_survivor_puts_down_its_bench_and_furnace_and_smelts_ore() {
    use server::agent::build::{Milestone, ANNEX, YARD};
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
        let (fx, fz) = (
            cx.checked_add_signed(i16::from(ANNEX.0)).unwrap(),
            cz.checked_add_signed(i16::from(ANNEX.1)).unwrap(),
        );
        assert_eq!(arch(fx, fz, 0), Some(ARCH_FURNACE), "the furnace behind");
        let (yx, yz) = (
            cx.checked_add_signed(i16::from(YARD.0)).unwrap(),
            cz.checked_add_signed(i16::from(YARD.1)).unwrap(),
        );
        assert_eq!(arch(yx, yz, 0), Some(ARCH_WORKBENCH), "the bench behind");
        // The kit's fragments paid for the bench. A feed takes a chunk of
        // everything the cupboard eats, and a base of wood and stone is
        // charged nothing in fragments: fed, they would have sat in its
        // stock, and the bench waited on loot runs for a hundred more.
        let frags = content.item_index("item.metal_frags").unwrap();
        let row = w.deploy.mats[..usize::from(w.deploy.mat_count)]
            .iter()
            .position(|&m| m == frags)
            .expect("the cupboard eats fragments");
        let hearth = w
            .deploys
            .hearths()
            .iter()
            .find(|r| r.owner == ID)
            .expect("its cupboard");
        assert_eq!(
            hearth.stock[row],
            0,
            "the cupboard was fed fragments: {}",
            h.explain()
        );
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
    // Smelted, not merely held: a road sign pays fragments too
    // (`loot.roadsign`), and a body whose base stands by the road can
    // smash its way to the count with the ore untouched in its pack.
    let smelted = h.until(5 * 60 * TICK_HZ, |b| {
        b.held("Metal Fragments") >= before + 40 && b.held("Metal Ore") <= 20
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
    // Code locks on the doors, as a guild's founder hangs them: a body
    // living alone puts key locks on with its doors and keeps them.
    let opts = server::explorer::SurvivorOpts {
        lock: Some(LockSecret::Code(code)),
        temperament: server::agent::combat::Temperament::Defensive,
        code_doors: true,
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
        gave_up |= h
            .bot
            .memory()
            .last
            .is_some_and(|r| r.goal == Goal::GoHome && r.outcome == Outcome::Failed(Why::Door));
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
    for item in ["item.hatchet_metal", "item.pickaxe_metal", "item.medkit"] {
        assert!(belted(item), "{item} not on the belt: {}", h.explain());
    }
    // The crossbow is made; the belt's ranged slot holds the best ranged
    // weapon owned, which is a revolver when the walk turned one up
    // (`loadout::RANGED`).
    assert!(units_of(me, id("item.crossbow")) >= 1, "{}", h.explain());
    assert!(
        belted("item.crossbow") || belted("item.revolver"),
        "no ranged weapon on the belt: {}",
        h.explain()
    );
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
/// bench in the first one's place behind the core, gunpowder made at it,
/// the pistol round, the revolver and the roadsign vest learned at that
/// bench's tree (tier two: `E` on the second bench, which the harness
/// checks at every unlock), made, and the vest worn.
#[test]
fn a_survivor_builds_its_second_bench_and_learns_the_revolver_and_vest() {
    use server::agent::build::{Milestone, YARD};
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
    // Four thousand each, not six: the base costs far less, and the surplus
    // filled the pack and the one box between them, so two salvage pieces
    // off a roadside barrel left no slot for the gunpowder it then makes
    // (a full pack is never "ready", and it explored for good).
    let mut kit = vec![stack("item.wood", 1000); 4];
    kit.extend([stack("item.stone", 1000); 4]);
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
    // Gunpowder waits on THE CRUCIBLE; this test is about the bench and the
    // tree, so the island has already lit it.
    h.light_works();
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
        plan.cx.checked_add_signed(i16::from(YARD.0)).unwrap(),
        plan.cz.checked_add_signed(i16::from(YARD.1)).unwrap(),
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
/// and put down in front of it, opened (`E` on the fire), wood in the fuel
/// slot and the meat on the grill with its panel open, lit (`C` on the same
/// fire), each piece taken off as soon as it is done, the fire put out.
/// Hungry, it eats the cooked meat.
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
    // Rust's camp fire has one grill slot, so ten pieces cook one after
    // another: 200 s of cooking alone.
    let done = h.until(6 * 60 * TICK_HZ, |b| {
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
    let key = sim_core::deploy::box_key(fire.cx, fire.cz, 0, 0);
    let held = w
        .deploys
        .boxes()
        .iter()
        .find(|b| sim_core::deploy::box_key(b.cx, b.cz, b.level, 0) == key)
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
            pose,
        } => protocol::encode_action_deploy(row, cx, cz, level, loc, pose, buf),
        ActionMsg::Upgrade {
            cx,
            cz,
            level,
            loc,
            material,
        } => protocol::encode_action_upgrade(cx, cz, level, loc, material, buf),
        ActionMsg::Team { op, target } => protocol::encode_action_team(op, target, buf),
        ActionMsg::Use { cx, cz, level, loc } => {
            protocol::encode_action_use(cx, cz, level, loc, buf)
        }
        ActionMsg::Access {
            cx,
            cz,
            level,
            loc,
            op,
            code,
        } => protocol::encode_action_access(cx, cz, level, loc, op, code, buf),
        _ => panic!("the puppet builds, teams up and works doors, and nothing else: {msg:?}"),
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
            say: server::mind::Say::Quiet,
            code: None,
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
            say: server::mind::Say::Quiet,
            code: None,
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
        pose: sim_core::footprint::Pose::CENTRE,
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
        let key = sim_core::deploy::box_key(bx + 1, bz, 0, 0);
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
    // No duds here. Which plant duds is a hash of its tick (`charge::dud`),
    // so anything that moves the raider's timeline moves it, and this test
    // counts the door's hp in satchels, not the dud's walk back to the wall.
    let satchel_ix = usize::from(content.item_index("item.satchel_charge").unwrap());
    for core in [&mut h.shard, &mut h.replay] {
        core.world.combat.throw[satchel_ix].dud_pct = 0;
    }
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

/// The ancients' decision source: a lock it can solve; a stone close by
/// it has not read; a card door it holds the card for; then the room's
/// crates; else, while `.0` says so, a walk (out of a room, if it stands
/// in one), or a stand and a look round. The test flips `.0`.
struct Ancients(std::sync::Arc<std::sync::atomic::AtomicBool>);

impl Ancients {
    fn new(explore: bool) -> (Self, std::sync::Arc<std::sync::atomic::AtomicBool>) {
        let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(explore));
        (Self(flag.clone()), flag)
    }
}

impl server::mind::DecisionSource for Ancients {
    fn kind(&self) -> server::mind::SourceKind {
        server::mind::SourceKind::Scripted
    }

    fn decide(&mut self, s: &server::mind::Summary) -> Result<server::mind::Choice, String> {
        use server::mind::Distance;
        let near = s
            .unread_stone
            .is_some_and(|p| matches!(p.distance, Distance::Here | Distance::Near));
        let goal = if s.offers(Goal::SolveLock) {
            Goal::SolveLock
        } else if near && s.offers(Goal::ReadStone) {
            Goal::ReadStone
        } else if s.offers(Goal::SwipeCard) {
            Goal::SwipeCard
        } else if s.offers(Goal::Loot) {
            Goal::Loot
        } else if self.0.load(std::sync::atomic::Ordering::Relaxed) {
            Goal::Explore
        } else {
            Goal::Wait
        };
        Ok(server::mind::Choice {
            goal,
            confidence: 1.0,
            reason: server::mind::Reason::EMPTY,
            input_tokens: 0,
            output_tokens: 0,
            say: server::mind::Say::Quiet,
            code: None,
        })
    }
}

/// Where stone or lock `spot` stands, and a standing point `back` metres
/// off it toward the island's middle, on the ground.
fn off_spot(spot: &sim_core::spot::Spot, back: f32) -> ([f32; 3], (f32, f32)) {
    let haven = sim_core::terrain::haven(SEED);
    let (x, y, z) = sim_core::spot::world(&haven, spot).unwrap();
    let c = sim_core::terrain::ISLAND_SIZE * 0.5;
    let (dx, dz) = (c - x, c - z);
    let d = dx.hypot(dz).max(1.0);
    ([x, y, z], (x + dx / d * back, z + dz / d * back))
}

/// THE RING LOCK's answer is written on the ring word in digits, and the
/// digits are learnt at the count stone. The agent knows no glyph. Set
/// down near the ring word first, it reads it and has no answer: the
/// digits are shapes to it. Near the count stone, it reads that and learns
/// them; what it read at the ring word now gives the answer. At the lock,
/// it walks to each dial and turns it a notch a press until it shows the
/// answer's digit, and the last turn pays it the green keycard.
#[test]
fn a_survivor_reads_the_stones_and_turns_the_ring_lock_to_their_answer() {
    let content = common::content();
    let t = server::net::bake_all(&content).unwrap();
    let find = |text: &str| {
        t.arc_text
            .inscriptions
            .iter()
            .position(|s| s.starts_with(text))
            .unwrap()
    };
    let (word, count) = (find("AT THE RING"), find("COUNT THE RINGS"));
    let (_, word_at) = off_spot(&t.lore.inscriptions[word].spot, 12.0);
    let (_, count_at) = off_spot(&t.lore.inscriptions[count].spot, 12.0);
    let lock = t.mechs.defs[0];
    let (_, lock_at) = off_spot(&lock.spot, 12.0);
    let mind = Mind::inline(Ancients::new(false).0, MindConfig::default()).unwrap();
    let mut h = Harness::with_mind(false, word_at, mind);
    let haven = h.shard.world.haven;
    let place = move |(x, z): (f32, f32)| sim_core::movement::Body::at(SEED, &haven, x, z);
    h.until(5, |_| false);
    assert_eq!(h.me().glyphs, 0, "it reads nothing yet");
    let reads = |h: &Harness| {
        h.bot
            .history
            .iter()
            .filter(|r| r.goal == Goal::ReadStone && r.outcome == Outcome::Done)
            .count()
    };
    // 1. The ring word, unread digits and all.
    h.until(90 * TICK_HZ, |b| {
        b.history
            .iter()
            .any(|r| r.goal == Goal::ReadStone && r.outcome == Outcome::Done)
    });
    assert!(
        reads(&h) >= 1,
        "the ring word was never read: {}",
        h.explain()
    );
    let core = h.bot.core().unwrap();
    assert!(core.lore.inscriptions[word].len > 0, "its words came");
    assert_eq!(
        server::agent::lore::answer(&core.lore, 0),
        None,
        "but no digits"
    );
    // 2. The count stone: the digits.
    let at = place(count_at);
    h.stage(move |p| p.body = at);
    let learnt = h.until(90 * TICK_HZ, |b| {
        b.core()
            .is_some_and(|c| server::agent::lore::answer(&c.lore, 0).is_some())
    });
    assert!(learnt, "the digits were never learnt: {}", h.explain());
    let core = h.bot.core().unwrap();
    let want = server::agent::lore::answer(&core.lore, 0).unwrap();
    let truth = sim_core::mech::answers(h.shard.world.arc.salt, SEED, 0, &lock);
    assert_eq!(want, truth, "the answer it read is the lock's");
    // 3. The lock.
    let at = place(lock_at);
    h.stage(move |p| p.body = at);
    let card = content.item_index("item.keycard_green").unwrap();
    let solved = h.until(180 * TICK_HZ, |b| {
        b.history
            .iter()
            .any(|r| r.goal == Goal::SolveLock && r.outcome == Outcome::Done)
    });
    assert!(solved, "the lock was never solved: {}", h.explain());
    assert_eq!(units_of(h.me(), card), 1, "the green keycard is its pay");
    assert_eq!(h.shard.world.arc.mechs[0].solves, 1);
    assert!(
        h.verbs.contains("arc") && h.pick_checks >= 3,
        "{:?}",
        h.verbs
    );
    assert_eq!(h.heap_ops, 0, "the frame path allocated");
}

/// A green keycard in the pack, outside the Black Ziggurat's rear portal:
/// the agent walks into the entry hall to the green door's reader, swipes,
/// and walks into the green room while the door stands open. There it
/// empties the room's crates, the blue card among what they hold, and
/// walks out again.
#[test]
fn a_survivor_swipes_into_the_green_room_and_loots_the_blue_card() {
    let haven = sim_core::terrain::haven(SEED);
    let zig = haven.ziggurat;
    assert!(zig.live, "the fixture seed has a ziggurat");
    let (x, z) = sim_core::kit::to_world(&zig.placed(), 0.0, 40.0);
    let content = common::content();
    let (ancients, explore) = Ancients::new(false);
    let mind = Mind::inline(ancients, MindConfig::default()).unwrap();
    let mut h = Harness::with_mind(false, (x, z), mind);
    h.until(5, |_| false);
    let green = sim_core::gather::ItemStack {
        count: 1,
        ..common::stack(&content, "item.keycard_green")
    };
    let at = sim_core::movement::Body::at(SEED, &haven, x, z);
    h.stage(move |p| {
        p.body = at;
        p.inv[HOTBAR_SLOTS] = green;
    });
    let room = |h: &Harness| {
        let me = h.me().body;
        server::agent::lore::room_of(
            &zig,
            me.qx as f32 * sim_core::movement::POS_XZ_Q,
            me.qy as f32 * sim_core::movement::POS_Y_Q,
            me.qz as f32 * sim_core::movement::POS_XZ_Q,
        )
    };
    let swiped = h.until(120 * TICK_HZ, |b| {
        b.history.iter().any(|r| r.goal == Goal::SwipeCard)
    });
    assert!(swiped, "no swipe ended: {}", h.explain());
    let report = h
        .bot
        .history
        .iter()
        .find(|r| r.goal == Goal::SwipeCard)
        .unwrap();
    assert_eq!(report.outcome, Outcome::Done, "{report:?}: {}", h.explain());
    assert_eq!(room(&h), Some(0), "in the green room");
    assert!(h.verbs.contains("swipe"));
    // The room's crates.
    let blue = content.item_index("item.keycard_blue").unwrap();
    for _ in 0..120 * TICK_HZ {
        if units_of(h.me(), blue) > 0 {
            break;
        }
        h.step();
    }
    assert!(
        units_of(h.me(), blue) > 0,
        "the blue card never came out of the green crate: {}",
        h.explain()
    );
    assert!(h.verbs.contains("container") && h.verbs.contains("move"));
    // The blue card goes up the chain another time. This run walks out of
    // the green room, by the door while it stands open (the hall's crates
    // are the loot run's next), else by the lever.
    drop_item(&mut h, blue);
    explore.store(true, std::sync::atomic::Ordering::Relaxed);
    for _ in 0..180 * TICK_HZ {
        if room(&h).is_none() {
            break;
        }
        h.step();
    }
    assert_eq!(
        room(&h),
        None,
        "never out of the green room: {}",
        h.explain()
    );
    assert_eq!(h.heap_ops, 0, "the frame path allocated");
}

/// The ziggurat test's staging: the agent in front of the front stairs
/// with `card` in its pack, and where it is in the ziggurat's own metres.
fn at_the_ziggurat(
    card: &str,
) -> (
    Harness,
    sim_core::monument::Ziggurat,
    std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    let haven = sim_core::terrain::haven(SEED);
    let zig = haven.ziggurat;
    assert!(zig.live, "the fixture seed has a ziggurat");
    let (x, z) = sim_core::kit::to_world(&zig.placed(), 0.0, -42.0);
    let content = common::content();
    let (ancients, explore) = Ancients::new(false);
    let mind = Mind::inline(ancients, MindConfig::default()).unwrap();
    let mut h = Harness::with_mind(false, (x, z), mind);
    h.until(5, |_| false);
    let card = sim_core::gather::ItemStack {
        count: 1,
        ..common::stack(&content, card)
    };
    let at = sim_core::movement::Body::at(SEED, &haven, x, z);
    h.stage(move |p| {
        p.body = at;
        p.inv[HOTBAR_SLOTS] = card;
    });
    (h, zig, explore)
}

/// Scene staging: every stack of `item` out of the agent's pack.
fn drop_item(h: &mut Harness, item: u16) {
    h.stage(move |p| {
        for s in p.inv.iter_mut().filter(|s| s.count > 0 && s.item == item) {
            *s = sim_core::gather::ItemStack::default();
        }
    });
}

/// The agent's card room, and whether it is up on the terraces.
fn where_in(h: &Harness, zig: &sim_core::monument::Ziggurat) -> (Option<u8>, bool) {
    let me = h.me().body;
    let (x, y, z) = (
        me.qx as f32 * sim_core::movement::POS_XZ_Q,
        me.qy as f32 * sim_core::movement::POS_Y_Q,
        me.qz as f32 * sim_core::movement::POS_XZ_Q,
    );
    (
        server::agent::lore::room_of(zig, x, y, z),
        server::agent::lore::aloft(zig, x, y, z),
    )
}

/// A blue keycard in the pack, in front of the Black Ziggurat: the route's
/// grid knows only the ground, so the agent walks the front flight up to
/// the first ledge and round to the blue door's reader on the east face,
/// swipes, and walks into the blue room. There it empties the room's crate
/// of the red card it guarantees. Sent walking once the door has shut, it
/// pulls the lever, walks out, and back round and down the flight.
#[test]
fn a_survivor_climbs_to_the_blue_door_loots_the_room_and_comes_back_down() {
    let (mut h, zig, explore) = at_the_ziggurat("item.keycard_blue");
    let content = common::content();
    let swiped = h.until(150 * TICK_HZ, |b| {
        b.history.iter().any(|r| r.goal == Goal::SwipeCard)
    });
    assert!(swiped, "no swipe ended: {}", h.explain());
    let report = h
        .bot
        .history
        .iter()
        .find(|r| r.goal == Goal::SwipeCard)
        .unwrap();
    assert_eq!(report.outcome, Outcome::Done, "{report:?}: {}", h.explain());
    assert_eq!(where_in(&h, &zig), (Some(1), true), "in the blue room");
    let red = content.item_index("item.keycard_red").unwrap();
    for _ in 0..120 * TICK_HZ {
        if units_of(h.me(), red) > 0 {
            break;
        }
        h.step();
    }
    assert!(
        units_of(h.me(), red) > 0,
        "no red card from the blue crate: {}",
        h.explain()
    );
    // The red card goes up the chain another time; this run waits out the
    // door, then walks: out, by the lever, and down. Any other card the crate
    // rolled goes too, or it walks off to swipe that one instead.
    drop_item(&mut h, red);
    for other in ["item.keycard_green", "item.keycard_blue"] {
        drop_item(&mut h, content.item_index(other).unwrap());
    }
    for _ in 0..90 * TICK_HZ {
        if h.shard.world.card_door_bits & 2 == 0 {
            break;
        }
        h.step();
    }
    assert_eq!(
        where_in(&h, &zig),
        (Some(1), true),
        "still in the blue room"
    );
    explore.store(true, std::sync::atomic::Ordering::Relaxed);
    let levers = h.bot.levers();
    for _ in 0..240 * TICK_HZ {
        if where_in(&h, &zig) == (None, false) {
            break;
        }
        h.step();
    }
    assert_eq!(
        where_in(&h, &zig),
        (None, false),
        "never back down: {}",
        h.explain()
    );
    assert!(h.bot.levers() > levers, "out by the lever");
    assert_eq!(h.heap_ops, 0, "the frame path allocated");
}

/// A red keycard: up all three front flights to the third ledge, round
/// its west side to the rear, and in at the red door to the sanctum.
#[test]
fn a_survivor_climbs_to_the_red_door_and_walks_into_the_sanctum() {
    let (mut h, zig, _) = at_the_ziggurat("item.keycard_red");
    let swiped = h.until(180 * TICK_HZ, |b| {
        b.history.iter().any(|r| r.goal == Goal::SwipeCard)
    });
    assert!(swiped, "no swipe ended: {}", h.explain());
    let report = h
        .bot
        .history
        .iter()
        .find(|r| r.goal == Goal::SwipeCard)
        .unwrap();
    assert_eq!(report.outcome, Outcome::Done, "{report:?}: {}", h.explain());
    assert_eq!(where_in(&h, &zig), (Some(2), true), "in the sanctum");
    assert!(h.verbs.contains("swipe"));
    assert_eq!(h.heap_ops, 0, "the frame path allocated");
}

/// A small open base the puppet builds by hand, the starter's layout without
/// its doors: two foundations, the core walled on three sides below, the L
/// flight up from the second cell (open to the island on its far side), and
/// a floor over the core. Returns the plot's corner in metres and the ground
/// floor's walk height; the puppet is left outside.
fn stage_stair_base(h: &mut Harness, near: (f32, f32)) -> (f32, f32, f32) {
    use server::population::base_rows;
    use sim_core::bots::{BasePlan, STARTER_FOOTPRINT, STARTER_STAND_M};
    use sim_core::build::{
        anchor, build_cell_of, column_floor_y, foundation_terrain_ok, terrain_band, BUILD_CELL_M,
        LOC_EDGE_XLO, LOC_EDGE_ZLO, LOC_PLANE, LOC_RISER_ZLO,
    };

    let haven = h.shard.world.haven;
    let buildable = |cx: u16, cz: u16| {
        let (ax, az) = anchor(cx, cz, LOC_PLANE);
        foundation_terrain_ok(SEED, &haven, ax, az)
            && !sim_core::terrain::build_reserved(&haven, ax, az, BUILD_CELL_M * 1.5)
    };
    let (c0, r0) = (build_cell_of(near.0) as i32, build_cell_of(near.1) as i32);
    let mut plot = None;
    'search: for r in 0..64i32 {
        for dz in -r..=r {
            for dx in -r..=r {
                if dx.abs() != r && dz.abs() != r {
                    continue;
                }
                let (cx, cz) = ((c0 + dx) as u16, (r0 + dz) as u16);
                let band = terrain_band(SEED, &haven, cx, cz);
                let even = [(0, 0), (1, 0), (2, 0), (0, 1), (1, 1)]
                    .iter()
                    .all(|&(ox, oz)| {
                        (terrain_band(SEED, &haven, cx + ox, cz + oz) - band).abs() <= 1
                    });
                let plan = BasePlan::new(0, cx, cz);
                if even
                    && STARTER_FOOTPRINT.iter().all(|&(dx, dz)| {
                        let (x, z) = plan.cell(dx, dz);
                        buildable(x, z)
                    })
                {
                    plot = Some((cx, cz));
                    break 'search;
                }
            }
        }
    }
    let (bx, bz) = plot.expect("an even plot near the scene");
    let content = common::content();
    let stack = |id: &str, count: u16| sim_core::gather::ItemStack {
        count,
        ..common::stack(&content, id)
    };
    let rows = base_rows(&content).unwrap();
    h.with_puppet();
    h.until(2, |_| false);
    let (x0, z0) = (f32::from(bx) * BUILD_CELL_M, f32::from(bz) * BUILD_CELL_M);
    let stand = (x0 + STARTER_STAND_M.0, z0 + STARTER_STAND_M.1);
    let pocket = [
        stack("item.wood", 1000),
        stack("item.wood", 1000),
        stack("item.wood", 1000),
        stack("item.stone", 1000),
        stack("item.hearth", 1),
    ];
    for core in [&mut h.shard, &mut h.replay] {
        let haven = core.world.haven;
        let p = core
            .world
            .players
            .iter_mut()
            .find(|p| p.active && p.id == PUPPET)
            .unwrap();
        p.body = sim_core::movement::Body::at(SEED, &haven, stand.0, stand.1);
        for (i, s) in pocket.iter().enumerate() {
            p.inv[HOTBAR_SLOTS + i] = *s;
        }
    }
    let place = |row: u16, cx: u16, cz: u16, level: u8, loc: u8| ActionMsg::Place {
        row,
        cx,
        cz,
        level,
        loc,
        freehand: false,
        plate: 0,
    };
    let acts = [
        place(rows.foundation, bx, bz, 0, LOC_PLANE),
        ActionMsg::Deploy {
            row: rows.hearth,
            cx: bx,
            cz: bz,
            level: 0,
            loc: LOC_PLANE,
            pose: sim_core::footprint::Pose::CENTRE,
        },
        place(rows.foundation, bx + 1, bz, 0, LOC_PLANE),
        place(rows.wall, bx, bz, 0, LOC_EDGE_XLO),
        place(rows.wall, bx, bz, 0, LOC_EDGE_ZLO),
        place(rows.wall, bx, bz + 1, 0, LOC_EDGE_ZLO),
        place(rows.wall, bx + 1, bz, 0, LOC_EDGE_ZLO),
        place(rows.wall, bx + 2, bz, 0, LOC_EDGE_XLO),
        place(rows.stairs, bx + 1, bz, 0, LOC_RISER_ZLO),
        place(rows.floor, bx, bz, 1, LOC_PLANE),
    ];
    for a in acts {
        h.puppet_act(a);
    }
    for core in [&mut h.shard, &mut h.replay] {
        let w = &core.world;
        assert!(
            w.pieces.find(bx + 1, bz, 0, LOC_RISER_ZLO).is_some(),
            "the flight"
        );
        assert!(
            w.pieces.find(bx, bz, 1, LOC_PLANE).is_some(),
            "the upper floor"
        );
    }
    let plate = h.shard.world.pieces.cols().plate(bx, bz).unwrap_or(0);
    let floor = column_floor_y(SEED, &haven, bx, bz, plate);
    (x0, z0, floor)
}

/// Feet height of a body, metres.
fn feet_y(p: &sim_core::world::Player) -> f32 {
    p.body.qy as f32 * sim_core::movement::POS_Y_Q
}

/// **Up the stairs after a teammate, and back down after them**
/// (`agent::floors`): the puppet stands on the floor over the core, and the
/// agent, on the ground at the foot of the L flight, climbs it and walks out
/// onto that floor to them. Then the puppet is outside on the island, and
/// the agent comes down the same flight to it. Both shards hash-identical,
/// the frame path allocation-free.
#[test]
fn a_teammate_follows_up_the_stairs_and_back_down() {
    let at = common::clearing(3);
    let mind = Mind::inline(FollowWhenOffered, MindConfig::default()).unwrap();
    let mut h = Harness::with_mind(false, at, mind);
    h.until(5, |_| false);
    let (x0, z0, floor) = stage_stair_base(&mut h, at);
    let haven = h.shard.world.haven;
    let place = move |x: f32, z: f32, y: Option<f32>| {
        let mut b = sim_core::movement::Body::at(SEED, &haven, x, z);
        if let Some(y) = y {
            b.qy = sim_core::movement::quant_y(y);
        }
        b
    };
    // Teamed up beside each other, outside the stair cell's open side.
    let outside = place(x0 + 4.5, z0 + 4.5, None);
    h.stage(move |p| p.body = outside);
    h.stage_puppet(move |p| p.body = outside);
    h.puppet_act(ActionMsg::Team {
        op: protocol::TEAM_OP_INVITE,
        target: ID,
    });
    let joined = |h: &Harness| h.shard.teams.team_of(ID).len() == 2;
    for _ in 0..30 * TICK_HZ {
        if joined(&h) {
            break;
        }
        h.step();
    }
    assert!(joined(&h), "the agent never joined: {}", h.explain());

    // Up: the puppet over the core, the agent at the foot of the flight.
    let upstairs = floor + sim_core::build::LEVEL_H_M;
    let mate = place(x0 + 1.5, z0 + 1.5, Some(upstairs));
    let foot = place(x0 + 4.5, z0 + 2.4, Some(floor));
    h.stage_puppet(move |p| p.body = mate);
    h.stage(move |p| p.body = foot);
    let up = |h: &Harness| {
        let me = h.me();
        let (x, z) = (me.body.qx as f32 * 0.03, me.body.qz as f32 * 0.03);
        (feet_y(me) - upstairs).abs() < 0.3
            && (x - (x0 + 1.5)).hypot(z - (z0 + 1.5)) <= server::explorer::FOLLOW_NEAR_M
    };
    for _ in 0..60 * TICK_HZ {
        if up(&h) {
            break;
        }
        h.step();
    }
    assert!(
        up(&h),
        "never got upstairs to its teammate (y {:.2}, floor {:.2}): {:?} {}",
        feet_y(h.me()),
        upstairs,
        h.bot.route().stats,
        h.explain()
    );

    // Down: the puppet back outside on the ground.
    h.stage_puppet(move |p| p.body = outside);
    let down = |h: &Harness| {
        let me = h.me();
        let (x, z) = (me.body.qx as f32 * 0.03, me.body.qz as f32 * 0.03);
        feet_y(me) < floor + 1.0
            && (x - (x0 + 4.5)).hypot(z - (z0 + 4.5)) <= 5.0
            && h.bot.route().stats.flights >= 2
    };
    for _ in 0..60 * TICK_HZ {
        if down(&h) {
            break;
        }
        h.step();
    }
    assert!(
        down(&h),
        "never came down to its teammate (y {:.2}): {:?} {}",
        feet_y(h.me()),
        h.bot.route().stats,
        h.explain()
    );
    assert_eq!(h.heap_ops, 0, "the frame path allocated");
}

/// **Left upstairs, it comes down for a ground goal** rather than standing
/// on the floor's edge: put on the floor over the core and sent exploring,
/// it takes the flight down before anything else, and never fails stuck.
#[test]
fn an_agent_put_upstairs_comes_down_for_a_ground_goal() {
    let at = common::clearing(3);
    let mind = Mind::inline(Prefer(&[Goal::Explore], None), MindConfig::default()).unwrap();
    let mut h = Harness::with_mind(false, at, mind);
    h.until(5, |_| false);
    let (x0, z0, floor) = stage_stair_base(&mut h, at);
    let haven = h.shard.world.haven;
    let upstairs = floor + sim_core::build::LEVEL_H_M;
    let mut b = sim_core::movement::Body::at(SEED, &haven, x0 + 1.5, z0 + 1.5);
    b.qy = sim_core::movement::quant_y(upstairs);
    h.stage(move |p| p.body = b);
    let down = |h: &Harness| feet_y(h.me()) < floor + 1.0 && h.bot.route().stats.flights >= 1;
    for _ in 0..40 * TICK_HZ {
        if down(&h) {
            break;
        }
        h.step();
    }
    assert!(
        down(&h),
        "never came down (y {:.2}): {:?} {}",
        feet_y(h.me()),
        h.bot.route().stats,
        h.explain()
    );
    assert!(
        h.bot.route().stats.flights >= 1,
        "{:?}",
        h.bot.route().stats
    );
    assert!(
        !h.bot
            .history
            .iter()
            .any(|r| r.outcome == server::mind::Outcome::Failed(Why::Stuck)),
        "a goal failed stuck on the way down"
    );
    assert_eq!(h.heap_ops, 0, "the frame path allocated");
}

/// The minds we ship hold no tools. A mind's inputs include strangers'
/// chat, so a mind that could also run something is the one road from a
/// chat line on the island to an act off it. Both of `claude_agent.py`'s
/// paths ask for a schema answer and nothing else, and the daily recap
/// asks the same way; a change to that is a decision, made here.
#[test]
fn the_minds_we_ship_hold_no_tools() {
    let agent =
        std::fs::read_to_string(root().join("crates/server/examples/claude_agent.py")).unwrap();
    assert!(
        agent.contains(r#""--tools", "", "--disallowedTools", "mcp__*", "--strict-mcp-config""#),
        "the Claude Code path must run with no built-in tools and no MCP server"
    );
    for file in [
        "crates/server/examples/claude_agent.py",
        "ops/buddy/recap.py",
    ] {
        let text = std::fs::read_to_string(root().join(file)).unwrap();
        assert!(
            !text.contains("tools=") && !text.contains("\"tools\":"),
            "{file} hands a model tools"
        );
    }
}
