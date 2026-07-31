//! `test_content` (CLAUDE.md wall 7, CONTENT.md §0): the repo's shipped
//! `content/` loads, validates, holds every balance band, and hashes
//! stably — and the validator provably refuses the bug classes it claims
//! to: orphan refs, duplicate ids, unknown fields (the never-table at the
//! schema layer), and band breaks.

use content::Content;
use std::path::Path;

fn content_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content")
}

/// The shipped set, as owned (name, text) pairs tests can mutate.
fn sources() -> Vec<(&'static str, String)> {
    content::FILES
        .iter()
        .map(|f| {
            let text = std::fs::read_to_string(content_dir().join(f))
                .unwrap_or_else(|e| panic!("read {f}: {e}"));
            (*f, text)
        })
        .collect()
}

fn build(sources: &[(&'static str, String)]) -> Result<Content, String> {
    let borrowed: Vec<(&str, &str)> = sources.iter().map(|(n, t)| (*n, t.as_str())).collect();
    Content::from_sources(&borrowed)
}

/// Mutate one file's text and expect the set to be refused with `phrase`
/// in the error — a green here is the validator catching it, loudly.
fn refuses(file: &str, from: &str, to: &str, phrase: &str) {
    let mut srcs = sources();
    let entry = srcs.iter_mut().find(|(n, _)| *n == file).expect(file);
    assert!(
        entry.1.contains(from),
        "test fixture rot: `{from}` not in {file}"
    );
    entry.1 = entry.1.replace(from, to);
    let err = build(&srcs).expect_err(&format!("{file}: `{to}` was accepted"));
    assert!(
        err.contains(phrase),
        "{file}: expected error containing `{phrase}`, got: {err}"
    );
}

#[test]
fn test_content() {
    let c = Content::load_dir(&content_dir()).expect("shipped content must load");

    // The alpha set is present (~45 items, CONTENT §2) and the catalog is
    // dark until A3 (ALPHA §2).
    assert!(
        (40..=60).contains(&c.items.len()),
        "alpha set is ~45 items, got {}",
        c.items.len()
    );
    assert!(c.skins.is_empty(), "skin catalog is dark until A3");

    // Hash: nonzero, stable across an independent reload (formatting and
    // comments don't move it — same parse, same digest).
    let h = c.hash();
    assert_ne!(h, 0);
    assert_eq!(h, Content::load_dir(&content_dir()).unwrap().hash());

    // The anchors came out of the banded range (or load_dir would have
    // refused); pin the shape of the v0 story so a drastic re-cut is a
    // conscious edit here too.
    let a = c.anchors();
    assert!(a.raid_ratio[1] >= 1.0 && a.raid_ratio[1] <= 3.0);
    assert!(a.raid_ratio[0] < a.raid_ratio[1] && a.raid_ratio[1] < a.raid_ratio[2]);
    assert!(a.upkeep_daily_minutes <= 15.0);
    assert!(!a.ttk.is_empty());
}

#[test]
fn hash_moves_with_values() {
    let base = build(&sources()).unwrap().hash();
    let mut srcs = sources();
    let items = srcs.iter_mut().find(|(n, _)| *n == "items.toml").unwrap();
    items.1 = items.1.replace("stack = 1000", "stack = 999");
    let moved = build(&srcs).unwrap().hash();
    assert_ne!(base, moved, "a value change must move the content hash");
}

#[test]
fn unknown_field_refused() {
    // The never-table at the schema layer (CONTENT §6): a stat field that
    // can't be written can't be sold. Injection on an item...
    refuses(
        "items.toml",
        "id = \"item.wood\"",
        "id = \"item.wood\"\nloot_odds_mult = 2",
        "unknown field",
    );
}

#[test]
fn skin_stat_field_refused() {
    // ...and on a skin row: a valid row parses, the same row carrying a
    // stat field does not.
    let mut srcs = sources();
    let skins = srcs.iter_mut().find(|(n, _)| *n == "skins.toml").unwrap();
    skins.1 = String::from(
        "[[skin]]\nid = \"skin.wood_gilt\"\ncovers = \"item.hatchet_stone\"\n\
         coin = \"SCRY\"\nprice = 10\nseason = \"alpha\"\n",
    );
    build(&srcs).expect("a plain appearance row must parse");
    let skins = srcs.iter_mut().find(|(n, _)| *n == "skins.toml").unwrap();
    skins.1.push_str("damage_bonus = 1\n");
    let err = build(&srcs).expect_err("a stat field on a skin was accepted");
    assert!(err.contains("unknown field"), "got: {err}");
}

#[test]
fn dollar_ticker_refused() {
    // Tickers are bare (CLAUDE.md wall 8): `$SCRY` is not a coin.
    let mut srcs = sources();
    let skins = srcs.iter_mut().find(|(n, _)| *n == "skins.toml").unwrap();
    skins.1 = String::from(
        "[[skin]]\nid = \"skin.wood_gilt\"\ncovers = \"item.hatchet_stone\"\n\
         coin = \"$SCRY\"\nprice = 10\nseason = \"alpha\"\n",
    );
    assert!(build(&srcs).is_err(), "$-prefixed ticker was accepted");
}

#[test]
fn orphan_refs_refused() {
    refuses(
        "recipes.toml",
        "inputs = [{ item = \"item.stone\", count = 15 }]",
        "inputs = [{ item = \"item.unobtanium\", count = 15 }]",
        "not an item",
    );
    refuses(
        "loot.toml",
        "{ item = \"item.cloth\", weight = 20",
        "{ item = \"item.ghost\", weight = 20",
        "not an item",
    );
    refuses(
        "gatherables.toml",
        "output = \"item.wood\"",
        "output = \"item.timber\"",
        "not an item",
    );
}

#[test]
fn duplicate_id_refused() {
    let mut srcs = sources();
    let items = srcs.iter_mut().find(|(n, _)| *n == "items.toml").unwrap();
    items.1.push_str(
        "\n[[item]]\nid = \"item.wood\"\nname = \"Wood Again\"\nstack = 1\n\
         tier = 0\nrarity = \"common\"\nslot = \"none\"\n",
    );
    let err = build(&srcs).expect_err("duplicate id accepted");
    assert!(err.contains("duplicate id"), "got: {err}");
}

#[test]
fn missing_file_refused() {
    let srcs: Vec<(&str, String)> = sources()
        .into_iter()
        .filter(|(n, _)| *n != "skins.toml")
        .collect();
    let borrowed: Vec<(&str, &str)> = srcs.iter().map(|(n, t)| (*n, t.as_str())).collect();
    let err = Content::from_sources(&borrowed).expect_err("missing file accepted");
    assert!(err.contains("missing"), "got: {err}");
}

#[test]
fn band_breaks_refused() {
    // TTK: a 5-damage spear is 20 hits to kill — far out of melee 3–5.
    refuses(
        "weapons.toml",
        "kind = \"melee\"\ndamage = 25",
        "kind = \"melee\"\ndamage = 5",
        "band break: ttk",
    );
    // Raid ratio: a 100-damage satchel needs 18 to open stone — past 3×.
    refuses(
        "weapons.toml",
        "kind = \"throwable\"\ndamage = 500",
        "kind = \"throwable\"\ndamage = 100",
        "band break: stone raid ratio",
    );
    // Farm rate: a 3-per-hit tier-1 hatchet starves the tree band.
    refuses(
        "gatherables.toml",
        "\"item.hatchet_metal\" = 30",
        "\"item.hatchet_metal\" = 3",
        "band break: node yield",
    );
    // Armor: 60% reduction turns 4 hits into 10 — over the +2 cap.
    refuses(
        "armor.toml",
        "reduction_pct = 25",
        "reduction_pct = 60",
        "band break: `item.armor_roadsign_body`",
    );
    // Headshot: ×5 would one-tap past the TTK band; ×2 exactly, banded.
    refuses(
        "weapons.toml",
        "headshot_mult = 2",
        "headshot_mult = 5",
        "band break: headshot",
    );
}

#[test]
fn door_must_stay_weaker_than_wall() {
    refuses(
        "deployables.toml",
        "material = \"wood\"\nhp = 200",
        "material = \"wood\"\nhp = 4000",
        "must stay under",
    );
}

/// The shipped set bakes into the sim's fixed tables, and the baked rows
/// say what the TOML says — the bridge across wall 7 carries the data
/// unchanged.
#[test]
fn bake_carries_the_shipped_numbers() {
    let c = Content::load_dir(&content_dir()).expect("shipped content must load");
    let gc = c.bake_gather().expect("shipped content must bake");

    let wood = c.item_index("item.wood").unwrap();
    let hatchet = c.item_index("item.hatchet_stone").unwrap();
    assert_eq!(gc.item_count as usize, c.items.len());
    assert_eq!(gc.stack_max[wood as usize], 1000, "items.toml wood stack");

    // gatherables.toml gather.tree: output wood, 10 hits, hand 5,
    // stone hatchet 20, weak-spot +50% — read back from the baked table.
    let tree = &gc.nodes[0];
    assert_eq!(tree.output, wood);
    assert_eq!(tree.hits, 10);
    assert_eq!(tree.yield_for(sim_core::gather::NO_ITEM), 5);
    assert_eq!(tree.yield_for(hatchet), 20);
    assert_eq!(tree.weak_pct, 50, "gatherables.toml weak_spot_bonus_pct");
    assert_eq!(gc.nodes[4].weak_pct, 0, "the bush carries no mark");

    // Index mapping is a bijection into 0..len.
    let mut seen = vec![false; c.items.len()];
    for item in &c.items {
        let i = c.item_index(&item.id).unwrap() as usize;
        assert!(!seen[i], "index {i} assigned twice");
        seen[i] = true;
    }
}

/// Two gatherable rows for one archetype cannot bake — the sim holds one
/// def per node kind.
#[test]
fn bake_refuses_duplicate_archetype() {
    let mut srcs = sources();
    let entry = srcs
        .iter_mut()
        .find(|(n, _)| *n == "gatherables.toml")
        .unwrap();
    entry.1.push_str(
        "\n[[gatherable]]\nid = \"gather.bush2\"\narchetype = \"bush\"\n\
         output = \"item.cloth\"\nhits = 1\nweak_spot_bonus_pct = 0\n\n\
         [gatherable.yield_per_hit]\nhand = 10\n",
    );
    let c = build(&srcs).expect("duplicate archetype is a bake error, not a schema error");
    let err = c.bake_gather().expect_err("duplicate archetype baked");
    assert!(err.contains("duplicate gatherable"), "{err}");
}

/// The shipped recipe ladder bakes into the sim's craft table, and the
/// baked rows say what recipes.toml says.
#[test]
fn bake_craft_carries_the_shipped_numbers() {
    let c = Content::load_dir(&content_dir()).expect("shipped content must load");
    let cc = c.bake_craft().expect("shipped recipes must bake");
    assert_eq!(cc.recipe_count as usize, c.recipes.len());

    // recipes.toml recipe.hatchet_stone: 100 wood + 50 stone → 1 hatchet,
    // 15 s at station none — read back from the baked row.
    let idx = c.recipe_index("recipe.hatchet_stone").unwrap() as usize;
    let def = &cc.recipes[idx];
    assert_eq!(def.output, c.item_index("item.hatchet_stone").unwrap());
    assert_eq!(def.out_count, 1);
    assert_eq!(def.ticks, 15 * sim_core::limits::TICK_HZ);
    assert_eq!(def.station, sim_core::craft::STATION_NONE);
    assert_eq!(def.n_inputs, 2);
    let wood = c.item_index("item.wood").unwrap();
    let stone = c.item_index("item.stone").unwrap();
    assert!(def.inputs[..2].contains(&(wood, 100)));
    assert!(def.inputs[..2].contains(&(stone, 50)));

    // Station codes map in schema order.
    let furnace = c.recipe_index("recipe.furnace").unwrap() as usize;
    assert_eq!(
        cc.recipes[furnace].station,
        sim_core::craft::STATION_WORKBENCH1
    );
    let frags = c.recipe_index("recipe.metal_frags").unwrap() as usize;
    assert_eq!(cc.recipes[frags].station, sim_core::craft::STATION_FURNACE);

    // Index mapping is a bijection into 0..len.
    let mut seen = vec![false; c.recipes.len()];
    for r in &c.recipes {
        let i = c.recipe_index(&r.id).unwrap() as usize;
        assert!(!seen[i], "recipe index {i} assigned twice");
        seen[i] = true;
    }
}

/// The craft bake refuses what the sim's capacities or the wire's field
/// widths can't hold — a refused bake is a refused boot.
#[test]
fn bake_craft_refuses_out_of_cap_rows() {
    // seconds 0 can't arm a timer.
    let mut srcs = sources();
    let entry = srcs.iter_mut().find(|(n, _)| *n == "recipes.toml").unwrap();
    entry.1 = entry.1.replace("seconds = 3\n", "seconds = 0\n");
    let c = build(&srcs).expect("zero seconds is a bake error, not a schema error");
    let err = c.bake_craft().expect_err("zero-second recipe baked");
    assert!(err.contains("seconds"), "{err}");

    // A fifth input exceeds MAX_RECIPE_INPUTS.
    let mut srcs = sources();
    let entry = srcs.iter_mut().find(|(n, _)| *n == "recipes.toml").unwrap();
    entry.1 = entry.1.replace(
        "inputs = [{ item = \"item.stone\", count = 15 }]",
        "inputs = [\n    { item = \"item.stone\", count = 15 },\n    { item = \"item.wood\", count = 1 },\n    { item = \"item.cloth\", count = 1 },\n    { item = \"item.fat\", count = 1 },\n    { item = \"item.charcoal\", count = 1 },\n]",
    );
    let c = build(&srcs).expect("five inputs is a bake error, not a schema error");
    let err = c.bake_craft().expect_err("five-input recipe baked");
    assert!(err.contains("inputs"), "{err}");
}
