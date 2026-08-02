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
  "client_action_deploy",
  "client_action_feed",
  "client_action_use",
  "client_action_lock",
  "client_action_upgrade",
  "client_predict_door",
  "client_deploy_changes_ptr",
  "client_deploy_changes_len",
  "client_deploy_defs_ptr",
  "client_deploy_defs_state",
  "client_deploy_refusal_pop",
  "client_removed_key",
  "client_removed_info",
  "client_stock_ptr",
  "client_stock_count",
  "client_action_chat",
  "client_chat_pop",
  "client_chat_ptr",
  "client_health",
  "client_hit_pop",
  "client_death_pop",
  "client_death_killer",
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
check(ex.client_proto_ver() === 11, "proto ver drifted without this gate hearing");
const helloLen = ex.client_hello();
check(helloLen > 0 && helloLen <= 64, `hello length odd: ${helloLen}`);

// --- handshake parse: the welcome's dev bit reaches JS --------------------
// The canonical v9 welcome fixture, driven through the same entry the
// browser boot uses. That word is the ONLY gate on the page's dev
// affordances (`__gatesDebug.setView`), so a bridge that dropped it would
// either ship them to every public shard or withhold them from the capture
// harness — and neither shows up anywhere else in this suite.
const welcomeGolden = readFileSync(
  join(root, "crates/protocol/tests/golden/v11_welcome.bin"),
);
const parseHandshake = (bytes) => {
  // ptr first, buffer second: a getter may grow memory and detach a
  // buffer captured before it (the boot bug of 2026-07-31).
  const inPtr = ex.client_in_ptr();
  new Uint8Array(ex.memory.buffer, inPtr, bytes.length).set(bytes);
  const kind = ex.client_parse_handshake(bytes.length);
  const hsPtr = ex.client_hs_ptr();
  const hs = new Uint32Array(ex.memory.buffer, hsPtr, 7);
  return { kind, playerId: hs[1], dev: hs[6] };
};
const devOn = parseHandshake(welcomeGolden);
check(devOn.kind === 1, `welcome should parse as kind 1: ${devOn.kind}`);
check(devOn.playerId === 0x107, `welcome player id odd: ${devOn.playerId}`);
check(devOn.dev === 1, "a dev shard's welcome must reach JS as dev = 1");
// The same bytes with the dev bit (index 131, LSB-first) cleared — exactly
// what a shard with no dev override sends.
const publicWelcome = Uint8Array.from(welcomeGolden);
publicWelcome[16] &= ~0x08;
const devOff = parseHandshake(publicWelcome);
check(devOff.kind === 1, `a public shard's welcome must still parse: ${devOff.kind}`);
check(devOff.dev === 0, "a public shard's welcome must reach JS as dev = 0");

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
// kind EVENT(5, 3 bits LSB-first) · subtype SLOT_HARVESTED(2, 5 bits) ·
// cx=1 (16 bits) · cz=2 (16 bits) — protocol/src/event.rs v5 layout.
check(ex.client_cell_harvested(1, 2) === 0, "cell must start standing");
new Uint8Array(ex.memory.buffer, ex.client_in_ptr(), 5).set([0x15, 0x01, 0x00, 0x02, 0x00]);
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

// A hand-framed weak-mark event: kind EVENT(5) · subtype WEAK_MARK(6, 5 bits) ·
// cx=1 (16) · cz=2 (16) · mark8=0x40 (8) · weak_hit=1 (1 bit).
new Uint8Array(ex.memory.buffer, ex.client_in_ptr(), 7).set([
  0x35, 0x01, 0x00, 0x02, 0x00, 0x40, 0x01,
]);
const markFlags = ex.client_on_stream(7);
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
// CRAFT_DONE(8, 5 bits) · item=9 (16 bits) · added=2 (16 bits).
new Uint8Array(ex.memory.buffer, ex.client_in_ptr(), 5).set([
  0x45, 0x09, 0x00, 0x02, 0x00,
]);
const doneFlags = ex.client_on_stream(5);
check(doneFlags === 128, `craft-done should apply with CRAFT_DONE flag: ${doneFlags}`);
check(
  (ex.client_craft_pop() >>> 0) === ((9 << 16) | 2),
  "craft toast should carry item 9 × 2",
);
check(ex.client_craft_pop() >>> 0 === 0xffffffff, "craft toast ring should drain");

// Hand-framed craft-refused: kind EVENT(5) · subtype CRAFT_REFUSED(9, 5 bits) ·
// reason=4 (8 bits).
new Uint8Array(ex.memory.buffer, ex.client_in_ptr(), 2).set([0x4d, 0x04]);
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
// PIECE_PLACED(11, 5 bits) · cx=341 (10) · cz=682 (10) · level=3 (3) ·
// loc=3 (2) · row=17 (8) — protocol/src/event.rs v5 layout.
new Uint8Array(ex.memory.buffer, ex.client_in_ptr(), 6).set([
  0x5d, 0x55, 0xa9, 0xba, 0x23, 0x00,
]);
const pieceFlags = ex.client_on_stream(6);
check(pieceFlags === 1024, `piece-placed should apply with PIECES flag: ${pieceFlags}`);
check(ex.client_piece_changes_len() === 1, "one piece change expected");
const pchange = new Uint32Array(ex.memory.buffer, ex.client_piece_changes_ptr(), 2);
check(pchange[0] === ((341 << 16) | 682), `piece change key odd: ${pchange[0]}`);
check(pchange[1] === ((3 << 16) | (3 << 8) | 17), `piece change info odd: ${pchange[1]}`);
// The same record again is a duplicate, not a change.
new Uint8Array(ex.memory.buffer, ex.client_in_ptr(), 6).set([
  0x5d, 0x55, 0xa9, 0xba, 0x23, 0x00,
]);
check(ex.client_on_stream(6) === 0, "duplicate piece must not re-flag");

// Hand-framed build-refused: kind EVENT(5) · subtype BUILD_REFUSED(13, 5 bits)
// · reason=2 (8 bits).
new Uint8Array(ex.memory.buffer, ex.client_in_ptr(), 2).set([0x6d, 0x02]);
const bRefused = ex.client_on_stream(2);
check(bRefused === 4096, `build-refused should apply with its flag: ${bRefused}`);
check(ex.client_build_refusal_pop() === 2, "build refusal reason should be 2");
check(ex.client_build_refusal_pop() >>> 0 === 0xffffffff, "build refusal ring should drain");
check((ex.client_piece_defs_state() >>> 0) === 0, "no piece defs dripped yet");

// --- deploy surface: deploy/feed actions out, deploy events in ------------
const deployLen = ex.client_action_deploy(3, 341, 341, 0, 0);
check(deployLen === 5, `deploy action length odd: ${deployLen}`);
check(ex.client_action_deploy(16, 0, 0, 0, 0) === 0, "row past the table must refuse");
const feedLen = ex.client_action_feed(341, 341, 0);
check(feedLen === 4, `feed action length odd: ${feedLen}`);
check(ex.client_action_feed(1024, 0, 0) === 0, "cx past the grid must refuse");
const useLen = ex.client_action_use(341, 341, 0, 2);
check(useLen === 4, `use action length odd: ${useLen}`);
check(ex.client_action_use(341, 341, 8, 2) === 0, "level past the grid must refuse");
check(ex.client_action_use(341, 341, 0, 4) === 0, "loc past the four must refuse");
const lockLen = ex.client_action_lock(341, 341, 0, 2, 1);
check(lockLen === 4, `lock action length odd: ${lockLen}`);
check(ex.client_action_lock(1024, 341, 0, 2, 1) === 0, "cx past the grid must refuse");
check(ex.client_action_lock(341, 341, 0, 4, 0) === 0, "loc past the four must refuse");
const upgradeLen = ex.client_action_upgrade(341, 341, 0, 2, 2);
check(upgradeLen === 5, `upgrade action length odd: ${upgradeLen}`);
check(ex.client_action_upgrade(341, 341, 0, 2, 3) === 0, "a fourth material must refuse");
check(ex.client_action_upgrade(1024, 341, 0, 2, 1) === 0, "cx past the grid must refuse");

// Hand-framed deploy-placed: kind EVENT(5) · subtype DEPLOY_PLACED(15, 5
// bits) · cx=341 (10) · cz=682 (10) · level=1 (3) · loc=0 (2) · row=3 (4).
new Uint8Array(ex.memory.buffer, ex.client_in_ptr(), 5).set([
  0x7d, 0x55, 0xa9, 0x1a, 0x06,
]);
const deployFlags = ex.client_on_stream(5);
check(deployFlags === 16384, `deploy-placed should apply with its flag: ${deployFlags}`);
check(ex.client_deploy_changes_len() === 1, "one deploy change expected");
const dchange = new Uint32Array(ex.memory.buffer, ex.client_deploy_changes_ptr(), 2);
check(dchange[0] === ((341 << 16) | 682), `deploy change key odd: ${dchange[0]}`);
check(dchange[1] === ((1 << 16) | 3), `deploy change info odd: ${dchange[1]}`);

// Hand-framed door announcement for that same address: kind EVENT(5) ·
// subtype DOOR(22, 5 bits) · cx=341 (10) · cz=682 (10) · level=1 (3) ·
// loc=0 (2) · open=1 (1) · locked=1 (1). The mirror flips and re-emits
// the record with both state bits set — 24 (open) and 25 (locked) of the
// packed change word.
new Uint8Array(ex.memory.buffer, ex.client_in_ptr(), 5).set([
  0xb5, 0x55, 0xa9, 0x1a, 0x06,
]);
const doorFlags = ex.client_on_stream(5);
check(doorFlags === 16384, `door should apply with the deploy flag: ${doorFlags}`);
check(ex.client_deploy_changes_len() === 1, "one deploy change expected for the door");
const dopen = new Uint32Array(ex.memory.buffer, ex.client_deploy_changes_ptr(), 2);
check(
  dopen[1] === ((1 << 25) | (1 << 24) | (1 << 16) | 3),
  `door change info odd: ${dopen[1]}`,
);

// Optimistic toggle (NETCODE.md §6.1): row 3 has no def dripped here, so
// the client cannot know it is a door and must decline to predict —
// declining is always safe, the announcement still lands.
check(
  (ex.client_predict_door(341, 682, 1, 0) >>> 0) === 0xffffffff,
  "predicted a toggle on a row whose archetype never arrived",
);
check(
  (ex.client_predict_door(1, 1, 0, 2) >>> 0) === 0xffffffff,
  "predicted a toggle at an address holding nothing",
);

// Hand-framed piece-removed: kind EVENT(5) · subtype PIECE_REMOVED(19, 5
// bits) · the piece placed above (cx=341 · cz=682 · level=3 · loc=3).
new Uint8Array(ex.memory.buffer, ex.client_in_ptr(), 5).set([
  0x9d, 0x55, 0xa9, 0xba, 0x01,
]);
const rmFlags = ex.client_on_stream(5);
check(rmFlags === 524288, `piece-removed should apply with its flag: ${rmFlags}`);
check(ex.client_removed_key() === ((341 << 16) | 682), "removed key mismatch");
check(ex.client_removed_info() === ((3 << 8) | 3), "removed info mismatch");
// Removing it again names no known address: no flag.
new Uint8Array(ex.memory.buffer, ex.client_in_ptr(), 5).set([
  0x9d, 0x55, 0xa9, 0xba, 0x01,
]);
check(ex.client_on_stream(5) === 0, "unknown removal must not flag");

// Hand-framed deploy-refused: kind EVENT(5) · subtype DEPLOY_REFUSED(17,
// 5 bits) · reason=7 (claim).
new Uint8Array(ex.memory.buffer, ex.client_in_ptr(), 2).set([0x8d, 0x07]);
const dRefused = ex.client_on_stream(2);
check(dRefused === 65536, `deploy-refused should apply with its flag: ${dRefused}`);
check(ex.client_deploy_refusal_pop() === 7, "deploy refusal reason should be 7");
check(ex.client_deploy_refusal_pop() >>> 0 === 0xffffffff, "deploy refusal ring should drain");
check((ex.client_deploy_defs_state() >>> 0) === 0, "no deploy defs dripped yet");
check(ex.client_stock_count() === 0, "no stock ack yet");

// With a door archetype dripped and a door placed, the press predicts —
// and a refusal must roll it back all the way out to the renderer, since
// the sim's state never moved and no announcement is ever coming.
// Hand-framed deploy-defs: total=1 · first=0 · count=1 · row 0 =
// (arch DOOR 6, placement DOORWAY 3, hp 60, item 4).
new Uint8Array(ex.memory.buffer, ex.client_in_ptr(), 8).set([
  0x95, 0x01, 0x84, 0xe7, 0x01, 0x20, 0x00, 0x00,
]);
check(ex.client_on_stream(8) === 131072, "deploy defs should apply with their flag");
// Hand-framed deploy-placed: cx=100 · cz=200 · level=0 · loc=2 · row=0 ·
// open=0 · locked=1 (a door places locked, lock v0).
new Uint8Array(ex.memory.buffer, ex.client_in_ptr(), 5).set([
  0x7d, 0x64, 0x20, 0x03, 0x41,
]);
check(ex.client_on_stream(5) === 16384, "door placement should apply");
const dplaced = new Uint32Array(ex.memory.buffer, ex.client_deploy_changes_ptr(), 2);
check((dplaced[1] >>> 25) === 1, `placed door must read locked: ${dplaced[1]}`);
// The press still predicts: whether the door answers to this hand is the
// server's verdict, and a refusal rolls the leaf back below.
check(ex.client_predict_door(100, 200, 0, 2) === 1, "the press must swing your own door");
// Hand-framed deploy-refused, reason 11 (REFUSE_D_DOOR).
new Uint8Array(ex.memory.buffer, ex.client_in_ptr(), 2).set([0x8d, 0x0b]);
const rolled = ex.client_on_stream(2);
check(
  (rolled & 16384) !== 0 && (rolled & 65536) !== 0,
  `a refused use must roll back AND reach the renderer: ${rolled}`,
);
check(ex.client_deploy_refusal_pop() === 11, "refusal reason should be 11");
check(ex.client_deploy_changes_len() === 1, "the rolled-back record must ride the changes");
const drolled = new Uint32Array(ex.memory.buffer, ex.client_deploy_changes_ptr(), 2);
check(((drolled[1] >>> 24) & 1) === 0, `rolled-back door must read closed: ${drolled[1]}`);
check((drolled[1] >>> 25) === 1, `a rolled-back leaf must keep its lock: ${drolled[1]}`);

// --- chat surface: encode out of the in buffer, relay in ------------------
// The one player-authored payload, so both directions get checked here:
// the encoder must refuse what the server would, and a relayed line must
// come back through the pop view byte-exact.
const chatEnc = new TextEncoder();
const writeIn = (bytes) =>
  new Uint8Array(ex.memory.buffer, ex.client_in_ptr(), bytes.length).set(bytes);

const say = chatEnc.encode("wall's at 4 — bring stone");
writeIn(say);
const chatLen = ex.client_action_chat(say.length, 0);
check(chatLen > 0 && chatLen <= 64, `chat frame length odd: ${chatLen}`);
check((new Uint8Array(ex.memory.buffer, ex.client_out_ptr(), 1)[0] & 7) === 7,
  "a chat frame must carry KIND_CHAT (7) in its low 3 bits");

writeIn(chatEnc.encode("   "));
check(ex.client_action_chat(3, 0) === 0, "a blank line must refuse");
writeIn(chatEnc.encode("two\nlines"));
check(ex.client_action_chat(9, 0) === 0, "a control character must refuse");
writeIn(new Uint8Array(49).fill(0x61));
check(ex.client_action_chat(49, 1) === 0, "a line past the 48 B cap must refuse");
writeIn(new Uint8Array(48).fill(0x61));
check(ex.client_action_chat(48, 1) > 0, "a line exactly at the cap must encode");

check(ex.client_chat_pop() === 0, "no line has arrived yet");
// Hand-framed relay: kind EVENT(5, 3 bits LSB-first) · subtype CHAT(23,
// 5 bits) · from=7 (32 bits) · global=1 (1 bit) · len=2 (6 bits) · "hi".
{
  const bits = [];
  const push = (v, n) => { for (let i = 0; i < n; i++) bits.push((v >>> i) & 1); };
  push(5, 3);
  push(23, 5);
  push(7, 32);
  push(1, 1);
  push(2, 6);
  push(0x68, 8);
  push(0x69, 8);
  const frame = new Uint8Array(Math.ceil(bits.length / 8));
  bits.forEach((b, i) => { if (b) frame[i >> 3] |= 1 << (i & 7); });
  writeIn(frame);
  const chatFlags = ex.client_on_stream(frame.length);
  check(chatFlags === 2097152, `chat event should apply with CHAT flag: ${chatFlags}`);
  check(ex.client_chat_pop() === 1, "the relayed line must pop");
  const view = new Uint8Array(ex.memory.buffer, ex.client_chat_ptr(), 54);
  const from = (view[0] | (view[1] << 8) | (view[2] << 16) | (view[3] << 24)) >>> 0;
  check(from === 7, `speaker id mismatch: ${from}`);
  check(view[4] === 1, "the global flag must survive the relay");
  check(view[5] === 2, `length mismatch: ${view[5]}`);
  check(view[6] === 0x68 && view[7] === 0x69, "the line's bytes drifted");
  check(ex.client_chat_pop() === 0, "the ring must be empty again");
}

// The combat lane through the raw C ABI (wire v11). Hand-framed, so this
// asserts the client's decoder and not the server's encoder: a layout
// that drifted on either side shows up as a wrong value here, and
// `test_protocol_golden` holds the byte shape these frames copy.
{
  const frame = (parts) => {
    const bits = [];
    for (const [v, n] of parts) for (let i = 0; i < n; i++) bits.push((v >>> i) & 1);
    const out = new Uint8Array(Math.ceil(bits.length / 8));
    bits.forEach((b, i) => { if (b) out[i >> 3] |= 1 << (i & 7); });
    return out;
  };
  check(ex.client_health() === 0, "no health reading has arrived yet");

  // Health: kind EVENT(5) · sub 25 · hp 25 · max 100.
  let f = frame([[5, 3], [25, 5], [25, 16], [100, 16]]);
  writeIn(f);
  check(ex.client_on_stream(f.length) === 4194304, "health must apply with the HEALTH flag");
  check((ex.client_health() >>> 0) === ((25 << 16) | 100), "health readout mismatch");

  // Hit: kind EVENT(5) · sub 24 · victim 4242 · damage 25.
  check(ex.client_hit_pop() >>> 0 === 0xffffffff, "the hit ring starts empty");
  f = frame([[5, 3], [24, 5], [4242, 32], [25, 16]]);
  writeIn(f);
  check(ex.client_on_stream(f.length) === 8388608, "a hit must apply with the HIT flag");
  check((ex.client_hit_pop() >>> 0) === 25, "hitmarker damage mismatch");
  check(ex.client_hit_pop() >>> 0 === 0xffffffff, "the hit ring must drain");

  // Death: kind EVENT(5) · sub 26 · victim 4242 · killer 7.
  f = frame([[5, 3], [26, 5], [4242, 32], [7, 32]]);
  writeIn(f);
  check(ex.client_on_stream(f.length) === 16777216, "a death must apply with the DEATH flag");
  check((ex.client_death_pop() >>> 0) === 4242, "death victim mismatch");
  check((ex.client_death_killer() >>> 0) === 7, "death killer mismatch");
  check(ex.client_death_pop() >>> 0 === 0xffffffff, "the death ring must drain");

  // hp > max is a server bug, not a bar to render: refused, error bit set.
  f = frame([[5, 3], [25, 5], [101, 16], [100, 16]]);
  writeIn(f);
  check((ex.client_on_stream(f.length) & 0x80000000) !== 0, "hp past max must be refused");
  check((ex.client_health() >>> 0) === ((25 << 16) | 100), "a refused reading must not land");
}

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
