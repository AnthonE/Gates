//! `test_protocol_golden` (DESIGN.md §12, CLAUDE.md wall 6): every packet
//! type's encoding is byte-stable against checked-in fixtures, decodes
//! back to exactly what was encoded, and the decoder is total — arbitrary
//! corruption never panics it. A diff against a fixture without a
//! `PROTO_VER` bump in the same commit is the wire drifting by accident.

use protocol::goldens::{
    action_cancel, action_consume, action_container, action_container_close, action_craft,
    action_deploy, action_feed, action_lock, action_move, action_move_box, action_place,
    action_repair, action_respawn, action_upgrade, action_use, chat, event_bag_dropped,
    event_bag_removed, event_bag_sync, event_build_refused, event_catalog, event_chat,
    event_consume_refused, event_consumed, event_cont_close, event_cont_sync, event_craft_done,
    event_craft_q, event_craft_refused, event_death, event_deploy_defs, event_deploy_placed,
    event_deploy_refused, event_deploy_sync, event_door, event_drank, event_gather, event_health,
    event_hit, event_inv, event_move_refused, event_moved, event_piece_defs, event_piece_placed,
    event_piece_repaired, event_piece_sync, event_recipes, event_removed, event_respawn,
    event_slot_change, event_slot_sync, event_stock, event_struct_hit_deploy,
    event_struct_hit_piece, event_vitals, event_weak_mark, hello, input_acks_only, input_full,
    refuse_full, snapshot_cap, snapshot_delta, snapshot_keyframe, welcome, SnapshotCase, FIXTURES,
};
use protocol::{
    decode_action, decode_chat, decode_event, decode_hello, decode_input, decode_refuse,
    decode_snapshot, decode_welcome, encode_action_cancel, encode_action_consume,
    encode_action_container, encode_action_craft, encode_action_deploy, encode_action_drink,
    encode_action_feed, encode_action_lock, encode_action_loot, encode_action_move,
    encode_action_place, encode_action_repair, encode_action_respawn, encode_action_upgrade,
    encode_action_use, encode_chat, encode_event_bag_dropped, encode_event_bag_removed,
    encode_event_bag_sync, encode_event_build_refused, encode_event_catalog, encode_event_chat,
    encode_event_consume_refused, encode_event_consumed, encode_event_cont_sync,
    encode_event_craft_done, encode_event_craft_q, encode_event_craft_refused, encode_event_death,
    encode_event_deploy_defs, encode_event_deploy_placed, encode_event_deploy_refused,
    encode_event_deploy_sync, encode_event_door, encode_event_drank, encode_event_gather,
    encode_event_health, encode_event_hit, encode_event_inv, encode_event_move_refused,
    encode_event_moved, encode_event_piece_defs, encode_event_piece_placed,
    encode_event_piece_repaired, encode_event_piece_sync, encode_event_recipes,
    encode_event_removed, encode_event_respawn, encode_event_slot_change, encode_event_slot_sync,
    encode_event_stock, encode_event_struct_hit, encode_event_vitals, encode_event_weak_mark,
    encode_hello, encode_input, encode_refuse, encode_snapshot, encode_welcome, peek_kind,
    ActionMsg, ChatMsg, EventMsg, InputDatagram, InvSlot, WireError, BAG_SYNC_BATCH, CATALOG_BATCH,
    CONT_SYNC_BATCH, DEPLOY_SYNC_BATCH, KIND_ACTION, KIND_CHAT, KIND_EVENT, KIND_HELLO, KIND_INPUT,
    KIND_REFUSE, KIND_SNAPSHOT, KIND_WELCOME, MAX_EVENT_MSG_BYTES, PIECE_DEFS_BATCH,
    PIECE_SYNC_BATCH, RECIPE_BATCH, SLOT_SYNC_BATCH,
};
use sim_core::input::InputFrame;
use sim_core::limits::DATAGRAM_BUDGET_BYTES;
use sim_core::rng::Pcg32;

const GOLDEN: [&[u8]; 68] = [
    include_bytes!("golden/v20_input_acks_only.bin"),
    include_bytes!("golden/v20_input_full.bin"),
    include_bytes!("golden/v20_snapshot_keyframe.bin"),
    include_bytes!("golden/v20_snapshot_delta.bin"),
    include_bytes!("golden/v20_snapshot_cap.bin"),
    include_bytes!("golden/v20_hello.bin"),
    include_bytes!("golden/v20_welcome.bin"),
    include_bytes!("golden/v20_refuse_full.bin"),
    include_bytes!("golden/v20_event_gather.bin"),
    include_bytes!("golden/v20_event_inv.bin"),
    include_bytes!("golden/v20_event_slot_harvested.bin"),
    include_bytes!("golden/v20_event_slot_respawned.bin"),
    include_bytes!("golden/v20_event_slot_sync.bin"),
    include_bytes!("golden/v20_event_catalog.bin"),
    include_bytes!("golden/v20_event_weak_mark.bin"),
    include_bytes!("golden/v20_event_craft_q.bin"),
    include_bytes!("golden/v20_event_craft_done.bin"),
    include_bytes!("golden/v20_event_craft_refused.bin"),
    include_bytes!("golden/v20_event_recipes.bin"),
    include_bytes!("golden/v20_action_craft.bin"),
    include_bytes!("golden/v20_action_cancel.bin"),
    include_bytes!("golden/v20_action_place.bin"),
    include_bytes!("golden/v20_event_piece_placed.bin"),
    include_bytes!("golden/v20_event_piece_sync.bin"),
    include_bytes!("golden/v20_event_build_refused.bin"),
    include_bytes!("golden/v20_event_piece_defs.bin"),
    include_bytes!("golden/v20_action_deploy.bin"),
    include_bytes!("golden/v20_action_feed.bin"),
    include_bytes!("golden/v20_event_deploy_placed.bin"),
    include_bytes!("golden/v20_event_deploy_sync.bin"),
    include_bytes!("golden/v20_event_deploy_refused.bin"),
    include_bytes!("golden/v20_event_deploy_defs.bin"),
    include_bytes!("golden/v20_event_piece_removed.bin"),
    include_bytes!("golden/v20_event_deploy_removed.bin"),
    include_bytes!("golden/v20_event_stock.bin"),
    include_bytes!("golden/v20_action_use.bin"),
    include_bytes!("golden/v20_action_lock.bin"),
    include_bytes!("golden/v20_event_door.bin"),
    include_bytes!("golden/v20_action_upgrade.bin"),
    include_bytes!("golden/v20_chat.bin"),
    include_bytes!("golden/v20_event_chat.bin"),
    include_bytes!("golden/v20_event_hit.bin"),
    include_bytes!("golden/v20_event_health.bin"),
    include_bytes!("golden/v20_event_death.bin"),
    include_bytes!("golden/v20_action_loot.bin"),
    include_bytes!("golden/v20_event_bag_dropped.bin"),
    include_bytes!("golden/v20_event_bag_sync.bin"),
    include_bytes!("golden/v20_event_bag_removed.bin"),
    include_bytes!("golden/v20_event_struct_hit_piece.bin"),
    include_bytes!("golden/v20_event_struct_hit_deploy.bin"),
    include_bytes!("golden/v20_event_vitals.bin"),
    include_bytes!("golden/v20_event_consumed.bin"),
    include_bytes!("golden/v20_event_consume_refused.bin"),
    include_bytes!("golden/v20_action_consume.bin"),
    include_bytes!("golden/v20_event_drank.bin"),
    include_bytes!("golden/v20_action_drink.bin"),
    include_bytes!("golden/v20_event_respawn.bin"),
    include_bytes!("golden/v20_action_respawn.bin"),
    include_bytes!("golden/v20_action_move.bin"),
    include_bytes!("golden/v20_event_moved.bin"),
    include_bytes!("golden/v20_event_move_refused.bin"),
    include_bytes!("golden/v20_action_move_box.bin"),
    include_bytes!("golden/v20_action_container.bin"),
    include_bytes!("golden/v20_action_container_close.bin"),
    include_bytes!("golden/v20_event_cont_sync.bin"),
    include_bytes!("golden/v20_event_cont_close.bin"),
    include_bytes!("golden/v20_action_repair.bin"),
    include_bytes!("golden/v20_event_piece_repaired.bin"),
];

fn encode_case(case: &SnapshotCase) -> ([u8; DATAGRAM_BUDGET_BYTES], usize) {
    let mut buf = [0u8; DATAGRAM_BUDGET_BYTES];
    let len = encode_snapshot(
        &case.header,
        case.removed,
        case.entities(),
        case.baseline(),
        &mut buf,
    )
    .expect("golden case encodes");
    (buf, len)
}

fn golden_input(dg: &InputDatagram, fixture: &[u8], name: &str) {
    let mut buf = [0u8; DATAGRAM_BUDGET_BYTES];
    let len = encode_input(dg, &mut buf).expect("golden case encodes");
    assert_eq!(&buf[..len], fixture, "{name}: bytes drifted");
    assert_eq!(peek_kind(fixture).unwrap(), KIND_INPUT);
    let back = decode_input(fixture).expect("fixture decodes");
    assert_eq!(&back, dg, "{name}: decode mismatch");
}

fn golden_snapshot(case: &SnapshotCase, fixture: &[u8], name: &str) {
    let (buf, len) = encode_case(case);
    assert_eq!(&buf[..len], fixture, "{name}: bytes drifted");
    assert_eq!(peek_kind(fixture).unwrap(), KIND_SNAPSHOT);
    let back = decode_snapshot(fixture, case.baseline()).expect("fixture decodes");
    assert_eq!(back.header, case.header, "{name}: header mismatch");
    assert_eq!(back.removed(), case.removed, "{name}: removals mismatch");
    assert_eq!(back.entities(), case.entities(), "{name}: entity mismatch");
}

#[test]
fn test_protocol_golden() {
    golden_input(&input_acks_only(), GOLDEN[0], FIXTURES[0]);
    golden_input(&input_full(), GOLDEN[1], FIXTURES[1]);
    golden_snapshot(&snapshot_keyframe(), GOLDEN[2], FIXTURES[2]);
    golden_snapshot(&snapshot_delta(), GOLDEN[3], FIXTURES[3]);
    golden_snapshot(&snapshot_cap(), GOLDEN[4], FIXTURES[4]);
    golden_stream(GOLDEN[5], FIXTURES[5]);
    golden_stream(GOLDEN[6], FIXTURES[6]);
    golden_stream(GOLDEN[7], FIXTURES[7]);
    for i in 8..19 {
        golden_event(GOLDEN[i], FIXTURES[i]);
    }
    golden_action(GOLDEN[19], FIXTURES[19]);
    golden_action(GOLDEN[20], FIXTURES[20]);
    golden_action(GOLDEN[21], FIXTURES[21]);
    for i in 22..26 {
        golden_event(GOLDEN[i], FIXTURES[i]);
    }
    golden_action(GOLDEN[26], FIXTURES[26]);
    golden_action(GOLDEN[27], FIXTURES[27]);
    for i in 28..35 {
        golden_event(GOLDEN[i], FIXTURES[i]);
    }
    golden_action(GOLDEN[35], FIXTURES[35]);
    golden_action(GOLDEN[36], FIXTURES[36]);
    golden_event(GOLDEN[37], FIXTURES[37]);
    golden_action(GOLDEN[38], FIXTURES[38]);
    golden_chat(GOLDEN[39], FIXTURES[39]);
    for i in 40..44 {
        golden_event(GOLDEN[i], FIXTURES[i]);
    }
    golden_action(GOLDEN[44], FIXTURES[44]);
    for i in 45..53 {
        golden_event(GOLDEN[i], FIXTURES[i]);
    }
    golden_action(GOLDEN[53], FIXTURES[53]);
    golden_event(GOLDEN[54], FIXTURES[54]);
    golden_action(GOLDEN[55], FIXTURES[55]);
    golden_event(GOLDEN[56], FIXTURES[56]);
    golden_action(GOLDEN[57], FIXTURES[57]);
    golden_action(GOLDEN[58], FIXTURES[58]);
    golden_event(GOLDEN[59], FIXTURES[59]);
    golden_event(GOLDEN[60], FIXTURES[60]);
    golden_action(GOLDEN[61], FIXTURES[61]);
    golden_action(GOLDEN[62], FIXTURES[62]);
    golden_action(GOLDEN[63], FIXTURES[63]);
    golden_event(GOLDEN[64], FIXTURES[64]);
    golden_event(GOLDEN[65], FIXTURES[65]);
    golden_action(GOLDEN[66], FIXTURES[66]);
    golden_event(GOLDEN[67], FIXTURES[67]);
    // Every fixture in the manifest was dispatched above: the loop bounds
    // are hand-written, so a fixture added to `FIXTURES` and forgotten
    // here would be a golden nobody checks. This is the count that makes
    // that impossible to miss quietly.
    assert_eq!(GOLDEN.len(), FIXTURES.len());
    assert_eq!(GOLDEN.len(), 68, "a new fixture must be dispatched above");
}

/// The C→S chat frame: its own kind, byte-stable, decode-exact.
fn golden_chat(fixture: &[u8], name: &str) {
    let mut buf = [0u8; 64];
    assert_eq!(peek_kind(fixture).unwrap(), KIND_CHAT, "{name}");
    let (text, global) = chat();
    assert_eq!(
        decode_chat(fixture).unwrap(),
        ChatMsg { global, text },
        "{name}: decode mismatch"
    );
    let len = encode_chat(text.as_bytes(), global, &mut buf).unwrap();
    assert_eq!(&buf[..len], fixture, "{name}: bytes drifted");
}

/// C→S action frames: byte-stable, kind-peekable, decode-exact.
fn golden_action(fixture: &[u8], name: &str) {
    let mut buf = [0u8; 64];
    assert_eq!(peek_kind(fixture).unwrap(), KIND_ACTION, "{name}");
    let len = match name {
        "v20_action_craft.bin" => {
            let (recipe, count) = action_craft();
            assert_eq!(
                decode_action(fixture).unwrap(),
                ActionMsg::Craft { recipe, count },
                "{name}: decode mismatch"
            );
            encode_action_craft(recipe, count, &mut buf).unwrap()
        }
        "v20_action_cancel.bin" => {
            let index = action_cancel();
            assert_eq!(
                decode_action(fixture).unwrap(),
                ActionMsg::CraftCancel { index },
                "{name}: decode mismatch"
            );
            encode_action_cancel(index, &mut buf).unwrap()
        }
        "v20_action_place.bin" => {
            let (row, cx, cz, level, loc) = action_place();
            assert_eq!(
                decode_action(fixture).unwrap(),
                ActionMsg::Place {
                    row,
                    cx,
                    cz,
                    level,
                    loc,
                },
                "{name}: decode mismatch"
            );
            encode_action_place(row, cx, cz, level, loc, &mut buf).unwrap()
        }
        "v20_action_deploy.bin" => {
            let (row, cx, cz, level, loc) = action_deploy();
            assert_eq!(
                decode_action(fixture).unwrap(),
                ActionMsg::Deploy {
                    row,
                    cx,
                    cz,
                    level,
                    loc,
                },
                "{name}: decode mismatch"
            );
            encode_action_deploy(row, cx, cz, level, loc, &mut buf).unwrap()
        }
        "v20_action_feed.bin" => {
            let (cx, cz, level) = action_feed();
            assert_eq!(
                decode_action(fixture).unwrap(),
                ActionMsg::Feed { cx, cz, level },
                "{name}: decode mismatch"
            );
            encode_action_feed(cx, cz, level, &mut buf).unwrap()
        }
        "v20_action_use.bin" => {
            let (cx, cz, level, loc) = action_use();
            assert_eq!(
                decode_action(fixture).unwrap(),
                ActionMsg::Use { cx, cz, level, loc },
                "{name}: decode mismatch"
            );
            encode_action_use(cx, cz, level, loc, &mut buf).unwrap()
        }
        "v20_action_lock.bin" => {
            let (cx, cz, level, loc, locked) = action_lock();
            assert_eq!(
                decode_action(fixture).unwrap(),
                ActionMsg::Lock {
                    cx,
                    cz,
                    level,
                    loc,
                    locked,
                },
                "{name}: decode mismatch"
            );
            encode_action_lock(cx, cz, level, loc, locked, &mut buf).unwrap()
        }
        "v20_action_upgrade.bin" => {
            let (cx, cz, level, loc, material) = action_upgrade();
            assert_eq!(
                decode_action(fixture).unwrap(),
                ActionMsg::Upgrade {
                    cx,
                    cz,
                    level,
                    loc,
                    material,
                },
                "{name}: decode mismatch"
            );
            encode_action_upgrade(cx, cz, level, loc, material, &mut buf).unwrap()
        }
        "v20_action_loot.bin" => {
            assert_eq!(
                decode_action(fixture).unwrap(),
                ActionMsg::Loot,
                "{name}: decode mismatch"
            );
            encode_action_loot(&mut buf).unwrap()
        }
        "v20_action_consume.bin" => {
            let slot = action_consume();
            assert_eq!(
                decode_action(fixture).unwrap(),
                ActionMsg::Consume { slot },
                "{name}: decode mismatch"
            );
            encode_action_consume(slot, &mut buf).unwrap()
        }
        "v20_action_drink.bin" => {
            assert_eq!(
                decode_action(fixture).unwrap(),
                ActionMsg::Drink,
                "{name}: decode mismatch"
            );
            encode_action_drink(&mut buf).unwrap()
        }
        "v20_action_respawn.bin" => {
            let on_bag = action_respawn();
            assert_eq!(
                decode_action(fixture).unwrap(),
                ActionMsg::Respawn { on_bag },
                "{name}: decode mismatch"
            );
            encode_action_respawn(on_bag, &mut buf).unwrap()
        }
        "v20_action_move.bin" => {
            let (cont, from_kind, from_slot, to_kind, to_slot, count) = action_move();
            assert_eq!(
                decode_action(fixture).unwrap(),
                ActionMsg::Move {
                    cont,
                    from_kind,
                    from_slot,
                    to_kind,
                    to_slot,
                    count,
                },
                "{name}: decode mismatch"
            );
            encode_action_move(
                cont, from_kind, from_slot, to_kind, to_slot, count, &mut buf,
            )
            .unwrap()
        }
        "v20_action_move_box.bin" => {
            let (cont, from_kind, from_slot, to_kind, to_slot, count) = action_move_box();
            assert_eq!(
                decode_action(fixture).unwrap(),
                ActionMsg::Move {
                    cont,
                    from_kind,
                    from_slot,
                    to_kind,
                    to_slot,
                    count,
                },
                "{name}: decode mismatch"
            );
            encode_action_move(
                cont, from_kind, from_slot, to_kind, to_slot, count, &mut buf,
            )
            .unwrap()
        }
        "v20_action_repair.bin" => {
            let (cx, cz, level, loc) = action_repair();
            assert_eq!(
                decode_action(fixture).unwrap(),
                ActionMsg::Repair { cx, cz, level, loc },
                "{name}: decode mismatch"
            );
            encode_action_repair(cx, cz, level, loc, &mut buf).unwrap()
        }
        "v20_action_container.bin" | "v20_action_container_close.bin" => {
            let (kind, cont) = if name == "v20_action_container.bin" {
                action_container()
            } else {
                action_container_close()
            };
            assert_eq!(
                decode_action(fixture).unwrap(),
                ActionMsg::Container { kind, cont },
                "{name}: decode mismatch"
            );
            encode_action_container(kind, cont, &mut buf).unwrap()
        }
        other => panic!("unknown action fixture {other}"),
    };
    assert_eq!(&buf[..len], fixture, "{name}: bytes drifted");
}

/// The handshake trio: byte-stable, kind-peekable, decode-exact.
fn golden_stream(fixture: &[u8], name: &str) {
    let mut buf = [0u8; 64];
    match name {
        "v20_hello.bin" => {
            let len = encode_hello(&hello(), &mut buf).unwrap();
            assert_eq!(&buf[..len], fixture, "{name}: bytes drifted");
            assert_eq!(peek_kind(fixture).unwrap(), KIND_HELLO);
            assert_eq!(decode_hello(fixture).unwrap(), hello());
        }
        "v20_welcome.bin" => {
            let len = encode_welcome(&welcome(), &mut buf).unwrap();
            assert_eq!(&buf[..len], fixture, "{name}: bytes drifted");
            assert_eq!(peek_kind(fixture).unwrap(), KIND_WELCOME);
            assert_eq!(decode_welcome(fixture).unwrap(), welcome());
        }
        "v20_refuse_full.bin" => {
            let len = encode_refuse(&refuse_full(), &mut buf).unwrap();
            assert_eq!(&buf[..len], fixture, "{name}: bytes drifted");
            assert_eq!(peek_kind(fixture).unwrap(), KIND_REFUSE);
            assert_eq!(decode_refuse(fixture).unwrap(), refuse_full());
        }
        other => panic!("unknown stream fixture {other}"),
    }
}

/// Event-lane messages: byte-stable, kind-peekable, decode-exact.
fn golden_event(fixture: &[u8], name: &str) {
    let mut buf = [0u8; MAX_EVENT_MSG_BYTES];
    assert_eq!(peek_kind(fixture).unwrap(), KIND_EVENT, "{name}");
    let len = match name {
        "v20_event_gather.bin" => {
            let (item, added) = event_gather();
            assert_eq!(
                decode_event(fixture).unwrap(),
                EventMsg::Gather { item, added },
                "{name}: decode mismatch"
            );
            encode_event_gather(item, added, &mut buf).unwrap()
        }
        "v20_event_inv.bin" => {
            let (slots, count) = event_inv();
            match decode_event(fixture).unwrap() {
                EventMsg::Inv {
                    slots: got,
                    count: got_n,
                } => {
                    assert_eq!(got_n as usize, count, "{name}: count mismatch");
                    assert_eq!(got[..count], slots[..count], "{name}: decode mismatch");
                }
                other => panic!("{name}: wrong variant {other:?}"),
            }
            encode_event_inv(&slots[..count], &mut buf).unwrap()
        }
        "v20_event_slot_harvested.bin" | "v20_event_slot_respawned.bin" => {
            let harvested = name == "v20_event_slot_harvested.bin";
            let (cx, cz) = event_slot_change();
            let want = if harvested {
                EventMsg::SlotHarvested { cx, cz }
            } else {
                EventMsg::SlotRespawned { cx, cz }
            };
            assert_eq!(decode_event(fixture).unwrap(), want, "{name}");
            encode_event_slot_change(harvested, cx, cz, &mut buf).unwrap()
        }
        "v20_event_slot_sync.bin" => {
            let (reset, cells) = event_slot_sync();
            match decode_event(fixture).unwrap() {
                EventMsg::SlotSync {
                    reset: got_r,
                    cells: got,
                    count,
                } => {
                    assert_eq!(got_r, reset, "{name}: reset mismatch");
                    assert_eq!(count as usize, SLOT_SYNC_BATCH);
                    assert_eq!(got, cells, "{name}: decode mismatch");
                }
                other => panic!("{name}: wrong variant {other:?}"),
            }
            encode_event_slot_sync(reset, &cells, &mut buf).unwrap()
        }
        "v20_event_catalog.bin" => {
            let cat = event_catalog();
            match decode_event(fixture).unwrap() {
                EventMsg::Catalog {
                    total,
                    first,
                    count,
                    names,
                    lens,
                } => {
                    assert_eq!(
                        (total, first, count),
                        (cat.count as u8, 0, CATALOG_BATCH as u8),
                        "{name}: header mismatch"
                    );
                    for i in 0..count as usize {
                        assert_eq!(
                            &names[i][..lens[i] as usize],
                            cat.name(i),
                            "{name}: name {i} mismatch"
                        );
                    }
                }
                other => panic!("{name}: wrong variant {other:?}"),
            }
            let (len, took) = encode_event_catalog(&cat, 0, &mut buf).unwrap();
            assert_eq!(took, CATALOG_BATCH, "{name}: batch shrank");
            len
        }
        "v20_event_weak_mark.bin" => {
            let (cx, cz, mark8, weak_hit) = event_weak_mark();
            assert_eq!(
                decode_event(fixture).unwrap(),
                EventMsg::WeakMark {
                    cx,
                    cz,
                    mark8,
                    weak_hit,
                },
                "{name}: decode mismatch"
            );
            encode_event_weak_mark(cx, cz, mark8, weak_hit, &mut buf).unwrap()
        }
        "v20_event_craft_q.bin" => {
            let (jobs, eta) = event_craft_q();
            match decode_event(fixture).unwrap() {
                EventMsg::CraftQ {
                    jobs: got,
                    count,
                    eta_ticks,
                } => {
                    assert_eq!(count as usize, jobs.len(), "{name}: count mismatch");
                    assert_eq!(eta_ticks, eta, "{name}: eta mismatch");
                    for (i, j) in jobs.iter().enumerate() {
                        assert_eq!(
                            got[i],
                            (j.recipe as u8, j.remaining as u8),
                            "{name}: job {i} mismatch"
                        );
                    }
                }
                other => panic!("{name}: wrong variant {other:?}"),
            }
            encode_event_craft_q(&jobs, eta, &mut buf).unwrap()
        }
        "v20_event_craft_done.bin" => {
            let (item, added) = event_craft_done();
            assert_eq!(
                decode_event(fixture).unwrap(),
                EventMsg::CraftDone { item, added },
                "{name}: decode mismatch"
            );
            encode_event_craft_done(item, added, &mut buf).unwrap()
        }
        "v20_event_craft_refused.bin" => {
            let reason = event_craft_refused();
            assert_eq!(
                decode_event(fixture).unwrap(),
                EventMsg::CraftRefused { reason },
                "{name}: decode mismatch"
            );
            encode_event_craft_refused(reason, &mut buf).unwrap()
        }
        "v20_event_recipes.bin" => {
            let cc = event_recipes();
            match decode_event(fixture).unwrap() {
                EventMsg::Recipes {
                    total,
                    first,
                    count,
                    rows,
                } => {
                    assert_eq!(
                        (total, first, count),
                        (cc.recipe_count as u8, 0, RECIPE_BATCH as u8),
                        "{name}: header mismatch"
                    );
                    for (i, row) in rows.iter().enumerate().take(count as usize) {
                        assert_eq!(*row, cc.recipes[i], "{name}: row {i} mismatch");
                    }
                }
                other => panic!("{name}: wrong variant {other:?}"),
            }
            let (len, took) = encode_event_recipes(&cc, 0, &mut buf).unwrap();
            assert_eq!(took, RECIPE_BATCH, "{name}: batch shrank");
            len
        }
        "v20_event_piece_placed.bin" => {
            let rec = event_piece_placed();
            assert_eq!(
                decode_event(fixture).unwrap(),
                EventMsg::PiecePlaced { rec },
                "{name}: decode mismatch"
            );
            encode_event_piece_placed(&rec, &mut buf).unwrap()
        }
        "v20_event_piece_sync.bin" => {
            let (reset, recs) = event_piece_sync();
            match decode_event(fixture).unwrap() {
                EventMsg::PieceSync {
                    reset: got_r,
                    recs: got,
                    count,
                } => {
                    assert_eq!(got_r, reset, "{name}: reset mismatch");
                    assert_eq!(count as usize, PIECE_SYNC_BATCH);
                    assert_eq!(got, recs, "{name}: decode mismatch");
                }
                other => panic!("{name}: wrong variant {other:?}"),
            }
            encode_event_piece_sync(reset, &recs, &mut buf).unwrap()
        }
        "v20_event_build_refused.bin" => {
            let reason = event_build_refused();
            assert_eq!(
                decode_event(fixture).unwrap(),
                EventMsg::BuildRefused { reason },
                "{name}: decode mismatch"
            );
            encode_event_build_refused(reason, &mut buf).unwrap()
        }
        "v20_event_piece_defs.bin" => {
            let bc = event_piece_defs();
            match decode_event(fixture).unwrap() {
                EventMsg::PieceDefs {
                    total,
                    first,
                    count,
                    rows,
                } => {
                    assert_eq!(
                        (total, first, count),
                        (bc.piece_count as u8, 0, PIECE_DEFS_BATCH as u8),
                        "{name}: header mismatch"
                    );
                    for (i, row) in rows.iter().enumerate().take(count as usize) {
                        assert_eq!(*row, bc.pieces[i], "{name}: row {i} mismatch");
                    }
                }
                other => panic!("{name}: wrong variant {other:?}"),
            }
            let (len, took) = encode_event_piece_defs(&bc, 0, &mut buf).unwrap();
            assert_eq!(took, PIECE_DEFS_BATCH, "{name}: batch shrank");
            len
        }
        "v20_event_deploy_placed.bin" => {
            let rec = event_deploy_placed();
            assert_eq!(
                decode_event(fixture).unwrap(),
                EventMsg::DeployPlaced { rec },
                "{name}: decode mismatch"
            );
            encode_event_deploy_placed(&rec, &mut buf).unwrap()
        }
        "v20_event_deploy_sync.bin" => {
            let (reset, recs) = event_deploy_sync();
            match decode_event(fixture).unwrap() {
                EventMsg::DeploySync {
                    reset: got_r,
                    recs: got,
                    count,
                } => {
                    assert_eq!(got_r, reset, "{name}: reset mismatch");
                    assert_eq!(count as usize, DEPLOY_SYNC_BATCH);
                    assert_eq!(got, recs, "{name}: decode mismatch");
                }
                other => panic!("{name}: wrong variant {other:?}"),
            }
            encode_event_deploy_sync(reset, &recs, &mut buf).unwrap()
        }
        "v20_event_deploy_refused.bin" => {
            let reason = event_deploy_refused();
            assert_eq!(
                decode_event(fixture).unwrap(),
                EventMsg::DeployRefused { reason },
                "{name}: decode mismatch"
            );
            encode_event_deploy_refused(reason, &mut buf).unwrap()
        }
        "v20_event_deploy_defs.bin" => {
            let dc = event_deploy_defs();
            match decode_event(fixture).unwrap() {
                EventMsg::DeployDefs {
                    total,
                    first,
                    count,
                    rows,
                } => {
                    assert_eq!(
                        (total, first, count),
                        (dc.def_count as u8, 0, dc.def_count as u8),
                        "{name}: header mismatch"
                    );
                    for (i, row) in rows.iter().enumerate().take(count as usize) {
                        assert_eq!(*row, dc.defs[i], "{name}: row {i} mismatch");
                    }
                }
                other => panic!("{name}: wrong variant {other:?}"),
            }
            let (len, took) = encode_event_deploy_defs(&dc, 0, &mut buf).unwrap();
            assert_eq!(took, dc.def_count as usize, "{name}: batch shrank");
            len
        }
        "v20_event_piece_removed.bin" | "v20_event_deploy_removed.bin" => {
            let piece = name == "v20_event_piece_removed.bin";
            let (cx, cz, level, loc) = event_removed();
            let want = if piece {
                EventMsg::PieceRemoved { cx, cz, level, loc }
            } else {
                EventMsg::DeployRemoved { cx, cz, level, loc }
            };
            assert_eq!(decode_event(fixture).unwrap(), want, "{name}");
            encode_event_removed(piece, cx, cz, level, loc, &mut buf).unwrap()
        }
        "v20_event_stock.bin" => {
            let (cx, cz, level, rows) = event_stock();
            match decode_event(fixture).unwrap() {
                EventMsg::Stock {
                    cx: gx,
                    cz: gz,
                    level: gl,
                    rows: got,
                    count,
                } => {
                    assert_eq!((gx, gz, gl), (cx, cz, level), "{name}: address");
                    assert_eq!(count as usize, rows.len(), "{name}: count");
                    assert_eq!(&got[..rows.len()], &rows, "{name}: rows mismatch");
                }
                other => panic!("{name}: wrong variant {other:?}"),
            }
            encode_event_stock(cx, cz, level, &rows, &mut buf).unwrap()
        }
        "v20_event_door.bin" => {
            let (cx, cz, level, loc, open, locked) = event_door();
            assert_eq!(
                decode_event(fixture).unwrap(),
                EventMsg::Door {
                    cx,
                    cz,
                    level,
                    loc,
                    open,
                    locked,
                },
                "{name}: decode mismatch"
            );
            encode_event_door(cx, cz, level, loc, open, locked, &mut buf).unwrap()
        }
        "v20_event_chat.bin" => {
            let (from, global, text) = event_chat();
            assert_eq!(
                decode_event(fixture).unwrap(),
                EventMsg::Chat { from, global, text },
                "{name}: decode mismatch"
            );
            encode_event_chat(from, global, &text, &mut buf).unwrap()
        }
        "v20_event_hit.bin" => {
            let (victim, damage) = event_hit();
            assert_eq!(
                decode_event(fixture).unwrap(),
                EventMsg::Hit { victim, damage },
                "{name}: decode mismatch"
            );
            encode_event_hit(victim, damage, &mut buf).unwrap()
        }
        "v20_event_health.bin" => {
            let (hp, max) = event_health();
            assert_eq!(
                decode_event(fixture).unwrap(),
                EventMsg::Health { hp, max },
                "{name}: decode mismatch"
            );
            encode_event_health(hp, max, &mut buf).unwrap()
        }
        "v20_event_death.bin" => {
            let (victim, killer, cause, item, range_cm) = event_death();
            assert_eq!(
                decode_event(fixture).unwrap(),
                EventMsg::Death {
                    victim,
                    killer,
                    cause,
                    item,
                    range_cm,
                },
                "{name}: decode mismatch"
            );
            encode_event_death(victim, killer, cause, item, range_cm, &mut buf).unwrap()
        }
        "v20_event_respawn.bin" => {
            let on_bag = event_respawn();
            assert_eq!(
                decode_event(fixture).unwrap(),
                EventMsg::Respawn { on_bag },
                "{name}: decode mismatch"
            );
            encode_event_respawn(on_bag, &mut buf).unwrap()
        }
        "v20_event_moved.bin" => {
            let (from_kind, from_slot, to_kind, to_slot, count, item) = event_moved();
            assert_eq!(
                decode_event(fixture).unwrap(),
                EventMsg::Moved {
                    from_kind,
                    from_slot,
                    to_kind,
                    to_slot,
                    count,
                    item,
                },
                "{name}: decode mismatch"
            );
            encode_event_moved(
                from_kind, from_slot, to_kind, to_slot, count, item, &mut buf,
            )
            .unwrap()
        }
        "v20_event_move_refused.bin" => {
            let (reason, from_kind, from_slot, to_kind, to_slot) = event_move_refused();
            assert_eq!(
                decode_event(fixture).unwrap(),
                EventMsg::MoveRefused {
                    reason,
                    from_kind,
                    from_slot,
                    to_kind,
                    to_slot,
                },
                "{name}: decode mismatch"
            );
            encode_event_move_refused(reason, from_kind, from_slot, to_kind, to_slot, &mut buf)
                .unwrap()
        }
        "v20_event_bag_dropped.bin" => {
            let b = event_bag_dropped();
            assert_eq!(
                decode_event(fixture).unwrap(),
                EventMsg::BagDropped {
                    id: b.id,
                    qx: b.qx,
                    qy: b.qy,
                    qz: b.qz,
                },
                "{name}: decode mismatch"
            );
            encode_event_bag_dropped(&b, &mut buf).unwrap()
        }
        "v20_event_bag_sync.bin" => {
            let (reset, recs) = event_bag_sync();
            assert_eq!(
                decode_event(fixture).unwrap(),
                EventMsg::BagSync {
                    reset,
                    recs,
                    count: BAG_SYNC_BATCH as u8,
                },
                "{name}: decode mismatch"
            );
            encode_event_bag_sync(reset, &recs, &mut buf).unwrap()
        }
        "v20_event_bag_removed.bin" => {
            let (id, why) = event_bag_removed();
            assert_eq!(
                decode_event(fixture).unwrap(),
                EventMsg::BagRemoved { id, why },
                "{name}: decode mismatch"
            );
            encode_event_bag_removed(id, why, &mut buf).unwrap()
        }
        n @ ("v20_event_struct_hit_piece.bin" | "v20_event_struct_hit_deploy.bin") => {
            let (deploy, cx, cz, level, loc, row, damage, left) = if n.ends_with("piece.bin") {
                event_struct_hit_piece()
            } else {
                event_struct_hit_deploy()
            };
            assert_eq!(
                decode_event(fixture).unwrap(),
                EventMsg::StructHit {
                    deploy,
                    cx,
                    cz,
                    level,
                    loc,
                    damage,
                    left,
                },
                "{name}: decode mismatch"
            );
            encode_event_struct_hit(deploy, cx, cz, level, loc, row, damage, left, &mut buf)
                .unwrap()
        }
        "v20_event_vitals.bin" => {
            let (food, water, max_food, max_water) = event_vitals();
            assert_eq!(
                decode_event(fixture).unwrap(),
                EventMsg::Vitals {
                    food,
                    water,
                    max_food,
                    max_water
                },
                "{name}: decode mismatch"
            );
            encode_event_vitals(food, water, max_food, max_water, &mut buf).unwrap()
        }
        "v20_event_consumed.bin" => {
            let (item, slot) = event_consumed();
            assert_eq!(
                decode_event(fixture).unwrap(),
                EventMsg::Consumed { item, slot },
                "{name}: decode mismatch"
            );
            encode_event_consumed(item, slot, &mut buf).unwrap()
        }
        "v20_event_consume_refused.bin" => {
            let reason = event_consume_refused();
            assert_eq!(
                decode_event(fixture).unwrap(),
                EventMsg::ConsumeRefused { reason },
                "{name}: decode mismatch"
            );
            encode_event_consume_refused(reason, &mut buf).unwrap()
        }
        "v20_event_drank.bin" => {
            let (water, hp_cost) = event_drank();
            assert_eq!(
                decode_event(fixture).unwrap(),
                EventMsg::Drank { water, hp_cost },
                "{name}: decode mismatch"
            );
            encode_event_drank(water, hp_cost, &mut buf).unwrap()
        }
        "v20_event_cont_sync.bin" => {
            let (kind, cont, reset, rows) = event_cont_sync();
            let mut slots = [InvSlot::default(); CONT_SYNC_BATCH];
            slots[..rows.len()].copy_from_slice(&rows);
            assert_eq!(
                decode_event(fixture).unwrap(),
                EventMsg::ContSync {
                    kind,
                    cont,
                    reset,
                    slots,
                    count: rows.len() as u8,
                },
                "{name}: decode mismatch"
            );
            encode_event_cont_sync(kind, cont, reset, &rows, &mut buf).unwrap()
        }
        "v20_event_piece_repaired.bin" => {
            let (cx, cz, level, loc, row, healed, hp) = event_piece_repaired();
            assert_eq!(
                decode_event(fixture).unwrap(),
                EventMsg::PieceRepaired {
                    cx,
                    cz,
                    level,
                    loc,
                    row,
                    healed,
                    hp,
                },
                "{name}: decode mismatch"
            );
            encode_event_piece_repaired(cx, cz, level, loc, row, healed, hp, &mut buf).unwrap()
        }
        "v20_event_cont_close.bin" => {
            let (kind, cont, reset) = event_cont_close();
            assert_eq!(
                decode_event(fixture).unwrap(),
                EventMsg::ContSync {
                    kind,
                    cont,
                    reset,
                    slots: [InvSlot::default(); CONT_SYNC_BATCH],
                    count: 0,
                },
                "{name}: decode mismatch"
            );
            encode_event_cont_sync(kind, cont, reset, &[], &mut buf).unwrap()
        }
        other => panic!("unknown event fixture {other}"),
    };
    assert_eq!(&buf[..len], fixture, "{name}: bytes drifted");
}

/// The hotbar selector's wire domain is 0–5: the encoder refuses 6+ and
/// the decoder refuses a 6 or 7 someone forges into the 3-bit field.
#[test]
fn test_input_sel_domain_is_enforced() {
    let mut dg = InputDatagram::new(1, 0, 9);
    dg.push(InputFrame {
        seq: 3,
        sel: 6,
        ..InputFrame::default()
    })
    .unwrap();
    let mut buf = [0u8; DATAGRAM_BUDGET_BYTES];
    assert_eq!(encode_input(&dg, &mut buf), Err(WireError::Range));

    // Forge sel = 7 on the wire: a one-frame datagram's sel field is its
    // last 3 payload bits. Encode a valid frame, then read the layout
    // back with the sel bits forced high via a re-encoded bit image.
    let mut ok = InputDatagram::new(1, 0, 9);
    ok.push(InputFrame {
        seq: 3,
        sel: 5,
        ..InputFrame::default()
    })
    .unwrap();
    let len = encode_input(&ok, &mut buf).unwrap();
    assert_eq!(decode_input(&buf[..len]).unwrap(), ok);
    // sel rides bits 151..154 (3+16+32+32+4 header, 16 first_seq, 48 frame
    // bits before it; the writer is LSB-first within each byte): force all
    // three high — sel 7 — and decode must refuse.
    let sel_bit = 3 + 16 + 32 + 32 + 4 + 16 + 48;
    for b in 0..3 {
        let bit = sel_bit + b;
        buf[bit / 8] |= 1 << (bit % 8);
    }
    assert_eq!(decode_input(&buf[..len]), Err(WireError::Malformed));
}

/// The upgrade action's material domain is 0–2 (wood/stone/metal): the
/// encoder refuses 3+ and the decoder refuses the 3 someone forges into
/// the 2-bit field. Without this the sim would be handed a material no
/// rung answers to — refused there too, but a wire that admits nonsense
/// is the wire's bug, not the sim's.
#[test]
fn test_upgrade_material_domain_is_enforced() {
    let (cx, cz, level, loc, _) = action_upgrade();
    let mut buf = [0u8; 64];
    assert_eq!(
        encode_action_upgrade(cx, cz, level, loc, 3, &mut buf),
        Err(WireError::Range)
    );

    // Forge material = 3 on the wire: the field is the frame's last two
    // payload bits (3 kind + 4 subtype + 10 + 10 + 3 + 2 address bits
    // before it, LSB-first within each byte). The subtype width moved
    // 3 → 4 in wire v12 (the loot action was the ninth).
    let len = encode_action_upgrade(cx, cz, level, loc, 1, &mut buf).unwrap();
    assert!(matches!(
        decode_action(&buf[..len]).unwrap(),
        ActionMsg::Upgrade { material: 1, .. }
    ));
    let mat_bit = 3 + 4 + 10 + 10 + 3 + 2;
    for b in 0..2 {
        let bit = mat_bit + b;
        buf[bit / 8] |= 1 << (bit % 8);
    }
    assert_eq!(decode_action(&buf[..len]), Err(WireError::Malformed));
}

/// Chat is the one field a player authors, so it is the one field a
/// player can forge. Both edges hold the same rule: the encoder refuses
/// to build a line the decoder would refuse, and the decoder refuses a
/// hand-built frame that skipped the encoder — including a line that is
/// merely *non-canonical* (untrimmed), so one line has exactly one
/// encoding and this fixture means what it says.
#[test]
fn test_chat_text_domain_is_enforced() {
    let mut buf = [0u8; 64];
    // Encoder side.
    for bad in [
        &b""[..],
        &b"   "[..],
        &b"line\nbreak"[..],
        &b"esc\x1b"[..],
        &[0xff, 0xfe][..],
        &[b'a'; protocol::CHAT_MAX_BYTES + 1][..],
    ] {
        assert_eq!(
            encode_chat(bad, false, &mut buf),
            Err(WireError::Range),
            "encoder accepted {bad:?}"
        );
    }
    assert!(encode_chat(&[b'a'; protocol::CHAT_MAX_BYTES], true, &mut buf).is_ok());

    // Decoder side: hand-built frames the encoder cannot produce.
    let forge = |global: bool, raw: &[u8], claim_len: u32, out: &mut [u8]| -> usize {
        let mut w = protocol::bits::BitWriter::new(out);
        w.write(KIND_CHAT, 3).unwrap();
        w.write_bit(global).unwrap();
        w.write(claim_len, 6).unwrap();
        for &b in raw {
            w.write(b as u32, 8).unwrap();
        }
        w.finish()
    };
    let mut wide = [0u8; 96];
    for (raw, claim) in [
        (&b" untrimmed"[..], 10u32), // non-canonical
        (&b"nul\0byte"[..], 8),      // control char
        (&[0xc3, 0x28][..], 2),      // invalid UTF-8
        (&[b'a'; 63][..], 63),       // length past CHAT_MAX_BYTES
        (&b""[..], 0),               // empty
    ] {
        let n = forge(false, raw, claim, &mut wide);
        assert_eq!(
            decode_chat(&wide[..n]),
            Err(WireError::Malformed),
            "decoder accepted a forged {raw:?}"
        );
    }
    // And the same rule on the relay: a forged S→C line is refused too —
    // the same message the encoder produces, rebuilt with a bell where
    // its first letter was.
    let (from, global, text) = event_chat();
    let n = encode_event_chat(from, global, &text, &mut wide).unwrap();
    let mut broken = wide;
    let mut w = protocol::bits::BitWriter::new(&mut broken[..n]);
    w.write(protocol::KIND_EVENT, 3).unwrap();
    w.write(23, 5).unwrap();
    w.write(from, 32).unwrap();
    w.write_bit(global).unwrap();
    w.write(text.len() as u32, 6).unwrap();
    w.write(0x07, 8).unwrap(); // a bell where the first letter was
    for &b in &text.as_bytes()[1..] {
        w.write(b as u32, 8).unwrap();
    }
    let m = w.finish();
    assert_eq!(decode_event(&broken[..m]), Err(WireError::Malformed));
}

/// The delta packet earns its keep: the same content absolute-encoded
/// must be strictly bigger, or delta encoding silently stopped engaging.
#[test]
fn test_delta_actually_compresses() {
    let case = snapshot_delta();
    let (_, delta_len) = encode_case(&case);
    let mut absolute = snapshot_delta();
    absolute.baseline_len = 0;
    absolute.header.baseline_age = 0;
    let (_, abs_len) = encode_case(&absolute);
    assert!(
        delta_len < abs_len,
        "delta ({delta_len} B) not smaller than absolute ({abs_len} B)"
    );
}

/// The welcome's `dev` bit survives a roundtrip BOTH ways. The golden
/// fixture pins the true case (a false bit is indistinguishable from the
/// zero padding it sits in); this pins the false case, and that the two
/// encodings actually differ — a `dev` the encoder dropped on the floor
/// would ship every public shard the dev affordances.
#[test]
fn test_welcome_dev_bit_roundtrips() {
    let mut on = [0u8; 64];
    let mut off = [0u8; 64];
    let dev_on = welcome();
    assert!(dev_on.dev, "the golden welcome must pin the true case");
    let dev_off = protocol::Welcome {
        dev: false,
        ..dev_on
    };
    let n_on = encode_welcome(&dev_on, &mut on).unwrap();
    let n_off = encode_welcome(&dev_off, &mut off).unwrap();
    assert_eq!(decode_welcome(&on[..n_on]).unwrap(), dev_on);
    assert_eq!(decode_welcome(&off[..n_off]).unwrap(), dev_off);
    assert_ne!(
        &on[..n_on],
        &off[..n_off],
        "dev true and false encode identically — the bit is not on the wire"
    );
}

/// Worst-case shape at the interest-set cap fits the datagram budget
/// (DESIGN.md §5.3/§5.5: 1100 B, shed-not-fragment).
#[test]
fn test_snapshot_cap_within_budget() {
    let case = snapshot_cap();
    let (_, len) = encode_case(&case);
    assert!(
        len <= DATAGRAM_BUDGET_BYTES,
        "cap snapshot {len} B blows the {DATAGRAM_BUDGET_BYTES} B budget"
    );
    // And the worst input datagram is nowhere near the budget either.
    let mut buf = [0u8; DATAGRAM_BUDGET_BYTES];
    let len = encode_input(&input_full(), &mut buf).unwrap();
    assert!(len <= DATAGRAM_BUDGET_BYTES);
}

/// Decode is total: every single-bit corruption of every fixture, every
/// truncation, and 10k pseudorandom buffers must return — Ok or Err, never
/// a panic (the server decodes client-driven bytes; a panic is a remote
/// crash). The decoded value on corruption is unspecified; not panicking
/// and not unbounded work is the contract.
#[test]
fn test_decode_is_total() {
    let delta_case = snapshot_delta();
    let try_both = |bytes: &[u8]| {
        let _ = decode_input(bytes);
        let _ = decode_snapshot(bytes, &[]);
        let _ = decode_snapshot(bytes, delta_case.baseline());
        let _ = decode_hello(bytes);
        let _ = decode_welcome(bytes);
        let _ = decode_refuse(bytes);
        let _ = decode_event(bytes);
        let _ = decode_action(bytes);
        let _ = decode_chat(bytes);
    };
    let mut scratch = [0u8; DATAGRAM_BUDGET_BYTES];
    for fixture in GOLDEN {
        for cut in 0..fixture.len() {
            try_both(&fixture[..cut]);
        }
        for bit in 0..fixture.len() * 8 {
            let s = &mut scratch[..fixture.len()];
            s.copy_from_slice(fixture);
            s[bit / 8] ^= 1 << (bit % 8);
            try_both(s);
        }
    }
    let mut rng = Pcg32::new(0x0047_4154_4553, 15);
    for _ in 0..10_000 {
        let len = rng.next_bounded(DATAGRAM_BUDGET_BYTES as u32 + 1) as usize;
        let s = &mut scratch[..len];
        for b in s.iter_mut() {
            *b = rng.next_bounded(256) as u8;
        }
        try_both(s);
    }
}
