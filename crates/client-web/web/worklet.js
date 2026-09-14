/* ── the page's audio thread ───────────────────────────────────────────────
   An AudioWorkletProcessor over the `sound-worklet` module
   (`crates/sound-worklet/src/lib.rs`), and it only copies.

   What this file MAY do: instantiate the module the page compiled and handed
   over in `processorOptions`; copy a posted frame of command records into the
   module's buffer and say how many; copy a posted bank into the box the
   module reserves for it; copy each rendered block out into the two channel
   arrays; and post the module's counters, seconds apart.

   What it may NOT do, and does not: load anything (a worklet has no fetch,
   and it must not decide what to load — the page compiles the module);
   construct a string (an AudioWorkletGlobalScope has no TextDecoder);
   mix, pan, gain, cull, time or route a cue — every sample written below is
   the Rust renderer's, so `crates/sound/tests/` is the gate on what is heard
   and this file has nothing to test. The counters are the module's too: a
   message this file cannot route is reported through `bad_message()`, not
   through prose.

   Allocation: NONE per quantum. `process` reads the rendered block through a
   cached view and writes it into the browser's channel arrays with one fixed
   loop. The views are re-made only when `memory.buffer` changes — a bank
   reservation may grow the memory, which detaches every view of the old
   buffer — or on the first quantum, when the block's address is first known
   (it is a static and never moves; the check is one compare). The ONE
   allocation on this thread is the report, one small array posted every
   `reportEvery` quanta, seconds apart: a per-quantum `postMessage` is
   exactly the garbage `findings/browser-audio-20260913.md` §2 names, so
   nothing here posts per quantum. The per-message allocation that remains is
   the browser's own deserialisation of what the page posts — a `Uint8Array`
   a frame, and one `Int16Array` per cue, once — which lands on this thread
   and is not this file's to remove. */
import { initSync } from "./sound_worklet.js";

/* Mirrors of the Rust side, pinned by `ci/knob_registry.mjs` against
   `sound::engine::BLOCK` and `sound_worklet::REPORT_WORDS` (`DECISIONS.md`
   §open: audio engine v0, browser audio v0). */
const BLOCK = 128;
const REPORT_WORDS = 8;
/* Seconds between reports when the page passes no `reportEvery`. */
const REPORT_S_FALLBACK = 2;

class GatesProcessor extends AudioWorkletProcessor {
  constructor(options) {
    super(options);
    const { module, reportEvery } = options.processorOptions;
    /* The raw exports: `initSync` returns `instance.exports`, so every call
       below is the wasm function itself. A wasm i32 comes back SIGNED, so
       every address and count is `>>> 0` — what the generated wrappers do,
       and this file uses the raw exports rather than them. */
    this.wasm = initSync({ module });
    this.wasm.init(sampleRate);
    this.cmdPtr = this.wasm.cmd_buf() >>> 0;
    this.cmdLen = this.wasm.cmd_buf_len() >>> 0;
    this.cmdBytes = this.wasm.cmd_bytes() >>> 0;
    this.repPtr = this.wasm.report() >>> 0;
    /* The block's address is known from the first `render()`; until then
       the block has no view, and `outPtr` says so. */
    this.outPtr = null;
    this.buf = null;
    this.cmd = null;
    this.rep = null;
    this.out = null;
    this.reportEvery = reportEvery > 0
      ? Math.floor(reportEvery)
      : Math.round((REPORT_S_FALLBACK * sampleRate) / BLOCK);
    this.quanta = 0;
    /* Quanta whose output was not two channels of BLOCK frames: nothing was
       written, and this says so. Posted after the module's words. */
    this.wrong = 0;
    this.port.onmessage = (e) => this.onMessage(e.data);
  }

  /* Re-make every view over the module's memory. The command buffer and the
     report are statics read once at construction; the block's address is
     `outPtr`, when known. */
  remap() {
    const buf = this.wasm.memory.buffer;
    this.buf = buf;
    this.cmd = new Uint8Array(buf, this.cmdPtr, this.cmdLen);
    this.rep = new Uint32Array(buf, this.repPtr, REPORT_WORDS);
    this.out = this.outPtr === null ? null : new Float32Array(buf, this.outPtr, 2 * BLOCK);
  }

  onMessage(d) {
    if (d instanceof Uint8Array) {
      /* A frame of command records, `cmdBytes` each. Copied whole into the
         module's buffer, clamped to it; the module counts the records past
         the buffer as dropped and refuses a malformed record itself. A
         length that is not whole records is a message the page got wrong,
         and is counted as such — the whole records in it still land. */
      if (this.buf !== this.wasm.memory.buffer) this.remap();
      this.cmd.set(d.length <= this.cmd.length ? d : d.subarray(0, this.cmd.length));
      if (d.length % this.cmdBytes !== 0) this.wasm.bad_message();
      this.wasm.push(Math.floor(d.length / this.cmdBytes));
    } else if (d && Number.isInteger(d.cue) && d.cue >= 0
               && (d.pcm instanceof Int16Array || d.pcm instanceof ArrayBuffer)) {
      /* A cue's bank, once. The module reserves the box and refuses — null,
         counted — a cue it has, a length it will not hold, or a memory it
         cannot grow. The view is taken AFTER the reservation: it may have
         grown the memory, and a view of the old buffer would be detached. */
      const s = d.pcm instanceof Int16Array ? d.pcm : new Int16Array(d.pcm);
      const p = this.wasm.bank_alloc(d.cue, s.length) >>> 0;
      if (p !== 0) {
        new Int16Array(this.wasm.memory.buffer, p, s.length).set(s);
        this.wasm.bank_done(d.cue);
      }
    } else {
      this.wasm.bad_message();
    }
  }

  process(inputs, outputs) {
    /* Render first, whatever the output's shape: the renderer's clock and
       its counters advance either way, which is how the page can tell a
       mis-wired node from a dead one. */
    const p = this.wasm.render() >>> 0;
    if (p !== this.outPtr || this.buf !== this.wasm.memory.buffer) {
      this.outPtr = p;
      this.remap();
    }
    const l = outputs[0]?.[0];
    const r = outputs[0]?.[1];
    if (l === undefined || r === undefined || l.length !== BLOCK || r.length !== BLOCK) {
      /* Not two channels of one block: nothing is written (the browser hands
         over silence) and the shape is counted, never guessed at. */
      this.wrong += 1;
    } else {
      const out = this.out;
      for (let i = 0, j = 0; i < BLOCK; i += 1, j += 2) {
        l[i] = out[j];
        r[i] = out[j + 1];
      }
    }
    this.quanta += 1;
    if (this.quanta >= this.reportEvery) {
      this.quanta = 0;
      /* The one allocation on this thread: the module's words plus this
         file's, transferred rather than copied. `report()` fills a static
         the cached view already covers. */
      this.wasm.report();
      const m = new Uint32Array(REPORT_WORDS + 1);
      m.set(this.rep);
      m[REPORT_WORDS] = this.wrong;
      this.port.postMessage(m, [m.buffer]);
    }
    return true;
  }
}

registerProcessor("gates", GatesProcessor);
