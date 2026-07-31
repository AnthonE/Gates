// Boot: form → WebTransport → hello/welcome → wasm client core → RAF
// loop. The loop itself allocates nothing after warmup: input state and
// datagrams flow through fixed wasm buffers, render state reads back
// through cached typed-array views (DESIGN.md L8).

import { loadWasm, WasmViews } from "./wasm.js";
import {
  connect,
  handshake,
  pumpDatagrams,
  pumpStream,
  makeSender,
} from "./net.js";
import { InputTracker } from "./input.js";
import { GameScene } from "./scene.js";
import { Terrain } from "./terrain.js";
import { Hud } from "./hud.js";

const WASM_URL = "/client_wasm.wasm";
const REFUSE_REASONS = ["protocol version mismatch", "shard is full"];

const $ = (id) => document.getElementById(id);
const urlInput = $("url");
const certInput = $("cert");
const errEl = $("starterr");

urlInput.value = localStorage.getItem("gates.url") || "https://127.0.0.1:4433";
certInput.value = localStorage.getItem("gates.cert") || "";

$("connect").addEventListener("click", () => {
  errEl.textContent = "";
  boot($("url").value.trim(), $("cert").value.trim()).catch((e) => {
    errEl.textContent = String(e && e.message ? e.message : e);
  });
});

async function boot(url, certHex) {
  localStorage.setItem("gates.url", url);
  localStorage.setItem("gates.cert", certHex);

  const ex = await loadWasm(WASM_URL);
  const wt = await connect(url, certHex);

  // Handshake: wasm encodes/decodes; JS only frames bytes on the stream.
  const views = new WasmViews(ex);
  const helloLen = ex.client_hello();
  const { reply, reader, leftover } = await handshake(
    wt,
    views.output.slice(0, helloLen),
  );
  views.refresh();
  views.input.set(reply);
  const kind = ex.client_parse_handshake(reply.length);
  const hs = new Uint32Array(ex.memory.buffer, ex.client_hs_ptr(), 6);
  if (kind === 2) {
    throw new Error(
      `refused: ${REFUSE_REASONS[hs[5]] || `code ${hs[5]}`}`,
    );
  }
  if (kind !== 1) throw new Error("unrecognized handshake reply");
  const playerId = hs[1];
  const seed = BigInt(hs[2]) | (BigInt(hs[3]) << 32n);
  const serverTick = hs[4];

  ex.client_new(seed, playerId, serverTick);
  views.refresh();

  $("start").style.display = "none";
  run(ex, views, wt, seed, playerId, reader, leftover);
}

function run(ex, views, wt, seed, playerId, streamReader, streamLeftover) {
  const canvas = $("gl");
  const scene = new GameScene(canvas);
  const terrain = new Terrain(scene.scene, seed, ex, WASM_URL);
  const input = new InputTracker(canvas);
  const hud = new Hud();
  hud.show();

  const sender = makeSender(wt);
  let closed = false;
  const onClosed = () => {
    if (closed) return;
    closed = true;
    $("start").style.display = "flex";
    errEl.textContent = "connection closed — reconnect to rejoin";
  };
  pumpDatagrams(
    wt,
    (bytes) => {
      if (bytes.length > views.inCap) return;
      views.refresh();
      views.input.set(bytes);
      ex.client_on_datagram(bytes.length);
    },
    onClosed,
  );

  // Item names fill in as catalog batches arrive; unnamed indices fall
  // back to the number (never cached, so the name wins once it lands).
  const nameCache = new Map();
  const textDecoder = new TextDecoder();
  const itemName = (idx) => {
    const cached = nameCache.get(idx);
    if (cached) return cached;
    const len = views.catalog[idx * 25];
    if (!len) return `#${idx}`;
    const name = textDecoder.decode(
      views.catalog.subarray(idx * 25 + 1, idx * 25 + 1 + len),
    );
    nameCache.set(idx, name);
    return name;
  };

  // The reliable event lane: gather payouts, inventory, node vanish /
  // respawn, join sync, catalog (protocol::event). Low-rate, outside the
  // RAF path; flags mirror client-wasm core::APPLIED_*.
  pumpStream(
    streamReader,
    streamLeftover,
    (bytes) => {
      if (bytes.length > views.inCap) return;
      views.refresh();
      views.input.set(bytes);
      const flags = ex.client_on_stream(bytes.length);
      if (flags & 0x80000000) {
        // Our own server sent bytes we can't decode — the smoke gate
        // fails on console.error, which is exactly right.
        console.error("event lane: message failed to decode");
        return;
      }
      views.refresh();
      if (flags & 4 /* RESET */) terrain.resetHarvested();
      if (flags & (2 | 4) /* SLOTS|RESET */) {
        const n = ex.client_slot_changes_len();
        for (let i = 0; i < n; i++) {
          terrain.setCellHarvested(
            views.slotChanges[i * 2],
            views.slotChanges[i * 2 + 1] === 1,
          );
        }
      }
      if (flags & 8 /* TOAST */) {
        for (;;) {
          // wasm i32 returns read signed in JS; >>> 0 recovers the u32.
          const t = ex.client_toast_pop() >>> 0;
          if (t === 0xffffffff) break;
          hud.toast(`+${t & 0xffff} ${itemName(t >>> 16)}`);
        }
      }
    },
    onClosed,
  );
  wt.closed.then(onClosed, onClosed);

  let last = performance.now();
  let stamp = 0;

  function frame(now) {
    if (closed) return;
    requestAnimationFrame(frame);
    const dt = now - last;
    last = now;

    ex.client_set_input(
      input.buttons(),
      input.yawU16(),
      input.pitchU8(),
      input.moveX(),
      input.moveZ(),
    );
    ex.client_advance(dt);
    const dgLen = ex.client_poll_input();
    views.refresh();
    if (dgLen > 0) sender.send(views.output, dgLen);

    const nRemotes = ex.client_render();
    views.refresh();
    const R = views.render;
    stamp++;
    if (R[0] === 1) {
      scene.setCamera(R[1], R[2], R[3], input.yaw, input.pitch);
      terrain.update(R[1], R[3]);
    }
    for (let k = 0; k < nRemotes; k++) {
      const b = 14 + k * 8;
      scene.upsertRemote(
        views.remoteIds[k],
        R[b],
        R[b + 1],
        R[b + 2],
        R[b + 3],
        R[b + 5] === 1,
        stamp,
      );
    }
    scene.sweepRemotes(stamp);
    scene.render();
  }
  requestAnimationFrame(frame);

  // HUD on its own slow timer, never in the render path. The debug
  // snapshot below rides the same timer for the same reason: it exists so
  // ci/browser_smoke.mjs can assert remotes actually MOVE (a count can't
  // tell a frozen remote from a live one), and page.evaluate reading a
  // window global costs the RAF loop nothing.
  setInterval(() => {
    if (closed) return;
    const R = views.render;
    hud.set(
      `gates m1 · player ${playerId}\n` +
        `tick ~${R[9].toFixed(0)} (client ${R[10].toFixed(0)})\n` +
        `snapshots ${R[8].toFixed(0)} · remotes ${R[13].toFixed(0)}\n` +
        `mispredict ${R[7].toFixed(0)} · resync ${R[11].toFixed(0)} · err ${R[6].toFixed(2)}m\n` +
        `dg sent ${sender.stats.sent} · oversize ${sender.stats.oversize}` +
        (R[0] === 1 ? "" : "\nwaiting for first snapshot…"),
    );
    const hotbar = [];
    for (let s = 0; s < 6; s++) {
      const count = views.inv[s * 2 + 1];
      hotbar.push(count > 0 ? `${itemName(views.inv[s * 2])} ×${count}` : "");
    }
    hud.setHotbar(hotbar);
    const remotes = [];
    const n = R[13] | 0;
    for (let k = 0; k < n; k++) {
      const b = 14 + k * 8;
      remotes.push([views.remoteIds[k], R[b], R[b + 1], R[b + 2]]);
    }
    globalThis.__gatesDebug = {
      playerId,
      inWorld: R[0] === 1,
      own: [R[1], R[2], R[3]],
      snapshots: R[8],
      remotes,
      oversize: sender.stats.oversize,
      hotbar,
    };
  }, 250);
}

if (!("WebTransport" in globalThis)) {
  errEl.textContent =
    "this browser has no WebTransport — need Chrome 97+, Edge 98+, Firefox 125+, or Safari 26.4+";
}
