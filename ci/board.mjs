#!/usr/bin/env node
// board.mjs — the reference board: what a game should look like, shot and
// compared on one sheet, with the operator's word on each view.
//
//   node board.mjs [board.json] look   [--views a,b]   # shoot, then the sheet: the loop's one step
//   node board.mjs [board.json] shoot  [--views a,b]   # run each view's own shooter, keep the PNGs as a pass
//   node board.mjs [board.json] sheet  [--pass STAMP]  # refs | now | last pass, measured, on one PNG
//   node board.mjs [board.json] mark   VIEW right|wrong|note "what the operator said" [--by NAME]
//   node board.mjs [board.json] ref    VIEW FILE --take "..." [--ignore "..."] [--source URL] [--licence CC0] [--private]
//   node board.mjs [board.json] status
//
// The board file (default board/board.json) names the views. A view is a
// fixed picture of the game (a seed, a camera, an hour), with an id and
// optionally a name a person reads, the command that
// shoots it with no GPU (the game's own: a headless page on SwiftShader, a
// native client on lavapipe), the PNG that command writes, and its
// references, each with what to take from it and what to ignore. A view may
// also name the page that draws it in a browser (`page`: its src, the window
// object it exposes, and the name to shoot), for a board shown in the page. A pass is
// one run of every shooter, kept under board/passes/<stamp>/ (not committed).
// Marks are board/marks.jsonl, one line each, append-only: the operator's
// verdict on a view, which is the bar. Numbers keep a floor under it, never
// replace it: Rec.601 luma over the frame, the sky band (top 25% of rows) and
// the near band (bottom 35%), and chroma (max − min channel), each 0–255,
// measured on both sides by the same code in the same run.
//
// A reference that is not ours or CC0 never enters the repo: mark it
// `private`, keep the file in the directory $ELO_REF_DIR names, and only its
// name and measurements are written down. A reference is a target for the
// look, never an asset: nothing is traced or copied from it.
//
// Canonical copy: the Elo Pros agent kit, skills/elo-look/scripts/board.mjs. A game
// repo vendors it as tools/board.mjs; fix it upstream and copy it back.
// No dependencies but Node; the sheet needs Playwright (or playwright-core)
// and its Chromium, which every Elo Pros game's gates already load.
import { spawnSync } from 'node:child_process';
import { appendFileSync, copyFileSync, existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { basename, dirname, extname, join, relative, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

export const VERDICTS = ['right', 'wrong', 'note'];
const KEEP_PASSES = 20;
const SHEET_COL = 360;

// ── the board file ───────────────────────────────────────────────────────────

/** Read and check a board. Throws with the reason a person can act on. */
export function readBoard(path) {
  const board = JSON.parse(readFileSync(path, 'utf8'));
  return checkBoard(board, path);
}

export function checkBoard(board, path = 'board.json') {
  const bad = (why) => { throw new Error(`${path}: ${why}`); };
  if (!board || typeof board !== 'object') bad('not an object');
  if (!Array.isArray(board.views) || !board.views.length) bad('no views');
  const ids = new Set();
  for (const v of board.views) {
    if (!v || typeof v.id !== 'string' || !/^[a-z0-9][a-z0-9@~_-]{0,63}$/.test(v.id)) bad(`a view needs an id of a-z 0-9 @ ~ _ - (got ${JSON.stringify(v && v.id)})`);
    if (ids.has(v.id)) bad(`view ${v.id} twice`);
    ids.add(v.id);
    if (!Array.isArray(v.shoot) || !v.shoot.length || !v.shoot.every((a) => typeof a === 'string')) bad(`view ${v.id}: shoot is the command as an array of strings`);
    if (typeof v.png !== 'string' || !v.png.endsWith('.png')) bad(`view ${v.id}: png is the file its shooter writes`);
    // optional: the page that draws this view in a browser, for a board shown in the page
    // (window[api].ready, then window[api].shot(shot) -> base64 PNG, or {png})
    if (v.page !== undefined && !(v.page && typeof v.page.src === 'string' && /^__?[A-Za-z]\w*$/.test(v.page.api || '') && typeof v.page.shot === 'string')) bad(`view ${v.id}: page is {src, api, shot}`);
    for (const r of v.refs || []) {
      if (!r || typeof r.file !== 'string' || !r.file) bad(`view ${v.id}: a ref needs a file`);
      if (typeof r.take !== 'string' || !r.take.trim()) bad(`view ${v.id}: ref ${r.file} needs what to take from it`);
      if (!r.private && !r.licence) bad(`view ${v.id}: ref ${r.file} is in the repo, so it needs its licence (CC0, CC-BY, ours) or private: true`);
    }
  }
  return board;
}

/** Where things live, relative to the board file. */
export function places(boardPath, board) {
  const dir = dirname(resolve(boardPath));
  return {
    dir,
    root: resolve(dir, board.root || '..'),          // where shoot commands run: the repo
    refs: resolve(dir, board.refs || 'refs'),
    passes: resolve(dir, 'passes'),
    marks: resolve(dir, 'marks.jsonl'),
    privateRefs: process.env[board.private_refs_env || 'ELO_REF_DIR'] || null,
  };
}

export function refPath(p, r) {
  if (!r.private) return join(p.refs, r.file);
  return p.privateRefs ? join(p.privateRefs, r.file) : null;
}

/** Views that share one shoot command are shot by one run of it. */
export function groupShots(views) {
  const by = new Map();
  for (const v of views) {
    const k = JSON.stringify(v.shoot);
    if (!by.has(k)) by.set(k, { shoot: v.shoot, views: [] });
    by.get(k).views.push(v);
  }
  return [...by.values()];
}

export function pick(board, list) {
  if (!list) return board.views;
  const want = list.split(',').map((s) => s.trim()).filter(Boolean);
  const unknown = want.filter((w) => !board.views.some((v) => v.id === w));
  if (unknown.length) throw new Error(`no view ${unknown.join(', ')} on this board (${board.views.map((v) => v.id).join(', ')})`);
  return board.views.filter((v) => want.includes(v.id));
}

// ── marks ────────────────────────────────────────────────────────────────────

export function readMarks(path) {
  if (!existsSync(path)) return [];
  return readFileSync(path, 'utf8').split('\n').filter(Boolean).map((l) => {
    try { return JSON.parse(l); } catch { return null; }
  }).filter((m) => m && typeof m.view === 'string' && VERDICTS.includes(m.verdict));
}

export function mark(path, view, verdict, note, by, now = new Date()) {
  if (!VERDICTS.includes(verdict)) throw new Error(`a mark is ${VERDICTS.join(', ')} (got ${verdict})`);
  const row = { at: now.toISOString(), view, verdict, note: String(note || '').trim(), by: by || null };
  appendFileSync(path, JSON.stringify(row) + '\n');
  return row;
}

/** The latest right/wrong mark per view (notes ride along, never decide). */
export function lastVerdicts(marks) {
  const out = {};
  for (const m of marks) {
    out[m.view] ??= { verdict: null, at: null, note: '', notes: [] };
    if (m.verdict === 'note') out[m.view].notes.push(m);
    else Object.assign(out[m.view], { verdict: m.verdict, at: m.at, note: m.note });
  }
  return out;
}

// ── passes ───────────────────────────────────────────────────────────────────

export function stamp(now = new Date()) {
  return now.toISOString().replace(/[-:]/g, '').replace(/\.\d+Z$/, 'Z');
}

export function listPasses(dir) {
  if (!existsSync(dir)) return [];
  return readdirSync(dir).filter((n) => /^\d{8}T\d{6}Z$/.test(n)).sort();
}

export function shoot(boardPath, board, list) {
  const p = places(boardPath, board);
  const views = pick(board, list);
  const out = join(p.passes, stamp());
  mkdirSync(out, { recursive: true });
  const got = {}, failed = [];
  for (const g of groupShots(views)) {
    const [cmd, ...args] = g.shoot;
    process.stderr.write(`board: ${g.shoot.join(' ')}  (${g.views.map((v) => v.id).join(', ')})\n`);
    const t0 = Date.now();
    const r = spawnSync(cmd, args, { cwd: p.root, stdio: ['ignore', 'inherit', 'inherit'] });
    const secs = ((Date.now() - t0) / 1000).toFixed(1);
    for (const v of g.views) {
      const src = resolve(p.root, v.png);
      if (r.status === 0 && existsSync(src)) {
        copyFileSync(src, join(out, `${v.id}.png`));
        got[v.id] = { png: `${v.id}.png`, secs: Number(secs) };
      } else {
        failed.push(v.id);
        got[v.id] = { png: null, why: r.error ? r.error.message : r.status !== 0 ? `the shooter exited ${r.status}` : `${v.png} was not written` };
      }
    }
  }
  writeFileSync(join(out, 'pass.json'), JSON.stringify({ at: new Date().toISOString(), views: got }, null, 1));
  for (const old of listPasses(p.passes).slice(0, -KEEP_PASSES)) spawnSync('rm', ['-rf', join(p.passes, old)]);
  return { dir: out, got, failed };
}

// ── measurement: one estimator for both sides ───────────────────────────────

/** Rec.601 luma (whole, sky band = top 25% of rows, near band = bottom 35%)
    and chroma (max − min channel), each 0–255, over RGBA pixels. Runs in
    Node for the tests and, as source, in the page that reads the images. */
export function measure(rgba, w, h) {
  let all = 0, sky = 0, near = 0, chroma = 0, nAll = 0, nSky = 0, nNear = 0;
  const skyEnd = Math.floor(h * 0.25), nearStart = Math.ceil(h * 0.65);
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      const i = (y * w + x) * 4;
      const r = rgba[i], g = rgba[i + 1], b = rgba[i + 2];
      const l = 0.299 * r + 0.587 * g + 0.114 * b;
      all += l; nAll++;
      chroma += Math.max(r, g, b) - Math.min(r, g, b);
      if (y < skyEnd) { sky += l; nSky++; }
      if (y >= nearStart) { near += l; nNear++; }
    }
  }
  const r1 = (s, n) => (n ? Math.round((s / n) * 10) / 10 : null);
  return { luma: r1(all, nAll), sky: r1(sky, nSky), near: r1(near, nNear), chroma: r1(chroma, nAll) };
}

export const STAT_KEYS = ['luma', 'sky', 'near', 'chroma'];

export function median(xs) {
  const s = xs.filter((x) => typeof x === 'number').sort((a, b) => a - b);
  if (!s.length) return null;
  const m = s.length >> 1;
  return s.length % 2 ? s[m] : Math.round(((s[m - 1] + s[m]) / 2) * 10) / 10;
}

/** now − median(refs), per stat; null where either side is missing. */
export function delta(now, refs) {
  const out = {};
  for (const k of STAT_KEYS) {
    const m = median(refs.map((r) => r && r[k]));
    out[k] = now && typeof now[k] === 'number' && m !== null ? Math.round((now[k] - m) * 10) / 10 : null;
  }
  return out;
}

// ── the sheet ────────────────────────────────────────────────────────────────

const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]);

/** The sheet's HTML. `src(kind, view, i)` gives each image's URL. */
export function sheetHTML(board, rows, verdicts, title) {
  const cell = (img, label, key) => `<figure class="c">${img ? `<img data-k="${esc(key)}" src="${esc(img)}">` : '<div class="none">none</div>'}`
    + `<figcaption>${esc(label)}<span class="st" data-st="${esc(key)}"></span></figcaption></figure>`;
  const body = rows.map((r) => {
    const v = verdicts[r.view.id] || {};
    const verdict = v.verdict ? `<b class="${v.verdict}">${v.verdict}</b> ${esc(v.note)} <i>${esc((v.at || '').slice(0, 10))}</i>` : '<i>no mark yet</i>';
    const notes = (v.notes || []).slice(-2).map((n) => `<div class="note">note: ${esc(n.note)}</div>`).join('');
    const refs = r.refs.length
      ? r.refs.map((f, i) => cell(f.src, `ref ${i + 1}${f.private ? ' (private)' : ''}: take ${f.take}${f.ignore ? ' · ignore ' + f.ignore : ''}`, `${r.view.id}:ref:${i}`)).join('')
      : cell(null, 'no reference yet', `${r.view.id}:ref:none`);
    return `<section><header><h2>${esc(r.view.name || r.view.id)}${r.view.name ? ` <span class="id">${esc(r.view.id)}</span>` : ''}</h2><p>${esc(r.view.what || '')}</p>`
      + (r.view.take ? `<p class="take">aim: ${esc(r.view.take)}</p>` : '')
      + `<p class="mark">${verdict}</p>${notes}</header>`
      + `<div class="row"><div class="refs">${refs}</div>`
      + cell(r.now, 'now', `${r.view.id}:now`) + cell(r.prev, 'last pass', `${r.view.id}:prev`)
      + '</div></section>';
  }).join('');
  return `<!doctype html><meta charset="utf-8"><title>${esc(title)}</title><style>
body{margin:0;padding:16px;background:#111;color:#ddd;font:13px/1.4 system-ui,sans-serif}
h1{font-size:18px;margin:0 0 12px}h2{font-size:15px;margin:0}h2 .id{font:12px ui-monospace,monospace;color:#888;margin-left:6px}section{border-top:1px solid #333;padding:10px 0}
header p{margin:2px 0;color:#aaa}.take{color:#cfc}.mark b.right{color:#6f6}.mark b.wrong{color:#f66}.note{color:#ccf}
.row{display:flex;gap:10px;align-items:flex-start}.refs{display:flex;gap:10px}
.c{margin:0;width:${SHEET_COL}px}.c img{width:${SHEET_COL}px;display:block;border:1px solid #333}
.none{width:${SHEET_COL}px;height:${Math.round(SHEET_COL * 0.6)}px;display:flex;align-items:center;justify-content:center;border:1px dashed #444;color:#666}
figcaption{font-size:12px;color:#bbb}.st{display:block;font:12px ui-monospace,monospace;color:#fff}.st .d{color:#fc6}
</style><h1>${esc(title)}</h1>${body}`;
}

function dataURL(path) {
  const ext = extname(path).slice(1).toLowerCase();
  const type = ext === 'jpg' ? 'jpeg' : ext;
  return `data:image/${type};base64,${readFileSync(path).toString('base64')}`;
}

function loadPlaywright() {
  const require = createRequire(import.meta.url);
  for (const n of ['playwright', 'playwright-core', resolve(process.cwd(), 'node_modules/playwright')]) {
    try { return require(n); } catch { /* next */ }
  }
  return null;
}

export async function sheet(boardPath, board, passName) {
  const p = places(boardPath, board);
  const passes = listPasses(p.passes);
  if (!passes.length) throw new Error('no pass yet: run shoot first');
  const name = passName || passes[passes.length - 1];
  const i = passes.indexOf(name);
  if (i < 0) throw new Error(`no pass ${name}`);
  const cur = join(p.passes, name), prev = i > 0 ? join(p.passes, passes[i - 1]) : null;
  const verdicts = lastVerdicts(readMarks(p.marks));
  const missingPrivate = [];
  const rows = board.views.map((view) => {
    const now = join(cur, `${view.id}.png`), was = prev && join(prev, `${view.id}.png`);
    const refs = (view.refs || []).map((r) => {
      const f = refPath(p, r);
      if (!f || !existsSync(f)) { if (r.private) missingPrivate.push(r.file); return { ...r, path: null }; }
      return { ...r, path: f };
    });
    return { view, nowPath: existsSync(now) ? now : null, prevPath: was && existsSync(was) ? was : null, refs };
  });
  const title = `${board.game || 'board'} · pass ${name}${prev ? ` · against ${passes[i - 1]}` : ''}`;
  // the page on disk reads the files beside it; the measured copy carries them inline
  const rel = (f) => (f ? relative(cur, f) : null);
  const disk = rows.map((r) => ({ view: r.view, now: rel(r.nowPath), prev: rel(r.prevPath), refs: r.refs.map((f) => ({ ...f, src: rel(f.path) })) }));
  writeFileSync(join(cur, 'sheet.html'), sheetHTML(board, disk, verdicts, title));
  const result = { pass: name, dir: cur, html: join(cur, 'sheet.html'), png: null, stats: {}, missingPrivate };
  const pw = loadPlaywright();
  if (!pw) { result.why = 'Playwright is not installed here, so the sheet is HTML only and nothing was measured'; return result; }
  const inline = rows.map((r) => ({ view: r.view, now: r.nowPath && dataURL(r.nowPath), prev: r.prevPath && dataURL(r.prevPath), refs: r.refs.map((f) => ({ ...f, src: f.path && dataURL(f.path) })) }));
  const exe = process.env.CHROMIUM_PATH;
  const browser = await pw.chromium.launch({ executablePath: exe && existsSync(exe) ? exe : undefined, args: ['--no-sandbox'] });
  try {
    const page = await browser.newPage({ viewport: { width: 1480, height: 900 } });
    await page.setContent(sheetHTML(board, inline, verdicts, title), { waitUntil: 'load' });
    const stats = await page.evaluate(async (src) => {
      const measure = eval(`(${src})`); // eslint-disable-line no-eval
      const out = {};
      for (const img of document.querySelectorAll('img[data-k]')) {
        await img.decode().catch(() => {});
        const w = 256, h = Math.max(1, Math.round((img.naturalHeight / img.naturalWidth) * 256));
        const c = document.createElement('canvas'); c.width = w; c.height = h;
        const g = c.getContext('2d'); g.drawImage(img, 0, 0, w, h);
        out[img.dataset.k] = measure(g.getImageData(0, 0, w, h).data, w, h);
      }
      return out;
    }, measure.toString());
    // write the numbers under each picture, with now's distance from the refs
    const byView = {};
    for (const r of rows) {
      const refs = r.refs.map((_, k) => stats[`${r.view.id}:ref:${k}`]).filter(Boolean);
      const now = stats[`${r.view.id}:now`] || null, prev = stats[`${r.view.id}:prev`] || null;
      byView[r.view.id] = { now, prev, refs, ref_median: Object.fromEntries(STAT_KEYS.map((k) => [k, median(refs.map((x) => x[k]))])), now_minus_refs: delta(now, refs) };
    }
    await page.evaluate(({ stats, byView, keys }) => {
      for (const el of document.querySelectorAll('[data-st]')) {
        const s = stats[el.dataset.st];
        if (!s) continue;
        const [view, kind] = el.dataset.st.split(':');
        const d = kind === 'now' ? byView[view].now_minus_refs : null;
        el.innerHTML = keys.map((k) => `${k} ${s[k]}` + (d && d[k] !== null ? ` <span class="d">(${d[k] > 0 ? '+' : ''}${d[k]})</span>` : '')).join(' · ');
      }
    }, { stats, byView, keys: STAT_KEYS });
    const png = join(cur, 'sheet.png');
    await page.screenshot({ path: png, fullPage: true });
    writeFileSync(join(cur, 'stats.json'), JSON.stringify(byView, null, 1));
    Object.assign(result, { png, stats: byView });
  } finally {
    await browser.close();
  }
  return result;
}

// ── status ───────────────────────────────────────────────────────────────────

export function statusRows(board, marks, passes) {
  const v = lastVerdicts(marks);
  return board.views.map((view) => {
    const m = v[view.id];
    const since = m && m.at ? passes.filter((s) => s > stamp(new Date(m.at))).length : passes.length;
    return { view: view.id, verdict: m ? m.verdict : null, note: m ? m.note : '', marked: m ? m.at : null, passes_since: since, refs: (view.refs || []).length };
  });
}

// ── the command line ─────────────────────────────────────────────────────────

function opt(argv, k, d = null) {
  const i = argv.indexOf(`--${k}`);
  return i >= 0 ? argv[i + 1] : d;
}

function printStats(r) {
  for (const [id, s] of Object.entries(r.stats)) {
    const f = (x) => (x ? STAT_KEYS.map((k) => `${k} ${x[k]}`).join(' ') : '—');
    const d = s.now_minus_refs, hasD = STAT_KEYS.some((k) => d[k] !== null);
    console.log(`  ${id.padEnd(18)} now: ${f(s.now)}${hasD ? `   vs refs: ${STAT_KEYS.map((k) => `${k} ${d[k] > 0 ? '+' : ''}${d[k]}`).join(' ')}` : '   (no reference measured)'}`);
  }
}

async function main(argv) {
  let boardPath = 'board/board.json';
  if (argv[0] && argv[0].endsWith('.json')) boardPath = argv.shift();
  const cmd = argv.shift() || 'status';
  const board = readBoard(boardPath);
  const p = places(boardPath, board);
  if (cmd === 'shoot' || cmd === 'look') {
    const s = shoot(boardPath, board, opt(argv, 'views'));
    console.log(`pass ${basename(s.dir)}: ${Object.keys(s.got).length - s.failed.length} shot${s.failed.length ? `, failed: ${s.failed.map((id) => `${id} (${s.got[id].why})`).join('; ')}` : ''}`);
    if (cmd === 'shoot') return s.failed.length ? 1 : 0;
  }
  if (cmd === 'sheet' || cmd === 'look') {
    const r = await sheet(boardPath, board, opt(argv, 'pass'));
    console.log(`sheet: ${r.png || r.html}${r.why ? `  (${r.why})` : ''}`);
    if (r.missingPrivate.length) console.log(`  private refs not found (set ${board.private_refs_env || 'ELO_REF_DIR'}): ${r.missingPrivate.join(', ')}`);
    printStats(r);
    const st = statusRows(board, readMarks(p.marks), listPasses(p.passes));
    for (const s of st) console.log(`  ${s.view.padEnd(18)} ${s.verdict ? `${s.verdict}: ${s.note}` : 'no mark yet'}`);
    return 0;
  }
  if (cmd === 'mark') {
    const [view, verdict, note] = argv;
    pick(board, view);
    console.log(JSON.stringify(mark(p.marks, view, verdict, note, opt(argv, 'by'))));
    return 0;
  }
  if (cmd === 'ref') {
    const [view, file] = argv;
    const v = pick(board, view)[0];
    if (!file || !existsSync(file)) throw new Error(`no file ${file}`);
    const take = opt(argv, 'take');
    if (!take) throw new Error('--take: what to take from this reference');
    const priv = argv.includes('--private');
    const licence = opt(argv, 'licence');
    if (!priv && !licence) throw new Error('--licence (CC0, CC-BY, ours), or --private to keep the file out of the repo');
    const name = basename(file);
    if (!priv) { mkdirSync(p.refs, { recursive: true }); copyFileSync(file, join(p.refs, name)); }
    else if (!p.privateRefs) throw new Error(`a private ref lives in $${board.private_refs_env || 'ELO_REF_DIR'}: set it, and put the file there`);
    else if (!existsSync(join(p.privateRefs, name))) copyFileSync(file, join(p.privateRefs, name));
    v.refs = [...(v.refs || []), { file: name, take, ignore: opt(argv, 'ignore') || undefined, source: opt(argv, 'source') || undefined, licence: licence || undefined, private: priv || undefined }];
    checkBoard(board, boardPath);
    writeFileSync(boardPath, JSON.stringify(board, null, 2) + '\n');
    console.log(`${view}: ref ${name}${priv ? ' (private: only its name is in the board)' : ''}`);
    return 0;
  }
  if (cmd === 'status') {
    for (const s of statusRows(board, readMarks(p.marks), listPasses(p.passes))) {
      console.log(`${s.view.padEnd(18)} ${String(s.refs).padStart(2)} refs  ${s.verdict ? `${s.verdict.padEnd(5)} ${s.note}  (${s.passes_since} passes since)` : 'no mark yet'}`);
    }
    return 0;
  }
  throw new Error(`unknown command ${cmd}: look, shoot, sheet, mark, ref, status`);
}

if (import.meta.url === pathToFileURL(process.argv[1] || '').href) {
  main(process.argv.slice(2)).then((code) => process.exit(code), (e) => { console.error(`board: ${e.message}`); process.exit(2); });
}
