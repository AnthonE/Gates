// Throwaway prop bench — renders prop builders on a plain sky so a still can be
// judged against `Rust Images/`. Not part of the client.
//
// NOT surfaceMaterial: its clipmap patch expects N shadow-casting directional
// lights and this bench has one, so the shader will not link. Judge shape and
// shading here, never colour or exposure.
import * as THREE from "three";
import { ARCHETYPES, ARCH_TREE, stumpGeometry } from "./props.js";

const renderer = new THREE.WebGLRenderer({ antialias: true });
renderer.setSize(1400, 900, false);
renderer.setPixelRatio(1);
renderer.shadowMap.enabled = true;
renderer.shadowMap.type = THREE.PCFSoftShadowMap;
renderer.toneMapping = THREE.ACESFilmicToneMapping;
renderer.toneMappingExposure = 0.8;
document.body.appendChild(renderer.domElement);

const scene = new THREE.Scene();
scene.background = new THREE.Color(0x8fb4d8);
const sun = new THREE.DirectionalLight(0xfff2dd, 2.6);
sun.position.set(-8, 12, 6);
sun.castShadow = true;
sun.shadow.mapSize.set(2048, 2048);
sun.shadow.camera.left = -16; sun.shadow.camera.right = 16;
sun.shadow.camera.top = 16; sun.shadow.camera.bottom = -2;
scene.add(sun);
scene.add(new THREE.HemisphereLight(0xa9c4e8, 0x6b6a4a, 1.15));
const ground = new THREE.Mesh(
  new THREE.PlaneGeometry(120, 120),
  new THREE.MeshStandardMaterial({ color: 0x6e7048, roughness: 1 }),
);
ground.rotation.x = -Math.PI / 2;
ground.receiveShadow = true;
scene.add(ground);

// The SHIPPED archetype rows, built exactly as terrain.js builds them: one
// InstancedMesh per part, one matrix stream, alpha-tested needles casting
// through their own depth material.
const stats = { parts: [] };
// Through the ARCHETYPE, deliberately. Calling pineParts() directly is what
// let a wiring gap survive a commit: the bench rendered a beautiful generated
// tree while the client drew cones, because only the client read this row.
const rows = ARCHETYPES[ARCH_TREE].parts();
const meshes = [];
const SPOTS = [[0,0,0,1.0,0.0],[4.6,0,-4.0,0.92,1.7],[-4.4,0,-3.2,1.08,3.1],[2.0,0,-9.0,0.97,4.6],[-2.6,0,-10.5,1.04,0.9]];
for (const row of rows) {
  const opts = { color: 0xffffff, vertexColors: !row.maps, roughness: row.part === "bark" ? 0.92 : 0.78, metalness: 0 };
  if (row.maps) Object.assign(opts, row.maps);
  if (row.alpha) { opts.side = THREE.DoubleSide; opts.transparent = false; }
  const mat = new THREE.MeshStandardMaterial(opts);
  const m = new THREE.InstancedMesh(row.geo, mat, SPOTS.length);
  if (row.alpha) {
    m.customDepthMaterial = new THREE.MeshDepthMaterial({
      depthPacking: THREE.RGBADepthPacking, side: THREE.DoubleSide,
      map: row.maps.map, alphaMap: row.maps.map, alphaTest: row.maps.alphaTest,
    });
  }
  m.castShadow = true; m.receiveShadow = true; m.frustumCulled = false;
  SPOTS.forEach(([x,y,z,k,yaw], i) => {
    m.setColorAt(i, new THREE.Color(1,1,1));
    m.setMatrixAt(i, new THREE.Matrix4().compose(
      new THREE.Vector3(x,y,z), new THREE.Quaternion().setFromEuler(new THREE.Euler(0,yaw,0)), new THREE.Vector3(k,k,k)));
  });
  scene.add(m); meshes.push(m);
  stats.parts.push({ part: row.part, surface: row.surface,
    tris: (row.geo.index ? row.geo.index.count : row.geo.attributes.position.count)/3,
    textured: !!(row.maps && row.maps.map), alphaTest: (row.maps && row.maps.alphaTest) || 0 });
}
stats.totalTris = stats.parts.reduce((a,p)=>a+p.tris,0);
const stumpM = new THREE.InstancedMesh(stumpGeometry(),
  new THREE.MeshStandardMaterial({ vertexColors: true, roughness: 0.95 }), 1);
stumpM.setColorAt(0, new THREE.Color(1,1,1));
stumpM.castShadow = true; stumpM.receiveShadow = true; stumpM.frustumCulled = false;
stumpM.setMatrixAt(0, new THREE.Matrix4().makeTranslation(1.8, 0.17, 2.2));
scene.add(stumpM);

const camera = new THREE.PerspectiveCamera(52, 1400 / 900, 0.1, 300);
const VIEWS = {
  melee: [[1.5, 1.65, 3.6], [0, 2.8, 0]],
  stand: [[7.0, 5.0, 15.0], [0, 3.0, -4.0]],
  silhouette: [[0, 1.1, 11.0], [0, 4.4, -1.0]],
  canopyUp: [[0.8, 0.4, 1.7], [0, 5.4, 0]],
};
globalThis.__bench = {
  shoot(n) {
    const [eye, at] = VIEWS[n];
    camera.position.set(...eye);
    camera.lookAt(...at);
    renderer.render(scene, camera);
    return true;
  },
  views: Object.keys(VIEWS),
  stats,
};
globalThis.__bench.shoot("melee");
globalThis.__benchReady = true;
