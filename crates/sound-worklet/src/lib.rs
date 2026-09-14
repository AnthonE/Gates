//! The browser's audio thread: `crates/sound`'s renderer inside an
//! `AudioWorkletProcessor`, as a wasm module of its own.
//!
//! **This is `render/audio_out.rs` on the other target** — the seam between
//! the game thread and the audio thread, and nothing else: the pool, the
//! pan, the ramps and the counters are all `sound::engine::Renderer`, gated
//! headless in `crates/sound/tests/`. Natively the seam is two SPSC rings
//! into a cpal callback. Here the audio thread is a different agent with its
//! own linear memory (`findings/web-build-20260909.md` §4.1: no shared
//! memory and no atomics at this Bevy version, and no header flips that), so
//! the seam is `postMessage`: the game thread posts a frame's commands as
//! [`CMD_BYTES`]-byte records (`Cmd::to_bytes`), the worklet's JS copies them
//! into [`cmd_buf`] and calls [`push`]; each cue's PCM crosses once, into a
//! box [`bank_alloc`] reserves and [`bank_done`] installs.
//! `crates/client-web/web/worklet.js` is the JS half, and it only copies.
//!
//! ## What allocates, and where
//!
//! **Nothing per quantum.** [`render`] fills a fixed array in a static and
//! returns its address; [`push`] decodes fixed records out of a static; the
//! [`report`] is a fixed array too. The allocations there are: [`init`],
//! once — the renderer, ~4 KB, built by value and moved into the static —
//! and [`bank_alloc`], once per cue: the bank's samples, the only heap the
//! renderer owns. Natively those arrive already boxed over the install ring;
//! here the box is made on this side because the bytes have to be copied
//! into this memory anyway, and the reservation may grow the memory, which
//! is why the JS re-makes its views when `memory.buffer` changes. **A bank
//! box is never freed here**: a second reservation for a cue is refused by
//! this module's own guard before the engine's, so the engine's hand-back
//! path is unreachable, and a free on the audio thread never happens.
//!
//! ## Pointers, not strings
//!
//! Every export takes and returns integers. An `AudioWorkletGlobalScope` has
//! no `TextDecoder`, so a string crossing here would need a decoder the
//! scope cannot construct — `ci/build_web.sh` refuses a generated glue that
//! constructs one unguarded. It also means no `String`, no `format!` and no
//! `unwrap` in this file: a fault is a counter, never a message.
//!
//! ## Counters
//!
//! Every fault this module can see folds into the engine's own report so
//! the page reads ONE record: records past [`CMD_BUF_RECORDS`] in a message
//! fold into `dropped` (the engine's drop-newest, one hop earlier); a
//! malformed record is the engine's own `bad_cmd`; a port message the JS
//! cannot route, a bank this module refuses, a second [`init`] and a call
//! before it all fold into `bad_cmd` through the counter [`bad_message`]
//! bumps. [`report`]'s doc is the layout.
//!
//! ## Why there is no panic hook, and why `thread_local!` is a static
//!
//! The web profile is `panic = "abort"`, so a panic here is a trap with no
//! message, by design: a hook would need `console` through `web-sys` and a
//! string through a decoder the scope cannot make, and everything a panic
//! could say is already a counter. And `thread_local!` on
//! `wasm32-unknown-unknown` without atomics is std's `statik` mode — a plain
//! static behind a `RefCell`, no lazy init, no destructor — so every address
//! handed to JS is stable for the life of the module (`memory.grow` extends
//! linear memory at its end and moves nothing), and no `unsafe` is needed to
//! hold state a JS caller re-enters synchronously.
#![cfg(target_arch = "wasm32")]

use std::cell::RefCell;

use sound::engine::{Cmd, Renderer, BLOCK, CMD_BYTES};
use sound::{Cue, CUE_COUNT, SAMPLE_RATE};
use wasm_bindgen::prelude::*;

/// Records the command buffer holds: one game frame, which is `client`'s
/// `CMD_FRAME_CAP` (`render/audio.rs`) — the same 32. This crate links
/// `sound` alone and cannot reach that constant to assert it, so the
/// registry pins both names (`DECISIONS.md` §open, browser audio v0). A
/// message carrying more is clamped and the excess counted as dropped.
pub const CMD_BUF_RECORDS: usize = 32;

/// The longest bank a cue may install, in samples: 30 s at the bank's rate,
/// 2.5× the 12 s bed that is the longest cue synthesised
/// (`sound::synth::BED_SECS`). A length past it is refused and counted
/// rather than allocated — a page cannot make this thread reserve a
/// gigabyte by posting one number.
pub const BANK_LEN_MAX: u32 = 1_323_000;
const _: () = assert!(BANK_LEN_MAX == 30 * SAMPLE_RATE);

/// Words in the [`report`]. `worklet.js` mirrors the number and posts its
/// own one word after them.
pub const REPORT_WORDS: usize = 8;

const NO_BOX: Option<Box<[i16]>> = None;

/// Everything the module holds, in one static.
struct State {
    renderer: Option<Renderer>,
    /// The command buffer the JS copies a frame's records into.
    cmd: [[u8; CMD_BYTES]; CMD_BUF_RECORDS],
    /// One rendered block, interleaved stereo. Zero until the first render.
    out: [f32; 2 * BLOCK],
    report: [u32; REPORT_WORDS],
    /// A bank box [`bank_alloc`] reserved and [`bank_done`] has not installed.
    pending: [Option<Box<[i16]>>; CUE_COUNT],
    /// Cues installed through this module — refused here before the
    /// engine's own reinstall guard, so no box ever comes back to be freed.
    banked: [bool; CUE_COUNT],
    /// Records past [`CMD_BUF_RECORDS`] in one message. Reported as dropped.
    over: u32,
    /// Port messages and calls this module refused. Reported as bad_cmd.
    bad_msg: u32,
}

impl State {
    const NEW: State = State {
        renderer: None,
        cmd: [[0; CMD_BYTES]; CMD_BUF_RECORDS],
        out: [0.0; 2 * BLOCK],
        report: [0; REPORT_WORDS],
        pending: [NO_BOX; CUE_COUNT],
        banked: [false; CUE_COUNT],
        over: 0,
        bad_msg: 0,
    };
}

thread_local! {
    static STATE: RefCell<State> = const { RefCell::new(State::NEW) };
}

/// One more of a saturating counter. A free function rather than a method
/// on [`State`] so it can be called on one field while another is borrowed.
fn bump(n: &mut u32) {
    *n = n.saturating_add(1);
}

/// Build the renderer for a device at `sample_rate` Hz — the worklet's
/// `sampleRate` global, the rate every `Start`'s `rate` is computed against
/// on the game thread (`render/audio.rs::Engine::out_rate`, which the page
/// sets from the same `AudioContext`). Once: a second call is refused and
/// counted, because re-making the renderer would drop the bank on this
/// thread.
#[wasm_bindgen]
pub fn init(sample_rate: u32) {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        if s.renderer.is_some() {
            bump(&mut s.bad_msg);
            return;
        }
        s.renderer = Some(Renderer::new(sample_rate));
    });
}

/// The command buffer: [`cmd_buf_len`] bytes at this address, which the JS
/// fills with whole records (`Cmd::to_bytes`, [`cmd_bytes`] each) before it
/// calls [`push`]. The address is static.
#[wasm_bindgen]
pub fn cmd_buf() -> *mut u8 {
    STATE.with(|s| s.borrow_mut().cmd.as_flattened_mut().as_mut_ptr())
}

/// Bytes per record — `sound::engine::CMD_BYTES`, so the JS never types 16.
#[wasm_bindgen]
pub fn cmd_bytes() -> u32 {
    CMD_BYTES as u32
}

/// Bytes the command buffer holds — `CMD_BUF_RECORDS · cmd_bytes()`, so the
/// JS clamps its copy to the buffer without typing either.
#[wasm_bindgen]
pub fn cmd_buf_len() -> u32 {
    (CMD_BUF_RECORDS * CMD_BYTES) as u32
}

/// Queue the first `records` records of the command buffer for the next
/// block. Past [`CMD_BUF_RECORDS`] the rest were never copied (the JS clamps
/// its copy to [`cmd_buf_len`]) and are counted as dropped — drop-newest,
/// the engine's own policy. A record `Cmd::from_bytes` refuses is the
/// engine's `bad_cmd`, and the records after it still land. A command
/// pushed here is applied by the NEXT [`render`], never inside a block that
/// was rendered before it arrived — the native drain-before-render contract.
#[wasm_bindgen]
pub fn push(records: u32) {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        let s = &mut *s;
        let Some(r) = s.renderer.as_mut() else {
            bump(&mut s.bad_msg);
            return;
        };
        let n = (records as usize).min(CMD_BUF_RECORDS);
        s.over = s.over.saturating_add((records as usize - n) as u32);
        for rec in &s.cmd[..n] {
            match Cmd::from_bytes(rec) {
                Some(cmd) => r.push(cmd),
                None => bump(&mut r.bad_cmd),
            }
        }
    });
}

/// Reserve `len` samples for `cue`'s bank and return where the JS copies
/// them — or null, refused and counted, when there is no renderer, `cue` is
/// off `Cue::ALL`, `len` is zero or past [`BANK_LEN_MAX`], the cue is
/// already banked, a reservation is already pending for it, or the memory
/// cannot grow. The one allocation per cue; a box's contents do not move,
/// so the address holds until [`bank_done`] installs it. **The JS must take
/// its view of memory AFTER this call**: the reservation may have grown the
/// memory, which detaches every view of the old buffer.
#[wasm_bindgen]
pub fn bank_alloc(cue: u32, len: u32) -> *mut i16 {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        let s = &mut *s;
        let Some(&c) = Cue::ALL.get(cue as usize) else {
            bump(&mut s.bad_msg);
            return core::ptr::null_mut();
        };
        let i = c.idx();
        let ok = s.renderer.is_some()
            && len != 0
            && len <= BANK_LEN_MAX
            && !s.banked[i]
            && s.pending[i].is_none();
        if !ok {
            bump(&mut s.bad_msg);
            return core::ptr::null_mut();
        }
        // Refused rather than trapped when the memory cannot grow: an
        // allocation failure on this thread would otherwise abort the
        // module with no message, and the page's sound with it.
        let mut v: Vec<i16> = Vec::new();
        if v.try_reserve_exact(len as usize).is_err() {
            bump(&mut s.bad_msg);
            return core::ptr::null_mut();
        }
        v.resize(len as usize, 0);
        let mut b = v.into_boxed_slice();
        let p = b.as_mut_ptr();
        s.pending[i] = Some(b);
        p
    })
}

/// Install the bank [`bank_alloc`] reserved for `cue`, now that the JS has
/// copied the samples in. No reservation pending, or a cue off `Cue::ALL`,
/// is refused and counted. The engine's own reinstall refusal cannot fire
/// through here — this module refuses the reservation first — and if it
/// ever did, the box it hands back is parked, not freed, and counted.
#[wasm_bindgen]
pub fn bank_done(cue: u32) {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        let s = &mut *s;
        let Some(&c) = Cue::ALL.get(cue as usize) else {
            bump(&mut s.bad_msg);
            return;
        };
        let i = c.idx();
        let Some(r) = s.renderer.as_mut() else {
            bump(&mut s.bad_msg);
            return;
        };
        let Some(pcm) = s.pending[i].take() else {
            bump(&mut s.bad_msg);
            return;
        };
        match r.install(c, pcm) {
            Ok(()) => s.banked[i] = true,
            Err(pcm) => {
                // Unreachable through this module's own guard. If it fires,
                // the engine holds a bank for this cue, this box stays here
                // rather than being freed on the audio thread, and the fact
                // is a number the page can read.
                s.pending[i] = Some(pcm);
                s.banked[i] = true;
                bump(&mut s.bad_msg);
            }
        }
    });
}

/// A port message the JS could not route — not a frame of records, not a
/// bank. Counted here so it lands in the report's `bad_cmd` rather than in
/// JS prose nobody reads.
#[wasm_bindgen]
pub fn bad_message() {
    STATE.with(|s| bump(&mut s.borrow_mut().bad_msg));
}

/// Render one block — `2 · BLOCK` interleaved floats, left then right — and
/// return its address (static). Applies every command [`push`]ed since the
/// last block first. Before [`init`] the block is the silence it started as,
/// and the call is counted.
#[wasm_bindgen]
pub fn render() -> *const f32 {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        let s = &mut *s;
        match s.renderer.as_mut() {
            Some(r) => r.render(&mut s.out),
            None => bump(&mut s.bad_msg),
        }
        s.out.as_ptr()
    })
}

/// Fill the report and return its address (static): [`REPORT_WORDS`]
/// `u32`s, in this order —
///
/// | word | what |
/// |---|---|
/// | 0 | `live` — one-shots sounding at the last block |
/// | 1 | `refused` — starts refused for want of a free voice |
/// | 2 | `unbanked` — starts and plays of a cue with no bank |
/// | 3 | `dropped` — commands the queue refused, plus records past the buffer |
/// | 4 | `bad_cmd` — malformed records, plus messages and calls this module refused |
/// | 5 | `cut` — held slots taken over while busy |
/// | 6 | `installed` — cues with a bank |
/// | 7 | `blocks` — rendered, saturated to `u32` |
///
/// Every counter is cumulative, so a report the page misses costs nothing.
/// `worklet.js` mirrors the layout and posts these plus its own `wrong`.
#[wasm_bindgen]
pub fn report() -> *const u32 {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        let s = &mut *s;
        let st = s.renderer.as_ref().map(Renderer::stats).unwrap_or_default();
        s.report = [
            st.live,
            st.refused,
            st.unbanked,
            st.dropped.saturating_add(s.over),
            st.bad_cmd.saturating_add(s.bad_msg),
            st.cut,
            st.installed,
            u32::try_from(st.blocks).unwrap_or(u32::MAX),
        ];
        s.report.as_ptr()
    })
}
