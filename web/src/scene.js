// The three.js scene shell (DESIGN.md §9): first-person camera at the
// predicted capsule's eye, remote players as capsule+nose groups keyed by
// id, sky/fog/light/water. All per-frame math goes through preallocated
// vectors — no allocations, no closures in the RAF path (L8).

import * as THREE from "three";

const EYE_HEIGHT = 1.6; // cosmetic (DECISIONS.md §open, client cosmetics)
const SKY = 0x8fb4d6;
const YAW_TO_RAD = (Math.PI * 2) / 65536;

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
