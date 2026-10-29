/* clip.js — the clip page: a player's last half minute drawn by the game
   itself and encoded on their own machine.

   The game page (`app.js`, `openClip`) frames this page and posts it the
   tape the clip key took (`client::render::clip`). This page loads the same
   module the game runs, hands it the tape (`Gates.film`, `render::film`),
   and takes each frame and each slice of the mix as the film draws it. The
   frames go through WebCodecs to H.264, the mix to AAC (Opus where the
   browser has no AAC encoder), and `mp4.js` writes the file. The video goes
   back to the game page as a Blob. Nothing is uploaded: there is no server
   on this path at all.

   A page holds one Bevy app for its life (`Gates.play`'s note), so one film
   per frame: the game page makes a new frame for every clip. */
import { mux } from "./mp4.js";

const q = new URLSearchParams(location.search);
const parent = window.parent !== window ? window.parent : window.opener;
const tell = (msg, transfer) => { if (parent) parent.postMessage(msg, location.origin, transfer || []); };

const MODULES = {
  webgl2: { js: "./client_web.js", wasm: "client_web_bg.wasm" },
  webgpu: { js: "./client_web_gpu.js", wasm: "client_web_gpu_bg.wasm" },
};

/* The film's own hooks into the page (`render/web.rs`): none of them has a
   page to change here, so each one either does nothing or ends the clip. */
window.gatesShown = () => {};
window.gatesPanic = (msg) => fail(String(msg || "the game stopped"));
window.gatesGpuError = (msg, kind) => { if (kind === "lost") fail("the graphics device was lost"); };
/* The film ends its own app when it hands over the last frame (`post`), and
   an ended app says "quit" here; only an end before that is a failure. */
window.gatesLeft = (why) => { if (!finishing) fail(String(why || "the film ended early")); };

let failed = false;
let finishing = false;
function fail(why) {
  if (failed) return;
  failed = true;
  console.error("clip:", why);
  tell({ type: "clip-failed", why });
}

async function load(gfx) {
  const m = MODULES[gfx] || MODULES.webgl2;
  let v = "";
  try {
    const b = await (await fetch("build.json", { cache: "no-cache" })).json();
    const sizes = (b.modules && b.modules[gfx]) || (gfx === "webgl2" ? b : null);
    if (sizes && sizes.sha256) v = "?v=" + String(sizes.sha256).slice(0, 12);
  } catch (err) { /* a checkout with no build.json: unversioned names */ }
  const res = fetch(m.wasm + v);
  const mod = await import(m.js + v);
  await mod.default({ module_or_path: res });
  return mod;
}

/* ── what goes over the picture ─────────────────────────────────────────── */

const FACE = "Gates Sans";
const fonts = Promise.all([
  new FontFace(FACE, "url(fonts/RobotoCondensed-Bold.ttf)", { weight: "700" }).load(),
  new FontFace(FACE, "url(fonts/RobotoCondensed-Regular.ttf)", { weight: "400" }).load(),
]).then((faces) => { for (const f of faces) document.fonts.add(f); }).catch(() => {});

function text(g, s, x, y, px, weight, align, alpha) {
  g.save();
  g.globalAlpha = alpha;
  g.font = `${weight} ${px}px "${FACE}", "Roboto Condensed", "Arial Narrow", sans-serif`;
  g.textAlign = align;
  g.textBaseline = "middle";
  g.lineWidth = Math.max(2, px * 0.12);
  g.strokeStyle = "rgba(0,0,0,.55)";
  g.lineJoin = "round";
  g.strokeText(s, x, y);
  g.fillStyle = "#f2ede2";
  g.fillText(s, x, y);
  g.restore();
}

/* The player's own line, across the top for the first seconds: what a post
   says over its first frame. */
function hook(g, w, h, line, t) {
  if (!line || t > 3.2) return;
  const a = Math.min(1, t / 0.25, (3.2 - t) / 0.3);
  const px = Math.round(Math.min(w, h) * 0.075);
  text(g, line, w / 2, h * (h > w ? 0.17 : 0.14), px, 700, "center", a);
}

/* The tag in a corner while the clip plays: where it was made. */
function tag(g, w, h) {
  const px = Math.round(Math.min(w, h) * 0.032);
  text(g, "GATES", px * 1.1, h - px * 2.2, px, 700, "left", 0.72);
}

/* The end card: the last frame dimmed, the wordmark, where to play. */
function card(g, w, h, last, link, u) {
  if (last) g.drawImage(last, 0, 0, w, h);
  g.fillStyle = `rgba(14,12,10,${0.35 + 0.45 * Math.min(1, u * 3)})`;
  g.fillRect(0, 0, w, h);
  const a = Math.min(1, u * 4);
  const big = Math.round(Math.min(w, h) * 0.2);
  text(g, "GATES", w / 2, h * 0.44, big, 700, "center", a);
  text(g, "Play free in your browser", w / 2, h * 0.44 + big * 0.85, Math.round(big * 0.28), 400, "center", a);
  text(g, link, w / 2, h * 0.44 + big * 1.35, Math.round(big * 0.3), 700, "center", a);
}

/* ── the encoders ───────────────────────────────────────────────────────── */

const AVC = ["avc1.640028", "avc1.4d0028", "avc1.640032", "avc1.42e032"];

async function videoConfig(w, h, fps) {
  if (!("VideoEncoder" in window)) throw new Error("this browser cannot encode video here (no WebCodecs); try Chrome, Edge or Safari");
  for (const codec of AVC) {
    const c = { codec, width: w, height: h, framerate: fps, bitrate: w * h > 1.5e6 ? 10e6 : 6e6, avc: { format: "avc" } };
    try {
      const r = await VideoEncoder.isConfigSupported(c);
      if (r.supported) return r.config;
    } catch (err) { /* the next one */ }
  }
  throw new Error("this browser cannot encode H.264 video; try Chrome, Edge or Safari");
}

async function audioConfig() {
  if (!("AudioEncoder" in window)) return null;
  for (const c of [
    { codec: "mp4a.40.2", sampleRate: 48000, numberOfChannels: 2, bitrate: 160000 },
    { codec: "opus", sampleRate: 48000, numberOfChannels: 2, bitrate: 128000 },
  ]) {
    try {
      const r = await AudioEncoder.isConfigSupported(c);
      if (r.supported) return r.config;
    } catch (err) { /* the next one */ }
  }
  return null;
}

const bytes = (d) => {
  if (!d) return null;
  if (d instanceof ArrayBuffer) return new Uint8Array(d);
  return new Uint8Array(d.buffer.slice(d.byteOffset, d.byteOffset + d.byteLength));
};

/* ── the film's sink ────────────────────────────────────────────────────── */

function sink(opts) {
  let w = 0, h = 0, fps = 30, frames = 0;
  let canvas = null, g = null, scratch = null, sg = null, last = null;
  let encoder = null, avcC = null;
  const samples = [];
  const pcm = [];
  let pcmLen = 0;
  let encErr = null;
  let ready = null;

  function open(ow, oh, rate) {
    w = ow; h = oh; fps = rate;
    canvas = new OffscreenCanvas(w, h);
    g = canvas.getContext("2d", { alpha: false });
    ready = videoConfig(w, h, fps).then((config) => {
      encoder = new VideoEncoder({
        output: (chunk, meta) => {
          if (meta && meta.decoderConfig && meta.decoderConfig.description) avcC = bytes(meta.decoderConfig.description);
          const data = new Uint8Array(chunk.byteLength);
          chunk.copyTo(data);
          samples.push({ data, pts: chunk.timestamp, key: chunk.type === "key" });
        },
        error: (e) => { encErr = e; },
      });
      encoder.configure(config);
    });
    ready.catch((e) => fail(e.message || String(e)));
  }

  function emit(draw) {
    const t = frames / fps;
    draw(g, t);
    const vf = new VideoFrame(canvas, { timestamp: Math.round((frames * 1e6) / fps), duration: Math.round(1e6 / fps) });
    encoder.encode(vf, { keyFrame: frames % (fps * 2) === 0 });
    vf.close();
    frames++;
  }

  /* Frames arrive faster than an encoder may take them only on a fast box;
     they are held here and fed in order as it drains. */
  const queue = [];
  let pumping = false;
  async function pump() {
    if (pumping) return;
    pumping = true;
    try {
      await ready;
      while (queue.length) {
        if (encErr) throw encErr;
        if (encoder.encodeQueueSize > 8) { await new Promise((r) => setTimeout(r, 4)); continue; }
        const job = queue.shift();
        emit(job);
      }
    } catch (e) {
      fail(e.message || String(e));
    } finally {
      pumping = false;
    }
  }

  return {
    shot(i, name, dw, dh, ow, oh, rate) {
      if (!canvas) open(ow, oh, rate);
      if (dw !== w || dh !== h) {
        scratch = new OffscreenCanvas(dw, dh);
        sg = scratch.getContext("2d");
      } else {
        scratch = null;
      }
    },
    frame(i, n, rgba, dw, dh) {
      const img = new ImageData(new Uint8ClampedArray(rgba.buffer, rgba.byteOffset, rgba.byteLength), dw, dh);
      queue.push((g, t) => {
        if (scratch) {
          sg.putImageData(img, 0, 0);
          g.drawImage(scratch, 0, 0, w, h);
        } else {
          g.putImageData(img, 0, 0);
        }
        if (!last) last = new OffscreenCanvas(w, h);
        last.getContext("2d").drawImage(canvas, 0, 0);
        if (opts.tag) tag(g, w, h);
        hook(g, w, h, opts.line, t);
      });
      pump();
    },
    audio(i, slice) {
      pcm.push(slice);
      pcmLen += slice.length;
    },
    shotEnd() {},
    progress(f) { tell({ type: "clip-progress", f }); },
    async done() {
      finishing = true;
      try {
        await fonts;
        if (opts.tag) {
          const n = Math.round(fps * 2.4);
          for (let k = 0; k < n; k++) {
            const u = k / n;
            queue.push((g) => card(g, w, h, last, opts.link, u));
          }
        }
        await pump();
        while (queue.length || pumping) await new Promise((r) => setTimeout(r, 10));
        await encoder.flush();
        if (encErr) throw encErr;
        tell({ type: "clip-progress", f: 1, encoding: true });
        const audio = await encodeAudio(pcm, pcmLen, frames / fps);
        const blob = mux({ video: { samples, width: w, height: h, fps, avcC }, audio });
        tell({ type: "clip-done", blob, seconds: frames / fps, width: w, height: h });
      } catch (e) {
        fail(e.message || String(e));
      }
    },
    fail(why) { fail(String(why)); },
  };
}

/* The game's own mix, laid end to end, run out to the video's length with a
   fade over the end card, and encoded. */
async function encodeAudio(chunks, len, seconds) {
  const config = await audioConfig();
  if (!config) return null;
  const rate = 48000, ch = 2;
  const total = Math.round(seconds * rate) * ch;
  const all = new Float32Array(total);
  let at = 0;
  for (const c of chunks) {
    if (at >= total) break;
    all.set(c.subarray(0, Math.min(c.length, total - at)), at);
    at += c.length;
  }
  const fadeFrom = Math.max(0, Math.min(at, total) - rate * ch * 0.8);
  for (let i = fadeFrom; i < total; i++) all[i] *= Math.max(0, 1 - (i - fadeFrom) / (rate * ch * 0.8));

  const out = [];
  let desc = null;
  let err = null;
  const encoder = new AudioEncoder({
    output: (chunk, meta) => {
      if (meta && meta.decoderConfig && meta.decoderConfig.description) desc = bytes(meta.decoderConfig.description);
      const data = new Uint8Array(chunk.byteLength);
      chunk.copyTo(data);
      out.push({ data, pts: chunk.timestamp, key: true });
    },
    error: (e) => { err = e; },
  });
  encoder.configure(config);
  const block = 4800;
  for (let f = 0; f * ch < total; f += block) {
    const n = Math.min(block, total / ch - f);
    encoder.encode(new AudioData({
      format: "f32", sampleRate: rate, numberOfChannels: ch, numberOfFrames: n,
      timestamp: Math.round((f * 1e6) / rate), data: all.subarray(f * ch, (f + n) * ch),
    }));
  }
  await encoder.flush();
  if (err) throw err;
  const opus = config.codec === "opus";
  return {
    samples: out,
    codec: opus ? "opus" : "aac",
    /* AAC-LC, 48 kHz, stereo, when the encoder named no config of its own. */
    config: desc || (opus ? null : new Uint8Array([0x11, 0x90])),
    rate, channels: ch,
    frame: opus ? 960 : 1024,
    bitrate: config.bitrate || 128000,
  };
}

/* ── the handoff ────────────────────────────────────────────────────────── */

let started = false;
window.addEventListener("message", async (ev) => {
  if (ev.origin !== location.origin || !ev.data || ev.data.type !== "clip-tape" || started) return;
  started = true;
  const { tape, style, line, link, tag: withTag } = ev.data;
  try {
    const mod = await load(q.get("gfx") || "webgl2");
    await fonts;
    const s = sink({ line: String(line || "").slice(0, 60), link: link || "elopros.com", tag: withTag !== false });
    /* Does not return: Bevy's loop owns the page from here (`Gates.film`). */
    mod.Gates.film(new Uint8Array(tape), style || "pov", "#film", s);
  } catch (e) {
    fail((e && e.message) || String(e));
  }
});
tell({ type: "clip-ready" });
