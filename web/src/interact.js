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
