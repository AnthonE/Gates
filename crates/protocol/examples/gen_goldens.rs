//! Regenerate the golden fixtures from `protocol::goldens` — run only
//! alongside a `PROTO_VER` bump, and commit the new bytes with it
//! (CLAUDE.md wall 6): `cargo run -p protocol --example gen_goldens`.

// Host-side fixture writer: printing and fs are its job. The walls ban
// them in hot-path code; an example binary is not hot-path code.
#![allow(clippy::disallowed_macros)]

use protocol::encode_action_consume;
use protocol::{
    encode_action_cancel, encode_action_craft, encode_action_deploy, encode_action_feed,
    encode_action_lock, encode_action_loot, encode_action_place, encode_action_upgrade,
    encode_action_use, encode_chat, encode_event_bag_dropped, encode_event_bag_removed,
    encode_event_bag_sync, encode_event_build_refused, encode_event_catalog, encode_event_chat,
    encode_event_consume_refused, encode_event_consumed, encode_event_craft_done,
    encode_event_craft_q, encode_event_craft_refused, encode_event_death, encode_event_deploy_defs,
    encode_event_deploy_placed, encode_event_deploy_refused, encode_event_deploy_sync,
    encode_event_door, encode_event_gather, encode_event_health, encode_event_hit,
    encode_event_inv, encode_event_piece_defs, encode_event_piece_placed, encode_event_piece_sync,
    encode_event_recipes, encode_event_removed, encode_event_slot_change, encode_event_slot_sync,
    encode_event_stock, encode_event_struct_hit, encode_event_vitals, encode_event_weak_mark,
    encode_hello, encode_input, encode_refuse, encode_snapshot, encode_welcome, goldens,
};
use sim_core::limits::DATAGRAM_BUDGET_BYTES;

fn write_fixture(name: &str, bytes: &[u8]) {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden");
    std::fs::create_dir_all(dir).expect("create tests/golden");
    let path = std::format!("{dir}/{name}");
    std::fs::write(&path, bytes).expect("write fixture");
    println!("wrote {path} ({} B)", bytes.len());
}

fn main() {
    let mut buf = [0u8; DATAGRAM_BUDGET_BYTES];

    let len = encode_input(&goldens::input_acks_only(), &mut buf).unwrap();
    write_fixture(goldens::FIXTURES[0], &buf[..len]);

    let len = encode_input(&goldens::input_full(), &mut buf).unwrap();
    write_fixture(goldens::FIXTURES[1], &buf[..len]);

    for (case, name) in [
        (goldens::snapshot_keyframe(), goldens::FIXTURES[2]),
        (goldens::snapshot_delta(), goldens::FIXTURES[3]),
        (goldens::snapshot_cap(), goldens::FIXTURES[4]),
    ] {
        let len = encode_snapshot(
            &case.header,
            case.removed,
            case.entities(),
            case.baseline(),
            &mut buf,
        )
        .unwrap();
        write_fixture(name, &buf[..len]);
    }

    let len = encode_hello(&goldens::hello(), &mut buf).unwrap();
    write_fixture(goldens::FIXTURES[5], &buf[..len]);
    let len = encode_welcome(&goldens::welcome(), &mut buf).unwrap();
    write_fixture(goldens::FIXTURES[6], &buf[..len]);
    let len = encode_refuse(&goldens::refuse_full(), &mut buf).unwrap();
    write_fixture(goldens::FIXTURES[7], &buf[..len]);

    let (item, added) = goldens::event_gather();
    let len = encode_event_gather(item, added, &mut buf).unwrap();
    write_fixture(goldens::FIXTURES[8], &buf[..len]);

    let (slots, count) = goldens::event_inv();
    let len = encode_event_inv(&slots[..count], &mut buf).unwrap();
    write_fixture(goldens::FIXTURES[9], &buf[..len]);

    let (cx, cz) = goldens::event_slot_change();
    let len = encode_event_slot_change(true, cx, cz, &mut buf).unwrap();
    write_fixture(goldens::FIXTURES[10], &buf[..len]);
    let len = encode_event_slot_change(false, cx, cz, &mut buf).unwrap();
    write_fixture(goldens::FIXTURES[11], &buf[..len]);

    let (reset, cells) = goldens::event_slot_sync();
    let len = encode_event_slot_sync(reset, &cells, &mut buf).unwrap();
    write_fixture(goldens::FIXTURES[12], &buf[..len]);

    let (len, took) = encode_event_catalog(&goldens::event_catalog(), 0, &mut buf).unwrap();
    assert_eq!(took, protocol::CATALOG_BATCH);
    write_fixture(goldens::FIXTURES[13], &buf[..len]);

    let (cx, cz, mark8, weak_hit) = goldens::event_weak_mark();
    let len = encode_event_weak_mark(cx, cz, mark8, weak_hit, &mut buf).unwrap();
    write_fixture(goldens::FIXTURES[14], &buf[..len]);

    let (jobs, eta) = goldens::event_craft_q();
    let len = encode_event_craft_q(&jobs, eta, &mut buf).unwrap();
    write_fixture(goldens::FIXTURES[15], &buf[..len]);

    let (item, added) = goldens::event_craft_done();
    let len = encode_event_craft_done(item, added, &mut buf).unwrap();
    write_fixture(goldens::FIXTURES[16], &buf[..len]);

    let len = encode_event_craft_refused(goldens::event_craft_refused(), &mut buf).unwrap();
    write_fixture(goldens::FIXTURES[17], &buf[..len]);

    let (len, took) = encode_event_recipes(&goldens::event_recipes(), 0, &mut buf).unwrap();
    assert_eq!(took, protocol::RECIPE_BATCH);
    write_fixture(goldens::FIXTURES[18], &buf[..len]);

    let (recipe, count) = goldens::action_craft();
    let len = encode_action_craft(recipe, count, &mut buf).unwrap();
    write_fixture(goldens::FIXTURES[19], &buf[..len]);

    let len = encode_action_cancel(goldens::action_cancel(), &mut buf).unwrap();
    write_fixture(goldens::FIXTURES[20], &buf[..len]);

    let (row, cx, cz, level, loc) = goldens::action_place();
    let len = encode_action_place(row, cx, cz, level, loc, &mut buf).unwrap();
    write_fixture(goldens::FIXTURES[21], &buf[..len]);

    let len = encode_event_piece_placed(&goldens::event_piece_placed(), &mut buf).unwrap();
    write_fixture(goldens::FIXTURES[22], &buf[..len]);

    let (reset, recs) = goldens::event_piece_sync();
    let len = encode_event_piece_sync(reset, &recs, &mut buf).unwrap();
    write_fixture(goldens::FIXTURES[23], &buf[..len]);

    let len = encode_event_build_refused(goldens::event_build_refused(), &mut buf).unwrap();
    write_fixture(goldens::FIXTURES[24], &buf[..len]);

    let (len, took) = encode_event_piece_defs(&goldens::event_piece_defs(), 0, &mut buf).unwrap();
    assert_eq!(took, protocol::PIECE_DEFS_BATCH);
    write_fixture(goldens::FIXTURES[25], &buf[..len]);

    let (row, cx, cz, level, loc) = goldens::action_deploy();
    let len = encode_action_deploy(row, cx, cz, level, loc, &mut buf).unwrap();
    write_fixture(goldens::FIXTURES[26], &buf[..len]);

    let (cx, cz, level) = goldens::action_feed();
    let len = encode_action_feed(cx, cz, level, &mut buf).unwrap();
    write_fixture(goldens::FIXTURES[27], &buf[..len]);

    let len = encode_event_deploy_placed(&goldens::event_deploy_placed(), &mut buf).unwrap();
    write_fixture(goldens::FIXTURES[28], &buf[..len]);

    let (reset, recs) = goldens::event_deploy_sync();
    let len = encode_event_deploy_sync(reset, &recs, &mut buf).unwrap();
    write_fixture(goldens::FIXTURES[29], &buf[..len]);

    let len = encode_event_deploy_refused(goldens::event_deploy_refused(), &mut buf).unwrap();
    write_fixture(goldens::FIXTURES[30], &buf[..len]);

    let dc = goldens::event_deploy_defs();
    let (len, took) = encode_event_deploy_defs(&dc, 0, &mut buf).unwrap();
    assert_eq!(took, dc.def_count as usize);
    write_fixture(goldens::FIXTURES[31], &buf[..len]);

    let (cx, cz, level, loc) = goldens::event_removed();
    let len = encode_event_removed(true, cx, cz, level, loc, &mut buf).unwrap();
    write_fixture(goldens::FIXTURES[32], &buf[..len]);
    let len = encode_event_removed(false, cx, cz, level, loc, &mut buf).unwrap();
    write_fixture(goldens::FIXTURES[33], &buf[..len]);

    let (cx, cz, level, rows) = goldens::event_stock();
    let len = encode_event_stock(cx, cz, level, &rows, &mut buf).unwrap();
    write_fixture(goldens::FIXTURES[34], &buf[..len]);

    let (cx, cz, level, loc) = goldens::action_use();
    let len = encode_action_use(cx, cz, level, loc, &mut buf).unwrap();
    write_fixture(goldens::FIXTURES[35], &buf[..len]);

    let (cx, cz, level, loc, locked) = goldens::action_lock();
    let len = encode_action_lock(cx, cz, level, loc, locked, &mut buf).unwrap();
    write_fixture(goldens::FIXTURES[36], &buf[..len]);

    let (cx, cz, level, loc, open, locked) = goldens::event_door();
    let len = encode_event_door(cx, cz, level, loc, open, locked, &mut buf).unwrap();
    write_fixture(goldens::FIXTURES[37], &buf[..len]);

    let (cx, cz, level, loc, material) = goldens::action_upgrade();
    let len = encode_action_upgrade(cx, cz, level, loc, material, &mut buf).unwrap();
    write_fixture(goldens::FIXTURES[38], &buf[..len]);

    let (text, global) = goldens::chat();
    let len = encode_chat(text.as_bytes(), global, &mut buf).unwrap();
    write_fixture(goldens::FIXTURES[39], &buf[..len]);

    let (from, global, text) = goldens::event_chat();
    let len = encode_event_chat(from, global, &text, &mut buf).unwrap();
    write_fixture(goldens::FIXTURES[40], &buf[..len]);

    let (victim, damage) = goldens::event_hit();
    let len = encode_event_hit(victim, damage, &mut buf).unwrap();
    write_fixture(goldens::FIXTURES[41], &buf[..len]);

    let (hp, max) = goldens::event_health();
    let len = encode_event_health(hp, max, &mut buf).unwrap();
    write_fixture(goldens::FIXTURES[42], &buf[..len]);

    let (victim, killer) = goldens::event_death();
    let len = encode_event_death(victim, killer, &mut buf).unwrap();
    write_fixture(goldens::FIXTURES[43], &buf[..len]);

    let len = encode_action_loot(&mut buf).unwrap();
    write_fixture(goldens::FIXTURES[44], &buf[..len]);

    let bag = goldens::event_bag_dropped();
    let len = encode_event_bag_dropped(&bag, &mut buf).unwrap();
    write_fixture(goldens::FIXTURES[45], &buf[..len]);

    let (reset, recs) = goldens::event_bag_sync();
    let len = encode_event_bag_sync(reset, &recs, &mut buf).unwrap();
    write_fixture(goldens::FIXTURES[46], &buf[..len]);

    let (id, why) = goldens::event_bag_removed();
    let len = encode_event_bag_removed(id, why, &mut buf).unwrap();
    write_fixture(goldens::FIXTURES[47], &buf[..len]);

    for (i, case) in [
        goldens::event_struct_hit_piece(),
        goldens::event_struct_hit_deploy(),
    ]
    .into_iter()
    .enumerate()
    {
        let (deploy, cx, cz, level, loc, row, damage, left) = case;
        let len = encode_event_struct_hit(deploy, cx, cz, level, loc, row, damage, left, &mut buf)
            .unwrap();
        write_fixture(goldens::FIXTURES[48 + i], &buf[..len]);
    }

    let (food, water, max_food, max_water) = goldens::event_vitals();
    let len = encode_event_vitals(food, water, max_food, max_water, &mut buf).unwrap();
    write_fixture(goldens::FIXTURES[50], &buf[..len]);

    let (item, slot) = goldens::event_consumed();
    let len = encode_event_consumed(item, slot, &mut buf).unwrap();
    write_fixture(goldens::FIXTURES[51], &buf[..len]);

    let len = encode_event_consume_refused(goldens::event_consume_refused(), &mut buf).unwrap();
    write_fixture(goldens::FIXTURES[52], &buf[..len]);

    let len = encode_action_consume(goldens::action_consume(), &mut buf).unwrap();
    write_fixture(goldens::FIXTURES[53], &buf[..len]);
}
