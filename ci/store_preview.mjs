#!/usr/bin/env node
// Render the asset store's preview pictures, and prove each pack opens in
// three.js. Reads the folders `ci/store_pack.py` wrote, loads every GLB with
// three's own GLTFLoader in headless Chromium (Playwright), poses the rigged
// ones on a clip, and writes assets/store/previews/<id>.jpg (1280×720).
//
// A file that fails to load, or still names a texture three cannot read, fails
// the run: a preview is only drawn from files a builder can open.
//
//   npm i three@0.186.1 playwright   (anywhere; THREE_DIR points at its three)
//   THREE_DIR=…/node_modules/three node ci/store_preview.mjs <packs dir> [pack id …]
//
// What each pack shows is POSES below; a pack with no row shows its models
// standing in a row.
import fs from "node:fs";
import http from "node:http";
import path from "node:path";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";

const ROOT = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const OUT = path.join(ROOT, "assets", "store", "previews");
const [packs, ...want] = process.argv.slice(2);
if (!packs) { console.error("usage: node ci/store_preview.mjs <packs dir> [pack id …]"); process.exit(2); }
const THREE_DIR = process.env.THREE_DIR || path.dirname(createRequire(import.meta.url).resolve("three/package.json"));
const { chromium } = createRequire(path.join(THREE_DIR, "..", "..", "x.js"))("playwright");

// pack id → [file, clip, seconds into it]; files not named are not drawn
const POSES = {
  "gates-fel-wolf": [["fel_wolf_light.glb", "Trot", 0.25], ["fel_wolf_medium.glb", "Howl", 0.9], ["fel_wolf_heavy.glb", "Bite", 0.35]],
  "gates-fel-boar": [["fel_boar_light.glb", "Trot", 0.25], ["fel_boar_medium.glb", "Idle", 0.5], ["fel_boar_heavy.glb", "Bite", 0.35]],
  "gates-fel-stag": [["fel_stag_light.glb", "Trot", 0.25], ["fel_stag_medium.glb", "Howl", 0.9], ["fel_stag_heavy.glb", "Idle", 0.5]],
  "gates-triple-t": [["triple_t.glb", "Idle_Loop", 0.4], ["triple_t.glb", "Jog_Fwd_Loop", 0.2], ["triple_t.glb", "Sword_Attack", 0.45]],
};

const PAGE = `<!doctype html><html><head><style>html,body{margin:0;background:#101312}</style>
<script type="importmap">{"imports":{"three":"/three/build/three.module.js","three/addons/":"/three/examples/jsm/"}}</script>
</head><body><script type="module">
import * as THREE from "three";
import { GLTFLoader } from "three/addons/loaders/GLTFLoader.js";
window.draw = async (files, poses) => {
  const W = 1280, H = 720;
  const r = new THREE.WebGLRenderer({ antialias: true, preserveDrawingBuffer: true });
  r.setSize(W, H); r.setPixelRatio(1);
  r.toneMapping = THREE.ACESFilmicToneMapping; r.toneMappingExposure = 1.15;
  r.shadowMap.enabled = true; r.shadowMap.type = THREE.PCFSoftShadowMap;
  document.body.appendChild(r.domElement);
  const scene = new THREE.Scene();
  scene.background = new THREE.Color(0x14181a);
  scene.add(new THREE.HemisphereLight(0xdfe8ff, 0x2a2620, 1.3));
  const sun = new THREE.DirectionalLight(0xfff2e0, 2.6); sun.position.set(4, 7, 5); sun.castShadow = true;
  sun.shadow.mapSize.set(2048, 2048); scene.add(sun);
  const rim = new THREE.DirectionalLight(0x9fd0ff, 1.4); rim.position.set(-5, 3, -6); scene.add(rim);
  const loader = new GLTFLoader();
  const report = [], items = [];
  for (const [file, clip, t] of poses) {
    let g;
    try { g = await loader.loadAsync(file); }
    catch (e) { report.push({ file, ok: false, error: String(e && e.message || e) }); continue; }
    let maps = 0, missing = 0;
    g.scene.traverse((o) => {
      if (!o.isMesh) return;
      o.castShadow = true; o.receiveShadow = true;
      for (const k of ["map", "normalMap", "roughnessMap", "metalnessMap", "emissiveMap"]) {
        const tex = o.material[k];
        if (tex) { maps++; if (!tex.image || !tex.image.width) missing++; }
      }
    });
    const names = g.animations.map((a) => a.name);
    if (clip) {
      const a = g.animations.find((x) => x.name === clip);
      if (!a) { report.push({ file, ok: false, error: "no clip " + clip, clips: names }); continue; }
      const mixer = new THREE.AnimationMixer(g.scene);
      mixer.clipAction(a).play(); mixer.update(t);
    }
    g.scene.updateMatrixWorld(true);
    report.push({ file, ok: missing === 0, maps, missing, clips: names.length });
    items.push(g.scene);
  }
  if (!items.length) return { report };
  const box = (o) => { o.updateMatrixWorld(true); return new THREE.Box3().setFromObject(o, true); };
  const size = (o) => box(o).getSize(new THREE.Vector3());
  const cam = new THREE.PerspectiveCamera(30, W / H, 0.01, 1000);
  const even = !poses[0][1];
  const placed = [];
  if (even) {
    // held things on a rack: each laid level (its long side across), turned
    // to show its broad face, evened to one length, stacked top to bottom
    const pivots = items.map((o) => {
      const p = new THREE.Group(); p.add(o);
      const s0 = size(p);
      if (s0.y >= s0.x && s0.y >= s0.z) p.rotateOnWorldAxis(new THREE.Vector3(0, 0, 1), -Math.PI / 2);
      else if (s0.z > s0.x) p.rotateOnWorldAxis(new THREE.Vector3(0, 1, 0), Math.PI / 2);
      const s1 = size(p);
      if (s1.z > s1.y) p.rotateOnWorldAxis(new THREE.Vector3(1, 0, 0), Math.PI / 2);
      p.rotateOnWorldAxis(new THREE.Vector3(0, 1, 0), -0.18);
      p.scale.multiplyScalar(1 / Math.max(size(p).x, 1e-3));
      return p;
    });
    const cols = pivots.length > 3 ? 2 : 1;
    let y = 0;
    for (let r0 = 0; r0 < pivots.length; r0 += cols) {
      const row = pivots.slice(r0, r0 + cols);
      const h = Math.max(...row.map((p) => size(p).y));
      row.forEach((p, k) => {
        const b = box(p), c0 = b.getCenter(new THREE.Vector3());
        p.position.x += k * 1.18 - c0.x; p.position.z -= c0.z; p.position.y += y - h / 2 - c0.y;
        scene.add(p); placed.push(p);
      });
      y -= h + 0.12;
    }
  } else {
    // creatures and characters in a row on the ground, a quarter turned
    let x = 0;
    items.forEach((o) => {
      o.rotation.y = -0.5;
      const b = box(o), sz = b.getSize(new THREE.Vector3()), c0 = b.getCenter(new THREE.Vector3());
      o.position.x += x - b.min.x; o.position.y -= b.min.y; o.position.z -= c0.z;
      x += sz.x + 0.3 * Math.max(sz.y, 0.3);
      scene.add(o); placed.push(o);
    });
  }
  const all = new THREE.Box3(); placed.forEach((o) => { o.updateMatrixWorld(true); all.expandByObject(o, true); });
  const c = all.getCenter(new THREE.Vector3()), s = all.getSize(new THREE.Vector3());
  if (!even) {
    const ground = new THREE.Mesh(new THREE.CircleGeometry(Math.max(s.x, s.z) * 1.4, 64),
      new THREE.MeshStandardMaterial({ color: 0x23282a, roughness: 0.95 }));
    ground.rotation.x = -Math.PI / 2; ground.position.set(c.x, all.min.y - 0.001, c.z); ground.receiveShadow = true;
    scene.add(ground);
  }
  const sh = Math.max(s.x, s.y);
  sun.shadow.camera.left = -sh; sun.shadow.camera.right = sh; sun.shadow.camera.top = sh; sun.shadow.camera.bottom = -sh;
  sun.target.position.copy(c); scene.add(sun.target);
  const fit = Math.max(s.x / (W / H), s.y) * (even ? 1.12 : 1.4);
  const d = fit / (2 * Math.tan((cam.fov * Math.PI) / 360));
  if (even) {
    cam.position.set(c.x, c.y + s.y * 0.08, c.z + d + s.z); cam.lookAt(c.x, c.y, c.z);
    const key = new THREE.DirectionalLight(0xffffff, 2.2); key.position.set(c.x - 1, c.y + 1.5, c.z + 4);
    key.target.position.copy(c); scene.add(key, key.target);
  }
  else { cam.position.set(c.x + d * 0.18, c.y + s.y * 0.35, c.z + d); cam.lookAt(c.x, c.y - s.y * 0.05, c.z); }
  r.render(scene, cam);
  return { report };
};
window.ready = true;
</script></body></html>`;

const types = { ".js": "text/javascript", ".glb": "model/gltf-binary", ".wasm": "application/wasm" };
const server = http.createServer((req, res) => {
  const u = decodeURIComponent(new URL(req.url, "http://x").pathname);
  if (u === "/") { res.writeHead(200, { "content-type": "text/html" }); return res.end(PAGE); }
  const [base, rest] = u.startsWith("/three/") ? [THREE_DIR, u.slice(7)] : u.startsWith("/packs/") ? [path.resolve(packs), u.slice(7)] : [null, null];
  const p = base && path.resolve(base, rest);
  if (!p || !p.startsWith(base) || !fs.existsSync(p)) { res.writeHead(404); return res.end(); }
  res.writeHead(200, { "content-type": types[path.extname(p)] || "application/octet-stream" });
  fs.createReadStream(p).pipe(res);
});
await new Promise((ok) => server.listen(0, "127.0.0.1", ok));
const port = server.address().port;
const browser = await chromium.launch({ args: ["--use-gl=angle", "--use-angle=swiftshader", "--enable-unsafe-swiftshader"] });
let bad = 0;
fs.mkdirSync(OUT, { recursive: true });
for (const id of fs.readdirSync(packs).sort()) {
  if (want.length && !want.includes(id)) continue;
  const mf = path.join(packs, id, "elo-asset.json");
  if (!fs.existsSync(mf)) continue;
  const m = JSON.parse(fs.readFileSync(mf, "utf8"));
  const page = await browser.newPage({ viewport: { width: 1280, height: 720 } });
  page.on("pageerror", (e) => console.error(`  ${id}: page error ${e.message}`));
  await page.goto(`http://127.0.0.1:${port}/`);
  await page.waitForFunction(() => window.ready === true);
  // every file loads, drawn or not
  const every = m.files.map((f) => [`/packs/${id}/${f.name}`, null, 0]);
  const check = await page.evaluate(async (p) => (await window.draw(p.map((x) => x[0]), p)).report, every);
  await page.close();
  const page2 = await browser.newPage({ viewport: { width: 1280, height: 720 } });
  await page2.goto(`http://127.0.0.1:${port}/`);
  await page2.waitForFunction(() => window.ready === true);
  const poses = (POSES[id] || m.files.filter((f) => !/carcass/.test(f.name)).map((f) => [f.name, null, 0]))
    .map(([f, c, t]) => [`/packs/${id}/${f}`, c, t]);
  const drawn = await page2.evaluate(async (p) => (await window.draw(null, p)).report, poses);
  for (const r of [...check, ...drawn]) {
    if (!r.ok) { bad++; console.error(`  FAIL ${id} ${r.file}: ${r.error || r.missing + " textures did not load"}`); }
  }
  await page2.locator("canvas").screenshot({ path: path.join(OUT, `${id}.jpg`), type: "jpeg", quality: 86 });
  await page2.close();
  console.log(`${id}: ${check.filter((r) => r.ok).length}/${check.length} files open in three.js, preview written`);
}
await browser.close();
server.close();
process.exit(bad ? 1 : 0);
