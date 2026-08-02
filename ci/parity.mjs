#!/usr/bin/env node
// Wasm half of test_parity_wasm: loads the sim-core cdylib built for
// wasm32-unknown-unknown (no bindgen — raw C ABI, u64 as BigInt) and prints
// probe digests in exactly the format examples/probe.rs prints natively.
// ci/gates.sh diffs the outputs. Any missing artifact is a loud failure,
// never a silent skip (CLAUDE.md trap list).

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const wasmPath = join(
  root,
  "target/wasm32-unknown-unknown/release/sim_core.wasm",
);

// Keep in lockstep with examples/probe.rs — a mismatch shows up as a diff.
const TERRAIN_SEEDS = [0x4741544553n, 0x1n, 0xdeadbeefn];
const PARITY_MASTER_SEED = 0x4741544553n;
const PARITY_SEQUENCES = 10000;
const PARITY_TICKS = 16;
const COMBAT_SEQUENCES = 500;
const COMBAT_TICKS = 256;

let bytes;
try {
  bytes = readFileSync(wasmPath);
} catch {
  console.error(`GATE FAIL: wasm artifact missing at ${wasmPath}`);
  console.error("build it: cargo build -p sim-core --release --target wasm32-unknown-unknown");
  process.exit(1);
}

const { instance } = await WebAssembly.instantiate(bytes, {});
const { probe_terrain, probe_parity, probe_combat } = instance.exports;
if (
  typeof probe_terrain !== "function" ||
  typeof probe_parity !== "function" ||
  typeof probe_combat !== "function"
) {
  console.error("GATE FAIL: probe exports missing from sim_core.wasm");
  process.exit(1);
}

const hex = (v) => "0x" + v.toString(16).padStart(16, "0");
for (const seed of TERRAIN_SEEDS) {
  console.log(`terrain ${hex(seed)} ${hex(BigInt.asUintN(64, probe_terrain(seed)))}`);
}
const p = BigInt.asUintN(
  64,
  probe_parity(PARITY_MASTER_SEED, PARITY_SEQUENCES, PARITY_TICKS),
);
console.log(
  `parity ${hex(PARITY_MASTER_SEED)} ${PARITY_SEQUENCES} ${PARITY_TICKS} ${hex(p)}`,
);
const c = BigInt.asUintN(
  64,
  probe_combat(PARITY_MASTER_SEED, COMBAT_SEQUENCES, COMBAT_TICKS),
);
console.log(
  `combat ${hex(PARITY_MASTER_SEED)} ${COMBAT_SEQUENCES} ${COMBAT_TICKS} ${hex(c)}`,
);
