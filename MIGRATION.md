# Gates · MIGRATION.md — the renderer stack, WebGL against WebGPU/TSL

**Decided 2026-08-04: route C. The client moves to `WebGPURenderer` + TSL.**
(Operator: *"for the record i am upgrading asap the graphics"* —
`DECISIONS.md` §Spoken, same date.) This doc costed the move while it was
still a question; it now serves as the **plan**. §4 records why C and not B,
§6 is the order of work, §7 is what is still unmeasured.

`NOW.md` remains the only list that answers "what next"; items get cut from
here into that queue, never the other way. Nothing here is a knob.

Dated 2026-08-04, measured against the installed `three@0.178.0`. A row that
disagrees with the code is wrong — fix the row. Every count below is
reproducible; §1 has the commands.

## 0 · Status: WebGL was never a decision — the move is

Recorded here because the asymmetry is the point. There was no row in
`DECISIONS.md`, `DESIGN.md`, or `ART.md` choosing WebGL.
The only mentions are incidental, and both read as a constraint someone
worked around rather than a call someone made:

- `DECISIONS.md`, shadow clipmap v0: *"**How it is built in WebGL** (the
  skill's API is WebGPU/TSL): N DirectionalLights, all casting…"*
- `web/src/shadows.js:33`: *"The skill's node API is WebGPU/TSL; this client
  is `WebGLRenderer`, so the…"*

WebGL arrived as the default of `import * as THREE from "three"` and every
graphics decision since has been made around it. **The move off it is
spoken and dated; the state it moves from never was.** That is the whole
reason this doc got written before the work started.

## 1 · Method

`three@0.178` ships the two stacks as disjoint builds in one package. Its
own export map:

```
"."         -> build/three.module.js    600 KB   ← every client file imports this
"./webgpu"  -> build/three.webgpu.js   1.79 MB
"./tsl"     -> build/three.tsl.js        28 KB
```

Counted in the installed builds, not assumed:

```sh
cd web/node_modules/three/build
grep -c "NodeMaterial" three.module.js        # 0
grep -c "class WebGLRenderer" three.webgpu.js # 0
grep -c "ShaderChunk" three.webgpu.js         # 0   (42 in three.module.js)
grep -c "onBeforeCompile" three.webgpu.js     # 0   (1 in three.module.js)
```

They do not overlap. TSL is not a shader dialect we could translate line by
line — it is a JavaScript node graph compiled at runtime by a compiler that
lives only in `three.webgpu.js`. `WebGLRenderer` has no node compiler, so a
`colorNode` assigned to a stock material is silently ignored; nothing throws.

Client surface, counted:

```sh
grep -rho "renderer\.[a-zA-Z]*" web/src/*.js | sort | uniq -c | sort -rn
grep -rn  "onBeforeCompile" web/src/*.js | wc -l
grep -rn  "getContext"      web/src/*.js
```

## 2 · What breaks, most expensive first

### 2.1 · The probe harness — 12 probes, 126 gate references

This is the migration's centre of gravity, and it is the **gates**, not the
game. Twelve probes in `scene.js` take the raw GL context and read the
default framebuffer synchronously:

| probe | `scene.js` | probe | `scene.js` |
|---|---|---|---|
| `horizonProbe` | 1029 | `aliasProbe` | 2034 |
| `contrastProbe` | 1196 | `baseProbe` | 2186 |
| `projectionProbe` | 1446 | `chromaProbe` | 2335 |
| `surfaceProbe` | 1654 | `propProbe` | 2503 |
| `daylightProbe` | 1764 | `costProbe` | 2793 |
| `shadowProbe` | 3114 | `farShadowProbe` | 3227 |

Each opens with `const gl = this.renderer.getContext()` and measures with
`gl.readPixels(0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, buf)` — 12 context
grabs, 43 `readPixels` call sites.
`ci/browser_smoke.mjs` references them 126 times. This is the whole
measurement culture: every number in `ART.md` §3, every 15b–15h assertion,
the far-shadow 0.78%-of-pixels floor, the A/B controls that make each probe's
zero point measured rather than argued.

Under WebGPU there is **no GL context and no synchronous readback**. The
replacement is `readRenderTargetPixelsAsync`, which differs twice over:

1. It reads a **render target**, not the default framebuffer. Every probe
   must first render into an explicit target it does not currently own.
2. It is **async**. Each probe body, and the gate harness calling them,
   changes shape.

`farShadowProbe` carries an extra hazard: it unprojects all eight frustum
corners into light space. WebGPU's clip-space Z is `0..1` where WebGL's is
`-1..1` (`renderer.coordinateSystem`), so that arithmetic must be re-derived,
not ported. It is the one probe whose *result* could change while its code
still looks correct — exactly the byte-golden-is-blind-to-meaning failure in
`CLAUDE.md`'s trap list.

**Nothing about this is optional.** A migration that lands the render path
and leaves the probes behind ships a client with no visual gates at all.

### 2.2 · Chunk splicing — 11 sites, three files

`shadows.js`, `materials.js`, `scene.js` (e.g. `scene.js:407`) all customise
lighting by splicing GLSL text into three's chunk system:

```js
shader.fragmentShader
  .replace("#include <shadowmap_pars_fragment>", `…${glsl}`)
  .replace("#include <lights_fragment_begin>", cachedChunk);
```

`onBeforeCompile`, `THREE.ShaderChunk`, `#include <…>` — all three are absent
from `three.webgpu.js`. The node system has no chunks and no compile hook;
you rebuild the graph instead.

The hardest single item is the clipmap shadow patch. Per `DECISIONS.md`, it
*"replaces three's per-light `getShadow(...)` in `lights_fragment_begin` with
one `gatesClipmapShadow()` reading all levels"*, reading the stock call off
the **installed** chunk and requiring it to appear exactly once — a boot-time
assertion that a three upgrade renames a shadow uniform. That safety net is
built out of the chunk system it protects, so it does not survive the move
either. Node-based shadows would need an equivalent assertion invented from
scratch.

Also here: `terrain.js` sets `customDepthMaterial` at four sites (335, 343,
400, 413), a WebGL shadow-pass concept with a different node analogue, and
`terrain.js:690` asserts its presence.

### 2.3 · The prewarm gate

`scene.js:961` calls `renderer.compile(scene, camera)`; `scene.js:962`,
`main.js:1191`, `1218`, `1319` read `renderer.info.programs.length`. The
trap list is explicit that the gate is a **COUNT** of program links after
`inWorld` and never a frame-time threshold, because median fps hides
shader-compile stalls.

WebGPU has no `WebGLProgram`. It has pipelines, `compileAsync()` is the
prewarm call, and `info` does not expose the same counter. The gate is
re-derivable — but per the trap list, a gate we cannot re-derive is worse
than the stall it catches, so proving the replacement counts the same class
of event is a precondition, not a follow-up.

### 2.4 · Small, mechanical

- `main.js:172` — `renderer.capabilities.getMaxAnisotropy()`. Different API;
  the `BASE_ANISOTROPY_MAX = 4` clamp above it is unaffected.
- `scene.js:473` — `NeutralToneMapping` / `toneMappingExposure`. The constant
  survives; under nodes, tone mapping moves to the output node, and
  `CLAUDE.md`'s one-owner law for tonemap/sky/exposure/fog means that move is
  the same owner's work or nobody's.
- Boot becomes async: `await renderer.init()`.
- **Post-processing has no bridge.** The legacy `EffectComposer` chain is
  WebGL-only with no forward path; the node pipeline is `PostProcessing` +
  the `tsl/display/*` nodes (§8.1). We ship no post today, so this is a
  greenfield choice rather than a port — and it is the same owner's, since
  tone map moves into the output node.

## 3 · What is unaffected

Most of the client. Of the 47 distinct `THREE.*` symbols in `web/src/` (162
references), the
geometry classes, `Vector2/3/4`, `Color`, `Matrix4`, `Quaternion`, `Euler`,
`Fog`, `DirectionalLight`, `HemisphereLight`, `InstancedMesh`,
`DataArrayTexture`, the wrapping/filter/colour-space constants — all exist
identically in `three.webgpu.js`. So does every one of `sim-core`, the wire,
the WAL, the terrain worker, and `textures.js`'s CPU-side mean measurement.

**This is a client rendering-path migration and nothing else.** No wall in
`CLAUDE.md` §The walls is touched: determinism, the wire, content, and the
allocation laws are all server- or sim-side. That bounds the blast radius
sharply, and it is the strongest single argument that the move is *possible*
whenever it is *wanted*.

## 4 · The three routes — **C chosen, 2026-08-04**

| route | what it is | buys | costs |
|---|---|---|---|
| **A · stay** | `WebGLRenderer`, hand-translate every node-API technique into GLSL | zero disruption; the probe harness and every gate keep working | the standing tax in §5; new three capability lands where we cannot reach it |
| **B · swap the renderer** | `WebGPURenderer` with the WebGL2 backend, stock materials, **keep GLSL splicing off the table** | nothing on its own | pays §2.1–2.3 in full for no visual gain. **Not recommended as a step** |
| **C · adopt the node stack** | `WebGPURenderer` + TSL for the material and lighting path | the skill pack becomes usable as written; volumetric clouds, node post, compute | §2 in full, one owner, one iteration, no visual deliverable until it lands |

**Route B is the trap and it survives the decision as a warning.** "Swap the
renderer now, port the materials later" reads like a safe increment and is
the worst of the three: chunk splicing and the probe harness both break the
moment the renderer changes, so B pays the entire bill and collects none of
the benefit. A route-C sequence that stalls after the renderer swap **is**
route B. §6's ordering exists to make that state unreachable.

The browser-reach argument decided nothing either way: `forceWebGL` and the
automatic WebGL2 fallback mean C costs no compatibility. The cost is entirely
in our own code.

## 5 · What route A was costing — the reason C was taken

Kept as the ledger behind the decision, not as a live comparison:

- The granted skill pack — `threejs-shadow-systems`, `threejs-volumetric-clouds`,
  `threejs-atmosphere-aerial-perspective` — is written in the node API. We
  hand-translate each one. `shadows.js:33` **is** that translation, written
  down as a comment.
- `ART.md`'s open visual gap is clouds in the sky (*"cloudless cannot reach a
  p90 of 189 with a sky mean of 143"*), and volumetric clouds are the most
  TSL-native technique in the current ecosystem.
- Third-party sky/weather work is unusable for the same reason —
  `SkyeShark/Eanpa-Sky` (MIT) is 300 KB of TSL against `globalThis.THREE`.
- We are 13 releases behind (`0.178.0` against `0.185.1`), and that gap is
  where the node stack is moving fastest.

## 6 · The order of work

Trigger 3 fired (operator, 2026-08-04), so this section is a sequence rather
than a condition. The ordering is not taste — each step exists because doing
it later costs more.

**Step 0 · bump three `0.178.0` → `0.185.1`, alone, on WebGL.** Land it as
its own change with nothing else in it. `shadows.js` throws at boot if the
stock `getShadow` call is not found exactly once in the installed chunk, and
seven minor versions is exactly the kind of gap that fires it. Read that
throw on a clean tree, where the only variable is the version — not inside a
renderer rewrite, where it would be one red among many. Everything downstream
also targets a moving library, and `SeedThree` already pins `^0.184.0`.

**Step 1 · port the probe harness first, still on WebGL.** Counter-intuitive
and the most important line in this doc. Give all 12 probes an explicit
`WebGLRenderTarget` and an async body *now*, while `WebGLRenderer` is still
underneath and every existing assertion can prove the port changed no number.
That converts §2.1 from "rewrite the gates blind during a renderer swap" into
"swap the renderer under gates that already have the right shape." Do it the
other way and there is a window with no visual gates at all, which
`DECISIONS.md` forbids outright.

**Step 2 · re-derive the prewarm COUNT.** Establish what replaces
`renderer.info.programs.length` under pipelines and prove it catches the same
event class, before the render path depends on it. A gate we cannot
re-derive is worse than the stall it catches.

**Step 3 · swap the renderer and rebuild the material path together.** This
is route C's single lane and it does not subdivide: `scene.js`,
`materials.js`, `shadows.js`, `terrain.js`, `main.js`. Node materials replace
the 11 `onBeforeCompile` sites; three's transpiler (§8.2) converts the GLSL
bodies mechanically; `CSMShadowNode` / `TileShadowNode` (§8.1) are the
worked references for the clipmap. Tone map moves to the output node, which
per `CLAUDE.md` puts sky, fog and exposure in the same hands in the same pass.

**Step 4 · only then, the visual work the upgrade was for.** Clouds (§8.3.1
donates the noise, presets and density function on either stack), `SkyMesh`
for the dome, `GTAONode` and `Lut3DNode` if wanted. None of this is a
prerequisite for steps 0–3, and mixing it in is how a renderer swap becomes
unreviewable.

**Two standing constraints for the whole sequence.** It cannot be split
across parallel loops — one owner per crate per iteration, and this touches
five client files plus `browser_smoke.mjs` at once. And the loop should be
stopped or fenced off it: every visual judge pass reads captures produced by
the harness being rebuilt, so a pass landing mid-sequence scores noise.

## 7 · Open, unmeasured

Named rather than guessed at, because none of it can be answered without
building something:

- **Cost on the gate box.** Eight cores, software rasterizer in CI. The
  WebGL2 backend of `WebGPURenderer` is a different code path from
  `WebGLRenderer` and its cost here is unmeasured. `DECISIONS.md`'s ground
  base maps row records anisotropy 16 keeping a second tab out of the world
  entirely — this box has form for making renderer changes expensive.
- **How close `CSMShadowNode` gets to the clipmap** (§8.1). The extension
  points exist and a 456-line worked example ships; whether concentric
  committed centres, per-level update budgets and `invalidate(sphere)` fit
  inside `ShadowBaseNode` or need a full custom lighting node is unmeasured.
- **Bundle size.** 600 KB → 1.79 MB uncompressed for the three build alone,
  against a browser game's first-load budget.
- **What the prewarm COUNT becomes** under pipelines, and whether it catches
  the same event class.

## 8 · Prior art — what not to rebuild

Surveyed 2026-08-04. The headline: **most of what route C would need is
already written, and a surprising amount of it is sitting unused in
`web/node_modules/`.** Reproduce with:

```sh
cd web/node_modules/three/examples/jsm
grep -rl "from 'three/tsl'" .    # 47 node-ready addons ship with r178
```

### 8.1 · Shipped with the three we already have

| module | lines | what it answers |
|---|---|---|
| `csm/CSMShadowNode.js` | — | **cascaded shadows as a TSL node.** The closest prior art to our clipmap |
| `tsl/shadows/TileShadowNode.js` | 456 | tiled shadow maps, one light+camera per tile, worked example of `ShadowBaseNode` |
| `objects/SkyMesh.js` | — | Preetham analytic sky dome, `turbidity` / `rayleigh` / `mieCoefficient` / `mieDirectionalG` as uniforms |
| `tsl/utils/Raymarching.js` | 70 | `RaymarchingBox` — box-intersection + stepped loop |
| `tsl/display/GTAONode.js` | 522 | ground-truth AO, the `threejs-screen-space-ambient-occlusion` skill's subject |
| `tsl/display/Lut3DNode.js` | 109 | 3D LUT grading, the `threejs-exposure-color-grading` skill's subject |
| `tsl/display/{Bloom,Denoise,SSR,TRAA,SMAA,FXAA,DepthOfField,MotionBlur}Node.js` | — | 30 post nodes total |
| `tsl/lighting/TiledLightsNode.js` | 422 | tiled light culling |
| `objects/{Water,Water2}Mesh.js` | — | node water |
| `transpiler/` | 4,000 | **GLSL → TSL transpiler** (§8.2) |

Two of these are direct hits on standing work:

- **`CSMShadowNode`** is the node system's supported answer to "several shadow
  maps composed into one lighting term" — exactly the mechanism our
  `lights_fragment_begin` patch hand-builds. It extends `ShadowBaseNode` and
  imports `WebGLCoordinateSystem` explicitly, which corroborates §2.1's
  clip-space-Z hazard: three's own cascade code treats the two coordinate
  systems as a thing you must branch on, not a detail that ports silently.
- **`SkyMesh`** covers the gradient-dome half of `ART.md`'s sky. It does
  **not** cover the other half — it is analytic and cloudless, and `ART.md`
  is explicit that *"cloudless cannot reach a p90 of 189 with a sky mean of
  143."* SkyMesh plus a cloud layer, not SkyMesh instead of one.

*Doc trap, verified:* `SkyMesh.js:15` carries a verbatim copy of `Sky.js`'s
note — *"this class can only be used with `WebGLRenderer`. When using
`WebGPURenderer`, use `SkyMesh`"* — inside `SkyMesh` itself. It is a stale
copy-paste in three's docs; the file imports `three/webgpu` and `three/tsl`
and is the node version. Do not read that line and conclude the opposite.

### 8.2 · The transpiler is aimed at exactly our §2.2

`examples/jsm/transpiler/` — 4,000 lines, GLSL decoder → AST → TSL encoder.
Its own docstring:

> `Transpiler` can only be used to convert GLSL into TSL right now. It is
> intended to support developers when they want to migrate their custom
> materials from the current to the new node-based material system.

That is our 11 `onBeforeCompile` sites described by three's own tooling. It
will not port the *splice* — there is no chunk to splice into — but the GLSL
bodies (`gatesClipmapShadow`, the splat blend, the octave stack in
`materials.js`) are the bulk of the typing, and this converts them
mechanically. It downgrades §2.2 from "rewrite" to "convert, then rewire".

The forum answer for lighting-chunk overrides specifically is thinner than
the tooling: the one on-point thread is about alpha clipping, not lighting.
`CSMShadowNode` and `TileShadowNode` are the real documentation.

### 8.3 · Third-party, and honest about status

| project | licence | stack | verdict |
|---|---|---|---|
| **`takram/three-atmosphere`** | MIT | **GLSL / WebGL today**; TSL+WebGPU "planned" | Sky, Stars, `AerialPerspectiveEffect`, `SunDirectionalLight`, `SkyLightProbe`, precomputed-texture generator. Usable on **route A**, now |
| **`takram/three-clouds`** | MIT | same — GLSL, WebGPU planned | volumetric clouds, same caveat |
| **`CK42BB/procedural-clouds-threejs`** | MIT | WebGPU raymarch **+ WebGL2 fallback**; shaders given in GLSL, WGSL *and* TSL | a Claude Code skill, not a library — guidance in both dialects, so it reads on either route |
| **`SkyeShark/SeedThree`** | MIT | WebGPU-first, **but the generator is renderer-agnostic** | **the one third-party item usable on route A today** — see below |
| **`SkyeShark/Eanpa-Sky`** | MIT | TSL/WebGPU, non-modular | never importable, but a **clean donor to copy the clouds from on either route** — §8.3.1. Audio is out of bounds: four xeno-canto recordings are **CC BY-NC-SA** |
| three.js `webgpu_volume_cloud` example | MIT | TSL | 3D-texture raymarch, the minimal shape |

**The reframe worth stating plainly:** the two most mature open atmosphere
and cloud libraries are *still GLSL/WebGL*, and their WebGPU support is
planned rather than shipped. So the ecosystem does **not** currently punish
route A on our actual open gap. It punishes route A on *skills* — the granted
pack, and the direction of three's own additions above — not on available
third-party sky code. That weakens the §5 tax in the near term and does not
weaken it at all in the long term.

**`SeedThree`, examined 2026-08-04, because it is the exception.** MIT,
73 stars / 10 forks / 6 open issues, 71 source and doc files, ~665 KB. It
generates trees and desert plants from Weber–Penn and an L-system
dichotomous generator, ten species, with an LOD chain down to baked
billboard impostors.

It is labelled WebGPU-first, and the label is misleading in our favour. The
generator imports `three/webgpu` for `Vector3`, `Quaternion`,
`BufferGeometry`, `Box3`, `Group`, `Mesh` — core classes identical in both
builds — and `weber-penn.js` says why in a comment:

> Import three math from `'three/webgpu'` to avoid mixing with the bare
> `'three'` entry (which the WebGPU docs warn against).

A convention, not a capability. Only the *material* path is TSL
(`leaf-cards.js`, `yucca-leaves.js`); `branch-mesh.js` and `export-glb.js`
carry none. And its own API README states the consequence:

> No dev server, no browser, and (for pure geometry) no GPU.

`generate()` runs headless in Node, and `exportGLB()` deliberately
substitutes **plain standard materials** so the output opens anywhere. So the
usable shape on route A is *offline*: generate species headless, export
`.glb`, load through `GLTFLoader` into the WebGL client. **The renderer
question never arises.** What we would not get is its foliage shading —
translucency, dome-normal, per-instance wind — which is TSL and would be
re-authored in GLSL against `threejs-procedural-vegetation`.

That matters because `ART.md` scores silhouette: *"if the silhouette is the
wrong shape against the sky, no material work will save it."* Geometry is
exactly the half this delivers without a migration.

**Maturity, stated honestly.** `0.1.0-alpha`, README says *"early and rough
in places. Expect sharp edges."* `private: true`, so not on npm — vendor or
fork, no `npm install`. Pinned to `three ^0.184.0` against our `0.178.0`.
Created 2026-07-04, **last code push 2026-07-06** — two days of work, then a
month quiet; treat it as a snapshot to vendor, not a dependency to track. It
is `© SkyeShark and Claudes`, i.e. agent-built like this repo, so it gets
read before it gets trusted. The `weber-penn.js` header is candid about its
own scope (*"pragmatic subset… nSegSplits is approximated by BaseSplits
multi-leader trunks for now"*), and the named parameters — `flare`, `nTaper`,
`ratioPower`, `baseSplits`, pipe-model radius law, AttractionUp tropism,
phyllotactic placement — are the real paper's. It is not vapour.

**Its audio is not usable and is worse-documented than Eanpa-Sky's.**
`assets/audio/` ships bird recordings (wren, crow, mallard, roadrunner) whose
`README.txt` cites xeno-canto and *"XC ids in the original Downloads
filenames"* but **states no licence for any file**. xeno-canto is
per-recording licensed and includes plenty of NC. Code and geometry only;
resolve nothing by assuming.

### 8.3.1 · `Eanpa-Sky`, judged as a donor for the clouds specifically

An earlier draft of this doc said Eanpa-Sky "can only ship ideas into a
head." **That was wrong on both halves and is corrected here.** It is MIT, so
copying is permitted outright, and `engine/sky_system.js` was read properly
(2,840 lines) rather than judged by its file size. As an *import* it is still
hopeless — an IIFE on `globalThis.THREE`, no exports. As a **donor to copy
from**, the clouds are unusually clean, and the most valuable parts are the
least TSL-bound.

Its own section banners give the map:

| lines | section | portability |
|---|---|---|
| 111–168 | cloud presets | plain data |
| 249–431 | shared noise | **plain JS — no TSL at all** |
| 431–970 | cloud density (two-stage erosion) | chained arithmetic |
| 970–1065 | light/froxel cache | tied to their frame graph — drop |
| 1065–1730 | cloud dome material + march | 2 loops |

Four things make it rippable, all verified in the file:

1. **The noise is generated at runtime, in plain JavaScript.** A 256²
   value-noise `DataTexture` from an LCG on a fixed seed, plus a 512²
   *tileable 1/f weather map* from a 5-octave fbm with its hash written out.
   No shipped asset, no licence surface, deterministic, and **not TSL** — it
   is a loop filling a `Uint8Array`. It copies verbatim into `textures.js`.
   The comment explaining *why* 1/f and not white noise (white gives fuzz,
   not masses; 42% of samples saturating the gain) is worth as much as the
   code.
2. **The math is chained-method arithmetic**, which maps to GLSL nearly
   1:1 — `a.mul(b)` → `a * b`, and `clamp`/`smoothstep`/`pow`/`exp`/`mix`
   are the same functions. In 2,840 lines there are **2 `Loop(`** (the camera
   march and a storm march) and 19 `If(`. In GLSL those are `for` and `if`.
   TSL→GLSL is the easy direction; three's own transpiler (§8.2) only goes
   the other way, and this does not need it.
3. **The ring entanglement is opt-in and strips at authoring time.**
   `RING_R = opts.ringCurve ?? 0` gates all 48 megastructure references;
   `ringSlopeAt` returns `float(0)` when it is zero, and `atmoHeight` has a
   clean non-ring base path (ordinary planet-curvature height).
4. **It is already written in the form GLSL wants.** Every texture fetch
   carries an explicit LOD, with the reason in-line: *"Chrome/Tint forbids
   implicit-derivative sampling in divergent flow."* That is the same rule
   GLSL has about derivatives in non-uniform control flow, already obeyed.

What is *not* free: the lighting model is standard published work (Beer's
law, the powder term, numerical Mie phase, Hillaire accumulation — Schneider
and Hillaire, cited in its own header), so what Eanpa-Sky actually donates
there is **tuning, not technique**. The froxel cache goes. The `Deno.env`
knobs (`RINGSHEAR` et al.) are knobs outside any registry and must land in
`DECISIONS.md` §open or as stated defaults, never as env reads.

**Route independence.** None of the four points above needs WebGPU. A GLSL
cloud dome on route A can take the noise generators verbatim, the presets and
TOD table as data, and the density function as a translation. **Ripping the
clouds is not a reason to migrate**, and it is available on the stack we have.

**Licence posture, which changes if we do this.** MIT permits shipping the
code, unlike the guidance-only clause the skill packs carry (that clause is
our convention for them, and `CLAUDE.md` already notes it is a licence
statement rather than a usage limit). Copying means **retaining the MIT
notice and copyright in the file that carries the code** — a header block at
the donor site, plus the `CLAUDE.md` credit entry — not a bullet alone. Get
that wrong and the cheapest item in this doc becomes the most expensive.

**The real cost is not translation.** A raymarched dome is per-fragment march
work, this box gates on a software rasterizer, and `CLAUDE.md`'s one-owner
law puts clouds in the same iteration as sky, fog, exposure and tone map.
The translation is a day; the budget and the single ownership are the item.

### 8.4 · Nothing here changes the §2.1 verdict

No prior art was found for the probe harness. Public three.js visual testing
is puppeteer + image-snapshot against golden PNGs — the opposite discipline
from ours, which renders A/B pairs in one process and measures deltas so each
probe's zero point is *measured rather than argued*. `readRenderTargetPixelsAsync`
plus explicit render targets remains hand-written work, 12 probes and 43
`readPixels` sites of it, and it stays the migration's largest single item.

### 8.5 · Credit — already due, and the clause is not the skill packs'

`Eanpa-Sky` and `SeedThree` were read as code during this survey, so
`CLAUDE.md` § Third-party credit carries them as of 2026-08-04. **Do not
copy the skill packs' wording onto them.** That clause is *guidance only, no
code ships* — our own convention for a pack we chose not to vendor. These are
MIT, which permits shipping, so the obligation is different and stricter in
one specific way: **the notice and copyright travel with the code, in the
file that carries it.** A bullet in `CLAUDE.md` is not sufficient on its own
for a copied block; it is the index, not the licence.

Two copy paths are open and both need that header at the donor site —
SeedThree as vendored `.glb` output (§8.3) and Eanpa-Sky's cloud noise,
presets and density function (§8.3.1). Neither has been taken yet.

Everything else surveyed here — the `three/addons` modules of §8.1–8.2 — is
already a dependency we ship under three's own MIT licence, and needs
nothing new.
