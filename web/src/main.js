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
  // [kind, player_id, seed_lo, seed_hi, tick, refuse_code, dev]
  const hs = new Uint32Array(ex.memory.buffer, ex.client_hs_ptr(), 7);
  if (kind === 2) {
    throw new Error(
      `refused: ${REFUSE_REASONS[hs[5]] || `code ${hs[5]}`}`,
    );
  }
  if (kind !== 1) throw new Error("unrecognized handshake reply");
  const playerId = hs[1];
  const seed = BigInt(hs[2]) | (BigInt(hs[3]) << 32n);
  const serverTick = hs[4];
  // The shard states whether it is a dev shard (protocol Welcome.dev, set
  // by shard.toml's dev_spawn). Nothing else gates the dev affordances
  // below, so a public shard's page never grows them.
  const dev = hs[6] === 1;

  ex.client_new(seed, playerId, serverTick);
  views.refresh();

  $("start").style.display = "none";
  run(ex, views, wt, seed, playerId, reader, writer, leftover, dev);
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
  "claimed by a hearth",
  "nothing to upgrade into",
];
// sim-core deploy.rs REFUSE_D_* order.
const DEPLOY_REFUSE_TEXT = [
  "no such deployable",
  "spot taken",
  "needs support",
  "bad ground",
  "out of reach",
  "item not in inventory",
  "world is full",
  "claimed by a hearth",
  "too close to a hearth",
  "bag limit reached",
  "no hearth there",
  "no door there",
  "not your door",
];
// sim-core build.rs shape/material code order (UI labels, not content).
const SHAPE_TEXT = ["foundation", "wall", "doorway", "floor", "stairs", "roof"];
const MAT_TEXT = ["wood", "stone", "metal"];
const BUILD_CELL = 3;
const MAX_LEVEL = 7;

function run(ex, views, wt, seed, playerId, streamReader, streamWriter, streamLeftover, dev) {
  const canvas = $("gl");
  const scene = new GameScene(canvas);
  // The clipmap rides along so a chunk streaming in or out can force the
  // cached coarse shadow levels to redraw (shadow clipmap v0).
  const terrain = new Terrain(scene.scene, seed, ex, WASM_URL, scene.clipmap);
  // The ground's material lives with the terrain that feeds it; the scene
  // borrows its uniforms so the surface probe has one handle (materials v0).
  scene.attachTerrainMaterial(terrain.material);
  // …and the far mesh's depth uniforms, so the horizon probe has one handle
  // on the horizon's caster (the horizon casts).
  scene.attachFarCaster(terrain.farDepth.userData.uniforms);
  // Compile every color program the session can wear, now, while the player
  // is still watching the connect screen — the depth half finishes over the
  // first in-world frames (see the RAF path and scene.prewarm's comment).
  // Terrain hands in the families a plain dummy cannot reach: the instanced
  // scatter pools and the far caster's custom depth.
  scene.prewarm(terrain.prewarmObjects());
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
  const sendDeploy = (row, cx, cz, level, loc) => {
    const len = ex.client_action_deploy(row, cx, cz, level, loc);
    views.refresh();
    if (len > 0) actions.send(views.output, len);
  };
  // Chat: JS writes the UTF-8 into the wasm in-buffer and calls straight
  // through, so nothing else can be mid-flight in it. A 0 length back is
  // the client declining to send (empty, over-long, or a control
  // character) — the server never sees it.
  const chatEncoder = new TextEncoder();
  const sendChat = (text, global) => {
    views.refresh();
    const bytes = chatEncoder.encode(text);
    if (bytes.length === 0 || bytes.length > views.inCap) return false;
    views.input.set(bytes);
    const len = ex.client_action_chat(bytes.length, global ? 1 : 0);
    views.refresh();
    if (len === 0) return false;
    return actions.send(views.output, len);
  };
  const sendFeed = (cx, cz, level) => {
    const len = ex.client_action_feed(cx, cz, level);
    views.refresh();
    if (len > 0) actions.send(views.output, len);
  };
  // Your own door swings on the press, not on the reply (NETCODE.md
  // §6.1) — the wasm mirror flips the record and the predictor's shut
  // bit, and this redraws it. The server's announcement is absolute, so
  // it confirms or corrects; a refusal rolls the mirror back and rides
  // out as a deploy change like any other, so the leaf swings back
  // through the same redraw path below.
  const sendUse = (cx, cz, level, loc) => {
    const len = ex.client_action_use(cx, cz, level, loc);
    views.refresh();
    if (len > 0) actions.send(views.output, len);
    const open = ex.client_predict_door(cx, cz, level, loc) >>> 0;
    if (open !== 0xffffffff) {
      const rec = deployRecs.get(((cx << 16) | cz) * 4096 + ((level << 8) | loc));
      if (rec) {
        rec.open = open === 1;
        drawDeploy(rec);
      }
    }
  };

  // Build mode (plain-UI stand-in for the radial at alpha): B toggles,
  // wheel cycles the piece row — and past the piece table, the deployable
  // rows — R/F moves the working level, right-click places at the aimed
  // grid address. The server validates everything.
  const build = { on: false, row: 0, level: 0 };
  const pieceRecs = new Map(); // address key -> rec, for defs-arrival redraws
  const deployRecs = new Map(); // address key -> rec, same idea
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
  const drawDeploy = (rec) => {
    scene.setDeploy(
      rec.cx,
      rec.cz,
      rec.level,
      rec.loc,
      views.deployDefs[rec.row * 4],
      groundAt(rec.cx, rec.cz),
      rec.open,
      rec.locked,
    );
  };
  const pieceTotal = () => (ex.client_piece_defs_state() >>> 0) >>> 16;
  const deployTotal = () => (ex.client_deploy_defs_state() >>> 0) >>> 16;
  // The selected build row: a piece row, or past the piece table a
  // deployable row (doors snap to edges like walls; the rest sit in the
  // cell body). One reused object — this runs in the RAF loop while
  // build mode is on (CLAUDE.md trap list: no per-frame allocations).
  const sel = { deploy: false, row: 0, shape: 0 };
  const selRow = () => {
    const pt = pieceTotal();
    if (build.row < pt) {
      sel.deploy = false;
      sel.row = build.row;
      sel.shape = views.pieceDefs[build.row * 8];
    } else {
      const dr = build.row - pt;
      sel.deploy = true;
      sel.row = dr;
      sel.shape = views.deployDefs[dr * 4 + 1] === 2 ? 2 : 0;
    }
    return sel;
  };
  // The aimed grid address for the selected piece: a point mid-reach
  // ahead of the feet picks the cell; wall shapes snap to the nearest
  // cell edge, canonicalized to west/north (sim-core build.rs). Fills
  // one reused object — this runs in the RAF loop while build mode is
  // on, and the RAF path allocates nothing (CLAUDE.md trap list).
  const bTarget = { cx: 0, cz: 0, level: 0, loc: 0, shape: 0, deploy: false, row: 0 };
  const buildTarget = () => {
    const R = views.render;
    const sel = selRow();
    const shape = sel.shape;
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
    bTarget.level = !sel.deploy && shape === 0 ? 0 : build.level;
    bTarget.loc = loc;
    bTarget.shape = shape;
    bTarget.deploy = sel.deploy;
    bTarget.row = sel.row;
    return bTarget;
  };
  const buildStrip = () => {
    if (!build.on) {
      hud.setBuild("");
      return;
    }
    const pt = pieceTotal();
    if (pt === 0) {
      hud.setBuild("build: waiting for piece table…");
      return;
    }
    const sel = selRow();
    let what;
    let costs;
    if (sel.deploy) {
      const b = sel.row * 4;
      what = itemName(views.deployDefs[b + 3]);
      costs = `1 ${what}`;
    } else {
      const D = views.pieceDefs;
      const b = sel.row * 8;
      what = `${MAT_TEXT[D[b + 1]] || "?"} ${SHAPE_TEXT[D[b]] || "?"}`;
      const parts = [];
      for (let k = 0; k < D[b + 3]; k++) {
        parts.push(`${D[b + 4 + k * 2 + 1]} ${itemName(D[b + 4 + k * 2])}`);
      }
      costs = parts.join(" + ");
    }
    hud.setBuild(
      `build: ${what} · L${build.level} · ${costs}` +
        ` — wheel piece · R/F level · right-click place · E use door / feed hearth` +
        ` · L lock door · U upgrade · B close`,
    );
  };
  // Feed the nearest hearth within reach of the feet (the authoritative
  // reach check is the server's 5 m — build grid v0, DECISIONS.md §open;
  // this only picks the target address inside that same radius).
  const REACH = 5;
  const tryFeed = () => {
    const R = views.render;
    let best = null;
    let bestD = REACH * REACH;
    for (const rec of deployRecs.values()) {
      if (views.deployDefs[rec.row * 4] !== 1) continue; // not a hearth
      const dx = rec.cx * BUILD_CELL + 1.5 - R[1];
      const dz = rec.cz * BUILD_CELL + 1.5 - R[3];
      const d2 = dx * dx + dz * dz;
      if (d2 < bestD) {
        bestD = d2;
        best = rec;
      }
    }
    if (best) sendFeed(best.cx, best.cz, best.level);
    else hud.toast("no hearth in reach");
  };
  // The nearest door within reach of the feet, or null. Reach is measured
  // to the door's **cell center**, the same distance the sim gates on —
  // picking a target the server would refuse only costs a rolled-back
  // prediction and a bounce.
  const nearestDoor = () => {
    const R = views.render;
    let best = null;
    let bestD = REACH * REACH;
    for (const rec of deployRecs.values()) {
      if (views.deployDefs[rec.row * 4] !== 6) continue; // not a door
      const dx = rec.cx * BUILD_CELL + BUILD_CELL / 2 - R[1];
      const dz = rec.cz * BUILD_CELL + BUILD_CELL / 2 - R[3];
      const d2 = dx * dx + dz * dz;
      if (d2 < bestD) {
        bestD = d2;
        best = rec;
      }
    }
    return best;
  };
  // E is the use key: the nearest door within reach toggles, and only
  // when there is none does E fall through to feeding a hearth.
  // Loot the nearest death backpack in reach. The action carries no
  // target — the sim picks, inside the same reach a feed and a door use
  // (backpack.rs) — so this checks reach only to give an honest local
  // answer when there is nothing there, and never to aim.
  const tryLoot = () => {
    const R = views.render;
    const n = ex.client_bags_len();
    for (let i = 0; i < n; i++) {
      const dx = views.bagPos[i * 3] - R[1];
      const dz = views.bagPos[i * 3 + 2] - R[3];
      if (dx * dx + dz * dz > REACH * REACH) continue;
      const len = ex.client_action_loot();
      views.refresh();
      if (len > 0) actions.send(views.output, len);
      return true;
    }
    return false;
  };
  // E is the one interact key, and its order is deliberate: a door is
  // aimed at, a bag is stood on, and a hearth is the thing you meant if
  // neither is there.
  const tryUse = () => {
    const best = nearestDoor();
    if (best) sendUse(best.cx, best.cz, best.level, best.loc);
    else if (!tryLoot()) tryFeed();
  };
  // L locks or unlocks it. Whether the door is yours is the server's
  // verdict — the wire carries the lock bit but never the owner — so the
  // press goes out either way and a refusal comes back as a toast. No
  // prediction rides along: the announcement is absolute and this is not
  // an action anyone spams.
  const tryLock = () => {
    const best = nearestDoor();
    if (!best) {
      hud.toast("no door in reach");
      return;
    }
    const len = ex.client_action_lock(best.cx, best.cz, best.level, best.loc, best.locked ? 0 : 1);
    views.refresh();
    if (len > 0) actions.send(views.output, len);
  };
  // The nearest piece within reach of the feet, measured to the same
  // anchor the sim gates on: a cell center for planes and stairs, the
  // edge's midpoint for walls and doorways (sim-core build.rs `anchor`).
  const nearestPiece = () => {
    const R = views.render;
    let best = null;
    let bestD = REACH * REACH;
    for (const rec of pieceRecs.values()) {
      const x0 = rec.cx * BUILD_CELL;
      const z0 = rec.cz * BUILD_CELL;
      const h = BUILD_CELL / 2;
      const ax = rec.loc === 2 ? x0 : x0 + h;
      const az = rec.loc === 3 ? z0 : z0 + h;
      const dx = ax - R[1];
      const dz = az - R[3];
      const d2 = dx * dx + dz * dz;
      if (d2 < bestD) {
        bestD = d2;
        best = rec;
      }
    }
    return best;
  };
  // U climbs the nearest piece one rung: wood → stone → metal. The wire
  // carries the rung, not the step, so the client only has to know what
  // the piece is made of today — and whether that rung exists, and who
  // may pay for it, stays the server's verdict, back as a toast. Nothing
  // is predicted: an upgrade never moves collision.
  const tryUpgrade = () => {
    const best = nearestPiece();
    if (!best) {
      hud.toast("no building in reach");
      return;
    }
    // The def rows drip separately from the pieces that reference them,
    // so a piece can be on screen before its material is known — asking
    // then would send a rung picked out of an empty table.
    const pieceDefsHave = (ex.client_piece_defs_state() >>> 0) & 0xffff;
    if (best.row >= pieceDefsHave) {
      hud.toast("still loading that piece");
      return;
    }
    const material = views.pieceDefs[best.row * 8 + 1] + 1;
    if (material > 2) {
      hud.toast("already metal");
      return;
    }
    const len = ex.client_action_upgrade(best.cx, best.cz, best.level, best.loc, material);
    views.refresh();
    if (len > 0) actions.send(views.output, len);
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

  // The composer owns the keyboard while it is open: no walking, no
  // building, no swinging on a key that is meant to be a letter. Escape
  // always closes it, Enter sends. A line starting with `/g ` goes
  // global; local (20 m) is the default channel.
  hud.onChatSend = (raw) => {
    const global = raw.startsWith("/g ");
    if (sendChat(global ? raw.slice(3) : raw, global)) return true;
    // The cap is 48 UTF-8 BYTES, which the composer's maxlength (UTF-16
    // code units) only approximates — a line of multi-byte characters can
    // fit the box and still be refused. Say so rather than swallowing it.
    hud.toast("line not sent — 48 bytes max, no control characters");
    return false;
  };
  document.addEventListener("keydown", (e) => {
    if (closed) return;
    if (hud.chatOpen) return; // the composer's own handler has it
    if (e.code === "KeyT" || e.code === "Enter") {
      hud.openChat();
      document.exitPointerLock();
      e.preventDefault();
      return;
    }
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
    } else if (e.code === "KeyE") {
      tryUse();
      e.preventDefault();
    } else if (e.code === "KeyL") {
      tryLock();
      e.preventDefault();
    } else if (e.code === "KeyU") {
      tryUpgrade();
      e.preventDefault();
    }
  });
  document.addEventListener("wheel", (e) => {
    if (!build.on || closed) return;
    const total = pieceTotal() + deployTotal();
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
      if (t.deploy) sendDeploy(t.row, t.cx, t.cz, t.level, t.loc);
      else sendPlace(t.row, t.cx, t.cz, t.level, t.loc);
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
      if (flags & (1 << 23) /* HIT */) {
        for (;;) {
          const d = ex.client_hit_pop() >>> 0;
          if (d === 0xffffffff) break;
          hud.toast(`hit −${d}`);
        }
      }
      if (flags & (1 << 24) /* DEATH */) {
        for (;;) {
          const victim = ex.client_death_pop() >>> 0;
          if (victim === 0xffffffff) break;
          const killer = ex.client_death_killer() >>> 0;
          // The kill feed, in the chat log until a feed of its own
          // exists. Names don't exist yet, so ids stand in — the same
          // stand-in chat already uses.
          hud.chatLine(
            killer,
            true,
            victim === playerId ? `killed you` : `killed #${victim}`,
            victim === playerId,
          );
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
      if (flags & 32768 /* DEPLOY_RESET */) {
        scene.clearDeploys();
        deployRecs.clear();
      }
      if (flags & (16384 | 32768) /* DEPLOYS|DEPLOY_RESET */) {
        const n = ex.client_deploy_changes_len();
        for (let i = 0; i < n; i++) {
          const key = views.deployChanges[i * 2];
          const info = views.deployChanges[i * 2 + 1];
          const rec = {
            cx: key >>> 16,
            cz: key & 0xffff,
            level: (info >>> 16) & 0xff,
            loc: (info >>> 8) & 0xff,
            row: info & 0xff,
            open: ((info >>> 24) & 1) === 1,
            locked: ((info >>> 25) & 1) === 1,
          };
          deployRecs.set(key * 4096 + ((info >>> 8) & 0xffff), rec);
          drawDeploy(rec);
        }
      }
      if (flags & 131072 /* DEPLOY_DEFS */) {
        for (const rec of deployRecs.values()) drawDeploy(rec);
        buildStrip();
      }
      if (flags & 65536 /* DEPLOY_REFUSED */) {
        for (;;) {
          const r = ex.client_deploy_refusal_pop() >>> 0;
          if (r === 0xffffffff) break;
          hud.toast(`can't place: ${DEPLOY_REFUSE_TEXT[r] || `code ${r}`}`);
        }
      }
      if (flags & 2097152 /* CHAT */) {
        for (;;) {
          if ((ex.client_chat_pop() >>> 0) === 0) break;
          views.refresh();
          const c = views.chat;
          const from =
            (c[0] | (c[1] << 8) | (c[2] << 16) | (c[3] << 24)) >>> 0;
          const global = c[4] === 1;
          const text = textDecoder.decode(c.subarray(6, 6 + c[5]));
          hud.chatLine(from, global, text, from === playerId);
        }
      }
      if (flags & 262144 /* STOCK */) {
        const n = ex.client_stock_count();
        const parts = [];
        for (let i = 0; i < n; i++) {
          parts.push(`${views.stock[i * 2 + 1]} ${itemName(views.stock[i * 2])}`);
        }
        hud.toast(`hearth stock: ${parts.join(" · ")}`);
      }
      if (flags & (524288 | 1048576) /* PIECE_REMOVED | DEPLOY_REMOVED */) {
        const key = ex.client_removed_key() >>> 0;
        const info = ex.client_removed_info() >>> 0;
        const cx = key >>> 16;
        const cz = key & 0xffff;
        const level = info >>> 8;
        const loc = info & 0xff;
        if (flags & 524288) {
          scene.removePiece(cx, cz, level, loc);
          pieceRecs.delete(key * 4096 + ((level << 8) | loc));
        } else {
          scene.removeDeploy(cx, cz, level, loc);
          deployRecs.delete(key * 4096 + ((level << 8) | loc));
        }
      }
      if (flags & 33554432 /* BAGS */) {
        scene.setBags(views.bagIds, views.bagPos, ex.client_bags_len());
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
  // A smoothed frame time, for the debug snapshot only. It is a TIMED number
  // and therefore never a claim about reference hardware — it exists as a
  // same-box regression signal, because the client is a hot path too (L8) and
  // a change that halves the frame rate is otherwise invisible to every gate
  // here. One number, one multiply-add, no allocation.
  let frameMs = 0;
  // Its distribution over the last ~4 s, same status (reported, never
  // asserted): the smoothed number hides exactly the stall class the prewarm
  // trap is about — a 90 fps median can carry 700 ms worst-frames. The ring
  // is preallocated and written in the RAF path without allocation; the
  // percentiles are computed (and allocate) only in the 250 ms timer below.
  const frameRing = new Float32Array(240);
  const framePct = () => {
    const n = Math.min(stamp, frameRing.length);
    if (n < 30) return null;
    const a = Array.from(frameRing.subarray(0, n)).sort((x, y) => x - y);
    return {
      p50: a[(n * 0.5) | 0],
      p95: a[Math.min(n - 1, (n * 0.95) | 0)],
      p99: a[Math.min(n - 1, (n * 0.99) | 0)],
      worst: a[n - 1],
    };
  };
  // The prewarm gate's two numbers (see scene.prewarm): programs linked when
  // the snapshot below was taken, and -1 until it is. The first in-world
  // frames park the prewarm dummies at the player so the shadow pass links
  // the depth program, then the dummies go and the count is pinned — from
  // that frame on, a program link means a material prewarm missed.
  let programsAtInWorld = -1;
  let programKeysAtInWorld = null;
  let prewarmFrames = -1;
  let pinStamp = -1;
  // Every frame where the live program count CHANGED, as [stamp, count]
  // pairs — a link is the anomaly this whole rig hunts, so writing on one is
  // not a hot-path allocation. Capped; 32 links means something is broken.
  const programLog = [];
  let lastProgCount = -1;

  // The dev-only camera hook (DECISIONS.md §open "dev view hook"), bound
  // ONCE here — the 250 ms timer republishes this same function object
  // rather than minting a closure per tick, and the RAF path never sees it
  // at all. Null on a public shard, where `setView` is then not a property
  // that exists.
  const devSetView = dev ? (yaw, pitch) => input.setView(yaw, pitch) : null;
  // Same shape, same reason (bound once, republished): the shadow probe
  // renders 2N extra frames and reads the drawing buffer back, so it is a
  // dev affordance and never ships to a public shard.
  const devShadowProbe = dev
    ? (yaws, pitch, minDelta) => scene.shadowProbe(yaws, pitch, minDelta)
    : null;
  // Materials v0's two: the same 2N-frame probe shape for the surface, and
  // a census of the splat weights the shader is actually fed (~100 k vertex
  // reads — a gate hook, never a frame one).
  const devSurfaceProbe = dev
    ? (yaws, pitch, minDelta) => scene.surfaceProbe(yaws, pitch, minDelta)
    : null;
  const devSplatCensus = dev ? () => terrain.splatCensus() : null;
  // Shadow clipmap v0's probe: the same difference shape, but the thing it
  // holds fixed is the frame and the thing it removes is every level past the
  // near one — so what it counts is shadow that only exists past 80 m.
  const devFarShadowProbe = dev
    ? (yaws, pitch, minDelta, nearM, fov, heightM) =>
        scene.farShadowProbe(yaws, pitch, minDelta, nearM, fov, heightM)
    : null;
  // And this slice's: the same difference shape again, but what it removes is
  // the far mesh's whole contribution as a CASTER — so what it counts is
  // shadow that only exists because the horizon casts.
  const devHorizonProbe = dev
    ? (yaws, pitch, minDelta, heightM) =>
        scene.horizonProbe(yaws, pitch, minDelta, heightM)
    : null;
  // And the cost probe (NOW.md item 1): the only one that answers what a
  // term COSTS rather than whether it reaches the image. It compiles the
  // ground six ways, so the variants it needs — and the swapper that wears
  // them — are handed to the scene here and nowhere else. On a public shard
  // this line does not run, so the variant programs are never built at all.
  const devCostProbe = dev
    ? (yaw, pitch, scales, frames, reps) =>
        scene.costProbe(yaw, pitch, scales, frames, reps)
    : null;
  // Materials v1's: the contrast probe needs viewpoints in WORLD space, and
  // this is the scope that holds the camera. A view is the player's own
  // position, optionally lifted, aimed by (yaw, pitch) — the near one is
  // grain's home ground, the lifted one is where grain must already be gone.
  const devGrainProbe = dev
    ? (uniformName, specs, minDelta) => {
        const views = [];
        for (const s of specs) {
          const p = scene.camera.position;
          const y = p.y + (s.lift || 0);
          const cp = Math.cos(s.pitch);
          views.push({
            label: s.label,
            eye: [p.x, y, p.z],
            at: [
              p.x + Math.sin(s.yaw) * cp,
              y + Math.sin(s.pitch),
              p.z + Math.cos(s.yaw) * cp,
            ],
          });
        }
        return scene.contrastProbe(views, uniformName, minDelta);
      }
    : null;
  // Materials v1's third pass: the projection probe takes views in WORLD space
  // like the contrast one, but the caller aims them at a FACE rather than by
  // (yaw, pitch) — a combed grain is only combed on a slope, so the gate finds
  // one first and computes the eye from what it found.
  const devProjectionProbe = dev
    ? (views, minDelta) => scene.projectionProbe(views, minDelta)
    : null;
  // …and the face-finder itself. Terrain owns the chunks, so Terrain scans
  // them; the camera's own xz is the centre because the gate's other views are
  // taken from there too.
  const devSteepestFace = dev
    ? (radiusM, binM, minVerts, sun, minLit) => {
        const p = scene.camera.position;
        // The eye rides along because the probe's control views are taken from
        // it — the same vantage 15b's near and far views use, so the level and
        // retired frames are comparable to the ones already in the log.
        return {
          eye: [p.x, p.y, p.z],
          ...terrain.steepestFace(p.x, p.z, radiusM, binM, minVerts, sun, minLit),
        };
      }
    : null;
  if (dev) {
    scene.attachTerrainCost({
      variants: () => terrain.costVariants(),
      projection: () => terrain.projectionVariant(),
      use: (m) => terrain.useMaterial(m),
    });
  }

  function frame(now) {
    if (closed) return;
    requestAnimationFrame(frame);
    const dt = now - last;
    last = now;
    frameMs += (dt - frameMs) * 0.05;
    frameRing[stamp % frameRing.length] = dt;

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
      // Prewarm, depth half: the first in-world frame parks the casting
      // dummies at the player (the clipmap's first update draws every level,
      // so they are in that pass), two frames later they go and the program
      // count is pinned. Scalar checks on the steady path; the three calls
      // run a total of twice per session.
      if (programsAtInWorld < 0) {
        if (prewarmFrames < 0) {
          scene.prewarmAt(R[1], R[2] - 1.5, R[3]);
          prewarmFrames = 0;
        } else if (++prewarmFrames >= 3) {
          scene.prewarmDone();
          programsAtInWorld = scene.renderer.info.programs.length;
          programKeysAtInWorld = scene.renderer.info.programs.map(
            (p) => p.cacheKey,
          );
          pinStamp = stamp;
        }
      }
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
    if (scene.renderer.info.programs.length !== lastProgCount && programLog.length < 64) {
      lastProgCount = scene.renderer.info.programs.length;
      programLog.push([stamp, lastProgCount]);
    }
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
    const health = ex.client_health() >>> 0;
    hud.setVitals(health >>> 16, health & 0xffff);
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
    const debug = {
      playerId,
      dev,
      inWorld: R[0] === 1,
      own: [R[1], R[2], R[3]],
      // What the camera is actually built from this frame (the RAF loop
      // hands scene.setCamera these two), so an aim is checkable.
      view: [input.yaw, input.pitch],
      snapshots: R[8],
      remotes,
      oversize: sender.stats.oversize,
      hotbar,
      recipes: (ex.client_recipes_state() >>> 0) & 0xffff,
      craftQ: qCount,
      pieceDefs: (ex.client_piece_defs_state() >>> 0) & 0xffff,
      pieces: scene.pieces.size,
      deployDefs: (ex.client_deploy_defs_state() >>> 0) & 0xffff,
      deploys: scene.deploys.size,
      // Death backpacks: what the client has been told stands, and what
      // the renderer actually has meshes for. Two numbers, because a
      // renderer that quietly stopped reconciling would still report a
      // healthy set from the first.
      bagsKnown: ex.client_bags_len(),
      bags: scene.bags.size,
      // The lighting rig's structural facts plus last frame's draw counts
      // (DESIGN §9's < 300 calls / < 1.5 M tris). Read off the scene, not
      // recomputed — what the gate asserts is what the renderer did.
      lighting: scene.lighting(),
      frameMs,
      // The distribution behind that number, and the prewarm gate's pair —
      // all three the same class of fact: counts and times off the renderer,
      // reported so the gate (and whoever reads the log) sees them.
      framePct: framePct(),
      programs: scene.renderer.info.programs.length,
      programsAtInWorld,
      // The programs that linked after the pin, by cache key — names are
      // empty on this three build, but the key carries the shader id and
      // every define, which is what identifies the missed material.
      latePrograms: programKeysAtInWorld
        ? scene.renderer.info.programs
            .filter((p) => !programKeysAtInWorld.includes(p.cacheKey))
            .map((p) => p.cacheKey)
        : null,
      pinnedPrograms: programKeysAtInWorld,
      programLog,
      pinStamp,
      // The material system's structural facts (materials v0): the ground's
      // patched splat material, the authored per-surface responses, and how
      // the scatter pools are tinted.
      materials: { ...scene.materials(), scatter: terrain.scatterFacts() },
      // The horizon's caster: whether the far mesh casts, which side the
      // ground casts from, and the hole the near ring punches in it.
      farCaster: terrain.farCasterFacts(),
    };
    if (devSetView) debug.setView = devSetView;
    if (devShadowProbe) debug.shadowProbe = devShadowProbe;
    if (devSurfaceProbe) debug.surfaceProbe = devSurfaceProbe;
    if (devSplatCensus) debug.splatCensus = devSplatCensus;
    if (devFarShadowProbe) debug.farShadowProbe = devFarShadowProbe;
    if (devHorizonProbe) debug.horizonProbe = devHorizonProbe;
    if (devCostProbe) debug.costProbe = devCostProbe;
    if (devGrainProbe) debug.grainProbe = devGrainProbe;
    if (devProjectionProbe) debug.projectionProbe = devProjectionProbe;
    if (devSteepestFace) debug.steepestFace = devSteepestFace;
    globalThis.__gatesDebug = debug;
  }, 250);
}

if (!("WebTransport" in globalThis)) {
  errEl.textContent =
    "this browser has no WebTransport — need Chrome 97+, Edge 98+, Firefox 125+, or Safari 26.4+";
}
