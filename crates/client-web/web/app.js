import init, { Gates } from "./client_web.js";

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

go.addEventListener("click", async () => {
  go.disabled = true;
  const server = document.getElementById("server").value.trim();
  const hash = document.getElementById("hash").value.trim() || undefined;
  say(address ? `connecting to ${server} as ${short(address)}…` : `connecting to ${server} as a guest…`);

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
  document.body.classList.add("playing");
  document.getElementById("gates").hidden = false;
  document.querySelector("main").hidden = true;
  // `play` CONSUMES the session and never returns — Bevy's wasm arm hands the
  // loop to requestAnimationFrame and `run()` does not come back.
  g.play("#gates");
});
