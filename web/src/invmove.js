/**
 * The move verb's INBOUND half: reading a `client_move_readout()` word.
 *
 * ## What used to be here, and why it is gone
 *
 * This file was scaffolding for a `crates/` defect. `APPLIED_MOVE` and
 * `STREAM_ERR` were both `1 << 31`, so a landed move and "our own server
 * sent bytes we cannot decode" came back bit-identical, and the only thing
 * on this side of the wall that could tell them apart was the panel's own
 * pending drag. `classifyMoveVerdict(readout, pending)` did that, and said
 * in its own docs what it could not close.
 *
 * The systems lane has since split them (`client-wasm/src/core.rs`):
 * bit 31 of word 0 is `STREAM_ERR` and nothing else, and the move verdict
 * moved to `APPLIED2_MOVE` in a **second applied-flag word**, read through
 * `client_applied2()` after every `client_on_stream`. So the ambiguity is
 * gone at the source, the disambiguation is deleted rather than updated,
 * and the residual that function documented — a genuine decode error
 * arriving while an identical move was in flight reading as that move's
 * verdict — is closed. `ci/ui_smoke.mjs` §L held the tripwire that caught
 * the split; it now holds the invariant that keeps bit 31 unshared.
 *
 * The collision also cost the client more than the verdict: `main.js` took
 * the error branch on `APPLIED_MOVE`, and that branch logs and returns
 * EARLY, so every other flag the same message carried — the inventory diff
 * most of all — went out with it. Reading word 1 at the end of the handler
 * rather than short-circuiting on bit 31 is what puts that back.
 *
 * ## What is left, and why it is not scaffolding
 *
 * The readout is a POSITIONAL PAYLOAD — two slot numbers, a container
 * kind and a refusal reason packed into one `u32` — and CLAUDE.md's trap
 * list is explicit that positional payloads are where the reference
 * ecosystem actually bled: ~27 of Oxide's shipped corrections were the
 * right value in the wrong position, four hooks corrected more than once.
 * A byte-golden is blind to it. So the unpack, and the shapes the sim
 * cannot produce, stay — they were never about the collision.
 */

/** Bit 31 of `client_on_stream`'s return: `core.rs`'s `STREAM_ERR`, and
 *  since the split it means that and only that. The return comes out of
 *  the C ABI as an i32, so it is tested as a mask and never as a
 *  comparison. */
export const STREAM_HIGH_BIT = 0x80000000;

/** `core.rs`'s `APPLIED2_MOVE` — bit 0 of the second applied word. A move
 *  landed or was refused; `client_move_readout()` says which. Word 0 has
 *  no spare bit to announce word 1 with, which is why the caller reads
 *  `client_applied2()` unconditionally rather than on a flag. */
export const APPLIED2_MOVE = 1 << 0;

/** `sim-core/src/inventory.rs:56` — the player's own 30 slots. */
export const CONT_SELF = 0;
/**
 * `sim-core/src/inventory.rs:61` — a death backpack's slots.
 *
 * Named on this side for the first time because `hud.js` now forms
 * ADDRESSES (kind + slot) rather than bare slot numbers, and an address
 * whose container has no name here would be a magic 1. Nothing sends one
 * yet: `main.js`'s move host refuses a non-self end, because the wire's
 * `bag` field wants an id no panel is open on. See `NOW.md`.
 */
export const CONT_BAG = 1;
/**
 * `sim-core/src/inventory.rs:76` — a deployed storage box.
 *
 * Named here for the same reason `CONT_BAG` is: the ceiling below is now
 * this kind, and a bare `2` would be exactly the magic number that comment
 * argues against. Nothing forms one yet — `hud.js` draws only `CONT_SELF`
 * (`hud.invContainers`) and `main.js`'s move host refuses a non-self end.
 * The box carries a packed `box_key(cx, cz, level)` in the wire's handle
 * field rather than an id, so the panel that opens one needs no new bytes.
 */
export const CONT_BOX = 2;
/** `sim-core/src/inventory.rs:81` — the largest container kind the wire's
 *  2-bit field will carry. `encode_action_move` range-checks against it, so
 *  this must EQUAL the sim's ceiling and not merely bound what this client
 *  happens to send today. Written as the alias, exactly as Rust writes it
 *  (`pub const CONT_MAX: u8 = CONT_BOX;`), so a fourth kind moves one line
 *  on each side instead of leaving a literal behind. */
export const CONT_MAX = CONT_BOX;
/** `sim-core/src/inventory.rs:109` — the largest `REFUSE_M_*`. */
export const REFUSE_M_MAX = 7;

/**
 * Unpack a `client_move_readout()` word, or `null` if it is not a verdict
 * this panel can act on.
 *
 * `readout` is `reason << 24 | to slot << 16 | from kind << 8 | from slot`
 * (`bridge.rs`'s `client_move_readout`). Returns `{ reason, from, to }` —
 * `reason` 0 for landed, else an `inventory.rs` `REFUSE_M_*`.
 *
 * Three rejections, each a shape the sim cannot produce for a move this
 * panel sent. They are checked here rather than in the panel because this
 * is arithmetic, and arithmetic is gated in the cheapest layer that can
 * hold it:
 *
 * - **`from kind` is not `CONT_SELF`.** Bag slots and self slots share
 *   numbers, so honouring a container's verdict here would unwind a cell
 *   the server never spoke about. This is the one the trap list is about.
 * - **`reason` is past `REFUSE_M_MAX`.** The sim cannot emit it, so the
 *   word was not a verdict — and the panel would otherwise toast a
 *   refusal string for a reason that does not exist.
 * - **`from === to`.** `hud.dropInvDrag` refuses `to === from`, so no
 *   move this panel sent can come back addressed to one slot. It is also
 *   what a bridge with no core returns (`unwrap_or(0)`): readout 0 unpacks
 *   to (0, 0), and that must never resolve a drag.
 *   NOTE the coupling: this holds while the panel addresses only its own
 *   container. When it grows bag moves the *to kind* has to enter this
 *   readout — the sim can then legitimately move self slot 3 to bag slot
 *   3 — and this check moves with it.
 *   The panel has since grown the addresses (`hud.js`: the drag, the
 *   pending record and the verdict match all carry a kind), so this word
 *   is now the only thing left in the way, and it is a `crates/` change
 *   the ui lane may not make. The one-line request is on `NOW.md`. Until
 *   it lands, the two rejections above are what keeps a container verdict
 *   from resolving a self move — they are load-bearing, not vestigial.
 *
 * The address is checked AGAIN by `hud.invMoveVerdict` against the drag it
 * actually has in flight; this route is not trusted to have matched.
 */
export function moveVerdict(readout) {
  const r = readout >>> 0;
  const reason = r >>> 24;
  const to = (r >>> 16) & 0xff;
  const fromKind = (r >>> 8) & 0xff;
  const from = r & 0xff;
  if (fromKind !== CONT_SELF) return null;
  if (reason > REFUSE_M_MAX) return null;
  if (from === to) return null;
  return { reason, from, to };
}
