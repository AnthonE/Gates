//! `test_protocol_golden` (DESIGN.md §12, CLAUDE.md wall 6): every packet
//! type's encoding is byte-stable against checked-in fixtures, decodes
//! back to exactly what was encoded, and the decoder is total — arbitrary
//! corruption never panics it. A diff against a fixture without a
//! `PROTO_VER` bump in the same commit is the wire drifting by accident.

use protocol::goldens::{
    action_access, action_access_crew, action_cancel, action_consume, action_container,
    action_container_close, action_container_world, action_craft, action_demolish, action_deploy,
    action_feed, action_move, action_move_box, action_move_world, action_place,
    action_repair_deploy, action_repair_piece, action_research, action_respawn,
    action_throw_deploy, action_throw_piece, action_unlock, action_upgrade, action_use, auth,
    challenge, chat, event_auth, event_bag_dropped, event_bag_removed, event_bag_sync, event_bags,
    event_build_refused, event_catalog, event_charge_placed_deploy, event_charge_placed_piece,
    event_chat, event_consume_refused, event_consumed, event_cont_close, event_cont_sync,
    event_cont_sync_world, event_craft_done, event_craft_q, event_craft_refused, event_death,
    event_deploy_defs, event_deploy_placed, event_deploy_refused, event_deploy_sync, event_door,
    event_drank, event_gather, event_gather_refused, event_health, event_hit, event_impact,
    event_inv, event_knock, event_known, event_move_refused, event_moved, event_oven_lit,
    event_oven_out, event_piece_defs, event_piece_placed, event_piece_repaired_deploy,
    event_piece_repaired_piece, event_piece_sync, event_recipes, event_removed, event_research,
    event_research_refused, event_research_rows, event_respawn, event_shot, event_slot_change,
    event_slot_sync, event_stock, event_struct_hit_deploy, event_struct_hit_piece, event_swing,
    event_vitals, event_weak_mark, hello, input_acks_only, input_full, refuse_full, snapshot_cap,
    snapshot_delta, snapshot_keyframe, welcome, SnapshotCase, FIXTURES,
};
use protocol::{
    decode_action, decode_auth, decode_challenge, decode_chat, decode_event, decode_hello,
    decode_input, decode_refuse, decode_snapshot, decode_welcome, encode_action_access,
    encode_action_cancel, encode_action_consume, encode_action_container, encode_action_craft,
    encode_action_demolish, encode_action_deploy, encode_action_drink, encode_action_feed,
    encode_action_loot, encode_action_move, encode_action_place, encode_action_repair,
    encode_action_research, encode_action_respawn, encode_action_throw, encode_action_unlock,
    encode_action_upgrade, encode_action_use, encode_auth, encode_challenge, encode_chat,
    encode_event_auth, encode_event_bag_dropped, encode_event_bag_removed, encode_event_bag_sync,
    encode_event_bags, encode_event_build_refused, encode_event_catalog,
    encode_event_charge_placed, encode_event_chat, encode_event_consume_refused,
    encode_event_consumed, encode_event_cont_sync, encode_event_craft_done, encode_event_craft_q,
    encode_event_craft_refused, encode_event_death, encode_event_deploy_defs,
    encode_event_deploy_placed, encode_event_deploy_refused, encode_event_deploy_sync,
    encode_event_door, encode_event_drank, encode_event_gather, encode_event_gather_refused,
    encode_event_health, encode_event_hit, encode_event_impact, encode_event_inv,
    encode_event_knock, encode_event_known, encode_event_move_refused, encode_event_moved,
    encode_event_oven, encode_event_piece_defs, encode_event_piece_placed,
    encode_event_piece_repaired, encode_event_piece_sync, encode_event_recipes,
    encode_event_removed, encode_event_research, encode_event_research_refused,
    encode_event_research_rows, encode_event_respawn, encode_event_shot, encode_event_slot_change,
    encode_event_slot_sync, encode_event_stock, encode_event_struct_hit, encode_event_swing,
    encode_event_vitals, encode_event_weak_mark, encode_hello, encode_input, encode_refuse,
    encode_snapshot, encode_welcome, peek_kind, ActionMsg, ChatMsg, EventMsg, InputDatagram,
    InvSlot, WireError, BAG_SYNC_BATCH, CATALOG_BATCH, CONT_SYNC_BATCH, DEPLOY_SYNC_BATCH,
    KIND_ACTION, KIND_AUTH, KIND_BITS, KIND_CHALLENGE, KIND_CHAT, KIND_EVENT, KIND_HELLO,
    KIND_INPUT, KIND_REFUSE, KIND_SNAPSHOT, KIND_WELCOME, MAX_EVENT_MSG_BYTES, PIECE_DEFS_BATCH,
    PIECE_SYNC_BATCH, RECIPE_BATCH, RESEARCH_BATCH, SLOT_SYNC_BATCH,
};
use sim_core::input::InputFrame;
use sim_core::limits::DATAGRAM_BUDGET_BYTES;
use sim_core::rng::Pcg32;

const GOLDEN: [&[u8]; 96] = [
    include_bytes!("golden/v47_input_acks_only.bin"),
    include_bytes!("golden/v47_input_full.bin"),
    include_bytes!("golden/v47_snapshot_keyframe.bin"),
    include_bytes!("golden/v47_snapshot_delta.bin"),
    include_bytes!("golden/v47_snapshot_cap.bin"),
    include_bytes!("golden/v47_hello.bin"),
    include_bytes!("golden/v47_welcome.bin"),
    include_bytes!("golden/v47_refuse_full.bin"),
    include_bytes!("golden/v47_event_gather.bin"),
    include_bytes!("golden/v47_event_inv.bin"),
    include_bytes!("golden/v47_event_slot_harvested.bin"),
    include_bytes!("golden/v47_event_slot_respawned.bin"),
    include_bytes!("golden/v47_event_slot_sync.bin"),
    include_bytes!("golden/v47_event_catalog.bin"),
    include_bytes!("golden/v47_event_weak_mark.bin"),
    include_bytes!("golden/v47_event_craft_q.bin"),
    include_bytes!("golden/v47_event_craft_done.bin"),
    include_bytes!("golden/v47_event_craft_refused.bin"),
    include_bytes!("golden/v47_event_recipes.bin"),
    include_bytes!("golden/v47_action_craft.bin"),
    include_bytes!("golden/v47_action_cancel.bin"),
    include_bytes!("golden/v47_action_place.bin"),
    include_bytes!("golden/v47_event_piece_placed.bin"),
    include_bytes!("golden/v47_event_piece_sync.bin"),
    include_bytes!("golden/v47_event_build_refused.bin"),
    include_bytes!("golden/v47_event_piece_defs.bin"),
    include_bytes!("golden/v47_action_deploy.bin"),
    include_bytes!("golden/v47_action_feed.bin"),
    include_bytes!("golden/v47_event_deploy_placed.bin"),
    include_bytes!("golden/v47_event_deploy_sync.bin"),
    include_bytes!("golden/v47_event_deploy_refused.bin"),
    include_bytes!("golden/v47_event_deploy_defs.bin"),
    include_bytes!("golden/v47_event_piece_removed.bin"),
    include_bytes!("golden/v47_event_deploy_removed.bin"),
    include_bytes!("golden/v47_event_stock.bin"),
    include_bytes!("golden/v47_action_use.bin"),
    include_bytes!("golden/v47_action_access.bin"),
    include_bytes!("golden/v47_event_door.bin"),
    include_bytes!("golden/v47_action_upgrade.bin"),
    include_bytes!("golden/v47_chat.bin"),
    include_bytes!("golden/v47_event_chat.bin"),
    include_bytes!("golden/v47_event_hit.bin"),
    include_bytes!("golden/v47_event_health.bin"),
    include_bytes!("golden/v47_event_death.bin"),
    include_bytes!("golden/v47_action_loot.bin"),
    include_bytes!("golden/v47_event_bag_dropped.bin"),
    include_bytes!("golden/v47_event_bag_sync.bin"),
    include_bytes!("golden/v47_event_bag_removed.bin"),
    include_bytes!("golden/v47_event_struct_hit_piece.bin"),
    include_bytes!("golden/v47_event_struct_hit_deploy.bin"),
    include_bytes!("golden/v47_event_vitals.bin"),
    include_bytes!("golden/v47_event_consumed.bin"),
    include_bytes!("golden/v47_event_consume_refused.bin"),
    include_bytes!("golden/v47_action_consume.bin"),
    include_bytes!("golden/v47_event_drank.bin"),
    include_bytes!("golden/v47_action_drink.bin"),
    include_bytes!("golden/v47_event_respawn.bin"),
    include_bytes!("golden/v47_action_respawn.bin"),
    include_bytes!("golden/v47_action_move.bin"),
    include_bytes!("golden/v47_event_moved.bin"),
    include_bytes!("golden/v47_event_move_refused.bin"),
    include_bytes!("golden/v47_action_move_box.bin"),
    include_bytes!("golden/v47_action_container.bin"),
    include_bytes!("golden/v47_action_container_close.bin"),
    include_bytes!("golden/v47_event_cont_sync.bin"),
    include_bytes!("golden/v47_event_cont_close.bin"),
    include_bytes!("golden/v47_action_repair_piece.bin"),
    include_bytes!("golden/v47_action_repair_deploy.bin"),
    include_bytes!("golden/v47_event_piece_repaired_piece.bin"),
    include_bytes!("golden/v47_event_piece_repaired_deploy.bin"),
    include_bytes!("golden/v47_action_throw_piece.bin"),
    include_bytes!("golden/v47_action_throw_deploy.bin"),
    include_bytes!("golden/v47_event_charge_placed_piece.bin"),
    include_bytes!("golden/v47_event_charge_placed_deploy.bin"),
    include_bytes!("golden/v47_challenge.bin"),
    include_bytes!("golden/v47_auth.bin"),
    include_bytes!("golden/v47_event_oven_lit.bin"),
    include_bytes!("golden/v47_event_oven_out.bin"),
    include_bytes!("golden/v47_event_knock.bin"),
    include_bytes!("golden/v47_event_auth.bin"),
    include_bytes!("golden/v47_action_access_crew.bin"),
    include_bytes!("golden/v47_action_demolish.bin"),
    include_bytes!("golden/v47_event_shot.bin"),
    include_bytes!("golden/v47_action_container_world.bin"),
    include_bytes!("golden/v47_action_move_world.bin"),
    include_bytes!("golden/v47_event_cont_sync_world.bin"),
    include_bytes!("golden/v47_action_unlock.bin"),
    include_bytes!("golden/v47_event_research_rows.bin"),
    include_bytes!("golden/v47_event_research.bin"),
    include_bytes!("golden/v47_event_research_refused.bin"),
    include_bytes!("golden/v47_event_known.bin"),
    include_bytes!("golden/v47_action_research.bin"),
    include_bytes!("golden/v47_event_gather_refused.bin"),
    include_bytes!("golden/v47_event_bags.bin"),
    include_bytes!("golden/v47_event_impact.bin"),
    include_bytes!("golden/v47_event_swing.bin"),
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
    golden_action(GOLDEN[67], FIXTURES[67]);
    golden_event(GOLDEN[68], FIXTURES[68]);
    golden_event(GOLDEN[69], FIXTURES[69]);
    golden_action(GOLDEN[70], FIXTURES[70]);
    golden_action(GOLDEN[71], FIXTURES[71]);
    golden_event(GOLDEN[72], FIXTURES[72]);
    golden_event(GOLDEN[73], FIXTURES[73]);
    // The handshake's two new messages (v27). Byte-stable and decode-exact
    // like every other, and they matter more than most: a signature is
    // computed over a nonce that arrived in one of them.
    golden_challenge(GOLDEN[74], FIXTURES[74]);
    golden_auth(GOLDEN[75], FIXTURES[75]);
    // The oven's two (v28), lit by a hand and out by itself — the actor is
    // what separates them and both bytes are pinned.
    golden_event(GOLDEN[76], FIXTURES[76]);
    golden_event(GOLDEN[77], FIXTURES[77]);
    // Lock v1's two new S→C lanes (v30), appended for the reason
    // `goldens::FIXTURES` states: the manifest is positional.
    golden_event(GOLDEN[78], FIXTURES[78]);
    golden_event(GOLDEN[79], FIXTURES[79]);
    // The crew half of `ACT_ACCESS` (v30): its own fixture, because the
    // op field picks which store the address means.
    golden_action(GOLDEN[80], FIXTURES[80]);
    // The seventeenth action (v30), the one that widened the lane.
    golden_action(GOLDEN[81], FIXTURES[81]);
    // The arrow becomes visible (v33): one broadcast carrying the shooter,
    // the aim and the round's ballistics.
    golden_event(GOLDEN[82], FIXTURES[82]);
    // World containers v0 (v37): the fourth container kind on its open,
    // its move and its sync. The value `3` was `Malformed` at both ends
    // through v36, so these three are the fixtures that say the two bits
    // are now fully spent — there is no forgeable kind left to test for.
    golden_action(GOLDEN[83], FIXTURES[83]);
    golden_action(GOLDEN[84], FIXTURES[84]);
    golden_event(GOLDEN[85], FIXTURES[85]);
    // The bench ladder + tech tree (v38): the unlock action, the
    // research-rows drip, and the research lane's three events pinned
    // for the first time since they landed at v32.
    golden_action(GOLDEN[86], FIXTURES[86]);
    for i in 87..91 {
        golden_event(GOLDEN[i], FIXTURES[i]);
    }
    // Every fixture in the manifest was dispatched above: the loop bounds
    // are hand-written, so a fixture added to `FIXTURES` and forgotten
    // here would be a golden nobody checks. This is the count that makes
    // that impossible to miss quietly.
    // The table verb's own action, kept through the 2026-08-15 integration:
    // the tree verb replaced this lane's other four fixtures and not this
    // one, and `encode_action_research` is still called by the client.
    golden_action(GOLDEN[91], FIXTURES[91]);
    // The gather refusal (v42) — item durability's wire window.
    golden_event(GOLDEN[92], FIXTURES[92]);
    // Bag choice v0 (v43): the own-fact bag list.
    golden_event(GOLDEN[93], FIXTURES[93]);
    // Surface marks v0 (v44): where an arrow stopped, and on what.
    golden_event(GOLDEN[94], FIXTURES[94]);
    golden_event(GOLDEN[95], FIXTURES[95]);
    assert_eq!(GOLDEN.len(), FIXTURES.len());
    assert_eq!(GOLDEN.len(), 96, "a new fixture must be dispatched above");
}

/// S→C challenge: byte-stable, kind-peekable, decode-exact.
fn golden_challenge(fixture: &[u8], name: &str) {
    let mut buf = [0u8; DATAGRAM_BUDGET_BYTES];
    let want = challenge();
    assert_eq!(peek_kind(fixture).unwrap(), KIND_CHALLENGE, "{name}");
    let len = encode_challenge(&want, &mut buf).expect("encodes");
    assert_eq!(&buf[..len], fixture, "{name}: bytes drifted");
    assert_eq!(decode_challenge(fixture).unwrap(), want, "{name}: decode");
}

/// C→S auth: byte-stable, kind-peekable, decode-exact.
fn golden_auth(fixture: &[u8], name: &str) {
    let mut buf = [0u8; DATAGRAM_BUDGET_BYTES];
    let want = auth();
    assert_eq!(peek_kind(fixture).unwrap(), KIND_AUTH, "{name}");
    let len = encode_auth(&want, &mut buf).expect("encodes");
    assert_eq!(&buf[..len], fixture, "{name}: bytes drifted");
    assert_eq!(decode_auth(fixture).unwrap(), want, "{name}: decode");
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
        "v47_action_unlock.bin" => {
            let recipe = action_unlock();
            assert_eq!(
                decode_action(fixture).unwrap(),
                ActionMsg::Unlock { recipe },
                "{name}: decode mismatch"
            );
            encode_action_unlock(recipe, &mut buf).unwrap()
        }
        "v47_action_craft.bin" => {
            let (recipe, count) = action_craft();
            assert_eq!(
                decode_action(fixture).unwrap(),
                ActionMsg::Craft { recipe, count },
                "{name}: decode mismatch"
            );
            encode_action_craft(recipe, count, &mut buf).unwrap()
        }
        "v47_action_cancel.bin" => {
            let index = action_cancel();
            assert_eq!(
                decode_action(fixture).unwrap(),
                ActionMsg::CraftCancel { index },
                "{name}: decode mismatch"
            );
            encode_action_cancel(index, &mut buf).unwrap()
        }
        "v47_action_place.bin" => {
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
        "v47_action_deploy.bin" => {
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
        "v47_action_feed.bin" => {
            let (cx, cz, level) = action_feed();
            assert_eq!(
                decode_action(fixture).unwrap(),
                ActionMsg::Feed { cx, cz, level },
                "{name}: decode mismatch"
            );
            encode_action_feed(cx, cz, level, &mut buf).unwrap()
        }
        "v47_action_use.bin" => {
            let (cx, cz, level, loc) = action_use();
            assert_eq!(
                decode_action(fixture).unwrap(),
                ActionMsg::Use { cx, cz, level, loc },
                "{name}: decode mismatch"
            );
            encode_action_use(cx, cz, level, loc, &mut buf).unwrap()
        }
        "v47_action_demolish.bin" => {
            let (deploy, cx, cz, level, loc) = action_demolish();
            assert_eq!(
                decode_action(fixture).unwrap(),
                ActionMsg::Demolish {
                    deploy,
                    cx,
                    cz,
                    level,
                    loc,
                },
                "{name}: decode mismatch"
            );
            encode_action_demolish(deploy, cx, cz, level, loc, &mut buf).unwrap()
        }
        "v47_action_access_crew.bin" => {
            let (cx, cz, level, loc, op, code) = action_access_crew();
            assert_eq!(
                decode_action(fixture).unwrap(),
                ActionMsg::Access {
                    cx,
                    cz,
                    level,
                    loc,
                    op,
                    code,
                },
                "{name}: decode mismatch"
            );
            encode_action_access(cx, cz, level, loc, op, code, &mut buf).unwrap()
        }
        "v47_action_access.bin" => {
            let (cx, cz, level, loc, op, code) = action_access();
            assert_eq!(
                decode_action(fixture).unwrap(),
                ActionMsg::Access {
                    cx,
                    cz,
                    level,
                    loc,
                    op,
                    code,
                },
                "{name}: decode mismatch"
            );
            encode_action_access(cx, cz, level, loc, op, code, &mut buf).unwrap()
        }
        "v47_action_upgrade.bin" => {
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
        "v47_action_loot.bin" => {
            assert_eq!(
                decode_action(fixture).unwrap(),
                ActionMsg::Loot,
                "{name}: decode mismatch"
            );
            encode_action_loot(&mut buf).unwrap()
        }
        "v47_action_consume.bin" => {
            let slot = action_consume();
            assert_eq!(
                decode_action(fixture).unwrap(),
                ActionMsg::Consume { slot },
                "{name}: decode mismatch"
            );
            encode_action_consume(slot, &mut buf).unwrap()
        }
        "v47_action_drink.bin" => {
            assert_eq!(
                decode_action(fixture).unwrap(),
                ActionMsg::Drink,
                "{name}: decode mismatch"
            );
            encode_action_drink(&mut buf).unwrap()
        }
        "v47_action_respawn.bin" => {
            let on_bag = action_respawn();
            assert_eq!(
                decode_action(fixture).unwrap(),
                ActionMsg::Respawn { on_bag },
                "{name}: decode mismatch"
            );
            encode_action_respawn(on_bag, &mut buf).unwrap()
        }
        "v47_action_move.bin" => {
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
        "v47_action_move_box.bin" | "v47_action_move_world.bin" => {
            let (cont, from_kind, from_slot, to_kind, to_slot, count) =
                if name == "v47_action_move_box.bin" {
                    action_move_box()
                } else {
                    action_move_world()
                };
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
        "v47_action_repair_piece.bin" | "v47_action_repair_deploy.bin" => {
            let (deploy, cx, cz, level, loc) = if name == "v47_action_repair_piece.bin" {
                action_repair_piece()
            } else {
                action_repair_deploy()
            };
            assert_eq!(
                decode_action(fixture).unwrap(),
                ActionMsg::Repair {
                    deploy,
                    cx,
                    cz,
                    level,
                    loc
                },
                "{name}: decode mismatch"
            );
            encode_action_repair(deploy, cx, cz, level, loc, &mut buf).unwrap()
        }
        "v47_action_throw_piece.bin" | "v47_action_throw_deploy.bin" => {
            let (deploy, cx, cz, level, loc) = if name == "v47_action_throw_piece.bin" {
                action_throw_piece()
            } else {
                action_throw_deploy()
            };
            assert_eq!(
                decode_action(fixture).unwrap(),
                ActionMsg::Throw {
                    deploy,
                    cx,
                    cz,
                    level,
                    loc
                },
                "{name}: decode mismatch"
            );
            encode_action_throw(deploy, cx, cz, level, loc, &mut buf).unwrap()
        }
        "v47_action_container.bin"
        | "v47_action_container_close.bin"
        | "v47_action_container_world.bin" => {
            let (kind, cont) = match name {
                "v47_action_container.bin" => action_container(),
                "v47_action_container_world.bin" => action_container_world(),
                _ => action_container_close(),
            };
            assert_eq!(
                decode_action(fixture).unwrap(),
                ActionMsg::Container { kind, cont },
                "{name}: decode mismatch"
            );
            encode_action_container(kind, cont, &mut buf).unwrap()
        }
        // Research (v32), pinned at v37.
        "v47_action_research.bin" => {
            let slot = action_research();
            assert_eq!(
                decode_action(fixture).unwrap(),
                ActionMsg::Research { slot },
                "{name}: decode mismatch"
            );
            encode_action_research(slot, &mut buf).unwrap()
        }
        other => panic!("unknown action fixture {other}"),
    };
    assert_eq!(&buf[..len], fixture, "{name}: bytes drifted");
}

/// The handshake trio: byte-stable, kind-peekable, decode-exact.
fn golden_stream(fixture: &[u8], name: &str) {
    let mut buf = [0u8; 64];
    match name {
        "v47_hello.bin" => {
            let len = encode_hello(&hello(), &mut buf).unwrap();
            assert_eq!(&buf[..len], fixture, "{name}: bytes drifted");
            assert_eq!(peek_kind(fixture).unwrap(), KIND_HELLO);
            assert_eq!(decode_hello(fixture).unwrap(), hello());
        }
        "v47_welcome.bin" => {
            let len = encode_welcome(&welcome(), &mut buf).unwrap();
            assert_eq!(&buf[..len], fixture, "{name}: bytes drifted");
            assert_eq!(peek_kind(fixture).unwrap(), KIND_WELCOME);
            assert_eq!(decode_welcome(fixture).unwrap(), welcome());
        }
        "v47_refuse_full.bin" => {
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
        "v47_event_research_rows.bin" => {
            let rc = event_research_rows();
            match decode_event(fixture).unwrap() {
                EventMsg::ResearchRows {
                    total,
                    first,
                    count,
                    coin,
                    rows,
                } => {
                    assert_eq!(total as usize, rc.row_count as usize, "{name}: total");
                    assert_eq!(first, 0, "{name}: first");
                    assert_eq!(count as usize, RESEARCH_BATCH, "{name}: count");
                    assert_eq!(coin, rc.coin, "{name}: coin");
                    assert_eq!(rows[..], rc.rows[..RESEARCH_BATCH], "{name}: rows");
                }
                other => panic!("{name}: wrong variant {other:?}"),
            }
            let (len, took) = encode_event_research_rows(&rc, 0, &mut buf).unwrap();
            assert_eq!(took, RESEARCH_BATCH, "{name}: batch size");
            len
        }
        "v47_event_research.bin" => {
            let (recipe, cost) = event_research();
            assert_eq!(
                decode_event(fixture).unwrap(),
                EventMsg::Research { recipe, cost },
                "{name}: decode mismatch"
            );
            encode_event_research(recipe, cost, &mut buf).unwrap()
        }
        "v47_event_research_refused.bin" => {
            let reason = event_research_refused();
            assert_eq!(
                decode_event(fixture).unwrap(),
                EventMsg::ResearchRefused { reason },
                "{name}: decode mismatch"
            );
            encode_event_research_refused(reason, &mut buf).unwrap()
        }
        "v47_event_known.bin" => {
            let mask = event_known();
            assert_eq!(
                decode_event(fixture).unwrap(),
                EventMsg::Known { mask },
                "{name}: decode mismatch"
            );
            encode_event_known(mask, &mut buf).unwrap()
        }
        "v47_event_bags.bin" => {
            let (bags, n) = event_bags();
            assert_eq!(
                decode_event(fixture).unwrap(),
                EventMsg::Bags {
                    bags,
                    count: n as u8
                },
                "{name}: decode mismatch"
            );
            encode_event_bags(&bags[..n], &mut buf).unwrap()
        }
        "v47_event_gather.bin" => {
            let (item, added) = event_gather();
            assert_eq!(
                decode_event(fixture).unwrap(),
                EventMsg::Gather { item, added },
                "{name}: decode mismatch"
            );
            encode_event_gather(item, added, &mut buf).unwrap()
        }
        "v47_event_gather_refused.bin" => {
            let (item, reason) = event_gather_refused();
            assert_eq!(
                decode_event(fixture).unwrap(),
                EventMsg::GatherRefused { item, reason },
                "{name}: decode mismatch"
            );
            encode_event_gather_refused(item, reason, &mut buf).unwrap()
        }
        "v47_event_inv.bin" => {
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
        "v47_event_slot_harvested.bin" | "v47_event_slot_respawned.bin" => {
            let harvested = name == "v47_event_slot_harvested.bin";
            let (cx, cz) = event_slot_change();
            let want = if harvested {
                EventMsg::SlotHarvested { cx, cz }
            } else {
                EventMsg::SlotRespawned { cx, cz }
            };
            assert_eq!(decode_event(fixture).unwrap(), want, "{name}");
            encode_event_slot_change(harvested, cx, cz, &mut buf).unwrap()
        }
        "v47_event_slot_sync.bin" => {
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
        "v47_event_catalog.bin" => {
            let cat = event_catalog();
            match decode_event(fixture).unwrap() {
                EventMsg::Catalog {
                    total,
                    first,
                    count,
                    names,
                    lens,
                    cond_max,
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
                        assert_eq!(
                            cond_max[i],
                            cat.cond_max(i),
                            "{name}: condition ceiling {i} mismatch (v46 column)"
                        );
                    }
                }
                other => panic!("{name}: wrong variant {other:?}"),
            }
            let (len, took) = encode_event_catalog(&cat, 0, &mut buf).unwrap();
            assert_eq!(took, CATALOG_BATCH, "{name}: batch shrank");
            len
        }
        "v47_event_weak_mark.bin" => {
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
        "v47_event_craft_q.bin" => {
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
        "v47_event_craft_done.bin" => {
            let (item, added) = event_craft_done();
            assert_eq!(
                decode_event(fixture).unwrap(),
                EventMsg::CraftDone { item, added },
                "{name}: decode mismatch"
            );
            encode_event_craft_done(item, added, &mut buf).unwrap()
        }
        "v47_event_craft_refused.bin" => {
            let reason = event_craft_refused();
            assert_eq!(
                decode_event(fixture).unwrap(),
                EventMsg::CraftRefused { reason },
                "{name}: decode mismatch"
            );
            encode_event_craft_refused(reason, &mut buf).unwrap()
        }
        "v47_event_recipes.bin" => {
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
        "v47_event_piece_placed.bin" => {
            let rec = event_piece_placed();
            assert_eq!(
                decode_event(fixture).unwrap(),
                EventMsg::PiecePlaced { rec },
                "{name}: decode mismatch"
            );
            encode_event_piece_placed(&rec, &mut buf).unwrap()
        }
        "v47_event_piece_sync.bin" => {
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
        "v47_event_build_refused.bin" => {
            let reason = event_build_refused();
            assert_eq!(
                decode_event(fixture).unwrap(),
                EventMsg::BuildRefused { reason },
                "{name}: decode mismatch"
            );
            encode_event_build_refused(reason, &mut buf).unwrap()
        }
        "v47_event_piece_defs.bin" => {
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
        "v47_event_deploy_placed.bin" => {
            let rec = event_deploy_placed();
            assert_eq!(
                decode_event(fixture).unwrap(),
                EventMsg::DeployPlaced { rec },
                "{name}: decode mismatch"
            );
            encode_event_deploy_placed(&rec, &mut buf).unwrap()
        }
        "v47_event_deploy_sync.bin" => {
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
        "v47_event_deploy_refused.bin" => {
            let reason = event_deploy_refused();
            assert_eq!(
                decode_event(fixture).unwrap(),
                EventMsg::DeployRefused { reason },
                "{name}: decode mismatch"
            );
            encode_event_deploy_refused(reason, &mut buf).unwrap()
        }
        "v47_event_deploy_defs.bin" => {
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
        "v47_event_piece_removed.bin" | "v47_event_deploy_removed.bin" => {
            let piece = name == "v47_event_piece_removed.bin";
            let (cx, cz, level, loc) = event_removed();
            let want = if piece {
                EventMsg::PieceRemoved { cx, cz, level, loc }
            } else {
                EventMsg::DeployRemoved { cx, cz, level, loc }
            };
            assert_eq!(decode_event(fixture).unwrap(), want, "{name}");
            encode_event_removed(piece, cx, cz, level, loc, &mut buf).unwrap()
        }
        "v47_event_stock.bin" => {
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
        "v47_event_door.bin" => {
            let (cx, cz, level, loc, open, locked, has_lock) = event_door();
            assert_eq!(
                decode_event(fixture).unwrap(),
                EventMsg::Door {
                    cx,
                    cz,
                    level,
                    loc,
                    open,
                    locked,
                    has_lock,
                },
                "{name}: decode mismatch"
            );
            encode_event_door(cx, cz, level, loc, open, locked, has_lock, &mut buf).unwrap()
        }
        "v47_event_knock.bin" => {
            let (cx, cz, level, loc, by) = event_knock();
            assert_eq!(
                decode_event(fixture).unwrap(),
                EventMsg::Knock {
                    cx,
                    cz,
                    level,
                    loc,
                    by,
                },
                "{name}: decode mismatch"
            );
            encode_event_knock(cx, cz, level, loc, by, &mut buf).unwrap()
        }
        "v47_event_auth.bin" => {
            let (cx, cz, level, loc, grant) = event_auth();
            assert_eq!(
                decode_event(fixture).unwrap(),
                EventMsg::Auth {
                    cx,
                    cz,
                    level,
                    loc,
                    grant,
                },
                "{name}: decode mismatch"
            );
            encode_event_auth(cx, cz, level, loc, grant, &mut buf).unwrap()
        }
        "v47_event_chat.bin" => {
            let (from, global, text) = event_chat();
            assert_eq!(
                decode_event(fixture).unwrap(),
                EventMsg::Chat { from, global, text },
                "{name}: decode mismatch"
            );
            encode_event_chat(from, global, &text, &mut buf).unwrap()
        }
        "v47_event_hit.bin" => {
            let (victim, damage) = event_hit();
            assert_eq!(
                decode_event(fixture).unwrap(),
                EventMsg::Hit { victim, damage },
                "{name}: decode mismatch"
            );
            encode_event_hit(victim, damage, &mut buf).unwrap()
        }
        "v47_event_health.bin" => {
            let (hp, max) = event_health();
            assert_eq!(
                decode_event(fixture).unwrap(),
                EventMsg::Health { hp, max },
                "{name}: decode mismatch"
            );
            encode_event_health(hp, max, &mut buf).unwrap()
        }
        "v47_event_death.bin" => {
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
        "v47_event_respawn.bin" => {
            let on_bag = event_respawn();
            assert_eq!(
                decode_event(fixture).unwrap(),
                EventMsg::Respawn { on_bag },
                "{name}: decode mismatch"
            );
            encode_event_respawn(on_bag, &mut buf).unwrap()
        }
        "v47_event_moved.bin" => {
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
        "v47_event_move_refused.bin" => {
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
        "v47_event_bag_dropped.bin" => {
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
        "v47_event_bag_sync.bin" => {
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
        "v47_event_bag_removed.bin" => {
            let (id, why) = event_bag_removed();
            assert_eq!(
                decode_event(fixture).unwrap(),
                EventMsg::BagRemoved { id, why },
                "{name}: decode mismatch"
            );
            encode_event_bag_removed(id, why, &mut buf).unwrap()
        }
        n @ ("v47_event_struct_hit_piece.bin" | "v47_event_struct_hit_deploy.bin") => {
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
        "v47_event_vitals.bin" => {
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
        "v47_event_consumed.bin" => {
            let (item, slot) = event_consumed();
            assert_eq!(
                decode_event(fixture).unwrap(),
                EventMsg::Consumed { item, slot },
                "{name}: decode mismatch"
            );
            encode_event_consumed(item, slot, &mut buf).unwrap()
        }
        "v47_event_consume_refused.bin" => {
            let reason = event_consume_refused();
            assert_eq!(
                decode_event(fixture).unwrap(),
                EventMsg::ConsumeRefused { reason },
                "{name}: decode mismatch"
            );
            encode_event_consume_refused(reason, &mut buf).unwrap()
        }
        "v47_event_drank.bin" => {
            let (water, hp_cost) = event_drank();
            assert_eq!(
                decode_event(fixture).unwrap(),
                EventMsg::Drank { water, hp_cost },
                "{name}: decode mismatch"
            );
            encode_event_drank(water, hp_cost, &mut buf).unwrap()
        }
        "v47_event_cont_sync.bin" | "v47_event_cont_sync_world.bin" => {
            let (kind, cont, reset, rows) = if name == "v47_event_cont_sync.bin" {
                event_cont_sync()
            } else {
                event_cont_sync_world()
            };
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
        "v47_event_piece_repaired_piece.bin" | "v47_event_piece_repaired_deploy.bin" => {
            let (deploy, cx, cz, level, loc, row, healed, hp) =
                if name == "v47_event_piece_repaired_piece.bin" {
                    event_piece_repaired_piece()
                } else {
                    event_piece_repaired_deploy()
                };
            assert_eq!(
                decode_event(fixture).unwrap(),
                EventMsg::PieceRepaired {
                    deploy,
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
            encode_event_piece_repaired(deploy, cx, cz, level, loc, row, healed, hp, &mut buf)
                .unwrap()
        }
        "v47_event_charge_placed_piece.bin" | "v47_event_charge_placed_deploy.bin" => {
            let (deploy, cx, cz, level, loc, row, fuse) =
                if name == "v47_event_charge_placed_piece.bin" {
                    event_charge_placed_piece()
                } else {
                    event_charge_placed_deploy()
                };
            assert_eq!(
                decode_event(fixture).unwrap(),
                EventMsg::ChargePlaced {
                    deploy,
                    cx,
                    cz,
                    level,
                    loc,
                    row,
                    fuse,
                },
                "{name}: decode mismatch"
            );
            encode_event_charge_placed(deploy, cx, cz, level, loc, row, fuse, &mut buf).unwrap()
        }
        "v47_event_cont_close.bin" => {
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
        "v47_event_oven_lit.bin" | "v47_event_oven_out.bin" => {
            let (cx, cz, level, lit, by) = if name == "v47_event_oven_lit.bin" {
                event_oven_lit()
            } else {
                event_oven_out()
            };
            assert_eq!(
                decode_event(fixture).unwrap(),
                EventMsg::Oven {
                    cx,
                    cz,
                    level,
                    lit,
                    by,
                },
                "{name}: decode mismatch"
            );
            encode_event_oven(cx, cz, level, lit, by, &mut buf).unwrap()
        }
        "v47_event_shot.bin" => {
            let (shooter, yaw, pitch, speed_mmpt, drop_mmpt2) = event_shot();
            assert_eq!(
                decode_event(fixture).unwrap(),
                EventMsg::Shot {
                    shooter,
                    yaw,
                    pitch,
                    speed_mmpt,
                    drop_mmpt2,
                },
                "{name}: decode mismatch"
            );
            encode_event_shot(shooter, yaw, pitch, speed_mmpt, drop_mmpt2, &mut buf).unwrap()
        }
        "v47_event_impact.bin" => {
            let (qx, qy, qz, surf) = event_impact();
            assert_eq!(
                decode_event(fixture).unwrap(),
                EventMsg::Impact { qx, qy, qz, surf },
                "{name}: decode mismatch"
            );
            encode_event_impact(qx, qy, qz, surf, &mut buf).unwrap()
        }
        "v47_event_swing.bin" => {
            let swinger = event_swing();
            assert_eq!(
                decode_event(fixture).unwrap(),
                EventMsg::Swing { swinger },
                "{name}: decode mismatch"
            );
            encode_event_swing(swinger, &mut buf).unwrap()
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
    // Offsets derived from `KIND_BITS`, never typed: v27 widened it 3 → 4
    // and a literal here silently moved every field this test pokes.
    let sel_bit = KIND_BITS as usize + 16 + 32 + 32 + 4 + 16 + 48;
    for b in 0..3 {
        let bit = sel_bit + b;
        buf[bit / 8] |= 1 << (bit % 8);
    }
    assert_eq!(decode_input(&buf[..len]), Err(WireError::Malformed));
}

/// The upgrade action's material domain is **saturated** since wire v34,
/// and that is a fact about this field worth stating rather than a test
/// that got easier. Twig took code 0 and pushed wood/stone/metal to 1/2/3
/// (`build.rs` `MAT_TWIG`), so all four values a 2-bit field can hold now
/// name a real rung — which means **the wire can no longer refuse a bad
/// material, because there is no longer such a thing as a bad one here.**
///
/// Before v34 this test forged a 3 into the field and watched the decoder
/// reject it. There is nothing left to forge. What replaces it is the two
/// checks that still have teeth:
///
/// 1. The **encoder** refuses 4+, which is the value that would not fit
///    the field at all and would otherwise be written truncated — the
///    same silent-wrong-value class the forge test was aimed at.
/// 2. **Every one of the four rungs round-trips to itself**, which is
///    what pins the field's position and width now that no illegal
///    pattern can do it. Get the offset wrong and at least one rung comes
///    back as another.
///
/// The thing that watches the saturation is
/// `event::wire_domains::every_domain_fits_its_wire_field`: a fifth grade
/// would fail it on capacity rather than reaching a decoder that has no
/// room left to complain. That test is the guard this one used to be.
#[test]
fn test_upgrade_material_domain_is_enforced() {
    let (cx, cz, level, loc, _) = action_upgrade();
    let mut buf = [0u8; 64];
    // Over the field's capacity: refused before a byte is written.
    for over in [4u8, 5, 255] {
        assert_eq!(
            encode_action_upgrade(cx, cz, level, loc, over, &mut buf),
            Err(WireError::Range),
            "encoder accepted material {over}, which does not fit 2 bits"
        );
    }

    // And every live rung survives the round trip as itself.
    for rung in [
        sim_core::build::MAT_TWIG,
        sim_core::build::MAT_WOOD,
        sim_core::build::MAT_STONE,
        sim_core::build::MAT_METAL,
    ] {
        let len = encode_action_upgrade(cx, cz, level, loc, rung, &mut buf).unwrap();
        match decode_action(&buf[..len]).unwrap() {
            ActionMsg::Upgrade { material, .. } => {
                assert_eq!(material, rung, "rung {rung} decoded as {material}")
            }
            other => panic!("material {rung}: wrong variant {other:?}"),
        }
    }
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
        w.write(KIND_CHAT, KIND_BITS).unwrap();
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
    w.write(protocol::KIND_EVENT, KIND_BITS).unwrap();
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

/// The module header at the top of `lib.rs` is the only place either
/// datagram's layout is spelled out end to end, so it is what a reader
/// (or a reviewer costing a worst-case packet) reasons from — and the
/// byte-goldens above are structurally incapable of noticing when it goes
/// stale, because they compare bytes the same encoder produced. Nothing in
/// a fixture disagrees with a comment. This gate closes that: the header
/// must state the kind width the code actually writes.
///
/// Two assertions, and neither counts occurrences — a later lane adding a
/// third layout line must not turn this red for the wrong reason.
///
/// 1. Every `kind:<n>` the header spells equals [`KIND_BITS`], and the
///    header names `KIND_BITS` itself, so the prose points at the constant
///    rather than restating a number that can drift away from it. It
///    caught exactly that: the header said `kind:3` from v0 through v30,
///    four bits wrong since the v27 widening.
/// 2. The present-tense phrase "3-bit kind space" appears nowhere in
///    lib.rs. Past-tense siblings are deliberately untouched and must stay
///    ("the last code the 3-bit kind field had left", on `KIND_CHAT`):
///    CLAUDE.md is explicit that a historical citation is history and
///    stays, while a present-tense claim about what the wire *is* is a
///    statement this gate is entitled to check. `KIND_CHALLENGE = 8`
///    falsifies the present-tense reading twenty lines below where it was
///    written.
#[test]
fn test_module_header_states_the_real_kind_width() {
    const SRC: &str = include_str!("../src/lib.rs");
    // Every `//!` line in the file, not the leading run: a blank line or an
    // inner attribute inserted mid-header would truncate a `take_while` and
    // leave the second schema's width silently unchecked while the vacuity
    // guard below still saw one. `clippy.toml` disallows `String` in this
    // crate, so the lines are walked as borrowed slices, never joined.
    let header = || SRC.lines().filter(|l| l.starts_with("//!"));
    assert!(
        header().next().is_some(),
        "lib.rs has no `//!` module header to check"
    );

    let mut widths_seen = 0usize;
    let mut names_the_constant = false;
    for line in header() {
        names_the_constant |= line.contains("KIND_BITS");
        let mut rest = line;
        while let Some(at) = rest.find("kind:") {
            rest = &rest[at + "kind:".len()..];
            let end = rest
                .find(|c: char| !c.is_ascii_digit())
                .unwrap_or(rest.len());
            let Ok(n) = rest[..end].parse::<u32>() else {
                continue;
            };
            widths_seen += 1;
            assert_eq!(
                n, KIND_BITS,
                "module header says the wire's kind field is {n} bits; \
                 KIND_BITS is {KIND_BITS} and that is what every encoder writes"
            );
        }
    }
    assert!(
        widths_seen > 0,
        "the module header no longer spells a `kind:<n>` width — the layout \
         statement this gate checks has gone missing, which is not a fix"
    );
    assert!(
        names_the_constant,
        "the module header states a kind width without naming KIND_BITS — \
         point the prose at the constant so the next widening cannot leave \
         it behind, as v27's did"
    );

    assert!(
        !SRC.contains("3-bit kind space"),
        "lib.rs still claims a 3-bit kind space in the present tense; \
         KIND_CHALLENGE = 8 does not fit in three bits"
    );
}

/// **Every encoder this crate exports has a byte pin.**
///
/// The gate that was missing, found by asking a question nobody had asked:
/// `ACT_RESEARCH`, `SUB_RESEARCH`, `SUB_RESEARCH_REFUSED` and `SUB_KNOWN`
/// landed at v32 and crossed **five versions** — v33, v34, v35, v36, v37 —
/// with no fixture. `test_protocol_golden` was green the whole way, because
/// it checks the fixtures that exist and had no opinion about the ones that
/// do not. `FIXTURES` is a hand-written manifest and a hand-written manifest
/// is only as complete as the last person to remember it.
///
/// That is the third time this exact shape has been found in the research
/// lane alone: `bake_research` had no caller (so a live shard installed an
/// empty table), the three event subtypes had no `decode_event` arm (so
/// every frame decoded `Malformed`), and now the whole lane had no golden.
/// Each time the sim was correct and gated, and each time the thing that
/// was wrong was a *seam nothing enumerated*. So this gate does not check a
/// value — it checks a **set**, which is the class of defect that keeps
/// getting through (`CLAUDE.md`: a build step that enumerates by walking is
/// only as correct as the tidiness of the box it runs on).
///
/// Deliberately no exemption list. Every one of the 64 encoders is pinned
/// as of this commit, so an exemption would only ever be a place to hide
/// the next one.
#[test]
fn every_encoder_has_a_golden() {
    const EVENT_SRC: &str = include_str!("../src/event.rs");
    const LIB_SRC: &str = include_str!("../src/lib.rs");
    const GEN_SRC: &str = include_str!("../examples/gen_goldens.rs");

    let mut encoders: Vec<&str> = Vec::new();
    for (src, prefix) in [
        (EVENT_SRC, "pub fn encode_event_"),
        (LIB_SRC, "pub fn encode_action_"),
    ] {
        for line in src.lines() {
            let line = line.trim();
            if line.starts_with("//") {
                continue;
            }
            let Some(rest) = line.strip_prefix(prefix) else {
                continue;
            };
            let Some((tail, _)) = rest.split_once('(') else {
                continue;
            };
            if !tail.is_empty() && tail.chars().all(|c| c.is_ascii_lowercase() || c == '_') {
                // `prefix` minus the `pub fn `, plus the name's tail.
                let full = &line["pub fn ".len()..line.find('(').unwrap()];
                if !encoders.contains(&full) {
                    encoders.push(full);
                }
            }
        }
    }

    assert!(
        encoders.len() > 50,
        "the encoder scan found only {} — the `pub fn encode_…` shape \
         changed and this gate is now checking nothing",
        encoders.len()
    );

    /// Does `src` contain a **call** to `name` — the name followed
    /// immediately by `(`?
    ///
    /// The call site, not the `use` list: `gen_goldens` imports by bare
    /// name, so matching the name alone would count an import as a pin. The
    /// paren is also what separates `encode_event_research` from
    /// `encode_event_research_refused`, which is a real pair here. Written
    /// as a scan rather than `contains(&format!(…))` because `format!` is
    /// clippy-walled in this workspace and one allocation is not worth an
    /// `#[allow]`.
    fn calls(src: &str, name: &str) -> bool {
        let mut from = 0;
        while let Some(i) = src[from..].find(name) {
            let at = from + i;
            if src[at + name.len()..].starts_with('(') {
                return true;
            }
            from = at + name.len();
        }
        false
    }

    let mut unpinned: Vec<&str> = Vec::new();
    for name in &encoders {
        if !calls(GEN_SRC, name) {
            unpinned.push(name);
        }
    }

    assert!(
        unpinned.is_empty(),
        "{} encoder(s) produce bytes that no golden fixture pins, so their \
         layout can drift with `test_protocol_golden` green: {:?}. Add a \
         constructor to `goldens.rs`, a name to `FIXTURES`, a writer to \
         `gen_goldens.rs` and a dispatch line to this file — the research \
         lane crossed five versions unpinned because nothing asked.",
        unpinned.len(),
        unpinned
    );
}

/// The fixture whose name ends in `suffix`, by index into `GOLDEN`.
///
/// By suffix rather than by index or by full name, because both of those
/// go stale in a way that is silent: `FIXTURES` is positional so an index
/// re-points at another message the moment a name is inserted rather than
/// appended, and the full name carries the version, so every bump would
/// leave these gates looking for a file that no longer exists — and a
/// lookup that finds nothing is one `unwrap_or` away from a gate that
/// checks nothing.
fn fixture_index(suffix: &str) -> usize {
    FIXTURES
        .iter()
        .position(|n| n.ends_with(suffix))
        .unwrap_or_else(|| panic!("no fixture named `*{suffix}` — this gate is aimed at nothing"))
}

/// **The input golden exercises all eight button bits, both ways.**
///
/// `input_full` drew `buttons` from `next_bounded(4)` from v0 to v46, so
/// the one fixture that carries a button pinned bits 0–1 and nothing else:
/// `BTN_PRIMARY`, `BTN_JUMP` and every unmeant bit were outside the draw
/// (NOW.md §5c, found while landing jump). No byte on the wire was wrong —
/// the field is eight bits wide either way — but under that draw an
/// encoder that masked or reordered the high nibble would have regenerated
/// a green fixture, and `decode_input`'s doc names a silently narrowed
/// octet as the one wrong answer this codec must never give.
///
/// **What this gate is actually for, measured rather than assumed.** With
/// the draw wide, a masking encoder reddens `golden_input`'s round-trip on
/// its own (proven: mask `& BTN_MASK` at `encode_input`, regenerate — that
/// gate fails too). What nothing else catches is the *coverage* narrowing
/// back: narrow the draw and regenerate and `test_protocol_golden` is
/// green, because the fixture agrees with the constructor that produced
/// it — this gate is the only red one (also proven). So its job is to keep
/// the fixture wide enough for the other gates to have something to see.
///
/// It reads the **fixture bytes** rather than `input_full()` because that
/// costs nothing and is strictly more: a constructor scan would say the
/// draw is wide even if the bytes on disk were written by an older,
/// narrower one.
///
/// Both directions: every bit set somewhere (a mask) and every bit clear
/// somewhere (an encoder that jams a bit high, and the reason a fixture of
/// all-`0xFF` frames would not do).
#[test]
fn the_input_golden_fuzzes_the_whole_button_octet() {
    let dg = decode_input(GOLDEN[fixture_index("_input_full.bin")]).expect("the fixture decodes");
    let frames = dg.frames();
    assert!(
        frames.len() > 1,
        "the input fixture carries {} frame(s) — a one-frame fixture cannot \
         cover an octet and this gate would be asserting on a coin flip",
        frames.len()
    );

    let mut ones: u8 = 0;
    let mut zeros: u8 = 0;
    for f in frames {
        ones |= f.buttons;
        zeros |= !f.buttons;
    }
    assert_eq!(
        ones, 0xFF,
        "button bits {:#04x} are set in no frame of the input golden, so \
         nothing pins that the encoder writes them: widen `input_full`'s \
         draw, or set the missing bits on a named frame the way \
         `rng_entity` sets `sleeping`. The wire width is the target, not \
         `BTN_MASK` — bits 4–7 name no button and cross whole on purpose \
         (`decode_input`'s doc).",
        !ones
    );
    assert_eq!(
        zeros, 0xFF,
        "button bits {:#04x} are set in EVERY frame of the input golden — \
         an encoder that jams a bit high would read as covered",
        !zeros
    );
}

/// **Each build store's `loc` fuzz covers that store's whole domain.**
///
/// The question §5c's button hole raised about every other fuzzed field:
/// `event_deploy_sync` draws `loc` from `next_bounded(4)`, which looks
/// identical to the button defect and is not one. A deployable lives on
/// the plane and the two straight edges — `loc_max(true)` — so four IS the
/// domain and a wider draw would make the fixture unencodable. The piece
/// store gained six more locs at v40 and its fixture draws all ten.
///
/// Written as coverage of a domain **derived from `sim_core::build`**
/// rather than as a bound copied from the generator, so it stays true if a
/// store gains a loc: the day `LOC_DIAG_B` is not the top, this goes red
/// on the fixture that no longer reaches it rather than on the constant.
#[test]
fn the_loc_fuzz_covers_each_stores_whole_domain() {
    use sim_core::build::{LOC_DIAG_B, LOC_EDGE_ZLO};

    let mut seen = [false; 16];
    match decode_event(GOLDEN[fixture_index("_event_piece_sync.bin")]).expect("piece sync decodes")
    {
        EventMsg::PieceSync { recs, count, .. } => {
            for rec in recs.iter().take(count as usize) {
                seen[rec.loc as usize] = true;
            }
        }
        other => panic!("the piece-sync fixture decoded as {other:?}"),
    }
    for loc in 0..=LOC_DIAG_B {
        assert!(
            seen[loc as usize],
            "no piece in the sync golden sits at loc {loc}, so its bytes are \
             pinned by nothing — the piece store addresses 0..={LOC_DIAG_B}"
        );
    }
    for (loc, hit) in seen.iter().enumerate().skip(LOC_DIAG_B as usize + 1) {
        assert!(
            !*hit,
            "the piece-sync golden carries loc {loc}, which no store can \
             address — the fixture is pinning a forgery as if it were legal"
        );
    }

    let mut seen = [false; 16];
    match decode_event(GOLDEN[fixture_index("_event_deploy_sync.bin")])
        .expect("deploy sync decodes")
    {
        EventMsg::DeploySync { recs, count, .. } => {
            for rec in recs.iter().take(count as usize) {
                seen[rec.loc as usize] = true;
            }
        }
        other => panic!("the deploy-sync fixture decoded as {other:?}"),
    }
    for loc in 0..=LOC_EDGE_ZLO {
        assert!(
            seen[loc as usize],
            "no deployable in the sync golden sits at loc {loc} — the \
             deploy store addresses 0..={LOC_EDGE_ZLO} and every one of \
             those values should reach the bytes"
        );
    }
    for (loc, hit) in seen.iter().enumerate().skip(LOC_EDGE_ZLO as usize + 1) {
        assert!(
            !*hit,
            "the deploy-sync golden carries loc {loc}, past the deploy \
             store's own top ({LOC_EDGE_ZLO}) — the encoder is supposed to \
             refuse that, so a fixture holding it means the refusal moved"
        );
    }
}
