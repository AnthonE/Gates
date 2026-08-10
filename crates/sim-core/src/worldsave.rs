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
use crate::gather::{ItemStack, SlotLife};
use crate::input::InputFrame;
use crate::limits::HOTBAR_SLOTS;
use crate::limits::{
    BOX_SLOTS, HEARTH_CREW_CAP, HEARTH_STOCK_ROWS, INV_SLOTS, LOCK_AUTH_CAP, LOCK_GUEST_CAP,
    MAX_BACKPACKS, MAX_BOXES, MAX_BUILD_COORD, MAX_BUILD_LEVELS, MAX_DEPLOYS, MAX_HEARTHS,
    MAX_LIVE_CHARGES, MAX_LOCKS, MAX_PIECES, MAX_PLAYERS, MAX_SLOT_LIVES,
};
use crate::lock::{LockRec, CODE_MAX, CODE_NONE};
use crate::movement;
use crate::oven::OvenState;
use crate::persist::{PlayerSave, SaveError, PLAYER_SAVE_BYTES};
use crate::terrain::ISLAND_SIZE;
use crate::world::{Player, World};

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
pub const WORLD_SAVE_FORMAT: u16 = 3;

/// Fixed head: format, tick, the three sweep cursors, the eviction counter,
/// the next bag id, and the nine section counts.
const HEAD_BYTES: usize = 2 + 8 + 4 * 3 + 8 + 4 + SECTION_COUNTS;
/// Eight `u16` counts and one `u32` (`slot_lives`, whose cap is 16 384 and
/// so does not fit a `u16` with room to be over-cap and *refused* rather
/// than wrapping — the count has to be able to say an illegal number).
const SECTION_COUNTS: usize = 8 * 2 + 4;

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
/// the weak-spot pair, the four death-screen facts, and `craft_done_at`.
const PLAYER_TAIL_BYTES: usize = 4 + 8 + 9 + 8 + 6 + 9 + 8;
/// On-disk stride of one saved body. Public because two byte-poking
/// tests in `tests/worldsave.rs` have to seek past the player section, and
/// a hand-copied 240 there is a silent wrong-offset the day `PlayerSave`
/// grows — which is exactly what happened at research v0.
pub const PLAYER_BYTES: usize = PLAYER_SAVE_BYTES + PLAYER_TAIL_BYTES;
/// A piece record plus its placement tick, which lives in a parallel
/// array in the store (`build.rs` says why it is not on the record) and
/// is written inline here for `DEPLOY_BYTES`' reason — a file has no
/// parallel arrays worth having.
const PIECE_BYTES: usize = 11 + 8;
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
const BOX_BYTES: usize = 9 + BOX_SLOTS * 4 + 6 + BOX_SLOTS * 2;
/// One code lock (lock v1). Address, owner, both codes, the locked bit,
/// both remembered lists with their counts, and the three brute-force
/// counters — the whole `LockRec`, because every field of it is hashed
/// and a save that dropped one would load to a different `state_hash`
/// than it was taken from (the `PLAYER_TAIL_BYTES` argument, one store
/// over).
const LOCK_BYTES: usize =
    6 + 4 + 2 + 2 + 1 + 1 + 1 + LOCK_AUTH_CAP * 4 + LOCK_GUEST_CAP * 4 + 1 + 8 + 8;
const BACKPACK_BYTES: usize = 28 + INV_SLOTS * 4;
const CHARGE_BYTES: usize = 21;
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
    + MAX_LIVE_CHARGES * CHARGE_BYTES
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
            Self::BadItemStack => "an item stack names an impossible item or count",
            Self::BadBackpackId => "a backpack id is zero, duplicated, or past the next id",
            Self::BadCharge => "a charge names an impossible structure",
            Self::BadHotbarSlot => "a body selects a hotbar slot that does not exist",
            Self::BadCode => "a code lock carries a code that is not four digits",
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
    o.u16(w.charges.len() as u16);
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
        o.u16(p.hp);
        o.u16(p.uh);
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
    for c in w.charges.entries() {
        o.u16(c.cx);
        o.u16(c.cz);
        o.u8(c.level);
        o.u8(c.loc);
        o.b(c.deploy);
        o.u16(c.structure);
        o.u64(c.fires_at);
        o.u32(c.owner);
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
        if item as usize >= max_item || (count == 0 && item != 0) {
            return Err(WorldSaveError::BadItemStack);
        }
        Ok(ItemStack { item, count })
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

    let n_players = r.count(MAX_PLAYERS)?;
    let n_pieces = r.count(MAX_PIECES)?;
    let n_deploys = r.count(MAX_DEPLOYS)?;
    let n_hearths = r.count(MAX_HEARTHS)?;
    let n_boxes = r.count(MAX_BOXES)?;
    let n_locks = r.count(MAX_LOCKS)?;
    let n_bags = r.count(MAX_BACKPACKS)?;
    let n_charges = r.count(MAX_LIVE_CHARGES)?;
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
            ws_cell,
            ws_hits,
            death_by,
            death_cause,
            death_item,
            death_range_cm,
            craft_done_at,
            body: save.body,
            inv: save.inv,
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
    for (i, p) in pieces.iter_mut().take(n_pieces).enumerate() {
        let cx = r.u16()?;
        let cz = r.u16()?;
        let level = r.u8()?;
        let loc = r.u8()?;
        let row = r.u8()?;
        let hp = r.u16()?;
        let uh = r.u16()?;
        placed[i] = r.u64()?;
        if !build_addr_ok(cx, cz, level) {
            return Err(WorldSaveError::AddressOutOfRange);
        }
        // The check that keeps the sim from panicking: `bc.pieces[row]` is
        // indexed unchecked by the shape lookup, the support sweep and
        // every collapse cascade.
        if row as usize >= piece_rows {
            return Err(WorldSaveError::BadContentRow);
        }
        *p = PieceRec {
            cx,
            cz,
            level,
            loc,
            row,
            hp,
            uh,
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

    // --- burning fuses ---------------------------------------------------
    let mut charges = [ChargeRec::default(); MAX_LIVE_CHARGES];
    for c in charges.iter_mut().take(n_charges) {
        let cx = r.u16()?;
        let cz = r.u16()?;
        let level = r.u8()?;
        let loc = r.u8()?;
        let deploy = r.b()?;
        let structure = r.u16()?;
        let fires_at = r.u64()?;
        let owner = r.u32()?;
        if !build_addr_ok(cx, cz, level) {
            return Err(WorldSaveError::AddressOutOfRange);
        }
        let rows = if deploy { deploy_rows } else { piece_rows };
        if structure as usize >= rows {
            return Err(WorldSaveError::BadCharge);
        }
        *c = ChargeRec {
            cx,
            cz,
            level,
            loc,
            deploy,
            structure,
            fires_at,
            owner,
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
    w.charges.restore(&charges[..n_charges]);
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

    /// The size constant, asserted rather than trusted — it is derived from
    /// nine caps, and it is what the server preallocates. A cap that moves
    /// without this test being read is a buffer that is silently too small.
    #[test]
    fn the_ceiling_is_what_the_caps_add_up_to() {
        assert_eq!(PLAYER_TAIL_BYTES, 52);
        assert_eq!(
            PLAYER_BYTES, 248,
            "a body is PlayerSave plus every other hashed field"
        );
        // The sum, spelled out, so the number below is checkable by
        // reading rather than by trusting the expression that produced it.
        // Written this way because the first version of this test asserted
        // a hand-computed total that was 495 bytes short — the box record
        // is 9 + 12 stacks = 57, and 55 is what you get by forgetting that
        // a stack is four bytes and not two. A constant a reader cannot
        // re-derive is a constant nobody checks twice.
        let by_hand = 54                    // head
            + 100 * 248                     // players
            + 8_192 * 19                    // pieces + placement tick
            + 1_024 * 33                    // deploys + bag_ready + placed
            + 256 * 66                      // hearths (25 + the crew: 1 + 10*4)
            + 256 * 87                      // containers: 57 + the oven's 30
            + 512 * 98                      // code locks
            + 256 * 148                     // bags
            + 64 * 21                       // charges
            + 16_384 * 14; // harvested slots
        assert_eq!(HEAD_BYTES, 54);
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
        assert_eq!(PIECE_BYTES, 19);
        assert_eq!(DEPLOY_BYTES, 33);
        assert_eq!(WORLD_SAVE_MAX_BYTES, by_hand);
        assert_eq!(
            WORLD_SAVE_MAX_BYTES, 572_246,
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
