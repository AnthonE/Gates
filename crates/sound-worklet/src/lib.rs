//! The `AudioWorkletProcessor`'s wasm module — glue, and nothing else.
//!
//! What the browser's audio thread runs. `crates/client-web/web/audio.js`
//! is the processor; this is what it calls per render quantum. Every
//! decision — the byte protocol, the de-interleave, the refusals and their
//! counters — is [`sound::worklet`], gated headless by
//! `crates/sound/tests/worklet.rs`. Read that file; this one has no opinions
//! in it.
//!
//! **Why the output crosses as a pointer.** `process()` is called every 128
//! frames — ~375 times a second at 48 kHz — and a `&[f32]` return through
//! wasm-bindgen is a freshly allocated `Float32Array` each time. Garbage at
//! that rate on the audio thread is the disease this whole slice exists to
//! cure (`findings/browser-audio-20260913.md`: cpal's web host allocated an
//! `AudioBuffer` and a source node per period, and that is what a GC pause is
//! made of). So the samples stay in wasm memory and JS copies them out of a
//! view it holds across calls.

#![cfg(target_arch = "wasm32")]

use sound::engine::BLOCK;
use sound::worklet::{Worklet as Inner, REPORT_BYTES};
use wasm_bindgen::prelude::*;

/// The renderer the processor owns, for one `AudioContext`.
#[wasm_bindgen]
pub struct Worklet {
    inner: Inner,
}

#[wasm_bindgen]
impl Worklet {
    /// Build a renderer for a context running at `out_rate` Hz — the
    /// `AudioContext`'s own `sampleRate`. The page reads it off the device
    /// and passes it down; nothing here chooses it, and `engine::rate` is
    /// then the only resample in the chain.
    #[wasm_bindgen(constructor)]
    pub fn new(out_rate: u32) -> Worklet {
        Worklet {
            inner: Inner::new(out_rate),
        }
    }

    /// Install a cue's samples, naming it by its `Cue::idx`. `false` when the
    /// index is not a cue or the cue is already installed — both counted, and
    /// neither a panic: a panic here takes the processor down for the rest of
    /// the session.
    pub fn install(&mut self, cue_idx: usize, pcm: &[i16]) -> bool {
        self.inner.install(cue_idx, pcm)
    }

    /// Queue a batch of 16-byte command records; how many were queued. A
    /// malformed record is refused and counted rather than guessed at.
    pub fn push(&mut self, bytes: &[u8]) -> usize {
        self.inner.push_bytes(bytes)
    }

    /// Render one quantum into the output buffer.
    pub fn render(&mut self) {
        self.inner.render();
    }

    /// The output buffer's address in wasm memory: `block()` left samples
    /// followed by `block()` right ones, planar, as `process()` wants them.
    ///
    /// ⚠ **The view JS builds over this is invalidated by memory growth** —
    /// installing a cue allocates — so the caller compares
    /// `memory.buffer` against the one it built the view from and rebuilds
    /// when they differ. A detached view reads as zeros, which is silence
    /// with every counter green, so `audio.js` checks rather than caches.
    #[wasm_bindgen(getter)]
    pub fn out_ptr(&self) -> *const f32 {
        self.inner.planar().as_ptr()
    }

    /// Frames per render quantum — `engine::BLOCK`, which is also the Web
    /// Audio render quantum. Read from here rather than typed as 128 in JS
    /// so the two cannot disagree.
    #[wasm_bindgen(getter)]
    pub fn block(&self) -> usize {
        BLOCK
    }

    /// The counters, as the `REPORT_BYTES` record `sound::worklet::Report`
    /// decodes on the page's side. Called on a slow cadence, not per
    /// quantum, so the allocation this returns is not on the hot path.
    pub fn report(&self) -> Vec<u8> {
        self.inner.report().to_bytes().to_vec()
    }

    /// The size of that record, so the page can check what it received
    /// rather than assume it.
    #[wasm_bindgen(getter)]
    pub fn report_bytes(&self) -> usize {
        REPORT_BYTES
    }
}
