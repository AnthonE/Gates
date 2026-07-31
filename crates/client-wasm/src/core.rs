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
use sim_core::gather::{cell_key, ItemStack};
use sim_core::input::InputFrame;
use sim_core::limits::{INV_SLOTS, MAX_SLOT_LIVES};

/// Gather toasts buffered for the HUD (drop-oldest — a toast is cosmetic).
pub const TOAST_RING: usize = 8;

/// What one event-lane message changed, as bit flags the bridge hands JS.
pub const APPLIED_INV: u32 = 1 << 0;
pub const APPLIED_SLOTS: u32 = 1 << 1;
pub const APPLIED_RESET: u32 = 1 << 2;
pub const APPLIED_TOAST: u32 = 1 << 3;
pub const APPLIED_CATALOG: u32 = 1 << 4;

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
            events_applied: 0,
            event_errors: 0,
        }
    }

    /// One event-lane message off the reliable stream. Returns the
    /// `APPLIED_*` flags saying what changed; cell-level detail for the
    /// renderer is in `slot_changes()` until the next call.
    pub fn on_stream(&mut self, bytes: &[u8]) -> Result<u32, WireError> {
        self.n_slot_changes = 0;
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
                flags |= APPLIED_SLOTS;
            }
            EventMsg::SlotRespawned { cx, cz } => {
                self.harvested.remove(cell_key(cx, cz));
                self.push_change(cell_key(cx, cz), false);
                flags |= APPLIED_SLOTS;
            }
            EventMsg::SlotSync {
                reset,
                cells,
                count,
            } => {
                if reset {
                    self.harvested.clear();
                    flags |= APPLIED_RESET;
                }
                for &(cx, cz) in cells.iter().take(count as usize) {
                    self.harvested.insert(cell_key(cx, cz));
                    self.push_change(cell_key(cx, cz), true);
                }
                flags |= APPLIED_SLOTS;
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
        }
        Ok(flags)
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

    /// The live input state; sampled once per generated frame.
    pub fn set_input(&mut self, buttons: u8, yaw: u16, pitch: u8, move_x: i8, move_z: i8) {
        self.input = InputState {
            buttons,
            yaw,
            pitch,
            move_x,
            move_z,
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
