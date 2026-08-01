#!/usr/bin/env node
// Browser smoke — the gate that would have caught the two bugs of 2026-07-31.
//
// Every other gate tests the client's LOGIC: client-wasm unit tests, the node
// bridge smoke, and server/tests/client_loop.rs (the client core against a real
// ShardCore over real datagrams). All honest, all native or node — so the JS
// boot path in web/src/*.js was never executed by anything. With 46 tests green
// and a judge PASS, the browser client could not start at all:
//
//   1. WasmViews.refresh() captured ex.memory.buffer, then called a ptr getter
//      in the same expression. The getter allocates on first call, grows wasm
//      memory, and detaches the captured buffer → "Cannot perform Construct on
//      a detached ArrayBuffer" before a single packet moved.
//   2. The terrain worker's `ex` is assigned inside an async handler, and
//      async onmessage does not serialize messages, so build requests arrived
//      while loadWasm was still in flight → "Cannot read properties of null".
//      The far mesh still rendered, so a screenshot looked fine.
//
// So this gate asserts what a frame cannot: the client REACHES THE WORLD, and
// NOTHING throws while it plays. Bug 2 only shows up as a page error, which is
// why zero-page-errors is an assertion and not a warning.
//
// It runs TWO browser contexts, because M0's exit condition (DESIGN.md §11) is
// two clients seeing each other walk. The shard gets `dev_spawn` (DECISIONS.md
// §open) so both land on one point — normal scatter is 224–1,824 m on a
// 2,048 m island, far outside the 176 m AOI enter. Each page then asserts the
// OTHER page's remote displaces while its key is held: `remotes 1` alone can't
// tell a frozen remote from a live one, movement can.
//
// It also guards the dev gate on the client's dev affordances, because that
// gate has no other home: `__gatesDebug.setView` (the capture harness's camera
// hook) is installed only when the shard's welcome says `dev`, and the only
// place that if-statement actually runs is a browser. So this gate boots a
// SECOND shard with no dev override — a public shard's config — and asserts the
// hook is absent there and present, aiming, on the dev one.
//
// Any missing dependency is a loud failure, never a silent skip.

import { spawn } from "node:child_process";
import http from "node:http";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { createRequire } from "node:module";

const root = path.join(path.dirname(fileURLToPath(import.meta.url)), "..");
const DIST = path.join(root, "web/dist");
const SHARD = path.join(root, "target/release/shard");
const PORT = Number(process.env.BROWSER_SMOKE_PORT || 8934);
// UDP port the temp shard binds; overridable so two smoke runs (or a smoke
// beside a dev shard) don't fight over 4433.
const WIRE_PORT = Number(process.env.BROWSER_SMOKE_WIRE_PORT || 4433);
// The public-config shard (no dev_spawn) the dev-gate check joins.
const PUBLIC_WIRE_PORT = Number(process.env.BROWSER_SMOKE_PUBLIC_WIRE_PORT || WIRE_PORT + 1);
const JOIN_TIMEOUT_MS = Number(process.env.BROWSER_SMOKE_TIMEOUT_MS || 60000);
const PLAY_MS = Number(process.env.BROWSER_SMOKE_PLAY_MS || 6000);
// Separation the chat assertion walks the two tabs to before claiming a local
// line is out of earshot. Comfortably past the 20 m radius (DECISIONS.md
// §open, "local chat") so interpolation lag on a shared box can't put the
// listener back inside it.
const CHAT_APART_M = Number(process.env.BROWSER_SMOKE_CHAT_APART_M || 30);
// Seed and point are guarded natively: sim-core world::tests asserts this
// exact spawn is walkable at this exact seed, so worldgen drift fails there
// first, with a message, instead of here as a mystery.
const SEED = 20260731;
const DEV_SPAWN = "1024,1024";
// Held-walk displacement floor, metres planar. Walk speed is 3 m/s over
// PLAY_MS of held key (~18 m); 2 m stays green under heavy same-box load
// while still failing a frozen or never-updated remote outright.
const MOVE_MIN_M = 2;
// The aim the dev hook is driven to: yaw pi/2 faces +X (sim-core yaw_lut —
// 0 faces +Z, increasing turns toward +X), pitch below level so the clamp is
// not what is being measured. Walking after it must carry the player east.
const AIM_YAW = Math.PI / 2;
const AIM_PITCH = -0.3;
const AIM_EPS = 1e-3;

const fail = (msg) => {
  console.error(`GATE FAIL: ${msg}`);
  process.exit(1);
};

// --- dependencies, each a loud failure -------------------------------------
const require = createRequire(path.join(root, "web/package.json"));
let chromium;
try {
  // playwright's entry is CJS: importing it puts the exports under .default,
  // and a bare specifier would resolve against ci/ rather than web/.
  const mod = await import(pathToFileURL(require.resolve("playwright")).href);
  chromium = mod.chromium ?? mod.default?.chromium;
  if (!chromium) throw new Error("playwright exported no chromium");
} catch (e) {
  fail(
    `playwright not installed (web devDependency): ${e.message}\n` +
      "  install it: (cd web && npm install --include=dev)",
  );
}
if (!fs.existsSync(SHARD)) {
  fail(`shard binary missing at ${SHARD}\n  build it: cargo build -p server --bin shard --release`);
}
if (!fs.existsSync(path.join(DIST, "index.html"))) {
  fail(`web bundle missing at ${DIST}\n  build it: (cd web && npx vite build)`);
}
if (!fs.existsSync(path.join(DIST, "client_wasm.wasm"))) {
  fail(`client_wasm.wasm missing from ${DIST} — the bundle is not playable`);
}

let server, browser, tmpDir;
const shards = [];
const cleanup = () => {
  try { browser && browser.close(); } catch {}
  try { server && server.close(); } catch {}
  for (const s of shards) { try { s.kill("SIGTERM"); } catch {} }
  try { tmpDir && fs.rmSync(tmpDir, { recursive: true, force: true }); } catch {}
};
process.on("exit", cleanup);

// --- real shards on real UDP ports ------------------------------------------
// `dev_spawn` set = a dev shard: spawns pinned to one point AND, from wire v7,
// a welcome that says `dev`. Unset = exactly a public shard's config.
tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "gates-smoke-"));
const startShard = (label, port, devSpawn) => {
  const cfgPath = path.join(tmpDir, `${label}.toml`);
  fs.writeFileSync(
    cfgPath,
    `bind = "127.0.0.1:${port}"\nseed = ${SEED}\n` +
      (devSpawn ? `dev_spawn = "${devSpawn}"\n` : ""),
  );
  const log = [];
  const proc = spawn(SHARD, [cfgPath], { cwd: root, env: { ...process.env, RUST_LOG: "warn" } });
  shards.push(proc);
  const ready = new Promise((resolve, reject) => {
    const t = setTimeout(() => reject(new Error(`${label} shard printed no cert hash in 30s`)), 30000);
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
    proc.on("exit", (c) => reject(new Error(`${label} shard exited (${c}): ${log.join(" | ")}`)));
  });
  return { label, port, log, ready };
};
const devShard = startShard("dev", WIRE_PORT, DEV_SPAWN);
const publicShard = startShard("public", PUBLIC_WIRE_PORT, null);
let certHash = null;
let publicCertHash = null;
try {
  [certHash, publicCertHash] = await Promise.all([devShard.ready, publicShard.ready]);
} catch (e) {
  fail(e.message);
}

// --- serve the built bundle, with the production COOP/COEP headers ----------
const MIME = { ".html": "text/html", ".js": "text/javascript", ".wasm": "application/wasm", ".css": "text/css" };
server = http.createServer((req, res) => {
  const rel = decodeURIComponent(req.url.split("?")[0]);
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

// --- the browser ------------------------------------------------------------
try {
  browser = await chromium.launch({
    headless: true,
    args: [
      // No GPU on the reference box or this one: ANGLE over SwiftShader.
      "--enable-unsafe-swiftshader", "--use-gl=angle", "--use-angle=swiftshader", "--ignore-gpu-blocklist",
      // Two live contexts: neither may be throttled as "background" or the
      // idle one stops pumping inputs and its remote really does freeze.
      "--disable-renderer-backgrounding", "--disable-backgrounding-occluded-windows", "--disable-background-timer-throttling",
    ],
  });
} catch (e) {
  fail(`chromium failed to launch: ${e.message}\n  install it: npx playwright install chromium`);
}

// One context per tab: separate sessions, separate localStorage — the same
// isolation two real players have.
const join = async (label, port = WIRE_PORT, cert = certHash) => {
  const context = await browser.newContext({ viewport: { width: 1280, height: 720 } });
  const page = await context.newPage();
  const errors = [];
  page.on("pageerror", (e) => errors.push(`pageerror: ${e.message}`));
  page.on("console", (m) => { if (m.type() === "error") errors.push(`console.error: ${m.text()}`); });

  await page.goto(`http://127.0.0.1:${PORT}/`, { waitUntil: "load" });
  await page.fill("#url", `https://127.0.0.1:${port}`);
  await page.fill("#cert", cert);
  await page.click("#connect");

  // Assertion 1 — the client reaches the world. Catches bug 1, and any
  // handshake, ring, AOI or delta-encoder break along the way.
  const t0 = Date.now();
  let dbg = null;
  while (Date.now() - t0 < JOIN_TIMEOUT_MS) {
    dbg = await page.evaluate(() => globalThis.__gatesDebug || null);
    if (dbg && dbg.inWorld && dbg.snapshots > 0) break;
    const err = await page.evaluate(() => document.getElementById("starterr")?.textContent || "");
    if (err) fail(`${label}: client refused to boot: ${err}`); // #starterr, where boot() puts failures
    await page.waitForTimeout(250);
  }
  if (!dbg || !dbg.inWorld) fail(`${label}: never reached the world in ${JOIN_TIMEOUT_MS}ms`);
  console.log(`  ${label}: in world as player ${dbg.playerId} at [${dbg.own.map((v) => v.toFixed(1))}]`);
  return { label, page, errors, playerId: dbg.playerId, dbg };
};

const A = await join("tab A");
const B = await join("tab B");
if (A.playerId === B.playerId) fail(`both tabs joined as player ${A.playerId}`);

// Each page must see the other's remote before anyone moves — dev_spawn puts
// them on the same point, 0 m apart, far inside the 176 m AOI enter.
const remoteOf = (tab, id) =>
  tab.page.evaluate(
    (want) => (globalThis.__gatesDebug?.remotes || []).find((r) => r[0] === want) || null,
    id,
  );
const waitForRemote = async (tab, id) => {
  const t0 = Date.now();
  while (Date.now() - t0 < JOIN_TIMEOUT_MS) {
    const r = await remoteOf(tab, id);
    if (r) return r;
    await tab.page.waitForTimeout(250);
  }
  fail(`${tab.label}: never saw player ${id} in AOI — both spawned at dev_spawn ${DEV_SPAWN}`);
};
const seenA = await waitForRemote(A, B.playerId); // A sees B
const seenB = await waitForRemote(B, A.playerId); // B sees A
console.log(`  mutual AOI: A sees ${B.playerId}, B sees ${A.playerId}`);

// --- chat, part 1: heard at the spawn --------------------------------------
// Assertion 2 — a line typed into the real composer in one browser reaches
// the other browser's log. Driven entirely through the UI (T, type, Enter)
// and read back off the DOM: no test-only hook, so what passes here is the
// path a player uses. Said HERE, before the walk, because the walk carries
// the two tabs past the 20 m local radius — which part 2 then relies on.
const chatlog = (tab) =>
  tab.page.evaluate(() => document.getElementById("chatlog").textContent || "");

const say = async (tab, text) => {
  await tab.page.keyboard.press("KeyT");
  await tab.page.keyboard.type(text);
  await tab.page.keyboard.press("Enter");
};

// `want` in the log within the window, or null. Polls, because the line
// crosses a real network and a real tick.
const waitForLine = async (tab, want, ms = 5000) => {
  const until = Date.now() + ms;
  for (;;) {
    const log = await chatlog(tab);
    if (log.includes(want)) return log;
    if (Date.now() > until) return null;
    await tab.page.waitForTimeout(250);
  }
};

const ownXZ = async (tab) => {
  const own = await tab.page.evaluate(() => globalThis.__gatesDebug.own);
  return [own[0], own[2]];
};
const apartNow = async () => {
  const [ax, az] = await ownXZ(A);
  const [bx, bz] = await ownXZ(B);
  return Math.hypot(bx - ax, bz - az);
};

const LOCAL_LINE = "stone at the ridge";
await say(A, LOCAL_LINE);
const heardLocalB = await waitForLine(B, LOCAL_LINE);
if (!heardLocalB) {
  fail(`tab B never heard A's local line from ${(await apartNow()).toFixed(1)} m away`);
}
if (!heardLocalB.includes(`#${A.playerId}`)) {
  fail(`tab B heard the line but not from #${A.playerId}: ${heardLocalB.trim()}`);
}
const heardLocalA = await waitForLine(A, LOCAL_LINE);
if (!heardLocalA) fail(`tab A never got its own echo — the delivery receipt is missing`);
console.log(`  chat: A's local line reached B, and A's own echo came back`);

// Play: A walks forward, B walks backward — opposite headings off the shared
// point. The terrain worker only builds once a player moves, so the window
// where bug 2 fires is AFTER the first snapshot.
await A.page.keyboard.down("KeyW");
await B.page.keyboard.down("KeyS");
await A.page.waitForTimeout(PLAY_MS);
await A.page.keyboard.up("KeyW");
await B.page.keyboard.up("KeyS");

const finalA = await remoteOf(A, B.playerId);
const finalB = await remoteOf(B, A.playerId);
const dbgA = await A.page.evaluate(() => globalThis.__gatesDebug);
const dbgB = await B.page.evaluate(() => globalThis.__gatesDebug);

// Assertion 3 — nothing threw on either page. Catches bug 2, which is
// invisible in a frame.
for (const tab of [A, B]) {
  if (tab.errors.length) {
    fail(
      `${tab.label}: ${tab.errors.length} page error(s) while playing — the client is throwing:\n` +
        tab.errors.slice(0, 8).map((e) => `    ${e}`).join("\n"),
    );
  }
}
// Assertion 4 — the wire actually moved on both sessions.
if (!(dbgA.snapshots > 0)) fail(`tab A: no snapshots received`);
if (!(dbgB.snapshots > 0)) fail(`tab B: no snapshots received`);
// Assertion 5 — CLAUDE.md's trap: an oversize browser datagram silently sends
// nothing. Zero is the only acceptable count.
if (dbgA.oversize !== 0) fail(`tab A: ${dbgA.oversize} oversize datagram(s) — clamp against the live maxDatagramSize`);
if (dbgB.oversize !== 0) fail(`tab B: ${dbgB.oversize} oversize datagram(s) — clamp against the live maxDatagramSize`);
// Assertion 6 — the M0 exit condition: each tab watched the OTHER walk. A
// frozen remote (dead interp, stalled input pump, throttled RAF) fails here.
const planar = (a, b) => Math.hypot(b[1] - a[1], b[3] - a[3]);
const moveA = finalA ? planar(seenA, finalA) : 0; // B's walk, seen from A
const moveB = finalB ? planar(seenB, finalB) : 0; // A's walk, seen from B
if (!finalA) fail(`tab A: player ${B.playerId} left AOI mid-walk`);
if (!finalB) fail(`tab B: player ${A.playerId} left AOI mid-walk`);
if (moveA < MOVE_MIN_M) fail(`tab A watched player ${B.playerId} move ${moveA.toFixed(2)} m — a live remote walks ≥ ${MOVE_MIN_M} m`);
if (moveB < MOVE_MIN_M) fail(`tab B watched player ${A.playerId} move ${moveB.toFixed(2)} m — a live remote walks ≥ ${MOVE_MIN_M} m`);

console.log(`  snapshots A ${dbgA.snapshots} B ${dbgB.snapshots} · oversize 0 · page errors 0`);
console.log(`  mutual movement: A saw B walk ${moveA.toFixed(1)} m, B saw A walk ${moveB.toFixed(1)} m`);

// --- chat, part 2: silent out of earshot ------------------------------------
// Assertion 7 — the 20 m local radius is a real edge, not a constant nobody
// applies. The radius itself is pinned to the quantum natively in
// server/tests/chat_wire.rs; what only a browser can prove is that the keys,
// the encoder, the stream, the fan-out and the log are one chain that also
// knows how to stay quiet.
//
// Walk them apart first. Bounded rounds, and a loud failure if they never
// separate — never a skipped assertion.
let apart = await apartNow();
for (let round = 0; round < 5 && apart < CHAT_APART_M; round++) {
  await A.page.keyboard.down("KeyW");
  await B.page.keyboard.down("KeyS");
  await A.page.waitForTimeout(3000);
  await A.page.keyboard.up("KeyW");
  await B.page.keyboard.up("KeyS");
  await A.page.waitForTimeout(400);
  apart = await apartNow();
}
if (apart < CHAT_APART_M) {
  fail(
    `tabs only walked ${apart.toFixed(1)} m apart in 5 rounds — the local-radius ` +
      `assertion needs ≥ ${CHAT_APART_M} m of separation to mean anything`,
  );
}

const FAR_LOCAL = "cannot hear this";
const GLOBAL_LINE = "wipe is at six";
await say(A, FAR_LOCAL);
await say(A, `/g ${GLOBAL_LINE}`);
const heardGlobalB = await waitForLine(B, GLOBAL_LINE);
if (!heardGlobalB) fail(`tab B never heard A's global line at ${apart.toFixed(1)} m`);
if (heardGlobalB.includes(FAR_LOCAL)) {
  fail(
    `tab B heard a LOCAL line from ${apart.toFixed(1)} m away — the 20 m radius ` +
      `is not being applied`,
  );
}
for (const tab of [A, B]) {
  if (tab.errors.length) {
    fail(
      `${tab.label}: ${tab.errors.length} page error(s) during chat:\n` +
        tab.errors.slice(0, 8).map((e) => `    ${e}`).join("\n"),
    );
  }
}
console.log(`  chat: local silent at ${apart.toFixed(1)} m, global heard through`);

// --- the dev view hook ------------------------------------------------------
// Assertion 8 — on a DEV shard the hook exists and actually aims. Headless
// pointer lock yields no movementX (gates-loop/art/probe-pointerlock.mjs), so
// this is the only path that can point the camera, and a hook that silently
// did nothing would leave the capture harness shooting spawn yaw every pass
// while reporting six distinct vantages.
if (!dbgA.dev) fail(`tab A: dev shard welcomed with dev=false — the client cannot install its dev hooks`);
// typeof INSIDE the page: page.evaluate cannot serialize a function, so
// returning the hook itself would read undefined no matter what is there.
const devHook = await A.page.evaluate(() => typeof globalThis.__gatesDebug.setView);
if (devHook !== "function") {
  fail(`tab A: __gatesDebug.setView is ${devHook} on a dev shard (dev_spawn is set) — art/capture.mjs is blocked`);
}
const aimed = await A.page.evaluate(
  ([y, p]) => globalThis.__gatesDebug.setView(y, p),
  [AIM_YAW, AIM_PITCH],
);
if (aimed !== true) fail(`tab A: setView(${AIM_YAW}, ${AIM_PITCH}) returned ${aimed}`);
// It has to survive into the next published snapshot, not just return true.
await A.page.waitForTimeout(600);
const view = await A.page.evaluate(() => globalThis.__gatesDebug.view);
if (Math.abs(view[0] - AIM_YAW) > AIM_EPS || Math.abs(view[1] - AIM_PITCH) > AIM_EPS) {
  fail(`tab A: aimed at [${AIM_YAW}, ${AIM_PITCH}] but the camera reads [${view}]`);
}
// And the aim must reach the SIM, not just the camera: yaw pi/2 faces +X, so a
// held W walks east. A hook that moved the render camera alone would pass the
// readback above and still frame a player walking sideways out of shot.
const beforeAim = (await A.page.evaluate(() => globalThis.__gatesDebug.own)).slice();
await A.page.keyboard.down("KeyW");
await A.page.waitForTimeout(PLAY_MS);
await A.page.keyboard.up("KeyW");
const afterAim = await A.page.evaluate(() => globalThis.__gatesDebug.own);
const dx = afterAim[0] - beforeAim[0];
const dz = afterAim[2] - beforeAim[2];
if (dx < MOVE_MIN_M || Math.abs(dz) > dx) {
  fail(
    `tab A: after setView(yaw pi/2) a held W moved [${dx.toFixed(2)}, ${dz.toFixed(2)}] m — ` +
      `yaw pi/2 faces +X, so the walk must be east-dominant and ≥ ${MOVE_MIN_M} m. ` +
      `The hook is not reaching the input the sim runs on.`,
  );
}
console.log(`  dev hook: aimed to [${view.map((v) => v.toFixed(2))}], walked +X ${dx.toFixed(1)} m (dz ${dz.toFixed(1)})`);

// Assertion 9 — the gate itself. A shard with no dev override is exactly a
// public shard's config, and its client must have no dev surface at all.
const P = await join("public tab", PUBLIC_WIRE_PORT, publicCertHash);
if (P.dbg.dev !== false) fail(`public tab: shard without dev_spawn welcomed with dev=${P.dbg.dev}`);
const publicSetView = await P.page.evaluate(() => typeof globalThis.__gatesDebug.setView);
if (publicSetView !== "undefined") {
  fail(`public tab: __gatesDebug.setView is ${publicSetView} on a shard with no dev override — a dev affordance shipped to a public shard`);
}
if (P.errors.length) {
  fail(`public tab: ${P.errors.length} page error(s):\n` + P.errors.slice(0, 8).map((e) => `    ${e}`).join("\n"));
}
console.log(`  dev gate: public shard welcomes dev=false, client installs no setView`);

console.log("browser smoke: all checks passed");
cleanup();
process.exit(0);
