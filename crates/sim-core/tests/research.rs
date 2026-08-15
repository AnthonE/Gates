//! Research (research v0) — the sink OBOL exists for.
//!
//! Four things have to hold and the rest of the verb is arithmetic over
//! them:
//!
//! 1. **A gated recipe is uncraftable until it is learned, and craftable
//!    after** — the whole point, and the one assertion that would make the
//!    verb pointless if it were wrong in either direction.
//! 2. **Nothing is taken unless everything is.** The price has two halves
//!    (a sample and a pile of coin) and a refusal must cost neither, or a
//!    player pays for a blueprint they did not get. That is `inventory.rs`'s
//!    validation-before-mutation rule applied to a verb that spends two
//!    things, and it is the failure `research::research`'s ordering exists
//!    to prevent.
//! 3. **Every refusal is announced with its own reason.** A verb that fails
//!    silently at a table reads as a broken key.
//! 4. **It survives a logout**, because a blueprint you paid a hoard for and
//!    lost by closing the game would make the sink a punishment.

use sim_core::build::{foundation_terrain_ok, BuildContent, BUILD_CELL_M, LOC_PLANE};
use sim_core::combat::CombatContent;
use sim_core::craft::{CraftContent, REFUSE_BLUEPRINT};
use sim_core::deploy::DeployContent;
use sim_core::gather::{GatherContent, ItemStack};
use sim_core::limits::TICK_HZ;
use sim_core::persist::PlayerSave;
use sim_core::research::{
    knows, ResearchContent, REFUSE_R_COST, REFUSE_R_ITEM, REFUSE_R_KNOWN, REFUSE_R_LOCKED,
    REFUSE_R_SLOT, REFUSE_R_TABLE,
};
use sim_core::survival::SurvivalContent;
use sim_core::world::{
    Command, World, EV_CRAFT_REFUSED, EV_KNOWN, EV_RESEARCH, EV_RESEARCH_REFUSED,
};

const SEED: u64 = 0x0FEE_0FEE;
const PLAYER: u32 = 3;
/// The id a returning connection is minted, never the save's own — a
/// restore seats the body under whatever id the door hands it.
const REJOIN: u32 = 9;
/// `DeployContent::probe_fixture` row 7 is the research table; it costs one
/// unit of item 10 to place. `ResearchContent::probe_fixture`: item 4
/// unlocks craft recipe 2 for 5 of item 3.
const TABLE_ROW: u16 = 7;
const TABLE_ITEM: u16 = 10;
const SAMPLE: u16 = 4;
const COIN: u16 = 3;
const COST: u16 = 5;
const GATED_RECIPE: u16 = 2;
/// Row 1 of the craft fixture is ungated, so it is the control: whatever
/// research does to the gated row, it must not do to this one.
const OPEN_RECIPE: u16 = 1;

fn cell_center(cx: u16, cz: u16) -> (f32, f32) {
    (
        (cx as f32 + 0.5) * BUILD_CELL_M,
        (cz as f32 + 0.5) * BUILD_CELL_M,
    )
}

fn buildable_cell(seed: u64) -> (u16, u16) {
    for r in 0..64i32 {
        for dz in -r..=r {
            for dx in -r..=r {
                if dx.abs() != r && dz.abs() != r {
                    continue;
                }
                let cx = (512 + dx).clamp(0, 1023) as u16;
                let cz = (512 + dz).clamp(0, 1023) as u16;
                let (x, z) = cell_center(cx, cz);
                if foundation_terrain_ok(seed, x, z) {
                    return (cx, cz);
                }
            }
        }
    }
    panic!("no buildable cell within 64 cells — the generator changed under this test");
}

/// A world with one player standing at a placed research table, holding a
/// sample in slot 0 and plenty of coin in slot 1.
fn table_world() -> (World, u16, u16) {
    let mut w = World::new(SEED);
    w.gather = GatherContent::probe_fixture();
    w.build = BuildContent::probe_fixture();
    w.deploy = DeployContent::probe_fixture();
    w.craft = CraftContent::probe_fixture();
    w.research = ResearchContent::probe_fixture();

    let (cx, cz) = buildable_cell(SEED);
    let (x, z) = cell_center(cx, cz);
    w.dev_spawn = Some((x, z));
    w.tick(&[Command::Join { id: PLAYER }]);
    w.players[0].body = sim_core::movement::Body::at(SEED, x, z);
    w.players[0].inv[0] = ItemStack {
        item: TABLE_ITEM,
        count: 1,
    };
    w.tick(&[Command::PlaceDeploy {
        id: PLAYER,
        row: TABLE_ROW,
        cx,
        cz,
        level: 0,
        loc: LOC_PLANE,
    }]);
    assert_eq!(w.deploys.len(), 1, "the fixture needs its table placed");
    assert_eq!(
        w.deploys.boxes().len(),
        0,
        "a research table is a station, not a container: it stands up no box"
    );
    stock(&mut w);
    (w, cx, cz)
}

/// Put a sample in slot 0 and 20 coin in slot 1.
fn stock(w: &mut World) {
    w.players[0].inv[0] = ItemStack {
        item: SAMPLE,
        count: 2,
    };
    w.players[0].inv[1] = ItemStack {
        item: COIN,
        count: 20,
    };
}

fn have(w: &World, item: u16) -> u32 {
    sim_core::craft::inv_count(&w.players[0].inv, item)
}

fn refusal(w: &World, code: u8) -> Option<u32> {
    w.events
        .entries()
        .iter()
        .find(|e| e.code == code)
        .map(|e| e.b)
}

/// Kill the body with the survival clock — a real cause through
/// `World::die`, which is the door a hand-set `dead` flag skips. Needs the
/// survival fixture armed, or nothing ever decays.
fn starve(w: &mut World) {
    let before = w.players[0].deaths;
    assert!(
        w.players[0].hp > 0,
        "the body is already at zero, so nothing can kill it — arm the \
         fixtures with `arm_for_dying` before starving"
    );
    w.players[0].food = 0;
    w.players[0].water = 0;
    for _ in 0..120 * TICK_HZ {
        w.tick(&[]);
        if w.players[0].deaths > before {
            return;
        }
    }
    panic!("the clock never killed the body — the survival fixture changed under this test");
}

/// `table_world` seats its body before the combat and survival fixtures
/// are on it, so the player has no health and no meters — a fine state for
/// the eleven tests that never leave the table, and a body nothing can
/// kill. Arm both and re-grant, for the tests that need a death.
fn arm_for_dying(w: &mut World) {
    w.combat = CombatContent::probe_fixture();
    w.survival = SurvivalContent::probe_fixture();
    w.players[0].hp = w.combat.player_hp;
    w.players[0].hp_max = w.combat.player_hp;
    sim_core::survival::grant(&w.survival, &mut w.players[0]);
}

fn ask(w: &mut World, slot: u8) {
    w.tick(&[Command::Research { id: PLAYER, slot }]);
}

/// (1) The whole point, both directions on one recipe.
#[test]
fn a_gated_recipe_is_uncraftable_until_it_is_learned() {
    let (mut w, _, _) = table_world();

    w.tick(&[Command::Craft {
        id: PLAYER,
        recipe: GATED_RECIPE,
        count: 1,
    }]);
    assert_eq!(
        refusal(&w, EV_CRAFT_REFUSED),
        Some(REFUSE_BLUEPRINT),
        "before: refused, and for the blueprint rather than for the station"
    );

    ask(&mut w, 0);
    assert!(
        knows(w.players[0].known, GATED_RECIPE),
        "the bit is set on the player who paid"
    );
    let ev = w
        .events
        .entries()
        .iter()
        .find(|e| e.code == EV_RESEARCH)
        .copied()
        .expect("the success is announced");
    assert_eq!(
        (ev.a, ev.b, ev.c),
        (PLAYER, GATED_RECIPE as u32, COST as u32),
        "and it names the learner, the recipe, and what it actually cost"
    );

    // After: the blueprint refusal is gone. The craft still needs its
    // station — that refusal is a different code and stands, which is the
    // half a test that only checked "no longer refused" would miss.
    w.tick(&[Command::Craft {
        id: PLAYER,
        recipe: GATED_RECIPE,
        count: 1,
    }]);
    assert_ne!(
        refusal(&w, EV_CRAFT_REFUSED),
        Some(REFUSE_BLUEPRINT),
        "after: the blueprint is no longer what stands in the way"
    );
}

/// The mask is per player, not per shard. Two people at one table learn
/// separately, which is what makes a blueprint worth anything.
#[test]
fn a_blueprint_is_learned_by_a_player_and_not_by_a_shard() {
    let (mut w, _, _) = table_world();
    ask(&mut w, 0);
    assert!(knows(w.players[0].known, GATED_RECIPE));
    for other in w.players.iter().skip(1) {
        assert_eq!(
            other.known, 0,
            "nobody else learned anything by standing nearby"
        );
    }
}

/// (2) The two-part price is all-or-nothing, tested from the expensive
/// side: enough sample, not enough coin. A verb that took the sample first
/// would eat a revolver and teach nothing.
#[test]
fn a_refusal_for_the_price_costs_neither_half() {
    let (mut w, _, _) = table_world();
    w.players[0].inv[1] = ItemStack {
        item: COIN,
        count: COST - 1,
    };
    ask(&mut w, 0);

    assert_eq!(refusal(&w, EV_RESEARCH_REFUSED), Some(REFUSE_R_COST));
    assert_eq!(have(&w, SAMPLE), 2, "the sample is untouched");
    assert_eq!(
        have(&w, COIN),
        (COST - 1) as u32,
        "and so is the coin it could not pay with"
    );
    assert!(!knows(w.players[0].known, GATED_RECIPE), "nothing learned");
}

/// And from the other side: the coin is spent across stacks, not out of
/// one. Three fives are fifteen.
#[test]
fn the_price_is_paid_across_stacks_and_the_sample_is_one_unit() {
    let (mut w, _, _) = table_world();
    w.players[0].inv[1] = ItemStack {
        item: COIN,
        count: 2,
    };
    w.players[0].inv[2] = ItemStack {
        item: COIN,
        count: 2,
    };
    w.players[0].inv[3] = ItemStack {
        item: COIN,
        count: 2,
    };
    ask(&mut w, 0);

    assert!(knows(w.players[0].known, GATED_RECIPE), "6 coin covers 5");
    assert_eq!(have(&w, COIN), 1, "and exactly the price came out");
    assert_eq!(have(&w, SAMPLE), 1, "one sample, not the stack");
}

/// (3) Every refusal, each with its own reason and each costing nothing.
#[test]
fn every_refusal_names_itself_and_changes_nothing() {
    // No table in reach — the one refusal that needs a different world.
    {
        let mut w = World::new(SEED);
        w.gather = GatherContent::probe_fixture();
        w.deploy = DeployContent::probe_fixture();
        w.research = ResearchContent::probe_fixture();
        let (cx, cz) = buildable_cell(SEED);
        let (x, z) = cell_center(cx, cz);
        w.dev_spawn = Some((x, z));
        w.tick(&[Command::Join { id: PLAYER }]);
        w.players[0].body = sim_core::movement::Body::at(SEED, x, z);
        stock(&mut w);
        ask(&mut w, 0);
        assert_eq!(refusal(&w, EV_RESEARCH_REFUSED), Some(REFUSE_R_TABLE));
        assert_eq!(have(&w, SAMPLE), 2);
        assert_eq!(have(&w, COIN), 20);
    }

    let (mut w, _, _) = table_world();

    // An empty slot, and a slot past the inventory: one reason, because to
    // a player they are the same mistake.
    ask(&mut w, 9);
    assert_eq!(refusal(&w, EV_RESEARCH_REFUSED), Some(REFUSE_R_SLOT));
    ask(&mut w, 250);
    assert_eq!(
        refusal(&w, EV_RESEARCH_REFUSED),
        Some(REFUSE_R_SLOT),
        "a forged index is a refusal, never a panic and never a disconnect"
    );

    // Something the table has no row for.
    ask(&mut w, 1); // the coin itself
    assert_eq!(refusal(&w, EV_RESEARCH_REFUSED), Some(REFUSE_R_ITEM));
    assert_eq!(have(&w, COIN), 20, "and it was not spent finding out");

    // Already known.
    ask(&mut w, 0);
    assert!(knows(w.players[0].known, GATED_RECIPE));
    let coin_after = have(&w, COIN);
    ask(&mut w, 0);
    assert_eq!(refusal(&w, EV_RESEARCH_REFUSED), Some(REFUSE_R_KNOWN));
    assert_eq!(
        have(&w, COIN),
        coin_after,
        "a second attempt is free, which is the point of refusing it"
    );
    assert_eq!(have(&w, SAMPLE), 1, "and the sample survives too");
}

/// An ungated recipe is untouched by all of this — the control that makes
/// every assertion above mean what it says.
#[test]
fn an_ungated_recipe_never_asks_about_a_blueprint() {
    let (mut w, _, _) = table_world();
    assert!(!knows(w.players[0].known, OPEN_RECIPE));
    w.tick(&[Command::Craft {
        id: PLAYER,
        recipe: OPEN_RECIPE,
        count: 1,
    }]);
    assert_ne!(
        refusal(&w, EV_CRAFT_REFUSED),
        Some(REFUSE_BLUEPRINT),
        "an open recipe is craftable by someone who has learned nothing"
    );
}

/// The content fixtures, without a table and without a player — the far
/// side of a reconnect, where a save arrives at a world that has never
/// seen its owner. `table_world` is this plus a body and the table it
/// placed.
///
/// Boxed for `persist.rs`'s reason: a `World` is several hundred KB and
/// these tests hold two live at once — the world that researched and the
/// world its owner reconnects to.
fn content_world() -> Box<World> {
    let mut w = Box::new(World::new(SEED));
    w.gather = GatherContent::probe_fixture();
    w.build = BuildContent::probe_fixture();
    w.deploy = DeployContent::probe_fixture();
    w.craft = CraftContent::probe_fixture();
    w.research = ResearchContent::probe_fixture();
    w
}

/// Whether the gated recipe is still refused *for the blueprint*, asked of
/// whichever body id is driving — the consequence half of "the mask
/// survived", and the only one a player can feel. A test that checks the
/// bit and stops is checking a `u64`; this checks that the recipe opened.
fn blueprint_blocks(w: &mut World, id: u32) -> bool {
    w.tick(&[Command::Craft {
        id,
        recipe: GATED_RECIPE,
        count: 1,
    }]);
    refusal(w, EV_CRAFT_REFUSED) == Some(REFUSE_BLUEPRINT)
}

/// The mask, as the door states it: `(low, high)` reassembled from the
/// one `EV_KNOWN` on the last tick, or `None` if the door said nothing.
fn announced(w: &World) -> Option<u64> {
    w.events
        .entries()
        .iter()
        .find(|e| e.code == EV_KNOWN)
        .map(|e| e.b as u64 | (e.c as u64) << 32)
}

/// (4a) **It survives a death**, which is the door that was losing it.
///
/// `wake` rebuilt the record from `Player::default()` and named the five
/// fields a respawn keeps; `known` was not among them, so every death
/// refunded nothing and deleted every blueprint the player had bought.
/// Death is the most common event in the game, so the OBOL sink emptied
/// itself faster than it filled.
///
/// **Every step here is a real cause**, and the first cut of this test is
/// why that is written down. It set `hp = 0` and `dead = true` by hand and
/// passed against a tree where the mask was still being erased — because
/// the mask is cleared by `World::die`, one door earlier, and a hand-set
/// flag never comes through it. The body starves for real now: two doors
/// were losing the mask and a synthetic precondition could only ever see
/// the second.
#[test]
fn a_blueprint_survives_a_death() {
    let (mut w, _, _) = table_world();
    // `table_world` leaves combat empty, so `player_hp` is 0 and a respawn
    // would stand up a body with no health — which would make the "a
    // respawn is a whole body" check below vacuous rather than failing it.
    // Armed here rather than in the fixture: every sibling test in this
    // file is about a table, not about dying.
    arm_for_dying(&mut w);
    ask(&mut w, 0);
    let mask = w.players[0].known;
    assert!(knows(mask, GATED_RECIPE), "there is something to lose");
    assert!(
        !blueprint_blocks(&mut w, PLAYER),
        "the recipe is open before the death, or this test proves nothing"
    );

    starve(&mut w);
    assert!(w.players[0].dead, "the clock did not put the body down");
    w.tick(&[Command::Respawn {
        id: PLAYER,
        on_bag: false,
    }]);
    assert!(!w.players[0].dead, "the respawn did not stand the body up");
    assert!(w.players[0].hp > 0, "a respawn is a whole body");

    assert_eq!(
        w.players[0].known, mask,
        "the respawn erased a blueprint bought with OBOL"
    );
    assert!(
        !blueprint_blocks(&mut w, PLAYER),
        "the bit survived but the recipe closed — the mask is not what \
         `craft` reads, and one of the two is lying"
    );
}

/// (4b) **It survives a restore.** The other lost door, and the one with a
/// note on disk: `store.rs` records `PlayerSave` growing the mask at
/// research v0, the codec round-trips it, and `seat` then dropped it on
/// the floor through `..Player::default()`.
///
/// Fires whenever a keyed player reconnects with no sleeper body waiting —
/// a shard restart without a world file, an eviction, a full sleeper
/// index. Nothing saw it: `test_replay`'s stream has no `JoinAs`, and
/// `a_blueprint_survives_a_save_and_a_load` below exercises the codec
/// without ever seating a world.
#[test]
fn a_blueprint_survives_a_restore() {
    let (mut w, _, _) = table_world();
    ask(&mut w, 0);
    let mask = w.players[0].known;
    assert!(knows(mask, GATED_RECIPE), "there is something to save");
    let save = w.save_of(PLAYER).expect("the player is in the world");
    assert_eq!(save.known, mask, "the save carries it — it always did");

    // A world that has never met this player, which is what a restart is.
    let mut w2 = content_world();
    w2.tick(&[Command::JoinAs { id: REJOIN, save }]);
    assert!(w2.players[0].active, "a restore must seat a body");
    assert_eq!(
        w2.players[0].known, mask,
        "the restore dropped a blueprint the save was carrying"
    );
    assert!(
        !blueprint_blocks(&mut w2, REJOIN),
        "the recipe is closed to a body that paid for it and came back"
    );
}

/// (4c) **Every door states the mask**, because a fact stated at two doors
/// out of three is a fact that depends on how you arrived.
///
/// `SUB_KNOWN` existed and only a purchase ever sent it, so a player who
/// researched on Monday and logged in on Tuesday was told nothing and
/// watched six recipes they owned stay grey until they bought a seventh.
/// The doors are `seat` (fresh and restored) and `wake` (a respawn); the
/// purchase is checked too, because it is the same event now and the
/// server has one thing to encode instead of two.
///
/// One test per door rather than one test for all of them: each frame then
/// holds at most two worlds, which is the stack ceiling `persist.rs`
/// documents, and a failure names the door.
///
/// The mask under test has **both halves populated** wherever the door can
/// carry one. A door that sent only the low 32 bits, or packed the two
/// backwards, passes any check made with a small number — `EV_KNOWN` is
/// `u64` state through two `u32` fields, which is exactly the packed-field
/// blindness `event_roles.rs` §2 refuses to allow.
const WIDE: u64 = 1 << 3 | 1 << 40;

#[test]
fn the_fresh_door_states_an_empty_mask_out_loud() {
    // Zero is a fact. A client-core reused across two characters would
    // otherwise keep the first one's recipes on offer, and the craft gate
    // would refuse them at the sim — a menu that lies rather than a menu
    // that is empty.
    let mut fresh = content_world();
    fresh.tick(&[Command::Join { id: PLAYER }]);
    assert_eq!(
        announced(&fresh),
        Some(0),
        "the fresh door said nothing at all"
    );
}

#[test]
fn the_restored_door_states_the_whole_mask() {
    let (mut w, _, _) = table_world();
    w.players[0].known = WIDE;
    let save = w.save_of(PLAYER).expect("the player is in the world");
    let mut restored = content_world();
    restored.tick(&[Command::JoinAs { id: REJOIN, save }]);
    assert_eq!(
        announced(&restored),
        Some(WIDE),
        "the restored door either said nothing or lost half the mask"
    );
}

#[test]
fn the_respawn_door_states_the_whole_mask() {
    let (mut dead, _, _) = table_world();
    arm_for_dying(&mut dead);
    dead.players[0].known = WIDE;
    starve(&mut dead);
    dead.tick(&[Command::Respawn {
        id: PLAYER,
        on_bag: false,
    }]);
    assert_eq!(
        announced(&dead),
        Some(WIDE),
        "the respawn door did not restate the mask it carried"
    );
}

#[test]
fn a_purchase_states_the_whole_mask_it_produced() {
    let (mut buy, _, _) = table_world();
    ask(&mut buy, 0);
    assert_eq!(
        announced(&buy),
        Some(buy.players[0].known),
        "a purchase must state the whole mask it produced"
    );
}

/// (4) It survives a logout. The mask rides `PlayerSave`, so this is the
/// codec's round trip on the one field that a player paid for.
#[test]
fn a_blueprint_survives_a_save_and_a_load() {
    let (mut w, _, _) = table_world();
    ask(&mut w, 0);
    let mask = w.players[0].known;
    assert!(mask != 0, "there is something to save");

    let save = PlayerSave::of(&w.players[0]);
    let mut bytes = [0u8; sim_core::persist::PLAYER_SAVE_BYTES];
    save.write_le(&mut bytes);
    let back = PlayerSave::read_le(&bytes).expect("a record we just wrote");
    assert_eq!(back.known, mask, "the mask round-trips whole");
    assert!(
        knows(back.known, GATED_RECIPE),
        "and it is still the bit that was paid for"
    );
}

/// (7) **The ladder.** A row whose prerequisite is unheld refuses, refuses
/// for the *right* reason, and costs nothing; learning the prerequisite
/// opens it. Landed 2026-08-15 with `ResearchRow::requires`.
///
/// The order of the two checks is the assertion that matters. A locked row
/// is refused BEFORE the price, so a player who cannot reach a blueprint is
/// never told they are poor — and, more concretely, is never billed for
/// finding out. That is why the fixture below is deliberately *rich*: the
/// coin is there, and the refusal still has to be `LOCKED`.
#[test]
fn a_row_behind_a_prerequisite_refuses_until_the_prerequisite_is_held() {
    let (mut w, _, _) = table_world();
    // Put the fixture's one row behind a recipe nobody knows yet. `OPEN_RECIPE`
    // is the craft fixture's ungated row, so this is a prerequisite that is
    // real, reachable and simply not held.
    w.research.rows[0].requires = 1u64 << OPEN_RECIPE;

    let coin_before = have(&w, COIN);
    let sample_before = have(&w, SAMPLE);
    ask(&mut w, 0);
    assert_eq!(
        refusal(&w, EV_RESEARCH_REFUSED),
        Some(REFUSE_R_LOCKED),
        "an unmet prerequisite is its own refusal, not `COST` and not `ITEM`"
    );
    assert!(
        !knows(w.players[0].known, GATED_RECIPE),
        "and it taught nothing"
    );
    assert_eq!(
        have(&w, COIN),
        coin_before,
        "a locked row is refused before the price, so it bills nothing"
    );
    assert_eq!(have(&w, SAMPLE), sample_before, "and consumes no sample");

    // Hold the prerequisite; the same request now goes through.
    w.players[0].known |= 1u64 << OPEN_RECIPE;
    ask(&mut w, 0);
    assert!(
        knows(w.players[0].known, GATED_RECIPE),
        "with the prerequisite held the row is buyable"
    );
    assert_eq!(
        have(&w, COIN),
        coin_before - COST as u32,
        "and now, and only now, it charges"
    );
}

/// The mask is an AND over ALL prerequisites, not any-of: two edges means
/// both. Cheap to state and the exact thing a `!=` vs `&` slip would break.
#[test]
fn every_prerequisite_is_required_not_merely_one_of_them() {
    let (mut w, _, _) = table_world();
    w.research.rows[0].requires = (1u64 << OPEN_RECIPE) | (1u64 << 0);

    w.players[0].known |= 1u64 << OPEN_RECIPE;
    ask(&mut w, 0);
    assert_eq!(
        refusal(&w, EV_RESEARCH_REFUSED),
        Some(REFUSE_R_LOCKED),
        "one of two prerequisites is not enough"
    );

    w.players[0].known |= 1u64 << 0;
    ask(&mut w, 0);
    assert!(
        knows(w.players[0].known, GATED_RECIPE),
        "both held, and it opens"
    );
}
