//! **A shard seats its own inhabitants.**
//!
//! Three consecutive merge-gate judges ranked the same thing first
//! (`findings/pass-20260813-230343-{14,16,18}-judge.md` §B.1): every opponent
//! in this tree lives inside a `#[test]` or needs an operator to start a
//! second binary by hand, so a human who joins the alpha shard meets a silent
//! island. This suite is the gate on the fix — `shard.toml`'s `population`
//! key and `server::population`.
//!
//! Asserts are **structural, never timed** (`GOAL.md` §what this box can
//! measure, and `bot_smoke.rs`'s own header): the live test polls observable
//! state — `joins`, `input_dg_ok`, `actions_ok`, the live gauge — and the
//! iteration ceiling is a *failure* bound that prints the counters, not a
//! deadline anything passes by beating.

use server::config::{parse_shard_toml, ShardConfig};
use server::net::spawn_shard;
use server::population::{self, PopulationStats, MAX_POPULATION};
use server::stats::ShardStats;
use server::store::Saves;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Four inhabitants: `run_bot` splits on `seed_stream % 2`, so this is two
/// owners building and locking against two attackers planting charges — the
/// smallest count that is a population rather than a pair.
const SEATED: usize = 4;

/// How many 100 ms looks the live test gives the shard before it calls the
/// population dead. **A failure bound, not a deadline**: nothing passes by
/// being fast, and the failure prints every counter so a red run says which
/// half stalled. Generous because this box shares four cores with nineteen
/// other services.
const LOOKS: usize = 300;

fn cfg_text(extra: &str) -> String {
    format!("bind = \"127.0.0.1:0\"\nseed = 42\n{extra}")
}

// ---------------------------------------------------------------------------
// The config key
// ---------------------------------------------------------------------------

#[test]
fn population_is_absent_by_default_and_parses_a_count() {
    let none = parse_shard_toml(&cfg_text("")).expect("two keys is a valid shard");
    assert_eq!(
        none.population, 0,
        "a shard with no population key seats nobody — that is the shipping \
         default and a public shard's operator did not ask for bots"
    );
    let some = parse_shard_toml(&cfg_text("population = 8\n")).expect("population parses");
    assert_eq!(some.population, 8);
    assert_eq!(
        ShardConfig::ephemeral(1).population,
        0,
        "the test/loopback config seats nobody either, or every suite in this \
         crate would grow four bots it did not ask for"
    );
}

#[test]
fn population_over_the_cap_is_refused_and_names_it() {
    let err = parse_shard_toml(&cfg_text(&format!("population = {}\n", MAX_POPULATION + 1)))
        .expect_err("a population that fills the shard is refused");
    assert!(
        err.contains(&MAX_POPULATION.to_string()),
        "the refusal has to name the cap so the operator can fix the line \
         without reading our source: {err}"
    );
    parse_shard_toml(&cfg_text(&format!("population = {MAX_POPULATION}\n")))
        .expect("the cap itself is allowed — it leaves exactly one human seat");
    assert_eq!(
        MAX_POPULATION,
        sim_core::limits::MAX_PLAYERS - 1,
        "the cap is MAX_PLAYERS minus the one seat that stays a person's; if \
         this moves, the reason it exists moved with it"
    );
}

#[test]
fn population_and_require_auth_are_refused_together() {
    let err = parse_shard_toml(&cfg_text("population = 4\nrequire_auth = true\n"))
        .expect_err("an authenticating shard cannot seat guests");
    assert!(
        err.contains("population") && err.contains("require_auth"),
        "both keys named, because the fix is choosing between them: {err}"
    );
    // Each alone is fine — the refusal is the *pair*, not either key.
    parse_shard_toml(&cfg_text("population = 4\n")).expect("guests + inhabitants");
    parse_shard_toml(&cfg_text("require_auth = true\n")).expect("auth + no inhabitants");
}

#[test]
fn a_bad_population_line_is_refused_rather_than_defaulted() {
    let err = parse_shard_toml(&cfg_text("population = lots\n"))
        .expect_err("an unparseable count does not silently seat nobody");
    assert!(err.contains("population"), "{err}");
}

// ---------------------------------------------------------------------------
// The wildcard-bind repair
// ---------------------------------------------------------------------------

#[test]
fn a_wildcard_bind_is_dialled_as_loopback() {
    // The one that matters: `0.0.0.0` is what every non-test shard binds, and
    // it is a wildcard to listen on rather than a host to connect to. Dialling
    // it verbatim fails every shift on the only config that is not a test's.
    let wild: SocketAddr = "0.0.0.0:4433".parse().unwrap();
    assert_eq!(
        population::dial_addr(wild),
        "127.0.0.1:4433".parse::<SocketAddr>().unwrap()
    );
    let wild6: SocketAddr = "[::]:4433".parse().unwrap();
    assert_eq!(
        population::dial_addr(wild6),
        "[::1]:4433".parse::<SocketAddr>().unwrap()
    );
    // A real host is left exactly alone — the port is never the broken half.
    for named in ["127.0.0.1:0", "10.0.0.7:4433", "[::1]:4433"] {
        let a: SocketAddr = named.parse().unwrap();
        assert_eq!(population::dial_addr(a), a, "{named} needed no repair");
    }
}

#[test]
fn seating_more_than_the_cap_is_refused_by_the_spawner_too() {
    // The parser refuses it at the config line; the spawner refuses it again
    // because `spawn_population` is public and a caller that is not a config
    // file exists (this suite is one).
    let shutdown = Arc::new(AtomicBool::new(true));
    let err = population::spawn_population(
        MAX_POPULATION + 1,
        "127.0.0.1:1".parse().unwrap(),
        None,
        Duration::from_secs(1),
        shutdown,
    )
    .err()
    .expect("over the cap is refused");
    assert!(err.contains(&MAX_POPULATION.to_string()), "{err}");
}

// ---------------------------------------------------------------------------
// The live one: a shard with a population is not a silent island
// ---------------------------------------------------------------------------

/// The shipped content, baked — the same boot path `bin/shard.rs` runs, and
/// **with the shipped spawn kit** rather than `SpawnKit::EMPTY`.
///
/// That is the whole difference from `bot_smoke.rs`, which spawns its fleet
/// naked on purpose so its snapshot numbers are not asserting on content.
/// Here the kit is the point: an inhabitant has to be able to afford what a
/// player who joined the same shard could afford, so the population runs on
/// exactly what `content/` hands a fresh body and nothing else.
struct Baked {
    content: content::Content,
    tables: server::net::SimTables,
}

fn baked() -> Baked {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content");
    let content = content::Content::load_dir(&dir).expect("shipped content loads");
    let tables = server::net::bake_all(&content).expect("shipped content bakes");
    Baked { content, tables }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_shard_seats_its_own_inhabitants_over_its_own_wire() {
    let b = baked();
    let mut cfg = ShardConfig::ephemeral(0xC0FFEE);
    cfg.population = SEATED;
    let handle = spawn_shard(
        cfg.clone(),
        b.tables,
        Saves::off(),
        server::worldfile::WorldBoot::off(),
    )
    .await
    .expect("shard boots");

    // `seat` and not `spawn_population`: this is the function `bin/shard.rs`
    // calls, so the count, the row resolution and the wildcard repair are all
    // under the gate rather than re-implemented beside it.
    let seated = population::seat(&cfg, &b.content, handle.local_addr, handle.shutdown.clone())
        .expect("population seats")
        .expect("population = 4 is not None");
    assert!(
        seated.no_raid.is_none(),
        "the shipped content has to carry the raid profile's four rows, or a \
         population is scenery: {:?}",
        seated.no_raid
    );
    assert_eq!(seated.dial, handle.local_addr, "ephemeral binds loopback");

    let s = &handle.stats;
    // Cloned rather than borrowed: `join()` consumes the population, and the
    // counters have to outlive it — the last assert is about what the posts
    // left behind.
    let g = seated.population.stats.clone();
    let mut looks = 0;
    // Every one of these is observable state, and every one of them is 0 on a
    // shard with no population — which is what makes this red without the
    // feature rather than slow.
    while !(ShardStats::get(&s.joins) >= SEATED as u64
        && seated.population.live() == SEATED as u64
        && ShardStats::get(&s.input_dg_ok) > 0
        && ShardStats::get(&s.actions_ok) > 0)
    {
        looks += 1;
        assert!(
            looks < LOOKS,
            "the island stayed silent: joins {} (want {SEATED}), live {} , \
             inputs {}, actions {}, shifts started/errored {}/{}",
            ShardStats::get(&s.joins),
            seated.population.live(),
            ShardStats::get(&s.input_dg_ok),
            ShardStats::get(&s.actions_ok),
            PopulationStats::get(&g.shifts_started),
            PopulationStats::get(&g.shift_errors),
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // The shard cannot tell an inhabitant from a player — that is the design —
    // so these are the shard's own counters, moved by nothing but the bots.
    println!(
        "seated {SEATED}: joins {} · inputs {} · actions ok/bad {}/{} · \
         shifts started {} errored {} · looks {looks}",
        ShardStats::get(&s.joins),
        ShardStats::get(&s.input_dg_ok),
        ShardStats::get(&s.actions_ok),
        ShardStats::get(&s.actions_bad),
        PopulationStats::get(&g.shifts_started),
        PopulationStats::get(&g.shift_errors),
    );
    assert_eq!(
        ShardStats::get(&s.actions_bad),
        0,
        "an inhabitant that hands the shard an action it cannot decode drops \
         its own session (`net.rs` action_reader_task) — the fleet must never \
         produce one"
    );
    assert_eq!(
        ShardStats::get(&s.refused_full),
        0,
        "the cap exists so a population never fills the shard it lives on"
    );

    // ---- and it leaves when the shard does -------------------------------
    handle.shutdown.store(true, Ordering::Relaxed);
    seated.population.join().await;
    assert_eq!(
        PopulationStats::get(&g.live),
        0,
        "every post ends on the shard's own shutdown flag — a population that \
         outlived its shard would hold the process open past `systemctl stop`"
    );
}

/// The zero case is the shipping default and gets its own gate: a shard that
/// was not asked for inhabitants must not grow any, and `seat` must not so
/// much as bind a client endpoint for it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_shard_with_no_population_seats_nobody() {
    let b = baked();
    let cfg = ShardConfig::ephemeral(0xC0FFEE);
    assert_eq!(cfg.population, 0);
    let shutdown = Arc::new(AtomicBool::new(false));
    let seated = population::seat(
        &cfg,
        &b.content,
        "127.0.0.1:4433".parse().unwrap(),
        shutdown.clone(),
    )
    .expect("no population is not an error");
    assert!(
        seated.is_none(),
        "population = 0 seats nobody and dials nothing"
    );
    shutdown.store(true, Ordering::Relaxed);
}
