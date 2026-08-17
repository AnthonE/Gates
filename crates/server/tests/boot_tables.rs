//! **Every content table a shard bakes actually reaches the sim.**
//!
//! This suite exists because one of them did not, for the whole life of the
//! feature. `Content::bake_research` was written, validated, unit-tested and
//! **never called**: `sim_thread` installed eleven tables, `World::research`
//! stayed `ResearchContent::EMPTY`, every `Command::Research` refused with
//! `REFUSE_R_ITEM`, and the six `blueprint = true` recipes in
//! `content/recipes.toml` were uncraftable by any player on any live shard.
//!
//! **Every gate in the tree was green**, and none of them could have failed.
//! `test_content` validates the toml and bakes the table — in a test, into a
//! local. `sim-core/tests/research.rs` drives the verb — against
//! `ResearchContent::probe_fixture()`, which it installs itself. The wire
//! suites boot a real shard and never ask it what it knows. The defect is
//! not a wrong value anywhere; it is a **missing call site**, and the only
//! thing that sees a missing call site is something that looks at the call
//! sites. Same shape, and the same remedy, as the single-drain rule
//! `crates/client/tests/sound.rs` greps for.
//!
//! Two halves, because either alone can be satisfied dishonestly:
//!
//! 1. **Structural** — every `pub fn bake_*` the content crate exposes is
//!    named in `net::bake_all`. Catches the next table on the day it is
//!    written, before anyone wonders why the verb does nothing.
//! 2. **Behavioural** — the shipped content, taken through the boot path a
//!    shard actually runs, produces a world that can answer a research
//!    request for a shipped item. Catches a table that is installed but
//!    empty.

use sim_core::research::NO_RECIPE;

fn content_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content")
}

fn read(rel: &str) -> String {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()))
}

/// (1) Every bake the content crate exposes has a home in `bake_all`.
///
/// A grep and not a trait, deliberately: the defect being gated is that
/// somebody wrote a function and nobody called it, and no signature can
/// express "and this is wired". The list of bakes is read out of the source
/// rather than typed here, so a new one is covered the moment it exists —
/// typing the list would reproduce the original bug one level up.
#[test]
fn every_content_bake_is_installed_on_the_boot_path() {
    let bake_src = read("../content/src/bake.rs");
    let net_src = read("src/net.rs");

    let bakes: Vec<&str> = bake_src
        .lines()
        .filter_map(|l| l.trim().strip_prefix("pub fn bake_"))
        .filter_map(|rest| rest.split(['(', '<']).next())
        .collect();

    assert!(
        bakes.len() >= 12,
        "expected the content crate to expose a dozen bakes, found {}: {bakes:?} \
         — if the declaration form moved, this gate is reading nothing and \
         passing for free",
        bakes.len()
    );

    let missing: Vec<&str> = bakes
        .iter()
        .filter(|b| !net_src.contains(&format!("bake_{b}()")))
        .copied()
        .collect();
    assert!(
        missing.is_empty(),
        "content bakes with no caller on the boot path: {missing:?}. \
         A baked table nobody installs is a feature that is dark on every \
         live shard with every gate green — which is exactly how \
         `bake_research` shipped. Add it to `net::bake_all` and to \
         `SimTables`, or delete it."
    );
}

/// (2) The same list, from the other end: `SimTables` has a field per bake,
/// so the compiler carries the invariant once the grep above has pointed at
/// it. A table in `bake_all` that no field receives cannot compile, and this
/// asserts the count agrees so a field cannot be quietly served twice.
#[test]
fn the_table_struct_has_one_field_per_bake() {
    let net_src = read("src/net.rs");
    let body = net_src
        .split_once("pub struct SimTables {")
        .expect("SimTables is declared in net.rs")
        .1
        .split_once("\n}")
        .expect("SimTables is closed")
        .0;
    let fields = body
        .lines()
        .filter(|l| l.trim().starts_with("pub "))
        .count();

    let call_body = net_src
        .split_once("pub fn bake_all(")
        .expect("bake_all is declared in net.rs")
        .1;
    let installs = call_body
        .lines()
        .take_while(|l| !l.starts_with('}'))
        .filter(|l| l.contains("bake_") && l.contains(':'))
        .count();

    assert_eq!(
        fields, installs,
        "`SimTables` has {fields} fields and `bake_all` fills {installs} — \
         a field that is not filled from a bake is a table the sim runs on \
         that nothing in `content/` authored"
    );
}

/// (3) The behavioural half. The shipped content, through the boot path,
/// answers a research request for a shipped item.
///
/// `row_for` returning `Some` is the exact predicate that was false on every
/// live shard before 2026-08-15: `ResearchContent::EMPTY` has `row_count = 0`,
/// so the linear scan found nothing and the verb refused `REFUSE_R_ITEM` no
/// matter what the player was holding or standing next to.
#[test]
fn the_shipped_research_table_reaches_a_booted_world() {
    let content = content::Content::load_dir(&content_dir()).expect("shipped content loads");
    let tables = server::net::bake_all(&content).expect("shipped content bakes");

    assert!(
        tables.research.row_count > 0,
        "the shipped research table reached the sim empty — the verb is dark"
    );
    assert_eq!(
        tables.research.row_count as usize,
        content.research.len(),
        "every authored row survives the boot path"
    );

    // Install it the way `sim_thread` does and ask the world a question a
    // player would ask.
    let mut w = sim_core::world::World::new(1);
    w.research = tables.research;

    let coin = content
        .item_index(&content.research_coin.item)
        .expect("the coin is an item");
    assert_eq!(
        w.research.coin, coin,
        "and it is priced in the coin the file names"
    );

    for r in &content.research {
        let item = content
            .item_index(&r.item)
            .unwrap_or_else(|| panic!("shipped {}", r.item));
        let row = w
            .research
            .row_for(item)
            .unwrap_or_else(|| panic!("`{}` is authored and the sim cannot find it", r.item));
        assert_ne!(
            row.recipe, NO_RECIPE,
            "`{}` resolved to no recipe, so learning it would unlock nothing",
            r.item
        );
        assert_eq!(
            row.cost as u32, r.cost,
            "`{}` is priced as authored",
            r.item
        );
    }
}

/// Every `blueprint = true` recipe is reachable through the table that is
/// actually installed — the player-facing statement of the same bug. Six
/// recipes in the shipped set were uncraftable by anyone; this is the
/// assertion that says so in the units a player would notice.
#[test]
fn no_shipped_blueprint_recipe_is_unreachable() {
    let content = content::Content::load_dir(&content_dir()).expect("shipped content loads");
    let tables = server::net::bake_all(&content).expect("shipped content bakes");

    let gated: Vec<&str> = content
        .recipes
        .iter()
        .filter(|k| k.blueprint)
        .map(|k| k.output.as_str())
        .collect();
    assert!(
        !gated.is_empty(),
        "no blueprint-gated recipe ships — this gate is passing for free"
    );

    for output in gated {
        let item = content
            .item_index(output)
            .unwrap_or_else(|| panic!("shipped {output}"));
        assert!(
            tables.research.row_for(item).is_some(),
            "`{output}` is blueprint-gated and the installed research table \
             cannot teach it — nobody on a live shard could ever craft it"
        );
    }
}

/// The catalog a shard drips carries the condition ceiling beside every
/// name (wire v46, NOW.md §0dur.1). Before this column existed the client
/// held per-slot condition with nothing to divide it by — the catalog was
/// names only, no def table carried a ceiling, and the client links no
/// content crate — so `ui::slots::pip_fraction`'s landed contract had no
/// caller. This is the behavioural half: shipped content, through the boot
/// path a shard runs, row for row against the authored `condition_max`.
#[test]
fn the_shipped_catalog_carries_every_condition_ceiling() {
    let content = content::Content::load_dir(&content_dir()).expect("shipped content loads");
    let tables = server::net::bake_all(&content).expect("shipped content bakes");

    let mut conditioned = 0usize;
    for item in &content.items {
        let idx = content.item_index(&item.id).expect("own id resolves") as usize;
        let authored = u16::try_from(item.condition_max).expect("validated content fits u16");
        assert_eq!(
            tables.catalog.cond_max(idx),
            authored,
            "`{}` ships condition_max {} and the catalog drips {} — the pip \
             would divide by the wrong ceiling",
            item.id,
            authored,
            tables.catalog.cond_max(idx)
        );
        if authored > 0 {
            conditioned += 1;
        }
    }
    assert!(
        conditioned > 0,
        "no shipped item carries a condition ceiling — the column is \
         untested by this content set and this gate is passing for free"
    );
}
