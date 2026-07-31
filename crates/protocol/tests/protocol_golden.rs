//! `test_protocol_golden` (DESIGN.md §12, CLAUDE.md wall 6): every packet
//! type's encoding is byte-stable against checked-in fixtures, decodes
//! back to exactly what was encoded, and the decoder is total — arbitrary
//! corruption never panics it. A diff against a fixture without a
//! `PROTO_VER` bump in the same commit is the wire drifting by accident.

use protocol::goldens::{
    hello, input_acks_only, input_full, refuse_full, snapshot_cap, snapshot_delta,
    snapshot_keyframe, welcome, SnapshotCase, FIXTURES,
};
use protocol::{
    decode_hello, decode_input, decode_refuse, decode_snapshot, decode_welcome, encode_hello,
    encode_input, encode_refuse, encode_snapshot, encode_welcome, peek_kind, InputDatagram,
    KIND_HELLO, KIND_INPUT, KIND_REFUSE, KIND_SNAPSHOT, KIND_WELCOME,
};
use sim_core::limits::DATAGRAM_BUDGET_BYTES;
use sim_core::rng::Pcg32;

const GOLDEN: [&[u8]; 8] = [
    include_bytes!("golden/v0_input_acks_only.bin"),
    include_bytes!("golden/v0_input_full.bin"),
    include_bytes!("golden/v0_snapshot_keyframe.bin"),
    include_bytes!("golden/v0_snapshot_delta.bin"),
    include_bytes!("golden/v0_snapshot_cap.bin"),
    include_bytes!("golden/v0_hello.bin"),
    include_bytes!("golden/v0_welcome.bin"),
    include_bytes!("golden/v0_refuse_full.bin"),
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
}

/// The handshake trio: byte-stable, kind-peekable, decode-exact.
fn golden_stream(fixture: &[u8], name: &str) {
    let mut buf = [0u8; 64];
    match name {
        "v0_hello.bin" => {
            let len = encode_hello(&hello(), &mut buf).unwrap();
            assert_eq!(&buf[..len], fixture, "{name}: bytes drifted");
            assert_eq!(peek_kind(fixture).unwrap(), KIND_HELLO);
            assert_eq!(decode_hello(fixture).unwrap(), hello());
        }
        "v0_welcome.bin" => {
            let len = encode_welcome(&welcome(), &mut buf).unwrap();
            assert_eq!(&buf[..len], fixture, "{name}: bytes drifted");
            assert_eq!(peek_kind(fixture).unwrap(), KIND_WELCOME);
            assert_eq!(decode_welcome(fixture).unwrap(), welcome());
        }
        "v0_refuse_full.bin" => {
            let len = encode_refuse(&refuse_full(), &mut buf).unwrap();
            assert_eq!(&buf[..len], fixture, "{name}: bytes drifted");
            assert_eq!(peek_kind(fixture).unwrap(), KIND_REFUSE);
            assert_eq!(decode_refuse(fixture).unwrap(), refuse_full());
        }
        other => panic!("unknown stream fixture {other}"),
    }
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
