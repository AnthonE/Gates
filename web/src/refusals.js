/**
 * Every "no" the server can send, as the sentence a player reads.
 *
 * Four tables, one shape: the array INDEX is the sim's own refusal code, and
 * the string is what that code says out loud. Nothing else lives here.
 *
 * WHY THIS FILE EXISTS — it is a walkability fix, not a tidy-up. Three of
 * these four tables were module-private `const`s in `web/src/main.js`, which
 * no gate can import (`ui_smoke` stubs `/src/main.js` at the route because it
 * boots three.js, and `browser_smoke` never presses the keys that reach them).
 * A private array nothing can walk is the exact condition that let the build
 * table fall behind the sim TWICE:
 *
 *   - `REFUSE_B_INTACT` (9) landed and the table stayed at nine entries, so
 *     repairing a wall that was already whole — the likeliest repair refusal
 *     there is — answered `can't build: code 9`.
 *   - `REFUSE_B_UNPRICED` (10) landed from the sim lane one run after the gate
 *     was written, and the gate was red on a clean tree the same day rather
 *     than silent until a player saw `code 10`.
 *
 * That table was moved somewhere importable and gated; these three were the
 * same bug waiting on the same trigger, and `deploy.rs` alone declares
 * thirteen reasons behind a `main.js` array that answered `code 13` for the
 * fourteenth. They are all here now, exported, walked together by `ui_smoke`
 * §W against the Rust that declares the codes.
 *
 * WHAT THE GATE PROVES, AND WHAT IT DOES NOT. §W asserts, per table: the
 * parsed Rust codes are contiguous from zero (so a gap cannot make the count
 * agree vacuously), the JS length equals the Rust count, every code indexes a
 * non-empty string, no two codes share a sentence, the accessor never falls
 * through to `code N` for a code the sim has, and it DOES fall through past
 * the end. All of that is satisfied by a placeholder, and by something worse
 * than a placeholder: correct sentences in the WRONG ORDER. The judge of
 * pass-20260805-111501-02 exchanged the values of `REFUSE_D_HEARTH` (10) and
 * `REFUSE_D_DOOR` (11) in `deploy.rs`, touched nothing here, and the gate
 * stayed green while a player placing on a missing hearth was told "no door
 * there" — `CLAUDE.md`'s positional-payload trap, in this file.
 *
 * So §W also holds each sentence to its constant's NAME: a keyword per code
 * which that sentence must contain and no other sentence in the table may.
 * Any transposition is red by construction. It still reads no English — a
 * wrong-but-well-formed sentence that happens to contain its keyword passes —
 * but the ORDER, which is the part that actually shipped wrong, is walled.
 *
 * Adding a refusal: add the sentence at its code, and add its keyword to that
 * table's `mean` in `ci/ui_smoke.mjs` — the gate is red until you do, on
 * purpose. Adding a table: declare it below and add its row to §W's
 * `REFUSAL_TABLES`. Do not restate one of these arrays at a call site.
 */

/**
 * `crates/protocol/src/lib.rs`'s `REFUSE_*: u8` — why a shard turned the
 * connection down at hello, before there is a world to be in.
 */
export const CONNECT_REFUSE_TEXT = ["protocol version mismatch", "shard is full"];

/** `crates/sim-core/src/craft.rs`'s `REFUSE_*: u32`. */
export const CRAFT_REFUSE_TEXT = [
  "no such recipe",
  "bad count",
  "needs a station",
  "queue full",
  "missing ingredients",
];

/**
 * `crates/sim-core/src/build.rs`'s `REFUSE_B_*: u32` — a build, an upgrade or
 * a repair on a structure piece.
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

/**
 * `crates/sim-core/src/deploy.rs`'s `REFUSE_D_*: u32` — placing a deployable.
 *
 * Codes 1–7 share their names with `REFUSE_B_*` and mostly share their words,
 * but not all of them: `REFUSE_D_COST` is an item you are not carrying where
 * `REFUSE_B_COST` is materials you cannot afford. The two tables are kept
 * separate for that reason and no check ties them together.
 */
export const DEPLOY_REFUSE_TEXT = [
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

/**
 * The sentence, or the bare code when the sim is ahead of the client.
 *
 * The fallback is what keeps a wire ahead of us honest rather than
 * mislabelled: an unknown code prints as itself instead of borrowing the
 * nearest sentence, so a player who reports `code 13` is reporting something
 * a developer can find.
 */
const refusalIn = (table, code) => table[code] || `code ${code}`;

/** @returns {string} why the shard refused the connection. */
export function connectRefusal(code) {
  return refusalIn(CONNECT_REFUSE_TEXT, code);
}

/** @returns {string} why the craft was turned down. */
export function craftRefusal(code) {
  return refusalIn(CRAFT_REFUSE_TEXT, code);
}

/** @returns {string} why the build, upgrade or repair was turned down. */
export function buildRefusal(code) {
  return refusalIn(BUILD_REFUSE_TEXT, code);
}

/** @returns {string} why the deployable would not go down. */
export function deployRefusal(code) {
  return refusalIn(DEPLOY_REFUSE_TEXT, code);
}
