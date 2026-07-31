//! Raw C-ABI wasm bridge — no bindgen, the same pattern as the parity
//! probe (`ci/parity.mjs`): plain `extern "C"` exports plus fixed buffers
//! the JS side views zero-copy over wasm memory (DESIGN.md L8: no
//! per-frame objects, no per-packet allocation). One client per instance —
//! a browser tab is one player. Compiled only for wasm32; native callers
//! use `ClientCore` directly.
//!
//! Calling convention: JS writes incoming bytes at `client_in_ptr()`,
//! calls the matching `*_on_*`/`*_parse_*` with the length; outgoing bytes
//! appear at `client_out_ptr()`; render floats at `client_render_ptr()`
//! (layout documented on `render_fill`). u64 seeds cross as BigInt.

use crate::core::ClientCore;
use protocol::{
    decode_refuse, decode_welcome, encode_hello, peek_kind, Hello, KIND_REFUSE, KIND_WELCOME,
    PROTO_VER,
};
use sim_core::limits::{DATAGRAM_BUDGET_BYTES, MAX_SNAPSHOT_ENTITIES};
use sim_core::terrain::{self, ScatterTable};
use std::cell::RefCell;

/// Incoming scratch: comfortably above the server's 1100 B send clamp.
const IN_CAP: usize = 2048;
/// Handshake words: [kind, player_id, seed_lo, seed_hi, tick, refuse_code].
const HS_WORDS: usize = 6;
/// Render layout: a 13-float own/status block, a count, then 8 floats per
/// remote (ids ride the parallel u32 buffer — f32 can't hold late-shard
/// generations exactly).
const OWN_FLOATS: usize = 13;
const REMOTE_FLOATS: usize = 8;
const RENDER_FLOATS: usize = OWN_FLOATS + 1 + MAX_SNAPSHOT_ENTITIES * REMOTE_FLOATS;
/// Terrain fill grid bound: the far mesh at 8 m over 2,048 m is 257
/// samples plus a 2-sample normal apron.
const HEIGHTS_MAX_N: usize = 259;
/// Slot fill: one chunk of 8×8 scatter cells, 6 floats per resolved slot.
const SLOTS_MAX_CELLS: usize = 8;
const SLOT_FLOATS: usize = 6;

struct Bridge {
    core: Option<ClientCore>,
    in_buf: [u8; IN_CAP],
    out_buf: [u8; DATAGRAM_BUDGET_BYTES],
    hs_buf: [u32; HS_WORDS],
    render: Box<[f32; RENDER_FLOATS]>,
    remote_ids: [u32; MAX_SNAPSHOT_ENTITIES],
    heights: Vec<f32>,
    slots: [f32; SLOTS_MAX_CELLS * SLOTS_MAX_CELLS * SLOT_FLOATS],
}

impl Bridge {
    fn new() -> Self {
        Self {
            core: None,
            in_buf: [0; IN_CAP],
            out_buf: [0; DATAGRAM_BUDGET_BYTES],
            hs_buf: [0; HS_WORDS],
            render: Box::new([0.0; RENDER_FLOATS]),
            remote_ids: [0; MAX_SNAPSHOT_ENTITIES],
            heights: vec![0.0; HEIGHTS_MAX_N * HEIGHTS_MAX_N],
            slots: [0.0; SLOTS_MAX_CELLS * SLOTS_MAX_CELLS * SLOT_FLOATS],
        }
    }
}

thread_local! {
    static BRIDGE: RefCell<Bridge> = RefCell::new(Bridge::new());
}

fn with<R>(f: impl FnOnce(&mut Bridge) -> R) -> R {
    BRIDGE.with(|b| f(&mut b.borrow_mut()))
}

#[no_mangle]
pub extern "C" fn client_proto_ver() -> u32 {
    PROTO_VER as u32
}

#[no_mangle]
pub extern "C" fn client_in_ptr() -> *mut u8 {
    with(|b| b.in_buf.as_mut_ptr())
}

#[no_mangle]
pub extern "C" fn client_in_cap() -> u32 {
    IN_CAP as u32
}

#[no_mangle]
pub extern "C" fn client_out_ptr() -> *const u8 {
    with(|b| b.out_buf.as_ptr())
}

/// Encode the hello message into the out buffer; returns its length.
#[no_mangle]
pub extern "C" fn client_hello() -> u32 {
    with(|b| {
        encode_hello(
            &Hello {
                proto_ver: PROTO_VER,
            },
            &mut b.out_buf,
        )
        .map(|n| n as u32)
        .unwrap_or(0)
    })
}

/// Parse a handshake reply from the in buffer. Returns 1 (welcome) or
/// 2 (refuse) with fields in the handshake words, 0 on garbage.
#[no_mangle]
pub extern "C" fn client_parse_handshake(len: u32) -> u32 {
    with(|b| {
        let bytes = &b.in_buf[..(len as usize).min(IN_CAP)];
        match peek_kind(bytes) {
            Ok(KIND_WELCOME) => match decode_welcome(bytes) {
                Ok(w) => {
                    b.hs_buf = [
                        1,
                        w.player_id,
                        w.seed as u32,
                        (w.seed >> 32) as u32,
                        w.tick,
                        0,
                    ];
                    1
                }
                Err(_) => 0,
            },
            Ok(KIND_REFUSE) => match decode_refuse(bytes) {
                Ok(r) => {
                    b.hs_buf = [2, 0, 0, 0, 0, r.code as u32];
                    2
                }
                Err(_) => 0,
            },
            _ => 0,
        }
    })
}

#[no_mangle]
pub extern "C" fn client_hs_ptr() -> *const u32 {
    with(|b| b.hs_buf.as_ptr())
}

/// Create (or recreate) the client. Everything prior is discarded — a
/// reconnect is a fresh join at v0.
#[no_mangle]
pub extern "C" fn client_new(seed: u64, player_id: u32, server_tick: u32) {
    with(|b| b.core = Some(ClientCore::new(seed, player_id, server_tick)));
}

/// One incoming datagram of `len` bytes from the in buffer. Returns the
/// `Ingest` code (0 error · 1 applied · 2 applied-delta · 3 stale · 4
/// no-baseline), or 5 when no client exists.
#[no_mangle]
pub extern "C" fn client_on_datagram(len: u32) -> u32 {
    with(|b| {
        let n = (len as usize).min(IN_CAP);
        let Bridge {
            core: Some(core),
            in_buf,
            ..
        } = b
        else {
            return 5;
        };
        core.on_datagram(&in_buf[..n]) as u32
    })
}

#[no_mangle]
pub extern "C" fn client_set_input(buttons: u32, yaw: u32, pitch: u32, move_x: i32, move_z: i32) {
    with(|b| {
        if let Some(core) = b.core.as_mut() {
            core.set_input(
                buttons as u8,
                yaw as u16,
                pitch as u8,
                move_x.clamp(-127, 127) as i8,
                move_z.clamp(-127, 127) as i8,
            );
        }
    })
}

/// Advance real time; returns the fixed client ticks that ran.
#[no_mangle]
pub extern "C" fn client_advance(dt_ms: f64) -> u32 {
    with(|b| b.core.as_mut().map(|c| c.advance(dt_ms)).unwrap_or(0))
}

/// The due input datagram, encoded into the out buffer; 0 when none.
#[no_mangle]
pub extern "C" fn client_poll_input() -> u32 {
    with(|b| {
        let Bridge {
            core: Some(core),
            out_buf,
            ..
        } = b
        else {
            return 0;
        };
        core.poll_input(out_buf) as u32
    })
}

/// Fill the render buffer and remote-id buffer; returns the remote count.
/// Also decays the correction offset — call exactly once per render frame.
///
/// Float layout: [0] started · [1..4] own render x/y/z (smoothed) ·
/// [4] vy m/s · [5] grounded · [6] nudge isn't wire state client-side, so:
/// [6] correction error m · [7] mispredictions · [8] snapshots applied ·
/// [9] server tick estimate · [10] client tick · [11] hard resyncs ·
/// [12] reserved · [13] remote count · then per remote k at
/// `14 + k*8`: x, y, z, yaw (0..65536), pitch (0..255), live, vy?=0, 0.
#[no_mangle]
pub extern "C" fn client_render() -> u32 {
    with(|b| {
        let Bridge {
            core: Some(core),
            render,
            remote_ids,
            ..
        } = b
        else {
            return 0;
        };
        core.predict.decay_error();
        let p = core.predict.render_position();
        render[0] = if core.predict.started { 1.0 } else { 0.0 };
        render[1] = p[0];
        render[2] = p[1];
        render[3] = p[2];
        render[4] = core.predict.body.qvy as f32 * 0.01;
        render[5] = if core.predict.body.grounded { 1.0 } else { 0.0 };
        render[6] = core.predict.error_magnitude();
        render[7] = core.predict.mispredictions as f32;
        render[8] = core.snapshots_applied as f32;
        render[9] = core.clock.server_est as f32;
        render[10] = core.clock.client_tick as f32;
        render[11] = core.clock.resyncs as f32;
        render[12] = 0.0;
        let at = core.render_tick();
        let mut n = 0usize;
        let mut rs = crate::interp::RemoteState::default();
        // Collect ids first: sample borrows interp immutably.
        let mut ids = [0u32; MAX_SNAPSHOT_ENTITIES];
        let mut n_ids = 0usize;
        for id in core.interp.ids() {
            ids[n_ids] = id;
            n_ids += 1;
        }
        for &id in &ids[..n_ids] {
            if core.interp.sample(id, at, &mut rs) {
                let base = OWN_FLOATS + 1 + n * REMOTE_FLOATS;
                remote_ids[n] = id;
                render[base] = rs.x;
                render[base + 1] = rs.y;
                render[base + 2] = rs.z;
                render[base + 3] = rs.yaw;
                render[base + 4] = rs.pitch;
                render[base + 5] = if rs.live { 1.0 } else { 0.0 };
                render[base + 6] = 0.0;
                render[base + 7] = 0.0;
                n += 1;
            }
        }
        render[OWN_FLOATS] = n as f32;
        n as u32
    })
}

#[no_mangle]
pub extern "C" fn client_render_ptr() -> *const f32 {
    with(|b| b.render.as_ptr())
}

#[no_mangle]
pub extern "C" fn client_remote_ids_ptr() -> *const u32 {
    with(|b| b.remote_ids.as_ptr())
}

// ---------------------------------------------------------------------------
// Terrain: the shared worldgen, for the render worker (TERRAIN.md §4)
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn terrain_height_at(seed: u64, x: f32, z: f32) -> f32 {
    terrain::height(seed, x, z)
}

#[no_mangle]
pub extern "C" fn terrain_moisture_at(seed: u64, x: f32, z: f32) -> f32 {
    terrain::moisture(seed, x, z)
}

/// Fill an `n × n` height grid at `(x0 + i·step, z0 + j·step)`, row-major
/// by j. Returns the float count written, 0 if `n` exceeds the buffer.
#[no_mangle]
pub extern "C" fn terrain_fill_heights(seed: u64, x0: f32, z0: f32, n: u32, step: f32) -> u32 {
    with(|b| {
        let n = n as usize;
        if n == 0 || n > HEIGHTS_MAX_N {
            return 0;
        }
        for j in 0..n {
            let z = z0 + j as f32 * step;
            for i in 0..n {
                b.heights[j * n + i] = terrain::height(seed, x0 + i as f32 * step, z);
            }
        }
        (n * n) as u32
    })
}

#[no_mangle]
pub extern "C" fn terrain_heights_ptr() -> *const f32 {
    with(|b| b.heights.as_ptr())
}

/// Resolve the scatter slots of a `cells × cells` block starting at cell
/// `(cx0, cz0)`. Writes 6 floats per resolved slot — occupant, x, y, z,
/// yaw (0..255), scale — and returns the slot count.
#[no_mangle]
pub extern "C" fn terrain_fill_slots(seed: u64, cx0: i32, cz0: i32, cells: u32) -> u32 {
    with(|b| {
        let cells = (cells as usize).min(SLOTS_MAX_CELLS) as i32;
        let table = ScatterTable::alpha_default();
        let mut n = 0usize;
        for dz in 0..cells {
            for dx in 0..cells {
                let s = terrain::scatter(seed, &table, cx0 + dx, cz0 + dz);
                if s.occupant == terrain::Occupant::None {
                    continue;
                }
                let base = n * SLOT_FLOATS;
                b.slots[base] = s.occupant as u8 as f32;
                b.slots[base + 1] = s.x;
                b.slots[base + 2] = s.y;
                b.slots[base + 3] = s.z;
                b.slots[base + 4] = s.yaw as f32;
                b.slots[base + 5] = s.scale;
                n += 1;
            }
        }
        n as u32
    })
}

#[no_mangle]
pub extern "C" fn terrain_slots_ptr() -> *const f32 {
    with(|b| b.slots.as_ptr())
}
