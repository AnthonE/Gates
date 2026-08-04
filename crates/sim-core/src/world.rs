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
    CRAFT_QUEUE, HOTBAR_SLOTS, INV_SLOTS, MAX_COMMANDS_PER_TICK, MAX_EVENTS_PER_TICK, MAX_PLAYERS,
    STATE_HASH_INTERVAL,
};
use crate::loot::{LootContent, LOOT_BARREL};
use crate::movement::{self, Body};
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
/// (decay took it; the wire broadcasts and restarts in-progress walks).
pub const EV_PIECE_REMOVED: u8 = 11;
/// EV_DEPLOY_REMOVED: a = build cell key, b = level << 16 | loc << 8 | row.
pub const EV_DEPLOY_REMOVED: u8 = 12;
/// EV_STOCK: a = feeder player id, b = hearth cell key, c = level — the
/// feed ack; the wire reads the hearth's stock from the world at encode.
pub const EV_STOCK: u8 = 13;
/// EV_DOOR: a = build cell key, b = level << 16 | loc << 8 | locked << 1
/// | open, c = the player whose action changed it. The door's whole state,
/// absolute, whether the toggle or the lock moved (lock v0). Broadcast —
/// door state is a world fact like a placement.
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
/// barrel's loot (`backpack::stand_up`). Broadcast — a container on the
/// ground is a world fact like a placement; the wire reads its position
/// out of the store at encode, the way a hearth's stock is read
/// (backpack.rs), and carries no owner at all, so `b` is a sim-side fact
/// only.
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

/// The highest code above, named rather than counted: the event codes are
/// `1..=EV_MAX` with no gaps, and `test_event_roles`'s coverage ledger
/// scans that range. It lived in that test as a literal `25`, which meant a
/// twenty-sixth code could land and read as classified while nothing had
/// classified it. Tying it to the last constant closes half of that; the
/// other half is the ledger's own `every_event_code_is_in_range`, which
/// parses this file and fails if a code is declared past this line.
pub const EV_MAX: u8 = EV_MOVE_REFUSED;

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
    Leave {
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
    /// Set the lock bit of the door at the address (owner-only; absolute,
    /// never a toggle — deploy.rs validates and refuses by event).
    Lock {
        id: u32,
        cx: u16,
        cz: u16,
        level: u8,
        loc: u8,
        locked: bool,
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
    /// Take everything that fits from the nearest backpack in reach
    /// (backpack.rs). No target crosses: the pick is the sim's, the same
    /// shape a swing's is.
    Loot {
        id: u32,
    },
    /// Move `count` items between two slots (`inventory.rs`). `bag` names
    /// the ground container either side may address — zero when the move
    /// is inside the sender's own inventory, and ignored when neither kind
    /// is `CONT_BAG`.
    ///
    /// Unlike `Loot`, this one *does* carry a target, and it has to: the
    /// whole verb is the player choosing which slot, which is the choice
    /// `Loot` takes away by moving everything that fits. What the sim
    /// keeps is the verdict — a forged kind, a slot past the container, a
    /// bag out of reach and a count past the stack all land on the same
    /// announced `EV_MOVE_REFUSED`, and none of them on a dropped session.
    Move {
        id: u32,
        bag: u32,
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
    /// Baked loot tables (loot.rs). Construction input too; the inert
    /// default leaves barrels standing, because a barrel that broke into
    /// nothing would be worse than one that does not break.
    pub loot: LootContent,
    /// Placed building pieces — sim state, hashed.
    pub pieces: Pieces,
    /// Placed deployables + the hearth list — sim state, hashed.
    pub deploys: Deploys,
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
    /// Sparse harvested/damaged slot records (TERRAIN.md §2).
    pub slot_lives: SlotLives,
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
        Self {
            seed,
            tick: 0,
            players: [Player::default(); MAX_PLAYERS],
            scatter: ScatterTable::alpha_default(),
            haven: terrain::haven(seed),
            gather: GatherContent::EMPTY,
            craft: CraftContent::EMPTY,
            build: BuildContent::EMPTY,
            deploy: DeployContent::EMPTY,
            combat: CombatContent::EMPTY,
            backpack: BackpackContent::EMPTY,
            survival: SurvivalContent::EMPTY,
            loot: LootContent::EMPTY,
            pieces: Pieces::new(),
            deploys: Deploys::new(),
            backpacks: Box::new(Backpacks::new()),
            sweep_piece: 0,
            sweep_deploy: 0,
            slot_lives: SlotLives::new(),
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
    fn live_slot_of(&self, id: u32) -> Option<usize> {
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
    #[allow(clippy::too_many_arguments)]
    fn move_item(
        &mut self,
        slot: usize,
        bag: u32,
        from_kind: u8,
        from_slot: u8,
        to_kind: u8,
        to_slot: u8,
        count: u16,
    ) {
        let pid = self.players[slot].id;
        let addr = inventory::addr(from_kind, from_slot, to_kind, to_slot);

        // 1. Is that an address at all? Kind, both slot indices, and the
        //    move-onto-itself case. Nothing has been read yet.
        if from_kind > inventory::CONT_MAX
            || to_kind > inventory::CONT_MAX
            || from_slot as usize >= INV_SLOTS
            || to_slot as usize >= INV_SLOTS
            || (from_kind == to_kind && from_slot == to_slot)
        {
            self.events
                .push(EV_MOVE_REFUSED, pid, inventory::REFUSE_M_SLOT, addr);
            return;
        }

        // 2. Resolve the ground container, if either side names one. A bag
        //    that left the world and a bag out of arm's reach are separate
        //    reasons because they are separate player-facing facts: one
        //    means "it is gone", the other "walk back".
        let touches_bag = from_kind == inventory::CONT_BAG || to_kind == inventory::CONT_BAG;
        let bag_idx = if touches_bag {
            let Some(i) = self.backpacks.index_of_id(bag) else {
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
        } else {
            None
        };
        // Safe past the guard above: `bag_idx` is `Some` whenever either
        // kind is `CONT_BAG`, and the zero is never indexed otherwise.
        let bi = bag_idx.unwrap_or(0);

        // 3. Read both sides as copies.
        let src = if from_kind == inventory::CONT_BAG {
            self.backpacks.slot(bi, from_slot as usize)
        } else {
            self.players[slot].inv[from_slot as usize]
        };
        let dst = if to_kind == inventory::CONT_BAG {
            self.backpacks.slot(bi, to_slot as usize)
        } else {
            self.players[slot].inv[to_slot as usize]
        };

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
        if from_kind == inventory::CONT_BAG {
            self.backpacks.set_slot(bi, from_slot as usize, new_src);
        } else {
            self.players[slot].inv[from_slot as usize] = new_src;
        }
        if to_kind == inventory::CONT_BAG {
            self.backpacks.set_slot(bi, to_slot as usize, new_dst);
        } else {
            self.players[slot].inv[to_slot as usize] = new_dst;
        }
        self.events.push(
            EV_MOVED,
            pid,
            addr,
            ((count as u32) << 16) | src.item as u32,
        );
        // A bag a withdrawal emptied leaves by `loot_nearest`'s route, so
        // the wire sees one removal contract however it was emptied.
        if let Some(i) = bag_idx {
            self.backpacks.drop_if_empty(i, &mut self.events);
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

    fn apply(&mut self, cmd: &Command) {
        match *cmd {
            Command::Join { id } => {
                if self.slot_of(id).is_some() {
                    return;
                }
                if let Some(slot) = self.players.iter().position(|p| !p.active) {
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
                    // Say it at the door. Health is only ever announced
                    // when it changes, so without this a fresh player has
                    // no vitals until the first thing that hurts them —
                    // which is the one moment a bar is no use. The meters
                    // are announced at the door for the same reason.
                    if hp > 0 {
                        self.events.push(EV_HEALTH, id, hp as u32, hp as u32);
                    }
                    survival::announce_vitals(
                        &self.survival,
                        &self.players[slot],
                        &mut self.events,
                    );
                }
                // No free slot: refuse silently here; the accept path
                // already hard-caps at the shard limit (limits.rs).
            }
            Command::Leave { id } => {
                if let Some(slot) = self.slot_of(id) {
                    self.players[slot].active = false;
                }
            }
            Command::Input { id, frame } => {
                if let Some(slot) = self.slot_of(id) {
                    let mut frame = frame;
                    if frame.sel as usize >= HOTBAR_SLOTS {
                        // The wire refuses 6–7 at decode; a non-wire
                        // command (bot, test, WAL) falls back to slot 0.
                        frame.sel = 0;
                    }
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
            Command::Lock {
                id,
                cx,
                cz,
                level,
                loc,
                locked,
            } => {
                if let Some(slot) = self.live_slot_of(id) {
                    deploy::set_lock(
                        &self.deploy,
                        &mut self.deploys,
                        &self.players[slot],
                        cx,
                        cz,
                        level,
                        loc,
                        locked,
                        &mut self.events,
                    );
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
                bag,
                from_kind,
                from_slot,
                to_kind,
                to_slot,
                count,
            } => {
                if let Some(slot) = self.live_slot_of(id) {
                    self.move_item(slot, bag, from_kind, from_slot, to_kind, to_slot, count);
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
        for cmd in commands.iter().take(MAX_COMMANDS_PER_TICK) {
            self.apply(cmd);
        }
        let seed = self.seed;
        let tick = self.tick;
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
                movement::step(seed, self.pieces.cols(), &mut self.players[i].body, &frame);
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
            movement::step(seed, self.pieces.cols(), &mut self.players[i].body, &frame);
            let swung = gather::swing(
                seed,
                tick,
                &self.gather,
                &self.loot,
                &self.scatter,
                &self.haven,
                &mut self.slot_lives,
                &mut self.events,
                &mut self.players[i],
            );
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
                        combat::raid(
                            &self.combat,
                            &self.build,
                            &self.deploy,
                            seed,
                            &self.players[i],
                            &mut self.pieces,
                            &mut self.deploys,
                            &mut self.events,
                        );
                    }
                }
            }
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
            &mut self.events,
        );
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
            let mut db = [0u8; 10];
            db[0..4].copy_from_slice(&p.death_by.to_le_bytes());
            db[4] = p.dead as u8;
            db[5] = p.death_cause;
            db[6..8].copy_from_slice(&p.death_item.to_le_bytes());
            db[8..10].copy_from_slice(&p.death_range_cm.to_le_bytes());
            h.update(&db);
            for s in p.inv.iter() {
                let mut sb = [0u8; 4];
                sb[0..2].copy_from_slice(&s.item.to_le_bytes());
                sb[2..4].copy_from_slice(&s.count.to_le_bytes());
                h.update(&sb);
            }
            let mut cb = [0u8; 8 + CRAFT_QUEUE * 4];
            cb[0..8].copy_from_slice(&p.craft_done_at.to_le_bytes());
            for (j, job) in p.jobs.iter().enumerate() {
                cb[8 + j * 4..8 + j * 4 + 2].copy_from_slice(&job.recipe.to_le_bytes());
                cb[8 + j * 4 + 2..8 + j * 4 + 4].copy_from_slice(&job.remaining.to_le_bytes());
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
