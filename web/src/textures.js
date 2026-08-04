// The base maps — real, measured, photographed detail for the ground and (in
// the slice after this one) the props.
//
// Why this file exists at all is `ART.md` §7 and the operator call behind it
// (2026-08-03, `DECISIONS.md`: *"if its CC0 im fine to pull in whatever helps
// us. then we can replace later"*). Everything in `materials.js` up to now is
// a noise field, and a noise field cannot encode what a camera measured: the
// reference set's near-ground neighbour contrast is 6.3 luma per pixel
// (`ART.md` §3) and eight passes of procedural octaves have moved ours from
// 0.26 to 0.26. A 1K photographed albedo carries that number by construction.
//
// The policy this implements, verbatim from §7, is HYBRID and not replacement.
// What arrives here is a base — albedo, normal, roughness — and every layer
// `materials.js` already had (the splat blend, the identity tint, the grain
// octave, the three structural octaves, the causal modifiers) stays on top of
// it as the variation layer. Two consequences are built into the arithmetic
// below rather than left to discipline:
//
//   · **The identity's authored colour is still its MEAN.** Each albedo layer
//     is measured at load (`meanLinear`) and delivered through a per-channel
//     gain `color / mean`, so what the photograph contributes is its VARIANCE
//     and what the palette contributes is its mean. Every assertion in the
//     tree about `IDENTITIES[i].color` — the tint octave's luminance
//     neutrality, 15d's "mean luma still", the identity's place in `ART.md`
//     §3's measured band — survives a source swap unchanged.
//   · **…which is also how §7's "an off-band source is tinted, not
//     tolerated" is enforced mechanically.** `MANIFEST.md` marks `rock`
//     (cliff_side) as too warm and too saturated for granite and asks for
//     exactly this treatment. It gets it for free: the gain divides the
//     sandstone's own mean out and multiplies the authored granite grey in.
//     The file on disk is never edited, so replacing it is still a file swap.
//
// One array texture per map, four layers, rather than twelve `sampler2D`.
// That is not a style choice: the terrain program already binds five shadow
// maps, and `MAX_TEXTURE_IMAGE_UNITS` is 16 on the software rasterizer this
// box's browser gate runs. Twelve more samplers does not fit; three does.

import * as THREE from "three";

// Vite resolves each of these to a URL and copies the file into `dist/assets/`
// at build time, which is what the browser gate serves. Explicit imports
// rather than a glob so that a missing or renamed file is a BUILD failure —
// the loudest kind — instead of a run-time 404 the client discovers in front
// of a player.
import grassAlbedoUrl from "../../assets/textures/grass_albedo.jpg";
import grassNormalUrl from "../../assets/textures/grass_normal.jpg";
import grassRoughUrl from "../../assets/textures/grass_rough.jpg";
import litterAlbedoUrl from "../../assets/textures/litter_albedo.jpg";
import litterNormalUrl from "../../assets/textures/litter_normal.jpg";
import litterRoughUrl from "../../assets/textures/litter_rough.jpg";
import rockAlbedoUrl from "../../assets/textures/rock_albedo.jpg";
import rockNormalUrl from "../../assets/textures/rock_normal.jpg";
import rockRoughUrl from "../../assets/textures/rock_rough.jpg";
import sandAlbedoUrl from "../../assets/textures/sand_albedo.jpg";
import sandNormalUrl from "../../assets/textures/sand_normal.jpg";
import sandRoughUrl from "../../assets/textures/sand_rough.jpg";

/**
 * The layer order IS `IDENTITIES`' order in `materials.js` (sand, grass,
 * litter, rock) and the splat weights' order, so `gmW[i]` and layer `i` are
 * the same identity by construction rather than by a lookup table that can
 * drift. `materials.js` asserts the two lists match at module load.
 */
export const GROUND_LAYERS = ["sand", "grass", "litter", "rock"];

const SOURCES = {
  sand: { albedo: sandAlbedoUrl, normal: sandNormalUrl, rough: sandRoughUrl },
  grass: { albedo: grassAlbedoUrl, normal: grassNormalUrl, rough: grassRoughUrl },
  litter: { albedo: litterAlbedoUrl, normal: litterNormalUrl, rough: litterRoughUrl },
  rock: { albedo: rockAlbedoUrl, normal: rockNormalUrl, rough: rockRoughUrl },
};

// sRGB -> linear, 256 entries, built once. The albedo means below have to be
// taken in LINEAR light or the gain would be wrong in the direction that
// matters most: the mean of a decoded sRGB byte is not the byte of the linear
// mean, and a texture with any contrast in it reads several percent too bright
// that way. This is the same transfer three applies on the sampler, written
// out here so the CPU-side mean and the GPU-side sample agree by construction.
const SRGB_TO_LINEAR = (() => {
  const t = new Float32Array(256);
  for (let i = 0; i < 256; i++) {
    const c = i / 255;
    t[i] = c <= 0.04045 ? c / 12.92 : Math.pow((c + 0.055) / 1.055, 2.4);
  }
  return t;
})();

/**
 * Decode one source file to raw bytes, already in the orientation the GPU
 * wants.
 *
 * `imageOrientation: "flipY"` with `texture.flipY = false` is the pair that
 * gives the shader the standard tangent frame: the bitmap's first row is the
 * image's BOTTOM, so v increases toward image-up, so the normal map's green
 * channel (+Y in the OpenGL convention Poly Haven publishes) points along +v.
 * `materials.js` maps v to world +z, so green means "tilted toward +z" and the
 * light lands on the side of a pebble a photograph would put it on. Getting
 * this pair wrong inverts every bump on the ground and nothing but a picture
 * would say so, which is why it is stated here rather than assumed.
 */
async function decode(url) {
  const res = await fetch(url);
  if (!res.ok) throw new Error(`base texture ${url}: HTTP ${res.status}`);
  const bmp = await createImageBitmap(await res.blob(), { imageOrientation: "flipY" });
  const width = bmp.width;
  const height = bmp.height;
  const canvas = new OffscreenCanvas(width, height);
  const ctx = canvas.getContext("2d", { willReadFrequently: true });
  ctx.drawImage(bmp, 0, 0);
  bmp.close();
  return { url, width, height, data: ctx.getImageData(0, 0, width, height).data };
}

/** Every layer of one map has to agree on size — an array texture is one grid. */
function assertUniform(images, map) {
  const { width, height } = images[0];
  for (const img of images) {
    if (img.width !== width || img.height !== height) {
      throw new Error(
        `base texture ${map}: ${img.url} is ${img.width}x${img.height}, ` +
          `expected ${width}x${height} — one array texture is one grid`,
      );
    }
  }
  return { width, height };
}

function makeArray(data, width, height, format, colorSpace) {
  const tex = new THREE.DataArrayTexture(data, width, height, GROUND_LAYERS.length);
  tex.format = format;
  tex.type = THREE.UnsignedByteType;
  tex.colorSpace = colorSpace;
  tex.wrapS = THREE.RepeatWrapping;
  tex.wrapT = THREE.RepeatWrapping;
  // Mipmaps are the whole reason a photographed base can be sampled at a 0.6–1
  // metre tile without turning into the aliasing the octave table spent a pass
  // learning to avoid: the octaves retire because a reconstructed field cannot
  // be filtered, and a texture does not have to retire because it can.
  tex.generateMipmaps = true;
  tex.minFilter = THREE.LinearMipmapLinearFilter;
  tex.magFilter = THREE.LinearFilter;
  tex.flipY = false;
  tex.needsUpdate = true;
  return tex;
}

/** Pack N decoded RGBA images into one interleaved layer buffer. */
function pack(images, channels, pick) {
  const { width, height } = images[0];
  const stride = width * height;
  const out = new Uint8Array(stride * channels * images.length);
  for (let layer = 0; layer < images.length; layer++) {
    const src = images[layer].data;
    let w = layer * stride * channels;
    for (let p = 0; p < stride; p++) {
      pick(src, p * 4, out, w);
      w += channels;
    }
  }
  return out;
}

let loaded = null;

/**
 * Load the four ground identities' albedo / normal / roughness and measure
 * what each one delivers.
 *
 * Called once, from `boot()`, and AWAITED before anything builds a material.
 * That ordering is load-bearing twice over: `makeTerrainMaterial()` throws
 * without it (a ground with no base is not a fallback anyone should ship
 * quietly), and the prewarm gate asserts zero program links after `inWorld` —
 * a texture that arrives late is a relink, so all twelve arrive at boot.
 */
export async function loadGroundTextures() {
  if (loaded) return loaded;

  const files = await Promise.all(
    GROUND_LAYERS.flatMap((name) => [
      decode(SOURCES[name].albedo),
      decode(SOURCES[name].normal),
      decode(SOURCES[name].rough),
    ]),
  );
  const albedo = GROUND_LAYERS.map((_, i) => files[i * 3]);
  const normal = GROUND_LAYERS.map((_, i) => files[i * 3 + 1]);
  const rough = GROUND_LAYERS.map((_, i) => files[i * 3 + 2]);

  const albSize = assertUniform(albedo, "albedo");
  const nrmSize = assertUniform(normal, "normal");
  const rghSize = assertUniform(rough, "rough");

  // What each layer actually delivers, measured over every texel rather than
  // sampled: this is a 1M-pixel pass per albedo behind a 256-entry LUT, which
  // is cheaper than the JPEG decode that produced the pixels, and a subsample
  // would have been a number with a standard error in it standing where an
  // exact one was available.
  const meanLinear = albedo.map((img) => {
    const d = img.data;
    let r = 0;
    let g = 0;
    let b = 0;
    for (let p = 0; p < d.length; p += 4) {
      r += SRGB_TO_LINEAR[d[p]];
      g += SRGB_TO_LINEAR[d[p + 1]];
      b += SRGB_TO_LINEAR[d[p + 2]];
    }
    const n = d.length / 4;
    return [r / n, g / n, b / n];
  });
  // Roughness is linear data, not colour, so its mean is the raw byte mean.
  const meanRough = rough.map((img) => {
    const d = img.data;
    let s = 0;
    for (let p = 0; p < d.length; p += 4) s += d[p];
    return s / (d.length / 4) / 255;
  });
  // Published for the gate, not used by the shader: the share of each albedo's
  // texels above its own mean and the spread around it, which is the number
  // that says a photograph arrived rather than a flat swatch. A solid colour
  // scores sd 0.
  const albedoSd = albedo.map((img, i) => {
    const d = img.data;
    const m = meanLinear[i];
    let s = 0;
    for (let p = 0; p < d.length; p += 4) {
      const l =
        0.2126 * SRGB_TO_LINEAR[d[p]] +
        0.7152 * SRGB_TO_LINEAR[d[p + 1]] +
        0.0722 * SRGB_TO_LINEAR[d[p + 2]];
      const mm = 0.2126 * m[0] + 0.7152 * m[1] + 0.0722 * m[2];
      s += (l - mm) * (l - mm);
    }
    return Math.sqrt(s / (d.length / 4));
  });

  loaded = {
    albedo: makeArray(
      pack(albedo, 4, (s, i, o, w) => {
        o[w] = s[i];
        o[w + 1] = s[i + 1];
        o[w + 2] = s[i + 2];
        o[w + 3] = 255;
      }),
      albSize.width,
      albSize.height,
      THREE.RGBAFormat,
      THREE.SRGBColorSpace,
    ),
    // Two channels is all a tangent normal needs — z is reconstructed in the
    // shader from x and y, which is exact for a unit vector and halves what
    // the biggest of the three arrays costs in memory.
    normal: makeArray(
      pack(normal, 2, (s, i, o, w) => {
        o[w] = s[i];
        o[w + 1] = s[i + 1];
      }),
      nrmSize.width,
      nrmSize.height,
      THREE.RGFormat,
      THREE.NoColorSpace,
    ),
    rough: makeArray(
      pack(rough, 1, (s, i, o, w) => {
        o[w] = s[i];
      }),
      rghSize.width,
      rghSize.height,
      THREE.RedFormat,
      THREE.NoColorSpace,
    ),
    meanLinear,
    meanRough,
    facts: {
      layers: GROUND_LAYERS.slice(),
      albedoSize: [albSize.width, albSize.height],
      normalSize: [nrmSize.width, nrmSize.height],
      roughSize: [rghSize.width, rghSize.height],
      meanLinear,
      meanRough,
      albedoSd,
      anisotropy: 0,
    },
  };
  return loaded;
}

/** The loaded set, or null. `materialFacts()` reports the null case honestly. */
export function groundTextures() {
  return loaded;
}

/**
 * The most anisotropy this material asks for.
 *
 * `getMaxAnisotropy()` is a device CAPABILITY, not a recommendation, and on
 * every renderer this client has been run on it answers 16. Taking it whole is
 * what the first cut did, and it is the wrong read of the number twice over:
 * the benefit of anisotropic filtering on a ground texture saturates by 4–8 —
 * past that the extra taps are resolving detail the mip chain has already
 * averaged — while the COST is linear in taps, and this ground takes up to
 * twenty-four filtered fetches per fragment since the biplanar wall tap
 * (twelve on level ground, where the wall block is skipped). 16 x 24 is 384
 * texel samples on a fragment, which the reference VPS would not notice and a
 * software rasterizer very much does: measured on this box under the twelve
 * that preceded biplanar, at 16 a second browser tab did not reach the world
 * at all, and the second plane only widened that.
 *
 * 4 is the conventional floor where aniso has already bought most of what it
 * buys over trilinear, and it is a proposed default recorded in
 * `DECISIONS.md` §open rather than a spoken one. The value that actually
 * ships is `min(this, device max)`, so a device that offers less still gets a
 * true answer out of `baseFacts()`.
 */
const BASE_ANISOTROPY_MAX = 4;

/**
 * Ask for it. A 0.6–1 m tile seen at a grazing angle is the case the whole
 * slice is aimed at (the near ground), and it is exactly the case an isotropic
 * mip chain over-blurs back into the flat wash this material exists to escape
 * — so the ask is real, it is just not unbounded.
 *
 * Called from `run()` after the renderer exists and before the first frame, so
 * the value is on the texture when it is first uploaded.
 */
export function setGroundAnisotropy(deviceMax) {
  if (!loaded) return 0;
  const aniso = Math.max(1, Math.min(BASE_ANISOTROPY_MAX, deviceMax || 1));
  for (const key of ["albedo", "normal", "rough"]) {
    loaded[key].anisotropy = aniso;
    loaded[key].needsUpdate = true;
  }
  loaded.facts.anisotropy = aniso;
  loaded.facts.anisotropyDeviceMax = deviceMax;
  loaded.facts.anisotropyMax = BASE_ANISOTROPY_MAX;
  return aniso;
}
