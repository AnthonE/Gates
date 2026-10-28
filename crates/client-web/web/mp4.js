/* mp4.js — the smallest MP4 writer a clip needs: one H.264 track and one
   AAC (or Opus) track, every sample already encoded and in memory, written
   with the index first (`moov` before `mdat`) so a phone starts playing it
   before it has the whole file, which is what an upload's preview does.

   The encoders are the browser's (WebCodecs, `clip.js`); this only lays out
   the boxes ISO/IEC 14496-12 and -15 name. One chunk per track, so each
   track's samples sit together in `mdat`, video first. */

const enc = new TextEncoder();

function box(type, ...parts) {
  let n = 8;
  for (const p of parts) n += p.byteLength;
  const out = new Uint8Array(n);
  const dv = new DataView(out.buffer);
  dv.setUint32(0, n);
  out.set(enc.encode(type), 4);
  let at = 8;
  for (const p of parts) {
    out.set(p instanceof Uint8Array ? p : new Uint8Array(p.buffer || p), at);
    at += p.byteLength;
  }
  return out;
}

/* A full box: version and flags ahead of the body. */
const full = (type, version, flags, ...parts) =>
  box(type, u8(version, flags >> 16, (flags >> 8) & 255, flags & 255), ...parts);

function u8(...v) { return new Uint8Array(v); }
function u16(...v) { const b = new Uint8Array(v.length * 2); const d = new DataView(b.buffer); v.forEach((x, i) => d.setUint16(i * 2, x)); return b; }
function u32(...v) { const b = new Uint8Array(v.length * 4); const d = new DataView(b.buffer); v.forEach((x, i) => d.setUint32(i * 4, x >>> 0)); return b; }
function i32(...v) { const b = new Uint8Array(v.length * 4); const d = new DataView(b.buffer); v.forEach((x, i) => d.setInt32(i * 4, x)); return b; }
function cat(...parts) {
  let n = 0;
  for (const p of parts) n += p.byteLength;
  const out = new Uint8Array(n);
  let at = 0;
  for (const p of parts) { out.set(p, at); at += p.byteLength; }
  return out;
}

const MATRIX = u32(0x10000, 0, 0, 0, 0x10000, 0, 0, 0, 0x40000000);

/* An MPEG-4 descriptor: tag, then its length in the four-byte form every
   reader accepts. */
function desc(tag, body) {
  const n = body.byteLength;
  return cat(u8(tag, 0x80 | ((n >> 21) & 127), 0x80 | ((n >> 14) & 127), 0x80 | ((n >> 7) & 127), n & 127), body);
}

function esds(asc, bitrate) {
  const dsi = desc(5, asc);
  const dcd = desc(4, cat(u8(0x40, 0x15, 0, 0, 0), u32(bitrate, bitrate), dsi));
  const es = desc(3, cat(u16(2), u8(0), dcd, desc(6, u8(2))));
  return full("esds", 0, 0, es);
}

/* `OpusHead` (little-endian, what WebCodecs hands back) as `dOps`
   (big-endian, what MP4 carries). */
function dOps(head, channels) {
  let ch = channels, skip = 312, rate = 48000, gain = 0, family = 0;
  if (head && head.byteLength >= 19) {
    const d = new DataView(head.buffer, head.byteOffset, head.byteLength);
    ch = d.getUint8(9);
    skip = d.getUint16(10, true);
    rate = d.getUint32(12, true);
    gain = d.getInt16(16, true);
    family = d.getUint8(18);
  }
  const g = new Uint8Array(2);
  new DataView(g.buffer).setInt16(0, gain);
  return box("dOps", u8(0, ch), u16(skip), u32(rate), g, u8(family));
}

function stbl(entry, t) {
  const n = t.sizes.length;
  const parts = [
    full("stsd", 0, 0, u32(1), entry),
    sttsBox(t.deltas),
  ];
  if (t.ctts) parts.push(full("ctts", 1, 0, u32(t.ctts.length), ...t.ctts.map(([c, o]) => cat(u32(c), i32(o)))));
  if (t.sync) parts.push(full("stss", 0, 0, u32(t.sync.length), u32(...t.sync)));
  parts.push(
    full("stsc", 0, 0, u32(1), u32(1, n, 1)),
    full("stsz", 0, 0, u32(0, n), u32(...t.sizes)),
    full("stco", 0, 0, u32(1), u32(t.offset)),
  );
  return box("stbl", ...parts);
}

/* Runs of equal deltas, the time-to-sample table's own compression. */
function sttsBox(deltas) {
  const runs = [];
  for (const d of deltas) {
    const last = runs[runs.length - 1];
    if (last && last[1] === d) last[0]++;
    else runs.push([1, d]);
  }
  return full("stts", 0, 0, u32(runs.length), ...runs.map(([c, d]) => u32(c, d)));
}

function trak(id, kind, t, movieMs, w, h) {
  const video = kind === "vide";
  const tkhd = full("tkhd", 0, 3,
    u32(0, 0, id, 0, movieMs), u32(0, 0), u16(0, 0, video ? 0 : 0x0100, 0), MATRIX,
    u32(video ? w << 16 : 0, video ? h << 16 : 0));
  const mdhd = full("mdhd", 0, 0, u32(0, 0, t.timescale, t.duration), u16(0x55c4, 0));
  const hdlr = full("hdlr", 0, 0, u32(0), enc.encode(kind), u32(0, 0, 0),
    enc.encode(video ? "VideoHandler\0" : "SoundHandler\0"));
  const head = video ? full("vmhd", 0, 1, u16(0, 0, 0, 0)) : full("smhd", 0, 0, u16(0, 0));
  const dinf = box("dinf", full("dref", 0, 0, u32(1), full("url ", 0, 1)));
  const minf = box("minf", head, dinf, stbl(t.entry, t));
  return box("trak", tkhd, box("mdia", mdhd, hdlr, minf));
}

/* Where each track's samples start and how long each one is, in its own
   timescale. `samples` are `{data, pts, key}` in decode order, `pts` in µs. */
function table(samples, timescale, step) {
  const sizes = samples.map((s) => s.data.byteLength);
  const deltas = samples.map(() => step);
  const sync = [];
  samples.forEach((s, i) => { if (s.key) sync.push(i + 1); });
  /* An encoder that reorders (B-frames) presents out of decode order: the
     offsets say by how much. Most never do, and then there is no table. */
  let ctts = null;
  const toTs = (us) => Math.round((us * timescale) / 1e6);
  if (samples.some((s, i) => toTs(s.pts) !== i * step)) {
    ctts = [];
    samples.forEach((s, i) => {
      const o = toTs(s.pts) - i * step;
      const last = ctts[ctts.length - 1];
      if (last && last[1] === o) last[0]++;
      else ctts.push([1, o]);
    });
  }
  return { sizes, deltas, sync, ctts, timescale, duration: samples.length * step };
}

/* The file. `video`: {samples, width, height, fps, avcC}. `audio`:
   {samples, codec: "aac"|"opus", config, rate, channels, frame, bitrate}
   or null. Returns a Blob. */
export function mux({ video, audio }) {
  const vt = table(video.samples, video.fps * 1000, 1000);
  vt.entry = box("avc1",
    u8(0, 0, 0, 0, 0, 0), u16(1), u16(0, 0), u32(0, 0, 0),
    u16(video.width, video.height), u32(0x480000, 0x480000), u32(0), u16(1),
    new Uint8Array(32), u16(0x18), u16(0xffff),
    box("avcC", new Uint8Array(video.avcC)));
  vt.sync = vt.sync.length === vt.sizes.length ? null : vt.sync;
  const movieMs = Math.round((vt.duration / vt.timescale) * 1000);

  let at = null;
  if (audio && audio.samples.length) {
    at = table(audio.samples, audio.rate, audio.frame);
    at.sync = null;
    at.ctts = null;
    const sampleEntry = (type, extra) => box(type,
      u8(0, 0, 0, 0, 0, 0), u16(1), u32(0, 0), u16(audio.channels, 16), u16(0, 0),
      u32(audio.rate << 16 >>> 0), extra);
    at.entry = audio.codec === "opus"
      ? sampleEntry("Opus", dOps(audio.config, audio.channels))
      : sampleEntry("mp4a", esds(audio.config, audio.bitrate));
  }

  const ftyp = box("ftyp", enc.encode("isom"), u32(512), enc.encode("isomiso2avc1mp41"));
  const vBytes = video.samples.reduce((n, s) => n + s.data.byteLength, 0);
  const aBytes = at ? audio.samples.reduce((n, s) => n + s.data.byteLength, 0) : 0;
  /* The index names absolute offsets into `mdat`, and its own size moves
     them: build it once to measure, then again with the offsets known. */
  const moov = () => {
    const mvhd = full("mvhd", 0, 0, u32(0, 0, 1000, movieMs, 0x10000), u16(0x0100, 0), u32(0, 0),
      MATRIX, u32(0, 0, 0, 0, 0, 0), u32(at ? 3 : 2));
    const traks = [trak(1, "vide", vt, movieMs, video.width, video.height)];
    if (at) traks.push(trak(2, "soun", at, Math.round((at.duration / at.timescale) * 1000)));
    return box("moov", mvhd, ...traks);
  };
  vt.offset = 0;
  if (at) at.offset = 0;
  const head = ftyp.byteLength + moov().byteLength + 8;
  vt.offset = head;
  if (at) at.offset = head + vBytes;
  const index = moov();
  const mdatHead = u32(8 + vBytes + aBytes);
  return new Blob([
    ftyp, index, mdatHead, enc.encode("mdat"),
    ...video.samples.map((s) => s.data),
    ...(at ? audio.samples.map((s) => s.data) : []),
  ], { type: "video/mp4" });
}
