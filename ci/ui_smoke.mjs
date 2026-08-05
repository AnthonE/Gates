#!/usr/bin/env node
// Gate: the interaction surface, in a real browser, with no renderer.
//
// Why this exists. `ci/gates.sh` splits into a code tier and a renderer tier,
// and the renderer tier is the overwhelming majority of the wall clock (a
// single `browser_smoke` run is 8-10 min). Its `renderer_touched` question
// matches `^web/`, so a one-line HUD change — a `<div>` moved, a toast string
// reworded — drags in `browser_smoke` AND `vantages` and pays ~19 minutes to
// prove that a DOM overlay still overlaid. That is the wrong owner paying the
// cost.
//
// This gate is the coverage that made the fix safe, and the fix is now ARMED
// (operator, 2026-08-04, `DECISIONS.md` §open): `renderer_touched` exempts
// `web/index.html`, `web/src/hud.js` and `web/src/input.js`, and every other
// path under `web/` still schedules the renderer tier. It earned that by
// asserting, as a strict SUPERSET, every `web/index.html` and `web/src/hud.js`
// contract that `browser_smoke` holds — eleven mutants of those two files, all
// eleven red.
//
// THE STANDING RULE that came with the arming, and it binds this file: a path
// joins that exemption list ONLY in a commit that also extends this gate to
// cover what that path can break. Subtracting a path from `renderer_touched`
// subtracts a gate from the merge, so the list is the operator's and never a
// lane branch's.
//
// The superset is the load-bearing claim, so it is written down rather than
// asserted in prose. `browser_smoke`'s HUD-owning reads, and where each is
// answered here:
//
//   browser_smoke                        | here
//   -------------------------------------|---------------------------------
//   :1445-1447 #url/#cert fill, #connect | A (present, editable, clickable)
//   :1530 #start inline display "none"   | A (present, inline field clear)
//   :1534,:1612 #starterr text empty     | A (present, empty at load)
//   :1745 #vitals INLINE display "block" | F (boxShown, asserted)
//   :1748 .vrow INLINE display "none"    | F (inline, not computed)
//   :1749-1751 .vfill/.vnum per row      | F
//   :1752-1756 .vfill class keys the row | F (exclusive, health has neither)
//   :1757 .vnum is a bare integer        | F (exact string, no unit)
//   :1871,:1903-1911 #chatlog text/#id   | D (verbatim text, '#7', '[g] ')
//   :1931-1940 #death + its three parts  | A (closed at load) and G
//   :1949-1956 #death computed "none"    | A (on a live body, before showDeath)
//   :5085-5089 CHAT_CAP eviction         | D
//
// Anything `browser_smoke` adds to that list has to be added here too, in the
// same commit, or the superset is a claim and not a fact.
//
// Why a real browser and not a node DOM stub. `Hud` touches `getElementById`,
// `createElement`, `createTextNode`, `appendChild`, `removeChild`,
// `firstChild`, `childElementCount`, `parentNode`, `classList`, `style`,
// `textContent`, `value`, `focus`, `blur`, `disabled`, `title` and
// `addEventListener` — a stub covering that is most of a DOM, and a stub that
// is subtly wrong is worse than no gate, because it passes. `ci/dom_shim.mjs`
// is four lines and covers none of it. More to the point, half the laws below
// are about EVENTS — a keystroke that must not reach the movement keys, a
// click that must not fire twice — and event propagation is the thing a stub
// gets wrong first. Chromium already implements all of it correctly.
//
// Why it is cheap, and how to keep it cheap. The renderer gates' cost is FRAME
// COUNT: thousands of frames at roughly one a second under SwiftShader, plus
// two shards, three contexts, and full drawing-buffer readbacks. This gate
// renders no frames. It creates no WebGL context, loads no wasm, starts no
// shard, and asks for no pixel statistic. `/src/main.js` — the entry that
// would pull in three.js, the wasm bridge and the transport — is stubbed at
// the route, so the page load is `index.html` and nothing else. If this file
// ever grows a shard, a canvas context or a screenshot, it has become
// `browser_smoke` and should be cut back instead.
//
// What it deliberately does NOT do. It is not the boot gate. `browser_smoke`
// owns "the client actually joins a shard and draws the world", and two hard
// boot bugs on 2026-07-31 are why it exists. Nothing here can replace it and
// nothing here is allowed to look like it does — this gate proves the
// interaction surface behaves, on a page where the game never started.
//
// The scaffold assertions (group A) are the load-bearing half of the proposal.
// `web/index.html` hosts the renderer's canvas and the client's entry script
// alongside the whole HUD; carving it out of the renderer tier would only be
// honest if something still asserted that the canvas is present, mounted and
// visible, and that the entry script is still wired. That is group A's job.
//
// Serves `web/` (source), not `web/dist`: `hud.js` and `input.js` import
// nothing at all, so the browser loads them raw with no bundling step, and the
// gate reads the file the lane actually edits.

import http from "node:http";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { createRequire } from "node:module";

const root = path.join(path.dirname(fileURLToPath(import.meta.url)), "..");
const WEB = path.join(root, "web");
// Port 0: the OS picks a free one and we read back what it gave us.
//
// This was 8952 — "away from `browser_smoke` (8934) and `vantages` (8971) so a
// UI gate can run beside either without a port fight" — and that reasoning was
// right about the wrong neighbours. The build runs THREE lanes in parallel
// (`looks`, `systems`, `ui`), each a worktree running its own `./ci/gates.sh`,
// and this gate is in the CODE tier, which every lane runs on every pass. So
// the collision was never with the renderer gates; it was with the other two
// copies of THIS one. A distinct fixed port cannot fix that, because the thing
// it collides with is itself.
//
// It showed up as a flaky wall on 2026-08-04: `./ci/gates.sh` red, then green
// on an immediate re-run of an unchanged clean tree (`logs/health-red1.log`
// against `logs/health.log`). A flaky wall is not a wall.
//
// Deliberately not a retry loop and not a scan for a free port: both re-open
// the same race with a longer fuse, and this repo has already paid for
// "widening a timeout is not a fix". Port 0 has no race — the kernel does not
// hand the same ephemeral port to two sockets. `UI_SMOKE_PORT` still overrides
// for a caller who needs a known address.
const PORT = Number(process.env.UI_SMOKE_PORT || 0);

let checks = 0;
let server = null;
let browser = null;
const cleanup = () => {
  try {
    server?.close();
  } catch {
    /* already down */
  }
};
process.on("exit", cleanup);

const fail = (msg) => {
  console.error(`GATE FAIL: ${msg}`);
  process.exit(1);
};
const check = (cond, msg) => {
  checks++;
  if (!cond) fail(msg);
};

// --- what the HUD reaches for, read from the HUD ----------------------------
// Restating the id list here would be a list that drifts — `pine_shape.mjs`
// makes the same argument about numbers ("a number restated here is a number
// that can drift") and reads its constants out of the source they live in.
// Same discipline: the ids and the caps come out of `hud.js` itself, so adding
// an element to the HUD extends this gate's coverage automatically instead of
// silently escaping it.
const HUD_SRC = path.join(WEB, "src/hud.js");
if (!fs.existsSync(HUD_SRC)) fail(`web/src/hud.js missing at ${HUD_SRC}`);
const hudSrc = fs.readFileSync(HUD_SRC, "utf8");
const HUD_IDS = [...hudSrc.matchAll(/getElementById\("([^"]+)"\)/g)].map((m) => m[1]);
// A parse that found nothing would make every id check below vacuous — the
// "gate that matches nothing" failure, which is the worst bug class in this
// repo. Assert the parse before trusting what it produced.
//
// This is a RATCHET, pinned at the count that actually ships, not a loose
// floor. It was written `>= 14` against an actual 16, which is the same hole
// one size down: two surfaces could leave the HUD entirely and the number
// would still clear the bar. Adding an element raises it (the check passes
// and this constant is updated in the same commit, like a golden); removing
// one has to be a stated act rather than a silent drift.
// 2026-08-04: 16 → 20, the inventory screen's #inv/#invgrid/#invbelt/#invdetail.
// 2026-08-05: 20 → 23, the open container's #cont/#conttitle/#contgrid.
// 2026-08-05: 23 → 24, the interaction prompt's #prompt.
const HUD_ID_COUNT = 24;
check(
  HUD_IDS.length >= HUD_ID_COUNT,
  `parsed ${HUD_IDS.length} getElementById ids out of hud.js, expected at least ${HUD_ID_COUNT}` +
    " — either the parse broke (every scaffold check below is then vacuous) or a HUD surface left;" +
    " if a surface was removed on purpose, move this constant in the same commit",
);
const hudConst = (name) => {
  const m = hudSrc.match(new RegExp(`const ${name} = (\\d+);`));
  if (!m) fail(`hud.js declares no ${name} — this gate reads its caps from the source`);
  return Number(m[1]);
};
const TOAST_CAP = hudConst("TOAST_CAP");
const CHAT_CAP = hudConst("CHAT_CAP");
// Same discipline for the inventory's shape. The LAYOUT split is hud.js's
// (`ALPHA.md` §1: 6 belt + 24 grid); the TOTAL is the sim's, and it is
// imported from `invmove.js` rather than read out of hud.js because as of
// 2026-08-05 there is exactly one JS declaration of it — pinned to
// `limits.rs` in group N below.
//
// That single declaration is the judge's ranked fix 1, taken structurally
// rather than literally. The report's finding was that group N restated the
// cap as a bare `30`, and that mutant M13 (widen the probe array to 36) left
// the gate GREEN while it silently stopped testing the `undefined` path its
// own failure message described. Editing those two literals to say
// `INV_SLOTS` would have closed M13 and left the CLASS open: the number was
// declared in hud.js, again in invmove.js, and a third time in the gate.
// One declaration cannot drift from itself.
const { INV_SLOTS, BOX_SLOTS } = await import(
  pathToFileURL(path.join(root, "web/src/invmove.js")).href
);
const INV_BELT = hudConst("INV_BELT");
const INV_GRID = hudConst("INV_GRID");
check(
  Number.isInteger(INV_SLOTS) && Number.isInteger(BOX_SLOTS),
  `invmove.js exports INV_SLOTS ${INV_SLOTS} / BOX_SLOTS ${BOX_SLOTS}, at least one not a number — every slot` +
    " bound below would then be checked against NaN, which is the gate-that-matches-nothing class",
);
check(
  INV_BELT + INV_GRID === INV_SLOTS,
  `hud.js declares INV_BELT ${INV_BELT} + INV_GRID ${INV_GRID} = ${INV_BELT + INV_GRID}, but INV_SLOTS ${INV_SLOTS}` +
    " — the belt and the grid together ARE the inventory; a gap or an overlap means slots nothing can draw",
);
check(
  INV_BELT === 6 && INV_SLOTS === 30,
  `hud.js declares a ${INV_BELT}-slot belt of invmove.js's ${INV_SLOTS} slots; ALPHA.md §1 fixes 6 and 24, and wasm.js:76` +
    " reads exactly 30 × 2 u16 words — a panel drawing a different count draws slots the wire does not carry",
);

// --- dependencies, each a loud failure --------------------------------------
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
// `src/map.js` joins the list because `hud.js` imports it: the page loads it
// raw, so a missing file is a module-resolution error inside a page evaluate
// rather than the plain sentence below.
for (const rel of ["index.html", "src/hud.js", "src/input.js", "src/map.js"]) {
  if (!fs.existsSync(path.join(WEB, rel))) fail(`web/${rel} missing — nothing to smoke`);
}

// --- serve web/ (source) ----------------------------------------------------
const MIME = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css" };
server = http.createServer((req, res) => {
  const rel = decodeURIComponent(req.url.split("?")[0]);
  // The page ships no favicon and the browser asks anyway. Answered rather
  // than filtered downstream: a page error fails this gate, and the way to
  // keep that strict is to stop producing the error (`vantages.mjs`).
  if (rel === "/favicon.ico") return res.writeHead(204).end();
  const file = path.join(WEB, rel === "/" ? "index.html" : rel);
  if (!file.startsWith(WEB) || !fs.existsSync(file) || fs.statSync(file).isDirectory()) {
    return res.writeHead(404).end("not found");
  }
  res.writeHead(200, {
    "content-type": MIME[path.extname(file)] || "application/octet-stream",
    // The production headers, so the page is cross-origin-isolated here too.
    "cross-origin-opener-policy": "same-origin",
    "cross-origin-embedder-policy": "require-corp",
  });
  fs.createReadStream(file).pipe(res);
});
// Listen with the `error` event actually handled. Without this line a bind
// failure is an UNHANDLED 'error' event: node prints a raw `node:events:497
// throw er` stack and dies, and `gates.sh` reports "GATE FAIL: ui smoke" with
// no cause attached to it — which is how the 2026-08-04 flake presented. A
// gate that cannot say why it failed costs a pass to diagnose.
await new Promise((resolve, reject) => {
  server.once("error", reject);
  server.listen(PORT, "127.0.0.1", () => {
    server.removeListener("error", reject);
    resolve();
  });
}).catch((e) =>
  fail(
    `the static server could not bind ${PORT === 0 ? "an ephemeral port" : `127.0.0.1:${PORT}`}: ${e.message}` +
      (PORT === 0 ? "" : " — UI_SMOKE_PORT pins the port; unset it to let the OS pick a free one"),
  ),
);
// Read back what the kernel actually assigned. With PORT=0 the value in
// `PORT` is 0 and navigating to `http://127.0.0.1:0/` would not resolve, so
// every URL below must come from here and never from the constant.
const port = server.address()?.port;
if (!port) fail("the static server bound no port — server.address() returned nothing");

// --- the browser ------------------------------------------------------------
try {
  browser = await chromium.launch({
    headless: true,
    // The same escape hatch `browser_smoke` and `vantages` carry under the
    // same name: a box whose installed Playwright build does not match the
    // revision `web/package.json` pins launches nothing, and a wall that
    // cannot run is not a wall.
    ...(process.env.VANTAGE_CHROME ? { executablePath: process.env.VANTAGE_CHROME } : {}),
    // No GPU flags on purpose: nothing here asks for a WebGL context, and a
    // gate that booted SwiftShader to move a `<div>` would be paying the bill
    // this file exists to stop paying.
  });
} catch (e) {
  fail(`chromium failed to launch: ${e.message}\n  install it: npx playwright install chromium`);
}

// A small viewport, deliberately: no frames are rendered, and the HUD is
// fixed-position overlay chrome that must lay out at a modest size anyway.
const context = await browser.newContext({ viewport: { width: 800, height: 600 } });
const page = await context.newPage();
const errors = [];
page.on("pageerror", (e) => errors.push(`pageerror: ${e.message}`));
page.on("console", (m) => {
  if (m.type() === "error") errors.push(`console.error: ${m.text()}`);
});

// The one interception. `index.html` is served byte-for-byte off disk — the
// point is to test the shipped scaffold — but its module entry would drag in
// three.js, the wasm bridge and a WebTransport connect to a shard that is not
// running. Stubbed to an empty module: the page below is the real HUD markup,
// the real CSS, and the real `Hud`/`InputTracker`, with the game never booted.
let mainRequests = 0;
await page.route("**/src/main.js", (route) => {
  mainRequests++;
  return route.fulfill({ status: 200, contentType: "text/javascript", body: "" });
});

await page.goto(`http://127.0.0.1:${port}/`, { waitUntil: "load" });

// =============================================================================
// A. the scaffold — what lets index.html leave the renderer tier
// =============================================================================
const scaffold = await page.evaluate((ids) => {
  const gl = document.getElementById("gl");
  const cs = gl ? getComputedStyle(gl) : null;
  const entry = [...document.querySelectorAll("script[type=module][src]")].map((s) =>
    s.getAttribute("src"),
  );
  const el = (id) => document.getElementById(id);
  const tag = (id) => el(id)?.tagName || null;
  const death = el("death");
  return {
    missing: ids.filter((id) => !el(id)),
    hasCanvas: !!gl && gl.tagName === "CANVAS",
    canvasDisplay: cs ? cs.display : null,
    canvasW: gl ? gl.clientWidth : 0,
    canvasH: gl ? gl.clientHeight : 0,
    entry,
    // The join form, as browser_smoke drives it: it fills #url and #cert and
    // clicks #connect (:1445-1447) with no assertion of its own — Playwright
    // throws on a missing or non-editable node and the throw is uncaught, so
    // a renamed input reads there as an unexplained crash. Here it is a
    // sentence.
    join: {
      url: tag("url"),
      cert: tag("cert"),
      connect: tag("connect"),
      urlDisabled: el("url")?.disabled ?? null,
      certDisabled: el("cert")?.disabled ?? null,
      connectDisabled: el("connect")?.disabled ?? null,
      // :1530 reads `#start`'s INLINE display and calls anything that is not
      // the exact string "none" a form still up. Note the `?.` there: a
      // MISSING #start reads "" !== "none" → still-up → the boot ladder
      // reports "stuck on rung 0" for an element that does not exist. That
      // is the assertion this line makes honest.
      startPresent: !!el("start"),
      startInline: el("start") ? el("start").style.display : null,
      // :1534/:1612 — a non-empty #starterr fails the join with its text as
      // the reason, so it must start empty and it must be a node that can
      // hold text at all.
      errPresent: !!el("starterr"),
      errText: el("starterr")?.textContent ?? null,
    },
    // :1949-1956 — on a LIVE body the death screen must compute to none, and
    // it is a computed read there because `#death { display: none }` is a
    // stylesheet rule in index.html (not an inline style), so a CSS edit that
    // dropped the rule would raise the overlay over a living player with no
    // JS involved. Group G below only ever looks after showDeath() has run;
    // this is the untouched-page state, which is the one browser_smoke holds.
    deathAtLoad: death ? getComputedStyle(death).display : null,
    deathParts: ["respawnbag", "respawnbeach", "deathcause"].filter((id) => !el(id)),
  };
}, HUD_IDS);

check(
  scaffold.missing.length === 0,
  `index.html is missing element(s) the Hud constructor reaches for: ${scaffold.missing.join(", ")}` +
    " — the client would throw on construction",
);
check(scaffold.hasCanvas, "index.html has no <canvas id=\"gl\"> — the renderer has nothing to mount on");
check(
  scaffold.canvasDisplay !== "none",
  `#gl computes display:${scaffold.canvasDisplay} — the renderer would draw into a hidden canvas`,
);
check(
  scaffold.canvasW > 0 && scaffold.canvasH > 0,
  `#gl lays out at ${scaffold.canvasW}x${scaffold.canvasH} — a zero-sized canvas renders nothing`,
);
check(
  scaffold.entry.includes("/src/main.js"),
  `index.html loads no <script type="module" src="/src/main.js"> (found: ${scaffold.entry.join(", ") || "none"})` +
    " — the client has no entry point",
);
check(mainRequests === 1, `the page requested /src/main.js ${mainRequests} times, expected exactly 1`);

// --- the contracts browser_smoke holds against index.html -------------------
// Everything from here to the end of group A exists because `browser_smoke`
// asserts it and this gate did not. See the table in the header.
const j = scaffold.join;
check(
  j.url === "INPUT" && j.cert === "INPUT",
  `the join form's fields are <${j.url}>/<${j.cert}>, expected two <input> — browser_smoke fills them by id`,
);
check(
  j.connect === "BUTTON",
  `#connect is <${j.connect}>, expected <button> — browser_smoke clicks it to start every one of its tabs`,
);
check(
  j.urlDisabled === false && j.certDisabled === false && j.connectDisabled === false,
  `the join form ships disabled (url=${j.urlDisabled}, cert=${j.certDisabled}, connect=${j.connectDisabled})` +
    " — nothing could type a shard address into it",
);
check(
  j.startPresent,
  "index.html has no #start — and browser_smoke:1530 cannot tell that from a form that is still up," +
    " so its failure would read as 'stuck on rung 0' for an element that does not exist",
);
check(
  j.startInline === "",
  `#start ships with an inline display of ${JSON.stringify(j.startInline)} — main.js:116 sets that field to` +
    ' "none" on connect and browser_smoke reads it back, so the scaffold must leave it clear',
);
check(j.errPresent, "index.html has no #starterr — a client that refused to boot has nowhere to say why");
check(
  j.errText === "",
  `#starterr ships carrying ${JSON.stringify(j.errText)} — browser_smoke fails any join whose error line is` +
    " non-empty, so a placeholder here would fail every boot",
);
check(
  scaffold.deathAtLoad === "none",
  `#death computes display:${scaffold.deathAtLoad} on an untouched page — the death screen is up over a` +
    " live player, which is a stylesheet edit away and needs no JS at all",
);
check(
  scaffold.deathParts.length === 0,
  `the death screen is missing ${scaffold.deathParts.join(", ")} — a dead player could not answer it`,
);

// --- construct the real HUD and tracker against the real scaffold ------------
const built = await page.evaluate(async () => {
  const { Hud } = await import("/src/hud.js");
  const { InputTracker } = await import("/src/input.js");
  // `main.js` hides the join form on connect (main.js:116) and is stubbed
  // here, so the form would sit over the death screen and swallow its clicks.
  // The harness stands in for that one line, and only that one.
  document.getElementById("start").style.display = "none";
  const hud = new Hud();
  const input = new InputTracker(document.getElementById("gl"));
  hud.show();
  // The class as well as the instance: group K asserts the constructor's
  // arming default by identity against `Hud.NO_MOVE_HOST`, which needs the
  // real class object and not a second import that could resolve elsewhere.
  const ui = { hud, Hud, input, respawns: [], chats: [] };
  hud.onRespawn = (onBag) => ui.respawns.push(onBag);
  hud.onChatSend = (line) => {
    ui.chats.push(line);
    return true;
  };
  globalThis.__ui = ui;
  return {
    cells: document.querySelectorAll("#hotbar .hotcell").length,
    hotbarShown: getComputedStyle(document.getElementById("hotbar")).display,
    selected: hud.selected,
    sel: input.sel,
  };
});

// =============================================================================
// B. the hotbar — six slots, one selection, and the digit keys that drive it
// =============================================================================
// `ALPHA.md` §1 fixes the belt at 6 slots and `main.js` derives the strings
// from the first 6 of wasm's 30 inventory slots. The count is a contract
// between three files and nothing asserted it.
check(built.cells === 6, `the hotbar built ${built.cells} cells, expected 6 (ALPHA.md §1: 6-slot belt)`);
check(built.hotbarShown === "flex", `Hud.show() left #hotbar at display:${built.hotbarShown}, expected flex`);
check(built.selected === -1, `a fresh Hud starts at selected=${built.selected}, expected -1 (nothing highlighted)`);
check(built.sel === 0, `a fresh InputTracker starts at sel=${built.sel}, expected 0`);

const hotbar = await page.evaluate(() => {
  const { hud } = globalThis.__ui;
  const out = { texts: null, sel: [] };
  hud.setHotbar(["rock", "torch", "", "hatchet", "", "bandage"]);
  out.texts = [...document.querySelectorAll("#hotbar .hotcell span")].map((s) => s.textContent);
  for (let i = 0; i < 6; i++) {
    hud.setSelected(i);
    const marked = [...document.querySelectorAll("#hotbar .hotcell")]
      .map((c, j) => (c.classList.contains("sel") ? j : -1))
      .filter((j) => j >= 0);
    out.sel.push(marked);
  }
  return out;
});
check(
  JSON.stringify(hotbar.texts) === JSON.stringify(["rock", "torch", "", "hatchet", "", "bandage"]),
  `setHotbar did not land its strings in the cells: ${JSON.stringify(hotbar.texts)}`,
);
for (let i = 0; i < 6; i++) {
  check(
    hotbar.sel[i].length === 1 && hotbar.sel[i][0] === i,
    `setSelected(${i}) marked cells ${JSON.stringify(hotbar.sel[i])} — exactly one cell carries .sel`,
  );
}

// Real keystrokes now, not method calls. `input.sel` is the payload `main.js`
// sends on a consume (main.js:648), so a digit that selected a slot outside
// 0..5 would be a forged-looking action, not a cosmetic slip.
const digits = [];
for (const [code, want] of [
  ["Digit1", 0],
  ["Digit3", 2],
  ["Digit6", 5],
]) {
  await page.keyboard.press(code);
  digits.push(
    await page.evaluate(() => {
      const { hud, input } = globalThis.__ui;
      // What main.js's slow timer does every tick (main.js:1309) — the whole
      // "press 4, slot 4 lights up" verb, end to end across both modules.
      hud.setSelected(input.sel);
      const marked = [...document.querySelectorAll("#hotbar .hotcell")].findIndex((c) =>
        c.classList.contains("sel"),
      );
      return { sel: input.sel, marked };
    }),
  );
  const got = digits[digits.length - 1];
  check(got.sel === want, `${code} set input.sel=${got.sel}, expected ${want}`);
  check(got.marked === want, `${code} highlighted cell ${got.marked}, expected ${want}`);
}

// The guard at input.js:49 (`n >= 0 && n <= 5`). Digit7 decodes to 6 — one
// past the belt — and must be ignored rather than clamped or accepted.
await page.keyboard.press("Digit7");
const afterSeven = await page.evaluate(() => globalThis.__ui.input.sel);
check(afterSeven === 5, `Digit7 moved the selection to ${afterSeven} — slot 6 does not exist`);

// =============================================================================
// C. the composer swallows keys — the ordering law, driven for real
// =============================================================================
// `hud.js:32` stops propagation on every key the composer sees so that "w" is
// a letter and not a step forward. `InputTracker` listens on `document`, so
// this is a real bubble-phase question that only a real browser answers. Same
// key, opposite result, one state bit apart.
await page.evaluate(() => globalThis.__ui.hud.openChat());
await page.keyboard.down("KeyW");
const whileOpen = await page.evaluate(() => {
  const { hud, input } = globalThis.__ui;
  return {
    open: hud.chatOpen,
    w: input.keys.w,
    moveZ: input.moveZ(),
    value: document.getElementById("chatinput").value,
    focused: document.activeElement?.id,
    held: document.getElementById("chatlog").classList.contains("held"),
  };
});
await page.keyboard.up("KeyW");
check(whileOpen.open === true, "openChat() left hud.chatOpen false");
check(whileOpen.focused === "chatinput", `openChat() focused '${whileOpen.focused}', expected chatinput`);
check(whileOpen.held === true, "openChat() did not hold the chat log (#chatlog is missing .held)");
check(
  whileOpen.w === false && whileOpen.moveZ === 0,
  `typing in the composer walked the player forward (keys.w=${whileOpen.w}, moveZ=${whileOpen.moveZ})`,
);
check(whileOpen.value === "w", `the composer did not receive the keystroke (value '${whileOpen.value}')`);

// Enter sends the trimmed line through onChatSend and closes; the composer
// clears on the next open (hud.js:255) so a sent line cannot be sent twice.
await page.keyboard.type(" hello ");
await page.keyboard.press("Enter");
const afterSend = await page.evaluate(() => {
  const { hud } = globalThis.__ui;
  return {
    chats: globalThis.__ui.chats,
    open: hud.chatOpen,
    display: getComputedStyle(document.getElementById("chat")).display,
    held: document.getElementById("chatlog").classList.contains("held"),
  };
});
check(
  afterSend.chats.length === 1 && afterSend.chats[0] === "w hello",
  `Enter sent ${JSON.stringify(afterSend.chats)}, expected one trimmed line 'w hello'`,
);
check(afterSend.open === false, "Enter left the composer open");
check(afterSend.display === "none", `Enter left #chat at display:${afterSend.display}`);
check(afterSend.held === false, "closing the composer did not release the chat log hold");

// And now the same key reaches the tracker, because the composer is shut.
await page.keyboard.down("KeyW");
const whileShut = await page.evaluate(() => {
  const { input } = globalThis.__ui;
  return { w: input.keys.w, moveZ: input.moveZ(), buttons: input.buttons() };
});
check(
  whileShut.w === true && whileShut.moveZ === 127,
  `with the composer shut, KeyW did not reach the tracker (keys.w=${whileShut.w}, moveZ=${whileShut.moveZ})`,
);

// A held key must not survive focus loss (input.js:79). Without this the
// player walks into the sea while alt-tabbed.
await page.evaluate(() => window.dispatchEvent(new Event("blur")));
const afterBlur = await page.evaluate(() => {
  const { input } = globalThis.__ui;
  return { w: input.keys.w, moveZ: input.moveZ(), buttons: input.buttons() };
});
check(
  afterBlur.w === false && afterBlur.moveZ === 0 && afterBlur.buttons === 0,
  `a window blur left keys held (keys.w=${afterBlur.w}, moveZ=${afterBlur.moveZ}, buttons=${afterBlur.buttons})`,
);
await page.keyboard.up("KeyW");

// Escape closes without sending — the other exit from the composer.
await page.evaluate(() => globalThis.__ui.hud.openChat());
await page.keyboard.type("discarded");
await page.keyboard.press("Escape");
const afterEsc = await page.evaluate(() => ({
  chats: globalThis.__ui.chats.length,
  open: globalThis.__ui.hud.chatOpen,
}));
check(afterEsc.open === false, "Escape left the composer open");
check(afterEsc.chats === 1, `Escape sent a line (${afterEsc.chats} sends, expected the 1 from Enter)`);

// =============================================================================
// D. the chat log — another player's bytes, and the cap
// =============================================================================
// `hud.js:279` states the law in a comment and nothing enforced it: the line
// is another player's bytes, so it goes in as text and never as markup. This
// is the assertion, and the cap check rides with it. Both run inside one
// evaluate so the 12 s reaper cannot race the read.
const HOSTILE = '<img src=x onerror="globalThis.__pwned=1"><b>bold</b>';
const chat = await page.evaluate(
  ([hostile, cap]) => {
    const { hud } = globalThis.__ui;
    const log = document.getElementById("chatlog");
    hud.chatLine(7, false, hostile, false);
    const injected = {
      elements: log.querySelectorAll("img, b").length,
      pwned: !!globalThis.__pwned,
      text: log.lastElementChild.textContent,
      who: log.lastElementChild.querySelector(".who")?.textContent,
    };
    for (let i = 0; i < cap + 3; i++) hud.chatLine(1, true, `line ${i}`, false);
    return {
      injected,
      kept: log.childElementCount,
      first: log.firstElementChild.textContent,
      globalMarked: log.lastElementChild.classList.contains("global"),
    };
  },
  [HOSTILE, CHAT_CAP],
);
check(
  chat.injected.elements === 0 && chat.injected.pwned === false,
  `a chat line built ${chat.injected.elements} element(s) out of another player's bytes (pwned=${chat.injected.pwned})` +
    " — the line must go in as text, never as markup",
);
check(
  chat.injected.text === `#7 ${HOSTILE}`,
  `the hostile line did not survive verbatim as text: ${JSON.stringify(chat.injected.text)}`,
);
check(chat.injected.who === "#7", `the speaker rendered as ${JSON.stringify(chat.injected.who)}, expected '#7'`);
check(chat.globalMarked, "a global chat line is not marked .global");
check(chat.kept === CHAT_CAP, `the chat log kept ${chat.kept} lines, CHAT_CAP is ${CHAT_CAP}`);
check(
  chat.first === "[g] #1 line 3",
  `the chat log evicted the wrong end: oldest surviving line is ${JSON.stringify(chat.first)}, expected '[g] #1 line 3'`,
);

// =============================================================================
// E. toasts — the gather feedback, and its cap
// =============================================================================
const toasts = await page.evaluate((cap) => {
  const { hud } = globalThis.__ui;
  const box = document.getElementById("toasts");
  for (let i = 0; i < cap + 3; i++) hud.toast(`+${i} Wood`);
  return { kept: box.childElementCount, first: box.firstElementChild.textContent };
}, TOAST_CAP);
check(toasts.kept === TOAST_CAP, `the toast stack kept ${toasts.kept}, TOAST_CAP is ${TOAST_CAP}`);
check(
  toasts.first === "+3 Wood",
  `the toast stack evicted the wrong end: oldest surviving is ${JSON.stringify(toasts.first)}, expected '+3 Wood'`,
);

// =============================================================================
// F. vitals — "no reading" and "empty" are opposite facts
// =============================================================================
// `hud.js:93-97` is explicit: a meter whose max is 0 means the server has
// stated nothing about it and is not drawn, while a meter at 0/100 is drawn
// loudly. A bar that rendered them the same would be lying at the worst
// moment. Nothing asserted the difference until now.
//
// The first check is a POSITIONAL one, and it is the reason this group leads
// with three distinct numbers. `setVitals` takes (hp, max, food, maxFood,
// water, maxWater) but stacks its rows hp / WATER / FOOD — the argument order
// and the row order disagree by design. CLAUDE.md's trap list names exactly
// this shape as where the reference ecosystem actually bled: the right value
// in the wrong position, invisible to every byte-level gate. Distinct values
// make a swap fail here.
const vitals = await page.evaluate(() => {
  const { hud } = globalThis.__ui;
  const box = document.getElementById("vitals");
  const read = () =>
    [...box.querySelectorAll(".vrow")].map((r) => ({
      shown: getComputedStyle(r).display !== "none",
      // browser_smoke:1748 reads the INLINE style, not the computed one, and
      // treats "none" as an absent meter. The two agree today only because
      // hud.js:138 writes the field directly; a rewrite that moved the
      // hide to a class would keep every computed check above green and
      // silently blind that gate. Captured separately so it cannot.
      inline: r.style.display,
      empty: r.classList.contains("empty"),
      num: r.querySelector(".vnum").textContent,
      width: r.querySelector(".vfill").style.width,
      kind: r.querySelector(".vfill").className,
    }));
  hud.setVitals(90, 100, 40, 100, 70, 100);
  const positional = read();
  // Read here, with meters live — not at the end of this function, where the
  // all-zero call below has already hidden the stack. The previous version
  // captured it there, which is why it could not be asserted.
  const boxShown = box.style.display;
  hud.setVitals(0, 100, 5, 100, 5, 100);
  const zeroed = read();
  hud.setVitals(50, 100, 0, 0, 0, 0);
  const unstated = read();
  hud.setVitals(150, 100, 33, 100, -5, 100);
  const clamped = read();
  hud.setVitals(0, 0, 0, 0, 0, 0);
  const silent = getComputedStyle(box).display;
  return { positional, zeroed, unstated, clamped, silent, boxShown };
});
const [hpRow, waterRow, foodRow] = vitals.positional;
check(
  hpRow.num === "90" && waterRow.num === "70" && foodRow.num === "40",
  "the vitals rows carry the wrong values — row order is hp/water/food but the arguments are " +
    `(hp, max, food, maxFood, water, maxWater); got ${hpRow.num}/${waterRow.num}/${foodRow.num}, expected 90/70/40`,
);
check(
  waterRow.kind.includes("water") && foodRow.kind.includes("food"),
  `the vitals fills are not the meters they claim: row1='${waterRow.kind}', row2='${foodRow.kind}'`,
);
// browser_smoke:1752-1756 keys a row by class and keys HEALTH BY ABSENCE:
// water if `.water`, food if `.food`, otherwise health. So a `.vfill` that
// grew a second class, or a health fill that gained either, is read as the
// wrong meter and the value comparison at :1794 then fails against the sim
// with a message about the wrong row. Exclusivity is the real contract.
check(
  !hpRow.kind.includes("water") && !hpRow.kind.includes("food"),
  `the health fill carries a meter class ('${hpRow.kind}') — browser_smoke keys health by the ABSENCE of both,` +
    " so this row would be read as the water or food meter",
);
check(
  !waterRow.kind.includes("food") && !foodRow.kind.includes("water"),
  `a vitals fill carries both meter classes (water='${waterRow.kind}', food='${foodRow.kind}') — the class` +
    " that keys the row must be exactly one",
);
// :1757 → :1794 does exact string equality between this text and
// `String(sim)`. A unit, a comma, a "/100" or a decimal point makes every
// vitals comparison in browser_smoke fail on a HUD that looks perfect.
for (const [label, row] of [
  ["health", hpRow],
  ["water", waterRow],
  ["food", foodRow],
]) {
  check(
    /^\d+$/.test(row.num),
    `the ${label} readout is ${JSON.stringify(row.num)} — it is compared against String(sim) by exact` +
      " equality, so it must be a bare integer with no unit, separator or suffix",
  );
}
check(hpRow.width === "90%", `a 90/100 meter filled to ${hpRow.width}`);
check(
  vitals.zeroed[0].shown === true && vitals.zeroed[0].empty === true,
  `a meter at 0/100 must be drawn and marked empty (shown=${vitals.zeroed[0].shown}, empty=${vitals.zeroed[0].empty})`,
);
check(
  vitals.unstated[1].shown === false && vitals.unstated[2].shown === false,
  "a meter the server has stated nothing about (max 0) must not be drawn — it is not the same fact as empty",
);
check(vitals.unstated[0].shown === true, "the health row vanished while its max was still 100");
check(
  vitals.unstated[1].inline === "none" && vitals.unstated[2].inline === "none",
  `an unstated meter hides at inline display '${vitals.unstated[1].inline}'/'${vitals.unstated[2].inline}'` +
    " — browser_smoke:1748 reads that exact field and counts anything else as a live meter",
);
check(
  vitals.positional[0].inline === "flex",
  `a live meter shows at inline display '${vitals.positional[0].inline}' — the same field, the shown case`,
);
// The one the judge caught: this value was read and thrown away. It is the
// whole of browser_smoke:1745 — `#vitals` not at inline "block" early-returns
// `{shown:false}` and fails at :1768 with "no health reached the HUD". A HUD
// that switched to `display:grid` or to a class would pass every other check
// in this group and take that gate down with a message about the shard.
check(
  vitals.boxShown === "block",
  `the vitals stack shows at inline display ${JSON.stringify(vitals.boxShown)}, expected "block"` +
    " — browser_smoke:1745 tests that string exactly and reports anything else as the shard sending no health",
);
check(
  vitals.clamped[0].width === "100%" && vitals.clamped[1].width === "0%",
  `the fill did not clamp to 0..100%: over-full read ${vitals.clamped[0].width}, negative read ${vitals.clamped[1].width}`,
);
check(
  vitals.silent === "none",
  `a shard that states no meter at all must hide the stack entirely, got display:${vitals.silent}`,
);

// =============================================================================
// G. the death screen — answered once, with real clicks
// =============================================================================
// `hud.js:318-321`: the buttons disable on the click rather than on the wake,
// so a second press cannot send a second action into a screen the server has
// already closed. And `ALPHA.md` §1 forbids a map position in the sentence —
// a screen that told you where you fell would hand the raider standing over
// you a pin to the base they just cleared. The cause line is asserted exactly
// so that adding one fails here.
const shown = await page.evaluate(() => {
  const { hud } = globalThis.__ui;
  hud.showDeath(0, 3, "stone hatchet", 12.53, false);
  return {
    display: getComputedStyle(document.getElementById("death")).display,
    cause: document.getElementById("deathcause").textContent,
    note: document.getElementById("deathnote").textContent,
    open: hud.deathOpen,
    disabled: [
      document.getElementById("respawnbag").disabled,
      document.getElementById("respawnbeach").disabled,
    ],
  };
});
check(shown.display === "flex", `showDeath left #death at display:${shown.display}`);
check(shown.open === true, "showDeath left hud.deathOpen false");
check(
  shown.cause === "#3 killed you with stone hatchet from 12.5 m",
  `the death sentence read ${JSON.stringify(shown.cause)} — expected who/weapon/range and no position`,
);
check(
  shown.disabled[0] === false && shown.disabled[1] === false,
  "showDeath raised a screen whose buttons were already disabled",
);

await page.click("#respawnbeach");
const firstClick = await page.evaluate(() => ({
  respawns: globalThis.__ui.respawns,
  note: document.getElementById("deathnote").textContent,
  disabled: [
    document.getElementById("respawnbag").disabled,
    document.getElementById("respawnbeach").disabled,
  ],
}));
check(
  firstClick.respawns.length === 1 && firstClick.respawns[0] === false,
  `one real click on 'beach' sent ${JSON.stringify(firstClick.respawns)}, expected exactly [false]`,
);
check(
  firstClick.disabled[0] === true && firstClick.disabled[1] === true,
  `the answer left buttons live (bag=${firstClick.disabled[0]}, beach=${firstClick.disabled[1]})`,
);
check(firstClick.note !== "", "the answered screen gives the player no acknowledgement");

// The second press. Dispatched rather than clicked, on purpose: the `disabled`
// attribute alone is not the law — `answerDeath` re-checks it (hud.js:323)
// and that early return is what a stray dispatch, a re-shown screen or a
// keyboard path would meet. Asserting only the attribute would leave the
// guard untested.
const secondClick = await page.evaluate(() => {
  document.getElementById("respawnbeach").dispatchEvent(new MouseEvent("click", { bubbles: true }));
  document.getElementById("respawnbag").dispatchEvent(new MouseEvent("click", { bubbles: true }));
  return globalThis.__ui.respawns.length;
});
check(
  secondClick === 1,
  `a second press sent another action (${secondClick} total) — the sim ignores it and the player is left guessing`,
);

// The wake, and the case the player has no other way to learn: asking for a
// bag inside its cooldown gets a beach (hud.js:330-341).
const woke = await page.evaluate(() => {
  const { hud } = globalThis.__ui;
  document.getElementById("toasts").textContent = "";
  hud.hideDeath(false, true);
  return {
    display: getComputedStyle(document.getElementById("death")).display,
    open: hud.deathOpen,
    toast: document.getElementById("toasts").lastElementChild?.textContent || "",
  };
});
check(woke.display === "none" && woke.open === false, "hideDeath left the death screen up");
check(
  woke.toast.includes("no bag"),
  `a refused bag told the player ${JSON.stringify(woke.toast)} — they must learn the bag did not answer`,
);

// =============================================================================
// H. craft, the queue, and the build strip — the surfaces that spend materials
// =============================================================================
// `setCraft` (hud.js:161) is the largest DOM builder in the file and neither
// gate asserted a line of it; `setCraftQueue` (:202) and `setBuild` (:224)
// were the same. They are grouped here because they share one failure mode:
// each carries an INDEX or a COUNT from the panel back into an action that
// spends the player's materials, and that is the positional-payload shape
// CLAUDE.md's trap list names — the right value in the wrong position, which
// every byte-level gate in this repo is blind to.
const CRAFT_ROWS = [
  {
    recipe: 4,
    name: "stone hatchet",
    count: 1,
    seconds: 7,
    gated: false,
    gateText: "",
    inputs: [
      { text: "wood 100", ok: true },
      { text: "stone 40", ok: false },
    ],
  },
  { recipe: 9, name: "arrow", count: 4, seconds: 2, gated: false, gateText: "", inputs: [{ text: "wood 25", ok: true }] },
  {
    recipe: 12,
    name: "sheet door",
    count: 1,
    seconds: 30,
    gated: true,
    gateText: "workbench 2",
    inputs: [{ text: "metal 200", ok: true }],
  },
];

const craft = await page.evaluate((rows) => {
  const { hud } = globalThis.__ui;
  const panel = document.getElementById("craft");
  const calls = [];
  // Built twice on purpose: main.js rebuilds this panel from a slow timer, so
  // a builder that appended instead of clearing would grow without bound and
  // every row would carry a stale click handler.
  hud.setCraft(rows, (recipe, n) => calls.push([recipe, n]));
  hud.setCraft(rows, (recipe, n) => calls.push([recipe, n]));
  const crows = [...panel.querySelectorAll(".crow")];
  const shape = crows.map((d) => ({
    gated: d.classList.contains("gated"),
    name: d.querySelector(".cname").textContent,
    ins: [...d.querySelectorAll(".cin")].map((s) => s.className),
    gate: d.querySelector(".gate")?.textContent ?? null,
  }));
  // A plain click, a shift-click, and a click on the gated row. The third is
  // the one that matters: a recipe the player cannot yet make must not be
  // able to spend anything.
  crows[0].dispatchEvent(new MouseEvent("click", { bubbles: true }));
  crows[1].dispatchEvent(new MouseEvent("click", { bubbles: true, shiftKey: true }));
  crows[2].dispatchEvent(new MouseEvent("click", { bubbles: true }));
  return { count: crows.length, shape, calls, heading: panel.querySelector("h2")?.textContent ?? null };
}, CRAFT_ROWS);

check(
  craft.count === CRAFT_ROWS.length,
  `the craft panel holds ${craft.count} rows after two rebuilds of ${CRAFT_ROWS.length} recipes` +
    " — it must clear before it builds, or a slow-timer rebuild grows it forever",
);
check(craft.heading === "CRAFT", `the craft panel's heading reads ${JSON.stringify(craft.heading)}`);
check(
  craft.shape[0].name === "stone hatchet" && craft.shape[1].name === "arrow ×4",
  `a craft row names its output wrongly: got ${JSON.stringify(craft.shape[0].name)} and` +
    ` ${JSON.stringify(craft.shape[1].name)}, expected 'stone hatchet' and 'arrow ×4' (count>1 shows the ×)`,
);
check(
  JSON.stringify(craft.shape[0].ins) === JSON.stringify(["cin ok", "cin miss"]),
  `the ingredient marks do not follow their ok flags: ${JSON.stringify(craft.shape[0].ins)} for` +
    " [have, missing] — a player reads affordability off exactly this",
);
check(
  craft.shape[2].gated === true && craft.shape[2].gate === " · workbench 2",
  `the gated recipe is not marked as gated (gated=${craft.shape[2].gated}, gate=${JSON.stringify(craft.shape[2].gate)})`,
);
check(
  craft.shape[0].gate === null && craft.shape[1].gate === null,
  "an ungated recipe is carrying a gate badge",
);
// The three clicks, in order: recipe 4 ×1, recipe 9 ×5, and nothing at all.
check(
  JSON.stringify(craft.calls) === JSON.stringify([
    [4, 1],
    [9, 5],
  ]),
  `the craft panel sent ${JSON.stringify(craft.calls)}, expected [[4,1],[9,5]] — a plain click crafts one,` +
    " shift crafts five, and the workbench-gated row must send nothing at all",
);

// The queue strip. `onCancel(j.index)` takes the job's OWN index field, which
// is deliberately not its position in the array here: a cancel that sent the
// array slot would cancel a different job, and the player would watch the
// wrong craft disappear.
const queue = await page.evaluate(() => {
  const { hud } = globalThis.__ui;
  const strip = document.getElementById("craftq");
  const cancels = [];
  hud.setCraftQueue([], () => cancels.push("empty"));
  const hidden = strip.style.display;
  hud.setCraftQueue(
    [
      { index: 3, label: "hatchet 4s" },
      { index: 7, label: "arrow ×4 2s" },
    ],
    (i) => cancels.push(i),
  );
  const cells = [...strip.querySelectorAll(".qcell")];
  cells[1].dispatchEvent(new MouseEvent("click", { bubbles: true }));
  return {
    hidden,
    shown: strip.style.display,
    labels: cells.map((c) => c.textContent),
    titled: cells.every((c) => c.title.length > 0),
    cancels,
  };
});
check(queue.hidden === "none", `an empty craft queue shows at display:${queue.hidden} — an empty strip is clutter`);
check(queue.shown === "flex", `a two-job craft queue shows at display:${queue.shown}, expected flex`);
check(
  JSON.stringify(queue.labels) === JSON.stringify(["hatchet 4s", "arrow ×4 2s"]),
  `the queue strip reads ${JSON.stringify(queue.labels)}`,
);
check(queue.titled, "a queue cell carries no title — nothing tells the player a click cancels it");
check(
  JSON.stringify(queue.cancels) === JSON.stringify([7]),
  `clicking the second job cancelled ${JSON.stringify(queue.cancels)}, expected [7] — the job's own index,` +
    " not its slot in the strip; sending the slot cancels somebody else's craft",
);

// The build strip: one line, and the empty string is the hide.
const build = await page.evaluate(() => {
  const { hud } = globalThis.__ui;
  const el = document.getElementById("build");
  const out = {};
  hud.setBuild("wall · wood · 200 wood");
  out.text = el.textContent;
  out.shown = el.style.display;
  hud.setBuild("");
  out.cleared = el.textContent;
  out.hidden = el.style.display;
  return out;
});
check(build.text === "wall · wood · 200 wood", `the build strip reads ${JSON.stringify(build.text)}`);
check(build.shown === "block", `a live build strip shows at display:${build.shown}, expected block`);
check(
  build.hidden === "none" && build.cleared === "",
  `leaving build mode left the strip at display:${build.hidden} reading ${JSON.stringify(build.cleared)}`,
);

// =============================================================================
// I. the inventory screen — 30 slots, and which cell each one lands in
// =============================================================================
// The load-bearing claim of `setInventory` is POSITIONAL: `texts[s]` is slot
// `s` as the sim numbers it, belt 0..5 then grid 6..29. CLAUDE.md's own trap
// list says this is where the reference ecosystem actually bled — 27 Oxide
// commits correcting a payload that had already shipped wrong, "the right
// value in the wrong position", four hooks corrected more than once — and
// that a byte-level golden catches none of it, because every field has the
// same type. Every one of these 30 cells is a string in a string array. So
// the check writes a DISTINCT string into each slot and reads back which DOM
// node holds it: an off-by-six or a swapped belt/grid fails on 30 cells at
// once, and no amount of "the array had 30 entries" would have noticed.
//
// What this group does NOT cover, stated rather than implied: the `Tab` bind
// and the `hud.eatsKey` call site both live in `main.js`, which this gate
// stubs at the route (see the header). `eatsKey`'s own truth table is driven
// below; its caller is not driven anywhere, here or in `browser_smoke`.
const invBuilt = await page.evaluate(() => {
  const { hud } = globalThis.__ui;
  return {
    open: hud.invOpen,
    computed: getComputedStyle(document.getElementById("inv")).display,
    grid: document.querySelectorAll("#invgrid .invcell").length,
    belt: document.querySelectorAll("#invbelt .invcell").length,
    cells: hud.invCells.length,
    focus: hud.invFocus,
    selected: hud.invSelected,
    detail: document.getElementById("invdetail").textContent,
  };
});
check(invBuilt.open === false, `a fresh Hud starts with invOpen=${invBuilt.open}, expected false`);
check(
  invBuilt.computed === "none",
  `#inv is visible at load (computed display:${invBuilt.computed}) — the inventory opens on a key, not on boot`,
);
check(
  invBuilt.grid === INV_GRID && invBuilt.belt === INV_BELT,
  `the panel built ${invBuilt.grid} grid + ${invBuilt.belt} belt cells, expected ${INV_GRID} + ${INV_BELT}`,
);
check(
  invBuilt.cells === INV_SLOTS,
  `hud.invCells holds ${invBuilt.cells} entries, expected ${INV_SLOTS} — one per wasm inventory slot`,
);
check(
  invBuilt.focus === -1 && invBuilt.selected === -1 && invBuilt.detail === "",
  `a fresh panel starts focus=${invBuilt.focus} selected=${invBuilt.selected} detail=${JSON.stringify(invBuilt.detail)},` +
    " expected nothing focused, nothing selected, and an empty readout",
);

const invToggle = await page.evaluate(() => {
  const { hud } = globalThis.__ui;
  const el = document.getElementById("inv");
  const opened = hud.toggleInv();
  const shown = { ret: opened, flag: hud.invOpen, inline: el.style.display };
  const closed = hud.toggleInv();
  return {
    shown,
    hidden: { ret: closed, flag: hud.invOpen, inline: el.style.display },
    reopened: hud.toggleInv(),
  };
});
check(
  invToggle.shown.ret === true && invToggle.shown.flag === true && invToggle.shown.inline === "flex",
  `toggleInv() opening returned ${invToggle.shown.ret}, left invOpen=${invToggle.shown.flag} at display:${invToggle.shown.inline}` +
    " — expected true/true/flex (main.js reads the return to decide whether to drop pointer lock)",
);
check(
  invToggle.hidden.ret === false && invToggle.hidden.flag === false && invToggle.hidden.inline === "none",
  `toggleInv() closing returned ${invToggle.hidden.ret}, left invOpen=${invToggle.hidden.flag} at display:${invToggle.hidden.inline}`,
);
check(invToggle.reopened === true, `toggleInv() did not reopen (returned ${invToggle.reopened})`);

// The positional law. Distinct string per slot, then read the DOM back in
// document order and rebuild the slot→cell map from what actually rendered.
const invPos = await page.evaluate((n) => {
  const { hud } = globalThis.__ui;
  const want = [];
  for (let s = 0; s < n; s++) want.push(`s${s}`);
  hud.setInventory(want);
  return {
    belt: [...document.querySelectorAll("#invbelt .invcell span")].map((e) => e.textContent),
    grid: [...document.querySelectorAll("#invgrid .invcell span")].map((e) => e.textContent),
  };
}, INV_SLOTS);
const wantBelt = Array.from({ length: INV_BELT }, (_, i) => `s${i}`);
const wantGrid = Array.from({ length: INV_GRID }, (_, i) => `s${i + INV_BELT}`);
check(
  JSON.stringify(invPos.belt) === JSON.stringify(wantBelt),
  `the belt row rendered ${JSON.stringify(invPos.belt)}, expected ${JSON.stringify(wantBelt)}` +
    " — slots 0..5 ARE the belt, and they are the six the digit keys select",
);
check(
  JSON.stringify(invPos.grid) === JSON.stringify(wantGrid),
  `the grid rendered ${JSON.stringify(invPos.grid)}, expected ${JSON.stringify(wantGrid)}` +
    ` — slot ${INV_BELT} is the first grid cell, not the first belt cell`,
);

// An empty slot draws nothing. `main.js` decides empty by COUNT (`inv[s*2+1]`,
// main.js:1305) and passes "", so a cell that kept its last string would show
// a player wood they no longer have.
const invEmpty = await page.evaluate((n) => {
  const { hud } = globalThis.__ui;
  const texts = [];
  for (let s = 0; s < n; s++) texts.push(s === 0 || s === 9 ? "" : `s${s}`);
  hud.setInventory(texts);
  const all = [
    ...[...document.querySelectorAll("#invbelt .invcell span")].map((e) => e.textContent),
    ...[...document.querySelectorAll("#invgrid .invcell span")].map((e) => e.textContent),
  ];
  return { slot0: all[0], slot9: all[9], slot1: all[1] };
}, INV_SLOTS);
check(
  invEmpty.slot0 === "" && invEmpty.slot9 === "",
  `emptied slots still read ${JSON.stringify(invEmpty.slot0)} / ${JSON.stringify(invEmpty.slot9)}` +
    " — a stale cell shows a player materials they have already spent",
);
check(invEmpty.slot1 === "s1", `clearing two slots also cleared slot 1 (${JSON.stringify(invEmpty.slot1)})`);

// Selection: the belt row inside the panel is the hotbar, so it carries the
// same `input.sel` and exactly one cell can be live. A grid cell must never
// take it — there is no key 7 and the sim's `client_set_input` only carries
// 0..5 (main.js:1226).
const invSel = await page.evaluate((belt) => {
  const { hud } = globalThis.__ui;
  const marked = () => {
    const b = [...document.querySelectorAll("#invbelt .invcell")]
      .map((c, i) => (c.classList.contains("sel") ? i : -1))
      .filter((i) => i >= 0);
    const g = [...document.querySelectorAll("#invgrid .invcell")].filter((c) =>
      c.classList.contains("sel"),
    ).length;
    return { b, g };
  };
  const out = [];
  for (let i = 0; i < belt; i++) {
    hud.setInvSelected(i);
    out.push(marked());
  }
  hud.setInvSelected(belt + 1);
  const past = marked();
  hud.setInvSelected(0);
  return { out, past };
}, INV_BELT);
for (let i = 0; i < INV_BELT; i++) {
  check(
    invSel.out[i].b.length === 1 && invSel.out[i].b[0] === i && invSel.out[i].g === 0,
    `setInvSelected(${i}) marked belt ${JSON.stringify(invSel.out[i].b)} and ${invSel.out[i].g} grid cells` +
      " — exactly one belt cell, never a grid cell",
  );
}
check(
  invSel.past.b.length === 0 && invSel.past.g === 0,
  `setInvSelected(${INV_BELT + 1}) marked ${JSON.stringify(invSel.past.b)} belt / ${invSel.past.g} grid cells` +
    " — a slot past the belt is not selectable and must clear the highlight rather than leave a stale one",
);

// Clicks, dispatched as real events so the listener path is what runs. A belt
// click is the same act as its digit key — it must reach `onInvSelect`, which
// main.js turns into `input.sel`. A grid click must NOT: there is no verb that
// makes slot 12 the held item, and firing one would send the sim a slot it
// refuses.
const invClick = await page.evaluate((n) => {
  const { hud } = globalThis.__ui;
  const picks = [];
  hud.onInvSelect = (s) => picks.push(s);
  const texts = [];
  for (let s = 0; s < n; s++) texts.push(s === 8 ? "" : `s${s}`);
  hud.setInventory(texts);
  const detail = document.getElementById("invdetail");
  const belt = [...document.querySelectorAll("#invbelt .invcell")];
  const grid = [...document.querySelectorAll("#invgrid .invcell")];
  const fire = (el) => el.dispatchEvent(new MouseEvent("click", { bubbles: true }));

  fire(belt[3]);
  const afterBelt = { picks: picks.slice(), focus: hud.invFocus, detail: detail.textContent };
  fire(grid[0]);
  const afterGrid = { picks: picks.slice(), focus: hud.invFocus, detail: detail.textContent };
  fire(grid[2]); // slot 8, emptied above
  const afterEmpty = { detail: detail.textContent };
  const focused = [...belt, ...grid].filter((c) => c.classList.contains("focus")).length;
  return { afterBelt, afterGrid, afterEmpty, focused };
}, INV_SLOTS);
check(
  JSON.stringify(invClick.afterBelt.picks) === "[3]" && invClick.afterBelt.focus === 3,
  `clicking belt cell 3 reported picks ${JSON.stringify(invClick.afterBelt.picks)} focus ${invClick.afterBelt.focus}` +
    " — expected exactly one onInvSelect(3), the same selection Digit4 makes",
);
check(
  invClick.afterBelt.detail === "belt 4 · s3",
  `the readout for belt slot 3 says ${JSON.stringify(invClick.afterBelt.detail)}, expected "belt 4 · s3"` +
    " — belt slots are named by the digit key that selects them, counting from 1",
);
check(
  JSON.stringify(invClick.afterGrid.picks) === "[3]" && invClick.afterGrid.focus === INV_BELT,
  `clicking the first grid cell reported picks ${JSON.stringify(invClick.afterGrid.picks)} focus ${invClick.afterGrid.focus}` +
    ` — it must focus slot ${INV_BELT} and fire NO onInvSelect; a grid slot cannot be the held item`,
);
check(
  invClick.afterGrid.detail === "slot 1 · s6",
  `the readout for the first grid cell says ${JSON.stringify(invClick.afterGrid.detail)}, expected "slot 1 · s6"` +
    " — grid slots number 1..24 within the grid, because there is no key 7 to name them by",
);
check(
  invClick.afterEmpty.detail === "slot 3 · empty",
  `an empty focused slot reads ${JSON.stringify(invClick.afterEmpty.detail)}, expected "slot 3 · empty"` +
    " — a blank readout and an empty slot are different facts, the same argument the vitals rows make",
);
check(
  invClick.focused === 1,
  `${invClick.focused} cells carry .focus after three clicks — focus moves, it does not accumulate`,
);

// A count that changes under an open panel has to reach the readout too: the
// slow timer calls setInventory every 250 ms and the focused slot's text is
// the same string the cell holds.
const invLive = await page.evaluate((n) => {
  const { hud } = globalThis.__ui;
  const texts = [];
  for (let s = 0; s < n; s++) texts.push(`wood ×${s}`);
  hud.setInventory(texts);
  return document.getElementById("invdetail").textContent;
}, INV_SLOTS);
check(
  invLive === "slot 3 · wood ×8",
  `after the slot's count changed the readout still says ${JSON.stringify(invLive)}, expected "slot 3 · wood ×8"`,
);

// `eatsKey` — the ordering law, as a truth table. main.js asks this once,
// ahead of every verb, and each of those verbs spends something: KeyE opens a
// door or loots a bag, KeyG eats, KeyH drinks, KeyC and KeyB open panels. Tab
// and Escape are the screen's own keys and must fall through.
const eats = await page.evaluate(() => {
  const { hud } = globalThis.__ui;
  const probe = ["KeyE", "KeyG", "KeyH", "KeyL", "KeyU", "KeyC", "KeyB", "Tab", "Escape"];
  const read = () => Object.fromEntries(probe.map((c) => [c, hud.eatsKey(c)]));
  if (hud.invOpen) hud.toggleInv();
  const closed = read();
  hud.toggleInv();
  const open = read();
  return { closed, open };
});
for (const code of ["KeyE", "KeyG", "KeyH", "KeyL", "KeyU", "KeyC", "KeyB", "Tab", "Escape"]) {
  check(
    eats.closed[code] === false,
    `with the panel closed eatsKey(${code}) said ${eats.closed[code]} — a closed panel owns no key at all`,
  );
}
for (const code of ["KeyE", "KeyG", "KeyH", "KeyL", "KeyU", "KeyC", "KeyB"]) {
  check(
    eats.open[code] === true,
    `with the panel open eatsKey(${code}) said ${eats.open[code]} — that verb spends something and was not asked for`,
  );
}
check(
  eats.open.Tab === false && eats.open.Escape === false,
  `an open panel ate Tab=${eats.open.Tab} Escape=${eats.open.Escape} — those are the keys that close it,` +
    " and a screen that swallowed its own exit would trap the player",
);

// Dying closes it. The death branch in main.js already refuses every verb on a
// corpse; a panel left open would be showing the slots of a body that is now
// lying on the ground as a bag.
const invDeath = await page.evaluate(() => {
  const { hud } = globalThis.__ui;
  if (!hud.invOpen) hud.toggleInv();
  const before = hud.invOpen;
  hud.showDeath(1, 0, "", 0, false);
  return {
    before,
    after: hud.invOpen,
    inline: document.getElementById("inv").style.display,
  };
});
check(
  invDeath.before === true && invDeath.after === false && invDeath.inline === "none",
  `dying with the inventory open left invOpen=${invDeath.after} at display:${invDeath.inline}, expected closed`,
);

// =============================================================================
// K. the item-move verb — validation ordering against the mutation
// =============================================================================
// CLAUDE.md's trap list calls this the most bug-prone thing in the reference
// and says exactly how it fails: three Oxide fixes in 28 minutes on one 2019
// day, all one-line splice-point moves on move/stack/loot, all landing as *the
// server disconnecting the client*, because container state diverged and a
// diverged container reads as a forged request. "The bug is validation
// ordering against the mutation, never arithmetic." So this group asserts the
// ORDER, not the sums — every check below is about what was mutated before
// what was checked, and which values a rollback restores.
//
// The refusal reasons come out of the sim rather than being restated here,
// for `pine_shape.mjs`'s reason: a number restated in a gate is a number that
// can drift away from the one that ships.
const invSrc = fs.readFileSync(path.join(root, "crates/sim-core/src/inventory.rs"), "utf8");

/**
 * The NUMBER a `pub const NAME: ty = …;` in `inventory.rs` carries, whether
 * it is written as a literal (`= 2;`) or as an alias for another const in the
 * same file (`pub const CONT_MAX: u8 = CONT_BOX;`). `null` when it cannot be
 * resolved, so a caller goes loudly red rather than comparing against `NaN`.
 *
 * Written on 2026-08-04 because the two ceiling checks below both read the
 * alias WRONG, in the two different ways an alias can be read wrong:
 *
 * - `CONT_MAX` pinned the alias's NAME (`contMaxAlias === "CONT_BAG"`) and
 *   then compared the client's number to the client's own `CONT_BAG`. It
 *   never read a Rust number at all. When the sim legitimately grew a third
 *   kind the name moved to `CONT_BOX` and the assertion became one that NO
 *   value of the client's constant could satisfy — a gate correct code
 *   cannot pass is testing the wrong thing, not merely an inconvenient one.
 * - `REFUSE_M_MAX` hardcoded which const the alias points AT
 *   (`REFUSE_M_UNSTACKABLE`). Green today and green for the wrong reason:
 *   it would keep passing while the alias moved somewhere else entirely.
 *
 * Resolving the alias catches both, and still catches everything the old
 * pair caught — a kind added on either side of the boundary lands red on the
 * commit that adds it rather than on the frame the server declines.
 */
const rustConst = (name, seen = new Set()) => {
  if (seen.has(name)) return null; // an alias cycle is unresolvable, not infinite
  seen.add(name);
  // Anchored to the start of a LINE (`m`), per the 2026-08-05 judge report's
  // ranked fix 1. Unanchored it took the first such form anywhere in the
  // file, a doc comment included — and `inventory.rs` already quotes const
  // declarations in prose near these very names, so a future comment could
  // silently shadow the declaration it was explaining.
  const m = invSrc.match(new RegExp(`^pub const ${name}: \\w+ = (\\w+);`, "m"));
  if (!m) return null;
  return /^\d+$/.test(m[1]) ? Number(m[1]) : rustConst(m[1], seen);
};

const REFUSE_MAX = rustConst("REFUSE_M_MAX");
check(
  Number.isInteger(REFUSE_MAX) && REFUSE_MAX >= 7,
  `could not resolve REFUSE_M_MAX out of inventory.rs (got ${REFUSE_MAX}) — the refusal table below` +
    " would then be checked against nothing, which is the gate-that-matches-nothing class",
);

// The arming default, asserted on a PRISTINE panel — before the test host
// below assigns over it. The previous pass gated "if `onInvMove` is the
// sentinel, no drag starts" by putting the sentinel BACK by hand, which is a
// hand-restored state and not the shipped one: reverting hud.js's constructor
// to `() => false` left that check green, because the check never looked at
// what the constructor actually does. Both halves are asserted here — the
// identity, and the behaviour that identity is supposed to buy.
const unarmed = await page.evaluate((n) => {
  const { hud, Hud } = globalThis.__ui;
  const t = [];
  for (let s = 0; s < n; s++) t.push(`s${s}`);
  hud.setInventory(t);
  return {
    isSentinel: hud.onInvMove === Hud.NO_MOVE_HOST,
    began: hud.beginInvDrag(3, 1),
    marked: hud.invDivs[3].classList.contains("drag"),
    drag: hud.invDrag,
  };
}, INV_SLOTS);
check(
  unarmed.isSentinel,
  "a freshly constructed Hud must sit at Hud.NO_MOVE_HOST — a constructor that arms the verb by default" +
    " hands the player a gesture no host can carry",
);
check(
  unarmed.began === false && unarmed.drag === -1 && unarmed.marked === false,
  `an unarmed panel began a drag (began=${unarmed.began}, invDrag=${unarmed.drag}, marked=${unarmed.marked})` +
    " on a FILLED slot — the affordance that always refuses is the one this rule exists to prevent",
);

// Read rather than restated, same reason as `REFUSE_MAX` above.
const selfKind = Number(invSrc.match(/pub const CONT_SELF: u8 = (\d+);/)?.[1]);
check(
  Number.isInteger(selfKind),
  "could not read CONT_SELF out of inventory.rs — the host below would then record addresses it could not check",
);

const move = await page.evaluate(
  ([n, belt, reasonMax, self]) => {
    const { hud } = globalThis.__ui;
    const sent = [];
    let allow = true;
    // The host takes ADDRESSES: (fromKind, from, toKind, to). This group
    // owns the SLOTS and the ordering, so it records the pair it always
    // did; group M owns the kinds. Any address naming a container other
    // than self is noted here anyway, so that "record only the slots"
    // cannot quietly become "the kinds are not looked at anywhere".
    const foreignKinds = [];
    hud.onInvMove = (fromKind, from, toKind, to) => {
      if (fromKind !== self || toKind !== self) foreignKinds.push([fromKind, toKind]);
      sent.push([from, to]);
      return allow;
    };
    const fill = () => {
      const t = [];
      for (let s = 0; s < n; s++) t.push(`s${s}`);
      hud.setInventory(t);
    };
    const cells = () => [
      ...document.querySelectorAll("#invbelt .invcell"),
      ...document.querySelectorAll("#invgrid .invcell"),
    ];
    const texts = () => cells().map((c) => c.querySelector("span").textContent);
    const down = (i) => cells()[i].dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, button: 0 }));
    const up = (i) => cells()[i].dispatchEvent(new PointerEvent("pointerup", { bubbles: true }));
    const out = {};

    // --- a drag from an empty slot is not a drag ----------------------------
    fill();
    hud.setInventory(Object.assign([], texts(), { 4: "" }));
    down(4);
    out.emptyDrag = { drag: hud.invDrag, marked: cells()[4].classList.contains("drag") };
    up(9);
    out.afterEmptyDrop = { sent: sent.length, s9: texts()[9] };

    // --- the happy path: draw it, then have it land -------------------------
    fill();
    sent.length = 0;
    down(0);
    out.picked = { drag: hud.invDrag, marked: cells()[0].classList.contains("drag") };
    up(7);
    out.predicted = { from: texts()[0], to: texts()[7], sent: sent.slice(), pending: !!hud.invPending };
    out.landed = hud.invMoveVerdict(0, 0, 7);
    out.afterLand = { from: texts()[0], to: texts()[7], pending: !!hud.invPending };

    // --- a refusal rolls back EXACTLY the two slots it drew -----------------
    fill();
    sent.length = 0;
    down(2);
    up(11);
    const drew = { from: texts()[2], to: texts()[11] };
    const handled = hud.invMoveVerdict(6, 2, 11);
    out.refused = {
      drew,
      handled,
      from: texts()[2],
      to: texts()[11],
      pending: !!hud.invPending,
      toast: document.getElementById("toasts").lastElementChild?.textContent || "",
    };

    // --- a verdict for a DIFFERENT address is not ours ----------------------
    // The positional-payload trap, one level above the encoder: rolling back
    // on somebody else's address corrupts a slot the sim never spoke about.
    fill();
    down(3);
    up(12);
    const mine = { from: texts()[3], to: texts()[12] };
    const alien = hud.invMoveVerdict(6, 3, 13); // same from, wrong to
    out.alien = {
      handled: alien,
      unchanged: texts()[3] === mine.from && texts()[12] === mine.to,
      stillPending: !!hud.invPending,
    };
    // and the real one still works afterwards
    out.alienThenReal = hud.invMoveVerdict(0, 3, 12);

    // --- an authoritative diff outranks the rollback snapshot ---------------
    // The server restated both slots while the move was in flight. A refusal
    // arriving after that must NOT put the stale snapshot back over it.
    fill();
    down(5);
    up(14);
    const t2 = texts();
    t2[5] = "server said this";
    t2[14] = "and this";
    hud.setInventory(t2);
    hud.invMoveVerdict(4, 5, 14);
    out.restated = { from: texts()[5], to: texts()[14] };

    // --- one move in flight at a time ---------------------------------------
    fill();
    sent.length = 0;
    down(1);
    up(8); // opens a pending move
    down(2);
    up(9); // must be refused locally
    out.serialised = { sent: sent.slice(), s2: texts()[2], s9: texts()[9] };
    hud.invMoveVerdict(0, 1, 8);

    // --- the wire refusing the shape draws NOTHING --------------------------
    // Ordering: the send is asked before the prediction is drawn, so a frame
    // that never went out leaves no drawn move to diverge from the server.
    fill();
    sent.length = 0;
    allow = false;
    down(6);
    up(15);
    out.unsendable = {
      sent: sent.length,
      from: texts()[6],
      to: texts()[15],
      pending: !!hud.invPending,
    };
    allow = true;

    // --- a drop on the source slot is a no-op, not a move -------------------
    fill();
    sent.length = 0;
    down(20);
    up(20);
    out.selfDrop = { sent: sent.length, pending: !!hud.invPending, marked: cells()[20].classList.contains("drag") };

    // --- released off a cell, and cancelled by closing the panel ------------
    fill();
    sent.length = 0;
    down(21);
    document.getElementById("inv").dispatchEvent(new PointerEvent("pointerup", { bubbles: true }));
    out.offCell = { drag: hud.invDrag, sent: sent.length };
    down(22);
    hud.toggleInv();
    out.closed = { drag: hud.invDrag, marked: cells()[22].classList.contains("drag") };
    if (!hud.invOpen) hud.toggleInv();

    // --- released outside the PANEL, which is where a real release lands ----
    // The `offCell` case above dispatches on `#inv` itself and so passes
    // straight over this: press a cell, walk the cursor onto the world, let
    // go. What is asserted is not the flag but the NEXT gesture, because a
    // stale drag is only a bug through the move it makes the player's next
    // press send — press 8, and the sim is asked to move 21.
    fill();
    sent.length = 0;
    down(21);
    document.body.dispatchEvent(new PointerEvent("pointerup", { bubbles: true }));
    out.offPanel = { drag: hud.invDrag, sent: sent.length, marked: cells()[21].classList.contains("drag") };
    down(8);
    up(17);
    out.afterOffPanel = sent.slice();
    hud.invMoveVerdict(0, 8, 17);

    // --- the page loses focus mid-drag --------------------------------------
    // Once it is not focused the release will never arrive, so a drag that
    // survives a blur survives forever. `cancelInvDrag`'s docstring named
    // blur as a caller while nothing wired it.
    fill();
    sent.length = 0;
    down(23);
    window.dispatchEvent(new Event("blur"));
    out.blurred = { drag: hud.invDrag, marked: cells()[23].classList.contains("drag") };
    down(9);
    up(18);
    out.afterBlur = sent.slice();
    hud.invMoveVerdict(0, 9, 18);

    // --- a second pointer cannot finish the first pointer's drag ------------
    // Two fingers: the second press is refused by the one-drag guard, which
    // has never had anything to say about the second RELEASE. Without
    // pointer identity that release runs the drop against the first finger's
    // source, and the window-level cancel above cannot help — both pointers
    // are live at once, so nothing is stale.
    fill();
    sent.length = 0;
    const at = (i, type, pointerId) =>
      cells()[i].dispatchEvent(new PointerEvent(type, { bubbles: true, button: 0, pointerId }));
    at(4, "pointerdown", 1);
    at(13, "pointerdown", 2); // refused: one drag at a time
    at(19, "pointerup", 2); // the foreign release
    out.foreignUp = { sent: sent.slice(), drag: hud.invDrag, s4: texts()[4], s19: texts()[19] };
    at(19, "pointerup", 1); // and the drag's OWN pointer still lands it
    out.ownUp = sent.slice();
    hud.invMoveVerdict(0, 4, 19);

    // --- an unarmed panel offers no gesture it cannot perform ---------------
    // With no host holding the move verb every drop toasts "that will not
    // move", which teaches the player the panel is broken rather than that
    // the verb is unbuilt. Restores the sentinel `onInvMove` starts at.
    fill();
    sent.length = 0;
    const armedHost = hud.onInvMove;
    hud.onInvMove = hud.constructor.NO_MOVE_HOST;
    down(24);
    out.unarmed = {
      drag: hud.invDrag,
      marked: cells()[24].classList.contains("drag"),
      began: hud.beginInvDrag(24, null),
    };
    up(25);
    out.afterUnarmed = { sent: sent.length, s24: texts()[24], s25: texts()[25], pending: !!hud.invPending };
    hud.onInvMove = armedHost;
    // ...and assigning a host arms it again, with no separate step
    down(24);
    out.rearmed = { drag: hud.invDrag, marked: cells()[24].classList.contains("drag") };
    hud.cancelInvDrag();

    // --- no drag means no drag pointer, through every door ------------------
    // `invDrag` and `invDragPointer` are one piece of state and every read of
    // the identity is guarded by the slot today — so a cancel that left the
    // pointer set is invisible until some later caller checks only `invDrag`
    // and reads a live identity off a drag that ended. Pinned here rather
    // than left to hold by accident: this exact mutation escaped the eight
    // checks above.
    fill();
    const doors = {};
    down(26);
    document.body.dispatchEvent(new PointerEvent("pointerup", { bubbles: true }));
    doors.release = { drag: hud.invDrag, pointer: hud.invDragPointer };
    down(26);
    window.dispatchEvent(new Event("blur"));
    doors.blur = { drag: hud.invDrag, pointer: hud.invDragPointer };
    down(26);
    hud.toggleInv();
    if (!hud.invOpen) hud.toggleInv();
    doors.escape = { drag: hud.invDrag, pointer: hud.invDragPointer };
    out.doors = doors;

    // --- every refusal reason says something, and says something distinct ---
    const said = [];
    for (let r = 1; r <= reasonMax; r++) {
      fill();
      down(0);
      up(belt + 1);
      hud.invMoveVerdict(r, 0, belt + 1);
      said.push(document.getElementById("toasts").lastElementChild?.textContent || "");
    }
    out.said = said;
    out.foreignKinds = foreignKinds;
    return out;
  },
  [INV_SLOTS, INV_BELT, REFUSE_MAX, selfKind],
);

check(
  move.foreignKinds.length === 0,
  `the panel's own thirty cells sent an address naming another container: ${JSON.stringify(move.foreignKinds)} —` +
    " these cells ARE the self container and every drag through them is self to self",
);

check(
  move.emptyDrag.drag === -1 && move.emptyDrag.marked === false,
  `a pointerdown on an EMPTY slot started a drag (invDrag=${move.emptyDrag.drag}, marked=${move.emptyDrag.marked})` +
    " — there is no such thing as dragging nothing, and every step after it would reason about a move with no item",
);
check(
  move.afterEmptyDrop.sent === 0,
  `dropping a drag that never started sent ${move.afterEmptyDrop.sent} frame(s) — the sim would answer a move of nothing`,
);
check(
  move.picked.drag === 0 && move.picked.marked === true,
  `pointerdown on a filled slot left invDrag=${move.picked.drag} marked=${move.picked.marked}, expected 0/true`,
);
check(
  move.predicted.from === "" && move.predicted.to === "s0",
  `the predicted move drew from=${JSON.stringify(move.predicted.from)} to=${JSON.stringify(move.predicted.to)},` +
    ' expected the source emptied and "s0" landed in the target — the client draws the move it is asking for',
);
check(
  JSON.stringify(move.predicted.sent) === "[[0,7]]",
  `the drop sent ${JSON.stringify(move.predicted.sent)}, expected exactly one [0,7] — the slots as the SIM numbers` +
    " them, belt 0..5 then grid 6..29, not an index into either row",
);
check(move.predicted.pending === true, "a drawn move left nothing pending — its verdict could never be matched");
check(
  move.landed === true && move.afterLand.from === "" && move.afterLand.to === "s0" && move.afterLand.pending === false,
  `a landed verdict did not settle the move (handled=${move.landed}, from=${JSON.stringify(move.afterLand.from)},` +
    ` to=${JSON.stringify(move.afterLand.to)}, pending=${move.afterLand.pending})`,
);
check(
  move.refused.drew.from === "" && move.refused.drew.to === "s2",
  "the refusal case did not draw its move first — it must, or the rollback below is testing nothing",
);
check(
  move.refused.handled === true && move.refused.from === "s2" && move.refused.to === "s11",
  `a refused move rolled back to from=${JSON.stringify(move.refused.from)} to=${JSON.stringify(move.refused.to)},` +
    ' expected "s2"/"s11" — the values the client predicted WITH, not whatever is there now',
);
check(move.refused.pending === false, "a refused move stayed pending — the next drag would be locked out forever");
check(
  move.refused.toast === "it is out of reach",
  `REFUSE_M_REACH told the player ${JSON.stringify(move.refused.toast)} — the panel owes them the reason`,
);
check(
  move.alien.handled === false && move.alien.unchanged === true && move.alien.stillPending === true,
  `a verdict for a DIFFERENT address was applied (handled=${move.alien.handled}, unchanged=${move.alien.unchanged},` +
    ` stillPending=${move.alien.stillPending}) — it would roll back a slot the sim never spoke about, which is` +
    " the right value in the wrong position one level above the encoder",
);
check(move.alienThenReal === true, "after ignoring an alien verdict the panel could no longer settle its own move");
check(
  move.restated.from === "server said this" && move.restated.to === "and this",
  `a refusal put its stale snapshot back over an authoritative diff (from=${JSON.stringify(move.restated.from)},` +
    ` to=${JSON.stringify(move.restated.to)}) — the server's word is newer than the prediction's rollback, and` +
    " restoring the snapshot would put back an item the sim has since moved elsewhere",
);
check(
  JSON.stringify(move.serialised.sent) === "[[1,8]]" && move.serialised.s2 === "s2" && move.serialised.s9 === "s9",
  `a second drag while one was in flight sent ${JSON.stringify(move.serialised.sent)} and drew over slots` +
    " — two concurrent splices on one container is what the reference actually shipped three times in 28 minutes",
);
check(
  move.unsendable.sent === 1 &&
    move.unsendable.from === "s6" &&
    move.unsendable.to === "s15" &&
    move.unsendable.pending === false,
  `a move the wire refused to encode still drew (from=${JSON.stringify(move.unsendable.from)},` +
    ` to=${JSON.stringify(move.unsendable.to)}, pending=${move.unsendable.pending}) — a drawn move with no frame` +
    " behind it IS the divergence, and the sim will never send a verdict to unwind it",
);
check(
  move.selfDrop.sent === 0 && move.selfDrop.pending === false && move.selfDrop.marked === false,
  `dropping a slot on itself sent ${move.selfDrop.sent} frame(s) — it is a no-op, not a move, and not a refusal`,
);
check(
  move.offCell.drag === -1 && move.offCell.sent === 0,
  `releasing off a cell left invDrag=${move.offCell.drag} — the next click anywhere would drop a stale drag into it`,
);
check(
  move.closed.drag === -1 && move.closed.marked === false,
  `closing the panel mid-drag left invDrag=${move.closed.drag} marked=${move.closed.marked} — a drag cannot` +
    " outlive the panel it started in",
);
check(
  move.offPanel.drag === -1 && move.offPanel.sent === 0 && move.offPanel.marked === false,
  `releasing OUTSIDE the panel left invDrag=${move.offPanel.drag} marked=${move.offPanel.marked} — the release` +
    " a player actually makes lands on the world, not on #inv, and a cancel bound to the panel never sees it",
);
check(
  JSON.stringify(move.afterOffPanel) === "[[8,17]]",
  `after a release outside the panel the next drag sent ${JSON.stringify(move.afterOffPanel)}, expected exactly` +
    " one [8,17] — this is the whole cost of a stale drag: the player presses 8 and the sim is asked to move 21," +
    " an unasked-for mutation on a container, which is how the reference's three fixes in 28 minutes read on the wire",
);
check(
  move.blurred.drag === -1 && move.blurred.marked === false,
  `a window blur mid-drag left invDrag=${move.blurred.drag} marked=${move.blurred.marked} — the release will` +
    " never arrive once the page is unfocused, so that drag survives forever",
);
check(
  JSON.stringify(move.afterBlur) === "[[9,18]]",
  `after a blur the next drag sent ${JSON.stringify(move.afterBlur)}, expected exactly one [9,18]`,
);
check(
  JSON.stringify(move.foreignUp.sent) === "[]" &&
    move.foreignUp.drag === 4 &&
    move.foreignUp.s4 === "s4" &&
    move.foreignUp.s19 === "s19",
  `a SECOND pointer's release finished the first pointer's drag (sent=${JSON.stringify(move.foreignUp.sent)},` +
    ` drag=${move.foreignUp.drag}) — the one-drag guard refuses the second press and has nothing to say about` +
    " its release, so without pointer identity that release moves an item nobody touched; and it must not cancel" +
    " the live drag either, which is still under the first finger",
);
check(
  JSON.stringify(move.ownUp) === "[[4,19]]",
  `after ignoring a foreign release the drag's OWN pointer sent ${JSON.stringify(move.ownUp)}, expected [[4,19]]` +
    " — scoping to a pointer must not cost the gesture it is protecting",
);
check(
  move.unarmed.drag === -1 && move.unarmed.marked === false && move.unarmed.began === false,
  `with no host holding the move verb a drag still started (invDrag=${move.unarmed.drag},` +
    ` marked=${move.unarmed.marked}, began=${move.unarmed.began}) — every drop would then toast "that will not` +
    ' move", and an affordance that always refuses teaches the player the panel is broken, not that the verb is unbuilt',
);
check(
  move.afterUnarmed.sent === 0 &&
    move.afterUnarmed.s24 === "s24" &&
    move.afterUnarmed.s25 === "s25" &&
    move.afterUnarmed.pending === false,
  `an unarmed panel still drew or sent a move (sent=${move.afterUnarmed.sent}, s24=${JSON.stringify(move.afterUnarmed.s24)},` +
    ` s25=${JSON.stringify(move.afterUnarmed.s25)}, pending=${move.afterUnarmed.pending})`,
);
check(
  move.rearmed.drag === 24 && move.rearmed.marked === true,
  `assigning a host did not arm the drag (invDrag=${move.rearmed.drag}, marked=${move.rearmed.marked}) — arming` +
    " is identity against Hud.NO_MOVE_HOST precisely so there is no second step to forget when main.js claims the verb",
);
check(
  ["release", "blur", "escape"].every((d) => move.doors[d].drag === -1 && move.doors[d].pointer === null),
  `a cancelled drag kept its pointer id (${JSON.stringify(move.doors)}) — invDrag and invDragPointer are one piece` +
    " of state, and a caller that checks only invDrag would then read a live identity off a drag that ended",
);
check(
  move.said.length === REFUSE_MAX && move.said.every((s) => s.length > 0),
  `a refusal reason told the player nothing: ${JSON.stringify(move.said)} for reasons 1..${REFUSE_MAX}` +
    " — the sim grew a reason the panel has no sentence for",
);
check(
  new Set(move.said).size === REFUSE_MAX,
  `two refusal reasons share a sentence (${JSON.stringify(move.said)}) — the sim keeps them distinct because they` +
    " are different news; 'it is gone' and 'it is out of reach' are not the same thing to a player standing there",
);

// =============================================================================
// L. the move verdict's inbound half — unpacking a positional payload
// =============================================================================
// `web/src/invmove.js` is imported here in node, not in the page: it touches
// no DOM and no wasm, so it is arithmetic, and arithmetic is gated in the
// cheapest layer that can hold it (`ci/pine_shape.mjs`'s precedent — import
// the shipped module and score it headlessly).
//
// This section used to hold a TRIPWIRE. `APPLIED_MOVE` and `STREAM_ERR` were
// both `1 << 31` in client-wasm, so the panel's verdict and "our own server
// sent undecodable bytes" arrived as the same word, and `classifyMoveVerdict`
// split them against the drag in flight. The tripwire was red-means-GOOD-NEWS:
// it asserted the collision still existed, so that the pass which split it
// would be told to delete the workaround rather than update the check. It
// fired on 2026-08-04 and did exactly that job.
//
// What replaces it is the standing invariant, not the scaffolding: bit 31 of
// word 0 is the error sentinel and NO applied flag may share it. `main.js`
// now treats that bit as unambiguously an error, which is only sound while
// that holds, so it is asserted here against the Rust rather than assumed.
const {
  CONT_SELF: JS_CONT_SELF,
  // The other kinds come up here rather than in group M below, because since
  // 2026-08-05 group L needs them too: the verdict unpack no longer rejects a
  // container kind outright, so the checks that it carries one correctly are
  // in L, while M still owns what the PANEL does with the address.
  CONT_BAG: JS_CONT_BAG,
  CONT_BOX: JS_CONT_BOX,
  CONT_MAX: JS_CONT_MAX,
  REFUSE_M_MAX: JS_REFUSE_MAX,
  STREAM_HIGH_BIT,
  APPLIED2_MOVE,
  moveVerdict,
} = await import(pathToFileURL(path.join(root, "web/src/invmove.js")).href);

// The constants it restates from Rust, checked against the Rust. A number
// restated in a gate can drift from the one that ships; so can a number
// restated in the client, and these are restated across a language boundary
// where no compiler is watching.
const coreSrc = fs.readFileSync(path.join(root, "crates/client-wasm/src/core.rs"), "utf8");
check(
  JS_CONT_SELF === Number(invSrc.match(/pub const CONT_SELF: u8 = (\d+);/)?.[1]),
  `invmove.js CONT_SELF (${JS_CONT_SELF}) has drifted from inventory.rs — the panel would accept a container's` +
    " verdict as its own and roll back a slot the server never spoke about",
);
check(
  JS_REFUSE_MAX === REFUSE_MAX,
  `invmove.js REFUSE_M_MAX (${JS_REFUSE_MAX}) has drifted from inventory.rs (${REFUSE_MAX}) — a reason the sim` +
    " grew would be rejected as corruption, or corruption accepted as a reason",
);
// The two flag words, read out of core.rs. `APPLIED2_MOVE` is the flag main.js
// dispatches the verdict on; if it moves within word 1, the client reads the
// wrong bit and every verdict silently stops arriving — an optimistic move
// left drawn forever, which is the container divergence itself.
const streamErrBit = coreSrc.match(/pub const STREAM_ERR: u32 = 1 << (\d+);/)?.[1];
const applied2Move = coreSrc.match(/pub const APPLIED2_MOVE: u32 = 1 << (\d+);/)?.[1];
check(
  streamErrBit === "31" && STREAM_HIGH_BIT === 2 ** 31,
  `invmove.js STREAM_HIGH_BIT (${STREAM_HIGH_BIT}) is not core.rs's STREAM_ERR (1 << ${streamErrBit}) — main.js` +
    " tests that bit as 'the server sent undecodable bytes' and would be reading the wrong one",
);
check(
  applied2Move !== undefined && APPLIED2_MOVE === 1 << Number(applied2Move),
  `invmove.js APPLIED2_MOVE (${APPLIED2_MOVE}) has drifted from core.rs (1 << ${applied2Move}) — the panel would` +
    " dispatch the move verdict on a bit the sim never sets, and every verdict would stop arriving",
);
// The invariant the split bought, asserted so it cannot be given back: NO
// `APPLIED_*` flag in word 0 may sit on bit 31. One that did is what put a
// landed move on the error branch — which logs and returns early, taking the
// inventory diff of the same message with it. Two crates, two constants, one
// value, every wall green (`core.rs`'s own note on the trap).
const word0Bits = [...coreSrc.matchAll(/pub const (APPLIED_[A-Z0-9_]+): u32 = 1 << (\d+);/g)];
check(
  word0Bits.length > 0,
  "no APPLIED_* flags were found in core.rs — this check reads the word 0 flag set by pattern, and a pattern" +
    " that matches nothing asserts nothing",
);
const onBit31 = word0Bits.filter(([, , bit]) => bit === "31").map(([, name]) => name);
check(
  onBit31.length === 0,
  `${onBit31.join(", ")} is on bit 31, which belongs to STREAM_ERR — main.js reads that bit as a decode error` +
    " and returns EARLY, so this flag's message would take every other flag it carried out with it. The applied" +
    " word is full: a new flag goes in word 1 beside APPLIED2_MOVE, never here",
);

// The truth table. `readout` is packed reason << 24 | to << 16 | from kind
// << 8 | from slot (`bridge.rs:904-908`).
const ro = (reason, to, kind, from) => ((reason << 24) | (to << 16) | (kind << 8) | from) >>> 0;

// The unpack, field by field, on one word whose four values are all
// distinguishable. This is the positional check itself: a transposition that
// swapped `from` and `to` — the single shape the reference corrected most
// often — reads here as 7 and 3 the wrong way round, and nothing downstream
// would catch it, because `hud.invMoveVerdict` compares the pair it is GIVEN
// against its pending record and a swapped pair is self-consistently wrong.
const mainSrcL = fs.readFileSync(path.join(root, "web/src/main.js"), "utf8");
const landed = moveVerdict(ro(0, 7, 0, 3));
check(
  landed !== null && landed.reason === 0 && landed.from === 3 && landed.to === 7,
  `a landed verdict did not unpack to reason 0, from 3, to 7: ${JSON.stringify(landed)} — the readout is a` +
    " positional payload and this is the position",
);
const transposed = moveVerdict(ro(0, 3, 0, 7));
check(
  transposed !== null && transposed.from === 7 && transposed.to === 3,
  `the transposed readout did not unpack transposed (${JSON.stringify(transposed)}) — if this reads the same as` +
    " the line above, the unpack is symmetric in from/to and would roll back the wrong two cells",
);
for (let reason = 1; reason <= REFUSE_MAX; reason++) {
  const v = moveVerdict(ro(reason, 7, 0, 3));
  check(
    v !== null && v.reason === reason && v.from === 3 && v.to === 7,
    `refusal reason ${reason} did not reach the panel (${JSON.stringify(v)}) — a refusal the panel never hears` +
      " leaves the optimistic move drawn forever, which is the container divergence itself",
  );
}
// The shapes the sim cannot produce for a move this panel sent. Each is a
// rejection, and each rejection is a cell NOT unwound on a word that was not
// this panel's verdict.
// A CONT_BAG verdict is now ACCEPTED and carries its kind up, because the
// panel draws a bag (group O). Until 2026-08-05 it was rejected here, and
// correctly: no bag was drawn, so honouring one would have unwound a self
// cell the server never spoke about. What replaced that rejection is
// `hud.invMoveVerdict` matching the kind against the move actually in flight
// — the same defence, one layer up, where it can tell WHICH container.
const bagVerdict = moveVerdict(ro(0, 7, JS_CONT_BAG, 3));
check(
  bagVerdict !== null && bagVerdict.fromKind === JS_CONT_BAG && bagVerdict.from === 3,
  `a CONT_BAG verdict unpacked to ${JSON.stringify(bagVerdict)} — the FROM kind is the field that says which` +
    " container a verdict is about, and dropping it is how a bag refusal comes to unwind a self cell",
);
check(
  moveVerdict(ro(0, 7, JS_CONT_MAX + 1, 3)) === null,
  `a verdict naming container kind ${JS_CONT_MAX + 1}, past CONT_MAX, was accepted — the sim cannot produce it,` +
    " so the word was not a verdict at all",
);
check(
  moveVerdict(ro(0, 7, JS_CONT_BOX, BOX_SLOTS)) === null,
  `a verdict naming BOX slot ${BOX_SLOTS} was accepted — a box has ${BOX_SLOTS} slots inside a ${INV_SLOTS}-slot` +
    " view, so the sim cannot have moved out of that one and the word is not a verdict",
);
check(
  moveVerdict(ro(REFUSE_MAX + 1, 7, 0, 3)) === null,
  `a reason past REFUSE_M_MAX (${REFUSE_MAX}) was accepted as a verdict — the sim cannot produce it, so the` +
    " word was not a verdict, and the panel would toast a refusal string for a reason that does not exist",
);
// The `from === to` rejection that used to live here is GONE, and its removal
// is a fix rather than a relaxation — see `moveVerdict`'s own note. A landed
// self-3-to-box-3 is a real move whose readout is indistinguishable from the
// degenerate word (the TO kind is not in it), so rejecting the shape left
// `invPending` uncleared and refused every later drag with "still moving
// that", permanently. At slot 0 the landed word is all-zero.
//
// What the rejection was PROTECTING is asserted instead, at the two places
// that actually hold it: the readout is only read under APPLIED2_MOVE (a
// bridge with no core sets no flag), and the panel matches the full address.
check(
  moveVerdict(ro(0, 3, JS_CONT_SELF, 3)) !== null && moveVerdict(0) !== null,
  "moveVerdict still rejects a readout addressed from a slot to its own number — that is a REAL landed move" +
    " once a second container exists (self 3 to box 3), the TO kind is not in the word to tell them apart, and" +
    " dropping it leaves the pending move uncleared and the panel stuck refusing every later drag",
);
check(
  /applied2 & APPLIED2_MOVE/.test(mainSrcL) &&
    mainSrcL.indexOf("applied2 & APPLIED2_MOVE") < mainSrcL.indexOf("client_move_readout()"),
  "main.js reads client_move_readout() outside the APPLIED2_MOVE check — that flag is now the only thing" +
    " keeping a bridge-with-no-core zero readout away from the unpack, since the address shape that used to" +
    " reject it is a legal landed move",
);

// =============================================================================
// M. addresses, not slot numbers — the aliasing that blocks a second panel
// =============================================================================
// The newest judge report's gap 1 ("there is nowhere to put anything") asks
// for a second panel bound to the same move verb. The thing in the way on this
// side is not netcode: bag slot 3 and self slot 3 are the same integer, and
// until this pass the panel keyed its drag, its pending record and its verdict
// match on that integer alone. A verdict about a container would then have
// resolved — and rolled back — a cell of the player's own inventory.
//
// So every address is now a (kind, slot) pair, and this group asserts the two
// places that can still alias: the verdict matcher, and the rollback.
// Every kind the client names, against the number the sim gives it. Read
// through `rustConst` so a kind written as an alias is resolved rather than
// string-matched — see its comment for the two ways that went wrong here.
for (const [name, js] of [
  ["CONT_BAG", JS_CONT_BAG],
  ["CONT_BOX", JS_CONT_BOX],
]) {
  const rs = rustConst(name);
  check(
    Number.isInteger(rs) && js === rs,
    `invmove.js ${name} (${js}) has drifted from inventory.rs (${rs}) — the panel would form addresses naming a` +
      " container the sim numbers differently, and every one of them would be answered about somewhere else",
  );
}
// The ceiling the wire range-checks kinds against (`encode_action_move`), and
// it must EQUAL the sim's — neither direction is survivable. Too high and the
// client encodes a kind the server decodes as `Malformed` and answers by
// ending the session, which is exactly how this verb failed in the reference.
// Too low — which is how this gate went red on 2026-08-04, after the systems
// lane added `CONT_BOX` in 4d7a926 — and the client silently refuses, on its
// own side, addresses the sim would have honoured: the box panel this lane is
// heading for simply never sends.
const RS_CONT_MAX = rustConst("CONT_MAX");
check(
  Number.isInteger(RS_CONT_MAX),
  "could not resolve inventory.rs's CONT_MAX to a number — the ceiling below would then be checked against" +
    " nothing, which is the gate-that-matches-nothing class",
);
check(
  JS_CONT_MAX === RS_CONT_MAX,
  `invmove.js CONT_MAX (${JS_CONT_MAX}) is not inventory.rs's CONT_MAX (${RS_CONT_MAX}) — see above; a kind added` +
    " on either side of the boundary must move both, and this is the commit that says so",
);

// The two slot COUNTS, against the sim's own. `limits.rs` and not
// `inventory.rs` — that is where they are declared and where `slots_in`
// imports them from — so this needs its own reader rather than `rustConst`.
//
// Both directions are load-bearing and they fail differently. Too high and
// the panel draws cells past the container's end, and a drop into one is a
// slot the sim answers REFUSE_M_SLOT while the client has already drawn the
// move. Too low and the tail of a real container is unreachable: items visible
// on the wire that no gesture can address. `BOX_SLOTS` is the sharp one — a box
// is twelve slots inside a THIRTY-slot view (`client_cont_ptr`), so the tail is
// zero-filled rather than absent, and every layer that forgets the distinction
// still reads a valid-looking empty stack.
const limitsSrc = fs.readFileSync(
  path.join(root, "crates/sim-core/src/limits.rs"),
  "utf8",
);
for (const [name, js] of [
  ["INV_SLOTS", INV_SLOTS],
  ["BOX_SLOTS", BOX_SLOTS],
]) {
  // Anchored at the line start, same reason `rustConst` is: a doc comment
  // quoting a declaration must not be able to stand in for it.
  const m = limitsSrc.match(new RegExp(`^pub const ${name}: \\w+ = (\\d+);`, "m"));
  const rs = m ? Number(m[1]) : null;
  check(
    Number.isInteger(rs),
    `could not read ${name} out of limits.rs — the container widths below would then be checked against` +
      " nothing, which is the gate-that-matches-nothing class",
  );
  check(
    js === rs,
    `invmove.js ${name} (${js}) has drifted from limits.rs (${rs}) — the panel would draw a container of one` +
      " width against a wire carrying another, and every slot past the shorter of the two is either a cell the" +
      " sim refuses a drop into or an item no gesture can reach",
  );
}
// And the width function that dispatches on kind, against `inventory.rs`'s
// `slots_in`. A mirrored branch is worth one check: getting it backwards
// gives a box thirty cells and the player's own inventory twelve.
const { slotsIn: JS_SLOTS_IN } = await import(
  pathToFileURL(path.join(root, "web/src/invmove.js")).href
);
check(
  JS_SLOTS_IN(JS_CONT_BOX) === BOX_SLOTS &&
    JS_SLOTS_IN(JS_CONT_BAG) === INV_SLOTS &&
    JS_SLOTS_IN(JS_CONT_SELF) === INV_SLOTS,
  `invmove.js slotsIn gives self/bag/box widths ` +
    `${JSON.stringify([JS_SLOTS_IN(JS_CONT_SELF), JS_SLOTS_IN(JS_CONT_BAG), JS_SLOTS_IN(JS_CONT_BOX)])},` +
    ` but inventory.rs's slots_in is BOX_SLOTS (${BOX_SLOTS}) for a box and INV_SLOTS (${INV_SLOTS}) for` +
    " everything else — the branch is mirrored backwards",
);

const addr = await page.evaluate(
  ([n, self, bag]) => {
    const { hud } = globalThis.__ui;
    const out = {};
    const sent = [];
    hud.onInvMove = (fk, f, tk, t) => {
      sent.push([fk, f, tk, t]);
      return true;
    };
    const fill = () => {
      const t = [];
      for (let s = 0; s < n; s++) t.push(`s${s}`);
      hud.setInventory(t);
    };
    const cells = () => [
      ...document.querySelectorAll("#invbelt .invcell"),
      ...document.querySelectorAll("#invgrid .invcell"),
    ];
    const texts = () => cells().map((c) => c.querySelector("span").textContent);

    // --- the panel names the containers it draws, and draws only one -------
    out.containers = hud.invContainers.slice();
    out.drawsSelf = hud.drawsContainer(self);
    out.drawsBag = hud.drawsContainer(bag);

    // --- a drag cannot start in a container the panel does not draw --------
    fill();
    out.bagBegan = hud.beginInvDrag(4, 1, bag);
    out.bagDrag = { drag: hud.invDrag, kind: hud.invDragKind };
    hud.cancelInvDrag();

    // --- the host is handed the ADDRESSES, in that order -------------------
    // Positional, and every value distinct from every other so a transposed
    // pair cannot pass: kind 0 twice is the only shape this host carries, so
    // the slots carry the distinctness (5 and 19).
    fill();
    sent.length = 0;
    hud.beginInvDrag(5, 1, self);
    out.sentDrop = hud.dropInvDrag(19, 1, self);
    out.sent = sent.slice();
    out.pending = hud.invPending && { ...hud.invPending };

    // --- a verdict from a DIFFERENT container is not this move's ----------
    // Same two slot numbers, same reason, other container. Before addresses
    // this was indistinguishable from the real verdict.
    out.alienFrom = hud.invMoveVerdict(0, 5, 19, bag, self);
    out.aliveAfterFrom = !!hud.invPending;
    out.alienTo = hud.invMoveVerdict(0, 5, 19, self, bag);
    out.aliveAfterTo = !!hud.invPending;
    out.mine = hud.invMoveVerdict(0, 5, 19, self, self);
    out.aliveAfterMine = !!hud.invPending;
    out.drawnAfterLand = { from: texts()[5], to: texts()[19] };

    // --- a rollback writes only cells this panel drew ----------------------
    // A move with one end in another container drew ONE cell. Restoring the
    // other end would put a foreign container's item into the self grid at a
    // slot the server never spoke about — the aliasing, by the back door.
    // The pending record is set directly here because the panel that would
    // draw the other end does not exist yet: this is the matcher under test,
    // not the drag.
    fill();
    hud.invPending = {
      fromKind: bag, from: 2, toKind: self, to: 8,
      wasFrom: "BAGITEM", wasTo: "s8", restated: false,
    };
    hud.setInvText(8, "BAGITEM"); // what the prediction drew, self end only
    out.rolled = hud.invMoveVerdict(6, 2, 8, bag, self);
    out.afterRoll = { self2: texts()[2], self8: texts()[8] };

    // --- a self restatement is not a bag end's restatement -----------------
    // `setInventory` is the OWN inventory diff. A write to self slot 2 while
    // a move out of BAG slot 2 is in flight is a different slot entirely,
    // and counting it would give up a rollback the panel still owes.
    fill();
    hud.invPending = {
      fromKind: bag, from: 2, toKind: self, to: 8,
      wasFrom: "BAGITEM", wasTo: "s8", restated: false,
    };
    hud.setInventory(Object.assign([], texts(), { 2: "CHANGED" }));
    out.restatedByForeign = hud.invPending.restated;
    hud.setInventory(Object.assign([], texts(), { 8: "ALSOCHANGED" }));
    out.restatedByOwn = hud.invPending.restated;
    hud.invPending = null;
    return out;
  },
  [INV_SLOTS, JS_CONT_SELF, JS_CONT_BAG],
);

check(
  addr.containers.length === 1 && addr.containers[0] === JS_CONT_SELF,
  `the panel claims to draw ${JSON.stringify(addr.containers)} — it has cells and a text array for exactly one` +
    " container, so listing a second here would promise a draw it cannot perform. Gap 1's second panel adds the" +
    " cells and the contents source in the same change that adds the entry",
);
check(
  addr.drawsSelf === true && addr.drawsBag === false,
  `drawsContainer disagrees with that list (self=${addr.drawsSelf}, bag=${addr.drawsBag})`,
);
check(
  addr.bagBegan === false && addr.bagDrag.drag === -1 && addr.bagDrag.kind === -1,
  `a drag started in a container the panel does not draw (${JSON.stringify(addr.bagDrag)}) — it would mark a cell` +
    " of the self grid and then send a move out of somewhere else",
);
check(
  addr.sentDrop === true &&
    addr.sent.length === 1 &&
    addr.sent[0][0] === JS_CONT_SELF &&
    addr.sent[0][1] === 5 &&
    addr.sent[0][2] === JS_CONT_SELF &&
    addr.sent[0][3] === 19,
  `the host was handed ${JSON.stringify(addr.sent)}, not [[self, 5, self, 19]] — this is the panel's own` +
    " positional payload, and main.js pours these four straight into client_action_move's four address arguments",
);
check(
  addr.pending &&
    addr.pending.fromKind === JS_CONT_SELF &&
    addr.pending.toKind === JS_CONT_SELF &&
    addr.pending.from === 5 &&
    addr.pending.to === 19,
  `the pending record is ${JSON.stringify(addr.pending)} — a record without the kinds is what made a container's` +
    " verdict indistinguishable from this one's",
);
check(
  addr.alienFrom === false && addr.aliveAfterFrom === true,
  "a verdict addressed out of ANOTHER container resolved this panel's move — the two slot numbers match because" +
    " containers share numbering, which is exactly why the kind has to be matched too",
);
check(
  addr.alienTo === false && addr.aliveAfterTo === true,
  "a verdict addressed INTO another container resolved this panel's move — same aliasing, other end",
);
check(
  addr.mine === true && addr.aliveAfterMine === false,
  "the panel's own verdict did not resolve its own move — the kind match must not reject the one address it sent",
);
check(
  addr.drawnAfterLand.from === "" && addr.drawnAfterLand.to === "s5",
  `a landed move left the grid at ${JSON.stringify(addr.drawnAfterLand)} — the three refused verdicts above must` +
    " not have unwound anything either",
);
check(
  addr.rolled === true && addr.afterRoll.self8 === "s8",
  `a refusal did not put the self end back (slot 8 = ${JSON.stringify(addr.afterRoll.self8)})`,
);
check(
  addr.afterRoll.self2 === "s2",
  `the rollback wrote the OTHER container's item into self slot 2 (${JSON.stringify(addr.afterRoll.self2)}) — the` +
    " panel does not draw that container, so that slot number named a cell the server never spoke about",
);
check(
  addr.restatedByForeign === false,
  "a write to self slot 2 counted as the restatement of a move out of BAG slot 2 — the panel would then decline" +
    " a rollback it still owes, and the optimistic draw stands forever, which is the container divergence itself",
);
check(
  addr.restatedByOwn === true,
  "a write to the move's own self end did NOT count as a restatement — the server's word is newer than the" +
    " snapshot and rolling back over it would put back an item the sim has since moved",
);

// =============================================================================
// N. the move verb's OUTBOUND half — six positional u32s into the bridge
// =============================================================================
// The 2026-08-05 judge report's ranked fix 4, verbatim: "the third hop of
// report 03's ranked fix 1 — `main.js`'s marshalling of (fromKind, from,
// toKind, to) into the wasm call — is covered only by `browser_smoke`, which
// is off this run. Closing it needs the marshalling extracted into something
// node-importable."
//
// Why it is worth a section of its own. `client_action_move` takes SIX `u32`s
// and every wall in this repo is blind to swapping two of them: the encoder is
// untouched so `test_protocol_golden` is green, the action queue is not in
// `state_hash` so `test_replay` is green, and all six are the same type so
// clippy is green. That is precisely CLAUDE.md's positional-payload trap —
// ~27 of Oxide's shipped corrections were the right value in the wrong
// position, four hooks corrected more than once, and their per-method
// `MSILHash`, the exact analogue of our protocol golden, caught none of them.
// It is also the verb the same trap list names as the most bug-prone thing in
// the reference, whose failures all landed as *the server disconnecting the
// client*, because an action frame the server cannot decode ends the reader
// task (`bridge.rs`'s own doc says so).
//
// The shape of the fix is the point: the order is stated ONCE, as names, in
// `invmove.js`'s `MOVE_ARG_ORDER`, and the first check below reads the same
// names out of `bridge.rs`. So this does not restate the client's opinion of
// the order and compare it to itself — the gate that matches nothing, which
// this file has already shipped twice. A systems-lane reorder of that ABI
// lands red here on the commit that makes it, rather than on the frame the
// server declines.
const { MOVE_ARG_ORDER, moveArgs } = await import(
  pathToFileURL(path.join(root, "web/src/invmove.js")).href
);
const bridgeSrc = fs.readFileSync(
  path.join(root, "crates/client-wasm/src/bridge.rs"),
  "utf8",
);
// The parameter list as Rust declares it, in order. Anchored at the `fn` so a
// doc comment quoting the call cannot stand in for the declaration — the same
// hole the judge's ranked fix 1 found in `rustConst` above.
const rsParams = bridgeSrc
  .match(/^pub extern "C" fn client_action_move\(([\s\S]*?)\)\s*->/m)?.[1]
  ?.split(",")
  .map((p) => p.trim().split(":")[0].trim())
  .filter(Boolean);
check(
  Array.isArray(rsParams) && rsParams.length === 6,
  `could not read client_action_move's parameter list out of bridge.rs (got ${JSON.stringify(rsParams)}) —` +
    " every check below would then compare the client's order against nothing, which is the" +
    " gate-that-matches-nothing class this section exists to avoid",
);
check(
  JSON.stringify(MOVE_ARG_ORDER) === JSON.stringify(rsParams),
  `invmove.js MOVE_ARG_ORDER ${JSON.stringify(MOVE_ARG_ORDER)} is not client_action_move's parameter list` +
    ` ${JSON.stringify(rsParams)} — the client would spread six u32s into the wrong slots of the same-typed` +
    " signature, the encoder would accept them, and the server would answer the forged frame by ending the session",
);

// Slot 3 holds item 9 ×4, slot 7 holds item 5 ×11. Four distinct numbers, so
// the count cannot be confused with the item id beside it (stride), with the
// destination's count (wrong end), or with either slot number.
const PROBE_INV = new Uint16Array(INV_SLOTS * 2);
PROBE_INV[3 * 2] = 9;
PROBE_INV[3 * 2 + 1] = 4;
PROBE_INV[7 * 2] = 5;
PROBE_INV[7 * 2 + 1] = 11;
// The OPEN container's own view, same layout, deliberately different numbers:
// slot 2 holds item 13 x6. If a count for a container-sourced drag were read
// out of `inv` instead of `cont` it would come back 4 or 11, never 6.
const PROBE_CONT = new Uint16Array(INV_SLOTS * 2);
PROBE_CONT[2 * 2] = 13;
PROBE_CONT[2 * 2 + 1] = 6;
// A real handle, and nothing else in this section shares its value. Until
// 2026-08-05 `bag` was hardcoded 0 and so were both kinds, which is why the
// judge's mutant M12 — swap `bag` and `from_kind` in the named object — was
// GREEN: three fields, one value, no probe could separate them. A handle that
// is neither 0, nor a kind, nor a slot, nor a count is what makes the
// transposition visible, and `client_cont_handle` is a packed `box_key` or a
// bag id, so a large number is the honest shape rather than a convenient one.
const PROBE_BAG = 0x1234abcd;
const probe = moveArgs(0, JS_CONT_SELF, 3, JS_CONT_SELF, 7, PROBE_INV, PROBE_CONT);
check(
  Array.isArray(probe) && probe.length === 6,
  `moveArgs did not return six arguments for a legal self-to-self drag (${JSON.stringify(probe)})`,
);
// Positions are taken BY NAME out of the Rust-checked order, never as literal
// indices: a check that hardcoded `probe[2]` would have to be edited in step
// with any real reorder, and would then agree with whatever it was edited to.
const at = (name) => probe?.[MOVE_ARG_ORDER.indexOf(name)];
check(
  at("from_slot") === 3 && at("to_slot") === 7,
  `moveArgs put the drag's ends at ${JSON.stringify([at("from_slot"), at("to_slot")])} rather than [3, 7] —` +
    " the two slot numbers are transposed, so the sim moves the destination's contents onto the source",
);
check(
  at("count") === 4,
  `moveArgs sent count ${at("count")} for a drag out of slot 3, which holds item 9 ×4 — 9 means it read the` +
    " ITEM ID at the even index of the (item, count) pair, 11 means it read the DESTINATION's count, and" +
    " either sends the server a quantity the panel never drew, which is the quantize-both-sides law broken",
);
check(
  at("bag") === 0,
  `moveArgs sent bag handle ${at("bag")} for a self move — the sim addresses containers by this field and 0` +
    " is the only value that means the player's own inventory",
);
check(
  at("from_kind") === 0 && at("to_kind") === 0,
  `moveArgs sent kinds ${JSON.stringify([at("from_kind"), at("to_kind")])} for a self-to-self drag`,
);
// A handle passed for a self-to-self move is NORMALIZED to 0, not carried.
// `world.rs` never reads the field for such a move and `encode_action_move`
// does not range-check it, so a stray handle would cross the wire and enter
// the WAL as a value nothing validates.
check(
  moveArgs(PROBE_BAG, JS_CONT_SELF, 3, JS_CONT_SELF, 7, PROBE_INV, PROBE_CONT)?.[
    MOVE_ARG_ORDER.indexOf("bag")
  ] === 0,
  "moveArgs carried a container handle on a self-to-self move — the sim ignores that field for such a move and" +
    " the encoder does not range-check it, so a wrong value there is one no layer would ever report",
);

// --- the three fields that used to be indistinguishable ---------------------
//
// This is the judge's ranked fix 2, closed in the pass that made it closeable.
// `bag`, `from_kind` and `to_kind` were 0 on every call this client could
// legally make, so mutant M12 (swap `bag` and `from_kind`) was green — a live
// wrong-position bug the moment a second container opened. It opens in this
// commit, so the probe that separates them lands in it.
const cprobe = moveArgs(PROBE_BAG, JS_CONT_SELF, 3, JS_CONT_BAG, 7, PROBE_INV, PROBE_CONT);
const cat = (name) => cprobe?.[MOVE_ARG_ORDER.indexOf(name)];
check(
  Array.isArray(cprobe) && cprobe.length === 6,
  `moveArgs did not marshal a legal self-to-bag drag (${JSON.stringify(cprobe)}) — the panel draws a second` +
    " container now, so this is the shape the verb exists for",
);
check(
  cat("bag") === PROBE_BAG,
  `moveArgs sent bag handle ${cat("bag")} rather than ${PROBE_BAG} for a drag into an open container — 0 means` +
    " the handle was dropped and the sim would address whatever container it indexes at 0, and 1 means the FROM" +
    " KIND landed in the handle's position, which is the transposition mutant M12 proved this gate could not see",
);
check(
  cat("from_kind") === JS_CONT_SELF && cat("to_kind") === JS_CONT_BAG,
  `moveArgs sent kinds ${JSON.stringify([cat("from_kind"), cat("to_kind")])} rather than` +
    ` [${JS_CONT_SELF}, ${JS_CONT_BAG}] — the two ends' containers are transposed, so the sim takes the item` +
    " out of the container the player was dragging INTO",
);
check(
  cat("count") === 4,
  `moveArgs sent count ${cat("count")} for a drag out of SELF slot 3 (item 9 x4) into a container — 6 means it` +
    " read the count out of the container's view rather than the source's, which is the quantize-both-sides law" +
    " broken by reading the wrong array rather than the wrong index",
);
// And the mirror: a drag OUT of the container reads the container's view.
const bprobe = moveArgs(PROBE_BAG, JS_CONT_BAG, 2, JS_CONT_SELF, 7, PROBE_INV, PROBE_CONT);
const bat = (name) => bprobe?.[MOVE_ARG_ORDER.indexOf(name)];
check(
  bat("count") === 6,
  `moveArgs sent count ${bat("count")} for a drag out of CONTAINER slot 2 (item 13 x6) — 4 or 11 means it read` +
    " the player's OWN inventory for a stack that is not in it, which is the label bug one array over: the same" +
    " shape, a different container, and a quantity the panel never drew",
);
check(
  bat("from_kind") === JS_CONT_BAG && bat("to_kind") === JS_CONT_SELF && bat("bag") === PROBE_BAG,
  `moveArgs marshalled a drag out of a container as ${JSON.stringify(bprobe)} — the ends or the handle are` +
    " transposed",
);

// The refusals, each with the reason it is not merely defensive.
check(
  moveArgs(0, JS_CONT_SELF, 3, JS_CONT_BAG, 7, PROBE_INV, PROBE_CONT) === null,
  "moveArgs marshalled a drag into a container with handle 0 — and 0 is not a safe placeholder: deploy.rs's" +
    " box_index has no zero guard and box_key(0,0,0) == 0, so a box at cell (0,0) level 0 is genuinely" +
    " addressed by it, and this frame would move items in a stranger's container rather than be refused",
);
check(
  moveArgs(PROBE_BAG, JS_CONT_BAG, 2, JS_CONT_BOX, 7, PROBE_INV, PROBE_CONT) === null,
  "moveArgs marshalled a drag between TWO different ground containers — Command::Move carries one handle" +
    " (world.rs), so the sim answers REFUSE_M_NO_CONTAINER and the panel has already drawn the move",
);
check(
  moveArgs(PROBE_BAG, JS_CONT_SELF, 3, JS_CONT_BOX, BOX_SLOTS, PROBE_INV, PROBE_CONT) === null,
  `moveArgs marshalled a drag into BOX slot ${BOX_SLOTS}, one past a box's own width — encode_action_move` +
    " bounds both ends against a flat INV_SLOTS and would carry it (loose on purpose: a tight check there makes" +
    " an over-wide slot a FRAME error, and a frame error ends the session), so the client owes this one",
);
check(
  moveArgs(PROBE_BAG, JS_CONT_SELF, 3, JS_CONT_BOX, BOX_SLOTS - 1, PROBE_INV, PROBE_CONT) !== null,
  `moveArgs refused a drag into BOX slot ${BOX_SLOTS - 1}, the LAST slot a box has — the bound is off by one` +
    " the other way, and the tail of every box is then unreachable",
);
check(
  moveArgs(PROBE_BAG, JS_CONT_BAG, 2, JS_CONT_BAG, 2, PROBE_INV, PROBE_CONT) === null,
  "moveArgs marshalled a drag onto its own ADDRESS — same kind and same slot is the no-op the sim refuses",
);
check(
  moveArgs(PROBE_BAG, JS_CONT_BAG, 2, JS_CONT_BAG, 5, PROBE_INV, PROBE_CONT) !== null,
  "moveArgs refused a drag WITHIN one open container — both ends non-self is legal when the kind is the same" +
    " (world.rs needs one handle for it), and rearranging an open box is exactly that shape",
);
check(
  moveArgs(0, JS_CONT_SELF, 5, JS_CONT_SELF, 7, PROBE_INV, PROBE_CONT) === null,
  "moveArgs marshalled a drag out of an EMPTY slot (5 holds nothing) — the sim refuses it and the panel has" +
    " already drawn it, which is the container divergence the reference kept shipping as a disconnect",
);
check(
  moveArgs(0, JS_CONT_SELF, INV_SLOTS, JS_CONT_SELF, 7, PROBE_INV, PROBE_CONT) === null,
  `moveArgs marshalled a drag out of slot ${INV_SLOTS}, one past the view — the count reads \`undefined\` there,` +
    " and `undefined <= 0` is FALSE, so the old inline test let it through to wasm and was correct only because" +
    " three layers below it a coerced 0 failed encode_action_move's range check",
);
check(
  moveArgs(0, JS_CONT_SELF, 3, JS_CONT_SELF, 7, undefined, undefined) === null,
  "moveArgs marshalled a drag with no inventory view at all — `views.inv` is null until the first refresh",
);
check(
  moveArgs(PROBE_BAG, JS_CONT_BAG, 2, JS_CONT_SELF, 7, PROBE_INV, undefined) === null,
  "moveArgs marshalled a drag out of a container with no container view — `views.cont` is where the count for" +
    " that end comes from, and without it the count is `undefined` rather than a quantity the panel drew",
);
check(
  moveArgs(PROBE_BAG, JS_CONT_SELF, 3, JS_CONT_MAX + 1, 7, PROBE_INV, PROBE_CONT) === null,
  `moveArgs marshalled a drag into container kind ${JS_CONT_MAX + 1}, past CONT_MAX — encode_action_move` +
    " range-checks it too, but a refusal that has to cross the wall to happen is one the panel has already drawn",
);

// And the call site, because an extraction nothing is held to is a suggestion.
// `main.js` is renderer-tier and cannot be imported here (it boots three.js),
// so this is a source assertion — the same standing this file already gives
// the HUD ids and the Rust constants it reads as text.
const mainSrc = fs.readFileSync(path.join(root, "web/src/main.js"), "utf8");
const hostBody = mainSrc.match(/hud\.onInvMove = \([^)]*\) => \{([\s\S]*?)\n  \};/)?.[1];
check(
  typeof hostBody === "string" && hostBody.includes("moveArgs("),
  "main.js's move host does not go through moveArgs — the marshalling is back inline, where the only gate that" +
    " can see it is browser_smoke",
);
const spreadBinding = hostBody?.match(/const (\w+) = moveArgs\(/)?.[1];
check(
  typeof spreadBinding === "string" &&
    new RegExp(`client_action_move\\(\\.\\.\\.${spreadBinding}\\)`).test(hostBody),
  "main.js does not spread moveArgs's own result into client_action_move — a longhand argument list is exactly" +
    " the six-u32 transposition this section was written to make unrepresentable",
);
check(
  (mainSrc.match(/client_action_move\(/g) || []).length === 1,
  "main.js calls client_action_move from more than one place — the second call site is unmarshalled and ungated",
);
check(
  // The string guard is folded in rather than left to check ORDER. This is
  // the judge's ranked fix 3: `hostBody` is a source regex, a reformat to a
  // named function or a re-indent makes it `undefined`, and `!undefined` is
  // TRUE — so this check alone would read green on nothing. It was defended
  // only by the `typeof hostBody === "string"` check above running first,
  // which is a property of the order these lines happen to be in.
  typeof hostBody === "string" && !hostBody.includes("views.inv["),
  "main.js's move host still indexes views.inv directly (or the host body no longer parses out of main.js at" +
    " all) — the (item, count) stride is arithmetic and belongs in invmove.js where node can probe it, not in" +
    " a file only a 19-minute renderer gate can reach",
);
check(
  typeof hostBody === "string" && !hostBody.includes("views.cont["),
  "main.js's move host indexes views.cont directly — the container's (item, count) stride is the same" +
    " arithmetic one array over, and it belongs in the same place for the same reason",
);

// =============================================================================
// O. the open container — a second panel, and the divergence it can cause
// =============================================================================
// The systems half landed at wire v19: `EventMsg::ContSync` carries (kind,
// handle, slots) to the opener alone, through `client_cont_kind` /
// `client_cont_handle` / `client_cont_ptr`. This is the panel that draws it.
//
// The reason this group is long is CLAUDE.md's trap list. The item-move verb
// is the most bug-prone thing in the reference — three Oxide fixes in 28
// minutes on one 2019 day, all splice-point moves on move/stack/loot, all
// landing as *the server disconnecting the client* — and the bug is always
// container state diverging, never arithmetic. A second container is exactly
// the state that can diverge, and this client has one specific way to do it:
//
//   `client_move_readout` packs `reason | to slot | from kind | from slot`
//   and `core.rs`'s `last_move` carries NO handle and NO sequence number.
//
// So once the open container changes, nothing in an arriving verdict
// distinguishes "the move you sent, answered" from "a move about the
// container you were looking at a moment ago". `hud.abandonContainerMove` is
// the answer and the checks below are what hold it to that.
const cont = await page.evaluate(
  ([self, bag, box, slots, boxSlots]) => {
    const { hud } = globalThis.__ui;
    const out = {};
    const sent = [];
    hud.onInvMove = (fk, f, tk, t) => {
      sent.push([fk, f, tk, t]);
      return true;
    };
    const shown = () =>
      [...document.querySelectorAll("#contgrid .invcell")].filter(
        (c) => c.style.display !== "none",
      ).length;
    const contText = (i) =>
      document.querySelectorAll("#contgrid .invcell")[i].querySelector("span").textContent;
    const selfText = (i) =>
      [
        ...document.querySelectorAll("#invbelt .invcell"),
        ...document.querySelectorAll("#invgrid .invcell"),
      ][i].querySelector("span").textContent;
    const fillSelf = () => {
      const t = [];
      for (let s = 0; s < slots; s++) t.push(`s${s}`);
      hud.setInventory(t);
    };
    const bagTexts = () => {
      const t = [];
      for (let s = 0; s < slots; s++) t.push(`b${s}`);
      return t;
    };
    const reset = () => {
      hud.closeContainer();
      hud.invPending = null;
      hud.cancelInvDrag();
      fillSelf();
    };

    // --- nothing is open until the server says so -------------------------
    out.startKind = hud.contKind;
    out.startDraws = hud.drawsContainer(bag);
    out.startCells = shown();

    // --- open a bag: thirty cells, both containers drawable ---------------
    if (!hud.invOpen) hud.toggleInv();
    fillSelf();
    out.opened = hud.openContainer(bag, 0x1234abcd, bagTexts());
    out.kind = hud.contKind;
    out.handle = hud.contHandle;
    out.containers = hud.invContainers.slice();
    out.bagCells = shown();
    out.bagCell7 = contText(7);
    out.panelShown = getComputedStyle(document.getElementById("cont")).display;

    // --- the panel rides with the inventory -------------------------------
    hud.toggleInv();
    out.hiddenWithInv = getComputedStyle(document.getElementById("cont")).display;
    hud.toggleInv();
    out.backWithInv = getComputedStyle(document.getElementById("cont")).display;

    // --- a BOX is narrower than the view that carries it ------------------
    out.boxOpened = hud.openContainer(box, 0x55d450, bagTexts());
    out.boxCells = shown();
    // The cells past a box's width are HIDDEN, not blank: an empty cell
    // invites a drop the sim answers REFUSE_M_SLOT.
    out.boxTailHidden =
      document.querySelectorAll("#contgrid .invcell")[boxSlots].style.display === "none";
    // And no drop may land past it, even by direct call.
    hud.beginInvDrag(0, 1, self);
    out.dropPastBox = hud.dropInvDrag(boxSlots, 1, box);
    hud.cancelInvDrag();
    hud.invPending = null;

    // --- a drag across the two containers ---------------------------------
    reset();
    hud.openContainer(bag, 0x1234abcd, bagTexts());
    sent.length = 0;
    out.beganSelf = hud.beginInvDrag(5, 1, self);
    out.crossDrop = hud.dropInvDrag(9, 1, bag);
    out.crossSent = sent.slice();
    out.crossPending = hud.invPending && { ...hud.invPending };
    // Predicted on BOTH panels, each in its own array.
    out.crossSelfCell = selfText(5);
    out.crossContCell = contText(9);

    // --- the verdict rolls both ends back ---------------------------------
    out.crossRefused = hud.invMoveVerdict(4, 5, 9, self);
    out.rolledSelf = selfText(5);
    out.rolledCont = contText(9);

    // --- a drag WITHIN the container --------------------------------------
    reset();
    hud.openContainer(bag, 0x1234abcd, bagTexts());
    sent.length = 0;
    hud.beginInvDrag(2, 1, bag);
    out.withinDrop = hud.dropInvDrag(6, 1, bag);
    out.withinSent = sent.slice();
    // The SELF cells must not have moved — the aliasing this whole shape
    // exists to remove is a container slot number reaching the self array.
    out.withinSelf2 = selfText(2);
    out.withinSelf6 = selfText(6);
    out.withinCont2 = contText(2);
    out.withinCont6 = contText(6);

    // --- THE INVARIANT: a close abandons a move it can no longer resolve --
    reset();
    hud.openContainer(bag, 0x1234abcd, bagTexts());
    hud.beginInvDrag(5, 1, self);
    hud.dropInvDrag(9, 1, bag);
    out.beforeClose = !!hud.invPending;
    out.closed = hud.closeContainer();
    out.afterClose = !!hud.invPending;
    // The self end is put back; the container's cells are gone anyway.
    out.selfRestored = selfText(5);
    // And the verdict that arrives late resolves NOTHING.
    out.lateVerdict = hud.invMoveVerdict(4, 5, 9, self);

    // --- re-aiming at another container is a close and an open ------------
    reset();
    hud.openContainer(bag, 0x1234abcd, bagTexts());
    hud.beginInvDrag(5, 1, self);
    hud.dropInvDrag(9, 1, bag);
    out.beforeReaim = !!hud.invPending;
    hud.openContainer(bag, 0x9999, bagTexts());
    out.afterReaim = !!hud.invPending;
    out.reaimHandle = hud.contHandle;

    // --- a self-to-self move SURVIVES a close -----------------------------
    // It has no end in the container, so nothing about it became
    // unresolvable. Abandoning it too would be a panel that forgets moves
    // for an unrelated reason.
    reset();
    hud.openContainer(bag, 0x1234abcd, bagTexts());
    hud.beginInvDrag(5, 1, self);
    hud.dropInvDrag(19, 1, self);
    hud.closeContainer();
    out.selfSurvives = !!hud.invPending;
    out.selfResolves = hud.invMoveVerdict(0, 5, 19, self);

    // --- a drag HELD in the container dies with it ------------------------
    reset();
    hud.openContainer(bag, 0x1234abcd, bagTexts());
    hud.beginInvDrag(3, 1, bag);
    out.heldBefore = hud.invDrag;
    hud.closeContainer();
    out.heldAfter = hud.invDrag;
    // A release arriving after the close must move nothing.
    out.deadDrop = hud.dropInvDrag(4, 1, bag);

    // --- and nothing addresses a container once it is shut ----------------
    out.shutContainers = hud.invContainers.slice();
    out.shutDraws = hud.drawsContainer(bag);
    out.shutBegan = hud.beginInvDrag(3, 1, bag);
    out.shutTexts = hud.textsFor(bag);

    // --- a server restatement outranks the rollback, in the container too -
    reset();
    hud.openContainer(bag, 0x1234abcd, bagTexts());
    hud.beginInvDrag(5, 1, self);
    hud.dropInvDrag(9, 1, bag);
    const restated = bagTexts();
    restated[9] = "SERVER";
    hud.setContainer(restated);
    out.restatedFlag = !!hud.invPending?.restated;
    hud.invMoveVerdict(4, 5, 9, self);
    out.restatedCell = contText(9);

    reset();
    return out;
  },
  [JS_CONT_SELF, JS_CONT_BAG, JS_CONT_BOX, INV_SLOTS, BOX_SLOTS],
);

check(
  cont.startKind === JS_CONT_SELF && cont.startDraws === false && cont.startCells === 0,
  `a fresh panel reports contKind ${cont.startKind}, drawsContainer(bag) ${cont.startDraws}, ${cont.startCells}` +
    " visible container cells — nothing is open until the SERVER says one is, and a panel that started open" +
    " would be predicting visibility rather than drawing it",
);
check(
  cont.opened === true && cont.kind === JS_CONT_BAG && cont.handle === 0x1234abcd,
  `openContainer left kind ${cont.kind} handle ${cont.handle} — the handle is what every move out of this` +
    " container carries as its `bag` argument, and a wrong one addresses a different container entirely",
);
check(
  JSON.stringify(cont.containers) === JSON.stringify([JS_CONT_SELF, JS_CONT_BAG]),
  `an open bag left invContainers ${JSON.stringify(cont.containers)} — that list is what every address is` +
    " checked against, so a container missing from it cannot be dragged and one left in it can be drawn over",
);
check(
  cont.bagCells === INV_SLOTS && cont.bagCell7 === "b7",
  `an open bag drew ${cont.bagCells} cells with slot 7 reading ${JSON.stringify(cont.bagCell7)}, expected` +
    ` ${INV_SLOTS} and "b7" — slot-indexed, the same positional contract setInventory has, and the same shape` +
    " CLAUDE.md's trap list says the reference actually shipped wrong",
);
check(
  cont.panelShown === "flex" && cont.hiddenWithInv === "none" && cont.backWithInv === "flex",
  `the container panel read display ${cont.panelShown} open / ${cont.hiddenWithInv} with the inventory shut /` +
    ` ${cont.backWithInv} reopened — every drag it exists for crosses to the inventory, so a container on` +
    " screen without it has nowhere to drag to",
);
check(
  cont.boxOpened === true && cont.boxCells === BOX_SLOTS && cont.boxTailHidden === true,
  `an open box drew ${cont.boxCells} cells (tail hidden: ${cont.boxTailHidden}), expected ${BOX_SLOTS} —` +
    " a box is twelve slots inside a THIRTY-slot view whose tail is zero-filled, so a panel reading the view's" +
    " width draws twelve slots and eighteen lies",
);
check(
  cont.dropPastBox === false,
  `a drop landed on box slot ${BOX_SLOTS}, one past a box's own width — encode_action_move bounds slots against` +
    " a flat INV_SLOTS and would carry it, and the sim would answer REFUSE_M_SLOT to a move already drawn",
);
check(
  cont.beganSelf === true && cont.crossDrop === true,
  `a drag from self into an open container returned began ${cont.beganSelf} / dropped ${cont.crossDrop} — this` +
    " is the gesture the second panel exists for",
);
check(
  JSON.stringify(cont.crossSent) === JSON.stringify([[JS_CONT_SELF, 5, JS_CONT_BAG, 9]]),
  `the host was handed ${JSON.stringify(cont.crossSent)} rather than [[${JS_CONT_SELF}, 5, ${JS_CONT_BAG}, 9]]` +
    " — four positional values of which two are container kinds, which is the payload shape the reference" +
    " ecosystem corrected ~27 times and a byte-golden cannot see",
);
check(
  cont.crossSelfCell === "" && cont.crossContCell === "s5",
  `the cross-container prediction drew self slot 5 as ${JSON.stringify(cont.crossSelfCell)} and container slot 9` +
    ` as ${JSON.stringify(cont.crossContCell)}, expected "" and "s5" — the item has to leave one panel and` +
    " appear in the other, in the two SEPARATE arrays, or the slot number has aliased across containers",
);
check(
  cont.crossRefused === true && cont.rolledSelf === "s5" && cont.rolledCont === "b9",
  `a refused cross-container move rolled back to ${JSON.stringify([cont.rolledSelf, cont.rolledCont])} rather` +
    ` than ["s5", "b9"] — both ends were drawn, so both ends unwind, each in its own container's array`,
);
check(
  cont.withinDrop === true &&
    JSON.stringify(cont.withinSent) === JSON.stringify([[JS_CONT_BAG, 2, JS_CONT_BAG, 6]]),
  `a drag within one open container sent ${JSON.stringify(cont.withinSent)} — both ends non-self is legal when` +
    " the kind is the same (world.rs carries one handle for it), and rearranging an open box is that shape",
);
check(
  cont.withinSelf2 === "s2" &&
    cont.withinSelf6 === "s6" &&
    cont.withinCont2 === "" &&
    cont.withinCont6 === "b2",
  `a within-container drag left self [${cont.withinSelf2}, ${cont.withinSelf6}] and container` +
    ` [${cont.withinCont2}, ${cont.withinCont6}] — the self cells must not move at all; a container slot` +
    " number reaching the self array is the exact aliasing addresses were introduced to remove",
);
// --- the invariant ---------------------------------------------------------
check(
  cont.beforeClose === true && cont.closed === true && cont.afterClose === false,
  `closing the container left invPending ${cont.afterClose} — a move with an end in a container that is gone` +
    " can never be resolved (the verdict word carries no handle and no sequence number), so a panel that kept" +
    " it would refuse every later drag with \"still moving that\" forever",
);
check(
  cont.selfRestored === "s5",
  `abandoning a cross-container move left self slot 5 as ${JSON.stringify(cont.selfRestored)} rather than "s5"` +
    " — the end this panel still draws is put back, or the player is left looking at a slot emptied by a move" +
    " that was never answered",
);
check(
  cont.lateVerdict === false,
  "a verdict arriving after the container closed still resolved a move — that word carries no handle, so it" +
    " cannot be told apart from a verdict about the container that is open NOW, and matching it would unwind a" +
    " cell the server never spoke about",
);
check(
  cont.beforeReaim === true && cont.afterReaim === false && cont.reaimHandle === 0x9999,
  `re-aiming the panel at handle ${cont.reaimHandle} left invPending ${cont.afterReaim} — a different container` +
    " behind the same panel is a close and an open, and a move sent against the OLD handle would otherwise be" +
    " matched against the new one purely because the kinds and slots line up",
);
check(
  cont.selfSurvives === true && cont.selfResolves === true,
  `a SELF-to-self move was abandoned by a container close (survived: ${cont.selfSurvives}, resolved:` +
    ` ${cont.selfResolves}) — it has no end in that container, so nothing about it became unresolvable, and a` +
    " panel that forgot it would drop moves for an unrelated reason",
);
check(
  cont.heldBefore === 3 && cont.heldAfter === -1 && cont.deadDrop === false,
  `a drag held in the container survived its close (drag ${cont.heldBefore} then ${cont.heldAfter}, late drop` +
    ` ${cont.deadDrop}) — the release would otherwise arrive holding a slot in a container that is gone`,
);
check(
  JSON.stringify(cont.shutContainers) === JSON.stringify([JS_CONT_SELF]) &&
    cont.shutDraws === false &&
    cont.shutBegan === false &&
    cont.shutTexts === null,
  `a shut container still answers drawsContainer ${cont.shutDraws} / beginInvDrag ${cont.shutBegan} /` +
    ` textsFor ${JSON.stringify(cont.shutTexts)} with invContainers ${JSON.stringify(cont.shutContainers)} —` +
    " the server shuts a panel when reach is lost, and a gesture that still addressed it would send a move the" +
    " sim answers REFUSE_M_REACH against cells nobody can see",
);
check(
  cont.restatedFlag === true && cont.restatedCell === "SERVER",
  `a server restatement of a predicted container slot left restated ${cont.restatedFlag} and the cell reading` +
    ` ${JSON.stringify(cont.restatedCell)} — the server's word is newer than the rollback snapshot, so a` +
    " refusal arriving after it must NOT put the snapshot back over an item the sim has since moved elsewhere",
);

// --- the host's ordering, as source ----------------------------------------
// Same standing as the call-site pins in group N: `main.js` boots three.js and
// cannot be imported here, so these are source assertions.
const contHandler = mainSrc.match(/applied2 & APPLIED2_CONT\)[\s\S]*?\n      \}/)?.[0];
check(
  typeof contHandler === "string" && contHandler.includes("client_cont_kind()"),
  "main.js does not read client_cont_kind() on APPLIED2_CONT — the SERVER owns whether a container is open" +
    " (it closes one when the thing despawns or the player walks out of reach), so a panel that tracked its own" +
    " visibility would outlive the reach its moves are judged against",
);
check(
  typeof contHandler === "string" && contHandler.includes("views.refresh()"),
  "main.js reads the container view without refreshing the wasm views first — client_on_stream can grow wasm" +
    " memory and detach every typed array with it, which is the boot bug no native or node gate could see",
);
check(
  mainSrc.indexOf("applied2 & APPLIED2_CONT") > 0 &&
    mainSrc.indexOf("applied2 & APPLIED2_CONT") < mainSrc.indexOf("applied2 & APPLIED2_MOVE"),
  "main.js applies the move verdict BEFORE the container sync — a ContSync that re-aims or closes the view is" +
    " exactly what makes a move in flight unresolvable, so handling the verdict first resolves it against a" +
    " container that is already gone. This is an ORDERING check and the ordering is the whole trap",
);
// Every container open and close reaches the SIM, and none of them is written
// twice. The close is the load-bearing half: without it the server keeps
// unicasting a container's contents every tick to a player who put it away.
//
// The count was 2 until 2026-08-05 (bag open + close), then 3 (the box open),
// and is 2 again — not because a call site was lost but because the resolver
// pass replaced the two per-kind open bodies with ONE `openPicked(kind,
// handle)` that the E dispatch calls with each kind. That is the same law with
// one fewer place to get it wrong, so the count moved and the two checks below
// hold what the count used to imply: both kinds still reach the sim, and each
// does so through the one helper.
check(
  (mainSrc.match(/client_action_container\(/g) || []).length === 2,
  `main.js calls client_action_container from ${(mainSrc.match(/client_action_container\(/g) || []).length}` +
    " places, expected 2 (the one open helper every kind goes through, and the close that rides with the" +
    " inventory) — a third is a per-kind open body growing back beside the shared one, and two places that" +
    " open a container are two places the kind and the handle can be paired wrong",
);
check(
  /case VERB_BAG:[\s\S]{0,400}?openPicked\(CONT_BAG, pick\.handle\)/.test(mainSrc),
  "main.js's E dispatch does not open the resolver's bag with CONT_BAG and the resolver's own handle — the" +
    " bag open stopped reaching the sim, or reaches it with an address the prompt never named",
);
check(
  /client_action_container\(CONT_SELF, 0\)/.test(mainSrc),
  "main.js has no CONT_SELF close — putting the inventory away must reach the SIM and not just the screen," +
    " or the server keeps unicasting a container's contents to a player who is not looking at it",
);
// Re-anchored 2026-08-05 from `tryOpenBag` to `openPicked`, the one helper the
// resolver pass replaced the per-kind open bodies with. The fallback string is
// deliberately one that FAILS the test, so a helper renamed out from under this
// check goes red rather than passing on an empty match.
check(
  !/hud\.(openContainer|closeContainer)\(/.test(
    mainSrc.match(/const openPicked = \(kind, handle\) => \{[\s\S]*?\n  \};/)?.[0] ??
      "hud.openContainer(",
  ),
  "main.js's open-container key draws the panel itself — the view arrives as ContSync on the event lane and" +
    " the server decides whether it opens at all, so drawing on the keypress predicts visibility rather than" +
    " contents",
);

// =============================================================================
// P. the box's address — a bit layout mirrored across the wall
// =============================================================================
// A death backpack has an id. A deployed box does not: it is addressed by its
// grid cell, packed into the one `u32` handle the open and move commands carry
// by `deploy.rs`'s `box_key`. So opening a box means mirroring a bit layout in
// JS, and CLAUDE.md's trap list is explicit that this is where the reference
// ecosystem actually bled.
//
// What makes THIS layout worse than the average mirror, and the reason the
// previous pass refused to write it without this section: `box_key` packs
// `cx << 16 | cz << 4`, and every other packing this client touches packs
// `cx << 16 | cz` — `deployChanges` word 0 (`bridge.rs:393`), the `deployRecs`
// map key (`main.js`), `client_removed_key`, `gather::cell_key`. The habitual
// form is the WRONG one here and it is wrong by twelve bits, which is not a
// crash but a handle naming a real box in a different cell. Every wall in this
// repo is blind to it: the handle is one opaque `u32` whose bytes the encoder
// never inspects (`test_protocol_golden` green), the client's arithmetic is not
// in `state_hash` (`test_replay` green), and every field is an integer (clippy
// green). Nothing else can catch this.
//
// So the layout is stated ONCE as data in `invmove.js`'s `BOX_KEY_LAYOUT`, and
// the checks below read the same three numbers back out of `deploy.rs` — this
// section does not restate the client's opinion and compare it to itself, which
// is the gate-that-matches-nothing class this file has already shipped twice.
const { BOX_KEY_LAYOUT, boxKey, MAX_BUILD_COORD, MAX_BUILD_LEVELS } = await import(
  pathToFileURL(path.join(root, "web/src/invmove.js")).href
);
const deploySrc = fs.readFileSync(
  path.join(root, "crates/sim-core/src/deploy.rs"),
  "utf8",
);
// Anchored at the `fn`, same reason `rustConst` and §N's parameter read are: a
// doc comment quoting the packing must not be able to stand in for it, and the
// doc comment directly above `box_key` quotes `cx << 16 | cz` — the WRONG
// packing, named there precisely to contrast with it. Unanchored, this section
// could gate the client against the very form it exists to rule out.
const boxKeyBody = deploySrc.match(
  /^pub fn box_key\([^)]*\)\s*->\s*u32\s*\{([\s\S]*?)\n\}/m,
)?.[1];
const rsCxShift = Number(boxKeyBody?.match(/\(cx as u32\)\s*<<\s*(\d+)/)?.[1]);
const rsCzShift = Number(boxKeyBody?.match(/\(cz as u32\)\s*<<\s*(\d+)/)?.[1]);
const rsLevelMask = Number(
  boxKeyBody?.match(/level as u32\s*&\s*(0x[0-9a-fA-F]+|\d+)/)?.[1],
);
check(
  typeof boxKeyBody === "string" &&
    Number.isInteger(rsCxShift) &&
    Number.isInteger(rsCzShift) &&
    Number.isInteger(rsLevelMask),
  `could not read box_key's packing out of deploy.rs (cx<<${rsCxShift}, cz<<${rsCzShift}, level&${rsLevelMask})` +
    " — every check below would then compare the client's layout against nothing, which is the" +
    " gate-that-matches-nothing class this section exists to avoid",
);
// Exactly two `|` in the body, so a FOURTH field added to the address on the
// Rust side lands red here on the commit that adds it. Without this a new term
// is invisible: the three the client mirrors keep matching, and the client
// keeps forming a handle that is now missing a component.
check(
  (boxKeyBody.match(/\|/g) || []).length === 2,
  `deploy.rs's box_key ORs ${(boxKeyBody.match(/\|/g) || []).length} times, expected 2 (cx, cz, level) —` +
    " a field was added to the box address and the client's mirror does not carry it, so every handle it" +
    " forms names a different box than the one the player is standing at",
);
check(
  BOX_KEY_LAYOUT.cx_shift === rsCxShift &&
    BOX_KEY_LAYOUT.cz_shift === rsCzShift &&
    BOX_KEY_LAYOUT.level_mask === rsLevelMask,
  `invmove.js BOX_KEY_LAYOUT ${JSON.stringify(BOX_KEY_LAYOUT)} is not deploy.rs's box_key packing` +
    ` (cx<<${rsCxShift} | cz<<${rsCzShift} | level&${rsLevelMask}) — the client would open, and then move` +
    " items into, a box in a different grid cell than the one the player is standing at",
);

// The function against RUST's numbers, not against the client's own constant —
// so a mirror that packed correctly from a wrong layout, or read the layout and
// then ignored it, both land red. Probes chosen so every wrong shift is a
// different number: cx and cz are distinct and non-zero (a transposition
// shows), the level is non-zero (a dropped `| level` shows), and the pair is
// asymmetric.
const rsBoxKey = (cx, cz, level) =>
  (((cx << rsCxShift) | (cz << rsCzShift) | (level & rsLevelMask)) >>> 0);
for (const [cx, cz, level] of [
  [5, 9, 3],
  [9, 5, 3],
  [1, 0, 0],
  [0, 1, 0],
  [0, 0, 1],
  [MAX_BUILD_COORD, MAX_BUILD_COORD, MAX_BUILD_LEVELS - 1],
]) {
  check(
    boxKey(cx, cz, level) === rsBoxKey(cx, cz, level),
    `invmove.boxKey(${cx}, ${cz}, ${level}) = ${boxKey(cx, cz, level)}, but deploy.rs's box_key packs` +
      ` ${rsBoxKey(cx, cz, level)} — the handle names a box the player is not standing at`,
  );
}
// The near-miss, named outright. `cx << 16 | cz` is the packing this client
// uses everywhere else, it is one line away in `main.js` (the `deployRecs` key
// unpacked from `deployChanges`), and for a box it is wrong. Pinning that the
// two DIFFER is what catches the mirror being "corrected" into the house style.
check(
  boxKey(5, 9, 3) !== (((5 << 16) | 9) >>> 0),
  "invmove.boxKey packs cx<<16|cz — the cell_key form used everywhere else in this client, and the one" +
    " form box_key deliberately is not (its own doc says so). Every handle would be short by cz<<4",
);

// The three fields only stay disjoint while the coordinates respect the limits
// Rust declares. `cz` sits at bits 4.. and `cx` starts at bit 16, so a `cz`
// past 4095 carries straight into `cx`'s field and silently names another cell.
// That is not hypothetical maintenance: it is one edit to `MAX_BUILD_COORD`
// away, and nothing else in the repo would notice.
for (const [name, js] of [
  ["MAX_BUILD_COORD", MAX_BUILD_COORD],
  ["MAX_BUILD_LEVELS", MAX_BUILD_LEVELS],
]) {
  const m = limitsSrc.match(new RegExp(`^pub const ${name}: \\w+ = ([\\d_]+);`, "m"));
  const rs = m ? Number(m[1].replace(/_/g, "")) : null;
  check(
    Number.isInteger(rs),
    `could not read ${name} out of limits.rs — the box address's bounds would then be checked against` +
      " nothing, which is the gate-that-matches-nothing class",
  );
  check(
    js === rs,
    `invmove.js ${name} (${js}) has drifted from limits.rs (${rs}) — boxKey would refuse addresses the sim` +
      " accepts, or form ones it cannot decode",
  );
}
check(
  (MAX_BUILD_COORD << rsCzShift) < (1 << rsCxShift),
  `cz at its ceiling (${MAX_BUILD_COORD}) shifted by ${rsCzShift} reaches bit ${rsCxShift} where cx begins —` +
    " the build grid has outgrown box_key's packing, and two different cells now share one container handle",
);
check(
  rsLevelMask < 1 << rsCzShift,
  `box_key's level mask (${rsLevelMask}) overlaps cz at bit ${rsCzShift} — a box on an upper storey and a box` +
    " in a neighbouring cell would share a handle",
);
check(
  MAX_BUILD_LEVELS - 1 <= rsLevelMask,
  `the top storey (${MAX_BUILD_LEVELS - 1}) does not fit box_key's level mask (${rsLevelMask}) — a box up` +
    " there would be addressed as one on a lower floor",
);

// The refusals. An address off the grid must answer null rather than pack one
// that aliases: `boxKey` is fed straight from `deployRecs`, whose fields are
// unpacked from the wire, so "a record this client should not have" is a shape
// it must be able to say no to. Non-integers matter for the same reason they
// do in `moveArgs` — `undefined << 16` is 0, and 0 is a LEGAL handle here (a
// box at cell (0,0) level 0), not a sentinel, so a silent coercion would open
// a real box rather than fail.
for (const [args, why] of [
  [[MAX_BUILD_COORD + 1, 0, 0], "cx past the grid"],
  [[0, MAX_BUILD_COORD + 1, 0], "cz past the grid"],
  [[0, 0, MAX_BUILD_LEVELS], "a storey above the grid"],
  [[-1, 0, 0], "a negative cx"],
  [[0, -1, 0], "a negative cz"],
  [[0, 0, -1], "a negative level"],
  [[1.5, 0, 0], "a non-integer cx"],
  [[undefined, 0, 0], "an undefined cx"],
  [[0, undefined, 0], "an undefined cz"],
  [[0, 0, undefined], "an undefined level"],
]) {
  check(
    boxKey(...args) === null,
    `invmove.boxKey(${args.join(", ")}) packed ${boxKey(...args)} for ${why} — an out-of-range field wraps` +
      " into its neighbour's bits, so this would address a real box somewhere else on the island",
  );
}
// And the corner that is NOT a refusal, pinned so nobody "fixes" it: cell
// (0,0) level 0 packs to 0, which is a legal handle the sim decodes
// (`deploy.rs`'s `box_index` has no zero guard). Biasing it here would put a
// JS-only fudge inside an address Rust decodes.
check(
  boxKey(0, 0, 0) === 0,
  `invmove.boxKey(0, 0, 0) = ${boxKey(0, 0, 0)}, expected 0 — a box at the grid origin is genuinely handle 0` +
    " and biasing it on this side desynchronises the address from the one deploy.rs decodes",
);

// The call site. Until 2026-08-05 this was `main.js`'s `tryOpenBox`, one link
// of the blind E chain; the resolver replaced that chain and the pick moved to
// `web/src/interact.js`, so these checks moved with it. Every contract below
// is the one it was before — where the handle is packed, what kind it opens,
// what it filters on, and that it draws nothing — re-anchored at the code that
// now holds it. A contract whose code moves and whose gate does not is a gate
// on nothing.
const interactSrc = fs.readFileSync(path.join(root, "web/src/interact.js"), "utf8");
const resolveBody = interactSrc.match(
  /export function resolveInteract\(out, aim, world\) \{[\s\S]*?\n\}/,
)?.[0];
check(
  typeof resolveBody === "string",
  "could not find interact.js's resolveInteract — the checks below would read undefined and pass on nothing," +
    " the exact shape of the escaped mutant the 2026-08-05 report found in \u00a7N's call-site pin",
);
// Scoped to the BOX branch rather than the whole function: the resolver
// legitimately writes `>>> 0` to read a bag id as unsigned, and a check that
// banned every shift in the file would be red for a reason that has nothing to
// do with the layout it is defending.
const boxBranch = resolveBody?.match(/if \(verb === VERB_BOX\) \{[\s\S]*?\n    \}/)?.[0];
check(
  typeof boxBranch === "string",
  "could not find the VERB_BOX branch of interact.js's resolveInteract — the packing check below would read" +
    " undefined and pass on nothing",
);
check(
  /boxKey\(/.test(boxBranch) && !/<<|>>|0x/.test(boxBranch),
  "interact.js's resolveInteract packs a box handle itself instead of going through invmove.boxKey — a second" +
    " place the layout is written is a second place it can be written wrong, and this one has no gate on it",
);
// And it refuses the off-grid cell rather than offering a box whose handle the
// keypress would then decline to form. §Q proves the behaviour; this pins that
// the refusal lives at the PICK, which is what stops the prompt disagreeing.
check(
  /if \(key === null\) continue;/.test(boxBranch),
  "interact.js's resolveInteract does not skip a box whose boxKey is null — an off-grid box would be offered" +
    " by the prompt and refused by the key, the exact disagreement the resolver exists to make impossible",
);
check(
  new RegExp("defs\\[[^\\]]*\\]").test(resolveBody) && /ARCH_BOX/.test(resolveBody),
  "interact.js's resolveInteract does not filter deployDefs on ARCH_BOX — E would form a box handle for a" +
    " hearth or a door and the server would answer a container that is not there",
);
// The open itself. One helper now performs every container open the resolver
// picks, so the kind must come from the DISPATCH — a `case VERB_BOX` that
// opened `CONT_BAG` is exactly the positional-argument class CLAUDE.md's trap
// list says a byte-golden is blind to.
const openPickedBody = mainSrc.match(
  /const openPicked = \(kind, handle\) => \{[\s\S]*?\n  \};/,
)?.[0];
check(
  typeof openPickedBody === "string",
  "could not find main.js's openPicked — the container open has no gated call site, which is the" +
    " gate-that-matches-nothing class",
);
check(
  !/hud\.(openContainer|closeContainer)\(/.test(openPickedBody),
  "main.js's openPicked draws the panel itself — the view arrives as ContSync on the event lane and the" +
    " server decides whether it opens at all, so drawing on the keypress predicts visibility rather than contents",
);
check(
  /case VERB_BOX:\s*\n\s*openPicked\(CONT_BOX, pick\.handle\);/.test(mainSrc),
  "main.js's E dispatch does not open the resolver's box with CONT_BOX and the resolver's own handle — a box" +
    " opened as a bag is a kind the sim resolves against the wrong store, and a handle from anywhere but the" +
    " pick is a handle the prompt never named",
);

// =============================================================================
// Q. the interaction prompt — one resolver behind the key and the text
// =============================================================================
// The judge's ranked gap 3 (`pass-20260805-002720-04`): E was five verbs deep
// in a blind fallthrough chain, "stand between a hearth and a box and E does
// something you did not choose, silently", and the only feedback the whole
// chain could produce was the LAST link's toast. The named fix is one resolver
// used by both the prompt and the keypress, "so the prompt can never disagree
// with what the key does".
//
// That property is what this section gates, and it is gated STRUCTURALLY —
// asserting that the prompt happens to read right on one probe would not stop
// a second pick appearing next to it, which is the bug. Three things are held:
// the resolver is the only thing that picks, every verb it can answer has both
// a prompt and a dispatch branch, and its numbers are the sim's rather than
// the client's own opinion.
//
// It is node arithmetic, not a browser probe, for the reason the whole file
// exists: a pick is `dx*dx + dz*dz` and a comparison. The DOM half at the end
// is four checks and is the only part that needs a page.
const interact = await import(pathToFileURL(path.join(root, "web/src/interact.js")).href);

// --- the archetypes and the reach are the SIM's ------------------------------
// `main.js` used to restate all three archetypes at its four scan sites — a
// named `ARCH_BOX = 2` and bare `1`/`6` for the hearth and the door. They are
// one table now, and it is pinned to Rust rather than to itself: a renumbered
// archetype lands red here on the commit that renumbers it, instead of as E
// opening the wrong kind of thing on a shard.
const rsArch = (name) =>
  Number(deploySrc.match(new RegExp(`^pub const ${name}: u8 = (\\d+);`, "m"))?.[1]);
for (const [jsName, rsName] of [
  ["ARCH_HEARTH", "ARCH_HEARTH"],
  ["ARCH_BOX", "ARCH_BOX"],
  ["ARCH_DOOR", "ARCH_DOOR"],
]) {
  const rs = rsArch(rsName);
  check(
    Number.isInteger(rs),
    `could not read ${rsName} out of deploy.rs — the archetype check below would compare the client's` +
      " number against nothing, which is the gate-that-matches-nothing class",
  );
  check(
    interact[jsName] === rs,
    `interact.js ${jsName} = ${interact[jsName]}, deploy.rs ${rsName} = ${rs} — E would resolve the wrong` +
      " archetype and open, feed or swing a door at something that is not one",
  );
}
// The reach is `build.rs`'s BUILD_REACH_M, which `deploy.rs` gates a box open
// and a door use on and `backpack.rs` aliases as LOOT_REACH_M for a bag. A
// client that picked outside it would spend a round trip on a refusal every
// press — quantize both sides, applied to reach.
const buildSrc = fs.readFileSync(path.join(root, "crates/sim-core/src/build.rs"), "utf8");
const rsReach = Number(buildSrc.match(/^pub const BUILD_REACH_M: f32 = ([\d.]+);/m)?.[1]);
check(
  Number.isFinite(rsReach),
  "could not read BUILD_REACH_M out of build.rs — the reach check below would compare against nothing",
);
check(
  interact.INTERACT_REACH_M === rsReach,
  `interact.js INTERACT_REACH_M = ${interact.INTERACT_REACH_M}, build.rs BUILD_REACH_M = ${rsReach} —` +
    " the client would offer a prompt for a thing the server refuses, or refuse one it would have accepted",
);
check(
  interact.INTERACT_AIM_RADIUS_M > 0 && interact.INTERACT_AIM_RADIUS_M < interact.INTERACT_REACH_M,
  `interact.js INTERACT_AIM_RADIUS_M = ${interact.INTERACT_AIM_RADIUS_M} is not inside (0, reach) — at or` +
    " above the reach every candidate is 'aimed' and the two ranks collapse back into nearest-wins, which is" +
    " the blind chain this resolver replaced",
);

// --- every verb has a prompt AND a dispatch branch ---------------------------
// The anti-drift check, and the one that makes "one resolver" hold over time.
// A verb added to interact.js with no prompt would draw an empty hint over a
// thing that does respond to E; a verb added with no dispatch branch in
// main.js would draw a hint for a key that does nothing. Both are silent, and
// both land red here on the commit that adds them — the same shape as the
// REFUSE_M_* table walk in §K.
const VERB_NAMES = [...interactSrc.matchAll(/^export const (VERB_[A-Z]+) = (\d+);/gm)]
  .filter(([, n]) => n !== "VERB_NONE" && n !== "VERB_MAX")
  .map(([, n, v]) => [n, Number(v)]);
check(
  VERB_NAMES.length === interact.VERB_MAX && interact.VERB_MAX >= 4,
  `parsed ${VERB_NAMES.length} verbs out of interact.js against VERB_MAX = ${interact.VERB_MAX} — either the` +
    " parse broke (every check below is then vacuous) or a verb was added without moving VERB_MAX, and the" +
    " walk would step straight over it",
);
// What each branch must DO, by verb. Escaped-regex fragments matched against
// the dispatch, so both the action and the fields it is handed are pinned: a
// door use given the hearth's three-argument form is a different bug from a
// door use given nothing, and neither is arithmetic.
const VERB_DISPATCH = {
  VERB_DOOR: "sendUse\\(pick\\.cx, pick\\.cz, pick\\.level, pick\\.loc\\);",
  VERB_BAG: "if \\(!openPicked\\(CONT_BAG, pick\\.handle\\)\\) takeAll\\(\\);",
  VERB_BOX: "openPicked\\(CONT_BOX, pick\\.handle\\);",
  VERB_HEARTH: "sendFeed\\(pick\\.cx, pick\\.cz, pick\\.level\\);",
};
const seenPrompts = new Set();
for (const [name, v] of VERB_NAMES) {
  const text = interact.promptFor({ verb: v, open: false, locked: false });
  check(
    typeof text === "string" && text.startsWith("[E] ") && text.length > 4,
    `interact.promptFor(${name}) = ${JSON.stringify(text)} — a verb E can perform with no prompt is a thing` +
      " the island never tells the player it offers, which is the whole of the judge's gap 3",
  );
  check(
    !seenPrompts.has(text),
    `interact.promptFor(${name}) = ${JSON.stringify(text)} duplicates another verb's prompt — the hint would` +
      " name one thing and the key would do another",
  );
  seenPrompts.add(text);
  check(
    new RegExp(`case ${name}:`).test(mainSrc),
    `main.js's E dispatch has no \`case ${name}:\` — the resolver can answer ${name} and the keypress would` +
      " fall to the default toast, so the prompt would advertise a verb the key does not perform",
  );
  // …and the branch performs THAT verb's action, on the pick's own fields.
  //
  // A `case` alone is not enough and this is the one hole a first pass of this
  // section shipped: swapping the hearth's `sendFeed` for the door's `sendUse`
  // left every check green while E answered a hearth by toggling it as a door.
  // That is CLAUDE.md's trap verbatim — "the right value in the wrong
  // position" — and the byte-golden is blind to it because both actions encode
  // cleanly. A verb added below without a row here goes red on the parse
  // check above, so the table cannot silently fall behind the enum.
  const action = VERB_DISPATCH[name];
  check(
    action !== undefined,
    `no expected dispatch recorded for ${name} — add its row to VERB_DISPATCH in the same commit that adds` +
      " the verb, or the branch's action is the one thing about it nothing checks",
  );
  // Sliced from this `case` to the next one rather than matched adjacently:
  // the branches carry comments, and a check that a comment can break is a
  // check that gets deleted the first time someone explains a branch.
  const branch = mainSrc
    .slice(mainSrc.indexOf(`case ${name}:`))
    .split(/\n\s*(?:case |default:)/)[0];
  check(
    action !== undefined && new RegExp(action).test(branch),
    `main.js's \`case ${name}:\` does not perform ${action} — a branch wired to another verb's action is the` +
      " right value in the wrong position, which CLAUDE.md's trap list names as the class every byte-golden" +
      " in this repo is blind to",
  );
}
check(
  interact.promptFor({ verb: interact.VERB_NONE }) === "" && interact.promptFor(null) === "",
  "interact.promptFor answers a non-empty string for no pick — the hint would stay on screen naming a verb" +
    " that is not there",
);

// The door's own state, which the walk above cannot see.
//
// The 2026-08-05 judge report's ranked fix 1, as a check. The walk calls
// `promptFor` with `{open: false, locked: false}` for every verb, so the
// `open`/`locked` branch (`interact.js:300-302`) is never exercised: replacing
// that whole line with a flat `[E] OPEN ${label}` passed all 381 checks, which
// is a prompt reading "OPEN DOOR" over a door that is already open shipping
// green. The door is the one verb whose TEXT is a function of state the wire
// carries, so it is the one verb a per-verb walk under-covers.
const doorPrompt = (open, locked) =>
  interact.promptFor({ verb: interact.VERB_DOOR, open, locked });
check(
  doorPrompt(false, false) !== doorPrompt(true, false),
  `promptFor answers ${JSON.stringify(doorPrompt(false, false))} for a door whether it is open or closed —` +
    " E toggles it, so a prompt blind to `open` names the opposite action half the time",
);
check(
  /\bCLOSE\b/.test(doorPrompt(true, false)) && !/\bCLOSE\b/.test(doorPrompt(false, false)),
  `an OPEN door prompts ${JSON.stringify(doorPrompt(true, false))} and a closed one` +
    ` ${JSON.stringify(doorPrompt(false, false))} — the open one is the one that must say CLOSE, and a` +
    " reversed pair is the right value in the wrong position rather than a missing branch",
);
check(
  doorPrompt(false, true) !== doorPrompt(false, false) && /LOCKED/.test(doorPrompt(false, true)),
  `a LOCKED door prompts ${JSON.stringify(doorPrompt(false, true))}, the same as an unlocked one — the wire` +
    " carries the lock bit and the player would learn about it only from a refusal",
);

// --- the pick itself: aim decides, and reach is honoured ---------------------
// The judge's exact sentence, as arithmetic. Two things in reach, one aimed at
// and one not; the aimed one must win, and turning round must reverse it.
// Nothing here restates the resolver's own opinion back at it: the expected
// answers are stated as verbs, and the setup is positions in metres.
const HEARTH_ROW = 0;
const BOX_ROW = 1;
const DOOR_ROW = 2;
const qDefs = [];
qDefs[HEARTH_ROW * 4] = interact.ARCH_HEARTH;
qDefs[BOX_ROW * 4] = interact.ARCH_BOX;
qDefs[DOOR_ROW * 4] = interact.ARCH_DOOR;
const rec = (row, cx, cz, extra = {}) => ({
  row,
  cx,
  cz,
  level: 0,
  loc: 0,
  open: false,
  locked: false,
  ...extra,
});
const CELL = 3; // main.js's BUILD_CELL; the world adapter's own field
const qWorld = (recs, bags = []) => ({
  cell: CELL,
  defs: qDefs,
  recs,
  bagCount: bags.length,
  bagPos: bags.flatMap(([x, z]) => [x, 0, z]),
  bagIds: bags.map((_, i) => i + 11),
});
const qPick = interact.newPick();
const qAt = (world, x, z, fx, fz, only) =>
  interact.resolveInteract(qPick, { x, z, fx, fz, only }, world);
// A hearth in cell (0,0) — centre (1.5, 1.5) — and a box in cell (0,1) —
// centre (1.5, 4.5). The player stands at (1.5, 3.0), exactly between them.
const between = qWorld([rec(HEARTH_ROW, 0, 0), rec(BOX_ROW, 0, 1)]);
check(
  qAt(between, 1.5, 3.0, 0, 1).verb === interact.VERB_BOX,
  "standing between a hearth and a box and LOOKING AT THE BOX, the resolver picked" +
    ` ${qPick.verb} instead of the box — this is the judge's gap 3 verbatim and the reason this file changed`,
);
check(
  qAt(between, 1.5, 3.0, 0, -1).verb === interact.VERB_HEARTH,
  `looking the other way the resolver still picked ${qPick.verb} — if aim does not reverse the pick then aim` +
    " is not deciding it, and the prompt is decoration over a fallthrough chain",
);
// The case that made the first attempt at this resolver wrong, kept as a gate
// because it is not obvious and it is not visible from the outside: the
// player standing exactly ON a deployable's cell centre. Its distance to the
// aim line is zero from every direction, so a pure closest-to-the-line metric
// hands it every press and no box in the room can ever be opened. The `t > 0`
// half of the aimed test is what rules it out.
const underfoot = qWorld([rec(HEARTH_ROW, 0, 0), rec(BOX_ROW, 0, 1)]);
check(
  qAt(underfoot, 1.5, 1.5, 0, 1).verb === interact.VERB_BOX,
  "standing exactly on the hearth's cell centre and looking at the box, the resolver picked" +
    ` ${qPick.verb} — a thing underfoot is at zero distance from the aim line in every direction and must not` +
    " therefore trump everything the player can actually see",
);
check(
  qAt(underfoot, 1.5, 1.5, 0, -1).verb === interact.VERB_HEARTH,
  "standing on the hearth with nothing aimed at, the resolver picked" +
    ` ${qPick.verb} — the nearby rank is what keeps every verb that worked before the resolver working,` +
    " and a thing at your feet is the case it exists for",
);
// The nearby rank, explicitly: behind you, with nothing in front, still opens.
// The old chain had no notion of aim at all, so anything this refuses that it
// used to accept is a regression rather than a fix.
check(
  qAt(qWorld([rec(DOOR_ROW, 0, 0)]), 1.5, 3.0, 0, 1).verb === interact.VERB_DOOR,
  `a door behind the player with nothing else in reach resolved to ${qPick.verb} — being off-aim must cost a` +
    " candidate only against something that is ON aim, never the press itself",
);
// Aimed beats nearby regardless of distance — a box at the far edge of reach,
// dead centre, against a hearth a metre off to the side.
check(
  qAt(qWorld([rec(HEARTH_ROW, 0, 0), rec(BOX_ROW, 0, 1)]), 1.5, 0.2, 0, 1).verb ===
    interact.VERB_HEARTH,
  `the nearer of two AIMED candidates lost (got ${qPick.verb}) — among things under the crosshair the first` +
    " one along the line is what a player is pointing at; the thing behind it is not",
);
// The exact tie, which is the only thing TIE_ORDER decides and therefore the
// only thing that can catch it being reordered. A hearth at (1.5, 1.5) and a
// box at (4.5, 1.5) with the player at (3.0, 1.5) looking crosswise: both are
// 1.5 m away, both are in the nearby rank, and every float in the comparison
// is identical. The pick still has to be a function of its inputs — the prompt
// is drawn on a timer and the verb runs on a keypress, and a pick that is a
// coin toss between them is the disagreement this section exists to rule out.
// The order it falls back to is the chain's own: a box outranks a hearth.
const tied = qWorld([rec(HEARTH_ROW, 0, 0), rec(BOX_ROW, 1, 0)]);
check(
  qAt(tied, 3.0, 1.5, 0, 1).verb === interact.VERB_BOX && qPick.d2 === 2.25,
  `two candidates at exactly equal distance and equal rank resolved to ${qPick.verb} (d2 ${qPick.d2}) —` +
    " expected the box, TIE_ORDER's answer and the fallthrough chain's. If this is not pinned, the tiebreak" +
    " can be reordered with no gate noticing and E answers a dead heat differently on two consecutive frames",
);
// Reach is a hard edge and it is the server's. Just inside opens, just outside
// does not, and "just outside" must produce NO pick rather than a silent one.
const far = qWorld([rec(BOX_ROW, 0, 4)]); // centre (1.5, 13.5)
check(
  qAt(far, 1.5, 9.0, 0, 1).verb === interact.VERB_BOX,
  `a box ${(13.5 - 9.0).toFixed(1)} m ahead is inside BUILD_REACH_M and resolved to ${qPick.verb}`,
);
check(
  qAt(far, 1.5, 8.0, 0, 1).verb === interact.VERB_NONE,
  `a box ${(13.5 - 8.0).toFixed(1)} m ahead is OUTSIDE BUILD_REACH_M and still resolved to ${qPick.verb} —` +
    " the client would prompt for a verb the server answers with a refusal",
);
// A bag is addressed by its id and carries a world position, not a cell.
// Getting the stride wrong (`bagPos` is x,y,z) reads a height as a Z and puts
// every backpack in the wrong place, which no other gate here would see.
// TWO bags at different distances, and the AIMED one is the second, so the
// check reads the stride rather than merely reaching a bag. It is written this
// way because the single-bag version of it passed a mutant that read `bagPos`
// with a stride of 2: with one bag, x still lands on x and every wrong answer
// still resolves to that bag with that id. Asserting the geometry — which bag,
// at what distance, aimed or not — is what makes the stride observable.
const bagW = qWorld([], [
  [1.5, 1.0],
  [1.5, 4.5],
]);
check(
  qAt(bagW, 1.5, 3.0, 0, 1).verb === interact.VERB_BAG &&
    qPick.bag === 1 &&
    qPick.handle === 12 &&
    qPick.aimed === true &&
    qPick.d2 === 2.25,
  `a bag 1.5 m dead ahead, with a second bag 2 m behind, resolved to verb ${qPick.verb} index ${qPick.bag}` +
    ` handle ${qPick.handle} aimed=${qPick.aimed} d2=${qPick.d2} — expected the aimed one (index 1, id 12,` +
    " d2 2.25). A wrong index loots a different corpse, and a wrong stride reads a bag's HEIGHT as its Z and" +
    " puts every backpack in the world somewhere it is not",
);
// The `only` filter, which is what makes L's lock the SAME pick as E's use.
// Without it L kept its own nearest-door scan and the two keys could name
// different doors — one more parallel pick, which is the gap this closed.
const boxAndDoor = qWorld([rec(BOX_ROW, 0, 1), rec(DOOR_ROW, 1, 1)]);
check(
  qAt(boxAndDoor, 1.5, 3.0, 0, 1).verb === interact.VERB_BOX,
  `unfiltered, the resolver picked ${qPick.verb} rather than the box it is aimed at`,
);
check(
  qAt(boxAndDoor, 1.5, 3.0, 0, 1, interact.VERB_DOOR).verb === interact.VERB_DOOR,
  `restricted to VERB_DOOR the resolver answered ${qPick.verb} — L must be able to name the door under the` +
    " aim by the same metric E uses, or the two keys disagree about which door the player means",
);
check(
  qAt(qWorld([rec(BOX_ROW, 0, 1)]), 1.5, 3.0, 0, 1, interact.VERB_DOOR).verb === interact.VERB_NONE,
  `restricted to VERB_DOOR with no door in the world the resolver answered ${qPick.verb} — a filter that` +
    " leaks another verb would have L locking a box",
);
// An off-grid box is not offered at all. `boxKey` answers null there rather
// than packing a handle that aliases into a neighbouring cell, and the pick is
// where that refusal has to happen: a prompt for a box the key then declines
// to open is exactly the disagreement this section exists to rule out.
const offGrid = qWorld([rec(BOX_ROW, MAX_BUILD_COORD + 1, 0)]);
check(
  boxKey(MAX_BUILD_COORD + 1, 0, 0) === null,
  "invmove.boxKey packs an off-grid cell instead of refusing it — §P's premise, restated here because the" +
    " check below is meaningless without it",
);
check(
  qAt(offGrid, (MAX_BUILD_COORD + 1) * CELL + 1.5, 1.5, 0, 1).verb === interact.VERB_NONE,
  `a box outside the build grid was offered as ${qPick.verb} — the prompt would name a box whose handle the` +
    " keypress refuses to form, and the player would press E at a box that never answers",
);
// Determinism: the same world and the same aim resolve to the same pick, every
// time. The prompt is drawn on a 250 ms timer and the verb runs on a keypress,
// so a resolver with any state of its own could answer them differently while
// the world stood still — which is precisely the disagreement the single
// resolver was supposed to make unrepresentable.
const detA = interact.newPick();
const detB = interact.newPick();
interact.resolveInteract(detA, { x: 1.5, z: 3.0, fx: 0, fz: 1 }, between);
interact.resolveInteract(detB, { x: 1.5, z: 3.0, fx: 0, fz: 1 }, between);
check(
  JSON.stringify(detA) === JSON.stringify(detB),
  `two identical resolves disagreed (${JSON.stringify(detA)} vs ${JSON.stringify(detB)}) — the prompt and the` +
    " keypress run this at different moments and must get the same answer from the same world",
);
// A non-unit look vector must not change the pick. `main.js` passes
// `(sin yaw, cos yaw)` today, but the metric normalises so that a caller who
// passes a scaled or unnormalised direction cannot silently widen the aim.
const scaled = interact.newPick();
interact.resolveInteract(scaled, { x: 1.5, z: 3.0, fx: 0, fz: 40 }, between);
check(
  scaled.verb === detA.verb && scaled.aimed === detA.aimed,
  `a look vector of length 40 resolved to ${scaled.verb} where a unit one resolved to ${detA.verb} —` +
    " an unnormalised direction scales the aim radius with it",
);

// --- main.js drives BOTH from the one resolver -------------------------------
// The structural half. Two call sites are correct (the keypress and the HUD
// timer); a THIRD scan next to them would be a second opinion, and the old
// chain's five scans are exactly what this replaced. `aimPick` is the single
// function both go through, so pinning it pins the property.
check(
  /const aimPick = \(out, only\) => \{[\s\S]*?resolveInteract\(out, interactAim, interactWorld\);/.test(
    mainSrc,
  ),
  "main.js's aimPick does not end in resolveInteract — the one place the client is allowed to decide what E" +
    " acts on has stopped being the place that decides it",
);
// The E prompt's text comes from `promptFor(aimPick(...))` and reaches the
// screen unaltered.
//
// This was one regex — `hud.setPrompt(promptFor(aimPick(` — until the swing
// prompt landed behind it and the call became two lines. RE-ANCHORED, not
// relaxed: the old form pinned that the two were composed in one expression,
// and these pin the same composition plus the thing the old one could not say,
// which is that whatever else `setPrompt` is handed, E's answer OUTRANKS it.
// A mutant that draws E's prompt from a second resolver still fails the first;
// a mutant that reorders the `||` so the swing hint wins fails the second.
check(
  /const text = promptFor\(aimPick\(pick, VERB_NONE\)\);/.test(mainSrc),
  "main.js does not compute the prompt text from promptFor(aimPick(...)) — a prompt computed any other way" +
    " is a second opinion, and the judge's gap 3 is that the prompt and the key must not be able to differ",
);
check(
  /hud\.setPrompt\(text \|\| /.test(mainSrc),
  "main.js does not hand `text` to setPrompt as the FIRST term of the fallback — E's pick is the half a" +
    " player cannot discover, so anything else drawn under the crosshair sits behind it or the door you are" +
    " standing at loses its hint to a tree",
);
// And it is CALLED. Pinning only the definition let a mutant that commented
// out the one call site pass green: the prompt would be correct code that
// never runs, which on screen is indistinguishable from the blind chain this
// replaced. The call has to be on the HUD's slow timer specifically — in the
// RAF loop it would sweep every deployable in the world per frame, against
// L8 (UI in plain DOM outside the loop).
const hudTimerBody = mainSrc.match(/setInterval\(\(\) => \{[\s\S]*?\n  \}, \d+\);/)?.[0];
check(
  typeof hudTimerBody === "string",
  "could not find main.js's HUD setInterval — the prompt's call site check below would pass on nothing",
);
check(/\n\s*updatePrompt\(\);/.test(hudTimerBody ?? ""),
  "main.js's HUD timer does not call updatePrompt() — the prompt is code that never runs, and a hint that is" +
    " never drawn leaves E exactly as silent as the fallthrough chain it replaced",
);
check(
  /const tryUse = \(\) => \{\s*\n\s*aimPick\(pick, VERB_NONE\);\s*\n\s*switch \(pick\.verb\)/.test(mainSrc),
  "main.js's tryUse does not open by resolving the pick and switching on it — if the keypress re-derives its" +
    " own target the prompt is decoration",
);
check(
  /const tryLock = \(\) => \{\s*\n\s*const best = aimPick\(lockPick, VERB_DOOR\);/.test(mainSrc),
  "main.js's tryLock does not take its door from aimPick — L would keep a parallel nearest-door scan and the" +
    " two keys could name different doors",
);
// The chain is gone, not merely bypassed. Left in place it is dead code that
// reads like a live second pick, and the next pass to touch E would find two.
for (const dead of ["tryOpenBag", "tryOpenBox", "nearestDoor", "tryFeed", "tryLoot"]) {
  check(
    !new RegExp(`const ${dead} = `).test(mainSrc),
    `main.js still defines ${dead} — the blind fallthrough chain was replaced by the resolver, and a surviving` +
      " link is a second pick sitting next to the one that is supposed to be the only one",
  );
}
// The empty answer reaches the player. The old chain's one piece of feedback
// was "no hearth in reach", fired on an empty island because the hearth
// happened to be the last link tried.
check(
  /default:[\s\S]{0,600}?hud\.toast\("nothing in reach"\)/.test(mainSrc),
  'main.js\'s E dispatch does not toast "nothing in reach" on no pick — the old chain answered an empty' +
    ' island with "no hearth in reach", which named a verb the player had not asked for',
);

// --- the DOM half ------------------------------------------------------------
// Four checks, the only part of this section that needs a page. `#prompt` is
// already in §A's scaffold sweep (HUD_IDS is parsed out of hud.js, so a new
// getElementById joins it automatically); what is left is that it starts
// hidden, that it shows and hides on the text, and that the text goes in as
// TEXT.
const prompt = await page.evaluate(() => {
  const { hud } = globalThis.__ui;
  const el = document.getElementById("prompt");
  const atLoad = getComputedStyle(el).display;
  hud.setPrompt("[E] OPEN BOX");
  const shown = { display: el.style.display, text: el.textContent };
  hud.setPrompt("");
  const hidden = { display: el.style.display, text: el.textContent };
  hud.setPrompt("<img src=x onerror=1>");
  const markup = { kids: el.childElementCount, text: el.textContent };
  hud.setPrompt("");
  return { atLoad, shown, hidden, markup };
});
check(
  prompt.atLoad === "none",
  `#prompt computed display at load is ${prompt.atLoad}, expected none — a hint naming a verb before the` +
    " player is even in the world is a hint for a thing that is not there",
);
check(
  prompt.shown.display === "block" && prompt.shown.text === "[E] OPEN BOX",
  `setPrompt drew ${JSON.stringify(prompt.shown)} — the prompt must show its exact text and nothing else`,
);
check(
  prompt.hidden.display === "none" && prompt.hidden.text === "",
  `setPrompt("") left ${JSON.stringify(prompt.hidden)} — walking away from a box must take its hint with it,` +
    " or the player is told E does something it no longer does",
);
check(
  prompt.markup.kids === 0 && prompt.markup.text === "<img src=x onerror=1>",
  `setPrompt built ${prompt.markup.kids} child elements from a string — the prompt names a thing whose text` +
    " will one day come from content or from another player, and textContent is that rule everywhere or nowhere",
);

// =============================================================================
// R. the OTHER crosshair answer: what a swing would hit
// =============================================================================
// `resolveSwing` mirrors `gather::swing`'s target selection, and the reason it
// needs a gate rather than a reading is that NOTHING ELSE IN THE REPO CAN SEE A
// DIVERGENCE. The client sends a swing as a button bit (`BTN_PRIMARY`) and the
// sim picks the node on its own: no packet carries the client's opinion of the
// target, so the protocol golden is untouched by any error here, the replay
// hash is untouched, and clippy sees ordinary float arithmetic. A prompt that
// names the wrong tree is invisible to every wall — it is CLAUDE.md's
// positional-payload class again, one level up: the right rule computed against
// the wrong constant.
//
// So the constants are read back out of Rust rather than restated, and the
// behaviour is probed at the boundaries the sim's own tests would move.
const gatherSrc = fs.readFileSync(path.join(root, "crates/sim-core/src/gather.rs"), "utf8");
const terrainSrc = fs.readFileSync(path.join(root, "crates/sim-core/src/terrain.rs"), "utf8");
const rsFloat = (src, file, name) => {
  const m = src.match(new RegExp(`^pub const ${name}: f32 = ([\\d._]+);`, "m"));
  check(
    m !== null,
    `could not read ${name} out of ${file} — every check keyed to it below would then compare against NaN,` +
      " which is the gate-that-matches-nothing class",
  );
  return m ? Number(m[1].replace(/_/g, "")) : NaN;
};
const rsReachG = rsFloat(gatherSrc, "gather.rs", "REACH_M");
const rsCone = rsFloat(gatherSrc, "gather.rs", "CONE_COS");
const rsDy = rsFloat(gatherSrc, "gather.rs", "DY_MAX_M");
const rsBlank = rsFloat(gatherSrc, "gather.rs", "POINT_BLANK_M2");
const rsCell = rsFloat(terrainSrc, "terrain.rs", "CELL_SIZE");
// The mirrors, each against the constant it claims to be. `CONE_COS` is
// authored to seven places in `gather.rs` precisely so no trig runs at
// runtime; a JS `Math.cos(Math.PI/6)` would be a DIFFERENT number and the
// cone would disagree with the sim's at its edge, which is the only place a
// cone is ever wrong.
for (const [name, js, rs, file] of [
  ["GATHER_REACH_M", interact.GATHER_REACH_M, rsReachG, "gather.rs REACH_M"],
  ["GATHER_CONE_COS", interact.GATHER_CONE_COS, rsCone, "gather.rs CONE_COS"],
  ["GATHER_DY_MAX_M", interact.GATHER_DY_MAX_M, rsDy, "gather.rs DY_MAX_M"],
  ["GATHER_POINT_BLANK_M2", interact.GATHER_POINT_BLANK_M2, rsBlank, "gather.rs POINT_BLANK_M2"],
  ["TERRAIN_CELL_M", interact.TERRAIN_CELL_M, rsCell, "terrain.rs CELL_SIZE"],
]) {
  check(
    js === rs,
    `interact.js ${name} = ${js} but ${file} = ${rs} — the prompt would describe a swing the arm does not` +
      " make, and no wire golden, replay hash or clippy wall can see the difference",
  );
}
// The occupant ordinals, out of the enum itself. Renumbering `Occupant` is the
// mutation this catches: `props.js` indexes its archetype table by the same
// ordinals, so a renumber would move the DRAWN thing and the PROMPTED thing
// together in the renderer and apart here.
const occOrd = (name) =>
  Number(terrainSrc.match(new RegExp(`^\\s*${name} = (\\d+),`, "m"))?.[1]);
for (const [name, js] of [
  ["Tree", interact.OCC_TREE],
  ["StoneNode", interact.OCC_STONE],
  ["MetalNode", interact.OCC_METAL],
  ["SulfurNode", interact.OCC_SULFUR],
  ["Bush", interact.OCC_BUSH],
  ["Rock", interact.OCC_ROCK],
  ["BarrelSlot", interact.OCC_BARREL],
]) {
  const rs = occOrd(name);
  check(
    Number.isInteger(rs) && rs === js,
    `interact.js has ${name} at ${js}, terrain.rs's Occupant enum has it at ${rs} — a swing prompt keyed to` +
      " the wrong ordinal names whatever archetype now sits at that number",
  );
}
// `GATHERABLE_KINDS` bounds the run 1..=N, and ROCK sitting one past it is the
// whole reason the prompt has to exclude something: `gather.rs` calls Rock and
// an empty cell "the two things a swing passes through".
const rsGatherable = Number(
  gatherSrc.match(/^pub const GATHERABLE_KINDS: usize = (\d+);/m)?.[1],
);
check(
  rsGatherable === interact.OCC_BUSH && interact.OCC_ROCK === rsGatherable + 1,
  `gather.rs GATHERABLE_KINDS = ${rsGatherable}, so occupants 1..=${rsGatherable} gather and ROCK is` +
    ` ${interact.OCC_ROCK} — interact.js's run must end exactly where the enum's does`,
);
// A label for every swingable ordinal and for nothing else. A missing row is a
// node that draws no prompt; a spare row is a prompt for a thing a swing
// passes through, and the second is worse because it reads as a promise.
const swingable = [1, 2, 3, 4, 5, 7];
for (let occ = 0; occ <= 8; occ++) {
  const text = interact.promptForSwing({ arch: occ });
  const want = swingable.includes(occ);
  check(
    want === (text !== ""),
    `promptForSwing(arch ${occ}) = ${JSON.stringify(text)} but ${want ? "is" : "is not"} swingable —` +
      " occupant 6 is ROCK and 8 is the client-only stump, and a prompt over either offers a swing that whiffs",
  );
}
check(
  interact.promptForSwing({ arch: 0 }) === "" && interact.promptForSwing(null) === "",
  "promptForSwing answers text for a whiff — the hint would stay on screen over empty ground",
);
const swingTexts = swingable.map((o) => interact.promptForSwing({ arch: o }));
check(
  new Set(swingTexts).size === swingTexts.length,
  `the swing prompts are not distinct (${JSON.stringify(swingTexts)}) — two nodes sharing a hint is the` +
    " prompt naming one thing while the arm hits another",
);
check(
  swingTexts.every((t) => t.startsWith("[LMB] ")),
  `a swing prompt does not name its button (${JSON.stringify(swingTexts)}) — the key is the half of the hint` +
    " the player cannot guess, which is the whole reason the E prompt names E",
);

// --- the pick, against the sim's own scan ------------------------------------
// A world of one cell entry, addressed the way `terrain.js`'s `cellIndex` is.
const GCELL = interact.TERRAIN_CELL_M;
const swingWorldOf = (entries) => ({
  cellAt: (cx, cz) => entries.find((e) => e.cx === cx && e.cz === cz) || null,
});
const gAt = (cx, cz, over) => ({
  cx,
  cz,
  arch: interact.OCC_TREE,
  x: cx * GCELL,
  y: 0,
  z: cz * GCELL,
  hidden: false,
  // Both halves of "down", because `terrain.js` keeps them in two fields and a
  // fixture carrying only one would let the resolver read only one.
  fellAt: 0,
  ...over,
});
const sPick = interact.newSwingPick();
// The player stands at the centre of cell (1,1) looking +X, and every case
// below moves ONE term.
const P = { x: GCELL + GCELL / 2, y: 0, z: GCELL + GCELL / 2, fx: 1, fz: 0 };
const swingAt = (entries, over) =>
  interact.resolveSwing(sPick, { ...P, ...over }, swingWorldOf(entries));
// Reach: the sim's test is `d2 <= REACH_M * REACH_M`, planar.
const inReach = gAt(1, 1, { x: P.x + rsReachG - 0.05, z: P.z });
const outReach = gAt(1, 1, { x: P.x + rsReachG + 0.05, z: P.z });
check(
  swingAt([inReach]).arch === interact.OCC_TREE,
  `a tree ${(rsReachG - 0.05).toFixed(2)} m ahead is inside gather.rs's ${rsReachG} m reach and resolved to` +
    ` ${swingAt([inReach]).arch}`,
);
check(
  swingAt([outReach]).arch === 0,
  `a tree ${(rsReachG + 0.05).toFixed(2)} m ahead is OUTSIDE the ${rsReachG} m reach and still prompted —` +
    " the hint would offer a swing the sim answers with a whiff, and the cooldown is paid on a whiff",
);
// The cone. A node at 90° is in reach and behind the cone's edge; the same
// node dead ahead is inside it. This is the term that separates a cone from a
// radius, and dropping it entirely is the mutation it catches.
const side = gAt(1, 1, { x: P.x, z: P.z + 1.0 });
const ahead = gAt(1, 1, { x: P.x + 1.0, z: P.z });
check(
  swingAt([ahead]).arch === interact.OCC_TREE && swingAt([side]).arch === 0,
  `a tree 1 m dead ahead resolved to ${swingAt([ahead]).arch} and one 1 m to the SIDE to` +
    ` ${swingAt([side]).arch} — the side one is outside a ${rsCone} cone and a swing at it whiffs`,
);
// Point-blank bypasses the cone: inside `POINT_BLANK_M2` there is no bearing.
const under = gAt(1, 1, { x: P.x, z: P.z + Math.sqrt(rsBlank) * 0.5 });
check(
  swingAt([under]).arch === interact.OCC_TREE,
  "a tree inside POINT_BLANK_M2 to the side did not resolve — the sim bypasses the cone there because a" +
    " zero-length offset has no direction to test, and a prompt that keeps testing it goes blank underfoot",
);
// The vertical window, which nothing else in this file has an analogue of.
check(
  swingAt([gAt(1, 1, { x: P.x + 1.0, z: P.z, y: rsDy - 0.05 })]).arch === interact.OCC_TREE &&
    swingAt([gAt(1, 1, { x: P.x + 1.0, z: P.z, y: rsDy + 0.05 })]).arch === 0,
  `the ±${rsDy} m vertical window is not honoured — a node on a ledge above the player is in planar reach` +
    " and out of the sim's, so the prompt would offer a swing at something overhead",
);
// ROCK is in reach, dead ahead, and must still answer nothing.
check(
  swingAt([gAt(1, 1, { x: P.x + 1.0, z: P.z, arch: interact.OCC_ROCK })]).arch === 0,
  "a ROCK dead ahead resolved to a swing target — gather.rs passes straight through it, so the prompt would" +
    " promise a payout that never arrives",
);
// Harvested: standing on this client's clock only, and the whole reason
// `entry.hidden` is read rather than the archetype alone.
check(
  swingAt([gAt(1, 1, { x: P.x + 1.0, z: P.z, hidden: true })]).arch === 0,
  "a HARVESTED node still prompted — `terrain.setCellHarvested` is the event lane telling this client the" +
    " thing is down, and a hint over a stump is the clearest possible lie",
);
// Mid-fall: the SAME lie, in the window where `hidden` has not caught up yet.
//
// A tree is the one occupant that does not vanish when the sim takes it.
// `setCellHarvested` sets `fellAt` and RETURNS — `hidden` stays false — and
// `_stepFells` only sets `hidden` once the trunk has landed and sunk, at
// `FELL_TICKS + FELL_SINK_TICKS` = 33 + 60 = 93 ticks, which is 3.1 s at
// `TICK_HZ` 30. For that whole window `cellEntry` hands back a non-hidden tree
// at a cell the sim has already harvested, so reading `hidden` alone offered
// "[LMB] CHOP TREE" over it and the swing it invited would whiff.
//
// `terrain.js`'s own `setCellHarvested` defines the state as
// `entry.hidden || entry.fellAt !== 0`; this pins the resolver to that same
// definition rather than to half of it.
check(
  swingAt([gAt(1, 1, { x: P.x + 1.0, z: P.z, fellAt: 1 })]).arch === 0,
  "a tree MID-FALL still prompted — `hidden` is false for the 93 ticks (3.1 s) between the harvest event and" +
    " the trunk landing, and terrain.js calls that state down; a resolver reading only `hidden` promises a" +
    " chop on a tree the sim has already taken",
);
// And the pair is read as an OR, not as an AND or as `fellAt` alone: a node
// that is hidden with no fall recorded (every non-tree harvest, and a tree
// whose fall has retired) must still be skipped.
check(
  swingAt([gAt(1, 1, { x: P.x + 1.0, z: P.z, hidden: true, fellAt: 1 })]).arch === 0 &&
    swingAt([gAt(1, 1, { x: P.x + 1.0, z: P.z, hidden: true, fellAt: 0 })]).arch === 0,
  "the two halves of `down` are not read as an OR — a rock harvest sets `hidden` with no `fellAt`, and a" +
    " landed tree carries `hidden` with `fellAt` cleared, so either alone must be enough to skip the cell",
);
// The window is 3×3 CELLS, not a radius: the sim never looks further, so a
// node inside the 2 m reach whose cell is two away cannot be hit.
check(
  swingAt([gAt(3, 1, { x: P.x + 1.0, z: P.z })]).arch === 0,
  "a node two cells away resolved even though gather.rs scans a 3x3 cell block around the player's own cell" +
    " — the prompt must be bounded by the scan, not by the reach alone",
);
// The tiebreak. Two nodes at an exact tie, and the sim walks dz outer, dx
// inner with a STRICT `<`, so the first one found keeps it. Reversing either
// the loop order or the comparison flips this.
// The two cells are (2,0) and (0,2) for a reason, and it is the one thing a
// first version of this check got wrong: a tie between (0,0) and (2,2) is kept
// by (0,0) under BOTH nestings, so transposing the loops escaped the gate
// green. These two swap places when the loops swap — dz outer reaches (2,0)
// first, dx outer reaches (0,2) first — which is what makes the walk order
// observable rather than merely stated in a comment.
const tieA = gAt(2, 0, { x: P.x + 1.0, z: P.z, arch: interact.OCC_STONE });
const tieB = gAt(0, 2, { x: P.x + 1.0, z: P.z, arch: interact.OCC_METAL });
check(
  swingAt([tieA, tieB]).arch === interact.OCC_STONE &&
    swingAt([tieB, tieA]).arch === interact.OCC_STONE,
  `an exact distance tie resolved to ${swingAt([tieA, tieB]).arch} and is order-dependent — gather.rs walks` +
    " dz outer, dx inner and compares strictly, so the FIRST cell in that order keeps the tie whatever order" +
    " the entries arrive in",
);
// Nearest wins inside the cone, which is the ordinary case.
// One scatter slot per cell, so the two nodes sit in two cells — which is also
// why the further one is listed FIRST: it is found first in the dz/dx walk, so
// nearest-wins has to actively overtake it rather than merely be found later.
check(
  swingAt([
    gAt(0, 1, { x: P.x + 1.6, z: P.z, arch: interact.OCC_METAL }),
    gAt(1, 1, { x: P.x + 0.6, z: P.z, arch: interact.OCC_BUSH }),
  ]).arch === interact.OCC_BUSH,
  "the FURTHER of two aimed nodes won the swing — nearest-wins is the sim's rule and the near one is what" +
    " the arm reaches",
);
// Turning round reverses it, which is the same sentence the E resolver's own
// aim check makes and the reason both exist.
check(
  swingAt([ahead], { fx: -1, fz: 0 }).arch === 0,
  "a tree in front still prompted with the player facing AWAY from it — the cone is the whole difference" +
    " between a swing target and a thing that happens to be nearby",
);
// A zero-length look vector must not throw or resolve everything: it degrades
// to the point-blank case, exactly as the sim's does.
check(
  swingAt([ahead], { fx: 0, fz: 0 }).arch === 0 && swingAt([under], { fx: 0, fz: 0 }).arch !== 0,
  "a zero-length look vector did not degrade to point-blank only — a client can hold one at spawn, and" +
    " normalising it to NaN would make the prompt name whatever the last cell scanned was",
);
// A cell the client has not streamed has no entry, and must be skipped rather
// than crash the HUD timer.
check(interact.resolveSwing(sPick, P, { cellAt: () => null }).arch === 0, "an unstreamed world threw or resolved");

// --- the wiring, in main.js --------------------------------------------------
// Same discipline as §Q: the resolver being right is worth nothing if the
// prompt is drawn from something else.
check(
  /hud\.setPrompt\(text \|\| promptForSwing\(swingAt\(\)\)\)/.test(mainSrc),
  "main.js does not fall back to promptForSwing(swingAt()) when E has no pick — the swing half of the" +
    " crosshair would compute correctly and never reach the screen",
);
check(
  /const text = promptFor\(aimPick\(pick, VERB_NONE\)\)/.test(mainSrc),
  "main.js no longer draws E's prompt through promptFor(aimPick(...)) — the fallback must sit BEHIND the" +
    " interact pick, or the swing hint would outrank a door the player is standing at",
);
check(
  /terrain\.cellEntry\(/.test(mainSrc) && /cx << 16\) \| cz/.test(mainSrc),
  "main.js's swing world does not read terrain.cellEntry at cx<<16|cz — that packing is gather::cell_key and" +
    " terrain.js's cellIndex agreeing, and a different one names another cell",
);
check(
  /cx >= 0 && cz >= 0 && cx <= 0xffff && cz <= 0xffff/.test(mainSrc),
  "main.js's cell key is formed without bounding cx/cz — a negative cell off the island shifts into a" +
    " positive key naming a real cell somewhere else, which is the twelve-bit box_key trap this client has" +
    " already paid for once",
);
// The cone constant, as a LITERAL in the shipped file. Every check above reads
// `interact.GATHER_CONE_COS` as a value, so computing it with `Math.cos` would
// satisfy all of them while producing a different number than the one
// `gather.rs` authored offline — and the cone would then disagree with the
// sim's at exactly the edge where a cone is ever wrong.
check(
  /export const GATHER_CONE_COS = [\d.]+;/.test(interactSrc) && !/Math\.(cos|acos)/.test(interactSrc),
  "interact.js computes its cone cosine rather than carrying gather.rs's authored literal — the sim runs no" +
    " trig on purpose (fmath's whole discipline), and a recomputed cosine is a second source for one number",
);

// --- the bearing both resolvers are judged on --------------------------------
// Every constant above is pinned against Rust. The one INPUT was not, and that
// is the hole: `gather::swing` does not resolve on the yaw the player holds. It
// runs `yaw_dir(p.frame.yaw)` = `YAW_LUT_BITS[yaw >> 8]` — 256 bearings,
// 360/256 = 1.40625 deg apart — and the 30 deg cone, the point-blank bypass and
// nearest-wins are all computed against THAT vector. A client judging the same
// cone on `Math.sin(input.yaw)` predicts the sim's pick from a bearing the sim
// will never use, by up to half a quantum: 0.703 deg. A node just inside the
// cone edge on the client's bearing is just outside it on the sim's.
//
// This is CLAUDE.md's "quantize both sides or prediction drifts by rounding —
// the server sims on the values it transmits", in its hint-shaped form, and
// nothing else in the repo can see it: the client sends a swing as a button
// bit, so no packet carries its opinion of the target. The protocol golden, the
// replay hash and clippy are all untouched by a wrong bearing.
//
// So the client's table is held to the Rust one BY VALUE, all 256 entries, as
// f32 bit patterns with NO tolerance — including no half-ulp one hidden inside
// the comparison, see the `fround` note in the loop. That is achievable rather
// than merely desirable: `yaw_lut.rs` is `ci/gen_yaw_lut.py`'s sin/cos rounded
// to f32, and `Float32Array` rounds identically — measured at 0 ulp across all
// 512 values. A tolerance would let a convention flip hide inside it (sin/cos
// swapped, index 0 facing +X, rotation reversed), and a swapped pair is
// precisely the positional-payload class this file exists for.
const inputMod = await import(pathToFileURL(path.join(root, "web/src/input.js")).href);
const yawLutSrc = fs.readFileSync(path.join(root, "crates/sim-core/src/yaw_lut.rs"), "utf8");
const rsLut = [];
for (
  let re = /\[(0x[0-9A-Fa-f]{8}), (0x[0-9A-Fa-f]{8})\], \/\/\s*(\d+)$/gm, m = re.exec(yawLutSrc);
  m;
  m = re.exec(yawLutSrc)
) {
  rsLut[Number(m[3])] = [parseInt(m[1], 16) >>> 0, parseInt(m[2], 16) >>> 0];
}
// Counted with an explicit loop, not `every`: `Array.prototype.every` SKIPS
// holes, so a sparse parse would sail through it reporting length 256 while
// comparing nothing — the gate-that-matches-nothing class, one level subtler.
let rsLutCount = 0;
for (let i = 0; i < 256; i++) if (Array.isArray(rsLut[i])) rsLutCount++;
check(
  rsLutCount === 256 && inputMod.YAW_BEARINGS === 256,
  `parsed ${rsLutCount} of 256 bearings out of yaw_lut.rs against input.js's YAW_BEARINGS =` +
    ` ${inputMod.YAW_BEARINGS} — a table this gate cannot read compares nothing at all`,
);
const f32bits = (() => {
  const buf = new ArrayBuffer(4);
  const f = new Float32Array(buf);
  const u = new Uint32Array(buf);
  return (x) => {
    f[0] = x;
    return u[0] >>> 0;
  };
})();
const yawOut = { fx: 0, fz: 0 };
let lutBad = 0;
let lutFirst = "";
for (let i = 0; i < rsLutCount; i++) {
  inputMod.yawDir(i << 8, yawOut);
  const gx = f32bits(yawOut.fx);
  const gz = f32bits(yawOut.fz);
  // `f32bits` ROUNDS to f32, so comparing bits alone would accept an f64 table
  // whose values merely round to Rust's — a half-ulp tolerance smuggled in by
  // the comparison itself. `fround(v) === v` closes it: the shipped value has
  // to already BE the f32, which is what makes "bit-exact" a fact and not a
  // description of the assertion's own rounding.
  const isF32 = Math.fround(yawOut.fx) === yawOut.fx && Math.fround(yawOut.fz) === yawOut.fz;
  if (isF32 && gx === rsLut[i][0] && gz === rsLut[i][1]) continue;
  if (!lutBad) {
    lutFirst =
      `first at index ${i}: input.js (0x${gx.toString(16)}, 0x${gz.toString(16)})` +
      ` vs yaw_lut.rs (0x${rsLut[i][0].toString(16)}, 0x${rsLut[i][1].toString(16)})`;
  }
  lutBad++;
}
check(
  lutBad === 0 && rsLutCount === 256,
  `${lutBad} of ${rsLutCount} bearings differ from sim-core's yaw_lut.rs — ${lutFirst}. Every aim cone in the` +
    " client would then be resolved on a direction the sim does not swing on, and no wire golden, replay hash" +
    " or clippy wall can see the difference",
);
// Only bits 8..15 carry the direction. The sim indexes `yaw >> 8` and discards
// the low byte outright, so the low byte must not move the client's bearing
// either — otherwise the aim is continuous again, merely in smaller steps —
// and the next index up must move it, or the table is degenerate.
const loA = inputMod.yawDir(0x1234, { fx: 0, fz: 0 });
const loB = inputMod.yawDir(0x12ff, { fx: 0, fz: 0 });
const loC = inputMod.yawDir(0x1300, { fx: 0, fz: 0 });
check(
  loA.fx === loB.fx && loA.fz === loB.fz && (loC.fx !== loA.fx || loC.fz !== loA.fz),
  `yawDir let the low byte of the wire yaw move the bearing, or the next index did not move it —` +
    ` 0x1234 -> (${loA.fx}, ${loA.fz}), 0x12ff -> (${loB.fx}, ${loB.fz}), 0x1300 -> (${loC.fx}, ${loC.fz}).` +
    " The sim reads yaw >> 8; a client reading more or less than that is quantized to a different grid",
);
// The METHOD the call sites use, against the FUNCTION the 256-entry table pins.
// These can drift — `aimDir` could quantize a different yaw (a stale copy, the
// render view's rather than the tracker's) and every check above would stay
// green. Built with `Object.create` off the prototype rather than `new`: the
// real methods and the real prototype chain, without the constructor, which
// wants a canvas and a document this gate's page never gives it.
const proto = inputMod.InputTracker.prototype;
const yawCases = [0, 0.001, 0.7, Math.PI / 2, Math.PI, 4.2, 6.28, -0.9, 123.456];
let aimBad = 0;
for (const yaw of yawCases) {
  const self = Object.assign(Object.create(proto), { yaw, pitch: 0 });
  const viaMethod = self.aimDir({ fx: 0, fz: 0 });
  const viaFn = inputMod.yawDir(self.yawU16(), { fx: 0, fz: 0 });
  if (viaMethod.fx !== viaFn.fx || viaMethod.fz !== viaFn.fz) aimBad++;
}
check(
  aimBad === 0,
  `aimDir disagrees with yawDir(yawU16()) at ${aimBad} of ${yawCases.length} yaws — the method is what main.js` +
    " calls and the function is what the table above pins, so a drift between the two is ungated on both sides",
);
// The wiring. The quantum is worth nothing if the resolvers are still fed the
// free-running value, and both call sites have to move together: E's pick and
// the swing's are separate resolvers reading the same tracker.
check(
  /input\.aimDir\(interactAim\)/.test(mainSrc) && /input\.aimDir\(swingAim\)/.test(mainSrc),
  "main.js does not feed BOTH resolvers through input.aimDir — whichever one still computes its own look" +
    " direction is judged on a bearing gather.rs and deploy.rs never see",
);
check(
  !/(interactAim|swingAim)\.f[xz] = /.test(mainSrc),
  "main.js still assigns a resolver's fx/fz directly — that is where the free-running Math.sin(input.yaw) was," +
    " and input.aimDir is what replaced it; an assignment beside the call would silently win by running second",
);
// And the one aim that must KEEP the free-running yaw, stated so a later pass
// does not "fix" it by symmetry: `buildTarget` computes a grid address the
// client SENDS — `client_action_place(row, cx, cz, level, loc)`. The sim does
// not re-derive that cell from yaw, so the client's opinion IS the payload and
// there is no second value for it to agree with. Quantizing there would only
// coarsen the preview to 1.4 deg steps for no one's benefit. The exemption
// rests on both halves, so both are checked: if either changes it needs
// deciding again rather than inheriting.
check(
  /R\[1\] \+ Math\.sin\(input\.yaw\)/.test(mainSrc) &&
    /client_action_place\(row, cx, cz, level, loc\)/.test(mainSrc),
  "buildTarget no longer aims with the free-running yaw, or placement no longer sends its own cell — those" +
    " two facts together are why placement is exempt from the quantum above, and neither may lapse silently",
);

// =============================================================================
// S. the compass — the bearing readout, and the axis it rests on
// =============================================================================
// The judge's ranked gap 3 (pass-20260805-063306) asked for "one bearing
// readout so the island has a direction in it". What makes it gateable rather
// than decorative is that a compass is an ARITHMETIC claim in two halves that
// no byte-golden can see:
//
//   1. the strip's pixel offset against the wire yaw, and
//   2. **which world axis is north** — a claim `hud.js` makes and nothing else
//      in the repo states. `scene.js:56` and `browser_smoke.mjs:282` both read
//      +Z as north; `build.rs:54-57` names low-z "north" for its build grid and
//      disagrees. The compass follows the client/CI reading, and §T below
//      pins it to `yawDir` so the two cannot drift apart in silence.
//
// This is the `EV_*` positional-payload class from `CLAUDE.md`'s trap list,
// wearing a `<div>`: swap a sign or drop the half-window offset and every
// existing gate stays green while the player is told east is west.
const compassConsts = await page.evaluate(async () => {
  const m = await import("/src/hud.js");
  return {
    fov: m.COMPASS_FOV_DEG,
    px: m.COMPASS_PX_PER_DEG,
    step: m.COMPASS_STEP_DEG,
    span: m.COMPASS_SPAN_DEG,
    card: m.COMPASS_CARD_DEG,
    cardinals: m.COMPASS_CARDINALS,
  };
});
for (const [k, v] of Object.entries(compassConsts)) {
  if (k === "cardinals") continue;
  check(
    typeof v === "number" && Number.isFinite(v) && v > 0,
    `hud.js no longer exports COMPASS_* constant '${k}' as a positive number (got ${v}) —` +
      " this whole section reads its expectations out of them, and an undefined here would make" +
      " every comparison below compare NaN and pass nothing",
  );
}
// `COMPASS_CARD_DEG` is written as a literal 45 so `ci/knob_registry.mjs` can
// read it off the source and pin it to DECISIONS.md. This is the derivation
// that literal stands for, kept as an assertion so the number and the table
// still cannot drift apart — adding a ninth cardinal without changing the
// stride, or vice versa, is red here.
check(
  Array.isArray(compassConsts.cardinals) && compassConsts.cardinals.length === 8,
  `hud.js exports ${compassConsts.cardinals?.length} cardinals, expected 8 (N NE E SE S SW W NW)`,
);
check(
  compassConsts.card * compassConsts.cardinals.length === 360,
  `COMPASS_CARD_DEG (${compassConsts.card}) * ${compassConsts.cardinals.length} cardinals =` +
    ` ${compassConsts.card * compassConsts.cardinals.length}, expected 360 — the stride and the` +
    " table disagree, so some cardinal is drawn at a bearing that is not its own",
);
check(
  compassConsts.cardinals[0] === "N" && compassConsts.cardinals[2] === "E",
  `the cardinal table starts ${JSON.stringify(compassConsts.cardinals.slice(0, 3))} — index is` +
    " bearing/45, so index 0 must be N and index 2 must be E for +Z north / +X east to hold",
);
check(
  compassConsts.span === 360 + compassConsts.fov,
  `COMPASS_SPAN_DEG is ${compassConsts.span}, expected 360 + COMPASS_FOV_DEG = ${360 + compassConsts.fov}` +
    " — the band must be one full turn PLUS one window or setBearing's unconditional write runs off an end",
);
check(
  compassConsts.card === 45,
  `COMPASS_CARD_DEG is ${compassConsts.card}, expected 45 (eight cardinals over 360)`,
);

// --- the scaffold, and CSS agreeing with the constants ----------------------
// `index.html` restates the two widths in px because CSS cannot read a JS
// constant. That restatement is the drift risk the header of this file keeps
// naming, so it is asserted rather than trusted.
const compassBox = await page.evaluate(() => {
  const win = document.getElementById("compass");
  const band = document.getElementById("compassband");
  const now = document.getElementById("compassnow");
  const cs = win ? getComputedStyle(win) : null;
  return {
    haveWin: !!win,
    haveBand: !!band,
    haveNow: !!now,
    winShown: win ? win.style.display : null,
    nowShown: now ? now.style.display : null,
    overflow: cs ? cs.overflowX : null,
    winW: win ? win.clientWidth : 0,
    bandW: band ? band.offsetWidth : 0,
    marginLeft: cs ? parseFloat(cs.marginLeft) : 0,
    scrollMax: win ? win.scrollWidth - win.clientWidth : 0,
  };
});
check(compassBox.haveWin, "#compass is missing from index.html");
check(compassBox.haveBand, "#compassband is missing from index.html");
check(compassBox.haveNow, "#compassnow (the fixed centre index) is missing from index.html");
// Inline, not computed — the same distinction group F draws: `show()` is what
// puts the strip on screen, and a stylesheet default would hide that lapsing.
check(
  compassBox.winShown === "block",
  `Hud.show() left #compass at inline display:'${compassBox.winShown}', expected 'block'`,
);
check(
  compassBox.nowShown === "block",
  `Hud.show() left #compassnow at inline display:'${compassBox.nowShown}', expected 'block'`,
);
check(
  compassBox.overflow === "hidden",
  `#compass overflow-x is '${compassBox.overflow}', expected 'hidden' — the window must CLIP the band,` +
    " and a visible overflow would both show the whole 450-degree strip and make scrollLeft a no-op",
);
check(
  compassBox.winW === compassConsts.fov * compassConsts.px,
  `#compass is ${compassBox.winW}px wide; COMPASS_FOV_DEG * COMPASS_PX_PER_DEG =` +
    ` ${compassConsts.fov * compassConsts.px}px. The CSS width and the JS constants disagree`,
);
check(
  compassBox.bandW === compassConsts.span * compassConsts.px,
  `#compassband is ${compassBox.bandW}px wide; COMPASS_SPAN_DEG * COMPASS_PX_PER_DEG =` +
    ` ${compassConsts.span * compassConsts.px}px. The CSS width and the JS constants disagree`,
);
check(
  compassBox.marginLeft === -compassBox.winW / 2,
  `#compass margin-left is ${compassBox.marginLeft}px, expected ${-compassBox.winW / 2}px —` +
    " left:50% plus a half-width negative margin is what centres it; off-centre, every bearing below is off",
);
// The whole point of a band one window longer than a turn: a full revolution
// of scroll is reachable. `>=` and not `===` because the end ticks are 40px
// boxes centred on their bearing, so the last one overhangs the band by half
// its width and the browser counts that in scrollWidth. The EXACT arithmetic
// is asserted per-bearing in §T, including the yaw that lands on the maximum.
check(
  compassBox.scrollMax >= 360 * compassConsts.px,
  `#compass can scroll ${compassBox.scrollMax}px; one full turn needs ${360 * compassConsts.px}px.` +
    " A shorter range means some bearing cannot be reached and the strip clamps instead of turning",
);

// --- the ticks: two mark sets, absolute-aligned ------------------------------
const ticks = await page.evaluate(() =>
  [...document.querySelectorAll("#compassband .ctick")].map((t) => ({
    left: parseFloat(t.style.left),
    text: t.textContent,
    card: t.classList.contains("card"),
  })),
);
// 51 = numbers every 10 over [-40, 400] (45) + letters every 45 over
// [-45, 405] (11) − the five multiples of 90 that carry both. Pinned as a
// literal on purpose: a change to COMPASS_FOV_DEG or either stride SHOULD
// stop here and be recounted, because the two strides are what the reference
// frame was measured for.
check(
  ticks.length === 51,
  `the compass band built ${ticks.length} ticks, expected 51 — recount if COMPASS_FOV_DEG` +
    ", COMPASS_STEP_DEG or COMPASS_CARD_DEG moved",
);
check(
  ticks.filter((t) => t.card).length === 11,
  `${ticks.filter((t) => t.card).length} ticks carry .card, expected 11 (every 45 deg across a 450 deg band)`,
);
// Position is the payload here, exactly as it is for an EV_* field: a tick
// with the right letter at the wrong offset is the failure this catches.
const half = compassConsts.fov / 2;
const tickAt = (bearing) =>
  ticks.find((t) => Math.abs(t.left - (bearing + half) * compassConsts.px) < 0.01);
for (const [bearing, want] of [
  [0, "N"],
  [45, "NE"],
  [90, "E"],
  [135, "SE"],
  [180, "S"],
  [225, "SW"],
  [270, "W"],
  [315, "NW"],
]) {
  const t = tickAt(bearing);
  check(
    !!t && t.text === want && t.card,
    `bearing ${bearing} deg should carry the cardinal '${want}' at band offset` +
      ` ${(bearing + half) * compassConsts.px}px; found ${t ? `'${t.text}' (card=${t.card})` : "no tick"}`,
  );
}
// The duplicated wrap window must agree with its in-range twin, or the strip
// jumps as the player crosses north.
for (const [outside, inside] of [
  [-45, 315],
  [-40, 320],
  [360, 0],
  [405, 45],
]) {
  const a = tickAt(outside);
  const b = tickAt(inside);
  check(
    !!a && !!b && a.text === b.text,
    `the wrap tick at ${outside} deg reads '${a ? a.text : "missing"}' but its twin at ${inside} deg` +
      ` reads '${b ? b.text : "missing"}' — the same heading must carry the same label on both copies`,
  );
}
// The reference draws NE BETWEEN 40 and 50 rather than replacing one of them.
for (const n of [40, 50]) {
  const t = tickAt(n);
  check(
    !!t && t.text === String(n) && !t.card,
    `bearing ${n} deg should still carry its number alongside NE at 45 (crafting/compass reference frame),` +
      ` found ${t ? `'${t.text}'` : "no tick"}`,
  );
}

// =============================================================================
// T. the compass against the wire — the axis claim, pinned to yawDir
// =============================================================================
// `setBearing` takes the WIRE yaw, so its arithmetic is checkable exactly:
// px = round(yaw * 360/65536 * COMPASS_PX_PER_DEG). And the semantic half —
// that the letter under the centre index is the direction the sim will
// actually walk — is pinned by asking `input.js`'s `yawDir` for the same yaw
// and checking the axis. That is the join that makes north one fact.
const bearings = await page.evaluate(async () => {
  const { hud } = globalThis.__ui;
  const { yawDir } = await import("/src/input.js");
  const win = document.getElementById("compass");
  const nowRect = document.getElementById("compassnow").getBoundingClientRect();
  const centreX = nowRect.left + nowRect.width / 2;
  const out = [];
  for (const yaw of [0, 8192, 16384, 24576, 32768, 40960, 49152, 57344, 65535]) {
    hud.setBearing(yaw);
    // Which tick is under the fixed centre index, measured in page space so
    // it accounts for the scroll, the margin and the band offset together.
    let best = null;
    let bestD = Infinity;
    for (const t of document.querySelectorAll("#compassband .ctick")) {
      const r = t.getBoundingClientRect();
      const d = Math.abs(r.left + r.width / 2 - centreX);
      if (d < bestD) {
        bestD = d;
        best = t.textContent;
      }
    }
    const dir = yawDir(yaw, { fx: 0, fz: 0 });
    out.push({ yaw, scrollLeft: win.scrollLeft, centred: best, fx: dir.fx, fz: dir.fz });
  }
  return out;
});
const pxPerU16 = (360 / 65536) * compassConsts.px;
let lastScroll = -1;
for (const b of bearings) {
  const want = Math.round(b.yaw * pxPerU16);
  check(
    b.scrollLeft === want,
    `setBearing(${b.yaw}) left #compass.scrollLeft at ${b.scrollLeft}px, expected ${want}px` +
      ` (yaw * 360/65536 * ${compassConsts.px}). The strip and the wire disagree`,
  );
  // A sign flip is the single most likely defect here and it is invisible to
  // every other assertion in this file.
  check(
    b.scrollLeft >= lastScroll,
    `scrollLeft went backwards (${lastScroll} -> ${b.scrollLeft}) as yaw increased to ${b.yaw} —` +
      " increasing wire yaw turns N->E->S->W, so the strip must scroll one way only",
  );
  lastScroll = b.scrollLeft;
}
// The four quarter-turns, each checked twice: the letter a player reads, and
// the axis `yaw_lut` will walk them down. `yawDir` is the sim's own table
// (input.js re-derives yaw_lut.rs bit-for-bit, asserted elsewhere in this
// file), so this is the compass measured against the sim and not against
// itself.
for (const [yaw, letter, axis] of [
  [0, "N", "fz+"],
  [16384, "E", "fx+"],
  [32768, "S", "fz-"],
  [49152, "W", "fx-"],
]) {
  const b = bearings.find((x) => x.yaw === yaw);
  check(
    b.centred === letter,
    `at wire yaw ${yaw} the centre index reads '${b.centred}', expected '${letter}'`,
  );
  const near = (v, t) => Math.abs(v - t) < 1e-6;
  const ok =
    axis === "fz+"
      ? near(b.fz, 1) && near(b.fx, 0)
      : axis === "fz-"
        ? near(b.fz, -1) && near(b.fx, 0)
        : axis === "fx+"
          ? near(b.fx, 1) && near(b.fz, 0)
          : near(b.fx, -1) && near(b.fz, 0);
  check(
    ok,
    `the compass says '${letter}' at wire yaw ${yaw}, but yawDir walks (fx=${b.fx}, fz=${b.fz}).` +
      " hud.js's COMPASS_DEG_PER_U16 rests on +Z being north and +X east; the sim's direction table" +
      " has moved out from under it (see input.js and crates/sim-core/src/yaw_lut.rs)",
  );
}

// The L8 claim in `setBearing`'s doc — it is the one HUD method on the RAF
// path, and it earns that by not writing when the rounded pixel did not move.
// Driven rather than read: park a sentinel in scrollLeft and check the
// repeat call leaves it alone.
const noWrite = await page.evaluate(() => {
  const { hud } = globalThis.__ui;
  const win = document.getElementById("compass");
  hud.setBearing(20000);
  const settled = win.scrollLeft;
  win.scrollLeft = 0; // the sentinel: a write would overwrite this
  hud.setBearing(20000); // same yaw
  const afterSame = win.scrollLeft;
  hud.setBearing(20001); // 0.0055 deg away — same rounded pixel
  const afterNudge = win.scrollLeft;
  hud.setBearing(30000); // a real move
  return { settled, afterSame, afterNudge, afterMove: win.scrollLeft };
});
check(
  noWrite.settled > 0,
  `setBearing(20000) produced scrollLeft ${noWrite.settled} — expected a nonzero offset, so the` +
    " sentinel below is actually testing something",
);
check(
  noWrite.afterSame === 0,
  `setBearing repeated with the same yaw wrote scrollLeft=${noWrite.afterSame} over the sentinel —` +
    " it must skip the write when the rounded pixel has not moved (L8: nothing spare in the RAF path)",
);
check(
  noWrite.afterNudge === 0,
  `a sub-pixel yaw nudge (20000 -> 20001) wrote scrollLeft=${noWrite.afterNudge} — the guard compares` +
    " the ROUNDED pixel, not the raw yaw, or the RAF path writes every frame a mouse moves",
);
check(
  noWrite.afterMove === Math.round(30000 * pxPerU16),
  `after the sentinel, a real bearing change left scrollLeft at ${noWrite.afterMove}, expected` +
    ` ${Math.round(30000 * pxPerU16)} — the guard must not latch and stop writing altogether`,
);

// And the caller: `main.js` must sample the wire yaw ONCE and hand the same
// value to the bridge and to the compass. Two `yawU16()` calls in one frame
// would reopen the temporal seam input.js names, between two statements.
check(
  /const yaw = input\.yawU16\(\);/.test(mainSrc) && /hud\.setBearing\(yaw\)/.test(mainSrc),
  "main.js no longer samples the wire yaw into a local and passes that same local to both" +
    " client_set_input and hud.setBearing — the compass must read the yaw the wire carried",
);
check(
  (mainSrc.match(/input\.yawU16\(\)/g) || []).length === 1,
  `main.js calls input.yawU16() ${(mainSrc.match(/input\.yawU16\(\)/g) || []).length} times;` +
    " exactly one sample per frame is the point — a second call is a second yaw",
);

// =============================================================================
// U. the map — the island, its grid, and which way is up
// =============================================================================
// The judge's ranked gap 3 (`pass-20260805-074623-01`) opens "There is nowhere
// to go" and leads with `MENUS.md:102`, Map MISSING. `web/src/map.js` is the
// answer and it is almost entirely arithmetic, so almost all of this is node.
//
// Three classes of claim, and they fail in different ways:
//
//   1. The map's constants are the WORLD's. A map drawn at the wrong extent
//      is not slightly wrong, it is a different island, and nothing on screen
//      would say so — the player would simply never find anything. Read back
//      out of `terrain.rs` and `bridge.rs`, the discipline §Q and §R already
//      use for the reach and the cone.
//   2. Orientation. `CLAUDE.md`'s trap list is explicit that a byte-golden is
//      blind to what a field MEANS and that positional payloads are where the
//      reference ecosystem actually bled. `paintMap` writes image row
//      `size-1-j` for height row `j` — one subtraction between "north is up"
//      and "the island is upside down", in a file with no golden at all. So a
//      synthetic field that is land in the north and sea in the south is
//      painted and read back, and the grid labels are checked at all four
//      corners rather than at one.
//   3. The palette is the SPLAT law's, blended and shaded, not a second biome
//      opinion. Flat ground must come out at exactly the palette colour, which
//      is what makes `MAP_SHADE_FLOOR`'s derivation a checkable claim instead
//      of a taste; and a pure weight must produce its own colour, which is
//      what stops the four channels being wired up in the wrong order.
//
// No wasm anywhere, which is why this stays in the code tier: `paintMap` takes
// the sampler from its caller and the fakes below are three arrow functions.
const map = await import(pathToFileURL(path.join(root, "web/src/map.js")).href);

// --- 1. the constants are the world's ---------------------------------------
const rsIsland = Number(terrainSrc.match(/^pub const ISLAND_SIZE: f32 = ([\d.]+);/m)?.[1]);
const rsSea = Number(terrainSrc.match(/^pub const SEA_LEVEL: f32 = ([\d.]+);/m)?.[1]);
const rsCells = Number(terrainSrc.match(/^pub const CELLS_PER_SIDE: i32 = (\d+);/m)?.[1]);
const rsCellSize = Number(terrainSrc.match(/^pub const CELL_SIZE: f32 = ([\d.]+);/m)?.[1]);
check(
  Number.isFinite(rsIsland) && Number.isFinite(rsSea) && Number.isInteger(rsCells),
  "could not read ISLAND_SIZE / SEA_LEVEL / CELLS_PER_SIDE out of terrain.rs — every extent check" +
    " below would compare the map against nothing, which is the gate-that-matches-nothing class",
);
check(
  map.WORLD_M === rsIsland,
  `map.js WORLD_M = ${map.WORLD_M}, terrain.rs ISLAND_SIZE = ${rsIsland} — the map would paint a` +
    " different island from the one the player walks on, and every grid reference on it would be wrong",
);
check(
  map.SEA_LEVEL === rsSea,
  `map.js SEA_LEVEL = ${map.SEA_LEVEL}, terrain.rs SEA_LEVEL = ${rsSea} — the coastline is the one` +
    " line on this screen a player navigates by",
);
check(
  map.MAP_N === rsCells,
  `map.js MAP_N = ${map.MAP_N}, terrain.rs CELLS_PER_SIDE = ${rsCells} — one map sample is meant to be` +
    " one scatter cell, so the map cannot claim detail the world does not have",
);
check(
  map.MAP_N * (map.WORLD_M / map.MAP_N) === map.WORLD_M &&
    map.WORLD_M / map.MAP_N === rsCellSize,
  `map.js samples every ${map.WORLD_M / map.MAP_N} m, terrain.rs CELL_SIZE = ${rsCellSize}`,
);
// The one-call claim, and it is load-bearing: `terrain_fill_heights` refuses a
// side above HEIGHTS_MAX_N by RETURNING 0 and leaving the scratch buffer as it
// was, so a MAP_N raised past the cap would paint a stale buffer rather than
// throw. main.js checks the count; this pins the inequality that makes the
// check pass, so raising MAP_N lands red HERE on the commit that raises it.
const rsHeightsMax = Number(bridgeSrc.match(/^const HEIGHTS_MAX_N: usize = (\d+);/m)?.[1]);
check(
  Number.isInteger(rsHeightsMax),
  "could not read HEIGHTS_MAX_N out of bridge.rs — the apron check below would compare against nothing",
);
check(
  map.MAP_N + 2 <= rsHeightsMax,
  `map.js MAP_N = ${map.MAP_N} needs a ${map.MAP_N + 2}-square fill (one-sample apron) but` +
    ` bridge.rs HEIGHTS_MAX_N = ${rsHeightsMax}. terrain_fill_heights REFUSES above the cap and returns 0` +
    " without touching the buffer, so the map would paint whatever the last caller left there",
);
check(
  /if \(wrote !== g \* g\)/.test(mainSrc),
  "main.js no longer checks terrain_fill_heights' returned count against g*g — a refusal returns 0 and" +
    " leaves the previous fill in the shared scratch buffer, which paints as a map of somewhere else",
);
// The grid divides the world exactly, and every column has a letter. A
// GRID_LETTERS one short of GRID_COLS would put `undefined` in the last
// column's label rather than fail anywhere.
check(
  map.GRID_COLS === map.WORLD_M / map.MAP_GRID_M && Number.isInteger(map.GRID_COLS),
  `map.js GRID_COLS = ${map.GRID_COLS} does not divide WORLD_M ${map.WORLD_M} by MAP_GRID_M` +
    ` ${map.MAP_GRID_M} exactly — a partial square on two edges has no honest label`,
);
check(
  map.GRID_LETTERS.length === map.GRID_COLS,
  `map.js GRID_LETTERS has ${map.GRID_LETTERS.length} letters for ${map.GRID_COLS} columns — the short` +
    " end would label its squares 'undefined13'",
);

// --- 2. orientation: north is up, and the corners prove it -------------------
// Both halves of the flip are checked, because they are separate subtractions
// in separate functions and either one alone would be a map that disagrees
// with its own labels.
for (const [x, z, want, where] of [
  [1, map.WORLD_M - 1, "A1", "north-west"],
  [map.WORLD_M - 1, map.WORLD_M - 1, "P1", "north-east"],
  [1, 1, "A16", "south-west"],
  [map.WORLD_M - 1, 1, "P16", "south-east"],
]) {
  const got = map.gridLabel(x, z);
  check(
    got === want,
    `map.js gridLabel(${x}, ${z}) — the ${where} corner — is '${got}', expected '${want}'.` +
      " Columns run A..P west to east and rows 1..16 NORTH to south (+Z is north, DECISIONS.md §open" +
      " compass axes v0), which is the same axis the compass strip rests on",
  );
}
check(
  map.gridLabel(-1, 5) === "" && map.gridLabel(5, map.WORLD_M) === "",
  "map.js gridLabel answers a label for a position off the island — a reference to a square that does" +
    " not exist is worse than no reference",
);
// The southern and eastern edges are exactly on a boundary and must land in
// the LAST square rather than one past it.
check(
  map.gridLabel(map.WORLD_M - 0.001, 0) === "P16",
  `map.js gridLabel at the exact south-east corner is '${map.gridLabel(map.WORLD_M - 0.001, 0)}',` +
    " expected 'P16' — z = 0 floors to one row past the last",
);
// The projection agrees with the labels: +z must move UP the image (py down).
const mp = {};
map.worldToMap(mp, 0, map.WORLD_M, 100);
check(
  mp.px === 0 && mp.py === 0,
  `map.js worldToMap of the north-west corner is (${mp.px}, ${mp.py}), expected (0, 0) — image y grows` +
    " SOUTH while +z is north",
);
map.worldToMap(mp, map.WORLD_M, 0, 100);
check(
  mp.px === 100 && mp.py === 100,
  `map.js worldToMap of the south-east corner is (${mp.px}, ${mp.py}), expected (100, 100)`,
);
const mw = {};
map.mapToWorld(mw, 25, 25, 100);
map.worldToMap(mp, mw.x, mw.z, 100);
check(
  Math.abs(mp.px - 25) < 1e-9 && Math.abs(mp.py - 25) < 1e-9,
  `map.js worldToMap(mapToWorld(25, 25)) came back (${mp.px}, ${mp.py}) — the two projections are` +
    " inverses or the marker sits somewhere the island is not",
);

// The painted image itself. Land in the north half, sea in the south half; if
// `paintMap`'s row flip is dropped or inverted, this comes back upside down.
const PAINT_N = 16;
const paintProbe = (heightAt, splat) => {
  const g = PAINT_N + 2;
  const H = new Float32Array(g * g);
  // j grows with +z (north), exactly what terrain_fill_heights writes.
  for (let j = 0; j < g; j++) {
    for (let i = 0; i < g; i++) H[j * g + i] = heightAt(i - 1, j - 1);
  }
  const out = new Uint8ClampedArray(PAINT_N * PAINT_N * 4);
  map.paintMap(out, PAINT_N, {
    heights: H,
    step: 1,
    x0: 0,
    z0: 0,
    moistAt: () => 0,
    splatAt: () => splat,
  });
  return (i, py) => [
    out[(py * PAINT_N + i) * 4],
    out[(py * PAINT_N + i) * 4 + 1],
    out[(py * PAINT_N + i) * 4 + 2],
    out[(py * PAINT_N + i) * 4 + 3],
  ];
};
// Pure grass (channel 1 of sand/grass/litter/rock, little end first).
const GRASS_ONLY = 0x0000ff00;
const northLand = paintProbe((_i, j) => (j >= PAINT_N / 2 ? 20 : -20), GRASS_ONLY);
const top = northLand(8, 0);
const bottom = northLand(8, PAINT_N - 1);
check(
  top[0] === map.MAP_GRASS[0] && top[1] === map.MAP_GRASS[1] && top[2] === map.MAP_GRASS[2],
  `map.js paintMap put (${top.slice(0, 3)}) at the TOP of the image for a field that is land in the` +
    ` north and sea in the south; expected the grass colour (${map.MAP_GRASS}). The image is upside` +
    " down: height row j grows with +z (NORTH) and image row py grows DOWNWARD (south), so py = size-1-j",
);
check(
  bottom[2] > bottom[0] && bottom[2] > bottom[1],
  `map.js paintMap put (${bottom.slice(0, 3)}) at the BOTTOM of that same field — the southern half is` +
    " sea and sea is blue-dominant. Same flip, seen from the other end",
);
// Alpha. An ImageData with a zero alpha anywhere is a hole in the island and
// nothing else in this file would notice.
let opaque = true;
for (let py = 0; py < PAINT_N && opaque; py++) {
  for (let i = 0; i < PAINT_N; i++) if (northLand(i, py)[3] !== 255) opaque = false;
}
check(opaque, "map.js paintMap left a non-opaque pixel — the map would ghost the world through itself");

// --- 2b. the sample grid IS the pixel grid ----------------------------------
// The judge's ranked fix 1 on `pass-20260805-074623-02`. `paintMap` flips rows
// by sample INDEX (`py = size - 1 - j`), `worldToMap` flips by continuous
// EXTENT, and until this block nothing held the two together: the checks above
// pin each formula on its own and both were, separately, correct.
//
// The report measured the disagreement at exactly one row, always, and noted
// that the x axis came out exact. That asymmetry is the tell, and it points
// past both formulas to the grid underneath them. Sampling from 0 put sample
// (i, j) on the low-x, low-z CORNER of the pixel it fills instead of in the
// middle of it, so the island was painted half a cell out on BOTH axes; on the
// flipped axis that half cell lands `worldToMap` exactly on the boundary
// between two rows, and `floor` takes the row to the south. On x the same
// offset lands on the low edge of the column, where `floor` comes out right —
// right by luck of the half-open interval, not by construction, which is why
// only one axis showed a defect.
//
// So the fix was neither formula. It is the origin, and the origin belongs to
// `main.js`: `paintMap` is handed a sampler and cannot see where it sampled.
// That is why the first half of this block reads the caller's source. A gate
// that carried its own `step / 2` would agree with itself forever while the
// shipped map went back to painting a half cell out — the gate-that-matches-
// nothing class, and the exact hole this block exists to close.
const mapOrig = mainSrc.match(/const orig = ([^;]+);/)?.[1]?.trim();
check(
  mapOrig === "step / 2",
  `main.js paints the island from a sample origin of '${mapOrig}', expected 'step / 2' — the samples` +
    " must sit at the CENTRES of the pixels they fill. An origin of 0 puts them on the corners, which" +
    " paints the island half a cell out on both axes and puts worldToMap exactly on a row boundary",
);
const mapFillArgs = mainSrc.match(
  /terrain_fill_heights\(\s*seed,\s*([^,]+),\s*([^,]+),\s*g,\s*step\s*\)/,
);
check(
  mapFillArgs &&
    mapFillArgs[1].trim() === "orig - step" &&
    mapFillArgs[2].trim() === "orig - step",
  `main.js fills the map's heights from (${mapFillArgs?.[1]?.trim()}, ${mapFillArgs?.[2]?.trim()}),` +
    " expected ('orig - step', 'orig - step') — the one-sample apron is one step BELOW the origin on" +
    " both axes, and an apron that disagrees with the origin shifts the whole island by a pixel",
);
const mapPaintSrc = mainSrc.match(/paintMap\(rgba, size, \{([\s\S]*?)\n\s*\}\);/)?.[1] ?? "";
check(
  /\bx0:\s*orig,/.test(mapPaintSrc) && /\bz0:\s*orig,/.test(mapPaintSrc),
  "main.js hands paintMap an x0/z0 that is not the `orig` it filled from — paintMap uses those to ask" +
    " the splat law where it is, so the colours would come from a different place than the heights",
);

// And the arithmetic that origin buys, swept over every row and every column
// rather than sampled at one. A field that is land in exactly ONE sample row is
// painted and read back, and the row it actually landed in must be both the
// index flip AND the floor of the projection — which is the whole claim, in one
// equality, in the only place the two conventions meet.
const CELL_M = map.WORLD_M / PAINT_N;
const CELL_ORIG = CELL_M / 2;
const bandProbe = (isLand) => {
  const g = PAINT_N + 2;
  const H = new Float32Array(g * g);
  for (let j = 0; j < g; j++) {
    for (let i = 0; i < g; i++) H[j * g + i] = isLand(i - 1, j - 1) ? 20 : -20;
  }
  const out = new Uint8ClampedArray(PAINT_N * PAINT_N * 4);
  map.paintMap(out, PAINT_N, {
    heights: H,
    step: CELL_M,
    x0: CELL_ORIG,
    z0: CELL_ORIG,
    moistAt: () => 0,
    splatAt: () => GRASS_ONLY,
  });
  // Land is grass here and grass is green-dominant; water is blue-dominant at
  // every depth. One comparison, so this reads the image without a second copy
  // of the palette that the palette section below could then drift from.
  const rows = new Set();
  const cols = new Set();
  for (let py = 0; py < PAINT_N; py++) {
    for (let i = 0; i < PAINT_N; i++) {
      const o = (py * PAINT_N + i) * 4;
      if (out[o + 1] > out[o + 2]) {
        rows.add(py);
        cols.add(i);
      }
    }
  }
  return { rows: [...rows], cols: [...cols] };
};
const mps = {};
for (let j = 0; j < PAINT_N; j++) {
  const { rows } = bandProbe((_i, jj) => jj === j);
  const zj = CELL_ORIG + j * CELL_M;
  map.worldToMap(mps, 0, zj, PAINT_N);
  check(
    rows.length === 1 && rows[0] === PAINT_N - 1 - j && Math.floor(mps.py) === rows[0],
    `map.js: a field that is land only in sample row ${j} painted rows [${rows}] — expected exactly` +
      ` [${PAINT_N - 1 - j}] — while worldToMap projects that row's own world z (${zj}) to py` +
      ` ${mps.py}, which floors to ${Math.floor(mps.py)}. The painted row and the projected row must` +
      " be the same row or the marker sits off the ground it claims to be standing on",
  );
  check(
    Math.abs(mps.py - Math.floor(mps.py) - 0.5) < 1e-9,
    `map.js: sample row ${j} projects to py ${mps.py}, which is ${mps.py - Math.floor(mps.py)} of the` +
      " way into its pixel, expected exactly 0.5. A sample lands in the MIDDLE of the pixel it fills;" +
      " 0 means the samples went back to the pixel corners and floor is one rounding from wrong",
  );
}
for (let i = 0; i < PAINT_N; i++) {
  const { cols } = bandProbe((ii) => ii === i);
  const xi = CELL_ORIG + i * CELL_M;
  map.worldToMap(mps, xi, 0, PAINT_N);
  check(
    cols.length === 1 && cols[0] === i && Math.floor(mps.px) === i,
    `map.js: a field that is land only in sample column ${i} painted columns [${cols}] — expected` +
      ` exactly [${i}] — while worldToMap projects that column's world x (${xi}) to px ${mps.px}.` +
      " The x axis has no flip and was already exact, and it is swept anyway: it is the axis that" +
      " proves the fix was the shared origin and not a subtraction bolted onto the flipped one",
  );
  check(
    Math.abs(mps.px - Math.floor(mps.px) - 0.5) < 1e-9,
    `map.js: sample column ${i} projects to px ${mps.px}, expected exactly half a pixel into its own` +
      " column — the same centre law as the rows, on the axis whose defect floor was hiding",
  );
}
// The round trip the two now close, at a pixel's own centre rather than its
// corner — which is what `mapToWorld`'s doc claims and what nothing checked.
map.mapToWorld(mw, 7 + 0.5, PAINT_N - 1 - 3 + 0.5, PAINT_N);
check(
  Math.abs(mw.x - (CELL_ORIG + 7 * CELL_M)) < 1e-9 &&
    Math.abs(mw.z - (CELL_ORIG + 3 * CELL_M)) < 1e-9,
  `map.js mapToWorld of the centre of the pixel holding sample (7, 3) is (${mw.x}, ${mw.z}),` +
    ` expected (${CELL_ORIG + 7 * CELL_M}, ${CELL_ORIG + 3 * CELL_M}) — the world position of that` +
    " very sample. This is the inverse stated as the paint's own geometry rather than as a round trip" +
    " through an arbitrary pixel, which two consistently-wrong projections would also pass",
);

// --- 3. the palette is the splat law's --------------------------------------
// Flat ground: lambert is exactly MAP_LIGHT[1] and the shade term is therefore
// exactly 1, so the palette colour survives untouched. That is what makes the
// derivation in MAP_SHADE_FLOOR's doc a claim this gate can hold.
check(
  Math.abs(map.MAP_SHADE_FLOOR + map.MAP_SHADE_GAIN * map.MAP_LIGHT[1] - 1) < 1e-9,
  `map.js MAP_SHADE_FLOOR ${map.MAP_SHADE_FLOOR} + MAP_SHADE_GAIN ${map.MAP_SHADE_GAIN} * lambert-on-flat` +
    ` ${map.MAP_LIGHT[1]} = ${map.MAP_SHADE_FLOOR + map.MAP_SHADE_GAIN * map.MAP_LIGHT[1]}, expected 1.` +
    " Flat ground must come out at the measured palette colour or the palette is a starting point, not a claim",
);
check(
  Math.abs(Math.hypot(...map.MAP_LIGHT) - 1) < 1e-6,
  `map.js MAP_LIGHT is not a unit vector (length ${Math.hypot(...map.MAP_LIGHT)}) — the lambert term` +
    " would scale the whole map by its length and the flat-ground identity above would be a coincidence",
);
check(
  map.MAP_LIGHT[0] < 0 && map.MAP_LIGHT[2] > 0 && map.MAP_LIGHT[1] > 0,
  `map.js MAP_LIGHT ${map.MAP_LIGHT} does not point down-and-from-the-north-west (-x, +y, +z) — every` +
    " printed relief map lights from the upper left and `Rust Images/mapraw.jpg` does too; the other" +
    " direction reads as valleys where there are ridges",
);
// Each of the four channels paints its own colour. Wiring the unpack in the
// wrong order is invisible to every check above — the island would still have
// the right shape, in the wrong biomes.
for (const [shift, name, want] of [
  [0, "sand", map.MAP_SAND],
  [8, "grass", map.MAP_GRASS],
  [16, "litter", map.MAP_LITTER],
  [24, "rock", map.MAP_ROCK],
]) {
  const flat = paintProbe(() => 20, (255 << shift) >>> 0)(0, 0);
  check(
    flat[0] === want[0] && flat[1] === want[1] && flat[2] === want[2],
    `map.js paintMap painted (${flat.slice(0, 3)}) for a pure ${name} weight (byte ${shift / 8} of` +
      ` terrain_splat_from, little end first), expected ${want}. The four weight bytes are unpacked in` +
      " the wrong order — the island keeps its shape and changes all its biomes",
  );
}
// Water: below sea level the splat law is not consulted at all (down there it
// describes the sea FLOOR, sand all the way, and a map that painted it would
// draw an island four times too big), and deeper is darker.
const shelf = paintProbe(() => -0.0001, 0x000000ff)(0, 0);
const deep = paintProbe(() => -map.MAP_DEEP_M, 0x000000ff)(0, 0);
check(
  shelf[2] > shelf[0] && deep[2] > deep[0],
  `map.js paintMap painted sea-floor rock (${shelf.slice(0, 3)} / ${deep.slice(0, 3)}) instead of water` +
    " — the splat law describes the ground under the sea and the map must override it, not read it",
);
check(
  deep[0] + deep[1] + deep[2] < shelf[0] + shelf[1] + shelf[2],
  `map.js paintMap made deep water (${deep.slice(0, 3)}) no darker than the shelf (${shelf.slice(0, 3)})` +
    " — the depth ramp is what separates a bay you can swim from open ocean",
);

// --- the DOM half -----------------------------------------------------------
// Four things a browser is actually needed for: the panel is closed at load,
// the toggle drives it, the readout tracks the projection, and the key law
// holds. Everything else above was arithmetic.
const mapDom = await page.evaluate(async ({ probe, yaws, at }) => {
  const { hud } = globalThis.__ui;
  const { MAP_DRAW_PX } = await import("/src/hud.js");
  const el = document.getElementById("map");
  const canvas = document.getElementById("mapcanvas");
  // COMPUTED at load, not inline: the panel is hidden by the stylesheet and
  // only ever gets an inline value from `toggleMap`, so the inline field is
  // empty here and reading it would assert nothing. Same reading `#death`
  // gets in group A, and for the same reason.
  const atLoad = getComputedStyle(el).display;
  // A tiny painted island so `drawMap` has something to blit — the real one
  // comes out of wasm, which this gate deliberately never loads.
  const n = 4;
  const rgba = new Uint8ClampedArray(n * n * 4).fill(200);
  hud.setMapTerrain(rgba, n);
  const opened = hud.toggleMap();
  const openDisplay = el.style.display;
  const refs = [];
  const marks = [];
  for (const [x, z] of probe) {
    hud.setMapView(x, z, 0);
    refs.push(document.getElementById("mapref").textContent);
    // Where drawMap actually put the marker. Read as client state and not off
    // the canvas — `hud.mapPos` is what the triangle was drawn from, and a
    // pixel probe here would be this gate growing the thing it exists not to
    // be. It catches the defect a pixel probe would: a second projection
    // written out beside the island's, which puts the marker somewhere the
    // player is not while the map underneath stays perfectly correct.
    marks.push({ px: hud.mapPos.px, py: hud.mapPos.py });
  }
  // Which way the triangle POINTS, swept, at one fixed position. The loop above
  // reads `mapPos` and nothing else, so the marker's bearing had no assertion
  // at all — the judge's M11 on `pass-20260805-074623-02` hard-coded the
  // rotation to zero (a map whose arrow always claims north) and was the one
  // survivor of eleven mutants. `mapDir` is the direction the triangle was
  // built from, so this reads the drawn heading and not a recomputation of it.
  const heads = [];
  for (const yaw of yaws) {
    hud.setMapView(at[0], at[1], yaw);
    heads.push({ yaw, dx: hud.mapDir.dx, dy: hud.mapDir.dy });
  }
  // Every verb key must be eaten while the map is up; its own two must not.
  //
  // The inventory is FORCED shut first, and that is not tidiness: `eatsKey`
  // answers for the union of the open panels, so with §I's inventory still up
  // every one of these reads true whatever the map does — the check would pass
  // against a `eatsKey` that had never heard of a map. `invWasOpen` is
  // returned so the assertion can prove the probe actually isolated it.
  if (hud.invOpen) hud.toggleInv();
  const invWasOpen = hud.invOpen;
  const eaten = {};
  for (const code of ["KeyE", "KeyU", "KeyL", "KeyB", "KeyG", "KeyM", "Escape"]) {
    eaten[code] = hud.eatsKey(code);
  }
  const closed = hud.toggleMap();
  return {
    atLoad,
    opened,
    openDisplay,
    closedAfter: closed,
    closedDisplay: el.style.display,
    refs,
    marks,
    heads,
    eaten,
    invWasOpen,
    drawPx: MAP_DRAW_PX,
    canvasW: canvas.width,
    canvasH: canvas.height,
  };
}, {
  probe: [
    [10, map.WORLD_M - 10],
    [map.WORLD_M / 2, map.WORLD_M / 2],
    [-5, -5],
  ],
  // North, east, south, west — and one off the cardinals, which is what pins
  // the QUANTUM rather than the four right angles. 8192 is 45 degrees only if
  // a wire unit is 360/65536 of a turn.
  yaws: [0, 16384, 32768, 49152, 8192],
  at: [map.WORLD_M / 2, map.WORLD_M / 2],
});
check(
  mapDom.atLoad === "none",
  `#map has inline display '${mapDom.atLoad}' at load, expected 'none' — a map covering the world on` +
    " the frame a player joins is the same class as the death screen being up before anyone died",
);
check(
  mapDom.opened === true && mapDom.openDisplay === "flex",
  `hud.toggleMap() returned ${mapDom.opened} and left #map at '${mapDom.openDisplay}', expected true and` +
    " 'flex' — main.js drops pointer lock on the returned value, so a toggle that lies leaves the player" +
    " locked to a mouse they cannot use",
);
check(
  mapDom.closedAfter === false && mapDom.closedDisplay === "none",
  `the second hud.toggleMap() returned ${mapDom.closedAfter} at display '${mapDom.closedDisplay}'`,
);
check(
  mapDom.canvasW === mapDom.drawPx && mapDom.canvasH === mapDom.drawPx,
  `#mapcanvas has a ${mapDom.canvasW}x${mapDom.canvasH} backing store against hud.js MAP_DRAW_PX` +
    ` ${mapDom.drawPx} — they must match or the browser resamples what drawMap deliberately did not`,
);
check(
  mapDom.drawPx % map.MAP_N === 0,
  `hud.js MAP_DRAW_PX ${mapDom.drawPx} is not an integer multiple of map.js MAP_N ${map.MAP_N} — a` +
    " nearest-neighbour upscale at a fractional factor gives some output pixels one sample and some two," +
    " which puts a moiré across the coastline",
);
for (const [i, [x, z]] of [
  [10, map.WORLD_M - 10],
  [map.WORLD_M / 2, map.WORLD_M / 2],
  [-5, -5],
].entries()) {
  const label = map.gridLabel(x, z);
  const want = label ? `grid ${label}` : "off the island";
  check(
    mapDom.refs[i] === want,
    `hud.setMapView(${x}, ${z}) left #mapref at '${mapDom.refs[i]}', expected '${want}' — the readout` +
      " must come from map.js's gridLabel and not from a second copy of the grid arithmetic",
  );
  // The marker's own projection, against map.js's. hud.js imports worldToMap
  // rather than owning a copy, and this is what holds that: a second
  // projection written out in drawMap would leave the island right and put
  // the player somewhere they are not, which is worse than no marker.
  map.worldToMap(mp, x, z, mapDom.drawPx);
  check(
    Math.abs(mapDom.marks[i].px - mp.px) < 1e-9 && Math.abs(mapDom.marks[i].py - mp.py) < 1e-9,
    `hud.drawMap put the marker at (${mapDom.marks[i].px}, ${mapDom.marks[i].py}) for world` +
      ` (${x}, ${z}), but map.js worldToMap says (${mp.px}, ${mp.py}) — the marker is projected by` +
      " something other than the projection that painted the island under it",
  );
}
// The marker's heading, by name. Written out as four cardinal vectors rather
// than as `(sin, -cos)` of the same yaw hud.js used: recomputing the formula
// here would agree with any formula drawMap happened to hold, including a
// wrong one, and the claim being made is not "the arithmetic is the arithmetic"
// but "north points UP the image". That is a positional payload one field wide,
// which `CLAUDE.md`'s trap list names as where the reference ecosystem actually
// bled, and it is the assertion M11 walked through.
const R2 = Math.SQRT1_2;
for (const [yaw, wdx, wdy, name, where] of [
  [0, 0, -1, "north", "UP the image — image y grows south while +z is north"],
  [16384, 1, 0, "east", "RIGHT"],
  [32768, 0, 1, "south", "DOWN"],
  [49152, -1, 0, "west", "LEFT"],
  [8192, R2, -R2, "north-east", "up and to the right, at exactly 45 degrees"],
]) {
  const h = mapDom.heads.find((e) => e.yaw === yaw);
  check(
    h && Math.abs(h.dx - wdx) < 1e-9 && Math.abs(h.dy - wdy) < 1e-9,
    `hud.drawMap pointed the marker along (${h?.dx}, ${h?.dy}) at wire yaw ${yaw} (${name}), expected` +
      ` (${wdx}, ${wdy}) — ${where}. The map's arrow and the compass strip's bearing are one fact shown` +
      " twice; an arrow that does not turn is a map that tells a lost player they are facing north",
  );
}
check(
  new Set(mapDom.heads.map((h) => `${h.dx},${h.dy}`)).size === mapDom.heads.length,
  "hud.drawMap drew two different wire yaws along the SAME image direction — the rotation is dropped or" +
    " collapsed, which every check above would still pass if it were pinned to one convenient angle",
);
check(
  mapDom.invWasOpen === false,
  "the map's key-ownership probe ran with the inventory still open — eatsKey answers for the UNION of" +
    " the open panels, so every check below would read true whether or not the map contributed anything",
);
for (const code of ["KeyE", "KeyU", "KeyL", "KeyB", "KeyG"]) {
  check(
    mapDom.eaten[code] === true,
    `hud.eatsKey('${code}') is ${mapDom.eaten[code]} with the map open — every verb below main.js's` +
      " eatsKey question spends something, and a player reading a map is aiming at a 512-pixel panel",
  );
}
for (const code of ["KeyM", "Escape"]) {
  check(
    mapDom.eaten[code] === false,
    `hud.eatsKey('${code}') is ${mapDom.eaten[code]} with the map open — the panel's own toggle and` +
      " close keys are excluded so the answer is self-contained, which is the rule the inventory set",
  );
}
// And the caller: the island is painted ONCE. 66,564 height samples through
// wasm on every open would be a quarter-second stall on a key a player presses
// to orient themselves, which is the moment it can least afford one.
check(
  /if \(islandPainted\) return;/.test(mainSrc) && /islandPainted = true;/.test(mainSrc),
  "main.js no longer guards paintIsland with a once-only flag — the whole-island fill is 66k height" +
    " samples and it is a function of the seed alone",
);

// =============================================================================
// J. no page errors anywhere in the above
// =============================================================================
check(errors.length === 0, `the page reported errors: ${errors.join(" | ")}`);

console.log(
  `  ui smoke: scaffold ${HUD_IDS.length} ids · join form · hotbar 6 cells · composer swallow · ` +
    `chat cap ${CHAT_CAP} · toast cap ${TOAST_CAP} · vitals positional+inline · death answered once · ` +
    `craft gate/×5 · queue index · inventory ${INV_SLOTS} slots positional + eatsKey · ` +
    `move ordering (send-before-draw, address-matched verdict, diff outranks rollback, ${REFUSE_MAX} reasons) · ` +
    "unarmed panel starts no drag · verdict on APPLIED2_MOVE, bit 31 unshared · " +
    "addresses are (kind, slot): foreign verdict refused, rollback stays in its own container · " +
    `outbound marshalling ${JSON.stringify(MOVE_ARG_ORDER)} against bridge.rs, spread at the one call site · ` +
    `container panel: bag ${INV_SLOTS} / box ${BOX_SLOTS} against limits.rs, cross-container drag both ends, ` +
    "close abandons what it cannot resolve, sync applied before the verdict · " +
    `box address cx<<${rsCxShift}|cz<<${rsCzShift}|level&${rsLevelMask} against deploy.rs, disjoint at the ` +
    `limits.rs ceilings (${MAX_BUILD_COORD}/${MAX_BUILD_LEVELS}), off-grid refused, packed at one call site \u00b7 ` +
    `E is one resolver: ${VERB_NAMES.length} verbs each with a prompt and a dispatch branch, ` +
    `archetypes + reach ${rsReach} m against deploy.rs/build.rs, aim beats nearby (underfoot does not trump), ` +
    "the chain's five scans are gone, prompt hidden at load and text-only, door prompt reads open/locked · " +
    `the swing is the other resolver: reach ${rsReachG} m / cone ${rsCone} / dy ${rsDy} / blank ${rsBlank} ` +
    `against gather.rs, occupant ordinals against terrain.rs (rock and stump pass through), 3x3 cell window, ` +
    "strict tie to the first cell in dz-then-dx order, harvested AND mid-fall skipped, drawn only where E " +
    "is silent · " +
    `both resolvers aim on the wire quantum: all ${rsLutCount} bearings bit-exact against yaw_lut.rs, low byte ` +
    "ignored as the sim ignores it, aimDir == yawDir(yawU16()), placement's sent-address aim exempt · " +
    `compass: ${ticks.length} ticks over ${compassConsts.span} deg (letters every ${compassConsts.card}, ` +
    `numbers every ${compassConsts.step}, wrap copies agree), CSS px == JS constants, scrollLeft == ` +
    "yaw*360/65536*px monotonic and unclamped, N/E/S/W centred and axis-checked against yawDir " +
    "(+Z north, +X east), no write on an unchanged pixel, one yawU16() sample per frame · " +
    `map: ${map.WORLD_M} m / ${map.MAP_N} samples / sea ${map.SEA_LEVEL} against terrain.rs, apron fits ` +
    `HEIGHTS_MAX_N ${rsHeightsMax} and the refused-fill count is checked, ${map.GRID_COLS}x${map.GRID_COLS} ` +
    "grid lettered A..P west-east and numbered 1..16 NORTH-south, all four corners, projection inverts, " +
    "north paints to the TOP row, four splat channels each to their own colour, sea overrides the law and " +
    "deepens, flat ground exactly the palette, panel closed at load, toggle drives display, #mapref from " +
    `gridLabel, every verb key eaten and M/Escape not, ${mapDom.drawPx} px an integer multiple of MAP_N, ` +
    "island painted once",
);
console.log(`ui smoke: ${checks} checks passed`);

// Teardown here and not in the `exit` handler. That handler is a trap this
// repo has already paid for (`vantages.mjs`, 2026-08-04): node fires `exit`
// when it runs out of live handles, and the browser connection and the static
// server ARE live handles — so cleanup would wait for an exit that cleanup was
// the only thing that could cause, and a gate that passes and never returns
// reads exactly like a hung one.
try {
  await browser?.close();
} catch {
  /* the checks already passed; a stubborn browser is not a gate failure */
}
cleanup();
process.exit(0);
