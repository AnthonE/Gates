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

/// The same, one stage later: content the *validator* is happy with and the
/// **bake** refuses. Its own helper because the two stages answer different
/// questions — `validate` asks whether the data is coherent, `bake` asks
/// whether the sim can represent it, and a limit that belongs to the sim's
/// machinery (a table width, a sampler's step) has no business being a
/// validation rule.
fn refuses_bake(file: &str, from: &str, to: &str, phrase: &str) {
    let mut srcs = sources();
    let entry = srcs.iter_mut().find(|(n, _)| *n == file).expect(file);
    assert!(
        entry.1.contains(from),
        "test fixture rot: `{from}` not in {file}"
    );
    entry.1 = entry.1.replace(from, to);
    let c = build(&srcs)
        .unwrap_or_else(|e| panic!("{file}: `{to}` should reach the bake, but validate said: {e}"));
    let err = c
        .bake_combat()
        .expect_err(&format!("{file}: `{to}` was baked"));
    assert!(
        err.contains(phrase),
        "{file}: expected bake error containing `{phrase}`, got: {err}"
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
    // The raid lane by hand: the door is breachable, every wall is not,
    // and the ladder rises. Same shape-pinning as the ratio above.
    assert!(a.door_breach_swings >= 30 && a.door_breach_swings <= 80);
    assert!(a.wall_breach_swings[0] >= 150);
    assert!(
        a.wall_breach_swings[0] < a.wall_breach_swings[1]
            && a.wall_breach_swings[1] < a.wall_breach_swings[2]
    );
    assert!(
        a.door_breach_swings < a.wall_breach_swings[0],
        "the door must stay the cheapest way in"
    );
}

#[test]
fn hash_moves_with_values() {
    let base = build(&sources()).unwrap().hash();
    let mut srcs = sources();
    let items = srcs.iter_mut().find(|(n, _)| *n == "items.toml").unwrap();
    items.1 = items.1.replace("stack = 1000", "stack = 999");
    let moved = build(&srcs).unwrap().hash();
    assert_ne!(base, moved, "a value change must move the content hash");

    // Every value the sim reads, including the ones added last: a field
    // that reaches the sim and not the hash lets two contents that play
    // differently canonicalise identically, and a replay is then handed a
    // WAL header claiming a match it does not have.
    let mut srcs = sources();
    let g = srcs
        .iter_mut()
        .find(|(n, _)| *n == "gatherables.toml")
        .unwrap();
    g.1 = g.1.replace("per_hit = 5", "per_hit = 4");
    assert_ne!(
        base,
        build(&srcs).unwrap().hash(),
        "the side payout's rate must move the content hash"
    );

    // `hits` decides how long a barrel takes to open, so two contents that
    // disagree about it play differently and must not canonicalise the
    // same. It reaches the sim through `bake_loot`.
    let mut srcs = sources();
    let l = srcs.iter_mut().find(|(n, _)| *n == "loot.toml").unwrap();
    l.1 = l.1.replace("hits = 3", "hits = 4");
    assert_ne!(
        base,
        build(&srcs).unwrap().hash(),
        "the barrel's hits-to-open must move the content hash"
    );

    // The repair price. It reaches the sim through `bake_building` into
    // `BuildContent::repair_pct`, so two shards disagreeing about it play
    // a materially different raid — one where a wall costs its own worth
    // to mend and one where it costs a fraction. A replay handed a WAL
    // header that matched across that difference would resim a base back
    // to full on materials the recorded session never had.
    let mut srcs = sources();
    let b = srcs.iter_mut().find(|(n, _)| *n == "balance.toml").unwrap();
    b.1 = b.1.replace("repair_cost_pct = 100", "repair_cost_pct = 60");
    assert_ne!(
        base,
        build(&srcs).unwrap().hash(),
        "the repair price must move the content hash"
    );

    // The satchel's fuse. It reaches the sim through `bake_combat` into
    // `ThrowDef::fuse_ticks`, and it is the newest field on the newest
    // path, which is exactly the shape `canon.rs` has been caught by twice
    // before: a field added to `schema.rs` and to `bake.rs` and not to the
    // canonical walk. Two shards disagreeing about how long a charge burns
    // play a materially different raid — the defender's seconds to answer
    // are the whole mechanic — so this is checked here rather than left to
    // whoever notices.
    let mut srcs = sources();
    let w = srcs.iter_mut().find(|(n, _)| *n == "weapons.toml").unwrap();
    w.1 = w.1.replace("fuse_s = 10", "fuse_s = 11");
    assert_ne!(
        base,
        build(&srcs).unwrap().hash(),
        "the satchel's fuse must move the content hash"
    );
}

/// The raid tool reaches the sim, and the raid ratio is arithmetic a
/// player can actually spend.
///
/// `content/balance.toml`'s anchor 1 divides a wall's hp by the throwable's
/// `structure` column, and for every version up to this one that division
/// had no verb behind it: `bake_combat` dropped every non-melee row, so the
/// number the whole economy is balanced around reached the sim as a zero.
/// A content file can state a raid ratio and a sim can be unable to raid,
/// and nothing between them would notice — which is what this asserts
/// against.
#[test]
fn the_raid_tool_crosses_into_the_sim() {
    let c = build(&sources()).expect("shipped content bakes");
    let cc = c.bake_combat().expect("combat bakes");
    let idx = c
        .item_index("item.satchel_charge")
        .expect("the satchel is an item") as usize;
    let def = cc.throw[idx];
    assert!(
        def.structure > 0,
        "the satchel baked with no structure damage — the raid ratio \
         divides wall hp by this number and would be dividing by nothing"
    );
    assert!(
        def.fuse_ticks > 0,
        "the satchel baked with no fuse — a charge that never blows is a \
         priced item with no verb, which is the gap this landed to close"
    );
    assert!(
        cc.held_throw(idx as u16).is_some(),
        "the sim cannot recognise the shipped raid tool in a hand"
    );

    // And it must actually get through a wall. Stone is the tier the
    // anchor names; the assert is that a finite number of charges breaches
    // it, not what that number is — the count itself is `balance.toml`'s
    // to tune and `balance.rs`'s to band.
    let bc = c.bake_building().expect("building bakes");
    let stone_wall = (0..bc.piece_count as usize)
        .map(|i| bc.pieces[i].hp)
        .max()
        .expect("the piece table is not empty");
    assert!(
        stone_wall > 0 && def.structure > 0,
        "a zero on either side makes the division below meaningless"
    );
    let charges = stone_wall.div_ceil(def.structure);
    assert!(
        (1..=64).contains(&charges),
        "the toughest piece takes {charges} charges — below 1 is a wall \
         that falls to nothing, and past `MAX_LIVE_CHARGES` no single \
         raider can carry the breach"
    );
}

/// A deployable's repair price is its recipe, joined at bake time.
///
/// Placement charges one crafted item, and a repair cannot be priced that
/// way: a fraction of one item rounds up to the whole thing, so mending a
/// scratched door would cost a whole door and nobody would ever do it. The
/// bake therefore copies the recipe's inputs onto the deployable row, and
/// this is the gate that the join actually happened and landed the right
/// numbers — a `n_costs` of 0 here reads as "unpriced" at runtime and
/// refuses the verb outright (`build::REFUSE_B_UNPRICED`), so a silent
/// failure of this join would look exactly like a door that cannot be
/// repaired at all.
#[test]
fn a_deployable_is_priced_for_repair_by_its_own_recipe() {
    let c = build(&sources()).expect("shipped content bakes");
    let dc = c.bake_deployables().expect("deploy table bakes");
    let mut checked = 0;
    for d in &c.deployables {
        let row = c.deploy_index(&d.id).expect("own id resolves") as usize;
        let def = dc.defs[row];
        let recipe = c
            .recipe_for(&d.id)
            .unwrap_or_else(|| panic!("`{}` has no recipe to price a repair against", d.id));
        assert_eq!(
            def.n_costs as usize,
            recipe.inputs.len(),
            "`{}` must carry every one of its recipe's inputs as a repair \
             cost row, or the price is quoted against half the materials",
            d.id
        );
        for (n, input) in recipe.inputs.iter().enumerate() {
            let item = c.item_index(&input.item).expect("input resolves");
            assert_eq!(
                def.costs[n],
                (item, input.count as u16),
                "`{}` repair cost row {n} must be its recipe's input, item \
                 and units in that order",
                d.id
            );
        }
        assert!(
            def.n_costs > 0,
            "`{}` bakes unpriced, so it could never be repaired",
            d.id
        );
        checked += 1;
    }
    assert!(
        checked >= 9,
        "the alpha set is 9 deployables; only {checked} were priced"
    );
}

/// The repair price's two live edges, refused at boot rather than played.
///
/// Both ends are a defect and neither is a taste. At `0` a wall heals for
/// nothing and no raid on the shard can ever land; over `100` a mend costs
/// more than the damage destroyed, which is a rebuild with extra steps and
/// makes the verb dead weight. `100` itself is the ceiling and must stay
/// legal — it is the shipped default (`DECISIONS.md` §open, repair v0).
#[test]
fn repair_price_bands_refused() {
    for bad in ["0", "101", "1000"] {
        refuses(
            "balance.toml",
            "repair_cost_pct = 100",
            &format!("repair_cost_pct = {bad}"),
            "repair_cost_pct",
        );
    }
    let mut srcs = sources();
    let b = srcs.iter_mut().find(|(n, _)| *n == "balance.toml").unwrap();
    b.1 = b.1.replace("repair_cost_pct = 100", "repair_cost_pct = 1");
    build(&srcs).expect("1% is legal — cheap is the operator's call to make");
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
    // Raid ratio: a 100-structure satchel needs 18 to open stone — past
    // 3×. The ratio divides by the `structure` column, not `damage`.
    refuses(
        "weapons.toml",
        "structure = 500",
        "structure = 100",
        "band break: stone raid ratio",
    );
    // A weapon better against a wall than a person is refused outright,
    // no band consulted: the spear that kills in 25 cannot chip 99.
    refuses(
        "weapons.toml",
        "damage = 25\nstructure = 3",
        "damage = 25\nstructure = 99",
        "exceeds its own body damage",
    );
    // Door breach: a 20-structure spear opens the wood door in 10 swings,
    // under the band's 30 — the breach point stops being a raid.
    refuses(
        "weapons.toml",
        "damage = 30\nstructure = 4",
        "damage = 30\nstructure = 20",
        "band break: door breach swings",
    );
    // Wall floor: at 6 the door still lands in band (34 swings) but the
    // wood wall falls in 125, under the 150 floor — a spear undercutting
    // the satchel is exactly what the floor is for.
    refuses(
        "weapons.toml",
        "damage = 30\nstructure = 4",
        "damage = 30\nstructure = 6",
        "melee swings",
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

/// The upgrade ladder is whole: every shape that has a stone or metal rung
/// has the rung below it (sim-core build.rs climbs shape by shape, so a
/// hole is a piece nothing can ever be upgraded into). Take the wood roof
/// out and the set must refuse to load.
#[test]
fn upgrade_ladder_must_be_whole() {
    refuses(
        "building.toml",
        "[[piece]]\nid = \"build.roof_wood\"\nshape = \"roof\"\nmaterial = \"wood\"\nhp = 750\ncost = [{ item = \"item.wood\", count = 350 }]\n",
        "",
        "upgrade ladder must be whole",
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

/// The shipped building set bakes into the sim's piece table, and the
/// baked rows say what building.toml says.
#[test]
fn bake_building_carries_the_shipped_numbers() {
    let c = Content::load_dir(&content_dir()).expect("shipped content must load");
    let bc = c.bake_building().expect("shipped building set must bake");
    assert_eq!(bc.piece_count as usize, c.pieces.len());

    // building.toml build.wall_stone: shape wall, stone, hp 1750,
    // 350 stone — read back from the baked row.
    let idx = c.piece_index("build.wall_stone").unwrap() as usize;
    let def = &bc.pieces[idx];
    assert_eq!(def.shape, sim_core::build::SHAPE_WALL);
    assert_eq!(def.material, sim_core::build::MAT_STONE);
    assert_eq!(def.hp, 1750);
    assert_eq!(def.n_costs, 1);
    let stone = c.item_index("item.stone").unwrap();
    assert_eq!(def.costs[0], (stone, 350));

    // Index mapping is a bijection into 0..len.
    let mut seen = vec![false; c.pieces.len()];
    for p in &c.pieces {
        let i = c.piece_index(&p.id).unwrap() as usize;
        assert!(!seen[i], "piece index {i} assigned twice");
        seen[i] = true;
    }
}

/// The building bake refuses what the sim's capacities or the wire's
/// field widths can't hold.
#[test]
fn bake_building_refuses_out_of_cap_rows() {
    // hp past u16 can't cross the wire's width.
    let mut srcs = sources();
    let entry = srcs
        .iter_mut()
        .find(|(n, _)| *n == "building.toml")
        .unwrap();
    entry.1 = entry.1.replacen("hp = 3000\n", "hp = 70000\n", 1);
    let c = build(&srcs).expect("oversize hp is a bake error, not a schema error");
    let err = c.bake_building().expect_err("70000-hp piece baked");
    assert!(err.contains("overflows u16"), "{err}");

    // A third cost row exceeds MAX_PIECE_COSTS.
    let mut srcs = sources();
    let entry = srcs
        .iter_mut()
        .find(|(n, _)| *n == "building.toml")
        .unwrap();
    entry.1 = entry.1.replacen(
        "cost = [{ item = \"item.wood\", count = 350 }]",
        "cost = [\n    { item = \"item.wood\", count = 350 },\n    { item = \"item.stone\", count = 1 },\n    { item = \"item.cloth\", count = 1 },\n]",
        1,
    );
    let c = build(&srcs).expect("three costs is a bake error, not a schema error");
    let err = c.bake_building().expect_err("three-cost piece baked");
    assert!(err.contains("cost rows"), "{err}");

    // Two cost rows naming the same item: the sim checks each row's
    // affordability independently, so a double-listed item would pass
    // the check yet under-collect — refuse at bake.
    let mut srcs = sources();
    let entry = srcs
        .iter_mut()
        .find(|(n, _)| *n == "building.toml")
        .unwrap();
    entry.1 = entry.1.replacen(
        "cost = [{ item = \"item.wood\", count = 350 }]",
        "cost = [\n    { item = \"item.wood\", count = 350 },\n    { item = \"item.wood\", count = 1 },\n]",
        1,
    );
    let c = build(&srcs).expect("a duplicate cost item is a bake error, not a schema error");
    let err = c
        .bake_building()
        .expect_err("double-listed cost item baked");
    assert!(err.contains("twice"), "{err}");
}

/// The deployable bake carries the shipped rows + the upkeep globals.
#[test]
fn bake_deployables_carries_the_shipped_numbers() {
    let c = Content::load_dir(&content_dir()).expect("shipped content must load");
    let dc = c.bake_deployables().expect("shipped deployables must bake");
    assert_eq!(dc.def_count as usize, c.deployables.len());

    // deployables.toml item.hearth: hearth archetype, foundation
    // placement, hp 500 — read back from the baked row.
    let idx = c.deploy_index("item.hearth").unwrap() as usize;
    let def = &dc.defs[idx];
    assert_eq!(def.arch, sim_core::deploy::ARCH_HEARTH);
    assert_eq!(def.placement, sim_core::deploy::PLACE_FOUNDATION);
    assert_eq!(def.hp, 500);
    assert_eq!(def.item, c.item_index("item.hearth").unwrap());

    // The doors keep their doorway placement and material pairing.
    let idx = c.deploy_index("item.door_wood").unwrap() as usize;
    assert_eq!(dc.defs[idx].arch, sim_core::deploy::ARCH_DOOR);
    assert_eq!(dc.defs[idx].placement, sim_core::deploy::PLACE_DOORWAY);

    // Upkeep materials are exactly the distinct build-cost items,
    // ascending, and the pct is balance.toml's global.
    let wood = c.item_index("item.wood").unwrap();
    let stone = c.item_index("item.stone").unwrap();
    let frags = c.item_index("item.metal_frags").unwrap();
    let mut want = [wood, stone, frags];
    want.sort_unstable();
    assert_eq!(dc.mat_count, 3);
    assert_eq!(&dc.mats[..3], &want);
    assert_eq!(
        dc.upkeep_pct_per_day as u32,
        c.balance.globals.upkeep_pct_per_day
    );

    // Index mapping is a bijection into 0..len.
    let mut seen = vec![false; c.deployables.len()];
    for d in &c.deployables {
        let i = c.deploy_index(&d.id).unwrap() as usize;
        assert!(!seen[i], "deploy index {i} assigned twice");
        seen[i] = true;
    }
}

/// The deployable bake refuses what the sim's capacities can't hold.
#[test]
fn bake_deployables_refuses_out_of_cap_rows() {
    // hp past u16 can't cross the wire's width.
    let mut srcs = sources();
    let entry = srcs
        .iter_mut()
        .find(|(n, _)| *n == "deployables.toml")
        .unwrap();
    entry.1 = entry.1.replacen("hp = 500\n", "hp = 70000\n", 1);
    let c = build(&srcs).expect("oversize hp is a bake error, not a schema error");
    let err = c.bake_deployables().expect_err("70000-hp deployable baked");
    assert!(err.contains("overflows u16"), "{err}");
}

/// The combat table bakes from the shipped weapons, and — the point of
/// the test — **the band the data declares is the band the sim plays.**
/// `anchors()` asserts TTK from `weapons.toml` ÷ `balance.toml`; this
/// re-derives the same hits-to-kill from the *baked* rows the sim will
/// actually run on, so a bake that silently rounded, truncated, or keyed
/// a row to the wrong item would show up here rather than as a fight that
/// takes one more hit than the doc says.
#[test]
fn bake_combat_plays_the_band_the_data_declares() {
    let c = Content::load_dir(&content_dir()).expect("shipped content must load");
    let cc = c.bake_combat().expect("shipped weapons must bake");
    assert_eq!(
        cc.player_hp as u32, c.balance.globals.player_hp,
        "max hp is balance.toml's, not a code constant"
    );

    let [lo, hi] = c.balance.bands.ttk_melee;
    let mut melee_rows = 0;
    for w in &c.weapons {
        let idx = c.item_index(&w.id).expect("weapon arms an item") as usize;
        let baked = cc.melee[idx];
        if w.kind != content::schema::WeaponKind::Melee {
            assert_eq!(
                baked.damage, 0,
                "`{}` is not melee and must not be armed in v0",
                w.id
            );
            continue;
        }
        melee_rows += 1;
        assert_eq!(baked.damage as u32, w.damage, "`{}` damage", w.id);
        assert_eq!(
            baked.reach_cm as u32,
            w.range_m * 100,
            "`{}` reach in cm",
            w.id
        );
        // Hits to kill, computed the way the sim computes it: whole
        // swings, each removing `damage` from `player_hp`.
        let mut hp = cc.player_hp;
        let mut hits = 0u32;
        while hp > 0 {
            hp -= baked.damage.min(hp);
            hits += 1;
        }
        assert!(
            (lo..=hi).contains(&hits),
            "`{}` kills in {hits} swings, outside the declared melee TTK band {lo}..={hi}",
            w.id
        );
    }
    assert!(
        melee_rows >= 2,
        "the shipped set must arm more than one melee weapon or this test proves nothing"
    );
}

/// A melee row that deals nothing or reaches nowhere never reaches the
/// sim: it is refused, and it does not matter to this test whether the
/// refusal comes from validation or from the bake — what matters is that
/// no boot path exists that hands the world a weapon which cannot work.
/// Asserting the *stage* would gate the plumbing; asserting the outcome
/// gates the rule.
#[test]
fn a_melee_row_that_cannot_work_never_reaches_the_sim() {
    for patch in ["damage = 0", "range_m = 0"] {
        let mut srcs = sources();
        let entry = srcs.iter_mut().find(|(n, _)| *n == "weapons.toml").unwrap();
        let field = patch.split(' ').next().unwrap();
        // Rewrite the first matching field rather than appending a row:
        // an appended row would be a second definition, not a broken one.
        let mut out = String::new();
        let mut patched = false;
        for line in entry.1.lines() {
            if !patched && line.starts_with(&format!("{field} = ")) {
                out.push_str(patch);
                patched = true;
            } else {
                out.push_str(line);
            }
            out.push('\n');
        }
        assert!(patched, "weapons.toml has no `{field}` line to break");
        entry.1 = out;
        let refused = match build(&srcs) {
            Err(e) => e,
            Ok(c) => c
                .bake_combat()
                .err()
                .unwrap_or_else(|| panic!("`{patch}` armed the sim anyway")),
        };
        assert!(
            !refused.is_empty(),
            "`{patch}` was refused without saying why"
        );
    }
}

/// The backpack despawn ladder bakes from the shipped balance table and
/// the rarity column `items.toml` has always declared drives it — and,
/// the point of the test, **the sim's lifetime is content's arithmetic,
/// never a code constant.** A bake that dropped the multiplier, keyed a
/// row to the wrong item, or read minutes as ticks would show up here as
/// a bag that outlives or undercuts what the file says.
#[test]
fn bake_backpack_walks_the_rarity_ladder_the_data_declares() {
    let c = Content::load_dir(&content_dir()).expect("shipped content must load");
    let bc = c.bake_backpack().expect("shipped balance must bake");
    let bp = &c.balance.backpack;
    let tick_hz = sim_core::limits::TICK_HZ;

    assert_eq!(
        bc.base_ticks,
        bp.despawn_base_min * 60 * tick_hz,
        "the floor is balance.toml's minutes at the sim's rate"
    );
    let mults = bp.mults();
    let mut seen = [false; 4];
    for item in &c.items {
        let idx = c.item_index(&item.id).expect("own id") as usize;
        let r = item.rarity.canon() as usize;
        seen[r] = true;
        assert_eq!(
            bc.despawn_ticks[idx],
            bc.base_ticks * mults[r],
            "`{}` ({:?}) must live base × its own multiplier",
            item.id,
            item.rarity
        );
    }
    assert!(seen[0], "the shipped set must exercise at least `common`");

    // A bag holding the rarest thing the set ships lives longest, and a
    // bag of nothing but the commonest rides the floor — the two ends of
    // `lifetime_ticks`, computed from the shipped table, not a fixture.
    let rarest = c
        .items
        .iter()
        .max_by_key(|i| i.rarity.canon())
        .expect("the set is non-empty");
    let commonest = c
        .items
        .iter()
        .min_by_key(|i| i.rarity.canon())
        .expect("the set is non-empty");
    let mut inv = [sim_core::gather::ItemStack::default(); sim_core::limits::INV_SLOTS];
    inv[0] = sim_core::gather::ItemStack {
        item: c.item_index(&commonest.id).unwrap(),
        count: 1,
    };
    assert_eq!(bc.lifetime_ticks(&inv), bc.base_ticks);
    inv[1] = sim_core::gather::ItemStack {
        item: c.item_index(&rarest.id).unwrap(),
        count: 1,
    };
    assert_eq!(
        bc.lifetime_ticks(&inv),
        bc.base_ticks * mults[rarest.rarity.canon() as usize],
        "one rare thing raises the whole bag"
    );
}

/// The ladder's two failure modes are refused at the boot edge: a base
/// nobody set, and a ladder that does not rise. A falling ladder would
/// make a rarer bag despawn *sooner* than a common one — the exact
/// inversion NETCODE.md §6.4's tier shape exists to prevent.
#[test]
fn a_backpack_ladder_that_does_not_rise_is_refused() {
    refuses(
        "balance.toml",
        "mult_uncommon = 4",
        "mult_uncommon = 1",
        "rise strictly",
    );
    refuses(
        "balance.toml",
        "despawn_base_min = 5",
        "despawn_base_min = 0",
        "≥ 1 minute",
    );
    refuses(
        "balance.toml",
        "mult_common = 1",
        "mult_common = 0",
        "mult_common must be ≥ 1",
    );
}

/// The survival clock, computed from the shipped data rather than
/// restated: the spans the sim plays are `balance.toml`'s minutes at the
/// sim's tick rate, every authored consumable reaches the table it is
/// keyed into, and the meters cannot be drained by a rate that empties
/// them faster than the design says.
#[test]
fn bake_survival_plays_the_clock_the_data_declares() {
    let c = Content::load_dir(&content_dir()).expect("shipped content must load");
    let sc = c.bake_survival().expect("shipped survival clock must bake");
    let s = &c.balance.survival;
    let tick_hz = sim_core::limits::TICK_HZ;

    assert_eq!(sc.max_food as u32, s.max_food);
    assert_eq!(sc.max_water as u32, s.max_water);
    assert_eq!(
        sc.food_span_ticks,
        s.food_minutes_to_empty * 60 * tick_hz,
        "the span the sim plays is balance.toml's minutes at the sim's rate"
    );
    assert_eq!(sc.water_span_ticks, s.water_minutes_to_empty * 60 * tick_hz);
    assert!(
        sc.water_span_ticks < sc.food_span_ticks,
        "thirst is the shorter fuse (DESIGN §2, and every game in the tradition)"
    );

    // Every authored row reaches the sim, keyed to its own item, and
    // nothing else in the table is food. A row that silently failed to
    // bake would be exactly the defect this whole slice exists to fix.
    let mut armed = 0;
    for con in &c.consumables {
        let idx = c
            .item_index(&con.id)
            .expect("a validated consumable names a real item") as usize;
        let row = sc.consumable[idx];
        assert_eq!(row.health as u32, con.health, "`{}` health", con.id);
        assert_eq!(row.food as u32, con.food, "`{}` food", con.id);
        assert_eq!(row.water as u32, con.water, "`{}` water", con.id);
        assert_eq!(row.seconds as u32, con.seconds, "`{}` seconds", con.id);
        assert!(row.is_food(), "`{}` must read as food", con.id);
        armed += 1;
    }
    assert!(armed >= 3, "the shipped set must author real food");
    let table_food = sc.consumable.iter().filter(|r| r.is_food()).count();
    assert_eq!(
        table_food, armed,
        "no item may be food the content did not author"
    );

    // The clock must not kill faster than the design's floor, computed
    // against the same `player_hp` the TTK anchor divides by — so the
    // survival band and the combat band cannot drift apart.
    let hp = c.balance.globals.player_hp;
    let worst = s.starve_hp_per_min + s.dehydrate_hp_per_min;
    assert!(
        hp / worst >= 5,
        "both meters empty must leave at least 5 minutes of {hp} hp at {worst} hp/min"
    );

    // And an untended fresh spawn must survive long enough that the
    // session is gathering and building, not eating: the first point of
    // damage lands no sooner than the shorter span.
    assert!(
        s.water_minutes_to_empty >= 30,
        "a fresh spawn gets at least half an hour before the clock bites"
    );

    // **The answer, priced in the clock's own units.** A validator refuses
    // content with no food source at all; this is the arithmetic that says
    // the source is worth walking to. One bush pickup must buy back a
    // meaningful share of a span, or the loop is a treadmill that happens
    // to pass a boolean.
    let gc = c.bake_gather().expect("shipped gather table must bake");
    let mut best_food_min = 0u32;
    let mut best_water_min = 0u32;
    for node in gc.nodes.iter() {
        for (item, per_hit) in [
            (node.output, node.hand_yield),
            (node.secondary.0, node.secondary.1),
        ] {
            if item == sim_core::gather::NO_ITEM || per_hit == 0 {
                continue;
            }
            let row = sc.consumable[item as usize];
            // Units of meter one pickup pays, over units the meter loses
            // in a minute — the ratio is minutes bought, and both sides
            // are the file's own numbers.
            best_food_min = best_food_min
                .max((row.food as u32 * per_hit as u32 * s.food_minutes_to_empty) / s.max_food);
            best_water_min = best_water_min
                .max((row.water as u32 * per_hit as u32 * s.water_minutes_to_empty) / s.max_water);
        }
    }
    assert!(
        best_food_min >= 20,
        "the best food a node pays buys {best_food_min} min of a \
         {}-min hunger span — a source nobody would cross the island for",
        s.food_minutes_to_empty
    );
    assert!(
        best_water_min >= 5,
        "the best water a node pays buys {best_water_min} min of a \
         {}-min thirst span",
        s.water_minutes_to_empty
    );
}

/// The bush's side payout reaches the sim: the shipped table's berries,
/// keyed to the berry item, at the file's rate — and no other archetype
/// grew one by accident.
#[test]
fn bake_gather_carries_the_side_payout() {
    let c = Content::load_dir(&content_dir()).expect("shipped content must load");
    let gc = c.bake_gather().expect("shipped gather table must bake");
    let mut declared = 0;
    for g in &c.gatherables {
        let Some(s) = &g.secondary else { continue };
        declared += 1;
        let idx = c.item_index(&s.output).expect("validated secondary");
        let node = gc
            .nodes
            .iter()
            .find(|n| n.secondary.0 == idx)
            .unwrap_or_else(|| panic!("`{}` secondary never reached the sim", g.id));
        assert_eq!(
            node.secondary.1 as u32, s.per_hit,
            "`{}` secondary pays the file's rate",
            g.id
        );
        assert_ne!(node.secondary.0, node.output, "two payouts, two items");
    }
    assert_eq!(
        declared,
        gc.nodes
            .iter()
            .filter(|n| n.secondary.0 != sim_core::gather::NO_ITEM)
            .count(),
        "the sim has exactly as many side payouts as the files declare"
    );
}

/// Every shape that would make the clock silently inert is refused at
/// load. Silence is the failure mode that matters here: a zero span or a
/// zero rate does not crash, it just quietly gives back the game where
/// standing still is free — which is the gap this slice closes.
#[test]
fn a_clock_that_would_not_tick_is_refused() {
    refuses(
        "balance.toml",
        "max_water = 100",
        "max_water = 0",
        "must be ≥ 1",
    );
    refuses(
        "balance.toml",
        "water_minutes_to_empty = 40",
        "water_minutes_to_empty = 0",
        "must be ≥ 1",
    );
    refuses(
        "balance.toml",
        "dehydrate_hp_per_min = 5",
        "dehydrate_hp_per_min = 0",
        "must be ≥ 1",
    );
    // Thirst before hunger is the shape, not a tuning choice.
    refuses(
        "balance.toml",
        "water_minutes_to_empty = 40",
        "water_minutes_to_empty = 90",
        "must empty before food",
    );
    // A clock that kills a full-hp body faster than it takes to notice.
    refuses(
        "balance.toml",
        "starve_hp_per_min = 3",
        "starve_hp_per_min = 40",
        "under 5 min",
    );
    // A heal needs a span to arrive over — the sim's ramp divides by it.
    refuses(
        "consumables.toml",
        "health = 20\nfood = 0\nwater = 0\nseconds = 4",
        "health = 20\nfood = 0\nwater = 0\nseconds = 0",
        "health needs a span",
    );
}

/// The other way a clock fails silently: it ticks perfectly and the island
/// has nothing to answer it with. That shipped — five consumable rows
/// parsed, validated and hashed while `gather.bush` paid cloth and nothing
/// else in the world paid a unit of food (`findings/archive-prestamp/
/// pass-20260803-041958-02-judge.md`, ranked gap 1). Both halves are
/// refused now, and the refusals are read off the same bush the shipped
/// content answers them with.
#[test]
fn a_clock_with_no_answer_is_refused() {
    // Take the berries off the bush: hunger drains, nothing pays food.
    refuses(
        "gatherables.toml",
        "[gatherable.secondary]\noutput = \"item.berries\"",
        "[gatherable.secondary]\noutput = \"item.cloth_UNUSED\"",
        "is not an item",
    );
    // The honest version of the same defect — the row simply absent.
    refuses(
        "gatherables.toml",
        "\n[gatherable.secondary]\noutput = \"item.berries\"\nper_hit = 5\n",
        "\n",
        "the clock has no answer",
    );
    // And the thirst half. It takes **both** answers off the island now:
    // the drink verb (wire v15) is the second way to answer thirst, so
    // berries that feed but do not water are no longer enough on their own
    // to leave the shorter fuse unanswerable — which is the widening, and
    // the case below is what pins it.
    let mut srcs = sources();
    for (name, text) in srcs.iter_mut() {
        if *name == "consumables.toml" {
            *text = text.replace(
                "id = \"item.berries\"\nhealth = 0\nfood = 15\nwater = 5",
                "id = \"item.berries\"\nhealth = 0\nfood = 15\nwater = 0",
            );
        }
    }
    build(&srcs).expect(
        "a bush that pays no water but an armed drink verb answers thirst — \
         that is exactly what wire v15 widened the wall for",
    );
    // Disarm the drink as well and the island is dry: no gatherable pays
    // water and no verb draws it, so the clock has no answer again.
    for (name, text) in srcs.iter_mut() {
        if *name == "balance.toml" {
            // Both to zero: a cost with no water is its own refusal one
            // check earlier, and it would answer for the wrong reason.
            *text = text.replace(
                "drink_water = 25\ndrink_hp_cost = 2",
                "drink_water = 0\ndrink_hp_cost = 0",
            );
        }
    }
    let err = build(&srcs).expect_err("a dry island with a disarmed drink must be refused");
    assert!(
        err.contains("the clock has no answer"),
        "expected the unanswerable-clock refusal, got: {err}"
    );
}

/// The drink's own bounds. Both would ship a verb that is worse than not
/// having one — a cost with no purchase, and a mouthful that kills.
#[test]
fn a_drink_that_is_not_a_trade_is_refused() {
    refuses(
        "balance.toml",
        "drink_water = 25\ndrink_hp_cost = 2",
        "drink_water = 0\ndrink_hp_cost = 2",
        "restores no water",
    );
    refuses(
        "balance.toml",
        "drink_water = 25\ndrink_hp_cost = 2",
        "drink_water = 25\ndrink_hp_cost = 100",
        "the sea is salt, not lethal",
    );
}

/// A side payout has to be a payout: no zero, no repeat of the primary,
/// and no item that does not exist.
#[test]
fn a_secondary_that_pays_nothing_is_refused() {
    refuses(
        "gatherables.toml",
        "output = \"item.berries\"\nper_hit = 5",
        "output = \"item.berries\"\nper_hit = 0",
        "pays nothing",
    );
    refuses(
        "gatherables.toml",
        "[gatherable.secondary]\noutput = \"item.berries\"",
        "[gatherable.secondary]\noutput = \"item.cloth\"",
        "repeats the primary output",
    );
}

/// The shipped loot tables reach the sim, and reach it intact.
///
/// A table that parses, validates and hashes but never bakes is what
/// `content/loot.toml` was until this slice: eight authored rows and a
/// revolver that nothing could drop. The assertions below are the ones a
/// silent mis-bake would break — the rare row surviving at weight 1, the
/// weight sum matching what the rows say, and the container name
/// resolving to the index the sim's verb uses.
#[test]
fn the_shipped_loot_tables_bake() {
    let c = build(&sources()).unwrap();
    let lc = c.bake_loot().expect("shipped loot tables must bake");

    let t = lc
        .table(sim_core::loot::LOOT_BARREL)
        .expect("the barrel table is armed");
    assert_eq!(t.len, 8, "the barrel table lost or gained a row");
    assert_eq!(t.rolls_min, 1);
    assert_eq!(t.rolls_max, 2);
    assert_eq!(t.hits, 3, "the barrel's hits came from content");

    let summed: u32 = t.entries[..t.len as usize]
        .iter()
        .map(|e| e.weight as u32)
        .sum();
    assert_eq!(
        t.total_weight, summed,
        "the baked weight sum disagrees with the rows it was summed from — \
         the roll would pick past the end of the table"
    );

    // The rarest thing on the island survived the bake at its authored
    // weight. A revolver quietly baked to weight 0 is unreachable loot
    // that every other gate would call fine.
    let revolver = c.item_index("item.revolver").expect("shipped item");
    let row = t.entries[..t.len as usize]
        .iter()
        .find(|e| e.item == revolver)
        .expect("the revolver is still a barrel drop");
    assert_eq!(row.weight, 1, "the revolver's rarity moved");
    assert_eq!((row.count_min, row.count_max), (1, 1));

    // Every row names a real item and a sane band.
    for e in &t.entries[..t.len as usize] {
        assert!(e.weight > 0, "a zero-weight row baked");
        assert!(e.count_min > 0 && e.count_min <= e.count_max, "bad band");
        assert!(
            (e.item as usize) < sim_core::limits::MAX_ITEM_DEFS,
            "a row baked an item index past the sim's table"
        );
    }

    // The crate table is authored and armed even though nothing spawns one
    // yet; the world lane's monument is what will reach it.
    let k = lc
        .table(sim_core::loot::LOOT_CRATE)
        .expect("the crate table is armed");
    assert_eq!(k.len, 9);
    assert_eq!(k.hits, 5);
}

/// A container the sim has no verb for is a bake refusal, not a row that
/// is silently dropped — a crate table that never spawns and never says so
/// is the failure this catches.
#[test]
fn an_unknown_container_is_refused_at_bake() {
    let mut srcs = sources();
    let l = srcs.iter_mut().find(|(n, _)| *n == "loot.toml").unwrap();
    l.1 =
        l.1.replace("container = \"crate\"", "container = \"lockbox\"");
    let err = build(&srcs)
        .unwrap()
        .bake_loot()
        .expect_err("`lockbox` was accepted");
    assert!(err.contains("no verb for"), "got: {err}");
}

/// Wall 4 on the roll loop: `rolls_max` is per-tick work chosen by
/// content, so a table past `MAX_LOOT_ROLLS` must not boot.
///
/// The number here is not a strawman. `rolls_max` is read as a `u32` and
/// narrowed by the bake's `small()`, whose only bound is `u16::MAX` — so
/// before this cap existed, `65_535` was valid content, and one smash then
/// walked a 16-row weight table 65_535 times inside a single tick. Nothing
/// else would have caught it: the arithmetic is integer, the store is
/// fixed-capacity, and the allocator never moves.
#[test]
fn a_roll_count_past_the_cap_is_refused_at_bake() {
    let mut srcs = sources();
    let l = srcs.iter_mut().find(|(n, _)| *n == "loot.toml").unwrap();
    l.1 = l.1.replacen("rolls_max = 2", "rolls_max = 65535", 1);
    let err = build(&srcs)
        .unwrap()
        .bake_loot()
        .expect_err("a 65_535-roll table baked");
    assert!(err.contains("per smash"), "got: {err}");

    // And the cap itself is the container's slot count, so the shipped
    // tables sit well under it rather than against it.
    let lc = build(&sources())
        .unwrap()
        .bake_loot()
        .expect("shipped loot must bake");
    for which in [sim_core::loot::LOOT_BARREL, sim_core::loot::LOOT_CRATE] {
        let t = lc.table(which).expect("shipped table is live");
        assert!(
            t.rolls_max as usize <= sim_core::limits::MAX_LOOT_ROLLS,
            "table {which} rolls {} past the {} cap",
            t.rolls_max,
            sim_core::limits::MAX_LOOT_ROLLS
        );
    }
}

/// A container nothing can open never pays, so zero hits is content that
/// disarms itself — refused at validate, before the bake ever sees it.
#[test]
fn a_container_that_cannot_be_opened_is_refused() {
    refuses(
        "loot.toml",
        "rolls_max = 2\nhits = 3",
        "rolls_max = 2\nhits = 0",
        "would never open",
    );
}

/// The bow's ballistics reach the sim as **per-tick integers**, and the
/// numbers are the data's own converted once.
///
/// This is the gate on the conversion boundary that ranged v0 opened. Every
/// number `ranged.rs` integrates with is produced here and nowhere else, so
/// a wrong divisor is a projectile that flies at the right speed in the
/// wrong units — which looks like a balance problem rather than a bug, and
/// is the reason the arithmetic is asserted against `weapons.toml` rather
/// than against a remembered constant.
#[test]
fn bows_bake_to_per_tick_integers_the_sim_can_integrate() {
    use sim_core::limits::{ARROW_STEP_MM, MAX_ARROW_LIFE_TICKS, MAX_ARROW_SUBSTEPS, TICK_HZ};

    let c = Content::load_dir(&content_dir()).expect("shipped content must load");
    let cc = c.bake_combat().expect("shipped weapons must bake");

    let mut bows = 0;
    for w in &c.weapons {
        let idx = c.item_index(&w.id).expect("weapon arms an item") as usize;
        let baked = cc.ranged[idx];
        if w.kind != content::schema::WeaponKind::Bow {
            assert_eq!(
                baked.damage, 0,
                "`{}` is not a bow and must not be armed in the ranged table",
                w.id
            );
            continue;
        }
        bows += 1;
        let b = w
            .ballistic
            .as_ref()
            .expect("validate refuses a bow without one");

        assert_eq!(baked.damage as u32, w.damage, "`{}` damage", w.id);
        assert_eq!(
            baked.speed_mmpt as u32,
            b.speed_mps * 1000 / TICK_HZ,
            "`{}` muzzle speed in mm/tick",
            w.id
        );
        assert_eq!(
            baked.drop_mmpt2 as u32,
            b.drop_mps2 * 1000 / (TICK_HZ * TICK_HZ),
            "`{}` drop in mm/tick^2",
            w.id
        );
        assert_eq!(
            baked.rate_ticks as u32,
            TICK_HZ * 60 / w.rate_per_min,
            "`{}` ticks between shots",
            w.id
        );
        let ammo = w.ammo.as_ref().expect("validate refuses a bow without one");
        assert_eq!(
            baked.ammo,
            c.item_index(ammo).expect("ammo is an item"),
            "`{}` spends the ammo it names",
            w.id
        );

        // The flight must actually cover the reach the data claims, and
        // then stop: a life derived too short makes `range_m` a lie, and
        // one derived too long makes `MAX_ARROWS` a leak.
        assert!(
            baked.life_ticks > 0 && baked.life_ticks <= MAX_ARROW_LIFE_TICKS,
            "`{}` lives {} ticks",
            w.id,
            baked.life_ticks
        );
        let reach_mm = baked.life_ticks as u32 * baked.speed_mmpt as u32;
        assert!(
            reach_mm >= w.range_m * 1000 - baked.speed_mmpt as u32,
            "`{}` expires {} mm short of its declared {} m",
            w.id,
            w.range_m * 1000 - reach_mm,
            w.range_m
        );

        // And the sampler wall, checked on the shipped rows rather than
        // only on the refusal path below — a weapon that sits exactly on
        // the ceiling would be traced honestly by one sample per step and
        // nothing would say so.
        assert!(
            baked.speed_mmpt as usize <= ARROW_STEP_MM as usize * MAX_ARROW_SUBSTEPS,
            "`{}` outruns the collision sampler at {} mm/tick",
            w.id,
            baked.speed_mmpt
        );
    }
    assert_eq!(bows, 2, "the alpha data ships a bow and a crossbow");
}

/// A muzzle speed the collision sampler cannot trace is refused at boot,
/// not clamped at tick time. A clamped projectile is a weapon whose reach
/// is a lie the data never admits to, and it would first be noticed as an
/// arrow passing through a wall.
#[test]
fn a_bow_that_outruns_the_collision_sampler_is_refused() {
    refuses_bake(
        "weapons.toml",
        "speed_mps = 40",
        "speed_mps = 400",
        "collision sampler",
    );
}

// ---------------------------------------------------------------------------
// The spawn kit
//
// It is testing scaffolding that ships in content, which makes it exactly the
// kind of thing that rots quietly: it is not on any player's critical path,
// so a kit that silently granted nothing, or granted an item the tables do
// not have, would be found by somebody wondering why their hands were empty.

/// The alpha kit is real, reaches the sim, and lands where it says it does.
#[test]
fn the_spawn_kit_bakes_and_seats() {
    let c = build(&sources()).expect("content builds");
    let kit = c.bake_spawn_kit().expect("the kit bakes");
    assert!(kit.count > 0, "the alpha kit grants nothing");

    // The order is load-bearing: `grant_kit` writes slots in order, so the
    // first HOTBAR_SLOTS entries are the belt. A kit whose plan and hammer
    // were not on the belt would need a trip to the inventory before the
    // player could build at all, which defeats the reason it exists.
    let plan = c
        .item_index("item.building_plan")
        .expect("the plan is an item");
    let hammer = c.item_index("item.hammer").expect("the hammer is an item");
    let belt: Vec<u16> = kit.stacks[..(kit.count as usize).min(sim_core::limits::HOTBAR_SLOTS)]
        .iter()
        .map(|s| s.item)
        .collect();
    assert!(belt.contains(&plan), "the building plan is not on the belt");
    assert!(belt.contains(&hammer), "the hammer is not on the belt");

    // And it pays for something. A kit with tools and no materials cannot
    // place a piece, which is the verb it exists to unblock.
    let wood = c.item_index("item.wood").expect("wood is an item");
    let carried: u32 = kit.stacks[..kit.count as usize]
        .iter()
        .filter(|s| s.item == wood)
        .map(|s| s.count as u32)
        .sum();
    assert!(carried > 0, "the kit carries no wood to build with");
}

/// Absent is legal, and it means naked.
///
/// The default matters more than the alpha kit does: a public shard wants a
/// beach spawn, and `#[serde(default)]` is what lets that be expressed by
/// deleting a table rather than by editing code.
#[test]
fn no_spawn_kit_is_a_naked_spawn() {
    let mut srcs = sources();
    let entry = srcs.iter_mut().find(|(n, _)| *n == "balance.toml").unwrap();
    let cut = entry
        .1
        .find("[[spawn_kit]]")
        .expect("the alpha kit is there");
    entry.1.truncate(cut);
    let c = build(&srcs).expect("content without a spawn kit still validates");
    let kit = c.bake_spawn_kit().expect("an absent kit bakes");
    assert_eq!(kit.count, 0, "an absent kit granted something");
}

#[test]
fn spawn_kit_refusals() {
    // An item the tables do not have.
    refuses(
        "balance.toml",
        "[[spawn_kit]]\nitem = \"item.hammer\"",
        "[[spawn_kit]]\nitem = \"item.jetpack\"",
        "no such item",
    );
    // A count of zero — a slot that would draw empty.
    refuses(
        "balance.toml",
        "item = \"item.hammer\"\ncount = 1",
        "item = \"item.hammer\"\ncount = 0",
        "grants 0",
    );
    // Past the item's own stack size.
    refuses(
        "balance.toml",
        "item = \"item.hammer\"\ncount = 1",
        "item = \"item.hammer\"\ncount = 99",
        "past its own stack size",
    );
    // The same item twice — `grant_kit` writes slots and never merges, so
    // this is a typo that halves what the author meant.
    refuses(
        "balance.toml",
        "[[spawn_kit]]\nitem = \"item.hammer\"\ncount = 1",
        "[[spawn_kit]]\nitem = \"item.hammer\"\ncount = 1\n\n[[spawn_kit]]\nitem = \"item.hammer\"\ncount = 1",
        "granted twice",
    );
}
