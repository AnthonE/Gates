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

import * as THREE from "three";

const CHUNK = 64;
const NEAR_N = 65; // 64 m at 1 m + shared edge
const NEAR_RADIUS = 2; // chunks: 5×5 ring around the player
const FAR_N = 257; // 2048 m at 8 m + edge
const FAR_STEP = 8;
const ISLAND = 2048;

// Scatter archetypes, indexed by sim-core Occupant (1..7).
const ARCHETYPES = [
  null,
  { geo: () => new THREE.ConeGeometry(1.7, 5.2, 6), color: 0x2f6b33, lift: 2.6 },
  { geo: () => new THREE.DodecahedronGeometry(1.0), color: 0x8f9399, lift: 0.5 },
  { geo: () => new THREE.DodecahedronGeometry(1.0), color: 0xa1785c, lift: 0.5 },
  { geo: () => new THREE.DodecahedronGeometry(1.0), color: 0xbfae4a, lift: 0.5 },
  { geo: () => new THREE.IcosahedronGeometry(0.7), color: 0x3a7a3a, lift: 0.45 },
  { geo: () => new THREE.DodecahedronGeometry(1.5), color: 0x75726d, lift: 0.55 },
  { geo: () => new THREE.CylinderGeometry(0.45, 0.45, 0.95, 10), color: 0x5e6b78, lift: 0.5 },
];
const POOL_CAP = 4096;
const YAW8_TO_RAD = (Math.PI * 2) / 256;

export class Terrain {
  constructor(scene, seed, ex, wasmUrl) {
    this.scene = scene;
    this.seed = seed;
    this.ex = ex; // main-thread wasm: slot queries only (fast, tiny)
    this.material = new THREE.MeshLambertMaterial({ vertexColors: true });
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
    for (let k = 0; k < ARCHETYPES.length; k++) {
      if (!ARCHETYPES[k]) {
        this.pools.push(null);
        this.owners.push(null);
        continue;
      }
      const a = ARCHETYPES[k];
      const geo = a.geo();
      geo.translate(0, a.lift, 0);
      const mesh = new THREE.InstancedMesh(
        geo,
        new THREE.MeshLambertMaterial({ color: a.color }),
        POOL_CAP,
      );
      mesh.count = 0;
      mesh.frustumCulled = false; // instances span the near ring
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
    geo.setAttribute("color", new THREE.BufferAttribute(msg.colors, 3));
    geo.setIndex(new THREE.BufferAttribute(msg.indices, 1));
    const mesh = new THREE.Mesh(geo, this.material);
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
