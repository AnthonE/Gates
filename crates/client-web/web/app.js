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

/* ── inside a post ─────────────────────────────────────────────────────────
   X's player card loads this page in a frame inside the post, at
   `?embed=1` (the `twitter:player` meta in index.html). Framed or asked for,
   the page draws the embed: one PLAY that joins the public server as a guest
   unless a wallet is already signed in, the wallet as the second choice, and
   the full page one tab away. A link out of the frame opens a tab rather than
   navigating the post's frame. */
const framed = (() => { try { return window.top !== window; } catch (err) { return true; } })();
const EMBED = framed || q.has("embed");
if (EMBED) {
  document.body.classList.add("embed");
  for (const a of document.querySelectorAll("a:not([target])")) {
    a.target = "_blank";
    a.rel = "noopener";
  }
}
/* Tell the game it is inside a post (`client::render::web::in_post`): X
   sandboxes the frame without `allow-pointer-lock`, so the world is looked
   around by dragging, and the game opens in the post rather than fullscreen.
   Only X may frame this page (the origin's frame-ancestors), so framed is
   that case. */
if (framed) window.gatesInPost = true;
/* ...and say so before PLAY, since dragging to look is not what a player
   expects. On the start screen rather than over the world: a line drawn over
   the game in its first seconds was measured not always painting. */
if (framed) $("drag-line").hidden = false;

for (const a of document.querySelectorAll("[data-site]")) a.href = SITE + a.dataset.site;

const status = $("status");
const watchStatus = $("watch-status");
const say = (text, cls, el = status) => {
  el.textContent = text;
  el.className = "status" + (cls ? " " + cls : "");
  el.removeAttribute("title");
};
/* Where a link to the game points, whichever host this copy is served from. */
const GAME_URL = "https://elopros.com/games/gates/play/";
/* `protocol::REFUSE_VERSION` and `REFUSE_AUTH`: the refusal codes the page
   itself reads. */
const REFUSE_VERSION = 0;
const REFUSE_AUTH = 2;

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
    const known = new Map(rows.map((r) => [r.id, r]));
    const fresh = listed.slice(0, 64).map((s) => {
      const was = known.get(String(s.id));
      return {
        id: String(s.id || s.addr),
        name: String(s.name || s.addr),
        addr: String(s.addr),
        map: s.map ? String(s.map) : null,
        players: Number.isInteger(s.players) ? s.players : (was ? was.players : undefined),
        agents: was ? was.agents : undefined,
        max: Number.isInteger(s.max_players) ? s.max_players : null,
        proto: was ? was.proto : null,
        nextWipe: was ? was.nextWipe : 0,
        live: was ? was.live : false,
        statusUrl: /^https:\/\/[^\s/]+\/\S*$/.test(String(s.status_url || "")) ? s.status_url : null,
      };
    });
    /* A link or a typed address that IS a listed server becomes that row, so
       an invite link shows the count, the wipe and the version like the list. */
    const listedAt = new Map(fresh.map((r) => [r.addr, r.id]));
    const chosen = rows.find((r) => r.id === picked);
    rows = rows.filter((r) => r.direct && !listedAt.has(r.addr)).concat(fresh);
    if (chosen && chosen.direct && listedAt.has(chosen.addr)) picked = listedAt.get(chosen.addr);
    /* the public fallback row is the listed one under another name */
    else if (picked === "public" || !rows.some((r) => r.id === picked)) picked = rows[0].id;
  }
  renderServers();
  rows.forEach((r) => pollStatus(r));
}

/* A shard answers `GET /status.json` about itself (`server::status`), and
   every reader asks it directly — nothing proxies the count. From this page
   that read needs the status host in the page's `connect-src` and an
   `Access-Control-Allow-Origin` on the status vhost; without both it fails
   here, and the count stays `?` exactly as the desktop menu draws a shard
   that did not answer. One read per load and per REFRESH, and once more
   after a join that never reached the shard (`join`, below) — never a timer.
   True when the shard answered. */
async function pollStatus(row, ms = 8000) {
  if (!row || !row.statusUrl) return false;
  const ac = new AbortController();
  const timer = setTimeout(() => ac.abort(), ms);
  try {
    const r = await fetch(row.statusUrl, { cache: "no-store", signal: ac.signal });
    if (!r.ok) return false;
    const s = await r.json();
    if (!Number.isInteger(s.players)) return false;
    row.players = s.players;
    /* `agents` is a share of `players`, never a second population */
    row.agents = Number.isInteger(s.agents) ? Math.min(s.agents, s.players) : 0;
    if (Number.isInteger(s.max_players)) row.max = s.max_players;
    row.proto = Number.isInteger(s.proto) ? s.proto : null;
    row.nextWipe = Number.isInteger(s.next_wipe) ? s.next_wipe : 0;
    row.live = true;
    renderServers();
    return true;
  } catch (err) {
    /* not answered, or not allowed to ask: unknown, never zero */
    return false;
  } finally {
    clearTimeout(timer);
  }
}

/* What a row says about who is on it. People and agents are counted apart
   (`agents` is how many of `players` declared themselves one), and a count
   is never drawn as a bare 0: an empty shard that answered is "up now". */
const plural = (n, one) => `${n} ${one}${n === 1 ? "" : "s"}`;
function population(r, kind) {
  if (!Number.isInteger(r.players)) return "? players";
  const agents = Number.isInteger(r.agents) && r.agents > 0 ? r.agents : 0;
  const people = Math.max(0, r.players - agents);
  const full = !!r.max && r.players >= r.max;
  const bits = [];
  if (full) bits.push("full");
  if (people > 0) bits.push(plural(people, "player"));
  else if (!full && r.live) bits.push(kind === "watch" ? "up now" : "up now · be first in");
  else if (!full) bits.push("be first in");
  if (agents > 0) bits.push(plural(agents, "agent"));
  return bits.join(" · ");
}
/* "wipes in 3d": `next_wipe` is unix seconds, 0 for none (`server::wipe`).
   Floored, the desktop list's rule too (`ui::servers::Listing::wipe_in`):
   "3d" is at least three whole days. */
function wipeIn(r) {
  if (!(r.nextWipe > 0)) return null;
  const left = Math.floor(r.nextWipe - Date.now() / 1000);
  return left <= 0 ? "wipe due"
    : left >= 86400 ? `wipes in ${Math.floor(left / 86400)}d`
    : left >= 3600 ? `wipes in ${Math.floor(left / 3600)}h` : "wipes within the hour";
}

/* **The shard's wire against the page's.** A shard on another `PROTO_VER`
   refuses this page at the handshake (`REFUSE_VERSION`), so the row says so
   before PLAY is pressed. A newer shard means a newer page is out (or about
   to be): reload. An older one means the shard is the one behind. */
let pageProto = null;   /* `build.json`'s `proto`, once read */
const skewOf = (r) => !r || !Number.isInteger(r.proto) || !Number.isInteger(pageProto) || r.proto === pageProto
  ? null : r.proto > pageProto ? "newer" : "older";
/* The reload asks for the page under the shard's wire number, so a cache
   between here and the origin cannot hand back the page it already had. A
   page that arrives under that number and is STILL older is a shard that
   moved before the browser build was published: that is said, not looped. */
const reloadedFor = (r) => q.get("fresh") === String(r.proto);
const staleFor = (r) => skewOf(r) === "newer" && !reloadedFor(r);
function freshReload(r) {
  const u = new URL(location.href);
  u.searchParams.set("fresh", String(r.proto));
  location.replace(u.toString());
}

/* Ours: the host is under elopros.com. Keyed on the address, never on
   anything a list says about a row, so a community shard cannot wear it
   (`client::ui::servers::is_official_addr` is the same rule). */
function official(r) {
  const at = String(r.addr || "").lastIndexOf(":");
  if (at < 1 || r.addr.startsWith("[")) return false;
  const host = r.addr.slice(0, at).toLowerCase().replace(/\.+$/, "");
  return host === "elopros.com" || (host.endsWith(".elopros.com") && host.length > ".elopros.com".length);
}

const current = () => rows.find((r) => r.id === picked) || rows[0];

function pick(id) {
  picked = id;
  renderServers();
}

function rowEl(r, onGo, kind) {
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
  if (official(r)) {
    const o = document.createElement("span");
    o.className = "official";
    o.textContent = "Official";
    nm.append(o);
  }
  const sub = document.createElement("span");
  sub.className = "sub";
  sub.textContent = [r.addr, r.map].filter(Boolean).join("  ·  ");
  const pop = document.createElement("span");
  pop.className = "pop num" + (Number.isInteger(r.players) ? "" : " unknown");
  if (r.live) {
    const live = document.createElement("span");
    live.className = "live";
    pop.append(live);
  }
  pop.append([population(r, kind), wipeIn(r)].filter(Boolean).join(" · "));
  mid.append(nm, sub, pop);
  const skew = skewOf(r);
  if (skew) {
    const s = document.createElement("span");
    const act = staleFor(r);
    s.className = "skew" + (act ? " act" : "");
    s.textContent = skew === "older" ? "this server is updating"
      : act ? "Gates just updated: reload" : "Gates just updated: the browser version follows soon";
    if (act) s.addEventListener("click", (ev) => { ev.stopPropagation(); freshReload(r); });
    mid.append(s);
  }
  b.append(dot, mid);
  b.addEventListener("click", () => pick(r.id));
  b.addEventListener("dblclick", () => { pick(r.id); onGo(); });
  return b;
}

const serverInput = $("server");
function renderServers() {
  $("servers").replaceChildren(...rows.map((r) => rowEl(r, () => goPlay(), "play")));
  $("watch-servers").replaceChildren(...rows.map((r) => rowEl(r, () => watch(), "watch")));
  const r = current();
  if (r && document.activeElement !== serverInput) serverInput.value = r.addr;
  /* The embed has no server list, so it says how busy the island is instead:
     the people the shard reported, and only while somebody is on. */
  const n = r && Number.isInteger(r.players)
    ? r.players - (Number.isInteger(r.agents) ? r.agents : 0) : 0;
  $("pop-line").hidden = !EMBED || n < 1;
  $("pop-line").textContent = n === 1 ? "1 player on the island now" : `${n} players on the island now`;
  paintGo();
}

/* A link that opens this page with a server picked (`?server=`, read by
   `baseRows`), copied; or, where the clipboard is refused, said. */
async function copyLink(url, done) {
  try {
    await navigator.clipboard.writeText(url);
    say(done, "good");
  } catch (err) {
    say("Copy this link: " + url);
  }
}
$("invite").addEventListener("click", () => {
  const r = current();
  if (r) copyLink(GAME_URL + "?server=" + encodeURIComponent(r.addr), "Invite link copied. It opens Gates on this server.");
});

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
   wallet, which renders it for the person about to approve it.

   **Every wallet in the browser, not only `window.ethereum`.** With two
   extensions installed only one wins that slot (or a proxy that routes to a
   "default" wallet), and some wallets leave it alone while another holds it —
   so a page that reads only the slot signs in with a wallet the player did not
   pick, or says "no wallet" with one installed. EIP-6963 has each wallet
   announce itself: one wallet is used as is, several are a picker, and the
   pick is remembered by the wallet's `rdns` (`elo.wallet`) until SIGN OUT.
   `window.ethereum` is the fallback for a wallet that announces nothing. A
   wallet that injects after this script announces late, and the panel
   repaints when it does (`walletAppeared`).

   Two more rows, both the platform's (`js/wallets.js` in AnthonE/scry-forge
   draws the same picker): a **phone wallet** (WalletConnect), once the
   platform's `js/wallet-config.js` carries a project ID, through the bundle
   it names; and **Solana-only wallets** (Wallet Standard), greyed, saying why
   — sign-in is an Ethereum signature, and Phantom, MetaMask and Backpack hold
   an Ethereum account beside the Solana one. */
const WALLET_PICK = "elo.wallet";
const PHONE = "walletconnect";   /* the remembered pick for a phone wallet */
let pageReady = false;  /* set at start: an announcement before it only queues */
const announced = [];
window.addEventListener("eip6963:announceProvider", (e) => {
  const d = e.detail;
  if (!d || !d.provider || typeof d.provider.request !== "function") return;
  if (announced.some((a) => a.provider === d.provider || (a.info.uuid && d.info && a.info.uuid === d.info.uuid))) return;
  announced.push({ info: d.info || {}, provider: d.provider });
  if (pageReady) walletAppeared();
});
window.addEventListener("ethereum#initialized", () => { if (pageReady) walletAppeared(); });

/* Wallet Standard: only the name, icon and chains are read. */
const solana = [];
const standard = Object.freeze({
  register(...ws) {
    for (const w of ws) {
      const chains = w && Array.isArray(w.chains) ? w.chains : [];
      const name = String((w && w.name) || "").trim();
      if (!name || !chains.some((c) => String(c).startsWith("solana:")) || solana.some((x) => x.name === name)) continue;
      solana.push({ name, icon: String(w.icon || "") });
      if (pageReady) walletAppeared();
    }
    return () => {};
  },
});
window.addEventListener("wallet-standard:register-wallet", (e) => {
  try { if (typeof e.detail === "function") e.detail(standard); } catch (err) { /* a wallet's own bug */ }
});

/* The phone wallet's config and bundle, from the platform. */
let walletCfg = null;
function loadWalletConfig() {
  return new Promise((done) => {
    const s = document.createElement("script");
    /* /js/ sits behind a four-hour edge cache; ten minutes is soon enough */
    s.src = SITE + "/js/wallet-config.js?v=" + Math.floor(Date.now() / 600000);
    s.addEventListener("load", () => { walletCfg = window.ELO_WALLET_CONFIG || null; done(); });
    s.addEventListener("error", () => done());
    document.head.append(s);
  });
}
const phoneOn = () => !!walletCfg
  && /^[0-9a-f]{32}$/i.test(String(walletCfg.walletconnectProjectId || ""))
  && /^\/js\/vendor\/[\w.-]+\.js$/.test(String(walletCfg.walletconnectScript || ""));
let phoneKit = null;
function phone() {
  if (!phoneKit) {
    phoneKit = new Promise((ok, no) => {
      if (window.EloWalletConnect) { ok(window.EloWalletConnect); return; }
      const s = document.createElement("script");
      s.src = SITE + walletCfg.walletconnectScript;
      s.addEventListener("load", () => (window.EloWalletConnect
        ? ok(window.EloWalletConnect) : no(new Error("the phone-wallet module did not load"))));
      s.addEventListener("error", () => no(new Error("the phone-wallet module did not load")));
      document.head.append(s);
    }).catch((err) => { phoneKit = null; throw err; });
  }
  return phoneKit;
}
const phoneOpts = () => ({ projectId: walletCfg.walletconnectProjectId, url: location.origin });
const PHONE_ICON = "data:image/svg+xml," + encodeURIComponent(
  '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#edebe6" '
  + 'stroke-width="1.6"><rect x="6.5" y="2.5" width="11" height="19" rx="2.5"/><path d="M10.5 18.5h3"/></svg>');

const remembered = () => { try { return localStorage.getItem(WALLET_PICK) || ""; } catch (err) { return ""; } };
const remember = (rdns) => {
  try { if (rdns) localStorage.setItem(WALLET_PICK, rdns); else localStorage.removeItem(WALLET_PICK); }
  catch (err) { /* private mode: the picker asks again next visit */ }
};

/* A wallet in `window.ethereum` that announces nothing, named by its flags.
   Rabby and others also set isMetaMask, so MetaMask is asked last. */
function legacyName(p) {
  return p.isRabby ? "Rabby Wallet" : p.isPhantom ? "Phantom" : p.isCoinbaseWallet ? "Coinbase Wallet"
    : p.isBraveWallet ? "Brave Wallet" : p.isTrust || p.isTrustWallet ? "Trust Wallet"
    : p.isOkxWallet || p.isOKExWallet ? "OKX Wallet" : p.isMetaMask ? "MetaMask" : "Browser wallet";
}
/* "Trust" and "Trust Wallet" are one wallet in two registries. */
const sameWallet = (n) => String(n).toLowerCase().replace(/wallet/g, "").replace(/[^a-z0-9]/g, "");

/* Every row to offer: { name, icon, rdns, provider, phone?, solanaOnly? }. */
function wallets() {
  const out = announced.length
    ? announced.map((a) => ({
      name: String(a.info.name || "Wallet"),
      icon: String(a.info.icon || ""),
      rdns: String(a.info.rdns || ""),
      provider: a.provider,
    }))
    : window.ethereum ? [{ name: legacyName(window.ethereum), icon: "", rdns: "", provider: window.ethereum }] : [];
  const held = new Set(out.map((w) => sameWallet(w.name)));
  for (const s of solana) {
    if (!held.has(sameWallet(s.name))) out.push({ name: s.name, icon: s.icon, rdns: "", provider: null, solanaOnly: true });
  }
  if (phoneOn()) out.push({ name: "Phone wallet", icon: PHONE_ICON, rdns: PHONE, provider: null, phone: true });
  return out;
}
const usable = () => wallets().filter((w) => !w.solanaOnly);
const solanaOnly = () => wallets().filter((w) => w.solanaOnly);

let provider = null;    /* the EIP-1193 provider this page signs with */
let providerIsPhone = false;
const watched = new WeakSet();
/* `rdns` undefined leaves the remembered pick alone (a silent resume). */
function useWallet(p, rdns, isPhone = false) {
  provider = p;
  providerIsPhone = isPhone;
  if (rdns !== undefined) remember(rdns);
  if (typeof p.on !== "function" || watched.has(p)) return;
  watched.add(p);
  /* Somebody switches account in the extension while the page is open. A
     provider event may not sign a signed-out reader back in: some wallets
     fire it on unlock and on tab focus. */
  p.on("accountsChanged", (a) => {
    if (p === provider) setAddress(signedOut() ? null : (a && a[0]) || null);
  });
  /* the phone ended the session */
  if (isPhone) {
    p.on("disconnect", () => {
      if (p !== provider) return;
      provider = null;
      providerIsPhone = false;
      remember("");
      setAddress(null);
    });
  }
}

/* The remembered wallet if it is here, the only one if there is one,
   otherwise the picker. Null when the player closes it. */
async function pickWallet() {
  const all = wallets();
  const can = all.filter((w) => !w.solanaOnly);
  const kept = remembered();
  const known = kept && can.find((w) => w.rdns === kept);
  if (known) return known;
  if (can.length === 1 && can[0].provider) return can[0];
  if (!all.length) return null;
  return askWhich(all);
}

function askWhich(all) {
  const dlg = $("wallet-pick");
  return new Promise((done) => {
    let chosen = null;
    $("wallet-list").replaceChildren(...all.map((w) => {
      const b = document.createElement("button");
      b.type = "button";
      /* EIP-6963 icons are data URIs, which the page's img-src allows */
      if (/^data:image\//.test(w.icon)) {
        const img = document.createElement("img");
        img.alt = "";
        img.src = w.icon;
        b.append(img);
      } else {
        const ph = document.createElement("span");
        ph.className = "ph";
        b.append(ph);
      }
      const t = document.createElement("span");
      const n = document.createElement("b");
      n.textContent = w.name;
      t.append(n);
      const note = w.solanaOnly ? "Solana only — needs an Ethereum account"
        : w.phone ? "Scan a code with your wallet app" : "";
      if (note) {
        const sm = document.createElement("small");
        sm.textContent = note;
        t.append(sm);
      }
      b.append(t);
      if (w.solanaOnly) b.disabled = true;
      else b.addEventListener("click", () => { chosen = w; dlg.close(); });
      return b;
    }));
    $("wallet-note").hidden = !all.some((w) => w.solanaOnly);
    dlg.addEventListener("close", () => done(chosen), { once: true });
    dlg.showModal();
  });
}
$("wallet-cancel").addEventListener("click", () => $("wallet-pick").close());

const lower = (list) => (list || []).map((a) => String(a).toLowerCase());
const wallet = {
  has: () => usable().length > 0,
  async accounts(p = provider) {
    if (!p) return [];
    try { return (await p.request({ method: "eth_accounts" })) || []; }
    catch (err) { return []; }
  },
  async connect() {
    const w = await pickWallet();
    if (!w && wallet.has()) throw Object.assign(new Error("no wallet picked."), { cancelled: true });
    if (!w) throw new Error(noWalletLine());
    const p = w.phone ? await (await phone()).connect(phoneOpts()) : w.provider;
    const a = await p.request({ method: "eth_requestAccounts" });
    if (!a || !a[0]) throw new Error("no account");
    useWallet(p, w.rdns, !!w.phone);
    return a[0];
  },
  async sign(message, address) {
    if (!provider) throw new Error("no wallet is connected. Press Sign in, then try again.");
    const want = String(address || "").toLowerCase();
    let have = lower(await wallet.accounts());
    /* A locked wallet answers [] to the silent read. Asking for accounts is
       what opens its unlock screen, so ask rather than refuse. */
    if (want && !have.length) {
      try { have = lower(await provider.request({ method: "eth_requestAccounts" })); }
      catch (err) { throw new Error(walletSaid(err)); }
    }
    if (want && !have.length)
      throw new Error("your wallet is locked or not connected to this site — open it, then try again.");
    if (want && !have.includes(want))
      throw new Error("your wallet is on " + short(have[0]) + ", and this asks "
        + short(want) + " to sign — switch your wallet to that address and try again.");
    const hex = "0x" + Array.from(new TextEncoder().encode(message))
      .map((b) => b.toString(16).padStart(2, "0")).join("");
    try { return await provider.request({ method: "personal_sign", params: [hex, address] }); }
    catch (err) { throw new Error(walletSaid(err)); }
  },
};
/* A wallet's own refusal, in words: 4001 is "no thanks", -32002 is a request
   already open (usually a window behind the browser). Same table as the
   platform's `Deck.purse.refusal`. */
const walletSaid = (err) => err && err.code === 4001 ? "you said no in your wallet."
  : err && err.code === -32002 ? "your wallet already has a request open — check its window, then try again."
  : (err && err.message) || String(err);
/* What a browser with no wallet that can sign in is told. */
function noWalletLine() {
  const sol = solanaOnly()[0];
  return sol
    ? `${sol.name} only holds Solana here, and Gates signs in with an Ethereum account, so you play as a guest. `
      + "Phantom, MetaMask and Backpack hold both: turn Ethereum on in the wallet's settings, then reload."
    : "No wallet in this browser, so you play as a guest and keep nothing. To keep your base, add one like MetaMask or Rabby, then reload.";
}

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

/* Already signed in, silently — never a prompt. The remembered wallet first,
   then the one in `window.ethereum` (the slot the platform's own pages read),
   then any other that already lets this site see an account. */
function resumeOrder() {
  const all = wallets();
  const kept = remembered();
  const out = all.filter((w) => kept && w.rdns === kept);
  if (window.ethereum && !out.some((w) => w.provider === window.ethereum)) {
    out.push(all.find((w) => w.provider === window.ethereum) || { provider: window.ethereum });
  }
  return out.concat(all.filter((w) => !out.some((o) => o.provider === w.provider)));
}

/* Two of these may run at once (a late wallet starts its own), so each one
   re-checks `address` after every await and the first to find one wins. */
async function resume() {
  if (signedOut()) return;
  /* a phone session this browser already has; never a QR code */
  if (remembered() === PHONE && phoneOn()) {
    try {
      const p = await (await phone()).restore(phoneOpts());
      const a = p && p.accounts && p.accounts[0];
      if (a && !address && !signedOut()) { useWallet(p, undefined, true); setAddress(a); return; }
    } catch (err) { /* no session, or the relay is away: the extensions below */ }
  }
  /* MetaMask answers [] for the first moments of a page and the real account
     after (`store.js` §silentAccount measured it), so a miss is asked again. */
  for (const wait of [0, 400, 1500]) {
    if (wait) await sleep(wait);
    if (address || signedOut()) return;
    for (const w of resumeOrder()) {
      const a = (await wallet.accounts(w.provider))[0];
      if (address || signedOut()) return;
      if (a) { useWallet(w.provider); setAddress(a); return; }
    }
  }
}

/* A wallet that injected after the page started. */
function walletAppeared() {
  paintAccount();
  if (!address) resume();
}

async function signIn() {
  try {
    const a = await wallet.connect();
    setSignedOut(false);
    setAddress(a);
    return a;
  } catch (err) {
    say(err && err.cancelled ? "" : walletSaid(err), err && err.cancelled ? "" : "bad");
    return null;
  }
}

async function signOut() {
  setSignedOut(true);
  try {
    Object.keys(sessionStorage).filter((k) => k.startsWith("elo.bal.")).forEach((k) => sessionStorage.removeItem(k));
  } catch (err) { /* private mode */ }
  const p = provider;
  const wasPhone = providerIsPhone;
  provider = null;
  providerIsPhone = false;
  /* the next sign-in may pick another wallet */
  remember("");
  setAddress(null);
  /* a phone session is ended rather than left paired */
  if (wasPhone && window.EloWalletConnect) { await window.EloWalletConnect.disconnect(); return; }
  /* the real revocation where the wallet has one (EIP-2255); the flag above
     is what makes this work on the wallets that do not */
  try { if (p) await p.request({ method: "wallet_revokePermissions", params: [{ eth_accounts: {} }] }); }
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
      line.textContent = noWalletLine();
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
      say(noWalletLine(), "bad");
      return;
    }
    b.disabled = true;
    await signIn();
    b.disabled = false;
  });
}

/* ── the download ──────────────────────────────────────────────────────────
   The module is the game — Bevy, the transport and the client core, ~34 MB
   before gzip — and it starts downloading the moment the page opens (on a
   phone, when PLAY is pressed: `start`, below), with the bar at the bottom
   and the PLAY button showing how far it is. PLAY pressed early is kept, and
   goes the moment the module is ready.

   `build.json` sits beside the module (`ci/build_web.sh`) and carries its
   size: with gzip on the wire, Content-Length is the compressed size and the
   bytes read here are the decompressed ones, so the header cannot be the
   denominator. No `build.json` means a bar that counts megabytes instead. */
let glue = null;     /* the wasm-bindgen module, once instantiated */
let dl = { started: false, got: 0, want: 0, done: false, failed: null };
const MB = (n) => (n / 1048576).toFixed(1);

/* `build.json`, read once at load whether or not the module is wanted yet: it
   is the page's own wire (`proto`, held against each shard's), the footer's
   build line, the module sizes and the audio pair's hash. No-cache, like the
   page: it is what says which files the page asks for. */
const built = (async () => {
  try {
    const r = await fetch("build.json", { cache: "no-cache" });
    return r.ok ? await r.json() : null;
  } catch (err) {
    return null;   /* a dev build without it: the bar counts megabytes */
  }
})();
const buildBits = (b) => b ? [b.version && "v" + b.version, b.commit, b.proto && "wire " + b.proto].filter(Boolean) : [];
built.then((b) => {
  if (b && Number.isInteger(b.proto)) pageProto = b.proto;
  if (!dl.started) $("build").textContent = ("Gates " + buildBits(b).join(" · ")).trim();
  renderServers();
});

function paintDownload() {
  const box = $("dl");
  const pct = dl.want ? Math.min(1, dl.got / dl.want) : 0;
  box.classList.toggle("done", dl.done);
  $("dl-bar").style.width = dl.done ? "100%" : !dl.started ? "0" : (dl.want ? (pct * 100).toFixed(1) + "%" : "8%");
  $("dl-what").textContent = dl.failed ? "Download failed" : dl.done ? "Ready" : !dl.started ? "Game files"
    : dl.want && dl.got >= dl.want ? "Preparing" : "Downloading";
  $("dl-n").textContent = dl.done || dl.failed || !dl.started ? ""
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
   time, so the page picks, before the download. A browser gets the WebGPU
   module when it hands back a hardware adapter with filterable 32-bit
   floats: Bevy binds the atmosphere's `Rgba32Float` tables as filterable and
   never checks, so without them the WebGPU frame is lost rather than
   degraded. A software adapter (`isFallbackAdapter`) gets WebGL2 too — the
   desktop's frame on a CPU is slower than the cheap one. Everything the
   device can or cannot bind past that, Rust reads off the device itself
   (`render::quality::GpuCaps`).

   `?gfx=webgl2` or `?gfx=webgpu` forces one. A WebGPU module that failed on
   this browser is remembered against that BUILD (`gates.webgpu.failed`), so
   the next visit goes straight to WebGL2 and a new publish tries again;
   `?gfx=webgpu` forgets it. An older `build.json` that names no modules is
   the WebGL2 module alone. */
const MODULES = {
  webgl2: { js: "./client_web.js", wasm: "client_web_bg.wasm", name: "WebGL2" },
  webgpu: { js: "./client_web_gpu.js", wasm: "client_web_gpu_bg.wasm", name: "WebGPU" },
};
const GPU_FAILED = "gates.webgpu.failed";
/* Started at load, beside the `build.json` fetch, rather than after it. */
const gpuProbe = (async () => {
  if (!navigator.gpu) return { ok: false, why: "no WebGPU in this browser" };
  try {
    /* A blocklisted GPU answers null; a wedged one may never answer. */
    const adapter = await Promise.race([
      navigator.gpu.requestAdapter({ powerPreference: "high-performance" }),
      new Promise((r) => setTimeout(() => r(null), 3000)),
    ]);
    if (!adapter) return { ok: false, why: "no WebGPU adapter" };
    const info = adapter.info || {};
    if (info.isFallbackAdapter || adapter.isFallbackAdapter) return { ok: false, why: "software WebGPU adapter" };
    if (!adapter.features.has("float32-filterable")) return { ok: false, why: "no float32-filterable" };
    return { ok: true, why: "WebGPU adapter" };
  } catch (err) {
    return { ok: false, why: "WebGPU adapter refused" };
  }
})();
const moduleKey = (sizes) => sizes ? String(sizes.sha256 || sizes.wasm_bytes || "") : "";

async function pickModule(built) {
  const has = (k) => (built ? !!built[k] : k === "webgl2");
  const forced = q.get("gfx");
  if (forced === "webgpu") { try { localStorage.removeItem(GPU_FAILED); } catch (err) { /* private mode */ } }
  if ((forced === "webgl2" || forced === "webgpu") && has(forced)) return { gfx: forced, why: "forced by ?gfx" };
  if (!has("webgpu")) return { gfx: "webgl2", why: "this build has no WebGPU module" };
  if (!has("webgl2")) return { gfx: "webgpu", why: "this build has no WebGL2 module" };
  try {
    if (localStorage.getItem(GPU_FAILED) === moduleKey(built.webgpu)) {
      return { gfx: "webgl2", why: "WebGPU failed here before" };
    }
  } catch (err) { /* private mode */ }
  const probe = await gpuProbe;
  return { gfx: probe.ok ? "webgpu" : "webgl2", why: probe.why };
}

/* Back to the page and into the WebGL2 module, once: the failure is written
   against this build before the reload, and the WebGL2 module has no hook
   that could send it back, so this cannot loop. The session is spent — a
   canvas that has held a WebGPU context never takes a WebGL2 one, and
   Bevy's `run()` does not return — so the way there is a reload. */
let builtModules = null;
let playStarted = 0;
function gpuFallBack(why) {
  const key = moduleKey(builtModules && builtModules.webgpu);
  let kept = false;
  try {
    localStorage.setItem(GPU_FAILED, key);
    kept = localStorage.getItem(GPU_FAILED) === key;
  } catch (err) { /* storage blocked */ }
  /* Where the flag will not hold — storage refused, or an address that
     forces WebGPU — the address itself carries the switch, or the reload
     would land on WebGPU again. */
  if ((!kept || q.get("gfx") === "webgpu") && builtModules && builtModules.webgl2) {
    const u = new URL(location.href);
    u.searchParams.set("gfx", "webgl2");
    history.replaceState(history.state, "", u);
  }
  window.gatesLeft(`${why} — switched to WebGL2`);
}

/* The WebGPU module's error hook (`render/web.rs`, `watch_gpu`). WebGPU
   refuses a pipeline quietly and the frame is simply not drawn. A LOST
   device draws nothing ever again, so that one reloads into WebGL2; a
   refusal puts a line over the canvas offering it, and is remembered. */
let gpuTrouble = false;
window.gatesGpuError = (msg, kind) => {
  if (window.gatesGfx !== "webgpu") return;
  if (kind === "lost") { gpuFallBack("the graphics device was lost under WebGPU"); return; }
  try { localStorage.setItem(GPU_FAILED, moduleKey(builtModules && builtModules.webgpu)); } catch (err) { /* private mode */ }
  if (gpuTrouble) return;
  gpuTrouble = true;
  const bar = document.createElement("div");
  bar.setAttribute("role", "alert");
  bar.style.cssText = "position:fixed;left:50%;top:12px;transform:translateX(-50%);z-index:50;"
    + "max-width:min(92vw,640px);padding:10px 14px;border-radius:6px;background:rgba(24,20,16,.92);"
    + "color:#f2e9dc;font:14px/1.4 var(--sans);box-shadow:0 2px 12px #0008";
  const link = document.createElement("a");
  const u = new URL(location.href);
  u.searchParams.set("gfx", "webgl2");
  link.href = u.toString();
  link.textContent = "Reload with WebGL2";
  link.style.cssText = "color:#ffcf7a;margin-left:8px";
  /* Leaving is the point of this link: the player's leave-site guard must
     not ask about it. */
  link.addEventListener("click", () => {
    window.removeEventListener("beforeunload", guardLeave);
    try { sessionStorage.setItem(LEFT, "WebGPU refused part of the frame — switched to WebGL2"); } catch (err) { /* private mode */ }
  });
  bar.append("The graphics card refused part of the frame under WebGPU (press Esc to click).", link);
  bar.title = String(msg).slice(0, 400);
  document.body.append(bar);
};
/* A WebGPU module whose renderer never started: Bevy's "Unable to find a
   GPU!" and its surface and device requests panic before the hook above
   exists. `client-web`'s panic hook calls this with the panic's text, and
   only a panic in Bevy's renderer set-up demotes the build — any other one
   leaves its message in the console for whoever reads it. */
window.gatesPanic = (msg) => {
  const m = String(msg || "");
  const renderer = /bevy_render[^\n]*renderer|Unable to find a GPU|wgpu surface|request_device/.test(m);
  if (window.gatesGfx === "webgpu" && renderer && playStarted && performance.now() - playStarted < 30000) {
    gpuFallBack("WebGPU did not start in this browser");
  }
};

async function load(gfx, sizes) {
  const m = MODULES[gfx];
  dl.got = 0;
  dl.want = sizes && Number(sizes.wasm_bytes) > 0 ? Number(sizes.wasm_bytes) : 0;
  paintDownload();
  /* The glue and its module are a pair (wasm-bindgen hashes the import
     names), so both are asked for by the pair's own hash: a cached glue from
     the last publish cannot meet this publish's module. */
  const v = sizes && sizes.sha256 ? "?v=" + String(sizes.sha256).slice(0, 12) : "";
  const mod = await import(m.js + v);
  const res = fetch(new URL(m.wasm + v, import.meta.url)).then(counted);
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
  dl.started = true;
  paintDownload();
  const b = await built;
  const bits = buildBits(b);
  if (b) {
    builtModules = b.modules || null;
    if (!builtModules && Number(b.wasm_bytes) > 0) builtModules = { webgl2: { wasm_bytes: b.wasm_bytes } };
  }
  let { gfx, why } = await pickModule(builtModules);
  const footer = () => {
    $("build").textContent = "Gates " + [...bits, MODULES[gfx].name].join(" · ");
    $("build").title = why;
  };
  footer();
  paintDownload();
  try {
    try {
      glue = await load(gfx, builtModules && builtModules[gfx]);
    } catch (err) {
      /* A WebGPU module that will not compile or instantiate here still
         leaves the WebGL2 one, and a player should get that rather than an
         error. Failures after this are Bevy's: `gatesPanic` and
         `gatesGpuError` above. */
      if (gfx !== "webgpu" || !(builtModules && builtModules.webgl2)) throw err;
      console.warn("gates: the WebGPU module failed, loading WebGL2", err);
      gfx = "webgl2";
      why = "the WebGPU module did not load";
      footer();
      glue = await load(gfx, builtModules && builtModules[gfx]);
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
    /* The script and its module are a pair, as the game's glue and module
       are, so both are asked for by the pair's own hash (`build.json`'s
       `audio.sha256`, `ci/build_web.sh`): a cached `audio.js` from the last
       publish cannot meet this publish's `.wasm`. */
    const b = await built;
    const v = b && b.audio && b.audio.sha256 ? "?v=" + String(b.audio.sha256).slice(0, 12) : "";
    /* `addModule` loads the CONCATENATION of `audio-prelude.js`, the
       sound-worklet glue and `audio-processor.js` (see `ci/build_web.sh`) —
       one script, no imports, because a worklet scope has no module graph
       worth relying on. */
    await ctx.audioWorklet.addModule("./audio.js" + v);
    /* Fetched HERE, because a worklet scope has no `fetch`, and posted down
       the port as BYTES, transferred, for the worklet to compile itself.
       ⚠ **Not as a compiled `WebAssembly.Module`**: Chromium 151 does not
       deserialize a Module in the AudioWorklet scope, so the processor got
       `messageerror`, never ran `init`, and the page was silent (measured
       2026-09-17). */
    const res = await fetch("./sound_worklet_bg.wasm" + v);
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

/* ── the score, started inside the gesture ─────────────────────────────────
   The music is whole tracks, four to five minutes each, streamed by the
   browser's own `<audio>` element from `assets/music/` rather than decoded
   into the game's memory (`crates/sound/src/music.rs`). The intro starts HERE,
   inside the PLAY or WATCH press, because a page may only start sound inside
   a gesture — so it plays through the download, the join and the loading
   screen. `render/audio_web.rs` takes it over when the game boots: it reads
   `track` and `time()` to know how far in it is, and from then on calls
   `play`, `stop` and `gain`. Volume is the element's own (`volume`, linear):
   the game sends the level every frame, fades included. Never fatal: a
   browser that cannot play Ogg Vorbis plays the game without music. */
const INTRO = "music/safe_zone.ogg";
const MUSIC_DEFAULT = 0.2;   /* `sound::MUSIC_DEFAULT`, until the game sends its own */
let music = null;
function startMusic() {
  if (music) return music;
  const el = new Audio();
  el.preload = "auto";
  el.volume = MUSIC_DEFAULT;
  const canOgg = el.canPlayType('audio/ogg; codecs="vorbis"') !== "";
  if (!canOgg) console.warn("gates music: this browser cannot play Ogg Vorbis; no score");
  let blocked = false;
  const kick = () => {
    const p = el.play();
    if (p && p.catch) p.catch(() => { blocked = true; });
  };
  /* A play refused for want of a gesture is retried on the next one. */
  const retry = () => { if (blocked && music.track) { blocked = false; kick(); } };
  addEventListener("pointerdown", retry, { passive: true });
  addEventListener("keydown", retry, { passive: true });
  music = {
    track: null,
    play(file) {
      if (!canOgg) return;
      music.track = file;
      el.src = "assets/" + file;
      kick();
    },
    stop() {
      music.track = null;
      el.pause();
      el.removeAttribute("src");
      el.load();
    },
    gain(g) {
      el.volume = Math.min(1, Math.max(0, Number(g) || 0));
    },
    time() { return music.track ? el.currentTime || 0 : 0; },
  };
  el.addEventListener("ended", () => { music.track = null; });
  globalThis.gatesMusic = music;
  music.play(INTRO);
  return music;
}

/* ── PLAY and WATCH ────────────────────────────────────────────────────────
   One button each, and each draws its own state: how far the download is,
   who it will sign in as, and what it is waiting on once pressed. */
let busy = null;        /* null, or { label, sub } while a join is in flight */
const canTransport = "WebTransport" in window;
const touchOnly = matchMedia("(pointer: coarse)").matches && !matchMedia("(any-pointer: fine)").matches;

function paintButton(btn, fill, sub, idleSub) {
  const loading = dl.started && !dl.done && !dl.failed;
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

/* On a phone the page does not download the game until asked (`start`,
   below): PLAY says what it needs, and pressing it anyway is the opt-in. */
const askFirst = () => touchOnly && !EMBED && !dl.started;

function paintGo() {
  const idle = askFirst() ? "needs a keyboard and mouse"
    : address ? `as ${(who && who.name) || short(address)}`
    : wallet.has() && !EMBED ? "signs in with your wallet" : "as a guest";
  paintButton($("go"), $("go-fill"), $("go-sub"), idle);
  $("go-label").textContent = busy ? busy.label : dl.failed ? "Reload" : "Play";
  paintButton($("watch-go"), $("watch-fill"), $("watch-sub"), "no wallet needed");
  /* A picked shard on a newer wire than this page refuses it at the
     handshake, so PLAY and WATCH become the reload that fixes it. */
  if (!busy && canTransport && staleFor(current())) {
    for (const [btn, fill, sub] of [[$("go"), $("go-fill"), $("go-sub")], [$("watch-go"), $("watch-fill"), $("watch-sub")]]) {
      btn.classList.remove("wait");
      fill.style.width = "0";
      sub.textContent = "Gates just updated";
    }
    $("go-label").textContent = "Reload";
  }
  $("send").hidden = !touchOnly || EMBED;
  /* The other way in, under PLAY. The public shard takes guests again
     (`shard-public.toml`, 2026-10-07): a guest plays a fresh character and
     keeps nothing, a wallet keeps your base. The full page leads with the
     wallet and offers the guest; the embed leads with the guest. */
  const other = !address && wallet.has() && !busy;
  $("guest").hidden = EMBED || !other;
  $("wallet-in").hidden = !EMBED || !other;
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
  /* A wallet asked to sign means the shard answered: whatever fails after
     that is not the network, and its own words say what it was. */
  let walletAsked = false;
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
          walletAsked = true;
          busy = { label: "Signing in", sub: "approve the sign-in in your wallet" };
          paintGo();
          return wallet.sign(text, identity);
        }
        : undefined);
    }
  } catch (e) {
    const raw = e && e.message !== undefined ? String(e.message) : String(e);
    const code = e && Number.isInteger(e.code) ? e.code : null;
    console.warn("gates: the join failed -", raw);
    if (code === REFUSE_VERSION && skewOf(row) === "older") {
      /* The shard is the one behind: a reload would change nothing. */
      say("This server is updating to the new Gates. Try again in a few minutes.", "bad", line);
    } else if (code === REFUSE_VERSION && skewOf(row) === "newer" && reloadedFor(row)) {
      /* Already reloaded for this wire and still behind: the shard moved
         before the browser build was published. Another reload is the loop. */
      say("Gates just updated. The browser version follows soon. Try again in a few minutes.", "bad", line);
    } else if (code === REFUSE_AUTH && walletAsked) {
      /* The wallet signed and the shard recovered somebody else. The shared
         sentence says "connect a wallet", which this player just did. */
      say("Your wallet signed, but the server couldn't match it to your address. Smart-contract and "
        + "multisig accounts can't sign in yet — switch to a regular account and press Play again.", "bad", line);
      line.title = raw;
    } else if (code !== null || walletAsked || !(e instanceof Error) || !(row && row.statusUrl)) {
      say(raw, "bad", line);
    } else {
      /* No code: the shard never answered. Its status endpoint is plain
         HTTPS, so asking it again splits "the server is down" from "this
         network will not carry QUIC" (UDP), which the browser's own error
         does not. The browser's words stay on the line's title. */
      busy = { label: "Connecting", sub: "checking the server" };
      paintGo();
      const up = await pollStatus(row, 6000);
      say(up && staleFor(row) ? "Gates just updated. Reload the page to play."
        : up ? "Couldn't reach the game server from this network. Gates uses UDP (QUIC), which some office, school and VPN networks block."
        : "The server isn't answering. It may be restarting. Try again in a minute.", "bad", line);
      line.title = raw;
    }
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
  playStarted = performance.now();
  g.play("#gates");
}

/* ── fullscreen that Esc does not end ───────────────────────────────────────
   Esc is the game's back key (the menu, the settings screen, a panel), and a
   browser also ends a page's fullscreen on Esc. Two answers:
   - F11 stays the browser's own fullscreen, which Esc does not end. winit
     calls preventDefault on every key the canvas gets, so F11 is stopped
     here, before the canvas, and the browser keeps it (the desktop client
     binds F11 itself: `settings::FULLSCREEN_KEY`).
   - FULLSCREEN on the settings screen is the page's (the Fullscreen API).
     While it is up, Keyboard Lock (Chromium) hands Esc to the game, and
     holding Esc still leaves, as the browser says. Where there is no lock,
     the settings screen's BACK button is the way out that keeps it. */
const keepF11 = (ev) => { if (ev.key === "F11") ev.stopImmediatePropagation(); };
window.addEventListener("keydown", keepF11, true);
window.addEventListener("keyup", keepF11, true);
document.addEventListener("fullscreenchange", () => {
  const kb = navigator.keyboard;
  if (!kb || !kb.lock) return;
  if (document.fullscreenElement) kb.lock(["Escape"]).catch(() => {});
  else kb.unlock();
});

async function play({ guest = false } = {}) {
  if (busy || !canTransport) return;
  if (dl.failed) { location.reload(); return; }
  if (staleFor(current())) { freshReload(current()); return; }
  const audio = startAudio();   /* synchronously inside the press */
  startMusic();
  let identity = guest ? null : address;
  /* The wallet may have moved account without telling the page (an event
     missed while the tab slept): play as the account it is on now, rather
     than ask it to sign as one it has left. */
  if (identity && provider) {
    busy = { label: "Play", sub: "checking your wallet" };   /* a second press waits */
    paintGo();
    const now = (await Promise.race([wallet.accounts(), sleep(2000).then(() => [])]))[0];
    busy = null;
    if (now && String(now).toLowerCase() !== identity.toLowerCase()) { setAddress(now); identity = now; }
  }
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
    if (!(await ready())) { busy = null; paintGo(); return; }
  }
  await join("play", audio, identity);
}

async function watch() {
  if (busy || !canTransport) return;
  if (dl.failed) { location.reload(); return; }
  if (staleFor(current())) { freshReload(current()); return; }
  const audio = startAudio();
  startMusic();
  if (!glue) {
    busy = { label: "Watch", sub: "starting when the download finishes" };
    paintGo();
    if (!(await ready())) { busy = null; paintGo(); return; }
  }
  await join("watch", audio, null);
}

/* PLAY itself: in the embed a guest unless a wallet is already signed in. */
const goPlay = () => play({ guest: EMBED && !address });
$("go").addEventListener("click", goPlay);
$("guest").addEventListener("click", () => play({ guest: true }));
$("wallet-in").addEventListener("click", () => play());
$("watch-go").addEventListener("click", () => watch());
/* A phone cannot play, but it found the game: hand the link to the machine
   that can — the share sheet where there is one, the clipboard where not. */
$("send").addEventListener("click", async () => {
  if (navigator.share) {
    try {
      await navigator.share({ title: "Gates", text: "Gates, a survival game you play in the browser", url: GAME_URL });
      return;
    } catch (err) {
      if (err && err.name === "AbortError") return;   /* closed the sheet */
    }
  }
  copyLink(GAME_URL, "Link copied. Open it on your computer to play.");
});
$("watch-who").addEventListener("keydown", (ev) => { if (ev.key === "Enter") watch(); });
document.addEventListener("keydown", (ev) => {
  if (ev.key !== "Enter" || ev.repeat || busy) return;
  const t = ev.target;
  if (t && (t.tagName === "INPUT" || t.tagName === "BUTTON" || t.tagName === "A")) return;
  const open = panes.find((p) => !p.hidden);
  if (open && open.dataset.pane === "play") goPlay();
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
/* Wallets already in the page answer this at once; later ones announce
   themselves when they load. */
window.dispatchEvent(new Event("eip6963:requestProvider"));
try { window.dispatchEvent(new CustomEvent("wallet-standard:app-ready", { detail: standard })); }
catch (err) { /* no Solana rows, nothing else lost */ }
pageReady = true;
/* the phone row appears once the platform's config says it may */
loadWalletConfig().then(() => { if (phoneOn()) walletAppeared(); });
rows = baseRows();
picked = rows[0].id;
renderServers();
paintAccount();
// WebTransport is the only path a page has to a QUIC shard, and it is not
// everywhere yet. Say so before the button is pressed rather than after.
if (!canTransport) say("This browser has no WebTransport, which Gates needs to reach its servers. Chrome, Edge and Firefox have it.", "bad");
else if (cameBack) say(cameBack + " — press Play to go back in.");
else if (touchOnly && EMBED) say("Gates plays with a keyboard and mouse.");
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
/* **The download, and on a phone only when asked.** The module is ~34 MB a
   phone cannot play, so a touch-only visitor downloads nothing until PLAY
   (or WATCH) says to; the embed and a player coming back from the world
   start at once, as everyone else does. */
let booting = null;
const ready = () => booting || (booting = boot());
if (!touchOnly || EMBED || cameBack) ready();
paintDownload();
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
