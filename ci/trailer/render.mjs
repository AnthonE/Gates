// Drive trailer.html in headless Chromium (make.sh copies this into build/).
// Shared with scry-forge's art/junk-trailer; TRAILER_STEP is the preview's.
//   node render.mjs stills OUTDIR t1,t2,...      one JPEG per time, for checking frames
//   node render.mjs video OUT.mkv [from] [to]    every frame, piped to ffmpeg (no audio)
//   node render.mjs cues OUT.json                the scene times and the audio cue list
// TRAILER_URL is the page (make.sh serves build/); TRAILER_Q its query, e.g. ?ratio=250.
import { chromium } from 'playwright';
import { spawn } from 'node:child_process';
import { mkdirSync, writeFileSync } from 'node:fs';

const [mode, out, a, b] = process.argv.slice(2);
const url = (process.env.TRAILER_URL || 'http://127.0.0.1:8765/trailer.html') + (process.env.TRAILER_Q || '');
const br = await chromium.launch({ args: ['--use-angle=swiftshader', '--enable-unsafe-swiftshader', '--ignore-gpu-blocklist'] });
const p = await br.newPage({ viewport: { width: 1920, height: 1080 }, deviceScaleFactor: 1 });
p.on('console', m => { if (!m.text().includes('WebGLRenderer')) console.log('page:', m.text()); });
p.on('pageerror', e => console.log('page error:', e.message));
await p.goto(url);
await p.waitForFunction(() => window.READY || window.FAILED, null, { timeout: 180000 });
const failed = await p.evaluate(() => window.FAILED);
if (failed) { console.error('trailer did not load:', failed); process.exit(1); }
const T = await p.evaluate(() => window.TRAILER);
const shot = () => p.screenshot({ type: 'jpeg', quality: 95, clip: { x: 0, y: 0, width: 1920, height: 1080 } });

if (mode === 'cues') {
  writeFileSync(out, JSON.stringify(T, null, 1));
} else if (mode === 'stills') {
  mkdirSync(out, { recursive: true });
  for (const t of a.split(',').map(Number)) {
    const id = await p.evaluate(t => window.seek(t), t);
    writeFileSync(`${out}/t${t.toFixed(2)}-${id}.jpg`, await shot());
  }
} else if (mode === 'video') {
  const from = a ? Math.round(+a * T.FPS) : 0, to = b ? Math.round(+b * T.FPS) : Math.round(T.TOTAL * T.FPS);
  const step = +(process.env.TRAILER_STEP || 1);   // preview: every Nth frame, held N frames
  const ff = spawn('ffmpeg', ['-v', 'error', '-y', '-f', 'image2pipe', '-framerate', String(T.FPS / step), '-c:v', 'mjpeg', '-i', '-',
    '-c:v', 'libx264', '-preset', 'medium', '-crf', '12', '-pix_fmt', 'yuv420p', out], { stdio: ['pipe', 'inherit', 'inherit'] });
  const t0 = Date.now();
  for (let f = from; f < to; f += step) {
    await p.evaluate(t => window.seek(t), f / T.FPS);
    const buf = await shot();
    if (!ff.stdin.write(buf)) await new Promise(r => ff.stdin.once('drain', r));
    if (f % (300 * step) === 0) console.log(`render: frame ${f}/${to}, ${((Date.now() - t0) / 1000).toFixed(0)} s`);
  }
  ff.stdin.end();
  await new Promise(r => ff.on('close', r));
} else {
  console.error('render.mjs stills|video|cues …'); process.exit(2);
}
await br.close();
