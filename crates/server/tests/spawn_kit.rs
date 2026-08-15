//! `spawn_kit` — what a body on the SHIPPED island actually wakes holding,
//! on both of the doors that grant it (DECISIONS.md 2026-08-15: *"we should
//! be starting with a rock and a torch"*, *"you cant smash trees with ur hans
//! lol u need a rock"*).
//!
//! **Why this lives in `crates/server/tests/` rather than beside the other
//! two halves.** `crates/content/tests/content.rs` can read the shipped TOML
//! but cannot seat a body; `crates/sim-core/tests/bag_respawn.rs` can seat a
//! body and die on it but has no content — sim-core does not depend on the
//! content crate, by design (wall 7 runs one way). This crate is the first
//! one that holds both, so it is the only place the sentence *"a fresh spawn
//! on the shipped island holds a rock and a torch, and so does a respawn"*
//! can be written as an assertion instead of as two halves that a reader has
//! to join.
//!
//! No sockets and no clock: a `World`, shipped bakes, and ticks.

use sim_core::gather::{ItemStack, NO_ITEM};
use sim_core::limits::TICK_HZ;
use sim_core::survival::SurvivalContent;
use sim_core::world::{Command, World};

/// The island the shard ships (DECISIONS.md 2026-08-14 kept this seed).
const SEED: u64 = 20260731;

fn shipped() -> content::Content {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content");
    content::Content::load_dir(&dir).expect("shipped content loads")
}

/// A world with the shipped kit and combat table, and the **probe** survival
/// clock.
///
/// The clock is the murder weapon and nothing else — its spans are seconds so
/// a whole death fits inside a test, exactly as `sim-core`'s `bag_respawn`
/// uses it. Substituting it does not weaken anything asserted below: no
/// assertion here reads a survival number, and the shipped clock would only
/// make the same death take five sim-minutes of ticks.
///
/// `backpack` is left inert, so `World::die` destroys the carried inventory
/// outright instead of standing a bag up. That is the harsher of the two
/// paths for the question this file asks — there is no bag to walk back to —
/// and it is what makes "the body woke with the kit" unambiguous.
fn shipped_world(c: &content::Content) -> Box<World> {
    let mut w = Box::new(World::new(SEED));
    w.gather = c.bake_gather().expect("gather bakes");
    w.combat = c.bake_combat().expect("weapons bake");
    w.spawn_kit = c.bake_spawn_kit().expect("the kit bakes");
    w.survival = SurvivalContent::probe_fixture();
    w
}

/// Every stack a body is carrying, in slot order, empties dropped.
fn pockets(w: &World, slot: usize) -> Vec<(u16, u16)> {
    w.players[slot]
        .inv
        .iter()
        .filter(|s| s.count > 0)
        .map(|s| (s.item, s.count))
        .collect()
}

/// Empty both meters and let the probe clock take the body. Returns on the
/// death screen, exactly as `bag_respawn::die` does.
fn die(w: &mut World) {
    let before = w.players[0].deaths;
    w.players[0].food = 0;
    w.players[0].water = 0;
    for _ in 0..120 * TICK_HZ {
        w.tick(&[]);
        if w.players[0].deaths > before {
            assert!(w.players[0].dead, "the body woke by itself");
            return;
        }
    }
    panic!("the probe clock never killed the body — the fixture changed");
}

/// **A fresh spawn holds exactly a rock and a torch, and a respawn holds them
/// too.** The whole of `NOW.md` §0kit's first and third parts, on shipped
/// content, in one gate.
///
/// The respawn half is the one that matters: it is `NOW.md` §0die's third
/// mechanism, which was a defect rather than a call to re-take. Before
/// 2026-08-15 the kit was granted on the fresh-spawn arm of `World::seat` and
/// nowhere else, so a body that died woke naked and — with bare hands paying
/// nothing on any node since the same day — stayed that way for good.
///
/// Proven red twice, once per half:
///
/// - delete `inventory::grant_kit` from `World::wake` and the second
///   `assert_eq!` reads `[]` against `[(rock, 1), (torch, 1)]`;
/// - put any other row back in `content/balance.toml`'s `[[spawn_kit]]` and
///   the first one fails, naming what was added.
#[test]
fn a_fresh_spawn_and_a_respawn_both_hold_a_rock_and_a_torch() {
    let c = shipped();
    let mut w = shipped_world(&c);
    let rock = c.item_index("item.rock").expect("the rock is shipped");
    let torch = c.item_index("item.torch").expect("the torch is shipped");
    let want = vec![(rock, 1u16), (torch, 1u16)];

    w.tick(&[Command::Join { id: 1 }]);
    assert_eq!(
        pockets(&w, 0),
        want,
        "a fresh spawn on the shipped island is not a rock and a torch"
    );

    // Farm something first, so the death has a thing to take that the kit
    // will not put back. Without this the respawn assertion would hold on a
    // world where death took nothing at all.
    let wood = c.item_index("item.wood").expect("wood is shipped");
    w.players[0].inv[4] = ItemStack {
        item: wood,
        count: 300,
    };

    die(&mut w);
    w.tick(&[Command::Respawn {
        id: 1,
        on_bag: false,
    }]);
    assert!(!w.players[0].dead, "the body did not wake");
    assert_eq!(
        pockets(&w, 0),
        want,
        "a respawned body did not wake with the kit — `NOW.md` §0die \
         mechanism 3 is back"
    );
}

/// The kit is worth granting: the rock it hands out is a live tool on every
/// swung node and a live weapon in a fist-fight, off the shipped bakes.
///
/// This is the assertion that makes the two halves of §0kit one decision
/// rather than two. Bare hands pay nothing now, so a kit whose tool the
/// gather table did not answer to would be a naked spawn wearing a kit — and
/// nothing in `sim-core` could tell, because a zero yield is legal content.
///
/// Proven red by deleting the `"item.rock"` row from any node in
/// `content/gatherables.toml`: that node then pays the rock its (absent, so
/// zero) hand row and the loop fails on it by name.
#[test]
fn the_kit_s_rock_is_a_live_tool_and_a_live_weapon() {
    let c = shipped();
    let gc = c.bake_gather().expect("gather bakes");
    let combat = c.bake_combat().expect("weapons bake");
    let rock = c.item_index("item.rock").expect("the rock is shipped");

    let mut swung = 0;
    for (i, node) in gc.nodes.iter().enumerate() {
        // A one-hit node is a pickup, not a swing — the bush, which keeps
        // its `hand` row on purpose (content/gatherables.toml).
        if node.output == NO_ITEM || node.hits < 2 {
            continue;
        }
        swung += 1;
        assert!(
            node.yield_for(rock) > 0,
            "node {i} pays the spawn kit's rock nothing — a fresh spawn \
             cannot start the loop"
        );
        assert_eq!(
            node.yield_for(NO_ITEM),
            0,
            "node {i} still pays bare hands (DECISIONS.md 2026-08-15)"
        );
    }
    assert!(
        swung >= 4,
        "only {swung} swung nodes reached the sim — this gate got thin"
    );

    // And the food loop is reachable from the beach: `hunt.rs` needs *some*
    // pocket a fresh character owns to be able to kill, and since the kit
    // shrank there is exactly one candidate.
    assert!(
        combat.held_melee(rock).is_some(),
        "the rock is not a live melee weapon — a fresh spawn cannot hunt, \
         and the kit no longer carries anything else that swings"
    );
}

/// The kit a respawn re-grants must stay cheap enough to re-grant, and
/// "cheap" has an arithmetic meaning here: **every entry is a single hand
/// item, and every one of them is craftable from what the world pays.**
///
/// The old nine-entry kit failed both halves — 900 wood is a stack of goods,
/// not a tool — and that is precisely why `inventory::grant_kit` refused to
/// run on `wake` until the kit changed. A material row added back to
/// `[[spawn_kit]]` would re-open the item printer with no other gate in this
/// repo able to see it: `sim-core` cannot tell a rock from a stack of wood,
/// and the content crate's own kit test asserts the same property one layer
/// up, on the TOML rather than on the baked table the sim runs.
///
/// Proven red by adding `[[spawn_kit]] item = "item.wood" count = 900` back
/// to `content/balance.toml` — the count assertion fires naming wood.
#[test]
fn the_re_granted_kit_is_tools_and_not_goods() {
    let c = shipped();
    let kit = c.bake_spawn_kit().expect("the kit bakes");
    let craftable: Vec<&str> = c.recipes.iter().map(|r| r.output.as_str()).collect();

    for e in &c.balance.spawn_kit {
        let item = c.item(&e.item).expect("the kit names a shipped item");
        assert_eq!(
            e.count, 1,
            "`{}` grants {} — a kit entry above 1 is a stack of goods, and \
             `World::wake` re-grants this on every death",
            e.item, e.count
        );
        assert_eq!(
            item.slot,
            content::schema::EquipSlot::Hand,
            "`{}` is not a hand item",
            e.item
        );
        // A spawn who loses one must be able to make another rather than
        // waiting for a death — `reference/DURABILITY.md` §4's whole point
        // about the reference's own bootstrap tool.
        assert!(
            craftable.contains(&e.item.as_str()),
            "`{}` is in the spawn kit and nothing crafts it — losing it is \
             a dead end until you die",
            e.item
        );
    }
    assert_eq!(
        kit.count as usize,
        c.balance.spawn_kit.len(),
        "the bake dropped a kit row"
    );
}

// ---------------------------------------------------------------------------
// The dev override (`shard.toml` `dev_spawn_kit`)
//
// `config.rs`'s own unit test owns the PARSE — what the key's text means. This
// owns the half that needs content: that the ids resolve against the shipped
// tables, that every refusal `bake_spawn_kit` makes this one makes too, and
// that unset still means the content's own kit.

/// Unset is the shipping default and changes nothing. **The first assertion a
/// public shard depends on**: a shard that never writes the key must be
/// indistinguishable from one built before it existed.
///
/// Proven red by having `dev_spawn_kit` return `Ok(Some(SpawnKit::EMPTY))` for
/// a `None` spec — the caller in `bin/shard.rs` would then overwrite the
/// content's kit with nothing on every boot, and every joiner on the public
/// shard would spawn naked with no diff to point at.
#[test]
fn an_unset_dev_kit_is_the_content_s_own_kit() {
    let c = shipped();
    assert!(
        server::config::dev_spawn_kit(None, &c)
            .expect("an unset key cannot fail")
            .is_none(),
        "an unset dev_spawn_kit produced a kit — a public shard's spawn is \
         no longer what content/balance.toml says"
    );
}

/// A set key resolves against the shipped tables, and refuses what
/// `bake_spawn_kit` refuses. The dev shard that boots is a dev shard whose
/// kit the sim can represent.
///
/// Proven red one refusal at a time by deleting each guard in
/// `config::dev_spawn_kit`: the matching case below then resolves instead of
/// erroring. The unknown-id case is the one that matters most in practice — a
/// typo'd id would otherwise silently drop a stack and read as a defect in
/// whatever verb the session was opened to test.
#[test]
fn a_dev_kit_resolves_against_shipped_content_and_refuses_the_rest() {
    let c = shipped();
    let spec = |v: &[(&str, u32)]| -> Vec<(String, u32)> {
        v.iter().map(|(i, n)| ((*i).to_string(), *n)).collect()
    };

    let kit = server::config::dev_spawn_kit(
        Some(&spec(&[("item.building_plan", 1), ("item.wood", 900)])),
        &c,
    )
    .expect("a legal dev kit resolves")
    .expect("a set key produces a kit");
    let plan = c.item_index("item.building_plan").expect("shipped plan");
    let wood = c.item_index("item.wood").expect("shipped wood");
    assert_eq!(
        (kit.count, kit.stacks[0].item, kit.stacks[0].count),
        (2, plan, 1),
        "the first entry did not land on hotbar slot 1"
    );
    assert_eq!(
        (kit.stacks[1].item, kit.stacks[1].count),
        (wood, 900),
        "the second entry did not land where it was written"
    );

    for (bad, phrase) in [
        (spec(&[("item.jetpack", 1)]), "no such item"),
        (spec(&[("item.wood", 0)]), "grants 0"),
        (spec(&[("item.rock", 99)]), "past its own stack size"),
        (spec(&[("item.wood", 1), ("item.wood", 1)]), "granted twice"),
    ] {
        // `match`, not `expect_err`: `SpawnKit` is deliberately not `Debug`
        // (it is a fixed array in a `Copy` struct on the sim's own side of
        // wall 7), so the `Ok` half has nothing to print.
        let err = match server::config::dev_spawn_kit(Some(&bad), &c) {
            Ok(_) => panic!("`{bad:?}` was accepted"),
            Err(e) => e,
        };
        assert!(
            err.contains(phrase),
            "expected an error naming `{phrase}`, got: {err}"
        );
    }
}

/// **The documented example is executable.** `shard.toml.example` carries the
/// nine-entry kit that used to ship in content, commented out, as the line a
/// developer uncomments to get the build verb back. A commented example that
/// no longer parses is worse than none: it is read as tested.
///
/// Proven red by renaming any item in that line (e.g. `item.hatchet_stone` →
/// `item.hatchet`) — the resolve below then fails naming it.
#[test]
fn the_documented_dev_kit_example_still_resolves() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../shard.toml.example");
    let text = std::fs::read_to_string(&path).expect("shard.toml.example is tracked");
    let line = text
        .lines()
        .map(str::trim_start)
        .find(|l| l.starts_with("# dev_spawn_kit ="))
        .expect("shard.toml.example no longer documents dev_spawn_kit");
    // Uncomment it and hand it to the real parser, so this gate reads the
    // documentation exactly as an operator's file would be read.
    let toml = format!(
        "bind = \"127.0.0.1:1\"\nseed = 1\n{}\n",
        line.trim_start_matches("# ")
    );
    let cfg = server::config::parse_shard_toml(&toml)
        .unwrap_or_else(|e| panic!("the documented dev_spawn_kit line does not parse: {e}"));
    let c = shipped();
    let kit = server::config::dev_spawn_kit(cfg.dev_spawn_kit.as_deref(), &c)
        .unwrap_or_else(|e| panic!("the documented dev_spawn_kit line does not resolve: {e}"))
        .expect("the documented line sets the key");
    assert!(
        kit.count >= 2,
        "the documented example grants {} stacks — it exists to unblock the \
         build verb, which needs a plan, a hammer and materials",
        kit.count
    );
}
