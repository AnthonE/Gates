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
        ticks: None,
        body: None,
        body_temp: 1.0,
        body_seed: 0,
    };
    let mut minds = bench::minds(&mind, "bench", 2).expect("scripted minds build");
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
        ticks: None,
        body: None,
        body_temp: 1.0,
        body_seed: 0,
    };
    let mut minds = bench::minds(&mind, "bench", 2).expect("scripted minds build inline");
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

/// **Every tick, recorded** (`--ticks`, `server::ticks`): a warped run
/// writes one fixed-size record per seat per tick, and a header that names
/// the record and the goals it saw.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_warped_bench_records_every_tick() {
    use server::ticks::{row_bytes, COLUMNS, FORMAT};
    let dir = std::env::temp_dir().join(format!("gates-ticks-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mind = MindArgs {
        source: Source::Scripted,
        inline: true,
        ..MindArgs::default()
    };
    let seconds = 20;
    let plan = Bench {
        mind: &mind,
        bots: 2,
        duration: Duration::from_secs(seconds),
        name: "bench",
        content: local_content(),
        warp: true,
        ticks: Some(dir.clone()),
        body: None,
        body_temp: 1.0,
        body_seed: 0,
    };
    let mut minds = bench::minds(&mind, "bench", 2).expect("scripted minds build inline");
    bench::run(&plan, &mut minds, 0)
        .await
        .expect("a warped run");

    let run = dir.join("run-00");
    let header: serde_json::Value =
        serde_json::from_slice(&std::fs::read(run.join("ticks.json")).expect("a header"))
            .expect("the header is JSON");
    let bin = std::fs::read(run.join("ticks.bin")).expect("records");
    let rows = 2 * seconds as usize * sim_core::limits::TICK_HZ as usize;
    assert_eq!(header["format"], FORMAT);
    assert_eq!(header["rows"], rows as u64, "a record per seat per tick");
    assert_eq!(header["row_bytes"], row_bytes() as u64);
    assert_eq!(bin.len(), rows * row_bytes(), "the file is its records");
    assert_eq!(
        header["dtype"].as_array().map(Vec::len),
        Some(COLUMNS.len())
    );
    let goals = header["goals"].as_array().expect("a goal table");
    assert!(!goals.is_empty(), "the mind picked goals: {header}");

    // Records are tick-major, seat-minor: the first two are tick 0.
    let field = |row: usize, name: &str| -> &[u8] {
        let mut at = row * row_bytes();
        for (n, t, k) in COLUMNS {
            let w = t.as_bytes().last().map_or(0, |c| (c - b'0') as usize) * k;
            if *n == name {
                return &bin[at..at + w];
            }
            at += w;
        }
        panic!("no column {name}");
    };
    let u32_at = |row, name| u32::from_le_bytes(field(row, name).try_into().unwrap());
    assert_eq!((u32_at(0, "tick"), field(0, "seat")[0]), (0, 0));
    assert_eq!((u32_at(1, "tick"), field(1, "seat")[0]), (0, 1));
    assert_eq!(u32_at(rows - 1, "tick"), rows as u32 / 2 - 1);
    // The bodies moved and their minds ran goals.
    let moved = (0..rows).any(|r| field(r, "f_move_z")[0] != 0 || field(r, "f_move_x")[0] != 0);
    assert!(moved, "some frame walked");
    let goaled = (0..rows).filter(|&r| field(r, "goal") != [0, 0]).count();
    assert!(
        goaled > rows / 2,
        "most ticks run a goal: {goaled} of {rows}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
