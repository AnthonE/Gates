//! Per-client netcode state, owned by the sim thread. Fixed-capacity
//! everything; allocation only at construction (L2/L4).
//!
//! The baseline model (NETCODE.md §3, the Q3/Gaffer shape): the server
//! keeps a ring of the last `SENT_SNAPSHOT_RING` snapshots **as sent** and
//! deltas each client against the newest *acked* one — by its sent
//! content, never a folded view. An ack means the client applied that
//! snapshot (protocol doc), so both sides hold byte-identical baselines by
//! construction; anything shed from the baseline snapshot simply encodes
//! absolute. An ack outside the ring, a fresh join, and every bookkeeping
//! overflow all fall to the same zero-state path — recovery is not a
//! special case.

use protocol::{ActionMsg, ChatMsg, EntityState, Nudge};
use sim_core::craft::CraftJob;
use sim_core::gather::ItemStack;
use sim_core::input::InputFrame;
use sim_core::inventory::CONT_SELF;
use sim_core::limits::{
    CRAFT_QUEUE, INPUT_BUFFER_CAP, INPUT_THROTTLE_DEPTH, INV_SLOTS, MAX_MOBS, MAX_PLAYERS,
    MAX_SNAPSHOT_ENTITIES, PENDING_REMOVALS_CAP, SENT_SNAPSHOT_RING, SNAPSHOT_INTERVAL_TICKS,
};

/// Consecutive starved ticks before the nudge escalates to `HardResync`
/// (NETCODE.md §4's "buffer empty" case, debounced so one lost datagram
/// doesn't trigger it). Proposed default, DECISIONS.md §open.
pub const STARVE_HARD_RESYNC_TICKS: u32 = 30;

/// One snapshot as sent: the delta baseline candidate and the removal
/// coverage record.
#[derive(Clone, Copy)]
pub struct SentSnap {
    pub used: bool,
    pub acked: bool,
    pub tick: u32,
    pub entity_count: u8,
    pub removed_count: u8,
    pub entities: [EntityState; MAX_SNAPSHOT_ENTITIES],
    pub removed: [u32; MAX_SNAPSHOT_ENTITIES],
}

impl SentSnap {
    fn empty() -> Self {
        Self {
            used: false,
            acked: false,
            tick: 0,
            entity_count: 0,
            removed_count: 0,
            entities: [EntityState::default(); MAX_SNAPSHOT_ENTITIES],
            removed: [0; MAX_SNAPSHOT_ENTITIES],
        }
    }

    pub fn entities(&self) -> &[EntityState] {
        &self.entities[..self.entity_count as usize]
    }

    pub fn removed(&self) -> &[u32] {
        &self.removed[..self.removed_count as usize]
    }
}

/// Ring index for a snapshot tick (snapshots land every
/// `SNAPSHOT_INTERVAL_TICKS`, so consecutive snapshots get consecutive
/// slots).
#[inline]
pub fn ring_index(tick: u32) -> usize {
    (tick as u64 / SNAPSHOT_INTERVAL_TICKS) as usize % SENT_SNAPSHOT_RING
}

pub struct ClientNetState {
    pub connected: bool,
    pub id: u32,
    /// This client's own slot in `World::players`, resolved after its join
    /// command lands (`usize::MAX` until then).
    pub own_wslot: usize,

    // --- interest + priority, indexed by world slot ---
    /// Which player id each per-slot entry was accrued for; a tenant change
    /// resets the entry (world slots are reused).
    pub tracked_id: [u32; MAX_PLAYERS],
    pub interest: [bool; MAX_PLAYERS],
    /// Priority accumulator (NETCODE.md §3): `+= w · 1/(1 + d/32 m)` per
    /// tick, reset to 0 on send. Send ordering only — never sim state, so
    /// f32 is fine here.
    pub accum: [f32; MAX_PLAYERS],
    /// Snapshots since last sent, per interest entity (staleness ceiling).
    pub unsent: [u8; MAX_PLAYERS],

    // --- the same three, for the animal roster (mob.rs) ---
    /// A parallel set rather than one widened array, and the reason is the
    /// removal rule above: a *world slot* is reused by a different player
    /// and needs `tracked_id` to notice, where a *roster slot* is one animal
    /// for the life of the shard — it dies and hatches again as itself, at
    /// the same id. There is no tenant change to watch for, so there is no
    /// third array here, and merging the two would have meant carrying a
    /// column for animals that answers a question they cannot ask.
    pub m_interest: [bool; MAX_MOBS],
    pub m_accum: [f32; MAX_MOBS],
    pub m_unsent: [u8; MAX_MOBS],

    // --- input buffer (NETCODE.md §4) ---
    in_frames: [InputFrame; INPUT_BUFFER_CAP],
    in_valid: [bool; INPUT_BUFFER_CAP],
    /// **The aim-staleness stamp** (`findings/lagcomp-design-20260818.md`
    /// §7 slice 1): for each buffered frame, the snapshot ack the datagram
    /// that first delivered it carried — the client saying *"the newest
    /// world I had applied when I made this frame is server tick S"*, as
    /// the low 16 bits of a server tick. `None` means "not a measurement":
    /// the client had not yet acked any snapshot this shard actually sent,
    /// so its ack field is the `(0, 0)` `ClientView::ack_fields` returns
    /// before the first snapshot lands, and subtracting it from the tick
    /// would report the shard's whole uptime as one player's lag.
    ///
    /// **Cap and overflow policy:** exactly `INPUT_BUFFER_CAP`, indexed
    /// identically to `in_frames` and `in_valid`, so it inherits their
    /// policy without adding one — a stamp cannot outlive its frame and a
    /// burst past the cap drops both together (drop-oldest, `push_frame`).
    /// It is a parallel array and not a field on `InputFrame` because
    /// `InputFrame` is the wire's type: this is a fact about the datagram
    /// the frame *arrived in*, which the frame itself does not carry.
    in_view: [Option<u16>; INPUT_BUFFER_CAP],
    pub last_executed: u16,
    got_input: bool,
    starve_ticks: u32,
    pub nudge: Nudge,

    // --- sent ring + acks + removals ---
    sent: [SentSnap; SENT_SNAPSHOT_RING],
    pub newest_acked: Option<u32>,
    pending_removals: [u32; PENDING_REMOVALS_CAP],
    pending_len: usize,

    // --- event lane (reliable bidi stream, protocol::event) ---
    /// Own inventory as last successfully queued to this client; the sim
    /// diffs the world's copy against it each tick and sends the changed
    /// slots. Only updated when the ring accepted the message, so a
    /// refused push re-diffs by itself.
    pub last_inv: [ItemStack; INV_SLOTS],
    /// Harvested-set walk: next `slot_lives` entry index to scan. At or
    /// past the store's len ⇒ the walk is done.
    pub sync_cursor: usize,
    /// The next sync batch carries the reset bit (fresh join or
    /// event-lane resync): the client clears its set before applying.
    pub sync_reset: bool,
    /// Next item index the catalog drip sends.
    pub catalog_cursor: usize,
    /// Next recipe row the recipe drip sends.
    pub recipes_cursor: usize,
    /// Next research row the tech-tree drip sends (tech tree v0).
    pub research_cursor: usize,
    /// Next piece-def row the build-menu drip sends.
    pub piece_defs_cursor: usize,
    /// Placed-piece walk: entries **still owed**, not the next index to
    /// send. The walk reads `world.pieces` from the tail down, so
    /// `[0, piece_sync_cursor)` is what this client has not been sent and
    /// zero means the walk is finished. A removal does not restart it —
    /// the entry a swap-remove moves is always one already sent — but it
    /// can leave this count past the end of a store that shrank under it,
    /// so `drip_client` clamps it where it reads it (`core.rs` carries the
    /// argument, `deploy_wire.rs` the gate).
    pub piece_sync_cursor: usize,
    /// The next piece batch carries the reset bit (fresh join or
    /// event-lane resync): the client clears its piece set first.
    pub piece_sync_reset: bool,
    /// Where this client's piece walk is **aimed**, in centimetres — the
    /// player position the walk was armed at, not where the player is now
    /// (class-S interest v0, `interest.rs` carries the argument).
    ///
    /// Fixed for a walk's duration on purpose. The filter has to answer the
    /// same way for every batch of one walk, or "this client has been sent
    /// everything within R" is a claim about a moving circle and means
    /// nothing. It is also what the `EV_PIECE_PLACED` broadcast tests
    /// against, so a piece placed after the walk finished is covered by the
    /// identical arithmetic rather than by a second opinion.
    pub piece_anchor_cm: (i64, i64),
    /// False until this client has a body to aim from — a connection whose
    /// join command is still queued has no position, and a walk aimed at
    /// the origin would stream the wrong corner of the island. The walk
    /// holds (it is owed a reset batch either way) and the placement
    /// broadcast passes unfiltered for that window, which is one tick.
    pub piece_anchor_valid: bool,
    /// Next deployable-def row the deploy-menu drip sends.
    pub deploy_defs_cursor: usize,
    /// Placed-deployable walk: the next `world.deploys` entry index to
    /// send, read upward, and a decay removal mid-walk restarts it (the
    /// store swap-removes). **Not** the piece walk's semantics any more —
    /// that one reads downward and never restarts, and this one is left as
    /// it was until its own placement seam is proven (`core.rs`).
    pub deploy_sync_cursor: usize,
    /// The next deploy batch carries the reset bit.
    pub deploy_sync_reset: bool,
    /// Standing-backpack walk cursor, restart semantics like the
    /// deployables' — a bag looted or despawned mid-walk swap-removes
    /// under it.
    pub bag_sync_cursor: usize,
    /// The next bag batch carries the reset bit.
    pub bag_sync_reset: bool,
    /// Which ground container this client has open, or `CONT_SELF` for
    /// none, with `open_cont_handle` naming it (a bag id, or a packed
    /// `box_key`) exactly as `Command::Move` names one.
    ///
    /// **A subscription, not a permission, and not sim state.** Nothing in
    /// `sim-core` reads it, no command carries it, it never enters the WAL
    /// and `World::state_hash` never sees it — a replay produces the same
    /// hash whether or not anyone had a panel open, which is the only
    /// answer that keeps wall 5 honest. It cannot grant anything either:
    /// `inventory::CONT_BAG` already states that reach is proved when a
    /// move resolves and never when a panel opened, and the container sync
    /// re-proves reach *every tick* against the same quantized positions
    /// the move verb will judge on. So the panel a client is fed is always
    /// a container that client could also move items in — which is the
    /// quantize-both-sides law (CLAUDE.md) applied to containers: a
    /// refusal must be computed on the values the client predicted with,
    /// and it is the same check that decided what it saw.
    pub open_cont_kind: u8,
    pub open_cont_handle: u32,
    /// The next container batch carries the reset bit: the whole container,
    /// forget what you had. Set by an open, cleared once a batch is away.
    pub open_cont_reset: bool,
    /// The open container's slots as last successfully queued to this
    /// client — the `last_inv` shadow, one container over. Sized to the
    /// widest container (`INV_SLOTS`); a box uses the first `BOX_SLOTS`
    /// and the tail stays zero on both sides, so it never manufactures a
    /// change.
    pub last_cont: [ItemStack; INV_SLOTS],
    /// One decoded C→S action awaiting its command slot (the sim drains
    /// the ring only into an empty hand — defer, never drop).
    pub pending_action: Option<ActionMsg>,
    /// One decoded C→S chat line awaiting its fan-out. Unlike the action
    /// hand this is never deferred: chat is not a transaction, so a line
    /// that can't be said this tick is dropped rather than held (the
    /// fan-out always takes it — see `ShardCore::pump_chat`).
    pub pending_chat: Option<ChatMsg>,
    /// Craft queue as last successfully queued to this client; the sim
    /// diffs the world's copy each tick, like `last_inv`.
    pub last_jobs: [CraftJob; CRAFT_QUEUE],
    /// The head-timer value behind the last queued queue message.
    /// `u64::MAX` forces a resend (the ev_resync path).
    pub last_done_at: u64,
}

impl ClientNetState {
    pub fn new() -> Self {
        Self {
            connected: false,
            id: 0,
            own_wslot: usize::MAX,
            tracked_id: [0; MAX_PLAYERS],
            interest: [false; MAX_PLAYERS],
            accum: [0.0; MAX_PLAYERS],
            unsent: [0; MAX_PLAYERS],
            m_interest: [false; MAX_MOBS],
            m_accum: [0.0; MAX_MOBS],
            m_unsent: [0; MAX_MOBS],
            in_frames: [InputFrame::default(); INPUT_BUFFER_CAP],
            in_valid: [false; INPUT_BUFFER_CAP],
            in_view: [None; INPUT_BUFFER_CAP],
            last_executed: 0,
            got_input: false,
            starve_ticks: 0,
            nudge: Nudge::Ok,
            sent: [SentSnap::empty(); SENT_SNAPSHOT_RING],
            newest_acked: None,
            pending_removals: [0; PENDING_REMOVALS_CAP],
            pending_len: 0,
            last_inv: [ItemStack::default(); INV_SLOTS],
            sync_cursor: 0,
            sync_reset: true,
            catalog_cursor: 0,
            recipes_cursor: 0,
            research_cursor: 0,
            piece_defs_cursor: 0,
            piece_sync_cursor: 0,
            piece_sync_reset: true,
            piece_anchor_cm: (0, 0),
            piece_anchor_valid: false,
            deploy_defs_cursor: 0,
            deploy_sync_cursor: 0,
            deploy_sync_reset: true,
            bag_sync_cursor: 0,
            bag_sync_reset: true,
            open_cont_kind: CONT_SELF,
            open_cont_handle: 0,
            open_cont_reset: false,
            last_cont: [ItemStack::default(); INV_SLOTS],
            pending_action: None,
            pending_chat: None,
            last_jobs: [CraftJob::default(); CRAFT_QUEUE],
            last_done_at: 0,
        }
    }

    /// Restart everything the event lane owes this client (fresh join and
    /// ring-overflow recovery are the same path): the harvested-set and
    /// placed-piece walks from the top with reset batches, the catalog /
    /// recipe / piece-def drips from row zero, and a forced craft-queue
    /// resend. The inventory shadow stays — it re-diffs against the world
    /// by itself.
    pub fn ev_resync(&mut self) {
        self.sync_cursor = 0;
        self.sync_reset = true;
        self.catalog_cursor = 0;
        self.recipes_cursor = 0;
        self.research_cursor = 0;
        self.piece_defs_cursor = 0;
        self.piece_sync_cursor = 0;
        self.piece_sync_reset = true;
        // The anchor is dropped with the walk it aimed: a resync re-arms
        // from where the player is *now*, which is the only position the
        // batch it is about to send can honestly claim to cover.
        self.piece_anchor_valid = false;
        self.deploy_defs_cursor = 0;
        self.deploy_sync_cursor = 0;
        self.deploy_sync_reset = true;
        self.bag_sync_cursor = 0;
        self.bag_sync_reset = true;
        // The open container is *closed*, not resynced. Every other line
        // here restarts a walk the client is owed; this one is the only
        // piece of event-lane state the client can hold an opinion about,
        // and after a ring overflow the two ends no longer agree on what
        // that opinion was. Re-sending contents for a panel the client may
        // have shut is worse than making it ask again — an open is one
        // nine-byte action away, and it is the client's press to make.
        self.close_container();
        self.last_done_at = u64::MAX;
    }

    /// Open `handle` of `kind` as this client's container view, or close
    /// whatever is open when `kind` is `CONT_SELF`. A re-open of the same
    /// container is a deliberate resync: the shadow is zeroed and the next
    /// batch carries `reset`, so the panel is redrawn from the truth
    /// rather than patched onto whatever it had.
    pub fn open_container(&mut self, kind: u8, handle: u32) {
        if kind == CONT_SELF {
            self.close_container();
            return;
        }
        self.open_cont_kind = kind;
        self.open_cont_handle = handle;
        self.open_cont_reset = true;
        self.last_cont = [ItemStack::default(); INV_SLOTS];
    }

    /// Nothing is open. The shadow is zeroed with it so the next open
    /// cannot diff against a stale container's slots.
    pub fn close_container(&mut self) {
        self.open_cont_kind = CONT_SELF;
        self.open_cont_handle = 0;
        self.open_cont_reset = false;
        self.last_cont = [ItemStack::default(); INV_SLOTS];
    }

    /// Arm the slot for a fresh connection. Everything netcode resets; the
    /// world-side join is the caller's command.
    pub fn reset(&mut self, id: u32) {
        *self = Self::new();
        self.connected = true;
        self.id = id;
    }

    // --- acks ---------------------------------------------------------

    /// Process the redundant ack header of one input datagram: mark ring
    /// entries acked, advance the newest-acked baseline, and clear pending
    /// removals a now-acked snapshot carried. Unknown low-16 ticks are
    /// ignored (the ring holds ≤ 64 ticks of history, far under the u16
    /// ambiguity window).
    pub fn on_acks(&mut self, snapshot_ack: u16, ack_bits: u32) {
        for i in 0..SENT_SNAPSHOT_RING {
            let (used, acked, tick) = {
                let s = &self.sent[i];
                (s.used, s.acked, s.tick)
            };
            if !used || acked {
                continue;
            }
            let diff = snapshot_ack.wrapping_sub(tick as u16) as u32;
            let hit = diff == 0 || (1..=32).contains(&diff) && (ack_bits >> (diff - 1)) & 1 == 1;
            if !hit {
                continue;
            }
            self.sent[i].acked = true;
            if self.newest_acked.is_none_or(|b| tick > b) {
                self.newest_acked = Some(tick);
            }
            let removed_count = self.sent[i].removed_count as usize;
            for r in 0..removed_count {
                let id = self.sent[i].removed[r];
                self.pending_remove(id);
            }
        }
    }

    /// The delta baseline for a snapshot at `tick`: the newest acked sent
    /// snapshot, if it still lives in the ring and its age fits the wire's
    /// u8. Otherwise the canonical zero-state.
    pub fn baseline(&self, tick: u64) -> Option<(u8, &SentSnap)> {
        let b = self.newest_acked?;
        let age = tick.checked_sub(b as u64)?;
        if age == 0 || age > u8::MAX as u64 {
            return None;
        }
        let s = &self.sent[ring_index(b)];
        if s.used && s.tick == b {
            Some((age as u8, s))
        } else {
            None
        }
    }

    /// Record what a snapshot actually carried (post-shed), so a future
    /// ack of it can become a baseline and clear its removals.
    pub fn record_sent(&mut self, tick: u64, entities: &[EntityState], removed: &[u32]) {
        let s = &mut self.sent[ring_index(tick as u32)];
        s.used = true;
        s.acked = false;
        s.tick = tick as u32;
        s.entity_count = entities.len() as u8;
        s.removed_count = removed.len() as u8;
        s.entities[..entities.len()].copy_from_slice(entities);
        s.removed[..removed.len()].copy_from_slice(removed);
    }

    /// Forget all sent/acked history: the next snapshot is zero-state, and
    /// only post-resync acks can seed a baseline. The client clears its
    /// class-D map when it applies a zero-state snapshot, so pending
    /// removals go too.
    pub fn force_resync(&mut self) {
        for s in self.sent.iter_mut() {
            s.used = false;
        }
        self.newest_acked = None;
        self.pending_len = 0;
    }

    // --- pending removals --------------------------------------------

    /// Track an id the client may know that just left its world. Returns
    /// `false` on overflow — the caller must `force_resync` (the honest
    /// escape hatch; the zero-state clears the client's map instead).
    #[must_use]
    pub fn pending_add(&mut self, id: u32) -> bool {
        if self.pending_removals[..self.pending_len].contains(&id) {
            return true;
        }
        if self.pending_len == PENDING_REMOVALS_CAP {
            return false;
        }
        self.pending_removals[self.pending_len] = id;
        self.pending_len += 1;
        true
    }

    pub fn pending_remove(&mut self, id: u32) {
        if let Some(pos) = self.pending_removals[..self.pending_len]
            .iter()
            .position(|&p| p == id)
        {
            self.pending_removals[pos] = self.pending_removals[self.pending_len - 1];
            self.pending_len -= 1;
        }
    }

    pub fn pending(&self) -> &[u32] {
        &self.pending_removals[..self.pending_len]
    }

    // --- input buffer -------------------------------------------------

    /// Buffer one frame by seq, with the snapshot ack of the datagram it
    /// rode in on (`in_view`; `None` when the client has not yet acked a
    /// snapshot this shard sent). The first frame ever anchors the seq
    /// window; duplicates and stale seqs drop; a burst beyond the buffer
    /// cap skips the window forward (drop-oldest — the client got ahead of
    /// a server stall, and old inputs are the wrong thing to honor).
    ///
    /// **The stamp is keep-first and that had to be written, not
    /// inherited.** `findings/lagcomp-design-20260818.md` §2.2 says
    /// `push_frame` "drops a frame it has already seen", so the stamp
    /// would be the first datagram's for free. It does not: the guard
    /// below drops a frame already *executed* or ancient, and a frame that
    /// is buffered-but-unexecuted is **overwritten** by the retransmit
    /// tail of the next datagram. Left alone, the stamp would therefore
    /// track the newest datagram to mention the frame — a fresher ack, so
    /// a smaller `T − S`, so a systematic *understatement* of staleness on
    /// every frame that waits a tick in the buffer, which is all of them
    /// under the consume throttle. Keeping the first arrival makes the
    /// stamp say what the client knew when it made the frame. The one
    /// place it is still generous is a frame whose original datagram was
    /// lost and which only ever arrives inside a retransmit tail: that one
    /// is stamped too new and measures too little, which is the safe
    /// direction (it under-favours the shooter on inputs that were already
    /// lost) and is the half of §2.2's claim that survives.
    pub fn push_frame(&mut self, f: InputFrame, view: Option<u16>) {
        if !self.got_input {
            self.got_input = true;
            self.last_executed = f.seq.wrapping_sub(1);
        }
        let ahead = f.seq.wrapping_sub(self.last_executed);
        if ahead == 0 || ahead > 0x7FFF {
            return; // executed already, or ancient
        }
        if ahead as usize > INPUT_BUFFER_CAP {
            self.last_executed = f.seq.wrapping_sub(INPUT_BUFFER_CAP as u16);
        }
        let slot = f.seq as usize % INPUT_BUFFER_CAP;
        // A repeat of a seq already sitting in this slot keeps the stamp it
        // arrived with; a different seq taking the slot takes the new one.
        let repeat = self.in_valid[slot] && self.in_frames[slot].seq == f.seq;
        self.in_frames[slot] = f;
        self.in_valid[slot] = true;
        if !repeat {
            self.in_view[slot] = view;
        }
    }

    fn buffered_depth(&self) -> usize {
        let mut depth = 0;
        for i in 0..INPUT_BUFFER_CAP {
            if self.in_valid[i] {
                let ahead = self.in_frames[i].seq.wrapping_sub(self.last_executed);
                if ahead >= 1 && ahead as usize <= INPUT_BUFFER_CAP {
                    depth += 1;
                }
            }
        }
        depth
    }

    fn take_next(&mut self) -> Option<(InputFrame, Option<u16>)> {
        let want = self.last_executed.wrapping_add(1);
        let slot = want as usize % INPUT_BUFFER_CAP;
        if self.in_valid[slot] && self.in_frames[slot].seq == want {
            self.in_valid[slot] = false;
            self.last_executed = want;
            return Some((self.in_frames[slot], self.in_view[slot].take()));
        }
        None
    }

    /// Oldest buffered seq ahead of the cursor, if any — the gap-jump
    /// target.
    fn oldest_ahead(&self) -> Option<u16> {
        let mut best: Option<u16> = None;
        for i in 0..INPUT_BUFFER_CAP {
            if !self.in_valid[i] {
                continue;
            }
            let seq = self.in_frames[i].seq;
            let ahead = seq.wrapping_sub(self.last_executed);
            let in_window = ahead >= 1 && ahead as usize <= INPUT_BUFFER_CAP;
            if in_window && best.is_none_or(|b| ahead < b.wrapping_sub(self.last_executed)) {
                best = Some(seq);
            }
        }
        best
    }

    /// One tick's consume (NETCODE.md §4): normally one frame; two when
    /// the buffer runs deep (the consume throttle); a gap with frames
    /// behind it jumps — 10-frame redundancy means a gap is a ≥ 10-datagram
    /// loss burst, not one missing packet. Returns the frame to execute
    /// this tick (`None` ⇒ reuse last, which the sim does by keeping
    /// `Player::frame`) **with its aim-staleness stamp** (`in_view`), and
    /// refreshes the nudge.
    ///
    /// The stamp rides the return rather than a field the caller reads
    /// afterwards, because a second reader of a value handed over once is
    /// the destructive-read defect `CLAUDE.md` keeps a trap entry for. Two
    /// frames consumed in one tick (the throttle) report the stamp of the
    /// one that **executes**, which is the newer of the two — the older
    /// frame's aim never reaches the world, so measuring it would price a
    /// swing nobody swung.
    pub fn consume_input(&mut self) -> Option<(InputFrame, Option<u16>)> {
        if !self.got_input {
            self.nudge = Nudge::Ok;
            return None;
        }
        let to_consume = if self.buffered_depth() > INPUT_THROTTLE_DEPTH {
            2
        } else {
            1
        };
        let mut executed: Option<(InputFrame, Option<u16>)> = None;
        for _ in 0..to_consume {
            if let Some(f) = self.take_next() {
                executed = Some(f);
            } else if executed.is_none() {
                if let Some(seq) = self.oldest_ahead() {
                    self.last_executed = seq.wrapping_sub(1);
                    executed = self.take_next();
                }
            }
        }
        let depth_after = self.buffered_depth();
        if executed.is_none() {
            self.starve_ticks += 1;
        } else {
            self.starve_ticks = 0;
        }
        self.nudge = if self.starve_ticks > STARVE_HARD_RESYNC_TICKS {
            Nudge::HardResync
        } else if depth_after == 0 {
            Nudge::Faster
        } else if depth_after <= 2 {
            Nudge::Ok
        } else {
            Nudge::Slower
        };
        executed
    }
}

impl Default for ClientNetState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(seq: u16) -> InputFrame {
        InputFrame {
            seq,
            ..InputFrame::default()
        }
    }

    #[test]
    fn input_buffer_executes_in_order_and_dedupes() {
        let mut c = ClientNetState::new();
        c.reset(1);
        c.push_frame(frame(100), None);
        c.push_frame(frame(101), None);
        c.push_frame(frame(100), None); // dup after anchor: stale, dropped
        assert_eq!(c.consume_input().unwrap().0.seq, 100);
        assert_eq!(c.consume_input().unwrap().0.seq, 101);
        assert!(c.consume_input().is_none());
        assert_eq!(c.nudge, Nudge::Faster);
    }

    #[test]
    fn throttle_consumes_two_when_deep() {
        let mut c = ClientNetState::new();
        c.reset(1);
        for s in 0..10u16 {
            c.push_frame(frame(s), None);
        }
        // depth 10 > 6: two consumed, the newer executes.
        assert_eq!(c.consume_input().unwrap().0.seq, 1);
        assert_eq!(c.nudge, Nudge::Slower);
    }

    #[test]
    fn gap_jumps_to_oldest_ahead() {
        let mut c = ClientNetState::new();
        c.reset(1);
        c.push_frame(frame(5), None);
        assert_eq!(c.consume_input().unwrap().0.seq, 5);
        // 6..=9 lost forever; 10 arrives.
        c.push_frame(frame(10), None);
        assert_eq!(c.consume_input().unwrap().0.seq, 10);
        assert_eq!(c.last_executed, 10);
    }

    #[test]
    fn acks_pick_newest_baseline_and_clear_removals() {
        let mut c = ClientNetState::new();
        c.reset(1);
        let e = [EntityState {
            id: 9,
            ..EntityState::default()
        }];
        c.record_sent(2, &e, &[7]);
        c.record_sent(4, &e, &[]);
        assert!(c.pending_add(7));
        c.on_acks(4, 0b10); // acks tick 4 and (bit 1) tick 2
        assert_eq!(c.newest_acked, Some(4));
        assert!(c.pending().is_empty(), "ack of tick 2 clears removal 7");
        let (age, snap) = c.baseline(10).unwrap();
        assert_eq!(age, 6);
        assert_eq!(snap.tick, 4);
    }

    #[test]
    fn ack_of_unknown_tick_is_ignored() {
        let mut c = ClientNetState::new();
        c.reset(1);
        c.record_sent(2, &[], &[]);
        c.on_acks(40_000, 0xFFFF_FFFF);
        assert_eq!(c.newest_acked, None);
    }

    #[test]
    fn resync_forgets_history() {
        let mut c = ClientNetState::new();
        c.reset(1);
        c.record_sent(2, &[], &[]);
        c.on_acks(2, 0);
        assert!(c.baseline(4).is_some());
        c.force_resync();
        assert!(c.baseline(4).is_none());
        c.on_acks(2, 0); // stale ack finds no ring entry
        assert!(c.baseline(4).is_none());
    }
}
