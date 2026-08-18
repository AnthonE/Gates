//! The whole client, composed: snapshot view, own-capsule predictor,
//! remote interpolation, and the dilated client clock. Pure — no I/O, no
//! wall clock. The host (a native test or the wasm bridge) feeds incoming
//! datagram bytes, real milliseconds, and the current input state, and
//! reads render state back. The server's integration gate drives this
//! exact struct against `ShardCore`; the browser drives it through
//! `bridge.rs`.

use crate::clock::ClientClock;
use crate::interp::Interp;
use crate::predict::Predictor;
use crate::view::{Applied, ClientView};
use protocol::{
    decode_event, encode_input, ChatText, EventMsg, InputDatagram, ItemCatalog, WireBag, WireError,
};
use sim_core::build::{BuildContent, PieceRec};
use sim_core::collide::ColIndex;
use sim_core::craft::CraftContent;
use sim_core::deploy::{BagAnchor, DeployContent, DeployRec, ARCH_DOOR, BAG_CAP};
use sim_core::gather::{cell_key, ItemStack, NO_CELL};
use sim_core::input::InputFrame;
use sim_core::inventory::CONT_SELF;
use sim_core::limits::{
    CRAFT_QUEUE, HEARTH_STOCK_ROWS, HOTBAR_SLOTS, INV_SLOTS, MAX_BACKPACKS, MAX_BOXES, MAX_DEPLOYS,
    MAX_PIECES, MAX_SLOT_LIVES,
};
use sim_core::movement::POS_XZ_Q;
use sim_core::occupy::{Harvested, Occupants, SlotCache};
use sim_core::terrain::{self, Haven, ScatterTable};

/// Gather toasts buffered for the HUD (drop-oldest — a toast is cosmetic).
pub const TOAST_RING: usize = 8;

/// Craft refusal reasons buffered for the HUD (drop-oldest, cosmetic).
pub const REFUSAL_RING: usize = 4;

/// How far a `BagDropped` may land from the dead predicted body and still
/// be tagged as this body's own bag (`ClientCore::own_bag`). Covers the
/// unreconciled prediction lead at death — a sprint's worth of round trip
/// (~5.5 m/s x 300 ms) with room to spare — while staying far under the
/// distance at which two simultaneous deaths stop being "the same spot".
pub const OWN_BAG_NEAR_M: f32 = 4.0;

/// Chat lines buffered for the log (drop-oldest). Deeper than the toast
/// ring because the pump drains it every event, not every frame, and a
/// dropped line is the one loss a player would actually notice.
pub const CHAT_RING: usize = 16;

/// Arrow impacts buffered for the renderer (drop-oldest, cosmetic).
///
/// **Deeper than `REFUSAL_RING` because this is a broadcast lane, not an
/// own-fact one.** A refusal is one hand pressing one key, so four covers
/// a stutter; an impact arrives from every archer on the island at once,
/// and the tick that ends a volley ends several arrows in it. Eight is a
/// full `TOAST_RING`'s worth of that, which is more than a drain window
/// can plausibly miss.
///
/// It is deliberately **not** sized to `MAX_ARROWS` (128). That is a
/// shard-wide cap on arrows in the air; this is a *view* buffer that the
/// renderer empties every frame, and the thing that actually bounds how
/// many marks exist is the decal pool, one crate over. Two caps for two
/// questions — `render::tracer`'s `TRACERS` makes the same split for the
/// same reason.
pub const IMPACT_RING: usize = 8;

/// Buffered swings — one entry is one body's arm starting to move
/// (wire v47).
///
/// **Eight, and eight because `FEED_CAP` is eight.** The render side drains
/// this into a fixed `Feed` buffer of that size every frame, so a deeper
/// ring here could only ever hand the drain a surplus it must then count as
/// dropped — a queue that overflows one layer further in is not a bigger
/// queue, it is a bigger silence. Sized against the shard rather than
/// guessed: `SWING_INTERVAL_TICKS` is 38, so 100 players all swinging as
/// fast as the sim allows produce ~2.6 swings a tick, and a frame that fell
/// behind by three ticks would still fit.
///
/// Drop-**oldest**, `IMPACT_RING`'s policy for `IMPACT_RING`'s reason: if
/// more arrive than a frame can take, the newest are the arcs still worth
/// drawing.
pub const SWING_RING: usize = 8;

/// The victim slot of a hitmarker that names no body.
///
/// The hitmarker ring carries two facts that read identically to a player —
/// *my swing landed for N* — and only one of them happened to a person:
/// `EV_HIT` names a victim, and `EV_STRUCT_HIT` names a wall. Rather than
/// split the ring (the marker and the music bump would then have to be
/// summed from two places), the structure's entry carries this instead of an
/// id. `sim_core::gather::NO_ITEM`'s shape, one field over.
///
/// `u32::MAX` is safe as a sentinel and not merely unlikely: a player id is
/// a roster index below `limits::MAX_PLAYERS` and an animal's is
/// `limits::MOB_ID_TAG | slot` with `slot < MAX_MOBS`, so neither family can
/// reach it.
pub const NO_VICTIM: u32 = u32::MAX;

/// What one event-lane message changed, as bit flags the bridge hands JS.
pub const APPLIED_INV: u32 = 1 << 0;
pub const APPLIED_SLOTS: u32 = 1 << 1;
pub const APPLIED_RESET: u32 = 1 << 2;
pub const APPLIED_TOAST: u32 = 1 << 3;
pub const APPLIED_CATALOG: u32 = 1 << 4;
/// The own weak-spot mark moved, appeared, or cleared.
pub const APPLIED_MARK: u32 = 1 << 5;
/// The own craft queue changed (jobs and/or the head timer).
pub const APPLIED_CRAFT_Q: u32 = 1 << 6;
/// A craft unit completed (a toast is buffered).
pub const APPLIED_CRAFT_DONE: u32 = 1 << 7;
/// A craft request bounced (a refusal reason is buffered).
pub const APPLIED_CRAFT_REFUSED: u32 = 1 << 8;
/// Recipe rows arrived (the craft menu's data grew).
pub const APPLIED_RECIPES: u32 = 1 << 9;
/// Placed pieces arrived (`piece_changes()` has the records).
pub const APPLIED_PIECES: u32 = 1 << 10;
/// The piece set reset first (join sync / resync) — clear meshes.
pub const APPLIED_PIECE_RESET: u32 = 1 << 11;
/// A place request bounced (a refusal reason is buffered).
pub const APPLIED_BUILD_REFUSED: u32 = 1 << 12;
/// Piece-def rows arrived (the build menu's data grew).
pub const APPLIED_PIECE_DEFS: u32 = 1 << 13;
/// Placed deployables arrived (`deploy_changes()` has the records).
pub const APPLIED_DEPLOYS: u32 = 1 << 14;
/// The deployable set reset first (join sync / resync) — clear meshes.
pub const APPLIED_DEPLOY_RESET: u32 = 1 << 15;
/// A deploy or feed request bounced (a refusal reason is buffered).
pub const APPLIED_DEPLOY_REFUSED: u32 = 1 << 16;
/// Deploy-def rows arrived (the deploy menu's data grew).
pub const APPLIED_DEPLOY_DEFS: u32 = 1 << 17;
/// A hearth stock ack arrived (`stock`/`stock_count` hold the rows).
pub const APPLIED_STOCK: u32 = 1 << 18;
/// Decay removed a piece (`removed_addr()` names the address).
pub const APPLIED_PIECE_REMOVED: u32 = 1 << 19;
/// Decay removed a deployable (`removed_addr()` names the address).
pub const APPLIED_DEPLOY_REMOVED: u32 = 1 << 20;
/// A chat line arrived (`pop_chat` has it).
pub const APPLIED_CHAT: u32 = 1 << 21;
/// Own health changed (`EventMsg::Health`).
pub const APPLIED_HEALTH: u32 = 1 << 22;
/// This client's swing landed (`EventMsg::Hit`).
pub const APPLIED_HIT: u32 = 1 << 23;
/// Someone died (`EventMsg::Death`) — broadcast, not necessarily you.
pub const APPLIED_DEATH: u32 = 1 << 24;
/// The standing-backpack set changed (dropped, synced, or removed). One
/// flag for all three because the client holds the whole set, not a
/// delta: the renderer re-reads `client_bags_ptr` and is done. A bag is
/// one small mesh at a point, so re-reading ≤ `MAX_BACKPACKS` of them is
/// cheaper than the bookkeeping a delta would need.
pub const APPLIED_BAGS: u32 = 1 << 25;
/// A structure's hp at an address changed and it is still standing —
/// `EventMsg::StructHit` taking it down, `EventMsg::PieceRepaired` putting
/// it back. `struct_hit` names the address and where the piece now stands
/// out of its maximum.
///
/// A hit sets this **and** `APPLIED_HIT`, which is what drains the
/// hitmarker ring; a repair sets this alone, because nobody was struck.
/// A reader that wants only raid damage checks for both.
pub const APPLIED_STRUCT_HIT: u32 = 1 << 26;
/// Own food/water changed (`EventMsg::Vitals`).
pub const APPLIED_VITALS: u32 = 1 << 27;
/// A consume answer arrived (`EventMsg::Consumed` / `ConsumeRefused`) — a
/// ring gained an entry. One flag for both because the response to either
/// is the same: drain `pop_consume_toast` and `pop_consume_refusal`, which
/// say which it was. Rings since 2026-08-15 — as a field plus this bit, two
/// answers in one drain window collapsed, and `KeyJ` + `KeyH` in one frame
/// is answered by one `World::tick`.
pub const APPLIED_CONSUME: u32 = 1 << 28;
/// A drink landed (`EventMsg::Drank`). Its own bit and not `CONSUME`'s: a
/// refused drink already arrives as a `ConsumeRefused`, so sharing the bit
/// would leave the HUD holding two readouts and no way to know which of
/// them this frame's flag was about.
pub const APPLIED_DRANK: u32 = 1 << 29;
/// The death screen opened or closed — `EventMsg::Death` naming *you*, or
/// the `EventMsg::Respawn` that answers it. One flag for both because the
/// response to either is to re-read [`ClientCore::dead`] (`render/death.rs`
/// is the reader), which says which it was; the same shape
/// `APPLIED_CONSUME` takes.
///
/// Not `APPLIED_DEATH`'s bit, and the distinction is the whole point of a
/// separate flag: `Death` is broadcast, so most of them are somebody
/// else's and belong in the kill feed. This one only ever fires for the
/// body this client is driving.
pub const APPLIED_RESPAWN: u32 = 1 << 30;

/// **Bit 31 is not an applied-flag and never becomes one.** It is the
/// bridge's `client_on_stream` error sentinel, which shares the return
/// channel with this word, so a flag placed here reads to JS as a decode
/// failure. `bridge.rs` re-exports it under its own name; the value lives
/// here so one file owns the whole word and
/// `applied_word_is_full_and_bit_31_is_the_error_sentinel` can prove it.
///
/// That is not hypothetical. `APPLIED_MOVE` was written here — bits 0..30
/// were already spent and the comment on it said "the last bit in the
/// word" — and the first `Moved` or `MoveRefused` of a session therefore
/// tripped the client's error branch, which logs and returns *early*, so
/// the inventory diff carried by the same pump iteration went with it.
/// Every wall stayed green: two crates, two constants, one value.
pub const STREAM_ERR: u32 = 1 << 31;

/// The second applied-flag word: word 0's low 31 bits are spent and its
/// high bit belongs to `STREAM_ERR`, so this is the "second word" the
/// note on `APPLIED_MOVE` said the thirty-third flag would need.
///
/// It is **not announced by word 0** — there is no spare bit left there to
/// announce it with. A caller reads it after every `client_on_stream`, the
/// way it already reads `slot_changes()`: valid until the next call, and
/// cleared by any message that sets nothing in it, so a stale verdict can
/// never be read as a fresh one.
///
/// A move landed or was refused (`EventMsg::Moved` / `MoveRefused`). One
/// flag for both: the answer is in `last_move` / `last_move_refused`, which
/// say which it was. (`client_move_readout`, named here until 2026-08-15,
/// was the deleted C-ABI bridge; the real reader is
/// `render/panels/mod.rs::sync_refusals`, which keys freshness on the
/// `last_move` counter rather than on this flag.)
pub const APPLIED2_MOVE: u32 = 1 << 0;
/// The open container's view changed (`EventMsg::ContSync`) — contents
/// arrived, or the server shut the panel. One flag for both, and for
/// `APPLIED2_MOVE`'s reason: the panel's response to either is to re-read
/// `client_cont_kind()`, which is zero when nothing is open and names the
/// container otherwise.
pub const APPLIED2_CONT: u32 = 1 << 1;

/// A satchel charge was planted somewhere in the world and its fuse is
/// burning — `EventMsg::ChargePlaced`. Re-read `client_charge_key`,
/// `client_charge_info` and `client_charge_fuse`.
///
/// It lands in word 1 rather than word 0 for a reason worth writing down:
/// **word 0 is full.** Bit 30 is `APPLIED_RESPAWN` and bit 31 is not an
/// applied-flag at all — it is `client_on_stream`'s error sentinel sharing
/// the return — so this word is where the next flag was always going to
/// go, and this is the first one to arrive since it was opened.
///
/// Broadcast like the event behind it, so most of these are somebody
/// else's charge on somebody else's wall. That is the point rather than a
/// caveat: the charge you most need drawn is the one you did not plant.
pub const APPLIED2_CHARGE: u32 = 1 << 2;

/// Something about research landed: a blueprint learned, a refusal, or the
/// known-mask restated (research v0). **One flag for all three**, the
/// shape `APPLIED_CONSUME` takes: the reader's response to any of them is
/// the same — re-read `known()` and drain the rings — so three bits would
/// be three ways to spell one redraw.
pub const APPLIED2_RESEARCH: u32 = 1 << 3;

/// A payout did not fit and went to the ground — an `EV_GATHER` or
/// `EV_CRAFT_DONE` whose "units actually added" was zero. Drain `pop_spill`.
///
/// **The wire did not change to carry this.** The zero was always on it;
/// `EV_CRAFT_DONE`'s doc line has declared its meaning since it landed and
/// `EV_GATHER`'s now does too, and the client simply threw both away — the
/// gather arm on an `if added > 0` guard, the craft arm by formatting the
/// zero into "crafted 0 × Hatchet" and showing it. So this is the client-
/// side read `NOW.md` §0sp2 asked whether existed. It does, for the whole-
/// spill case, and it does not for the other two halves: a PARTIAL spill
/// (some fit, some did not) is invisible here because the shortfall never
/// leaves the sim, and the four give-backs — demolish refund, deployable
/// pick-up, lock removal, craft cancel — emit no payout event at all.
/// Both of those need a wire field, which is the operator's question.
///
/// Word 1 because word 0 is full; see `APPLIED2_CHARGE`.
pub const APPLIED2_SPILL: u32 = 1 << 4;

/// This client's own bag list arrived (`EventMsg::Bags`, wire v43) —
/// re-read `own_bags()`. Word 1 for `APPLIED2_CHARGE`'s reason.
///
/// The list is sent on a death and nowhere else, so in practice this bit
/// is raised one message before the `Death` that raises the screen
/// reading it. A reader that wants "is it fresh" wants this bit; a reader
/// that wants "what do I own" can read the field any time — it is a
/// latched set, not a ring, and it is only ever replaced whole.
pub const APPLIED2_BAGS: u32 = 1 << 5;

/// The client's mirror of the server's harvested-cell set — which scatter
/// slots currently have no node standing. Bounded like the server's store
/// (`MAX_SLOT_LIVES` covers every slot a seed produces); a server-driven
/// insert past capacity is dropped (the node ghosts until its respawn
/// event, the same bounded staleness the sync walk already accepts).
pub struct HarvestedSet {
    cells: Box<[u32]>,
    len: usize,
}

impl HarvestedSet {
    fn new() -> Self {
        Self {
            cells: vec![0; MAX_SLOT_LIVES].into_boxed_slice(),
            len: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn contains(&self, key: u32) -> bool {
        self.cells[..self.len].contains(&key)
    }

    fn insert(&mut self, key: u32) {
        if self.contains(key) || self.len == self.cells.len() {
            return;
        }
        self.cells[self.len] = key;
        self.len += 1;
    }

    fn remove(&mut self, key: u32) {
        if let Some(i) = self.cells[..self.len].iter().position(|&c| c == key) {
            self.len -= 1;
            self.cells[i] = self.cells[self.len];
        }
    }

    fn clear(&mut self) {
        self.len = 0;
    }
}

/// The client's half of `occupy::Harvested`. Same question the server's
/// `SlotLives` answers, off a bare key set: a mirror needs no hit counts and
/// no respawn ticks, only whether the node is standing right now.
impl Harvested for HarvestedSet {
    fn is_harvested(&self, cx: u16, cz: u16) -> bool {
        self.contains(cell_key(cx, cz))
    }
}

/// The client's mirror of the placed-piece set, keyed by grid address —
/// bounded like the server's store (`MAX_PIECES`). Insertion is
/// idempotent (a piece can arrive by broadcast AND by the sync walk); an
/// insert past capacity is dropped, the same bounded posture as the
/// harvested set (the server refuses placements there too, so a full
/// client store only desyncs against a server bug, and the next resync
/// walk retries).
///
/// The mirror also keeps the predictor's collision index (`ColIndex`,
/// collide.rs) in lockstep. A record's collision shape comes from its
/// baked row, which drips separately (`PieceDefs`): rows not received
/// yet contribute no collision, and the defs-arrival handler rebuilds
/// the index — bounded staleness the next reconcile absorbs.
pub struct PieceSet {
    recs: Box<[PieceRec]>,
    len: usize,
    cols: Box<ColIndex>,
}

/// The collision shape of piece row `row`, if that row has dripped in.
/// Undripped rows are `PieceDef::INERT` (shape 0 = a plane!), so gating
/// on `have` is what keeps unknown rows out of the index.
fn shape_of(defs: &BuildContent, have: u16, row: u8) -> Option<u8> {
    if (row as u16) < have.min(defs.piece_count) {
        Some(defs.pieces[row as usize].shape)
    } else {
        None
    }
}

impl PieceSet {
    fn new() -> Self {
        Self {
            recs: vec![PieceRec::default(); MAX_PIECES].into_boxed_slice(),
            len: 0,
            cols: Box::new(ColIndex::new()),
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn entries(&self) -> &[PieceRec] {
        &self.recs[..self.len]
    }

    /// The predictor's collision view (movement::step's `cols`).
    pub fn cols(&self) -> &ColIndex {
        &self.cols
    }

    /// Set or clear a closed-door bit in the predictor's index. Doors
    /// live in the deploy mirror, but they seal *pieces*, so the bit
    /// belongs to this index — `ClientCore` is what keeps the two
    /// stores' views of a doorway in step (the sim does the same).
    fn set_door(&mut self, cx: u16, cz: u16, level: u8, loc: u8, shut: bool) {
        self.cols.set_door(cx, cz, level, loc, shut);
    }

    /// `set_door`'s twin for the solid-deployable nibbles (deploy
    /// collision v0): the furnace the sim walls off, the predictor walls
    /// off, through the same index the sim's own lockstep writes.
    fn set_solid(&mut self, cx: u16, cz: u16, level: u8, arch: Option<u8>) {
        self.cols.set_solid(cx, cz, level, arch);
    }

    /// Re-band the piece at an address, if one stands there (wire v44).
    ///
    /// **The sync walk is not enough on its own and this is the half that
    /// makes the field live.** A piece record reaches a client on join and
    /// on resync; nothing re-sends it because a wall lost hp. So without
    /// this, a base raided in front of you keeps drawing untouched until
    /// you relog — the band would be correct exactly once, at the moment
    /// you arrived, which is the worst kind of correct.
    ///
    /// `EV_STRUCT_HIT` already carries the address and the hp left, and
    /// `EV_PIECE_REPAIRED` the way back, so both edges are on the wire
    /// already and this costs no byte.
    fn set_dmg(&mut self, cx: u16, cz: u16, level: u8, loc: u8, dmg: u8) {
        for r in self.recs[..self.len].iter_mut() {
            if r.cx == cx && r.cz == cz && r.level == level && r.loc == loc {
                r.dmg = dmg;
                return;
            }
        }
    }

    /// True if the set changed (a known address with the same row is a
    /// duplicate, not a change).
    fn insert(&mut self, rec: PieceRec, defs: &BuildContent, have: u16) -> bool {
        for r in self.recs[..self.len].iter_mut() {
            if r.cx == rec.cx && r.cz == rec.cz && r.level == rec.level && r.loc == rec.loc {
                if *r == rec {
                    return false;
                }
                let old_row = r.row;
                *r = rec;
                if old_row != rec.row {
                    if let Some(shape) = shape_of(defs, have, old_row) {
                        self.cols.del(rec.cx, rec.cz, rec.level, rec.loc, shape);
                    }
                    if let Some(shape) = shape_of(defs, have, rec.row) {
                        self.cols.add(rec.cx, rec.cz, rec.level, rec.loc, shape);
                    }
                }
                return true;
            }
        }
        if self.len == self.recs.len() {
            return false;
        }
        self.recs[self.len] = rec;
        self.len += 1;
        if let Some(shape) = shape_of(defs, have, rec.row) {
            self.cols.add(rec.cx, rec.cz, rec.level, rec.loc, shape);
        }
        true
    }

    fn clear(&mut self) {
        self.len = 0;
        self.cols.clear();
    }

    fn remove(
        &mut self,
        cx: u16,
        cz: u16,
        level: u8,
        loc: u8,
        defs: &BuildContent,
        have: u16,
    ) -> bool {
        if let Some(i) = self.recs[..self.len]
            .iter()
            .position(|r| r.cx == cx && r.cz == cz && r.level == level && r.loc == loc)
        {
            if let Some(shape) = shape_of(defs, have, self.recs[i].row) {
                self.cols.del(cx, cz, level, loc, shape);
            }
            self.len -= 1;
            self.recs[i] = self.recs[self.len];
            true
        } else {
            false
        }
    }

    /// Rebuild the collision index from the records — the defs-arrival
    /// path, where rows that were unknown at insert time gain shapes.
    /// Event-lane cadence, never the render loop.
    fn rebuild_cols(&mut self, defs: &BuildContent, have: u16) {
        self.cols.clear();
        for r in self.recs[..self.len].iter() {
            if let Some(shape) = shape_of(defs, have, r.row) {
                self.cols.add(r.cx, r.cz, r.level, r.loc, shape);
            }
        }
    }
}

/// The client's mirror of the placed-deployable set — the same bounded,
/// address-keyed posture as `PieceSet` (`MAX_DEPLOYS`).
/// The client's mirror of the standing death backpacks (backpack.rs).
/// Bounded exactly like the server's store; a server-driven insert past
/// capacity is dropped, and the next join sync repairs it — the same
/// bounded staleness every other mirrored set here accepts.
pub struct BagSet {
    recs: Box<[WireBag]>,
    len: usize,
}

impl BagSet {
    fn new() -> Self {
        Self {
            recs: vec![WireBag::default(); MAX_BACKPACKS].into_boxed_slice(),
            len: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn entries(&self) -> &[WireBag] {
        &self.recs[..self.len]
    }

    fn clear(&mut self) {
        self.len = 0;
    }

    /// True if the set changed. Identity is the bag id, and a bag never
    /// moves, so a repeat is a no-op rather than an update.
    fn insert(&mut self, rec: WireBag) -> bool {
        if self.recs[..self.len].iter().any(|r| r.id == rec.id) {
            return false;
        }
        if self.len == self.recs.len() {
            return false;
        }
        self.recs[self.len] = rec;
        self.len += 1;
        true
    }

    fn remove(&mut self, id: u32) -> bool {
        if let Some(i) = self.recs[..self.len].iter().position(|r| r.id == id) {
            self.len -= 1;
            self.recs[i] = self.recs[self.len];
            true
        } else {
            false
        }
    }
}

pub struct DeploySet {
    recs: Box<[DeployRec]>,
    len: usize,
}

impl DeploySet {
    fn new() -> Self {
        Self {
            recs: vec![DeployRec::default(); MAX_DEPLOYS].into_boxed_slice(),
            len: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn entries(&self) -> &[DeployRec] {
        &self.recs[..self.len]
    }

    /// `PieceSet::set_dmg`'s twin — that one carries the reasoning.
    fn set_dmg(&mut self, cx: u16, cz: u16, level: u8, loc: u8, dmg: u8) {
        for r in self.recs[..self.len].iter_mut() {
            if r.cx == cx && r.cz == cz && r.level == level && r.loc == loc {
                r.dmg = dmg;
                return;
            }
        }
    }

    /// True if the set changed (same contract as `PieceSet::insert`).
    fn insert(&mut self, rec: DeployRec) -> bool {
        for r in self.recs[..self.len].iter_mut() {
            if r.cx == rec.cx && r.cz == rec.cz && r.level == rec.level && r.loc == rec.loc {
                if *r == rec {
                    return false;
                }
                *r = rec;
                return true;
            }
        }
        if self.len == self.recs.len() {
            return false;
        }
        self.recs[self.len] = rec;
        self.len += 1;
        true
    }

    /// The removed record, so the caller can unseal a doorway the gone
    /// door was holding shut.
    fn remove(&mut self, cx: u16, cz: u16, level: u8, loc: u8) -> Option<DeployRec> {
        if let Some(i) = self.recs[..self.len]
            .iter()
            .position(|r| r.cx == cx && r.cz == cz && r.level == level && r.loc == loc)
        {
            let gone = self.recs[i];
            self.len -= 1;
            self.recs[i] = self.recs[self.len];
            Some(gone)
        } else {
            None
        }
    }

    /// Apply a door announcement to the mirrored record; returns the
    /// record as it now stands, or None when this client has never heard
    /// of that address (the deploy walk will bring it, carrying state).
    /// `lock` is None where only the leaf moved (an optimistic toggle and
    /// its rollback never touch the lock); `Some((locked, has_lock))` is
    /// the pair the announcement carried, applied together because they
    /// are one fact about one lock (lock v1).
    fn set_open(
        &mut self,
        cx: u16,
        cz: u16,
        level: u8,
        loc: u8,
        open: bool,
        lock: Option<(bool, bool)>,
    ) -> Option<DeployRec> {
        let r = self.recs[..self.len]
            .iter_mut()
            .find(|r| r.cx == cx && r.cz == cz && r.level == level && r.loc == loc)?;
        r.open = open;
        if let Some((locked, has_lock)) = lock {
            r.locked = locked;
            r.has_lock = has_lock;
        }
        Some(*r)
    }

    fn clear(&mut self) {
        self.len = 0;
    }
}

/// The lit-fire set: addresses only, bounded by the sim's own container
/// cap, insertion-ordered, swap-removed. No allocation and no map — the
/// client is a hot path too (CLAUDE.md's trap list), and the set is
/// walked once per frame by the renderer at a size the sim has already
/// bounded.
pub struct LitOvens {
    addrs: [(u16, u16, u8); MAX_BOXES],
    len: usize,
}

impl LitOvens {
    fn new() -> Self {
        Self {
            addrs: [(0, 0, 0); MAX_BOXES],
            len: 0,
        }
    }

    /// Absolute, never a toggle — the event carries the state and this
    /// stores exactly what it carried, so two announcements crossing can
    /// never leave a client inverted (`EV_OVEN`'s own argument).
    fn set(&mut self, cx: u16, cz: u16, level: u8, lit: bool) {
        let at = self.addrs[..self.len]
            .iter()
            .position(|a| *a == (cx, cz, level));
        match (lit, at) {
            (true, None) => {
                if self.len < MAX_BOXES {
                    self.addrs[self.len] = (cx, cz, level);
                    self.len += 1;
                }
            }
            (false, Some(i)) => {
                self.len -= 1;
                self.addrs[i] = self.addrs[self.len];
            }
            _ => {}
        }
    }

    pub fn is_lit(&self, cx: u16, cz: u16, level: u8) -> bool {
        self.addrs[..self.len].contains(&(cx, cz, level))
    }

    pub fn addrs(&self) -> &[(u16, u16, u8)] {
        &self.addrs[..self.len]
    }

    fn clear(&mut self) {
        self.len = 0;
    }
}

impl ClientCore {
    /// The fires this client has heard are burning.
    pub fn ovens(&self) -> &LitOvens {
        &self.ovens
    }

    /// The island this client stands on, as the seed plus the same four
    /// borrows the predictor hands `movement::step` — including **the very
    /// `SlotCache` the predictor just filled**.
    ///
    /// That sharing is the whole point, and it is `gather::swing`'s argument
    /// one crate over: a swing reads the same nine cells at the same position
    /// on the same tick as the movement step that preceded it, so the lines it
    /// wants are already resolved. `ui::interact::resolve_swing` runs on the
    /// crosshair EVERY FRAME rather than once per swing, so a cold 3×3 scan
    /// there costs ~540 `noise2` at 60 Hz against the server's once per 38
    /// ticks (`NOW.md` §0sp) — the same defect the sim already fixed, on the
    /// hotter side of the wire.
    ///
    /// The seed comes off the predictor rather than from the caller for a
    /// reason the borrow checker cannot state: `SlotCache::slot` **flushes**
    /// when handed a seed it does not hold, so a caller free to pass its own
    /// could empty the predictor's lines every frame and turn the memo into a
    /// slower cold scan. `render::WorldId` keeps a second `seed`/`haven`/
    /// `table` triple derived the same way, and this is the copy that owns the
    /// cache.
    pub fn island(&mut self) -> (u64, Occupants<'_>) {
        (
            self.predict.seed(),
            Occupants {
                table: &self.scatter_table,
                haven: &self.haven,
                harvested: &self.harvested,
                cache: &mut self.slot_cache,
            },
        )
    }
}

/// True if deploy row `row` is a door **and this client knows it is**.
/// Rows that haven't dripped in yet read as arch 0, so the `have` gate
/// is what keeps an unknown row from sealing a doorway it doesn't own;
/// the defs-arrival handler re-derives once the rows land.
fn is_door(defs: &DeployContent, have: u16, row: u8) -> bool {
    (row as u16) < have.min(defs.def_count) && defs.defs[row as usize].arch == ARCH_DOOR
}

/// The archetype of a solid (movement-blocking) deploy row, or `None` —
/// `is_door`'s twin for the collision index's solid nibbles (deploy
/// collision v0). Same `have` gate, same consequence: a row that has not
/// dripped yet blocks nothing, and the defs-arrival handler re-derives.
/// Until then prediction under-blocks and the server corrects — the
/// bounded, self-healing staleness every mirror here accepts.
fn solid_arch(defs: &DeployContent, have: u16, row: u8) -> Option<u8> {
    if (row as u16) >= have.min(defs.def_count) {
        return None;
    }
    let arch = defs.defs[row as usize].arch;
    sim_core::deploy::solid_vol(arch).map(|_| arch)
}

/// What `on_datagram` did with the bytes (the bridge's status code).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Ingest {
    Error = 0,
    Applied = 1,
    AppliedDelta = 2,
    Stale = 3,
    NoBaseline = 4,
}

#[derive(Clone, Copy, Default)]
struct InputState {
    buttons: u8,
    yaw: u16,
    pitch: u8,
    move_x: i8,
    move_z: i8,
    sel: u8,
}

pub struct ClientCore {
    pub player_id: u32,
    pub view: ClientView,
    pub predict: Predictor,
    pub interp: Interp,
    pub clock: ClientClock,
    input: InputState,
    /// Buttons seen down at ANY point since the last input frame was built.
    ///
    /// **The render loop samples input; the sim consumes it on a fixed 30 Hz
    /// tick, and those two rates are not the same number.** `set_input`
    /// overwrites, so without this the only render frame that reached the sim
    /// was whichever one happened to be the last before a tick boundary — and
    /// every press that began and ended between two ticks was silently
    /// dropped. Measured before this existed: a normal ~50 ms click survived
    /// at 100 % on every framerate (which is why nobody noticed), but a press
    /// lasting one render frame survived at 100 % at 30 fps, 50 % at 60, and
    /// essentially by luck above that. The probability of losing a tap is
    /// roughly `1 - press_ms / 33.3`, so **the faster the client renders, the
    /// more likely a fast click is to vanish** — a frame at 400 fps is 2.5 ms
    /// and the tick that would carry it is 33 ms away.
    ///
    /// Sticky OR, consumed by the first tick that runs. That makes a tap
    /// exactly one tick of the button rather than none or two: the level
    /// (`input.buttons`) already carries a genuinely held button, so this only
    /// ever supplies a bit that was released before anyone looked.
    ///
    /// **Not sim state and not on the wire in any new way.** It changes which
    /// byte the client puts in a frame it was already sending, so the server
    /// sims what it receives exactly as before and the predictor uses the same
    /// frame it sends — no `PROTO_VER`, no goldens, no determinism surface.
    ///
    /// Deliberately buttons only. The move axes stay level-sampled: an axis
    /// has no impulse meaning, and synthesizing a whole tick of travel (~18 cm
    /// at run speed) out of a 2.5 ms tap invents motion the player never held.
    /// The view angles need nothing — `Look` integrates mouse delta into a
    /// free-running angle, so every pixel of motion between two ticks is
    /// already inside the value the tick samples.
    sticky_buttons: u8,
    next_seq: u16,
    input_due: bool,
    pub snapshots_applied: u64,
    pub snapshots_delta: u64,
    pub snapshots_stale: u64,
    pub snapshots_no_baseline: u64,
    pub decode_errors: u64,

    // --- event lane (reliable stream, protocol::event) ---
    /// Authoritative own inventory as last announced by the server.
    pub inv: [ItemStack; INV_SLOTS],
    /// The container this client has open as the server last said it —
    /// `CONT_SELF` for none, else the kind and the handle it was opened
    /// with, echoed back by every batch.
    ///
    /// The echo is what makes the panel safe to draw: a batch whose kind
    /// and handle do not match what this client believes is open is
    /// dropped, so contents can never land in a panel the player already
    /// closed or re-aimed. Two opens in flight and an open crossing a
    /// server-sent close both resolve that way rather than by trusting
    /// arrival order.
    pub cont_kind: u8,
    pub cont_handle: u32,
    /// The open container's slots. Sized to the widest container; a box
    /// fills the first `BOX_SLOTS` and the rest stay empty, which is what
    /// the server's shadow holds too.
    pub cont: [ItemStack; INV_SLOTS],
    pub harvested: HarvestedSet,
    /// The three parts the occupant collision query needs beyond the
    /// harvested mirror above (`sim_core::occupy`). They live here rather
    /// than in `Predictor` for the same reason `pieces` does: the predictor
    /// is handed its collision view, it does not own one.
    ///
    /// `haven` is resolved once here because `terrain::haven` is a bounded
    /// argmax over the whole road ring — measured at ~5.4 k height taps — and
    /// `scatter` needs it for every cell. Join-time cost, never a frame's.
    scatter_table: ScatterTable,
    haven: Haven,
    /// Boxed: 24 kB of fixed capacity against wasm's 1 MB shadow stack.
    slot_cache: Box<SlotCache>,
    pub catalog: ItemCatalog,
    /// Cell changes the last `on_stream` call produced: (key, harvested).
    slot_changes: [(u32, bool); protocol::SLOT_SYNC_BATCH],
    n_slot_changes: usize,
    toasts: [(u16, u16); TOAST_RING],
    toast_head: usize,
    toast_len: usize,
    /// Items the pack could not hold, buffered for the HUD (drop-oldest,
    /// cosmetic — the toast rings' posture exactly).
    ///
    /// One ring for both producers. `EV_GATHER` and `EV_CRAFT_DONE` both
    /// carry "units actually added", and a zero from either means the same
    /// thing to the player — *this did not fit and it is on the ground* —
    /// so the cause does not change the sentence or the action, and a
    /// discriminator nothing reads would be structure invented for its own
    /// sake. If a later pass wants a different cue per cause, the two arms
    /// that fill this are the place to widen it.
    ///
    /// Only the item index is kept, because only the item index is known:
    /// the wire carries what *reached the hands* and never what was paid,
    /// so the client can say which item spilled but not how much of it.
    /// Closing that half needs a wire field — `NOW.md` §0sp2 holds it as
    /// the operator's question and this ring is the half that did not.
    spills: [u16; TOAST_RING],
    spill_head: usize,
    spill_len: usize,
    /// The own weak-spot mark (server-announced, per-player): the node's
    /// cell (`NO_CELL` = none), the mark heading over the 256-entry yaw
    /// LUT, and whether the announcing hit landed weak.
    pub mark_cell: u32,
    pub mark8: u8,
    pub mark_weak_hit: bool,
    /// The recipe table as dripped so far (same rows the sim runs).
    pub recipes: CraftContent,
    /// Rows received so far; the table is complete when this reaches
    /// `recipes.recipe_count` (batches arrive in order).
    pub recipes_have: u16,
    /// The research table as dripped so far (tech tree v0) — the tree
    /// panel's data: each node's cost and its `requires` edge, in the
    /// same rows the sim runs.
    pub research: sim_core::research::ResearchContent,
    /// Rows received so far, `recipes_have`'s shape.
    pub research_have: u16,
    /// Authoritative own craft queue as last announced: (recipe,
    /// remaining) live jobs, head first, and the head unit's remaining
    /// ticks at announce time.
    pub jobs: [(u8, u8); CRAFT_QUEUE],
    pub jobs_count: u8,
    pub craft_eta_ticks: u16,
    craft_toasts: [(u16, u16); TOAST_RING],
    craft_toast_head: usize,
    craft_toast_len: usize,
    /// Blueprints this client's player knows, as the server last stated
    /// them (research v0). The **whole mask** every time, never a delta —
    /// `SUB_KNOWN` says why: a dropped success would otherwise grey a
    /// recipe the player paid for with no event left to correct it.
    known: u64,
    /// `(recipe, cost)` per blueprint learned, and the refusal reasons —
    /// both drop-oldest and cosmetic, `craft_toasts`' posture exactly.
    research_toasts: [(u16, u16); TOAST_RING],
    research_toast_head: usize,
    research_toast_len: usize,
    research_refusals: [u8; REFUSAL_RING],
    research_refusal_head: usize,
    research_refusal_len: usize,
    refusals: [u8; REFUSAL_RING],
    refusal_head: usize,
    refusal_len: usize,
    /// `(held item, reason)` per refused gather swing (wire v42) —
    /// drop-oldest and cosmetic, the craft refusal ring's posture. The
    /// item rides beside the reason because the sentence names it: *a
    /// torch cannot fell a tree*, never "bare hands" (`NOW.md` §0kit 2).
    gather_refusals: [(u16, u8); REFUSAL_RING],
    gather_refusal_head: usize,
    gather_refusal_len: usize,
    /// Chat lines as received: (speaker id, global, text).
    chats: [(u32, bool, ChatText); CHAT_RING],
    chat_head: usize,
    chat_len: usize,
    /// Own health, absolute as the server last stated it. `hp_max` 0 means
    /// no `Health` has arrived yet — a shard whose content disarms combat
    /// never sends one, and the HUD must not draw an empty bar for a
    /// player who simply cannot be hurt.
    pub hp: u16,
    pub hp_max: u16,
    /// Own food and water, absolute as the server last stated them.
    /// `max_food` 0 means no `Vitals` has arrived — a shard whose content
    /// has no `[survival]` section never sends one, and the HUD reads that
    /// as "no meters to draw" rather than as "starving".
    pub food: u16,
    pub water: u16,
    pub max_food: u16,
    pub max_water: u16,
    /// Eats that landed, oldest first: (item, the slot it was spent from).
    /// Drop-oldest, cosmetic — the toast rings' posture. Rings since
    /// 2026-08-15: as a field plus one bit (`last_eat` / `last_eat_refused`)
    /// two consume answers in one drain window collapsed — `Consumed` zeroed
    /// the reason and `ConsumeRefused` overwrote it — and one frame reaches
    /// that from the keyboard, because `KeyJ` and `KeyH` are two independent
    /// presses answered by one `World::tick`. Drained by `render/feed.rs`,
    /// worded by `render/hud.rs` off `Feed::consumed`.
    consume_toasts: [(u16, u16); TOAST_RING],
    consume_toast_head: usize,
    consume_toast_len: usize,
    /// Refused consumes (`sim_core::survival::REFUSE_C_*`), eat and drink
    /// both — one wire event answers the two verbs. Drop-oldest, cosmetic,
    /// `refusals`' posture exactly. Drained by `render/feed.rs` onto the
    /// shared refusal queue as `Refused::Consume`.
    consume_refusals: [u8; REFUSAL_RING],
    consume_refusal_head: usize,
    consume_refusal_len: usize,
    /// The last drink: water restored << 16 | hp it cost — the hp is how
    /// the HUD can name what took it. Read by `render/hud.rs::feedback` off
    /// `APPLIED_DRANK` (`client_drank`, named here until 2026-08-15, was
    /// the deleted C-ABI bridge).
    pub last_drink: u32,
    /// The last move's address, `sim_core::inventory::addr`'s pack
    /// verbatim, and the refusal reason (0 = the move landed). Read
    /// together by `render/panels/mod.rs::sync_refusals` and the panel's
    /// rollback (`client_move_readout`, named here until 2026-08-15, was
    /// the deleted C-ABI bridge).
    ///
    /// The **slot contents are deliberately not applied here.** The server
    /// diffs the whole inventory against its last-acked copy every tick
    /// and pushes the changed slots as `EventMsg::Inv` (`server/core.rs`),
    /// so a move that touched this body's inventory arrives as authoritative
    /// slots on the same lane a beat later — applying it twice would be two
    /// writers on one mirror, which is a divergence rather than a
    /// prediction. What the panel needs from *this* event is the thing the
    /// inventory diff cannot say: whether the drag it drew was accepted,
    /// and if not, which drag to roll back.
    pub last_move: u32,
    pub last_move_refused: u8,
    /// The accepted move's payload: count << 16 | the item that left the
    /// source slot. Zero on a refusal. The item is the panel's reconcile
    /// hook — an id it did not predict means its picture of the container
    /// had drifted, so it redraws rather than trusting the drag it drew.
    pub last_move_count: u32,
    /// Own landed hits, oldest first: `(victim, damage dealt)`. The
    /// hitmarker ring.
    ///
    /// **The victim was thrown away until 2026-08-18** — the `EV_HIT` arm
    /// read `let _ = victim; // v0 marks the hit, not who took it` — which
    /// meant the client knew a blow had landed and not on whom, and the
    /// renderer could not flinch the body that took it. The field was
    /// already on the wire, so keeping it costs no packet and no
    /// `PROTO_VER` bump.
    ///
    /// [`NO_VICTIM`] for the structure half of this ring, which names an
    /// address rather than a person.
    hits: [(u32, u16); TOAST_RING],
    hit_head: usize,
    hit_len: usize,
    /// Deaths as broadcast, oldest first: (victim, killer) — the kill feed.
    deaths: [(u32, u32); TOAST_RING],
    death_head: usize,
    death_len: usize,
    /// Killer of the death `pop_death` returned last, so one pop hands the
    /// caller a whole line (the `removed_key`/`removed_info` pattern).
    pub last_death_killer: u32,
    /// The death screen, own-body only. `dead` is set by the `Death` whose
    /// victim is `player_id` and cleared by the `Respawn` that answers it;
    /// the four beside it are the sentence the screen says (ALPHA.md §1:
    /// "who/what killed you — range and weapon, no map position"), held
    /// rather than ringed because there is only ever one of them and the
    /// screen is up until it is answered.
    ///
    /// `woke_on_bag` is the last respawn's anchor, and it outlives `dead`
    /// on purpose: a player who asked for a bag and got a beach is told so
    /// *after* the screen closes, which is the only moment the fact is
    /// worth anything.
    pub dead: bool,
    pub own_death_killer: u32,
    pub own_death_cause: u8,
    pub own_death_item: u16,
    pub own_death_range_cm: u16,
    pub woke_on_bag: bool,
    /// **The bags this player has placed**, as the server last stated
    /// them, with each one's cooldown state at send time (wire v43).
    ///
    /// Its own field rather than something derived off `deploys`, because
    /// it cannot be derived: `DeployRec::owner` is deliberately not on the
    /// wire, so the deploy mirror knows where every bed on the island is
    /// and not which are this player's. `SUB_BAGS` has the whole argument.
    ///
    /// Read through `own_bags()`. Replaced whole by every message, never
    /// merged, so a bag a raider destroyed cannot survive in it.
    ///
    /// ⚠ **Not `own_bag` below, and the two are a letter apart.** This is
    /// the set of SLEEPING bags you placed — where you may wake. That is
    /// the death BACKPACK that fell where you died — where your stuff is.
    /// The tree calls both of them bags (`ARCH_BAG` is the deployable,
    /// `WireBag` is the corpse's container) and that predates both fields;
    /// the plural is the anchor, the singular is the loot.
    own_bags: [BagAnchor; BAG_CAP],
    own_bags_count: usize,
    /// The bag that stood where THIS body died — `WireBag::id`, zero for
    /// none (the store's own "no bag" sentinel). The wire deliberately
    /// carries no bag owner (`BackpackRec::owner` is sim-side only), so
    /// this is a client-side join, exact in everything the client knows:
    /// a `BagDropped` that lands while `dead`, before any other has since
    /// this death, within [`OWN_BAG_NEAR_M`] of the predicted body — which
    /// while dead IS the death position, since a dead body neither moves
    /// nor predicts. A neighbour dying on the same spot inside the same
    /// window can mis-tag; the cost is one map mark ranked as yours, and
    /// the alternative is a wire field `ALPHA.md` §1 reserves as an
    /// operator call. Cleared by the bag's own `BagRemoved`, NOT by
    /// respawn — walking back to it is the point.
    ///
    /// ⚠ See `own_bags` above for why these two nearly-identical names are
    /// different things.
    pub own_bag: u32,
    /// Armed by the `Death` naming this body, spent by the first
    /// qualifying `BagDropped`: the latch that keeps a stranger's later
    /// bag from re-tagging while the screen is up.
    own_bag_pending: bool,
    /// The placed-piece mirror (address-keyed; the renderer's truth).
    pub pieces: PieceSet,
    /// Piece records the last `on_stream` call added or replaced.
    piece_changes: [PieceRec; protocol::PIECE_SYNC_BATCH],
    n_piece_changes: usize,
    /// The piece-def table as dripped so far (same rows the sim runs).
    pub piece_defs: BuildContent,
    /// Rows received so far (batches arrive in order).
    pub piece_defs_have: u16,
    build_refusals: [u8; REFUSAL_RING],
    build_refusal_head: usize,
    build_refusal_len: usize,
    /// The placed-deployable mirror (address-keyed; the renderer's truth).
    pub deploys: DeploySet,
    /// Standing death backpacks, server truth (no prediction: a bag
    /// appears when a body falls, which the client cannot foresee).
    pub bags: BagSet,
    /// Deployable records the last `on_stream` call added or replaced.
    deploy_changes: [DeployRec; protocol::DEPLOY_SYNC_BATCH],
    n_deploy_changes: usize,
    /// The deploy-def table as dripped so far (same rows the sim runs).
    pub deploy_defs: DeployContent,
    /// Rows received so far (batches arrive in order).
    pub deploy_defs_have: u16,
    deploy_refusals: [u8; REFUSAL_RING],
    /// Knocks heard this frame (lock v1): address + who. Broadcast, so
    /// this ring is the *only* one here that can carry somebody else's
    /// action — the mixer wants it positional and the HUD wants to say
    /// somebody is at your door.
    knocks: [(u16, u16, u8, u8, u32); REFUSAL_RING],
    knock_head: usize,
    knock_len: usize,
    /// Shots seen this frame (wire v33): shooter, yaw, pitch, speed and
    /// drop in mm/tick. Broadcast like `knocks`, and for the same reason —
    /// an arrow in the air is somebody else's action that this client has
    /// to draw.
    ///
    /// **Purely cosmetic, and that is load-bearing.** Nothing downstream
    /// of this ring may decide anything: the arrow that matters is the
    /// server's, and its hit arrives on `EV_HIT` whether or not a tracer
    /// was ever drawn. A dropped entry costs one streak of motion.
    shots: [(u32, u16, u8, u16, u16); REFUSAL_RING],
    shot_head: usize,
    shot_len: usize,
    /// Where arrows stopped, in the wire's quanta, with the surface kind
    /// (`sim_core::ranged::SURF_*`). Drop-oldest and purely cosmetic.
    impacts: [(i32, i32, i32, u8); IMPACT_RING],
    impact_head: usize,
    impact_len: usize,
    swings: [u32; SWING_RING],
    swing_head: usize,
    swing_len: usize,
    /// Grants this client earned (lock v1): address + `lock::GRANT_*`. An
    /// own-fact, and the only thing that tells a client its code landed —
    /// the door itself does not move on a correct code.
    auths: [(u16, u16, u8, u8, u8); REFUSAL_RING],
    auth_head: usize,
    auth_len: usize,
    /// Placements that HAPPENED (`PiecePlaced`/`DeployPlaced` broadcasts):
    /// address + which store (`true` = deployable). **Never fed by a sync
    /// walk**, and that asymmetry is the ring's whole reason to exist: the
    /// walk *restates* the world (a join streams every standing piece, a
    /// resync streams them again), while the broadcast is the server saying
    /// one just went down — so the place cue's producer can read this and
    /// stay silent through a join flood with no timer knob deciding when
    /// the flood is over. The same-tick duplicate delivery (a broadcast is
    /// followed by the walk's tail batch carrying the same record) cannot
    /// double-ring, because the mirror insert is idempotent and only a
    /// successful insert rings.
    placed: [(u16, u16, u8, u8, bool); TOAST_RING],
    placed_head: usize,
    placed_len: usize,
    deploy_refusal_head: usize,
    deploy_refusal_len: usize,
    /// Which ovens this client has heard are lit, by address.
    ///
    /// Its own store rather than a bit on the mirrored `DeployRec`, and
    /// that is the same argument the sim makes one layer down: the deploy
    /// record is what the deploy-sync packet carries, the burn state
    /// deliberately is not on it, and a client that wrote `lit` into the
    /// record would lose every fire the moment a resync walked the store
    /// and overwrote it with the server's own (unlit) copy. Keyed by
    /// address, so a resync leaves it standing and the next toggle still
    /// lands on the right fire.
    ovens: LitOvens,
    /// The address of a door this client toggled optimistically on its
    /// own input and has not heard back about (NETCODE.md §6.1: your own
    /// door plays on input, remote doors on the event). At most one is
    /// outstanding, so a prediction never compounds. The next action
    /// outcome resolves it: the door announcement confirms (absolute
    /// state, so it corrects just as well), any deploy refusal rolls it
    /// back, and an `ev_resync` walk re-derives the lot. A refusal that
    /// belonged to some *other* queued action rolls this door back early
    /// — harmless, because a use that does land announces right behind.
    pending_door: Option<(u16, u16, u8, u8)>,
    /// The last removal's grid address (valid while the flags say a
    /// removal was applied this message).
    pub removed_addr: (u16, u16, u8, u8),
    /// The last structure hit: (cx, cz, level, loc, hp left, hp max).
    /// `max` is resolved from the def table the client already holds, so
    /// the HUD can draw a fraction without the wire carrying one; a hit on
    /// a row the defs have not arrived for yet reports `max = 0`, and the
    /// caller draws nothing rather than a lie.
    pub struct_hit: (u16, u16, u8, u8, u16, u16),
    /// The last charge planted: (cx, cz, level, loc, row, fuse ticks). The
    /// store bit rides in `charge_deploy` beside it rather than inside the
    /// tuple, because a caller drawing a countdown on a wall needs the
    /// address first and the store only to pick which mesh to stick it to.
    pub charge_placed: (u16, u16, u8, u8, u8, u16),
    pub charge_deploy: bool,
    /// The last stock ack: hearth address, rows, live row count.
    pub stock_addr: (u16, u16, u8),
    pub stock: [(u16, u32); HEARTH_STOCK_ROWS],
    pub stock_count: u8,
    /// The `APPLIED2_*` word for the last `on_stream` call, read back by
    /// `applied2()`. Rebuilt from zero on every call for the same reason
    /// `n_slot_changes` is: it describes one message, not a running state,
    /// and a verdict that outlived its message would be read as a fresh one.
    applied2: u32,
    pub events_applied: u64,
    pub event_errors: u64,
}

impl ClientCore {
    pub fn new(seed: u64, player_id: u32, server_tick: u32) -> Self {
        Self {
            player_id,
            view: ClientView::new(),
            predict: Predictor::new(seed),
            interp: Interp::new(),
            clock: ClientClock::new(server_tick),
            input: InputState::default(),
            sticky_buttons: 0,
            next_seq: 1,
            input_due: false,
            snapshots_applied: 0,
            snapshots_delta: 0,
            snapshots_stale: 0,
            snapshots_no_baseline: 0,
            decode_errors: 0,
            inv: [ItemStack::default(); INV_SLOTS],
            cont_kind: CONT_SELF,
            cont_handle: 0,
            cont: [ItemStack::default(); INV_SLOTS],
            harvested: HarvestedSet::new(),
            scatter_table: ScatterTable::alpha_default(),
            haven: terrain::haven(seed),
            slot_cache: Box::new(SlotCache::new()),
            catalog: ItemCatalog::EMPTY,
            slot_changes: [(0, false); protocol::SLOT_SYNC_BATCH],
            n_slot_changes: 0,
            toasts: [(0, 0); TOAST_RING],
            toast_head: 0,
            toast_len: 0,
            spills: [0; TOAST_RING],
            spill_head: 0,
            spill_len: 0,
            mark_cell: NO_CELL,
            mark8: 0,
            mark_weak_hit: false,
            recipes: CraftContent::EMPTY,
            recipes_have: 0,
            research: sim_core::research::ResearchContent::EMPTY,
            research_have: 0,
            jobs: [(0, 0); CRAFT_QUEUE],
            jobs_count: 0,
            craft_eta_ticks: 0,
            craft_toasts: [(0, 0); TOAST_RING],
            craft_toast_head: 0,
            craft_toast_len: 0,
            refusals: [0; REFUSAL_RING],
            chats: [(0, false, ChatText::EMPTY); CHAT_RING],
            chat_head: 0,
            chat_len: 0,
            hp: 0,
            hp_max: 0,
            food: 0,
            water: 0,
            max_food: 0,
            max_water: 0,
            consume_toasts: [(0, 0); TOAST_RING],
            consume_toast_head: 0,
            consume_toast_len: 0,
            consume_refusals: [0; REFUSAL_RING],
            consume_refusal_head: 0,
            consume_refusal_len: 0,
            last_drink: 0,
            last_move: 0,
            last_move_refused: 0,
            last_move_count: 0,
            hits: [(NO_VICTIM, 0); TOAST_RING],
            hit_head: 0,
            hit_len: 0,
            deaths: [(0, 0); TOAST_RING],
            death_head: 0,
            death_len: 0,
            last_death_killer: 0,
            dead: false,
            own_death_killer: 0,
            own_death_cause: 0,
            own_death_item: sim_core::gather::NO_ITEM,
            own_death_range_cm: 0,
            woke_on_bag: false,
            own_bags: [BagAnchor::default(); BAG_CAP],
            own_bags_count: 0,
            own_bag: 0,
            own_bag_pending: false,
            refusal_head: 0,
            refusal_len: 0,
            pieces: PieceSet::new(),
            piece_changes: [PieceRec::default(); protocol::PIECE_SYNC_BATCH],
            n_piece_changes: 0,
            piece_defs: BuildContent::EMPTY,
            piece_defs_have: 0,
            build_refusals: [0; REFUSAL_RING],
            build_refusal_head: 0,
            build_refusal_len: 0,
            deploys: DeploySet::new(),
            bags: BagSet::new(),
            deploy_changes: [DeployRec::default(); protocol::DEPLOY_SYNC_BATCH],
            n_deploy_changes: 0,
            deploy_defs: DeployContent::EMPTY,
            deploy_defs_have: 0,
            deploy_refusals: [0; REFUSAL_RING],
            knocks: [(0, 0, 0, 0, 0); REFUSAL_RING],
            shots: [(0, 0, 0, 0, 0); REFUSAL_RING],
            shot_head: 0,
            shot_len: 0,
            impacts: [(0, 0, 0, 0); IMPACT_RING],
            impact_head: 0,
            impact_len: 0,
            swings: [0; SWING_RING],
            swing_head: 0,
            swing_len: 0,
            knock_head: 0,
            knock_len: 0,
            auths: [(0, 0, 0, 0, 0); REFUSAL_RING],
            auth_head: 0,
            auth_len: 0,
            placed: [(0, 0, 0, 0, false); TOAST_RING],
            placed_head: 0,
            placed_len: 0,
            deploy_refusal_head: 0,
            deploy_refusal_len: 0,
            ovens: LitOvens::new(),
            pending_door: None,
            removed_addr: (0, 0, 0, 0),
            struct_hit: (0, 0, 0, 0, 0, 0),
            charge_placed: (0, 0, 0, 0, 0, 0),
            charge_deploy: false,
            stock_addr: (0, 0, 0),
            stock: [(0, 0); HEARTH_STOCK_ROWS],
            stock_count: 0,
            applied2: 0,
            known: 0,
            research_toasts: [(0, 0); TOAST_RING],
            research_toast_head: 0,
            research_toast_len: 0,
            research_refusals: [0; REFUSAL_RING],
            gather_refusals: [(0, 0); REFUSAL_RING],
            gather_refusal_head: 0,
            gather_refusal_len: 0,
            research_refusal_head: 0,
            research_refusal_len: 0,
            events_applied: 0,
            event_errors: 0,
        }
    }

    /// The bags this player has placed, as the server last stated them.
    ///
    /// Empty is a real answer and the one the death screen changes shape
    /// for: no bag placed, no bag to offer, and the only way back is a
    /// beach. It is also the state before the first `Bags` message of a
    /// session, which is the same picture for the same reason — a client
    /// that has not been told it owns a bag must not claim one.
    pub fn own_bags(&self) -> &[BagAnchor] {
        &self.own_bags[..self.own_bags_count]
    }

    /// Whether any own bag's cooldown had lapsed as of the last `Bags`
    /// message. Advisory: the bit ages (see `APPLIED2_BAGS`), and the sim
    /// is the authority either way.
    pub fn any_bag_ready(&self) -> bool {
        self.own_bags().iter().any(|b| b.ready)
    }

    /// The `APPLIED2_*` flags the last `on_stream` call set — word 1 of
    /// the applied word, which word 0 has no spare bit to announce. Valid
    /// until the next `on_stream`, like `slot_changes()`.
    pub fn applied2(&self) -> u32 {
        self.applied2
    }

    /// One event-lane message off the reliable stream. Returns the
    /// `APPLIED_*` flags saying what changed; cell-level detail for the
    /// renderer is in `slot_changes()` until the next call.
    pub fn on_stream(&mut self, bytes: &[u8]) -> Result<u32, WireError> {
        self.n_slot_changes = 0;
        self.n_piece_changes = 0;
        self.n_deploy_changes = 0;
        self.applied2 = 0;
        let msg = match decode_event(bytes) {
            Ok(m) => m,
            Err(e) => {
                self.event_errors += 1;
                return Err(e);
            }
        };
        self.events_applied += 1;
        let mut flags = 0u32;
        match msg {
            EventMsg::Gather { item, added } => {
                if added > 0 {
                    if self.toast_len == TOAST_RING {
                        // Drop oldest: advance the head.
                        self.toast_head = (self.toast_head + 1) % TOAST_RING;
                        self.toast_len -= 1;
                    }
                    self.toasts[(self.toast_head + self.toast_len) % TOAST_RING] = (item, added);
                    self.toast_len += 1;
                    flags |= APPLIED_TOAST;
                } else {
                    // The pack was full. `sim-core` only pushes this event
                    // for a payout it actually owed (`gather.rs`'s `pay > 0`,
                    // `backpack.rs`'s `took == 0` skip), so the zero is the
                    // spill and not a swing that earned nothing.
                    self.push_spill(item);
                }
            }
            EventMsg::GatherRefused { item, reason } => {
                if self.gather_refusal_len == REFUSAL_RING {
                    self.gather_refusal_head = (self.gather_refusal_head + 1) % REFUSAL_RING;
                    self.gather_refusal_len -= 1;
                }
                self.gather_refusals
                    [(self.gather_refusal_head + self.gather_refusal_len) % REFUSAL_RING] =
                    (item, reason);
                self.gather_refusal_len += 1;
                flags |= APPLIED_TOAST;
            }
            EventMsg::Inv { slots, count } => {
                for s in slots.iter().take(count as usize) {
                    self.inv[s.slot as usize] = s.stack;
                }
                flags |= APPLIED_INV;
            }
            EventMsg::SlotHarvested { cx, cz } => {
                self.harvested.insert(cell_key(cx, cz));
                self.push_change(cell_key(cx, cz), true);
                flags |= APPLIED_SLOTS | self.clear_mark_if(cell_key(cx, cz));
            }
            EventMsg::SlotRespawned { cx, cz } => {
                self.harvested.remove(cell_key(cx, cz));
                self.push_change(cell_key(cx, cz), false);
                flags |= APPLIED_SLOTS | self.clear_mark_if(cell_key(cx, cz));
            }
            EventMsg::SlotSync {
                reset,
                cells,
                count,
            } => {
                if reset {
                    self.harvested.clear();
                    flags |= APPLIED_RESET | self.clear_mark_if(self.mark_cell);
                }
                for &(cx, cz) in cells.iter().take(count as usize) {
                    self.harvested.insert(cell_key(cx, cz));
                    self.push_change(cell_key(cx, cz), true);
                    flags |= self.clear_mark_if(cell_key(cx, cz));
                }
                flags |= APPLIED_SLOTS;
            }
            EventMsg::WeakMark {
                cx,
                cz,
                mark8,
                weak_hit,
            } => {
                self.mark_cell = cell_key(cx, cz);
                self.mark8 = mark8;
                self.mark_weak_hit = weak_hit;
                flags |= APPLIED_MARK;
            }
            EventMsg::Catalog {
                total,
                first,
                count,
                names,
                lens,
                cond_max,
            } => {
                self.catalog.count = total as u16;
                for i in 0..count as usize {
                    // Server-sent lens are wire-validated ≤ the cap.
                    let _ = self.catalog.set(
                        first as usize + i,
                        &names[i][..lens[i] as usize],
                        cond_max[i],
                    );
                }
                flags |= APPLIED_CATALOG;
            }
            EventMsg::CraftQ {
                jobs,
                count,
                eta_ticks,
            } => {
                self.jobs = jobs;
                self.jobs_count = count;
                self.craft_eta_ticks = eta_ticks;
                flags |= APPLIED_CRAFT_Q;
            }
            EventMsg::CraftDone { item, added } => {
                if added == 0 {
                    // A finished unit that could not fit. This used to fall
                    // through to the toast ring, and the HUD formatted it
                    // straight out as `crafted 0 × Stone Hatchet` — a line
                    // that told the player the craft had failed when it had
                    // succeeded and was lying on the floor. Gather's arm has
                    // always guarded its zero; this one now does too.
                    self.push_spill(item);
                } else {
                    if self.craft_toast_len == TOAST_RING {
                        self.craft_toast_head = (self.craft_toast_head + 1) % TOAST_RING;
                        self.craft_toast_len -= 1;
                    }
                    self.craft_toasts
                        [(self.craft_toast_head + self.craft_toast_len) % TOAST_RING] =
                        (item, added);
                    self.craft_toast_len += 1;
                    flags |= APPLIED_CRAFT_DONE;
                }
            }
            EventMsg::CraftRefused { reason } => {
                if self.refusal_len == REFUSAL_RING {
                    self.refusal_head = (self.refusal_head + 1) % REFUSAL_RING;
                    self.refusal_len -= 1;
                }
                self.refusals[(self.refusal_head + self.refusal_len) % REFUSAL_RING] = reason;
                self.refusal_len += 1;
                flags |= APPLIED_CRAFT_REFUSED;
            }
            EventMsg::Known { mask } => {
                self.known = mask;
                self.applied2 |= APPLIED2_RESEARCH;
            }
            EventMsg::Research { recipe, cost } => {
                if self.research_toast_len == TOAST_RING {
                    self.research_toast_head = (self.research_toast_head + 1) % TOAST_RING;
                    self.research_toast_len -= 1;
                }
                self.research_toasts
                    [(self.research_toast_head + self.research_toast_len) % TOAST_RING] =
                    (recipe, cost);
                self.research_toast_len += 1;
                // The mask is NOT set here, deliberately: `SUB_KNOWN`
                // follows on the same tick and is the authority. Setting
                // the bit locally would work right up until a content
                // hotfix moved the recipe indices under a client that had
                // been told one number and inferred another.
                self.applied2 |= APPLIED2_RESEARCH;
            }
            EventMsg::ResearchRefused { reason } => {
                if self.research_refusal_len == REFUSAL_RING {
                    self.research_refusal_head = (self.research_refusal_head + 1) % REFUSAL_RING;
                    self.research_refusal_len -= 1;
                }
                self.research_refusals
                    [(self.research_refusal_head + self.research_refusal_len) % REFUSAL_RING] =
                    reason;
                self.research_refusal_len += 1;
                self.applied2 |= APPLIED2_RESEARCH;
            }
            EventMsg::Recipes {
                total,
                first,
                count,
                rows,
            } => {
                self.recipes.recipe_count = total as u16;
                for (i, row) in rows.iter().enumerate().take(count as usize) {
                    self.recipes.recipes[first as usize + i] = *row;
                }
                self.recipes_have = self.recipes_have.max(first as u16 + count as u16);
                flags |= APPLIED_RECIPES;
            }
            EventMsg::ResearchRows {
                total,
                first,
                count,
                coin,
                rows,
            } => {
                self.research.row_count = total as u16;
                self.research.coin = coin;
                for (i, row) in rows.iter().enumerate().take(count as usize) {
                    self.research.rows[first as usize + i] = *row;
                }
                self.research_have = self.research_have.max(first as u16 + count as u16);
                // The recipes flag, deliberately: every reader of one
                // table reads the other (the craft panel greys, the tree
                // panel prices), so a second bit would be a second way to
                // spell the same redraw.
                flags |= APPLIED_RECIPES;
            }
            EventMsg::PiecePlaced { rec } => {
                if self
                    .pieces
                    .insert(rec, &self.piece_defs, self.piece_defs_have)
                {
                    self.push_piece_change(rec);
                    // A broadcast is a placement HAPPENING; only it rings
                    // (see `placed`). The walk's tail batch carrying the
                    // same record fails the insert above and stays silent.
                    self.push_placed(rec.cx, rec.cz, rec.level, rec.loc, false);
                    flags |= APPLIED_PIECES;
                }
            }
            EventMsg::PieceSync { reset, recs, count } => {
                if reset {
                    self.pieces.clear();
                    flags |= APPLIED_PIECE_RESET;
                }
                for &rec in recs.iter().take(count as usize) {
                    if self
                        .pieces
                        .insert(rec, &self.piece_defs, self.piece_defs_have)
                    {
                        self.push_piece_change(rec);
                        flags |= APPLIED_PIECES;
                    }
                }
                if reset {
                    // The clear took every door's shut bit with it.
                    self.apply_doors();
                }
            }
            EventMsg::BuildRefused { reason } => {
                if self.build_refusal_len == REFUSAL_RING {
                    self.build_refusal_head = (self.build_refusal_head + 1) % REFUSAL_RING;
                    self.build_refusal_len -= 1;
                }
                self.build_refusals
                    [(self.build_refusal_head + self.build_refusal_len) % REFUSAL_RING] = reason;
                self.build_refusal_len += 1;
                flags |= APPLIED_BUILD_REFUSED;
            }
            EventMsg::PieceDefs {
                total,
                first,
                count,
                rows,
            } => {
                self.piece_defs.piece_count = total as u16;
                for (i, row) in rows.iter().enumerate().take(count as usize) {
                    self.piece_defs.pieces[first as usize + i] = *row;
                }
                self.piece_defs_have = self.piece_defs_have.max(first as u16 + count as u16);
                // Records that arrived before their rows had no collision
                // shape yet — the new rows may name them. The rebuild
                // clears the index, doors included, so they go back on.
                self.pieces
                    .rebuild_cols(&self.piece_defs, self.piece_defs_have);
                self.apply_doors();
                flags |= APPLIED_PIECE_DEFS;
            }
            EventMsg::DeployPlaced { rec } => {
                if self.deploys.insert(rec) {
                    self.seal_for(rec);
                    self.push_deploy_change(rec);
                    // Broadcast, not walk — rings for the same reason the
                    // `PiecePlaced` arm does.
                    self.push_placed(rec.cx, rec.cz, rec.level, rec.loc, true);
                    flags |= APPLIED_DEPLOYS;
                }
            }
            EventMsg::DeploySync { reset, recs, count } => {
                if reset {
                    // Every door this client knew is now unknown: drop
                    // their bits and let the walk re-seal what it brings.
                    // The walk is server truth, so an outstanding
                    // prediction has nothing left to roll back onto.
                    self.pending_door = None;
                    self.deploys.clear();
                    // The fires go out with them, and that is the honest
                    // reading rather than a loss: a deploy reset means
                    // this client's picture of the furniture is being
                    // rebuilt from the server's, and the walk carries no
                    // burn state, so keeping a lit address across it
                    // would be keeping a fire that may no longer stand.
                    // The next toggle or self-snuff re-lights it, which
                    // is at most one fuel unit away.
                    self.ovens.clear();
                    self.pieces
                        .rebuild_cols(&self.piece_defs, self.piece_defs_have);
                    flags |= APPLIED_DEPLOY_RESET;
                }
                for &rec in recs.iter().take(count as usize) {
                    if self.deploys.insert(rec) {
                        self.seal_for(rec);
                        self.push_deploy_change(rec);
                        flags |= APPLIED_DEPLOYS;
                    }
                }
            }
            EventMsg::BagDropped { id, qx, qy, qz } => {
                if self.bags.insert(WireBag { id, qx, qy, qz }) {
                    flags |= APPLIED_BAGS;
                }
                // The own-bag join (`own_bag`'s field doc): while dead the
                // predicted body is the death position, and the server
                // stands the bag at the body it killed, so the first drop
                // since this death that lands beside it is this body's.
                if self.own_bag_pending && self.dead {
                    let dx = (qx - self.predict.body.qx) as f32 * POS_XZ_Q;
                    let dz = (qz - self.predict.body.qz) as f32 * POS_XZ_Q;
                    if dx * dx + dz * dz <= OWN_BAG_NEAR_M * OWN_BAG_NEAR_M {
                        self.own_bag = id;
                        self.own_bag_pending = false;
                    }
                }
            }
            EventMsg::BagSync { reset, recs, count } => {
                if reset {
                    self.bags.clear();
                    flags |= APPLIED_BAGS;
                }
                for &rec in recs.iter().take(count as usize) {
                    if self.bags.insert(rec) {
                        flags |= APPLIED_BAGS;
                    }
                }
            }
            EventMsg::ContSync {
                kind,
                cont,
                reset,
                slots,
                count,
            } => {
                if kind == CONT_SELF {
                    // The server shut the panel: gone, or out of reach.
                    self.cont_kind = CONT_SELF;
                    self.cont_handle = 0;
                    self.cont = [ItemStack::default(); INV_SLOTS];
                    self.applied2 |= APPLIED2_CONT;
                } else if reset {
                    // The whole container, and the only message that may
                    // change which container is open — so a diff can never
                    // silently re-aim the panel at something else.
                    self.cont_kind = kind;
                    self.cont_handle = cont;
                    self.cont = [ItemStack::default(); INV_SLOTS];
                    for s in slots.iter().take(count as usize) {
                        self.cont[s.slot as usize] = s.stack;
                    }
                    self.applied2 |= APPLIED2_CONT;
                } else if kind == self.cont_kind && cont == self.cont_handle {
                    for s in slots.iter().take(count as usize) {
                        self.cont[s.slot as usize] = s.stack;
                    }
                    self.applied2 |= APPLIED2_CONT;
                }
                // else: a diff for a container this client no longer has
                // open. Dropped rather than applied — the alternative is
                // slots landing in a panel aimed somewhere else, which is
                // the positional-payload defect one level up.
            }
            EventMsg::BagRemoved { id, why: _ } => {
                // The reason is on the wire for a feed line the HUD does
                // not have yet; the set only cares that it is gone.
                if self.bags.remove(id) {
                    flags |= APPLIED_BAGS;
                }
                if self.own_bag == id {
                    self.own_bag = 0; // looted out, expired or evicted
                }
            }
            EventMsg::DeployRefused { reason } => {
                // Refusals are sender-only, so this one is *this client's*
                // — but not necessarily this door's: the action ring holds
                // 8, so an earlier deploy or feed can bounce while a door
                // toggle is still queued. Rolling back on either is what
                // keeps a genuinely refused use from sticking; a
                // mis-attributed rollback self-heals, because a use that
                // does land announces absolute state right behind it.
                if self.rollback_door() {
                    flags |= APPLIED_DEPLOYS;
                }
                if self.deploy_refusal_len == REFUSAL_RING {
                    self.deploy_refusal_head = (self.deploy_refusal_head + 1) % REFUSAL_RING;
                    self.deploy_refusal_len -= 1;
                }
                self.deploy_refusals
                    [(self.deploy_refusal_head + self.deploy_refusal_len) % REFUSAL_RING] = reason;
                self.deploy_refusal_len += 1;
                flags |= APPLIED_DEPLOY_REFUSED;
            }
            EventMsg::DeployDefs {
                total,
                first,
                count,
                rows,
            } => {
                self.deploy_defs.def_count = total as u16;
                for (i, row) in rows.iter().enumerate().take(count as usize) {
                    self.deploy_defs.defs[first as usize + i] = *row;
                }
                self.deploy_defs_have = self.deploy_defs_have.max(first as u16 + count as u16);
                // Records whose row was unknown never sealed anything;
                // the new rows may say they are doors.
                self.apply_doors();
                flags |= APPLIED_DEPLOY_DEFS;
            }
            EventMsg::StructHit {
                deploy,
                cx,
                cz,
                level,
                loc,
                damage,
                left,
            } => {
                // The raid's own hitmarker: the same ring a body hit uses,
                // because "my swing landed for N" is the same fact. It is
                // the one entry with no victim — a wall is not a person and
                // has nothing to flinch — so it rides `NO_VICTIM`.
                if self.hit_len == TOAST_RING {
                    self.hit_head = (self.hit_head + 1) % TOAST_RING;
                    self.hit_len -= 1;
                }
                self.hits[(self.hit_head + self.hit_len) % TOAST_RING] = (NO_VICTIM, damage);
                self.hit_len += 1;
                let addressed =
                    |r: &(u16, u16, u8, u8)| r.0 == cx && r.1 == cz && r.2 == level && r.3 == loc;
                let max = if deploy {
                    self.deploys
                        .entries()
                        .iter()
                        .find(|r| addressed(&(r.cx, r.cz, r.level, r.loc)))
                        .filter(|r| (r.row as u16) < self.deploy_defs_have)
                        .map(|r| self.deploy_defs.defs[r.row as usize].hp)
                } else {
                    self.pieces
                        .entries()
                        .iter()
                        .find(|r| addressed(&(r.cx, r.cz, r.level, r.loc)))
                        .filter(|r| (r.row as u16) < self.piece_defs_have)
                        .map(|r| self.piece_defs.pieces[r.row as usize].hp)
                };
                self.struct_hit = (cx, cz, level, loc, left, max.unwrap_or(0));
                // …and re-band the mirror, so the wall the player is
                // watching come apart actually comes apart (`set_dmg`).
                // An unknown maximum bands to 0 by `damage_band`'s own
                // rule rather than guessing a fraction of nothing.
                let band = sim_core::build::damage_band(left, max.unwrap_or(0));
                if deploy {
                    self.deploys.set_dmg(cx, cz, level, loc, band);
                } else {
                    self.pieces.set_dmg(cx, cz, level, loc, band);
                }
                // Both flags on purpose: `HIT` is the hitmarker fact and
                // owns draining the ring, `STRUCT_HIT` adds where it
                // landed. One flag would either strand the ring or make
                // every caller of the marker learn about addresses.
                flags |= APPLIED_HIT | APPLIED_STRUCT_HIT;
            }
            EventMsg::PieceRepaired {
                // `StructHit` reads its bit to pick which store to look a
                // maximum up in. This one has nothing to look up — `hp` is
                // both halves of the pair by construction, because the
                // verb's whole contract is that a repaired structure
                // stands at its baked row's hp and never a point over — so
                // the bit is ignored here and the readout means the same
                // thing for a door as for the doorway it stands in.
                deploy: _,
                cx,
                cz,
                level,
                loc,
                row: _,
                healed: _,
                hp,
            } => {
                // The same readout `StructHit` writes, from the other
                // direction — and no hitmarker, so `APPLIED_HIT` stays
                // off: nobody was struck.
                self.struct_hit = (cx, cz, level, loc, hp, hp);
                // Band 0 without consulting `damage_band`, and that is not
                // a shortcut: the verb's whole contract is that a repaired
                // structure stands at its baked row's hp and never a point
                // over, which is the same reason the `deploy` bit is
                // ignored above. `damage_band(hp, hp)` is 0 by definition.
                self.deploys.set_dmg(cx, cz, level, loc, 0);
                self.pieces.set_dmg(cx, cz, level, loc, 0);
                flags |= APPLIED_STRUCT_HIT;
            }
            EventMsg::ChargePlaced {
                deploy,
                cx,
                cz,
                level,
                loc,
                row,
                fuse,
            } => {
                // Recorded, not predicted, and it touches neither store: a
                // charge is not a piece and not a deployable, so nothing
                // here adds to a mirror that a sync walk would then
                // disagree with. It is a fact with a clock on it, and the
                // renderer's job is to draw the clock running down.
                self.charge_placed = (cx, cz, level, loc, row, fuse);
                self.charge_deploy = deploy;
                self.applied2 |= APPLIED2_CHARGE;
            }
            EventMsg::PieceRemoved { cx, cz, level, loc } => {
                if self
                    .pieces
                    .remove(cx, cz, level, loc, &self.piece_defs, self.piece_defs_have)
                {
                    self.removed_addr = (cx, cz, level, loc);
                    flags |= APPLIED_PIECE_REMOVED;
                }
            }
            EventMsg::DeployRemoved { cx, cz, level, loc } => {
                if self.pending_door == Some((cx, cz, level, loc)) {
                    self.pending_door = None; // predicted a door that's gone
                }
                if let Some(gone) = self.deploys.remove(cx, cz, level, loc) {
                    if is_door(&self.deploy_defs, self.deploy_defs_have, gone.row) {
                        self.pieces.set_door(cx, cz, level, loc, false);
                    }
                    if solid_arch(&self.deploy_defs, self.deploy_defs_have, gone.row).is_some() {
                        self.pieces.set_solid(cx, cz, level, None);
                    }
                    self.removed_addr = (cx, cz, level, loc);
                    flags |= APPLIED_DEPLOY_REMOVED;
                }
            }
            EventMsg::Door {
                cx,
                cz,
                level,
                loc,
                open,
                locked,
                has_lock,
            } => {
                // Absolute state, all three bits: this confirms an
                // optimistic toggle or corrects it, and either way the
                // wait is over.
                if self.pending_door == Some((cx, cz, level, loc)) {
                    self.pending_door = None;
                }
                if let Some(rec) =
                    self.deploys
                        .set_open(cx, cz, level, loc, open, Some((locked, has_lock)))
                {
                    self.seal_for(rec);
                    self.push_deploy_change(rec);
                    flags |= APPLIED_DEPLOYS;
                }
            }
            EventMsg::Oven {
                cx,
                cz,
                level,
                lit,
                by: _,
            } => {
                // Absolute state, like the door's. The actor is on the
                // wire and deliberately unread here: what a toast should
                // say about someone else's fire is a UI question, and the
                // core's job is the fact.
                self.ovens.set(cx, cz, level, lit);
                flags |= APPLIED_DEPLOYS;
            }
            EventMsg::Vitals {
                food,
                water,
                max_food,
                max_water,
            } => {
                // Absolute like `Health`, and for the same reason — a
                // missed reading repairs itself on the next one, so the
                // HUD never runs its own drain timer.
                self.food = food;
                self.water = water;
                self.max_food = max_food;
                self.max_water = max_water;
                flags |= APPLIED_VITALS;
            }
            EventMsg::Consumed { item, slot } => {
                if self.consume_toast_len == TOAST_RING {
                    self.consume_toast_head = (self.consume_toast_head + 1) % TOAST_RING;
                    self.consume_toast_len -= 1;
                }
                self.consume_toasts
                    [(self.consume_toast_head + self.consume_toast_len) % TOAST_RING] =
                    (item, slot as u16);
                self.consume_toast_len += 1;
                flags |= APPLIED_CONSUME;
            }
            EventMsg::ConsumeRefused { reason } => {
                if self.consume_refusal_len == REFUSAL_RING {
                    self.consume_refusal_head = (self.consume_refusal_head + 1) % REFUSAL_RING;
                    self.consume_refusal_len -= 1;
                }
                self.consume_refusals
                    [(self.consume_refusal_head + self.consume_refusal_len) % REFUSAL_RING] =
                    reason;
                self.consume_refusal_len += 1;
                flags |= APPLIED_CONSUME;
            }
            EventMsg::Moved {
                from_kind,
                from_slot,
                to_kind,
                to_slot,
                count,
                item,
            } => {
                self.last_move = sim_core::inventory::addr(from_kind, from_slot, to_kind, to_slot);
                self.last_move_refused = 0;
                self.last_move_count = ((count as u32) << 16) | item as u32;
                self.applied2 |= APPLIED2_MOVE;
            }
            EventMsg::MoveRefused {
                reason,
                from_kind,
                from_slot,
                to_kind,
                to_slot,
            } => {
                self.last_move = sim_core::inventory::addr(from_kind, from_slot, to_kind, to_slot);
                self.last_move_refused = reason;
                self.last_move_count = 0;
                self.applied2 |= APPLIED2_MOVE;
            }
            EventMsg::Drank { water, hp_cost } => {
                self.last_drink = ((water as u32) << 16) | hp_cost as u32;
                flags |= APPLIED_DRANK;
            }
            EventMsg::Health { hp, max } => {
                // Absolute, so a missed one repairs itself. Max travels
                // with every reading: the bar never has to guess its
                // denominator from content it does not have.
                self.hp = hp;
                self.hp_max = max;
                flags |= APPLIED_HEALTH;
            }
            EventMsg::Hit { victim, damage } => {
                // **The victim is kept, and it used to be dropped here.**
                // `EV_HIT` is unicast to the attacker, so this names the
                // body *your* blow landed on — which is the whole of what
                // `render::bodies` needs to flinch it. It may be an animal
                // (`mob::mob_id`): `mobs::stream` draws those and carries
                // no `BodyAnim`, so the id simply matches nothing.
                if self.hit_len == TOAST_RING {
                    self.hit_head = (self.hit_head + 1) % TOAST_RING;
                    self.hit_len -= 1;
                }
                self.hits[(self.hit_head + self.hit_len) % TOAST_RING] = (victim, damage);
                self.hit_len += 1;
                flags |= APPLIED_HIT;
            }
            EventMsg::Death {
                victim,
                killer,
                cause,
                item,
                range_cm,
            } => {
                // Drop-oldest, like chat: a feed that stalls on the oldest
                // kill is worse than one that loses it.
                if self.death_len == TOAST_RING {
                    self.death_head = (self.death_head + 1) % TOAST_RING;
                    self.death_len -= 1;
                }
                self.deaths[(self.death_head + self.death_len) % TOAST_RING] = (victim, killer);
                self.death_len += 1;
                flags |= APPLIED_DEATH;
                // …and if it was this body, the screen. Held outside the
                // feed ring because the ring is drop-oldest and a death
                // screen that could be dropped by two strangers dying
                // nearby would strand the player behind an overlay with no
                // sentence on it.
                if victim == self.player_id {
                    self.dead = true;
                    self.own_bag_pending = true;
                    self.own_death_killer = killer;
                    self.own_death_cause = cause;
                    self.own_death_item = item;
                    self.own_death_range_cm = range_cm;
                    flags |= APPLIED_RESPAWN;
                }
            }
            EventMsg::Respawn { on_bag } => {
                self.dead = false;
                self.woke_on_bag = on_bag;
                flags |= APPLIED_RESPAWN;
            }
            EventMsg::Shot {
                shooter,
                yaw,
                pitch,
                speed_mmpt,
                drop_mmpt2,
            } => {
                // Drop-oldest, the knock ring's policy: under a volley
                // worth more than `REFUSAL_RING`, the newest tracers are
                // the ones still worth drawing.
                if self.shot_len == REFUSAL_RING {
                    self.shot_head = (self.shot_head + 1) % REFUSAL_RING;
                    self.shot_len -= 1;
                }
                self.shots[(self.shot_head + self.shot_len) % REFUSAL_RING] =
                    (shooter, yaw, pitch, speed_mmpt, drop_mmpt2);
                self.shot_len += 1;
            }
            EventMsg::Impact { qx, qy, qz, surf } => {
                // Drop-oldest, the shot ring's policy and its reason: when
                // more marks arrive than a frame can take, the newest are
                // the ones still near enough to look at.
                //
                // **Nothing is validated here and nothing needs to be.**
                // The decoder already refused a surface kind this build
                // does not know and a point outside the island's window,
                // and a mark is read by exactly one system that draws it —
                // no rule, no prediction and no state hangs off this.
                if self.impact_len == IMPACT_RING {
                    self.impact_head = (self.impact_head + 1) % IMPACT_RING;
                    self.impact_len -= 1;
                }
                self.impacts[(self.impact_head + self.impact_len) % IMPACT_RING] =
                    (qx, qy, qz, surf);
                self.impact_len += 1;
            }
            EventMsg::Swing { swinger } => {
                // Drop-oldest, and nothing is validated: an id that names
                // no body simply matches nothing when `render::bodies`
                // walks its live set, which is the same answer it gives a
                // body that left the interest band mid-arc.
                if self.swing_len == SWING_RING {
                    self.swing_head = (self.swing_head + 1) % SWING_RING;
                    self.swing_len -= 1;
                }
                self.swings[(self.swing_head + self.swing_len) % SWING_RING] = swinger;
                self.swing_len += 1;
            }
            EventMsg::Knock {
                cx,
                cz,
                level,
                loc,
                by,
            } => {
                // Drop-oldest, like every other ring here: a knock that
                // stalls the newest one is worse than one that is lost,
                // and a knock is a sound rather than a state change.
                if self.knock_len == REFUSAL_RING {
                    self.knock_head = (self.knock_head + 1) % REFUSAL_RING;
                    self.knock_len -= 1;
                }
                self.knocks[(self.knock_head + self.knock_len) % REFUSAL_RING] =
                    (cx, cz, level, loc, by);
                self.knock_len += 1;
            }
            EventMsg::Auth {
                cx,
                cz,
                level,
                loc,
                grant,
            } => {
                if self.auth_len == REFUSAL_RING {
                    self.auth_head = (self.auth_head + 1) % REFUSAL_RING;
                    self.auth_len -= 1;
                }
                self.auths[(self.auth_head + self.auth_len) % REFUSAL_RING] =
                    (cx, cz, level, loc, grant);
                self.auth_len += 1;
            }
            EventMsg::Chat { from, global, text } => {
                // Drop-oldest: a chat log that stalls on the oldest line
                // is worse than one that loses it.
                if self.chat_len == CHAT_RING {
                    self.chat_head = (self.chat_head + 1) % CHAT_RING;
                    self.chat_len -= 1;
                }
                self.chats[(self.chat_head + self.chat_len) % CHAT_RING] = (from, global, text);
                self.chat_len += 1;
                flags |= APPLIED_CHAT;
            }
            EventMsg::Stock {
                cx,
                cz,
                level,
                rows,
                count,
            } => {
                self.stock_addr = (cx, cz, level);
                self.stock = rows;
                self.stock_count = count;
                flags |= APPLIED_STOCK;
            }
            // The whole list every time, so this is a replace and never a
            // merge — `Known`'s posture, and for the same reason: a client
            // that accumulated would offer a bag a raider took.
            EventMsg::Bags { bags, count } => {
                self.own_bags = bags;
                self.own_bags_count = (count as usize).min(BAG_CAP);
                self.applied2 |= APPLIED2_BAGS;
            }
        }
        // Bit 31 is the bridge's error sentinel and shares this return
        // channel; a flag that reached it would read to JS as a decode
        // failure. `applied_word_is_full_and_bit_31_is_the_error_sentinel`
        // proves no constant can, this catches a raw literal.
        debug_assert_eq!(flags & STREAM_ERR, 0, "an applied-flag hit bit 31");
        Ok(flags)
    }

    /// Clear the mark when `key` is its node; returns the flag that says
    /// the mark changed (0 otherwise). `NO_CELL` never matches.
    fn clear_mark_if(&mut self, key: u32) -> u32 {
        if self.mark_cell != NO_CELL && self.mark_cell == key {
            self.mark_cell = NO_CELL;
            self.mark_weak_hit = false;
            APPLIED_MARK
        } else {
            0
        }
    }

    fn push_change(&mut self, key: u32, harvested: bool) {
        if self.n_slot_changes < self.slot_changes.len() {
            self.slot_changes[self.n_slot_changes] = (key, harvested);
            self.n_slot_changes += 1;
        }
    }

    /// Cell changes from the last applied message (renderer detail).
    pub fn slot_changes(&self) -> &[(u32, bool)] {
        &self.slot_changes[..self.n_slot_changes]
    }

    fn push_piece_change(&mut self, rec: PieceRec) {
        if self.n_piece_changes < self.piece_changes.len() {
            self.piece_changes[self.n_piece_changes] = rec;
            self.n_piece_changes += 1;
        }
    }

    /// Piece records the last applied message added (renderer detail).
    pub fn piece_changes(&self) -> &[PieceRec] {
        &self.piece_changes[..self.n_piece_changes]
    }

    /// Toggle the door at the address **optimistically**, on this
    /// client's own input, before the server has spoken — NETCODE.md
    /// §6.1: your own door plays on input, remote doors on the event.
    /// The mirror and the predictor's shut bit flip together, so the body
    /// you are predicting walks through the door you just opened instead
    /// of holding at the threshold for half an RTT.
    ///
    /// Returns the predicted open state, or None when there is nothing to
    /// predict: no known door at that address, or a prediction already
    /// outstanding (one at a time — the next action outcome resolves it).
    /// Refusing to predict is always safe; the announcement still lands.
    pub fn predict_door(&mut self, cx: u16, cz: u16, level: u8, loc: u8) -> Option<bool> {
        if self.pending_door.is_some() {
            return None;
        }
        let rec = *self
            .deploys
            .entries()
            .iter()
            .find(|r| r.cx == cx && r.cz == cz && r.level == level && r.loc == loc)?;
        if !is_door(&self.deploy_defs, self.deploy_defs_have, rec.row) {
            return None;
        }
        let rec = self.deploys.set_open(cx, cz, level, loc, !rec.open, None)?;
        self.seal_for(rec);
        self.pending_door = Some((cx, cz, level, loc));
        Some(rec.open)
    }

    /// Undo an outstanding optimistic toggle — the sim refused the use
    /// (out of reach, or the door went away under it), so the door never
    /// moved. Server truth wins; the mirror goes back where it was.
    ///
    /// True when a record actually moved, so the caller raises
    /// `APPLIED_DEPLOYS` — the renderer swung that leaf on the press and
    /// nothing else will ever tell it to swing back (no announcement is
    /// coming: the server's state never changed).
    fn rollback_door(&mut self) -> bool {
        let Some((cx, cz, level, loc)) = self.pending_door.take() else {
            return false;
        };
        let Some(rec) = self
            .deploys
            .entries()
            .iter()
            .find(|r| r.cx == cx && r.cz == cz && r.level == level && r.loc == loc)
            .copied()
        else {
            return false; // the record left the mirror; nothing to put back
        };
        match self.deploys.set_open(cx, cz, level, loc, !rec.open, None) {
            Some(rec) => {
                self.seal_for(rec);
                self.push_deploy_change(rec);
                true
            }
            None => false,
        }
    }

    /// One record's contribution to the predictor's collision index: a
    /// closed door seals its doorway, a solid body deploy walls its cell
    /// (deploy collision v0). An open door — and every archetype that is
    /// neither — contributes nothing.
    fn seal_for(&mut self, rec: DeployRec) {
        if is_door(&self.deploy_defs, self.deploy_defs_have, rec.row) {
            self.pieces
                .set_door(rec.cx, rec.cz, rec.level, rec.loc, !rec.open);
        }
        if let Some(arch) = solid_arch(&self.deploy_defs, self.deploy_defs_have, rec.row) {
            self.pieces.set_solid(rec.cx, rec.cz, rec.level, Some(arch));
        }
    }

    /// Re-seal every closed door and re-wall every solid deploy in the
    /// mirror. The collision index is derived state, so anything that
    /// clears or rebuilds it drops the door and solid bits along with the
    /// piece bits — this puts them back. One bounded pass over the deploy
    /// mirror, event-lane cadence only, never the render loop.
    fn apply_doors(&mut self) {
        let Self {
            pieces,
            deploys,
            deploy_defs,
            deploy_defs_have,
            ..
        } = self;
        for rec in deploys.entries() {
            if is_door(deploy_defs, *deploy_defs_have, rec.row) {
                pieces.set_door(rec.cx, rec.cz, rec.level, rec.loc, !rec.open);
            }
            if let Some(arch) = solid_arch(deploy_defs, *deploy_defs_have, rec.row) {
                pieces.set_solid(rec.cx, rec.cz, rec.level, Some(arch));
            }
        }
    }

    fn push_deploy_change(&mut self, rec: DeployRec) {
        if self.n_deploy_changes < self.deploy_changes.len() {
            self.deploy_changes[self.n_deploy_changes] = rec;
            self.n_deploy_changes += 1;
        }
    }

    /// Deployable records the last applied message added (renderer detail).
    pub fn deploy_changes(&self) -> &[DeployRec] {
        &self.deploy_changes[..self.n_deploy_changes]
    }

    /// Oldest buffered deploy refusal reason
    /// (`sim_core::deploy::REFUSE_D_*`).
    pub fn pop_deploy_refusal(&mut self) -> Option<u8> {
        if self.deploy_refusal_len == 0 {
            return None;
        }
        let r = self.deploy_refusals[self.deploy_refusal_head];
        self.deploy_refusal_head = (self.deploy_refusal_head + 1) % REFUSAL_RING;
        self.deploy_refusal_len -= 1;
        Some(r)
    }

    /// Oldest buffered knock: the door's address and who knocked on it.
    pub fn pop_knock(&mut self) -> Option<(u16, u16, u8, u8, u32)> {
        if self.knock_len == 0 {
            return None;
        }
        let k = self.knocks[self.knock_head];
        self.knock_head = (self.knock_head + 1) % REFUSAL_RING;
        self.knock_len -= 1;
        Some(k)
    }

    /// Oldest buffered shot: who fired, where they aimed, and the round's
    /// speed and drop in mm/tick.
    ///
    /// **Single-consumer, like every `pop_*` here** — CLAUDE.md's
    /// clean-merge trap is exactly this ring shape, so the one caller is
    /// `render::feed::drain` and a second reader takes `Res<Feed>`.
    pub fn pop_shot(&mut self) -> Option<(u32, u16, u8, u16, u16)> {
        if self.shot_len == 0 {
            return None;
        }
        let s = self.shots[self.shot_head];
        self.shot_head = (self.shot_head + 1) % REFUSAL_RING;
        self.shot_len -= 1;
        Some(s)
    }

    /// Oldest buffered impact: where an arrow stopped, in the wire's
    /// quanta (3 cm x/z, 1 cm y), and what it stopped on
    /// (`sim_core::ranged::SURF_*`).
    ///
    /// **Single-consumer, like every `pop_*` here.** `render::decal::mark`
    /// is the one caller and a second reader takes `Res<Feed>` — the
    /// clean-merge trap in CLAUDE.md is this exact ring shape, and
    /// `tests/sound.rs` derives its verb list from this file so a ring
    /// added and never drained reddens on its own.
    pub fn pop_impact(&mut self) -> Option<(i32, i32, i32, u8)> {
        if self.impact_len == 0 {
            return None;
        }
        let i = self.impacts[self.impact_head];
        self.impact_head = (self.impact_head + 1) % IMPACT_RING;
        self.impact_len -= 1;
        Some(i)
    }

    /// Oldest buffered swing: the id of a body whose arm started to move.
    ///
    /// **Single-consumer, like every `pop_*` here.** `render::feed::drain`
    /// is the one caller and `render::bodies::stream` reads the drained
    /// slice through `Res<Feed>` — the clean-merge trap in CLAUDE.md is
    /// this exact ring shape, and `tests/sound.rs` derives its verb list
    /// from this file, so a ring added and never drained reddens on its
    /// own and a second reader reddens too.
    pub fn pop_swing(&mut self) -> Option<u32> {
        if self.swing_len == 0 {
            return None;
        }
        let s = self.swings[self.swing_head];
        self.swing_head = (self.swing_head + 1) % SWING_RING;
        self.swing_len -= 1;
        Some(s)
    }

    /// Oldest buffered grant: the lock's address and what it now allows
    /// this client (`sim_core::lock::GRANT_*`).
    pub fn pop_auth(&mut self) -> Option<(u16, u16, u8, u8, u8)> {
        if self.auth_len == 0 {
            return None;
        }
        let a = self.auths[self.auth_head];
        self.auth_head = (self.auth_head + 1) % REFUSAL_RING;
        self.auth_len -= 1;
        Some(a)
    }

    /// Drop-oldest, like the knock ring and for its reason: a placement is
    /// a sound rather than a state change (the mirror already holds the
    /// state), so stalling the newest would be worse than losing one.
    fn push_placed(&mut self, cx: u16, cz: u16, level: u8, loc: u8, deploy: bool) {
        if self.placed_len == TOAST_RING {
            self.placed_head = (self.placed_head + 1) % TOAST_RING;
            self.placed_len -= 1;
        }
        self.placed[(self.placed_head + self.placed_len) % TOAST_RING] =
            (cx, cz, level, loc, deploy);
        self.placed_len += 1;
    }

    /// Oldest buffered placement broadcast: address + which store (`true` =
    /// deployable). Only a `PiecePlaced`/`DeployPlaced` broadcast rings —
    /// never a sync walk, so a join or resync restating the world hands
    /// over nothing here (see the `placed` field).
    pub fn pop_placed(&mut self) -> Option<(u16, u16, u8, u8, bool)> {
        if self.placed_len == 0 {
            return None;
        }
        let p = self.placed[self.placed_head];
        self.placed_head = (self.placed_head + 1) % TOAST_RING;
        self.placed_len -= 1;
        Some(p)
    }

    /// Oldest buffered build refusal reason (`sim_core::build::REFUSE_B_*`).
    pub fn pop_build_refusal(&mut self) -> Option<u8> {
        if self.build_refusal_len == 0 {
            return None;
        }
        let r = self.build_refusals[self.build_refusal_head];
        self.build_refusal_head = (self.build_refusal_head + 1) % REFUSAL_RING;
        self.build_refusal_len -= 1;
        Some(r)
    }

    /// Oldest buffered hitmarker, if any: `(victim, damage)` this client's
    /// blow dealt. The victim is [`NO_VICTIM`] when the thing struck was a
    /// wall rather than a body — see the `hits` field.
    pub fn pop_hit(&mut self) -> Option<(u32, u16)> {
        if self.hit_len == 0 {
            return None;
        }
        let h = self.hits[self.hit_head];
        self.hit_head = (self.hit_head + 1) % TOAST_RING;
        self.hit_len -= 1;
        Some(h)
    }

    /// Oldest buffered death, if any: the victim's id, with the killer
    /// left in `last_death_killer` so one pop yields a whole feed line.
    pub fn pop_death(&mut self) -> Option<u32> {
        if self.death_len == 0 {
            return None;
        }
        let (victim, killer) = self.deaths[self.death_head];
        self.death_head = (self.death_head + 1) % TOAST_RING;
        self.death_len -= 1;
        self.last_death_killer = killer;
        Some(victim)
    }

    /// Oldest buffered gather toast, if any: (item index, units added).
    pub fn pop_toast(&mut self) -> Option<(u16, u16)> {
        if self.toast_len == 0 {
            return None;
        }
        let t = self.toasts[self.toast_head];
        self.toast_head = (self.toast_head + 1) % TOAST_RING;
        self.toast_len -= 1;
        Some(t)
    }

    /// Buffer an item the pack could not hold, and raise `APPLIED2_SPILL`.
    ///
    /// Private and shared by the two arms that can see a zero, so neither
    /// can drift from the other on the drop-oldest rule.
    fn push_spill(&mut self, item: u16) {
        if self.spill_len == TOAST_RING {
            self.spill_head = (self.spill_head + 1) % TOAST_RING;
            self.spill_len -= 1;
        }
        self.spills[(self.spill_head + self.spill_len) % TOAST_RING] = item;
        self.spill_len += 1;
        self.applied2 |= APPLIED2_SPILL;
    }

    /// Oldest buffered spill, if any: the item index that did not fit.
    ///
    /// Destructive, single-consumer — `render/feed.rs` owns the drain and
    /// `tests/sound.rs` greps for any second call site.
    pub fn pop_spill(&mut self) -> Option<u16> {
        if self.spill_len == 0 {
            return None;
        }
        let s = self.spills[self.spill_head];
        self.spill_head = (self.spill_head + 1) % TOAST_RING;
        self.spill_len -= 1;
        Some(s)
    }

    /// Oldest buffered craft-done toast: (item index, units added).
    pub fn pop_craft_toast(&mut self) -> Option<(u16, u16)> {
        if self.craft_toast_len == 0 {
            return None;
        }
        let t = self.craft_toasts[self.craft_toast_head];
        self.craft_toast_head = (self.craft_toast_head + 1) % TOAST_RING;
        self.craft_toast_len -= 1;
        Some(t)
    }

    /// Oldest buffered chat line: (speaker id, global, text).
    pub fn pop_chat(&mut self) -> Option<(u32, bool, ChatText)> {
        if self.chat_len == 0 {
            return None;
        }
        let c = self.chats[self.chat_head];
        self.chat_head = (self.chat_head + 1) % CHAT_RING;
        self.chat_len -= 1;
        Some(c)
    }

    /// Blueprints known, as the server last stated them. The craft panel
    /// asks this per row; `sim_core::research::knows` is the one place the
    /// shift is written, so nothing here re-implements it.
    pub fn known(&self) -> u64 {
        self.known
    }

    /// Oldest buffered `(recipe, cost)` learned.
    pub fn pop_research_toast(&mut self) -> Option<(u16, u16)> {
        if self.research_toast_len == 0 {
            return None;
        }
        let t = self.research_toasts[self.research_toast_head];
        self.research_toast_head = (self.research_toast_head + 1) % TOAST_RING;
        self.research_toast_len -= 1;
        Some(t)
    }

    /// Oldest buffered research refusal (`sim_core::research::REFUSE_R_*`).
    pub fn pop_research_refusal(&mut self) -> Option<u8> {
        if self.research_refusal_len == 0 {
            return None;
        }
        let r = self.research_refusals[self.research_refusal_head];
        self.research_refusal_head = (self.research_refusal_head + 1) % REFUSAL_RING;
        self.research_refusal_len -= 1;
        Some(r)
    }

    /// Oldest buffered gather refusal: `(held item, reason)` —
    /// `sim_core::gather::REFUSE_G_*`, item `NO_ITEM` = bare hands.
    pub fn pop_gather_refusal(&mut self) -> Option<(u16, u8)> {
        if self.gather_refusal_len == 0 {
            return None;
        }
        let r = self.gather_refusals[self.gather_refusal_head];
        self.gather_refusal_head = (self.gather_refusal_head + 1) % REFUSAL_RING;
        self.gather_refusal_len -= 1;
        Some(r)
    }

    /// Oldest buffered landed eat: (item index, the slot it was spent from).
    ///
    /// Single-consumer like every `pop_*` here: `render::feed::drain` is the
    /// one caller and `render/hud.rs` words it off `Feed::consumed`.
    pub fn pop_consume_toast(&mut self) -> Option<(u16, u16)> {
        if self.consume_toast_len == 0 {
            return None;
        }
        let t = self.consume_toasts[self.consume_toast_head];
        self.consume_toast_head = (self.consume_toast_head + 1) % TOAST_RING;
        self.consume_toast_len -= 1;
        Some(t)
    }

    /// Oldest buffered consume refusal (`sim_core::survival::REFUSE_C_*`),
    /// eat and drink both — one wire event answers the two verbs.
    pub fn pop_consume_refusal(&mut self) -> Option<u8> {
        if self.consume_refusal_len == 0 {
            return None;
        }
        let r = self.consume_refusals[self.consume_refusal_head];
        self.consume_refusal_head = (self.consume_refusal_head + 1) % REFUSAL_RING;
        self.consume_refusal_len -= 1;
        Some(r)
    }

    /// Oldest buffered craft refusal reason (`sim_core::craft::REFUSE_*`).
    pub fn pop_craft_refusal(&mut self) -> Option<u8> {
        if self.refusal_len == 0 {
            return None;
        }
        let r = self.refusals[self.refusal_head];
        self.refusal_head = (self.refusal_head + 1) % REFUSAL_RING;
        self.refusal_len -= 1;
        Some(r)
    }

    /// The live input state; sampled once per generated frame. `sel`
    /// clamps into the hotbar (the encoder refuses 6+ outright).
    pub fn set_input(&mut self, buttons: u8, yaw: u16, pitch: u8, move_x: i8, move_z: i8, sel: u8) {
        // See `sticky_buttons`: the level is overwritten, the presses are
        // remembered. Without the OR, a press that begins and ends between
        // two 30 Hz ticks never reaches the sim at all.
        self.sticky_buttons |= buttons;
        self.input = InputState {
            buttons,
            yaw,
            pitch,
            move_x,
            move_z,
            sel: sel.min(HOTBAR_SLOTS as u8 - 1),
        };
    }

    /// The button bits this client is currently **sending** — the exact
    /// byte `advance` stamps into every input frame.
    ///
    /// Read by the viewmodel's swing cadence, and read from here rather
    /// than from the render layer's own copy of the mouse on purpose: what
    /// the sim acts on is this byte, after every decision `input::gather`
    /// already made — a panel eating the click (buttons go out zeroed), the
    /// held-item modality that turns a left click into a placement, the
    /// death screen standing the corpse down. A second reading of the mouse
    /// would animate a swing in every one of those cases.
    pub fn buttons(&self) -> u8 {
        self.input.buttons
    }

    /// Advance real time: run the fixed client ticks that elapsed, each
    /// generating one input frame and stepping prediction. Returns steps.
    ///
    /// **This is also where the reconciliation offset decays, and it lives
    /// here rather than at the caller because a caller forgot.**
    /// `Predictor::decay_error` is documented "call once per render frame",
    /// and until 2026-08-17 the only things that called it were the eight
    /// `server/tests/*_wire.rs` harnesses — each hand-rolling the line with
    /// the comment *"the render loop's once-per-frame call"*. The actual
    /// render loop (`render::input::gather` → `Session::pump` → here) never
    /// did. `err` is written `+=` on every mispredicted reconcile
    /// (`predict.rs`) and nothing drained it, so the shipping client
    /// accumulated every correction it had ever taken into a permanent
    /// offset between the camera (`render_position`) and the body the world
    /// actually collides against — bounded only by `SNAP_AT_M`'s 4 m zero,
    /// which is a teleport rather than a fix. Reported from play: a tree's
    /// trunk sitting a foot to the side of the thing that stopped you.
    ///
    /// Every gate was green because the harnesses supplied the missing call
    /// themselves — `client_loop.rs` asserts `error_magnitude() < 0.05`
    /// eleven lines after making the call the product does not make, so the
    /// suite proved the function works and never asked whether anyone runs
    /// it. The fix is placement, not logic: `advance` is the one entry every
    /// render frame goes through on both paths, so the decay cannot be
    /// forgotten by a caller again. Same shape as the `pop_*` single-drain
    /// rule in `CLAUDE.md` — an owner named in code, not in a comment.
    ///
    /// Once per CALL, never per step: the offset is a render-frame quantity
    /// (`SMOOTH_NEAR` is a per-frame rate) and a frame that happened to
    /// carry two 30 Hz ticks would otherwise decay twice as fast as one
    /// that carried one.
    pub fn advance(&mut self, dt_ms: f64) -> u32 {
        let steps = self.clock.advance(dt_ms);
        self.predict.decay_error();
        for _ in 0..steps {
            let frame = InputFrame {
                seq: self.next_seq,
                // The level, plus anything pressed since the last tick and
                // already released. Cleared below so a tap is ONE tick of the
                // button — a second step in the same `advance` call must not
                // replay it as a hold.
                buttons: self.input.buttons | self.sticky_buttons,
                yaw: self.input.yaw,
                pitch: self.input.pitch,
                move_x: self.input.move_x,
                move_z: self.input.move_z,
                sel: self.input.sel,
            };
            // Consumed by the FIRST step of this call, never carried into a
            // second. A frame that happened to span two ticks would otherwise
            // turn one tap into a two-tick hold, which for a held-item swing
            // is a second swing the player did not ask for.
            self.sticky_buttons = 0;
            self.next_seq = self.next_seq.wrapping_add(1);
            self.clock.client_tick = self.clock.client_tick.wrapping_add(1);
            self.predict.step(
                frame,
                self.pieces.cols(),
                &mut Occupants {
                    table: &self.scatter_table,
                    haven: &self.haven,
                    harvested: &self.harvested,
                    cache: &mut self.slot_cache,
                },
            );
            self.input_due = true;
        }
        steps
    }

    /// Encode the due input datagram — the unacked tail plus the redundant
    /// ack header — into `buf`. Returns 0 when none is due. One datagram
    /// per client tick (30 Hz): the tail already carries the loss cover,
    /// and the upstream budget (DESIGN.md §5.3) prefers 30 to a 144 Hz
    /// render loop's worth of duplicates.
    pub fn poll_input(&mut self, buf: &mut [u8]) -> usize {
        if !self.input_due {
            return 0;
        }
        self.input_due = false;
        let (ack, ack_bits) = self.view.ack_fields();
        let tail = self.predict.tail();
        let first_tick = self.clock.client_tick.wrapping_sub(tail.len() as u32);
        let mut dg = InputDatagram::new(ack, ack_bits, first_tick);
        for f in tail {
            if dg.push(*f).is_err() {
                break; // tail is wire-capped already; defensive only
            }
        }
        encode_input(&dg, buf).unwrap_or(0)
    }

    /// One incoming datagram: apply through the view, then feed the clock,
    /// the interpolation history, and the predictor.
    pub fn on_datagram(&mut self, bytes: &[u8]) -> Ingest {
        match self.view.apply(bytes) {
            Ok(Applied::Ok { delta }) => {
                self.snapshots_applied += 1;
                if delta {
                    self.snapshots_delta += 1;
                }
                let Some(snap) = self.view.newest() else {
                    return Ingest::Error; // unreachable: just applied
                };
                let header = snap.header;
                if header.baseline_age == 0 {
                    self.interp.clear();
                }
                for &id in snap.removed() {
                    self.interp.remove(id);
                }
                for e in snap.entities() {
                    if e.id != self.player_id {
                        self.interp.push(header.tick, e);
                    }
                }
                self.clock.on_snapshot(header.tick, header.nudge);
                if let Some(own) = self.view.get(self.player_id).copied() {
                    self.predict.reconcile(
                        &own,
                        header.last_executed_seq,
                        self.pieces.cols(),
                        &mut Occupants {
                            table: &self.scatter_table,
                            haven: &self.haven,
                            harvested: &self.harvested,
                            cache: &mut self.slot_cache,
                        },
                    );
                }
                if delta {
                    Ingest::AppliedDelta
                } else {
                    Ingest::Applied
                }
            }
            Ok(Applied::Stale) => {
                self.snapshots_stale += 1;
                Ingest::Stale
            }
            Ok(Applied::NoBaseline) => {
                self.snapshots_no_baseline += 1;
                Ingest::NoBaseline
            }
            Err(_) => {
                self.decode_errors += 1;
                Ingest::Error
            }
        }
    }

    /// The float server tick remote entities render at.
    pub fn render_tick(&self) -> f64 {
        self.clock.server_est - crate::interp::INTERP_DELAY_TICKS
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::{
        encode_event_catalog, encode_event_craft_done, encode_event_gather, encode_event_inv,
        encode_event_move_refused, encode_event_moved, encode_event_slot_change,
        encode_event_slot_sync, InvSlot, MAX_EVENT_MSG_BYTES,
    };

    fn core() -> ClientCore {
        ClientCore::new(1, 0x107, 0)
    }

    /// Every `APPLIED_*` flag of word 0, in bit order. A new flag added
    /// without a row here fails the completeness assert below rather than
    /// silently landing on a bit that is already spoken for.
    const APPLIED_LO: [u32; 31] = [
        APPLIED_INV,
        APPLIED_SLOTS,
        APPLIED_RESET,
        APPLIED_TOAST,
        APPLIED_CATALOG,
        APPLIED_MARK,
        APPLIED_CRAFT_Q,
        APPLIED_CRAFT_DONE,
        APPLIED_CRAFT_REFUSED,
        APPLIED_RECIPES,
        APPLIED_PIECES,
        APPLIED_PIECE_RESET,
        APPLIED_BUILD_REFUSED,
        APPLIED_PIECE_DEFS,
        APPLIED_DEPLOYS,
        APPLIED_DEPLOY_RESET,
        APPLIED_DEPLOY_REFUSED,
        APPLIED_DEPLOY_DEFS,
        APPLIED_STOCK,
        APPLIED_PIECE_REMOVED,
        APPLIED_DEPLOY_REMOVED,
        APPLIED_CHAT,
        APPLIED_HEALTH,
        APPLIED_HIT,
        APPLIED_DEATH,
        APPLIED_BAGS,
        APPLIED_STRUCT_HIT,
        APPLIED_VITALS,
        APPLIED_CONSUME,
        APPLIED_DRANK,
        APPLIED_RESPAWN,
    ];

    /// Word 1. One flag today; the list is here so the thirty-fourth is a
    /// row and an assert, not a rediscovery.
    const APPLIED_HI: [u32; 1] = [APPLIED2_MOVE];

    /// The gate on the trap that put `APPLIED_MOVE` on the error bit.
    ///
    /// The defect was invisible to every wall the repo has: two crates,
    /// two `pub const … = 1 << 31`, one shared return channel, and each
    /// half correct on its own. What catches it is not a type and not a
    /// golden — it is the claim that the word is **exactly** full, so the
    /// next flag written as `1 << 31` collides with a value asserted here
    /// and the next one written as `1 << 32` does not compile.
    #[test]
    fn applied_word_is_full_and_bit_31_is_the_error_sentinel() {
        // Distinct, single-bit, and none of them the sentinel.
        let mut seen = 0u32;
        for (i, &f) in APPLIED_LO.iter().enumerate() {
            assert_eq!(f.count_ones(), 1, "APPLIED_LO[{i}] is not one bit");
            assert_eq!(seen & f, 0, "APPLIED_LO[{i}] collides with an earlier flag");
            assert_eq!(f & STREAM_ERR, 0, "APPLIED_LO[{i}] is the error sentinel");
            assert_eq!(f, 1 << i, "APPLIED_LO[{i}] is out of bit order");
            seen |= f;
        }
        // Exactly full: bits 0..30 all spent, bit 31 untouched. Both
        // halves matter — the first says there is no free bit for a
        // thirty-second lo flag, the second says the sentinel is clear.
        assert_eq!(seen, 0x7FFF_FFFF, "word 0 is not exactly bits 0..30");
        assert_eq!(STREAM_ERR, 1 << 31);
        assert_eq!(seen & STREAM_ERR, 0);
        assert_eq!(seen | STREAM_ERR, u32::MAX, "word 0 has an unclaimed bit");

        // Word 1 under the same discipline, with room to grow.
        let mut seen2 = 0u32;
        for (i, &f) in APPLIED_HI.iter().enumerate() {
            assert_eq!(f.count_ones(), 1, "APPLIED_HI[{i}] is not one bit");
            assert_eq!(
                seen2 & f,
                0,
                "APPLIED_HI[{i}] collides with an earlier flag"
            );
            assert_eq!(f, 1 << i, "APPLIED_HI[{i}] is out of bit order");
            seen2 |= f;
        }
        assert_eq!(seen2, (1 << APPLIED_HI.len()) - 1);
    }

    /// The behaviour the collision broke: a move verdict must not read as
    /// a stream error, and it must not cost the caller the rest of the
    /// message pump. Both directions, because both set the flag.
    #[test]
    fn a_move_verdict_is_never_the_error_bit() {
        let mut c = core();
        let mut buf = [0u8; MAX_EVENT_MSG_BYTES];

        // A landed move: from (kind 1, slot 9) to (kind 0, slot 22),
        // 7 of item 5 — every field distinct from every other.
        let len = encode_event_moved(1, 9, 0, 22, 7, 5, &mut buf).unwrap();
        let flags = c.on_stream(&buf[..len]).unwrap();
        assert_eq!(flags & STREAM_ERR, 0, "a move read as a decode error");
        assert_eq!(c.applied2(), APPLIED2_MOVE);
        assert_eq!(c.last_move_refused, 0);
        assert_eq!(c.last_move_count, (7 << 16) | 5);

        // A refusal, same rule.
        let len = encode_event_move_refused(4, 0, 11, 1, 26, &mut buf).unwrap();
        let flags = c.on_stream(&buf[..len]).unwrap();
        assert_eq!(flags & STREAM_ERR, 0, "a refusal read as a decode error");
        assert_eq!(c.applied2(), APPLIED2_MOVE);
        assert_eq!(c.last_move_refused, 4);

        // And word 1 describes one message: the next event clears it, so
        // an unconditional read cannot mistake the refusal above for a
        // verdict on a drag that has not been answered yet.
        let len = encode_event_gather(3, 7, &mut buf).unwrap();
        let flags = c.on_stream(&buf[..len]).unwrap();
        assert_eq!(flags, APPLIED_TOAST);
        assert_eq!(c.applied2(), 0, "a stale move verdict outlived its message");

        // A decode failure leaves nothing behind either.
        assert!(c.on_stream(&[0xFF, 0xFF, 0xFF]).is_err());
        assert_eq!(c.applied2(), 0);
    }

    #[test]
    fn stream_applies_inventory_and_toasts() {
        let mut c = core();
        let mut buf = [0u8; MAX_EVENT_MSG_BYTES];
        let len = encode_event_gather(3, 7, &mut buf).unwrap();
        assert_eq!(c.on_stream(&buf[..len]).unwrap(), APPLIED_TOAST);
        // A full pack (added 0) is still not a toast — `+0 × Wood` was
        // never the right line — but it is no longer *nothing*: it is a
        // spill, on word 1, and the item survives so the HUD can name it.
        let len = encode_event_gather(3, 0, &mut buf).unwrap();
        assert_eq!(c.on_stream(&buf[..len]).unwrap(), 0, "a spill toasted");
        assert_eq!(c.applied2(), APPLIED2_SPILL);
        let slots = [InvSlot {
            slot: 4,
            stack: ItemStack {
                item: 3,
                count: 21,
                cond: 0,
            },
        }];
        let len = encode_event_inv(&slots, &mut buf).unwrap();
        assert_eq!(c.on_stream(&buf[..len]).unwrap(), APPLIED_INV);
        assert_eq!(
            c.inv[4],
            ItemStack {
                item: 3,
                count: 21,
                cond: 0,
            }
        );
        assert_eq!(c.pop_toast(), Some((3, 7)));
        assert_eq!(c.pop_toast(), None);
        // The whiff went here instead, and carries which item it was.
        assert_eq!(c.pop_spill(), Some(3));
        assert_eq!(c.pop_spill(), None);
    }

    /// The gap this closes, stated as the two lines a player used to read.
    ///
    /// A full pack made gathering say *nothing at all* (the arm dropped the
    /// event on an `if added > 0`) and made crafting say something worse —
    /// `crafted 0 × …`, which reads as a failed craft for an item that in
    /// fact succeeded and is lying on the ground. Both now land on one ring
    /// the HUD turns into a sentence, and neither leaves a toast behind for
    /// the HUD to format a zero out of.
    #[test]
    fn a_full_pack_spills_from_both_producers_and_toasts_from_neither() {
        let mut c = core();
        let mut buf = [0u8; MAX_EVENT_MSG_BYTES];

        let len = encode_event_gather(11, 0, &mut buf).unwrap();
        assert_eq!(c.on_stream(&buf[..len]).unwrap() & APPLIED_TOAST, 0);
        assert_eq!(c.applied2(), APPLIED2_SPILL);

        let len = encode_event_craft_done(12, 0, &mut buf).unwrap();
        let flags = c.on_stream(&buf[..len]).unwrap();
        assert_eq!(
            flags & APPLIED_CRAFT_DONE,
            0,
            "a spilled craft still buffered a `crafted 0 × …` toast"
        );
        assert_eq!(c.applied2(), APPLIED2_SPILL);

        assert_eq!(c.pop_toast(), None, "a spill leaked into the gather ring");
        assert_eq!(
            c.pop_craft_toast(),
            None,
            "a spill leaked into the craft ring"
        );
        assert_eq!(c.pop_spill(), Some(11));
        assert_eq!(c.pop_spill(), Some(12));
        assert_eq!(c.pop_spill(), None);
    }

    /// The ring is cosmetic and bounded, so it drops oldest like its
    /// siblings rather than growing or refusing.
    #[test]
    fn spill_ring_drops_oldest() {
        let mut c = core();
        let mut buf = [0u8; MAX_EVENT_MSG_BYTES];
        for i in 0..(TOAST_RING + 2) {
            let len = encode_event_gather(i as u16, 0, &mut buf).unwrap();
            c.on_stream(&buf[..len]).unwrap();
        }
        // The first two fell off the front; the rest survive in order.
        for i in 2..(TOAST_RING + 2) {
            assert_eq!(c.pop_spill(), Some(i as u16));
        }
        assert_eq!(c.pop_spill(), None);
    }

    /// §0eat's remaining half, proven RED on the shape it replaced before
    /// the rings landed: `last_eat` / `last_eat_refused` were fields plus
    /// one bit, so two consume answers in one drain window collapsed —
    /// `Consumed` zeroed the reason and `ConsumeRefused` overwrote it. One
    /// frame reaches that from the keyboard: `KeyJ` and `KeyH` are two
    /// independent `just_pressed` checks in one system, and one
    /// `World::tick` answers both. Run against the field pair (2026-08-15),
    /// the land-then-refuse half failed "the landed eat vanished under the
    /// refusal"; refuse-then-land lost the refusal the same way.
    #[test]
    fn two_consume_answers_in_one_drain_window_both_surface() {
        let mut buf = [0u8; MAX_EVENT_MSG_BYTES];

        // The ugly order: the eat lands, then the drink is refused, both
        // inside one drain window. The field pair told the player "no
        // water within reach" while the bandage was gone and its heal
        // ramp running.
        let mut c = core();
        let len = protocol::encode_event_consumed(9, 2, &mut buf).unwrap();
        assert_eq!(c.on_stream(&buf[..len]).unwrap(), APPLIED_CONSUME);
        let len = protocol::encode_event_consume_refused(3, &mut buf).unwrap();
        assert_eq!(c.on_stream(&buf[..len]).unwrap(), APPLIED_CONSUME);
        assert_eq!(
            c.pop_consume_toast(),
            Some((9, 2)),
            "the landed eat vanished under the refusal"
        );
        assert_eq!(
            c.pop_consume_refusal(),
            Some(3),
            "the refusal vanished under the landed eat"
        );
        assert_eq!(c.pop_consume_toast(), None);
        assert_eq!(c.pop_consume_refusal(), None);

        // The reverse order lost the refusal outright on the field pair.
        let mut c = core();
        let len = protocol::encode_event_consume_refused(3, &mut buf).unwrap();
        c.on_stream(&buf[..len]).unwrap();
        let len = protocol::encode_event_consumed(9, 2, &mut buf).unwrap();
        c.on_stream(&buf[..len]).unwrap();
        assert_eq!(c.pop_consume_refusal(), Some(3));
        assert_eq!(c.pop_consume_toast(), Some((9, 2)));
    }

    /// Wall 4 on the two consume rings: bounded, drop-oldest, and each
    /// stays in arrival order past its cap.
    #[test]
    fn consume_rings_drop_oldest() {
        let mut c = core();
        let mut buf = [0u8; MAX_EVENT_MSG_BYTES];
        for i in 0..(TOAST_RING + 2) as u16 {
            let len = protocol::encode_event_consumed(i, 1, &mut buf).unwrap();
            c.on_stream(&buf[..len]).unwrap();
        }
        for i in 2..(TOAST_RING + 2) as u16 {
            assert_eq!(c.pop_consume_toast(), Some((i, 1)), "oldest-first broke");
        }
        assert_eq!(c.pop_consume_toast(), None);

        // The refusal ring under the same rule. Codes cycle inside the
        // sim's own ledger (1..=REFUSE_C_MAX): the encoder range-checks
        // both ends now, so a test that invented codes above the ledger
        // to tell slots apart would no longer encode at all.
        for i in 0..(REFUSAL_RING + 2) as u8 {
            let code = (i % sim_core::survival::REFUSE_C_MAX as u8) + 1;
            let len = protocol::encode_event_consume_refused(code, &mut buf).unwrap();
            c.on_stream(&buf[..len]).unwrap();
        }
        for i in 2..(REFUSAL_RING + 2) as u8 {
            let code = (i % sim_core::survival::REFUSE_C_MAX as u8) + 1;
            assert_eq!(c.pop_consume_refusal(), Some(code), "oldest-first broke");
        }
        assert_eq!(c.pop_consume_refusal(), None);
    }

    #[test]
    fn toast_ring_drops_oldest() {
        let mut c = core();
        let mut buf = [0u8; MAX_EVENT_MSG_BYTES];
        for i in 0..TOAST_RING as u16 + 2 {
            let len = encode_event_gather(i, 1, &mut buf).unwrap();
            c.on_stream(&buf[..len]).unwrap();
        }
        assert_eq!(c.pop_toast(), Some((2, 1)), "two oldest dropped");
    }

    #[test]
    fn stream_tracks_the_harvested_set() {
        let mut c = core();
        let mut buf = [0u8; MAX_EVENT_MSG_BYTES];
        let len = encode_event_slot_change(true, 10, 20, &mut buf).unwrap();
        assert_eq!(c.on_stream(&buf[..len]).unwrap(), APPLIED_SLOTS);
        assert!(c.harvested.contains(cell_key(10, 20)));
        assert_eq!(c.slot_changes(), &[(cell_key(10, 20), true)]);

        let len = encode_event_slot_change(false, 10, 20, &mut buf).unwrap();
        c.on_stream(&buf[..len]).unwrap();
        assert!(!c.harvested.contains(cell_key(10, 20)));
        assert_eq!(c.slot_changes(), &[(cell_key(10, 20), false)]);

        // Sync with reset replaces whatever the client believed.
        let len = encode_event_slot_change(true, 1, 1, &mut buf).unwrap();
        c.on_stream(&buf[..len]).unwrap();
        let len = encode_event_slot_sync(true, &[(2, 2), (3, 3)], &mut buf).unwrap();
        let flags = c.on_stream(&buf[..len]).unwrap();
        assert_eq!(flags, APPLIED_SLOTS | APPLIED_RESET);
        assert!(!c.harvested.contains(cell_key(1, 1)));
        assert!(c.harvested.contains(cell_key(2, 2)));
        assert!(c.harvested.contains(cell_key(3, 3)));
        assert_eq!(c.harvested.len(), 2);

        // A non-reset batch adds; duplicates stay single entries.
        let len = encode_event_slot_sync(false, &[(2, 2), (4, 4)], &mut buf).unwrap();
        assert_eq!(c.on_stream(&buf[..len]).unwrap(), APPLIED_SLOTS);
        assert_eq!(c.harvested.len(), 3);
    }

    #[test]
    fn weak_mark_sets_moves_and_clears_with_its_node() {
        use protocol::encode_event_weak_mark;
        let mut c = core();
        let mut buf = [0u8; MAX_EVENT_MSG_BYTES];
        assert_eq!(c.mark_cell, NO_CELL);

        let len = encode_event_weak_mark(10, 20, 0x40, false, &mut buf).unwrap();
        assert_eq!(c.on_stream(&buf[..len]).unwrap(), APPLIED_MARK);
        assert_eq!(
            (c.mark_cell, c.mark8, c.mark_weak_hit),
            (cell_key(10, 20), 0x40, false)
        );

        // The mark moves with the next landed hit (weak this time).
        let len = encode_event_weak_mark(10, 20, 0x9C, true, &mut buf).unwrap();
        c.on_stream(&buf[..len]).unwrap();
        assert_eq!((c.mark8, c.mark_weak_hit), (0x9C, true));

        // Another node's harvest leaves it; its own clears it.
        let len = encode_event_slot_change(true, 11, 21, &mut buf).unwrap();
        assert_eq!(c.on_stream(&buf[..len]).unwrap(), APPLIED_SLOTS);
        assert_eq!(c.mark_cell, cell_key(10, 20));
        let len = encode_event_slot_change(true, 10, 20, &mut buf).unwrap();
        assert_eq!(
            c.on_stream(&buf[..len]).unwrap(),
            APPLIED_SLOTS | APPLIED_MARK
        );
        assert_eq!(c.mark_cell, NO_CELL);

        // A sync reset clears whatever mark survived.
        let len = encode_event_weak_mark(3, 4, 0x11, false, &mut buf).unwrap();
        c.on_stream(&buf[..len]).unwrap();
        let len = encode_event_slot_sync(true, &[], &mut buf).unwrap();
        let flags = c.on_stream(&buf[..len]).unwrap();
        assert_ne!(flags & APPLIED_MARK, 0);
        assert_eq!(c.mark_cell, NO_CELL);
    }

    #[test]
    fn stream_tracks_craft_queue_recipes_and_toasts() {
        use protocol::{
            encode_event_craft_done, encode_event_craft_q, encode_event_craft_refused,
            encode_event_recipes,
        };
        use sim_core::craft::CraftJob;

        let mut c = core();
        let mut buf = [0u8; MAX_EVENT_MSG_BYTES];

        // Recipes drip in and land at their table rows.
        let cc = CraftContent::probe_fixture();
        let (len, took) = encode_event_recipes(&cc, 0, &mut buf).unwrap();
        assert_eq!(c.on_stream(&buf[..len]).unwrap(), APPLIED_RECIPES);
        assert_eq!((c.recipes.recipe_count, c.recipes_have), (3, took as u16));
        assert_eq!(c.recipes.recipes[1], cc.recipes[1]);

        // The queue announce replaces the whole view.
        let jobs = [
            CraftJob {
                recipe: 1,
                remaining: 2,
            },
            CraftJob {
                recipe: 0,
                remaining: 5,
            },
        ];
        let len = encode_event_craft_q(&jobs, 90, &mut buf).unwrap();
        assert_eq!(c.on_stream(&buf[..len]).unwrap(), APPLIED_CRAFT_Q);
        assert_eq!(c.jobs_count, 2);
        assert_eq!(c.jobs[0], (1, 2));
        assert_eq!(c.craft_eta_ticks, 90);
        let len = encode_event_craft_q(&[], 0, &mut buf).unwrap();
        c.on_stream(&buf[..len]).unwrap();
        assert_eq!(c.jobs_count, 0, "empty announce clears the queue");

        // Done toasts and refusals ride their own rings.
        let len = encode_event_craft_done(3, 2, &mut buf).unwrap();
        assert_eq!(c.on_stream(&buf[..len]).unwrap(), APPLIED_CRAFT_DONE);
        assert_eq!(c.pop_craft_toast(), Some((3, 2)));
        assert_eq!(c.pop_craft_toast(), None);
        assert_eq!(c.pop_toast(), None, "craft toast never leaks into gather's");
        let len = encode_event_craft_refused(4, &mut buf).unwrap();
        assert_eq!(c.on_stream(&buf[..len]).unwrap(), APPLIED_CRAFT_REFUSED);
        assert_eq!(c.pop_craft_refusal(), Some(4));
        assert_eq!(c.pop_craft_refusal(), None);
    }

    #[test]
    fn stream_fills_the_catalog_and_counts_errors() {
        let mut c = core();
        let mut cat = ItemCatalog::EMPTY;
        cat.count = 3;
        cat.set(0, b"Wood", 0).unwrap();
        cat.set(1, b"Stone", 0).unwrap();
        cat.set(2, b"Cloth", 12_000).unwrap();
        let mut buf = [0u8; MAX_EVENT_MSG_BYTES];
        let (len, took) = encode_event_catalog(&cat, 0, &mut buf).unwrap();
        assert_eq!(took, 3);
        assert_eq!(c.on_stream(&buf[..len]).unwrap(), APPLIED_CATALOG);
        assert_eq!(c.catalog.count, 3);
        assert_eq!(c.catalog.name(1), b"Stone");
        assert_eq!(
            c.catalog.cond_max(2),
            12_000,
            "the ceiling column (wire v46) must land beside the name — \
             it is what pip_fraction divides by"
        );
        assert_eq!(c.catalog.cond_max(0), 0);
        assert_eq!(c.events_applied, 1);

        assert!(c.on_stream(&[0xFF, 0xFF, 0xFF]).is_err());
        assert_eq!(c.event_errors, 1);
    }

    /// The predictor collides against the same closed doors the sim does,
    /// and the shut bit survives every path that rebuilds the derived
    /// index. A door the client renders shut but predicts through is the
    /// exact desync this test exists to catch.
    #[test]
    fn doors_seal_the_predictor_index_and_survive_a_rebuild() {
        use protocol::{
            encode_event_deploy_defs, encode_event_deploy_placed, encode_event_deploy_refused,
            encode_event_door, encode_event_piece_defs, encode_event_piece_placed,
            encode_event_removed,
        };
        use sim_core::build::{BuildContent, PieceRec, LOC_EDGE_XLO, SHAPE_DOORWAY};
        use sim_core::deploy::DeployContent;

        let mut c = core();
        let mut buf = [0u8; MAX_EVENT_MSG_BYTES];
        let (cx, cz, level) = (341u16, 682u16, 0u8);
        let shut = |c: &ClientCore| c.pieces.cols().get(cx, cz).shut_xlo & 1 != 0;

        // The doorway piece and the def tables the client needs to read
        // "row 2 is a door" (probe fixtures: piece row 3 is the doorway,
        // deploy row 2 the door).
        let bc = BuildContent::probe_fixture();
        let (len, _) = encode_event_piece_defs(&bc, 0, &mut buf).unwrap();
        c.on_stream(&buf[..len]).unwrap();
        let dc = DeployContent::probe_fixture();
        let (len, _) = encode_event_deploy_defs(&dc, 0, &mut buf).unwrap();
        c.on_stream(&buf[..len]).unwrap();
        let doorway = PieceRec {
            cx,
            cz,
            level,
            loc: LOC_EDGE_XLO,
            row: 3,
            ..PieceRec::default()
        };
        let len = encode_event_piece_placed(&doorway, &mut buf).unwrap();
        c.on_stream(&buf[..len]).unwrap();
        assert_eq!(c.piece_defs.pieces[3].shape, SHAPE_DOORWAY);
        assert!(!shut(&c), "an empty doorway is not sealed");

        // The door lands closed: the doorway seals.
        let door = DeployRec {
            cx,
            cz,
            level,
            loc: LOC_EDGE_XLO,
            row: 2,
            ..DeployRec::default()
        };
        let len = encode_event_deploy_placed(&door, &mut buf).unwrap();
        c.on_stream(&buf[..len]).unwrap();
        assert!(shut(&c), "a placed door seals its doorway");

        // Opened, then closed again — the announcement is absolute, and
        // it carries the lock bit beside the leaf (lock v0, wire v8).
        let len =
            encode_event_door(cx, cz, level, LOC_EDGE_XLO, true, true, true, &mut buf).unwrap();
        assert_eq!(c.on_stream(&buf[..len]).unwrap(), APPLIED_DEPLOYS);
        assert!(!shut(&c), "an open door passes");
        assert!(c.deploy_changes()[0].open, "the renderer hears the state");
        assert!(
            c.deploy_changes()[0].locked,
            "the renderer hears the lock too"
        );
        let len =
            encode_event_door(cx, cz, level, LOC_EDGE_XLO, false, false, true, &mut buf).unwrap();
        c.on_stream(&buf[..len]).unwrap();
        assert!(shut(&c), "a reclosed door seals again");
        assert!(
            !c.deploys.entries()[0].locked,
            "an absolute announcement clears the lock as readily as it sets it"
        );

        // A later piece-def batch rebuilds the collision index from the
        // piece mirror alone; the door bits must go back on with it.
        let (len, _) = encode_event_piece_defs(&bc, 0, &mut buf).unwrap();
        c.on_stream(&buf[..len]).unwrap();
        assert!(shut(&c), "the defs rebuild dropped the door bit");

        // Your own door swings on input, not on the reply (NETCODE.md
        // §6.1): the predictor unseals immediately, and only one
        // prediction is outstanding at a time.
        assert_eq!(c.predict_door(cx, cz, level, LOC_EDGE_XLO), Some(true));
        assert!(!shut(&c), "the predicted open door still blocks the body");
        assert_eq!(
            c.predict_door(cx, cz, level, LOC_EDGE_XLO),
            None,
            "a second toggle must wait for the first to resolve"
        );
        // The announcement confirms it and frees the next prediction.
        let len =
            encode_event_door(cx, cz, level, LOC_EDGE_XLO, true, true, true, &mut buf).unwrap();
        c.on_stream(&buf[..len]).unwrap();
        assert!(!shut(&c));
        assert_eq!(c.predict_door(cx, cz, level, LOC_EDGE_XLO), Some(false));
        assert!(shut(&c), "the predicted close seals for the predictor");

        // A refusal instead rolls the prediction back to server truth —
        // and must say so with APPLIED_DEPLOYS, because the renderer
        // swung that leaf on the press and no announcement is coming
        // (the sim's state never moved, so there is nothing to announce).
        let len =
            encode_event_deploy_refused(sim_core::deploy::REFUSE_D_REACH as u8, &mut buf).unwrap();
        let flags = c.on_stream(&buf[..len]).unwrap();
        assert_eq!(
            flags & (APPLIED_DEPLOYS | APPLIED_DEPLOY_REFUSED),
            APPLIED_DEPLOYS | APPLIED_DEPLOY_REFUSED,
            "a rolled-back door must reach the renderer"
        );
        assert_eq!(
            c.deploy_changes().len(),
            1,
            "the rolled-back record must ride the change list"
        );
        assert!(
            c.deploy_changes()[0].open,
            "the change carries the state the sim never left"
        );
        assert!(!shut(&c), "a refused toggle must not leave the door shut");
        assert!(
            c.deploys.entries()[0].open,
            "the mirror kept the state the sim never left"
        );
        // Nothing is outstanding now, so the next press predicts again.
        assert_eq!(c.predict_door(cx, cz, level, LOC_EDGE_XLO), Some(false));
        assert!(
            c.deploys.entries()[0].locked,
            "predicting the leaf must not touch the lock"
        );
        let len =
            encode_event_door(cx, cz, level, LOC_EDGE_XLO, false, true, true, &mut buf).unwrap();
        c.on_stream(&buf[..len]).unwrap();
        assert!(shut(&c));

        // And the door going away unseals the doorway it was holding.
        let len = encode_event_removed(false, cx, cz, level, LOC_EDGE_XLO, &mut buf).unwrap();
        c.on_stream(&buf[..len]).unwrap();
        assert!(!shut(&c), "a removed door leaves nothing sealed");
    }

    /// The own-bag join, whole: a `Death` naming this body arms the
    /// latch, the first `BagDropped` beside the dead predicted body is
    /// tagged, a stranger's bag farther away is not, a later neighbour
    /// bag cannot re-tag while the screen is still up, and the bag's own
    /// `BagRemoved` clears the tag. The wire carries no owner — this is
    /// the client-side join `own_bag`'s field doc states, gated end to
    /// end over real encoded events.
    #[test]
    fn the_own_bag_is_tagged_by_death_and_distance_and_cleared_by_removal() {
        let mut c = core();
        let mut buf = [0u8; MAX_EVENT_MSG_BYTES];

        // A stranger's bag before any death: never tagged.
        let far = protocol::WireBag {
            id: 9,
            qx: 90_000,
            qy: 0,
            qz: 90_000,
        };
        let len = protocol::encode_event_bag_dropped(&far, &mut buf).unwrap();
        c.on_stream(&buf[..len]).unwrap();
        assert_eq!(c.own_bag, 0, "no death, no tag");

        // This body dies. The predicted body sits at the default origin,
        // so the death position is (0, 0).
        let len = protocol::encode_event_death(0x107, 5, 0, 0, 100, &mut buf).unwrap();
        c.on_stream(&buf[..len]).unwrap();
        assert!(c.dead);

        // A bag beside the body (1 m off, inside OWN_BAG_NEAR_M): tagged.
        let near_q = (1.0 / POS_XZ_Q) as i32;
        let mine = protocol::WireBag {
            id: 11,
            qx: near_q,
            qy: 0,
            qz: 0,
        };
        let len = protocol::encode_event_bag_dropped(&mine, &mut buf).unwrap();
        c.on_stream(&buf[..len]).unwrap();
        assert_eq!(c.own_bag, 11, "the first drop beside the dead body is mine");

        // A neighbour dies on the same spot a moment later: the latch is
        // spent, the tag holds.
        let theirs = protocol::WireBag {
            id: 12,
            qx: 0,
            qy: 0,
            qz: near_q,
        };
        let len = protocol::encode_event_bag_dropped(&theirs, &mut buf).unwrap();
        c.on_stream(&buf[..len]).unwrap();
        assert_eq!(c.own_bag, 11, "a spent latch cannot re-tag");

        // Respawn does NOT clear it — walking back is the point.
        let len = protocol::encode_event_respawn(false, &mut buf).unwrap();
        c.on_stream(&buf[..len]).unwrap();
        assert_eq!(c.own_bag, 11, "the tag outlives the death screen");

        // The bag's own removal does.
        let len = protocol::encode_event_bag_removed(11, 1, &mut buf).unwrap();
        c.on_stream(&buf[..len]).unwrap();
        assert_eq!(c.own_bag, 0, "a removed bag is nobody's mark");
    }

    /// The distance half alone: a death whose only bag lands beyond
    /// `OWN_BAG_NEAR_M` tags nothing — a stranger dying across the meadow
    /// in the same window is not this body's bag, and the latch stays
    /// armed for the drop that IS.
    #[test]
    fn a_far_bag_does_not_spend_the_own_bag_latch() {
        let mut c = core();
        let mut buf = [0u8; MAX_EVENT_MSG_BYTES];
        let len = protocol::encode_event_death(0x107, 5, 0, 0, 100, &mut buf).unwrap();
        c.on_stream(&buf[..len]).unwrap();

        let far_q = (8.0 / POS_XZ_Q) as i32; // 2x the join radius
        let theirs = protocol::WireBag {
            id: 21,
            qx: far_q,
            qy: 0,
            qz: 0,
        };
        let len = protocol::encode_event_bag_dropped(&theirs, &mut buf).unwrap();
        c.on_stream(&buf[..len]).unwrap();
        assert_eq!(c.own_bag, 0, "eight metres away is somebody else");

        let mine = protocol::WireBag {
            id: 22,
            qx: 0,
            qy: 0,
            qz: 0,
        };
        let len = protocol::encode_event_bag_dropped(&mine, &mut buf).unwrap();
        c.on_stream(&buf[..len]).unwrap();
        assert_eq!(c.own_bag, 22, "the latch waited for the drop at the body");
    }
}
