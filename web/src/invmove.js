/**
 * The move verb's INBOUND half: deciding what bit 31 of `client_on_stream`
 * actually meant.
 *
 * ## Why this file exists
 *
 * `client_on_stream` returns one `u32` that carries two unrelated things:
 * the `APPLIED_*` flag set, and an error signal. `APPLIED_MOVE`
 * (`client-wasm/src/core.rs:122`) is `1 << 31` and `STREAM_ERR`
 * (`client-wasm/src/bridge.rs:64`) is also `1 << 31`, so a landed move and
 * "our own server sent bytes we cannot decode" come back bit-identical —
 * `0x80000000` in both cases, with no other bit set to tell them apart.
 *
 * That is a `crates/` defect and the systems lane owns the fix. It is NOT
 * the one-line constant change it is usually described as: the flag word
 * is FULL — `core.rs:38-122` assigns every bit 0..31 — so `APPLIED_MOVE`
 * has nowhere to move to. The error is what has to leave the word (a
 * second word, or a `client_on_datagram`-shaped return code, or an
 * out-of-band `client_stream_err()`). See `NOW.md`.
 *
 * ## What this file does about it, and what it deliberately does not
 *
 * The panel draws a move optimistically and needs the verdict to confirm
 * or roll it back. Until the collision clears, the only thing on this side
 * of the wall that can disambiguate is the panel's own pending drag: the
 * client knows which move it just sent, and `client_move_readout()` says
 * which move the word is about.
 *
 * So exactly one case changes from "decode error": the high bit, while a
 * drag is in flight, carrying a readout that matches that drag in every
 * field. Everything else still reports corruption exactly as before. That
 * ordering is deliberate — a suppressed error is a worse bug than a missed
 * move, so the suppression is the narrow case and the error is the default.
 *
 * **The residual, stated plainly.** `last_move` is not cleared when a
 * verdict is consumed, and a decode error leaves it untouched
 * (`bridge.rs:288` returns above every refresh). So a genuine decode error
 * arriving while a move identical to the previously resolved one is in
 * flight reads as that move's verdict. It requires a server already
 * emitting undecodable bytes, and the next authoritative `setInventory`
 * corrects the panel. It cannot be closed from JS without a second event
 * encoder in production code, which would be a wire-drift hazard worse
 * than the hole it plugged. It closes for free when the crate fix lands.
 */

/** The collided bit. `client_on_stream`'s return is an i32 out of the C
 *  ABI, so this is tested as a mask and never as a comparison. */
export const STREAM_HIGH_BIT = 0x80000000;

/** `sim-core/src/inventory.rs:56` — the player's own 30 slots. */
export const CONT_SELF = 0;
/** `sim-core/src/inventory.rs:109` — the largest `REFUSE_M_*`. */
export const REFUSE_M_MAX = 7;

/** The word was a verdict on the move this panel drew. Apply it. */
export const VERDICT = "verdict";
/** The word was `STREAM_ERR`, or a move this panel is not waiting for.
 *  Report it — the corruption signal is not worth trading away. */
export const DECODE_ERROR = "error";

/**
 * Classify a `client_move_readout()` word against the drag in flight.
 *
 * `readout` is `reason << 24 | to slot << 16 | from kind << 8 | from slot`
 * (`bridge.rs:904-908`). `pending` is `hud.invPending` — `{from, to}` or
 * null/undefined when no drag is outstanding.
 *
 * Every field is checked, not just the pair: the readout is a positional
 * payload, and CLAUDE.md's trap list is explicit that positional payloads
 * are where the reference ecosystem actually bled — the right value in the
 * wrong position, ~27 shipped corrections. `from kind` in particular must
 * be `CONT_SELF`: a bag's verdict carrying slot numbers this panel also
 * uses would otherwise roll back a cell the server never spoke about.
 *
 * Returns `{ kind, reason, from, to }`. On `DECODE_ERROR` the three
 * numbers are meaningless and the caller must not read them.
 */
export function classifyMoveVerdict(readout, pending) {
  if (!pending) return { kind: DECODE_ERROR, reason: 0, from: -1, to: -1 };
  const r = readout >>> 0;
  const reason = r >>> 24;
  const to = (r >>> 16) & 0xff;
  const fromKind = (r >>> 8) & 0xff;
  const from = r & 0xff;
  if (fromKind !== CONT_SELF) return { kind: DECODE_ERROR, reason: 0, from: -1, to: -1 };
  if (reason > REFUSE_M_MAX) return { kind: DECODE_ERROR, reason: 0, from: -1, to: -1 };
  if (from !== pending.from || to !== pending.to) {
    return { kind: DECODE_ERROR, reason: 0, from: -1, to: -1 };
  }
  return { kind: VERDICT, reason, from, to };
}
