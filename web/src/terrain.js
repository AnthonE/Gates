// Terrain streaming (TERRAIN.md §4, cut for M0): one far mesh of the
// whole island at 8 m built once, plus a ring of 64 m near chunks at 1 m
// streamed around the player — one build in flight, one teardown per
// frame (stream-out is budgeted too; the trap list's forgotten half).
// Adjacent near chunks share exact edge heights, so same-LOD seams don't
// crack; the far mesh sits 0.15 m under the near ring to cover the
// near↔far boundary until the LOD-skirt pass.
//
// Shadows: BOTH meshes cast. The far mesh casts through the depth material
// in shadows.js, which discards the near ring's own footprint, so each XZ
// column of the world has exactly one caster and the two LODs never put
// disagreeing silhouettes of one hillside into the same map. This file owns
// the footprint, because it owns the ring.
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
import {
  albedoLuma,
  makeTerrainCostVariants,
  makeTerrainMaterial,
  makeTerrainProjectionVariant,
  makeWindDepthMaterial,
  setWindTime,
  surfaceMaterial,
  windFacts,
  windWeight,
} from "./materials.js";
import {
  castInLevels,
  makeFarCasterDepthMaterial,
  FAR_CASTER_MIN_LEVEL,
  NEAR_CASTER_MAX_LEVEL,
} from "./shadows.js";

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
function bakedGeometry(parts, wind) {
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
  // The wind cantilever, one weight per vertex (materials.js). Ramped over
  // the WHOLE archetype rather than per part, which is why it is a single
  // `{y0, y1}` here and not a band on each: the pine's trunk, skirt and crown
  // overlap in y, and three independently ramped parts would put a step in
  // the middle of one tree. Absent `wind` the array stays zero, and an
  // archetype whose weights are all zero is displaced by nothing — the rocks
  // and the ore take the same patch and never move.
  const wnd = new Float32Array(total);
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
      if (wind) {
        wnd[o + i] = windWeight((gp[i * 3 + 1] - wind.y0) / (wind.y1 - wind.y0));
      }
    }
    o += n;
    part.g.dispose();
  }
  const geo = new THREE.BufferGeometry();
  geo.setAttribute("position", new THREE.BufferAttribute(pos, 3));
  geo.setAttribute("normal", new THREE.BufferAttribute(nrm, 3));
  geo.setAttribute("color", new THREE.BufferAttribute(col, 3));
  geo.setAttribute("aWind", new THREE.BufferAttribute(wnd, 1));
  return geo;
}

// How far a canopy ring may be pulled IN toward the trunk, as a share of its
// own radius. The visual judge's ranked gap 1(c): "the silhouette is a clean
// polygon edge with no needle cards, no alpha detail, no sub-branch break-up",
// and a blind reader named "flat-shaded cones stacked on a cylinder trunk with
// visible facet edges and no sub-branch silhouette" in 3 of the 3 frames that
// show a tree. This is the cheapest thing that breaks that edge: each ring of
// each cone gets a deterministic per-vertex radial pull, so the outline is a
// ragged whorl instead of a circle, at zero extra vertices and zero extra
// draw calls.
//
// It only ever pulls IN, never out, and that is load-bearing rather than
// tasteful: the beach-spawn clearance in `DECISIONS.md` §open derives its 4 m
// from "the widest archetype the client draws is the tree cone at r 1.7 m x
// 1.1 max scale ~= 1.9 m", so a canopy that could grow would invalidate a
// spoken sim number from the renderer. A canopy that can only shrink cannot.
const PINE_RAGGED = 0.34;

/**
 * A conifer: bare trunk, dark lower skirt, lighter crown — the cones' rings
 * pulled in per vertex so the outline is a whorl and not a circle.
 *
 * Deterministic by construction (the hash is over the vertex's own integer ring
 * and segment index), so every pine in the world is this pine and a chunk that
 * streams out and back is bit-identical — the same discipline `instanceTint`
 * carries for colour.
 */
function raggedCone(geo, seed) {
  const pos = geo.attributes.position;
  for (let i = 0; i < pos.count; i++) {
    const x = pos.getX(i);
    const z = pos.getZ(i);
    const r = Math.hypot(x, z);
    if (r < 1e-4) continue; // the apex and the cap centre stay put
    // A hash over the vertex's quantized angle and height, so the two vertices
    // a shared edge is built from get the same pull and the seam cannot open.
    const th = Math.atan2(z, x);
    const a = Math.round(((th < 0 ? th + Math.PI * 2 : th) / (Math.PI * 2)) * 4096) % 4096;
    const y = Math.round(pos.getY(i) * 512);
    let hh = Math.imul(a ^ (y * 0x9e3779b9) ^ seed, 0x85ebca6b);
    hh ^= hh >>> 13;
    hh = Math.imul(hh, 0xc2b2ae35);
    hh ^= hh >>> 16;
    const k = 1 - ((hh >>> 8) & 0x3ff) / 1023 * PINE_RAGGED;
    pos.setX(i, x * k);
    pos.setZ(i, z * k);
  }
  pos.needsUpdate = true;
  geo.computeVertexNormals();
  return geo;
}

// The pine's three colour bands, hoisted out of the geometry builder so the
// albedo law can be asserted over them (`scatterFacts`) instead of over a hex
// buried in a call argument. Trunk and skirt were rescaled by prop albedo v1 —
// both sat under `ALBEDO_LUMA_BAND`'s floor, the trunk by 1.9x — and the
// rescale is a linear multiply of both ends of each ramp, so every hue and
// every ramp shape here is the one materials v0 authored.
const PINE_BANDS = [
  { part: "trunk", lo: 0x503b2b, hi: 0x70583f, y0: 0, y1: 1.7 },
  // The skirt's lo is one green level above the exact rescale: 0x204725 is
  // 0.0495 after 8-bit quantization and the band's floor is 0.05, so the
  // authored hex — not the arithmetic that produced it — is what has to clear
  // it. The gate scores the hex.
  { part: "skirt", lo: 0x204825, hi: 0x3c783d, y0: 1.1, y1: 4.2 },
  { part: "crown", lo: 0x2c5c2c, hi: 0x4d8845, y0: 2.7, y1: 5.2 },
];

function pineGeometry() {
  const trunk = new THREE.CylinderGeometry(0.16, 0.24, 1.7, 6, 1, true);
  trunk.translate(0, 0.85, 0);
  // Nine segments, up from seven, and still ONE height ring. Nine gives the
  // pull nine independent silhouette events per cone where seven gives seven;
  // a second ring would give it more, and costs triangles this frame does not
  // have. `browser_smoke` asserts DESIGN §9's 1.5 M over the main pass plus
  // every shadow level, and the peak measured before this pass was 1.06 M —
  // 29% of headroom, which a canopy tessellation is not the right thing to
  // spend. 40 -> 48 triangles a pine is what this costs; three height rings
  // would have been 108.
  const skirt = new THREE.ConeGeometry(1.7, 3.1, 9);
  skirt.translate(0, 2.65, 0);
  const crown = new THREE.ConeGeometry(1.15, 2.5, 9);
  crown.translate(0, 3.95, 0);
  const geos = [trunk, raggedCone(skirt, 0x51ed270b), raggedCone(crown, 0x2545f491)];
  return bakedGeometry(
    PINE_BANDS.map((b, i) => ({ ...b, geo: geos[i] })),
    PINE_WIND,
  );
}

// The pine's wind band: base of the trunk to the tip of the crown (3.95 +
// 2.5/2). One ramp for all three parts — see `bakedGeometry`.
const PINE_WIND = { y0: 0, y1: 5.2 };

/**
 * What a felled pine leaves behind.
 *
 * `TERRAIN.md` §2 has said "Trees fell to stumps the same way" since before
 * there was a client to draw one, and until this slice a chopped tree set its
 * instance scale to 0 and was simply gone — the world's memory of the last ten
 * swings was nothing at all. The stump is the memory: it stands for the whole
 * 20–45 minute respawn window and vanishes when the tree comes back, so a
 * cleared ridge still reads as cleared an hour later.
 *
 * Six sides and both caps is 24 triangles, and it only exists where a tree
 * came down, so the pool is bounded by the felled trees inside the near ring
 * rather than by the forest. Its ramp is the pine trunk's own, which puts the
 * pale end of the bark ramp on the cut face — a stump is lightest where the
 * axe left it.
 */
function stumpGeometry() {
  return bakedGeometry([
    {
      part: "stump",
      geo: new THREE.CylinderGeometry(0.26, 0.32, 0.34, 6),
      lo: PINE_BANDS[0].lo,
      hi: PINE_BANDS[0].hi,
      y0: -0.17,
      y1: 0.17,
    },
  ]);
}

// Scatter archetypes, indexed by sim-core Occupant (1..7). `surface` names
// an authored PBR response in materials.js; `tint` is the amplitude of the
// deterministic per-instance colour variation (0 = every instance alike).
const ARCHETYPES = [
  null,
  { geo: pineGeometry, composite: true, wind: PINE_WIND, surface: "foliage", lo: 0xffffff, lift: 0, tint: 0.17 },
  { geo: () => new THREE.DodecahedronGeometry(1.0), surface: "rock", lo: 0x8f9399, lift: 0.5, tint: 0.11 },
  { geo: () => new THREE.DodecahedronGeometry(1.0), surface: "ore", lo: 0xa1785c, lift: 0.5, tint: 0.1 },
  { geo: () => new THREE.DodecahedronGeometry(1.0), surface: "ore", lo: 0xbfae4a, lift: 0.5, tint: 0.1 },
  { geo: () => new THREE.IcosahedronGeometry(0.7), surface: "foliage", lo: 0x2c5f2e, hi: 0x4b8a3f, y0: -0.7, y1: 0.7, wind: { y0: -0.7, y1: 0.7 }, lift: 0.45, tint: 0.19 },
  { geo: () => new THREE.DodecahedronGeometry(1.5), surface: "rock", lo: 0x75726d, lift: 0.55, tint: 0.12 },
  { geo: () => new THREE.CylinderGeometry(0.45, 0.45, 0.95, 10), surface: "metal", lo: 0x5e6b78, lift: 0.5, tint: 0.08 },
  // 8: the stump. The only archetype sim-core does not scatter — it is not a
  // slot occupant but the CONSEQUENCE of one, minted when a tree finishes
  // falling and retired when the slot respawns. It rides every other path a
  // scatter entry does (chunk lists, swap-remove on stream-out, the tint), so
  // the index has to be a real archetype rather than a special case bolted to
  // the side; `_addScatter` only ever reads occupants 1..7 out of the slot
  // buffer, so 8 cannot collide with one.
  { geo: stumpGeometry, composite: true, surface: "wood", lo: PINE_BANDS[0].lo, hi: PINE_BANDS[0].hi, lift: 0.17, tint: 0.11 },
];
/** Client-only archetype index for a felled pine's stump. */
const ARCH_STUMP = 8;
/** The archetype a fell animation applies to (sim-core `Occupant::Tree`). */
const ARCH_TREE = 1;
/**
 * Every authored colour band an archetype bakes, as `{part, lo, hi}`.
 *
 * The pine is the only archetype built from more than one part, and its bands
 * live in `PINE_BANDS` because the geometry builder and the albedo gate both
 * need them. Everything else carries its ramp on the archetype row itself.
 */
function albedoParts(k) {
  const a = ARCHETYPES[k];
  if (!a) return [];
  return a.geo === pineGeometry ? PINE_BANDS : [{ part: a.surface, lo: a.lo, hi: a.hi }];
}

const POOL_CAP = 4096;
const YAW8_TO_RAD = (Math.PI * 2) / 256;

// --- felling ---------------------------------------------------------------
//
// The sim has owned every part of chopping a tree for a long time: ten swings
// with the right tool, a yield table per tool row, a harvested bit and a
// 20–45 minute respawn timer, all of it authoritative and all of it on the
// wire as two event codes (`gather.rs`, `EV_SLOT_HARVESTED`). The client's
// entire contribution was `scale = 0`. The tenth swing landed and the tree was
// simply not there any more — no fall, no direction, no sound the eye could
// see, and the most physical verb in the game read as a rendering bug.
//
// So this is the missing half and nothing more: no new sim state, no new wire
// byte, no protocol version. It is an animation the client already had all the
// information to play, and the only interesting question is where the fall
// DIRECTION comes from, because two players watching the same tree must see it
// land the same way or the world stops being shared.
//
// It comes from the cell's own hash — the same hash `instanceTint` derives a
// tree's colour from — so it is a pure function of a fact both clients already
// have, and it costs zero bytes. The alternative is honest and rejected: the
// direction a chopper is standing in is genuinely better (a tree should fall
// away from the axe), the sim knows it, and putting it in `EV_SLOT_HARVESTED`'s
// spare `b` bits would cost a `PROTO_VER` bump and regenerated goldens under
// wall 6. That is a real slice; it is not this one.

/** Ticks from the last swing to flat on the ground (30 Hz). */
const FELL_TICKS = 33;
/** Ticks the felled trunk then takes to sink out of the world. */
const FELL_SINK_TICKS = 60;
/** How far it sinks, in metres of instance scale. */
const FELL_SINK_M = 2.4;
const HALF_PI = Math.PI * 0.5;

/**
 * The bearing this cell's tree falls along, radians.
 *
 * A different mix of the cell hash than `instanceTint` takes, so a tree that
 * happens to be pale does not also happen to fall north.
 */
function fellBearing(cellKey) {
  let h = Math.imul(cellKey ^ 0x7f4a7c15, 0x2545f491);
  h ^= h >>> 15;
  h = Math.imul(h, 0x27d4eb2f);
  h ^= h >>> 16;
  return ((h >>> 9) & 0x3ff) * ((Math.PI * 2) / 1024);
}

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
  constructor(scene, seed, ex, wasmUrl, clipmap) {
    this.scene = scene;
    // The shadow clipmap caches its coarse levels, so a chunk that streams in
    // or out has to say so or it casts nothing until the cache ages out
    // (shadows.js `invalidate`, the reference's "streamed terrain arrival").
    this.clipmap = clipmap || null;
    this.seed = seed;
    this.ex = ex; // main-thread wasm: slot queries only (fast, tiny)
    this.material = makeTerrainMaterial();
    // Built on the cost probe's first ask and never on a public shard.
    this._costVariants = null;
    // …and the projection probe's partner, on the same terms.
    this._projectionVariant = null;
    this.chunks = new Map(); // "cx,cz" -> { cx, cz, mesh? , pending? }
    this.queue = [];
    this.inFlight = false;
    this.teardown = [];
    this.farBuilt = false;
    this.farMesh = null;
    // The horizon's depth pass. `uNearHole` is the near ring's footprint,
    // republished by _updateHole() whenever the ring gains or loses a chunk;
    // the Vector4 is held directly so the update writes four numbers and
    // allocates nothing.
    this.farDepth = makeFarCasterDepthMaterial();
    this.farHole = this.farDepth.userData.uniforms.uNearHole.value;
    // The near chunks' depth material, explicit for the same reason the far
    // mesh's is: three hands every plain caster ONE shared MeshDepthMaterial
    // and sets `side` per object — but its recompile check never looks at
    // `side`, so whichever caster renders first owns the program and the
    // ground's FrontSide fix (materials.js — a heightfield has no back
    // faces) only takes effect when an unrelated invalidation forces a
    // relink. Until that race resolves, chunks shadow-render FLIPPED and
    // hills cast nothing. An owned instance never flips, compiles once, and
    // is prewarmable (prewarmObjects below).
    this.nearDepth = new THREE.MeshDepthMaterial({
      depthPacking: THREE.RGBADepthPacking,
      side: THREE.FrontSide,
    });
    // And the swaying casters' own, for the sharper version of the same
    // problem: three's shared depth material carries no wind, so a pine
    // patched only in its surface material would lean out from under a shadow
    // that stayed bolted to the ground. One material for every wind archetype
    // — one program, prewarmed below.
    this.windDepth = makeWindDepthMaterial();
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
      // `composite` archetypes bake their own parts (and their own wind band);
      // everything else is one primitive wearing the row's ramp.
      const geo = a.composite
        ? a.geo()
        : bakedGeometry([{ ...a, geo: a.geo() }], a.wind);
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
      // …and it casts the shape the wind actually put there.
      if (a.wind) mesh.customDepthMaterial = this.windDepth;
      // A pine only exists inside the near ring, so every pixel it can darken
      // is inside level 1's box (shadows.js, "which level gets which caster").
      castInLevels(mesh, 0, NEAR_CASTER_MAX_LEVEL);
      scene.add(mesh);
      this.pools.push(mesh);
      this.owners.push([]);
    }
    this.chunkSlots = new Map(); // key -> [{arch, idx, key, cellKey, …}]
    this.cellIndex = new Map(); // cellKey (cx<<16|cz) -> entry
    // The two program families this terrain otherwise links MID-PLAY, built
    // as dummies for the scene's prewarm group (scene.prewarm): a count-1
    // instanced sample — the pools above boot at count 0, never enter a
    // render list, and so defeat their own pre-allocated colour buffer until
    // the first tree streams in — and a far-caster sample wearing the custom
    // depth material, which otherwise links when the far mesh first casts.
    // The group carries them through boot and the first in-world shadow
    // pass, then disposes them.
    this.prewarmObjects = () => {
      const objs = [];
      const pool = this.pools.find(Boolean);
      if (pool) {
        const m = new THREE.InstancedMesh(
          new THREE.PlaneGeometry(0.02, 0.02),
          pool.material,
          1,
        );
        m.setColorAt(0, this._c.setRGB(1, 1, 1));
        m.castShadow = true;
        m.frustumCulled = false;
        objs.push(m);
        // The wind caster's depth program, which is a THIRD family and links
        // the first time a swaying archetype reaches a shadow pass — i.e. on
        // the first frame there is a tree in the ring, which is the frame the
        // player arrives. Prewarmed here so the count gate stays flat.
        const w = new THREE.InstancedMesh(
          new THREE.PlaneGeometry(0.02, 0.02),
          pool.material,
          1,
        );
        w.setColorAt(0, this._c.setRGB(1, 1, 1));
        w.customDepthMaterial = this.windDepth;
        w.castShadow = true;
        w.frustumCulled = false;
        objs.push(w);
      }
      const far = new THREE.Mesh(
        new THREE.PlaneGeometry(0.02, 0.02),
        this.material,
      );
      far.customDepthMaterial = this.farDepth;
      far.castShadow = true;
      far.frustumCulled = false;
      objs.push(far);
      const near = new THREE.Mesh(
        new THREE.PlaneGeometry(0.02, 0.02),
        this.material,
      );
      near.customDepthMaterial = this.nearDepth;
      near.castShadow = true;
      near.frustumCulled = false;
      objs.push(near);
      return objs;
    };
    this._m4 = new THREE.Matrix4();
    this._q = new THREE.Quaternion();
    this._e = new THREE.Euler();
    this._v = new THREE.Vector3();
    this._s = new THREE.Vector3();
    // The fell's two extra scratch objects, preallocated with the rest: a
    // falling tree rewrites its matrix every frame, and the RAF path allocates
    // nothing (DESIGN.md L8).
    this._qf = new THREE.Quaternion();
    this._axis = new THREE.Vector3();
    // Trees mid-fall. Small by construction — a player can only be swinging
    // at one — and swept in `update`, which is also the only thing that walks
    // it, so a swap-remove is enough.
    this._felling = [];
    // cellKey -> the stump entry standing in for a felled tree. Deliberately
    // NOT `cellIndex`, which maps a cell to its TREE: both live at the same
    // cell for the whole respawn window and one map cannot hold both.
    this.stumpIndex = new Map();
    // Sim ticks, published by `update` from the render view. The client's one
    // animation clock: see materials.js `windUniform`.
    this._tick = 0;

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
    // Shadows: both LODs cast and both receive. The far mesh pays for it with
    // a custom depth material that discards the near ring's footprint, so the
    // 131 k triangles it costs a shadow level buy the horizon rather than a
    // second, disagreeing copy of ground the near ring already casts.
    mesh.receiveShadow = true;
    mesh.castShadow = true;
    if (msg.key === "far") {
      mesh.position.set(0, -0.15, 0);
      mesh.customDepthMaterial = this.farDepth;
      castInLevels(mesh, FAR_CASTER_MIN_LEVEL, Infinity);
      this.farBuilt = true;
      this.farMesh = mesh;
      this.scene.add(mesh);
      // The horizon just appeared in the coarse levels' caster set.
      this._invalidateShadows(mesh);
    } else {
      const entry = this.chunks.get(msg.key);
      if (!entry || !entry.pending) {
        geo.dispose(); // unloaded while building
      } else {
        mesh.position.set(msg.x0, 0, msg.z0);
        mesh.customDepthMaterial = this.nearDepth;
        castInLevels(mesh, 0, NEAR_CASTER_MAX_LEVEL);
        this.scene.add(mesh);
        entry.pending = false;
        entry.mesh = mesh;
        this._addScatter(msg.key, msg.x0, msg.z0);
        this._invalidateShadows(mesh);
        this._updateHole();
      }
    }
    this._kick();
  }

  /**
   * Tell the clipmap a chunk's worth of casters arrived or left. The sphere
   * is the geometry's own bounding sphere in world space — measured, not a
   * guessed radius, and the scatter standing on the chunk shares its
   * footprint. Stream-in/out only: never the steady-state RAF path.
   */
  _invalidateShadows(mesh) {
    if (!this.clipmap) return;
    const geo = mesh.geometry;
    if (!geo.boundingSphere) geo.computeBoundingSphere();
    const s = geo.boundingSphere;
    this.clipmap.invalidate(
      mesh.position.x + s.center.x,
      mesh.position.y + s.center.y,
      mesh.position.z + s.center.z,
      s.radius,
    );
  }

  /**
   * Republish the near ring's footprint to the far caster, and tell the
   * clipmap that the horizon's silhouette moved.
   *
   * The hole is the box of the near chunks that are actually LOADED, not the
   * ones that have been asked for. Two reasons, and they pull opposite ways,
   * which is why it is the loaded set and not either bound:
   *
   *   * A chunk still in the build queue casts nothing. Punching its column
   *     out of the far mesh the moment it is queued would leave that ground
   *     unshadowed for as long as the queue takes — the one gap direction
   *     that shows.
   *   * Stream-out runs on a radius+1 hysteresis, so chunks live one ring
   *     past the desired set and every one of them casts. If the hole were
   *     the desired 5x5, that outer ring would be a double caster against the
   *     far mesh — the exact acne this whole arrangement exists to avoid.
   *
   * The loaded box satisfies both by construction: every caster the ring has
   * is inside it, and nothing inside it is missing a caster except the few
   * frames a corner chunk is in flight.
   *
   * Called on chunk arrival and teardown only, never the steady-state RAF
   * path: the box cannot move while the ring does not change. Writes four
   * numbers into a held Vector4 and allocates nothing (L8).
   */
  _updateHole() {
    let minCx = Infinity;
    let minCz = Infinity;
    let maxCx = -Infinity;
    let maxCz = -Infinity;
    for (const entry of this.chunks.values()) {
      if (!entry.mesh) continue;
      if (entry.cx < minCx) minCx = entry.cx;
      if (entry.cz < minCz) minCz = entry.cz;
      if (entry.cx > maxCx) maxCx = entry.cx;
      if (entry.cz > maxCz) maxCz = entry.cz;
    }
    let cx = 0;
    let cz = 0;
    let hx = 0;
    let hz = 0;
    if (minCx <= maxCx) {
      hx = ((maxCx - minCx + 1) * CHUNK) / 2;
      hz = ((maxCz - minCz + 1) * CHUNK) / 2;
      cx = minCx * CHUNK + hx;
      cz = minCz * CHUNK + hz;
    }
    const h = this.farHole;
    if (h.x === cx && h.y === cz && h.z === hx && h.w === hz) return;
    h.set(cx, cz, hx, hz);
    // A cached coarse level is holding the far mesh drawn against the OLD
    // hole, which is now wrong over the whole box. The per-chunk
    // invalidation above covers the chunk that arrived; this covers the
    // shape the arrival changed, which is a different thing.
    if (this.clipmap && hx > 0) {
      this.clipmap.invalidate(cx, 0, cz, Math.sqrt(hx * hx + hz * hz));
    }
  }

  /**
   * The ground's cost variants, built once on first ask (NOW.md item 1).
   *
   * Terrain owns them because Terrain owns the material and the meshes that
   * wear it; `GameScene.costProbe` borrows them the same way it borrows the
   * splat uniforms. Dev-only by construction — nothing calls this but the
   * probe, and the probe is not installed on a public shard.
   */
  costVariants() {
    if (!this._costVariants) {
      this._costVariants = makeTerrainCostVariants();
      // The shipped material is one of the four by name; hand back the LIVE
      // one for "ship" so the probe's baseline is the program the frame has
      // been rendering, not a second copy of it.
      this._costVariants = this._costVariants.map((m) =>
        m.userData.cost.variant === "ship" ? this.material : m,
      );
    }
    return this._costVariants;
  }

  /**
   * The grain octave's projection partner, built on first ask (materials v1,
   * third pass). Same ownership argument as `costVariants()` and the same
   * dev-only-by-construction guarantee: nothing but `projectionProbe` calls
   * this and the probe is not installed on a public shard.
   */
  projectionVariant() {
    if (!this._projectionVariant) {
      this._projectionVariant = makeTerrainProjectionVariant();
    }
    return this._projectionVariant;
  }

  /**
   * The steepest coherent FACE of ground near (px, pz) — dev-only, and the
   * thing the projection probe has to be aimed at.
   *
   * A grain that is combed downhill is only combed on a slope, so the gate
   * that measures it needs a slope to measure ON, and "the slope near spawn"
   * is a worldgen fact nobody may hardcode: a seed change would silently aim
   * the probe at a meadow and every measurement after it would pass by
   * default. So this finds one, and reports enough about what it found that
   * the gate can refuse a face that is not steep enough to be evidence.
   *
   * Vertices inside `radiusM` are binned on a `binM` world grid and each bin's
   * position and normal are averaged. A bin is a FACE and not a spike because
   * of two reported numbers the gate asserts on: `verts`, how many vertices
   * agreed to make it, and `coherence` — the mean normal's LENGTH, which is 1
   * only if every normal in the bin pointed the same way and collapses toward
   * 0 on a ridge line or a crumpled patch.
   *
   * Deterministic on purpose. Chunks stream in in whatever order the worker
   * finishes them, so the accumulation walks them in sorted key order, and
   * candidate bins are ranked with their key as the tie-break — otherwise the
   * face the gate lands on could differ between two runs of one build, which
   * is the class of flake this box has already paid for twice.
   *
   * `sun` and `minLit` are a third requirement, and not a cosmetic one: a face
   * is only evidence if the frame taken of it carries the octave, and the
   * octave reaches the image as a swing in ALBEDO. On a face turned away from a
   * 21°-elevation key light that swing falls under the probe's own luma
   * threshold and the measurement is taken over a few hundred stray pixels —
   * which is what the steepest face near this spawn turned out to be, and it
   * cost a run to find out. So a candidate must face the light by `minLit`
   * (`dot(n, sun)`, the lambert term), and the steepest of the ones that do is
   * what comes back.
   *
   * Allocates a Map and scans every near vertex: never the RAF path.
   */
  steepestFace(px, pz, radiusM, binM, minVerts, sun, minLit) {
    const bins = new Map();
    const r2 = radiusM * radiusM;
    let scanned = 0;
    let chunks = 0;
    for (const key of [...this.chunks.keys()].sort()) {
      const entry = this.chunks.get(key);
      if (!entry || !entry.mesh) continue;
      chunks++;
      const geo = entry.mesh.geometry;
      const pos = geo.getAttribute("position");
      const nor = geo.getAttribute("normal");
      if (!pos || !nor) continue;
      const ox = entry.mesh.position.x;
      const oy = entry.mesh.position.y;
      const oz = entry.mesh.position.z;
      for (let i = 0; i < pos.count; i++) {
        const x = pos.getX(i) + ox;
        const z = pos.getZ(i) + oz;
        const dx = x - px;
        const dz = z - pz;
        if (dx * dx + dz * dz > r2) continue;
        scanned++;
        const bk = `${Math.floor(x / binM)},${Math.floor(z / binM)}`;
        let b = bins.get(bk);
        if (!b) {
          b = { key: bk, n: 0, x: 0, y: 0, z: 0, nx: 0, ny: 0, nz: 0 };
          bins.set(bk, b);
        }
        b.n++;
        b.x += x;
        b.y += pos.getY(i) + oy;
        b.z += z;
        b.nx += nor.getX(i);
        b.ny += nor.getY(i);
        b.nz += nor.getZ(i);
      }
    }
    const cands = [];
    let unlit = 0;
    for (const b of bins.values()) {
      if (b.n < minVerts) continue;
      const len = Math.hypot(b.nx, b.ny, b.nz);
      if (len <= 0) continue;
      const n = [b.nx / len, b.ny / len, b.nz / len];
      const lit = sun ? n[0] * sun[0] + n[1] * sun[1] + n[2] * sun[2] : 1;
      if (lit < (minLit || 0)) {
        unlit++;
        continue;
      }
      cands.push({
        key: b.key,
        center: [b.x / b.n, b.y / b.n, b.z / b.n],
        normal: n,
        upness: n[1],
        lit,
        coherence: len / b.n,
        verts: b.n,
      });
    }
    cands.sort((a, c) => a.upness - c.upness || (a.key < c.key ? -1 : 1));
    const best = cands[0] || null;
    return {
      found: !!best,
      chunks,
      scanned,
      bins: bins.size,
      candidates: cands.length,
      unlit,
      radiusM,
      binM,
      minVerts,
      minLit: minLit || 0,
      ...(best
        ? {
            key: best.key,
            center: best.center,
            normal: best.normal,
            upness: best.upness,
            slopeDeg: (Math.acos(Math.min(1, Math.max(-1, best.upness))) * 180) / Math.PI,
            lit: best.lit,
            coherence: best.coherence,
            verts: best.verts,
          }
        : {}),
    };
  }

  /**
   * Put `material` on every piece of ground — both LODs, every streamed
   * chunk. Probe-only: the shipped material is restored by the same call.
   *
   * The far mesh's `customDepthMaterial` is left alone on purpose. A variant
   * changes what the ground COSTS to shade, and the depth pass shades
   * nothing; swapping it too would move the shadow maps and make the timing
   * a measurement of two things.
   */
  useMaterial(material) {
    for (const entry of this.chunks.values()) {
      if (entry.mesh) entry.mesh.material = material;
    }
    if (this.farMesh) this.farMesh.material = material;
  }

  /**
   * The far caster's state, for the browser gate: whether the horizon casts
   * at all, which side the ground casts from, and the hole + skirt that keep
   * the two LODs out of each other's map.
   */
  farCasterFacts() {
    const h = this.farHole;
    let loaded = 0;
    for (const entry of this.chunks.values()) if (entry.mesh) loaded++;
    return {
      built: this.farBuilt,
      casts: !!(this.farMesh && this.farMesh.castShadow),
      customDepth: !!(this.farMesh && this.farMesh.customDepthMaterial),
      // FrontSide (0). three's default for a FrontSide material is to cast
      // from the BACK face, which culls a heightfield out of the depth pass
      // entirely — the ground cast nothing at all before this slice.
      shadowSide: this.material.shadowSide,
      triangles: this.farMesh ? this.farMesh.geometry.index.count / 3 : 0,
      sinkM: this.farDepth.userData.uniforms.uCasterSink.value,
      holeCenter: [h.x, h.y],
      holeHalf: [h.z, h.w],
      nearChunks: loaded,
      chunkM: CHUNK,
      // The per-level caster policy, so the gate can assert the two LODs are
      // split across the levels rather than both submitted to all of them.
      farMinLevel: FAR_CASTER_MIN_LEVEL,
      nearMaxLevel: NEAR_CASTER_MAX_LEVEL,
    };
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
        // Streaming in never animates. A tree that was already down when the
        // chunk arrived was felled minutes ago and possibly by someone else —
        // replaying its fall on arrival would have the forest collapsing
        // around anyone who walked back into it.
        fellAt: 0,
      };
      pool.count = idx + 1;
      this._composeEntry(entry);
      this._composeTint(entry);
      this.owners[arch][idx] = entry;
      this.cellIndex.set(entry.cellKey, entry);
      list.push(entry);
    }
    this.chunkSlots.set(key, list);
    // Stumps after the list exists, because `_setStump` appends to it — hence
    // the captured length, so this walks the slots the chunk arrived with and
    // not the stumps it is growing behind itself.
    const n = list.length;
    for (let i = 0; i < n; i++) {
      const entry = list[i];
      if (entry.arch === ARCH_TREE && entry.hidden) this._setStump(entry, true);
    }
  }

  /**
   * Write an entry's matrix — scale 0 while its node is harvested, and the
   * fall pose while it is coming down.
   *
   * The tilt is applied about the instance ORIGIN, which for a pine is the
   * base of its trunk (`lift: 0`, and `pineGeometry` builds from y = 0 up), so
   * a rotation here is already a rotation about the stump. That is why felling
   * needs no pivot offset and no second matrix: the geometry was authored
   * standing on its own origin.
   *
   * `premultiply` and not `multiply`, because the fall bearing is a WORLD
   * direction and the yaw is the instance's own — tilt after yaw, or every
   * tree falls along a bearing its own random rotation picked for it.
   */
  _composeEntry(entry) {
    const pool = this.pools[entry.arch];
    this._e.set(0, entry.yaw8 * YAW8_TO_RAD, 0);
    this._q.setFromEuler(this._e);
    this._v.set(entry.x, entry.y, entry.z);
    const s = entry.hidden ? 0 : entry.scale;
    if (entry.fellAt) {
      const e = this._tick - entry.fellAt;
      const u = Math.min(e / FELL_TICKS, 1);
      // u^2: a tree comes off the cut slowly and arrives fast, because that is
      // what gravity does to a hinge. Linear reads as a door closing.
      this._axis.set(Math.sin(entry.fellDir), 0, -Math.cos(entry.fellDir));
      this._qf.setFromAxisAngle(this._axis, HALF_PI * u * u);
      this._q.premultiply(this._qf);
      if (e > FELL_TICKS) {
        // v^2 again, the other way up: the trunk lies there for most of a
        // second before it starts going, which is the beat that makes it read
        // as a felled tree rather than a despawn. One easing, no second knob.
        const v = Math.min((e - FELL_TICKS) / FELL_SINK_TICKS, 1);
        this._v.y -= FELL_SINK_M * entry.scale * v * v;
      }
    }
    this._s.set(s, s, s);
    this._m4.compose(this._v, this._q, this._s);
    pool.setMatrixAt(entry.idx, this._m4);
    pool.instanceMatrix.needsUpdate = true;
  }

  /**
   * Stand a stump at a felled tree's cell, or take it away again.
   *
   * The stump is an ordinary scatter entry in every respect that matters —
   * it joins the chunk's own list, so a chunk that streams out takes its
   * stumps with it through the same swap-remove every tree uses, and one that
   * streams back in re-derives them from the harvested set.
   */
  _setStump(entry, on) {
    const have = this.stumpIndex.get(entry.cellKey);
    if (on === !!have) return;
    const pool = this.pools[ARCH_STUMP];
    if (on) {
      if (pool.count >= POOL_CAP) return;
      const stump = {
        arch: ARCH_STUMP,
        idx: pool.count,
        key: entry.key,
        cellKey: entry.cellKey,
        x: entry.x,
        y: entry.y,
        z: entry.z,
        yaw8: entry.yaw8,
        // A stump is the tree's own trunk, so it wears the tree's own scale.
        scale: entry.scale,
        hidden: false,
        fellAt: 0,
      };
      pool.count = stump.idx + 1;
      this._composeEntry(stump);
      this._composeTint(stump);
      this.owners[ARCH_STUMP][stump.idx] = stump;
      this.stumpIndex.set(entry.cellKey, stump);
      const list = this.chunkSlots.get(entry.key);
      if (list) list.push(stump);
      return;
    }
    this._dropEntry(have);
    const list = this.chunkSlots.get(have.key);
    if (list) {
      const at = list.indexOf(have);
      if (at >= 0) list.splice(at, 1);
    }
  }

  /**
   * Swap-remove one entry from its pool, fixing up whatever entry moved into
   * its slot. Lifted out of `_removeScatter` so a stump can be retired on its
   * own — a respawn takes one stump away while its chunk stays live.
   */
  _dropEntry(entry) {
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
    entry.gone = true;
    if (entry.arch === ARCH_STUMP) this.stumpIndex.delete(entry.cellKey);
    else this.cellIndex.delete(entry.cellKey);
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

  /**
   * Event-lane fact: the node at this cell vanished or came back.
   *
   * A tree does not vanish — it is felled, which takes `FELL_TICKS` and ends
   * with `hidden` set by `_stepFells`. Everything else still goes instantly,
   * because a rock that stops being a rock has nothing to animate.
   */
  setCellHarvested(cellKey, harvested) {
    const entry = this.cellIndex.get(cellKey);
    if (!entry) return;
    // "Down" is hidden OR on its way down. Comparing `hidden` alone would make
    // a respawn that lands mid-fall a no-op — the event would be dropped as
    // redundant, the fall would run to its end, and the slot would stand
    // harvested on the client and standing on the server until the chunk
    // streamed. The respawn window is 20–45 minutes so nothing in play gets
    // there, but a resync can deliver one at any moment and this is a state
    // machine, not a race.
    const down = entry.hidden || entry.fellAt !== 0;
    if (down === harvested) return;
    if (harvested && entry.arch === ARCH_TREE && !entry.fellAt) {
      entry.fellAt = this._tick;
      entry.fellDir = fellBearing(cellKey);
      this._felling.push(entry);
      // The stump appears with the first degree of lean, not after the trunk
      // lands: the cut is what makes the tree fall, so the stump is already
      // there by the time anything moves.
      this._setStump(entry, true);
      return;
    }
    entry.hidden = harvested;
    if (!harvested) {
      entry.fellAt = 0;
      this._setStump(entry, false);
    }
    this._composeEntry(entry);
  }

  /** Sync reset: un-hide everything; the batch that follows re-hides. */
  resetHarvested() {
    for (const entry of this.cellIndex.values()) {
      if (entry.hidden || entry.fellAt) {
        entry.hidden = false;
        entry.fellAt = 0;
        this._setStump(entry, false);
        this._composeEntry(entry);
      }
    }
    this._felling.length = 0;
  }

  /**
   * Advance every tree mid-fall by one frame, retiring the ones that have
   * finished into a hidden trunk over a standing stump.
   */
  _stepFells() {
    for (let i = this._felling.length - 1; i >= 0; i--) {
      const entry = this._felling[i];
      // Three ways off this list, and only ONE of them ends with a hidden
      // trunk. `gone` is a chunk that streamed out from under the fall;
      // `fellAt === 0` is a respawn that overtook it (setCellHarvested), and
      // that tree is standing again — hiding it here would undo the event
      // that just saved it.
      const landed = this._tick - entry.fellAt >= FELL_TICKS + FELL_SINK_TICKS;
      if (entry.gone || !entry.fellAt || landed) {
        if (!entry.gone && entry.fellAt) {
          entry.hidden = true;
          entry.fellAt = 0;
          this._composeEntry(entry);
        }
        this._felling[i] = this._felling[this._felling.length - 1];
        this._felling.pop();
        continue;
      }
      this._composeEntry(entry);
    }
  }

  _removeScatter(key) {
    const list = this.chunkSlots.get(key);
    if (!list) return;
    // Highest index first so swap-with-last stays valid. One sort over the
    // union of every archetype in the chunk is enough, and stumps join it on
    // the same terms: any subsequence of a descending sequence is descending,
    // so per-pool order survives the interleave.
    for (const entry of list) {
      if (!entry.gone) this._dropEntry(entry);
    }
    this.chunkSlots.delete(key);
  }

  /**
   * Per-frame: advance the wind and any falling trees, retarget the near
   * ring, one build kick, one teardown.
   *
   * `tick` is the client's fixed 30 Hz sim tick, not `performance.now()`, and
   * that choice is the whole of this client's answer to `NOW.md` item 12. Wind
   * is the first animated thing in the frame, so it is also the first thing
   * that could make two captures of one seed differ; taking its clock from the
   * sim means a shot at tick N is the same shot every time it is taken, and a
   * capture harness gets determinism by pinning the tick it already pins.
   *
   * The cost of that choice is that wind moves in 33 ms steps. At the two
   * frequencies it runs (1.15 and 2.63 rad/s) the largest step is under a
   * millimetre at the tip of a pine, which is why this is free rather than a
   * trade — and it would NOT be free for anything fast, which is the line to
   * remember when leaf flutter arrives.
   */
  update(px, pz, tick) {
    this._tick = tick;
    setWindTime(tick * (1 / 30));
    if (this._felling.length) this._stepFells();
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
        this._invalidateShadows(entry.mesh);
        this.scene.remove(entry.mesh);
        entry.mesh.geometry.dispose();
        this._removeScatter(key);
        this.chunks.delete(key);
        this._updateHole();
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
        // Prop albedo v1. Every authored colour this archetype bakes into its
        // vertices, as the linear luminance the FRAGMENT multiplies — derived
        // through the renderer's own sRGB conversion rather than restated, so
        // a hex edited in the table above cannot pass by agreeing with a
        // number written beside it. The pine publishes three bands; everything
        // else is one part, and `hi` defaults to `lo` exactly as
        // `bakedGeometry` reads it.
        albedo: albedoParts(k).map((p) => ({
          part: p.part,
          lo: albedoLuma(p.lo),
          hi: albedoLuma(p.hi === undefined ? p.lo : p.hi),
        })),
      });
    }
    return out;
  }

  /**
   * The wind's knobs and live state, plus what this class owns about it: the
   * fell timings, whether the swaying pools cast through a wind-bearing depth
   * material, and how many trees are on their way down right now.
   *
   * `depthPatched` is the one worth a gate. A tree that sways while its shadow
   * stands still is the exact failure this slice was written around, and it is
   * invisible to every pixel assertion taken from the camera's own side.
   */
  windFacts() {
    const swaying = [];
    for (let k = 1; k < ARCHETYPES.length; k++) {
      if (ARCHETYPES[k] && ARCHETYPES[k].wind) swaying.push(k);
    }
    return {
      ...windFacts(),
      swaying,
      depthPatched: swaying.every(
        (k) => this.pools[k].customDepthMaterial === this.windDepth,
      ),
      tick: this._tick,
      fellTicks: FELL_TICKS,
      fellSinkTicks: FELL_SINK_TICKS,
      felling: this._felling.length,
      stumps: this.pools[ARCH_STUMP].count,
    };
  }

  /**
   * Dev-only: the nearest live instance of each scatter archetype to a point.
   *
   * The prop-surface probe has to photograph a PROP, and where the props are
   * is a worldgen fact this class owns — the gate cannot aim at one from a
   * hardcoded bearing without silently rotting the day the scatter weights
   * move. Terrain scans, the caller computes the eye, exactly the split
   * `steepestFace` already uses.
   *
   * ~4 k matrix reads per pool, so it is a gate hook and never a frame one.
   */
  nearestProps(px, pz, maxR) {
    const out = [];
    const m = new THREE.Matrix4();
    for (let k = 1; k < ARCHETYPES.length; k++) {
      const pool = this.pools[k];
      if (!pool || pool.count === 0) continue;
      let best = null;
      let bestD = maxR * maxR;
      for (let i = 0; i < pool.count; i++) {
        pool.getMatrixAt(i, m);
        const e = m.elements;
        const dx = e[12] - px;
        const dz = e[14] - pz;
        const d = dx * dx + dz * dz;
        if (d < bestD) {
          bestD = d;
          best = { pos: [e[12], e[13], e[14]], scale: m.getMaxScaleOnAxis() };
        }
      }
      if (best) {
        out.push({
          arch: k,
          surface: ARCHETYPES[k].surface,
          pos: best.pos,
          scale: best.scale,
          distance: Math.sqrt(bestD),
          count: pool.count,
        });
      }
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
