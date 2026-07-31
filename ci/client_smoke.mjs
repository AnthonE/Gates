#!/usr/bin/env node
// Bridge smoke for the web client: loads client_wasm.wasm exactly as the
// browser does (raw C ABI, no bindgen) and drives the exported surface —
// client lifecycle, input encode, render fill, terrain fill. Logic
// correctness is owned by the native tests (client-wasm unit tests +
// server/tests/client_loop.rs); this gate proves the wasm artifact
// actually exposes and runs that logic. Any missing artifact or export is
// a loud failure, never a silent skip (CLAUDE.md trap list).

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const wasmPath = join(
  root,
  "target/wasm32-unknown-unknown/release/client_wasm.wasm",
);

let bytes;
try {
  bytes = readFileSync(wasmPath);
} catch {
  console.error(`GATE FAIL: wasm artifact missing at ${wasmPath}`);
  console.error(
    "build it: cargo build -p client-wasm --release --target wasm32-unknown-unknown",
  );
  process.exit(1);
}

const { instance } = await WebAssembly.instantiate(bytes, {});
const ex = instance.exports;

const REQUIRED = [
  "memory",
  "client_proto_ver",
  "client_in_ptr",
  "client_in_cap",
  "client_out_ptr",
  "client_hello",
  "client_parse_handshake",
  "client_hs_ptr",
  "client_new",
  "client_on_datagram",
  "client_set_input",
  "client_advance",
  "client_poll_input",
  "client_render",
  "client_render_ptr",
  "client_remote_ids_ptr",
  "terrain_height_at",
  "terrain_moisture_at",
  "terrain_fill_heights",
  "terrain_heights_ptr",
  "terrain_fill_slots",
  "terrain_slots_ptr",
];

let failed = 0;
const check = (ok, what) => {
  if (!ok) {
    failed += 1;
    console.error(`GATE FAIL: ${what}`);
  }
};

for (const name of REQUIRED) {
  check(name in ex, `export missing: ${name}`);
}
if (failed) process.exit(1);

const SEED = 0x4741544553n;

// --- terrain surface: shared worldgen reachable from JS -------------------
const h = ex.terrain_height_at(SEED, 1024, 1024);
check(Number.isFinite(h), `terrain_height_at not finite: ${h}`);
const m = ex.terrain_moisture_at(SEED, 1024, 1024);
check(Number.isFinite(m) && m >= -1.5 && m <= 1.5, `moisture odd: ${m}`);

const n = 9;
const count = ex.terrain_fill_heights(SEED, 1000, 1000, n, 8);
check(count === n * n, `fill_heights count ${count} != ${n * n}`);
const grid = new Float32Array(ex.memory.buffer, ex.terrain_heights_ptr(), n * n);
check(
  Math.abs(grid[0] - ex.terrain_height_at(SEED, 1000, 1000)) < 1e-6,
  "grid[0] disagrees with the height function",
);
let interior = false;
for (const v of grid) {
  check(Number.isFinite(v), "non-finite height in grid");
  if (v > 1.0) interior = true;
}
check(interior, "island interior produced no land above 1 m");
check(ex.terrain_fill_heights(SEED, 0, 0, 100000, 1) === 0, "oversize grid must refuse");

const slotCount = ex.terrain_fill_slots(SEED, 128, 128, 8);
check(slotCount >= 0 && slotCount <= 64, `slot count out of range: ${slotCount}`);
const slots = new Float32Array(ex.memory.buffer, ex.terrain_slots_ptr(), slotCount * 6);
for (let i = 0; i < slotCount; i++) {
  const kind = slots[i * 6];
  check(kind >= 1 && kind <= 7, `slot kind out of range: ${kind}`);
}

// --- client lifecycle: create, tick, emit an input datagram ---------------
check(ex.client_proto_ver() === 0, "proto ver drifted without this gate hearing");
const helloLen = ex.client_hello();
check(helloLen > 0 && helloLen <= 64, `hello length odd: ${helloLen}`);

ex.client_new(SEED, 257, 100);
ex.client_set_input(1, 12000, 128, 0, 127);
const steps = ex.client_advance(100.0);
check(steps >= 1 && steps <= 4, `advance(100ms) steps odd: ${steps}`);
const dgLen = ex.client_poll_input();
check(dgLen >= 12 && dgLen <= 80, `input datagram length odd: ${dgLen}`);
check(ex.client_poll_input() === 0, "second poll must have nothing due");

const remotes = ex.client_render();
check(remotes === 0, `no snapshot yet, remotes should be 0: ${remotes}`);
const render = new Float32Array(ex.memory.buffer, ex.client_render_ptr(), 14);
check(render[0] === 0, "not started before the first own snapshot");
check(render[10] >= 100, `client tick should run ahead of 100: ${render[10]}`);

// Garbage datagram must be refused, not trap.
new Uint8Array(ex.memory.buffer, ex.client_in_ptr(), 4).set([0xff, 0xff, 0xff, 0xff]);
check(ex.client_on_datagram(4) === 0, "garbage datagram must return error code");

if (failed) {
  console.error(`client bridge smoke: ${failed} failure(s)`);
  process.exit(1);
}
console.log("client bridge smoke: all checks passed");
