#!/usr/bin/env node
// Gate: the interaction surface, in a real browser, with no renderer.
//
// Why this exists. `ci/gates.sh` splits into a code tier and a renderer tier,
// and the renderer tier is the overwhelming majority of the wall clock (a
// single `browser_smoke` run is 8-10 min). Its `renderer_touched` question
// matched `^web/`, so a one-line HUD change — a `<div>` moved, a toast string
// reworded — dragged in `browser_smoke` AND `vantages` and paid ~19 minutes to
// prove that a DOM overlay still overlaid. That is the wrong owner paying the
// cost. This gate is what lets `web/index.html`, `web/src/hud.js` and
// `web/src/input.js` route out of the renderer tier: it asserts the things
// those three files can break, and it runs in seconds.
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
// The scaffold assertions (group A) are the load-bearing half of the tier
// change. `web/index.html` hosts the renderer's canvas and the client's entry
// script alongside the whole HUD; carving it out of the renderer tier is only
// honest if something still asserts that the canvas is present, mounted and
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
// Away from `browser_smoke` (8934) and `vantages` (8971) so a UI gate can run
// beside either without a port fight.
const PORT = Number(process.env.UI_SMOKE_PORT || 8952);

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
check(
  HUD_IDS.length >= 14,
  `parsed only ${HUD_IDS.length} getElementById ids out of hud.js — the scaffold check would be vacuous`,
);
const hudConst = (name) => {
  const m = hudSrc.match(new RegExp(`const ${name} = (\\d+);`));
  if (!m) fail(`hud.js declares no ${name} — this gate reads its caps from the source`);
  return Number(m[1]);
};
const TOAST_CAP = hudConst("TOAST_CAP");
const CHAT_CAP = hudConst("CHAT_CAP");

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
for (const rel of ["index.html", "src/hud.js", "src/input.js"]) {
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
await new Promise((r) => server.listen(PORT, "127.0.0.1", r));

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

await page.goto(`http://127.0.0.1:${PORT}/`, { waitUntil: "load" });

// =============================================================================
// A. the scaffold — what lets index.html leave the renderer tier
// =============================================================================
const scaffold = await page.evaluate((ids) => {
  const gl = document.getElementById("gl");
  const cs = gl ? getComputedStyle(gl) : null;
  const entry = [...document.querySelectorAll("script[type=module][src]")].map((s) =>
    s.getAttribute("src"),
  );
  return {
    missing: ids.filter((id) => !document.getElementById(id)),
    hasCanvas: !!gl && gl.tagName === "CANVAS",
    canvasDisplay: cs ? cs.display : null,
    canvasW: gl ? gl.clientWidth : 0,
    canvasH: gl ? gl.clientHeight : 0,
    entry,
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
  const ui = { hud, input, respawns: [], chats: [] };
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
      empty: r.classList.contains("empty"),
      num: r.querySelector(".vnum").textContent,
      width: r.querySelector(".vfill").style.width,
      kind: r.querySelector(".vfill").className,
    }));
  hud.setVitals(90, 100, 40, 100, 70, 100);
  const positional = read();
  hud.setVitals(0, 100, 5, 100, 5, 100);
  const zeroed = read();
  hud.setVitals(50, 100, 0, 0, 0, 0);
  const unstated = read();
  hud.setVitals(150, 100, 33, 100, -5, 100);
  const clamped = read();
  hud.setVitals(0, 0, 0, 0, 0, 0);
  const silent = getComputedStyle(box).display;
  return { positional, zeroed, unstated, clamped, silent, boxShown: box.style.display };
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
// H. no page errors anywhere in the above
// =============================================================================
check(errors.length === 0, `the page reported errors: ${errors.join(" | ")}`);

console.log(
  `  ui smoke: scaffold ${HUD_IDS.length} ids · hotbar 6 cells · composer swallow · ` +
    `chat cap ${CHAT_CAP} · toast cap ${TOAST_CAP} · vitals positional · death answered once`,
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
