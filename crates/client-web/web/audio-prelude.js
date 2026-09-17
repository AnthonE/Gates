/* ── what the worklet scope lacks, supplied before the glue ─────────────────
   `ci/build_web.sh` writes `audio.js` as THIS file, then the `sound-worklet`
   glue, then `audio-processor.js`.

   **An `AudioWorkletGlobalScope` has no `TextDecoder`** — it is
   `[Exposed=(Window,Worker)]`, and a worklet is neither. wasm-bindgen's
   `--target no-modules` glue builds one at top level, unguarded, for the
   strings a Rust panic or a thrown `JsError` carries across. That statement
   threw before `registerProcessor` was reached, so `addModule` resolved, the
   page's `new AudioWorkletNode(ctx, "gates-audio")` refused a name nobody had
   registered, and every browser player heard silence — measured on the
   published page under Chromium 151, 2026-09-17, with `ci/check_worklet.mjs`
   green because node has a `TextDecoder` of its own. That check now hides it.

   A `var`, not a global assignment, and unconditional: at the top of a module
   it binds for the glue below whether or not the browser has one, and in the
   check's `new Function` body it rebinds the parameter that stands in for the
   missing global — so the page and the check run the same decoder.

   Only strings from wasm reach it, and those are errors, so it is never on
   the quantum path. It decodes UTF-8 and throws on a code point out of range,
   which is as much of `fatal: true` as an error message needs. */
var TextDecoder = class {
  decode(bytes) {
    if (!bytes || bytes.length === 0) return "";
    let s = "";
    for (let i = 0; i < bytes.length;) {
      const b = bytes[i++];
      let c;
      if (b < 0x80) c = b;
      else if (b >= 0xf0) c = ((b & 0x07) << 18) | ((bytes[i++] & 0x3f) << 12) | ((bytes[i++] & 0x3f) << 6) | (bytes[i++] & 0x3f);
      else if (b >= 0xe0) c = ((b & 0x0f) << 12) | ((bytes[i++] & 0x3f) << 6) | (bytes[i++] & 0x3f);
      else c = ((b & 0x1f) << 6) | (bytes[i++] & 0x3f);
      s += String.fromCodePoint(c);
    }
    return s;
  }
};
