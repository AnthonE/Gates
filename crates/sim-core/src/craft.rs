//! Craft — the second survival verb (DESIGN.md §2, M1). A craft request
//! names a recipe row baked from `content/recipes.toml`; the sim consumes
//! the inputs at enqueue, runs a per-player job queue on the tick clock,
//! and pays the output into the inventory unit by unit. Pure and
//! fixed-capacity like gather: content reaches it only as the baked
//! `CraftContent` table, the inert `EMPTY` default makes craft a no-op,
//! and `probe_fixture()` is the synthetic table for the parity/replay/
//! alloc gates.
//!
//! Verb rules below are proposed defaults, DECISIONS.md §open ("craft
//! verb v0" row): queue of `CRAFT_QUEUE` jobs · count ≤ `CRAFT_COUNT_MAX`
//! per request · inputs consumed up front for the whole batch · cancel
//! refunds the remaining units' inputs · **an output an inventory can't
//! hold now falls at the crafter's feet** (2026-08-14: `step` spills into
//! the caller's tick buffer and `world.rs` stands a bag up, the same lane
//! gather's yield takes), **and so does a cancel's refund** — the four
//! give-back paths took the same lane later the same day, so nothing in
//! this module destroys items at a full pack any more ·
//! station-gated recipes need a placed station deployable
//! (workbench/furnace archetype, deploy.rs) within `STATION_RADIUS_M` of
//! the crafter at enqueue — enqueue-time only, the reference behavior:
//! walking away never cancels a queue.

use crate::deploy::{DeployContent, Deploys, ARCH_FURNACE};
use crate::gather::{inv_add_spilling, GatherContent, ItemStack};
use crate::limits::{
    CRAFT_COUNT_MAX, CRAFT_QUEUE, INV_SLOTS, MAX_ITEM_DEFS, MAX_RECIPES, MAX_RECIPE_INPUTS,
};
use crate::world::{EventQueue, Player, EV_CRAFT_DONE, EV_CRAFT_REFUSED};

/// Station codes (schema order: CONTENT.md §1
/// `none|workbench1|workbench2|workbench3|furnace`). The three bench
/// codes are **contiguous and equal to their ladder tier** — the tiered
/// check passes `def.station` to `Deploys::bench_near` as the tier
/// itself, and the const block below pins that identity so neither
/// ladder can be reordered alone. The furnace moved 2 → 4 when the
/// ladder landed (bench ladder v0, 2026-08-15); that was a station-field
/// re-code and rode the same `PROTO_VER` turn as the width it forced.
pub const STATION_NONE: u8 = 0;
pub const STATION_WORKBENCH1: u8 = 1;
pub const STATION_WORKBENCH2: u8 = 2;
pub const STATION_WORKBENCH3: u8 = 3;
pub const STATION_FURNACE: u8 = 4;
/// The highest live station, named rather than counted — the wire's
/// encode/decode guards and the domain gate bound against this, so a
/// sixth station is a one-line move here and a loud refusal everywhere
/// it was forgotten.
pub const STATION_MAX: u8 = STATION_FURNACE;

const _: () = {
    // The bench codes ARE the bench tiers (`deploy::bench_tier`): the
    // craft gate hands `def.station` straight to the tier scan, which is
    // only sound while the two ladders are one ladder.
    assert!(crate::deploy::bench_tier(crate::deploy::ARCH_WORKBENCH) == STATION_WORKBENCH1);
    assert!(crate::deploy::bench_tier(crate::deploy::ARCH_WORKBENCH2) == STATION_WORKBENCH2);
    assert!(crate::deploy::bench_tier(crate::deploy::ARCH_WORKBENCH3) == STATION_WORKBENCH3);
};

/// How close (planar, meters) a placed station must stand at enqueue —
/// the reference's workbench-proximity read. Proposed default,
/// DECISIONS.md §open ("deployables v0").
pub const STATION_RADIUS_M: f32 = 5.0;

/// Integer refusal reasons (CLAUDE.md wall 3: integer event codes only),
/// carried by EV_CRAFT_REFUSED / the craft-refused wire subtype.
pub const REFUSE_RECIPE: u32 = 0;
pub const REFUSE_COUNT: u32 = 1;
pub const REFUSE_STATION: u32 = 2;
pub const REFUSE_QUEUE_FULL: u32 = 3;
pub const REFUSE_INPUTS: u32 = 4;
/// The recipe is blueprint-gated and this player has not researched it
/// (`research.rs`). Its own reason rather than `REFUSE_RECIPE`, because
/// the two mean opposite things to a player: one says the recipe does not
/// exist, and this says *go find one and take it to a table*.
/// (No `_MAX` beside these, unlike the deployable and research refusals:
/// the craft-refused subtype writes a full byte and the wire bounds
/// nothing, which `NOW.md` §5b already carries as the decode-side gap.
/// A `REFUSE_C_MAX` here would also collide with `survival.rs`'s consume
/// refusals, whose prefix the domain gate scans crate-wide.)
pub const REFUSE_BLUEPRINT: u32 = 5;

/// One baked recipe row. `out_count == 0` ⇒ inert (the empty-table row).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecipeDef {
    /// Item index the craft pays out.
    pub output: u16,
    /// Units paid per completed craft.
    pub out_count: u16,
    /// Ticks one unit takes (content seconds × TICK_HZ; bake keeps this
    /// exact and ≥ 1, so a craft never completes in its enqueue tick).
    pub ticks: u32,
    /// `STATION_*` code. Anything but `STATION_NONE` needs a placed
    /// station deployable near the crafter at enqueue.
    pub station: u8,
    /// Locked behind research: nobody may craft this until they have
    /// learned it (`research.rs`, `Player::known`). A **per-player** gate,
    /// unlike `station`, which is a fact about where you are standing —
    /// so it is checked against the crafter and never against the world.
    pub blueprint: bool,
    /// Live rows in `inputs`.
    pub n_inputs: u8,
    /// (item index, units per craft) — consumed per unit crafted.
    pub inputs: [(u16, u16); MAX_RECIPE_INPUTS],
}

impl RecipeDef {
    pub const INERT: Self = Self {
        output: 0,
        out_count: 0,
        ticks: 1,
        station: STATION_NONE,
        blueprint: false,
        n_inputs: 0,
        inputs: [(0, 0); MAX_RECIPE_INPUTS],
    };
}

/// The whole craft ruleset the sim knows. Construction input like the
/// gather table: the boot path bakes it from `content/recipes.toml`
/// before the first tick, and the WAL pins the content hash it came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CraftContent {
    pub recipes: [RecipeDef; MAX_RECIPES],
    pub recipe_count: u16,
}

impl CraftContent {
    /// Inert: no recipe exists, every request refuses. `World::new`
    /// starts here.
    pub const EMPTY: Self = Self {
        recipes: [RecipeDef::INERT; MAX_RECIPES],
        recipe_count: 0,
    };

    /// Synthetic table for the parity/replay/alloc gates, over the gather
    /// probe fixture's 8 items (fixture, not game content). **Row 2 is
    /// both station-gated and blueprint-gated**, so both refusal paths are
    /// inside the gates — and stacked on one row rather than two on
    /// purpose: it proves the checks are ordered rather than merely
    /// present, because a player at a bench without the blueprint must
    /// hear about the blueprint. `ResearchContent::probe_fixture` unlocks
    /// exactly this recipe.
    pub fn probe_fixture() -> Self {
        let mut c = Self::EMPTY;
        c.recipe_count = 3;
        c.recipes[0] = RecipeDef {
            output: 2,
            out_count: 2,
            ticks: 2,
            station: STATION_NONE,
            blueprint: false,
            n_inputs: 1,
            inputs: [(0, 3), (0, 0), (0, 0), (0, 0)],
        };
        c.recipes[1] = RecipeDef {
            output: 3,
            out_count: 1,
            ticks: 3,
            station: STATION_NONE,
            blueprint: false,
            n_inputs: 2,
            inputs: [(1, 2), (2, 1), (0, 0), (0, 0)],
        };
        c.recipes[2] = RecipeDef {
            output: 4,
            out_count: 1,
            ticks: 1,
            station: STATION_WORKBENCH1,
            blueprint: true,
            n_inputs: 1,
            inputs: [(0, 1), (0, 0), (0, 0), (0, 0)],
        };
        c
    }
}

/// One craft-queue job. Empty ⇔ `remaining == 0`; emptied jobs zero both
/// fields so the state hash stays canonical. The queue is dense: the head
/// lives at index 0 and completion/cancel shift the tail left.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CraftJob {
    pub recipe: u16,
    pub remaining: u16,
}

/// Total units of `item` across the inventory (u32: 30 slots × u16 max
/// overflows u16).
pub fn inv_count(inv: &[ItemStack; INV_SLOTS], item: u16) -> u32 {
    let mut total = 0u32;
    for s in inv.iter() {
        if s.count > 0 && s.item == item {
            total += s.count as u32;
        }
    }
    total
}

/// Remove `amount` of `item` in slot order, zeroing emptied slots (the
/// canonical empty representation). Returns what was actually removed —
/// callers check availability first, so anything less is defensive.
pub fn inv_take(inv: &mut [ItemStack; INV_SLOTS], item: u16, amount: u32) -> u32 {
    let mut left = amount;
    for s in inv.iter_mut() {
        if left == 0 {
            break;
        }
        if s.count > 0 && s.item == item {
            let take = (s.count as u32).min(left);
            s.count -= take as u16;
            left -= take;
            if s.count == 0 {
                *s = ItemStack::default();
            }
        }
    }
    amount - left
}

fn shift_left(jobs: &mut [CraftJob; CRAFT_QUEUE], from: usize) {
    for i in from..CRAFT_QUEUE - 1 {
        jobs[i] = jobs[i + 1];
    }
    jobs[CRAFT_QUEUE - 1] = CraftJob::default();
}

/// Apply one craft request (`Command::Craft`). Refusals are events, not
/// errors — the client hears why. Inputs for the whole batch are consumed
/// here; the head job's first unit starts its timer immediately.
/// `dc`/`deploys` carry the placed stations for the proximity gate.
#[allow(clippy::too_many_arguments)]
pub fn enqueue(
    cc: &CraftContent,
    dc: &DeployContent,
    deploys: &Deploys,
    tick: u64,
    p: &mut Player,
    recipe: u16,
    count: u16,
    events: &mut EventQueue,
) {
    if recipe >= cc.recipe_count {
        events.push(EV_CRAFT_REFUSED, p.id, REFUSE_RECIPE, 0);
        return;
    }
    if count == 0 || count > CRAFT_COUNT_MAX {
        events.push(EV_CRAFT_REFUSED, p.id, REFUSE_COUNT, 0);
        return;
    }
    let def = &cc.recipes[recipe as usize];
    if def.out_count == 0 || def.output as usize >= MAX_ITEM_DEFS {
        events.push(EV_CRAFT_REFUSED, p.id, REFUSE_RECIPE, 0);
        return;
    }
    // The blueprint, **before** the station, and the order is a design
    // call rather than an accident. A recipe can be gated by both, and
    // only one of the two refusals is worth a player's attention: a
    // missing station is transient and self-evident (walk to your bench),
    // while a missing blueprint is permanent until you go and do something
    // else entirely. Reporting the station first would send a player who
    // needs a research table back to a workbench they are already
    // standing at, which is the refusal actively misleading them.
    if def.blueprint && !crate::research::knows(p.known, recipe) {
        events.push(EV_CRAFT_REFUSED, p.id, REFUSE_BLUEPRINT, 0);
        return;
    }
    if def.station != STATION_NONE {
        let px = p.body.qx as f32 * crate::movement::POS_XZ_Q;
        let pz = p.body.qz as f32 * crate::movement::POS_XZ_Q;
        // The furnace is its own station; the three bench codes are the
        // tier ladder, and a higher bench satisfies a lower recipe —
        // `bench_near`'s ≥, not an archetype equality. Before the ladder
        // this was an if/else whose `else` mapped every non-furnace code
        // to the tier-1 bench, which would have crafted a level-3 recipe
        // at a level-1 bench the day a second tier existed.
        let ok = if def.station == STATION_FURNACE {
            deploys.arch_near(dc, ARCH_FURNACE, px, pz, STATION_RADIUS_M)
        } else {
            deploys.bench_near(dc, def.station, px, pz, STATION_RADIUS_M)
        };
        if !ok {
            events.push(EV_CRAFT_REFUSED, p.id, REFUSE_STATION, 0);
            return;
        }
    }
    let Some(slot) = p.jobs.iter().position(|j| j.remaining == 0) else {
        events.push(EV_CRAFT_REFUSED, p.id, REFUSE_QUEUE_FULL, 0);
        return;
    };
    for &(item, per) in def.inputs.iter().take(def.n_inputs as usize) {
        if inv_count(&p.inv, item) < per as u32 * count as u32 {
            events.push(EV_CRAFT_REFUSED, p.id, REFUSE_INPUTS, 0);
            return;
        }
    }
    for &(item, per) in def.inputs.iter().take(def.n_inputs as usize) {
        inv_take(&mut p.inv, item, per as u32 * count as u32);
    }
    p.jobs[slot] = CraftJob {
        recipe,
        remaining: count,
    };
    if slot == 0 {
        p.craft_done_at = tick + def.ticks as u64;
    }
}

/// Start the head job's unit timer at `tick`, or clear it if the queue is
/// empty. **The one expression of "when does the head finish"** — it was
/// written out four times (cancel, two branches of `step`, and the restore),
/// and a fifth copy is how a queue ends up armed against a stale clock.
///
/// It is also what makes a saved craft queue restorable at all: the timer
/// is an absolute tick and a restarted shard's clock begins at 0, so a
/// restore re-arms rather than reloads (`persist.rs`).
///
/// A head job whose recipe is past the live table is a content hotfix
/// shrinking the set under a queue; `step` drops that job on its next
/// visit, and arming it with the inert row's span here is harmless — the
/// row exists (`MAX_RECIPES` is the array bound, and both the wire and
/// `PlayerSave::read_le` refuse anything past it).
#[inline]
pub fn rearm(cc: &CraftContent, tick: u64, p: &mut Player) {
    p.craft_done_at = if p.jobs[0].remaining > 0 {
        tick + cc.recipes[p.jobs[0].recipe as usize].ticks as u64
    } else {
        0
    };
}

/// Apply one cancel (`Command::CraftCancel`, &mut [ItemStack::default(); INV_SLOTS]): refund the remaining units'
/// inputs (the in-progress unit refunds whole) and close the gap. An
/// index naming no live job is ignored — a cancel racing a completion is
/// a race, not an attack.
///
/// `spill` is `step`'s buffer and the fall-point is the same — the
/// crafter's feet — because a cancel is the one give-back of the four with
/// no second address even arguable: there is no object in the world to
/// refund *from* (`NOW.md` §0sp2, 2026-08-14).
///
/// **The chunk loop no longer breaks**, and that is the whole fix here. It
/// used to stop at the first chunk that did not fit whole, losing both the
/// short chunk's tail and every chunk after it; a cancel of a large queue
/// at a nearly-full pack could therefore destroy most of a refund and
/// report nothing. Every chunk now lands in the pack or in the spill, so
/// the loop runs to `left == 0` and the only remaining bound is the
/// spill's own `INV_SLOTS`, which `inv_add_spilling` documents.
pub fn cancel(
    cc: &CraftContent,
    gc: &GatherContent,
    tick: u64,
    p: &mut Player,
    index: u16,
    spill: &mut [ItemStack; INV_SLOTS],
) {
    let index = index as usize;
    if index >= CRAFT_QUEUE || p.jobs[index].remaining == 0 {
        return;
    }
    let job = p.jobs[index];
    if (job.recipe as usize) < MAX_RECIPES {
        let def = &cc.recipes[job.recipe as usize];
        for &(item, per) in def.inputs.iter().take(def.n_inputs as usize) {
            let refund = per as u32 * job.remaining as u32;
            let mut left = refund;
            while left > 0 {
                let chunk = left.min(u16::MAX as u32) as u16;
                inv_add_spilling(
                    &mut p.inv,
                    spill,
                    item,
                    chunk,
                    gc.stack_max[item as usize],
                    gc.cond_max[item as usize],
                );
                left -= chunk as u32;
            }
        }
    }
    shift_left(&mut p.jobs, index);
    if index == 0 {
        rearm(cc, tick, p);
    }
}

/// One player's per-tick craft progress: at most one unit completes per
/// tick (recipe ticks are ≥ 1 by bake), paying the output and starting
/// the next unit's — or the next job's — timer.
///
/// `spill` is the caller's tick buffer, the same one `gather::swing`
/// writes: a finished craft whose output does not fit falls at the
/// crafter's feet instead of vanishing, which matters more here than at a
/// node because the ingredients are already spent.
pub fn step(
    cc: &CraftContent,
    gc: &GatherContent,
    tick: u64,
    p: &mut Player,
    events: &mut EventQueue,
    spill: &mut [ItemStack; INV_SLOTS],
) {
    if p.jobs[0].remaining == 0 || tick < p.craft_done_at {
        return;
    }
    let recipe = p.jobs[0].recipe;
    if recipe >= cc.recipe_count {
        // A table swap shrank the set under a live job (content hotfix):
        // drop the job rather than pay from a stale row.
        shift_left(&mut p.jobs, 0);
        rearm(cc, tick, p);
        return;
    }
    let def = &cc.recipes[recipe as usize];
    // A crafted tool arrives whole: re-craft IS the repair (Q3, operator
    // 2026-08-15), so the mint at the ceiling is the design and not a
    // convenience — an output minted at 0 would be dead on arrival.
    let added = inv_add_spilling(
        &mut p.inv,
        spill,
        def.output,
        def.out_count,
        gc.stack_max[def.output as usize],
        gc.cond_max[def.output as usize],
    );
    events.push(
        EV_CRAFT_DONE,
        p.id,
        ((def.output as u32) << 16) | added as u32,
        0,
    );
    p.jobs[0].remaining -= 1;
    if p.jobs[0].remaining == 0 {
        shift_left(&mut p.jobs, 0);
    }
    rearm(cc, tick, p);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gather::NO_CELL;
    use crate::input::InputFrame;
    use crate::movement::Body;

    /// `step` for the cases where the crafter's pack has room. Asserts the
    /// spill stayed empty, so a future change that starts dropping a
    /// craftable on the ground when it did not have to reddens the whole
    /// existing suite rather than passing under it.
    fn step_nospill(
        cc: &CraftContent,
        gc: &GatherContent,
        tick: u64,
        p: &mut Player,
        events: &mut EventQueue,
    ) {
        let mut spill = [ItemStack::default(); INV_SLOTS];
        step(cc, gc, tick, p, events, &mut spill);
        assert!(
            spill.iter().all(|s| s.count == 0),
            "nothing should have spilled at tick {tick}"
        );
    }

    fn player(inv0: &[(u16, u16)]) -> Player {
        let mut p = Player {
            id: 7,
            active: true,
            body: Body::default(),
            frame: InputFrame::default(),
            inv: [ItemStack::default(); INV_SLOTS],
            next_swing: 0,
            ws_cell: NO_CELL,
            ws_hits: 0,
            jobs: [CraftJob::default(); CRAFT_QUEUE],
            craft_done_at: 0,
            hp: 0,
            ..Player::default()
        };
        for (i, &(item, count)) in inv0.iter().enumerate() {
            p.inv[i] = ItemStack {
                item,
                count,
                cond: 0,
            };
        }
        p
    }

    fn fixture() -> (CraftContent, GatherContent) {
        (
            CraftContent::probe_fixture(),
            GatherContent::probe_fixture(),
        )
    }

    #[test]
    fn inv_take_spans_stacks_and_zeroes_empties() {
        let mut p = player(&[(0, 3), (1, 5), (0, 4)]);
        assert_eq!(inv_count(&p.inv, 0), 7);
        assert_eq!(inv_take(&mut p.inv, 0, 5), 5);
        assert_eq!(p.inv[0], ItemStack::default(), "emptied slot zeroes");
        assert_eq!(
            p.inv[2],
            ItemStack {
                item: 0,
                count: 2,
                cond: 0
            }
        );
        assert_eq!(inv_count(&p.inv, 0), 2);
        assert_eq!(inv_take(&mut p.inv, 0, 9), 2, "partial take reports");
    }

    #[test]
    fn enqueue_consumes_starts_and_step_pays() {
        let (cc, gc) = fixture();
        let (dc, nod) = (DeployContent::EMPTY, Deploys::new());
        let mut p = player(&[(0, 10)]);
        let mut ev = EventQueue::default();
        enqueue(&cc, &dc, &nod, 100, &mut p, 0, 2, &mut ev);
        assert!(ev.is_empty(), "no refusal");
        assert_eq!(
            p.jobs[0],
            CraftJob {
                recipe: 0,
                remaining: 2
            }
        );
        assert_eq!(inv_count(&p.inv, 0), 4, "3 × 2 consumed up front");
        assert_eq!(p.craft_done_at, 102);

        step_nospill(&cc, &gc, 101, &mut p, &mut ev);
        assert!(ev.is_empty(), "not due yet");
        step_nospill(&cc, &gc, 102, &mut p, &mut ev);
        assert_eq!(ev.len(), 1);
        assert_eq!(ev.entries()[0].code, EV_CRAFT_DONE);
        assert_eq!(ev.entries()[0].b, (2 << 16) | 2, "item 2 × 2 landed");
        assert_eq!(inv_count(&p.inv, 2), 2);
        assert_eq!(p.jobs[0].remaining, 1);
        assert_eq!(p.craft_done_at, 104, "next unit re-arms");

        step_nospill(&cc, &gc, 104, &mut p, &mut ev);
        assert_eq!(p.jobs[0], CraftJob::default(), "batch done, queue empty");
        assert_eq!(p.craft_done_at, 0);
        assert_eq!(inv_count(&p.inv, 2), 4);
    }

    #[test]
    fn refusals_name_their_reason_and_change_nothing() {
        let (cc, _gc) = fixture();
        let (dc, nod) = (DeployContent::EMPTY, Deploys::new());
        let mut p = player(&[(0, 100), (1, 100), (2, 100)]);
        let mut ev = EventQueue::default();
        let cases: [(u16, u16, u32); 4] = [
            (99, 1, REFUSE_RECIPE),
            (0, 0, REFUSE_COUNT),
            (0, CRAFT_COUNT_MAX + 1, REFUSE_COUNT),
            // Recipe 2 is gated twice, and the BLUEPRINT is what a
            // player hears — the order `enqueue` states, asserted here so
            // it cannot be swapped back without a red gate.
            (2, 1, REFUSE_BLUEPRINT),
        ];
        for (recipe, count, reason) in cases {
            enqueue(&cc, &dc, &nod, 10, &mut p, recipe, count, &mut ev);
            let e = ev.entries()[ev.len() - 1];
            assert_eq!((e.code, e.a, e.b), (EV_CRAFT_REFUSED, 7, reason));
        }
        // And with the blueprint learned, the SAME request falls through
        // to the station — so the ordering above is a priority and not a
        // check that swallowed the other one.
        p.known |= 1 << 2;
        enqueue(&cc, &dc, &nod, 10, &mut p, 2, 1, &mut ev);
        let e = ev.entries()[ev.len() - 1];
        assert_eq!((e.code, e.a, e.b), (EV_CRAFT_REFUSED, 7, REFUSE_STATION));
        // Missing inputs: recipe 1 wants 2×item1 + 1×item2 per unit.
        let mut poor = player(&[(1, 1)]);
        enqueue(&cc, &dc, &nod, 10, &mut poor, 1, 1, &mut ev);
        let e = ev.entries()[ev.len() - 1];
        assert_eq!(e.b, REFUSE_INPUTS);
        assert_eq!(inv_count(&poor.inv, 1), 1, "nothing consumed on refusal");
        assert_eq!(poor.jobs[0], CraftJob::default());
        // Queue full: fill all four, the fifth bounces.
        let mut busy = player(&[(0, 90)]);
        for _ in 0..CRAFT_QUEUE {
            enqueue(&cc, &dc, &nod, 10, &mut busy, 0, 1, &mut ev);
        }
        assert!(busy.jobs.iter().all(|j| j.remaining == 1));
        let before = ev.len();
        enqueue(&cc, &dc, &nod, 10, &mut busy, 0, 1, &mut ev);
        assert_eq!(ev.entries()[before].b, REFUSE_QUEUE_FULL);
    }

    #[test]
    fn cancel_refunds_remaining_and_rearms_the_head() {
        let (cc, gc) = fixture();
        let (dc, nod) = (DeployContent::EMPTY, Deploys::new());
        let mut p = player(&[(0, 30), (1, 20), (2, 20)]);
        let mut ev = EventQueue::default();
        enqueue(&cc, &dc, &nod, 50, &mut p, 0, 3, &mut ev); // 9 × item0
        enqueue(&cc, &dc, &nod, 50, &mut p, 1, 2, &mut ev); // 4 × item1, 2 × item2
        assert_eq!(inv_count(&p.inv, 0), 21);
        assert_eq!(p.craft_done_at, 52);

        // Cancel the head mid-batch: full refund (nothing completed yet),
        // job 1 becomes the head and re-arms from `tick`.
        cancel(
            &cc,
            &gc,
            55,
            &mut p,
            0,
            &mut [ItemStack::default(); INV_SLOTS],
        );
        assert_eq!(inv_count(&p.inv, 0), 30, "9 refunded");
        assert_eq!(
            p.jobs[0],
            CraftJob {
                recipe: 1,
                remaining: 2
            }
        );
        assert_eq!(p.jobs[1], CraftJob::default());
        assert_eq!(p.craft_done_at, 58, "new head restarts its unit");

        // Cancel the now-head after one unit completes: only the
        // remaining unit refunds.
        step_nospill(&cc, &gc, 58, &mut p, &mut ev);
        assert_eq!(p.jobs[0].remaining, 1);
        cancel(
            &cc,
            &gc,
            60,
            &mut p,
            0,
            &mut [ItemStack::default(); INV_SLOTS],
        );
        assert_eq!(inv_count(&p.inv, 1), 18, "2 of 4 came back");
        assert_eq!(inv_count(&p.inv, 2), 19, "1 of 2 came back");
        assert_eq!(p.craft_done_at, 0);

        // Cancelling nothing is silent.
        let before = ev.len();
        cancel(
            &cc,
            &gc,
            61,
            &mut p,
            3,
            &mut [ItemStack::default(); INV_SLOTS],
        );
        cancel(
            &cc,
            &gc,
            61,
            &mut p,
            99,
            &mut [ItemStack::default(); INV_SLOTS],
        );
        assert_eq!(ev.len(), before);
    }

    #[test]
    fn placed_station_arms_the_gated_recipe() {
        use crate::build::{BuildContent, Pieces, LOC_PLANE};
        use crate::movement::Body;

        let (cc, _gc) = fixture();
        let dc = crate::deploy::DeployContent::probe_fixture();
        let mut nod = Deploys::new();
        let mut ev = EventQueue::default();
        // The browser-smoke cell (world::tests guards it walkable), so
        // the any-class workbench can stand on bare terrain.
        const SEED: u64 = 20260731;
        let mut p = player(&[(0, 10), (3, 1)]);
        p.body = Body::at(SEED, 1024.0, 1024.0);
        crate::deploy::place_deploy(
            SEED,
            &dc,
            &BuildContent::probe_fixture(),
            &mut Pieces::new(),
            &mut nod,
            &mut p,
            0,
            1, // fixture row 1: the workbench
            341,
            341,
            0,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(
            ev.entries()[ev.len() - 1].code,
            crate::world::EV_DEPLOY_PLACED
        );

        // Beside the bench, the workbench recipe enqueues — once its
        // blueprint is learned, which row 2 also wants (research v0).
        p.known |= 1 << 2;
        enqueue(&cc, &dc, &nod, 10, &mut p, 2, 1, &mut ev);
        assert_eq!(
            p.jobs[0],
            CraftJob {
                recipe: 2,
                remaining: 1
            }
        );

        // Out of the station radius, it refuses again — with the
        // blueprint learned, so the reason is the distance and not the
        // gate that outranks it.
        let mut far = player(&[(0, 10)]);
        far.known |= 1 << 2;
        far.body = Body::at(SEED, 1024.0 + STATION_RADIUS_M + 2.0, 1024.0);
        enqueue(&cc, &dc, &nod, 10, &mut far, 2, 1, &mut ev);
        let e = ev.entries()[ev.len() - 1];
        assert_eq!((e.code, e.b), (EV_CRAFT_REFUSED, REFUSE_STATION));
    }

    /// The ladder's own gate (bench ladder v0), both directions of the ≥:
    /// a tier-2 recipe refuses at a tier-1 bench with the bench in reach —
    /// the defect the old if/else would have shipped, a level-3 recipe
    /// crafting at a level-1 bench — and a HIGHER bench satisfies a lower
    /// recipe, so upgrading a bench never costs a verb.
    #[test]
    fn a_higher_recipe_refuses_at_a_lower_bench_and_not_the_reverse() {
        use crate::build::{BuildContent, Pieces, LOC_PLANE};
        use crate::deploy::{DeployDef, ARCH_WORKBENCH2, PLACE_ANY};
        use crate::movement::Body;

        // The shared fixture, plus: recipe 2 re-gated to the second rung,
        // and a tier-2 bench def appended past the shared set (the
        // `boxed_fixture` precedent — local to this test, so the parity
        // and replay worlds never see it).
        let (mut cc, _gc) = fixture();
        cc.recipes[2].station = STATION_WORKBENCH2;
        let mut dc = crate::deploy::DeployContent::probe_fixture();
        let wb2_row = dc.def_count as usize;
        dc.defs[wb2_row] = DeployDef {
            arch: ARCH_WORKBENCH2,
            placement: PLACE_ANY,
            hp: 80,
            item: 3,
            n_costs: 1,
            costs: [(0, 20), (0, 0), (0, 0), (0, 0)],
        };
        dc.def_count += 1;

        const SEED: u64 = 20260731;
        let bc = BuildContent::probe_fixture();
        let mut nod = Deploys::new();
        let mut ev = EventQueue::default();
        let mut p = player(&[(0, 40), (3, 2)]);
        p.known |= 1 << 2;
        p.body = Body::at(SEED, 1024.0, 1024.0);

        // A tier-1 bench beside the crafter: the tier-2 recipe still
        // refuses on the station, because near is not rung enough.
        crate::deploy::place_deploy(
            SEED,
            &dc,
            &bc,
            &mut Pieces::new(),
            &mut nod,
            &mut p,
            0,
            1, // fixture row 1: the tier-1 workbench
            341,
            341,
            0,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(
            ev.entries()[ev.len() - 1].code,
            crate::world::EV_DEPLOY_PLACED
        );
        enqueue(&cc, &dc, &nod, 10, &mut p, 2, 1, &mut ev);
        let e = ev.entries()[ev.len() - 1];
        assert_eq!((e.code, e.b), (EV_CRAFT_REFUSED, REFUSE_STATION));

        // The tier-2 bench on the next cell arms it.
        crate::deploy::place_deploy(
            SEED,
            &dc,
            &bc,
            &mut Pieces::new(),
            &mut nod,
            &mut p,
            0,
            wb2_row as u16,
            342,
            341,
            0,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(
            ev.entries()[ev.len() - 1].code,
            crate::world::EV_DEPLOY_PLACED
        );
        enqueue(&cc, &dc, &nod, 10, &mut p, 2, 1, &mut ev);
        assert_eq!(
            p.jobs[0],
            CraftJob {
                recipe: 2,
                remaining: 1
            },
            "the matching rung arms the recipe"
        );

        // And the ≥, run the other way: a fresh crafter beside ONLY the
        // tier-2 bench asks for a tier-1 recipe — recipe 2 re-gated back
        // down — and the higher bench satisfies it.
        let mut cc1 = CraftContent::probe_fixture();
        cc1.recipes[2].station = STATION_WORKBENCH1;
        let mut only_wb2 = Deploys::new();
        let mut q = player(&[(0, 40), (3, 2)]);
        q.known |= 1 << 2;
        q.body = Body::at(SEED, 1024.0, 1024.0);
        crate::deploy::place_deploy(
            SEED,
            &dc,
            &bc,
            &mut Pieces::new(),
            &mut only_wb2,
            &mut q,
            0,
            wb2_row as u16,
            341,
            341,
            0,
            LOC_PLANE,
            &mut ev,
        );
        enqueue(&cc1, &dc, &only_wb2, 10, &mut q, 2, 1, &mut ev);
        assert_eq!(
            q.jobs[0],
            CraftJob {
                recipe: 2,
                remaining: 1
            },
            "a higher bench satisfies a lower recipe"
        );
    }

    /// Was `overflowing_output_is_lost_not_wedged` until 2026-08-14: the
    /// output is no longer lost, it spills. Every assertion that test made
    /// is still made here — the event still reports **zero** reaching the
    /// hands, and the batch still advances — with the one that matters
    /// added, that the units are in the spill rather than nowhere. The
    /// queue not wedging on a full pack is the half that was always the
    /// point, and it is unchanged.
    #[test]
    fn overflowing_output_spills_and_the_queue_still_advances() {
        let (cc, gc) = fixture();
        let (dc, nod) = (DeployContent::EMPTY, Deploys::new());
        // 7 of item 0: the batch consumes 6, so slot 0 keeps one unit and
        // stays occupied — no slot frees up for the output.
        let mut p = player(&[(0, 7)]);
        // Fill every other slot with the output item at stack cap.
        for s in p.inv.iter_mut().skip(1) {
            *s = ItemStack {
                item: 2,
                count: 100,
                cond: 0,
            };
        }
        let mut ev = EventQueue::default();
        let mut spill = [ItemStack::default(); INV_SLOTS];
        enqueue(&cc, &dc, &nod, 10, &mut p, 0, 2, &mut ev);
        step(&cc, &gc, 12, &mut p, &mut ev, &mut spill);
        let e = ev.entries()[ev.len() - 1];
        assert_eq!(e.code, EV_CRAFT_DONE);
        assert_eq!(
            e.b & 0xFFFF,
            0,
            "nothing reached the hands, and the event says so"
        );
        assert_eq!(p.jobs[0].remaining, 1, "the batch still advances");
        assert_eq!(
            inv_count(&spill, 2),
            cc.recipes[0].out_count as u32,
            "the whole output fell to the spill instead of being destroyed"
        );
    }
}
