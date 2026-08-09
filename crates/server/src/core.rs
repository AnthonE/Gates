//! ShardCore: the sim thread's whole world — `sim_core::World` plus every
//! client's netcode state, AOI, priority fill, and snapshot encoding into
//! caller-provided sends. Pure: no I/O, no clock, no locks; the net layer
//! drives it through rings and it allocates only in `new` (L1–L4). Tests
//! drive it directly, no sockets required.

use crate::client::ClientNetState;
use crate::stats::ShardStats;
use crate::store::PlayerKey;
use protocol::{
    encode_event_auth, encode_event_bag_dropped, encode_event_bag_removed, encode_event_bag_sync,
    encode_event_build_refused, encode_event_catalog, encode_event_charge_placed,
    encode_event_chat, encode_event_consume_refused, encode_event_consumed, encode_event_cont_sync,
    encode_event_craft_done, encode_event_craft_q, encode_event_craft_refused, encode_event_death,
    encode_event_deploy_defs, encode_event_deploy_placed, encode_event_deploy_refused,
    encode_event_deploy_sync, encode_event_door, encode_event_drank, encode_event_gather,
    encode_event_health, encode_event_hit, encode_event_inv, encode_event_knock,
    encode_event_move_refused, encode_event_moved, encode_event_oven, encode_event_piece_defs,
    encode_event_piece_placed, encode_event_piece_repaired, encode_event_piece_sync,
    encode_event_recipes, encode_event_removed, encode_event_respawn, encode_event_slot_change,
    encode_event_slot_sync, encode_event_stock, encode_event_struct_hit, encode_event_vitals,
    encode_event_weak_mark, ActionMsg, ChatMsg, EntityState, InputDatagram, InvSlot, ItemCatalog,
    SnapshotEncoder, SnapshotHeader, WireBag, WireError, BAG_SYNC_BATCH, CONT_SYNC_BATCH,
    DEPLOY_SYNC_BATCH, MAX_EVENT_MSG_BYTES, PIECE_SYNC_BATCH, SLOT_SYNC_BATCH,
};
use sim_core::build::{PieceRec, LOC_PLANE};
use sim_core::craft::CraftJob;
use sim_core::deploy::DeployRec;
use sim_core::gather::{ItemStack, NO_ITEM};
use sim_core::inventory::{slots_in, CONT_BAG, CONT_BOX, CONT_SELF};
use sim_core::limits::{
    AOI_ENTER_CM, AOI_EXIT_CM, CHAT_LOCAL_CM, CRAFT_QUEUE, DATAGRAM_BUDGET_BYTES,
    HEARTH_STOCK_ROWS, INV_SLOTS, MAX_COMMANDS_PER_TICK, MAX_MOBS, MAX_PLAYERS,
    MAX_SNAPSHOT_ENTITIES, SNAPSHOT_INTERVAL_TICKS, STALENESS_CEILING, SYNC_SCAN_PER_TICK,
};
use sim_core::mob;
use sim_core::persist::PlayerSave;
use sim_core::world::{
    Command, Player, World, DEATH_BY_CLOCK, EV_AUTH, EV_BAG_DROPPED, EV_BAG_REMOVED,
    EV_BUILD_REFUSED, EV_CHARGE_PLACED, EV_CONSUMED, EV_CONSUME_REFUSED, EV_CRAFT_DONE,
    EV_CRAFT_REFUSED, EV_DEATH, EV_DEPLOY_PLACED, EV_DEPLOY_REFUSED, EV_DEPLOY_REMOVED, EV_DOOR,
    EV_DRANK, EV_GATHER, EV_HEALTH, EV_HIT, EV_KNOCK, EV_MOVED, EV_MOVE_REFUSED, EV_OVEN,
    EV_PIECE_PLACED, EV_PIECE_REMOVED, EV_PIECE_REPAIRED, EV_RESPAWN, EV_SLOT_HARVESTED,
    EV_SLOT_RESPAWNED, EV_STOCK, EV_STRUCT_HIT, EV_VITALS, EV_WEAK_MARK, STRUCT_DEPLOY_BIT,
};

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
    /// Item display names for the catalog drip. Boot input like the baked
    /// gather table: the shard installs it before the first tick; empty
    /// (the default) sends no catalog, which is what content-less tests
    /// run under.
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
                    let record = self
                        .sleepers
                        .key_of(victim)
                        .and_then(|k| self.world.save_of(victim).map(|s| (k, s)));
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
    /// every datagram), then the frame tail into the seq buffer.
    pub fn push_input(&mut self, slot: usize, dg: &InputDatagram) {
        let c = &mut self.clients[slot];
        if !c.connected {
            return;
        }
        c.on_acks(dg.snapshot_ack, dg.ack_bits);
        for f in dg.frames() {
            c.push_frame(*f);
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
    pub fn tick(&mut self, stats: &ShardStats, mut send: impl FnMut(Lane, usize, &[u8]) -> bool) {
        let mut n = self.queued_len;
        self.cmd_buf[..n].copy_from_slice(&self.queued[..n]);
        self.queued_len = 0;
        for slot in 0..MAX_PLAYERS {
            let c = &mut self.clients[slot];
            if !c.connected {
                continue;
            }
            if let Some(frame) = c.consume_input() {
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
                    // The one action that is not a command, and it says so
                    // where the table says everything else: it changes what
                    // this connection is *shown*, not what the world *is*.
                    // No `Command` carries it, the sim never hears it, the
                    // WAL never records it, and `World::state_hash` is
                    // identical either way. The `continue` is the whole
                    // statement — the arm's type is `!`, so no command is
                    // written and none is counted, and a future reader
                    // cannot add a `Command::Container` without deleting
                    // this sentence. It still spends the same one-action-
                    // per-tick hand as every other action, so an open
                    // cannot be spammed to jump the queue.
                    ActionMsg::Container { kind, cont } => {
                        c.open_container(kind, cont);
                        continue;
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
                    } => Command::Place {
                        id: c.id,
                        row,
                        cx,
                        cz,
                        level,
                        loc,
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
                    ActionMsg::Consume { slot } => Command::Consume { id: c.id, slot },
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

        self.pump_chat(stats, &mut send);
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
    fn pump_chat(&mut self, stats: &ShardStats, send: &mut impl FnMut(Lane, usize, &[u8]) -> bool) {
        for from_slot in 0..MAX_PLAYERS {
            let Some(msg) = self.clients[from_slot].pending_chat.take() else {
                continue;
            };
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
                    let rec = PieceRec {
                        cx: (ev.a >> 16) as u16,
                        cz: ev.a as u16,
                        level: (ev.b >> 16) as u8,
                        loc: (ev.b >> 8) as u8,
                        row: ev.b as u8,
                        ..PieceRec::default()
                    };
                    match encode_event_piece_placed(&rec, &mut self.ev_buf) {
                        Ok(len) => {
                            for slot in 0..MAX_PLAYERS {
                                if !self.clients[slot].connected {
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
                        _ => encode_event_consume_refused(ev.b as u8, &mut self.ev_buf),
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
                    // A knock is broadcast for the same reason a door's
                    // state is, and for one more: the whole point of the
                    // event is that somebody *other* than the sender
                    // hears it (`reference/DOORS.md` §4). AOI'ing it
                    // would silence the case it exists for — a defender
                    // asleep on the far side of their own base.
                    let (cx, cz) = ((ev.a >> 16) as u16, ev.a as u16);
                    let (level, loc) = ((ev.b >> 16) as u8, (ev.b >> 8) as u8);
                    match encode_event_knock(cx, cz, level, loc, ev.c, &mut self.ev_buf) {
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
                            let store_len = if piece {
                                self.world.pieces.len()
                            } else {
                                self.world.deploys.len()
                            };
                            for slot in 0..MAX_PLAYERS {
                                if !self.clients[slot].connected {
                                    continue;
                                }
                                // A swap-remove reshuffles the store under
                                // any in-progress walk (cursor inside the
                                // shrunken store): restart that walk with
                                // a reset batch. Finished walks (cursor
                                // past the store) hear the broadcast.
                                let c = &mut self.clients[slot];
                                if piece {
                                    if c.piece_sync_cursor > 0 && c.piece_sync_cursor <= store_len {
                                        c.piece_sync_cursor = 0;
                                        c.piece_sync_reset = true;
                                    }
                                } else if c.deploy_sync_cursor > 0
                                    && c.deploy_sync_cursor <= store_len
                                {
                                    c.deploy_sync_cursor = 0;
                                    c.deploy_sync_reset = true;
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
            for slot in 0..MAX_PLAYERS {
                if self.clients[slot].connected {
                    self.clients[slot].ev_resync();
                    ShardStats::bump(&stats.ev_resyncs);
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
        // harvested set. The store is append-only this slice, so the walk
        // is stable; a piece that also arrived by broadcast lands twice
        // and the client's address-keyed apply dedups it.
        let c = &self.clients[slot];
        let pieces = self.world.pieces.entries();
        if c.piece_sync_reset || c.piece_sync_cursor < pieces.len() {
            let n = PIECE_SYNC_BATCH.min(pieces.len() - c.piece_sync_cursor.min(pieces.len()));
            let batch = &pieces[c.piece_sync_cursor.min(pieces.len())..][..n];
            match encode_event_piece_sync(c.piece_sync_reset, batch, &mut self.ev_buf) {
                Ok(len) => {
                    if send(Lane::Event, slot, &self.ev_buf[..len]) {
                        ShardStats::bump(&stats.ev_sent);
                        let c = &mut self.clients[slot];
                        c.piece_sync_reset = false;
                        c.piece_sync_cursor += n;
                    } else {
                        return;
                    }
                }
                Err(_) => ShardStats::bump(&stats.encode_range_errors),
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
            let batch = &deploys[at..][..n];
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
            let live = match kind {
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
                        // The box stands on the plane, so its lock shares
                        // `box_key`'s triple plus `LOC_PLANE` — the move
                        // path's address, byte for byte. An oven at the
                        // same shape of address carries no lock
                        // (`lockable`) and passes as bare.
                        let b = self.world.deploys.boxes()[i];
                        self.world
                            .deploys
                            .lock_passes(b.cx, b.cz, b.level, LOC_PLANE, p.id)
                    }),
                _ => None,
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
                    for (s, out) in now.iter_mut().enumerate().take(width) {
                        *out = if kind == CONT_BAG {
                            self.world.backpacks.slot(i, s)
                        } else {
                            self.world.deploys.box_slot(i, s)
                        };
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

    /// AOI v0 (DESIGN.md §5.5, radius-only): planar hysteresis band, enter
    /// 176 m / exit 208 m, plus the NETCODE.md §3 priority accrual for
    /// everything inside. Entities leaving the client's world (range,
    /// disconnect, or slot reuse) go to the pending-removal set until an
    /// acked snapshot covers them.
    fn update_interest(&mut self, slot: usize, stats: &ShardStats) {
        let c = &mut self.clients[slot];
        if c.own_wslot == usize::MAX
            || !self.world.players[c.own_wslot].active
            || self.world.players[c.own_wslot].id != c.id
        {
            c.own_wslot = match Self::world_slot_of(&self.world, c.id) {
                Some(w) => w,
                None => return, // join command still queued
            };
        }
        let own = self.world.players[c.own_wslot].body;
        let mut overflow = false;
        for w in 0..MAX_PLAYERS {
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
            let d2 = dx * dx + dz * dz;
            if c.interest[w] {
                if d2 > AOI_EXIT_CM * AOI_EXIT_CM {
                    c.interest[w] = false;
                    c.accum[w] = 0.0;
                    c.unsent[w] = 0;
                    overflow |= !c.pending_add(p.id);
                }
            } else if d2 <= AOI_ENTER_CM * AOI_ENTER_CM {
                c.interest[w] = true;
                c.accum[w] = 0.0;
                c.unsent[w] = 0;
                c.pending_remove(p.id);
            }
            if c.interest[w] {
                let d_m = ((d2 as f32).sqrt()) * 0.01;
                c.accum[w] += PRIORITY_W_PLAYER / (1.0 + d_m / PRIORITY_HALF_SCALE_M);
            }
        }
        // The roster, on the same band and the same accrual. Simpler than
        // the player pass by exactly one thing: there is no tenant change
        // to detect, because a roster slot is one animal for the life of
        // the shard (client.rs). A death is therefore the only way an
        // animal leaves, and it leaves the way a disconnect does — into
        // the pending-removal set, so the client despawns it rather than
        // holding a corpse that never moves again.
        for slot in 0..MAX_MOBS {
            let m = &self.world.mobs.m[slot];
            if !m.alive {
                if c.m_interest[slot] {
                    overflow |= !c.pending_add(mob::mob_id(slot));
                    c.m_interest[slot] = false;
                }
                c.m_accum[slot] = 0.0;
                c.m_unsent[slot] = 0;
                continue;
            }
            let dx = (m.body.qx - own.qx) as i64 * 3;
            let dz = (m.body.qz - own.qz) as i64 * 3;
            let d2 = dx * dx + dz * dz;
            if c.m_interest[slot] {
                if d2 > AOI_EXIT_CM * AOI_EXIT_CM {
                    c.m_interest[slot] = false;
                    c.m_accum[slot] = 0.0;
                    c.m_unsent[slot] = 0;
                    overflow |= !c.pending_add(mob::mob_id(slot));
                }
            } else if d2 <= AOI_ENTER_CM * AOI_ENTER_CM {
                c.m_interest[slot] = true;
                c.m_accum[slot] = 0.0;
                c.m_unsent[slot] = 0;
                c.pending_remove(mob::mob_id(slot));
            }
            if c.m_interest[slot] {
                let d_m = ((d2 as f32).sqrt()) * 0.01;
                c.m_accum[slot] += PRIORITY_W_MOB / (1.0 + d_m / PRIORITY_HALF_SCALE_M);
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
            yaw: p.frame.yaw,
            pitch: p.frame.pitch,
        }
    }

    /// One animal as the same record. Three of the nine fields have no
    /// meaning here and each is answered rather than left to a default:
    /// `pitch` is zero because nothing about a pig looks up or down;
    /// `sleeping` is false because that bit means *nobody is driving this
    /// body*, and something always is — dormancy is not the same fact and
    /// a client would draw the slumped pose for it; `yaw` is the animal's
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

        let len = match enc.finish() {
            Ok(len) => len,
            Err(_) => {
                ShardStats::bump(&stats.encode_range_errors);
                return None;
            }
        };

        for (w, &was_sent) in sent_mask.iter().enumerate() {
            if w < MAX_PLAYERS {
                if !c.interest[w] {
                    continue;
                }
                if was_sent {
                    c.accum[w] = 0.0;
                    c.unsent[w] = 0;
                } else {
                    c.unsent[w] = c.unsent[w].saturating_add(1);
                }
            } else {
                let slot = w - MAX_PLAYERS;
                if !c.m_interest[slot] {
                    continue;
                }
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
