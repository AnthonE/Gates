//! Local, bounded load measurement: `cargo run --profile ci -p server
//! --bin soak -- [seconds=3600] [bots=100]`. JSON lines on stdout.
//! No wallet, external services or persistence. Run on an otherwise idle
//! machine; this measures the local shard and clients sharing that machine.
use server::botclient::{bot_endpoint, run_metered_bot, BotMeter};
use server::config::ShardConfig;
use server::net::{bake_all, spawn_shard};
use server::stats::ShardStats;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let seconds: u64 = args
        .first()
        .map_or(Ok(3600), |s| s.parse())
        .map_err(|e| format!("seconds: {e}"))?;
    let count: usize = args
        .get(1)
        .map_or(Ok(100), |s| s.parse())
        .map_err(|e| format!("bots: {e}"))?;
    if seconds == 0 || seconds > 86400 || count == 0 || count > sim_core::limits::MAX_PLAYERS {
        return Err("seconds must be 1..=86400; bots must fit MAX_PLAYERS".into());
    }
    let content = content::Content::load_dir(std::path::Path::new("content"))
        .map_err(|e| format!("content: {e}"))?;
    let tables = bake_all(&content)?;
    let handle = spawn_shard(
        ShardConfig::ephemeral(20260731),
        tables,
        server::store::Saves::off(),
        server::worldfile::WorldBoot::off(),
    )
    .await?;
    let endpoint = Arc::new(bot_endpoint()?);
    let mut tasks = tokio::task::JoinSet::new();
    let meters: Vec<_> = (0..count).map(|_| Arc::new(BotMeter::default())).collect();
    // A work-budget backstop longer than seating plus the measured window.
    // The normal end is the shared stop request after the final byte sample.
    let bot_duration = Duration::from_secs(seconds + 60);
    for (i, meter) in meters.iter().enumerate() {
        let endpoint = endpoint.clone();
        let addr = handle.local_addr;
        let meter = meter.clone();
        tasks.spawn(async move {
            (
                i,
                run_metered_bot(&endpoint, addr, i as u64, bot_duration, meter).await,
            )
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let seated = tokio::time::timeout(Duration::from_secs(45), async {
        while ShardStats::get(&handle.stats.players) != count as u64
            || meters.iter().any(|m| !m.ready.load(Ordering::Acquire))
        {
            if let Some(result) = tasks.try_join_next() {
                return Err(format!("a bot ended while seating: {result:?}"));
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        Ok(())
    })
    .await;
    if !matches!(seated, Ok(Ok(()))) {
        handle.shutdown.store(true, Ordering::Relaxed);
        return Err(format!("seating failed: {seated:?}"));
    }
    let stats = &handle.stats;
    let first_tick = ShardStats::get(&stats.current_tick);
    let dropped = ShardStats::get(&stats.ticks_dropped);
    let initial: Vec<_> = meters
        .iter()
        .map(|m| (Instant::now(), m.snapshot()))
        .collect();
    let started = Instant::now();
    stats.tick_jitter.enabled.store(true, Ordering::Relaxed);
    println!(
        "{}",
        serde_json::json!({"kind":"start", "clients":count,
        "seconds":seconds, "seed":20260731, "profile":"walk",
        "build":protocol::version::BUILD_ID, "proto":protocol::PROTO_VER,
        "tick":first_tick})
    );
    let mut minimum = count as u64;
    let mut lost = false;
    let mut progress_at = 60;
    while started.elapsed() < Duration::from_secs(seconds) {
        tokio::time::sleep(Duration::from_millis(100)).await;
        minimum = minimum.min(ShardStats::get(&stats.players));
        lost |= tasks.try_join_next().is_some();
        if started.elapsed().as_secs() >= progress_at {
            println!(
                "{}",
                serde_json::json!({"kind":"progress",
                "elapsed_seconds":started.elapsed().as_secs(),
                "clients":ShardStats::get(&stats.players),
                "ticks_dropped":ShardStats::get(&stats.ticks_dropped)-dropped})
            );
            progress_at += 60;
        }
    }
    stats.tick_jitter.enabled.store(false, Ordering::Relaxed);
    let elapsed_us = started.elapsed().as_micros();
    let measured: Vec<_> = meters
        .iter()
        .zip(initial.iter())
        .map(|(m, (at, initial))| {
            let end = m.snapshot();
            let bytes: [u64; 4] = std::array::from_fn(|i| end[i] - initial[i]);
            (at.elapsed().as_micros(), bytes)
        })
        .collect();
    for m in &meters {
        m.stop.store(true, Ordering::Release);
    }
    println!(
        "{}",
        serde_json::json!({"kind":"jitter", "elapsed_us":elapsed_us,
        "minimum_clients_polled_100ms":minimum, "client_ended_early":lost,
        "ticks":ShardStats::get(&stats.current_tick)-first_tick,
        "ticks_dropped":ShardStats::get(&stats.ticks_dropped)-dropped,
        "absolute_error_us_log2_bins":stats.tick_jitter.snapshot(),
        "early":ShardStats::get(&stats.tick_jitter.early),
        "late":ShardStats::get(&stats.tick_jitter.late),
        "max_absolute_error_us":ShardStats::get(&stats.tick_jitter.max_us)})
    );
    let mut failed = lost || minimum != count as u64;
    let mut completed = 0usize;
    while let Some(result) = tasks.join_next().await {
        match result {
            Ok((i, Ok(r))) => {
                completed += 1;
                let (window_us, bytes) = measured[i];
                failed |= r.walk_truncated || r.decode_errors != 0 || r.event_decode_errors != 0;
                println!(
                    "{}",
                    serde_json::json!({"kind":"client", "client":i,
                    "player_id":r.player_id, "elapsed_us":window_us, "session_elapsed_us":r.elapsed_us,
                    "dg_in_bytes":bytes[0], "dg_out_bytes":bytes[1],
                    "stream_in_bytes":bytes[2], "stream_out_bytes":bytes[3],
                    "snapshots":r.snapshots_applied, "stale":r.stale_snapshots,
                    "no_baseline":r.no_baseline, "decode_errors":r.decode_errors,
                    "event_decode_errors":r.event_decode_errors,
                    "walk_truncated":r.walk_truncated, "ticks_walked":r.ticks_walked})
                );
            }
            other => {
                failed = true;
                eprintln!("bot failed: {other:?}");
            }
        }
    }
    handle.shutdown.store(true, Ordering::Relaxed);
    println!(
        "{}",
        serde_json::json!({"kind":"complete", "clients":completed,
        "passed":!failed && completed == count})
    );
    if failed || completed != count {
        return Err("soak failed; preserve findings".into());
    }
    Ok(())
}
