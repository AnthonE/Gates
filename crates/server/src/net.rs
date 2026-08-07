//! The I/O shell around `ShardCore`: wtransport termination, the session
//! hello (DESIGN.md §5.9), per-connection tasks, and the pinned 30 Hz sim
//! thread. Transport config of record per NETCODE.md §2.2: keep-alive
//! 10 s, idle 30 s, 64 KiB datagram send buffer, `send_datagram`
//! (drop-oldest) and never `_wait`.

use crate::config::ShardConfig;
use crate::core::{Admitted, Lane, ShardCore};
use crate::slot::{
    generation_of, state_of, Connect, EvMsg, Link, SaveMsg, SlotTable, SnapMsg, WriteMsg,
    SLOT_LEAVING, SLOT_LIVE,
};
use crate::stats::ShardStats;
use crate::store::{PlayerKey, SaveFile, SaveStore, Saves};
use protocol::{
    decode_action, decode_chat, decode_hello, decode_input, encode_refuse, encode_welcome,
    peek_kind, ActionMsg, ChatMsg, ItemCatalog, Refuse, Welcome, KIND_CHAT, KIND_INPUT,
    MAX_EVENT_MSG_BYTES, MAX_STREAM_MSG_BYTES, PROTO_VER, REFUSE_FULL, REFUSE_VERSION,
};
use rtrb::RingBuffer;
use sim_core::limits::{
    ACTION_RING_CAP, CHAT_RING_CAP, CTRL_RING_CAP, EVENT_RING_CAP, GRAVEYARD_RING_CAP,
    INPUT_RING_CAP, MAX_PLAYERS, SAVE_RING_CAP, SNAPSHOT_RING_CAP, TICK_HZ,
};
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
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
/// Writer poll cadence: how often per-connection outbound rings drain.
/// Latency floor for a snapshot, far under the 66 ms snapshot interval.
const WRITER_POLL: Duration = Duration::from_millis(2);
/// Sim thread abandons backlog beyond this many ticks (a debugger pause,
/// a VM freeze) instead of sprinting to catch up.
const MAX_TICK_BACKLOG: u32 = 8;

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
    loot: sim_core::loot::LootContent,
    catalog: ItemCatalog,
    saves: Saves,
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

    {
        let stats = stats.clone();
        let shutdown = shutdown.clone();
        let slots = slots.clone();
        let seed = cfg.seed;
        let dev_spawn = cfg.dev_spawn;
        std::thread::Builder::new()
            .name("sim".into())
            .spawn(move || {
                sim_thread(
                    seed, dev_spawn, gather, craft, build, deploy, combat, backpack, survival,
                    loot, catalog, ctrl_rx, grave_tx, save_tx, slots, stats, shutdown,
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
            .spawn(move || store_thread(file, write_rx, stats))
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
        },
        ctrl_tx,
        grave_rx,
        save_rx,
        write_tx,
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
#[derive(Clone, Copy)]
struct ShardFacts {
    seed: u64,
    dev: bool,
    /// `shard.toml require_auth`. See `config.rs` for why it is a knob.
    require_auth: bool,
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
#[derive(Clone, Copy, Default)]
struct KeySlot {
    key: Option<PlayerKey>,
    id: u32,
}

#[allow(clippy::too_many_arguments)]
async fn accept_loop(
    endpoint: Endpoint<Server>,
    facts: ShardFacts,
    mut ctrl_tx: rtrb::Producer<Connect>,
    mut grave_rx: rtrb::Consumer<Link>,
    mut save_rx: rtrb::Consumer<SaveMsg>,
    mut write_tx: rtrb::Producer<WriteMsg>,
    mut store: SaveStore,
    slots: Arc<SlotTable>,
    stats: Arc<ShardStats>,
    shutdown: Arc<AtomicBool>,
) {
    // Net-side plumbing between handshake tasks and this loop; the sim
    // thread never touches it (L3 is about the sim thread, not tokio).
    let (done_tx, mut done_rx) = tokio::sync::mpsc::channel::<Handshaken>(MAX_PLAYERS);
    let mut keys = [KeySlot::default(); MAX_PLAYERS];
    let mut sweep = tokio::time::interval(Duration::from_millis(100));
    loop {
        tokio::select! {
            incoming = endpoint.accept() => {
                let stats = stats.clone();
                let done_tx = done_tx.clone();
                tokio::spawn(handshake_task(incoming, done_tx, stats, facts.require_auth));
            }
            Some(done) = done_rx.recv() => {
                // Before the claim, never after: a record for the slot's
                // previous tenant has to be filed under the key it was
                // written for, and installing first would overwrite that key.
                drain_saves(&mut save_rx, &mut write_tx, &mut store, &keys, &stats);
                install(done, facts, &mut ctrl_tx, &mut keys, &store, &slots, &stats).await;
            }
            _ = sweep.tick() => {
                while let Ok(link) = grave_rx.pop() {
                    drop(link); // net side deallocates, never the sim
                }
                drain_saves(&mut save_rx, &mut write_tx, &mut store, &keys, &stats);
                if shutdown.load(Ordering::Relaxed) {
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
fn push_save(
    save_tx: &mut rtrb::Producer<SaveMsg>,
    id: u32,
    save: sim_core::persist::PlayerSave,
    stats: &Arc<ShardStats>,
) {
    if save_tx.push(SaveMsg { id, save }).is_err() {
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
        // `id = generation << 8 | slot` (see `install`), so the slot is the
        // low byte and the generation check is the id equality below.
        let slot = (msg.id & 0xFF) as usize;
        if slot >= MAX_PLAYERS || keys[slot].id != msg.id {
            continue;
        }
        let Some(key) = keys[slot].key else {
            continue; // a guest: admitted, remembered by nobody
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
    stats: Arc<ShardStats>,
) {
    loop {
        let mut idle = true;
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
        if write_rx.is_abandoned() {
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
    let Ok(Ok((connection, send, recv, hello))) = result else {
        ShardStats::bump(&stats.handshake_errors);
        return;
    };
    if hello.proto_ver != PROTO_VER {
        ShardStats::bump(&stats.refused_version);
        spawn_refusal(connection, send, REFUSE_VERSION);
        return;
    }
    // Admission, and now also identity — the same call answers both, and
    // that is what made persistence possible without settling the identity
    // question (`auth.rs`). **The token is still not validated here and this
    // is the honest half of the slice**: `validate_session` is the seam where
    // the shard asks scry whether the token is good, and it is a stub that
    // resolves any non-empty token to itself until that API is wired
    // (`auth.rs` in `protocol` has the model). So today `require_auth = true`
    // proves a client CARRIED a credential, not that the credential is real —
    // which is why the default is `false` and why arming it on a public shard
    // waits for the validator. Named rather than implied, because a shard
    // operator reading `require_auth` would otherwise assume more than it
    // does.
    let key = crate::auth::validate_session(&hello.token);
    if require_auth && key.is_none() {
        ShardStats::bump(&stats.refused_auth);
        spawn_refusal(connection, send, protocol::REFUSE_AUTH);
        return;
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
    facts: ShardFacts,
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
    let id = (generation << 8) | slot as u32;
    // Who this connection is, for the whole of its life. Recorded before the
    // sim is told anything, so a record coming back from the very first tick
    // already has a key to be filed under.
    keys[slot] = KeySlot { key, id };
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

async fn reader_task(
    connection: Connection,
    mut input_tx: rtrb::Producer<protocol::InputDatagram>,
    slots: Arc<SlotTable>,
    stats: Arc<ShardStats>,
    slot: usize,
    generation: u32,
) {
    while let Ok(dg) = connection.receive_datagram().await {
        let ok = peek_kind(&dg).map(|k| k == KIND_INPUT).unwrap_or(false);
        if !ok {
            ShardStats::bump(&stats.input_dg_bad);
            continue;
        }
        match decode_input(&dg) {
            Ok(decoded) => {
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
/// `MAX_EVENT_MSG_BYTES` (bots and native harnesses; the browser's framer
/// lives in `web/src/net.js` with the same cap).
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

async fn write_welcome(send: &mut SendStream, msg: &Welcome) -> Result<(), ()> {
    let mut payload = [0u8; MAX_STREAM_MSG_BYTES];
    let len = encode_welcome(msg, &mut payload).map_err(|_| ())?;
    write_frame(send, &payload[..len]).await
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
    loot: sim_core::loot::LootContent,
    catalog: ItemCatalog,
    mut ctrl_rx: rtrb::Consumer<Connect>,
    mut grave_tx: rtrb::Producer<Link>,
    mut save_tx: rtrb::Producer<SaveMsg>,
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
    core.world.loot = loot;
    core.catalog = catalog;
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
            if let Some(how) = core.connect_as(c.slot, c.id, c.key, c.save) {
                links[c.slot] = Some(c.link);
                ShardStats::bump(&stats.joins);
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
                        push_save(&mut save_tx, id, save, &stats);
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
            push_save(&mut save_tx, id, save, &stats);
        }
        // Tick + publish.
        core.tick(&stats, |lane, slot, bytes| {
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
        stats.current_tick.store(core.world.tick, Ordering::Relaxed);
        // Two gauges, mirrored off the world rather than accumulated here:
        // the eviction policy lives in `World::seat` and nothing on this
        // thread is told when it fires, so the counter is read, not bumped.
        // `sleepers()` is an O(MAX_PLAYERS) scan of a 100-element array on
        // a thread that has just done a tick's work — measured against the
        // alternative, which is a second copy of the count that can drift
        // from the array it describes.
        ShardStats::set(&stats.sleepers_evicted, core.world.evictions);
        ShardStats::set(&stats.sleepers, core.world.sleepers() as u64);

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
}
