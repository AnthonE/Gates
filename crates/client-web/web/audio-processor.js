/* ── the game's audio thread ────────────────────────────────────────────────
   The `AudioWorkletProcessor` half of `crates/client/src/render/audio_web.rs`.

   **This file is not served as it is written.** `ci/build_web.sh` concatenates
   `audio-prelude.js` and the `sound-worklet` module's wasm-bindgen glue in
   front of it and writes the three as `audio.js`, which is what `addModule`
   loads. The glue is generated
   with `--target no-modules` on purpose: it defines one global,
   `wasm_bindgen`, and contains no `import`, so the whole processor is a single
   script with no module graph to resolve. An `AudioWorkletGlobalScope` has no
   `fetch`, no `XMLHttpRequest` and no DOM, and static `import` inside a worklet
   has been uneven across browsers for years — so the safe shape is one file
   that asks for nothing.

   The `.wasm` cannot be fetched from in here either, which is why the page
   fetches it and posts the BYTES down the port, and `initSync` compiles and
   instantiates them without awaiting. Not a compiled `WebAssembly.Module`:
   Chromium 151 does not deserialize one in this scope (`app.js` has the
   measurement), and a message that does not decode arrives here as
   `messageerror`, which is now reported rather than read as silence.

   **Nothing here decides anything.** Every refusal, every counter and the
   de-interleave are `sound::worklet`, gated on a box with no browser by
   `crates/sound/tests/worklet.rs`. What is genuinely this file's is the
   quantum loop and the memory-growth check below, and both are written to be
   read rather than trusted. */

/* Quanta between reports: ~256 ms at 48 kHz. The counters are gauges on the
   F7 report, not a control loop, and a message per quantum would be 375
   allocations a second on this thread — the disease, not the cure. */
const REPORT_EVERY = 96;

class GatesAudio extends AudioWorkletProcessor {
  constructor() {
    super();
    this.w = null;
    this.memory = null;
    /* The ArrayBuffer `view` was built over. Compared by identity every
       quantum — see `process`. */
    this.buffer = null;
    this.view = null;
    this.sinceReport = 0;
    this.port.onmessage = (e) => this.onMessage(e.data);
    this.port.onmessageerror = () =>
      this.port.postMessage({ kind: "error", why: "a message did not decode in the worklet" });
  }

  onMessage(m) {
    try {
      if (m.kind === "init") {
        /* `initSync` returns the instance's exports, which is where `memory`
           comes from. Nothing else in this file may touch the wasm heap. */
        const exports = wasm_bindgen.initSync({ module: m.bytes });
        this.memory = exports.memory;
        this.w = new wasm_bindgen.Worklet(m.rate);
        this.port.postMessage({ kind: "ready" });
        return;
      }
      if (!this.w) return;
      if (m.kind === "install") this.w.install(m.cue, m.pcm);
      else if (m.kind === "cmds") this.w.push(m.bytes);
    } catch (e) {
      /* A throw in here would otherwise be swallowed by the worklet scope and
         read on the page as ordinary silence — the failure shape this whole
         slice exists to stop being invisible. */
      this.port.postMessage({ kind: "error", why: String((e && e.message) || e) });
    }
  }

  process(inputs, outputs) {
    const w = this.w;
    const out = outputs[0];
    if (!w || !out || out.length === 0) return true;

    const left = out[0];
    const right = out.length > 1 ? out[1] : null;
    const block = w.block;

    /* The render quantum is 128 frames today and `engine::BLOCK` is 128 to
       match, but the spec has a variable-quantum proposal and `left.length`
       is the number actually being asked for — so this fills whatever it is
       handed rather than asserting the two agree. */
    let filled = 0;
    while (filled < left.length) {
      w.render();
      /* ⚠ **Growing wasm memory detaches every view over it.** Installing a
         cue allocates ~12 MB across the bank, so this WILL happen during
         loading. A detached Float32Array reads as zeros — silence with every
         counter green, which is exactly the class of bug that put this file
         here — so the buffer is compared by identity rather than cached once.
         One reference compare per quantum. */
      if (this.buffer !== this.memory.buffer) {
        this.buffer = this.memory.buffer;
        this.view = new Float32Array(this.buffer, w.out_ptr, 2 * block);
      }
      const n = Math.min(block, left.length - filled);
      left.set(this.view.subarray(0, n), filled);
      if (right) right.set(this.view.subarray(block, block + n), filled);
      filled += n;
    }
    /* A context with more than two output channels gets the left ear on the
       rest rather than silence, which is `Feed::fill`'s rule natively for the
       same reason: a six-channel device must not play the game into two
       speakers and hiss out of four. */
    for (let c = 2; c < out.length; c++) out[c].set(left);

    if (++this.sinceReport >= REPORT_EVERY) {
      this.sinceReport = 0;
      this.port.postMessage({ kind: "report", bytes: w.report() });
    }
    /* Never false: returning false retires the processor and the node goes
       permanently silent. */
    return true;
  }
}

registerProcessor("gates-audio", GatesAudio);
