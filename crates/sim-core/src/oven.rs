//! Ovens — the campfire and the furnace, which are one thing (oven v0,
//! DECISIONS.md §open). A placed `ARCH_FIRE` or `ARCH_FURNACE` deployable
//! is a container that **burns**: it holds fuel and inputs in the same
//! box store a storage box uses, a use press lights or snuffs it, and
//! while it is lit a bounded sweep spends fuel, banks the fuel's
//! byproduct, and advances one cook row per occupied slot.
//!
//! **One class for both, and that is the reference's own shape.** Every
//! cooking hook in `reference/rust-systems.txt` hangs off a single
//! `BaseOven` — `OnOvenToggle SVSwitch(…)`, `OnOvenStart StartCooking()`,
//! `OnOvenCook Cook(Single)`, `OnFuelConsume ConsumeFuel(Item,
//! ItemModBurnable)` — and the campfire, the furnace and every later
//! cooker are that class with different data. So the archetype here is a
//! *content* discriminator on a cook row and never a second code path:
//! what separates a fire from a furnace is which rows name it.
//!
//! Model v0 (proposed defaults, DECISIONS.md §open "oven v0"):
//! - **The contents are a box.** An oven's slots live in `deploy.rs`'s
//!   `BoxStore`, so `CONT_BOX` addresses it, the open/move/sync verbs
//!   work unchanged, and no wire field was invented for a second kind of
//!   container. The oven's *state* rides an index-aligned array beside
//!   it, exactly as `bag_ready` rides beside the deploy records and for
//!   the same reason: `DeployRec` is what the deploy-sync packet mirrors,
//!   so a burn timer may not ride on it.
//! - **A use press is the switch** (`world.rs` routes `Command::Use` by
//!   address). Lighting needs fuel already inside — the reference's
//!   ignite is not a fuel grant — and refuses with `REFUSE_D_FUEL`
//!   otherwise. Snuffing always works. An oven that runs out of fuel
//!   snuffs itself, with the same event and no actor.
//! - **Only fuel and inputs go in.** A move into an oven that names
//!   anything else refuses (`inventory::REFUSE_M_OVEN`). That is the
//!   reference's 2025 rule ("you'll no longer be able to store non
//!   cooking/smelting-related items inside your cooking deployables"),
//!   and it is what keeps a 100-wood campfire from being a cheaper
//!   storage box than the 100-wood box that exists to be one.
//! - **The byproduct is banked, not rolled.** The reference gives ~75% of
//!   a charcoal per wood by chance; a chance is an RNG draw in the tick
//!   and this crate would rather carry an integer. `byproduct_pct` is
//!   added to a per-oven accumulator each fuel unit and pays one unit per
//!   100, so a hundred wood yields exactly the reference's expectation
//!   with nothing random in the hash.
//! - **Output lands in the oven**, topping up its own slots the way a
//!   pickup tops up an inventory. An oven with no room keeps what it has:
//!   the conversion does not run, the progress stays where it is, and
//!   nothing is destroyed. (`inv_add`'s take-what-fits is wrong here for
//!   the reason `inventory.rs` gives — a half-applied conversion is a
//!   dupe or a loss depending on which half you count.)
//!
//! **The recycler is the third one, and it is the same class again**
//! (recycler v0, `DECISIONS.md` §open). `ARCH_RECYCLER` holds items, takes
//! a use press, and advances on this sweep — what it does not do is burn,
//! so the fuel half of a step runs only for the archetypes that
//! [`OvenState::burns`]. Two deltas, both small, and both bought
//! deliberately rather than by writing a second module:
//! - **No fuel.** A recycler lights on a press with nothing laid in it,
//!   because there is nothing to lay. `accepts` therefore refuses wood at
//!   a recycler: an archetype that cannot burn must not become a wood
//!   chest by inheriting the fuel clause.
//! - **More than one output per conversion.** A `CookRow` pays `count`
//!   units now, and **every row sharing an (archetype, input) fires
//!   together** — that is what makes a gear worth metal *and* coin without
//!   a second table. The bake holds such rows to one `seconds`
//!   (`validate::structural`), because the timer belongs to the slot; and
//!   the whole conversion is applied to a scratch copy and committed only
//!   if all of it fits, since with two outputs the half-applied case is
//!   reachable rather than theoretical (`inventory.rs`: a half-applied
//!   move is a dupe or a loss depending on which half you count).
//!
//! **What the recycler pays is content, and that is the point.** Nothing
//! here knows the difference between paying metal and paying a coin, so
//! arming the economy's first faucet (`ALPHA.md` A2 — an operator act) is
//! a row in `content/cooking.toml` and not a line in this file.
//!
//! Not in this slice (documented, not forgotten): no burnt state (the
//! reference's overcook), because a burnt row is a cook row whose input
//! is a cooked item and the food to demonstrate it does not exist yet;
//! no spoilage clock; no warmth, comfort or light radius (the sim has no
//! temperature); no smelting migration — the furnace's ore rows are
//! still station-gated crafts in `content/recipes.toml`, and moving them
//! here is a content decision that re-prices the powder chain, not a
//! code one.

use crate::deploy::{Deploys, ARCH_FIRE, ARCH_FURNACE, ARCH_RECYCLER};
use crate::gather::{GatherContent, ItemStack};
use crate::limits::{BOX_SLOTS, MAX_COOK_ROWS};
use crate::world::{EventQueue, EV_OVEN};

/// Ticks between one oven's steps. Every oven steps on the tick where its
/// index and the tick agree mod this, so the store is walked in
/// `OVEN_PERIOD_TICKS` slices and no tick visits more than
/// `ceil(MAX_BOXES / OVEN_PERIOD_TICKS)` of them — bounded work (wall 4)
/// without a cursor to persist, and every oven still advances at one real
/// rate. Timers below therefore move in whole periods, never in ticks.
///
/// Bounded-work constant, not a knob: it is the sweep's shape and no
/// content value depends on it (a cook row states seconds, and the step
/// adds a period to a tick counter).
pub const OVEN_PERIOD_TICKS: u64 = 15;

/// One baked cook row: what goes in, what comes out, how long, and which
/// oven does it. `ticks == 0` ⇒ inert (the empty-table row).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CookRow {
    pub input: u16,
    pub output: u16,
    /// Units of `output` one conversion pays. One for every cooking row —
    /// a fire turns one steak into one steak — and the reason the field
    /// exists is the recycler, where a component is worth eight fragments
    /// and the ladder would otherwise be eight rows deep. The bake refuses
    /// zero, so a live row always pays.
    pub count: u16,
    /// Ticks one unit takes (content seconds × TICK_HZ; the bake keeps
    /// this ≥ 1, so nothing converts on the step it starts).
    ///
    /// Every row sharing this row's `(arch, input)` carries the same value
    /// — `validate::structural` refuses a set that disagrees — because the
    /// timer lives on the container's slot and one slot cannot hold two
    /// clocks.
    pub ticks: u32,
    /// `ARCH_FIRE`, `ARCH_FURNACE` or `ARCH_RECYCLER` — which container
    /// runs the row.
    pub arch: u8,
}

impl CookRow {
    pub const INERT: Self = Self {
        input: 0,
        output: 0,
        count: 0,
        ticks: 0,
        arch: ARCH_FIRE,
    };
}

/// The whole oven ruleset the sim knows, baked from
/// `content/cooking.toml`. Construction input like the gather table: the
/// boot path installs it before the first tick and the WAL pins the
/// content hash it came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CookContent {
    /// What burns. `fuel_ticks == 0` ⇒ nothing burns, so nothing lights.
    pub fuel_item: u16,
    pub fuel_ticks: u32,
    /// What a burned unit leaves behind, and how much of one per unit in
    /// hundredths. `byproduct_pct == 0` ⇒ fuel leaves nothing.
    pub byproduct: u16,
    pub byproduct_pct: u16,
    pub rows: [CookRow; MAX_COOK_ROWS],
    pub row_count: u16,
}

impl CookContent {
    /// Inert: nothing burns, nothing cooks, every light refuses.
    /// `World::new` starts here.
    pub const EMPTY: Self = Self {
        fuel_item: 0,
        fuel_ticks: 0,
        byproduct: 0,
        byproduct_pct: 0,
        rows: [CookRow::INERT; MAX_COOK_ROWS],
        row_count: 0,
    };

    /// Synthetic table for the parity/replay/alloc gates, over the gather
    /// probe fixture's 8 items (fixture, not game content). Item 0 burns,
    /// leaves item 5 at half a unit each, and item 1 cooks into item 6 on
    /// a fire — so both halves of the sweep run inside the gates even
    /// while the shipped content has no cook row.
    pub fn probe_fixture() -> Self {
        let mut c = Self::EMPTY;
        c.fuel_item = 0;
        c.fuel_ticks = 30;
        c.byproduct = 5;
        c.byproduct_pct = 50;
        c.row_count = 3;
        c.rows[0] = CookRow {
            input: 1,
            output: 6,
            count: 1,
            ticks: 60,
            arch: ARCH_FIRE,
        };
        // The recycler's half (recycler v0), and it is deliberately TWO
        // rows over one input: item 2 pays item 6 twice and item 7 once on
        // a single 30-tick timer, which is the multi-row conversion path —
        // every row over an input firing together, applied to a scratch
        // copy and committed whole. One row would exercise nothing the
        // fire above does not.
        //
        // The probe script places a recycler (deploy row 6) and switches
        // it on, so the archetype's own branch — the burn that is skipped,
        // and the match that must not refuse for want of fuel — runs
        // inside parity, replay and alloc. What it does NOT drive is a
        // conversion, because no command in the probe puts an item inside
        // a container; the fire has had that gap since oven v0 and
        // `tests/oven.rs` is where both are actually gated.
        c.rows[1] = CookRow {
            input: 2,
            output: 6,
            count: 2,
            ticks: 30,
            arch: ARCH_RECYCLER,
        };
        c.rows[2] = CookRow {
            input: 2,
            output: 7,
            count: 1,
            ticks: 30,
            arch: ARCH_RECYCLER,
        };
        c
    }

    /// The row that carries the CLOCK for `item` at this archetype — the
    /// first of what may be several. Every row over one `(arch, input)`
    /// shares a `ticks` (the bake refuses a set that does not), so "the
    /// first" and "all of them" agree about when the conversion lands, and
    /// only the pay differs.
    pub fn row_for(&self, arch: u8, item: u16) -> Option<&CookRow> {
        self.rows[..self.row_count as usize]
            .iter()
            .find(|r| r.ticks > 0 && r.arch == arch && r.input == item)
    }

    /// Every live row over one `(arch, input)` — what a finished
    /// conversion pays, in table order.
    pub fn rows_for(&self, arch: u8, item: u16) -> impl Iterator<Item = &CookRow> {
        self.rows[..self.row_count as usize]
            .iter()
            .filter(move |r| r.ticks > 0 && r.arch == arch && r.input == item)
    }

    /// May `item` go into a container of this archetype at all — the move
    /// verb's whole question (`inventory::REFUSE_M_OVEN`).
    ///
    /// The fuel and byproduct clauses are asked of BURNERS only. A
    /// recycler that inherited them would accept wood it can never spend
    /// and charcoal it can never make — a second wood chest wearing an
    /// oven's rule, which is the exact thing that rule exists to prevent.
    pub fn accepts(&self, arch: u8, item: u16) -> bool {
        let burns = OvenState::arch_burns(arch);
        (burns && self.fuel_ticks > 0 && item == self.fuel_item)
            || self.row_for(arch, item).is_some()
            // What the container itself made stays where it landed: an
            // output that could not be moved back out through its own door
            // would be trapped by the rule meant to keep junk out.
            || (burns && self.byproduct_pct > 0 && item == self.byproduct)
            || self.rows[..self.row_count as usize]
                .iter()
                .any(|r| r.ticks > 0 && r.arch == arch && r.output == item)
    }
}

/// One oven's own state, index-aligned to `Deploys::boxes()`. Every
/// container carries one; `arch` is what says whether it is an oven at
/// all, and it is the placed deployable's archetype rather than a bool so
/// the cook rows can be selected without a second lookup into the deploy
/// store on every step.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OvenState {
    /// The owning deployable's archetype (`deploy::ARCH_*`). A storage
    /// box carries `ARCH_BOX` here and `is_oven` is false for it.
    pub arch: u8,
    pub lit: bool,
    /// Ticks left on the fuel unit currently burning. Zero with `lit` set
    /// means the next step must consume a unit or snuff the oven.
    pub burn: u16,
    /// Byproduct accumulator in hundredths of a unit.
    pub bank: u16,
    /// Per-slot cook progress in ticks. Frozen, not reset, while the oven
    /// is out: a fire you relight finishes what it started, which is the
    /// kinder reading and the one that cannot lose a player's food to a
    /// misclick.
    pub cook: [u16; BOX_SLOTS],
}

impl OvenState {
    /// Does this archetype spend fuel to run? Fire and furnace do; the
    /// recycler does not, and a storage box is not a converter at all.
    ///
    /// An associated fn rather than a method because `CookContent::accepts`
    /// asks it of a bare archetype — the move verb knows what it is moving
    /// into before it has a state row in hand.
    pub fn arch_burns(arch: u8) -> bool {
        arch == ARCH_FIRE || arch == ARCH_FURNACE
    }

    /// Does this container convert at all — the sweep's filter and
    /// `oven_index`'s. A recycler answers yes and burns nothing.
    pub fn arch_converts(arch: u8) -> bool {
        Self::arch_burns(arch) || arch == ARCH_RECYCLER
    }

    pub fn burns(&self) -> bool {
        Self::arch_burns(self.arch)
    }

    pub fn is_converter(&self) -> bool {
        Self::arch_converts(self.arch)
    }
}

/// Add `amount` of `item` to a container's slots: top up matching stacks
/// in slot order, then fill empties. Returns what fit.
///
/// `gather::inv_add`'s shape against a slice, because an oven's slots are
/// `BOX_SLOTS` wide and that function is typed to `INV_SLOTS`. Callers
/// here refuse to run a conversion that would not fit whole, so the
/// take-what-fits return is checked and never a silent loss.
fn slots_add(slots: &mut [ItemStack], item: u16, amount: u16, stack_max: u16) -> u16 {
    if stack_max == 0 {
        return 0;
    }
    let mut left = amount;
    for s in slots.iter_mut() {
        if left == 0 {
            break;
        }
        if s.count > 0 && s.item == item && s.count < stack_max {
            let room = stack_max - s.count;
            let put = room.min(left);
            s.count += put;
            left -= put;
        }
    }
    for s in slots.iter_mut() {
        if left == 0 {
            break;
        }
        if s.count == 0 {
            let put = stack_max.min(left);
            *s = ItemStack { item, count: put };
            left -= put;
        }
    }
    amount - left
}

/// Would `amount` of `item` fit whole? The question `slots_add` cannot be
/// asked after the fact, and every conversion below asks it first.
fn slots_room(slots: &[ItemStack], item: u16, amount: u16, stack_max: u16) -> bool {
    if stack_max == 0 {
        return false;
    }
    let mut room = 0u32;
    for s in slots.iter() {
        if s.count == 0 {
            room += stack_max as u32;
        } else if s.item == item && s.count < stack_max {
            room += (stack_max - s.count) as u32;
        }
        if room >= amount as u32 {
            return true;
        }
    }
    false
}

/// Announce an oven's state. `by` is the hand that pressed, or 0 when the
/// oven snuffed itself — the same posture `EV_SLOT_RESPAWNED` takes for a
/// fact with no actor behind it.
fn announce(cx: u16, cz: u16, level: u8, lit: bool, by: u32, events: &mut EventQueue) {
    events.push(
        EV_OVEN,
        crate::gather::cell_key(cx, cz),
        ((level as u32) << 16) | lit as u32,
        by,
    );
}

/// Integer refusal reason for a use press on an oven with nothing to
/// burn. Lives in `deploy.rs`'s `REFUSE_D_*` numbering because it rides
/// `EV_DEPLOY_REFUSED` with the rest of the deployable verbs' refusals.
pub use crate::deploy::REFUSE_D_FUEL;

/// Apply one use press against the oven at the address: light it, or
/// snuff it. Reach is the box's own (`BUILD_REACH_M`, planar), so the
/// press that opens a fire is the press that can light it.
///
/// Returns false when no oven stands there — `world.rs` then routes the
/// press to the door verb, which owns the same command.
pub fn toggle(
    cc: &CookContent,
    deploys: &mut Deploys,
    p: &crate::world::Player,
    cx: u16,
    cz: u16,
    level: u8,
    events: &mut EventQueue,
) -> bool {
    let Some(i) = deploys.oven_index(crate::deploy::box_key(cx, cz, level)) else {
        return false;
    };
    if !deploys.box_in_reach(i, p) {
        events.push(
            crate::world::EV_DEPLOY_REFUSED,
            p.id,
            crate::deploy::REFUSE_D_REACH,
            0,
        );
        return true;
    }
    let (boxes, ovens) = deploys.oven_parts_mut();
    let st = &mut ovens[i];
    if st.lit {
        st.lit = false;
        announce(cx, cz, level, false, p.id, events);
        return true;
    }
    // Fuel already inside is the whole condition: a match lights what is
    // laid, it does not lay it. Asked of BURNERS only — a recycler has no
    // fuel to lay, so the switch is just a switch, and refusing it for
    // want of something it never consumes would be a rule with no reason
    // behind it.
    let has_fuel = !st.burns()
        || st.burn > 0
        || (cc.fuel_ticks > 0
            && boxes[i]
                .items
                .iter()
                .any(|s| s.count > 0 && s.item == cc.fuel_item));
    if !has_fuel {
        events.push(crate::world::EV_DEPLOY_REFUSED, p.id, REFUSE_D_FUEL, 0);
        return true;
    }
    st.lit = true;
    announce(cx, cz, level, true, p.id, events);
    true
}

/// Step every oven whose turn this tick is (`OVEN_PERIOD_TICKS`).
///
/// One step is, in order: spend the period off the burning unit; take a
/// fresh unit (and bank its byproduct) when the last one is gone, or
/// snuff the oven when there is none; then advance every slot holding a
/// cookable input and convert the ones that finished. The order matters
/// and is the reference's: an oven that runs dry this step does not also
/// cook this step.
pub fn sweep(
    cc: &CookContent,
    gather: &GatherContent,
    deploys: &mut Deploys,
    tick: u64,
    events: &mut EventQueue,
) {
    let period = OVEN_PERIOD_TICKS;
    let phase = tick % period;
    let (boxes, ovens) = deploys.oven_parts_mut();
    for i in (phase as usize..ovens.len()).step_by(period as usize) {
        if !ovens[i].is_converter() || !ovens[i].lit {
            continue;
        }
        let arch = ovens[i].arch;
        let step = period as u16;

        // 1. Fuel. The unit burning now, then the next one. Burners only:
        //    a recycler runs on nothing and can never snuff itself, which
        //    is why its switch is the only thing that turns it off.
        if ovens[i].burns() {
            ovens[i].burn = ovens[i].burn.saturating_sub(step);
            if ovens[i].burn == 0 {
                let taken = if cc.fuel_ticks == 0 {
                    false
                } else {
                    take_one(&mut boxes[i].items, cc.fuel_item)
                };
                if !taken {
                    ovens[i].lit = false;
                    let (cx, cz, level) = (boxes[i].cx, boxes[i].cz, boxes[i].level);
                    announce(cx, cz, level, false, 0, events);
                    continue;
                }
                ovens[i].burn = cc.fuel_ticks.min(u16::MAX as u32) as u16;
                // The byproduct, banked in hundredths and paid whole.
                ovens[i].bank = ovens[i].bank.saturating_add(cc.byproduct_pct);
                let units = ovens[i].bank / 100;
                if units > 0 {
                    let cap = gather.stack_max_of(cc.byproduct);
                    if slots_room(&boxes[i].items, cc.byproduct, units, cap) {
                        slots_add(&mut boxes[i].items, cc.byproduct, units, cap);
                        ovens[i].bank -= units * 100;
                    }
                    // No room: the bank keeps counting. Nothing is destroyed
                    // and nothing is paid twice — the next step that finds
                    // room pays the whole debt (bounded by the accumulator's
                    // own saturating add).
                }
            }
        }

        // 2. Convert. Every slot, because a fire cooks what is on it
        //    rather than one thing at a time (the reference's four-slot
        //    grill), and a recycler eats a hopper the same way.
        for s in 0..BOX_SLOTS {
            let stack = boxes[i].items[s];
            let Some(row) = cc.row_for(arch, stack.item).copied() else {
                ovens[i].cook[s] = 0;
                continue;
            };
            if stack.count == 0 {
                ovens[i].cook[s] = 0;
                continue;
            }
            ovens[i].cook[s] = ovens[i].cook[s].saturating_add(step);
            if (ovens[i].cook[s] as u32) < row.ticks {
                continue;
            }
            // Pay every row over this input, on a scratch copy, and commit
            // only if the WHOLE conversion fit. One output could be
            // checked in place (`slots_room`) and several cannot: the
            // first payment changes the room left for the second, so a
            // per-row check would pass a set that does not fit and leave a
            // half-converted container behind. Half-applied is a dupe or a
            // loss depending on which half you count (`inventory.rs`), so
            // the unit of work is the whole row set or none of it.
            //
            // `BOX_SLOTS` stacks on the stack, copied twice — no
            // allocation in the tick (wall 2), and `test_alloc_zero` drives
            // this branch through the probe fixture's fire.
            let mut scratch = boxes[i].items;
            scratch[s].count -= 1;
            if scratch[s].count == 0 {
                scratch[s] = ItemStack::default();
            }
            let mut fits = true;
            for r in cc.rows_for(arch, stack.item) {
                let cap = gather.stack_max_of(r.output);
                if slots_add(&mut scratch, r.output, r.count, cap) < r.count {
                    fits = false;
                    break;
                }
            }
            if !fits {
                // Full. Hold the finished unit on the fire rather than
                // burning it away: progress stays at the line, and the
                // step after a slot frees pays it out.
                ovens[i].cook[s] = row.ticks.min(u16::MAX as u32) as u16;
                continue;
            }
            boxes[i].items = scratch;
            ovens[i].cook[s] = 0;
        }
    }
}

/// Take one unit of `item` out of a container's slots, in slot order.
/// Returns whether a unit was found.
fn take_one(slots: &mut [ItemStack], item: u16) -> bool {
    for s in slots.iter_mut() {
        if s.count > 0 && s.item == item {
            s.count -= 1;
            if s.count == 0 {
                *s = ItemStack::default();
            }
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stacks(n: usize) -> Vec<ItemStack> {
        vec![ItemStack::default(); n]
    }

    #[test]
    fn slots_add_tops_up_then_fills() {
        let mut s = stacks(3);
        s[0] = ItemStack { item: 1, count: 8 };
        assert_eq!(slots_add(&mut s, 1, 5, 10), 5);
        assert_eq!(s[0], ItemStack { item: 1, count: 10 });
        assert_eq!(s[1], ItemStack { item: 1, count: 3 });
    }

    #[test]
    fn slots_room_agrees_with_what_add_takes() {
        let mut s = stacks(2);
        s[0] = ItemStack { item: 1, count: 9 };
        s[1] = ItemStack { item: 2, count: 4 };
        assert!(slots_room(&s, 1, 1, 10));
        assert!(!slots_room(&s, 1, 2, 10));
        assert_eq!(slots_add(&mut s, 1, 2, 10), 1);
    }

    /// A stack cap of zero is an item the content never declared, and
    /// nothing may be paid into a container for it.
    #[test]
    fn a_zero_cap_holds_nothing() {
        let mut s = stacks(2);
        assert!(!slots_room(&s, 1, 1, 0));
        assert_eq!(slots_add(&mut s, 1, 1, 0), 0);
    }

    #[test]
    fn take_one_empties_the_slot_canonically() {
        let mut s = stacks(2);
        s[1] = ItemStack { item: 3, count: 1 };
        assert!(take_one(&mut s, 3));
        assert_eq!(s[1], ItemStack::default());
        assert!(!take_one(&mut s, 3));
    }

    /// The fixture's two halves: item 0 burns, item 1 cooks, and nothing
    /// else may be put in a fire.
    #[test]
    fn the_fixture_accepts_only_its_own_items() {
        let cc = CookContent::probe_fixture();
        assert!(cc.accepts(ARCH_FIRE, 0), "fuel");
        assert!(cc.accepts(ARCH_FIRE, 1), "input");
        assert!(cc.accepts(ARCH_FIRE, 5), "byproduct comes back out");
        assert!(cc.accepts(ARCH_FIRE, 6), "output comes back out");
        assert!(!cc.accepts(ARCH_FIRE, 2), "everything else");
        assert!(
            !cc.accepts(ARCH_FURNACE, 1),
            "a fire row is not a furnace row"
        );
        assert!(cc.accepts(ARCH_FURNACE, 0), "fuel burns in both");
    }

    /// The inert table lights nothing: `fuel_ticks == 0` is the whole
    /// guard, and it is what `World::new` starts holding.
    #[test]
    fn the_empty_table_accepts_nothing() {
        let cc = CookContent::EMPTY;
        assert!(!cc.accepts(ARCH_FIRE, 0));
        assert!(cc.row_for(ARCH_FIRE, 0).is_none());
    }
}
