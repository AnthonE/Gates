/* ── the one eval this page answers, without 'unsafe-eval' ─────────────────
   `cpal` (under `bevy_audio` → `rodio`) decides whether the browser has Web
   Audio with `js_sys::eval("typeof AudioContext !== 'undefined'")` (cpal
   0.15.3, `host/webaudio/mod.rs`). This page's CSP allows 'wasm-unsafe-eval'
   and NOT 'unsafe-eval' (scry-forge's nginx block for /games/gates/), so that
   eval throws, cpal counts the throw as "no Web Audio", and `bevy_audio` logs
   "No audio device found": a silent game with every check green. Measured
   2026-09-13 as an A/B of one build — served without the header it found its
   device, served with it it did not.

   Loosening the policy would re-enable eval for everything on the page to
   answer one question, so the page answers that question instead. The glue
   calls `eval` by its global name at call time; this returns the real answer
   to exactly cpal's string and hands every other string to the ORIGINAL eval,
   where the browser applies the CSP exactly as before — so nothing gains an
   eval it did not have, and the refusal is the browser's own.

   ⚠ It delegates rather than throwing, and that is measured, not tidy: the
   first version threw its own EvalError and broke Playwright, whose
   `evaluate` and `waitForFunction` compile their predicates through the page's
   global `eval` (and are exempt from the CSP). */
const CPAL_WEBAUDIO_PROBE = "typeof AudioContext !== 'undefined'";
const originalEval = globalThis.eval;
globalThis.eval = (src) =>
  src === CPAL_WEBAUDIO_PROBE ? typeof AudioContext !== "undefined" : originalEval(src);

/* ── the page is the game's front end ──────────────────────────────────────
   The desktop client draws its menu in Bevy (`client::render::menu`); here
   the page draws the same shell — wordmark, nav column, a control panel and a
   wide pane over footage — and Bevy starts at the loading screen, because the
   page joins before a renderer exists (`client-web/src/lib.rs`, `play`).

   It is served from elopros.com, so the account, the item store and the news
   are the platform's own and are read from the same origin: the wallet this
   browser signed in with on Elo Pros is the one it plays with here, and
   signing out on either signs out of both (`elo.signedout`, below). */
const $ = (id) => document.getElementById(id);
const q = new URLSearchParams(location.search);
const onOrigin = /(^|\.)elopros\.com$/.test(location.hostname);
/* Where Elo Pros is. On the origin every platform link and read is relative;
   anywhere else (a checkout, `python3 -m http.server`) links go to the public
   site and the reads are skipped, since the origin sends no CORS headers. */
const SITE = onOrigin ? "" : "https://elopros.com";
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const short = (a) => String(a).slice(0, 6) + "…" + String(a).slice(-4);

for (const a of document.querySelectorAll("[data-site]")) a.href = SITE + a.dataset.site;

const status = $("status");
const watchStatus = $("watch-status");
const say = (text, cls, el = status) => {
  el.textContent = text;
  el.className = "status" + (cls ? " " + cls : "");
};

/* A same-origin JSON read with a deadline. Never throws: null is "could not
   look", which every caller draws as a sentence rather than as an empty list. */
async function getJSON(path, ms = 12000) {
  if (!onOrigin) return null;
  const ac = new AbortController();
  const timer = setTimeout(() => ac.abort(), ms);
  try {
    const r = await fetch(path, { headers: { Accept: "application/json" }, signal: ac.signal });
    return r.ok ? await r.json() : null;
  } catch (err) {
    return null;
  } finally {
    clearTimeout(timer);
  }
}

/* ── coming back ───────────────────────────────────────────────────────────
   Leaving the world — the Esc menu's DISCONNECT, Escape on the loading screen,
   the shard hanging up, QUIT — hands control back here
   (`client::render::web::hand_back`) by calling `gatesLeft(status)`. The
   renderer cannot be handed a second session on the same canvas (Bevy's
   `run()` never returns on wasm and a second `App` over the first's context
   is two owners of one WebGL context), so the page notes why and RELOADS,
   and says so on the way back in. */
const LEFT = "gates.left";
/* **Ask before a tab closes on a player** (crouch v83). Crouch is Ctrl, and
   Ctrl+W — crouch and walk forward — closes the tab; no page can swallow it.
   The browser's own "leave site?" prompt is the only guard there is, armed
   while a player is in the world and dropped before the page reloads itself. */
const guardLeave = (e) => { e.preventDefault(); e.returnValue = ""; };
window.gatesLeft = (why) => {
  window.removeEventListener("beforeunload", guardLeave);
  try { sessionStorage.setItem(LEFT, String(why || "left the world")); } catch (err) { /* private mode */ }
  location.reload();
};
let cameBack = null;
try {
  cameBack = sessionStorage.getItem(LEFT);
  if (cameBack) sessionStorage.removeItem(LEFT);
} catch (err) { /* private mode */ }

/* ── the panes ─────────────────────────────────────────────────────────────
   One screen with several payloads, as the reference's is: picking a nav
   entry swaps the panel and the pane and nothing else moves. The pane is in
   the address (`#store`, `#news`) so a link can open one. */
const panes = [...document.querySelectorAll(".pane-set")];
const navs = [...document.querySelectorAll("#nav [data-pane]")];
const opened = new Set();
const onFirstShow = {
  news: fillNews,
  store: fillStore,
};
function show(name) {
  if (!panes.some((p) => p.dataset.pane === name)) name = "play";
  for (const p of panes) p.hidden = p.dataset.pane !== name;
  for (const n of navs) n.setAttribute("aria-current", String(n.dataset.pane === name));
  if (!opened.has(name)) {
    opened.add(name);
    if (onFirstShow[name]) onFirstShow[name]();
  }
  try {
    history.replaceState(null, "", name === "play"
      ? location.pathname + location.search
      : location.pathname + location.search + "#" + name);
  } catch (err) { /* a sandboxed frame */ }
}
for (const n of navs) n.addEventListener("click", () => show(n.dataset.pane));

/* ── the servers ───────────────────────────────────────────────────────────
   The rows are the title's own shard list — `elo-shardlist-v1`, the document
   the desktop menu and the elo launcher both read (`client::shardlist`) —
   served by the origin at `/api/launcher/servers/gates`. Until it answers,
   and off the origin, the page offers the address it has always dialled.
   `?server=` (a join link) puts that address first and picks it. */
const PUBLIC_ADDR = "game.elopros.com:61234";
const DEV_ADDR = "127.0.0.1:4433";
let rows = [];
let picked = null;

function baseRows() {
  const out = [];
  const linked = (q.get("server") || "").trim();
  if (linked) out.push({ id: "link", name: "From your link", addr: linked, direct: true });
  if (!onOrigin) out.push({ id: "dev", name: "Local dev server", addr: DEV_ADDR, direct: true });
  out.push({ id: "public", name: "Gates public server", addr: PUBLIC_ADDR, map: null, max: 100 });
  return out;
}

async function loadServers() {
  const doc = await getJSON("/api/launcher/servers/gates");
  const listed = doc && Array.isArray(doc.servers) ? doc.servers : null;
  if (listed && listed.length) {
    const keep = rows.filter((r) => r.direct);
    const known = new Map(rows.map((r) => [r.id, r]));
    rows = keep.concat(listed.slice(0, 64).map((s) => {
      const was = known.get(String(s.id));
      return {
        id: String(s.id || s.addr),
        name: String(s.name || s.addr),
        addr: String(s.addr),
        map: s.map ? String(s.map) : null,
        players: Number.isInteger(s.players) ? s.players : (was ? was.players : undefined),
        max: Number.isInteger(s.max_players) ? s.max_players : null,
        statusUrl: /^https:\/\/[^\s/]+\/\S*$/.test(String(s.status_url || "")) ? s.status_url : null,
      };
    }));
    /* the public fallback row is the listed one under another name */
    if (picked === "public" || !rows.some((r) => r.id === picked)) picked = rows[0].id;
  }
  renderServers();
  rows.forEach(pollStatus);
}

/* A shard answers `GET /status.json` about itself (`server::status`), and
   every reader asks it directly — nothing proxies the count. From this page
   that read needs the status host in the page's `connect-src` and an
   `Access-Control-Allow-Origin` on the status vhost; without both it fails
   here, and the count stays `?` exactly as the desktop menu draws a shard
   that did not answer. One read per load and per REFRESH, never a timer. */
async function pollStatus(row) {
  if (!row.statusUrl) return;
  try {
    const r = await fetch(row.statusUrl, { cache: "no-store" });
    if (!r.ok) return;
    const s = await r.json();
    if (Number.isInteger(s.players)) row.players = s.players;
    if (Number.isInteger(s.max_players)) row.max = s.max_players;
    row.live = Number.isInteger(s.players);
    renderServers();
  } catch (err) {
    /* not answered, or not allowed to ask: unknown, never zero */
  }
}

const population = (r) => Number.isInteger(r.players)
  ? `${r.players} / ${r.max ?? "?"}`
  : (r.max ? `? / ${r.max}` : "?");
const current = () => rows.find((r) => r.id === picked) || rows[0];

function pick(id) {
  picked = id;
  renderServers();
}

function rowEl(r, onGo) {
  const b = document.createElement("button");
  b.type = "button";
  b.className = "srv";
  b.setAttribute("role", "radio");
  b.setAttribute("aria-checked", String(r.id === picked));
  const dot = document.createElement("span");
  dot.className = "dot";
  const mid = document.createElement("span");
  const nm = document.createElement("span");
  nm.className = "nm";
  nm.textContent = r.name;
  const sub = document.createElement("span");
  sub.className = "sub";
  sub.textContent = [r.addr, r.map].filter(Boolean).join("  ·  ");
  mid.append(nm, sub);
  const pop = document.createElement("span");
  pop.className = "pop num" + (Number.isInteger(r.players) ? "" : " unknown");
  if (r.live) {
    const live = document.createElement("span");
    live.className = "live";
    pop.append(live);
  }
  pop.append(population(r));
  const label = document.createElement("small");
  label.textContent = "PLAYERS";
  pop.append(label);
  b.append(dot, mid, pop);
  b.addEventListener("click", () => pick(r.id));
  b.addEventListener("dblclick", () => { pick(r.id); onGo(); });
  return b;
}

const serverInput = $("server");
function renderServers() {
  $("servers").replaceChildren(...rows.map((r) => rowEl(r, () => play())));
  $("watch-servers").replaceChildren(...rows.map((r) => rowEl(r, () => watch())));
  const r = current();
  if (r && document.activeElement !== serverInput) serverInput.value = r.addr;
}

/* Typing an address in OPTIONS is a row of its own, picked. */
serverInput.addEventListener("change", () => {
  const addr = serverInput.value.trim();
  if (!addr) return;
  const same = rows.find((r) => r.addr === addr);
  if (same) { pick(same.id); return; }
  rows = rows.filter((r) => r.id !== "typed");
  rows.unshift({ id: "typed", name: "Typed in Options", addr, direct: true });
  pick("typed");
});
$("hash").value = q.get("hash") || "";
$("refresh").addEventListener("click", () => {
  say("refreshing the server list…");
  loadServers().then(() => say(""));
});

/* ── the wallet ────────────────────────────────────────────────────────────
   Deliberately the same calls as the platform's own `Deck.wallet`
   (`watchtower/js/deck-shim.js` in AnthonE/scry-forge): has, accounts,
   connect, sign — and the address check before signing is copied from it for
   the reason its own comment gives: somebody switches account in the
   extension, the page still holds the old address, and `personal_sign` then
   fails with a provider-internal error naming nothing anyone can act on.

   WHAT THIS DOES NOT DO, and must not: compose the message. The text comes
   out of Rust (`protocol::siwe_message`, through `Proof::message`) because
   the shard rebuilds the same bytes to verify them, and a second copy of an
   EIP-4361 message in JavaScript would drift into every login failing as
   `WrongSigner`. `sign` receives a finished string and hands it to the
   wallet, which renders it for the person about to approve it. */
const wallet = {
  has: () => !!window.ethereum,
  async accounts() {
    if (!wallet.has()) return [];
    try { return (await window.ethereum.request({ method: "eth_accounts" })) || []; }
    catch (err) { return []; }
  },
  async connect() {
    if (!wallet.has()) throw new Error("no wallet in this browser");
    const a = await window.ethereum.request({ method: "eth_requestAccounts" });
    if (!a || !a[0]) throw new Error("no account");
    return a[0];
  },
  async sign(message, address) {
    const want = String(address || "").toLowerCase();
    const have = (await wallet.accounts()).map((a) => String(a).toLowerCase());
    if (want && have.length && !have.includes(want))
      throw new Error("your wallet is on " + short(have[0]) + ", and this asks "
        + short(want) + " to sign — switch your wallet to that address and try again.");
    if (want && !have.length)
      throw new Error("your wallet is locked or not connected to this site — open it, then try again.");
    const hex = "0x" + Array.from(new TextEncoder().encode(message))
      .map((b) => b.toString(16).padStart(2, "0")).join("");
    return window.ethereum.request({ method: "personal_sign", params: [hex, address] });
  },
};
/* A wallet's own refusal, in words: 4001 is "no thanks", -32002 is a request
   already open (usually a window behind the browser). Same table as the
   platform's `Deck.purse.refusal`. */
const walletSaid = (err) => err && err.code === 4001 ? "you said no in your wallet."
  : err && err.code === -32002 ? "your wallet already has a request open — check its window, then try again."
  : (err && err.message) || String(err);

/* ── signed in on Elo Pros is signed in here ───────────────────────────────
   There is no session anywhere: identity is a connected wallet. The platform
   keeps one bit of its own — `elo.signedout`, written by its SIGN OUT —
   because most wallets cannot revoke a site's access, and a sign-out that
   `eth_accounts` quietly undid would be a button that does nothing. This
   page is on the same origin, so it reads and writes the same key: signing
   out here signs out of the store, and the other way round
   (`store.js` §signed out; `grep -rn elo.signedout` finds every end). */
const SIGNED_OUT = "elo.signedout";
const signedOut = () => { try { return localStorage.getItem(SIGNED_OUT) === "1"; } catch (err) { return false; } };
const setSignedOut = (on) => {
  try { if (on) localStorage.setItem(SIGNED_OUT, "1"); else localStorage.removeItem(SIGNED_OUT); }
  catch (err) { /* nothing depends on it being writable */ }
};

let address = null;   /* the connected wallet, or null */
let who = null;       /* { name, face, sworn } from the platform's register */
let balance = null;   /* whole ELO, as the platform prints it */

async function resume() {
  if (!wallet.has() || signedOut()) return;
  /* MetaMask answers [] for the first moments of a page and the real account
     after (`store.js` §silentAccount measured it), so a miss is asked again. */
  for (const wait of [0, 400, 1500]) {
    if (wait) await sleep(wait);
    if (address || signedOut()) return;
    const a = (await wallet.accounts())[0];
    if (a) { setAddress(a); return; }
  }
}

async function signIn() {
  try {
    const a = await wallet.connect();
    setSignedOut(false);
    setAddress(a);
    return a;
  } catch (err) {
    say(walletSaid(err), "bad");
    return null;
  }
}

async function signOut() {
  setSignedOut(true);
  try {
    Object.keys(sessionStorage).filter((k) => k.startsWith("elo.bal.")).forEach((k) => sessionStorage.removeItem(k));
  } catch (err) { /* private mode */ }
  setAddress(null);
  /* the real revocation where the wallet has one (EIP-2255); the flag above
     is what makes this work on the wallets that do not */
  try { await window.ethereum.request({ method: "wallet_revokePermissions", params: [{ eth_accounts: {} }] }); }
  catch (err) { /* unsupported or refused: the flag still stands */ }
}

function setAddress(a) {
  const next = a ? String(a) : null;
  if ((next || "").toLowerCase() === (address || "").toLowerCase()) { paintAccount(); return; }
  address = next;
  who = null;
  balance = null;
  paintAccount();
  if (address) { readWho(address); readBalance(address); }
  if (opened.has("store")) fillOwned();
}

/* The name and face the platform shows for this wallet — the same read and
   the same face order as its masthead (`store.js` acctIdentity / faceSrc):
   the name and picture set on the account page, then a seat, then a picture
   on a media host, then the derived mark. */
async function readWho(addr) {
  const r = await getJSON("/api/who?handle=" + encodeURIComponent(addr));
  const f = r && r.found;
  if (!f || addr !== address) return;
  const prof = f.profile || {};
  const own = typeof prof.picture === "string" && prof.picture.startsWith("/api/face/") ? prof.picture : "";
  const seat = f.seat && typeof f.seat.src === "string" ? f.seat.src : "";
  const pic = f.face && typeof f.face.picture === "string" ? f.face.picture : "";
  const mark = typeof f.avatar === "string" ? f.avatar : "";
  who = {
    name: prof.name ? String(prof.name) : f.agent ? String(f.agent) : null,
    sworn: !!f.vow_id,
    face: own ? own
      : seat.startsWith("/") && !seat.startsWith("//") ? "/api" + seat
      : /^https:\/\/[a-z0-9.-]+\/(media|upload)\//i.test(pic) ? pic
      : mark.startsWith("/") && !mark.startsWith("//") ? "/api" + mark
      : null,
  };
  paintAccount();
}

/* ELO, from `/api/holdings`, through the platform's own one-minute cache key
   so a visit from the store is already warm. */
async function readBalance(addr) {
  const key = "elo.bal." + addr.toLowerCase();
  try {
    const got = JSON.parse(sessionStorage.getItem(key) || "null");
    if (got && Date.now() - got.t < 60000) { balance = got.v; paintAccount(); return; }
  } catch (err) { /* not ours, or private mode */ }
  const r = await getJSON("/api/holdings/" + encodeURIComponent(addr));
  const elo = r && Array.isArray(r.coins) ? r.coins.find((c) => c.sym === "ELO") : null;
  if (!elo || elo.whole == null || addr !== address) return;
  balance = elo.whole;
  try { sessionStorage.setItem(key, JSON.stringify({ t: Date.now(), v: balance })); } catch (err) { /* fine */ }
  paintAccount();
}

function paintFace(el) {
  el.replaceChildren();
  if (!address) { el.textContent = "·"; return; }
  const name = who && who.name;
  el.textContent = name ? name.trim().charAt(0).toUpperCase() : address.slice(2, 4).toUpperCase();
  if (who && who.face) {
    const img = document.createElement("img");
    img.alt = "";
    img.src = SITE + who.face;
    img.addEventListener("error", () => img.remove());
    el.append(img);
  }
}

function paintAccount() {
  const here = !!address;
  const name = (who && who.name) || (address ? short(address) : "");
  const bal = balance != null ? `${balance} ELO` : "";
  $("acct-in").hidden = here;
  $("acct-chip").hidden = !here;
  $("who-here").hidden = !here;
  $("who-away").hidden = here;
  if (here) {
    $("acct-name").textContent = name;
    $("acct-bal").textContent = bal || short(address);
    $("acct-addr").textContent = address;
    $("who-name").textContent = name;
    $("who-addr").textContent = short(address);
    const wb = $("who-bal");
    wb.textContent = bal;
    wb.className = bal ? "elo-tag num" : "";
    paintFace($("acct-face"));
    paintFace($("who-face"));
    for (const a of document.querySelectorAll("[data-who]"))
      a.href = SITE + a.dataset.site + "?who=" + encodeURIComponent(address);
  } else {
    closeMenu();
    const line = $("who-away-line");
    const inBtn = $("who-in");
    if (wallet.has()) {
      line.textContent = "Sign in with your wallet. Your base, your name and your skins follow the wallet, on every server.";
    } else {
      line.textContent = "No wallet in this browser. Gates signs you in with one, like MetaMask or Rabby — add one, then reload.";
    }
    inBtn.hidden = !wallet.has();
    $("who-get").hidden = wallet.has();
  }
  paintGo();
}

const chip = $("acct-chip");
const menuPop = $("acct-menu");
function closeMenu() { menuPop.hidden = true; chip.setAttribute("aria-expanded", "false"); }
chip.addEventListener("click", (ev) => {
  ev.stopPropagation();
  const open = menuPop.hidden;
  menuPop.hidden = !open;
  chip.setAttribute("aria-expanded", String(open));
});
document.addEventListener("click", (ev) => { if (!menuPop.contains(ev.target)) closeMenu(); });
document.addEventListener("keydown", (ev) => { if (ev.key === "Escape") closeMenu(); });
$("acct-out").addEventListener("click", () => { closeMenu(); signOut(); });
for (const b of [$("acct-in"), $("who-in")]) {
  b.addEventListener("click", async () => {
    if (!wallet.has()) {
      show("play");
      say("No wallet in this browser — add one like MetaMask or Rabby, then reload.", "bad");
      return;
    }
    b.disabled = true;
    await signIn();
    b.disabled = false;
  });
}
/* Somebody switches account in the extension while the page is open. A
   provider event may not sign a signed-out reader back in: some wallets fire
   it on unlock and on tab focus. */
if (wallet.has() && typeof window.ethereum.on === "function") {
  window.ethereum.on("accountsChanged", (a) => setAddress(signedOut() ? null : (a && a[0]) || null));
}

/* ── the download ──────────────────────────────────────────────────────────
   The module is the game — Bevy, the transport and the client core, ~34 MB
   before gzip — and it starts downloading the moment the page opens, with
   the bar at the bottom and the PLAY button showing how far it is. PLAY
   pressed early is kept, and goes the moment the module is ready.

   `build.json` sits beside the module (`ci/build_web.sh`) and carries its
   size: with gzip on the wire, Content-Length is the compressed size and the
   bytes read here are the decompressed ones, so the header cannot be the
   denominator. No `build.json` means a bar that counts megabytes instead. */
let glue = null;     /* the wasm-bindgen module, once instantiated */
let dl = { got: 0, want: 0, done: false, failed: null };
const MB = (n) => (n / 1048576).toFixed(1);

function paintDownload() {
  const box = $("dl");
  const pct = dl.want ? Math.min(1, dl.got / dl.want) : 0;
  box.classList.toggle("done", dl.done);
  $("dl-bar").style.width = dl.done ? "100%" : (dl.want ? (pct * 100).toFixed(1) + "%" : "8%");
  $("dl-what").textContent = dl.failed ? "Download failed" : dl.done ? "Ready"
    : dl.want && dl.got >= dl.want ? "Preparing" : "Downloading";
  $("dl-n").textContent = dl.done || dl.failed ? ""
    : dl.want ? `${MB(dl.got)} / ${MB(dl.want)} MB` : `${MB(dl.got)} MB`;
  paintGo();
}

function counted(res) {
  if (!res.ok) throw new Error(`the game module answered HTTP ${res.status}`);
  if (!dl.want && !res.headers.get("content-encoding"))
    dl.want = Number(res.headers.get("content-length")) || 0;
  if (!res.body || typeof TransformStream !== "function") return res;
  let last = 0;
  const body = res.body.pipeThrough(new TransformStream({
    transform(chunk, ctl) {
      dl.got += chunk.byteLength;
      const now = performance.now();
      if (now - last > 120) { last = now; paintDownload(); }
      ctl.enqueue(chunk);
    },
  }));
  /* the bytes are already decoded, so only the type travels: it is what lets
     `instantiateStreaming` compile while the rest is still arriving */
  return new Response(body, { status: 200, headers: { "Content-Type": "application/wasm" } });
}

/* ── which module ──────────────────────────────────────────────────────────
   The game is built twice (`ci/build_web.sh`): once drawing through WebGPU,
   which is the desktop's frame — the atmosphere, ambient occlusion, bloom,
   four shadow cascades — and once through WebGL2 for every browser without
   it. Bevy picks its GPU API when it is COMPILED and has no fallback at run
   time, so the page picks, before the download: a browser that hands back a
   WebGPU adapter gets the WebGPU module — if it has the two optional
   features that frame leans on and Bevy never checks for: filterable 32-bit
   floats (the atmosphere's lookup tables) and an `rg11b10ufloat` render
   target (bloom). Without either the WebGPU frame would be lost, not
   degraded, so that browser gets WebGL2. `?gfx=webgl2` or `?gfx=webgpu`
   forces one. An older `build.json` that names no modules is the WebGL2
   module alone. */
const MODULES = {
  webgl2: { js: "./client_web.js", wasm: "client_web_bg.wasm", name: "WebGL2" },
  webgpu: { js: "./client_web_gpu.js", wasm: "client_web_gpu_bg.wasm", name: "WebGPU" },
};
async function pickModule(built) {
  const has = (k) => k === "webgl2" || !!(built && built[k]);
  const forced = q.get("gfx");
  if ((forced === "webgl2" || forced === "webgpu") && has(forced)) return forced;
  if (!has("webgpu") || !navigator.gpu) return "webgl2";
  try {
    /* A blocklisted GPU answers null; a wedged one may never answer. */
    const adapter = await Promise.race([
      navigator.gpu.requestAdapter({ powerPreference: "high-performance" }),
      new Promise((r) => setTimeout(() => r(null), 3000)),
    ]);
    const needs = ["float32-filterable", "rg11b10ufloat-renderable"];
    return adapter && needs.every((f) => adapter.features.has(f)) ? "webgpu" : "webgl2";
  } catch (err) {
    return "webgl2";
  }
}

/* The WebGPU module's error hook (`render/web.rs`, `watch_gpu`): WebGPU
   refuses a pipeline quietly and the frame is simply not drawn, so the first
   refusal puts a line over the canvas offering the WebGL2 module. The join is
   spent by then, so the way there is a reload. */
let gpuTrouble = false;
window.gatesGpuError = (msg) => {
  if (gpuTrouble || window.gatesGfx !== "webgpu") return;
  gpuTrouble = true;
  const bar = document.createElement("div");
  bar.setAttribute("role", "alert");
  bar.style.cssText = "position:fixed;left:50%;top:12px;transform:translateX(-50%);z-index:50;"
    + "max-width:min(92vw,640px);padding:10px 14px;border-radius:6px;background:rgba(24,20,16,.92);"
    + "color:#f2e9dc;font:14px/1.4 'Roboto Condensed',system-ui,sans-serif;box-shadow:0 2px 12px #0008";
  const link = document.createElement("a");
  const u = new URL(location.href);
  u.searchParams.set("gfx", "webgl2");
  link.href = u.toString();
  link.textContent = "Reload with WebGL2";
  link.style.cssText = "color:#ffcf7a;margin-left:8px";
  bar.append("The graphics card refused part of the frame under WebGPU.", link);
  bar.title = String(msg).slice(0, 400);
  document.body.append(bar);
};

async function load(gfx, sizes) {
  const m = MODULES[gfx];
  dl.got = 0;
  dl.want = sizes && Number(sizes.wasm_bytes) > 0 ? Number(sizes.wasm_bytes) : 0;
  paintDownload();
  const mod = await import(m.js);
  const res = fetch(new URL(m.wasm, import.meta.url)).then(counted);
  /* The module's exports, kept on the window for a person with the console
     open: `gatesWasm.memory.buffer.byteLength` is the wasm heap, which is
     the number to read when a tab dies with no message (an out-of-memory
     abort in wasm is a bare `RuntimeError: unreachable`). `gatesGfx` is
     which of the two it is. */
  window.gatesWasm = await mod.default({ module_or_path: res });
  window.gatesGfx = gfx;
  return mod;
}

async function boot() {
  let built = null;
  let bits = [];
  try {
    const r = await fetch("build.json", { cache: "no-cache" });
    if (r.ok) {
      const b = await r.json();
      built = b.modules || null;
      bits = [b.version && "v" + b.version, b.commit, b.proto && "wire " + b.proto].filter(Boolean);
      if (!built && Number(b.wasm_bytes) > 0) built = { webgl2: { wasm_bytes: b.wasm_bytes } };
    }
  } catch (err) { /* a dev build without it: the bar counts megabytes */ }
  let gfx = await pickModule(built);
  const footer = () => {
    $("build").textContent = "Gates " + [...bits, MODULES[gfx].name].join(" · ");
  };
  footer();
  paintDownload();
  try {
    try {
      glue = await load(gfx, built && built[gfx]);
    } catch (err) {
      /* A WebGPU module that will not compile or instantiate here still
         leaves the WebGL2 one, and a player should get that rather than an
         error. Anything after instantiation is Bevy's, and fails in the
         canvas — `?gfx=webgl2` is the way past it. */
      if (gfx !== "webgpu") throw err;
      console.warn("gates: the WebGPU module failed, loading WebGL2", err);
      gfx = "webgl2";
      footer();
      glue = await load(gfx, built && built[gfx]);
    }
    dl.done = true;
  } catch (err) {
    dl.failed = (err && err.message) || String(err);
    say(`The game did not download (${dl.failed}). Reload to try again.`, "bad");
  }
  paintDownload();
  return glue;
}

/* ── the audio thread, opened inside the gesture ────────────────────────────
   The game's sound is rendered in Rust inside an `AudioWorkletProcessor`
   (`crates/sound/src/worklet.rs`; the page-side seam is
   `client/src/render/audio_web.rs`). This builds it and leaves the result on
   `globalThis.gatesAudio`, where the renderer looks for it.

   ⚠ **`new AudioContext()` must happen inside a user gesture**, which is why
   PLAY calls this before its first `await`: a context created without one
   starts `suspended`, and a suspended context is silence with no error raised
   anywhere. Built once per page — a refused wallet prompt and a second press
   reuse the same context rather than stacking contexts a browser caps.

   A browser that refuses any of it is left with no `gatesAudio`, which
   `audio_web::open` reports once as `audio output: none` and then drops every
   command and counts it. A tab with no sound still plays. */
let audioStarted = null;
function startAudio() {
  if (!audioStarted) audioStarted = openAudio();
  return audioStarted;
}
async function openAudio() {
  const ctx = new AudioContext({ latencyHint: "interactive" });
  try {
    /* `addModule` loads the CONCATENATION of `audio-prelude.js`, the
       sound-worklet glue and `audio-processor.js` (see `ci/build_web.sh`) —
       one script, no imports, because a worklet scope has no module graph
       worth relying on. */
    await ctx.audioWorklet.addModule("./audio.js");
    /* Fetched HERE, because a worklet scope has no `fetch`, and posted down
       the port as BYTES, transferred, for the worklet to compile itself.
       ⚠ **Not as a compiled `WebAssembly.Module`**: Chromium 151 does not
       deserialize a Module in the AudioWorklet scope, so the processor got
       `messageerror`, never ran `init`, and the page was silent (measured
       2026-09-17). */
    const res = await fetch("./sound_worklet_bg.wasm");
    if (!res.ok) throw new Error(`sound_worklet_bg.wasm: ${res.status}`);
    const bytes = await res.arrayBuffer();
    const node = new AudioWorkletNode(ctx, "gates-audio", {
      numberOfInputs: 0,
      numberOfOutputs: 1,
      outputChannelCount: [2],
    });
    node.port.onmessageerror = () => console.warn("gates audio: a message did not decode");
    node.connect(ctx.destination);
    node.port.postMessage({ kind: "init", bytes, rate: ctx.sampleRate }, [bytes]);
    /* The renderer reads `rate` to build every playback rate against
       (`engine::rate` is then the only resample in the chain) and `port` to
       post to. Nothing else here is the game's business. */
    globalThis.gatesAudio = { context: ctx, node, port: node.port, rate: ctx.sampleRate };
    /* Some browsers suspend a context when the tab is hidden and do not
       resume it on return; a click inside the canvas is a gesture we already
       get for pointer lock. */
    const wake = () => { if (ctx.state === "suspended") ctx.resume().catch(() => {}); };
    addEventListener("pointerdown", wake, { passive: true });
    addEventListener("keydown", wake, { passive: true });
    console.info(`gates audio: AudioWorklet at ${ctx.sampleRate} Hz`);
  } catch (err) {
    /* Never fatal: the game is playable silent. */
    console.warn("gates audio: none -", (err && err.message) || err);
    try { await ctx.close(); } catch (closeErr) { /* already closed */ }
    delete globalThis.gatesAudio;
  }
}

/* ── PLAY and WATCH ────────────────────────────────────────────────────────
   One button each, and each draws its own state: how far the download is,
   who it will sign in as, and what it is waiting on once pressed. */
let busy = null;        /* null, or { label, sub } while a join is in flight */
const canTransport = "WebTransport" in window;
const touchOnly = matchMedia("(pointer: coarse)").matches && !matchMedia("(any-pointer: fine)").matches;

function paintButton(btn, fill, sub, idleSub) {
  const loading = !dl.done && !dl.failed;
  btn.classList.toggle("busy", !!busy);
  btn.classList.toggle("wait", loading && !busy);
  btn.disabled = !!busy || !canTransport;
  fill.style.width = loading && dl.want ? Math.min(100, (dl.got / dl.want) * 100).toFixed(1) + "%" : "0";
  sub.textContent = busy ? busy.sub
    : !canTransport ? "this browser cannot reach the server"
    : dl.failed ? "the download failed — press to reload"
    : loading && dl.want && dl.got >= dl.want ? "preparing the game"
    : loading ? (dl.want ? `downloading ${Math.floor((dl.got / dl.want) * 100)}%` : `downloading ${MB(dl.got)} MB`)
    : idleSub;
}

function paintGo() {
  const idle = address ? `as ${(who && who.name) || short(address)}`
    : wallet.has() ? "signs in with your wallet" : "as a guest";
  paintButton($("go"), $("go-fill"), $("go-sub"), idle);
  $("go-label").textContent = busy ? busy.label : dl.failed ? "Reload" : "Play";
  paintButton($("watch-go"), $("watch-fill"), $("watch-sub"), "no wallet needed");
  /* A guest join is offered where a guest can get in: off the origin (a dev
     shard) or on a server a link named. The public shard runs `require_auth`
     (`shard-public.toml`), so on the origin the wallet is the way in. */
  $("guest").hidden = !!address || !wallet.has() || !!busy || (onOrigin && !q.get("server"));
}

/* Join, and hand the world to Bevy. Both buttons come through here, so the
   page reads a refusal in exactly one place — and reads its CODE, never its
   text: `Gates.join` throws an Error carrying the shard's own `REFUSE_*`
   number as `e.code`, and its message is already right for a browser
   (`client::refusal_sentence`). `crates/client/tests/refusals.rs` scans the
   catch below and fails if it ever matches on words. */
async function join(kind, audio, identity) {
  const line = kind === "watch" ? watchStatus : status;
  const row = current();
  const server = (row ? row.addr : serverInput.value).trim();
  const hash = $("hash").value.trim() || undefined;
  const { Gates } = glue;
  let g;
  try {
    if (kind === "watch") {
      const target = $("watch-who").value.trim();
      busy = { label: "Connecting", sub: server };
      paintGo();
      say(`connecting to ${server} to watch…`, "", line);
      g = await Gates.watch(server, hash, target && target !== "any" ? target : undefined);
    } else {
      busy = { label: "Connecting", sub: server };
      paintGo();
      say(identity ? `connecting to ${server} as ${short(identity)}…` : `connecting to ${server} as a guest…`, "", line);
      /* Address and signer together or neither — `Gates.join` refuses the odd
         pair rather than dropping half of it. */
      g = await Gates.join(server, hash, identity ?? undefined, identity
        ? (text) => {
          busy = { label: "Signing in", sub: "approve the sign-in in your wallet" };
          paintGo();
          return wallet.sign(text, identity);
        }
        : undefined);
    }
  } catch (e) {
    say(e.message ?? String(e), "bad", line);
    busy = null;
    paintGo();
    return;
  }
  say(g.watching ? `${g.watching} — starting the renderer…`
    : `in the world — player ${g.player_id}, seed ${g.seed} — starting the renderer…`, "good", line);
  enter(g, audio, row);
}

/* ── hand over to Bevy ──────────────────────────────────────────────────────
   The page's job ends here. It must not pump the session: `pump` drains the
   client core's own-fact rings destructively and `render/input.rs` calls it
   from a Bevy system every frame — two drivers would each see half the hits,
   toasts and refusals. Input goes with it: winit reads the canvas's own
   keyboard and pointer events, and the pointer is captured on the first click
   inside the world, by the same rule the desktop has (`ui::pointer`). */
async function enter(g, audio, row) {
  $("handoff-line").textContent = g.watching ? g.watching : `entering ${row ? row.name : "the island"}…`;
  $("handoff").hidden = false;
  document.body.classList.add("playing");
  if (!g.watching) window.addEventListener("beforeunload", guardLeave);
  $("gates").hidden = false;
  $("menu").hidden = true;
  $("scene").hidden = true;
  /* The worklet has to exist before `render::audio_web::open` looks for it,
     which runs inside the plugin build. Never throws. */
  await audio;
  /* `play` CONSUMES the session and never returns — Bevy's wasm arm hands the
     loop to requestAnimationFrame and `run()` does not come back. */
  g.play("#gates");
}

async function play({ guest = false } = {}) {
  if (busy || !canTransport) return;
  if (dl.failed) { location.reload(); return; }
  const audio = startAudio();   /* synchronously inside the press */
  let identity = guest ? null : address;
  if (!identity && !guest && wallet.has()) {
    busy = { label: "Signing in", sub: "check your wallet" };
    paintGo();
    identity = await signIn();
    if (!identity) { busy = null; paintGo(); return; }
  }
  if (!glue) {
    busy = { label: "Play", sub: "starting when the download finishes" };
    paintGo();
    say("Got it — the game starts as soon as it has downloaded.");
    if (!(await ready)) { busy = null; paintGo(); return; }
  }
  await join("play", audio, identity);
}

async function watch() {
  if (busy || !canTransport) return;
  if (dl.failed) { location.reload(); return; }
  const audio = startAudio();
  if (!glue) {
    busy = { label: "Watch", sub: "starting when the download finishes" };
    paintGo();
    if (!(await ready)) { busy = null; paintGo(); return; }
  }
  await join("watch", audio, null);
}

$("go").addEventListener("click", () => play());
$("guest").addEventListener("click", () => play({ guest: true }));
$("watch-go").addEventListener("click", () => watch());
$("watch-who").addEventListener("keydown", (ev) => { if (ev.key === "Enter") watch(); });
document.addEventListener("keydown", (ev) => {
  if (ev.key !== "Enter" || ev.repeat || busy) return;
  const t = ev.target;
  if (t && (t.tagName === "INPUT" || t.tagName === "BUTTON" || t.tagName === "A")) return;
  const open = panes.find((p) => !p.hidden);
  if (open && open.dataset.pane === "play") play();
  if (open && open.dataset.pane === "watch") watch();
});

/* ── the latest update, the news, the store ────────────────────────────────
   All three are the platform's own records, read the way its pages read
   them: the listing's `record[]` (`/api/store/catalog`, the committed
   catalog with the desk's edits over it, falling back to the static file),
   the house posts (`/posts/posts.json`), and the item store's card
   (`/api/items/store/gates`). Off the origin each pane says where it lives. */
let catalogRead = null;
function gatesListing() {
  if (!catalogRead) {
    catalogRead = (async () => {
      const d = (await getJSON("/api/store/catalog")) || (await getJSON("/listings/listings.json"));
      const titles = d && Array.isArray(d.titles) ? d.titles : [];
      return titles.find((t) => t && t.slug === "gates") || null;
    })();
  }
  return catalogRead;
}
const siteHref = (h) => !h ? SITE + "/news.html"
  : /^https?:\/\//i.test(h) ? h
  : SITE + "/" + String(h).replace(/^\/+/, "");
const newest = (t) => (Array.isArray(t && t.record) ? t.record : [])
  .filter((r) => r && r.date && r.title)
  .sort((a, b) => String(b.date).localeCompare(String(a.date)));
/* "0.8.0 — the distance stops fizzing" draws as a version tile and a line;
   anything else keeps its whole title and the tile shows the date. */
function splitTitle(r) {
  const parts = String(r.title).split(" — ");
  const ver = parts.length > 1 && /^\d+\.\d+(\.\d+)?$/.test(parts[0]) ? parts[0] : null;
  return ver ? { ver, title: parts.slice(1).join(" — ") } : { ver: null, title: String(r.title) };
}

async function fillFeature() {
  const r = newest(await gatesListing())[0];
  if (!r) return;
  const { ver, title } = splitTitle(r);
  $("feature-kicker").textContent = ["Latest update", ver, String(r.date)].filter(Boolean).join("  ·  ");
  $("feature-title").textContent = title.charAt(0).toUpperCase() + title.slice(1);
  $("feature-line").textContent = String(r.line || "");
  $("feature").href = siteHref(r.href);
  $("feature").hidden = false;
}

function card(href, src, date, title, line) {
  const a = document.createElement("a");
  a.className = "card";
  a.href = href;
  a.target = "_blank";
  a.rel = "noopener";
  const meta = document.createElement("div");
  meta.className = "meta";
  const s = document.createElement("span");
  s.className = "src";
  s.textContent = src;
  const d = document.createElement("span");
  d.className = "num";
  d.textContent = date;
  meta.append(s, d);
  const h = document.createElement("h3");
  h.textContent = title;
  const p = document.createElement("p");
  p.textContent = line || "";
  a.append(meta, h, p);
  return a;
}

async function fillNews() {
  const box = $("news");
  const none = $("news-empty");
  const [t, posts] = await Promise.all([gatesListing(), getJSON("/posts/posts.json")]);
  const items = newest(t).map((r) => {
    const { ver, title } = splitTitle(r);
    return { date: String(r.date), el: card(siteHref(r.href), ver ? "Gates " + ver : "Gates", String(r.date),
      title.charAt(0).toUpperCase() + title.slice(1), r.line) };
  });
  for (const p of (posts && Array.isArray(posts.posts) ? posts.posts : [])) {
    if (!p || !p.id || !p.title) continue;
    items.push({ date: String(p.date || ""), el: card(SITE + "/news.html?post=" + encodeURIComponent(p.id),
      "Elo Pros", String(p.date || ""), String(p.title), p.line) });
  }
  items.sort((a, b) => b.date.localeCompare(a.date));
  box.replaceChildren(...items.slice(0, 18).map((i) => i.el));
  none.hidden = items.length > 0;
  none.textContent = onOrigin ? "Couldn’t load the news. Try again in a moment." : "The news lives on Elo Pros — open All news.";
}

let storeCard = null;
async function fillStore() {
  const box = $("skins");
  const none = $("skins-empty");
  storeCard = await getJSON("/api/items/store/gates");
  const skins = storeCard && Array.isArray(storeCard.skins) ? storeCard.skins : [];
  box.replaceChildren(...skins.map(skinCard));
  none.hidden = skins.length > 0;
  none.textContent = onOrigin ? "Couldn’t load the item store. Try again in a moment." : "The item store lives on Elo Pros — open it from the panel.";
  fillOwned();
}

function skinCard(s) {
  const el = document.createElement("div");
  el.className = "card skin";
  el.dataset.catalog = String(s.catalog_id);
  const tint = /^#[0-9a-f]{6}$/i.test(String(s.tint)) ? s.tint : "#888888";
  /* The item's own icon under the skin's tint, multiplied — which is what the
     tint IS in the game (`content/skins.toml`: an sRGB multiply over the
     item's colours). The icon file is the item's name, snake-cased. */
  const icon = "assets/icons/" + String(s.covers || "").toLowerCase().replace(/[^a-z0-9]+/g, "_").replace(/^_|_$/g, "") + ".png";
  const art = document.createElement("div");
  art.className = "art";
  art.style.setProperty("--tint", tint);
  const img = document.createElement("img");
  img.alt = "";
  img.src = icon;
  const layer = document.createElement("span");
  layer.className = "tint";
  layer.style.setProperty("--icon", `url("${icon}")`);
  img.addEventListener("error", () => { img.remove(); layer.remove(); });
  const sw = document.createElement("span");
  sw.className = "sw";
  art.append(img, layer, sw);
  const txt = document.createElement("div");
  txt.className = "txt";
  const h = document.createElement("h3");
  h.textContent = String(s.name || "Skin");
  const p = document.createElement("p");
  p.textContent = s.covers ? `for the ${s.covers}` : "";
  const price = document.createElement("div");
  price.className = "price" + (s.on_sale ? "" : " soon");
  price.textContent = s.on_sale && s.price_text ? String(s.price_text) : "Coming soon";
  txt.append(h, p, price);
  el.append(art, txt);
  return el;
}

async function fillOwned() {
  const line = $("store-owned");
  for (const b of document.querySelectorAll(".skin .owned")) b.remove();
  if (!address) { line.textContent = "Sign in to see which skins you own."; return; }
  if (!onOrigin) { line.textContent = "Open the item store to see yours."; return; }
  line.textContent = "Reading your wallet…";
  const addr = address;
  const r = await getJSON("/api/items/of/" + encodeURIComponent(addr) + "?limit=200", 20000);
  if (addr !== address) return;
  if (!r || r.reachable === false) { line.textContent = "Couldn’t read your wallet just now."; return; }
  const mine = new Set((Array.isArray(r.items) ? r.items : []).map((i) => String(i && i.catalog_id)));
  let n = 0;
  for (const el of document.querySelectorAll(".skin")) {
    if (!mine.has(el.dataset.catalog)) continue;
    n += 1;
    const tag = document.createElement("span");
    tag.className = "owned";
    tag.textContent = "OWNED";
    el.querySelector(".art").append(tag);
  }
  line.textContent = n ? `You own ${n} skin${n === 1 ? "" : "s"} for Gates.` : "You don’t own any Gates skins yet.";
}

/* ── start ─────────────────────────────────────────────────────────────── */
rows = baseRows();
picked = rows[0].id;
renderServers();
paintAccount();
// WebTransport is the only path a page has to a QUIC shard, and it is not
// everywhere yet. Say so before the button is pressed rather than after.
if (!canTransport) say("This browser has no WebTransport, which Gates needs to reach its servers. Chrome, Edge and Firefox have it.", "bad");
else if (cameBack) say(cameBack + " — press Play to go back in.");
else if (touchOnly) say("Gates plays with a keyboard and mouse.");
/* A dev server is self-signed, and a page cannot skip certificate checks, so
   its hash has to be pasted — the old page opened those fields off the
   origin for exactly this. */
else if (!onOrigin && !q.get("hash")) say("Local dev server? Paste the certificate hash it printed into Options.");

/* ── watching instead of playing ───────────────────────────────────────────
   `?spectate` (any agent the shard lets you watch) or `?spectate=0x…` (that
   wallet's agent) opens SPECTATE with the target filled in: a READ-ONLY seat
   (wire v73; NETCODE.md §2.3), anonymous, signing nothing — the shard, not
   this page, decides who may be watched. */
if (q.has("spectate")) {
  const t = q.get("spectate");
  $("watch-who").value = t && t !== "any" ? t : "";
  show("watch");
} else {
  show((location.hash || "").slice(1) || "play");
}
const ready = boot();
loadServers();
fillFeature();
resume();

/* ── the main thread's stalls, counted ─────────────────────────────────────
   `findings/browser-audio-20260913.md` §4: the audio hiccup is either a long
   task on the one thread the tab has (cpal's output timer fires late and the
   buffer is scheduled in the past) or the JS garbage that timer makes. The
   renderer's own `web::heap_report` prints `dt` every two seconds; this
   prints what happens BETWEEN those prints — every task over 50 ms, as a
   count and the longest — with the JS heap beside it where Chrome exposes it.
   Read the two lines together next to the hiccup. */
if (typeof PerformanceObserver !== "undefined") {
  let longTasks = 0, longestMs = 0;
  try {
    new PerformanceObserver((list) => {
      for (const t of list.getEntries()) { longTasks += 1; longestMs = Math.max(longestMs, t.duration); }
    }).observe({ type: "longtask", buffered: true });
    setInterval(() => {
      const playing = document.body.classList.contains("playing");
      if (!playing) { longTasks = 0; longestMs = 0; return; }
      const heap = performance.memory ? ` · js heap ${(performance.memory.usedJSHeapSize / 1e6).toFixed(0)} MB` : "";
      console.info(`page: long tasks ${longTasks} (longest ${longestMs.toFixed(0)} ms)${heap}`);
      longTasks = 0; longestMs = 0;
    }, 5000);
  } catch (err) { /* no longtask entry type here */ }
}
