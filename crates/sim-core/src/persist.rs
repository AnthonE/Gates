//! What a shard remembers about a player, as a fixed record — **the game
//! cannot remember anyone without this file.**
//!
//! Before it, `Command::Join` built a fresh character every time: beach
//! spawn, full hp, empty inventory, nothing loaded because there was
//! nothing to load from. Every disconnect was a wipe of one. So no
//! admission scheme bought anything — you could authenticate perfectly and
//! still lose everything on a dropped connection.
//!
//! ## Identity is not here, deliberately
//!
//! This record carries **no key**. It is what is saved, never who it is
//! saved for: the shard's store keys it under an opaque `PlayerKey` that
//! the admission seam returns (`server/src/store.rs`), and whether that key
//! came from a session token, a signature or a dev flag is one function's
//! return type. `Player::id` is not saved either, and that is the same
//! point from the other side — an id is `generation << 8 | slot`, minted
//! per connection and meaningless across two of them.
//!
//! Nor is anything elo owns. A player's profile, coins and items live in
//! the launcher's world, not this one; a shard that tried to cache them
//! would be a second, stale copy of somebody else's ledger. What a shard
//! knows is where your body stood and what was in its hands.
//!
//! ## The record IS the format (wall 6's posture, for a file)
//!
//! Field order and [`PLAYER_SAVE_BYTES`] are the on-disk layout. Changing
//! either is a format change: bump `SAVE_FORMAT` in `server/src/store.rs`
//! **in the same commit** as the golden below is regenerated, exactly as a
//! packet layout moves with `PROTO_VER`. A save file whose header does not
//! match is refused at boot, so a silent reinterpretation of these bytes is
//! not reachable — but only because the header check exists, not because
//! the layout is self-describing. It is not.
//!
//! ## A save file is untrusted input to the sim
//!
//! It is the only path by which non-command data enters `World`, so
//! [`PlayerSave::read_le`] validates like a wire decoder rather than
//! trusting its own writer: every field that indexes a content table is
//! range-checked, the craft queue's density invariant is checked, and a
//! record that fails is refused whole. `craft::step` indexes
//! `cc.recipes[job.recipe]` with no bound of its own — a hand-edited
//! record naming recipe 70_000 would panic the sim thread, and the check
//! here is what makes that unreachable.
//!
//! Pure like everything else in this crate: no I/O, no clock, no
//! allocation. The bytes are handed in and out as fixed arrays; the file
//! they came from is the server's business.

use crate::craft::CraftJob;
use crate::gather::ItemStack;
use crate::limits::{
    CRAFT_COUNT_MAX, CRAFT_QUEUE, INV_SLOTS, MAX_ITEM_DEFS, MAX_RECIPES, WEAR_SLOTS,
};
use crate::movement::{self, Body};
use crate::terrain::ISLAND_SIZE;
use crate::world::Player;

/// Encoded size of one record. Derived from the two capacities it carries
/// rather than restated, so widening the inventory or the craft queue moves
/// this number and takes the format version with it (`store.rs` asserts the
/// file's `record_len` against it at boot).
///
/// An inventory slot is **6 bytes since `SAVE_FORMAT` 3** (item durability
/// v0): item, count, condition. 196 → 256. A worn slot is the same six
/// bytes and there are `WEAR_SLOTS` of them since `SAVE_FORMAT` 4 (armor
/// v0): 256 → 268. You log off in your armor, you log in in your armor —
/// the same sentence `hp` gets, and the alternative (a naked wake) would
/// make logging out a way to lose a plate.
pub const PLAYER_SAVE_BYTES: usize =
    SCALARS_BYTES + CRAFT_QUEUE * 4 + INV_SLOTS * 6 + WEAR_SLOTS * 6;

/// The fixed head of the record: body, meters, heal, counters and the
/// blueprint mask. 52 → 60 at research v0, which took `SAVE_FORMAT` with
/// it (`store.rs`) — a record whose layout moves without the format
/// version is the one bug the header check exists to catch.
const SCALARS_BYTES: usize = 60;

/// Vertical sanity bound on a decoded body, in centimetres of `POS_Y_Q`
/// (±1000 m). Not a gameplay limit — the island's own heights are inside a
/// few hundred metres of zero — but a bound, because wall 4 says every
/// decoded field has one. A record past it is refused rather than clamped:
/// a body a kilometre under the seabed is a corrupt record, not a player.
/// Proposed default, DECISIONS.md §open ("player persistence v0").
pub const SAVE_Y_LIMIT_Q: i32 = 100_000;

/// Vertical velocity sanity bound, in centimetres/s of `VEL_Q` (±1000 m/s).
/// Same posture as [`SAVE_Y_LIMIT_Q`] — terminal fall speed is an order
/// under it, so this catches corruption and nothing a player can do.
/// Proposed default, DECISIONS.md §open ("player persistence v0").
pub const SAVE_VY_LIMIT_Q: i32 = 100_000;

/// Why a record was refused. Integer-shaped like every other refusal in
/// this crate (wall 3): the reason crosses to a server that can print, and
/// nothing here formats a string.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveError {
    /// A `bool` byte that was neither 0 nor 1.
    NotABool,
    /// The body is off the island, under it, or moving impossibly.
    BodyOutOfRange,
    /// `hp > hp_max` — a body healthier than its own ceiling.
    HpOverMax,
    /// A craft job names a recipe row past `MAX_RECIPES`, or asks for more
    /// units than one request may.
    BadCraftJob,
    /// The craft queue is not dense (an empty job before a live one). The
    /// head-at-0 invariant is what `craft::step` reads; a sparse queue
    /// would strand the tail forever.
    SparseCraftQueue,
    /// An inventory slot names an item past `MAX_ITEM_DEFS`, or is not in
    /// the canonical empty form (`count == 0` ⇔ `item == 0`).
    BadItemStack,
}

impl SaveError {
    /// A fixed sentence per reason. `&'static str`, never `String` — wall 3
    /// bans heap strings in this crate, and the server is what prints.
    pub fn reason(&self) -> &'static str {
        match self {
            Self::NotABool => "a boolean byte was neither 0 nor 1",
            Self::BodyOutOfRange => "the body is off the island or moving impossibly",
            Self::HpOverMax => "hp is over hp_max",
            Self::BadCraftJob => "a craft job names an impossible recipe or count",
            Self::SparseCraftQueue => "the craft queue has a gap before a live job",
            Self::BadItemStack => "an inventory slot names an impossible item or count",
        }
    }
}

/// One player's persistable state. `Copy` and POD like every other record
/// that crosses a boundary here, so it rides a `Command` and an SPSC ring
/// without an allocation.
///
/// ## What is deliberately NOT here, and why
///
/// - **`id`** — minted per connection (`generation << 8 | slot`). The key
///   is the store's, outside the sim.
/// - **`frame`** (the last input) — the client's, not the world's.
///   Restoring `seq` would lie to prediction about which input the sim last
///   executed, and restoring the look angle would fight the mouse the
///   player is already holding. `wake` leaves it alone for the same reason.
/// - **`next_swing`** and **`craft_done_at`** — absolute tick numbers, and
///   a restarted shard's clock starts at 0. A cooldown from a dead session
///   is not a smaller number, it is a meaningless one. The swing arm comes
///   back ready; the craft queue is re-armed against the *current* tick at
///   restore (`craft::rearm`), which is why `jobs` IS saved and its timer
///   is not.
/// - **`ws_cell` / `ws_hits`** — the weak-spot chase against one terrain
///   cell, which is a fact about an encounter you are no longer in. `die`
///   drops it too.
/// - **the death screen's four facts** — see [`PlayerSave::dead`]. A corpse
///   does not come back as a corpse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlayerSave {
    /// Where the body stood, quantized exactly as the sim and the wire
    /// carry it — the quantize-both-sides law applied to a disk: what is
    /// written back is what was transmitted, so a restored body is not one
    /// rounding away from where the client last drew it.
    pub body: Body,
    /// **This body was down when it was saved**, and the restore reads it
    /// as "wake at a beach", not "come back dead".
    ///
    /// The death screen is a *choice* (ALPHA.md §1: beach or a bag) and
    /// logging off is declining it. Two things follow, and both are why
    /// this is one bit rather than five saved fields. The screen cannot be
    /// restored: it is drawn from `EV_DEATH`, which is broadcast — the kill
    /// feed — so re-announcing it at a join would tell the whole shard
    /// somebody had just been killed who had just logged in. And the
    /// position must NOT be restored: a corpse's `body` is where it fell,
    /// so restoring it in place would stand the player up alive at their
    /// own death site with their sleeping bag still unspent, which is a
    /// better outcome than staying to click the button. So a dead save
    /// wakes on the ring at `deaths`, which is the beach it would have got
    /// for pressing "beach".
    pub dead: bool,
    /// The 6 hotbar + 24 backpack rows (ALPHA.md §1), canonical-empty.
    pub inv: [ItemStack; INV_SLOTS],
    /// What the body was wearing, indexed by slot (`combat::WEAR_HEAD`,
    /// `WEAR_BODY`). Canonical-empty like the inventory, and validated the
    /// same way — the item index is bounded and an empty stack is all
    /// zeroes, because `state_hash` reads these bytes and a record that
    /// said "0 of item 7" would hash differently from the same body the sim
    /// produced.
    ///
    /// **Not checked against the baked slot here, deliberately.** `persist`
    /// cannot see `CombatContent` and must not learn to: a record naming a
    /// body plate in the head slot is refused *at the read that matters* —
    /// `combat::worn_pct` ignores a stack whose baked row names another
    /// slot — so a forged record buys nothing a bound here would prevent.
    pub worn: [ItemStack; WEAR_SLOTS],
    /// The craft queue, dense with the head at 0. Saved because its inputs
    /// were already spent at enqueue: dropping it would eat a player's
    /// materials for closing the game, and refunding it would need a
    /// mutation on the leave path that the WAL never recorded.
    pub jobs: [CraftJob; CRAFT_QUEUE],
    /// Blueprints known: bit `i` = recipe `i` (`research.rs`). Saved for
    /// the reason research exists — a blueprint you paid a hoard of coin
    /// for and lost by closing the game would make the sink a punishment.
    /// **Not validated on read, deliberately**: every one of the 64 bits
    /// is a legal value, and a bit set past `MAX_RECIPES` names no recipe
    /// and can never be read (`research::knows` bounds the shift and
    /// `craft::enqueue` only ever asks about a live index), so there is
    /// nothing a forged mask can do that refusing it would prevent.
    pub known: u64,
    /// Hit points and the ceiling a heal clamps to. You log off hurt, you
    /// log in hurt.
    pub hp: u16,
    pub hp_max: u16,
    /// Deaths so far — not just a counter: it walks the spawn ring forward,
    /// so a dead save's beach is the one that death had earned.
    pub deaths: u16,
    /// The survival clock, accumulators included. A restore that zeroed the
    /// remainders would hand back a fraction of a minute of food on every
    /// reconnect, which is a small exploit and an exact-arithmetic bug.
    pub food: u16,
    pub water: u16,
    pub food_acc: u32,
    pub water_acc: u32,
    pub hurt_acc: u32,
    /// The in-flight heal from a consumed row.
    pub heal_rem: u16,
    pub heal_total: u16,
    pub heal_span: u32,
    pub heal_acc: u32,
}

impl Default for PlayerSave {
    fn default() -> Self {
        Self::EMPTY
    }
}

impl PlayerSave {
    /// A zeroed record. Not a playable character — `hp_max == 0` — and
    /// present because a fixed-capacity store needs something to hold in a
    /// slot nobody occupies. Every field is the zero of its type on purpose,
    /// `grounded` included: an unwritten record on disk is all zeros, and it
    /// must decode to exactly this or a fresh file would refuse itself.
    pub const EMPTY: Self = Self {
        body: Body {
            qx: 0,
            qy: 0,
            qz: 0,
            qvy: 0,
            grounded: false,
        },
        dead: false,
        inv: [ItemStack {
            item: 0,
            count: 0,
            cond: 0,
        }; INV_SLOTS],
        worn: [ItemStack {
            item: 0,
            count: 0,
            cond: 0,
        }; WEAR_SLOTS],
        jobs: [CraftJob {
            recipe: 0,
            remaining: 0,
        }; CRAFT_QUEUE],
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
        known: 0,
    };

    /// The record for a player as they stand. A pure read — the server
    /// takes one at a leave, at a shutdown, and on the autosave sweep, and
    /// none of those may mutate the world (a mutation the WAL never saw is
    /// wall 5's whole subject).
    pub fn of(p: &Player) -> Self {
        Self {
            body: p.body,
            dead: p.dead,
            inv: p.inv,
            worn: p.worn,
            jobs: p.jobs,
            known: p.known,
            hp: p.hp,
            hp_max: p.hp_max,
            deaths: p.deaths,
            food: p.food,
            water: p.water,
            food_acc: p.food_acc,
            water_acc: p.water_acc,
            hurt_acc: p.hurt_acc,
            heal_rem: p.heal_rem,
            heal_total: p.heal_total,
            heal_span: p.heal_span,
            heal_acc: p.heal_acc,
        }
    }

    /// Encode into the fixed record. Infallible by construction: every
    /// field is an integer of a declared width and the buffer is sized by
    /// [`PLAYER_SAVE_BYTES`].
    pub fn write_le(&self, out: &mut [u8; PLAYER_SAVE_BYTES]) {
        out[0..4].copy_from_slice(&self.body.qx.to_le_bytes());
        out[4..8].copy_from_slice(&self.body.qy.to_le_bytes());
        out[8..12].copy_from_slice(&self.body.qz.to_le_bytes());
        out[12..16].copy_from_slice(&self.body.qvy.to_le_bytes());
        out[16] = self.body.grounded as u8;
        out[17] = self.dead as u8;
        out[18..20].copy_from_slice(&self.hp.to_le_bytes());
        out[20..22].copy_from_slice(&self.hp_max.to_le_bytes());
        out[22..24].copy_from_slice(&self.deaths.to_le_bytes());
        out[24..26].copy_from_slice(&self.food.to_le_bytes());
        out[26..28].copy_from_slice(&self.water.to_le_bytes());
        out[28..32].copy_from_slice(&self.food_acc.to_le_bytes());
        out[32..36].copy_from_slice(&self.water_acc.to_le_bytes());
        out[36..40].copy_from_slice(&self.hurt_acc.to_le_bytes());
        out[40..42].copy_from_slice(&self.heal_rem.to_le_bytes());
        out[42..44].copy_from_slice(&self.heal_total.to_le_bytes());
        out[44..48].copy_from_slice(&self.heal_span.to_le_bytes());
        out[48..52].copy_from_slice(&self.heal_acc.to_le_bytes());
        out[52..60].copy_from_slice(&self.known.to_le_bytes());
        let mut at = SCALARS_BYTES;
        for j in self.jobs.iter() {
            out[at..at + 2].copy_from_slice(&j.recipe.to_le_bytes());
            out[at + 2..at + 4].copy_from_slice(&j.remaining.to_le_bytes());
            at += 4;
        }
        for s in self.inv.iter().chain(self.worn.iter()) {
            out[at..at + 2].copy_from_slice(&s.item.to_le_bytes());
            out[at + 2..at + 4].copy_from_slice(&s.count.to_le_bytes());
            out[at + 4..at + 6].copy_from_slice(&s.cond.to_le_bytes());
            at += 6;
        }
        debug_assert_eq!(at, PLAYER_SAVE_BYTES);
    }

    /// Decode and validate. See the module header: this is a wire decoder's
    /// job, on the one input path into `World` that no `Command` covers.
    pub fn read_le(src: &[u8; PLAYER_SAVE_BYTES]) -> Result<Self, SaveError> {
        let i32_at = |o: usize| i32::from_le_bytes([src[o], src[o + 1], src[o + 2], src[o + 3]]);
        let u32_at = |o: usize| u32::from_le_bytes([src[o], src[o + 1], src[o + 2], src[o + 3]]);
        let u16_at = |o: usize| u16::from_le_bytes([src[o], src[o + 1]]);
        let bool_at = |o: usize| match src[o] {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(SaveError::NotABool),
        };

        let body = Body {
            qx: i32_at(0),
            qy: i32_at(4),
            qz: i32_at(8),
            qvy: i32_at(12),
            grounded: bool_at(16)?,
        };
        // The island is the bound, taken through the same quantizer the
        // body was written with rather than a second constant that could
        // disagree with it.
        let xz_max = movement::quant_xz(ISLAND_SIZE);
        let in_xz = |q: i32| (0..=xz_max).contains(&q);
        if !in_xz(body.qx)
            || !in_xz(body.qz)
            || !(-SAVE_Y_LIMIT_Q..=SAVE_Y_LIMIT_Q).contains(&body.qy)
            || !(-SAVE_VY_LIMIT_Q..=SAVE_VY_LIMIT_Q).contains(&body.qvy)
        {
            return Err(SaveError::BodyOutOfRange);
        }

        let dead = bool_at(17)?;
        let hp = u16_at(18);
        let hp_max = u16_at(20);
        if hp > hp_max {
            return Err(SaveError::HpOverMax);
        }

        let mut jobs = [CraftJob::default(); CRAFT_QUEUE];
        let mut at = SCALARS_BYTES;
        for j in jobs.iter_mut() {
            *j = CraftJob {
                recipe: u16_at(at),
                remaining: u16_at(at + 2),
            };
            if j.recipe as usize >= MAX_RECIPES || j.remaining > CRAFT_COUNT_MAX {
                return Err(SaveError::BadCraftJob);
            }
            at += 4;
        }
        // Dense, head at 0: once a job is empty every later one must be.
        let live = jobs
            .iter()
            .position(|j| j.remaining == 0)
            .unwrap_or(CRAFT_QUEUE);
        if jobs[live..].iter().any(|j| j.remaining > 0) {
            return Err(SaveError::SparseCraftQueue);
        }

        let mut inv = [ItemStack::default(); INV_SLOTS];
        let mut worn = [ItemStack::default(); WEAR_SLOTS];
        // One loop over both arrays: a worn slot is an inventory slot in
        // every respect the decoder cares about — same six bytes, same
        // canonical-empty rule, same bound — and giving it a second loop
        // with the same three checks would be two places for the rule to
        // drift apart.
        for s in inv.iter_mut().chain(worn.iter_mut()) {
            *s = ItemStack {
                item: u16_at(at),
                count: u16_at(at + 2),
                cond: u16_at(at + 4),
            };
            // Canonical empty: an emptied slot zeroes ALL THREE fields,
            // which is what the state hash reads. A record that said "0 of
            // item 7" would hash differently from the same inventory the
            // sim produced, and a difference nothing can see is wall 5's
            // failure mode — and since durability v0 the SAME sentence
            // holds for condition: a slot emptied by a path that forgot to
            // zero `cond` hashes differently from the sim's own empty, so
            // `count == 0 && cond != 0` is refused exactly as
            // `count == 0 && item != 0` always was.
            if s.item as usize >= MAX_ITEM_DEFS
                || (s.count == 0 && s.item != 0)
                || (s.count == 0 && s.cond != 0)
            {
                return Err(SaveError::BadItemStack);
            }
            at += 6;
        }
        debug_assert_eq!(at, PLAYER_SAVE_BYTES);

        Ok(Self {
            body,
            dead,
            inv,
            worn,
            jobs,
            hp,
            hp_max,
            deaths: u16_at(22),
            food: u16_at(24),
            water: u16_at(26),
            food_acc: u32_at(28),
            water_acc: u32_at(32),
            hurt_acc: u32_at(36),
            heal_rem: u16_at(40),
            heal_total: u16_at(42),
            heal_span: u32_at(44),
            heal_acc: u32_at(48),
            known: u64::from_le_bytes(src[52..60].try_into().expect("8 bytes at 52")),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A record with every field distinct, so a transposition in either
    /// direction of the codec shows up as a wrong value rather than a
    /// coincidence. The positional-payload trap: a byte-golden cannot see
    /// that two fields swapped places if both hold the same number.
    fn fixture() -> PlayerSave {
        let mut s = PlayerSave {
            body: Body {
                qx: 34_133,
                qy: 4_211,
                qz: 12_345,
                qvy: -777,
                grounded: false,
            },
            dead: false,
            inv: [ItemStack::default(); INV_SLOTS],
            worn: [ItemStack::default(); WEAR_SLOTS],
            jobs: [CraftJob::default(); CRAFT_QUEUE],
            // Distinct in every byte, like everything else here: a mask of
            // all-ones or all-zeros could not catch a codec that wrote the
            // halves in the wrong order.
            known: 0x0123_4567_89AB_CDEF,
            hp: 71,
            hp_max: 100,
            deaths: 3,
            food: 411,
            water: 322,
            food_acc: 1_234_567,
            water_acc: 7_654_321,
            hurt_acc: 99,
            heal_rem: 12,
            heal_total: 40,
            heal_span: 300,
            heal_acc: 17,
        };
        s.jobs[0] = CraftJob {
            recipe: 2,
            remaining: 5,
        };
        s.jobs[1] = CraftJob {
            recipe: 1,
            remaining: 9,
        };
        for (i, slot) in s.inv.iter_mut().enumerate() {
            if i % 3 == 0 {
                *slot = ItemStack {
                    item: (i % MAX_ITEM_DEFS) as u16,
                    count: (i as u16 + 1) * 7,
                    cond: 0,
                };
            }
        }
        // Slot 0 would be "0 of item 0" under the rule above; give it a
        // real stack so the fixture is a legal record — a WORN one since
        // format 3, so the round trip and the byte golden both pin the
        // condition field in a nonzero state (gate 7: a saved tool comes
        // back worn).
        s.inv[0] = ItemStack {
            item: 5,
            count: 42,
            cond: 0x2233,
        };
        // The worn slots, distinct from each other and from every
        // inventory stack, so the codec cannot pass by writing the
        // inventory twice or by transposing head and body. They sit at the
        // very end of the record, which is exactly where a stride mistake
        // stops being caught by the `debug_assert_eq!(at, …)`.
        s.worn[0] = ItemStack {
            item: 9,
            count: 1,
            cond: 0x4455,
        };
        s.worn[1] = ItemStack {
            item: 11,
            count: 1,
            cond: 0x6677,
        };
        s
    }

    #[test]
    fn a_record_round_trips() {
        let want = fixture();
        let mut buf = [0u8; PLAYER_SAVE_BYTES];
        want.write_le(&mut buf);
        assert_eq!(PlayerSave::read_le(&buf), Ok(want));
    }

    /// The format's own size, asserted rather than trusted. It is derived
    /// from `INV_SLOTS`, `CRAFT_QUEUE` and `WEAR_SLOTS`, so widening any of
    /// them moves it — and that is a format change, which is the sentence
    /// this test is here to make somebody read (`store.rs` `SAVE_FORMAT`).
    /// 256 → 268 at `SAVE_FORMAT` 4: two worn stacks (armor v0).
    #[test]
    fn the_record_is_the_size_the_format_declares() {
        assert_eq!(PLAYER_SAVE_BYTES, 268, "the on-disk record size moved");
        assert_eq!(INV_SLOTS, 30);
        assert_eq!(CRAFT_QUEUE, 4);
        assert_eq!(WEAR_SLOTS, 2);
    }

    /// A byte golden over the fixture. Not decoration: `write_le` is 20
    /// hand-written offsets, and the trap list's own entry says a positional
    /// payload is where the reference ecosystem actually bled — the right
    /// value written to the wrong offset. A round-trip test cannot see it
    /// (both sides move together); this can.
    #[test]
    fn the_encoding_is_pinned_byte_for_byte() {
        let mut buf = [0u8; PLAYER_SAVE_BYTES];
        fixture().write_le(&mut buf);
        // The scalar head, field by field, read back with an independent
        // decoder — deliberately not `read_le`, which would only prove the
        // two halves agree with each other.
        assert_eq!(&buf[0..4], &34_133i32.to_le_bytes(), "qx at 0");
        assert_eq!(&buf[4..8], &4_211i32.to_le_bytes(), "qy at 4");
        assert_eq!(&buf[8..12], &12_345i32.to_le_bytes(), "qz at 8");
        assert_eq!(&buf[12..16], &(-777i32).to_le_bytes(), "qvy at 12");
        assert_eq!(buf[16], 0, "grounded at 16");
        assert_eq!(buf[17], 0, "dead at 17");
        assert_eq!(&buf[18..20], &71u16.to_le_bytes(), "hp at 18");
        assert_eq!(&buf[20..22], &100u16.to_le_bytes(), "hp_max at 20");
        assert_eq!(&buf[22..24], &3u16.to_le_bytes(), "deaths at 22");
        assert_eq!(&buf[24..26], &411u16.to_le_bytes(), "food at 24");
        assert_eq!(&buf[26..28], &322u16.to_le_bytes(), "water at 26");
        assert_eq!(&buf[28..32], &1_234_567u32.to_le_bytes(), "food_acc at 28");
        assert_eq!(&buf[32..36], &7_654_321u32.to_le_bytes(), "water_acc at 32");
        assert_eq!(&buf[36..40], &99u32.to_le_bytes(), "hurt_acc at 36");
        assert_eq!(&buf[40..42], &12u16.to_le_bytes(), "heal_rem at 40");
        assert_eq!(&buf[42..44], &40u16.to_le_bytes(), "heal_total at 42");
        assert_eq!(&buf[44..48], &300u32.to_le_bytes(), "heal_span at 44");
        assert_eq!(&buf[48..52], &17u32.to_le_bytes(), "heal_acc at 48");
        // The blueprint mask closes the head (research v0), which is what
        // moved both arrays eight bytes along — the shift this whole block
        // exists to make loud.
        assert_eq!(
            &buf[52..60],
            &0x0123_4567_89AB_CDEFu64.to_le_bytes(),
            "known at 52"
        );
        // The two arrays start where the head ends, in declaration order.
        assert_eq!(&buf[60..62], &2u16.to_le_bytes(), "jobs[0].recipe at 60");
        assert_eq!(&buf[62..64], &5u16.to_le_bytes(), "jobs[0].remaining at 62");
        assert_eq!(&buf[76..78], &5u16.to_le_bytes(), "inv[0].item at 76");
        assert_eq!(&buf[78..80], &42u16.to_le_bytes(), "inv[0].count at 78");
        assert_eq!(
            &buf[80..82],
            &0x2233u16.to_le_bytes(),
            "inv[0].cond at 80 — the slot stride is 6 since format 3"
        );
        // The worn slots close the record (format 4), and they are pinned
        // here rather than left to the round trip for this block's stated
        // reason twice over: they are the LAST thing written, so a stride
        // mistake anywhere earlier lands on them, and `write_le`/`read_le`
        // walk them with one shared `.chain()` — which means the two halves
        // agree about a wrong offset by construction and only an
        // independent decoder can say so.
        assert_eq!(&buf[256..258], &9u16.to_le_bytes(), "worn[0].item at 256");
        assert_eq!(&buf[258..260], &1u16.to_le_bytes(), "worn[0].count at 258");
        assert_eq!(
            &buf[260..262],
            &0x4455u16.to_le_bytes(),
            "worn[0].cond at 260"
        );
        assert_eq!(&buf[262..264], &11u16.to_le_bytes(), "worn[1].item at 262");
        assert_eq!(
            &buf[266..268],
            &0x6677u16.to_le_bytes(),
            "worn[1].cond at 266 — and 268 is the whole record"
        );
    }

    /// Every refusal, one per reason, built by hand off a legal record —
    /// no encoder can produce these, which is exactly why the decoder has
    /// to refuse them rather than trusting its own writer.
    #[test]
    fn a_corrupt_record_is_refused_by_reason() {
        let mut base = [0u8; PLAYER_SAVE_BYTES];
        fixture().write_le(&mut base);
        assert!(PlayerSave::read_le(&base).is_ok(), "the base must be legal");

        let bent = |f: &dyn Fn(&mut [u8; PLAYER_SAVE_BYTES])| {
            let mut b = base;
            f(&mut b);
            PlayerSave::read_le(&b)
        };

        assert_eq!(bent(&|b| b[16] = 2), Err(SaveError::NotABool));
        assert_eq!(bent(&|b| b[17] = 0xff), Err(SaveError::NotABool));
        // Off the east edge of the island, and under it.
        let past_edge = movement::quant_xz(ISLAND_SIZE) + 1;
        assert_eq!(
            bent(&|b| b[0..4].copy_from_slice(&past_edge.to_le_bytes())),
            Err(SaveError::BodyOutOfRange)
        );
        assert_eq!(
            bent(&|b| b[8..12].copy_from_slice(&(-1i32).to_le_bytes())),
            Err(SaveError::BodyOutOfRange)
        );
        assert_eq!(
            bent(&|b| b[4..8].copy_from_slice(&(SAVE_Y_LIMIT_Q + 1).to_le_bytes())),
            Err(SaveError::BodyOutOfRange)
        );
        assert_eq!(
            bent(&|b| b[12..16].copy_from_slice(&(-SAVE_VY_LIMIT_Q - 1).to_le_bytes())),
            Err(SaveError::BodyOutOfRange)
        );
        // Healthier than its own ceiling.
        assert_eq!(
            bent(&|b| b[18..20].copy_from_slice(&u16::MAX.to_le_bytes())),
            Err(SaveError::HpOverMax)
        );
        // The one that would panic the sim thread: a recipe row that does
        // not exist, indexed unchecked by `craft::step`.
        assert_eq!(
            bent(&|b| b[60..62].copy_from_slice(&(MAX_RECIPES as u16).to_le_bytes())),
            Err(SaveError::BadCraftJob)
        );
        assert_eq!(
            bent(&|b| b[62..64].copy_from_slice(&(CRAFT_COUNT_MAX + 1).to_le_bytes())),
            Err(SaveError::BadCraftJob)
        );
        // A gap before a live job: job 0 emptied, jobs 1 still live.
        assert_eq!(
            bent(&|b| b[62..64].copy_from_slice(&0u16.to_le_bytes())),
            Err(SaveError::SparseCraftQueue)
        );
        // An item past the table, and a non-canonical empty.
        assert_eq!(
            bent(&|b| b[76..78].copy_from_slice(&(MAX_ITEM_DEFS as u16).to_le_bytes())),
            Err(SaveError::BadItemStack)
        );
        assert_eq!(
            bent(&|b| b[78..80].copy_from_slice(&0u16.to_le_bytes())),
            Err(SaveError::BadItemStack)
        );
        // The cond half of canonical empty (format 3): inv[1] is a zeroed
        // slot in the fixture, at offset 82; condition on an empty slot is
        // state nothing can see and is refused exactly as "0 of item 7"
        // always was. This is the check nobody would think of — a slot
        // emptied by a path that forgot to zero `cond` hashes differently
        // from the sim's own empty (wall 5).
        assert_eq!(
            bent(&|b| b[86..88].copy_from_slice(&7u16.to_le_bytes())),
            Err(SaveError::BadItemStack),
            "an empty slot may not carry condition"
        );
    }

    /// A full queue is dense and must pass — the density check has to
    /// accept the legal maximum, or it would refuse a player mid-craft.
    #[test]
    fn a_full_craft_queue_is_dense_enough() {
        let mut s = fixture();
        s.jobs = [CraftJob {
            recipe: 1,
            remaining: 2,
        }; CRAFT_QUEUE];
        let mut buf = [0u8; PLAYER_SAVE_BYTES];
        s.write_le(&mut buf);
        assert_eq!(PlayerSave::read_le(&buf), Ok(s));
    }

    /// `EMPTY` is a legal record. The store holds one in every unoccupied
    /// slot and decodes what it reads back, so a zeroed record must survive
    /// the validator — otherwise a fresh file would refuse itself.
    #[test]
    fn the_empty_record_is_legal() {
        let mut buf = [0u8; PLAYER_SAVE_BYTES];
        PlayerSave::EMPTY.write_le(&mut buf);
        assert_eq!(PlayerSave::read_le(&buf), Ok(PlayerSave::EMPTY));
        assert_eq!(buf, [0u8; PLAYER_SAVE_BYTES], "EMPTY must encode to zeros");
    }
}
