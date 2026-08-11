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
use sim_core::deploy::{DeployContent, DeployRec, ARCH_DOOR};
use sim_core::gather::{cell_key, ItemStack, NO_CELL};
use sim_core::input::InputFrame;
use sim_core::inventory::CONT_SELF;
use sim_core::limits::{
    CRAFT_QUEUE, HEARTH_STOCK_ROWS, HOTBAR_SLOTS, INV_SLOTS, MAX_BACKPACKS, MAX_BOXES, MAX_DEPLOYS,
    MAX_PIECES, MAX_SLOT_LIVES,
};
use sim_core::occupy::{Harvested, Occupants, SlotCache};
use sim_core::terrain::{self, Haven, ScatterTable};

/// Gather toasts buffered for the HUD (drop-oldest — a toast is cosmetic).
pub const TOAST_RING: usize = 8;

/// Craft refusal reasons buffered for the HUD (drop-oldest, cosmetic).
pub const REFUSAL_RING: usize = 4;

/// Chat lines buffered for the log (drop-oldest). Deeper than the toast
/// ring because the pump drains it every event, not every frame, and a
/// dropped line is the one loss a player would actually notice.
pub const CHAT_RING: usize = 16;

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
/// An eat landed or was refused (`EventMsg::Consumed` /
/// `EventMsg::ConsumeRefused`). One flag: the HUD's response to both is to
/// re-read the eat readout, which says which it was.
pub const APPLIED_CONSUME: u32 = 1 << 28;
/// A drink landed (`EventMsg::Drank`). Its own bit and not `CONSUME`'s: a
/// refused drink already arrives as a `ConsumeRefused`, so sharing the bit
/// would leave the HUD holding two readouts and no way to know which of
/// them this frame's flag was about.
pub const APPLIED_DRANK: u32 = 1 << 29;
/// The death screen opened or closed — `EventMsg::Death` naming *you*, or
/// the `EventMsg::Respawn` that answers it. One flag for both because the
/// HUD's response to either is to re-read `client_death_screen`, which
/// says which it was; the same shape `APPLIED_CONSUME` takes.
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
/// flag for both, `APPLIED_CONSUME`'s shape: the panel's response to
/// either is to re-read `client_move_readout`, which says which it was.
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
    /// The last eat: item << 16 | slot, and the refusal reason (0 = the
    /// eat landed). Read together by `client_consume`.
    pub last_eat: u32,
    pub last_eat_refused: u8,
    /// The last drink: water restored << 16 | hp it cost. Read by
    /// `client_drank`, and the reason the HUD can name what took the hp.
    pub last_drink: u32,
    /// The last move's address, `sim_core::inventory::addr`'s pack
    /// verbatim, and the refusal reason (0 = the move landed). Read
    /// together by `client_move_readout`.
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
    /// Own landed hits, oldest first: damage dealt. The hitmarker ring.
    hits: [u16; TOAST_RING],
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
            mark_cell: NO_CELL,
            mark8: 0,
            mark_weak_hit: false,
            recipes: CraftContent::EMPTY,
            recipes_have: 0,
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
            last_eat: 0,
            last_eat_refused: 0,
            last_drink: 0,
            last_move: 0,
            last_move_refused: 0,
            last_move_count: 0,
            hits: [0; TOAST_RING],
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
            research_refusal_head: 0,
            research_refusal_len: 0,
            events_applied: 0,
            event_errors: 0,
        }
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
                }
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
            } => {
                self.catalog.count = total as u16;
                for i in 0..count as usize {
                    // Server-sent lens are wire-validated ≤ the cap.
                    let _ = self
                        .catalog
                        .set(first as usize + i, &names[i][..lens[i] as usize]);
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
                if self.craft_toast_len == TOAST_RING {
                    self.craft_toast_head = (self.craft_toast_head + 1) % TOAST_RING;
                    self.craft_toast_len -= 1;
                }
                self.craft_toasts[(self.craft_toast_head + self.craft_toast_len) % TOAST_RING] =
                    (item, added);
                self.craft_toast_len += 1;
                flags |= APPLIED_CRAFT_DONE;
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
                // because "my swing landed for N" is the same fact.
                if self.hit_len == TOAST_RING {
                    self.hit_head = (self.hit_head + 1) % TOAST_RING;
                    self.hit_len -= 1;
                }
                self.hits[(self.hit_head + self.hit_len) % TOAST_RING] = damage;
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
                self.last_eat = ((item as u32) << 16) | slot as u32;
                self.last_eat_refused = 0;
                flags |= APPLIED_CONSUME;
            }
            EventMsg::ConsumeRefused { reason } => {
                self.last_eat_refused = reason;
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
                let _ = victim; // v0 marks the hit, not who took it
                if self.hit_len == TOAST_RING {
                    self.hit_head = (self.hit_head + 1) % TOAST_RING;
                    self.hit_len -= 1;
                }
                self.hits[(self.hit_head + self.hit_len) % TOAST_RING] = damage;
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
            self.pieces
                .set_solid(rec.cx, rec.cz, rec.level, Some(arch));
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

    /// Oldest buffered hitmarker, if any: damage this client's swing dealt.
    pub fn pop_hit(&mut self) -> Option<u16> {
        if self.hit_len == 0 {
            return None;
        }
        let d = self.hits[self.hit_head];
        self.hit_head = (self.hit_head + 1) % TOAST_RING;
        self.hit_len -= 1;
        Some(d)
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
        self.input = InputState {
            buttons,
            yaw,
            pitch,
            move_x,
            move_z,
            sel: sel.min(HOTBAR_SLOTS as u8 - 1),
        };
    }

    /// Advance real time: run the fixed client ticks that elapsed, each
    /// generating one input frame and stepping prediction. Returns steps.
    pub fn advance(&mut self, dt_ms: f64) -> u32 {
        let steps = self.clock.advance(dt_ms);
        for _ in 0..steps {
            let frame = InputFrame {
                seq: self.next_seq,
                buttons: self.input.buttons,
                yaw: self.input.yaw,
                pitch: self.input.pitch,
                move_x: self.input.move_x,
                move_z: self.input.move_z,
                sel: self.input.sel,
            };
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
        encode_event_catalog, encode_event_gather, encode_event_inv, encode_event_move_refused,
        encode_event_moved, encode_event_slot_change, encode_event_slot_sync, InvSlot,
        MAX_EVENT_MSG_BYTES,
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
        // A full-inventory whiff (added 0) is not a toast.
        let len = encode_event_gather(3, 0, &mut buf).unwrap();
        assert_eq!(c.on_stream(&buf[..len]).unwrap(), 0);
        let slots = [InvSlot {
            slot: 4,
            stack: ItemStack { item: 3, count: 21 },
        }];
        let len = encode_event_inv(&slots, &mut buf).unwrap();
        assert_eq!(c.on_stream(&buf[..len]).unwrap(), APPLIED_INV);
        assert_eq!(c.inv[4], ItemStack { item: 3, count: 21 });
        assert_eq!(c.pop_toast(), Some((3, 7)));
        assert_eq!(c.pop_toast(), None);
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
        cat.set(0, b"Wood").unwrap();
        cat.set(1, b"Stone").unwrap();
        cat.set(2, b"Cloth").unwrap();
        let mut buf = [0u8; MAX_EVENT_MSG_BYTES];
        let (len, took) = encode_event_catalog(&cat, 0, &mut buf).unwrap();
        assert_eq!(took, 3);
        assert_eq!(c.on_stream(&buf[..len]).unwrap(), APPLIED_CATALOG);
        assert_eq!(c.catalog.count, 3);
        assert_eq!(c.catalog.name(1), b"Stone");
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
        use sim_core::build::{BuildContent, PieceRec, LOC_EDGE_W, SHAPE_DOORWAY};
        use sim_core::deploy::DeployContent;

        let mut c = core();
        let mut buf = [0u8; MAX_EVENT_MSG_BYTES];
        let (cx, cz, level) = (341u16, 682u16, 0u8);
        let shut = |c: &ClientCore| c.pieces.cols().get(cx, cz).shut_w & 1 != 0;

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
            loc: LOC_EDGE_W,
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
            loc: LOC_EDGE_W,
            row: 2,
            ..DeployRec::default()
        };
        let len = encode_event_deploy_placed(&door, &mut buf).unwrap();
        c.on_stream(&buf[..len]).unwrap();
        assert!(shut(&c), "a placed door seals its doorway");

        // Opened, then closed again — the announcement is absolute, and
        // it carries the lock bit beside the leaf (lock v0, wire v8).
        let len = encode_event_door(cx, cz, level, LOC_EDGE_W, true, true, true, &mut buf).unwrap();
        assert_eq!(c.on_stream(&buf[..len]).unwrap(), APPLIED_DEPLOYS);
        assert!(!shut(&c), "an open door passes");
        assert!(c.deploy_changes()[0].open, "the renderer hears the state");
        assert!(
            c.deploy_changes()[0].locked,
            "the renderer hears the lock too"
        );
        let len =
            encode_event_door(cx, cz, level, LOC_EDGE_W, false, false, true, &mut buf).unwrap();
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
        assert_eq!(c.predict_door(cx, cz, level, LOC_EDGE_W), Some(true));
        assert!(!shut(&c), "the predicted open door still blocks the body");
        assert_eq!(
            c.predict_door(cx, cz, level, LOC_EDGE_W),
            None,
            "a second toggle must wait for the first to resolve"
        );
        // The announcement confirms it and frees the next prediction.
        let len = encode_event_door(cx, cz, level, LOC_EDGE_W, true, true, true, &mut buf).unwrap();
        c.on_stream(&buf[..len]).unwrap();
        assert!(!shut(&c));
        assert_eq!(c.predict_door(cx, cz, level, LOC_EDGE_W), Some(false));
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
        assert_eq!(c.predict_door(cx, cz, level, LOC_EDGE_W), Some(false));
        assert!(
            c.deploys.entries()[0].locked,
            "predicting the leaf must not touch the lock"
        );
        let len =
            encode_event_door(cx, cz, level, LOC_EDGE_W, false, true, true, &mut buf).unwrap();
        c.on_stream(&buf[..len]).unwrap();
        assert!(shut(&c));

        // And the door going away unseals the doorway it was holding.
        let len = encode_event_removed(false, cx, cz, level, LOC_EDGE_W, &mut buf).unwrap();
        c.on_stream(&buf[..len]).unwrap();
        assert!(!shut(&c), "a removed door leaves nothing sealed");
    }
}
