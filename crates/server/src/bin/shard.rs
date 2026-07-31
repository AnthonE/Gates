//! The shard server: `cargo run -p server --bin shard` (reads shard.toml,
//! CLAUDE.md commands). Prints the bound address and the dev-cert hash the
//! browser's `serverCertificateHashes` flow needs, then a stats line every
//! 10 s. Runs until killed.

use server::config::parse_shard_toml;
use server::net::spawn_shard;
use server::stats::ShardStats;
use std::time::Duration;

#[tokio::main]
async fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "shard.toml".into());
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("shard: cannot read {path}: {e}");
            eprintln!(
                "shard: shard.toml is per-deployment (gitignored); start from shard.toml.example"
            );
            std::process::exit(1);
        }
    };
    let cfg = match parse_shard_toml(&text) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("shard: {e}");
            std::process::exit(1);
        }
    };
    // Content validates at boot (CLAUDE.md wall 7): a set that fails
    // schema, reference, or balance-band checks does not get a shard. The
    // hash pins into the WAL header when the WAL file format lands.
    let content = match content::Content::load_dir(std::path::Path::new(&cfg.content_dir)) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("shard: content refused: {e}");
            std::process::exit(1);
        }
    };
    let gather = match content.bake_gather() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("shard: content bake refused: {e}");
            std::process::exit(1);
        }
    };
    let craft = match content.bake_craft() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("shard: content bake refused: {e}");
            std::process::exit(1);
        }
    };
    let catalog = match server::net::bake_catalog(&content) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("shard: content bake refused: {e}");
            std::process::exit(1);
        }
    };
    let a = content.anchors();
    println!(
        "content ok: {} items · hash {:016x}",
        content.items.len(),
        content.hash()
    );
    println!(
        "content anchors: raid w/s/m {:.2}/{:.2}/{:.2} · satchel {:.1} min · starter {:.1} min · upkeep {:.1} min/day",
        a.raid_ratio[0],
        a.raid_ratio[1],
        a.raid_ratio[2],
        a.satchel_minutes,
        a.starter_minutes,
        a.upkeep_daily_minutes
    );
    let seed = cfg.seed;
    let handle = match spawn_shard(cfg, gather, craft, catalog).await {
        Ok(h) => h,
        Err(e) => {
            eprintln!("shard: {e}");
            std::process::exit(1);
        }
    };
    println!("shard up on {} (seed {seed})", handle.local_addr);
    println!("dev cert sha256 {}", handle.cert_hash);

    let mut report = tokio::time::interval(Duration::from_secs(10));
    report.tick().await; // immediate first tick consumed
    loop {
        report.tick().await;
        let s = &handle.stats;
        println!(
            "tick {} · joins {} leaves {} · in ok/bad/drop {}/{}/{} · snap sent/skip/err {}/{}/{} · refused v/full {}/{} · dropped-ticks {}",
            ShardStats::get(&s.current_tick),
            ShardStats::get(&s.joins),
            ShardStats::get(&s.leaves),
            ShardStats::get(&s.input_dg_ok),
            ShardStats::get(&s.input_dg_bad),
            ShardStats::get(&s.input_ring_drops),
            ShardStats::get(&s.snap_sent),
            ShardStats::get(&s.snap_ring_skips),
            ShardStats::get(&s.snap_send_errors),
            ShardStats::get(&s.refused_version),
            ShardStats::get(&s.refused_full),
            ShardStats::get(&s.ticks_dropped),
        );
    }
}
