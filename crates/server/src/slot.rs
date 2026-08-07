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
//!   never deallocates — L2 outlives the tick);
//! - global, the save path (`store.rs`): sim→accept records ring, then
//!   accept→storage-thread writes ring. One direction, two hops, and the
//!   reason for the second one is that the sim thread may not touch a file
//!   and the accept loop may not block on `sync_data`.

use crate::store::PlayerKey;
use protocol::{ActionMsg, ChatMsg, InputDatagram, MAX_EVENT_MSG_BYTES};
use rtrb::{Consumer, Producer};
use sim_core::limits::DATAGRAM_BUDGET_BYTES;
use sim_core::persist::PlayerSave;
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
    /// C→S chat lines, off the same stream as the actions. The sim drains
    /// at most one per tick; a full ring drops the newest line (limits.rs
    /// — chat never backpressures the shared stream, or a spammer would
    /// stall their own transactions behind their own typing).
    pub chats: Consumer<ChatMsg>,
    pub snaps: Producer<SnapMsg>,
    pub events: Producer<EvMsg>,
}

/// Accept→sim: install a freshly handshaken connection.
pub struct Connect {
    pub slot: usize,
    pub id: u32,
    /// What the store remembered about this player, if it knew them. `None`
    /// ⇒ a fresh character: no key (a guest), no record under their key, or
    /// no save file configured at all.
    ///
    /// The record travels *with the connection* rather than being fetched by
    /// the sim, because the sim thread may not read a file and because a
    /// restore has to enter the world as a command (`Command::JoinAs`) for a
    /// replay to reproduce it.
    pub save: Option<PlayerSave>,
    /// Who this connection is, as the store's opaque key. `None` ⇒ a guest
    /// the admission seam could not name, who therefore cannot be handed
    /// back a record **or** the body they left behind.
    ///
    /// It rides along for one job: `ShardCore` has to file the sleeper a
    /// leave creates under the identity that will come back for it, and it
    /// cannot ask the accept loop's key table from the sim thread. Note
    /// what it is not — the key never enters `sim-core`, and this struct is
    /// the boundary where that stops being true (`persist.rs`).
    pub key: Option<PlayerKey>,
    pub link: Link,
}

/// Sim→accept: one player's state, for the store's index to file under their
/// key. Carries the player **id**, not the key — the sim has never heard of a
/// key, and that is the wall that keeps identity out of the deterministic
/// core. The accept loop owns the id→key table and does the pairing.
#[derive(Clone, Copy)]
pub struct SaveMsg {
    pub id: u32,
    pub save: PlayerSave,
}

/// Accept→storage thread: one record and where in the file it goes. The
/// index is assigned by the store's index owner, so the thread that touches
/// the disk decides nothing — it seeks, writes, flushes.
#[derive(Clone, Copy)]
pub struct WriteMsg {
    pub index: usize,
    pub key: PlayerKey,
    /// Unix seconds, stamped by the index owner. It orders eviction and it
    /// has to survive a restart, which the world's tick counter does not.
    pub stamp: u64,
    pub save: PlayerSave,
}
