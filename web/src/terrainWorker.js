// Terrain mesh worker (TERRAIN.md §4): heights and normals from the
// shared wasm worldgen, meshes built off the main thread, transferred
// back. Biome coloring is per-vertex from (height, moisture, slope) —
// the splat-shader-with-textures upgrade is an art pass, not a worldgen
// change; the bands below mirror sim-core terrain.rs biome().

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
      built.colors.buffer,
      built.indices.buffer,
    ]);
  }
};

// Palette (v0 vertex colors; DECISIONS.md §open, client cosmetics).
const C_SEAFLOOR = [0.5, 0.47, 0.36];
const C_BEACH = [0.78, 0.71, 0.52];
const C_MEADOW = [0.35, 0.49, 0.23];
const C_FOREST = [0.15, 0.33, 0.16];
const C_HIGHLAND = [0.55, 0.55, 0.58];
const C_PEAK = [0.72, 0.72, 0.75];
const C_CLIFF = [0.46, 0.44, 0.4];
const CLIFF_SLOPE = 1.19; // tan(50°), the sim's cliff threshold

function biomeColor(h, moist, slope, out) {
  let c;
  if (h < 0.6) c = C_SEAFLOOR;
  else if (h < 2.0) c = C_BEACH;
  else if (h > 52.0) {
    const t = Math.min((h - 52.0) / 28.0, 1.0);
    out[0] = C_HIGHLAND[0] + (C_PEAK[0] - C_HIGHLAND[0]) * t;
    out[1] = C_HIGHLAND[1] + (C_PEAK[1] - C_HIGHLAND[1]) * t;
    out[2] = C_HIGHLAND[2] + (C_PEAK[2] - C_HIGHLAND[2]) * t;
    c = out;
  } else if (moist > 0.05) c = C_FOREST;
  else c = C_MEADOW;
  // Cliff mask forces rock (TERRAIN.md §4), blended in from 80% slope.
  const ct = Math.max(0, Math.min((slope - CLIFF_SLOPE * 0.8) / (CLIFF_SLOPE * 0.4), 1));
  out[0] = c[0] + (C_CLIFF[0] - c[0]) * ct;
  out[1] = c[1] + (C_CLIFF[1] - c[1]) * ct;
  out[2] = c[2] + (C_CLIFF[2] - c[2]) * ct;
  return out;
}

/** Deterministic tiny per-vertex tint so flats don't read as plastic. */
function jitter(i, j) {
  let h = (i * 374761393 + j * 668265263) | 0;
  h = Math.imul(h ^ (h >>> 13), 1274126177);
  return (((h ^ (h >>> 16)) & 0xff) / 255 - 0.5) * 0.07;
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
  const colors = new Float32Array(n * n * 3);
  const indices = new Uint32Array((n - 1) * (n - 1) * 6);
  const col = [0, 0, 0];
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
      biomeColor(h, moist, slope, col);
      const jt = jitter(x0 + i * step, z0 + j * step);
      colors[v * 3] = Math.max(0, Math.min(1, col[0] + jt));
      colors[v * 3 + 1] = Math.max(0, Math.min(1, col[1] + jt));
      colors[v * 3 + 2] = Math.max(0, Math.min(1, col[2] + jt));
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

  return { key, x0, z0, positions, normals, colors, indices };
}
