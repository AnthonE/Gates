//! `test_content` (CLAUDE.md wall 7, CONTENT.md §0): the repo's shipped
//! `content/` loads, validates, holds every balance band, and hashes
//! stably — and the validator provably refuses the bug classes it claims
//! to: orphan refs, duplicate ids, unknown fields (the never-table at the
//! schema layer), and band breaks.

use content::schema::WeaponKind;
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

/// The two recovery globals reach the sim, which is the disease this row
/// was written to avoid — a column nothing bakes is a number that looks
/// tuned and does nothing.
///
/// It used to cite `headshot_mult` as the live instance of that. **It is
/// not one any more** (headshot v0): `bake_ranged` carries it and
/// `the_headshot_column_reaches_the_sim` below is its own version of this
/// check. The example moved, the rule did not.
///
/// It pins the *wiring* and not the values — asserting 15 and 10 here
/// would make a balance pass red for no reason, and `CONTENT.md` §4's
/// bands are what decide whether a value may land.
#[test]
fn the_arrow_recovery_globals_reach_the_sim() {
    let c = Content::load_dir(&content_dir()).expect("shipped content must load");
    let cc = c.bake_combat().expect("shipped content must bake");
    assert_eq!(
        u32::from(cc.arrow_break_pct),
        c.balance.globals.arrow_break_pct,
        "the break chance the sim rolls is not the one the file declares"
    );
    assert_eq!(
        cc.arrow_lodge_ticks,
        c.balance.globals.arrow_lodge_s * sim_core::limits::TICK_HZ,
        "the lodge the sim waits out is not the file's seconds at TICK_HZ"
    );
}

/// **The headshot column reaches the sim**, which it did not for the whole
/// life of this crate: `headshot_mult` was parsed, pinned to exactly the
/// band and folded into the content hash while `bake_ranged` dropped it one
/// line before `RangedDef` could hold it (`reference/PROJECTILES.md` §9.4).
/// A number that is validated, banded and hashed *looks* enforced from
/// every direction except the only one that matters.
///
/// It pins the **wiring** and not the value — asserting `2` here would turn
/// a balance pass red for no reason, and `CONTENT.md` §4's bands plus
/// `balance.rs`'s exact-equality check are what decide whether a value may
/// land. What it asserts is that every ranged row the file declares arrives
/// in the sim's table carrying the file's own number, whatever that is.
///
/// The bow, the crossbow and the revolver — every `WeaponKind` that bakes
/// through `bake_ranged`. Melee is deliberately absent: `MeleeDef` has no
/// such field and `sim-core`'s `tests/headshot.rs` is where that decision
/// is written down and gated.
///
/// Mutant watched red: `headshot_mult: 1` hard-coded at the bake.
#[test]
fn the_headshot_column_reaches_the_sim() {
    let c = Content::load_dir(&content_dir()).expect("shipped content must load");
    let cc = c.bake_combat().expect("shipped content must bake");
    let mut checked = 0;
    for w in &c.weapons {
        let idx = c.item_index(&w.id).expect("own id resolves") as usize;
        if cc.ranged[idx].damage == 0 {
            continue; // melee or a throwable: no ranged row to carry it
        }
        assert_eq!(
            u32::from(cc.ranged[idx].headshot_mult),
            w.headshot_mult,
            "`{}` declares headshot_mult {} and the sim's table says {}",
            w.id,
            w.headshot_mult,
            cc.ranged[idx].headshot_mult
        );
        assert_eq!(
            u32::from(cc.ranged[idx].limb_pct),
            w.limb_pct,
            "`{}` declares limb_pct {} and the sim's table says {}",
            w.id,
            w.limb_pct,
            cc.ranged[idx].limb_pct
        );
        checked += 1;
    }
    assert!(
        checked >= 3,
        "the file ships a bow, a crossbow and a revolver: only {checked} \
         ranged rows reached the table, so this passed by finding nothing"
    );
}

/// Both ends of the ladder are pinned to a band, and the band is what
/// stops a data edit from moving the shape of a fight.
///
/// **The TTK band says nothing about either end**, which is why this
/// exists as a second check rather than a comment: `hits_to_kill` is
/// measured on *body* hits, so `[bands] ttk_firearm` is green whether a
/// leg is worth 100% or 1% of the column. `balance.rs` refuses a row that
/// disagrees with `[bands] headshot_mult` / `limb_pct`, and this asserts
/// the two the shipped file actually carries — so the values are here in
/// one place rather than spread over eleven rows nobody diffs.
///
/// The throwable is skipped by `balance.rs` (a blast has no anatomy) and
/// carries the identities at both ends; that is asserted too, because
/// "skipped by the band" and "unset" look identical in a TOML file.
///
/// **This check cannot see a weakened predicate**, and that is measured
/// rather than assumed: `balance.rs`'s `!=` mutated to `<` survived it and
/// every other gate in the workspace. Asserting that the data agrees with
/// the band says nothing about what happens when it disagrees, so
/// `the_body_part_ladder_refuses_what_it_names` is the other half and the
/// two are only useful together.
///
/// Mutant watched red: the shipped `[bands] limb_pct` moved off 50.
#[test]
fn the_body_part_ladder_is_the_band_on_every_row() {
    let c = Content::load_dir(&content_dir()).expect("shipped content must load");
    let b = &c.balance.bands;
    assert_eq!(b.headshot_mult, 2, "the head is the reference's x2");
    assert_eq!(b.limb_pct, 50, "and the leg its x0.5, as a percent");
    let mut banded = 0;
    let mut opted_out = 0;
    for w in &c.weapons {
        if matches!(w.kind, WeaponKind::Throwable) {
            assert_eq!(
                (w.headshot_mult, w.limb_pct),
                (1, 100),
                "`{}` is a throwable and must carry both identities",
                w.id
            );
            opted_out += 1;
            continue;
        }
        assert_eq!(
            (w.headshot_mult, w.limb_pct),
            (b.headshot_mult, b.limb_pct),
            "`{}` is off the band",
            w.id
        );
        banded += 1;
    }
    assert!(banded >= 10, "only {banded} banded rows: the file shrank");
    assert_eq!(opted_out, 1, "exactly one throwable ships");
}

/// The ladder's refusals, driven from the file rather than from the
/// predicate — because the check above **cannot see a weakened one.**
///
/// Measured, and it is the reason this test exists: `balance.rs`'s
/// `w.limb_pct != bands.limb_pct` mutated to `<` was run and stayed
/// **green** through every gate in the workspace. Every shipped row
/// carries exactly the band, so a comparison that only fires *below* it
/// never fires at all — a row-by-row assertion proves the data agrees with
/// the band and says nothing about whether disagreeing would be caught.
/// The only way to ask is to hand the loader a row that disagrees.
///
/// Both directions, because they fail for different reasons: 100 is a leg
/// worth a chest (`Part`'s ordering inverted in data) and 25 is a leg the
/// band never priced.
///
/// Mutants watched red: `!=` → `<` and `!=` → `>` in `balance.rs`, and
/// each of `validate.rs`'s two bounds dropped.
#[test]
fn the_body_part_ladder_refuses_what_it_names() {
    // The rock is the first weapon row in the file and the only one whose
    // `damage = 20` line is unique, so it is where every bait below goes.
    const ROCK: &str = "id = \"item.rock\"\nkind = \"melee\"\ndamage = 20\nstructure = 1\nheadshot_mult = 2\nlimb_pct = 50";
    let bait = |limb: &str| ROCK.replace("limb_pct = 50", &format!("limb_pct = {limb}"));

    // Above the band: a leg worth as much as the chest above it.
    refuses("weapons.toml", ROCK, &bait("100"), "band break: limb pct");
    // Below it: a leg the band never priced.
    refuses("weapons.toml", ROCK, &bait("25"), "band break: limb pct");
    // Past the ordering entirely — `validate` refuses this before
    // `balance` ever sees it, which is the bound that stops a leg from
    // being worth more than the chest `part_crossed` would have scored.
    refuses("weapons.toml", ROCK, &bait("101"), "outside 1..=100");
    // And a leg hit that costs the body nothing.
    refuses("weapons.toml", ROCK, &bait("0"), "outside 1..=100");
    // The head's twin, which had no refusal test of its own either.
    refuses(
        "weapons.toml",
        ROCK,
        &ROCK.replace("headshot_mult = 2", "headshot_mult = 3"),
        "band break: headshot mult",
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
    assert!(a.wall_breach_swings[0] >= 60);
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

    // The declared farm rate. `canon.rs` walks it, but no probe proved the
    // walk until the farm-rate agreement check landed on top of it — and a
    // rate the anchors price everything with, canonicalising identically
    // across a disagreement, is the exact defect class this test exists
    // for.
    let mut srcs = sources();
    let b = srcs.iter_mut().find(|(n, _)| *n == "balance.toml").unwrap();
    b.1 = b.1.replace("\"item.wood\" = 50", "\"item.wood\" = 49");
    assert_ne!(
        base,
        build(&srcs).unwrap().hash(),
        "the declared farm rate must move the content hash"
    );

    // The night notice radius — now the newest field on this path, and the
    // one with the sharpest reason to be here: it is the only content
    // number the *hour* selects, so two sets differing only in it produce
    // shards that hunt differently for a third of every cycle while
    // agreeing about every other byte. Exactly the "plays differently,
    // canonicalises the same" defect above, with a clock in front of it.
    let mut srcs = sources();
    let m = srcs.iter_mut().find(|(n, _)| *n == "mobs.toml").unwrap();
    m.1 = m.1.replace("night_spook_m = 15", "night_spook_m = 16");
    assert_ne!(
        base,
        build(&srcs).unwrap().hash(),
        "the night notice radius must move the content hash"
    );

    // The tree edge (tech tree v0): `requires` reaches the sim through
    // `bake_research`, and two contents that disagree about a parent
    // charge different path totals for the same node — the same "plays
    // differently, canonicalises the same" defect, priced in coin.
    let mut srcs = sources();
    let r = srcs
        .iter_mut()
        .find(|(n, _)| *n == "research.toml")
        .unwrap();
    // Anchored on the revolver's whole block: two rows require gunpowder,
    // and a bare `replace` moved both — which strips the satchel's
    // craft-graph floor edge and fails validation for an unrelated reason.
    r.1 = r.1.replace(
        "item = \"item.revolver\"\ncost = 75\nrequires = \"item.gunpowder\"",
        "item = \"item.revolver\"\ncost = 75\nrequires = \"item.medkit\"",
    );
    assert_ne!(
        base,
        build(&srcs).unwrap().hash(),
        "the tree edge must move the content hash"
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
         coin = \"ELO\"\nprice = 10\nseason = \"alpha\"\n",
    );
    build(&srcs).expect("a plain appearance row must parse");
    let skins = srcs.iter_mut().find(|(n, _)| *n == "skins.toml").unwrap();
    skins.1.push_str("damage_bonus = 1\n");
    let err = build(&srcs).expect_err("a stat field on a skin was accepted");
    assert!(err.contains("unknown field"), "got: {err}");
}

#[test]
fn dollar_ticker_refused() {
    // Tickers are bare (CLAUDE.md wall 8): `$ELO` is not a coin.
    let mut srcs = sources();
    let skins = srcs.iter_mut().find(|(n, _)| *n == "skins.toml").unwrap();
    skins.1 = String::from(
        "[[skin]]\nid = \"skin.wood_gilt\"\ncovers = \"item.hatchet_stone\"\n\
         coin = \"$ELO\"\nprice = 10\nseason = \"alpha\"\n",
    );
    assert!(build(&srcs).is_err(), "$-prefixed ticker was accepted");
}

#[test]
fn orphan_refs_refused() {
    refuses(
        "recipes.toml",
        "inputs = [{ item = \"item.stone\", count = 10 }]",
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
    // TTK: a 5-damage melee row is 20 hits to kill — far out of melee 3–5.
    // Keyed off the stone hatchet's 25, the first `melee` row in the file.
    refuses(
        "weapons.toml",
        "kind = \"melee\"\ndamage = 25",
        "kind = \"melee\"\ndamage = 5",
        "band break: ttk",
    );
    // Raid ratio: the satchel's own column, and the fixture is the value
    // this file shipped BEFORE the reference alignment (2026-08-08). At
    // 500 a satchel opens a 500-hp stone wall in one throw, so raiding
    // costs a third of what building does — 0.35× against a [1.0, 3.0]
    // band. The ratio divides by `structure`, not `damage`.
    refuses(
        "weapons.toml",
        "structure = 125",
        "structure = 500",
        "band break: stone raid ratio",
    );
    // A weapon better against a wall than a person is refused outright,
    // no band consulted: what kills in 25 cannot chip 99.
    refuses(
        "weapons.toml",
        "damage = 25\nstructure = 2",
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
        "\"item.hatchet_metal\" = 87",
        "\"item.hatchet_metal\" = 3",
        "band break: node yield",
    );
    // The declared effective rate may never beat standing at the node:
    // wood's at-node ceiling is 5887/min (870 over the 7 marked swings
    // that exhaust a 10-hit node, at the 38-tick cadence), and a
    // declaration above it prices walking as a bonus. The mutation has to
    // clear the ceiling to be refused, so it moved with the yields —
    // 3000 was over the old 2030 and sits comfortably under this one.
    refuses(
        "balance.toml",
        "\"item.wood\" = 50",
        "\"item.wood\" = 9000",
        "farm rate break",
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
    // A break chance is a percentage, and BOTH ends of it are legal — 0 is
    // an arrow that lasts forever and 100 is the game that existed before
    // arrow recovery v0. So the only refusable value is one that is not a
    // percentage at all, and 101 is the smallest of those.
    refuses(
        "balance.toml",
        "arrow_break_pct = 15",
        "arrow_break_pct = 101",
        "arrow_break_pct 101 is not a percentage",
    );
}

/// The declared farm rate is now compared against the sim's own arithmetic
/// — the latent defect `reference/BALANCE.md` §4.3 named ("nothing checks
/// that it agrees with `yield_per_hit`") is closed as a ceiling: an
/// effective travel-included rate can never beat standing at the node.
/// The values here are the shipped set's derivation, pinned so the
/// at-node side is hand-checkable. Since the mark buys speed and not
/// yield (2026-08-09), a node's payout is invariant at `hits × per-hit`
/// and the ceiling is that total over the FEWEST swings that exhaust it
/// — every swing marked, `ceil(hits × 100 / (100 + weak))`. A yield,
/// mark or cadence move re-pins these numbers in the same commit — that
/// is fixture discipline, not rot.
#[test]
fn the_declared_farm_rate_cannot_beat_standing_at_the_node() {
    let c = Content::load_dir(&content_dir()).expect("shipped content must load");
    let rates = &c.anchors().farm_rates;
    // (item, declared, at-node). Every node empties in the same
    // ceil(1000/150) = 7 marked swings, so the ceiling is just the node's
    // total × 1800 / (7 × 38 = 266): wood 870 → 5887, stone 1000 → 6766,
    // metal 600 → 4060, sulfur 300 → 2030. Cloth is the bush's hand 10
    // over one unmarked hit → 10 × 1800 / 38 = 473.
    //
    // **The ceilings tripled on 2026-08-10 and the declared rates did
    // not**, so the gap widened from ~24–68× to ~24–135× (cloth 23.7×,
    // sulfur 67.7×, wood 117.7×, stone and metal 135.3×). That is not a
    // regression being pinned quietly: `farm_per_min` is a number the
    // reference has no equivalent for (`reference/RIPLIST.md` §3), its
    // semantics are the open operator knob, and the node take deliberately
    // left it alone rather than tune one unmeasured number against
    // another. The widening is the visible cost of that choice and it
    // belongs in a fixture where the next pass cannot miss it.
    for (item, declared, at_node) in [
        ("item.wood", 50, 5887),
        ("item.stone", 50, 6766),
        ("item.metal_ore", 30, 4060),
        ("item.sulfur_ore", 30, 2030),
        ("item.cloth", 20, 473),
    ] {
        let row = rates
            .iter()
            .find(|(id, _, _)| id == item)
            .unwrap_or_else(|| panic!("no farm-rate anchor row for `{item}`"));
        assert_eq!(
            (row.1, row.2),
            (declared, at_node),
            "`{item}`: declared/at-node moved — re-pin alongside the data \
             (and re-read the DECISIONS.md §open farm rate row)"
        );
    }
    assert_eq!(
        rates.len(),
        5,
        "a new farm_per_min row landed without extending this pin"
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
        "[[piece]]\nid = \"build.roof_wood\"\nshape = \"roof\"\nmaterial = \"wood\"\nhp = 250\ncost = [{ item = \"item.wood\", count = 100 }]\n",
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

    // gatherables.toml gather.tree: output wood, 10 hits, **no hand row**,
    // rock 50, stone hatchet 81, weak-spot +50% — read back from the baked
    // table. The tool yields are the reference's large-tree totals over our
    // own 10 hits (810 ÷ 10 at the stone hatchet); they moved 2026-08-10
    // with the rest of the node take.
    //
    // **The hand yield was 25 and is now 0** (DECISIONS.md 2026-08-15:
    // bare hands gather nothing). It is pinned by VALUE rather than left
    // unasserted because the zero is the whole decision and because it is
    // expressed by an ABSENT row: `bake.rs` initialises `hand_yield: 0`,
    // so a `hand` line silently re-added to gatherables.toml reddens here
    // and nowhere else. `yield_for` falls back to the hand yield for any
    // item not in the tool table, so this also pins what a torch or a
    // hammer draws off a tree, which `gather::swing` now refuses on.
    let tree = &gc.nodes[0];
    let rock = c.item_index("item.rock").unwrap();
    assert_eq!(tree.output, wood);
    assert_eq!(tree.hits, 10);
    assert_eq!(
        tree.yield_for(sim_core::gather::NO_ITEM),
        0,
        "bare hands pay something on a tree — the `hand` row is back"
    );
    assert_eq!(tree.yield_for(rock), 50, "the bootstrap tool pays nothing");
    assert_eq!(tree.yield_for(hatchet), 81);
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
    assert!(def.inputs[..2].contains(&(wood, 200)));
    assert!(def.inputs[..2].contains(&(stone, 100)));

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
        "inputs = [{ item = \"item.stone\", count = 10 }]",
        "inputs = [\n    { item = \"item.stone\", count = 10 },\n    { item = \"item.wood\", count = 1 },\n    { item = \"item.cloth\", count = 1 },\n    { item = \"item.fat\", count = 1 },\n    { item = \"item.charcoal\", count = 1 },\n]",
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

    // building.toml build.wall_stone: shape wall, stone, hp 500,
    // 300 stone — read back from the baked row.
    let idx = c.piece_index("build.wall_stone").unwrap() as usize;
    let def = &bc.pieces[idx];
    assert_eq!(def.shape, sim_core::build::SHAPE_WALL);
    assert_eq!(def.material, sim_core::build::MAT_STONE);
    assert_eq!(def.hp, 500);
    assert_eq!(def.n_costs, 1);
    let stone = c.item_index("item.stone").unwrap();
    assert_eq!(def.costs[0], (stone, 300));

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
    entry.1 = entry.1.replacen("hp = 1000\n", "hp = 70000\n", 1);
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
        "cost = [{ item = \"item.wood\", count = 50 }]",
        "cost = [\n    { item = \"item.wood\", count = 50 },\n    { item = \"item.stone\", count = 1 },\n    { item = \"item.cloth\", count = 1 },\n]",
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
        "cost = [{ item = \"item.wood\", count = 50 }]",
        "cost = [\n    { item = \"item.wood\", count = 50 },\n    { item = \"item.wood\", count = 1 },\n]",
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
    // placement, hp 1000 — read back from the baked row.
    let idx = c.deploy_index("item.hearth").unwrap() as usize;
    let def = &dc.defs[idx];
    assert_eq!(def.arch, sim_core::deploy::ARCH_HEARTH);
    assert_eq!(def.placement, sim_core::deploy::PLACE_FOUNDATION);
    assert_eq!(def.hp, 1000);
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
        cond: 0,
    };
    assert_eq!(bc.lifetime_ticks(&inv), bc.base_ticks);
    inv[1] = sim_core::gather::ItemStack {
        item: c.item_index(&rarest.id).unwrap(),
        count: 1,
        cond: 0,
    };
    assert_eq!(
        bc.lifetime_ticks(&inv),
        bc.base_ticks * mults[rarest.rarity.canon() as usize],
        "one rare thing raises the whole bag"
    );
}

/// The seven durability rules (item durability v0), each proven against
/// the shipped set with one edit. V7 and V4 are the two the slice's gates
/// name — the stack law everything else leans on, and the set check this
/// repo keeps getting bitten by — and the other five are the dead-row and
/// width refusals between them.
#[test]
fn the_durability_rules_refuse_what_they_name() {
    // V7: a condition-carrying item must stack to 1. The rock keeps its
    // 10 000 hundredths and grows a stack of 3 — refused, because
    // condition is per-stack state and `plan_move`/`inv_add` keep their
    // arithmetic only while no merge can ever meet two conditions.
    refuses(
        "items.toml",
        "name = \"Rock\"\nstack = 1",
        "name = \"Rock\"\nstack = 3",
        "(V7)",
    );
    // V4, the set check: a condition-carrying tool that pays on a node
    // must have a loss row there. Dropping the stone hatchet's tree row
    // leaves a tool that farms the forest free forever.
    refuses(
        "gatherables.toml",
        "[gatherable.condition_loss]\n\"item.rock\" = 30\n\"item.hatchet_stone\" = 30\n\"item.hatchet_metal\" = 30",
        "[gatherable.condition_loss]\n\"item.rock\" = 30\n\"item.hatchet_metal\" = 30",
        "(V4)",
    );
    // V1: the ceiling must fit the sim's u16 hundredths.
    refuses(
        "items.toml",
        "condition_max = 40000\n\n[[item]]\nid = \"item.pickaxe_metal\"",
        "condition_max = 90000\n\n[[item]]\nid = \"item.pickaxe_metal\"",
        "(V1)",
    );
    // V2: bare hands do not wear — a `hand` loss row is refused. The
    // bush is the one node with a hand row, so it is where the bait goes.
    refuses(
        "gatherables.toml",
        "[gatherable.yield_per_hit]\nhand = 10",
        "[gatherable.yield_per_hit]\nhand = 10\n\n[gatherable.condition_loss]\nhand = 30",
        "(V2)",
    );
    // V3: a loss row must name a tool with a yield row on the same node —
    // wear lands only on a paying hit, so a row for a non-paying tool is
    // unreachable coverage. The torch carries condition and pays nowhere.
    refuses(
        "gatherables.toml",
        "[gatherable.condition_loss]\n\"item.rock\" = 30\n\"item.hatchet_stone\" = 30\n\"item.hatchet_metal\" = 30",
        "[gatherable.condition_loss]\n\"item.rock\" = 30\n\"item.hatchet_stone\" = 30\n\"item.hatchet_metal\" = 30\n\"item.torch\" = 30",
        "(V3)",
    );
    // V5: a zero loss is an inert row, not a statement.
    refuses(
        "gatherables.toml",
        "[gatherable.condition_loss]\n\"item.rock\" = 30\n\"item.pickaxe_stone\" = 30\n\"item.pickaxe_metal\" = 30\n\n# Bush",
        "[gatherable.condition_loss]\n\"item.rock\" = 30\n\"item.pickaxe_stone\" = 0\n\"item.pickaxe_metal\" = 30\n\n# Bush",
        "(V5)",
    );
    // V6: a loss row's tool must declare a condition to lose. Strip the
    // stone hatchet's ceiling while its tree loss row still names it.
    refuses(
        "items.toml",
        "# 100 pts / 0.3 per hit (wiki-confirmed) = 333 hits, ~33 trees.\ncondition_max = 10000",
        "# 100 pts / 0.3 per hit (wiki-confirmed) = 333 hits, ~33 trees.\ncondition_max = 0",
        "(V6)",
    );
}

/// The two torch-fuel rules (torch fuel v0), each proven against the
/// shipped set with one edit.
///
/// Both exist because `light_burn` is a **predicate as well as a price** —
/// nonzero is what makes an item a light at all (`sim_core::light::is_lit`
/// fact 2) — so the two ways to write a nonsense light are a light that
/// cannot pay and a light that pays impossibly fast.
#[test]
fn the_torch_fuel_rules_refuse_what_they_name() {
    // V8: a light must have condition to spend. Strip the torch's ceiling
    // and leave its burn rate, and the shard ships a flame that burns
    // forever for nothing — the free light the field exists to forbid.
    refuses(
        "items.toml",
        "condition_max = 5000
# Hundredths of condition per minute",
        "condition_max = 0
# Hundredths of condition per minute",
        "(V8)",
    );
    // V9: the rate fits `u16`, which is what bounds the sim's per-tick
    // debit to one point without a clamp anywhere (wall 4).
    refuses(
        "items.toml",
        "light_burn = 1000",
        "light_burn = 200000",
        "(V9)",
    );
}

/// The shipped torch is **five minutes**, asserted off the two shipped
/// numbers rather than off either one.
///
/// `condition_max` and `light_burn` are each individually plausible at any
/// value and neither states the duration, which is the thing taken from
/// the reference (`reference/BALANCE.md` §6: 1/6 of a point a second off a
/// max of 50). A balance pass that moved one and not the other would leave
/// both bands green and quietly hand the player a 30-second torch.
#[test]
fn the_shipped_torch_is_five_minutes_of_light() {
    let c = Content::load_dir(&content_dir()).expect("shipped content must load");
    let torch = c
        .items
        .iter()
        .find(|i| i.id == "item.torch")
        .expect("content/items.toml no longer ships a torch");
    assert!(torch.light_burn > 0, "the torch stopped being a light");
    // Hundredths / (hundredths per minute) = minutes, exactly — and the
    // exactness is the point: the reference's rate divides its ceiling
    // whole, so a remainder here would mean one of the two moved off it.
    assert_eq!(
        torch.condition_max % torch.light_burn,
        0,
        "the torch's ceiling is no longer a whole number of minutes at \
         its own burn rate"
    );
    assert_eq!(
        torch.condition_max / torch.light_burn,
        5,
        "the shipped torch is no longer the reference's five minutes"
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
    // the source is worth walking to. One harvest must buy back a
    // meaningful share of a span, or the loop is a treadmill that happens
    // to pass a boolean.
    //
    // **A harvest is no longer only a bush.** This scanned nodes alone
    // until 2026-08-08, when the reference alignment moved the meters to
    // 500/250 and forage to the reference's own calorie-poor values — at
    // which point a bush bought 6 minutes and this assertion failed,
    // correctly, on a rule that had gone stale rather than on a bad number.
    // The reference's food economy is **meat-centric**: forage hydrates,
    // meat feeds, and a player who eats only berries starves. So the source
    // set is nodes AND what an animal drops, and the bar is unchanged —
    // *something* in the world has to be worth walking to, and now the
    // thing that is, is the pig.
    let gc = c.bake_gather().expect("shipped gather table must bake");
    let mc = c.bake_mobs().expect("shipped animals must bake");
    let mut best_food_min = 0u32;
    let mut best_water_min = 0u32;
    // What a kill pays, run through the fire: raw meat is not edible, so
    // the food a mob is worth is its cooked output's row.
    let cooks = c.bake_cooking().expect("shipped cooking must bake");
    for def in mc.defs.iter() {
        for drop in def.loot.iter() {
            if drop.item == sim_core::gather::NO_ITEM || drop.count == 0 {
                continue;
            }
            let eaten = cooks
                .row_for(sim_core::deploy::ARCH_FIRE, drop.item)
                .map(|r| r.output)
                .unwrap_or(drop.item);
            let row = sc.consumable[eaten as usize];
            best_food_min = best_food_min
                .max((row.food as u32 * drop.count as u32 * s.food_minutes_to_empty) / s.max_food);
            best_water_min = best_water_min.max(
                (row.water as u32 * drop.count as u32 * s.water_minutes_to_empty) / s.max_water,
            );
        }
    }
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
        "the best food in the world buys {best_food_min} min of a \
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
        "max_water = 250",
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
    // The honest version of the same defect — the rows simply absent. All
    // three of them since world containers v0 widened the clock's
    // reachable set (2026-08-17): the tree's mushrooms answer hunger on
    // their own, and so does the barrel's corn now that a verb opens every
    // shipped container — which is the redundancy working, not the check
    // weakening.
    let mut srcs = sources();
    let g = srcs
        .iter_mut()
        .find(|(n, _)| *n == "gatherables.toml")
        .unwrap();
    for row in [
        "\n[gatherable.secondary]\noutput = \"item.mushrooms\"\nper_hit = 1\n",
        "\n[gatherable.secondary]\noutput = \"item.berries\"\nper_hit = 5\n",
    ] {
        assert!(
            g.1.contains(row),
            "test fixture rot: `{row}` not in gatherables.toml"
        );
        g.1 = g.1.replace(row, "\n");
    }
    // Positive control for the widening itself: with both secondaries gone
    // the barrel's corn is the island's only food, and it counts — before
    // 2026-08-17 this exact set was refused as unanswerable, which had
    // become a false refusal the day the open verb landed.
    build(&srcs).expect("the barrel's corn answers hunger since world containers v0");
    let l = srcs.iter_mut().find(|(n, _)| *n == "loot.toml").unwrap();
    let corn_row = "    { item = \"item.corn\", weight = 8, count_min = 2, count_max = 4 },\n";
    assert!(
        l.1.contains(corn_row),
        "test fixture rot: the barrel's corn row moved"
    );
    l.1 = l.1.replace(corn_row, "");
    let err = build(&srcs).expect_err("a foodless island must be refused");
    assert!(
        err.contains("the clock has no answer"),
        "expected the unanswerable-clock refusal, got: {err}"
    );
    // And the thirst half. It takes **both** answers off the island now:
    // the drink verb (wire v15) is the second way to answer thirst, so
    // berries that feed but do not water are no longer enough on their own
    // to leave the shorter fuse unanswerable — which is the widening, and
    // the case below is what pins it.
    let mut srcs = sources();
    for (name, text) in srcs.iter_mut() {
        if *name == "consumables.toml" {
            // **Every node-reachable water source, not just the bush.** This
            // zeroed berries alone until 2026-08-08, when the tree grew a
            // mushroom secondary in one lane while the other moved forage to
            // the reference's hydrate-don't-feed split — so after the merge
            // the "dry island" still had a drink in it and the refusal this
            // asserts never fired. A fixture that names one row is a fixture
            // that goes stale the moment the world grows a second.
            *text = text.replace(
                "id = \"item.berries\"\nhealth = 0\nfood = 10\nwater = 20",
                "id = \"item.berries\"\nhealth = 0\nfood = 10\nwater = 0",
            );
            *text = text.replace(
                "id = \"item.mushrooms\"\nhealth = 3\nfood = 15\nwater = 5",
                "id = \"item.mushrooms\"\nhealth = 3\nfood = 15\nwater = 0",
            );
            // The barrel's corn counts toward the clock since world
            // containers v0 widened the reachable set, so the dry island
            // has to dry it out too.
            *text = text.replace(
                "id = \"item.corn\"\nhealth = 0\nfood = 40\nwater = 20",
                "id = \"item.corn\"\nhealth = 0\nfood = 40\nwater = 0",
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
    assert_eq!(t.len, 9, "the barrel table lost or gained a row");
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

/// The decay ladder must not invert.
///
/// The four numbers look like taste and one relationship in them is
/// not: if metal rots faster than wood, an upgrade spends materials to
/// *shorten* a base's life, and nothing downstream would ever say so —
/// the sweep would just quietly eat the expensive walls first. Twig
/// leads the ladder and is the same rule at the bottom: a scaffold that
/// outlasted a wooden wall would be a base, not a draft.
#[test]
fn an_inverted_decay_ladder_is_refused() {
    refuses(
        "balance.toml",
        "decay_pct_per_period = { twig = 100, wood = 34, stone = 20, metal = 13 }",
        "decay_pct_per_period = { twig = 100, wood = 13, stone = 20, metal = 34 }",
        "not monotone",
    );
    refuses(
        "balance.toml",
        "decay_pct_per_period = { twig = 100, wood = 34, stone = 20, metal = 13 }",
        "decay_pct_per_period = { twig = 20, wood = 34, stone = 20, metal = 13 }",
        "not monotone",
    );
    refuses(
        "balance.toml",
        "decay_pct_per_period = { twig = 100, wood = 34, stone = 20, metal = 13 }",
        "decay_pct_per_period = { twig = 100, wood = 34, stone = 20, metal = 0 }",
        "never rots",
    );
}

/// The ladder the sim plays is the ladder the file wrote, keyed by the
/// sim's own material codes — the conversion boundary, gated like the
/// bow's (`bows_bake_to_per_tick_integers_the_sim_can_integrate`).
#[test]
fn the_decay_ladder_reaches_the_sim_keyed_by_material() {
    let c = Content::load_dir(&content_dir()).expect("shipped content must load");
    let dc = c.bake_deployables().expect("shipped deployables must bake");
    let d = &c.balance.globals.decay_pct_per_period;
    for (m, code) in [
        (
            content::schema::Material::Wood,
            sim_core::build::MAT_WOOD as usize,
        ),
        (
            content::schema::Material::Stone,
            sim_core::build::MAT_STONE as usize,
        ),
        (
            content::schema::Material::Metal,
            sim_core::build::MAT_METAL as usize,
        ),
    ] {
        assert_eq!(
            dc.decay_pct[code] as u32, d[&m],
            "{m:?} reached the sim at the wrong index — the ladder is keyed \
             by material and a crossed pair would rot the wrong walls"
        );
    }
}

/// The code lock's row and its placement class are one thing said twice,
/// and the sim indexes on both (lock v1, `reference/DOORS.md` §9.1).
///
/// `place_deploy` picks the lock branch off the **archetype** and picks
/// "the address must hold a door" off the **placement**. A row carrying
/// one without the other is not a validation nicety: `lock` + `doorway`
/// is a lock that mints a deploy record standing in an empty doorway, and
/// `box` + `door` is a chest a player is invited to hang on a door.
/// Neither crashes anything, which is exactly why it needs a gate.
#[test]
fn a_lock_and_its_placement_class_are_refused_apart() {
    refuses(
        "deployables.toml",
        "archetype = \"lock\"\nplacement = \"door\"",
        "archetype = \"lock\"\nplacement = \"doorway\"",
        "a lock is placement `door`",
    );
    refuses(
        "deployables.toml",
        "archetype = \"box\"\nplacement = \"any\"",
        "archetype = \"box\"\nplacement = \"door\"",
        "placement `door` is the lock's alone",
    );
}

/// One lock row, or none.
///
/// `deploy::lock_row` resolves the item a taken-off lock hands back by
/// scanning the baked table for the archetype. With two rows that scan
/// picks the first and returns the wrong item — silently, only on the
/// take verb, and only for whoever bolted the second kind on.
#[test]
fn a_second_lock_row_is_refused() {
    refuses(
        "deployables.toml",
        "id = \"item.lock_code\"\narchetype = \"lock\"",
        "id = \"item.hammer\"\narchetype = \"lock\"\nplacement = \"door\"\nhp = 100\n\n[[deployable]]\nid = \"item.lock_code\"\narchetype = \"lock\"",
        "the sim can only name one",
    );
}

/// The lock reaches the sim as a real baked row the placement path can
/// find, and it is the only one wearing `PLACE_DOOR`.
#[test]
fn the_code_lock_bakes_to_the_archetype_the_sim_branches_on() {
    let c = Content::load_dir(&content_dir()).expect("shipped content must load");
    let dc = c.bake_deployables().expect("shipped deployables must bake");
    let rows: Vec<_> = dc.defs[..dc.def_count as usize]
        .iter()
        .filter(|d| d.arch == sim_core::deploy::ARCH_LOCK)
        .collect();
    assert_eq!(rows.len(), 1, "exactly one lock reaches the sim");
    assert_eq!(
        rows[0].placement,
        sim_core::deploy::PLACE_DOOR,
        "the lock's placement class is the one that wants an occupied address"
    );
    assert_eq!(
        rows[0].item,
        c.item_index("item.lock_code").expect("the lock is an item"),
        "the row must resolve to the item the take verb hands back"
    );
    for d in dc.defs[..dc.def_count as usize].iter() {
        assert!(
            d.arch == sim_core::deploy::ARCH_LOCK || d.placement != sim_core::deploy::PLACE_DOOR,
            "a non-lock wears the lock's placement class"
        );
    }
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
    use sim_core::gather::NO_ITEM;
    use sim_core::limits::{ARROW_STEP_MM, MAX_ARROW_LIFE_TICKS, MAX_ARROW_SUBSTEPS, TICK_HZ};

    let c = Content::load_dir(&content_dir()).expect("shipped content must load");
    let cc = c.bake_combat().expect("shipped weapons must bake");

    let mut bows = 0;
    for w in &c.weapons {
        let idx = c.item_index(&w.id).expect("weapon arms an item") as usize;
        let baked = cc.ranged[idx];
        if w.kind != content::schema::WeaponKind::Bow {
            // A firearm shares this table and is the one row in it that is
            // NOT a projectile: `hitscan` is what separates them, and it is
            // asserted both ways here so a bake that set the flag on
            // everything would fail rather than read as covered.
            if w.kind == content::schema::WeaponKind::Firearm {
                assert!(
                    baked.hitscan,
                    "`{}` is a firearm and must bake as hitscan",
                    w.id
                );
                continue;
            }
            assert_eq!(
                baked.damage, 0,
                "`{}` is neither a bow nor a firearm and must not be armed in the ranged table",
                w.id
            );
            continue;
        }
        bows += 1;
        assert!(
            !baked.hitscan,
            "`{}` is a bow — its arrow flies, so it must not bake as hitscan",
            w.id
        );
        assert_eq!(baked.damage as u32, w.damage, "`{}` damage", w.id);
        assert_eq!(
            baked.rate_ticks as u32,
            TICK_HZ * 60 / w.rate_per_min,
            "`{}` ticks between shots",
            w.id
        );
        assert_eq!(
            baked.range_mm,
            w.range_m * 1000,
            "`{}` reach in millimetres",
            w.id
        );

        // The round list bakes in **declared order** — order is the whole
        // ammo policy until a switch verb exists (`PROJECTILES.md` §9.3),
        // so a bake that sorted or deduped it would silently change which
        // arrow a bow reaches for first.
        let rounds = w.ammo.as_ref().expect("validate refuses a bow without one");
        for (slot, id) in rounds.iter().enumerate() {
            assert_eq!(
                baked.ammo[slot],
                c.item_index(id).expect("round is an item"),
                "`{}` round {slot} is the one it names",
                w.id
            );
        }
        for slot in rounds.len()..baked.ammo.len() {
            assert_eq!(
                baked.ammo[slot], NO_ITEM,
                "`{}` pads unused round slots rather than repeating one",
                w.id
            );
        }

        // Every round the bow lists must fly, and must cover the reach the
        // weapon claims. Flight time is no longer baked — it is
        // `range_mm / speed` at the moment of the shot, because with the
        // speed on the round one bow's fast arrow and its slow arrow cross
        // the same range in different numbers of ticks. This asserts the
        // sim's arithmetic against the data for each round in turn.
        for id in rounds {
            let a = c
                .ammo
                .iter()
                .find(|a| &a.id == id)
                .expect("validate refuses a round with no [[ammo]] row");
            let ball = cc
                .ammo_def(c.item_index(id).expect("round is an item"))
                .expect("a listed round is armed");
            assert_eq!(
                ball.speed_mmpt as u32,
                a.speed_mps * 1000 / TICK_HZ,
                "`{id}` muzzle speed in mm/tick"
            );
            assert_eq!(
                ball.drop_mmpt2 as u32,
                a.drop_mps2 * 1000 / (TICK_HZ * TICK_HZ),
                "`{id}` drop in mm/tick^2"
            );

            // The flight must actually cover the reach the data claims and
            // then stop: derived too short makes `range_m` a lie, too long
            // makes `MAX_ARROWS` a leak. Same expression `ranged::draw`
            // evaluates, asserted against `weapons.toml` rather than a
            // remembered constant.
            let life =
                (baked.range_mm / ball.speed_mmpt as u32).clamp(1, MAX_ARROW_LIFE_TICKS as u32);
            assert!(
                life > 0 && life <= MAX_ARROW_LIFE_TICKS as u32,
                "`{}` firing `{id}` lives {life} ticks",
                w.id
            );
            let reach_mm = life * ball.speed_mmpt as u32;
            assert!(
                reach_mm >= w.range_m * 1000 - ball.speed_mmpt as u32,
                "`{}` firing `{id}` expires {} mm short of its declared {} m",
                w.id,
                w.range_m * 1000 - reach_mm,
                w.range_m
            );

            // And the sampler wall, checked on the shipped rows rather than
            // only on the refusal path below — a round that sits exactly on
            // the ceiling would be traced honestly by one sample per step
            // and nothing would say so.
            assert!(
                ball.speed_mmpt as usize <= ARROW_STEP_MM as usize * MAX_ARROW_SUBSTEPS,
                "`{id}` outruns the collision sampler at {} mm/tick",
                ball.speed_mmpt
            );
        }
    }
    assert_eq!(bows, 2, "the alpha data ships a bow and a crossbow");
}

/// The revolver reaches the sim, armed, and kills inside the band
/// `balance.toml` declares for a firearm.
///
/// **This is the gate on a charged dead end rather than on a number.**
/// Until 2026-08-19 `bake_combat` dropped every row that was not melee,
/// throwable or bow, and the revolver was not inert data while it sat
/// there: it is a barrel drop at weight 1 (`the_shipped_loot_tables_bake`),
/// it has a recipe, it is on the research ladder behind gunpowder, and its
/// round both drops and crafts. So a player could spend scrap on the
/// research, materials on the gun and more on ammo, and pull the trigger on
/// nothing. What this asserts is the whole chain out of that: the row bakes,
/// it bakes as hitscan, its round is the one it names, its reach is
/// traceable at the sampler's spacing, and the hits it takes to kill is the
/// number the band already gates.
#[test]
fn the_firearm_reaches_the_sim_and_kills_in_the_band_it_declares() {
    use sim_core::gather::NO_ITEM;
    use sim_core::limits::{ARROW_STEP_MM, MAX_HITSCAN_SAMPLES, TICK_HZ};

    let c = Content::load_dir(&content_dir()).expect("shipped content must load");
    let cc = c.bake_combat().expect("shipped weapons must bake");
    let [lo, hi] = c.balance.bands.ttk_firearm;

    let mut guns = 0;
    for w in &c.weapons {
        if w.kind != content::schema::WeaponKind::Firearm {
            continue;
        }
        guns += 1;
        let idx = c.item_index(&w.id).expect("weapon arms an item") as usize;
        let baked = cc.ranged[idx];
        assert!(baked.hitscan, "`{}` must bake as hitscan", w.id);
        assert_eq!(baked.damage as u32, w.damage, "`{}` damage", w.id);
        assert_eq!(
            baked.rate_ticks as u32,
            TICK_HZ * 60 / w.rate_per_min,
            "`{}` ticks between shots",
            w.id
        );
        assert_eq!(
            baked.range_mm,
            w.range_m * 1000,
            "`{}` reach in millimetres",
            w.id
        );

        // The rounds, in declared order and `NO_ITEM`-padded — the bow's
        // rule, because it is the same field and the same policy.
        let rounds = w
            .ammo
            .as_ref()
            .expect("validate refuses a firearm without one");
        for (slot, id) in rounds.iter().enumerate() {
            assert_eq!(
                baked.ammo[slot],
                c.item_index(id).expect("round is an item"),
                "`{}` round {slot} is the one it names",
                w.id
            );
            // And it is a round with no ballistics, which is what makes it
            // hitscan rather than a projectile nobody baked a speed for.
            assert!(
                cc.ammo_def(c.item_index(id).expect("round is an item"))
                    .is_none(),
                "`{}` round `{id}` carries ballistics — it would be a projectile",
                w.id
            );
        }
        for slot in rounds.len()..baked.ammo.len() {
            assert_eq!(
                baked.ammo[slot], NO_ITEM,
                "`{}` pads unused round slots rather than repeating one",
                w.id
            );
        }

        // The hitscan sampler wall, on the shipped row rather than only on
        // the refusal path: a reach sitting exactly on the ceiling would be
        // traced by one sample per step and nothing would say so.
        let taps = baked.range_mm as usize / ARROW_STEP_MM as usize + 1;
        assert!(
            taps <= MAX_HITSCAN_SAMPLES,
            "`{}` needs {taps} collision samples for its {} m, past the {MAX_HITSCAN_SAMPLES} \
             one shot may take",
            w.id,
            w.range_m
        );

        // Hits to kill, computed the way the sim computes it — the melee
        // test's arithmetic against the firearm band.
        let mut hp = cc.player_hp;
        let mut hits = 0u32;
        while hp > 0 {
            hp -= baked.damage.min(hp);
            hits += 1;
        }
        assert!(
            (lo..=hi).contains(&hits),
            "`{}` kills in {hits} shots, outside the declared firearm TTK band {lo}..={hi}",
            w.id
        );
    }
    assert_eq!(guns, 1, "the alpha data ships exactly one firearm");
}

/// A firearm reaching further than one shot can be sampled is refused at
/// boot, not clamped at tick time — the round's sampler wall, one clock
/// over. A clamped reach is a gun that shoots through cover past some
/// distance the data never admits to.
#[test]
fn a_firearm_that_outreaches_the_collision_sampler_is_refused() {
    refuses_bake(
        "weapons.toml",
        "range_m = 50",
        "range_m = 500",
        "shoot through cover",
    );
}

/// A firearm whose round carries `[[ammo]]` ballistics is refused at boot.
///
/// The bow's refusal inverted, and the reason is the same one: the pairing
/// is what tells a projectile from a hitscan, so a round that is both would
/// leave the question to whichever reader asked it.
#[test]
fn a_firearm_whose_round_has_ballistics_is_refused() {
    refuses(
        "weapons.toml",
        "ammo = [\"item.pistol_ammo\"]",
        "ammo = [\"item.arrow_wood\"]",
        "it is a projectile",
    );
}

/// A muzzle speed the collision sampler cannot trace is refused at boot,
/// not clamped at tick time. A clamped projectile is a weapon whose reach
/// is a lie the data never admits to, and it would first be noticed as an
/// arrow passing through a wall.
///
/// **The refusal belongs to the round, not to the bow, and that is the
/// point of where it now lives** (`reference/PROJECTILES.md` §9.3). While
/// the speed sat on the weapon, an untraceably fast arrow was only refused
/// if some weapon declared that speed; with the speed on the ammo, the
/// arrow is refused whichever bow picks it up — including a bow added later
/// that lists it as a second round.
#[test]
fn a_round_that_outruns_the_collision_sampler_is_refused() {
    refuses_bake(
        "weapons.toml",
        "speed_mps = 40",
        "speed_mps = 400",
        "collision sampler",
    );
}

/// A bow naming a round with no `[[ammo]]` row is refused at boot.
///
/// This is the check the schema move exists for. A bow used to carry its own
/// ballistics and could not be missing them; now it names rounds that have
/// to supply them, and the failure mode without this refusal is quiet — the
/// round bakes to a zero-speed `AmmoDef`, `ammo_def` filters it out, and the
/// bow silently refuses to fire with a full quiver.
#[test]
fn a_bow_whose_round_has_no_ballistics_is_refused() {
    refuses(
        "weapons.toml",
        "ammo = [\"item.arrow_wood\"]",
        "ammo = [\"item.cloth\"]",
        "no [[ammo]] row",
    );
}

/// A weapon may not list the same round twice.
///
/// Order is the whole of the ammo policy until a switch verb exists, so a
/// duplicate is not harmless padding — it is an entry that can never be
/// reached, in the one field whose meaning is its ordering.
#[test]
fn a_weapon_that_lists_a_round_twice_is_refused() {
    refuses(
        "weapons.toml",
        "ammo = [\"item.arrow_wood\"]",
        "ammo = [\"item.arrow_wood\", \"item.arrow_wood\"]",
        "listed twice",
    );
}

/// A round with no muzzle speed is refused rather than defaulted.
///
/// Zero is a division at the shot (`range_mm / speed`), and it is also the
/// inert value the whole table starts at, so a shipped zero would be
/// indistinguishable from an unarmed slot.
#[test]
fn a_round_that_cannot_fly_is_refused() {
    refuses(
        "weapons.toml",
        "speed_mps = 40",
        "speed_mps = 0",
        "muzzle speed",
    );
}

/// Ballistics live on the round, and the whole point is that a second round
/// on one bow flies by its own numbers.
///
/// The shipped data lists one round per bow, so nothing in `weapons.toml`
/// exercises the list — this builds a two-round bow and proves the bake
/// keeps both, in order, with each round's own speed. Without this, §9.3's
/// capacity is asserted by a comment and nothing else.
#[test]
fn one_bow_carries_several_rounds_each_with_its_own_ballistics() {
    let mut src = sources();
    let w = src
        .iter_mut()
        .find(|(n, _)| *n == "weapons.toml")
        .expect("weapons.toml is a source");
    w.1 = w.1.replace(
        "ammo = [\"item.arrow_wood\"]",
        "ammo = [\"item.arrow_wood\", \"item.arrow_metal\"]",
    );
    let c = build(&src).expect("a bow with two rounds is legal content");
    let cc = c.bake_combat().expect("and it bakes");

    let bow = c.item_index("item.bow").expect("the bow is an item");
    let wood = c.item_index("item.arrow_wood").expect("wood is an item");
    let metal = c.item_index("item.arrow_metal").expect("metal is an item");
    let baked = cc.ranged[bow as usize];

    assert_eq!(
        [baked.ammo[0], baked.ammo[1]],
        [wood, metal],
        "the bow keeps both rounds in declared order"
    );
    let a = cc.ammo_def(wood).expect("wood is armed");
    let b = cc.ammo_def(metal).expect("metal is armed");
    assert_ne!(
        a.speed_mmpt, b.speed_mmpt,
        "the two rounds must differ, or this proves nothing about per-round ballistics"
    );
    // And the flight the sim would derive differs with them — the reason
    // `life_ticks` could not stay a baked constant on the weapon.
    assert_ne!(
        baked.range_mm / a.speed_mmpt as u32,
        baked.range_mm / b.speed_mmpt as u32,
        "one bow's two rounds must not share a flight time"
    );
}

// ---------------------------------------------------------------------------
// The spawn kit
//
// **It is on every player's critical path now**, which is a change of kind and
// not of degree. Until 2026-08-15 it was testing scaffolding — nine entries
// whose own comment called them that — and this block existed so a kit that
// silently granted nothing would be found by a gate rather than by somebody
// wondering why their hands were empty. It is now the reference's naked spawn
// (a rock and a torch, DECISIONS.md 2026-08-15), it is granted again on every
// respawn (`World::wake`), and with bare hands paying nothing on any swung
// node it is the ONLY thing standing between a fresh body and a world it
// cannot touch. So the assertions below pin the decision, not the plumbing.

/// The kit is exactly the rock and the torch, on the belt, and the rock is a
/// live tool on every node the game is farmed from.
///
/// Three claims, each of which was true of the old kit for a different reason
/// and would be a different bug if it broke:
///
/// - **Exactly two entries, in order.** The spoken kit is "a rock and a
///   torch"; a third entry is content nobody spoke.
/// - **Both on the belt.** `grant_kit` writes slots in order, so this is
///   free today — but it is the property that makes the kit usable without a
///   trip to the inventory, and it was load-bearing when the kit was nine
///   deep and could push a tool past `HOTBAR_SLOTS`.
/// - **Every entry is a single hand item, never a stack of material.** This
///   is what makes the kit safe to re-grant on death: the old kit could not
///   be, because 900 wood on every respawn is an item printer, and that
///   arithmetic is the whole of `world.rs::wake`'s argument. A material row
///   added back here re-opens it silently — nothing in `sim-core` can see
///   the difference between a tool and a stack of wood.
#[test]
fn the_spawn_kit_bakes_and_seats() {
    let c = build(&sources()).expect("content builds");
    let kit = c.bake_spawn_kit().expect("the kit bakes");

    let rock = c.item_index("item.rock").expect("the rock is an item");
    let torch = c.item_index("item.torch").expect("the torch is an item");
    let granted: Vec<(u16, u16)> = kit.stacks[..kit.count as usize]
        .iter()
        .map(|s| (s.item, s.count))
        .collect();
    assert_eq!(
        granted,
        vec![(rock, 1), (torch, 1)],
        "the naked spawn is a rock and a torch, in that order \
         (DECISIONS.md 2026-08-15) — this kit is something else"
    );
    assert!(
        kit.count as usize <= sim_core::limits::HOTBAR_SLOTS,
        "the kit runs past the belt: {} entries",
        kit.count
    );

    // Tools, not materials — the re-grant safety property, read off the
    // shipped item rows rather than off the two ids above.
    for e in &c.balance.spawn_kit {
        let item = c.item(&e.item).expect("kit names a shipped item");
        assert_eq!(
            item.slot,
            content::schema::EquipSlot::Hand,
            "`{}` is not a hand item — a kit of materials cannot be \
             re-granted on death (world.rs::wake)",
            e.item
        );
        assert_eq!(
            e.count, 1,
            "`{}` grants {} — a kit entry above 1 is a stack of goods, \
             which is the item printer `wake` is priced against",
            e.item, e.count
        );
    }

    // And the rock actually works: with no `hand` row on any swung node,
    // a kit whose tool paid nothing would be a naked spawn wearing a kit.
    let gc = c.bake_gather().expect("gather bakes");
    let mut swung = 0;
    for node in gc.nodes.iter() {
        if node.output == sim_core::gather::NO_ITEM || node.hits < 2 {
            continue; // the bush is a one-hit pickup, not a swung node
        }
        swung += 1;
        assert!(
            node.yield_for(rock) > 0,
            "a swung node pays the spawn kit's rock nothing — the loop \
             cannot start from the beach"
        );
        assert_eq!(
            node.yield_for(sim_core::gather::NO_ITEM),
            0,
            "a swung node still pays bare hands — DECISIONS.md 2026-08-15"
        );
    }
    assert!(swung >= 4, "only {swung} swung nodes — this gate got thin");
}

/// Absent is legal, and it means naked — **while bare hands can start the
/// loop.** The default matters more than the alpha kit does: a public shard
/// wants a beach spawn, and `#[serde(default)]` is what lets that be
/// expressed by deleting a table rather than by editing code.
///
/// The fixture restores a `hand` row on the tree first, because over the
/// SHIPPED gatherables (no swung node pays bare hands, DECISIONS.md
/// 2026-08-15) an absent kit is an unwinnable world and the boot rule
/// refuses it — that half is `a_kit_that_cannot_start_the_loop_is_refused`.
#[test]
fn no_spawn_kit_is_a_naked_spawn() {
    let mut srcs = sources();
    let g = srcs
        .iter_mut()
        .find(|(n, _)| *n == "gatherables.toml")
        .unwrap();
    let anchor = "[gatherable.yield_per_hit]\n\"item.rock\" = 50";
    assert!(
        g.1.contains(anchor),
        "fixture rot: the tree's rock row moved"
    );
    g.1 = g.1.replace(
        anchor,
        "[gatherable.yield_per_hit]\nhand = 25\n\"item.rock\" = 50",
    );
    let entry = srcs.iter_mut().find(|(n, _)| *n == "balance.toml").unwrap();
    let cut = entry
        .1
        .find("[[spawn_kit]]")
        .expect("the alpha kit is there");
    entry.1.truncate(cut);
    let c = build(&srcs).expect("with a hand row back, content without a spawn kit validates");
    let kit = c.bake_spawn_kit().expect("an absent kit bakes");
    assert_eq!(kit.count, 0, "an absent kit granted something");
}

/// The boot rule (NOW.md §0kit remainder 2, landed 2026-08-17): with no
/// `hand` row on any swung node, a kit that grants no paying tool boots a
/// world where every swing is refused forever — unwinnable, and until this
/// rule it validated green. Three cases: the empty kit, the kit of
/// non-tools, and the coupling itself (a restored hand row makes the empty
/// kit a legal naked spawn again, so the rule fires on the pair of tables
/// and never on the kit alone).
#[test]
fn a_kit_that_cannot_start_the_loop_is_refused() {
    // An empty kit over handless nodes: delete the whole table.
    let mut srcs = sources();
    let entry = srcs.iter_mut().find(|(n, _)| *n == "balance.toml").unwrap();
    let cut = entry
        .1
        .find("[[spawn_kit]]")
        .expect("the alpha kit is there");
    entry.1.truncate(cut);
    let err = build(&srcs).expect_err("an empty kit over handless swung nodes booted");
    assert!(
        err.contains("no tool any swung node pays"),
        "expected the unwinnable-spawn refusal, got: {err}"
    );

    // A kit of only the torch: an item, a hand item even, and a tool no
    // swung node has a yield row for.
    refuses(
        "balance.toml",
        "[[spawn_kit]]\nitem = \"item.rock\"\ncount = 1\n\n",
        "",
        "no tool any swung node pays",
    );

    // The coupling: restore a hand row on one swung node and the same
    // empty kit is a naked beach spawn again, not a refusal.
    let mut srcs = sources();
    let g = srcs
        .iter_mut()
        .find(|(n, _)| *n == "gatherables.toml")
        .unwrap();
    let anchor = "[gatherable.yield_per_hit]\n\"item.rock\" = 50";
    assert!(
        g.1.contains(anchor),
        "fixture rot: the tree's rock row moved"
    );
    g.1 = g.1.replace(
        anchor,
        "[gatherable.yield_per_hit]\nhand = 25\n\"item.rock\" = 50",
    );
    let entry = srcs.iter_mut().find(|(n, _)| *n == "balance.toml").unwrap();
    let cut = entry
        .1
        .find("[[spawn_kit]]")
        .expect("the alpha kit is there");
    entry.1.truncate(cut);
    build(&srcs).expect("a hand row on the tree makes the empty kit legal again");
}

/// The four refusals, re-anchored on the rock when the kit became a rock
/// and a torch (2026-08-15). They read the same because the rules did not
/// move — only the row they are spelled against.
#[test]
fn spawn_kit_refusals() {
    // An item the tables do not have.
    refuses(
        "balance.toml",
        "[[spawn_kit]]\nitem = \"item.rock\"",
        "[[spawn_kit]]\nitem = \"item.jetpack\"",
        "no such item",
    );
    // A count of zero — a slot that would draw empty.
    refuses(
        "balance.toml",
        "item = \"item.rock\"\ncount = 1",
        "item = \"item.rock\"\ncount = 0",
        "grants 0",
    );
    // Past the item's own stack size, which for the rock is 1 — so this
    // case is now tighter than it was against the hammer, not looser.
    refuses(
        "balance.toml",
        "item = \"item.rock\"\ncount = 1",
        "item = \"item.rock\"\ncount = 99",
        "past its own stack size",
    );
    // The same item twice — `grant_kit` writes slots and never merges, so
    // this is a typo that halves what the author meant.
    refuses(
        "balance.toml",
        "[[spawn_kit]]\nitem = \"item.rock\"\ncount = 1",
        "[[spawn_kit]]\nitem = \"item.rock\"\ncount = 1\n\n[[spawn_kit]]\nitem = \"item.rock\"\ncount = 1",
        "granted twice",
    );
}

/// The shipped pig crosses wall 7 intact: content says seconds, metres and
/// percentages, and the sim receives ticks, centimetres and a move axis.
#[test]
fn the_shipped_pig_bakes() {
    let c = Content::load_dir(&content_dir()).expect("shipped content must load");
    let mc = c.bake_mobs().expect("the pig must bake");
    let pig = mc.def(sim_core::mob::MOB_PIG);

    assert_eq!(pig.hp, 80);
    // 50% and 70% of the −127..=127 axis, floored.
    assert_eq!(pig.gait, 63);
    assert_eq!(pig.flee_gait, 88);
    // **The catchability relation, and it is gameplay rather than balance.**
    // The flight rides the sprint button, so `flee_gait/127` of
    // `SPRINT_SPEED` is the pig's top speed and the player's is
    // `SPRINT_SPEED` flat. A species at or above 127 can never be caught on
    // foot however long the chase runs, which is what shipped for an hour
    // on 2026-08-08 with every gate green — the defect was found by booting
    // the game and looking. This is the assertion that would have caught it.
    assert!(
        pig.flee_gait < 127,
        "a pig fleeing at {} of 127 runs at or above the player's sprint — \
         it can never be melee'd, and no gate but this one notices",
        pig.flee_gait
    );
    // Seconds → ticks at the sim's own rate, so this asserts the
    // conversion and not the number: 3 s and 300 s at 30 Hz.
    assert_eq!(pig.flee_ticks as u32, 3 * sim_core::limits::TICK_HZ);
    assert_eq!(pig.respawn_ticks, 300 * sim_core::limits::TICK_HZ);
    // Metres → centimetres, the unit the sim compares distances in.
    assert_eq!(pig.roam_cm, 6_000);
    assert_eq!(pig.spook_cm, 1_200);
    // Prey's flinch is not clock-keyed. Asserted rather than assumed
    // because the field is required on every row, so "the pig's night
    // radius" is a number somebody chose and not one the schema defaulted:
    // equal is the choice, and this is where changing it costs a test.
    assert_eq!(pig.night_spook_cm, pig.spook_cm);
    // The leash must be wider than the fright radius or a pig spends its
    // life being turned around — validate refuses it, assert it here too
    // because this is the pair that produces the behaviour.
    assert!(pig.roam_cm > pig.spook_cm);

    // Drops resolve to real item indices, in file order, and the tail is
    // `NO_ITEM` rather than a zero-count row nothing distinguishes.
    for i in 0..3 {
        assert_ne!(pig.loot[i].item, sim_core::gather::NO_ITEM);
        assert!(pig.loot[i].count > 0);
    }
    assert_eq!(pig.loot[3].item, sim_core::gather::NO_ITEM);
    assert_ne!(pig.loot[0].item, pig.loot[1].item);
}

/// **The shipped wolf hunts a narrower circle after dusk, and the sim asks
/// the right question to find that out.**
///
/// Two halves, deliberately in one test because either alone passes for the
/// wrong reason: the *table* (30 m by day, 15 m by night, through the same
/// metres→centimetres crossing every other radius makes) and the *selector*
/// (`spook_at` returns the day number in daylight and the night number
/// after dusk). A table with no selector is a field nothing reads; a
/// selector over equal numbers is a branch nothing distinguishes.
///
/// The direction is asserted as an inequality rather than as `15`, so a
/// balance pass may retune the number without touching this file — but it
/// may not quietly invert the design, which is the part that has a source
/// (`content/mobs.toml`'s comment, and `DECISIONS.md` §open "nocturnal
/// senses").
#[test]
fn the_shipped_wolf_hunts_a_narrower_circle_after_dusk() {
    let c = Content::load_dir(&content_dir()).expect("shipped content must load");
    let mc = c.bake_mobs().expect("the wolf must bake");
    let wolf = mc.def(sim_core::mob::MOB_WOLF);

    assert_eq!(wolf.spook_cm, 3_000);
    assert_eq!(wolf.night_spook_cm, 1_500);
    assert!(
        wolf.night_spook_cm < wolf.spook_cm,
        "the predator's night radius must not be the wider one: the \
         reference game shipped wider-at-night and removed it"
    );
    // The leash still clears the widest radius in force at any hour.
    assert!(wolf.roam_cm > wolf.spook_cm.max(wolf.night_spook_cm));
    // And the bite is reachable at the tighter hour, or it is a bite on
    // something the animal never noticed.
    assert!(wolf.attack_range_cm <= wolf.night_spook_cm);

    // The selector, against the shipped table.
    let day = (0..sim_core::limits::DAY_TICKS)
        .find(|&t| !sim_core::world::is_night(t))
        .expect("the cycle must contain a day");
    let night = (0..sim_core::limits::DAY_TICKS)
        .find(|&t| sim_core::world::is_night(t))
        .expect("the cycle must contain a night");
    assert_eq!(wolf.spook_at(day), wolf.spook_cm);
    assert_eq!(wolf.spook_at(night), wolf.night_spook_cm);
}

/// **The loop, across three content files that cannot see each other.**
///
/// The pig pays a raw food, the campfire is the only station that turns it
/// into a cooked one, and the cooked one is the only half of the pair the
/// eat verb accepts. Each of those is one row in a different file, none of
/// them references the others by anything but an item id, and any one of
/// them dropping out leaves the other two validating perfectly while the
/// player is left holding an item with no use — which is exactly the state
/// both halves shipped in for a day (`content/cooking.toml`'s own header).
#[test]
fn the_kill_the_fire_and_the_meal_are_one_loop() {
    let c = Content::load_dir(&content_dir()).expect("shipped content must load");
    let mc = c.bake_mobs().expect("the pig bakes");
    let pig = mc.def(sim_core::mob::MOB_PIG);
    let raw = c.item_index("item.raw_meat").expect("raw meat is an item");
    let cooked = c
        .item_index("item.cooked_meat")
        .expect("cooked meat is an item");

    // 1 — something in the world pays the raw half.
    assert!(
        pig.loot.iter().any(|s| s.item == raw && s.count > 0),
        "nothing on the island drops raw meat, so the fire has no job again"
    );

    // 2 — a fire, and only a fire, turns it into the cooked half.
    let row = c
        .cooks
        .iter()
        .find(|k| k.input == "item.raw_meat")
        .expect("no cook row consumes raw meat");
    assert_eq!(row.output, "item.cooked_meat");
    assert_eq!(row.station, content::schema::CookStation::Fire);

    // 3 — the cooked half is food and the raw half is not. That asymmetry
    // IS the verb: without it the fire is optional and the walk is not a
    // loop, it is a detour.
    assert!(
        c.consumables.iter().any(|k| k.id == "item.cooked_meat"),
        "cooked meat is not food, so cooking pays nothing"
    );
    assert!(
        !c.consumables.iter().any(|k| k.id == "item.raw_meat"),
        "raw meat is edible, so the campfire is decoration again"
    );

    // 4 — and the two BAKED tables agree on the index, not just the two
    // toml files on the string. The rows above are matched by item id; the
    // sim never sees an id, only a `u16` rank, and the pig's table and the
    // oven's are baked by different functions. A drop the fire would refuse
    // is the one way this loop can break with every row present.
    let cooked_row = c
        .bake_cooking()
        .expect("cooking bakes")
        .row_for(sim_core::deploy::ARCH_FIRE, raw)
        .copied()
        .expect("a fire does not accept the index the pig actually pays");
    assert_eq!(cooked_row.output, cooked);
    assert_eq!(
        pig.loot
            .iter()
            .filter(|s| s.item != sim_core::gather::NO_ITEM && s.count > 0)
            .filter(|s| s.item == raw)
            .count(),
        1,
        "the pig pays raw meat more than once"
    );
}

/// **The burnt link — the loop's fourth row, and the fire's own clock.**
///
/// A meal left on a lit fire keeps cooking: the burnt row's INPUT is the
/// cooked row's OUTPUT, which is the whole mechanic (the oven advances
/// whatever row matches what its slots hold, so overcooking is content and
/// not code — `sim-core/oven.rs`'s header predicted exactly this shape).
/// Delete just the burnt cook row and every other row still validates
/// while the fire silently stops being able to ruin anything — this test
/// is what notices.
#[test]
fn the_meal_left_on_the_fire_burns() {
    let c = Content::load_dir(&content_dir()).expect("shipped content must load");
    let burnt = c
        .item_index("item.burnt_meat")
        .expect("burnt meat is an item");
    let cooked = c
        .item_index("item.cooked_meat")
        .expect("cooked meat is an item");

    // 1 — the chain links: the burnt row consumes what the cooked row
    // produces, at the same station. Matched through the cooked row rather
    // than by literal id so the link itself is what is asserted.
    let cooked_row = c
        .cooks
        .iter()
        .find(|k| k.input == "item.raw_meat")
        .expect("no cook row consumes raw meat");
    let burnt_row = c
        .cooks
        .iter()
        .find(|k| k.input == cooked_row.output)
        .expect("nothing overcooks the cooked meat — the burnt state is gone");
    assert_eq!(burnt_row.output, "item.burnt_meat");
    assert_eq!(burnt_row.station, cooked_row.station);

    // 2 — burnt is barely edible: the conservative call (the repo's
    // reference notes give the burnt state's shape and no numbers) is the
    // worst food in the set. STRICTLY worst — tying a real meal would make
    // overcooking free — and it never heals, because a meal you ruined is
    // not a bandage.
    let row = c
        .consumables
        .iter()
        .find(|k| k.id == "item.burnt_meat")
        .expect("burnt meat is not food at all — the reference's burnt is barely edible");
    assert!(row.food > 0, "burnt meat must be worth more than nothing");
    assert_eq!(row.health, 0, "a ruined meal must not heal");
    for other in c.consumables.iter().filter(|k| k.id != "item.burnt_meat") {
        if other.food > 0 {
            assert!(
                row.food < other.food,
                "burnt meat ({} food) must be strictly the worst food in the \
                 set, but `{}` pays {}",
                row.food,
                other.id,
                other.food
            );
        }
    }

    // 3 — the BAKED table carries the link too: the index the fire's
    // finished cook pays into its own slots is an index the fire accepts
    // again. Same argument as the loop test's step 4 — the sim never sees
    // an id, only a rank.
    let baked = c.bake_cooking().expect("cooking bakes");
    let b = baked
        .row_for(sim_core::deploy::ARCH_FIRE, cooked)
        .copied()
        .expect("a fire does not accept the cooked meat it just produced");
    assert_eq!(b.output, burnt);

    // 4 — the chain terminates: burnt meat is the end state, not a fuel
    // and not a further input. A burnt→X row would put the fire on a
    // treadmill nothing in the design asked for.
    assert!(
        !c.cooks.iter().any(|k| k.input == "item.burnt_meat"),
        "burnt meat cooks into something — the overcook chain must end"
    );
}

/// Every consumable id in `c` with no producer, walked from the live verbs.
///
/// The seed set is what the world pays DIRECTLY, one entry per verb the sim
/// actually runs: a swing on a node (gather primary + secondary), a kill
/// (mob drops), an opened container — every table `bake::container_index`
/// knows, because a barrel is smashed (`gather::smash`) and a crate or a
/// cache is opened (`worldcont::open`, world containers v0, 2026-08-14),
/// while a table naming any other container is refused at bake and seeds
/// nothing (this walk counted barrels alone until 2026-08-17, on the
/// then-true reason "no verb opens a cache or a crate yet") — and the
/// spawn kit. The closure then walks the transformations: the oven's burn
/// (fuel → byproduct), cook rows, and recipes whose inputs — and whose
/// station, itself an item that must be produced — are all reachable, to a
/// fixpoint.
fn unreachable_consumables(c: &Content) -> Vec<String> {
    let mut have = std::collections::BTreeSet::new();
    for g in &c.gatherables {
        have.insert(g.output.as_str());
        if let Some(s) = &g.secondary {
            have.insert(s.output.as_str());
        }
    }
    for m in &c.mobs {
        for d in &m.drops {
            have.insert(d.item.as_str());
        }
    }
    for l in &c.loot_tables {
        if content::bake::container_index(&l.container).is_some() {
            for e in &l.entries {
                have.insert(e.item.as_str());
            }
        }
    }
    for s in &c.balance.spawn_kit {
        have.insert(s.item.as_str());
    }
    loop {
        let mut grew = false;
        if have.contains(c.fuel.item.as_str()) {
            grew |= have.insert(c.fuel.byproduct.as_str());
        }
        for k in &c.cooks {
            if have.contains(k.input.as_str()) {
                grew |= have.insert(k.output.as_str());
            }
        }
        for r in &c.recipes {
            let station_ok = match r.station {
                content::schema::Station::None => true,
                content::schema::Station::Workbench1 => have.contains("item.workbench1"),
                content::schema::Station::Workbench2 => have.contains("item.workbench2"),
                content::schema::Station::Workbench3 => have.contains("item.workbench3"),
                content::schema::Station::Furnace => have.contains("item.furnace"),
            };
            if station_ok && r.inputs.iter().all(|i| have.contains(i.item.as_str())) {
                grew |= have.insert(r.output.as_str());
            }
        }
        if !grew {
            break;
        }
    }
    c.consumables
        .iter()
        .filter(|k| !have.contains(k.id.as_str()))
        .map(|k| k.id.clone())
        .collect()
}

/// **Every consumable the content ships is REACHABLE** — the general form
/// of the meal-loop gate, and the one that would have caught mushrooms and
/// corn the day `consumables.toml` grew them. Five rows shipped 2026-08-03
/// with the survival clock and two of them (`item.mushrooms`, `item.corn`)
/// were producible by nothing for six days: parsed, validated, hashed into
/// the WAL header, drawn in the eat verb's tables, and absent from every
/// verb that puts an item in a hand. The clock wall (`validate.rs`) only
/// asks that SOMETHING answers hunger, so berries alone kept it green —
/// this asks the per-row question the wall deliberately does not.
#[test]
fn every_consumable_the_content_ships_is_reachable() {
    let c = build(&sources()).expect("shipped content builds");
    assert!(
        c.consumables.len() >= 5,
        "the consumable set shrank to {} rows — this gate is checking little",
        c.consumables.len()
    );
    let missing = unreachable_consumables(&c);
    assert!(
        missing.is_empty(),
        "consumables nothing in the world can produce: {missing:?} — every \
         row in consumables.toml must be producible by a live verb chain \
         (gather, kill, container smash/open, spawn kit; then \
         burn/cook/recipe closure)"
    );

    // The enumeration is honest: strip one producer row and its consumable
    // must be reported stranded — exactly, so a walk that quietly counted
    // everything (or nothing) goes red here rather than in a shipped file.
    let mut srcs = sources();
    let g = srcs
        .iter_mut()
        .find(|(n, _)| *n == "gatherables.toml")
        .unwrap();
    let row = "\n[gatherable.secondary]\noutput = \"item.mushrooms\"\nper_hit = 1\n";
    assert!(
        g.1.contains(row),
        "fixture rot: the tree's mushroom row moved"
    );
    g.1 = g.1.replace(row, "\n");
    let mutant = build(&srcs).expect("still valid — berries keep the clock answered");
    assert_eq!(
        unreachable_consumables(&mutant),
        vec!["item.mushrooms".to_string()],
        "deleting the tree's mushroom secondary must strand exactly the mushrooms"
    );

    let mut srcs = sources();
    let l = srcs.iter_mut().find(|(n, _)| *n == "loot.toml").unwrap();
    let row = "    { item = \"item.corn\", weight = 8, count_min = 2, count_max = 4 },\n";
    assert!(
        l.1.contains(row),
        "fixture rot: the barrel's corn row moved"
    );
    l.1 = l.1.replace(row, "");
    let mutant = build(&srcs).expect("still valid — the bush and the tree keep the clock answered");
    assert_eq!(
        unreachable_consumables(&mutant),
        vec!["item.corn".to_string()],
        "deleting the barrel's corn row must strand exactly the corn"
    );

    // World containers v0 (2026-08-14) made the crate and the cache
    // verb-openable (`worldcont::open`), so a consumable that exists ONLY
    // in one of their tables is reachable — the pre-widening walk (barrel
    // rows alone) called exactly this fixture stranded, which is the
    // regression this pins.
    let corn_row = "    { item = \"item.corn\", weight = 8, count_min = 2, count_max = 4 },\n";
    for anchor in [
        // The crate's metal_frags row — unique counts, so the corn lands
        // in the crate's table.
        "    { item = \"item.metal_frags\", weight = 15, count_min = 25, count_max = 50 },\n",
        // The cache's — same, for the third container kind.
        "    { item = \"item.metal_frags\", weight = 15, count_min = 15, count_max = 30 },\n",
    ] {
        let mut srcs = sources();
        let l = srcs.iter_mut().find(|(n, _)| *n == "loot.toml").unwrap();
        assert!(
            l.1.contains(corn_row) && l.1.contains(anchor),
            "fixture rot: the corn row or the metal_frags anchor moved"
        );
        l.1 = l.1.replace(corn_row, "");
        l.1 = l.1.replace(anchor, &format!("{anchor}{corn_row}"));
        let mutant = build(&srcs).expect("moving the corn between containers is legal content");
        assert_eq!(
            unreachable_consumables(&mutant),
            Vec::<String>::new(),
            "corn only in a verb-openable container's table is reachable — \
             a walk that counts barrels alone is stale since world containers v0"
        );
    }
}

/// The mob validator refuses what it claims to. Each of these is a content
/// bug that would read as a bug in the sim if it booted.
#[test]
fn mob_refusals() {
    refuses("mobs.toml", "hp = 80", "hp = 0", "zero hp");
    refuses(
        "mobs.toml",
        "walk_pct = 50",
        "walk_pct = 0",
        "1–100 percent",
    );
    refuses(
        "mobs.toml",
        "walk_pct = 50",
        "walk_pct = 140",
        "1–100 percent",
    );
    refuses("mobs.toml", "roam_m = 60", "roam_m = 10", "treadmill");
    refuses(
        "mobs.toml",
        "flee_seconds = 3",
        "flee_seconds = 0",
        "scenery",
    );
    // **The same three bands, stated at the other hour.** Before nocturnal
    // senses each of these read `spook_m` alone, which was total when there
    // was one radius and became a hole the moment there were two: a species
    // could be given a night radius outside its own leash, a night it is
    // blind through, or a bite reaching further than it can notice after
    // dark, and every band above would still have passed it. The wolf's
    // `night_spook_m = 15` is the anchor because it is the only row that
    // carries that value.
    refuses(
        "mobs.toml",
        "night_spook_m = 15",
        "night_spook_m = 200",
        "treadmill",
    );
    refuses(
        "mobs.toml",
        "night_spook_m = 15",
        "night_spook_m = 0",
        "scenery",
    );
    refuses(
        "mobs.toml",
        "night_spook_m = 15",
        "night_spook_m = 1",
        "outreaches",
    );
    refuses(
        "mobs.toml",
        "respawn_seconds = 300",
        "respawn_seconds = 0",
        "zero respawn",
    );
    refuses(
        "mobs.toml",
        "item.fat",
        "item.unobtainium",
        "is not an item",
    );
    refuses("mobs.toml", "count = 15", "count = 0", "zero count");
    // A species the sim has no roster kind for is a boot refusal, not a
    // silently ignored row: the content hash would otherwise promise
    // wildlife the shard does not have.
    let mut srcs = sources();
    let e = srcs.iter_mut().find(|(n, _)| *n == "mobs.toml").unwrap();
    e.1 = e.1.replace("mob.pig", "mob.bear");
    let c = build(&srcs).expect("a well-formed unknown species reaches the bake");
    let err = c.bake_mobs().expect_err("an unknown species must not bake");
    assert!(err.contains("no roster kind"), "got: {err}");
}

// ---------------------------------------------------------------------------
// The research ladder (`content/research.toml` §requires, `validate::structural`,
// `bake_research`). Added 2026-08-15 with the ladder itself; before that day
// the whole table reached the sim as `ResearchContent::EMPTY`, because
// `bake_research` had no caller — so these tests exist as much to prove the
// *edges* work as to prove the table is installed at all, which is
// `crates/server/tests/boot_tables.rs`.
// ---------------------------------------------------------------------------

/// The shipped tree is a tree: it bakes, its one authored edge survives into
/// the sim's own bit space, and the row it points at is a root.
#[test]
fn the_shipped_research_tree_bakes_with_its_edge_intact() {
    let c = Content::load_dir(&content_dir()).expect("shipped content loads");
    let rc = c.bake_research().expect("shipped research bakes");

    assert_eq!(
        rc.row_count as usize,
        c.research.len(),
        "every authored row reaches the sim"
    );
    assert!(rc.row_count > 0, "an empty table is the pre-2026-08-15 bug");

    let idx = |id: &str| c.item_index(id).unwrap_or_else(|| panic!("shipped {id}"));
    let powder_recipe = c
        .recipe_index("recipe.gunpowder")
        .expect("shipped recipe.gunpowder");

    let satchel = rc
        .row_for(idx("item.satchel_charge"))
        .expect("satchel is researchable");
    assert_eq!(
        satchel.requires, powder_recipe,
        "the satchel's prerequisite is gunpowder's RECIPE INDEX — what the \
         tree verb looks up in `Player::known`, or the check means nothing"
    );

    let powder = rc
        .row_for(idx("item.gunpowder"))
        .expect("gunpowder is researchable");
    assert_eq!(
        powder.requires,
        sim_core::research::NO_RECIPE,
        "gunpowder is a root of the tree"
    );
}

/// The floor: an edge the craft graph already implies may not be dropped
/// from the tree. This is the drift that makes a tech tree lie — the recipe
/// says you need gunpowder, the tree says you do not, and a player buys a
/// blueprint for a thing they cannot make.
#[test]
fn a_prerequisite_the_recipe_already_implies_cannot_be_dropped() {
    refuses(
        "research.toml",
        "item = \"item.satchel_charge\"\ncost = 75\nrequires = \"item.gunpowder\"",
        "item = \"item.satchel_charge\"\ncost = 75",
        "must require it",
    );
}

/// Every other way an edge can be wrong.
#[test]
fn research_edge_refusals() {
    // A row that requires itself is a cycle of length one.
    refuses(
        "research.toml",
        "item = \"item.satchel_charge\"\ncost = 75\nrequires = \"item.gunpowder\"",
        "item = \"item.satchel_charge\"\ncost = 75\nrequires = \"item.satchel_charge\"",
        "requires itself",
    );
    // A prerequisite that names no item at all.
    refuses(
        "research.toml",
        "item = \"item.satchel_charge\"\ncost = 75\nrequires = \"item.gunpowder\"",
        "item = \"item.satchel_charge\"\ncost = 75\nrequires = \"item.nonesuch\"",
        "is not an item",
    );
    // A prerequisite that is a real item but is not researchable: nobody can
    // ever learn it, so the row behind it is locked forever.
    refuses(
        "research.toml",
        "item = \"item.satchel_charge\"\ncost = 75\nrequires = \"item.gunpowder\"",
        "item = \"item.satchel_charge\"\ncost = 75\nrequires = \"item.rock\"",
        "is not researchable",
    );
    // The "same edge twice" case that stood here is DELETED by the
    // 2026-08-15 integration rather than skipped: `requires` is one parent
    // now, not a list, so a repeat is unrepresentable and the validator
    // check it exercised is gone with it.
}

/// A cycle, and the row stranded behind it, are one refusal — because
/// "can never be learned" is what a player experiences and a cycle is only
/// one cause of it. Gunpowder is made to depend on the satchel that depends
/// on gunpowder; the medkit is untouched and must stay reachable, which is
/// what proves the walk reports the stuck rows rather than giving up.
#[test]
fn a_prerequisite_cycle_is_refused() {
    let mut srcs = sources();
    let entry = srcs
        .iter_mut()
        .find(|(n, _)| *n == "research.toml")
        .expect("research.toml");
    entry.1 = entry.1.replace(
        "[[research]]\nitem = \"item.gunpowder\"\ncost = 40",
        "[[research]]\nitem = \"item.gunpowder\"\ncost = 40\nrequires = \"item.satchel_charge\"",
    );
    let err = build(&srcs).expect_err("a research cycle was accepted");
    assert!(
        err.contains("can never be learned"),
        "expected the reachability walk to refuse, got: {err}"
    );
    assert!(
        err.contains("item.gunpowder") && err.contains("item.satchel_charge"),
        "the refusal must name the stuck rows, got: {err}"
    );
    assert!(
        !err.contains("item.medkit"),
        "a root outside the cycle is still reachable and must not be named: {err}"
    );
}

/// The ladder is part of what a content set MEANS, so it digests. Two sets
/// whose tree is wired differently must not canonicalise identically — the
/// exact hole `canon.rs` documents for three other columns.
#[test]
fn the_hash_moves_with_the_ladder() {
    let base = Content::load_dir(&content_dir()).expect("shipped content loads");

    let mut srcs = sources();
    let entry = srcs
        .iter_mut()
        .find(|(n, _)| *n == "research.toml")
        .expect("research.toml");
    // A legal edge that changes the tree: the revolver behind gunpowder.
    // Legal because the floor is a minimum and authoring MORE is a design
    // call — which is exactly why the hash has to be able to see it.
    entry.1 = entry.1.replace(
        "[[research]]\nitem = \"item.arrow_metal\"\ncost = 20",
        "[[research]]\nitem = \"item.arrow_metal\"\ncost = 20\nrequires = \"item.gunpowder\"",
    );
    let moved = build(&srcs).expect("an added edge is legal content");
    assert_ne!(
        base.hash(),
        moved.hash(),
        "two contents whose tech tree differs canonicalise identically"
    );
}

/// **`content/armor.toml` reaches the sim.** Every row lands at its item's
/// index carrying the percent and the slot the file declares — the assertion
/// that stopped this file being priced, validated, hashed, balance-anchored
/// and *unread* from M1 to 2026-08-19 (`reference/ARMOR.md` §9.1).
///
/// Slot-checked as well as valued, because the slot is not decoration:
/// `Player::worn` is indexed by it, so a bake that keyed the head piece to
/// the body row would put a headwrap in a slot that never pays.
#[test]
fn bake_combat_arms_the_armor_the_data_prices() {
    use content::schema::ArmorSlot;
    use sim_core::combat::{WEAR_BODY, WEAR_HEAD, WEAR_NONE};

    let c = Content::load_dir(&content_dir()).expect("shipped content must load");
    let cc = c.bake_combat().expect("shipped armor must bake");
    assert!(
        c.armors.len() >= 3,
        "the shipped set prices {} armor rows — fewer than the three this \
         test was written against, so it is now proving less than it says",
        c.armors.len()
    );
    for a in &c.armors {
        let idx = c.item_index(&a.id).expect("armor arms an item") as usize;
        let baked = cc.armor[idx];
        assert_eq!(
            baked.reduction_pct as u32, a.reduction_pct,
            "`{}` reduction did not survive the bake",
            a.id
        );
        assert_eq!(
            baked.slot,
            match a.slot {
                ArmorSlot::Head => WEAR_HEAD,
                ArmorSlot::Body => WEAR_BODY,
            },
            "`{}` was baked into the wrong wear slot",
            a.id
        );
    }
    // And the table is not simply full: an item nobody armors stays inert,
    // so `slot == WEAR_NONE` still means "not wearable".
    let rock = c.item_index("item.rock").expect("the rock is an item") as usize;
    assert_eq!(
        cc.armor[rock].slot, WEAR_NONE,
        "a rock is wearable — the bake filled rows it was never given"
    );
}

/// **A body in the shipped burlap shirt takes six rock hits instead of
/// five**, computed through the sim's own reducer against the shipped
/// numbers. The one sentence this whole slice is for, in the crate that
/// owns the numbers.
#[test]
fn the_shipped_burlap_shirt_costs_an_attacker_a_swing() {
    let c = Content::load_dir(&content_dir()).expect("shipped content must load");
    let cc = c.bake_combat().expect("shipped content must bake");
    let rock = c
        .weapons
        .iter()
        .find(|w| w.id == "item.rock")
        .expect("the rock is a weapon");
    let shirt = c
        .armors
        .iter()
        .find(|a| a.id == "item.armor_burlap_body")
        .expect("the burlap shirt is priced");

    // Hits to kill, swung rather than divided: the loop `bake_combat_plays_
    // the_band_the_data_declares` already uses, with the reduction the sim
    // applies inserted where the sim applies it.
    let swings = |pct: u32| {
        let mut hp = cc.player_hp;
        let mut hits = 0u32;
        let per = sim_core::combat::reduce(rock.damage as u16, pct);
        assert!(per > 0, "a rock that deals nothing never kills");
        while hp > 0 {
            hp -= per.min(hp);
            hits += 1;
        }
        hits
    };
    assert_eq!(swings(0), 5, "a naked body and a rock");
    assert_eq!(
        swings(shirt.reduction_pct),
        6,
        "the shipped burlap shirt ({} %) changed nothing about a rock \
         fight — it is craftable for 20 cloth and must buy the wearer a \
         swing",
        shirt.reduction_pct
    );
}

/// **The band's arithmetic and the sim's are one function, not two that
/// agree by luck.**
///
/// `balance::hits_to_kill` divides `player_hp` by the exact rational
/// `damage × (100 − pct) / 100`; the sim computes
/// `combat::reduce(damage, pct)` once — an integer floor — and subtracts it
/// every swing. Those are different functions, and where they disagree the
/// band describes a fight nobody has (`findings/armor-design-20260818.md`
/// §5). They agree on every pair we ship; this is what says so, and it
/// covers the *set* percentages too, which no band currently reaches.
#[test]
fn the_band_and_the_sim_kill_in_the_same_number_of_hits() {
    let c = Content::load_dir(&content_dir()).expect("shipped content must load");
    let hp = c.balance.globals.player_hp;

    // Every single piece, plus every legal pairing of one head and one
    // body — which is what `Player::worn` can actually hold, and what
    // `combat::worn_pct` sums.
    let mut sets: Vec<u32> = vec![0];
    for a in &c.armors {
        sets.push(a.reduction_pct);
    }
    for h in c
        .armors
        .iter()
        .filter(|a| a.slot == content::schema::ArmorSlot::Head)
    {
        for b in c
            .armors
            .iter()
            .filter(|a| a.slot == content::schema::ArmorSlot::Body)
        {
            sets.push(h.reduction_pct + b.reduction_pct);
        }
    }
    assert!(
        sets.len() >= 6,
        "only {} protection values to check — the shipped armor set shrank \
         and this pin is now guarding almost nothing",
        sets.len()
    );

    let mut pairs = 0;
    for w in &c.weapons {
        if w.kind == content::schema::WeaponKind::Throwable {
            continue; // structure damage, no TTK
        }
        for &pct in &sets {
            let banded = content::balance::hits_to_kill(hp, w.damage, pct);
            let per = sim_core::combat::reduce(w.damage as u16, pct);
            assert!(per > 0, "`{}` under {pct}% deals nothing a hit", w.id);
            let mut left = hp as u16;
            let mut played = 0u32;
            while left > 0 {
                left -= per.min(left);
                played += 1;
            }
            assert_eq!(
                banded, played,
                "`{}` against {pct}% armor: the band computes {banded} hits \
                 and the sim plays {played}. The two arithmetics have \
                 drifted, so `armor_extra_hits_max` is now describing a \
                 fight nobody has",
                w.id
            );
            pairs += 1;
        }
    }
    assert!(
        pairs >= 20,
        "only {pairs} (weapon, armor) pairs were checked — the loop is \
         guarding nothing"
    );
}

/// An armor row the sim cannot represent never reaches it. A zero
/// reduction is the interesting one: `validate` has no opinion about it
/// (it only refuses over 90), but zero **is** the sim's "not armor"
/// sentinel, so a piece declaring no protection would silently become an
/// unwearable item rather than a useless one.
#[test]
fn an_armor_row_that_cannot_work_never_reaches_the_sim() {
    refuses_bake(
        "armor.toml",
        "reduction_pct = 15",
        "reduction_pct = 0",
        "protects from nothing",
    );
}

/// **No weapon outranges the band that decides who is told it fired.**
///
/// This is a relationship between a content number and a limit, and until
/// 2026-08-24 nothing held it from either side.
///
/// `EV_SHOT` is filtered to the shooter's class-D interest set
/// (`server/core.rs::body_event_visible`), so a client that cannot see the
/// hand is not told it loosed anything. Two things make that safe, and the
/// second is this gate: `render/tracer.rs` already discards a shot it has
/// no body to hang on, **and** no weapon can throw a projectile far enough
/// to reach somebody outside the band anyway. The first is behavioural and
/// holds whatever the numbers say; the second is arithmetic and a content
/// edit can break it in silence.
///
/// The failure it prevents is a real product decision arriving as an
/// accident: someone adds a 200 m rifle to `weapons.toml`, every gate in
/// the tree stays green, and its tracer stops existing for exactly the
/// players it was aimed at. If this goes red, the fix is **not** to raise
/// the number here — it is to decide whether that weapon wants a wider
/// filter (a range-aware radius rather than the interest bit) and to say
/// so in `DECISIONS.md`.
///
/// `AOI_ENTER_CM` rather than `AOI_EXIT_CM`: enter is the *narrower* band
/// and the one a client must have crossed to hold the body, so it is the
/// conservative side to measure against.
#[test]
fn no_weapon_outranges_the_interest_band() {
    let c = build(&sources()).expect("shipped content builds");
    let band_m = (sim_core::limits::AOI_ENTER_CM / 100) as u32;
    let worst = c
        .weapons
        .iter()
        .max_by_key(|w| w.range_m)
        .expect("content ships at least one weapon");
    assert!(
        worst.range_m < band_m,
        "`{}` reaches {} m and the interest band is {} m — a shot from \
         outside a client's band could land inside its world, and \
         `EV_SHOT`'s filter would have thrown the tracer away. Do not \
         raise this assertion; decide what that weapon's audience is.",
        worst.id,
        worst.range_m,
        band_m
    );
}

/// The three ranged rows' `structure` column reaches the sim.
///
/// **`the_raid_tool_crosses_into_the_sim`, one weapon kind over, and it was
/// broken in exactly the same way for longer.** `content/weapons.toml` has
/// given the bow, the crossbow and the revolver a `structure` since the
/// content crate was written; it is parsed, range-checked by
/// `balance.rs`'s two laws, and folded into the content hash by `canon.rs`
/// — so editing it moved the hash, moved the WAL header, and moved nothing
/// a player could see, because `bake_ranged` wrote a `RangedDef` that had
/// no field to put it in. Ranged structure damage v0 (2026-08-28) gave it
/// one; this is the assertion that keeps it wired.
///
/// The value is read off the file rather than typed, so a balance pass on
/// `weapons.toml` is not a red gate here — what is asserted is the
/// *crossing*, which is the thing that was silently absent.
#[test]
fn every_ranged_weapon_carries_its_structure_column_into_the_sim() {
    let c = build(&sources()).expect("shipped content bakes");
    let cc = c.bake_combat().expect("combat bakes");
    let mut seen = 0;
    for w in &c.weapons {
        if !matches!(
            w.kind,
            content::schema::WeaponKind::Bow | content::schema::WeaponKind::Firearm
        ) {
            continue;
        }
        seen += 1;
        let idx = c
            .item_index(&w.id)
            .unwrap_or_else(|| panic!("ranged weapon `{}` arms no item", w.id))
            as usize;
        assert_eq!(
            cc.ranged[idx].structure as u32, w.structure,
            "`{}` declares structure {} and the sim baked {} — the column is \
             hashed either way, so this drift is invisible to every other gate",
            w.id, w.structure, cc.ranged[idx].structure
        );
    }
    assert!(
        seen >= 3,
        "expected the bow, the crossbow and the revolver; found {seen} ranged \
         rows — if a row was cut, cut it here too rather than letting this \
         pass on an empty loop"
    );
}

/// Every deployable with a collision volume places on the **plane** and
/// nowhere else.
///
/// This is a claim about `content/deployables.toml` that a function in
/// `sim-core` depends on and cannot check. `collide::deploy_stop` returns a
/// four-part address so a shot can charge the thing it stopped on, and the
/// two stores share one address space — a door and its doorway have one —
/// so the walk has to supply a `loc`. The collision index does not carry
/// one: `ColMasks::solid` is a nibble per level holding an archetype code,
/// and nothing else. `LOC_PLANE` is therefore derived from the placement
/// class, and that derivation is only sound while every solid archetype
/// places `ground`, `foundation` or `any`.
///
/// **The failure it prevents is silent.** A future row that gave a solid
/// archetype the `doorway` class would place at `LOC_EDGE_XLO`, the shot
/// walk would keep saying `LOC_PLANE`, `Deploys::find_index` would find
/// nothing there and `World::chip` would drop the chip — a deployable that
/// stops every arrow and never loses a point of hp, with no event, no
/// refusal and no red gate anywhere.
///
/// Read off the file and the sim's own tables rather than typed, so this is
/// the crossing and not a copy of the roster.
#[test]
fn every_solid_deployable_places_on_the_plane() {
    use sim_core::build::LOC_PLANE;
    let c = build(&sources()).expect("shipped content bakes");
    let dc = c.bake_deployables().expect("deployables bake");
    let mut seen = 0;
    for i in 0..dc.def_count as usize {
        let def = &dc.defs[i];
        if sim_core::deploy::solid_vol(def.arch).is_none() {
            continue;
        }
        seen += 1;
        for loc in 0u8..=sim_core::build::LOC_DIAG_B {
            let fits = sim_core::deploy::loc_fits_placement(def.placement, loc);
            assert_eq!(
                fits,
                loc == LOC_PLANE,
                "deployable row {i} (archetype {}, placement {}) has a \
                 collision volume and {} loc {loc} — `collide::deploy_stop` \
                 names LOC_PLANE for every solid it finds, so a shot would \
                 charge an address this row does not occupy",
                def.arch,
                def.placement,
                if fits { "admits" } else { "refuses" }
            );
        }
    }
    assert_eq!(
        seen, 9,
        "expected exactly nine solid rows — the hearth, the box (twice), the \
         furnace, the three benches, the recycler and the research table — \
         and found {seen}. The floor used to be `>= 7`, which its own message \
         already contradicted: two solid rows could have been deleted with \
         this gate green (judged 2026-08-28). A row added here is a \
         deliberate edit to this number."
    );
}
