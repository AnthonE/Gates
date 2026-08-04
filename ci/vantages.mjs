#!/usr/bin/env node
// Gate: the ground's material holds up somewhere other than the beach.
//
// Why this file exists, stated as the failure it is for. Every material
// assertion in `browser_smoke.mjs` — 15b, 15c, 15d, 15e, 15h, 15i — fires from
// ONE spawn (`dev_spawn = 1024,1024`) at ONE bearing (every view in that file
// is `yaw: 0`, only the pitch varies), on ONE seed. Measured over that spawn's
// own 40 m ring, **100.0% of it is under 10 degrees of tilt and the steepest
// thing in it is 6.2 degrees**, while the island as a whole runs 31.6% over
// 10 degrees and its steepest faces reach 69. Three separate defects lived in
// that blind spot for as many passes and shipped green:
//
//   · the base maps were projected on world XZ and smeared 1/u along every
//     fall line — invisible at 6.2 degrees, x2.8 at 69;
//   · every octave retired on the horizontal footprint instead of the world
//     one, so on a steep face they ran three times past their own Nyquist;
//   · snow and wetness REPLACED albedo and roughness with constants, and no
//     probe in the tree ever stood above 52 m or below the wade line.
//
// None of that is a subtle gate. It is the camera. `threejs-visual-validation`
// (the checked-in skill pack) lists the required capture set as design view,
// near/detail, far/silhouette, no-post baseline, one controlling diagnostic,
// one failure-sensitive diagnostic, and **one stress condition** — and names
// "approval relies on a single frame" as a rejection criterion. This file is
// the two of those the tree did not have: the camera envelope, and the seed.
//
// It is deliberately NOT more assertions bolted onto `browser_smoke.mjs`.
// That file's two-tab join is a 60 s clock and the trap list records what that
// costs on a loaded box; this one runs **one tab at a time**, one shard per
// vantage, sequentially, and settles on observable state only. It reuses the
// probes that already exist rather than adding shader instrumentation: the
// question "does the ground carry the photograph's detail" is `baseProbe`'s,
// and asking it somewhere other than the beach is the whole change.
import { spawn } from "node:child_process";
import fs from "node:fs";
import http from "node:http";
import os from "node:os";
import path from "node:path";
import { createRequire } from "node:module";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = path.join(path.dirname(fileURLToPath(import.meta.url)), "..");
const DIST = path.join(root, "web/dist");
const SHARD = path.join(root, "target/release/shard");
const PORT = Number(process.env.VANTAGE_PORT || 8971);
const WIRE0 = Number(process.env.VANTAGE_WIRE_PORT || 4610);
const OUT = process.env.VANTAGE_OUT || "";

// browser_smoke's own constants, so the two files' numbers are comparable.
const MIN_DELTA = 1;
// What is asserted.
//
// MIN_CONTRAST: 15h's own floor, in 15h's units, asked somewhere 15h cannot
// stand. The beach vantage here reads the same 9.22 luma/px that
// `browser_smoke`'s own "down" view reports, which is the check that these
// numbers are the same numbers — an earlier cut of this file read 3.10 there
// and the difference was an unbuilt near ring, not a different statistic. The
// settle below is what fixed it, and the agreement is worth re-reading if it
// ever drifts.
const MIN_CONTRAST = Number(process.env.VANTAGE_MIN_CONTRAST || 4.0);
// MIN_RATIO: the photograph over the procedural ground, at the same framing.
// This is the assertion that catches a REPLACING modifier, and it is the one
// that would have caught the snow: a constant albedo scores ~1.0 here while
// 15h stays green on a beach that has no snow in it.
const MIN_RATIO = Number(process.env.VANTAGE_MIN_RATIO || 3.0);
// MAX_CHROMA: 15i's own ceiling, in 15i's units, unchanged.
const MAX_CHROMA = Number(process.env.VANTAGE_MAX_CHROMA || 0.35);
// The reference target both are aimed at, printed on every run beside what the
// tree actually reads, so the gap cannot quietly stop closing (15h's own
// discipline, applied to a gate that has more than one vantage to close it at).
const REF_CHROMA_TARGET = 0.193;

// The vantages. Each is a spawn the worldgen actually produces, found by
// scanning the seed's own height field rather than chosen by eye — the tilt
// and height below are asserted at run time, so a worldgen change that flattens
// one of these goes red saying so instead of quietly measuring a meadow.
const VANTAGES = [
  {
    label: "beach",
    seed: 20260731,
    spawn: "1024,1024",
    yaw: 0,
    pitch: -0.8,
    // The control, and it is `browser_smoke`'s own ground: if this one moves,
    // the change is not confined to slopes and every number in that file is in
    // play. Nothing here may be steep.
    maxTiltDeg: 12,
  },
  {
    label: "slope",
    seed: 20260731,
    spawn: "954,1242",
    yaw: -0.322,
    pitch: 0.35,
    // The steepest face this seed builds, looked straight up. This is the
    // frame all three defects were visible in and none of them was measured.
    minTiltDeg: 45,
    // This vantage carried a two-sided regression wall for exactly one pass
    // (chroma 0.75 against the real 0.35, ratio x1.5 against x3.0), recording
    // two defects that were live when it was written. Both were arithmetic in
    // the biplanar projection — a footprint differentiated after a rotating
    // frame instead of before it, and a blend with no sharpening exponent —
    // and fixing them took this vantage to chroma 0.127 and ratio x3.5. The
    // wall's own staleness check is what said so, on the run after: "the
    // defect the wall was recording is fixed, delete the override". Deleted.
  },
  {
    label: "alpine",
    seed: 20260731,
    spawn: "924,1368",
    yaw: 1.2,
    pitch: -0.25,
    // Above the snow line (SNOW_RANGE tops out at 80 m), which no probe in
    // this repo had ever stood above. The causal modifiers had zero coverage.
    minHeightM: 80,
  },
  {
    label: "stress-seed",
    seed: 1,
    spawn: "1024,1024",
    yaw: 0,
    pitch: -0.4,
    // The protocol's stress condition. Same spawn coordinates as the beach
    // control, a different world: seed 1 puts a 60.9 degree face inside the
    // ring that seed 20260731 fills with 6.2 degrees of meadow. One seed is
    // not a sweep, and the cheapest possible sweep is two.
    minTiltDeg: 30,
  },
];

let checks = 0;
const fail = (msg) => {
  console.error(`GATE FAIL: ${msg}`);
  process.exit(1);
};
const check = (cond, msg) => {
  checks++;
  if (!cond) fail(msg);
};

// --- dependencies, each a loud failure (never a silent skip) ----------------
const require = createRequire(path.join(root, "web/package.json"));
let chromium;
try {
  const mod = await import(pathToFileURL(require.resolve("playwright")).href);
  chromium = mod.chromium ?? mod.default?.chromium;
  if (!chromium) throw new Error("playwright exported no chromium");
} catch (e) {
  fail(`playwright not installed (web devDependency): ${e.message}`);
}
if (!fs.existsSync(SHARD)) fail(`shard binary missing at ${SHARD}`);
if (!fs.existsSync(path.join(DIST, "index.html"))) fail(`web bundle missing at ${DIST}`);
const WASM = path.join(root, "target/wasm32-unknown-unknown/release/client_wasm.wasm");
if (!fs.existsSync(WASM)) fail(`client wasm missing at ${WASM} — the tilt check reads worldgen from it`);

// --- the terrain, read from the sim's own worldgen --------------------------
// The vantage claims ("this one is steep", "this one is above the snow line")
// are checked against `terrain_height_at` rather than trusted, because a
// coordinate that silently stops being a cliff is exactly how this gate would
// rot into measuring another meadow.
const { instance: wasm } = await WebAssembly.instantiate(fs.readFileSync(WASM), {});
const heightAt = (seed, x, z) => wasm.exports.terrain_height_at(BigInt(seed), x, z);
const tiltDeg = (seed, x, z, d = 1) => {
  const gx = (heightAt(seed, x + d, z) - heightAt(seed, x - d, z)) / (2 * d);
  const gz = (heightAt(seed, x, z + d) - heightAt(seed, x, z - d)) / (2 * d);
  return (Math.atan(Math.hypot(gx, gz)) * 180) / Math.PI;
};
// The steepest face inside the near ring the vantage's own camera can see.
const ringMaxTilt = (seed, cx, cz, r = 40) => {
  let m = 0;
  for (let dx = -r; dx <= r; dx += 2) {
    for (let dz = -r; dz <= r; dz += 2) {
      if (dx * dx + dz * dz > r * r) continue;
      m = Math.max(m, tiltDeg(seed, cx + dx, cz + dz));
    }
  }
  return m;
};

let server, browser, tmpDir;
const shards = [];
const cleanup = () => {
  try { browser && browser.close(); } catch {}
  try { server && server.close(); } catch {}
  for (const s of shards) { try { s.kill("SIGKILL"); } catch {} }
  try { tmpDir && fs.rmSync(tmpDir, { recursive: true, force: true }); } catch {}
};
process.on("exit", cleanup);
tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "gates-vantage-"));

const MIME = { ".html": "text/html", ".js": "text/javascript", ".wasm": "application/wasm", ".css": "text/css" };
server = http.createServer((req, res) => {
  const rel = decodeURIComponent(req.url.split("?")[0]);
  // The bundle ships no favicon and the browser asks for one anyway. Answered
  // with an explicit 204 rather than filtered out of the console-error check
  // downstream: a page error is a gate failure here, and the way to keep that
  // strict is to stop producing the error, not to learn to ignore one.
  if (rel === "/favicon.ico") return res.writeHead(204).end();
  const file = path.join(DIST, rel === "/" ? "index.html" : rel);
  if (!file.startsWith(DIST) || !fs.existsSync(file) || fs.statSync(file).isDirectory()) {
    return res.writeHead(404).end("not found");
  }
  res.writeHead(200, {
    "content-type": MIME[path.extname(file)] || "application/octet-stream",
    "cross-origin-opener-policy": "same-origin",
    "cross-origin-embedder-policy": "require-corp",
  });
  fs.createReadStream(file).pipe(res);
});
await new Promise((r) => server.listen(PORT, "127.0.0.1", r));

// The reference box has no GPU and neither does CI: ANGLE over SwiftShader,
// same flags `browser_smoke` launches with.
try {
  browser = await chromium.launch({
    headless: true,
    ...(process.env.VANTAGE_CHROME ? { executablePath: process.env.VANTAGE_CHROME } : {}),
    args: [
      "--enable-unsafe-swiftshader", "--use-gl=angle", "--use-angle=swiftshader", "--ignore-gpu-blocklist",
      "--disable-renderer-backgrounding", "--disable-backgrounding-occluded-windows",
      "--disable-background-timer-throttling",
    ],
  });
} catch (e) {
  fail(`chromium failed to launch: ${e.message}\n  install it: npx playwright install chromium`);
}

if (OUT) fs.mkdirSync(OUT, { recursive: true });
const rows = [];

for (let vi = 0; vi < VANTAGES.length; vi++) {
  const v = VANTAGES[vi];
  const [sx, sz] = v.spawn.split(",").map(Number);

  // --- the vantage is what it says it is, before anything renders ----------
  const ringTilt = ringMaxTilt(v.seed, sx, sz);
  const groundY = heightAt(v.seed, sx, sz);
  if (v.minTiltDeg !== undefined) {
    check(
      ringTilt >= v.minTiltDeg,
      `${v.label}: the steepest face within 40 m of (${v.spawn}) on seed ${v.seed} is ` +
        `${ringTilt.toFixed(1)} deg, under the ${v.minTiltDeg} this vantage exists to look at. ` +
        `A vantage that stopped being steep is a probe quietly re-pointed at a meadow, which ` +
        `is the exact failure this file was written for.`,
    );
  }
  if (v.maxTiltDeg !== undefined) {
    check(
      ringTilt <= v.maxTiltDeg,
      `${v.label}: this is the FLAT control and its ring now reaches ${ringTilt.toFixed(1)} deg ` +
        `(ceiling ${v.maxTiltDeg}) — it can no longer separate "changed on slopes" from ` +
        `"changed everywhere", which is the only thing it is for`,
    );
  }
  if (v.minHeightM !== undefined) {
    check(
      groundY >= v.minHeightM,
      `${v.label}: (${v.spawn}) on seed ${v.seed} stands at ${groundY.toFixed(1)} m, below the ` +
        `${v.minHeightM} m this vantage exists to be above — the snow modifier is not in frame`,
    );
  }

  // --- one shard, one tab, torn down before the next -----------------------
  const port = WIRE0 + vi;
  const cfg = path.join(tmpDir, `${v.label}.toml`);
  fs.writeFileSync(cfg, `bind = "127.0.0.1:${port}"\nseed = ${v.seed}\ndev_spawn = "${v.spawn}"\n`);
  const log = [];
  const proc = spawn(SHARD, [cfg], { cwd: root, env: { ...process.env, RUST_LOG: "warn" } });
  shards.push(proc);
  let cert;
  try {
    cert = await new Promise((resolve, reject) => {
      const t = setTimeout(() => reject(new Error(`${v.label}: shard printed no cert hash in 30 s`)), 30000);
      const feed = (d) => {
        for (const line of String(d).split("\n")) {
          if (!line) continue;
          log.push(line);
          const m = line.match(/dev cert sha256\s+([0-9a-fA-F:]+)/);
          if (m) { clearTimeout(t); resolve(m[1]); }
        }
      };
      proc.stdout.on("data", feed);
      proc.stderr.on("data", feed);
      proc.on("exit", (c) => reject(new Error(`${v.label}: shard exited (${c}): ${log.join(" | ")}`)));
    });
  } catch (e) {
    fail(e.message);
  }

  // Fresh context per vantage: a page that has already rendered another world
  // carries its programs and its ring, and the capture discipline says a fresh
  // page per shot.
  const context = await browser.newContext({ viewport: { width: 1280, height: 720 } });
  const page = await context.newPage();
  const errors = [];
  page.on("pageerror", (e) => errors.push(`pageerror: ${e.message}`));
  page.on("console", (m) => { if (m.type() === "error") errors.push(`console.error: ${m.text()}`); });
  await page.goto(`http://127.0.0.1:${PORT}/`, { waitUntil: "load" });
  await page.fill("#url", `https://127.0.0.1:${port}`);
  await page.fill("#cert", cert);
  await page.click("#connect");

  // Observable state, never elapsed ms (CLAUDE.md). The window is generous
  // because this box may be loaded; what it waits FOR is a fact, not a clock.
  //
  // Two conditions, and the second is the one this gate had to learn. Reaching
  // the world is not the same as having a ring to photograph: the chunk worker
  // amortizes one build per frame, and on a software rasterizer the near ring
  // can still be filling in long after `inWorld` goes true. Measured here, the
  // beach control read 3.10 luma/px against its own 9.22 on a settled frame —
  // the same code, the same camera, an unbuilt ring. So the settle waits for
  // the PROGRAM COUNT to stop moving (nothing left to compile is the closest
  // observable proxy for nothing left to build) as well as for snapshots, and
  // it is a state condition rather than a sleep, per CLAUDE.md.
  try {
    await page.waitForFunction(
      () => {
        const d = globalThis.__gatesDebug;
        if (!d || d.inWorld !== true) return false;
        const g = (globalThis.__vantageSettle ||= { last: -1, same: 0 });
        if (d.programs === g.last) g.same++;
        else { g.last = d.programs; g.same = 0; }
        return g.same > 8 && d.snapshots > 120;
      },
      null, { timeout: 300000, polling: 250 },
    );
  } catch {
    const state = await page.evaluate(() => ({
      hasDebug: typeof globalThis.__gatesDebug,
      inWorld: globalThis.__gatesDebug?.inWorld,
      snapshots: globalThis.__gatesDebug?.snapshots,
      starterr: document.getElementById("starterr")?.textContent ?? "",
    }));
    fail(`${v.label}: never reached the world — ${JSON.stringify(state)} · ${errors.join(" | ")}`);
  }

  const dev = await page.evaluate(() => typeof globalThis.__gatesDebug.baseProbe);
  check(dev === "function", `${v.label}: __gatesDebug.baseProbe is ${dev} on a dev shard`);

  // The probes aim themselves, and they aim FROM the live camera's position —
  // so nothing here touches the camera before they run. That is not a
  // convenience, it is what makes these numbers comparable to 15h's: this
  // client's camera rig moves the eye with the pitch, so calling `setView`
  // first and then letting the probe aim to the same bearing measures from a
  // different point entirely. Measured: the beach control reads 9.22 luma/px
  // probed from the natural eye and 3.10 probed after a `setView(0, -0.8)`,
  // on identical code. A gate that aims first is measuring a different frame
  // from the one it is quoting a floor against.
  const view = [{ label: v.label, yaw: v.yaw, pitch: v.pitch }];
  const base = await page.evaluate(([w, d]) => globalThis.__gatesDebug.baseProbe(w, d), [view, MIN_DELTA]);
  check(base && base.samples && base.samples.length === 1, `${v.label}: baseProbe returned nothing`);
  const chroma = await page.evaluate(([w, d]) => globalThis.__gatesDebug.chromaProbe(w, d), [view, MIN_DELTA]);
  check(chroma && chroma.samples && chroma.samples.length === 1, `${v.label}: chromaProbe returned nothing`);

  const s = base.samples[0];
  const c = chroma.samples[0];
  const at = await page.evaluate(() => globalThis.__gatesDebug.snapshots);
  const ratio = s.contrastShip / Math.max(s.contrastFlat, 1e-6);
  rows.push({ label: v.label, tilt: ringTilt, y: groundY, ship: s.contrastShip, flat: s.contrastFlat, ratio, chroma: c.ratio, at });

  rows[rows.length - 1].noise = s.noise;
  // A human-readable frame of the vantage, for the findings directory. Taken
  // last and aimed, deliberately: it is an artifact, not evidence, and every
  // probe above it has already restored the uniforms it borrowed.
  if (OUT) {
    await page.evaluate(([y, pi]) => globalThis.__gatesDebug.setView(y, pi), [v.yaw, v.pitch]);
    await page.waitForFunction(
      () => {
        const d = globalThis.__gatesDebug;
        if (!d) return false;
        const g = (globalThis.__vantageShot ||= { last: -1, same: 0 });
        if (d.snapshots === g.last) g.same++;
        else { g.last = d.snapshots; g.same = 0; }
        return d.snapshots > 0 && g.same >= 0;
      },
      null, { timeout: 60000, polling: 250 },
    );
    await page.screenshot({ path: path.join(OUT, `${v.label}.png`) });
  }
  check(errors.length === 0, `${v.label}: page errors — ${errors.join(" | ")}`);

  await context.close();
  proc.kill("SIGTERM");
}

console.log("  vantages (15h's statistic and 15i's, asked off the beach):");
for (const r of rows) {
  console.log(
    `    ${r.label.padEnd(12)} ring ${r.tilt.toFixed(1).padStart(4)} deg · y ${r.y.toFixed(1).padStart(6)} m · ` +
      `detail ${r.ship.toFixed(2).padStart(5)} luma/px (flat ${r.flat.toFixed(2)}, x${r.ratio.toFixed(1)}) · ` +
      `chroma ${r.chroma.toFixed(3)} · at snapshot ${r.at}`,
  );
}

// Asserted only after every vantage has reported. A gate that exits on the
// first red prints one number and hides the three that would have explained it.
const byLabel = Object.fromEntries(VANTAGES.map((v) => [v.label, v]));
for (const r of rows) {
  const v = byLabel[r.label];
  const minRatio = v.minRatio ?? MIN_RATIO;
  const maxChroma = v.maxChroma ?? MAX_CHROMA;
  check(
    r.noise < 1000,
    `${r.label}: the probe's same-state control differs on ${r.noise} px — the measurement is noise`,
  );
  check(
    r.ship >= MIN_CONTRAST,
    `${r.label}: near-ground neighbour contrast is ${r.ship.toFixed(2)} luma/px against a floor of ` +
      `${MIN_CONTRAST} (ART.md §3's reference target is 6.3) — 15h's statistic, asked where 15h cannot stand`,
  );
  check(
    r.ratio >= minRatio,
    `${r.label}: the maps deliver x${r.ratio.toFixed(2)} the procedural ground's contrast (floor ` +
      `x${minRatio}${v.minRatio ? `, a regression wall — the bar is x${MIN_RATIO}` : ""}) — the ` +
      `photograph is not reaching this surface. A causal modifier that REPLACES albedo or roughness ` +
      `with a constant lands here and passes every gate measured on the beach.`,
  );
  check(
    r.chroma <= maxChroma,
    `${r.label}: chroma per unit luma is ${r.chroma.toFixed(3)} over a ceiling of ${maxChroma}` +
      `${v.maxChroma ? ` (a regression wall — the ceiling is ${MAX_CHROMA})` : ""} — Rust Images/ ` +
      `reach ${REF_CHROMA_TARGET} at worst. This ground is aliasing, not texturing.`,
  );
  // Two-sided: a wall that only catches regressions never notices the day it
  // could be retired. If a vantage comes in far under its wall, the wall is
  // stale and the row that set it is now wrong about the tree.
  if (v.maxChroma !== undefined) {
    check(
      r.chroma > MAX_CHROMA * 0.9,
      `${r.label}: chroma is ${r.chroma.toFixed(3)}, at or under the ${MAX_CHROMA} bar this vantage ` +
        `carries a ${v.maxChroma} regression wall instead of. The defect the wall was recording is ` +
        `fixed — delete the override and let this vantage hold the real ceiling.`,
    );
  }
}

console.log(`  vantage gate: ${checks} checks passed over ${VANTAGES.length} vantages and 2 seeds`);

// Teardown, and it has to happen HERE rather than in the `exit` handler above.
// That handler is the whole bug: node fires `exit` when it runs out of live
// handles, and the browser connection, the static server and the shard children
// ARE live handles — so cleanup waited for an exit that cleanup was the only
// thing that could cause. Measured 2026-08-04: every check on this gate passed
// and the process then sat for 52 minutes, which `ci/gates.sh` waits on
// forever. A gate that passes and never returns reads exactly like a hung one.
//
// `browser.close()` is a promise, so it is awaited before the exit rather than
// fired into one — a Playwright handle that outlives the process is how the
// orphaned chromium in this box's other failure mode gets made. The registered
// handler stays for the `fail()` path, where a best-effort sync kill is all
// that is available.
try {
  await browser?.close();
} catch {}
cleanup();
process.exit(0);
