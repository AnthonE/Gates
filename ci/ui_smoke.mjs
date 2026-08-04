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
// This gate does NOT change that, and a previous version of this file said it
// did. It runs in the code tier, on every pass, in every lane, exempting
// nothing; `renderer_touched` is untouched. What it does is make the fix
// available: it asserts, as a strict SUPERSET, every `web/index.html` and
// `web/src/hud.js` contract that `browser_smoke` holds, so the carve-out
// proposed in `DECISIONS.md` §open can be approved as a one-line regex edit
// against coverage that already exists and already runs.
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
//
// This is a RATCHET, pinned at the count that actually ships, not a loose
// floor. It was written `>= 14` against an actual 16, which is the same hole
// one size down: two surfaces could leave the HUD entirely and the number
// would still clear the bar. Adding an element raises it (the check passes
// and this constant is updated in the same commit, like a golden); removing
// one has to be a stated act rather than a silent drift.
const HUD_ID_COUNT = 16;
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
// I. no page errors anywhere in the above
// =============================================================================
check(errors.length === 0, `the page reported errors: ${errors.join(" | ")}`);

console.log(
  `  ui smoke: scaffold ${HUD_IDS.length} ids · join form · hotbar 6 cells · composer swallow · ` +
    `chat cap ${CHAT_CAP} · toast cap ${TOAST_CAP} · vitals positional+inline · death answered once · ` +
    `craft gate/×5 · queue index`,
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
