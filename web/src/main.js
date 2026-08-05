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
import {
  APPLIED2_CONT,
  APPLIED2_MOVE,
  CONT_BAG,
  CONT_BOX,
  CONT_SELF,
  STREAM_HIGH_BIT,
  moveArgs,
  moveVerdict,
  slotsIn,
} from "./invmove.js";
import {
  INTERACT_REACH_M,
  REPAIR_STORE_DEPLOY,
  VERB_BAG,
  VERB_BOX,
  VERB_DOOR,
  VERB_HEARTH,
  VERB_NONE,
  centrePrompt,
  describeDeploy,
  describePiece,
  nearestPiece,
  nearestRepairable,
  newPick,
  newPiecePick,
  newRepairPick,
  newSwingPick,
  resolveInteract,
  resolveSwing,
  structNews,
} from "./interact.js";
import { MAP_N, WORLD_M, paintMap } from "./map.js";
import { buildRefusal, connectRefusal, craftRefusal, deployRefusal } from "./refusals.js";
import { loadGroundTextures, setGroundAnisotropy } from "./textures.js";

// Resolved against the document, not the origin root: the page is served
// from the site root in dev and from /games/gates/alpha/ in production, and
// an absolute "/client_wasm.wasm" 404s under a subpath. Absolute by the time
// the terrain worker receives it, which is what the worker needs.
const WASM_URL = new URL("client_wasm.wasm", document.baseURI).href;
const MARK_TO_RAD = (Math.PI * 2) / 256;

const $ = (id) => document.getElementById(id);
const urlInput = $("url");
const certInput = $("cert");
const errEl = $("starterr");

// Served from anywhere but a dev machine, the shard is the public one —
// a visitor should not have to know a port. Localhost keeps the dev default
// so `./web/dev.sh` and browser_smoke behave exactly as before.
const PUBLIC_SHARD = "https://game.moreright.xyz:61234";
const LOCAL = ["localhost", "127.0.0.1", "[::1]", ""].includes(location.hostname);
const DEFAULT_URL = LOCAL ? "https://127.0.0.1:4433" : PUBLIC_SHARD;
// A REMEMBERED url is only worth remembering while it still points at a
// shard that exists. The public shard moved ports once already (4466 ->
// 61234, see shard-public.toml) and every browser that had touched the old
// one kept trying it forever, because a stored value beats a changed
// default. So: on a public page, a stored url whose ORIGIN is not the
// current shard's is stale by construction — drop it. A deliberate override
// to some other host survives; only the outdated copy of our own default
// does not.
const stored = localStorage.getItem("gates.url");
let initial = stored || DEFAULT_URL;
if (!LOCAL && stored) {
  try {
    if (new URL(stored).origin !== new URL(PUBLIC_SHARD).origin
        && new URL(stored).hostname === new URL(PUBLIC_SHARD).hostname) {
      localStorage.removeItem("gates.url");
      initial = DEFAULT_URL;
    }
  } catch {
    localStorage.removeItem("gates.url");
    initial = DEFAULT_URL;
  }
}
urlInput.value = initial;
certInput.value = localStorage.getItem("gates.cert") || "";

$("connect").addEventListener("click", () => {
  errEl.textContent = "";
  boot($("url").value.trim(), $("cert").value.trim()).catch((e) => {
    errEl.textContent = String(e && e.message ? e.message : e);
  });
});

async function boot(url, certHex) {
  // Persist only a DELIBERATE override. Saving the default back means a
  // later change to the default can never reach a returning browser.
  if (url === DEFAULT_URL) localStorage.removeItem("gates.url");
  else localStorage.setItem("gates.url", url);
  localStorage.setItem("gates.cert", certHex);

  // The base maps ride alongside the wasm fetch rather than behind it: both
  // are boot-time payload with no dependency on each other, and both must be
  // in hand before `run()` builds a material. Awaited here and not later
  // because a texture that arrives after the first frame is a program relink,
  // and the prewarm gate counts links after `inWorld` (CLAUDE.md's trap list).
  const [ex] = await Promise.all([loadWasm(WASM_URL), loadGroundTextures()]);
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
      `refused: ${connectRefusal(hs[5])}`,
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

const STATION_TEXT = ["", "needs workbench", "needs furnace"];
// The craft, deploy and connect refusal tables moved to
// `web/src/refusals.js` beside the build one, for the reason that file's
// header gives: a bare array in here is one nothing can walk, and the build
// table fell behind the sim twice while it lived that way. Gated together by
// `ui_smoke` §W. Route a refusal through the accessor, never through a copy.
// The shape/material labels moved to `web/src/interact.js` beside
// `describePiece`, the one function that read them (gated: `ui_smoke` §V).
const BUILD_CELL = 3;
const MAX_LEVEL = 7;
// The three deployable archetypes E can act on used to be restated here and
// at four scan sites — `ARCH_BOX = 2` named, the hearth's `1` and the door's
// `6` bare. They now live in `web/src/interact.js` beside the resolver that
// reads them, gated against `deploy.rs` by `ci/ui_smoke.mjs` §Q.

function run(ex, views, wt, seed, playerId, streamReader, streamWriter, streamLeftover, dev) {
  const canvas = $("gl");
  const scene = new GameScene(canvas);
  // The clipmap rides along so a chunk streaming in or out can force the
  // cached coarse shadow levels to redraw (shadow clipmap v0).
  // Anisotropy is the renderer's to state, not this file's — and it has to be
  // on the texture before the first upload, which is why it lands between the
  // renderer existing and the ground being built. A 0.9 m tile at a grazing
  // angle is the exact case an isotropic mip chain blurs back into the wash
  // this slice is here to remove.
  setGroundAnisotropy(scene.renderer.capabilities.getMaxAnisotropy());
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
  // What the selected build row IS: its name, its full cost, and the shortfall
  // of the first ingredient the player cannot cover.
  //
  // One decode of the def tables, read by two rows of the HUD — the strip at
  // the bottom and the centre hint under the crosshair. It was the strip's
  // alone; a second copy for the prompt would have been a second reading of a
  // stride-8 table whose ingredient pairs live at `4 + k*2`, which is the
  // positional-payload shape `CLAUDE.md`'s trap list names as where the
  // reference ecosystem actually bled. `what` is `""` when the piece table has
  // not arrived, which is what both callers branch on.
  //
  // One reused object: this runs off the HUD timer, not the RAF path, but the
  // prompt asks for it four times a second and the client is a hot path too.
  const desc = { what: "", costs: "", need: "" };
  const selDesc = () => {
    desc.what = "";
    desc.costs = "";
    desc.need = "";
    if (pieceTotal() === 0) return desc;
    const sel = selRow();
    return sel.deploy
      ? describeDeploy(desc, views.deployDefs, sel.row, itemName, invHave)
      : describePiece(desc, views.pieceDefs, sel.row, itemName, invHave);
  };
  const buildStrip = () => {
    if (!build.on) {
      hud.setBuild("");
      return;
    }
    const d = selDesc();
    if (!d.what) {
      hud.setBuild("build: waiting for piece table…");
      return;
    }
    hud.setBuild(
      `build: ${d.what} · L${build.level} · ${d.costs}` +
        ` — wheel piece · R/F level · right-click place · E use door / feed hearth` +
        ` · L lock door · U upgrade · B close`,
    );
  };
  // ===========================================================================
  // E, the one interact key — one resolver, one prompt.
  // ===========================================================================
  // Until 2026-08-05 this was five independent scans tried in order (door,
  // bag, box, take-all, hearth), each with its own copy of the reach test and
  // each acting the moment it found anything. The judge's ranked gap 3
  // (`pass-20260805-002720-04`) named what that cost a player: stand between a
  // hearth and a box and E did something you did not choose, silently, and the
  // only feedback the chain could ever produce was the LAST link's toast.
  //
  // Now `interact.resolveInteract` makes the pick, `interact.promptFor` says
  // what it is, and the block below only DISPATCHES. Both the HUD timer's
  // prompt and this keypress call the resolver with the same arguments, so the
  // prompt cannot advertise a verb the key does not perform. The resolver is
  // pure and node-importable, so `ci/ui_smoke.mjs` §Q scores the pick itself in
  // milliseconds instead of in a browser.
  //
  // The reach is the SIM's (`build.rs`'s `BUILD_REACH_M`, aliased for bags as
  // `backpack.rs`'s `LOOT_REACH_M`), imported rather than restated — picking a
  // target the server would refuse only buys a round trip and a bounce.
  const pick = newPick();
  const lockPick = newPick();
  // One world adapter, reused. This runs four times a second off the HUD timer
  // and CLAUDE.md's client law is no per-frame allocations; the fields that
  // change per call are assigned, the object is not rebuilt.
  const interactWorld = {
    cell: BUILD_CELL,
    defs: null,
    recs: deployRecs.values(),
    bagCount: 0,
    bagPos: null,
    bagIds: null,
  };
  const interactAim = { x: 0, z: 0, fx: 0, fz: 0, reach: INTERACT_REACH_M, only: VERB_NONE };
  /**
   * Resolve what E would act on right now. `only` restricts the pick to a
   * single verb — L's lock uses it to find the door under the aim by the
   * SAME metric E uses, so the two keys can never disagree about which door
   * the player means.
   */
  const aimPick = (out, only) => {
    const R = views.render;
    interactAim.x = R[1];
    interactAim.z = R[3];
    // Forward in XZ, on the wire's 256-bearing grid — `input.aimDir` is
    // `yaw_lut::yaw_dir`, not `Math.sin(input.yaw)`. The free-running yaw sits
    // up to 0.703 deg off the bearing the sim resolves with, and this pick is a
    // prediction of the sim's, so it has to be judged on the sim's value.
    input.aimDir(interactAim);
    interactAim.only = only || VERB_NONE;
    interactWorld.defs = views.deployDefs;
    interactWorld.recs = deployRecs.values();
    interactWorld.bagCount = ex.client_bags_len();
    interactWorld.bagPos = views.bagPos;
    interactWorld.bagIds = views.bagIds;
    return resolveInteract(out, interactAim, interactWorld);
  };
  // Open a container the resolver picked: the panel, not the blind take-all.
  //
  // Nothing is drawn here. The view arrives as `ContSync` on the event lane
  // and `hud.openContainer` draws it then — the server owns whether this
  // container is open at all, so a panel that opened itself on the keypress
  // would be predicting visibility rather than contents.
  //
  // The panel is only visible with the inventory up — every drag it exists
  // for crosses between the two — so opening one opens that as well.
  const openPicked = (kind, handle) => {
    const len = ex.client_action_container(kind, handle);
    views.refresh();
    if (len === 0) return false;
    if (!actions.send(views.output, len)) return false;
    if (!hud.invOpen && hud.toggleInv()) document.exitPointerLock();
    return true;
  };
  // The payload-free take-all, kept as the fallback for a bag the open action
  // would not encode. It carries no target — the sim picks, inside the same
  // reach — so it is only ever reached for a bag the resolver already found.
  const takeAll = () => {
    const len = ex.client_action_loot();
    views.refresh();
    if (len > 0) actions.send(views.output, len);
  };
  const tryUse = () => {
    aimPick(pick, VERB_NONE);
    switch (pick.verb) {
      case VERB_DOOR:
        sendUse(pick.cx, pick.cz, pick.level, pick.loc);
        break;
      case VERB_BAG:
        // Opening beats emptying: the panel lets you leave the stone and take
        // the gunpowder, and the take-all is what happens when it will not
        // encode.
        if (!openPicked(CONT_BAG, pick.handle)) takeAll();
        break;
      case VERB_BOX:
        openPicked(CONT_BOX, pick.handle);
        break;
      case VERB_HEARTH:
        sendFeed(pick.cx, pick.cz, pick.level);
        break;
      default:
        // The honest answer, and the one the old chain could not give: it used
        // to report "no hearth in reach" for an empty island because the
        // hearth happened to be the last link tried.
        hud.toast("nothing in reach");
    }
  };
  // What a SWING would hit, on the sim's own terms. `terrain.cellEntry` is the
  // streamed scatter entry at a cell — the client's copy of the thing
  // `gather::swing` scans for — and the accessor is bound once here rather
  // than rebuilt per call, for the same no-per-frame-allocation reason the
  // world adapter above is reused.
  const swingPick = newSwingPick();
  const swingAim = { x: 0, y: 0, z: 0, fx: 0, fz: 0 };
  const swingWorld = {
    cellAt: (cx, cz) =>
      cx >= 0 && cz >= 0 && cx <= 0xffff && cz <= 0xffff
        ? terrain.cellEntry((((cx << 16) | cz) >>> 0))
        : null,
  };
  /**
   * Resolve what a swing would connect with right now. The cell key is
   * `cx<<16|cz`, which is `gather::cell_key` and `terrain.js`'s `cellIndex`
   * agreeing; the guard is what keeps a negative cell off the island from
   * packing into a positive key that names a real cell somewhere else — the
   * twelve-bit `box_key` trap this client already paid for once, in the one
   * other place it shifts a coordinate.
   */
  const swingAt = () => {
    const R = views.render;
    swingAim.x = R[1];
    swingAim.y = R[2];
    swingAim.z = R[3];
    // The same quantum, and here it is the one that matters most: `gather::swing`
    // runs a 30 deg cone off `yaw_dir(p.frame.yaw)`, so a bearing half a
    // quantum out puts a node on the far side of the cone edge from the arm.
    input.aimDir(swingAim);
    return resolveSwing(swingPick, swingAim, swingWorld);
  };
  /**
   * The prompt, off the HUD's slow timer. Same resolvers, same arguments as
   * the keys that act.
   *
   * Three verbs can be true at once — a ghost over the aimed cell, a box in
   * reach, a tree inside 2 m — and the row holds one. Which one is
   * `interact.centrePrompt`'s call and not this file's: it was three chained
   * `||`s here with nothing asserting the order, and `ci/ui_smoke.mjs` §V now
   * sweeps all eight combinations of the three in node. The ordering and its
   * reasons are documented at the function.
   *
   * This costs the short-circuit the `||` chain used to have — all three picks
   * are resolved every call now, where a door in reach used to skip the swing
   * scan. That is four times a second against two bounded scans (deploy recs
   * and bags; a 3x3 terrain cell block), off the slow timer and never the RAF
   * path, and it allocates nothing: the picks and `desc` are all reused
   * objects. Gating the order is worth a scan the worst case already paid.
   */
  const updatePrompt = () => {
    if (views.render[0] !== 1) {
      hud.setPrompt("");
      return;
    }
    hud.setPrompt(
      centrePrompt(
        build.on ? selDesc() : null,
        aimPick(pick, VERB_NONE),
        swingAt(),
        build.on ? null : aimRepair(),
      ),
    );
  };
  // ===========================================================================
  // The map's one wasm call.
  // ===========================================================================
  // `map.js` owns the geography and takes a sampler; this is the sampler, and
  // it is here rather than in `map.js` because `map.js` is node-importable and
  // wasm is not (`ci/ui_smoke.mjs` loads no wasm at all — that is what keeps it
  // in the code tier).
  //
  // Every ground fact comes out of the SAME worldgen the 3D ground is built
  // from, through the same three exports `terrainWorker.js` uses. The map
  // cannot therefore draw an island the player would not walk onto.
  //
  // The fresh-view discipline is `terrainWorker.js:99` and `clutterField.js`'s
  // comment, and it is not optional: wasm memory MOVES when it grows and a
  // held view detaches — the boot bug that shipped green on 2026-07-31. So the
  // `Float32Array` is constructed AFTER the fill, off `ex.memory.buffer` read
  // at that moment, and never cached across calls.
  let islandPainted = false;
  /** The last wire yaw the RAF loop sampled — see the assignment beside
   * `hud.setBearing`. The map marker's heading, and nothing else. */
  let lastYawU16 = 0;
  const paintIsland = () => {
    if (islandPainted) return;
    const size = MAP_N;
    const g = size + 2;
    const step = WORLD_M / size;
    // Where sample (0, 0) sits in the world: the CENTRE of the pixel it fills,
    // not that pixel's corner. Half a step, and the reason is the whole of the
    // judge's ranked fix 1 on `pass-20260805-074623-02`.
    //
    // That report named the symptom — `paintMap` flips rows by sample index
    // while `worldToMap` flips by continuous extent, "and the two disagree by
    // exactly one row, always" — and observed that the x axis is exact. The
    // asymmetry is the tell, and it points past the two formulas to the grid
    // under them: sampling from 0 put every sample on the LOW-x, LOW-z corner
    // of the pixel's extent, so the painted island is half a cell out on both
    // axes, and the row flip turns that half cell into a whole row. Sample j
    // at z = j*step projects to py = size - j exactly, which is the boundary
    // between rows, and `floor` takes the row to the south of the one it was
    // painted in. On x the same half-cell offset lands on the low edge of the
    // column, where `floor` happens to come out right — right by luck of the
    // half-open interval, not by construction.
    //
    // So neither formula was wrong; the sample grid was offset from the pixel
    // grid. Move the samples to the pixel centres and both are correct as
    // written: sample (i, j) lands at px = i + 0.5, py = size - 1 - j + 0.5 —
    // strictly INSIDE the pixel it is painted in, on both axes, with no
    // boundary for `floor` to tip over. `ui_smoke` §U asserts that agreement
    // directly rather than either formula alone.
    const orig = step / 2;
    // One call for the whole island: `terrain_fill_heights` refuses any side
    // above `HEIGHTS_MAX_N` (259) and 258 fits. It refuses by RETURNING 0 and
    // leaving the buffer untouched, so an unchecked call would paint whatever
    // was in the scratch buffer — hence the exact-count check, which is
    // `terrainWorker.js:98`'s, and `ui_smoke` §U pins the inequality that
    // makes it pass.
    const wrote = ex.terrain_fill_heights(seed, orig - step, orig - step, g, step);
    if (wrote !== g * g) {
      hud.toast("map unavailable");
      return;
    }
    const heights = new Float32Array(ex.memory.buffer, ex.terrain_heights_ptr(), g * g);
    const rgba = new Uint8ClampedArray(size * size * 4);
    paintMap(rgba, size, {
      heights,
      step,
      x0: orig,
      z0: orig,
      moistAt: (x, z) => ex.terrain_moisture_at(seed, x, z),
      splatAt: (h, moist, slope) => ex.terrain_splat_from(h, moist, slope),
    });
    hud.setMapTerrain(rgba, size);
    islandPainted = true;
  };
  const closeOpenContainer = () => {
    if ((ex.client_cont_kind() >>> 0) === CONT_SELF) return false;
    const len = ex.client_action_container(CONT_SELF, 0);
    views.refresh();
    if (len === 0) return false;
    return actions.send(views.output, len);
  };
  // L locks or unlocks it. Whether the door is yours is the server's
  // verdict — the wire carries the lock bit but never the owner — so the
  // press goes out either way and a refusal comes back as a toast. No
  // prediction rides along: the announcement is absolute and this is not
  // an action anyone spams.
  //
  // The door it acts on is the resolver's, restricted to `VERB_DOOR` — the
  // same aim metric E uses, so L can never lock a different door than the one
  // the prompt named. It was a separate nearest-door scan until 2026-08-05,
  // which is one more of the parallel picks the judge's gap 3 was about.
  const tryLock = () => {
    const best = aimPick(lockPick, VERB_DOOR);
    if (best.verb !== VERB_DOOR) {
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
  // The scan itself is `interact.nearestPiece` — pure, and gated in node.
  // It lived here until 2026-08-05 reading an undeclared `REACH`, so it
  // threw on its first line and every verb behind it was dead.
  const piecePick = newPiecePick();
  const pieceAt = { x: 0, z: 0 };
  const pieceWorld = { cell: BUILD_CELL, recs: [] };
  const aimPiece = () => {
    const R = views.render;
    pieceAt.x = R[1];
    pieceAt.z = R[3];
    // `values()` is a one-shot iterator, so it is taken fresh per scan.
    pieceWorld.recs = pieceRecs.values();
    return nearestPiece(piecePick, pieceAt, pieceWorld);
  };
  // U climbs the nearest piece one rung: wood → stone → metal. The wire
  // carries the rung, not the step, so the client only has to know what
  // the piece is made of today — and whether that rung exists, and who
  // may pay for it, stays the server's verdict, back as a toast. Nothing
  // is predicted: an upgrade never moves collision.
  const tryUpgrade = () => {
    const best = aimPiece();
    if (!best.found) {
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
  // R buys a damaged thing back — the other half of the piece's life story,
  // and until now the half nothing could press. Unlike U it addresses BOTH
  // stores: the door is the breach point a raid actually uses, and a repair
  // that could only reach built pieces would refuse to mend the one thing
  // most likely to be broken. `nearestRepairable` reports which store won
  // and that answer rides the wire as the leading argument — see the store
  // constants in `interact.js` for why getting it wrong is invisible.
  //
  // Nothing is predicted. A repair moves no collision and spends materials
  // the server owns the count of, so the client asks and reads the answer
  // back: either `SUB_PIECE_REPAIRED` (the `repaired left/max` toast, via
  // `structNews`) or a build refusal (`refusals.js`, "not damaged" and
  // "cannot be repaired" among them).
  const repairPick = newRepairPick();
  const repairAt = { x: 0, z: 0 };
  const repairWorld = { cell: BUILD_CELL, pieces: [], deploys: [] };
  const repairDesc = { what: "", costs: "", need: "" };
  const aimRepair = () => {
    const R = views.render;
    repairAt.x = R[1];
    repairAt.z = R[3];
    // Both `values()` calls are one-shot iterators, so they are taken fresh.
    repairWorld.pieces = pieceRecs.values();
    repairWorld.deploys = deployRecs.values();
    nearestRepairable(repairPick, repairAt, repairWorld);
    // The name is the caller's to fill (the def tables and the wasm string
    // table are both out of `interact.js`'s reach), and it is what decides
    // whether the prompt draws at all. A row that has not dripped yet leaves
    // `what` empty and the prompt stays blank rather than reading "REPAIR ?".
    if (repairPick.found) {
      const deploy = repairPick.store === REPAIR_STORE_DEPLOY;
      const have = deploy
        ? (ex.client_deploy_defs_state() >>> 0) & 0xffff
        : (ex.client_piece_defs_state() >>> 0) & 0xffff;
      if (repairPick.row < have) {
        repairPick.what = deploy
          ? describeDeploy(repairDesc, views.deployDefs, repairPick.row, itemName, invHave).what
          : describePiece(repairDesc, views.pieceDefs, repairPick.row, itemName, invHave).what;
      }
    }
    return repairPick;
  };
  const tryRepair = () => {
    const best = aimRepair();
    if (!best.found) {
      hud.toast("nothing to repair in reach");
      return;
    }
    const len = ex.client_action_repair(best.store, best.cx, best.cz, best.level, best.loc);
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
  // The death screen's two buttons. The action is one bit — the sim picks
  // which of your bags is nearest and whether it is ready, so there is no
  // id here to forge and no bag list the client has to keep honest
  // (world.rs). `askedForBag` is remembered so the wake can say "no bag
  // ready, you woke on a beach" instead of leaving the player to work it
  // out from the scenery.
  let askedForBag = false;
  hud.onRespawn = (onBag) => {
    askedForBag = onBag;
    const len = ex.client_action_respawn(onBag ? 1 : 0);
    views.refresh();
    if (len > 0) actions.send(views.output, len);
  };
  // Clicking a belt cell in the inventory screen is the same act as
  // pressing its digit key — one selection, `input.sel`, which is what
  // rides the input frame to the sim. The screen adds no second notion of
  // "held".
  hud.onInvSelect = (slot) => {
    input.sel = slot;
  };
  // The move verb's OUTBOUND half — this assignment is what arms the drag
  // (`Hud.NO_MOVE_HOST`), so the gesture does not exist until the host can
  // actually carry it.
  //
  // What the drag becomes — the count off `views.inv` rather than off the
  // panel's label, the two refusals for an end this client cannot address,
  // and above all the ORDER of the six arguments — is `moveArgs`, in
  // `invmove.js`, next to the inbound unpack and gated the same way
  // (`ci/ui_smoke.mjs` §N reads `client_action_move`'s parameter list out of
  // `bridge.rs` and holds the client to it). It was written out longhand here
  // until 2026-08-05, where the only gate that ever drove it was
  // `browser_smoke` — six positional `u32`s of the same type, whose
  // transposition the encoder golden, the replay hash and clippy are all
  // blind to. Spreading one array is what leaves nothing here to transpose.
  //
  // Ordering, which is the whole trap and is still this host's job:
  // `client_action_move` validates the shape and returns 0 for one the wire
  // will not carry, and it is asked BEFORE `dropInvDrag` draws anything. A
  // drawn move with no frame behind it is the container divergence itself,
  // and divergence is what the reference kept shipping as a disconnect.
  hud.onInvMove = (fromKind, from, toKind, to) => {
    views.refresh();
    // The handle comes off the BRIDGE, not off the panel — `client_cont_handle`
    // is the sim's own answer to "which container is open", and the panel's
    // `contHandle` is a copy of it that a stale frame could have left behind.
    // A move that named the wrong container is the divergence CLAUDE.md's trap
    // list is about, and `deploy.rs`'s `box_index` has no zero guard
    // (`box_key(0,0,0) == 0`), so a wrong handle is not harmlessly refused —
    // it addresses a real box. Handed over unconditionally: `moveArgs` zeroes
    // it for a self→self move and refuses a ground end that has none, so there
    // is no rule about the handle written a second time here.
    const args = moveArgs(
      ex.client_cont_handle() >>> 0,
      fromKind,
      from,
      toKind,
      to,
      views.inv,
      views.cont,
    );
    if (args === null) return false;
    const len = ex.client_action_move(...args);
    views.refresh();
    if (len === 0) return false;
    return actions.send(views.output, len);
  };
  document.addEventListener("keydown", (e) => {
    if (closed) return;
    if (hud.chatOpen) return; // the composer's own handler has it
    if (hud.deathOpen) {
      // A corpse does not walk, build, chat or swing — the sim refuses all
      // of it anyway (`live_slot_of`), and a client that kept sending
      // would be predicting a body the server is not moving. The two keys
      // that mean something here are the two buttons.
      if (e.code === "Digit1" || e.code === "KeyF") hud.answerDeath(true);
      else if (e.code === "Digit2" || e.code === "KeyG") hud.answerDeath(false);
      e.preventDefault();
      return;
    }
    if (e.code === "KeyT" || e.code === "Enter") {
      hud.openChat();
      document.exitPointerLock();
      e.preventDefault();
      return;
    }
    // The inventory screen. Tab because that is the key the genre trained
    // every player of it to press, and nothing here bound it. Escape
    // closes, the same way it closes the composer. preventDefault on both
    // or Tab walks the browser's own focus ring off the canvas.
    if (e.code === "Tab") {
      if (hud.toggleInv()) document.exitPointerLock();
      else closeOpenContainer();
      e.preventDefault();
      return;
    }
    if (hud.invOpen && e.code === "Escape") {
      hud.toggleInv();
      closeOpenContainer();
      e.preventDefault();
      return;
    }
    // The map. M because it is the genre's key for it and nothing here bound
    // it. It releases pointer lock like the inventory does: this is a screen
    // you read, not an overlay you fight under, and `hud.eatsKey` below states
    // the same thing about the keyboard.
    //
    // The island is painted on the FIRST open and never again — it is a
    // function of the seed alone, the seed does not change inside a session,
    // and 66,564 height samples through wasm is not a thing to do four times a
    // second. Painting it lazily rather than at boot keeps it off the join
    // path, where `browser_smoke` measures how long the client takes to reach
    // the world.
    if (e.code === "KeyM") {
      if (hud.toggleMap()) {
        paintIsland();
        document.exitPointerLock();
      }
      e.preventDefault();
      return;
    }
    if (hud.mapOpen && e.code === "Escape") {
      hud.toggleMap();
      e.preventDefault();
      return;
    }
    // An open panel owns the keyboard: every verb below spends something —
    // materials, a swing, a door — and a player reading their bag asked
    // for none of it. One question, asked before any mutation, rather than
    // a guard bolted onto each branch (hud.eatsKey, ci/ui_smoke.mjs I).
    if (hud.eatsKey(e.code)) {
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
      // The centre hint changes owner on this key — into and out of the build
      // row — so it is redrawn on the keypress and not up to 250 ms later on
      // the HUD timer. A hint that lags the ghost it describes is the defect.
      updatePrompt();
      e.preventDefault();
    } else if (build.on && (e.code === "KeyR" || e.code === "KeyF")) {
      const d = e.code === "KeyR" ? 1 : -1;
      build.level = Math.max(0, Math.min(MAX_LEVEL, build.level + d));
      buildStrip();
      e.preventDefault();
    } else if (e.code === "KeyE") {
      tryUse();
      e.preventDefault();
    } else if (e.code === "KeyG") {
      // Eat what is in the selected hotbar slot. G rather than a
      // right-click because the swing arm is already spoken for and a
      // consume that shared it would fire every time you chopped a tree
      // holding berries. Whether the slot holds food is the sim's
      // verdict, announced back either way (survival.rs).
      const len = ex.client_action_consume(input.sel);
      views.refresh();
      if (len > 0) actions.send(views.output, len);
      e.preventDefault();
    } else if (e.code === "KeyH") {
      // Drink from the water at your feet. H because G is already the
      // eat and the two are the same gesture from the player's side —
      // adjacent keys, one hand. Payload-free: the sim reads the
      // heightfield under the body, so there is nothing to aim and no
      // reach for the client to guess (survival.rs).
      const len = ex.client_action_drink();
      views.refresh();
      if (len > 0) actions.send(views.output, len);
      e.preventDefault();
    } else if (e.code === "KeyL") {
      tryLock();
      e.preventDefault();
    } else if (e.code === "KeyU") {
      tryUpgrade();
      e.preventDefault();
    } else if (e.code === "KeyR") {
      // Repair, and ONLY out of build mode. R is already the build-level
      // raise, and that branch sits above this one in this same chain — the
      // ordering is the binding, so it is asserted in `ui_smoke` §X rather
      // than left to whoever next reorders these branches. `updatePrompt`
      // reads the same `build.on` question the other way round, so the row
      // never advertises `[R] REPAIR` while R would step a floor up.
      tryRepair();
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
      // The wheel changes WHICH piece the hint names, so it redraws here too.
      updatePrompt();
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

  // The open container's cells, slot-indexed, exactly as the inventory pump
  // formats its own. Only the slots that container KIND actually has — a box
  // is twelve inside a thirty-slot view and the tail is zero, so formatting
  // all thirty would draw eighteen empty cells the sim would refuse a drop
  // into (`slotsIn`, and `hud.openContainer` hides them for the same reason).
  const contTexts = (kind) => {
    const out = [];
    for (let s = 0; s < slotsIn(kind); s++) {
      const count = views.cont[s * 2 + 1];
      out.push(count > 0 ? `${itemName(views.cont[s * 2])} ×${count}` : "");
    }
    return out;
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
      // Word 1 of the applied word, read unconditionally and right here,
      // because word 0 has no spare bit to announce it with — bits 0..30
      // are flags and bit 31 is the error below (`bridge.rs`'s
      // `client_applied2`). It is a load, it is zero on any message that
      // set nothing in it, and it stays valid until the next
      // `client_on_stream` — so reading it now and acting on it at the
      // bottom of this handler cannot see a stale verdict.
      const applied2 = ex.client_applied2() >>> 0;
      if (flags & STREAM_HIGH_BIT) {
        // Bit 31 is `STREAM_ERR` and nothing else. It used to be
        // `APPLIED_MOVE` as well, and this branch logs and returns EARLY —
        // so the first landed move of a session took the inventory diff
        // riding the same message out with it. `web/src/invmove.js` has
        // the history; the verdict now arrives on `APPLIED2_MOVE` below
        // and no longer competes with the error for this bit.
        //
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
      if (flags & (1 << 28) /* CONSUME */) {
        // The eat landed or it didn't, and the player is told which. A
        // press that vanishes silently is indistinguishable from a
        // broken key, which is the whole reason the sim announces a
        // refusal at all (survival.rs).
        const c = ex.client_consume() >>> 0;
        const reason = c >>> 24;
        if (reason === 0) hud.toast(`ate ${itemName(c & 0xffff)}`);
        else if (reason === 2) hud.toast("already full");
        else if (reason === 3) hud.toast("no water in reach");
        else hud.toast("not food");
      }
      if (flags & (1 << 29) /* DRANK */) {
        // The sea is salt, so the toast names both halves: a health drop
        // with no cause on screen is the thing `EV_DRANK` exists to
        // prevent (survival.rs).
        const d = ex.client_drank() >>> 0;
        const cost = d & 0xffff;
        hud.toast(cost > 0 ? `drank +${d >>> 16} −${cost} hp` : `drank +${d >>> 16}`);
      }
      if (flags & (1 << 26) /* STRUCT_HIT */) {
        // The breach readout, and its opposite. A repair raises this flag
        // too and only a hit also raises APPLIED_HIT, so the two are told
        // apart on the bit the wasm sets for exactly that — see
        // `interact.structNews`, which is where the whole rule lives.
        const news = structNews(flags, ex.client_struct_hit_hp() >>> 0);
        if (news) hud.toast(news);
      }
      if (flags & (1 << 30) /* RESPAWN — the death screen opened or closed */) {
        // One flag, two events: `client_death_screen` says which. Packed
        // `dead << 24 | woke_on_bag << 16 | cause` (bridge.rs), so one call
        // across the wasm boundary answers the whole question.
        const screen = ex.client_death_screen() >>> 0;
        if (screen >>> 24) {
          const killer = ex.client_death_by() >>> 0;
          const w = ex.client_death_weapon() >>> 0;
          const item = w >>> 16;
          hud.showDeath(
            screen & 0xff,
            killer,
            item === 0xffff ? null : itemName(item),
            (w & 0xffff) / 100,
            killer === playerId,
          );
          document.exitPointerLock();
        } else {
          hud.hideDeath(((screen >>> 16) & 1) === 1, askedForBag);
          askedForBag = false;
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
          hud.toast(`can't craft: ${craftRefusal(r)}`);
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
          hud.toast(`can't build: ${buildRefusal(r)}`);
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
          hud.toast(`can't place: ${deployRefusal(r)}`);
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
      // The move verdict, LAST — read at the top of this handler per the
      // bridge contract, applied here so that nothing else the same
      // message carried is skipped or reordered by it. That is not a
      // preference: the old code took the error branch on this verdict and
      // returned early, and the inventory diff riding the same message
      // went with it. Landing the verdict after the word-0 dispatch makes
      // that failure unrepresentable rather than fixed.
      // The open container, BEFORE the move verdict. The order is the
      // point: a `ContSync` that re-aims or closes the view invalidates a
      // move in flight against the old one (`hud.abandonContainerMove` —
      // the verdict word carries no handle and no sequence number, so there
      // is nothing in it that could tell them apart). Applying the verdict
      // first would resolve it against a container that is already gone.
      if (applied2 & APPLIED2_CONT) {
        // `client_on_stream` above can have grown wasm memory and detached
        // every view with it — the boot bug no native gate could see.
        views.refresh();
        const kind = ex.client_cont_kind() >>> 0;
        if (kind === CONT_SELF) hud.closeContainer();
        else hud.openContainer(kind, ex.client_cont_handle() >>> 0, contTexts(kind));
      }
      if (applied2 & APPLIED2_MOVE) {
        const v = moveVerdict(ex.client_move_readout());
        if (v) {
          // `invMoveVerdict` re-checks the address against its own pending
          // record before it unwinds anything — this route is not trusted
          // to have matched. The TO kind is not in the word at all, so it
          // is not passed: see `hud.invMoveVerdict`.
          hud.invMoveVerdict(v.reason, v.from, v.to, v.fromKind);
          views.refresh();
        } else {
          // The sim said a move resolved and handed us a word that is not
          // a verdict this panel can act on. That is the corruption signal
          // the collision used to swallow, and it is worth a report for
          // exactly that reason.
          console.error("event lane: APPLIED2_MOVE carried a malformed readout");
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
  // The daylight register's probe: four renders a yaw, each one the frame
  // minus exactly one of the register's parts (the dome, the air, the key),
  // so what it reports is the part and never the box's rasterizer.
  const devDaylightProbe = dev
    ? (yaws, pitch, minDelta, heightM) =>
        scene.daylightProbe(yaws, pitch, minDelta, heightM)
    : null;
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
  // `minChroma` is materials v3's: 0 leaves the chroma track off (grain's
  // call), and a positive value arms the second mask the tint octave needs
  // because it moves no luma for a luma mask to find.
  const devGrainProbe = dev
    ? (uniformName, specs, minDelta, minChroma = 0) => {
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
        return scene.contrastProbe(views, uniformName, minDelta, minChroma);
      }
    : null;
  // The alias probe: the same world-space view shape the contrast probe takes,
  // because it asks its question at the capture vantages and those are aimed,
  // not derived. What it measures is not whether a term reaches the image but
  // whether a screen DERIVATIVE does — see `scene.aliasProbe`.
  const devAliasProbe = dev
    ? (specs, minDelta) => {
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
        return scene.aliasProbe(views, minDelta);
      }
    : null;
  // The base-map probe (ART.md §7). Takes (yaw, pitch) from the player's own
  // eye like the surface and grain probes do, because the number it is aimed
  // at — ART.md §3's near-ground neighbour contrast — is a statement about
  // what the ground looks like from standing height and not at a world point.
  const devBaseProbe = dev ? (views, minDelta) => scene.baseProbe(views, minDelta) : null;
  const devChromaProbe = dev ? (views, minDelta) => scene.chromaProbe(views, minDelta) : null;
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
  // Prop surfaces v0: the same difference shape again, but what it toggles is
  // the field on everything that is NOT the ground. Terrain owns where the
  // props are, so it finds them; this scope owns the camera, so it computes the
  // eye.
  //
  // `off` is a metre offset from the instance's own origin, scaled by its
  // instance scale, and `aim` is the height on it the camera looks at — a fixed
  // offset rather than a bearing, so the frame is the same one every run at a
  // pinned spawn. Both views look DOWN at their prop from well above the
  // surrounding ground, and that is not framing taste: the first cut placed the
  // eye level with the trunk 7.5 m out and photographed a hillside, scoring
  // 0.00% on a class whose field is the strongest in the table. A prop is
  // wherever worldgen put it, and the only thing between an eye and a prop that
  // a probe cannot predict is terrain.
  // Lighting v1's pair. `tonalProbe` is the only probe in this file that
  // measures an ABSOLUTE quantity — where the image sits, not how much a
  // toggle moved it — and it is aimed by the same (yaw, pitch) vantages the
  // capture harness uses, so the register it reports is the register the
  // visual judge will be looking at. `sunProbe` aims down the key's own
  // direction and asks whether the dome drew a sun there.
  const devTonalProbe = dev ? (views) => scene.tonalProbe(views) : null;
  const devSunProbe = dev ? () => scene.sunProbe() : null;
  const devPropProbe = dev
    ? (specs, minDelta, radiusM) => {
        const p = scene.camera.position;
        const found = terrain.nearestProps(p.x, p.z, radiusM);
        const views = [];
        for (const s of specs) {
          const hit = found.find((f) => f.surface === s.surface);
          if (!hit) continue;
          const k = hit.scale || 1;
          views.push({
            label: s.label,
            surface: s.surface,
            distance: hit.distance,
            instances: hit.count,
            eye: [
              hit.pos[0] + s.off[0] * k,
              hit.pos[1] + s.off[1] * k,
              hit.pos[2] + s.off[2] * k,
            ],
            at: [hit.pos[0], hit.pos[1] + s.aim * k, hit.pos[2]],
          });
        }
        return { views, found, ...scene.propProbe(views, minDelta) };
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

    // A corpse takes no input. The server zeroes the frame it steps a dead
    // body with (world.rs) rather than skipping the step, so the predictor
    // has to zero the same three fields or every tick spent on the death
    // screen is a mispredict against a body that is not moving. Yaw and
    // pitch survive: they are the camera's, not the world's, and freezing
    // them would lock the view on the frame the player died.
    const dead = hud.deathOpen;
    // Sampled ONCE and used twice. The compass shows the bearing the wire
    // carries, so it must be the same sample the wire carried this frame —
    // calling `yawU16()` a second time for the readout would reintroduce,
    // between two statements, exactly the temporal seam `input.js` names.
    const yaw = input.yawU16();
    ex.client_set_input(
      dead ? 0 : input.buttons(),
      yaw,
      input.pitchU8(),
      dead ? 0 : input.moveX(),
      dead ? 0 : input.moveZ(),
      input.sel,
    );
    // The one HUD call in the RAF path, and the only one that belongs
    // here: see `Hud.setBearing` for why it costs no allocation. Yaw is
    // the camera's, not the world's, so a corpse still gets a compass —
    // the same reason the block above spares yaw and pitch from `dead`.
    hud.setBearing(yaw);
    // Kept for the map marker, which runs on the HUD's quarter-second timer
    // and therefore cannot reach this local. Parked rather than re-sampled:
    // `ci/ui_smoke.mjs` §T asserts main.js calls `yawU16()` EXACTLY once, and
    // that assertion is the whole of the seam argument three lines above —
    // the marker and the compass are one fact drawn twice, so they must come
    // from one sample, not from two calls a quarter second apart.
    lastYawU16 = yaw;
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
      // R[10] is the client's fixed sim tick — the wind and the fell both run
      // off it rather than off `now`, so a capture at a tick is repeatable.
      terrain.update(R[1], R[3], R[10]);
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
    // What E would do, from the same resolver the keypress uses. On this
    // timer and not in the RAF loop, per L8 (UI in plain DOM outside the
    // loop): a prompt that lags a quarter second behind the crosshair is a
    // prompt, and a sweep of every deployable in the RAF path is a frame
    // budget spent on a `<div>`.
    updatePrompt();
    // Where you are, and which way you face, on the same timer and for the
    // same reason. The heading is the wire's bearing quantum and not the
    // free-running `input.yaw`, because the compass strip reads the same
    // quantum and the marker is that one fact drawn a second way. It comes off
    // the RAF loop's parked sample rather than from a fresh call — §T of
    // `ci/ui_smoke.mjs` holds this file to one sample per frame, and that
    // assertion IS the argument: two reads a quarter second apart are two yaws.
    if (hud.mapOpen) hud.setMapView(R[1], R[3], lastYawU16);
    if (hud.invOpen) {
      // Slot-indexed and all 30, because that is setInventory's contract:
      // the belt row IS slots 0..5 (the six already formatted above) and
      // the grid IS 6..29. Only while the screen is open — a closed panel
      // reading 24 more slots and decoding 24 more names off the catalog
      // every 250 ms would be paying for pixels nobody is looking at.
      const slots = hotbar.slice();
      for (let s = 6; s < 30; s++) {
        const count = views.inv[s * 2 + 1];
        slots.push(count > 0 ? `${itemName(views.inv[s * 2])} ×${count}` : "");
      }
      hud.setInventory(slots);
      hud.setInvSelected(input.sel);
    }
    const health = ex.client_health() >>> 0;
    const vit = ex.client_vitals() >>> 0;
    const vitMax = ex.client_vitals_max() >>> 0;
    hud.setVitals(
      health >>> 16,
      health & 0xffff,
      vit >>> 16,
      vitMax >>> 16,
      vit & 0xffff,
      vitMax & 0xffff,
    );
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
      // The open container, as the BRIDGE reports it beside what the panel
      // drew. Two values rather than one on purpose: they disagreeing is
      // the container divergence, and a probe that only read the panel
      // could not see it.
      contKind: ex.client_cont_kind() >>> 0,
      contHandle: ex.client_cont_handle() >>> 0,
      panelContKind: hud.contKind,
      panelContHandle: hud.contHandle,
      // The lighting rig's structural facts plus last frame's draw counts
      // (DESIGN §9's < 300 calls / < 1.5 M tris). Read off the scene, not
      // recomputed — what the gate asserts is what the renderer did.
      lighting: scene.lighting(),
      // The sim's own vitals mirror, so the HUD gate can assert display
      // against authoritative state instead of against content's STARTING
      // values — which the survival clock now drains on the sim's schedule,
      // making "HUD equals content" a time-dependent (flaky) assertion.
      // [hp, maxHp, food, maxFood, water, maxWater], same unpack as
      // hud.setVitals above.
      vitals: (() => {
        const h = ex.client_health() >>> 0;
        const v = ex.client_vitals() >>> 0;
        const m = ex.client_vitals_max() >>> 0;
        return [h >>> 16, h & 0xffff, v >>> 16, m >>> 16, v & 0xffff, m & 0xffff];
      })(),
      // The death screen: whether the overlay is up, and the two bytes its
      // buttons put on the wire. The encoders are exposed rather than the
      // send, because a gate that sent one would be answering a screen
      // nobody raised — and `world::apply` would rightly ignore it.
      deathOpen: hud.deathOpen,
      encodeRespawn: (onBag) => ex.client_action_respawn(onBag),
      encodeRespawnByte: (onBag) => {
        const len = ex.client_action_respawn(onBag);
        views.refresh();
        return len > 0 ? views.output[0] : -1;
      },
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
      // Wind and felling: the knobs the vertex shader is actually running,
      // the clock it is standing at, and how many trees are mid-fall. The
      // clock is the assertable one — it is sim seconds, so a gate can read
      // it back and know the frame it captured was not timed off a wall.
      wind: terrain.windFacts(),
      // The horizon's caster: whether the far mesh casts, which side the
      // ground casts from, and the hole the near ring punches in it.
      farCaster: terrain.farCasterFacts(),
    };
    if (devSetView) debug.setView = devSetView;
    if (devShadowProbe) debug.shadowProbe = devShadowProbe;
    if (devSurfaceProbe) debug.surfaceProbe = devSurfaceProbe;
    if (devSplatCensus) debug.splatCensus = devSplatCensus;
    if (devDaylightProbe) debug.daylightProbe = devDaylightProbe;
    if (devFarShadowProbe) debug.farShadowProbe = devFarShadowProbe;
    if (devHorizonProbe) debug.horizonProbe = devHorizonProbe;
    if (devCostProbe) debug.costProbe = devCostProbe;
    if (devGrainProbe) debug.grainProbe = devGrainProbe;
    if (devProjectionProbe) debug.projectionProbe = devProjectionProbe;
    if (devAliasProbe) debug.aliasProbe = devAliasProbe;
    if (devSteepestFace) debug.steepestFace = devSteepestFace;
    if (devPropProbe) debug.propProbe = devPropProbe;
    if (devBaseProbe) debug.baseProbe = devBaseProbe;
    if (devChromaProbe) debug.chromaProbe = devChromaProbe;
    if (devTonalProbe) debug.tonalProbe = devTonalProbe;
    if (devSunProbe) debug.sunProbe = devSunProbe;
    globalThis.__gatesDebug = debug;
  }, 250);
}

if (!("WebTransport" in globalThis)) {
  errEl.textContent =
    "this browser has no WebTransport — need Chrome 97+, Edge 98+, Firefox 125+, or Safari 26.4+";
}
