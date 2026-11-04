#!/usr/bin/env node
// fxstrip.mjs — one staged effect as a filmstrip, measured: what an agent
// (or a person) reads to judge an effect and to tell what a change did.
//
//   node fxstrip.mjs frames OUT        # compose OUT/frames/*.png a native stage wrote (OUT/stage.json)
//   node fxstrip.mjs page OUT (--url U | --serve DIR [--query Q]) [--hook fx|bc]
//                    [--span S] [--frames N] [--steps 0,1,4,…] [--size 768x432] [--name N]
//   node fxstrip.mjs keep OUT…         # move OUT's last run to OUT/prev (a native shooter calls it first)
//   node fxstrip.mjs selftest          # the measurements on made-up pixels, no browser
//
// The stage (`fx::stage` in the `fx` crate) fires one effect twice at once:
// in front of a dark card (the frame's left half; light reads here) and a
// light card (the right half; smoke and dust read here), on a clock that
// steps 1/60 s. Step 0 is the plate, the frame before it fires; every number
// is measured against it, per card, on sRGB 0–255 Rec.601 luma:
//
//   add    mean light added over the card (sparks, flash, fire)
//   sub    mean light taken away (smoke, dust, blood on the light card)
//   cover  % of the card that changed at all (any channel moved > 6)
//   peak   99.9th-percentile luma of what changed; clip: % of it at 250+
//   bbox   where it changed, 0–1 of the card; c: the change's centre
//
// and over the run (`envelope`, on add + sub): when it peaks, when it falls
// to half, when the last of it is gone, its widest cover and how far its
// centre travelled. Against the last run (OUT/prev): how much of each frame
// changed, so a tuning step shows what it moved and nothing else. Writes
// OUT/strip.png (the brightest frame large, the envelope, every frame),
// OUT/diff.png and OUT/numbers.json, and prints the table.
//
// A page is driven frame-exactly, never by the wall clock: hook `fx` sets
// `window.__fxStep = k` and waits for `window.__fx = {step: k, held >= 2}`
// (oneshot's `?pose=1&fx=`; `__fx` may also say its game, name, what, span,
// live, seed, knobs and recipes); hook `bc` sets `window.__bcFilm = {step: k}`,
// wakes the app and waits for `__bc.showcase_steps >= k` (Before Colony's
// `?showcase=fx`). A native stage writes its own frames and stage.json
// (Gates' `examples/fx_stage.rs`, `ci/fx.sh`).
//
// Canonical copy: Gates' ci/fxstrip.mjs. oneshot and Before Colony vendor it
// with their fx-sync.sh; fix it in Gates and sync. Needs Node and Playwright
// with its Chromium (CHROMIUM_PATH, else /opt/pw-browsers/chromium if there).
import { createServer } from 'node:http';
import { createRequire } from 'node:module';
import { existsSync, mkdirSync, readFileSync, readdirSync, renameSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { basename, extname, join, resolve } from 'node:path';

const HZ = 60;
const CHANGED = 6; // a channel moved more than this: the pixel changed
const PLATE = { dark: 25, light: 185, tol: 20 };

// ── the measurements: pure functions, run in Node (selftest) and, as source, in the page ──

/** A frame against its plate, per card half (left dark, right light). */
function measure(px, plate, w, h) {
  const half = (x0, x1) => {
    const n = (x1 - x0) * h;
    let add = 0, sub = 0, changed = 0, clip = 0, wsum = 0, cx = 0, cy = 0, r = 0, g = 0, b = 0;
    let bx0 = 1e9, by0 = 1e9, bx1 = -1, by1 = -1;
    const hist = new Uint32Array(256);
    for (let y = 0; y < h; y++) {
      for (let x = x0; x < x1; x++) {
        const i = (y * w + x) * 4;
        const dr = px[i] - plate[i], dg = px[i + 1] - plate[i + 1], db = px[i + 2] - plate[i + 2];
        const l = 0.299 * px[i] + 0.587 * px[i + 1] + 0.114 * px[i + 2];
        const d = 0.299 * dr + 0.587 * dg + 0.114 * db;
        if (d > 0) add += d; else sub -= d;
        if (Math.max(Math.abs(dr), Math.abs(dg), Math.abs(db)) > CHANGED) {
          changed++;
          hist[Math.min(255, Math.round(l))]++;
          if (l >= 250) clip++;
          const wgt = Math.abs(d) + 1e-3;
          wsum += wgt; cx += wgt * (x - x0); cy += wgt * y;
          r += wgt * px[i]; g += wgt * px[i + 1]; b += wgt * px[i + 2];
          if (x < bx0) bx0 = x; if (x > bx1) bx1 = x; if (y < by0) by0 = y; if (y > by1) by1 = y;
        }
      }
    }
    let peak = null;
    if (changed) {
      const want = changed * 0.999;
      let acc = 0;
      for (let v = 0; v < 256; v++) { acc += hist[v]; if (acc >= want) { peak = v; break; } }
    }
    const hw = x1 - x0, r2 = (v) => Math.round(v * 100) / 100, r3 = (v) => Math.round(v * 1000) / 1000;
    return {
      add: r2(add / n), sub: r2(sub / n), cover: r2((changed / n) * 100), peak,
      clip: changed ? r2((clip / changed) * 100) : 0,
      bbox: changed ? [r3(bx0 / hw), r3(by0 / h), r3((bx1 + 1) / hw), r3((by1 + 1) / h)] : null,
      c: changed ? [r3(cx / wsum / hw), r3(cy / wsum / h)] : null,
      hue: changed ? [Math.round(r / wsum), Math.round(g / wsum), Math.round(b / wsum)] : null,
    };
  };
  const mid = w >> 1;
  return { dark: half(0, mid), light: half(mid, w) };
}

/** The plate's mean luma per half, and whether the cards read as the stage's. */
function plateLuma(px, w, h) {
  const mid = w >> 1, m = [0, 0];
  for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) {
    const i = (y * w + x) * 4;
    m[x < mid ? 0 : 1] += 0.299 * px[i] + 0.587 * px[i + 1] + 0.114 * px[i + 2];
  }
  const dark = Math.round((m[0] / (mid * h)) * 10) / 10, light = Math.round((m[1] / ((w - mid) * h)) * 10) / 10;
  return { dark, light, ok: Math.abs(dark - 25) <= 20 && Math.abs(light - 185) <= 20 };
}

/** How much of a frame moved since the last run's frame at the same step. */
function versus(px, prev) {
  let changed = 0, mad = 0;
  const n = px.length / 4;
  for (let i = 0; i < px.length; i += 4) {
    const dr = px[i] - prev[i], dg = px[i + 1] - prev[i + 1], db = px[i + 2] - prev[i + 2];
    if (Math.max(Math.abs(dr), Math.abs(dg), Math.abs(db)) > CHANGED) changed++;
    mad += Math.abs(0.299 * dr + 0.587 * dg + 0.114 * db);
  }
  return { changed: Math.round((changed / n) * 10000) / 100, mad: Math.round((mad / n) * 100) / 100 };
}

/** The run's shape per card from its frames' numbers (step 0 excluded). */
function envelope(frames, card) {
  const fs = frames.filter((f) => f.step > 0);
  const e = fs.map((f) => f[card].add + f[card].sub);
  if (!fs.length) return null;
  let k = 0;
  e.forEach((v, i) => { if (v > e[k]) k = i; });
  const peak = e[k], r3 = (v) => (v == null ? null : Math.round(v * 1000) / 1000);
  const fall = fs.findIndex((f, i) => i > k && e[i] < peak / 2);
  const alive = fs.map((f) => f[card].cover > 0.05);
  const last = alive.lastIndexOf(true), first = alive.indexOf(true);
  const ca = first >= 0 ? fs[first][card].c : null, cb = last >= 0 ? fs[last][card].c : null;
  return {
    peak: Math.round(peak * 100) / 100, peak_t: r3(fs[k].t),
    fall_t: fall >= 0 ? r3(fs[fall].t) : null,
    life_t: last >= 0 ? r3(fs[last].t) : null,
    gone: last >= 0 && last < fs.length - 1,
    area_max: Math.max(...fs.map((f) => f[card].cover)),
    travel: ca && cb ? r3(Math.hypot(cb[0] - ca[0], cb[1] - ca[1])) : null,
  };
}

/** The steps a strip shoots over `span` seconds, `n` frames of it: 0 (the
    plate), then 1 to the end packed toward the start (as `fx::stage::steps`). */
function stripSteps(span, n) {
  const last = Math.max(1, Math.round(Math.max(span, 1 / HZ) * HZ));
  const out = [0];
  for (let i = 0; i < Math.max(1, n); i++) {
    const f = n <= 1 ? 1 : i / (n - 1);
    const s = 1 + Math.round(f * f * (last - 1));
    if (out[out.length - 1] !== s) out.push(s);
  }
  return out;
}

// ── the page side: decode, measure, compose ──────────────────────────────────

/** Runs in the page: every frame decoded and measured against the plate and
    the last run, then the strip and the diff drawn. Returns PNG data URLs. */
async function composeInPage(input) {
  const load = async (url) => {
    const bmp = await createImageBitmap(await (await fetch(url)).blob());
    const c = new OffscreenCanvas(bmp.width, bmp.height);
    const g = c.getContext('2d', { willReadFrequently: true });
    g.drawImage(bmp, 0, 0);
    return { bmp, w: bmp.width, h: bmp.height, px: g.getImageData(0, 0, bmp.width, bmp.height).data };
  };
  const frames = [];
  for (const f of input.frames) frames.push({ ...f, img: await load(f.src), prev: f.prev ? await load(f.prev) : null });
  const plate = frames.find((f) => f.step === 0) || frames[0];
  const { w, h } = plate.img;
  const numbers = frames.map((f) => {
    const m = measure(f.img.px, plate.img.px, w, h);
    const vs = f.prev && f.prev.w === w && f.prev.h === h ? versus(f.img.px, f.prev.px) : null;
    return { step: f.step, t: f.t, live: f.live ?? null, ...m, vs_prev: vs };
  });
  const prevNumbers = input.prevNumbers || null;
  const env = { dark: envelope(numbers, 'dark'), light: envelope(numbers, 'light') };
  const pl = plateLuma(plate.img.px, w, h);

  // Layout: a header, the brightest frame large beside the envelope, then every frame.
  const CW = 384, CH = Math.round((CW * h) / w), COLS = 4, CAP = 34, W = CW * COLS, HEAD = 58;
  // The hero is the frame with the most change, weighted by how bright the
  // brightest of it is: a spark's frame over a dim dust cloud's.
  const energy = numbers.map((n) => (n.step === 0 ? -1
    : (n.dark.add + n.dark.sub + n.light.add + n.light.sub) * (Math.max(n.dark.peak ?? 0, n.light.peak ?? 0, 64) / 255)));
  const hero = energy.indexOf(Math.max(...energy));
  const rows = Math.ceil(frames.length / COLS);
  const FOOT = 86;
  const H = HEAD + 2 * CH + CAP + rows * (CH + CAP) + FOOT;
  const c = new OffscreenCanvas(W, H);
  const g = c.getContext('2d');
  g.fillStyle = '#111'; g.fillRect(0, 0, W, H);
  const text = (s, x, y, col = '#ddd', size = 13, bold = false) => {
    g.fillStyle = col; g.font = `${bold ? 'bold ' : ''}${size}px ui-monospace, monospace`; g.fillText(s, x, y);
  };
  const m = input.meta;
  text(`${m.name} — ${m.what || ''}`, 10, 22, '#fff', 17, true);
  text(`${m.game || ''} · ${frames.length} frames at ${HZ} Hz · seed ${m.seed ?? 0}${m.bloom ? ' · bloom' : ' · no bloom'} · ${w}×${h}`
    + `${m.knobs ? ' · knobs ' + m.knobs : ''}`, 10, 42, '#aaa', 12);
  text(`plate dark ${pl.dark} light ${pl.light} ${pl.ok ? 'ok' : 'NOT the stage’s cards'}`, W - 330, 42, pl.ok ? '#8c8' : '#f66', 12);
  const caption = (n, x, y) => {
    const d = n.dark, l = n.light;
    text(`step ${n.step} · ${n.t.toFixed(3)} s${n.live ? ` · live ${n.live.join('/')}` : ''}${n.vs_prev ? ` · Δprev ${n.vs_prev.changed}%` : ''}`, x + 4, y + 14, '#ccc', 12);
    if (n.step > 0) text(`dark +${d.add} ▢${d.cover}% pk ${d.peak ?? '-'} · light −${l.sub} +${l.add} ▢${l.cover}%`, x + 4, y + 29, '#999', 11);
  };
  // The hero.
  const hf = frames[hero >= 0 ? hero : 0];
  g.drawImage(hf.img.bmp, 0, HEAD, 2 * CW, 2 * CH);
  caption(numbers[hero >= 0 ? hero : 0], 0, HEAD + 2 * CH);
  // The envelope: add + sub per card over time, the last run dashed.
  const X0 = 2 * CW + 40, Y0 = HEAD + 10, PW = 2 * CW - 60, PH = 2 * CH - 40;
  const tmax = Math.max(...numbers.map((n) => n.t), 1 / HZ);
  const series = (ns, card) => ns.filter((n) => n.step > 0).map((n) => [n.t, n[card].add + n[card].sub]);
  const all = [...series(numbers, 'dark'), ...series(numbers, 'light'),
    ...(prevNumbers ? [...series(prevNumbers, 'dark'), ...series(prevNumbers, 'light')] : [])];
  const ymax = Math.max(1, ...all.map((p) => p[1]));
  g.strokeStyle = '#444'; g.lineWidth = 1; g.strokeRect(X0, Y0, PW, PH);
  const plot = (pts, col, dash) => {
    if (!pts.length) return;
    g.strokeStyle = col; g.lineWidth = 2; g.setLineDash(dash ? [6, 5] : []);
    g.beginPath();
    pts.forEach(([t, v], i) => { const x = X0 + (t / tmax) * PW, y = Y0 + PH - (v / ymax) * PH; if (i) g.lineTo(x, y); else g.moveTo(x, y); });
    g.stroke(); g.setLineDash([]);
  };
  if (prevNumbers) { plot(series(prevNumbers, 'dark'), '#a65', true); plot(series(prevNumbers, 'light'), '#58a', true); }
  plot(series(numbers, 'dark'), '#fa6', false); plot(series(numbers, 'light'), '#6af', false);
  text(`change per card (add + sub, luma) over ${tmax.toFixed(2)} s`, X0, Y0 + PH + 18, '#aaa', 12);
  text(`— dark card   — light card${prevNumbers ? '   - - last run' : ''}`, X0, Y0 + PH + 34, '#aaa', 12);
  text(`${ymax.toFixed(1)}`, X0 - 36, Y0 + 12, '#777', 11); text('0', X0 - 14, Y0 + PH, '#777', 11);
  // Every frame.
  const top = HEAD + 2 * CH + CAP;
  frames.forEach((f, i) => {
    const x = (i % COLS) * CW, y = top + Math.floor(i / COLS) * (CH + CAP);
    g.drawImage(f.img.bmp, x, y, CW, CH);
    g.strokeStyle = i === hero ? '#fc6' : '#222'; g.lineWidth = i === hero ? 2 : 1; g.strokeRect(x + 0.5, y + 0.5, CW - 1, CH - 1);
    caption(numbers[i], x, y + CH);
  });
  // The envelope in words.
  const fy = top + rows * (CH + CAP) + 18;
  const line = (card, e) => e
    ? `${card}: peak ${e.peak} at ${e.peak_t} s · half by ${e.fall_t ?? '—'} s · last seen ${e.life_t ?? '—'} s${e.gone ? '' : ' (still there at the end)'} · widest ${e.area_max}% · travel ${e.travel ?? '—'}`
    : `${card}: nothing`;
  text(line('dark ', env.dark), 10, fy, '#fc9', 12);
  text(line('light', env.light), 10, fy + 18, '#9cf', 12);
  const vsAll = numbers.filter((n) => n.vs_prev);
  const vs = vsAll.length ? { frames_changed: vsAll.filter((n) => n.vs_prev.changed > 0.1).length, changed_max: Math.max(...vsAll.map((n) => n.vs_prev.changed)) } : null;
  text(vs ? `vs last run: ${vs.frames_changed} of ${vsAll.length} frames changed, at most ${vs.changed_max}% of a frame` : 'vs last run: no last run', 10, fy + 36, '#ccc', 12);
  const strip = await blobURL(c);

  // The diff: |now − last run| per frame, ×4, black → red → yellow → white.
  let diff = null;
  if (vsAll.length) {
    const DH = HEAD + rows * (CH + CAP);
    const d = new OffscreenCanvas(W, DH);
    const dg = d.getContext('2d');
    dg.fillStyle = '#111'; dg.fillRect(0, 0, W, DH);
    dg.fillStyle = '#fff'; dg.font = 'bold 17px ui-monospace, monospace'; dg.fillText(`${m.name} — now against the last run`, 10, 22);
    dg.fillStyle = '#aaa'; dg.font = '12px ui-monospace, monospace'; dg.fillText('|Δ luma| × 4: black unchanged, red → yellow → white moved', 10, 42);
    const cell = new OffscreenCanvas(w, h), cg = cell.getContext('2d');
    frames.forEach((f, i) => {
      const x = (i % COLS) * CW, y = HEAD + Math.floor(i / COLS) * (CH + CAP);
      if (f.prev && f.prev.w === w && f.prev.h === h) {
        const out = cg.createImageData(w, h);
        for (let p = 0; p < f.img.px.length; p += 4) {
          const a = f.img.px, b = f.prev.px;
          const v = Math.min(255, 4 * Math.abs(0.299 * (a[p] - b[p]) + 0.587 * (a[p + 1] - b[p + 1]) + 0.114 * (a[p + 2] - b[p + 2])));
          out.data[p] = Math.min(255, v * 3); out.data[p + 1] = Math.max(0, Math.min(255, v * 3 - 255)); out.data[p + 2] = Math.max(0, v * 3 - 510); out.data[p + 3] = 255;
        }
        cg.putImageData(out, 0, 0);
        dg.drawImage(cell, x, y, CW, CH);
      }
      dg.fillStyle = '#ccc'; dg.font = '12px ui-monospace, monospace';
      const n = numbers[i];
      dg.fillText(`step ${n.step} · ${n.vs_prev ? `${n.vs_prev.changed}% moved, mean ${n.vs_prev.mad}` : 'no last frame'}`, x + 4, y + CH + 14);
    });
    diff = await blobURL(d);
  }
  return { numbers, env, plate: pl, vs, strip, diff };

  async function blobURL(canvas) {
    const blob = await canvas.convertToBlob({ type: 'image/png' });
    const buf = new Uint8Array(await blob.arrayBuffer());
    let s = '';
    for (let i = 0; i < buf.length; i += 0x8000) s += String.fromCharCode(...buf.subarray(i, i + 0x8000));
    return btoa(s);
  }
}

// ── Node: files, the browser, the table ─────────────────────────────────────

function loadPlaywright() {
  const require = createRequire(import.meta.url);
  for (const n of ['playwright', 'playwright-core', resolve(process.cwd(), 'node_modules/playwright')]) {
    try { return require(n); } catch { /* next */ }
  }
  return null;
}

async function browser(pw, extra = []) {
  const exe = process.env.CHROMIUM_PATH || (existsSync('/opt/pw-browsers/chromium') && statSync('/opt/pw-browsers/chromium').isFile() ? '/opt/pw-browsers/chromium' : undefined);
  return pw.chromium.launch({ executablePath: exe, args: ['--no-sandbox', ...extra] });
}

const dataURL = (p) => `data:image/png;base64,${readFileSync(p).toString('base64')}`;
const pad = (n) => String(n).padStart(4, '0');

/** Move OUT's last run (frames, stage.json, numbers, strip, diff) to OUT/prev. */
function keep(out) {
  if (!existsSync(join(out, 'frames'))) return;
  const prev = join(out, 'prev');
  rmSync(prev, { recursive: true, force: true });
  mkdirSync(prev, { recursive: true });
  for (const f of ['frames', 'stage.json', 'numbers.json', 'strip.png', 'diff.png']) {
    if (existsSync(join(out, f))) renameSync(join(out, f), join(prev, f));
  }
}

/** OUT/frames + OUT/stage.json (+ OUT/prev) → strip.png, diff.png, numbers.json. */
async function compose(out) {
  const meta = JSON.parse(readFileSync(join(out, 'stage.json'), 'utf8'));
  const prevDir = join(out, 'prev', 'frames');
  const prevNumbersPath = join(out, 'prev', 'numbers.json');
  const frames = meta.frames.map((f) => {
    const src = join(out, f.file);
    const prev = join(prevDir, basename(f.file));
    return { step: f.step, t: f.t, live: f.live ?? null, src: dataURL(src), prev: existsSync(prev) ? dataURL(prev) : null };
  });
  let prevNumbers = null;
  if (existsSync(prevNumbersPath)) {
    try { prevNumbers = JSON.parse(readFileSync(prevNumbersPath, 'utf8')).frames; } catch { /* a bad last run is no last run */ }
  }
  const pw = loadPlaywright();
  if (!pw) throw new Error('fxstrip: Playwright is not installed here (npm i -D playwright, or a global one)');
  const br = await browser(pw);
  let r;
  try {
    const page = await br.newPage();
    await page.setContent('<!doctype html><title>fxstrip</title>');
    await page.addScriptTag({ content: [measure, plateLuma, versus, envelope].map(String).join('\n') + `\nconst HZ=${HZ}, CHANGED=${CHANGED};` });
    r = await page.evaluate(composeInPage, { frames, meta, prevNumbers });
  } finally {
    await br.close();
  }
  writeFileSync(join(out, 'strip.png'), Buffer.from(r.strip, 'base64'));
  if (r.diff) writeFileSync(join(out, 'diff.png'), Buffer.from(r.diff, 'base64'));
  else rmSync(join(out, 'diff.png'), { force: true });
  const numbers = {
    name: meta.name, game: meta.game, what: meta.what, at: new Date().toISOString(), hz: HZ, size: meta.size,
    seed: meta.seed ?? 0, knobs: meta.knobs || '', bloom: !!meta.bloom, plate: r.plate,
    frames: r.numbers, envelope: r.env, vs_prev: r.vs, recipes: meta.recipes || null,
  };
  writeFileSync(join(out, 'numbers.json'), JSON.stringify(numbers, null, 1) + '\n');
  print(out, numbers);
  return numbers;
}

function print(out, n) {
  const f1 = (v) => (v == null ? '   -' : String(v).padStart(5));
  console.log(`fxstrip: ${n.name} (${n.game || '?'}), ${n.frames.length} frames, seed ${n.seed}${n.knobs ? `, knobs ${n.knobs}` : ''} → ${join(out, 'strip.png')}`);
  console.log('  step      t |  dark  +add cover  peak  clip | light  −sub  +add cover |  live     Δprev');
  for (const f of n.frames) {
    console.log(`  ${String(f.step).padStart(4)} ${f.t.toFixed(3).padStart(6)} |       ${f1(f.dark.add)} ${f1(f.dark.cover)} ${f1(f.dark.peak)} ${f1(f.dark.clip)} |       ${f1(f.light.sub)} ${f1(f.light.add)} ${f1(f.light.cover)} | ${(f.live ? f.live.join('/') : '-').padStart(5)} ${f.vs_prev ? `${f.vs_prev.changed}%`.padStart(9) : '        -'}`);
  }
  for (const card of ['dark', 'light']) {
    const e = n.envelope[card];
    console.log(e ? `  ${card.padEnd(5)}: peak ${e.peak} at ${e.peak_t} s, half by ${e.fall_t ?? '-'} s, last seen ${e.life_t ?? '-'} s${e.gone ? '' : ' (still there)'}, widest ${e.area_max}%, travel ${e.travel ?? '-'}` : `  ${card}: nothing`);
  }
  console.log(`  plate: dark ${n.plate.dark}, light ${n.plate.light} (${n.plate.ok ? 'ok' : `want ~${PLATE.dark} and ~${PLATE.light}`})`);
  console.log(n.vs_prev ? `  vs last run: ${n.vs_prev.frames_changed} frames changed, at most ${n.vs_prev.changed_max}% of one` : '  vs last run: none');
}

/** A static server for a built page (wasm with its MIME type). */
function serve(dir) {
  const types = { '.html': 'text/html', '.js': 'text/javascript', '.mjs': 'text/javascript', '.wasm': 'application/wasm', '.json': 'application/json', '.css': 'text/css', '.png': 'image/png', '.jpg': 'image/jpeg', '.ktx2': 'image/ktx2', '.glb': 'model/gltf-binary', '.ogg': 'audio/ogg', '.wav': 'audio/wav', '.svg': 'image/svg+xml' };
  const root = resolve(dir);
  const srv = createServer((req, res) => {
    let p = decodeURIComponent(new URL(req.url, 'http://x').pathname);
    if (p.endsWith('/')) p += 'index.html';
    const f = resolve(join(root, p));
    if (!f.startsWith(root) || !existsSync(f) || !statSync(f).isFile()) { res.writeHead(404); res.end(); return; }
    res.writeHead(200, { 'content-type': types[extname(f)] || 'application/octet-stream', 'cache-control': 'no-store' });
    res.end(readFileSync(f));
  });
  return new Promise((ok) => srv.listen(0, '127.0.0.1', () => ok({ srv, url: `http://127.0.0.1:${srv.address().port}` })));
}

async function page(out, o) {
  const pw = loadPlaywright();
  if (!pw) throw new Error('fxstrip: Playwright is not installed here');
  const [w, h] = (o.size || '768x432').split('x').map(Number);
  let steps = o.steps ? o.steps.split(',').map(Number) : null;
  const hook = o.hook || 'fx';
  let srv = null, url = o.url;
  if (!url) {
    if (!o.serve) throw new Error('fxstrip page: --url or --serve DIR');
    srv = await serve(o.serve);
    url = `${srv.url}/${o.query ? `?${o.query}` : ''}`;
  }
  keep(out);
  mkdirSync(join(out, 'frames'), { recursive: true });
  const br = await browser(pw, ['--use-angle=swiftshader', '--enable-unsafe-swiftshader', '--ignore-gpu-blocklist', '--no-proxy-server']);
  const errors = [];
  const timeout = Number(o.timeout || 240) * 1000;
  let info = {};
  const shot = [];
  try {
    const pg = await br.newPage({ viewport: { width: w, height: h } });
    pg.on('pageerror', (e) => errors.push(String(e)));
    pg.on('console', (m) => { if (m.type() === 'error' && /panicked|RuntimeError/.test(m.text())) errors.push(m.text()); });
    if (hook === 'bc') {
      await pg.addInitScript(() => {
        window.__bcFilm = { step: 0 };
        window.__bcWake = () => document.getElementById('bc')?.dispatchEvent(new PointerEvent('pointermove', { bubbles: true, clientX: 2, clientY: 2 }));
        addEventListener('DOMContentLoaded', () => { const s = document.createElement('style'); s.textContent = '#boot{display:none!important}'; document.head.append(s); });
      });
    }
    await pg.goto(url);
    const t0 = Date.now();
    const until = async (expr, wake) => {
      let last = Date.now();
      for (;;) {
        if (errors.length) throw new Error(`fxstrip: the page failed: ${errors[0].slice(0, 600)}`);
        if (await pg.evaluate(expr)) return;
        if (Date.now() - t0 > timeout) throw new Error(`fxstrip: timed out waiting for ${expr}`);
        if (wake && Date.now() - last > 5000) { await pg.evaluate(() => window.__bcWake && window.__bcWake()); last = Date.now(); }
        await pg.waitForTimeout(15);
      }
    };
    if (hook === 'fx') {
      await pg.evaluate(() => { window.__fxStep = 0; });
      await until('window.__fx && window.__fx.ready && window.__fx.step === 0 && window.__fx.held >= 2');
      info = await pg.evaluate(() => ({ ...window.__fx }));
    } else {
      await until('(window.__bc?.showcase_frames ?? 0) >= 10 && (window.__bc?.showcase_steps ?? -1) >= 0', true);
      info = await pg.evaluate(() => ({ ...(window.__fx || {}) }));
    }
    // The span the stage says its effect lasts, unless the caller said.
    steps ||= stripSteps(Number(o.span || info.span || 1), Number(o.frames || 11));
    for (const k of steps) {
      if (hook === 'fx') {
        await pg.evaluate((k) => { window.__fxStep = k; }, k);
        await until(`window.__fx.step === ${k} && window.__fx.held >= 2`);
      } else if (k > 0) {
        await pg.evaluate((k) => { window.__bcFilm = { step: k }; window.__bcWake(); }, k);
        await until(`(window.__bc?.showcase_steps ?? -1) >= ${k}`, true);
      }
      const buf = await pg.screenshot({ type: 'png' });
      writeFileSync(join(out, 'frames', `${pad(k)}.png`), buf);
      const live = hook === 'fx' ? await pg.evaluate(() => window.__fx.live ?? null) : null;
      shot.push({ step: k, t: Math.max(0, k - 1) / HZ, file: `frames/${pad(k)}.png`, live });
    }
  } finally {
    await br.close();
    if (srv) srv.srv.close();
  }
  const name = o.name || info.name || basename(out);
  const meta = { game: o.game || info.game || '', name, what: o.what || info.what || '', hz: HZ, size: [w, h], seed: info.seed ?? 0, knobs: info.knobs || '', bloom: !!info.bloom, steps, frames: shot, recipes: info.recipes || null, url };
  writeFileSync(join(out, 'stage.json'), JSON.stringify(meta, null, 1) + '\n');
  return compose(out);
}

// ── selftest: the measurements on pixels made up here ───────────────────────

function selftest() {
  const w = 40, h = 20, mk = (f) => { const px = new Uint8ClampedArray(w * h * 4); for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) { const i = (y * w + x) * 4; const [r, g, b] = f(x, y); px[i] = r; px[i + 1] = g; px[i + 2] = b; px[i + 3] = 255; } return px; };
  const card = (x) => (x < w / 2 ? 25 : 185);
  const plate = mk((x) => [card(x), card(x), card(x)]);
  const ok = (c, msg) => { if (!c) { console.error(`fxstrip selftest: ${msg}`); process.exit(1); } };
  const same = measure(plate, plate, w, h);
  ok(same.dark.cover === 0 && same.light.cover === 0 && same.dark.add === 0, 'a plate against itself changes nothing');
  // A white spark on the dark card (4×2 at x 4..8, y 4..6), smoke on the light one (a 5×5 grey block).
  const f = mk((x, y) => (x >= 4 && x < 8 && y >= 4 && y < 6 ? [255, 255, 255] : x >= 25 && x < 30 && y >= 10 && y < 15 ? [100, 100, 100] : [card(x), card(x), card(x)]));
  const m = measure(f, plate, w, h);
  ok(m.dark.cover === 2 && m.dark.peak === 255 && m.dark.clip === 100, `the spark is 2% of the dark card at 255: ${JSON.stringify(m.dark)}`);
  ok(m.dark.add === 4.6 && m.dark.sub === 0, 'a spark adds light and takes none');
  ok(m.light.cover === 6.25 && m.light.sub === 5.31 && m.light.add === 0, `smoke takes light from 6.25% of the light card: ${JSON.stringify(m.light)}`);
  ok(JSON.stringify(m.dark.bbox) === '[0.2,0.2,0.4,0.3]', `the spark's box: ${m.dark.bbox}`);
  const pl = plateLuma(plate, w, h);
  ok(pl.ok && pl.dark === 25 && pl.light === 185, 'the plate reads as the stage’s cards');
  ok(versus(f, f).changed === 0 && versus(f, plate).changed === 4.13, `a frame against itself and its plate: ${versus(f, plate).changed}`);
  const e = envelope([{ step: 0, t: 0, ...same }, { step: 1, t: 0, ...m }, { step: 2, t: 0.1, ...same }], 'dark');
  ok(e.peak_t === 0 && e.fall_t === 0.1 && e.life_t === 0 && e.gone, `the envelope: ${JSON.stringify(e)}`);
  const s = stripSteps(1, 11);
  ok(s[0] === 0 && s[1] === 1 && s[s.length - 1] === 60 && s[5] < 20, `the steps: ${s}`);
  console.log('fxstrip selftest: ok');
}

// ── main ─────────────────────────────────────────────────────────────────────

function opts(argv) {
  const o = { _: [] };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a.startsWith('--')) o[a.slice(2)] = argv[i + 1] && !argv[i + 1].startsWith('--') ? argv[++i] : true;
    else o._.push(a);
  }
  return o;
}

const [cmd, ...rest] = process.argv.slice(2);
const o = opts(rest);
try {
  if (cmd === 'frames' && o._[0]) await compose(o._[0]);
  else if (cmd === 'page' && o._[0]) await page(o._[0], o);
  else if (cmd === 'keep' && o._.length) o._.forEach(keep);
  else if (cmd === 'selftest') selftest();
  else {
    console.error('usage: fxstrip.mjs frames OUT | page OUT (--url U | --serve DIR [--query Q]) [--hook fx|bc] [--span S] [--frames N] [--steps …] [--size WxH] | keep OUT… | selftest');
    process.exit(2);
  }
} catch (e) {
  console.error(e.message || e);
  process.exit(1);
}
