// Pointer-lock mouse look + WASD, packed to the wire's shapes
// (sim-core input.rs): yaw u16 where 0 faces +Z and increasing turns
// toward +X, pitch u8 with 128 level and 255 straight up (client-side
// convention — the sim never reads pitch yet), move axes i8.

const TWO_PI = Math.PI * 2;
const PITCH_LIMIT = Math.PI / 2 - 0.02;

/**
 * How many bearings the SIM can actually face: `yaw_lut.rs` is 256 entries and
 * `yaw_dir` indexes it `yaw >> 8`, so the top eight bits of the wire yaw are
 * the whole of the direction. The low eight are carried and never read.
 */
export const YAW_BEARINGS = 256;

/**
 * The 256 bearings, as f32, interleaved `[dx, dz, dx, dz, …]`.
 *
 * This is `crates/sim-core/src/yaw_lut.rs`'s `YAW_LUT_BITS` re-derived rather
 * than re-typed, and it is bit-identical to it: `ci/ui_smoke.mjs` reads all 256
 * pairs out of that file and compares them to this array as f32 bit patterns,
 * with no tolerance. That holds because the Rust table is `ci/gen_yaw_lut.py`'s
 * `sin`/`cos` of the same angles rounded to f32, and `Float32Array` performs the
 * identical rounding — measured over all 512 values, worst difference 0 ulp.
 *
 * `sim-core` may not call trig at runtime (CLAUDE.md wall 1) and so ships the
 * table as bits; this client has no such wall, so it computes the same numbers
 * at import. Built once — a `Float32Array` at module scope, never in the RAF
 * loop — because the client is a hot path too.
 */
const YAW_DIR = new Float32Array(YAW_BEARINGS * 2);
for (let i = 0; i < YAW_BEARINGS; i++) {
  const th = i * (TWO_PI / YAW_BEARINGS);
  // Index 0 faces +Z and increasing index rotates toward +X — the convention
  // this file's header states, and the one `yaw_lut.rs` states in its own.
  YAW_DIR[i * 2] = Math.sin(th);
  YAW_DIR[i * 2 + 1] = Math.cos(th);
}

/**
 * The look direction the SIM will use for a wire yaw — `yaw_lut::yaw_dir`, in
 * JS, on the same 256-bearing grid.
 *
 * Why this exists at all: CLAUDE.md's "quantize both sides or prediction drifts
 * by rounding — the server sims on the values it transmits." A client that
 * judges an aim cone on its own free-running `this.yaw` is testing a bearing
 * the arm will never swing on. The bearings are 360/256 = 1.40625 deg apart, so
 * the free-running value sits up to HALF that — 0.703 deg — off the one
 * `gather::swing` runs its 30 deg cone with, and a target inside the cone on one
 * side of that seam is outside it on the other. That is a prompt naming a tree
 * the swing misses, and nothing on the wire can see the disagreement: the client
 * sends a swing as a button bit and the sim picks the node alone.
 *
 * What it does NOT close, said plainly: the prompt is recomputed off the HUD's
 * slow timer and the swing lands on whatever yaw the next input frame carries,
 * so the two can still differ because TIME passed. This closes the quantization
 * seam, not the temporal one.
 *
 * @param yawU16 the wire yaw, 0..65535 — `yawU16()` below, or a value read back
 *               off the wire; only bits 8..15 are read, exactly as the sim does.
 * @param out    `{fx, fz}`, mutated in place and returned. No allocation: the
 *               callers reach this from the HUD timer and the frame path.
 */
export function yawDir(yawU16, out) {
  const i = (yawU16 >>> 8) & (YAW_BEARINGS - 1);
  out.fx = YAW_DIR[i * 2];
  out.fz = YAW_DIR[i * 2 + 1];
  return out;
}

export class InputTracker {
  constructor(canvas) {
    this.yaw = 0; // radians, free-running
    this.pitch = 0; // radians, +up
    this.sel = 0; // selected hotbar slot 0..5 (keys 1–6)
    this.keys = {
      w: false,
      a: false,
      s: false,
      d: false,
      sprint: false,
      primary: false,
    };
    this.locked = false;

    canvas.addEventListener("click", () => {
      if (!this.locked) canvas.requestPointerLock();
    });
    document.addEventListener("pointerlockchange", () => {
      this.locked = document.pointerLockElement === canvas;
      if (!this.locked) this.keys.primary = false;
    });
    // The swing/use button (sim BTN_PRIMARY). The lock-acquiring click
    // never swings: `locked` is still false when it fires.
    document.addEventListener("mousedown", (e) => {
      if (this.locked && e.button === 0) this.keys.primary = true;
    });
    document.addEventListener("mouseup", (e) => {
      if (e.button === 0) this.keys.primary = false;
    });
    document.addEventListener("mousemove", (e) => {
      if (!this.locked) return;
      this.yaw += e.movementX * 0.0022;
      this.pitch -= e.movementY * 0.0022;
      if (this.pitch > PITCH_LIMIT) this.pitch = PITCH_LIMIT;
      if (this.pitch < -PITCH_LIMIT) this.pitch = -PITCH_LIMIT;
    });
    const onKey = (e, down) => {
      if (down && e.code.startsWith("Digit")) {
        const n = e.code.charCodeAt(5) - 49; // "Digit1" → 0 … "Digit6" → 5
        if (n >= 0 && n <= 5) {
          this.sel = n;
          e.preventDefault();
          return;
        }
      }
      switch (e.code) {
        case "KeyW":
          this.keys.w = down;
          break;
        case "KeyA":
          this.keys.a = down;
          break;
        case "KeyS":
          this.keys.s = down;
          break;
        case "KeyD":
          this.keys.d = down;
          break;
        case "ShiftLeft":
        case "ShiftRight":
          this.keys.sprint = down;
          break;
        default:
          return;
      }
      e.preventDefault();
    };
    document.addEventListener("keydown", (e) => onKey(e, true));
    document.addEventListener("keyup", (e) => onKey(e, false));
    window.addEventListener("blur", () => {
      this.keys.w = this.keys.a = this.keys.s = this.keys.d = false;
      this.keys.sprint = false;
      this.keys.primary = false;
    });
  }

  // Aim from outside the mouse. Headless Chromium grants pointer lock but
  // produces no movementX/movementY, so a capture harness can walk and
  // never look; this is the only way to park the camera at a fixed
  // vantage. Same clamps the mouse path applies, so nothing downstream can
  // tell the two apart — and no allocation, since the caller reaches it
  // off the 250 ms timer's debug object, never the RAF loop.
  setView(yaw, pitch) {
    if (!Number.isFinite(yaw) || !Number.isFinite(pitch)) return false;
    this.yaw = yaw;
    this.pitch =
      pitch > PITCH_LIMIT ? PITCH_LIMIT : pitch < -PITCH_LIMIT ? -PITCH_LIMIT : pitch;
    return true;
  }

  yawU16() {
    let t = this.yaw / TWO_PI;
    t -= Math.floor(t);
    return Math.round(t * 65536) & 0xffff;
  }

  /**
   * The look direction to resolve a pick with — this tracker's own yaw, put
   * through the wire quantum first.
   *
   * Every aim resolver goes through here rather than through `Math.sin(yaw)`,
   * so the cone a prompt is computed with is the cone the arm swings. See
   * `yawDir` above for the seam and for the half of it this does not close.
   */
  aimDir(out) {
    return yawDir(this.yawU16(), out);
  }

  pitchU8() {
    const v = Math.round((this.pitch / Math.PI + 0.5) * 255);
    return v < 0 ? 0 : v > 255 ? 255 : v;
  }

  moveX() {
    return (this.keys.d ? 127 : 0) + (this.keys.a ? -127 : 0);
  }

  moveZ() {
    return (this.keys.w ? 127 : 0) + (this.keys.s ? -127 : 0);
  }

  buttons() {
    // BTN_SPRINT | BTN_PRIMARY (sim-core input.rs bit layout).
    return (this.keys.sprint ? 1 : 0) | (this.keys.primary ? 4 : 0);
  }
}
