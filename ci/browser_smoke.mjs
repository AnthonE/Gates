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
// It is also where the LIGHTING rig is gated, for the same reason: a shadow map
// only exists inside a renderer. The flags (map enabled, key casting, a tone
// map that is not NoToneMapping) are the cheap half; the assertion that matters
// renders the live scene twice per sample yaw — key light casting, then not —
// reads both frames back off the drawing buffer and requires the shadow pass to
// have actually taken pixels down. Every flag can be true while the image is
// unchanged: a bias that pushes every sample past its caster, a coverage box
// parked somewhere else, casters that never got castShadow. Only a frame knows.
// The same two renders also count the shadow pass's draw calls, which is what
// makes DESIGN §9's < 300 calls / < 1.5 M tris budget assertable here at all.
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
// --- the lighting gate (DECISIONS.md §open, "lighting v0") ------------------
// The shadow probe sweeps four yaws so the assertion does not depend on the
// player having walked to a spot with a caster in one particular direction,
// and looks slightly down so the ground the shadows land on fills the frame.
const SHADOW_PROBE_YAWS = [0, Math.PI / 2, Math.PI, (3 * Math.PI) / 2];
const SHADOW_PROBE_PITCH = -0.45;
// The near ring streams one chunk at a time; probing before it lands would
// score a frame with most of its casters missing.
const SHADOW_SETTLE_MS = Number(process.env.BROWSER_SMOKE_SHADOW_SETTLE_MS || 4000);
// A pixel counts as shadowed when the shadow pass took its luma down by more
// than this out of 255 — comfortably above SwiftShader's dither/AA noise
// between two renders of an identical frame, well below a real shadow (whose
// mean delta on this scene runs in the tens).
const SHADOW_PROBE_MIN_DELTA = 6;
// Floor on the darkened share of the sweep. Raised from 0.015 by the slice
// that made the GROUND cast: with the terrain culled out of every depth pass
// (three casts a FrontSide material from its back face, and a heightfield has
// none turned at the sky) the pinned spawn measured 10.5%, worst yaw 3.6% —
// all of it from the scatter and the other tab's avatar. With hills casting it
// measures 24.0%, worst yaw 20.4%, stable across runs. So these floors now
// bite on the real failure and not only on the total absence of a rig: a
// terrain material that forgets `shadowSide` scores 10.5% and 3.6%.
const SHADOW_MIN_FRACTION = Number(process.env.BROWSER_SMOKE_SHADOW_MIN || 0.15);
// And a floor on EVERY yaw, which is the assertion that actually pins the
// WORLD as a caster. Dropping castShadow from the terrain and the scatter
// pools still cleared the aggregate above: the other tab's avatar stands on
// the shared spawn point and, under a 21° sun, throws a long enough shadow
// across a downward-pitched frame to score 6% on its own — from two of the
// four yaws, with the other two at exactly 0.0%. Worst real yaw was 2.8% then
// and is 20.4% now that the ground itself casts, which is what let the floor
// below move from 1% to 10%.
const SHADOW_MIN_FRACTION_PER_YAW = Number(process.env.BROWSER_SMOKE_SHADOW_MIN_YAW || 0.1);
// Same failure from the other side, counted rather than sampled: the seven
// scatter pools are frustumCulled=false, so a rig where the world casts
// submits at least them to the shadow pass. That mutation submitted 2 (the
// avatar's two meshes); the intact rig submits 25.
const SHADOW_PASS_MIN_CALLS = Number(process.env.BROWSER_SMOKE_SHADOW_MIN_CALLS || 8);
const SHADOW_MIN_MAP_PX = 1024;
// --- the clipmap gate (DECISIONS.md §open, "shadow clipmap v0") -------------
// Two probes, because the slice makes two separate claims: that the coarse
// levels DARKEN pixels the near level cannot reach, and that those pixels are
// genuinely outside the near level's box rather than just far-ish.
//
// The camera is aimed perpendicular to the sun's bearing, read off the scene
// rather than pinned here. Light-space X is the horizontal axis across the
// sun, and it is the only axis on which the near level's 80 m half-width is
// 80 m of GROUND: along the sun's bearing the same box reaches 80/sin(21°) ≈
// 227 m, so a "past 80 m" claim measured there would be worth nothing.
// Lift the viewpoint and pitch it down, so the far band is most of the frame
// rather than a strip under the horizon (from eye height the same probe
// measured 0.10% — real shadow, at 27 mean Δluma, on almost no pixels). The
// clipmap's centre stays on the player, so the lift changes the sample size
// and nothing else.
const FAR_SHADOW_HEIGHT_M = 80;
const FAR_SHADOW_PROBE_PITCH = -0.42;
// Push the near plane out and narrow the frame, so nothing inside the near
// level's reach is drawn at all. The probe then MEASURES the resulting
// distance (min |light-space x − centre| over all eight frustum corners) and
// hands it back; the assertion below is on the measurement, not on these.
const FAR_SHADOW_NEAR_M = 120;
const FAR_SHADOW_FOV_DEG = 28;
const FAR_SHADOW_MIN_DELTA = 6;
// Floors on the share of that far-only frame the coarse levels darken. The
// pinned spawn measures 0.78% (per yaw 1.3 / 0.3), so these leave ~3x of
// headroom; the failure they guard — a rig whose shadows stop at the near
// box, which is exactly what shipped before this slice — scores 0.00%, and
// that zero is measured rather than assumed (see the control below).
// Small in absolute terms because most of a downward frame at this range is
// lit ground: what is asserted is that the shadow is THERE, and the mean
// Δluma the failure message prints (~29 of 255) says it is not noise.
const FAR_SHADOW_MIN_FRACTION = Number(process.env.BROWSER_SMOKE_FAR_SHADOW_MIN || 0.0025);
const FAR_SHADOW_MIN_FRACTION_PER_YAW = Number(
  process.env.BROWSER_SMOKE_FAR_SHADOW_MIN_YAW || 0.001,
);
// A coarse level may only ever REMOVE light. Any pixel the clipmap makes
// BRIGHTER means the extra levels are lighting the scene instead of
// shadowing it — the failure a zero-intensity level exists to prevent, and
// the one that would otherwise show up as a washed-out frame nobody gates.
const FAR_SHADOW_MAX_LIFTED_FRACTION = 1e-4;
// --- the horizon-caster gate (DECISIONS.md §open, "the horizon casts") ------
// Everything that cast before this slice lived in the near ring: the 5x5x64 m
// chunk box, its scatter, pieces and players. What this slice adds is the far
// mesh, casting everywhere the ring is NOT — so the measurement that proves it
// is not a camera pushed out to some distance, it is the caster taken away.
// The horizon probe's own sweep: four yaws, lifted so the ground past the ring
// is most of the frame rather than a strip under the horizon, and pitched down
// like the other two. There is no near-plane trick here and none is needed —
// the caster it removes is, by construction, only outside the ring.
const HORIZON_PROBE_YAWS = [0, Math.PI / 2, Math.PI, (3 * Math.PI) / 2];
const HORIZON_PROBE_PITCH = -0.42;
const HORIZON_PROBE_HEIGHT_M = 80;
// Floors from the measurement with the usual headroom; the failure they guard
// — a far mesh that receives and never casts, which is what shipped before
// this slice — scores exactly 0, and the probe's control says its zero point
// is a real zero. Measured 0.54% of the sweep at mean Δluma 45-48 (which is
// not a marginal darkening: it is the same order as the near shadows).
const HORIZON_MIN_FRACTION = Number(process.env.BROWSER_SMOKE_HORIZON_MIN || 0.0015);
// Deliberately NOT a floor on every yaw, unlike the near shadow probe. Past
// the ring the only caster is the ground itself — the scatter stops at the
// ring's edge — so a direction casts only where the island has relief steep
// enough to shade at a 21° sun, and which side of the spawn that is, is
// worldgen's fact and not the caster's. Measured 1.57% / 0.57% on two of the
// four yaws and ~0 on the other two. So: how many directions show it, with
// room for one of the two real ones to be a thin frame.
const HORIZON_MIN_DIRECTIONS = 2;
const HORIZON_MIN_FRACTION_PER_YAW = Number(process.env.BROWSER_SMOKE_HORIZON_MIN_YAW || 0.001);
// The island is this wide (TERRAIN.md §4), so a hole half-extent at or past it
// is not a ring footprint — it is the probe's own suppression left behind.
const ISLAND_M = 2048;
// three's FrontSide. The ground must name its shadow side: three's default for
// a FrontSide material is to cast from the BACK face — the right answer for a
// closed solid and the wrong one for a heightfield, which has no back face
// pointed at the sky and is therefore culled out of the depth pass entirely.
const THREE_FRONT_SIDE = 0;
// DESIGN §9's client budget, gated for the first time here. Both counts are
// per rendered frame and INCLUDE the shadow pass, which is the point: a
// second pass over every caster is the obvious way to eat this budget.
const DRAW_CALL_BUDGET = 300;
const TRIANGLE_BUDGET = 1_500_000;
// --- the materials gate (DECISIONS.md §open, "materials v0") ----------------
// Same three-part shape as the lighting gate above, for the same reason: the
// flags say a material was configured, the census says what the shader is
// actually FED, and only a pair of frames says what reached the image.
const SURFACE_PROBE_YAWS = [0, Math.PI / 2, Math.PI, (3 * Math.PI) / 2];
// Steeper than the shadow probe's: this one is about the GROUND's material,
// so the frames it scores should be mostly ground in every direction.
const SURFACE_PROBE_PITCH = -0.7;
// A pixel counts as moved when the field took its luma up or down by more
// than this out of 255 — the same floor the shadow probe uses, and for the
// same reason (SwiftShader's dither noise between two renders sits under it).
const SURFACE_PROBE_MIN_DELTA = 6;
// Floors on the swept pixels the procedural field moves. The pinned spawn
// measures 12.8% (per yaw 2.4 / 6.8 / 8.4 / 33.7), stable to the decimal
// across runs; the failure they guard — a material whose field contributes
// nothing — scores exactly 0.
const SURFACE_MIN_FRACTION = Number(process.env.BROWSER_SMOKE_SURFACE_MIN || 0.04);
const SURFACE_MIN_FRACTION_PER_YAW = Number(process.env.BROWSER_SMOKE_SURFACE_MIN_YAW || 0.01);
// And the half that is hard to fake, because the two above were SHOWN not to
// bite on their own: collapsing the noise scales so the whole ring lands in
// one lattice cell moved 66–96% of the pixels — a uniform darkening — and
// sailed past both floors. Microstructure lightens some pixels and darkens
// others; a constant field, a global tint or an exposure slip cannot. That
// mutation scored +0.00% up on two of the four yaws against a worst real
// yaw of +0.5%.
const SURFACE_MIN_DIRECTIONAL = Number(process.env.BROWSER_SMOKE_SURFACE_MIN_DIR || 0.002);
// The splat weights the shader is fed, over the streamed near ring. Constant
// or one-hot weights render a perfectly convincing single material, so:
// at least this many identities must hold real ground, and this much of the
// ring must be a genuine two-identity blend rather than a hard biome cell.
const SPLAT_MIN_IDENTITIES = 2;
const SPLAT_IDENTITY_SHARE = 0.01;
// Measured at the pinned spawn: spreads 1.00 / 1.00 / 0.00 / 0.16, deepest
// second identity 0.50, 1.5% of the ring blended. Pinning the weights to a
// constant vector takes all four spreads to 0.000.
const SPLAT_MIN_SPREAD = Number(process.env.BROWSER_SMOKE_SPLAT_MIN_SPREAD || 0.15);
const SPLAT_MIN_SECOND = Number(process.env.BROWSER_SMOKE_SPLAT_MIN_SECOND || 0.35);
// Area, not just an extremum: the transition must be a band, not one row of
// vertices. Which biomes a 320 m ring contains is a worldgen fact (the
// moisture channel's features are ~700 m across, so a ring is often one
// biome) — this floor is set from the pinned spawn's measured 1.5%.
const SPLAT_MIN_MIXED = Number(process.env.BROWSER_SMOKE_SPLAT_MIN_MIXED || 0.005);

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
  if (!dbg || !dbg.inWorld) {
    // Say WHY. "never reached the world" alone is the same message for a
    // handshake that never landed, a shard that died, and a client whose
    // debug publisher threw on its first tick — three very different bugs.
    fail(
      `${label}: never reached the world in ${JOIN_TIMEOUT_MS}ms ` +
        `(__gatesDebug ${dbg ? `present, inWorld=${dbg.inWorld}, snapshots=${dbg.snapshots}` : "never published"})` +
        (errors.length ? `\n` + errors.slice(0, 8).map((e) => `    ${e}`).join("\n") : "\n    no page errors"),
    );
  }
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

// --- lighting: the key light and the shadow map -----------------------------
// Measured HERE, before the walk, on purpose: tab A is still standing on
// `dev_spawn` at a pinned seed, so the frames the probe scores are the same
// frames every pass. After the walk the position depends on how much wall
// clock a shared box gave the input pump, and the floor below would be
// scoring a different place each run.
//
// Assertion 9 — the rig is wired. Cheap and structural, and it exists so that
// assertion 10's failure is diagnosable: "no darkening" means something very
// different when the shadow map was never enabled in the first place.
await A.page.waitForTimeout(SHADOW_SETTLE_MS); // the near ring streams in
const lit = await A.page.evaluate(() => globalThis.__gatesDebug.lighting);
if (!lit) fail(`tab A: __gatesDebug.lighting missing — the scene publishes no lighting state`);
if (lit.shadowMap !== true) fail(`tab A: renderer.shadowMap.enabled is ${lit.shadowMap}`);
if (lit.sunCasts !== true) fail(`tab A: the key light has castShadow=${lit.sunCasts}`);
if (!(lit.mapSize >= SHADOW_MIN_MAP_PX)) {
  fail(`tab A: shadow map is ${lit.mapSize} px — below ${SHADOW_MIN_MAP_PX}, silhouettes cannot resolve`);
}
if (!(lit.radiusM > 0)) fail(`tab A: shadow coverage radius is ${lit.radiusM} m`);
if (!(lit.normalBias > 0)) fail(`tab A: shadow normalBias is ${lit.normalBias} — acne is not being biased away`);
// Tone mapping is the other half of "grade toward a darker edge": 0 is
// THREE.NoToneMapping, i.e. nothing owns the highlight roll-off.
if (!(lit.toneMapping > 0)) fail(`tab A: renderer.toneMapping is NoToneMapping — nothing owns the transfer`);
if (!(lit.exposure > 0)) fail(`tab A: toneMappingExposure is ${lit.exposure}`);
// The fill must not be able to wash the key out; a fill at or above the key
// is the flat-hemisphere state this work exists to end.
if (!(lit.sunIntensity > lit.fillIntensity)) {
  fail(
    `tab A: key ${lit.sunIntensity} is not above fill ${lit.fillIntensity} — ` +
      `a fill that strong flattens every shadow the map draws`,
  );
}

// Assertion 9b — the clipmap is a clipmap. Structural, and it exists for the
// same reason as 9: so 11b's failure is diagnosable. Every check here is on a
// property the reference names — concentric levels, a real guard band, bias
// scaled BY texel width rather than shared across levels, and exactly one
// level carrying the key's energy.
const cm = lit.clipmap;
if (!cm) fail(`tab A: lighting publishes no clipmap state — the levels are not wired`);
if (!(cm.levelCount >= 2)) {
  fail(`tab A: the shadow clipmap has ${cm.levelCount} level(s) — nothing past the near box casts`);
}
if (cm.levels.length !== cm.levelCount) {
  fail(`tab A: clipmap says ${cm.levelCount} levels but published ${cm.levels.length}`);
}
if (cm.activeLevels !== cm.levelCount) {
  fail(
    `tab A: ${cm.activeLevels} of ${cm.levelCount} clipmap levels are contributing — ` +
      `a probe left the scene with levels switched off`,
  );
}
if (cm.levels[0].halfWidthM !== lit.radiusM) {
  fail(`tab A: level 0 is ${cm.levels[0].halfWidthM} m but lighting reports radius ${lit.radiusM} m`);
}
// One bias in texels across every level, which is what "scale normal bias by
// world texel width" means in a number: same texel count, different metres.
const biasTexels = cm.levels.map((L) => L.normalBias / L.texelM);
for (let i = 0; i < cm.levels.length; i++) {
  const L = cm.levels[i];
  if (i > 0 && !(L.halfWidthM > cm.levels[i - 1].halfWidthM)) {
    fail(`tab A: clipmap level ${i} (${L.halfWidthM} m) does not contain level ${i - 1}`);
  }
  if (i > 0 && !(L.texelM > cm.levels[i - 1].texelM)) {
    fail(`tab A: clipmap level ${i} texel ${L.texelM} m is not coarser than level ${i - 1}`);
  }
  // The ortho frustum has to contain the ground its own box covers. A level
  // bounds light-space X and Y; how much light-space DEPTH the ground inside
  // those bounds spans is set by how low the sun is — half·cot(elevation) each
  // way, which at a 21° sun is 2.66 m per metre of half-width. A flat depth
  // (lighting v0 carried 260 m below the centre for every level) is right for
  // one size and clips the far half of every larger one: level 2's 720 m box
  // needs 1913 m. Nothing used to live out there, so nothing showed it.
  const boxDepthM = 2 * L.halfWidthM * (Math.cos(lit.sunElevation) / Math.sin(lit.sunElevation));
  if (!(L.farM >= boxDepthM)) {
    fail(
      `tab A: clipmap level ${i}'s ortho depth is ${L.farM.toFixed(0)} m against the ` +
        `${boxDepthM.toFixed(0)} m its own ${L.halfWidthM} m box spans along the light at a ` +
        `${((lit.sunElevation * 180) / Math.PI).toFixed(0)}° sun — the far half of the level ` +
        `clips out and casts nothing`,
    );
  }
  if (!(L.sampledHalfWidthM < L.halfWidthM)) {
    fail(`tab A: clipmap level ${i} samples its full ${L.halfWidthM} m — no guard band, so a PCF tap can leave the map`);
  }
  if (L.casts !== true) fail(`tab A: clipmap level ${i} does not cast — it has no depth texture`);
  if (!(L.mapPx >= SHADOW_MIN_MAP_PX)) {
    fail(`tab A: clipmap level ${i} is ${L.mapPx} px — below ${SHADOW_MIN_MAP_PX}, silhouettes cannot resolve`);
  }
  if (!L.valid || !(L.renders > 0)) {
    fail(`tab A: clipmap level ${i} has never rendered (valid=${L.valid}, renders=${L.renders})`);
  }
  // Exactly one level lights the scene. Two would double the key.
  const wantsIntensity = i === 0;
  if (wantsIntensity !== L.intensity > 0) {
    fail(
      `tab A: clipmap level ${i} has intensity ${L.intensity} — only level 0 may carry the key, ` +
        `or N levels light the scene N times over`,
    );
  }
  if (Math.abs(biasTexels[i] - biasTexels[0]) > 1e-6) {
    fail(
      `tab A: clipmap level ${i}'s normal bias is ${biasTexels[i].toFixed(3)} texels against level 0's ` +
        `${biasTexels[0].toFixed(3)} — one metre value across ${cm.levels[0].texelM.toFixed(3)} m and ` +
        `${L.texelM.toFixed(3)} m texels is not a coherent bias`,
    );
  }
}
// The coarse levels are CACHED, not redrawn every frame — the whole reason
// the draw budget survives adding them. Counted over the session, not
// asserted from a flag: level 0 is dynamic and redraws every frame, so a
// coarse level that matched it would be uncached.
for (let i = 1; i < cm.levels.length; i++) {
  if (!(cm.levels[i].renders < cm.levels[0].renders)) {
    fail(
      `tab A: clipmap level ${i} has rendered ${cm.levels[i].renders} times against level 0's ` +
        `${cm.levels[0].renders} — the coarse level is not being cached at all`,
    );
  }
}
// Frame time is reported, never asserted: this box shares four cores with
// nineteen other services and the gate's renderer is a software rasterizer,
// so the number is a same-box regression signal and not a claim about the
// reference VPS. It is printed because the clipmap is the kind of change that
// pays for coverage with fill rate, and a doubled frame time should be
// visible to whoever reads this log rather than only to the flaky assertions
// it would eventually start tripping.
const frameA = await A.page.evaluate(() => globalThis.__gatesDebug.frameMs);
console.log(
  `  clipmap: ${cm.levelCount} levels ` +
    cm.levels
      .map(
        (L) =>
          `[${L.halfWidthM}m/${L.texelM.toFixed(3)}m px @${L.mapPx}, bias ${L.normalBias.toFixed(3)}m, ` +
          `${L.filterTaps} taps, ${L.renders} renders]`,
      )
      .join(" ") +
    `, budget ${cm.updateBudget}/frame, max age ${cm.maxCacheAge}` +
    ` · frame ~${frameA.toFixed(1)} ms (shared box, software GL — a trend, not a claim)`,
);

// Assertion 10 — the shadow map DARKENS PIXELS. A flag says the renderer was
// asked for shadows; only a frame says it got any. The dev-only probe renders
// the live scene twice per yaw — shadow pass on, then off — and counts pixels
// the shadow pass took down. Everything else about this rig (bias, radius,
// caster flags, the light's own position) can be individually wrong in a way
// that leaves every flag above true and the image unchanged.
const probeHook = await A.page.evaluate(() => typeof globalThis.__gatesDebug.shadowProbe);
if (probeHook !== "function") {
  fail(`tab A: __gatesDebug.shadowProbe is ${probeHook} on a dev shard — the shadow gate cannot run`);
}
const probe = await A.page.evaluate(
  ([yaws, pitch, minDelta]) => globalThis.__gatesDebug.shadowProbe(yaws, pitch, minDelta),
  [SHADOW_PROBE_YAWS, SHADOW_PROBE_PITCH, SHADOW_PROBE_MIN_DELTA],
);
const litFraction = probe.darkened / probe.pixels;
if (litFraction < SHADOW_MIN_FRACTION) {
  fail(
    `tab A: the shadow pass darkened ${(litFraction * 100).toFixed(3)}% of ` +
      `${probe.pixels} probed pixels across ${probe.samples.length} yaws — ` +
      `below ${(SHADOW_MIN_FRACTION * 100).toFixed(2)}%. Shadows are enabled but nothing is ` +
      `casting into the frame.\n` +
      probe.samples
        .map(
          (s) =>
            `    yaw ${s.yaw.toFixed(2)}: ${s.darkened} px, mean Δluma ${s.meanDelta.toFixed(1)}, ` +
            `max ${s.maxDelta}, frame luma ${s.litMean.toFixed(1)} unshadowed / ${s.shadowedMean.toFixed(1)} shadowed`,
        )
        .join("\n"),
  );
}
// Per-yaw floor. The aggregate above can be carried by one direction, and it
// was: see SHADOW_MIN_FRACTION_PER_YAW. Shadows in every direction can only
// come from the world casting, not from whatever happens to be next to you.
const thin = probe.samples.filter((s) => s.fraction < SHADOW_MIN_FRACTION_PER_YAW);
if (thin.length) {
  fail(
    `tab A: ${thin.length} of ${probe.samples.length} probed directions have almost no ` +
      `shadow in them (floor ${(SHADOW_MIN_FRACTION_PER_YAW * 100).toFixed(1)}% per yaw). ` +
      `Something near the camera is casting and the world is not.\n` +
      probe.samples
        .map((s) => `    yaw ${s.yaw.toFixed(2)}: ${(s.fraction * 100).toFixed(2)}% (${s.darkened} px)`)
        .join("\n"),
  );
}

// The shadow pass is extra DRAW CALLS, not just an extra texture lookup, and
// the budget below is asserted on a count that includes them. Prove that from
// the same two renders rather than asserting it in a comment — and require
// enough of them that the terrain and the scatter pools must be among them.
const shadowPassCalls = probe.samples.map((s) => s.callsShadowed - s.callsUnshadowed);
if (!shadowPassCalls.every((d) => d >= SHADOW_PASS_MIN_CALLS)) {
  fail(
    `tab A: the shadow pass submitted [${shadowPassCalls}] draw calls, below ` +
      `${SHADOW_PASS_MIN_CALLS} — either it is drawing nothing (so the budget assertion ` +
      `is not counting it either) or the world's meshes are not casters`,
  );
}
console.log(
  `  shadows: ${(litFraction * 100).toFixed(2)}% of ${probe.pixels} probed pixels darkened by the ` +
    `shadow pass (${probe.samples.map((s) => (s.fraction * 100).toFixed(1) + "%").join(" ")}), ` +
    `+${Math.min(...shadowPassCalls)}..${Math.max(...shadowPassCalls)} draw calls`,
);

// Assertion 11b — SHADOWS PAST THE NEAR BOX. The point of the slice, and the
// one thing no flag above can show: lighting v0 passed every assertion in
// this file with a hill at 100 m casting nothing.
//
// The probe renders the same frame with the whole clipmap and with only level
// 0 — which IS lighting v0's reach — and counts what the coarse levels took
// down. Two things make that a claim about DISTANCE. The camera's near plane
// is pushed past the near level so no close geometry is drawn at all; and the
// probe measures the resulting frustum's distance from the near level's
// committed centre along light-space X, so the assertion below is on a number
// read out of the actual camera matrices, not on the two constants that
// produced it.
const farHook = await A.page.evaluate(() => typeof globalThis.__gatesDebug.farShadowProbe);
if (farHook !== "function") {
  fail(`tab A: __gatesDebug.farShadowProbe is ${farHook} on a dev shard — the clipmap gate cannot run`);
}
const farYaws = [lit.sunAzimuth + Math.PI / 2, lit.sunAzimuth - Math.PI / 2];
const far = await A.page.evaluate(
  ([yaws, pitch, minDelta, nearM, fov, heightM]) =>
    globalThis.__gatesDebug.farShadowProbe(yaws, pitch, minDelta, nearM, fov, heightM),
  [
    farYaws,
    FAR_SHADOW_PROBE_PITCH,
    FAR_SHADOW_MIN_DELTA,
    FAR_SHADOW_NEAR_M,
    FAR_SHADOW_FOV_DEG,
    FAR_SHADOW_HEIGHT_M,
  ],
);
// First: is the frame actually outside the near level? If this does not hold
// the darkening below could be ordinary near-level shadow and the whole
// measurement is void, so it is checked before anything is read from it.
if (!(far.minLightXm > far.nearLevelHalfWidthM)) {
  fail(
    `tab A: the far-shadow probe's frustum comes within ${far.minLightXm.toFixed(1)} m of the near ` +
      `level's centre along light-space X, inside its ${far.nearLevelHalfWidthM} m half-width — ` +
      `the frame is not past the near box and nothing it measures is a claim about distance`,
  );
}
// Second: the probe's zero point, measured. Each yaw also renders the
// near-only state TWICE and counts pixels that moved between two identical
// frames. Anything but zero means the counts below are partly the software
// rasterizer talking, and every floor in this block is calibrated against
// noise instead of shadow.
const noisy = far.samples.filter((s) => s.noise > 0);
if (noisy.length) {
  fail(
    `tab A: the far-shadow probe's control renders disagree — ` +
      noisy.map((s) => `yaw ${s.yaw.toFixed(2)}: ${s.noise} px`).join(", ") +
      `. Two renders of the same state moved pixels by > ${FAR_SHADOW_MIN_DELTA}/255, so the ` +
      `darkening measured below cannot be attributed to the clipmap`,
  );
}
const farFraction = far.darkened / far.pixels;
if (farFraction < FAR_SHADOW_MIN_FRACTION) {
  fail(
    `tab A: past the near level's box (every pixel ≥ ${far.minLightXm.toFixed(0)} m out along ` +
      `light-space X) the coarse clipmap levels darkened ${(farFraction * 100).toFixed(3)}% of ` +
      `${far.pixels} pixels — below ${(FAR_SHADOW_MIN_FRACTION * 100).toFixed(2)}%. The levels ` +
      `exist and render, but nothing out there casts into the frame.\n` +
      far.samples
        .map(
          (s) =>
            `    yaw ${s.yaw.toFixed(2)}: ${s.darkened} px, mean Δluma ${s.meanDelta.toFixed(1)}, ` +
            `max ${s.maxDelta}, frustum reach ${s.reachM.toFixed(1)} m`,
        )
        .join("\n"),
  );
}
const farThin = far.samples.filter((s) => s.fraction < FAR_SHADOW_MIN_FRACTION_PER_YAW);
if (farThin.length) {
  fail(
    `tab A: ${farThin.length} of ${far.samples.length} far directions have almost no shadow ` +
      `(floor ${(FAR_SHADOW_MIN_FRACTION_PER_YAW * 100).toFixed(2)}% per yaw). Both are perpendicular ` +
      `to the sun, so one carrying the aggregate means the coverage is one-sided.\n` +
      far.samples
        .map((s) => `    yaw ${s.yaw.toFixed(2)}: ${(s.fraction * 100).toFixed(2)}% (${s.darkened} px)`)
        .join("\n"),
  );
}
const liftedWorst = Math.max(...far.samples.map((s) => s.liftedFraction));
if (liftedWorst > FAR_SHADOW_MAX_LIFTED_FRACTION) {
  fail(
    `tab A: turning the coarse clipmap levels on BRIGHTENED ${(liftedWorst * 100).toFixed(3)}% of a ` +
      `frame — they are contributing light, not shadow. A level past the first must have zero ` +
      `intensity or the key is counted once per level`,
  );
}
console.log(
  `  far shadows: ${(farFraction * 100).toFixed(2)}% of ${far.pixels} pixels darkened by levels ` +
    `past the near box (${far.samples.map((s) => (s.fraction * 100).toFixed(1) + "%").join(" ")}), ` +
    `frame ≥ ${far.minLightXm.toFixed(0)} m out vs a ${far.nearLevelHalfWidthM} m near level, ` +
    `lifted ${(liftedWorst * 100).toFixed(4)}%`,
);

// Assertion 11c — the GROUND casts, and the two LODs of it are kept apart.
// Structural, cheap, and here so 11d's failure is diagnosable: "the horizon
// darkens nothing" means something very different when the ground is still
// being culled out of every depth pass it is submitted to.
const fc = await A.page.evaluate(() => globalThis.__gatesDebug.farCaster);
if (!fc) fail(`tab A: the client publishes no far-caster state — the horizon is not wired`);
if (fc.shadowSide !== THREE_FRONT_SIDE) {
  fail(
    `tab A: the ground's material has shadowSide ${fc.shadowSide}, not FrontSide ` +
      `(${THREE_FRONT_SIDE}). three derives the depth pass from this: for a FrontSide ` +
      `material it flips to BackSide, and a heightfield has no back face turned at the ` +
      `sky — every terrain triangle is culled and hills cast nothing, near or far`,
  );
}
if (!fc.built) fail(`tab A: the far mesh has not been built — nothing can be asserted about the horizon`);
if (fc.casts !== true) fail(`tab A: the far mesh does not cast — the horizon is still lit flat`);
if (fc.customDepth !== true) {
  fail(
    `tab A: the far mesh casts with no custom depth material — nothing punches the near ` +
      `ring out of it, so both LODs of one hillside are in the same map`,
  );
}
if (!(fc.sinkM > 0)) {
  fail(`tab A: the far caster's seam skirt is ${fc.sinkM} m — the two LODs meet with no offset at all`);
}
if (!(fc.triangles > 0)) fail(`tab A: the far caster has ${fc.triangles} triangles`);
// The hole must actually track the ring. Two independent ways of being wrong:
// a hole frozen at the origin (the uniform never written), and a hole that has
// stopped containing the chunks that cast into it.
if (!(fc.nearChunks > 0)) fail(`tab A: no near chunks are loaded — the hole has nothing to track`);
const holeChunksX = (fc.holeHalf[0] * 2) / fc.chunkM;
const holeChunksZ = (fc.holeHalf[1] * 2) / fc.chunkM;
if (!(holeChunksX >= 1 && holeChunksZ >= 1)) {
  fail(
    `tab A: the far caster's hole is ${fc.holeHalf} m against ${fc.nearChunks} loaded near ` +
      `chunks — the near ring's footprint is never written, so the far mesh casts straight ` +
      `through it and every hillside in the ring has two silhouettes in one map`,
  );
}
if (holeChunksX * holeChunksZ < fc.nearChunks) {
  fail(
    `tab A: the hole is ${holeChunksX}x${holeChunksZ} chunks but ${fc.nearChunks} are loaded ` +
      `and casting — a chunk outside the hole is a double caster against the far mesh`,
  );
}
const ownA = await A.page.evaluate(() => globalThis.__gatesDebug.own);
if (
  Math.abs(ownA[0] - fc.holeCenter[0]) > fc.holeHalf[0] ||
  Math.abs(ownA[2] - fc.holeCenter[1]) > fc.holeHalf[1]
) {
  fail(
    `tab A: the player is at [${ownA[0].toFixed(0)}, ${ownA[2].toFixed(0)}] but the hole is ` +
      `centred on [${fc.holeCenter.map((v) => v.toFixed(0))}] ±[${fc.holeHalf}] — the hole is ` +
      `not following the ring`,
  );
}
// And the per-level split that pays for the third level: the two LODs are not
// both submitted to every map.
if (!(fc.farMinLevel >= 1)) {
  fail(`tab A: the far mesh casts from level ${fc.farMinLevel} — level 0's box is inside the hole, so that is a whole map's fill discarded`);
}
if (!(fc.nearMaxLevel < cm.levelCount - 1)) {
  fail(
    `tab A: the near ring casts up to level ${fc.nearMaxLevel} of ${cm.levelCount} — the ` +
      `coarsest level is read only by pixels the ring cannot reach, so that is ${fc.nearChunks} ` +
      `draw calls of caster nothing samples`,
  );
}
console.log(
  `  far caster: ${fc.triangles} tris, shadowSide ${fc.shadowSide} (FrontSide), skirt ${fc.sinkM} m, ` +
    `hole ${holeChunksX}x${holeChunksZ} chunks over ${fc.nearChunks} loaded, ` +
    `near ring in levels 0..${fc.nearMaxLevel}, far mesh in ${fc.farMinLevel}..`,
);

// Assertion 11d — the HORIZON casts, measured by removing it.
//
// 11b's probe proves the coarse levels reach past the near LEVEL's box. It
// cannot prove the horizon casts, and never could: until this slice the near
// ring WAS the caster set, and a frame 97 m out is deep inside it.
//
// This one holds the frame fixed and removes the caster. The far mesh casts
// through a depth material that discards the near ring's footprint; opening
// that hole to swallow the world discards all of it, which is exactly the
// state that shipped before this slice. The difference between the two frames
// is the horizon — and it is a claim about DISTANCE for free, because the hole
// means every pixel it darkens was cast by geometry more than the ring's own
// footprint away. No camera geometry has to be argued.
const horizonHook = await A.page.evaluate(() => typeof globalThis.__gatesDebug.horizonProbe);
if (horizonHook !== "function") {
  fail(`tab A: __gatesDebug.horizonProbe is ${horizonHook} on a dev shard — the horizon gate cannot run`);
}
const horizon = await A.page.evaluate(
  ([yaws, pitch, minDelta, heightM]) =>
    globalThis.__gatesDebug.horizonProbe(yaws, pitch, minDelta, heightM),
  [HORIZON_PROBE_YAWS, HORIZON_PROBE_PITCH, FAR_SHADOW_MIN_DELTA, HORIZON_PROBE_HEIGHT_M],
);
if (!horizon) fail(`tab A: the horizon probe found no far caster to toggle`);
// The probe must have scored the LIVE ring footprint, not a leftover: a hole
// already swallowing the world would make both its frames identical and the
// measurement below vacuously zero in a way the floor would blame on casting.
if (!(horizon.holeHalf[0] > 0) || horizon.holeHalf[0] > ISLAND_M) {
  fail(`tab A: the horizon probe ran with a hole half-extent of ${horizon.holeHalf} m — not the live ring footprint`);
}
const horizonNoisy = horizon.samples.filter((s) => s.noise > 0);
if (horizonNoisy.length) {
  fail(
    `tab A: the horizon probe's control renders disagree — ` +
      horizonNoisy.map((s) => `yaw ${s.yaw.toFixed(2)}: ${s.noise} px`).join(", ") +
      `. Two renders of the same state moved pixels by > ${FAR_SHADOW_MIN_DELTA}/255, so what ` +
      `the toggle below measures cannot be attributed to the far caster`,
  );
}
const horizonFraction = horizon.darkened / horizon.pixels;
if (horizonFraction < HORIZON_MIN_FRACTION) {
  fail(
    `tab A: suppressing the far mesh's shadow changed ${(horizonFraction * 100).toFixed(3)}% of ` +
      `${horizon.pixels} swept pixels — below ${(HORIZON_MIN_FRACTION * 100).toFixed(2)}%. The ` +
      `horizon receives and does not cast, which is exactly the state this slice exists to end.\n` +
      horizon.samples
        .map(
          (s) =>
            `    yaw ${s.yaw.toFixed(2)}: ${s.darkened} px, mean Δluma ${s.meanDelta.toFixed(1)}, max ${s.maxDelta}`,
        )
        .join("\n"),
  );
}
const horizonLive = horizon.samples.filter((s) => s.fraction >= HORIZON_MIN_FRACTION_PER_YAW);
if (horizonLive.length < HORIZON_MIN_DIRECTIONS) {
  fail(
    `tab A: only ${horizonLive.length} of ${horizon.samples.length} directions lose anything when ` +
      `the far caster is suppressed (floor ${(HORIZON_MIN_FRACTION_PER_YAW * 100).toFixed(2)}% per ` +
      `yaw, ${HORIZON_MIN_DIRECTIONS} required) — one direction carrying the whole aggregate is a ` +
      `fluke of where the camera happened to be pointed, not a horizon that casts\n` +
      horizon.samples
        .map((s) => `    yaw ${s.yaw.toFixed(2)}: ${(s.fraction * 100).toFixed(2)}% (${s.darkened} px)`)
        .join("\n"),
  );
}
const horizonLifted = Math.max(...horizon.samples.map((s) => s.liftedFraction));
if (horizonLifted > FAR_SHADOW_MAX_LIFTED_FRACTION) {
  fail(
    `tab A: removing the far caster DARKENED ${(horizonLifted * 100).toFixed(3)}% of a frame — a ` +
      `caster cannot add light, so the hole is inverted and the far mesh is casting inside the ` +
      `near ring instead of outside it`,
  );
}
console.log(
  `  horizon: suppressing the far caster moved ${(horizonFraction * 100).toFixed(2)}% of ` +
    `${horizon.pixels} swept pixels ` +
    `(${horizon.samples.map((s) => (s.fraction * 100).toFixed(1) + "%").join(" ")}), ` +
    `${horizonLive.length}/${horizon.samples.length} directions, mean Δluma ` +
    `${(horizonLive.reduce((a, s) => a + s.meanDelta, 0) / Math.max(1, horizonLive.length)).toFixed(1)}, ` +
    `hole ±${horizon.holeHalf[0]} m, lifted ${(horizonLifted * 100).toFixed(4)}%`,
);

// --- materials: the surface read --------------------------------------------
// Measured here, on the same pinned frame the shadow gate uses, for the same
// reason: after the walk the ground under the camera depends on how much wall
// clock a shared box gave the input pump.
//
// Assertion 13 — the material system is wired, and its channels are real
// channels. Cheap and structural, and it exists so 14's and 15's failures are
// diagnosable: "the field paints nothing" means something very different when
// the ground is still a Lambert material.
const mat = await A.page.evaluate(() => globalThis.__gatesDebug.materials);
if (!mat) fail(`tab A: __gatesDebug.materials missing — the scene publishes no material state`);
if (mat.terrain.type !== "MeshStandardMaterial") {
  fail(`tab A: the ground is a ${mat.terrain.type} — roughness is not a channel it has`);
}
if (mat.terrain.patched !== true) {
  fail(`tab A: the terrain material carries no splat uniforms — onBeforeCompile did not install`);
}
if (mat.identities.length !== 4) {
  fail(`tab A: ${mat.identities.length} terrain identities (${mat.identities}) — TERRAIN.md §4 specifies four sets`);
}
// Roughness and bump per identity, not one number applied to everything: the
// procedural-materials failure list's "roughness is a scalar afterthought".
const distinct = (xs) => new Set(xs.map((v) => v.toFixed(4))).size;
if (distinct(mat.identityRoughness) < 3) {
  fail(`tab A: only ${distinct(mat.identityRoughness)} distinct identity roughness values in [${mat.identityRoughness}] — sand, grass, litter and rock answer light the same way`);
}
if (distinct(mat.identityBump) < 3) {
  fail(`tab A: only ${distinct(mat.identityBump)} distinct identity bump amplitudes in [${mat.identityBump}]`);
}
if (!(mat.breakup > 0)) fail(`tab A: splat break-up amplitude is ${mat.breakup} — biome boundaries stay smooth ramps`);
if (!(mat.specAA > 0)) fail(`tab A: specular-AA gain is ${mat.specAA} — the perturbed normal is unfiltered`);
if (!(mat.fadeMicroM[1] > mat.fadeMicroM[0] && mat.fadeMicroM[0] > 0)) {
  fail(`tab A: micro-octave footprint fade is [${mat.fadeMicroM}] — detail below a pixel is not being faded out`);
}
// The tier read (bases.webp): wood, stone and metal must not answer the key
// light identically, and metal must actually be a conductor.
const tierRough = mat.tiers.map((t) => t[1]);
if (mat.tiers.some((t) => t[0] !== "MeshStandardMaterial")) {
  fail(`tab A: build tiers are [${mat.tiers.map((t) => t[0])}] — a tier with no roughness cannot read as its material`);
}
if (distinct(tierRough) !== 3) fail(`tab A: build tiers share roughness [${tierRough}] — wood, stone and metal read alike`);
if (!(mat.tiers[2][2] > 0.5)) fail(`tab A: the metal tier has metalness ${mat.tiers[2][2]} — it is a grey dielectric`);
if (mat.tiers[0][2] !== 0 || mat.tiers[1][2] !== 0) {
  fail(`tab A: wood/stone tiers have metalness [${mat.tiers[0][2]}, ${mat.tiers[1][2]}] — only metal is a conductor`);
}
if (!(mat.water[0] < 0.3)) fail(`tab A: water roughness is ${mat.water[0]} — the sun leaves no track on it`);
// Scatter: every pool authored, colour-baked and per-instance tinted. One
// solid green cone pool (what this slice replaced) shows up as tint 0 or a
// missing instance colour buffer.
if (!Array.isArray(mat.scatter) || mat.scatter.length < 7) {
  fail(`tab A: ${mat.scatter?.length} scatter pools published — sim-core scatters seven archetypes`);
}
for (const p of mat.scatter) {
  if (p.type !== "MeshStandardMaterial") fail(`tab A: scatter archetype ${p.arch} is a ${p.type}`);
  if (!p.vertexColors) fail(`tab A: scatter archetype ${p.arch} has no baked vertex colours — it is one flat colour`);
  if (!p.instanceColor) fail(`tab A: scatter archetype ${p.arch} has no per-instance colour buffer — every instance is identical`);
  if (!(p.tint > 0)) fail(`tab A: scatter archetype ${p.arch} has tint amplitude ${p.tint} — a forest of one green`);
}
if (!mat.scatter.some((p) => p.metalness > 0)) {
  fail(`tab A: no scatter archetype is metallic — the ore nodes read as painted rock`);
}
if (!mat.scatter.some((p) => p.count > 0)) {
  fail(`tab A: every scatter pool is empty — nothing was streamed, so nothing above was measured on real instances`);
}

// Assertion 14 — the splat weights are a field, not a constant. The shader
// below can blend four identities perfectly and still paint one, if what it
// is fed is uniform or one-hot; no pixel probe can tell those apart from a
// well-blended world, so count the attribute itself.
const censusHook = await A.page.evaluate(() => typeof globalThis.__gatesDebug.splatCensus);
if (censusHook !== "function") {
  fail(`tab A: __gatesDebug.splatCensus is ${censusHook} on a dev shard — the splat gate cannot run`);
}
const census = await A.page.evaluate(() => globalThis.__gatesDebug.splatCensus());
if (!(census.vertices > 0)) fail(`tab A: splat census saw ${census.vertices} vertices — the near ring never streamed`);
console.log(
  `  splat: ${census.vertices} vertices over ${census.chunks} chunks (${mat.identities.join("/")}) · ` +
    `dominant ${census.dominantFraction.map((f) => (f * 100).toFixed(1) + "%").join("/")} · ` +
    `present ${census.presentFraction.map((f) => (f * 100).toFixed(1) + "%").join("/")} · ` +
    `spread ${census.spread.map((f) => f.toFixed(2)).join("/")} · ` +
    `${(census.mixedFraction * 100).toFixed(1)}% blended, deepest second identity ` +
    `${census.maxSecond.toFixed(2)}`,
);
// Spread, not biome membership: which biomes the pinned spawn's 320 m ring
// happens to contain is a worldgen fact (the moisture channel's feature size
// is ~700 m, so one ring is often one biome). What must be true of the
// ATTRIBUTE anywhere is that it varies — a constant or a stuck weight vector
// reads zero spread on every channel no matter where the ring is.
const varying = census.spread.filter((s) => s >= SPLAT_MIN_SPREAD).length;
if (varying < SPLAT_MIN_IDENTITIES) {
  fail(
    `tab A: only ${varying} of 4 identity weights vary by ≥ ${SPLAT_MIN_SPREAD} over the near ring ` +
      `(spreads ${census.spread.map((s) => s.toFixed(3)).join(" ")}) — the splat attribute is not a ` +
      `field derived from the world, so the shader is blending a constant`,
  );
}
const held = census.presentFraction.filter((f) => f >= SPLAT_IDENTITY_SHARE).length;
if (held < SPLAT_MIN_IDENTITIES) {
  fail(
    `tab A: ${held} of 4 identities contribute to ≥ ${(SPLAT_IDENTITY_SHARE * 100).toFixed(0)}% of the ` +
      `near ring (${census.presentFraction.map((f) => (f * 100).toFixed(1) + "%").join(" ")}) — ` +
      `the ground is one material wearing a four-slot attribute`,
  );
}
// How gradually they meet. Also location-independent: it needs one boundary
// vertex anywhere in the ring, and the check above guarantees one exists.
// A hard threshold hands each vertex wholly to one identity, so the deepest
// second weight it can ever reach is the cliff mask's — not a biome ramp's.
if (census.maxSecond < SPLAT_MIN_SECOND) {
  fail(
    `tab A: the deepest any second identity gets on the near ring is ${census.maxSecond.toFixed(3)} ` +
      `(floor ${SPLAT_MIN_SECOND}) — two identities never genuinely share a vertex, so the biome ` +
      `bands are hard thresholds and the boundary is a seam on the vertex grid`,
  );
}
if (census.mixedFraction < SPLAT_MIN_MIXED) {
  fail(
    `tab A: only ${(census.mixedFraction * 100).toFixed(2)}% of ${census.vertices} near-ring vertices ` +
      `are a real blend of two identities (floor ${(SPLAT_MIN_MIXED * 100).toFixed(2)}%) — the ` +
      `transition is one row of vertices wide, which is a seam with extra steps`,
  );
}

// Assertion 15 — the procedural field REACHES THE FRAME. Everything above can
// be true while the ground is a flat wash: a field scaled into one lattice
// cell, uniforms never bound, a bump term cancelled by its own footprint
// fade. The probe renders the live scene twice per yaw with the field on and
// off and counts the pixels that moved, by direction. Held fixed across the
// pair: the vertex splat weights, the four authored identities, and the
// causal modifiers. Removed: every contribution of the noise field — the
// weight break-up, the mottling, the roughness variation, the bump.
const surfaceHook = await A.page.evaluate(() => typeof globalThis.__gatesDebug.surfaceProbe);
if (surfaceHook !== "function") {
  fail(`tab A: __gatesDebug.surfaceProbe is ${surfaceHook} on a dev shard — the surface gate cannot run`);
}
const surf = await A.page.evaluate(
  ([yaws, pitch, minDelta]) => globalThis.__gatesDebug.surfaceProbe(yaws, pitch, minDelta),
  [SURFACE_PROBE_YAWS, SURFACE_PROBE_PITCH, SURFACE_PROBE_MIN_DELTA],
);
if (!surf) fail(`tab A: surfaceProbe returned null — the scene never took the terrain material's uniforms`);
const surfDetail = () =>
  surf.samples
    .map(
      (s) =>
        `    yaw ${s.yaw.toFixed(2)}: ${(s.fraction * 100).toFixed(2)}% moved ` +
        `(+${(s.upFraction * 100).toFixed(2)}% / −${(s.downFraction * 100).toFixed(2)}%), ` +
        `mean Δluma ${s.meanDelta.toFixed(1)}, max ${s.maxDelta}`,
    )
    .join("\n");
const surfFraction = surf.changed / surf.pixels;
if (surfFraction < SURFACE_MIN_FRACTION) {
  fail(
    `tab A: the procedural surface moved ${(surfFraction * 100).toFixed(3)}% of ${surf.pixels} probed ` +
      `pixels across ${surf.samples.length} yaws — below ${(SURFACE_MIN_FRACTION * 100).toFixed(2)}%. ` +
      `The material is configured and paints nothing.\n${surfDetail()}`,
  );
}
const flatYaws = surf.samples.filter((s) => s.fraction < SURFACE_MIN_FRACTION_PER_YAW);
if (flatYaws.length) {
  fail(
    `tab A: ${flatYaws.length} of ${surf.samples.length} probed directions are flat ground ` +
      `(floor ${(SURFACE_MIN_FRACTION_PER_YAW * 100).toFixed(1)}% per yaw).\n${surfDetail()}`,
  );
}
// Two-sided: a uniform change can only move the frame one way.
const oneSided = surf.samples.filter(
  (s) => s.upFraction < SURFACE_MIN_DIRECTIONAL || s.downFraction < SURFACE_MIN_DIRECTIONAL,
);
if (oneSided.length) {
  fail(
    `tab A: ${oneSided.length} of ${surf.samples.length} directions moved the frame only one way ` +
      `(floor ${(SURFACE_MIN_DIRECTIONAL * 100).toFixed(1)}% up AND down per yaw). Microstructure ` +
      `lightens some pixels and darkens others; a global tint or exposure shift cannot.\n${surfDetail()}`,
  );
}
console.log(
  `  surface: ${(surfFraction * 100).toFixed(2)}% of ${surf.pixels} probed pixels moved by the ` +
    `procedural field, per yaw ` +
    surf.samples
      .map(
        (s) =>
          `${(s.fraction * 100).toFixed(1)}%(+${(s.upFraction * 100).toFixed(1)}/−${(s.downFraction * 100).toFixed(1)})`,
      )
      .join(" "),
);

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

// Assertion 11 — DESIGN §9's draw budget, which had no gate until the shadow
// pass gave it teeth: a second full pass over every caster is exactly the kind
// of change that eats it. Counted and structural, so it means the same here as
// on the reference box; timing still does not. Read HERE, at the end, rather
// than off the same snapshot as assertion 9: by now both tabs have walked, so
// the near ring has streamed and torn down and the scene is at its fullest.
//
// Asserted on the PEAK across every frame since boot, not on whichever frame
// this line happened to catch. The clipmap made that distinction real: a
// cached coarse level draws on some frames and not others, so a last-frame
// count can miss the expensive one entirely. The budget is what the GPU was
// ever asked to draw.
const budget = await A.page.evaluate(() => globalThis.__gatesDebug.lighting);
if (!(budget.calls > 0)) fail(`tab A: renderer reported ${budget.calls} draw calls — the stats are not being read`);
if (!(budget.peakCalls >= budget.calls)) {
  fail(`tab A: peak ${budget.peakCalls} draw calls is below this frame's ${budget.calls} — the peak is not being tracked`);
}
if (budget.peakCalls >= DRAW_CALL_BUDGET) {
  fail(
    `tab A: peak ${budget.peakCalls} draw calls (main + every shadow level, worst frame; ` +
      `${budget.calls} this frame) — DESIGN §9 budgets < ${DRAW_CALL_BUDGET}`,
  );
}
if (budget.peakTriangles >= TRIANGLE_BUDGET) {
  fail(
    `tab A: peak ${budget.peakTriangles} triangles (main + every shadow level, worst frame; ` +
      `${budget.triangles} this frame) — DESIGN §9 budgets < ${TRIANGLE_BUDGET}`,
  );
}
console.log(
  `  budget: ${budget.calls} draw calls / ${budget.triangles} triangles this frame, ` +
    `peak ${budget.peakCalls} / ${budget.peakTriangles} (all passes)`,
);

// Assertion 12 — the gate itself. A shard with no dev override is exactly a
// public shard's config, and its client must have no dev surface at all.
const P = await join("public tab", PUBLIC_WIRE_PORT, publicCertHash);
if (P.dbg.dev !== false) fail(`public tab: shard without dev_spawn welcomed with dev=${P.dbg.dev}`);
const publicSetView = await P.page.evaluate(() => typeof globalThis.__gatesDebug.setView);
if (publicSetView !== "undefined") {
  fail(`public tab: __gatesDebug.setView is ${publicSetView} on a shard with no dev override — a dev affordance shipped to a public shard`);
}
for (const hook of ["shadowProbe", "farShadowProbe", "surfaceProbe", "splatCensus", "horizonProbe"]) {
  const t = await P.page.evaluate((h) => typeof globalThis.__gatesDebug[h], hook);
  if (t !== "undefined") {
    fail(`public tab: __gatesDebug.${hook} is ${t} on a shard with no dev override — a dev affordance shipped to a public shard`);
  }
}
if (P.errors.length) {
  fail(`public tab: ${P.errors.length} page error(s):\n` + P.errors.slice(0, 8).map((e) => `    ${e}`).join("\n"));
}
console.log(`  dev gate: public shard welcomes dev=false, client installs no setView`);

console.log("browser smoke: all checks passed");
cleanup();
process.exit(0);
