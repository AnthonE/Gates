/* Replay a Rust-authored fixture through the REAL browser audio module.
 *
 *   ./ci/check_worklet.mjs <audio.js> <sound_worklet_bg.wasm> <fixture.bin>
 *
 * **Not a gate, and it must not become one.** `CLAUDE.md` is explicit that no
 * gate in this repo starts a browser; this does not start one either, but it
 * does need node, which `ci/gates.sh` has never required. It is the audio
 * twin of `ci/build_web.sh`: a thing a person runs when they touch this seam.
 *
 * **What it proves that no Rust test can.** `crates/sound/tests/worklet.rs`
 * gates the arithmetic on a box with no browser, and it would pass with the
 * wasm module unbuildable, the glue not loading outside a DOM, `initSync`
 * taking a different argument shape, `out_ptr` pointing at the wrong array,
 * or the processor never registering. Each of those is silence with every
 * gate green — which is the exact failure this whole slice was written to
 * end, so the seam gets a check that crosses the ABI for real.
 *
 * The fixture is written by `cargo run -p sound --example worklet_fixture`:
 * the cue PCM, the command bytes `worklet::encode` produced, and the samples
 * a native `Renderer` rendered from them. So the comparison is the browser's
 * module against this repo's own arithmetic, not against typed-in numbers.
 *
 * ## Mutants run by hand against `audio-processor.js` (2026-09-16)
 *
 * | mutant | reddened by |
 * |---|---|
 * | ears crossed in `process()` | the comparison, quantum 1 sample 125 L: −0.1669 against −0.7510 |
 * | the memory-growth check removed (view cached once) | a `TypeError` on the detached view at the first quantum after an install |
 * | `process()` returns false | "process() returned false at quantum 0" |
 * | the report cadence never fires | "no report was ever posted" |
 * | the `install` message dropped | the comparison, 0 against −0.7510 |
 * | the `cmds` message dropped | the comparison, 0 against −0.7510 |
 *
 * ⚠ **The growth mutant was GREEN until the fixture spread its installs**, and
 * that is the finding worth keeping. The first version installed every cue
 * before the first `process()`, so the view was built after the last growth
 * and the rebuild was never exercised — a check aimed squarely at the
 * production path (`SPREAD_BANK` is true on wasm32: one cue per frame, while
 * rendering) that could not see it. Same family as `client/tests/ground.rs`'s
 * four origins that never landed on the coast road. */

import { readFileSync } from "node:fs";

const [audioJs, wasmPath, fixturePath] = process.argv.slice(2);
if (!audioJs || !wasmPath || !fixturePath) {
  console.error("usage: check_worklet.mjs <audio.js> <sound_worklet_bg.wasm> <fixture.bin>");
  process.exit(2);
}

/* ── the fixture ─────────────────────────────────────────────────────────── */
const fx = readFileSync(fixturePath);
let at = 0;
const u32 = () => { const v = fx.readUInt32LE(at); at += 4; return v; };
const outRate = u32();
const cues = [];
for (let n = u32(), i = 0; i < n; i++) {
  const idx = u32();
  const installAt = u32();
  const len = u32();
  // A copy, because the fixture Buffer's byteOffset is not 2-byte aligned.
  const pcm = new Int16Array(len);
  for (let s = 0; s < len; s++) pcm[s] = fx.readInt16LE(at + s * 2);
  at += len * 2;
  cues.push({ idx, installAt, pcm });
}
const cmdLen = u32();
const cmdBytes = new Uint8Array(fx.subarray(at, at + cmdLen)); at += cmdLen;
const quanta = u32();
const BLOCK = 128;
const expected = [];
for (let q = 0; q < quanta; q++) {
  const planar = new Float32Array(2 * BLOCK);
  for (let i = 0; i < 2 * BLOCK; i++) planar[i] = fx.readFloatLE(at + i * 4);
  at += 2 * BLOCK * 4;
  expected.push(planar);
}

/* ── the worklet scope, stubbed ──────────────────────────────────────────── */
const fromWorklet = [];
class FakePort {
  constructor() { this.onmessage = null; this.onmessageerror = null; }
  postMessage(m) { fromWorklet.push(m); }
}
class AudioWorkletProcessor {
  constructor() { this.port = new FakePort(); }
}
let Processor = null;
const registerProcessor = (_name, cls) => { Processor = cls; };

/* `new Function` rather than an import: the served file is a classic script
   (the glue's `let wasm_bindgen` is top-level) and this is how a worklet runs
   it — as a body, with the scope's own globals injected. */
const src = readFileSync(audioJs, "utf8");
new Function("AudioWorkletProcessor", "registerProcessor", src)(
  AudioWorkletProcessor, registerProcessor,
);
if (!Processor) { console.error("FAIL: audio.js registered no processor"); process.exit(1); }

const proc = new Processor();
const deliver = (m) => proc.port.onmessage({ data: m });

/* The page's job: compile and post. A worklet cannot fetch. */
const module = new WebAssembly.Module(readFileSync(wasmPath));
deliver({ kind: "init", module, rate: outRate });
deliver({ kind: "cmds", bytes: cmdBytes });

const errors = fromWorklet.filter((m) => m.kind === "error");
if (errors.length) { console.error("FAIL: processor errored -", errors[0].why); process.exit(1); }
if (!fromWorklet.some((m) => m.kind === "ready")) {
  console.error("FAIL: processor never reported ready"); process.exit(1);
}

/* ── render and compare ──────────────────────────────────────────────────── */
let worst = 0, worstAt = "", peak = 0;
for (let q = 0; q < quanta; q++) {
  /* Spread, as `SPREAD_BANK` spreads it: a cue lands between quanta while the
     processor is already rendering. Each install allocates and so grows wasm
     memory, which detaches the processor's view of the output — the rebuild
     that catches is only exercised here. */
  for (const c of cues) if (c.installAt === q) deliver({ kind: "install", cue: c.idx, pcm: c.pcm });
  const left = new Float32Array(BLOCK), right = new Float32Array(BLOCK);
  if (proc.process([], [[left, right]], {}) !== true) {
    console.error(`FAIL: process() returned false at quantum ${q} — the node retires`);
    process.exit(1);
  }
  for (let i = 0; i < BLOCK; i++) {
    peak = Math.max(peak, Math.abs(left[i]), Math.abs(right[i]));
    for (const [got, want, ear] of [[left[i], expected[q][i], "L"],
                                    [right[i], expected[q][BLOCK + i], "R"]]) {
      const d = Math.abs(got - want);
      if (d > worst) { worst = d; worstAt = `quantum ${q} sample ${i} ${ear}: ${got} against ${want}`; }
    }
  }
}
if (worst !== 0) {
  console.error(`FAIL: the browser module does not render what the native path renders.`);
  console.error(`      worst ${worst} at ${worstAt}`);
  process.exit(1);
}
/* A module that renders silence matches nothing, but say it out loud rather
   than inferring it from a passing diff. */
if (peak <= 0.05) { console.error(`FAIL: rendered a peak of ${peak} — silence`); process.exit(1); }

/* The report has to cross too, and at the size the page decodes. */
for (let q = 0; q < 200; q++) proc.process([], [[new Float32Array(BLOCK), new Float32Array(BLOCK)]], {});
const report = fromWorklet.filter((m) => m.kind === "report").pop();
if (!report) { console.error("FAIL: no report was ever posted"); process.exit(1); }
if (report.bytes.length !== 56) {
  console.error(`FAIL: report is ${report.bytes.length} bytes, the page decodes 56`);
  process.exit(1);
}
const late = fromWorklet.filter((m) => m.kind === "error");
if (late.length) { console.error("FAIL: processor errored while rendering -", late[0].why); process.exit(1); }

console.log(`OK: ${quanta} quanta at ${outRate} Hz bit-identical to the native renderer`);
console.log(`    peak ${peak.toFixed(4)}, ${cues.length} cues installed, report ${report.bytes.length} bytes`);
