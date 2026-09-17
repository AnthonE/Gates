import init, { Gates } from "./client_web.js";

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

/* ⚠ NOTHING IN THIS FILE MAY `await` AT THE TOP LEVEL BEFORE THE LISTENERS ARE
   WIRED, and the page shipped for a week doing exactly that. `await init()`
   sat above the Play handler, so the button existed, said "Play", was not
   disabled — and had no click listener at all until 34 MB of module had
   landed. A player pressing it in the first twenty seconds got nothing: no
   sound, no message, no state change. The module is now loaded by a promise
   (`loadModule` below) and the button is wired and HONEST about its state from
   the first paint: disabled while the module lands, showing what it is waiting
   for, and remembering a press that arrives early. */

const $ = (id) => document.getElementById(id);
const status = $("status");
const go = $("go");
const bar = $("bar");
const barfill = $("barfill");
const connect = $("connect");
const guest = $("guest");
const account = $("account");
const chip = $("chip");
const chipText = $("chip-text");
const advanced = $("advanced");
const shardBox = $("shards");
const say = (text, cls) => { status.textContent = text; status.className = cls ?? ""; };
const short = (a) => String(a).slice(0, 6) + "…" + String(a).slice(-4);
const mb = (n) => (n / 1048576).toFixed(1);

/* ── where to dial ─────────────────────────────────────────────────────────
   Served from elopros.com the page dials the public shard, whose Let's
   Encrypt chain a browser trusts outright, so the hash field stays empty;
   served from anywhere else (a checkout, `python3 -m http.server`) it dials
   the dev default and expects the self-signed shard's hash. `?server=` and
   `?hash=` override either, which is what a join link is. The fields live
   behind ADVANCED: a player on the origin never needs them, and a developer
   off it gets them open, because a dev shard's hash has to be typed. */
const q = new URLSearchParams(location.search);
const onOrigin = /(^|\.)elopros\.com$/.test(location.hostname);
const DEFAULT_SHARD = onOrigin ? "game.elopros.com:61234" : "127.0.0.1:4433";
const serverField = $("server");
const hashField = $("hash");

/* The shard somebody played last, so a second visit lands where the first one
   did. Per browser and nowhere else — this is a convenience, not an identity,
   and a `?server=` link always wins over it. */
const LAST_SHARD = "gates.shard";
const remembered = () => {
  try { return localStorage.getItem(LAST_SHARD) || ""; } catch { return ""; }
};
const remember = (addr) => {
  try { localStorage.setItem(LAST_SHARD, addr); } catch { /* private mode */ }
};

serverField.value = q.get("server") || remembered() || DEFAULT_SHARD;
hashField.value = q.get("hash") || "";
advanced.open = !onOrigin && !q.has("server");

/* Off the origin these point at pages that do not exist. Strip rather than
   ship a dead link — a 404 in the corner of a dev checkout is the kind of
   small wrongness that gets copied into a screenshot. */
if (!onOrigin) {
  for (const id of ["home", "listing"]) {
    const a = $(id);
    a.removeAttribute("href");
    a.style.pointerEvents = "none";
  }
  chip.removeAttribute("href");
}

/* ── coming back ───────────────────────────────────────────────────────────
   The page is the menu on this target: leaving the world — the Esc menu's
   DISCONNECT, Escape on the loading screen, the shard hanging up, QUIT —
   hands control back here (`client::render::web::hand_back`) by calling
   `gatesLeft(status)`. The renderer cannot be handed a second session on
   the same canvas (Bevy's `run()` never returns on wasm and a second `App`
   over the first's context is two owners of one WebGL context), so the
   page notes why and RELOADS, and says so on the way back in. Without the
   hook the renderer reloads by itself and the reason is lost. */
const LEFT = "gates.left";
window.gatesLeft = (why) => {
  try { sessionStorage.setItem(LEFT, String(why || "left the world")); } catch {}
  location.reload();
};
let leftWhy = "";
try {
  leftWhy = sessionStorage.getItem(LEFT) || "";
  if (leftWhy) sessionStorage.removeItem(LEFT);
} catch {}

/* WebTransport is the only path a page has to a QUIC shard, and it is not
   everywhere yet. Say so before the button is pressed rather than after, and
   latch it: nothing below may hand this browser a working Play button. */
let blocked = false;
if (!("WebTransport" in window)) {
  blocked = true;
  go.disabled = true;
  go.textContent = "Not supported here";
  say("This browser has no WebTransport. Gates speaks QUIC and a page has no other way to reach it.", "bad");
}

/* ── the wallet ────────────────────────────────────────────────────────────
   Deliberately the same calls as the platform's own `Deck.wallet`
   (`watchtower/js/deck-shim.js` in AnthonE/scry-forge): has, accounts,
   connect, sign. It stays a copy rather than a `<script src="/js/deck-shim.js">`
   for one reason — this page has to work off the origin, in a checkout with no
   platform beside it, and a shim that 404s there would take the wallet with
   it. What IS shared is the thing that has to be: the sign-out key, below,
   which is a STRING both sides agree on (`grep -rn elo.signedout` finds every
   end of it) — the same way store.js and deck-shim.js share it with each
   other.

   The address check before signing is copied verbatim for the reason its own
   comment gives: somebody switches account in the extension, the page still
   holds the old address, and `personal_sign` then fails with a
   provider-internal error naming nothing anyone can act on.

   WHAT THIS DOES NOT DO, and must not: compose the message. The text comes
   out of Rust (`protocol::siwe_message`, through `Proof::message`) because
   the shard rebuilds the same bytes to verify them, and a second copy of an
   EIP-4361 message in JavaScript would drift into every login failing as
   `WrongSigner`. This function receives a finished string and hands it to
   the wallet, which renders it for the person about to approve it. */
const wallet = {
  has: () => !!window.ethereum,
  async accounts() {
    if (!wallet.has()) return [];
    try { return (await window.ethereum.request({ method: "eth_accounts" })) || []; }
    catch { return []; }
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
  /* A refusal in words the reader can act on — `Deck.purse.refusal`'s two
     codes. 4001 is the ordinary "no thanks"; -32002 is the one that reads as
     a broken button, because the wallet already has a window open (usually
     behind the browser) and it is what an impatient second click produces. */
  refusal(e) {
    if (e && e.code === 4001) return "you refused that in your wallet.";
    if (e && e.code === -32002) return "your wallet already has a request open — check its window, then try again.";
    return (e && e.message) || String(e);
  },
};

/* ── signed out ────────────────────────────────────────────────────────────
   THE SAME KEY the platform writes (scry-forge `js/store.js` and
   `js/deck-shim.js`). This page is served from elopros.com, so it is the same
   localStorage as the store — which is the whole of why the resume below
   needs it. Without the check, pressing SIGN OUT in the store masthead and
   then opening the game would silently sign you back in, because
   `eth_accounts` still answers for a site the wallet has authorised. Sign-out
   is a decision about this browser, not about one page. */
const SIGNED_OUT = "elo.signedout";
const signedOut = () => {
  try { return localStorage.getItem(SIGNED_OUT) === "1"; }
  catch { return false; }
};

let address = null;   /* the connected wallet, or null for a guest join */
let guesting = false; /* chose to play as a guest with a wallet available */

function line(el, ...parts) { el.replaceChildren(...parts); }
function addr(a) {
  const s = document.createElement("span");
  s.className = "addr";
  s.textContent = short(a);
  return s;
}

function showAccount() {
  if (address) {
    line(account, addr(address),
      document.createTextNode(" — the shard will ask this wallet to prove it."));
    account.className = "good";
    account.title = address;
    connect.textContent = "Change wallet";
    guest.hidden = false;
    chip.dataset.state = "here";
    chipText.textContent = short(address);
  } else if (!wallet.has()) {
    account.textContent = "No wallet extension in this browser — you can still join a shard that takes guests.";
    account.className = "";
    account.removeAttribute("title");
    connect.disabled = true;
    connect.textContent = "Connect wallet";
    guest.hidden = true;
    chip.dataset.state = "none";
    chipText.textContent = "no wallet";
  } else {
    account.textContent = guesting
      ? "Playing as a guest. A shard that takes guests will let you in."
      : "Not connected — a shard that takes guests will still let you in.";
    account.className = "";
    account.removeAttribute("title");
    connect.textContent = guesting ? "Sign in with a wallet" : "Connect wallet";
    guest.hidden = true;
    chip.dataset.state = "away";
    chipText.textContent = "sign in";
  }
}

/* ── the silent resume ─────────────────────────────────────────────────────
   `eth_accounts` NEVER PROMPTS — it answers with what this site is already
   authorised for, or []. That is the whole of "auto-connect": somebody who
   connected a wallet to elopros.com once, anywhere on it, arrives here already
   signed in and presses one button. `eth_requestAccounts`, which does prompt,
   appears once below and only ever from a press.

   This is `Deck.purse.resume()`'s rule, and the reason it is a separate
   function there and here: a page load may never open a wallet dialog. */
async function resume() {
  if (!wallet.has()) { showAccount(); return; }
  if (signedOut()) { showAccount(); return; }
  try {
    address = (await wallet.accounts())[0] || null;
  } catch { address = null; }
  showAccount();
}

connect.addEventListener("click", async () => {
  connect.disabled = true;
  try {
    address = await wallet.connect();
    guesting = false;
    /* Asking to sign in is the unambiguous end of being signed out, so the
       flag goes — after the prompt, so a refusal leaves the reader where they
       were. Same order as the platform's own purse. */
    try { localStorage.removeItem(SIGNED_OUT); } catch {}
    if (status.className === "bad") say("");
  } catch (e) {
    // Not a shard refusal and not read as one: this is the extension talking
    // about its own prompt, and its message is the only useful thing to show.
    say(wallet.refusal(e), "bad");
  }
  connect.disabled = false;
  showAccount();
});

/* The other direction, and the page had no way to go it: connected, and you
   want in as a guest anyway. Session-only — it drops the address this page is
   holding and writes no flag, because signing out of the PLATFORM is the
   store's button and not this one's. */
guest.addEventListener("click", () => {
  address = null;
  guesting = true;
  showAccount();
});

// The wallet's own event: somebody switches account in the extension while
// this page is open. Without this the page keeps a stale address and the
// signature step fails with a provider error instead of a sentence. A
// provider event may not sign a signed-out reader back in — some wallets fire
// `accountsChanged` on unlock, on tab focus, and after a revoke.
if (wallet.has() && typeof window.ethereum.on === "function") {
  window.ethereum.on("accountsChanged", (a) => {
    address = signedOut() ? null : ((a && a[0]) || null);
    if (address) guesting = false;
    showAccount();
  });
}

/* Sign out in the store's masthead, in another tab, and this page follows.
   `storage` fires in every OTHER tab on the origin, which is exactly the set
   that needs to hear it. */
addEventListener("storage", (e) => {
  if (e.key !== SIGNED_OUT) return;
  if (signedOut()) { address = null; guesting = false; }
  else resume();
  showAccount();
});

showAccount();
resume();

/* ── which shards there are ────────────────────────────────────────────────
   The game publishes its own shard list and the origin hands it over
   byte-for-byte (`scry-forge meter/launcher.py::launcher_servers`, which
   holds no opinion about the rows and invents no count). The same document
   the elo launcher's Servers window reads, so a player sees the same shards
   in both places.

   Off the origin there is no such route, so the page does not ask: the
   Advanced field is already open there and the dev shard is already in it. */
const SHARD_LIST = "/api/launcher/servers/gates";

/* The live count, and the one honest constraint on it. nginx maps
   `/games/gates/status.json` onto the public shard's own loopback responder
   (`crates/server/src/status.rs`, `status_addr = 127.0.0.1:8431`; the
   location lives in scry-forge `deploy/nginx/elopros.com.conf`). That is a
   SAME-ORIGIN read, which is why the page can make it at all — each shard's
   own `status_url` is on `game.elopros.com` and the page's `connect-src`
   names no such origin.

   One shard is fronted this way, so one row can carry a count. A count that
   cannot be read is ABSENT, never zero: "nobody is playing" and "we could not
   look" are opposite facts and only one of them is a reason not to join. */
const STATUS_FOR = "game.elopros.com:61234";
const STATUS_URL = "status.json";

let shards = [];
let picked = serverField.value.trim();

function pick(addr) {
  picked = addr;
  serverField.value = addr;
  remember(addr);
  for (const b of shardBox.querySelectorAll(".shard"))
    b.setAttribute("aria-pressed", b.dataset.addr === addr ? "true" : "false");
}

function shardRow(s) {
  const b = document.createElement("button");
  b.type = "button";
  b.className = "shard";
  b.dataset.addr = s.addr;
  b.setAttribute("aria-pressed", s.addr === picked ? "true" : "false");

  const name = document.createElement("span");
  name.className = "shard-name";
  name.textContent = s.name || s.id || s.addr;

  const meta = document.createElement("span");
  meta.className = "shard-meta";
  meta.textContent = s.map ? String(s.map) : s.addr;

  const count = document.createElement("span");
  count.className = "shard-count";
  count.dataset.addr = s.addr;
  count.textContent = s.max_players ? `up to ${s.max_players}` : "";

  b.append(name, meta, count);
  b.addEventListener("click", () => pick(s.addr));
  return b;
}

function noShards(why) {
  const p = document.createElement("p");
  p.className = "note";
  p.textContent = why;
  shardBox.replaceChildren(p);
}

async function loadShards() {
  if (!onOrigin) {
    noShards("Off the origin — dial a shard by address below.");
    return;
  }
  let doc;
  try {
    const r = await fetch(SHARD_LIST, { cache: "no-cache" });
    if (!r.ok) throw new Error(String(r.status));
    doc = await r.json();
  } catch (e) {
    // The launcher route separates these two on purpose (503 "could not look"
    // vs 404 "publishes no list") and so does this sentence: neither of them
    // is "there are no shards".
    noShards("Could not read the shard list just now — the address below still works.");
    return;
  }
  shards = Array.isArray(doc && doc.servers) ? doc.servers.filter((s) => s && s.addr) : [];
  if (!shards.length) { noShards("This game publishes no shard list right now."); return; }

  /* A remembered or linked address that is not on the list is still the one to
     dial — the player typed it or followed it — so it is not overwritten. */
  const known = shards.some((s) => s.addr === picked);
  if (!known && !q.has("server") && !remembered()) picked = shards[0].addr;
  shardBox.replaceChildren(...shards.map(shardRow));
  pick(picked);
  readStatus();
}

async function readStatus() {
  const cell = shardBox.querySelector(`.shard-count[data-addr="${CSS.escape(STATUS_FOR)}"]`);
  if (!cell) return;
  try {
    const r = await fetch(STATUS_URL, { cache: "no-store" });
    if (!r.ok) throw new Error(String(r.status));
    const s = await r.json();
    const now = Number(s.players), cap = Number(s.max_players);
    if (!Number.isFinite(now) || !Number.isFinite(cap)) throw new Error("shape");
    cell.textContent = `${now} of ${cap} playing`;
    cell.dataset.live = now > 0 ? "1" : "0";
  } catch {
    /* Absent, not zero. Whatever the row already says (the published cap) is
       the honest thing to leave on screen. */
    delete cell.dataset.live;
  }
}

loadShards();
/* A hidden tab does not read — the platform's own poll rule, and the reason
   is the same here: a launcher left open overnight should not be asking the
   box for a player count every twenty seconds. */
setInterval(() => { if (!document.hidden) readStatus(); }, 20000);
document.addEventListener("visibilitychange", () => { if (!document.hidden) readStatus(); });

/* ── the module, and what it costs ─────────────────────────────────────────
   The client is tens of megabytes of wasm and the page used to say nothing at
   all while it landed. It says two things now: how far along it is, and that
   the download happens once.

   ⚠ **`content-length` is not the denominator**, and believing it is is how a
   progress bar reaches 400%. The origin serves `client_web_bg.wasm.gz` beside
   the module (`gzip_static`, `ci/build_web.sh` writes it), so the header is
   the COMPRESSED length while the stream below yields DECODED bytes — a ratio
   of about four. `build.json`, written by the same build, carries the decoded
   size and is right whatever re-encoded the body on the way here. Without it
   (an older publish, a checkout) the page falls back to `content-length` only
   when nothing set `content-encoding`, and otherwise counts megabytes with no
   bar rather than drawing a percentage it cannot compute. */
const WASM = "./client_web_bg.wasm";
const BUILD = "./build.json";

let loaded = false;
let armed = false;     /* Play was pressed before the module landed */
let audio = null;      /* the AudioContext promise, opened inside that press */

function progress(got, total) {
  if (total > 0) {
    const pct = Math.min(100, (got / total) * 100);
    bar.hidden = false;
    barfill.style.setProperty("--p", pct.toFixed(1) + "%");
    say(`loading the client — ${mb(got)} of ${mb(total)} MB`);
  } else {
    say(`loading the client — ${mb(got)} MB`);
  }
}

async function decodedSize() {
  try {
    const r = await fetch(BUILD, { cache: "no-cache" });
    if (!r.ok) return 0;
    const j = await r.json();
    return Number(j.wasm_bytes) || 0;
  } catch { return 0; }
}

async function loadModule() {
  let total = await decodedSize();
  try {
    const res = await fetch(WASM);
    if (!res.ok) throw new Error(`client_web_bg.wasm: ${res.status}`);
    if (!res.body) throw new Error("no readable body");
    if (!total && !res.headers.get("content-encoding"))
      total = Number(res.headers.get("content-length")) || 0;

    /* A Response the glue can still compile as a STREAM — `__wbg_load` takes
       the `instantiateStreaming` path for any Response that says
       `application/wasm`, so counting the bytes costs no buffering and no
       second copy of a 34 MB module in memory. */
    const reader = res.body.getReader();
    let got = 0;
    const counted = new ReadableStream({
      async pull(c) {
        const { done, value } = await reader.read();
        if (done) { c.close(); return; }
        got += value.byteLength;
        progress(got, total);
        c.enqueue(value);
      },
      cancel(reason) { return reader.cancel(reason); },
    });
    return await init({
      module_or_path: new Response(counted, { headers: { "Content-Type": "application/wasm" } }),
    });
  } catch (e) {
    /* Never fatal, and the fallback is the glue's own fetch of the same URL —
       which the browser will usually answer out of the cache the attempt above
       just filled. A page that loads without a progress bar beats a page that
       does not load. */
    console.warn("gates: counted load failed, falling back to the glue's own fetch -", e);
    bar.hidden = true;
    say("loading the client…");
    return await init();
  }
}

function ready() {
  loaded = true;
  bar.hidden = true;
  if (blocked) return;
  if (armed) { armed = false; join(); return; }
  go.disabled = false;
  go.removeAttribute("data-armed");
  go.textContent = "Play";
  say(leftWhy
    ? leftWhy + " — press Play to go back in."
    : "Ready — the client is cached now, so coming back will not download it again.");
}

function readyFailed(e) {
  bar.hidden = true;
  go.disabled = true;
  go.textContent = "Could not load";
  say((e && e.message) || String(e), "bad");
}

/* Started here and NOT awaited — see the warning at the top of this file.

   A browser with no WebTransport never gets to press Play, so it never gets
   the download either: thirty-odd megabytes for a page that has already said
   it cannot connect is a bill with nothing on the other side of it, and the
   progress line would paint straight over the sentence explaining why. */
const moduleReady = blocked ? Promise.resolve() : loadModule().then(
  (exports) => {
    // The module's exports, kept on the window for a person with the console
    // open: `gatesWasm.memory.buffer.byteLength` is the wasm heap, which is
    // the number to read when a tab dies with no message (an out-of-memory
    // abort in wasm is a bare `RuntimeError: unreachable`).
    window.gatesWasm = exports;
    ready();
  },
  (e) => readyFailed(e),
);

/* ⚠ **PLAY IS LIVE WHILE THE MODULE IS STILL LANDING, and that is the whole
   point of the queue below — a DISABLED BUTTON DISPATCHES NO CLICK AT ALL.**
   The first cut of this greyed the button out during the download and armed
   the press from its click handler, which is a handler the browser never
   calls: `disabled` is not a style, it takes the element out of the event
   path entirely. So the press would have been dropped exactly as silently as
   the bug it was written to fix. The button stays pressable, the bar and the
   status line say what is happening, and the press is kept. Disabled here is
   reserved for the two states where pressing really can do nothing: no
   WebTransport, and a module that failed to load.

   The markup ships it disabled, which is the honest state for a visitor whose
   scripts never ran. This is the line that hands it over. */
if (!blocked) {
  go.disabled = false;
  go.textContent = "Play";
  say(leftWhy ? leftWhy + " — the client is loading." : "loading the client…");
}

/* ── the audio thread, opened inside the gesture ────────────────────────────
   The game's sound is rendered in Rust inside an `AudioWorkletProcessor`
   (`crates/sound/src/worklet.rs`; the page-side seam is
   `client/src/render/audio_web.rs`). This function builds it and leaves the
   result on `globalThis.gatesAudio`, where the renderer looks for it.

   ⚠ **`new AudioContext()` must happen inside a user gesture**, and that is
   why this is called at the TOP of the PLAY handler rather than beside the
   renderer: a context created without one starts `suspended`, and a suspended
   context is silence with no error raised anywhere — the same shape as the
   two audio bugs this page has already shipped (the CSP `eval` above, and the
   worklet-less flush that followed it). Everything up to the first `await`
   runs synchronously in the click, so the context is `running` before the
   join is even dialled; the async half — `addModule`, compiling the module —
   finishes while the shard handshake is in flight.

   ⚠ This is also why a press that arrives BEFORE the module has landed calls
   this straight away and then waits: the context has to be born in the click
   that the player actually made, not in the continuation that runs twenty
   seconds later with no gesture anywhere near it.

   A browser that refuses any of it is left with no `gatesAudio`, which
   `audio_web::open` reports once as `audio output: none` and then drops every
   command and counts it. A tab with no sound still plays. */
async function startAudio() {
  const ctx = new AudioContext({ latencyHint: "interactive" });
  try {
    /* `addModule` loads the CONCATENATION of `audio-prelude.js`, the
       sound-worklet glue and `audio-processor.js` (see `ci/build_web.sh`) —
       one script, no imports, because a worklet scope has no module graph
       worth relying on. */
    await ctx.audioWorklet.addModule("./audio.js");
    /* Fetched HERE, because a worklet scope has no `fetch`, and posted down
       the port as BYTES, transferred, for the worklet to compile itself.
       ⚠ **Not as a compiled `WebAssembly.Module`**, which is what this did
       first and it never arrived: Chromium 151 does not deserialize a Module
       in the AudioWorklet scope, so the processor got `messageerror`, never
       ran `init`, and the page was silent (measured 2026-09-17). The module is
       ~240 KB, well inside any synchronous-compile limit, and the page's
       `'wasm-unsafe-eval'` reaches the worklet, which inherits the page's
       CSP. */
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
    /* A belt-and-braces resume: some browsers suspend a context when the tab
       is hidden and do not resume it on return, and a click inside the canvas
       is a gesture we already get for pointer lock. */
    const wake = () => { if (ctx.state === "suspended") ctx.resume().catch(() => {}); };
    addEventListener("pointerdown", wake, { passive: true });
    addEventListener("keydown", wake, { passive: true });
    console.info(`gates audio: AudioWorklet at ${ctx.sampleRate} Hz`);
  } catch (e) {
    /* Never fatal. The game is playable silent, and `audio_web::open` says so
       once on the console rather than the page. */
    console.warn("gates audio: none -", (e && e.message) || e);
    try { await ctx.close(); } catch {}
    delete globalThis.gatesAudio;
  }
}

go.addEventListener("click", () => {
  if (blocked || go.disabled) return;
  // Synchronously inside the click — see `startAudio`. Opened once, whether
  // the join happens now or as soon as the module lands.
  if (!audio) audio = startAudio();
  if (!loaded) {
    /* The press is KEPT rather than dropped, and the button says so — a
       control that swallows a press and looks unchanged is the defect this
       whole path replaced. It goes inert only now, because now there is
       nothing a second press could add. */
    armed = true;
    go.disabled = true;
    go.dataset.armed = "1";
    go.textContent = "Starting when it lands…";
    return;
  }
  join();
});

async function join() {
  go.disabled = true;
  go.removeAttribute("data-armed");
  go.textContent = "Connecting…";
  const server = serverField.value.trim();
  const hash = hashField.value.trim() || undefined;
  if (server) remember(server);
  const named = (shards.find((s) => s.addr === server) || {}).name || server;
  say(address ? `connecting to ${named} as ${short(address)}…` : `connecting to ${named} as a guest…`);

  let g;
  try {
    // Address and signer together or neither — `Gates.join` refuses the odd
    // pair rather than dropping half of it, because an address nobody can
    // sign for reaches a locked shard as REFUSE_AUTH and blames the shard.
    g = await Gates.join(server, hash, address ?? undefined,
                         address ? ((text) => wallet.sign(text, address)) : undefined);
  } catch (e) {
    // The message is already right for a browser — `client::refusal_sentence`
    // overrides the shared wording wherever it names something a page cannot
    // do — so this displays it rather than deciding anything from it.
    //
    // The page used to read `String(e).includes("auth")` here, against a
    // sentence containing no "auth", and told a player in a tab to open the
    // elo launcher. `e.code` is the shard's own REFUSE_* number when a shard
    // answered at all; branch on that if this ever needs to branch.
    // `crates/client/tests/refusals.rs` fails if this file matches on text.
    say(e.message ?? String(e), "bad");
    go.disabled = false;
    go.textContent = "Play";
    return;
  }

  say(`in the world — player ${g.player_id}, seed ${g.seed} — starting the renderer…`, "good");

  // ── hand over to Bevy ────────────────────────────────────────────────────
  // The page's job ends here. It owned the frame loop while there was no
  // renderer — `g.pump(now - last)` out of a `requestAnimationFrame` — and
  // that loop is GONE rather than dormant: `pump` drains the client core's
  // own-fact rings destructively, `render/input.rs` calls it from a Bevy
  // system every frame, and two drivers would each see half the hits, toasts
  // and refusals. One drain, one owner — `CLAUDE.md` records that exact defect
  // merging cleanly and breaking the game's sound silently.
  //
  // Input goes with it: winit reads the canvas's own keyboard and pointer
  // events, so the WASD handling this page used to do is Bevy's too. The
  // pointer is captured on the first click inside the world, by the same
  // click-to-capture rule the desktop has (`ui::pointer`), and released on
  // Escape by the browser and the game together.
  //
  // `body.playing` is the whole handover on the page's side: one rule in the
  // stylesheet takes every other element out, so nothing can be left painting
  // under a fixed, stretched canvas.
  document.body.classList.add("playing");
  $("gates").hidden = false;
  // The worklet has to exist before `render::audio_web::open` looks for it:
  // that runs inside the plugin build, and a `gatesAudio` that lands one tick
  // later is a whole session with no sound and a line on the console saying
  // the page built no AudioWorklet. Never throws — `startAudio` swallows its
  // own failure, because a silent game still plays.
  await audio;
  // `play` CONSUMES the session and never returns — Bevy's wasm arm hands the
  // loop to requestAnimationFrame and `run()` does not come back.
  g.play("#gates");
}

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
      for (const e of list.getEntries()) { longTasks += 1; longestMs = Math.max(longestMs, e.duration); }
    }).observe({ type: "longtask", buffered: true });
    setInterval(() => {
      const heap = performance.memory ? ` · js heap ${(performance.memory.usedJSHeapSize / 1e6).toFixed(0)} MB` : "";
      console.info(`page: long tasks ${longTasks} (longest ${longestMs.toFixed(0)} ms)${heap}`);
      longTasks = 0; longestMs = 0;
    }, 5000);
  } catch {}
}

/* Kept so an unhandled rejection from the loader cannot land on the console as
   a bare promise — `readyFailed` has already said it on the page. */
moduleReady.catch(() => {});
