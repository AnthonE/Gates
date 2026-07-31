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
    for cz in 40..216i32 {
        for cx in 40..216i32 {
            let s = terrain::scatter(seed, &table, cx, cz);
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
                    let n = terrain::scatter(seed, &table, pcx + dx, pcz + dz);
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
    let s = terrain::scatter(SEED, &table, cx, cz);
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
    let mut harvested_tick_had_mark = false;
    let mut saw_harvest = false;
    'outer: for t in 0..SWING_INTERVAL_TICKS * (hits as u64 + 1) {
        w.tick(&[hold_primary(yaw, t as u16)]);
        let mut tick_harvest = false;
        let mut tick_mark = false;
        for e in w.events.entries() {
            match e.code {
                EV_WEAK_MARK => tick_mark = true,
                EV_SLOT_HARVESTED => tick_harvest = true,
                _ => {}
            }
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
    assert_eq!(
        marks_seen,
        hits as u32 - 1,
        "every other landed hit announces one"
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
