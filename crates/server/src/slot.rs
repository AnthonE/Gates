//! Connection slots and the ring payloads that cross threads. One slot per
//! potential player, preallocated at boot; the accept loop claims slots,
//! connection tasks mark them dying, the sim thread frees them. All of it
//! rides one packed atomic per slot — no locks anywhere (L3).
//!
//! Ring topology (all bounded SPSC, DESIGN.md §4):
//! - per connection: net→sim input ring, sim→net snapshot ring
//!   (created at accept — DESIGN.md §4's "preallocated at accept");
//! - global: accept→sim control ring (installs a connection's ring
//!   handles), sim→accept graveyard ring (returns them, so the sim thread
//!   never deallocates — L2 outlives the tick).

use protocol::{ActionMsg, InputDatagram, MAX_EVENT_MSG_BYTES};
use rtrb::{Consumer, Producer};
use sim_core::limits::DATAGRAM_BUDGET_BYTES;
use std::sync::atomic::{AtomicU32, Ordering};

/// Slot lifecycle, 2 bits of the packed word. Claims bump the generation
/// (the other 30 bits) so a task from a dead connection can never act on a
/// slot's next tenant.
pub const SLOT_EMPTY: u32 = 0;
pub const SLOT_LIVE: u32 = 1;
pub const SLOT_LEAVING: u32 = 2;

const STATE_BITS: u32 = 2;
const STATE_MASK: u32 = (1 << STATE_BITS) - 1;

#[inline]
pub fn pack(state: u32, generation: u32) -> u32 {
    (generation << STATE_BITS) | state
}

#[inline]
pub fn state_of(word: u32) -> u32 {
    word & STATE_MASK
}

#[inline]
pub fn generation_of(word: u32) -> u32 {
    word >> STATE_BITS
}

/// The per-slot atomic table, shared by accept loop, connection tasks and
/// the sim thread.
pub struct SlotTable {
    words: Box<[AtomicU32]>,
}

impl SlotTable {
    pub fn new(slots: usize) -> Self {
        let mut v = Vec::with_capacity(slots);
        v.resize_with(slots, || AtomicU32::new(pack(SLOT_EMPTY, 0)));
        Self {
            words: v.into_boxed_slice(),
        }
    }

    pub fn load(&self, slot: usize) -> u32 {
        self.words[slot].load(Ordering::Acquire)
    }

    /// Accept loop only: claim an empty slot, bumping its generation.
    /// Returns the new generation.
    pub fn claim(&self, slot: usize) -> Option<u32> {
        let cur = self.load(slot);
        if state_of(cur) != SLOT_EMPTY {
            return None;
        }
        let generation = generation_of(cur).wrapping_add(1) & 0x3FFF_FFFF;
        let next = pack(SLOT_LIVE, generation);
        self.words[slot]
            .compare_exchange(cur, next, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| generation)
    }

    /// Accept loop only: undo a claim whose control-ring push was refused.
    pub fn unclaim(&self, slot: usize, generation: u32) {
        let _ = self.words[slot].compare_exchange(
            pack(SLOT_LIVE, generation),
            pack(SLOT_EMPTY, generation),
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }

    /// Connection tasks: flag the slot for sim-side cleanup. A stale
    /// generation loses the race silently — the slot already moved on.
    pub fn mark_leaving(&self, slot: usize, generation: u32) {
        let _ = self.words[slot].compare_exchange(
            pack(SLOT_LIVE, generation),
            pack(SLOT_LEAVING, generation),
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }

    /// Sim thread only: release a cleaned slot.
    pub fn free(&self, slot: usize, generation: u32) {
        let _ = self.words[slot].compare_exchange(
            pack(SLOT_LEAVING, generation),
            pack(SLOT_EMPTY, generation),
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }
}

/// One encoded snapshot crossing sim→net. Fixed payload, no allocation on
/// push (rtrb copies the value into its preallocated buffer).
#[derive(Clone, Copy)]
pub struct SnapMsg {
    pub len: u16,
    pub buf: [u8; DATAGRAM_BUDGET_BYTES],
}

impl SnapMsg {
    pub fn bytes(&self) -> &[u8] {
        &self.buf[..self.len as usize]
    }
}

/// One encoded event-lane message crossing sim→net (the reliable bidi
/// stream). Fixed payload like `SnapMsg`; the writer task frames it.
#[derive(Clone, Copy)]
pub struct EvMsg {
    pub len: u16,
    pub buf: [u8; MAX_EVENT_MSG_BYTES],
}

impl EvMsg {
    pub fn bytes(&self) -> &[u8] {
        &self.buf[..self.len as usize]
    }
}

/// The sim thread's ends of one connection's rings.
pub struct Link {
    pub generation: u32,
    pub input: Consumer<InputDatagram>,
    /// C→S reliable actions (craft requests). The sim drains at most one
    /// per tick; a full ring backpressures the stream reader (limits.rs).
    pub actions: Consumer<ActionMsg>,
    pub snaps: Producer<SnapMsg>,
    pub events: Producer<EvMsg>,
}

/// Accept→sim: install a freshly handshaken connection.
pub struct Connect {
    pub slot: usize,
    pub id: u32,
    pub link: Link,
}
