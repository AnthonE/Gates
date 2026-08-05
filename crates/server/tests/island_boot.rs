//! The island validates at boot, and the seeds this repo ships have to pass
//! their own check.
//!
//! `crates/server/src/boot.rs` holds the unit tests for the *refusal* — they
//! build a `Haven` with a dead `Waystation` by hand, because no seed measured
//! reaches that branch (module docs there carry the 20_000-seed scan). This
//! file is the other half and the one that is not hypothetical: it reads the
//! tracked configs and the seeds the rest of the suite boots on, and asserts
//! the wall `spawn_shard` now enforces does not stand in front of them.
//!
//! Without this, a terrain change that shortened a tier on 0xC0FFEE would
//! surface as every wire test in `crates/server/tests` failing at
//! `spawn_shard` with an island message and no hint that the seed was the
//! thing that moved. Here it fails once, by name.

use server::boot::{check_seed, AUTHORED_SITES};
use server::config::parse_shard_toml;

/// The tracked per-deployment configs. `shard.toml` itself is gitignored, so
/// these two are the only seeds the repo can actually assert on — and
/// `shard-public.toml` is the one that matters, because it is what the
/// operator runs when they publish.
const TRACKED_CONFIGS: [&str; 2] = ["shard.toml.example", "shard-public.toml"];

fn repo_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn every_tracked_config_names_a_seed_that_fills_its_island() {
    for name in TRACKED_CONFIGS {
        let path = repo_root().join(name);
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("{name} is tracked and must be readable: {e}"));
        let cfg = parse_shard_toml(&text)
            .unwrap_or_else(|e| panic!("{name} must parse — the shard reads it verbatim: {e}"));
        match check_seed(cfg.seed) {
            Ok(live) => assert_eq!(
                live, AUTHORED_SITES,
                "{name}: seed {} reported {live} sites but did not refuse — \
                 check_island and sites_live disagree, which means the counter \
                 the shard prints is not the number the wall tests",
                cfg.seed
            ),
            Err(e) => panic!(
                "{name} names seed {}, which no longer fills its island, so \
                 `spawn_shard` will refuse to boot it: {e}",
                cfg.seed
            ),
        }
    }
}

/// The seeds the rest of this crate's suite spawns shards on. `spawn_shard`
/// refuses a short island before it binds, so a terrain change that shortens
/// any of these takes every wire test down with it; this names the seed
/// instead of leaving ten suites to report a bind-time string.
#[test]
fn the_suites_boot_seeds_all_fill_their_islands() {
    const SUITE_SEEDS: [u64; 6] = [0, 1, 1024, 0xC0FFEE, 20_260_731, 20_260_804];
    for seed in SUITE_SEEDS {
        assert_eq!(
            check_seed(seed),
            Ok(AUTHORED_SITES),
            "seed {seed} is spawned by a suite in crates/server/tests and no \
             longer fills its island"
        );
    }
}

/// The check is a pure function of the seed — it has to be, or two boots of
/// one `shard.toml` could disagree about whether the island is servable.
#[test]
fn the_check_is_a_pure_function_of_the_seed() {
    for seed in [0u64, 42, 20_260_731, 0xDEAD_BEEF] {
        assert_eq!(
            check_seed(seed),
            check_seed(seed),
            "seed {seed}: two island checks disagreed"
        );
    }
}
