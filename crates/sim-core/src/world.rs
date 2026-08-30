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
    MAX_EVENTS_PER_TICK, MAX_MAGS, MAX_PLAYERS, MAX_REMOVALS_PER_TICK, STATE_HASH_INTERVAL,
    WEAR_SLOTS,
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
/// Clearance from any scatter slot center, metres.
///
/// **Raised 4.0 → 4.5 when the tree pool gained a second, wider species**
/// (operator, 2026-08-10: *"im fine with raising the sim or whatever we got
/// the juice i think"*). The derivation, which is now checked rather than
/// merely stated: the widest canopy ceiling (`tree::TREE_MAX_R`, 2.9 m) at the
/// largest scale `scatter` rolls (`SLOT_SCALE_MAX`, 1.1) is a 3.19 m canopy
/// edge; plus `CAPSULE_RADIUS_M` (0.4) is 3.59 m of touching distance; plus
/// standing room so a spawn does not open with a trunk filling the frame.
///
/// **This used to credit `ci/pine_shape.mjs` with closing that arithmetic, and
/// that gate does not exist** — it went with the browser client, so the
/// derivation was a dead citation of the exact kind `CLAUDE.md` says to
/// assume about any `.mjs` a doc comment names. It is closed in Rust now, by
/// `crates/client/tests/tree.rs::a_fresh_spawn_stands_clear_of_the_widest_tree`
/// — the client is the crate that can see both the mesh and this constant.
/// `pub` for that reason and no other.
pub const SPAWN_CLEAR_M: f32 = 4.5;

/// Integer event codes (CLAUDE.md wall 3) — the sim's outbound facts, one
/// ring per tick, drained by the server after `tick` returns.
/// EV_GATHER: a = player id, b = item index << 16 | units actually added
/// (0 = the pack was full and every unit went to the ground; the loss is
/// announced, never silent — `EV_CRAFT_DONE` has said this since it
/// landed and this one now says it too).
/// Read it as "these units entered your inventory", not as "a node paid":
/// looting a backpack (backpack.rs) announces its take the same way, and
/// deliberately — the client's `+N Item` toast is the right feedback for
/// both, and loot pays in the currency gathering already pays in.
///
/// **Every producer owes the zero its meaning.** All three only push when
/// something was actually owed — `gather::swing` guards on `pay > 0` and
/// on `sec_pay > 0`, `backpack`'s loot walk skips a slot it took nothing
/// from — so a zero here is never "nothing happened". A fourth producer
/// that pushes an unowed zero silently turns the client's "pack full" line
/// into a lie, which is why the guards are stated rather than assumed.
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
/// EV_HIT: a = attacker player id, b = victim player id, c = the part in
/// the high bits over the damage dealt (`hit_c`, v58).
/// The attacker's fact — the hitmarker, not the truth; EV_HEALTH is what
/// the victim's bar reads (combat.rs).
///
/// `c` is **packed**, and that is the whole of the "two spare bits" this
/// event had: `a` is what the server routes on and `b` already carries a
/// mob tag in its own high bits (`mob::mob_id`), so the only field with
/// room was the damage — a `u16` sitting in a `u32`. Read it with
/// [`hit_part`] and [`hit_damage`], never by hand.
///
/// Why the part rides here at all: the ladder pays x2 / x1 / x0.5 off
/// where the line crossed the cylinder, and until v58 the wire carried
/// only the product. A halved number is easier to misread as a miss than
/// a doubled one is to read as a skull, so aim was unlearnable in play —
/// three rungs of arithmetic with no way to perceive any of them.
pub const EV_HIT: u8 = 15;

/// Where the part sits in [`EV_HIT`]'s `c`. Sixteen, because the damage
/// below it is a `u16` and occupies the whole low half.
pub const HIT_PART_SHIFT: u32 = 16;

/// Pack an `EV_HIT` payload: the part above the damage it already scaled.
#[inline]
pub fn hit_c(part: crate::collide::Part, damage: u16) -> u32 {
    ((part.bits() as u32) << HIT_PART_SHIFT) | damage as u32
}

/// The rung out of an `EV_HIT` payload.
///
/// Total, unlike [`crate::collide::Part::from_bits`], because the only
/// producer is [`hit_c`] and a value it cannot make is a sim bug rather
/// than an untrusted input — `Chest` is the identity rung and the
/// documented fallback, so a garbled read costs the marker its colour and
/// never its existence. The wire does **not** inherit this leniency; the
/// decoder range-checks (`protocol::event`).
#[inline]
pub fn hit_part(c: u32) -> crate::collide::Part {
    crate::collide::Part::from_bits(((c >> HIT_PART_SHIFT) & 0b11) as u8)
        .unwrap_or(crate::collide::Part::Chest)
}

/// The scaled damage out of an `EV_HIT` payload.
#[inline]
pub fn hit_damage(c: u32) -> u16 {
    (c & 0xffff) as u16
}
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

/// EV_SHOT: a = the shooter's player id, b = yaw << 8 | pitch, c =
/// speed mm/tick << 16 | drop mm/tick². **Broadcast** — an arrow in the
/// air is a world fact like a door swinging, and it is the one fact in
/// combat that everyone near it needs and only the shooter had.
///
/// **`speed == 0` reads as *instantaneous*, and then the low half is the
/// reach in decimetres rather than a drop** (wire v54, `DECISIONS.md`
/// §open). A projectile cannot leave the muzzle at rest, so that pattern
/// was unreachable rather than merely unused, which is what makes it
/// spendable: it costs no field and it partitions the event into the two
/// things a ranged weapon can be. A flight is re-flown from
/// `(speed, drop)`; a beam is drawn from `(yaw, pitch, reach)` and gone
/// the same frame. Until v54 a firearm raised no `EV_SHOT` at all, so it
/// announced itself only by what it *reached* and a gunfight had no
/// sound, no flash and no line — the disclosure the reference gets from
/// audio at a hundred metres (`reference/AUDIO.md` §9).
///
/// **The payload is what a tracer needs and not one field more, and the
/// omissions are the design.** No origin: the client knows where the
/// shooter is from the snapshot, and `ranged::ARROW_EYE_MM` above the feet
/// is a constant on both sides. No item: an arrow is an arrow to look at,
/// and the day a fire arrow must *look* different is the day this earns a
/// field. What it does carry is the ballistics, because `client-core`
/// holds no content tables at all — it is a wire and prediction layer, so
/// speed and drop have to cross or the client cannot draw the curve.
///
/// Carrying them has a better reason than necessity, though. The
/// trajectory is a pure function of `(origin, yaw, pitch, speed, drop)`,
/// so a client handed all five integrates **the same arc the sim did** —
/// the quantize-both-sides law (CLAUDE.md's trap list) applied to a
/// tracer. The drawn arrow is not an approximation of the real one that
/// drifts over a second of flight; it is the same arithmetic.
///
/// This is deliberately **not** the reference game's model, and §9.1 of
/// `reference/PROJECTILES.md` is why: theirs lets the client own the
/// projectile and audits it with thirteen tolerance convars. Here the
/// client owns a *picture* of a projectile the server already fired, and
/// a forged one changes nothing but what its author sees.
pub const EV_SHOT: u8 = 35;

/// EV_KNOWN: a = the player who holds it, b = the blueprint mask's low 32
/// bits, c = its high 32 bits. **Own-fact** — a blueprint is personal, so
/// only the hand that holds it hears this.
///
/// The whole mask, never a delta, and the reason is the one `SUB_KNOWN`
/// was already written for: a dropped increment would grey a recipe the
/// player has paid JUNK for, with no later event able to correct it. A
/// full statement of the fact is self-healing — the next one repairs
/// every loss before it.
///
/// **It fires at the doors, not only at the purchase, and that is the
/// whole point of the code existing.** `Player::known` is sim state that
/// survives a death and a restore, but nothing on the wire ever said so:
/// the server synthesised `SUB_KNOWN` from `EV_RESEARCH` alone, so a
/// player who researched on Monday, logged out and came back on Tuesday
/// was told nothing, and their craft panel greyed six recipes they owned
/// until they bought a seventh. Three doors emit it — `seat`, `take_over`
/// and `wake` — because those are the three places a body starts being
/// driven, and a fact stated at only two of them is a fact that depends
/// on how you arrived.
///
/// Two `u32`s for one `u64` because an event field is a `u32` and
/// `KNOWN_MASK_BITS` is 64. Low first: `b` is `mask as u32` and `c` is
/// `(mask >> 32) as u32`, which is the order `encode_event_known` puts
/// them back together in.
pub const EV_KNOWN: u8 = 36;

/// EV_GATHER_REFUSED: a = player id, b = held item index << 16 |
/// `gather::REFUSE_G_*` reason. Own-fact, `EV_CRAFT_REFUSED`'s posture: a
/// button that did nothing says so. The high half names the **held item**
/// (`NO_ITEM` = bare hands) rather than only a reason, because the
/// sentence the client owes is *a torch cannot fell a tree* — hotbar 2 is
/// one key from the rock, and a new player pressing `2` at a tree used to
/// get nothing at all (`NOW.md` §0kit item 2). Bounded by the swing
/// cadence: at most one per `SWING_INTERVAL_TICKS` per player.
pub const EV_GATHER_REFUSED: u8 = 37;

/// EV_IMPACT: a = `ranged::SURF_*` << 24 | the stop point's x in `POS_XZ_Q`
/// quanta, b = its z in the same, c = its y in `POS_Y_Q` quanta **as a
/// signed `i32` reinterpreted** — an arrow can stop below sea level, and
/// this is the one field in the lane that can be negative.
///
/// **Broadcast**, `EV_SHOT`'s posture and its reason: where an arrow
/// landed is a world fact, and the mark it leaves is visible to anyone who
/// walks past it later. A client that misses one loses a scuff.
///
/// **It fires only where something met the WORLD, never where it met
/// flesh.** `ranged::step` resolves a body first and leaves by another
/// door, so flesh is `EV_HIT` and this is everything else — the two are
/// exclusive and a reader never has to ask which kind of mark to make.
///
/// **Three producers since 2026-08-28, and it was never really an arrow's
/// fact.** `ranged::step` pushes it where an arrow stopped, `gather::swing`
/// where a landed melee swing bit an occupant, and `combat::raid` where one
/// bit a built piece or a solid deployable (`combat::piece_mark`). The fact
/// is *a surface was struck at this point*, which belongs to none of the
/// three verbs, so a mark on a tree and a mark on a plank each cost no wire
/// byte, no `PROTO_VER` bump and no client line — `render/decal.rs` was
/// already the single reader and cannot tell them apart, which is the test
/// of whether reuse was honest rather than convenient. **The third one is
/// why the first two were worth reusing**: the shard had taken the right hp
/// off the right record for an arrow, a bullet, four wall orientations and
/// nine deployable archetypes while a raider could not see any of it
/// (`NOW.md` §0mk item 1; the merge-gate judge's second ranked gap,
/// 2026-08-28), and closing that was one emit site rather than an event.
///
/// A swing that lands on NOTHING pushes nothing, on all three paths: the
/// mark sits past the node's tool refusal, past `raid`'s target pick and
/// past its store lookup, so a torch waved at a tree leaves the bark clean
/// and a swing at empty air leaves the base unmarked.
///
/// The position is here rather than read back out of a store at encode
/// (`EV_BAG_DROPPED`'s trick) because on the arrow path there is nothing
/// left to read — the arrow's slot is freed on the same line that pushes
/// this — and on the swing path the point is not a thing at all, only the
/// place where two things touched. An impact is a fact about a moment, not
/// about a thing that persists, which is also why nothing about it is
/// saved — see `worldsave.rs` for what is.
pub const EV_IMPACT: u8 = 38;

/// EV_TRUST: a = the player who acted, b = the counterparty — the player
/// whose record the verb answered to, c = `TRUST_*` verb << 8 |
/// `PRESENCE_*`.
///
/// **The trust ledger's own row** (`PLAYERS.md` §the four walls, wall 3).
/// Every other event in this lane says what happened to a *thing*; this
/// one says what happened between two *people*, and it carries the one
/// field none of the others has room for: whether the counterparty was
/// online when it happened. That field is what the agent-player
/// measurement turns on, it is ordinary game state a human client already
/// sees standing there, and it cannot be added later — a shard-hour
/// logged without it is a shard-hour that cannot answer the question.
///
/// **Sim-side only, and deliberately.** Nothing encodes it, so no wire
/// byte moves (wall 6 is untouched) — and that is the design, not an
/// omission being deferred. Broadcasting "this base's owner is asleep"
/// would hand every client a fact it has to *walk to a base and watch*
/// to learn, which is `PLAYERS.md` wall 1's affordance rule failing on
/// the human side first. The record is the server's to keep, the way a
/// bag's position is the server's to read at encode.
///
/// **It fires when the verb answered to somebody else's record**, and the
/// filter is one rule in one place (`World::log_trust`): a verb on your
/// own door, your own hearth, your own box creates no trust relationship,
/// so `counterparty == actor` is silent. So is `counterparty == 0` (no
/// player placed it) and so is a `mob::mob_id` (a boar's corpse bag is
/// loot, not a counterparty).
///
/// ⚠ **It rides the same drop-newest ring as everything else, and it is
/// the one passenger a resync cannot re-derive.** `MAX_EVENTS_PER_TICK` is
/// 256 against a `MAX_COMMANDS_PER_TICK` of 256, so a tick saturated with
/// trust verbs was already at the ring's edge before this code existed —
/// what is new is that each such verb now costs two seats instead of one.
/// Every other event in the lane is a fact about *state*, which the late-
/// join sync walk re-derives from the world; this is a fact about a
/// *moment*, and a dropped one is gone. `EventQueue::dropped` counts it,
/// which is the honest floor and not a fix.
///
/// **No address.** Three fields are spent on who/whom/what, and the
/// address is not lost: every push here rides the same tick as the verb's
/// own addressed event — `EV_DOOR` for a leaf, `EV_AUTH` for a grant or a
/// crew seat, `EV_MOVED` for a container — so a reader joins the two by
/// tick and loses nothing. That is a claim, so each of the four causes in
/// `tests/event_roles.rs` asserts its verb's own event is on the tick
/// beside the trust row.
pub const EV_TRUST: u8 = 39;

/// EV_SWING: a = the swinging player's id, b = 0, c = 0.
///
/// **Broadcast**, `EV_SHOT`'s posture and its reason: a swing is a fact
/// about a body that other clients are drawing, and a client that misses
/// one loses an arc. It carries no position, because it does not need to —
/// every body's place is already in the snapshot, and the one thing a
/// remote client cannot derive is *that the arm moved*, since it never
/// receives another player's input frame.
///
/// **Outcome-free, and that is the whole point.** It fires once per swing
/// on a hit AND on a whiff, from the only line in the tree that runs
/// exactly once per swing regardless of what the swing found:
/// `gather::swing`'s cadence gate. Every later exit — a refusal, a free
/// arm handed on to flesh, a smashed barrel — is downstream of a decision
/// the swinger has already committed to with their arm. A fact that only
/// fires when something was hit is not a swing fact; it is a hit fact, and
/// this lane already has one (`EV_HIT`), which is unicast to the attacker
/// and drops field `a` at encode precisely because it is theirs alone.
///
/// **`b` and `c` are zero and stay zero until something reads them.** The
/// held item would be cheap in bits and would make an a/b transposition
/// detectable by `event_roles.rs`'s own discipline, but nothing draws a
/// different arc for a rock than for a hatchet yet, and a field nobody
/// reads is a field nobody maintains (`validate.rs` names that shape).
pub const EV_SWING: u8 = 40;

/// EV_HURT: a = victim player id, b = the world bearing sector from the
/// victim toward whatever hurt them, c = damage dealt.
///
/// **The victim's fact, and the mirror of `EV_HIT`.** A hit has two people
/// in it and the wire only ever told one of them: `EV_HIT` is unicast to
/// the attacker (its own doc says so, and it drops field `a` at encode for
/// that reason), so the whole of being shot was `EV_HEALTH` — an absolute
/// number, with no direction and no author. You could not tell a sniper
/// from a bear from starving. This is the other half, unicast to `a`.
///
/// **Five routes raise it, and the two that do not are named rather than
/// forgotten.** A swing (`combat::strike`), an arrow and a bullet
/// (`ranged`), a bite (the mob loop below) and a blast (`charge::detonate`)
/// all point somewhere; starvation (`survival`) and the keypad shock
/// (`deploy`) do not, because neither has a direction to point at. That
/// split is the load-bearing part and it is gated rather than written down
/// here alone: `tests/damage_routes.rs`'s `ROUTES` table carries an
/// announce column, so a **new** damage route is born announced or born
/// exempt with a reason, the same way it is already born reduced or
/// unreduced. The bite and the blast were silent until 2026-08-30, which
/// is what that column exists to stop happening again.
///
/// **`b` is a sector, not an angle, and that is a disclosure decision.**
/// `combat::HURT_SECTORS` of them, 22.5° each, clockwise from north on
/// `look::bearing_of`'s axes (+Z north, −X east). It is enough to turn
/// toward and not enough to aim with, which is the same reasoning
/// `reference/VOICE.md` §9.1 applies to a voice radius: a client handed a
/// precise position it did not earn is a wallhack whatever the UI does
/// with it. Absolute rather than relative to the victim's own facing, so
/// the client subtracts its own yaw at draw time and the mark stays
/// anchored in the world while the player turns to look.
///
/// **Not a broadcast.** `DECISIONS.md` §open ("attacker-side flinch v0")
/// refuses a bystander flinch on fan-out grounds — one message per landed
/// blow per player, unfiltered, on a fight's hottest path — and that
/// refusal stands. This is one message per landed blow to exactly one
/// recipient, the person it happened to.
pub const EV_HURT: u8 = 41;

/// Which trust-bearing verb `EV_TRUST.c` is about, in its high byte.
///
/// A leaf someone else placed, worked by this hand (`deploy::use_door`).
/// The counterparty is the **deployable's** owner and not the lock's: the
/// thing worked is the door, a lock may be bolted on by another hand
/// entirely, and "who placed this leaf" is the question a raid record
/// wants answered.
pub const TRUST_DOOR: u8 = 1;
/// An access list someone else owns, exercised by this hand — a correct
/// code on their lock (`lock::Outcome::Authorized`) or a crew op on their
/// hearth (`deploy::crew_op`). One value for both, because `EV_AUTH` is
/// already one event for both: "who may do this here" is one question
/// whichever `Roster` answers it (`reference/BUILDING.md` §1 fact 1).
pub const TRUST_AUTH: u8 = 2;
/// A container someone else owns, moved through by this hand — a box, an
/// oven or a bag (`World::move_item`). A world container has no owner and
/// is therefore never this: nobody's crate is nobody's trust.
pub const TRUST_CONT: u8 = 3;
/// The highest verb above, named rather than counted — `EV_MAX`'s
/// discipline applied to a value domain, exactly as `DEATH_BY_MAX` is.
///
/// The difference from `DEATH_BY_MAX` is worth stating, because it is the
/// reason this one is cheaper: a new `DEATH_BY_*` is refused by an
/// encoder at runtime and nothing sees it until a death screen fails to
/// open. Nothing encodes `EV_TRUST`, so a new verb here cannot be refused
/// by the wire — which means the only thing standing between a new value
/// and an unclassified log column is this constant and the ledger that
/// reads it (`event_roles.rs`).
///
/// `PLAYERS.md`'s verb list names a fourth — **give** — and it is
/// deliberately absent: there is no player-to-player give verb in the sim
/// yet, so a `TRUST_GIVE` declared now would be a value with no cause,
/// which is the one thing this lane's discipline refuses. It lands in the
/// commit that lands the verb.
pub const TRUST_VERB_MAX: u8 = TRUST_CONT;

/// Whether the counterparty was online, in `EV_TRUST.c`'s low byte.
///
/// A body with a client driving it. A player on the death screen is
/// **awake**: they are watching, and the whole question this field asks is
/// whether the act was witnessed.
pub const PRESENCE_AWAKE: u8 = 1;
/// A sleeper — the body is standing in the world and nobody is behind it
/// (`Player::sleeping`). This is the value the measurement is for, and the
/// same bit offline raiding already runs on.
pub const PRESENCE_ASLEEP: u8 = 2;
/// No body at all: the id names nobody in the world, because the slot was
/// freed. Its own value rather than folded into `ASLEEP`, because they are
/// different facts about the counterparty and folding them would let the
/// record say a player was reachable when their body was gone.
pub const PRESENCE_GONE: u8 = 3;
/// The highest presence above, `TRUST_VERB_MAX`'s discipline for the low
/// byte. Zero is deliberately not a value here: `EV_TRUST.c` is two
/// packed fields, and a zero in either half is a half that reads the same
/// as a field nobody wrote.
pub const PRESENCE_MAX: u8 = PRESENCE_GONE;

/// EV_RELOAD: a = player id, b = rounds now loaded << 16 | the magazine's
/// ceiling (`ranged::mag_pair`), c = rounds taken from the pack — **zero
/// on a shot**, which is how one code says both halves of the fact.
/// **Own-fact** — how full your cylinder is is yours, and a broadcast of it
/// would be a wallhack that told the shard when to push you.
///
/// The pair rather than the delta, and `EV_KNOWN`'s reason word for word: a
/// dropped increment would leave the HUD claiming rounds the sim does not
/// have, with no later event able to correct it, and the player would find
/// out by pulling the trigger in a fight. A full statement of the fact is
/// self-healing — the next one repairs every loss before it — which is also
/// why the *ceiling* travels on every event instead of once at a catalog
/// drip: `client-core` holds no content tables, so a client handed only the
/// loaded count cannot draw `4/6`.
///
/// `c` is what actually left the pack, which is not `magazine` and not the
/// difference the client could compute: a partial reload off a nearly-empty
/// pack takes fewer rounds than the cylinder wanted, and the toast the
/// player is owed says how many they got. **Zero partitions the event into
/// its two causes** — a fill (`c > 0`, worth a sound and a toast) and a
/// spend (`c == 0`, worth only the readout ticking down) — which is
/// `EV_SHOT`'s `speed == 0` trick one lane over, and spendable for the same
/// reason: a fill that moved no rounds is not a fill, so the pattern was
/// unreachable before it was given a meaning.
///
/// **It fires on the shot as well as on the reload, and that is what makes
/// the readout authoritative rather than predicted.** The alternative was
/// for the client to decrement its own count on `EV_SHOT` — but `EV_SHOT`
/// is a broadcast and `client-core` has no notion of which body is its own,
/// so "was that me" would have had to be invented, and a count the client
/// maintains is a count that drifts silently the first time a datagram is
/// lost. One own-fact event per shot to one recipient is cheaper than that
/// bug, and it costs less than the broadcast `EV_SHOT` beside it.
pub const EV_RELOAD: u8 = 42;

/// EV_RELOAD_REFUSED: a = player id, b = held item index << 16 |
/// `ranged::REFUSE_RL_*` reason, c = rounds now loaded << 16 | the
/// magazine's ceiling. Own-fact, `EV_GATHER_REFUSED`'s posture and its
/// packing: a button that did nothing says so, and the high half of `b`
/// names the **held item** rather than only a reason, because the sentence
/// the client owes is *a bow has no magazine* and not *refused*.
///
/// **It carries the count too, and that is the half that matters.** The
/// dry click — a trigger pulled on an empty cylinder — arrives here rather
/// than as a shot, so this event is the authoritative statement that the
/// magazine is at zero. Without `c` the HUD would have to infer emptiness
/// from a shot that never came, which is the same silence as a dropped
/// datagram.
///
/// Bounded by the weapon's cadence on that path (`ranged::hitscan` pays
/// `rate_ticks` before refusing) and by one action per client per tick on
/// the reload path, so a held trigger cannot flood the lane.
pub const EV_RELOAD_REFUSED: u8 = 43;

/// The highest code above, named rather than counted: the event codes are
/// `1..=EV_MAX` with no gaps, and `test_event_roles`'s coverage ledger
/// scans that range. It lived in that test as a literal `25`, which meant a
/// twenty-sixth code could land and read as classified while nothing had
/// classified it. Tying it to the last constant closes half of that; the
/// other half is the ledger's own `every_event_code_is_in_range`, which
/// parses this file and fails if a code is declared past this line.
pub const EV_MAX: u8 = EV_RELOAD_REFUSED;

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
/// A shot (ranged.rs) — an arrow, and since hitscan v0 a bullet too. Its
/// own cause and not `DEATH_BY_HAND`'s for the same reason `DEATH_BY_SALT`
/// is not the clock's: the sentence the death screen builds is "who, with
/// what, from how far", and 34 m is a different fact about a fight from
/// 1.6 m. `death_item` is the bow or the gun, never the round — the weapon
/// is what the killer held, and it is what makes the shared sentence exact
/// ("shot you with the revolver" / "shot you with the bow").
///
/// **The name is the arrow's because the value is, and a firearm sharing
/// it is a deliberate refusal rather than an oversight.** A seventh cause
/// would fit `DEATH_CAUSE_BITS` (3 bits, six values spent) and move no
/// layout, and it is still a **wire change**: an eighth bit pattern both
/// ends currently refuse as forged would become a live fact, so an old
/// client and a new server would disagree about a packet whose bytes are
/// identical. `protocol`'s `every_domain_fits_its_wire_field` pins
/// `live_max` for exactly that and says so in its own failure — it was
/// what refused `DEATH_BY_BULLET` when hitscan v0 tried to add one. The
/// bump, the goldens and the pin land together or not at all (wall 6), and
/// this slice does not bump. Renaming this constant is refused for a
/// different reason: `event_roles.rs` and `protocol/src/event.rs` narrate
/// the 2026-08-05 failure by this name, and a rename would falsify three
/// histories to tidy one word.
///
/// **That refusal was conditional and the condition is now met.** It said
/// the bump, the goldens and the pin land together or not at all and that
/// *that* slice did not bump; arrow recovery v1 bumps, so
/// `DEATH_BY_BULLET` lands in this merge window on that bump, exactly the
/// way `DEATH_BY_MOB` and `DEATH_BY_CHARGE` did on v36's. This constant is
/// the arrow's again, and now only the arrow's.
pub const DEATH_BY_ARROW: u8 = 3;
/// An animal's bite (mob.rs — the pig that fights back). `death_by` is
/// the roster slot's **tagged** id (`mob::mob_id`), which is how the
/// death screen knows to name a species instead of printing a player
/// number; `death_item` is `NO_ITEM`, because a boar holds nothing. The
/// fifth cause, and the one the 2 → 3 bit widening at wire v36 was for.
pub const DEATH_BY_MOB: u8 = 4;
/// A satchel blast (charge.rs — satchel blast v0). `death_by` is the
/// planter, who may be the victim: standing at your own bomb is a
/// sentence of its own on the death screen. `death_item` is `NO_ITEM` —
/// the bomb already went with the plant — and `death_range_cm` is the
/// distance from the epicentre, which is the whole story of a blast
/// death the way range is of an arrow's.
pub const DEATH_BY_CHARGE: u8 = 5;

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
/// (And it was not an accident when it happened: `DEATH_BY_MOB` is the
/// cause that saturated the two-bit field, and wire v36 widened it;
/// `DEATH_BY_CHARGE` landed in the same merge window on the same bump.)
/// A firearm's hitscan round (`ranged::hitscan`). `death_by` is the
/// shooter, `death_item` the weapon, `death_range_cm` the distance the
/// shot crossed — the same three facts an arrow death carries, which is
/// why this is a seventh *cause* and not a seventh event.
///
/// **The seventh, and it was refused once.** `DEATH_BY_ARROW`'s doc holds
/// the refusal in full: a cause fits `DEATH_CAUSE_BITS` with room to
/// spare, but minting one turns a bit pattern both ends currently refuse
/// as forged into a live fact, so an old client and a new server would
/// disagree about a packet whose bytes are identical. That is a wire
/// change with no layout move — the hardest kind to notice — and the
/// answer is the `PROTO_VER` bump, the regenerated goldens and the
/// `live_max` pin in one commit. Until arrow recovery v1 there was no
/// bump to ride, and a firearm kill told the victim they were shot with
/// an arrow for twenty-three days.
pub const DEATH_BY_BULLET: u8 = 6;

pub const DEATH_BY_MAX: u8 = DEATH_BY_BULLET;

/// Where in the day/night cycle a tick falls, `0.0..1.0` — 0 is dawn,
/// `limits::DAY_PORTION` is dusk (day/night v0, `DECISIONS.md` §open).
///
/// A pure function of the tick, deliberately (`limits::DAY_TICKS`' doc has
/// the argument): the client derives it from the smoothed tick estimate it
/// already keeps, so no wire byte carries it, and gameplay that reads the
/// clock calls this same function on the sim's own tick and stays
/// deterministic for free.
///
/// **The sim reads it now** — [`is_night`] is the door, and `mob::think`
/// walked through it (nocturnal senses, 2026-08-14). The bet this doc used
/// to hedge on has been called: the curve is a divergence surface today,
/// not just a look, which is why `is_night` exists as one comparison rather
/// than as a threshold each caller writes for itself.
///
/// Wall 1: one modulo and one division, no trig — the sun curve the
/// renderer builds from this is the renderer's own.
pub fn day_frac(tick: u64) -> f32 {
    use crate::limits::{DAY_PHASE_TICKS, DAY_TICKS};
    ((tick.wrapping_add(DAY_PHASE_TICKS)) % DAY_TICKS) as f32 / DAY_TICKS as f32
}

/// Is this tick after dusk? The one place the day/night boundary is a
/// comparison, for both the sim and the renderer.
///
/// It exists because the renderer's bird layer had already open-coded the
/// threshold (`render/audio.rs` compared `day_frac` against `DAY_PORTION`
/// itself), and the moment the sim wanted the same answer a second hand-
/// written `<` would be a determinism surface keyed off a boundary two
/// files could disagree about. Both call this now, so a species is
/// nocturnal on exactly the ticks the birds stop, by construction rather
/// than by two matching literals.
///
/// `render/rig.rs::sun_elevation` still names `DAY_PORTION` and is
/// deliberately not a caller: it takes a *fraction*, not a tick, and uses
/// the constant as the breakpoint between two half-sine arcs rather than
/// as a question about an hour. Same number, different job.
///
/// Dusk itself (`frac == DAY_PORTION`) is night — the half-open convention
/// the renderer's own curve already used, so the sun is at the horizon on
/// the first night tick and not on the last day one.
pub fn is_night(tick: u64) -> bool {
    day_frac(tick) >= crate::limits::DAY_PORTION
}

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
    /// Worn equipment, **indexed by slot**: `[0]` is the head and `[1]` the
    /// body (`combat::WEAR_HEAD`/`WEAR_BODY`, one-based in the baked row).
    /// A piece in the wrong index protects nobody — `combat::worn_pct`
    /// checks the baked slot against the array index rather than trusting
    /// the placement — which is what holds "one piece per slot" up before
    /// any verb exists to enforce it.
    ///
    /// **Written by the move verb since armor v1** (wire v51). This said
    /// "nothing in the sim writes this yet, and that is the honest state
    /// of it" from 2026-08-19, and priced the fix: a `CONT_WEAR` kind, a
    /// `CONT_KIND_BITS` widening, a `PROTO_VER` bump. That is what landed,
    /// so the writers are now `move_item` (through `set_cont_slot`), a
    /// save, and tests — in that order of importance.
    ///
    /// The invariant above still does the work it always did, and now it
    /// has a second enforcer rather than none: `worn_pct` ignores a piece
    /// in the wrong index, and `combat::wearable_in` refuses to put one
    /// there. Both are spelled `slot == index + 1`, deliberately.
    pub worn: [ItemStack; WEAR_SLOTS],
    /// Tick the next swing is allowed at (gather.rs cadence).
    ///
    /// **Reload v1 pays here too, deliberately.** A reload sets this
    /// forward by `RangedDef::reload_ticks`, so the one field already
    /// shared by gather, melee and both ranged paths is also the beat you
    /// are helpless for: a reload stops a swing and a swing stops a
    /// reload, with no second clock to keep in step and nothing new in
    /// the hash.
    pub next_swing: u64,
    /// Rounds loaded, indexed by `RangedDef::mag_slot` — **not** by
    /// inventory slot and **not** by item index.
    ///
    /// Keyed by the weapon row, so nothing has to move it: a gun that
    /// travels from the hotbar to a box to the ground and back keeps its
    /// count, because the count was never attached to where the gun was.
    /// That is the whole reason it is not a field on `ItemStack` — the
    /// stated cost is in `RangedDef::mag_slot`, and `tests/reload.rs`
    /// gates it.
    ///
    /// Zero on a fresh body, so a spawned revolver starts empty and the
    /// first thing a player does with a gun is load it.
    pub mag: [u16; MAX_MAGS],
    /// Which round is in each magazine, `NO_ITEM` when empty — parallel to
    /// [`Player::mag`].
    ///
    /// A weapon lists up to `MAX_WEAPON_AMMO` rounds in preference order,
    /// so "six loaded" is not a fact until you say six of *what*: the
    /// magazine has to remember, or a reload that topped up a partly-full
    /// cylinder with a different round would fire the wrong damage and an
    /// unload would return the wrong item to the pack. `ranged::reload`
    /// refuses to mix — a magazine holding one kind tops up with that kind
    /// or not at all.
    pub mag_round: [u16; MAX_MAGS],
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
    /// The sub-point remainder of a burning torch (torch fuel v0,
    /// `light.rs`). Hundredths×ticks, drained against `light::BURN_DEN`.
    ///
    /// The **only** state a flame has, because a flame is derived rather
    /// than stored — `light::is_lit` reads the latch, the content row and
    /// the fuel, and there is no `lit` bit for the two ends of the wire to
    /// disagree about. This is here for the reason `food_acc` is: a
    /// remainder that reset on a restore would hand back a fraction of a
    /// torch on every reconnect, which is a small exploit and an exact-
    /// arithmetic bug (`persist.rs`).
    pub light_acc: u32,
}

impl Default for Player {
    fn default() -> Self {
        Self {
            id: 0,
            active: false,
            body: Body::default(),
            frame: InputFrame::default(),
            inv: [ItemStack::default(); INV_SLOTS],
            worn: [ItemStack::default(); WEAR_SLOTS],
            next_swing: 0,
            mag: [0; MAX_MAGS],
            // `NO_ITEM` and not 0, for the reason `RangedDef::ammo` is
            // `NO_ITEM`-padded: item 0 is a real item, so a zeroed round
            // array would say every empty magazine holds one.
            mag_round: [NO_ITEM; MAX_MAGS],
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
            light_acc: 0,
        }
    }
}

/// Every mutation the sim accepts. The WAL is exactly this stream plus the
/// tick numbers (DESIGN.md §7).
#[derive(Clone, Copy, Debug)]
// `JoinAs` carries a `PlayerSave` BY VALUE, and that is `persist.rs`'s own
// stated contract — "`Copy` and POD like every other record that crosses a
// boundary here, so it rides a `Command` and an SPSC ring without an
// allocation." Boxing it to please the variant-size lint would put an
// allocation on the command path and a deallocation inside the tick that
// consumes it, which is the exact traffic wall 2's counting allocator
// exists to refuse. The size is priced, not accidental.
#[allow(clippy::large_enum_variant)]
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
    /// Move `id`'s body to `to`'s feet — the admin lane's travel verb
    /// (admin v0, `ALPHA.md` §3).
    ///
    /// **Not reachable from the wire**, `Evict`'s posture and for a
    /// stronger reason: no `ActionMsg` maps here, so a client cannot ask
    /// for it however it forges its bytes, and the only mint site is the
    /// server's admin dispatch behind a wallet allowlist. It is a
    /// `Command` rather than a poke at `world.players` from outside
    /// because **the WAL is the command stream** (`Command::JoinAs`'
    /// argument): a body that moved by side channel would replay as a
    /// body that never moved, and every hash after it would differ. An
    /// admin act being *visible in a replay* is also what `ALPHA.md` §3
    /// asks for in the same breath as the lane.
    ///
    /// A miss — either id naming nobody live — is a legal no-op, `Wake`'s
    /// posture, because a WAL replayed against a world that diverged must
    /// refuse rather than teleport somebody into a hole.
    ///
    /// The destination is the *target's* body, not a coordinate, and that
    /// is the whole safety story: an arbitrary xyz would need bounds, a
    /// walkability check and a "you are now inside a rock" answer;
    /// somewhere a player is already standing is known-good ground.
    AdminTeleport {
        id: u32,
        to: u32,
    },
    /// Put `count` of item row `item` in `id`'s inventory — the admin
    /// lane's other verb, `AdminTeleport`'s posture in every respect
    /// (not wire-reachable, minted only behind the allowlist, a command
    /// so it replays).
    ///
    /// Overflow is `gather::inv_add`'s documented policy and not this
    /// command's business: a full inventory keeps what fits.
    AdminGive {
        id: u32,
        item: u16,
        count: u16,
    },
    /// One client input frame, plus how many ticks of lag compensation the
    /// server is willing to grant this player's verbs *this tick*.
    ///
    /// `favour` is **not on the wire** and never will be: it is minted
    /// server-side from the client's `snapshot_ack` (slice 5,
    /// `findings/lagcomp-design-20260818.md` §2.2), so a client cannot ask
    /// for a rewind depth. Every non-server construction — bots, probes,
    /// tests, a replayed WAL — passes `0`, which is the present tick and
    /// therefore the pre-lag-compensation behaviour bit for bit.
    ///
    /// It rides on the command rather than on `Player` on purpose: the
    /// value is spent inside one tick, so storing it would be storing a
    /// fact `state_hash` has to answer for. `apply` clamps it into a
    /// tick-local array — the `removals` precedent, see `World::tick`.
    Input {
        id: u32,
        frame: InputFrame,
        favour: u8,
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
    /// Learn recipe row `recipe` through the tech tree (tech tree v0):
    /// at a workbench of the node's tier, along the `requires` graph,
    /// paying the row's coin — no sample needed (research.rs says why
    /// the two verbs stay asymmetric). The recipe index is the sender's
    /// claim and the sim is the verdict, as `Research`'s slot is.
    Unlock {
        id: u32,
        recipe: u16,
    },
    /// Cancel the queue job at `index`, refunding its remaining inputs.
    CraftCancel {
        id: u32,
        index: u16,
    },
    /// Place baked building-piece row `row` at grid address (cx, cz,
    /// level, loc) (build.rs validates and refuses by event, never by
    /// panic).
    ///
    /// `freehand` declines the plate latch — see [`build::plate_for`]. It
    /// rides the command rather than being re-derived because the server
    /// knows which neighbour is built and cannot know which floor the
    /// player wanted (freehand placement v0).
    Place {
        id: u32,
        row: u16,
        cx: u16,
        cz: u16,
        level: u8,
        loc: u8,
        freehand: bool,
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
    /// Take the nearest ready spent arrow in reach back into the quiver
    /// (spent.rs — arrow recovery v1, the verb `take_near` was gated for).
    ///
    /// `Loot`'s shape and for `Loot`'s reason: no target crosses, so there
    /// is no id to forge and nothing can be picked up that the sender is
    /// not standing on. It is the first verb whose pick is resolved in
    /// **three** dimensions — an arrow lodged up a trunk is out of reach
    /// where a backpack at your feet never is.
    Pickup {
        id: u32,
    },
    /// Open the authored world container at cell key `cont`
    /// (`worldcont.rs`) — the haven pad's crate, a waystation's cache.
    ///
    /// **The only container verb that enters the sim at all.** Opening a
    /// bag or a box is pure server-side subscription (`open_container`):
    /// the contents already exist, so a view of them changes no state and
    /// mints no command. A world container's contents do *not* exist until
    /// somebody opens it — the roll is the state change — so this one has
    /// to be a `Command`, has to be in the WAL, and has to be in
    /// `state_hash`. A replay that skipped it would replay a shard whose
    /// crates were all still full.
    ///
    /// Payload is the cell key alone, and the sim treats it as a claim:
    /// `terrain::scatter` re-derives what actually stands there, so a
    /// handle naming an empty meadow opens nothing. Reach is re-proved
    /// here and again on every tick the panel is open.
    OpenWorldCont {
        id: u32,
        cont: u32,
    },
    /// Move `count` items between two slots (`inventory.rs`). `cont` names
    /// the one ground container this move touches — a bag id for
    /// `CONT_BAG`, a packed `deploy::box_key` address for `CONT_BOX`, a
    /// packed `gather::cell_key` for `CONT_WORLD`, zero when the move is
    /// inside the sender's own inventory.
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
    /// Fill the held weapon's magazine from the pack (`ranged::reload`).
    ///
    /// No target and no count: the weapon is whatever is in the selected
    /// hotbar slot and the amount is however much the cylinder takes, so
    /// there is nothing here a client could forge that the sim would not
    /// have decided for itself. That is deliberate and it is why this is
    /// an action rather than a button bit — a reload is a one-shot intent
    /// with a *duration*, and `BTN_LIGHT`'s doc gives the rule: a level
    /// both sides must agree on every tick is a bit, and a one-shot the
    /// world has to acknowledge is a message.
    Reload {
        id: u32,
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
    /// Where every body stood for the last `REWIND_TICKS` ticks
    /// (`rewind.rs`) — lag compensation's store, and the **one field on
    /// `World` that is neither hashed nor saved**.
    ///
    /// It is derived from state that already is both: the row for tick `T`
    /// is a copy of poses `state_hash` covered at `T`, so two shards
    /// agreeing on every hash from tick 0 hold identical rings by
    /// construction. That is `Pieces::cols`' argument (*"Derived, never
    /// hashed"*) and the event ring's, and it is why hashing this would be
    /// adding a second name for a fact the hash already carries.
    ///
    /// Not saved either, on `worldsave.rs`'s arrows-in-flight precedent —
    /// but unlike `cols` it is **not reconstructible at load**, so what
    /// keeps wall 5 whole is `Rewind::pose_at`'s fallback to the live body
    /// on a stamp that is not the tick asked for. Read that module's header
    /// before touching it; the fallback looks like a rough edge and is the
    /// design.
    ///
    /// **Read by `combat::strike` since slice 4** of
    /// `findings/lagcomp-design-20260818.md` §7 — the melee target scan
    /// resolves against `pose_at` at the tick's granted `favour` — and by
    /// **`ranged::hitscan` since the gun's slice**, which closes the
    /// asymmetry the line here used to describe: for one pass melee
    /// rewound and the firearm did not, so the only fight decided by ping
    /// was the ranged one, where lead error is largest.
    ///
    /// `ranged::step` still resolves against present-tick bodies and that
    /// is now a **refusal with a type behind it** (`ranged::Pose::Live`),
    /// not an omission: an arrow in the store was launched on an earlier
    /// tick. Its *launch* aim is the one question left and it is in
    /// `DECISIONS.md` §open rather than in a findings note nobody re-reads.
    pub rewind: crate::rewind::Rewind,
    /// Authored world containers a player has opened — sim state, hashed
    /// (`worldcont.rs`). Boxed inside, for `backpacks`' reason: 64 records
    /// of `INV_SLOTS` stacks is ~8.7 kB of fixed capacity on a stack-built
    /// `World`. Nothing here allocates in the tick, and nothing here runs
    /// in the tick at all — a world container is touched only by the verb
    /// that opens it and the move that empties it.
    pub world_conts: crate::worldcont::WorldConts,
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
    /// Arrows that have landed and can be taken back — sim state, hashed
    /// and **saved**, which is the pair's whole distinction (`spent.rs`).
    /// One field up is a trajectory between two ticks and `worldsave.rs`
    /// drops it on purpose; this is an item lying on a hillside, and
    /// dropping it across a restart would delete ammunition a player
    /// earned. Boxed for `slot_cache`'s reason.
    pub spent: Box<crate::spent::SpentArrows>,
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
            rewind: crate::rewind::Rewind::new(),
            world_conts: crate::worldcont::WorldConts::new(),
            sweep_piece: 0,
            sweep_deploy: 0,
            sweep_support: 0,
            evictions: 0,
            slot_lives: SlotLives::new(),
            slot_cache: Box::new(crate::occupy::SlotCache::new()),
            arrows: Box::new(ranged::Arrows::new()),
            spent: Box::new(crate::spent::SpentArrows::new()),
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

    /// Was this player online — [`PRESENCE_AWAKE`], [`PRESENCE_ASLEEP`] or
    /// [`PRESENCE_GONE`] (`EV_TRUST`'s low byte).
    ///
    /// The linear scan is `slot_of`'s, and deliberately not a smarter
    /// lookup: a player id is minted `generation << 8 | slot` by the
    /// **server**, and sim-core does not know that — deriving a slot from
    /// an id here would be this crate learning a convention it is not
    /// allowed to depend on, and it would be silently wrong the first time
    /// the minter changes. `MAX_PLAYERS` compares on a door press is
    /// bounded work in a tick that already did a reach check.
    ///
    /// A corpse reads awake on purpose: the death screen is a player
    /// watching, and the question this answers is whether the act had a
    /// witness.
    pub fn presence_of(&self, id: u32) -> u8 {
        match self.players.iter().find(|p| p.active && p.id == id) {
            None => PRESENCE_GONE,
            Some(p) if p.sleeping => PRESENCE_ASLEEP,
            Some(_) => PRESENCE_AWAKE,
        }
    }

    /// Push one `EV_TRUST` row: `actor` exercised `verb` against a record
    /// `counterparty` owns, and this is whether they were there to see it.
    ///
    /// **The one place the "is this a counterparty at all" rule lives**,
    /// so the four emit sites cannot disagree about it. Three ids are not
    /// counterparties and each is silent for its own reason:
    ///
    /// - `0` — nobody placed it. Every deployable in the world today comes
    ///   from a player (`NOW.md` §4b), so this is the authored-site case
    ///   arriving early rather than a live path; it is here because the
    ///   record must not gain a row whose subject is the number zero the
    ///   day one lands.
    /// - `actor` — your own door, your own hearth, your own box. A verb on
    ///   your own thing creates no trust relationship, and logging it would
    ///   bury the rows that are the measurement under the rows that are
    ///   ordinary play.
    /// - a `mob::mob_id` — a corpse bag carries the dead animal's tagged
    ///   id where a player's would be (`EV_BAG_DROPPED`). A boar is not a
    ///   counterparty, and without this check every skinned carcass would
    ///   log one against a player number that does not exist.
    fn log_trust(&mut self, actor: u32, counterparty: u32, verb: u8) {
        if counterparty == 0
            || counterparty == actor
            || crate::mob::slot_of_id(counterparty).is_some()
        {
            return;
        }
        let presence = self.presence_of(counterparty);
        self.events.push(
            EV_TRUST,
            actor,
            counterparty,
            ((verb as u32) << 8) | presence as u32,
        );
    }

    /// One slot of whichever container `kind` names, by value. `ci` is the
    /// resolved ground-container index and is only read when the kind is a
    /// ground container — the caller has already proved it resolves.
    ///
    /// **`pub` because it is the only answer to "what is in slot `s` of
    /// this kind", and a second answer is a shipped defect.** It was
    /// private until 2026-08-14, so the server's per-tick container drip
    /// could not call it and re-implemented the dispatch as a two-way
    /// `if kind == CONT_BAG { backpacks } else { deploys }`
    /// (`server/core.rs`). That was correct for the two kinds alive when
    /// it was written and silently wrong the moment a third landed:
    /// `CONT_WORLD` fell through the `else` and read `deploys.box_slot`
    /// with a `world_conts` index — an index that is always in range
    /// (`MAX_WORLD_CONTS` 64 < 1 024 deploys) and therefore never panics,
    /// so opening the pad's crate drew a *deploy box's* contents with
    /// every gate green. The kinds are wire `u8` constants, so no
    /// exhaustive `match` can be made to catch the next one; the only
    /// structural defence is that there is one function, and it is this
    /// one. Call it — do not re-derive it.
    pub fn cont_slot(&self, slot: usize, kind: u8, s: u8, ci: usize) -> ItemStack {
        match kind {
            inventory::CONT_BAG => self.backpacks.slot(ci, s as usize),
            inventory::CONT_BOX => self.deploys.box_slot(ci, s as usize),
            inventory::CONT_WORLD => self.world_conts.slot(ci, s as usize),
            // Armor v1. This arm is exactly the defect the paragraph above
            // describes, caught before it shipped rather than after: the
            // `_` fallback is `inv`, so a `CONT_WEAR` reaching it would
            // read and write the *inventory* array under a wear address —
            // in range, never a panic, green everywhere, and the helmet
            // you dragged onto your head would land in backpack slot 0.
            // The wrong store here is a *plausible* store, which is what
            // makes it worse than the `CONT_WORLD` case above it.
            // `container_wire`'s wear test is the mutant proof.
            inventory::CONT_WEAR => self.players[slot].worn[s as usize],
            _ => self.players[slot].inv[s as usize],
        }
    }

    /// The mirror of `cont_slot`, and the only place a move writes.
    fn set_cont_slot(&mut self, slot: usize, kind: u8, s: u8, ci: usize, v: ItemStack) {
        match kind {
            inventory::CONT_BAG => self.backpacks.set_slot(ci, s as usize, v),
            inventory::CONT_BOX => self.deploys.set_box_slot(ci, s as usize, v),
            // The one kind whose write needs the clock: emptying the last
            // stack arms the refill timer, and the jitter is the barrel's
            // own so a crate and a barrel come back on one schedule
            // (`gather::RESPAWN_*`, DECISIONS.md §open "node/barrel
            // respawn 20–45 min" — reused, not re-spoken).
            inventory::CONT_WORLD => {
                let c = self.world_conts.entries()[ci];
                let refill = crate::worldcont::refill_ticks(self.seed, c.cx, c.cz, self.tick);
                self.world_conts
                    .set_slot(ci, s as usize, v, self.tick, refill);
            }
            inventory::CONT_WEAR => self.players[slot].worn[s as usize] = v,
            _ => self.players[slot].inv[s as usize] = v,
        }
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
    //
    // (This block sat on `cont_slot` until 2026-08-14 — a doc comment
    // attaches to the item that follows it, and an intervening helper had
    // been added above `move_item` without moving it down.)
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
        //    "Ground" is `!is_own`, not `!= CONT_SELF`: since armor v1 a
        //    player carries two containers, and a wear slot has no handle
        //    to be named by and nothing to be out of reach of. Spelled the
        //    old way, `CONT_SELF` -> `CONT_WEAR` would have been read as a
        //    move into a ground container, handed whatever handle the
        //    command carried, and refused as `REFUSE_M_NO_CONTAINER`.
        if !inventory::is_own(from_kind) && !inventory::is_own(to_kind) && from_kind != to_kind {
            self.events
                .push(EV_MOVE_REFUSED, pid, inventory::REFUSE_M_NO_CONTAINER, addr);
            return;
        }
        let ground = if !inventory::is_own(from_kind) {
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
            // A world container resolves by cell key, and — unlike a bag
            // and a box — a handle that resolves to nothing is the
            // *ordinary* case rather than a lost container: it means
            // nobody has opened this crate yet, so no record exists. The
            // move still refuses. `open` is the only path that mints one,
            // because minting is where the loot is rolled, and rolling
            // from inside the move verb would let a client skip the reach
            // and occupant checks that `open` is made of.
            inventory::CONT_WORLD => {
                let Some(i) = self.world_conts.index_of(cont) else {
                    self.events
                        .push(EV_MOVE_REFUSED, pid, inventory::REFUSE_M_NO_CONTAINER, addr);
                    return;
                };
                if !self.world_conts.in_reach(i, &self.players[slot]) {
                    self.events
                        .push(EV_MOVE_REFUSED, pid, inventory::REFUSE_M_REACH, addr);
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
        // A wear slot takes the piece that goes in it and nothing else
        // (`CanWearItem`, `combat::wearable_in`). Asked here for the same
        // reason the oven is: it is a rule about what a container accepts,
        // which is a property of content, and `plan_move` decides
        // arithmetic and only arithmetic.
        //
        // **Both landing sites, not just the destination.** A move's
        // result is two writes, and when the destination is occupied
        // `plan_move` answers SWAP — so `dst` travels backwards into
        // `from_slot`. Check only the forward half and a body piece
        // dragged from the body slot onto a helmet puts the helmet in the
        // body slot: refused in one direction, admitted in the other, by
        // one verb. The source side is guarded on `dst.count > 0` because
        // that is exactly when a swap can happen; an empty destination
        // falls through to `plan_move`'s own ladder with the reason it
        // would have given anyway, and `src.count > 0` leaves an empty
        // source to `REFUSE_M_EMPTY` rather than answering it here.
        let wear_refused = (to_kind == inventory::CONT_WEAR
            && src.count > 0
            && !combat::wearable_in(&self.combat, src.item, to_slot))
            || (from_kind == inventory::CONT_WEAR
                && dst.count > 0
                && !combat::wearable_in(&self.combat, dst.item, from_slot));
        if wear_refused {
            self.events
                .push(EV_MOVE_REFUSED, pid, inventory::REFUSE_M_WEAR, addr);
            return;
        }

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
        // Whose container this was — the trust row for the move verb
        // (`EV_TRUST`). Read here rather than at the top, because it must
        // be read after the move succeeded and before `drop_if_empty`
        // below can take the bag's record out from under it.
        //
        // A world container is deliberately absent: a loot crate is
        // nobody's, so a hand in one answers to nobody. `CONT_SELF` to
        // `CONT_SELF` leaves `ground` at `CONT_SELF` and falls through the
        // same way — arranging your own hotbar is not a relationship.
        let owner = match ground {
            inventory::CONT_BOX => Some(self.deploys.boxes()[ci].owner),
            inventory::CONT_BAG => Some(self.backpacks.entries()[ci].owner),
            _ => None,
        };
        if let Some(owner) = owner {
            self.log_trust(pid, owner, TRUST_CONT);
        }
        // A bag a withdrawal emptied leaves by `loot_nearest`'s route, so
        // the wire sees one removal contract however it was emptied. A box
        // does **not** take that route and that is the difference between
        // the two: a bag is litter with a timer, a box is furniture you
        // paid for and placed. Emptying one leaves it standing.
        if ground == inventory::CONT_BAG {
            self.backpacks.drop_if_empty(ci, &mut self.events);
        }
    }

    /// Charge one shot's structure damage to the piece **or deployable** it
    /// stopped on — `ranged`'s half of the raid verb, written here for the
    /// reason `Chip` states: the shot pass holds the collision index, and
    /// this holds both stores, the content and the tick's removal budget.
    ///
    /// **The address is re-resolved, never carried as an index.** Between
    /// the walk that found this piece and this line sit the rest of the
    /// arrows, every other player's bullet and — on the arrow pass — the
    /// bodies `die` has already laid down, any of which can drop a piece
    /// and swap-remove another into its slot (`Pieces::remove_at`). A hit
    /// whose address no longer holds a piece is simply no longer a hit; the
    /// shot still stopped there and still drew its `EV_IMPACT`.
    ///
    /// **Hard and soft sides apply exactly as they do to a swing**
    /// (`combat::raid`, hard/soft v0): a shot meeting a sided piece on its
    /// hard face lands `HARD_SIDE_STRUCTURE` whatever fired it. Sharing the
    /// law rather than restating it is the point — otherwise a bow pays a
    /// price a hatchet pays or does not, depending on which file was
    /// written first. That sharing is literal since 2026-08-28: both call
    /// `build::structure_price`, where ranged structure damage v0 had
    /// copied the three lines and *said* they were shared. What differs is
    /// only *whose* position names the side: `raid` asks where the attacker
    /// stands, a shot asks where the shot came from.
    ///
    /// **A deployable takes it flat, and that is not an omission.** The
    /// deploy arm pays no side price and spends no removal budget because
    /// neither exists for it anywhere else: `combat::raid`'s own
    /// `Target::Deploy` arm and `charge::detonate` both hand
    /// `damage_deploy` the raw number, a box has no facing to be on the
    /// wrong side of, and `drop_deploy` collapses nothing so there is no
    /// cascade for a budget to bound. Adding either here would make a shot
    /// the one verb in the game that prices a furnace differently.
    ///
    /// Bounded: one `find_index` walk and one damage write per chip, and
    /// the chip array is capped at `MAX_ARROWS`. The removal budget is the
    /// same allowance a swing spends, so an arrow cannot drop a piece past
    /// the cap that bounds every other remover.
    fn chip(&mut self, c: &ranged::Chip, removals: &mut usize) {
        if c.deploy {
            // Same re-resolve, other store — `charge::detonate`'s two-arm
            // shape, and for its stated reason: an address cannot go
            // stale, an index can.
            let Some(i) = self
                .deploys
                .find_index(c.hit.cx, c.hit.cz, c.hit.level, c.hit.loc)
            else {
                return;
            };
            deploy::damage_deploy(
                &self.deploy,
                &mut self.pieces,
                &mut self.deploys,
                i,
                c.structure,
                &mut self.events,
            );
            return;
        }
        let Some(i) = self
            .pieces
            .find_index(c.hit.cx, c.hit.cz, c.hit.level, c.hit.loc)
        else {
            return;
        };
        let rec = self.pieces.entries()[i];
        let amount = crate::build::structure_price(
            &rec,
            self.build.pieces[rec.row as usize].shape,
            c.from_x,
            c.from_z,
            c.structure,
        );
        deploy::damage_piece(
            &self.deploy,
            &self.build,
            &mut self.pieces,
            &mut self.deploys,
            i,
            amount,
            removals,
            &mut self.events,
        );
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
            // Carried for a different reason, and it is the one this
            // spread had already got wrong once. The backpack takes the
            // inventory, so a corpse holding nothing is correct — but a
            // blueprint is not carried, it is *known*, and `..default()`
            // cleared it here at the instant of death, one door before
            // `wake` could have carried it. Fixing only `wake` would have
            // looked right and shipped a zero.
            //
            // Found 2026-08-15 by `event_roles.rs`'s `known_names_the_
            // holder…`, which starves a body for real rather than setting
            // `dead` by hand — the hand-set version of the same test
            // passed, because it never came through this function.
            known: body.known,
            ..Player::default()
        };
        // **What it was wearing goes into the bag with what it was
        // carrying**, and that is the decision `tests/persist.rs`'s field
        // ledger forces into a test rather than into a reviewer's
        // attention. Armor as loot is what makes killing an armored player
        // worth doing; a corpse that keeps its plates is a body nobody
        // fights for. So `worn` is *not* named in the literal above —
        // `..Player::default()` clears it, exactly as it clears `inv` —
        // and this is where it lands instead.
        //
        // Through `drain_spill` rather than by widening `drop_for`'s
        // buffer, and the difference is whether an item can be destroyed.
        // A bag holds `INV_SLOTS`, a full pocket plus two plates is
        // `INV_SLOTS + WEAR_SLOTS`, and merging into the array `drop_for`
        // copies wholesale would drop whatever did not fit. `spill_at`
        // merges into the bag standing in reach — the one the line above
        // just stood up, at distance zero — and stands a second one up for
        // the remainder, so the pack being full costs the killer a walk
        // and never an item. It is the same drain the six existing
        // producers use; this is the seventh, and it is named here for the
        // reason `backpack.rs`'s header names the others.
        let mut shed = [ItemStack::default(); INV_SLOTS];
        shed[..WEAR_SLOTS].copy_from_slice(&body.worn);
        self.drain_spill(slot, &mut shed);
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
    /// (its doc says so, and `browser_smoke` used it to put two tabs on
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
        // `known` is the fifth thing a body carries through a death, and
        // it is carried for the same reason `deaths` is: both are ledgers
        // of what the player *did*, not of what they were holding when
        // they fell. A blueprint is bought with JUNK, and JUNK is the
        // scarcest thing on the shard.
        //
        // **It was not carried until 2026-08-15, so dying deleted every
        // blueprint you had paid for.** The mask landed at research v0
        // into `Player`, `Default`, `PlayerSave`, `state_hash` and
        // `worldsave.rs`, and the `..Player::default()` below answered
        // "no, a body does not keep that" without anybody deciding it —
        // the spread's silence is the whole defect. Death is the most
        // common event in the game, so this was the JUNK sink emptying
        // itself on a timer. The gates are `research.rs`'s
        // `a_blueprint_survives_a_death`, `persist.rs`'s
        // `the_carried_decisions_survive_a_real_death`, and
        // `event_roles.rs`'s
        // `known_names_the_holder_then_the_mask_low_half_first`;
        // `persist.rs`'s `every_player_field_is_classified_across_a_death`
        // is why the next field cannot land here silently.
        //
        // (This comment named a `combat.rs` test that has never existed —
        // corrected 2026-08-15. A citation is a claim that something is
        // enforced, so an invented one reads as covered while nothing
        // checks it, which is `CLAUDE.md`'s dead-citation ⚠ exactly. The
        // check when you write one is `ls`, or a grep for the `fn`.)
        let known = body.known;
        self.players[slot] = Player {
            id,
            active: true,
            body: Body::at(self.seed, &self.haven, x, z),
            frame,
            hp,
            hp_max: hp,
            deaths,
            known,
            ..Player::default()
        };
        // A player who starved does not respawn already starving.
        survival::grant(&self.survival, &mut self.players[slot]);
        // **And a player who died does not respawn unable to play**
        // (DECISIONS.md 2026-08-15; `NOW.md` §0die mechanism 3).
        //
        // The kit was fresh-arm-only until here, and `inventory.rs` gave
        // the reason: re-granting "would be an item printer". That was
        // correct arithmetic against a kit worth 900 wood, 500 stone and
        // 100 metal frags, and it is void against one worth a rock and a
        // torch — 10 stone of craftable tools, which is less than one
        // swing of the node the rock opens. What the old reasoning bought
        // instead was the compound §0die names: the inventory drops into a
        // bag where you fell, the bag despawns on its rarity timer, and no
        // kit ever comes back — so one death ended a session for good.
        //
        // **`wake` is where a new body is built**, and this sits beside
        // `survival::grant` for exactly its reason: both restore the floor
        // a body needs to be playable, and neither restores anything the
        // player earned. So the rule is *once per BODY*, not once per
        // character and not once per login.
        //
        // ⚠ **`Command::Respawn` is not the only caller, and a comment
        // here said it was until 2026-08-15.** `grep -n '\.wake(' ` on this
        // file returns three: the respawn command, `seat`'s restore arm
        // when the save is `dead` (logged off on the death screen), and the
        // sleeper takeover when the body it takes over is dead. All three
        // rebuild the body from `..Player::default()`, so all three are new
        // bodies and all three are paid — and since 2026-08-17 all three
        // are gated: `tests/bag_respawn.rs` and `server/tests/spawn_kit.rs`
        // drive `Command::Respawn`;
        // `persist.rs::a_dead_save_wakes_holding_the_spawn_kit` drives the
        // dead restore; `sleepers.rs::
        // a_takeover_of_a_dead_body_wakes_holding_the_spawn_kit` drives the
        // takeover. Each of the latter two is proven red both under an
        // early return at its own door and under deleting this `grant_kit`
        // call alone — the mutation the doors' older position/hp gates
        // cannot see.
        //
        // A returning login whose save is ALIVE is the different door and
        // still grants nothing, because a saved character keeps what it
        // saved.
        //
        // It writes slots in order and does not merge, so a bag-spawn
        // respawn that already looted itself gets its slot-0 and slot-1
        // overwritten. That is `grant_kit`'s documented shape rather than
        // an oversight here; the alternative — merging — would make the
        // grant depend on what you were carrying, which is the item
        // printer this comment is about.
        inventory::grant_kit(&self.spawn_kit, &mut self.players[slot]);
        self.events.push(EV_RESPAWN, id, bag.is_some() as u32, 0);
        self.events.push(EV_HEALTH, id, hp as u32, hp as u32);
        survival::announce_vitals(&self.survival, &self.players[slot], &mut self.events);
        self.announce_known(slot);
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
                    body: Body::at(self.seed, &self.haven, x, z),
                    hp,
                    hp_max: hp,
                    ..Player::default()
                };
                survival::grant(&self.survival, &mut self.players[slot]);
                // The spawn kit, on the FRESH arm only — of this door. A
                // restore keeps what it saved; re-granting on every LOGIN
                // would be an item printer, which is the same reason
                // `survival::grant` is here and not below the match.
                //
                // ⚠ **A respawn is not a login and grants it too** since
                // 2026-08-15 (`wake`, above). Read the two together before
                // moving either: the rule is not "once per character", it
                // is "once per body" — a fresh body gets the floor it needs
                // to play, and a body that already exists gets nothing.
                // Death makes a new body; a login does not.
                //
                // ⚠ **"a restore gets nothing" is true of a LIVE restore
                // only**, which the arm below is not alone in being. A save
                // marked `dead` falls through `Some(s)` and then calls
                // `wake` (~60 lines down), so it takes the kit — correctly,
                // because declining the death screen is choosing the beach
                // and that is a new body. Stated because the sentence above
                // read as covering every restore until 2026-08-15, and the
                // counterexample sits inside the same `match`.
                inventory::grant_kit(&self.spawn_kit, &mut self.players[slot]);
            }
            Some(s) => {
                // Every field named, and no `..Player::default()` — this
                // is `worldsave.rs`'s discipline, moved here because this
                // door had the bug that one was written to prevent.
                //
                // `known` was missing from this list. `PlayerSave` carried
                // the blueprint mask correctly, the codec round-tripped it,
                // and the spread then quietly answered `known: 0` — so a
                // keyed player reconnecting after a restart lost every
                // blueprint they had paid JUNK for, with `test_replay`
                // blind to it (its stream has no `JoinAs`) and
                // `a_blueprint_survives_a_save_and_a_load` blind to it too
                // (it exercises the codec and never seats a world).
                //
                // Named rather than spread, so **a field added to
                // `PlayerSave` stops compiling here** and whoever adds it
                // has to decide whether a returning body remembers it. The
                // default silently answered "no" once already.
                //
                // The fields the save does not carry are still
                // `Player::default()`'s, and deliberately: the input frame,
                // the swing cooldown, the weak-spot chase, the craft timer
                // (re-armed below), the death record and the sleep flags.
                // They are written out here rather than spread so that the
                // list is a decision instead of an omission.
                self.players[slot] = Player {
                    id,
                    active: true,
                    body: s.body,
                    inv: s.inv,
                    // You log off in your armor, you log in in your armor
                    // — `hp`'s sentence, and the alternative would make
                    // closing the game a way to lose a plate.
                    worn: s.worn,
                    jobs: s.jobs,
                    known: s.known,
                    hp: s.hp,
                    hp_max: s.hp_max,
                    deaths: s.deaths,
                    food: s.food,
                    water: s.water,
                    food_acc: s.food_acc,
                    water_acc: s.water_acc,
                    hurt_acc: s.hurt_acc,
                    light_acc: s.light_acc,
                    heal_rem: s.heal_rem,
                    heal_total: s.heal_total,
                    heal_span: s.heal_span,
                    heal_acc: s.heal_acc,
                    // Not from the save, by design (`persist.rs` says
                    // which and why). `dead` is read below rather than
                    // stored: a body that logged off dead wakes on a
                    // beach, so `wake` owns that record, not this one.
                    dead: false,
                    frame: InputFrame::default(),
                    next_swing: 0,
                    // Not from the save, and for `next_swing`'s reason one
                    // line up rather than a new one: `PlayerSave` is the
                    // store's record of a body that has LEFT the world, and
                    // reload v1 did not widen it. A player who reconnects
                    // to a sleeper still standing in the world keeps their
                    // rounds — that body was never serialized through here
                    // — and one whose record came back off disk finds the
                    // cylinder empty and presses reload. Stated rather than
                    // silent; `NOW.md` §0rl carries the remainder.
                    mag: [0; MAX_MAGS],
                    mag_round: [NO_ITEM; MAX_MAGS],
                    // `NO_CELL`, not zero: the weak-spot chase names a
                    // cell and cell 0 is a real one, so a `0` here would
                    // restore a player already half-way through chasing
                    // the weak spot on whatever stands at the origin.
                    ws_cell: NO_CELL,
                    ws_hits: 0,
                    craft_done_at: 0,
                    death_by: 0,
                    death_cause: 0,
                    death_item: NO_ITEM,
                    death_range_cm: 0,
                    sleeping: false,
                    slept_at: 0,
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
        self.announce_known(slot);
    }

    /// State the blueprint mask, whole. The third thing said at a door,
    /// for the same reason as the first two: `known` is only ever
    /// announced when it changes, so without this a returning player has
    /// no blueprints until they buy one — and that is the one moment a
    /// greyed-out recipe they already own is worst.
    ///
    /// **Unconditional, unlike `EV_HEALTH` above.** An empty mask is a
    /// fact too, and the interesting case is a client-core reused across
    /// two characters: a stale mask left standing would offer recipes the
    /// new body has not earned, and the craft gate would then refuse them
    /// at the sim. Saying `0` out loud costs one event per door and makes
    /// the client's copy a statement rather than a residue.
    fn announce_known(&mut self, slot: usize) {
        let (id, mask) = (self.players[slot].id, self.players[slot].known);
        self.events
            .push(EV_KNOWN, id, mask as u32, (mask >> 32) as u32);
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
        // The body kept its mask — a takeover never rebuilt the record,
        // which is why this door was the one that did not lose it. The
        // announcement is owed anyway: the *connection* is new even when
        // the body is not, so the client arriving through it knows
        // nothing until told.
        self.announce_known(slot);
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
            // The solid nibble is the shut bit's twin and derived the same
            // way (deploy collision v0): `Pieces::restore` cleared the
            // index, so every standing body deploy re-blocks here or a
            // loaded shard's furniture is walk-through until re-placed.
            if crate::deploy::solid_vol(arch).is_some() {
                self.pieces.set_solid(d.cx, d.cz, d.level, Some(arch));
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

    /// **The one drain.** Hand whatever a verb could not fit into the
    /// player in `slot` to the ground under that body, merging into a bag
    /// already in reach before minting one. `spill` comes back empty; an
    /// all-empty buffer costs a scan of `INV_SLOTS` and nothing else.
    ///
    /// This is a function rather than two copies of nine lines because the
    /// number of producers went from two to six this pass (a node's yield,
    /// a finished craft, and the four give-backs of `NOW.md` §0sp2), and
    /// **a container with a single-consumer contract needs an owner named
    /// in code** — `CLAUDE.md`'s clean-merge trap, which cost this repo a
    /// silent audio outage when two lanes each added a reader to a ring
    /// that hands each fact over once. The producers write; this drains;
    /// nothing else calls `spill_at`.
    ///
    /// The fall-point is always the body, never the object given back.
    /// `build::demolish`'s doc carries the argument in full: every one of
    /// the six producers refuses beyond `BUILD_REACH_M`, and
    /// `backpack::LOOT_REACH_M` *is* `BUILD_REACH_M`, so the object's own
    /// address is inside the merge reach of the feet by construction and
    /// choosing between them cannot change what a player finds.
    ///
    /// `self.tick` and `World::tick`'s local `tick` are the same value —
    /// the local is bound after the command loop and nothing advances the
    /// field mid-tick — so this reads identically from `apply` and from the
    /// player loop. A bag's expiry would otherwise depend on which caller
    /// stood it up, which the state hash would see.
    fn drain_spill(&mut self, slot: usize, spill: &mut [ItemStack; INV_SLOTS]) {
        if spill.iter().all(|s| s.count == 0) {
            return;
        }
        let (owner, sx, sy, sz) = {
            let p = &self.players[slot];
            (
                p.id,
                p.body.qx,
                p.body.qy + crate::backpack::BAG_Y_OFFSET_Q,
                p.body.qz,
            )
        };
        let tick = self.tick;
        self.backpacks.spill_at(
            &self.backpack,
            &self.gather,
            sx,
            sy,
            sz,
            owner,
            spill,
            tick,
            &mut self.events,
        );
    }

    fn apply(&mut self, cmd: &Command, removals: &mut usize, favour: &mut [u8; MAX_PLAYERS]) {
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
            Command::AdminTeleport { id, to } => {
                // Both bodies resolved before either is touched, and a
                // miss on either is a no-op: `Wake`'s rule, because a WAL
                // replayed against a diverged world must refuse rather
                // than move somebody somewhere nobody is standing.
                if let (Some(from), Some(dest)) = (self.live_slot_of(id), self.live_slot_of(to)) {
                    if from != dest {
                        let body = self.players[dest].body;
                        self.players[from].body = body;
                    }
                }
            }
            Command::AdminGive { id, item, count } => {
                if let Some(slot) = self.live_slot_of(id) {
                    // An unknown row is refused rather than stored: the
                    // stack cap is read off the item table, and an index
                    // past it would be a stack with no rule.
                    if (item as usize) < self.gather.stack_max.len() {
                        let cap = self.gather.stack_max[item as usize];
                        if cap > 0 {
                            let cond = self.gather.cond_max[item as usize];
                            crate::gather::inv_add(
                                &mut self.players[slot].inv,
                                item,
                                count,
                                cap,
                                cond,
                            );
                        }
                    }
                }
            }
            Command::Input {
                id,
                frame,
                favour: want,
            } => {
                if let Some(slot) = self.slot_of(id) {
                    // Clamped, not refused. `pose_at` is already total over
                    // `back` — an out-of-range depth falls back to the live
                    // body — so this is not a safety check but a statement
                    // of the ceiling in one place, next to the ring it
                    // indexes. `REWIND_MAX_TICKS` is 250 ms floored to
                    // whole ticks (`limits.rs`), and the clamp direction is
                    // the conservative one: a forged favour buys the
                    // shooter *less* help, never more.
                    //
                    // Written on `slot_of`, not `live_slot_of`, for the
                    // same reason the frame below is: the arm is one
                    // condition, and a sleeper's verbs do not run anyway.
                    favour[slot] = want.min(crate::rewind::Rewind::max_back());
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
            Command::Unlock { id, recipe } => {
                if let Some(s) = self.live_slot_of(id) {
                    crate::research::unlock(
                        &self.research,
                        &self.craft,
                        &self.deploy,
                        &self.deploys,
                        &mut self.players[s],
                        recipe,
                        &mut self.events,
                    );
                }
            }
            Command::CraftCancel { id, index } => {
                if let Some(slot) = self.live_slot_of(id) {
                    let mut spill = [ItemStack::default(); INV_SLOTS];
                    craft::cancel(
                        &self.craft,
                        &self.gather,
                        self.tick,
                        &mut self.players[slot],
                        index,
                        &mut spill,
                    );
                    self.drain_spill(slot, &mut spill);
                }
            }
            Command::Place {
                id,
                row,
                cx,
                cz,
                level,
                loc,
                freehand,
            } => {
                if let Some(slot) = self.live_slot_of(id) {
                    build::place(
                        self.seed,
                        &self.haven,
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
                        freehand,
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
                        &self.haven,
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
                        let owner = deploy::use_door(
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
                        // The trust row rides here rather than inside the
                        // verb, and that is the whole reason the verb
                        // returns an owner instead of pushing the event:
                        // `use_door` holds `&mut players[slot]`, so it
                        // cannot also read the roster the presence
                        // question is asked of. One borrow later, this can.
                        if let Some(owner) = owner {
                            self.log_trust(id, owner, TRUST_DOOR);
                        }
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
                    let mut spill = [ItemStack::default(); INV_SLOTS];
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
                            &mut spill,
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
                            &mut spill,
                        );
                    }
                    // One buffer for both arms: they are two verbs behind
                    // one command and exactly one of them ran.
                    self.drain_spill(slot, &mut spill);
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
                    // One `TRUST_AUTH` for both stores, because `EV_AUTH`
                    // is already one event for both — the crew and the
                    // lock's list are the same `Roster` answering the same
                    // question.
                    if deploy::op_is_crew(op) {
                        let owner = deploy::crew_op(
                            &mut self.deploys,
                            &self.players[slot],
                            cx,
                            cz,
                            level,
                            op,
                            &mut self.events,
                        );
                        if let Some(owner) = owner {
                            self.log_trust(id, owner, TRUST_AUTH);
                        }
                    } else {
                        let mut spill = [ItemStack::default(); INV_SLOTS];
                        let owner = deploy::lock_op(
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
                            &mut spill,
                        );
                        self.drain_spill(slot, &mut spill);
                        if let Some(owner) = owner {
                            self.log_trust(id, owner, TRUST_AUTH);
                        }
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
            Command::Pickup { id } => {
                if let Some(slot) = self.live_slot_of(id) {
                    crate::spent::pickup(
                        &mut self.spent,
                        &self.gather,
                        self.tick,
                        &mut self.players[slot],
                        &mut self.events,
                    );
                }
            }
            Command::OpenWorldCont { id, cont } => {
                if let Some(slot) = self.live_slot_of(id) {
                    self.world_conts.open(
                        self.seed,
                        &self.scatter,
                        &self.haven,
                        &self.loot,
                        &self.gather,
                        self.tick,
                        (cont >> 16) as u16,
                        (cont & 0xFFFF) as u16,
                        &self.players[slot],
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
            Command::Reload { id } => {
                // `live_slot_of`: a corpse and a sleeper do not reload,
                // for the reason `hitscan` restates the same rule — the
                // arm belongs to a body somebody is driving.
                if let Some(slot) = self.live_slot_of(id) {
                    ranged::reload(
                        self.tick,
                        &self.combat,
                        &mut self.events,
                        &mut self.players[slot],
                    );
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
        // How far back each slot's verbs may look this tick, in ticks.
        //
        // Minted here beside `removals` and for the same reason: it is
        // spent and forgotten inside one tick, so it is a tick-local and
        // never a `World` field. Storing it would put a latency-derived
        // number into `state_hash` and `persist.rs`, and a replay of the
        // same command stream would then have to reproduce a network
        // condition — which is the determinism violation wall 5 forbids.
        // Here the favour arrives *in the command*, so the WAL already
        // carries everything a replay needs.
        //
        // Zero is the floor and the default, in three places at once: a
        // slot nobody sent an input for this tick, a slot whose input
        // arrived with `favour: 0`, and every non-server construction of
        // `Command::Input`. Zero means `pose_at` returns the live body,
        // which is the behaviour that predates lag compensation.
        let mut favour = [0u8; MAX_PLAYERS];
        for cmd in commands.iter().take(MAX_COMMANDS_PER_TICK) {
            self.apply(cmd, &mut removals, &mut favour);
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
        // Iterated over the favour array rather than `0..MAX_PLAYERS`, which
        // is the same slot order — the array is exactly `MAX_PLAYERS` wide
        // and is never written inside this loop, only by `apply` above.
        for (i, &granted) in favour.iter().enumerate() {
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
                    &self.haven,
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
                    &self.haven,
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
            // The torch burns on the same footing as the metabolism, and
            // deliberately beside it: both are clocks that spend something
            // the player is carrying, and both run before the arm so a
            // flame that dies this tick is dead for this tick's swing too.
            //
            // Only the live path. `light::is_lit` refuses a sleeper and a
            // corpse on its own, so this placement is not what makes that
            // true — but a body nobody is driving holds a stale frame, and
            // the cheapest way to never spend an absent player's inventory
            // is to not run the sweep over them.
            crate::light::step(&mut self.players[i], &self.gather);
            let frame = self.players[i].frame;
            movement::step(
                seed,
                &self.haven,
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
            // What a full pack could not hold this tick. Written by
            // `gather::swing` and `craft::step`, drained once below into a
            // bag at this body's feet — one fixed drain point, because two
            // producers each standing their own bag up is the shape
            // CLAUDE.md's single-consumer trap is about.
            let mut spill = [ItemStack::default(); INV_SLOTS];
            let swung = if ranged::draw(
                tick,
                &self.combat,
                &mut self.arrows,
                &mut self.events,
                &mut self.players[i],
            ) {
                gather::Swing::Absorbed
            } else {
                gather::swing(
                    seed,
                    tick,
                    &self.gather,
                    &self.loot,
                    &self.scatter,
                    &self.haven,
                    &mut self.slot_cache,
                    &mut self.slot_lives,
                    &mut self.events,
                    &mut self.players[i],
                    &mut spill,
                )
            };
            craft::step(
                &self.craft,
                &self.gather,
                tick,
                &mut self.players[i],
                &mut self.events,
                &mut spill,
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
            // Drain the tick's spill. After the barrel arm on purpose: a
            // smashed barrel's bag stands at arm's length, so a spill in
            // the same tick merges into it instead of minting a second
            // container a step away.
            self.drain_spill(i, &mut spill);
            if swung == Swing::Free || swung == Swing::Refused {
                // node → player → structure: the arm passes on only what
                // nothing nearer absorbed. A `Refused` swing carries this
                // far too — a node must not become cover — and stops at
                // the animal: it was aimed at a gather node, so the wall
                // behind that node is not a target (`Swing::Refused`).
                match combat::strike(
                    &self.combat,
                    i,
                    &mut self.players,
                    &mut self.events,
                    &self.rewind,
                    tick,
                    granted,
                ) {
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
                        if !took && swung == Swing::Free {
                            combat::raid(
                                &self.haven,
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
        let mut blast_kills = crate::charge::BlastKills::new();
        crate::charge::tick_fuses(
            seed,
            &self.haven,
            &self.build,
            &self.deploy,
            &self.combat,
            &mut self.charges,
            &mut self.pieces,
            &mut self.deploys,
            &mut self.players,
            tick,
            &mut removals,
            &mut blast_kills,
            &mut self.events,
        );
        // The blast's dead, laid down after every fuse resolved — the
        // bite buffer's split, for its reason: `die` needs the whole
        // world. The hp is already zero and the events already rang
        // inside `detonate`; this is the corpse's half.
        for &(victim, owner, range_cm) in blast_kills.entries() {
            let slot = victim as usize;
            if self.players[slot].active && self.players[slot].hp == 0 && !self.players[slot].dead {
                self.die(slot, owner, DEATH_BY_CHARGE, NO_ITEM, range_cm);
            }
        }

        // The roster steps after the player loop and before the arrows, and
        // both sides of that are deliberate. After the players, because an
        // animal reads player positions to decide whether it is awake and
        // which way to run, and reading them mid-loop would make the answer
        // depend on the reader's slot index. Before the arrows, because a
        // shot must resolve against where the animal ended this tick — the
        // same rule the player loop's ordering states in the comment above.
        let mut bites = mob::Bites::new();
        mob::step(
            seed,
            &self.haven,
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
            &mut bites,
        );
        // The bites land after the whole roster stepped, so every animal
        // decided against one consistent tick — the borrow split `Bites`'
        // own doc names. The hp and the deaths counter go through
        // `combat::hurt`, the one debit (this loop used to hand-copy
        // "`combat::strike`'s exact damage liturgy" and said so); what
        // stays here is the half the funnel deliberately does not own —
        // EV_HURT and EV_HEALTH to the victim, EV_DEATH broadcast, and
        // `die` laying the body down with the cause the wire widened for.
        // Still no EV_HIT: a hitmarker is an attacker's fact and a pig has
        // no screen to draw one on. EV_HURT is the other side of exactly
        // that asymmetry — the victim has a screen, and until 2026-08-30
        // the only thing on it for a bear in the dark was a number going
        // down (`NOW.md` §0hrt item 5).
        for b in bites.entries() {
            let victim = b.victim as usize;
            // The animal's body, read before the victim is borrowed
            // mutably — `combat::strike` takes the attacker's position in
            // the same order and for the same borrow. Post-`mob::step`, so
            // it is where the animal was standing when it bit rather than
            // where it started the tick.
            let (mqx, mqz) = {
                let body = &self.mobs.m[b.mob_slot as usize].body;
                (body.qx as i64, body.qz as i64)
            };
            let v = &mut self.players[victim];
            if !v.active || v.hp == 0 {
                continue; // died to something else since the roster looked
            }
            let sector =
                crate::combat::bearing_sector(mqx - v.body.qx as i64, mqz - v.body.qz as i64);
            // The funnel, reduced: a bite is a hit.
            let crate::combat::Hurt { left, died, .. } =
                crate::combat::hurt(&self.combat, v, b.damage);
            let victim_id = v.id;
            self.events
                .push(EV_HURT, victim_id, sector as u32, b.damage as u32);
            self.events.push(
                EV_HEALTH,
                victim_id,
                left as u32,
                self.combat.player_hp as u32,
            );
            if died {
                let by = mob::mob_id(b.mob_slot as usize);
                self.events.push(EV_DEATH, victim_id, by, 0);
                self.die(victim, by, DEATH_BY_MOB, NO_ITEM, b.range_cm);
            }
        }

        // Arrows fly after the player loop, never inside it. Two reasons,
        // both structural: every body has already taken its step, so a shot
        // resolves against final positions instead of positions that are
        // final for the low slots and stale for the high ones; and nothing
        // about a hit may depend on the shooter's slot index, which it
        // would if flight ran while the loop still held one player
        // mutably. **`removals` IS spent here now** (ranged structure
        // damage v0): an arrow chips the wall it stops on, so a shot can
        // drop a piece and pays out of the same tick allowance a swing
        // does — this line said the opposite until 2026-08-28.
        let mut kills = [ranged::Kill::default(); MAX_ARROWS];
        // Reused between the two passes exactly as `kills` is, and drained
        // by each before the other fills it — one entry per arrow that
        // stopped on a piece, and `hitscan` writes at most one per player
        // under the same `MAX_PLAYERS <= MAX_ARROWS` const assert.
        let mut chips = [ranged::Chip::default(); MAX_ARROWS];
        // A firearm resolves here rather than in the loop above, for the
        // arrow's two reasons — final positions, and no dependence on the
        // shooter's slot index — and it goes **first** because it is the
        // only shot on this tick that was fired on it. An arrow in the
        // store was launched on an earlier one and has a tick of flight to
        // spend before it can reach anybody, so resolving the instant shot
        // ahead of it is the chronology, not a preference. `kills` is
        // reused rather than doubled: this pass writes at most one entry
        // per player, the array is drained before `step` fills it again,
        // and `ranged.rs`'s const assert holds `MAX_PLAYERS <= MAX_ARROWS`.
        let (n_shot, n_chips) = ranged::hitscan(
            seed,
            &self.haven,
            self.pieces.cols(),
            &mut crate::occupy::Occupants {
                table: &self.scatter,
                haven: &self.haven,
                harvested: &self.slot_lives,
                cache: &mut self.slot_cache,
            },
            tick,
            // Lag compensation, the gun's half (`ranged::hitscan`). Melee
            // rewound one pass earlier and this did not, which left the
            // firearm as the only weapon on the shard decided by ping.
            // `favour` is the same tick-local the melee loop above spends,
            // read here by the shooter's slot — it is still never a
            // `World` field and still never in `state_hash`.
            &self.rewind,
            &favour,
            &self.combat,
            &mut self.players,
            &mut self.events,
            &mut kills,
            &mut chips,
        );
        // Chips before deaths, and the order is the tick's chronology
        // rather than a preference: a bullet reaches the wall it stops on
        // during this pass, and `die` lays a body down, drops its bag and
        // can itself take a deployable with it. Draining the shot's own
        // consequence first keeps the wall's `EV_STRUCT_HIT` adjacent to
        // the `EV_IMPACT` that explains it.
        for c in chips.iter().take(n_chips) {
            self.chip(c, &mut removals);
        }
        for k in kills.iter().take(n_shot) {
            // Was `DEATH_BY_ARROW` from hitscan v0 to arrow recovery v1,
            // under the refusal that constant's doc states and this bump
            // lifts. A rifle no longer reports an arrow.
            self.die(k.victim, k.by, DEATH_BY_BULLET, k.item, k.range_cm);
        }
        let (n_kills, n_chips) = ranged::step(
            seed,
            self.tick,
            &self.haven,
            self.pieces.cols(),
            &mut crate::occupy::Occupants {
                table: &self.scatter,
                haven: &self.haven,
                harvested: &self.slot_lives,
                cache: &mut self.slot_cache,
            },
            &self.combat,
            &mut self.arrows,
            &mut self.spent,
            &mut self.players,
            &mut self.events,
            &mut kills,
            &mut chips,
        );
        for c in chips.iter().take(n_chips) {
            self.chip(c, &mut removals);
        }
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
            let (x, y, z) = deploy::box_drop_pos(
                seed,
                &self.haven,
                self.pieces.cols(),
                bx.cx,
                bx.cz,
                bx.level,
            );
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
        // Every body's pose, recorded for tick `tick` — the last thing the
        // tick does, and deliberately *after* the phase note above says
        // positions are final. Three of the four `movement::step` sites are
        // in the player loop and a death, a respawn or a blast can still
        // replace a body after it, so a snapshot taken inside that loop
        // would record a pose the tick then overwrote.
        //
        // Here `self.tick` is still `T`, so row `T & (REWIND_TICKS - 1)`
        // holds end-of-tick poses for `T` and during tick `T + 1` the ring
        // answers for `T` back to `T - REWIND_TICKS + 1`. Derived output:
        // it is not hashed and not saved (`rewind.rs`).
        self.rewind.write_row(self.tick, &self.players);
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
            // The magazine, in its own update rather than widening the
            // buffer above — and hashed at all because it decides whether
            // a gun fires. Sim state outside the hash is the shape this
            // repo pays for twice over: two shards could disagree about a
            // loaded cylinder with `test_replay` green, and a save that
            // dropped it would restore a world that hashes the same and
            // plays differently. Both halves: `mag_round` is what the next
            // shot spends, so a magazine holding the wrong round is a
            // divergence even at an identical count.
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
            let mut hb = [0u8; 12];
            hb[0..4].copy_from_slice(&p.heal_span.to_le_bytes());
            hb[4..8].copy_from_slice(&p.heal_acc.to_le_bytes());
            // The torch's remainder, for the accumulators' reason exactly
            // (torch fuel v0): a replay resuming mid-point with a zeroed
            // remainder burns the next point six seconds late and every
            // one after it. There is no `lit` byte to hash beside it
            // because a flame is derived, not stored — `light.rs`.
            hb[8..12].copy_from_slice(&p.light_acc.to_le_bytes());
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
                let mut sb = [0u8; 6];
                sb[0..2].copy_from_slice(&s.item.to_le_bytes());
                sb[2..4].copy_from_slice(&s.count.to_le_bytes());
                sb[4..6].copy_from_slice(&s.cond.to_le_bytes());
                h.update(&sb);
            }
            // What this body is wearing (armor v0), in its own loop
            // appended after the inventory rather than folded into it.
            // Two reasons and both are load-bearing. It is sim state — a
            // worn piece changes what every hit takes off, so two shards
            // that disagreed about it would disagree about a fight and
            // then about a death — and it is a **separate array**, so
            // widening the inventory's loop to cover it would make one
            // digest out of two stores and hide the next widening of
            // either.
            //
            // This is what moved `GOLDEN_FINAL_HASH` on 2026-08-19, and
            // it moved it deliberately: `worn` is per-player and always
            // present, so unlike `world.rs`'s store loops there is no
            // length prefix to fold zeroes into, and the digest changes
            // the moment the field exists whether or not anything is in
            // it. The hash is behavioural now — it says the sim carries
            // worn equipment — where before it said nothing at all.
            for s in p.worn.iter() {
                let mut wb = [0u8; 6];
                wb[0..2].copy_from_slice(&s.item.to_le_bytes());
                wb[2..4].copy_from_slice(&s.count.to_le_bytes());
                wb[4..6].copy_from_slice(&s.cond.to_le_bytes());
                h.update(&wb);
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
            let mut buf = [0u8; 13];
            buf[0..2].copy_from_slice(&r.cx.to_le_bytes());
            buf[2..4].copy_from_slice(&r.cz.to_le_bytes());
            buf[4] = r.level;
            buf[5] = r.loc;
            buf[6] = r.row;
            buf[7..9].copy_from_slice(&r.hp.to_le_bytes());
            buf[9..11].copy_from_slice(&r.uh.to_le_bytes());
            // The soft-side facing (hard/soft v0) in the buffer's one
            // spare byte: it prices a swing, so two shards disagreeing
            // about it disagree about a raid.
            buf[11] = r.facing;
            // The column's plate (build plate v1), which widened this
            // buffer from 12 — the first piece field that is a CHOICE
            // rather than a function of (seed, cell). Two shards that
            // disagree about it disagree about where every surface in the
            // column is, which is further than a raid's price: it is
            // whether a body is standing or falling.
            buf[12] = r.plate as u8;
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
            let mut buf = [0u8; 40];
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
            buf[34..36].copy_from_slice(&a.structure.to_le_bytes());
            buf[36..38].copy_from_slice(&a.life.to_le_bytes());
            buf[38..40].copy_from_slice(&a.round.to_le_bytes());
            h.update(&buf);
            h.update(&a.flown.to_le_bytes());
        }
        // Arrows that have landed (`spent.rs`). Length-prefixed, unlike the
        // block above — this store is dense with an explicit `len` where
        // `Arrows` is a slotted array with holes, so the count is state
        // here and an artefact there.
        //
        // **The whole block is skipped while nothing has ever landed**,
        // which is the arrow idiom's payoff kept rather than its shape:
        // a world where no arrow has hit anything folds not one byte, so
        // `GOLDEN_FINAL_HASH` stays evidence about the script it pins.
        // `evictions` is part of the condition and not only of the body,
        // because a store that filled and was then emptied by pickups has
        // a zero length and a history — and that history is the only
        // evidence an eviction leaves (`MAX_SPENT_ARROWS`, and
        // `World::evictions` one field over makes the same argument about
        // sleeping bodies).
        if !self.spent.is_empty() || self.spent.evictions() > 0 {
            h.update(&(self.spent.len() as u64).to_le_bytes());
            for e in self.spent.entries() {
                let mut buf = [0u8; 14];
                buf[0..4].copy_from_slice(&e.qx.to_le_bytes());
                buf[4..8].copy_from_slice(&e.qy.to_le_bytes());
                buf[8..12].copy_from_slice(&e.qz.to_le_bytes());
                buf[12..14].copy_from_slice(&e.round.to_le_bytes());
                h.update(&buf);
                h.update(&e.ready_at.to_le_bytes());
            }
            h.update(&self.spent.evictions().to_le_bytes());
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
        // `respawn_at` and `roused_until`, which are deadlines rather than
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
            buf[24..32].copy_from_slice(&m.roused_until.to_le_bytes());
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
            let mut buf = [0u8; 25];
            buf[0..2].copy_from_slice(&c.cx.to_le_bytes());
            buf[2..4].copy_from_slice(&c.cz.to_le_bytes());
            buf[4] = c.level;
            buf[5] = c.loc;
            buf[6] = c.deploy as u8;
            buf[7..9].copy_from_slice(&c.structure.to_le_bytes());
            // The blast's other two copied-at-plant numbers (satchel
            // blast v0): two shards disagreeing about a live charge's
            // radius disagree about which walls are standing next tick.
            buf[9..11].copy_from_slice(&c.damage.to_le_bytes());
            buf[11..13].copy_from_slice(&c.blast_cm.to_le_bytes());
            buf[13..21].copy_from_slice(&c.fires_at.to_le_bytes());
            // The planter rides the digest too: it is on the wire off this
            // record, so a replay that reproduced the blast while
            // inventing the raider would put a different name on it.
            buf[21..25].copy_from_slice(&c.owner.to_le_bytes());
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
                let mut sb = [0u8; 6];
                sb[0..2].copy_from_slice(&s.item.to_le_bytes());
                sb[2..4].copy_from_slice(&s.count.to_le_bytes());
                sb[4..6].copy_from_slice(&s.cond.to_le_bytes());
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
                let mut sb = [0u8; 6];
                sb[0..2].copy_from_slice(&s.item.to_le_bytes());
                sb[2..4].copy_from_slice(&s.count.to_le_bytes());
                sb[4..6].copy_from_slice(&s.cond.to_le_bytes());
                h.update(&sb);
            }
        }
        // The id counter is state, not a cursor: a replay that reused an
        // id the first run retired would name two different bags the same
        // thing, and every downstream client keyed on it would agree with
        // neither.
        h.update(&self.backpacks.next_id().to_le_bytes());
        // World containers (`worldcont.rs`). Every field is hashed
        // including `refill_at`: two shards whose crates were emptied on
        // different ticks agree about the contents (both empty) and
        // disagree about *when they pay again*, which is a divergence that
        // would otherwise stay silent until the first refill.
        h.update(&(self.world_conts.len() as u64).to_le_bytes());
        for c in self.world_conts.entries() {
            let mut buf = [0u8; 21];
            buf[0..2].copy_from_slice(&c.cx.to_le_bytes());
            buf[2..4].copy_from_slice(&c.cz.to_le_bytes());
            buf[4..8].copy_from_slice(&c.qx.to_le_bytes());
            buf[8..12].copy_from_slice(&c.qz.to_le_bytes());
            buf[12] = c.table;
            buf[13..21].copy_from_slice(&c.refill_at.to_le_bytes());
            h.update(&buf);
            for s in c.items.iter() {
                let mut sb = [0u8; 6];
                sb[0..2].copy_from_slice(&s.item.to_le_bytes());
                sb[2..4].copy_from_slice(&s.count.to_le_bytes());
                sb[4..6].copy_from_slice(&s.cond.to_le_bytes());
                h.update(&sb);
            }
        }
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

    /// The point and seed `ci/browser_smoke.mjs` put both tabs on. That gate
    /// is deleted; this native guard is what it was written to back up, so a
    /// worldgen change that sinks or steepens the spawn fails here.
    const SMOKE_SEED: u64 = 20260731;
    const SMOKE_SPAWN: (f32, f32) = (1024.0, 1024.0);

    /// **Wear is in `state_hash`, in all four container stores** (item
    /// durability v0, gate 4): two worlds differing ONLY in one stack's
    /// condition must hash differently — for the player inventory, a
    /// ground bag, a deployed box and a world container, each store
    /// proven alone. Proven red by omitting the `cond` bytes from any of
    /// the four `[0u8; 6]` loops above: the store whose loop was narrowed
    /// hashes the two worlds identical and its assert fires.
    #[test]
    fn condition_is_hashed_in_every_container_store() {
        use crate::backpack::BackpackRec;
        use crate::deploy::BoxRec;
        use crate::worldcont::WorldContRec;

        let base = || {
            let mut w = Box::new(World::new(SMOKE_SEED));
            w.tick(&[Command::Join { id: 7 }]);
            w
        };
        let stack = |cond: u16| ItemStack {
            item: 3,
            count: 1,
            cond,
        };

        // 1. The player inventory loop.
        let mut a = base();
        let mut b = base();
        a.players[0].inv[4] = stack(100);
        b.players[0].inv[4] = stack(101);
        assert_ne!(
            a.state_hash(),
            b.state_hash(),
            "two worlds differing only in a held stack's condition hashed \
             the same — the inventory loop dropped the cond bytes"
        );

        // 2. The ground-bag loop.
        let bag = |cond: u16| {
            let mut r = BackpackRec {
                id: 1,
                qx: 100,
                qy: 50,
                qz: 100,
                owner: 7,
                expires: 999,
                items: [ItemStack::default(); INV_SLOTS],
            };
            r.items[2] = stack(cond);
            r
        };
        let mut a = base();
        let mut b = base();
        a.backpacks.restore(&[bag(100)], 2);
        b.backpacks.restore(&[bag(101)], 2);
        assert_ne!(
            a.state_hash(),
            b.state_hash(),
            "two worlds differing only in a bagged stack's condition hashed \
             the same — the backpack loop dropped the cond bytes"
        );

        // 3. The deployed-box loop. The deploy store restores empty except
        // for one box record; the record slices are all index-aligned.
        let boxed = |cond: u16| {
            let mut r = BoxRec {
                cx: 10,
                cz: 10,
                level: 0,
                owner: 7,
                items: [ItemStack::default(); crate::limits::BOX_SLOTS],
            };
            r.items[1] = stack(cond);
            r
        };
        let mut a = base();
        let mut b = base();
        a.deploys.restore(
            &[],
            &[],
            &[],
            &[],
            &[boxed(100)],
            &[crate::oven::OvenState::default()],
            &[],
        );
        b.deploys.restore(
            &[],
            &[],
            &[],
            &[],
            &[boxed(101)],
            &[crate::oven::OvenState::default()],
            &[],
        );
        assert_ne!(
            a.state_hash(),
            b.state_hash(),
            "two worlds differing only in a boxed stack's condition hashed \
             the same — the box loop dropped the cond bytes"
        );

        // 4. The world-container loop.
        let cont = |cond: u16| {
            let mut r = WorldContRec {
                cx: 20,
                cz: 20,
                qx: 5_000,
                qz: 5_000,
                table: crate::loot::LOOT_CRATE as u8,
                refill_at: 0,
                items: [ItemStack::default(); INV_SLOTS],
            };
            r.items[0] = stack(cond);
            r
        };
        let mut a = base();
        let mut b = base();
        a.world_conts.restore(&[cont(100)]);
        b.world_conts.restore(&[cont(101)]);
        assert_ne!(
            a.state_hash(),
            b.state_hash(),
            "two worlds differing only in a crated stack's condition hashed \
             the same — the world-container loop dropped the cond bytes"
        );
    }

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
