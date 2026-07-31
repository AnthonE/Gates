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
use protocol::{decode_event, encode_input, EventMsg, InputDatagram, ItemCatalog, WireError};
use sim_core::build::{BuildContent, PieceRec};
use sim_core::craft::CraftContent;
use sim_core::gather::{cell_key, ItemStack, NO_CELL};
use sim_core::input::InputFrame;
use sim_core::limits::{CRAFT_QUEUE, HOTBAR_SLOTS, INV_SLOTS, MAX_PIECES, MAX_SLOT_LIVES};

/// Gather toasts buffered for the HUD (drop-oldest — a toast is cosmetic).
pub const TOAST_RING: usize = 8;

/// Craft refusal reasons buffered for the HUD (drop-oldest, cosmetic).
pub const REFUSAL_RING: usize = 4;

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

/// The client's mirror of the placed-piece set, keyed by grid address —
/// bounded like the server's store (`MAX_PIECES`). Insertion is
/// idempotent (a piece can arrive by broadcast AND by the sync walk); an
/// insert past capacity is dropped, the same bounded posture as the
/// harvested set (the server refuses placements there too, so a full
/// client store only desyncs against a server bug, and the next resync
/// walk retries).
pub struct PieceSet {
    recs: Box<[PieceRec]>,
    len: usize,
}

impl PieceSet {
    fn new() -> Self {
        Self {
            recs: vec![PieceRec::default(); MAX_PIECES].into_boxed_slice(),
            len: 0,
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

    /// True if the set changed (a known address with the same row is a
    /// duplicate, not a change).
    fn insert(&mut self, rec: PieceRec) -> bool {
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

    fn clear(&mut self) {
        self.len = 0;
    }
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
    pub harvested: HarvestedSet,
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
    refusals: [u8; REFUSAL_RING],
    refusal_head: usize,
    refusal_len: usize,
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
            harvested: HarvestedSet::new(),
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
            events_applied: 0,
            event_errors: 0,
        }
    }

    /// One event-lane message off the reliable stream. Returns the
    /// `APPLIED_*` flags saying what changed; cell-level detail for the
    /// renderer is in `slot_changes()` until the next call.
    pub fn on_stream(&mut self, bytes: &[u8]) -> Result<u32, WireError> {
        self.n_slot_changes = 0;
        self.n_piece_changes = 0;
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
                if self.pieces.insert(rec) {
                    self.push_piece_change(rec);
                    flags |= APPLIED_PIECES;
                }
            }
            EventMsg::PieceSync { reset, recs, count } => {
                if reset {
                    self.pieces.clear();
                    flags |= APPLIED_PIECE_RESET;
                }
                for &rec in recs.iter().take(count as usize) {
                    if self.pieces.insert(rec) {
                        self.push_piece_change(rec);
                        flags |= APPLIED_PIECES;
                    }
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
                flags |= APPLIED_PIECE_DEFS;
            }
        }
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
            self.predict.step(frame);
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
                    self.predict.reconcile(&own, header.last_executed_seq);
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
        encode_event_catalog, encode_event_gather, encode_event_inv, encode_event_slot_change,
        encode_event_slot_sync, InvSlot, MAX_EVENT_MSG_BYTES,
    };

    fn core() -> ClientCore {
        ClientCore::new(1, 0x107, 0)
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
}
