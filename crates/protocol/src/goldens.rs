//! The canonical packets behind `test_protocol_golden` (DESIGN.md §12):
//! one construction, two consumers — `examples/gen_goldens.rs` writes the
//! fixture bytes, `tests/protocol_golden.rs` asserts today's encoder still
//! produces them byte-for-byte. Content is deterministic (sim-core Pcg32,
//! fixed seeds), so "regenerate" is reproducible on any box.
//!
//! Regenerating fixtures is only ever legitimate alongside a `PROTO_VER`
//! bump in the same commit (CLAUDE.md wall 6). A diff here without a bump
//! is the wire drifting by accident — the exact thing the gate exists to
//! catch.

use crate::{EntityState, InputDatagram, Nudge, SnapshotHeader};
use sim_core::input::InputFrame;
use sim_core::limits::{MAX_INPUT_FRAMES, MAX_SNAPSHOT_ENTITIES};
use sim_core::rng::Pcg32;

/// Fixture file names, keyed by wire version (`PROTO_VER` 0 ⇒ `v0_*`).
pub const FIXTURES: [&str; 5] = [
    "v0_input_acks_only.bin",
    "v0_input_full.bin",
    "v0_snapshot_keyframe.bin",
    "v0_snapshot_delta.bin",
    "v0_snapshot_cap.bin",
];

fn rng_entity(rng: &mut Pcg32, id: u32) -> EntityState {
    EntityState {
        id,
        // Island interior in quanta (3 cm): ~300 m .. ~1800 m.
        qx: 10_000 + rng.next_bounded(50_000) as i32,
        qy: -1_200 + rng.next_bounded(7_000) as i32,
        qz: 10_000 + rng.next_bounded(50_000) as i32,
        qvy: rng.next_bounded(10_000) as i32 - 5_000,
        grounded: rng.next_bounded(2) == 0,
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
pub fn input_full() -> InputDatagram {
    let mut rng = Pcg32::new(0x0047_4154_4553, 11);
    let mut dg = InputDatagram::new(0x0102, 0xFFFF_FFFF, 0xFFFF_FFFE);
    for i in 0..MAX_INPUT_FRAMES as u16 {
        let f = InputFrame {
            seq: 0xFFFC_u16.wrapping_add(i),
            buttons: rng.next_bounded(4) as u8,
            yaw: rng.next_bounded(0x1_0000) as u16,
            pitch: rng.next_bounded(0x100) as u8,
            move_x: rng.next_bounded(255) as i32 as i8,
            move_z: (rng.next_bounded(255) as i32 - 127) as i8,
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
    // id 1: unchanged — the 37-bit record.
    entities[0] = baseline[0];
    // id 2: one snapshot interval of sprint + a look turn.
    entities[1] = baseline[1];
    entities[1].qx += 12;
    entities[1].qy -= 3;
    entities[1].qz += 7;
    entities[1].yaw = entities[1].yaw.wrapping_add(0x0400);
    entities[1].pitch = entities[1].pitch.wrapping_add(3);
    // id 3: starts falling, nothing else.
    entities[2] = baseline[2];
    entities[2].qvy = -450;
    entities[2].grounded = false;
    // id 4: teleported beyond the delta window — absolute fallback.
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
