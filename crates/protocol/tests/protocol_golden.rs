//! `test_protocol_golden` (DESIGN.md §12, CLAUDE.md wall 6): every packet
//! type's encoding is byte-stable against checked-in fixtures, decodes
//! back to exactly what was encoded, and the decoder is total — arbitrary
//! corruption never panics it. A diff against a fixture without a
//! `PROTO_VER` bump in the same commit is the wire drifting by accident.

use protocol::goldens::{
    action_cancel, action_craft, action_place, event_build_refused, event_catalog,
    event_craft_done, event_craft_q, event_craft_refused, event_gather, event_inv,
    event_piece_defs, event_piece_placed, event_piece_sync, event_recipes, event_slot_change,
    event_slot_sync, event_weak_mark, hello, input_acks_only, input_full, refuse_full,
    snapshot_cap, snapshot_delta, snapshot_keyframe, welcome, SnapshotCase, FIXTURES,
};
use protocol::{
    decode_action, decode_event, decode_hello, decode_input, decode_refuse, decode_snapshot,
    decode_welcome, encode_action_cancel, encode_action_craft, encode_action_place,
    encode_event_build_refused, encode_event_catalog, encode_event_craft_done,
    encode_event_craft_q, encode_event_craft_refused, encode_event_gather, encode_event_inv,
    encode_event_piece_defs, encode_event_piece_placed, encode_event_piece_sync,
    encode_event_recipes, encode_event_slot_change, encode_event_slot_sync, encode_event_weak_mark,
    encode_hello, encode_input, encode_refuse, encode_snapshot, encode_welcome, peek_kind,
    ActionMsg, EventMsg, InputDatagram, WireError, CATALOG_BATCH, KIND_ACTION, KIND_EVENT,
    KIND_HELLO, KIND_INPUT, KIND_REFUSE, KIND_SNAPSHOT, KIND_WELCOME, MAX_EVENT_MSG_BYTES,
    PIECE_DEFS_BATCH, PIECE_SYNC_BATCH, RECIPE_BATCH, SLOT_SYNC_BATCH,
};
use sim_core::input::InputFrame;
use sim_core::limits::DATAGRAM_BUDGET_BYTES;
use sim_core::rng::Pcg32;

const GOLDEN: [&[u8]; 26] = [
    include_bytes!("golden/v4_input_acks_only.bin"),
    include_bytes!("golden/v4_input_full.bin"),
    include_bytes!("golden/v4_snapshot_keyframe.bin"),
    include_bytes!("golden/v4_snapshot_delta.bin"),
    include_bytes!("golden/v4_snapshot_cap.bin"),
    include_bytes!("golden/v4_hello.bin"),
    include_bytes!("golden/v4_welcome.bin"),
    include_bytes!("golden/v4_refuse_full.bin"),
    include_bytes!("golden/v4_event_gather.bin"),
    include_bytes!("golden/v4_event_inv.bin"),
    include_bytes!("golden/v4_event_slot_harvested.bin"),
    include_bytes!("golden/v4_event_slot_respawned.bin"),
    include_bytes!("golden/v4_event_slot_sync.bin"),
    include_bytes!("golden/v4_event_catalog.bin"),
    include_bytes!("golden/v4_event_weak_mark.bin"),
    include_bytes!("golden/v4_event_craft_q.bin"),
    include_bytes!("golden/v4_event_craft_done.bin"),
    include_bytes!("golden/v4_event_craft_refused.bin"),
    include_bytes!("golden/v4_event_recipes.bin"),
    include_bytes!("golden/v4_action_craft.bin"),
    include_bytes!("golden/v4_action_cancel.bin"),
    include_bytes!("golden/v4_action_place.bin"),
    include_bytes!("golden/v4_event_piece_placed.bin"),
    include_bytes!("golden/v4_event_piece_sync.bin"),
    include_bytes!("golden/v4_event_build_refused.bin"),
    include_bytes!("golden/v4_event_piece_defs.bin"),
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
}

/// C→S action frames: byte-stable, kind-peekable, decode-exact.
fn golden_action(fixture: &[u8], name: &str) {
    let mut buf = [0u8; 64];
    assert_eq!(peek_kind(fixture).unwrap(), KIND_ACTION, "{name}");
    let len = match name {
        "v4_action_craft.bin" => {
            let (recipe, count) = action_craft();
            assert_eq!(
                decode_action(fixture).unwrap(),
                ActionMsg::Craft { recipe, count },
                "{name}: decode mismatch"
            );
            encode_action_craft(recipe, count, &mut buf).unwrap()
        }
        "v4_action_cancel.bin" => {
            let index = action_cancel();
            assert_eq!(
                decode_action(fixture).unwrap(),
                ActionMsg::CraftCancel { index },
                "{name}: decode mismatch"
            );
            encode_action_cancel(index, &mut buf).unwrap()
        }
        "v4_action_place.bin" => {
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
        other => panic!("unknown action fixture {other}"),
    };
    assert_eq!(&buf[..len], fixture, "{name}: bytes drifted");
}

/// The handshake trio: byte-stable, kind-peekable, decode-exact.
fn golden_stream(fixture: &[u8], name: &str) {
    let mut buf = [0u8; 64];
    match name {
        "v4_hello.bin" => {
            let len = encode_hello(&hello(), &mut buf).unwrap();
            assert_eq!(&buf[..len], fixture, "{name}: bytes drifted");
            assert_eq!(peek_kind(fixture).unwrap(), KIND_HELLO);
            assert_eq!(decode_hello(fixture).unwrap(), hello());
        }
        "v4_welcome.bin" => {
            let len = encode_welcome(&welcome(), &mut buf).unwrap();
            assert_eq!(&buf[..len], fixture, "{name}: bytes drifted");
            assert_eq!(peek_kind(fixture).unwrap(), KIND_WELCOME);
            assert_eq!(decode_welcome(fixture).unwrap(), welcome());
        }
        "v4_refuse_full.bin" => {
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
        "v4_event_gather.bin" => {
            let (item, added) = event_gather();
            assert_eq!(
                decode_event(fixture).unwrap(),
                EventMsg::Gather { item, added },
                "{name}: decode mismatch"
            );
            encode_event_gather(item, added, &mut buf).unwrap()
        }
        "v4_event_inv.bin" => {
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
        "v4_event_slot_harvested.bin" | "v4_event_slot_respawned.bin" => {
            let harvested = name == "v4_event_slot_harvested.bin";
            let (cx, cz) = event_slot_change();
            let want = if harvested {
                EventMsg::SlotHarvested { cx, cz }
            } else {
                EventMsg::SlotRespawned { cx, cz }
            };
            assert_eq!(decode_event(fixture).unwrap(), want, "{name}");
            encode_event_slot_change(harvested, cx, cz, &mut buf).unwrap()
        }
        "v4_event_slot_sync.bin" => {
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
        "v4_event_catalog.bin" => {
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
        "v4_event_weak_mark.bin" => {
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
        "v4_event_craft_q.bin" => {
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
        "v4_event_craft_done.bin" => {
            let (item, added) = event_craft_done();
            assert_eq!(
                decode_event(fixture).unwrap(),
                EventMsg::CraftDone { item, added },
                "{name}: decode mismatch"
            );
            encode_event_craft_done(item, added, &mut buf).unwrap()
        }
        "v4_event_craft_refused.bin" => {
            let reason = event_craft_refused();
            assert_eq!(
                decode_event(fixture).unwrap(),
                EventMsg::CraftRefused { reason },
                "{name}: decode mismatch"
            );
            encode_event_craft_refused(reason, &mut buf).unwrap()
        }
        "v4_event_recipes.bin" => {
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
        "v4_event_piece_placed.bin" => {
            let rec = event_piece_placed();
            assert_eq!(
                decode_event(fixture).unwrap(),
                EventMsg::PiecePlaced { rec },
                "{name}: decode mismatch"
            );
            encode_event_piece_placed(&rec, &mut buf).unwrap()
        }
        "v4_event_piece_sync.bin" => {
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
        "v4_event_build_refused.bin" => {
            let reason = event_build_refused();
            assert_eq!(
                decode_event(fixture).unwrap(),
                EventMsg::BuildRefused { reason },
                "{name}: decode mismatch"
            );
            encode_event_build_refused(reason, &mut buf).unwrap()
        }
        "v4_event_piece_defs.bin" => {
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
        let _ = decode_event(bytes);
        let _ = decode_action(bytes);
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
