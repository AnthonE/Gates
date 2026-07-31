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
  makeActionSender,
} from "./net.js";
import { InputTracker } from "./input.js";
import { GameScene } from "./scene.js";
import { Terrain } from "./terrain.js";
import { Hud } from "./hud.js";

const WASM_URL = "/client_wasm.wasm";
const REFUSE_REASONS = ["protocol version mismatch", "shard is full"];
const MARK_TO_RAD = (Math.PI * 2) / 256;

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
  const { reply, reader, writer, leftover } = await handshake(
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
  run(ex, views, wt, seed, playerId, reader, writer, leftover);
}

const REFUSE_TEXT = [
  "no such recipe",
  "bad count",
  "needs a station",
  "queue full",
  "missing ingredients",
];
const STATION_TEXT = ["", "needs workbench", "needs furnace"];
// sim-core build.rs REFUSE_B_* order.
const BUILD_REFUSE_TEXT = [
  "no such piece",
  "spot taken",
  "needs support",
  "bad ground",
  "out of reach",
  "missing materials",
  "world is full",
];
// sim-core build.rs shape/material code order (UI labels, not content).
const SHAPE_TEXT = ["foundation", "wall", "doorway", "floor", "stairs", "roof"];
const MAT_TEXT = ["wood", "stone", "metal"];
const BUILD_CELL = 3;
const MAX_LEVEL = 7;

function run(ex, views, wt, seed, playerId, streamReader, streamWriter, streamLeftover) {
  const canvas = $("gl");
  const scene = new GameScene(canvas);
  const terrain = new Terrain(scene.scene, seed, ex, WASM_URL);
  const input = new InputTracker(canvas);
  const hud = new Hud();
  hud.show();

  const sender = makeSender(wt);
  const actions = makeActionSender(streamWriter);
  let closed = false;
  let craftDirty = true;
  // The queue countdown re-anchors on every authoritative announce.
  let craftEta = { ticks: 0, at: performance.now() };

  const sendCraft = (recipe, count) => {
    const len = ex.client_action_craft(recipe, count);
    views.refresh();
    if (len > 0) actions.send(views.output, len);
  };
  const sendCancel = (index) => {
    const len = ex.client_action_cancel(index);
    views.refresh();
    if (len > 0) actions.send(views.output, len);
  };
  const sendPlace = (row, cx, cz, level, loc) => {
    const len = ex.client_action_place(row, cx, cz, level, loc);
    views.refresh();
    if (len > 0) actions.send(views.output, len);
  };

  // Build mode (plain-UI stand-in for the radial at alpha): B toggles,
  // wheel cycles the piece row, R/F moves the working level, right-click
  // places at the aimed grid address. The server validates everything.
  const build = { on: false, row: 0, level: 0 };
  const pieceRecs = new Map(); // address key -> rec, for defs-arrival redraws
  const groundAt = (cx, cz) =>
    ex.terrain_height_at(seed, cx * BUILD_CELL + 1.5, cz * BUILD_CELL + 1.5);
  const drawPiece = (rec) => {
    const D = views.pieceDefs;
    scene.setPiece(
      rec.cx,
      rec.cz,
      rec.level,
      rec.loc,
      D[rec.row * 8],
      D[rec.row * 8 + 1],
      groundAt(rec.cx, rec.cz),
    );
  };
  // The aimed grid address for the selected piece: a point mid-reach
  // ahead of the feet picks the cell; wall shapes snap to the nearest
  // cell edge, canonicalized to west/north (sim-core build.rs). Fills
  // one reused object — this runs in the RAF loop while build mode is
  // on, and the RAF path allocates nothing (CLAUDE.md trap list).
  const bTarget = { cx: 0, cz: 0, level: 0, loc: 0, shape: 0 };
  const buildTarget = () => {
    const R = views.render;
    const shape = views.pieceDefs[build.row * 8];
    const ax = R[1] + Math.sin(input.yaw) * 3.5;
    const az = R[3] + Math.cos(input.yaw) * 3.5;
    let cx = Math.max(0, Math.min(1023, Math.floor(ax / BUILD_CELL)));
    let cz = Math.max(0, Math.min(1023, Math.floor(az / BUILD_CELL)));
    let loc = 0;
    if (shape === 1 || shape === 2) {
      const fx = ax / BUILD_CELL - cx;
      const fz = az / BUILD_CELL - cz;
      const m = Math.min(fx, 1 - fx, fz, 1 - fz);
      if (m === fx) loc = 2;
      else if (m === 1 - fx) (cx += 1), (loc = 2);
      else if (m === fz) loc = 3;
      else (cz += 1), (loc = 3);
    } else if (shape === 4) {
      loc = 1;
    }
    bTarget.cx = cx;
    bTarget.cz = cz;
    bTarget.level = shape === 0 ? 0 : build.level;
    bTarget.loc = loc;
    bTarget.shape = shape;
    return bTarget;
  };
  const buildStrip = () => {
    if (!build.on) {
      hud.setBuild("");
      return;
    }
    const D = views.pieceDefs;
    const total = (ex.client_piece_defs_state() >>> 0) >>> 16;
    if (total === 0) {
      hud.setBuild("build: waiting for piece table…");
      return;
    }
    const b = build.row * 8;
    const costs = [];
    for (let k = 0; k < D[b + 3]; k++) {
      costs.push(`${D[b + 4 + k * 2 + 1]} ${itemName(D[b + 4 + k * 2])}`);
    }
    hud.setBuild(
      `build: ${MAT_TEXT[D[b + 1]] || "?"} ${SHAPE_TEXT[D[b]] || "?"} · L${build.level} · ` +
        `${costs.join(" + ")} — wheel piece · R/F level · right-click place · B close`,
    );
  };

  // Rebuild the craft panel + queue strip from the wasm views. Runs on
  // the slow HUD timer and event flags only — never the RAF path.
  const invHave = (item) => {
    let n = 0;
    for (let s = 0; s < 30; s++) {
      if (views.inv[s * 2] === item && views.inv[s * 2 + 1] > 0) {
        n += views.inv[s * 2 + 1];
      }
    }
    return n;
  };
  const rebuildCraft = () => {
    const state = ex.client_recipes_state() >>> 0;
    const have = state & 0xffff;
    if (hud.craftOpen) {
      const rows = [];
      for (let r = 0; r < have; r++) {
        const b = r * 14;
        const R = views.recipes;
        const station = R[b + 4];
        const inputs = [];
        let craftable = true;
        for (let k = 0; k < R[b + 5]; k++) {
          const item = R[b + 6 + k * 2];
          const need = R[b + 6 + k * 2 + 1];
          const got = invHave(item);
          if (got < need) craftable = false;
          inputs.push({ text: `${itemName(item)} ${got}/${need}`, ok: got >= need });
        }
        rows.push({
          recipe: r,
          name: itemName(R[b]),
          count: R[b + 1],
          seconds: Math.round((R[b + 2] | (R[b + 3] << 16)) / 30),
          gated: station !== 0,
          gateText: STATION_TEXT[station] || "needs a station",
          craftable,
          inputs,
        });
      }
      // Hand-craftable first, gated last — the reference rail's read.
      rows.sort((a, b) => (a.gated === b.gated ? 0 : a.gated ? 1 : -1));
      hud.setCraft(rows, sendCraft);
    }
    const q = ex.client_craft_q() >>> 0;
    const count = q >>> 16;
    const jobs = [];
    for (let j = 0; j < count; j++) {
      const recipe = views.craftJobs[j * 2];
      const remaining = views.craftJobs[j * 2 + 1];
      const b = recipe * 14;
      const name = itemName(views.recipes[b]);
      let label = `${name} ×${remaining}`;
      if (j === 0) {
        const elapsed = (performance.now() - craftEta.at) / 1000;
        const left = Math.max(0, craftEta.ticks / 30 - elapsed);
        label += ` · ${left.toFixed(0)}s`;
      }
      jobs.push({ index: j, label });
    }
    hud.setCraftQueue(jobs, sendCancel);
  };

  document.addEventListener("keydown", (e) => {
    if (closed) return;
    if (e.code === "KeyC") {
      if (hud.toggleCraft()) {
        document.exitPointerLock();
        craftDirty = true;
      }
      e.preventDefault();
    } else if (e.code === "KeyB") {
      build.on = !build.on;
      if (!build.on) scene.hideGhost();
      buildStrip();
      e.preventDefault();
    } else if (build.on && (e.code === "KeyR" || e.code === "KeyF")) {
      const d = e.code === "KeyR" ? 1 : -1;
      build.level = Math.max(0, Math.min(MAX_LEVEL, build.level + d));
      buildStrip();
      e.preventDefault();
    }
  });
  document.addEventListener("wheel", (e) => {
    if (!build.on || closed) return;
    const total = (ex.client_piece_defs_state() >>> 0) >>> 16;
    if (total > 0) {
      const d = e.deltaY > 0 ? 1 : total - 1;
      build.row = (build.row + d) % total;
      buildStrip();
    }
  });
  document.addEventListener("contextmenu", (e) => e.preventDefault());
  document.addEventListener("mousedown", (e) => {
    if (build.on && input.locked && e.button === 2 && !closed) {
      const t = buildTarget();
      sendPlace(build.row, t.cx, t.cz, t.level, t.loc);
    }
  });
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
      if (flags & 128 /* CRAFT_DONE */) {
        for (;;) {
          const t = ex.client_craft_pop() >>> 0;
          if (t === 0xffffffff) break;
          const added = t & 0xffff;
          hud.toast(
            added > 0
              ? `crafted ${itemName(t >>> 16)} ×${added}`
              : `crafted ${itemName(t >>> 16)} — inventory full, lost`,
          );
        }
      }
      if (flags & 256 /* CRAFT_REFUSED */) {
        for (;;) {
          const r = ex.client_craft_refusal_pop() >>> 0;
          if (r === 0xffffffff) break;
          hud.toast(`can't craft: ${REFUSE_TEXT[r] || `code ${r}`}`);
        }
      }
      if (flags & 64 /* CRAFT_Q */) {
        craftEta = { ticks: ex.client_craft_q() & 0xffff, at: performance.now() };
      }
      if (flags & (1 | 64 | 512) /* INV | CRAFT_Q | RECIPES */) {
        craftDirty = true;
      }
      if (flags & 2048 /* PIECE_RESET */) {
        scene.clearPieces();
        pieceRecs.clear();
      }
      if (flags & (1024 | 2048) /* PIECES|PIECE_RESET */) {
        const n = ex.client_piece_changes_len();
        for (let i = 0; i < n; i++) {
          const key = views.pieceChanges[i * 2];
          const info = views.pieceChanges[i * 2 + 1];
          const rec = {
            cx: key >>> 16,
            cz: key & 0xffff,
            level: info >>> 16,
            loc: (info >>> 8) & 0xff,
            row: info & 0xff,
          };
          pieceRecs.set(key * 4096 + (info >>> 8), rec);
          drawPiece(rec);
        }
      }
      if (flags & 8192 /* PIECE_DEFS */) {
        // Def rows can land after pieces that reference them (a late
        // joiner's sync walk outruns the def drip): redraw everything —
        // the set is small and this fires at most a handful of times.
        for (const rec of pieceRecs.values()) drawPiece(rec);
        buildStrip();
      }
      if (flags & 4096 /* BUILD_REFUSED */) {
        for (;;) {
          const r = ex.client_build_refusal_pop() >>> 0;
          if (r === 0xffffffff) break;
          hud.toast(`can't build: ${BUILD_REFUSE_TEXT[r] || `code ${r}`}`);
        }
      }
      if (flags & 32 /* MARK */) {
        const cell = ex.client_weak_mark_cell() >>> 0;
        const entry = cell === 0xffffffff ? null : terrain.cellEntry(cell);
        if (!entry) {
          scene.hideWeakMark();
        } else {
          // Heading u8 over the shared yaw LUT: 0 faces +Z, rotates
          // toward +X. Offsets are cosmetics (DECISIONS.md §open).
          const a = (ex.client_weak_mark_info() & 0xff) * MARK_TO_RAD;
          const r = entry.arch === 1 ? 0.9 : 1.05;
          const lift = entry.arch === 1 ? 1.4 : 0.6;
          scene.setWeakMark(
            entry.x + Math.sin(a) * r,
            entry.y + lift,
            entry.z + Math.cos(a) * r,
          );
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
      input.sel,
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
      if (build.on) {
        // Scalar math into a reused object + one mesh transform.
        const t = buildTarget();
        scene.setGhost(t.shape, t.cx, t.cz, t.level, t.loc, groundAt(t.cx, t.cz));
      }
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
    hud.setSelected(input.sel);
    // The craft views rebuild on change, or every timer tick while the
    // panel or queue is visible (the ETA countdown text).
    const qCount = (ex.client_craft_q() >>> 0) >>> 16;
    if (craftDirty || hud.craftOpen || qCount > 0) {
      views.refresh();
      rebuildCraft();
      craftDirty = false;
    }
    if (build.on) buildStrip();
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
      recipes: (ex.client_recipes_state() >>> 0) & 0xffff,
      craftQ: qCount,
      pieceDefs: (ex.client_piece_defs_state() >>> 0) & 0xffff,
      pieces: scene.pieces.size,
    };
  }, 250);
}

if (!("WebTransport" in globalThis)) {
  errEl.textContent =
    "this browser has no WebTransport — need Chrome 97+, Edge 98+, Firefox 125+, or Safari 26.4+";
}
