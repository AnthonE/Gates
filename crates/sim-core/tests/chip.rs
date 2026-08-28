//! Ranged structure damage v0 (2026-08-28) — a shot chips the wall it stops
//! on, through `World`.
//!
//! **`tests/shoot.rs` gates the address and this gates the write**, and the
//! split is the sim's own: `ranged` holds the collision index and finds the
//! piece, `World` holds the store, the content and the tick's removal budget
//! and charges it. Neither half is checkable from the other's seat — a
//! facing lives on a `PieceRec` and never reaches the column index
//! `shoot.rs` builds by hand, so the hard/soft rule is only askable here.
//!
//! What the slice actually changed is small and the reason it went unbuilt
//! for so long is not: `content/weapons.toml` has given the bow, the
//! crossbow and the revolver `structure = 1` since the content crate was
//! written, and `bake_ranged` dropped the column one line before
//! `RangedDef` could hold it. The number was parsed, range-checked,
//! balance-checked and folded into the content hash, and moved nothing.
//! `crates/content/tests/content.rs`'s
//! `every_ranged_weapon_carries_its_structure_column_into_the_sim` is the
//! gate on that half. (This line named a file that does not exist until
//! 2026-08-28 — the judge's finding on the pass that wrote it, and the
//! second dead test-path citation in three passes. `ls` the file.)
//!
//! **Fixtures, not shipped numbers.** The shipped bow chips 1 and a twig
//! wall has hundreds of hp, so a shipped-content case would be a thousand
//! ticks of loop to watch one wall fall. Every case here arms a fixture bow
//! whose `structure` is a stated fraction of the fixture wall's 100 hp, so
//! a fall lands inside a counted window — `combat::probe_fixture`'s own
//! reasoning for its 34, one table over.

use sim_core::build::{
    foundation_terrain_ok, BuildContent, BUILD_CELL_M, LEVEL_H_M, LOC_EDGE_XLO, LOC_PLANE,
};
use sim_core::combat::{AmmoDef, CombatContent, RangedDef, HARD_SIDE_STRUCTURE};
use sim_core::deploy::DeployContent;
use sim_core::gather::{cell_key, GatherContent, ItemStack, NO_ITEM};
use sim_core::input::{InputFrame, BTN_PRIMARY};
use sim_core::movement::{quant_y, Body};
use sim_core::ranged::SURF_BUILT;
use sim_core::world::{
    Command, SimEvent, World, EV_DEPLOY_REMOVED, EV_IMPACT, EV_PIECE_REMOVED, EV_STRUCT_HIT,
};

const SEED: u64 = 20260802;
/// The archer's network id — `event_roles.rs`' `BUILDER`, and the same
/// player builds and shoots here so the wall's facing is known.
const SHOOTER: u32 = 4;
/// `BuildContent::probe_fixture`'s rows: 0 is the foundation, 1 the wall.
const PIECE_FOUNDATION: u16 = 0;
const PIECE_WALL: u16 = 1;
/// That wall's hp, which every damage assertion below is a fraction of.
const WALL_HP: u32 = 100;
const GROUND: u8 = 0;
/// Item indices for the fixture bow and its round, **deliberately above
/// `probe_fixture`'s four melee rows and two armor rows**: an item that was
/// both a bow and a club would make `held_struct` answer, and the melee
/// raid verb would chip the wall this suite is watching a shot chip.
const BOW: u16 = 8;
const ARROW: u16 = 9;
const GUN: u16 = 10;
const ROUND: u16 = 11;
/// Yaw facing +x over the 256-entry LUT, and the stance is one cell in -x —
/// so a shot travels toward the `LOC_EDGE_XLO` face of the target cell.
/// The two hotbar slots the weapons sit in — inside `HOTBAR_SLOTS`, which
/// is the whole of what makes `sel` reach them.
const SLOT_BOW: usize = 4;
const SLOT_GUN: usize = 5;
const YAW_PLUS_X: u16 = 64 << 8;
const YAW_MINUS_X: u16 = 192 << 8;
/// How far from the wall the archer stands, in build cells.
const STANCE_CELLS: f32 = 1.0;
/// Ticks a case may spend waiting for an event before it is a failure
/// rather than a slow arrow. A 40 m/s arrow crosses one 3 m cell in about
/// three ticks; this is two orders of margin and still a bound.
const MAX_STEPS: u32 = 400;

/// The memoized haven — `event_roles.rs`' helper and its reason: without
/// it every fixture call re-solves a few thousand `terrain::height` taps.
fn hv(seed: u64) -> &'static sim_core::terrain::Haven {
    use std::cell::RefCell;
    thread_local! {
        static CACHE: RefCell<Vec<(u64, &'static sim_core::terrain::Haven)>> =
            const { RefCell::new(Vec::new()) };
    }
    let hit = CACHE.with(|c| c.borrow().iter().find(|(s, _)| *s == seed).map(|&(_, h)| h));
    if let Some(h) = hit {
        return h;
    }
    let h: &'static sim_core::terrain::Haven = Box::leak(Box::new(sim_core::terrain::haven(seed)));
    CACHE.with(|c| c.borrow_mut().push((seed, h)));
    h
}

/// Ask the sim's own rule for a buildable cell rather than typing a
/// coordinate — `event_roles.rs`' helper, and the reason worldgen moved
/// under a hand-typed cell twice.
fn buildable_cell(seed: u64) -> (u16, u16) {
    for r in 0..64i32 {
        for dz in -r..=r {
            for dx in -r..=r {
                if dx.abs() != r && dz.abs() != r {
                    continue;
                }
                let cx = (170 + dx).clamp(0, 1023) as u16;
                let cz = (170 + dz).clamp(0, 1023) as u16;
                if cx == cz {
                    continue;
                }
                let (x, z) = (
                    (cx as f32 + 0.5) * BUILD_CELL_M,
                    (cz as f32 + 0.5) * BUILD_CELL_M,
                );
                if foundation_terrain_ok(seed, hv(seed), x, z) {
                    return (cx, cz);
                }
            }
        }
    }
    panic!("no buildable cell within 64 cells — the generator changed under this test");
}

/// A bow that chips `structure`, and a revolver that chips the same.
///
/// Built on `probe_fixture` so the four melee rows and the wall's hp stay
/// the ones every other counted gate reasons about.
fn shooter_combat(structure: u16) -> CombatContent {
    let mut c = CombatContent::probe_fixture();
    c.ranged[BOW as usize] = RangedDef {
        damage: 30,
        ammo: [ARROW, NO_ITEM, NO_ITEM, NO_ITEM],
        // One tick, so a case that needs several shots is several ticks
        // rather than several seconds. The cadence is not what any
        // assertion here is about.
        rate_ticks: 1,
        hitscan: false,
        range_mm: 60_000,
        structure,
    };
    c.ammo[ARROW as usize] = AmmoDef {
        speed_mmpt: 1333,
        drop_mmpt2: 22,
    };
    c.ranged[GUN as usize] = RangedDef {
        damage: 20,
        ammo: [ROUND, NO_ITEM, NO_ITEM, NO_ITEM],
        rate_ticks: 1,
        hitscan: true,
        range_mm: 50_000,
        structure,
    };
    c
}

/// A world with content armed, a joined shooter, and a foundation + wall
/// standing at the returned cell.
///
/// **Takes `&mut World` and never returns one** — `event_roles.rs`'
/// measured rule: a `World` is ~440 kB and moving one out of a frame puts
/// two in a debug test thread's 2 MiB stack.
///
/// **The wall is placed from the shooting stance**, which is the trap this
/// suite would otherwise walk into: hard/soft v0 puts a placement's soft
/// side toward the placer, so building from inside the cell and shooting
/// from outside lands `HARD_SIDE_STRUCTURE` on every case and quietly turns
/// every damage assertion into a test of the side rule.
fn walled_world(w: &mut World, structure: u16) -> (u16, u16) {
    w.gather = GatherContent::probe_fixture();
    w.combat = shooter_combat(structure);
    w.build = BuildContent::probe_fixture();
    w.deploy = DeployContent::probe_fixture();
    w.tick(&[Command::Join { id: SHOOTER }]);
    let (cx, cz) = buildable_cell(SEED);
    seat(w, cx, cz, STANCE_CELLS);
    // Build materials in the low slots, the bow and its quiver above them.
    for (slot, item) in [(0usize, 0u16), (1, 1), (2, 2), (3, 4)] {
        w.players[0].inv[slot] = ItemStack {
            item,
            count: 200,
            cond: 0,
        };
    }
    // **The two weapons live in HOTBAR slots and their ammo does not.**
    // `sel` addresses the hotbar (`limits::HOTBAR_SLOTS` = 6) and
    // `world::apply` falls a wider `sel` back to 0 rather than refusing it,
    // so a gun parked at slot 6 is silently the club at slot 0 — which is
    // what the first draft of this suite did, and it read as the revolver
    // dealing the fixture club's 34. A round is taken by `inv_take` over
    // the whole inventory, so only the weapons need the bar.
    w.players[0].inv[SLOT_BOW] = ItemStack {
        item: BOW,
        count: 1,
        cond: 0,
    };
    w.players[0].inv[SLOT_GUN] = ItemStack {
        item: GUN,
        count: 1,
        cond: 0,
    };
    w.players[0].inv[6] = ItemStack {
        item: ARROW,
        count: 200,
        cond: 0,
    };
    w.players[0].inv[7] = ItemStack {
        item: ROUND,
        count: 200,
        cond: 0,
    };
    place(w, PIECE_FOUNDATION, cx, cz, GROUND, LOC_PLANE);
    place(w, PIECE_WALL, cx, cz, GROUND, LOC_EDGE_XLO);
    (cx, cz)
}

/// Stand the shooter `cells` build cells along -x from the centre of
/// (cx, cz). A negative `cells` puts them on the far side.
fn seat(w: &mut World, cx: u16, cz: u16, cells: f32) {
    let (x, z) = (
        (cx as f32 + 0.5 - cells) * BUILD_CELL_M,
        (cz as f32 + 0.5) * BUILD_CELL_M,
    );
    w.players[0].body = Body::at(SEED, hv(SEED), x, z);
}

fn place(w: &mut World, row: u16, cx: u16, cz: u16, level: u8, loc: u8) {
    let before = w.pieces.len();
    w.tick(&[Command::Place {
        id: SHOOTER,
        row,
        cx,
        cz,
        level,
        loc,
        freehand: false,
    }]);
    assert_eq!(
        w.pieces.len(),
        before + 1,
        "piece row {row} did not place at ({cx}, {cz}) level {level} loc {loc} \
         — the fixture, not the mechanic"
    );
}

/// Hold the trigger on `slot` at `yaw` until `code` lands, and return the
/// tick's whole event ring.
///
/// `World::tick` clears the ring at the top, so stopping *on* the tick the
/// code appeared is what makes the returned events that shot's and not a
/// sum over the loop.
fn shoot_until(w: &mut World, slot: u8, yaw: u16, code: u8) -> Vec<SimEvent> {
    let mut seq = 0u16;
    for _ in 0..MAX_STEPS {
        w.tick(&[Command::Input {
            id: SHOOTER,
            frame: InputFrame {
                seq,
                buttons: BTN_PRIMARY,
                yaw,
                // Level. The muzzle sits at `ARROW_EYE_MM` and the wall
                // spans the whole storey, so a flat shot meets its face.
                pitch: 128,
                sel: slot,
                ..InputFrame::default()
            },
        }]);
        seq = seq.wrapping_add(1);
        if w.events.entries().iter().any(|e| e.code == code) {
            return w.events.entries().to_vec();
        }
    }
    panic!("event code {code} never landed in {MAX_STEPS} ticks of shooting");
}

fn only(events: &[SimEvent], code: u8) -> SimEvent {
    let hits: Vec<_> = events.iter().filter(|e| e.code == code).collect();
    assert_eq!(
        hits.len(),
        1,
        "expected exactly one event code {code} on this tick, saw {}",
        hits.len()
    );
    *hits[0]
}

/// An arrow that stops on a wall takes the bow's `structure` off it, and
/// says so on the wire in `EV_STRUCT_HIT`'s own payload roles.
///
/// This is the whole slice in one case: the address the walk found, the
/// column the bake now carries, and the write `combat::raid` already owned.
#[test]
fn an_arrow_chips_the_wall_it_stops_on() {
    const S: u16 = 25;
    let mut w = World::new(SEED);
    let (cx, cz) = walled_world(&mut w, S);
    let events = shoot_until(&mut w, SLOT_BOW as u8, YAW_PLUS_X, EV_STRUCT_HIT);
    let h = only(&events, EV_STRUCT_HIT);

    assert_eq!(
        h.a,
        cell_key(cx, cz),
        "the struck cell is not the one the wall stands in"
    );
    assert_eq!(
        (h.b >> 16) as u8,
        GROUND,
        "the chip named level {} and the wall is on {GROUND}",
        h.b >> 16
    );
    assert_eq!(
        ((h.b >> 8) & 0xff) as u8,
        LOC_EDGE_XLO,
        "the chip named loc {} and the wall is at LOC_EDGE_XLO ({LOC_EDGE_XLO}) \
         — an arrow that chips the wrong face of a cell is indistinguishable \
         from one that works, until a player watches the wrong wall fall",
        (h.b >> 8) & 0xff
    );
    assert_eq!(
        h.c >> 16,
        S as u32,
        "the arrow dealt {} where the bow's structure column is {S}",
        h.c >> 16
    );
    assert_eq!(
        h.c & 0xffff,
        WALL_HP - S as u32,
        "the wall was left on {} of {WALL_HP}",
        h.c & 0xffff
    );
}

/// The shot still stops, and still draws its mark, on the tick it chips.
///
/// The mutant is a chip that REPLACES the impact — every damage assertion
/// above passes under it, and the player sees a wall lose hp with no arrow
/// stuck in it and no puff of splinters.
#[test]
fn the_chipping_shot_still_reports_where_it_landed() {
    let mut w = World::new(SEED);
    walled_world(&mut w, 25);
    let events = shoot_until(&mut w, SLOT_BOW as u8, YAW_PLUS_X, EV_STRUCT_HIT);
    let i = only(&events, EV_IMPACT);
    assert_eq!(
        (i.a >> 24) as u8,
        SURF_BUILT,
        "the tick that chipped a wall reported surface {} rather than \
         SURF_BUILT",
        i.a >> 24
    );
}

/// A shot on the wall's HARD face pays `HARD_SIDE_STRUCTURE`, exactly as a
/// swing does.
///
/// Sharing `combat::raid`'s law rather than restating it is the point: the
/// mutant is a shot that ignores the side, and it is invisible with shipped
/// numbers, because the shipped bow's `structure` is 1 and so is
/// `HARD_SIDE_STRUCTURE` — the two branches return the same value. This
/// case exists because a fixture can separate them and content cannot.
#[test]
fn a_shot_on_the_hard_face_pays_the_hard_side_price() {
    const S: u16 = 25;
    // A compile-time check, which is what it always wanted to be: if the
    // fixture's structure ever slid down to `HARD_SIDE_STRUCTURE` the two
    // branches would return the same number and this case would pass
    // without testing anything.
    const { assert!(S > HARD_SIDE_STRUCTURE) };
    let mut w = World::new(SEED);
    let (cx, cz) = walled_world(&mut w, S);
    // Around to the far side and turn back: the wall's soft side faces
    // where it was placed from, so this is its hard face.
    seat(&mut w, cx, cz, -STANCE_CELLS);
    let events = shoot_until(&mut w, SLOT_BOW as u8, YAW_MINUS_X, EV_STRUCT_HIT);
    let h = only(&events, EV_STRUCT_HIT);
    assert_eq!(
        h.c >> 16,
        HARD_SIDE_STRUCTURE as u32,
        "a shot on the hard face dealt {} where the side rule prices it at \
         {HARD_SIDE_STRUCTURE}",
        h.c >> 16
    );
}

/// Enough arrows bring the wall down, through the same `drop_piece` a swing
/// reaches.
///
/// The count is asserted as a window rather than a number: `damage_piece`
/// defers a kill when the tick's removal budget is spent, so "exactly four"
/// would be a claim about the budget and not about the arrow.
#[test]
fn enough_arrows_bring_the_wall_down() {
    const S: u16 = 25;
    let mut w = World::new(SEED);
    let (cx, cz) = walled_world(&mut w, S);
    let standing = w.pieces.len();
    let events = shoot_until(&mut w, SLOT_BOW as u8, YAW_PLUS_X, EV_PIECE_REMOVED);
    let r = only(&events, EV_PIECE_REMOVED);
    assert_eq!(
        r.a,
        cell_key(cx, cz),
        "the piece that fell is not the one that was shot"
    );
    assert_eq!(
        w.pieces.len(),
        standing - 1,
        "the wall left the store when it fell"
    );
}

/// A bullet chips it too, on the same address through the same write.
///
/// `hitscan` and `step` share `world_stop` precisely so a bullet and an
/// arrow cannot disagree about what a wall is; this is the assertion that
/// the sharing survived the slice.
#[test]
fn a_bullet_chips_the_same_wall_the_same_way() {
    const S: u16 = 25;
    let mut w = World::new(SEED);
    let (cx, cz) = walled_world(&mut w, S);
    let events = shoot_until(&mut w, SLOT_GUN as u8, YAW_PLUS_X, EV_STRUCT_HIT);
    let h = only(&events, EV_STRUCT_HIT);
    assert_eq!(h.a, cell_key(cx, cz));
    assert_eq!(
        ((h.b >> 8) & 0xff) as u8,
        LOC_EDGE_XLO,
        "the bullet chipped loc {} and the arrow chips LOC_EDGE_XLO",
        (h.b >> 8) & 0xff
    );
    assert_eq!(
        h.c >> 16,
        S as u32,
        "the revolver dealt {} where its structure column is {S}",
        h.c >> 16
    );
}

/// A weapon with no structure column stops on the wall and takes nothing
/// off it.
///
/// The mutant this kills is charging damage on every `SURF_BUILT` stop. It
/// is the one that would have shipped: the shot already knew it had hit a
/// piece, so "and therefore damage it" reads as the obvious line.
#[test]
fn a_shot_with_no_structure_column_leaves_the_wall_whole() {
    let mut w = World::new(SEED);
    walled_world(&mut w, 0);
    let events = shoot_until(&mut w, SLOT_BOW as u8, YAW_PLUS_X, EV_IMPACT);
    assert_eq!(
        (only(&events, EV_IMPACT).a >> 24) as u8,
        SURF_BUILT,
        "the shot must still be stopped by the wall"
    );
    assert!(
        !events.iter().any(|e| e.code == EV_STRUCT_HIT),
        "a weapon with structure 0 announced a struct hit"
    );
    let hp: u32 = w.pieces.entries()[..w.pieces.len()]
        .iter()
        .map(|p| p.hp as u32)
        .sum();
    assert_eq!(
        hp,
        WALL_HP * 2,
        "the foundation and the wall are both whole at {WALL_HP} each"
    );
}

// --- The deployable half. ---------------------------------------------------
//
// Everything above charges a *piece*. A solid deployable is the other half of
// what a base is made of and it sat in a walk the shot path never called, so
// an arrow flew through a furnace, a box and a bench (`NOW.md` §0mk item 2).
// `tests/shoot.rs` gates the address; this gates that the write lands in the
// **other store** — which is the part no collision fixture can see, because
// both stores answer to one four-part address by design.

/// `DeployContent::probe_fixture`'s row 1: a workbench, `PLACE_ANY`, hp 80,
/// its own item 3. Solid — `DEPLOY_VOL[ARCH_WORKBENCH]` is 1.6 x 0.9 x 0.7.
const DEPLOY_BENCH: u16 = 1;
/// Its hp, as the fixture rows it.
const BENCH_HP: u32 = 80;

/// `walled_world`, plus a workbench on the foundation and the shooter
/// standing over it firing straight down.
///
/// **Straight down, from inside the cell**, which is
/// `shoot.rs`' `a_floor_stops_a_shot_fired_down_through_it` stance and is
/// forced rather than chosen: the muzzle sits at `ARROW_EYE_MM` (1.6 m) and
/// the tallest solid deployable in the table is 1.15 m, so a LEVEL shot —
/// every other case in this suite — flies over every bench in the game and
/// would test nothing at all while reporting a clean miss.
///
/// The body is seated on the foundation's own top rather than on the dirt,
/// because the whole geometry is measured from that plane and the terrain
/// under it is a band away. Standing inside the bench is legal and is not an
/// accident of the fixture: `movement.rs` says in its own comment that a
/// deploy can be placed around a standing body, and the escape latch is what
/// lets them walk out.
fn benched_world(w: &mut World, structure: u16) -> (u16, u16, f32) {
    let (cx, cz) = walled_world(w, structure);
    // The bench's **own item** is its cost, consumed whole (`place_deploy`
    // — a deployable is not crafted at the spot). Row 1's is item 3, which
    // `walled_world` does not stock: its low slots are the build materials
    // and slot 3 is the door's item 4.
    w.players[0].inv[8] = ItemStack {
        item: 3,
        count: 4,
        cond: 0,
    };
    let before = w.deploys.len();
    w.tick(&[Command::PlaceDeploy {
        id: SHOOTER,
        row: DEPLOY_BENCH,
        cx,
        cz,
        level: GROUND,
        loc: LOC_PLANE,
    }]);
    assert_eq!(
        w.deploys.len(),
        before + 1,
        "the workbench did not place at ({cx}, {cz}) — refusal {:?}, and this \
         is the fixture, not the mechanic",
        w.events
            .entries()
            .iter()
            .find(|e| e.code == sim_core::world::EV_DEPLOY_REFUSED)
            .map(|e| e.b)
    );
    // Stand on the foundation the bench is bolted to. `col_base_y` is not
    // public, so the fixture asks the same question `build` answers for it:
    // the column's floor under whatever plate the foundation latched.
    let top = sim_core::build::column_floor_y(
        SEED,
        hv(SEED),
        cx,
        cz,
        w.pieces.cols().plate(cx, cz).unwrap_or(0),
    );
    let (x, z) = (
        (cx as f32 + 0.5) * BUILD_CELL_M,
        (cz as f32 + 0.5) * BUILD_CELL_M,
    );
    w.players[0].body = Body::at(SEED, hv(SEED), x, z);
    w.players[0].body.qy = quant_y(top);
    (cx, cz, top)
}

/// Fire straight down until `code` lands. `shoot_until`'s twin at pitch 0 —
/// see `benched_world` for why a level shot cannot reach a bench.
fn shoot_down_until(w: &mut World, slot: u8, code: u8) -> Vec<SimEvent> {
    let mut seq = 0u16;
    for _ in 0..MAX_STEPS {
        w.tick(&[Command::Input {
            id: SHOOTER,
            frame: InputFrame {
                seq,
                buttons: BTN_PRIMARY,
                yaw: YAW_PLUS_X,
                pitch: 0,
                sel: slot,
                ..InputFrame::default()
            },
        }]);
        seq = seq.wrapping_add(1);
        if w.events.entries().iter().any(|e| e.code == code) {
            return w.events.entries().to_vec();
        }
    }
    panic!("event code {code} never landed in {MAX_STEPS} ticks of shooting down");
}

/// An arrow that stops on a workbench takes the bow's `structure` off **the
/// bench**, and says so with `STRUCT_DEPLOY_BIT` set.
///
/// The bit is the assertion. Without it the same payload names a piece at the
/// same address — which is a real address, because the foundation the bench
/// stands on holds it — so a client would draw the damage band on the
/// foundation and the shard would have taken hp off neither.
#[test]
fn an_arrow_chips_the_workbench_it_stops_on() {
    let mut w = Box::new(World::new(SEED));
    let (cx, cz, top) = benched_world(&mut w, 20);
    assert_eq!(
        w.deploys.entries()[0].hp as u32,
        BENCH_HP,
        "the fixture bench must start at its rowed hp"
    );

    let ev = shoot_down_until(&mut w, SLOT_BOW as u8, EV_STRUCT_HIT);
    let hit = only(&ev, EV_STRUCT_HIT);
    assert_eq!(
        hit.a,
        cell_key(cx, cz),
        "the hit names build cell ({cx}, {cz})"
    );
    assert_ne!(
        hit.b & sim_core::world::STRUCT_DEPLOY_BIT,
        0,
        "the shot charged the PIECE store at ({cx}, {cz}) — the foundation \
         holds that same address, so nothing about the payload looks wrong \
         except which thing lost hp"
    );
    assert_eq!(
        hit.b & !sim_core::world::STRUCT_DEPLOY_BIT,
        ((GROUND as u32) << 16) | ((LOC_PLANE as u32) << 8) | DEPLOY_BENCH as u32,
        "…and the rest of the payload is the bench's own address and row"
    );
    assert_eq!(
        hit.c,
        (20u32 << 16) | (BENCH_HP - 20),
        "20 dealt, {} left",
        BENCH_HP - 20
    );
    assert_eq!(
        w.deploys.entries()[0].hp as u32,
        BENCH_HP - 20,
        "the store must agree with the wire"
    );

    // The foundation and the wall are whole: a chip routed to the piece
    // store would have landed on one of them, and both are in this cell.
    for i in 0..w.pieces.len() {
        let rec = w.pieces.entries()[i];
        assert_eq!(
            rec.hp as u32, WALL_HP,
            "piece {i} at ({}, {}) loc {} lost hp to a shot aimed at the bench",
            rec.cx, rec.cz, rec.loc
        );
    }

    // The mark is still drawn, and still says "built".
    let imp = only(&ev, EV_IMPACT);
    assert_eq!(
        (imp.a >> 24) as u8,
        SURF_BUILT,
        "a shot that stopped on a bench reported a surface that is not built"
    );
    let y = imp.c as i32 as f32 * sim_core::movement::POS_Y_Q;
    assert!(
        y > top,
        "the impact landed at y={y:.2}, at or under the foundation top \
         {top:.2} — the arrow went through the bench and marked the slab"
    );
}

/// Enough arrows bring the bench down, and the removal is announced.
///
/// `damage_deploy` has no removal budget and `drop_deploy` collapses
/// nothing, which is exactly why this case is here: the piece path's
/// budget floor (hp parks at 1 when the tick's allowance is spent) has no
/// analogue, so a bench must actually reach zero and leave the store.
#[test]
fn enough_arrows_bring_the_workbench_down() {
    let mut w = Box::new(World::new(SEED));
    let (cx, cz, _) = benched_world(&mut w, 20);
    let ev = shoot_down_until(&mut w, SLOT_BOW as u8, EV_DEPLOY_REMOVED);
    let gone = only(&ev, EV_DEPLOY_REMOVED);
    assert_eq!(gone.a, cell_key(cx, cz), "the bench fell in its own cell");
    assert_eq!(
        gone.b,
        ((GROUND as u32) << 16) | ((LOC_PLANE as u32) << 8) | DEPLOY_BENCH as u32,
        "…at its own address and row"
    );
    assert_eq!(w.deploys.len(), 0, "the bench is out of the store");
    assert!(
        w.pieces.len() >= 2,
        "the foundation and the wall must still stand — the bench is what fell"
    );
}

/// A bullet reaches a bench the same way an arrow does.
///
/// The two passes share `world_stop` and therefore share the new rung, and
/// this is the case that proves the sharing rather than asserting it in a
/// comment — `hitscan` walks its own `upto` and mints its own `Chip`.
#[test]
fn a_bullet_chips_the_same_bench_the_same_way() {
    let mut w = Box::new(World::new(SEED));
    benched_world(&mut w, 20);
    let ev = shoot_down_until(&mut w, SLOT_GUN as u8, EV_STRUCT_HIT);
    let hit = only(&ev, EV_STRUCT_HIT);
    assert_ne!(
        hit.b & sim_core::world::STRUCT_DEPLOY_BIT,
        0,
        "the revolver's round charged the piece store"
    );
    assert_eq!(
        w.deploys.entries()[0].hp as u32,
        BENCH_HP - 20,
        "a bullet takes the same 20 off the bench an arrow does"
    );
}

/// A shot fired past the bench, inside its own cell, leaves it whole — and
/// lands on the foundation under it instead.
///
/// The mutant: stop on the solid nibble and never measure the box. Every
/// case above fires down the cell's exact centre and passes under it.
///
/// **The pair is what makes it evidence.** "No `EV_STRUCT_HIT`" would be a
/// weaker claim than it looks, satisfied by an arrow that never flew; the
/// same shot must still charge something, and what it charges is the slab
/// the bench is bolted to — one `STRUCT_DEPLOY_BIT` away from the case
/// above it, over identical geometry, at the same address.
#[test]
fn a_shot_past_the_workbench_leaves_it_whole() {
    let mut w = Box::new(World::new(SEED));
    let (cx, cz, top) = benched_world(&mut w, 20);
    let (hw, _, hd) = sim_core::deploy::solid_vol(sim_core::deploy::ARCH_WORKBENCH)
        .expect("the workbench is a solid archetype");
    // Inside the build cell, outside the bench on both axes at once — so a
    // mutant that dropped either clamp is still caught.
    let off = 1.2f32;
    assert!(
        off > hw && off > hd && off < BUILD_CELL_M * 0.5,
        "the fixture offset must clear the bench ({hw:.2}, {hd:.2}) and stay \
         inside the cell"
    );
    let (x, z) = (
        (cx as f32 + 0.5) * BUILD_CELL_M + off,
        (cz as f32 + 0.5) * BUILD_CELL_M + off,
    );
    w.players[0].body = Body::at(SEED, hv(SEED), x, z);
    w.players[0].body.qy = quant_y(top);

    let ev = shoot_down_until(&mut w, SLOT_BOW as u8, EV_STRUCT_HIT);
    let hit = only(&ev, EV_STRUCT_HIT);
    assert_eq!(
        hit.b & sim_core::world::STRUCT_DEPLOY_BIT,
        0,
        "a shot fired {off:.1} m off the cell centre charged a bench whose \
         half-extents are ({hw:.2}, {hd:.2})"
    );
    assert_eq!(
        hit.a,
        cell_key(cx, cz),
        "…and what it did charge is in the bench's own cell: the foundation"
    );
    assert_eq!(
        w.deploys.entries()[0].hp as u32,
        BENCH_HP,
        "the bench lost hp to a shot that missed it"
    );
}

/// `BuildContent::probe_fixture`'s row 2: a floor, for the upper storey.
const PIECE_FLOOR: u16 = 2;

/// Two benches in one column, one storey apart, and a shot from above
/// charges the **upper** one.
///
/// **This is the case the level-0 fixtures could not make.** Every other
/// deployable case here stands its bench at level 0, so a `World::chip` that
/// threw the chip's level away and looked up level 0 was invisible to all of
/// them — a mutant that survived the first round, and the same shape as the
/// finding ranged structure damage v0 filed about `shoot.rs`' all-`LOC_PLANE`
/// fixtures. A base with a box downstairs and a furnace upstairs is not an
/// exotic geometry; it is the ordinary one, and under that mutant shooting
/// the furnace takes hp off the box.
#[test]
fn a_shot_from_above_charges_the_upper_bench_not_the_lower() {
    let mut w = Box::new(World::new(SEED));
    let (cx, cz, top) = benched_world(&mut w, 20);
    // A floor over the cell, and a second bench standing on it.
    place(&mut w, PIECE_FLOOR, cx, cz, 1, LOC_PLANE);
    let before = w.deploys.len();
    w.tick(&[Command::PlaceDeploy {
        id: SHOOTER,
        row: DEPLOY_BENCH,
        cx,
        cz,
        level: 1,
        loc: LOC_PLANE,
    }]);
    assert_eq!(
        w.deploys.len(),
        before + 1,
        "the upper workbench did not place — refusal {:?}, and this is the \
         fixture, not the mechanic",
        w.events
            .entries()
            .iter()
            .find(|e| e.code == sim_core::world::EV_DEPLOY_REFUSED)
            .map(|e| e.b)
    );

    // Stand on the upper floor and fire down its own storey.
    w.players[0].body.qy = quant_y(top + LEVEL_H_M);
    let ev = shoot_down_until(&mut w, SLOT_BOW as u8, EV_STRUCT_HIT);
    let hit = only(&ev, EV_STRUCT_HIT);
    assert_ne!(
        hit.b & sim_core::world::STRUCT_DEPLOY_BIT,
        0,
        "the shot charged the piece store"
    );
    assert_eq!(
        (hit.b & !sim_core::world::STRUCT_DEPLOY_BIT) >> 16,
        1,
        "the shot charged level {} — the bench it stopped on is on level 1",
        (hit.b & !sim_core::world::STRUCT_DEPLOY_BIT) >> 16
    );
    // The stores, not just the wire: which record actually lost hp is the
    // whole question, and both benches are the same row at the same loc.
    let lower = w
        .deploys
        .entries()
        .iter()
        .take(w.deploys.len())
        .find(|d| d.level == 0)
        .expect("the lower bench is still in the store");
    let upper = w
        .deploys
        .entries()
        .iter()
        .take(w.deploys.len())
        .find(|d| d.level == 1)
        .expect("the upper bench is still in the store");
    assert_eq!(
        upper.hp as u32,
        BENCH_HP - 20,
        "the upper bench took the shot and must be down 20"
    );
    assert_eq!(
        lower.hp as u32, BENCH_HP,
        "the bench a storey below the shot lost hp"
    );
}
