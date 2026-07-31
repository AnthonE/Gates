// Pointer-lock mouse look + WASD, packed to the wire's shapes
// (sim-core input.rs): yaw u16 where 0 faces +Z and increasing turns
// toward +X, pitch u8 with 128 level and 255 straight up (client-side
// convention — the sim never reads pitch yet), move axes i8.

const TWO_PI = Math.PI * 2;
const PITCH_LIMIT = Math.PI / 2 - 0.02;

export class InputTracker {
  constructor(canvas) {
    this.yaw = 0; // radians, free-running
    this.pitch = 0; // radians, +up
    this.keys = { w: false, a: false, s: false, d: false, sprint: false };
    this.locked = false;

    canvas.addEventListener("click", () => {
      if (!this.locked) canvas.requestPointerLock();
    });
    document.addEventListener("pointerlockchange", () => {
      this.locked = document.pointerLockElement === canvas;
    });
    document.addEventListener("mousemove", (e) => {
      if (!this.locked) return;
      this.yaw += e.movementX * 0.0022;
      this.pitch -= e.movementY * 0.0022;
      if (this.pitch > PITCH_LIMIT) this.pitch = PITCH_LIMIT;
      if (this.pitch < -PITCH_LIMIT) this.pitch = -PITCH_LIMIT;
    });
    const onKey = (e, down) => {
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
    });
  }

  yawU16() {
    let t = this.yaw / TWO_PI;
    t -= Math.floor(t);
    return Math.round(t * 65536) & 0xffff;
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
    return this.keys.sprint ? 1 : 0; // BTN_SPRINT
  }
}
