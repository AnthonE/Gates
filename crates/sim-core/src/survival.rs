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
//! **v15 gave thirst its real answer.** Until it, water came only from
//! berries — 10 minutes of a 40-minute meter per bush — while a player
//! spawned *beside an ocean* and could not touch it, which is a survival
//! loop with one input and therefore not a loop. `drink` is the verb, and
//! the sea is salt: it restores water and costs hit points, so the ocean
//! is the answer that is always there and never free, and the bush stays
//! worth walking to. It is also the first verb here that reads the world
//! rather than the inventory — `terrain::height` under the body's own
//! feet, the same pure function the client draws the ground from.
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
    EventQueue, Player, EV_CONSUMED, EV_CONSUME_REFUSED, EV_DRANK, EV_HEALTH, EV_VITALS,
};

/// How far a mouthful reaches, planar. Not a knob of its own: it is
/// `build::BUILD_REACH_M`, the same distance a hearth feed, a door use, a
/// lock and a bag loot all measure — "what you can touch from where you
/// stand" is one number in this sim, and a second one would be a second
/// thing to keep honest.
pub use crate::build::BUILD_REACH_M as DRINK_REACH_M;

/// Ticks in a minute — the denominator every per-minute rate in this
/// module divides by. Derived from `limits::TICK_HZ`, never restated.
pub const TICKS_PER_MIN: u32 = TICK_HZ * 60;

/// One item's consumable row, baked from `content/consumables.toml`. An
/// all-zero row means the item is not food: the whole table starts that
/// way, so eating a stack of wood is the same non-event as swinging it at
/// a person with no melee row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    /// Per mille of the Verdigris meter (`Player::fel`) the item moves,
    /// immediately: negative draws the corruption out (the Charcoal Tonic,
    /// Rust's anti-rad pills).
    pub fel: i16,
    /// Hp of bleeding it closes, as a negative (Rust's bandage −50).
    pub bleed: i16,
    /// Ticks in the hands before it lands (Rust's `MedicalTool`, the
    /// bandage's wrap); zero is instant. See [`begin_use`].
    pub use_ticks: u8,
    /// What eating one leaves in the eater's hand (`NO_ITEM` for nothing),
    /// `leaves_pm` per mille of the time: a cob leaves a corn seed, a
    /// handful of berries now and then a berry seed (`World`'s consume arm).
    pub leaves: u16,
    pub leaves_pm: u16,
    /// A tea (`exposure::Brew`, clothing v1): `brew` is its kind plus one
    /// (0: not a tea), what it gives, its floor, and how many seconds it
    /// works for.
    pub brew: u8,
    pub brew_amount: u16,
    pub brew_floor_dc: i16,
    pub brew_s: u16,
}

impl Default for ConsumableDef {
    fn default() -> Self {
        Self::NONE
    }
}

impl ConsumableDef {
    /// The row of an item that is not food.
    pub const NONE: Self = Self {
        health: 0,
        food: 0,
        water: 0,
        seconds: 0,
        fel: 0,
        bleed: 0,
        use_ticks: 0,
        leaves: NO_ITEM,
        leaves_pm: 0,
        brew: 0,
        brew_amount: 0,
        brew_floor_dc: crate::exposure::NO_FLOOR,
        brew_s: 0,
    };

    /// Does this row do anything at all? The bake refuses a row that does
    /// not, so this is the sim's read of "is the held item food".
    #[inline]
    pub fn is_food(&self) -> bool {
        self.health > 0
            || self.food > 0
            || self.water > 0
            || self.fel != 0
            || self.bleed != 0
            || self.brew > 0
    }
}

/// How many water vessels content may author (a skin, a jug, ...).
pub const MAX_VESSELS: usize = 4;

/// One water vessel: the item empty, full of fresh water, and full of salt
/// (`NO_ITEM` for a vessel that is never filled with it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Vessel {
    pub empty: u16,
    pub fresh: u16,
    pub salt: u16,
}

impl Vessel {
    pub const NONE: Self = Self {
        empty: NO_ITEM,
        fresh: NO_ITEM,
        salt: NO_ITEM,
    };
}

/// The whole survival ruleset the sim knows. Construction input like the
/// seed and the gather table; the WAL pins the content hash it was baked
/// from (CONTENT.md §0).
#[derive(Clone, Copy, Debug)]
pub struct SurvivalContent {
    /// Indexed by item index (the sorted-rank mapping `bake` owns).
    pub consumable: [ConsumableDef; MAX_ITEM_DEFS],
    /// Items that save a failed wounded roll when carried on the belt.
    /// Content opts in; the sim never identifies a medkit by name or index.
    pub belt_recovery: [bool; MAX_ITEM_DEFS],
    /// Items that stand a downed body up when used on it (`Command::Treat`,
    /// the syringe). Content opts in, as with `belt_recovery`.
    pub revive: [bool; MAX_ITEM_DEFS],
    /// Meter ceilings, and what a join grants. Zero is the inert default
    /// and disarms the module: no meter, no drain, no starvation.
    pub max_food: u16,
    pub max_water: u16,
    /// What a fresh body starts with, Rust-style (a spawn is hungry,
    /// thirsty and hurt). Zero is full.
    pub spawn_hp: u16,
    pub spawn_food: u16,
    pub spawn_water: u16,
    /// Ticks from a full meter to an empty one. The bake derives these
    /// from `[survival]`'s minutes so the sim never multiplies a clock.
    pub food_span_ticks: u32,
    pub water_span_ticks: u32,
    /// Hit points a minute while the matching meter reads zero. Both empty
    /// stack — the rates add before the accumulator sees them.
    pub starve_hp_per_min: u16,
    pub dehydrate_hp_per_min: u16,
    /// Water units one mouthful of the sea puts back, and the hit points
    /// it costs to swallow it. **The sea is salt** — that is the whole
    /// design of the verb: the ocean is always there and always answers,
    /// and it grinds you down for it, so berries stay worth walking to.
    /// A zero `drink_water` disarms the verb without disarming the module
    /// (the refusal path still announces), which is how content that
    /// authors no drink still boots.
    pub drink_water: u16,
    pub drink_hp_cost: u16,
    /// A mouthful of the SEA, water units (water pass): less than a lake's
    /// `drink_water`, and it still costs `drink_hp_cost`. Zero refuses salt
    /// water outright (`REFUSE_C_SALT`), the reference's own rule since 2016.
    pub drink_water_salt: u16,
    /// The water vessels (water pass): what each holds empty, fresh and
    /// salt. Holding an empty one at water, `drink` fills it instead; a full
    /// fresh one is a consumable row whose drinking hands the empty one
    /// back; a salt one is boiled fresh at a fire (`cooking.toml`); either
    /// pours (`World::pour`). The first `vessel_count` rows are live.
    pub vessels: [Vessel; MAX_VESSELS],
    pub vessel_count: u8,
    /// Wet and cold (weather v0, `exposure.rs`): the other half of the body
    /// the clock keeps, baked from `[exposure]`. `EMPTY` disarms it.
    pub exposure: crate::exposure::ExposureContent,
    /// Hp a minute an open wound costs (`Player::bleed`, Rust's 1 hp every
    /// 3 s). The hit that opens it is `combat::hurt`'s.
    pub bleed_hp_per_min: u16,
}

impl SurvivalContent {
    pub const EMPTY: Self = Self {
        consumable: [ConsumableDef::NONE; MAX_ITEM_DEFS],
        belt_recovery: [false; MAX_ITEM_DEFS],
        revive: [false; MAX_ITEM_DEFS],
        max_food: 0,
        max_water: 0,
        spawn_hp: 0,
        spawn_food: 0,
        spawn_water: 0,
        food_span_ticks: 0,
        water_span_ticks: 0,
        starve_hp_per_min: 0,
        dehydrate_hp_per_min: 0,
        drink_water: 0,
        drink_hp_cost: 0,
        drink_water_salt: 0,
        vessels: [Vessel::NONE; MAX_VESSELS],
        vessel_count: 0,
        exposure: crate::exposure::ExposureContent::EMPTY,
        bleed_hp_per_min: 0,
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
        // A quarter of the meter for a fifth of the body: unlike the game
        // content, the fixture's drink is meant to be *lethal* inside a
        // counted window, so the probes exercise the salt death and not
        // only the happy path.
        c.drink_water = 25;
        c.drink_hp_cost = 20;
        c.drink_water_salt = 25;
        c.consumable[0] = ConsumableDef {
            health: 20,
            food: 40,
            water: 30,
            seconds: 2,
            fel: 0,
            ..ConsumableDef::NONE
        };
        c
    }

    /// The live vessel rows.
    #[inline]
    pub fn vessels(&self) -> &[Vessel] {
        &self.vessels[..(self.vessel_count as usize).min(MAX_VESSELS)]
    }

    /// The vessel `item` is the empty one of.
    #[inline]
    pub fn vessel_empty(&self, item: u16) -> Option<Vessel> {
        if item == NO_ITEM {
            return None;
        }
        self.vessels().iter().copied().find(|v| v.empty == item)
    }

    /// The vessel `item` is a full one of, and whether what it holds is salt.
    #[inline]
    pub fn vessel_full(&self, item: u16) -> Option<(Vessel, bool)> {
        if item == NO_ITEM {
            return None;
        }
        self.vessels().iter().find_map(|v| {
            if v.fresh == item {
                Some((*v, false))
            } else if v.salt == item {
                Some((*v, true))
            } else {
                None
            }
        })
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

    /// Does `item` revive a downed body it is used on?
    #[inline]
    pub fn revives(&self, item: u16) -> bool {
        item != NO_ITEM && self.revive.get(item as usize).copied().unwrap_or(false)
    }

    /// First nonempty recovery stack on the belt, in inventory order.
    /// The selected hand does not matter; the backpack never qualifies.
    pub fn belt_recovery_slot(&self, p: &Player) -> Option<usize> {
        p.inv[..crate::limits::HOTBAR_SLOTS].iter().position(|s| {
            s.count > 0
                && self
                    .belt_recovery
                    .get(s.item as usize)
                    .copied()
                    .unwrap_or(false)
        })
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
/// nothing). Eating here would destroy the item for no effect. A drink
/// with a full water meter lands here too — same fact, same line.
pub const REFUSE_C_FULL: u32 = 2;
/// There is no water within reach of the feet, so the drink was a press
/// at dry ground. Announced rather than swallowed for `REFUSE_C_NOT_FOOD`'s
/// reason: the player needs to learn where the verb works.
pub const REFUSE_C_NO_WATER: u32 = 3;
/// Salt water (water pass): the content refuses the sea to the mouth
/// (`drink_water_salt` zero), or the vessel in hand holds salt — boil it.
pub const REFUSE_C_SALT: u32 = 4;
/// The highest reason above — `DEATH_BY_MAX`'s posture for this ledger.
/// The wire field is four bits, so a forged `reason` in 4..=15 fits the
/// width (zero the encoder already refuses); the server refuses the whole
/// out-of-domain range against this constant at the encode boundary
/// (`server/core.rs`, NOW.md §5b). A **literal**, unlike `DEATH_BY_MAX`,
/// because `protocol`'s domain scrape reads this block as text and its
/// exempt list is a wire-pass edit; `tests/domain_ledger.rs` fails if the
/// literal stops naming the ledger's top.
pub const REFUSE_C_MAX: u32 = 4;

/// The deepest a wound gets, in hp it owes: Rust's `bleeding` meter tops
/// out at 100.
pub const BLEED_MAX: u16 = 100;

/// One rational step: add `num` to `acc`, take out whole units against
/// `den`, keep the remainder. The one piece of arithmetic every meter in
/// this module shares, so an exactness bug can only exist in one place.
///
/// `den == 0` yields zero units and leaves `acc` alone — the inert-content
/// path, and the reason no caller needs its own divide-by-zero guard.
///
/// `pub(crate)` since torch fuel v0, and for the sentence above rather
/// than for convenience: `light::step` is a fourth meter over the same
/// rational step, and a second copy of these five lines would be a second
/// place an exactness bug could live.
#[inline]
pub(crate) fn tick_units(acc: &mut u32, num: u32, den: u32) -> u32 {
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
    /// An open wound took the last hp (`Player::bleed`): `Died`, with its
    /// own sentence on the death screen.
    BledOut,
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
            // The funnel, **unreduced**: a clock is not a hit, so armor
            // must never blunt it. `crate::combat::hurt_unreduced`'s doc
            // carries the three reasons; the name is the choice.
            died = crate::combat::hurt_unreduced(p, dmg).died;
        }
    }

    // 2b · Bleed. An open wound costs its rate until it has taken what it
    // owes (Rust: 1 hp every 3 s); each hp it takes is one hp less owed.
    let mut bled = false;
    if p.bleed > 0 && !died {
        let dmg = tick_units(&mut p.bleed_acc, sc.bleed_hp_per_min as u32, TICKS_PER_MIN);
        if dmg > 0 {
            let dmg = dmg.min(u32::from(p.bleed)) as u16;
            p.bleed -= dmg;
            // Unreduced, the clock's reason: the wound already got
            // through the armor once, when it was opened.
            bled = crate::combat::hurt_unreduced(p, dmg).died;
        }
        if p.bleed == 0 {
            p.bleed_acc = 0;
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
            if !died && !bled {
                let max = max_hp(p);
                p.hp = (p.hp + give).min(max);
            }
        }
        if p.heal_rem == 0 {
            clear_heal(p);
        }
    }

    if died {
        return Step::Died;
    }
    if bled {
        return Step::BledOut;
    }
    if (p.food, p.water, p.hp) != before {
        announce(sc, p, events);
        return Step::Changed;
    }
    Step::Quiet
}

// The clock's deaths are announced by `World::die`, with every other
// death's (wounded v0) — this module reports `Step::Died` and nothing
// else. It used to push `EV_DEATH` here through `died_by_the_world`,
// which was right while a lethal debit and a corpse were the same event.

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

/// Grant a fresh body its meters and its starting hp. Called at join and
/// at respawn, after `hp_max` is set, so a player who starved does not
/// respawn already starving.
#[inline]
pub fn grant(sc: &SurvivalContent, p: &mut Player) {
    let start = |spawn: u16, max: u16| if spawn == 0 { max } else { spawn.min(max) };
    p.food = start(sc.spawn_food, sc.max_food);
    p.water = start(sc.spawn_water, sc.max_water);
    p.hp = start(sc.spawn_hp, p.hp_max);
    p.food_acc = 0;
    p.water_acc = 0;
    p.hurt_acc = 0;
    p.bleed = 0;
    p.bleed_acc = 0;
    clear_heal(p);
    clear_use(p);
    crate::exposure::reset(p);
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

/// Pour `def`'s tea into `p`, replacing any of the same kind (Rust's: one
/// tea of a kind at a time, the last one drunk).
pub fn brew(def: ConsumableDef, p: &mut Player) {
    if def.brew == 0 || def.brew_s == 0 {
        return;
    }
    if let Some(b) = p.brew.get_mut(def.brew as usize - 1) {
        *b = crate::exposure::Brew {
            amount: def.brew_amount,
            floor_dc: def.brew_floor_dc,
            left_s: def.brew_s,
        };
    }
}

/// Start `def`'s heal ramp on `p` — the half of eating that does not spend
/// anything, shared by `consume` and a revive (`World::treat`).
pub fn start_heal(def: ConsumableDef, p: &mut Player, heal_pct: u32) {
    if def.health > 0 && def.seconds > 0 {
        // A second consume replaces the ramp rather than queueing behind
        // it: two bandages at once is one bandage's worth of book-keeping,
        // and the remainder of the first is folded into the second so the
        // hp is not lost. **The span grows with the pool, so the RATE never
        // exceeds this item's own**: folding the remainder in over one
        // item's span made N items at once heal N times as fast — thirty
        // bandages in a second was a full heal in one bandage's time
        // (found 2026-09-25). Integer and rounded up, so never faster.
        let carried = p.heal_rem;
        p.heal_rem = def.health.saturating_add(carried);
        p.heal_total = p.heal_rem;
        // The Wells' ceiling (`works::KNOB_HEAL_PCT`) shortens the span, so
        // the same heal lands sooner; 100 is the item's own pace.
        let span = (def.seconds as u64 * TICK_HZ as u64 * 100 / heal_pct.max(1) as u64).max(1);
        p.heal_span =
            ((p.heal_rem as u64 * span).div_ceil(def.health as u64)).min(u32::MAX as u64) as u32;
        p.heal_acc = 0;
    }
}

/// Eat what is in hotbar slot `slot`. Consumes exactly one unit, applies
/// food and water immediately and starts the health ramp.
///
/// The slot is the *sender's claim*; this function is the sim's verdict, so
/// a forged slot index, an empty hand and a stack of wood all land on the
/// same refusal path rather than on an inventory write.
pub fn consume(
    sc: &SurvivalContent,
    slot: usize,
    p: &mut Player,
    events: &mut EventQueue,
    heal_pct: u32,
) -> bool {
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
        // A vessel of sea water is not drunk: it is boiled first.
        let why = if sc.vessel_full(stack.item).is_some_and(|(_, salt)| salt) {
            REFUSE_C_SALT
        } else {
            REFUSE_C_NOT_FOOD
        };
        events.push(EV_CONSUME_REFUSED, p.id, why, 0);
        return false;
    };
    // Refuse a consume that could not do anything: a full pair and no heal
    // would destroy the item for nothing. A row that heals is always
    // allowed — hp is the one meter a player may top up deliberately.
    let would_feed = (def.food > 0 && p.food < sc.max_food)
        || (def.water > 0 && p.water < sc.max_water)
        || (def.bleed < 0 && p.bleed > 0)
        || (def.fel < 0 && p.fel > 0)
        || def.fel > 0
        || def.brew > 0;
    if def.health == 0 && !would_feed {
        events.push(EV_CONSUME_REFUSED, p.id, REFUSE_C_FULL, 0);
        return false;
    }

    p.inv[slot].count -= 1;
    let drunk = sc
        .vessel_full(stack.item)
        .filter(|(v, _)| v.fresh == stack.item);
    if let (0, Some((v, _))) = (p.inv[slot].count, drunk) {
        // A drunk vessel is an empty one, in the same slot.
        p.inv[slot] = crate::gather::ItemStack {
            item: v.empty,
            count: 1,
            cond: 0,
            skin: stack.skin,
        };
    } else if p.inv[slot].count == 0 {
        // The canonical empty, all three fields — NOT `NO_ITEM`, which
        // `PlayerSave::read_le` refuses twice over (`item >= MAX_ITEM_DEFS`
        // and the canonical-empty rule): eat your last stack, get swept by
        // the autosave, and the record could never be read back. Every
        // reader of an inventory slot keys on `count`, never on the stored
        // item of an empty slot (`held_item` derives its own `NO_ITEM`).
        p.inv[slot] = crate::gather::ItemStack::default();
    }
    // Saturating, not wrapping: `min` clamps the *result*, so a plain add
    // would have to be correct before the clamp could help it. The bake
    // checks each side fits u16 and never their sum, and content is
    // operator-authored — but "unreachable through today's validator" is
    // not the same as "cannot happen", and this is one word.
    p.food = p.food.saturating_add(def.food).min(sc.max_food);
    p.water = p.water.saturating_add(def.water).min(sc.max_water);
    p.fel = (i32::from(p.fel) + i32::from(def.fel)).clamp(0, 1000) as u16;
    brew(def, p);
    p.bleed = p.bleed.saturating_sub(def.bleed.unsigned_abs());
    if p.bleed == 0 {
        p.bleed_acc = 0;
    }
    start_heal(def, p, heal_pct);
    events.push(
        EV_CONSUMED,
        p.id,
        ((stack.item as u32) << 16) | slot as u32,
        0,
    );
    announce(sc, p, events);
    true
}

/// Start a use that takes time in the hands (Rust's `MedicalTool`, the
/// bandage's wrap): nothing is spent until it lands in [`tick_use`], and a
/// hand switched away before then cancels it. `false` when the row in
/// `slot` is instant (or not a row at all), so the caller eats it now and
/// [`consume`] gives any refusal. A second press mid-use does nothing.
pub fn begin_use(sc: &SurvivalContent, slot: usize, p: &mut Player, tick: u64) -> bool {
    let Some(stack) = p.inv.get(slot).copied().filter(|s| s.count > 0) else {
        return false;
    };
    if !sc.armed() || !sc.row(stack.item).is_some_and(|d| d.use_ticks > 0) {
        return false;
    }
    if p.use_at == 0 {
        p.use_at = tick + u64::from(sc.consumable[stack.item as usize].use_ticks);
        p.use_slot = slot as u8;
        p.use_sel = p.frame.sel;
        p.use_item = stack.item;
    }
    true
}

/// Land the use in the hands on its tick, or cancel it the tick the hand
/// that began it is put away or the item leaves its slot. A cancel spends
/// nothing and says nothing: the player is the one who switched.
pub fn tick_use(
    sc: &SurvivalContent,
    p: &mut Player,
    events: &mut EventQueue,
    tick: u64,
    heal_pct: u32,
) {
    if p.use_at == 0 {
        return;
    }
    let slot = p.use_slot as usize;
    let s = p.inv.get(slot).copied().unwrap_or_default();
    if p.frame.sel != p.use_sel || s.count == 0 || s.item != p.use_item {
        clear_use(p);
    } else if tick >= p.use_at {
        clear_use(p);
        consume(sc, slot, p, events, heal_pct);
    }
}

/// Whether a use in the hands holds them: begun, and the hand that began
/// it still up (put away, `tick_use` cancels it). Rust's bandage is the
/// held item, but a `Consume` may name any slot and the hand-held item
/// fired on through the wrap, so the wrap holds the hand instead: nothing
/// in it swings, draws, fires or throws until the wrap lands (`World::tick`'s
/// arm, `ranged::hitscan`, `Command::Throw`).
#[inline]
pub fn hands_busy(p: &Player) -> bool {
    p.use_at != 0 && p.frame.sel == p.use_sel
}

/// Drop any use in the hands. One place, `clear_heal`'s reason.
#[inline]
pub fn clear_use(p: &mut Player) {
    p.use_at = 0;
    p.use_slot = 0;
    p.use_sel = 0;
    p.use_item = NO_ITEM;
}

/// Is there water within `DRINK_REACH_M` of this body, planar? Five taps
/// on the heightfield — the feet and the four cardinal points at the reach
/// — and any one of them below `SEA_LEVEL` is water you can put a hand in.
///
/// **Five and not a ring**, because a ring costs trig and this module has
/// none available (wall 1: no libm, no trig). Five is also enough for the
/// shape being tested: the sea is a half-space with a coastline that wanders
/// tens of metres per cycle, so a body inside 5 m of it hits the sample on
/// its seaward side long before the coastline can thread between two of
/// them. What it deliberately cannot do is find a pond smaller than the gap
/// between taps. The water in this world is the ocean and the lakes
/// (`terrain` stage 4e); a pond that small is missed, and that is accepted.
///
/// Pure, bounded, allocation-free and float-restricted to the ops wall 1
/// allows: it is five `terrain::height` calls and five compares, the same
/// function the client draws the ground from and the same one the spawn
/// ring picks a beach with, so "am I at water" has exactly one answer on
/// both sides of the wire.
///
/// The answer is `None` for no water, else whether it is **salt**: a tap in
/// water the lake carve dug (`terrain::height_open`'s share under a half)
/// is fresh, and fresh wins — a body at a lagoon's edge drinks the lake
/// side. The same call as the height, so the lakes cost nothing extra.
#[inline]
fn water_in_reach(seed: u64, x: f32, z: f32) -> Option<bool> {
    let r = DRINK_REACH_M;
    let taps = [(0.0, 0.0), (r, 0.0), (-r, 0.0), (0.0, r), (0.0, -r)];
    let mut wet = false;
    let mut fresh = false;
    for (dx, dz) in taps {
        // No early return: the loop is the same work every call, which is
        // what keeps this verb off the tick-jitter budget's variable side.
        let (h, open) = crate::terrain::height_open(seed, x + dx, z + dz);
        let here = h < crate::terrain::SEA_LEVEL;
        wet |= here;
        fresh |= here && open < 0.5;
    }
    wet.then_some(!fresh)
}

/// What an `EV_DRANK` reports (water pass): a mouthful (`b`'s low half the
/// water it restored, `c` the hp it cost), a vessel filled (`b`'s low half
/// 1 for salt), or one poured out (`World::pour`: `b`'s low half the fires
/// it put out, `c` the planters it reached, with [`POURED_SALT`] set when
/// it was salt). The kind rides `b`'s high half — [`drank_b`].
pub const DRANK_DRINK: u16 = 0;
pub const DRANK_FILL: u16 = 1;
pub const DRANK_POUR: u16 = 2;
/// A pour's `c` flag: the water was salt, so the planters it reached went dry.
pub const POURED_SALT: u32 = 1 << 16;

/// `EV_DRANK`'s `b`: the kind over the count.
#[inline]
pub fn drank_b(kind: u16, n: u16) -> u32 {
    (u32::from(kind) << 16) | u32::from(n)
}

/// Drink from the water at your feet — thirst's real answer, and the first
/// verb in this module that reads the world instead of the inventory.
///
/// The sea is salt: a mouthful restores `drink_water` and costs
/// `drink_hp_cost`, so the ocean is the answer that is always there and
/// never free. That is what keeps the bush worth walking to after this
/// lands, and it is the reason this verb can kill — a body drinking on its
/// last points of health dies of it, and dies through the module's one kill
/// site, exactly as a starved body does. **A lake is fresh** and costs
/// nothing, as the reference's lakes and rivers do: it is the inland water
/// worth walking to.
///
/// Every input is the sim's own: the position comes off the body, the water
/// off the heightfield, the numbers off baked content. Nothing here crosses
/// the wire, so there is nothing here to forge (`ActionMsg::Drink`).
pub fn drink(sc: &SurvivalContent, seed: u64, p: &mut Player, events: &mut EventQueue) -> Step {
    if !sc.armed() || sc.drink_water == 0 {
        // Content that authors no drink refuses the press rather than
        // swallowing it — the same posture as an eat with no food row.
        events.push(EV_CONSUME_REFUSED, p.id, REFUSE_C_NO_WATER, 0);
        return Step::Quiet;
    }
    let x = p.body.qx as f32 * crate::movement::POS_XZ_Q;
    let z = p.body.qz as f32 * crate::movement::POS_XZ_Q;
    let Some(salt) = water_in_reach(seed, x, z) else {
        events.push(EV_CONSUME_REFUSED, p.id, REFUSE_C_NO_WATER, 0);
        return Step::Quiet;
    };
    // An empty vessel in hand fills instead of drinking: with fresh water
    // where there is any, else with the sea when the vessel holds salt.
    let sel = p.frame.sel as usize;
    let held = p.inv.get(sel).copied().unwrap_or_default();
    if let Some(v) = sc.vessel_empty(held.item).filter(|_| held.count == 1) {
        let into = if salt { v.salt } else { v.fresh };
        if into != NO_ITEM {
            p.inv[sel].item = into;
            events.push(EV_DRANK, p.id, drank_b(DRANK_FILL, salt as u16), 0);
            return Step::Changed;
        }
    }
    let gulp = if salt {
        sc.drink_water_salt
    } else {
        sc.drink_water
    };
    if gulp == 0 {
        // The content keeps the sea from the mouth, the reference's rule.
        events.push(EV_CONSUME_REFUSED, p.id, REFUSE_C_SALT, 0);
        return Step::Quiet;
    }
    if p.water >= sc.max_water {
        // Refuse rather than charge: a full meter would pay hp for
        // nothing, which is `REFUSE_C_FULL`'s exact case on the eat side.
        events.push(EV_CONSUME_REFUSED, p.id, REFUSE_C_FULL, 0);
        return Step::Quiet;
    }

    // The meter first, the cost second, so a drink that kills you still
    // delivers the water — the ordering `step` uses for the same reason
    // (drain, then hurt: the tick a thing happens is the tick it counts).
    let before = p.water;
    p.water = p.water.saturating_add(gulp).min(sc.max_water);
    let restored = p.water - before;
    // The funnel, **unreduced**: the hp *is* the price of the drink, and a
    // helmet is not a desalinator. `dealt` rather than the raw cost, so
    // `EV_DRANK` reports what the body actually paid.
    let took = crate::combat::hurt_unreduced(p, if salt { sc.drink_hp_cost } else { 0 });
    events.push(
        EV_DRANK,
        p.id,
        drank_b(DRANK_DRINK, restored),
        took.dealt as u32,
    );
    announce(sc, p, events);
    // The "a body already at zero is not killed twice" guard this line
    // used to spell out by hand now lives in `combat::debit`, which is the
    // one place that can state it once for every route.
    if took.died {
        return Step::Died;
    }
    Step::Changed
}

/// A hard landing (`World::fall`): `dmg` hp, unreduced — a plate does not
/// break a fall — and with no bearing, like the cold. `Died` when it was
/// lethal; the caller downs the body.
pub fn fall(p: &mut Player, dmg: u16, events: &mut EventQueue) -> Step {
    if dmg == 0 || p.hp == 0 {
        return Step::Quiet;
    }
    if crate::combat::hurt_unreduced(p, dmg).died {
        return Step::Died;
    }
    events.push(EV_HEALTH, p.id, p.hp as u32, p.hp_max as u32);
    Step::Changed
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
        p.inv[0] = ItemStack {
            item: 0,
            count: 2,
            cond: 0,
            skin: 0,
        };
        let mut q = EventQueue::default();
        assert!(
            !consume(&sc, 0, &mut p, &mut q, 100),
            "full meters, no heal"
        );
        assert_eq!(p.inv[0].count, 2, "the item is not destroyed");
    }

    /// Ten heals eaten in ten ticks heal no faster than one: the pool grows
    /// and so does the span. Red on the old fold, which kept one item's span.
    #[test]
    fn stacked_heals_do_not_heal_faster() {
        let sc = SurvivalContent::probe_fixture();
        let mut p = player(&sc);
        p.hp = 1;
        p.food = 0;
        p.water = 0;
        p.inv[0] = ItemStack {
            item: 0,
            count: 10,
            cond: 0,
            skin: 0,
        };
        let mut q = EventQueue::default();
        let span = sc.consumable[0].seconds as u32 * TICK_HZ;
        let per_item = sc.consumable[0].health as u32;
        let start = p.hp as u32;
        for _ in 0..10 {
            consume(&sc, 0, &mut p, &mut q, 100);
            step(&sc, &mut p, &mut q);
        }
        for _ in 10..span {
            step(&sc, &mut p, &mut q);
        }
        let healed = p.hp as u32 - start;
        assert!(
            healed <= per_item + 1,
            "{healed} hp inside one item's span from stacked heals (one item is {per_item})"
        );
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
        p.inv[0] = ItemStack {
            item: 0,
            count: 1,
            cond: 0,
            skin: 0,
        };
        let mut q = EventQueue::default();
        assert!(consume(&sc, 0, &mut p, &mut q, 100));
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

    /// A row that would overflow the meter's own type still lands on the
    /// ceiling. Today's validator cannot produce this content; the add is
    /// saturating anyway, because the clamp is on the result and a wrapped
    /// sum is already wrong by the time it reaches one.
    /// The tonic: it draws the Verdigris out at full food and water (it
    /// heals a little, so it is never refused), and never below zero. A row
    /// that only draws refuses on a clean body.
    #[test]
    fn a_tonic_draws_the_green_out() {
        let mut sc = SurvivalContent::probe_fixture();
        sc.consumable[0] = ConsumableDef {
            health: 5,
            food: 0,
            water: 0,
            seconds: 1,
            fel: -400,
            ..ConsumableDef::NONE
        };
        let drink = |sc: &SurvivalContent, fel: u16| {
            let mut p = player(sc);
            p.food = sc.max_food;
            p.water = sc.max_water;
            p.fel = fel;
            p.inv[0] = ItemStack {
                item: 0,
                count: 1,
                cond: 0,
                skin: 0,
            };
            let mut ev = EventQueue::default();
            let ok = consume(sc, 0, &mut p, &mut ev, 100);
            (ok, p.fel)
        };
        assert_eq!(drink(&sc, 900), (true, 500));
        assert_eq!(drink(&sc, 300), (true, 0), "never below clean");
        assert_eq!(drink(&sc, 0), (true, 0), "the heal still takes it");
        sc.consumable[0].health = 0;
        sc.consumable[0].seconds = 0;
        assert_eq!(drink(&sc, 0), (false, 0), "nothing to draw out");
        assert_eq!(drink(&sc, 200), (true, 0));
    }

    #[test]
    fn an_absurd_row_still_clamps_to_the_ceiling() {
        let mut sc = SurvivalContent::probe_fixture();
        sc.consumable[0] = ConsumableDef {
            health: 0,
            food: u16::MAX,
            water: u16::MAX,
            seconds: 0,
            fel: 0,
            ..ConsumableDef::NONE
        };
        let mut p = player(&sc);
        p.food = sc.max_food - 1;
        p.water = sc.max_water - 1;
        p.inv[0] = ItemStack {
            item: 0,
            count: 1,
            cond: 0,
            skin: 0,
        };
        let mut q = EventQueue::default();
        assert!(consume(&sc, 0, &mut p, &mut q, 100));
        assert_eq!((p.food, p.water), (sc.max_food, sc.max_water));
    }

    /// Drinking a full vessel leaves the empty one in the slot.
    #[test]
    fn a_drunk_vessel_comes_back_empty() {
        let mut sc = SurvivalContent::probe_fixture();
        sc.vessels[0] = Vessel {
            empty: 3,
            fresh: 0,
            salt: NO_ITEM,
        };
        sc.vessel_count = 1;
        let mut p = player(&sc);
        p.water = 0;
        p.inv[2] = ItemStack {
            item: 0,
            count: 1,
            cond: 0,
            skin: 0,
        };
        let mut q = EventQueue::default();
        assert!(consume(&sc, 2, &mut p, &mut q, 100));
        assert_eq!((p.inv[2].item, p.inv[2].count), (3, 1));
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

    /// A bandage: a wound bleeds a point every 3 s, the wrap spends nothing
    /// until its last tick, switching hands cancels it, and landing it
    /// closes the wound.
    #[test]
    fn a_wrap_lands_on_its_tick_and_closes_the_wound() {
        let mut sc = SurvivalContent::probe_fixture();
        sc.bleed_hp_per_min = 20;
        sc.consumable[0].bleed = -50;
        sc.consumable[0].use_ticks = 120;
        let mut p = player(&sc);
        p.inv[0] = ItemStack {
            item: 0,
            count: 2,
            cond: 0,
            skin: 0,
        };
        p.bleed = 60;
        let mut q = EventQueue::default();
        let hp = p.hp;
        for _ in 0..90 {
            step(&sc, &mut p, &mut q);
        }
        assert_eq!(
            (p.hp, p.bleed),
            (hp - 1, 59),
            "1 hp per 3 s, owed down by one"
        );

        assert!(begin_use(&sc, 0, &mut p, 1000));
        p.frame.sel = 1;
        tick_use(&sc, &mut p, &mut q, 1001, 100);
        assert_eq!(
            (p.use_at, p.inv[0].count),
            (0, 2),
            "a hand put away spends nothing"
        );

        p.frame.sel = 0;
        assert!(begin_use(&sc, 0, &mut p, 2000));
        assert!(begin_use(&sc, 0, &mut p, 2050), "a second press mid-wrap");
        tick_use(&sc, &mut p, &mut q, 2119, 100);
        assert_eq!(p.inv[0].count, 2, "not before its tick");
        tick_use(&sc, &mut p, &mut q, 2120, 100);
        assert_eq!((p.inv[0].count, p.bleed, p.use_at), (1, 9, 0));
    }
}
