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

use crate::{
    EntityState, Hello, InputDatagram, InvSlot, ItemCatalog, Nudge, Refuse, SnapshotHeader,
    Welcome, DEPLOY_SYNC_BATCH, PIECE_SYNC_BATCH, SLOT_SYNC_BATCH,
};
use sim_core::build::{BuildContent, PieceDef, PieceRec};
use sim_core::craft::{
    CraftContent, CraftJob, RecipeDef, STATION_FURNACE, STATION_NONE, STATION_WORKBENCH1,
};
use sim_core::deploy::{DeployContent, DeployRec};
use sim_core::gather::ItemStack;
use sim_core::input::InputFrame;
use sim_core::limits::{
    INV_SLOTS, MAX_INPUT_FRAMES, MAX_PIECE_COSTS, MAX_RECIPE_INPUTS, MAX_SNAPSHOT_ENTITIES,
};
use sim_core::rng::Pcg32;

/// Fixture file names, keyed by wire version (`PROTO_VER` 7 ⇒ `v7_*`).
pub const FIXTURES: [&str; 37] = [
    "v7_input_acks_only.bin",
    "v7_input_full.bin",
    "v7_snapshot_keyframe.bin",
    "v7_snapshot_delta.bin",
    "v7_snapshot_cap.bin",
    "v7_hello.bin",
    "v7_welcome.bin",
    "v7_refuse_full.bin",
    "v7_event_gather.bin",
    "v7_event_inv.bin",
    "v7_event_slot_harvested.bin",
    "v7_event_slot_respawned.bin",
    "v7_event_slot_sync.bin",
    "v7_event_catalog.bin",
    "v7_event_weak_mark.bin",
    "v7_event_craft_q.bin",
    "v7_event_craft_done.bin",
    "v7_event_craft_refused.bin",
    "v7_event_recipes.bin",
    "v7_action_craft.bin",
    "v7_action_cancel.bin",
    "v7_action_place.bin",
    "v7_event_piece_placed.bin",
    "v7_event_piece_sync.bin",
    "v7_event_build_refused.bin",
    "v7_event_piece_defs.bin",
    "v7_action_deploy.bin",
    "v7_action_feed.bin",
    "v7_event_deploy_placed.bin",
    "v7_event_deploy_sync.bin",
    "v7_event_deploy_refused.bin",
    "v7_event_deploy_defs.bin",
    "v7_event_piece_removed.bin",
    "v7_event_deploy_removed.bin",
    "v7_event_stock.bin",
    "v7_action_use.bin",
    "v7_event_door.bin",
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

/// The bidi-lane handshake trio (DESIGN.md §5.9), fixed values so the
/// stream lane is golden-pinned like the datagrams.
pub fn hello() -> Hello {
    Hello {
        proto_ver: crate::PROTO_VER,
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
/// length — the fixture encodes the batch at `first = 0`.
pub fn event_catalog() -> ItemCatalog {
    let mut cat = ItemCatalog::EMPTY;
    cat.count = 11;
    let names: [&[u8]; 11] = [
        b"Wood",
        b"Stone",
        b"Metal Ore",
        b"Sulfur Ore",
        b"Cloth",
        b"Animal Fat",
        b"Charcoal",
        b"Fixture Name Of Width 24",
        b"Sulfur",
        b"Gunpowder",
        b"Low Grade Fuel",
    ];
    for (i, n) in names.iter().enumerate() {
        cat.set(i, n).expect("golden names are in-cap by design");
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
        (20, 10, 5 * 30, STATION_WORKBENCH1, &[(6, 20), (8, 10)]),
        (
            31,
            1,
            30 * 30,
            STATION_WORKBENCH1,
            &[(20, 240), (12, 2), (13, 1), (5, 4)],
        ),
        (7, 1, 2 * 30, STATION_FURNACE, &[(2, 1)]),
        (63, 255, 65_535 * 30, STATION_NONE, &[(62, 65_535)]),
    ];
    for (i, &(output, out_count, ticks, station, inputs)) in rows.iter().enumerate() {
        let mut def = RecipeDef {
            output,
            out_count,
            ticks,
            station,
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

/// A cancel of queue job 2.
pub fn action_cancel() -> u16 {
    2
}

/// A place request: (row, cx, cz, level, loc) — a stone wall on a cell's
/// north edge, one storey up.
pub fn action_place() -> (u16, u16, u16, u8, u8) {
    (13, 341, 682, 1, sim_core::build::LOC_EDGE_N)
}

/// The piece record behind the placed broadcast.
pub fn event_piece_placed() -> PieceRec {
    PieceRec {
        cx: 341,
        cz: 682,
        level: 1,
        loc: sim_core::build::LOC_EDGE_N,
        row: 13,
        ..PieceRec::default()
    }
}

/// A full piece-sync batch with the reset bit set — the join-sync first
/// message at its cap.
pub fn event_piece_sync() -> (bool, [PieceRec; PIECE_SYNC_BATCH]) {
    let mut rng = Pcg32::new(0x0047_4154_4553, 18);
    let recs = core::array::from_fn(|_| PieceRec {
        cx: rng.next_bounded(1024) as u16,
        cz: rng.next_bounded(1024) as u16,
        level: rng.next_bounded(8) as u8,
        loc: rng.next_bounded(4) as u8,
        row: rng.next_bounded(32) as u8,
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
        (sim_core::build::SHAPE_FLOOR, 0, 750, &[(0, 350), (4, 10)]),
        (sim_core::build::SHAPE_STAIRS, 1, 1750, &[(1, 200)]),
        (sim_core::build::SHAPE_ROOF, 2, 65_535, &[(65_535, 65_535)]),
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
/// doorway on a cell's west edge.
pub fn action_deploy() -> (u16, u16, u16, u8, u8) {
    (9, 341, 682, 0, sim_core::build::LOC_EDGE_W)
}

/// A feed of the hearth at (cx, cz, level).
pub fn action_feed() -> (u16, u16, u8) {
    (341, 682, 0)
}

/// The deployable record behind the placed broadcast (owner/hp/uh are
/// sim-side and never cross — the fixture keeps their defaults). The
/// open bit is set so the wire's newest bit is pinned at 1 somewhere.
pub fn event_deploy_placed() -> DeployRec {
    DeployRec {
        cx: 341,
        cz: 682,
        level: 1,
        loc: sim_core::build::LOC_PLANE,
        row: 9,
        open: true,
        ..DeployRec::default()
    }
}

/// A full deploy-sync batch with the reset bit set.
pub fn event_deploy_sync() -> (bool, [DeployRec; DEPLOY_SYNC_BATCH]) {
    let mut rng = Pcg32::new(0x0047_4154_4553, 19);
    let recs = core::array::from_fn(|_| DeployRec {
        cx: rng.next_bounded(1024) as u16,
        cz: rng.next_bounded(1024) as u16,
        level: rng.next_bounded(8) as u8,
        loc: rng.next_bounded(4) as u8,
        row: rng.next_bounded(16) as u8,
        open: rng.next_bounded(2) == 0,
        ..DeployRec::default()
    });
    (true, recs)
}

/// A use request: the address of a door on a cell's west edge.
pub fn action_use() -> (u16, u16, u8, u8) {
    (341, 682, 0, sim_core::build::LOC_EDGE_W)
}

/// A door announcement: the same address, now open.
pub fn event_door() -> (u16, u16, u8, u8, bool) {
    (341, 682, 0, sim_core::build::LOC_EDGE_W, true)
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
    (341, 682, 0, sim_core::build::LOC_EDGE_N)
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
