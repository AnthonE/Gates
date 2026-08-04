// Throwaway prop bench — renders prop builders on a plain sky so a still can be
// judged against `Rust Images/`. Not part of the client.
//
// NOT surfaceMaterial: its clipmap patch expects N shadow-casting directional
// lights and this bench has one, so the shader will not link. Judge shape and
// shading here, never colour or exposure.
import * as THREE from "three";
import { Tree } from "@dgreenheck/ez-tree";
import { pineGeometry } from "./props.js";

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

// Target height for every variant, so the only thing that varies is detail.
const TARGET_H = 6.6;

// Dial-downs of "Pine Medium". The preset is 59,616 tris and 55 m tall; the
// cost lives in children[0] (82 first-level branches), leaves.count (30 cards
// each) and the double billboard (every card drawn twice, crossed).
const VARIANTS = [
  { name: "hero", children: 46, leafCount: 18, billboard: "double", sec1: 8, seg1: 5, seg0: 7 },
  { name: "near", children: 30, leafCount: 12, billboard: "double", sec1: 6, seg1: 4, seg0: 6 },
  { name: "mid", children: 22, leafCount: 9, billboard: "single", sec1: 5, seg1: 3, seg0: 5 },
  { name: "far", children: 14, leafCount: 6, billboard: "single", sec1: 4, seg1: 3, seg0: 4 },
];

const stats = { ours: pineGeometry().attributes.position.count / 3, variants: [] };

function build(cfg, seed, x, z) {
  const t = new Tree();
  t.loadPreset("Pine Medium");
  const o = t.options;
  o.seed = seed;
  o.branch.children[0] = cfg.children;
  o.branch.sections[1] = cfg.sec1;
  o.branch.segments[1] = cfg.seg1;
  o.branch.segments[0] = cfg.seg0;
  o.branch.sections[0] = 8;
  o.leaves.count = cfg.leafCount;
  o.leaves.billboard = cfg.billboard;
  t.generate();
  // Preset trunk length 50 gives a 55 m tree; normalize every variant to the
  // same height so tri counts are comparable and the scale question is closed.
  const box = new THREE.Box3().setFromObject(t);
  const k = TARGET_H / (box.max.y - box.min.y);
  t.scale.setScalar(k);
  t.position.set(x, 0, z);
  let tris = 0;
  let leafTris = 0;
  t.traverse((m) => {
    if (!m.isMesh) return;
    m.castShadow = true;
    m.receiveShadow = true;
    const n = m.geometry.index ? m.geometry.index.count / 3 : m.geometry.attributes.position.count / 3;
    tris += n;
    if (m.material && m.material.alphaTest > 0) leafTris += n;
  });
  scene.add(t);
  return { tris, leafTris, barkTris: tris - leafTris, heightM: +(TARGET_H).toFixed(2) };
}

VARIANTS.forEach((cfg, i) => {
  const r = build(cfg, 1000 + i * 137, -9 + i * 6, 0);
  stats.variants.push({ ...cfg, ...r });
});
// …and ours on the far right, same ground, same sun.
const oursMesh = new THREE.InstancedMesh(
  pineGeometry(),
  new THREE.MeshStandardMaterial({ vertexColors: true, roughness: 0.86, metalness: 0 }),
  1,
);
oursMesh.setColorAt(0, new THREE.Color(1, 1, 1));
oursMesh.castShadow = true;
oursMesh.receiveShadow = true;
oursMesh.frustumCulled = false;
oursMesh.setMatrixAt(0, new THREE.Matrix4().makeTranslation(15, 0, 0));
scene.add(oursMesh);

const camera = new THREE.PerspectiveCamera(52, 1400 / 900, 0.1, 300);
const VIEWS = {
  lineup: [[2.0, 5.2, 22.0], [2.0, 3.4, 0]],
  nearPair: [[-2.0, 1.7, 7.0], [-3.0, 3.2, 0]],
  silhouette: [[2.0, 1.0, 16.0], [2.0, 4.6, 0]],
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
globalThis.__bench.shoot("lineup");
globalThis.__benchReady = true;
