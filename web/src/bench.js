// Throwaway prop bench — renders the shipped pine on a plain sky so a still
// can be judged against `Rust Images/`. Not part of the client; deleted after.
import * as THREE from "three";
import { pineGeometry, stumpGeometry } from "./props.js";
// NOT surfaceMaterial: its clipmap patch expects N shadow-casting directional
// lights and this bench has one, so the shader will not link. The roughness and
// metalness below are the SURFACES table's own values for each identity, so
// what this shows is the real silhouette, the real baked vertex colours and the
// real normals — everything except the procedural detail field.
const SURF = { foliage: 0.86, wood: 0.95 };

const renderer = new THREE.WebGLRenderer({ antialias: true });
renderer.setSize(1100, 900, false);
renderer.setPixelRatio(1);
renderer.shadowMap.enabled = true;
renderer.shadowMap.type = THREE.PCFSoftShadowMap;
renderer.toneMapping = THREE.ACESFilmicToneMapping;
renderer.toneMappingExposure = 0.8;
document.body.appendChild(renderer.domElement);

const scene = new THREE.Scene();
scene.background = new THREE.Color(0x8fb4d8);

const sun = new THREE.DirectionalLight(0xfff2dd, 2.6);
sun.position.set(-6, 9, 5);
sun.castShadow = true;
sun.shadow.mapSize.set(2048, 2048);
sun.shadow.camera.left = -8;
sun.shadow.camera.right = 8;
sun.shadow.camera.top = 12;
sun.shadow.camera.bottom = -2;
scene.add(sun);
scene.add(new THREE.HemisphereLight(0xa9c4e8, 0x6b6a4a, 1.15));

const ground = new THREE.Mesh(
  new THREE.PlaneGeometry(40, 40),
  new THREE.MeshStandardMaterial({ color: 0x6e7048, roughness: 1 }),
);
ground.rotation.x = -Math.PI / 2;
ground.receiveShadow = true;
scene.add(ground);

// The real pool shape: an InstancedMesh wearing the shipped surface material,
// so the vertex-colour bake, the per-instance colour buffer and the wind patch
// are all on the same path they take in play.
function pool(geo, surface, n) {
  const mat = new THREE.MeshStandardMaterial({ vertexColors: true, roughness: SURF[surface], metalness: 0 });
  const m = new THREE.InstancedMesh(geo, mat, n);
  m.castShadow = true;
  m.receiveShadow = true;
  m.frustumCulled = false;
  for (let i = 0; i < n; i++) m.setColorAt(i, new THREE.Color(1, 1, 1));
  scene.add(m);
  return m;
}

const trees = pool(pineGeometry(), "foliage", 6);
const stumps = pool(stumpGeometry(), "wood", 1);
const m4 = new THREE.Matrix4();
const q = new THREE.Quaternion();
const e = new THREE.Euler();
const v = new THREE.Vector3();
const s = new THREE.Vector3();
const spots = [
  [0, 0, 0, 1.0], [4.4, 0, -3.0, 0.92], [-4.8, 0, -2.2, 1.08],
  [2.2, 0, -8.0, 0.95], [-3.0, 0, -9.5, 1.05], [7.5, 0, -11.0, 1.0],
];
spots.forEach(([x, y, z, k], i) => {
  e.set(0, i * 1.7, 0);
  q.setFromEuler(e);
  v.set(x, y, z);
  s.set(k, k, k);
  trees.setMatrixAt(i, m4.compose(v, q, s));
});
trees.instanceMatrix.needsUpdate = true;
stumps.setMatrixAt(0, m4.compose(v.set(1.9, 0, 1.6), q.setFromEuler(e.set(0, 0.6, 0)), s.set(1, 1, 1)));
stumps.instanceMatrix.needsUpdate = true;

const camera = new THREE.PerspectiveCamera(58, 1100 / 900, 0.1, 200);
const VIEWS = {
  // eye, look-at — the framings that matter: the chopper's view, the
  // silhouette against sky, and the mid-distance stand.
  melee: [[1.6, 1.65, 3.4], [0, 2.6, 0]],
  silhouette: [[7.0, 1.2, 9.0], [0, 4.2, 0]],
  stand: [[11.0, 6.5, 16.0], [0, 3.0, -5.0]],
  canopyUp: [[0.9, 0.35, 1.5], [0, 5.0, 0]],
};
globalThis.__bench = {
  shoot(name) {
    const [eye, at] = VIEWS[name];
    camera.position.set(...eye);
    camera.lookAt(...at);
    renderer.render(scene, camera);
    return true;
  },
  views: Object.keys(VIEWS),
};
globalThis.__bench.shoot("melee");
globalThis.__benchReady = true;
