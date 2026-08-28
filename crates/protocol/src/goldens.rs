//! The canonical packets behind `test_protocol_golden` (DESIGN.md §12):
//! one construction, two consumers — `examples/gen_goldens.rs` writes the
//! fixture bytes, `tests/protocol_golden.rs` asserts today's encoder still
//! produces them byte-for-byte. Content is deterministic (sim-core Pcg32,
//! fixed seeds), so "regenerate" is reproducible on any box.
//!
//! Regenerating fixtures is legitimate for exactly two reasons, the diff
//! looks the same either way, and that is why each takes its own commit.
//! **The encoder moved** — a layout or a meaning change — takes a
//! `PROTO_VER` bump in the same commit (CLAUDE.md wall 6); a diff here
//! without one is the wire drifting by accident, the exact thing the gate
//! exists to catch. **The fixture's INPUT moved** — a fuzz draw widened, a
//! deliberate value pinned — takes no bump, because a constructor in this
//! module is the test's coverage and not a fact about the wire
//! (`PROTO_VER`'s own doc carries that rule, decided 2026-08-18 under
//! NOW.md §5c). Never both at once: the bytes would then move for two
//! reasons and no reader afterwards can tell which byte answers to which.

use crate::{
    ChatText, EntityState, Hello, InputDatagram, InvSlot, ItemCatalog, Nudge, Refuse,
    SnapshotHeader, Welcome, WireBag, BAG_SYNC_BATCH, DEPLOY_SYNC_BATCH, PIECE_SYNC_BATCH,
    PLATE_BIAS, PLATE_BITS, SLOT_SYNC_BATCH,
};
use sim_core::build::{BuildContent, PieceDef, PieceRec};
use sim_core::craft::{
    CraftContent, CraftJob, RecipeDef, STATION_FURNACE, STATION_NONE, STATION_WORKBENCH1,
    STATION_WORKBENCH2, STATION_WORKBENCH3,
};
use sim_core::deploy::{BagAnchor, DeployContent, DeployRec, BAG_CAP};
use sim_core::gather::ItemStack;
use sim_core::input::InputFrame;
use sim_core::limits::{
    INV_SLOTS, MAX_INPUT_FRAMES, MAX_PIECE_COSTS, MAX_RECIPE_INPUTS, MAX_SNAPSHOT_ENTITIES,
};
use sim_core::research::{ResearchContent, ResearchRow, NO_RECIPE};
use sim_core::rng::Pcg32;

/// Fixture file names, keyed by wire version (`PROTO_VER` 10 ⇒ `v10_*`).
pub const FIXTURES: [&str; 100] = [
    "v51_input_acks_only.bin",
    "v51_input_full.bin",
    "v51_snapshot_keyframe.bin",
    "v51_snapshot_delta.bin",
    "v51_snapshot_cap.bin",
    "v51_hello.bin",
    "v51_welcome.bin",
    "v51_refuse_full.bin",
    "v51_event_gather.bin",
    "v51_event_inv.bin",
    "v51_event_slot_harvested.bin",
    "v51_event_slot_respawned.bin",
    "v51_event_slot_sync.bin",
    "v51_event_catalog.bin",
    "v51_event_weak_mark.bin",
    "v51_event_craft_q.bin",
    "v51_event_craft_done.bin",
    "v51_event_craft_refused.bin",
    "v51_event_recipes.bin",
    "v51_action_craft.bin",
    "v51_action_cancel.bin",
    "v51_action_place.bin",
    "v51_event_piece_placed.bin",
    "v51_event_piece_sync.bin",
    "v51_event_build_refused.bin",
    "v51_event_piece_defs.bin",
    "v51_action_deploy.bin",
    "v51_action_feed.bin",
    "v51_event_deploy_placed.bin",
    "v51_event_deploy_sync.bin",
    "v51_event_deploy_refused.bin",
    "v51_event_deploy_defs.bin",
    "v51_event_piece_removed.bin",
    "v51_event_deploy_removed.bin",
    "v51_event_stock.bin",
    "v51_action_use.bin",
    "v51_action_access.bin",
    "v51_event_door.bin",
    "v51_action_upgrade.bin",
    "v51_chat.bin",
    "v51_event_chat.bin",
    "v51_event_hit.bin",
    "v51_event_health.bin",
    "v51_event_death.bin",
    "v51_action_loot.bin",
    "v51_event_bag_dropped.bin",
    "v51_event_bag_sync.bin",
    "v51_event_bag_removed.bin",
    "v51_event_struct_hit_piece.bin",
    "v51_event_struct_hit_deploy.bin",
    "v51_event_vitals.bin",
    "v51_event_consumed.bin",
    "v51_event_consume_refused.bin",
    "v51_action_consume.bin",
    "v51_event_drank.bin",
    "v51_action_drink.bin",
    "v51_event_respawn.bin",
    "v51_action_respawn.bin",
    "v51_action_move.bin",
    "v51_event_moved.bin",
    "v51_event_move_refused.bin",
    "v51_action_move_box.bin",
    "v51_action_container.bin",
    "v51_action_container_close.bin",
    "v51_event_cont_sync.bin",
    "v51_event_cont_close.bin",
    "v51_action_repair_piece.bin",
    "v51_action_repair_deploy.bin",
    "v51_event_piece_repaired_piece.bin",
    "v51_event_piece_repaired_deploy.bin",
    "v51_action_throw_piece.bin",
    "v51_action_throw_deploy.bin",
    "v51_event_charge_placed_piece.bin",
    "v51_event_charge_placed_deploy.bin",
    "v51_challenge.bin",
    "v51_auth.bin",
    "v51_event_oven_lit.bin",
    "v51_event_oven_out.bin",
    // Appended rather than slotted beside `v30_event_door`: the
    // fixture list is positional (`gen_goldens` indexes it), so a new
    // name in the middle silently renumbers every writer after it.
    "v51_event_knock.bin",
    "v51_event_auth.bin",
    "v51_action_access_crew.bin",
    "v51_action_demolish.bin",
    "v51_event_shot.bin",
    // World containers v0 (v37): the fourth container kind. Three
    // fixtures and not one, because `action_move_box`'s own doc records
    // what happens otherwise — the third kind crossed the wire for a
    // whole version with only the *open* pinned, so the bytes that mean
    // "take it out of the box" were checked by nothing. Kind 3 gets its
    // open, its move and its sync in the commit that legalises it.
    "v51_action_container_world.bin",
    "v51_action_move_world.bin",
    "v51_event_cont_sync_world.bin",
    // The bench ladder + tech tree (v38): the unlock action and the
    // research-rows drip, plus the three research-lane events that had
    // ridden unpinned since v32 — the role gate checked their payloads
    // and nothing checked their bytes, which is the exact seat the v37
    // world-container note called out as empty.
    "v51_action_unlock.bin",
    "v51_event_research_rows.bin",
    "v51_event_research.bin",
    "v51_event_research_refused.bin",
    "v51_event_known.bin",
    // The table verb's own action, pinned by the local branch and kept
    // through the 2026-08-15 integration: `encode_action_research` is
    // still live (the client's `verbs.rs` calls it), so
    // `every_encoder_has_a_golden` requires these bytes.
    "v51_action_research.bin",
    // The gather refusal (v42) — appended, because the manifest is
    // positional and a name in the middle silently renumbers every
    // writer after it.
    "v51_event_gather_refused.bin",
    // Bag choice v0 (v43): the own-fact bag list the death screen shapes
    // itself around. Appended for the same positional reason.
    "v51_event_bags.bin",
    // Surface marks v0 (v45): where an arrow stopped, and on what.
    // Appended, like the two above — `gen_goldens` writes this list by
    // INDEX, so inserting anywhere but the end silently re-points every
    // fixture after the insertion at another message's bytes.
    "v51_event_impact.bin",
    "v51_event_swing.bin",
    // Armor v1 (v51): the fifth container kind, and the first that is
    // carried on the player rather than standing in the world. Four
    // fixtures on the v37 precedent above — its open, its move and its
    // sync in the commit that legalises it — plus the refusal, because
    // `REFUSE_M_WEAR` is a reason no fixture has ever carried and the
    // refusal message is the only one that pins a container kind inside
    // an *address* rather than as a field of its own.
    "v51_action_container_wear.bin",
    "v51_action_move_wear.bin",
    "v51_event_cont_sync_wear.bin",
    "v51_event_move_refused_wear.bin",
];

/// The move action: container handle (a bag id, or a packed
/// `box_key(cx, cz, level)` — the kinds say which), from (kind, slot), to
/// (kind, slot), count.
///
/// Every part is distinguishable from every other, which on this message
/// is not decoration — it is the entire defence. `reference/FINDINGS.md`
/// §1 counts ~27 payloads in the reference ecosystem that shipped with
/// the right value in the wrong position, and a positional message with
/// two `(kind, slot)` pairs in it is the exact shape that goes wrong. So:
/// the two kinds differ (self→bag, not self→self), the two slots differ
/// and neither is near the other, and the count matches nothing else in
/// the message. A transposed pair moves bytes, and moved bytes fail
/// `test_protocol_golden`.
pub fn action_move() -> (u32, u8, u8, u8, u8, u16) {
    (0x0BA6_1D01, 0, 3, 1, 17, 42)
}

/// The move acknowledgement. The kinds run the *other* way from
/// `action_move` (bag→self, a withdrawal) so a copy-paste between the two
/// lanes shows up as drifted bytes rather than as a fixture that happens
/// to agree with itself.
pub fn event_moved() -> (u8, u8, u8, u8, u16, u16) {
    (1, 9, 0, 22, 7, 5)
}

/// A refused move: `REFUSE_M_NO_ROOM`, carrying the address that was
/// asked for. The reason is deliberately neither the first nor the last
/// of the seven — a fixture pinned to 1 would pass while the field was
/// being written as a bool.
pub fn event_move_refused() -> (u8, u8, u8, u8, u8) {
    (4, 0, 11, 1, 26)
}

/// A move into a **box** — the meaning wire v18 added and never pinned.
///
/// `action_move` above encodes a `CONT_SELF`→`CONT_BAG` move and is
/// byte-identical to the fixture v17 shipped, so the third container kind
/// crossed the wire for a whole version with no fixture exercising it: the
/// bytes that decode as "put it in the box at this address" were checked
/// by nothing. This is that case.
///
/// The handle is a real `deploy::box_key(291, 744, 5)` — `cx << 16 |
/// cz << 4 | level` — and not a round number, because the whole risk on a
/// packed address is a field sliding within it, and three parts that are
/// each distinguishable from the other two is what makes a slide move
/// bytes. It is also nothing like `action_move`'s bag id, so a fixture
/// copied from that one shows up as drifted bytes rather than as two
/// fixtures agreeing with each other.
///
/// The destination slot is inside `BOX_SLOTS`, so this is a move the sim
/// would also accept — a fixture that only the wire would take is a
/// weaker statement about the same bytes.
pub fn action_move_box() -> (u32, u8, u8, u8, u8, u16) {
    (0x0123_2E85, 0, 6, 2, 9, 13)
}

/// Opening a box (wire v19). The kind is the packed-address one on
/// purpose: the bag case is the easy one, and an address that loses a
/// field is the failure this lane actually has.
///
/// A different `box_key` from `action_move_box` — `box_key(88, 1001, 3)`
/// — for the same reason its handle differs from `action_move`'s: two
/// fixtures that share a constant cannot catch a copy-paste between them.
pub fn action_container() -> (u8, u32) {
    (2, 0x0058_3E93)
}

/// Closing whatever is open. The one legal shape of a close, pinned
/// because "one field, three meanings" is a design a later reader could
/// reasonably talk themselves into widening: the day someone adds a
/// handle to a close, or an `open` bit beside the kind, these bytes move.
pub fn action_container_close() -> (u8, u32) {
    (0, 0)
}

/// An open container's contents, as a **diff** (`reset` false) over a
/// bag — the kind with thirty addressable slots, so the widest slot index
/// on this message is exercised.
///
/// `reset` is deliberately false here and true in `event_cont_close`, so
/// the bit is pinned in both states across the fixture set. A bool that
/// only ever crosses as `true` is a bool no golden has actually checked.
///
/// Three rows, and no two of them share a value in any position: a
/// transposed pair of rows, or a slot that swapped with an item, moves
/// bytes. The slots are spread to the ends of the range rather than
/// bunched, because an off-by-one in the slot field is invisible between
/// 3 and 4 and obvious between 2 and 28.
pub fn event_cont_sync() -> (u8, u32, bool, [InvSlot; 3]) {
    (
        1,
        0x0BA6_2C07,
        false,
        [
            InvSlot {
                slot: 2,
                stack: ItemStack {
                    item: 5,
                    count: 40,
                    cond: 0,
                },
            },
            InvSlot {
                slot: 17,
                stack: ItemStack {
                    item: 31,
                    count: 3,
                    // A worn tool in a container: nonzero, distinct from
                    // every other field in the batch, so a codec that
                    // wrote the halves in the wrong order moves bytes.
                    cond: 4_321,
                },
            },
            InvSlot {
                slot: 28,
                stack: ItemStack {
                    item: 12,
                    count: 77,
                    cond: 9_876,
                },
            },
        ],
    )
}

/// The server shutting a panel: nothing is open, no handle, no slots, and
/// `reset` set. Its own fixture and not a variant of the one above,
/// because this is the message that carries the security property — a
/// client walking out of reach is told, and told in bytes that cannot
/// drift into "here are some contents" without failing here first.
pub fn event_cont_close() -> (u8, u32, bool) {
    (0, 0, true)
}

/// The drink acknowledgement: water restored and the hp it cost. Both
/// nonzero and different from each other, so a transposed pair cannot
/// pass — and the cost nonzero specifically, because a zero there is the
/// wire saying "the sea is fresh", which is the one claim this fixture
/// exists to pin.
pub fn event_drank() -> (u16, u16) {
    (37, 2)
}

fn rng_entity(rng: &mut Pcg32, id: u32) -> EntityState {
    EntityState {
        id,
        // Island interior in quanta (3 cm): ~300 m .. ~1800 m.
        qx: 10_000 + rng.next_bounded(50_000) as i32,
        qy: -1_200 + rng.next_bounded(7_000) as i32,
        qz: 10_000 + rng.next_bounded(50_000) as i32,
        qvy: rng.next_bounded(10_000) as i32 - 5_000,
        grounded: rng.next_bounded(2) == 0,
        // Awake, and drawn from no `rng` call on purpose. A random sleeper
        // would pin whichever value the generator happened to produce,
        // which is a fixture that documents nothing; the cases below set
        // the bit deliberately and say what each one covers. It also keeps
        // the draw sequence where it was, so this field alone does not
        // reshuffle every other field of every later entity.
        sleeping: false,
        // Alive, and drawn from no `rng` call for `sleeping`'s reason
        // exactly: the cases below set this bit deliberately and say which
        // encoder path each one covers, and a random draw here would both
        // document nothing and reshuffle every later field.
        dead: false,
        yaw: rng.next_bounded(0x1_0000) as u16,
        pitch: rng.next_bounded(0x100) as u8,
    }
}

/// Acks-only input datagram (tab-backgrounded client heartbeat): zero
/// frames, header fields still live.
pub fn input_acks_only() -> InputDatagram {
    InputDatagram::new(0xBEEF, 0xA5A5_5A5A, 123_456)
}

/// A full input datagram: `MAX_INPUT_FRAMES` consecutive frames with
/// varied field content, seq run crossing the u16 wrap.
///
/// **`buttons` is drawn across the whole octet, and it was drawn from
/// `next_bounded(4)` until 2026-08-18** — so from v0 to v46 the only
/// fixture that carries a button pinned bits 0–1 and nothing else.
/// `BTN_PRIMARY` and `BTN_JUMP` were outside the draw, and so was every
/// unmeant bit. Nothing on the wire was wrong: the field is eight bits
/// wide either way and the layout was pinned. What no fixture could see is
/// an encoder that masks or reorders the high nibble on its way out —
/// `decode_input`'s doc calls a silently narrowed octet the one wrong
/// answer, and the golden was blind to precisely that.
///
/// The draw is the **wire width**, not `BTN_MASK`: bits 4–7 name no button
/// and the codec carries them whole on purpose, so a fixture that stopped
/// at the mask would re-open half the hole. The seed happens to cover all
/// eight bits set and all eight clear across the ten frames — happens to,
/// so `the_input_golden_fuzzes_the_whole_button_octet` checks it off the
/// fixture bytes rather than trusting it. If a future draw change loses a
/// bit, set it deliberately on a named frame (`rng_entity`'s `sleeping`
/// precedent) rather than rerolling until it comes back.
///
/// Widening it changed this fixture's bytes and **turned no `PROTO_VER`**:
/// what a test feeds an encoder is the test's coverage, not the wire's
/// meaning (`PROTO_VER`'s narrowing-rule section, third clause).
pub fn input_full() -> InputDatagram {
    let mut rng = Pcg32::new(0x0047_4154_4553, 11);
    let mut dg = InputDatagram::new(0x0102, 0xFFFF_FFFF, 0xFFFF_FFFE);
    for i in 0..MAX_INPUT_FRAMES as u16 {
        let f = InputFrame {
            seq: 0xFFFC_u16.wrapping_add(i),
            buttons: rng.next_bounded(0x100) as u8,
            yaw: rng.next_bounded(0x1_0000) as u16,
            pitch: rng.next_bounded(0x100) as u8,
            move_x: rng.next_bounded(255) as i32 as i8,
            move_z: (rng.next_bounded(255) as i32 - 127) as i8,
            // 6 is `HOTBAR_SLOTS`, i.e. the whole encodable domain and not
            // a narrowed one: `encode_input` refuses 6–7 and `decode_input`
            // refuses them back, so a wider draw here would make the
            // fixture unencodable rather than better covered. Read that
            // beside `buttons` above, where the same-looking bound WAS the
            // defect — the two differ because the codec is total over
            // `sel` and deliberately not over `buttons`.
            sel: rng.next_bounded(6) as u8,
        };
        dg.push(f).expect("golden construction is in-cap by design");
    }
    dg
}

/// A snapshot case: header + what the server would feed the encoder. The
/// expected decode is `entities` verbatim (the decoder reconstructs
/// absolutes), so tests compare against these fields directly.
pub struct SnapshotCase {
    pub header: SnapshotHeader,
    pub removed: &'static [u32],
    pub baseline: [EntityState; MAX_SNAPSHOT_ENTITIES],
    pub baseline_len: usize,
    pub entities: [EntityState; MAX_SNAPSHOT_ENTITIES],
    pub entity_len: usize,
}

impl SnapshotCase {
    pub fn baseline(&self) -> &[EntityState] {
        &self.baseline[..self.baseline_len]
    }
    pub fn entities(&self) -> &[EntityState] {
        &self.entities[..self.entity_len]
    }
}

/// Zero-state keyframe (join / ack-gap recovery): three absolute records,
/// no baseline, no removals.
pub fn snapshot_keyframe() -> SnapshotCase {
    let mut rng = Pcg32::new(0x0047_4154_4553, 12);
    let mut entities = [EntityState::default(); MAX_SNAPSHOT_ENTITIES];
    for (i, slot) in entities.iter_mut().take(3).enumerate() {
        *slot = rng_entity(&mut rng, 100 + i as u32);
    }
    // One at-rest body so the elision bit is pinned too.
    entities[1].qvy = 0;
    // One sleeper, so the absolute encoder's sleeping bit is pinned at
    // **true** by some fixture. Without this every entity in every golden
    // carries a zero there and the bit is indistinguishable from padding —
    // which is the positional-payload trap in `CLAUDE.md` wearing its other
    // face: a byte-golden pins bytes, not meanings, and a field that is
    // only ever zero has no byte to pin.
    entities[2].sleeping = true;
    // One corpse, so the absolute encoder's `dead` bit is pinned at **true**
    // by some fixture — the same argument the sleeper above is here for, and
    // a different entity so the two bits are never on together and a swap
    // between them cannot pass. A body still falling at the moment it is
    // killed is the case: the record keeps the position and the velocity it
    // had, which is exactly why the bit has to be carried rather than
    // inferred from any of the others.
    entities[0].dead = true;
    SnapshotCase {
        header: SnapshotHeader {
            tick: 96,
            baseline_age: 0,
            last_executed_seq: 0x0203,
            nudge: Nudge::HardResync,
        },
        removed: &[],
        baseline: [EntityState::default(); MAX_SNAPSHOT_ENTITIES],
        baseline_len: 0,
        entities,
        entity_len: 3,
    }
}

/// The delta paths, all of them in one packet: an unchanged entity, a
/// small move + look change, a velocity-only change, a teleport that falls
/// back to absolute, a brand-new entity, and two removals.
pub fn snapshot_delta() -> SnapshotCase {
    let mut rng = Pcg32::new(0x0047_4154_4553, 13);
    let mut baseline = [EntityState::default(); MAX_SNAPSHOT_ENTITIES];
    for (i, slot) in baseline.iter_mut().take(4).enumerate() {
        *slot = rng_entity(&mut rng, 1 + i as u32);
    }
    let mut entities = [EntityState::default(); MAX_SNAPSHOT_ENTITIES];
    // id 1: unchanged — the minimal record. 39 bits: it was 37, then v26
    // added the sleeping bit beside `grounded` and v48 the dead bit beside
    // that, and neither is delta-gated, so the floor rises by one each time.
    entities[0] = baseline[0];
    // id 2: one snapshot interval of sprint + a look turn.
    entities[1] = baseline[1];
    entities[1].qx += 12;
    entities[1].qy -= 3;
    entities[1].qz += 7;
    entities[1].yaw = entities[1].yaw.wrapping_add(0x0400);
    entities[1].pitch = entities[1].pitch.wrapping_add(3);
    // …and was killed on the way — the delta encoder's `dead` bit at
    // **true**, as a TRANSITION (baseline alive → record dead) rather than a
    // body that was already a corpse in both. That is the case a
    // change-gated bit would have got wrong, and it is the ordinary one: a
    // player dies mid-stride and the body keeps moving for the ticks it
    // takes to settle (`sim-core/world.rs` — a corpse still falls, it just
    // does not act).
    entities[1].dead = true;
    // id 3: starts falling, and its owner disconnected on the same
    // snapshot — the delta encoder's sleeping bit at **true**, and the
    // transition (baseline awake → record asleep) rather than a body that
    // was already asleep in both. That is the case a change-gated bit would
    // have got wrong, and it is a real one: a player who logs off in the
    // air keeps falling.
    entities[2] = baseline[2];
    entities[2].qvy = -450;
    entities[2].grounded = false;
    entities[2].sleeping = true;
    // id 4: teleported beyond the delta window — absolute fallback, and
    // asleep in both baseline and record, so the absolute path is pinned
    // carrying a true it did not have to change to.
    baseline[3].sleeping = true;
    entities[3] = baseline[3];
    entities[3].qx += 600;
    entities[3].qz -= 600;
    // id 5: entered the interest set this snapshot.
    entities[4] = rng_entity(&mut rng, 5);
    SnapshotCase {
        header: SnapshotHeader {
            tick: 3_000,
            baseline_age: 4,
            last_executed_seq: 0x7788,
            nudge: Nudge::Faster,
        },
        removed: &[90, 91],
        baseline,
        baseline_len: 4,
        entities,
        entity_len: 5,
    }
}

/// The bidi-lane handshake trio (DESIGN.md §5.9), fixed values so the
/// stream lane is golden-pinned like the datagrams.
pub fn hello() -> Hello {
    Hello {
        proto_ver: crate::PROTO_VER,
        // **Literals, not `version::VER`/`version::BUILD`.** A fixture keyed
        // on this build's own version would change bytes on every release and
        // on every commit, so the golden would need regenerating for reasons
        // that have nothing to do with the wire — which is the one thing a
        // byte-golden must never do. These are stand-ins whose only job is to
        // exercise the field widths: 1_002_003 is 1.2.3 packed, and the digest
        // is a fixed pattern that is asymmetric across its two halves so a
        // swapped-halves encoder cannot pass.
        ver: 1_002_003,
        build: 0x0123_4567_89ab_cdef,
    }
}

/// The server's challenge. Every nonce byte is distinct so a transposition
/// in either direction of the codec shows as a wrong value rather than a
/// coincidence — the positional-payload rule, applied to the one message
/// whose bytes a signature is computed over.
pub fn challenge() -> crate::Challenge {
    let mut nonce = [0u8; crate::NONCE_BYTES];
    for (i, b) in nonce.iter_mut().enumerate() {
        *b = (i as u8).wrapping_mul(7).wrapping_add(3);
    }
    crate::Challenge {
        nonce,
        issued_at: 1_770_000_123,
    }
}

/// The client's answer. A real-shaped address and a signature whose bytes
/// are all distinct — **not** `Signature::NONE`, because a fixture of zeros
/// would ship green past any defect that dropped the field entirely, which
/// is the same lesson the old token fixture was written for.
pub fn auth() -> crate::Auth {
    let mut sig = [0u8; crate::SIGNATURE_BYTES];
    for (i, b) in sig.iter_mut().enumerate() {
        *b = (i as u8).wrapping_mul(5).wrapping_add(11);
    }
    crate::Auth {
        address: crate::Address::from_hex(b"0x7e5f4552091a69125d5dfcb7b8c2659029395bdf")
            .expect("a known-good address"),
        signature: crate::Signature(sig),
    }
}

/// `dev` is pinned TRUE here on purpose: a false bit is byte-identical to
/// the zero padding it sits in, so only the set bit actually locks the
/// field's position in the fixture. The false case is covered by the
/// roundtrip in `tests/protocol_golden.rs`.
pub fn welcome() -> Welcome {
    Welcome {
        player_id: 0x0000_0107,
        seed: 0x0047_4154_4553_2121,
        tick: 654_321,
        dev: true,
    }
}

pub fn refuse_full() -> Refuse {
    Refuse {
        code: crate::REFUSE_FULL,
    }
}

// ---------------------------------------------------------------------------
// Event-lane cases (v1): fixed values so the reliable lane is golden-pinned
// like everything else. Encoders live in `event.rs`; the tests encode from
// these and compare bytes and decodes.
// ---------------------------------------------------------------------------

pub fn event_gather() -> (u16, u16) {
    (7, 13)
}

/// The gather refusal (wire v42): the held item and the reason, both
/// nonzero and distinct so a transposed pair moves bytes — and the item
/// deliberately NOT `NO_ITEM`, because the whole point of the field is
/// naming the torch rather than saying "bare hands".
pub fn event_gather_refused() -> (u16, u8) {
    (23, 2)
}

/// A worst-shape inventory update: every slot changed.
pub fn event_inv() -> ([InvSlot; INV_SLOTS], usize) {
    let mut rng = Pcg32::new(0x0047_4154_4553, 16);
    let mut slots = [InvSlot::default(); INV_SLOTS];
    for (i, s) in slots.iter_mut().enumerate() {
        *s = InvSlot {
            slot: i as u8,
            stack: ItemStack {
                item: rng.next_bounded(64) as u16,
                count: rng.next_bounded(1000) as u16,
                // The condition field (wire v42), drawn from the same
                // stream so the fixture pins it in thirty different
                // states rather than one.
                cond: rng.next_bounded(40_001) as u16,
            },
        };
    }
    (slots, INV_SLOTS)
}

pub fn event_slot_change() -> (u16, u16) {
    (0x0102, 0x0304)
}

/// A full sync batch with the reset bit set — the join-sync first message
/// at its cap.
pub fn event_slot_sync() -> (bool, [(u16, u16); SLOT_SYNC_BATCH]) {
    let mut rng = Pcg32::new(0x0047_4154_4553, 17);
    let cells =
        core::array::from_fn(|_| (rng.next_bounded(256) as u16, rng.next_bounded(256) as u16));
    (true, cells)
}

/// A weak-mark message with the weak-hit bit set: (cx, cz, mark8, weak_hit).
pub fn event_weak_mark() -> (u16, u16, u8, bool) {
    (0x0102, 0x0304, 0xA7, true)
}

/// A catalog whose first batch is exactly `CATALOG_BATCH` names of mixed
/// length — the fixture encodes the batch at `first = 0`. The ceilings
/// (v46) mix 0 (no condition) with real values and the u16 corner so the
/// golden pins the column's width and order, not just its presence.
pub fn event_catalog() -> ItemCatalog {
    let mut cat = ItemCatalog::EMPTY;
    cat.count = 11;
    let rows: [(&[u8], u16); 11] = [
        (b"Wood", 0),
        (b"Stone", 0),
        (b"Metal Ore", 0),
        (b"Sulfur Ore", 0),
        (b"Cloth", 0),
        (b"Animal Fat", 0),
        (b"Charcoal", 40_000),
        (b"Fixture Name Of Width 24", u16::MAX),
        (b"Sulfur", 0),
        (b"Gunpowder", 1),
        (b"Low Grade Fuel", 0),
    ];
    for (i, (n, cm)) in rows.iter().enumerate() {
        cat.set(i, n, *cm)
            .expect("golden names are in-cap by design");
    }
    cat
}

/// A part-full craft queue (head mid-batch) with a live head timer.
pub fn event_craft_q() -> ([CraftJob; 3], u16) {
    (
        [
            CraftJob {
                recipe: 5,
                remaining: 2,
            },
            CraftJob {
                recipe: 0,
                remaining: 99,
            },
            CraftJob {
                recipe: 63,
                remaining: 1,
            },
        ],
        777,
    )
}

/// One completed unit: (item index, units that actually landed).
pub fn event_craft_done() -> (u16, u16) {
    (12, 3)
}

/// A refusal carrying `sim_core::craft::REFUSE_INPUTS`.
pub fn event_craft_refused() -> u8 {
    sim_core::craft::REFUSE_INPUTS as u8
}

/// A six-row recipe table whose first batch is exactly `RECIPE_BATCH`
/// rows of mixed station / input-count shapes.
pub fn event_recipes() -> CraftContent {
    /// (output, out_count, ticks, station, inputs) — fixture shorthand.
    type Row = (u16, u16, u32, u8, &'static [(u16, u16)]);
    let mut cc = CraftContent::EMPTY;
    cc.recipe_count = 6;
    let rows: [Row; 6] = [
        (4, 1, 15 * 30, STATION_NONE, &[(0, 100), (1, 50)]),
        (9, 3, 5 * 30, STATION_NONE, &[(0, 25), (1, 10)]),
        // The two ladder rungs v38 minted, in the first batch so the
        // widened station field is pinned at values a v37 decoder never
        // accepted (bench ladder v0).
        (20, 10, 5 * 30, STATION_WORKBENCH2, &[(6, 20), (8, 10)]),
        (
            31,
            1,
            30 * 30,
            STATION_WORKBENCH3,
            &[(20, 240), (12, 2), (13, 1), (5, 4)],
        ),
        (7, 1, 2 * 30, STATION_FURNACE, &[(2, 1)]),
        (63, 255, 65_535 * 30, STATION_WORKBENCH1, &[(62, 65_535)]),
    ];
    for (i, &(output, out_count, ticks, station, inputs)) in rows.iter().enumerate() {
        let mut def = RecipeDef {
            output,
            out_count,
            ticks,
            station,
            // Alternating, so the golden pins the bit in BOTH positions:
            // a fixture where every row agreed would be byte-identical
            // under an encoder that wrote a constant (research v0).
            blueprint: i % 2 == 1,
            n_inputs: inputs.len() as u8,
            inputs: [(0, 0); MAX_RECIPE_INPUTS],
        };
        def.inputs[..inputs.len()].copy_from_slice(inputs);
        cc.recipes[i] = def;
    }
    cc
}

/// A craft request: (recipe index, count).
pub fn action_craft() -> (u16, u16) {
    (33, 5)
}

/// A tech-tree unlock request naming recipe 21 (tech tree v0) — a value
/// sharing no byte with `action_craft`'s, so a transposed encoder cannot
/// pass both.
pub fn action_unlock() -> u16 {
    21
}

/// A five-row research table whose first batch is exactly
/// `RESEARCH_BATCH` rows: a free root, a priced root, and two chained
/// nodes — so the wire's `0xFF`-means-no-parent spelling is pinned in
/// both positions, exactly as the recipes fixture alternates its
/// blueprint bit.
pub fn event_research_rows() -> ResearchContent {
    let mut rc = ResearchContent::EMPTY;
    rc.coin = 9;
    rc.row_count = 5;
    let rows: [(u16, u16, u16, u16); 5] = [
        (11, 3, 0, NO_RECIPE),
        (23, 21, 20, NO_RECIPE),
        (31, 33, 40, 21),
        (47, 55, 120, 33),
        (60, 63, 65_535, 55),
    ];
    for (i, &(item, recipe, cost, requires)) in rows.iter().enumerate() {
        rc.rows[i] = ResearchRow {
            item,
            recipe,
            cost,
            requires,
        };
    }
    rc
}

/// A blueprint learned: (recipe, cost burned).
pub fn event_research() -> (u16, u16) {
    (33, 40)
}

/// A research refusal carrying `REFUSE_R_PARENT` — a reason v38 minted,
/// so the fixture pins a byte no earlier version could produce.
pub fn event_research_refused() -> u8 {
    sim_core::research::REFUSE_R_PARENT as u8
}

/// A known-mask restate. Bits 0, 4, 21, 33 and 63: both ends of the
/// field and the two recipes the fixtures above unlock, so a halved or
/// byte-swapped mask cannot reproduce it.
pub fn event_known() -> u64 {
    (1 << 0) | (1 << 4) | (1 << 21) | (1 << 33) | (1 << 63)
}

/// Your own bags: a **partial** rack, so the count is pinned as something
/// other than zero and other than `BAG_CAP`, and a **mixed** one, so the
/// ready bit is pinned in both states.
///
/// Every field is distinguishable from every other for `action_move`'s
/// reason — this is a positional payload with three numbers per entry, the
/// shape `reference/FINDINGS.md` §1 counts ~27 shipped transpositions of.
/// So no two entries share a `cx`, no `cx` equals its own `cz`, the levels
/// differ, and the ready bits alternate: a swapped pair moves bytes.
pub fn event_bags() -> ([BagAnchor; BAG_CAP], usize) {
    let mut bags = [BagAnchor::default(); BAG_CAP];
    bags[0] = BagAnchor {
        cx: 341,
        cz: 682,
        level: 0,
        ready: true,
    };
    bags[1] = BagAnchor {
        cx: 17,
        cz: 900,
        level: 3,
        ready: false,
    };
    bags[2] = BagAnchor {
        cx: 1023,
        cz: 4,
        level: 7,
        ready: true,
    };
    (bags, 3)
}

/// A cancel of queue job 2.
pub fn action_cancel() -> u16 {
    2
}

/// A place request: (row, cx, cz, level, loc) — a stone wall on a cell's
/// low-z edge, one storey up.
/// The freehand bit is pinned **true** here, not defaulted false: a fixture
/// whose new field carries the zero value is a fixture that cannot tell a
/// live bit from a dropped one, which is the positional-payload trap
/// (CLAUDE.md) one field over. Proven by deleting the `w.write` in
/// `encode_action_place` — with `false` the bytes are unchanged.
pub fn action_place() -> (u16, u16, u16, u8, u8, bool) {
    (13, 341, 682, 1, sim_core::build::LOC_EDGE_ZLO, true)
}

/// The piece record behind the placed broadcast.
pub fn event_piece_placed() -> PieceRec {
    PieceRec {
        cx: 341,
        cz: 682,
        level: 1,
        loc: sim_core::build::LOC_EDGE_ZLO,
        row: 13,
        // Set, not defaulted: the fixture pins the facing bit's live value
        // (wire v39) the way the defs fixture pins shape 7 — a bit that
        // never carries a 1 is a bit nothing gates.
        facing: 1,
        // And the plate NEGATIVE (build plate v1, wire v49), for the same
        // rule one line up and one turn sharper: `plate` is the only signed
        // field on this record, it is written biased, and every way of
        // getting a bias wrong — the wrong constant, an unsigned read, a
        // width one short — agrees with a correct encoder on zero. A fixture
        // carrying 0 would pin bytes that cannot tell the two apart.
        plate: -1,
        ..PieceRec::default()
    }
}

/// A full piece-sync batch with the reset bit set — the join-sync first
/// message at its cap.
pub fn event_piece_sync() -> (bool, [PieceRec; PIECE_SYNC_BATCH]) {
    let mut rng = Pcg32::new(0x0047_4154_4553, 18);
    let recs = core::array::from_fn(|i| PieceRec {
        cx: rng.next_bounded(1024) as u16,
        cz: rng.next_bounded(1024) as u16,
        level: rng.next_bounded(8) as u8,
        // All ten live locs since v40, so the batch pins triangle and
        // diagonal addresses in golden bytes, not just the square four.
        loc: rng.next_bounded(10) as u8,
        row: rng.next_bounded(32) as u8,
        facing: rng.next_bounded(2) as u8,
        // The whole FIELD, both signs — the batch is where a width is pinned
        // across many records, so it covers the range rather than a value
        // (`event_piece_placed` pins the single deliberate one).
        //
        // **The wire's range, not the sim's knobs.** `PLATE_BITS` is
        // deliberately wider than `build::PLATE_RISE_MAX_BANDS` and
        // `PLATE_SINK_MAX_BANDS`, and its own doc says why: those two are
        // balance knobs, and a wire field sized to today's knob is a
        // `PROTO_VER` bump for every balance pass. A fixture cycled off the
        // knobs has the same defect one layer up, and it fired the day it was
        // written — adopting the reference's ±half-a-wall offset moved every
        // byte of this golden for a reason that has nothing to do with the
        // wire. The field's own width pins what the LAYOUT can carry, which
        // is what a wire golden is for, and it stays put while balance moves.
        //
        // **Cycled off the index, deliberately NOT drawn from `rng`.** Every
        // field above shares one stream, so an extra draw per record shifts
        // every later one — and the first version of this line did draw,
        // which slid the `loc` sequence far enough that one of the ten
        // addresses stopped appearing in the batch at all.
        // `the_loc_fuzz_covers_each_stores_whole_domain` caught it, which is
        // that gate's whole reason for existing.
        plate: (i as i32 % (1 << PLATE_BITS) - PLATE_BIAS) as i8,
        ..PieceRec::default()
    });
    (true, recs)
}

/// A refusal carrying `sim_core::build::REFUSE_B_SUPPORT`.
pub fn event_build_refused() -> u8 {
    sim_core::build::REFUSE_B_SUPPORT as u8
}

/// A seven-row piece table whose first batch is exactly
/// `PIECE_DEFS_BATCH` rows of mixed shape/material/cost-count shapes.
pub fn event_piece_defs() -> BuildContent {
    /// (shape, material, hp, costs) — fixture shorthand.
    type Row = (u8, u8, u16, &'static [(u16, u16)]);
    let mut bc = BuildContent::EMPTY;
    bc.piece_count = 7;
    let rows: [Row; 7] = [
        (sim_core::build::SHAPE_FOUNDATION, 0, 750, &[(0, 350)]),
        (sim_core::build::SHAPE_WALL, 1, 1750, &[(1, 350)]),
        (sim_core::build::SHAPE_DOORWAY, 2, 3000, &[(7, 160)]),
        // The window keeps this row's two costs: the 2-bit n_costs field
        // stays pinned at its top by the same row that now pins a
        // catalogue-v1 shape code.
        (sim_core::build::SHAPE_WINDOW, 0, 750, &[(0, 350), (4, 10)]),
        // The tri roof is 10 — the TOP live code of the 4-bit shape field
        // since v40 (the frame held this seat while the field was 3 bits),
        // pinned here the way the roof row pins material's top: a width
        // that never carries its widest value is a width nothing gates.
        (sim_core::build::SHAPE_TRI_ROOF, 1, 1750, &[(1, 200)]),
        // The max-everything row, and the one that pins the TOP of the
        // 2-bit material field: metal is 3 since twig v0, and a fixture
        // that never wrote a 3 would leave the widest rung ungated.
        (sim_core::build::SHAPE_ROOF, 3, 65_535, &[(65_535, 65_535)]),
        (sim_core::build::SHAPE_WALL, 0, 750, &[(0, 350)]),
    ];
    for (i, &(shape, material, hp, costs)) in rows.iter().enumerate() {
        let mut def = PieceDef {
            shape,
            material,
            hp,
            n_costs: costs.len() as u8,
            costs: [(0, 0); MAX_PIECE_COSTS],
        };
        def.costs[..costs.len()].copy_from_slice(costs);
        bc.pieces[i] = def;
    }
    bc
}

/// A deploy-place request: (row, cx, cz, level, loc) — a door into a
/// doorway on a cell's low-x edge.
pub fn action_deploy() -> (u16, u16, u16, u8, u8) {
    (9, 341, 682, 0, sim_core::build::LOC_EDGE_XLO)
}

/// A feed of the hearth at (cx, cz, level).
pub fn action_feed() -> (u16, u16, u8) {
    (341, 682, 0)
}

/// The deployable record behind the placed broadcast (owner/hp/uh are
/// sim-side and never cross — the fixture keeps their defaults). All
/// three door bits are set so the wire's newest ones are pinned at 1
/// somewhere, and set independently below so their order is pinned too.
pub fn event_deploy_placed() -> DeployRec {
    DeployRec {
        cx: 341,
        cz: 682,
        level: 1,
        loc: sim_core::build::LOC_PLANE,
        row: 9,
        open: true,
        locked: true,
        has_lock: true,
        ..DeployRec::default()
    }
}

/// A full deploy-sync batch with the reset bit set.
///
/// `loc`'s `next_bounded(4)` **looks like `input_full`'s old button draw
/// and is not the same thing** (checked 2026-08-18, NOW.md §5c). The
/// deployable store lives on the plane and the two straight edges, so its
/// whole domain is `0..=LOC_EDGE_ZLO` — `loc_max(true)`, which
/// `encode_event_deploy_sync` enforces — and a wider draw would make this
/// fixture *unencodable*, not better covered. The field's own width is
/// four bits since v40 and its top two are pinned elsewhere:
/// `event_piece_sync` draws `next_bounded(10)`, the piece store's whole
/// domain including the diagonals. So the octet-shaped hole does not exist
/// here; every value either store can send appears in some fixture, and
/// `the_loc_fuzz_covers_each_stores_whole_domain` is what says so.
pub fn event_deploy_sync() -> (bool, [DeployRec; DEPLOY_SYNC_BATCH]) {
    let mut rng = Pcg32::new(0x0047_4154_4553, 19);
    let recs = core::array::from_fn(|_| DeployRec {
        cx: rng.next_bounded(1024) as u16,
        cz: rng.next_bounded(1024) as u16,
        level: rng.next_bounded(8) as u8,
        loc: rng.next_bounded(4) as u8,
        row: rng.next_bounded(16) as u8,
        open: rng.next_bounded(2) == 0,
        locked: rng.next_bounded(2) == 0,
        has_lock: rng.next_bounded(2) == 0,
        ..DeployRec::default()
    });
    (true, recs)
}

/// A use request: the address of a door on a cell's low-x edge.
pub fn action_use() -> (u16, u16, u8, u8) {
    (341, 682, 0, sim_core::build::LOC_EDGE_XLO)
}

/// A lock request: the same address, setting a code. Both payload fields
/// are **nonzero** on purpose, and it is the same reason wire v8's single
/// bit was 1 — at 0 the frame is byte-identical to one whose encoder never
/// wrote the field at all (the padding absorbs the width), so a new C→S
/// field would cross the golden gate ungated. The op is `SET_CODE` rather
/// than the numerically larger `TAKE` because the code only *means*
/// something on an op that carries one, and a golden whose two fields are
/// mutually incoherent teaches a reader the wrong shape.
pub fn action_access() -> (u16, u16, u8, u8, u8, u16) {
    (
        341,
        682,
        0,
        sim_core::build::LOC_EDGE_XLO,
        sim_core::deploy::ACCESS_OP_SET_CODE,
        4207,
    )
}

/// A demolish request: `action_repair`'s address and its store bit, on
/// purpose — the two verbs address the same pair of stores the same way,
/// so a golden that used a different address would hide a divergence
/// between them rather than pin it. The bit is **1** (the deployable
/// half) for `action_lock`'s reason: at 0 the frame is byte-identical to
/// one whose encoder never wrote the field.
pub fn action_demolish() -> (bool, u16, u16, u8, u8) {
    (true, 341, 682, 0, sim_core::build::LOC_EDGE_XLO)
}

/// A crew op: the same cell, on the **body** rather than an edge, joining
/// a hearth's crew (hearth crew v1). Its own fixture beside the lock's
/// because the two halves of `ACT_ACCESS` differ in exactly the field a
/// byte-golden is blind to — the op picks which *store* the address means,
/// so a swap there encodes clean and acts on the wrong thing
/// (`reference/FINDINGS.md` §1's bug class, on our newest verb).
///
/// The code rides as `CODE_NONE`: a crew op carries no code, and pinning
/// the sentinel is what stops the encoder quietly sending a real one.
pub fn action_access_crew() -> (u16, u16, u8, u8, u8, u16) {
    (
        341,
        682,
        0,
        sim_core::build::LOC_PLANE,
        sim_core::deploy::ACCESS_OP_CREW_JOIN,
        sim_core::lock::CODE_NONE,
    )
}

/// An upgrade request: the same address again, climbing to metal. The
/// material is **nonzero** on purpose — at 0 the frame would be
/// byte-identical to one whose encoder never wrote the field (the padding
/// absorbs the width), so wire v9's only new C→S field would cross the
/// golden gate ungated. It reads 3 since twig v0 took code 0 and pushed
/// the ladder up, which is one of the two reasons v34's bytes moved.
pub fn action_upgrade() -> (u16, u16, u8, u8, u8) {
    (
        341,
        682,
        0,
        sim_core::build::LOC_EDGE_XLO,
        sim_core::build::MAT_METAL,
    )
}

/// A door announcement: the same address, now open, still locked and
/// still carrying its keypad — the state its owner sees after swinging
/// their own door. All three bits are **1** for `action_access`'s reason.
pub fn event_door() -> (u16, u16, u8, u8, bool, bool, bool) {
    (341, 682, 0, sim_core::build::LOC_EDGE_XLO, true, true, true)
}

/// A knock: the same door, and player 9 outside it. The knocker is
/// nonzero for `action_access`'s reason, and is deliberately *not* the
/// player id any other fixture uses, so a crossed `a`/`c` at the emit
/// site shows up as a changed golden rather than as a coincidence
/// (`reference/FINDINGS.md` §1).
pub fn event_knock() -> (u16, u16, u8, u8, u32) {
    (341, 682, 0, sim_core::build::LOC_EDGE_XLO, 9)
}

/// A grant: the same door, at full rights. `GRANT_FULL` is 2, so the
/// two-bit field is nonzero in both of its bits — the strongest form of
/// `action_access`'s rule for a field this narrow.
pub fn event_auth() -> (u16, u16, u8, u8, u8) {
    (
        341,
        682,
        0,
        sim_core::build::LOC_EDGE_XLO,
        sim_core::lock::GRANT_FULL,
    )
}

/// A chat line the sanitizer keeps whole: multi-byte UTF-8 (an em dash
/// and an apostrophe) and interior spaces, so the golden pins that the
/// wire carries bytes and not ASCII.
pub fn chat_line() -> ChatText {
    ChatText::sanitize("wall's at 4 — bring stone".as_bytes()).expect("a clean line")
}

/// The C→S frame: local channel (the default one a player speaks on).
pub fn chat() -> (ChatText, bool) {
    (chat_line(), false)
}

/// The S→C relay of that line, on the global channel from player 7.
pub fn event_chat() -> (u32, bool, ChatText) {
    (7, true, chat_line())
}

/// A landed melee hit: the wooden spear's 25 on another player.
pub fn event_hit() -> (u32, u16) {
    (4_242, 25)
}

/// A health readout partway down the shipped 100-hp bar — three spear
/// hits taken, one left before the fourth ends it.
pub fn event_health() -> (u16, u16) {
    (25, 100)
}

/// A kill: victim and killer, both real ids, neither zero (a zero would
/// let a decoder's default pass for a value) — and, since v16, the three
/// fields the death screen is made of. `DEATH_BY_HAND` is deliberately
/// *not* the cause here: it is code 0, and a fixture that pinned the
/// default could not tell an encoder that dropped the field from one that
/// wrote it. `DEATH_BY_SALT` (2) is the top of the two-bit range, so the
/// same fixture also pins the width. The weapon is a real row and the
/// range is a real reach, both nonzero and different from each other and
/// from every other number in the tuple.
pub fn event_death() -> (u32, u32, u8, u16, u16) {
    (4_242, 7, 2, 13, 187)
}

/// A wake on the player's own bag. `true` and not `false` for the reason
/// the cause above is not zero: a one-bit field pinned at its default is a
/// fixture an encoder that never wrote the bit would still pass.
pub fn event_respawn() -> bool {
    true
}

/// The beach half of the choice — `false`, so the action fixture and the
/// event fixture pin opposite values of the two one-bit fields v16 adds
/// and neither can be satisfied by a stuck encoder.
pub fn action_respawn() -> bool {
    false
}

/// A meter pair partway down the shipped 100/100 ceilings, with food and
/// water at *different* values and both below their maxima — four distinct
/// numbers, so a codec that transposed a pair or echoed a ceiling could
/// not produce these bytes.
pub fn event_vitals() -> (u16, u16, u16, u16) {
    (62, 38, 100, 100)
}

/// An eat: a real item index out of a hotbar slot that is neither 0 nor
/// the last, so an off-by-one on either end shows.
pub fn event_consumed() -> (u16, u8) {
    (11, 3)
}

/// A refusal carrying `survival::REFUSE_C_FULL` — the nonzero reason the
/// codec insists on.
pub fn event_consume_refused() -> u8 {
    2
}

/// The eat verb aimed at a backpack slot rather than the belt, so the
/// five-bit width is exercised past the six hotbar slots.
pub fn action_consume() -> u8 {
    19
}

/// A death backpack standing in the island interior, in the same quanta
/// an entity's position uses. Nonzero on every axis and a nonzero id: a
/// zero anywhere would let a decoder's default pass for a value.
pub fn event_bag_dropped() -> WireBag {
    WireBag {
        id: 9_001,
        qx: 34_133,
        qy: 512,
        qz: 22_050,
    }
}

/// A full standing-bag batch: `BAG_SYNC_BATCH` bags spread across the
/// island, `reset` set — the shape a join walk's first message takes,
/// and the widest this subtype gets.
pub fn event_bag_sync() -> (bool, [WireBag; BAG_SYNC_BATCH]) {
    let mut rng = Pcg32::new(0x0042_4147_5359, 21);
    let mut recs = [WireBag::default(); BAG_SYNC_BATCH];
    for (i, r) in recs.iter_mut().enumerate() {
        *r = WireBag {
            id: 500 + i as u32,
            qx: 10_000 + rng.next_bounded(50_000) as i32,
            qy: -1_200 + rng.next_bounded(7_000) as i32,
            qz: 10_000 + rng.next_bounded(50_000) as i32,
        };
    }
    (true, recs)
}

/// A bag that someone else emptied before you got there — the reason
/// that is not "it timed out", so the fixture pins the field that tells
/// them apart.
pub fn event_bag_removed() -> (u32, u8) {
    (9_001, sim_core::backpack::BAG_GONE_EMPTIED as u8)
}

/// A raid swing landing on a wall that is still standing. Two fixtures,
/// one per store, because the row field is a different width in each
/// (`PIECE_ROW_BITS` vs `DEPLOY_ROW_BITS`) — a single-store fixture would
/// pin only half the layout. Every field is nonzero and distinct: a zero
/// or a repeat anywhere would let a decoder reading the wrong field pass.
/// `(deploy, cx, cz, level, loc, row, damage, left)`.
pub fn event_struct_hit_piece() -> (bool, u16, u16, u8, u8, u8, u16, u16) {
    (
        false,
        341,
        347,
        3,
        sim_core::build::LOC_EDGE_ZLO,
        7,
        4,
        1_746,
    )
}

pub fn event_struct_hit_deploy() -> (bool, u16, u16, u8, u8, u8, u16, u16) {
    (true, 512, 128, 1, sim_core::build::LOC_EDGE_XLO, 5, 3, 197)
}

/// A refusal carrying `sim_core::deploy::REFUSE_D_CLAIM`.
pub fn event_deploy_refused() -> u8 {
    sim_core::deploy::REFUSE_D_CLAIM as u8
}

/// The deploy-def table (the sim's probe fixture is already the mixed
/// shape the drip needs: four archetypes over four placements).
pub fn event_deploy_defs() -> DeployContent {
    DeployContent::probe_fixture()
}

/// The removed-piece address: (cx, cz, level, loc).
pub fn event_removed() -> (u16, u16, u8, u8) {
    (341, 682, 0, sim_core::build::LOC_EDGE_ZLO)
}

/// A feed ack: hearth address + three stock rows.
pub fn event_stock() -> (u16, u16, u8, [(u16, u32); 3]) {
    (341, 682, 0, [(44, 1_900), (38, 0), (25, 123_456)])
}

/// The worst-case shape (DESIGN.md §12 `test_snapshot_budget` at the
/// protocol layer): a zero-state snapshot at the interest-set cap, every
/// record absolute and every body moving, so nothing is elided. This must
/// fit `DATAGRAM_BUDGET_BYTES` — the budget test asserts it.
pub fn snapshot_cap() -> SnapshotCase {
    let mut rng = Pcg32::new(0x0047_4154_4553, 14);
    let mut entities = [EntityState::default(); MAX_SNAPSHOT_ENTITIES];
    for (i, slot) in entities.iter_mut().enumerate() {
        let mut e = rng_entity(&mut rng, 1_000 + i as u32);
        if e.qvy == 0 {
            e.qvy = -1; // keep every velocity on the wire
        }
        *slot = e;
    }
    SnapshotCase {
        header: SnapshotHeader {
            tick: 0xFFFF_FFFF,
            baseline_age: 0,
            last_executed_seq: 0xFFFF,
            nudge: Nudge::Slower,
        },
        removed: &[],
        baseline: [EntityState::default(); MAX_SNAPSHOT_ENTITIES],
        baseline_len: 0,
        entities,
        entity_len: MAX_SNAPSHOT_ENTITIES,
    }
}

/// The repair request: `action_use`'s address, deliberately — the two
/// verbs address a structure the same way, so a fixture that differed
/// would prove only that two constants differ. `loc` is an edge rather
/// than 0 so a dropped `loc` field cannot pass as a zero.
///
/// Two fixtures, one per store, for `event_struct_hit_*`'s reason: the
/// leading bit is the whole point of v21 and a single-store fixture would
/// pin only one of its two values. The deploy one carries a different
/// address as well, so a decoder that read the bit but then took the
/// wrong branch cannot land on the same bytes.
/// `(deploy, cx, cz, level, loc)`.
pub fn action_repair_piece() -> (bool, u16, u16, u8, u8) {
    (false, 341, 682, 2, sim_core::build::LOC_EDGE_ZLO)
}

pub fn action_repair_deploy() -> (bool, u16, u16, u8, u8) {
    (true, 512, 128, 1, sim_core::build::LOC_EDGE_XLO)
}

/// A repair landing: the address, what the payment bought back, and where
/// the structure now stands.
///
/// `healed` and `hp` are different, both nonzero, and `healed < hp` — the
/// three things a transposed or dropped pair could not survive. `row` is
/// nonzero for the same reason `action_access`'s bit is 1: a zero there is
/// byte-identical to a field the encoder never wrote.
///
/// Two fixtures again, and here the bit also *sizes* the row field
/// (`PIECE_ROW_BITS` vs `DEPLOY_ROW_BITS`), so the deploy row is chosen
/// inside four bits — a row of 4 would encode identically at both widths
/// and prove nothing about the branch.
/// `(deploy, cx, cz, level, loc, row, healed, hp)`.
pub fn event_piece_repaired_piece() -> (bool, u16, u16, u8, u8, u8, u16, u16) {
    (
        false,
        341,
        682,
        2,
        sim_core::build::LOC_EDGE_ZLO,
        200,
        610,
        1750,
    )
}

pub fn event_piece_repaired_deploy() -> (bool, u16, u16, u8, u8, u8, u16, u16) {
    (true, 512, 128, 1, sim_core::build::LOC_EDGE_XLO, 11, 37, 60)
}

/// The plant request. `action_repair_*`'s layout to the bit, which is
/// exactly why these addresses are **different** from the repair
/// fixtures': the two encoders write identical field sequences and differ
/// only in the subtype, so sharing an address would let a decoder arm
/// wired to the wrong verb produce bytes that still matched. Two fixtures,
/// one per store value, for `action_repair_*`'s reason.
/// `(deploy, cx, cz, level, loc)`.
pub fn action_throw_piece() -> (bool, u16, u16, u8, u8) {
    (false, 733, 91, 5, sim_core::build::LOC_RISER)
}

pub fn action_throw_deploy() -> (bool, u16, u16, u8, u8) {
    (true, 64, 900, 3, sim_core::build::LOC_PLANE)
}

/// A fuse burning at an address: the address, and how long is left.
///
/// `fuse` is nonzero (a zero is refused at both ends) and is deliberately
/// not a round number of seconds at any plausible tick rate, so a fixture
/// that had quietly become "seconds" rather than "ticks" would not still
/// look right. `row` is nonzero for `event_piece_repaired_*`'s reason, and
/// the deploy row is again chosen inside four bits so the width the store
/// bit selects is actually pinned.
/// `(deploy, cx, cz, level, loc, row, fuse)`.
pub fn event_charge_placed_piece() -> (bool, u16, u16, u8, u8, u8, u16) {
    (false, 733, 91, 5, sim_core::build::LOC_RISER, 173, 307)
}

pub fn event_charge_placed_deploy() -> (bool, u16, u16, u8, u8, u8, u16) {
    (true, 64, 900, 3, sim_core::build::LOC_PLANE, 9, 44)
}

/// An oven announcement: the same body address a deployable sync uses, lit
/// by a hand — and the same address gone out with **no** hand, which is
/// what a fire that ran dry looks like on the wire. Two fixtures because
/// the actor is the field that separates them and a single one would pin
/// only half of it.
///
/// `(cx, cz, level, lit, by)`.
pub fn event_oven_lit() -> (u16, u16, u8, bool, u32) {
    (64, 900, 3, true, 0x0000_1F07)
}

pub fn event_oven_out() -> (u16, u16, u8, bool, u32) {
    (64, 900, 3, false, 0)
}

/// An arrow leaving a bow (wire v33): shooter, yaw, pitch, speed, drop.
///
/// **Every field is a distinct value and none is a round number**, which
/// is the positional-payload rule (`reference/FINDINGS.md` §1) applied to
/// a message with two pairs of same-typed neighbours. `yaw`/`pitch` and
/// `speed`/`drop` are each two integers side by side, so a fixture that
/// used 0 for the pitch or shared a value between the pair would encode
/// identically under a transposition and the golden would bless the swap.
/// The yaw also deliberately exceeds a byte, so a decoder that read it at
/// the pitch's width would truncate visibly rather than plausibly.
pub fn event_shot() -> (u32, u16, u8, u16, u16) {
    (0x0000_2B17, 41_234, 203, 1_333, 22)
}

/// Where an arrow stopped (wire v44): x, y, z in the entity lane's quanta
/// and a `ranged::SURF_*`.
///
/// **Three same-typed neighbours, so the positional rule bites hardest
/// here** (`reference/FINDINGS.md` §1, and it is the trap `event_roles.rs`
/// exists for on the sim side). `qx` and `qz` are the pair a transposition
/// would silently swap, so they differ in both halves and neither is a
/// round number; `qy` is **negative**, which is this fixture's real job —
/// it is the only signed field on the event lane, it rides a bias, and a
/// decoder that forgot either would produce a plausible positive altitude
/// rather than a visibly wrong one. −312 is 3.12 m under datum: a shot
/// into a riverbed, well inside the window and nowhere near its edge, so
/// the fixture proves the bias rather than the clamp.
///
/// `surf` is `SURF_WORLD` — the middle of the three, so a decoder that
/// read the field one bit narrow or wide lands on a different live kind
/// instead of on a value the refusal would have caught for free.
pub fn event_impact() -> (i32, i32, i32, u8) {
    (0x0000_A179, -312, 0x0000_58A3, 1)
}

/// The swinger of the broadcast swing fact (wire v47).
///
/// Distinguishable in **both** 16-bit halves and in all four bytes, and
/// neither zero nor a small index: a field read one bit narrow or wide, or
/// a decoder that swapped the halves, cannot coincide with the right
/// answer. The same reasoning `event_shot`'s shooter is picked under —
/// this lane's failures are positional, not arithmetic.
pub fn event_swing() -> u32 {
    0x5A3C_91E7
}

/// Opening a **world container** (wire v37) — the fourth kind, and the
/// first whose open reaches the sim rather than only the subscription.
///
/// The handle is a real `gather::cell_key(147, 92)` — `cx << 16 | cz` on
/// a 256×256 grid — and both halves are chosen to be distinguishable from
/// each other and from every other handle in this file: a `cell_key`
/// whose two 8-bit cells were transposed, or read at the wrong width,
/// moves bytes here. It is deliberately unlike `action_container`'s
/// `box_key`, which packs three fields into the same 32 bits; two
/// different packings sharing a constant is how a decoder that used the
/// wrong one would still pass.
pub fn action_container_world() -> (u8, u32) {
    (3, 0x0093_005C)
}

/// Taking something **out of** a world container (wire v37).
///
/// Withdrawal rather than deposit, so the direction that actually matters
/// for a crate is the one pinned — and the reverse of `action_move_box`'s
/// deposit, so a copy-paste between the two shows as drifted bytes. The
/// handle is a third distinct `cell_key`, `(31, 208)`, for the reason
/// `action_move_box` gives about sharing a constant.
///
/// The source slot is 23: inside `INV_SLOTS` and past `BOX_SLOTS`, which
/// is the one address a world container accepts and a box does not, so a
/// decoder that resolved kind 3's width against the box's table would
/// refuse this fixture rather than quietly narrow the container.
pub fn action_move_world() -> (u32, u8, u8, u8, u8, u16) {
    (0x001F_00D0, 3, 23, 0, 4, 9)
}

/// A world container's contents arriving as an opening `reset`
/// (wire v37) — the shape a crate always sends first, since the roll is
/// what the open produced and the client has nothing to diff against.
///
/// `reset` is true here where `event_cont_sync`'s is false, so the fourth
/// kind is pinned in the state the third one is not. The slot indices
/// again reach past `BOX_SLOTS` and up to 29 — the last addressable slot
/// of the widest container — because the ceiling is exactly what a
/// per-kind width check gets wrong.
pub fn event_cont_sync_world() -> (u8, u32, bool, [InvSlot; 3]) {
    (
        3,
        0x0093_005C,
        true,
        [
            InvSlot {
                slot: 0,
                stack: ItemStack {
                    item: 19,
                    count: 64,
                    cond: 0,
                },
            },
            InvSlot {
                slot: 14,
                stack: ItemStack {
                    item: 7,
                    count: 2,
                    cond: 12_345,
                },
            },
            InvSlot {
                slot: 29,
                stack: ItemStack {
                    item: 44,
                    count: 11,
                    cond: 40_000,
                },
            },
        ],
    )
}

// ---------------------------------------------------------------------
// Research (wire v32), pinned at v37 — five versions late.
//
// `ACT_RESEARCH`, `SUB_RESEARCH`, `SUB_RESEARCH_REFUSED` and `SUB_KNOWN`
// all landed in v32 and crossed v33–v37 with **no fixture**, so the only
// thing asserting their bytes was the absence of a complaint. That is the
// same shape `action_move_box`'s doc records for the third container kind
// and the same shape twice more in the research lane itself (a bake with
// no caller, then three encoders with no `decode_event` arm) — a verb
// proven correct in the sim and unchecked on the wire. Pinning them does
// not move a byte, so no `PROTO_VER` bump is owed: these are the bytes
// v32 through v37 already sent. `every_encoder_has_a_golden` is the gate
// that makes the next one impossible to forget.

/// Learning at a table: the inventory slot holding the sample.
///
/// `23` rather than a small index — it is `0b10111`, so it fills all five
/// of `ACTION_SLOT_BITS` and a field narrowed to four would truncate it to
/// `7` instead of staying plausible. Under `INV_SLOTS` (30) and clear of
/// both ends.
pub fn action_research() -> u8 {
    23
}

// `event_research`, `event_research_refused` and `event_known` were
// declared a second time here by the local branch that pinned this lane on
// 2026-08-15. The integrated branch declares all three above (with the same
// intent and its own reasoning), so the duplicates are gone and the
// survivors are the ones `FIXTURES` names. `action_research` above has no
// counterpart there and stays: it is the fifth encoder of this lane, and
// `every_encoder_has_a_golden` is why it may not quietly lose its bytes.

/// Opening the **wear** container (wire v51, armor v1).
///
/// Kind 4 with handle **zero**, and the zero is the fixture's whole
/// point: `CONT_WEAR` is an own container (`inventory::is_own`), so
/// unlike the box and the crate above it there is no address to name —
/// the body is wherever the sender is. Every other container fixture in
/// this file carries a deliberately distinctive handle so a transposition
/// moves bytes; this one carries the absence of a handle, which is the
/// thing a decoder that treated kind 4 as ground would get wrong first.
///
/// Note what this pins about the *encoder's* one handle rule: it refuses
/// a nonzero handle on `CONT_SELF` only, so kind 4 is legal here purely
/// because it is a kind and not because anything special-cases it.
pub fn action_container_wear() -> (u8, u32) {
    (4, 0)
}

/// **Putting armor on** (wire v51) — the verb equipment actually is.
///
/// `CONT_SELF` slot 19 → `CONT_WEAR` slot 1, one piece. There is no
/// `ACT_EQUIP` to pin because there is no `ACT_EQUIP`: these are the
/// bytes that make a body wear something, and they are `ACT_MOVE`'s
/// (`reference/ARMOR.md` §9.2).
///
/// The count is **1 and honestly 1** — a wear slot holds one piece and
/// `stack_max` says so — rather than a number picked to differ from
/// everything else the way `action_move`'s 42 is. So the transposition
/// defence here rests on the other three fields instead, and they carry
/// it: the kinds differ (0 → 4, the two ends of the widened field), the
/// slots differ and are far apart (19 → 1), and swapping either
/// `(kind, slot)` pair for the other yields `(4, 1, 0, 19)`, which is
/// different bytes. The handle is zero for the reason above.
///
/// Slot 19 is also past `BOX_SLOTS`, so a decoder resolving the source
/// width against the wrong container's table refuses this fixture.
pub fn action_move_wear() -> (u32, u8, u8, u8, u8, u16) {
    (0, 0, 19, 4, 1, 1)
}

/// What a body is wearing, arriving as an opening `reset` (wire v51).
///
/// Two slots, which is all `WEAR_SLOTS` has, and they are **both filled
/// and both non-zero items** so the head/body pair cannot be read in
/// either order without moving bytes: slot 0 carries item 4 and slot 1
/// carries item 5, which are `combat.rs`'s two probe armor rows, in the
/// one arrangement `worn_pct` scores as a full set.
///
/// `reset` is true, matching `event_cont_sync_world` and unlike
/// `event_cont_sync`, because a wear container has no incremental state
/// to diff against on open either.
pub fn event_cont_sync_wear() -> (u8, u32, bool, [InvSlot; 2]) {
    (
        4,
        0,
        true,
        [
            InvSlot {
                slot: 0,
                stack: ItemStack {
                    item: 4,
                    count: 1,
                    cond: 9_100,
                },
            },
            InvSlot {
                slot: 1,
                stack: ItemStack {
                    item: 5,
                    count: 1,
                    cond: 10_000,
                },
            },
        ],
    )
}

/// A move refused because **that is not where that goes** (wire v51):
/// `REFUSE_M_WEAR`, carrying the address that was asked for.
///
/// Two firsts in five bytes, which is why it is pinned separately from
/// `event_move_refused`. The reason is **9** — the first value to need
/// `REFUSE_M_BITS`' fourth bit for a reason rather than for headroom, so
/// a field narrowed back to three bits truncates it into
/// `REFUSE_M_SLOT`'s neighbourhood and says "resync" where the sim said
/// "wrong slot". And the destination kind is 4 *inside an address*: the
/// refusal packs two `(kind, slot)` pairs, so this is the message where a
/// container kind travels without a `cont` handle beside it to make the
/// width obvious.
///
/// The address is `action_move_wear`'s with the destination slot changed
/// from 1 to 0 — the body piece offered to the head slot, which is
/// exactly what `combat::wearable_in` refuses — so the two fixtures read
/// as the same drag succeeding and failing.
pub fn event_move_refused_wear() -> (u8, u8, u8, u8, u8) {
    (9, 0, 19, 4, 0)
}
