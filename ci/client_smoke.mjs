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
  "client_action_drink",
  "client_drank",
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
  "client_vitals",
  "client_vitals_max",
  "client_consume",
  "client_action_consume",
  "client_hit_pop",
  "client_death_pop",
  "client_death_killer",
  "client_death_screen",
  "client_death_by",
  "client_death_weapon",
  "client_action_respawn",
  "client_action_move",
  "client_applied2",
  "client_move_readout",
  "client_move_payload",
  "client_bag_ids_ptr",
  "client_bags_ptr",
  "client_bags_len",
  "client_action_loot",
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
check(ex.client_proto_ver() === 18, "proto ver drifted without this gate hearing");

// Every hand-framed S->C event below is built here, from the field widths
// `protocol/src/event.rs` declares — never from a byte literal. Wire v13
// widened the subtype field 5 -> 6 bits, which moved every one of these by
// a bit; a literal would have made that a nineteen-line hand edit with no
// way to tell a typo from a real drift. `test_protocol_golden` still owns
// the byte shape; this owns the client's decoder reading it.
const KIND_EVENT = 5;
const KIND_BITS = 3;
const EV_SUB_BITS = 6;
const bitsToBytes = (bits) => {
  const out = new Uint8Array(Math.ceil(bits.length / 8));
  bits.forEach((b, i) => { if (b) out[i >> 3] |= 1 << (i & 7); });
  return out;
};
/// LSB-first bit packing, the writer's own order (protocol/src/bits.rs).
const packed = (parts) => {
  const bits = [];
  for (const [v, n] of parts) for (let i = 0; i < n; i++) bits.push((v >>> i) & 1);
  return bitsToBytes(bits);
};
const evFrame = (sub, parts) =>
  packed([[KIND_EVENT, KIND_BITS], [sub, EV_SUB_BITS], ...parts]);
/// Write a frame into the client's inbox and hand it to the stream path.
const onEvent = (f) => {
  new Uint8Array(ex.memory.buffer, ex.client_in_ptr(), f.length).set(f);
  return ex.client_on_stream(f.length);
};
const helloLen = ex.client_hello();
check(helloLen > 0 && helloLen <= 64, `hello length odd: ${helloLen}`);

// --- handshake parse: the welcome's dev bit reaches JS --------------------
// The canonical v9 welcome fixture, driven through the same entry the
// browser boot uses. That word is the ONLY gate on the page's dev
// affordances (`__gatesDebug.setView`), so a bridge that dropped it would
// either ship them to every public shard or withhold them from the capture
// harness — and neither shows up anywhere else in this suite.
const welcomeGolden = readFileSync(
  // Keyed off the version the artifact reports, not off a literal. The
  // fixtures are renamed wholesale on every bump, so a literal here goes
  // stale in exactly the commit that is least able to notice — and the
  // pin above already asserts which version that is, so deriving the path
  // from it cannot silently read the wrong fixture.
  join(root, `crates/protocol/tests/golden/v${ex.client_proto_ver()}_welcome.bin`),
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
// subtype SLOT_HARVESTED(2) · cx=1 (16) · cz=2 (16).
check(ex.client_cell_harvested(1, 2) === 0, "cell must start standing");
const evFlags = onEvent(evFrame(2, [[1, 16], [2, 16]]));
check(evFlags === 2, `harvest event should apply with SLOTS flag: ${evFlags}`);
check(ex.client_slot_changes_len() === 1, "one cell change expected");
const change = new Uint32Array(ex.memory.buffer, ex.client_slot_changes_ptr(), 2);
check(change[0] === ((1 << 16) | 2), `change key odd: ${change[0]}`);
check(change[1] === 1, "change must say harvested");
check(ex.client_cell_harvested(1, 2) === 1, "cell must read harvested now");
// wasm i32 returns read signed in JS; >>> 0 recovers the u32 sentinel.
check(ex.client_toast_pop() >>> 0 === 0xffffffff, "no toast should be buffered");
check(ex.client_weak_mark_cell() >>> 0 === 0xffffffff, "no weak mark should be up");

// subtype WEAK_MARK(6) · cx=1 (16) · cz=2 (16) · mark8=0x40 (8) ·
// weak_hit=1 (1).
const markFlags = onEvent(evFrame(6, [[1, 16], [2, 16], [0x40, 8], [1, 1]]));
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

// subtype CRAFT_DONE(8) · item=9 (16) · added=2 (16).
const doneFlags = onEvent(evFrame(8, [[9, 16], [2, 16]]));
check(doneFlags === 128, `craft-done should apply with CRAFT_DONE flag: ${doneFlags}`);
check(
  (ex.client_craft_pop() >>> 0) === ((9 << 16) | 2),
  "craft toast should carry item 9 × 2",
);
check(ex.client_craft_pop() >>> 0 === 0xffffffff, "craft toast ring should drain");

// subtype CRAFT_REFUSED(9) · reason=4 (8).
const refusedFlags = onEvent(evFrame(9, [[4, 8]]));
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

// subtype PIECE_PLACED(11) · cx=341 (10) · cz=682 (10) · level=3 (3) ·
// loc=3 (2) · row=17 (8).
const placedPiece = evFrame(11, [[341, 10], [682, 10], [3, 3], [3, 2], [17, 8]]);
const pieceFlags = onEvent(placedPiece);
check(pieceFlags === 1024, `piece-placed should apply with PIECES flag: ${pieceFlags}`);
check(ex.client_piece_changes_len() === 1, "one piece change expected");
const pchange = new Uint32Array(ex.memory.buffer, ex.client_piece_changes_ptr(), 2);
check(pchange[0] === ((341 << 16) | 682), `piece change key odd: ${pchange[0]}`);
check(pchange[1] === ((3 << 16) | (3 << 8) | 17), `piece change info odd: ${pchange[1]}`);
// The same record again is a duplicate, not a change.
check(onEvent(placedPiece) === 0, "duplicate piece must not re-flag");

// subtype BUILD_REFUSED(13) · reason=2 (8).
const bRefused = onEvent(evFrame(13, [[2, 8]]));
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
// 5 B since wire v12: the action subtype field widened 3 → 4 bits for
// the loot action, and the lock frame was exactly on a byte boundary.
check(lockLen === 5, `lock action length odd: ${lockLen}`);
check(ex.client_action_lock(1024, 341, 0, 2, 1) === 0, "cx past the grid must refuse");
check(ex.client_action_lock(341, 341, 0, 4, 0) === 0, "loc past the four must refuse");
const upgradeLen = ex.client_action_upgrade(341, 341, 0, 2, 2);
check(upgradeLen === 5, `upgrade action length odd: ${upgradeLen}`);
check(ex.client_action_upgrade(341, 341, 0, 2, 3) === 0, "a fourth material must refuse");
check(ex.client_action_upgrade(1024, 341, 0, 2, 1) === 0, "cx past the grid must refuse");

// subtype DEPLOY_PLACED(15) · cx=341 (10) · cz=682 (10) · level=1 (3) ·
// loc=0 (2) · row=3 (4) · open=0 (1) · locked=0 (1).
const deployFlags = onEvent(
  evFrame(15, [[341, 10], [682, 10], [1, 3], [0, 2], [3, 4], [0, 1], [0, 1]]),
);
check(deployFlags === 16384, `deploy-placed should apply with its flag: ${deployFlags}`);
check(ex.client_deploy_changes_len() === 1, "one deploy change expected");
const dchange = new Uint32Array(ex.memory.buffer, ex.client_deploy_changes_ptr(), 2);
check(dchange[0] === ((341 << 16) | 682), `deploy change key odd: ${dchange[0]}`);
check(dchange[1] === ((1 << 16) | 3), `deploy change info odd: ${dchange[1]}`);

// The door announcement for that same address: subtype DOOR(22) ·
// cx=341 (10) · cz=682 (10) · level=1 (3) · loc=0 (2) · open=1 (1) ·
// locked=1 (1). The mirror flips and re-emits the record with both state
// bits set — 24 (open) and 25 (locked) of the packed change word.
const doorFlags = onEvent(
  evFrame(22, [[341, 10], [682, 10], [1, 3], [0, 2], [1, 1], [1, 1]]),
);
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

// subtype PIECE_REMOVED(19) · the piece placed above (cx=341 · cz=682 ·
// level=3 · loc=3).
const removedPiece = evFrame(19, [[341, 10], [682, 10], [3, 3], [3, 2]]);
const rmFlags = onEvent(removedPiece);
check(rmFlags === 524288, `piece-removed should apply with its flag: ${rmFlags}`);
check(ex.client_removed_key() === ((341 << 16) | 682), "removed key mismatch");
check(ex.client_removed_info() === ((3 << 8) | 3), "removed info mismatch");
// Removing it again names no known address: no flag.
check(onEvent(removedPiece) === 0, "unknown removal must not flag");

// subtype DEPLOY_REFUSED(17) · reason=7 (claim).
const dRefused = onEvent(evFrame(17, [[7, 8]]));
check(dRefused === 65536, `deploy-refused should apply with its flag: ${dRefused}`);
check(ex.client_deploy_refusal_pop() === 7, "deploy refusal reason should be 7");
check(ex.client_deploy_refusal_pop() >>> 0 === 0xffffffff, "deploy refusal ring should drain");
check((ex.client_deploy_defs_state() >>> 0) === 0, "no deploy defs dripped yet");
check(ex.client_stock_count() === 0, "no stock ack yet");

// With a door archetype dripped and a door placed, the press predicts —
// and a refusal must roll it back all the way out to the renderer, since
// the sim's state never moved and no announcement is ever coming.
// subtype DEPLOY_DEFS(18) · total=1 (5) · first=0 (5) · count=1 (4) ·
// row 0 = (arch DOOR 6 (3), placement DOORWAY 3 (2), hp 60 (16),
// item 4 (16)).
check(
  onEvent(evFrame(18, [[1, 5], [0, 5], [1, 4], [6, 3], [3, 2], [60, 16], [4, 16]])) ===
    131072,
  "deploy defs should apply with their flag",
);
// subtype DEPLOY_PLACED(15) · cx=100 · cz=200 · level=0 · loc=2 · row=0 ·
// open=0 · locked=1 (a door places locked, lock v0).
check(
  onEvent(evFrame(15, [[100, 10], [200, 10], [0, 3], [2, 2], [0, 4], [0, 1], [1, 1]])) ===
    16384,
  "door placement should apply",
);
const dplaced = new Uint32Array(ex.memory.buffer, ex.client_deploy_changes_ptr(), 2);
check((dplaced[1] >>> 25) === 1, `placed door must read locked: ${dplaced[1]}`);
// The press still predicts: whether the door answers to this hand is the
// server's verdict, and a refusal rolls the leaf back below.
check(ex.client_predict_door(100, 200, 0, 2) === 1, "the press must swing your own door");
// subtype DEPLOY_REFUSED(17) · reason 11 (REFUSE_D_DOOR).
const rolled = onEvent(evFrame(17, [[11, 8]]));
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
{
  // The relay: subtype CHAT(23) · from=7 (32) · global=1 (1) ·
  // len=2 (6) · "hi".
  const chatFlags = onEvent(
    evFrame(23, [[7, 32], [1, 1], [2, 6], [0x68, 8], [0x69, 8]]),
  );
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

// The combat, backpack and raid lanes through the raw C ABI (wire v13).
// Hand-framed, so this asserts the client's decoder and not the server's
// encoder: a layout that drifted on either side shows up as a wrong value
// here, and `test_protocol_golden` holds the byte shape these frames copy.
{
  check(ex.client_health() === 0, "no health reading has arrived yet");

  // Health: sub 25 · hp 25 · max 100.
  let f = evFrame(25, [[25, 16], [100, 16]]);
  writeIn(f);
  check(ex.client_on_stream(f.length) === 4194304, "health must apply with the HEALTH flag");
  check((ex.client_health() >>> 0) === ((25 << 16) | 100), "health readout mismatch");

  // The survival clock's three (wire v14). Vitals: sub 31 · food 62 ·
  // water 38 · max 100/100 — the same four distinct numbers the golden
  // fixture carries, so a transposed pair cannot pass either gate.
  check(ex.client_vitals_max() === 0, "no meter reading has arrived yet");
  f = evFrame(31, [[62, 16], [38, 16], [100, 16], [100, 16]]);
  writeIn(f);
  check(ex.client_on_stream(f.length) === 134217728, "vitals must apply with the VITALS flag");
  check((ex.client_vitals() >>> 0) === ((62 << 16) | 38), "meter readout mismatch");
  check(
    (ex.client_vitals_max() >>> 0) === ((100 << 16) | 100),
    "meter ceilings mismatch",
  );

  // Consumed: sub 32 · item 11 · slot 3 (5-bit slot field).
  f = evFrame(32, [[11, 16], [3, 5]]);
  writeIn(f);
  check(ex.client_on_stream(f.length) === 268435456, "an eat must apply with the CONSUME flag");
  check((ex.client_consume() >>> 0) === ((3 << 16) | 11), "eat readout mismatch");

  // ConsumeRefused: sub 33 · reason 2 (REFUSE_C_FULL), 4-bit field. The
  // refusal must be distinguishable from the landed eat above, which is
  // the whole reason the reason rides the high byte.
  f = evFrame(33, [[2, 4]]);
  writeIn(f);
  check(ex.client_on_stream(f.length) === 268435456, "a refusal must apply with the CONSUME flag");
  check((ex.client_consume() >>> 0) >>> 24 === 2, "refusal reason mismatch");

  // And the eat verb crosses C->S: a real slot encodes, a forged one does
  // not — the width IS the range check (protocol/src/lib.rs).
  check(ex.client_action_consume(3) > 0, "the eat verb must encode a real slot");
  check(ex.client_action_consume(30) === 0, "a slot past INV_SLOTS must not encode");

  // Drank: sub 34 · water 25 · hp cost 2 (wire v15). The pair is what the
  // subtype exists for — the HUD names the cost, and `Health` alone
  // could not (survival.rs).
  const DRANK = 536870912; // APPLIED_DRANK = 1 << 29
  check(ex.client_drank() >>> 0 === 0, "no drink has landed yet");
  f = evFrame(34, [[25, 16], [2, 16]]);
  writeIn(f);
  check(ex.client_on_stream(f.length) === DRANK, "a drink must apply with the DRANK flag");
  check((ex.client_drank() >>> 0) === ((25 << 16) | 2), "drink readout mismatch");
  // A drink that restored nothing and cost nothing is not a drink — the
  // decoder refuses the all-zero pair rather than reporting a no-op, the
  // same posture as a refusal with reason 0.
  f = evFrame(34, [[0, 16], [0, 16]]);
  writeIn(f);
  check(
    (ex.client_on_stream(f.length) & 0x80000000) !== 0,
    "an empty drink must be refused, not reported as a no-op",
  );
  check((ex.client_drank() >>> 0) === ((25 << 16) | 2), "the refused frame must not overwrite");

  // A refused drink rides the eat readout — one refusal channel for the
  // whole survival module. Reason 3 is REFUSE_C_NO_WATER.
  f = evFrame(33, [[3, 4]]);
  writeIn(f);
  check(ex.client_on_stream(f.length) === 268435456, "a dry press must apply with the CONSUME flag");
  check((ex.client_consume() >>> 0) >>> 24 === 3, "no-water refusal reason mismatch");

  // And the drink verb crosses C->S. Payload-free, so there is no forged
  // variant to test — the absence of a payload IS the range check.
  check(ex.client_action_drink() > 0, "the drink verb must encode");

  // The move verb, both directions (wire v17). The move verdict is the
  // one applied-flag that is NOT in the word `client_on_stream` returns:
  // bits 0..30 of that word are spent and bit 31 is the bridge's error
  // sentinel, so it rides word 1 and is read through `client_applied2`.
  //
  // That is the whole point of these two checks. `APPLIED_MOVE` was
  // `1 << 31` — the sentinel's own value — so the first move of a session
  // reached the client as a decode error, which logs and returns early,
  // taking the rest of that pump iteration with it. Asserting bit 31 is
  // CLEAR on a verdict is the C-ABI half of the ledger test in `core.rs`:
  // that one proves no constant can reach the bit, this one proves the
  // browser's actual calling path does not hand one to JS.
  const MOVE2 = 1; // core::APPLIED2_MOVE, word 1 bit 0
  const STREAM_ERR = 2147483648; // 1 << 31, and nothing else may be it

  // Moved: sub 36 · from (kind 1, slot 9) · to (kind 0, slot 22) · count 7
  // · item 5. Every part distinct from every other, so a transposed pair
  // cannot pass — the same discipline the goldens and `event_roles` use,
  // and for the same reason: this payload is two (kind, slot) pairs, which
  // is the exact shape ~27 of the reference's corrections landed on.
  f = evFrame(36, [[1, 2], [9, 5], [0, 2], [22, 5], [7, 16], [5, 16]]);
  writeIn(f);
  let mflags = ex.client_on_stream(f.length) >>> 0;
  check((mflags & STREAM_ERR) === 0, "a move must not read as a stream error");
  check(ex.client_applied2() === MOVE2, "a move must apply with the MOVE flag in word 1");
  check(
    (ex.client_move_readout() >>> 0) === ((22 << 16) | (1 << 8) | 9),
    "move readout mismatch: reason 0, to slot 22, from kind 1, from slot 9",
  );
  check(
    (ex.client_move_payload() >>> 0) === ((7 << 16) | 5),
    "move payload mismatch: count 7 of item 5",
  );

  // MoveRefused: sub 37 · reason 4 (REFUSE_M_NO_ROOM, 3-bit field) · the
  // address it was asked for. The reason must be distinguishable from the
  // landed move above, which is why it rides the high byte.
  f = evFrame(37, [[4, 3], [0, 2], [11, 5], [1, 2], [26, 5]]);
  writeIn(f);
  mflags = ex.client_on_stream(f.length) >>> 0;
  check((mflags & STREAM_ERR) === 0, "a refusal must not read as a stream error");
  check(ex.client_applied2() === MOVE2, "a refusal must apply with the MOVE flag in word 1");
  check((ex.client_move_readout() >>> 0) >>> 24 === 4, "move refusal reason mismatch");
  check(
    (ex.client_move_readout() >>> 0 & 0xffff) === 11,
    "a refusal must still carry the from slot, or the panel cannot roll back",
  );
  check(ex.client_move_payload() === 0, "a refusal moved nothing");

  // Word 1 describes ONE message. JS has to read it unconditionally —
  // word 0 has no spare bit to announce it with — so a verdict that
  // outlived its message would be read as the answer to a drag the server
  // has not answered yet, and the panel would roll back a live move.
  f = evFrame(33, [[3, 4]]); // a consume, which sets nothing in word 1
  writeIn(f);
  ex.client_on_stream(f.length);
  check(ex.client_applied2() === 0, "a stale move verdict outlived its message");

  // And the move verb crosses C->S. A real drag encodes; a forged kind, a
  // slot past INV_SLOTS and a zero count do not — the bridge refuses them
  // locally rather than handing the server a frame it answers by ending
  // the session, which is precisely how this verb failed in the reference.
  check(ex.client_action_move(0, 0, 3, 0, 7, 5) > 0, "a real move must encode");
  check(ex.client_action_move(9001, 1, 0, 0, 4, 1) > 0, "a bag move must encode");
  // Wire v18 made kind 2 the deployed box, so the boundary moved by one
  // and both sides of it are checked: a box move encodes, and the first
  // forgeable kind above it still does not. `CONT_KIND_BITS` is 2, so 3 is
  // the last value that fits in the field and there is nothing past it to
  // test — a fourth kind would have to widen the field.
  check(
    ex.client_action_move(0x0155_d450, 2, 3, 0, 7, 5) > 0,
    "a box move must encode (CONT_BOX, wire v18)",
  );
  check(
    ex.client_action_move(0x0155_d450, 0, 3, 2, 11, 5) > 0,
    "a box move must encode in the deposit direction too",
  );
  check(ex.client_action_move(0, 3, 3, 0, 7, 5) === 0, "a kind past CONT_MAX must not encode");
  check(ex.client_action_move(0, 0, 30, 0, 7, 5) === 0, "a slot past INV_SLOTS must not encode");
  check(ex.client_action_move(0, 0, 3, 0, 7, 0) === 0, "a zero count is not a move");

  // --- and the BYTES, field by field ---------------------------------------
  // The five checks above are the whole of what pinned this call until now,
  // and every one of them asks the same question: did a frame come out. So
  // `client_action_move(0, 0, 7, 0, 3, 5)` is exactly as green as
  // `(0, 0, 3, 0, 7, 5)` — a `from`/`to` transposition, or a kind swapped
  // with the slot beside it, produces a valid frame of the same length and
  // no gate in this repo looks at it. That is CLAUDE.md's positional-payload
  // trap on the client's own outbound edge: ~27 of the reference ecosystem's
  // shipped corrections were the right value in the wrong position, four
  // hooks corrected more than once, and their per-method hash — the exact
  // analogue of `test_protocol_golden` — caught none of them, because the
  // encoder is untouched when the CALLER swaps two arguments.
  //
  // `test_protocol_golden` owns the byte shape; this owns which value the
  // browser's calling path puts in each of those bytes. The widths come out
  // of `protocol/src/lib.rs` rather than being restated, for the reason the
  // event frames above already give: wire v13 moved every field by a bit,
  // and a literal cannot tell a typo from a real drift.
  const protoSrc = readFileSync(join(root, "crates/protocol/src/lib.rs"), "utf8");
  const protoConst = (name) => {
    const m = protoSrc.match(new RegExp(`const ${name}: u32 = (\\d+);`));
    const v = Number(m?.[1]);
    check(
      Number.isInteger(v),
      `could not read ${name} out of protocol/src/lib.rs — this decode would then be checked against` +
        " nothing, which is the gate-that-matches-nothing class",
    );
    return v;
  };
  const fieldBits = {
    kind: protoConst("KIND_BITS"),
    sub: protoConst("ACTION_SUB_BITS"),
    contKind: protoConst("CONT_KIND_BITS"),
    slot: protoConst("ACTION_SLOT_BITS"),
    count: protoConst("MOVE_COUNT_BITS"),
  };
  // Lower-camel deliberately: `ci/knob_registry.mjs` pins every SHOUTY
  // constant it can see against `DECISIONS.md`, and these are read out of the
  // Rust at run time rather than declared here, so a registry that tried to
  // parse them would be pinning a function call.
  const kindAction = protoConst("KIND_ACTION");
  const actMove = protoConst("ACT_MOVE");
  /// LSB-first, the mirror of `packed` above and of `protocol/src/bits.rs`.
  const unpack = (buf, widths) => {
    let bit = 0;
    return widths.map((n) => {
      let v = 0;
      for (let i = 0; i < n; i++, bit++)
        if ((buf[bit >> 3] >> (bit & 7)) & 1) v += 2 ** i;
      return v;
    });
  };
  // Every field a distinct value, and distinct from every other field's, so
  // a transposition cannot pass by coincidence: bag 9001, from (kind 1, slot
  // 6), to (kind 0, slot 21), count 13. The two kinds differ, the two slots
  // differ, and no slot equals a kind.
  const mlen = ex.client_action_move(9001, 1, 6, 0, 21, 13);
  check(mlen > 0, "the field-by-field move must encode at all");
  const mbuf = new Uint8Array(ex.memory.buffer, ex.client_out_ptr(), mlen).slice();
  const [mKind, mSub, mBag, mFromKind, mFromSlot, mToKind, mToSlot, mCount] = unpack(mbuf, [
    fieldBits.kind, fieldBits.sub, 32, fieldBits.contKind, fieldBits.slot, fieldBits.contKind, fieldBits.slot, fieldBits.count,
  ]);
  check(
    mKind === kindAction && mSub === actMove,
    `the move frame is not an ACT_MOVE action (kind ${mKind}, sub ${mSub}) — the decode below would then be` +
      " reading some other message's fields and calling them a move",
  );
  check(mBag === 9001, `move bag encoded as ${mBag}, not 9001 — the sim addresses the container by this id`);
  check(
    mFromKind === 1 && mFromSlot === 6,
    `the move's FROM encoded as (kind ${mFromKind}, slot ${mFromSlot}), not (1, 6) — a kind and the slot beside` +
      " it are both small integers in adjacent fields, and swapping them is a valid frame the server acts on",
  );
  check(
    mToKind === 0 && mToSlot === 21,
    `the move's TO encoded as (kind ${mToKind}, slot ${mToSlot}), not (0, 21)`,
  );
  check(
    mCount === 13,
    `move count encoded as ${mCount}, not 13 — the count is what the server sims on, so a wrong one here is the` +
      " quantize-both-sides law broken at its own call site",
  );
  // The transposition itself, driven rather than reasoned about: the same
  // call with `from` and `to` exchanged must produce a DIFFERENT frame. If
  // these compare equal the encoder is symmetric in the pair and every check
  // above is satisfied by a client that has them the wrong way round — which
  // is the single shape the reference corrected most often.
  const tlen = ex.client_action_move(9001, 0, 21, 1, 6, 13);
  const tbuf = new Uint8Array(ex.memory.buffer, ex.client_out_ptr(), tlen).slice();
  check(
    tlen === mlen && !mbuf.every((b, i) => b === tbuf[i]),
    "exchanging the move's from and to produced a byte-identical frame — the two ends are the same position on" +
      " the wire, so nothing downstream can tell a move from its reverse",
  );

  // Hit: sub 24 · victim 4242 · damage 25.
  check(ex.client_hit_pop() >>> 0 === 0xffffffff, "the hit ring starts empty");
  f = evFrame(24, [[4242, 32], [25, 16]]);
  writeIn(f);
  check(ex.client_on_stream(f.length) === 8388608, "a hit must apply with the HIT flag");
  check((ex.client_hit_pop() >>> 0) === 25, "hitmarker damage mismatch");
  check(ex.client_hit_pop() >>> 0 === 0xffffffff, "the hit ring must drain");

  // Death: sub 26 · victim 4242 · killer 7 · cause 1 · no weapon · no range.
  f = evFrame(26, [[4242, 32], [7, 32], [1, 2], [0xffff, 16], [0, 16]]);
  writeIn(f);
  check(ex.client_on_stream(f.length) === 16777216, "a death must apply with the DEATH flag");
  check((ex.client_death_pop() >>> 0) === 4242, "death victim mismatch");
  check((ex.client_death_killer() >>> 0) === 7, "death killer mismatch");
  check(ex.client_death_pop() >>> 0 === 0xffffffff, "the death ring must drain");

  // The death backpack, hand-framed end to end. Positions are
  // the same quanta an entity's are: x/z 17 bits unsigned, y 14 bits
  // biased by POS_Y_BIAS = 2048. 34133 quanta x = 1024.0 m at the 3 cm
  // x/z quantum, which is the smoke shard's own spawn point.
  const BAGS = 33554432; // APPLIED_BAGS = 1 << 25
  check(ex.client_bags_len() === 0, "no bag has been announced yet");

  // Bag dropped: sub 27 · id 9001 · qx · qy+bias · qz.
  f = evFrame(27, [[9001, 32], [34133, 17], [512 + 2048, 14], [22050, 17]]);
  writeIn(f);
  check(ex.client_on_stream(f.length) === BAGS, "a dropped bag must apply with the BAGS flag");
  check(ex.client_bags_len() === 1, "the bag set must hold it");
  {
    const ids = new Uint32Array(ex.memory.buffer, ex.client_bag_ids_ptr(), 1);
    const pos = new Float32Array(ex.memory.buffer, ex.client_bags_ptr(), 3);
    check(ids[0] === 9001, "bag id mismatch");
    check(Math.abs(pos[0] - 34133 * 0.03) < 1e-3, "bag x mismatch");
    check(Math.abs(pos[1] - 512 * 0.01) < 1e-3, "bag y mismatch");
    check(Math.abs(pos[2] - 22050 * 0.03) < 1e-3, "bag z mismatch");
  }

  // The same bag again must be a no-op, not a second mesh: a bag never
  // moves, so identity is the id and a repeat carries nothing new.
  writeIn(f);
  check(ex.client_on_stream(f.length) === 0, "a repeated bag must apply nothing");
  check(ex.client_bags_len() === 1, "and must not double the set");

  // Bag sync: sub 28 · reset 1 · count 2 · two records.
  // The reset clears what the broadcast left, so the set is the walk's.
  f = evFrame(28, [
    [1, 1], [2, 5],
    [17, 32], [1000, 17], [100 + 2048, 14], [2000, 17],
    [18, 32], [1100, 17], [110 + 2048, 14], [2100, 17],
  ]);
  writeIn(f);
  check(ex.client_on_stream(f.length) === BAGS, "a bag sync must apply with the BAGS flag");
  check(ex.client_bags_len() === 2, "the walk replaces the set it resets");
  {
    const ids = new Uint32Array(ex.memory.buffer, ex.client_bag_ids_ptr(), 2);
    check(ids[0] === 17 && ids[1] === 18, "synced bag ids mismatch");
  }

  // Bag removed: sub 29 · id 17 · why 1 (emptied).
  f = evFrame(29, [[17, 32], [1, 2]]);
  writeIn(f);
  check(ex.client_on_stream(f.length) === BAGS, "a removal must apply with the BAGS flag");
  check(ex.client_bags_len() === 1, "the removed bag must leave the set");
  {
    const ids = new Uint32Array(ex.memory.buffer, ex.client_bag_ids_ptr(), 1);
    check(ids[0] === 18, "the wrong bag was removed");
  }

  // A removal for a bag nobody knows about changes nothing and must not
  // trap — the client can miss a drop and still hear its removal.
  f = evFrame(29, [[999, 32], [0, 2]]);
  writeIn(f);
  check(ex.client_on_stream(f.length) === 0, "an unknown removal must apply nothing");
  check(ex.client_bags_len() === 1, "and must not disturb the set");

  // The raid lane (wire v13): a structure takes a hit and the client
  // reads how much of it is left. The address is the door placed above
  // (cx=100 · cz=200 · level=0 · loc=2 · row=0), whose def row this
  // gate already dripped with hp 60 — so `client_struct_hit_hp` must
  // resolve a real max out of the client's own def table, not zero.
  // sub 30 · deploy=1 (1) · cx (10) · cz (10) · level (3) · loc (2) ·
  // row (4 for the deploy store, 8 for the piece store) · damage (16) ·
  // left (16).
  const STRUCT_HIT = 67108864; // APPLIED_STRUCT_HIT = 1 << 26
  const HIT = 8388608;
  f = evFrame(30, [
    [1, 1], [100, 10], [200, 10], [0, 3], [2, 2], [0, 4], [26, 16], [34, 16],
  ]);
  writeIn(f);
  const raidFlags = ex.client_on_stream(f.length);
  check(
    (raidFlags & STRUCT_HIT) !== 0 && (raidFlags & HIT) !== 0,
    `a raid hit must raise STRUCT_HIT and the hitmarker: ${raidFlags}`,
  );
  check((ex.client_hit_pop() >>> 0) === 26, "the raid must feed the hitmarker ring");
  check(ex.client_hit_pop() >>> 0 === 0xffffffff, "and drain it");
  check(
    ex.client_struct_hit_key() === ((100 << 16) | 200),
    "struct-hit address mismatch",
  );
  check(ex.client_struct_hit_info() === ((0 << 8) | 2), "struct-hit info mismatch");
  check(
    (ex.client_struct_hit_hp() >>> 0) === ((34 << 16) | 60),
    `struct-hit hp must read left 34 of the dripped max 60: ${ex.client_struct_hit_hp() >>> 0}`,
  );
  // A piece the client has never heard of: the hit still lands (the
  // hitmarker is the attacker's own fact) but max reads 0, which is the
  // signal the HUD uses to draw nothing rather than a bar off a guess.
  f = evFrame(30, [
    [0, 1], [900, 10], [900, 10], [0, 3], [0, 2], [0, 8], [7, 16], [93, 16],
  ]);
  writeIn(f);
  check((ex.client_on_stream(f.length) & STRUCT_HIT) !== 0, "an unknown address still marks");
  check(
    (ex.client_struct_hit_hp() >>> 0) === ((93 << 16) | 0),
    "an unknown row must report max 0, never a guess",
  );
  check((ex.client_hit_pop() >>> 0) === 7, "and still feed the hitmarker");

  // The loot action: payload-free by design — kind ACTION(6) in three
  // bits and sub 8 in four is exactly one byte, and nothing else.
  {
    const n = ex.client_action_loot();
    check(n === 1, `loot frame must be one byte, got ${n}`);
    const out = new Uint8Array(ex.memory.buffer, ex.client_out_ptr(), 1);
    check(out[0] === (6 | (8 << 3)), `loot frame byte mismatch: ${out[0]}`);
  }

  // The death screen (wire v16). `client_new` above joined as player 257,
  // so a Death naming 257 is this body's and a Death naming anyone else is
  // the kill feed's — the distinction the screen depends on, and the one
  // no native test can make on this side of the wire.
  // sub 26 · victim (32) · killer (32) · cause (2) · item (16) · range (16).
  {
    const RESPAWN = 1073741824; // APPLIED_RESPAWN = 1 << 30
    const DEATH = 16777216; // APPLIED_DEATH = 1 << 24
    check(ex.client_death_screen() === 0, "the screen must start closed");

    // Somebody else's death: the feed hears it, the screen does not.
    f = evFrame(26, [[999, 32], [7, 32], [0, 2], [3, 16], [140, 16]]);
    writeIn(f);
    let df = ex.client_on_stream(f.length);
    check((df & DEATH) !== 0, `a stranger's death must reach the feed: ${df}`);
    check((df & RESPAWN) === 0, "a stranger's death must not raise our screen");
    check(ex.client_death_screen() === 0, "a stranger's death opened our screen");
    check((ex.client_death_pop() >>> 0) === 999, "the feed lost the stranger");
    check((ex.client_death_killer() >>> 0) === 7, "the feed lost the killer");

    // Ours: cause DEATH_BY_HAND(0), weapon item 3, from 140 cm.
    f = evFrame(26, [[257, 32], [42, 32], [0, 2], [3, 16], [140, 16]]);
    writeIn(f);
    df = ex.client_on_stream(f.length);
    check((df & RESPAWN) !== 0, `our own death must raise the screen: ${df}`);
    check(
      (ex.client_death_screen() >>> 0) === ((1 << 24) | 0),
      `screen word after our death: ${ex.client_death_screen() >>> 0}`,
    );
    check((ex.client_death_by() >>> 0) === 42, "the screen lost who killed us");
    check(
      (ex.client_death_weapon() >>> 0) === ((3 << 16) | 140),
      `the screen lost the weapon or the range: ${ex.client_death_weapon() >>> 0}`,
    );

    // A stranger dying now must not overwrite the sentence on our screen.
    f = evFrame(26, [[998, 32], [1, 32], [1, 2], [0xffff, 16], [0, 16]]);
    writeIn(f);
    ex.client_on_stream(f.length);
    check(
      (ex.client_death_screen() >>> 0) === ((1 << 24) | 0),
      "a stranger's death rewrote our death screen",
    );
    check((ex.client_death_by() >>> 0) === 42, "…and its killer");

    // The answer: kind ACTION(6) in three bits, sub 11 in four, and the
    // choice bit — one byte, and the bit is the whole payload.
    let n = ex.client_action_respawn(1);
    check(n === 1, `respawn frame must be one byte, got ${n}`);
    let out = new Uint8Array(ex.memory.buffer, ex.client_out_ptr(), 1);
    check(out[0] === (6 | (11 << 3) | (1 << 7)), `bag respawn byte: ${out[0]}`);
    n = ex.client_action_respawn(0);
    out = new Uint8Array(ex.memory.buffer, ex.client_out_ptr(), 1);
    check(out[0] === (6 | (11 << 3)), `beach respawn byte: ${out[0]}`);

    // …and the Respawn event closes it, carrying which anchor answered.
    // sub 35 · on_bag (1).
    f = evFrame(35, [[1, 1]]);
    writeIn(f);
    check((ex.client_on_stream(f.length) & RESPAWN) !== 0, "the wake must raise RESPAWN");
    check(
      (ex.client_death_screen() >>> 0) === (1 << 16),
      `screen must close and remember the bag: ${ex.client_death_screen() >>> 0}`,
    );
    f = evFrame(35, [[0, 1]]);
    writeIn(f);
    ex.client_on_stream(f.length);
    check(ex.client_death_screen() === 0, "a beach wake must leave the screen closed");

    // A forged fourth cause has no meaning and must not decode — the two
    // bits are the range check, the hotbar selector's posture.
    f = evFrame(26, [[257, 32], [1, 32], [3, 2], [0, 16], [0, 16]]);
    writeIn(f);
    check(
      (ex.client_on_stream(f.length) & 0x80000000) !== 0,
      "a forged death cause must be refused",
    );
    check(ex.client_death_screen() === 0, "a refused death must not open the screen");
  }

  // hp > max is a server bug, not a bar to render: refused, error bit set.
  f = evFrame(25, [[101, 16], [100, 16]]);
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
