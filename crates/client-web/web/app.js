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

const status = document.getElementById("status");
const go = document.getElementById("go");
const connect = document.getElementById("connect");
const account = document.getElementById("account");
const advanced = document.getElementById("advanced");
const say = (text, cls) => { status.textContent = text; status.className = cls ?? ""; };

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

/* ── watching instead of playing ───────────────────────────────────────────
   `?spectate` (any agent the shard lets you watch) or `?spectate=0x…` (that
   wallet's agent) takes a READ-ONLY seat (wire v73; NETCODE.md §2.3): the
   agent's own view, drawn by this tab's GPU from the ordinary snapshot
   stream. A watcher is anonymous — no wallet is asked for and nothing is
   signed — and the shard, not this page, decides who may be watched. */
const spectate = q.has("spectate") ? (q.get("spectate") || "any") : null;
document.getElementById("server").value =
  q.get("server") || (onOrigin ? "game.elopros.com:61234" : "127.0.0.1:4433");
document.getElementById("hash").value = q.get("hash") || "";
advanced.open = !onOrigin && !q.has("server");

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
try {
  const why = sessionStorage.getItem(LEFT);
  if (why) {
    sessionStorage.removeItem(LEFT);
    say(why + " — press Play to go back in.");
  }
} catch {}

// WebTransport is the only path a page has to a QUIC shard, and it is not
// everywhere yet. Say so before the button is pressed rather than after.
if (!("WebTransport" in window)) {
  say("This browser has no WebTransport. Gates speaks QUIC and a page has no other way to reach it.", "bad");
  go.disabled = true;
}

/* ── the wallet ────────────────────────────────────────────────────────────
   Deliberately the same three calls as the platform's own `Deck.wallet`
   (`watchtower/js/deck-shim.js` in AnthonE/scry-forge): has, connect, sign.
   A page served from elopros.com should load that file and delete this
   object — it is the one implementation of the extension's quirks, and the
   address check below is copied from it verbatim for the reason its own
   comment gives: somebody switches account in the extension, the page still
   holds the old address, and `personal_sign` then fails with a
   provider-internal error naming nothing anyone can act on.

   WHAT THIS DOES NOT DO, and must not: compose the message. The text comes
   out of Rust (`protocol::siwe_message`, through `Proof::message`) because
   the shard rebuilds the same bytes to verify them, and a second copy of an
   EIP-4361 message in JavaScript would drift into every login failing as
   `WrongSigner`. This function receives a finished string and hands it to
   the wallet, which renders it for the person about to approve it. */
const short = (a) => String(a).slice(0, 6) + "…" + String(a).slice(-4);
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
};

let address = null;   /* the connected wallet, or null for a guest join */

const showAccount = () => {
  account.textContent = address
    ? address + " — this shard will be asked to prove it"
    : "not connected — a shard that takes guests will still let you in";
  account.className = address ? "good" : "dim";
  connect.textContent = address ? "Change wallet" : "Connect wallet";
};

if (!wallet.has()) {
  connect.disabled = true;
  account.textContent = "no wallet extension here — you can still join a shard that takes guests";
}
if (spectate !== null) {
  // A watcher signs nothing, so the wallet row would only be a question the
  // page never asks. Hidden, and the button says what it will do.
  document.getElementById("who").hidden = true;
  go.textContent = "Watch";
  say(spectate === "any" ? "watch any agent on this shard" : `watch ${short(spectate)}`);
}

connect.addEventListener("click", async () => {
  connect.disabled = true;
  try {
    address = await wallet.connect();
    showAccount();
  } catch (e) {
    // Not a shard refusal and not read as one: this is the extension talking
    // about its own prompt, and its message is the only useful thing to show.
    say(e.message ?? String(e), "bad");
  }
  connect.disabled = false;
});

// The wallet's own event: somebody switches account in the extension while
// this page is open. Without this the page keeps a stale address and the
// signature step fails with a provider error instead of a sentence.
if (wallet.has() && typeof window.ethereum.on === "function") {
  window.ethereum.on("accountsChanged", (a) => {
    address = (a && a[0]) || null;
    showAccount();
  });
}
showAccount();

// The module's exports, kept on the window for a person with the console
// open: `gatesWasm.memory.buffer.byteLength` is the wasm heap, which is the
// number to read when a tab dies with no message (an out-of-memory abort in
// wasm is a bare `RuntimeError: unreachable`).
window.gatesWasm = await init();

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

go.addEventListener("click", async () => {
  go.disabled = true;
  // Synchronously inside the click — see `startAudio`. Awaited just before
  // the renderer takes over, so the processor exists before Bevy looks for it.
  const audio = startAudio();
  const server = document.getElementById("server").value.trim();
  const hash = document.getElementById("hash").value.trim() || undefined;
  say(spectate !== null
    ? `connecting to ${server} to watch…`
    : address ? `connecting to ${server} as ${short(address)}…` : `connecting to ${server} as a guest…`);

  let g;
  try {
    if (spectate !== null) {
      // A read-only seat: no address, no signer. The shard answers with whose
      // view this is, or refuses with its own code (6: cannot be watched here,
      // 7: too many watching).
      g = await Gates.watch(server, hash, spectate === "any" ? undefined : spectate);
    } else {
      // Address and signer together or neither — `Gates.join` refuses the odd
      // pair rather than dropping half of it, because an address nobody can
      // sign for reaches a locked shard as REFUSE_AUTH and blames the shard.
      g = await Gates.join(server, hash, address ?? undefined,
                           address ? ((text) => wallet.sign(text, address)) : undefined);
    }
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
    return;
  }

  say(g.watching
    ? `${g.watching} — starting the renderer…`
    : `in the world — player ${g.player_id}, seed ${g.seed} — starting the renderer…`, "good");

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
  document.body.classList.add("playing");
  document.getElementById("gates").hidden = false;
  document.querySelector("main").hidden = true;
  // The worklet has to exist before `render::audio_web::open` looks for it:
  // that runs inside the plugin build, and a `gatesAudio` that lands one tick
  // later is a whole session with no sound and a line on the console saying
  // the page built no AudioWorklet. Never throws — `startAudio` swallows its
  // own failure, because a silent game still plays.
  await audio;
  // `play` CONSUMES the session and never returns — Bevy's wasm arm hands the
  // loop to requestAnimationFrame and `run()` does not come back.
  g.play("#gates");
});

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
