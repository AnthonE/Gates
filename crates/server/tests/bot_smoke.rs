//! The 50-bot smoke (DESIGN.md §11 M0): real wtransport connections
//! against a real shard on loopback — handshake, inputs, snapshots, acks,
//! deltas, all through the actual net stack. Asserts are structural
//! (connected counts, decode integrity, ack loop engagement), never timed:
//! this box shares cores, so wall-clock is not a claim here.

use server::botclient::{bot_endpoint, run_bot};
use server::config::ShardConfig;
use server::net::spawn_shard;
use server::stats::ShardStats;
use std::time::Duration;

const BOTS: usize = 50;

/// The shipped content set, baked — the smoke runs the same boot path the
/// shard binary does, so gather and the catalog are live under the herd.
fn baked_content() -> (
    sim_core::gather::GatherContent,
    sim_core::craft::CraftContent,
    protocol::ItemCatalog,
) {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content");
    let content = content::Content::load_dir(&dir).expect("shipped content loads");
    let gather = content.bake_gather().expect("shipped content bakes");
    let craft = content.bake_craft().expect("shipped recipes bake");
    let catalog = server::net::bake_catalog(&content).expect("shipped catalog bakes");
    (gather, craft, catalog)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_bot_smoke_50() {
    let (gather, craft, catalog) = baked_content();
    let handle = spawn_shard(ShardConfig::ephemeral(0xC0FFEE), gather, craft, catalog)
        .await
        .expect("shard boots");
    let addr = handle.local_addr;

    let endpoint = std::sync::Arc::new(bot_endpoint().expect("client endpoint"));
    let mut tasks = Vec::with_capacity(BOTS);
    for i in 0..BOTS {
        let endpoint = endpoint.clone();
        tasks.push(tokio::spawn(async move {
            run_bot(&endpoint, addr, i as u64, Duration::from_secs(4)).await
        }));
        // Stagger the TLS herd; the walk windows still overlap heavily.
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let mut reports = Vec::with_capacity(BOTS);
    for (i, t) in tasks.into_iter().enumerate() {
        let report = t
            .await
            .expect("bot task join")
            .unwrap_or_else(|e| panic!("bot {i} failed: {e}"));
        reports.push(report);
    }

    // Every bot got welcomed with the shard's seed and a unique id.
    let mut ids: Vec<u32> = reports.iter().map(|r| r.player_id).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), BOTS, "player ids must be unique");
    for r in &reports {
        let w = r.welcome.expect("welcomed");
        assert_eq!(w.seed, 0xC0FFEE, "welcome carries the world seed");
    }

    // The pipeline moved: snapshots applied, own entity seen, the input
    // ack loop engaged (last_executed advanced from 0).
    for (i, r) in reports.iter().enumerate() {
        assert!(
            r.snapshots_applied >= 10,
            "bot {i}: only {} snapshots applied",
            r.snapshots_applied
        );
        assert!(r.own_updates > 0, "bot {i}: never saw own entity");
        assert!(
            r.last_executed_seq > 0,
            "bot {i}: server never executed an input"
        );
        assert_eq!(r.decode_errors, 0, "bot {i}: decode errors");
        assert_eq!(r.no_baseline, 0, "bot {i}: baseline anomalies");
        assert!(r.inputs_sent > 0, "bot {i}: sent no inputs");
        // The event lane spoke: with content loaded every client gets at
        // least the catalog drip, and every message must decode.
        assert!(r.events_received > 0, "bot {i}: event lane silent");
        assert_eq!(r.event_decode_errors, 0, "bot {i}: event decode errors");
    }

    // Interest works: 50 spawns scattered over the island won't all share
    // AOI, but at least some bots must see a neighbour.
    let bots_with_others = reports.iter().filter(|r| r.max_entities_seen > 1).count();
    assert!(
        bots_with_others > 0,
        "no bot ever saw another entity — AOI or spawn scatter broken"
    );

    // The ack→baseline loop closed: deltas engaged for most bots (a bot
    // whose whole window rode zero-states would mean acks never landed).
    let bots_with_deltas = reports.iter().filter(|r| r.delta_snapshots > 0).count();
    assert!(
        bots_with_deltas >= BOTS * 8 / 10,
        "deltas engaged for only {bots_with_deltas}/{BOTS} bots"
    );

    // Server-side integrity: all bots joined, nothing malformed arrived,
    // and the encoder never met an out-of-range body.
    let s = &handle.stats;
    assert_eq!(ShardStats::get(&s.joins), BOTS as u64, "joins");
    assert_eq!(ShardStats::get(&s.input_dg_bad), 0, "malformed inputs");
    assert_eq!(ShardStats::get(&s.encode_range_errors), 0, "range errors");
    assert_eq!(ShardStats::get(&s.forced_resyncs), 0, "forced resyncs");
    assert!(ShardStats::get(&s.ticks) > 60, "sim thread ticked");

    handle
        .shutdown
        .store(true, std::sync::atomic::Ordering::Relaxed);
}

/// A wrong protocol version is refused with a posted reason, not a hang
/// (DESIGN.md §5.9) — exercised through the real handshake path.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_version_gate_refuses() {
    use protocol::{encode_hello, Hello, MAX_STREAM_MSG_BYTES};
    use server::net::{read_frame, write_frame};

    let (gather, craft, catalog) = baked_content();
    let handle = spawn_shard(ShardConfig::ephemeral(7), gather, craft, catalog)
        .await
        .expect("boots");
    let endpoint = bot_endpoint().expect("endpoint");
    let connection = endpoint
        .connect(&format!("https://{}", handle.local_addr))
        .await
        .expect("connects");
    let opening = connection.open_bi().await.expect("open_bi");
    let (mut send, mut recv) = opening.await.expect("bi");
    let mut buf = [0u8; MAX_STREAM_MSG_BYTES];
    let len = encode_hello(&Hello { proto_ver: 999 }, &mut buf).expect("encode");
    write_frame(&mut send, &buf[..len]).await.expect("write");
    let (reply, reply_len) = tokio::time::timeout(Duration::from_secs(5), read_frame(&mut recv))
        .await
        .expect("reply inside 5 s")
        .expect("a refusal frame, not silence");
    let refuse = protocol::decode_refuse(&reply[..reply_len]).expect("refuse decodes");
    assert_eq!(refuse.code, protocol::REFUSE_VERSION);
    assert_eq!(ShardStats::get(&handle.stats.refused_version), 1);
    handle
        .shutdown
        .store(true, std::sync::atomic::Ordering::Relaxed);
}
