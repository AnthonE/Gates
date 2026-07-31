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
  "client_on_stream",
  "client_slot_changes_ptr",
  "client_slot_changes_len",
  "client_inv_ptr",
  "client_catalog_ptr",
  "client_toast_pop",
  "client_cell_harvested",
  "client_craft_jobs_ptr",
  "client_craft_q",
  "client_recipes_ptr",
  "client_recipes_state",
  "client_craft_pop",
  "client_craft_refusal_pop",
  "client_action_craft",
  "client_action_cancel",
  "client_action_place",
  "client_piece_changes_ptr",
  "client_piece_changes_len",
  "client_piece_defs_ptr",
  "client_piece_defs_state",
  "client_build_refusal_pop",
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
const slots = new Float32Array(ex.memory.buffer, ex.terrain_slots_ptr(), slotCount * 8);
for (let i = 0; i < slotCount; i++) {
  const kind = slots[i * 8];
  check(kind >= 1 && kind <= 7, `slot kind out of range: ${kind}`);
  const cx = slots[i * 8 + 6];
  const cz = slots[i * 8 + 7];
  check(cx >= 128 && cx < 136 && cz >= 128 && cz < 136, `slot cell out of block: ${cx},${cz}`);
}

// --- client lifecycle: create, tick, emit an input datagram ---------------
check(ex.client_proto_ver() === 4, "proto ver drifted without this gate hearing");
const helloLen = ex.client_hello();
check(helloLen > 0 && helloLen <= 64, `hello length odd: ${helloLen}`);

ex.client_new(SEED, 257, 100);
ex.client_set_input(1, 12000, 128, 0, 127, 3);
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

// --- event lane: a hand-framed harvest event through the stream path ------
// kind EVENT(5, 3 bits LSB-first) · subtype SLOT_HARVESTED(2, 4 bits) ·
// cx=1 (16 bits) · cz=2 (16 bits) — protocol/src/event.rs layout.
check(ex.client_cell_harvested(1, 2) === 0, "cell must start standing");
new Uint8Array(ex.memory.buffer, ex.client_in_ptr(), 5).set([0x95, 0x00, 0x00, 0x01, 0x00]);
const evFlags = ex.client_on_stream(5);
check(evFlags === 2, `harvest event should apply with SLOTS flag: ${evFlags}`);
check(ex.client_slot_changes_len() === 1, "one cell change expected");
const change = new Uint32Array(ex.memory.buffer, ex.client_slot_changes_ptr(), 2);
check(change[0] === ((1 << 16) | 2), `change key odd: ${change[0]}`);
check(change[1] === 1, "change must say harvested");
check(ex.client_cell_harvested(1, 2) === 1, "cell must read harvested now");
// wasm i32 returns read signed in JS; >>> 0 recovers the u32 sentinel.
check(ex.client_toast_pop() >>> 0 === 0xffffffff, "no toast should be buffered");
check(ex.client_weak_mark_cell() >>> 0 === 0xffffffff, "no weak mark should be up");

// A hand-framed weak-mark event: kind EVENT(5) · subtype WEAK_MARK(6) ·
// cx=1 (16) · cz=2 (16) · mark8=0x40 (8) · weak_hit=1 (1 bit).
new Uint8Array(ex.memory.buffer, ex.client_in_ptr(), 6).set([
  0xb5, 0x00, 0x00, 0x01, 0x00, 0xa0,
]);
const markFlags = ex.client_on_stream(6);
check(markFlags === 32, `weak-mark event should apply with MARK flag: ${markFlags}`);
check(ex.client_weak_mark_cell() >>> 0 === ((1 << 16) | 2), "mark cell mismatch");
check(ex.client_weak_mark_info() === ((1 << 8) | 0x40), "mark info mismatch");
const invView = new Uint16Array(ex.memory.buffer, ex.client_inv_ptr(), 60);
check(invView.every((v) => v === 0), "inventory view should start empty");

// --- craft surface: action encode out, craft events in --------------------
const craftLen = ex.client_action_craft(3, 2);
check(craftLen > 0 && craftLen <= 4, `craft action length odd: ${craftLen}`);
check(ex.client_action_craft(64, 1) === 0, "recipe past the table must refuse");
check(ex.client_action_craft(0, 0) === 0, "zero count must refuse");
const cancelLen = ex.client_action_cancel(1);
check(cancelLen > 0 && cancelLen <= 2, `cancel action length odd: ${cancelLen}`);
check(ex.client_action_cancel(4) === 0, "index past the queue must refuse");

// Hand-framed craft-done: kind EVENT(5, 3 bits LSB-first) · subtype
// CRAFT_DONE(8, 4 bits) · item=9 (16 bits) · added=2 (16 bits).
new Uint8Array(ex.memory.buffer, ex.client_in_ptr(), 5).set([
  0xc5, 0x04, 0x00, 0x01, 0x00,
]);
const doneFlags = ex.client_on_stream(5);
check(doneFlags === 128, `craft-done should apply with CRAFT_DONE flag: ${doneFlags}`);
check(
  (ex.client_craft_pop() >>> 0) === ((9 << 16) | 2),
  "craft toast should carry item 9 × 2",
);
check(ex.client_craft_pop() >>> 0 === 0xffffffff, "craft toast ring should drain");

// Hand-framed craft-refused: kind EVENT(5) · subtype CRAFT_REFUSED(9) ·
// reason=4 (8 bits).
new Uint8Array(ex.memory.buffer, ex.client_in_ptr(), 2).set([0x4d, 0x02]);
const refusedFlags = ex.client_on_stream(2);
check(
  refusedFlags === 256,
  `craft-refused should apply with CRAFT_REFUSED flag: ${refusedFlags}`,
);
check(ex.client_craft_refusal_pop() === 4, "refusal reason should be 4");
check(ex.client_craft_refusal_pop() >>> 0 === 0xffffffff, "refusal ring should drain");
check((ex.client_craft_q() >>> 0) === 0, "queue should still be empty");
check((ex.client_recipes_state() >>> 0) === 0, "no recipes dripped yet");

// --- build surface: place action out, piece events in ---------------------
const placeLen = ex.client_action_place(0, 341, 341, 0, 0);
check(placeLen === 5, `place action length odd: ${placeLen}`);
check(ex.client_action_place(32, 0, 0, 0, 0) === 0, "row past the table must refuse");
check(ex.client_action_place(0, 1024, 0, 0, 0) === 0, "cx past the grid must refuse");
check(ex.client_action_place(0, 0, 0, 8, 0) === 0, "level past the cap must refuse");

// Hand-framed piece-placed: kind EVENT(5, 3 bits LSB-first) · subtype
// PIECE_PLACED(11, 4 bits) · cx=341 (10) · cz=682 (10) · level=3 (3) ·
// loc=3 (2) · row=17 (8) — protocol/src/event.rs layout.
new Uint8Array(ex.memory.buffer, ex.client_in_ptr(), 5).set([
  0xdd, 0xaa, 0x54, 0xdd, 0x11,
]);
const pieceFlags = ex.client_on_stream(5);
check(pieceFlags === 1024, `piece-placed should apply with PIECES flag: ${pieceFlags}`);
check(ex.client_piece_changes_len() === 1, "one piece change expected");
const pchange = new Uint32Array(ex.memory.buffer, ex.client_piece_changes_ptr(), 2);
check(pchange[0] === ((341 << 16) | 682), `piece change key odd: ${pchange[0]}`);
check(pchange[1] === ((3 << 16) | (3 << 8) | 17), `piece change info odd: ${pchange[1]}`);
// The same record again is a duplicate, not a change.
new Uint8Array(ex.memory.buffer, ex.client_in_ptr(), 5).set([
  0xdd, 0xaa, 0x54, 0xdd, 0x11,
]);
check(ex.client_on_stream(5) === 0, "duplicate piece must not re-flag");

// Hand-framed build-refused: kind EVENT(5) · subtype BUILD_REFUSED(13) ·
// reason=2 (8 bits).
new Uint8Array(ex.memory.buffer, ex.client_in_ptr(), 2).set([0x6d, 0x01]);
const bRefused = ex.client_on_stream(2);
check(bRefused === 4096, `build-refused should apply with its flag: ${bRefused}`);
check(ex.client_build_refusal_pop() === 2, "build refusal reason should be 2");
check(ex.client_build_refusal_pop() >>> 0 === 0xffffffff, "build refusal ring should drain");
check((ex.client_piece_defs_state() >>> 0) === 0, "no piece defs dripped yet");

// Garbage on the stream must be refused with the error bit, not trap.
new Uint8Array(ex.memory.buffer, ex.client_in_ptr(), 3).set([0xff, 0xff, 0xff]);
check((ex.client_on_stream(3) & 0x80000000) !== 0, "garbage stream msg must error");

// Garbage datagram must be refused, not trap.
new Uint8Array(ex.memory.buffer, ex.client_in_ptr(), 4).set([0xff, 0xff, 0xff, 0xff]);
check(ex.client_on_datagram(4) === 0, "garbage datagram must return error code");

if (failed) {
  console.error(`client bridge smoke: ${failed} failure(s)`);
  process.exit(1);
}
console.log("client bridge smoke: all checks passed");
