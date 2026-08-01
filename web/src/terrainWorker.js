// Terrain mesh worker (TERRAIN.md §4): heights and normals from the
// shared wasm worldgen, meshes built off the main thread, transferred
// back. What rides with them is the SPLAT WEIGHT attribute — four
// identity weights per vertex derived from (height, moisture, slope),
// the same three channels sim-core terrain.rs biome() decides on. The
// colour is no longer computed here: materials.js blends the four
// authored identities from these weights and does every causal modifier
// (waterline wetness, snow, cliff darkening) in the shader, where the
// noise that breaks up the boundaries lives too.
//
// Weights are soft ramps CENTRED ON THE OLD HARD EDGES (beach 2 m,
// highland 52 m, forest moisture 0.05) rather than the thresholds
// themselves: a one-hot attribute would put a hard biome seam on the
// vertex grid that no amount of in-shader noise can hide.

import { loadWasm } from "./wasm.js";

let ex = null;
let seed = 0n;
let loading = null;

// `async onmessage` does NOT serialize messages: while `init` awaits loadWasm,
// later messages are still delivered. The main thread's _kick() posts `build`
// from the RAF loop as soon as the first snapshot lands, guarded only by
// `inFlight` — which is false until `ready`. So builds arrived with `ex` still
// null and the near ring died on every chunk ("Cannot read properties of null
// reading 'terrain_fill_heights'"), while the far mesh — posted from the ready
// handler — rendered fine and made screenshots look correct.
//
// Sequencing here rather than on the main thread: the worker owns whether it is
// usable, so no future caller can reintroduce the race by forgetting to wait.
self.onmessage = async (e) => {
  const msg = e.data;
  if (msg.type === "init") {
    seed = msg.seed;
    loading = loadWasm(msg.wasmUrl).then((m) => {
      ex = m;
    });
    await loading;
    self.postMessage({ type: "ready" });
    return;
  }
  if (msg.type === "build") {
    if (!ex) {
      if (!loading) {
        // Unreachable today — terrain.js posts `init` in its constructor,
        // before any build can queue. If a refactor reorders that, a throw
        // here would vanish into an unhandled worker rejection; a posted
        // error reaches the page, where the browser-smoke gate counts it.
        self.postMessage({ type: "error", message: "build before init — worker has no wasm" });
        return;
      }
      await loading;
    }
    const built = build(msg);
    self.postMessage({ type: "built", ...built }, [
      built.positions.buffer,
      built.normals.buffer,
      built.splat.buffer,
      built.indices.buffer,
    ]);
  }
};

// The band edges, each the retired palette's hard threshold with a ramp
// hung around it (DECISIONS.md §open, materials v0).
const BEACH_BAND = [1.0, 3.0]; // was: beach below h 2.0
const ALPINE_BAND = [44.0, 60.0]; // was: highland above h 52.0
const MOIST_BAND = [0.01, 0.09]; // was: forest above moisture 0.05
const CLIFF_SLOPE = 1.19; // tan(50°), the sim's cliff threshold
const CLIFF_BAND = [CLIFF_SLOPE * 0.8, CLIFF_SLOPE * 1.2];

function ramp(lo, hi, v) {
  const t = Math.max(0, Math.min((v - lo) / (hi - lo), 1));
  return t * t * (3 - 2 * t);
}

/**
 * Four identity weights — sand · grass · forest litter · rock — from the
 * three channels the sim's biome() decides on. Written as bytes: these are
 * a blend, and 1/255 of a weight is far below what the shader's break-up
 * moves them by.
 */
function splatWeights(h, moist, slope, out, o) {
  const sand = 1 - ramp(BEACH_BAND[0], BEACH_BAND[1], h);
  const alpine = ramp(ALPINE_BAND[0], ALPINE_BAND[1], h);
  const wood = ramp(MOIST_BAND[0], MOIST_BAND[1], moist);
  const cliff = ramp(CLIFF_BAND[0], CLIFF_BAND[1], slope);
  const land = 1 - sand;
  let w0 = sand;
  let w1 = land * (1 - alpine) * (1 - wood);
  let w2 = land * (1 - alpine) * wood;
  let w3 = land * alpine;
  // The cliff mask forces rock (TERRAIN.md §4) — the one veto that
  // overrides the biome rather than blending with it.
  w0 += (0 - w0) * cliff;
  w1 += (0 - w1) * cliff;
  w2 += (0 - w2) * cliff;
  w3 += (1 - w3) * cliff;
  const sum = w0 + w1 + w2 + w3 || 1;
  const k = 255 / sum;
  out[o] = (w0 * k + 0.5) | 0;
  out[o + 1] = (w1 * k + 0.5) | 0;
  out[o + 2] = (w2 * k + 0.5) | 0;
  out[o + 3] = (w3 * k + 0.5) | 0;
}

/**
 * Build one n×n vertex grid at (x0, z0), `step` meters apart, positions
 * local to the chunk origin. A one-sample apron supplies the normals at
 * the edges, so adjacent same-step chunks share exact edge normals too.
 */
function build({ key, x0, z0, n, step }) {
  const g = n + 2;
  const wrote = ex.terrain_fill_heights(seed, x0 - step, z0 - step, g, step);
  if (wrote !== g * g) throw new Error(`heights fill refused: n=${g}`);
  const H = new Float32Array(ex.memory.buffer, ex.terrain_heights_ptr(), g * g);

  const positions = new Float32Array(n * n * 3);
  const normals = new Float32Array(n * n * 3);
  const splat = new Uint8Array(n * n * 4);
  const indices = new Uint32Array((n - 1) * (n - 1) * 6);
  const inv2s = 1 / (2 * step);

  for (let j = 0; j < n; j++) {
    for (let i = 0; i < n; i++) {
      const v = j * n + i;
      const h = H[(j + 1) * g + (i + 1)];
      positions[v * 3] = i * step;
      positions[v * 3 + 1] = h;
      positions[v * 3 + 2] = j * step;

      const dhdx = (H[(j + 1) * g + (i + 2)] - H[(j + 1) * g + i]) * inv2s;
      const dhdz = (H[(j + 2) * g + (i + 1)] - H[j * g + (i + 1)]) * inv2s;
      const inv = 1 / Math.sqrt(dhdx * dhdx + 1 + dhdz * dhdz);
      normals[v * 3] = -dhdx * inv;
      normals[v * 3 + 1] = inv;
      normals[v * 3 + 2] = -dhdz * inv;

      const moist = ex.terrain_moisture_at(seed, x0 + i * step, z0 + j * step);
      const slope = Math.sqrt(dhdx * dhdx + dhdz * dhdz);
      splatWeights(h, moist, slope, splat, v * 4);
    }
  }

  let q = 0;
  for (let j = 0; j < n - 1; j++) {
    for (let i = 0; i < n - 1; i++) {
      const a = j * n + i;
      const b = a + 1;
      const c = a + n;
      const d = c + 1;
      indices[q++] = a;
      indices[q++] = c;
      indices[q++] = b;
      indices[q++] = b;
      indices[q++] = c;
      indices[q++] = d;
    }
  }

  return { key, x0, z0, positions, normals, splat, indices };
}
