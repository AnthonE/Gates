//! The survival clock — the thing that says *now* (DESIGN.md §2, M1).
//!
//! Until this module a player could log in, stand still for an hour, and be
//! in precisely the state they started in. Gathering fed crafting fed
//! building that protected a body nothing threatened, and
//! `content/consumables.toml` authored five rows that were parsed,
//! validated and hashed into the WAL header by a sim that then never read
//! them. This is the smallest honest fix: **two meters that only ever fall,
//! and food that puts them back.**
//!
//! Three verbs, one tick order — drain, then hurt, then heal:
//!
//! 1. **Drain.** Food and water fall from full to empty over a span the
//!    content states. Nothing refills them but eating.
//! 2. **Hurt.** A meter at zero takes hp per minute; both at zero stack, so
//!    thirst and hunger together are the fast death, not a slower one.
//! 3. **Heal.** A consumed row's `health` is delivered over its `seconds`,
//!    not instantly — which is what makes a bandage a decision under
//!    pressure rather than a button.
//!
//! **Every rate is exact integer arithmetic**, because a sim that drifts by
//! a rounding step is a sim whose replay does not reproduce (wall 5). The
//! shape is one rational accumulator per meter: add the numerator each
//! tick, take out whole units by division, keep the remainder. No floats,
//! no `while`, no clock — the accumulator is bounded by its own denominator
//! and the work per tick is constant, so `test_alloc_zero` and the tick
//! jitter assert both hold (walls 1–3).
//!
//! **Where the gates actually stand on this module**, because a claim
//! about coverage is worth nothing unless it names the file that holds it:
//! `test_alloc_zero` installs `probe_fixture` on a hundred bodies and
//! asserts, inside the counted window, that meters drained to empty, that
//! a body starved and was granted a fresh pair, and that the eat verb both
//! landed and refused. `test_replay` installs it on sixty-four and pins the
//! resulting hash. `test_parity_wasm` runs `probe_combat`, which installs
//! it too, and asserts the native and wasm digests are byte-identical. Each
//! of those three arms this module in its own fixture; none of them
//! inherits it from another.
//!
//! Content reaches the sim only as a baked `SurvivalContent` table (wall
//! 7): the meters and their spans from `content/balance.toml`'s
//! `[survival]`, the consumable rows keyed by item index from
//! `content/consumables.toml`. The inert `EMPTY` default disarms the module
//! entirely — `max_food == 0` means no meter is granted at join, nothing
//! drains, and nothing starves — so a shard booted on content without a
//! `[survival]` section plays exactly the game it played before this module
//! existed.
//!
//! **What v0 deliberately does not do**, registered in `DECISIONS.md` §open
//! ("survival clock v0"): no temperature and no wetness (the `WET 36%` chip
//! in the reference frames is M2's, and it wants weather first), no
//! comfort, no metabolism from activity (sprinting costs exactly what
//! standing does — the span is wall-clock, not work), no food spoilage, no
//! stomach cap on how much you can eat at once beyond the meter's own
//! ceiling, and no day/night coupling (`DESIGN.md` §2 pairs them; the other
//! half is blocked behind the ground's structure moving from bump into
//! albedo).

use crate::gather::NO_ITEM;
use crate::limits::{MAX_ITEM_DEFS, TICK_HZ};
use crate::world::{
    EventQueue, Player, EV_CONSUMED, EV_CONSUME_REFUSED, EV_DEATH, EV_HEALTH, EV_VITALS,
};

/// Ticks in a minute — the denominator every per-minute rate in this
/// module divides by. Derived from `limits::TICK_HZ`, never restated.
pub const TICKS_PER_MIN: u32 = TICK_HZ * 60;

/// One item's consumable row, baked from `content/consumables.toml`. An
/// all-zero row means the item is not food: the whole table starts that
/// way, so eating a stack of wood is the same non-event as swinging it at
/// a person with no melee row.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ConsumableDef {
    /// Hit points delivered over `seconds` — not instantly (module note).
    pub health: u16,
    /// Food units restored, immediately.
    pub food: u16,
    /// Water units restored, immediately.
    pub water: u16,
    /// How long `health` takes to arrive. Zero with a nonzero `health`
    /// would be an instant heal; the bake refuses that row rather than
    /// letting a division by zero decide.
    pub seconds: u16,
}

impl ConsumableDef {
    /// Does this row do anything at all? The bake refuses a row that does
    /// not, so this is the sim's read of "is the held item food".
    #[inline]
    pub fn is_food(&self) -> bool {
        self.health > 0 || self.food > 0 || self.water > 0
    }
}

/// The whole survival ruleset the sim knows. Construction input like the
/// seed and the gather table; the WAL pins the content hash it was baked
/// from (CONTENT.md §0).
#[derive(Clone, Copy, Debug)]
pub struct SurvivalContent {
    /// Indexed by item index (the sorted-rank mapping `bake` owns).
    pub consumable: [ConsumableDef; MAX_ITEM_DEFS],
    /// Meter ceilings, and what a join grants. Zero is the inert default
    /// and disarms the module: no meter, no drain, no starvation.
    pub max_food: u16,
    pub max_water: u16,
    /// Ticks from a full meter to an empty one. The bake derives these
    /// from `[survival]`'s minutes so the sim never multiplies a clock.
    pub food_span_ticks: u32,
    pub water_span_ticks: u32,
    /// Hit points a minute while the matching meter reads zero. Both empty
    /// stack — the rates add before the accumulator sees them.
    pub starve_hp_per_min: u16,
    pub dehydrate_hp_per_min: u16,
}

impl SurvivalContent {
    pub const EMPTY: Self = Self {
        consumable: [ConsumableDef {
            health: 0,
            food: 0,
            water: 0,
            seconds: 0,
        }; MAX_ITEM_DEFS],
        max_food: 0,
        max_water: 0,
        food_span_ticks: 0,
        water_span_ticks: 0,
        starve_hp_per_min: 0,
        dehydrate_hp_per_min: 0,
    };

    /// Synthetic table for the parity/replay/alloc gates. Deliberately
    /// unlike game content: the spans are *seconds*, not the better part of
    /// an hour, so a drain, an empty meter and a starvation death all land
    /// inside a counted probe window instead of waiting out a real clock.
    /// Item 0 is food — the same fixture item `probe_fixture` arms as a
    /// weapon — so the consume verb has something to eat without the probe
    /// having to craft first.
    pub fn probe_fixture() -> Self {
        let mut c = Self::EMPTY;
        c.max_food = 100;
        c.max_water = 100;
        // 10 s and 8 s at 30 Hz: water empties first, as in the content.
        c.food_span_ticks = 10 * TICK_HZ;
        c.water_span_ticks = 8 * TICK_HZ;
        // Fast enough to kill inside the probe's window once both are dry.
        c.starve_hp_per_min = 600;
        c.dehydrate_hp_per_min = 900;
        c.consumable[0] = ConsumableDef {
            health: 20,
            food: 40,
            water: 30,
            seconds: 2,
        };
        c
    }

    /// The consumable row of an item, or `None` when the hand is empty, the
    /// item is off the table, or the row is not food.
    #[inline]
    pub fn row(&self, item: u16) -> Option<ConsumableDef> {
        if item == NO_ITEM || item as usize >= MAX_ITEM_DEFS {
            return None;
        }
        let d = self.consumable[item as usize];
        if d.is_food() {
            Some(d)
        } else {
            None
        }
    }

    /// Is this table armed? One read, so the three call sites cannot
    /// disagree about what "inert content" means.
    #[inline]
    pub fn armed(&self) -> bool {
        self.max_food > 0 || self.max_water > 0
    }
}

impl Default for SurvivalContent {
    fn default() -> Self {
        Self::EMPTY
    }
}

/// Why a consume did nothing. Announced, never silent — a button that eats
/// the input and says nothing is indistinguishable from a broken one.
pub const REFUSE_C_NOT_FOOD: u32 = 1;
/// The meter this row would have filled is already full (and it heals
/// nothing). Eating here would destroy the item for no effect.
pub const REFUSE_C_FULL: u32 = 2;

/// One rational step: add `num` to `acc`, take out whole units against
/// `den`, keep the remainder. The one piece of arithmetic every meter in
/// this module shares, so an exactness bug can only exist in one place.
///
/// `den == 0` yields zero units and leaves `acc` alone — the inert-content
/// path, and the reason no caller needs its own divide-by-zero guard.
#[inline]
fn tick_units(acc: &mut u32, num: u32, den: u32) -> u32 {
    if den == 0 || num == 0 {
        return 0;
    }
    *acc += num;
    let units = *acc / den;
    *acc -= units * den;
    units
}

/// What one survival step did to a player.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Nothing worth announcing moved.
    Quiet,
    /// A meter or hp changed; the caller has already been told via events.
    Changed,
    /// The clock killed them. The slot needs a respawn — the caller's job,
    /// exactly as it is for `combat::Strike::Killed`, because the spawn
    /// ring needs the whole world and this function needs one player.
    Died,
}

/// One tick of the clock for one player: drain, then hurt, then heal.
///
/// Order matters and is deliberate. Draining before hurting means the tick
/// a meter reaches zero is also the first tick it costs hp — no free tick.
/// Healing last means a bandage started this tick still pays out this tick,
/// so a player racing starvation with food in hand can win the race.
pub fn step(sc: &SurvivalContent, p: &mut Player, events: &mut EventQueue) -> Step {
    if !sc.armed() {
        return Step::Quiet;
    }
    let before = (p.food, p.water, p.hp);

    // 1 · Drain. Numerator is the ceiling, denominator the span, so the
    // meter reaches exactly zero on exactly the tick the span names.
    let f = tick_units(&mut p.food_acc, sc.max_food as u32, sc.food_span_ticks);
    p.food = p.food.saturating_sub(f as u16);
    let w = tick_units(&mut p.water_acc, sc.max_water as u32, sc.water_span_ticks);
    p.water = p.water.saturating_sub(w as u16);

    // 2 · Hurt. Both meters empty stack: the rates add before the
    // accumulator sees them, so two empty meters kill at the sum of their
    // rates and not at the worse of the two.
    let mut rate = 0u32;
    if p.food == 0 {
        rate += sc.starve_hp_per_min as u32;
    }
    if p.water == 0 {
        rate += sc.dehydrate_hp_per_min as u32;
    }
    let mut died = false;
    if rate == 0 {
        // Fed again: the partial minute does not bank against the next
        // famine. A meter refilled at 99% of a point of damage owes nothing.
        p.hurt_acc = 0;
    } else {
        let dmg = tick_units(&mut p.hurt_acc, rate, TICKS_PER_MIN);
        if dmg > 0 && p.hp > 0 {
            let dmg = if dmg > u16::MAX as u32 {
                u16::MAX
            } else {
                dmg as u16
            };
            p.hp = p.hp.saturating_sub(dmg);
            if p.hp == 0 {
                died = true;
            }
        }
    }

    // 3 · Heal. A consumed row's health arrives over its seconds; the same
    // rational accumulator, numerator the total, denominator the span.
    if p.heal_rem > 0 {
        let give = tick_units(&mut p.heal_acc, p.heal_total as u32, p.heal_span);
        if give > 0 {
            let give = (give as u16).min(p.heal_rem);
            p.heal_rem -= give;
            // A heal cannot outrun a death that already happened this tick;
            // hp 0 is the end of the tick's story, and the respawn clears
            // the pending heal with the rest of the body.
            if !died {
                let max = max_hp(p);
                p.hp = (p.hp + give).min(max);
            }
        }
        if p.heal_rem == 0 {
            clear_heal(p);
        }
    }

    if died {
        // A death is counted where it happens — the same rule `combat` keeps
        // at its own kill site, and the reason it is one line here rather
        // than one in the caller. Without it a clock death is invisible to
        // `spawn_pos_n(id, deaths)`, so a body that starved was put back on
        // the *identical* beach to starve on the same ground; the count is
        // also what the scoreboard and the wire's `deaths` field read.
        p.deaths = p.deaths.saturating_add(1);
        // Self-inflicted by the world: victim and killer are the same id,
        // which `EV_DEATH`'s own doc comment already anticipated ("equal to
        // `a` if that ever becomes possible"). This is that.
        events.push(EV_DEATH, p.id, p.id, 0);
        return Step::Died;
    }
    if (p.food, p.water, p.hp) != before {
        announce(sc, p, events);
        return Step::Changed;
    }
    Step::Quiet
}

/// Max hp for a player mid-heal. The survival module never learns
/// `CombatContent`, so the ceiling a heal clamps to is the hp the player
/// was granted at join — carried on the player rather than looked up, which
/// is also what makes a heal correct across a content reload.
#[inline]
fn max_hp(p: &Player) -> u16 {
    p.hp_max
}

/// Drop any in-flight heal. One place, so a respawn and a finished heal
/// cannot leave different residue in the state hash.
#[inline]
pub fn clear_heal(p: &mut Player) {
    p.heal_rem = 0;
    p.heal_total = 0;
    p.heal_span = 0;
    p.heal_acc = 0;
}

/// Grant a fresh body its meters. Called at join and at respawn, so a
/// player who starved does not respawn already starving.
#[inline]
pub fn grant(sc: &SurvivalContent, p: &mut Player) {
    p.food = sc.max_food;
    p.water = sc.max_water;
    p.food_acc = 0;
    p.water_acc = 0;
    p.hurt_acc = 0;
    clear_heal(p);
}

/// Announce the whole vitals truth, absolute. Own-fact and absolute for
/// exactly `EV_HEALTH`'s reason: a client that misses one hears the whole
/// truth from the next, so no client-side accumulator can drift.
#[inline]
fn announce(sc: &SurvivalContent, p: &Player, events: &mut EventQueue) {
    events.push(
        EV_VITALS,
        p.id,
        ((p.food as u32) << 16) | p.water as u32,
        ((sc.max_food as u32) << 16) | sc.max_water as u32,
    );
    events.push(EV_HEALTH, p.id, p.hp as u32, p.hp_max as u32);
}

/// The same announcement, from outside — the join door and the respawn use
/// it so a fresh body states its meters without waiting for the first tick
/// that happens to move one.
#[inline]
pub fn announce_vitals(sc: &SurvivalContent, p: &Player, events: &mut EventQueue) {
    if !sc.armed() {
        return;
    }
    events.push(
        EV_VITALS,
        p.id,
        ((p.food as u32) << 16) | p.water as u32,
        ((sc.max_food as u32) << 16) | sc.max_water as u32,
    );
}

/// Eat what is in hotbar slot `slot`. Consumes exactly one unit, applies
/// food and water immediately and starts the health ramp.
///
/// The slot is the *sender's claim*; this function is the sim's verdict, so
/// a forged slot index, an empty hand and a stack of wood all land on the
/// same refusal path rather than on an inventory write.
pub fn consume(sc: &SurvivalContent, slot: usize, p: &mut Player, events: &mut EventQueue) -> bool {
    if !sc.armed() || slot >= p.inv.len() {
        events.push(EV_CONSUME_REFUSED, p.id, REFUSE_C_NOT_FOOD, 0);
        return false;
    }
    let stack = p.inv[slot];
    if stack.count == 0 {
        events.push(EV_CONSUME_REFUSED, p.id, REFUSE_C_NOT_FOOD, 0);
        return false;
    }
    let Some(def) = sc.row(stack.item) else {
        events.push(EV_CONSUME_REFUSED, p.id, REFUSE_C_NOT_FOOD, 0);
        return false;
    };
    // Refuse a consume that could not do anything: a full pair and no heal
    // would destroy the item for nothing. A row that heals is always
    // allowed — hp is the one meter a player may top up deliberately.
    let would_feed =
        (def.food > 0 && p.food < sc.max_food) || (def.water > 0 && p.water < sc.max_water);
    if def.health == 0 && !would_feed {
        events.push(EV_CONSUME_REFUSED, p.id, REFUSE_C_FULL, 0);
        return false;
    }

    p.inv[slot].count -= 1;
    if p.inv[slot].count == 0 {
        p.inv[slot].item = NO_ITEM;
    }
    p.food = (p.food + def.food).min(sc.max_food);
    p.water = (p.water + def.water).min(sc.max_water);
    if def.health > 0 && def.seconds > 0 {
        // A second consume replaces the ramp rather than queueing behind
        // it: two bandages at once is one bandage's worth of book-keeping,
        // and the remainder of the first is folded into the second so the
        // hp is not lost.
        let carried = p.heal_rem;
        p.heal_rem = def.health.saturating_add(carried);
        p.heal_total = p.heal_rem;
        p.heal_span = def.seconds as u32 * TICK_HZ;
        p.heal_acc = 0;
    }
    events.push(
        EV_CONSUMED,
        p.id,
        ((stack.item as u32) << 16) | slot as u32,
        0,
    );
    announce(sc, p, events);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gather::ItemStack;
    use crate::world::EventQueue;

    fn player(sc: &SurvivalContent) -> Player {
        let mut p = Player {
            id: 7,
            active: true,
            hp: 100,
            hp_max: 100,
            ..Player::default()
        };
        grant(sc, &mut p);
        p
    }

    /// The span is exact: a meter reaches zero on the tick the content
    /// names, not one before and not one after. This is the assertion the
    /// rational accumulator exists for — integer truncation would drift.
    #[test]
    fn span_is_exact() {
        let sc = SurvivalContent::probe_fixture();
        let mut p = player(&sc);
        let mut q = EventQueue::default();
        // Water: 8 s at 30 Hz = 240 ticks for 100 units.
        for _ in 0..(sc.water_span_ticks - 1) {
            step(&sc, &mut p, &mut q);
        }
        assert_eq!(p.water, 1, "one unit left on the last tick of the span");
        step(&sc, &mut p, &mut q);
        assert_eq!(p.water, 0, "empty exactly on the span's tick");
    }

    /// Both meters empty stack, and the sum is what kills.
    #[test]
    fn empty_meters_stack_and_kill() {
        let sc = SurvivalContent::probe_fixture();
        let mut p = player(&sc);
        let mut q = EventQueue::default();
        let mut died = false;
        for _ in 0..(60 * TICK_HZ) {
            if step(&sc, &mut p, &mut q) == Step::Died {
                died = true;
                break;
            }
        }
        assert!(
            died,
            "an untended body dies inside a minute at fixture rates"
        );
        assert_eq!(p.hp, 0);
    }

    /// A full meter pair with no heal refuses rather than eating the item.
    #[test]
    fn consume_refuses_when_it_would_do_nothing() {
        let mut sc = SurvivalContent::probe_fixture();
        sc.consumable[0].health = 0;
        let mut p = player(&sc);
        p.inv[0] = ItemStack { item: 0, count: 2 };
        let mut q = EventQueue::default();
        assert!(!consume(&sc, 0, &mut p, &mut q), "full meters, no heal");
        assert_eq!(p.inv[0].count, 2, "the item is not destroyed");
    }

    /// Eating restores the pair and delivers health over `seconds` — not
    /// instantly. The ramp is the reason `seconds` is read at all.
    #[test]
    fn heal_arrives_over_seconds() {
        let sc = SurvivalContent::probe_fixture();
        let mut p = player(&sc);
        p.hp = 50;
        p.food = 10;
        p.water = 10;
        p.inv[0] = ItemStack { item: 0, count: 1 };
        let mut q = EventQueue::default();
        assert!(consume(&sc, 0, &mut p, &mut q));
        assert_eq!(p.food, 50, "food applied immediately");
        assert_eq!(p.water, 40, "water applied immediately");
        assert_eq!(p.hp, 50, "health has not arrived yet");
        let span = sc.consumable[0].seconds as u32 * TICK_HZ;
        for _ in 0..span {
            step(&sc, &mut p, &mut q);
        }
        assert_eq!(p.heal_rem, 0, "the whole ramp paid out inside its span");
        // 20 hp of heal against the drain the same window costs.
        assert!(p.hp > 50, "the ramp raised hp");
    }

    /// Inert content plays the game that existed before this module.
    #[test]
    fn empty_content_is_a_no_op() {
        let sc = SurvivalContent::EMPTY;
        let mut p = player(&sc);
        p.hp = 100;
        let mut q = EventQueue::default();
        for _ in 0..(10 * TICK_HZ) {
            assert_eq!(step(&sc, &mut p, &mut q), Step::Quiet);
        }
        assert_eq!(p.hp, 100);
        assert_eq!(p.food, 0);
    }

    /// Refilling a meter does not bank the partial minute of damage.
    #[test]
    fn refill_clears_the_partial_minute() {
        let sc = SurvivalContent::probe_fixture();
        let mut p = player(&sc);
        let mut q = EventQueue::default();
        p.food = 0;
        p.water = 0;
        step(&sc, &mut p, &mut q);
        assert!(p.hurt_acc > 0, "an empty pair accumulates damage");
        p.food = 50;
        p.water = 50;
        step(&sc, &mut p, &mut q);
        assert_eq!(p.hurt_acc, 0, "the partial minute is forgiven, not banked");
    }
}
