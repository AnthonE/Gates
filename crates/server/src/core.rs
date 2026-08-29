//! ShardCore: the sim thread's whole world — `sim_core::World` plus every
//! client's netcode state, AOI, priority fill, and snapshot encoding into
//! caller-provided sends. Pure: no I/O, no clock, no locks; the net layer
//! drives it through rings and it allocates only in `new` (L1–L4). Tests
//! drive it directly, no sockets required.

use crate::client::ClientNetState;
use crate::interest::{self, PIECE_SCAN_BATCH};
use crate::stats::ShardStats;
use crate::store::PlayerKey;
use protocol::{
    encode_event_auth, encode_event_bag_dropped, encode_event_bag_removed, encode_event_bag_sync,
    encode_event_bags, encode_event_build_refused, encode_event_catalog,
    encode_event_charge_placed, encode_event_chat, encode_event_consume_refused,
    encode_event_consumed, encode_event_cont_sync, encode_event_craft_done, encode_event_craft_q,
    encode_event_craft_refused, encode_event_death, encode_event_deploy_defs,
    encode_event_deploy_placed, encode_event_deploy_refused, encode_event_deploy_sync,
    encode_event_door, encode_event_drank, encode_event_gather, encode_event_gather_refused,
    encode_event_health, encode_event_hit, encode_event_impact, encode_event_inv,
    encode_event_knock, encode_event_known, encode_event_move_refused, encode_event_moved,
    encode_event_oven, encode_event_piece_defs, encode_event_piece_placed,
    encode_event_piece_repaired, encode_event_piece_sync, encode_event_recipes,
    encode_event_removed, encode_event_research, encode_event_research_refused,
    encode_event_research_rows, encode_event_respawn, encode_event_shot, encode_event_slot_change,
    encode_event_slot_sync, encode_event_stock, encode_event_struct_hit, encode_event_swing,
    encode_event_vitals, encode_event_weak_mark, ActionMsg, ChatMsg, EntityState, InputDatagram,
    InvSlot, ItemCatalog, SnapshotEncoder, SnapshotHeader, WireBag, WireError, BAG_SYNC_BATCH,
    CONT_SYNC_BATCH, DEPLOY_SYNC_BATCH, MAX_EVENT_MSG_BYTES, PIECE_SYNC_BATCH, SLOT_SYNC_BATCH,
};
use sim_core::backpack::BAG_GONE_MAX;
use sim_core::build::{damage_band, BuildContent, PieceRec, LOC_PLANE};
use sim_core::craft::CraftJob;
use sim_core::deploy::{BagAnchor, DeployContent, DeployRec, BAG_CAP};
use sim_core::gather::{ItemStack, NO_ITEM};
use sim_core::inventory::{slots_in, CONT_BAG, CONT_BOX, CONT_SELF, CONT_WEAR, CONT_WORLD};
use sim_core::limits::{
    AOI_ENTER_CM, AOI_EXIT_CM, AOI_RANK_ENTER, AOI_RANK_EXIT, CHAT_LOCAL_CM, CRAFT_QUEUE,
    DATAGRAM_BUDGET_BYTES, HEARTH_STOCK_ROWS, INV_SLOTS, MAX_COMMANDS_PER_TICK, MAX_MOBS,
    MAX_PLAYERS, MAX_SNAPSHOT_ENTITIES, SNAPSHOT_INTERVAL_TICKS, STALENESS_CEILING,
    SYNC_SCAN_PER_TICK, WEAR_SLOTS,
};
use sim_core::mob;
use sim_core::persist::PlayerSave;
use sim_core::survival::REFUSE_C_MAX;
use sim_core::world::{
    Command, Player, World, DEATH_BY_CLOCK, EV_AUTH, EV_BAG_DROPPED, EV_BAG_REMOVED,
    EV_BUILD_REFUSED, EV_CHARGE_PLACED, EV_CONSUMED, EV_CONSUME_REFUSED, EV_CRAFT_DONE,
    EV_CRAFT_REFUSED, EV_DEATH, EV_DEPLOY_PLACED, EV_DEPLOY_REFUSED, EV_DEPLOY_REMOVED, EV_DOOR,
    EV_DRANK, EV_GATHER, EV_GATHER_REFUSED, EV_HEALTH, EV_HIT, EV_IMPACT, EV_KNOCK, EV_KNOWN,
    EV_MOVED, EV_MOVE_REFUSED, EV_OVEN, EV_PIECE_PLACED, EV_PIECE_REMOVED, EV_PIECE_REPAIRED,
    EV_RESEARCH, EV_RESEARCH_REFUSED, EV_RESPAWN, EV_SHOT, EV_SLOT_HARVESTED, EV_SLOT_RESPAWNED,
    EV_STOCK, EV_STRUCT_HIT, EV_SWING, EV_VITALS, EV_WEAK_MARK, STRUCT_DEPLOY_BIT,
};

/// A piece row's baked maximum hp, or 0 if the row is past the table.
///
/// 0 is not a fallback here, it is the answer `damage_band` wants: an
/// unknown maximum reports "untouched" rather than a fraction of nothing,
/// which is `hud::struct_hit_line`'s rule for the same problem.
fn piece_hp_max(bc: &BuildContent, row: u8) -> u16 {
    if (row as u16) < bc.piece_count {
        bc.pieces[row as usize].hp
    } else {
        0
    }
}

/// The same for a deployable row.
fn deploy_hp_max(dc: &DeployContent, row: u8) -> u16 {
    if (row as u16) < dc.def_count {
        dc.defs[row as usize].hp
    } else {
        0
    }
}

/// Unpack `sim_core::inventory::addr` — from kind, from slot, to kind, to
/// slot. One function, used by both move events, so the two can never
/// disagree about which byte is which.
fn addr_parts(addr: u32) -> (u8, u8, u8, u8) {
    (
        (addr >> 24) as u8,
        (addr >> 16) as u8,
        (addr >> 8) as u8,
        addr as u8,
    )
}

/// Priority accumulator v0 weights (NETCODE.md §3): players w=100; the
/// distance falloff half-scale is 32 m. Other classes land with their
/// entities.
const PRIORITY_W_PLAYER: f32 = 100.0;
const PRIORITY_HALF_SCALE_M: f32 = 32.0;

/// Animals accrue at a quarter of a player's rate (`mob.rs`).
///
/// Not a guess about how interesting a pig is: it is the shed order stated
/// where the shed happens. A snapshot that cannot carry everything must drop
/// the animal before the player, because a player's position is what
/// prediction reconciles and combat is fought on, and an animal's is what an
/// interpolator smooths. A quarter puts a pig at 15 m on par with a player
/// at 96 m, which is the trade this weight is claiming.
const PRIORITY_W_MOB: f32 = 25.0;

/// Consecutive byte-overflow refusals before the fill loop stops trying
/// smaller records (bounded work per snapshot, not a wire number).
const FILL_OVERFLOW_STREAK: u32 = 3;

/// Which pipe `tick`'s send closure should put the bytes on. Snapshots
/// ride datagrams (lossy, superseding); events ride the reliable bidi
/// stream. The closure returns whether the bytes were accepted — only the
/// event lane acts on a refusal (ring full ⇒ `ev_resync`, the same
/// recovery a fresh join uses).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lane {
    Snapshot,
    Event,
}

pub struct ShardCore {
    pub world: World,
    pub clients: Box<[ClientNetState]>,
    /// Joins/leaves queued between ticks (accept/cleanup driven). Overflow
    /// policy: refuse — the caller retries next tick.
    ///
    /// Boxed, with `cmd_buf` below, for the reason `World` boxes `backpacks`:
    /// `ShardCore` is built on the stack (`ShardCore::new`, every wire test)
    /// and a `Command` is no longer 16 bytes. `Command::JoinAs` carries a
    /// 188-byte `PlayerSave`, so the enum is ~192 and each of these buffers is
    /// ~49 kB — 98 kB of stack the `*_wire` suites were within a documented
    /// margin of not having (`CLAUDE.md`: they already need `RUST_MIN_STACK`
    /// on the reference box). Two construction-time allocations, none in the
    /// tick (wall 2), and the stack footprint ends up *smaller* than before
    /// the restore existed.
    queued: Box<[Command]>,
    queued_len: usize,
    /// The tick's own command list: what `queued` plus this tick's inputs and
    /// actions add up to, handed to `World::tick` as one slice. A field rather
    /// than a local for the boxing above — and it borrow-splits cleanly against
    /// `clients` and `world` because they are all distinct fields of `self`.
    cmd_buf: Box<[Command]>,
    /// Scratch: baseline copy (borrow-splits the client during encode).
    baseline_buf: [EntityState; MAX_SNAPSHOT_ENTITIES],
    /// Scratch: what actually got encoded, for `record_sent`.
    sent_buf: [EntityState; MAX_SNAPSHOT_ENTITIES],
    removed_buf: [u32; MAX_SNAPSHOT_ENTITIES],
    /// Scratch: encode target; the closure receives its bytes.
    dg_buf: [u8; DATAGRAM_BUDGET_BYTES],
    /// Item display names with their condition ceilings (v46) and armor
    /// columns (v52), for the catalog drip. Boot input like the baked gather table: the shard installs it
    /// before the first tick; empty (the default) sends no catalog, which
    /// is what content-less tests run under.
    pub catalog: ItemCatalog,
    /// Scratch: event-lane encode target.
    ev_buf: [u8; MAX_EVENT_MSG_BYTES],
    /// Autosave sweep cursor: which connection slot [`Self::autosave`] looks
    /// at next. One slot per call, so the work is O(1) per tick and every
    /// connected player is visited once every `MAX_PLAYERS` ticks (3.3 s at
    /// 30 Hz) — bounded like everything else, and the reason a shard that is
    /// killed mid-session costs seconds of a player's progress rather than
    /// the whole session.
    autosave_at: usize,
    /// The last record handed out per connection slot, so the sweep can skip
    /// a player whose state has not moved. `PlayerSave` is `Eq` because every
    /// field of it is quantized (`movement::Body`), which is what makes this
    /// comparison exact rather than a tolerance.
    last_saved: Box<[PlayerSave]>,
    /// Who is on each connection slot, for exactly as long as they are —
    /// so [`Self::disconnect`] can file the sleeper it is about to create
    /// under the identity that will come back for it.
    ///
    /// The accept loop has its own copy of this (`net.rs`'s `KeySlot`) and
    /// that is not duplication worth removing: that one lives on an async
    /// task and answers "whose record is this save message", this one lives
    /// on the sim thread and answers "whose body did this leave just put to
    /// sleep". Sharing one across the two threads would be a lock or a
    /// race, and the fact is two bytes wide.
    keys: Box<[Option<PlayerKey>]>,
    /// key → the world id of the body they left behind.
    ///
    /// **This is the whole of the identity problem sleepers create.** A
    /// player id is minted per connection, so the sim cannot recognise a
    /// returning player and the store's opaque key never enters the sim
    /// (`persist.rs` is explicit that it must not). Something outside the
    /// world has to hold the one arrow between them, and it is this.
    ///
    /// Never in the *player* save file: an id means nothing after a
    /// restart, so persisting one there would be persisting a dangling
    /// pointer. The world file is the one place the pairing does survive —
    /// a save writes ids and keys in the same breath ([`Self::identities`],
    /// `worldfile.rs`), and a boot that loads one rebuilds this index from
    /// it ([`Self::adopt_identities`]) so the bodies it restored are
    /// claimable.
    sleepers: SleeperIndex,
    /// The wallets this shard trusts with the admin lane (admin v0). Pure
    /// config, read once at boot and never written — the same standing as
    /// the baked content tables, and the reason it may live on a struct
    /// whose header says it holds no `ShardStats`: a list of addresses is
    /// data, not a side effect.
    admins: crate::admin::Admins,
}

/// The three side channels an admin verb needs and the sim's own state
/// cannot provide — passed into [`ShardCore::tick`] rather than held,
/// because every one of them belongs to a thread that is not this one.
///
/// A bundle rather than three parameters for `charge::tick_fuses`' reason
/// inverted: these are not distinct owners of the world, they are one
/// answer to "what does this tick owe the outside".
pub struct Ops<'a> {
    /// The anomaly log's producer. Every admin act and every `/bug` lands
    /// here; so do the counter deltas, on the sweep's cadence.
    pub log: &'a mut crate::anomaly::Sink,
    /// Kicks and bans, bound for the accept loop that owns the sockets.
    ///
    /// `None` ⇒ **nowhere to send one**, which is a real state and not a
    /// test stub: a shard driven without an accept loop has no socket to
    /// close. It takes the same path a full ring takes — the act is
    /// refused, counted and logged — so the two cannot diverge.
    pub admin_tx: Option<&'a mut rtrb::Producer<crate::admin::AdminAct>>,
    /// Raised by `/save`, read and cleared by the sim thread's world-save
    /// cadence — a flag rather than a call because the blob is written by
    /// a different thread again, and this tick has no business waiting.
    pub save_now: &'a mut bool,
}

/// Which of the three doors a join came through
/// ([`ShardCore::connect_as`]). Returned rather than counted inside,
/// because the counters are the *server's* — `ShardCore` is pure and holds
/// no `ShardStats` — and because a caller that logged "restored" for a
/// takeover would be reporting persistence working when what worked was
/// the world. That exact over-report is what this type exists to prevent:
/// the accept loop used to bump `saves_restored` on `save.is_some()`, which
/// stopped being the same question the moment a sleeper could outrank a
/// record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Admitted {
    /// Took over the sleeping body they left behind.
    TookOver,
    /// Restored from the store's record — the world did not have them.
    Restored,
    /// A fresh character: a first visit, a guest, or a wiped shard.
    Fresh,
}

/// The key → sleeper-id table, fixed at `MAX_PLAYERS` because that is the
/// hard ceiling on sleepers: a sleeper holds a world player slot, and there
/// are `MAX_PLAYERS` of those.
///
/// Entries can go stale — a disconnect files the arrow *before* its `Leave`
/// lands, so a `Leave` a full queue refused leaves an arrow at a body that
/// never slept. (Eviction used to be the other source, when the world took
/// sleepers on its own authority; two-phase eviction forgets the arrow at
/// the pick, `connect_as`.) So a hit here is a *hint*, checked against
/// `World::is_sleeper` before it is acted on, and a full table is swept of
/// its dead entries before it is called full. That ordering is the reason
/// this cannot wedge: staleness is bounded by the same number as the thing
/// it tracks.
struct SleeperIndex {
    entries: Box<[Option<(PlayerKey, u32)>]>,
}

impl SleeperIndex {
    fn new() -> Self {
        Self {
            entries: vec![None; MAX_PLAYERS].into_boxed_slice(),
        }
    }

    fn find(&self, key: &PlayerKey) -> Option<u32> {
        self.entries
            .iter()
            .flatten()
            .find(|(k, _)| k == key)
            .map(|(_, id)| *id)
    }

    /// The key filed for sleeper `id`, if any — `find` reversed, for the
    /// eviction path: a victim is picked by body, and the record it leaves
    /// behind has to be filed under the identity that will come back for
    /// it. `None` ⇒ a guest's body: admitted, remembered by nobody, so
    /// there is no record to file and never was.
    fn key_of(&self, id: u32) -> Option<PlayerKey> {
        self.entries
            .iter()
            .flatten()
            .find(|(_, i)| *i == id)
            .map(|(k, _)| *k)
    }

    fn forget(&mut self, key: &PlayerKey) {
        for e in self.entries.iter_mut() {
            if e.map(|(k, _)| k == *key).unwrap_or(false) {
                *e = None;
            }
        }
    }

    /// File `id` under `key`, replacing any earlier body for the same
    /// player. `live` is asked only if the table is full, and only about
    /// entries already in it — the sweep that keeps a table of stale
    /// pointers from refusing a real sleeper.
    fn put(&mut self, key: &PlayerKey, id: u32, live: impl Fn(u32) -> bool) {
        self.forget(key);
        if self.entries.iter().all(|e| e.is_some()) {
            for e in self.entries.iter_mut() {
                if let Some((_, sleeper)) = *e {
                    if !live(sleeper) {
                        *e = None;
                    }
                }
            }
        }
        if let Some(free) = self.entries.iter_mut().find(|e| e.is_none()) {
            *free = Some((*key, id));
        }
        // Still full ⇒ every entry names a body that is genuinely asleep,
        // so there are `MAX_PLAYERS` sleepers and this player's own body is
        // one of them. Dropping the arrow costs them the takeover and not
        // the character: they come back through `JoinAs` off the store,
        // which is the same outcome an eviction gives (`SAVES.md` §9.2).
        // Unreachable while `forget` runs first — this player cannot be
        // both absent from the table and occupying all of it — and left
        // silent rather than asserted for that reason.
    }
}

impl ShardCore {
    pub fn new(seed: u64) -> Self {
        let mut clients = Vec::with_capacity(MAX_PLAYERS);
        clients.resize_with(MAX_PLAYERS, ClientNetState::new);
        Self {
            world: World::new(seed),
            clients: clients.into_boxed_slice(),
            queued: vec![Command::Leave { id: 0 }; MAX_COMMANDS_PER_TICK].into_boxed_slice(),
            queued_len: 0,
            cmd_buf: vec![Command::Leave { id: 0 }; MAX_COMMANDS_PER_TICK].into_boxed_slice(),
            baseline_buf: [EntityState::default(); MAX_SNAPSHOT_ENTITIES],
            sent_buf: [EntityState::default(); MAX_SNAPSHOT_ENTITIES],
            removed_buf: [0; MAX_SNAPSHOT_ENTITIES],
            dg_buf: [0; DATAGRAM_BUDGET_BYTES],
            catalog: ItemCatalog::EMPTY,
            ev_buf: [0; MAX_EVENT_MSG_BYTES],
            admins: crate::admin::Admins::none(),
            autosave_at: 0,
            last_saved: vec![PlayerSave::EMPTY; MAX_PLAYERS].into_boxed_slice(),
            keys: vec![None; MAX_PLAYERS].into_boxed_slice(),
            sleepers: SleeperIndex::new(),
        }
    }

    fn queue(&mut self, cmd: Command) -> bool {
        // Half the command budget is reserved for the per-tick inputs.
        if self.queued_len >= MAX_COMMANDS_PER_TICK - MAX_PLAYERS {
            return false;
        }
        self.queued[self.queued_len] = cmd;
        self.queued_len += 1;
        true
    }

    /// Install a client on `slot` with player `id`, as a **fresh** character.
    /// False ⇒ retry next tick (queue full — refuse, never grow).
    #[must_use]
    pub fn connect(&mut self, slot: usize, id: u32) -> bool {
        self.connect_as(slot, id, None, None).is_some()
    }

    /// Whether the next queued seat would meet a world with no free slot —
    /// the moment two-phase eviction has to act. Counted against the world
    /// **plus this window's own queue**: seats queued ahead will consume
    /// free slots before this one lands and queued `Evict`s will open them,
    /// so the arithmetic is about the world the join will actually meet,
    /// not the one standing now. (A `Wake` is neither: a takeover reuses
    /// the sleeper's own slot.)
    fn slots_short(&self) -> bool {
        let free = self.world.players.iter().filter(|p| !p.active).count();
        let mut seats = 0usize;
        let mut freed = 0usize;
        for cmd in self.queued[..self.queued_len].iter() {
            match cmd {
                Command::Join { .. } | Command::JoinAs { .. } => seats += 1,
                Command::Evict { .. } => freed += 1,
                _ => {}
            }
        }
        free + freed <= seats
    }

    /// Whether this window's queue has already committed the sleeping body
    /// `id`: an `Evict` is about to remove it, or a `Wake` is about to hand
    /// it back to its owner. Either way it is not available to be woken by
    /// — or evicted for — the join being admitted now, and without this
    /// check two joins in one window would nominate the same victim and
    /// the second would land on a world with no slot to give.
    fn spoken_for(&self, sleeper: u32) -> bool {
        self.queued[..self.queued_len].iter().any(|cmd| match *cmd {
            Command::Evict { id } => id == sleeper,
            Command::Wake { sleeper: s, .. } => s == sleeper,
            _ => false,
        })
    }

    /// The eviction policy — **the longest-asleep sleeper**, ties broken on
    /// slot index — moved here from `World::seat` with two-phase eviction:
    /// the same scan `seat` ran when it evicted on its own authority, minus
    /// bodies this window has already spoken for. The server owns the pick
    /// because only the server can file the victim's save before the world
    /// forgets the body; the world only obeys the id (`Command::Evict`),
    /// which keeps the choice in the command stream (wall 5).
    fn evict_victim(&self) -> Option<u32> {
        let mut pick: Option<usize> = None;
        for (i, p) in self.world.players.iter().enumerate() {
            if !p.active || !p.sleeping || self.spoken_for(p.id) {
                continue;
            }
            let older = match pick {
                None => true,
                Some(b) => p.slept_at < self.world.players[b].slept_at,
            };
            if older {
                pick = Some(i);
            }
        }
        pick.map(|i| self.world.players[i].id)
    }

    /// Players with a live connection right now — the number the status
    /// endpoint publishes as `players` (`stats.rs` mirrors it each tick).
    ///
    /// Counted off `clients[].connected` rather than derived from the
    /// `joins`/`leaves` counters, because those can legitimately disagree
    /// with occupancy: a refused `connect_as` parks the link and rides the
    /// LEAVING sweep out, which bumps `leaves` with no matching `join`, so
    /// the difference drifts one short per refusal. An O(MAX_PLAYERS) scan
    /// of a 100-element array, priced and justified where `sleepers` is
    /// (`net.rs`): a second copy of the count could drift from the array
    /// it describes.
    pub fn connected(&self) -> usize {
        self.clients.iter().filter(|c| c.connected).count()
    }

    /// Install a client: onto the body they left behind if it is still
    /// standing, restoring `save` if it is not and the store had one, and
    /// as a fresh character otherwise.
    ///
    /// **Three doors, and their order is the design.** The world outranks
    /// the store, always. A sleeper is what actually happened to that
    /// player since they left — including being killed in it — while the
    /// record is what was true when they last stopped playing. Asking the
    /// store first would hand a raided player their inventory back and
    /// quietly delete the consequence somebody else worked for
    /// (`reference/SAVES.md` §9.2: the record's job is how you return when
    /// the world has **not** got you).
    ///
    /// Whichever door opens, the world is changed by a command and only by
    /// a command — `Wake`, `JoinAs` or `Join` — so a replay of the stream
    /// reproduces the session that wrote it (wall 5, and `world.rs`'s
    /// `JoinAs` says it at more length). The takeover check is a pure read
    /// on the sim thread, which is the same posture `disconnect` already
    /// takes with `World::save_of`.
    ///
    /// **The second return value is two-phase eviction's phase one.** A
    /// seat with no free slot needs one made, and the world no longer
    /// evicts on its own authority — its record of the victim would be
    /// frozen at their leave, so a sleeper raided and then evicted came
    /// back from the stale record (`reference/SAVES.md` §9.2's one
    /// remaining hole). Instead this picks the victim ([`Self::evict_victim`]),
    /// takes a **current** save off the live body, queues `Command::Evict`
    /// *ahead of* the join, and hands the record back — `Some` ⇒ the caller
    /// must file it on the sweep's own write path, keyed, before the tick
    /// that applies the `Evict`. `None` rides most admissions: no slot
    /// pressure, a takeover (which reuses its own sleeper's slot), or a
    /// keyless victim with no record to file. The record returns rather
    /// than being pushed here because `ShardCore` holds no rings — the
    /// same seam `disconnect` and `autosave` already cross by returning.
    #[must_use]
    pub fn connect_as(
        &mut self,
        slot: usize,
        id: u32,
        key: Option<PlayerKey>,
        save: Option<PlayerSave>,
    ) -> Option<(Admitted, Option<(PlayerKey, PlayerSave)>)> {
        // A hint from the index, verified against the world before it is
        // trusted — and against this window's own queue, because a sleeper
        // a queued `Evict` has already condemned is one this join must not
        // count on waking.
        let sleeper = key
            .and_then(|k| self.sleepers.find(&k))
            .filter(|&s| self.world.is_sleeper(s))
            .filter(|&s| !self.spoken_for(s));
        let (cmd, how) = match (sleeper, save) {
            (Some(sleeper), _) => (Command::Wake { id, sleeper }, Admitted::TookOver),
            (None, Some(save)) => (Command::JoinAs { id, save }, Admitted::Restored),
            (None, None) => (Command::Join { id }, Admitted::Fresh),
        };
        // Two-phase eviction, phase one. Order is the design: save, then
        // `Evict`, then the join — all inside one window, so the tick
        // applies them back to back and the join lands on the freed slot.
        let evicted = if !matches!(how, Admitted::TookOver) && self.slots_short() {
            match self.evict_victim() {
                Some(victim) => {
                    // Room for both commands or neither: an `Evict` whose
                    // join was then refused by a full queue would delete a
                    // body and seat nobody in its place.
                    if self.queued_len + 2 > MAX_COMMANDS_PER_TICK - MAX_PLAYERS {
                        return None;
                    }
                    // The record comes off the live body NOW — the current
                    // state, raid included, not the one frozen at the
                    // victim's leave. A keyless victim is a guest: no
                    // record to file, and never was one.
                    // `zip` rather than `and_then(|k| …map(|s| (k, s)))`: same
                    // pair, and `save_of` is a pure `&self` lookup
                    // (`world.rs`) so evaluating it eagerly costs a slot
                    // lookup on the guest path and changes nothing else.
                    let record = self.sleepers.key_of(victim).zip(self.world.save_of(victim));
                    if let Some((k, _)) = record.as_ref() {
                        // The arrow points at a body the command below is
                        // about to remove.
                        self.sleepers.forget(k);
                    }
                    let roomed = self.queue(Command::Evict { id: victim });
                    debug_assert!(roomed, "room for two was checked above");
                    record
                }
                // Every slot holds an awake body (or a sleeper this window
                // already spoke for). Queue the join anyway: the world
                // refuses it silently, which is the full-shard behaviour
                // that predates sleepers, and the accept path hard-caps
                // connections ahead of this.
                None => None,
            }
        } else {
            None
        };
        if !self.queue(cmd) {
            return None;
        }
        if let Some(k) = key.as_ref() {
            // The arrow is spent. Leaving it would point at a body that is
            // now awake and owned by this connection, and the next join by
            // anyone would find `is_sleeper` false and fall through — right
            // answer, wrong reason, and one that stops being right the
            // moment ids are reused.
            self.sleepers.forget(k);
        }
        self.keys[slot] = key;
        self.clients[slot].reset(id);
        // The sweep must not read this connection's arrival as "nothing has
        // changed" against the previous tenant of the slot.
        self.last_saved[slot] = PlayerSave::EMPTY;
        Some((how, evicted))
    }

    /// Encode the whole world into `out`, returning its length.
    ///
    /// A pure read on the sim thread, and that is the design: the reference
    /// game's save is a stop-the-world walk *on its main thread* and thirteen
    /// years have not fixed the freeze (`reference/SAVES.md` §4). Ours splits
    /// the two halves — the walk is a linear pass writing integers into a
    /// buffer that is already allocated, and the file I/O is somebody else's
    /// thread entirely. Neither half blocks, neither allocates, and the cost
    /// here is bounded by `WORLD_SAVE_MAX_BYTES` no matter what the world
    /// grew to.
    pub fn encode_world(&self, out: &mut [u8]) -> Option<usize> {
        self.world.save_world(out).ok()
    }

    /// Who every body in the world belongs to, into a caller-owned buffer;
    /// returns how many were written.
    ///
    /// **The half of a world save the sim may not hold.** A body carries the
    /// id it had, and an id is minted per connection — so a saved sleeper is
    /// unclaimable unless something writes down whose it is, and the thing
    /// that knows is the opaque `PlayerKey` that `persist.rs` and
    /// `worldsave.rs` both insist never enters `sim-core`.
    ///
    /// Two sources, because a save catches players in both states: the
    /// sleeper index holds everyone who left, and `keys` holds everyone still
    /// connected — who will be sleepers by the time this file is read, since
    /// a restart ends every connection.
    ///
    /// A caller buffer rather than a `Vec` for wall 2: this runs on the sim
    /// thread, and a save that allocated would allocate on whatever tick the
    /// cadence happened to land on.
    pub fn identities(&self, out: &mut [(PlayerKey, u32)]) -> usize {
        let mut n = 0;
        let mut put = |k: PlayerKey, id: u32, out: &mut [(PlayerKey, u32)]| {
            if n < out.len() && !out[..n].iter().any(|(_, have)| *have == id) {
                out[n] = (k, id);
                n += 1;
            }
        };
        for slot in 0..MAX_PLAYERS {
            if self.clients[slot].connected {
                if let Some(k) = self.keys[slot] {
                    put(k, self.clients[slot].id, out);
                }
            }
        }
        for e in self.sleepers.entries.iter().flatten() {
            if self.world.is_sleeper(e.1) {
                put(e.0, e.1, out);
            }
        }
        n
    }

    /// Seed the sleeper index from a loaded world file — **boot only**.
    ///
    /// Every body in a loaded world is asleep (`worldsave.rs`), so this is
    /// what makes them claimable: a returning player's key resolves to the
    /// body id the file recorded, `connect_as` verifies it against the world,
    /// and the takeover is the same `Command::Wake` a mid-run reconnect uses.
    /// Without it the bodies stand there unclaimable and every player is
    /// handed a store record instead — which is the world persisting and the
    /// persistence buying nothing.
    pub fn adopt_identities(&mut self, idents: &[(PlayerKey, u32)]) {
        for (key, id) in idents {
            if self.world.is_sleeper(*id) {
                let world = &self.world;
                self.sleepers.put(key, *id, |s| world.is_sleeper(s));
            }
        }
    }

    /// Tear a client down, and **hand back what this shard should remember
    /// about them**, with the id it belongs to. `None` ⇒ nothing to remember:
    /// an already-disconnected slot, or a player the world has no body for.
    ///
    /// The id rides along rather than being read back off the slot by the
    /// caller, because by the time the caller acts the slot may have been
    /// freed and re-claimed — and a record filed under the wrong player's key
    /// hands somebody else's inventory away. One return value, no window.
    ///
    /// The read happens here, before the `Leave` is queued, because it is a
    /// read: `World::save_of` cannot mutate, so the departing player's record
    /// is taken off the live body and the command that removes it is
    /// unchanged. Where the record then goes — a ring, an index, a file — is
    /// `net.rs`'s business and none of it touches the sim thread's laws.
    pub fn disconnect(&mut self, slot: usize) -> Option<(u32, PlayerSave)> {
        let id = self.clients[slot].id;
        if !self.clients[slot].connected {
            return None;
        }
        let save = self.world.save_of(id);
        // Queue overflow here would strand the world entity; the
        // reserve (MAX_PLAYERS of headroom) makes that impossible for
        // real leave rates.
        let _ = self.queue(Command::Leave { id });
        // The body is about to become a sleeper, so remember whose it is.
        // Filed here rather than when the command lands because this is the
        // only place both halves are in hand at once, and filing an arrow
        // to a body that the queue then refused is harmless — the takeover
        // check finds no sleeper and the join falls through to the record.
        if let Some(k) = self.keys[slot] {
            let world = &self.world;
            self.sleepers.put(&k, id, |s| world.is_sleeper(s));
        }
        self.keys[slot] = None;
        self.clients[slot].connected = false;
        self.last_saved[slot] = PlayerSave::EMPTY;
        save.map(|s| (id, s))
    }

    /// One step of the autosave sweep: the next connected player whose state
    /// has moved since it was last taken, or `None`.
    ///
    /// Called once per tick by the sim loop. Bounded to one slot per call —
    /// so the cost is a fixed comparison whatever the population — and
    /// skipping an unchanged player is what keeps an idle full shard from
    /// writing 30 identical records a second.
    ///
    /// A leave is the exact save and this is the approximate one: it can be
    /// up to `MAX_PLAYERS` ticks stale, and a player killed by a shard crash
    /// loses that much. The alternative — saving every player every tick —
    /// would be unbounded work in the tick for a guarantee no genre in this
    /// tradition offers.
    pub fn autosave(&mut self) -> Option<(u32, PlayerSave)> {
        let slot = self.autosave_at;
        self.autosave_at = (slot + 1) % MAX_PLAYERS;
        if !self.clients[slot].connected {
            return None;
        }
        let id = self.clients[slot].id;
        let save = self.world.save_of(id)?;
        if save == self.last_saved[slot] {
            return None;
        }
        self.last_saved[slot] = save;
        Some((id, save))
    }

    /// One decoded input datagram from this client: acks first (they ride
    /// every datagram), then the frame tail into the seq buffer, each frame
    /// stamped with the ack this datagram carried.
    ///
    /// **The stamp is `None` until the client has acked a snapshot this
    /// shard actually sent**, and `newest_acked` is the test rather than
    /// `snapshot_ack != 0`. `on_acks` runs first and only credits ticks out
    /// of the server's own sent ring, so it is a server-verified fact that
    /// this connection has ever seen a world — where the ack field is a
    /// client claim, and before the first snapshot lands
    /// `ClientView::ack_fields` returns a flat `(0, 0)` that would measure
    /// the shard's entire uptime as one player's lag. Ordering matters and
    /// is the point of the two lines being adjacent: acking first means the
    /// very first datagram carrying a real ack is measured, not the second.
    pub fn push_input(&mut self, slot: usize, dg: &InputDatagram) {
        let c = &mut self.clients[slot];
        if !c.connected {
            return;
        }
        c.on_acks(dg.snapshot_ack, dg.ack_bits);
        let view = c.newest_acked.map(|_| dg.snapshot_ack);
        for f in dg.frames() {
            c.push_frame(*f, view);
        }
    }

    /// Whether this client can accept another C→S action this tick — the
    /// net thread pops its action ring only through an open hand, so a
    /// deferred action stays ringed (and, past the ring, in the stream).
    pub fn wants_action(&self, slot: usize) -> bool {
        self.clients[slot].connected && self.clients[slot].pending_action.is_none()
    }

    /// Hand one decoded action to this client's pending slot. Callers
    /// check `wants_action` first; a push into a full hand is dropped
    /// (defensive — the contract keeps it unreachable).
    pub fn push_action(&mut self, slot: usize, act: ActionMsg) {
        let c = &mut self.clients[slot];
        if c.connected && c.pending_action.is_none() {
            c.pending_action = Some(act);
        }
    }

    /// Hand one decoded chat line to this client's pending slot. The line
    /// is said next tick or not at all — a second line arriving in the
    /// same tick replaces the first, which cannot happen through the net
    /// path (it pops one per tick) and is the right answer if it ever
    /// does: chat is not owed delivery the way an action is.
    pub fn push_chat(&mut self, slot: usize, chat: ChatMsg) {
        let c = &mut self.clients[slot];
        if c.connected {
            c.pending_chat = Some(chat);
        }
    }

    /// One fixed tick: queued joins/leaves + one consumed input per client
    /// → `World::tick`, then interest/priority accrual, then the event
    /// lane (sim events routed + per-client sync/catalog/inventory drips),
    /// then — on the 15 Hz cadence — one encoded snapshot per connected
    /// client. All bytes go to `send(lane, slot, bytes)`; its bool is the
    /// ring's verdict and only the event lane acts on it.
    /// [`Self::tick`] with no side channels — no log, nowhere to send a
    /// kick, and `/save` answered by nobody.
    ///
    /// **Not a second tick and not a stub**: it builds the same [`Ops`]
    /// the shard builds, with every channel in its absent state, and every
    /// absent state is one a real shard can be in (an unconfigured log, an
    /// accept loop that has stopped reading). So a verb exercised here
    /// takes exactly the path it takes in production when the outside
    /// world is not listening. It costs no allocation, which is why the
    /// suites that drive thousands of ticks use it.
    pub fn tick_bare(&mut self, stats: &ShardStats, send: impl FnMut(Lane, usize, &[u8]) -> bool) {
        let mut log = crate::anomaly::Sink::off();
        let mut save_now = false;
        let mut ops = Ops {
            log: &mut log,
            admin_tx: None,
            save_now: &mut save_now,
        };
        self.tick(stats, &mut ops, send);
    }

    pub fn tick(
        &mut self,
        stats: &ShardStats,
        ops: &mut Ops<'_>,
        mut send: impl FnMut(Lane, usize, &[u8]) -> bool,
    ) {
        let mut n = self.queued_len;
        self.cmd_buf[..n].copy_from_slice(&self.queued[..n]);
        self.queued_len = 0;
        // The tick this loop's frames are about to be executed at — `T` in
        // the aim-staleness measurement (`stats::record_aim_stale`), read
        // before the loop because `World::tick` and `clients` are two
        // fields of the same `self` and the loop borrows one of them. Low
        // 16 bits, because `snapshot_ack` is: the subtraction is wrapping
        // and belongs to that method, not here.
        let now = self.world.tick as u16;
        for slot in 0..MAX_PLAYERS {
            let c = &mut self.clients[slot];
            if !c.connected {
                continue;
            }
            if let Some((frame, view)) = c.consume_input() {
                // Measured on the frame the throttle CHOSE to execute (the
                // newer of two, when it consumed two), and measured before
                // the command-buffer check on purpose: dropping the sample
                // exactly on the ticks that ran out of command room would
                // bias the distribution toward the quiet ticks, which is
                // the reverse of what a lag measurement is for.
                stats.record_aim_stale(now, view);
                if n < MAX_COMMANDS_PER_TICK {
                    self.cmd_buf[n] = Command::Input { id: c.id, frame };
                    n += 1;
                }
            }
        }
        // Pending actions ride after inputs, at most one per client per
        // tick. A full command buffer defers the action to the next tick
        // (limits.rs policy) — it stays in the client's hand.
        for slot in 0..MAX_PLAYERS {
            let c = &mut self.clients[slot];
            if !c.connected || n == MAX_COMMANDS_PER_TICK {
                continue;
            }
            if let Some(act) = c.pending_action.take() {
                self.cmd_buf[n] = match act {
                    // The action that is **usually** not a command, and the
                    // split is the whole of what world containers v0 added
                    // here. For a bag and a box it changes what this
                    // connection is *shown*, not what the world *is*: the
                    // contents already exist, so no `Command` carries it,
                    // the sim never hears it, the WAL never records it, and
                    // `World::state_hash` is identical either way. The
                    // `continue` is the whole statement — the arm's type is
                    // `!`, so no command is written and none is counted.
                    //
                    // `CONT_WORLD` is the exception, and it is an exception
                    // about **state**, not about permissions: a crate's
                    // loot does not exist until somebody opens it, so the
                    // open IS the roll. That has to reach the sim, the WAL
                    // and the hash, or a replay would rebuild a shard whose
                    // crates were all still full. The subscription is set
                    // either way — the sim decides whether there is
                    // anything to subscribe *to*, and a handle naming an
                    // empty meadow mints a record for nobody, after which
                    // the next tick's drip finds no container and closes
                    // the panel (`worldcont::open`).
                    //
                    // Both halves still spend the same one-action-per-tick
                    // hand as every other action, so an open cannot be
                    // spammed to jump the queue — which is also the only
                    // rate limit on the roll.
                    ActionMsg::Container { kind, cont } => {
                        c.open_container(kind, cont);
                        if kind == sim_core::inventory::CONT_WORLD {
                            Command::OpenWorldCont { id: c.id, cont }
                        } else {
                            continue;
                        }
                    }
                    ActionMsg::Craft { recipe, count } => Command::Craft {
                        id: c.id,
                        recipe,
                        count,
                    },
                    ActionMsg::CraftCancel { index } => Command::CraftCancel { id: c.id, index },
                    ActionMsg::Place {
                        row,
                        cx,
                        cz,
                        level,
                        loc,
                        freehand,
                    } => Command::Place {
                        id: c.id,
                        row,
                        cx,
                        cz,
                        level,
                        loc,
                        freehand,
                    },
                    ActionMsg::Deploy {
                        row,
                        cx,
                        cz,
                        level,
                        loc,
                    } => Command::PlaceDeploy {
                        id: c.id,
                        row,
                        cx,
                        cz,
                        level,
                        loc,
                    },
                    ActionMsg::Feed { cx, cz, level } => Command::Feed {
                        id: c.id,
                        cx,
                        cz,
                        level,
                    },
                    ActionMsg::Use { cx, cz, level, loc } => Command::Use {
                        id: c.id,
                        cx,
                        cz,
                        level,
                        loc,
                    },
                    ActionMsg::Demolish {
                        deploy,
                        cx,
                        cz,
                        level,
                        loc,
                    } => Command::Demolish {
                        id: c.id,
                        deploy,
                        cx,
                        cz,
                        level,
                        loc,
                    },
                    ActionMsg::Access {
                        cx,
                        cz,
                        level,
                        loc,
                        op,
                        code,
                    } => Command::Access {
                        id: c.id,
                        cx,
                        cz,
                        level,
                        loc,
                        op,
                        code,
                    },
                    ActionMsg::Upgrade {
                        cx,
                        cz,
                        level,
                        loc,
                        material,
                    } => Command::Upgrade {
                        id: c.id,
                        cx,
                        cz,
                        level,
                        loc,
                        material,
                    },
                    ActionMsg::Repair {
                        deploy,
                        cx,
                        cz,
                        level,
                        loc,
                    } => Command::Repair {
                        id: c.id,
                        deploy,
                        cx,
                        cz,
                        level,
                        loc,
                    },
                    ActionMsg::Throw {
                        deploy,
                        cx,
                        cz,
                        level,
                        loc,
                    } => Command::Throw {
                        id: c.id,
                        deploy,
                        cx,
                        cz,
                        level,
                        loc,
                    },
                    ActionMsg::Loot => Command::Loot { id: c.id },
                    ActionMsg::Pickup => Command::Pickup { id: c.id },
                    ActionMsg::Consume { slot } => Command::Consume { id: c.id, slot },
                    ActionMsg::Research { slot } => Command::Research { id: c.id, slot },
                    ActionMsg::Unlock { recipe } => Command::Unlock { id: c.id, recipe },
                    ActionMsg::Drink => Command::Drink { id: c.id },
                    ActionMsg::Respawn { on_bag } => Command::Respawn { id: c.id, on_bag },
                    ActionMsg::Move {
                        cont,
                        from_kind,
                        from_slot,
                        to_kind,
                        to_slot,
                        count,
                    } => Command::Move {
                        id: c.id,
                        cont,
                        from_kind,
                        from_slot,
                        to_kind,
                        to_slot,
                        count,
                    },
                };
                n += 1;
            }
        }
        self.world.tick(&self.cmd_buf[..n]);

        for slot in 0..MAX_PLAYERS {
            if self.clients[slot].connected {
                self.update_interest(slot, stats);
            }
        }

        self.pump_chat(stats, ops, &mut send);
        self.pump_events(stats, &mut send);

        if self.world.tick.is_multiple_of(SNAPSHOT_INTERVAL_TICKS) {
            for slot in 0..MAX_PLAYERS {
                if !self.clients[slot].connected {
                    continue;
                }
                if let Some(len) = self.encode_snapshot(slot, stats) {
                    ShardStats::bump(&stats.snap_sent);
                    send(Lane::Snapshot, slot, &self.dg_buf[..len]);
                }
            }
        }
    }

    /// This client's live world slot, or None while its join command is
    /// still queued. `update_interest` refreshes the cache every tick, so
    /// this is a validated read of it, never a scan — chat routing is
    /// O(recipients), not O(recipients × players).
    fn live_wslot(&self, slot: usize) -> Option<usize> {
        let c = &self.clients[slot];
        let w = c.own_wslot;
        if w == usize::MAX {
            return None;
        }
        let p = &self.world.players[w];
        (p.active && p.id == c.id).then_some(w)
    }

    /// Install the admin allowlist. Boot-only, beside the content tables,
    /// for the same reason: it is construction input, and a list that
    /// could change mid-run is a privilege that could change mid-run.
    pub fn install_admins(&mut self, admins: crate::admin::Admins) {
        self.admins = admins;
    }

    /// Run one slash line on behalf of `from_slot` (admin v0).
    ///
    /// **Permission is the wallet's, never the client's claim.** The key
    /// compared here is the one SIWE proved at the handshake (`auth.rs`),
    /// which is why there is no forgeable path into this function: a
    /// client that types `/kick` without being on the list gets a private
    /// refusal and a line in the anomaly log.
    ///
    /// Every outcome is logged — the act, and the refusal too. A refused
    /// admin attempt is exactly the thing an operator wants to read
    /// afterwards, and it is the half a design that only logged successes
    /// would lose.
    fn run_command(
        &mut self,
        from_slot: usize,
        text: &protocol::ChatText,
        stats: &ShardStats,
        ops: &mut Ops<'_>,
        send: &mut impl FnMut(Lane, usize, &[u8]) -> bool,
    ) {
        use crate::admin::{self, AdminAct};
        use crate::anomaly::{Kind, Record};
        use protocol::admin::AdminCmd;

        let tick = self.world.tick;
        let who = self.clients[from_slot].id;
        let Some(cmd) = protocol::admin::parse(text) else {
            // Shaped like a command, not one we know. Logged with the
            // verb code that means "unknown" so a typo is visible, and
            // the line is swallowed rather than relayed.
            ops.log.push(Record::new(
                tick,
                Kind::AdminRefused,
                admin::VERB_UNKNOWN,
                who,
            ));
            return;
        };

        // `/bug` is everybody's (`ALPHA.md` §4). The server stamps the
        // tick and the position so the note is all a player has to type —
        // which is the whole reason it is a server verb and not a client
        // one: a client-side bug report cannot prove where it was.
        if let AdminCmd::Bug { note } = cmd {
            let (qx, qy, qz) = match self.live_wslot(from_slot) {
                Some(w) => {
                    let b = self.world.players[w].body;
                    (b.qx as i64, b.qy as i64, b.qz as i64)
                }
                // No body — a bug filed from the death screen or the
                // one-tick window before the join lands. Still worth
                // keeping; the zeros say "nowhere" rather than "origin",
                // and the tick is what a replay needs anyway.
                None => (0, 0, 0),
            };
            ops.log.push(
                Record::new(tick, Kind::Bug, 0, who)
                    .with(qx, qy, qz)
                    .note(note.as_bytes()),
            );
            return;
        }

        // Everything else needs the allowlist.
        let allowed = self.keys[from_slot]
            .as_ref()
            .is_some_and(|k| self.admins.allows(k));
        let verb = admin::verb_of(&cmd);
        if !allowed {
            ops.log
                .push(Record::new(tick, Kind::AdminRefused, verb, who));
            return;
        }

        // The id an admin types is a *player* id; resolve it once, here,
        // so every verb below is working on a live slot or refusing.
        let target_slot = |me: &Self, id: u32| -> Option<usize> {
            (0..MAX_PLAYERS).find(|&s| me.clients[s].connected && me.clients[s].id == id)
        };

        let mut logged = Record::new(tick, Kind::AdminAct, verb, who);
        match cmd {
            AdminCmd::Kick { id } | AdminCmd::Ban { id } => {
                let ban = matches!(cmd, AdminCmd::Ban { .. });
                let Some(slot) = target_slot(self, id) else {
                    ops.log.push(
                        Record::new(tick, Kind::AdminRefused, verb, who).with(id as i64, 0, 0),
                    );
                    return;
                };
                let act = if ban {
                    // The wallet, not the id: an id is meaningless after a
                    // reconnect, which is the whole point of a ban.
                    let Some(key) = self.keys[slot] else {
                        ops.log.push(
                            Record::new(tick, Kind::AdminRefused, verb, who).with(id as i64, 0, 0),
                        );
                        return;
                    };
                    AdminAct::Ban { id, key }
                } else {
                    AdminAct::Kick { id }
                };
                let sent = ops.admin_tx.as_mut().is_some_and(|tx| tx.push(act).is_ok());
                if !sent {
                    // The ring to the accept loop is full — the act did
                    // NOT happen, and saying so is the difference between
                    // a bounded queue and a lie.
                    ops.log.push(
                        Record::new(tick, Kind::AdminRefused, verb, who).with(id as i64, 0, 0),
                    );
                    return;
                }
                logged = logged.with(id as i64, 0, 0);
            }
            AdminCmd::Say { text } => {
                // The house's line, marked and sent from `from = 0` — an
                // id no player can hold (ids start at 256), so no client
                // has to learn a new message shape to render it.
                let line = admin::server_line(&text);
                let len = match protocol::encode_event_chat(0, true, &line, &mut self.ev_buf) {
                    Ok(len) => len,
                    Err(_) => {
                        ShardStats::bump(&stats.encode_range_errors);
                        return;
                    }
                };
                for to_slot in 0..MAX_PLAYERS {
                    if !self.clients[to_slot].connected {
                        continue;
                    }
                    if send(Lane::Event, to_slot, &self.ev_buf[..len]) {
                        ShardStats::bump(&stats.ev_sent);
                    } else {
                        // `pump_chat`'s policy: a lost line is counted and
                        // not resynced, because no walk would bring it back.
                        ShardStats::bump(&stats.chat_undelivered);
                    }
                }
                logged = logged.note(text.as_bytes());
            }
            AdminCmd::Teleport { id } => {
                let Some(slot) = target_slot(self, id) else {
                    ops.log.push(
                        Record::new(tick, Kind::AdminRefused, verb, who).with(id as i64, 0, 0),
                    );
                    return;
                };
                // Queued as a command, so it lands next tick and lands in
                // the WAL — `Command::AdminTeleport`'s own doc has the
                // argument. `queue` refusing (a full buffer) is a dropped
                // act and is logged as a refusal rather than assumed.
                let to = self.clients[slot].id;
                if !self.queue(Command::AdminTeleport { id: who, to }) {
                    ops.log.push(
                        Record::new(tick, Kind::AdminRefused, verb, who).with(id as i64, 0, 0),
                    );
                    return;
                }
                logged = logged.with(id as i64, 0, 0);
            }
            AdminCmd::Give { item, count } => {
                if !self.queue(Command::AdminGive {
                    id: who,
                    item,
                    count,
                }) {
                    ops.log
                        .push(Record::new(tick, Kind::AdminRefused, verb, who).with(
                            item as i64,
                            count as i64,
                            0,
                        ));
                    return;
                }
                logged = logged.with(item as i64, count as i64, 0);
            }
            AdminCmd::SaveNow => {
                *ops.save_now = true;
            }
            // Handled above, before the allowlist.
            AdminCmd::Bug { .. } => return,
        }
        ops.log.push(logged);
    }

    /// Chat's whole fan-out (ALPHA.md §1: "global text + 20 m local").
    /// Runs after the sim step and before the event pump, on the
    /// positions this tick just produced.
    ///
    /// Chat never entered `World` — it is not sim state, not a `Command`,
    /// not in the WAL — so this is the only place a line exists on the
    /// server, and it exists for exactly one tick. `global` reaches every
    /// connected client; local reaches everyone within `CHAT_LOCAL_CM`
    /// planar of the speaker, the speaker included: the echo is the
    /// delivery receipt, so a client never renders its own line on faith.
    fn pump_chat(
        &mut self,
        stats: &ShardStats,
        ops: &mut Ops<'_>,
        send: &mut impl FnMut(Lane, usize, &[u8]) -> bool,
    ) {
        for from_slot in 0..MAX_PLAYERS {
            let Some(msg) = self.clients[from_slot].pending_chat.take() else {
                continue;
            };
            // A slash line is addressed to the server, not to the room —
            // intercepted before the fan-out so a mistyped `/kick` never
            // announces to everybody what you were trying to do (admin
            // v0; `protocol::admin`'s header has the transport argument).
            if protocol::admin::is_command(&msg.text) {
                self.run_command(from_slot, &msg.text, stats, ops, send);
                continue;
            }
            if !self.clients[from_slot].connected {
                // The speaker left between the line being ringed and this
                // tick. Counted like every other undelivered line — a
                // silent drop here would be the one that never shows up
                // in the numbers.
                ShardStats::bump(&stats.chat_undelivered);
                continue;
            }
            let from_id = self.clients[from_slot].id;
            // No position ⇒ nothing to measure a radius from. A line
            // typed inside the one-tick window between the welcome and
            // the join command landing is dropped, not guessed at.
            let Some(from_w) = self.live_wslot(from_slot) else {
                ShardStats::bump(&stats.chat_undelivered);
                continue;
            };
            let own = self.world.players[from_w].body;
            let len = match encode_event_chat(from_id, msg.global, &msg.text, &mut self.ev_buf) {
                Ok(len) => len,
                Err(_) => {
                    ShardStats::bump(&stats.encode_range_errors);
                    continue;
                }
            };
            for to_slot in 0..MAX_PLAYERS {
                if !self.clients[to_slot].connected {
                    continue;
                }
                if !msg.global {
                    let Some(to_w) = self.live_wslot(to_slot) else {
                        continue;
                    };
                    let p = self.world.players[to_w].body;
                    let dx = (p.qx - own.qx) as i64 * 3;
                    let dz = (p.qz - own.qz) as i64 * 3;
                    if dx * dx + dz * dz > CHAT_LOCAL_CM * CHAT_LOCAL_CM {
                        continue;
                    }
                }
                if send(Lane::Event, to_slot, &self.ev_buf[..len]) {
                    ShardStats::bump(&stats.ev_sent);
                } else {
                    // Deliberately no `ev_resync` here, unlike every other
                    // event: a resync restarts the harvested/piece/deploy
                    // walks, and none of them would bring this line back.
                    // The line is gone; say so in a counter and move on.
                    ShardStats::bump(&stats.chat_undelivered);
                }
            }
        }
    }

    /// The event lane, one tick's worth: this tick's sim events routed to
    /// their audiences, then per client at most one catalog batch, one
    /// harvested-set sync batch, and one inventory diff. A refused push
    /// (or a dropped sim event) flags the affected clients for
    /// `ev_resync` — the walk restarts; nothing is silently lost.
    fn pump_events(
        &mut self,
        stats: &ShardStats,
        send: &mut impl FnMut(Lane, usize, &[u8]) -> bool,
    ) {
        for i in 0..self.world.events.len() {
            let ev = self.world.events.entries()[i];
            match ev.code {
                EV_GATHER => {
                    let Some(slot) = self.client_slot_of(ev.a) else {
                        continue; // gatherer left this tick
                    };
                    let item = (ev.b >> 16) as u16;
                    let added = ev.b as u16;
                    match encode_event_gather(item, added, &mut self.ev_buf) {
                        Ok(len) => {
                            if send(Lane::Event, slot, &self.ev_buf[..len]) {
                                ShardStats::bump(&stats.ev_sent);
                            } else {
                                // A lost toast is cosmetic, but the resync
                                // costs nothing when nothing else was lost.
                                self.clients[slot].ev_resync();
                                ShardStats::bump(&stats.ev_resyncs);
                            }
                        }
                        Err(_) => ShardStats::bump(&stats.encode_range_errors),
                    }
                }
                EV_GATHER_REFUSED => {
                    let Some(slot) = self.client_slot_of(ev.a) else {
                        continue; // swinger left this tick
                    };
                    // b = held item << 16 | reason (world.rs's role line).
                    let item = (ev.b >> 16) as u16;
                    let reason = ev.b as u8;
                    match encode_event_gather_refused(item, reason, &mut self.ev_buf) {
                        Ok(len) => {
                            if send(Lane::Event, slot, &self.ev_buf[..len]) {
                                ShardStats::bump(&stats.ev_sent);
                            } else {
                                // A lost refusal toast is cosmetic; the
                                // resync is the uniform recovery.
                                self.clients[slot].ev_resync();
                                ShardStats::bump(&stats.ev_resyncs);
                            }
                        }
                        Err(_) => ShardStats::bump(&stats.encode_range_errors),
                    }
                }
                EV_WEAK_MARK => {
                    let Some(slot) = self.client_slot_of(ev.a) else {
                        continue; // swinger left this tick
                    };
                    let cx = (ev.b >> 16) as u16;
                    let cz = ev.b as u16;
                    let mark8 = ev.c as u8;
                    let weak_hit = ev.c & 0x100 != 0;
                    match encode_event_weak_mark(cx, cz, mark8, weak_hit, &mut self.ev_buf) {
                        Ok(len) => {
                            if send(Lane::Event, slot, &self.ev_buf[..len]) {
                                ShardStats::bump(&stats.ev_sent);
                            } else {
                                // A lost mark is cosmetic; the resync is
                                // the uniform recovery, same as a toast.
                                self.clients[slot].ev_resync();
                                ShardStats::bump(&stats.ev_resyncs);
                            }
                        }
                        Err(_) => ShardStats::bump(&stats.encode_range_errors),
                    }
                }
                EV_CRAFT_DONE | EV_CRAFT_REFUSED => {
                    let Some(slot) = self.client_slot_of(ev.a) else {
                        continue; // crafter left this tick
                    };
                    let enc = if ev.code == EV_CRAFT_DONE {
                        encode_event_craft_done((ev.b >> 16) as u16, ev.b as u16, &mut self.ev_buf)
                    } else {
                        encode_event_craft_refused(ev.b as u8, &mut self.ev_buf)
                    };
                    match enc {
                        Ok(len) => {
                            if send(Lane::Event, slot, &self.ev_buf[..len]) {
                                ShardStats::bump(&stats.ev_sent);
                            } else {
                                // The craft-queue shadow re-diffs after the
                                // resync; the toast itself is cosmetic.
                                self.clients[slot].ev_resync();
                                ShardStats::bump(&stats.ev_resyncs);
                            }
                        }
                        Err(_) => ShardStats::bump(&stats.encode_range_errors),
                    }
                }
                // The blueprint mask, whole, wherever it was stated —
                // a purchase or a door (`EV_KNOWN` in `world.rs` has the
                // three). Own-fact: a blueprint is personal, so only the
                // hand that holds it hears this.
                //
                // This arm used to live inside the `EV_RESEARCH` one
                // below, reading `world.players[…].known` back out at
                // encode time with an `unwrap_or(0)` if the researcher
                // had left. That is gone: the sim states the mask in the
                // event, so there is nothing to look up and no way for
                // the encoder to disagree with the tick that caused it.
                EV_KNOWN => {
                    let Some(slot) = self.client_slot_of(ev.a) else {
                        continue; // the holder left this tick
                    };
                    let mask = ev.b as u64 | (ev.c as u64) << 32;
                    match encode_event_known(mask, &mut self.ev_buf) {
                        Ok(len) => {
                            if send(Lane::Event, slot, &self.ev_buf[..len]) {
                                ShardStats::bump(&stats.ev_sent);
                            } else {
                                self.clients[slot].ev_resync();
                                ShardStats::bump(&stats.ev_resyncs);
                            }
                        }
                        Err(_) => ShardStats::bump(&stats.encode_range_errors),
                    }
                }
                // Research (research v0). Own-fact, both halves: a
                // blueprint is personal, so only the hand that pressed
                // hears anything. The mask that follows a success is
                // `EV_KNOWN`'s arm above, not this one's business.
                EV_RESEARCH | EV_RESEARCH_REFUSED => {
                    let Some(slot) = self.client_slot_of(ev.a) else {
                        continue; // the researcher left this tick
                    };
                    let enc = if ev.code == EV_RESEARCH {
                        encode_event_research(ev.b as u16, ev.c as u16, &mut self.ev_buf)
                    } else {
                        encode_event_research_refused(ev.b as u8, &mut self.ev_buf)
                    };
                    match enc {
                        Ok(len) => {
                            if send(Lane::Event, slot, &self.ev_buf[..len]) {
                                ShardStats::bump(&stats.ev_sent);
                            } else {
                                self.clients[slot].ev_resync();
                                ShardStats::bump(&stats.ev_resyncs);
                            }
                        }
                        Err(_) => ShardStats::bump(&stats.encode_range_errors),
                    }
                }
                EV_BUILD_REFUSED => {
                    let Some(slot) = self.client_slot_of(ev.a) else {
                        continue; // placer left this tick
                    };
                    match encode_event_build_refused(ev.b as u8, &mut self.ev_buf) {
                        Ok(len) => {
                            if send(Lane::Event, slot, &self.ev_buf[..len]) {
                                ShardStats::bump(&stats.ev_sent);
                            } else {
                                // A lost refusal is cosmetic; the resync is
                                // the uniform recovery.
                                self.clients[slot].ev_resync();
                                ShardStats::bump(&stats.ev_resyncs);
                            }
                        }
                        Err(_) => ShardStats::bump(&stats.encode_range_errors),
                    }
                }
                EV_PIECE_PLACED => {
                    // The address comes off the event; the RECORD comes off
                    // the store. Rebuilding it from the payload alone was
                    // fine while the payload was the whole record — the
                    // facing bit (hard/soft v0) made it not be, and a
                    // broadcast that defaulted it would disagree with the
                    // piece-sync walk about which side of a wall is soft:
                    // the exact two-lanes drift the trap list warns about,
                    // caught here because the store is the single source.
                    let addr = (
                        (ev.a >> 16) as u16,
                        ev.a as u16,
                        (ev.b >> 16) as u8,
                        (ev.b >> 8) as u8,
                    );
                    let rec = match self.world.pieces.find(addr.0, addr.1, addr.2, addr.3) {
                        Some(r) => *r,
                        // Placed and removed in one tick (a collapse can):
                        // announce what the event said; the removal event
                        // follows in this same drain.
                        None => PieceRec {
                            cx: addr.0,
                            cz: addr.1,
                            level: addr.2,
                            loc: addr.3,
                            row: ev.b as u8,
                            ..PieceRec::default()
                        },
                    };
                    match encode_event_piece_placed(&rec, &mut self.ev_buf) {
                        Ok(len) => {
                            for slot in 0..MAX_PLAYERS {
                                let c = &self.clients[slot];
                                if !c.connected {
                                    continue;
                                }
                                // Class-S interest (`interest.rs`), the same
                                // predicate against the same anchor the walk
                                // uses — which is what makes this a filter
                                // rather than a second opinion. A piece
                                // placed after a client's walk finished can
                                // only reach it here, so the two have to
                                // agree about where that client is or the
                                // walk's guarantee has a hole in it exactly
                                // the width of the disagreement.
                                //
                                // An anchor that is not yet valid passes
                                // everything: the client has no body this
                                // tick, and its pending walk will cover the
                                // store from the position it does get.
                                if c.piece_anchor_valid
                                    && !interest::piece_in_interest(
                                        c.piece_anchor_cm,
                                        addr.0,
                                        addr.1,
                                    )
                                {
                                    ShardStats::bump(&stats.piece_events_skipped);
                                    continue;
                                }
                                if send(Lane::Event, slot, &self.ev_buf[..len]) {
                                    ShardStats::bump(&stats.ev_sent);
                                } else {
                                    // The piece walk re-derives it.
                                    self.clients[slot].ev_resync();
                                    ShardStats::bump(&stats.ev_resyncs);
                                }
                            }
                        }
                        Err(_) => ShardStats::bump(&stats.encode_range_errors),
                    }
                }
                EV_DEPLOY_REFUSED => {
                    let Some(slot) = self.client_slot_of(ev.a) else {
                        continue; // requester left this tick
                    };
                    match encode_event_deploy_refused(ev.b as u8, &mut self.ev_buf) {
                        Ok(len) => {
                            if send(Lane::Event, slot, &self.ev_buf[..len]) {
                                ShardStats::bump(&stats.ev_sent);
                            } else {
                                self.clients[slot].ev_resync();
                                ShardStats::bump(&stats.ev_resyncs);
                            }
                        }
                        Err(_) => ShardStats::bump(&stats.encode_range_errors),
                    }
                }
                EV_DEPLOY_PLACED => {
                    // Owner (ev.c) stays sim-side: the wire record is
                    // address + row + open + locked (event.rs). Everything
                    // places closed; a door places locked, which is a
                    // world fact the whole shard sees (who it answers to
                    // is not).
                    let placed_door = self
                        .world
                        .deploys
                        .find(
                            (ev.a >> 16) as u16,
                            ev.a as u16,
                            (ev.b >> 16) as u8,
                            (ev.b >> 8) as u8,
                        )
                        .is_some_and(|d| d.locked);
                    let rec = DeployRec {
                        cx: (ev.a >> 16) as u16,
                        cz: ev.a as u16,
                        level: (ev.b >> 16) as u8,
                        loc: (ev.b >> 8) as u8,
                        row: ev.b as u8,
                        locked: placed_door,
                        ..DeployRec::default()
                    };
                    match encode_event_deploy_placed(&rec, &mut self.ev_buf) {
                        Ok(len) => {
                            for slot in 0..MAX_PLAYERS {
                                if !self.clients[slot].connected {
                                    continue;
                                }
                                if send(Lane::Event, slot, &self.ev_buf[..len]) {
                                    ShardStats::bump(&stats.ev_sent);
                                } else {
                                    // The deploy walk re-derives it.
                                    self.clients[slot].ev_resync();
                                    ShardStats::bump(&stats.ev_resyncs);
                                }
                            }
                        }
                        Err(_) => ShardStats::bump(&stats.encode_range_errors),
                    }
                }
                EV_VITALS | EV_CONSUMED | EV_CONSUME_REFUSED | EV_DRANK => {
                    // The survival module's four, all own-facts to the one
                    // body they are about — same audience shape as health,
                    // and absolute for the same reason: a client that
                    // misses one hears the whole truth from the next.
                    let Some(slot) = self.client_slot_of(ev.a) else {
                        continue; // that player left this tick
                    };
                    let enc = match ev.code {
                        EV_VITALS => encode_event_vitals(
                            (ev.b >> 16) as u16,
                            ev.b as u16,
                            (ev.c >> 16) as u16,
                            ev.c as u16,
                            &mut self.ev_buf,
                        ),
                        EV_CONSUMED => {
                            encode_event_consumed((ev.b >> 16) as u16, ev.b as u8, &mut self.ev_buf)
                        }
                        EV_DRANK => encode_event_drank(ev.b as u16, ev.c as u16, &mut self.ev_buf),
                        _ => {
                            // NOW.md §5b: the wire field is four bits and
                            // the reason domain is 1..=REFUSE_C_MAX — the
                            // encoder bounds zero and the width, so a
                            // forged 4..=15 would cross intact. The sim
                            // can never mean one; refuse it into the same
                            // counter the encoder's own range check uses.
                            if !(1..=REFUSE_C_MAX).contains(&ev.b) {
                                ShardStats::bump(&stats.encode_range_errors);
                                continue;
                            }
                            encode_event_consume_refused(ev.b as u8, &mut self.ev_buf)
                        }
                    };
                    match enc {
                        Ok(len) => {
                            if send(Lane::Event, slot, &self.ev_buf[..len]) {
                                ShardStats::bump(&stats.ev_sent);
                            } else {
                                self.clients[slot].ev_resync();
                                ShardStats::bump(&stats.ev_resyncs);
                            }
                        }
                        Err(_) => ShardStats::bump(&stats.encode_range_errors),
                    }
                }
                EV_HIT | EV_HEALTH => {
                    // Both are own-facts and the audience is the same
                    // shape: the hit goes to the hand that landed it, the
                    // health to the body that took it. A client that
                    // misses either is not left holding half a truth —
                    // health is absolute, so the next one repairs it, and
                    // a hitmarker is cosmetic.
                    let Some(slot) = self.client_slot_of(ev.a) else {
                        continue; // that player left this tick
                    };
                    let enc = if ev.code == EV_HIT {
                        encode_event_hit(ev.b, ev.c as u16, &mut self.ev_buf)
                    } else {
                        encode_event_health(ev.b as u16, ev.c as u16, &mut self.ev_buf)
                    };
                    match enc {
                        Ok(len) => {
                            if send(Lane::Event, slot, &self.ev_buf[..len]) {
                                ShardStats::bump(&stats.ev_sent);
                            } else {
                                self.clients[slot].ev_resync();
                                ShardStats::bump(&stats.ev_resyncs);
                            }
                        }
                        Err(_) => ShardStats::bump(&stats.encode_range_errors),
                    }
                }
                EV_DEATH => {
                    // Broadcast, like a door: a death is a world fact, and
                    // it is what a kill feed is made of. Not AOI'd — the
                    // reference frames' feed reports kills nobody saw.
                    //
                    // Cause, weapon and range are read off the victim's own
                    // record rather than carried on the event, exactly as a
                    // bag's position is read out of the backpack store: the
                    // sim's three event fields are spent, and the corpse is
                    // still in its slot because the death screen is waiting
                    // on it (world.rs `die`). A victim who is somehow gone
                    // is still worth a feed line, so the fallback is the
                    // world's own cause and no weapon — never a dropped
                    // death.
                    // **The victim's own bags go first, on the same
                    // ordered stream** (bag choice v0, wire v43). The
                    // death screen shapes itself around this list — two
                    // rows and a map with a bag on it, or one row and the
                    // beach — so it has to be in hand by the time `Death`
                    // raises the screen. The event lane is reliable and
                    // ordered, so "before" here is a guarantee and not a
                    // race.
                    //
                    // A death is the only moment this is sent, and that is
                    // the whole bound: one message per death, never a
                    // per-tick scan of `MAX_DEPLOYS` per client. What it
                    // costs is a `ready` bit that ages while a player sits
                    // on the screen — a cooldown lapses on a clock nothing
                    // announces. `own_bags`' doc states it; the fallback
                    // is the sim's own (ask for a bag that is not ready,
                    // get a beach, and be told so).
                    if let Some(slot) = self.client_slot_of(ev.a) {
                        let mut anchors = [BagAnchor::default(); BAG_CAP];
                        let n = self.world.deploys.own_bags(
                            &self.world.deploy,
                            ev.a,
                            self.world.tick,
                            &mut anchors,
                        );
                        match encode_event_bags(&anchors[..n], &mut self.ev_buf) {
                            Ok(len) => {
                                if send(Lane::Event, slot, &self.ev_buf[..len]) {
                                    ShardStats::bump(&stats.ev_sent);
                                } else {
                                    self.clients[slot].ev_resync();
                                    ShardStats::bump(&stats.ev_resyncs);
                                }
                            }
                            Err(_) => ShardStats::bump(&stats.encode_range_errors),
                        }
                    }
                    let (cause, item, range_cm) = self
                        .world
                        .players
                        .iter()
                        .find(|p| p.active && p.id == ev.a)
                        .map(|p| (p.death_cause, p.death_item, p.death_range_cm))
                        .unwrap_or((DEATH_BY_CLOCK, NO_ITEM, 0));
                    match encode_event_death(ev.a, ev.b, cause, item, range_cm, &mut self.ev_buf) {
                        Ok(len) => {
                            for slot in 0..MAX_PLAYERS {
                                if !self.clients[slot].connected {
                                    continue;
                                }
                                if send(Lane::Event, slot, &self.ev_buf[..len]) {
                                    ShardStats::bump(&stats.ev_sent);
                                } else {
                                    // Nothing re-derives a death: it is an
                                    // instant, not a state. The resync
                                    // still costs nothing and repairs
                                    // whatever else that client lost.
                                    self.clients[slot].ev_resync();
                                    ShardStats::bump(&stats.ev_resyncs);
                                }
                            }
                        }
                        Err(_) => ShardStats::bump(&stats.encode_range_errors),
                    }
                }
                EV_BAG_DROPPED => {
                    // Broadcast, like a placement: a bag on the ground is
                    // a world fact, and unlike the death that made it, it
                    // is a thing that stays. Position is read out of the
                    // store at encode — the event carries identity only,
                    // the same shape the hearth's stock ack takes.
                    let Some(bag) = self.world.backpacks.find(ev.a).map(WireBag::of) else {
                        continue; // looted or despawned inside the same tick
                    };
                    match encode_event_bag_dropped(&bag, &mut self.ev_buf) {
                        Ok(len) => {
                            for slot in 0..MAX_PLAYERS {
                                if !self.clients[slot].connected {
                                    continue;
                                }
                                if send(Lane::Event, slot, &self.ev_buf[..len]) {
                                    ShardStats::bump(&stats.ev_sent);
                                } else {
                                    // The bag walk re-derives it.
                                    self.clients[slot].ev_resync();
                                    ShardStats::bump(&stats.ev_resyncs);
                                }
                            }
                        }
                        Err(_) => ShardStats::bump(&stats.encode_range_errors),
                    }
                }
                EV_RESPAWN => {
                    // Own-fact, `EV_HEALTH`'s audience and posture: the one
                    // body that woke is the only one this concerns, and the
                    // world already learns where it stands from the next
                    // snapshot. What this closes is the death screen, so a
                    // client that missed it would sit behind an overlay
                    // over a world it can see — which is why the client
                    // also drops the screen on any own-position snapshot it
                    // cannot reconcile with a corpse (`client-core`).
                    let Some(slot) = self.client_slot_of(ev.a) else {
                        continue; // that player left this tick
                    };
                    match encode_event_respawn(ev.b != 0, &mut self.ev_buf) {
                        Ok(len) => {
                            if send(Lane::Event, slot, &self.ev_buf[..len]) {
                                ShardStats::bump(&stats.ev_sent);
                            } else {
                                self.clients[slot].ev_resync();
                                ShardStats::bump(&stats.ev_resyncs);
                            }
                        }
                        Err(_) => ShardStats::bump(&stats.encode_range_errors),
                    }
                }
                EV_MOVED | EV_MOVE_REFUSED => {
                    // Own-fact, `EV_RESPAWN`'s audience: a move inside your
                    // own inventory is nobody else's business, and a move
                    // into a bag is already visible to everyone else as the
                    // bag's own sync. What rides here is the *reconcile* —
                    // the sender predicted this drag and is waiting to be
                    // told to keep it or roll it back — so it goes to the
                    // one client that predicted it and to no one else.
                    let Some(slot) = self.client_slot_of(ev.a) else {
                        continue; // that player left this tick
                    };
                    // `inventory::addr`'s pack, unpacked. The sim and the
                    // wire agree on the order (from kind, from slot, to
                    // kind, to slot) and `test_event_roles` holds the sim
                    // half of that agreement to the sentence in `world.rs`.
                    let (kind_a, slot_a, kind_b, slot_b) = if ev.code == EV_MOVED {
                        addr_parts(ev.b)
                    } else {
                        addr_parts(ev.c)
                    };
                    let encoded = if ev.code == EV_MOVED {
                        encode_event_moved(
                            kind_a,
                            slot_a,
                            kind_b,
                            slot_b,
                            (ev.c >> 16) as u16,
                            ev.c as u16,
                            &mut self.ev_buf,
                        )
                    } else {
                        encode_event_move_refused(
                            ev.b as u8,
                            kind_a,
                            slot_a,
                            kind_b,
                            slot_b,
                            &mut self.ev_buf,
                        )
                    };
                    match encoded {
                        Ok(len) => {
                            if send(Lane::Event, slot, &self.ev_buf[..len]) {
                                ShardStats::bump(&stats.ev_sent);
                            } else {
                                self.clients[slot].ev_resync();
                                ShardStats::bump(&stats.ev_resyncs);
                            }
                        }
                        Err(_) => ShardStats::bump(&stats.encode_range_errors),
                    }
                }
                EV_BAG_REMOVED => {
                    // NOW.md §5b: the wire field is two bits and the
                    // reason domain tops out at BAG_GONE_MAX. Since the
                    // §5b decode pass the encoder bounds the DOMAIN too
                    // (`encode_event_bag_removed` refuses `why == 3`), so
                    // this pump check is the belt to that suspender — it
                    // still runs first because validation goes ahead of
                    // mutation (the item-move trap), into
                    // the same counter the encoder's own range check uses,
                    // and refuse **before** the cursor loop below moves
                    // anything (validation ahead of mutation — the
                    // item-move trap).
                    if ev.b > BAG_GONE_MAX {
                        ShardStats::bump(&stats.encode_range_errors);
                        continue;
                    }
                    // Same posture as a piece/deploy removal, including
                    // the walk restart: the store swap-removes, so a
                    // cursor inside the shrunken store is now pointing at
                    // an entry it already sent.
                    let store_len = self.world.backpacks.len();
                    match encode_event_bag_removed(ev.a, ev.b as u8, &mut self.ev_buf) {
                        Ok(len) => {
                            for slot in 0..MAX_PLAYERS {
                                if !self.clients[slot].connected {
                                    continue;
                                }
                                let c = &mut self.clients[slot];
                                if c.bag_sync_cursor > 0 && c.bag_sync_cursor <= store_len {
                                    c.bag_sync_cursor = 0;
                                    c.bag_sync_reset = true;
                                }
                                if send(Lane::Event, slot, &self.ev_buf[..len]) {
                                    ShardStats::bump(&stats.ev_sent);
                                } else {
                                    self.clients[slot].ev_resync();
                                    ShardStats::bump(&stats.ev_resyncs);
                                }
                            }
                        }
                        Err(_) => ShardStats::bump(&stats.encode_range_errors),
                    }
                }
                EV_DOOR => {
                    // A door's state is a world fact: broadcast, not
                    // AOI'd, like the placement that put it there. A
                    // client that misses one re-derives it from the
                    // deploy walk — the sync record carries the bit.
                    let (cx, cz) = ((ev.a >> 16) as u16, ev.a as u16);
                    let (level, loc) = ((ev.b >> 16) as u8, (ev.b >> 8) as u8);
                    let (open, locked) = (ev.b & 1 != 0, ev.b & 2 != 0);
                    let has_lock = ev.b & 4 != 0;
                    match encode_event_door(
                        cx,
                        cz,
                        level,
                        loc,
                        open,
                        locked,
                        has_lock,
                        &mut self.ev_buf,
                    ) {
                        Ok(len) => {
                            for slot in 0..MAX_PLAYERS {
                                if !self.clients[slot].connected {
                                    continue;
                                }
                                if send(Lane::Event, slot, &self.ev_buf[..len]) {
                                    ShardStats::bump(&stats.ev_sent);
                                } else {
                                    self.clients[slot].ev_resync();
                                    ShardStats::bump(&stats.ev_resyncs);
                                }
                            }
                        }
                        Err(_) => ShardStats::bump(&stats.encode_range_errors),
                    }
                }
                EV_KNOCK => {
                    // A knock reaches the **neighbourhood**, which is what
                    // the reference means by it (`DOORS.md` §4): the one
                    // channel a locked-out player has to the person
                    // inside. The point of the event is still that
                    // somebody *other* than the sender hears it.
                    //
                    // ⚠ **This paragraph used to argue against filtering
                    // and the argument did not survive its own radius.**
                    // It said AOI'ing would silence "a defender asleep on
                    // the far side of their own base" — but the band is
                    // `PIECE_INTEREST_CM`, 208 m, and a base spans tens of
                    // metres, so that defender is four to seven times
                    // inside it. What the unfiltered version actually did
                    // was toast *"knock knock"* on every screen on the
                    // island for every knock anywhere on it — `hud.rs`
                    // says the quiet part, that it fires "for a door
                    // across the base as readily as the one in front of
                    // you", and there is no owner check anywhere to stop
                    // it at your own base's edge.
                    //
                    // Safe to filter where `EV_DOOR` and `EV_OVEN` — the
                    // two events beside it in this arm's family — are not,
                    // and the difference is not the address, it is the
                    // residue. A knock is an instant; those two are
                    // *state*, on records the client holds shard-wide
                    // because the deploy walk is unaimed. Filtering a
                    // state change onto a record somebody keeps is how a
                    // door stays shut on one screen forever.
                    //
                    // **Still open and the operator's**: whether the OWNER
                    // should hear their own door knocked from anywhere on
                    // the island. That is a game question, not a routing
                    // one, and nothing here has an owner check to hang it
                    // on. `NOW.md` §0fan.
                    let (cx, cz) = ((ev.a >> 16) as u16, ev.a as u16);
                    let (level, loc) = ((ev.b >> 16) as u8, (ev.b >> 8) as u8);
                    let at = interest::cell_cm(cx, cz);
                    match encode_event_knock(cx, cz, level, loc, ev.c, &mut self.ev_buf) {
                        Ok(len) => {
                            for slot in 0..MAX_PLAYERS {
                                if !self.clients[slot].connected {
                                    continue;
                                }
                                if !self.point_event_visible(slot, at) {
                                    ShardStats::bump(&stats.ev_interest_skipped);
                                    continue;
                                }
                                if send(Lane::Event, slot, &self.ev_buf[..len]) {
                                    ShardStats::bump(&stats.ev_sent);
                                } else {
                                    self.clients[slot].ev_resync();
                                    ShardStats::bump(&stats.ev_resyncs);
                                }
                            }
                        }
                        Err(_) => ShardStats::bump(&stats.encode_range_errors),
                    }
                }
                EV_OVEN => {
                    // A lit fire is a world fact and is visible from
                    // outside the base it is in, so it broadcasts exactly
                    // as a door's state does — with the same consequence
                    // when a client misses one, except that no sync
                    // record carries the bit yet: the deploy walk mirrors
                    // `DeployRec`, and the burn state deliberately does
                    // not ride on it (`oven.rs`). A client that missed
                    // this hears the next toggle, or the snuff when the
                    // fuel runs out, which is at most one fuel unit away.
                    let (cx, cz) = ((ev.a >> 16) as u16, ev.a as u16);
                    let level = (ev.b >> 16) as u8;
                    let lit = ev.b & 1 != 0;
                    match encode_event_oven(cx, cz, level, lit, ev.c, &mut self.ev_buf) {
                        Ok(len) => {
                            for slot in 0..MAX_PLAYERS {
                                if !self.clients[slot].connected {
                                    continue;
                                }
                                if send(Lane::Event, slot, &self.ev_buf[..len]) {
                                    ShardStats::bump(&stats.ev_sent);
                                } else {
                                    self.clients[slot].ev_resync();
                                    ShardStats::bump(&stats.ev_resyncs);
                                }
                            }
                        }
                        Err(_) => ShardStats::bump(&stats.encode_range_errors),
                    }
                }
                EV_SHOT => {
                    // Broadcast **to the interest set**, `EV_SWING`'s
                    // posture and — after this was checked rather than
                    // assumed — its reason too. A client that misses one
                    // loses a tracer and nothing else: the arrow itself is
                    // the sim's, and the hit arrives on its own events
                    // whether the shot was drawn or not.
                    //
                    // **The obvious objection is that a projectile
                    // travels**, so unlike a swing it could matter to
                    // somebody who cannot see the hand that loosed it. It
                    // does not, for two independent reasons, and the first
                    // alone is sufficient. `render/tracer.rs` already
                    // refuses a shot whose shooter it holds no body for —
                    // *"Nothing to hang it on, so it is dropped rather
                    // than drawn from the origin"* — and that is the same
                    // set this filter reads, so nothing that was ever
                    // drawn stops being drawn. And the arithmetic agrees:
                    // the longest `range_m` in `content/weapons.toml` is
                    // 80 against an `AOI_ENTER_CM` of 176 m, so a shot
                    // from outside a client's interest cannot put a
                    // projectile within 96 m of it.
                    // `content/tests/content.rs` gates that second reason,
                    // because it is a relationship between a content
                    // number and a limit and nothing else was holding it.
                    let (yaw, pitch) = ((ev.b >> 8) as u16, ev.b as u8);
                    let (speed, drop) = ((ev.c >> 16) as u16, ev.c as u16);
                    let sh = Self::world_slot_of(&self.world, ev.a);
                    match encode_event_shot(ev.a, yaw, pitch, speed, drop, &mut self.ev_buf) {
                        Ok(len) => {
                            for slot in 0..MAX_PLAYERS {
                                if !self.clients[slot].connected {
                                    continue;
                                }
                                if !self.body_event_visible(slot, ev.a, sh) {
                                    ShardStats::bump(&stats.ev_interest_skipped);
                                    continue;
                                }
                                if send(Lane::Event, slot, &self.ev_buf[..len]) {
                                    ShardStats::bump(&stats.ev_sent);
                                } else {
                                    self.clients[slot].ev_resync();
                                    ShardStats::bump(&stats.ev_resyncs);
                                }
                            }
                        }
                        Err(_) => ShardStats::bump(&stats.encode_range_errors),
                    }
                }
                EV_SWING => {
                    // Broadcast **to the interest set**, which is the whole
                    // of the routing: an arm that moved is a fact about a
                    // body other people are drawing, so the audience is
                    // exactly the clients drawing that body.
                    // `body_event_visible` states the three pass-throughs;
                    // the filter is legitimate here — where
                    // `EV_PIECE_REMOVED`'s refuses one — because a swing is
                    // an instant and leaves no residue to be wrong about.
                    // `EV_HIT`'s arm below sends to one slot and drops field
                    // `a` at encode; copy that here and the feature is a
                    // body standing still for everybody, with every other
                    // gate green — which is why `gather_wire.rs`'s
                    // `a_swing_reaches_every_client_not_just_the_swinger`
                    // exists. (That citation named a `swing_wire.rs` that was
                    // never written, for one commit: the exact dead-citation
                    // class `CLAUDE.md` says to `ls` before writing.)
                    //
                    // ⚠ **This paragraph used to say the opposite of the
                    // code.** It claimed the swing went to everyone EXCEPT
                    // the hand that swung; the loop had no such skip and a
                    // named gate pinned the copy. The copy stays — it is one
                    // message per event, not one per client, and the client
                    // discards it by itself (`bodies::stream` skips
                    // `core.player_id`) — and the sentence is now the one
                    // the code implements.
                    //
                    // ⚠ **What this does NOT bound.** Post-filter peak
                    // fan-in per client is `AOI_RANK_EXIT`. Co-located
                    // swingers — a raid, i.e. the case where everyone
                    // swings at once — are all inside each other's
                    // interest, so the filter is a no-op there by
                    // construction. It buys the dispersed shard, which is
                    // every other minute of play.
                    //
                    // This arm is one of `BODY_BROADCAST_ARMS`, and that
                    // count is what `EVENT_RING_CAP` is sized from — it
                    // was equal to a single band until `EV_SHOT` became
                    // the second such arm at wire v54 and overflowed it.
                    let sw = Self::world_slot_of(&self.world, ev.a);
                    match encode_event_swing(ev.a, &mut self.ev_buf) {
                        Ok(len) => {
                            for slot in 0..MAX_PLAYERS {
                                if !self.clients[slot].connected {
                                    continue;
                                }
                                if !self.body_event_visible(slot, ev.a, sw) {
                                    ShardStats::bump(&stats.ev_interest_skipped);
                                    continue;
                                }
                                if send(Lane::Event, slot, &self.ev_buf[..len]) {
                                    ShardStats::bump(&stats.ev_sent);
                                } else {
                                    self.clients[slot].ev_resync();
                                    ShardStats::bump(&stats.ev_resyncs);
                                }
                            }
                        }
                        Err(_) => ShardStats::bump(&stats.encode_range_errors),
                    }
                }
                EV_IMPACT => {
                    // Broadcast, `EV_SHOT`'s posture one arm up and for a
                    // longer reason: a shot is a world fact for as long as
                    // it is in the air, and the mark it leaves is one for
                    // as long as anybody walks past it.
                    //
                    // **`c` is read back signed and that is the whole
                    // subtlety here.** The sim packs `qy as u32` — the
                    // two's-complement bit pattern of a coordinate that
                    // goes negative below datum — so `ev.c as i32` is the
                    // reinterpretation that undoes it. Reading it
                    // unsigned would put every riverbed impact 42,000 km
                    // up and the encoder would refuse it, which is the
                    // failure being loud rather than wrong; `a`'s cell is
                    // plain because the island starts at zero.
                    let surf = (ev.a >> 24) as u8;
                    let qx = (ev.a & 0x00FF_FFFF) as i32;
                    let qz = ev.b as i32;
                    let qy = ev.c as i32;
                    // Filtered on the **point**, not on a body — see
                    // `point_event_visible`. A mark is the one thing in
                    // this arm's family a client can place without holding
                    // anything, so this filter removes a decal that would
                    // otherwise have been spawned, and it is worth it: the
                    // pool is fixed and evicts, so a sub-pixel impact past
                    // the band takes a slot from a mark at the player's
                    // feet.
                    let at = interest::body_cm(qx, qz);
                    match encode_event_impact(qx, qy, qz, surf, &mut self.ev_buf) {
                        Ok(len) => {
                            for slot in 0..MAX_PLAYERS {
                                if !self.clients[slot].connected {
                                    continue;
                                }
                                if !self.point_event_visible(slot, at) {
                                    ShardStats::bump(&stats.ev_interest_skipped);
                                    continue;
                                }
                                if send(Lane::Event, slot, &self.ev_buf[..len]) {
                                    ShardStats::bump(&stats.ev_sent);
                                } else {
                                    self.clients[slot].ev_resync();
                                    ShardStats::bump(&stats.ev_resyncs);
                                }
                            }
                        }
                        Err(_) => ShardStats::bump(&stats.encode_range_errors),
                    }
                }
                EV_AUTH => {
                    // The opposite posture, one event apart: a grant is
                    // true of exactly one player, so it goes to that
                    // player and to nobody else. A broadcast here would
                    // publish a base's access list to the shard.
                    let (cx, cz) = ((ev.a >> 16) as u16, ev.a as u16);
                    let (level, loc) = ((ev.b >> 16) as u8, (ev.b >> 8) as u8);
                    let grant = ev.b as u8;
                    let Some(slot) = self.client_slot_of(ev.c) else {
                        continue;
                    };
                    match encode_event_auth(cx, cz, level, loc, grant, &mut self.ev_buf) {
                        Ok(len) => {
                            if send(Lane::Event, slot, &self.ev_buf[..len]) {
                                ShardStats::bump(&stats.ev_sent);
                            } else {
                                self.clients[slot].ev_resync();
                                ShardStats::bump(&stats.ev_resyncs);
                            }
                        }
                        Err(_) => ShardStats::bump(&stats.encode_range_errors),
                    }
                }
                EV_STRUCT_HIT => {
                    // A structure still standing after a raid swing: the
                    // address, what it took, what is left. Broadcast like
                    // a placement — the wall is a world fact, and anyone
                    // in earshot of the base should see it come apart. No
                    // sync walk moves: nothing left the store.
                    let (cx, cz) = ((ev.a >> 16) as u16, ev.a as u16);
                    let deploy = ev.b & STRUCT_DEPLOY_BIT != 0;
                    let (level, loc, row) = ((ev.b >> 16) as u8, (ev.b >> 8) as u8, ev.b as u8);
                    let (damage, left) = ((ev.c >> 16) as u16, ev.c as u16);
                    match encode_event_struct_hit(
                        deploy,
                        cx,
                        cz,
                        level,
                        loc,
                        row,
                        damage,
                        left,
                        &mut self.ev_buf,
                    ) {
                        Ok(len) => {
                            for slot in 0..MAX_PLAYERS {
                                if !self.clients[slot].connected {
                                    continue;
                                }
                                if send(Lane::Event, slot, &self.ev_buf[..len]) {
                                    ShardStats::bump(&stats.ev_sent);
                                } else {
                                    self.clients[slot].ev_resync();
                                    ShardStats::bump(&stats.ev_resyncs);
                                }
                            }
                        }
                        Err(_) => ShardStats::bump(&stats.encode_range_errors),
                    }
                }
                EV_PIECE_REPAIRED => {
                    // The mirror of the arm above, unpacked with the same
                    // shifts because the sim packs it the same way, and
                    // broadcast for the same reason: a wall coming back up
                    // is news to the person outside it, not only to the
                    // person who paid. No sync walk moves — the address
                    // held this row before and holds it after.
                    let (cx, cz) = ((ev.a >> 16) as u16, ev.a as u16);
                    let deploy = ev.b & STRUCT_DEPLOY_BIT != 0;
                    let (level, loc, row) = ((ev.b >> 16) as u8, (ev.b >> 8) as u8, ev.b as u8);
                    let (healed, hp) = ((ev.c >> 16) as u16, ev.c as u16);
                    match encode_event_piece_repaired(
                        deploy,
                        cx,
                        cz,
                        level,
                        loc,
                        row,
                        healed,
                        hp,
                        &mut self.ev_buf,
                    ) {
                        Ok(len) => {
                            for slot in 0..MAX_PLAYERS {
                                if !self.clients[slot].connected {
                                    continue;
                                }
                                if send(Lane::Event, slot, &self.ev_buf[..len]) {
                                    ShardStats::bump(&stats.ev_sent);
                                } else {
                                    self.clients[slot].ev_resync();
                                    ShardStats::bump(&stats.ev_resyncs);
                                }
                            }
                        }
                        Err(_) => ShardStats::bump(&stats.encode_range_errors),
                    }
                }
                EV_CHARGE_PLACED => {
                    // `EV_PIECE_REPAIRED`'s arm, unpacked with the same
                    // shifts because the sim packs the address the same
                    // way. Broadcast, and here that is not merely the
                    // cheaper choice — a burning fuse is the one piece of
                    // news the *defender* needs more than the actor, and
                    // unicasting it to the raider would be a raid nobody
                    // can answer. No sync walk moves: a charge is not in
                    // either store, so the address holds what it held.
                    let (cx, cz) = ((ev.a >> 16) as u16, ev.a as u16);
                    let deploy = ev.b & STRUCT_DEPLOY_BIT != 0;
                    let (level, loc, row) = ((ev.b >> 16) as u8, (ev.b >> 8) as u8, ev.b as u8);
                    let fuse = ev.c as u16;
                    match encode_event_charge_placed(
                        deploy,
                        cx,
                        cz,
                        level,
                        loc,
                        row,
                        fuse,
                        &mut self.ev_buf,
                    ) {
                        Ok(len) => {
                            for slot in 0..MAX_PLAYERS {
                                if !self.clients[slot].connected {
                                    continue;
                                }
                                if send(Lane::Event, slot, &self.ev_buf[..len]) {
                                    ShardStats::bump(&stats.ev_sent);
                                } else {
                                    self.clients[slot].ev_resync();
                                    ShardStats::bump(&stats.ev_resyncs);
                                }
                            }
                        }
                        Err(_) => ShardStats::bump(&stats.encode_range_errors),
                    }
                }
                EV_PIECE_REMOVED | EV_DEPLOY_REMOVED => {
                    let piece = ev.code == EV_PIECE_REMOVED;
                    let (cx, cz) = ((ev.a >> 16) as u16, ev.a as u16);
                    let (level, loc) = ((ev.b >> 16) as u8, (ev.b >> 8) as u8);
                    match encode_event_removed(piece, cx, cz, level, loc, &mut self.ev_buf) {
                        Ok(len) => {
                            let store_len = self.world.deploys.len();
                            for slot in 0..MAX_PLAYERS {
                                if !self.clients[slot].connected {
                                    continue;
                                }
                                // A swap-remove reshuffles the store under
                                // an in-progress **deployable** walk
                                // (cursor inside the shrunken store):
                                // restart that walk with a reset batch.
                                // Finished walks (cursor past the store)
                                // hear the broadcast and nothing else.
                                //
                                // **The piece walk is not here any more**,
                                // and that is this arm's whole news. The
                                // restart is correct and its *cost* is
                                // unbounded: a full walk is `store_len /
                                // PIECE_SYNC_BATCH` ticks, and removals
                                // arriving faster than that walk a client
                                // back to zero indefinitely — a raid clears
                                // that bar easily, and the client-side
                                // symptom, a world that never finishes
                                // arriving, does not read as a network
                                // problem (`reference/NETWORK.md` §9.2.1).
                                // The piece walk reads its store from the
                                // tail down instead, where the entry a
                                // swap-remove moves is always one already
                                // sent, so a removal costs it nothing and
                                // it clamps its own cursor where it reads
                                // it (`drip_client` carries the argument).
                                //
                                // The deployable walk still reads upward
                                // and so still restarts here: the same
                                // defect one store over, left standing
                                // deliberately rather than ported blind,
                                // because the downward walk trades the
                                // restart for a dependency on every
                                // *placement* reaching the client, and that
                                // seam is worth proving one store at a time
                                // (`stats.rs` `piece_walk_restarts`).
                                //
                                // **A removal is broadcast to everyone, and
                                // class-S interest deliberately does not
                                // filter it** (`interest.rs`). A placement
                                // a client is not told about costs it
                                // nothing — it has no record to be wrong
                                // about — but a removal it is not told
                                // about is a wall that stands in its world
                                // forever, because nothing re-derives an
                                // absence: the walk sends what IS there and
                                // has no way to say what stopped being.
                                // Until a client can be un-subscribed from
                                // a region and drop what it holds there,
                                // the asymmetry is the correct one, and it
                                // is cheap — a removal is nine bytes and a
                                // raid produces far fewer of them than the
                                // walk it used to restart.
                                let c = &mut self.clients[slot];
                                if !piece
                                    && c.deploy_sync_cursor > 0
                                    && c.deploy_sync_cursor <= store_len
                                {
                                    c.deploy_sync_cursor = 0;
                                    c.deploy_sync_reset = true;
                                    ShardStats::bump(&stats.piece_walk_restarts);
                                }
                                if send(Lane::Event, slot, &self.ev_buf[..len]) {
                                    ShardStats::bump(&stats.ev_sent);
                                } else {
                                    self.clients[slot].ev_resync();
                                    ShardStats::bump(&stats.ev_resyncs);
                                }
                            }
                        }
                        Err(_) => ShardStats::bump(&stats.encode_range_errors),
                    }
                }
                EV_STOCK => {
                    let Some(slot) = self.client_slot_of(ev.a) else {
                        continue; // feeder left this tick
                    };
                    let (cx, cz) = ((ev.b >> 16) as u16, ev.b as u16);
                    let level = ev.c as u8;
                    let Some(hr) = self
                        .world
                        .deploys
                        .hearths()
                        .iter()
                        .find(|h| h.cx == cx && h.cz == cz && h.level == level)
                    else {
                        continue; // hearth decayed in the same tick
                    };
                    let mut rows = [(0u16, 0u32); HEARTH_STOCK_ROWS];
                    let n = self.world.deploy.mat_count as usize;
                    for (m, row) in rows.iter_mut().enumerate().take(n) {
                        *row = (self.world.deploy.mats[m], hr.stock[m]);
                    }
                    match encode_event_stock(cx, cz, level, &rows[..n], &mut self.ev_buf) {
                        Ok(len) => {
                            if send(Lane::Event, slot, &self.ev_buf[..len]) {
                                ShardStats::bump(&stats.ev_sent);
                            } else {
                                // The next feed re-announces; cosmetic.
                                self.clients[slot].ev_resync();
                                ShardStats::bump(&stats.ev_resyncs);
                            }
                        }
                        Err(_) => ShardStats::bump(&stats.encode_range_errors),
                    }
                }
                EV_SLOT_HARVESTED | EV_SLOT_RESPAWNED => {
                    let cx = (ev.a >> 16) as u16;
                    let cz = ev.a as u16;
                    let harvested = ev.code == EV_SLOT_HARVESTED;
                    match encode_event_slot_change(harvested, cx, cz, &mut self.ev_buf) {
                        Ok(len) => {
                            for slot in 0..MAX_PLAYERS {
                                if !self.clients[slot].connected {
                                    continue;
                                }
                                if send(Lane::Event, slot, &self.ev_buf[..len]) {
                                    ShardStats::bump(&stats.ev_sent);
                                } else {
                                    self.clients[slot].ev_resync();
                                    ShardStats::bump(&stats.ev_resyncs);
                                }
                            }
                        }
                        Err(_) => ShardStats::bump(&stats.encode_range_errors),
                    }
                }
                _ => {}
            }
        }
        if self.world.events.dropped > 0 {
            // The ring refused events this tick; whatever they announced,
            // the sync walk re-derives (limits.rs event-ring policy).
            //
            // **Counted in two places, because the cause and the
            // consequence are different questions.** `EventQueue::dropped`
            // is reset by `clear()` on the first line of the next
            // `World::tick`, so unless it is folded in here the fact that
            // the sim outran its per-tick event budget leaves no trace at
            // all — only a shard-wide resync that reads exactly like a
            // hundred connections falling behind at once. `ev_resyncs`
            // keeps counting the total so nothing watching it changes
            // meaning; `ev_resyncs_dropped` is the share this branch owns.
            ShardStats::add(&stats.ev_sim_dropped, self.world.events.dropped as u64);
            for slot in 0..MAX_PLAYERS {
                if self.clients[slot].connected {
                    self.clients[slot].ev_resync();
                    ShardStats::bump(&stats.ev_resyncs);
                    ShardStats::bump(&stats.ev_resyncs_dropped);
                }
            }
        }

        for slot in 0..MAX_PLAYERS {
            if self.clients[slot].connected {
                self.drip_client(slot, stats, send);
            }
        }
    }

    /// Resolve which connection slot player `id` belongs to.
    fn client_slot_of(&self, id: u32) -> Option<usize> {
        (0..MAX_PLAYERS).find(|&s| self.clients[s].connected && self.clients[s].id == id)
    }

    /// One client's drip work: catalog batch, harvested-set sync batch,
    /// inventory diff — each at most one message per tick, so per-client
    /// event work is bounded regardless of world size. A refused push
    /// stops this client's drip for the tick (the ring is full; the same
    /// state re-offers next tick).
    fn drip_client(
        &mut self,
        slot: usize,
        stats: &ShardStats,
        send: &mut impl FnMut(Lane, usize, &[u8]) -> bool,
    ) {
        // Catalog: names first — toasts and hotbar labels want them early.
        let c = &self.clients[slot];
        if self.catalog.count > 0 && c.catalog_cursor < self.catalog.count as usize {
            match encode_event_catalog(&self.catalog, c.catalog_cursor, &mut self.ev_buf) {
                Ok((len, took)) => {
                    if send(Lane::Event, slot, &self.ev_buf[..len]) {
                        ShardStats::bump(&stats.ev_sent);
                        self.clients[slot].catalog_cursor += took;
                    } else {
                        return;
                    }
                }
                Err(_) => ShardStats::bump(&stats.encode_range_errors),
            }
        }

        // Recipe rows, same drip shape (the craft menu's data).
        let c = &self.clients[slot];
        let cc = &self.world.craft;
        if cc.recipe_count > 0 && c.recipes_cursor < cc.recipe_count as usize {
            match encode_event_recipes(cc, c.recipes_cursor, &mut self.ev_buf) {
                Ok((len, took)) => {
                    if send(Lane::Event, slot, &self.ev_buf[..len]) {
                        ShardStats::bump(&stats.ev_sent);
                        self.clients[slot].recipes_cursor += took;
                    } else {
                        return;
                    }
                }
                Err(_) => ShardStats::bump(&stats.encode_range_errors),
            }
        }

        // Research rows, same drip shape (the tech tree panel's data).
        let c = &self.clients[slot];
        let rc = &self.world.research;
        if rc.row_count > 0 && c.research_cursor < rc.row_count as usize {
            match encode_event_research_rows(rc, c.research_cursor, &mut self.ev_buf) {
                Ok((len, took)) => {
                    if send(Lane::Event, slot, &self.ev_buf[..len]) {
                        ShardStats::bump(&stats.ev_sent);
                        self.clients[slot].research_cursor += took;
                    } else {
                        return;
                    }
                }
                Err(_) => ShardStats::bump(&stats.encode_range_errors),
            }
        }

        // Piece-def rows, same drip shape (the build menu's data).
        let c = &self.clients[slot];
        let bc = &self.world.build;
        if bc.piece_count > 0 && c.piece_defs_cursor < bc.piece_count as usize {
            match encode_event_piece_defs(bc, c.piece_defs_cursor, &mut self.ev_buf) {
                Ok((len, took)) => {
                    if send(Lane::Event, slot, &self.ev_buf[..len]) {
                        ShardStats::bump(&stats.ev_sent);
                        self.clients[slot].piece_defs_cursor += took;
                    } else {
                        return;
                    }
                }
                Err(_) => ShardStats::bump(&stats.encode_range_errors),
            }
        }

        // Harvested-set walk (join sync / resync), drip-fed. The cursor
        // walks the live store; entries that move behind it mid-walk stay
        // unsynced until their own respawn event — bounded staleness the
        // respawn window already caps, documented over machinery.
        let c = &self.clients[slot];
        let lives = &self.world.slot_lives;
        if c.sync_reset || c.sync_cursor < lives.len() {
            let mut cells = [(0u16, 0u16); SLOT_SYNC_BATCH];
            let mut n_cells = 0usize;
            let mut scanned = 0usize;
            let entries = lives.entries();
            while c.sync_cursor + scanned < entries.len()
                && scanned < SYNC_SCAN_PER_TICK
                && n_cells < SLOT_SYNC_BATCH
            {
                let e = entries[c.sync_cursor + scanned];
                if e.respawn_at != 0 {
                    cells[n_cells] = (e.cx, e.cz);
                    n_cells += 1;
                }
                scanned += 1;
            }
            if c.sync_reset || n_cells > 0 {
                match encode_event_slot_sync(c.sync_reset, &cells[..n_cells], &mut self.ev_buf) {
                    Ok(len) => {
                        if send(Lane::Event, slot, &self.ev_buf[..len]) {
                            ShardStats::bump(&stats.ev_sent);
                            let c = &mut self.clients[slot];
                            c.sync_reset = false;
                            c.sync_cursor += scanned;
                        } else {
                            return;
                        }
                    }
                    Err(_) => ShardStats::bump(&stats.encode_range_errors),
                }
            } else {
                // Window held only standing-damage entries: nothing to say.
                self.clients[slot].sync_cursor += scanned;
            }
        }

        // Placed-piece walk (join sync / resync), drip-fed like the
        // harvested set — and read from the **tail down**, which is the
        // one thing here worth understanding.
        //
        // The store swap-removes: taking entry `i` out moves the store's
        // *last* entry into the hole. A walk reading upward cannot survive
        // that, because the entry that moved can land below the cursor,
        // where the walk will never look again — so every removal had to
        // zero the cursor and re-send the world from scratch. Correct, and
        // unbounded: a full walk is `len / PIECE_SYNC_BATCH` ticks and a
        // raid removes pieces faster than that, so a client under one
        // could be walked back to the start every tick and never converge
        // at all (`reference/NETWORK.md` §9.2.1).
        //
        // Downward, the entry a swap-remove moves is always one this walk
        // has **already sent** — it comes off the tail, and the tail is
        // where the walk starts. It can only land in the not-yet-sent
        // region, where it is sent a second time and the client's
        // address-keyed apply dedups it, exactly as it dedups a piece that
        // also arrived by broadcast. So no removal can hide an entry from
        // the walk, the cursor only ever needs clamping to the store that
        // is actually there, and the walk finishes in a bounded number of
        // ticks no matter what a raid does. `piece_walk_completes` is what
        // says it finished.
        //
        // `piece_sync_cursor` therefore counts entries **still owed** —
        // `[0, cursor)` is what the client has not been sent — and
        // `piece_sync_reset` doubles as "this walk has not started", since
        // a fresh join and `ev_resync` both leave the cursor at 0, which is
        // also what a *finished* walk holds.
        //
        // What the walk no longer re-derives is an append. A piece placed
        // after the walk started lands on the tail, above the cursor, and
        // reaches the client as the EV_PIECE_PLACED broadcast that every
        // placement pushes — a refused push there calls `ev_resync`, which
        // re-arms this walk from the new tail, and a dropped event ring
        // does the same for everyone. That is the trade the paragraph
        // above buys, and it is why the deployable walk was left reading
        // upward until its own placement seam is proven.
        //
        // **And it is aimed** (class-S interest v0, `interest.rs`). The
        // walk streams what is within `PIECE_INTEREST_CM` of the anchor it
        // was armed at and skips the rest, so a joiner pays for the base
        // it landed beside rather than for every structure on the island;
        // walking `PIECE_REARM_CM` out from under that anchor re-arms the
        // walk at the new position, which is the class-D hysteresis band
        // spent as this walk's margin. The two rates are separate on
        // purpose: `PIECE_SCAN_BATCH` entries are *looked at* per tick, of
        // which at most `PIECE_SYNC_BATCH` are *said*, and a window that
        // says nothing still advances — silence is the filter working.
        if let Some(w) = self.live_wslot(slot) {
            let body = self.world.players[w].body;
            let here = interest::body_cm(body.qx, body.qz);
            let len = self.world.pieces.len();
            let c = &mut self.clients[slot];
            if !c.piece_anchor_valid {
                c.piece_anchor_cm = here;
                c.piece_anchor_valid = true;
            } else if interest::d2_cm(c.piece_anchor_cm, here)
                > interest::PIECE_REARM_CM * interest::PIECE_REARM_CM
            {
                // Re-arm from the tail, and **without** the reset bit: the
                // client keeps every piece it has been told about and the
                // address-keyed apply dedups whatever arrives twice. A
                // re-arm mid-walk cannot livelock the way a removal
                // restart could — a full-store walk is 32 ticks and a
                // sprinter covers 5.9 m of the 32 m that triggers this
                // (`interest::PIECE_SCAN_BATCH` carries the arithmetic).
                c.piece_anchor_cm = here;
                c.piece_sync_cursor = len;
                ShardStats::bump(&stats.piece_walk_rearms);
            }
        }
        let c = &self.clients[slot];
        let pieces = self.world.pieces.entries();
        let anchor = c.piece_anchor_cm;
        let owed = if c.piece_sync_reset {
            pieces.len()
        } else {
            c.piece_sync_cursor.min(pieces.len())
        };
        // No body to aim from (the join command is still queued): hold the
        // walk rather than aim it at the origin. It is owed a reset batch
        // either way, and this is a one-tick window.
        if c.piece_anchor_valid && (c.piece_sync_reset || owed > 0) {
            let window = PIECE_SCAN_BATCH.min(owed);
            // The band is filled HERE and stored nowhere (`PieceRec::dmg`).
            // A stack copy of the batch, not an allocation — wall 2 counts
            // the tick and this is `PIECE_SYNC_BATCH` records deep.
            let mut wire = [PieceRec::default(); PIECE_SYNC_BATCH];
            let mut n = 0usize;
            let mut scanned = 0usize;
            // Scanned from the TOP of the owed region down, because the
            // cursor names a contiguous prefix: stopping early has to
            // leave `[0, owed − scanned)` owed, and only a downward scan
            // makes what was consumed adjacent to what was already sent.
            for src in pieces[owed - window..owed].iter().rev() {
                scanned += 1;
                if !interest::rec_in_interest(anchor, src) {
                    continue;
                }
                wire[n] = *src;
                wire[n].dmg = damage_band(src.hp, piece_hp_max(&self.world.build, src.row));
                n += 1;
                if n == PIECE_SYNC_BATCH {
                    break;
                }
            }
            // Turned back to store order, so a batch with nothing filtered
            // out of it is the same bytes this walk has always sent.
            wire[..n].reverse();
            let rest = owed - scanned;
            let done = |c: &mut ClientNetState| {
                c.piece_sync_reset = false;
                c.piece_sync_cursor = rest;
                // Counted where the cursor moves, not where the scan ran: a
                // refused ring push re-offers this same window next tick,
                // and a counter that bumped on the offer would report the
                // filter doing work it has not done yet.
                ShardStats::add(&stats.piece_sync_skipped, (scanned - n) as u64);
                if rest == 0 {
                    // The client now holds every piece the store had
                    // within its anchor's radius when this walk began.
                    // Counted here and nowhere else: an empty world — or
                    // one whose every piece is out of range — completes on
                    // the reset batch alone, which is the honest answer to
                    // "has this client got the world yet".
                    ShardStats::bump(&stats.piece_walk_completes);
                }
            };
            if n > 0 || c.piece_sync_reset {
                match encode_event_piece_sync(c.piece_sync_reset, &wire[..n], &mut self.ev_buf) {
                    Ok(len) => {
                        if send(Lane::Event, slot, &self.ev_buf[..len]) {
                            ShardStats::bump(&stats.ev_sent);
                            done(&mut self.clients[slot]);
                        } else {
                            return;
                        }
                    }
                    Err(_) => ShardStats::bump(&stats.encode_range_errors),
                }
            } else {
                // The whole window was out of range: no message, and the
                // walk still advances. This is the byte the filter saves.
                done(&mut self.clients[slot]);
            }
        }

        // Deploy-def rows, same drip shape (the deploy menu's data).
        let c = &self.clients[slot];
        let dc = &self.world.deploy;
        if dc.def_count > 0 && c.deploy_defs_cursor < dc.def_count as usize {
            match encode_event_deploy_defs(dc, c.deploy_defs_cursor, &mut self.ev_buf) {
                Ok((len, took)) => {
                    if send(Lane::Event, slot, &self.ev_buf[..len]) {
                        ShardStats::bump(&stats.ev_sent);
                        self.clients[slot].deploy_defs_cursor += took;
                    } else {
                        return;
                    }
                }
                Err(_) => ShardStats::bump(&stats.encode_range_errors),
            }
        }

        // Placed-deployable walk (join sync / resync), drip-fed like the
        // piece walk. A decay removal mid-walk restarts it (pump_events).
        let c = &self.clients[slot];
        let deploys = self.world.deploys.entries();
        if c.deploy_sync_reset || c.deploy_sync_cursor < deploys.len() {
            let at = c.deploy_sync_cursor.min(deploys.len());
            let n = DEPLOY_SYNC_BATCH.min(deploys.len() - at);
            // The band, filled at the boundary — `PieceRec::dmg`'s note.
            let mut wire = [DeployRec::default(); DEPLOY_SYNC_BATCH];
            for (dst, src) in wire.iter_mut().zip(&deploys[at..][..n]) {
                *dst = *src;
                dst.dmg = damage_band(src.hp, deploy_hp_max(&self.world.deploy, src.row));
            }
            let batch = &wire[..n];
            match encode_event_deploy_sync(c.deploy_sync_reset, batch, &mut self.ev_buf) {
                Ok(len) => {
                    if send(Lane::Event, slot, &self.ev_buf[..len]) {
                        ShardStats::bump(&stats.ev_sent);
                        let c = &mut self.clients[slot];
                        c.deploy_sync_reset = false;
                        c.deploy_sync_cursor = at + n;
                    } else {
                        return;
                    }
                }
                Err(_) => ShardStats::bump(&stats.encode_range_errors),
            }
        }

        // Standing-backpack walk (join sync / resync), drip-fed like the
        // deploy walk. A loot or a despawn mid-walk restarts it
        // (pump_events), for the same swap-remove reason.
        let c = &self.clients[slot];
        let n_bags = self.world.backpacks.len();
        if c.bag_sync_reset || c.bag_sync_cursor < n_bags {
            let at = c.bag_sync_cursor.min(n_bags);
            let n = BAG_SYNC_BATCH.min(n_bags - at);
            let mut batch = [WireBag::default(); BAG_SYNC_BATCH];
            for (i, b) in self.world.backpacks.entries()[at..][..n].iter().enumerate() {
                batch[i] = WireBag::of(b);
            }
            match encode_event_bag_sync(c.bag_sync_reset, &batch[..n], &mut self.ev_buf) {
                Ok(len) => {
                    if send(Lane::Event, slot, &self.ev_buf[..len]) {
                        ShardStats::bump(&stats.ev_sent);
                        let c = &mut self.clients[slot];
                        c.bag_sync_reset = false;
                        c.bag_sync_cursor = at + n;
                    } else {
                        return;
                    }
                }
                Err(_) => ShardStats::bump(&stats.encode_range_errors),
            }
        }

        // The open container's contents — unicast, and re-proved every
        // tick rather than trusted from the open.
        //
        // This is the whole security argument for the message, so it is
        // written here rather than left to the reader: an open grants
        // nothing. Every tick the server resolves the handle again and
        // spends the *same* `in_reach` the move verb will spend, on the
        // same quantized body position, against the same store — and, for
        // a box, the same `lock_passes` at the same plane address
        // (`World::move_item`'s `CONT_BOX` arm; `DOORS.md` §9.8). A forged
        // open of a box across the map resolves and fails reach, so it
        // yields the close below and not one slot. A real open of a box
        // the player then walks away from does the same. A locked box a
        // stranger opens — or one that locks while their panel is up —
        // does the same again, deliberately through the same close and
        // not a new refusal: the sim already refuses every mutation, and
        // a view that kept streaming a locked box's slots read-only would
        // be raid intelligence the lock exists to hide. The set of
        // containers a client can see is therefore exactly the set it can
        // move items in — which is the quantize-both-sides law applied to
        // containers, and the reason a refusal can never disagree with
        // what the panel was drawn from.
        let c = &self.clients[slot];
        if c.own_wslot != usize::MAX && c.open_cont_kind != CONT_SELF {
            let (kind, handle) = (c.open_cont_kind, c.open_cont_handle);
            let p = &self.world.players[c.own_wslot];
            // A corpse resolves nothing. `World::die` keeps the slot, the
            // body and the position — that is what the death screen is
            // made of — so reach and the lock both still say yes at the
            // address the player fell on, and the subscription would go on
            // paying. Nothing on the death path shuts it: `die` writes no
            // client state, and the open is not a command the sim ever
            // hears, so this resolution is the only place that can.
            //
            // It belongs *here*, in the resolution, rather than at the
            // action: refusing the open would leave a panel opened while
            // alive streaming through the death, which is the same bug
            // wearing the fix's clothes. Falling through to `None` shuts
            // both mouths with the message that is already encoded below.
            //
            // And it is the sentence above, not a new rule: the move verb
            // resolves through `World::live_slot_of`, so a corpse moves no
            // item — a corpse that could still *see* a box would be
            // exactly the see-but-cannot-move split this view exists to
            // forbid. Dying next to your own loot must not buy you a
            // camera on the raider emptying it.
            let live = if p.dead {
                None
            } else {
                match kind {
                    CONT_BAG => self
                        .world
                        .backpacks
                        .index_of_id(handle)
                        .filter(|&i| self.world.backpacks.in_reach(i, p)),
                    CONT_BOX => self
                        .world
                        .deploys
                        .box_index(handle)
                        .filter(|&i| self.world.deploys.box_in_reach(i, p))
                        .filter(|&i| {
                            // The box stands on the plane, so its lock
                            // shares `box_key`'s triple plus `LOC_PLANE` —
                            // the move path's address, byte for byte. An
                            // oven at the same shape of address carries no
                            // lock (`lockable`) and passes as bare.
                            let b = self.world.deploys.boxes()[i];
                            self.world
                                .deploys
                                .lock_passes(b.cx, b.cz, b.level, LOC_PLANE, p.id)
                        }),
                    // A world container resolves against the store, never
                    // against terrain: `worldcont::open` already paid the
                    // `terrain::scatter` that proved the cell, and paying
                    // it again here would be ~60 `noise2` evaluations per
                    // open panel per tick — the cold-scatter spike
                    // `occupy.rs` exists to refuse, in the one loop that
                    // runs for every connected client. An unopened cell
                    // resolves to nothing, which is correct: there is no
                    // container there yet.
                    CONT_WORLD => self
                        .world
                        .world_conts
                        .index_of(handle)
                        .filter(|&i| self.world.world_conts.in_reach(i, p)),
                    // **The body does not ride here any more.** It had
                    // this arm from armor v1 to 2026-08-28 and resolved
                    // to `Some(0)` — no store, no reach, no lock, the one
                    // kind for which every line of this resolution was a
                    // formality. That is precisely why it was moved to
                    // its own stream below (`ClientNetState::last_wear`):
                    // sharing the slot bought nothing and evicted the
                    // wear view whenever a box opened.
                    //
                    // `open_container` refuses `CONT_WEAR` outright, so
                    // this field cannot hold it and the arm is gone
                    // rather than left answering. Falling to `None` is
                    // the safe direction if it ever did: a close costs a
                    // panel that is being fed by the other stream anyway.
                    _ => None,
                }
            };
            match live {
                // Gone, out of reach, or behind a lock that does not know
                // this hand. Same message every way, and deliberately:
                // "the bag despawned", "you walked away" and "it locked"
                // are one fact to a panel, which is that it must shut. The
                // client is told rather than left holding a view the
                // server has stopped feeding — a stale panel is where a
                // player drags into a container that is not there and
                // reads the refusal as the game breaking.
                None => match encode_event_cont_sync(CONT_SELF, 0, true, &[], &mut self.ev_buf) {
                    Ok(len) => {
                        if send(Lane::Event, slot, &self.ev_buf[..len]) {
                            ShardStats::bump(&stats.ev_sent);
                            self.clients[slot].close_container();
                        } else {
                            return;
                        }
                    }
                    Err(_) => ShardStats::bump(&stats.encode_range_errors),
                },
                Some(i) => {
                    let width = slots_in(kind);
                    let mut now = [ItemStack::default(); INV_SLOTS];
                    // **Through `World::cont_slot`, never a store directly.**
                    // This read used to be its own two-way dispatch —
                    // `if kind == CONT_BAG { backpacks } else { deploys }`
                    // — which was correct while two ground kinds existed
                    // and became a silent defect the day a third landed:
                    // `CONT_WORLD` fell through the `else` and indexed
                    // `deploys.box_slot` with a `world_conts` index, so
                    // the pad's crate drew a deploy box's contents. It
                    // never panicked (64 world containers index safely
                    // into 1 024 deploys) and no gate named `CONT_WORLD`
                    // here, so world containers v0 shipped with its panel
                    // wired to the wrong store and every wall green. The
                    // kinds are wire `u8`s and cannot be matched
                    // exhaustively, so the defence is arithmetic having
                    // one owner: `cont_slot` is that owner, and the drip
                    // asks it rather than answering again.
                    //
                    // `own_wslot` was passed for the `CONT_SELF` arm the
                    // drip can never reach — the honest argument rather
                    // than a placeholder, so the call would stay correct
                    // if that guard ever moved. **It is load-bearing as
                    // of armor v1** and that is worth recording: the fifth
                    // kind is `CONT_WEAR`, which reads `players[slot].worn`,
                    // so this argument now selects a body on every wear
                    // drip. Passing a placeholder here would have been
                    // free for four kinds and drawn every player the wrong
                    // armor on the fifth.
                    for (s, out) in now.iter_mut().enumerate().take(width) {
                        *out = self.world.cont_slot(c.own_wslot, kind, s as u8, i);
                    }
                    // At most `width` slots can differ and `width` is at
                    // most `INV_SLOTS`, which is `CONT_SYNC_BATCH` — so the
                    // diff never overflows the message and never needs a
                    // cursor. That is why this walk has no `reset` to
                    // restart, unlike every sync above it.
                    let mut changed = [InvSlot::default(); CONT_SYNC_BATCH];
                    let mut n_changed = 0usize;
                    for (s, (now, last)) in now.iter().zip(c.last_cont.iter()).enumerate() {
                        if now != last {
                            changed[n_changed] = InvSlot {
                                slot: s as u8,
                                stack: *now,
                            };
                            n_changed += 1;
                        }
                    }
                    // An open sends even when nothing changed: "you opened
                    // an empty box" is a fact, and a panel with no message
                    // behind it is a panel that never draws.
                    if c.open_cont_reset || n_changed > 0 {
                        match encode_event_cont_sync(
                            kind,
                            handle,
                            c.open_cont_reset,
                            &changed[..n_changed],
                            &mut self.ev_buf,
                        ) {
                            Ok(len) => {
                                if send(Lane::Event, slot, &self.ev_buf[..len]) {
                                    ShardStats::bump(&stats.ev_sent);
                                    let c = &mut self.clients[slot];
                                    c.open_cont_reset = false;
                                    c.last_cont = now;
                                } else {
                                    return;
                                }
                            }
                            Err(_) => ShardStats::bump(&stats.encode_range_errors),
                        }
                    }
                }
            }
        }

        // **The body, beside the container and never instead of it.**
        //
        // `CONT_WEAR` used to ride the subscription above, so opening a
        // box evicted the wear view and the route from a looted helmet to
        // a head was: take it, close the box, open the inventory, drag
        // again (`NOW.md` §0eq item 4). It rides its own stream now, for
        // the reason `ClientNetState::last_wear` states: it is the one
        // `is_own` kind, so it has no handle to resolve, no reach to
        // re-prove and no lock to pass — the whole resolution the block
        // above spends its length on says `Some(0)` for this kind and
        // always did. What is left when that is gone is a two-slot diff.
        //
        // It is deliberately not gated on a panel being up. A view the
        // client did not ask for costs nothing while nothing changes —
        // the shadow below sends only differences — and gating it on an
        // open would put back the press, the race and the eviction in one
        // step. The quantize-both-sides law is untouched: a wear move is
        // refused on `players[slot].worn`, which is the array this drip
        // reads, so the panel and the refusal cannot disagree.
        //
        // A dead player still has a body and it is still fed. Unlike a
        // box there is nothing here that can despawn, lock or move out of
        // reach, so `None` would name no fact — and the death screen's
        // panel showing what the corpse is wearing is the truth.
        let c = &self.clients[slot];
        if c.own_wslot != usize::MAX {
            let wslot = c.own_wslot;
            let mut now = [ItemStack::default(); WEAR_SLOTS];
            // Through `World::cont_slot` for the reason spelled at length
            // above: the arithmetic that turns a kind and a slot into a
            // stack has one owner, and a second reader of `worn` here
            // would be the `CONT_WORLD` defect waiting to happen again.
            // The handle is 0 and means it — this kind resolves to the
            // body of `wslot` and to nothing else.
            for (s, out) in now.iter_mut().enumerate() {
                *out = self.world.cont_slot(wslot, CONT_WEAR, s as u8, 0);
            }
            let c = &self.clients[slot];
            let mut changed = [InvSlot::default(); CONT_SYNC_BATCH];
            let mut n_changed = 0usize;
            for (s, (now, last)) in now.iter().zip(c.last_wear.iter()).enumerate() {
                if now != last {
                    changed[n_changed] = InvSlot {
                        slot: s as u8,
                        stack: *now,
                    };
                    n_changed += 1;
                }
            }
            if c.wear_reset || n_changed > 0 {
                match encode_event_cont_sync(
                    CONT_WEAR,
                    0,
                    c.wear_reset,
                    &changed[..n_changed],
                    &mut self.ev_buf,
                ) {
                    Ok(len) => {
                        if send(Lane::Event, slot, &self.ev_buf[..len]) {
                            ShardStats::bump(&stats.ev_sent);
                            let c = &mut self.clients[slot];
                            c.wear_reset = false;
                            c.last_wear = now;
                        } else {
                            return;
                        }
                    }
                    Err(_) => ShardStats::bump(&stats.encode_range_errors),
                }
            }
        }

        // Inventory diff against the last successfully queued copy.
        let c = &self.clients[slot];
        if c.own_wslot == usize::MAX {
            return; // join still queued; no inventory to speak of
        }
        let inv: [ItemStack; INV_SLOTS] = self.world.players[c.own_wslot].inv;
        let mut changed = [InvSlot::default(); INV_SLOTS];
        let mut n_changed = 0usize;
        for (i, (now, last)) in inv.iter().zip(c.last_inv.iter()).enumerate() {
            if now != last {
                changed[n_changed] = InvSlot {
                    slot: i as u8,
                    stack: *now,
                };
                n_changed += 1;
            }
        }
        if n_changed > 0 {
            match encode_event_inv(&changed[..n_changed], &mut self.ev_buf) {
                Ok(len) => {
                    if send(Lane::Event, slot, &self.ev_buf[..len]) {
                        ShardStats::bump(&stats.ev_sent);
                        self.clients[slot].last_inv = inv;
                    } else {
                        return;
                    }
                }
                Err(_) => ShardStats::bump(&stats.encode_range_errors),
            }
        }

        // Craft-queue diff against the last successfully queued copy. The
        // shadow includes the head timer: a completed unit that leaves the
        // queue shape identical (batch of N, next unit starts) still moves
        // `craft_done_at`, and the client re-anchors its countdown.
        let c = &self.clients[slot];
        let p = &self.world.players[c.own_wslot];
        let (jobs, done_at): ([CraftJob; CRAFT_QUEUE], u64) = (p.jobs, p.craft_done_at);
        if jobs != c.last_jobs || done_at != c.last_done_at {
            let live = jobs
                .iter()
                .position(|j| j.remaining == 0)
                .unwrap_or(CRAFT_QUEUE);
            let eta = done_at.saturating_sub(self.world.tick).min(u16::MAX as u64) as u16;
            match encode_event_craft_q(&jobs[..live], eta, &mut self.ev_buf) {
                Ok(len) => {
                    if send(Lane::Event, slot, &self.ev_buf[..len]) {
                        ShardStats::bump(&stats.ev_sent);
                        let c = &mut self.clients[slot];
                        c.last_jobs = jobs;
                        c.last_done_at = done_at;
                    }
                }
                Err(_) => ShardStats::bump(&stats.encode_range_errors),
            }
        }
    }

    /// Resolve the world slot player `id` landed in (join order is
    /// deterministic but not slot-aligned with connections).
    fn world_slot_of(world: &World, id: u32) -> Option<usize> {
        world.players.iter().position(|p| p.active && p.id == id)
    }

    /// Does connection `slot`'s class-D interest array describe this tick?
    ///
    /// `update_interest` gives up before pass 1 when the connection has no
    /// live body — a join command still queued, or a world slot whose
    /// tenant changed under it — and returns **without touching
    /// `interest`**, so the array is not "empty", it is meaningless.
    /// Anything reading it as a routing filter has to ask this first, or
    /// it reads "interested in nobody" off a client that has simply not
    /// been measured yet and mutes it. Named once and used by both the
    /// producer and the consumer, so the two cannot drift.
    fn interest_settled(&self, slot: usize) -> bool {
        let c = &self.clients[slot];
        c.own_wslot != usize::MAX
            && self.world.players[c.own_wslot].active
            && self.world.players[c.own_wslot].id == c.id
    }

    /// May connection `slot` be told that body `subject` did something?
    ///
    /// The class-D interest set read as a routing filter for a
    /// **body-addressed instant** — an event whose whole payload is "this
    /// player did a visible thing", which carries no position because the
    /// snapshot already said where the body is, and which leaves no
    /// residue if it is never delivered. `EV_SWING` is the first arm to
    /// use it; `EV_SHOT` and `EV_IMPACT` are the same shape and are the
    /// intended next two, which is why this is a method rather than an
    /// expression inlined in one arm.
    ///
    /// **Only for instants.** A state change may not be filtered this way,
    /// and `EV_PIECE_REMOVED`'s arm says why at length: an absence that
    /// nothing re-derives is a wall standing in a client's world forever.
    /// An unheard swing is an arm that did not move on a screen that was
    /// not looking at it.
    ///
    /// `subject_wslot` is passed in rather than resolved here because the
    /// caller hoists it out of the per-client loop — one `world_slot_of`
    /// scan per event, not one per connection.
    ///
    /// Three pass-throughs, each load-bearing:
    ///
    /// - **The subject's own connection.** A body is never a candidate for
    ///   itself (`update_interest` pass 1: `p.id != c.id`), so
    ///   `interest[own]` is false *by construction* and filtering on it
    ///   alone would silently drop the copy the actor gets. That copy is
    ///   one message per event rather than one per client, and
    ///   `gather_wire.rs`'s `a_swing_reaches_every_client_not_just_the_swinger`
    ///   pins it.
    /// - **A subject with no world slot.** Nothing to index; fail open.
    /// - **A recipient whose interest is unsettled** (above). This is the
    ///   one that bites, and it fails open for the same reason
    ///   `EV_PIECE_PLACED` passes everything through an invalid
    ///   `piece_anchor_valid`: a filter that guesses is worse than one
    ///   that waits a tick.
    fn body_event_visible(&self, slot: usize, subject: u32, subject_wslot: Option<usize>) -> bool {
        let c = &self.clients[slot];
        if c.id == subject {
            return true;
        }
        let Some(w) = subject_wslot else {
            return true;
        };
        !self.interest_settled(slot) || c.interest[w]
    }

    /// May connection `slot` be told about something that happened at
    /// world point `at_cm` (centimetres)?
    ///
    /// The **position-addressed** twin of `body_event_visible`, for an
    /// event that names a place rather than a body — `EV_IMPACT` today.
    /// Those cannot use class-D interest at all: an arrow's stop point is
    /// not an entity and has no world slot, so the question is a distance
    /// and the set to measure it against is the class-S anchor
    /// `EV_PIECE_PLACED` already filters on. Same predicate, same anchor,
    /// same fail-open when the anchor is not yet valid.
    ///
    /// **This one is not free the way the body filter is.** A client
    /// discards a swing or a shot it cannot hang on a body by itself
    /// (`render/tracer.rs` says so in as many words), so filtering those
    /// removes nothing that was ever drawn. A decal needs no body — it is
    /// placed at the point — so `render/decal.rs` will happily spawn one
    /// 500 m away, claim a slot from a fixed pool and **evict a mark at
    /// the player's feet for one that is sub-pixel**. That eviction is the
    /// thing this removes; the visible cost is a 0.22 m quad past 208 m,
    /// which is under a pixel at any sane field of view.
    fn point_event_visible(&self, slot: usize, at_cm: (i64, i64)) -> bool {
        let c = &self.clients[slot];
        !c.piece_anchor_valid || interest::point_in_interest(c.piece_anchor_cm, at_cm)
    }

    /// AOI v0 (DESIGN.md §5.5): **two** hysteresis bands over the same
    /// candidate field, plus the NETCODE.md §3 priority accrual for
    /// everything inside. A distance band — enter 176 m, leave 208 m — and
    /// a rank band — enter at rank < `AOI_RANK_ENTER`, leave at rank ≥
    /// `AOI_RANK_EXIT` — because a radius alone bounds the set at
    /// `MAX_PLAYERS + MAX_MOBS` and `MAX_SNAPSHOT_ENTITIES` claims to bound
    /// it at 64 (wall 4; `limits.rs` states both bands). Entities leaving
    /// the client's world — range, rank, disconnect, or slot reuse — go to
    /// the pending-removal set until an acked snapshot covers them.
    ///
    /// Three passes rather than one, and the split is forced by the rank:
    /// an entity's rank is not a property of the entity, it is its position
    /// among *all* the candidates, so nothing can be admitted until every
    /// candidate has been measured. Pass 1 measures and settles what is not
    /// a rank question (a tenant change, a death). Pass 2 turns the field
    /// into the two order statistics the bands compare against. Pass 3
    /// decides, on both bands at once, and accrues.
    fn update_interest(&mut self, slot: usize, stats: &ShardStats) {
        /// Packed candidate index: `< MAX_PLAYERS` is a world slot, above
        /// it a roster slot — `encode_snapshot`'s packing, for the reason
        /// it has one. Players and animals are ranked in **one** field
        /// because they compete for the same 64 records, and two fields
        /// with a merge is where the ordering would get lost.
        const CANDIDATES: usize = MAX_PLAYERS + MAX_MOBS;
        /// The rank key of a candidate that is not there. Sorts last, so a
        /// sparse shard's thresholds come out unbounded and every band
        /// decision falls through to distance — no special case needed.
        const ABSENT: i64 = i64::MAX;

        if !self.interest_settled(slot) {
            match Self::world_slot_of(&self.world, self.clients[slot].id) {
                Some(w) => self.clients[slot].own_wslot = w,
                None => return, // join command still queued
            }
        }
        let c = &mut self.clients[slot];
        let own = self.world.players[c.own_wslot].body;
        let mut overflow = false;

        // --- pass 1: measure, and settle what the rank has no say in.
        let mut d2_of = [ABSENT; CANDIDATES];
        // The same measurements again, compacted to the candidates that
        // actually exist, in index order — pass 2's field. Built here
        // rather than in a scan of its own because pass 1 already holds
        // every value it wants and every index it wants them at: a second
        // walk of `d2_of` to strip the padding is 164 loads and 164
        // branches to learn what this loop knew at the time.
        let mut ranked = [(ABSENT, u16::MAX); CANDIDATES];
        let mut n_ranked = 0usize;
        // Candidates inside the *exit* radius — the only ones the rank band
        // can have anything to say about, since the distance band already
        // holds everything past it out of the set. Counting these rather
        // than every live body is what keeps pass 2 off a shard whose
        // hundred players are spread over two kilometres.
        let mut n_near = 0usize;
        for (w, d2_out) in d2_of.iter_mut().enumerate().take(MAX_PLAYERS) {
            let p = &self.world.players[w];
            let live = p.active && p.id != c.id;
            if !live || c.tracked_id[w] != p.id {
                // Tenant left or changed: the id the client knew is gone.
                if c.interest[w] {
                    overflow |= !c.pending_add(c.tracked_id[w]);
                    c.interest[w] = false;
                }
                c.accum[w] = 0.0;
                c.unsent[w] = 0;
                c.tracked_id[w] = if live { p.id } else { 0 };
                if !live {
                    continue;
                }
            }
            let dx = (p.body.qx - own.qx) as i64 * 3;
            let dz = (p.body.qz - own.qz) as i64 * 3;
            *d2_out = dx * dx + dz * dz;
            ranked[n_ranked] = (*d2_out, w as u16);
            n_ranked += 1;
            n_near += usize::from(*d2_out <= AOI_EXIT_CM * AOI_EXIT_CM);
        }
        // The roster, in the same field and on the same two bands. Simpler
        // than the player pass by exactly one thing: there is no tenant
        // change to detect, because a roster slot is one animal for the
        // life of the shard (client.rs). A death is therefore the only way
        // an animal leaves, and it leaves the way a disconnect does — into
        // the pending-removal set, so the client despawns it rather than
        // holding a corpse that never moves again.
        for s in 0..MAX_MOBS {
            let m = &self.world.mobs.m[s];
            if !m.alive {
                if c.m_interest[s] {
                    overflow |= !c.pending_add(mob::mob_id(s));
                    c.m_interest[s] = false;
                }
                c.m_accum[s] = 0.0;
                c.m_unsent[s] = 0;
                continue;
            }
            let dx = (m.body.qx - own.qx) as i64 * 3;
            let dz = (m.body.qz - own.qz) as i64 * 3;
            let d2 = dx * dx + dz * dz;
            d2_of[MAX_PLAYERS + s] = d2;
            ranked[n_ranked] = (d2, (MAX_PLAYERS + s) as u16);
            n_ranked += 1;
            n_near += usize::from(d2 <= AOI_EXIT_CM * AOI_EXIT_CM);
        }

        // --- pass 2: the two order statistics the rank band compares
        // against. `(d2, index)` is a **total** order — the index is
        // unique — so "rank < N" is exactly "key ≤ the Nth smallest key",
        // at most N candidates satisfy it, and `AOI_RANK_EXIT` is
        // therefore a hard cap on the set rather than a target.
        //
        // Skipped whole when the field is smaller than the admission rank,
        // which is the ordinary case (NETCODE.md §9: typical ~15 in the
        // set): a sort here would be work done to learn that nothing is
        // crowded out.
        //
        // **Selected, not sorted, and only over the candidates that exist.**
        // Two order statistics are wanted and a full sort computes 164 of
        // them; `select_nth_unstable` is linear where a sort is n log n, and
        // the padding — every slot with no body in it — never enters the
        // field at all. Measured 2026-08-11 on the clustered worst case
        // (`bin/profile.rs`): the sort here plus its recursion was the
        // largest single item in the whole shard profile, ~23 % of every
        // instruction the server ran, ahead of the snapshot encoder.
        //
        // Identical answers, and the two reasons are worth stating because
        // this is a hot path nothing else gates:
        //
        // 1. `ABSENT` is `i64::MAX` and a real `d2` is a squared distance in
        //    centimetres over an island 2 km across, so **every** present
        //    key sorts before **every** absent one. The k-th smallest of the
        //    whole array is therefore the k-th smallest of the present
        //    prefix, for every k below the present count.
        // 2. Past that count the old sort yielded `(ABSENT, some index)`,
        //    and pass 3 only ever compares *present* keys against it — every
        //    one of which is strictly smaller whatever that index was. So
        //    `(ABSENT, u16::MAX)` decides identically, which is the same
        //    substitution the `else` arm below has always made.
        let (enter_key, exit_key) = if n_near > AOI_RANK_ENTER {
            const ABSENT_KEY: (i64, u16) = (ABSENT, u16::MAX);
            let n = n_ranked;
            let field = &mut ranked[..n];
            // Exit first: it is the higher rank, so selecting it leaves the
            // 63 smallest keys in the prefix below it and the enter rank is
            // a second selection over that shorter run rather than the
            // whole field.
            let exit_key = if n >= AOI_RANK_EXIT {
                *field.select_nth_unstable(AOI_RANK_EXIT - 1).1
            } else {
                ABSENT_KEY
            };
            let head = n.min(AOI_RANK_EXIT - 1);
            let enter_key = if n >= AOI_RANK_ENTER {
                *field[..head].select_nth_unstable(AOI_RANK_ENTER - 1).1
            } else {
                ABSENT_KEY
            };
            (enter_key, exit_key)
        } else {
            ((ABSENT, u16::MAX), (ABSENT, u16::MAX))
        };

        // --- pass 3: both bands, then the accrual. An entity is in the set
        // when it is inside *both* enter sides, and it leaves the moment it
        // is outside *either* exit side.
        for (w, &d2) in d2_of.iter().enumerate().take(MAX_PLAYERS) {
            if d2 == ABSENT {
                continue; // not a candidate; pass 1 already settled it
            }
            let key = (d2, w as u16);
            if c.interest[w] {
                if d2 > AOI_EXIT_CM * AOI_EXIT_CM || key > exit_key {
                    c.interest[w] = false;
                    c.accum[w] = 0.0;
                    c.unsent[w] = 0;
                    overflow |= !c.pending_add(self.world.players[w].id);
                }
            } else if d2 <= AOI_ENTER_CM * AOI_ENTER_CM && key <= enter_key {
                c.interest[w] = true;
                c.accum[w] = 0.0;
                c.unsent[w] = 0;
                c.pending_remove(self.world.players[w].id);
            }
            if c.interest[w] {
                let d_m = ((d2 as f32).sqrt()) * 0.01;
                c.accum[w] += PRIORITY_W_PLAYER / (1.0 + d_m / PRIORITY_HALF_SCALE_M);
            }
        }
        for s in 0..MAX_MOBS {
            let d2 = d2_of[MAX_PLAYERS + s];
            if d2 == ABSENT {
                continue;
            }
            let key = (d2, (MAX_PLAYERS + s) as u16);
            if c.m_interest[s] {
                if d2 > AOI_EXIT_CM * AOI_EXIT_CM || key > exit_key {
                    c.m_interest[s] = false;
                    c.m_accum[s] = 0.0;
                    c.m_unsent[s] = 0;
                    overflow |= !c.pending_add(mob::mob_id(s));
                }
            } else if d2 <= AOI_ENTER_CM * AOI_ENTER_CM && key <= enter_key {
                c.m_interest[s] = true;
                c.m_accum[s] = 0.0;
                c.m_unsent[s] = 0;
                c.pending_remove(mob::mob_id(s));
            }
            if c.m_interest[s] {
                let d_m = ((d2 as f32).sqrt()) * 0.01;
                c.m_accum[s] += PRIORITY_W_MOB / (1.0 + d_m / PRIORITY_HALF_SCALE_M);
            }
        }
        if overflow {
            c.force_resync();
            ShardStats::bump(&stats.forced_resyncs);
        }
    }

    fn wire_entity(p: &Player) -> EntityState {
        EntityState {
            id: p.id,
            qx: p.body.qx,
            qy: p.body.qy,
            qz: p.body.qz,
            qvy: p.body.qvy,
            grounded: p.body.grounded,
            sleeping: p.sleeping,
            dead: p.dead,
            yaw: p.frame.yaw,
            pitch: p.frame.pitch,
        }
    }

    /// One animal as the same record. Four of the ten fields have no
    /// meaning here and each is answered rather than left to a default:
    /// `pitch` is zero because nothing about a pig looks up or down;
    /// `sleeping` is false because that bit means *nobody is driving this
    /// body*, and something always is — dormancy is not the same fact and
    /// a client would draw the slumped pose for it; `dead` is false because
    /// a mob that dies is *removed* rather than left in its slot (`mob.rs`
    /// clears `alive` and the snapshot skips it), so unlike a player there
    /// is never a corpse of one on the wire to flag; `yaw` is the animal's
    /// heading, which is both where it is going and where it is facing,
    /// because a quadruped does not strafe.
    fn wire_mob(slot: usize, m: &sim_core::mob::Mob) -> EntityState {
        EntityState {
            id: mob::mob_id(slot),
            qx: m.body.qx,
            qy: m.body.qy,
            qz: m.body.qz,
            qvy: m.body.qvy,
            grounded: m.body.grounded,
            sleeping: false,
            dead: false,
            yaw: m.yaw,
            pitch: 0,
        }
    }

    /// Priority-filled snapshot for one client (DESIGN.md §5.5): removals
    /// first, then own entity, then stale-preempted and accumulator-ranked
    /// interest entities until budget or cap. Returns the encoded length,
    /// or None only on an encoder-refusal bug (counted, never panicking).
    fn encode_snapshot(&mut self, slot: usize, stats: &ShardStats) -> Option<usize> {
        let tick = self.world.tick;
        let c = &mut self.clients[slot];
        if c.own_wslot == usize::MAX {
            return None; // not in the world yet: nothing to snapshot
        }

        let (age, baseline_len) = match c.baseline(tick) {
            Some((age, snap)) => {
                let n = snap.entity_count as usize;
                self.baseline_buf[..n].copy_from_slice(snap.entities());
                (age, n)
            }
            None => (0, 0),
        };
        let header = SnapshotHeader {
            tick: tick as u32,
            baseline_age: age,
            last_executed_seq: c.last_executed,
            nudge: c.nudge,
        };
        let mut enc = match SnapshotEncoder::begin(
            &mut self.dg_buf,
            &header,
            &self.baseline_buf[..baseline_len],
        ) {
            Ok(enc) => enc,
            Err(_) => {
                ShardStats::bump(&stats.encode_range_errors);
                return None;
            }
        };

        // Removals — only against a real baseline: a zero-state snapshot
        // clears the client's class-D map by definition (Q3 semantics), so
        // removal ids would be wasted bytes there.
        let mut n_removed = 0usize;
        if age > 0 {
            for i in 0..c.pending().len() {
                let id = c.pending()[i];
                match enc.add_removed(id) {
                    Ok(()) => {
                        self.removed_buf[n_removed] = id;
                        n_removed += 1;
                    }
                    Err(_) => break, // cap/budget: the rest stay pending
                }
            }
        }

        // Candidates: own entity first (reconciliation needs it every
        // snapshot), then interest by (stale-preempt, accumulator).
        //
        // **One list, players and animals together**, ranked by the same
        // two keys. That is the whole reason `PRIORITY_W_MOB` is a weight
        // and not a second pass: a scheme that sent every player and then
        // whatever animals fit would give the pig at your feet lower
        // priority than a player at the far edge of AOI, and the
        // accumulator exists precisely so that comparison is made on
        // distance and staleness rather than on class. The index is
        // packed — `< MAX_PLAYERS` is a world slot, above it a roster slot
        // — because the alternative is two arrays and a merge, and the
        // merge is the part that would get the ordering wrong.
        const CANDIDATES: usize = MAX_PLAYERS + MAX_MOBS;
        let mut order: [(u16, f32, bool); CANDIDATES] = [(0, 0.0, false); CANDIDATES];
        let mut n_cand = 0usize;
        for w in 0..MAX_PLAYERS {
            if c.interest[w] && self.world.players[w].active {
                order[n_cand] = (w as u16, c.accum[w], c.unsent[w] >= STALENESS_CEILING - 1);
                n_cand += 1;
            }
        }
        for slot in 0..MAX_MOBS {
            if c.m_interest[slot] && self.world.mobs.m[slot].alive {
                order[n_cand] = (
                    (MAX_PLAYERS + slot) as u16,
                    c.m_accum[slot],
                    c.m_unsent[slot] >= STALENESS_CEILING - 1,
                );
                n_cand += 1;
            }
        }
        order[..n_cand]
            .sort_unstable_by(|a, b| b.2.cmp(&a.2).then(b.1.total_cmp(&a.1)).then(a.0.cmp(&b.0)));

        let mut n_sent = 0usize;
        let own = Self::wire_entity(&self.world.players[c.own_wslot]);
        match enc.add_entity(&own) {
            Ok(()) => {
                self.sent_buf[n_sent] = own;
                n_sent += 1;
            }
            Err(_) => {
                ShardStats::bump(&stats.encode_range_errors);
                return None;
            }
        }
        let mut sent_mask = [false; MAX_PLAYERS + MAX_MOBS];
        let mut overflow_streak = 0u32;
        for &(w, _, _) in order[..n_cand].iter() {
            let w = w as usize;
            let e = if w < MAX_PLAYERS {
                Self::wire_entity(&self.world.players[w])
            } else {
                Self::wire_mob(w - MAX_PLAYERS, &self.world.mobs.m[w - MAX_PLAYERS])
            };
            match enc.add_entity(&e) {
                Ok(()) => {
                    self.sent_buf[n_sent] = e;
                    n_sent += 1;
                    sent_mask[w] = true;
                    overflow_streak = 0;
                }
                Err(WireError::Overflow) => {
                    overflow_streak += 1;
                    if overflow_streak >= FILL_OVERFLOW_STREAK {
                        break;
                    }
                }
                Err(WireError::Cap) => break,
                Err(_) => {
                    ShardStats::bump(&stats.encode_range_errors);
                }
            }
        }

        // What the fill could not carry. Counted once, here, rather than at
        // the three ways out of the loop above — an entity can be skipped by
        // an overflow and then skipped again by the break, so counting at
        // the refusal sites double-counts exactly when the budget is
        // tightest. `n_sent` includes the own entity, which is not a
        // candidate, hence the `- 1`.
        //
        // Not an error. Shedding is the designed degradation (NETCODE.md §3:
        // shed, never fragment) and it was previously the only path by which
        // snapshot quality drops under load with nothing recording it — see
        // `stats.rs` and `reference/NETWORK.md` §9.2.3.
        //
        // The offered and carried halves are added on the same three lines
        // for the same reason and with the same `- 1`: shedding is a ratio,
        // and a shed count with no denominator cannot tell a shard that
        // offered ten million from one that offered a million and one
        // (`stats.rs` on `snap_candidates`). Counting them here rather than
        // where each is known is not tidiness — `n_cand` is final at the
        // sort above, but there is a `return None` between that and here for
        // the own-entity refusal, and a snapshot that never went out must
        // not appear in any of the three.
        let carried = n_sent.saturating_sub(1);
        let shed = n_cand.saturating_sub(carried);
        ShardStats::add(&stats.snap_candidates, n_cand as u64);
        ShardStats::add(&stats.snap_entities_sent, carried as u64);
        if shed > 0 {
            ShardStats::add(&stats.snap_entities_shed, shed as u64);
        }

        let len = match enc.finish() {
            Ok(len) => len,
            Err(_) => {
                ShardStats::bump(&stats.encode_range_errors);
                return None;
            }
        };

        // The staleness bookkeeping, over the **candidate list** rather than
        // the whole packed index. `order[..n_cand]` *is* the interest set —
        // it was built from `c.interest`/`c.m_interest` a few lines up and
        // nothing between here and there can change either — so the slots
        // this skips are exactly the ones the `!interest` guard used to
        // skip, and it reaches at most `AOI_RANK_EXIT` entries instead of
        // `MAX_PLAYERS + MAX_MOBS`. Order does not matter: every arm writes
        // only its own slot.
        for &(w, _, _) in order[..n_cand].iter() {
            let w = w as usize;
            let was_sent = sent_mask[w];
            if w < MAX_PLAYERS {
                if was_sent {
                    c.accum[w] = 0.0;
                    c.unsent[w] = 0;
                } else {
                    c.unsent[w] = c.unsent[w].saturating_add(1);
                }
            } else {
                let slot = w - MAX_PLAYERS;
                if was_sent {
                    c.m_accum[slot] = 0.0;
                    c.m_unsent[slot] = 0;
                } else {
                    c.m_unsent[slot] = c.m_unsent[slot].saturating_add(1);
                }
            }
        }
        c.record_sent(
            tick,
            &self.sent_buf[..n_sent],
            &self.removed_buf[..n_removed],
        );
        Some(len)
    }
}

/// NOW.md §5b, the S→C half: the two event payload domains the wire
/// carries wider than the sim means — `EV_BAG_REMOVED`'s `why` (two bits,
/// domain 0..=2) and `EV_CONSUME_REFUSED`'s `reason` (four bits, domain
/// 1..=3) — are refused at the encode boundary. The sim cannot emit either
/// forged value, which is exactly why these tests inject them into the
/// world's ring directly and drive the **real** pump: the guard exists for
/// the emitter bug that would otherwise put a meaningless fact on every
/// client's screen. Unit tests rather than a wire suite because the pump
/// is private and the seam (`world.events` is pub) needs no socket —
/// `accept_chat`'s precedent, one lane over.
#[cfg(test)]
mod tests {
    use super::*;
    use protocol::{decode_event, EventMsg};
    use sim_core::backpack::BackpackContent;

    const SEED: u64 = 0x5B_F06E;
    const PLAYER: u32 = 7;

    /// Run the real event pump once, capturing every event-lane payload.
    fn pumped(core: &mut ShardCore, stats: &ShardStats) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        core.pump_events(stats, &mut |lane, _slot, bytes: &[u8]| {
            if lane == Lane::Event {
                out.push(bytes.to_vec());
            }
            true
        });
        out
    }

    /// A core with one connected client, its join landed, and the event
    /// ring drained to empty so a test's injected event is the only sim
    /// event the next pump sees.
    fn quiet_core(stats: &ShardStats) -> ShardCore {
        let mut core = ShardCore::new(SEED);
        assert!(core.connect(0, PLAYER), "connect");
        core.tick_bare(stats, |_, _, _| true);
        // A direct world tick clears the ring; with no content armed and
        // nobody moving, nothing refills it.
        core.world.tick(&[]);
        assert!(core.world.events.is_empty(), "ring quiet after setup");
        core
    }

    #[test]
    fn bag_removed_refuses_the_reason_the_sim_cannot_mean() {
        let stats = ShardStats::default();
        let mut core = quiet_core(&stats);
        // A real bag in the store, and a client mid-walk over it, so the
        // cursor reset below the guard is a mutation the forged event
        // would actually reach — the order half of the assert.
        core.world.backpack = BackpackContent::probe_fixture();
        let one = [ItemStack {
            item: 0,
            count: 1,
            cond: 0,
        }; INV_SLOTS];
        let w = &mut core.world;
        w.backpacks
            .stand_up(&w.backpack, 0, 0, 0, PLAYER, &one, 0, &mut w.events)
            .expect("bag stands");
        core.world.tick(&[]); // flush the EV_BAG_DROPPED it pushed
        assert!(core.world.events.is_empty(), "ring quiet again");
        core.clients[0].bag_sync_cursor = 1;
        core.clients[0].bag_sync_reset = false;

        // Just outside the domain: why == 3 fits the two-bit field, so
        // the encoder alone would put it on the wire.
        core.world
            .events
            .push(EV_BAG_REMOVED, 42, BAG_GONE_MAX + 1, 0);
        let range_before = ShardStats::get(&stats.encode_range_errors);
        let sent = pumped(&mut core, &stats);
        assert!(
            !sent
                .iter()
                .any(|b| matches!(decode_event(b), Ok(EventMsg::BagRemoved { .. }))),
            "a why the sim cannot mean crossed the wire"
        );
        assert_eq!(
            ShardStats::get(&stats.encode_range_errors),
            range_before + 1,
            "the refusal is a count"
        );
        assert_eq!(
            core.clients[0].bag_sync_cursor, 1,
            "the walk cursor moved for a refused event — the refusal is \
             not ordered before the mutation"
        );
        assert!(!core.clients[0].bag_sync_reset, "same, the reset flag");

        // Just inside: why == BAG_GONE_MAX still crosses, and the cursor
        // reset that comes with a real removal happens.
        core.world.tick(&[]);
        core.world.events.push(EV_BAG_REMOVED, 42, BAG_GONE_MAX, 0);
        let sent = pumped(&mut core, &stats);
        let removed = sent
            .iter()
            .filter_map(|b| match decode_event(b) {
                Ok(EventMsg::BagRemoved { id, why }) => Some((id, why)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(removed, vec![(42, BAG_GONE_MAX as u8)]);
        assert_eq!(
            ShardStats::get(&stats.encode_range_errors),
            range_before + 1,
            "the in-domain reason is not counted as refused"
        );
    }

    #[test]
    fn consume_refused_refuses_the_reason_the_sim_cannot_mean() {
        let stats = ShardStats::default();
        let mut core = quiet_core(&stats);

        // Just outside both ends of the domain: zero (the refusal that
        // refuses to say why) and REFUSE_C_MAX + 1 (fits the four-bit
        // field, so the encoder's width check alone would pass it).
        let range_before = ShardStats::get(&stats.encode_range_errors);
        core.world.events.push(EV_CONSUME_REFUSED, PLAYER, 0, 0);
        core.world
            .events
            .push(EV_CONSUME_REFUSED, PLAYER, REFUSE_C_MAX + 1, 0);
        let sent = pumped(&mut core, &stats);
        assert!(
            !sent
                .iter()
                .any(|b| matches!(decode_event(b), Ok(EventMsg::ConsumeRefused { .. }))),
            "a reason the sim cannot mean crossed the wire"
        );
        assert_eq!(
            ShardStats::get(&stats.encode_range_errors),
            range_before + 2,
            "both refusals are counts"
        );

        // Just inside: REFUSE_C_MAX itself still crosses.
        core.world.tick(&[]);
        core.world
            .events
            .push(EV_CONSUME_REFUSED, PLAYER, REFUSE_C_MAX, 0);
        let sent = pumped(&mut core, &stats);
        let reasons = sent
            .iter()
            .filter_map(|b| match decode_event(b) {
                Ok(EventMsg::ConsumeRefused { reason }) => Some(reason),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(reasons, vec![REFUSE_C_MAX as u8]);
        assert_eq!(
            ShardStats::get(&stats.encode_range_errors),
            range_before + 2,
            "the in-domain reason is not counted as refused"
        );
    }
}
