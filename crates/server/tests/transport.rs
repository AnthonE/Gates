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

/// The raid profile's rows, resolved by content id — the same resolution
/// `bin/bots` and a booting shard use, never a typed row number (wall 7).
fn raid_rows() -> Option<sim_core::bots::RaidRows> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content");
    let content = content::Content::load_dir(&dir).ok()?;
    server::population::raid_rows(&content).ok()
}

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

/// **The shard counts the bytes it actually moved** (NOW.md §0q item 4).
///
/// The 100-bot soak's headline bandwidth number — 16.5 kB/s/client — was
/// budget × rate × clients, computed by hand, because nothing in `crates/`
/// had ever counted a byte. This drives one real client over all four lanes
/// and asserts the counters moved, that each byte total is consistent with
/// its own message count, and — the part that makes it a measurement rather
/// than a "> 0" — that the inbound datagram arrivals **reconcile with the
/// three verdict counters that partition them**, which is a second source
/// for the same events.
///
/// It does not assert a rate. A loopback shard with one bot is not the
/// soak; the deliverable here is the instrument.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_shard_counts_the_bytes_it_actually_moved() {
    let tables = baked_content();
    let handle = spawn_shard(
        ShardConfig::ephemeral(0xB17E5),
        tables,
        Saves::off(),
        server::worldfile::WorldBoot::off(),
    )
    .await
    .expect("shard boots");
    let addr = handle.local_addr;

    let endpoint = std::sync::Arc::new(bot_endpoint().expect("client endpoint"));
    // Raiding, so the C→S **stream** lane carries action frames: a walking
    // bot writes nothing after its handshake and `net_stream_in_*` would
    // stay at zero for a reason that has nothing to do with the counter.
    let rows = raid_rows();
    assert!(
        rows.is_some(),
        "the shipped content must resolve the raid rows, or this gate would \
         silently stop exercising the C→S stream lane"
    );
    let report = run_bot(&endpoint, addr, 1, Duration::from_secs(3), rows)
        .await
        .expect("bot ran");

    let s = &handle.stats;
    // Read the verdict counters BEFORE the arrival count: the arrival pair
    // is bumped first at the site, so this ordering can only leave the
    // arrival count ahead by the one datagram in flight, never behind.
    let verdicts = ShardStats::get(&s.input_dg_ok)
        + ShardStats::get(&s.input_dg_bad)
        + ShardStats::get(&s.input_dg_forged);
    let dg_in = ShardStats::get(&s.net_dg_in_count);
    let dg_in_bytes = ShardStats::get(&s.net_dg_in_bytes);
    let dg_out = ShardStats::get(&s.net_dg_out_count);
    let dg_out_bytes = ShardStats::get(&s.net_dg_out_bytes);
    let st_out = ShardStats::get(&s.net_stream_out_frames);
    let st_out_bytes = ShardStats::get(&s.net_stream_out_bytes);
    let st_in = ShardStats::get(&s.net_stream_in_frames);
    let st_in_bytes = ShardStats::get(&s.net_stream_in_bytes);

    assert!(
        dg_in > 0 && dg_out > 0,
        "no datagram was counted either way"
    );
    assert!(
        st_out > 0,
        "no event-lane frame was counted — the S→C stream is silent or unwatched"
    );
    assert!(
        st_in > 0,
        "no C→S frame was counted, and this bot raided: the action lane is unwatched"
    );

    // Bytes against counts. A datagram is at least one byte and at most the
    // budget it is encoded against, so the mean lands inside that band —
    // which is what says the number is *bytes* and not entities, frames, or
    // whatever else a plausible wrong call site would have handed it.
    let budget = sim_core::limits::DATAGRAM_BUDGET_BYTES as u64;
    assert!(
        dg_out_bytes >= dg_out && dg_out_bytes <= dg_out * budget,
        "outbound mean {} B over {dg_out} datagrams is outside 1..={budget} B",
        dg_out_bytes / dg_out
    );
    assert!(
        dg_in_bytes >= dg_in,
        "inbound bytes cannot be below arrivals"
    );
    assert!(
        st_out_bytes >= st_out * (server::net::FRAME_PREFIX_BYTES as u64 + 1),
        "stream-out bytes {st_out_bytes} cannot cover {st_out} prefixed frames"
    );
    assert!(
        st_in_bytes >= st_in * (server::net::FRAME_PREFIX_BYTES as u64 + 1),
        "stream-in bytes {st_in_bytes} cannot cover {st_in} prefixed frames"
    );

    // The second source. `<= 1` is the one datagram that can sit between
    // the two bumps of a single reader task, and nothing wider.
    assert!(
        dg_in >= verdicts && dg_in - verdicts <= 1,
        "arrivals {dg_in} do not reconcile with verdicts {verdicts} — one of \
         the two is counting a different set of datagrams"
    );

    // And the per-client half, off the same run: what one client measured
    // of its own traffic must be non-zero on every lane it used. The two
    // sides do not have to agree to the byte (loss, and the shard counts
    // every client), but a client reporting zero while the shard reports
    // traffic means one of the two is measuring nothing.
    assert!(
        report.dg_in_bytes > 0 && report.dg_in_count > 0,
        "the bot received snapshots and counted no bytes"
    );
    assert!(
        report.dg_out_bytes > 0,
        "the bot sent inputs and counted no bytes"
    );
    assert!(
        report.ev_in_bytes > 0,
        "the bot drained the event lane and counted no bytes"
    );
    assert!(
        report.act_out_bytes > 0,
        "the bot wrote {} action frames and counted no bytes",
        report.actions_sent
    );
}

/// A shard configured for fake latency must still serve — late, and
/// measurably through the detour (netcode v2, the netsim knob).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_netsim_shard_serves_late_datagrams_rather_than_none() {
    // The knob-plumbing gate, in the cc knob's shape (the comment there
    // names the risk verbatim): a knob is worth nothing if it parses and
    // is then dropped on the way to the tasks. Real latency both ways,
    // zero loss, and the bot must still join and see a world — plus the
    // shim's own counter proving datagrams actually took the detour.
    let mut cfg = ShardConfig::ephemeral(0xFA4E);
    cfg.netsim = Some(server::config::NetSim {
        lat_ms: 40,
        jitter_ms: 10,
        loss_pct: 0,
    });
    let handle = spawn_shard(
        cfg,
        baked_content(),
        Saves::off(),
        server::worldfile::WorldBoot::off(),
    )
    .await
    .expect("a shard configured for netsim must boot");
    let addr = handle.local_addr;
    let endpoint = std::sync::Arc::new(bot_endpoint().expect("client endpoint"));
    let report = run_bot(&endpoint, addr, 1, Duration::from_secs(2), None)
        .await
        .expect("a bot must join a netsim shard — late is not broken");
    assert!(
        report.welcome.is_some(),
        "the fake latency sits on the datagram lanes, never the handshake"
    );
    assert!(
        report.snapshots_applied > 0,
        "snapshots must still flow through the delay line"
    );
    assert!(
        server::stats::ShardStats::get(&handle.stats.netsim_delayed) > 0,
        "the knob parsed and was then dropped on the way to the tasks — \
         nothing detoured"
    );
    assert_eq!(
        server::stats::ShardStats::get(&handle.stats.netsim_dropped),
        0,
        "zero configured loss must drop zero datagrams"
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
