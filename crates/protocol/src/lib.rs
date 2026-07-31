//! Packet schemas, bit-level codec, quantization tables, golden tests.
//! Shared native/wasm. Zero game logic (DESIGN.md §4) — the sim's types
//! (`InputFrame`, the quantized body fields) come from `sim-core`; this
//! crate only says how they cross the wire.
//!
//! Two datagram schemas, v0 (DESIGN.md §5.4/§5.5, NETCODE.md §3):
//!
//! **Input, C→S** — `kind:3 · snapshot_ack:16 · ack_bits:32 ·
//! first_client_tick:32 · frame_count:4`, then if any frames:
//! `first_seq:16` and per frame `buttons:8 · yaw:16 · pitch:8 · move_x:8 ·
//! move_z:8`. Frames are the client's unacked tail, oldest first,
//! seq-consecutive by construction (seq rides the wire once).
//!
//! **Snapshot, S→C** — `kind:3 · tick:32 · baseline_age:8 ·
//! last_executed_seq:16 · nudge:2 · removed_count:7 · entity_count:7`,
//! then removed ids (`u32` each), then entity records. `baseline_age == 0`
//! is the canonical zero-state (NETCODE.md §3, the Quake-3 move): every
//! record absolute, no baseline needed. Otherwise records may delta
//! against the client's state at `tick − baseline_age`.
//!
//! Encode and decode both live on hot paths, so both are allocation-free
//! and total: every failure is a `WireError`, never a panic — decode eats
//! arbitrary bytes (client-driven on the server side), and the golden
//! suite flips bits to prove it.

pub mod bits;
pub mod event;
pub mod goldens;

pub use bits::WireError;
use bits::{BitReader, BitWriter};
pub use event::{
    decode_event, encode_event_catalog, encode_event_gather, encode_event_inv,
    encode_event_slot_change, encode_event_slot_sync, EventMsg, InvSlot, ItemCatalog,
    CATALOG_BATCH, MAX_EVENT_MSG_BYTES, MAX_ITEM_NAME_BYTES, SLOT_SYNC_BATCH,
};
use sim_core::input::InputFrame;
use sim_core::limits::{MAX_INPUT_FRAMES, MAX_SNAPSHOT_ENTITIES};

/// Wire protocol version. Bumps only with a packet-layout change and
/// regenerated goldens in the same commit (CLAUDE.md wall 6). v1 added
/// the reliable event lane (`KIND_EVENT`, `event.rs`) — a v0 client would
/// not understand the stream messages a v1 server sends unprompted, so
/// the hello gate refuses the pair; fixtures are keyed `v1_*`.
pub const PROTO_VER: u16 = 1;

/// Datagram kind field width — room for the class-S lanes to grow into.
pub const KIND_BITS: u32 = 3;
pub const KIND_INPUT: u32 = 0;
pub const KIND_SNAPSHOT: u32 = 1;
/// Stream-lane message kinds (the bidi handshake, DESIGN.md §5.9). Same
/// 3-bit kind space; stream messages ride length-prefixed (u16 LE) frames
/// on the reliable lane, never datagrams.
pub const KIND_HELLO: u32 = 2;
pub const KIND_WELCOME: u32 = 3;
pub const KIND_REFUSE: u32 = 4;
/// S→C reliable event-lane messages (`event.rs`): subtyped, so the whole
/// lane spends one kind.
pub const KIND_EVENT: u32 = 5;

/// Longest stream-lane message payload the handshake accepts. Overflow
/// policy: refuse (`Malformed`) — a hello has no business being big.
pub const MAX_STREAM_MSG_BYTES: usize = 64;

const FRAME_COUNT_BITS: u32 = 4;
const COUNT_BITS: u32 = 7;

// Position on the wire, absolute records (DESIGN.md §5.5 quanta: 3 cm x/z,
// 1 cm y; widths + biases registered in DECISIONS.md §open, pinned by
// `test_protocol_golden`):
// x/z: 17 bits of 3 cm quanta = 0..3932 m, covers ISLAND_SIZE 2048 m.
// y: 14 bits of 1 cm quanta biased −20.48 m = −20.48..+143.35 m, covers
//    sea floor −12 m through ridge ~60 m with fall headroom.
// vy: 14 bits of 1 cm/s quanta biased −81.92 m/s = ±81.9 m/s, covers
//     TERMINAL_VELOCITY 50 (NETCODE §3's ±16 figure predates the spoken
//     terminal; the at-rest bit elides it entirely when still).
pub const POS_XZ_BITS: u32 = 17;
pub const POS_Y_BITS: u32 = 14;
pub const POS_Y_BIAS: i32 = 2048;
pub const VEL_BITS: u32 = 14;
pub const VEL_BIAS: i32 = 8192;

// Delta records: per-axis position deltas vs baseline, ±3.81 m x/z and
// ±5.11 m y — a sprint covers 0.37 m and terminal fall 3.33 m between
// 15 Hz snapshots, so real motion always fits; anything bigger falls back
// to an absolute record.
const DPOS_XZ_BITS: u32 = 8;
const DPOS_XZ_BIAS: i32 = 128;
const DPOS_Y_BITS: u32 = 10;
const DPOS_Y_BIAS: i32 = 512;

/// Peek a datagram's kind without decoding it — the dispatch read.
pub fn peek_kind(buf: &[u8]) -> Result<u32, WireError> {
    BitReader::new(buf).read(KIND_BITS)
}

// ---------------------------------------------------------------------------
// Stream-lane handshake messages (bidi, DESIGN.md §5.9)
// ---------------------------------------------------------------------------

/// C→S on the bidi stream: `hello{proto_ver}`. The version gate happens
/// before anything else exists for this client.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hello {
    pub proto_ver: u16,
}

/// S→C: the join bundle v0 — player id, world seed, current server tick.
/// Grows (spawn ring, catalog hash) with the slices that add those.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Welcome {
    pub player_id: u32,
    pub seed: u64,
    pub tick: u32,
}

/// S→C: refusal with a posted reason — a shard at cap refuses at hello,
/// never hangs (DESIGN.md §5.9).
pub const REFUSE_VERSION: u8 = 0;
pub const REFUSE_FULL: u8 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Refuse {
    pub code: u8,
}

pub fn encode_hello(msg: &Hello, buf: &mut [u8]) -> Result<usize, WireError> {
    let mut w = BitWriter::new(buf);
    w.write(KIND_HELLO, KIND_BITS)?;
    w.write(msg.proto_ver as u32, 16)?;
    Ok(w.finish())
}

pub fn decode_hello(buf: &[u8]) -> Result<Hello, WireError> {
    let mut r = BitReader::new(buf);
    if r.read(KIND_BITS)? != KIND_HELLO {
        return Err(WireError::Malformed);
    }
    let proto_ver = r.read(16)? as u16;
    expect_zero_padding(&mut r)?;
    Ok(Hello { proto_ver })
}

pub fn encode_welcome(msg: &Welcome, buf: &mut [u8]) -> Result<usize, WireError> {
    let mut w = BitWriter::new(buf);
    w.write(KIND_WELCOME, KIND_BITS)?;
    w.write(msg.player_id, 32)?;
    w.write(msg.seed as u32, 32)?;
    w.write((msg.seed >> 32) as u32, 32)?;
    w.write(msg.tick, 32)?;
    Ok(w.finish())
}

pub fn decode_welcome(buf: &[u8]) -> Result<Welcome, WireError> {
    let mut r = BitReader::new(buf);
    if r.read(KIND_BITS)? != KIND_WELCOME {
        return Err(WireError::Malformed);
    }
    let player_id = r.read(32)?;
    let lo = r.read(32)? as u64;
    let hi = r.read(32)? as u64;
    let tick = r.read(32)?;
    expect_zero_padding(&mut r)?;
    Ok(Welcome {
        player_id,
        seed: lo | (hi << 32),
        tick,
    })
}

pub fn encode_refuse(msg: &Refuse, buf: &mut [u8]) -> Result<usize, WireError> {
    let mut w = BitWriter::new(buf);
    w.write(KIND_REFUSE, KIND_BITS)?;
    w.write(msg.code as u32, 8)?;
    Ok(w.finish())
}

pub fn decode_refuse(buf: &[u8]) -> Result<Refuse, WireError> {
    let mut r = BitReader::new(buf);
    if r.read(KIND_BITS)? != KIND_REFUSE {
        return Err(WireError::Malformed);
    }
    let code = r.read(8)? as u8;
    expect_zero_padding(&mut r)?;
    Ok(Refuse { code })
}

// ---------------------------------------------------------------------------
// Input datagram (C→S)
// ---------------------------------------------------------------------------

/// One input datagram: the Gaffer ack header plus the client's unacked
/// input tail (NETCODE.md §3). Frames are seq-consecutive, oldest first;
/// `push` enforces it so the wire can carry one seq for all of them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InputDatagram {
    /// Newest snapshot tick **applied** (low 16 bits, wrap-compare). An
    /// ack means applied, not merely received: a stale snapshot the client
    /// discarded is never acked, so the server's mirror of what the client
    /// knows (its delta baseline) folds exactly the acked set.
    pub snapshot_ack: u16,
    /// Bit n set ⇒ snapshot tick `snapshot_ack − n − 1` also applied.
    pub ack_bits: u32,
    /// Client tick of `frames[0]`; with no frames, the client's current
    /// tick (the datagram still feeds the server's clock estimate).
    pub first_client_tick: u32,
    frames: [InputFrame; MAX_INPUT_FRAMES],
    frame_count: u8,
}

impl InputDatagram {
    pub fn new(snapshot_ack: u16, ack_bits: u32, first_client_tick: u32) -> Self {
        Self {
            snapshot_ack,
            ack_bits,
            first_client_tick,
            frames: [InputFrame::default(); MAX_INPUT_FRAMES],
            frame_count: 0,
        }
    }

    /// Append the next unacked frame. Refuses past `MAX_INPUT_FRAMES`
    /// (policy: the *client* drops its oldest before pushing — the cap is
    /// on the datagram, not the intent) and refuses a seq gap (`Malformed`)
    /// because the wire reconstructs seqs from the first one.
    pub fn push(&mut self, frame: InputFrame) -> Result<(), WireError> {
        let n = self.frame_count as usize;
        if n >= MAX_INPUT_FRAMES {
            return Err(WireError::Cap);
        }
        if n > 0 && frame.seq != self.frames[n - 1].seq.wrapping_add(1) {
            return Err(WireError::Malformed);
        }
        self.frames[n] = frame;
        self.frame_count += 1;
        Ok(())
    }

    pub fn frames(&self) -> &[InputFrame] {
        &self.frames[..self.frame_count as usize]
    }

    /// Client tick of frame `i` (ticks advance in lockstep with seqs).
    pub fn client_tick_of(&self, i: usize) -> u32 {
        self.first_client_tick.wrapping_add(i as u32)
    }
}

pub fn encode_input(dg: &InputDatagram, buf: &mut [u8]) -> Result<usize, WireError> {
    let mut w = BitWriter::new(buf);
    w.write(KIND_INPUT, KIND_BITS)?;
    w.write(dg.snapshot_ack as u32, 16)?;
    w.write(dg.ack_bits, 32)?;
    w.write(dg.first_client_tick, 32)?;
    let frames = dg.frames();
    w.write(frames.len() as u32, FRAME_COUNT_BITS)?;
    if let Some(first) = frames.first() {
        w.write(first.seq as u32, 16)?;
        for f in frames {
            w.write(f.buttons as u32, 8)?;
            w.write(f.yaw as u32, 16)?;
            w.write(f.pitch as u32, 8)?;
            w.write(f.move_x as u8 as u32, 8)?;
            w.write(f.move_z as u8 as u32, 8)?;
        }
    }
    Ok(w.finish())
}

pub fn decode_input(buf: &[u8]) -> Result<InputDatagram, WireError> {
    let mut r = BitReader::new(buf);
    if r.read(KIND_BITS)? != KIND_INPUT {
        return Err(WireError::Malformed);
    }
    let snapshot_ack = r.read(16)? as u16;
    let ack_bits = r.read(32)?;
    let first_client_tick = r.read(32)?;
    let count = r.read(FRAME_COUNT_BITS)? as usize;
    if count > MAX_INPUT_FRAMES {
        return Err(WireError::Malformed);
    }
    let mut dg = InputDatagram::new(snapshot_ack, ack_bits, first_client_tick);
    if count > 0 {
        let first_seq = r.read(16)? as u16;
        for i in 0..count {
            let frame = InputFrame {
                seq: first_seq.wrapping_add(i as u16),
                buttons: r.read(8)? as u8,
                yaw: r.read(16)? as u16,
                pitch: r.read(8)? as u8,
                move_x: r.read(8)? as u8 as i8,
                move_z: r.read(8)? as u8 as i8,
            };
            // Cannot fail: count ≤ cap, seqs consecutive by construction.
            dg.push(frame)?;
        }
    }
    expect_zero_padding(&mut r)?;
    Ok(dg)
}

// ---------------------------------------------------------------------------
// Snapshot datagram (S→C)
// ---------------------------------------------------------------------------

/// The time-dilation nudge, 2 bits in every snapshot header (NETCODE.md §4,
/// the Overwatch scheme).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Nudge {
    #[default]
    Ok = 0,
    Faster = 1,
    Slower = 2,
    HardResync = 3,
}

impl Nudge {
    fn from_bits(v: u32) -> Self {
        match v {
            1 => Nudge::Faster,
            2 => Nudge::Slower,
            3 => Nudge::HardResync,
            _ => Nudge::Ok,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapshotHeader {
    /// Server tick, low 32 bits (4.5 years at 30 Hz before wrap).
    pub tick: u32,
    /// 0 ⇒ zero-state baseline (all records absolute); else the baseline
    /// is the client's acked state at `tick − baseline_age`. The sent-state
    /// ring is 32 snapshots ≈ 64 ticks (NETCODE.md §3), well inside u8.
    pub baseline_age: u8,
    /// Newest input seq the sim executed for this client — drives
    /// prediction reconciliation (DESIGN.md §5.6).
    pub last_executed_seq: u16,
    pub nudge: Nudge,
}

/// One class-D entity as the wire sees it: the quantized body (the sim
/// runs on exactly these values — NETCODE.md §3) plus view yaw/pitch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EntityState {
    pub id: u32,
    /// Position in world-absolute quanta (3 cm x/z, 1 cm y — `movement.rs`).
    pub qx: i32,
    pub qy: i32,
    pub qz: i32,
    /// Vertical velocity in 1 cm/s quanta; 0 rides the at-rest bit.
    pub qvy: i32,
    pub grounded: bool,
    pub yaw: u16,
    pub pitch: u8,
}

/// Incremental snapshot encoder — the priority-fill loop's tool
/// (DESIGN.md §5.5): removals first, then entities best-first; an
/// `Overflow` cleanly rolls back the record that didn't fit and the
/// caller sheds it (it stays accumulated and wins soon). The budget IS
/// the buffer: hand it `DATAGRAM_BUDGET_BYTES` and overflow is the gate.
pub struct SnapshotEncoder<'a, 'b> {
    w: BitWriter<'a>,
    baseline: &'b [EntityState],
    has_baseline: bool,
    removed_count_at: usize,
    entity_count_at: usize,
    removed: u32,
    entities: u32,
    entity_started: bool,
}

impl<'a, 'b> SnapshotEncoder<'a, 'b> {
    /// `baseline` is the client's acked entity set at `tick −
    /// baseline_age`. Zero-state (`baseline_age == 0`) has no entities by
    /// definition, so a non-empty baseline with it is refused — that
    /// mismatch is a server bug surfacing, not a case to paper over.
    pub fn begin(
        buf: &'a mut [u8],
        header: &SnapshotHeader,
        baseline: &'b [EntityState],
    ) -> Result<Self, WireError> {
        if header.baseline_age == 0 && !baseline.is_empty() {
            return Err(WireError::Malformed);
        }
        let mut w = BitWriter::new(buf);
        w.write(KIND_SNAPSHOT, KIND_BITS)?;
        w.write(header.tick, 32)?;
        w.write(header.baseline_age as u32, 8)?;
        w.write(header.last_executed_seq as u32, 16)?;
        w.write(header.nudge as u32, 2)?;
        let removed_count_at = w.bit_pos();
        w.write(0, COUNT_BITS)?;
        let entity_count_at = w.bit_pos();
        w.write(0, COUNT_BITS)?;
        Ok(Self {
            w,
            baseline,
            has_baseline: header.baseline_age != 0,
            removed_count_at,
            entity_count_at,
            removed: 0,
            entities: 0,
            entity_started: false,
        })
    }

    /// An entity that left the interest set. Must precede every
    /// `add_entity` (wire order); capped like the set itself.
    pub fn add_removed(&mut self, id: u32) -> Result<(), WireError> {
        if self.entity_started {
            return Err(WireError::Order);
        }
        if self.removed as usize >= MAX_SNAPSHOT_ENTITIES {
            return Err(WireError::Cap);
        }
        let mark = self.w.bit_pos();
        if let Err(e) = self.w.write(id, 32) {
            self.w.rewind_to(mark);
            return Err(e);
        }
        self.removed += 1;
        Ok(())
    }

    /// Add one entity, delta-encoded against the baseline when it's there
    /// and the motion fits the delta widths, absolute otherwise. On
    /// `Overflow` the record is rolled back whole; the encoder stays
    /// usable (a smaller record may still fit).
    pub fn add_entity(&mut self, e: &EntityState) -> Result<(), WireError> {
        self.entity_started = true;
        if self.entities as usize >= MAX_SNAPSHOT_ENTITIES {
            return Err(WireError::Cap);
        }
        check_ranges(e)?;
        let mark = self.w.bit_pos();
        if let Err(err) = self.encode_entity(e) {
            self.w.rewind_to(mark);
            return Err(err);
        }
        self.entities += 1;
        Ok(())
    }

    fn encode_entity(&mut self, e: &EntityState) -> Result<(), WireError> {
        self.w.write(e.id, 32)?;
        if self.has_baseline {
            if let Some(b) = self.baseline.iter().find(|b| b.id == e.id) {
                let dx = e.qx as i64 - b.qx as i64;
                let dy = e.qy as i64 - b.qy as i64;
                let dz = e.qz as i64 - b.qz as i64;
                let fits = |d: i64, bias: i32, bits: u32| -> bool {
                    d + bias as i64 >= 0 && d + (bias as i64) < (1i64 << bits)
                };
                if fits(dx, DPOS_XZ_BIAS, DPOS_XZ_BITS)
                    && fits(dy, DPOS_Y_BIAS, DPOS_Y_BITS)
                    && fits(dz, DPOS_XZ_BIAS, DPOS_XZ_BITS)
                {
                    return self.encode_delta(e, b, dx, dy, dz);
                }
            }
        }
        self.encode_absolute(e)
    }

    fn encode_delta(
        &mut self,
        e: &EntityState,
        b: &EntityState,
        dx: i64,
        dy: i64,
        dz: i64,
    ) -> Result<(), WireError> {
        self.w.write_bit(true)?; // is_delta
        let pos_changed = dx != 0 || dy != 0 || dz != 0;
        let vel_changed = e.qvy != b.qvy;
        let look_changed = e.yaw != b.yaw || e.pitch != b.pitch;
        self.w.write_bit(pos_changed)?;
        self.w.write_bit(vel_changed)?;
        self.w.write_bit(look_changed)?;
        self.w.write_bit(e.grounded)?;
        if pos_changed {
            self.w
                .write((dx + DPOS_XZ_BIAS as i64) as u32, DPOS_XZ_BITS)?;
            self.w
                .write((dy + DPOS_Y_BIAS as i64) as u32, DPOS_Y_BITS)?;
            self.w
                .write((dz + DPOS_XZ_BIAS as i64) as u32, DPOS_XZ_BITS)?;
        }
        if vel_changed {
            self.write_vel(e.qvy)?;
        }
        if look_changed {
            self.w.write(e.yaw as u32, 16)?;
            self.w.write(e.pitch as u32, 8)?;
        }
        Ok(())
    }

    fn encode_absolute(&mut self, e: &EntityState) -> Result<(), WireError> {
        self.w.write_bit(false)?; // is_delta
        self.w.write(e.qx as u32, POS_XZ_BITS)?;
        self.w.write((e.qy + POS_Y_BIAS) as u32, POS_Y_BITS)?;
        self.w.write(e.qz as u32, POS_XZ_BITS)?;
        self.w.write_bit(e.grounded)?;
        self.write_vel(e.qvy)?;
        self.w.write(e.yaw as u32, 16)?;
        self.w.write(e.pitch as u32, 8)?;
        Ok(())
    }

    fn write_vel(&mut self, qvy: i32) -> Result<(), WireError> {
        let at_rest = qvy == 0;
        self.w.write_bit(at_rest)?;
        if !at_rest {
            self.w.write((qvy + VEL_BIAS) as u32, VEL_BITS)?;
        }
        Ok(())
    }

    /// Patch the counts, return the encoded byte length.
    pub fn finish(mut self) -> Result<usize, WireError> {
        self.w
            .patch(self.removed_count_at, self.removed, COUNT_BITS)?;
        self.w
            .patch(self.entity_count_at, self.entities, COUNT_BITS)?;
        Ok(self.w.finish())
    }
}

/// Absolute-record range check, up front so a `Range` refusal never needs
/// a rollback and delta eligibility never hides an unencodable state.
fn check_ranges(e: &EntityState) -> Result<(), WireError> {
    let in_window = |v: i64, bias: i32, bits: u32| -> bool {
        v + bias as i64 >= 0 && v + (bias as i64) < (1i64 << bits)
    };
    if !in_window(e.qx as i64, 0, POS_XZ_BITS)
        || !in_window(e.qy as i64, POS_Y_BIAS, POS_Y_BITS)
        || !in_window(e.qz as i64, 0, POS_XZ_BITS)
        || !in_window(e.qvy as i64, VEL_BIAS, VEL_BITS)
    {
        return Err(WireError::Range);
    }
    Ok(())
}

/// Whole-snapshot convenience over the incremental encoder — the shape
/// tests and goldens use. `Overflow` here is an error, not a shed: callers
/// that need the fill loop use `SnapshotEncoder` directly.
pub fn encode_snapshot(
    header: &SnapshotHeader,
    removed: &[u32],
    entities: &[EntityState],
    baseline: &[EntityState],
    buf: &mut [u8],
) -> Result<usize, WireError> {
    let mut enc = SnapshotEncoder::begin(buf, header, baseline)?;
    for &id in removed {
        enc.add_removed(id)?;
    }
    for e in entities {
        enc.add_entity(e)?;
    }
    enc.finish()
}

/// A decoded snapshot: reconstructed absolute states (deltas already
/// applied onto the baseline) plus the removed-id list. Fixed storage;
/// unused tail slots stay `Default` so equality is well-defined.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub header: SnapshotHeader,
    removed: [u32; MAX_SNAPSHOT_ENTITIES],
    removed_count: u8,
    entities: [EntityState; MAX_SNAPSHOT_ENTITIES],
    entity_count: u8,
}

impl Snapshot {
    pub fn removed(&self) -> &[u32] {
        &self.removed[..self.removed_count as usize]
    }

    pub fn entities(&self) -> &[EntityState] {
        &self.entities[..self.entity_count as usize]
    }
}

/// Header-only read, so a receiver can pick the baseline snapshot
/// (`tick − baseline_age`) out of its applied ring before the full
/// `decode_snapshot` call. Reads exactly the header bits; the body goes
/// untouched.
pub fn peek_snapshot_header(buf: &[u8]) -> Result<SnapshotHeader, WireError> {
    let mut r = BitReader::new(buf);
    if r.read(KIND_BITS)? != KIND_SNAPSHOT {
        return Err(WireError::Malformed);
    }
    read_snapshot_header(&mut r)
}

fn read_snapshot_header(r: &mut BitReader) -> Result<SnapshotHeader, WireError> {
    Ok(SnapshotHeader {
        tick: r.read(32)?,
        baseline_age: r.read(8)? as u8,
        last_executed_seq: r.read(16)? as u16,
        nudge: Nudge::from_bits(r.read(2)?),
    })
}

/// Decode against the baseline the header names — the **sent content** of
/// the snapshot at `tick − baseline_age`, which the client holds because
/// it applied (and therefore acked) that snapshot. Entities absent from
/// that snapshot arrive absolute; nothing is ever folded across snapshots,
/// so both sides hold byte-identical baselines by construction. Total:
/// arbitrary bytes in, `Ok` or a `WireError` out, never a panic.
pub fn decode_snapshot(buf: &[u8], baseline: &[EntityState]) -> Result<Snapshot, WireError> {
    let mut r = BitReader::new(buf);
    if r.read(KIND_BITS)? != KIND_SNAPSHOT {
        return Err(WireError::Malformed);
    }
    let header = read_snapshot_header(&mut r)?;
    let removed_count = r.read(COUNT_BITS)? as usize;
    let entity_count = r.read(COUNT_BITS)? as usize;
    if removed_count > MAX_SNAPSHOT_ENTITIES || entity_count > MAX_SNAPSHOT_ENTITIES {
        return Err(WireError::Malformed);
    }
    let mut snap = Snapshot {
        header,
        removed: [0; MAX_SNAPSHOT_ENTITIES],
        removed_count: removed_count as u8,
        entities: [EntityState::default(); MAX_SNAPSHOT_ENTITIES],
        entity_count: entity_count as u8,
    };
    for slot in snap.removed.iter_mut().take(removed_count) {
        *slot = r.read(32)?;
    }
    for slot in snap.entities.iter_mut().take(entity_count) {
        *slot = decode_entity(&mut r, header.baseline_age, baseline)?;
    }
    expect_zero_padding(&mut r)?;
    Ok(snap)
}

fn decode_entity(
    r: &mut BitReader,
    baseline_age: u8,
    baseline: &[EntityState],
) -> Result<EntityState, WireError> {
    let id = r.read(32)?;
    let is_delta = r.read_bit()?;
    if !is_delta {
        let qx = r.read(POS_XZ_BITS)? as i32;
        let qy = r.read(POS_Y_BITS)? as i32 - POS_Y_BIAS;
        let qz = r.read(POS_XZ_BITS)? as i32;
        let grounded = r.read_bit()?;
        let qvy = read_vel(r)?;
        return Ok(EntityState {
            id,
            qx,
            qy,
            qz,
            qvy,
            grounded,
            yaw: r.read(16)? as u16,
            pitch: r.read(8)? as u8,
        });
    }
    if baseline_age == 0 {
        return Err(WireError::Malformed);
    }
    let b = baseline
        .iter()
        .find(|b| b.id == id)
        .ok_or(WireError::Malformed)?;
    let mut e = *b;
    let pos_changed = r.read_bit()?;
    let vel_changed = r.read_bit()?;
    let look_changed = r.read_bit()?;
    e.grounded = r.read_bit()?;
    if pos_changed {
        // wrapping: baseline values are the decoder's own prior state, but
        // totality on arbitrary bytes must hold regardless.
        e.qx =
            b.qx.wrapping_add(r.read(DPOS_XZ_BITS)? as i32 - DPOS_XZ_BIAS);
        e.qy = b.qy.wrapping_add(r.read(DPOS_Y_BITS)? as i32 - DPOS_Y_BIAS);
        e.qz =
            b.qz.wrapping_add(r.read(DPOS_XZ_BITS)? as i32 - DPOS_XZ_BIAS);
    }
    if vel_changed {
        e.qvy = read_vel(r)?;
    }
    if look_changed {
        e.yaw = r.read(16)? as u16;
        e.pitch = r.read(8)? as u8;
    }
    Ok(e)
}

fn read_vel(r: &mut BitReader) -> Result<i32, WireError> {
    if r.read_bit()? {
        Ok(0)
    } else {
        Ok(r.read(VEL_BITS)? as i32 - VEL_BIAS)
    }
}

/// Strict tail: only byte padding may remain, and it must be zero — a
/// packet with trailing garbage never passes as valid.
pub(crate) fn expect_zero_padding(r: &mut BitReader) -> Result<(), WireError> {
    let rem = r.remaining_bits();
    if rem >= 8 {
        return Err(WireError::Malformed);
    }
    if rem > 0 && r.read(rem as u32)? != 0 {
        return Err(WireError::Malformed);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_core::limits::DATAGRAM_BUDGET_BYTES;

    fn ent(id: u32) -> EntityState {
        EntityState {
            id,
            qx: 34_000,
            qy: 900,
            qz: 34_000,
            qvy: 0,
            grounded: true,
            yaw: 0x1234,
            pitch: 7,
        }
    }

    #[test]
    fn wire_widths_cover_the_sim() {
        use sim_core::movement::{POS_XZ_Q, POS_Y_Q, TERMINAL_VELOCITY, VEL_Q};
        use sim_core::terrain::{AMPLITUDE, ISLAND_SIZE};
        // x/z: the whole island fits the absolute width.
        assert!(((ISLAND_SIZE / POS_XZ_Q) as i64) < (1i64 << POS_XZ_BITS));
        // y: bias reaches below any sea floor; ceiling clears the relief.
        assert!((POS_Y_BIAS as f32) * POS_Y_Q >= 12.0);
        assert!(((1 << POS_Y_BITS) - POS_Y_BIAS) as f32 * POS_Y_Q > AMPLITUDE);
        // vy: terminal velocity fits with headroom.
        assert!((VEL_BIAS as f32) * VEL_Q > TERMINAL_VELOCITY);
    }

    #[test]
    fn zero_state_refuses_a_baseline() {
        let hdr = SnapshotHeader {
            tick: 1,
            baseline_age: 0,
            last_executed_seq: 0,
            nudge: Nudge::Ok,
        };
        let baseline = [ent(1)];
        let mut buf = [0u8; 64];
        assert!(matches!(
            SnapshotEncoder::begin(&mut buf, &hdr, &baseline),
            Err(WireError::Malformed)
        ));
    }

    #[test]
    fn removals_precede_entities() {
        let hdr = SnapshotHeader {
            tick: 1,
            baseline_age: 0,
            last_executed_seq: 0,
            nudge: Nudge::Ok,
        };
        let mut buf = [0u8; 128];
        let mut enc = SnapshotEncoder::begin(&mut buf, &hdr, &[]).unwrap();
        enc.add_entity(&ent(1)).unwrap();
        assert_eq!(enc.add_removed(9), Err(WireError::Order));
    }

    #[test]
    fn out_of_range_state_is_refused_not_clamped() {
        let hdr = SnapshotHeader {
            tick: 1,
            baseline_age: 0,
            last_executed_seq: 0,
            nudge: Nudge::Ok,
        };
        let mut buf = [0u8; 128];
        let mut enc = SnapshotEncoder::begin(&mut buf, &hdr, &[]).unwrap();
        let mut bad = ent(1);
        bad.qx = -1;
        assert_eq!(enc.add_entity(&bad), Err(WireError::Range));
        bad = ent(2);
        bad.qy = (1 << POS_Y_BITS) - POS_Y_BIAS;
        assert_eq!(enc.add_entity(&bad), Err(WireError::Range));
    }

    #[test]
    fn overflow_sheds_the_record_and_the_rest_still_decodes() {
        let hdr = SnapshotHeader {
            tick: 42,
            baseline_age: 0,
            last_executed_seq: 5,
            nudge: Nudge::Ok,
        };
        // Room for the header and one absolute record, not two.
        let mut buf = [0u8; 28];
        let mut enc = SnapshotEncoder::begin(&mut buf, &hdr, &[]).unwrap();
        enc.add_entity(&ent(1)).unwrap();
        assert_eq!(enc.add_entity(&ent(2)), Err(WireError::Overflow));
        let len = enc.finish().unwrap();
        let snap = decode_snapshot(&buf[..len], &[]).unwrap();
        assert_eq!(snap.entities(), &[ent(1)]);
        assert_eq!(snap.removed(), &[] as &[u32]);
    }

    #[test]
    fn input_push_enforces_the_cap_and_seq_run() {
        let mut dg = InputDatagram::new(7, 0, 100);
        let mut f = InputFrame {
            seq: 40,
            ..InputFrame::default()
        };
        dg.push(f).unwrap();
        f.seq = 42; // gap
        assert_eq!(dg.push(f), Err(WireError::Malformed));
        f.seq = 41;
        dg.push(f).unwrap();
        for i in 0..MAX_INPUT_FRAMES as u16 - 2 {
            f.seq = 42 + i;
            dg.push(f).unwrap();
        }
        f.seq = 40 + MAX_INPUT_FRAMES as u16;
        assert_eq!(dg.push(f), Err(WireError::Cap));
    }

    #[test]
    fn input_seq_wraps_across_u16() {
        let mut dg = InputDatagram::new(0, 0, 9);
        for i in 0..4u16 {
            let f = InputFrame {
                seq: 0xFFFE_u16.wrapping_add(i),
                ..InputFrame::default()
            };
            dg.push(f).unwrap();
        }
        let mut buf = [0u8; DATAGRAM_BUDGET_BYTES];
        let len = encode_input(&dg, &mut buf).unwrap();
        let back = decode_input(&buf[..len]).unwrap();
        assert_eq!(back, dg);
        assert_eq!(back.frames()[3].seq, 1);
    }

    #[test]
    fn trailing_garbage_is_malformed() {
        let dg = InputDatagram::new(1, 2, 3);
        let mut buf = [0u8; DATAGRAM_BUDGET_BYTES];
        let len = encode_input(&dg, &mut buf).unwrap();
        // One spare byte after a valid packet must fail the strict tail.
        assert_eq!(decode_input(&buf[..len + 1]), Err(WireError::Malformed));
    }
}
