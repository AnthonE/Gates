//! Regenerate the golden fixtures from `protocol::goldens` — run only
//! alongside a `PROTO_VER` bump, and commit the new bytes with it
//! (CLAUDE.md wall 6): `cargo run -p protocol --example gen_goldens`.

// Host-side fixture writer: printing and fs are its job. The walls ban
// them in hot-path code; an example binary is not hot-path code.
#![allow(clippy::disallowed_macros)]

use protocol::{
    encode_action_cancel, encode_action_craft, encode_event_catalog, encode_event_craft_done,
    encode_event_craft_q, encode_event_craft_refused, encode_event_gather, encode_event_inv,
    encode_event_recipes, encode_event_slot_change, encode_event_slot_sync, encode_event_weak_mark,
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
}
