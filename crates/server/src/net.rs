//! The I/O shell around `ShardCore`: wtransport termination, the session
//! hello (DESIGN.md §5.9), per-connection tasks, and the pinned 30 Hz sim
//! thread. Transport config of record per NETCODE.md §2.2: keep-alive
//! 10 s, idle 30 s, 64 KiB datagram send buffer, `send_datagram`
//! (drop-oldest) and never `_wait`.

use crate::config::ShardConfig;
use crate::core::{Admitted, Lane, ShardCore};
use crate::slot::{
    generation_of, state_of, Connect, EvMsg, Link, SaveMsg, SlotTable, SnapMsg, WorldDone,
    WorldMsg, WriteMsg, SLOT_LEAVING, SLOT_LIVE,
};
use crate::stats::ShardStats;
use crate::store::{PlayerKey, SaveFile, SaveStore, Saves};
use protocol::{
    decode_action, decode_chat, decode_hello, decode_input, encode_refuse, encode_welcome,
    peek_kind, ActionMsg, ChatMsg, ItemCatalog, Refuse, Welcome, KIND_CHAT, KIND_INPUT,
    MAX_EVENT_MSG_BYTES, MAX_STREAM_MSG_BYTES, PROTO_VER, REFUSE_FULL, REFUSE_VERSION,
};
use rtrb::RingBuffer;
use sim_core::input::BTN_MASK;
use sim_core::limits::{
    ACTION_RING_CAP, CHAT_RING_CAP, CTRL_RING_CAP, EVENT_RING_CAP, GRAVEYARD_RING_CAP,
    INPUT_RING_CAP, MAX_PLAYERS, SAVE_RING_CAP, SNAPSHOT_RING_CAP, TICK_HZ, WORLD_RING_CAP,
};
use sim_core::worldsave::WORLD_SAVE_MAX_BYTES;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use wtransport::endpoint::endpoint_side::Server;
use wtransport::endpoint::IncomingSession;
use wtransport::error::SendDatagramError;
use wtransport::tls::Sha256DigestFmt;
use wtransport::{Connection, Endpoint, Identity, RecvStream, SendStream, ServerConfig};

/// Handshake must finish inside this or the session drops (edge flood
/// control, DESIGN.md §10). Plumbing bound, DECISIONS.md §open row.
/// Bits of the slot's claim generation that reach a player id. See the id
/// derivation below: the ceiling is `limits::MOB_ID_TAG`, not arithmetic.
const PLAYER_ID_GEN_MASK: u32 = 0x007F_FFFF;

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
/// Writer poll cadence: how often per-connection outbound rings drain.
/// Latency floor for a snapshot, far under the 66 ms snapshot interval.
const WRITER_POLL: Duration = Duration::from_millis(2);
/// Sim thread abandons backlog beyond this many ticks (a debugger pause,
/// a VM freeze) instead of sprinting to catch up.
const MAX_TICK_BACKLOG: u32 = 8;

/// How long the accept loop will keep draining the save ring after a
/// shutdown is signalled, waiting for the sim thread's final flush: 200
/// tries × 5 ms = 1 s.
///
/// A backstop and not the mechanism. The real exit is `save_rx` being
/// *abandoned* — the sim thread dropping its producer, which is exact — and
/// this only bounds the case where the sim thread is wedged rather than
/// finishing. A shutdown that takes a second is a shutdown; one that hangs
/// forever is a deploy that never completes.
const SHUTDOWN_DRAIN_TRIES: u32 = 200;
const SHUTDOWN_DRAIN_POLL: Duration = Duration::from_millis(5);

/// Chat rate limit (ALPHA.md §1: "rate-limited server-side"), a token
/// bucket per connection: `CHAT_BURST` lines may go back to back, then
/// one more every `CHAT_REFILL` — a sustained ~30 lines/min with room to
/// answer a question fast. Proposed defaults, DECISIONS.md §open
/// ("chat v0"). Refused lines are counted, not announced: the limiter is
/// the shard's business, not a conversation with the spammer.
const CHAT_BURST: u32 = 3;
const CHAT_REFILL: Duration = Duration::from_secs(2);

/// One connection's chat token bucket. Lives in the reader task, so it is
/// per stream — a reconnect buys a fresh bucket, and the handshake costs
/// far more than the three lines that would buy.
struct ChatLimiter {
    tokens: u32,
    /// When the bucket was last accounted; None until the first line.
    last: Option<Instant>,
}

impl ChatLimiter {
    fn new() -> Self {
        Self {
            tokens: CHAT_BURST,
            last: None,
        }
    }

    /// Spend a token if one has accrued. `now` is a parameter, not a
    /// `Instant::now()` call inside, so the policy is testable without
    /// sleeping.
    fn allow(&mut self, now: Instant) -> bool {
        let last = *self.last.get_or_insert(now);
        let steps = now.saturating_duration_since(last).as_nanos() / CHAT_REFILL.as_nanos();
        if steps >= CHAT_BURST as u128 {
            // Idle long enough to fill the bucket. The clock restarts
            // here on purpose: leaving `last` in the deep past would hand
            // out a full bucket on every call from then on — a limiter
            // that stops limiting, which is the failure mode worth
            // naming.
            self.tokens = CHAT_BURST;
            self.last = Some(now);
        } else if steps > 0 {
            let steps = steps as u32;
            self.tokens = (self.tokens + steps).min(CHAT_BURST);
            self.last = Some(last + CHAT_REFILL * steps);
        }
        if self.tokens == 0 {
            return false;
        }
        self.tokens -= 1;
        true
    }
}

pub struct ShardHandle {
    pub local_addr: SocketAddr,
    /// SHA-256 of the dev certificate, `serverCertificateHashes` format.
    pub cert_hash: String,
    pub stats: Arc<ShardStats>,
    pub shutdown: Arc<AtomicBool>,
}

/// Bake the item-name catalog from validated content (the same
/// index-is-sorted-rank mapping `bake_gather` uses). Boot path: a name
/// the wire can't carry refuses the boot, same as any other bake error.
pub fn bake_catalog(content: &content::Content) -> Result<ItemCatalog, String> {
    let mut cat = ItemCatalog::EMPTY;
    cat.count = content.items.len() as u16;
    for item in &content.items {
        let idx = content.item_index(&item.id).expect("own id resolves") as usize;
        cat.set(idx, item.name.as_bytes()).map_err(|_| {
            format!(
                "catalog: item `{}` name `{}` is empty or over {} bytes",
                item.id,
                item.name,
                protocol::MAX_ITEM_NAME_BYTES
            )
        })?;
    }
    Ok(cat)
}

/// Boot a shard: bind, spawn the sim thread and the accept loop, return.
/// The caller owns process lifetime; `shutdown` stops the sim thread.
/// `gather`, `craft`, `build`, `deploy`, `combat`, and `catalog` are the
/// content bake (CLAUDE.md wall 7) — data the world runs on, handed over
/// before the first tick like the seed.
///
/// `saves` is the player store, already opened and validated against this
/// seed and content (`store::open`) — or `Saves::off()`, which is a shard
/// that was told to remember nobody and is what every test here runs. It is
/// a parameter rather than something opened in here because validating it
/// needs the *content hash*, which this function is never handed: the
/// binary bakes content and therefore the binary opens the file, and a
/// refusal lands before a port is bound.
#[allow(clippy::too_many_arguments)]
pub async fn spawn_shard(
    cfg: ShardConfig,
    gather: sim_core::gather::GatherContent,
    craft: sim_core::craft::CraftContent,
    build: sim_core::build::BuildContent,
    deploy: sim_core::deploy::DeployContent,
    combat: sim_core::combat::CombatContent,
    backpack: sim_core::backpack::BackpackContent,
    survival: sim_core::survival::SurvivalContent,
    cook: sim_core::oven::CookContent,
    spawn_kit: sim_core::inventory::SpawnKit,
    loot: sim_core::loot::LootContent,
    mobs: sim_core::mob::MobContent,
    catalog: ItemCatalog,
    saves: Saves,
    world_boot: crate::worldfile::WorldBoot,
) -> Result<ShardHandle, String> {
    // The island validates at boot the way content does (CLAUDE.md wall 7),
    // and here rather than in `bin/shard.rs` so that every path that raises a
    // shard — the binary, the bots, the wire tests — refuses the same seed.
    // First, before an identity is loaded or a port is bound: a refusal this
    // cheap should cost neither.
    crate::boot::check_seed(cfg.seed)?;

    // A PUBLIC shard serves a real certificate chain and browsers trust it
    // outright; the dev flow self-signs for loopback and the page passes the
    // hash below through `serverCertificateHashes`. Both or neither is
    // enforced at parse (config.rs), so one `is_some` decides it.
    let identity = match (&cfg.cert_pem, &cfg.key_pem) {
        (Some(cert), Some(key)) => Identity::load_pemfiles(cert, key)
            .await
            .map_err(|e| format!("loading {cert} / {key}: {e}"))?,
        _ => Identity::self_signed(["localhost", "127.0.0.1", "::1"])
            .map_err(|e| format!("self-signed identity: {e}"))?,
    };
    let cert_hash = identity
        .certificate_chain()
        .as_slice()
        .first()
        .ok_or("empty certificate chain")?
        .hash()
        .fmt(Sha256DigestFmt::DottedHex);

    let mut transport = wtransport::config::QuicTransportConfig::default();
    // NETCODE.md §2.2: bound worst-case queued staleness; snapshots
    // replace, never accumulate.
    transport.datagram_send_buffer_size(64 * 1024);
    let server_config = ServerConfig::builder()
        .with_bind_address(cfg.bind)
        .with_custom_transport(identity, transport)
        .max_idle_timeout(Some(Duration::from_secs(30)))
        .map_err(|e| format!("idle timeout: {e}"))?
        .keep_alive_interval(Some(Duration::from_secs(10)))
        .build();

    let endpoint =
        Endpoint::server(server_config).map_err(|e| format!("endpoint bind {}: {e}", cfg.bind))?;
    let local_addr = endpoint
        .local_addr()
        .map_err(|e| format!("local_addr: {e}"))?;

    let stats = Arc::new(ShardStats::default());
    let shutdown = Arc::new(AtomicBool::new(false));
    let slots = Arc::new(SlotTable::new(MAX_PLAYERS));

    let (ctrl_tx, ctrl_rx) = RingBuffer::<Connect>::new(CTRL_RING_CAP);
    let (grave_tx, grave_rx) = RingBuffer::<Link>::new(GRAVEYARD_RING_CAP);
    // The save path, two hops and one direction: the sim takes records and
    // knows no keys; the accept loop owns the key table and the index; the
    // storage thread owns the file and decides nothing. Each hop is a bounded
    // SPSC ring, so nothing on the way to a disk can block a tick.
    let (save_tx, save_rx) = RingBuffer::<SaveMsg>::new(SAVE_RING_CAP);
    let (write_tx, write_rx) = RingBuffer::<WriteMsg>::new(SAVE_RING_CAP);
    // The world path, one hop and two directions: full buffers down to the
    // store thread, emptied ones back. Depth 2 because that is what a
    // double buffer is — one being written while one is being filled — and
    // a deeper queue would only let stale worlds pile up behind a slow disk
    // when the correct answer to a slow disk is to skip a save.
    let (world_tx, world_rx) = RingBuffer::<WorldMsg>::new(WORLD_RING_CAP);
    let (world_done_tx, world_done_rx) = RingBuffer::<WorldDone>::new(WORLD_RING_CAP);
    // The admin path, sim → accept, one direction: a kick needs a socket
    // and the sim thread holds none (`admin.rs`'s split). Shallow because
    // an admin is a person typing — `CTRL_RING_CAP` is the same size for
    // the same reason — and a full ring refuses the act out loud rather
    // than queueing a kick nobody remembers ordering.
    let (admin_tx, admin_rx) = RingBuffer::<crate::admin::AdminAct>::new(CTRL_RING_CAP);
    let crate::worldfile::WorldBoot {
        file: world_file,
        idents: world_idents,
        blob: world_blob,
        interval_ticks: world_interval,
    } = world_boot;

    // The anomaly log, opened before the sim thread that writes to it —
    // and a failure to open is a **boot** failure, not a silent downgrade:
    // an operator who configured a log and got none would be reading an
    // empty file after the incident they wanted it for (`anomaly.rs`).
    let log = match cfg.anomaly_file.as_deref() {
        Some(path) => {
            let (sink, _thread) = crate::anomaly::spawn(std::path::Path::new(path), stats.clone())
                .map_err(|e| format!("anomaly log `{path}`: {e}"))?;
            // The handle is deliberately dropped: the thread ends when the
            // sim thread's `Sink` drops at shutdown, which is the signal
            // that every record has been written — joining here would mean
            // waiting for it before the shard has started.
            sink
        }
        None => crate::anomaly::Sink::off(),
    };

    {
        let stats = stats.clone();
        let shutdown = shutdown.clone();
        let slots = slots.clone();
        let seed = cfg.seed;
        let dev_spawn = cfg.dev_spawn;
        let admins = cfg.admins.clone();
        std::thread::Builder::new()
            .name("sim".into())
            .spawn(move || {
                sim_thread(
                    seed,
                    dev_spawn,
                    gather,
                    craft,
                    build,
                    deploy,
                    combat,
                    backpack,
                    survival,
                    cook,
                    spawn_kit,
                    loot,
                    mobs,
                    catalog,
                    world_blob,
                    world_idents,
                    world_interval,
                    ctrl_rx,
                    grave_tx,
                    save_tx,
                    world_tx,
                    world_done_rx,
                    admin_tx,
                    log,
                    admins,
                    slots,
                    stats,
                    shutdown,
                )
            })
            .map_err(|e| format!("sim thread spawn: {e}"))?;
    }

    {
        // Blocking file I/O, on a thread of its own: DESIGN.md §8's "the
        // storage thread serializes". Not a tokio task, because a `sync_data`
        // inside the runtime would stall whatever else that worker owed —
        // including an accept.
        let stats = stats.clone();
        let file = saves.file;
        std::thread::Builder::new()
            .name("store".into())
            .spawn(move || store_thread(file, write_rx, world_file, world_rx, world_done_tx, stats))
            .map_err(|e| format!("store thread spawn: {e}"))?;
    }

    tokio::spawn(accept_loop(
        endpoint,
        ShardFacts {
            seed: cfg.seed,
            // A shard running a dev override is a dev shard, and says so
            // in every welcome — that bit is the client's only dev gate.
            dev: cfg.dev_spawn.is_some(),
            require_auth: cfg.require_auth,
            domain: cfg.domain.clone(),
            entitle: cfg.entitle.clone(),
            min_client: cfg.min_client,
        },
        ctrl_tx,
        grave_rx,
        save_rx,
        write_tx,
        admin_rx,
        saves.store,
        slots,
        stats.clone(),
        shutdown.clone(),
    ));

    Ok(ShardHandle {
        local_addr,
        cert_hash,
        stats,
        shutdown,
    })
}

// ---------------------------------------------------------------------------
// Accept side
// ---------------------------------------------------------------------------

/// What every joiner is told about the shard itself: the seed its whole
/// world derives from, and whether this is a dev shard — the bit the
/// client gates its dev affordances on.
#[derive(Clone)]
struct ShardFacts {
    seed: u64,
    dev: bool,
    /// `shard.toml require_auth`. See `config.rs` for why it is a knob.
    require_auth: bool,
    /// The SIWE domain: what this shard calls itself in the message players
    /// sign. Must be the host they dialled (`config.rs`).
    domain: String,
    /// The ticket door (`entitle.rs`). `Config::off()` — the default — checks
    /// nothing, which is what every test and every community shard runs.
    entitle: crate::entitle::Config,
    /// `shard.toml min_client`, packed. 0 — the default — admits every client
    /// whose `PROTO_VER` already matched, which is every client that could
    /// have got this far.
    min_client: u32,
}

/// What a handshake task hands back once the client said a valid hello.
/// The recv half stays: after the hello it is the C→S action lane.
struct Handshaken {
    connection: Connection,
    send: SendStream,
    recv: RecvStream,
    /// Who the admission seam said this is, if it could say. `None` ⇒ a
    /// guest: admitted (on a shard that takes guests) and remembered by
    /// nobody, because there is no stable string to file them under.
    key: Option<PlayerKey>,
}

/// The accept loop's own id→key table, one entry per connection slot.
///
/// **This is where identity meets the world's ids, and it is deliberately
/// the only place.** The sim hands back saves labelled with a player id;
/// this turns an id into the key the record is filed under. It stores the id
/// alongside the key and matches it exactly, because a slot is reused: a
/// save for the previous tenant paired against the current one's key would
/// hand somebody else's inventory to whoever claimed the slot next. Ordering
/// makes that unreachable (the sim pushes a leave's record before it frees
/// the slot, and this loop drains the ring before it installs anyone), and
/// the id check is what makes it unreachable *by construction* rather than
/// by argument.
///
/// **`conn` is the roster sweep's only reach into a live connection.** A
/// player who sells their copy mid-session is not doing anything the sim can
/// see, so the kick cannot come from the sim thread — it comes from here,
/// and this is the handle it closes. Held as an `Option` because a guest
/// slot has no wallet to sweep and a freed slot has nothing at all.
///
/// No longer `Copy`: a `Connection` is refcounted and cloning it is a
/// decision, not a memcpy. The two read sites take `.key` and `.id` by value
/// as before.
#[derive(Clone, Default)]
struct KeySlot {
    key: Option<PlayerKey>,
    id: u32,
    conn: Option<Connection>,
}

#[allow(clippy::too_many_arguments)]
async fn accept_loop(
    endpoint: Endpoint<Server>,
    facts: ShardFacts,
    mut ctrl_tx: rtrb::Producer<Connect>,
    mut grave_rx: rtrb::Consumer<Link>,
    mut save_rx: rtrb::Consumer<SaveMsg>,
    mut write_tx: rtrb::Producer<WriteMsg>,
    mut admin_rx: rtrb::Consumer<crate::admin::AdminAct>,
    mut store: SaveStore,
    slots: Arc<SlotTable>,
    stats: Arc<ShardStats>,
    shutdown: Arc<AtomicBool>,
) {
    // Net-side plumbing between handshake tasks and this loop; the sim
    // thread never touches it (L3 is about the sim thread, not tokio).
    let (done_tx, mut done_rx) = tokio::sync::mpsc::channel::<Handshaken>(MAX_PLAYERS);
    // `[T; N]` initialiser syntax needs `Copy` and `KeySlot` stopped being
    // `Copy` when it started holding a `Connection`. Built on the heap and
    // converted, which is also the shape `boxed_array` uses next door for a
    // different reason (wasm's shadow stack) — here it is simply the way to
    // fill a fixed array with a clonable value.
    let mut keys: [KeySlot; MAX_PLAYERS] = std::array::from_fn(|_| KeySlot::default());
    // Wallets banned for this uptime (admin v0). Memory only, and
    // `admin.rs`' header says why that is stated rather than hidden: a
    // persisted ban wants its own file with its own format version.
    let mut bans = crate::admin::Bans::new();
    let mut sweep = tokio::time::interval(Duration::from_millis(100));
    // ---- the roster sweep -------------------------------------------------
    //
    // A join check alone is a door with no lock behind it: a player can sell
    // the ticket and keep playing. This re-asks about everybody on an
    // interval (`entitle::DEFAULT_SWEEP`), and the interval IS the security
    // property — it is how long a sold copy can linger, which is a posted
    // knob rather than a hole.
    //
    // Results come back through a channel rather than being awaited inline,
    // because this loop is also the accept path: a blocked sweep would be a
    // shard that stops taking players while scry is slow. `in_flight` is the
    // no-stacking rule — one round at a time, whatever the origin does, the
    // same refusal `status.rs`'s poller makes.
    let (kick_tx, mut kick_rx) = tokio::sync::mpsc::channel::<Vec<(usize, u32)>>(1);
    let mut entitle_sweep = tokio::time::interval(facts.entitle.sweep);
    let mut sweep_in_flight = false;
    loop {
        tokio::select! {
            incoming = endpoint.accept() => {
                let stats = stats.clone();
                let done_tx = done_tx.clone();
                tokio::spawn(handshake_task(
                    incoming,
                    done_tx,
                    stats,
                    facts.require_auth,
                    facts.domain.clone(),
                    facts.entitle.clone(),
                    facts.min_client,
                ));
            }
            Some(done) = done_rx.recv() => {
                // Before the claim, never after: a record for the slot's
                // previous tenant has to be filed under the key it was
                // written for, and installing first would overwrite that key.
                drain_saves(&mut save_rx, &mut write_tx, &mut store, &keys, &stats);
                install(done, &facts, &mut ctrl_tx, &mut keys, &store, &slots, &stats).await;
            }
            _ = entitle_sweep.tick(), if facts.entitle.armed() && !sweep_in_flight => {
                // Snapshot who to ask about, with the generation each answer
                // must still match when it lands. A slot that turned over
                // while the round was out is a DIFFERENT player, and kicking
                // them on the previous tenant's verdict is the same class of
                // bug the save path's `id` check exists to prevent.
                let roster: Vec<(usize, u32, String)> = keys
                    .iter()
                    .enumerate()
                    .filter_map(|(slot, ks)| {
                        let k = ks.key.as_ref()?;
                        let wallet = std::str::from_utf8(k.as_bytes()).ok()?.to_string();
                        let gen = crate::slot::generation_of(slots.load(slot));
                        Some((slot, gen, wallet))
                    })
                    .collect();
                if !roster.is_empty() {
                    sweep_in_flight = true;
                    let cfg = facts.entitle.clone();
                    let tx = kick_tx.clone();
                    let stats2 = stats.clone();
                    tokio::task::spawn_blocking(move || {
                        let wallets: Vec<String> =
                            roster.iter().map(|(_, _, w)| w.clone()).collect();
                        let verdicts = crate::entitle::check_many(&cfg, &wallets);
                        let mut kicks = Vec::new();
                        for ((slot, gen, _), v) in roster.iter().zip(verdicts) {
                            match v {
                                crate::entitle::Verdict::Nope => kicks.push((*slot, *gen)),
                                crate::entitle::Verdict::Unknown => {
                                    ShardStats::bump(&stats2.entitle_unknown);
                                }
                                crate::entitle::Verdict::Owns => {}
                            }
                        }
                        // Send even when empty: it is what clears the
                        // in-flight flag, so a quiet round cannot wedge the
                        // sweep off permanently.
                        let _ = tx.blocking_send(kicks);
                    });
                }
            }
            Some(kicks) = kick_rx.recv() => {
                sweep_in_flight = false;
                for (slot, gen) in kicks {
                    // The generation guard: only kick the tenant we asked
                    // about. A reconnect between the ask and the answer gets
                    // its own join check, so nothing is skipped by waiting.
                    if slot >= MAX_PLAYERS
                        || crate::slot::generation_of(slots.load(slot)) != gen
                        || crate::slot::state_of(slots.load(slot)) != crate::slot::SLOT_LIVE
                    {
                        continue;
                    }
                    ShardStats::bump(&stats.entitle_kicked);
                    slots.mark_leaving(slot, gen);
                    if let Some(conn) = keys[slot].conn.take() {
                        // Closed with the refusal code rather than dropped,
                        // so the player is told WHY by the same table a join
                        // refusal uses. A silent close reads as a network
                        // fault, and "my internet is broken" is the wrong
                        // thing to believe when the fix is to buy a copy.
                        conn.close(
                            wtransport::VarInt::from_u32(protocol::REFUSE_TICKET as u32),
                            protocol::refuse_text(protocol::REFUSE_TICKET)
                                .unwrap_or("no copy")
                                .as_bytes(),
                        );
                    }
                }
            }
            _ = sweep.tick() => {
                while let Ok(link) = grave_rx.pop() {
                    drop(link); // net side deallocates, never the sim
                }
                // Admin kicks and bans, on the same cadence as the
                // graveyard: an admin is a person typing, so 100 ms is
                // immediate and the arm costs a pop on an empty ring.
                while let Ok(act) = admin_rx.pop() {
                    let (id, ban_key) = match act {
                        crate::admin::AdminAct::Kick { id } => (id, None),
                        crate::admin::AdminAct::Ban { id, key } => (id, Some(key)),
                    };
                    if let Some(key) = ban_key {
                        // Recorded before the kick, so a full list refuses
                        // the BAN rather than kicking and forgetting why.
                        if !bans.insert(key) {
                            ShardStats::bump(&stats.admin_refused);
                            continue;
                        }
                    }
                    // The slot is found by id rather than carried, because
                    // the sim named a player and slots are the accept
                    // loop's business — and a reconnect between the two
                    // must not be kicked in the first one's name.
                    let Some(slot) = (0..MAX_PLAYERS).find(|&s| keys[s].id == id && keys[s].key.is_some())
                    else {
                        ShardStats::bump(&stats.admin_refused);
                        continue;
                    };
                    let word = slots.load(slot);
                    if crate::slot::state_of(word) != crate::slot::SLOT_LIVE {
                        ShardStats::bump(&stats.admin_refused);
                        continue;
                    }
                    ShardStats::bump(&stats.admin_kicked);
                    slots.mark_leaving(slot, crate::slot::generation_of(word));
                    if let Some(conn) = keys[slot].conn.take() {
                        // Closed with a posted reason, the entitle kick's
                        // rule: a silent close reads as a network fault,
                        // and "my internet broke" is the wrong thing for a
                        // kicked player to believe.
                        conn.close(
                            wtransport::VarInt::from_u32(protocol::REFUSE_ADMIN as u32),
                            protocol::refuse_text(protocol::REFUSE_ADMIN)
                                .unwrap_or("kicked")
                                .as_bytes(),
                        );
                    }
                }
                drain_saves(&mut save_rx, &mut write_tx, &mut store, &keys, &stats);
                if shutdown.load(Ordering::Relaxed) {
                    // The sim thread is flushing every connected player's
                    // record right now, and this loop is the only thing that
                    // can carry those to the store. Returning on the first
                    // look at the flag — which is what this did — threw them
                    // away and left the shutdown flush writing into a ring
                    // nobody would ever read.
                    //
                    // Waited on the *producer being dropped*, not on a
                    // duration: the sim thread drops `save_tx` when it is
                    // finished, so this is an exact signal. The try count is
                    // a backstop for a sim thread that is wedged rather than
                    // finishing, because "no bound is wait" applies here too.
                    for _ in 0..SHUTDOWN_DRAIN_TRIES {
                        drain_saves(&mut save_rx, &mut write_tx, &mut store, &keys, &stats);
                        if save_rx.is_abandoned() {
                            break;
                        }
                        tokio::time::sleep(SHUTDOWN_DRAIN_POLL).await;
                    }
                    drain_saves(&mut save_rx, &mut write_tx, &mut store, &keys, &stats);
                    endpoint.close(wtransport::VarInt::from_u32(0), b"shutdown");
                    return;
                }
            }
        }
    }
}

/// Hand one record to the store's index. Called only from the sim thread, so
/// it does exactly one thing that can fail and counts it: a full ring drops
/// the newest record (limits.rs `SAVE_RING_CAP`), which costs freshness and
/// never correctness — the sweep comes round again and a record is filed by
/// key, in place.
///
/// One caller passes `key`: the eviction save, whose ring-drop story is
/// different and still freshness — the store keeps the record the victim's
/// leave filed, stale by the raid but present, and the same interval the
/// autosave sweep already leaves at risk.
fn push_save(
    save_tx: &mut rtrb::Producer<SaveMsg>,
    id: u32,
    key: Option<PlayerKey>,
    save: sim_core::persist::PlayerSave,
    stats: &Arc<ShardStats>,
) {
    if save_tx.push(SaveMsg { id, key, save }).is_err() {
        ShardStats::bump(&stats.save_ring_drops);
    }
}

/// Unix seconds. Read here and nowhere near the sim thread — a save's stamp
/// orders eviction across restarts, which the world's tick counter cannot do
/// because it begins again at 0.
fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// File every record the sim has handed over, and pass each on to the disk.
///
/// Bounded by the ring's own capacity, so this cannot run long however far
/// behind it fell. A record for an id whose slot has already moved on is
/// dropped: see [`KeySlot`] for why that is the safe direction, and why the
/// ordering above keeps it from happening at all.
fn drain_saves(
    save_rx: &mut rtrb::Consumer<SaveMsg>,
    write_tx: &mut rtrb::Producer<WriteMsg>,
    store: &mut SaveStore,
    keys: &[KeySlot; MAX_PLAYERS],
    stats: &Arc<ShardStats>,
) {
    while let Ok(msg) = save_rx.pop() {
        let key = match msg.key {
            // The sim named the key itself: an eviction save. The victim's
            // connection ended long ago, so the id below names a slot some
            // later tenant may hold — the table here cannot resolve it, and
            // the sim's own sleeper index was the last pairing standing
            // (`SaveMsg::key`).
            Some(key) => key,
            None => {
                // `id = generation << 8 | slot` (see `install`), so the
                // slot is the low byte and the generation check is the id
                // equality below.
                let slot = (msg.id & 0xFF) as usize;
                if slot >= MAX_PLAYERS || keys[slot].id != msg.id {
                    continue;
                }
                let Some(key) = keys[slot].key else {
                    continue; // a guest: admitted, remembered by nobody
                };
                key
            }
        };
        let stamp = now_secs();
        let put = store.put(&key, stamp, msg.save);
        ShardStats::bump(&stats.saves_taken);
        if put.evicted {
            ShardStats::bump(&stats.saves_evicted);
        }
        if write_tx
            .push(WriteMsg {
                index: put.index,
                key,
                stamp,
                save: msg.save,
            })
            .is_err()
        {
            // The index has it; the disk does not. Freshness, not
            // correctness — the sweep re-takes this player, and the record
            // is idempotent (filed by key, written in place).
            ShardStats::bump(&stats.save_ring_drops);
        }
    }
}

/// Take one world save: fill a pooled buffer and hand it to the store
/// thread, or skip and count.
///
/// **The whole of the sim thread's cost is in here**, and it is two linear
/// passes over state that is already in cache: `encode_world` writes
/// integers into a buffer that already exists, and `identities` fills a
/// `Vec` whose capacity was reserved at boot. No allocation (wall 2), no
/// lock or syscall (wall 3), and a ceiling that does not move with the size
/// of the world (`WORLD_SAVE_MAX_BYTES`, wall 4).
///
/// That is the whole answer to `reference/SAVES.md` §4 — a stop-the-world
/// freeze the reference game has not fixed in thirteen years, whose only
/// mitigation is a convar with a bad end on both sides. The freeze is not
/// caused by saving being expensive; it is caused by *serialising an object
/// graph on the thread that runs the game*. Split those two and the knob
/// stops having a second end.
///
/// A skip is the stated overflow policy and not a failure: the cadence
/// comes around again with a fresher world, so nothing is lost that a later
/// save does not carry.
fn take_world_save(
    core: &mut ShardCore,
    pool: &mut Vec<Box<[u8]>>,
    idents: &mut Vec<Vec<(PlayerKey, u32)>>,
    world_tx: &mut rtrb::Producer<WorldMsg>,
    stats: &ShardStats,
) {
    let (Some(mut buf), Some(mut ids)) = (pool.pop(), idents.pop()) else {
        ShardStats::bump(&stats.world_saves_skipped);
        return;
    };
    let Some(len) = core.encode_world(&mut buf) else {
        // Unreachable: the buffer is `WORLD_SAVE_MAX_BYTES` and that is the
        // ceiling by construction. Counted rather than unwrapped, and the
        // buffer goes back to the pool either way.
        ShardStats::bump(&stats.world_save_errors);
        pool.push(buf);
        idents.push(ids);
        return;
    };
    ids.resize(MAX_PLAYERS, (PlayerKey::PLACEHOLDER, 0));
    let n = core.identities(&mut ids);
    ids.truncate(n);
    let tick = core.world.tick;
    if let Err(rtrb::PushError::Full(msg)) = world_tx.push(WorldMsg {
        tick,
        len,
        buf,
        idents: ids,
    }) {
        ShardStats::bump(&stats.world_saves_skipped);
        pool.push(msg.buf);
        let mut back = msg.idents;
        back.clear();
        idents.push(back);
    }
}

/// The storage thread: the only thing in this process that touches the save
/// file, and it makes no decisions — the index owner already chose the slot.
///
/// Exits when its producer is gone AND the ring is dry, which is exact rather
/// than timed: the accept loop dropping `write_tx` is the shutdown signal, and
/// a sleep-then-guess would either lose the last records or hold the process
/// open for a fixed pause.
fn store_thread(
    mut file: SaveFile,
    mut write_rx: rtrb::Consumer<WriteMsg>,
    mut world_file: crate::worldfile::WorldFile,
    mut world_rx: rtrb::Consumer<WorldMsg>,
    mut world_done_tx: rtrb::Producer<WorldDone>,
    stats: Arc<ShardStats>,
) {
    loop {
        let mut idle = true;
        // The world first: it is the bigger write and the rarer one, and
        // taking it before the player records means a shutdown flush lands
        // the world before the thread notices its producers are gone.
        while let Ok(msg) = world_rx.pop() {
            idle = false;
            match world_file.write(msg.tick, &msg.buf[..msg.len], &msg.idents) {
                Ok(true) => ShardStats::bump(&stats.world_saves_written),
                // No file: nothing written, nothing counted — the same rule
                // `SaveFile::write` states, and for the same reason.
                Ok(false) => {}
                Err(_) => ShardStats::bump(&stats.world_save_errors),
            }
            // The buffer goes home whatever happened. A write that failed
            // must not also cost the pool a buffer, or a shard with a full
            // disk would stop being *able* to save once the disk was fixed.
            let WorldMsg {
                buf, mut idents, ..
            } = msg;
            idents.clear();
            let _ = world_done_tx.push(WorldDone { buf, idents });
        }
        while let Ok(msg) = write_rx.pop() {
            idle = false;
            match file.write(msg.index, &msg.key, msg.stamp, &msg.save) {
                Ok(true) => ShardStats::bump(&stats.saves_written),
                // No file: nothing was written and nothing is counted. The
                // index still has the record, which is the documented
                // in-memory case (`config.rs` on `save_file`) — counting it as
                // written would report persistence a shard does not have.
                Ok(false) => {}
                // A shard that cannot persist keeps running: dropping every
                // player because a disk filled would be a worse outcome than
                // a shard that forgets. Counted, and that counter is the only
                // place this is visible.
                Err(_) => ShardStats::bump(&stats.save_write_errors),
            }
        }
        // Both rings, not just the player one. The ordering makes a
        // single check safe in practice — the accept loop only drops
        // `write_tx` after the sim has already dropped `world_tx` — but
        // "safe because of what another thread does first" is a thing to
        // write down or check, and checking is one `&&`.
        if write_rx.is_abandoned() && world_rx.is_abandoned() {
            // Everything this process will ever write has been written.
            // `bin/shard.rs` waits on this before it exits, which is what
            // turns a SIGTERM into a save instead of a lost hour.
            ShardStats::raise(&stats.store_stopped);
            return;
        }
        if idle {
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

/// Session + hello, off the accept loop so a slow client can't
/// head-of-line-block accepts. Version gate lives here; the cap gate needs
/// the slot table and lives in `install`.
async fn handshake_task(
    incoming: IncomingSession,
    done_tx: tokio::sync::mpsc::Sender<Handshaken>,
    stats: Arc<ShardStats>,
    require_auth: bool,
    facts_domain: String,
    entitle: crate::entitle::Config,
    min_client: u32,
) {
    let result = tokio::time::timeout(HANDSHAKE_TIMEOUT, async {
        let request = incoming.await.map_err(|_| ())?;
        let connection = request.accept().await.map_err(|_| ())?;
        let (send, mut recv) = connection.accept_bi().await.map_err(|_| ())?;
        let (hello_buf, hello_len) = read_frame(&mut recv).await.ok_or(())?;
        let hello = decode_hello(&hello_buf[..hello_len]).map_err(|_| ())?;
        Ok::<_, ()>((connection, send, recv, hello))
    })
    .await;
    let Ok(Ok((connection, send, mut recv, hello))) = result else {
        ShardStats::bump(&stats.handshake_errors);
        return;
    };
    if hello.proto_ver != PROTO_VER {
        ShardStats::bump(&stats.refused_version);
        spawn_refusal(connection, send, REFUSE_VERSION);
        return;
    }
    // The release floor, second because it is only meaningful once the two
    // sides agree on what the bytes are — `hello.ver` is not a number until
    // `proto_ver` says the layout it was read from is this one.
    //
    // `<`, never `!=`: a client NEWER than the shard is admitted. That is
    // deliberate and it is the direction that keeps a release shippable — a
    // player who updated first must not be locked out of a shard its operator
    // has not restarted yet, and the wire compatibility that would actually
    // break is `PROTO_VER`'s to refuse, one gate up.
    if hello.ver < min_client {
        // The counter is the whole record, and that is this crate's shape
        // rather than a shortcut: there is no logger in `server`'s lib — the
        // bins own every line of output — so a refusal is observable the way
        // every other refusal here is, through `ShardStats`. The shard binary
        // prints the floor and its own build at boot, which is the other half
        // an operator needs to read this number.
        ShardStats::bump(&stats.refused_build);
        spawn_refusal(connection, send, protocol::REFUSE_BUILD);
        return;
    }
    // ---- SIWE, and the nonce never leaves this stack frame --------------
    //
    // The server picks a nonce, the client signs a message containing it,
    // and the server recovers the signer. There is no nonce table to size,
    // expire or sweep: the value lives in this task's local and nothing else
    // in the process can see it, so a signature captured on one connection
    // is worthless on every other — no other connection ever chose it.
    //
    // Sent to a stranger before anything about them exists, which is safe
    // precisely because it is random and means nothing: the server has
    // nothing to lose by challenging someone it will go on to refuse.
    let mut nonce = [0u8; protocol::NONCE_BYTES];
    if getrandom::getrandom(&mut nonce).is_err() {
        // No OS entropy is not a thing to work around with a weaker nonce —
        // a guessable one is a signature anybody can replay. Refuse the
        // connection instead.
        ShardStats::bump(&stats.handshake_errors);
        spawn_refusal(connection, send, protocol::REFUSE_AUTH);
        return;
    }
    let issued_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let challenge = protocol::Challenge { nonce, issued_at };

    let mut send = send;
    let exchange = tokio::time::timeout(HANDSHAKE_TIMEOUT, async {
        write_challenge(&mut send, &challenge).await?;
        let (buf, len) = read_frame(&mut recv).await.ok_or(())?;
        protocol::decode_auth(&buf[..len]).map_err(|_| ())
    })
    .await;
    let Ok(Ok(auth)) = exchange else {
        ShardStats::bump(&stats.handshake_errors);
        return;
    };

    // A guest offers no address and is admitted only where guests are.
    // Everyone else is *proved*: the signature is recovered against the
    // message this server built from the nonce it chose, so the key below is
    // an address somebody holds the private key for and not a string they
    // typed.
    let key = if auth.address.is_guest() {
        None
    } else {
        match crate::auth::verify(&facts_domain, &nonce, issued_at, &auth) {
            Ok(key) => Some(key),
            Err(_) => {
                // Counted, never explained to the caller: which of the four
                // ways a signature can be wrong is not a stranger's business.
                ShardStats::bump(&stats.refused_auth);
                spawn_refusal(connection, send, protocol::REFUSE_AUTH);
                return;
            }
        }
    };
    if require_auth && key.is_none() {
        ShardStats::bump(&stats.refused_auth);
        spawn_refusal(connection, send, protocol::REFUSE_AUTH);
        return;
    }

    // ---- the ticket door ------------------------------------------------
    //
    // Asked only of a PROVED address, and after `require_auth`, because a
    // guest has no wallet to ask about — `config.rs` refuses the armed-over-
    // open pairing at boot so this cannot silently check nobody.
    //
    // On its own blocking thread rather than inline: `ureq` is synchronous
    // and this task shares a tokio worker with every other handshake in
    // flight, so a slow origin would stall strangers who are not waiting on
    // it. `HANDSHAKE_TIMEOUT` already bounds the whole task above, and
    // `entitle::Config::timeout` bounds the call itself.
    //
    // **`Unknown` admits.** The only value that refuses is a definite
    // on-chain zero; an outage must not become a shard nobody can join.
    // `entitle::Verdict::admits` is the one place that decision lives.
    if entitle.armed() {
        if let Some(k) = key.as_ref() {
            // The key IS the wallet: `auth::key_of` builds it from
            // `Address::to_hex`, which is ASCII `0x…` lowercase. Decoded
            // rather than transmuted, and a key that somehow is not utf8
            // becomes an empty string that `entitle::is_wallet` refuses —
            // which is `Unknown`, which admits.
            let wallet = std::str::from_utf8(k.as_bytes())
                .unwrap_or_default()
                .to_string();
            let cfg = entitle.clone();
            let verdict =
                tokio::task::spawn_blocking(move || crate::entitle::check_one(&cfg, &wallet))
                    .await
                    // A panicked or cancelled blocking task is "we could not look",
                    // and reaches the same door every other failure does.
                    .unwrap_or(crate::entitle::Verdict::Unknown);

            match verdict {
                crate::entitle::Verdict::Nope => {
                    ShardStats::bump(&stats.refused_ticket);
                    spawn_refusal(connection, send, protocol::REFUSE_TICKET);
                    return;
                }
                // Counted, not logged: the point of the counter is that a
                // fail-open is VISIBLE to the operator. An address in a log
                // line would be the other thing.
                crate::entitle::Verdict::Unknown => {
                    ShardStats::bump(&stats.entitle_unknown);
                }
                crate::entitle::Verdict::Owns => {}
            }
        }
    }
    let _ = done_tx
        .send(Handshaken {
            connection,
            send,
            recv,
            key,
        })
        .await;
}

/// Claim a slot, build the rings, hand the sim its ends, welcome the
/// client, spawn its reader/writer tasks. Any refusal is posted, never a
/// hang (DESIGN.md §5.9).
#[allow(clippy::too_many_arguments)]
async fn install(
    done: Handshaken,
    facts: &ShardFacts,
    ctrl_tx: &mut rtrb::Producer<Connect>,
    keys: &mut [KeySlot; MAX_PLAYERS],
    store: &SaveStore,
    slots: &Arc<SlotTable>,
    stats: &Arc<ShardStats>,
) {
    let Handshaken {
        connection,
        mut send,
        recv,
        key,
    } = done;
    let Some((slot, generation)) = (0..MAX_PLAYERS).find_map(|s| slots.claim(s).map(|g| (s, g)))
    else {
        ShardStats::bump(&stats.refused_full);
        spawn_refusal(connection, send, REFUSE_FULL);
        return;
    };
    // Player id: slot in the low byte, claim generation above — unique
    // across slot reuse, stable for the connection's life.
    //
    // The generation is masked to 23 bits **so bit 31 is never set**, and
    // that is a wire contract rather than a tidiness: `limits::MOB_ID_TAG`
    // is the high bit and it means "this class-D entity is an animal, draw
    // it as one". `SlotTable::claim` masks to 30, which after the shift
    // reaches bit 37 — so a slot re-claimed 2²³ times would have minted a
    // player id that every client draws as a pig. 8.4 million reconnects on
    // one slot is not a scenario, which is exactly why it would never have
    // been found; the mask costs nothing and `tests/mob.rs` asserts it.
    let id = ((generation & PLAYER_ID_GEN_MASK) << 8) | slot as u32;
    // Who this connection is, for the whole of its life. Recorded before the
    // sim is told anything, so a record coming back from the very first tick
    // already has a key to be filed under.
    keys[slot] = KeySlot {
        key,
        id,
        // Cloned for the sweep. Every other clone of this handle lives in a
        // reader/writer task; this one lives exactly as long as the slot
        // does, and `install` overwrites it on the next tenant.
        conn: Some(connection.clone()),
    };
    // Does this shard remember them? A miss is the ordinary case and it is
    // not a failure: a guest, a first visit, or a shard with no save file.
    let save = key.and_then(|k| store.find(&k));

    let (input_tx, input_rx) = RingBuffer::new(INPUT_RING_CAP);
    let (action_tx, action_rx) = RingBuffer::<ActionMsg>::new(ACTION_RING_CAP);
    let (chat_tx, chat_rx) = RingBuffer::<ChatMsg>::new(CHAT_RING_CAP);
    let (snap_tx, snap_rx) = RingBuffer::<SnapMsg>::new(SNAPSHOT_RING_CAP);
    let (ev_tx, ev_rx) = RingBuffer::<EvMsg>::new(EVENT_RING_CAP);
    let link = Link {
        generation,
        input: input_rx,
        actions: action_rx,
        chats: chat_rx,
        snaps: snap_tx,
        events: ev_tx,
    };
    if ctrl_tx
        .push(Connect {
            slot,
            id,
            save,
            key,
            link,
        })
        .is_err()
    {
        // Control ring full: refuse rather than wait (L4 — no bound is
        // "wait"). The claim reverts; the client may retry.
        slots.unclaim(slot, generation);
        ShardStats::bump(&stats.refused_full);
        spawn_refusal(connection, send, REFUSE_FULL);
        return;
    }
    // `saves_restored` is **not** counted here, and it used to be. A record
    // existing is no longer the same fact as a record being used: since
    // sleepers, the world outranks the store at the door, so a player whose
    // body is still standing is admitted by `Command::Wake` and never reads
    // the record this task just fetched. The sim thread counts it, because
    // the sim thread is where the choice is made (`ShardCore::connect_as`
    // → `Admitted`). The comment this replaced had the right principle — a
    // counter that reports restores which never reached a world is worse
    // than no counter — and this is that principle applied one seam later.

    let welcome = Welcome {
        player_id: id,
        seed: facts.seed,
        tick: ShardStats::get(&stats.current_tick) as u32,
        dev: facts.dev,
    };
    let _ = write_welcome(&mut send, &welcome).await;

    tokio::spawn(reader_task(
        connection.clone(),
        input_tx,
        slots.clone(),
        stats.clone(),
        slot,
        generation,
    ));
    // The bidi recv half stays open for the connection's life: after the
    // hello it is the C→S action lane (craft requests) and the chat lane,
    // demultiplexed by kind.
    tokio::spawn(action_reader_task(
        recv,
        action_tx,
        chat_tx,
        slots.clone(),
        stats.clone(),
        slot,
        generation,
    ));
    // The bidi send half stays open for the connection's life: after the
    // welcome it becomes the reliable event lane (protocol::event).
    tokio::spawn(event_writer_task(
        send,
        ev_rx,
        slots.clone(),
        stats.clone(),
        slot,
        generation,
    ));
    tokio::spawn(writer_task(
        connection,
        snap_rx,
        slots.clone(),
        stats.clone(),
        slot,
        generation,
    ));
}

/// The input half of the datagram lane, split out of the async task the
/// way `accept_chat` is: the **wiring** — peek, decode, the domain
/// refusal, then the ring, and which counter each exit moves — is
/// reachable by a test without a socket.
///
/// The domain refusal is NOW.md §5b's: `buttons` is a full octet on the
/// wire and the sim means only `BTN_MASK`'s four bits, so a frame carrying
/// an unknown bit is a value no client of this version can have built —
/// forged, not mistyped. The whole datagram is refused, exactly as decode
/// already refuses a forged `sel` (`Malformed` refuses the datagram, not
/// the frame), **before** anything reaches the ring: the refusal is
/// ordered ahead of every mutation, so a forged head cannot smuggle a
/// valid tail into the sim (the item-move trap's lesson, applied here).
/// Counted (`input_dg_forged`), dropped, never a disconnect — the datagram
/// lane's loss policy is redundancy, not framing trust.
fn accept_input(
    dg: &[u8],
    input_tx: &mut rtrb::Producer<protocol::InputDatagram>,
    stats: &ShardStats,
) {
    let ok = peek_kind(dg).map(|k| k == KIND_INPUT).unwrap_or(false);
    if !ok {
        ShardStats::bump(&stats.input_dg_bad);
        return;
    }
    match decode_input(dg) {
        Ok(decoded) => {
            if decoded.frames().iter().any(|f| f.buttons & !BTN_MASK != 0) {
                ShardStats::bump(&stats.input_dg_forged);
                return;
            }
            ShardStats::bump(&stats.input_dg_ok);
            if input_tx.push(decoded).is_err() {
                // Ring full: drop newest — the next datagram
                // re-carries the unacked tail (limits.rs).
                ShardStats::bump(&stats.input_ring_drops);
            }
        }
        Err(_) => ShardStats::bump(&stats.input_dg_bad),
    }
}

async fn reader_task(
    connection: Connection,
    mut input_tx: rtrb::Producer<protocol::InputDatagram>,
    slots: Arc<SlotTable>,
    stats: Arc<ShardStats>,
    slot: usize,
    generation: u32,
) {
    while let Ok(dg) = connection.receive_datagram().await {
        accept_input(&dg, &mut input_tx, &stats);
    }
    slots.mark_leaving(slot, generation);
}

/// Reads length-prefixed C→S frames off the bidi stream for the
/// connection's life and demultiplexes them by kind: actions to the
/// action ring, chat to the chat ring.
///
/// The two lanes have deliberately opposite pressure policies. A full
/// **action** ring backpressures: the task stops reading until the sim
/// drains, and QUIC flow control holds the client (limits.rs
/// `ACTION_RING_CAP` — the reliable lane never drops a transaction). A
/// full **chat** ring drops the line, because backpressuring the shared
/// stream on chat would stall the sender's own transactions behind their
/// own typing — the spammer's punishment would land on their building.
///
/// An action frame that fails to decode drops the session: framing trust
/// is gone. A *chat* frame that fails to decode does not — chat text is
/// the one payload a client bug can plausibly malform, and it is counted
/// and swallowed instead (`chat_bad`). Neither ever panics.
/// The chat half of that demux, split out of the async task so the
/// **wiring** — decode, then limiter, then ring, then which counter moves
/// — is reachable by a test without a socket. A limiter that is never
/// called is a limiter that stopped limiting, and that is not something a
/// unit test of the bucket alone can notice.
fn accept_chat(
    frame: &[u8],
    limiter: &mut ChatLimiter,
    now: Instant,
    chat_tx: &mut rtrb::Producer<ChatMsg>,
    stats: &ShardStats,
) {
    let Ok(chat) = decode_chat(frame) else {
        ShardStats::bump(&stats.chat_bad);
        return;
    };
    if !limiter.allow(now) {
        ShardStats::bump(&stats.chat_rate_limited);
        return;
    }
    if chat_tx.push(chat).is_err() {
        ShardStats::bump(&stats.chat_ring_drops);
        return;
    }
    // Counted only once it is actually ringed — `chat_ok` says "decoded
    // and ringed", so a dropped line must not also read as an accepted
    // one.
    ShardStats::bump(&stats.chat_ok);
}

async fn action_reader_task(
    mut recv: RecvStream,
    mut action_tx: rtrb::Producer<ActionMsg>,
    mut chat_tx: rtrb::Producer<ChatMsg>,
    slots: Arc<SlotTable>,
    stats: Arc<ShardStats>,
    slot: usize,
    generation: u32,
) {
    let mut limiter = ChatLimiter::new();
    loop {
        let word = slots.load(slot);
        if state_of(word) != SLOT_LIVE || generation_of(word) != generation {
            return; // slot moved on; sim (or another task) already knows
        }
        let Some((buf, len)) = read_frame(&mut recv).await else {
            break; // stream closed or oversize frame: the session is done
        };
        if peek_kind(&buf[..len]) == Ok(KIND_CHAT) {
            accept_chat(
                &buf[..len],
                &mut limiter,
                Instant::now(),
                &mut chat_tx,
                &stats,
            );
            continue;
        }
        match decode_action(&buf[..len]) {
            Ok(mut act) => {
                ShardStats::bump(&stats.actions_ok);
                loop {
                    match action_tx.push(act) {
                        Ok(()) => break,
                        Err(rtrb::PushError::Full(back)) => {
                            act = back;
                            let word = slots.load(slot);
                            if state_of(word) != SLOT_LIVE || generation_of(word) != generation {
                                return;
                            }
                            tokio::time::sleep(WRITER_POLL).await;
                        }
                    }
                }
            }
            Err(_) => {
                ShardStats::bump(&stats.actions_bad);
                break;
            }
        }
    }
    slots.mark_leaving(slot, generation);
}

async fn writer_task(
    connection: Connection,
    mut snap_rx: rtrb::Consumer<SnapMsg>,
    slots: Arc<SlotTable>,
    stats: Arc<ShardStats>,
    slot: usize,
    generation: u32,
) {
    let mut poll = tokio::time::interval(WRITER_POLL);
    poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        poll.tick().await;
        let word = slots.load(slot);
        if state_of(word) != SLOT_LIVE || generation_of(word) != generation {
            return; // slot moved on; sim (or reader) already knows
        }
        // Drain to the newest — snapshots replace, never accumulate.
        let mut newest: Option<SnapMsg> = None;
        while let Ok(msg) = snap_rx.pop() {
            newest = Some(msg);
        }
        if let Some(msg) = newest {
            // Clamp against the live datagram budget (the trap list:
            // oversize sends fail, and in a browser they fail silently).
            let max = connection.max_datagram_size().unwrap_or(0);
            if msg.bytes().len() > max {
                ShardStats::bump(&stats.snap_send_errors);
                continue;
            }
            match connection.send_datagram(msg.bytes()) {
                Ok(()) => {}
                Err(SendDatagramError::NotConnected) => break,
                Err(_) => ShardStats::bump(&stats.snap_send_errors),
            }
        }
    }
    slots.mark_leaving(slot, generation);
}

/// Drains the per-connection event ring onto the reliable bidi stream —
/// in order, every message (this lane is the one that never drops; the
/// bound lives at the ring, whose refusal the sim converts to a resync).
/// A stalled peer parks this task on stream flow control; the keep-alive/
/// idle-timeout pair reaps dead ones.
async fn event_writer_task(
    mut send: SendStream,
    mut ev_rx: rtrb::Consumer<EvMsg>,
    slots: Arc<SlotTable>,
    stats: Arc<ShardStats>,
    slot: usize,
    generation: u32,
) {
    let mut poll = tokio::time::interval(WRITER_POLL);
    poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        poll.tick().await;
        let word = slots.load(slot);
        if state_of(word) != SLOT_LIVE || generation_of(word) != generation {
            return; // slot moved on; sim (or reader) already knows
        }
        while let Ok(msg) = ev_rx.pop() {
            if write_frame(&mut send, msg.bytes()).await.is_err() {
                ShardStats::bump(&stats.ev_send_errors);
                slots.mark_leaving(slot, generation);
                return;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Stream framing (u16 LE length prefix per message)
// ---------------------------------------------------------------------------

/// One length-prefixed message off a stream: `(buffer, len)`, or None on
/// EOF/oversize (the caller drops the session). The 64 B cap is the
/// server's C→S acceptance — a hello has no business being big.
pub async fn read_frame(recv: &mut RecvStream) -> Option<([u8; MAX_STREAM_MSG_BYTES], usize)> {
    let mut len_buf = [0u8; 2];
    recv.read_exact(&mut len_buf).await.ok()?;
    let len = u16::from_le_bytes(len_buf) as usize;
    if len == 0 || len > MAX_STREAM_MSG_BYTES {
        return None;
    }
    let mut buf = [0u8; MAX_STREAM_MSG_BYTES];
    recv.read_exact(&mut buf[..len]).await.ok()?;
    Some((buf, len))
}

/// The client-side read: one S→C event-lane frame, sized for
/// `MAX_EVENT_MSG_BYTES` (the bot client here; the native client's framer
/// lives in `crates/client/src/lib.rs` with the same cap).
pub async fn read_event_frame(recv: &mut RecvStream) -> Option<([u8; MAX_EVENT_MSG_BYTES], usize)> {
    let mut len_buf = [0u8; 2];
    recv.read_exact(&mut len_buf).await.ok()?;
    let len = u16::from_le_bytes(len_buf) as usize;
    if len == 0 || len > MAX_EVENT_MSG_BYTES {
        return None;
    }
    let mut buf = [0u8; MAX_EVENT_MSG_BYTES];
    recv.read_exact(&mut buf[..len]).await.ok()?;
    Some((buf, len))
}

async fn write_refuse(send: &mut SendStream, code: u8) -> Result<(), ()> {
    let mut payload = [0u8; MAX_STREAM_MSG_BYTES];
    let len = encode_refuse(&Refuse { code }, &mut payload).map_err(|_| ())?;
    write_frame(send, &payload[..len]).await
}

/// Refusals are posted, never hung (DESIGN.md §5.9) — and never block the
/// accept loop. A buffered write dies with the dropped connection, so
/// delivery needs `finish` (retransmit-until-acked) before the drop; that
/// waits on the peer, so it runs detached, bounded by the handshake
/// timeout so a non-acking client can't pin the task.
fn spawn_refusal(connection: Connection, mut send: SendStream, code: u8) {
    tokio::spawn(async move {
        let _ = tokio::time::timeout(HANDSHAKE_TIMEOUT, async {
            write_refuse(&mut send, code).await?;
            send.finish().await.map_err(|_| ())
        })
        .await;
        drop(connection);
    });
}

async fn write_challenge(send: &mut SendStream, msg: &protocol::Challenge) -> Result<(), ()> {
    let mut payload = [0u8; MAX_STREAM_MSG_BYTES];
    let len = protocol::encode_challenge(msg, &mut payload).map_err(|_| ())?;
    write_frame(send, &payload[..len]).await
}

async fn write_welcome(send: &mut SendStream, msg: &Welcome) -> Result<(), ()> {
    let mut payload = [0u8; MAX_STREAM_MSG_BYTES];
    let len = encode_welcome(msg, &mut payload).map_err(|_| ())?;
    write_frame(send, &payload[..len]).await
}

/// **The client half of the handshake, in one place.**
///
/// Hello → read the challenge → answer it → read the welcome. Four steps
/// that have to agree exactly with `handshake_task`, and they are written
/// once because the alternative is what this repo already had: the bot
/// client, three fixtures in `bot_smoke` and the real client each spelling
/// the same exchange out, so a wire turn is five edits and the one that gets
/// missed fails as a hang rather than as a compile error.
///
/// `sign` is asked to sign the SIWE message this shard chose; returning
/// `None` is how a **guest** connects — no address, no signature, admitted
/// only where `require_auth` is off. Bots and every test in this repo are
/// guests, deliberately: giving a load harness a credential would be a lie
/// about what is being measured and a standing reason for somebody to put a
/// real key in a fixture.
pub async fn client_handshake(
    send: &mut SendStream,
    recv: &mut RecvStream,
    domain: &str,
    address: protocol::Address,
    sign: impl FnOnce(&[u8]) -> Option<protocol::Signature>,
) -> Result<Welcome, String> {
    let mut buf = [0u8; MAX_STREAM_MSG_BYTES];
    let len = protocol::encode_hello(
        &protocol::Hello {
            proto_ver: PROTO_VER,
            // The bots are this build, so they state this build — which is
            // also what makes them a real exercise of the floor: a shard with
            // `min_client` above this release refuses its own bot fleet, and
            // that is the correct answer rather than a special case.
            ver: protocol::version::VER,
            build: protocol::version::BUILD,
        },
        &mut buf,
    )
    .map_err(|e| format!("encode hello: {e:?}"))?;
    write_frame(send, &buf[..len])
        .await
        .map_err(|_| "write hello".to_string())?;

    let (frame, n) = read_frame(recv).await.ok_or("no challenge")?;
    // A refusal can arrive here instead of a challenge — a version mismatch
    // is caught before the server ever challenges — so both are handled and
    // the refusal is reported by code rather than as "unexpected".
    match peek_kind(&frame[..n]) {
        Ok(protocol::KIND_REFUSE) => {
            let r = protocol::decode_refuse(&frame[..n]).map_err(|e| format!("refuse: {e:?}"))?;
            return Err(match protocol::refuse_text(r.code) {
                Some(why) => format!("refused: {why}"),
                None => format!("refused: code {}", r.code),
            });
        }
        Ok(protocol::KIND_CHALLENGE) => {}
        other => return Err(format!("expected a challenge, got {other:?}")),
    }
    let challenge =
        protocol::decode_challenge(&frame[..n]).map_err(|e| format!("challenge: {e:?}"))?;

    // The message is built from the domain **this client dialled**, never
    // from anything the server said — that is the whole of SIWE's domain
    // binding, and handing the server the choice would let one shard collect
    // a signature valid at another.
    //
    // **The address goes in before the signing, not after**, and the first
    // version of this function got that backwards: it built the text with
    // `Address::GUEST` and then asked for a signature. The server rebuilds
    // the message from the address the client *claims*, so the two texts
    // would have differed by 42 characters and every real login would have
    // been refused as `WrongSigner` — with the crypto, the nonce and the
    // domain binding all correct. The address is a parameter for that
    // reason: there is no order in which it can be learned late.
    let auth = if address.is_guest() {
        protocol::Auth::default()
    } else {
        let mut text = [0u8; protocol::SIWE_MESSAGE_MAX];
        let tlen = protocol::siwe_message(
            domain,
            &address,
            &challenge.nonce,
            challenge.issued_at,
            &mut text,
        );
        match sign(&text[..tlen.min(protocol::SIWE_MESSAGE_MAX)]) {
            Some(signature) => protocol::Auth { address, signature },
            // The launcher refused, is not running, or handed the player a
            // consent prompt they declined. That is a *guest*, not an error:
            // a shard that takes guests should still take this one.
            None => protocol::Auth::default(),
        }
    };
    let len = protocol::encode_auth(&auth, &mut buf).map_err(|e| format!("encode auth: {e:?}"))?;
    write_frame(send, &buf[..len])
        .await
        .map_err(|_| "write auth".to_string())?;

    let (frame, n) = read_frame(recv).await.ok_or("no handshake reply")?;
    match peek_kind(&frame[..n]) {
        Ok(protocol::KIND_WELCOME) => {
            protocol::decode_welcome(&frame[..n]).map_err(|e| format!("welcome: {e:?}"))
        }
        Ok(protocol::KIND_REFUSE) => {
            let r = protocol::decode_refuse(&frame[..n]).map_err(|e| format!("refuse: {e:?}"))?;
            Err(match protocol::refuse_text(r.code) {
                Some(why) => format!("refused: {why}"),
                None => format!("refused: code {}", r.code),
            })
        }
        other => Err(format!("unexpected handshake reply: {other:?}")),
    }
}

pub async fn write_frame(send: &mut SendStream, payload: &[u8]) -> Result<(), ()> {
    let len = (payload.len() as u16).to_le_bytes();
    send.write_all(&len).await.map_err(|_| ())?;
    send.write_all(payload).await.map_err(|_| ())?;
    Ok(())
}

// ---------------------------------------------------------------------------
// The sim thread
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn sim_thread(
    seed: u64,
    dev_spawn: Option<(f32, f32)>,
    gather: sim_core::gather::GatherContent,
    craft: sim_core::craft::CraftContent,
    build: sim_core::build::BuildContent,
    deploy: sim_core::deploy::DeployContent,
    combat: sim_core::combat::CombatContent,
    backpack: sim_core::backpack::BackpackContent,
    survival: sim_core::survival::SurvivalContent,
    cook: sim_core::oven::CookContent,
    spawn_kit: sim_core::inventory::SpawnKit,
    loot: sim_core::loot::LootContent,
    mobs: sim_core::mob::MobContent,
    catalog: ItemCatalog,
    world_blob: Vec<u8>,
    world_idents: crate::worldfile::Identities,
    world_interval: u64,
    mut ctrl_rx: rtrb::Consumer<Connect>,
    mut grave_tx: rtrb::Producer<Link>,
    mut save_tx: rtrb::Producer<SaveMsg>,
    mut world_tx: rtrb::Producer<WorldMsg>,
    mut world_done_rx: rtrb::Consumer<WorldDone>,
    mut admin_tx: rtrb::Producer<crate::admin::AdminAct>,
    mut log: crate::anomaly::Sink,
    admins: crate::admin::Admins,
    slots: Arc<SlotTable>,
    stats: Arc<ShardStats>,
    shutdown: Arc<AtomicBool>,
) {
    let mut core = ShardCore::new(seed);
    core.world.dev_spawn = dev_spawn;
    core.world.gather = gather;
    core.world.craft = craft;
    core.world.build = build;
    core.world.deploy = deploy;
    core.world.combat = combat;
    core.world.backpack = backpack;
    core.world.survival = survival;
    core.world.cook = cook;
    core.world.spawn_kit = spawn_kit;
    core.world.loot = loot;
    core.world.mob = mobs;
    core.catalog = catalog;
    core.install_admins(admins);
    // The counter sweep's memory, beside the sink it feeds (`anomaly.rs`).
    let mut watch = crate::anomaly::Watch::new();
    // **The load, and this is the only place it may happen**: after the
    // content tables above are installed — the decoder range-checks every
    // row against them — and before the first tick, because a loaded world
    // is the origin of a run and not a mutation inside one (`worldsave.rs`
    // has the wall-5 argument in full).
    //
    // A refusal here cannot happen: `bin/shard.rs` already loaded these same
    // bytes into a trial world before it bound a port, and refused the boot
    // if they did not take. It is still handled rather than unwrapped,
    // because "cannot happen" and "panics the sim thread if it does" is a
    // trade nothing in this file makes.
    if !world_blob.is_empty() {
        match core.world.load(&world_blob) {
            Ok(()) => {
                core.adopt_identities(&world_idents);
                ShardStats::set(&stats.current_tick, core.world.tick);
            }
            Err(_) => ShardStats::bump(&stats.world_load_errors),
        }
    }
    drop(world_blob);
    // The world-save buffer pool. Allocated here, once, and never again:
    // every later save fills one of these and hands the box to the store
    // thread, which hands the box back. Wall 2 counts the tick, and the tick
    // only writes into a buffer that already exists.
    let mut world_pool: Vec<Box<[u8]>> = (0..WORLD_RING_CAP)
        .map(|_| vec![0u8; WORLD_SAVE_MAX_BYTES].into_boxed_slice())
        .collect();
    let mut ident_pool: Vec<Vec<(PlayerKey, u32)>> = (0..WORLD_RING_CAP)
        .map(|_| Vec::with_capacity(MAX_PLAYERS))
        .collect();
    let mut next_world_save = core.world.tick + world_interval.min(u64::MAX / 2);
    let mut links: Vec<Option<Link>> = Vec::with_capacity(MAX_PLAYERS);
    links.resize_with(MAX_PLAYERS, || None);
    let mut links = links.into_boxed_slice();

    let tick_dur = Duration::from_nanos(1_000_000_000 / TICK_HZ as u64);
    let mut next = Instant::now();
    // The boundary loop: clock read and sleep live here and only here;
    // the tick body makes no syscalls (L1 — the sleep IS the boundary).
    while !shutdown.load(Ordering::Relaxed) {
        // Install fresh connections.
        while let Ok(c) = ctrl_rx.pop() {
            if let Some((how, evicted)) = core.connect_as(c.slot, c.id, c.key, c.save) {
                links[c.slot] = Some(c.link);
                ShardStats::bump(&stats.joins);
                // Two-phase eviction, the filing half: this join is about
                // to cost a sleeper its slot, and this record is that body
                // as it stands NOW — raid included — not as its leave left
                // it. Keyed by the sim (the victim has no connection slot
                // for `drain_saves` to resolve), and pushed before the
                // tick below applies the `Evict`, on the same ring every
                // other record rides.
                if let Some((key, save)) = evicted {
                    push_save(&mut save_tx, 0, Some(key), save, &stats);
                }
                // Counted here and nowhere else, because here is the only
                // place that knows which door opened. The accept task can
                // see that a record *exists*; it cannot see that the world
                // still had the body and the record went unread.
                match how {
                    Admitted::TookOver => ShardStats::bump(&stats.takeovers),
                    Admitted::Restored => ShardStats::bump(&stats.saves_restored),
                    Admitted::Fresh => {}
                }
            } else {
                // Command queue refused. Unreachable by arithmetic (ctrl
                // cap + leave cap < queue reserve), but handled: park the
                // link and route it through the normal LEAVING sweep —
                // the sim thread never drops a ring handle itself.
                let generation = c.link.generation;
                links[c.slot] = Some(c.link);
                slots.mark_leaving(c.slot, generation);
                ShardStats::bump(&stats.handshake_errors);
            }
        }
        // Clean up dead connections.
        for slot in 0..MAX_PLAYERS {
            let word = slots.load(slot);
            if state_of(word) != SLOT_LEAVING {
                continue;
            }
            let matches = links[slot]
                .as_ref()
                .map(|l| l.generation == generation_of(word))
                .unwrap_or(false);
            if !matches {
                continue; // install still in flight; next tick
            }
            let link = links[slot].take().expect("checked above");
            let generation = link.generation;
            match grave_tx.push(link) {
                Ok(()) => {
                    // The exact save: taken off the live body, before the
                    // `Leave` is queued and — the part that matters —
                    // **before the slot is freed**. A freed slot is
                    // claimable, so pushing after would race a new tenant's
                    // key into the accept loop's table ahead of this record
                    // (`KeySlot`). The order here is what closes that.
                    if let Some((id, save)) = core.disconnect(slot) {
                        push_save(&mut save_tx, id, None, save, &stats);
                    }
                    slots.free(slot, generation);
                    ShardStats::bump(&stats.leaves);
                }
                Err(rtrb::PushError::Full(link)) => {
                    // Graveyard full: hold the handles, retry next tick.
                    links[slot] = Some(link);
                }
            }
        }
        // Drain inputs, and at most one action per client per tick — the
        // ring buffers the burst, the stream backpressures past it.
        for slot in 0..MAX_PLAYERS {
            if let Some(link) = links[slot].as_mut() {
                while let Ok(dg) = link.input.pop() {
                    core.push_input(slot, &dg);
                }
                if core.wants_action(slot) {
                    if let Ok(act) = link.actions.pop() {
                        core.push_action(slot, act);
                    }
                }
                // Chat drains unconditionally, one line per client per
                // tick: the fan-out always takes the line (it is said or
                // it is lost), so unlike an action there is no hand to
                // check for room.
                if let Ok(chat) = link.chats.pop() {
                    core.push_chat(slot, chat);
                }
            }
        }
        // One step of the autosave sweep, before the tick rather than after
        // it for a reason worth stating: the record is then the state the
        // *previous* tick published, which is the state a client has actually
        // been shown. It costs one tick of staleness against a sweep that is
        // already `MAX_PLAYERS` ticks coarse, and it keeps the save read out
        // of the tick's own borrow of the world.
        if let Some((id, save)) = core.autosave() {
            push_save(&mut save_tx, id, None, save, &stats);
        }
        // Buffers coming home from the store thread.
        while let Ok(done) = world_done_rx.pop() {
            world_pool.push(done.buf);
            ident_pool.push(done.idents);
        }
        // The world, on its cadence. Before the tick for the reason the
        // autosave sweep is: the blob is then the state the *previous* tick
        // published, which is a state clients have actually been shown.
        if core.world.tick >= next_world_save {
            next_world_save = core.world.tick.saturating_add(world_interval);
            take_world_save(
                &mut core,
                &mut world_pool,
                &mut ident_pool,
                &mut world_tx,
                &stats,
            );
        }
        // Tick + publish. `Ops` is the tick's three side channels (admin
        // v0) — the anomaly log, the kick ring, and `/save`'s flag.
        let mut save_now = false;
        let mut ops = crate::core::Ops {
            log: &mut log,
            admin_tx: Some(&mut admin_tx),
            save_now: &mut save_now,
        };
        core.tick(&stats, &mut ops, |lane, slot, bytes| {
            let Some(link) = links[slot].as_mut() else {
                return false;
            };
            match lane {
                Lane::Snapshot => {
                    let mut msg = SnapMsg {
                        len: bytes.len() as u16,
                        buf: [0; sim_core::limits::DATAGRAM_BUDGET_BYTES],
                    };
                    msg.buf[..bytes.len()].copy_from_slice(bytes);
                    if link.snaps.push(msg).is_err() {
                        ShardStats::bump(&stats.snap_ring_skips);
                        return false; // counted; snapshots supersede anyway
                    }
                    true
                }
                Lane::Event => {
                    let mut msg = EvMsg {
                        len: bytes.len() as u16,
                        buf: [0; MAX_EVENT_MSG_BYTES],
                    };
                    msg.buf[..bytes.len()].copy_from_slice(bytes);
                    link.events.push(msg).is_ok()
                }
            }
        });
        ShardStats::bump(&stats.ticks);
        // `/save` asked for the world now rather than at the cadence. Set
        // the deadline to this tick and let the cadence check above take
        // it on the next pass — the admin verb moves the deadline, it does
        // not perform the write, so there is one world-save path and not
        // two (`take_world_save` is still the only writer).
        if save_now {
            next_world_save = core.world.tick;
        }
        // The anomaly log's counter sweep, once a second rather than once a
        // tick: a counter that moved is interesting to the second, and the
        // sweep is ~30 atomic loads (`anomaly::Watch`).
        if core
            .world
            .tick
            .is_multiple_of(sim_core::limits::TICK_HZ as u64)
        {
            watch.sweep(core.world.tick, &stats, &mut log);
        }
        stats.current_tick.store(core.world.tick, Ordering::Relaxed);
        // Three gauges, mirrored off the state rather than accumulated here:
        // the eviction policy lives in `World::seat` and nothing on this
        // thread is told when it fires, so the counter is read, not bumped.
        // `sleepers()` and `connected()` are each an O(MAX_PLAYERS) scan of
        // a 100-element array on a thread that has just done a tick's work —
        // measured against the alternative, which is a second copy of the
        // count that can drift from the array it describes.
        ShardStats::set(&stats.sleepers_evicted, core.world.evictions);
        ShardStats::set(&stats.sleepers, core.world.sleepers() as u64);
        ShardStats::set(&stats.players, core.connected() as u64);

        // Pace (the boundary).
        next += tick_dur;
        let now = Instant::now();
        if now < next {
            std::thread::sleep(next - now);
        } else {
            let behind = now - next;
            if behind > tick_dur * MAX_TICK_BACKLOG {
                let missed = (behind.as_nanos() / tick_dur.as_nanos()) as u64;
                stats.ticks_dropped.fetch_add(missed, Ordering::Relaxed);
                next = now;
            }
        }
    }

    // ---- shutdown, and this is the whole of `NOW.md` §0y item 6 ----------
    //
    // A kill used to cost up to `MAX_PLAYERS` ticks of every player's
    // progress — the autosave sweep's own coarseness, 3.3 s at 30 Hz — plus,
    // since world persistence, up to a whole `world_save_interval_ticks` of
    // everybody's *base*. Neither is necessary on a shutdown the shard can
    // see coming, and an operator restarting a shard to deploy is the most
    // common restart there is.
    //
    // **The world goes first, while everyone is still connected**, and the
    // order is load-bearing rather than tidy: `identities` reads connected
    // clients out of the key table and sleepers out of the sleeper index, so
    // taking the world *after* the disconnects below would find the key
    // table cleared and the `Leave` commands still queued behind a tick that
    // is never going to run — a world full of bodies with nobody's name on
    // them. The encoder puts an awake body to sleep on the way out anyway
    // (`worldsave.rs`), so nothing is lost by saving them awake.
    while let Ok(done) = world_done_rx.pop() {
        world_pool.push(done.buf);
        ident_pool.push(done.idents);
    }
    take_world_save(
        &mut core,
        &mut world_pool,
        &mut ident_pool,
        &mut world_tx,
        &stats,
    );
    // Then every connected player's exact record, which is the same read a
    // leave takes and is what makes a clean restart cost nobody anything.
    for slot in 0..MAX_PLAYERS {
        if let Some((id, save)) = core.disconnect(slot) {
            push_save(&mut save_tx, id, None, save, &stats);
        }
    }
    // Both producers drop here, which is the signal the other two threads
    // are already written to wait for: the accept loop drains `save_rx`
    // until it is abandoned, and the store thread exits when its own ring is
    // dry and abandoned. Nothing is timed and nothing is guessed.
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The chat limiter is the only wall between one client and everyone
    /// else's screen, so its shape is pinned: a burst of `CHAT_BURST`
    /// goes, the next is refused, and tokens come back one per
    /// `CHAT_REFILL` — never faster, and never *all at once* after a long
    /// silence, which is the bug a naive "top up from `last`" writes.
    #[test]
    fn chat_limiter_spends_a_burst_then_drips() {
        let t0 = Instant::now();
        let mut lim = ChatLimiter::new();
        for i in 0..CHAT_BURST {
            assert!(lim.allow(t0), "burst line {i} refused");
        }
        assert!(!lim.allow(t0), "the line past the burst must be refused");

        // Half a refill buys nothing.
        assert!(!lim.allow(t0 + CHAT_REFILL / 2));
        // A whole one buys exactly one.
        assert!(lim.allow(t0 + CHAT_REFILL));
        assert!(!lim.allow(t0 + CHAT_REFILL));

        // A long silence fills the bucket and no further — and, crucially,
        // the next long-silence check does not refill it again from a
        // stale timestamp.
        let quiet = t0 + CHAT_REFILL * 1_000;
        for i in 0..CHAT_BURST {
            assert!(lim.allow(quiet), "post-silence line {i} refused");
        }
        assert!(
            !lim.allow(quiet),
            "a stale accounting timestamp is a limiter that stopped limiting"
        );
        assert!(!lim.allow(quiet + CHAT_REFILL / 2));
    }

    /// And the limiter is actually *wired*: the reader's chat path runs
    /// decode → limit → ring in that order, and each refusal moves its own
    /// counter. Deleting the limiter call would leave the bucket test above
    /// green and this one red, which is the point of testing the wiring
    /// rather than the struct.
    #[test]
    fn accept_chat_decodes_then_limits_then_rings() {
        let stats = ShardStats::default();
        let mut limiter = ChatLimiter::new();
        let (mut tx, mut rx) = RingBuffer::<ChatMsg>::new(CHAT_RING_CAP);
        let t0 = Instant::now();
        let mut frame = [0u8; MAX_STREAM_MSG_BYTES];
        let n = protocol::encode_chat(b"hearth is up", false, &mut frame).expect("encodes");
        // The demux itself: this frame is chat, and an action frame is not.
        assert_eq!(peek_kind(&frame[..n]), Ok(KIND_CHAT));
        let mut act = [0u8; MAX_STREAM_MSG_BYTES];
        let an = protocol::encode_action_cancel(0, &mut act).expect("encodes");
        assert_eq!(peek_kind(&act[..an]), Ok(protocol::KIND_ACTION));

        // The burst is accepted and ringed; everything past it is refused
        // by the limiter, not by the ring.
        for _ in 0..CHAT_BURST {
            accept_chat(&frame[..n], &mut limiter, t0, &mut tx, &stats);
        }
        assert_eq!(ShardStats::get(&stats.chat_ok), CHAT_BURST as u64);
        assert_eq!(ShardStats::get(&stats.chat_rate_limited), 0);
        for _ in 0..5 {
            accept_chat(&frame[..n], &mut limiter, t0, &mut tx, &stats);
        }
        assert_eq!(
            ShardStats::get(&stats.chat_ok),
            CHAT_BURST as u64,
            "a line past the burst was ringed — the limiter is not wired in"
        );
        assert_eq!(ShardStats::get(&stats.chat_rate_limited), 5);

        // Nothing but the burst reached the ring.
        let mut ringed = 0;
        while rx.pop().is_ok() {
            ringed += 1;
        }
        assert_eq!(ringed, CHAT_BURST as usize);

        // A full ring drops the line and counts it as dropped, never as ok.
        let (mut full_tx, _full_rx) = RingBuffer::<ChatMsg>::new(CHAT_RING_CAP);
        let mut fresh = ChatLimiter::new();
        let ok_before = ShardStats::get(&stats.chat_ok);
        for i in 0..CHAT_RING_CAP + 2 {
            accept_chat(
                &frame[..n],
                &mut fresh,
                t0 + CHAT_REFILL * i as u32,
                &mut full_tx,
                &stats,
            );
        }
        assert_eq!(
            ShardStats::get(&stats.chat_ok) - ok_before,
            CHAT_RING_CAP as u64
        );
        assert_eq!(ShardStats::get(&stats.chat_ring_drops), 2);

        // Bytes that fail the text rules are counted and swallowed — the
        // session is not dropped the way a bad action frame drops it.
        let bad = [KIND_CHAT as u8 | 0x08, 0xff, 0xff];
        accept_chat(&bad, &mut fresh, t0, &mut tx, &stats);
        assert_eq!(ShardStats::get(&stats.chat_bad), 1);
    }

    /// NOW.md §5b: the wire carries `buttons` as a full octet and the sim
    /// means only `BTN_MASK` — a frame carrying an unknown bit is refused
    /// at the accept boundary, through the **real** encode → decode path,
    /// and nothing of that datagram reaches the ring (the refusal is
    /// ordered before the mutation). The value just inside the boundary —
    /// every meaningful bit at once — still crosses.
    #[test]
    fn accept_input_refuses_buttons_the_sim_cannot_mean() {
        use protocol::{encode_input, InputDatagram};
        use sim_core::input::InputFrame;

        let stats = ShardStats::default();
        let (mut tx, mut rx) = RingBuffer::<InputDatagram>::new(INPUT_RING_CAP);
        let encode = |buttons: u8| {
            let mut dg = InputDatagram::new(0, 0, 0);
            dg.push(InputFrame {
                buttons,
                ..InputFrame::default()
            })
            .expect("one frame fits");
            let mut buf = [0u8; 256];
            let n = encode_input(&dg, &mut buf).expect("encodes");
            (buf, n)
        };

        // Just inside: all four meaningful bits at once.
        let (buf, n) = encode(BTN_MASK);
        accept_input(&buf[..n], &mut tx, &stats);
        assert_eq!(ShardStats::get(&stats.input_dg_ok), 1);
        assert_eq!(ShardStats::get(&stats.input_dg_forged), 0);
        let ringed = rx.pop().expect("the in-domain datagram is ringed");
        assert_eq!(ringed.frames()[0].buttons, BTN_MASK);

        // Just outside: bit 4 — the octet's first meaningless bit. The
        // encoder writes it happily (the field is 8 wide since v0), which
        // is exactly the slack being closed here.
        let (buf, n) = encode(BTN_MASK | 0x10);
        accept_input(&buf[..n], &mut tx, &stats);
        assert_eq!(ShardStats::get(&stats.input_dg_forged), 1);
        assert_eq!(
            ShardStats::get(&stats.input_dg_ok),
            1,
            "a forged datagram must not also count as accepted"
        );
        assert_eq!(
            ShardStats::get(&stats.input_dg_bad),
            0,
            "forged is not malformed — the bytes decode fine"
        );
        assert!(
            rx.pop().is_err(),
            "the forged datagram reached the ring — the refusal is not \
             ordered before the mutation"
        );

        // A valid tail does not ride a forged head: one datagram, two
        // frames, only the second in-domain — the whole datagram drops,
        // exactly as decode's own `sel` refusal drops it.
        let mut dg = InputDatagram::new(0, 0, 0);
        dg.push(InputFrame {
            buttons: 0x80,
            ..InputFrame::default()
        })
        .expect("first frame fits");
        dg.push(InputFrame {
            seq: 1,
            buttons: BTN_MASK,
            ..InputFrame::default()
        })
        .expect("second frame fits");
        let mut buf = [0u8; 256];
        let n = encode_input(&dg, &mut buf).expect("encodes");
        accept_input(&buf[..n], &mut tx, &stats);
        assert_eq!(ShardStats::get(&stats.input_dg_forged), 2);
        assert!(rx.pop().is_err(), "no frame of a forged datagram survives");
    }
}
