//! The shard server: `cargo run -p server --bin shard` (reads shard.toml,
//! CLAUDE.md commands). Prints the bound address and the dev-cert hash the
//! browser's `serverCertificateHashes` flow needs, then a stats line every
//! 10 s. Runs until killed.

use server::config::parse_shard_toml;
use server::net::spawn_shard;
use server::stats::ShardStats;
use std::path::Path;
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
    let deploy = match content.bake_deployables() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("shard: content bake refused: {e}");
            std::process::exit(1);
        }
    };
    let build = match content.bake_building() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("shard: content bake refused: {e}");
            std::process::exit(1);
        }
    };
    let combat = match content.bake_combat() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("shard: content bake refused: {e}");
            std::process::exit(1);
        }
    };
    let backpack = match content.bake_backpack() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("shard: content bake refused: {e}");
            std::process::exit(1);
        }
    };
    let survival = match content.bake_survival() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("shard: content bake refused: {e}");
            std::process::exit(1);
        }
    };
    let loot = match content.bake_loot() {
        Ok(l) => l,
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
    println!(
        "content anchors: melee breach — door {} swings · wall w/s/m {}/{}/{}",
        a.door_breach_swings,
        a.wall_breach_swings[0],
        a.wall_breach_swings[1],
        a.wall_breach_swings[2]
    );
    let seed = cfg.seed;
    // The island gets the same boot treatment content does. `spawn_shard`
    // refuses a short one on every path; this call is here for the *counter*,
    // which is the half that has something to say on a seed that fills — the
    // shard printed its content hash and its balance anchors and then went
    // quiet about how much island it had built.
    match server::boot::check_seed(seed) {
        Ok(live) => println!(
            "island ok: {live}/{} authored sites (haven pad + {} waystations)",
            server::boot::AUTHORED_SITES,
            sim_core::terrain::WAYSTATIONS
        ),
        Err(e) => {
            eprintln!("shard: {e}");
            std::process::exit(1);
        }
    }
    // The player store, opened and validated before a port is bound — the
    // posture content and the island already take (CLAUDE.md wall 7). It needs
    // the content hash, which is why it is opened HERE and handed over rather
    // than inside `spawn_shard`: a save file describes inventories as item
    // *indices*, so restoring it under moved content would hand players the
    // wrong things, and that has to be a refusal rather than a surprise.
    let saves = match cfg.save_file.as_deref() {
        None => {
            println!("saves off: no save_file — every join builds a fresh character");
            server::store::Saves::off()
        }
        Some(path) => match server::store::open(Path::new(path), cfg.seed, content.hash()) {
            Ok((saves, found)) => {
                if found.created {
                    println!("saves ok: created {path} — remembering nobody yet");
                } else {
                    // The backup depth is named at boot because it is the thing
                    // an operator needs to know BEFORE the save goes bad, not
                    // after: recovery is copying `<file>.1` over `<file>`, and
                    // `.2` exists because `.1` can share the corruption
                    // (reference/SAVES.md §6).
                    println!(
                        "saves ok: {} players remembered from {path} · {} backup(s) rotated \
                         ({path}.1 is the previous run)",
                        found.live,
                        server::store::SAVE_BACKUP_COUNT
                    );
                }
                if found.corrupt > 0 {
                    // Its own line, and said out loud rather than folded into
                    // the count: a corrupt record is somebody's base, and the
                    // operator is the only one who can judge whether N of them
                    // is one torn write or a disk going bad.
                    println!(
                        "saves WARNING: {} record(s) REFUSED as corrupt — that many players \
                         start over. One is a torn write (the shard was killed mid-save); \
                         many is the disk.",
                        found.corrupt
                    );
                }
                if !cfg.require_auth {
                    // The trap this warns about is silent: saves armed,
                    // admission open, so every joiner is a guest with no key
                    // to be filed under and the store stays empty forever
                    // while every gate is green.
                    println!(
                        "saves WARNING: require_auth is false, so joiners have no identity \
                         to save under — a guest is admitted and remembered by nobody, and \
                         this shard will write nothing to that file"
                    );
                }
                saves
            }
            Err(e) => {
                eprintln!("shard: {e}");
                std::process::exit(1);
            }
        },
    };
    let handle = match spawn_shard(
        cfg, gather, craft, build, deploy, combat, backpack, survival, loot, catalog, saves,
    )
    .await
    {
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
            "tick {} · joins {} leaves {} · in ok/bad/drop {}/{}/{} · snap sent/skip/err {}/{}/{} · refused v/full {}/{} · dropped-ticks {} · saves restored/written/lost {}/{}/{}",
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
            ShardStats::get(&s.saves_restored),
            ShardStats::get(&s.saves_written),
            // The three failures that mean a player lost something, summed:
            // a full ring, an evicted table slot, a disk that refused.
            ShardStats::get(&s.save_ring_drops)
                + ShardStats::get(&s.saves_evicted)
                + ShardStats::get(&s.save_write_errors),
        );
    }
}
