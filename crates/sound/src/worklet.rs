//! The browser's half of the audio seam, as pure Rust.
//!
//! Natively the game thread hands [`engine::Renderer`] its work over two
//! `rtrb` rings and cpal calls it back on the device thread
//! (`client/src/render/audio_out.rs`). A tab has neither: an
//! `AudioWorkletProcessor` runs on the browser's own audio thread, in a
//! global scope with no shared memory with the page, and everything that
//! crosses does so as a `postMessage` **copy**. So the seam is a byte
//! protocol rather than a ring, and this module is both of its ends —
//! encoder and decoder in one file, round-tripped by
//! `crates/sound/tests/worklet.rs`, so the two halves cannot disagree about
//! a field the way the reference ecosystem's positional payloads did
//! (`CLAUDE.md`, the byte-golden trap).
//!
//! **Why this is not in the wasm crate.** `crates/sound-worklet` is the
//! `cdylib` a browser loads, and everything in it is `#[wasm_bindgen]` glue
//! — which a headless box cannot run. Every decision here is therefore in
//! this crate instead, where `cargo test --workspace` drives it with no
//! browser, exactly as `client-core/tests/wire.rs` gates the transport's
//! framing. The wasm crate is a wrapper with no logic in it.
//!
//! **What it must not do.** This runs on the audio thread, so it is held to
//! the renderer's discipline: no allocation after a cue is installed, every
//! input bounded and its refusals counted, no clock and no I/O
//! (`client/tests/platform_calls.rs` walks this crate too). The one
//! allocation is [`Worklet::install`], which copies a cue's samples out of
//! the JS heap into a box the renderer owns — out of band, once per cue per
//! boot, and the same shape as the box the native seam moves across its
//! install ring.

use crate::engine::{self, Cmd, Renderer, Stats, BLOCK, CMD_BYTES};
use crate::{Cue, CUE_COUNT};

/// A report record: the renderer's [`Stats`] plus the two counters that are
/// this seam's own, little-endian, fixed width.
///
/// Fixed rather than a JS object with named keys because the decoder is
/// Rust on the other side and a shape both ends agree on by construction is
/// cheaper than one they agree on by convention.
pub const REPORT_BYTES: usize = 56;

/// Write a `u32` at `at`.
fn put32(b: &mut [u8; REPORT_BYTES], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

/// Read a `u32` at `at`.
fn get32(b: &[u8; REPORT_BYTES], at: usize) -> u32 {
    let mut w = [0u8; 4];
    w.copy_from_slice(&b[at..at + 4]);
    u32::from_le_bytes(w)
}

/// What the worklet posts back, decoded: the renderer's counters and the
/// seam's two.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub stats: Stats,
    /// Command records the decoder refused as malformed ([`Worklet::push_bytes`]).
    pub bad_records: u32,
    /// Install messages naming a cue index at or past [`CUE_COUNT`].
    pub bad_installs: u32,
}

impl Report {
    /// Encode. Layout is `[refused, unbanked, cut, dropped, bad_cmd,
    /// reinstall_refused, return_dropped, live, installed, bad_records,
    /// bad_installs, pad, blocks(u64)]`, all little-endian.
    pub fn to_bytes(self) -> [u8; REPORT_BYTES] {
        let mut b = [0u8; REPORT_BYTES];
        let s = self.stats;
        put32(&mut b, 0, s.refused);
        put32(&mut b, 4, s.unbanked);
        put32(&mut b, 8, s.cut);
        put32(&mut b, 12, s.dropped);
        put32(&mut b, 16, s.bad_cmd);
        put32(&mut b, 20, s.reinstall_refused);
        put32(&mut b, 24, s.return_dropped);
        put32(&mut b, 28, s.live);
        put32(&mut b, 32, s.installed);
        put32(&mut b, 36, self.bad_records);
        put32(&mut b, 40, self.bad_installs);
        b[48..56].copy_from_slice(&s.blocks.to_le_bytes());
        b
    }

    /// Decode a record written by [`Report::to_bytes`].
    pub fn from_bytes(b: &[u8; REPORT_BYTES]) -> Report {
        let mut blocks = [0u8; 8];
        blocks.copy_from_slice(&b[48..56]);
        Report {
            stats: Stats {
                refused: get32(b, 0),
                unbanked: get32(b, 4),
                cut: get32(b, 8),
                dropped: get32(b, 12),
                bad_cmd: get32(b, 16),
                reinstall_refused: get32(b, 20),
                return_dropped: get32(b, 24),
                live: get32(b, 28),
                installed: get32(b, 32),
                blocks: u64::from_le_bytes(blocks),
            },
            bad_records: get32(b, 36),
            bad_installs: get32(b, 40),
        }
    }
}

/// The renderer, its scratch, and the seam's counters — everything the
/// `AudioWorkletProcessor` owns.
///
/// The processor's `process()` is handed **planar** channels (one
/// `Float32Array` per ear) and [`Renderer::render`] writes **interleaved**,
/// so the de-interleave happens here rather than in JS: it is a loop over
/// 128 frames either way, and in JS it would be a loop the page's own
/// garbage collector can interrupt.
pub struct Worklet {
    r: Box<Renderer>,
    /// Interleaved, as the renderer writes it: L R L R …
    lr: [f32; 2 * BLOCK],
    /// De-interleaved, as `process()` wants it: left in `[..BLOCK]`, right
    /// in `[BLOCK..]`. Held rather than written straight into JS memory
    /// because a wasm-bindgen `&mut [f32]` argument is a copy in AND out,
    /// and the copy in is of a buffer we are about to overwrite entirely.
    planar: [f32; 2 * BLOCK],
    bad_records: u32,
    bad_installs: u32,
}

impl Worklet {
    /// A worklet for a context running at `out_rate` Hz — the
    /// `AudioContext`'s own `sampleRate`, which the page reads off the
    /// device and never chooses. Nothing is installed yet; every cue is
    /// silence until [`Worklet::install`].
    pub fn new(out_rate: u32) -> Self {
        Self {
            r: Box::new(Renderer::new(out_rate)),
            lr: [0.0; 2 * BLOCK],
            planar: [0.0; 2 * BLOCK],
            bad_records: 0,
            bad_installs: 0,
        }
    }

    /// The rate the renderer was built for.
    pub fn out_rate(&self) -> u32 {
        self.r.out_rate()
    }

    /// Install a cue's samples, naming the cue by its [`Cue::idx`].
    ///
    /// An index at or past [`CUE_COUNT`] is refused and counted rather than
    /// panicking: it arrives from JS, and a panic on the audio thread takes
    /// the processor down for the rest of the session. A second install of
    /// a cue the renderer already holds is refused by [`Renderer::install`]
    /// and counted there; the box is dropped here, which is where it was
    /// allocated.
    pub fn install(&mut self, cue_idx: usize, pcm: &[i16]) -> bool {
        let Some(&cue) = Cue::ALL.get(cue_idx) else {
            self.bad_installs = self.bad_installs.saturating_add(1);
            return false;
        };
        self.r.install(cue, pcm.to_vec().into_boxed_slice()).is_ok()
    }

    /// Decode a batch of [`CMD_BYTES`]-byte command records and queue each
    /// for the next block; how many were queued.
    ///
    /// The renderer's own queue is bounded at `CMD_RING_CAP` and drops the
    /// newest past it ([`Renderer::dropped`]), so a page that posts faster
    /// than the audio thread renders loses the newest commands and says so —
    /// it does not grow a buffer on the audio thread. A trailing partial
    /// record, or one whose tag is not a [`Cmd`], is counted in
    /// [`Report::bad_records`] and skipped.
    pub fn push_bytes(&mut self, bytes: &[u8]) -> usize {
        let mut queued = 0;
        for rec in bytes.chunks_exact(CMD_BYTES) {
            let mut b = [0u8; CMD_BYTES];
            b.copy_from_slice(rec);
            match Cmd::from_bytes(&b) {
                Some(cmd) => {
                    self.r.push(cmd);
                    queued += 1;
                }
                None => self.bad_records = self.bad_records.saturating_add(1),
            }
        }
        if !bytes.len().is_multiple_of(CMD_BYTES) {
            self.bad_records = self.bad_records.saturating_add(1);
        }
        queued
    }

    /// Render one quantum and de-interleave it. [`Worklet::planar`] is the
    /// result.
    pub fn render(&mut self) {
        self.r.render(&mut self.lr);
        for i in 0..BLOCK {
            self.planar[i] = self.lr[2 * i];
            self.planar[BLOCK + i] = self.lr[2 * i + 1];
        }
    }

    /// The last [`Worklet::render`]'s output: left in `[..BLOCK]`, right in
    /// `[BLOCK..]`.
    pub fn planar(&self) -> &[f32; 2 * BLOCK] {
        &self.planar
    }

    /// The counters, as one record to post back.
    pub fn report(&self) -> Report {
        Report {
            stats: self.r.stats(),
            bad_records: self.bad_records,
            bad_installs: self.bad_installs,
        }
    }

    /// How many cues the renderer holds.
    pub fn installed(&self) -> usize {
        self.r.installed()
    }
}

/// Encode a frame's commands into `out` as [`CMD_BYTES`]-byte records; how
/// many bytes were written.
///
/// The page's end of the same protocol, here rather than in `client` so that
/// one file holds both halves. `out` is the caller's reused buffer and the
/// bound is its length: a command that does not fit is **not** written, and
/// the caller learns how many did from the return rather than from a
/// truncated last record — half a record is not a smaller command, the same
/// reason `net/web.rs` refuses an oversized datagram rather than truncating
/// it.
pub fn encode(cmds: impl Iterator<Item = Cmd>, out: &mut [u8]) -> usize {
    let mut at = 0;
    for cmd in cmds {
        if at + CMD_BYTES > out.len() {
            break;
        }
        out[at..at + CMD_BYTES].copy_from_slice(&cmd.to_bytes());
        at += CMD_BYTES;
    }
    at
}

/// The bytes a full frame of commands needs, for sizing the page's buffer.
pub const fn encode_cap(cmds: usize) -> usize {
    cmds * CMD_BYTES
}

const _: () = assert!(CUE_COUNT == Cue::ALL.len());
const _: () = assert!(engine::BLOCK == BLOCK);
