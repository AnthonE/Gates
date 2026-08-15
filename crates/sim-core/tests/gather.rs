//! Gather behavior gates (M1): a player beside a known node swings and is
//! paid per the baked table; the node exhausts, respawns inside its
//! window, and every path replays bit-identically. Uses the synthetic
//! probe fixture — content correctness for the real TOML set is the
//! content crate's bake tests.

use sim_core::gather::{
    cell_key, weak_mark8, GatherContent, ItemStack, NO_CELL, RESPAWN_MIN_TICKS,
    RESPAWN_RANGE_TICKS, SWING_INTERVAL_TICKS,
};
use sim_core::input::{InputFrame, BTN_PRIMARY};
use sim_core::movement;
use sim_core::terrain::{self, Occupant, ScatterTable, CELL_SIZE};
use sim_core::world::{
    Command, World, EV_GATHER, EV_SLOT_HARVESTED, EV_SLOT_RESPAWNED, EV_WEAK_MARK,
};
use sim_core::yaw_dir;

/// Arbitrary fixed seed; the tests derive everything else from it.
const SEED: u64 = 20_260_731;

/// A gatherable slot, the walkable point 1.2 m west of it, and the yaw
/// (of 256 headings) that best faces it. Panics if the seed offers no
/// sufficiently isolated node — that is a test-setup failure, not a skip.
fn find_isolated(seed: u64, want: Occupant) -> ((f32, f32), u16, (i32, i32)) {
    let table = ScatterTable::alpha_default();
    let haven = terrain::haven(seed);
    for cz in 40..216i32 {
        for cx in 40..216i32 {
            let s = terrain::scatter(seed, &table, &haven, cx, cz);
            if s.occupant != want {
                continue;
            }
            let (px, pz) = (s.x - 1.2, s.z);
            let py = terrain::height(seed, px, pz);
            if (s.y - py).max(py - s.y) > 1.0 || py < 1.0 {
                continue; // node on a ledge or in the sea — keep looking
            }
            // The node must be the only gatherable within reach+slack of
            // the stand point, so the swing target is unambiguous.
            let pcx = (px / CELL_SIZE) as i32;
            let pcz = (pz / CELL_SIZE) as i32;
            let mut rivals = 0;
            for dz in -1..=1i32 {
                for dx in -1..=1i32 {
                    let n = terrain::scatter(seed, &table, &haven, pcx + dx, pcz + dz);
                    if sim_core::gather::node_index(n.occupant).is_some() {
                        let d2 = (n.x - px) * (n.x - px) + (n.z - pz) * (n.z - pz);
                        if d2 <= 6.25 && (n.x != s.x || n.z != s.z) {
                            rivals += 1;
                        }
                    }
                }
            }
            if rivals > 0 {
                continue;
            }
            // Best of the 256 LUT headings toward the node.
            let (dx, dz) = (s.x - px, s.z - pz);
            let mut best_yaw = 0u16;
            let mut best_dot = f32::MIN;
            for hi in 0..=255u16 {
                let yaw = hi << 8;
                let (fx, fz) = yaw_dir(yaw);
                let dot = fx * dx + fz * dz;
                if dot > best_dot {
                    best_dot = dot;
                    best_yaw = yaw;
                }
            }
            return ((px, pz), best_yaw, (cx, cz));
        }
    }
    panic!("seed {seed:#x} offered no isolated {want:?} in the scanned block");
}

fn hold_primary(yaw: u16, seq: u16) -> Command {
    Command::Input {
        id: 1,
        frame: sim_core::input::InputFrame {
            seq,
            buttons: BTN_PRIMARY,
            yaw,
            pitch: 0,
            move_x: 0,
            move_z: 0,
            sel: 0,
        },
    }
}

fn stand_still(yaw: u16, seq: u16) -> Command {
    Command::Input {
        id: 1,
        frame: sim_core::input::InputFrame {
            seq,
            buttons: 0,
            yaw,
            pitch: 0,
            move_x: 0,
            move_z: 0,
            sel: 0,
        },
    }
}

/// Build a world with player 1 standing at `pos`, gather fixture armed.
/// Boxed: a `World` is several hundred KB, and by-value copies through
/// test frames overflow default test-thread stacks (the same reason the
/// piece store boxes its column index).
fn world_at(pos: (f32, f32)) -> Box<World> {
    let mut w = Box::new(World::new(SEED));
    w.gather = GatherContent::probe_fixture();
    w.dev_spawn = Some(pos);
    w.tick(&[Command::Join { id: 1 }]);
    w
}

#[test]
fn swing_pays_exhausts_and_replays() {
    let (pos, yaw, (cx, cz)) = find_isolated(SEED, Occupant::Tree);
    let fixture = GatherContent::probe_fixture();
    let tree = &fixture.nodes[0];
    let run = || {
        let mut w = world_at(pos);
        // This test pins the BASE pay path: every swing must pay exactly
        // the hand row, so the weak-spot bonus (its own tests below) is
        // disabled — a stationary swinger would otherwise land in the
        // roaming mark's sector on some seeds and double a payout.
        for n in w.gather.nodes.iter_mut() {
            n.weak_pct = 0;
        }
        let mut gathers = 0u32;
        let mut harvested = 0u32;
        // Enough ticks for every exhausting swing at the fixed cadence.
        let ticks = SWING_INTERVAL_TICKS * (tree.hits as u64 + 1);
        for t in 0..ticks {
            w.tick(&[hold_primary(yaw, t as u16)]);
            for e in w.events.entries() {
                match e.code {
                    EV_GATHER => {
                        assert_eq!(e.a, 1, "gather credited to the swinger");
                        assert_eq!(
                            e.b,
                            ((tree.output as u32) << 16) | tree.hand_yield as u32,
                            "bare-hand swing pays the hand row"
                        );
                        gathers += 1;
                    }
                    EV_SLOT_HARVESTED => {
                        assert_eq!(e.a, ((cx as u32) << 16) | cz as u32);
                        harvested += 1;
                    }
                    _ => {}
                }
            }
        }
        (w.state_hash(), gathers, harvested, w.players[0].inv)
    };

    let (hash_a, gathers, harvested, inv) = run();
    assert_eq!(
        gathers, tree.hits as u32,
        "the node pays exactly its hit count, then stops"
    );
    assert_eq!(harvested, 1, "exhaustion announces itself exactly once");
    assert_eq!(
        inv[0],
        ItemStack {
            item: tree.output,
            count: tree.hits * tree.hand_yield,
        },
        "yield stacked into the first slot"
    );

    let (hash_b, ..) = run();
    assert_eq!(hash_a, hash_b, "gather must replay bit-identically");
}

/// The bush pays two things, and the second one is the only reason the
/// survival clock has an answer. Both land in the inventory, both announce
/// themselves, and the side payout is **flat** — the weak-spot bonus and
/// the tool ladder move the primary and leave it alone.
#[test]
fn a_bush_pays_its_side_yield_flat_and_says_so() {
    let (pos, yaw, _) = find_isolated(SEED, Occupant::Bush);
    let fixture = GatherContent::probe_fixture();
    let bush = &fixture.nodes[4];
    let (sec_item, sec_pay) = bush.secondary;
    assert_ne!(
        sec_item, bush.output,
        "the fixture's bush must pay two different items or this proves nothing"
    );

    let mut w = world_at(pos);
    // A tool in hand, and the mark left armed: neither may reach the side
    // payout. The tool row is the primary's (item 0 gathers item 1 faster),
    // so holding it is the sharpest version of this — if the secondary read
    // `yield_for` it would move here.
    w.players[0].inv[0] = ItemStack {
        item: fixture.nodes[0].tools[0].0,
        count: 1,
    };
    w.tick(&[hold_primary(yaw, 0)]);

    let mut primary = 0u32;
    let mut side = 0u32;
    for e in w.events.entries() {
        if e.code == EV_GATHER {
            let (item, amount) = ((e.b >> 16) as u16, e.b & 0xFFFF);
            if item == bush.output {
                primary += 1;
            } else if item == sec_item {
                assert_eq!(
                    amount, sec_pay as u32,
                    "the side payout is flat — no tool row, no weak-spot bonus"
                );
                side += 1;
            }
        }
    }
    assert_eq!(primary, 1, "one primary payout for one landed swing");
    assert_eq!(side, 1, "one side payout for one landed swing, announced");

    let held: u16 = w.players[0]
        .inv
        .iter()
        .filter(|s| s.item == sec_item && s.count > 0)
        .map(|s| s.count)
        .sum();
    assert_eq!(held, sec_pay, "the side yield reached the inventory");
}

/// A node with no `secondary` pays once, which is every other archetype.
/// Without this the test above would pass on a payout that fires for
/// everything, and every tree in the world would rain berries.
#[test]
fn a_tree_pays_once() {
    let (pos, yaw, _) = find_isolated(SEED, Occupant::Tree);
    let mut w = world_at(pos);
    w.tick(&[hold_primary(yaw, 0)]);
    let gathers = w
        .events
        .entries()
        .iter()
        .filter(|e| e.code == EV_GATHER)
        .count();
    assert_eq!(gathers, 1, "a node with no secondary pays exactly once");
}

#[test]
fn harvested_node_respawns_inside_the_window() {
    let (pos, yaw, (cx, cz)) = find_isolated(SEED, Occupant::Tree);
    let mut w = world_at(pos);
    let hits = w.gather.nodes[0].hits as u64;
    for t in 0..SWING_INTERVAL_TICKS * hits {
        w.tick(&[hold_primary(yaw, t as u16)]);
    }
    assert!(
        w.slot_lives.is_harvested(cx as u16, cz as u16),
        "node should be down after {hits} swings"
    );
    let harvested_at = w.tick;

    // Stand idle until it comes back; the window bounds the wait.
    w.tick(&[stand_still(yaw, 0)]);
    let mut respawn_seen = false;
    while w.tick - harvested_at <= RESPAWN_MIN_TICKS + RESPAWN_RANGE_TICKS {
        w.tick(&[]);
        if !w.events.is_empty() {
            assert_eq!(w.events.entries()[0].code, EV_SLOT_RESPAWNED);
            respawn_seen = true;
            break;
        }
    }
    assert!(respawn_seen, "node never respawned inside the window");
    let waited = w.tick - harvested_at;
    assert!(
        waited >= RESPAWN_MIN_TICKS,
        "respawned after {waited} ticks — under the 20-min floor"
    );
    assert!(w.slot_lives.is_empty(), "released entry leaves the store");
}

#[test]
fn tool_in_slot0_outyields_hand() {
    let (pos, yaw, _) = find_isolated(SEED, Occupant::Tree);
    let mut w = world_at(pos);
    let (tool, per_hit) = w.gather.nodes[0].tools[0];
    w.players[0].inv[0] = ItemStack {
        item: tool,
        count: 1,
    };
    w.tick(&[hold_primary(yaw, 0)]);
    let out = w.gather.nodes[0].output;
    assert_eq!(
        w.players[0].inv[1],
        ItemStack {
            item: out,
            count: per_hit,
        },
        "held tool pays its row, stacked past the occupied slot 0"
    );
}

#[test]
fn cone_refuses_a_node_behind_you() {
    let (pos, yaw, (cx, cz)) = find_isolated(SEED, Occupant::Tree);
    let away = yaw.wrapping_add(0x8000);
    let mut w = world_at(pos);
    for t in 0..SWING_INTERVAL_TICKS * 2 {
        w.tick(&[hold_primary(away, t as u16)]);
    }
    assert_eq!(w.players[0].inv[0], ItemStack::default(), "no yield");
    assert!(w.slot_lives.find(cx as u16, cz as u16).is_none());
    assert!(
        w.players[0].next_swing > 0,
        "the whiff still paid its cooldown"
    );
}

#[test]
fn cadence_gates_to_one_swing_per_interval() {
    let (pos, yaw, _) = find_isolated(SEED, Occupant::Tree);
    let mut w = world_at(pos);
    let hand = w.gather.nodes[0].hand_yield;
    for t in 0..SWING_INTERVAL_TICKS {
        // Every tick inside one interval holds the button…
        w.tick(&[hold_primary(yaw, t as u16)]);
    }
    // …and exactly one swing lands.
    assert_eq!(w.players[0].inv[0].count, hand);
}

/// Best of the 256 LUT headings toward planar offset (dx, dz).
fn facing_yaw(dx: f32, dz: f32) -> u16 {
    let mut best_yaw = 0u16;
    let mut best_dot = f32::MIN;
    for hi in 0..=255u16 {
        let yaw = hi << 8;
        let (fx, fz) = yaw_dir(yaw);
        let dot = fx * dx + fz * dz;
        if dot > best_dot {
            best_dot = dot;
            best_yaw = yaw;
        }
    }
    best_yaw
}

/// Teleport player 0 to 1.2 m out from the node at `(nx, ny, nz)` along
/// heading `bearing8`, and return the yaw that faces the node from there.
fn stand_at_bearing(w: &mut World, nx: f32, ny: f32, nz: f32, bearing8: u8) -> u16 {
    let (bx, bz) = yaw_dir((bearing8 as u16) << 8);
    let px = nx + bx * 1.2;
    let pz = nz + bz * 1.2;
    w.players[0].body.qx = movement::quant_xz(px);
    w.players[0].body.qz = movement::quant_xz(pz);
    w.players[0].body.qy = movement::quant_y(ny);
    facing_yaw(nx - px, nz - pz)
}

#[test]
fn weak_spot_pays_only_inside_the_marks_sector() {
    let (pos, yaw, (cx, cz)) = find_isolated(SEED, Occupant::Tree);
    let (cxu, czu) = (cx as u16, cz as u16);
    let table = ScatterTable::alpha_default();
    let haven = terrain::haven(SEED);
    let s = terrain::scatter(SEED, &table, &haven, cx, cz);
    let def = GatherContent::probe_fixture().nodes[0];
    assert!(def.weak_pct > 0, "fixture tree must carry a weak bonus");

    // Hit 1 from the scan's stand point: never a bonus, starts the chase,
    // and announces mark 1 to the swinger.
    let mut w = world_at(pos);
    w.tick(&[hold_primary(yaw, 0)]);
    assert_eq!(w.players[0].inv[0].count, def.hand_yield, "hit 1 is base");
    assert_eq!(w.players[0].ws_cell, cell_key(cxu, czu));
    assert_eq!(w.players[0].ws_hits, 1);
    let mark1 = weak_mark8(SEED, cxu, czu, 1, 1);
    let marks: u32 = w
        .events
        .entries()
        .iter()
        .filter(|e| e.code == EV_WEAK_MARK)
        .map(|e| {
            assert_eq!((e.a, e.b), (1, cell_key(cxu, czu)));
            assert_eq!(e.c, mark1 as u32, "mark 1, weak bit clear");
            1
        })
        .sum();
    assert_eq!(marks, 1, "a landed non-exhausting hit announces the mark");

    // Stand inside mark 1's sector: hit 2 pays hand × (100 + pct) / 100,
    // the weak bit rides, and the mark moves to n = 2.
    let face = stand_at_bearing(&mut w, s.x, s.y, s.z, mark1);
    for t in 0..SWING_INTERVAL_TICKS {
        w.tick(&[hold_primary(face, t as u16 + 1)]);
    }
    let bonus_pay = (def.hand_yield as u32 * (100 + def.weak_pct as u32) / 100) as u16;
    assert!(bonus_pay > def.hand_yield, "fixture bonus must be visible");
    assert_eq!(
        w.players[0].inv[0].count,
        def.hand_yield + bonus_pay,
        "aligned hit 2 pays the weak bonus"
    );
    let mark2 = weak_mark8(SEED, cxu, czu, 1, 2);
    let weak_marks: u32 = w
        .events
        .entries()
        .iter()
        .filter(|e| e.code == EV_WEAK_MARK)
        .map(|e| {
            assert_eq!(e.c, (1 << 8) | mark2 as u32, "weak bit set, mark moved");
            1
        })
        .sum();
    assert_eq!(weak_marks, 1);

    // Same second hit stood opposite the mark: base pay, weak bit clear.
    let mut w2 = world_at(pos);
    w2.tick(&[hold_primary(yaw, 0)]);
    let face2 = stand_at_bearing(&mut w2, s.x, s.y, s.z, mark1.wrapping_add(128));
    for t in 0..SWING_INTERVAL_TICKS {
        w2.tick(&[hold_primary(face2, t as u16 + 1)]);
    }
    assert_eq!(
        w2.players[0].inv[0].count,
        def.hand_yield * 2,
        "misaligned hit 2 pays base"
    );
    for e in w2.events.entries() {
        if e.code == EV_WEAK_MARK {
            assert_eq!(e.c, mark2 as u32, "weak bit clear when misaligned");
        }
    }
}

#[test]
fn exhaustion_clears_the_chase_and_mutes_the_mark() {
    let (pos, yaw, _) = find_isolated(SEED, Occupant::Tree);
    let mut w = world_at(pos);
    let hits = w.gather.nodes[0].hits;
    let mut marks_seen = 0u32;
    let mut landed = 0u32;
    let mut harvested_tick_had_mark = false;
    let mut saw_harvest = false;
    'outer: for t in 0..SWING_INTERVAL_TICKS * (hits as u64 + 1) {
        w.tick(&[hold_primary(yaw, t as u16)]);
        let mut tick_harvest = false;
        let mut tick_mark = false;
        let mut tick_gather = false;
        for e in w.events.entries() {
            match e.code {
                EV_WEAK_MARK => tick_mark = true,
                EV_SLOT_HARVESTED => tick_harvest = true,
                EV_GATHER => tick_gather = true,
                _ => {}
            }
        }
        if tick_gather || tick_harvest {
            landed += 1;
        }
        if tick_mark {
            marks_seen += 1;
        }
        if tick_harvest {
            saw_harvest = true;
            harvested_tick_had_mark = tick_mark;
            break 'outer;
        }
    }
    assert!(saw_harvest, "the node never exhausted");
    assert!(
        !harvested_tick_had_mark,
        "the exhausting hit must not announce a mark for a vanished node"
    );
    // Counted against the swings that actually LANDED, not against
    // `hits`: since the mark buys speed (2026-08-09) a marked swing
    // spends more than one hit's budget, so the swing count depends on
    // how often the swinger happens to stand in the sector — and this
    // walker stands still, which stumbles into it about a quarter of the
    // time. `hits` is the node's budget, no longer its swing count.
    assert_eq!(
        marks_seen,
        landed - 1,
        "every landed hit but the exhausting one announces a mark"
    );
    assert!(
        landed <= hits as u32,
        "a swing can never spend less than one hit's budget"
    );
    assert_eq!(w.players[0].ws_cell, NO_CELL, "chase cleared");
    assert_eq!(w.players[0].ws_hits, 0);
}

#[test]
fn selected_slot_is_the_held_item_and_invalid_sel_falls_back() {
    let (pos, yaw, _) = find_isolated(SEED, Occupant::Tree);
    let hold_sel = |sel: u8| Command::Input {
        id: 1,
        frame: InputFrame {
            seq: 0,
            buttons: BTN_PRIMARY,
            yaw,
            pitch: 0,
            move_x: 0,
            move_z: 0,
            sel,
        },
    };

    // The tool sits in hotbar slot 3; selecting it swings its row.
    let mut w = world_at(pos);
    let (tool, per_hit) = w.gather.nodes[0].tools[0];
    let out = w.gather.nodes[0].output;
    w.players[0].inv[3] = ItemStack {
        item: tool,
        count: 1,
    };
    w.tick(&[hold_sel(3)]);
    assert_eq!(
        w.players[0].inv[0],
        ItemStack {
            item: out,
            count: per_hit,
        },
        "held tool in the selected slot pays its row"
    );

    // An out-of-range selector falls back to slot 0 — the bare hand here.
    let mut w2 = world_at(pos);
    w2.players[0].inv[3] = ItemStack {
        item: tool,
        count: 1,
    };
    w2.tick(&[hold_sel(7)]);
    assert_eq!(w2.players[0].frame.sel, 0, "invalid selector clamps to 0");
    assert_eq!(
        w2.players[0].inv[0],
        ItemStack {
            item: out,
            count: w2.gather.nodes[0].hand_yield,
        },
        "fallback swings the hand row"
    );
}

#[test]
fn inert_content_gathers_nothing() {
    let (pos, yaw, _) = find_isolated(SEED, Occupant::Tree);
    let mut w = Box::new(World::new(SEED)); // no fixture: GatherContent::EMPTY
    w.dev_spawn = Some(pos);
    w.tick(&[Command::Join { id: 1 }]);
    for t in 0..SWING_INTERVAL_TICKS * 2 {
        w.tick(&[hold_primary(yaw, t as u16)]);
    }
    assert_eq!(w.players[0].inv[0], ItemStack::default());
    assert!(w.slot_lives.is_empty());
}

/// Strike an isolated node to exhaustion, re-standing before every swing
/// either INSIDE the current mark's sector (`chase`, what a skilled
/// player does) or directly opposite it (what a player who never finds
/// it does). Returns (swings landed, total primary yield).
///
/// The unchased run must *avoid* the mark rather than merely ignore it:
/// the sector is 90° wide, so a fixed stand point lands in it about a
/// quarter of the time by luck, and the first cut of this helper
/// measured 3 swings for both runs because the "plain" walker kept
/// stumbling onto the mark.
fn strike_out(occ: Occupant, chase: bool) -> (u32, u32) {
    let (pos, yaw, (cx, cz)) = find_isolated(SEED, occ);
    let (cxu, czu) = (cx as u16, cz as u16);
    let table = ScatterTable::alpha_default();
    let haven = terrain::haven(SEED);
    let s = terrain::scatter(SEED, &table, &haven, cx, cz);
    let mut w = world_at(pos);
    let mut face = yaw;
    let (mut swings, mut seq) = (0u32, 0u16);
    // The first swing lands immediately; each later one waits a cadence.
    // Bounded by the budget's own worst case so a stuck chase cannot spin.
    for _ in 0..64 {
        if w.players[0].ws_hits > 0 {
            let mark = weak_mark8(SEED, cxu, czu, 1, w.players[0].ws_hits);
            let bearing = if chase { mark } else { mark.wrapping_add(128) };
            face = stand_at_bearing(&mut w, s.x, s.y, s.z, bearing);
        }
        let before = w.players[0].inv[0].count;
        seq = seq.wrapping_add(1);
        w.tick(&[hold_primary(face, seq)]);
        if w.players[0].inv[0].count != before || w.slot_lives.is_harvested(cxu, czu) {
            swings += 1;
        }
        if w.slot_lives.is_harvested(cxu, czu) {
            break;
        }
        for _ in 1..SWING_INTERVAL_TICKS {
            seq = seq.wrapping_add(1);
            w.tick(&[stand_still(face, seq)]);
        }
    }
    assert!(
        w.slot_lives.is_harvested(cxu, czu),
        "the node never exhausted — the strike loop is broken, not the sim"
    );
    (swings, w.players[0].inv[0].count as u32)
}

/// **The mark buys speed, not yield** (operator, 2026-08-09; the
/// reference's own model). Chasing the mark must empty the node in
/// strictly fewer swings and pay exactly the same total — the old model
/// paid 1.5× per marked hit and made a skilled player richer, which is a
/// different game from the one being copied.
///
/// This is the gate for a change no other gate can see: every payout is
/// still integer, still bounded, still deterministic, and the node still
/// exhausts. Only the *total* distinguishes the two models.
#[test]
fn the_mark_buys_speed_and_never_yield() {
    let (plain_swings, plain_total) = strike_out(Occupant::Tree, false);
    let (chased_swings, chased_total) = strike_out(Occupant::Tree, true);

    assert_eq!(
        plain_total, chased_total,
        "chasing the mark changed the node's payout ({plain_total} plain vs \
         {chased_total} chased) — the mark is paying yield again"
    );
    assert!(
        chased_swings < plain_swings,
        "chasing the mark cost {chased_swings} swings against {plain_swings} \
         plain — it bought no speed, so it bought nothing"
    );
    // And the payout is the table's own arithmetic, not an accident of
    // the loop: hits × per-hit for the tool in hand (bare, here).
    let def = GatherContent::probe_fixture().nodes[0];
    assert_eq!(
        plain_total,
        def.hits as u32 * def.hand_yield as u32,
        "an unmarked strike-out must pay exactly `hits × per-hit`"
    );
}

/// The finishing share is a redistribution, never a bonus on top: a node
/// with one pays the same total as one without, but back-loads it onto
/// whoever lands the exhausting swing — so abandoning a half-struck node
/// costs you (the reference's anti-cherry-picking rule, Devblog 166).
#[test]
fn the_finish_share_moves_when_yield_arrives_not_how_much() {
    let fixture = GatherContent::probe_fixture();
    let stone = fixture.nodes[1];
    assert!(
        stone.finish_pct > 0,
        "fixture stone node must withhold a finish share"
    );

    let (pos, yaw, (cx, cz)) = find_isolated(SEED, Occupant::StoneNode);
    let (cxu, czu) = (cx as u16, cz as u16);
    let mut w = world_at(pos);
    let mut seq = 0u16;
    let mut last = 0u16;
    let mut pays: Vec<u16> = Vec::new();
    for _ in 0..(stone.hits as u32 + 2) {
        seq = seq.wrapping_add(1);
        w.tick(&[hold_primary(yaw, seq)]);
        let now = w.players[0].inv[0].count;
        if now != last {
            pays.push(now - last);
            last = now;
        }
        if w.slot_lives.is_harvested(cxu, czu) {
            break;
        }
        for _ in 1..SWING_INTERVAL_TICKS {
            seq = seq.wrapping_add(1);
            w.tick(&[stand_still(yaw, seq)]);
        }
    }
    assert!(
        w.slot_lives.is_harvested(cxu, czu),
        "the stone node never exhausted"
    );

    // Total is the table's, untouched by the withholding.
    let total: u32 = pays.iter().map(|p| *p as u32).sum();
    assert_eq!(
        total,
        stone.hits as u32 * stone.hand_yield as u32,
        "the finish share changed the node's payout — it is a \
         redistribution, not a bonus"
    );
    // And it is genuinely back-loaded: the last swing beats every other.
    let finish = *pays.last().expect("at least one payout");
    for (i, p) in pays.iter().enumerate().take(pays.len() - 1) {
        assert!(
            finish > *p,
            "swing {i} paid {p} and the finishing swing paid {finish} — \
             nothing is being withheld for the finisher"
        );
    }
}

// ---------------------------------------------------------------------------
// The wrong tool (DECISIONS.md 2026-08-15: bare hands gather nothing)
//
// Shipped content carries no `hand` row on any swung node, so `bake.rs`'s
// `hand_yield: 0` is what a tree pays a fist. `NodeDef::yield_for` falls back
// to that row for anything not in the tool table, so a torch, a hammer and an
// empty hand all read 0 — and the refusal below is about the pair (node,
// held), never about hands specifically.
//
// The fixture keeps its hand rows, because most of this file is *about* the
// hand row; these two tests zero one node's row for the duration.

/// A swing the node pays nothing for is refused, and **the node does not pay
/// for the refusal.**
///
/// This is the half that is a correctness bug rather than a balance change.
/// `gather::swing` spends the node's budget before it computes a yield, so
/// with the guard removed a bare fist walks the whole `hits × HIT_UNIT`
/// budget down, pays nothing, announces nothing, and arms the 20–45 minute
/// respawn — a griefing hole against any node on the island, and a self-grief
/// hole for the player who lost their rock.
///
/// Proven red by deleting the `def.yield_for(held) == 0` guard: four
/// bare-hand swings then exhaust the fixture tree and the `EV_SLOT_HARVESTED`
/// assertion inside the loop fires.
#[test]
fn a_swing_the_node_pays_nothing_for_is_refused_and_costs_the_node_nothing() {
    let (pos, yaw, (cx, cz)) = find_isolated(SEED, Occupant::Tree);
    let mut w = world_at(pos);
    // The mark buys speed, so leaving it armed would make the swing count
    // (and therefore this test's loop bound) a function of where the body
    // happens to stand. Every other base-pay test in this file does the same.
    for n in w.gather.nodes.iter_mut() {
        n.weak_pct = 0;
    }
    // Content with no `hand` row — what `bake.rs` produces for the shipped
    // gatherables since 2026-08-15.
    w.gather.nodes[0].hand_yield = 0;
    let tree = w.gather.nodes[0];
    let (tool, per_hit) = tree.tools[0];
    assert!(
        tool != sim_core::gather::NO_ITEM && per_hit > 0,
        "fixture rot: the tree has no tool row, so this test cannot tell a \
         refusal from an empty node"
    );

    // Swing bare-handed for longer than the node could survive if the
    // budget were being spent.
    let mut seq = 0u16;
    for _ in 0..SWING_INTERVAL_TICKS * (tree.hits as u64 + 2) {
        w.tick(&[hold_primary(yaw, seq)]);
        seq = seq.wrapping_add(1);
        for e in w.events.entries() {
            assert_ne!(
                e.code, EV_GATHER,
                "a bare-hand swing paid — the `hand` row is back, or the \
                 refusal reads the wrong item"
            );
            assert_ne!(
                e.code, EV_SLOT_HARVESTED,
                "bare hands felled the tree without being paid for it — the \
                 swing is spending the node's budget before the yield check"
            );
        }
    }
    assert!(
        !w.slot_lives.is_harvested(cx as u16, cz as u16),
        "the node is harvested after a bare-hand beating"
    );
    assert!(
        w.players[0].inv.iter().all(|s| s.count == 0),
        "bare hands filled a pocket"
    );

    // **The node is whole, not merely standing.** A guard that refused the
    // pay and still charged the budget would pass everything above and fail
    // here: the tool would collect a remainder instead of the full total.
    w.players[0].inv[0] = ItemStack {
        item: tool,
        count: 1,
    };
    let mut paid = 0u32;
    let mut harvested = 0u32;
    for _ in 0..SWING_INTERVAL_TICKS * (tree.hits as u64 + 2) {
        w.tick(&[hold_primary(yaw, seq)]);
        seq = seq.wrapping_add(1);
        for e in w.events.entries() {
            match e.code {
                EV_GATHER => paid += e.b & 0xFFFF,
                EV_SLOT_HARVESTED => harvested += 1,
                _ => {}
            }
        }
    }
    assert_eq!(harvested, 1, "the node never exhausted for the right tool");
    assert_eq!(
        paid,
        tree.hits as u32 * per_hit as u32,
        "the tool drew a partial node — the bare-hand swings spent budget"
    );
}

/// The refusal is a **whiff, not an absorb**: the arm stays free, so the
/// swing carries on to `combat::strike` exactly as a swing into empty air
/// does (`world.rs`: node → player → structure).
///
/// This is what keeps the wrong tool a *weapon* while it is not a tool. It
/// is not a hypothetical — the shipped rock is both, and a spear is neither
/// a tree's tool nor a harmless thing to be swung at somebody standing in
/// front of one. A guard that absorbed the swing would make a node into
/// cover: stand behind a tree and the fight stops.
///
/// The fixture holds item 2, which `CombatContent::probe_fixture` arms as
/// melee and which is **not** in the tree's tool row (that is item 1) — so
/// `yield_for` falls through to the zeroed hand row and the guard fires,
/// while the strike behind it has a live weapon to resolve.
///
/// Proven red twice, and the pair is the point:
///
/// - return `Swing::Absorbed` from the guard and the victim's hp never
///   moves — the node ate a swing it was paid nothing for;
/// - delete the guard entirely and the node absorbs the swing itself, which
///   is the behaviour before 2026-08-15, and the victim's hp never moves
///   either.
#[test]
fn a_refused_gather_swing_leaves_the_arm_free() {
    let (pos, yaw, _) = find_isolated(SEED, Occupant::Tree);
    let mut w = Box::new(World::new(SEED));
    w.gather = GatherContent::probe_fixture();
    w.combat = sim_core::combat::CombatContent::probe_fixture();
    for n in w.gather.nodes.iter_mut() {
        n.weak_pct = 0;
    }
    w.gather.nodes[0].hand_yield = 0;
    assert_ne!(
        w.gather.nodes[0].tools[0].0, 2,
        "fixture rot: item 2 became the tree's tool, so the swing below \
         would be absorbed by the node and prove nothing"
    );
    // Both bodies on the same point: `dev_spawn` pins every join, so the
    // victim is inside the weapon's reach without a walk.
    w.dev_spawn = Some(pos);
    w.tick(&[Command::Join { id: 1 }, Command::Join { id: 2 }]);
    let victim = w
        .players
        .iter()
        .position(|p| p.active && p.id == 2)
        .expect("the second body seated");
    let hp_before = w.players[victim].hp;
    assert!(hp_before > 0, "the combat fixture granted the victim no hp");

    // A melee-armed item the tree pays nothing for.
    w.players[0].inv[0] = ItemStack { item: 2, count: 1 };
    let mut seq = 0u16;
    for _ in 0..SWING_INTERVAL_TICKS * 3 {
        w.tick(&[hold_primary(yaw, seq)]);
        seq = seq.wrapping_add(1);
    }
    assert!(
        w.players[victim].hp < hp_before,
        "a swing the node refused was absorbed by it anyway — a tree is \
         cover, and the arm never reached the body standing at it"
    );
}
