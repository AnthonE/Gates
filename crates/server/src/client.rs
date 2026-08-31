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
use sim_core::input::{self, InputFrame};
use sim_core::inventory::{CONT_SELF, CONT_WEAR};
use sim_core::limits::{
    CRAFT_QUEUE, INPUT_BUFFER_CAP, INPUT_THROTTLE_DEPTH, INV_SLOTS, MAX_MOBS, MAX_PLAYERS,
    MAX_SNAPSHOT_ENTITIES, PENDING_REMOVALS_CAP, SENT_SNAPSHOT_RING, SNAPSHOT_INTERVAL_TICKS,
    WEAR_SLOTS,
};

/// Consecutive starved ticks before the nudge escalates to `HardResync`
/// (NETCODE.md §4's "buffer empty" case, debounced so one lost datagram
/// doesn't trigger it). Proposed default, DECISIONS.md §open.
pub const STARVE_HARD_RESYNC_TICKS: u32 = 30;

/// One tick's executed input (`consume_input`): the frame that acts, the
/// aim-staleness stamp of the datagram that first delivered it, and — on a
/// throttle tick — the OLDER consumed frame, which must still move the
/// body (`world.rs Command::InputPair`; the ring the client reconciles
/// against stepped every seq exactly once).
pub struct Consumed {
    pub frame: InputFrame,
    pub view: Option<u16>,
    pub prev: Option<InputFrame>,
}

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
    /// Datagrams from this client whose claimed view sat further behind the
    /// server's own evidence than reordering explains (lagcomp slice 5,
    /// `ShardCore::push_input`). **A relay, not a statistic**: it is written
    /// on the datagram path and drained to `ShardStats::favour_disagree` by
    /// the tick loop, because `push_input` has no `&ShardStats` and giving
    /// it one to carry a single counter would thread a parameter through
    /// nineteen call sites for a diagnostic.
    ///
    /// **Cap and overflow policy: saturating.** A `u16` holds 65 535
    /// disagreements between two ticks, which no client can send — the
    /// input ring is `INPUT_BUFFER_CAP` deep. Saturating rather than
    /// wrapping so that if one ever could, the counter pins at "very many"
    /// instead of rolling to zero and reporting innocence.
    ///
    /// Drained destructively by exactly one reader
    /// (`take_ack_regressions`), which is the single-consumer contract
    /// `CLAUDE.md` keeps a trap entry for; `tests/sound.rs`'s scrape is
    /// about `ClientCore`'s rings and does not reach here, so the owner is
    /// named in this sentence and gated by
    /// `the_disagreement_relay_has_one_reader`.
    ack_regressions: u16,
    pub last_executed: u16,
    got_input: bool,
    starve_ticks: u32,
    /// The last REAL frame this connection executed — the decay mint's
    /// source (`core.rs`, the starved branch). Deliberately a server-side
    /// copy and not the world's `Player::frame`: the mint overwrites the
    /// world's copy with each decayed ghost, and scaling THAT would
    /// compound (two thirds of two thirds). This one only moves when a
    /// frame the client actually sent executes.
    last_real: InputFrame,
    /// Post-consume input-buffer depth, cached at consume time for the
    /// snapshot header (`SnapshotHeader::buffered_depth`) — `buffered_depth()`
    /// is an O(cap) scan and the header is built later in the same tick.
    /// Saturated to the wire's 4-bit field, and the saturation is policy,
    /// not a silent clamp: this is a report of a bounded gauge (the buffer
    /// caps at `INPUT_BUFFER_CAP`), and 15 already means "far past every
    /// threshold that matters" (`INPUT_THROTTLE_DEPTH` is 6).
    depth_report: u8,
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
    /// The **body's** slots as last successfully queued — `last_cont`'s
    /// twin, for a stream that runs beside the ground container rather
    /// than taking its place.
    ///
    /// `CONT_WEAR` is the one kind that is `inventory::is_own`: it has no
    /// handle, no store to resolve, no reach and no lock, so the three
    /// reasons the subscription above is exclusive — an address, a
    /// distance and a permission — none of them apply to it. Sharing one
    /// slot with a box therefore bought nothing and cost the move the
    /// feature exists for: a helmet out of a raided box could not reach a
    /// head without closing the box first (`NOW.md` §0eq item 4, the
    /// merge-gate judge's pass `-06` fix 2).
    ///
    /// It is implicit and permanent rather than opened: every live player
    /// has exactly one body, always addressable, so there is no press for
    /// the client to make and nothing for a close to mean. The cost is a
    /// two-slot diff per tick against this shadow, which sends nothing
    /// while nothing changes — the same arithmetic `last_cont` already
    /// pays, over 2 slots instead of 30.
    pub last_wear: [ItemStack; WEAR_SLOTS],
    /// The next wear batch carries the reset bit: the whole body, forget
    /// what you had. Armed by a fresh slot and by `ev_resync`, cleared
    /// once a batch is away.
    pub wear_reset: bool,
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
            ack_regressions: 0,
            last_executed: 0,
            got_input: false,
            starve_ticks: 0,
            last_real: InputFrame::default(),
            depth_report: 0,
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
            last_wear: [ItemStack::default(); WEAR_SLOTS],
            wear_reset: true,
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
        // The **body**, unlike the container above, is resynced rather
        // than dropped. The distinction is whose opinion the state is: an
        // open box is the client's press and after a ring overflow the two
        // ends no longer agree it happened, so making it ask again is the
        // honest answer. A body is not a press — it is a fact about a
        // player the server can always name — so there is nothing for the
        // client to re-ask and no panel it may have shut.
        self.resync_wear();
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
        // The body has its own stream and cannot be *opened* into this
        // one — a client that asks is asking for a resync of something it
        // is already being fed, which is exactly what an old client's
        // `open_worn` press means and the only thing it can honestly be
        // answered with. Taking the ground slot for it would put the two
        // views back in one place, which is the defect this pair exists
        // to remove: a box open would evict the body again.
        if kind == CONT_WEAR {
            self.resync_wear();
            return;
        }
        self.open_cont_kind = kind;
        self.open_cont_handle = handle;
        self.open_cont_reset = true;
        self.last_cont = [ItemStack::default(); INV_SLOTS];
    }

    /// Send the whole body next tick. The shadow is zeroed so the reset
    /// batch is built from the truth rather than diffed against whatever
    /// this client was last told.
    pub fn resync_wear(&mut self) {
        self.wear_reset = true;
        self.last_wear = [ItemStack::default(); WEAR_SLOTS];
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

    /// One datagram claimed a staler view than this connection's own ack
    /// history supports, by more than the band that absorbs reordering
    /// (`ShardCore::push_input`). Saturating, per the field's policy.
    pub fn note_ack_regression(&mut self) {
        self.ack_regressions = self.ack_regressions.saturating_add(1);
    }

    /// Drain the relay. **The one reader** — `ShardCore::tick`, once per
    /// client per tick, folding into `ShardStats::favour_disagree`. A
    /// second caller would silently halve the shard's count of the only
    /// lag-compensation signal that accuses anybody, which is the
    /// destructive-read defect `CLAUDE.md` keeps an entry for; the gate is
    /// a grep for this name, not a value assertion.
    pub fn take_ack_regressions(&mut self) -> u64 {
        core::mem::take(&mut self.ack_regressions) as u64
    }

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
    /// loss burst, not one missing packet. Returns what to execute this
    /// tick **with the aim-staleness stamp** (`in_view`) of the frame that
    /// acts, and refreshes the nudge. `None` ⇒ starved: the caller mints
    /// the decayed reuse (`ghost_frame`) — the sim would otherwise re-run
    /// the stale `Player::frame` verbatim at full strength forever, which
    /// is the overshoot a player feels as a snap on every stop.
    ///
    /// **Both consumed frames execute.** When the throttle takes two, the
    /// older rides back as `prev` and steps movement first
    /// (`world.rs Command::InputPair`) — this method consumed two and
    /// returned one until netcode v2 (DECISIONS.md 2026-08-31), which
    /// marked the older seq executed while its movement never ran: a
    /// guaranteed one-tick misprediction on every throttle tick, since the
    /// client's ring steps every seq exactly once.
    ///
    /// The stamp rides the return rather than a field the caller reads
    /// afterwards, because a second reader of a value handed over once is
    /// the destructive-read defect `CLAUDE.md` keeps a trap entry for. Two
    /// frames consumed in one tick report the stamp of the NEWER — the
    /// older frame's buttons never act, so measuring its aim would price a
    /// swing nobody swung.
    pub fn consume_input(&mut self) -> Option<Consumed> {
        if !self.got_input {
            self.nudge = Nudge::Ok;
            self.depth_report = 0;
            return None;
        }
        let to_consume = if self.buffered_depth() > INPUT_THROTTLE_DEPTH {
            2
        } else {
            1
        };
        let mut first: Option<(InputFrame, Option<u16>)> = None;
        let mut second: Option<(InputFrame, Option<u16>)> = None;
        for _ in 0..to_consume {
            if let Some(f) = self.take_next() {
                if first.is_none() {
                    first = Some(f);
                } else {
                    second = Some(f);
                }
            } else if first.is_none() {
                if let Some(seq) = self.oldest_ahead() {
                    self.last_executed = seq.wrapping_sub(1);
                    first = self.take_next();
                }
            }
        }
        let depth_after = self.buffered_depth();
        self.depth_report = depth_after.min(0xF) as u8;
        let executed = match (first, second) {
            (Some((prev, _)), Some((frame, view))) => Some(Consumed {
                frame,
                view,
                prev: Some(prev),
            }),
            (Some((frame, view)), None) => Some(Consumed {
                frame,
                view,
                prev: None,
            }),
            (None, _) => None,
        };
        if let Some(c) = &executed {
            self.starve_ticks = 0;
            self.last_real = c.frame;
        } else {
            self.starve_ticks += 1;
        }
        // `HardResync` is the rung that still steers (netcode v2 S4):
        // the client's clock runs a proportional controller on the
        // header's `buffered_depth` gauge now, so `Faster`/`Slower` are
        // stamped for the wire's continuity and diagnostics and ignored
        // by the shipped client.
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

    /// The starved tick's stand-in (netcode v2): the last REAL frame this
    /// connection executed, decayed by how long it has been re-covering
    /// for the missing one — `sim_core::input::decay_frame`'s 2/3 → 1/3 →
    /// 0 ramp on movement, buttons cleared except the light latch, look
    /// held. `None` once the ramp is spent: after `DECAY_STEPS` mints the
    /// world's stored frame IS the fully decayed one, so the sim's
    /// implicit reuse of it is bit-identical to minting on — the mint
    /// stops paying command budget the moment it stops changing anything.
    /// Also `None` before the first real frame (nothing to decay) — the
    /// sim's zero-frame default already stands still.
    pub fn ghost_frame(&self) -> Option<InputFrame> {
        if !self.got_input || self.starve_ticks == 0 || self.starve_ticks > input::DECAY_STEPS {
            return None;
        }
        Some(input::decay_frame(&self.last_real, self.starve_ticks))
    }

    /// The snapshot header's two netcode-v2 gauges: post-consume buffer
    /// depth (4-bit saturating — the field doc in `client.rs` states the
    /// policy) and consecutive starved ticks (3-bit saturating; ≥ 7 means
    /// "long past fully decayed").
    pub fn depth_report(&self) -> u8 {
        self.depth_report
    }

    pub fn repeat_report(&self) -> u8 {
        self.starve_ticks.min(7) as u8
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
        assert_eq!(c.consume_input().unwrap().frame.seq, 100);
        assert_eq!(c.consume_input().unwrap().frame.seq, 101);
        assert!(c.consume_input().is_none());
        assert_eq!(c.nudge, Nudge::Faster);
    }

    /// The throttle consumes two and hands BOTH back — the newer as the
    /// tick's frame, the older as `prev`, owed a movement step
    /// (`world.rs Command::InputPair`). This asserted "the newer executes"
    /// alone until netcode v2: the older seq was marked executed while its
    /// movement never ran, one guaranteed misprediction per throttle tick.
    #[test]
    fn throttle_consumes_two_and_returns_both() {
        let mut c = ClientNetState::new();
        c.reset(1);
        for s in 0..10u16 {
            c.push_frame(frame(s), None);
        }
        // depth 10 > 6: two consumed, both handed back, oldest first.
        let got = c.consume_input().unwrap();
        assert_eq!(got.prev.unwrap().seq, 0);
        assert_eq!(got.frame.seq, 1);
        assert_eq!(c.last_executed, 1);
        assert_eq!(c.nudge, Nudge::Slower);
        assert_eq!(c.depth_report(), 8);
    }

    #[test]
    fn gap_jumps_to_oldest_ahead() {
        let mut c = ClientNetState::new();
        c.reset(1);
        c.push_frame(frame(5), None);
        assert_eq!(c.consume_input().unwrap().frame.seq, 5);
        // 6..=9 lost forever; 10 arrives.
        c.push_frame(frame(10), None);
        assert_eq!(c.consume_input().unwrap().frame.seq, 10);
        assert_eq!(c.last_executed, 10);
    }

    /// A starved tick mints the decayed stand-in off the last REAL frame —
    /// never off the world's stored copy, which the mint itself replaces
    /// (two thirds of two thirds is the compounding this field exists to
    /// refuse) — and the mint stops once the ramp is spent, because from
    /// there the sim's implicit reuse is bit-identical to minting on.
    #[test]
    fn starvation_mints_the_decay_ramp_then_stops() {
        use sim_core::input::BTN_SPRINT;
        let mut c = ClientNetState::new();
        c.reset(1);
        assert!(c.ghost_frame().is_none(), "nothing to decay before input");
        let real = InputFrame {
            seq: 4,
            move_z: 127,
            buttons: BTN_SPRINT,
            yaw: 900,
            ..InputFrame::default()
        };
        c.push_frame(real, None);
        assert_eq!(c.consume_input().unwrap().frame.seq, 4);
        assert!(c.ghost_frame().is_none(), "a fed tick mints nothing");
        // Three starved ticks walk the ramp; the fourth mints nothing.
        let mut walked = Vec::new();
        for _ in 0..4 {
            assert!(c.consume_input().is_none());
            walked.push(c.ghost_frame());
        }
        let g1 = walked[0].unwrap();
        assert_eq!((g1.move_z, g1.buttons, g1.yaw), (84, 0, 900));
        assert_eq!(walked[1].unwrap().move_z, 42);
        assert_eq!(walked[2].unwrap().move_z, 0);
        assert!(
            walked[3].is_none(),
            "ramp spent: implicit reuse is identical"
        );
        assert_eq!(c.repeat_report(), 4);
        // A fresh real frame re-arms the ramp from the new movement.
        c.push_frame(
            InputFrame {
                seq: 5,
                move_x: -90,
                ..InputFrame::default()
            },
            None,
        );
        assert_eq!(c.consume_input().unwrap().frame.seq, 5);
        assert!(c.consume_input().is_none());
        assert_eq!(c.ghost_frame().unwrap().move_x, -60);
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
