// The one resolver behind E, and the text the prompt shows for its answer.
//
// Why this file exists. Until 2026-08-05 E was a blind fallthrough chain in
// `main.js` — door, then bag, then box, then take-all, then hearth — where
// every link scanned the world itself and the first one that found anything
// acted. The judge's ranked gap 3 (`pass-20260805-002720-04`) named both
// consequences: standing between a hearth and a box, E did something you did
// not choose; and nothing on screen ever said the island had a verb at all,
// the only feedback in the entire chain being a toast fired when the LAST
// link failed ("no hearth in reach").
//
// The fix is that this file is the only thing that picks. The centre-screen
// prompt and the keypress call `resolveInteract` with the same arguments, so
// the prompt cannot advertise a verb the key does not perform — not because
// two code paths were written to agree, but because there is one.
//
// Pure and node-importable on purpose: no DOM, no wasm, no imports beyond
// `invmove.js`'s box packing. The pick is arithmetic, so `ci/ui_smoke.mjs`
// scores it in node in milliseconds rather than in a browser — the same
// reason `invmove.js` exists, and the same reason `ci/pine_shape.mjs` imports
// the shipped builder out of `props.js`.

import { boxKey } from "./invmove.js";

/**
 * The deployable archetypes this resolver can offer, mirrored from
 * `crates/sim-core/src/deploy.rs`. They arrive on the wire as
 * `deployDefs[row * 4]` and until now `main.js` restated all three as bare
 * literals at four scan sites (`1` for a hearth, `ARCH_BOX = 2`, `6` for a
 * door). `ci/ui_smoke.mjs` §Q reads these three back out of `deploy.rs`, so an
 * archetype renumbered on the Rust side lands red on the commit that renumbers
 * it rather than as E silently opening the wrong kind of thing.
 */
export const ARCH_HEARTH = 1;
export const ARCH_BOX = 2;
export const ARCH_DOOR = 6;

/**
 * How far E reaches, in metres. This is NOT a client knob: it is
 * `crates/sim-core/src/build.rs`'s `BUILD_REACH_M`, which the sim gates a
 * door use, a hearth feed, a box open (`deploy.rs:463`) and a bag loot
 * (`backpack.rs:45` aliases it as `LOOT_REACH_M`) on. Picking a target
 * outside it costs a round trip and a refusal, so the client picks inside the
 * same radius the server will accept — the quantize-both-sides law
 * (`CLAUDE.md`) applied to reach. `ui_smoke` §Q pins it to that constant.
 */
export const INTERACT_REACH_M = 5;

/**
 * How far off the aim line a thing may sit and still count as aimed at, in
 * metres. Proposed default, `DECISIONS.md` §open (interact aim radius v0) —
 * it is the one number this resolver needed that no doc and no Rust constant
 * already fixed, so it is written down there rather than left a literal here.
 *
 * 1.0 m is the deployable's own scale: a box and a hearth are roughly a metre
 * across in a 3 m build cell, so a crosshair within a metre of the centre is a
 * crosshair on the thing. It is not a reach and it is not a cone — see
 * `resolveInteract` for what it actually decides, which is only whether a
 * candidate is in the aimed rank or the nearby one.
 */
export const INTERACT_AIM_RADIUS_M = 1.0;

/** No verb: nothing is in reach. Never carries a prompt. */
export const VERB_NONE = 0;
export const VERB_DOOR = 1;
export const VERB_BAG = 2;
export const VERB_BOX = 3;
export const VERB_HEARTH = 4;
/** The highest verb. `ui_smoke` walks 1..VERB_MAX and requires a distinct
 * prompt AND a dispatch branch in `main.js` for every one of them, so a verb
 * added here without either lands red on the commit that adds it. */
export const VERB_MAX = 4;

/**
 * The noun each verb names, for the prompt. Generic kinds only — `CONTENT.md`
 * owns item names and these four name a KIND of thing, the same way
 * `hud.js`'s `CONT_NAMES` does for the container panel's title.
 */
export const VERB_LABEL = {
  [VERB_DOOR]: "DOOR",
  [VERB_BAG]: "BACKPACK",
  [VERB_BOX]: "BOX",
  [VERB_HEARTH]: "HEARTH",
};

/**
 * The tiebreak order, and the only place the old chain's ordering survives.
 *
 * Two candidates can score exactly equal — two boxes placed symmetrically
 * about the aim ray is the ordinary case, and floats compare exactly here
 * because both sides came out of the same arithmetic. The pick still has to
 * be a function of its inputs and nothing else, or the prompt drawn on one
 * tick and the verb run on the keypress could differ while the world stood
 * still. So a dead tie falls back to the order E has always used: a door is
 * aimed at, a bag is stood on, a box is the durable one, and a hearth is the
 * thing you meant if none of those is there.
 */
const TIE_ORDER = [VERB_NONE, VERB_DOOR, VERB_BAG, VERB_BOX, VERB_HEARTH];

/** A reusable pick. One is allocated per caller at wire-up and mutated in
 * place — the HUD timer runs this four times a second and `CLAUDE.md`'s
 * client law is no per-frame allocations. */
export function newPick() {
  return {
    verb: VERB_NONE,
    /** The container handle: a bag id, or a box's packed `box_key`. */
    handle: 0,
    /** The bag's index in `bagPos`/`bagIds`, or -1. */
    bag: -1,
    cx: 0,
    cz: 0,
    level: 0,
    loc: 0,
    /** Door state, straight off the wire (`main.js`'s deploy rec). */
    open: false,
    locked: false,
    /** Squared distance from the player, and from the aim line. Diagnostics
     * for the gate; nothing draws them. */
    d2: 0,
    perp2: 0,
    /** True when the pick was AIMED at rather than merely the nearest thing
     * in reach. See `resolveInteract`. */
    aimed: false,
  };
}

/**
 * What the player is aiming at, and what E therefore does.
 *
 * Two ranks, and the whole design is that the first one always beats the
 * second:
 *
 *   **aimed** — the candidate is in FRONT of the player (its projection onto
 *   the look direction is positive) and lies within `INTERACT_AIM_RADIUS_M`
 *   of the aim line. Among these the NEAREST wins, which is what a raycast
 *   would answer: a hearth a metre in front of a box is the thing between you
 *   and the box, not a worse version of it.
 *
 *   **nearby** — everything else in reach. Among these the nearest wins.
 *   This is the old chain's behaviour, kept exactly so that no verb that
 *   worked before stops working: a door behind you with nothing else around
 *   still opens, and a backpack you are standing on still opens without
 *   making you look down at it.
 *
 * Ties inside a rank fall to `TIE_ORDER`.
 *
 * The two ranks are what fixes the judge's case. Standing between a hearth and
 * a box, look at the box: the box is aimed, the hearth is merely nearby, and
 * the box wins however much closer the hearth is. Turn around and it reverses.
 * **Aim decides**, and nothing is ever excluded for being off-aim — being
 * off-aim only means losing to something that is not.
 *
 * The `t > 0` half of the aimed test is load-bearing and was learned by
 * probing this function rather than reasoning about it: without it, a hearth
 * whose cell centre is exactly under the player's feet sits at distance zero
 * from the aim line and trumps every box in the room, which is the old chain's
 * bug with a new cause.
 *
 * @param out   a pick from `newPick()`, mutated in place and returned
 * @param aim   `{x, z, fx, fz, reach, radius, only}` — feet position, look
 *              direction (need not be unit; it is normalised here so a caller
 *              cannot distort the metric by passing a scaled vector), the
 *              reach and aim radius to use, and optionally a single verb to
 *              restrict the pick to (`VERB_NONE`/absent = any).
 * @param world `{cell, defs, recs, bagCount, bagPos, bagIds}` — the build
 *              cell size, `deployDefs`, the deploy records (any iterable of
 *              `{cx, cz, level, loc, row, open, locked}`), and the bag views.
 */
export function resolveInteract(out, aim, world) {
  out.verb = VERB_NONE;
  out.handle = 0;
  out.bag = -1;
  out.cx = 0;
  out.cz = 0;
  out.level = 0;
  out.loc = 0;
  out.open = false;
  out.locked = false;
  out.d2 = 0;
  out.perp2 = 0;
  out.aimed = false;

  const reach = aim.reach === undefined ? INTERACT_REACH_M : aim.reach;
  const radius = aim.radius === undefined ? INTERACT_AIM_RADIUS_M : aim.radius;
  const only = aim.only || VERB_NONE;
  const reach2 = reach * reach;
  const radius2 = radius * radius;
  // Normalise once. A zero-length look direction is not a reason to refuse to
  // answer — the segment collapses to the player's own position and the metric
  // degrades to plain nearest-wins, which is exactly the old behaviour.
  const flen = Math.sqrt(aim.fx * aim.fx + aim.fz * aim.fz);
  const fx = flen > 0 ? aim.fx / flen : 0;
  const fz = flen > 0 ? aim.fz / flen : 0;

  // Best-so-far, as scalars rather than a candidate object, so the sweep
  // allocates nothing.
  let bestAimed = false;
  let bestD2 = Infinity;
  let bestTie = Infinity;

  /** Score one candidate; answer whether it takes the lead. */
  const wins = (verb, x, z) => {
    const dx = x - aim.x;
    const dz = z - aim.z;
    const d2 = dx * dx + dz * dz;
    if (d2 > reach2) return false; // out of the server's reach
    // Projection onto the look direction. Positive means in front; the
    // perpendicular offset is only meaningful there, which is why `aimed`
    // requires it.
    const t = dx * fx + dz * fz;
    const px = dx - t * fx;
    const pz = dz - t * fz;
    const perp2 = px * px + pz * pz;
    const aimed = t > 0 && perp2 <= radius2;
    const tie = TIE_ORDER.indexOf(verb);
    // Rank first, then distance inside the rank, then the chain's old order.
    if (bestAimed && !aimed) return false;
    if (aimed === bestAimed) {
      if (d2 > bestD2) return false;
      if (d2 === bestD2 && tie >= bestTie) return false;
    }
    bestAimed = aimed;
    bestD2 = d2;
    bestTie = tie;
    out.verb = verb;
    out.d2 = d2;
    out.perp2 = perp2;
    out.aimed = aimed;
    return true;
  };

  const half = world.cell / 2;
  for (const rec of world.recs) {
    const arch = world.defs[rec.row * 4];
    let verb = VERB_NONE;
    if (arch === ARCH_DOOR) verb = VERB_DOOR;
    else if (arch === ARCH_BOX) verb = VERB_BOX;
    else if (arch === ARCH_HEARTH) verb = VERB_HEARTH;
    else continue;
    if (only !== VERB_NONE && verb !== only) continue;
    // A box is addressed by its packed cell, and `boxKey` answers null for a
    // cell outside the build grid rather than packing a handle that would
    // alias into a neighbour's. A record off the grid is one this client
    // should not have, so it is not offered at all — the same "send nothing"
    // the old `tryOpenBox` chose, moved to where the pick is made so the
    // prompt cannot offer a box the key would decline to open.
    let handle = 0;
    if (verb === VERB_BOX) {
      const key = boxKey(rec.cx, rec.cz, rec.level);
      if (key === null) continue;
      handle = key;
    }
    // Reach is measured to the cell CENTRE for all three, which is the metric
    // `deploy.rs`'s `box_in_reach` and `door_in_reach` gate on.
    if (!wins(verb, rec.cx * world.cell + half, rec.cz * world.cell + half)) continue;
    out.handle = handle;
    out.bag = -1;
    out.cx = rec.cx;
    out.cz = rec.cz;
    out.level = rec.level;
    out.loc = rec.loc;
    out.open = rec.open === true;
    out.locked = rec.locked === true;
  }

  if (only === VERB_NONE || only === VERB_BAG) {
    for (let i = 0; i < world.bagCount; i++) {
      // A bag carries a world position, not a grid cell — it is dropped where
      // its owner died.
      if (!wins(VERB_BAG, world.bagPos[i * 3], world.bagPos[i * 3 + 2])) continue;
      out.handle = world.bagIds[i] >>> 0;
      out.bag = i;
      out.cx = 0;
      out.cz = 0;
      out.level = 0;
      out.loc = 0;
      out.open = false;
      out.locked = false;
    }
  }

  return out;
}

/**
 * What the prompt says for a pick, or `""` for nothing in reach.
 *
 * The key is named in the text because that is the whole job: the reference
 * genre's centre-screen hint (`Rust Images/choppingtree.jpg`) is how a player
 * learns the island has verbs at all. A door additionally reports the two bits
 * of its state the wire already carries — `open` says whether E closes it or
 * opens it, and `locked` is stated without claiming the press will fail,
 * because the wire carries the lock bit but never the owner and only the
 * server knows whether this door is yours.
 */
export function promptFor(pick) {
  if (!pick || pick.verb === VERB_NONE) return "";
  const label = VERB_LABEL[pick.verb];
  if (!label) return "";
  if (pick.verb === VERB_DOOR) {
    return `[E] ${pick.open ? "CLOSE" : "OPEN"} ${label}${pick.locked ? " · LOCKED" : ""}`;
  }
  if (pick.verb === VERB_HEARTH) return `[E] FEED ${label}`;
  return `[E] OPEN ${label}`;
}

// ---------------------------------------------------------------------------
// The other half of the crosshair: what a SWING would hit.
//
// E's resolver above answers a key you have to be told about. The swing is the
// mouse button players already hold down, and until now nothing on screen ever
// named its target either — the prompt landed on 2026-08-05 covering
// deployables only, and its own `NOW.md` entry said so ("gathering and building
// have no prompt, only deployables do"). This is that remainder.
//
// Why it is a SECOND resolver rather than four more verbs in the first one.
// The two picks disagree about every term of the metric: E reaches
// `build::BUILD_REACH_M` (5 m) and ranks an aim RADIUS against a nearby rank,
// while a swing reaches `gather::REACH_M` (2 m) through a 30° CONE with a
// vertical window and a point-blank exception, over a 3×3 block of terrain
// cells rather than over the deploy records. Folding them would have to pick
// one metric and would then advertise a verb on the wrong terms — the exact
// failure the one-resolver law exists to prevent. One resolver per key is the
// law; these are two keys.
//
// Everything below MIRRORS `crates/sim-core/src/gather.rs`'s `swing()` target
// selection and invents nothing. That is not a style preference: the sim picks
// the swing's target on its own, the client sends only a button bit
// (`BTN_PRIMARY`), so a prompt computed on any other rule would name a node the
// arm does not swing at. `ci/ui_smoke.mjs` §R reads every constant here back
// out of `gather.rs` and `terrain.rs`, so a rule changed on the Rust side lands
// red on the commit that changes it rather than as a prompt that quietly lies.
// ---------------------------------------------------------------------------

/**
 * Terrain cell size in metres — `terrain::CELL_SIZE`. This is NOT `BUILD_CELL`:
 * the scatter grid a node stands on is 8 m and the build grid is its own
 * thing, and the 3×3 block below is nine terrain cells.
 */
export const TERRAIN_CELL_M = 8;

/** Gather reach in metres — `gather::REACH_M`. Planar, like the sim's test. */
export const GATHER_REACH_M = 2.0;
/**
 * Cosine of the swing's aim cone half-angle (30°) — `gather::CONE_COS`,
 * authored offline there so the sim runs no trig, and mirrored as the same
 * literal here for the same reason the reach is.
 */
export const GATHER_CONE_COS = 0.8660254;
/** Vertical acceptance window in metres — `gather::DY_MAX_M`. */
export const GATHER_DY_MAX_M = 3.0;
/**
 * Squared planar distance inside which the cone test is bypassed —
 * `gather::POINT_BLANK_M2`. Standing in the node leaves no bearing to judge.
 */
export const GATHER_POINT_BLANK_M2 = 0.04;

/**
 * The terrain occupant ordinals a swing can connect with, mirrored from
 * `terrain::Occupant`. `gather::node_index` takes 1..=`GATHERABLE_KINDS` and
 * `target_index` adds the barrel; ROCK (6) and an empty cell are "the two
 * things a swing passes through" in that file's own words, so neither gets a
 * prompt however close it is.
 */
export const OCC_TREE = 1;
export const OCC_STONE = 2;
export const OCC_METAL = 3;
export const OCC_SULFUR = 4;
export const OCC_BUSH = 5;
export const OCC_ROCK = 6;
export const OCC_BARREL = 7;

/**
 * The verb and the noun for each swingable occupant. Generic kinds only, the
 * same rule `VERB_LABEL` and `hud.js`'s `CONT_NAMES` follow — `CONTENT.md`
 * owns item names and none of these is one.
 *
 * The verbs differ on purpose: a bush is picked and a tree is chopped, which
 * is the one thing the prompt can say about *why* a bare hand is fine on one
 * and slow on the other. A barrel is smashed and pays a container rather than
 * a resource (`gather::smash` → `loot.rs`), so it does not say "gather".
 */
export const SWING_LABEL = {
  [OCC_TREE]: "CHOP TREE",
  [OCC_STONE]: "MINE STONE",
  [OCC_METAL]: "MINE METAL",
  [OCC_SULFUR]: "MINE SULFUR",
  [OCC_BUSH]: "PICK BUSH",
  [OCC_BARREL]: "SMASH BARREL",
};

/** A reusable swing pick, allocated once per caller like `newPick`. */
export function newSwingPick() {
  return {
    /** The terrain occupant ordinal, or 0 for nothing swingable in reach. */
    arch: 0,
    cx: 0,
    cz: 0,
    /** Squared planar distance to the node. Diagnostics; nothing draws it. */
    d2: 0,
  };
}

/**
 * What a swing would connect with right now, or `arch = 0` for a whiff.
 *
 * A transcription of `gather::swing`'s scan (`gather.rs:494-532`), and the
 * places it would be tempting to improve on it are exactly the places it must
 * not be:
 *
 * - **The window is 3×3 terrain cells around the player's own cell**, not a
 *   radius sweep. A node whose cell is two cells away cannot be hit even if
 *   its position is inside the 2 m reach, because the sim never looks at it.
 * - **`d2 < best.d2` is strict**, and the sim walks dz outer, dx inner. So an
 *   exact tie goes to the cell found FIRST in that order. Ties are not
 *   hypothetical here — two nodes at mirrored offsets is ordinary — and a
 *   prompt that broke them the other way would name the other node.
 * - **The cone test is unnormalised on purpose**: `dot > CONE_COS * sqrt(d2)`
 *   with a unit look vector, which is the sim's own arrangement of it. Written
 *   as an angle comparison instead, it would round differently at the edge.
 * - **`dy` is measured from the player's feet to the node's own y**, and the
 *   test is on its absolute value against the window.
 *
 * The harvested bit is the client's, not the sim's: `lives.is_harvested` there
 * is `entry.hidden || entry.fellAt` here — BOTH, because a felled tree spends
 * 93 ticks down-but-not-yet-hidden and is already gone as far as the sim is
 * concerned. The event lane keeps them current (`EV_SLOT_HARVESTED` →
 * `terrain.setCellHarvested`). A node the client has not streamed has no entry
 * at all and is skipped — the prompt describes what this client can see, which
 * is the honest bound on a client-side hint.
 *
 * @param out   a pick from `newSwingPick()`, mutated in place and returned
 * @param aim   `{x, y, z, fx, fz}` — feet position (y included: the swing has
 *              a vertical window E does not) and look direction, normalised
 *              here so a caller cannot distort the cone with a scaled vector
 * @param world `{cellAt(cx, cz)}` — the streamed scatter entry at a cell, or
 *              null. One bound accessor, called nine times; nothing allocates.
 */
export function resolveSwing(out, aim, world) {
  out.arch = 0;
  out.cx = 0;
  out.cz = 0;
  out.d2 = 0;

  const flen = Math.sqrt(aim.fx * aim.fx + aim.fz * aim.fz);
  const fx = flen > 0 ? aim.fx / flen : 0;
  const fz = flen > 0 ? aim.fz / flen : 0;

  const pcx = Math.floor(aim.x / TERRAIN_CELL_M);
  const pcz = Math.floor(aim.z / TERRAIN_CELL_M);
  let best = Infinity;

  // dz outer, dx inner — the sim's own order, and the tiebreak depends on it.
  for (let dzc = -1; dzc <= 1; dzc++) {
    for (let dxc = -1; dxc <= 1; dxc++) {
      const cx = pcx + dxc;
      const cz = pcz + dzc;
      const e = world.cellAt(cx, cz);
      if (!e) continue;
      const arch = e.arch | 0;
      // 1..5 gather, 7 smashes. 6 (ROCK) and anything else is passed through,
      // which includes the client-only stump archetype standing where a felled
      // tree was: a stump is the CONSEQUENCE of a harvest, never a target.
      if (!(arch >= OCC_TREE && arch <= OCC_BUSH) && arch !== OCC_BARREL) continue;
      // Down, on this client's clock only — and "down" is `hidden` OR falling.
      //
      // `terrain.js`'s `setCellHarvested` defines the same state that way
      // (`entry.hidden || entry.fellAt !== 0`) and this must not define it
      // differently. A tree does not vanish when the sim harvests it: the event
      // sets `fellAt` and RETURNS, leaving `hidden` false while `_stepFells`
      // animates the fall, and only sets `hidden` once the trunk has landed and
      // sunk — `FELL_TICKS + FELL_SINK_TICKS` = 33 + 60 = 93 ticks, which at
      // `TICK_HZ` 30 is 3.1 seconds. Reading `hidden` alone therefore offered
      // "[LMB] CHOP TREE" for three seconds over a tree the sim had already
      // taken, and the swing it invited would whiff.
      if (e.hidden || e.fellAt) continue;
      const dx = e.x - aim.x;
      const dy = e.y - aim.y;
      const dz = e.z - aim.z;
      const d2 = dx * dx + dz * dz;
      if (d2 > GATHER_REACH_M * GATHER_REACH_M) continue;
      if (Math.abs(dy) > GATHER_DY_MAX_M) continue;
      const aimed =
        d2 <= GATHER_POINT_BLANK_M2 || dx * fx + dz * fz > GATHER_CONE_COS * Math.sqrt(d2);
      if (!aimed) continue;
      if (d2 >= best) continue; // strict: the first of an exact tie keeps it
      best = d2;
      out.arch = arch;
      out.cx = cx;
      out.cz = cz;
      out.d2 = d2;
    }
  }
  return out;
}

/**
 * What the prompt says for a swing pick, or `""` for a whiff.
 *
 * `[LMB]` rather than a verb name because the swing is a button, and the
 * button is the thing the player has to connect the text to.
 */
export function promptForSwing(pick) {
  if (!pick || !pick.arch) return "";
  const label = SWING_LABEL[pick.arch];
  if (!label) return "";
  return `[LMB] ${label}`;
}

// ---------------------------------------------------------------------------
// The third half of the crosshair: what a PLACEMENT would put there.
//
// Build mode was the one verb with no centre hint at all. `main.js:287` calls
// it "the plain-UI stand-in for the radial at alpha" and it draws a green
// wireframe ghost 3.5 m ahead, but the only text naming the button that
// commits it is the bottom strip 96 px above the hotbar — and until this
// landed, the hint UNDER the crosshair went on advertising `[LMB] CHOP TREE`
// while the ghost sat over the aimed cell. The player's eye is on the ghost;
// the row that describes it was somewhere else.
// ---------------------------------------------------------------------------

/** Stride of one row of the wasm piece-def table (`views.pieceDefs`). */
export const PIECE_DEF_STRIDE = 8;
/** Where a piece row's ingredient pairs start, and how wide a pair is. */
export const PIECE_COST_AT = 4;
export const PIECE_COST_STRIDE = 2;
/** Stride of one row of the wasm deployable-def table (`views.deployDefs`). */
export const DEPLOY_DEF_STRIDE = 4;
// UI labels for sim-core `build.rs`'s shape and material codes — labels, not
// content: `CONTENT.md` owns every number and these name none of them. They sat
// in `main.js` beside the one function that read them; they moved here so the
// decode below could move here, and `ci/ui_smoke.mjs` §V walks both against
// `build.rs`'s own enums.
export const BUILD_SHAPE_LABEL = ["foundation", "wall", "doorway", "floor", "stairs", "roof"];
export const BUILD_MAT_LABEL = ["wood", "stone", "metal"];

/**
 * Decode one row of the piece-def table into `{ what, costs, need }`.
 *
 * Pure, and here rather than in `main.js` for exactly one reason: it is a
 * POSITIONAL read of a flat table — shape at `b`, material at `b + 1`, the
 * ingredient count at `b + 3`, and the pairs at `b + 4 + k*2` — and
 * `CLAUDE.md`'s trap list names that shape as where the reference ecosystem
 * actually bled (49 Oxide commits touching a hook's arguments, ~27 correcting
 * a payload that had already shipped wrong). Swap the item and the quantity in
 * that pair and the client asks for "3 stone wall" instead of "200 wood": a
 * byte-golden cannot see it, every field is a number, and nothing else here
 * would go red. In `main.js` it was a closure inside `run()` that no gate could
 * import. Here `ci/ui_smoke.mjs` §V evaluates it against a hand-built table.
 *
 * `itemName` and `have` are injected because both are the caller's: item names
 * come from the wasm string table and inventory counts from `views.inv`, and
 * this file may not reach either. Fills a caller-owned object — the client is a
 * hot path and this runs off the HUD timer four times a second.
 */
export function describePiece(out, defs, row, itemName, have) {
  const b = row * PIECE_DEF_STRIDE;
  out.what = `${BUILD_MAT_LABEL[defs[b + 1]] || "?"} ${BUILD_SHAPE_LABEL[defs[b]] || "?"}`;
  out.need = "";
  const parts = [];
  for (let k = 0; k < defs[b + 3]; k++) {
    const item = defs[b + PIECE_COST_AT + k * PIECE_COST_STRIDE];
    const qty = defs[b + PIECE_COST_AT + k * PIECE_COST_STRIDE + 1];
    parts.push(`${qty} ${itemName(item)}`);
    // First unmet ingredient only. The build strip already lists every cost;
    // the hint is ONE row, and the actionable number for a player standing at
    // a ghost is how much more to go and get.
    const gap = qty - have(item);
    if (gap > 0 && !out.need) out.need = `${gap} more ${itemName(item)}`;
  }
  out.costs = parts.join(" + ");
  return out;
}

/**
 * Decode one row of the deployable-def table into the same shape.
 *
 * A deployable is placed FROM the stack rather than crafted in place, so its
 * cost is one of itself — and the item id is at `b + 3`, the same positional
 * read with a different stride.
 */
export function describeDeploy(out, defs, row, itemName, have) {
  const item = defs[row * DEPLOY_DEF_STRIDE + 3];
  out.what = itemName(item);
  out.costs = `1 ${out.what}`;
  out.need = have(item) < 1 ? `1 ${out.what}` : "";
  return out;
}

/**
 * What the prompt says for a build pick, or `""` when build mode is off (or
 * the piece table has not arrived, which is `what === ""`).
 *
 * `[RMB]` for the same reason the swing says `[LMB]`: right-click is what
 * commits the placement (`main.js`'s `mousedown`, button 2), and the button is
 * the thing the text has to connect to. `need` is the SHORTFALL of the first
 * ingredient the player is missing, not the piece's total cost — the total is
 * already on the build strip, and the number a player standing at a ghost
 * wants is how much more wood to go and get. `""` means affordable, which is
 * the ordinary case and gets no chrome.
 *
 * Affordability is advisory and stated as a cost, never as a refusal: the
 * server owns whether this placement is legal (materials is only one of
 * `BUILD_REFUSE_TEXT`'s reasons), so this line never says "can't".
 */
export function promptForBuild(pick) {
  if (!pick || !pick.what) return "";
  const head = `[RMB] PLACE ${pick.what.toUpperCase()}`;
  return pick.need ? `${head} · NEED ${pick.need.toUpperCase()}` : head;
}

/**
 * The one centre hint, chosen — the whole of `#prompt`'s policy, in one pure
 * function so it can be gated.
 *
 * The reference's centre hint is a SINGLE row (`Rust Images/choppingtree.jpg`)
 * and two stacked prompts under a crosshair is a menu, not a hint — so three
 * candidate verbs have to become one, and the ordering is the entire decision.
 * It was three chained `||`s inside `main.js`'s RAF-adjacent `updatePrompt`
 * with nothing asserting it, which is precisely the positional shape
 * `CLAUDE.md`'s trap list names: swap two of these and every other gate stays
 * green while the crosshair advertises a verb the button will not perform.
 *
 * Build outranks E outranks the swing, and the reason is which verb the
 * player's own attention is already on. In build mode there is a ghost drawn
 * over the aimed cell and right-click is about to act on it; E still works
 * (the build strip says so) but it is not what the eye is on. Off build mode,
 * E outranks the swing because the mouse button is already held down while
 * nothing on screen would ever suggest pressing E — the half a player cannot
 * otherwise discover wins the row.
 */
export function centrePrompt(buildPick, interactPick, swingPick) {
  return (
    promptForBuild(buildPick) || promptFor(interactPick) || promptForSwing(swingPick)
  );
}

// =============================================================================
// The other half of a piece's life story: what breaks it, and what mends it.
// =============================================================================

/**
 * Where a piece's address sits on the ground, mirrored from
 * `crates/sim-core/src/build.rs`'s `LOC_*`. The same physical edge is never
 * addressable twice, so which of the four a piece carries decides where its
 * anchor is — and the anchor is what BOTH reach checks measure to.
 */
export const LOC_PLANE = 0;
export const LOC_RISER = 1;
export const LOC_EDGE_W = 2;
export const LOC_EDGE_N = 3;

/**
 * The planar anchor of a build address — `build.rs`'s `anchor`, said in JS.
 *
 * This is a positional payload in the exact sense `CLAUDE.md`'s trap list
 * means it: swap the two `half` terms and every byte-golden stays green while
 * the client measures reach to the wrong corner of the cell, so U and the
 * repair key refuse at a distance the server would have accepted (and reach at
 * one it will not). It is written out here, rather than left inline in
 * `main.js`, so `ui_smoke` can walk it against the Rust in node.
 *
 * `out` is a two-element array, mutated in place — this runs inside a scan
 * over every piece in reach and the hot-path law forbids the tuple.
 */
export function pieceAnchor(out, cx, cz, loc, cell) {
  const x0 = cx * cell;
  const z0 = cz * cell;
  const half = cell * 0.5;
  out[0] = loc === LOC_EDGE_W ? x0 : x0 + half;
  out[1] = loc === LOC_EDGE_N ? z0 : z0 + half;
  return out;
}

/** The pick `nearestPiece` fills. `found` is false when nothing is in reach. */
export function newPiecePick() {
  return { found: false, cx: 0, cz: 0, level: 0, loc: 0, row: 0, d2: 0 };
}

/**
 * The nearest placed piece within reach of the player's feet — the target for
 * the verbs that address a piece rather than a thing under the crosshair (U
 * today, repair when the client can send one).
 *
 * Why it is here and not in `main.js`. It WAS in `main.js`, and it read
 * `bestD = REACH * REACH` against a `REACH` that is declared nowhere in the
 * repo — so `nearestPiece` threw a `ReferenceError` on its first line and U
 * had been dead at runtime for as long as the binding existed. Nothing caught
 * it: `ui_smoke` cannot execute `main.js` (it boots three.js and is stubbed at
 * the route), `browser_smoke` never presses U, and a free variable is not a
 * syntax error, so the bundle built clean. The fix is not the missing
 * constant — it is that the arithmetic now lives where a node gate can call
 * it, which is the same move `describePiece` made for the same reason.
 *
 * The reach is `INTERACT_REACH_M`, i.e. the sim's own `BUILD_REACH_M`, which
 * is what `build.rs`'s `repair` and `upgrade` both gate on. Quantize both
 * sides: the client picks inside the radius the server will accept.
 *
 * `at` = `{x, z}` (the feet, world metres), `world` = `{cell, recs}` where
 * `recs` is any iterable of `{cx, cz, level, loc, row}`.
 */
const ANCHOR_SCRATCH = [0, 0];
export function nearestPiece(out, at, world) {
  out.found = false;
  out.cx = 0;
  out.cz = 0;
  out.level = 0;
  out.loc = 0;
  out.row = 0;
  out.d2 = 0;
  let bestD = INTERACT_REACH_M * INTERACT_REACH_M;
  for (const rec of world.recs) {
    const a = pieceAnchor(ANCHOR_SCRATCH, rec.cx, rec.cz, rec.loc, world.cell);
    const dx = a[0] - at.x;
    const dz = a[1] - at.z;
    const d2 = dx * dx + dz * dz;
    if (d2 < bestD) {
      bestD = d2;
      out.found = true;
      out.cx = rec.cx;
      out.cz = rec.cz;
      out.level = rec.level;
      out.loc = rec.loc;
      out.row = rec.row;
      out.d2 = d2;
    }
  }
  return out;
}

/**
 * What the sim says when it turns down a build, an upgrade or a repair,
 * indexed by `build.rs`'s `REFUSE_B_*` code.
 *
 * The index IS the sim's number, which is the whole reason this table cannot
 * live as a bare array in `main.js` where nothing can walk it: it fell one
 * entry short of the sim the day `REFUSE_B_INTACT` (9) landed, so repairing a
 * wall that is already whole — by far the likeliest repair refusal, and the
 * one every player will hit first — answered `can't build: code 9`. `ui_smoke`
 * §W now walks it against the constants `build.rs` declares, by name and by
 * value, so the next reason the sim grows lands red on the commit that grows
 * it rather than as a number on a player's screen.
 *
 * It has since done exactly that once: `REFUSE_B_UNPRICED` (10) landed from
 * the sim lane and this table stayed at ten entries, and the gate was red on a
 * clean tree the same run rather than silent until a player saw `code 10`.
 */
export const BUILD_REFUSE_TEXT = [
  "no such piece",
  "spot taken",
  "needs support",
  "bad ground",
  "out of reach",
  "missing materials",
  "world is full",
  "claimed by a hearth",
  "nothing to upgrade into",
  "not damaged",
  "cannot be repaired",
];

/** The refusal sentence, or the bare code when the sim is ahead of us. */
export function buildRefusal(code) {
  return BUILD_REFUSE_TEXT[code] || `code ${code}`;
}

/**
 * The two flag bits that carry a structure's hp news, mirrored from
 * `crates/client-wasm/src/core.rs`.
 */
export const APPLIED_HIT_BIT = 1 << 23;
export const APPLIED_STRUCT_HIT_BIT = 1 << 26;

/**
 * The breach readout — and, since repair v0, its opposite.
 *
 * `EventMsg::StructHit` and `EventMsg::PieceRepaired` write the SAME readout
 * from opposite directions and both raise `APPLIED_STRUCT_HIT`; only a hit
 * also raises `APPLIED_HIT`, because only a hit struck somebody. `core.rs:98`
 * states it outright — *"A reader that wants only raid damage checks for
 * both"* — and the one reader in this client checked one, so a wall being
 * mended announced itself as a wall being broken: `breaching 750/750`, the
 * most alarming sentence in the game, fired by your own repair. A law with a
 * prose statement and no gate is the class `CLAUDE.md` names; this function is
 * the gate's surface.
 *
 * `hp` is the packed `left << 16 | max` of `client_struct_hit_hp`. A `max` of
 * 0 means the piece's def row has not arrived, so there is no bar to draw and
 * the honest answer is silence.
 */
export function structNews(flags, hp) {
  if (!(flags & APPLIED_STRUCT_HIT_BIT)) return "";
  const max = hp & 0xffff;
  if (max === 0) return "";
  const left = hp >>> 16;
  return flags & APPLIED_HIT_BIT ? `breaching ${left}/${max}` : `repaired ${left}/${max}`;
}
