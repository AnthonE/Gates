//! Research (research v0; the timed table is research table v1) — the sink
//! JUNK exists for.
//!
//! Five things have to hold and the rest of the verb is arithmetic over
//! them:
//!
//! 1. **A gated recipe is uncraftable until it is learned, and craftable
//!    after** — the whole point, and the one assertion that would make the
//!    verb pointless if it were wrong in either direction.
//! 2. **Nothing is taken unless everything is, and only at the landing.**
//!    The price has two halves (a sample and a pile of coin), both are
//!    checked when the table starts and taken when the research lands — so
//!    a refusal costs neither, and a table taken down mid-research spills
//!    both whole. That is `inventory.rs`'s validation-before-mutation rule
//!    applied across a ten-second wait.
//! 3. **Every refusal is announced with its own reason.** A verb that fails
//!    silently at a table reads as a broken key.
//! 4. **The paper is an item**: it outlives the researcher's interest in
//!    it, anyone can read it, and reading one you already know keeps it.
//! 5. **It survives a logout, a death and a restart** — the mask because a
//!    blueprint you paid a hoard for and lost by closing the game would make
//!    the sink a punishment, and a research half done because a table is a
//!    container and containers are saved.

use sim_core::backpack::BackpackContent;
use sim_core::build::{foundation_terrain_ok, BuildContent, BUILD_CELL_M, LOC_PLANE};
use sim_core::combat::CombatContent;
use sim_core::craft::{CraftContent, REFUSE_BLUEPRINT, STATION_WORKBENCH2};
use sim_core::deploy::{box_key, DeployContent, DeployDef, ARCH_WORKBENCH2, PLACE_ANY};
use sim_core::gather::{GatherContent, ItemStack};
use sim_core::inventory::{CONT_BOX, CONT_SELF, REFUSE_M_BUSY, REFUSE_M_TABLE};
use sim_core::limits::TICK_HZ;
use sim_core::oven::OVEN_PERIOD_TICKS;
use sim_core::persist::PlayerSave;
use sim_core::research::{
    blueprint_of, blueprint_target, knows, ResearchContent, REFUSE_R_BENCH, REFUSE_R_BUSY,
    REFUSE_R_COST, REFUSE_R_ITEM, REFUSE_R_KNOWN, REFUSE_R_PARENT, REFUSE_R_SLOT, REFUSE_R_TABLE,
};
use sim_core::survival::SurvivalContent;
use sim_core::worldsave;

/// The solved authored sites for `seed` — what `terrain::ground` needs in order
/// to know where the carve is.
///
/// Memoized per seed, and that is not premature: `terrain::haven` is a few
/// thousand `height` taps (a shoreline march, a bisect and a rosette per
/// candidate bearing), these suites call it from inside assertion loops, and
/// the first draft of this helper resolved it per call and took the workspace
/// test run past five minutes. It is a pure function of the seed, so caching
/// cannot change a result.
fn hv(seed: u64) -> &'static sim_core::terrain::Haven {
    use std::cell::RefCell;
    // A thread-local rather than a `Mutex`: `std::sync::Mutex` is on
    // `sim-core/clippy.toml`'s disallowed list (wall 3), and that list is
    // crate-scoped, so it binds this suite too. Per-thread is the right shape
    // anyway — the cache exists to stop a per-assertion recompute, not to be
    // shared.
    thread_local! {
        static CACHE: RefCell<Vec<(u64, &'static sim_core::terrain::Haven)>> =
            const { RefCell::new(Vec::new()) };
    }
    let hit = CACHE.with(|c| c.borrow().iter().find(|(s, _)| *s == seed).map(|&(_, h)| h));
    if let Some(h) = hit {
        return h;
    }
    let h: &'static sim_core::terrain::Haven = Box::leak(Box::new(sim_core::terrain::haven(seed)));
    CACHE.with(|c| c.borrow_mut().push((seed, h)));
    h
}

use sim_core::world::{
    Command, World, EV_CRAFT_REFUSED, EV_KNOWN, EV_MOVE_REFUSED, EV_OVEN, EV_RESEARCH,
    EV_RESEARCH_REFUSED,
};

const SEED: u64 = 0x0FEE_0FEE;
const PLAYER: u32 = 3;
/// A second hand at the same table — the one who ends up with the paper.
const FRIEND: u32 = 4;
/// The id a returning connection is minted, never the save's own — a
/// restore seats the body under whatever id the door hands it.
const REJOIN: u32 = 9;
/// `DeployContent::probe_fixture` row 7 is the research table; it costs one
/// unit of item 10 to place. `ResearchContent::probe_fixture`: item 4
/// researches into paper for craft recipe 2, for 5 of item 3, in 30 ticks.
const TABLE_ROW: u16 = 7;
const TABLE_ITEM: u16 = 10;
const SAMPLE: u16 = 4;
const COIN: u16 = 3;
const COST: u16 = 5;
const GATED_RECIPE: u16 = 2;
/// The fixture's paper — the gather fixture's one stack-of-one item.
const PAPER: u16 = 11;
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
                if foundation_terrain_ok(seed, hv(seed), x, z) {
                    return (cx, cz);
                }
            }
        }
    }
    panic!("no buildable cell within 64 cells — the generator changed under this test");
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
    w.backpack = BackpackContent::probe_fixture();
    w
}

/// A placed research table and the world around it.
struct Table {
    w: Box<World>,
    cx: u16,
    cz: u16,
}

impl Table {
    /// The container handle the table answers to — its packed address.
    fn handle(&self) -> u32 {
        box_key(self.cx, self.cz, 0)
    }

    fn slot(&self, s: usize) -> ItemStack {
        let i = self
            .w
            .deploys
            .table_index(self.handle())
            .expect("the table stands");
        self.w.deploys.box_slot(i, s)
    }

    fn running(&self) -> bool {
        let i = self
            .w
            .deploys
            .table_index(self.handle())
            .expect("the table stands");
        self.w.deploys.oven_states()[i].lit
    }
}

/// A world with one player standing at a placed research table, holding a
/// sample in slot 0 and plenty of coin in slot 1.
fn table_world() -> Table {
    let mut w = content_world();
    let (cx, cz) = buildable_cell(SEED);
    let (x, z) = cell_center(cx, cz);
    w.dev_spawn = Some((x, z));
    w.tick(&[Command::Join { id: PLAYER }]);
    w.players[0].body = sim_core::movement::Body::at(SEED, hv(SEED), x, z);
    w.players[0].inv[0] = ItemStack {
        item: TABLE_ITEM,
        count: 1,
        cond: 0,
        skin: 0,
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
        1,
        "a research table is a container since research table v1: it stands up a box"
    );
    stock(&mut w);
    Table { w, cx, cz }
}

/// Put two samples in slot 0 and 20 coin in slot 1.
fn stock(w: &mut World) {
    w.players[0].inv[0] = ItemStack {
        item: SAMPLE,
        count: 2,
        cond: 0,
        skin: 0,
    };
    w.players[0].inv[1] = ItemStack {
        item: COIN,
        count: 20,
        cond: 0,
        skin: 0,
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

/// One move by `id`, the verb a drag sends.
fn shift(t: &mut Table, id: u32, from: (u8, u8), to: (u8, u8), count: u16) {
    let cont = t.handle();
    t.w.tick(&[Command::Move {
        id,
        cont,
        from_kind: from.0,
        from_slot: from.1,
        to_kind: to.0,
        to_slot: to.1,
        count,
    }]);
}

/// Load the table the way a player does: one sample out of inventory slot
/// `from` into the item slot, and all 20 coin into the coin slot.
fn load(t: &mut Table, from: u8) {
    shift(t, PLAYER, (CONT_SELF, from), (CONT_BOX, 0), 1);
    assert_eq!(
        refusal(&t.w, EV_MOVE_REFUSED),
        None,
        "the sample should go in"
    );
    shift(t, PLAYER, (CONT_SELF, 1), (CONT_BOX, 1), 20);
    assert_eq!(
        refusal(&t.w, EV_MOVE_REFUSED),
        None,
        "the coin should go in"
    );
}

/// The switch — the same use press that lights a fire.
fn press(t: &mut Table, id: u32) {
    let (cx, cz) = (t.cx, t.cz);
    t.w.tick(&[Command::Use {
        id,
        cx,
        cz,
        level: 0,
        loc: LOC_PLANE,
    }]);
}

/// Ticks until any research started now has certainly landed: the wait,
/// plus the one period a table may sit before its first visit.
fn wait_out(t: &mut Table) {
    let ticks = t.w.research.table_ticks as u64 + OVEN_PERIOD_TICKS;
    for _ in 0..ticks {
        t.w.tick(&[]);
    }
}

/// Load, press, wait: the whole research. Returns the table's item slot.
fn research(t: &mut Table) -> ItemStack {
    load(t, 0);
    press(t, PLAYER);
    assert!(t.running(), "the press should have started the table");
    wait_out(t);
    t.slot(0)
}

/// Take the paper out of the table into the first empty inventory slot
/// of the player at index `who`, as `id`. Returns the slot it landed in.
fn take_paper(t: &mut Table, who: usize, id: u32) -> u8 {
    let to = t.w.players[who]
        .inv
        .iter()
        .position(|s| s.count == 0)
        .expect("room in the pack") as u8;
    shift(t, id, (CONT_BOX, 0), (CONT_SELF, to), 1);
    to
}

/// Read the paper in inventory `slot`.
fn study(w: &mut World, id: u32, slot: u8) {
    w.tick(&[Command::Research { id, slot }]);
}

/// The whole road: research the sample, take the paper, read it.
fn learn(t: &mut Table) {
    research(t);
    let slot = take_paper(t, 0, PLAYER);
    study(&mut t.w, PLAYER, slot);
    assert!(
        knows(t.w.players[0].known, GATED_RECIPE),
        "the road should have taught the recipe"
    );
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
/// the tests that never leave the table, and a body nothing can kill. Arm
/// both and re-grant, for the tests that need a death.
fn arm_for_dying(w: &mut World) {
    w.combat = CombatContent::probe_fixture();
    w.survival = SurvivalContent::probe_fixture();
    w.players[0].hp = w.combat.player_hp;
    w.players[0].hp_max = w.combat.player_hp;
    sim_core::survival::grant(&w.survival, &mut w.players[0]);
}

/// (1) The whole point, both directions on one recipe.
#[test]
fn a_gated_recipe_is_uncraftable_until_it_is_learned() {
    let mut t = table_world();

    t.w.tick(&[Command::Craft {
        id: PLAYER,
        recipe: GATED_RECIPE,
        count: 1,
        skin: 0,
    }]);
    assert_eq!(
        refusal(&t.w, EV_CRAFT_REFUSED),
        Some(REFUSE_BLUEPRINT),
        "before: refused, and for the blueprint rather than for the station"
    );

    research(&mut t);
    let slot = take_paper(&mut t, 0, PLAYER);
    study(&mut t.w, PLAYER, slot);
    assert!(
        knows(t.w.players[0].known, GATED_RECIPE),
        "the bit is set on the player who read the paper"
    );
    let ev =
        t.w.events
            .entries()
            .iter()
            .find(|e| e.code == EV_RESEARCH)
            .copied()
            .expect("the success is announced");
    assert_eq!(
        (ev.a, ev.b, ev.c),
        (PLAYER, GATED_RECIPE as u32, 0),
        "it names the learner and the recipe, and reading cost nothing — \
         the table was paid when the paper was made"
    );
    assert_eq!(
        t.w.players[0].inv[slot as usize],
        ItemStack::default(),
        "and the paper was read away"
    );

    // After: the blueprint refusal is gone. The craft still needs its
    // station — that refusal is a different code and stands, which is the
    // half a test that only checked "no longer refused" would miss.
    t.w.tick(&[Command::Craft {
        id: PLAYER,
        recipe: GATED_RECIPE,
        count: 1,
        skin: 0,
    }]);
    assert_ne!(
        refusal(&t.w, EV_CRAFT_REFUSED),
        Some(REFUSE_BLUEPRINT),
        "after: the blueprint is no longer what stands in the way"
    );
}

/// (2) The wait is real, and the price lands with the paper. Until the
/// table's wait has passed, the sample and the coin sit in it untouched;
/// the tick it lands, the sample IS the paper and exactly the price has
/// left the coin slot.
#[test]
fn a_research_takes_its_wait_and_charges_when_it_lands() {
    let mut t = table_world();
    load(&mut t, 0);
    assert_eq!(have(&t.w, SAMPLE), 1, "one sample went in, one stayed out");
    press(&mut t, PLAYER);
    let lit =
        t.w.events
            .entries()
            .iter()
            .find(|e| e.code == EV_OVEN)
            .copied()
            .expect("the start is announced on the oven's own event");
    assert_eq!(
        (lit.b & 1, lit.c),
        (1, PLAYER),
        "lit, by the hand that pressed"
    );

    // Short of the wait: nothing has happened yet but the clock. The
    // earliest a landing can come is one period short of the wait — when
    // the table's first visit falls on the press's own tick — so one tick
    // less than that is certainly still waiting.
    for _ in 0..t.w.research.table_ticks as u64 - OVEN_PERIOD_TICKS - 1 {
        t.w.tick(&[]);
    }
    assert!(t.running(), "still running inside its wait");
    assert_eq!(t.slot(0).item, SAMPLE, "the sample is not taken early");
    assert_eq!(t.slot(1).count, 20, "and neither is the coin");

    wait_out(&mut t);
    assert!(!t.running(), "it landed");
    assert_eq!(
        t.slot(0),
        blueprint_of(&t.w.research, SAMPLE),
        "the sample became the paper that teaches it"
    );
    assert_eq!(
        t.slot(1).count,
        20 - COST,
        "exactly the price came out of the coin slot"
    );
    assert!(
        !knows(t.w.players[0].known, GATED_RECIPE),
        "and nobody knows anything yet — the paper has to be read"
    );
}

/// (3) Every start refusal, each with its own reason and each costing
/// nothing.
#[test]
fn every_start_refusal_names_itself_and_changes_nothing() {
    // Nothing in the item slot.
    let mut t = table_world();
    press(&mut t, PLAYER);
    assert_eq!(refusal(&t.w, EV_RESEARCH_REFUSED), Some(REFUSE_R_SLOT));
    assert!(!t.running());

    // Short on coin: 4 in the coin slot against a price of 5.
    shift(&mut t, PLAYER, (CONT_SELF, 0), (CONT_BOX, 0), 1);
    shift(&mut t, PLAYER, (CONT_SELF, 1), (CONT_BOX, 1), COST - 1);
    press(&mut t, PLAYER);
    assert_eq!(refusal(&t.w, EV_RESEARCH_REFUSED), Some(REFUSE_R_COST));
    assert!(!t.running(), "a refused start starts nothing");
    assert_eq!(
        (t.slot(0).count, t.slot(1).count),
        (1, COST - 1),
        "and takes nothing"
    );

    // A sample the table no longer researches — a content swap under a
    // loaded table, the one road to this reason now that the move verb
    // keeps anything else out of the slot.
    shift(&mut t, PLAYER, (CONT_SELF, 1), (CONT_BOX, 1), 1);
    let rows = t.w.research;
    t.w.research.rows[0] = sim_core::research::ResearchRow::INERT;
    press(&mut t, PLAYER);
    assert_eq!(refusal(&t.w, EV_RESEARCH_REFUSED), Some(REFUSE_R_ITEM));
    t.w.research = rows;

    // Out of reach: a table the player walked away from.
    let far = t.w.players[0].body;
    let (x, z) = cell_center(t.cx + 6, t.cz);
    t.w.players[0].body = sim_core::movement::Body::at(SEED, hv(SEED), x, z);
    press(&mut t, PLAYER);
    assert_eq!(refusal(&t.w, EV_RESEARCH_REFUSED), Some(REFUSE_R_TABLE));
    t.w.players[0].body = far;

    // And the loaded table still starts once everything is right, so every
    // refusal above was about its own reason and nothing else.
    press(&mut t, PLAYER);
    assert!(t.running(), "the loaded table starts");
}

/// **Knowing the recipe is not a refusal at the table.** The reference's
/// table exists to make paper, and paper for what you know is the paper
/// you hand somebody who does not. Reading it yourself is refused — and
/// the paper is KEPT.
#[test]
fn known_paper_can_be_made_and_is_kept_when_read() {
    let mut t = table_world();
    learn(&mut t);

    // A second research, of a recipe already known, is allowed. The coin
    // slot still holds what the first one left (15 of 20), so only a
    // sample goes in.
    shift(&mut t, PLAYER, (CONT_SELF, 0), (CONT_BOX, 0), 1);
    assert_eq!(refusal(&t.w, EV_MOVE_REFUSED), None);
    press(&mut t, PLAYER);
    assert!(t.running(), "the table makes paper for a known recipe");
    wait_out(&mut t);
    let slot = take_paper(&mut t, 0, PLAYER);
    let paper = t.w.players[0].inv[slot as usize];
    assert_eq!(blueprint_target(&t.w.research, paper), Some(SAMPLE));

    study(&mut t.w, PLAYER, slot);
    assert_eq!(refusal(&t.w, EV_RESEARCH_REFUSED), Some(REFUSE_R_KNOWN));
    assert_eq!(
        t.w.players[0].inv[slot as usize], paper,
        "known paper is refused and kept, not destroyed"
    );
}

/// A running table is locked — in and out, and against a second start —
/// which is what makes the price checked at the start the price charged
/// at the end.
#[test]
fn a_running_table_refuses_moves_and_a_second_start() {
    let mut t = table_world();
    load(&mut t, 0);
    press(&mut t, PLAYER);
    assert!(t.running());

    shift(&mut t, PLAYER, (CONT_BOX, 1), (CONT_SELF, 5), 1);
    assert_eq!(
        refusal(&t.w, EV_MOVE_REFUSED),
        Some(REFUSE_M_BUSY),
        "nothing comes out while it runs"
    );
    shift(&mut t, PLAYER, (CONT_SELF, 0), (CONT_BOX, 0), 1);
    assert_eq!(
        refusal(&t.w, EV_MOVE_REFUSED),
        Some(REFUSE_M_BUSY),
        "and nothing goes in"
    );
    press(&mut t, PLAYER);
    assert_eq!(refusal(&t.w, EV_RESEARCH_REFUSED), Some(REFUSE_R_BUSY));
    assert_eq!(
        (t.slot(0).count, t.slot(1).count),
        (1, 20),
        "the refusals took nothing"
    );

    wait_out(&mut t);
    shift(&mut t, PLAYER, (CONT_BOX, 1), (CONT_SELF, 5), 1);
    assert_eq!(
        refusal(&t.w, EV_MOVE_REFUSED),
        None,
        "and the lock lifts when it lands"
    );
}

/// Each slot takes its own things through the move verb — including the
/// two ways past the rule a naive check would miss: a merge that makes
/// one sample two, and a swap that would put the paper back in.
#[test]
fn the_move_verb_keeps_each_table_slot_to_its_own_items() {
    let mut t = table_world();
    // The coin into the item slot, the sample into the coin slot, and
    // anything at all into the slots past them.
    shift(&mut t, PLAYER, (CONT_SELF, 1), (CONT_BOX, 0), 1);
    assert_eq!(refusal(&t.w, EV_MOVE_REFUSED), Some(REFUSE_M_TABLE));
    shift(&mut t, PLAYER, (CONT_SELF, 0), (CONT_BOX, 1), 1);
    assert_eq!(refusal(&t.w, EV_MOVE_REFUSED), Some(REFUSE_M_TABLE));
    shift(&mut t, PLAYER, (CONT_SELF, 1), (CONT_BOX, 2), 1);
    assert_eq!(refusal(&t.w, EV_MOVE_REFUSED), Some(REFUSE_M_TABLE));
    // Two samples at once, and then a second onto the first.
    shift(&mut t, PLAYER, (CONT_SELF, 0), (CONT_BOX, 0), 2);
    assert_eq!(refusal(&t.w, EV_MOVE_REFUSED), Some(REFUSE_M_TABLE));
    shift(&mut t, PLAYER, (CONT_SELF, 0), (CONT_BOX, 0), 1);
    assert_eq!(refusal(&t.w, EV_MOVE_REFUSED), None, "one goes in");
    shift(&mut t, PLAYER, (CONT_SELF, 0), (CONT_BOX, 0), 1);
    assert_eq!(
        refusal(&t.w, EV_MOVE_REFUSED),
        Some(REFUSE_M_TABLE),
        "a merge that would make it two is refused"
    );
    assert_eq!(t.slot(0).count, 1);

    // Land a research, then try to swap the paper back in: drag a second
    // sample onto the paper is fine (the paper comes out), but dragging
    // the paper onto a sample in the item slot is not.
    shift(&mut t, PLAYER, (CONT_SELF, 1), (CONT_BOX, 1), 20);
    press(&mut t, PLAYER);
    wait_out(&mut t);
    let slot = take_paper(&mut t, 0, PLAYER);
    shift(&mut t, PLAYER, (CONT_SELF, 0), (CONT_BOX, 0), 1);
    assert_eq!(t.slot(0).item, SAMPLE, "a fresh sample goes in");
    shift(&mut t, PLAYER, (CONT_SELF, slot), (CONT_BOX, 0), 1);
    assert_eq!(
        refusal(&t.w, EV_MOVE_REFUSED),
        Some(REFUSE_M_TABLE),
        "paper goes out of the table and never back in"
    );
    assert_eq!(t.slot(0).item, SAMPLE, "and the refused swap moved nothing");
}

/// (4) The paper is an item, not a grant: the researcher does not learn
/// by researching, and whoever holds the paper can read it. Here the
/// friend takes it out of the table and reads it; the researcher paid and
/// knows nothing.
#[test]
fn the_paper_is_learned_by_whoever_reads_it() {
    let mut t = table_world();
    t.w.tick(&[Command::Join { id: FRIEND }]);
    let body = t.w.players[0].body;
    t.w.players[1].body = body;
    research(&mut t);

    let slot = take_paper(&mut t, 1, FRIEND);
    assert_eq!(
        blueprint_target(&t.w.research, t.w.players[1].inv[slot as usize]),
        Some(SAMPLE),
        "the friend is holding the paper"
    );
    study(&mut t.w, FRIEND, slot);
    assert!(
        knows(t.w.players[1].known, GATED_RECIPE),
        "the reader learned it"
    );
    assert!(
        !knows(t.w.players[0].known, GATED_RECIPE),
        "and the researcher, who paid, did not — paper is traded, not bound"
    );
}

/// Reading refuses what is not a readable blueprint, and takes nothing.
#[test]
fn only_paper_with_a_target_can_be_read() {
    let mut t = table_world();
    // An empty slot, and a slot past the inventory: one reason, because to
    // a player they are the same mistake.
    study(&mut t.w, PLAYER, 9);
    assert_eq!(refusal(&t.w, EV_RESEARCH_REFUSED), Some(REFUSE_R_SLOT));
    study(&mut t.w, PLAYER, 250);
    assert_eq!(
        refusal(&t.w, EV_RESEARCH_REFUSED),
        Some(REFUSE_R_SLOT),
        "a forged index is a refusal, never a panic and never a disconnect"
    );
    // The sample itself is not paper: the instant research this verb was
    // until research table v1 is gone.
    study(&mut t.w, PLAYER, 0);
    assert_eq!(refusal(&t.w, EV_RESEARCH_REFUSED), Some(REFUSE_R_ITEM));
    assert_eq!(have(&t.w, SAMPLE), 2, "and it was not eaten finding out");
    // A blank sheet — what any other road would mint — teaches nothing.
    t.w.players[0].inv[5] = ItemStack {
        item: PAPER,
        count: 1,
        cond: 0,
        skin: 0,
    };
    study(&mut t.w, PLAYER, 5);
    assert_eq!(refusal(&t.w, EV_RESEARCH_REFUSED), Some(REFUSE_R_ITEM));
    // Paper teaching something no row researches (a forged or stale
    // target) is refused the same way and kept.
    t.w.players[0].inv[6] = blueprint_of(&t.w.research, 9);
    study(&mut t.w, PLAYER, 6);
    assert_eq!(refusal(&t.w, EV_RESEARCH_REFUSED), Some(REFUSE_R_ITEM));
    assert_eq!(t.w.players[0].inv[6].count, 1, "kept");
    assert_eq!(t.w.players[0].known, 0, "nothing was learned");
}

/// A table taken down mid-research spills what is in it **whole**: the
/// price is charged at the landing, so a landing that never comes charges
/// nothing.
#[test]
fn a_table_taken_down_mid_research_spills_and_charges_nothing() {
    let mut t = table_world();
    load(&mut t, 0);
    press(&mut t, PLAYER);
    assert!(t.running());
    let (cx, cz) = (t.cx, t.cz);
    t.w.tick(&[Command::Demolish {
        id: PLAYER,
        deploy: true,
        cx,
        cz,
        level: 0,
        loc: LOC_PLANE,
    }]);
    assert_eq!(t.w.deploys.len(), 0, "the table came up");
    let mut spilled = [0u32; 2];
    for bag in t.w.backpacks.entries() {
        for s in bag.items.iter().filter(|s| s.count > 0) {
            if s.item == SAMPLE {
                spilled[0] += s.count as u32;
            }
            if s.item == COIN {
                spilled[1] += s.count as u32;
            }
        }
    }
    assert_eq!(
        spilled,
        [1, 20],
        "the sample and every coin fell out — nothing was charged"
    );
}

/// (5) A research half done is saved, loaded, and lands after the load
/// exactly as it would have — the table is a container, and containers
/// and their clocks are world state.
#[test]
fn a_research_in_progress_survives_a_world_save() {
    let mut t = table_world();
    load(&mut t, 0);
    press(&mut t, PLAYER);
    // Inside the wait whatever the table's phase: `wait_out`'s argument,
    // from the other end.
    for _ in 0..OVEN_PERIOD_TICKS - 1 {
        t.w.tick(&[]);
    }
    assert!(t.running(), "saved while running");

    let mut bytes = vec![0; worldsave::WORLD_SAVE_MAX_BYTES];
    let n = worldsave::encode(&t.w, &mut bytes).expect("the world encodes");
    let mut back = content_world();
    worldsave::decode_into(&mut back, &bytes[..n]).expect("and decodes");
    let mut after = Table {
        w: back,
        cx: t.cx,
        cz: t.cz,
    };
    assert!(after.running(), "the clock came back running");
    assert_eq!(after.slot(0).item, SAMPLE);

    wait_out(&mut after);
    assert!(!after.running());
    assert_eq!(
        after.slot(0),
        blueprint_of(&after.w.research, SAMPLE),
        "and it lands in the loaded world as it would have in the old one"
    );
    assert_eq!(after.slot(1).count, 20 - COST);
}

/// The mask is per player, not per shard. Two people at one table learn
/// separately, which is what makes a blueprint worth anything.
#[test]
fn a_blueprint_is_learned_by_a_player_and_not_by_a_shard() {
    let mut t = table_world();
    learn(&mut t);
    for other in t.w.players.iter().skip(1) {
        assert_eq!(
            other.known, 0,
            "nobody else learned anything by standing nearby"
        );
    }
}

/// An ungated recipe is untouched by all of this — the control that makes
/// every assertion above mean what it says.
#[test]
fn an_ungated_recipe_never_asks_about_a_blueprint() {
    let mut t = table_world();
    assert!(!knows(t.w.players[0].known, OPEN_RECIPE));
    t.w.tick(&[Command::Craft {
        id: PLAYER,
        recipe: OPEN_RECIPE,
        count: 1,
        skin: 0,
    }]);
    assert_ne!(
        refusal(&t.w, EV_CRAFT_REFUSED),
        Some(REFUSE_BLUEPRINT),
        "an open recipe is craftable by someone who has learned nothing"
    );
}

// ---------------------------------------------------------------------
// The tech tree (tech tree v0): the second road to the same mask.
// `ResearchContent::probe_fixture` row 1 is the tree probe — recipe 1,
// cost 4, requiring recipe 2 — and both fixture recipes are node-tier 1,
// so the fixture workbench (deploy row 2 below is the DOOR; the bench is
// row 1) is the only station the suite needs.

/// Deploy fixture row 1 is the tier-1 workbench; placing it costs
/// holding item 3.
const BENCH_ROW: u16 = 1;
const BENCH_ITEM: u16 = 3;
const NODE_RECIPE: u16 = 1;
const NODE_COST: u16 = 4;

/// `table_world`, plus a workbench on the neighbouring cell — the tree's
/// own station, since a research table is not a bench.
fn bench_world() -> Table {
    let mut t = table_world();
    t.w.players[0].inv[2] = ItemStack {
        item: BENCH_ITEM,
        count: 1,
        cond: 0,
        skin: 0,
    };
    let (cx, cz) = (t.cx, t.cz);
    t.w.tick(&[Command::PlaceDeploy {
        id: PLAYER,
        row: BENCH_ROW,
        cx: cx + 1,
        cz,
        level: 0,
        loc: LOC_PLANE,
    }]);
    assert_eq!(t.w.deploys.len(), 2, "the suite needs its bench placed");
    // The bench's carrier item is the fixture's COIN (item 3), so any
    // unspent unit would inflate every coin count below — clear it and
    // restock to the suite's known 20.
    t.w.players[0].inv[2] = ItemStack::default();
    stock(&mut t.w);
    t
}

fn ask_unlock(w: &mut World, recipe: u16) {
    w.tick(&[Command::Unlock { id: PLAYER, recipe }]);
}

/// The tree's whole point, in order: out-of-order refuses on the parent
/// and costs nothing; the root unlocks with no sample; the child then
/// unlocks along the edge — and the coin is all either of them takes.
#[test]
fn the_tree_unlocks_along_its_edges_and_refuses_out_of_order() {
    let mut t = bench_world();
    let w = &mut t.w;

    // The child first: refused on the parent, coin intact.
    ask_unlock(w, NODE_RECIPE);
    assert_eq!(refusal(w, EV_RESEARCH_REFUSED), Some(REFUSE_R_PARENT));
    assert_eq!(have(w, COIN), 20, "an out-of-order ask costs nothing");
    assert!(!knows(w.players[0].known, NODE_RECIPE));

    // The root: no sample in hand — the tree never asks for one.
    ask_unlock(w, GATED_RECIPE);
    assert!(knows(w.players[0].known, GATED_RECIPE));
    assert_eq!(have(w, COIN), 20 - COST as u32);
    assert_eq!(
        have(w, SAMPLE),
        2,
        "the tree took the price and never the sample"
    );

    // The child, now in order.
    ask_unlock(w, NODE_RECIPE);
    assert!(knows(w.players[0].known, NODE_RECIPE));
    assert_eq!(have(w, COIN), 20 - (COST + NODE_COST) as u32);
}

/// The tree's refusal ladder, each with its own reason and nothing
/// taken: not a node, already known, no bench, short on coin.
#[test]
fn the_tree_refuses_with_reasons_and_takes_nothing() {
    let mut t = bench_world();
    let w = &mut t.w;

    // Recipe 0 exists and no research row teaches it.
    ask_unlock(w, 0);
    assert_eq!(refusal(w, EV_RESEARCH_REFUSED), Some(REFUSE_R_ITEM));

    // Already known.
    ask_unlock(w, GATED_RECIPE);
    let coin_after = have(w, COIN);
    ask_unlock(w, GATED_RECIPE);
    assert_eq!(refusal(w, EV_RESEARCH_REFUSED), Some(REFUSE_R_KNOWN));
    assert_eq!(have(w, COIN), coin_after, "a second ask is free");

    // Short on coin: the child costs 4 and the pocket holds 3.
    w.players[0].inv[1] = ItemStack {
        item: COIN,
        count: 3,
        cond: 0,
        skin: 0,
    };
    ask_unlock(w, NODE_RECIPE);
    assert_eq!(refusal(w, EV_RESEARCH_REFUSED), Some(REFUSE_R_COST));
    assert_eq!(have(w, COIN), 3, "and the 3 are still there");
    assert!(!knows(w.players[0].known, NODE_RECIPE));
}

/// No bench in reach is its own sentence — `REFUSE_R_BENCH`, not the
/// table's `REFUSE_R_TABLE` — because "walk to your workbench" and "walk
/// to a research table" send a player to different buildings. The world
/// here has ONLY the table placed, which is exactly the confusion the
/// two codes exist to keep apart.
#[test]
fn the_tree_needs_a_bench_and_says_so() {
    let mut t = table_world();
    ask_unlock(&mut t.w, GATED_RECIPE);
    assert_eq!(refusal(&t.w, EV_RESEARCH_REFUSED), Some(REFUSE_R_BENCH));
    assert_eq!(have(&t.w, COIN), 20, "and asking cost nothing");
}

/// **A tree unlock states the whole mask it produced**, as a read of paper
/// does. `client-core` takes `SUB_KNOWN` as the authority and sets no bit
/// off `EV_RESEARCH`, so until 2026-09-22 an unlock was a purchase the
/// client never heard: the node stayed locked, its child stayed blocked,
/// and the craft panel kept the recipe LOCKED until a respawn restated the
/// mask. `WIDE` is already held so the statement has to carry both halves.
#[test]
fn an_unlock_states_the_whole_mask_it_produced() {
    let mut t = bench_world();
    t.w.players[0].known = WIDE;
    ask_unlock(&mut t.w, GATED_RECIPE);
    assert!(
        knows(t.w.players[0].known, GATED_RECIPE),
        "the unlock landed"
    );
    assert_eq!(
        announced(&t.w),
        Some(WIDE | 1 << GATED_RECIPE),
        "a tree unlock must state the whole mask it produced, both halves"
    );
}

/// The bench check is a RUNG, not a presence: a node whose recipe is
/// tier 2 refuses at a tier-1 bench standing right there, with the bench
/// sentence and nothing taken. The client's tab strip hides such a node
/// (`ui::techtree::tabs`); this is the sim refusing it anyway, because a
/// hidden button is not a gate.
#[test]
fn a_higher_tier_node_refuses_at_a_lower_bench() {
    let mut t = bench_world();
    t.w.craft.recipes[GATED_RECIPE as usize].station = STATION_WORKBENCH2;
    ask_unlock(&mut t.w, GATED_RECIPE);
    assert_eq!(refusal(&t.w, EV_RESEARCH_REFUSED), Some(REFUSE_R_BENCH));
    assert_eq!(have(&t.w, COIN), 20, "a refused unlock costs nothing");
    assert!(!knows(t.w.players[0].known, GATED_RECIPE));
}

/// And the ≥, the other way: a tier-2 bench unlocks a tier-1 node (the
/// operator's 2026-09-22 call — a higher bench keeps the lower trees as
/// tabs), and a tier-2 node at its own rung. The tier-2 def is appended
/// past the shared fixture, `craft::tests`' precedent, so the parity and
/// replay worlds never see it.
#[test]
fn a_higher_bench_unlocks_a_lower_tier_node() {
    let mut t = table_world();
    let wb2_row = t.w.deploy.def_count;
    t.w.deploy.defs[wb2_row as usize] = DeployDef {
        arch: ARCH_WORKBENCH2,
        placement: PLACE_ANY,
        hp: 80,
        item: BENCH_ITEM,
        n_costs: 1,
        costs: [(0, 20), (0, 0), (0, 0), (0, 0)],
    };
    t.w.deploy.def_count += 1;
    t.w.players[0].inv[2] = ItemStack {
        item: BENCH_ITEM,
        count: 1,
        cond: 0,
        skin: 0,
    };
    let (cx, cz) = (t.cx, t.cz);
    t.w.tick(&[Command::PlaceDeploy {
        id: PLAYER,
        row: wb2_row,
        cx: cx + 1,
        cz,
        level: 0,
        loc: LOC_PLANE,
    }]);
    assert_eq!(t.w.deploys.len(), 2, "the tier-2 bench has to stand");
    t.w.players[0].inv[2] = ItemStack::default();
    stock(&mut t.w);

    ask_unlock(&mut t.w, GATED_RECIPE);
    assert!(
        knows(t.w.players[0].known, GATED_RECIPE),
        "a tier-2 bench unlocks a tier-1 node"
    );
    t.w.craft.recipes[NODE_RECIPE as usize].station = STATION_WORKBENCH2;
    ask_unlock(&mut t.w, NODE_RECIPE);
    assert!(
        knows(t.w.players[0].known, NODE_RECIPE),
        "and a tier-2 node at its own rung"
    );
}

/// The table ignores the tree: a looted sample researches into paper for a
/// node whose parent was never learned, and that paper reads. Research-
/// what-you-loot bypassing the graph is the reference's own two-system
/// split, and it is what keeps a lucky find worth the walk.
#[test]
fn the_table_researches_a_sample_with_no_questions_about_parents() {
    let mut t = table_world();
    // Row 1's sample (item 5) in hand, parent (recipe 2) unknown.
    t.w.players[0].inv[0] = ItemStack {
        item: 5,
        count: 1,
        cond: 0,
        skin: 0,
    };
    assert!(!knows(t.w.players[0].known, GATED_RECIPE));
    research(&mut t);
    let slot = take_paper(&mut t, 0, PLAYER);
    study(&mut t.w, PLAYER, slot);
    assert!(
        knows(t.w.players[0].known, NODE_RECIPE),
        "the table taught the child with the parent unlearned"
    );
}

/// It survives a logout. The mask rides `PlayerSave`, so this is the
/// codec's round trip on the one field that a player paid for.
#[test]
fn a_blueprint_survives_a_save_and_a_load() {
    let mut t = table_world();
    learn(&mut t);
    let mask = t.w.players[0].known;
    assert!(mask != 0, "there is something to save");

    let save = PlayerSave::of(&t.w.players[0]);
    let mut bytes = [0u8; sim_core::persist::PLAYER_SAVE_BYTES];
    save.write_le(&mut bytes);
    let back = PlayerSave::read_le(&bytes).expect("a record we just wrote");
    assert_eq!(back.known, mask, "the mask round-trips whole");
    assert!(
        knows(back.known, GATED_RECIPE),
        "and it is still the bit that was paid for"
    );
}

/// And the paper survives one too, target and all: `cond` is saved with
/// every inventory stack, and it is what the sheet teaches.
#[test]
fn a_sheet_of_paper_survives_a_save_and_a_load() {
    let mut t = table_world();
    research(&mut t);
    let slot = take_paper(&mut t, 0, PLAYER);
    let save = PlayerSave::of(&t.w.players[0]);
    let mut bytes = [0u8; sim_core::persist::PLAYER_SAVE_BYTES];
    save.write_le(&mut bytes);
    let back = PlayerSave::read_le(&bytes).expect("a record we just wrote");
    assert_eq!(
        blueprint_target(&t.w.research, back.inv[slot as usize]),
        Some(SAMPLE),
        "the saved sheet still teaches what it was researched from"
    );
}

// ---------------------------------------------------------------------------
// The five doors the blueprint mask crosses (`known` at death, respawn, save,
// restore and the fresh join). Rescued into the 2026-08-15 integration by
// hand: the FIX these cover merged cleanly into `world.rs` and `persist.rs`,
// but the tests sat inside a `research.rs` conflict hunk resolved the other
// way and would have gone silently. A fix whose test is gone is a fix waiting
// to regress — `known` was cleared at four of these five doors once already.
// ---------------------------------------------------------------------------

/// The mask these doors are tested with, and the reason it is not a small
/// number: **both halves are populated**. A door that sent only the low 32
/// bits, or packed the two backwards, passes any check made with a small
/// value — `EV_KNOWN` is `u64` state carried through two `u32` fields,
/// which is exactly the packed-field blindness `event_roles.rs` §2 refuses.
const WIDE: u64 = 1 << 3 | 1 << 40;

/// The mask, as the door states it: `(low, high)` reassembled from the
/// one `EV_KNOWN` on the last tick, or `None` if the door said nothing.
fn announced(w: &World) -> Option<u64> {
    w.events
        .entries()
        .iter()
        .find(|e| e.code == EV_KNOWN)
        .map(|e| e.b as u64 | (e.c as u64) << 32)
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
        skin: 0,
    }]);
    refusal(w, EV_CRAFT_REFUSED) == Some(REFUSE_BLUEPRINT)
}

#[test]
fn a_read_states_the_whole_mask_it_produced() {
    let mut t = table_world();
    research(&mut t);
    let slot = take_paper(&mut t, 0, PLAYER);
    t.w.players[0].known = WIDE;
    study(&mut t.w, PLAYER, slot);
    assert_eq!(
        announced(&t.w),
        Some(WIDE | 1 << GATED_RECIPE),
        "a read must state the whole mask it produced, both halves"
    );
}

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

/// **It survives a death**, which is the door that was losing it.
///
/// `wake` rebuilt the record from `Player::default()` and named the five
/// fields a respawn keeps; `known` was not among them, so every death
/// refunded nothing and deleted every blueprint the player had bought.
/// Death is the most common event in the game, so the JUNK sink emptied
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
    let mut t = table_world();
    // `table_world` leaves combat empty, so `player_hp` is 0 and a respawn
    // would stand up a body with no health — which would make the "a
    // respawn is a whole body" check below vacuous rather than failing it.
    // Armed here rather than in the fixture: every sibling test in this
    // file is about a table, not about dying.
    arm_for_dying(&mut t.w);
    learn(&mut t);
    let w = &mut t.w;
    let mask = w.players[0].known;
    assert!(knows(mask, GATED_RECIPE), "there is something to lose");
    assert!(
        !blueprint_blocks(w, PLAYER),
        "the recipe is open before the death, or this test proves nothing"
    );

    starve(w);
    assert!(w.players[0].dead, "the clock did not put the body down");
    w.tick(&[Command::Respawn {
        id: PLAYER,
        on_bag: false,
    }]);
    assert!(!w.players[0].dead, "the respawn did not stand the body up");
    assert!(w.players[0].hp > 0, "a respawn is a whole body");

    assert_eq!(
        w.players[0].known, mask,
        "the respawn erased a blueprint bought with JUNK"
    );
    assert!(
        !blueprint_blocks(w, PLAYER),
        "the bit survived but the recipe closed — the mask is not what \
         `craft` reads, and one of the two is lying"
    );
}

/// **It survives a restore.** The other lost door, and the one with a
/// note on disk: `store.rs` records `PlayerSave` growing the mask at
/// research v0, the codec round-trips it, and `seat` then dropped it on
/// the floor through `..Player::default()`.
///
/// Fires whenever a keyed player reconnects with no sleeper body waiting —
/// a shard restart without a world file, an eviction, a full sleeper
/// index. Nothing saw it: `test_replay`'s stream has no `JoinAs`, and
/// `a_blueprint_survives_a_save_and_a_load` above exercises the codec
/// without ever seating a world.
#[test]
fn a_blueprint_survives_a_restore() {
    let mut t = table_world();
    learn(&mut t);
    let mask = t.w.players[0].known;
    assert!(knows(mask, GATED_RECIPE), "there is something to save");
    let save = t.w.save_of(PLAYER).expect("the player is in the world");
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

#[test]
fn the_respawn_door_states_the_whole_mask() {
    let mut t = table_world();
    let dead = &mut t.w;
    arm_for_dying(dead);
    dead.players[0].known = WIDE;
    starve(dead);
    dead.tick(&[Command::Respawn {
        id: PLAYER,
        on_bag: false,
    }]);
    assert_eq!(
        announced(dead),
        Some(WIDE),
        "the respawn door did not restate the mask it carried"
    );
}

#[test]
fn the_restored_door_states_the_whole_mask() {
    let mut t = table_world();
    t.w.players[0].known = WIDE;
    let save = t.w.save_of(PLAYER).expect("the player is in the world");
    let mut restored = content_world();
    restored.tick(&[Command::JoinAs { id: REJOIN, save }]);
    assert_eq!(
        announced(&restored),
        Some(WIDE),
        "the restored door either said nothing or lost half the mask"
    );
}
