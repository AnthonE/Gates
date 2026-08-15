//! The transport tells the truth about itself (`NETCODE.md` §2.2).
//!
//! Three of §2.2's "config of record" rows described things that did not
//! exist until 2026-08-15 — a socket-buffer request, a congestion-control
//! selection, and a QUIC-level admission gate — and nothing anywhere read a
//! single transport statistic, so every congestion claim in that document
//! was unfalsifiable. This suite is the half that could not be unit-tested:
//! it boots a real shard, dials it over the real net stack, and asserts the
//! numbers moved.
//!
//! Asserts are on **observable state bounded by a timeout**, never on
//! elapsed time (`CLAUDE.md`: a gate that waits on a clock is not a gate on
//! this box).

use server::botclient::{bot_endpoint, run_bot};
use server::config::ShardConfig;
use server::net::spawn_shard;
use server::stats::ShardStats;
use server::store::Saves;
use std::time::Duration;

fn baked_content() -> server::net::SimTables {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content");
    let content = content::Content::load_dir(&dir).expect("shipped content loads");
    let mut tables = server::net::bake_all(&content).expect("shipped content bakes");
    tables.spawn_kit = sim_core::inventory::SpawnKit::EMPTY;
    tables
}

/// **The gate that would have caught the whole gap.**
///
/// Before this pass `writer_task` held a live `Connection` and never asked
/// it anything, so a shard could be losing a third of its packets and no
/// number anywhere would move. The claim here is deliberately weak in one
/// direction and strong in the other: it does *not* assert a loss rate (that
/// is the path's business, not ours, and loopback loses nothing), it asserts
/// that the sampling path **runs and reaches `ShardStats`** — because the
/// failure this guards is not a wrong number, it is no number at all.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_transport_reports_its_own_numbers() {
    let tables = baked_content();
    let handle = spawn_shard(
        ShardConfig::ephemeral(0xC0FFEE),
        tables,
        Saves::off(),
        server::worldfile::WorldBoot::off(),
    )
    .await
    .expect("shard boots");
    let addr = handle.local_addr;

    // The socket buffer readback happens at bind, so it is already true
    // before anybody connects. Both numbers must be recorded — see the unit
    // test in `net.rs` for why we assert the *pair* and not the size.
    let asked = ShardStats::get(&handle.stats.net_rcvbuf_asked);
    let got = ShardStats::get(&handle.stats.net_rcvbuf_bytes);
    assert!(asked > 0, "the shard must record what it asked the OS for");
    assert!(
        got > 0,
        "the shard must read back what the OS granted — an unread setsockopt \
         is the exact failure this row existed to prevent"
    );

    let endpoint = std::sync::Arc::new(bot_endpoint().expect("client endpoint"));
    let bot =
        tokio::spawn(
            async move { run_bot(&endpoint, addr, 1, Duration::from_secs(3), None).await },
        );

    // Observable state, bounded: the writer samples on its first poll, so
    // this resolves in milliseconds on a healthy shard and fails loudly
    // rather than hanging if the sampling path is gone.
    tokio::time::timeout(Duration::from_secs(20), async {
        while ShardStats::get(&handle.stats.net_sent_packets) == 0 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("no transport telemetry ever reached ShardStats — the writer_task sample is gone");

    bot.await.expect("bot task join").expect("bot ran");

    // The path MTU is the one reading that proves we are talking to quinn's
    // real path state rather than to a zeroed struct: it can only be
    // populated by the transport itself, and QUIC's floor is 1200.
    let mtu = ShardStats::get(&handle.stats.net_mtu_last);
    assert!(
        mtu >= 1200,
        "path MTU {mtu} is below QUIC's own floor — the sample is not reading a live path"
    );
    // Our design number must still fit inside what the path reports, which
    // is the invariant `DATAGRAM_BUDGET_BYTES` is chosen against.
    assert!(
        mtu as usize >= sim_core::limits::DATAGRAM_BUDGET_BYTES,
        "the datagram budget does not fit the path MTU the transport reports"
    );
}

/// A shard configured for BBR must actually boot on BBR.
///
/// The knob is worth nothing if it parses and is then dropped on the way to
/// quinn — which is precisely what "config of record" meant for three rows
/// of §2.2 until this pass. This cannot read the controller back out of
/// quinn (it exposes no getter), so it asserts the reachable half: the
/// selection survives config into a booted shard that serves traffic.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_shard_boots_on_the_controller_it_was_told_to_use() {
    let mut cfg = ShardConfig::ephemeral(0xBB4);
    cfg.congestion = server::config::Congestion::Bbr;
    let handle = spawn_shard(
        cfg,
        baked_content(),
        Saves::off(),
        server::worldfile::WorldBoot::off(),
    )
    .await
    .expect("a shard configured for bbr must boot");
    let addr = handle.local_addr;

    let endpoint = std::sync::Arc::new(bot_endpoint().expect("client endpoint"));
    let report = run_bot(&endpoint, addr, 1, Duration::from_secs(2), None)
        .await
        .expect("a bot must join a bbr shard exactly as it joins a cubic one");
    assert!(
        report.welcome.is_some(),
        "the controller is a transport choice and must not change the handshake"
    );
}
