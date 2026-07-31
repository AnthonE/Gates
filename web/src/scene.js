// The three.js scene shell (DESIGN.md §9): first-person camera at the
// predicted capsule's eye, remote players as capsule+nose groups keyed by
// id, sky/fog/light/water. All per-frame math goes through preallocated
// vectors — no allocations, no closures in the RAF path (L8).

import * as THREE from "three";

const EYE_HEIGHT = 1.6; // cosmetic (DECISIONS.md §open, client cosmetics)
const SKY = 0x8fb4d6;
const YAW_TO_RAD = (Math.PI * 2) / 65536;

// Build-grid render dimensions. Cell/level sizes are the sim's grid
// (DECISIONS.md §open, build grid v0); lift, thicknesses, and tier
// colors are cosmetics (client cosmetics row).
const CELL = 3;
const LEVEL_H = 3;
const LIFT = 0.3; // foundation top sits this far above the terrain sample
const SLAB = 0.3; // plane-piece thickness
const WALL_T = 0.24; // edge-piece thickness
const TIER_COLORS = [0x8a6a45, 0x84837c, 0x5f6a72]; // wood · stone · metal
// Deployable stand-ins by archetype code (sim deploy.rs order: bag,
// hearth, box, fire, furnace, workbench, door): [w, h, d, color].
// Cosmetics (DECISIONS.md §open, client cosmetics row).
const DEPLOY_STYLE = [
  [1.2, 0.25, 0.7, 0x7a9c4e], // bag
  [0.9, 0.9, 0.9, 0x8c3b2e], // hearth
  [1.0, 0.7, 1.0, 0x7a5c3a], // box
  [0.7, 0.4, 0.7, 0xd07030], // fire
  [1.1, 1.5, 1.1, 0x4f4a45], // furnace
  [1.6, 0.9, 0.9, 0xa1793f], // workbench
  [0.12, 2.1, 0.9, 0x6b4a2b], // door (thickness, height, width)
];

export class GameScene {
  constructor(canvas) {
    this.renderer = new THREE.WebGLRenderer({ canvas, antialias: true });
    this.renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
    this.renderer.setClearColor(SKY);
    this.scene = new THREE.Scene();
    this.scene.fog = new THREE.Fog(SKY, 250, 900);
    this.camera = new THREE.PerspectiveCamera(
      75,
      window.innerWidth / window.innerHeight,
      0.1,
      1500,
    );

    const hemi = new THREE.HemisphereLight(0xcfe5ff, 0x4a4436, 0.95);
    this.scene.add(hemi);
    const sun = new THREE.DirectionalLight(0xfff2d8, 1.15);
    sun.position.set(0.6, 1.0, 0.35);
    this.scene.add(sun);

    // One translucent plane at sea level; nothing simulates (TERRAIN.md §4).
    const water = new THREE.Mesh(
      new THREE.PlaneGeometry(6144, 6144),
      new THREE.MeshLambertMaterial({
        color: 0x2b5d7d,
        transparent: true,
        opacity: 0.62,
      }),
    );
    water.rotation.x = -Math.PI / 2;
    water.position.set(1024, 0.0, 1024);
    this.scene.add(water);

    this.remotes = new Map(); // id -> { group, stamp }
    this._capsuleGeo = new THREE.CapsuleGeometry(0.4, 1.0, 3, 10);
    this._noseGeo = new THREE.ConeGeometry(0.12, 0.34, 8);
    this._noseGeo.rotateX(Math.PI / 2); // apex points +Z (the yaw forward)
    this._remoteMat = new THREE.MeshLambertMaterial({ color: 0xc8a072 });
    this._remoteFrozenMat = new THREE.MeshLambertMaterial({
      color: 0x8a8a8a,
    });

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
    this._tierMats = TIER_COLORS.map(
      (c) => new THREE.MeshLambertMaterial({ color: c }),
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
  setDeploy(cx, cz, level, loc, arch, groundY) {
    const key = `${cx},${cz},${level},${loc}`;
    const old = this.deploys.get(key);
    if (old) this.scene.remove(old);
    const [w, h, d, color] = DEPLOY_STYLE[arch] || DEPLOY_STYLE[2];
    let mat = this._deployMats.get(arch);
    if (!mat) {
      mat = new THREE.MeshLambertMaterial({ color });
      this._deployMats.set(arch, mat);
    }
    const obj = new THREE.Mesh(new THREE.BoxGeometry(w, h, d), mat);
    const baseY = groundY + LIFT + level * LEVEL_H;
    if (loc === 2 || loc === 3) {
      // A door in a doorway edge, oriented like the wall there.
      if (loc === 2) {
        obj.position.set(cx * CELL, baseY + h / 2, cz * CELL + CELL / 2);
      } else {
        obj.rotation.y = Math.PI / 2;
        obj.position.set(cx * CELL + CELL / 2, baseY + h / 2, cz * CELL);
      }
    } else {
      obj.position.set(cx * CELL + CELL / 2, baseY + h / 2, cz * CELL + CELL / 2);
    }
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
    this.renderer.render(this.scene, this.camera);
  }
}
