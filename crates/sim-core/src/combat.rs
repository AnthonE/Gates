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
//! headshots (aim is planar until M2's rewound raycasts, so there is no
//! head to hit), no per-weapon cadence (every swing rides gather's one
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
//! on a worn piece, no `move_penalty_pct` consumer, and — the one a player
//! notices — **no way to put armor on**, because that is `CONT_WEAR` and a
//! wire bump (`findings/armor-design-20260818.md` §4).
//!
//! The throwable's damage does not flow through this module's swing arm at
//! all, and that is the design rather than an omission: a charge resolves
//! on the tick its fuse runs out, not on the tick a button was pressed, so
//! it is `World::tick`'s phase list that owns it. What it shares with a
//! swing is the *ending* — both spend the same `MAX_REMOVALS_PER_TICK`
//! allowance and both land through `deploy::damage_piece`.

use crate::build::{
    anchor, BuildContent, Pieces, BUILD_CELL_M, LEVEL_H_M, LOC_EDGE_XLO, LOC_EDGE_ZLO, LOC_PLANE,
    LOC_RISER,
};
use crate::collide::{col_base_y, CAPSULE_HEIGHT_M};
use crate::deploy::{damage_deploy, damage_piece, DeployContent, Deploys};
use crate::fmath::fabs;
use crate::gather::{CONE_COS, DY_MAX_M, NO_ITEM, POINT_BLANK_M2};
use crate::limits::{
    MAX_BUILD_COORD, MAX_BUILD_LEVELS, MAX_ITEM_DEFS, MAX_PLAYERS, MAX_WEAPON_AMMO, WEAR_SLOTS,
};
use crate::movement::{POS_XZ_Q, POS_Y_Q};
use crate::world::{EventQueue, Player, EV_DEATH, EV_HEALTH, EV_HIT};
use crate::yaw_lut::yaw_dir;

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
}

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
    /// `ranged::armed` reads this one first — a bow in hand does not swing.
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
}

impl CombatContent {
    pub const EMPTY: Self = Self {
        melee: [MeleeDef {
            damage: 0,
            structure: 0,
            reach_cm: 0,
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
#[inline]
pub fn hurt(cc: &CombatContent, v: &mut Player, raw: u16) -> Hurt {
    debit(v, reduce(raw, worn_pct(cc, v)))
}

/// What a worn set takes off every hit, in percent — the sum over the
/// slots, clamped at [`ARMOR_MAX_PCT`].
///
/// **The sum, not the slot that was hit, and that is a v0 decision rather
/// than an oversight.** Aim is planar and there is no head to hit
/// (`combat.rs`'s header, still true), so a coverage model has nothing to
/// key on: crediting only the body piece would ship
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
pub fn strike(
    cc: &CombatContent,
    attacker: usize,
    players: &mut [Player; MAX_PLAYERS],
    events: &mut EventQueue,
) -> Strike {
    if cc.player_hp == 0 {
        return Strike::Missed; // inert content: combat is not armed
    }
    let a = &players[attacker];
    if !a.active || a.hp == 0 {
        return Strike::Missed;
    }
    let Some(def) = cc.held_melee(held_item(a)) else {
        return Strike::Missed;
    };
    let reach = def.reach_cm as f32 * 0.01;
    let ax = a.body.qx as f32 * POS_XZ_Q;
    let ay = a.body.qy as f32 * POS_Y_Q;
    let az = a.body.qz as f32 * POS_XZ_Q;
    let (fx, fz) = yaw_dir(a.frame.yaw);
    let attacker_id = a.id;

    let mut best: Option<(f32, usize)> = None;
    for (j, t) in players.iter().enumerate() {
        if j == attacker || !t.active || t.hp == 0 {
            continue;
        }
        let dx = t.body.qx as f32 * POS_XZ_Q - ax;
        let dy = t.body.qy as f32 * POS_Y_Q - ay;
        let dz = t.body.qz as f32 * POS_XZ_Q - az;
        let d2 = dx * dx + dz * dz;
        if d2 > reach * reach || fabs(dy) > DY_MAX_M {
            continue;
        }
        // Standing inside someone has no bearing to test, same rule the
        // node scan uses.
        let aimed = d2 <= POINT_BLANK_M2 || dx * fx + dz * fz > CONE_COS * d2.sqrt();
        if aimed && best.is_none_or(|(bd2, _)| d2 < bd2) {
            best = Some((d2, j));
        }
    }
    let Some((d2, victim)) = best else {
        return Strike::Missed;
    };
    // Floor-by-cast of a sqrt, both on wall 1's allowed list, and the two
    // sides quantize identically because they run the same expression on
    // the same values — the reason the sim sims on what it transmits.
    let range_cm = (d2.sqrt() * 100.0) as u16;
    let weapon = held_item(&players[attacker]);

    let v = &mut players[victim];
    let victim_id = v.id;
    // The funnel, reduced: a swing is the route armor exists to blunt.
    let Hurt { left, died, .. } = hurt(cc, v, def.damage);
    events.push(EV_HIT, attacker_id, victim_id, def.damage as u32);
    events.push(EV_HEALTH, victim_id, left as u32, cc.player_hp as u32);
    if died {
        events.push(EV_DEATH, victim_id, attacker_id, 0);
        return Strike::Killed {
            victim,
            item: weapon,
            range_cm,
        };
    }
    Strike::Hit
}

/// The build cells a swing can reach out of the attacker's own cell.
/// Weapon reach is under 3 m in the alpha data and `BUILD_CELL_M` is 3, so
/// the ring one out covers every anchor a reach can touch; the reach test
/// itself, not this radius, is what decides.
const RAID_CELL_RING: i32 = 1;

/// What a melee swing lands on the HARD side of an edge piece, whatever
/// the tool (hard/soft v0). One, flat — the reference's rule as players
/// meet it: the hard face is not a farm, and the number is small enough
/// that "wrong side" reads instantly off the hit numbers. Proposed
/// default, DECISIONS.md §open ("hard/soft v0"). Explosives ignore it.
pub const HARD_SIDE_STRUCTURE: u16 = 1;

/// Which store a raid swing found, and where.
#[derive(Clone, Copy)]
enum Target {
    Piece {
        cx: u16,
        cz: u16,
        level: u8,
        loc: u8,
    },
    Deploy(usize),
}

/// Resolve one already-taken swing (cadence paid, no node and no player
/// hit) against the base in front of the attacker. Returns true when
/// something took the hit.
///
/// Nearest anchor inside the weapon's reach and gather's aim cone wins —
/// the same pick a node and a player get — and reach is measured to
/// `build::anchor`, the exact point placement measures build reach to: what
/// you can build, you can break. Vertically the rule is the movement
/// collider's own storey-overlap test, so **you can hit what could block
/// you**, and a swing at a ground-floor wall never reaches a wall two
/// storeys up.
///
/// A deployable at the same address as a piece wins the tie, always: the
/// door in the doorway is the intended breach point and it must not be
/// possible to swing "past" it into the frame it hangs in.
///
/// Bounded: `(2·RAID_CELL_RING+1)² = 9` columns × 4 locs of planar tests,
/// each resolving at most `MAX_BUILD_LEVELS` mask bits, plus one pass over
/// the deployable store — on a swing tick only, and allocation-free. The
/// piece store is never scanned linearly; the column index answers
/// occupancy in one probe.
#[allow(clippy::too_many_arguments)]
pub fn raid(
    haven: &crate::terrain::Haven,
    cc: &CombatContent,
    bc: &BuildContent,
    dc: &DeployContent,
    seed: u64,
    attacker: &Player,
    pieces: &mut Pieces,
    deploys: &mut Deploys,
    budget: &mut usize,
    events: &mut EventQueue,
) -> bool {
    if !attacker.active || attacker.hp == 0 {
        return false;
    }
    let Some(def) = cc.held_struct(held_item(attacker)) else {
        return false;
    };
    let reach = def.reach_cm as f32 * 0.01;
    let reach2 = reach * reach;
    let px = attacker.body.qx as f32 * POS_XZ_Q;
    let feet_y = attacker.body.qy as f32 * POS_Y_Q;
    let pz = attacker.body.qz as f32 * POS_XZ_Q;
    let (fx, fz) = yaw_dir(attacker.frame.yaw);

    // Planar reach + aim cone against one anchor — the node scan's shape,
    // written once for both stores.
    let aimed_at = |ax: f32, az: f32| -> Option<f32> {
        let (dx, dz) = (ax - px, az - pz);
        let d2 = dx * dx + dz * dz;
        if d2 > reach2 {
            return None;
        }
        let aimed = d2 <= POINT_BLANK_M2 || dx * fx + dz * fz > CONE_COS * d2.sqrt();
        aimed.then_some(d2)
    };
    // The storey-overlap test `collide::cell_edges_block` uses: the level
    // is reachable when the attacker's capsule spans any of its height.
    let storey_ok = |base: f32, level: u8| -> bool {
        let bottom = base + level as f32 * LEVEL_H_M;
        feet_y < bottom + LEVEL_H_M && feet_y + CAPSULE_HEIGHT_M > bottom
    };

    let mut best: Option<(f32, Target)> = None;

    // --- deployables: first, so an equidistant piece cannot displace one.
    for (di, rec) in deploys.entries().iter().enumerate() {
        let Some(d2) = aimed_at_rec(&aimed_at, rec.cx, rec.cz, rec.loc) else {
            continue;
        };
        if !storey_ok(
            col_base_y(seed, haven, pieces.cols(), rec.cx, rec.cz),
            rec.level,
        ) {
            continue;
        }
        if best.is_none_or(|(bd2, _)| d2 < bd2) {
            best = Some((d2, Target::Deploy(di)));
        }
    }

    // --- pieces, through the column index (never the store).
    let pcx = crate::fmath::floor_i32(px / BUILD_CELL_M);
    let pcz = crate::fmath::floor_i32(pz / BUILD_CELL_M);
    let mut dcz = -RAID_CELL_RING;
    while dcz <= RAID_CELL_RING {
        let mut dcx = -RAID_CELL_RING;
        while dcx <= RAID_CELL_RING {
            let (cx, cz) = (pcx + dcx, pcz + dcz);
            dcx += 1;
            if cx < 0 || cz < 0 || cx >= MAX_BUILD_COORD as i32 || cz >= MAX_BUILD_COORD as i32 {
                continue;
            }
            let (cx, cz) = (cx as u16, cz as u16);
            let m = pieces.cols().get(cx, cz);
            let mut base: Option<f32> = None;
            for (loc, mask) in [
                (LOC_PLANE, m.planes),
                (LOC_RISER, m.stairs),
                (
                    LOC_EDGE_XLO,
                    m.walls_xlo | m.doors_xlo | m.wins_xlo | m.frames_xlo,
                ),
                (
                    LOC_EDGE_ZLO,
                    m.walls_zlo | m.doors_zlo | m.wins_zlo | m.frames_zlo,
                ),
                (crate::build::LOC_TRI_XLO_ZLO, m.tri_xlo_zlo),
                (crate::build::LOC_TRI_XHI_ZLO, m.tri_xhi_zlo),
                (crate::build::LOC_TRI_XLO_ZHI, m.tri_xlo_zhi),
                (crate::build::LOC_TRI_XHI_ZHI, m.tri_xhi_zhi),
                (crate::build::LOC_DIAG_A, m.diag_a),
                (crate::build::LOC_DIAG_B, m.diag_b),
            ] {
                if mask == 0 {
                    continue;
                }
                let (ax, az) = anchor(cx, cz, loc);
                let Some(d2) = aimed_at(ax, az) else { continue };
                if best.is_some_and(|(bd2, _)| d2 >= bd2) {
                    continue; // an equal or nearer target already stands
                }
                let base =
                    *base.get_or_insert_with(|| col_base_y(seed, haven, pieces.cols(), cx, cz));
                for level in 0..MAX_BUILD_LEVELS as u8 {
                    if mask & (1 << level) == 0 || !storey_ok(base, level) {
                        continue;
                    }
                    best = Some((d2, Target::Piece { cx, cz, level, loc }));
                    // A 1.7 m capsule spans at most two 3 m storeys; the
                    // lower one is the one the feet are in, and the pick
                    // has to be a rule, not an ordering accident.
                    break;
                }
            }
        }
        dcz += 1;
    }

    match best {
        None => false,
        Some((_, Target::Deploy(di))) => {
            damage_deploy(dc, pieces, deploys, di, def.structure, events);
            true
        }
        Some((_, Target::Piece { cx, cz, level, loc })) => {
            // The index the column index promised: an address the masks
            // report occupied is in the store, but the lookup is fallible
            // by type and refusing beats indexing on a promise.
            match pieces.find_index(cx, cz, level, loc) {
                Some(i) => {
                    // Hard side and soft side (hard/soft v0,
                    // `reference/BUILDING.md` §7b.5): a melee swing on an
                    // edge piece's HARD face lands `HARD_SIDE_STRUCTURE`,
                    // whatever the tool — the reference's own rule as a
                    // raider meets it. Planes and risers have no sides;
                    // the satchel does not care either (`charge.rs` — a
                    // blast is a point, not a stance).
                    let rec = pieces.entries()[i];
                    let sided = crate::build::shape_has_facing(bc.pieces[rec.row as usize].shape);
                    let amount = if sided && !crate::build::soft_side(&rec, px, pz) {
                        HARD_SIDE_STRUCTURE
                    } else {
                        def.structure
                    };
                    damage_piece(dc, bc, pieces, deploys, i, amount, budget, events);
                    true
                }
                None => false,
            }
        }
    }
}

/// `aimed_at` against a store record's address — the anchor lookup the two
/// scans would otherwise repeat.
#[inline]
fn aimed_at_rec(
    aimed_at: &impl Fn(f32, f32) -> Option<f32>,
    cx: u16,
    cz: u16,
    loc: u8,
) -> Option<f32> {
    let (ax, az) = anchor(cx, cz, loc);
    aimed_at(ax, az)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gather::ItemStack;
    use crate::limits::{INV_SLOTS, MAX_REMOVALS_PER_TICK};

    /// One tick's structural removal budget, as `World::tick` hands it
    /// out — these fixtures never approach it (build.rs owns the tests
    /// that do).
    fn tick_budget() -> usize {
        MAX_REMOVALS_PER_TICK
    }

    use crate::movement::Body;
    use crate::world::{EV_DEPLOY_REMOVED, EV_PIECE_REMOVED, EV_STRUCT_HIT};

    const SEED: u64 = 20260731;

    /// The solved authored sites for `SEED`, memoized.
    ///
    /// `terrain::haven` is a few thousand `height` taps and these cases call
    /// the carved-ground path from nearly every assertion, so resolving it
    /// once per suite is the difference between a fast test and a slow one.
    /// It is a pure function of the seed, so caching it cannot change a result.
    fn hv() -> &'static crate::terrain::Haven {
        static HV: std::sync::OnceLock<crate::terrain::Haven> = std::sync::OnceLock::new();
        HV.get_or_init(|| crate::terrain::haven(SEED))
    }
    const CX: u16 = 341;
    const CZ: u16 = 341;
    /// Wire yaw facing +X — LUT index 64 of 256 (yaw_lut.rs: index 0 is
    /// +Z, increasing index rotates toward +X).
    const YAW_PLUS_X: u16 = 64 << 8;
    const YAW_MINUS_X: u16 = 192 << 8;

    fn cc() -> CombatContent {
        CombatContent::probe_fixture()
    }

    fn last(ev: &EventQueue) -> (u8, u32, u32, u32) {
        let e = ev.entries()[ev.len() - 1];
        (e.code, e.a, e.b, e.c)
    }

    /// The raid rig: a wood foundation (fixture row 0, 100 hp) at
    /// (CX, CZ) and a raider standing on it holding fixture item 0 —
    /// 34 structure damage a swing, so three swings take it.
    fn rig() -> (
        CombatContent,
        BuildContent,
        DeployContent,
        Pieces,
        Deploys,
        Player,
    ) {
        let bc = BuildContent::probe_fixture();
        let dc = DeployContent::probe_fixture();
        let mut pieces = Pieces::new();
        let deploys = Deploys::new();
        let mut builder = raider(CX, CZ);
        builder.inv[0] = ItemStack {
            item: 0,
            count: 99,
            cond: 0,
        };
        let mut ev = EventQueue::default();
        crate::build::place(
            SEED,
            hv(),
            &bc,
            &deploys,
            &mut pieces,
            &mut builder,
            0,
            0,
            CX,
            CZ,
            0,
            LOC_PLANE,
            false,
            &mut ev,
        );
        assert_eq!(pieces.len(), 1, "the rig needs its foundation");
        (
            CombatContent::probe_fixture(),
            bc,
            dc,
            pieces,
            deploys,
            raider(CX, CZ),
        )
    }

    /// A player standing at the center of build cell (cx, cz), holding
    /// fixture item 0 and facing +x.
    fn raider(cx: u16, cz: u16) -> Player {
        let mut p = Player {
            id: 7,
            active: true,
            hp: 100,
            body: Body::at(
                SEED,
                hv(),
                (cx as f32 + 0.5) * BUILD_CELL_M,
                (cz as f32 + 0.5) * BUILD_CELL_M,
            ),
            ..Player::default()
        };
        p.inv[0] = ItemStack {
            item: 0,
            count: 1,
            cond: 0,
        };
        p
    }

    fn codes(ev: &EventQueue) -> Vec<u8> {
        ev.entries().iter().map(|e| e.code).collect()
    }

    #[test]
    fn three_swings_fell_a_hundred_hp_foundation() {
        let (cc, bc, dc, mut pieces, mut deploys, p) = rig();
        let mut ev = EventQueue::default();

        // 100 hp, 34 a swing: 66, 32, gone.
        for expect_left in [66u32, 32] {
            ev.clear();
            assert!(raid(
                hv(),
                &cc,
                &bc,
                &dc,
                SEED,
                &p,
                &mut pieces,
                &mut deploys,
                &mut tick_budget(),
                &mut ev
            ));
            let e = ev.entries()[0];
            assert_eq!(e.code, EV_STRUCT_HIT);
            assert_eq!(e.a, crate::gather::cell_key(CX, CZ));
            assert_eq!(e.b, LOC_PLANE as u32);
            assert_eq!(
                e.c,
                (34 << 16) | expect_left,
                "damage dealt << 16 | hp left"
            );
            assert_eq!(pieces.len(), 1, "still standing");
        }

        ev.clear();
        assert!(raid(
            hv(),
            &cc,
            &bc,
            &dc,
            SEED,
            &p,
            &mut pieces,
            &mut deploys,
            &mut tick_budget(),
            &mut ev
        ));
        assert_eq!(
            ev.entries()[0].c,
            32 << 16,
            "the last swing deals only the hp that was left, and leaves zero"
        );
        assert_eq!(codes(&ev), [EV_STRUCT_HIT, EV_PIECE_REMOVED]);
        assert!(pieces.is_empty(), "the foundation fell");

        // Nothing left to hit: the swing finds no target at all.
        ev.clear();
        assert!(!raid(
            hv(),
            &cc,
            &bc,
            &dc,
            SEED,
            &p,
            &mut pieces,
            &mut deploys,
            &mut tick_budget(),
            &mut ev
        ));
        assert!(ev.is_empty());
    }

    /// Hard/soft v0 (`reference/BUILDING.md` §7b.5): the same tool, the
    /// same wall, two stances — full structure damage from the soft side,
    /// `HARD_SIDE_STRUCTURE` from the hard one. The wall was placed from
    /// inside the cell, so its soft face is the +x side the builder stood
    /// on; the foundation (a plane) stays sideless and keeps taking full
    /// damage from anywhere, which the base rig's own tests already pin.
    #[test]
    fn the_soft_side_pays_full_and_the_hard_side_pays_one() {
        let (cc, bc, dc, mut pieces, mut deploys, _) = rig();
        // The wall on the cell's west edge, placed by a builder standing
        // at the cell centre — east of the edge, so soft faces east.
        let mut builder = raider(CX, CZ);
        builder.inv[0] = ItemStack {
            item: 0,
            count: 99,
            cond: 0,
        };
        let mut ev = EventQueue::default();
        crate::build::place(
            SEED,
            hv(),
            &bc,
            &deploys,
            &mut pieces,
            &mut builder,
            0,
            1,
            CX,
            CZ,
            0,
            crate::build::LOC_EDGE_XLO,
            false,
            &mut ev,
        );
        let wall = pieces.find(CX, CZ, 0, crate::build::LOC_EDGE_XLO).unwrap();
        assert_eq!(wall.facing, 1, "soft faces the builder's side (+x)");
        let wall_hp = wall.hp;

        let x0 = CX as f32 * BUILD_CELL_M;
        let z0 = CZ as f32 * BUILD_CELL_M;
        // Both attackers face +Z (default yaw) at the wall's anchor,
        // 0.3 m to either side of the edge plane.
        let mut soft = raider(CX, CZ);
        soft.body = Body::at(SEED, hv(), x0 + 0.3, z0 + 0.5);
        let mut hard = raider(CX, CZ);
        hard.body = Body::at(SEED, hv(), x0 - 0.3, z0 + 0.5);

        ev.clear();
        assert!(raid(
            hv(),
            &cc,
            &bc,
            &dc,
            SEED,
            &soft,
            &mut pieces,
            &mut deploys,
            &mut tick_budget(),
            &mut ev
        ));
        let e = ev.entries()[0];
        assert_eq!(e.code, EV_STRUCT_HIT);
        assert_eq!(
            e.c >> 16,
            34,
            "the soft side takes the tool's whole structure damage"
        );

        ev.clear();
        assert!(raid(
            hv(),
            &cc,
            &bc,
            &dc,
            SEED,
            &hard,
            &mut pieces,
            &mut deploys,
            &mut tick_budget(),
            &mut ev
        ));
        let e = ev.entries()[0];
        assert_eq!(e.code, EV_STRUCT_HIT);
        assert_eq!(
            e.c >> 16,
            HARD_SIDE_STRUCTURE as u32,
            "the hard side takes one, whatever the tool"
        );
        assert_eq!(
            e.c & 0xFFFF,
            (wall_hp - 34 - HARD_SIDE_STRUCTURE) as u32,
            "both swings landed on the one wall"
        );
    }

    #[test]
    fn a_swing_out_of_reach_or_behind_you_breaks_nothing() {
        let (cc, bc, dc, mut pieces, mut deploys, _) = rig();
        let mut ev = EventQueue::default();

        // Four cells away — 12 m, far past the fixture's 2 m reach.
        let far = raider(CX + 4, CZ);
        assert!(!raid(
            hv(),
            &cc,
            &bc,
            &dc,
            SEED,
            &far,
            &mut pieces,
            &mut deploys,
            &mut tick_budget(),
            &mut ev
        ));

        // In reach but aimed away: one cell at +x (3 m; the anchor is 3 m
        // at −x of them) facing +x, so the foundation is behind.
        let mut turned = raider(CX + 1, CZ);
        turned.frame.yaw = 0;
        assert!(!raid(
            hv(),
            &cc,
            &bc,
            &dc,
            SEED,
            &turned,
            &mut pieces,
            &mut deploys,
            &mut tick_budget(),
            &mut ev
        ));

        assert!(ev.is_empty(), "no event, and no phantom damage");
        assert_eq!(pieces.entries()[0].hp, 100, "untouched");
    }

    #[test]
    fn the_deployable_wins_the_tie_and_falls_first() {
        let (cc, bc, dc, mut pieces, mut deploys, p) = rig();
        let mut ev = EventQueue::default();
        // A hearth (fixture row 0, 100 hp) on the foundation — the same
        // address, so both are equidistant from the swing.
        let mut owner = raider(CX, CZ);
        owner.inv[0] = ItemStack {
            item: 0,
            count: 99,
            cond: 0,
        };
        owner.inv[1] = ItemStack {
            item: 2,
            count: 9,
            cond: 0,
        }; // the hearth's own item
        crate::deploy::place_deploy(
            SEED,
            hv(),
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut owner,
            0,
            0,
            CX,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(deploys.len(), 1, "the rig needs its hearth");

        for _ in 0..3 {
            ev.clear();
            assert!(raid(
                hv(),
                &cc,
                &bc,
                &dc,
                SEED,
                &p,
                &mut pieces,
                &mut deploys,
                &mut tick_budget(),
                &mut ev
            ));
            assert_eq!(
                ev.entries()[0].b & crate::world::STRUCT_DEPLOY_BIT,
                crate::world::STRUCT_DEPLOY_BIT,
                "the hearth takes it, never the foundation under it"
            );
        }
        assert!(deploys.is_empty(), "the hearth fell in three");
        assert_eq!(
            pieces.entries()[0].hp,
            100,
            "the foundation never took a point while the hearth stood"
        );

        // With the hearth gone the same swing reaches the foundation.
        ev.clear();
        assert!(raid(
            hv(),
            &cc,
            &bc,
            &dc,
            SEED,
            &p,
            &mut pieces,
            &mut deploys,
            &mut tick_budget(),
            &mut ev
        ));
        assert_eq!(pieces.entries()[0].hp, 66);
    }

    #[test]
    fn a_locked_door_is_no_longer_a_vault() {
        // The gap this slice closes, stated as a test: a door locked
        // against a stranger opens to force. Breaking it must also unseal
        // the edge — a breach nobody can walk through is not a breach.
        let bc = BuildContent::probe_fixture();
        let dc = DeployContent::probe_fixture();
        let mut pieces = Pieces::new();
        let mut deploys = Deploys::new();
        let mut ev = EventQueue::default();
        let mut owner = raider(CX, CZ);
        owner.inv[0] = ItemStack {
            item: 0,
            count: 99,
            cond: 0,
        };
        owner.inv[4] = ItemStack {
            item: 4,
            count: 9,
            cond: 0,
        };
        owner.inv[5] = ItemStack {
            item: 7,
            count: 2,
            cond: 0,
        };
        crate::build::place(
            SEED,
            hv(),
            &bc,
            &deploys,
            &mut pieces,
            &mut owner,
            0,
            0,
            CX,
            CZ,
            0,
            LOC_PLANE,
            false,
            &mut ev,
        );
        crate::build::place(
            SEED,
            hv(),
            &bc,
            &deploys,
            &mut pieces,
            &mut owner,
            0,
            3,
            CX,
            CZ,
            0,
            LOC_EDGE_XLO,
            false,
            &mut ev,
        );
        assert_eq!(pieces.len(), 2, "foundation + doorway");
        crate::deploy::place_deploy(
            SEED,
            hv(),
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut owner,
            0,
            2,
            CX,
            CZ,
            0,
            LOC_EDGE_XLO,
            &mut ev,
        );
        assert_eq!(deploys.len(), 1, "the door hangs in the doorway");
        // Bolt the code lock on (row 5) and arm it — lock v1: a door is
        // bare until somebody pays for the security.
        crate::deploy::place_deploy(
            SEED,
            hv(),
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut owner,
            0,
            5,
            CX,
            CZ,
            0,
            LOC_EDGE_XLO,
            &mut ev,
        );
        crate::deploy::lock_op(
            &dc,
            &crate::gather::GatherContent::probe_fixture(),
            &mut deploys,
            &mut owner,
            CX,
            CZ,
            0,
            LOC_EDGE_XLO,
            crate::deploy::ACCESS_OP_SET_CODE,
            1234,
            0,
            &mut ev,
            &mut [ItemStack::default(); INV_SLOTS],
        );
        assert!(deploys.entries()[0].locked, "and it is locked");
        assert_ne!(
            pieces.cols().get(CX, CZ).shut_xlo,
            0,
            "a shut door seals its edge"
        );

        // A stranger outside, one cell at −x, facing the door: the lock
        // refuses the use verb…
        let mut foe = raider(CX - 1, CZ);
        foe.id = 9;
        foe.frame.yaw = YAW_PLUS_X;
        ev.clear();
        crate::deploy::use_door(
            &dc,
            &mut pieces,
            &mut deploys,
            &mut foe,
            CX,
            CZ,
            0,
            LOC_EDGE_XLO,
            &mut ev,
        );
        assert_eq!(
            last(&ev).2,
            crate::deploy::REFUSE_D_OWNER,
            "the lock still holds against the use verb"
        );
        assert_ne!(pieces.cols().get(CX, CZ).shut_xlo, 0, "still sealed");

        // …and the swing does not care. 60 hp, 34 a swing: two.
        ev.clear();
        assert!(raid(
            hv(),
            &cc(),
            &bc,
            &dc,
            SEED,
            &foe,
            &mut pieces,
            &mut deploys,
            &mut tick_budget(),
            &mut ev
        ));
        assert_eq!(
            ev.entries()[0].b & crate::world::STRUCT_DEPLOY_BIT,
            crate::world::STRUCT_DEPLOY_BIT,
            "the door, not the doorway it hangs in"
        );
        assert_eq!(ev.entries()[0].c, (34 << 16) | 26);
        assert!(raid(
            hv(),
            &cc(),
            &bc,
            &dc,
            SEED,
            &foe,
            &mut pieces,
            &mut deploys,
            &mut tick_budget(),
            &mut ev
        ));
        assert!(deploys.is_empty(), "the door fell to force");
        assert_eq!(
            pieces.cols().get(CX, CZ).shut_xlo,
            0,
            "and the edge is open — the raider can walk in"
        );
        assert_eq!(pieces.len(), 2, "the frame it hung in still stands");

        // The frame next, and it must not cascade a door that is gone.
        // The raider walks IN through the edge the fallen door opened
        // first: the doorway's soft face is the builder's side (hard/soft
        // v0), and from the street its hard face pays `HARD_SIDE_STRUCTURE`
        // a swing — the melee raid the rule exists to slow. Standing
        // inside, aimed back out at the frame, the swings land whole.
        // Just inside the edge, not the cell centre: the centre is the
        // foundation's own anchor, and the nearest pick would hand the
        // swings to the slab underfoot.
        foe.body = Body::at(
            SEED,
            hv(),
            CX as f32 * BUILD_CELL_M + 0.4,
            (CZ as f32 + 0.5) * BUILD_CELL_M,
        );
        foe.frame.yaw = YAW_MINUS_X;
        ev.clear();
        for _ in 0..3 {
            raid(
                hv(),
                &cc(),
                &bc,
                &dc,
                SEED,
                &foe,
                &mut pieces,
                &mut deploys,
                &mut tick_budget(),
                &mut ev,
            );
        }
        assert_eq!(pieces.len(), 1, "the doorway fell; the foundation stands");
        assert!(
            !codes(&ev).contains(&EV_DEPLOY_REMOVED),
            "no deployable was left to cascade"
        );
        assert!(codes(&ev).contains(&EV_PIECE_REMOVED));
    }

    #[test]
    fn a_storey_out_of_the_capsule_is_out_of_reach() {
        // The vertical rule is the movement collider's own: you can hit
        // what could block you. Lift the swinger two storeys and the
        // foundation under them stops being a target.
        let (cc, bc, dc, mut pieces, mut deploys, ground) = rig();
        let mut ev = EventQueue::default();
        assert!(raid(
            hv(),
            &cc,
            &bc,
            &dc,
            SEED,
            &ground,
            &mut pieces,
            &mut deploys,
            &mut tick_budget(),
            &mut ev
        ));
        assert_eq!(pieces.entries()[0].hp, 66, "from the ground, reachable");

        let mut aloft = ground;
        // +6 m in the 1 cm position quantum (movement::POS_Y_Q): two
        // storeys of LEVEL_H_M, well clear of a 1.7 m capsule.
        aloft.body.qy += 600;
        ev.clear();
        assert!(!raid(
            hv(),
            &cc,
            &bc,
            &dc,
            SEED,
            &aloft,
            &mut pieces,
            &mut deploys,
            &mut tick_budget(),
            &mut ev
        ));
        assert_eq!(pieces.entries()[0].hp, 66, "two storeys up, untouched");
        assert!(ev.is_empty());
    }

    #[test]
    fn inert_structure_damage_never_touches_a_base() {
        let (_, bc, dc, mut pieces, mut deploys, p) = rig();
        let cc = CombatContent::EMPTY;
        let mut ev = EventQueue::default();
        assert!(!raid(
            hv(),
            &cc,
            &bc,
            &dc,
            SEED,
            &p,
            &mut pieces,
            &mut deploys,
            &mut tick_budget(),
            &mut ev
        ));
        assert_eq!(pieces.entries()[0].hp, 100);
        assert!(ev.is_empty());
    }

    #[test]
    fn a_dead_or_empty_handed_raider_breaks_nothing() {
        let (cc, bc, dc, mut pieces, mut deploys, _) = rig();
        let mut ev = EventQueue::default();

        let mut dead = raider(CX, CZ);
        dead.hp = 0;
        assert!(!raid(
            hv(),
            &cc,
            &bc,
            &dc,
            SEED,
            &dead,
            &mut pieces,
            &mut deploys,
            &mut tick_budget(),
            &mut ev
        ));

        let mut empty = raider(CX, CZ);
        empty.inv[0] = ItemStack::default();
        assert!(!raid(
            hv(),
            &cc,
            &bc,
            &dc,
            SEED,
            &empty,
            &mut pieces,
            &mut deploys,
            &mut tick_budget(),
            &mut ev
        ));

        let mut gone = raider(CX, CZ);
        gone.active = false;
        assert!(!raid(
            hv(),
            &cc,
            &bc,
            &dc,
            SEED,
            &gone,
            &mut pieces,
            &mut deploys,
            &mut tick_budget(),
            &mut ev
        ));

        assert_eq!(pieces.entries()[0].hp, 100);
        assert!(ev.is_empty());
    }

    #[test]
    fn a_swing_that_hit_a_player_never_also_hits_the_wall() {
        // The tick's rule, asserted where it lives: `Strike::Hit` is not
        // `Strike::Missed`, so `world::tick` never passes the arm on.
        let cc = CombatContent::probe_fixture();
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
            p.body = Body::at(SEED, hv(), 512.0, 512.0);
        }
        let mut ev = EventQueue::default();
        assert_eq!(strike(&cc, 0, &mut players, &mut ev), Strike::Hit);
        assert_eq!(players[1].hp, 66);
        assert_eq!(strike(&cc, 0, &mut players, &mut ev), Strike::Hit);
        assert_eq!(
            strike(&cc, 0, &mut players, &mut ev),
            Strike::Killed {
                victim: 1,
                // The two bodies stand on the same point, so the blow lands
                // at zero range — and the weapon is the fixture's item 0,
                // which is what the death screen will name.
                item: 0,
                range_cm: 0,
            },
            "the third of three"
        );
        assert_eq!(
            strike(&cc, 0, &mut players, &mut ev),
            Strike::Missed,
            "a corpse is not a target, and the arm is free for a wall"
        );
    }

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
        assert_eq!(strike(&cc, 0, &mut players, &mut ev), Strike::Missed);
        assert_eq!(players[1].hp, 100);
        assert!(ev.is_empty());
    }
}
