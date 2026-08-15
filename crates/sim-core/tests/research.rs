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
use sim_core::craft::{CraftContent, REFUSE_BLUEPRINT};
use sim_core::deploy::DeployContent;
use sim_core::gather::{GatherContent, ItemStack};
use sim_core::persist::PlayerSave;
use sim_core::research::{
    knows, ResearchContent, REFUSE_R_COST, REFUSE_R_ITEM, REFUSE_R_KNOWN, REFUSE_R_LOCKED,
    REFUSE_R_SLOT, REFUSE_R_TABLE,
};
use sim_core::world::{Command, World, EV_CRAFT_REFUSED, EV_RESEARCH, EV_RESEARCH_REFUSED};

const SEED: u64 = 0x0FEE_0FEE;
const PLAYER: u32 = 3;
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
