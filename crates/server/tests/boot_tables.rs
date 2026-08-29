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

/// (2) The same list, from the other end: every `SimTables` field is filled
/// **by name** in `bake_all`.
///
/// **By name and not by count**, since 2026-08-28. It counted before —
/// fields against lines matching `bake_` and `:` — and that read the
/// *shape* of the constructor rather than the fact under assertion. Wire
/// v52 made the catalog read the already-baked combat table instead of
/// re-deriving the armor rows from `content.armors`, which needs one bake
/// hoisted into a `let` above the literal; the hoisted line has no colon,
/// the field line has no `bake_`, and a green gate went red over a
/// constructor that still fills all thirteen. A count is also the weaker
/// assertion in the direction that matters: two fields served from one
/// bake and one served from none counts the same as thirteen correct.
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
    let fields: Vec<&str> = body
        .lines()
        .filter_map(|l| l.trim().strip_prefix("pub "))
        .filter_map(|rest| rest.split(':').next())
        .collect();

    assert!(
        fields.len() >= 13,
        "only {} `SimTables` fields parsed: {fields:?} — the declaration \
         shape moved and this gate is reading nothing",
        fields.len()
    );

    let call_body = net_src
        .split_once("pub fn bake_all(")
        .expect("bake_all is declared in net.rs")
        .1
        .split_once("\n}")
        .expect("bake_all is closed")
        .0;

    for f in &fields {
        // Two shapes, and each carries its own half of the claim.
        //
        // `field: <expr>,` in the literal — the expression must name a
        // bake, which is the half the old count assertion was carrying:
        // `catalog: ItemCatalog::EMPTY` fills the field, compiles, and is
        // `bake_research`'s defect exactly (a table the sim runs on that
        // nothing in `content/` authored). Found by running the mutant on
        // the rewrite: the name check alone let it through.
        //
        // `field,` — the shorthand for a value hoisted above the literal,
        // which `combat` is because the catalog reads it. Then the *let*
        // has to be the one naming the bake.
        let by_expr = call_body.lines().any(|l| {
            l.trim()
                .strip_prefix(f)
                .and_then(|rest| rest.strip_prefix(':'))
                .is_some_and(|expr| expr.contains("bake_"))
        });
        let by_binding = call_body.lines().any(|l| l.trim() == format!("{f},"))
            && call_body.lines().any(|l| {
                let l = l.trim();
                l.starts_with(&format!("let {f} ")) && l.contains("bake_")
            });
        assert!(
            by_expr || by_binding,
            "`SimTables::{f}` is not filled from a bake in `bake_all` — \
             either the field is missing from the constructor, or it is \
             served from something that is not a `bake_*` call. A table \
             the sim runs on that nothing in `content/` authored is \
             `bake_research`'s defect, and it compiles."
        );
    }
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

/// (5) The armor columns' behavioural half (wire v52), the same shape one
/// version later — and the same reason: **the client links no content
/// crate**, so the wear panel's protection total is whatever the catalog
/// says and nothing else. If the columns are inert, the panel prints 0 %
/// on a full set with every wire gate green, because a golden pins bytes
/// and not whether the boot path filled them.
///
/// Checked against `content.armors` rather than against `tables.combat`,
/// deliberately: `bake_catalog` reads the combat table, so comparing the
/// two would compare a value with itself. The authored toml is the
/// independent source, and it is the one an editor changes.
#[test]
fn the_shipped_catalog_carries_every_armor_row() {
    let content = content::Content::load_dir(&content_dir()).expect("shipped content loads");
    let tables = server::net::bake_all(&content).expect("shipped content bakes");

    for a in &content.armors {
        let idx = content.item_index(&a.id).expect("armor is item-backed") as usize;
        assert_eq!(
            u32::from(tables.catalog.armor_pct(idx)),
            a.reduction_pct,
            "`{}` is authored at {} % and the catalog drips {} % — the wear \
             panel would print a total the server never subtracts",
            a.id,
            a.reduction_pct,
            tables.catalog.armor_pct(idx)
        );
        let authored_slot = match a.slot {
            content::schema::ArmorSlot::Head => sim_core::combat::WEAR_HEAD,
            content::schema::ArmorSlot::Body => sim_core::combat::WEAR_BODY,
        };
        assert_eq!(
            tables.catalog.wear_slot(idx),
            authored_slot,
            "`{}` is authored for slot {} and the catalog drips {} — the \
             client would light the wrong cell during a drag",
            a.id,
            authored_slot,
            tables.catalog.wear_slot(idx)
        );
    }
    assert!(
        content.armors.len() >= 3,
        "only {} armor rows ship — the columns are untested by this content \
         set and this gate is passing for free",
        content.armors.len()
    );

    // The other direction, and the one a per-row loop cannot see: an item
    // that is *not* armor must carry an inert row. Without this, a bake
    // that wrote `WEAR_HEAD` into every row would pass everything above
    // and make a rock protect your head.
    let mut inert = 0usize;
    for item in &content.items {
        if content.armors.iter().any(|a| a.id == item.id) {
            continue;
        }
        let idx = content.item_index(&item.id).expect("own id resolves") as usize;
        assert_eq!(
            (tables.catalog.armor_pct(idx), tables.catalog.wear_slot(idx)),
            (0, sim_core::combat::WEAR_NONE),
            "`{}` is not armor and the catalog gives it a wear row",
            item.id
        );
        inert += 1;
    }
    assert!(inert > 0, "every shipped item is armor?");
}
