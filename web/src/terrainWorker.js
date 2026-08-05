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

// The splat law does not live here any more.
//
// It used to: five band constants, a `ramp`, and the cliff override, in JS,
// beside a Rust `biome()` that decided on the same three channels. That is the
// arrangement `threejs-procedural-fields` names in its rejection list —
// "geometry and shading claim the same feature but evaluate different
// functions" — and it went from latent to load-bearing the moment the ground
// grew a population: `terrain::clutter_cell` picks a tuft or a pebble from
// these same four weights, so a JS copy drifting by one rounding step would
// put grass geometry on sand. One law, in `crates/sim-core/src/terrain.rs`
// (`splat_from`), reached through the bridge — held by CONSTRUCTION, since
// deleting the copy left nothing to hold equal. The band numbers and their
// derivation went with it. (This line used to cite a `ci/splat_parity.mjs`
// that has never existed in `ci/`; a wall claimed only in prose is the mood
// CLAUDE.md warns about, so the claim goes rather than the arrangement.)

/**
 * Four identity weights — sand · grass · forest litter · rock — as bytes,
 * from the three channels the sim's biome() decides on.
 */
function splatWeights(h, moist, slope, out, o) {
  const p = ex.terrain_splat_from(h, moist, slope);
  out[o] = p & 255;
  out[o + 1] = (p >>> 8) & 255;
  out[o + 2] = (p >>> 16) & 255;
  out[o + 3] = (p >>> 24) & 255;
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
