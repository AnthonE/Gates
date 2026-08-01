// Terrain streaming (TERRAIN.md §4, cut for M0): one far mesh of the
// whole island at 8 m built once, plus a ring of 64 m near chunks at 1 m
// streamed around the player — one build in flight, one teardown per
// frame (stream-out is budgeted too; the trap list's forgotten half).
// Adjacent near chunks share exact edge heights, so same-LOD seams don't
// crack; the far mesh sits 0.15 m under the near ring to cover the
// near↔far boundary until the LOD-skirt pass.
//
// Scatter: per-archetype InstancedMesh pools filled from the shared slot
// list (sim-core scatter via the wasm bridge) — a forest is instances,
// not draw calls (DESIGN.md §9).
//
// Surfaces (materials v0): the ground is the splat material in
// materials.js, fed by a per-vertex weight attribute this worker ships
// instead of a baked colour. Scatter carries an authored PBR response per
// archetype plus baked vertex colours and a deterministic per-instance
// tint, so a forest is a forest rather than one green repeated 350 times —
// all of it still one draw call per archetype.

import * as THREE from "three";
import { makeTerrainMaterial, surfaceMaterial } from "./materials.js";

const CHUNK = 64;
const NEAR_N = 65; // 64 m at 1 m + shared edge
const NEAR_RADIUS = 2; // chunks: 5×5 ring around the player
const FAR_N = 257; // 2048 m at 8 m + edge
const FAR_STEP = 8;
const ISLAND = 2048;

/**
 * Merge positioned primitives into one non-indexed geometry carrying baked
 * vertex colours, each part ramped between two colours over a y band. This
 * is how an archetype gets more than one colour without costing a second
 * draw call: a pine's trunk, its lower skirt and its lit crown are one
 * buffer (DECISIONS.md §open, materials v0).
 */
function bakedGeometry(parts) {
  let total = 0;
  const flat = parts.map((p) => {
    const g = p.geo.index ? p.geo.toNonIndexed() : p.geo;
    if (p.geo.index) p.geo.dispose();
    total += g.attributes.position.count;
    return { g, ...p };
  });
  const pos = new Float32Array(total * 3);
  const nrm = new Float32Array(total * 3);
  const col = new Float32Array(total * 3);
  const lo = new THREE.Color();
  const hi = new THREE.Color();
  const c = new THREE.Color();
  let o = 0;
  for (const part of flat) {
    const n = part.g.attributes.position.count;
    const gp = part.g.attributes.position.array;
    pos.set(gp, o * 3);
    nrm.set(part.g.attributes.normal.array, o * 3);
    // setHex with SRGBColorSpace lands in the renderer's working space —
    // the same conversion the sky dome and every material colour gets.
    lo.setHex(part.lo, THREE.SRGBColorSpace);
    hi.setHex(part.hi === undefined ? part.lo : part.hi, THREE.SRGBColorSpace);
    const y0 = part.y0 === undefined ? 0 : part.y0;
    const y1 = part.y1 === undefined ? 1 : part.y1;
    for (let i = 0; i < n; i++) {
      const t = Math.max(0, Math.min((gp[i * 3 + 1] - y0) / (y1 - y0), 1));
      c.copy(lo).lerp(hi, t);
      col[(o + i) * 3] = c.r;
      col[(o + i) * 3 + 1] = c.g;
      col[(o + i) * 3 + 2] = c.b;
    }
    o += n;
    part.g.dispose();
  }
  const geo = new THREE.BufferGeometry();
  geo.setAttribute("position", new THREE.BufferAttribute(pos, 3));
  geo.setAttribute("normal", new THREE.BufferAttribute(nrm, 3));
  geo.setAttribute("color", new THREE.BufferAttribute(col, 3));
  return geo;
}

/** A conifer: bare trunk, dark lower skirt, lighter crown. */
function pineGeometry() {
  const trunk = new THREE.CylinderGeometry(0.16, 0.24, 1.7, 6, 1, true);
  trunk.translate(0, 0.85, 0);
  const skirt = new THREE.ConeGeometry(1.7, 3.1, 7);
  skirt.translate(0, 2.65, 0);
  const crown = new THREE.ConeGeometry(1.15, 2.5, 7);
  crown.translate(0, 3.95, 0);
  return bakedGeometry([
    { geo: trunk, lo: 0x3a2a1e, hi: 0x53402d, y0: 0, y1: 1.7 },
    { geo: skirt, lo: 0x1e4423, hi: 0x39733a, y0: 1.1, y1: 4.2 },
    { geo: crown, lo: 0x2c5c2c, hi: 0x4d8845, y0: 2.7, y1: 5.2 },
  ]);
}

// Scatter archetypes, indexed by sim-core Occupant (1..7). `surface` names
// an authored PBR response in materials.js; `tint` is the amplitude of the
// deterministic per-instance colour variation (0 = every instance alike).
const ARCHETYPES = [
  null,
  { geo: pineGeometry, surface: "foliage", lo: 0xffffff, lift: 0, tint: 0.17 },
  { geo: () => new THREE.DodecahedronGeometry(1.0), surface: "rock", lo: 0x8f9399, lift: 0.5, tint: 0.11 },
  { geo: () => new THREE.DodecahedronGeometry(1.0), surface: "ore", lo: 0xa1785c, lift: 0.5, tint: 0.1 },
  { geo: () => new THREE.DodecahedronGeometry(1.0), surface: "ore", lo: 0xbfae4a, lift: 0.5, tint: 0.1 },
  { geo: () => new THREE.IcosahedronGeometry(0.7), surface: "foliage", lo: 0x2c5f2e, hi: 0x4b8a3f, y0: -0.7, y1: 0.7, lift: 0.45, tint: 0.19 },
  { geo: () => new THREE.DodecahedronGeometry(1.5), surface: "rock", lo: 0x75726d, lift: 0.55, tint: 0.12 },
  { geo: () => new THREE.CylinderGeometry(0.45, 0.45, 0.95, 10), surface: "metal", lo: 0x5e6b78, lift: 0.5, tint: 0.08 },
];
const POOL_CAP = 4096;
const YAW8_TO_RAD = (Math.PI * 2) / 256;

/**
 * Per-instance tint from the slot's own cell, so a chunk that streams out
 * and back gets the same forest — deterministic like everything else the
 * scatter derives. Warm/cool and value move on independent bits of one
 * hash, which is what stops a tinted pool reading as a brightness ramp.
 */
function instanceTint(cellKey, amp, out) {
  let h = Math.imul(cellKey ^ 0x9e3779b9, 0x85ebca6b);
  h ^= h >>> 13;
  h = Math.imul(h, 0xc2b2ae35);
  h ^= h >>> 16;
  const v = ((h & 0x3ff) / 1023 - 0.5) * 2 * amp;
  const w = (((h >>> 10) & 0x3ff) / 1023 - 0.5) * 2 * amp * 0.5;
  out.setRGB(1 + v + w, 1 + v, 1 + v - w);
}

export class Terrain {
  constructor(scene, seed, ex, wasmUrl) {
    this.scene = scene;
    this.seed = seed;
    this.ex = ex; // main-thread wasm: slot queries only (fast, tiny)
    this.material = makeTerrainMaterial();
    this.chunks = new Map(); // "cx,cz" -> { cx, cz, mesh? , pending? }
    this.queue = [];
    this.inFlight = false;
    this.teardown = [];
    this.farBuilt = false;
    // The desired-set scan runs only on chunk-boundary crossings, so the
    // steady-state RAF path builds no key strings (DESIGN.md L8).
    this.lastCcx = -1000;
    this.lastCcz = -1000;

    this.pools = [];
    this.owners = []; // per archetype: entry objects parallel to instances
    this._c = new THREE.Color();
    for (let k = 0; k < ARCHETYPES.length; k++) {
      if (!ARCHETYPES[k]) {
        this.pools.push(null);
        this.owners.push(null);
        continue;
      }
      const a = ARCHETYPES[k];
      const geo = a.geo === pineGeometry ? pineGeometry() : bakedGeometry([{ ...a, geo: a.geo() }]);
      geo.translate(0, a.lift, 0);
      const mesh = new THREE.InstancedMesh(
        geo,
        surfaceMaterial(a.surface, { color: 0xffffff, vertexColors: true }),
        POOL_CAP,
      );
      // Allocate the per-instance colour buffer up front: the tint is
      // written on stream-in, and a pool that first renders without one
      // would make three recompile its program mid-play.
      mesh.setColorAt(0, this._c.setRGB(1, 1, 1));
      mesh.count = 0;
      mesh.frustumCulled = false; // instances span the near ring
      // A forest that casts nothing is the whole reason the ground reads
      // flat; one InstancedMesh casts for every instance in it.
      mesh.castShadow = true;
      mesh.receiveShadow = true;
      scene.add(mesh);
      this.pools.push(mesh);
      this.owners.push([]);
    }
    this.chunkSlots = new Map(); // key -> [{arch, idx, key, cellKey, …}]
    this.cellIndex = new Map(); // cellKey (cx<<16|cz) -> entry
    this._m4 = new THREE.Matrix4();
    this._q = new THREE.Quaternion();
    this._e = new THREE.Euler();
    this._v = new THREE.Vector3();
    this._s = new THREE.Vector3();

    this.worker = new Worker(new URL("./terrainWorker.js", import.meta.url), {
      type: "module",
    });
    this.worker.onmessage = (e) => this._onWorker(e.data);
    this.worker.postMessage({ type: "init", seed, wasmUrl });
  }

  _onWorker(msg) {
    if (msg.type === "error") {
      // Worker-side invariant break: make it page-visible — the browser
      // smoke gate fails on any console.error.
      console.error(`terrain worker: ${msg.message}`);
      return;
    }
    if (msg.type === "ready") {
      // Far mesh first: the horizon exists before the player does.
      this.worker.postMessage({
        type: "build",
        key: "far",
        x0: 0,
        z0: 0,
        n: FAR_N,
        step: FAR_STEP,
      });
      this.inFlight = true;
      return;
    }
    if (msg.type !== "built") return;
    this.inFlight = false;
    const geo = new THREE.BufferGeometry();
    geo.setAttribute("position", new THREE.BufferAttribute(msg.positions, 3));
    geo.setAttribute("normal", new THREE.BufferAttribute(msg.normals, 3));
    // Four identity weights per vertex, normalized bytes: 4 B where the
    // retired baked colour cost 12, and the shader blends the identities.
    geo.setAttribute("splat", new THREE.BufferAttribute(msg.splat, 4, true));
    geo.setIndex(new THREE.BufferAttribute(msg.indices, 1));
    const mesh = new THREE.Mesh(geo, this.material);
    // Shadows: the near ring both casts and receives. The far mesh only
    // receives — it is the 8 m LOD of the SAME ground the near ring already
    // casts from, so letting it cast would put two disagreeing silhouettes
    // in one map (self-shadow acne along the whole near↔far boundary) and
    // spend 131 k triangles a frame doing it.
    mesh.receiveShadow = true;
    mesh.castShadow = msg.key !== "far";
    if (msg.key === "far") {
      mesh.position.set(0, -0.15, 0);
      this.farBuilt = true;
      this.scene.add(mesh);
    } else {
      const entry = this.chunks.get(msg.key);
      if (!entry || !entry.pending) {
        geo.dispose(); // unloaded while building
      } else {
        mesh.position.set(msg.x0, 0, msg.z0);
        this.scene.add(mesh);
        entry.pending = false;
        entry.mesh = mesh;
        this._addScatter(msg.key, msg.x0, msg.z0);
      }
    }
    this._kick();
  }

  _addScatter(key, x0, z0) {
    const count = this.ex.terrain_fill_slots(this.seed, x0 / 8, z0 / 8, 8);
    const slots = new Float32Array(
      this.ex.memory.buffer,
      this.ex.terrain_slots_ptr(),
      count * 8,
    );
    const list = [];
    for (let i = 0; i < count; i++) {
      const arch = slots[i * 8] | 0;
      const pool = this.pools[arch];
      if (!pool || pool.count >= POOL_CAP) continue;
      const idx = pool.count;
      const cx = slots[i * 8 + 6] | 0;
      const cz = slots[i * 8 + 7] | 0;
      const entry = {
        arch,
        idx,
        key,
        cellKey: ((cx << 16) | cz) >>> 0,
        x: slots[i * 8 + 1],
        y: slots[i * 8 + 2],
        z: slots[i * 8 + 3],
        yaw8: slots[i * 8 + 4],
        scale: slots[i * 8 + 5],
        // Chunks stream in after the join sync: ask the client core.
        hidden: this.ex.client_cell_harvested(cx, cz) === 1,
      };
      pool.count = idx + 1;
      this._composeEntry(entry);
      this._composeTint(entry);
      this.owners[arch][idx] = entry;
      this.cellIndex.set(entry.cellKey, entry);
      list.push(entry);
    }
    this.chunkSlots.set(key, list);
  }

  /** Write an entry's matrix — scale 0 while its node is harvested. */
  _composeEntry(entry) {
    const pool = this.pools[entry.arch];
    this._e.set(0, entry.yaw8 * YAW8_TO_RAD, 0);
    this._q.setFromEuler(this._e);
    this._v.set(entry.x, entry.y, entry.z);
    const s = entry.hidden ? 0 : entry.scale;
    this._s.set(s, s, s);
    this._m4.compose(this._v, this._q, this._s);
    pool.setMatrixAt(entry.idx, this._m4);
    pool.instanceMatrix.needsUpdate = true;
  }

  /** Write an entry's per-instance tint — derived, never stored. */
  _composeTint(entry) {
    const pool = this.pools[entry.arch];
    instanceTint(entry.cellKey, ARCHETYPES[entry.arch].tint, this._c);
    pool.setColorAt(entry.idx, this._c);
    pool.instanceColor.needsUpdate = true;
  }

  /** The rendered scatter entry at cellKey, or null (chunk not streamed). */
  cellEntry(cellKey) {
    return this.cellIndex.get(cellKey) || null;
  }

  /** Event-lane fact: the node at this cell vanished or came back. */
  setCellHarvested(cellKey, harvested) {
    const entry = this.cellIndex.get(cellKey);
    if (!entry || entry.hidden === harvested) return;
    entry.hidden = harvested;
    this._composeEntry(entry);
  }

  /** Sync reset: un-hide everything; the batch that follows re-hides. */
  resetHarvested() {
    for (const entry of this.cellIndex.values()) {
      if (entry.hidden) {
        entry.hidden = false;
        this._composeEntry(entry);
      }
    }
  }

  _removeScatter(key) {
    const list = this.chunkSlots.get(key);
    if (!list) return;
    // Highest index first so swap-with-last stays valid.
    list.sort((a, b) => b.idx - a.idx);
    for (const entry of list) {
      const pool = this.pools[entry.arch];
      const owners = this.owners[entry.arch];
      const last = pool.count - 1;
      if (entry.idx !== last) {
        pool.getMatrixAt(last, this._m4);
        pool.setMatrixAt(entry.idx, this._m4);
        const mover = owners[last];
        mover.idx = entry.idx;
        owners[entry.idx] = mover;
        // The tint is a pure function of the mover's cell, so recompute it
        // at its new index rather than reading the old slot back.
        this._composeTint(mover);
      }
      pool.count = last;
      owners.pop();
      pool.instanceMatrix.needsUpdate = true;
      this.cellIndex.delete(entry.cellKey);
    }
    this.chunkSlots.delete(key);
  }

  /** Per-frame: retarget the near ring, one build kick, one teardown. */
  update(px, pz) {
    const ccx = Math.floor(px / CHUNK);
    const ccz = Math.floor(pz / CHUNK);
    if (ccx !== this.lastCcx || ccz !== this.lastCcz) {
      this.lastCcx = ccx;
      this.lastCcz = ccz;
      for (let dz = -NEAR_RADIUS; dz <= NEAR_RADIUS; dz++) {
        for (let dx = -NEAR_RADIUS; dx <= NEAR_RADIUS; dx++) {
          const cx = ccx + dx;
          const cz = ccz + dz;
          if (cx < 0 || cz < 0 || cx * CHUNK >= ISLAND || cz * CHUNK >= ISLAND)
            continue;
          const key = cx + "," + cz;
          if (!this.chunks.has(key)) {
            this.chunks.set(key, { cx, cz, pending: true });
            this.queue.push({ key, cx, cz, d: Math.abs(dx) + Math.abs(dz) });
          }
        }
      }
      // Stream-out: mark chunks beyond radius+1 (hysteresis), drop 1/frame.
      for (const [key, entry] of this.chunks) {
        if (!entry.mesh) continue;
        if (
          Math.abs(entry.cx - ccx) > NEAR_RADIUS + 1 ||
          Math.abs(entry.cz - ccz) > NEAR_RADIUS + 1
        ) {
          this.teardown.push(key);
        }
      }
    }
    const key = this.teardown.pop();
    if (key) {
      const entry = this.chunks.get(key);
      if (entry && entry.mesh) {
        this.scene.remove(entry.mesh);
        entry.mesh.geometry.dispose();
        this._removeScatter(key);
        this.chunks.delete(key);
      }
    }
    this._kick();
  }

  /**
   * Per-archetype surface facts, for the browser gate: which authored
   * response each pool wears, whether it carries baked vertex colours and a
   * per-instance tint, and how many instances are live. One flat green cone
   * pool would show here as `tint: 0` with no colour attribute.
   */
  scatterFacts() {
    const out = [];
    for (let k = 1; k < ARCHETYPES.length; k++) {
      const pool = this.pools[k];
      if (!pool) continue;
      out.push({
        arch: k,
        surface: ARCHETYPES[k].surface,
        type: pool.material.type,
        roughness: pool.material.roughness,
        metalness: pool.material.metalness,
        vertexColors: !!pool.material.vertexColors,
        instanceColor: pool.instanceColor !== null,
        tint: ARCHETYPES[k].tint,
        count: pool.count,
      });
    }
    return out;
  }

  /**
   * Dev-only: what the splat attribute actually says over the streamed near
   * ring. The shader can blend four identities beautifully and still paint
   * one, if the weights it is fed are constant or one-hot — neither of which
   * a pixel probe can tell apart from a well-blended world. So this counts
   * the ground the player is standing in: which identities are present, and
   * how much of it is a genuine blend rather than a hard biome cell.
   *
   * Scans every near-chunk vertex (~100 k reads), so it is a gate hook and
   * never a per-frame one — ci/browser_smoke.mjs calls it once.
   */
  splatCensus() {
    const dominant = [0, 0, 0, 0];
    const present = [0, 0, 0, 0];
    const lo = [255, 255, 255, 255];
    const hi = [0, 0, 0, 0];
    let vertices = 0;
    let mixed = 0;
    let maxSecond = 0;
    for (const entry of this.chunks.values()) {
      if (!entry.mesh) continue;
      const a = entry.mesh.geometry.getAttribute("splat");
      if (!a) continue;
      const w = a.array;
      for (let i = 0; i < a.count; i++) {
        const o = i * 4;
        let best = 0;
        let second = 0;
        let bestK = 0;
        for (let k = 0; k < 4; k++) {
          const v = w[o + k];
          // A twentieth of a vertex is a contribution the shader's blend
          // can see; anything under it is quantization dust.
          if (v > 12) present[k]++;
          if (v < lo[k]) lo[k] = v;
          if (v > hi[k]) hi[k] = v;
          if (v > best) {
            second = best;
            best = v;
            bestK = k;
          } else if (v > second) {
            second = v;
          }
        }
        if (second > maxSecond) maxSecond = second;
        dominant[bestK]++;
        // Below 90% of the byte range, at least two identities are really
        // sharing this vertex — that is a splat blend and not a biome cell.
        if (best < 230) mixed++;
        vertices++;
      }
    }
    return {
      chunks: this.chunks.size,
      vertices,
      dominant,
      dominantFraction: dominant.map((n) => (vertices ? n / vertices : 0)),
      presentFraction: present.map((n) => (vertices ? n / vertices : 0)),
      // Per-identity range over the ring. This is the measure that says the
      // weights are a FIELD: a constant attribute has zero spread on every
      // channel no matter which biome the sample happens to sit in.
      spread: hi.map((h, k) => (vertices ? (h - lo[k]) / 255 : 0)),
      // The most any second identity ever gets. A hard threshold hands the
      // whole vertex to one identity, so a biome edge crossed by a ramp
      // reaches ~0.5 somewhere and a hard one never leaves ~0. This is the
      // measure that does not care how much of the ring is a boundary.
      maxSecond: maxSecond / 255,
      mixedFraction: vertices ? mixed / vertices : 0,
    };
  }

  _kick() {
    if (this.inFlight || this.queue.length === 0) return;
    this.queue.sort((a, b) => a.d - b.d);
    const next = this.queue.shift();
    if (!this.chunks.has(next.key)) return; // torn down while queued
    this.inFlight = true;
    this.worker.postMessage({
      type: "build",
      key: next.key,
      x0: next.cx * CHUNK,
      z0: next.cz * CHUNK,
      n: NEAR_N,
      step: 1,
    });
  }
}
