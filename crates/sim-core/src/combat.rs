//! Combat — the verb that takes something away (DESIGN.md §2, M1). Until
//! this module, every hp in the world decayed and nothing attacked it: a
//! locked door was furniture and `content/weapons.toml` was data nothing
//! played. Melee v0 is the smallest honest fix — **the swing that already
//! fells a tree also lands on a person.**
//!
//! One verb, one gate: `gather::swing` owns the cadence and the target
//! pick for scatter; a swing that finds no node is handed here, and looks
//! for a player instead. Same reach shape, same aim cone, same tick — a
//! hatchet swung at a neighbour is not a second mechanic.
//!
//! Content reaches the sim only as a baked `CombatContent` table (CLAUDE.md
//! wall 7): per-item melee damage and reach from `content/weapons.toml`,
//! max hp from `content/balance.toml`'s `globals.player_hp` — the same
//! number `test_content`'s TTK anchor divides by, so the band the data
//! declares and the band the sim plays are one number, not two. The inert
//! `EMPTY` default makes combat a no-op (nothing has damage, nobody has
//! hp), and `probe_fixture()` is a synthetic table for the parity/replay/
//! alloc gates.
//!
//! Pure and fixed-capacity like the rest of the crate: the target scan is
//! one pass over the `MAX_PLAYERS` slot array, taken only on a swing tick
//! that missed every node, and it allocates nothing.
//!
//! **Piece damage rides the same arm** (`raid`). A swing that found no
//! node and no player looks last for a wall, a doorway, or a deployable —
//! so the target order is node → player → structure, each the nearest
//! candidate inside the same reach and the same aim cone. A base is no
//! longer a vault: `content/weapons.toml`'s `structure` column says what
//! one hit takes off it, and `content/balance.toml`'s breach bands hold
//! the door as the intended breach point while every wall stays the
//! satchel's job.
//!
//! **What v0 deliberately does not do**, all of it registered in
//! `DECISIONS.md` §open ("melee combat v0" and "piece damage v0"): no
//! headshots **for a swing** — ⚠ **retired 2026-09-05 and kept here because
//! the clause moved twice**: it was true of the whole crate, then only of the
//! swing when headshot v0 gave `ranged` a head, and melee aim v1 made the
//! swing a ray, so `MeleeDef` carries `headshot_mult` and `limb_pct` and a
//! spear pays the same rungs a bullet does — no per-weapon cadence (every swing rides gather's one
//! interval, which is the melee rows' own rate), and no corpse: death drops what you carried into a
//! backpack where you fell. That last clause is about the SIM and stays
//! true — there is no lootable body entity, only a bag — while the client
//! has drawn a fallen one since wire v48 (`render/anim.rs` `Clip::Death`).
//! The two do not disagree: the drawn body is not a container and not a
//! target, which is exactly why it was safe to lay it down.
//!
//! ⚠ **"No ranged of any kind" stood here after it stopped being true**,
//! which is `CLAUDE.md`'s dead-citation trap in the present tense: bows
//! are baked (`content/bake.rs` — `WeaponKind::Bow` takes `bake_ranged`
//! and the `[[ammo]]` rows go first) and fired (`ranged.rs`), so an arrow
//! had killed a player well before that line was re-read on 2026-08-18.
//! **The live half of it died a day later.** It said `bake_combat` still
//! dropped the firearm rows, and that was true until 2026-08-19: the
//! revolver now bakes through the same `bake_ranged` a bow does and fires
//! through `ranged::hitscan`, which is `ranged::step` with the flight
//! deleted. Nothing in `weapons.toml` is priced and unarmed any more.
//!
//! Three clauses that used to stand here have since landed and are named
//! rather than deleted, because each is a place this module's shape was
//! decided by something outside it: **repair** (`build::repair`, wire
//! v21), **structural collapse** (`build::collapse_from` — a foundation
//! broken under a wall now takes the wall with it), and **throwables**
//! (`charge.rs`, wire v23 — the satchel had a price and no verb for
//! fifteen wire versions, which is the gap that made the raid ratio in
//! `balance.toml` a ratio of nothing).
//!
//! ⚠ **"No armor reduction" stood in that list until 2026-08-19 and does
//! not any more.** `content/armor.toml` had been priced, validated, hashed
//! and balance-anchored since M1 with nothing in this crate reading a row
//! (`reference/ARMOR.md` §9.1 was the audit); `bake_combat` installs the
//! table now and `hurt` reads it, so a burlap shirt turns a rock's five
//! hits into six. What is still *not* here is named rather than implied:
//! no damage types (one scalar — `ArmorDef`'s doc has the argument and its
//! source), no hit areas (the set is one number, `worn_pct`), no condition
//! on a worn piece, and no `move_penalty_pct` consumer. The item this list
//! led with — *"the one a player notices: no way to put armor on"* — is
//! **closed** (armor v1, wire v51): `CONT_WEAR` is the fifth container
//! kind and equipping is a move into it, checked here by `wearable_in`.
//!
//! The throwable's damage does not flow through this module's swing arm at
//! all, and that is the design rather than an omission: a charge resolves
//! on the tick its fuse runs out, not on the tick a button was pressed, so
//! it is `World::tick`'s phase list that owns it. What it shares with a
//! swing is the *ending* — both spend the same `MAX_REMOVALS_PER_TICK`
//! allowance and both land through `deploy::damage_piece`.

use crate::collide::Part;
use crate::gather::NO_ITEM;
use crate::limits::{MAX_ITEM_DEFS, MAX_PLAYERS, MAX_WEAPON_AMMO, WEAR_SLOTS};
use crate::movement::POS_Y_Q;
use crate::world::{EventQueue, Player, EV_DEATH, EV_HEALTH, EV_HIT, EV_HURT};

/// One item's melee row. `damage == 0` ⇒ the item is not a weapon (the
/// whole table starts that way), so a bare hand and a stack of wood are
/// the same swing: nothing. Reach is centimetres so the baked table
/// carries no float rounding of `content/weapons.toml`'s `range_m`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MeleeDef {
    pub damage: u16,
    /// Damage this item takes off a building piece or a deployable — the
    /// `structure` column of `content/weapons.toml`, its own number.
    pub structure: u16,
    pub reach_cm: u16,
    /// What a head is worth, as a multiplier on `damage` — the
    /// `headshot_mult` column, which every melee row has carried, priced
    /// and content-hashed, since the content crate existed, and which the
    /// bake dropped one line before this struct could hold it. Read since
    /// melee aim v1 (2026-09-05): a swing has a line to cross now
    /// (`melee::cast`), so it pays the same ladder a shot pays
    /// (`part_damage`). 1 is the identity.
    pub headshot_mult: u16,
    /// What a leg is worth, in **percent** of `damage` — the `limb_pct`
    /// column, `headshot_mult`'s other end. 100 is the identity.
    pub limb_pct: u16,
}

/// One item's throwable row — the raid tool (`charge.rs`). Separate from
/// `MeleeDef` rather than a nullable column on it because the two are
/// read by different verbs at different times: a melee row answers a
/// swing this tick, a throwable row plants something that resolves
/// `fuse_ticks` later. `structure == 0` ⇒ not a throwable, the same inert
/// sentinel `MeleeDef::damage` uses.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ThrowDef {
    /// Damage the blast is intended to take off a *player* standing in it
    /// at the epicentre. The same `damage` column the melee rows read,
    /// which the bake discarded for throwables until the blast had anyone
    /// to hurt. Zero is a legal row and means a charge that breaks walls
    /// and not people — the counted fixtures below rely on it.
    ///
    /// **Applied since the blast grew a falloff**: `charge.rs:521` gates on
    /// it (a zero row hurts nobody and skips the body scan outright) and
    /// `:532` scales it by distance. This line said *"carried, not applied:
    /// no sim code reads it"* through two judges who each checked.
    pub damage: u16,
    /// Damage the blast takes off the piece or deployable it was planted
    /// on — the same `structure` column the melee rows read, and the
    /// number `balance.toml`'s raid ratio divides wall hp by. It arrives
    /// whole at the planted address and **falls off linearly to zero at
    /// `blast_cm`** for everything else in the volume (`charge::falloff`,
    /// `:347`). This line said the planted address was the only one it
    /// reached, which stopped being true when the falloff landed.
    pub structure: u16,
    /// Ticks between planting and the blast, baked from `fuse_s` against
    /// `TICK_HZ` so the sim never divides a content number itself. `u16`
    /// because the wire carries it (`EV_CHARGE_PLACED`'s `c`) at that
    /// width — the bake refuses a longer fuse rather than letting the
    /// encoder be the first thing that notices.
    pub fuse_ticks: u16,
    /// How far from the anchor a charge may be planted — `range_m × 100`,
    /// `MeleeDef::reach_cm`'s treatment of the same column.
    pub reach_cm: u16,
    /// Blast radius, `blast_m × 100`.
    ///
    /// **It is the falloff's divisor**, which is what it was landed ahead of:
    /// `charge::falloff` (`:347-351`) scales both damage columns linearly to
    /// zero at this radius, `:436` reads it off the live charge, and
    /// `:277-279` copies all three off the row at plant time so a table swap
    /// cannot change what an armed charge does. `validate` and the bake still
    /// refuse a zero, which is why the sim does not guard one every tick.
    ///
    /// This line said *"nothing reads this"* through two judges. The
    /// mechanism worth remembering is the one `CLAUDE.md` names: a doc that
    /// claims a field is inert is read as *this is safe to change*, and
    /// nothing in CI compares a doc comment to a call site.
    pub blast_cm: u16,
}

/// One item's ranged row. `damage == 0` ⇒ the item fires nothing, which is
/// how the whole table starts and how every melee weapon, tool and stack of
/// wood stays.
///
/// Every rate here is already **per tick**, converted once at bake time and
/// never at tick time. That is the quantize-both-sides law (CLAUDE.md's trap
/// list) applied to a projectile: the sim integrates in exactly the integers
/// it was handed, so an arrow's path has no rounding to accumulate and no
/// per-tick float to drift between native and wasm.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RangedDef {
    /// Body damage on a hit. No falloff — `content/weapons.toml` has no
    /// falloff curve to read (CONTENT.md §1 describes one; the schema has
    /// never had the field).
    pub damage: u16,
    /// Item indices of the rounds this weapon can spend, in **preference
    /// order**, `NO_ITEM`-padded. The sim walks the list and spends the
    /// first round the shooter is actually carrying.
    ///
    /// A list because the reference game's bow is a `BaseProjectile` that
    /// can `SwitchAmmoTo` (`reference/PROJECTILES.md` §1) — one weapon,
    /// several rounds. No switch verb exists here, so order is the whole
    /// of the policy.
    pub ammo: [u16; MAX_WEAPON_AMMO],
    /// Ticks between shots, from `rate_per_min`. A bow does not borrow the
    /// melee cadence: `SWING_INTERVAL_TICKS` is one shared number and a
    /// 30/min bow is not a 47/min club.
    pub rate_ticks: u16,
    /// **Whether the shot resolves on the tick the trigger is pulled.**
    /// `false` is a bow: the launch stands an `Arrow` up and the flight
    /// resolves it over the following ticks. `true` is a firearm: there is
    /// no projectile, and `ranged::hitscan` traces the whole reach in one
    /// pass after the player loop.
    ///
    /// It is a field rather than an inference, and the inference was
    /// available: a bow's rounds carry `[[ammo]]` ballistics and a
    /// firearm's do not (`content/weapons.toml`'s header states exactly
    /// that contract), so `ammo_def` returning `None` for every listed
    /// round would have said the same thing. The trouble is what it says
    /// when content is wrong — a bow whose arrow lost its `[[ammo]]` row
    /// would quietly become a hitscan rifle rather than fail to load.
    /// `content/validate.rs` refuses that pairing at boot and this field
    /// records the answer once, at bake, so the sim never re-derives it.
    pub hitscan: bool,
    /// The weapon's reach in **millimetres**, from `range_m`.
    ///
    /// Flight time used to be baked here as `life_ticks` and cannot be any
    /// more: with ballistics on the round (§9.3), one bow's fast arrow and
    /// its slow arrow cross the same range in different numbers of ticks.
    /// The sim divides this by the chosen round's speed at the moment of
    /// the shot — integer division, once per shot, never per tick.
    pub range_mm: u32,
    /// What one hit takes off a **building piece** — `weapons.toml`'s
    /// second damage column, the same one `MeleeDef::structure` carries and
    /// under the same law (`balance.rs`: never above the row's own
    /// `damage`, never at the raid tool's).
    ///
    /// **It was priced, validated and content-hashed for months while the
    /// bake threw it away.** `content/weapons.toml` has given the bow,
    /// the crossbow and the revolver `structure = 1` since the content
    /// crate; `bake_combat` sends a `bow`/`firearm` row to `bake_ranged`
    /// before the melee table's `structure` read, and `RangedDef` had
    /// nowhere to put it — so `canon.rs` hashed a number that changed
    /// nothing, which is the same "armed and unread" shape that left the
    /// whole bow unfired until hitscan v0 (this module's header).
    ///
    /// Zero is a weapon that cannot mark a wall at all, and the sim reads
    /// it as exactly that rather than as "unset": `ranged.rs` skips the
    /// damage write, and the shot still stops and still draws its impact.
    pub structure: u16,
    /// What a hit that crossed the head band is multiplied by — the
    /// `headshot_mult` column of `content/weapons.toml`, `= 2` on every
    /// banded row.
    ///
    /// **The third column to arrive here armed and unread**, after the bow
    /// itself and `structure`, and the longest-standing of the three: it
    /// has been parsed (`schema.rs`), pinned to exactly the band
    /// (`balance.rs`), and folded into the content hash (`canon.rs`) since
    /// the content crate was written, while `bake_ranged` dropped it one
    /// line before this struct could hold it. `reference/PROJECTILES.md`
    /// §9.4 named it as a bug of the shape this module keeps repeating —
    /// a number that looks tuned and does nothing — and the fix is the
    /// same one `structure` got: carry it.
    ///
    /// `MeleeDef` deliberately has no twin. A swing is resolved feet-to-
    /// feet in a plane (`strike`), so there is no height to test and no
    /// head to cross; inventing one from `frame.pitch` would be a second
    /// hit model rather than the same one, and the band on a melee row
    /// stays what it has always been — content priced for a mechanic that
    /// does not exist yet, which `balance.rs:122` says in the file.
    pub headshot_mult: u16,
    /// What a hit that reached nothing above the leg band is multiplied
    /// by, in **percent** — the `limb_pct` column of
    /// `content/weapons.toml`, `= 50` on every banded row.
    ///
    /// **The fourth column to arrive here, and the first that did not
    /// arrive armed and unread.** The bow, `structure` and
    /// `headshot_mult` were each parsed, banded and content-hashed for
    /// months before a line of sim read them (this struct's three doc
    /// comments above say so in order). This one landed in the same
    /// commit as the code that reads it, which is the shape the next
    /// column should copy.
    ///
    /// Percent because the ladder needs a *fraction* and a `u16`
    /// multiplier cannot say a half ([`limb`]). 100 is the identity and
    /// is a weapon that does not discount a leg at all.
    ///
    /// `MeleeDef` has no twin, for [`RangedDef::headshot_mult`]'s reason
    /// word for word: `strike` is resolved feet-to-feet in a plane, so
    /// there is no height to test and no band to miss. The column is on
    /// the melee rows in content and stays priced for a mechanic that
    /// does not exist yet, exactly as the head multiplier is.
    pub limb_pct: u16,
    /// Rounds this weapon holds **loaded**, or 0 for a weapon that spends
    /// straight out of the pack — the `magazine` column of
    /// `content/weapons.toml`, set on the revolver and absent on both bows.
    ///
    /// Zero is read as exactly that and not as "unset": `ranged::draw`
    /// (every bow) keeps spending out of the quiver, so a magazine is an
    /// opt-in per row and the arrow path did not move for this.
    ///
    /// The mechanic it buys is the one a body-part ladder needs to be
    /// legible: `rate_ticks` alone makes a firefight an unbroken stream of
    /// identical clicks, and the difference between a leg and a chest is
    /// then only a slightly longer stream. With a magazine, missing has a
    /// price — you go dry, and [`RangedDef::reload_ticks`] is how long you
    /// are helpless for.
    pub magazine: u16,
    /// Ticks to fill the magazine, from `reload_ms`. Zero exactly when
    /// `magazine` is: `validate.rs` refuses one without the other, and the
    /// bake refuses a `reload_ms` that rounds to nothing at `TICK_HZ`.
    ///
    /// Paid on `Player::next_swing`, which is the field the gather, melee
    /// and both ranged cadences already share — so a reload stops a swing
    /// and a swing stops a reload, for free and without a second clock to
    /// keep in step. That sharing is the whole of "helpless for a beat".
    pub reload_ticks: u16,
    /// This weapon's index into `Player::mag`, or [`NO_MAG`].
    ///
    /// Dense and assigned at bake in `weapons.toml` order, so it is not
    /// the item index: `Player::mag` is per-player state and
    /// `MAX_ITEM_DEFS` slots of it would be sixty-four `u16` pairs per
    /// body of which one is used. The bake refuses a weapon past
    /// `limits::MAX_MAGS` rather than dropping its slot, because a weapon
    /// with no slot falls back to spending out of the pack — the mechanic
    /// silently gone with every gate green.
    ///
    /// **Keyed by the weapon row and not by the stack, and the cost is
    /// stated.** Two revolvers in one pack share one magazine, and a gun
    /// handed to another player arrives at the receiver's count for that
    /// kind. The alternative was a fourth field on `ItemStack`, which is
    /// ~500 struct literals across 82 files — and the alternative to
    /// *that* was a side table keyed by inventory slot, which has ten
    /// movers (move, loot, craft, pickup, spill, death, backpack,
    /// containers…) and a silently wrong count at each. Keyed by the
    /// content row there is nothing to move, so there is no site to miss.
    /// `sim-core/tests/reload.rs` gates the corner rather than leaving it
    /// to be rediscovered.
    pub mag_slot: u8,
}

/// A weapon that carries no magazine, in [`RangedDef::mag_slot`].
///
/// `u8::MAX` and not 0, for the reason `NO_ITEM` is not 0 one field up:
/// slot 0 is a real magazine, so a zero-defaulted `mag_slot` on a bow
/// would name the first firearm's rounds — a bow that drained a revolver.
/// Out of range of `MAX_MAGS` by construction, so an unguarded index with
/// it panics in a debug build rather than aliasing.
pub const NO_MAG: u8 = u8::MAX;

/// **Hand-written rather than derived, and the reason is the ammo array.**
/// `#[derive(Default)]` fills a `[u16; N]` with zeros, and zero is a valid
/// item index — item 0 in the sorted-rank mapping is a real item — so a
/// derived default would describe a weapon that fires four copies of
/// whatever sorts first. The empty round slot is `NO_ITEM`, not 0.
///
/// Nothing constructs one this way today (`CombatContent::EMPTY`, the bake
/// and the test fixture all fill every field), and `held_ranged` filters on
/// `damage > 0` so an unarmed row is never handed out regardless. This
/// exists so that stays true by construction rather than by coincidence.
impl Default for RangedDef {
    fn default() -> Self {
        Self {
            damage: 0,
            ammo: [NO_ITEM; MAX_WEAPON_AMMO],
            rate_ticks: 0,
            hitscan: false,
            range_mm: 0,
            structure: 0,
            headshot_mult: 1,
            limb_pct: 100,
            magazine: 0,
            reload_ticks: 0,
            mag_slot: NO_MAG,
        }
    }
}

/// One round's ballistics — what used to be a `[weapon.ballistic]` block on
/// the bow and is now the ammo's own (`reference/PROJECTILES.md` §9.3, the
/// reference game's `ItemModProjectile`). Indexed by item index like every
/// other table here.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AmmoDef {
    /// Muzzle speed in **millimetres per tick**, from `speed_mps`. Zero is
    /// the inert default and means "this item is not a round"; `validate`
    /// refuses a zero in content, so a zero here is only ever an unarmed
    /// slot and never a shipped value.
    pub speed_mmpt: u16,
    /// Gravity in **millimetres per tick squared**, from `drop_mps2`,
    /// subtracted from the vertical velocity once per tick.
    pub drop_mmpt2: u16,
}

/// **Not wearable.** `ArmorDef::slot` is one-based so that a zeroed table
/// is an inert one: item index 0 is a real item, and a zero *slot* has to
/// mean "this is not armor" rather than "this is a headpiece".
pub const WEAR_NONE: u8 = 0;
/// `Player::worn[0]`.
pub const WEAR_HEAD: u8 = 1;
/// `Player::worn[1]`.
pub const WEAR_BODY: u8 = 2;

/// The most a whole worn set may take off one hit, in percent.
///
/// Deliberately **the same 90 `content/validate.rs` already refuses a
/// single row past**, and for the same reason one level up: a body that
/// takes no damage is not a body, and a set is where per-row ceilings stop
/// being enough. Not a new knob — the number is the one the content rail
/// has enforced since M1, applied to the sum rather than to a term.
pub const ARMOR_MAX_PCT: u32 = 90;

/// One item's armor row — `content/armor.toml`, baked by item index like
/// every other table in `CombatContent`.
///
/// `slot == WEAR_NONE` ⇒ the item is not wearable, so the whole table
/// starts inert exactly as `MeleeDef::damage == 0` does. The two columns
/// travel together on purpose: a reduction with no slot protects
/// everything and a slot with no reduction protects nothing, and both are
/// content bugs the bake refuses rather than shapes the sim has to guard.
///
/// **One scalar, not a per-damage-type vector, and that is a decision with
/// a source.** `reference/RIPLIST.md` §1h read the reference game's own
/// Protection tables for all three pieces we ship and found its Projectile
/// and Melee cells **equal on every row** — so one number expresses theirs
/// exactly for every piece that exists here. The columns a vector would add
/// (Bite, Radiation, Cold) key mechanics we either do not ship or have no
/// number for, and the weapon-side half of a type column — which of
/// slash/blunt/stab each of our twenty weapons is — is not in any source
/// anybody here has read. `DECISIONS.md` §open ("armor reduction v0")
/// carries the argument; the vector is the next slice, and it changes
/// numbers rather than the funnel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ArmorDef {
    /// Percent of an incoming hit this piece takes off. `content/validate.rs`
    /// refuses a row past 90, so a `u8` cannot be overrun by content.
    pub reduction_pct: u8,
    /// Which of `WEAR_SLOTS` this piece occupies, **one-based**:
    /// `WEAR_NONE`, `WEAR_HEAD` or `WEAR_BODY`.
    pub slot: u8,
}

/// The whole combat ruleset the sim knows. Construction input like the
/// seed and the gather table; the WAL pins the content hash it was baked
/// from (CONTENT.md §0).
#[derive(Clone, Copy, Debug)]
pub struct CombatContent {
    /// Indexed by item index (the sorted-rank mapping `bake` owns).
    pub melee: [MeleeDef; MAX_ITEM_DEFS],
    /// Throwable rows, indexed the same way. Every entry inert until the
    /// bake installs the throwable rows of `content/weapons.toml`.
    pub throw: [ThrowDef; MAX_ITEM_DEFS],

    /// Indexed the same way. A row here and a row in `melee` are not
    /// exclusive in the type, but nothing in the alpha data is both, and
    /// the ranged read happens first — a bow in hand does not swing.
    ///
    /// ⚠ This line named **`ranged::armed`** until 2026-08-30 and there has
    /// never been a function by that name in the tree. The two reads that
    /// actually do the job are `held_ranged` (below — `damage > 0` is the
    /// whole test) and `ranged::draw`, whose `true` means *the weapon took
    /// the arm*, which is what `World::tick` uses to skip `gather::swing`.
    /// Corrected while arming this table's first fixture ranged row; the
    /// class is `CLAUDE.md`'s dead-citation ⚠, one level down — a doc that
    /// names a symbol is a claim you can `grep`.
    pub ranged: [RangedDef; MAX_ITEM_DEFS],
    /// Ballistics, indexed by the **round's** item index rather than the
    /// weapon's (`reference/PROJECTILES.md` §9.3). A stack of arrows in a
    /// pocket has a row here; the bow that fires them does not.
    pub ammo: [AmmoDef; MAX_ITEM_DEFS],
    /// Worn protection, indexed the same way — the row of the item, not of
    /// the body wearing it. Every entry inert until the bake installs
    /// `content/armor.toml`, which is what it did for the first time on
    /// 2026-08-19: the file had been priced, validated, hashed and
    /// balance-anchored since M1 with nothing in this crate reading a row
    /// (`reference/ARMOR.md` §9.1 is the audit).
    pub armor: [ArmorDef; MAX_ITEM_DEFS],
    /// Max player hp — `content/balance.toml` `globals.player_hp`. Zero is
    /// the inert default and disarms the module entirely: no hp is granted
    /// at join, so no damage is applied and nobody can die.
    pub player_hp: u16,
    /// Chance in 100 that a landed arrow is destroyed rather than lodged
    /// (`globals.arrow_break_pct`, arrow recovery v0). **Zero is not the
    /// inert default here and that is deliberate**: the disarmed value is
    /// 100, which destroys every arrow and is exactly the game that
    /// existed before `spent.rs` — so a content set with no recovery row
    /// cannot silently make ammunition free. `CombatContent::EMPTY` says
    /// 100 for that reason and it is the one field of this struct whose
    /// inert value is not zero.
    pub arrow_break_pct: u16,
    /// Ticks an arrow that DEALT DAMAGE waits before it may be taken back
    /// (`globals.arrow_lodge_s` × `TICK_HZ`, baked). A missed arrow is
    /// takeable on the tick it lands, so this number prices exactly one
    /// thing: re-using the arrow you just shot someone with, mid-fight.
    pub arrow_lodge_ticks: u32,
}

impl CombatContent {
    pub const EMPTY: Self = Self {
        melee: [MeleeDef {
            damage: 0,
            structure: 0,
            reach_cm: 0,
            headshot_mult: 1,
            limb_pct: 100,
        }; MAX_ITEM_DEFS],
        throw: [ThrowDef {
            damage: 0,
            structure: 0,
            fuse_ticks: 0,
            reach_cm: 0,
            blast_cm: 0,
        }; MAX_ITEM_DEFS],
        ranged: [RangedDef {
            damage: 0,
            ammo: [NO_ITEM; MAX_WEAPON_AMMO],
            rate_ticks: 0,
            hitscan: false,
            range_mm: 0,
            structure: 0,
            // One, not zero: the empty row's multiplier is the identity, so
            // a table nothing baked cannot silently delete a hit.
            headshot_mult: 1,
            // A hundred for the same reason at the other end of the
            // ladder: a percent of zero would make the empty row delete a
            // leg hit outright, which is the same defect spelled the other
            // way round.
            limb_pct: 100,
            // No magazine on the empty row, and `NO_MAG` rather than slot
            // 0 for the reason the constant states: a table nothing baked
            // must not name the first firearm's rounds.
            magazine: 0,
            reload_ticks: 0,
            mag_slot: NO_MAG,
        }; MAX_ITEM_DEFS],
        ammo: [AmmoDef {
            speed_mmpt: 0,
            drop_mmpt2: 0,
        }; MAX_ITEM_DEFS],
        armor: [ArmorDef {
            reduction_pct: 0,
            slot: WEAR_NONE,
        }; MAX_ITEM_DEFS],
        player_hp: 0,
        // 100, not 0 — see the field. An inert recovery rule must destroy
        // every arrow, because the opposite failure (a content set with no
        // row silently giving ammunition back forever) is the one that
        // cannot be noticed by looking at the game.
        arrow_break_pct: 100,
        arrow_lodge_ticks: 0,
    };

    /// Synthetic table for the parity/replay/alloc gates. Deliberately
    /// unlike game content: every hotbar-reachable fixture item is a
    /// weapon, and item 0 — which the gather fixture also makes a tool —
    /// kills in three hits, so hits, deaths and respawns land inside the
    /// counted windows instead of waiting on a lucky wander. Reach is the
    /// real melee rows' 2 m on purpose: a fixture that could hit across
    /// the island would put every herd gate quietly into a brawl.
    pub fn probe_fixture() -> Self {
        let mut c = Self::EMPTY;
        c.player_hp = 100;
        // (body damage, structure damage). Item 0's structure damage is
        // deliberately a third of the build fixture's 100 hp piece, so a
        // piece falls inside a counted window in three swings — the same
        // reason its body damage kills in three.
        let rows: [(u16, u16); 4] = [(34, 34), (12, 6), (25, 9), (50, 20)];
        let mut i = 0;
        while i < rows.len() {
            c.melee[i] = MeleeDef {
                damage: rows[i].0,
                structure: rows[i].1,
                reach_cm: 200,
                // The shipped ladder (`content/weapons.toml`: ×2 head,
                // ×0.5 limbs), so the counted gates cross a head band
                // with a club as well as with a bullet — a fixture row
                // nothing reaches rides the parity surface while covering
                // nothing (item 6's own lesson, below).
                headshot_mult: 2,
                limb_pct: 50,
            };
            i += 1;
        }
        // Item 3 is also the fixture's throwable, so the plant verb has a
        // row to read in the counted gates. Its structure damage is the
        // whole of the build fixture's 100 hp piece, so one charge is one
        // wall — a demolition a replay can see land in a single event
        // rather than one it has to sum. Four ticks of fuse for the same
        // reason the reach is 2 m: everything here has to resolve inside a
        // counted window, not a play session.
        c.throw[3] = ThrowDef {
            damage: 0,
            structure: 100,
            fuse_ticks: 4,
            reach_cm: 200,
            blast_cm: 1,
        };
        // Two armor rows, on items 4 and 5 — deliberately *above* the four
        // weapon rows, so no fixture item is both a weapon and a piece of
        // armor and a test cannot accidentally arm what it meant to wear.
        // Nothing in the counted gates wears anything, so these change no
        // probe digest; they exist so `tests/armor.rs` reads a table the
        // fixture declares instead of poking one it built itself.
        c.armor[4] = ArmorDef {
            reduction_pct: 10,
            slot: WEAR_HEAD,
        };
        c.armor[5] = ArmorDef {
            reduction_pct: 20,
            slot: WEAR_BODY,
        };
        // **Item 6 is a hitscan firearm and item 7 is its round**, and this
        // row is the whole reason `probe_combat`'s rewind claim is true.
        // Until 2026-08-30 this table had no `ranged` entry at all, so every
        // `held_ranged` in that probe answered `None`, every `BTN_PRIMARY`
        // fell through to `gather::swing`, and `test_parity_wasm` covered
        // the *melee* reader while `NOW.md` §0lc read as though it covered
        // both. `ranged::hitscan` is the only shot path that consults the
        // rewind ring (`Pose::Rewound`); the arrow deliberately stays live.
        //
        // Continuing the ladder above rather than picking a free number:
        // 0–3 are the melee rows, 4–5 the armor rows, 6–7 the gun and its
        // round. Nothing is two things at once, so a test cannot arm what
        // it meant to wear or shoot what it meant to swing.
        //
        // Every number is chosen for what it *reaches* inside a counted
        // window, which is this fixture's rule everywhere else:
        //   damage 25 — four shots to kill against `player_hp`, so a
        //     gunfight resolves inside 256 ticks without one-shotting the
        //     melee brawl out of the digest;
        //   headshot_mult 2 — nonzero and not 1, so `part_crossed`'s head
        //     band changes an outcome rather than being computed and
        //     discarded;
        //   rate_ticks 8 — faster than the shared swing cadence, so holding
        //     the gun is a different tempo and not a re-skinned club;
        //   range_mm 20_000 — the melee row's 2 m reason at gun scale. Long
        //     enough to cross the spawn ring the three bots share, short
        //     enough not to shoot across the island, and 118 sampler taps
        //     against `MAX_HITSCAN_SAMPLES` (320), so the backstop at
        //     `ranged.rs`'s sample check is not what stops the shot;
        //   structure 0 — a bullet chips nothing here on purpose. The piece
        //     stores are `probe_parity`'s subject and a fixture that
        //     demolished them would hollow that probe out, which is the
        //     same trade `raid_fixture`'s one point of damage refuses.
        // The round carries **no `ammo` row**, and that is not an omission:
        // `hitscan` never looks one up, and a firearm whose round had
        // ballistics is the pairing `content/validate.rs` refuses at boot.
        c.ranged[6] = RangedDef {
            damage: 25,
            ammo: [7, NO_ITEM, NO_ITEM, NO_ITEM],
            rate_ticks: 8,
            hitscan: true,
            range_mm: 20_000,
            structure: 0,
            // A magazine on the fixture's firearm, because a fixture row
            // nothing reaches rides the parity surface while covering
            // nothing (the note four lines down says so about `limb_pct`).
            // Six and a two-tick fill: short enough that a probe running a
            // few hundred ticks empties it and reloads more than once, so
            // the magazine is on the digest's surface rather than beside
            // it. `mag_slot` 0 — the fixture bakes no other magazine.
            magazine: 6,
            reload_ticks: 2,
            mag_slot: 0,
            headshot_mult: 2,
            //   limb_pct 50 — the reference's ×0.5 rather than the
            //     identity, so a leg hit is a *different* number in the
            //     digest and not a chest hit spelled twice. **And the
            //     bots reach it**, which is measured rather than assumed:
            //     0lc slice 6's lesson is that a fixture row nothing
            //     reaches rides the parity surface while covering
            //     nothing. Flipping this one line to the identity moves
            //     the `combat` digest (0x4cbe7083 → 0x3c7b11ac) and its
            //     rewound-hitscan count (2412 → 2415) at 500×256, so the
            //     leg band is on the parity surface for a reason and not
            //     by hope.
            limb_pct: 50,
        };
        c
    }

    /// A table that can raid and cannot fight: hp granted, body damage
    /// zero on every row, one point of structure damage. `probe_parity`
    /// installs it, and the pairing is the point — that probe's bots must
    /// survive to fill inventories, stand pieces and reach the upgrade
    /// rung, so arming the brawl there would hollow out its coverage.
    /// One point a swing chips a wall over a long run without demolishing
    /// the base the rest of the probe is built on.
    pub fn raid_fixture() -> Self {
        let mut c = Self::EMPTY;
        c.player_hp = 100;
        let mut i = 0;
        while i < MAX_ITEM_DEFS {
            c.melee[i] = MeleeDef {
                damage: 0,
                structure: 1,
                reach_cm: 200,
                headshot_mult: 1,
                limb_pct: 100,
            };
            // One point a blast, for the swing's reason: `probe_parity`'s
            // bots must keep the base they built standing long enough to
            // reach the upgrade rung, and a fixture charge that flattened
            // a wall would hollow out the coverage the probe exists for.
            c.throw[i] = ThrowDef {
                damage: 0,
                structure: 1,
                fuse_ticks: 4,
                reach_cm: 200,
                blast_cm: 1,
            };
            i += 1;
        }
        c
    }

    /// The row of the item a player is holding, or `None` when the hand is
    /// empty or the held item is off the table. The two callers filter it:
    /// `held_melee` wants body damage, `held_struct` wants structure
    /// damage, and a row is allowed to carry one without the other.
    #[inline]
    fn held_row(&self, held: u16) -> Option<MeleeDef> {
        if held == NO_ITEM || held as usize >= MAX_ITEM_DEFS {
            return None;
        }
        Some(self.melee[held as usize])
    }

    /// The melee row of the item a player is holding, or `None` when the
    /// hand is empty, the held item is off the table, or the item is not
    /// a weapon.
    #[inline]
    pub fn held_melee(&self, held: u16) -> Option<MeleeDef> {
        self.held_row(held).filter(|d| d.damage > 0)
    }

    /// The same row, filtered on the structure column — what a raid swing
    /// reads. A tool that can dent a wall but not a person would answer
    /// here and not to `held_melee`; nothing in the alpha data is one, and
    /// the bake refuses a melee row missing either column.
    #[inline]
    pub fn held_struct(&self, held: u16) -> Option<MeleeDef> {
        self.held_row(held).filter(|d| d.structure > 0)
    }

    /// The throwable row of the item a player is holding — what the plant
    /// verb reads (`charge.rs`). Both columns are required, because a row
    /// missing either is a charge that cannot hurt a wall or one that
    /// never goes off, and planting either would take the item and give
    /// nothing back.
    ///
    /// **The held item is the cost.** There is no separate price row for a
    /// charge: you plant what is in your hand, which is why this returns
    /// the row without also naming an item. It keeps the raid tool a
    /// content decision — any `throwable` in `content/weapons.toml` is one
    /// — instead of a name the sim would have to know.
    #[inline]
    pub fn held_throw(&self, held: u16) -> Option<ThrowDef> {
        if held == NO_ITEM || held as usize >= MAX_ITEM_DEFS {
            return None;
        }
        let d = self.throw[held as usize];
        // `blast_cm` joins the liveness test because it is a divisor-to-be,
        // not because a zero radius would be a strange charge: an inert row
        // reaching a future falloff would divide by zero, and wall 1's
        // float rules have no NaN to fall back on. Asserted here now so the
        // consumer inherits the invariant instead of having to add it.
        (d.structure > 0 && d.fuse_ticks > 0 && d.blast_cm > 0).then_some(d)
    }

    /// The ranged row of the item a player is holding, or `None` when the
    /// hand is empty, the held item is off the table, or the item fires
    /// nothing. Same bounds rule as `held_row`, against the other array.
    #[inline]
    pub fn held_ranged(&self, held: u16) -> Option<RangedDef> {
        if held == NO_ITEM || held as usize >= MAX_ITEM_DEFS {
            return None;
        }
        Some(self.ranged[held as usize]).filter(|d| d.damage > 0)
    }

    /// The ballistics of round `item`, or `None` when that item is not a
    /// round. `held_ranged`'s bounds rule against the ammo table.
    #[inline]
    pub fn ammo_def(&self, item: u16) -> Option<AmmoDef> {
        if item == NO_ITEM || item as usize >= MAX_ITEM_DEFS {
            return None;
        }
        Some(self.ammo[item as usize]).filter(|a| a.speed_mmpt > 0)
    }
}

impl Default for CombatContent {
    fn default() -> Self {
        Self::EMPTY
    }
}

/// The item in a player's selected hotbar slot, or `NO_ITEM` for an empty
/// hand — the same read `gather::swing` makes, kept in one place.
#[inline]
pub fn held_item(p: &Player) -> u16 {
    if p.inv[p.frame.sel as usize].count > 0 {
        p.inv[p.frame.sel as usize].item
    } else {
        NO_ITEM
    }
}

/// What one debit took off a body.
///
/// `left` is read back rather than recomputed by the caller: `EV_HEALTH`
/// is absolute (its own doc), so every route that announces reads the
/// post-debit hp, and handing it back is what stops four call sites from
/// each deciding when to sample it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hurt {
    /// What actually came off — `min(raw, hp before)`. Never more than the
    /// body had, so a 40-damage blow on a 12-hp body reports 12.
    pub dealt: u16,
    /// The hp left standing after the debit.
    pub left: u16,
    /// The body reached zero **on this debit**. A body that was already at
    /// zero is not killed again, which is what stops a second route in the
    /// same tick from counting a death the first one already counted.
    pub died: bool,
}

/// **The one place a player loses hp.** Every damage route in this crate
/// debits through here, and `tests/damage_routes.rs` is the gate that says
/// so — a write to a player's `hp` anywhere else is a site that test
/// cannot classify, and it fails naming the line.
///
/// Why one place: armor had been priced, validated, hashed and
/// balance-anchored in `content/armor.toml` since M1 with nothing in this
/// crate reading a row of it (`reference/ARMOR.md` §9.1). Reduction is an
/// arm inside *this* function — one line, added on 2026-08-19 and reaching
/// all four hit routes at once, which is the whole of what the funnel was
/// built to buy. It could not be added to three routes and forgotten on
/// the fourth, which is the shape the reference ecosystem's payload bugs
/// actually took.
///
/// **What the funnel owns is the body, and nothing else.** Two writes: the
/// hp and the death count. It does not push an event and it does not lay
/// the corpse down, because neither is uniform across the routes and
/// pretending otherwise would ship a bug every test would pass:
///
/// * `EV_HIT` is an **attacker's** fact — a hitmarker. `strike` and an
///   arrow push it; a blast and a pig deliberately do not (`world.rs`'s
///   bite loop says why: a pig has no screen to draw one on). A funnel
///   with a fixed event set gives pigs hitmarkers.
/// * `EV_HEALTH` differs in its ceiling by route (`cc.player_hp` at the
///   melee and bite sites, `hp_max` at the arrow and the shock) and
///   `survival` announces it packed beside `EV_VITALS` through its own
///   `announce`, only when something moved. Emitting it here would
///   double-announce every drink.
/// * `EV_DEATH` and `World::die` need the whole world — backpacks, the
///   player slot, the event ring — and this takes one `&mut Player`. So
///   the funnel *reports* the death and the caller performs it, which is
///   the shape all five kill sites already had.
///
/// Wall 1: `min`, `saturating_add` and one `u16` subtraction that cannot
/// underflow because `dealt <= before`. No float, no clock, no allocation.
/// The head multiplier, applied to the **raw** damage before the funnel
/// sees it.
///
/// **Before armor and not after, and the order is a decision.** `reduce`
/// takes a percentage, so scaling first and scaling last differ only by
/// integer rounding — but they differ in what they *mean*: a plate that
/// stops 30% of a blow stops 30% of the blow that arrived, and a headshot
/// is a bigger blow, not a smaller plate. `reference/ARMOR.md` §0 has
/// protection proportional to damage for exactly this reason. Doing it
/// here also keeps the funnel's signature a `u16`, so `tests/
/// damage_routes.rs` still sees one door into a body's hp.
///
/// **Not on the funnel itself**, because the funnel is every route and only
/// two of them have a head to hit. A pig's bite (`world.rs`) and a satchel
/// blast (`charge.rs`) resolve against a body with no line to cross, and a
/// `hurt` that took a `head: bool` would ask both of them a question
/// neither can answer — the shape `hurt`'s own doc refuses for `EV_HIT`.
///
/// Saturating rather than wrapping: `u16::MAX` hp does not exist
/// (`balance.toml` caps a body at three digits) so the clamp is
/// unreachable with shipped content, and the alternative is a headshot
/// that heals.
///
/// Wall 1: two `u32` multiplies and a `min`. No float, no allocation.
#[inline]
pub fn headshot(raw: u16, mult: u16) -> u16 {
    (raw as u32 * mult as u32).min(u16::MAX as u32) as u16
}

/// What a hit that reached nothing above the leg band is worth: `pct`
/// percent of the raw damage, floored.
///
/// **A percent and not a multiplier, and the asymmetry with
/// [`headshot`] is the point.** The reference's ladder is ×2 head, ×1
/// chest, ×0.5 limbs (`reference/PROJECTILES.md` §0), and a `u16`
/// multiplier cannot say a half. Widening `headshot_mult` into a percent
/// would move a shipped, banded, content-hashed column on eleven rows to
/// buy nothing the second column does not, so the two live side by side:
/// one says how much *more* a skull is worth and one how much *less* a
/// shin is.
///
/// **Floored, and the floor is reachable in principle and not in
/// content.** `1 × 50 / 100` is 0, a hit that costs a body nothing —
/// `validate` refuses `damage == 0`, so the smallest shipped weapon is
/// the rock's 20 and the smallest leg hit is 10. A shipped zero would
/// need a one-damage weapon, which is a content edit this gate would
/// catch on the way past (`tests/headshot.rs`).
///
/// `pct == 100` is the identity by construction rather than by a branch,
/// which is what lets a weapon opt out of the ladder in *data*: a
/// `limb_pct` of 100 is a weapon whose legs are worth a chest, exactly as
/// `headshot_mult = 1` is one whose skull is worth a chest. The satchel
/// charge already carries the second and now carries the first.
///
/// Wall 1: one `u32` multiply, one integer divide, one `min`. No float,
/// no allocation. The product is at most `u16::MAX × u16::MAX / 100` =
/// 42 948 362, so the `u32` cannot overflow before the `min` clamps it.
#[inline]
pub fn limb(raw: u16, pct: u16) -> u16 {
    (raw as u32 * pct as u32 / 100).min(u16::MAX as u32) as u16
}

/// The one door a shot's damage goes through once the body part is known.
///
/// **One function rather than two branches at two call sites**, and the
/// reason is the trap list's: `ranged` resolves a shot twice (an arrow in
/// `step`, a beam in `hitscan`) and the head multiplier had to be
/// remembered at both. A second multiplier doubles the number of places
/// that can disagree, so the ladder lives here and each site asks it
/// once. [`Part::Chest`] is the identity, which is what makes the
/// fallback free.
///
/// Wall 1: a match and one of [`headshot`]/[`limb`]. No float.
#[inline]
pub fn part_damage(raw: u16, part: Part, head_mult: u16, limb_pct: u16) -> u16 {
    match part {
        Part::Head => headshot(raw, head_mult),
        Part::Chest => raw,
        Part::Limb => limb(raw, limb_pct),
    }
}

#[inline]
pub fn hurt(cc: &CombatContent, v: &mut Player, raw: u16) -> Hurt {
    debit(v, reduce(raw, worn_pct(cc, v)))
}

/// What a worn set takes off every hit, in percent — the sum over the
/// slots, clamped at [`ARMOR_MAX_PCT`].
///
/// **The sum, not the slot that was hit, and that is a v0 decision rather
/// than an oversight.** It used to rest on there being no head to hit at
/// all; since headshot v0 a *shot* has one, so the reason is now the
/// narrower and truer one — **a head band is not a coverage model.**
/// `ranged` knows whether a line crossed the crown; nothing anywhere knows
/// which worn piece was under it, and a swing still has no height at all.
/// Crediting only the body piece would ship
/// `item.armor_burlap_head` as *charged* dead content — craftable, priced,
/// and protecting nobody — on the very day armor started working, which is
/// the same defect this slice exists to remove. Until hit areas land
/// (`findings/armor-design-20260818.md` §7 S6) a worn set is one number and
/// every piece in it contributes. `content/balance.rs`'s anchor already
/// reads armor this way, slot-blind, so this makes the two agree rather
/// than adding a second model.
///
/// A stack only pays in the slot its **baked row** names, one-based. That
/// is what stops two body plates from being worn at once before any wear
/// verb exists to refuse it: the array is indexed by slot, and a piece in
/// the wrong index is ignored rather than counted.
///
/// Wall 1: `+` and `min` over `u32`, one bounded loop of `WEAR_SLOTS`.
/// Wall 2: no allocation.
#[inline]
pub fn worn_pct(cc: &CombatContent, v: &Player) -> u32 {
    let mut pct = 0u32;
    let mut i = 0usize;
    while i < WEAR_SLOTS {
        let s = v.worn[i];
        if s.count > 0 && (s.item as usize) < MAX_ITEM_DEFS {
            let a = cc.armor[s.item as usize];
            if a.slot as usize == i + 1 {
                pct += a.reduction_pct as u32;
            }
        }
        i += 1;
    }
    pct.min(ARMOR_MAX_PCT)
}

/// **May `item` be worn in wear slot `s`?** — `CanWearItem(Item, Int32)`
/// (`reference/ARMOR.md` §1), and the whole of what `CONT_WEAR` refuses.
///
/// One-based `ArmorDef::slot` against a zero-based array index, exactly
/// as `worn_pct` reads it two functions up. That is not a coincidence to
/// be tidied away — it is the invariant. `worn_pct` **ignores** a piece
/// sitting in the wrong index rather than counting it, so before armor v1
/// a mis-slotted piece was inert; now that a move can put one there, the
/// two predicates have to be the same predicate or a helmet in the body
/// slot becomes a thing you can wear for no protection. Written as
/// `slot == s + 1` at both sites, deliberately, so a reader diffing them
/// sees one expression twice.
///
/// `WEAR_NONE` is 0 and `s + 1` is never 0, so "this item is not armor at
/// all" needs no separate branch and no separate refusal — which is why
/// there is exactly one `REFUSE_M_WEAR` rather than three.
///
/// An item index past the table reads as not wearable, the same way
/// `worn_pct` bounds it and for the same reason: content decides the
/// table's width, and a forged index is a wire fact, not a content one.
///
/// **`s` is bounded here rather than by its caller's discipline.** This is
/// `pub` and total, and `s + 1` on a `u8` was neither: it overflow-panics
/// in debug and *wraps to 0* in release, and 0 is `WEAR_NONE` — so a
/// release-mode call with `s = 255` answered **true** for every item in
/// the table that is not armor, which is the exact inverse of what this
/// function is for. Nothing reached it: the one call site bounds `s` by
/// `slots_in(CONT_WEAR)` two steps earlier. A `pub` predicate whose
/// correctness lives at a call site two steps away is a predicate waiting
/// for its second caller, and this one is named in a reference doc as the
/// thing the next equipment slice extends. Raised by the merge-gate judge
/// on pass `20260828-065501-06`.
///
/// Wall 1: one compare. Wall 2: no allocation.
#[inline]
pub fn wearable_in(cc: &CombatContent, item: u16, s: u8) -> bool {
    (s as usize) < WEAR_SLOTS
        && (item as usize) < MAX_ITEM_DEFS
        && cc.armor[item as usize].slot == s + 1
}

/// What gets through `pct` percent of protection.
///
/// **The floor lands on the damage, not on the absorption**, and the two are
/// not the same function under integer division: 25 damage against 35 %
/// is 16 here and 17 the other way round. This form is the one
/// `content/balance.rs`'s `hits_to_kill` divides by — its per-hit number is
/// `damage × (100 − pct) / 100` — so the band the data declares and the
/// fight the sim plays are one arithmetic rather than two that agree by
/// luck. `crates/content/tests/content.rs` pins them equal for every
/// (weapon, armor) pair we ship.
///
/// A hit small enough to round to zero deals zero. That is not new
/// behaviour: `charge::falloff` already returns zero at the edge of a blast
/// and the site skips it, and `debit` treats a zero as the no-op it is.
///
/// Wall 1: `×`, `÷`, `min` on `u32`. No float, no libm.
#[inline]
pub fn reduce(raw: u16, pct: u32) -> u16 {
    let pct = pct.min(ARMOR_MAX_PCT);
    ((raw as u32 * (100 - pct)) / 100) as u16
}

/// The same debit, for the routes that armor must **never** reduce. The
/// name is the choice, made visible in a diff rather than left as an
/// omission that is invisible by construction — the three of them:
///
/// * **starve / dehydrate** (`survival::step`) — metabolic, not a hit. A
///   chest plate does not feed you, and farming in armor must not cost a
///   repair bill.
/// * **salt water** (`survival::drink`) — the hp *is* the price of the
///   drink. Reducing it would make a helmet a desalinator.
/// * **the keypad shock** (`deploy::lock_op`) — it floors at 1 hp and
///   never kills (`lock.rs`'s header). Reducing it would make an armored
///   raider immune to a mechanic whose entire job is to cost tries.
///
/// It is deliberately the same body as [`hurt`]: the difference is which
/// number arrives, never how a body takes it.
#[inline]
pub fn hurt_unreduced(v: &mut Player, raw: u16) -> Hurt {
    debit(v, raw)
}

// Note the shapes: `hurt` takes the content table because reduction is a
// lookup, `hurt_unreduced` deliberately does not take it at all. A route
// that cannot reach the armor table cannot accidentally consult it.

/// The debit itself. Private, so `hurt`/`hurt_unreduced` are the only
/// doors and the choice between them is always written down at the call.
#[inline]
fn debit(v: &mut Player, raw: u16) -> Hurt {
    let before = v.hp;
    let dealt = raw.min(before);
    // `before > 0` and not `dealt >= before` alone: a body already at zero
    // is one the caller has not walked to its respawn yet, and counting a
    // second death for it would be the kill site lying — `survival`'s salt
    // route reasoned its way to exactly this guard before there was a
    // funnel to hold it.
    let died = before > 0 && raw >= before;
    v.hp = before - dealt;
    if died {
        // A death is counted where it happens. Without it a death is
        // invisible to `spawn_pos_n(id, deaths)`, so a body goes back to
        // the identical beach; the count is also what the scoreboard and
        // the wire's `deaths` field read.
        v.deaths = v.deaths.saturating_add(1);
    }
    Hurt {
        dealt,
        left: v.hp,
        died,
    }
}

/// What one swing found. `Missed` is the only outcome that hands the arm
/// on to `raid` — a swing that landed on a person does not also land on
/// the wall behind them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Strike {
    Missed,
    /// A player took the hit and lived.
    Hit,
    /// A player took the hit and died; the slot needs laying down. That is
    /// the caller's, because the backpack drop and the death screen need
    /// the whole world and this function only needs the slot array.
    ///
    /// The weapon and the range travel with the verdict because this is the
    /// only place that still knows them, and the death screen is made of
    /// them: ALPHA.md §1 says a player is told "who/what killed you — range
    /// and weapon, no map position", and by the time `world` has the corpse
    /// the swing that made it is gone. `range_cm` is the planar distance
    /// between the two capsules at the instant of the blow — centimetres
    /// because the reach table is already in them (`MeleeDef::reach_cm`),
    /// so no unit is invented to carry it.
    Killed {
        victim: usize,
        item: u16,
        range_cm: u16,
    },
}

/// Resolve one already-taken swing (cadence paid, no node hit) against
/// the other players.
///
/// Bounded: one pass over `MAX_PLAYERS`, on a swing tick only. Nearest
/// eligible target inside the weapon's reach and gather's aim cone wins,
/// exactly as a node does; the attacker is never a candidate, so no weapon
/// can ever hit its own holder.
/// How many world-bearing sectors an `EV_HURT` is quantized to.
///
/// Sixteen, so a sector is 22.5° — the eight compass points a player can
/// name, halved once so "front-left" is expressible. The width is the
/// disclosure: it is enough to turn toward and not enough to aim with, and
/// widening it is a decision about what a victim learns, not a cosmetic
/// one (`world.rs`, `EV_HURT`).
pub const HURT_SECTORS: u8 = 16;

/// The sector of the world bearing along `(dx, dz)`, clockwise from north.
///
/// `dx`/`dz` are a delta *toward* the thing being pointed at, in any units
/// so long as both axes share them — only the ratio is read. North is `+Z`
/// and east is `−X` (`DECISIONS.md` 2026-08-15, and `client/src/look.rs`
/// `bearing_of` is the float twin this must agree with, and
/// `client/src/render/hud.rs`'s
/// `the_integer_bearing_and_the_float_bearing_are_the_same_bearing` checks
/// the two against each other rather than restating either — it lives on
/// that side because this crate cannot reach `atan2` to check itself).
///
/// **Integer only, because wall 1 forbids the obvious spelling.** There is
/// no `atan2` in this crate and there is not going to be one. A bearing
/// quantized to sixteen sectors does not need one: the sector is decided by
/// four comparisons against `tan(11.25°)`, `tan(33.75°)`, `tan(56.25°)` and
/// `tan(78.75°)`, each held as a millionth so the compare is an `i64`
/// multiply. That is exact — the same integers on native and on wasm — where
/// a float `atan2` is the one thing `test_parity_wasm` exists to catch.
///
/// A zero delta has no bearing. It happens (two bodies standing inside each
/// other is point-blank, which `strike` explicitly allows), and the answer
/// is sector 0: arbitrary, documented, and no more wrong than any other
/// direction when the attacker is on top of you.
pub fn bearing_sector(dx: i64, dz: i64) -> u8 {
    if dx == 0 && dz == 0 {
        return 0;
    }
    // |east| is |dx| and |north| is |dz|, so the quadrant is read off the
    // signs below and the magnitudes never need the negation.
    let a = dx.saturating_abs();
    let b = dz.saturating_abs();
    // How far off the north/south axis, in sectors: 0 is on it, 4 is square
    // onto the east/west one. `tan(11.25°) = 0.198912` and its three
    // siblings, scaled by a million.
    let k = if a.saturating_mul(1_000_000) < b.saturating_mul(198_912) {
        0
    } else if a.saturating_mul(1_000_000) < b.saturating_mul(668_179) {
        1
    } else if a.saturating_mul(1_000_000) < b.saturating_mul(1_496_606) {
        2
    } else if a.saturating_mul(1_000_000) < b.saturating_mul(5_027_339) {
        3
    } else {
        4
    };
    // East is `−X`, so `dx <= 0` is the eastern half. The two axis cases
    // land on the same sector from either side (k == 4 is due east in both
    // eastern quadrants, k == 0 is due south in both southern ones), which
    // is what makes the seams here unobservable.
    match (dx <= 0, dz >= 0) {
        (true, true) => k,              // N..E
        (true, false) => 8 - k,         // E..S
        (false, false) => 8 + k,        // S..W
        (false, true) => (16 - k) % 16, // W..N
    }
}

/// Land one already-taken swing on the body `melee::cast` found the ray
/// entering first.
///
/// **Lag-compensated since slice 4** (`findings/lagcomp-design-20260818.md`
/// §7) and the compensation now lives in the cast: `hit` was solved against
/// bodies put back `favour` ticks by `ranged::nearest_body`, exactly as a
/// bullet's are, and `hit.qy` is the victim's feet *as they were then*, so
/// the head band is measured off the cylinder the blow was decided against
/// (`BodyHit::qy`'s doc has the argument).
///
/// **The ladder is the shot's** (melee aim v1): `part_crossed` over the span
/// the ray spent inside the body, clipped at `stop_t` where the world would
/// have stopped it, then `part_damage` with the melee row's own
/// `headshot_mult` and `limb_pct`. A swing that crosses the crown pays the
/// head, one that reaches only the shins pays the legs, and the attacker's
/// hitmarker says which (`EV_HIT`'s packed part).
///
/// Only the **target** was rewound. The attacker's own eye is read live and
/// deliberately: this swing is his own input, the server has already
/// stepped him this tick, and he is exactly where he thinks he is.
pub fn strike_body(
    cc: &CombatContent,
    attacker: usize,
    hit: &crate::ranged::BodyHit,
    ray: &crate::melee::Ray,
    stop_t: f32,
    players: &mut [Player; MAX_PLAYERS],
    events: &mut EventQueue,
) -> Strike {
    if cc.player_hp == 0 {
        return Strike::Missed; // inert content: combat is not armed
    }
    let a = &players[attacker];
    if !a.active || a.hp == 0 || hit.slot == attacker || hit.slot >= MAX_PLAYERS {
        return Strike::Missed;
    }
    let Some(def) = cc.held_melee(held_item(a)) else {
        return Strike::Missed;
    };
    let attacker_id = a.id;
    // Read before the victim is borrowed mutably, and in the body's own
    // quanta rather than metres: `bearing_sector` wants integers.
    let (aqx, aqz) = (a.body.qx as i64, a.body.qz as i64);
    let weapon = held_item(a);
    {
        let t = &players[hit.slot];
        if !t.active || t.hp == 0 {
            return Strike::Missed;
        }
    }
    // The span inside the body, clipped against the world's stop, scored at
    // its most significant band — `hitscan`'s three lines, with the
    // rewound feet the cast carried out.
    let feet_mm = hit.qy as f32 * (POS_Y_Q * crate::ranged::MM_PER_M);
    let part =
        crate::ranged::part_crossed(ray.o.1, ray.s.1, feet_mm, hit.enter, hit.exit.min(stop_t));
    let dmg = part_damage(def.damage, part, def.headshot_mult, def.limb_pct);
    // The death screen's range: the PLANAR distance to the victim's axis at
    // the closest approach, centimetres — `Strike::Killed`'s documented
    // meaning, kept. `hit.t` is the planar closest-approach fraction, so it
    // scales the ray's planar length and not its 3D one; a stab pitched
    // down at somebody a metre away is a metre, not the hypotenuse. Measured
    // on the geometry the hit was decided on. Floor-by-cast, wall 1's list.
    let planar_mm = (ray.s.0 * ray.s.0 + ray.s.2 * ray.s.2).sqrt();
    let range_cm = (planar_mm * hit.t / 10.0) as u16;

    let v = &mut players[hit.slot];
    let victim_id = v.id;
    // **Live on both ends, and not rewound with the cast.** `range_cm` is a
    // fact about the blow; this bearing is an instruction to the victim —
    // *turn this way* — and they are at their present position when the
    // arc appears, so the only useful bearing is from where they are now to
    // where the attacker is now.
    let sector = bearing_sector(aqx - v.body.qx as i64, aqz - v.body.qz as i64);
    // The funnel, reduced: a swing is the route armor exists to blunt.
    let Hurt { left, died, .. } = hurt(cc, v, dmg);
    events.push(
        EV_HIT,
        attacker_id,
        victim_id,
        crate::world::hit_c(part, dmg),
    );
    // The other half of the same blow, addressed to the other person in it.
    events.push(EV_HURT, victim_id, sector as u32, dmg as u32);
    events.push(EV_HEALTH, victim_id, left as u32, cc.player_hp as u32);
    if died {
        events.push(EV_DEATH, victim_id, attacker_id, 0);
        return Strike::Killed {
            victim: hit.slot,
            item: weapon,
            range_cm,
        };
    }
    Strike::Hit
}

/// What a melee swing lands on the HARD side of an edge piece, whatever
/// the tool (hard/soft v0). One, flat — the reference's rule as players
/// meet it: the hard face is not a farm, and the number is small enough
/// that "wrong side" reads instantly off the hit numbers. Proposed
/// default, DECISIONS.md §open ("hard/soft v0"). Explosives ignore it.
pub const HARD_SIDE_STRUCTURE: u16 = 1;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gather::ItemStack;

    // The raid pick's cases — three swings fell a foundation, the soft side
    // pays full and the hard side one, the deployable wins the tie, a
    // locked door is no longer a vault, a storey out of the capsule is out
    // of reach — used to live here against a hand-built rig, because
    // `combat::raid` was a function this module owned. The pick is
    // `melee::cast` now and the bill is `World::chip`, so those claims are
    // held where the two meet: `tests/melee_aim.rs`, through the world.

    #[test]
    fn held_melee_refuses_hands_junk_and_the_table_edge() {
        let cc = CombatContent::probe_fixture();
        assert_eq!(cc.held_melee(0).map(|d| d.damage), Some(34));
        assert_eq!(cc.held_melee(NO_ITEM), None, "a bare hand is not a weapon");
        assert_eq!(cc.held_melee(9), None, "an item with no weapon row");
        assert_eq!(
            cc.held_melee(MAX_ITEM_DEFS as u16),
            None,
            "an index past the table"
        );
    }

    #[test]
    fn held_item_reads_the_selected_slot_only() {
        let mut p = Player::default();
        p.inv[0] = ItemStack {
            item: 3,
            count: 1,
            cond: 0,
        };
        p.inv[2] = ItemStack {
            item: 7,
            count: 5,
            cond: 0,
        };
        assert_eq!(held_item(&p), 3);
        p.frame.sel = 2;
        assert_eq!(held_item(&p), 7);
        p.frame.sel = 1;
        assert_eq!(held_item(&p), NO_ITEM, "an empty slot is an empty hand");
    }

    /// The fixture's melee rows carry the shipped ladder, and the inert
    /// table carries the identity — so a counted gate crosses a head band
    /// with a club, and `EMPTY` cannot double a zero into something.
    #[test]
    fn the_fixture_melee_rows_carry_the_ladder_and_empty_carries_the_identity() {
        let cc = CombatContent::probe_fixture();
        let spear = cc.held_melee(0).expect("item 0 is the fixture's spear");
        assert_eq!((spear.headshot_mult, spear.limb_pct), (2, 50));
        assert_eq!(
            part_damage(
                spear.damage,
                Part::Head,
                spear.headshot_mult,
                spear.limb_pct
            ),
            spear.damage * 2
        );
        assert_eq!(
            part_damage(
                spear.damage,
                Part::Limb,
                spear.headshot_mult,
                spear.limb_pct
            ),
            spear.damage / 2
        );
        let e = CombatContent::EMPTY.melee[0];
        assert_eq!((e.headshot_mult, e.limb_pct), (1, 100));
    }

    #[test]
    fn inert_content_never_hurts_anyone() {
        let cc = CombatContent::EMPTY;
        let mut players = [Player::default(); MAX_PLAYERS];
        for (i, p) in players.iter_mut().take(2).enumerate() {
            p.id = i as u32 + 1;
            p.active = true;
            p.hp = 100;
            p.inv[0] = ItemStack {
                item: 0,
                count: 1,
                cond: 0,
            };
        }
        let mut ev = EventQueue::default();
        // A hit the cast would have handed over, at a body one metre out.
        let ray = crate::melee::ray(&players[0].body, 0, 128, 2000.0);
        let hit = crate::ranged::BodyHit {
            t: 0.5,
            slot: 1,
            enter: 0.3,
            exit: 0.7,
            qy: players[1].body.qy,
        };
        assert_eq!(
            strike_body(&cc, 0, &hit, &ray, 1.0, &mut players, &mut ev),
            Strike::Missed
        );
        assert_eq!(players[1].hp, 100);
        assert!(ev.is_empty());
    }

    /// The landing scores the span the way a bullet does: a level swing
    /// between two bodies on one ground crosses the head band and pays the
    /// row's multiplier; the same swing clipped by a wall at the shins is a
    /// leg and pays the percent.
    #[test]
    fn the_landing_pays_the_ladder_off_the_span_it_is_handed() {
        let cc = CombatContent::probe_fixture();
        let mut players = [Player::default(); MAX_PLAYERS];
        for (i, p) in players.iter_mut().take(2).enumerate() {
            p.id = i as u32 + 1;
            p.active = true;
            p.hp = cc.player_hp;
            p.hp_max = cc.player_hp;
            p.inv[0] = ItemStack {
                item: 0,
                count: 1,
                cond: 0,
            };
        }
        let def = cc.held_melee(0).unwrap();
        // Level, from the eye: the ray runs at 1.6 m the whole way, inside
        // the head band of a body standing on the same ground.
        let ray = crate::melee::ray(&players[0].body, 0, 128, 2000.0);
        let hit = crate::ranged::BodyHit {
            t: 0.5,
            slot: 1,
            enter: 0.3,
            exit: 0.7,
            qy: players[1].body.qy,
        };
        let mut ev = EventQueue::default();
        assert_eq!(
            strike_body(&cc, 0, &hit, &ray, 1.0, &mut players, &mut ev),
            Strike::Hit
        );
        let e = ev
            .entries()
            .iter()
            .find(|e| e.code == EV_HIT)
            .expect("a hit");
        assert_eq!(crate::world::hit_part(e.c), Part::Head);
        assert_eq!(crate::world::hit_damage(e.c), def.damage * 2);
        assert_eq!(players[1].hp, cc.player_hp - def.damage * 2);
    }
}
