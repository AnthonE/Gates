//! The world, as bytes — **what a restart does not delete.**
//!
//! `persist.rs` remembers a player. This remembers the place they were
//! standing in: the bases, the boxes and their contents, the bags on the
//! ground, the fuses burning, which trees are stumps and how long until
//! they are trees again — and, since sleepers, the bodies themselves.
//!
//! ## Why this had a deadline the day sleepers landed
//!
//! A sleeper is world state. Before it, a restart cost the world's
//! furniture and every player still came back through their own record, so
//! "the world is not persisted" was a missing feature. After it, a restart
//! **deletes every body on the shard**, and the raid somebody landed on one
//! is undone by the store record that outlived it. `NOW.md` §0y item 1 made
//! item 2 urgent by being finished.
//!
//! ## The load is construction, and that is how wall 5 survives it
//!
//! A player restore rides `Command::JoinAs` because it happens *during* a
//! run: the WAL is the command stream, so a mutation that arrived through a
//! side channel would replay as something else. A world load cannot work
//! that way — the state is a quarter of a megabyte, not a 188-byte record —
//! and it does not have to, because **it happens before tick 0**. Nothing
//! has been replayed yet. The loaded world is not a mutation of a run, it
//! is the *origin* of one.
//!
//! So wall 5's sentence widens by exactly one word and stays checkable:
//! same build + same **origin** + same command stream → same state hashes.
//! The origin is nameable — [`World::state_hash`] of the world the moment
//! the load returns — which is why [`World::load`] is boot-only and says so,
//! and why a WAL header (when there is one) pins that number beside the seed
//! and the content hash. `reference/SAVES.md` §8 records that the reference
//! game is no help here: it has no replay, so a loaded world only has to be
//! *legal* for them. Ours has to be legal **and** repeatable from.
//!
//! ## A save file is untrusted input to the sim
//!
//! Same rule as `persist.rs`, one level up and with more surface: this is
//! the only non-command path into `World`, so [`decode_into`] validates like
//! a wire decoder rather than trusting its own writer. Every count is capped
//! before it is looped on, every row index that reaches a content table is
//! range-checked, every address is bounded to the island, and the reader
//! cannot read past the end of the buffer. A record that fails is refused
//! **whole and by reason** — never clamped, never partially applied. The
//! reference game's loader takes the file's word for it (§9.5); ours must
//! not, because `craft::step` and `build`'s shape lookup both index content
//! tables with no bound of their own and a hand-edited row would panic the
//! sim thread.
//!
//! ## What is deliberately not here
//!
//! - **Arrows in flight.** Sub-second state whose whole meaning is a
//!   trajectory between two ticks; restoring one would land it at a time
//!   nobody shot for. They are dropped, which is the same thing that
//!   happens to them at the end of their life anyway.
//! - **`Pieces::cols`**, the collision index. Derived, never hashed, and
//!   rebuilt from the pieces at load — storing it would be storing a cache
//!   that could disagree with the records it came from.
//! - **Identity.** No `PlayerKey` appears in this file, exactly as none
//!   appears in `persist.rs`. The bodies carry the world ids they had, and
//!   the server keeps the key→body map in its own section of its own file
//!   (`server/src/store.rs`) — because an id means nothing across a restart
//!   and a key may not enter this crate.
//! - **Content.** The baked tables are boot input, pinned by the content
//!   hash in the server's header, not copied in here. A save replayed under
//!   different content is refused at the door, never silently reinterpreted.
//!
//! Pure like everything else in this crate: no I/O, no clock, no
//! allocation. Bytes in, bytes out; the file they live in is the server's.

use crate::backpack::BackpackRec;
use crate::build::PieceRec;
use crate::charge::ChargeRec;
use crate::deploy::{BoxRec, DeployRec, HearthRec};
use crate::gather::{ItemStack, SlotLife, NO_ITEM};
use crate::input::InputFrame;
use crate::limits::HOTBAR_SLOTS;
use crate::limits::{
    BOX_SLOTS, HEARTH_CREW_CAP, HEARTH_STOCK_ROWS, INV_SLOTS, LOCK_AUTH_CAP, LOCK_GUEST_CAP,
    MAX_BACKPACKS, MAX_BOXES, MAX_BUILD_COORD, MAX_BUILD_LEVELS, MAX_DEPLOYS, MAX_HEARTHS,
    MAX_LIVE_CHARGES, MAX_LOCKS, MAX_MAGS, MAX_PIECES, MAX_PLAYERS, MAX_SLOT_LIVES,
    MAX_SPENT_ARROWS, MAX_WORLD_CONTS,
};
use crate::lock::{LockRec, CODE_MAX, CODE_NONE};
use crate::loot::{LOOT_CACHE, LOOT_CRATE};
use crate::movement;
use crate::oven::OvenState;
use crate::persist::{PlayerSave, SaveError, PLAYER_SAVE_BYTES};
use crate::spent::SpentRec;
use crate::terrain::{self, ISLAND_SIZE};
use crate::world::{Player, World};
use crate::worldcont::WorldContRec;

/// The blob's own format version, and it moves for the same reason
/// `SAVE_FORMAT` and `PROTO_VER` do: the layout below **is** the format, and
/// nothing in it is self-describing. A file whose version does not match is
/// refused at boot rather than reinterpreted (`server/src/store.rs` checks
/// it), so a field added here without turning this number is a world
/// silently decoded as a different world.
///
/// **3 skips no shape — it resolves a collision.** Two lanes were open at
/// once and each bumped 1→2 for its own layout (the oven state inline on a
/// container; the lock section, the crew tail and the two placement ticks).
/// Merging them makes a third layout that is neither, and a version number
/// that two different files can both claim is worse than no version number
/// at all, so the merge takes the next free one.
///
/// **4 — a charge carries its blast** (satchel blast v0): two `u16`s,
/// `damage` and `blast_cm`, between `structure` and `fires_at`, copied at
/// plant time like the field they follow. The same bump deletes a check
/// that would have refused any real save mid-fuse: the decoder compared
/// `structure` — a damage *amount* — against the content table's row
/// *count*, so a satchel's 125 against a dozen rows was "an impossible
/// structure". No live world ever hit it (a ten-second fuse rarely meets
/// a save), which is exactly why it survived to be found by reading.
///
/// **6 — a piece carries its facing** (hard/soft v0): one byte between
/// `row` and `hp`, the soft side's direction, validated ≤ 1. It is in
/// `state_hash` — a swing's price reads it — so a save without it would
/// resume a shard whose walls forgot which way they were built.
///
/// **7 — every item stack carries its condition** (item durability v0):
/// the shared `stack` writer/reader grows two bytes, so every player
/// inventory, bag, box and world container widens together, and the
/// canonical-empty rule widens with them — `count == 0 && cond != 0` is
/// refused beside `count == 0 && item != 0`, or a slot emptied by a path
/// that forgot to zero `cond` would hash differently from the sim's own
/// empty (wall 5's failure mode, named where the old rule was).
///
/// **8 — a body carries what it is wearing** (armor v0): `PlayerSave`
/// grew `worn`, `WEAR_SLOTS` stacks at the same six-byte stride, so a
/// player record goes 308 → 320 and the whole player section with it. It
/// is in `state_hash` for the same reason `facing` is — a hit's price
/// reads it — so a world resumed without it would stand every body up
/// naked and change every fight in it.
///
/// **9 — a piece carries its plate** (build plate v1): one byte after `uh`,
/// the signed band offset `build::plate_for` latched when the piece went
/// down, validated against the two stilt limits. This is the first piece
/// field that is not derivable from (seed, cell) — a base's floor height is
/// now a CHOICE the first foundation made, so a save without it would
/// resume every stilted base flat on the terrain it was built over,
/// dropping bases into hillsides and leaving others in the air. It is in
/// `state_hash` for `facing`'s reason and more sharply: it decides where
/// every collision surface in the column is.
///
/// **10 — arrows that landed are in the file** (arrow recovery v0): a
/// tenth section count, a `u32` eviction counter in the head beside the
/// body one, and `MAX_SPENT_ARROWS` records of 22 bytes. It is the first
/// section whose *sibling is deliberately absent* — `Arrows` is still not
/// saved, and the module doc four screens up says why: a trajectory
/// between two ticks is meaningless a restart later, and an arrow lying
/// on a hillside is ammunition somebody earned. Both are in `state_hash`,
/// so the distinction is only about which one survives a save, and
/// getting it backwards either way is wall 5 failing at the origin.
///
/// **11 — a burning torch keeps its remainder** (torch fuel v0): no new
/// section and no new head field, but `PlayerSave`'s scalar head grew
/// `light_acc`, so a player record is 320 → 324 bytes and every section
/// after the players moved. That is exactly the silent reinterpretation
/// this number exists to refuse — an old file read as a new one would
/// take four bytes of the first player's craft queue as their torch and
/// slide every byte after it.
///
/// **12 — a body keeps its loaded rounds** (reload v1): `Player` grew
/// `mag` and `mag_round`, `MAX_MAGS` `u16` pairs written inline after
/// `next_swing`, so a player record grew and every section after the
/// players moved again. Saved rather than dropped because `state_hash`
/// folds the magazine: a snapshot without it restores a world that hashes
/// differently from the one that was written.
pub const WORLD_SAVE_FORMAT: u16 = 12;

/// Fixed head: format, tick, the three sweep cursors, the eviction counter,
/// the next bag id, and the ten section counts.
///
/// **Public for `PLAYER_BYTES`' reason, and it was made public the day that
/// reason came true a second time.** Two byte-poking tests in
/// `tests/worldsave.rs` seek past this head to reach the first piece and
/// the first deploy, and both spelled the length as `34 + 20` — correct
/// until format 5 added a ninth section count, at which point they poked
/// two bytes short and failed with "the deploy stride drifted" rather than
/// with anything about the head. A hand-copied offset is a silent
/// wrong-seek the day the layout grows; naming the constant is what makes
/// the next section free.
pub const HEAD_BYTES: usize = 2 + 8 + 4 * 3 + 8 + 4 + 4 + SECTION_COUNTS;
/// Ten `u16` counts and one `u32` (`slot_lives`, whose cap is 16 384 and
/// so does not fit a `u16` with room to be over-cap and *refused* rather
/// than wrapping — the count has to be able to say an illegal number).
/// The ninth is `world_conts` (format 5), the tenth `spent` (format 10).
/// **Public for `HEAD_BYTES`' reason, and it was made public the day that
/// reason came true a third time.** `tests/worldsave.rs` spelled the offset
/// of the first section count as a hand-copied `34`, which was right until
/// format 10 put a `u32` in the head ahead of the counts — at which point
/// the byte-poke landed in the eviction counter and the test failed with
/// "a player count past MAX_PLAYERS was accepted" rather than with anything
/// about the head. The offset is `HEAD_BYTES - SECTION_COUNTS` now.
pub const SECTION_COUNTS: usize = 10 * 2 + 4;

/// One body: everything `PlayerSave` already validates, plus every
/// remaining field `World::state_hash` reads off a player.
///
/// **The tail is not optional and the reason is a gate.** `PlayerSave` is
/// the *player's* record — what a person carries between shards — and it
/// deliberately drops the input frame, the swing arm, the weak-spot chase
/// and the craft timer, because those belong to a session
/// (`persist.rs` argues each one). A world save is a different question: it
/// is the *world's* record, and every one of those fields is hashed, so a
/// blob that dropped them would load to a world whose `state_hash` differs
/// from the one that was saved — which is wall 5 failing at the origin, and
/// the round-trip test in `tests/worldsave.rs` is what caught it.
///
/// The tail: id, `slept_at`, the seven input-frame fields, `next_swing`,
/// **the magazine** (format 12: `MAX_MAGS` pairs of `u16`, the loaded count
/// and the round in it), the weak-spot pair, the four death-screen facts,
/// and `craft_done_at`.
const PLAYER_TAIL_BYTES: usize = 4 + 8 + 9 + 8 + MAX_MAGS * 4 + 6 + 9 + 8;
/// On-disk stride of one saved body. Public because two byte-poking
/// tests in `tests/worldsave.rs` have to seek past the player section, and
/// a hand-copied 240 there is a silent wrong-offset the day `PlayerSave`
/// grows — which is exactly what happened at research v0.
pub const PLAYER_BYTES: usize = PLAYER_SAVE_BYTES + PLAYER_TAIL_BYTES;
/// A piece record plus its placement tick, which lives in a parallel
/// array in the store (`build.rs` says why it is not on the record) and
/// is written inline here for `DEPLOY_BYTES`' reason — a file has no
/// parallel arrays worth having.
///
/// ⚠ **This was 11 + 8 from format 6 until 2026-08-21, and the encoder wrote
/// 12 + 8 the whole time.** `facing` joined the record and this constant did
/// not move with it, so [`WORLD_SAVE_MAX_BYTES`] under-counted by one byte per
/// piece — 8 KiB at the cap — and a shard holding `MAX_PIECES` would have
/// failed to save, with `save_world` returning "too small" for a buffer
/// sized by this crate's own published ceiling. Nothing caught it because the
/// only two checks on the number were a `by_hand` sum and a pin, and both were
/// re-derived from this same wrong constant; the byte-poking test in
/// `tests/worldsave.rs` had the true stride typed as a literal 20 beside a
/// comment saying 12 + 8, which is the disagreement that finally surfaced it.
/// It is `pub` now, that test reads it instead of a literal, and
/// `the_piece_stride_is_what_the_encoder_writes` measures it against two real
/// encodes — so the next field to join gets a red test rather than a bigger
/// silent gap.
///
/// 12 → 13 at format 9: the plate (build plate v1).
pub const PIECE_BYTES: usize = 13 + 8;
/// A deploy record plus its `bag_ready` cooldown, which lives in a parallel
/// array in the store (`deploy.rs` says why it is not on the record) and is
/// written inline here because a file has no parallel arrays worth having.
const DEPLOY_BYTES: usize = 17 + 8 + 8;
/// A hearth: address, owner, the stock rows, and the crew — its count
/// and the whole backing array, for `LOCK_BYTES`' reason (every byte of a
/// roster is hashed, tail included, so a save that dropped the tail would
/// reload to a different hash).
const HEARTH_BYTES: usize = 9 + HEARTH_STOCK_ROWS * 4 + 1 + HEARTH_CREW_CAP * 4;
/// A container record plus its oven state, which lives in a parallel
/// array for `bag_ready`'s reason and is written inline here for
/// `DEPLOY_BYTES`'s: a file has no parallel arrays worth having. Every
/// container carries the state — a storage box's says `ARCH_BOX` and is
/// six zeroed bytes plus twelve zeroed counters, which is the price of
/// the two stores staying one store (`deploy::holds_items`).
const BOX_BYTES: usize = 9 + BOX_SLOTS * 6 + 6 + BOX_SLOTS * 2;
/// One code lock (lock v1). Address, owner, both codes, the locked bit,
/// both remembered lists with their counts, and the three brute-force
/// counters — the whole `LockRec`, because every field of it is hashed
/// and a save that dropped one would load to a different `state_hash`
/// than it was taken from (the `PLAYER_TAIL_BYTES` argument, one store
/// over).
const LOCK_BYTES: usize =
    6 + 4 + 2 + 2 + 1 + 1 + 1 + LOCK_AUTH_CAP * 4 + LOCK_GUEST_CAP * 4 + 1 + 8 + 8;
const BACKPACK_BYTES: usize = 28 + INV_SLOTS * 6;
/// One authored world container (format 5, `worldcont.rs`): the cell, the
/// quantized stand position, the table it rolls, the refill deadline, and
/// its slots. The position is saved rather than re-derived from
/// `terrain::scatter` on load for the reason the record stores it at all —
/// a load must not cost 60 `noise2` evaluations per container — and the
/// decoder re-checks it against the cell it claims, so a hand-edited save
/// cannot move a crate to the player's feet.
const WORLD_CONT_BYTES: usize = 2 + 2 + 4 + 4 + 1 + 8 + INV_SLOTS * 6;
/// One spent arrow (format 10, `spent.rs`): three millimetre coordinates,
/// the round it is, and the tick it becomes takeable. Millimetres and not
/// the body's coarser quanta because the reach test `spent::pickup` runs
/// measures against this number, and a 3 cm floor on where an arrow lies
/// is a 3 cm floor on how precisely you can reach for it.
const SPENT_BYTES: usize = 4 + 4 + 4 + 2 + 8;
/// A burning fuse: address + store bit + the three copied-at-plant
/// numbers (structure, damage, blast — format 4) + deadline + planter.
const CHARGE_BYTES: usize = 25;
const SLOT_LIFE_BYTES: usize = 14;

/// The largest blob this world can produce — every store at capacity.
///
/// Derived from the caps rather than stated, so widening any of them moves
/// this number and takes the buffer the server preallocates with it. It is
/// also the bound that makes the encode a *bounded* piece of work in the
/// sense wall 4 means: the sim thread's cost is O(live records) with a
/// ceiling nothing can exceed, not O(whatever the world grew to).
pub const WORLD_SAVE_MAX_BYTES: usize = HEAD_BYTES
    + MAX_PLAYERS * PLAYER_BYTES
    + MAX_PIECES * PIECE_BYTES
    + MAX_DEPLOYS * DEPLOY_BYTES
    + MAX_HEARTHS * HEARTH_BYTES
    + MAX_BOXES * BOX_BYTES
    + MAX_LOCKS * LOCK_BYTES
    + MAX_BACKPACKS * BACKPACK_BYTES
    + MAX_WORLD_CONTS * WORLD_CONT_BYTES
    + MAX_LIVE_CHARGES * CHARGE_BYTES
    + MAX_SPENT_ARROWS * SPENT_BYTES
    + MAX_SLOT_LIVES * SLOT_LIFE_BYTES;

/// Why a world was refused. Integer-shaped like every refusal in this crate
/// (wall 3) — the reason crosses to a server that can print, and nothing
/// here formats a string.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorldSaveError {
    /// The blob is shorter than the fields it claims to hold. The one error
    /// every other check depends on: a reader that could run off the end
    /// would turn a truncated file into a panic on the boot path.
    Truncated,
    /// Not this format version.
    Format(u16),
    /// A section count is past the cap the store can hold. Checked *before*
    /// the loop that would allocate against it, which is the difference
    /// between a refusal and a very long boot.
    CountOverCap,
    /// A `bool` byte that was neither 0 nor 1.
    NotABool,
    /// A player record failed `PlayerSave`'s own validator. Carries its
    /// reason rather than flattening it — the caller prints one sentence
    /// and both halves earned their own.
    Player(SaveError),
    /// Two bodies claim the same id, so `slot_of` would answer one of them
    /// arbitrarily and a takeover would seat somebody in the wrong body.
    DuplicatePlayerId,
    /// A structure's grid address is off the island.
    AddressOutOfRange,
    /// A piece or deployable names a content row that does not exist. The
    /// one that would panic the sim: `bc.pieces[row].shape` is indexed
    /// unchecked at every rebuild and every collapse.
    BadContentRow,
    /// Two pieces in one build column claim different plates (build plate
    /// v1). The sim cannot produce it — `build::place` adopts the column's
    /// plate before it inserts — so a file that holds it was edited.
    ///
    /// **Refused rather than normalised, because the two readers disagree.**
    /// The renderer draws each piece at its OWN plate (the record carries
    /// it); every collision walk asks the COLUMN (`ColMasks::plate`, one
    /// value, last write wins on rebuild). A column with two plates is
    /// therefore a base you can see standing where you cannot walk — the
    /// exact drawn-vs-collided split the plate exists to close, arriving
    /// through the one door that is not a command.
    PieceColumnPlateSplit,
    /// An item stack names an item past the table, or is not in the
    /// canonical empty form (`count == 0` ⇔ `item == 0`) — the same rule
    /// `PlayerSave` enforces, for the same state-hash reason.
    BadItemStack,
    /// A bag id is 0 (which means "no bag"), duplicated, or at/above the
    /// store's own `next_id`, which would let the next death mint a
    /// colliding one.
    BadBackpackId,
    /// A charge names a structure kind or address that cannot hold one.
    BadCharge,
    /// A body's selected hotbar slot is past the hotbar.
    BadHotbarSlot,
    /// A code lock carries a code outside 0000..=9999 (and not
    /// `lock::CODE_NONE`). Refused rather than clamped: a clamped code is
    /// a door whose owner's own four digits no longer open it.
    BadCode,
    /// A world container names a loot table that is not a container's
    /// (`LOOT_CRATE` or `LOOT_CACHE`). Refused rather than coerced to a
    /// default: a crate silently rolling the barrel's table is the whole
    /// destination gradient quietly deleted, and it would look like
    /// nothing at all in a log.
    BadWorldContTable,
    /// Two world containers claim one cell. The move verb resolves by
    /// `index_of`, which takes the first — so the second is loot nothing
    /// can reach and a `state_hash` term nothing can explain. The
    /// duplicate-bag-id refusal, one store over.
    DuplicateWorldCont,
}

impl WorldSaveError {
    /// A fixed sentence per reason. `&'static str`, never `String`.
    pub fn reason(&self) -> &'static str {
        match self {
            Self::Truncated => "the world blob ends inside a record it promised",
            Self::Format(_) => "the world blob is a different format version",
            Self::CountOverCap => "a section claims more records than the store can hold",
            Self::NotABool => "a boolean byte was neither 0 nor 1",
            Self::Player(e) => e.reason(),
            Self::DuplicatePlayerId => "two bodies claim the same player id",
            Self::AddressOutOfRange => "a structure stands off the island",
            Self::BadContentRow => "a structure names a content row that does not exist",
            Self::PieceColumnPlateSplit => "two pieces in one build column stand on two floors",
            Self::BadItemStack => "an item stack names an impossible item or count",
            Self::BadBackpackId => "a backpack id is zero, duplicated, or past the next id",
            Self::BadCharge => "a charge names an impossible structure",
            Self::BadHotbarSlot => "a body selects a hotbar slot that does not exist",
            Self::BadCode => "a code lock carries a code that is not four digits",
            Self::BadWorldContTable => "a world container names a table no container rolls",
            Self::DuplicateWorldCont => "two world containers claim the same cell",
        }
    }
}

// ---------------------------------------------------------------- writing

/// A bounds-checked forward writer. Every `put` is infallible by
/// construction — the caller sized the buffer from
/// [`WORLD_SAVE_MAX_BYTES`] — but the cursor is still checked, because
/// "infallible by construction" is a claim about a caller and this is the
/// boot path of a shard.
struct W<'a> {
    buf: &'a mut [u8],
    at: usize,
}

impl<'a> W<'a> {
    fn new(buf: &'a mut [u8]) -> Self {
        Self { buf, at: 0 }
    }
    fn u8(&mut self, v: u8) {
        if self.at < self.buf.len() {
            self.buf[self.at] = v;
        }
        self.at += 1;
    }
    fn b(&mut self, v: bool) {
        self.u8(v as u8);
    }
    fn u16(&mut self, v: u16) {
        self.bytes(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.bytes(&v.to_le_bytes());
    }
    fn i32(&mut self, v: i32) {
        self.bytes(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.bytes(&v.to_le_bytes());
    }
    fn bytes(&mut self, v: &[u8]) {
        let end = self.at + v.len();
        if end <= self.buf.len() {
            self.buf[self.at..end].copy_from_slice(v);
        }
        self.at = end;
    }
    fn stack(&mut self, s: &ItemStack) {
        self.u16(s.item);
        self.u16(s.count);
        self.u16(s.cond);
    }
}

/// Encode the world. Returns the encoded length, or [`WorldSaveError::Truncated`]
/// if `out` is smaller than [`WORLD_SAVE_MAX_BYTES`] would have needed.
///
/// **A pure read, and that is the same design decision `save_of` made one
/// level down.** The server takes one of these on a cadence and at
/// shutdown, and none of those is a command — so none of them may change
/// the world, or a replay of the same stream would diverge from the shard
/// that wrote it. Everything that mutates stays on the `Command` path; this
/// only looks.
pub fn encode(w: &World, out: &mut [u8]) -> Result<usize, WorldSaveError> {
    let mut o = W::new(out);
    o.u16(WORLD_SAVE_FORMAT);
    o.u64(w.tick);
    o.u32(w.sweep_piece);
    o.u32(w.sweep_deploy);
    o.u32(w.sweep_support);
    o.u64(w.evictions);
    o.u32(w.backpacks.next_id());
    // The spent store's eviction counter, in the head beside the body one
    // and for its reason: it is hashed, it is not derivable from the
    // records, and a save that dropped it would load to a different
    // `state_hash` than it was taken from.
    o.u32(w.spent.evictions());

    // Bodies. Everyone in the file is written as a sleeper *by the loader*,
    // not here — see `decode_into`. What is written is who was in the world.
    let bodies = w.players.iter().filter(|p| p.active).count();
    o.u16(bodies as u16);
    o.u16(w.pieces.len() as u16);
    o.u16(w.deploys.len() as u16);
    o.u16(w.deploys.hearths().len() as u16);
    o.u16(w.deploys.boxes().len() as u16);
    o.u16(w.deploys.locks().len() as u16);
    o.u16(w.backpacks.len() as u16);
    o.u16(w.world_conts.len() as u16);
    o.u16(w.charges.len() as u16);
    o.u16(w.spent.len() as u16);
    o.u32(w.slot_lives.len() as u32);

    for p in w.players.iter().filter(|p| p.active) {
        let mut rec = [0u8; PLAYER_SAVE_BYTES];
        PlayerSave::of(p).write_le(&mut rec);
        o.bytes(&rec);
        o.u32(p.id);
        // A body that was awake when the shard went down falls asleep at
        // the tick it went down on, which is the fairest reading available
        // and the one that puts it at the *back* of the eviction queue —
        // an eviction order that reaped the players who were mid-session at
        // the crash would be exactly backwards.
        o.u64(if p.sleeping { p.slept_at } else { w.tick });
        // The frame, **normalized the way `Command::Leave` normalizes it**:
        // facing kept, movement and buttons dropped. Duplicated rather than
        // shared with that arm because they answer different questions —
        // `Leave` decides when a body stops being driven, this decides what
        // a body that is already undriven looks like on disk — but the
        // rule is one rule, and it is the sentence in `Leave`: a body
        // nobody is driving must not be recorded as still holding W, or
        // every hash of it reads as a player mid-sprint.
        o.u16(p.frame.seq);
        o.u8(0); // buttons
        o.u16(p.frame.yaw);
        o.u8(p.frame.pitch);
        o.u8(0); // move_x
        o.u8(0); // move_z
        o.u8(p.frame.sel);
        o.u64(p.next_swing);
        // The magazine (format 12). Written because `state_hash` folds it:
        // a snapshot that dropped it would restore a world that hashes
        // differently from the one that was saved, which is wall 5 failing
        // on a field nobody could see. Both arrays, in slot order, because
        // "six loaded" is not a fact until you say six of what.
        for (loaded, round) in p.mag.iter().zip(p.mag_round.iter()) {
            o.u16(*loaded);
            o.u16(*round);
        }
        o.u32(p.ws_cell);
        o.u16(p.ws_hits);
        o.u32(p.death_by);
        o.u8(p.death_cause);
        o.u16(p.death_item);
        o.u16(p.death_range_cm);
        o.u64(p.craft_done_at);
    }
    for (p, placed) in w.pieces.entries().iter().zip(w.pieces.placed()) {
        o.u16(p.cx);
        o.u16(p.cz);
        o.u8(p.level);
        o.u8(p.loc);
        o.u8(p.row);
        o.u8(p.facing);
        o.u16(p.hp);
        o.u16(p.uh);
        // The plate, as raw two's complement (format 9). Signed because a
        // base can be stilted over its ground or cut a band into it, and
        // the sign is the difference between a leg and a buried floor.
        o.u8(p.plate as u8);
        o.u64(*placed);
    }
    for (d, ready) in w.deploys.entries().iter().zip(w.deploys.bag_ready()) {
        o.u16(d.cx);
        o.u16(d.cz);
        o.u8(d.level);
        o.u8(d.loc);
        o.u8(d.row);
        o.u32(d.owner);
        o.u16(d.hp);
        o.u16(d.uh);
        o.b(d.open);
        o.b(d.locked);
        o.u64(*ready);
    }
    for placed in w.deploys.placed() {
        o.u64(*placed);
    }
    for h in w.deploys.hearths() {
        o.u16(h.cx);
        o.u16(h.cz);
        o.u8(h.level);
        o.u32(h.owner);
        for s in h.stock.iter() {
            o.u32(*s);
        }
        o.u8(h.crew.len() as u8);
        for id in h.crew.raw().iter() {
            o.u32(*id);
        }
    }
    for (b, ov) in w.deploys.boxes().iter().zip(w.deploys.oven_states()) {
        o.u16(b.cx);
        o.u16(b.cz);
        o.u8(b.level);
        o.u32(b.owner);
        for s in b.items.iter() {
            o.stack(s);
        }
        // The oven half, inline on the container it belongs to — the two
        // are index-aligned in the store and writing them together is
        // what makes that alignment unforgeable in a file.
        o.u8(ov.arch);
        o.b(ov.lit);
        o.u16(ov.burn);
        o.u16(ov.bank);
        for c in ov.cook.iter() {
            o.u16(*c);
        }
    }
    for l in w.deploys.locks() {
        o.u16(l.cx);
        o.u16(l.cz);
        o.u8(l.level);
        o.u8(l.loc);
        o.u32(l.owner);
        o.u16(l.code);
        o.u16(l.guest_code);
        o.b(l.locked);
        o.u8(l.auth.len() as u8);
        o.u8(l.guests.len() as u8);
        for id in l.auth.raw().iter() {
            o.u32(*id);
        }
        for id in l.guests.raw().iter() {
            o.u32(*id);
        }
        o.u8(l.misses);
        o.u64(l.last_miss);
        o.u64(l.shut_until);
    }
    for b in w.backpacks.entries() {
        o.u32(b.id);
        o.i32(b.qx);
        o.i32(b.qy);
        o.i32(b.qz);
        o.u32(b.owner);
        o.u64(b.expires);
        for s in b.items.iter() {
            o.stack(s);
        }
    }
    for c in w.world_conts.entries() {
        o.u16(c.cx);
        o.u16(c.cz);
        o.i32(c.qx);
        o.i32(c.qz);
        o.u8(c.table);
        o.u64(c.refill_at);
        for s in c.items.iter() {
            o.stack(s);
        }
    }
    for c in w.charges.entries() {
        o.u16(c.cx);
        o.u16(c.cz);
        o.u8(c.level);
        o.u8(c.loc);
        o.b(c.deploy);
        o.u16(c.structure);
        o.u16(c.damage);
        o.u16(c.blast_cm);
        o.u64(c.fires_at);
        o.u32(c.owner);
    }
    for a in w.spent.entries() {
        o.i32(a.qx);
        o.i32(a.qy);
        o.i32(a.qz);
        o.u16(a.round);
        o.u64(a.ready_at);
    }
    for s in w.slot_lives.entries() {
        o.u16(s.cx);
        o.u16(s.cz);
        o.u16(s.hits);
        o.u64(s.respawn_at);
    }

    if o.at > o.buf.len() {
        return Err(WorldSaveError::Truncated);
    }
    Ok(o.at)
}

// ---------------------------------------------------------------- reading

/// A bounds-checked forward reader. **Totality is the contract**: every
/// accessor returns `Err(Truncated)` rather than panicking, so arbitrary
/// bytes on the boot path produce a refusal an operator can read and never a
/// backtrace.
struct R<'a> {
    buf: &'a [u8],
    at: usize,
}

impl<'a> R<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, at: 0 }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], WorldSaveError> {
        let end = self.at.checked_add(n).ok_or(WorldSaveError::Truncated)?;
        if end > self.buf.len() {
            return Err(WorldSaveError::Truncated);
        }
        let s = &self.buf[self.at..end];
        self.at = end;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, WorldSaveError> {
        Ok(self.take(1)?[0])
    }
    fn b(&mut self) -> Result<bool, WorldSaveError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(WorldSaveError::NotABool),
        }
    }
    fn u16(&mut self) -> Result<u16, WorldSaveError> {
        let s = self.take(2)?;
        Ok(u16::from_le_bytes([s[0], s[1]]))
    }
    fn u32(&mut self) -> Result<u32, WorldSaveError> {
        let s = self.take(4)?;
        Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }
    fn i32(&mut self) -> Result<i32, WorldSaveError> {
        Ok(self.u32()? as i32)
    }
    fn u64(&mut self) -> Result<u64, WorldSaveError> {
        let s = self.take(8)?;
        Ok(u64::from_le_bytes([
            s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7],
        ]))
    }
    /// A count, refused against its cap before anything loops on it.
    fn count(&mut self, cap: usize) -> Result<usize, WorldSaveError> {
        let n = self.u16()? as usize;
        (n <= cap).then_some(n).ok_or(WorldSaveError::CountOverCap)
    }
    fn count32(&mut self, cap: usize) -> Result<usize, WorldSaveError> {
        let n = self.u32()? as usize;
        (n <= cap).then_some(n).ok_or(WorldSaveError::CountOverCap)
    }
    /// An item stack in its canonical form. The empty-form rule is the same
    /// one `PlayerSave::read_le` enforces and it is there for the state
    /// hash, not for tidiness: "0 of item 7" hashes differently from the
    /// empty slot the sim would have produced, and a difference nothing can
    /// see is wall 5's failure mode.
    fn stack(&mut self, max_item: usize) -> Result<ItemStack, WorldSaveError> {
        let item = self.u16()?;
        let count = self.u16()?;
        let cond = self.u16()?;
        // The canonical-empty rule, all three fields (format 7): a slot
        // emptied by a path that forgot to zero `cond` hashes differently
        // from the same empty slot the sim produced — a difference nothing
        // can see, which is wall 5's failure mode.
        if item as usize >= max_item || (count == 0 && item != 0) || (count == 0 && cond != 0) {
            return Err(WorldSaveError::BadItemStack);
        }
        Ok(ItemStack { item, count, cond })
    }
}

/// A **build**-grid address: pieces, deployables, hearths, boxes and
/// charges all live on it. Bounded by the same `MAX_BUILD_COORD` the
/// placement path refuses on (`build.rs`, `REFUSE_B_SPOT`), and by the same
/// `MAX_BUILD_LEVELS` the wire's 3-bit level field carries — deliberately
/// those constants and not a second pair, because a decoder that admitted
/// an address the placer would have refused is a decoder that lets a file
/// build something no player could.
fn build_addr_ok(cx: u16, cz: u16, level: u8) -> bool {
    (cx as usize) < MAX_BUILD_COORD
        && (cz as usize) < MAX_BUILD_COORD
        && (level as usize) < MAX_BUILD_LEVELS
}

/// A **terrain**-grid address — a different grid, and the distinction is
/// the kind of thing a save file gets quietly wrong. Harvested slots are
/// keyed by terrain cell (`CELL_SIZE`, 8 m); everything structural above is
/// keyed by build cell (`BUILD_CELL_M`, 3 m). Bounding one with the other's
/// ceiling would either refuse legal stumps or admit addresses no scatter
/// table has.
fn terrain_cell_ok(cx: u16, cz: u16) -> bool {
    let cells = (ISLAND_SIZE / crate::terrain::CELL_SIZE) as u16;
    cx < cells && cz < cells
}

/// Decode a blob into `w`, replacing every store it names.
///
/// **Boot-only, and the doc comment is the enforcement** — see the module
/// header. Called after the content tables are installed and before the
/// first `tick`, because it is the origin of a run and not a mutation
/// inside one. Calling it mid-run would not corrupt anything; it would do
/// something worse, which is silently make the state hashes after it
/// unreproducible from the command stream that appears to have caused them.
///
/// Everything in the file arrives **asleep**. That is not a simplification:
/// a restart ends every connection, so there is no body anybody is driving,
/// and the reference model has exactly this property — at boot a shard is
/// nothing but sleepers (`reference/SAVES.md` §1). A player returning takes
/// theirs over through the same `Command::Wake` a mid-run reconnect uses,
/// which is why there is one path and not two.
pub fn decode_into(w: &mut World, blob: &[u8]) -> Result<(), WorldSaveError> {
    let mut r = R::new(blob);
    let format = r.u16()?;
    if format != WORLD_SAVE_FORMAT {
        return Err(WorldSaveError::Format(format));
    }
    let tick = r.u64()?;
    let sweep_piece = r.u32()?;
    let sweep_deploy = r.u32()?;
    let sweep_support = r.u32()?;
    let evictions = r.u64()?;
    let next_bag = r.u32()?;
    let spent_evictions = r.u32()?;

    let n_players = r.count(MAX_PLAYERS)?;
    let n_pieces = r.count(MAX_PIECES)?;
    let n_deploys = r.count(MAX_DEPLOYS)?;
    let n_hearths = r.count(MAX_HEARTHS)?;
    let n_boxes = r.count(MAX_BOXES)?;
    let n_locks = r.count(MAX_LOCKS)?;
    let n_bags = r.count(MAX_BACKPACKS)?;
    let n_conts = r.count(MAX_WORLD_CONTS)?;
    let n_charges = r.count(MAX_LIVE_CHARGES)?;
    let n_spent = r.count(MAX_SPENT_ARROWS)?;
    let n_slots = r.count32(MAX_SLOT_LIVES)?;

    let max_item = crate::limits::MAX_ITEM_DEFS;
    let piece_rows = w.build.piece_count as usize;
    let deploy_rows = w.deploy.def_count as usize;

    // --- bodies ---------------------------------------------------------
    // Decoded into a scratch array first, because a refusal in the last
    // record must leave the world untouched: a half-loaded shard is a
    // shard whose operator cannot tell what it is running.
    let mut players = [Player::default(); MAX_PLAYERS];
    for slot in players.iter_mut().take(n_players) {
        let mut rec = [0u8; PLAYER_SAVE_BYTES];
        rec.copy_from_slice(r.take(PLAYER_SAVE_BYTES)?);
        let save = PlayerSave::read_le(&rec).map_err(WorldSaveError::Player)?;
        let id = r.u32()?;
        let slept_at = r.u64()?;
        let seq = r.u16()?;
        let buttons = r.u8()?;
        let yaw = r.u16()?;
        let pitch = r.u8()?;
        let move_x = r.u8()? as i8;
        let move_z = r.u8()? as i8;
        let sel = r.u8()?;
        let next_swing = r.u64()?;
        // The magazine (format 12), in the order it was written. Not
        // bounds-checked here beyond the widths: `mag` is a count the sim
        // clamps against `RangedDef::magazine` on the next reload and a
        // forged one can only overstate a cylinder the shooter already
        // holds, and `mag_round` names an item index that `ranged::hitscan`
        // spends without lookup — a forged one fires a round the pack was
        // never debited for, which is the same theft as a forged `inv`
        // stack and is refused by the same thing that refuses that: a save
        // file is not a client (`ItemStack`'s bound is the model).
        let mut mag = [0u16; MAX_MAGS];
        let mut mag_round = [NO_ITEM; MAX_MAGS];
        for (loaded, round) in mag.iter_mut().zip(mag_round.iter_mut()) {
            *loaded = r.u16()?;
            *round = r.u16()?;
        }
        let ws_cell = r.u32()?;
        let ws_hits = r.u16()?;
        let death_by = r.u32()?;
        let death_cause = r.u8()?;
        let death_item = r.u16()?;
        let death_range_cm = r.u16()?;
        let craft_done_at = r.u64()?;
        // The hotbar index is read unchecked by every verb that asks what
        // is in your hand, so it is bounded here for the same reason the
        // wire bounds it at decode (`Command::Input` falls back to 0 for a
        // non-wire command; a file gets refused instead, because a file is
        // not a live client making a mistake).
        if sel as usize >= HOTBAR_SLOTS {
            return Err(WorldSaveError::BadHotbarSlot);
        }
        *slot = Player {
            id,
            active: true,
            sleeping: true,
            slept_at,
            frame: InputFrame {
                seq,
                buttons,
                yaw,
                pitch,
                move_x,
                move_z,
                sel,
            },
            next_swing,
            mag,
            mag_round,
            ws_cell,
            ws_hits,
            death_by,
            death_cause,
            death_item,
            death_range_cm,
            craft_done_at,
            body: save.body,
            inv: save.inv,
            worn: save.worn,
            jobs: save.jobs,
            known: save.known,
            hp: save.hp,
            hp_max: save.hp_max,
            deaths: save.deaths,
            food: save.food,
            water: save.water,
            food_acc: save.food_acc,
            water_acc: save.water_acc,
            hurt_acc: save.hurt_acc,
            heal_rem: save.heal_rem,
            heal_total: save.heal_total,
            heal_span: save.heal_span,
            heal_acc: save.heal_acc,
            // Yes, a world remembers it — the decision this block's own
            // comment demands. It is the survival accumulators' answer for
            // the survival accumulators' reason (torch fuel v0).
            light_acc: save.light_acc,
            dead: save.dead,
            // No `..Player::default()`, and clippy is what pointed it out:
            // every field is named, so the struct-update syntax was a
            // no-op. Left named on purpose now that it is — **a field added
            // to `Player` will stop compiling here**, which forces whoever
            // adds it to decide whether a world remembers it. The default
            // would have silently answered "no", and the whole class of bug
            // this file has to avoid is a field that is state everywhere
            // except on disk.
        };
    }
    // Ids have to be unique or `slot_of` answers arbitrarily and a takeover
    // seats somebody in a stranger's body. O(n²) over at most 100 slots, on
    // the boot path, once.
    for i in 0..n_players {
        for j in (i + 1)..n_players {
            if players[i].id == players[j].id {
                return Err(WorldSaveError::DuplicatePlayerId);
            }
        }
    }

    // --- pieces ---------------------------------------------------------
    let mut pieces = [PieceRec::default(); MAX_PIECES];
    let mut placed = crate::boxed_array::<u64, MAX_PIECES>(0);
    // Indexed rather than `iter_mut`, because the plate check below has to
    // read the records already decoded while this one is still being built.
    for i in 0..n_pieces {
        let cx = r.u16()?;
        let cz = r.u16()?;
        let level = r.u8()?;
        let loc = r.u8()?;
        let row = r.u8()?;
        let facing = r.u8()?;
        let hp = r.u16()?;
        let uh = r.u16()?;
        let plate = r.u8()? as i8;
        placed[i] = r.u64()?;
        // One column, one plate — the invariant `plate_for` maintains and
        // every collision walk reads. Linear over what has been read so far,
        // bounded by `MAX_PIECES`, on the boot path.
        if pieces[..i]
            .iter()
            .any(|p| p.cx == cx && p.cz == cz && p.plate != plate)
        {
            return Err(WorldSaveError::PieceColumnPlateSplit);
        }
        // A plate outside the stilt band is a hand-edited file, and it would
        // flow straight into `state_hash` and into every collision query in
        // the column — the `facing > 1` check's reason, on the field that
        // moves geometry rather than price.
        if (plate as i32) > crate::build::PLATE_RISE_MAX_BANDS
            || (plate as i32) < -crate::build::PLATE_SINK_MAX_BANDS
        {
            return Err(WorldSaveError::AddressOutOfRange);
        }
        if !build_addr_ok(cx, cz, level) {
            return Err(WorldSaveError::AddressOutOfRange);
        }
        // The check that keeps the sim from panicking: `bc.pieces[row]` is
        // indexed unchecked by the shape lookup, the support sweep and
        // every collapse cascade.
        if row as usize >= piece_rows {
            return Err(WorldSaveError::BadContentRow);
        }
        // A facing is one bit wearing a byte; anything else is a
        // hand-edited file, and it would flow into `state_hash`.
        if facing > 1 {
            return Err(WorldSaveError::AddressOutOfRange);
        }
        pieces[i] = PieceRec {
            cx,
            cz,
            level,
            loc,
            row,
            facing,
            hp,
            uh,
            // **The save format did not grow for wire v44**, and that is
            // the payoff of deriving the band at the wire boundary rather
            // than storing it: there is nothing here to write, nothing to
            // read back, and no format bump. A loaded piece bands itself
            // correctly the first time it is encoded, off `hp`.
            dmg: 0,
            plate,
        };
    }

    // --- deployables, with their bag cooldowns --------------------------
    let mut deploys = [DeployRec::default(); MAX_DEPLOYS];
    let mut bag_ready = [0u64; MAX_DEPLOYS];
    for i in 0..n_deploys {
        let cx = r.u16()?;
        let cz = r.u16()?;
        let level = r.u8()?;
        let loc = r.u8()?;
        let row = r.u8()?;
        let owner = r.u32()?;
        let hp = r.u16()?;
        let uh = r.u16()?;
        let open = r.b()?;
        // The locked byte is consumed (and bool-checked, so a corrupt
        // file still refuses) but deliberately never kept — the mirror
        // comment below says why.
        let _locked = r.b()?;
        let ready = r.u64()?;
        if !build_addr_ok(cx, cz, level) {
            return Err(WorldSaveError::AddressOutOfRange);
        }
        if row as usize >= deploy_rows {
            return Err(WorldSaveError::BadContentRow);
        }
        deploys[i] = DeployRec {
            cx,
            cz,
            level,
            loc,
            row,
            owner,
            hp,
            uh,
            open,
            // The two mirror bits are read but not trusted — **neither of
            // them**: both are re-derived from the lock section by
            // `World::rebuild_doors` at the commit below, exactly as the
            // collision index is rebuilt from the pieces. The rebuild only
            // walks archetypes `lockable` names, so the file's `locked`
            // byte has to be dropped *here* or a forged save could present
            // locked:true / has_lock:false on a fire or a hearth — a
            // mirror state no lock verb can produce — and nothing after
            // this line would ever look at it again.
            has_lock: false,
            locked: false,
            // Not saved — see the piece load above.
            dmg: 0,
        };
        bag_ready[i] = ready;
    }
    // The deploy placement clocks, in their own run rather than inline on
    // the record: the record's own encoding is pinned by
    // `tests/worldsave.rs`'s round trip and by `DEPLOY_BYTES`, and
    // appending a second run is the change that does not move the first.
    let mut deploy_placed = crate::boxed_array::<u64, MAX_DEPLOYS>(0);
    for slot in deploy_placed.iter_mut().take(n_deploys) {
        *slot = r.u64()?;
    }

    let mut hearths = [HearthRec::default(); MAX_HEARTHS];
    for h in hearths.iter_mut().take(n_hearths) {
        let cx = r.u16()?;
        let cz = r.u16()?;
        let level = r.u8()?;
        let owner = r.u32()?;
        let mut stock = [0u32; HEARTH_STOCK_ROWS];
        for s in stock.iter_mut() {
            *s = r.u32()?;
        }
        let n_crew = r.u8()?;
        let mut crew_ids = [0u32; HEARTH_CREW_CAP];
        for id in crew_ids.iter_mut() {
            *id = r.u32()?;
        }
        if !build_addr_ok(cx, cz, level) {
            return Err(WorldSaveError::AddressOutOfRange);
        }
        let Some(crew) = crate::deploy::CrewList::restore(&crew_ids, n_crew) else {
            return Err(WorldSaveError::CountOverCap);
        };
        *h = HearthRec {
            cx,
            cz,
            level,
            owner,
            stock,
            crew,
        };
    }

    let mut boxes = [BoxRec::default(); MAX_BOXES];
    let mut ovens = [OvenState::default(); MAX_BOXES];
    for (b, ov) in boxes.iter_mut().zip(ovens.iter_mut()).take(n_boxes) {
        let cx = r.u16()?;
        let cz = r.u16()?;
        let level = r.u8()?;
        let owner = r.u32()?;
        let mut items = [ItemStack::default(); BOX_SLOTS];
        for s in items.iter_mut() {
            *s = r.stack(max_item)?;
        }
        let arch = r.u8()?;
        let lit = r.b()?;
        let burn = r.u16()?;
        let bank = r.u16()?;
        let mut cook = [0u16; BOX_SLOTS];
        for c in cook.iter_mut() {
            *c = r.u16()?;
        }
        if !build_addr_ok(cx, cz, level) {
            return Err(WorldSaveError::AddressOutOfRange);
        }
        // A save is the one non-command path into `World`
        // (`reference/SAVES.md` §9.3: their loader trusts the file and
        // ours cannot), so the archetype is checked here rather than
        // trusted: a byte naming an archetype the sim has no store for
        // would be a container that is neither box nor oven, and every
        // reader downstream would disagree about which.
        if !crate::deploy::holds_items(arch) {
            return Err(WorldSaveError::BadContentRow);
        }
        *b = BoxRec {
            cx,
            cz,
            level,
            owner,
            items,
        };
        *ov = OvenState {
            arch,
            lit,
            burn,
            bank,
            cook,
        };
    }

    // --- code locks -------------------------------------------------------
    let mut locks = [LockRec::default(); MAX_LOCKS];
    for l in locks.iter_mut().take(n_locks) {
        let cx = r.u16()?;
        let cz = r.u16()?;
        let level = r.u8()?;
        let loc = r.u8()?;
        let owner = r.u32()?;
        let code = r.u16()?;
        let guest_code = r.u16()?;
        let locked = r.b()?;
        let n_auth = r.u8()?;
        let n_guests = r.u8()?;
        let mut auth = [0u32; LOCK_AUTH_CAP];
        for id in auth.iter_mut() {
            *id = r.u32()?;
        }
        let mut guests = [0u32; LOCK_GUEST_CAP];
        for id in guests.iter_mut() {
            *id = r.u32()?;
        }
        let misses = r.u8()?;
        let last_miss = r.u64()?;
        let shut_until = r.u64()?;
        if !build_addr_ok(cx, cz, level) {
            return Err(WorldSaveError::AddressOutOfRange);
        }
        // A count past its own array is refused whole, never clamped: a
        // clamped count is a list that quietly forgot somebody, and the
        // whole point of the roster's overflow policy is that it never
        // forgets anyone silently. `Roster::restore` is where that rule
        // lives, so this asks it rather than restating the comparison.
        let (Some(auth), Some(guests)) = (
            crate::lock::AuthList::restore(&auth, n_auth),
            crate::lock::GuestList::restore(&guests, n_guests),
        ) else {
            return Err(WorldSaveError::CountOverCap);
        };
        // A code out of the four-digit range is refused for the reason a
        // content row index is: `lock::apply` compares against it and a
        // hand-edited value would make a door nobody can ever open.
        // `CODE_NONE` is the one legal value above the range.
        if (code > CODE_MAX && code != CODE_NONE)
            || (guest_code > CODE_MAX && guest_code != CODE_NONE)
        {
            return Err(WorldSaveError::BadCode);
        }
        *l = LockRec {
            cx,
            cz,
            level,
            loc,
            owner,
            code,
            guest_code,
            locked,
            auth,
            guests,
            misses,
            last_miss,
            shut_until,
        };
    }

    // --- bags on the ground ---------------------------------------------
    let mut bags = [BackpackRec::default(); MAX_BACKPACKS];
    for b in bags.iter_mut().take(n_bags) {
        let id = r.u32()?;
        let qx = r.i32()?;
        let qy = r.i32()?;
        let qz = r.i32()?;
        let owner = r.u32()?;
        let expires = r.u64()?;
        let mut items = [ItemStack::default(); INV_SLOTS];
        for s in items.iter_mut() {
            *s = r.stack(max_item)?;
        }
        // Zero is "no bag" everywhere in the sim, and an id at or past
        // `next_id` would be minted again by the next death — two bags with
        // one id, and a loot action that resolves to whichever the scan
        // reached first.
        if id == 0 || id >= next_bag {
            return Err(WorldSaveError::BadBackpackId);
        }
        let xz = movement::quant_xz(ISLAND_SIZE);
        if !(0..=xz).contains(&qx) || !(0..=xz).contains(&qz) {
            return Err(WorldSaveError::AddressOutOfRange);
        }
        *b = BackpackRec {
            id,
            qx,
            qy,
            qz,
            owner,
            expires,
            items,
        };
    }
    for i in 0..n_bags {
        for j in (i + 1)..n_bags {
            if bags[i].id == bags[j].id {
                return Err(WorldSaveError::BadBackpackId);
            }
        }
    }

    // --- authored world containers (format 5) ---------------------------
    let mut conts = crate::boxed_array::<WorldContRec, MAX_WORLD_CONTS>(WorldContRec::default());
    for c in conts.iter_mut().take(n_conts) {
        let cx = r.u16()?;
        let cz = r.u16()?;
        let qx = r.i32()?;
        let qz = r.i32()?;
        let table = r.u8()?;
        let refill_at = r.u64()?;
        let mut items = [ItemStack::default(); INV_SLOTS];
        for s in items.iter_mut() {
            *s = r.stack(max_item)?;
        }
        // A save is the one non-command path into `World`, so the file is
        // checked and never trusted (`reference/SAVES.md` §9.3 — their
        // loader trusts it, ours cannot). Three claims, three checks:
        //
        // 1. The cell is on the island. `CELLS_PER_SIDE` is the grid, and
        //    a cell past it would index a scatter that refuses and leave a
        //    container nothing can ever resolve.
        // 2. The table is one a container actually rolls. A forged index
        //    into `LootContent::tables` would silently pay a barrel's
        //    table — or, past the array, pay nothing forever.
        // 3. The stand position is inside the cell it claims. This is the
        //    check that matters, because `qx`/`qz` are what the reach test
        //    reads: without it, a hand-edited save moves the haven pad's
        //    crate to a player's feet and the walk — which is the entire
        //    price of the loot — is gone.
        if !(0..terrain::CELLS_PER_SIDE).contains(&(cx as i32))
            || !(0..terrain::CELLS_PER_SIDE).contains(&(cz as i32))
        {
            return Err(WorldSaveError::AddressOutOfRange);
        }
        if table as usize != LOOT_CRATE && table as usize != LOOT_CACHE {
            return Err(WorldSaveError::BadWorldContTable);
        }
        let cell_m = terrain::CELL_SIZE;
        let x = qx as f32 * movement::POS_XZ_Q;
        let z = qz as f32 * movement::POS_XZ_Q;
        if x < cx as f32 * cell_m
            || x >= (cx as f32 + 1.0) * cell_m
            || z < cz as f32 * cell_m
            || z >= (cz as f32 + 1.0) * cell_m
        {
            return Err(WorldSaveError::AddressOutOfRange);
        }
        *c = WorldContRec {
            cx,
            cz,
            qx,
            qz,
            table,
            refill_at,
            items,
        };
    }
    // One record per cell. Two records naming one cell is the world
    // container's version of the duplicate-bag-id defect above: the move
    // verb resolves by `index_of`, which takes the first, so the second
    // would be loot nothing can reach and a hash nothing can explain.
    for i in 0..n_conts {
        for j in (i + 1)..n_conts {
            if conts[i].cx == conts[j].cx && conts[i].cz == conts[j].cz {
                return Err(WorldSaveError::DuplicateWorldCont);
            }
        }
    }

    // --- burning fuses ---------------------------------------------------
    let mut charges = [ChargeRec::default(); MAX_LIVE_CHARGES];
    for c in charges.iter_mut().take(n_charges) {
        let cx = r.u16()?;
        let cz = r.u16()?;
        let level = r.u8()?;
        let loc = r.u8()?;
        let deploy = r.b()?;
        let structure = r.u16()?;
        let damage = r.u16()?;
        let blast_cm = r.u16()?;
        let fires_at = r.u64()?;
        let owner = r.u32()?;
        if !build_addr_ok(cx, cz, level) {
            return Err(WorldSaveError::AddressOutOfRange);
        }
        // Format 3 compared `structure` — a damage amount — against the
        // content table's row COUNT here, which would have refused any
        // real save holding a live satchel (125 damage against a dozen
        // rows read as "an impossible structure"). The field is a number,
        // not an index; the address check above is the whole shape test.
        // What IS bounded is the blast, against the same one-cell ceiling
        // `validate` holds content to — a forged radius past the scan's
        // ring would damage walls the detonation never looks at.
        if blast_cm > crate::limits::BLAST_MAX_CM {
            return Err(WorldSaveError::BadCharge);
        }
        *c = ChargeRec {
            cx,
            cz,
            level,
            loc,
            deploy,
            structure,
            damage,
            blast_cm,
            fires_at,
            owner,
        };
    }

    // --- arrows on the ground (format 10) --------------------------------
    // Heap-filled for `conts`' reason: `MAX_SPENT_ARROWS` records is 12 kB
    // of scratch, and a fixed array that size built in a decode frame is
    // the wasm shadow-stack trap `CLAUDE.md` lists three sightings of.
    let mut spent = crate::boxed_array::<SpentRec, MAX_SPENT_ARROWS>(SpentRec::default());
    for a in spent.iter_mut().take(n_spent) {
        let qx = r.i32()?;
        let qy = r.i32()?;
        let qz = r.i32()?;
        let round = r.u16()?;
        let ready_at = r.u64()?;
        // A save is the one non-command path into `World`, so the file is
        // checked and never trusted. Two claims here and they are not the
        // world container's three, because a spent arrow has no address to
        // forge and no table to name:
        //
        // 1. It lies on the island. `qx`/`qz` are millimetres, and the only
        //    reader is a distance test — so a coordinate out past the edge
        //    is not exploitable the way a moved crate is, but it IS the
        //    coordinate `take_near` squares, and the i64 arithmetic there
        //    is sized for an island rather than for `i32::MAX`.
        // 2. The round is an item the content table actually has. A forged
        //    index would hand back a stack of whatever item happens to sit
        //    at that rank — the loot-table check's shape, one store over.
        let side_mm = terrain::ISLAND_SIZE as i64 * 1000;
        if i64::from(qx) < 0
            || i64::from(qx) > side_mm
            || i64::from(qz) < 0
            || i64::from(qz) > side_mm
        {
            return Err(WorldSaveError::AddressOutOfRange);
        }
        if round == 0 || round as usize >= max_item {
            return Err(WorldSaveError::BadContentRow);
        }
        *a = SpentRec {
            qx,
            qy,
            qz,
            round,
            ready_at,
        };
    }

    // --- harvested terrain ------------------------------------------------
    let mut slots = [SlotLife::default(); MAX_SLOT_LIVES];
    for s in slots.iter_mut().take(n_slots) {
        let cx = r.u16()?;
        let cz = r.u16()?;
        let hits = r.u16()?;
        let respawn_at = r.u64()?;
        if !terrain_cell_ok(cx, cz) {
            return Err(WorldSaveError::AddressOutOfRange);
        }
        *s = SlotLife {
            cx,
            cz,
            hits,
            respawn_at,
        };
    }

    // --- commit -----------------------------------------------------------
    // Nothing above this line touched `w`. Every refusal returns with the
    // world exactly as the caller had it, which is what lets a shard report
    // "the save was refused, running a fresh world" instead of running half
    // of somebody's base.
    w.tick = tick;
    w.sweep_piece = sweep_piece;
    w.sweep_deploy = sweep_deploy;
    w.sweep_support = sweep_support;
    w.evictions = evictions;
    w.players = players;
    w.pieces
        .restore(&pieces[..n_pieces], &placed[..n_pieces], &w.build);
    w.deploys.restore(
        &deploys[..n_deploys],
        &bag_ready[..n_deploys],
        &deploy_placed[..n_deploys],
        &hearths[..n_hearths],
        &boxes[..n_boxes],
        &ovens[..n_boxes],
        &locks[..n_locks],
    );
    w.backpacks.restore(&bags[..n_bags], next_bag);
    w.world_conts.restore(&conts[..n_conts]);
    w.charges.restore(&charges[..n_charges]);
    w.spent.restore(&spent[..n_spent], spent_evictions);
    w.slot_lives.restore(&slots[..n_slots]);
    // The collision index is derived, so it is rebuilt rather than stored —
    // and the doors are the half that is easy to forget, because a door's
    // shut bit lives on a *deployable* while the surface it blocks is a
    // *piece*. A rebuild that skipped this would leave every door on the
    // shard open to walk through while the wire drew them closed.
    w.rebuild_doors();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The canonical-empty rule's **container** arm (format 7): a bag slot
    /// emptied by a path that forgot to zero `cond` is refused by THIS
    /// module's own `stack` reader — the player sections go through
    /// `PlayerSave::read_le`, which carries the same rule separately, so
    /// dropping the arm here alone would leave every bag, box and world
    /// container unguarded with the player-side test still green. In-crate
    /// because the corrupt record is minted through `restore`, which no
    /// command can produce — that is the point: the save file is the one
    /// non-command path into `World`.
    #[test]
    fn a_container_slot_emptied_without_zeroing_cond_is_refused() {
        use crate::backpack::BackpackRec;
        use crate::gather::ItemStack;
        use crate::world::World;

        let mut w = Box::new(World::new(1));
        let mut bag = BackpackRec {
            id: 1,
            qx: 100,
            qy: 50,
            qz: 100,
            owner: 3,
            expires: 500,
            items: [ItemStack::default(); crate::limits::INV_SLOTS],
        };
        // The non-canonical empty nothing can see: no item, no count, a
        // condition. Written verbatim by the encoder (a pure read), so the
        // decoder is the only thing standing between it and `state_hash`.
        bag.items[0] = ItemStack {
            item: 0,
            count: 0,
            cond: 9,
        };
        // A second, legal stack so the bag is not empty-by-construction.
        bag.items[1] = ItemStack {
            item: 1,
            count: 3,
            cond: 0,
        };
        w.backpacks.restore(&[bag], 2);

        let mut blob = vec![0u8; WORLD_SAVE_MAX_BYTES];
        let n = encode(&w, &mut blob).expect("encodes");
        blob.truncate(n);

        let mut w2 = Box::new(World::new(1));
        assert_eq!(
            decode_into(&mut w2, &blob),
            Err(WorldSaveError::BadItemStack),
            "a bag slot with condition and no count must refuse the load"
        );
    }

    /// The size constant, asserted rather than trusted — it is derived from
    /// ten caps, and it is what the server preallocates. A cap that moves
    /// without this test being read is a buffer that is silently too small.
    #[test]
    fn the_ceiling_is_what_the_caps_add_up_to() {
        // 52 → 84 at format 12: the magazine, `MAX_MAGS` pairs of `u16`
        // written inline after `next_swing` (reload v1). Saved because
        // `state_hash` folds it — a snapshot that dropped it would restore
        // a world that hashes differently from the one it was written from.
        assert_eq!(PLAYER_TAIL_BYTES, 84);
        // 308 → 320 at format 8: `PlayerSave` carries `WEAR_SLOTS` worn
        // stacks at the inventory's six-byte stride (armor v0). 320 → 324
        // at format 11: the torch's remainder in its scalar head (torch
        // fuel v0).
        assert_eq!(
            PLAYER_BYTES, 356,
            "a body is PlayerSave plus every other hashed field"
        );
        // The sum, spelled out, so the number below is checkable by
        // reading rather than by trusting the expression that produced it.
        // Written this way because the first version of this test asserted
        // a hand-computed total that was 495 bytes short — the box record
        // is 9 + 12 stacks = 57, and 55 is what you get by forgetting that
        // a stack is four bytes and not two. A constant a reader cannot
        // re-derive is a constant nobody checks twice.
        let by_hand = 62                    // head
            + 100 * 356                     // players (6 B a stack, 2 worn at format 8, light_acc at 11, the magazine at 12)
            + 8_192 * 21                    // pieces + plate + placement tick
            + 1_024 * 33                    // deploys + bag_ready + placed
            + 256 * 66                      // hearths (25 + the crew: 1 + 10*4)
            + 256 * 111                     // containers: 9 + 12 six-byte stacks + the oven's 30
            + 512 * 98                      // code locks
            + 256 * 208                     // bags: 28 + 30 six-byte stacks
            + 64 * 201                      // world containers: 21 + 30 six-byte stacks
            + 64 * 25                       // charges
            + 512 * 22                      // spent arrows (format 10)
            + 16_384 * 14; // harvested slots
                           // 54 -> 56 at format 5: a ninth section count is a `u16` in the head.
                           // 56 -> 62 at format 10: a tenth count, plus the
                           // spent store's `u32` eviction counter beside the
                           // body one.
        assert_eq!(HEAD_BYTES, 62);
        // Three millimetre coordinates, the round, and the ready deadline.
        assert_eq!(SPENT_BYTES, 22);
        // A world container is 201: 4 cell + 8 quantized position + 1 table
        // + 8 refill deadline, then `INV_SLOTS` stacks at six bytes
        // (format 7: item, count, condition).
        assert_eq!(WORLD_CONT_BYTES, 201);
        // A lock is 98: 6 address + 4 owner + 2 + 2 codes + 1 locked + 2
        // counts + 8 auth ids + 8 guest ids at four bytes each + 1 miss
        // counter + 8 + 8 for the two tick deadlines.
        assert_eq!(LOCK_BYTES, 98);
        // A hearth is 66: 9 address + owner, 16 stock, then the crew's
        // count and its whole ten-slot backing array.
        assert_eq!(HEARTH_BYTES, 66);
        // Both stores grew eight bytes a record at demolish v1: the
        // placement tick is a parallel array in the store and inline
        // here, for `DEPLOY_BYTES`' stated reason.
        // 19 → 21: 20 is what the encoder has written since format 6 (the
        // constant was a byte short — see `PIECE_BYTES`), plus the plate.
        assert_eq!(PIECE_BYTES, 21);
        assert_eq!(DEPLOY_BYTES, 33);
        assert_eq!(WORLD_SAVE_MAX_BYTES, by_hand);
        // Moved 572_246 → 572_502 at format 4: a charge grew four bytes
        // (damage + blast_cm, satchel blast v0) and there are 64 of them.
        // Moved 572_502 → 581_528 at format 5: the world-container section,
        // `MAX_WORLD_CONTS` (64) × 141, plus two head bytes for its count.
        // Moved 581_528 → 612_872 at format 7: every stack in every store
        // is six bytes (item durability v0) — 100 inventories, 256 bags,
        // 256 boxes and 64 world containers all widened together, because
        // one shared `stack` writer is what they all go through.
        // Moved 612_872 → 614_072 at format 8: a player record carries two
        // worn stacks (armor v0) — 100 × 12 bytes, and only the player
        // section, because nothing else wears anything.
        // Moved 614_072 → 630_456 at format 9, and TWO bytes a piece of that
        // is one change: the plate (build plate v1), plus the byte `facing`
        // has cost since format 6 that this ceiling never counted
        // (`PIECE_BYTES`). Only the piece section moves — a deployable reads
        // its column's plate out of the pieces rather than storing a copy.
        // Moved 630_456 → 641_726 at format 10: a whole new section
        // (`MAX_SPENT_ARROWS` × 22) plus six bytes of head.
        // Moved 641_726 → 642_126 at format 11: four bytes a player for the
        // torch's remainder, over `MAX_PLAYERS`.
        // Moved 642_126 → 645_326 at format 12: the magazine, `MAX_MAGS`
        // (8) pairs of `u16` a player over `MAX_PLAYERS` — 3 200 bytes,
        // and the widest single-slice growth the player section has taken.
        // It buys a magazine keyed by weapon row rather than a fourth
        // field on `ItemStack`, which would have widened every stack in
        // every store instead (format 7 is what that costs: 31 kB).
        assert_eq!(
            WORLD_SAVE_MAX_BYTES, 645_326,
            "the world save ceiling moved"
        );
    }

    /// A blob of arbitrary bytes must refuse, never panic. The boot path
    /// reads a file an operator may have truncated, swapped or edited, and
    /// a backtrace there is a shard that will not start with no sentence
    /// saying why.
    #[test]
    fn decode_is_total_on_arbitrary_bytes() {
        let mut rng = crate::rng::Pcg32::new(0x0047_4154_4553, 77);
        for len in [0usize, 1, 2, 9, 40, 64, 200, 4096] {
            for _ in 0..40 {
                let mut blob = vec![0u8; len];
                for b in blob.iter_mut() {
                    *b = rng.next_bounded(256) as u8;
                }
                // A valid format word most of the time, so the fuzz gets
                // past the first gate and into the record loops.
                if len >= 2 {
                    blob[0..2].copy_from_slice(&WORLD_SAVE_FORMAT.to_le_bytes());
                }
                let mut w = World::new(1);
                let _ = decode_into(&mut w, &blob);
            }
        }
    }
}
