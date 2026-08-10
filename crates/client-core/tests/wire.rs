//! Wire in, core state out — what `ci/client_smoke.mjs` used to assert
//! through a browser's C ABI, asserted here against the `ClientCore` the
//! desktop client actually runs.
//!
//! ## Why this file exists, and what it is NOT a port of
//!
//! The deleted gate loaded `client_wasm.wasm` and drove 199 `extern "C"`
//! exports, and its own header said its job was proving *"the wasm
//! artifact actually exposes and runs"* logic owned elsewhere. Half of it
//! was therefore about a door — export presence, pointer arithmetic into
//! `memory.buffer`, argument order across the ABI — and that half died
//! with the door (operator, 2026-08-08: *"we use desktop build no more
//! web"*). The desktop client calls `ClientCore` directly; there is no
//! ABI between them to get wrong.
//!
//! The other half was real and is here: **a wire message arrives and the
//! core's state has to be exactly what that message said.** Those checks
//! had drifted into being the only place several of them lived —
//! containers, the move verdict, the kill feed, the death screen, the bag
//! set, the struct-hit readout — so deleting the gate without this file
//! would have deleted coverage, which is not what cutting a web build is
//! supposed to cost.
//!
//! ## One difference from the original, and it is an improvement
//!
//! The JS built every frame by hand from the field widths `event.rs`
//! declares, because it could not link the encoder. This links it, so
//! every frame below comes from `protocol::encode_event_*`. That is
//! strictly better for what is being tested — a hand-framed bit pattern
//! tests the decoder against the test author's reading of the layout,
//! while the encoder tests it against the layout the server will actually
//! send. What the hand-framing bought (catching a width change) is
//! `test_protocol_golden`'s job and always was.
//!
//! The discipline that carries across intact: **every number in a payload
//! is distinct from every other one in it.** These messages are mostly
//! (kind, slot) pairs and (item, count) pairs, which is the exact shape
//! ~27 of the reference's positional-payload corrections landed on
//! (`CLAUDE.md` traps, `reference/FINDINGS.md` §1). A rotation has to
//! fail, so no two fields may share a value.

use client_core::core::{
    ClientCore, APPLIED2_CONT, APPLIED2_MOVE, APPLIED_BAGS, APPLIED_DEATH, APPLIED_HIT,
};
use protocol::{
    encode_event_bag_dropped, encode_event_bag_removed, encode_event_bag_sync,
    encode_event_cont_sync, encode_event_death, encode_event_hit, encode_event_move_refused,
    encode_event_moved, encode_event_respawn, encode_event_shot, encode_event_struct_hit,
    encode_event_vitals, InvSlot, WireBag, MAX_EVENT_MSG_BYTES,
};
use sim_core::backpack::{BAG_GONE_DESPAWN, BAG_GONE_EMPTIED};
use sim_core::gather::ItemStack;
use sim_core::inventory::{addr, CONT_BAG, CONT_BOX, CONT_SELF, REFUSE_M_NO_ROOM};
use sim_core::world::{DEATH_BY_ARROW, DEATH_BY_HAND};

/// The player this core is. Distinct from every other id below, because
/// "is this death mine" is a comparison against exactly this number.
const ME: u32 = 0x107;
/// A packed `box_key`, the same one the deleted gate used.
const BOX: u32 = 0x0155_d450;

fn core() -> ClientCore {
    ClientCore::new(1, ME, 0)
}

/// Feed one encoded event and return the word-0 flags, failing loudly on a
/// decode error — the sentinel the browser path used to report as bit 31.
fn feed(c: &mut ClientCore, bytes: &[u8]) -> u32 {
    c.on_stream(bytes).expect("a well-formed event must decode")
}

/// The container view, both directions.
///
/// The claim that matters is the **echo**: a batch names the container it
/// belongs to, and a batch naming a different one is dropped whole.
/// Without that, a diff for a box the player already closed lands in
/// whatever panel is open — right values, wrong destination, which is the
/// positional-payload defect one level up.
#[test]
fn a_container_sync_opens_diffs_and_closes_and_a_foreign_batch_is_dropped() {
    let mut c = core();
    let mut buf = [0u8; MAX_EVENT_MSG_BYTES];

    assert_eq!(c.cont_kind, CONT_SELF, "nothing is open before a sync");
    assert_eq!(c.cont_handle, 0, "a closed panel names no container");

    // The open: CONT_BOX, reset, two rows. Slot 0 / item 5 / count 40 and
    // slot 11 / item 31 / count 3 — six distinct numbers, so a (slot,
    // item, count) rotation cannot pass.
    let rows = [
        InvSlot {
            slot: 0,
            stack: ItemStack { item: 5, count: 40 },
        },
        InvSlot {
            slot: 11,
            stack: ItemStack { item: 31, count: 3 },
        },
    ];
    let len = encode_event_cont_sync(CONT_BOX, BOX, true, &rows, &mut buf).unwrap();
    feed(&mut c, &buf[..len]);
    assert_eq!(c.applied2(), APPLIED2_CONT, "an open applies the CONT flag");
    assert_eq!(c.cont_kind, CONT_BOX, "the open panel names CONT_BOX");
    assert_eq!(
        c.cont_handle, BOX,
        "the open panel lost its handle — a move typed against the wrong \
         address is the container divergence the reference answered by \
         disconnecting the client"
    );
    assert_eq!(c.cont[0], ItemStack { item: 5, count: 40 });
    assert_eq!(c.cont[11], ItemStack { item: 31, count: 3 });
    assert_eq!(
        c.cont[2],
        ItemStack::default(),
        "a reset leaves the untouched slots empty"
    );

    // A diff for the SAME container, emptying slot 0. An item 0 of count 0
    // is a real change and not a no-op: "that is gone" has to be sayable.
    let gone = [InvSlot {
        slot: 0,
        stack: ItemStack::default(),
    }];
    let len = encode_event_cont_sync(CONT_BOX, BOX, false, &gone, &mut buf).unwrap();
    feed(&mut c, &buf[..len]);
    assert_eq!(c.applied2(), APPLIED2_CONT, "a diff applies the CONT flag");
    assert_eq!(c.cont[0], ItemStack::default(), "an emptied slot clears");
    assert_eq!(
        c.cont[11],
        ItemStack { item: 31, count: 3 },
        "a diff must not disturb the slots it did not name"
    );

    // A diff for a container this client does NOT have open, dropped
    // whole. Same kind, one bit of the handle different — the cheapest
    // thing an echo check can get wrong.
    let foreign = [InvSlot {
        slot: 11,
        stack: ItemStack { item: 7, count: 99 },
    }];
    let len = encode_event_cont_sync(CONT_BOX, BOX ^ 1, false, &foreign, &mut buf).unwrap();
    feed(&mut c, &buf[..len]);
    assert_eq!(
        c.applied2(),
        0,
        "a diff for another container applies nothing"
    );
    assert_eq!(
        c.cont[11],
        ItemStack { item: 31, count: 3 },
        "a foreign diff reached this panel's slots"
    );

    // The server's close: kind 0, no handle, no slots, reset set.
    let len = encode_event_cont_sync(CONT_SELF, 0, true, &[], &mut buf).unwrap();
    feed(&mut c, &buf[..len]);
    assert_eq!(c.applied2(), APPLIED2_CONT, "a close applies the CONT flag");
    assert_eq!(c.cont_kind, CONT_SELF, "a close shuts the panel");
    assert_eq!(c.cont_handle, 0, "a close forgets the handle");
}

/// The move verdict, both outcomes.
///
/// The verdict rides word 1 rather than word 0, and that is not a layout
/// preference: `APPLIED_MOVE` was once `1 << 31`, which is the decode-error
/// sentinel's own value, so the first move of a session read as an error.
/// `core.rs`'s ledger test proves no constant can reach that bit; this one
/// proves the path a panel actually reads reports a verdict and not an
/// error.
#[test]
fn a_move_and_its_refusal_land_in_word_one_with_the_address_intact() {
    let mut c = core();
    let mut buf = [0u8; MAX_EVENT_MSG_BYTES];

    // from (CONT_BAG, slot 9) → to (CONT_SELF, slot 22), 7 of item 5.
    // Every part distinct from every other.
    let a = addr(CONT_BAG, 9, CONT_SELF, 22);
    let len = encode_event_moved(CONT_BAG, 9, CONT_SELF, 22, 7, 5, &mut buf).unwrap();
    feed(&mut c, &buf[..len]);
    assert_eq!(c.applied2(), APPLIED2_MOVE, "a move applies the MOVE flag");
    assert_eq!(c.last_move, a, "the move readout lost its address");
    assert_eq!(c.last_move_refused, 0, "an accepted move has no reason");
    assert_eq!(
        c.last_move_count,
        (7 << 16) | 5,
        "move payload mismatch: count 7 of item 5"
    );

    // The refusal: a different address, so a readout that kept the last
    // one would fail here rather than pass by coincidence.
    let b = addr(CONT_SELF, 11, CONT_BAG, 26);
    let len = encode_event_move_refused(
        REFUSE_M_NO_ROOM as u8,
        CONT_SELF,
        11,
        CONT_BAG,
        26,
        &mut buf,
    )
    .unwrap();
    feed(&mut c, &buf[..len]);
    assert_eq!(
        c.applied2(),
        APPLIED2_MOVE,
        "a refusal applies the MOVE flag"
    );
    assert_eq!(
        c.last_move_refused as u32, REFUSE_M_NO_ROOM,
        "move refusal reason mismatch"
    );
    assert_eq!(
        c.last_move, b,
        "a refusal must still carry the address, or the panel cannot roll \
         back the drag it drew"
    );
    assert_eq!(c.last_move_count, 0, "a refusal moved nothing");

    // Word 1 describes ONE message. A verdict that outlived its message
    // would be read as the answer to a drag the server has not answered
    // yet, and the panel would roll back a live move.
    let len = encode_event_vitals(50, 50, 100, 100, &mut buf).unwrap();
    feed(&mut c, &buf[..len]);
    assert_eq!(c.applied2(), 0, "a stale move verdict outlived its message");
}

/// The hitmarker ring: own landed hits, oldest first, drained by the
/// reader.
#[test]
fn the_hit_ring_fills_and_drains() {
    let mut c = core();
    let mut buf = [0u8; MAX_EVENT_MSG_BYTES];
    assert_eq!(c.pop_hit(), None, "the hit ring starts empty");

    let len = encode_event_hit(9, 37, &mut buf).unwrap();
    assert_eq!(feed(&mut c, &buf[..len]) & APPLIED_HIT, APPLIED_HIT);
    assert_eq!(c.pop_hit(), Some(37), "hitmarker damage mismatch");
    assert_eq!(c.pop_hit(), None, "the hit ring must drain");
}

/// A shot crosses whole and drains once (wire v33).
///
/// **The five fields are the test.** `EV_SHOT` carries two pairs of
/// same-typed neighbours — (yaw, pitch) and (speed, drop) — and the tracer
/// flies the arc they describe, so a transposition inside either pair is an
/// arrow that leaves at the wrong angle or falls at its own muzzle speed.
/// The fixture picks values no two of which could be mistaken for each
/// other, and asserts each by name rather than asserting the tuple is
/// non-default.
///
/// It also pins the ring's contract: drained exactly once. `render::feed`
/// is the single consumer, and CLAUDE.md's clean-merge trap is what a
/// second one costs.
#[test]
fn a_shot_crosses_whole_and_drains_once() {
    let mut c = core();
    let mut buf = [0u8; MAX_EVENT_MSG_BYTES];
    assert_eq!(c.pop_shot(), None, "the shot ring starts empty");

    // A yaw past a byte, a pitch that is not the level default, and a
    // speed two orders off the drop: every field distinguishable from
    // every other.
    let (shooter, yaw, pitch, speed, drop) = (0x2B17u32, 41_234u16, 203u8, 1_333u16, 22u16);
    let len = encode_event_shot(shooter, yaw, pitch, speed, drop, &mut buf).unwrap();
    feed(&mut c, &buf[..len]);
    assert_eq!(
        c.pop_shot(),
        Some((shooter, yaw, pitch, speed, drop)),
        "the shot arrived with a field in the wrong seat"
    );
    assert_eq!(c.pop_shot(), None, "the shot ring must drain");
}

/// A round with no muzzle speed is refused rather than drawn.
///
/// Zero speed is a tracer that hangs at the shooter's eye forever, and
/// `content::validate` refuses such a round at boot, so the value can only
/// reach a client from a forged or corrupted datagram. Both ends refuse it:
/// the encoder returns `Range`, and this is the decoder's half.
///
/// **The forge is bit-exact and paired with a control**, because a test that
/// corrupted the frame at large would be refused for any number of reasons
/// and prove nothing about this one. The event header is 10 bits
/// (`KIND_BITS` 4 + `SUB_BITS` 6) and the writer packs LSB-first, so the
/// fields land at bit 10 (shooter, 32), 42 (yaw, 16), 58 (pitch, 8), 66
/// (speed, 16), 82 (drop, 16). Only bits 66..=81 are cleared; the control
/// clears a strict subset that leaves the speed nonzero and must still
/// decode. If the control ever starts failing, this test has stopped
/// isolating the zero.
#[test]
fn a_shot_with_no_speed_is_malformed() {
    let mut c = core();
    let mut buf = [0u8; MAX_EVENT_MSG_BYTES];
    // speed = 1, so the field's only set bit is its bit 0, at wire
    // position 66 — byte 8, offset 2.
    let len = encode_event_shot(0x2B17, 41_234, 203, 1, 22, &mut buf).unwrap();

    // Control: clear the speed's middle byte only. Speed stays 1, and the
    // whole frame must still decode — which is what proves the assertion
    // below is about the zero and not about the tampering.
    let mut control = buf[..len].to_vec();
    control[9] = 0;
    assert!(
        c.on_stream(&control).is_ok(),
        "the control must decode, or the forge below proves nothing"
    );
    assert!(c.pop_shot().is_some(), "and it must reach the ring");

    // Forge: clear exactly bits 66..=81. Byte 8 keeps its low two bits
    // (the top of `pitch`), byte 10 keeps everything above its low two
    // (the bottom of `drop`).
    let mut forged = buf[..len].to_vec();
    forged[8] &= 0b0000_0011;
    forged[9] = 0;
    forged[10] &= 0b1111_1100;
    assert!(
        c.on_stream(&forged).is_err(),
        "a zero-speed shot must be refused, not drawn"
    );
    assert_eq!(c.pop_shot(), None, "and nothing may reach the ring");
}

/// The kill feed and the death screen are two different things, and the
/// difference is whose body fell. A stranger's death is a feed line; the
/// screen is own-body only, and the respawn that answers it is what
/// closes it.
#[test]
fn a_death_feeds_everyone_and_only_our_own_raises_the_screen() {
    let mut c = core();
    let mut buf = [0u8; MAX_EVENT_MSG_BYTES];
    assert!(!c.dead, "the screen starts closed");

    // A stranger's, with four values all distinct from ours.
    let len = encode_event_death(0x2A, 0x3B, DEATH_BY_HAND, 4, 250, &mut buf).unwrap();
    assert_eq!(feed(&mut c, &buf[..len]) & APPLIED_DEATH, APPLIED_DEATH);
    assert!(!c.dead, "a stranger's death opened our screen");
    assert_eq!(c.pop_death(), Some(0x2A), "the feed lost the stranger");
    assert_eq!(c.last_death_killer, 0x3B, "the feed lost the killer");
    assert_eq!(c.pop_death(), None, "the death ring must drain");

    // Ours, by an arrow — the cause that has to decode rather than error,
    // and the range and weapon the screen says.
    let len = encode_event_death(ME, 0x4C, DEATH_BY_ARROW, 6, 1_337, &mut buf).unwrap();
    feed(&mut c, &buf[..len]);
    assert!(c.dead, "our own death must raise the screen");
    assert_eq!(c.own_death_killer, 0x4C, "the screen lost who shot us");
    assert_eq!(
        c.own_death_cause, DEATH_BY_ARROW,
        "the arrow cause must decode"
    );
    assert_eq!(c.own_death_item, 6, "the screen lost the weapon");
    assert_eq!(c.own_death_range_cm, 1_337, "the screen lost the range");

    // A stranger's death while we are down must not rewrite our screen.
    let len = encode_event_death(0x5D, 0x6E, DEATH_BY_HAND, 1, 100, &mut buf).unwrap();
    feed(&mut c, &buf[..len]);
    assert!(c.dead, "a stranger's death closed our screen");
    assert_eq!(
        c.own_death_killer, 0x4C,
        "a stranger's death rewrote our death screen"
    );

    // The wake answers it. A beach wake leaves `woke_on_bag` false, and
    // that outlives the screen on purpose — it is the only moment the
    // fact is worth anything to a player who asked for a bag.
    let len = encode_event_respawn(false, &mut buf).unwrap();
    feed(&mut c, &buf[..len]);
    assert!(!c.dead, "the wake must close the screen");
    assert!(!c.woke_on_bag, "a beach wake is not a bag wake");
}

/// The bag set: dropped, repeated, replaced by a sync walk, removed, and
/// an unknown removal that must disturb nothing.
#[test]
fn the_bag_set_adds_syncs_and_removes() {
    let mut c = core();
    let mut buf = [0u8; MAX_EVENT_MSG_BYTES];
    assert!(c.bags.entries().is_empty(), "no bag has been announced yet");

    let bag = WireBag {
        id: 77,
        qx: 1_000,
        qy: 2_000,
        qz: 3_000,
    };
    let len = encode_event_bag_dropped(&bag, &mut buf).unwrap();
    assert_eq!(feed(&mut c, &buf[..len]) & APPLIED_BAGS, APPLIED_BAGS);
    assert_eq!(c.bags.entries().len(), 1, "the bag set must hold it");
    assert_eq!(c.bags.entries()[0].id, 77, "bag id mismatch");

    // The same bag again is not a second bag.
    feed(&mut c, &buf[..len]);
    assert_eq!(
        c.bags.entries().len(),
        1,
        "a repeat must not double the set"
    );

    // A sync walk with reset replaces whatever the client believed.
    let synced = [
        WireBag {
            id: 91,
            qx: 4,
            qy: 5,
            qz: 6,
        },
        WireBag {
            id: 92,
            qx: 7,
            qy: 8,
            qz: 9,
        },
    ];
    let len = encode_event_bag_sync(true, &synced, &mut buf).unwrap();
    assert_eq!(feed(&mut c, &buf[..len]) & APPLIED_BAGS, APPLIED_BAGS);
    let ids: Vec<u32> = c.bags.entries().iter().map(|b| b.id).collect();
    assert_eq!(ids, vec![91, 92], "the walk replaces the set it resets");

    let len = encode_event_bag_removed(91, BAG_GONE_EMPTIED as u8, &mut buf).unwrap();
    assert_eq!(feed(&mut c, &buf[..len]) & APPLIED_BAGS, APPLIED_BAGS);
    let ids: Vec<u32> = c.bags.entries().iter().map(|b| b.id).collect();
    assert_eq!(ids, vec![92], "the wrong bag was removed");

    // A removal for a bag this client never heard of changes nothing.
    let len = encode_event_bag_removed(9_999, BAG_GONE_DESPAWN as u8, &mut buf).unwrap();
    feed(&mut c, &buf[..len]);
    let ids: Vec<u32> = c.bags.entries().iter().map(|b| b.id).collect();
    assert_eq!(ids, vec![92], "an unknown removal disturbed the set");
}

/// The struct-hit readout, including the one field the wire does not
/// carry: `max` is resolved from the def table this client holds, and a
/// row it has not received yet reports 0 rather than a guess.
#[test]
fn a_struct_hit_carries_its_address_and_never_guesses_a_maximum() {
    let mut c = core();
    let mut buf = [0u8; MAX_EVENT_MSG_BYTES];

    // Address and payload all distinct: cell (12, 34), level 5, loc 1,
    // row 9, 40 damage, 60 left.
    let len = encode_event_struct_hit(false, 12, 34, 5, 1, 9, 40, 60, &mut buf).unwrap();
    feed(&mut c, &buf[..len]);
    let (cx, cz, level, loc, left, max) = c.struct_hit;
    assert_eq!(
        (cx, cz, level, loc),
        (12, 34, 5, 1),
        "struct-hit address mismatch"
    );
    assert_eq!(left, 60, "struct-hit hp-left mismatch");
    assert_eq!(
        max, 0,
        "a row whose defs have not arrived must report max 0, never a guess"
    );
    assert_eq!(
        c.pop_hit(),
        Some(40),
        "a raid swing must still feed the hitmarker ring"
    );
}

/// Vitals, and the reading that must be refused rather than clamped: the
/// meters are absolute and a value past its own maximum is a frame this
/// client cannot have been sent.
#[test]
fn vitals_apply_and_an_impossible_reading_is_refused() {
    let mut c = core();
    let mut buf = [0u8; MAX_EVENT_MSG_BYTES];

    // Four distinct numbers, so a food/water or value/max transposition
    // cannot pass.
    let len = encode_event_vitals(30, 45, 90, 120, &mut buf).unwrap();
    feed(&mut c, &buf[..len]);
    assert_eq!(c.food, 30, "food mismatch");
    assert_eq!(c.water, 45, "water mismatch");
    assert_eq!(c.max_food, 90, "max food mismatch");
    assert_eq!(c.max_water, 120, "max water mismatch");

    // Past its own maximum: refused at the encoder, so the frame cannot be
    // built at all. The decoder's half of the same rule is `decode_event`'s
    // and is exercised by `test_decode_is_total`.
    assert!(
        encode_event_vitals(200, 45, 90, 120, &mut buf).is_err(),
        "a meter past its own maximum must not encode"
    );
}

/// Garbage in, refusal out — never a panic and never a silent apply. The
/// browser path reported this as bit 31; the desktop path reads the
/// `Result`, which is the same fact with a type on it.
#[test]
fn garbage_is_refused_rather_than_applied() {
    let mut c = core();
    assert!(c.on_stream(&[0xff; 12]).is_err(), "garbage must not decode");
    assert!(
        c.on_stream(&[]).is_err(),
        "an empty message must not decode"
    );
    // And the state is untouched: a refused frame is not a partial apply.
    assert_eq!(c.cont_kind, CONT_SELF);
    assert_eq!(c.pop_hit(), None);
    assert!(!c.dead);
}
