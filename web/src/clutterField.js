/**
 * The clutter POOLS and their streaming — the half of `clutter.js` that needs
 * a renderer. Split from it so the geometry and the numbers stay importable
 * from bare node, which is what lets `ci/clutter_shape.mjs` assert them in
 * twenty seconds instead of nineteen minutes. Same split, same reason, as
 * `props.js` (geometry) against `terrain.js` (pools).
 */

import * as THREE from "three";
import { surfaceMaterial } from "./materials.js";
import {
  CLUTTER_FILLS_PER_FRAME,
  CLUTTER_FLOATS,
  CLUTTER_POOL_CAP,
  CLUTTER_RING,
  CLUTTER_ROWS,
  CLUTTER_TILE_CAP,
  CLUTTER_TILE_M,
} from "./clutter.js";

/**
 * One pooled `InstancedMesh` per kind, and a ring of tiles cached as raw
 * bridge floats.
 *
 * The streaming shape is deliberately the simple one. Tiles are filled at
 * most `CLUTTER_FILLS_PER_FRAME` per frame (the expensive half — 625 cells of
 * worldgen through wasm), and the instance matrices are rewritten wholesale
 * from the cache whenever the set changes. A wholesale rewrite is ~15k matrix
 * composes off numbers already in memory; the alternative — a dense-swap pool
 * with per-tile index ranges and a back-index — buys a fraction of that and
 * costs the bookkeeping that the VFX pack warns gets it wrong ("updating only
 * `mesh.count` without copying custom attributes"). The rewrite happens on a
 * tile-boundary crossing, which is every 16 m walked, not every frame.
 */
export class ClutterField {
  constructor(scene, seed, ex) {
    this.seed = seed;
    this.ex = ex;
    this.scene = scene;
    this.tileX = null;
    this.tileZ = null;
    this.dirty = false;

    // Preallocated, once: the RAF loop allocates nothing (CLAUDE.md's client
    // hot-path law — a GC pause here feels identical to a server blip).
    this._m = new THREE.Matrix4();
    this._q = new THREE.Quaternion();
    this._p = new THREE.Vector3();
    this._s = new THREE.Vector3();
    this._up = new THREE.Vector3(0, 1, 0);
    this._c = new THREE.Color();

    const side = 2 * CLUTTER_RING + 1;
    /** Ring slots, row-major; each holds its tile coords and its elements. */
    this.tiles = [];
    for (let i = 0; i < side * side; i++) {
      this.tiles.push({
        tx: 0,
        tz: 0,
        n: 0,
        loaded: false,
        data: new Float32Array(CLUTTER_TILE_CAP * CLUTTER_FLOATS),
      });
    }
    /** Slot indices still to fill, drained `CLUTTER_FILLS_PER_FRAME` a frame. */
    this.pending = [];

    this.pools = CLUTTER_ROWS.map((row) => {
      const geo = row.geo();
      const mesh = new THREE.InstancedMesh(
        geo,
        surfaceMaterial(row.surface, { color: 0xffffff, vertexColors: true }),
        CLUTTER_POOL_CAP,
      );
      // Allocate the per-instance colour buffer at boot, exactly as the
      // scatter pools do. This is not decoration: `USE_INSTANCING_COLOR` is a
      // shader DEFINE, so a pool that first renders without one is a
      // different program from the pools beside it, and it would link on the
      // frame the first tuft streams in — a mid-play link, which is the
      // prewarm count gate's whole subject.
      mesh.setColorAt(0, this._c.setRGB(1, 1, 1));
      mesh.count = 0;
      mesh.frustumCulled = false; // the ring surrounds the camera
      mesh.castShadow = false; // drawn once, not once per cascade — see the header
      mesh.receiveShadow = true;
      scene.add(mesh);
      return mesh;
    });
  }

  /**
   * Dummies for the scene's prewarm group — NOT the live pools.
   *
   * `scene.prewarm` compiles what it is handed and then REMOVES it, so
   * handing it the real pools would take the population out of the scene on
   * the frame after boot. The pools themselves boot at `count = 0` and never
   * enter a render list, which is the same reason `terrain.js` builds a
   * count-1 sample for its own: a pool that has never rendered has never had
   * its program linked. One dummy per kind, because the kinds differ in
   * surface identity and the prop field's uniforms ride on that.
   */
  prewarmObjects() {
    return this.pools.map((pool) => {
      const m = new THREE.InstancedMesh(new THREE.PlaneGeometry(0.02, 0.02), pool.material, 1);
      m.setColorAt(0, this._c.setRGB(1, 1, 1));
      m.castShadow = false;
      m.receiveShadow = true;
      m.frustumCulled = false;
      return m;
    });
  }

  /**
   * Called every frame from `Terrain.update`. Cheap in the steady state: a
   * pair of floors and a compare, then nothing.
   */
  update(px, pz) {
    const tx = Math.floor(px / CLUTTER_TILE_M);
    const tz = Math.floor(pz / CLUTTER_TILE_M);
    if (tx !== this.tileX || tz !== this.tileZ) {
      this._reseat(tx, tz);
      this.tileX = tx;
      this.tileZ = tz;
    }
    for (let i = 0; i < CLUTTER_FILLS_PER_FRAME && this.pending.length; i++) {
      this._fill(this.pending.pop());
    }
    if (this.dirty) {
      this._writeInstances();
      this.dirty = false;
    }
  }

  /**
   * Move the ring to a new centre, keeping every tile that is still in it.
   * Without the keep, a step across one boundary would refill all 25 tiles
   * instead of the 5 that actually changed.
   */
  _reseat(tx, tz) {
    const side = 2 * CLUTTER_RING + 1;
    const keep = new Map();
    for (const t of this.tiles) {
      if (t.loaded) keep.set(t.tx * 100003 + t.tz, t);
    }
    const fresh = [];
    const reused = new Set();
    for (let j = 0; j < side; j++) {
      for (let i = 0; i < side; i++) {
        const wx = tx + i - CLUTTER_RING;
        const wz = tz + j - CLUTTER_RING;
        const hit = keep.get(wx * 100003 + wz);
        if (hit && hit.tx === wx && hit.tz === wz) {
          fresh.push(hit);
          reused.add(hit);
        } else {
          fresh.push(null);
        }
      }
    }
    // Recycle the slots that fell out of the ring into the holes.
    const spare = this.tiles.filter((t) => !reused.has(t));
    this.pending.length = 0;
    for (let i = 0; i < fresh.length; i++) {
      if (fresh[i]) continue;
      const t = spare.pop();
      t.tx = tx + (i % side) - CLUTTER_RING;
      t.tz = tz + Math.floor(i / side) - CLUTTER_RING;
      t.n = 0;
      t.loaded = false;
      fresh[i] = t;
      this.pending.push(t);
    }
    this.tiles = fresh;
    this.dirty = true;
  }

  /** One tile's worth of worldgen, copied out of the bridge buffer. */
  _fill(tile) {
    const n = this.ex.terrain_fill_clutter(this.seed, tile.tx, tile.tz);
    if (n > 0) {
      // A fresh view every fill: wasm memory moves when it grows, and a held
      // view detaches — the boot bug that shipped green on 2026-07-31.
      const src = new Float32Array(
        this.ex.memory.buffer,
        this.ex.terrain_clutter_ptr(),
        n * CLUTTER_FLOATS,
      );
      tile.data.set(src);
    }
    tile.n = Math.min(n, CLUTTER_TILE_CAP);
    tile.loaded = true;
    this.dirty = true;
  }

  /**
   * Rewrite every pool's instance stream from the tile cache. Bounded by
   * `CLUTTER_POOL_CAP` per kind, and a kind that overflows drops the rest
   * LOUDLY rather than silently — a cap nobody is told about reads as
   * coverage that was never there.
   */
  _writeInstances() {
    const counts = [0, 0, 0, 0];
    let dropped = 0;
    for (const tile of this.tiles) {
      if (!tile.loaded) continue;
      const d = tile.data;
      for (let i = 0; i < tile.n; i++) {
        const b = i * CLUTTER_FLOATS;
        const k = (d[b] | 0) - 1;
        if (k < 0 || k >= this.pools.length) continue;
        const at = counts[k];
        if (at >= CLUTTER_POOL_CAP) {
          dropped++;
          continue;
        }
        this._p.set(d[b + 1], d[b + 2], d[b + 3]);
        this._q.setFromAxisAngle(this._up, (d[b + 4] / 256) * Math.PI * 2);
        const s = d[b + 5];
        this._s.set(s, s, s);
        this._m.compose(this._p, this._q, this._s);
        this.pools[k].setMatrixAt(at, this._m);
        // A per-instance value jitter off the element's own yaw byte — the
        // same bits, reused, so it costs no draw and no wire. A tuft field at
        // one exact green is the thing that reads as a printed texture no
        // matter how much geometry is in it.
        const t = 1 + (d[b + 4] / 255 - 0.5) * 2 * CLUTTER_ROWS[k].tint;
        this.pools[k].setColorAt(at, this._c.setRGB(t, t, t));
        counts[k] = at + 1;
      }
    }
    for (let k = 0; k < this.pools.length; k++) {
      this.pools[k].count = counts[k];
      this.pools[k].instanceMatrix.needsUpdate = true;
      if (this.pools[k].instanceColor) this.pools[k].instanceColor.needsUpdate = true;
    }
    if (dropped > 0) {
      console.warn(
        `clutter: dropped ${dropped} elements at the pool cap (${CLUTTER_POOL_CAP}/kind)`,
      );
    }
  }

  /** Live instance counts per kind — the budget evidence `ci/` reads. */
  stats() {
    return {
      kinds: CLUTTER_ROWS.map((r, k) => ({ name: r.name, count: this.pools[k].count })),
      total: this.pools.reduce((a, m) => a + m.count, 0),
      loaded: this.tiles.filter((t) => t.loaded).length,
    };
  }

  dispose() {
    for (const m of this.pools) {
      this.scene.remove(m);
      m.geometry.dispose();
      m.material.dispose();
      m.dispose();
    }
    this.pools.length = 0;
  }
}
