// The three.js scene shell (DESIGN.md §9): first-person camera at the
// predicted capsule's eye, remote players as capsule+nose groups keyed by
// id, sky/fog/light/water. All per-frame math goes through preallocated
// vectors — no allocations, no closures in the RAF path (L8).

import * as THREE from "three";
import { materialFacts, surfaceMaterial } from "./materials.js";

const EYE_HEIGHT = 1.6; // cosmetic (DECISIONS.md §open, client cosmetics)
const YAW_TO_RAD = (Math.PI * 2) / 65536;

// --- lighting v0 (DECISIONS.md §open, "lighting v0") ------------------------
// One key, one fill, one bounded shadow map, one tone map. The register is
// the spoken art direction — "Rust with a darker edge": a low warm sun so
// everything rakes, and a cold fill deliberately kept under the key so a
// shadow stays a shadow. Nothing here is a post stack; the only stages are
// light ratios → tone map → sRGB output.
//
// The tone map is Khronos PBR Neutral, not ACES, and that was measured
// rather than chosen: ACES's toe put the dark-albedo scatter (the 0x2f6b33
// pine) at ~20/255 on its shaded side, which is a crushed image, not a dark
// one. Neutral is identity below ~0.8 linear and only rolls the highlights
// off, so what darkens this scene is the shadow map, not the transfer.

// Where the sun sits. Azimuth is the compass bearing of the sun itself
// (0 = +Z, increasing toward +X, matching the sim's yaw); elevation is
// its angle above the horizon. Low, so shadows are long and read as shape.
const SUN_AZIMUTH = 2.35;
const SUN_ELEVATION = 0.36;
const SUN_COLOR = 0xffe1b8;
const SUN_INTENSITY = 3.0;
// The fill is sky-above / earth-below, cold over warm, and it is the whole
// ambient budget: a shadow lit only by this reads blue and stays dark.
const FILL_SKY = 0xa9c3e2;
const FILL_GROUND = 0x6b5f4a;
const FILL_INTENSITY = 1.15;
// One tone map, owned by the renderer. No material sets its own.
const EXPOSURE = 0.8;
// Fog and the sky dome share a horizon colour, so the seam is exact by
// construction rather than by tuning two numbers against each other.
const FOG_COLOR = 0x808f9c;
const FOG_NEAR = 180;
const FOG_FAR = 1000;
const SKY_ZENITH = 0x2c4463;
const SKY_CURVE = 0.62; // horizon→zenith ramp; <1 lifts the gradient early
const SKY_RADIUS = 10;

// The shadow map covers a bounded square around the player and nothing
// else — the one case `threejs-shadow-systems` allows a single map. Casters
// outside SHADOW_RADIUS_M do not shadow; the cascade/clipmap that would fix
// that is a later slice, and until then this bound is the honest statement
// of what the rig does.
const SHADOW_RADIUS_M = 80;
const SHADOW_MAP_PX = 2048;
const SHADOW_BACK_M = 260; // how far back along the sun ray the light sits
const SHADOW_FAR_M = 520; // ortho depth range; relief is ~90 m (TERRAIN §6)
const SHADOW_NEAR_M = 1;
// Normal bias in texels, not metres: the acne it fixes is a texel-footprint
// artefact, so the metre value has to move with the footprint.
const SHADOW_NORMAL_BIAS_TEXELS = 1.2;
const SHADOW_TEXEL_M = (SHADOW_RADIUS_M * 2) / SHADOW_MAP_PX;
// Z is quantized coarsely on purpose: it moves depth coverage, not the
// projected texel grid (clipmap reference §4).
const SHADOW_Z_QUANTUM_M = SHADOW_RADIUS_M * 0.5;

// Build-grid render dimensions. Cell/level sizes are the sim's grid
// (DECISIONS.md §open, build grid v0). LIFT and WALL_T (and the doorway
// post width below) mirror sim-core collide.rs — collision truth since
// piece collision v0; SLAB and tier colors stay cosmetics.
const CELL = 3;
const LEVEL_H = 3;
const LIFT = 0.3; // collide.rs PIECE_LIFT_M
const SLAB = 0.3; // plane-piece thickness (cosmetic)
const WALL_T = 0.24; // collide.rs WALL_THICKNESS_M
const TIER_COLORS = [0x8a6a45, 0x84837c, 0x5f6a72]; // wood · stone · metal
// …and the response that makes the tier read at a distance, before any of
// them has a texture: wood is matte, stone is matte-but-tighter, metal is
// a conductor with a real specular lobe (materials v0). The reference
// frames' tier read is as much sheen as colour (`bases.webp`).
const TIER_SURFACES = ["wood", "stone", "metal"];
// Deployable stand-ins by archetype code (sim deploy.rs order: bag,
// hearth, box, fire, furnace, workbench, door): [w, h, d, color, surface].
// Cosmetics (DECISIONS.md §open, client cosmetics row).
const DEPLOY_STYLE = [
  [1.2, 0.25, 0.7, 0x7a9c4e, "cloth"], // bag
  [0.9, 0.9, 0.9, 0x8c3b2e, "stone"], // hearth
  [1.0, 0.7, 1.0, 0x7a5c3a, "wood"], // box
  [0.7, 0.4, 0.7, 0xd07030, "stone"], // fire
  [1.1, 1.5, 1.1, 0x4f4a45, "stone"], // furnace
  [1.6, 0.9, 0.9, 0xa1793f, "wood"], // workbench
  [0.12, 2.1, 0.9, 0x6b4a2b, "wood"], // door (thickness, height, width)
];
// A locked door reads as banded iron over the wood — the one bit of door
// state a passer-by can see, and the thing they'd have to break.
const DOOR_LOCKED_COLOR = 0x3c3f44;

/** Mark an object (and a group's children) as both caster and receiver. */
function shadowed(obj) {
  obj.castShadow = true;
  obj.receiveShadow = true;
  for (let i = 0; i < obj.children.length; i++) {
    obj.children[i].castShadow = true;
    obj.children[i].receiveShadow = true;
  }
  return obj;
}

/**
 * The sky dome's geometry: a sphere whose vertex colours ramp from the fog
 * colour at and below the horizon to SKY_ZENITH overhead. Colours are
 * written in the working (linear) space THREE.Color converts the sRGB hex
 * into — the same conversion the fog colour gets — so the horizon ring
 * matches the fog exactly before either is tone-mapped.
 */
function skyDomeGeometry() {
  const geo = new THREE.SphereGeometry(SKY_RADIUS, 24, 16);
  const pos = geo.attributes.position;
  const horizon = new THREE.Color(FOG_COLOR);
  const zenith = new THREE.Color(SKY_ZENITH);
  const colors = new Float32Array(pos.count * 3);
  const c = new THREE.Color();
  for (let i = 0; i < pos.count; i++) {
    const y = pos.getY(i) / SKY_RADIUS;
    const t = y <= 0 ? 0 : Math.pow(y, SKY_CURVE);
    c.copy(horizon).lerp(zenith, t);
    colors[i * 3] = c.r;
    colors[i * 3 + 1] = c.g;
    colors[i * 3 + 2] = c.b;
  }
  geo.setAttribute("color", new THREE.BufferAttribute(colors, 3));
  return geo;
}

export class GameScene {
  constructor(canvas) {
    this.renderer = new THREE.WebGLRenderer({ canvas, antialias: true });
    this.renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
    // Single tone-map ownership: the renderer maps, materials do not, and
    // nothing re-encodes sRGB downstream. The clear colour is the only
    // surface the tone mapper never sees, which is exactly why the sky is
    // geometry below — the clear is a fallback that the dome always covers.
    this.renderer.setClearColor(FOG_COLOR);
    this.renderer.toneMapping = THREE.NeutralToneMapping;
    this.renderer.toneMappingExposure = EXPOSURE;
    this.renderer.shadowMap.enabled = true;
    this.renderer.shadowMap.type = THREE.PCFSoftShadowMap;
    // three resets renderer.info AFTER the shadow pass and before the main
    // one, so the default counters silently exclude every shadow draw — the
    // exact half of the budget this rig just added. Own the reset instead.
    this.renderer.info.autoReset = false;
    this.scene = new THREE.Scene();
    this.scene.fog = new THREE.Fog(FOG_COLOR, FOG_NEAR, FOG_FAR);
    this.camera = new THREE.PerspectiveCamera(
      75,
      window.innerWidth / window.innerHeight,
      0.1,
      1500,
    );

    // The sky is a dome, not a clear colour, for one reason: fog is shaded
    // and therefore tone-mapped, a clear colour is not. Painting the sky as
    // geometry puts both through the same tone map, so the horizon seam is
    // exact instead of tuned. Its horizon ring IS the fog colour.
    this.sky = new THREE.Mesh(
      skyDomeGeometry(),
      new THREE.MeshBasicMaterial({
        vertexColors: true,
        side: THREE.BackSide,
        fog: false,
        depthWrite: false,
        depthTest: false,
      }),
    );
    this.sky.renderOrder = -1;
    this.sky.frustumCulled = false;
    this.scene.add(this.sky);

    const fill = new THREE.HemisphereLight(FILL_SKY, FILL_GROUND, FILL_INTENSITY);
    this.scene.add(fill);
    this.fill = fill;

    // The key. Its shadow box follows the player, snapped to its own texel
    // grid every frame (see updateSun) so the map does not crawl.
    this.sun = new THREE.DirectionalLight(SUN_COLOR, SUN_INTENSITY);
    this.sun.castShadow = true;
    const sh = this.sun.shadow;
    sh.mapSize.set(SHADOW_MAP_PX, SHADOW_MAP_PX);
    sh.camera.left = -SHADOW_RADIUS_M;
    sh.camera.right = SHADOW_RADIUS_M;
    sh.camera.top = SHADOW_RADIUS_M;
    sh.camera.bottom = -SHADOW_RADIUS_M;
    sh.camera.near = SHADOW_NEAR_M;
    sh.camera.far = SHADOW_FAR_M;
    sh.bias = 0;
    sh.normalBias = SHADOW_NORMAL_BIAS_TEXELS * SHADOW_TEXEL_M;
    sh.camera.updateProjectionMatrix();
    this.scene.add(this.sun);
    this.scene.add(this.sun.target); // or its matrixWorld never updates

    // World-space unit vector pointing AT the sun, and the light-space
    // basis derived from it once — the sun does not move in v0, so this is
    // built here and only read in the RAF path.
    const ce = Math.cos(SUN_ELEVATION);
    this._toSun = new THREE.Vector3(
      ce * Math.sin(SUN_AZIMUTH),
      Math.sin(SUN_ELEVATION),
      ce * Math.cos(SUN_AZIMUTH),
    ).normalize();
    // Matrix4.lookAt puts +Z along (eye − target): with the eye at the sun
    // and the target at the origin, that is the shadow camera's own basis.
    this._lightToWorld = new THREE.Matrix4().lookAt(
      this._toSun,
      new THREE.Vector3(0, 0, 0),
      new THREE.Vector3(0, 1, 0),
    );
    this._worldToLight = this._lightToWorld.clone().transpose(); // pure rotation
    this._sunCenter = new THREE.Vector3(); // committed, snapped, world space
    this._sunProbe = new THREE.Vector3();

    // One translucent plane at sea level; nothing simulates (TERRAIN.md §4).
    // It neither casts nor receives: a transparent sheet in the shadow pass
    // buys artefacts, not depth.
    // Smooth, so the low sun leaves a specular track on it — the one thing
    // that separates water from a blue plane before it animates.
    const water = new THREE.Mesh(
      new THREE.PlaneGeometry(6144, 6144),
      surfaceMaterial("water", {
        color: 0x2b5d7d,
        transparent: true,
        opacity: 0.62,
      }),
    );
    water.rotation.x = -Math.PI / 2;
    water.position.set(1024, 0.0, 1024);
    this.scene.add(water);
    this.water = water;

    this.remotes = new Map(); // id -> { group, stamp }
    this._capsuleGeo = new THREE.CapsuleGeometry(0.4, 1.0, 3, 10);
    this._noseGeo = new THREE.ConeGeometry(0.12, 0.34, 8);
    this._noseGeo.rotateX(Math.PI / 2); // apex points +Z (the yaw forward)
    this._remoteMat = surfaceMaterial("cloth", { color: 0xc8a072 });
    this._remoteFrozenMat = surfaceMaterial("cloth", { color: 0x8a8a8a });

    // The weak-spot glint (DESIGN.md §2 "the Rust juice"): one unlit
    // octahedron parked on the marked node's flank; hidden when no mark.
    // Sizes are cosmetics (DECISIONS.md §open, client cosmetics row).
    this.weakMark = new THREE.Mesh(
      new THREE.OctahedronGeometry(0.18),
      new THREE.MeshBasicMaterial({ color: 0xffe066 }),
    );
    this.weakMark.visible = false;
    this.scene.add(this.weakMark);

    // Placed building pieces, keyed by grid address. Shared geometries +
    // one material per tier; meshes are added on placement events (never
    // the RAF path) and swept only by a piece-set reset.
    this.pieces = new Map(); // "cx,cz,level,loc" -> Object3D
    this.deploys = new Map(); // "cx,cz,level,loc" -> Object3D
    this._deployMats = new Map(); // arch -> material (shared per kind)
    this._planeGeo = new THREE.BoxGeometry(CELL - 0.04, SLAB, CELL - 0.04);
    this._wallGeo = new THREE.BoxGeometry(WALL_T, LEVEL_H, CELL - 0.04);
    this._postGeo = new THREE.BoxGeometry(WALL_T, LEVEL_H, 0.9);
    this._lintelGeo = new THREE.BoxGeometry(WALL_T, 0.9, CELL - 0.04 - 1.8);
    this._stairsGeo = new THREE.BoxGeometry(CELL - 0.04, SLAB, 4.15);
    this._tierMats = TIER_COLORS.map((c, i) =>
      surfaceMaterial(TIER_SURFACES[i], { color: c }),
    );
    // The placement ghost: one wireframe box, rescaled to the aimed
    // piece's shape each frame build mode is on.
    this.ghost = new THREE.Mesh(
      new THREE.BoxGeometry(1, 1, 1),
      new THREE.MeshBasicMaterial({ color: 0x9fd08f, wireframe: true }),
    );
    this.ghost.visible = false;
    this.scene.add(this.ghost);

    this._dir = new THREE.Vector3();
    this._target = new THREE.Vector3();
    // Last frame's draw counts (DESIGN §9's budget), refreshed in render().
    this.stats = { calls: 0, triangles: 0 };

    window.addEventListener("resize", () => {
      this.camera.aspect = window.innerWidth / window.innerHeight;
      this.camera.updateProjectionMatrix();
      this.renderer.setSize(window.innerWidth, window.innerHeight);
    });
    this.renderer.setSize(window.innerWidth, window.innerHeight);
  }

  /** Feet position + look angles → camera at the eye. */
  setCamera(x, y, z, yawRad, pitchRad) {
    const c = this.camera;
    c.position.set(x, y + EYE_HEIGHT, z);
    const cp = Math.cos(pitchRad);
    this._dir.set(Math.sin(yawRad) * cp, Math.sin(pitchRad), Math.cos(yawRad) * cp);
    this._target.copy(c.position).add(this._dir);
    c.lookAt(this._target);
    this.sky.position.copy(c.position);
    this.updateSun(x, y, z);
  }

  /**
   * Park the shadow box on the player, snapped to its own texel grid.
   *
   * A directional shadow map that simply tracks the camera crawls: the
   * projected texel grid slides under the geometry and every silhouette
   * edge shimmers. The fix (threejs-shadow-systems) is to quantize the box
   * centre in LIGHT space by the world width of one texel, so the grid is
   * nailed to the world and the box moves in whole-texel steps. Z is
   * quantized far more coarsely — it changes depth coverage, not the
   * projected grid.
   *
   * Scalar math on three preallocated vectors; no allocation, no closure.
   */
  updateSun(x, y, z) {
    const p = this._sunProbe.set(x, y, z).applyMatrix4(this._worldToLight);
    p.x = Math.round(p.x / SHADOW_TEXEL_M) * SHADOW_TEXEL_M;
    p.y = Math.round(p.y / SHADOW_TEXEL_M) * SHADOW_TEXEL_M;
    p.z = Math.round(p.z / SHADOW_Z_QUANTUM_M) * SHADOW_Z_QUANTUM_M;
    p.applyMatrix4(this._lightToWorld);
    if (p.equals(this._sunCenter)) return; // same texel cell: nothing moved
    this._sunCenter.copy(p);
    this.sun.target.position.copy(p);
    this.sun.position.copy(p).addScaledVector(this._toSun, SHADOW_BACK_M);
  }

  /** Park the weak-spot glint at a world position, or hide it. */
  setWeakMark(x, y, z) {
    this.weakMark.position.set(x, y, z);
    this.weakMark.visible = true;
  }

  hideWeakMark() {
    this.weakMark.visible = false;
  }

  /**
   * Upsert one placed piece. `groundY` is the shared-worldgen terrain
   * height at the cell center — both tabs derive the same y, no piece
   * height rides the wire. Shape codes are sim-core build.rs's.
   */
  setPiece(cx, cz, level, loc, shape, material, groundY) {
    const key = `${cx},${cz},${level},${loc}`;
    const old = this.pieces.get(key);
    if (old) this.scene.remove(old);
    const mat = this._tierMats[material] || this._tierMats[0];
    const baseY = groundY + LIFT + level * LEVEL_H;
    const cxm = cx * CELL + CELL / 2;
    const czm = cz * CELL + CELL / 2;
    let obj;
    if (shape === 1 || shape === 2) {
      // Wall / doorway on the west (x = cx·3) or north (z = cz·3) edge;
      // the doorway keeps its opening — the intended breach point reads.
      if (shape === 1) {
        obj = new THREE.Mesh(this._wallGeo, mat);
      } else {
        obj = new THREE.Group();
        const a = new THREE.Mesh(this._postGeo, mat);
        a.position.z = -(CELL - 0.9) / 2 + 0.0;
        const b = new THREE.Mesh(this._postGeo, mat);
        b.position.z = (CELL - 0.9) / 2 - 0.0;
        const l = new THREE.Mesh(this._lintelGeo, mat);
        l.position.y = LEVEL_H / 2 - 0.45;
        obj.add(a, b, l);
      }
      if (loc === 2) {
        obj.position.set(cx * CELL, baseY + LEVEL_H / 2, czm);
      } else {
        obj.rotation.y = Math.PI / 2;
        obj.position.set(cxm, baseY + LEVEL_H / 2, cz * CELL);
      }
    } else if (shape === 4) {
      // Stairs: a ramp through the level. The grid stores no facing, so
      // the ramp always rises toward +Z (cosmetic, v0).
      obj = new THREE.Mesh(this._stairsGeo, mat);
      obj.rotation.x = -Math.PI / 4;
      obj.position.set(cxm, baseY + LEVEL_H / 2, czm);
    } else {
      // Foundation / floor / roof: a slab whose top is the level plane.
      obj = new THREE.Mesh(this._planeGeo, mat);
      obj.position.set(cxm, baseY - SLAB / 2, czm);
    }
    shadowed(obj);
    this.scene.add(obj);
    this.pieces.set(key, obj);
  }

  clearPieces() {
    for (const obj of this.pieces.values()) this.scene.remove(obj);
    this.pieces.clear();
  }

  removePiece(cx, cz, level, loc) {
    const key = `${cx},${cz},${level},${loc}`;
    const obj = this.pieces.get(key);
    if (obj) {
      this.scene.remove(obj);
      this.pieces.delete(key);
    }
  }

  /**
   * Upsert one deployable: a colored box per archetype, standing on the
   * level plane (body deploys) or filling a doorway edge (doors).
   */
  /**
   * Park a deployable at a grid address. `open` and `locked` only mean
   * anything for a door: closed it fills its doorway edge, open it swings
   * a quarter turn onto its hinge — the same read the sim's collision
   * has, so a player never walks through a leaf that still looks shut —
   * and locked it wears the iron.
   */
  setDeploy(cx, cz, level, loc, arch, groundY, open, locked) {
    const key = `${cx},${cz},${level},${loc}`;
    const old = this.deploys.get(key);
    if (old) this.scene.remove(old);
    const [w, h, d, color, surface] = DEPLOY_STYLE[arch] || DEPLOY_STYLE[2];
    // Two materials for the door archetype, one for everything else;
    // both cached, because this runs on every door swing. The locked leaf
    // takes the metal response with the iron colour — the band is what a
    // passer-by sees, the sheen is what tells them it is not wood.
    const ironclad = arch === 6 && locked;
    const matKey = ironclad ? "door-locked" : arch;
    let mat = this._deployMats.get(matKey);
    if (!mat) {
      mat = ironclad
        ? surfaceMaterial("metal", { color: DOOR_LOCKED_COLOR })
        : surfaceMaterial(surface, { color });
      this._deployMats.set(matKey, mat);
    }
    const obj = new THREE.Mesh(new THREE.BoxGeometry(w, h, d), mat);
    const baseY = groundY + LIFT + level * LEVEL_H;
    if (loc === 2 || loc === 3) {
      // A door in a doorway edge, oriented like the wall there. Open, it
      // swings off the hinge end of its leaf and lies across the cell.
      if (loc === 2) {
        if (open) {
          obj.rotation.y = Math.PI / 2;
          obj.position.set(cx * CELL + d / 2, baseY + h / 2, cz * CELL + CELL / 2 - d / 2);
        } else {
          obj.position.set(cx * CELL, baseY + h / 2, cz * CELL + CELL / 2);
        }
      } else if (open) {
        obj.position.set(cx * CELL + CELL / 2 - d / 2, baseY + h / 2, cz * CELL + d / 2);
      } else {
        obj.rotation.y = Math.PI / 2;
        obj.position.set(cx * CELL + CELL / 2, baseY + h / 2, cz * CELL);
      }
    } else {
      obj.position.set(cx * CELL + CELL / 2, baseY + h / 2, cz * CELL + CELL / 2);
    }
    shadowed(obj);
    this.scene.add(obj);
    this.deploys.set(key, obj);
  }

  removeDeploy(cx, cz, level, loc) {
    const key = `${cx},${cz},${level},${loc}`;
    const obj = this.deploys.get(key);
    if (obj) {
      this.scene.remove(obj);
      this.deploys.delete(key);
    }
  }

  clearDeploys() {
    for (const obj of this.deploys.values()) this.scene.remove(obj);
    this.deploys.clear();
  }

  /** Park the placement ghost over the aimed address. */
  setGhost(shape, cx, cz, level, loc, groundY) {
    const g = this.ghost;
    const baseY = groundY + LIFT + level * LEVEL_H;
    const cxm = cx * CELL + CELL / 2;
    const czm = cz * CELL + CELL / 2;
    if (shape === 1 || shape === 2) {
      g.scale.set(WALL_T, LEVEL_H, CELL);
      g.rotation.y = loc === 3 ? Math.PI / 2 : 0;
      if (loc === 2) g.position.set(cx * CELL, baseY + LEVEL_H / 2, czm);
      else g.position.set(cxm, baseY + LEVEL_H / 2, cz * CELL);
    } else if (shape === 4) {
      g.scale.set(CELL, LEVEL_H, CELL);
      g.rotation.y = 0;
      g.position.set(cxm, baseY + LEVEL_H / 2, czm);
    } else {
      g.scale.set(CELL, SLAB, CELL);
      g.rotation.y = 0;
      g.position.set(cxm, baseY - SLAB / 2, czm);
    }
    g.visible = true;
  }

  hideGhost() {
    this.ghost.visible = false;
  }

  /** Upsert one interpolated remote; `stamp` drives mark-and-sweep. */
  upsertRemote(id, x, y, z, yawWire, live, stamp) {
    let r = this.remotes.get(id);
    if (!r) {
      const group = new THREE.Group();
      const body = new THREE.Mesh(this._capsuleGeo, this._remoteMat);
      body.position.y = 0.9;
      const nose = new THREE.Mesh(this._noseGeo, this._remoteMat);
      nose.position.set(0, 1.45, 0.42);
      group.add(body);
      group.add(nose);
      shadowed(group);
      this.scene.add(group);
      r = { group, body, nose, stamp: 0 };
      this.remotes.set(id, r);
    }
    r.group.position.set(x, y, z);
    r.group.rotation.y = yawWire * YAW_TO_RAD;
    const mat = live ? this._remoteMat : this._remoteFrozenMat;
    if (r.body.material !== mat) {
      r.body.material = mat;
      r.nose.material = mat;
    }
    r.stamp = stamp;
  }

  /** Remove remotes not seen this frame (entity left the interest set). */
  sweepRemotes(stamp) {
    for (const [id, r] of this.remotes) {
      if (r.stamp !== stamp) {
        this.scene.remove(r.group);
        this.remotes.delete(id);
      }
    }
  }

  render() {
    // Reset before, not after: with autoReset off these counts then cover
    // BOTH passes — the budget in DESIGN §9 is what the GPU was asked to
    // draw, not what is in view once. Copied into plain numbers so the
    // debug snapshot can be read without holding a live renderer object.
    this.renderer.info.reset();
    this.renderer.render(this.scene, this.camera);
    const r = this.renderer.info.render;
    this.stats.calls = r.calls;
    this.stats.triangles = r.triangles;
  }

  /**
   * The ground's material is built by Terrain (it owns the worker that feeds
   * it); the scene borrows its uniforms so the surface probe has one handle
   * on the whole splat system. Called once at boot.
   */
  attachTerrainMaterial(material) {
    this._terrainMat = material;
    this._terrainUniforms = material.userData.uniforms || null;
  }

  /**
   * Dev-only: does the procedural surface actually reach the frame?
   *
   * Same shape as shadowProbe and for the same reason. Every structural
   * fact about a material can be right — standard material, splat weights
   * on the geometry, four authored identities, a shader that compiled —
   * while the image is a flat wash: a field scaled into a single lattice
   * cell, a break-up amplitude of zero, uniforms never bound, a bump term
   * cancelled by its own footprint fade. So this renders the live scene
   * twice per yaw with `uSurface` at 1 and at 0 and counts the pixels that
   * moved, separately by direction.
   *
   * What the toggle holds fixed is the vertex splat weights, the four
   * authored identities and the causal modifiers (wetness, snow, cliff);
   * what it removes is every contribution of the noise field — the weight
   * break-up, the albedo mottling, the roughness variation and the bump.
   * So the delta is the field, and nothing else.
   *
   * The direction split is the part that is hard to fake: microstructure
   * lightens some pixels and darkens others, and any uniform change (an
   * exposure slip, a global tint) can only move them one way.
   *
   * Allocates and renders 2N frames; never call it from the RAF path.
   */
  surfaceProbe(yaws, pitchRad, minDelta) {
    const u = this._terrainUniforms;
    if (!u) return null;
    const gl = this.renderer.getContext();
    const w = gl.drawingBufferWidth;
    const h = gl.drawingBufferHeight;
    const full = new Uint8Array(w * h * 4);
    const flat = new Uint8Array(w * h * 4);
    const keepQ = this.camera.quaternion.clone();
    const pos = this.camera.position;
    const samples = [];
    let changed = 0;
    for (let i = 0; i < yaws.length; i++) {
      const cp = Math.cos(pitchRad);
      this._dir.set(
        Math.sin(yaws[i]) * cp,
        Math.sin(pitchRad),
        Math.cos(yaws[i]) * cp,
      );
      this._target.copy(pos).add(this._dir);
      this.camera.lookAt(this._target);
      u.uSurface.value = 1;
      this.renderer.render(this.scene, this.camera);
      gl.readPixels(0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, full);
      u.uSurface.value = 0;
      this.renderer.render(this.scene, this.camera);
      gl.readPixels(0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, flat);
      let up = 0;
      let down = 0;
      let sum = 0;
      let max = 0;
      for (let p = 0; p < full.length; p += 4) {
        const a = (full[p] * 2 + full[p + 1] * 5 + full[p + 2]) >> 3;
        const b = (flat[p] * 2 + flat[p + 1] * 5 + flat[p + 2]) >> 3;
        const d = a - b;
        const m = d < 0 ? -d : d;
        if (m > minDelta) {
          if (d > 0) up++;
          else down++;
          sum += m;
          if (m > max) max = m;
        }
      }
      const n = up + down;
      samples.push({
        yaw: yaws[i],
        up,
        down,
        changed: n,
        fraction: n / (w * h),
        upFraction: up / (w * h),
        downFraction: down / (w * h),
        meanDelta: n > 0 ? sum / n : 0,
        maxDelta: max,
      });
      changed += n;
    }
    u.uSurface.value = 1;
    this.camera.quaternion.copy(keepQ);
    this.renderer.render(this.scene, this.camera);
    return { width: w, height: h, pixels: w * h * yaws.length, changed, samples };
  }

  /** The material system's structural facts, for the browser gate. */
  materials() {
    const m = this._terrainMat;
    return {
      ...materialFacts(),
      terrain: {
        type: m ? m.type : null,
        // The splat shader is a patch on a stock standard material; the
        // uniforms it hands back are the proof the patch is installed.
        patched: !!this._terrainUniforms,
        surface: this._terrainUniforms ? this._terrainUniforms.uSurface.value : null,
        roughness: m ? m.roughness : null,
      },
      tiers: this._tierMats.map((t) => [t.type, t.roughness, t.metalness]),
      water: [this.water.material.roughness, this.water.material.metalness],
      remote: [this._remoteMat.roughness, this._remoteMat.metalness],
    };
  }

  /** The structural facts about the rig, for the browser gate to assert. */
  lighting() {
    return {
      shadowMap: this.renderer.shadowMap.enabled,
      shadowType: this.renderer.shadowMap.type,
      sunCasts: this.sun.castShadow,
      mapSize: this.sun.shadow.mapSize.x,
      radiusM: SHADOW_RADIUS_M,
      texelM: SHADOW_TEXEL_M,
      normalBias: this.sun.shadow.normalBias,
      toneMapping: this.renderer.toneMapping,
      exposure: this.renderer.toneMappingExposure,
      fillIntensity: this.fill.intensity,
      sunIntensity: this.sun.intensity,
      calls: this.stats.calls,
      triangles: this.stats.triangles,
    };
  }

  /**
   * Dev-only: does the shadow map actually darken the frame?
   *
   * A flag says the renderer was ASKED for shadows. This measures whether
   * any pixel got one. Per sample yaw it renders the live scene twice —
   * shadow pass on, then off — reads the drawing buffer back both times
   * and counts pixels the shadow pass took down by more than `minDelta`
   * of luma. It restores the camera, the shadow state and the frame
   * before returning, so a probe leaves nothing behind.
   *
   * Allocates, recompiles two programs, and renders 2N frames: never call
   * it from the RAF path. It exists for ci/browser_smoke.mjs.
   */
  shadowProbe(yaws, pitchRad, minDelta) {
    const gl = this.renderer.getContext();
    const w = gl.drawingBufferWidth;
    const h = gl.drawingBufferHeight;
    const lit = new Uint8Array(w * h * 4);
    const shad = new Uint8Array(w * h * 4);
    const keepQ = this.camera.quaternion.clone();
    const pos = this.camera.position;
    const samples = [];
    let darkened = 0;
    for (let i = 0; i < yaws.length; i++) {
      const cp = Math.cos(pitchRad);
      this._dir.set(
        Math.sin(yaws[i]) * cp,
        Math.sin(pitchRad),
        Math.cos(yaws[i]) * cp,
      );
      this._target.copy(pos).add(this._dir);
      this.camera.lookAt(this._target);
      // Toggle the LIGHT, not renderer.shadowMap.enabled: three only
      // recompiles a material when the lights-state version moves, and
      // flipping castShadow changes the shadow count that version is
      // hashed from. Flipping shadowMap.enabled alone leaves every already
      // compiled program shadowing exactly as before — two identical
      // frames and a probe that always reads zero.
      this.sun.castShadow = true;
      this.renderer.info.reset();
      this.renderer.render(this.scene, this.camera);
      const callsShadowed = this.renderer.info.render.calls;
      gl.readPixels(0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, shad);
      this.sun.castShadow = false;
      this.renderer.info.reset();
      this.renderer.render(this.scene, this.camera);
      const callsUnshadowed = this.renderer.info.render.calls;
      gl.readPixels(0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, lit);
      let n = 0;
      let sum = 0;
      let max = 0;
      let litSum = 0;
      let shadSum = 0;
      for (let p = 0; p < lit.length; p += 4) {
        // Rec.601-ish integer luma; the absolute scale does not matter,
        // only the difference between two renders of the same pixel.
        const a = (lit[p] * 2 + lit[p + 1] * 5 + lit[p + 2]) >> 3;
        const b = (shad[p] * 2 + shad[p + 1] * 5 + shad[p + 2]) >> 3;
        litSum += a;
        shadSum += b;
        const d = a - b;
        if (d > minDelta) {
          n++;
          sum += d;
          if (d > max) max = d;
        }
      }
      samples.push({
        yaw: yaws[i],
        darkened: n,
        fraction: n / (w * h),
        meanDelta: n > 0 ? sum / n : 0,
        maxDelta: max,
        // Whole-frame means, so a probe that reads back nothing at all is
        // distinguishable from a rig that casts nothing.
        litMean: litSum / (w * h),
        shadowedMean: shadSum / (w * h),
        // The same frame drawn with and without the shadow pass. The
        // difference IS the shadow pass, which is how the draw budget below
        // is shown to be counting it.
        callsShadowed,
        callsUnshadowed,
      });
      darkened += n;
    }
    this.sun.castShadow = true;
    this.camera.quaternion.copy(keepQ);
    this.renderer.render(this.scene, this.camera);
    return { width: w, height: h, pixels: w * h * yaws.length, darkened, samples };
  }
}
