//! ShardCore: the sim thread's whole world — `sim_core::World` plus every
//! client's netcode state, AOI, priority fill, and snapshot encoding into
//! caller-provided sends. Pure: no I/O, no clock, no locks; the net layer
//! drives it through rings and it allocates only in `new` (L1–L4). Tests
//! drive it directly, no sockets required.

use crate::client::ClientNetState;
use crate::interest::{self, PIECE_SCAN_BATCH};
use crate::slot::MAX_CONNS;
use crate::stats::{self, ShardStats, FAVOUR_DISAGREE_BAND_TICKS};
use crate::store::PlayerKey;
use protocol::{
    encode_event_ammo, encode_event_assist, encode_event_auth, encode_event_bag_dropped,
    encode_event_bag_removed, encode_event_bag_sync, encode_event_bags, encode_event_build_refused,
    encode_event_catalog, encode_event_charge_placed, encode_event_chat,
    encode_event_consume_refused, encode_event_consumed, encode_event_cont_sync,
    encode_event_craft_done, encode_event_craft_q, encode_event_craft_refused, encode_event_death,
    encode_event_deploy_defs, encode_event_deploy_placed, encode_event_deploy_refused,
    encode_event_deploy_sync, encode_event_door, encode_event_drank, encode_event_fire,
    encode_event_gather, encode_event_gather_refused, encode_event_gitem_sync, encode_event_health,
    encode_event_heard, encode_event_hit, encode_event_hurt, encode_event_impact, encode_event_inv,
    encode_event_knock, encode_event_known, encode_event_lodged_sync, encode_event_move_refused,
    encode_event_moved, encode_event_oven, encode_event_piece_defs, encode_event_piece_placed,
    encode_event_piece_repaired, encode_event_piece_sync, encode_event_recipes,
    encode_event_recovered, encode_event_reload, encode_event_reload_refused, encode_event_removed,
    encode_event_research, encode_event_research_refused, encode_event_research_rows,
    encode_event_respawn, encode_event_shot, encode_event_slot_change, encode_event_slot_sync,
    encode_event_stock, encode_event_struct_hit, encode_event_swing, encode_event_vitals,
    encode_event_weak_mark, encode_event_wounded, ActionMsg, ChatMsg, EntityState, InputDatagram,
    InvSlot, ItemCatalog, SnapshotEncoder, SnapshotHeader, WireBag, WireError, WireGItem,
    WireLodged, BAG_SYNC_BATCH, CONT_SYNC_BATCH, DEPLOY_SYNC_BATCH, GITEM_SYNC_BATCH,
    LODGED_SYNC_BATCH, MAX_EVENT_MSG_BYTES, PIECE_SYNC_BATCH, SLOT_SYNC_BATCH,
};
use protocol::{
    DEED_DRAW, DEED_DRINK, DEED_KEYPAD, DEED_MEAL, DEED_OPEN_BAG, DEED_OPEN_BOX, DEED_RELOAD,
};
use sim_core::backpack::BAG_GONE_MAX;
use sim_core::build::{damage_band, BuildContent, PieceRec};
use sim_core::craft::CraftJob;
use sim_core::deploy::{BagAnchor, DeployContent, DeployRec, ARCH_BAG, BAG_CAP};
use sim_core::gather::{GatherContent, ItemStack, NO_ITEM};
use sim_core::inventory::{slots_in, CONT_BAG, CONT_BOX, CONT_SELF, CONT_WEAR, CONT_WORLD};
use sim_core::limits::{
    ACT_HEAR_CM, AOI_ENTER_CM, AOI_EXIT_CM, AOI_RANK_ENTER, AOI_RANK_EXIT, CHAT_LOCAL_CM,
    CRAFT_QUEUE, DATAGRAM_BUDGET_BYTES, HEARTH_STOCK_ROWS, HOTBAR_SLOTS, INV_SLOTS,
    MAX_COMMANDS_PER_TICK, MAX_MOBS, MAX_PLAYERS, MAX_SNAPSHOT_ENTITIES, MAX_SPECTATORS,
    SNAPSHOT_INTERVAL_TICKS, STALENESS_CEILING, SYNC_SCAN_PER_TICK, WEAR_SLOTS,
};
use sim_core::mob;
use sim_core::persist::PlayerSave;
use sim_core::survival::REFUSE_C_MAX;
use sim_core::world::{
    Command, Player, World, DEATH_BY_CLOCK, EV_AMMO, EV_ASSIST, EV_AUTH, EV_BAG_DROPPED,
    EV_BAG_REMOVED, EV_BUILD_REFUSED, EV_CHARGE_PLACED, EV_CONSUMED, EV_CONSUME_REFUSED,
    EV_CRAFT_DONE, EV_CRAFT_REFUSED, EV_DEATH, EV_DEPLOY_PLACED, EV_DEPLOY_REFUSED,
    EV_DEPLOY_REMOVED, EV_DOOR, EV_DRANK, EV_FIRE, EV_GATHER, EV_GATHER_REFUSED, EV_HEALTH, EV_HIT,
    EV_HOWL, EV_HURT, EV_IMPACT, EV_KNOCK, EV_KNOWN, EV_MOVED, EV_MOVE_REFUSED, EV_OVEN,
    EV_PIECE_PLACED, EV_PIECE_REMOVED, EV_PIECE_REPAIRED, EV_RECOVERED, EV_RELOAD,
    EV_RELOAD_REFUSED, EV_RESEARCH, EV_RESEARCH_REFUSED, EV_RESPAWN, EV_SENTRY_LOCK, EV_SHOT,
    EV_SLOT_HARVESTED, EV_SLOT_RESPAWNED, EV_STOCK, EV_STRUCT_HIT, EV_STUMP_GRUBBED, EV_SWING,
    EV_SWIPE, EV_SWIPE_REFUSED, EV_VEND, EV_VEND_REFUSED, EV_VITALS, EV_WEAK_MARK, EV_WOUNDED,
    STRUCT_DEPLOY_BIT,
};
use sim_core::world::{EV_ARC_DID, EV_ARC_REFUSED, EV_GAVE, EV_GROW, EV_MECH_SOLVED, EV_WORK};

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

/// Whether the sim reads this action's subject off the selected hotbar slot
/// (`combat::held_item`): the charge planted, the gun reloaded, the vessel
/// filled at the water. These wait for the frames buffered ahead of them
/// (`ClientNetState::wait_for_hand`, NOW §0rc 2); a verb that names its
/// slot or its target acts at once.
fn reads_hand(act: &ActionMsg) -> bool {
    matches!(
        act,
        ActionMsg::Throw { .. } | ActionMsg::Reload | ActionMsg::Unload | ActionMsg::Drink
    )
}

/// Ticks between two connections' crew-vital pushes: the period spread over
/// every slot a push can go to (players, then seats), so slot `s` is due on
/// its own phase and no two share a tick (`crew_vital_due`).
const CREW_VITAL_STRIDE: u64 =
    sim_core::deploy::CREW_VITAL_TICKS / (MAX_PLAYERS + MAX_SPECTATORS) as u64;
const _: () = assert!(CREW_VITAL_STRIDE > 0);

/// Whether connection `slot` is owed its crew-vital push this tick (NOW
/// §0up 3): once per `CREW_VITAL_TICKS`, on a phase of its own, so a full
/// shard's crew never ask for their bills on one tick.
fn crew_vital_due(tick: u64, slot: usize) -> bool {
    (tick + slot as u64 * CREW_VITAL_STRIDE).is_multiple_of(sim_core::deploy::CREW_VITAL_TICKS)
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
    /// Each item's description line, dripped after the catalog
    /// (`SUB_ITEM_DESC`). Empty sends nothing.
    pub item_descs: Box<protocol::ItemDescs>,
    /// The skin catalog the drip sends (skins v0), baked beside `catalog`
    /// from `content/skins.toml`. Boxed: ~9 kB of fixed capacity. Empty
    /// sends nothing.
    pub skin_catalog: Box<protocol::SkinCatalog>,
    /// Each vendor's name, kiosk order (wire v85's offer drip carries them).
    pub vendor_names: Vec<String>,
    /// Each work's name, file order, and each unlock's, code order (code 1
    /// first): the work drip carries them (wire v95).
    pub work_names: Vec<String>,
    pub unlock_names: Vec<String>,
    /// The words speakers say and stones hold (`content::bake::ArcText`):
    /// the server composes them; the sim never sees a string.
    pub arc_text: content::bake::ArcText,
    /// Scratch: event-lane encode target.
    ev_buf: [u8; MAX_EVENT_MSG_BYTES],
    /// Deeds other clients should hear this tick (wire v93): (body,
    /// `protocol::DEED_*`, item), at most one per body — so `MAX_PLAYERS`
    /// cannot fill — and flushed by [`Self::flush_heard`] at the end of
    /// `route_events`. A box opened in a drip lands after that flush and is
    /// heard with the next tick's.
    heard: [(u32, u8, u16); MAX_PLAYERS],
    heard_len: usize,
    /// Which world slots were drawing a bow last tick, for the edge a draw
    /// is heard on (`route_events`).
    drawing: [bool; MAX_PLAYERS],
    /// Who each player slot is (`EventMsg::Tag`): the proven address from
    /// the join, the platform name and picture when `faces.rs`'s read lands.
    /// A row outlives its connection, so a sleeper keeps its name for late
    /// joiners until the slot's next tenant overwrites it. Never in the sim.
    tags: Box<[TagRow]>,
    /// What each world slot's body was last seen wearing — its tenant's id,
    /// the item in each wear slot (`NO_ITEM` for none) and each piece's skin
    /// (v102, zero for none). A change owes every connection a `SUB_WORN`
    /// (`note_worn`, then `drip_client`); a reskin of a worn piece is one.
    worn_seen: [WornSeen; MAX_PLAYERS],
    /// World slots whose tenant has worn anything since it arrived. Only
    /// these are worth a `SUB_WORN`: a client draws a body it was never told
    /// about in nothing, so an outfit that has always been empty says nothing.
    worn_dressed: u128,
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
    /// Eviction records the store may not have filed yet ([`EvictMemo`]): a
    /// victim back before its record lands rejoins from this, not from the
    /// stale copy the accept loop read. In memory only: after a restart the
    /// store's copy is what a victim gets back.
    evicted: EvictMemo,
    /// The wallets this shard trusts with the admin lane (admin v0). Pure
    /// config, read once at boot and never written — the same standing as
    /// the baked content tables, and the reason it may live on a struct
    /// whose header says it holds no `ShardStats`: a list of addresses is
    /// data, not a side effect.
    admins: crate::admin::Admins,
    /// The spectator seats (`NETCODE.md` §2.3): seat `i` is connection slot
    /// `MAX_PLAYERS + i`, and holds the connection slot it watches. `None` is
    /// an empty seat. A seat is **fed only while its target is still that
    /// player on that slot** ([`Self::seat_live`]); the moment the target
    /// leaves, the seat stops receiving anything, and the accept loop closes
    /// it with `REFUSE_WATCH_ENDED` on its next sweep.
    watching: [Option<Seat>; MAX_SPECTATORS],
    /// The trust ledger's sink (`trustlog.rs`), drained once per tick.
    /// `Tap::off()` unless the shard installed a log.
    pub trust: crate::trustlog::Tap,
    /// The wipe clock (`wipe.rs`): the posted schedule, or an admin's
    /// countdown. Polled by the boundary loop with the wall clock
    /// ([`Self::wipe_poll`]), so this struct still reads no clock itself.
    pub wipe: crate::wipe::Clock,
    /// Unix seconds at the last poll: what `/wipe 10` counts from.
    wipe_now: u64,
    /// A countdown line owed to everyone, said by the next `pump_chat`.
    wipe_say: Option<protocol::ChatText>,
    /// Per slot, the tick from which it is owed the next-wipe line (a join,
    /// a few seconds after so the client is up; or a bare `/wipe`). 0 = not
    /// owed.
    wipe_tell: [u64; MAX_PLAYERS],
    /// The clock reached zero: `Some(blueprints)` until the boundary loop
    /// takes it ([`Self::take_wipe`]) and stops the shard.
    wipe_fired: Option<bool>,
    /// The wipe's standings (`standings.rs`), tallied here off the sim's
    /// events per wallet, recounted by the boundary loop once a minute.
    pub standings: crate::standings::Standings,
    /// Per slot, the tick from which it is owed the standings' join lines.
    standings_tell: [u64; MAX_PLAYERS],
    /// Lines owed to everyone by the next chat pump.
    standings_say: Vec<String>,
    /// The act at the last look, 0 before the first: an act turning over
    /// is told to everyone.
    standings_act: u8,
    /// The wipe's last hour was told: what a base holds at the wipe counts.
    standings_freeze_said: bool,
    /// Per slot, the boards it is owed (`SUB_STANDING`), one bit each.
    standings_owed: [u8; MAX_PLAYERS],
    /// The board generation everyone was last owed at.
    standings_gen_seen: u32,
    /// Per slot, whether its tenant declared itself an agent.
    standings_agent: [bool; MAX_PLAYERS],
}

/// Every standings board, one bit each (`protocol::STANDING_BOARDS`).
const STANDINGS_ALL: u8 = (1 << protocol::STANDING_BOARDS) - 1;
const _: () = assert!(crate::standings::TOP == protocol::STANDING_TOP);
const _: () = assert!(crate::standings::BOARD_LAST + 1 == protocol::STANDING_BOARDS);

/// One spectator seat's sim-side state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Seat {
    /// The watched player's connection slot, `< MAX_PLAYERS`.
    target: usize,
    /// The own-facts a player hears once, at its join, as events — health,
    /// vitals, what it has researched — are still owed to this watcher. Sent
    /// once from the body's state and cleared; the mirrored events keep it
    /// current after that. (Inventory, wear and the craft queue need nothing:
    /// they are diffed per connection, so a new seat's empty shadow already
    /// asks for the whole of them.)
    catchup: bool,
}

/// Where a mirrored event copy goes: `NO_SEAT` for an empty seat, else the
/// target's connection slot. `u8` because `MAX_PLAYERS` fits one, and the
/// table is copied into the mirror closure every tick.
const NO_SEAT: u8 = u8::MAX;
const _: () = assert!(MAX_PLAYERS < NO_SEAT as usize);

/// The side channels an admin verb needs and the sim's own state
/// cannot provide — passed into [`ShardCore::tick`] rather than held,
/// because every one of them belongs to a thread that is not this one.
///
/// A bundle rather than four parameters for `charge::tick_fuses`' reason
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
    /// The accept loop's answers to those acts, said to the admin by the
    /// chat pump (`admin::AdminReply`). `None` ⇒ no accept loop, no answers.
    pub admin_answers: Option<&'a mut rtrb::Consumer<crate::admin::AdminReply>>,
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
    /// Restored from a record — the world did not have them. The shard's
    /// own eviction record when it kept one for the key (`EvictMemo`),
    /// else the store's, as the accept loop read it.
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

/// The sim's own copy of every eviction record it has handed out, by key
/// (NOW §0y 2). `connect_as` reads it ahead of the record a joiner brings.
///
/// The eviction record leaves the sim on the save ring and reaches the
/// store a hop later, but a reconnecting victim's `install` reads the store
/// on the accept loop, *before* its `Connect` reaches the sim. Inside the
/// eviction's window — or for good, if a full save ring dropped the record
/// — it brings the copy its leave filed, frozen before the raid, and
/// `JoinAs` would undo the raid that the current-body save exists to keep.
/// The current record is already in hand here, so it wins.
///
/// It may always win, though not because it is newer than the store's
/// copy: once the ring files it, the store holds this very record. What
/// makes it safe is that nothing newer can be written while it stands. The
/// only writer of a key's records is that key's own connection (its leave,
/// its autosaves), and every admission of the key spends the entry, as
/// does any leave of it — so a victim who comes back, plays on and is
/// restored later gets the store's newer record, never this one
/// (`persist_store.rs`). Nothing expires on a clock for that reason; a full
/// table drops its oldest put, which costs that one victim only the hole
/// this closes. Boxed at `new`, and `PlayerSave` is `Copy`, so a put never
/// allocates.
struct EvictMemo {
    entries: Box<[Option<(PlayerKey, PlayerSave, u64)>]>,
    /// The next put's stamp: oldest-first replacement with no ties when one
    /// window evicts more than once.
    next: u64,
}

impl EvictMemo {
    fn new() -> Self {
        Self {
            entries: vec![None; MAX_PLAYERS].into_boxed_slice(),
            next: 0,
        }
    }

    fn find(&self, key: &PlayerKey) -> Option<PlayerSave> {
        self.entries
            .iter()
            .flatten()
            .find(|(k, _, _)| k == key)
            .map(|(_, s, _)| *s)
    }

    fn forget(&mut self, key: &PlayerKey) {
        for e in self.entries.iter_mut() {
            if e.map(|(k, _, _)| k == *key).unwrap_or(false) {
                *e = None;
            }
        }
    }

    /// File `save` under `key`, replacing any earlier one for the same key,
    /// else into a free row, else over the oldest put.
    fn put(&mut self, key: &PlayerKey, save: PlayerSave) {
        self.forget(key);
        let stamp = |e: &Option<(PlayerKey, PlayerSave, u64)>| e.map_or(0, |(_, _, n)| n);
        let row = self
            .entries
            .iter()
            .position(|e| e.is_none())
            .or_else(|| (0..self.entries.len()).min_by_key(|&i| stamp(&self.entries[i])));
        if let Some(i) = row {
            self.entries[i] = Some((*key, save, self.next));
            self.next += 1;
        }
    }
}

/// What one world slot's body was last seen wearing (`ShardCore::worn_seen`):
/// its tenant's id, the item in each wear slot and each piece's skin.
type WornSeen = (u32, [u16; WEAR_SLOTS], [u16; WEAR_SLOTS]);
/// An empty world slot's: nobody, wearing nothing.
const WORN_NONE: WornSeen = (0, [sim_core::gather::NO_ITEM; WEAR_SLOTS], [0; WEAR_SLOTS]);

/// One player slot's tag (`ShardCore::tags`). `id == 0` is an empty row —
/// a guest or a slot nobody has joined — and is never sent.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct TagRow {
    id: u32,
    address: protocol::Address,
    name: protocol::Name,
    pic: u32,
}

impl ShardCore {
    pub fn new(seed: u64) -> Self {
        // Every connection slot: players, then spectator seats.
        let mut clients = Vec::with_capacity(MAX_CONNS);
        clients.resize_with(MAX_CONNS, ClientNetState::new);
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
            item_descs: Box::new(protocol::ItemDescs::EMPTY),
            skin_catalog: Box::new(protocol::SkinCatalog::EMPTY),
            vendor_names: Vec::new(),
            work_names: Vec::new(),
            unlock_names: Vec::new(),
            arc_text: content::bake::ArcText::default(),
            ev_buf: [0; MAX_EVENT_MSG_BYTES],
            heard: [(0, 0, 0); MAX_PLAYERS],
            heard_len: 0,
            drawing: [false; MAX_PLAYERS],
            tags: vec![TagRow::default(); MAX_PLAYERS].into_boxed_slice(),
            worn_seen: [WORN_NONE; MAX_PLAYERS],
            worn_dressed: 0,
            admins: crate::admin::Admins::none(),
            autosave_at: 0,
            last_saved: vec![PlayerSave::EMPTY; MAX_PLAYERS].into_boxed_slice(),
            keys: vec![None; MAX_PLAYERS].into_boxed_slice(),
            sleepers: SleeperIndex::new(),
            evicted: EvictMemo::new(),
            watching: [None; MAX_SPECTATORS],
            trust: crate::trustlog::Tap::off(),
            wipe: crate::wipe::Clock::off(),
            wipe_now: 0,
            wipe_say: None,
            wipe_tell: [0; MAX_PLAYERS],
            wipe_fired: None,
            standings: crate::standings::Standings::off(),
            standings_tell: [0; MAX_PLAYERS],
            standings_say: Vec::new(),
            standings_act: 0,
            standings_freeze_said: false,
            standings_owed: [0; MAX_PLAYERS],
            standings_gen_seen: 0,
            standings_agent: [false; MAX_PLAYERS],
        }
    }

    /// One poll of the wipe clock, from the boundary loop about once a
    /// second with the wall clock's unix seconds. A line to say is held for
    /// the next tick's chat pump; a wipe that came due is held for
    /// [`Self::take_wipe`].
    pub fn wipe_poll(&mut self, now: u64) {
        self.wipe_now = now;
        match self.wipe.poll(now) {
            crate::wipe::Tick::Quiet => {}
            crate::wipe::Tick::Say(line) => {
                self.wipe_say = crate::wipe::chat(&line);
                let left = self
                    .wipe
                    .next()
                    .map_or(u64::MAX, |p| p.at.saturating_sub(now));
                if left <= 3_600 && !self.standings_freeze_said {
                    self.standings_freeze_said = true;
                    self.standings_say
                        .push("the hoard freezes at the wipe".into());
                    self.standings_say
                        .push("what your base holds then counts".into());
                }
            }
            crate::wipe::Tick::Wipe { blueprints, line } => {
                // The last count: what every base holds as the island ends.
                self.standings_recount();
                self.standings.rebuild();
                let last = self.standings.final_lines();
                self.standings_say.extend(last);
                self.wipe_say = crate::wipe::chat(&line);
                self.wipe_fired = Some(blueprints);
            }
        }
    }

    /// The wipe that came due, once: `Some(blueprints)`.
    pub fn take_wipe(&mut self) -> Option<bool> {
        self.wipe_fired.take()
    }

    /// Say a server line to one connected slot, or to all of them.
    /// `/brain`'s answer for the player `who`: the nearest living animal to
    /// their body (`admin::brain_line`), or why there is none.
    fn brain_answer(&self, who: u32) -> String {
        let w = &self.world;
        let Some(me) = w.players.iter().find(|p| p.active && p.id == who) else {
            return "no body to measure from".to_string();
        };
        let (mx, mz) = (
            me.body.qx as f32 * sim_core::movement::POS_XZ_Q,
            me.body.qz as f32 * sim_core::movement::POS_XZ_Q,
        );
        let mut best: Option<(usize, f32)> = None;
        for (slot, m) in w.mobs.m.iter().enumerate() {
            if !m.alive {
                continue;
            }
            let dx = m.body.qx as f32 * sim_core::movement::POS_XZ_Q - mx;
            let dz = m.body.qz as f32 * sim_core::movement::POS_XZ_Q - mz;
            let d2 = dx * dx + dz * dz;
            if best.is_none_or(|(_, b)| d2 < b) {
                best = Some((slot, d2));
            }
        }
        let Some((slot, d2)) = best else {
            return "no animal alive".to_string();
        };
        let m = &w.mobs.m[slot];
        let target = w
            .players
            .get(m.target as usize)
            .filter(|p| p.active)
            .map(|p| p.id);
        crate::admin::brain_line(slot, m, target, d2.sqrt(), w.tick)
    }

    /// An admin verb's answer, said to the asker alone as a `[server]` line.
    fn answer(
        &mut self,
        to: usize,
        mut line: String,
        stats: &ShardStats,
        send: &mut impl FnMut(Lane, usize, &[u8]) -> bool,
    ) {
        line.truncate(protocol::ChatText::CAP);
        if let Some(text) = protocol::ChatText::sanitize(line.as_bytes()) {
            let line = crate::admin::server_line(&text);
            self.say_server(Some(to), &line, stats, send);
        }
    }

    fn say_server(
        &mut self,
        to: Option<usize>,
        line: &protocol::ChatText,
        stats: &ShardStats,
        send: &mut impl FnMut(Lane, usize, &[u8]) -> bool,
    ) {
        let len = match protocol::encode_event_chat(0, true, line, &mut self.ev_buf) {
            Ok(len) => len,
            Err(_) => {
                ShardStats::bump(&stats.encode_range_errors);
                return;
            }
        };
        for to_slot in 0..MAX_PLAYERS {
            if !self.clients[to_slot].connected || to.is_some_and(|t| t != to_slot) {
                continue;
            }
            if send(Lane::Event, to_slot, &self.ev_buf[..len]) {
                ShardStats::bump(&stats.ev_sent);
            } else {
                // `pump_chat`'s policy: a lost line is counted and not
                // resynced, because no walk would bring it back.
                ShardStats::bump(&stats.chat_undelivered);
            }
        }
    }

    /// The wipe lines this tick owes: the countdown to everyone, and the
    /// next-wipe line to whoever joined or asked.
    fn pump_wipe(&mut self, stats: &ShardStats, send: &mut impl FnMut(Lane, usize, &[u8]) -> bool) {
        if let Some(line) = self.wipe_say.take() {
            self.say_server(None, &line, stats, send);
        }
        for slot in 0..MAX_PLAYERS {
            let at = self.wipe_tell[slot];
            if at == 0 || self.world.tick < at {
                continue;
            }
            self.wipe_tell[slot] = 0;
            let line = self
                .wipe
                .notice()
                .unwrap_or_else(|| "no wipe is scheduled".into());
            if let Some(line) = crate::wipe::chat(&line) {
                self.say_server(Some(slot), &line, stats, send);
            }
        }
    }

    /// Queue the sky/clock verb outside the admin lane — the boot's
    /// `dev_env` (`config.rs`). False ⇒ the command buffer was full.
    pub fn queue_env(&mut self, weather: u8, time_pm: u16) -> bool {
        self.queue(Command::AdminEnv { weather, time_pm })
    }

    /// Player `id` joined connection `slot` as `key` (the proven wallet, or
    /// `None` for a guest). Their tag starts as the address alone and is owed
    /// to everyone; the platform name follows through [`Self::set_face`].
    pub fn tag_join(&mut self, slot: usize, id: u32, key: Option<&PlayerKey>) {
        if slot >= MAX_PLAYERS {
            return;
        }
        // The next wipe, said a few seconds after the door so the client is
        // up to read it (`pump_wipe`).
        if self.wipe.next().is_some() {
            self.wipe_tell[slot] = self.world.tick + 5 * sim_core::limits::TICK_HZ as u64;
        }
        // Then the standings: last wipe, your hall, where you stand.
        self.standings_tell[slot] = self.world.tick + 7 * sim_core::limits::TICK_HZ as u64;
        self.standings_owed[slot] = STANDINGS_ALL;
        let address = key.and_then(|k| protocol::Address::from_hex(k.as_bytes()));
        self.tags[slot] = match address {
            Some(address) if !address.is_guest() => TagRow {
                id,
                address,
                name: protocol::Name::EMPTY,
                pic: 0,
            },
            _ => TagRow::default(),
        };
        self.owe_tag(slot);
    }

    /// The platform said what player `id` on `slot` is called and looks like
    /// (`faces.rs`). Dropped if the slot has a new tenant; a no-op if nothing
    /// moved, so a repeated read costs no bandwidth.
    pub fn set_face(&mut self, slot: usize, id: u32, name: protocol::Name, pic: u32) {
        if slot >= MAX_PLAYERS || !self.clients[slot].connected || self.clients[slot].id != id {
            return;
        }
        let row = &mut self.tags[slot];
        if row.id != id || (row.name == name && row.pic == pic) {
            return;
        }
        row.name = name;
        row.pic = pic;
        self.owe_tag(slot);
        if let Some(key) = self.key_str(slot) {
            self.standings.name(&key, name.as_str());
        }
    }

    /// Whether `slot`'s new tenant declared itself an agent (`HELLO_AGENT`).
    pub fn note_agent(&mut self, slot: usize, agent: bool) {
        if let Some(a) = self.standings_agent.get_mut(slot) {
            *a = agent;
        }
    }

    /// A minute of play for every connected wallet: what a purse's
    /// `prize_min_minutes` is measured in. From the boundary loop's minute.
    pub fn standings_minute(&mut self) {
        for slot in 0..MAX_PLAYERS {
            if !self.clients[slot].connected {
                continue;
            }
            let Some(key) = self.key_str(slot) else {
                continue;
            };
            let tag = &self.tags[slot];
            let name = if tag.id == self.clients[slot].id {
                tag.name.as_str().to_string()
            } else {
                String::new()
            };
            let agent = self.standings_agent[slot];
            self.standings.minute(&key, &name, agent);
        }
    }

    /// The wallet on `slot`, as the standings file it. `None` for a guest.
    fn key_str(&self, slot: usize) -> Option<String> {
        let k = self.keys.get(slot)?.as_ref()?;
        core::str::from_utf8(k.as_bytes()).ok().map(str::to_string)
    }

    /// Who player `id` is to the standings: their wallet and what the
    /// platform calls them (empty for not yet). A connected player, else a
    /// sleeper; a guest, an animal or a gone body is nobody.
    fn standing_who(&self, id: u32) -> Option<(String, String)> {
        if let Some(slot) =
            (0..MAX_PLAYERS).find(|&s| self.clients[s].connected && self.clients[s].id == id)
        {
            let key = self.key_str(slot)?;
            let tag = &self.tags[slot];
            let name = if tag.id == id { tag.name.as_str() } else { "" };
            return Some((key, name.to_string()));
        }
        let k = self.sleepers.key_of(id)?;
        let key = core::str::from_utf8(k.as_bytes()).ok()?.to_string();
        Some((key, String::new()))
    }

    /// This tick's events, tallied: what was given to the works, the deeds,
    /// the kills. After `World::tick`, before the events are routed.
    fn note_standings(&mut self) {
        use sim_core::works::{WORK_EV_LIT, WORK_EV_REKINDLED};
        for i in 0..self.world.events.len() {
            let ev = self.world.events.entries()[i];
            match ev.code {
                EV_GAVE => {
                    if let Some((k, l)) = self.standing_who(ev.a) {
                        self.standings.gave(&k, &l, (ev.b >> 16) as u16, ev.c);
                    }
                }
                EV_WORK if ev.b == WORK_EV_LIT || ev.b == WORK_EV_REKINDLED => {
                    let Some((k, l)) = self.standing_who(ev.c) else {
                        continue;
                    };
                    let name = self
                        .work_names
                        .get(ev.a as usize)
                        .map_or("a work", |n| n.as_str());
                    if ev.b == WORK_EV_LIT {
                        let act = self
                            .world
                            .works_def
                            .get(ev.a as usize)
                            .map_or(1, |d| d.act.max(1)) as u64;
                        let what = format!("lit {name}");
                        let points = self.standings.rules.lit_points * act;
                        self.standings.deed(&k, &l, points, Some(&what));
                    } else {
                        let points = self.standings.rules.rekindled_points;
                        self.standings.deed(&k, &l, points, None);
                    }
                }
                EV_MECH_SOLVED => {
                    let Some((k, l)) = self.standing_who(ev.b) else {
                        continue;
                    };
                    let name = self
                        .arc_text
                        .mech_names
                        .get(ev.a as usize)
                        .map_or("a lock", |n| n.as_str());
                    let what = format!("opened {name}");
                    let points = self.standings.rules.solved_points;
                    self.standings.deed(&k, &l, points, Some(&what));
                }
                EV_DEATH => {
                    let victim = self.standing_who(ev.a);
                    let killer = (ev.b != ev.a).then(|| self.standing_who(ev.b)).flatten();
                    self.standings.kill(
                        killer.as_ref().map(|(k, l)| (k.as_str(), l.as_str())),
                        victim.as_ref().map(|(k, l)| (k.as_str(), l.as_str())),
                    );
                }
                _ => {}
            }
        }
    }

    /// Count every base: its boxes' contents, its hearth's stock and the
    /// deployables standing in its claim, split evenly across the crew the
    /// standings can name. Once a minute from the boundary loop, and at the
    /// wipe and the shutdown before the last save. Against the claim cache
    /// the last tick's sweep refreshed (`Deploys::crew_hearth_at`'s
    /// contract).
    pub fn standings_recount(&mut self) {
        let d = &self.world.deploys;
        let dc = &self.world.deploy;
        let rules = &self.standings.rules;
        let hearths = d.hearths();
        let mut value = vec![0u64; hearths.len()];
        let covering = |x: f32, z: f32| (0..hearths.len()).find(|&hi| d.hearth_covers(hi, x, z));
        for b in d.boxes() {
            let (x, z) = b.xz();
            if let Some(hi) = covering(x, z) {
                for st in b.items.iter().filter(|st| st.count > 0) {
                    value[hi] += rules.worth(st.item, st.count as u32);
                }
            }
        }
        for e in d.entries() {
            let (x, z) = e.xz();
            if let (Some(hi), Some(def)) = (covering(x, z), dc.defs.get(e.row as usize)) {
                value[hi] += rules.worth(def.item, 1);
            }
        }
        for (hi, h) in hearths.iter().enumerate() {
            for (i, &n) in h.stock.iter().enumerate().take(dc.mat_count as usize) {
                value[hi] += rules.worth(dc.mats[i], n);
            }
        }
        let mut shares = Vec::new();
        for (hi, h) in hearths.iter().enumerate() {
            if value[hi] == 0 {
                continue;
            }
            let crew: Vec<(String, String)> = h
                .crew
                .members()
                .iter()
                .filter_map(|&id| self.standing_who(id))
                .collect();
            if crew.is_empty() {
                continue;
            }
            let each = value[hi] / crew.len() as u64;
            shares.extend(crew.into_iter().map(|(k, l)| (k, l, each)));
        }
        self.standings.set_hoards(&shares);
    }

    /// What the standings owe chat this tick: the join lines, an act
    /// turning over, the leaders every two hours, the wipe's last word.
    fn pump_standings(
        &mut self,
        stats: &ShardStats,
        send: &mut impl FnMut(Lane, usize, &[u8]) -> bool,
    ) {
        let tick = self.world.tick;
        let act = self.world.works.act(&self.world.works_def);
        if self.standings_act != 0 && act > self.standings_act {
            self.standings_say.push(format!(
                "ACT {}: the works wake",
                ["I", "II", "III", "IV"][(act as usize).clamp(1, 4) - 1]
            ));
            self.standings_say
                .push("what you give them counts: /top".into());
        }
        self.standings_act = act;
        let every = 2 * 3_600 * sim_core::limits::TICK_HZ as u64;
        if tick > 0 && tick.is_multiple_of(every) {
            self.standings.rebuild();
            if !self
                .standings
                .board(crate::standings::BOARD_ISLAND)
                .top
                .is_empty()
            {
                let line = self.standings.leader_line(crate::standings::BOARD_ISLAND);
                self.standings_say.push(format!("leads: {line}"));
            }
        }
        for line in std::mem::take(&mut self.standings_say) {
            if let Some(text) = crate::wipe::chat(&line) {
                self.say_server(None, &text, stats, send);
            }
        }
        for slot in 0..MAX_PLAYERS {
            let at = self.standings_tell[slot];
            if at == 0 || tick < at {
                continue;
            }
            self.standings_tell[slot] = 0;
            if !self.clients[slot].connected {
                continue;
            }
            self.standings.rebuild();
            let key = self.key_str(slot);
            for line in self.standings.join_lines(key.as_deref()) {
                self.answer(slot, line, stats, send);
            }
        }
    }

    /// Look at what every body wears, and owe each connection the ones that
    /// changed since last tick (a new tenant in a slot counts as a change).
    /// 100 slots × two compares a tick.
    fn note_worn(&mut self) {
        for w in 0..MAX_PLAYERS {
            let p = &self.world.players[w];
            let now = if p.active {
                let mut items = [sim_core::gather::NO_ITEM; WEAR_SLOTS];
                let mut skins = [0u16; WEAR_SLOTS];
                for ((item, skin), s) in items.iter_mut().zip(skins.iter_mut()).zip(p.worn.iter()) {
                    if s.count > 0 {
                        *item = s.item;
                        *skin = s.skin;
                    }
                }
                (p.id, items, skins)
            } else {
                WORN_NONE
            };
            if self.worn_seen[w] != now {
                let bit = 1u128 << w;
                if self.worn_seen[w].0 != now.0 {
                    self.worn_dressed &= !bit; // a new tenant starts undressed
                }
                if now.1 != [sim_core::gather::NO_ITEM; WEAR_SLOTS] {
                    self.worn_dressed |= bit;
                }
                self.worn_seen[w] = now;
                if self.worn_dressed & bit != 0 {
                    for c in self.clients.iter_mut() {
                        c.worn_owed |= bit;
                    }
                }
            }
        }
    }

    fn owe_tag(&mut self, slot: usize) {
        let bit = 1u128 << slot;
        for c in self.clients.iter_mut() {
            c.tags_owed |= bit;
        }
    }

    /// The platform said what connection `slot`, player `id`, owns
    /// (`skins.rs`, read off the sim thread by the accept loop). Held on the
    /// client until the next tick queues it as `Command::SkinsOwned`, so a
    /// full command queue delays the set rather than dropping it. An id
    /// that no longer names this slot's tenant is a stale answer about
    /// somebody who left, and is dropped.
    pub fn skins_owned(&mut self, slot: usize, id: u32, owned: sim_core::skin::SkinSet) {
        let Some(c) = self.clients.get_mut(slot) else {
            return;
        };
        if c.connected && c.id == id {
            c.skins_pending = Some(owned);
        }
    }

    /// The platform's item store said what each skin costs
    /// (`skins::prices_of`). A row whose coin or price moved is rewritten,
    /// and when any did every client's skin drip starts over, so a store
    /// screen shows what the store charges. Rows are overwritten by index on
    /// the client, so a re-drip replaces and never appends.
    pub fn skin_prices(
        &mut self,
        prices: &[Option<crate::skins::Price>; sim_core::limits::MAX_SKINS],
    ) {
        let n = (self.skin_catalog.count as usize).min(sim_core::limits::MAX_SKINS);
        let mut moved = false;
        for (row, price) in self.skin_catalog.rows[..n].iter_mut().zip(prices) {
            let (coin, amount) = price.unwrap_or((protocol::COIN_NONE, 0));
            if row.coin != coin || row.price != amount {
                row.coin = coin;
                row.price = amount;
                moved = true;
            }
        }
        if moved {
            for c in self.clients.iter_mut() {
                c.skins_cursor = 0;
            }
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
        // Players only: a spectator seat is not a player and must not read
        // as one on the status endpoint (`MAX_PLAYERS` bounds the scan).
        self.clients[..MAX_PLAYERS]
            .iter()
            .filter(|c| c.connected)
            .count()
    }

    /// Occupied spectator seats — the `spectators` gauge (`stats.rs`).
    pub fn spectators(&self) -> usize {
        self.watching.iter().filter(|w| w.is_some()).count()
    }

    /// Seat a spectator on connection slot `slot` (past `MAX_PLAYERS`),
    /// watching the player `target_id` on connection slot `target`.
    ///
    /// **No command, and that is the design**: a watcher has no body, so the
    /// world never hears of it — nothing enters the WAL, `state_hash` is
    /// untouched, and a replay without the watcher is the same run (wall 5).
    /// What it gets is a connection's netcode state bound to the target's
    /// body: `id` is the target's, so interest, the drips and the snapshot's
    /// own record all resolve to the body being watched.
    ///
    /// False ⇒ refused: not a seat slot, or the target is no longer that id on
    /// that slot (it left between the accept loop's check and this tick). The
    /// caller parks the link and lets the LEAVING sweep take it.
    #[must_use]
    pub fn connect_spectator(&mut self, slot: usize, target_id: u32, target: usize) -> bool {
        if !(MAX_PLAYERS..MAX_CONNS).contains(&slot)
            || target >= MAX_PLAYERS
            || !self.clients[target].connected
            || self.clients[target].id != target_id
        {
            return false;
        }
        self.clients[slot].reset(target_id);
        self.watching[slot - MAX_PLAYERS] = Some(Seat {
            target,
            catchup: true,
        });
        true
    }

    /// Is spectator seat `i` still watching the player it was seated for?
    /// A target's connection slot is reused after it leaves, and the new
    /// tenant has a different id — so the id check is what stops a seat
    /// being fed a stranger's view (a privacy defect, not a cosmetic one).
    fn seat_live(&self, i: usize) -> bool {
        match self.watching[i] {
            Some(seat) => {
                let t = &self.clients[seat.target];
                let me = &self.clients[MAX_PLAYERS + i];
                me.connected && t.connected && t.id == me.id
            }
            None => false,
        }
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
    ///
    /// **`save` is what the accept loop read, and it can be stale.** It
    /// fetched the store before this join's `Connect` reached the sim, so a
    /// victim reconnecting in its own eviction's window (or after the save
    /// ring dropped the eviction record) brings the copy its leave filed,
    /// raid not included. The record this function kept at the pick
    /// ([`EvictMemo`]) outranks it whenever the world has no body for the
    /// key (NOW §0y 2).
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
        // No body, so a record: this shard's own eviction record first, the
        // store's (as the accept loop read it) only if it has none. Peeked,
        // not taken — it is spent below only once the join is queued.
        let save = match key {
            Some(k) if sleeper.is_none() => self.evicted.find(&k).or(save),
            _ => save,
        };
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
                    if let Some((k, s)) = record.as_ref() {
                        // The arrow points at a body the command below is
                        // about to remove. The record is kept for the
                        // victim's return, which may beat the store to it.
                        self.sleepers.forget(k);
                        self.evicted.put(k, *s);
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
            // Whichever door opened, the world has a body for this key now,
            // and every record the store gets for it from here on is newer
            // than an eviction's.
            self.evicted.forget(k);
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
        // A spectator seat leaves nothing behind: no body, no sleeper, no
        // record, no command. Branched first because every array below this
        // line is player-sized.
        if slot >= MAX_PLAYERS {
            if slot < MAX_CONNS {
                self.clients[slot].connected = false;
                self.watching[slot - MAX_PLAYERS] = None;
            }
            return None;
        }
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
            // This leave's record supersedes any eviction record for the
            // key — reachable only when one key held two connections.
            self.evicted.forget(&k);
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
    ///
    /// **And the stamp is now the fresher of two readings, not the claim**
    /// (lagcomp slice 5). Once a favour is minted from this number, the
    /// number is worth lying about: a client that acks *backwards* looks
    /// staler than it is and buys rewind depth it has not earned, which is
    /// peeker's advantage on demand. `newest_acked` is the server's own
    /// record of the newest snapshot it has seen this client ack, so the
    /// two are independent and the smaller staleness wins
    /// (`findings/lagcomp-design-20260818.md` §6.2's stated rule).
    ///
    /// ⚠ **§6.2's mechanism was a wall clock and this is not it.** That
    /// bullet asks for an `Instant`-derived RTT estimate on the I/O thread
    /// compared against the ack-derived staleness. It is not buildable as
    /// written: the only transport RTT this shard reads is quinn's, folded
    /// into a shard-wide `net_rtt_us_max` high-water mark at ~1 Hz
    /// (`net.rs`), and there is no per-client channel from the I/O tasks to
    /// this thread at all — `Link`'s four rings carry datagrams, not
    /// derived numbers. Building one would be a per-slot table invented for
    /// a counter, which is the thing `stats.rs`'s aim-staleness block
    /// explicitly decided against. The check here is strictly cheaper and
    /// strictly better: no clock (so it is a gate under `CLAUDE.md`'s
    /// rule), no new transport, and it compares the claim against
    /// *evidence* rather than against a second estimate.
    pub fn push_input(&mut self, slot: usize, dg: &InputDatagram) {
        // Read before the client is borrowed: `world` and `clients` are two
        // fields of one `self`, the `tick` loop's problem one method over.
        let now = self.world.tick as u16;
        let c = &mut self.clients[slot];
        if !c.connected {
            return;
        }
        c.on_acks(dg.snapshot_ack, dg.ack_bits);
        // Copied out before the block below, which needs `c` mutably to
        // count a disagreement.
        let newest = c.newest_acked;
        let view = newest.map(|b| {
            // **The cross-check, and it needed no clock**
            // (`findings/lagcomp-design-20260818.md` §6.2, built differently
            // — see this method's doc).
            //
            // Two independent readings of one quantity, both already here:
            // `dg.snapshot_ack` is what the client *claims* its newest
            // applied world is, and `newest_acked` is the newest snapshot
            // the server has *watched it ack* out of the server's own sent
            // ring. Compare them as ages against `now` rather than as
            // magnitudes, because `snapshot_ack` is 16 bits of a `u64` tick
            // and a shard crosses that boundary every 36 minutes.
            let claim = now.wrapping_sub(dg.snapshot_ack);
            let evidence = now.wrapping_sub(b as u16);
            if evidence >= claim {
                return dg.snapshot_ack;
            }
            // §6.2's rule verbatim — **use the smaller of the two
            // estimates.** A client acking backwards is asking to be
            // treated as staler than the server has seen it be, and
            // staleness is what buys rewind depth.
            if claim - evidence > FAVOUR_DISAGREE_BAND_TICKS {
                c.note_ack_regression();
            }
            b as u16
        });
        // The playout report is a level, not an edge: latest datagram
        // wins, exactly as the nudge and the gauges are levels in every
        // header going the other way. Clamped at the favour mint, not
        // here — the mint is the one place the claim buys anything.
        c.reported_playout = dg.playout_ticks;
        // A spectator's datagram is its acks and nothing else. The net side
        // already refuses one carrying frames (`spectate_input_refused`);
        // this is the second wall, so a frame that got past it still reaches
        // no command — and the tick's input loop stops at `MAX_PLAYERS`
        // besides, which is the third.
        if slot >= MAX_PLAYERS {
            return;
        }
        for f in dg.frames() {
            c.push_frame(*f, view);
        }
    }

    /// Whether this client can accept another C→S action this tick — the
    /// net thread pops its action ring only through an open hand, so a
    /// deferred action stays ringed (and, past the ring, in the stream).
    pub fn wants_action(&self, slot: usize) -> bool {
        // A spectator never acts; its hand is never open.
        slot < MAX_PLAYERS
            && self.clients[slot].connected
            && self.clients[slot].pending_action.is_none()
    }

    /// Hand one decoded action to this client's pending slot. Callers
    /// check `wants_action` first; a push into a full hand is dropped
    /// (defensive — the contract keeps it unreachable).
    pub fn push_action(&mut self, slot: usize, act: ActionMsg) {
        if slot >= MAX_PLAYERS {
            return;
        }
        let c = &mut self.clients[slot];
        if c.connected && c.pending_action.is_none() {
            c.wait_for_hand(reads_hand(&act));
            c.pending_action = Some(act);
        }
    }

    /// Hand one decoded chat line to this client's pending slot. The line
    /// is said next tick or not at all — a second line arriving in the
    /// same tick replaces the first, which cannot happen through the net
    /// path (it pops one per tick) and is the right answer if it ever
    /// does: chat is not owed delivery the way an action is.
    pub fn push_chat(&mut self, slot: usize, chat: ChatMsg) {
        if slot >= MAX_PLAYERS {
            return;
        }
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
            admin_answers: None,
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
        // Owned skin sets the platform answered since the last tick, queued
        // behind whatever else this window holds — so a join queued in the
        // same window lands first and the set finds its body.
        for slot in 0..MAX_PLAYERS {
            if let Some(owned) = self.clients[slot].skins_pending {
                let id = self.clients[slot].id;
                if self.queue(Command::SkinsOwned { id, owned }) {
                    self.clients[slot].skins_pending = None;
                }
            }
        }
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
            if let Some(consumed) = c.consume_input() {
                let (frame, view) = (consumed.frame, consumed.view);
                // Measured on the frame whose buttons ACT (the newer of
                // two, when the throttle consumed two), and measured before
                // the command-buffer check on purpose: dropping the sample
                // exactly on the ticks that ran out of command room would
                // bias the distribution toward the quiet ticks, which is
                // the reverse of what a lag measurement is for.
                stats.record_aim_stale(now, view);
                // Every disagreement this client's datagrams accumulated
                // since the last tick, drained here because this is the one
                // reader (`ClientNetState::take_ack_regressions`) and
                // because it belongs beside the number it is about.
                ShardStats::add(&stats.favour_disagree, c.take_ack_regressions());
                if n < MAX_COMMANDS_PER_TICK {
                    // **The mint** — lag compensation stops being dead code
                    // on this line (`findings/lagcomp-design-20260818.md`
                    // §7 slice 5). Everything under it shipped first and
                    // sat unreachable behind the `0` that used to be here:
                    // the ring (`sim_core::rewind`), the clamp
                    // (`world.rs`'s `Input` arm), `combat::strike`'s
                    // rewound target scan and `ranged::hitscan`'s.
                    //
                    // **Bound once and used twice**, deliberately: a
                    // counter that called `favour_for` a second time would
                    // be a second derivation agreeing with itself
                    // (`CLAUDE.md`'s naive-rebuild trap, in a counter).
                    //
                    // ⚠ It does **not** follow that the counter proves the
                    // command. Split these two lines and every gate in
                    // `lagcomp_measure.rs` that reads a counter stays
                    // green — measured, not supposed. What holds this line
                    // is the swing gate at the end of that file, which
                    // asserts hp on the far side of `World::tick`.
                    //
                    // Counted inside the command-room check, unlike the
                    // staleness above it: a frame the buffer had no room
                    // for spends no favour, and counting one would report
                    // help nobody received.
                    let favour = stats::favour_for(now, view, c.reported_playout);
                    stats.record_favour(favour);
                    // A throttle tick carries BOTH consumed frames — the
                    // older must still move the body (`InputPair`'s doc:
                    // the client's ring stepped every seq exactly once).
                    self.cmd_buf[n] = match consumed.prev {
                        Some(prev) => Command::InputPair {
                            id: c.id,
                            prev,
                            frame,
                            favour,
                        },
                        None => Command::Input {
                            id: c.id,
                            frame,
                            favour,
                        },
                    };
                    n += 1;
                }
            } else if let Some(ghost) = c.ghost_frame() {
                // The starved tick's stand-in (netcode v2, DECISIONS.md
                // 2026-08-31): feed the sim the last real frame DECAYED
                // (2/3 → 1/3 → 0 on movement) instead of letting it re-run
                // the stale one verbatim at full strength — the overshoot
                // a player felt as a snap on every stop, because the
                // release frame was late and the reconcile dragged them
                // back from a place they never went. As a command, so the
                // WAL carries it and a replay reproduces the ghost bit for
                // bit; favour 0 — no aim was measured, nothing swings (the
                // decay cleared every acting button). One command per
                // player at most, so the input reserve arithmetic above
                // this loop still holds.
                if n < MAX_COMMANDS_PER_TICK {
                    self.cmd_buf[n] = Command::Input {
                        id: c.id,
                        frame: ghost,
                        favour: 0,
                    };
                    n += 1;
                }
            }
        }
        // Pending actions ride after inputs, at most one per client per
        // tick. A full command buffer defers the action to the next tick
        // (limits.rs policy) — it stays in the client's hand.
        //
        // **And no faster than a person** (`pace.rs`): each kind of action
        // keeps its own gap on the sim's tick, and an early one waits in the
        // hand — defer, never drop — so a script hammering a key gets a
        // person's pace and a request still gets its answer.
        //
        // **And not ahead of the hand it reads** (NOW §0rc 2): a throw, a
        // reload or a drink waits for the frames that were already buffered
        // when it reached the hand, so the hotbar slot the client was on
        // when it pressed is the one in force (`ClientNetState::hand_ready`).
        // Asked before the pace, so a wait does not start the kind's clock.
        let now = self.world.tick;
        for slot in 0..MAX_PLAYERS {
            let c = &mut self.clients[slot];
            if !c.connected || n == MAX_COMMANDS_PER_TICK {
                continue;
            }
            if let Some(act) = c.pending_action.take() {
                if !c.hand_ready() || !c.pace.go(&act, now) {
                    c.pending_action = Some(act);
                    continue;
                }
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
                        // A close is the client's own press: its panel is
                        // already shut, so it is owed no word back.
                        if kind == CONT_SELF {
                            c.cont_shown = false;
                        }
                        if kind == sim_core::inventory::CONT_WORLD {
                            Command::OpenWorldCont { id: c.id, cont }
                        } else {
                            continue;
                        }
                    }
                    ActionMsg::Assist { target } => Command::Assist { id: c.id, target },
                    ActionMsg::Treat { slot, target } => Command::Treat {
                        id: c.id,
                        slot,
                        target,
                    },
                    ActionMsg::Give {
                        slot,
                        count,
                        target,
                    } => Command::Give {
                        id: c.id,
                        slot,
                        count,
                        target,
                    },
                    ActionMsg::Craft {
                        recipe,
                        count,
                        skin,
                    } => Command::Craft {
                        id: c.id,
                        recipe,
                        count,
                        skin,
                    },
                    ActionMsg::Reskin { slot, skin } => Command::Reskin {
                        id: c.id,
                        slot,
                        skin,
                    },
                    // Answered by the accept loop before it reaches this
                    // ring (`net.rs` `action_reader_task`); one that got
                    // here anyway asks the sim for nothing.
                    ActionMsg::SkinsRefresh => continue,
                    ActionMsg::CraftCancel { index } => Command::CraftCancel { id: c.id, index },
                    ActionMsg::CraftFastTrack { index, recipe } => Command::CraftFastTrack {
                        id: c.id,
                        index,
                        recipe,
                    },
                    ActionMsg::Place {
                        row,
                        cx,
                        cz,
                        level,
                        loc,
                        freehand,
                        plate,
                    } => Command::Place {
                        id: c.id,
                        row,
                        cx,
                        cz,
                        level,
                        loc,
                        freehand,
                        plate,
                    },
                    ActionMsg::Deploy {
                        row,
                        cx,
                        cz,
                        level,
                        loc,
                        pose,
                    } => Command::PlaceDeploy {
                        id: c.id,
                        row,
                        cx,
                        cz,
                        level,
                        loc,
                        pose,
                    },
                    ActionMsg::Feed { cx, cz, level } => Command::Feed {
                        id: c.id,
                        cx,
                        cz,
                        level,
                    },
                    ActionMsg::TakeStock { cx, cz, level, row } => Command::TakeStock {
                        id: c.id,
                        cx,
                        cz,
                        level,
                        row,
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
                    ActionMsg::Rotate { cx, cz, level, loc } => Command::Rotate {
                        id: c.id,
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
                    ActionMsg::RespawnAt { cx, cz, level } => Command::RespawnAt {
                        id: c.id,
                        cx,
                        cz,
                        level,
                    },
                    ActionMsg::Drop { slot, count } => Command::Drop {
                        id: c.id,
                        slot,
                        count,
                    },
                    ActionMsg::Vend { offer, times } => Command::Vend {
                        id: c.id,
                        offer,
                        times,
                    },
                    ActionMsg::Swipe { door } => Command::Swipe { id: c.id, door },
                    ActionMsg::Arc { op, target, arg } => Command::Arc {
                        id: c.id,
                        op,
                        target,
                        arg,
                    },
                    ActionMsg::Pick { cell } => Command::Pick { id: c.id, cell },
                    ActionMsg::Research { slot } => Command::Research { id: c.id, slot },
                    ActionMsg::Unlock { recipe } => Command::Unlock { id: c.id, recipe },
                    ActionMsg::Drink => Command::Drink { id: c.id },
                    ActionMsg::Reload => Command::Reload { id: c.id },
                    ActionMsg::Unload => Command::Unload { id: c.id },
                    ActionMsg::Respawn { on_bag } => Command::Respawn { id: c.id, on_bag },
                    ActionMsg::RespawnGate => Command::RespawnGate { id: c.id },
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
        self.note_standings();
        // The boards, rebuilt at most every five seconds; whatever moved
        // them since the last look is owed to everyone.
        if self
            .world
            .tick
            .is_multiple_of(5 * sim_core::limits::TICK_HZ as u64)
        {
            self.standings.rebuild();
        }
        if self.standings.gen() != self.standings_gen_seen {
            self.standings_gen_seen = self.standings.gen();
            self.standings_owed = [STANDINGS_ALL; MAX_PLAYERS];
        }
        // This tick's trust rows leave for the log before the next
        // `World::tick` clears them (`trustlog.rs`).
        crate::trustlog::Tap::drain(self, stats);

        for slot in 0..MAX_PLAYERS {
            if self.clients[slot].connected {
                self.update_interest(slot, stats);
            }
        }
        // Spectator seats (`NETCODE.md` §2.3), decided once per tick: a seat
        // whose target left is fed nothing more from here on, and the accept
        // loop closes it on its next sweep. A live seat measures interest
        // from the body it watches, exactly as that player's own connection
        // does — `clients[seat].id` IS the target's id.
        let live = self.live_seats();
        for (i, &l) in live.iter().enumerate() {
            if l {
                self.update_interest(MAX_PLAYERS + i, stats);
            }
        }
        self.fan_out(&live, stats, ops, &mut send);

        self.note_worn();
        // The drips: every connection's own walk over world state, unmirrored.
        for slot in 0..MAX_PLAYERS {
            if self.clients[slot].connected {
                self.drip_client(slot, stats, &mut send);
            }
        }
        for (i, &l) in live.iter().enumerate() {
            if l {
                if self.watching[i].is_some_and(|s| s.catchup) {
                    self.spectator_catchup(i, stats, &mut send);
                }
                self.sync_seat_container(i, stats, &mut send);
                self.drip_client(MAX_PLAYERS + i, stats, &mut send);
            }
        }

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
            // A seat's own snapshot, against its own acked baselines — never
            // a copy of the target's datagram, which deltas against acks the
            // watcher never sent and would not decode after one loss.
            for (i, &l) in live.iter().enumerate() {
                if !l {
                    continue;
                }
                let slot = MAX_PLAYERS + i;
                if let Some(len) = self.encode_snapshot(slot, stats) {
                    ShardStats::bump(&stats.snap_sent);
                    send(Lane::Snapshot, slot, &self.dg_buf[..len]);
                }
            }
        }
    }

    /// Which seats are live this tick (`seat_live`), decided once.
    fn live_seats(&self) -> [bool; MAX_SPECTATORS] {
        let mut live = [false; MAX_SPECTATORS];
        for (i, l) in live.iter_mut().enumerate() {
            *l = self.seat_live(i);
        }
        live
    }

    /// The sim's facts routed — chat and the event arms — with **the mirror**
    /// around them: every message those address to a watched player is
    /// copied to that player's live seats, byte for byte.
    ///
    /// That is what makes a watcher's HUD read what the player's reads —
    /// every own-fact (a gather, a hit, a death, a respawn, the vitals) and
    /// every public event the player was shown, with the player's own
    /// interest filter already applied — without a second routing decision
    /// anywhere in the drain. The drips are NOT mirrored (`tick` runs them
    /// after this, with the bare `send`): they are each connection's own
    /// walk over world state, and a seat runs its own. The table is copied
    /// out so the closure can hold it while the pumps borrow `self`; `u8`
    /// slots, no allocation (wall 2).
    fn fan_out(
        &mut self,
        live: &[bool; MAX_SPECTATORS],
        stats: &ShardStats,
        ops: &mut Ops<'_>,
        send: &mut impl FnMut(Lane, usize, &[u8]) -> bool,
    ) {
        let mut mirror = [NO_SEAT; MAX_SPECTATORS];
        for (i, m) in mirror.iter_mut().enumerate() {
            if let (true, Some(seat)) = (live[i], self.watching[i]) {
                *m = seat.target as u8;
            }
        }
        let watched = mirror.iter().any(|&t| t != NO_SEAT);
        let mut refused = [false; MAX_SPECTATORS];
        {
            let mut mirrored = |lane: Lane, slot: usize, bytes: &[u8]| -> bool {
                let ok = send(lane, slot, bytes);
                if watched && lane == Lane::Event && slot < MAX_PLAYERS {
                    for (i, &t) in mirror.iter().enumerate() {
                        if t as usize == slot && !send(Lane::Event, MAX_PLAYERS + i, bytes) {
                            refused[i] = true;
                        }
                    }
                }
                ok
            };
            self.pump_chat(stats, ops, &mut mirrored);
            self.route_events(stats, &mut mirrored);
        }
        // A seat that lost a mirrored copy (its ring was full) or whose
        // target's facts the sim itself dropped re-walks the world and is
        // re-told the join facts, the same repair a player gets
        // (`ev_resync`). What a resync cannot bring back is a transient —
        // a toast, a hitmarker — which a watcher loses exactly as a player
        // with a full ring would.
        let sim_dropped = self.world.events.dropped > 0;
        for (i, &l) in live.iter().enumerate() {
            if l && (refused[i] || sim_dropped) {
                self.clients[MAX_PLAYERS + i].ev_resync();
                if let Some(seat) = self.watching[i].as_mut() {
                    seat.catchup = true;
                }
                ShardStats::bump(&stats.ev_resyncs);
                ShardStats::bump(&stats.spectate_resyncs);
            }
        }
    }

    /// Tell a freshly seated (or resynced) watcher the own-facts its target
    /// heard once, as events, when it joined: health, vitals and the research
    /// mask. Absolute values off the body, so a copy that arrives beside a
    /// mirrored event says the same thing. A refused push leaves the debt in
    /// place for the next tick; nothing here can be partially owed wrongly,
    /// because every message is the whole truth of its field.
    fn spectator_catchup(
        &mut self,
        i: usize,
        stats: &ShardStats,
        send: &mut impl FnMut(Lane, usize, &[u8]) -> bool,
    ) {
        let slot = MAX_PLAYERS + i;
        // No body yet (the target's join is still queued): owed next tick.
        let Some(w) = self.live_wslot(slot) else {
            return;
        };
        let p = &self.world.players[w];
        let (hp, hp_max, food, water, known) = (p.hp, p.hp_max, p.food, p.water, p.known);
        let (max_food, max_water) = (self.world.survival.max_food, self.world.survival.max_water);
        for k in 0..3 {
            let enc = match k {
                0 => encode_event_health(hp, hp_max, &mut self.ev_buf),
                1 => encode_event_vitals(food, water, max_food, max_water, &mut self.ev_buf),
                _ => encode_event_known(known, &mut self.ev_buf),
            };
            match enc {
                Ok(len) => {
                    if !send(Lane::Event, slot, &self.ev_buf[..len]) {
                        return; // ring full: still owed, whole, next tick
                    }
                    ShardStats::bump(&stats.ev_sent);
                }
                Err(_) => ShardStats::bump(&stats.encode_range_errors),
            }
        }
        if let Some(seat) = self.watching[i].as_mut() {
            seat.catchup = false;
        }
    }

    /// Point seat `i`'s container subscription at whatever its target has
    /// open (NOW §5sp), so the watcher's panel shows the box, bag or crate
    /// the player is looting.
    ///
    /// **The subscription is mirrored, the contents are not.** A seat never
    /// sends `ActionMsg::Container`, so without this its `open_cont_kind`
    /// stays `CONT_SELF` and the container drip (unmirrored, like every
    /// drip: `fan_out`) never feeds it. Copying the target's `ContSync`
    /// bytes would be wrong for the reason snapshots are not copied: the
    /// diff is against the target's shadow, which the seat never had. So
    /// the seat opens the same handle and its own drip, run right after
    /// this, sends it a reset batch off its own shadow. That drip resolves
    /// reach and the lock on the seat's body, which is the target's body
    /// (`clients[seat].id` is the target's id), so a watcher sees exactly
    /// the container its player can move items in and nothing more.
    ///
    /// The player drips run first, so a target whose open the drip just shut
    /// (gone, out of reach, locked) already reads `CONT_SELF` here. A close
    /// is told to the seat directly, since its drip has nothing open to
    /// close; a refused push leaves the seat open and retries next tick.
    ///
    /// The close is owed by what the watcher was **told**, not by the
    /// subscription (`cont_shown`): a refused close followed by a resync
    /// (`ev_resync` drops the subscription silently, which a player answers
    /// by asking again and a seat cannot) would otherwise leave the
    /// subscriptions agreeing and the watcher's panel up, stale, for good.
    fn sync_seat_container(
        &mut self,
        i: usize,
        stats: &ShardStats,
        send: &mut impl FnMut(Lane, usize, &[u8]) -> bool,
    ) {
        let Some(seat) = self.watching[i] else {
            return;
        };
        let slot = MAX_PLAYERS + i;
        let t = &self.clients[seat.target];
        let want = (t.open_cont_kind, t.open_cont_handle);
        let me = &self.clients[slot];
        if want.0 != CONT_SELF {
            if want != (me.open_cont_kind, me.open_cont_handle) {
                self.clients[slot].open_container(want.0, want.1);
            }
            return;
        }
        if me.open_cont_kind == CONT_SELF && !me.cont_shown {
            return;
        }
        match encode_event_cont_sync(CONT_SELF, 0, true, &[], &mut self.ev_buf) {
            Ok(len) => {
                if send(Lane::Event, slot, &self.ev_buf[..len]) {
                    ShardStats::bump(&stats.ev_sent);
                    self.clients[slot].close_container();
                    self.clients[slot].cont_shown = false;
                }
            }
            Err(_) => ShardStats::bump(&stats.encode_range_errors),
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

        // A bare `/wipe` is everybody's too: when the island ends is a
        // posted fact, said only to the asker.
        if let AdminCmd::WipeWhen = cmd {
            if from_slot < MAX_PLAYERS {
                self.wipe_tell[from_slot] = self.world.tick.max(1);
            }
            return;
        }

        // `/top` is everybody's: the standings, to the asker.
        if let AdminCmd::Top = cmd {
            let key = self.key_str(from_slot);
            self.standings.rebuild();
            for line in self.standings.top_lines(key.as_deref()) {
                self.answer(from_slot, line, stats, send);
            }
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
                // Every refusal here is answered too, as the accept loop
                // answers the acts it runs: an admin who hears nothing
                // cannot tell a refused kick from a slow one.
                let ban = matches!(cmd, AdminCmd::Ban { .. });
                let Some(slot) = target_slot(self, id) else {
                    ops.log.push(
                        Record::new(tick, Kind::AdminRefused, verb, who).with(id as i64, 0, 0),
                    );
                    self.answer(from_slot, admin::kicked_line(id, false), stats, send);
                    return;
                };
                let act = if ban {
                    // The wallet, not the id: an id is meaningless after a
                    // reconnect, which is the whole point of a ban.
                    let Some(key) = self.keys[slot] else {
                        ops.log.push(
                            Record::new(tick, Kind::AdminRefused, verb, who).with(id as i64, 0, 0),
                        );
                        self.answer(from_slot, admin::no_wallet_line(id), stats, send);
                        return;
                    };
                    AdminAct::Ban { by: who, id, key }
                } else {
                    AdminAct::Kick { by: who, id }
                };
                let sent = ops.admin_tx.as_mut().is_some_and(|tx| tx.push(act).is_ok());
                if !sent {
                    // The ring to the accept loop is full — the act did
                    // NOT happen, and saying so is the difference between
                    // a bounded queue and a lie.
                    ops.log.push(
                        Record::new(tick, Kind::AdminRefused, verb, who).with(id as i64, 0, 0),
                    );
                    self.answer(from_slot, admin::RING_FULL_LINE.into(), stats, send);
                    return;
                }
                logged = logged.with(id as i64, 0, 0);
            }
            AdminCmd::Unban { prefix } => {
                // The list is the accept loop's, so it decides whether the
                // prefix names one ban and answers on `admin_answers`.
                let act = AdminAct::Unban { by: who, prefix };
                if !ops.admin_tx.as_mut().is_some_and(|tx| tx.push(act).is_ok()) {
                    ops.log.push(
                        Record::new(tick, Kind::AdminRefused, verb, who).note(prefix.as_bytes()),
                    );
                    self.answer(from_slot, admin::RING_FULL_LINE.into(), stats, send);
                    return;
                }
                logged = logged.note(prefix.as_bytes());
            }
            AdminCmd::Say { text } => {
                // The house's line, marked and sent from `from = 0` — an
                // id no player can hold (ids start at 256), so no client
                // has to learn a new message shape to render it.
                let line = admin::server_line(&text);
                self.say_server(None, &line, stats, send);
                logged = logged.note(text.as_bytes());
            }
            AdminCmd::Wipe {
                minutes,
                blueprints,
            } => {
                // The clock says it to everyone on its next poll, so the
                // admin hears the same line the island does.
                self.wipe.admin(self.wipe_now, minutes, blueprints);
                logged = logged.with(minutes as i64, blueprints as i64, 0);
            }
            AdminCmd::WipeCancel => {
                if !self.wipe.cancel(self.wipe_now) {
                    ops.log
                        .push(Record::new(tick, Kind::AdminRefused, verb, who));
                    return;
                }
                self.wipe_say = crate::wipe::chat("wipe cancelled");
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
            AdminCmd::Weather { mode } => {
                if !self.queue(Command::AdminEnv {
                    weather: mode,
                    time_pm: sim_core::weather::KEEP_TIME,
                }) {
                    ops.log
                        .push(Record::new(tick, Kind::AdminRefused, verb, who).with(
                            mode as i64,
                            0,
                            0,
                        ));
                    return;
                }
                logged = logged.with(mode as i64, 0, 0);
                self.answer(from_slot, admin::weather_line(mode), stats, send);
            }
            AdminCmd::Time { frac_pm } => {
                if !self.queue(Command::AdminEnv {
                    weather: sim_core::weather::KEEP_WEATHER,
                    time_pm: frac_pm,
                }) {
                    ops.log
                        .push(Record::new(tick, Kind::AdminRefused, verb, who).with(
                            frac_pm as i64,
                            0,
                            0,
                        ));
                    return;
                }
                logged = logged.with(frac_pm as i64, 0, 0);
                self.answer(from_slot, admin::time_line(frac_pm), stats, send);
            }
            AdminCmd::SaveNow => {
                *ops.save_now = true;
            }
            AdminCmd::Brain => {
                let line = self.brain_answer(who);
                self.answer(from_slot, line, stats, send);
            }
            AdminCmd::Who => {
                let mut line = String::new();
                let mut n = 0;
                for c in self.clients.iter().filter(|c| c.connected) {
                    n += 1;
                    line.push(' ');
                    line.push_str(&c.id.to_string());
                }
                self.answer(from_slot, format!("{n} on:{line}"), stats, send);
            }
            // Handled above, before the allowlist.
            AdminCmd::Bug { .. } | AdminCmd::WipeWhen | AdminCmd::Top => return,
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
        self.pump_wipe(stats, send);
        self.pump_standings(stats, send);
        // How the accept loop's half of an admin verb went, to the admin
        // alone. One who left meanwhile hears nothing, which is counted.
        if let Some(rx) = ops.admin_answers.as_mut() {
            while let Ok(r) = rx.pop() {
                match (0..MAX_PLAYERS)
                    .find(|&s| self.clients[s].connected && self.clients[s].id == r.to)
                {
                    Some(slot) => {
                        let line = crate::admin::server_line(&r.text);
                        self.say_server(Some(slot), &line, stats, send);
                    }
                    None => ShardStats::bump(&stats.chat_undelivered),
                }
            }
        }
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

    /// The event lane, one tick's worth, in the order `tick` runs it: the
    /// sim's facts routed ([`Self::route_events`]), then every player's drips.
    /// `tick` calls the two halves itself, with the spectator mirror between
    /// them; this keeps the whole lane one call for the tests that drive it.
    #[cfg(test)]
    fn pump_events(
        &mut self,
        stats: &ShardStats,
        send: &mut impl FnMut(Lane, usize, &[u8]) -> bool,
    ) {
        self.route_events(stats, send);
        for slot in 0..MAX_PLAYERS {
            if self.clients[slot].connected {
                self.drip_client(slot, stats, send);
            }
        }
    }

    /// The event lane's first half: this tick's sim events routed to their
    /// audiences. A refused push (or a dropped sim event) flags the affected
    /// clients for `ev_resync` — the walk restarts; nothing is silently lost.
    /// The per-client drips — at most one catalog batch, one harvested-set
    /// sync batch, one inventory diff — are the second half, run by `tick`.
    fn route_events(
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
                    let dropped = ev.c.min(u16::MAX as u32) as u16;
                    match encode_event_gather(item, added, dropped, &mut self.ev_buf) {
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
                EV_AMMO => {
                    // Which arrow the archer's bow looses — theirs alone,
                    // the readout's statement (`world.rs`'s role line).
                    let Some(slot) = self.client_slot_of(ev.a) else {
                        continue;
                    };
                    let (weapon, round) = ((ev.b >> 16) as u16, ev.b as u16);
                    match encode_event_ammo(weapon, round, &mut self.ev_buf) {
                        Ok(len) => {
                            if send(Lane::Event, slot, &self.ev_buf[..len]) {
                                ShardStats::bump(&stats.ev_sent);
                            } else {
                                // A level with one statement, the reload's
                                // reason: a lost one would leave the readout
                                // naming the wrong arrow.
                                self.clients[slot].ev_resync();
                                ShardStats::bump(&stats.ev_resyncs);
                            }
                        }
                        Err(_) => ShardStats::bump(&stats.encode_range_errors),
                    }
                }
                EV_RELOAD => {
                    // A fill is heard by whoever is near; the count is not.
                    if ev.c > 0 {
                        self.hear(ev.a, DEED_RELOAD, NO_ITEM);
                    }
                    let Some(slot) = self.client_slot_of(ev.a) else {
                        continue; // shooter left this tick
                    };
                    // b = loaded << 16 | ceiling, c = rounds taken from
                    // the pack, zero on a spend (world.rs's role line).
                    let loaded = sim_core::ranged::mag_loaded(ev.b);
                    let ceiling = sim_core::ranged::mag_ceiling(ev.b);
                    match encode_event_reload(loaded, ceiling, ev.c as u16, &mut self.ev_buf) {
                        Ok(len) => {
                            if send(Lane::Event, slot, &self.ev_buf[..len]) {
                                ShardStats::bump(&stats.ev_sent);
                            } else {
                                // The readout is a level and this event is
                                // its only statement, so a lost one is not
                                // cosmetic the way a toast is — it leaves
                                // the HUD claiming rounds the sim does not
                                // have. The resync is the uniform recovery
                                // and the next shot or fill repairs it
                                // regardless (the event's self-healing
                                // shape, `world.rs`).
                                self.clients[slot].ev_resync();
                                ShardStats::bump(&stats.ev_resyncs);
                            }
                        }
                        Err(_) => ShardStats::bump(&stats.encode_range_errors),
                    }
                }
                EV_VEND | EV_VEND_REFUSED => {
                    let Some(slot) = self.client_slot_of(ev.a) else {
                        continue;
                    };
                    let r = if ev.code == EV_VEND {
                        protocol::encode_event_vend(ev.b as u8, ev.c as u8, &mut self.ev_buf)
                    } else {
                        protocol::encode_event_vend_refused(
                            ev.b as u8,
                            ev.c as u8,
                            &mut self.ev_buf,
                        )
                    };
                    match r {
                        Ok(len) => {
                            if send(Lane::Event, slot, &self.ev_buf[..len]) {
                                ShardStats::bump(&stats.ev_sent);
                            }
                        }
                        Err(_) => ShardStats::bump(&stats.encode_range_errors),
                    }
                }
                EV_SWIPE_REFUSED => {
                    let Some(slot) = self.client_slot_of(ev.a) else {
                        continue;
                    };
                    match protocol::encode_event_swipe_refused(
                        ev.b as u8,
                        ev.c as u8,
                        &mut self.ev_buf,
                    ) {
                        Ok(len) => {
                            if send(Lane::Event, slot, &self.ev_buf[..len]) {
                                ShardStats::bump(&stats.ev_sent);
                            }
                        }
                        Err(_) => ShardStats::bump(&stats.encode_range_errors),
                    }
                }
                // Every client hears a door through the open-door mirror
                // (the drip below), so the swipe itself rides no wire.
                EV_SWIPE => {}
                EV_ARC_REFUSED => {
                    let Some(slot) = self.client_slot_of(ev.a) else {
                        continue;
                    };
                    match protocol::encode_event_arc_refused(
                        ev.b as u8,
                        (ev.c >> 8) as u8,
                        ev.c as u8,
                        &mut self.ev_buf,
                    ) {
                        Ok(len) => {
                            if send(Lane::Event, slot, &self.ev_buf[..len]) {
                                ShardStats::bump(&stats.ev_sent);
                            }
                        }
                        Err(_) => ShardStats::bump(&stats.encode_range_errors),
                    }
                }
                EV_ARC_DID => {
                    // A word or a read: the server answers it with the
                    // words, composed against the world now, to the one who
                    // stood there. A turn has no words; the dials ride
                    // their own drip.
                    let Some(slot) = self.client_slot_of(ev.a) else {
                        continue;
                    };
                    let (target, arg) = ((ev.c >> 8) as u8, ev.c as u8);
                    let composed: Option<(u8, String)> = match ev.b as u8 {
                        sim_core::lore::OP_TALK => self
                            .arc_text
                            .line(target as usize, arg as usize, &self.world.works)
                            .map(|l| (protocol::ARC_SPEAKER, l.to_string())),
                        sim_core::lore::OP_READ => self
                            .arc_text
                            .inscription(
                                target as usize,
                                &self.world.mech_def,
                                self.world.arc.salt,
                                self.world.seed,
                            )
                            .map(|t| (protocol::ARC_INSCRIPTION, t)),
                        _ => None,
                    };
                    let Some((kind, text)) = composed else {
                        continue;
                    };
                    let bytes = &text.as_bytes()[..text.len().min(protocol::ARC_TEXT_BYTES)];
                    match protocol::encode_event_arc_text(
                        kind,
                        target,
                        arg,
                        bytes,
                        &mut self.ev_buf,
                    ) {
                        Ok(len) => {
                            if send(Lane::Event, slot, &self.ev_buf[..len]) {
                                ShardStats::bump(&stats.ev_sent);
                            }
                        }
                        Err(_) => ShardStats::bump(&stats.encode_range_errors),
                    }
                }
                EV_MECH_SOLVED => {
                    match protocol::encode_event_mech_solved(ev.a as u8, ev.b, &mut self.ev_buf) {
                        Ok(len) => {
                            for slot in 0..MAX_PLAYERS {
                                if self.clients[slot].connected
                                    && send(Lane::Event, slot, &self.ev_buf[..len])
                                {
                                    ShardStats::bump(&stats.ev_sent);
                                }
                            }
                        }
                        Err(_) => ShardStats::bump(&stats.encode_range_errors),
                    }
                }
                EV_WORK => {
                    // A work opening, lighting or going to embers changes
                    // the shard for everybody, so everybody hears it. A
                    // client that misses one still converges: the work's
                    // state rides its own per-client drip.
                    match protocol::encode_event_work(
                        ev.a as u8,
                        ev.b as u8,
                        ev.c,
                        &mut self.ev_buf,
                    ) {
                        Ok(len) => {
                            for slot in 0..MAX_PLAYERS {
                                if !self.clients[slot].connected {
                                    continue;
                                }
                                if send(Lane::Event, slot, &self.ev_buf[..len]) {
                                    ShardStats::bump(&stats.ev_sent);
                                }
                            }
                        }
                        Err(_) => ShardStats::bump(&stats.encode_range_errors),
                    }
                }
                EV_RELOAD_REFUSED => {
                    let Some(slot) = self.client_slot_of(ev.a) else {
                        continue; // shooter left this tick
                    };
                    // b = held item << 16 | reason, c = loaded << 16 |
                    // ceiling (world.rs's role line).
                    let item = (ev.b >> 16) as u16;
                    let reason = ev.b as u8;
                    let loaded = sim_core::ranged::mag_loaded(ev.c);
                    let ceiling = sim_core::ranged::mag_ceiling(ev.c);
                    match encode_event_reload_refused(
                        item,
                        reason,
                        loaded,
                        ceiling,
                        &mut self.ev_buf,
                    ) {
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
                        encode_event_craft_done(
                            (ev.b >> 16) as u16,
                            ev.b as u16,
                            ev.c.min(u16::MAX as u32) as u16,
                            &mut self.ev_buf,
                        )
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
                    ShardStats::bump(&stats.pieces_placed);
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
                    let Some(r) = self.world.pieces.find(addr.0, addr.1, addr.2, addr.3) else {
                        // Removed or moved again in this tick: its removal
                        // still follows in this drain. Inventing a record
                        // here would reset the client's whole column plate
                        // to zero even after that transient piece is removed.
                        continue;
                    };
                    let rec = PieceRec {
                        // Rotation and upgrades reuse this upsert. The
                        // store's wire-only band stays zero, so derive
                        // it here exactly as the full-sync walk does.
                        dmg: damage_band(r.hp, piece_hp_max(&self.world.build, r.row)),
                        ..*r
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
                    ShardStats::bump(&stats.deploys_placed);
                    self.owe_bags_if_bag(ev.c, ev.b as u8);
                    // Owner (ev.c) stays sim-side: the wire record is
                    // address + row + open + locked (event.rs). Everything
                    // places closed; a door places locked, which is a
                    // world fact the whole shard sees (who it answers to
                    // is not).
                    // The record as it stands, for its pose (free placement:
                    // where in the cell it went is the sim's to say).
                    let placed = self.world.deploys.find(
                        (ev.a >> 16) as u16,
                        ev.a as u16,
                        (ev.b >> 16) as u8,
                        (ev.b >> 8) as u8,
                    );
                    let rec = DeployRec {
                        cx: (ev.a >> 16) as u16,
                        cz: ev.a as u16,
                        level: (ev.b >> 16) as u8,
                        loc: (ev.b >> 8) as u8,
                        row: ev.b as u8,
                        locked: placed.is_some_and(|d| d.locked),
                        pose: placed.map(|d| d.pose).unwrap_or_default(),
                        // Its hp off the store too (wire v102): the hammer
                        // quotes a repair from it.
                        hp: placed.map_or(0, |d| d.hp),
                        ..DeployRec::default()
                    };
                    let at = interest::cell_cm(rec.cx, rec.cz);
                    match encode_event_deploy_placed(&rec, &mut self.ev_buf) {
                        Ok(len) => {
                            for slot in 0..MAX_PLAYERS {
                                let c = &self.clients[slot];
                                if !c.connected {
                                    continue;
                                }
                                // Class-S interest, `EV_PIECE_PLACED`'s
                                // gate with the deploy walk's own predicate
                                // (owner exempt), against the same anchor:
                                // a deployable placed after a client's walk
                                // finished can only reach it here.
                                if c.piece_anchor_valid
                                    && !interest::owned_in_interest(
                                        c.piece_anchor_cm,
                                        c.id,
                                        ev.c,
                                        at,
                                    )
                                {
                                    ShardStats::bump(&stats.deploy_events_skipped);
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
                    // A meal and a drink are also heard by whoever is near.
                    match ev.code {
                        EV_CONSUMED => self.hear(ev.a, DEED_MEAL, (ev.b >> 16) as u16),
                        EV_DRANK => self.hear(ev.a, DEED_DRINK, NO_ITEM),
                        _ => {}
                    }
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
                EV_HURT => {
                    // The victim's own fact, so the audience is `a` for
                    // `EV_HIT`'s reason read from the other end: this is the
                    // one message in a fight addressed to the person being
                    // hit. Not AOI-filtered and not broadcast — a bystander
                    // flinch is the fan-out `DECISIONS.md` §open
                    // ("attacker-side flinch v0") refuses, and this is one
                    // packet to one slot.
                    let Some(slot) = self.client_slot_of(ev.a) else {
                        continue; // that player left this tick
                    };
                    // The sector is sim-made and the encoder range-checks it
                    // anyway; a widened `HURT_SECTORS` that outgrew the field
                    // lands here as a counted encode error rather than as a
                    // marker pointing the wrong way.
                    match encode_event_hurt(ev.b as u8, ev.c as u16, &mut self.ev_buf) {
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
                        // `c` is packed since v58: the rung above the
                        // damage. Unpacked here rather than on the wire so
                        // the encoder takes a `Part` and cannot be handed
                        // a fourth rung by a caller doing its own shifting.
                        encode_event_hit(
                            ev.b,
                            sim_core::world::hit_part(ev.c),
                            sim_core::world::hit_damage(ev.c),
                            &mut self.ev_buf,
                        )
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
                    // A death is not the only moment this is sent (the
                    // drip also sends it at a join, after a resync and on
                    // the tick one of the owner's bags is placed or taken
                    // down — `owe_bags_if_bag`), but it is the one that
                    // must not wait for the drip, for the ordering above.
                    // Every send is event-driven, never a per-tick scan of
                    // `MAX_DEPLOYS` per client. What that costs is a
                    // `ready` bit that ages while a player sits on the
                    // screen — a cooldown lapses on a clock nothing
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
                    let Some(rec) = self.world.backpacks.find(ev.a) else {
                        continue; // looted or despawned inside the same tick
                    };
                    let bag = WireBag::of(rec);
                    // `interest::bag_in_interest`, unpacked so the store
                    // borrow ends here.
                    let (owner, at) = (rec.owner, interest::body_cm(rec.qx, rec.qz));
                    match encode_event_bag_dropped(&bag, &mut self.ev_buf) {
                        Ok(len) => {
                            for slot in 0..MAX_PLAYERS {
                                let c = &self.clients[slot];
                                if !c.connected {
                                    continue;
                                }
                                // Aimed like a placement, with the bag
                                // walk's predicate: the owner always hears
                                // its own bag drop (the client's `own_bag`
                                // join keys on this very event), and a
                                // stranger 400 m off does not.
                                if c.piece_anchor_valid
                                    && !interest::owned_in_interest(
                                        c.piece_anchor_cm,
                                        c.id,
                                        owner,
                                        at,
                                    )
                                {
                                    ShardStats::bump(&stats.bag_events_skipped);
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
                    //
                    // A wake on a bag spends it: its cooldown starts now,
                    // so the list's `ready` bit is stale from this tick and
                    // the drip owes the owner (and their seats) a fresh one.
                    if ev.b != 0 {
                        self.owe_bags(ev.a);
                    }
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
                // Progress is a diff against authoritative state in drip_client,
                // including zero after a release. A lost event must not leave
                // a participant's countdown suspended forever.
                EV_ASSIST => {}
                EV_WOUNDED | EV_RECOVERED => {
                    // Own-fact, `EV_RESPAWN`'s audience and its posture
                    // (wounded v0): the one body on the ground is the only
                    // one this concerns — everyone else reads the `wounded`
                    // bit off the snapshot — and what it closes or opens
                    // is not a screen but a clock on the HUD. The sim's
                    // fields are `u32`; the clock is under 1,500 and the
                    // odds under 1,000, so the narrowing cannot truncate,
                    // and the encoder refuses odds past the unit anyway.
                    let Some(slot) = self.client_slot_of(ev.a) else {
                        continue; // that player left this tick
                    };
                    let enc = if ev.code == EV_WOUNDED {
                        encode_event_wounded(ev.b as u16, ev.c as u16, &mut self.ev_buf)
                    } else {
                        encode_event_recovered(ev.b as u16, ev.c as u16, &mut self.ev_buf)
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
                    // this pump check is the belt to that suspender, into
                    // the same counter the encoder's own range check uses.
                    // The arm no longer moves any walk state (the cursor
                    // reset it once guarded is gone, below), so all it
                    // spares is the encode; the unit test
                    // `bag_removed_refuses_the_reason_the_sim_cannot_mean`
                    // pins both.
                    if ev.b > BAG_GONE_MAX {
                        ShardStats::bump(&stats.encode_range_errors);
                        continue;
                    }
                    // Same posture as a piece/deploy removal: broadcast to
                    // everyone, unfiltered (an absence is the one thing no
                    // walk re-derives), and **no walk restart** — the bag
                    // walk reads tail-down, so the entry the swap-remove
                    // moved is one it already sent (`drip_client`).
                    match encode_event_bag_removed(ev.a, ev.b as u8, &mut self.ev_buf) {
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
                EV_DOOR => {
                    // A door's state is a world fact: broadcast, not
                    // AOI'd — though the placement that put it there is
                    // now (`NOW.md` §0n1 item 2). A client keeps every
                    // record it was ever told about (nothing un-subscribes
                    // it), so one that walked away still holds this door
                    // and must hear it swing. Aiming this needs the re-arm
                    // argument, and it is `NOW.md` §0fan's. A client that
                    // misses one re-derives it from the deploy walk — the
                    // sync record carries the bit.
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
                    // *state*, on records a client keeps once told — the
                    // deploy walk is aimed now, but nothing un-subscribes,
                    // so a client holds every record it was ever in range
                    // of. Filtering a state change onto a record somebody
                    // keeps is how a door stays shut on one screen forever.
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
                    let loc = (ev.b >> 8) as u8;
                    let lit = ev.b & 1 != 0;
                    match encode_event_oven(cx, cz, level, loc, lit, ev.c, &mut self.ev_buf) {
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
                EV_GROW => {
                    // A planter's beds (crops v1): broadcast, `EV_OVEN`'s
                    // posture. A client that misses one reads the byte off
                    // the deploy walk on its resync.
                    let (cx, cz) = ((ev.a >> 16) as u16, ev.a as u16);
                    let level = (ev.b >> 16) as u8;
                    let loc = (ev.b >> 8) as u8;
                    let stages = ev.b as u8;
                    match protocol::encode_event_planter(
                        cx,
                        cz,
                        level,
                        loc,
                        stages,
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
                    // the longest reach in `content/weapons.toml`, the
                    // crossbow's metal arrow lobbed (`v²/g`, 151 m), is
                    // inside an `AOI_ENTER_CM` of 176 m, so a shot from
                    // outside a client's interest cannot land a projectile
                    // within 25 m of it on flat ground.
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
                EV_SENTRY_LOCK => {
                    // A town sentry's lock-on beep, to the clients that have
                    // the gun in interest — the target always among them,
                    // since it stands in the town under the gun. Not a
                    // whole-population fan-in (`BODY_BROADCAST_ARMS`): four
                    // guns lock at most once a look each.
                    match protocol::encode_event_sentry_lock(ev.a, ev.b, &mut self.ev_buf) {
                        Ok(len) => {
                            for slot in 0..MAX_PLAYERS {
                                if !self.clients[slot].connected {
                                    continue;
                                }
                                if !self.roster_event_visible(slot, ev.a) {
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
                EV_HOWL => {
                    // A pack call, to the clients drawing the animal that
                    // made it: the roster's own interest set (`m_interest`),
                    // which is the audience a snapshot of that animal has.
                    // Before a client's interest has settled it hears every
                    // howl, the fail-open every broadcast arm here takes.
                    let Some(s) = mob::slot_of_id(ev.a) else {
                        ShardStats::bump(&stats.encode_range_errors);
                        continue;
                    };
                    match protocol::encode_event_howl(ev.a, &mut self.ev_buf) {
                        Ok(len) => {
                            for slot in 0..MAX_PLAYERS {
                                if !self.clients[slot].connected {
                                    continue;
                                }
                                if self.interest_settled(slot) && !self.clients[slot].m_interest[s]
                                {
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
                    let (surf, kind, qx) = sim_core::world::impact_parts(ev.a);
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
                    match encode_event_impact(qx, qy, qz, surf, kind, &mut self.ev_buf) {
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
                    // The keypad's beep is the exception, and it carries
                    // none of the grant: whoever stands near hears a code
                    // go in, as they would see the door open after it.
                    if grant != sim_core::lock::GRANT_NONE {
                        self.hear(ev.c, DEED_KEYPAD, NO_ITEM);
                    }
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
                    ShardStats::bump(&stats.struct_hits);
                    // A structure still standing after a raid swing: the
                    // address, what it took, what is left. Broadcast like
                    // a placement — the wall is a world fact, and anyone
                    // in earshot of the base should see it come apart. No
                    // sync walk moves: nothing left the store.
                    let (cx, cz) = ((ev.a >> 16) as u16, ev.a as u16);
                    let deploy = ev.b & STRUCT_DEPLOY_BIT != 0;
                    let (level, loc, row) = ((ev.b >> 16) as u8, (ev.b >> 8) as u8, ev.b as u8);
                    let (damage, left) = ((ev.c >> 16) as u16, ev.c as u16);
                    // The killing blow: the sim announces it (the raid gates
                    // count it), and the removal right behind it is what the
                    // wire says. The encoder refuses a zero-hp hit by
                    // design, so sending it only counted a range error per
                    // structure destroyed.
                    if left == 0 {
                        continue;
                    }
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
                    ShardStats::bump(&stats.charges_armed);
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
                EV_FIRE => {
                    // Broadcast, unfiltered: a fire hurts whoever walks into
                    // it, so every client should know where one burns before
                    // it walks there — and until when, so it can put it out.
                    let (ticks, qx) = sim_core::world::fire_parts(ev.a);
                    let (qz, qy) = (ev.b as i32, ev.c as i32);
                    let until = (self.world.tick as u32).wrapping_add(u32::from(ticks));
                    match encode_event_fire(qx, qy, qz, until, &mut self.ev_buf) {
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
                    if !piece {
                        self.owe_bags_if_bag(ev.c, ev.b as u8);
                    }
                    let (cx, cz) = ((ev.a >> 16) as u16, ev.a as u16);
                    let (level, loc) = ((ev.b >> 16) as u8, (ev.b >> 8) as u8);
                    match encode_event_removed(piece, cx, cz, level, loc, &mut self.ev_buf) {
                        Ok(len) => {
                            for slot in 0..MAX_PLAYERS {
                                if !self.clients[slot].connected {
                                    continue;
                                }
                                // **No walk restarts here any more**, piece
                                // or deployable (`NOW.md` §0n1 item 2). A
                                // restart is correct under the store's
                                // swap-remove and its *cost* is unbounded:
                                // a full walk is `store_len / batch` ticks,
                                // and removals arriving faster than that
                                // walk a client back to zero indefinitely —
                                // a raid or a decay sweep clears that bar
                                // easily, and the client-side symptom, a
                                // world that never finishes arriving, does
                                // not read as a network problem
                                // (`reference/NETWORK.md` §9.2.1). Both
                                // walks read their store from the tail down
                                // instead, where the entry a swap-remove
                                // moves is always one already sent, so a
                                // removal costs them nothing and each
                                // clamps its own cursor where it reads it
                                // (`drip_client` carries the argument).
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
                    let Some(hi) = self
                        .world
                        .deploys
                        .hearths()
                        .iter()
                        .position(|h| h.cx == cx && h.cz == cz && h.level == level)
                    else {
                        continue; // hearth decayed in the same tick
                    };
                    let hr = self.world.deploys.hearths()[hi];
                    // Only the crew reads the stock and the bill. To anyone
                    // else they are the base's decay clock, which a stranger
                    // would otherwise read for the price of one plank fed
                    // (the feed itself still lands: a gift is not a grief).
                    if !hr.crew.contains(ev.a) {
                        continue;
                    }
                    if let Some(len) = self.encode_stock(hi, stats) {
                        if send(Lane::Event, slot, &self.ev_buf[..len]) {
                            ShardStats::bump(&stats.ev_sent);
                            self.clients[slot].stock_hearth = Some((cx, cz, level));
                        } else {
                            // The next feed re-announces; cosmetic.
                            self.clients[slot].ev_resync();
                            ShardStats::bump(&stats.ev_resyncs);
                        }
                    }
                }
                EV_SLOT_HARVESTED | EV_SLOT_RESPAWNED | EV_STUMP_GRUBBED => {
                    let cx = (ev.a >> 16) as u16;
                    let cz = ev.a as u16;
                    // A tree comes back as a sapling (tree growth v0): `c`
                    // says so and `b` is the tick it is grown by.
                    let encoded = if ev.code == EV_SLOT_HARVESTED {
                        encode_event_slot_change(true, cx, cz, &mut self.ev_buf)
                    } else if ev.code == EV_STUMP_GRUBBED {
                        protocol::encode_event_stump_grubbed(cx, cz, &mut self.ev_buf)
                    } else {
                        protocol::encode_event_slot_respawned(
                            cx,
                            cz,
                            (ev.c != 0).then_some(ev.b),
                            &mut self.ev_buf,
                        )
                    };
                    match encoded {
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
        // A bow drawn: the aim going down with a draw weapon in hand, the
        // edge the drawer's own client creaks on (`viewmodel`'s draw).
        for w in 0..MAX_PLAYERS {
            let (id, drawing) = {
                let p = &self.world.players[w];
                let bow = self
                    .world
                    .combat
                    .held_ranged(sim_core::combat::held_item(p))
                    .is_some_and(|d| d.draw_ticks > 0 && !d.hitscan);
                let aiming = p.frame.buttons & sim_core::input::BTN_AIM != 0;
                (p.id, p.active && !p.dead && !p.wounded && aiming && bow)
            };
            if drawing && !self.drawing[w] {
                self.hear(id, DEED_DRAW, NO_ITEM);
            }
            self.drawing[w] = drawing;
        }
        self.flush_heard(stats, send);
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
        // The per-connection drips run in `tick`, after this returns — they
        // used to close this function, and moved so the spectator mirror
        // (which wraps this function's `send`) forwards the sim's facts and
        // never a connection's own walk state (`NETCODE.md` §2.3).
    }

    /// Queue a deed the clients near `body` should hear. One per body per
    /// flush — the bound the flush's fan-in is counted on — so a second in
    /// the same tick is dropped.
    fn hear(&mut self, body: u32, deed: u8, item: u16) {
        let n = self.heard_len;
        if n == self.heard.len() || self.heard[..n].iter().any(|h| h.0 == body) {
            return;
        }
        self.heard[n] = (body, deed, item);
        self.heard_len = n + 1;
    }

    /// Send this tick's heard deeds: each to the clients drawing that body
    /// (the class-D filter, so a raid fans in at most `AOI_RANK_EXIT` a
    /// tick, the third arm `BODY_BROADCAST_ARMS` counts) and standing within
    /// `ACT_HEAR_CM` of it. Never back to the hands that did it: they hear
    /// their own deed off its own-fact.
    fn flush_heard(
        &mut self,
        stats: &ShardStats,
        send: &mut impl FnMut(Lane, usize, &[u8]) -> bool,
    ) {
        for i in 0..self.heard_len {
            let (body, deed, item) = self.heard[i];
            let Some(w) = Self::world_slot_of(&self.world, body) else {
                continue; // gone this tick
            };
            let at = self.world.players[w].body;
            let len = match encode_event_heard(body, deed, item, &mut self.ev_buf) {
                Ok(len) => len,
                Err(_) => {
                    ShardStats::bump(&stats.encode_range_errors);
                    continue;
                }
            };
            for slot in 0..MAX_PLAYERS {
                if !self.clients[slot].connected || self.clients[slot].id == body {
                    continue;
                }
                if !self.body_event_visible(slot, body, Some(w)) {
                    ShardStats::bump(&stats.ev_interest_skipped);
                    continue;
                }
                let Some(to) = self.live_wslot(slot) else {
                    continue;
                };
                let p = self.world.players[to].body;
                let dx = (p.qx - at.qx) as i64 * 3;
                let dz = (p.qz - at.qz) as i64 * 3;
                if dx * dx + dz * dz > ACT_HEAR_CM * ACT_HEAR_CM {
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
        self.heard_len = 0;
    }

    /// Owe player `owner` their own-bag list (`SUB_BAGS`, NOW §0die 2) when
    /// deploy row `row` is a bag — one of theirs was just placed or taken
    /// down. Their own connection and every seat watching them (a seat's
    /// `id` is its target's, `NETCODE.md` §2.3) hear the whole list from
    /// the next drip, so a tick that moves several of one owner's bags
    /// still sends it once. The row is a `u8` off the event and the table
    /// is `MAX_DEPLOY_DEFS` long, hence `get`.
    fn owe_bags_if_bag(&mut self, owner: u32, row: u8) {
        let bag = self
            .world
            .deploy
            .defs
            .get(row as usize)
            .is_some_and(|d| d.arch == ARCH_BAG);
        if bag {
            self.owe_bags(owner);
        }
    }

    /// Owe player `owner`'s connection and every seat watching them their
    /// own-bag list on the next drip: a bag of theirs moved or was spent.
    fn owe_bags(&mut self, owner: u32) {
        for c in self.clients.iter_mut() {
            if c.connected && c.id == owner {
                c.bags_owed = true;
            }
        }
    }

    /// Resolve which connection slot player `id` belongs to.
    fn client_slot_of(&self, id: u32) -> Option<usize> {
        (0..MAX_PLAYERS).find(|&s| self.clients[s].connected && self.clients[s].id == id)
    }

    /// Hearth `hi`'s stock ack into `ev_buf`: each row's item, what the
    /// hearth holds and what a day charges (wire v89) — the sweep's own
    /// arithmetic over the claim cache the tick just refreshed (upkeep v2's
    /// readout). A walk of the piece store with an O(1) answer for the
    /// base's own pieces, asked per feed press and by the crew vital's
    /// push, which [`crew_vital_due`] holds to one connection a tick.
    /// `None` (counted) when the rows would not encode.
    fn encode_stock(&mut self, hi: usize, stats: &ShardStats) -> Option<usize> {
        let hr = self.world.deploys.hearths()[hi];
        let bill = sim_core::upkeep::bill(
            &self.world.deploy,
            &self.world.build,
            &self.world.pieces,
            &self.world.deploys,
            hi,
        );
        let mut rows = [(0u16, 0u32, 0u32); HEARTH_STOCK_ROWS];
        let n = (self.world.deploy.mat_count as usize).min(HEARTH_STOCK_ROWS);
        for (m, row) in rows.iter_mut().enumerate().take(n) {
            *row = (self.world.deploy.mats[m], hr.stock[m], bill[m]);
        }
        match encode_event_stock(hr.cx, hr.cz, hr.level, &rows[..n], &mut self.ev_buf) {
            Ok(len) => Some(len),
            Err(_) => {
                ShardStats::bump(&stats.encode_range_errors);
                None
            }
        }
    }

    /// One owed standings board to `slot`, the lowest first: the board's
    /// top rows and where this player stands on it.
    /// False when the ring refused it: the rest of the drip waits too.
    fn drip_standing(
        &mut self,
        slot: usize,
        stats: &ShardStats,
        send: &mut impl FnMut(Lane, usize, &[u8]) -> bool,
    ) -> bool {
        // A spectator seat watches a body; it is ranked on nothing.
        let owed = self.standings_owed.get(slot).copied().unwrap_or(0);
        if owed == 0 {
            return true;
        }
        let b = owed.trailing_zeros() as u8;
        let key = self.key_str(slot);
        let board = self.standings.board(b);
        let mine = key.as_deref().and_then(|k| board.of(k));
        let wipe = if b == crate::standings::BOARD_LAST {
            self.standings.hall.last_wipe
        } else {
            self.standings.wipe
        };
        let mut wire = protocol::StandingBoard {
            board: b,
            wipe: wipe.min(u16::MAX as u32) as u16,
            ranked: board.ranked.min(u16::MAX as u32) as u16,
            my_rank: mine.map_or(0, |m| m.0.min(u16::MAX as u32) as u16),
            my_score: mine.map_or(0, |m| m.1.min(u32::MAX as u64) as u32),
            ..protocol::StandingBoard::EMPTY
        };
        for (name, score) in &board.top {
            wire.push(name, (*score).min(u32::MAX as u64) as u32);
        }
        // The purse: what each place pays, what this wallet would take now,
        // and its minutes against the minimum (`standings::Prizes`). On the
        // last-wipe board, what it won.
        let clamp = |v: u64| v.min(u32::MAX as u64) as u32;
        let prizes = &self.standings.prizes;
        if b == crate::standings::BOARD_LAST {
            let hall = &self.standings.hall;
            if let Some(won) = key.as_deref().and_then(|k| hall.last_paid.get(k)) {
                wire.set_purse(&hall.last_ticker, &[]);
                wire.my_prize = clamp(*won);
            }
        } else if prizes.armed() {
            let places: Vec<u32> = prizes.places[b as usize]
                .iter()
                .map(|&a| clamp(a))
                .collect();
            wire.set_purse(&prizes.ticker, &places);
            wire.my_prize = key
                .as_deref()
                .map_or(0, |k| clamp(self.standings.would_win(b, k)));
            wire.min_minutes = prizes.min_minutes.min(u16::MAX as u32) as u16;
            wire.my_minutes = key
                .as_deref()
                .and_then(|k| self.standings.row_of(k))
                .map_or(0, |r| r.played.min(u16::MAX as u32) as u16);
        }
        match protocol::encode_event_standing(&wire, &mut self.ev_buf) {
            Ok(len) => {
                if !send(Lane::Event, slot, &self.ev_buf[..len]) {
                    return false;
                }
                self.standings_owed[slot] &= !(1 << b);
                ShardStats::bump(&stats.ev_sent);
            }
            Err(_) => {
                self.standings_owed[slot] &= !(1 << b);
                ShardStats::bump(&stats.encode_range_errors);
            }
        }
        true
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
        if !self.drip_standing(slot, stats, send) {
            return;
        }
        let assist = self.live_wslot(slot).map_or((0, 0, 0), |wslot| {
            let p = &self.world.players[wslot];
            if p.assist_by != 0 {
                (p.assist_by, p.id, p.assist_ticks)
            } else if p.assist_target != 0 {
                Self::world_slot_of(&self.world, p.assist_target)
                    .filter(|&target| self.world.players[target].assist_by == p.id)
                    .map_or((0, 0, 0), |target| {
                        let q = &self.world.players[target];
                        (p.id, q.id, q.assist_ticks)
                    })
            } else {
                (0, 0, 0)
            }
        });
        if assist != self.clients[slot].last_assist {
            match encode_event_assist(assist.0, assist.1, assist.2, &mut self.ev_buf) {
                Ok(len) => {
                    if send(Lane::Event, slot, &self.ev_buf[..len]) {
                        self.clients[slot].last_assist = assist;
                        ShardStats::bump(&stats.ev_sent);
                    } else {
                        return;
                    }
                }
                Err(_) => ShardStats::bump(&stats.encode_range_errors),
            }
        }

        // Wet and cold (weather v0): the owner's per-cent readout when it
        // moves. A client with no live body (dead, not yet joined) owes
        // nothing and keeps its last reading until one exists.
        if let Some(wslot) = self.live_wslot(slot) {
            let expo = sim_core::exposure::readout(
                &self.world.survival.exposure,
                &self.world.players[wslot],
            );
            if self.world.survival.exposure.armed() && Some(expo) != self.clients[slot].last_expo {
                match protocol::encode_event_exposure(expo.0, expo.1, expo.2, &mut self.ev_buf) {
                    Ok(len) => {
                        if send(Lane::Event, slot, &self.ev_buf[..len]) {
                            self.clients[slot].last_expo = Some(expo);
                            ShardStats::bump(&stats.ev_sent);
                        } else {
                            return;
                        }
                    }
                    Err(_) => ShardStats::bump(&stats.encode_range_errors),
                }
            }
        }

        // Until when the owner is hostile (wire v89): the SAFE ZONE chip's
        // other half. `tick + hostile` holds still while the timer runs
        // down, so this sends on an attack (or the timer's end), not per
        // tick.
        if let Some(wslot) = self.live_wslot(slot) {
            let left = self.world.players[wslot].hostile as u32;
            let until = if left == 0 {
                0
            } else {
                (self.world.tick as u32).wrapping_add(left)
            };
            if self.clients[slot].last_hostile != Some(until) {
                match protocol::encode_event_hostile(until, &mut self.ev_buf) {
                    Ok(len) => {
                        if send(Lane::Event, slot, &self.ev_buf[..len]) {
                            self.clients[slot].last_hostile = Some(until);
                            ShardStats::bump(&stats.ev_sent);
                        } else {
                            return;
                        }
                    }
                    Err(_) => ShardStats::bump(&stats.encode_range_errors),
                }
            }
        }

        // THE GATE's respawn point (wire v92), for the death screen: 0 while
        // locked, else the tick it is next free (1 when it never rested).
        if let Some(wslot) = self.live_wslot(slot) {
            let p = &self.world.players[wslot];
            let ready_at = if p.gate_spawn {
                (p.gate_spawn_at as u32).max(1)
            } else {
                0
            };
            if self.clients[slot].last_gate_spawn != Some(ready_at) {
                match protocol::encode_event_gate_spawn(ready_at, &mut self.ev_buf) {
                    Ok(len) => {
                        if send(Lane::Event, slot, &self.ev_buf[..len]) {
                            self.clients[slot].last_gate_spawn = Some(ready_at);
                            ShardStats::bump(&stats.ev_sent);
                        } else {
                            return;
                        }
                    }
                    Err(_) => ShardStats::bump(&stats.encode_range_errors),
                }
            }
        }

        // The sky and the clock (weather v0): the whole record whenever it
        // differs from what this client last heard, which is also how a
        // fresh join and a resync hear it.
        let env = self.world.env;
        if self.clients[slot].last_env != Some(env) {
            match protocol::encode_event_env(&env, &mut self.ev_buf) {
                Ok(len) => {
                    if send(Lane::Event, slot, &self.ev_buf[..len]) {
                        self.clients[slot].last_env = Some(env);
                        ShardStats::bump(&stats.ev_sent);
                    } else {
                        return;
                    }
                }
                Err(_) => ShardStats::bump(&stats.encode_range_errors),
            }
        }

        // The fires burning (wire v94), every one, to a fresh join and after
        // a resync: the broadcast as each was lit reached only who was here.
        // All again on a full lane; the client keeps one per point.
        if self.clients[slot].fires_owed {
            for f in self.world.fires.entries() {
                match encode_event_fire(f.qx, f.qy, f.qz, f.until as u32, &mut self.ev_buf) {
                    Ok(len) => {
                        if send(Lane::Event, slot, &self.ev_buf[..len]) {
                            ShardStats::bump(&stats.ev_sent);
                        } else {
                            return;
                        }
                    }
                    Err(_) => ShardStats::bump(&stats.encode_range_errors),
                }
            }
            self.clients[slot].fires_owed = false;
        }

        // The owner's own bags, whole (`SUB_BAGS`, NOW §0die 2), whenever
        // they are owed: a fresh join, a resync, the tick one of them was
        // placed or taken down (`owe_bags_if_bag`), and a wake that spent
        // one (`EV_RESPAWN`, so its `ready` bit is fresh). Not only at a death
        // any more — the map tags your beds off this list while you live,
        // and a list from your last death misses every bed since. A seat
        // runs it too, against its target's id, as it runs every drip. One
        // store scan, only on a tick that owes it.
        if self.clients[slot].bags_owed {
            let mut anchors = [BagAnchor::default(); BAG_CAP];
            let n = self.world.deploys.own_bags(
                &self.world.deploy,
                self.clients[slot].id,
                self.world.tick,
                &mut anchors,
            );
            match encode_event_bags(&anchors[..n], &mut self.ev_buf) {
                Ok(len) => {
                    if send(Lane::Event, slot, &self.ev_buf[..len]) {
                        ShardStats::bump(&stats.ev_sent);
                    } else {
                        return;
                    }
                }
                Err(_) => ShardStats::bump(&stats.encode_range_errors),
            }
            self.clients[slot].bags_owed = false;
        }

        // The crew HUD vital (NOW §0up 3): a body standing in a claim it is
        // crew of is re-told that hearth's stock and bill every
        // `CREW_VITAL_TICKS`, the feed ack's own message (`EventMsg::Stock`),
        // so the HUD can say UPKEEP 1D 4H / BASE DECAYING without a press.
        // Phased by slot, so a full shard pays at most one bill a tick. A
        // full ring skips one push; the next is ten seconds off. A seat runs
        // it against its target's body and id, as it runs every drip.
        if crew_vital_due(self.world.tick, slot) {
            let hi = self.live_wslot(slot).and_then(|w| {
                let b = self.world.players[w].body;
                let (x, z) = (
                    b.qx as f32 * sim_core::movement::POS_XZ_Q,
                    b.qz as f32 * sim_core::movement::POS_XZ_Q,
                );
                // Where two of your claims overlap, the hearth the client
                // already holds answers while it still covers you.
                let c = &self.clients[slot];
                self.world
                    .deploys
                    .crew_hearth_at(x, z, c.id, c.stock_hearth)
            });
            let stock = hi.and_then(|hi| self.encode_stock(hi, stats).map(|len| (hi, len)));
            if let Some((hi, len)) = stock {
                if send(Lane::Event, slot, &self.ev_buf[..len]) {
                    ShardStats::bump(&stats.ev_sent);
                    let h = self.world.deploys.hearths()[hi];
                    self.clients[slot].stock_hearth = Some((h.cx, h.cz, h.level));
                } else {
                    return;
                }
            }
        }

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
        // Then one description line a tick, skipping items with none, once
        // the names are all there (`SUB_ITEM_DESC`).
        let c = &self.clients[slot];
        let n = self.catalog.count as usize;
        if c.catalog_cursor >= n && c.desc_cursor < n {
            let mut i = c.desc_cursor;
            while i < n && self.item_descs.get(i).is_empty() {
                i += 1;
            }
            if i < n {
                match protocol::encode_event_item_desc(
                    i as u16,
                    self.item_descs.get(i),
                    &mut self.ev_buf,
                ) {
                    Ok(len) => {
                        if !send(Lane::Event, slot, &self.ev_buf[..len]) {
                            return;
                        }
                        ShardStats::bump(&stats.ev_sent);
                    }
                    Err(_) => ShardStats::bump(&stats.encode_range_errors),
                }
            }
            self.clients[slot].desc_cursor = i + 1;
        }

        // Skin rows (skins v0), the item catalog's drip shape: the store
        // screen and every skinned item's look read them.
        let c = &self.clients[slot];
        let sk = &self.skin_catalog;
        if sk.count > 0 && c.skins_cursor < sk.count as usize {
            match protocol::encode_event_skins(sk, c.skins_cursor, &mut self.ev_buf) {
                Ok((len, took)) => {
                    if send(Lane::Event, slot, &self.ev_buf[..len]) {
                        ShardStats::bump(&stats.ev_sent);
                        self.clients[slot].skins_cursor += took;
                    } else {
                        return;
                    }
                }
                Err(_) => ShardStats::bump(&stats.encode_range_errors),
            }
        }

        // The town's vendor offers (wire v85), the skins' drip shape.
        let c = &self.clients[slot];
        let vc = &self.world.vend;
        if vc.count > 0 && c.vend_cursor < vc.count as usize {
            let names: [&[u8]; sim_core::limits::MAX_VENDORS] = core::array::from_fn(|i| {
                self.vendor_names
                    .get(i)
                    .map(|n| n.as_bytes())
                    .unwrap_or(&[])
            });
            match protocol::encode_event_vend_offers(vc, &names, c.vend_cursor, &mut self.ev_buf) {
                Ok((len, took)) => {
                    if send(Lane::Event, slot, &self.ev_buf[..len]) {
                        ShardStats::bump(&stats.ev_sent);
                        self.clients[slot].vend_cursor += took;
                    } else {
                        return;
                    }
                }
                Err(_) => ShardStats::bump(&stats.encode_range_errors),
            }
        }

        // The works (wire v95): each row once, the vendor drip's shape, then
        // each work's state whenever what this player sees of it moves —
        // the quota, the tank, their own share, the unlocks held.
        let wc = &self.world.works_def;
        let k = self.clients[slot].work_cursor;
        if k < wc.count as usize {
            let def = wc.defs[k];
            let names = &self.unlock_names;
            let un = |u: u8| -> &[u8] {
                match u {
                    0 => &[],
                    u => names.get(u as usize - 1).map_or(&[], |n| n.as_bytes()),
                }
            };
            let name = self.work_names.get(k).map_or(&[][..], |n| n.as_bytes());
            match protocol::encode_event_work_def(
                wc,
                k,
                name,
                un(def.floor),
                un(def.ceiling),
                &mut self.ev_buf,
            ) {
                Ok(len) => {
                    if send(Lane::Event, slot, &self.ev_buf[..len]) {
                        ShardStats::bump(&stats.ev_sent);
                        self.clients[slot].work_cursor += 1;
                    } else {
                        return;
                    }
                }
                Err(_) => {
                    ShardStats::bump(&stats.encode_range_errors);
                    self.clients[slot].work_cursor += 1;
                }
            }
        } else {
            let id = self.clients[slot].id;
            for k in 0..wc.count as usize {
                let w = self.world.works.w[k];
                let seen = crate::client::WorkSeen {
                    state: w.state,
                    fuel: w.fuel,
                    got: w.got,
                    mine: w.points_of(id),
                    unlocks: self.world.works.unlocks,
                };
                if self.clients[slot].last_works[k] == Some(seen) {
                    continue;
                }
                match protocol::encode_event_work_state(
                    k as u8,
                    &w,
                    wc.defs[k].n_inputs,
                    seen.mine,
                    seen.unlocks,
                    &mut self.ev_buf,
                ) {
                    Ok(len) => {
                        if send(Lane::Event, slot, &self.ev_buf[..len]) {
                            ShardStats::bump(&stats.ev_sent);
                            self.clients[slot].last_works[k] = Some(seen);
                        } else {
                            return;
                        }
                    }
                    Err(_) => ShardStats::bump(&stats.encode_range_errors),
                }
            }
        }

        // The arc's alphabet, once (wire v96).
        if !self.clients[slot].alphabet_sent && !self.arc_text.alphabet.is_empty() {
            match protocol::encode_event_alphabet(
                self.arc_text.alphabet.as_bytes(),
                &mut self.ev_buf,
            ) {
                Ok(len) => {
                    if send(Lane::Event, slot, &self.ev_buf[..len]) {
                        ShardStats::bump(&stats.ev_sent);
                        self.clients[slot].alphabet_sent = true;
                    } else {
                        return;
                    }
                }
                Err(_) => {
                    ShardStats::bump(&stats.encode_range_errors);
                    self.clients[slot].alphabet_sent = true;
                }
            }
        }
        // Where every speaker, stone and lock stands (wire v96), one a tick:
        // speakers, then inscriptions, then mechanisms.
        let (lc, mc) = (&self.world.lore_def, &self.world.mech_def);
        let (ns, ni, nm) = (
            lc.n_speakers as usize,
            lc.n_inscriptions as usize,
            mc.count as usize,
        );
        let at = self.clients[slot].arc_cursor;
        if at < ns + ni + nm {
            let r = if at < ns {
                let sp = &self.arc_text.speakers[at];
                let titles: Vec<&[u8]> = sp.topics.iter().map(|t| t.title.as_bytes()).collect();
                protocol::encode_event_arc_place(
                    protocol::ARC_SPEAKER,
                    at as u8,
                    ns as u8,
                    &lc.speakers[at].spot,
                    sp.name.as_bytes(),
                    &titles,
                    0,
                    0,
                    &mut self.ev_buf,
                )
            } else if at < ns + ni {
                let k = at - ns;
                protocol::encode_event_arc_place(
                    protocol::ARC_INSCRIPTION,
                    k as u8,
                    ni as u8,
                    &lc.inscriptions[k].spot,
                    &[],
                    &[],
                    0,
                    0,
                    &mut self.ev_buf,
                )
            } else {
                let k = at - ns - ni;
                let def = &mc.defs[k];
                protocol::encode_event_arc_place(
                    protocol::ARC_MECH,
                    k as u8,
                    nm as u8,
                    &def.spot,
                    self.arc_text.mech_names[k].as_bytes(),
                    &[],
                    def.dials,
                    def.values,
                    &mut self.ev_buf,
                )
            };
            match r {
                Ok(len) => {
                    if send(Lane::Event, slot, &self.ev_buf[..len]) {
                        ShardStats::bump(&stats.ev_sent);
                        self.clients[slot].arc_cursor += 1;
                    } else {
                        return;
                    }
                }
                Err(_) => {
                    ShardStats::bump(&stats.encode_range_errors);
                    self.clients[slot].arc_cursor += 1;
                }
            }
        }
        // Each mechanism's dials, whenever they move for anyone.
        for k in 0..self.world.mech_def.count as usize {
            let def = self.world.mech_def.defs[k];
            let m = self.world.arc.mechs[k];
            let seen = (m.dials, m.rest_until > self.world.tick);
            if self.clients[slot].last_dials[k] == Some(seen) {
                continue;
            }
            match protocol::encode_event_arc_dials(
                k as u8,
                &m.dials[..def.dials as usize],
                seen.1,
                &mut self.ev_buf,
            ) {
                Ok(len) => {
                    if send(Lane::Event, slot, &self.ev_buf[..len]) {
                        ShardStats::bump(&stats.ev_sent);
                        self.clients[slot].last_dials[k] = Some(seen);
                    } else {
                        return;
                    }
                }
                Err(_) => ShardStats::bump(&stats.encode_range_errors),
            }
        }
        // The glyphs this player reads, whenever the mask moves.
        if let Some(wslot) = Self::world_slot_of(&self.world, self.clients[slot].id) {
            let mask = self.world.players[wslot].glyphs;
            if self.clients[slot].last_glyphs != Some(mask) {
                match protocol::encode_event_glyphs(mask, &mut self.ev_buf) {
                    Ok(len) => {
                        if send(Lane::Event, slot, &self.ev_buf[..len]) {
                            ShardStats::bump(&stats.ev_sent);
                            self.clients[slot].last_glyphs = Some(mask);
                        } else {
                            return;
                        }
                    }
                    Err(_) => ShardStats::bump(&stats.encode_range_errors),
                }
            }
        }

        // The ziggurat's open doors (wire v86), whenever they move.
        let bits = self.world.card_door_bits as u8;
        if self.clients[slot].last_doors != Some(bits) {
            match protocol::encode_event_card_doors(bits, &mut self.ev_buf) {
                Ok(len) => {
                    if send(Lane::Event, slot, &self.ev_buf[..len]) {
                        ShardStats::bump(&stats.ev_sent);
                        self.clients[slot].last_doors = Some(bits);
                    } else {
                        return;
                    }
                }
                Err(_) => ShardStats::bump(&stats.encode_range_errors),
            }
        }

        // What this player owns, whenever the sim's copy moves (a join's
        // first read, a refresh, a new session's reset to none).
        if let Some(wslot) = Self::world_slot_of(&self.world, self.clients[slot].id) {
            let owned = self.world.players[wslot].skins;
            if self.clients[slot].last_skins != Some(owned) {
                match protocol::encode_event_skins_owned(&owned, &mut self.ev_buf) {
                    Ok(len) => {
                        if send(Lane::Event, slot, &self.ev_buf[..len]) {
                            self.clients[slot].last_skins = Some(owned);
                            ShardStats::bump(&stats.ev_sent);
                        } else {
                            return;
                        }
                    }
                    Err(_) => ShardStats::bump(&stats.encode_range_errors),
                }
            }
        }

        // Who everyone is (v85): one owed tag per tick, lowest slot first.
        // The bit clears only when the push is accepted, so a full ring
        // delays a name and never loses one.
        let owed = self.clients[slot].tags_owed;
        if owed != 0 {
            let i = owed.trailing_zeros() as usize;
            let bit = 1u128 << i;
            let row = self.tags.get(i).copied().unwrap_or_default();
            if row.id == 0 {
                self.clients[slot].tags_owed &= !bit;
            } else {
                match protocol::encode_event_tag(
                    row.id,
                    &row.address,
                    &row.name,
                    row.pic,
                    &mut self.ev_buf,
                ) {
                    Ok(len) => {
                        if send(Lane::Event, slot, &self.ev_buf[..len]) {
                            self.clients[slot].tags_owed &= !bit;
                            ShardStats::bump(&stats.ev_sent);
                        } else {
                            return;
                        }
                    }
                    Err(_) => {
                        self.clients[slot].tags_owed &= !bit;
                        ShardStats::bump(&stats.encode_range_errors);
                    }
                }
            }
        }

        // What every body wears (v98): one owed per tick, `tags_owed`'s
        // shape. An empty world slot, or a body that has never worn
        // anything, has nothing to say.
        let owed = self.clients[slot].worn_owed;
        if owed != 0 {
            let i = owed.trailing_zeros() as usize;
            let bit = 1u128 << i;
            let (id, items, skins) = self.worn_seen[i];
            if id == 0 || self.worn_dressed & bit == 0 {
                self.clients[slot].worn_owed &= !bit;
            } else {
                match protocol::encode_event_worn(id, &items, &skins, &mut self.ev_buf) {
                    Ok(len) => {
                        if send(Lane::Event, slot, &self.ev_buf[..len]) {
                            self.clients[slot].worn_owed &= !bit;
                            ShardStats::bump(&stats.ev_sent);
                        } else {
                            return;
                        }
                    }
                    Err(_) => {
                        self.clients[slot].worn_owed &= !bit;
                        ShardStats::bump(&stats.encode_range_errors);
                    }
                }
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
        //
        // The same window carries the regrowing trees (tree growth v0) in a
        // second message: a late joiner has to know a sapling is a sapling,
        // and when it will be grown, or it would walk through what it sees
        // as a full tree's trunk. Sent after the harvested batch, so a
        // reset has cleared both of the client's sets before any arrive.
        let c = &self.clients[slot];
        let lives = &self.world.slot_lives;
        if c.sync_reset || c.sync_cursor < lives.len() {
            let mut cells = [(0u16, 0u16); SLOT_SYNC_BATCH];
            let mut grubbed = 0u64;
            let mut grows = [(0u16, 0u16, 0u32); protocol::GROW_SYNC_BATCH];
            let mut n_cells = 0usize;
            let mut n_grows = 0usize;
            let mut scanned = 0usize;
            let entries = lives.entries();
            while c.sync_cursor + scanned < entries.len()
                && scanned < SYNC_SCAN_PER_TICK
                && n_cells < SLOT_SYNC_BATCH
                && n_grows < protocol::GROW_SYNC_BATCH
            {
                let e = entries[c.sync_cursor + scanned];
                if e.respawn_at != 0 {
                    cells[n_cells] = (e.cx, e.cz);
                    if e.hits == sim_core::gather::STUMP_GRUBBED {
                        grubbed |= 1 << n_cells;
                    }
                    n_cells += 1;
                } else if e.grown_at != 0 {
                    grows[n_grows] = (e.cx, e.cz, e.grown_at as u32);
                    n_grows += 1;
                }
                scanned += 1;
            }
            // A window whose second send is refused goes again whole next
            // tick; the client's sets take a repeat as a no-op.
            let reset = c.sync_reset;
            let mut said = true;
            if reset || n_cells > 0 {
                match encode_event_slot_sync(reset, &cells[..n_cells], grubbed, &mut self.ev_buf) {
                    Ok(len) => {
                        if !send(Lane::Event, slot, &self.ev_buf[..len]) {
                            return;
                        }
                        ShardStats::bump(&stats.ev_sent);
                        self.clients[slot].sync_reset = false;
                    }
                    Err(_) => {
                        ShardStats::bump(&stats.encode_range_errors);
                        said = false;
                    }
                }
            }
            if said && n_grows > 0 {
                match protocol::encode_event_slot_grow_sync(&grows[..n_grows], &mut self.ev_buf) {
                    Ok(len) => {
                        if !send(Lane::Event, slot, &self.ev_buf[..len]) {
                            return;
                        }
                        ShardStats::bump(&stats.ev_sent);
                    }
                    Err(_) => {
                        ShardStats::bump(&stats.encode_range_errors);
                        said = false;
                    }
                }
            }
            // Past this window: its harvested and growing entries are said,
            // and the standing-damage ones had nothing to say.
            if said {
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
        // above buys. The deployable and backpack walks below make it too
        // (`NOW.md` §0n1 item 2), on the same seam: every runtime insert
        // into either store pushes its placement event (`deploy::
        // place_deploy`, `Backpacks::stand_up`), and the only other inserts
        // are boot-time (`stand_authored`, `restore`), before anyone is
        // connected.
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
                //
                // The deployable and backpack walks share the anchor, so
                // they re-arm with it: a record the old anchor skipped may
                // be in range of the new one.
                c.piece_anchor_cm = here;
                c.piece_sync_cursor = len;
                c.deploy_sync_cursor = self.world.deploys.len();
                c.bag_sync_cursor = self.world.backpacks.len();
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

        // Deploy-def rows, same drip shape (the deploy menu's data), and
        // the fire's warmth reach the HUD confirms a fire with (wire v102).
        let c = &self.clients[slot];
        let dc = &self.world.deploy;
        let heat = self.world.survival.exposure.heat_radius_cm;
        if dc.def_count > 0 && c.deploy_defs_cursor < dc.def_count as usize {
            match encode_event_deploy_defs(dc, heat, c.deploy_defs_cursor, &mut self.ev_buf) {
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

        // Placed-deployable walk (join sync / resync): the piece walk
        // above, line for line, over `world.deploys` (`NOW.md` §0n1 item
        // 2). Tail-down, so a decay removal mid-walk costs it nothing and
        // the cursor only needs clamping; aimed from the same anchor, so a
        // joiner pays for the deployables it landed beside. The one
        // difference is the owner's exemption (`interest::
        // owned_in_interest`): a client's own beds and hearths are always
        // owed, because its map draws them wherever it stands.
        let c = &self.clients[slot];
        let deploys = self.world.deploys.entries();
        let anchor = c.piece_anchor_cm;
        let viewer = c.id;
        let owed = if c.deploy_sync_reset {
            deploys.len()
        } else {
            c.deploy_sync_cursor.min(deploys.len())
        };
        if c.piece_anchor_valid && (c.deploy_sync_reset || owed > 0) {
            let window = PIECE_SCAN_BATCH.min(owed);
            // The band, filled at the boundary — `PieceRec::dmg`'s note.
            let mut wire = [DeployRec::default(); DEPLOY_SYNC_BATCH];
            let mut n = 0usize;
            let mut scanned = 0usize;
            for src in deploys[owed - window..owed].iter().rev() {
                scanned += 1;
                if !interest::deploy_in_interest(anchor, viewer, src) {
                    continue;
                }
                let dst = &mut wire[n];
                *dst = *src;
                dst.dmg = damage_band(src.hp, deploy_hp_max(&self.world.deploy, src.row));
                // A planter's beds (crops v1), off the oven row's mirror.
                dst.grow = self
                    .world
                    .deploys
                    .oven_index(sim_core::deploy::box_key(
                        src.cx, src.cz, src.level, src.loc,
                    ))
                    .map(|i| self.world.deploys.oven_states()[i])
                    .filter(|o| o.arch == sim_core::deploy::ARCH_PLANTER)
                    .map_or(0, |o| o.bank as u8);
                n += 1;
                if n == DEPLOY_SYNC_BATCH {
                    break;
                }
            }
            wire[..n].reverse();
            let rest = owed - scanned;
            let done = |c: &mut ClientNetState| {
                c.deploy_sync_reset = false;
                c.deploy_sync_cursor = rest;
                ShardStats::add(&stats.deploy_sync_skipped, (scanned - n) as u64);
                if rest == 0 {
                    ShardStats::bump(&stats.deploy_walk_completes);
                }
            };
            if n > 0 || c.deploy_sync_reset {
                match encode_event_deploy_sync(c.deploy_sync_reset, &wire[..n], &mut self.ev_buf) {
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
                done(&mut self.clients[slot]);
            }
        }

        // Standing-backpack walk (join sync / resync): the deployable walk
        // again, over `world.backpacks` — tail-down, aimed, the owner
        // exempt. The exemption is what keeps a death bag on its owner's
        // map after a respawn far away and a resync (`ui/map.rs` finds
        // `own_bag` by id in this mirror). A carcass bag's owner is a
        // roster id, which no client is.
        let c = &self.clients[slot];
        let bags = self.world.backpacks.entries();
        let owed = if c.bag_sync_reset {
            bags.len()
        } else {
            c.bag_sync_cursor.min(bags.len())
        };
        if c.piece_anchor_valid && (c.bag_sync_reset || owed > 0) {
            let window = PIECE_SCAN_BATCH.min(owed);
            let mut batch = [WireBag::default(); BAG_SYNC_BATCH];
            let mut n = 0usize;
            let mut scanned = 0usize;
            for b in bags[owed - window..owed].iter().rev() {
                scanned += 1;
                if !interest::bag_in_interest(anchor, viewer, b) {
                    continue;
                }
                batch[n] = WireBag::of(b);
                n += 1;
                if n == BAG_SYNC_BATCH {
                    break;
                }
            }
            batch[..n].reverse();
            let rest = owed - scanned;
            let done = |c: &mut ClientNetState| {
                c.bag_sync_reset = false;
                c.bag_sync_cursor = rest;
                ShardStats::add(&stats.bag_sync_skipped, (scanned - n) as u64);
                if rest == 0 {
                    ShardStats::bump(&stats.bag_walk_completes);
                }
            };
            if n > 0 || c.bag_sync_reset {
                match encode_event_bag_sync(c.bag_sync_reset, &batch[..n], &mut self.ev_buf) {
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
                done(&mut self.clients[slot]);
            }
        }

        // Loose-stack walk (ground items v0), drip-fed and read upward.
        //
        // **Keyed on the store's fingerprint rather than on an event**,
        // which is the one thing this walk does differently and the
        // reason is in `grounditem.rs`: a scatter and a despawn are both
        // silent, because litter does not deserve the reliable lane.
        // `(next_id, len)` is complete — every insert bumps `next_id`, so
        // a barrel that dropped one stack while a player took another
        // still reads as a change where a length would not.
        //
        // **Growth alone does not restart it.** Every insert appends and
        // bumps `next_id`, so when both moved by the same amount nothing
        // was removed, every record already sent is where it was, and the
        // walk carries on to the new tail. Landed arrows are loose stacks
        // (`spent.rs`): a fight lands them tick after tick, and a restart
        // per landing would never reach the newest ones.
        let c = &self.clients[slot];
        let fp = (
            self.world.ground_items.next_id(),
            self.world.ground_items.len(),
        );
        if c.gitem_seen != fp {
            let (seen_id, seen_len) = c.gitem_seen;
            let grew = fp.1 > seen_len && fp.0.wrapping_sub(seen_id) as usize == fp.1 - seen_len;
            let c = &mut self.clients[slot];
            if !grew {
                c.gitem_sync_cursor = 0;
                c.gitem_sync_reset = true;
            }
            c.gitem_seen = fp;
        }
        let c = &self.clients[slot];
        let n_gitems = self.world.ground_items.len();
        if c.gitem_sync_reset || c.gitem_sync_cursor < n_gitems {
            let at = c.gitem_sync_cursor.min(n_gitems);
            let n = GITEM_SYNC_BATCH.min(n_gitems - at);
            let mut batch = [WireGItem::default(); GITEM_SYNC_BATCH];
            for (i, g) in self.world.ground_items.entries()[at..][..n]
                .iter()
                .enumerate()
            {
                batch[i] = WireGItem::of(g);
            }
            match encode_event_gitem_sync(c.gitem_sync_reset, &batch[..n], &mut self.ev_buf) {
                Ok(len) => {
                    if send(Lane::Event, slot, &self.ev_buf[..len]) {
                        ShardStats::bump(&stats.ev_sent);
                        let c = &mut self.clients[slot];
                        c.gitem_sync_reset = false;
                        c.gitem_sync_cursor = at + n;
                    } else {
                        return;
                    }
                }
                Err(_) => ShardStats::bump(&stats.encode_range_errors),
            }
        }

        // Arrows-in-bodies walk (wire v94): the spent store's entries that
        // have a host, drip-fed like the loose stacks. Keyed on the store's
        // change stamp: an arrow going into a body or coming out of one
        // restarts it, and nothing else moves what it sends — an arrow in a
        // body is drawn off the body, so its riding along costs no bytes.
        let stamp = self.world.spent.stamp();
        if self.clients[slot].lodged_seen != stamp {
            let c = &mut self.clients[slot];
            c.lodged_seen = stamp;
            c.lodged_sync_cursor = 0;
            c.lodged_sync_reset = true;
        }
        let c = &self.clients[slot];
        let spent = self.world.spent.entries();
        if c.lodged_sync_reset || c.lodged_sync_cursor < spent.len() {
            let mut batch = [WireLodged::default(); LODGED_SYNC_BATCH];
            let (mut n, mut at) = (0, c.lodged_sync_cursor.min(spent.len()));
            while at < spent.len() && n < LODGED_SYNC_BATCH {
                if spent[at].host != 0 {
                    batch[n] = WireLodged::of(&spent[at]);
                    n += 1;
                }
                at += 1;
            }
            if n > 0 || c.lodged_sync_reset {
                match encode_event_lodged_sync(c.lodged_sync_reset, &batch[..n], &mut self.ev_buf) {
                    Ok(len) => {
                        if send(Lane::Event, slot, &self.ev_buf[..len]) {
                            ShardStats::bump(&stats.ev_sent);
                            let c = &mut self.clients[slot];
                            c.lodged_sync_reset = false;
                            c.lodged_sync_cursor = at;
                        } else {
                            return;
                        }
                    }
                    Err(_) => ShardStats::bump(&stats.encode_range_errors),
                }
            } else {
                self.clients[slot].lodged_sync_cursor = at;
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
            // A box shut by its lock, rather than gone or out of reach —
            // the one close the player is owed a reason for.
            let mut lock_refused = false;
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
                            // The box's own slot, the move path's address
                            // byte for byte: free placement puts several
                            // bodies in one cell, and slot 0's lock is not
                            // this box's. An oven carries no lock
                            // (`lockable`) and passes as bare.
                            let b = self.world.deploys.boxes()[i];
                            let ok = self
                                .world
                                .deploys
                                .lock_passes(b.cx, b.cz, b.level, b.loc, p.id);
                            lock_refused = !ok;
                            ok
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
                None => {
                    // The lock says why, ahead of the close: `E` on a box
                    // that does not know you was a panel that shut with no
                    // word, where a drag into it already said this.
                    if lock_refused {
                        if let Ok(len) = encode_event_deploy_refused(
                            sim_core::deploy::REFUSE_D_OWNER as u8,
                            &mut self.ev_buf,
                        ) {
                            if send(Lane::Event, slot, &self.ev_buf[..len]) {
                                ShardStats::bump(&stats.ev_sent);
                            }
                        }
                    }
                    match encode_event_cont_sync(CONT_SELF, 0, true, &[], &mut self.ev_buf) {
                        Ok(len) => {
                            if send(Lane::Event, slot, &self.ev_buf[..len]) {
                                ShardStats::bump(&stats.ev_sent);
                                self.clients[slot].close_container();
                                self.clients[slot].cont_shown = false;
                            } else {
                                return;
                            }
                        }
                        Err(_) => ShardStats::bump(&stats.encode_range_errors),
                    }
                }
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
                                    let opened = c.open_cont_reset;
                                    c.open_cont_reset = false;
                                    c.last_cont = now;
                                    c.cont_shown = true;
                                    // The lid, heard by whoever is near —
                                    // once, off the player's own open: a
                                    // seat mirroring it (`sync_seat_container`)
                                    // opened nothing in the world.
                                    if opened && slot < MAX_PLAYERS {
                                        let id = c.id;
                                        let deed = if kind == CONT_BAG {
                                            DEED_OPEN_BAG
                                        } else {
                                            DEED_OPEN_BOX
                                        };
                                        self.hear(id, deed, NO_ITEM);
                                    }
                                    // Whether what was opened is burning,
                                    // told to the hand that opened it: the
                                    // lit bit is broadcast only when it
                                    // changes (`EV_OVEN`), so a fire lit
                                    // before this client arrived would
                                    // otherwise offer TURN ON while it
                                    // burns. Absolute, so a repeat is free.
                                    // `i` indexes the box store only for
                                    // a box: a bag's or a crate's index is
                                    // into its own store.
                                    let st = (opened && kind == CONT_BOX)
                                        .then(|| self.world.deploys.oven_states().get(i).copied())
                                        .flatten()
                                        .filter(|st| {
                                            st.is_converter()
                                                || st.arch == sim_core::deploy::ARCH_RESEARCH
                                        });
                                    if let Some(st) = st {
                                        let b = self.world.deploys.boxes()[i];
                                        if let Ok(len) = encode_event_oven(
                                            b.cx,
                                            b.cz,
                                            b.level,
                                            b.loc,
                                            st.lit,
                                            0,
                                            &mut self.ev_buf,
                                        ) {
                                            if send(Lane::Event, slot, &self.ev_buf[..len]) {
                                                ShardStats::bump(&stats.ev_sent);
                                            }
                                        }
                                    }
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
    /// - **A roster slot** (the heli's gun): the roster's own interest,
    ///   `m_interest`, under the same unsettled rule below.
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
        // A roster slot's fact — the heli's gun (`heli.rs`), which no world
        // slot names — is filtered by the roster's own interest, or a burst
        // would reach the whole island.
        if let Some(m) = mob::slot_of_id(subject) {
            return !self.interest_settled(slot) || c.m_interest[m];
        }
        let Some(w) = subject_wslot else {
            return true;
        };
        !self.interest_settled(slot) || c.interest[w]
    }

    /// Does connection `slot` have roster slot `subject` (a tagged mob id)
    /// in interest? `body_event_visible`'s roster half, for a fact that is
    /// not a body broadcast — a sentry's lock.
    fn roster_event_visible(&self, slot: usize, subject: u32) -> bool {
        match mob::slot_of_id(subject) {
            Some(m) => !self.interest_settled(slot) || self.clients[slot].m_interest[m],
            None => false,
        }
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

    /// One player as the wire sees them.
    ///
    /// **`gc` is here for the hand and nothing else** (wire v56). `held`
    /// and `lit` are the only two fields on this record a client cannot
    /// derive for a body that is not its own: `SUB_INV` carries condition
    /// for your own bag, the `BTN_LIGHT` latch is your own input, and the
    /// content row that says a torch burns at all lives in `GatherContent`.
    /// So the server resolves both once per record, on the same values the
    /// sim ran on — the quantize-both-sides law applied to a flag.
    fn wire_entity(p: &Player, gc: &GatherContent) -> EntityState {
        EntityState {
            id: p.id,
            qx: p.body.qx,
            qy: p.body.qy,
            qz: p.body.qz,
            qvy: p.body.qvy,
            grounded: p.body.grounded,
            sleeping: p.sleeping,
            dead: p.dead,
            wounded: p.wounded,
            // The stance every sim rule reads (v83), so what is drawn is
            // what a shot is tested against.
            crouched: p.crouched(),
            yaw: p.frame.yaw,
            pitch: p.frame.pitch,
            // A downed body has dropped what it held (wounded v0 — the
            // reference's rule, `reference/WOUNDED.md` §2.3), so the hand
            // reads empty and the flame is out, whatever the hotbar says.
            // The sim skips the arm and the torch for it (`tick`'s wounded
            // branch); this is the wire agreeing.
            held: if p.wounded { None } else { Self::held_of(p) },
            lit: !p.wounded && sim_core::light::is_lit(p, gc),
            // The held item's skin (skins v0), under `held`'s rule: an
            // empty or dropped hand wears nothing.
            held_skin: if p.wounded || Self::held_of(p).is_none() {
                0
            } else {
                p.inv[p.frame.sel as usize].skin
            },
        }
    }

    /// What is in this body's selected hotbar slot, or nothing.
    ///
    /// **Bounded here rather than trusted**, `light::is_lit`'s reason
    /// exactly: `sel` arrives from a datagram on one path and from a WAL
    /// on another, `world::apply` clamps the wire's three bits, and this
    /// is the second of the two places that must hold whichever one fed
    /// it. An empty stack is an empty hand — `count == 0` is a slot with
    /// a stale item id in it, and drawing the tool a player just spent
    /// would be the inventory lying about itself, one body over.
    ///
    /// A corpse keeps its hand. `dead` and `sleeping` are their own bits
    /// on this record and it is the client that decides what a body in
    /// either state is drawn holding — the wire says what is true, and
    /// hiding a fact here would put a render policy in the sim's answer.
    fn held_of(p: &Player) -> Option<u16> {
        let sel = p.frame.sel as usize;
        if sel >= HOTBAR_SLOTS {
            return None;
        }
        let s = p.inv[sel];
        if s.count == 0 || s.item == NO_ITEM {
            return None;
        }
        Some(s.item)
    }

    /// One animal as the same record. Four of the ten fields have no
    /// meaning here and each is answered rather than left to a default:
    /// `pitch` is zero because nothing about a pig looks up or down;
    /// `sleeping` is the brain's Sleep state — the animal lying down, which
    /// is what the client draws for it (`render/mobs.rs`) and what stops it
    /// being extrapolated, since nothing moves a sleeper. Dormancy is not
    /// that fact and does not set it: a dormant animal is merely unthought
    /// about, standing where it stopped. `dead` is false because
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
            sleeping: m.state == sim_core::brain::AiState::Sleep,
            dead: false,
            wounded: false,
            // The hunt (v99): an animal never crouches, so the bit says it
            // is after someone, which is what the client's voice reads.
            crouched: matches!(
                m.state,
                sim_core::brain::AiState::Chase
                    | sim_core::brain::AiState::Attack
                    | sim_core::brain::AiState::Orbit
            ),
            yaw: m.yaw,
            pitch: 0,
            // Six of twelve now. A pig has no hotbar, so the hand is
            // empty and the flame is off — and unlike the four above,
            // these two would be *wrong* rather than merely meaningless
            // if they were ever filled: `held` is an index into the item
            // catalog and a mob has no inventory to index it from.
            held: None,
            lit: false,
            held_skin: 0,
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
            // Both cached at consume time earlier this tick, so the header
            // reports the state the tick actually ran under (netcode v2).
            buffered_depth: c.depth_report(),
            repeat_count: c.repeat_report(),
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
        let own = Self::wire_entity(&self.world.players[c.own_wslot], &self.world.gather);
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
                Self::wire_entity(&self.world.players[w], &self.world.gather)
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
    use sim_core::build::LOC_PLANE;

    const SEED: u64 = 0x5B_F06E;
    const PLAYER: u32 = 7;

    /// The crew vital's stagger (NOW §0up 3): every slot a push can go to
    /// is due exactly once a period, and never on the same tick as another
    /// — so a full shard standing in its bases pays one bill a tick at most.
    #[test]
    fn crew_vital_pushes_are_one_slot_a_tick() {
        let period = sim_core::deploy::CREW_VITAL_TICKS;
        let slots = MAX_PLAYERS + MAX_SPECTATORS;
        let mut due_ticks = vec![0u32; period as usize];
        for slot in 0..slots {
            let due: Vec<u64> = (5 * period..6 * period)
                .filter(|&t| crew_vital_due(t, slot))
                .collect();
            assert_eq!(due.len(), 1, "slot {slot} is due once a period");
            due_ticks[(due[0] % period) as usize] += 1;
        }
        assert!(
            due_ticks.iter().all(|&n| n <= 1),
            "two slots share a push tick"
        );
    }

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

    /// The standings are tallied per wallet off the sim's own events: a
    /// keyed player's deposit lights the fixture work, and the WORKS board
    /// credits what they gave and the deed, under their platform name; the
    /// drip then carries the board to them with their rank on it.
    #[test]
    fn a_deposit_lands_on_the_givers_wallet_and_rides_the_wire() {
        use sim_core::gather::{GatherContent, ItemStack};
        use sim_core::works::{WorksContent, ARG_ALL, OP_DEPOSIT, WORK_LIT, WORK_OPEN};
        let stats = ShardStats::default();
        let mut core = Box::new(ShardCore::new(SEED));
        core.world.gather = GatherContent::probe_fixture();
        core.world.works_def = WorksContent::probe_fixture();
        core.work_names = vec!["THE KILN".into()];
        core.standings.rules = crate::standings::Rules {
            given_pct: 200,
            lit_points: 300,
            worth: vec![100, 250],
            ..crate::standings::Rules::default()
        };
        core.standings.prizes = crate::standings::Prizes {
            ticker: "ORBS".into(),
            places: [vec![], vec![200, 100], vec![], vec![]],
            min_minutes: 0,
            agents: false,
        };
        let key = PlayerKey::new(format!("0x{:040x}", 0xa1).as_bytes()).unwrap();
        let wallet = core::str::from_utf8(key.as_bytes()).unwrap().to_string();
        assert!(core.connect_as(0, 256, Some(key), None).is_some());
        core.tag_join(0, 256, Some(&key));
        core.tick_bare(&stats, |_, _, _| true);
        core.set_face(0, 256, protocol::Name::new("Ash").unwrap(), 0);
        let w = ShardCore::world_slot_of(&core.world, 256).expect("seated");
        let spot = core.world.works_def.defs[0].spot;
        let (x, _, z) = sim_core::spot::world(&core.world.haven, &spot).expect("a town");
        let p = &mut core.world.players[w];
        p.body = sim_core::movement::Body::at(SEED, &core.world.haven, x, z);
        let stack = |item, count| ItemStack {
            item,
            count,
            cond: 0,
            skin: 0,
        };
        p.inv[0] = stack(0, 6);
        p.inv[1] = stack(1, 2);
        for _ in 0..sim_core::works::WORKS_PERIOD_TICKS {
            if core.world.works.w[0].state == WORK_OPEN {
                break;
            }
            core.tick_bare(&stats, |_, _, _| true);
        }
        assert!(core.queue(Command::Arc {
            id: 256,
            op: OP_DEPOSIT,
            target: 0,
            arg: ARG_ALL,
        }));
        let mut boards = Vec::new();
        // Past one rebuild (every five seconds) and its drip.
        for _ in 0..6 * sim_core::limits::TICK_HZ as u64 {
            core.tick_bare(&stats, |lane, _, bytes| {
                if lane == Lane::Event {
                    if let Ok(EventMsg::Standing(b)) = decode_event(bytes) {
                        boards.push(b);
                    }
                }
                true
            });
        }
        assert_eq!(core.world.works.w[0].state, WORK_LIT);
        let row = core.standings.row_of(&wallet).expect("the giver is ranked");
        // 6 × 1.00 + 2 × 2.50, doubled; and the lighting deed at act 2.
        assert_eq!((row.given, row.deeds), (2_200, 60_000));
        assert_eq!(row.label, "Ash");
        assert_eq!(core.standings.deeds()[0].1, "lit THE KILN");
        core.standings.rebuild();
        assert_eq!(
            core.standings
                .board(crate::standings::BOARD_WORKS)
                .of(&wallet),
            Some((1, 622))
        );
        let works = boards
            .iter()
            .rev()
            .find(|b| b.board == crate::standings::BOARD_WORKS)
            .expect("the works board was dripped");
        assert_eq!((works.my_rank, works.my_score, works.n), (1, 622, 1));
        assert_eq!(works.name(0), "Ash");
        // The purse rides with it: first place pays 200, and that is what
        // this wallet would take now.
        assert_eq!(works.ticker(), "ORBS");
        assert_eq!(&works.prizes[..works.n_prizes as usize], &[200, 100]);
        assert_eq!(works.my_prize, 200);
    }

    /// Tags (v85): each player learns who every tagged player is, a name
    /// read for a slot's previous tenant is dropped, and a late joiner is
    /// caught up one tag per tick.
    #[test]
    fn every_player_learns_every_tagged_player_and_a_late_joiner_catches_up() {
        let stats = ShardStats::default();
        let mut core = Box::new(ShardCore::new(SEED));
        let key = |n: u8| PlayerKey::new(format!("0x{:040x}", n).as_bytes()).unwrap();
        for (slot, id, n) in [(0usize, 256u32, 0xa1u8), (1, 257, 0xb2)] {
            assert!(core.connect_as(slot, id, Some(key(n)), None).is_some());
            core.tag_join(slot, id, Some(&key(n)));
        }
        core.set_face(0, 256, protocol::Name::new("Ash").unwrap(), 7);
        core.set_face(1, 999, protocol::Name::new("Nope").unwrap(), 1);
        let tags = |core: &mut ShardCore| {
            let mut got = Vec::new();
            core.tick_bare(&stats, |lane, slot, bytes| {
                if lane == Lane::Event {
                    if let Ok(EventMsg::Tag { id, name, pic, .. }) = decode_event(bytes) {
                        got.push((slot, id, name.as_str().to_string(), pic));
                    }
                }
                true
            });
            got
        };
        let mut seen = Vec::new();
        for _ in 0..4 {
            seen.extend(tags(&mut core));
        }
        for slot in [0, 1] {
            assert!(seen.contains(&(slot, 256, "Ash".into(), 7)), "{seen:?}");
            assert!(seen.contains(&(slot, 257, String::new(), 0)), "{seen:?}");
        }
        assert!(!seen.iter().any(|t| t.2 == "Nope"), "a stale id is dropped");

        assert!(core.connect_as(2, 258, Some(key(0xc3)), None).is_some());
        core.tag_join(2, 258, Some(&key(0xc3)));
        let first = tags(&mut core);
        assert_eq!(first.iter().filter(|t| t.0 == 2).count(), 1, "one per tick");
        let mut late: Vec<u32> = first.iter().filter(|t| t.0 == 2).map(|t| t.1).collect();
        for _ in 0..3 {
            late.extend(tags(&mut core).iter().filter(|t| t.0 == 2).map(|t| t.1));
        }
        late.sort_unstable();
        assert_eq!(late, vec![256, 257, 258]);
    }

    /// **A client eating every tick eats about once a second** (`pace.rs`).
    /// The lane is thirty actions a second and a mouthful is not: the ask
    /// that comes early waits in the hand, so a stack is not gone in one.
    #[test]
    fn a_client_eating_every_tick_eats_about_once_a_second() {
        let stats = ShardStats::default();
        let mut core = Box::new(ShardCore::new(SEED));
        core.world.survival = sim_core::survival::SurvivalContent::probe_fixture();
        assert!(core.connect(0, PLAYER));
        core.tick_bare(&stats, |_, _, _| true);
        let w = core.live_wslot(0).unwrap();
        // The fixture's item 0 heals, so a full body never refuses it.
        core.world.players[w].inv[0] = ItemStack {
            item: 0,
            count: 60,
            cond: 0,
            skin: 0,
        };
        for _ in 0..90 {
            if core.wants_action(0) {
                core.push_action(0, ActionMsg::Consume { slot: 0 });
            }
            core.tick_bare(&stats, |_, _, _| true);
        }
        let eaten = 60 - core.world.players[w].inv[0].count;
        assert!(
            (3..=4).contains(&eaten),
            "{eaten} eaten in three seconds of asking every tick"
        );
    }

    #[test]
    fn rotation_event_overflow_resyncs_each_clients_final_stair() {
        use client_core::core::{ClientCore, APPLIED_PIECE_RESET};
        use sim_core::build::{SHAPE_STAIRS, STAIR_LOCS};
        use sim_core::limits::MAX_EVENTS_PER_TICK;

        const BUILD_SEED: u64 = 20_260_731;
        const CELL: u16 = 682;
        let stats = ShardStats::default();
        let mut core = Box::new(ShardCore::new(BUILD_SEED));
        core.world.dev_spawn = Some((2048.0, 2048.0));
        core.world.gather = GatherContent::probe_fixture();
        core.world.build = BuildContent::probe_fixture();
        let row = core.world.build.piece_count;
        core.world.build.pieces[row as usize] = sim_core::build::PieceDef {
            shape: SHAPE_STAIRS,
            ..core.world.build.pieces[1]
        };
        core.world.build.piece_count += 1;
        let mut mirrors = [
            ClientCore::new(BUILD_SEED, PLAYER, 0),
            ClientCore::new(BUILD_SEED, PLAYER + 1, 0),
        ];
        for (slot, id) in [PLAYER, PLAYER + 1].into_iter().enumerate() {
            assert!(core.connect(slot, id));
        }
        let mut receive = |lane, slot: usize, bytes: &[u8]| {
            if lane == Lane::Event {
                mirrors[slot].on_stream(bytes).unwrap();
            }
            true
        };
        core.tick_bare(&stats, &mut receive);
        let builder = core.live_wslot(0).unwrap();
        core.world.players[builder].inv[0] = ItemStack {
            item: 0,
            count: 20,
            cond: 0,
            skin: 0,
        };
        for (row, loc) in [(0, LOC_PLANE), (row, STAIR_LOCS[0])] {
            core.push_action(
                0,
                ActionMsg::Place {
                    row,
                    cx: CELL,
                    cz: CELL,
                    level: 0,
                    loc,
                    freehand: false,
                    plate: 0,
                },
            );
            // Two placements a tick apart: the second waits out the build
            // pace in the hand (`pace.rs`) before it lands.
            for _ in 0..=crate::pace::gap(crate::pace::Kind::Build) {
                core.tick_bare(&stats, &mut receive);
            }
        }
        assert_eq!(core.world.pieces.len(), 2);
        // Directly exercise the sim command ceiling: more than half the
        // event cap in paired rotations. An odd count leaves a different
        // final orientation from the prefix the event ring can retain.
        let turns = MAX_COMMANDS_PER_TICK - 1;
        assert!(turns * 2 > MAX_EVENTS_PER_TICK);
        let commands: Vec<_> = (0..turns)
            .map(|turn| Command::Rotate {
                id: PLAYER,
                cx: CELL,
                cz: CELL,
                level: 0,
                loc: STAIR_LOCS[turn % STAIR_LOCS.len()],
            })
            .collect();
        core.world.tick(&commands);
        assert!(core.world.events.dropped > 0);
        let last = STAIR_LOCS[turns % STAIR_LOCS.len()];
        assert!(core.world.pieces.find(CELL, CELL, 0, last).is_some());
        let mut flags = [0u32; 2];
        core.pump_events(&stats, &mut |lane, slot, bytes| {
            if lane == Lane::Event {
                flags[slot] |= mirrors[slot].on_stream(bytes).unwrap();
            }
            true
        });
        for (slot, mirror) in mirrors.iter().enumerate() {
            assert_ne!(flags[slot] & APPLIED_PIECE_RESET, 0);
            assert_eq!(mirror.pieces.len(), core.world.pieces.len());
            assert!(mirror.pieces.entries().iter().any(|r| r.loc == last));
            assert_eq!(
                mirror.pieces.cols().get(CELL, CELL),
                core.world.pieces.cols().get(CELL, CELL)
            );
        }
        assert_eq!(ShardStats::get(&stats.ev_resyncs_dropped), 2);
        assert_eq!(
            ShardStats::get(&stats.ev_sim_dropped),
            core.world.events.dropped as u64
        );
    }

    #[test]
    fn bag_removed_refuses_the_reason_the_sim_cannot_mean() {
        let stats = ShardStats::default();
        let mut core = quiet_core(&stats);
        // A real bag in the store, and a client whose walk over it has
        // finished. No removal restarts the bag walk any more (it reads
        // tail-down, `drip_client`), so the walk state must come through
        // both the refused event and the real one untouched.
        core.world.backpack = BackpackContent::probe_fixture();
        let one = [ItemStack {
            item: 0,
            count: 1,
            cond: 0,
            skin: 0,
        }; INV_SLOTS];
        let w = &mut core.world;
        w.backpacks
            .stand_up(&w.backpack, 0, 0, 0, PLAYER, &one, 0, &mut w.events)
            .expect("bag stands");
        core.world.tick(&[]); // flush the EV_BAG_DROPPED it pushed
        assert!(core.world.events.is_empty(), "ring quiet again");
        core.clients[0].bag_sync_cursor = 0;
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
            core.clients[0].bag_sync_cursor, 0,
            "the walk cursor moved for a refused event"
        );
        assert!(!core.clients[0].bag_sync_reset, "same, the reset flag");

        // Just inside: why == BAG_GONE_MAX still crosses, and it restarts
        // nothing.
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
        assert!(
            core.clients[0].bag_sync_cursor == 0 && !core.clients[0].bag_sync_reset,
            "a removal restarted the bag walk"
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

    /// A killing blow's `EV_STRUCT_HIT` (hp left 0) stays off the wire and
    /// out of the fault counters: the removal behind it says the piece is
    /// gone. A hit that leaves hp still crosses.
    #[test]
    fn a_killing_struct_hit_is_the_removals_to_say() {
        let stats = ShardStats::default();
        let mut core = quiet_core(&stats);
        let range_before = ShardStats::get(&stats.encode_range_errors);
        let key = (40 << 16) | 41;
        core.world.events.push(EV_STRUCT_HIT, key, 0, 10 << 16);
        core.world
            .events
            .push(EV_STRUCT_HIT, key, 0, (10 << 16) | 90);
        let sent = pumped(&mut core, &stats);
        let lefts = sent
            .iter()
            .filter_map(|b| match decode_event(b) {
                Ok(EventMsg::StructHit { left, .. }) => Some(left),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(lefts, vec![90], "only the hit that left hp crossed");
        assert_eq!(
            ShardStats::get(&stats.encode_range_errors),
            range_before,
            "a destroyed structure is not an encode fault"
        );
    }

    /// `EV_GATHER`'s and `EV_CRAFT_DONE`'s `c` is what went to the ground
    /// (NOW §0sp2), and the route copies it into the wire's `dropped`: the
    /// spill toast's count comes from nowhere else, so a route that dropped
    /// `c` would say "0 dropped" with every gate on the codec green.
    #[test]
    fn gather_and_craft_done_carry_what_spilled() {
        let stats = ShardStats::default();
        let mut core = quiet_core(&stats);
        core.world.events.push(EV_GATHER, PLAYER, (3 << 16) | 4, 5);
        core.world
            .events
            .push(EV_CRAFT_DONE, PLAYER, (3 << 16) | 4, 5);
        let sent = pumped(&mut core, &stats);
        let got = sent
            .iter()
            .filter_map(|b| match decode_event(b) {
                Ok(m @ (EventMsg::Gather { .. } | EventMsg::CraftDone { .. })) => Some(m),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            got,
            vec![
                EventMsg::Gather {
                    item: 3,
                    added: 4,
                    dropped: 5
                },
                EventMsg::CraftDone {
                    item: 3,
                    added: 4,
                    dropped: 5
                },
            ]
        );
    }

    /// Everything `fan_out` sent, per connection slot, in order.
    fn fanned(core: &mut ShardCore, stats: &ShardStats) -> Vec<Vec<Vec<u8>>> {
        let mut out = vec![Vec::new(); MAX_CONNS];
        let live = core.live_seats();
        let mut log = crate::anomaly::Sink::off();
        let mut save_now = false;
        let mut ops = Ops {
            log: &mut log,
            admin_tx: None,
            admin_answers: None,
            save_now: &mut save_now,
        };
        core.fan_out(&live, stats, &mut ops, &mut |lane, slot, bytes: &[u8]| {
            if lane == Lane::Event {
                out[slot].push(bytes.to_vec());
            }
            true
        });
        out
    }

    /// **A seat hears exactly what its target hears, and nothing a stranger
    /// hears** (`NETCODE.md` §2.3). Two players, one watched: the target's
    /// own-facts and the broadcast it was shown reach the seat byte for byte
    /// and in order; the other player's own-facts reach neither.
    #[test]
    fn a_seat_hears_every_fact_its_target_hears_and_no_strangers() {
        const OTHER: u32 = PLAYER + 1;
        let stats = ShardStats::default();
        let mut core = quiet_core(&stats);
        assert!(core.connect(1, OTHER));
        core.tick_bare(&stats, |_, _, _| true);
        let seat = MAX_PLAYERS + 3;
        assert!(core.connect_spectator(seat, PLAYER, 0), "seated");
        core.tick_bare(&stats, |_, _, _| true);
        core.world.tick(&[]);
        assert!(core.world.events.is_empty());

        // Own-facts for each player, and one broadcast.
        core.world.events.push(EV_HEALTH, PLAYER, 55, 100);
        core.world.events.push(EV_HEALTH, OTHER, 11, 100);
        core.world.events.push(EV_GATHER, PLAYER, (3 << 16) | 9, 0);
        core.world.events.push(EV_GATHER, OTHER, (4 << 16) | 2, 0);
        core.world
            .events
            .push(EV_SLOT_HARVESTED, (40 << 16) | 41, 0, 0);
        let out = fanned(&mut core, &stats);
        assert!(
            out[0].len() >= 3,
            "the target heard its facts: {:?}",
            out[0]
        );
        assert_eq!(out[seat], out[0], "the seat must hear exactly the target");
        assert_ne!(out[1], out[0], "the stranger heard different facts");
        // And spelled out, so a mirror that forwarded EVERY player's copy
        // (the stranger's too) could not pass by coincidence of ordering.
        let healths: Vec<u16> = out[seat]
            .iter()
            .filter_map(|b| match decode_event(b) {
                Ok(EventMsg::Health { hp, .. }) => Some(hp),
                _ => None,
            })
            .collect();
        assert_eq!(healths, vec![55], "only the target's own health");
        // No seat that watches nobody is spoken to.
        for (i, msgs) in out.iter().enumerate().skip(MAX_PLAYERS) {
            if i != seat {
                assert!(msgs.is_empty(), "seat slot {i} heard {msgs:?}");
            }
        }
    }

    /// **A seat whose target left is fed nothing — not the next tenant of the
    /// target's slot.** Connection slots are reused and a reused slot's new
    /// player has a new id; without the id check a seat would be handed a
    /// stranger's own-facts and view. A privacy defect, not a cosmetic one.
    #[test]
    fn a_seat_whose_target_left_hears_nothing_from_the_slots_next_tenant() {
        const NEXT: u32 = PLAYER + 0x100;
        let stats = ShardStats::default();
        let mut core = quiet_core(&stats);
        let seat = MAX_PLAYERS;
        assert!(core.connect_spectator(seat, PLAYER, 0));
        core.tick_bare(&stats, |_, _, _| true);
        // The target leaves and a stranger takes its connection slot.
        let _ = core.disconnect(0);
        core.tick_bare(&stats, |_, _, _| true);
        assert!(core.connect(0, NEXT));
        core.tick_bare(&stats, |_, _, _| true);
        core.world.tick(&[]);
        core.world.events.push(EV_HEALTH, NEXT, 42, 100);
        let out = fanned(&mut core, &stats);
        assert!(!out[0].is_empty(), "the new tenant hears its own facts");
        assert!(
            out[seat].is_empty(),
            "the seat heard a stranger: {:?}",
            out[seat]
        );
        // Nor does it get the stranger's view: a whole tick sends it nothing.
        let mut to_seat = 0;
        core.tick_bare(&stats, |_, slot, _| {
            to_seat += usize::from(slot == seat);
            true
        });
        assert_eq!(to_seat, 0, "an orphaned seat was sent {to_seat} messages");
        assert_eq!(
            core.spectators(),
            1,
            "still seated until the accept loop closes it"
        );
        assert_eq!(core.connected(), 1, "and never counted as a player");
    }

    /// **A new seat hears its target's join facts first** — health, vitals
    /// and the research mask, from the body, ahead of its first drip on the
    /// same ordered lane. A player hears these once, as events at its join;
    /// without this a late watcher's HUD would read zero until the next
    /// hunger tick happened to announce them.
    #[test]
    fn a_new_seat_is_told_its_targets_join_facts_before_anything_else() {
        let stats = ShardStats::default();
        let mut core = quiet_core(&stats);
        let w = core.live_wslot(0).expect("the target has a body");
        core.world.players[w].hp = 61;
        core.world.players[w].hp_max = 100;
        core.world.players[w].known = 0b1011;
        let seat = MAX_PLAYERS + 2;
        assert!(core.connect_spectator(seat, PLAYER, 0));
        let mut first = Vec::new();
        core.tick_bare(&stats, |lane, slot, bytes| {
            if lane == Lane::Event && slot == seat {
                first.push(decode_event(bytes).expect("decodes"));
            }
            true
        });
        let hp_max = core.world.players[w].hp_max;
        assert!(
            matches!(first.first(), Some(EventMsg::Health { hp: 61, max }) if *max == hp_max),
            "the seat's first fact must be its target's health: {:?}",
            first.first()
        );
        assert!(
            matches!(first.get(1), Some(EventMsg::Vitals { .. })),
            "{first:?}"
        );
        assert!(
            matches!(first.get(2), Some(EventMsg::Known { mask: 0b1011 })),
            "{first:?}"
        );
        // Owed once: the next tick does not repeat it.
        let mut again = 0;
        core.tick_bare(&stats, |lane, slot, bytes| {
            if lane == Lane::Event && slot == seat {
                again += usize::from(matches!(decode_event(bytes), Ok(EventMsg::Health { .. })));
            }
            true
        });
        assert_eq!(again, 0, "the catch-up is owed once, not every tick");
    }

    /// A seat's datagrams are acks: frames that reach `push_input` anyway
    /// (past the net side's refusal) become nothing the tick can execute.
    #[test]
    fn a_seat_never_buffers_an_input_frame() {
        let stats = ShardStats::default();
        let mut core = quiet_core(&stats);
        let seat = MAX_PLAYERS + 1;
        assert!(core.connect_spectator(seat, PLAYER, 0));
        let mut dg = InputDatagram::new(0, 0, 4);
        for seq in 1..=5u16 {
            dg.push(sim_core::input::InputFrame {
                seq,
                move_z: 127,
                ..Default::default()
            })
            .unwrap();
        }
        core.push_input(seat, &dg);
        assert!(
            core.clients[seat].consume_input().is_none(),
            "a seat buffered a frame"
        );
        assert!(!core.wants_action(seat), "a seat's hand is never open");
    }

    /// A seat is refused for a target that is not (or no longer) that id on
    /// that slot, and for a slot that is not a seat.
    #[test]
    fn a_seat_is_refused_for_the_wrong_target_or_slot() {
        let stats = ShardStats::default();
        let mut core = quiet_core(&stats);
        assert!(
            !core.connect_spectator(MAX_PLAYERS, PLAYER + 1, 0),
            "wrong id"
        );
        assert!(
            !core.connect_spectator(MAX_PLAYERS, PLAYER, 1),
            "empty slot"
        );
        assert!(
            !core.connect_spectator(5, PLAYER, 0),
            "a player slot is not a seat"
        );
        assert!(
            !core.connect_spectator(MAX_CONNS, PLAYER, 0),
            "past the seats"
        );
        assert_eq!(core.spectators(), 0);
    }

    /// A full eviction memo gives up its oldest put, never a newer one, and
    /// a key filed twice holds one row (its newer record).
    #[test]
    fn a_full_evict_memo_drops_its_oldest_put() {
        let key = |n: usize| PlayerKey::new(format!("victim-{n}").as_bytes()).unwrap();
        let save = |n: usize| PlayerSave {
            hp: n as u16,
            ..PlayerSave::EMPTY
        };
        let mut memo = EvictMemo::new();
        for n in 0..MAX_PLAYERS {
            memo.put(&key(n), save(n));
        }
        memo.put(&key(3), save(1000));
        assert_eq!(memo.find(&key(3)), Some(save(1000)), "re-filed in place");
        memo.put(&key(MAX_PLAYERS), save(MAX_PLAYERS));
        assert_eq!(memo.find(&key(0)), None, "the oldest put goes");
        assert_eq!(memo.find(&key(1)), Some(save(1)));
        assert_eq!(memo.find(&key(MAX_PLAYERS)), Some(save(MAX_PLAYERS)));
        memo.forget(&key(1));
        assert_eq!(memo.find(&key(1)), None);
    }
}
