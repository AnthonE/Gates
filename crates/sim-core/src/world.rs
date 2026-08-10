//! World state, the command buffer, the tick, and `state_hash` (DESIGN.md
//! §4/§7). Fixed-capacity everything: no allocation anywhere in this module,
//! at construction or in the tick. All mutation flows through `Command`s
//! applied in submission order, then players step in slot order — the fixed
//! order determinism requires.

use crate::backpack::{BackpackContent, Backpacks};
use crate::build::{self, BuildContent, Pieces};
use crate::combat::{self, CombatContent};
use crate::craft::{self, CraftContent, CraftJob};
use crate::deploy::{self, DeployContent, Deploys};
use crate::fmath::floor_i32;
use crate::gather::{self, cell_key, GatherContent, ItemStack, SlotLives, Swing, NO_CELL, NO_ITEM};
use crate::input::InputFrame;
use crate::inventory;
use crate::limits::{
    BOX_SLOTS, CRAFT_QUEUE, HOTBAR_SLOTS, INV_SLOTS, MAX_ARROWS, MAX_COMMANDS_PER_TICK,
    MAX_EVENTS_PER_TICK, MAX_PLAYERS, MAX_REMOVALS_PER_TICK, STATE_HASH_INTERVAL,
};
use crate::loot::{LootContent, LOOT_BARREL};
use crate::mob;
use crate::movement::{self, quant_xz, quant_y, Body};
use crate::persist::PlayerSave;
use crate::ranged;
use crate::rng::cell_hash;
use crate::survival::{self, SurvivalContent};
use crate::terrain::{self, ScatterTable};
use crate::yaw_lut::yaw_dir;
use xxhash_rust::xxh3::Xxh3;

/// Noise channel reserved for spawn-point selection.
const CH_SPAWN: u32 = 96;

/// Beach spawn ring (DECISIONS.md §open, "beach spawn ring"). Every number
/// here is a documented default, none of them spoken.
///
/// The ray bracket is geometry, not taste: the continent falloff puts the
/// coastline at `CONTINENT_RADIUS` ± wobble with a 160 m edge, so land is
/// solid well inside 640 m and the sea floor is below the target well
/// before 1024 m — which is also the largest radius that keeps every
/// bearing's outer probe inside the 2048 m island square (an axis bearing
/// lands exactly on its edge).
const SPAWN_CANDIDATES: i32 = 48;
const SPAWN_RAY_INNER: f32 = 640.0;
const SPAWN_RAY_OUTER: f32 = 1024.0;
/// Where on the beach to stand: above `movement::WADE_GROUND_MAX` (0.4 m,
/// so a fresh spawn is on sand and not wading) and below the 2 m beach mask.
const SPAWN_TARGET_H: f32 = 1.2;
/// 384 m of bracket halved 12 times = under 10 cm of shoreline resolution.
const SPAWN_BISECT_ITERS: i32 = 12;
/// The walkability shape used by foundations and the old placeholder alike.
const SPAWN_MAX_SLOPE: f32 = 1.0;
/// Clearance from any scatter slot center. The widest archetype the client
/// draws is the tree, whose canopy radius is capped at `PINE_MAX_R` = 1.7 m
/// (`web/src/props.js`); 1.7 × 1.1 max scale ≈ 1.9 m, add the 0.4 m capsule
/// and 4 m leaves a spawn standing clear of it, not merely outside it.
///
/// That was a sentence about a constant in another language until
/// `ci/pine_shape.mjs`, which reads this number, the capsule, the scatter's
/// scale range and the tree's actual vertices, and closes the arithmetic. A
/// canopy widened for taste on the client used to be able to put fresh spawns
/// back inside trees with every gate green.
const SPAWN_CLEAR_M: f32 = 4.0;

/// Integer event codes (CLAUDE.md wall 3) — the sim's outbound facts, one
/// ring per tick, drained by the server after `tick` returns.
/// EV_GATHER: a = player id, b = item index << 16 | units actually added.
/// Read it as "these units entered your inventory", not as "a node paid":
/// looting a backpack (backpack.rs) announces its take the same way, and
/// deliberately — the client's `+N Item` toast is the right feedback for
/// both, and loot pays in the currency gathering already pays in.
pub const EV_GATHER: u8 = 1;
/// EV_SLOT_HARVESTED: a = cell key (cx << 16 | cz), b = terrain occupant
/// ordinal (`terrain::Occupant as u32`) — *what* stopped standing there,
/// not which row of the gather table it came from. It read "gatherable
/// index" while only nodes could be exhausted, and that was the same
/// number minus one; a barrel has no gather row at all, so the event now
/// names the occupant and covers both. The wire is unaffected either way:
/// the server sends the cell alone (`encode_event_slot_change`) and the
/// client re-derives the occupant from shared worldgen.
pub const EV_SLOT_HARVESTED: u8 = 2;
/// EV_SLOT_RESPAWNED: a = cell key, b = 0.
pub const EV_SLOT_RESPAWNED: u8 = 3;
/// EV_WEAK_MARK: a = player id, b = cell key, c = weak-hit bit << 8 |
/// next mark heading (u8 over the 256-entry yaw LUT). Swinger-only fact:
/// the mark is per-player (gather.rs).
pub const EV_WEAK_MARK: u8 = 4;
/// EV_CRAFT_DONE: a = player id, b = item index << 16 | units actually
/// added (0 = full inventory; the loss is announced, never silent).
pub const EV_CRAFT_DONE: u8 = 5;
/// EV_CRAFT_REFUSED: a = player id, b = `craft::REFUSE_*` reason code.
pub const EV_CRAFT_REFUSED: u8 = 6;
/// EV_PIECE_PLACED: a = build cell key (cx << 16 | cz), b = level << 16 |
/// loc << 8 | piece row.
pub const EV_PIECE_PLACED: u8 = 7;
/// EV_BUILD_REFUSED: a = player id, b = `build::REFUSE_B_*` reason code.
pub const EV_BUILD_REFUSED: u8 = 8;
/// EV_DEPLOY_PLACED: a = build cell key, b = level << 16 | loc << 8 |
/// row, c = owner player id.
pub const EV_DEPLOY_PLACED: u8 = 9;
/// EV_DEPLOY_REFUSED: a = player id, b = `deploy::REFUSE_D_*` reason.
pub const EV_DEPLOY_REFUSED: u8 = 10;
/// EV_PIECE_REMOVED: a = build cell key, b = level << 16 | loc << 8 | row
/// (the wire broadcasts and restarts in-progress walks). One code for all
/// three ways a piece leaves the world — decay's sweep, a raid swing, and
/// the structural collapse either of them can start (build.rs
/// `collapse_from`) — because a client redraws the same way for each.
pub const EV_PIECE_REMOVED: u8 = 11;
/// EV_DEPLOY_REMOVED: a = build cell key, b = level << 16 | loc << 8 | row.
pub const EV_DEPLOY_REMOVED: u8 = 12;
/// EV_STOCK: a = feeder player id, b = hearth cell key, c = level — the
/// feed ack; the wire reads the hearth's stock from the world at encode.
pub const EV_STOCK: u8 = 13;
/// EV_DOOR: a = build cell key, b = level << 16 | loc << 8 | has_lock << 2
/// | locked << 1 | open, c = the player whose action changed it. The
/// door's whole state, absolute, whether the toggle, the lock or the
/// keypad moved it (lock v1). Broadcast — door state is a world fact like
/// a placement. The three bits are the door as the world sees it and
/// nothing more: no code, no owner and no remembered list is on this
/// lane, because none of it is a client's to know. A **box's** lock rides
/// the same lane (locks on boxes): the event is addressed, and a box's
/// `open` bit is simply always 0.
pub const EV_DOOR: u8 = 14;
/// EV_HIT: a = attacker player id, b = victim player id, c = damage dealt.
/// The attacker's fact — the hitmarker, not the truth; EV_HEALTH is what
/// the victim's bar reads (combat.rs).
pub const EV_HIT: u8 = 15;
/// EV_HEALTH: a = player id, b = hp after the change, c = max hp. Own-fact,
/// absolute: a client that misses one hears the whole truth from the next.
pub const EV_HEALTH: u8 = 16;
/// EV_DEATH: a = the player who died, b = the player who killed them
/// (equal to `a` if that ever becomes possible; today nothing but another
/// hand can kill). Broadcast — a death is a world fact like a placement.
pub const EV_DEATH: u8 = 17;
/// EV_BAG_DROPPED: a = container id, b = the container's owner — the
/// player whose body it came off for a death bag, the smasher for a
/// barrel's loot, the dead animal's tagged mob id for a corpse bag
/// (`backpack::stand_up`). Broadcast — a container on the ground is a
/// world fact like a placement; the wire reads its position out of the
/// store at encode, the way a hearth's stock is read (backpack.rs), and
/// carries no owner at all, so `b` is a sim-side fact only.
pub const EV_BAG_DROPPED: u8 = 18;
/// EV_BAG_REMOVED: a = backpack id, b = `backpack::BAG_GONE_*` (despawn,
/// emptied, evicted). Broadcast, and it restarts in-progress bag sync
/// walks the same way a piece/deploy removal does.
pub const EV_BAG_REMOVED: u8 = 19;
/// EV_STRUCT_HIT: a = build cell key, b = `STRUCT_DEPLOY_BIT` | level << 16
/// | loc << 8 | row, c = damage dealt << 16 | hp left. The raid's progress
/// bar — a wall that shows nothing under thirty swings reads as an
/// invulnerable wall, so this is the one place a structure's hp crosses
/// the wire (build.rs otherwise keeps hp sim-only). Destruction still
/// arrives as EV_PIECE_REMOVED / EV_DEPLOY_REMOVED; this never carries it.
pub const EV_STRUCT_HIT: u8 = 20;
/// EV_VITALS: a = player id, b = food << 16 | water, c = max food << 16 |
/// max water. Own-fact and absolute, for exactly `EV_HEALTH`'s reason — a
/// client that misses one hears the whole truth from the next, so no
/// client-side meter can drift away from the sim's (survival.rs).
pub const EV_VITALS: u8 = 21;
/// EV_CONSUMED: a = player id, b = item index << 16 | inventory slot. The
/// eat acknowledgement — own-fact, and the client's cue to play the ramp.
pub const EV_CONSUMED: u8 = 22;
/// EV_CONSUME_REFUSED: a = player id, b = `survival::REFUSE_C_*`. A button
/// that eats the input and says nothing is indistinguishable from a broken
/// one, so every refusal is announced (craft/build/deploy's posture).
pub const EV_CONSUME_REFUSED: u8 = 23;
/// EV_DRANK: a = player id, b = water units restored, c = hp the drink
/// cost. Own-fact, and it exists for the cost: `EV_HEALTH` is absolute and
/// states the new number without naming what took it, so a client that
/// heard only the health drop could not tell a mouthful of sea from a
/// knife (survival.rs). A refused drink rides `EV_CONSUME_REFUSED` — one
/// refusal channel for the whole module.
pub const EV_DRANK: u8 = 24;
/// EV_RESPAWN: a = player id, b = 1 if the body woke on its own sleeping
/// bag, 0 if the spawn ring answered instead. Own-fact, announced by
/// `World::wake` on every respawn without exception, so "where did I wake
/// and why" is a fact the sim states rather than one a reader infers by
/// comparing a position against a bag list.
///
/// On the wire since v16, and the reason it is worth a subtype is the
/// death screen: the body no longer wakes by itself, so this is the one
/// message that says the screen may close. It still carries no position —
/// the snapshot does that, as it always did — only which anchor answered,
/// because "the bag was spent, you are on a beach" is a fact the player
/// needs and cannot read off a coordinate.
pub const EV_RESPAWN: u8 = 25;
/// EV_MOVED: a = player id, b = the move's address
/// (`inventory::addr`: from kind << 24 | from slot << 16 | to kind << 8 |
/// to slot), c = count moved << 16 | the item that left the `from` slot.
///
/// Own-fact, and it names what the sender asked for rather than the whole
/// container, because a move here is all-or-nothing: the address plus the
/// count plus the item is the entire difference between the state the
/// client predicted and the state the sim now holds. `c`'s item is the
/// reconcile hook — a client whose picture of the source slot was stale
/// gets an item id it did not expect and knows to redraw, instead of
/// silently carrying a divergence the way a partial move would leave one.
/// A swap reads as the same event; the client asked for a whole stack onto
/// an occupied slot and already holds both sides of the exchange.
pub const EV_MOVED: u8 = 26;
/// EV_MOVE_REFUSED: a = player id, b = `inventory::REFUSE_M_*` reason,
/// c = the move's address, exactly as `EV_MOVED.b` packs it.
///
/// `b` is the reason and `c` the address — the order every other refusal
/// in this lane uses (`EV_DEPLOY_REFUSED`, `EV_CONSUME_REFUSED`), so the
/// field a reader reaches for first is the same one across the lane.
///
/// The address rides along, and that is the point of the event: it is what
/// lets a client roll back the one move it predicted rather than resync a
/// container. This is the disconnect that never happens — see
/// `inventory.rs`, where the reference's three-fixes-in-half-an-hour day
/// is written down.
pub const EV_MOVE_REFUSED: u8 = 27;
/// EV_PIECE_REPAIRED: a = build cell key (cx << 16 | cz), b =
/// `STRUCT_DEPLOY_BIT` | level << 16 | loc << 8 | row, c = healed << 16 |
/// hp now.
///
/// `b` carries `EV_STRUCT_HIT`'s bit in `EV_STRUCT_HIT`'s position and
/// with its meaning: the address names the deployable store rather than
/// the piece store. A door stands in its doorway and the two share one
/// address, so the event has to say which one was mended for the same
/// reason the hit has to say which one was struck.
///
/// `a` and `b` are otherwise `EV_PIECE_PLACED`'s exactly, because it is
/// the same address said the same way; `c` is `EV_STRUCT_HIT`'s — its `c` is
/// `dealt << 16 | left`, and a repair is that event's opposite, so the two
/// halves of a wall's life story pack their numbers in the same order. A
/// client that mirrors hp off `EV_STRUCT_HIT` reads this with the same
/// shifts and no new rule.
///
/// Broadcast, not unicast: a wall's hp is a world fact, and the attacker
/// standing outside it has more use for the news than the owner does.
pub const EV_PIECE_REPAIRED: u8 = 28;

/// EV_CHARGE_PLACED: a = build cell key (cx << 16 | cz), b =
/// `STRUCT_DEPLOY_BIT` | level << 16 | loc << 8 | row, c = fuse ticks
/// until the blast.
///
/// `a` and `b` are `EV_PIECE_REPAIRED`'s exactly — the same address said
/// the same way, store bit at 24 — so a client that can draw a hit or a
/// mend on a wall can stick a charge to it with no new layout to learn.
///
/// `c` is the fuse **remaining**, not the tick it fires on. A client has
/// no shared tick origin to subtract an absolute deadline from, and a
/// countdown is what it draws either way; the sim keeps the deadline
/// (`charge::ChargeRec::fires_at`) because a deadline cannot drift and a
/// decremented counter can.
///
/// Broadcast, not unicast, and that is the point of the event: a burning
/// fuse is the one piece of news in this game that the *defender* needs
/// more than the actor does. The blast itself has no event of its own — it
/// arrives as `EV_STRUCT_HIT` from `charge::tick_fuses`, which is the same
/// news a swing makes and is already drawn.
pub const EV_CHARGE_PLACED: u8 = 29;

/// An oven's fire went in or out (`oven.rs`). `a` = `cell_key(cx, cz)`,
/// `b` = `level << 16 | lit`, `c` = the hand that pressed, or **0 when
/// the oven ran dry and snuffed itself** — a fact with no actor behind
/// it, the posture `EV_SLOT_RESPAWNED` already takes.
///
/// Absolute, never a delta, for the reason `EV_DOOR` is: a client that
/// toggled optimistically is confirmed or corrected by the same event,
/// and two presses racing must not leave the two sides disagreeing about
/// which way the fire ended up. Broadcast, like the door: a lit fire is
/// visible from outside the base it is in, so hiding it from the
/// neighbourhood would make the sim disagree with the picture.
pub const EV_OVEN: u8 = 30;
/// EV_KNOCK: a = build cell key, b = level << 16 | loc << 8, c = the
/// player who knocked. Broadcast, and that **is** the feature: a knock is
/// the only channel a locked-out player has to the person inside
/// (`reference/DOORS.md` §4, shipped by the reference in Nov 2014). It
/// rides the refusal path — every press the lock turns away knocks — so
/// there is no verb to forge and nothing to rate-limit beyond the reach
/// check the press already paid.
///
/// No state, nothing persisted, no field for *why*: a knock says somebody
/// is at the door and deliberately not who is allowed through it.
pub const EV_KNOCK: u8 = 31;
/// EV_AUTH: a = build cell key, b = level << 16 | loc << 8 | grant, c =
/// the player now remembered (`lock::GRANT_*`). **Own-fact** — the sender
/// learns their own rights and nobody learns anyone else's, which is the
/// same posture `EV_HEALTH` takes and the reason a lock's remembered list
/// never crosses the wire as a list.
///
/// Only a *correct* code produces one; a wrong one is
/// `EV_DEPLOY_REFUSED` with `REFUSE_D_CODE`, so the two outcomes are two
/// codes and a client cannot mistake silence for a grant.
pub const EV_AUTH: u8 = 32;

/// EV_RESEARCH: a = the player who learned it, b = recipe index, c = the
/// coin burned. **Own-fact** — a blueprint is personal (`research.rs`), so
/// nobody else's client has any use for it and broadcasting what a rival
/// has unlocked would be handing out their tech level for free.
///
/// The cost rides `c` rather than being looked up, because it is what the
/// player just paid and the table can change under a shard: an ack that
/// re-derived the price would tell them a number they were not charged.
pub const EV_RESEARCH: u8 = 33;
/// EV_RESEARCH_REFUSED: a = the player who asked, b = a
/// `research::REFUSE_R_*` reason. Own-fact, `EV_CRAFT_REFUSED`'s shape
/// exactly: a verb that refuses says why, in an integer, to the one hand
/// that pressed.
pub const EV_RESEARCH_REFUSED: u8 = 34;

/// The highest code above, named rather than counted: the event codes are
/// `1..=EV_MAX` with no gaps, and `test_event_roles`'s coverage ledger
/// scans that range. It lived in that test as a literal `25`, which meant a
/// twenty-sixth code could land and read as classified while nothing had
/// classified it. Tying it to the last constant closes half of that; the
/// other half is the ledger's own `every_event_code_is_in_range`, which
/// parses this file and fails if a code is declared past this line.
pub const EV_MAX: u8 = EV_RESEARCH_REFUSED;

/// Why a body fell (`Player::death_cause`). Sim state on the record rather
/// than fields on `EV_DEATH`, whose three are already spent — the server
/// reads the cause, the weapon and the range back out of the store when it
/// encodes the death, exactly the way a bag's position is read out of the
/// backpack store at encode (`EV_BAG_DROPPED`). The deferred respawn is
/// what makes that safe: the corpse is still in its slot until the player
/// answers the screen, so the facts are always there to read.
///
/// Another player's hand. `death_by` is the killer, `death_item` the
/// weapon they held, `death_range_cm` how far the blow landed from —
/// ALPHA.md §1's "who/what killed you — range and weapon, no map position".
pub const DEATH_BY_HAND: u8 = 0;
/// The survival clock finished the job: starving, dried out, or both
/// stacked (survival.rs). `death_by` is the player's own id.
pub const DEATH_BY_CLOCK: u8 = 1;
/// A mouthful of sea water on the last points of health (survival.rs).
/// Its own cause and not the clock's, for `EV_DRANK`'s reason: a death the
/// player *pressed a key for* is a different sentence from one that
/// happened to them.
pub const DEATH_BY_SALT: u8 = 2;
/// An arrow (ranged.rs). Its own cause and not `DEATH_BY_HAND`'s for the
/// same reason `DEATH_BY_SALT` is not the clock's: the sentence the death
/// screen builds is "who, with what, from how far", and 34 m is a
/// different fact about a fight from 1.6 m. `death_item` is the bow, not
/// the arrow — the weapon is what the killer held.
pub const DEATH_BY_ARROW: u8 = 3;

/// The highest cause above, named rather than counted — `EV_MAX`'s
/// discipline applied to a *value domain* instead of a code ledger.
///
/// The difference matters, because this is the half wall 6 does not cover.
/// `test_protocol_golden` pins the **layout** of `EV_DEATH`, and a fourth
/// cause moves no layout: `DEATH_CAUSE_BITS` is 2, three values are spent,
/// and the fourth bit pattern is already on the wire being *refused* by
/// both ends. So a new `DEATH_BY_*` lands with the golden green, the
/// replay green (the event ring is not in `state_hash`) and clippy green
/// (every cause is a `u8`) — and the encoder returns `Err(Range)` for
/// every death by that cause, forever. The server counts the error and
/// drops the packet, so the victim is a corpse whose death screen never
/// opens.
///
/// That is not hypothetical: it is the shape of a judged **FAIL** on
/// 2026-08-05, reproduced executably by the judge before it was believed.
/// Protocol derives `DEATH_CAUSE_MAX` from this constant rather than
/// restating it, and `death_causes_are_a_closed_ledger` parses this file
/// so a cause declared past this line fails loudly instead of silently.
/// A widened *meaning* is still a wire change (`protocol/src/lib.rs`) —
/// this makes the widening impossible to do by accident, not permitted.
pub const DEATH_BY_MAX: u8 = DEATH_BY_ARROW;

/// Bit 24 of `EV_STRUCT_HIT`'s `b`: the address names the deployable store
/// (a door, a box) rather than the piece store. Level, loc and row are all
/// 8-bit fields below it, so bit 24 is the first free one.
pub const STRUCT_DEPLOY_BIT: u32 = 1 << 24;

#[derive(Clone, Copy, Debug, Default)]
pub struct SimEvent {
    pub code: u8,
    pub a: u32,
    pub b: u32,
    pub c: u32,
}

/// Per-tick event ring (limits.rs: MAX_EVENTS_PER_TICK, drop newest,
/// counted). Derived output, not sim state — it stays out of state_hash
/// the way `last_hash` does.
pub struct EventQueue {
    entries: [SimEvent; MAX_EVENTS_PER_TICK],
    len: usize,
    /// Events refused by a full ring since the last clear (diagnostic).
    pub dropped: u32,
}

impl EventQueue {
    pub fn push(&mut self, code: u8, a: u32, b: u32, c: u32) {
        if self.len == MAX_EVENTS_PER_TICK {
            self.dropped += 1;
            return;
        }
        self.entries[self.len] = SimEvent { code, a, b, c };
        self.len += 1;
    }

    pub fn entries(&self) -> &[SimEvent] {
        &self.entries[..self.len]
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub(crate) fn clear(&mut self) {
        self.len = 0;
        self.dropped = 0;
    }
}

impl Default for EventQueue {
    fn default() -> Self {
        Self {
            entries: [SimEvent::default(); MAX_EVENTS_PER_TICK],
            len: 0,
            dropped: 0,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Player {
    pub id: u32,
    pub active: bool,
    pub body: Body,
    /// Last applied input — sim state, so input-reuse replays for free.
    pub frame: InputFrame,
    /// 6 hotbar + 24 backpack (ALPHA.md §1). A join starts empty — the
    /// naked spawn punches its first resources (gatherables' hand rows).
    pub inv: [ItemStack; INV_SLOTS],
    /// Tick the next swing is allowed at (gather.rs cadence).
    pub next_swing: u64,
    /// Weak-spot chase: the cell this player last landed a hit on
    /// (`NO_CELL` = none) and how many hits they've landed there. The mark
    /// heading derives from these (gather.rs), so they are sim state.
    pub ws_cell: u32,
    pub ws_hits: u16,
    /// Craft queue, dense with the head at 0 (craft.rs). Sim state.
    pub jobs: [CraftJob; CRAFT_QUEUE],
    /// Tick the head job's current unit completes at; 0 = idle.
    pub craft_done_at: u64,
    /// Blueprints this player has researched: bit `i` = recipe `i`
    /// (research.rs). Sim state, saved with the player, and the reason
    /// `Player` carries a mask rather than a set — one shift on the craft
    /// path, nothing to iterate, nothing to allocate.
    pub known: u64,
    /// Hit points. A join grants `CombatContent::player_hp`, so inert
    /// content leaves this 0 and nothing can be killed (combat.rs).
    pub hp: u16,
    /// The hp this body was granted at join — the ceiling a heal clamps to
    /// (survival.rs). Carried on the player rather than read back out of
    /// `CombatContent` so a heal is correct without the survival module
    /// having to learn the combat table.
    pub hp_max: u16,
    /// How many times this player has died. Sim state, and not only a
    /// counter: it walks the spawn ring's candidate sequence forward, so
    /// two deaths are two different beaches (`spawn_pos_n`).
    pub deaths: u16,
    /// The survival clock (survival.rs). Food and water only ever fall;
    /// eating puts them back. `*_acc` are the rational accumulators that
    /// make each rate exact — sim state, because a replay that resumed
    /// mid-span with a zeroed remainder would drift.
    pub food: u16,
    pub water: u16,
    pub food_acc: u32,
    pub water_acc: u32,
    /// Starvation damage accumulator, cleared the moment either meter is
    /// refilled — a partial minute is forgiven, never banked.
    pub hurt_acc: u32,
    /// The in-flight heal from a consumed row: hp still owed, the total the
    /// rate is computed from, the span in ticks, and the accumulator.
    pub heal_rem: u16,
    pub heal_total: u16,
    pub heal_span: u32,
    pub heal_acc: u32,
    /// The death screen. `true` between the tick this body fell and the
    /// `Command::Respawn` that answers — ALPHA.md §1's respawn flow is a
    /// *choice* ("choose beach or a bag"), and a choice needs somewhere to
    /// wait. A dead body keeps its slot, its position and its death, and
    /// nothing else: it takes no input, runs no clock, swings at nothing,
    /// crafts nothing and cannot be hit again (`combat::strike` already
    /// skips `hp == 0`). It is not a timer — no clock releases it, because
    /// the one thing this state is for is that the player decides.
    ///
    /// `hp == 0` would nearly serve as the predicate and deliberately does
    /// not: inert content grants `player_hp == 0` at the door, so every
    /// body in an unarmed world would read as a corpse.
    pub dead: bool,
    /// What the death screen says. Written once at the death site, read by
    /// the server at encode and cleared by the wake — see `DEATH_BY_HAND`.
    pub death_by: u32,
    pub death_cause: u8,
    pub death_item: u16,
    pub death_range_cm: u16,
    /// **Nobody is driving this body, and it is still here.** A `Leave`
    /// used to clear `active`, which deleted the body outright and made a
    /// disconnect the safest thing a player could do — the design the
    /// reference game's own Devblog 7 says it *replaced*
    /// (`reference/SAVES.md` §1, §9.1). A sleeper stays in the world: it
    /// stands, it takes no input, its metabolism runs, and it can be
    /// killed. Offline raiding is not a feature built on top of this; it
    /// is what this bit *is*.
    ///
    /// Still `active` on purpose. Every predicate that asks "is there a
    /// body here" — `combat::strike`'s target scan, `ranged`'s, the
    /// snapshot's interest filter, `state_hash` — must answer yes, and the
    /// one thing that must NOT is the input path. So the bit is read where
    /// agency is decided (`tick`) and nowhere else, which is why adding it
    /// changed no target scan.
    pub sleeping: bool,
    /// The tick this body fell asleep, and the only reason it is stored:
    /// slots are `MAX_PLAYERS` and sleepers hold them, so a shard with
    /// every slot asleep must still admit a player (wall 4 — every store
    /// has a cap and a stated overflow policy). The policy is **evict the
    /// longest-asleep**, and this is the key it is ordered by. The scan
    /// lives on the server (`ShardCore::evict_victim` — two-phase eviction,
    /// so the victim's save is taken before the body is removed), which
    /// reads this field through the world it owns.
    ///
    /// Sim state, so it is hashed: it is what the server's pick is ordered
    /// by, and a shard that drifted on it would nominate a different
    /// victim on its next replayed session.
    pub slept_at: u64,
}

impl Default for Player {
    fn default() -> Self {
        Self {
            id: 0,
            active: false,
            body: Body::default(),
            frame: InputFrame::default(),
            inv: [ItemStack::default(); INV_SLOTS],
            next_swing: 0,
            ws_cell: NO_CELL,
            ws_hits: 0,
            jobs: [CraftJob::default(); CRAFT_QUEUE],
            craft_done_at: 0,
            known: 0,
            hp: 0,
            hp_max: 0,
            deaths: 0,
            food: 0,
            water: 0,
            food_acc: 0,
            water_acc: 0,
            hurt_acc: 0,
            heal_rem: 0,
            heal_total: 0,
            heal_span: 0,
            heal_acc: 0,
            dead: false,
            death_by: 0,
            death_cause: 0,
            death_item: NO_ITEM,
            death_range_cm: 0,
            sleeping: false,
            slept_at: 0,
        }
    }
}

/// Every mutation the sim accepts. The WAL is exactly this stream plus the
/// tick numbers (DESIGN.md §7).
#[derive(Clone, Copy, Debug)]
pub enum Command {
    Join {
        id: u32,
    },
    /// Join **as a saved character** — the restore, and it rides the command
    /// stream rather than being read out of a file by the sim.
    ///
    /// That is the whole reason this variant exists instead of a `World`
    /// method the server could call: the WAL is "exactly this stream plus
    /// the tick numbers" (below), so a join that loaded state through a side
    /// channel would replay as a *fresh* spawn and every hash after it would
    /// differ. Carrying the record makes the stream self-contained — a
    /// replay restores what the session restored, byte for byte, with no
    /// file to still be there.
    ///
    /// It costs the enum its size: `PlayerSave` is 188 bytes, so `Command`
    /// grows from 16 to ~192 and the two `MAX_COMMANDS_PER_TICK` buffers
    /// with it (~45 kB each, measured — `ShardCore` was 443 kB). Paid
    /// knowingly; the alternative was a determinism hole.
    JoinAs {
        id: u32,
        save: PlayerSave,
    },
    /// The connection ended. **This does not remove the body** — it puts
    /// it to sleep (`Player::sleeping`), which is the whole of
    /// `reference/SAVES.md` §9.1 in one arm.
    Leave {
        id: u32,
    },
    /// Take over a sleeping body: seat connection `id` onto the sleeper
    /// currently carrying id `sleeper`.
    ///
    /// Two ids because the sim has no idea who anybody is. A player id is
    /// `generation << 8 | slot`, minted per connection and meaningless
    /// across two of them (`persist.rs` says so from the other side), so
    /// the identity that survives a disconnect is the server's opaque
    /// `PlayerKey` and it deliberately never enters this crate. The server
    /// resolves key → sleeper id outside the sim and names both here, which
    /// is what keeps the command stream self-contained: a replay wakes the
    /// same body without a key table still existing.
    ///
    /// **A miss is legal and ordinary**, not an error to report: the
    /// sleeper may have been evicted for slot pressure since the server
    /// last saw it. `ShardCore` checks first and falls back to `JoinAs`,
    /// and this arm re-checks anyway, because a WAL replayed against a
    /// world that evicted differently must not seat a body from nothing.
    Wake {
        id: u32,
        sleeper: u32,
    },
    /// Remove the sleeping body `id` — **the second phase of two-phase
    /// eviction**, and the only way a body ever vacates its slot (death
    /// does not: the corpse keeps it while the death screen waits).
    ///
    /// `seat` used to evict the longest-asleep sleeper on its own authority
    /// when a join found no free slot. That was right for the world and
    /// wrong for persistence (`reference/SAVES.md` §9.2): the store's
    /// record for the victim was frozen at the moment they left, so a
    /// sleeper raided *after* that and then evicted came back from the
    /// stale record — the raid quietly undone. Only the server can take a
    /// current save off the live body, so the order has to be: pick the
    /// victim, take its save, **then** queue this, then the join — and the
    /// policy (longest-asleep, `ShardCore::evict_victim`) moved to the
    /// server with the save. The id travels here so a replayed stream
    /// evicts the same body (wall 5); `evictions` is hashed and counts it.
    ///
    /// A miss — `id` names nobody, or a body that is awake — is legal and
    /// a no-op, `Wake`'s posture: a WAL replayed against a world that
    /// diverged must refuse rather than delete a body somebody is driving.
    /// **Not reachable from the wire**: no `ActionMsg` maps to a command in
    /// this family (a client must not be able to evict anybody) — it is
    /// minted by `ShardCore::connect_as` alone, beside `Join` and `Leave`.
    Evict {
        id: u32,
    },
    Input {
        id: u32,
        frame: InputFrame,
    },
    /// Enqueue `count` crafts of recipe row `recipe` (craft.rs validates
    /// and refuses by event, never by panic).
    Craft {
        id: u32,
        recipe: u16,
        count: u16,
    },
    /// Learn the blueprint for whatever is in inventory `slot`, at a
    /// research table in reach, paying the row's coin (research.rs).
    /// The slot is the sender's claim and the sim is the verdict: a forged
    /// index, an empty hand and a stack of wood all land on the same
    /// announced refusal, exactly as `Consume`'s does.
    Research {
        id: u32,
        slot: u8,
    },
    /// Cancel the queue job at `index`, refunding its remaining inputs.
    CraftCancel {
        id: u32,
        index: u16,
    },
    /// Place baked building-piece row `row` at grid address (cx, cz,
    /// level, loc) (build.rs validates and refuses by event, never by
    /// panic).
    Place {
        id: u32,
        row: u16,
        cx: u16,
        cz: u16,
        level: u8,
        loc: u8,
    },
    /// Place baked deployable row `row` at grid address (deploy.rs
    /// validates and refuses by event, never by panic).
    PlaceDeploy {
        id: u32,
        row: u16,
        cx: u16,
        cz: u16,
        level: u8,
        loc: u8,
    },
    /// Feed the hearth at the address from the feeder's inventory
    /// (deploy.rs).
    Feed {
        id: u32,
        cx: u16,
        cz: u16,
        level: u8,
    },
    /// Toggle the door at the address open/closed (deploy.rs validates
    /// and refuses by event, never by panic).
    Use {
        id: u32,
        cx: u16,
        cz: u16,
        level: u8,
        loc: u8,
    },
    /// Take the thing at the address back down and get it back —
    /// **demolish v1**. `deploy` picks the store, `Repair`'s bit for
    /// `Repair`'s reason: a door and its doorway share one address.
    ///
    /// The two halves are deliberately different verbs wearing one
    /// command. A **piece** comes down only inside its grace window and
    /// refunds whole (`build::demolish`); a **deployable** comes up any
    /// time you may build there and returns its item (`deploy::pick_up`).
    /// The reference draws the line in the same place and for the same
    /// reason: a box is furniture, a wall is a base.
    Demolish {
        id: u32,
        deploy: bool,
        cx: u16,
        cz: u16,
        level: u8,
        loc: u8,
    },
    /// Run one access op at the address — **who may do this here**.
    /// `deploy::ACCESS_OP_*` names the nine: 0..=5 against the code lock
    /// on a door (`deploy::lock_op`), 6..=8 against the hearth's crew
    /// (`deploy::crew_op`). One command with an op field rather than nine,
    /// because the wire's action space was full at fifteen in four bits.
    ///
    /// Both halves validate and refuse by event, never by panic, and both
    /// re-check the op and the code the wire already checked — a replayed
    /// frame reaches the sim without passing a decoder.
    Access {
        id: u32,
        cx: u16,
        cz: u16,
        level: u8,
        loc: u8,
        op: u8,
        code: u16,
    },
    /// Upgrade the piece at the address into `material` — same shape, same
    /// address, a rung up the ladder (build.rs validates and refuses by
    /// event, never by panic).
    Upgrade {
        id: u32,
        cx: u16,
        cz: u16,
        level: u8,
        loc: u8,
        material: u8,
    },
    /// Repair the structure at the address back to its baked hp, paid in
    /// its own materials (build.rs validates and refuses by event). No
    /// amount crosses the wire: how much is missing is the server's fact,
    /// so a client cannot ask to be healed by a number it chose.
    ///
    /// `deploy` picks the store, because a door and its doorway share an
    /// address — the sender's bit, carried through unexamined, because
    /// which of the two a player aimed at is not something the server can
    /// recover after the fact.
    Repair {
        id: u32,
        deploy: bool,
        cx: u16,
        cz: u16,
        level: u8,
        loc: u8,
    },
    /// Plant the held throwable against the structure at the address, with
    /// its content fuse burning (`charge.rs`). `Repair`'s fields exactly,
    /// including the store bit and for the same reason — and, like
    /// `Repair`, no amount and no fuse crosses: what the charge takes off
    /// and how long it burns are the throwable's content row, so neither
    /// is a number a client can choose.
    ///
    /// The one thing this verb does *not* inherit from `Repair` is its
    /// claim check. See `charge::place`.
    Throw {
        id: u32,
        deploy: bool,
        cx: u16,
        cz: u16,
        level: u8,
        loc: u8,
    },
    /// Take everything that fits from the nearest backpack in reach
    /// (backpack.rs). No target crosses: the pick is the sim's, the same
    /// shape a swing's is.
    Loot {
        id: u32,
    },
    /// Move `count` items between two slots (`inventory.rs`). `cont` names
    /// the one ground container this move touches — a bag id for
    /// `CONT_BAG`, a packed `deploy::box_key` address for `CONT_BOX`, zero
    /// when the move is inside the sender's own inventory.
    ///
    /// Unlike `Loot`, this one *does* carry a target, and it has to: the
    /// whole verb is the player choosing which slot, which is the choice
    /// `Loot` takes away by moving everything that fits. What the sim
    /// keeps is the verdict — a forged kind, a slot past the container, a
    /// container out of reach and a count past the stack all land on the
    /// same announced `EV_MOVE_REFUSED`, and none of them on a dropped
    /// session.
    Move {
        id: u32,
        cont: u32,
        from_kind: u8,
        from_slot: u8,
        to_kind: u8,
        to_slot: u8,
        count: u16,
    },
    /// Eat what is in inventory slot `slot` (survival.rs). The slot is the
    /// sender's claim and the sim is the verdict: a forged index, an empty
    /// hand and a stack of wood all land on the same announced refusal.
    Consume {
        id: u32,
        slot: u8,
    },
    /// Drink from the water under your own feet (survival.rs). No target
    /// and no position: the heightfield is a pure function of the seed,
    /// so the sim asks it where the body already is.
    Drink {
        id: u32,
    },
    /// Answer the death screen (ALPHA.md §1: "choose beach or a bag").
    /// `on_bag` asks for the nearest of the sender's own ready sleeping
    /// bags; the ring answers when it is false, and also when it is true
    /// and no bag of theirs is ready — a request the world cannot fill is
    /// a beach, never a refusal, because a player stuck on a death screen
    /// has left the game. From a live body it does nothing at all.
    Respawn {
        id: u32,
        on_bag: bool,
    },
}

pub struct World {
    pub seed: u64,
    pub tick: u64,
    pub players: [Player; MAX_PLAYERS],
    pub scatter: ScatterTable,
    /// The haven pad site (TERRAIN.md §1 stage 8). Resolved once here
    /// because it is a bounded argmax over the road ring — world init,
    /// never a tick (CLAUDE.md wall 2). Derived purely from `seed`, so it
    /// is not state: a replay recomputes the identical site.
    pub haven: terrain::Haven,
    /// Baked gather rules (gather.rs). Construction input like `seed`:
    /// inert until the boot path installs the table baked from
    /// `content/*.toml`, before the first tick. The WAL pins the content
    /// hash it was baked from when the WAL file format lands.
    pub gather: GatherContent,
    /// Baked recipe rules (craft.rs). Construction input like `gather`.
    pub craft: CraftContent,
    /// Baked building-piece rules (build.rs). Construction input too.
    pub build: BuildContent,
    /// Baked deployable rules + upkeep globals (deploy.rs). Construction
    /// input too.
    pub deploy: DeployContent,
    /// Baked fuel + cook rows (oven.rs). Construction input too; the
    /// inert default leaves every fire cold, which is the game that
    /// existed before the module.
    pub cook: crate::oven::CookContent,
    /// Baked research rules (research.rs). Construction input, like every
    /// other content table; `EMPTY` teaches nothing.
    pub research: crate::research::ResearchContent,
    /// Baked melee rows + max hp (combat.rs). Construction input too; the
    /// inert default leaves the world unable to hurt anyone.
    pub combat: CombatContent,
    /// Baked backpack despawn ladder (backpack.rs). Construction input
    /// too; the inert default means death destroys instead of dropping.
    pub backpack: BackpackContent,
    /// Baked survival meters + consumable rows (survival.rs). Construction
    /// input too; the inert default leaves the world without a clock, which
    /// is the game that existed before the module.
    pub survival: SurvivalContent,
    /// What a fresh character is granted (`content/balance.toml`
    /// `[[spawn_kit]]`). `EMPTY` is the default and means a naked spawn.
    pub spawn_kit: inventory::SpawnKit,
    /// Baked loot tables (loot.rs). Construction input too; the inert
    /// default leaves barrels standing, because a barrel that broke into
    /// nothing would be worse than one that does not break.
    pub loot: LootContent,
    /// Baked animal species (mob.rs). Construction input too; the inert
    /// default gives every species zero hit points and no slot ever
    /// hatches, so a content set with no `mobs.toml` rows is a shard
    /// without wildlife rather than a shard with invisible wildlife.
    pub mob: mob::MobContent,
    /// The animal roster — sim state, hashed. Homes are drawn from the
    /// seed at construction beside `haven` and never move; everything else
    /// in it is a tick's business.
    ///
    /// Boxed, the same one-allocation-at-construction posture `backpacks`,
    /// `slot_cache` and `arrows` take: `World` is ~440 kB and is built on
    /// the stack (`ShardCore::new`, every wire test, `probe.rs`).
    ///
    /// **This roster overflowed the wasm shadow stack when it landed inline,
    /// and two branches found that wall on the same day from opposite
    /// ends.** `test_parity_wasm` died as `memory access out of bounds`
    /// inside `Deploys::new` — a constructor neither branch had touched —
    /// because rustc's 1 MiB default had the gate at ~99%. The oven found it
    /// with 8 kB of container state; this found it with 3.5 kB of animals.
    /// The durable fix is theirs and it is `.cargo/config.toml`, which now
    /// states a 4 MiB shadow stack instead of inheriting one. Boxing stays
    /// because it is the right posture for a fixed-capacity store on a
    /// stack-built `World`, not because it is what holds that gate up.
    /// Nothing here allocates in the tick.
    pub mobs: Box<mob::Mobs>,
    /// Placed building pieces — sim state, hashed.
    pub pieces: Pieces,
    /// Placed deployables + the hearth list — sim state, hashed.
    pub deploys: Deploys,
    /// Planted satchel charges with a fuse burning — sim state, hashed.
    /// Small enough to sit inline (`MAX_LIVE_CHARGES` × 24 B ≈ 1.5 kB)
    /// beside the stores it damages, unlike `backpacks` next door.
    pub charges: crate::charge::Charges,
    /// Death backpacks standing on the ground — sim state, hashed.
    /// Boxed, and for one reason: the store is 38 kB of fixed capacity and
    /// `World` is built on the stack (`ShardCore::new`, every wire test),
    /// where it was already within ~600 kB of a 2 MB thread limit. One
    /// construction-time allocation — the same posture `ShardCore` takes
    /// for its client array — keeps `World`'s stack footprint where this
    /// slice found it. Nothing here allocates in the tick (wall 2).
    pub backpacks: Box<Backpacks>,
    /// Upkeep/decay sweep cursors (deploy.rs) — sim state, hashed.
    pub sweep_piece: u32,
    pub sweep_deploy: u32,
    /// Standing-support sweep cursor (build.rs `support_sweep`) — sim
    /// state, hashed. The backstop behind `MAX_COLLAPSE_PIECES`: a
    /// cascade capped mid-fall leaves pieces standing on nothing, and this
    /// cursor is what finds them on a later tick.
    pub sweep_support: u32,
    /// How many sleeping bodies this world has evicted to free a slot
    /// (`Command::Evict`). Wall 4 asks every cap for a stated overflow
    /// policy, and a policy nobody can measure is the mood the walls list
    /// warns about — this is the number that says whether "evict the
    /// longest-asleep" ever fires in practice or is dead code guarding an
    /// unreachable case.
    ///
    /// Hashed, though it drives nothing. An eviction is the one sim event
    /// whose evidence is an *absence*: the body is simply not in the player
    /// scan any more, and two shards that evicted different bodies at
    /// different ticks would still hash the survivors identically for as
    /// long as the two victims were standing still. The counter is what
    /// makes that divergence loud on the tick it happens.
    pub evictions: u64,
    /// Sparse harvested/damaged slot records (TERRAIN.md §2).
    pub slot_lives: SlotLives,
    /// Memo of `terrain::scatter` behind the occupant collision query
    /// (occupy.rs). **Not sim state and deliberately not hashed** — a memo of
    /// a pure function, where which lines happen to be resident changes only
    /// how long an answer took, never the answer. Boxed for the reason
    /// `backpacks` is: `World` is built on the stack and this is 24 kB of
    /// fixed capacity. One allocation at construction, none in the tick.
    pub slot_cache: Box<crate::occupy::SlotCache>,
    /// Every arrow in the air (ranged.rs). Boxed for `slot_cache`'s
    /// reason — `World` is built on the stack and this is fixed capacity
    /// that only grows with `MAX_ARROWS`. One allocation at construction,
    /// none in the tick.
    pub arrows: Box<ranged::Arrows>,
    /// This tick's outbound events; cleared at tick start.
    pub events: EventQueue,
    /// Hash stamped every `STATE_HASH_INTERVAL` ticks (0 until the first).
    pub last_hash: u64,
    /// Dev-only fixed spawn override in meters (DECISIONS.md §open). None
    /// (the default) is the shipping behavior: scattered `spawn_pos`. Set
    /// only from `shard.toml dev_spawn` — it exists so a test can put two
    /// clients inside AOI range on demand. Config, not state: it is world
    /// construction input like `seed`, so it stays out of `state_hash`;
    /// when the WAL file format lands, it pins into the header beside the
    /// seed so replays reproduce the spawns they were played under.
    pub dev_spawn: Option<(f32, f32)>,
}

impl World {
    pub fn new(seed: u64) -> Self {
        let haven = terrain::haven(seed);
        Self {
            seed,
            tick: 0,
            players: [Player::default(); MAX_PLAYERS],
            scatter: ScatterTable::alpha_default(),
            mob: mob::MobContent::EMPTY,
            // After `haven`, because a home is rejected against the two
            // authored sites (mob.rs `home_of`).
            mobs: Box::new(mob::Mobs::new(seed, &haven)),
            haven,
            gather: GatherContent::EMPTY,
            craft: CraftContent::EMPTY,
            build: BuildContent::EMPTY,
            deploy: DeployContent::EMPTY,
            cook: crate::oven::CookContent::EMPTY,
            research: crate::research::ResearchContent::EMPTY,
            combat: CombatContent::EMPTY,
            backpack: BackpackContent::EMPTY,
            survival: SurvivalContent::EMPTY,
            spawn_kit: inventory::SpawnKit::EMPTY,
            loot: LootContent::EMPTY,
            pieces: Pieces::new(),
            deploys: Deploys::new(),
            charges: crate::charge::Charges::new(),
            backpacks: Box::new(Backpacks::new()),
            sweep_piece: 0,
            sweep_deploy: 0,
            sweep_support: 0,
            evictions: 0,
            slot_lives: SlotLives::new(),
            slot_cache: Box::new(crate::occupy::SlotCache::new()),
            arrows: Box::new(ranged::Arrows::new()),
            events: EventQueue::default(),
            last_hash: 0,
            dev_spawn: None,
        }
    }

    /// The beach spawn ring (TERRAIN.md §1 stage 6 — **beach** is the spawn
    /// zone; DESIGN.md §2 — "spawn naked on a beach of a seeded island").
    ///
    /// One candidate = one hashed bearing off the island center, then a
    /// bisection along that ray for the shoreline crossing at
    /// `SPAWN_TARGET_H`. The crossing lands in the beach band by
    /// construction, and the beach band sits *below* the forest band, so a
    /// spawn there is clear of trees structurally — not by rejection-
    /// sampling a forest, which is what the placeholder did and why fresh
    /// spawns stood inside scatter.
    ///
    /// What still gets rejected is local: a cliff shore (slope), and the
    /// beach's own scatter — barrels wash up, bushes and rocks sit there
    /// (TERRAIN.md §1's beach row), and a neighbouring cell one metre
    /// uphill is already meadow or forest and may hold a tree.
    ///
    /// Bounded and allocation-free: at most `SPAWN_CANDIDATES` bearings,
    /// each a fixed `SPAWN_BISECT_ITERS` halvings and a fixed 3×3 scatter
    /// scan. Two documented fallbacks, both gated as unreachable by
    /// `spawn_ring_lands_on_a_clear_beach`: the first merely-walkable shore
    /// point if every candidate's ground is occupied, and the island center
    /// if no bearing brackets a shore at all.
    pub fn spawn_pos(&self, id: u32) -> (f32, f32) {
        self.spawn_pos_n(id, 0)
    }

    /// The spawn ring at generation `gen` — `gen` 0 is the join, and each
    /// death walks it forward one. The generation shifts which candidate
    /// bearings are drawn (`gen · SPAWN_CANDIDATES` further along the same
    /// hashed sequence), so waking up after a death is a different beach
    /// without a second selector, a second constant, or a second ring.
    pub fn spawn_pos_n(&self, id: u32, gen: u32) -> (f32, f32) {
        if let Some(p) = self.dev_spawn {
            return p;
        }
        let c = terrain::ISLAND_SIZE * 0.5;
        let base = (gen as i32).wrapping_mul(SPAWN_CANDIDATES);
        let mut relaxed: Option<(f32, f32)> = None;
        let mut attempt = 0i32;
        while attempt < SPAWN_CANDIDATES {
            let h = cell_hash(self.seed, id as i32, base.wrapping_add(attempt), CH_SPAWN);
            attempt += 1;
            // Index the 256-entry yaw LUT: a bearing, no trig (wall 1).
            // `yaw_dir` indexes by the high byte, so shift the draw up.
            let (dx, dz) = yaw_dir(((h & 0xFF) as u16) << 8);

            // Bracket the crossing, or this bearing has no shore in range:
            // inland must be above the target and the outer radius below it.
            let mut lo = SPAWN_RAY_INNER;
            let mut hi = SPAWN_RAY_OUTER;
            if terrain::height(self.seed, c + dx * lo, c + dz * lo) <= SPAWN_TARGET_H
                || terrain::height(self.seed, c + dx * hi, c + dz * hi) > SPAWN_TARGET_H
            {
                continue;
            }
            let mut i = 0i32;
            while i < SPAWN_BISECT_ITERS {
                let mid = (lo + hi) * 0.5;
                if terrain::height(self.seed, c + dx * mid, c + dz * mid) > SPAWN_TARGET_H {
                    lo = mid;
                } else {
                    hi = mid;
                }
                i += 1;
            }

            // `lo` is the landward side of the crossing: above the target,
            // within a bisection width of it. A gentle shore therefore
            // lands just inside the beach band; a cliff shore overshoots
            // it, and the slope check is what refuses that.
            let x = c + dx * lo;
            let z = c + dz * lo;
            let hy = terrain::height(self.seed, x, z);
            if hy >= terrain::BEACH_MAX_H || terrain::slope(self.seed, x, z) >= SPAWN_MAX_SLOPE {
                continue;
            }
            if relaxed.is_none() {
                relaxed = Some((x, z));
            }
            if self.scatter_clear(x, z) {
                return (x, z);
            }
        }
        relaxed.unwrap_or((c, c))
    }

    /// True if no scatter slot stands within `SPAWN_CLEAR_M` of (x, z).
    ///
    /// Scans the 3×3 cell block around the point, which is conservative
    /// for any clearance under 9 m: a slot two cells out has its center at
    /// least 2·`CELL_SIZE` = 16 m from this cell's center, the point sits
    /// at most half a cell (4 m) from that center, and jitter moves a slot
    /// at most 3 m — so 16 − 4 − 3 = 9 m of unavoidable distance.
    fn scatter_clear(&self, x: f32, z: f32) -> bool {
        let cx = floor_i32(x / terrain::CELL_SIZE);
        let cz = floor_i32(z / terrain::CELL_SIZE);
        let mut ox = -1i32;
        while ox <= 1 {
            let mut oz = -1i32;
            while oz <= 1 {
                let s = terrain::scatter(self.seed, &self.scatter, &self.haven, cx + ox, cz + oz);
                oz += 1;
                if s.occupant == terrain::Occupant::None {
                    continue;
                }
                let sx = s.x - x;
                let sz = s.z - z;
                if sx * sx + sz * sz < SPAWN_CLEAR_M * SPAWN_CLEAR_M {
                    return false;
                }
            }
            ox += 1;
        }
        true
    }

    fn slot_of(&self, id: u32) -> Option<usize> {
        self.players.iter().position(|p| p.active && p.id == id)
    }

    /// The slot of a player who is **standing**. A corpse keeps its slot
    /// while the death screen waits on it, so every verb that acts on the
    /// world resolves through this instead of `slot_of` — otherwise a body
    /// on the death screen could still craft, build, feed a hearth, lock a
    /// door and drink, which is a dead player playing the game.
    ///
    /// Two commands deliberately use `slot_of` instead: `Respawn`, which
    /// only a corpse may send, and `Input`, which is the client's own
    /// frame and keeps flowing so prediction and the server agree about a
    /// body that is standing still (the tick zeroes what it acts on).
    pub fn live_slot_of(&self, id: u32) -> Option<usize> {
        self.slot_of(id).filter(|&s| !self.players[s].dead)
    }

    /// The move verb's one mutating body. Everything it decides is decided
    /// by `inventory.rs`, which cannot write; everything it writes is
    /// below the last `return`, and there are exactly two writes.
    ///
    /// Read the shape rather than the lines: the source and destination
    /// stacks are read out **by value** (`ItemStack` is `Copy`), so the
    /// planner is never handed a reference into a container and the two
    /// sides cannot alias even when both are the same array — which is the
    /// ordinary case, since arranging your own hotbar is `CONT_SELF` to
    /// `CONT_SELF`. That is also why the same-slot ask is refused up front
    /// instead of being allowed to fall through as a no-op: with copies, a
    /// slot moved onto itself would be written twice and the second write
    /// would win, which is a dupe.
    ///
    /// Every exit before the writes announces itself. There is no silent
    /// path and no path that ends the session.
    /// One slot of whichever container `kind` names, by value. `ci` is the
    /// resolved ground-container index and is only read when the kind is a
    /// ground container — the caller has already proved it resolves.
    fn cont_slot(&self, slot: usize, kind: u8, s: u8, ci: usize) -> ItemStack {
        match kind {
            inventory::CONT_BAG => self.backpacks.slot(ci, s as usize),
            inventory::CONT_BOX => self.deploys.box_slot(ci, s as usize),
            _ => self.players[slot].inv[s as usize],
        }
    }

    /// The mirror of `cont_slot`, and the only place a move writes.
    fn set_cont_slot(&mut self, slot: usize, kind: u8, s: u8, ci: usize, v: ItemStack) {
        match kind {
            inventory::CONT_BAG => self.backpacks.set_slot(ci, s as usize, v),
            inventory::CONT_BOX => self.deploys.set_box_slot(ci, s as usize, v),
            _ => self.players[slot].inv[s as usize] = v,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn move_item(
        &mut self,
        slot: usize,
        cont: u32,
        from_kind: u8,
        from_slot: u8,
        to_kind: u8,
        to_slot: u8,
        count: u16,
    ) {
        let pid = self.players[slot].id;
        let addr = inventory::addr(from_kind, from_slot, to_kind, to_slot);

        // 1. Is that an address at all? Kind, both slot indices against
        //    *their own* container's size, and the move-onto-itself case.
        //    Nothing has been read yet.
        if from_kind > inventory::CONT_MAX
            || to_kind > inventory::CONT_MAX
            || from_slot as usize >= inventory::slots_in(from_kind)
            || to_slot as usize >= inventory::slots_in(to_kind)
            || (from_kind == to_kind && from_slot == to_slot)
        {
            self.events
                .push(EV_MOVE_REFUSED, pid, inventory::REFUSE_M_SLOT, addr);
            return;
        }

        // 2. Resolve the ground container, if either side names one. The
        //    command carries **one** handle, so at most one side may be a
        //    ground container — two different ones is a destination the
        //    message cannot address, not a rule about what players may do
        //    (`REFUSE_M_NO_CONTAINER`).
        if from_kind != inventory::CONT_SELF
            && to_kind != inventory::CONT_SELF
            && from_kind != to_kind
        {
            self.events
                .push(EV_MOVE_REFUSED, pid, inventory::REFUSE_M_NO_CONTAINER, addr);
            return;
        }
        let ground = if from_kind != inventory::CONT_SELF {
            from_kind
        } else {
            to_kind
        };
        // A container that left the world and a container out of arm's
        // reach are separate reasons because they are separate
        // player-facing facts: one means "it is gone", the other "walk
        // back". Both kinds answer them the same way — a bag by id, a box
        // by packed address — which is the whole of what a third kind
        // costs here.
        let cont_idx = match ground {
            inventory::CONT_BAG => {
                let Some(i) = self.backpacks.index_of_id(cont) else {
                    self.events
                        .push(EV_MOVE_REFUSED, pid, inventory::REFUSE_M_NO_CONTAINER, addr);
                    return;
                };
                if !self.backpacks.in_reach(i, &self.players[slot]) {
                    self.events
                        .push(EV_MOVE_REFUSED, pid, inventory::REFUSE_M_REACH, addr);
                    return;
                }
                Some(i)
            }
            inventory::CONT_BOX => {
                let Some(i) = self.deploys.box_index(cont) else {
                    self.events
                        .push(EV_MOVE_REFUSED, pid, inventory::REFUSE_M_NO_CONTAINER, addr);
                    return;
                };
                if !self.deploys.box_in_reach(i, &self.players[slot]) {
                    self.events
                        .push(EV_MOVE_REFUSED, pid, inventory::REFUSE_M_REACH, addr);
                    return;
                }
                // The lock's question, asked where the box actually opens
                // (locks on boxes, `DOORS.md` §9.8). The move verb IS the
                // box's open path — the container view is a subscription
                // that grants nothing — so this one check gates deposit
                // and withdrawal alike, on both halves of a move. The box
                // stands on the plane (`loc_fits_placement`), so its lock
                // shares `box_key`'s triple plus `LOC_PLANE`; an oven at
                // the same shape of address can carry no lock (`lockable`)
                // and passes as bare. The refusal is the locked door's
                // own, not a move reason: the panel draws server truth and
                // predicted nothing to roll back (`ui/slots.rs`), and
                // *this lock does not know you* is one sentence whichever
                // verb it refused.
                let b = self.deploys.boxes()[i];
                if !self
                    .deploys
                    .lock_passes(b.cx, b.cz, b.level, build::LOC_PLANE, pid)
                {
                    self.events
                        .push(EV_DEPLOY_REFUSED, pid, deploy::REFUSE_D_OWNER, 0);
                    return;
                }
                Some(i)
            }
            _ => None,
        };
        // Safe past the guard above: `cont_idx` is `Some` whenever either
        // kind is a ground container, and the zero is never indexed
        // otherwise.
        let ci = cont_idx.unwrap_or(0);

        // 3. Read both sides as copies.
        let src = self.cont_slot(slot, from_kind, from_slot, ci);
        // An oven takes fuel, what it cooks, and what it made — nothing
        // else (`oven.rs`, `inventory::REFUSE_M_OVEN`). Asked here, of
        // the *source item*, after the address resolves and before
        // anything is planned: a rule about what may enter a container is
        // a property of this world's content, which `plan_move` has no
        // access to and deliberately never will (it decides arithmetic,
        // and only arithmetic). Rearranging inside the oven is untouched
        // — the item is already in there.
        if to_kind == inventory::CONT_BOX && from_kind != inventory::CONT_BOX {
            let arch = self.deploys.oven_states().get(ci).map(|o| o.arch);
            if let Some(arch) = arch.filter(|_| self.deploys.oven_index(cont).is_some()) {
                if !self.cook.accepts(arch, src.item) {
                    self.events
                        .push(EV_MOVE_REFUSED, pid, inventory::REFUSE_M_OVEN, addr);
                    return;
                }
            }
        }
        let dst = self.cont_slot(slot, to_kind, to_slot, ci);

        // 4. Plan. This is the whole of the validation, and it holds no
        //    reference to anything it could damage.
        let cap = self.gather.stack_max_of(src.item);
        let plan = match inventory::plan_move(cap, src, dst, count) {
            Ok(p) => p,
            Err(why) => {
                self.events.push(EV_MOVE_REFUSED, pid, why, addr);
                return;
            }
        };
        let (new_src, new_dst) = inventory::resolve(plan, src, dst);

        // 5. Mutate. Every check is behind us and both writes always land.
        self.set_cont_slot(slot, from_kind, from_slot, ci, new_src);
        self.set_cont_slot(slot, to_kind, to_slot, ci, new_dst);
        self.events.push(
            EV_MOVED,
            pid,
            addr,
            ((count as u32) << 16) | src.item as u32,
        );
        // A bag a withdrawal emptied leaves by `loot_nearest`'s route, so
        // the wire sees one removal contract however it was emptied. A box
        // does **not** take that route and that is the difference between
        // the two: a bag is litter with a timer, a box is furniture you
        // paid for and placed. Emptying one leaves it standing.
        if ground == inventory::CONT_BAG {
            self.backpacks.drop_if_empty(ci, &mut self.events);
        }
    }

    /// Death, v3: the body falls **and stays down**. What you were carrying
    /// is lying where you fell, the kill is already counted and announced
    /// (combat.rs, survival.rs), and the consequence splits in two — this
    /// half drops the backpack and puts the body on the death screen;
    /// `wake` is the other half and only a `Command::Respawn` reaches it.
    ///
    /// **The wait is the feature.** ALPHA.md §1's respawn flow is "death
    /// screen (who/what killed you — range and weapon, no map position),
    /// choose beach or a bag" — three nouns, and a *choice*. v2 wired the
    /// bag and picked for the player: the nearest ready one always won, so
    /// a body killed at its own doorstep by the raider still standing in it
    /// woke up in that raider's reach with nothing in hand, over and over,
    /// until the bag's cooldown ran out. Choosing the beach is how you
    /// leave a fight you have already lost, and it is the whole reason the
    /// screen has two buttons rather than one.
    ///
    /// No timer releases this state. A body waiting on the screen costs
    /// exactly what a standing one costs — its slot, bounded by
    /// `MAX_PLAYERS` like everything else — and inventing a "you have been
    /// dead long enough" span would be inventing a knob nobody spoke.
    ///
    /// What the corpse keeps is its id, its death count, its position, its
    /// facing and the five facts the screen is made of. Everything else is
    /// `Player::default()` — the inventory went into the bag, and the craft
    /// queue, the heal and the weak-spot chase are destroyed here for the
    /// reasons `wake` records below.
    fn die(&mut self, slot: usize, by: u32, cause: u8, item: u16, range_cm: u16) {
        // A copy of the body as it fell: the bag is built from it after
        // the slot is already being written, and `Player` is `Copy`.
        let body = self.players[slot];
        self.backpacks
            .drop_for(&self.backpack, &body, self.tick, &mut self.events);
        self.players[slot] = Player {
            id: body.id,
            active: true,
            body: body.body,
            frame: body.frame,
            hp: 0,
            hp_max: body.hp_max,
            deaths: body.deaths,
            dead: true,
            death_by: by,
            death_cause: cause,
            death_item: item,
            death_range_cm: range_cm,
            // **Carried, and this is the line the whole slice rests on.**
            // `..Player::default()` would clear it, and a killed sleeper
            // that stopped being a sleeper is one the server can no longer
            // find at the owner's next join — so it would fall through to
            // `JoinAs` and hand them back the record they left with, alive,
            // with everything they were carrying. Offline raiding would
            // cost the raider a fight and pay them nothing.
            sleeping: body.sleeping,
            slept_at: body.slept_at,
            ..Player::default()
        };
    }

    /// The other half: you wake up naked **on your own bag if you asked for
    /// one and one of yours is ready**, and on a different beach otherwise.
    ///
    /// **Where that body stands is the difference between building a base
    /// and having one.** Before this, `deploy.rs` placed sleeping bags,
    /// capped them per owner, hashed them into the WAL and decayed them,
    /// and nothing anywhere read one at a death: every death evicted a
    /// player to a blind ring point with no map and no compass to walk
    /// back by, so every session ended where its first death landed (the
    /// merge-gate judge's ranked gap 1,
    /// `findings/archive-prestamp/pass-20260803-064506-04-judge.md`).
    ///
    /// The bag is asked first and the ring is the fallback, which is also
    /// the order of the two `dev_spawn` obeys: `dev_spawn` pins the *ring*
    /// (its doc says so, and `browser_smoke` uses it to put two tabs on
    /// one beach), and a bag is not the ring. A shard pinned for testing
    /// still honours a bag its player placed, which is the behaviour the
    /// player would report a bug about otherwise.
    ///
    /// The craft queue and the weak-spot chase are destroyed at the death
    /// and stay destroyed here, deliberately: a queued craft is a promise
    /// to a body that no longer exists, and its inputs were already spent
    /// when it was queued — refunding them into the bag would pay the
    /// killer twice for one farm. Only carried items drop, which is what
    /// DESIGN.md §2 says.
    ///
    /// Content that never armed the ladder (`base_ticks == 0`) still
    /// destroys the inventory outright, which is what this did before the
    /// backpack existed — an inert table can add a rule but must never
    /// silently change one.
    ///
    /// The input frame survives on purpose: it is the client's, not the
    /// world's, and resetting `seq` would lie to prediction about which
    /// input the sim last executed.
    fn wake(&mut self, slot: usize, on_bag: bool) {
        let body = self.players[slot];
        let (id, deaths, frame) = (body.id, body.deaths, body.frame);
        // Nearest own ready bag to where the body fell; the scan spends it
        // for `BAG_COOLDOWN_TICKS`, so a chain of deaths inside one
        // cooldown walks the player's other bags and then the ring. Asked
        // only when the player asked: a bag the beach button did not want
        // must not be spent, or the choice would cost the same either way.
        let bag = if on_bag {
            self.deploys.claim_bag(
                &self.deploy,
                id,
                body.body.qx as f32 * movement::POS_XZ_Q,
                body.body.qz as f32 * movement::POS_XZ_Q,
                self.tick,
            )
        } else {
            None
        };
        let (x, z) = match bag {
            Some(p) => p,
            None => self.spawn_pos_n(id, deaths as u32),
        };
        let hp = self.combat.player_hp;
        self.players[slot] = Player {
            id,
            active: true,
            body: Body::at(self.seed, x, z),
            frame,
            hp,
            hp_max: hp,
            deaths,
            ..Player::default()
        };
        // A player who starved does not respawn already starving.
        survival::grant(&self.survival, &mut self.players[slot]);
        self.events.push(EV_RESPAWN, id, bag.is_some() as u32, 0);
        self.events.push(EV_HEALTH, id, hp as u32, hp as u32);
        survival::announce_vitals(&self.survival, &self.players[slot], &mut self.events);
    }

    /// **The one door into the world for a player**, and there are two
    /// commands behind it: `Join` seats a fresh character, `JoinAs` seats a
    /// saved one. One function rather than two arms, because a second
    /// player-creation path is how the two drift — a field added to the
    /// fresh spawn and forgotten on the restore is a bug no gate here can
    /// see, since both produce a legal `Player`.
    ///
    /// A restore writes the saved fields and **defaults everything the save
    /// does not carry** (`persist.rs` says which and why): the input frame,
    /// the swing cooldown, the weak-spot chase and the craft timer are all
    /// `Player::default()`'s, and the craft timer is then re-armed against
    /// the *current* tick, because the queue survives a restart and its
    /// absolute completion tick cannot.
    fn seat(&mut self, id: u32, save: Option<PlayerSave>) {
        if self.slot_of(id).is_some() {
            return;
        }
        // A genuinely empty slot, or a silent refusal. Eviction is no
        // longer this function's call: it used to take the longest-asleep
        // sleeper on its own authority here, which was right for the world
        // and wrong for persistence — the victim's record was frozen at
        // their leave, so a sleeper raided and then evicted came back from
        // the stale record (`reference/SAVES.md` §9.2's one remaining
        // hole). The server now picks the victim, takes a current save off
        // the live body, and queues `Command::Evict` *ahead of* the join
        // that needs the slot (`ShardCore::connect_as`) — so by the time a
        // legitimate full-shard join reaches this line, the slot is free.
        //
        // A join with no free slot and no eviction ahead of it is refused
        // silently, exactly as a shard full of awake players always was:
        // the accept path already hard-caps connections at the shard
        // limit, so this is a WAL replayed against a diverged world or a
        // server that mis-counted, and neither may seat a body over one
        // that is standing.
        let Some(slot) = self.players.iter().position(|p| !p.active) else {
            return;
        };
        match save {
            None => {
                let (x, z) = self.spawn_pos(id);
                let hp = self.combat.player_hp;
                self.players[slot] = Player {
                    id,
                    active: true,
                    body: Body::at(self.seed, x, z),
                    hp,
                    hp_max: hp,
                    ..Player::default()
                };
                survival::grant(&self.survival, &mut self.players[slot]);
                // The spawn kit, on the FRESH arm only. A restore keeps what
                // it saved; re-granting on every login would be an item
                // printer, which is the same reason `survival::grant` is
                // here and not below the match.
                inventory::grant_kit(&self.spawn_kit, &mut self.players[slot]);
            }
            Some(s) => {
                self.players[slot] = Player {
                    id,
                    active: true,
                    body: s.body,
                    inv: s.inv,
                    jobs: s.jobs,
                    hp: s.hp,
                    hp_max: s.hp_max,
                    deaths: s.deaths,
                    food: s.food,
                    water: s.water,
                    food_acc: s.food_acc,
                    water_acc: s.water_acc,
                    hurt_acc: s.hurt_acc,
                    heal_rem: s.heal_rem,
                    heal_total: s.heal_total,
                    heal_span: s.heal_span,
                    heal_acc: s.heal_acc,
                    ..Player::default()
                };
                craft::rearm(&self.craft, self.tick, &mut self.players[slot]);
                if s.dead {
                    // Logged off on the death screen. Declining the choice
                    // is choosing the beach — `wake` re-derives the whole
                    // body from the spawn ring at `deaths` and announces
                    // everything this function would have, so it is the
                    // exit and not a step (`PlayerSave::dead` says why the
                    // corpse itself is not restorable).
                    self.wake(slot, false);
                    return;
                }
            }
        }
        // Say it at the door. Health is only ever announced when it
        // changes, so without this a player has no vitals until the first
        // thing that hurts them — which is the one moment a bar is no use.
        // The meters are announced at the door for the same reason.
        let (hp, hp_max) = (self.players[slot].hp, self.players[slot].hp_max);
        if hp > 0 {
            self.events.push(EV_HEALTH, id, hp as u32, hp_max as u32);
        }
        survival::announce_vitals(&self.survival, &self.players[slot], &mut self.events);
    }

    /// The slot holding the sleeping body `id`, or `None` — the sleeper
    /// was evicted, killed into a slot somebody else took, or never
    /// existed. A pure read, and the server's whole test before it commits
    /// to `Command::Wake` rather than `JoinAs`.
    pub fn sleeper_slot(&self, id: u32) -> Option<usize> {
        self.slot_of(id).filter(|&s| self.players[s].sleeping)
    }

    /// Whether a sleeping body by this id is still in the world.
    pub fn is_sleeper(&self, id: u32) -> bool {
        self.sleeper_slot(id).is_some()
    }

    /// How many bodies are asleep right now — the population the eviction
    /// policy is drawn from, and a stat the server publishes.
    pub fn sleepers(&self) -> usize {
        self.players
            .iter()
            .filter(|p| p.active && p.sleeping)
            .count()
    }

    /// Seat a returning connection onto the body it left behind.
    ///
    /// **The body wins over the record, and that is the point of the
    /// slice.** A player whose sleeper was killed while they were away
    /// comes back to a dead body, not to the state their leave-save
    /// recorded — restoring the record here is precisely the hole that
    /// would make offline raiding pay nothing.
    ///
    /// A dead sleeper wakes on a beach rather than on the death screen,
    /// which is the rule `PlayerSave::dead` already reasoned out for the
    /// other door and this one inherits: the screen is drawn from
    /// `EV_DEATH`, `EV_DEATH` is the shard-wide kill feed, and re-emitting
    /// it at a join would announce a fresh killing of somebody who had just
    /// logged in. Declining the choice is choosing the beach, and being
    /// killed in your sleep is declining it.
    fn take_over(&mut self, id: u32, sleeper: u32) {
        let Some(slot) = self.sleeper_slot(sleeper) else {
            return;
        };
        // Refuse only if `id` already names a **different** body. The
        // obvious guard — "refuse if this id is in the world" — is wrong,
        // and wrong in a case that is not exotic: **`id == sleeper` is the
        // ordinary path after a restart.** A player id is
        // `generation << 8 | slot`, and a restart resets the slot table, so
        // the first connection into slot 0 is minted the same id the body
        // saved in slot 0 already carries. That guard turned every
        // first-reconnect-after-a-restart into a silent no-op: the takeover
        // was counted, the wake command was queued, and the body stayed
        // asleep while the player sat in an empty world.
        //
        // Found by `server/tests/world_persist.rs`, which is the only test
        // that has both halves — a saved world and a fresh id space — and
        // could not have been found by either alone.
        if self.slot_of(id).is_some_and(|other| other != slot) {
            return;
        }
        let p = &mut self.players[slot];
        p.id = id;
        p.sleeping = false;
        p.slept_at = 0;
        // The frame is the new connection's to fill. Unlike a respawn —
        // which keeps it, because the same client is still holding the same
        // mouse — a takeover is a different session whose input seq starts
        // at 0, and a stale `seq` would tell prediction the sim had already
        // executed inputs this client has not sent (`persist.rs` says the
        // same thing about restoring one from a file).
        p.frame = InputFrame::default();
        if self.players[slot].dead {
            self.wake(slot, false);
            return;
        }
        // The craft queue survived the sleep; its completion tick did not
        // survive being an absolute number in a world that kept ticking
        // without the player. Re-armed against now, exactly as `JoinAs`
        // does — one rule, two doors.
        craft::rearm(&self.craft, self.tick, &mut self.players[slot]);
        let (hp, hp_max) = (self.players[slot].hp, self.players[slot].hp_max);
        if hp > 0 {
            self.events.push(EV_HEALTH, id, hp as u32, hp_max as u32);
        }
        survival::announce_vitals(&self.survival, &self.players[slot], &mut self.events);
    }

    /// The world-side half of two-phase eviction: remove the sleeping body
    /// `id`, exactly as `seat`'s own-authority eviction used to — the slot
    /// goes inactive and the eviction is counted (`evictions` is hashed, so
    /// two shards that evicted different bodies diverge loudly). Everything
    /// else in the record is residue the next `seat` overwrites, invisible
    /// until then because every read of a player — `state_hash` included —
    /// filters on `active` first.
    ///
    /// Only a **sleeping** body: the server picks victims from the sleeper
    /// population, and a replayed stream whose world diverged must refuse
    /// to delete a body somebody is driving (`Command::Evict` says why a
    /// miss is legal).
    fn evict(&mut self, id: u32) {
        let Some(slot) = self.sleeper_slot(id) else {
            return;
        };
        self.players[slot].active = false;
        self.evictions += 1;
    }

    /// Re-apply every door's shut bit to the collision index, after a world
    /// load has replaced both stores.
    ///
    /// **Doors are the seam between the two stores, and the only piece of
    /// derived state a load can get wrong quietly.** A door is a
    /// *deployable* record carrying `open`; what it blocks is a *piece* —
    /// the doorway it was placed in — whose closed-ness lives as a bit in
    /// `Pieces::cols`. `Pieces::restore` rebuilds that index from the piece
    /// records alone and cannot know about doors, so without this pass every
    /// door on the shard comes back walkable while the wire still draws it
    /// shut: a raid that costs nothing, visible to any player, invisible to
    /// `state_hash` (the index is never hashed) and unreachable by any gate
    /// that only compares two runs of the same binary.
    ///
    /// A door places closed, so the bit is `!open` and not `open` — the
    /// same expression `deploy.rs`'s use-toggle writes, deliberately
    /// duplicated rather than shared, because the toggle owns *when* and
    /// this owns *from what*.
    /// The lock's two mirror bits are re-derived in the same pass and for
    /// the same reason (lock v1). `DeployRec::has_lock` and
    /// `DeployRec::locked` are a *view* of `lock.rs`'s store, and a saved
    /// view is a saved cache: the file could carry a locked door whose
    /// lock is not in the file's lock section, and the shard would draw a
    /// locked door that the access check waves everyone through. Deriving
    /// them here means the store is the only thing the save has to get
    /// right, which is the same argument that keeps `Pieces::cols` out of
    /// the file entirely. Every lockable archetype gets the derivation —
    /// a **box** carries the same lock as a door (locks on boxes,
    /// `DOORS.md` §9.8); only the door touches the collision index,
    /// because only a door has a leaf that blocks.
    pub fn rebuild_doors(&mut self) {
        for i in 0..self.deploys.len() {
            let d = self.deploys.entries()[i];
            let arch = self.deploy.defs[d.row as usize].arch;
            if arch == crate::deploy::ARCH_DOOR {
                self.pieces.set_door(d.cx, d.cz, d.level, d.loc, !d.open);
            }
            if !crate::deploy::lockable(arch) {
                continue;
            }
            let lock = self
                .deploys
                .locks()
                .iter()
                .find(|l| l.cx == d.cx && l.cz == d.cz && l.level == d.level && l.loc == d.loc)
                .copied();
            self.deploys
                .set_lock_mirror(i, lock.is_some(), lock.is_some_and(|l| l.locked));
        }
    }

    /// Load a world from a save blob — **the boot path, and only the boot
    /// path** (`worldsave.rs` has the argument in full).
    ///
    /// Call after the content tables are installed and before the first
    /// `tick`. The loaded world is the *origin* of a run, not a mutation
    /// inside one, which is what keeps wall 5 intact without the state
    /// having to ride a command the way `Command::JoinAs` does: there is no
    /// stream yet for it to be inconsistent with.
    ///
    /// On refusal the world is untouched — every field is decoded and
    /// checked before anything is written — so a shard whose save is
    /// corrupt starts a fresh world rather than half of somebody's base.
    pub fn load(&mut self, blob: &[u8]) -> Result<(), crate::worldsave::WorldSaveError> {
        crate::worldsave::decode_into(self, blob)
    }

    /// This world as a save blob, into a caller-owned buffer at least
    /// [`crate::worldsave::WORLD_SAVE_MAX_BYTES`] long. A pure read, for
    /// the reason [`Self::save_of`] is one.
    pub fn save_world(&self, out: &mut [u8]) -> Result<usize, crate::worldsave::WorldSaveError> {
        crate::worldsave::encode(self, out)
    }

    /// What this shard would remember about `id` if the connection ended
    /// now. `None` ⇒ nobody by that id is in the world.
    ///
    /// **A pure read, and that is the whole design.** The server takes one
    /// at a leave, at a shutdown and on its autosave sweep, and none of
    /// those is a command — so none of them may change the world, or a
    /// replay of the same WAL would diverge from the shard that wrote it
    /// (wall 5). Everything that mutates stays on the `Command` path; this
    /// only looks.
    pub fn save_of(&self, id: u32) -> Option<PlayerSave> {
        self.slot_of(id).map(|s| PlayerSave::of(&self.players[s]))
    }

    fn apply(&mut self, cmd: &Command, removals: &mut usize) {
        match *cmd {
            Command::Join { id } => self.seat(id, None),
            Command::JoinAs { id, save } => self.seat(id, Some(save)),
            Command::Leave { id } => {
                // A second `Leave` for a body already asleep is a no-op: it
                // must not restamp `slept_at`, or a duplicate would move a
                // sleeper to the back of the eviction queue.
                if let Some(slot) = self.slot_of(id).filter(|&s| !self.players[s].sleeping) {
                    let now = self.tick;
                    let p = &mut self.players[slot];
                    p.sleeping = true;
                    p.slept_at = now;
                    // The last input this body was carrying is dropped down
                    // to its facing. The step below zeroes a sleeper's frame
                    // anyway, so this changes no motion — what it changes is
                    // what the *state* says: a body nobody is driving must
                    // not be recorded as still holding W, or every hash of
                    // it reads as a player mid-sprint.
                    p.frame = InputFrame {
                        seq: p.frame.seq,
                        yaw: p.frame.yaw,
                        pitch: p.frame.pitch,
                        sel: p.frame.sel,
                        ..InputFrame::default()
                    };
                }
            }
            Command::Wake { id, sleeper } => self.take_over(id, sleeper),
            Command::Evict { id } => self.evict(id),
            Command::Input { id, frame } => {
                if let Some(slot) = self.slot_of(id) {
                    let mut frame = frame;
                    if frame.sel as usize >= HOTBAR_SLOTS {
                        // The wire refuses 6–7 at decode; a non-wire
                        // command (bot, test, WAL) falls back to slot 0.
                        frame.sel = 0;
                    }
                    // The wire refuses unknown button bits at the server's
                    // accept boundary (net.rs `accept_input`); a non-wire
                    // command is masked instead — `sel`'s rule, applied to
                    // bits. The stored frame is hashed, so a bit no verb
                    // reads must never reach it (NOW.md §5b).
                    frame.buttons &= crate::input::BTN_MASK;
                    self.players[slot].frame = frame;
                }
            }
            Command::Craft { id, recipe, count } => {
                if let Some(slot) = self.live_slot_of(id) {
                    craft::enqueue(
                        &self.craft,
                        &self.deploy,
                        &self.deploys,
                        self.tick,
                        &mut self.players[slot],
                        recipe,
                        count,
                        &mut self.events,
                    );
                }
            }
            Command::Research { id, slot } => {
                if let Some(s) = self.live_slot_of(id) {
                    crate::research::research(
                        &self.research,
                        &self.deploy,
                        &self.deploys,
                        &mut self.players[s],
                        slot,
                        &mut self.events,
                    );
                }
            }
            Command::CraftCancel { id, index } => {
                if let Some(slot) = self.live_slot_of(id) {
                    craft::cancel(
                        &self.craft,
                        &self.gather,
                        self.tick,
                        &mut self.players[slot],
                        index,
                    );
                }
            }
            Command::Place {
                id,
                row,
                cx,
                cz,
                level,
                loc,
            } => {
                if let Some(slot) = self.live_slot_of(id) {
                    build::place(
                        self.seed,
                        &self.build,
                        &self.deploys,
                        &mut self.pieces,
                        &mut self.players[slot],
                        self.tick,
                        row,
                        cx,
                        cz,
                        level,
                        loc,
                        &mut self.events,
                    );
                }
            }
            Command::PlaceDeploy {
                id,
                row,
                cx,
                cz,
                level,
                loc,
            } => {
                if let Some(slot) = self.live_slot_of(id) {
                    deploy::place_deploy(
                        self.seed,
                        &self.deploy,
                        &self.build,
                        &mut self.pieces,
                        &mut self.deploys,
                        &mut self.players[slot],
                        self.tick,
                        row,
                        cx,
                        cz,
                        level,
                        loc,
                        &mut self.events,
                    );
                }
            }
            Command::Feed { id, cx, cz, level } => {
                if let Some(slot) = self.live_slot_of(id) {
                    deploy::feed(
                        &self.deploy,
                        &mut self.deploys,
                        &mut self.players[slot],
                        cx,
                        cz,
                        level,
                        &mut self.events,
                    );
                }
            }
            Command::Use {
                id,
                cx,
                cz,
                level,
                loc,
            } => {
                if let Some(slot) = self.live_slot_of(id) {
                    // One key, two verbs, picked by what stands at the
                    // address — the reference's own E menu, where a door
                    // offers open/close and a fire offers ignite/
                    // extinguish. The two can never collide: a door lives
                    // on a doorway's edge address and an oven on the
                    // plane, so this is a lookup and not a guess about
                    // what the player aimed at.
                    let lit = crate::oven::toggle(
                        &self.cook,
                        &mut self.deploys,
                        &self.players[slot],
                        cx,
                        cz,
                        level,
                        &mut self.events,
                    );
                    if !lit {
                        deploy::use_door(
                            &self.deploy,
                            &mut self.pieces,
                            &mut self.deploys,
                            &mut self.players[slot],
                            cx,
                            cz,
                            level,
                            loc,
                            &mut self.events,
                        );
                    }
                }
            }
            Command::Demolish {
                id,
                deploy,
                cx,
                cz,
                level,
                loc,
            } => {
                if let Some(slot) = self.live_slot_of(id) {
                    if deploy {
                        deploy::pick_up(
                            &self.deploy,
                            &self.gather,
                            &mut self.pieces,
                            &mut self.deploys,
                            &mut self.players[slot],
                            cx,
                            cz,
                            level,
                            loc,
                            &mut self.events,
                        );
                    } else {
                        build::demolish(
                            &self.deploy,
                            &self.build,
                            &self.gather,
                            &mut self.pieces,
                            &mut self.deploys,
                            &mut self.players[slot],
                            self.tick,
                            cx,
                            cz,
                            level,
                            loc,
                            removals,
                            &mut self.events,
                        );
                    }
                }
            }
            Command::Access {
                id,
                cx,
                cz,
                level,
                loc,
                op,
                code,
            } => {
                if let Some(slot) = self.live_slot_of(id) {
                    // The one branch: a crew op addresses the hearth on
                    // the cell body, every other op addresses the lock on
                    // a door's edge. `deploy::op_is_crew` is the split,
                    // written once so the wire's range check and this
                    // cannot disagree about which store an op means.
                    if deploy::op_is_crew(op) {
                        deploy::crew_op(
                            &mut self.deploys,
                            &self.players[slot],
                            cx,
                            cz,
                            level,
                            op,
                            &mut self.events,
                        );
                    } else {
                        deploy::lock_op(
                            &self.deploy,
                            &self.gather,
                            &mut self.deploys,
                            &mut self.players[slot],
                            cx,
                            cz,
                            level,
                            loc,
                            op,
                            code,
                            self.tick,
                            &mut self.events,
                        );
                    }
                }
            }
            Command::Upgrade {
                id,
                cx,
                cz,
                level,
                loc,
                material,
            } => {
                if let Some(slot) = self.live_slot_of(id) {
                    build::upgrade(
                        &self.build,
                        &self.deploys,
                        &mut self.pieces,
                        &mut self.players[slot],
                        cx,
                        cz,
                        level,
                        loc,
                        material,
                        &mut self.events,
                    );
                }
            }
            Command::Repair {
                id,
                deploy,
                cx,
                cz,
                level,
                loc,
            } => {
                if let Some(slot) = self.live_slot_of(id) {
                    build::repair(
                        &self.build,
                        &self.deploy,
                        &mut self.deploys,
                        &mut self.pieces,
                        &mut self.players[slot],
                        deploy,
                        cx,
                        cz,
                        level,
                        loc,
                        &mut self.events,
                    );
                }
            }
            Command::Throw {
                id,
                deploy,
                cx,
                cz,
                level,
                loc,
            } => {
                if let Some(slot) = self.live_slot_of(id) {
                    crate::charge::place(
                        &self.build,
                        &self.deploy,
                        &self.combat,
                        &mut self.charges,
                        &self.deploys,
                        &self.pieces,
                        &mut self.players[slot],
                        self.tick,
                        deploy,
                        cx,
                        cz,
                        level,
                        loc,
                        &mut self.events,
                    );
                }
            }
            Command::Consume { id, slot: inv } => {
                if let Some(slot) = self.live_slot_of(id) {
                    survival::consume(
                        &self.survival,
                        inv as usize,
                        &mut self.players[slot],
                        &mut self.events,
                    );
                }
            }
            Command::Drink { id } => {
                if let Some(slot) = self.live_slot_of(id) {
                    // The one verb that can kill the player who pressed it.
                    // Handled exactly as a clock death and a combat death
                    // are — the callee counts it and announces it, the
                    // caller lays the body down — because `die` needs the
                    // whole world and the verb needs one player.
                    let seed = self.seed;
                    let id = self.players[slot].id;
                    if survival::drink(
                        &self.survival,
                        seed,
                        &mut self.players[slot],
                        &mut self.events,
                    ) == survival::Step::Died
                    {
                        self.die(slot, id, DEATH_BY_SALT, NO_ITEM, 0);
                    }
                }
            }
            Command::Loot { id } => {
                if let Some(slot) = self.live_slot_of(id) {
                    self.backpacks.loot_nearest(
                        &self.gather,
                        &mut self.players[slot],
                        &mut self.events,
                    );
                }
            }
            Command::Move {
                id,
                cont,
                from_kind,
                from_slot,
                to_kind,
                to_slot,
                count,
            } => {
                if let Some(slot) = self.live_slot_of(id) {
                    self.move_item(slot, cont, from_kind, from_slot, to_kind, to_slot, count);
                }
            }
            Command::Respawn { id, on_bag } => {
                // `slot_of`, not `live_slot_of`: this is the one verb only
                // a corpse may send, and the `dead` test below is the whole
                // of its authority. A press from a standing body — a
                // duplicate, a forged one, a second click on a screen that
                // already closed — does nothing at all rather than moving a
                // live player to a beach.
                if let Some(slot) = self.slot_of(id) {
                    if self.players[slot].dead {
                        self.wake(slot, on_bag);
                    }
                }
            }
        }
    }

    /// One fixed tick: apply at most `MAX_COMMANDS_PER_TICK` commands in
    /// order (overflow policy: defer — the caller keeps the tail), step
    /// every active player in slot order (move, then swing), release due
    /// respawns, stamp the hash on cadence.
    pub fn tick(&mut self, commands: &[Command]) {
        self.events.clear();
        // The tick's structural removal budget is minted **before** the
        // commands rather than after them, because since demolish v1 a
        // command can take a piece out of the store and seed a cascade —
        // so the verbs and the sweep spend one allowance between them.
        // Two budgets would be two caps and therefore no cap.
        let mut removals = MAX_REMOVALS_PER_TICK;
        for cmd in commands.iter().take(MAX_COMMANDS_PER_TICK) {
            self.apply(cmd, &mut removals);
        }
        let seed = self.seed;
        let tick = self.tick;
        // The tick's structural removal budget, spent by every path that
        // takes a piece out of the store — a raider's killing blow below,
        // the decay sweep, the support backstop, and every cascade they
        // seed. A tick-local, reset here with the ring it protects: a
        // removal that finds it empty is deferred to a later tick, never
        // dropped, so this bounds latency and not what comes down.
        //
        // Deliberately not a `World` field. It is spent and forgotten
        // inside one tick exactly as `events` is, and a store that lives
        // across ticks is a store `state_hash` has to answer for. Minted
        // at the top of `tick` since demolish v1 — see there.
        // Slot order, and inside a slot: move, swing, craft. The swing is
        // one arm — `gather::swing` gets first claim on it (a tree in
        // reach is always the nearer target) and hands it on only when
        // nothing standing absorbed it.
        for i in 0..MAX_PLAYERS {
            if !self.players[i].active {
                continue;
            }
            // A body on the death screen is still a body: it falls and it
            // settles, so a player killed off a roof lands on the ground
            // the screen's next respawn will not be measured from. What it
            // does not do is act — the frame is zeroed rather than the step
            // skipped, and that ordering is what keeps the client's own
            // predictor in agreement about a corpse it can still see.
            if self.players[i].dead {
                let frame = InputFrame {
                    seq: self.players[i].frame.seq,
                    yaw: self.players[i].frame.yaw,
                    pitch: self.players[i].frame.pitch,
                    sel: self.players[i].frame.sel,
                    ..InputFrame::default()
                };
                movement::step(
                    seed,
                    self.pieces.cols(),
                    &mut crate::occupy::Occupants {
                        table: &self.scatter,
                        haven: &self.haven,
                        harvested: &self.slot_lives,
                        cache: &mut self.slot_cache,
                    },
                    &mut self.players[i].body,
                    &frame,
                );
                continue;
            }
            // A sleeper: alive, in the world, and driving nothing.
            //
            // The clock runs — that is what "keeps its metabolism" costs,
            // and it is the reason logging off is no longer free: a body
            // left standing long enough starves, dies where it stands and
            // drops its bag through the same `die` every other death takes.
            // What it does not do is act, so the arm, the craft queue and
            // the bow are all skipped and the frame handed to `movement` is
            // zeroed rather than the step skipped — the same shape the
            // death screen above uses, and for the same reason: a body that
            // stopped falling when its owner disconnected would hang in the
            // air over a base that decayed out from under it.
            if self.players[i].sleeping {
                if survival::step(&self.survival, &mut self.players[i], &mut self.events)
                    == survival::Step::Died
                {
                    let id = self.players[i].id;
                    self.die(i, id, DEATH_BY_CLOCK, NO_ITEM, 0);
                    continue;
                }
                let frame = InputFrame {
                    seq: self.players[i].frame.seq,
                    yaw: self.players[i].frame.yaw,
                    pitch: self.players[i].frame.pitch,
                    sel: self.players[i].frame.sel,
                    ..InputFrame::default()
                };
                movement::step(
                    seed,
                    self.pieces.cols(),
                    &mut crate::occupy::Occupants {
                        table: &self.scatter,
                        haven: &self.haven,
                        harvested: &self.slot_lives,
                        cache: &mut self.slot_cache,
                    },
                    &mut self.players[i].body,
                    &frame,
                );
                continue;
            }
            // The clock runs before the arm. A body that starves this tick
            // does not also get to swing on it — and running the clock
            // first is what makes the death below the tick's last word
            // about this slot, exactly as a combat death is.
            if survival::step(&self.survival, &mut self.players[i], &mut self.events)
                == survival::Step::Died
            {
                let id = self.players[i].id;
                self.die(i, id, DEATH_BY_CLOCK, NO_ITEM, 0);
                continue;
            }
            let frame = self.players[i].frame;
            movement::step(
                seed,
                self.pieces.cols(),
                &mut crate::occupy::Occupants {
                    table: &self.scatter,
                    haven: &self.haven,
                    harvested: &self.slot_lives,
                    cache: &mut self.slot_cache,
                },
                &mut self.players[i].body,
                &frame,
            );
            // A drawn bow takes the arm before the gather scan sees it.
            // `gather::swing` searches the 3×3 cell ring for a node and
            // absorbs the swing into it, which is precisely what would eat
            // an archer's shot for the crime of standing next to a tree —
            // and standing next to a tree is where an archer stands. The
            // bow answers first or it does not work at all.
            let swung = if ranged::draw(tick, &self.combat, &mut self.arrows, &mut self.players[i])
            {
                gather::Swing::Absorbed
            } else {
                gather::swing(
                    seed,
                    tick,
                    &self.gather,
                    &self.loot,
                    &self.scatter,
                    &self.haven,
                    &mut self.slot_lives,
                    &mut self.events,
                    &mut self.players[i],
                )
            };
            craft::step(
                &self.craft,
                &self.gather,
                tick,
                &mut self.players[i],
                &mut self.events,
            );
            if let Swing::Smashed { cx, cz, qx, qy, qz } = swung {
                // The barrel is already gone (gather.rs marked the slot and
                // announced it). What falls out is decided here, because
                // this is where the container store lives: gather owns the
                // slot bit, loot owns the table, and neither owns the other.
                //
                // An empty roll stands nothing up — `stand_up` refuses it —
                // and that is correct rather than a lost drop: the barrel
                // still broke, still respawns on its timer, and the player
                // still paid three swings for a bad table.
                let mut items = [ItemStack::default(); INV_SLOTS];
                self.loot.roll_into(
                    LOOT_BARREL,
                    &self.gather,
                    seed,
                    cell_key(cx, cz),
                    tick,
                    &mut items,
                );
                let owner = self.players[i].id;
                self.backpacks.stand_up(
                    &self.backpack,
                    qx,
                    qy,
                    qz,
                    owner,
                    &items,
                    tick,
                    &mut self.events,
                );
            }
            if swung == Swing::Free {
                // node → player → structure: the arm passes on only what
                // nothing nearer absorbed.
                match combat::strike(&self.combat, i, &mut self.players, &mut self.events) {
                    combat::Strike::Killed {
                        victim,
                        item,
                        range_cm,
                    } => {
                        let by = self.players[i].id;
                        self.die(victim, by, DEATH_BY_HAND, item, range_cm);
                    }
                    combat::Strike::Hit => {}
                    combat::Strike::Missed => {
                        // node → player → **animal** → structure. An
                        // animal outranks the wall behind it and never
                        // outranks a player: standing between a raider and
                        // a door must not become a way to eat the swing.
                        let took = mob::strike(
                            &self.combat,
                            &self.backpack,
                            &self.mob,
                            tick,
                            i,
                            &self.players,
                            &mut self.mobs,
                            &mut self.backpacks,
                            &mut self.events,
                        );
                        if !took {
                            combat::raid(
                                &self.combat,
                                &self.build,
                                &self.deploy,
                                seed,
                                &self.players[i],
                                &mut self.pieces,
                                &mut self.deploys,
                                &mut removals,
                                &mut self.events,
                            );
                        }
                    }
                }
            }
        }
        // Fuses burn after the player loop that can light one and before
        // the sweeps that clean up after a collapse. A charge planted this
        // tick cannot fire on it — `validate` refuses `fuse_s = 0` — so
        // the ordering never lets a plant and its blast land in one tick's
        // event ring, which is what would make the fuse invisible.
        //
        // `removals` is the same allowance the swings above just spent:
        // wall 4 does not hand out a second one because the damage arrived
        // on a timer.
        crate::charge::tick_fuses(
            &self.build,
            &self.deploy,
            &mut self.charges,
            &mut self.pieces,
            &mut self.deploys,
            tick,
            &mut removals,
            &mut self.events,
        );

        // The roster steps after the player loop and before the arrows, and
        // both sides of that are deliberate. After the players, because an
        // animal reads player positions to decide whether it is awake and
        // which way to run, and reading them mid-loop would make the answer
        // depend on the reader's slot index. Before the arrows, because a
        // shot must resolve against where the animal ended this tick — the
        // same rule the player loop's ordering states in the comment above.
        mob::step(
            seed,
            tick,
            &self.mob,
            self.pieces.cols(),
            &mut crate::occupy::Occupants {
                table: &self.scatter,
                haven: &self.haven,
                harvested: &self.slot_lives,
                cache: &mut self.slot_cache,
            },
            &mut self.mobs,
            &self.players,
        );

        // Arrows fly after the player loop, never inside it. Two reasons,
        // both structural: every body has already taken its step, so a shot
        // resolves against final positions instead of positions that are
        // final for the low slots and stale for the high ones; and nothing
        // about a hit may depend on the shooter's slot index, which it
        // would if flight ran while the loop still held one player
        // mutably. `removals` is not spent here — an arrow does not chip a
        // wall in v0, it stops on one.
        let mut kills = [ranged::Kill::default(); MAX_ARROWS];
        let n_kills = ranged::step(
            seed,
            self.pieces.cols(),
            &mut crate::occupy::Occupants {
                table: &self.scatter,
                haven: &self.haven,
                harvested: &self.slot_lives,
                cache: &mut self.slot_cache,
            },
            &mut self.arrows,
            &mut self.players,
            &mut self.events,
            &mut kills,
        );
        for k in kills.iter().take(n_kills) {
            self.die(k.victim, k.by, DEATH_BY_ARROW, k.item, k.range_cm);
        }
        self.slot_lives.respawn_due(tick, &mut self.events);
        // Bags time out on the sim's clock, before the tick advances, so
        // a bag dropped at tick T with a lifetime of L is gone the tick
        // its own `expires` names and not one later.
        self.backpacks.expire_due(tick, &mut self.events);
        deploy::upkeep_sweep(
            &self.deploy,
            &self.build,
            &mut self.pieces,
            &mut self.deploys,
            tick,
            &mut self.sweep_piece,
            &mut self.sweep_deploy,
            &mut removals,
            &mut self.events,
        );
        // Every oven whose turn this tick is: fuel down, byproduct banked,
        // what is on the fire a step closer to done (oven.rs). Before the
        // sweeps that can remove the thing it is stepping — an oven that
        // decays this tick has already spent its period, which is the
        // ordering a raid and a decay have to agree on.
        crate::oven::sweep(
            &self.cook,
            &self.gather,
            &mut self.deploys,
            tick,
            &mut self.events,
        );
        // The structural backstop, after the sweep that can create work for
        // it: anything a capped cascade left hanging in the air comes down
        // here, one piece and its own cascade per tick (build.rs).
        build::support_sweep(
            &self.deploy,
            &self.build,
            &mut self.pieces,
            &mut self.deploys,
            &mut self.sweep_support,
            &mut removals,
            &mut self.events,
        );
        // Boxes that came apart this tick — raided in the player loop
        // above, or decayed by the sweep just now — empty onto the floor
        // they stood on. Drained here rather than at the removal because
        // that path (`deploy::drop_deploy`) holds neither the bag store
        // nor the clock, and because doing it last means one bag per box
        // however the box died.
        //
        // The same `stand_up` a corpse and a barrel use, so a broken box
        // is looted by every route that already exists and the wire sees
        // one bag contract, not a third kind of litter. An empty box
        // parks nothing (`remove_at`), so this loop is dead code on a base
        // that decayed with nothing in it.
        for i in 0..self.deploys.box_spill_len() {
            let bx = self.deploys.box_spill_at(i);
            let (x, y, z) = deploy::box_drop_pos(seed, bx.cx, bx.cz, bx.level);
            let mut items = [ItemStack::default(); INV_SLOTS];
            items[..BOX_SLOTS].copy_from_slice(&bx.items);
            self.backpacks.stand_up(
                &self.backpack,
                quant_xz(x),
                quant_y(y),
                quant_xz(z),
                bx.owner,
                &items,
                tick,
                &mut self.events,
            );
        }
        self.deploys.clear_box_spill();
        self.tick += 1;
        if self.tick.is_multiple_of(STATE_HASH_INTERVAL) {
            self.last_hash = self.state_hash();
        }
    }

    /// xxh3 over canonical sim state, allocation-free. Slot order is the
    /// canonical order. `dev_spawn` and the baked `gather` table are
    /// construction input, not state — they influence the sim the way
    /// `seed` does, and pin alongside it (seed + content hash in the WAL
    /// header). The event ring is derived output and stays out.
    pub fn state_hash(&self) -> u64 {
        let mut h = Xxh3::new();
        h.update(&self.seed.to_le_bytes());
        h.update(&self.tick.to_le_bytes());
        for p in self.players.iter() {
            if !p.active {
                continue;
            }
            let mut buf = [0u8; 48];
            buf[0..4].copy_from_slice(&p.id.to_le_bytes());
            buf[4..8].copy_from_slice(&p.body.qx.to_le_bytes());
            buf[8..12].copy_from_slice(&p.body.qy.to_le_bytes());
            buf[12..16].copy_from_slice(&p.body.qz.to_le_bytes());
            buf[16..20].copy_from_slice(&p.body.qvy.to_le_bytes());
            buf[20] = p.body.grounded as u8;
            buf[21..23].copy_from_slice(&p.frame.seq.to_le_bytes());
            buf[23] = p.frame.buttons;
            buf[24..26].copy_from_slice(&p.frame.yaw.to_le_bytes());
            buf[26] = p.frame.pitch;
            buf[27] = p.frame.move_x as u8;
            buf[28] = p.frame.move_z as u8;
            buf[29..37].copy_from_slice(&p.next_swing.to_le_bytes());
            buf[37] = p.frame.sel;
            buf[38..42].copy_from_slice(&p.ws_cell.to_le_bytes());
            buf[42..44].copy_from_slice(&p.ws_hits.to_le_bytes());
            buf[44..46].copy_from_slice(&p.hp.to_le_bytes());
            buf[46..48].copy_from_slice(&p.deaths.to_le_bytes());
            h.update(&buf);
            // The survival clock, in its own buffer rather than widening
            // the one above — every byte of it is sim state (the
            // accumulators included: a replay resuming mid-span with a
            // zeroed remainder would drift, which is wall 5's whole point).
            let mut sv = [0u8; 24];
            sv[0..2].copy_from_slice(&p.hp_max.to_le_bytes());
            sv[2..4].copy_from_slice(&p.food.to_le_bytes());
            sv[4..6].copy_from_slice(&p.water.to_le_bytes());
            sv[6..10].copy_from_slice(&p.food_acc.to_le_bytes());
            sv[10..14].copy_from_slice(&p.water_acc.to_le_bytes());
            sv[14..18].copy_from_slice(&p.hurt_acc.to_le_bytes());
            sv[18..20].copy_from_slice(&p.heal_rem.to_le_bytes());
            sv[20..22].copy_from_slice(&p.heal_total.to_le_bytes());
            h.update(&sv);
            let mut hb = [0u8; 8];
            hb[0..4].copy_from_slice(&p.heal_span.to_le_bytes());
            hb[4..8].copy_from_slice(&p.heal_acc.to_le_bytes());
            h.update(&hb);
            // The death screen, in its own buffer for the survival clock's
            // reason: every byte is sim state. `dead` most obviously — two
            // shards that disagree about whether a body is standing
            // disagree about everything downstream of it — but the four
            // facts too, because the wire encodes them off this record and
            // a replay that reproduced the position while inventing the
            // weapon would put a different sentence on a player's screen.
            //
            // `sleeping` and `slept_at` ride the same buffer for the same
            // reason. The first decides whether this body has agency at
            // all, so two shards disagreeing about it disagree about every
            // tick after; the second decides which body an eviction takes,
            // so a shard that drifted on it would delete a different
            // player.
            let mut db = [0u8; 19];
            db[0..4].copy_from_slice(&p.death_by.to_le_bytes());
            db[4] = p.dead as u8;
            db[5] = p.death_cause;
            db[6..8].copy_from_slice(&p.death_item.to_le_bytes());
            db[8..10].copy_from_slice(&p.death_range_cm.to_le_bytes());
            db[10] = p.sleeping as u8;
            db[11..19].copy_from_slice(&p.slept_at.to_le_bytes());
            h.update(&db);
            for s in p.inv.iter() {
                let mut sb = [0u8; 4];
                sb[0..2].copy_from_slice(&s.item.to_le_bytes());
                sb[2..4].copy_from_slice(&s.count.to_le_bytes());
                h.update(&sb);
            }
            let mut cb = [0u8; 16 + CRAFT_QUEUE * 4];
            cb[0..8].copy_from_slice(&p.craft_done_at.to_le_bytes());
            // The blueprint mask (research v0). It belongs here for the
            // reason `[backpack]`'s ladder had to reach `canon::hash`, one
            // layer over: a `Command::Research` mutates it, and what it
            // changes is which craft requests the sim will honour from
            // then on. Two replays of one WAL that disagreed about a
            // player's blueprints would diverge on the first gated craft
            // — silently, because every other field still matched.
            cb[8..16].copy_from_slice(&p.known.to_le_bytes());
            for (j, job) in p.jobs.iter().enumerate() {
                cb[16 + j * 4..16 + j * 4 + 2].copy_from_slice(&job.recipe.to_le_bytes());
                cb[16 + j * 4 + 2..16 + j * 4 + 4].copy_from_slice(&job.remaining.to_le_bytes());
            }
            h.update(&cb);
        }
        h.update(&(self.slot_lives.len() as u64).to_le_bytes());
        for e in self.slot_lives.entries() {
            let mut buf = [0u8; 16];
            buf[0..2].copy_from_slice(&e.cx.to_le_bytes());
            buf[2..4].copy_from_slice(&e.cz.to_le_bytes());
            buf[4..6].copy_from_slice(&e.hits.to_le_bytes());
            buf[8..16].copy_from_slice(&e.respawn_at.to_le_bytes());
            h.update(&buf);
        }
        h.update(&(self.pieces.len() as u64).to_le_bytes());
        // The placement clocks, in their own pass rather than widening
        // the buffer below — they live in a parallel array precisely so
        // the wire's mirror of `PieceRec` does not carry them
        // (`build.rs`), and the digest follows the storage. State like
        // any other timer: two shards that disagree about when a wall
        // went up disagree about whether it can still be taken down.
        for t in self.pieces.placed() {
            h.update(&t.to_le_bytes());
        }
        for r in self.pieces.entries() {
            let mut buf = [0u8; 12];
            buf[0..2].copy_from_slice(&r.cx.to_le_bytes());
            buf[2..4].copy_from_slice(&r.cz.to_le_bytes());
            buf[4] = r.level;
            buf[5] = r.loc;
            buf[6] = r.row;
            buf[7..9].copy_from_slice(&r.hp.to_le_bytes());
            buf[9..11].copy_from_slice(&r.uh.to_le_bytes());
            h.update(&buf);
        }
        // Arrows in the air, and deliberately on the **player** idiom
        // rather than every store's above: skip-if-inactive, with no length
        // prefix. That is not a shortcut, it is the difference between a
        // structural addition that moves `GOLDEN_FINAL_HASH` and one that
        // does not. Every `h.update(&(store.len()))` here folds eight zero
        // bytes even when the store is empty — the box store did exactly
        // that once and `tests/replay.rs`'s doc comment records the eight
        // bytes it cost. An arrow store hashed this way contributes nothing
        // until an arrow is actually fired, so the pinned replay hash stays
        // evidence that this slice changed no path the script walks.
        //
        // Safe without a count for the reason `players` is: slot order is
        // allocation order, allocation is deterministic, so two runs that
        // agree about the shot agree about the index.
        for a in self.arrows.entries() {
            let mut buf = [0u8; 36];
            buf[0..4].copy_from_slice(&a.qx.to_le_bytes());
            buf[4..8].copy_from_slice(&a.qy.to_le_bytes());
            buf[8..12].copy_from_slice(&a.qz.to_le_bytes());
            buf[12..16].copy_from_slice(&a.vx.to_le_bytes());
            buf[16..20].copy_from_slice(&a.vy.to_le_bytes());
            buf[20..24].copy_from_slice(&a.vz.to_le_bytes());
            buf[24..26].copy_from_slice(&a.drop.to_le_bytes());
            buf[26..30].copy_from_slice(&a.owner.to_le_bytes());
            buf[30..32].copy_from_slice(&a.item.to_le_bytes());
            buf[32..34].copy_from_slice(&a.damage.to_le_bytes());
            buf[34..36].copy_from_slice(&a.life.to_le_bytes());
            h.update(&buf);
            h.update(&a.flown.to_le_bytes());
        }
        // The animal roster, on the arrow idiom above and for its reason:
        // **skip-if-not-alive, no length prefix**, so a world whose content
        // arms no species folds not one byte here and `GOLDEN_FINAL_HASH`
        // stays evidence about the script it pins rather than about this
        // slice landing.
        //
        // A slot's home is NOT hashed and that is the same call `haven`
        // makes one field over: it is a pure function of the seed,
        // recomputed identically by every build, so it is worldgen and not
        // state. What is hashed is everything a tick can move — including
        // `respawn_at` and `flee_until`, which are deadlines rather than
        // counters for the reason `charges` states, and `awake`, because
        // two shards that disagree about which animals are dormant will
        // disagree about every position downstream of it.
        for m in self.mobs.m.iter() {
            if !m.alive {
                continue;
            }
            let mut buf = [0u8; 40];
            buf[0..4].copy_from_slice(&m.body.qx.to_le_bytes());
            buf[4..8].copy_from_slice(&m.body.qy.to_le_bytes());
            buf[8..12].copy_from_slice(&m.body.qz.to_le_bytes());
            buf[12..16].copy_from_slice(&m.body.qvy.to_le_bytes());
            buf[16] = m.body.grounded as u8;
            buf[17] = m.kind;
            buf[18] = m.gait as u8;
            buf[19] = m.awake as u8;
            buf[20..22].copy_from_slice(&m.yaw.to_le_bytes());
            buf[22..24].copy_from_slice(&m.hp.to_le_bytes());
            buf[24..32].copy_from_slice(&m.flee_until.to_le_bytes());
            buf[32..40].copy_from_slice(&m.respawn_at.to_le_bytes());
            h.update(&buf);
        }
        h.update(&(self.deploys.len() as u64).to_le_bytes());
        for d in self.deploys.entries() {
            let mut buf = [0u8; 17];
            buf[0..2].copy_from_slice(&d.cx.to_le_bytes());
            buf[2..4].copy_from_slice(&d.cz.to_le_bytes());
            buf[4] = d.level;
            buf[5] = d.loc;
            buf[6] = d.row;
            buf[7..9].copy_from_slice(&d.hp.to_le_bytes());
            buf[9..11].copy_from_slice(&d.uh.to_le_bytes());
            buf[11..15].copy_from_slice(&d.owner.to_le_bytes());
            buf[15] = d.open as u8;
            buf[16] = d.locked as u8;
            h.update(&buf);
        }
        // The code locks (lock v1). Every byte is state a shard can
        // disagree about and a raid can turn on: two shards that disagree
        // about a remembered list disagree about who is inside the base
        // ten seconds from now, and the miss counters decide whether the
        // next press is a shock or a shut keypad.
        //
        // Hashed on the **arrow** idiom — no length prefix — and that is
        // deliberate for the reason the arrow store states next door: a
        // `h.update(&len)` folds eight zero bytes even for an empty store
        // and would move `GOLDEN_FINAL_HASH` for a slice the replay
        // script never exercises. A dense store's iteration order is
        // insert order rewritten by swap-remove, and both are commands'
        // consequences, so it is as deterministic as `entries` above.
        for l in self.deploys.locks() {
            let mut buf = [0u8; 14];
            buf[0..2].copy_from_slice(&l.cx.to_le_bytes());
            buf[2..4].copy_from_slice(&l.cz.to_le_bytes());
            buf[4] = l.level;
            buf[5] = l.loc;
            buf[6..10].copy_from_slice(&l.owner.to_le_bytes());
            buf[10..12].copy_from_slice(&l.code.to_le_bytes());
            buf[12..14].copy_from_slice(&l.guest_code.to_le_bytes());
            h.update(&buf);
            let mut sb = [0u8; 20];
            sb[0] = l.locked as u8;
            sb[1] = l.auth.len() as u8;
            sb[2] = l.guests.len() as u8;
            sb[3] = l.misses;
            sb[4..12].copy_from_slice(&l.last_miss.to_le_bytes());
            sb[12..20].copy_from_slice(&l.shut_until.to_le_bytes());
            h.update(&sb);
            // The lists themselves, whole rather than to `n_auth`: a slot
            // past the count is zeroed by `reset_lists` and by
            // `remove_at`, so folding all of it is folding state, and a
            // store that ever left residue there would be caught rather
            // than hidden.
            for id in l.auth.raw().iter().chain(l.guests.raw().iter()) {
                h.update(&id.to_le_bytes());
            }
        }
        // Burning fuses. State as plainly as anything here: two shards
        // that disagree about a live charge disagree about whether a base
        // is standing ten seconds from now, and `fires_at` is hashed
        // rather than a remaining count for the reason the record keeps a
        // deadline — the absolute tick is the fact, and a remainder is a
        // view of it that a replay resuming mid-fuse would have to rebuild.
        h.update(&(self.charges.len() as u64).to_le_bytes());
        for c in self.charges.entries() {
            let mut buf = [0u8; 21];
            buf[0..2].copy_from_slice(&c.cx.to_le_bytes());
            buf[2..4].copy_from_slice(&c.cz.to_le_bytes());
            buf[4] = c.level;
            buf[5] = c.loc;
            buf[6] = c.deploy as u8;
            buf[7..9].copy_from_slice(&c.structure.to_le_bytes());
            buf[9..17].copy_from_slice(&c.fires_at.to_le_bytes());
            // The planter rides the digest too: it is on the wire off this
            // record, so a replay that reproduced the blast while
            // inventing the raider would put a different name on it.
            buf[17..21].copy_from_slice(&c.owner.to_le_bytes());
            h.update(&buf);
        }
        // The bag cooldowns, in their own pass rather than widening the
        // buffer above — they live in a parallel array precisely so the
        // wire's mirror of `DeployRec` does not carry them (deploy.rs), and
        // the digest follows the storage. They are state like any other
        // timer here: two shards that disagree about which bags are spent
        // disagree about where the next death wakes, and a hash that could
        // not see it would call them the same world.
        for r in self.deploys.bag_ready() {
            h.update(&r.to_le_bytes());
        }
        // ...and the deploy placement clocks, for the same reason.
        for t in self.deploys.placed() {
            h.update(&t.to_le_bytes());
        }
        h.update(&(self.deploys.hearths().len() as u64).to_le_bytes());
        for hr in self.deploys.hearths() {
            let mut buf = [0u8; 12];
            buf[0..2].copy_from_slice(&hr.cx.to_le_bytes());
            buf[2..4].copy_from_slice(&hr.cz.to_le_bytes());
            buf[4] = hr.level;
            buf[5..9].copy_from_slice(&hr.owner.to_le_bytes());
            h.update(&buf);
            for s in hr.stock.iter() {
                h.update(&s.to_le_bytes());
            }
            // The crew (hearth crew v1). State as plainly as the stock is:
            // two shards that disagree about who may build inside a claim
            // disagree about whether the next foundation lands. The whole
            // backing array, tail included, for `Roster`'s stated reason —
            // residue there would be invisible state, and `remove` zeroes
            // what it vacates precisely so this fold is exact.
            h.update(&(hr.crew.len() as u32).to_le_bytes());
            for id in hr.crew.raw().iter() {
                h.update(&id.to_le_bytes());
            }
        }
        // Box contents, in the dense list's own order — which is
        // placement order rewritten by swap-remove, and deterministic for
        // the same reason `entries` above is: every insert and every
        // removal is a command's consequence, replayed in the same order.
        h.update(&(self.deploys.boxes().len() as u64).to_le_bytes());
        for bx in self.deploys.boxes() {
            let mut buf = [0u8; 9];
            buf[0..2].copy_from_slice(&bx.cx.to_le_bytes());
            buf[2..4].copy_from_slice(&bx.cz.to_le_bytes());
            buf[4] = bx.level;
            buf[5..9].copy_from_slice(&bx.owner.to_le_bytes());
            h.update(&buf);
            for s in bx.items.iter() {
                let mut sb = [0u8; 4];
                sb[0..2].copy_from_slice(&s.item.to_le_bytes());
                sb[2..4].copy_from_slice(&s.count.to_le_bytes());
                h.update(&sb);
            }
        }
        // Oven state, in its own pass beside the contents for the reason
        // the bag cooldowns get one: it lives in a parallel array so the
        // container-sync message does not carry it, and the digest
        // follows the storage. It is state and not decoration — two
        // shards that disagree about which fires are lit disagree about
        // how much charcoal exists an hour from now.
        for ov in self.deploys.oven_states() {
            let mut buf = [0u8; 6];
            buf[0] = ov.arch;
            buf[1] = ov.lit as u8;
            buf[2..4].copy_from_slice(&ov.burn.to_le_bytes());
            buf[4..6].copy_from_slice(&ov.bank.to_le_bytes());
            h.update(&buf);
            for c in ov.cook.iter() {
                h.update(&c.to_le_bytes());
            }
        }
        h.update(&(self.backpacks.len() as u64).to_le_bytes());
        for b in self.backpacks.entries() {
            let mut buf = [0u8; 28];
            buf[0..4].copy_from_slice(&b.id.to_le_bytes());
            buf[4..8].copy_from_slice(&b.qx.to_le_bytes());
            buf[8..12].copy_from_slice(&b.qy.to_le_bytes());
            buf[12..16].copy_from_slice(&b.qz.to_le_bytes());
            buf[16..20].copy_from_slice(&b.owner.to_le_bytes());
            buf[20..28].copy_from_slice(&b.expires.to_le_bytes());
            h.update(&buf);
            for s in b.items.iter() {
                let mut sb = [0u8; 4];
                sb[0..2].copy_from_slice(&s.item.to_le_bytes());
                sb[2..4].copy_from_slice(&s.count.to_le_bytes());
                h.update(&sb);
            }
        }
        // The id counter is state, not a cursor: a replay that reused an
        // id the first run retired would name two different bags the same
        // thing, and every downstream client keyed on it would agree with
        // neither.
        h.update(&self.backpacks.next_id().to_le_bytes());
        h.update(&self.sweep_piece.to_le_bytes());
        h.update(&self.sweep_deploy.to_le_bytes());
        h.update(&self.sweep_support.to_le_bytes());
        h.update(&self.evictions.to_le_bytes());
        h.digest()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terrain;

    /// The point and seed `ci/browser_smoke.mjs` puts both tabs on. Guarded
    /// here natively so a worldgen change that sinks or steepens it fails
    /// this test, not the browser gate.
    const SMOKE_SEED: u64 = 20260731;
    const SMOKE_SPAWN: (f32, f32) = (1024.0, 1024.0);

    #[test]
    fn dev_spawn_overrides_every_join() {
        let mut w = World::new(SMOKE_SEED);
        w.dev_spawn = Some(SMOKE_SPAWN);
        w.tick(&[Command::Join { id: 7 }, Command::Join { id: 8 }]);
        for id in [7u32, 8] {
            let p = w.players.iter().find(|p| p.active && p.id == id).unwrap();
            // Body::at quantizes at 3 cm x/z: exact in quantized space.
            assert_eq!(p.body.qx, movement::quant_xz(SMOKE_SPAWN.0));
            assert_eq!(p.body.qz, movement::quant_xz(SMOKE_SPAWN.1));
        }
        // And None still scatters: two ids land apart.
        let mut w2 = World::new(SMOKE_SEED);
        w2.tick(&[Command::Join { id: 7 }, Command::Join { id: 8 }]);
        let a = w2.players[0].body;
        let b = w2.players[1].body;
        assert!(a.qx != b.qx || a.qz != b.qz);
    }

    /// The slice's whole point (NOW.md): a fresh spawn is **clear of
    /// scatter**, not merely walkable. Nothing here calls the selector's
    /// own predicate — it re-derives the facts from terrain, and scans a
    /// 5×5 cell block where `scatter_clear` scans 3×3, so a clearance
    /// radius that outgrew the scanned block would fail here rather than
    /// pass silently.
    ///
    /// Also gates both fallbacks: the island-center miss is forest (biome
    /// assert), and the relaxed merely-walkable one is by definition
    /// occupied (clearance assert). Neither can fire without reddening.
    #[test]
    fn spawn_ring_lands_on_a_clear_beach() {
        // 32 islands × 64 joins. Measured on the way in: the worst spawn
        // over 400 seeds × 64 ids took 7 of the 48 candidates, so the
        // sweep is nowhere near the fallback and a regression that starts
        // exhausting candidates shows up here as a failed assert, not as
        // a quietly worse spawn.
        for i in 0..32u64 {
            let seed = if i == 0 { SMOKE_SEED } else { i * 7919 + 3 };
            let w = World::new(seed);
            let mut quadrants = [0u32; 4];
            for id in 1..=64u32 {
                let (x, z) = w.spawn_pos(id);
                let h = terrain::height(seed, x, z);
                let m = terrain::moisture(seed, x, z);
                assert_eq!(
                    terrain::biome(h, m),
                    terrain::Biome::Beach,
                    "seed {seed} id {id}: spawn ({x},{z}) height {h} is not beach"
                );
                assert!(
                    h > movement::WADE_GROUND_MAX,
                    "seed {seed} id {id}: spawn ({x},{z}) height {h} is in the wade band"
                );
                let s = terrain::slope(seed, x, z);
                assert!(s < 1.0, "seed {seed} id {id}: spawn ({x},{z}) slope {s}");

                for ox in -2..=2 {
                    for oz in -2..=2 {
                        let cx = crate::fmath::floor_i32(x / terrain::CELL_SIZE) + ox;
                        let cz = crate::fmath::floor_i32(z / terrain::CELL_SIZE) + oz;
                        let slot = terrain::scatter(seed, &w.scatter, &w.haven, cx, cz);
                        if slot.occupant == terrain::Occupant::None {
                            continue;
                        }
                        let (dx, dz) = (slot.x - x, slot.z - z);
                        let d2 = dx * dx + dz * dz;
                        assert!(
                            d2 >= SPAWN_CLEAR_M * SPAWN_CLEAR_M,
                            "seed {seed} id {id}: spawn ({x},{z}) stands {} m from a {:?} \
                             at ({},{})",
                            d2.sqrt(),
                            slot.occupant,
                            slot.x,
                            slot.z
                        );
                    }
                }

                let c = terrain::ISLAND_SIZE * 0.5;
                quadrants[(usize::from(x > c)) | (usize::from(z > c) << 1)] += 1;
            }
            // A ring, not a lucky cove: 64 ids reach every quadrant of the
            // coast. (The old placeholder would pass every assert above at
            // one point on one beach.)
            assert!(
                quadrants.iter().all(|&n| n > 0),
                "seed {seed}: spawns are not distributed around the ring: {quadrants:?}"
            );
        }
    }

    #[test]
    fn smoke_spawn_point_is_walkable() {
        let (x, z) = SMOKE_SPAWN;
        let h = terrain::height(SMOKE_SEED, x, z);
        let s = terrain::slope(SMOKE_SEED, x, z);
        assert!(
            (1.5..45.0).contains(&h) && s < 1.0,
            "browser-smoke spawn ({x},{z}) unwalkable at seed {SMOKE_SEED}: height {h} slope {s}"
        );
    }
}
