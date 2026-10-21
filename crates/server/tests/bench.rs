//! **GatesBench end to end, short** (`server::bench`, `jev-bot --bench`).
//!
//! A scripted pair on a fresh private island, each a throwaway wallet the
//! loopback shard admits, played for seconds rather than minutes, the island
//! shut down so it writes its standings, and every bot's row read back. Then
//! the same minds again on a second island: a bench plays one mind through
//! every run and counts each run apart.

use server::agent_demo::{local_config, local_content, MindArgs, Source};
use server::bench::{self, Bench, Header};
use std::time::Duration;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_short_scripted_bench_plays_counts_and_shuts_its_islands() {
    let mind = MindArgs {
        source: Source::Scripted,
        model: Some("test-model".into()),
        ..MindArgs::default()
    };
    let plan = Bench {
        mind: &mind,
        bots: 2,
        duration: Duration::from_secs(6),
        name: "bench",
        content: local_content(),
        warp: false,
    };
    let mut minds = bench::minds(&mind, 2).expect("scripted minds build");
    let mut runs = Vec::new();
    for r in 0..2 {
        let bots = bench::run(&plan, &mut minds, r).await.expect("a run");
        assert_eq!(bots.len(), 2, "a row for every bot");
        assert_eq!(minds.len(), 2, "and every mind back for the next run");
        for b in &bots {
            assert!(b.decisions > 0, "run {r}: the mind decided: {b:?}");
            assert_eq!(
                (b.answered, b.autopilot),
                (b.decisions, 0),
                "a scripted mind answers its own turns"
            );
            assert!(b.alive_s > 0 && b.alive_s <= 6, "alive for the run: {b:?}");
        }
        runs.push(bots);
    }
    let header = Header {
        commit: bench::commit(),
        proto: u32::from(protocol::PROTO_VER),
        seed: local_config().unwrap().seed,
        layout: 1,
        minutes: 1,
        bots: 2,
        mind: mind.word(),
        harness: None,
        model: mind.labels().1.map(str::to_owned),
        think_ms: 1000,
    };
    let v = bench::result(&header, &runs);
    assert_eq!(v["format"], "gates-bench-1");
    assert_eq!(v["mind"], "scripted");
    assert_eq!(v["model"], "test-model");
    assert_eq!(v["seed"], 20_260_731);
    assert_eq!(v["score"]["n"], 2);
    assert_eq!(v["runs"][1]["bots"].as_array().map(Vec::len), Some(2));
}

/// **The same bench, warped** (`--warp`): a minute of island time per run on
/// lockstep islands, in a fraction of that on the clock. Every mind decides
/// inline and every run's minds come back with their counts; a model mind is
/// refused, because a warped island does not wait for one.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_warped_bench_plays_a_minute_in_less_and_refuses_a_model() {
    let mind = MindArgs {
        source: Source::Scripted,
        inline: true,
        ..MindArgs::default()
    };
    let plan = Bench {
        mind: &mind,
        bots: 2,
        duration: Duration::from_secs(60),
        name: "bench",
        content: local_content(),
        warp: true,
    };
    let mut minds = bench::minds(&mind, 2).expect("scripted minds build inline");
    let started = std::time::Instant::now();
    for r in 0..2 {
        let bots = bench::run(&plan, &mut minds, r)
            .await
            .expect("a warped run");
        assert_eq!(bots.len(), 2, "a row for every bot");
        assert_eq!(minds.len(), 2, "and every mind back for the next run");
        for b in &bots {
            assert!(b.decisions > 0, "run {r}: the mind decided: {b:?}");
            assert_eq!(b.autopilot, 0, "a scripted mind answers its own turns");
            assert!(
                b.alive_s > 30 && b.alive_s <= 60,
                "alive for the island's minute: {b:?}"
            );
        }
    }
    assert!(
        started.elapsed() < Duration::from_secs(120),
        "two warped minutes took {:?}",
        started.elapsed()
    );
    let jev = MindArgs {
        inline: true,
        ..MindArgs::default()
    };
    let err = jev.build().err().expect("a model mind cannot warp");
    assert!(err.contains("--warp"), "{err}");
}
