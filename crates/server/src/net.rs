//! The I/O shell around `ShardCore`: wtransport termination, the session
//! hello (DESIGN.md §5.9), per-connection tasks, and the pinned 30 Hz sim
//! thread. Transport config of record per NETCODE.md §2.2: keep-alive
//! 10 s, idle 30 s, 64 KiB datagram send buffer, `send_datagram`
//! (drop-oldest) and never `_wait`.

use crate::config::ShardConfig;
use crate::core::{Lane, ShardCore};
use crate::slot::{
    generation_of, state_of, Connect, EvMsg, Link, SlotTable, SnapMsg, SLOT_LEAVING, SLOT_LIVE,
};
use crate::stats::ShardStats;
use protocol::{
    decode_hello, decode_input, encode_refuse, encode_welcome, peek_kind, ItemCatalog, Refuse,
    Welcome, KIND_INPUT, MAX_EVENT_MSG_BYTES, MAX_STREAM_MSG_BYTES, PROTO_VER, REFUSE_FULL,
    REFUSE_VERSION,
};
use rtrb::RingBuffer;
use sim_core::limits::{
    CTRL_RING_CAP, EVENT_RING_CAP, GRAVEYARD_RING_CAP, INPUT_RING_CAP, MAX_PLAYERS,
    SNAPSHOT_RING_CAP, TICK_HZ,
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
/// `gather` and `catalog` are the content bake (CLAUDE.md wall 7) — data
/// the world runs on, handed over before the first tick like the seed.
pub async fn spawn_shard(
    cfg: ShardConfig,
    gather: sim_core::gather::GatherContent,
    catalog: ItemCatalog,
) -> Result<ShardHandle, String> {
    let identity = Identity::self_signed(["localhost", "127.0.0.1", "::1"])
        .map_err(|e| format!("self-signed identity: {e}"))?;
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
                    seed, dev_spawn, gather, catalog, ctrl_rx, grave_tx, slots, stats, shutdown,
                )
            })
            .map_err(|e| format!("sim thread spawn: {e}"))?;
    }

    tokio::spawn(accept_loop(
        endpoint,
        cfg.seed,
        ctrl_tx,
        grave_rx,
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

/// What a handshake task hands back once the client said a valid hello.
struct Handshaken {
    connection: Connection,
    send: SendStream,
}

async fn accept_loop(
    endpoint: Endpoint<Server>,
    seed: u64,
    mut ctrl_tx: rtrb::Producer<Connect>,
    mut grave_rx: rtrb::Consumer<Link>,
    slots: Arc<SlotTable>,
    stats: Arc<ShardStats>,
    shutdown: Arc<AtomicBool>,
) {
    // Net-side plumbing between handshake tasks and this loop; the sim
    // thread never touches it (L3 is about the sim thread, not tokio).
    let (done_tx, mut done_rx) = tokio::sync::mpsc::channel::<Handshaken>(MAX_PLAYERS);
    let mut sweep = tokio::time::interval(Duration::from_millis(100));
    loop {
        tokio::select! {
            incoming = endpoint.accept() => {
                let stats = stats.clone();
                let done_tx = done_tx.clone();
                tokio::spawn(handshake_task(incoming, done_tx, stats));
            }
            Some(done) = done_rx.recv() => {
                install(done, seed, &mut ctrl_tx, &slots, &stats).await;
            }
            _ = sweep.tick() => {
                while let Ok(link) = grave_rx.pop() {
                    drop(link); // net side deallocates, never the sim
                }
                if shutdown.load(Ordering::Relaxed) {
                    endpoint.close(wtransport::VarInt::from_u32(0), b"shutdown");
                    return;
                }
            }
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
) {
    let result = tokio::time::timeout(HANDSHAKE_TIMEOUT, async {
        let request = incoming.await.map_err(|_| ())?;
        let connection = request.accept().await.map_err(|_| ())?;
        let (send, mut recv) = connection.accept_bi().await.map_err(|_| ())?;
        let (hello_buf, hello_len) = read_frame(&mut recv).await.ok_or(())?;
        let hello = decode_hello(&hello_buf[..hello_len]).map_err(|_| ())?;
        Ok::<_, ()>((connection, send, hello))
    })
    .await;
    let Ok(Ok((connection, send, hello))) = result else {
        ShardStats::bump(&stats.handshake_errors);
        return;
    };
    if hello.proto_ver != PROTO_VER {
        ShardStats::bump(&stats.refused_version);
        spawn_refusal(connection, send, REFUSE_VERSION);
        return;
    }
    let _ = done_tx.send(Handshaken { connection, send }).await;
}

/// Claim a slot, build the rings, hand the sim its ends, welcome the
/// client, spawn its reader/writer tasks. Any refusal is posted, never a
/// hang (DESIGN.md §5.9).
async fn install(
    done: Handshaken,
    seed: u64,
    ctrl_tx: &mut rtrb::Producer<Connect>,
    slots: &Arc<SlotTable>,
    stats: &Arc<ShardStats>,
) {
    let Handshaken {
        connection,
        mut send,
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

    let (input_tx, input_rx) = RingBuffer::new(INPUT_RING_CAP);
    let (snap_tx, snap_rx) = RingBuffer::<SnapMsg>::new(SNAPSHOT_RING_CAP);
    let (ev_tx, ev_rx) = RingBuffer::<EvMsg>::new(EVENT_RING_CAP);
    let link = Link {
        generation,
        input: input_rx,
        snaps: snap_tx,
        events: ev_tx,
    };
    if ctrl_tx.push(Connect { slot, id, link }).is_err() {
        // Control ring full: refuse rather than wait (L4 — no bound is
        // "wait"). The claim reverts; the client may retry.
        slots.unclaim(slot, generation);
        ShardStats::bump(&stats.refused_full);
        spawn_refusal(connection, send, REFUSE_FULL);
        return;
    }

    let welcome = Welcome {
        player_id: id,
        seed,
        tick: ShardStats::get(&stats.current_tick) as u32,
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
    catalog: ItemCatalog,
    mut ctrl_rx: rtrb::Consumer<Connect>,
    mut grave_tx: rtrb::Producer<Link>,
    slots: Arc<SlotTable>,
    stats: Arc<ShardStats>,
    shutdown: Arc<AtomicBool>,
) {
    let mut core = ShardCore::new(seed);
    core.world.dev_spawn = dev_spawn;
    core.world.gather = gather;
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
            if core.connect(c.slot, c.id) {
                links[c.slot] = Some(c.link);
                ShardStats::bump(&stats.joins);
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
                    core.disconnect(slot);
                    slots.free(slot, generation);
                    ShardStats::bump(&stats.leaves);
                }
                Err(rtrb::PushError::Full(link)) => {
                    // Graveyard full: hold the handles, retry next tick.
                    links[slot] = Some(link);
                }
            }
        }
        // Drain inputs.
        for slot in 0..MAX_PLAYERS {
            if let Some(link) = links[slot].as_mut() {
                while let Ok(dg) = link.input.pop() {
                    core.push_input(slot, &dg);
                }
            }
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
