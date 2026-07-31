//! Regenerate the golden fixtures from `protocol::goldens` — run only
//! alongside a `PROTO_VER` bump, and commit the new bytes with it
//! (CLAUDE.md wall 6): `cargo run -p protocol --example gen_goldens`.

// Host-side fixture writer: printing and fs are its job. The walls ban
// them in hot-path code; an example binary is not hot-path code.
#![allow(clippy::disallowed_macros)]

use protocol::{encode_input, encode_snapshot, goldens};
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
}
