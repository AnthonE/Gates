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
//! refunds the remaining units' inputs · output (and refund) an inventory
//! can't hold is lost, the same documented policy as gather until ground
//! drops land · station-gated recipes refuse until placed stations exist
//! (the build slice).

use crate::gather::{inv_add, GatherContent, ItemStack};
use crate::limits::{
    CRAFT_COUNT_MAX, CRAFT_QUEUE, INV_SLOTS, MAX_ITEM_DEFS, MAX_RECIPES, MAX_RECIPE_INPUTS,
};
use crate::world::{EventQueue, Player, EV_CRAFT_DONE, EV_CRAFT_REFUSED};

/// Station codes (schema order: CONTENT.md §1 `none|workbench1|furnace`).
pub const STATION_NONE: u8 = 0;
pub const STATION_WORKBENCH1: u8 = 1;
pub const STATION_FURNACE: u8 = 2;

/// Integer refusal reasons (CLAUDE.md wall 3: integer event codes only),
/// carried by EV_CRAFT_REFUSED / the craft-refused wire subtype.
pub const REFUSE_RECIPE: u32 = 0;
pub const REFUSE_COUNT: u32 = 1;
pub const REFUSE_STATION: u32 = 2;
pub const REFUSE_QUEUE_FULL: u32 = 3;
pub const REFUSE_INPUTS: u32 = 4;

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
    /// `STATION_*` code. Anything but `STATION_NONE` refuses at enqueue
    /// until placed stations exist (the build slice).
    pub station: u8,
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
    /// probe fixture's 8 items (fixture, not game content). Row 2 is
    /// station-gated so the refusal path is inside the gates too.
    pub fn probe_fixture() -> Self {
        let mut c = Self::EMPTY;
        c.recipe_count = 3;
        c.recipes[0] = RecipeDef {
            output: 2,
            out_count: 2,
            ticks: 2,
            station: STATION_NONE,
            n_inputs: 1,
            inputs: [(0, 3), (0, 0), (0, 0), (0, 0)],
        };
        c.recipes[1] = RecipeDef {
            output: 3,
            out_count: 1,
            ticks: 3,
            station: STATION_NONE,
            n_inputs: 2,
            inputs: [(1, 2), (2, 1), (0, 0), (0, 0)],
        };
        c.recipes[2] = RecipeDef {
            output: 4,
            out_count: 1,
            ticks: 1,
            station: STATION_WORKBENCH1,
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
pub fn enqueue(
    cc: &CraftContent,
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
    if def.station != STATION_NONE {
        // No placed stations exist yet; the build slice turns this into a
        // proximity check instead of a flat refusal.
        events.push(EV_CRAFT_REFUSED, p.id, REFUSE_STATION, 0);
        return;
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

/// Apply one cancel (`Command::CraftCancel`): refund the remaining units'
/// inputs (the in-progress unit refunds whole) and close the gap. An
/// index naming no live job is ignored — a cancel racing a completion is
/// a race, not an attack.
pub fn cancel(cc: &CraftContent, gc: &GatherContent, tick: u64, p: &mut Player, index: u16) {
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
                let added = inv_add(&mut p.inv, item, chunk, gc.stack_max[item as usize]);
                left -= chunk as u32;
                if added < chunk {
                    break; // inventory full: the rest is lost (documented)
                }
            }
        }
    }
    shift_left(&mut p.jobs, index);
    if index == 0 {
        p.craft_done_at = if p.jobs[0].remaining > 0 {
            tick + cc.recipes[p.jobs[0].recipe as usize].ticks as u64
        } else {
            0
        };
    }
}

/// One player's per-tick craft progress: at most one unit completes per
/// tick (recipe ticks are ≥ 1 by bake), paying the output and starting
/// the next unit's — or the next job's — timer.
pub fn step(
    cc: &CraftContent,
    gc: &GatherContent,
    tick: u64,
    p: &mut Player,
    events: &mut EventQueue,
) {
    if p.jobs[0].remaining == 0 || tick < p.craft_done_at {
        return;
    }
    let recipe = p.jobs[0].recipe;
    if recipe >= cc.recipe_count {
        // A table swap shrank the set under a live job (content hotfix):
        // drop the job rather than pay from a stale row.
        shift_left(&mut p.jobs, 0);
        p.craft_done_at = if p.jobs[0].remaining > 0 {
            tick + cc.recipes[p.jobs[0].recipe as usize].ticks as u64
        } else {
            0
        };
        return;
    }
    let def = &cc.recipes[recipe as usize];
    let added = inv_add(
        &mut p.inv,
        def.output,
        def.out_count,
        gc.stack_max[def.output as usize],
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
    p.craft_done_at = if p.jobs[0].remaining > 0 {
        tick + cc.recipes[p.jobs[0].recipe as usize].ticks as u64
    } else {
        0
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gather::NO_CELL;
    use crate::input::InputFrame;
    use crate::movement::Body;

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
        };
        for (i, &(item, count)) in inv0.iter().enumerate() {
            p.inv[i] = ItemStack { item, count };
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
        assert_eq!(p.inv[2], ItemStack { item: 0, count: 2 });
        assert_eq!(inv_count(&p.inv, 0), 2);
        assert_eq!(inv_take(&mut p.inv, 0, 9), 2, "partial take reports");
    }

    #[test]
    fn enqueue_consumes_starts_and_step_pays() {
        let (cc, gc) = fixture();
        let mut p = player(&[(0, 10)]);
        let mut ev = EventQueue::default();
        enqueue(&cc, 100, &mut p, 0, 2, &mut ev);
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

        step(&cc, &gc, 101, &mut p, &mut ev);
        assert!(ev.is_empty(), "not due yet");
        step(&cc, &gc, 102, &mut p, &mut ev);
        assert_eq!(ev.len(), 1);
        assert_eq!(ev.entries()[0].code, EV_CRAFT_DONE);
        assert_eq!(ev.entries()[0].b, (2 << 16) | 2, "item 2 × 2 landed");
        assert_eq!(inv_count(&p.inv, 2), 2);
        assert_eq!(p.jobs[0].remaining, 1);
        assert_eq!(p.craft_done_at, 104, "next unit re-arms");

        step(&cc, &gc, 104, &mut p, &mut ev);
        assert_eq!(p.jobs[0], CraftJob::default(), "batch done, queue empty");
        assert_eq!(p.craft_done_at, 0);
        assert_eq!(inv_count(&p.inv, 2), 4);
    }

    #[test]
    fn refusals_name_their_reason_and_change_nothing() {
        let (cc, _gc) = fixture();
        let mut p = player(&[(0, 100), (1, 100), (2, 100)]);
        let mut ev = EventQueue::default();
        let cases: [(u16, u16, u32); 4] = [
            (99, 1, REFUSE_RECIPE),
            (0, 0, REFUSE_COUNT),
            (0, CRAFT_COUNT_MAX + 1, REFUSE_COUNT),
            (2, 1, REFUSE_STATION),
        ];
        for (recipe, count, reason) in cases {
            enqueue(&cc, 10, &mut p, recipe, count, &mut ev);
            let e = ev.entries()[ev.len() - 1];
            assert_eq!((e.code, e.a, e.b), (EV_CRAFT_REFUSED, 7, reason));
        }
        // Missing inputs: recipe 1 wants 2×item1 + 1×item2 per unit.
        let mut poor = player(&[(1, 1)]);
        enqueue(&cc, 10, &mut poor, 1, 1, &mut ev);
        let e = ev.entries()[ev.len() - 1];
        assert_eq!(e.b, REFUSE_INPUTS);
        assert_eq!(inv_count(&poor.inv, 1), 1, "nothing consumed on refusal");
        assert_eq!(poor.jobs[0], CraftJob::default());
        // Queue full: fill all four, the fifth bounces.
        let mut busy = player(&[(0, 90)]);
        for _ in 0..CRAFT_QUEUE {
            enqueue(&cc, 10, &mut busy, 0, 1, &mut ev);
        }
        assert!(busy.jobs.iter().all(|j| j.remaining == 1));
        let before = ev.len();
        enqueue(&cc, 10, &mut busy, 0, 1, &mut ev);
        assert_eq!(ev.entries()[before].b, REFUSE_QUEUE_FULL);
    }

    #[test]
    fn cancel_refunds_remaining_and_rearms_the_head() {
        let (cc, gc) = fixture();
        let mut p = player(&[(0, 30), (1, 20), (2, 20)]);
        let mut ev = EventQueue::default();
        enqueue(&cc, 50, &mut p, 0, 3, &mut ev); // 9 × item0
        enqueue(&cc, 50, &mut p, 1, 2, &mut ev); // 4 × item1, 2 × item2
        assert_eq!(inv_count(&p.inv, 0), 21);
        assert_eq!(p.craft_done_at, 52);

        // Cancel the head mid-batch: full refund (nothing completed yet),
        // job 1 becomes the head and re-arms from `tick`.
        cancel(&cc, &gc, 55, &mut p, 0);
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
        step(&cc, &gc, 58, &mut p, &mut ev);
        assert_eq!(p.jobs[0].remaining, 1);
        cancel(&cc, &gc, 60, &mut p, 0);
        assert_eq!(inv_count(&p.inv, 1), 18, "2 of 4 came back");
        assert_eq!(inv_count(&p.inv, 2), 19, "1 of 2 came back");
        assert_eq!(p.craft_done_at, 0);

        // Cancelling nothing is silent.
        let before = ev.len();
        cancel(&cc, &gc, 61, &mut p, 3);
        cancel(&cc, &gc, 61, &mut p, 99);
        assert_eq!(ev.len(), before);
    }

    #[test]
    fn overflowing_output_is_lost_not_wedged() {
        let (cc, gc) = fixture();
        // 7 of item 0: the batch consumes 6, so slot 0 keeps one unit and
        // stays occupied — no slot frees up for the output.
        let mut p = player(&[(0, 7)]);
        // Fill every other slot with the output item at stack cap.
        for s in p.inv.iter_mut().skip(1) {
            *s = ItemStack {
                item: 2,
                count: 100,
            };
        }
        let mut ev = EventQueue::default();
        enqueue(&cc, 10, &mut p, 0, 2, &mut ev);
        step(&cc, &gc, 12, &mut p, &mut ev);
        let e = ev.entries()[ev.len() - 1];
        assert_eq!(e.code, EV_CRAFT_DONE);
        assert_eq!(e.b & 0xFFFF, 0, "nothing fit; the loss is announced");
        assert_eq!(p.jobs[0].remaining, 1, "the batch still advances");
    }
}
