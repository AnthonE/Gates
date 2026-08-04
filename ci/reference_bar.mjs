#!/usr/bin/env node
// The reference bar, measured — not quoted.
//
// `art/RUBRIC.md` scores our frames against `Rust Images/` as an ABSOLUTE bar,
// and its criterion 8 (lighting and depth) is the one this repo has failed on
// every capture so far. Every previous lighting number in this tree was a
// ratio against the pass before it, which is exactly how a scene can improve
// six times and still sit a stop and a half under the thing it is being
// compared to.
//
// So the bar is read off the checked-in reference frames, in the same run, by
// the same code path that reads ours: decode in a browser page, convert to
// Rec.601 luma, take percentiles. The reference images ship in the repo
// (`Rust Images/`, 18 of them), so this needs no network and no fixture.
//
// Two things it deliberately does NOT do:
//
//   · It does not crop the HUD out. Both sides carry a HUD, both are measured
//     whole, and the frames chosen below are ones whose HUD is a thin strip.
//   · It does not compare composition, content or colour — only the tonal
//     REGISTER (where the midtones sit, how far the range reaches). That is
//     the one axis a light rig owns, and the one this file's caller asserts.
//
// Runnable two ways: imported by `ci/browser_smoke.mjs` (which already has a
// page open, so the measurement costs one decode per image), or standalone
// (`node ci/reference_bar.mjs`) to re-derive the numbers a knob was set from.

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
export const REFERENCE_DIR = path.join(ROOT, "Rust Images");

// The outdoor-daylight subset. Every frame here is first-person or near-ground
// gameplay under an open sky — the thing our six vantages are. The interiors
// (`storageandtoolchest`), the menus (`inventory`, `crafting`, `building`), the
// maps and the night/wounded frames are excluded on purpose: a tonal bar taken
// over a fullscreen inventory screen would be a bar about UI, not about light.
export const REFERENCE_FRAMES = [
  "generichighview2.jpg", // pine forest, granite, shoreline, cumulus sky
  "gameplayfoundbase.jpeg", // first-person, held item, log base
  "choppingtree.jpg", // gather feedback, forest floor
  "spawnedrock.jpg", // stone node, grass
  "generic2.jpeg", // biome transition, long sightlines
  "roads.jpeg", // arid biome, road, aerial perspective
];

/**
 * Decode one image in the page and return its luma percentiles.
 *
 * Rec.601 with the same integer weights `scene.js`'s probes use
 * ((2R + 5G + B) >> 3), so a number measured here and a number measured off
 * our own framebuffer are the same statistic and can be compared directly.
 */
async function measureOne(page, name, bytes) {
  const b64 = bytes.toString("base64");
  const mime = name.endsWith(".png")
    ? "image/png"
    : name.endsWith(".webp")
      ? "image/webp"
      : "image/jpeg";
  return await page.evaluate(
    async ([url, label]) => {
      const res = await fetch(url);
      const blob = await res.blob();
      const bmp = await createImageBitmap(blob);
      const cv = new OffscreenCanvas(bmp.width, bmp.height);
      const ctx = cv.getContext("2d", { willReadFrequently: true });
      ctx.drawImage(bmp, 0, 0);
      const d = ctx.getImageData(0, 0, bmp.width, bmp.height).data;
      const hist = new Int32Array(256);
      const n = bmp.width * bmp.height;
      for (let i = 0; i < n; i++) {
        const p = i * 4;
        hist[(d[p] * 2 + d[p + 1] * 5 + d[p + 2]) >> 3]++;
      }
      const at = (q) => {
        let acc = 0;
        const want = q * n;
        for (let l = 0; l < 256; l++) {
          acc += hist[l];
          if (acc >= want) return l;
        }
        return 255;
      };
      let sum = 0;
      for (let l = 0; l < 256; l++) sum += l * hist[l];
      // The sky band: the top eighth of the frame. Every reference frame in
      // the set is level-or-below horizon, so those rows are sky in all of
      // them, and it is the band our dome has to answer.
      const skyRows = Math.max(1, Math.round(bmp.height / 8));
      let skySum = 0;
      let skyMin = 255;
      let skyMax = 0;
      const skyHist = new Int32Array(256);
      for (let y = 0; y < skyRows; y++) {
        for (let x = 0; x < bmp.width; x++) {
          const p = (y * bmp.width + x) * 4;
          const l = (d[p] * 2 + d[p + 1] * 5 + d[p + 2]) >> 3;
          skySum += l;
          skyHist[l]++;
          if (l < skyMin) skyMin = l;
          if (l > skyMax) skyMax = l;
        }
      }
      const skyN = skyRows * bmp.width;
      // Distinct luma levels present in the sky band, as a share of the range
      // it spans: the posterization measure. A dome ramped over 24x16 vertex
      // colours and quantized to 8 bits has long flat runs; a real sky (or a
      // dithered one) does not.
      let skyLevels = 0;
      for (let l = 0; l < 256; l++) if (skyHist[l] > 0) skyLevels++;
      return {
        label,
        width: bmp.width,
        height: bmp.height,
        p05: at(0.05),
        p10: at(0.1),
        p50: at(0.5),
        p90: at(0.9),
        p95: at(0.95),
        mean: sum / n,
        skyMean: skySum / skyN,
        skyMin,
        skyMax,
        skyLevels,
      };
    },
    [`data:${mime};base64,${b64}`, name],
  );
}

/**
 * Measure the reference subset. Loud failure on a missing or unreadable file:
 * a bar that silently measures five frames instead of six is a bar that moved
 * without anyone deciding to move it.
 */
export async function measureReference(page, frames = REFERENCE_FRAMES) {
  const out = [];
  for (const name of frames) {
    const file = path.join(REFERENCE_DIR, name);
    if (!fs.existsSync(file)) {
      throw new Error(
        `reference frame missing: ${file} — the tonal bar is measured off ` +
          `\`Rust Images/\`, so a missing frame is a moved bar, not a skip`,
      );
    }
    const m = await measureOne(page, name, fs.readFileSync(file));
    if (!(m.width > 0) || !(m.p90 > m.p10)) {
      throw new Error(`reference frame ${name} decoded to nothing usable: ${JSON.stringify(m)}`);
    }
    out.push(m);
  }
  const med = (key) => {
    const v = out.map((m) => m[key]).sort((a, b) => a - b);
    return v.length % 2 ? v[(v.length - 1) / 2] : (v[v.length / 2 - 1] + v[v.length / 2]) / 2;
  };
  return {
    frames: out,
    // The bar itself: the MEDIAN across the subset, not the mean and not the
    // minimum. A median cannot be dragged by one atypical frame in either
    // direction, and "half the reference set is brighter than this" is a
    // sentence a threshold can be written from.
    p10: med("p10"),
    p50: med("p50"),
    p90: med("p90"),
    skyMean: med("skyMean"),
    skyLevels: med("skyLevels"),
    range: med("p90") - med("p10"),
  };
}

// --- standalone ---------------------------------------------------------------
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const { createRequire } = await import("node:module");
  const require = createRequire(path.join(ROOT, "web/package.json"));
  const { pathToFileURL } = await import("node:url");
  const mod = await import(pathToFileURL(require.resolve("playwright")).href);
  const chromium = mod.chromium || mod.default?.chromium;
  const browser = await chromium.launch({ args: ["--allow-file-access-from-files"] });
  const page = await browser.newPage();
  const bar = await measureReference(page);
  for (const m of bar.frames) {
    console.log(
      `  ${m.label.padEnd(24)} ${m.width}x${m.height}  p10 ${String(m.p10).padStart(3)} · ` +
        `p50 ${String(m.p50).padStart(3)} · p90 ${String(m.p90).padStart(3)} · ` +
        `mean ${m.mean.toFixed(1).padStart(5)} · sky ${m.skyMean.toFixed(1).padStart(5)} ` +
        `[${m.skyMin}..${m.skyMax}, ${m.skyLevels} levels]`,
    );
  }
  console.log(
    `\nBAR (median of ${bar.frames.length}): p10 ${bar.p10} · p50 ${bar.p50} · p90 ${bar.p90} · ` +
      `range ${bar.range} · sky mean ${bar.skyMean.toFixed(1)} · sky levels ${bar.skyLevels}`,
  );
  await browser.close();
}
