#!/usr/bin/env node
// Browser smoke — the gate that would have caught the two bugs of 2026-07-31.
//
// Every other gate tests the client's LOGIC: client-wasm unit tests, the node
// bridge smoke, and server/tests/client_loop.rs (the client core against a real
// ShardCore over real datagrams). All honest, all native or node — so the JS
// boot path in web/src/*.js was never executed by anything. With 46 tests green
// and a judge PASS, the browser client could not start at all:
//
//   1. WasmViews.refresh() captured ex.memory.buffer, then called a ptr getter
//      in the same expression. The getter allocates on first call, grows wasm
//      memory, and detaches the captured buffer → "Cannot perform Construct on
//      a detached ArrayBuffer" before a single packet moved.
//   2. The terrain worker's `ex` is assigned inside an async handler, and
//      async onmessage does not serialize messages, so build requests arrived
//      while loadWasm was still in flight → "Cannot read properties of null".
//      The far mesh still rendered, so a screenshot looked fine.
//
// So this gate asserts what a frame cannot: the client REACHES THE WORLD, and
// NOTHING throws while it plays. Bug 2 only shows up as a page error, which is
// why zero-page-errors is an assertion and not a warning.
//
// It runs TWO browser contexts, because M0's exit condition (DESIGN.md §11) is
// two clients seeing each other walk. The shard gets `dev_spawn` (DECISIONS.md
// §open) so both land on one point — normal scatter is 224–1,824 m on a
// 2,048 m island, far outside the 176 m AOI enter. Each page then asserts the
// OTHER page's remote displaces while its key is held: `remotes 1` alone can't
// tell a frozen remote from a live one, movement can.
//
// It also guards the dev gate on the client's dev affordances, because that
// gate has no other home: `__gatesDebug.setView` (the capture harness's camera
// hook) is installed only when the shard's welcome says `dev`, and the only
// place that if-statement actually runs is a browser. So this gate boots a
// SECOND shard with no dev override — a public shard's config — and asserts the
// hook is absent there and present, aiming, on the dev one. That third tab boots
// only after the first two are CLOSED: on a four-core box with no GPU the join
// time is a function of how many renderers are live (0.4 s / 34 s / 55 s for
// one / two / three), and running it beside the other two is what put this wall
// in the red on 2026-08-01. It shares nothing with them, so it waits for them.
//
// It is also where the LIGHTING rig is gated, for the same reason: a shadow map
// only exists inside a renderer. The flags (map enabled, key casting, a tone
// map that is not NoToneMapping) are the cheap half; the assertion that matters
// renders the live scene twice per sample yaw — key light casting, then not —
// reads both frames back off the drawing buffer and requires the shadow pass to
// have actually taken pixels down. Every flag can be true while the image is
// unchanged: a bias that pushes every sample past its caster, a coverage box
// parked somewhere else, casters that never got castShadow. Only a frame knows.
// The same two renders also count the shadow pass's draw calls, which is what
// makes DESIGN §9's < 300 calls / < 1.5 M tris budget assertable here at all.
//
// Any missing dependency is a loud failure, never a silent skip.

import { spawn } from "node:child_process";
import http from "node:http";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { createRequire } from "node:module";
import { measureReference, REFERENCE_FRAMES } from "./reference_bar.mjs";

const root = path.join(path.dirname(fileURLToPath(import.meta.url)), "..");
const DIST = path.join(root, "web/dist");
const SHARD = path.join(root, "target/release/shard");
const PORT = Number(process.env.BROWSER_SMOKE_PORT || 8934);
// UDP port the temp shard binds; overridable so two smoke runs (or a smoke
// beside a dev shard) don't fight over 4433.
const WIRE_PORT = Number(process.env.BROWSER_SMOKE_WIRE_PORT || 4433);
// The public-config shard (no dev_spawn) the dev-gate check joins.
const PUBLIC_WIRE_PORT = Number(process.env.BROWSER_SMOKE_PUBLIC_WIRE_PORT || WIRE_PORT + 1);
const JOIN_TIMEOUT_MS = Number(process.env.BROWSER_SMOKE_TIMEOUT_MS || 60000);
// How the join is WATCHED inside that window — the instrument, not the budget.
// A look is a `page.evaluate`, which has to be scheduled on the tab's own main
// thread; with three tabs live on this box one measured 20 s. So up to 4 may be
// outstanding instead of each waiting for the last to come back, and 250 ms is
// the FLOOR on the gap between launches — not a cadence, since a look settling
// also frees a slot immediately. See `join()`.
const JOIN_POLL_MS = 250;
const JOIN_POLL_INFLIGHT = 4;
const PLAY_MS = Number(process.env.BROWSER_SMOKE_PLAY_MS || 6000);
// Separation the chat assertion walks the two tabs to before claiming a local
// line is out of earshot. Comfortably past the 20 m radius (DECISIONS.md
// §open, "local chat") so interpolation lag on a shared box can't put the
// listener back inside it.
const CHAT_APART_M = Number(process.env.BROWSER_SMOKE_CHAT_APART_M || 30);
// Seed and point are guarded natively: sim-core world::tests asserts this
// exact spawn is walkable at this exact seed, so worldgen drift fails there
// first, with a message, instead of here as a mystery.
const SEED = 20260731;
const DEV_SPAWN = "1024,1024";
// Held-walk displacement floor, metres planar. Walk speed is 3 m/s over
// PLAY_MS of held key (~18 m); 2 m stays green under heavy same-box load
// while still failing a frozen or never-updated remote outright.
const MOVE_MIN_M = 2;
// The aim the dev hook is driven to: yaw pi/2 faces +X (sim-core yaw_lut —
// 0 faces +Z, increasing turns toward +X), pitch below level so the clamp is
// not what is being measured. Walking after it must carry the player east.
const AIM_YAW = Math.PI / 2;
const AIM_PITCH = -0.3;
const AIM_EPS = 1e-3;
// Before the aimed walk is measured, the PREVIOUS walk has to be all the way
// out of the player's position. `own` is the client's own predicted position,
// republished on the 250 ms HUD timer; the chat section walks both tabs apart
// with held keys, and on this box the tail of that walk keeps arriving after
// the key is up. A fixed settle wait was what stood here, and on 2026-08-01
// 15:51 it was not enough: the aimed walk measured [5.01, 11.91] m — the +Z
// leg of the CHAT walk still draining while the +X leg was already running,
// against an assertion that requires the walk to be east-dominant.
//
// The question to ask is "is the player still WALKING", not "is the player
// perfectly still". Those are different, and asking the second one is how the
// first attempt at this reddened the wall itself: a client with no key held
// does not come to a dead stop on a starved box — reconciliation keeps nudging
// the predicted position — so an epsilon on raw displacement was a test the
// client can fail while behaving correctly (measured: 0.13 m of residual left
// standing after 25 rounds, against a 0.05 m bar).
//
// What separates the two is SPEED, and the two cases are not close. A stale
// input backlog drains at the walk speed the sim runs on, 3 m/s, until it is
// empty; the residual left over after it measures 0.02-0.30 m/s across the six
// readings taken while this was built. The bar sits in that gap, and it is set
// from BOTH sides, because both sides are failures:
//
//   under it — 6x below a walk, so a draining backlog cannot clear it;
//   over it  — 1.7x above the worst residual seen, so a correctly behaving
//              client is not asked to reach a stillness it never reaches. That
//              is the mistake that reddened the first attempt at this.
//
// And it is bounded by what the assertion downstream can absorb, which is the
// number that actually matters: the residual runs +Z, the walk is measured
// east-dominant, and a starved box shrinks the walk (dx has come in at 5.4 m)
// at the same time as it lengthens the drain. At 0.5 m/s the residual can add
// 3 m of dz over the 6 s walk, against a dx that has never measured under 5.4.
// A 1.0 bar would allow 6 m and lose that margin.
//
// Two CONSECUTIVE intervals must clear it, so a decaying tail cannot be caught
// at a trough.
const AIM_REST_SPEED_MPS = 0.5;
const AIM_REST_CLEAR_RUNS = 2;
// Budget denominated in what is actually being waited for — fresh publishes —
// with a wall deadline behind it so a client that has stopped publishing at all
// fails loudly here instead of hanging. Under the worst starvation this gate
// has recorded (6 publishes in 10 s) the deadline still buys ~15.
const AIM_REST_PUBLISHES = 16;
const AIM_REST_DEADLINE_MS = 25000;
const AIM_REST_POLL_MS = 400;
// --- the lighting gate (DECISIONS.md §open, "lighting v0") ------------------
// The shadow probe sweeps four yaws so the assertion does not depend on the
// player having walked to a spot with a caster in one particular direction,
// and looks slightly down so the ground the shadows land on fills the frame.
const SHADOW_PROBE_YAWS = [0, Math.PI / 2, Math.PI, (3 * Math.PI) / 2];
const SHADOW_PROBE_PITCH = -0.45;
// The near ring streams one chunk at a time; probing before it lands would
// score a frame with most of its casters missing.
const SHADOW_SETTLE_MS = Number(process.env.BROWSER_SMOKE_SHADOW_SETTLE_MS || 4000);
// A pixel counts as shadowed when the shadow pass took its luma down by more
// than this out of 255 — comfortably above SwiftShader's dither/AA noise
// between two renders of an identical frame, well below a real shadow (whose
// mean delta on this scene runs in the tens).
const SHADOW_PROBE_MIN_DELTA = 6;
// Floor on the darkened share of the sweep. Raised from 0.015 by the slice
// that made the GROUND cast: with the terrain culled out of every depth pass
// (three casts a FrontSide material from its back face, and a heightfield has
// none turned at the sky) the pinned spawn measured 10.5%, worst yaw 3.6% —
// all of it from the scatter and the other tab's avatar. With hills casting it
// measures 24.0%, worst yaw 20.4%, stable across runs. So these floors now
// bite on the real failure and not only on the total absence of a rig: a
// terrain material that forgets `shadowSide` scores 10.5% and 3.6%.
//
// **Unchanged by lighting v1, and worth knowing why they could not have
// been.** Shadow area from a vertical caster is `height x cot(elevation)`, so
// these two floors are only meaningful at a fixed sun: raising it to 45° takes
// the same intact rig to 8.0% aggregate and 0.6% on the yaw that looks toward
// the sun. That was built and measured, and it is the reason the sun did NOT
// move (`scene.js`, SUN_ELEVATION). Had it moved, these numbers would have had
// to come down by 3x and 16x — which is exactly the shape of a gate being
// quietly relaxed to fit a change, and exactly why the attribution leg below
// was written instead: it measures the mutation these were hand-calibrated
// against, every run, and the sun's elevation cannot flatter it.
const SHADOW_MIN_FRACTION = Number(process.env.BROWSER_SMOKE_SHADOW_MIN || 0.15);
// And a floor on EVERY yaw, which is the assertion that actually pins the
// WORLD as a caster. Dropping castShadow from the terrain and the scatter
// pools still cleared the aggregate above: the other tab's avatar stands on
// the shared spawn point and, under a 21° sun, throws a long enough shadow
// across a downward-pitched frame to score 6% on its own — from two of the
// four yaws, with the other two at exactly 0.0%. Worst real yaw was 2.8% then
// and is 20.4% now that the ground itself casts, which is what let the floor
// below move from 1% to 10%.
const SHADOW_MIN_FRACTION_PER_YAW = Number(process.env.BROWSER_SMOKE_SHADOW_MIN_YAW || 0.1);
// --- and the assertion the two above were a proxy for -----------------------
// `shadowProbe` now renders a fourth leg per yaw: the same frame with
// `castShadow` off everything that is not another player. The difference
// between that and the ship frame is shadow the WORLD cast — the terrain ring,
// the far mesh, the scatter pools — attributed by construction rather than
// inferred from an aggregate.
//
// This is the mutation the floors above were calibrated by hand against on
// 2026-08-01, taken every run. Its floor cannot be cleared by the avatar
// standing on the shared spawn, because the avatar is the thing it holds
// fixed, and — unlike an area floor — it survives the sun moving: at any
// elevation, "the hills and the pines account for the shadow in this frame"
// is either true or the world stopped casting. Measured 3.12% at a 28.6° sun
// and 4.79% at 45° while the aggregate area floor was failing at both, which
// is the property that makes it worth having.
const SHADOW_MIN_WORLD_FRACTION = Number(process.env.BROWSER_SMOKE_SHADOW_MIN_WORLD || 0.015);
// And on the sweep's best direction rather than on every one of them: a
// vantage looking toward the sun sees the lit side of everything and has no
// shadow in it to attribute, at any elevation. What this catches is a world
// that stopped casting in EVERY direction.
const SHADOW_MIN_WORLD_BEST_YAW = Number(process.env.BROWSER_SMOKE_SHADOW_MIN_WORLD_YAW || 0.02);
// The mutation has to actually remove something. A world-caster list that came
// back empty would measure zero world shadow (reading as catastrophe) and a
// list that swallowed the avatars too would measure all of it (reading as
// perfection); neither is the leg doing its job. The intact scene submits 25
// draws to the shadow pass, of which 2 are the avatar's.
const SHADOW_MIN_WORLD_CASTERS = 8;
// Same failure from the other side, counted rather than sampled: the seven
// scatter pools are frustumCulled=false, so a rig where the world casts
// submits at least them to the shadow pass. That mutation submitted 2 (the
// avatar's two meshes); the intact rig submits 25.
const SHADOW_PASS_MIN_CALLS = Number(process.env.BROWSER_SMOKE_SHADOW_MIN_CALLS || 8);
const SHADOW_MIN_MAP_PX = 1024;
// --- the tonal gate (DECISIONS.md §open, "lighting v1") ---------------------
// Every lighting assertion above this line is a DIFFERENCE: the shadow probe
// counts pixels the shadow map darkened, the surface probe counts pixels the
// field moved, the prop probe divides a field by what it was laid on. All of
// them are blind to an offset, and the defect the visual judge has returned on
// every capture is an offset — the whole image sitting a stop and a half under
// `Rust Images/`, which it scores us against as an absolute bar.
//
// So this one has no toggle and no baseline. It measures where the image IS,
// in Rec.601 luma percentiles over the whole frame, at the same six vantages
// the capture harness shoots — and it compares them to the same statistic read
// off the reference frames in this same run (`ci/reference_bar.mjs`).
//
// The six vantages are copied from `art/capture.mjs`'s VANTAGES (yaw, pitch),
// which lives in the loop harness and is checksummed between passes. They are
// copied rather than imported for exactly that reason: the harness is outside
// this repo and a gate may not depend on a file the repo cannot see. If they
// ever disagree, this gate is measuring a register at framings the judge does
// not score, which is a weaker claim but not a false one — the register is a
// property of the light rig, not of where the camera points.
const TONAL_VIEWS = [
  { label: "01-horizon-north", yaw: 0, pitch: 0 },
  { label: "02-horizon-east", yaw: Math.PI / 2, pitch: 0 },
  { label: "03-canopy-up", yaw: 0, pitch: 0.9 },
  { label: "04-ground-down", yaw: 0, pitch: -0.8 },
  { label: "05-held-level", yaw: Math.PI / 4, pitch: -0.15 },
  { label: "06-hud", yaw: Math.PI, pitch: 0 },
];
// The floors. Plain consts, no env override, so `ci/knob_registry.mjs` pins
// them to their §open declarations — the discipline `PROP_MIN_VALUE` started.
//
// Where they come from: `ci/reference_bar.mjs` measures the six outdoor
// daylight frames in `Rust Images/` and reports the MEDIAN p10 40 · p50 91 ·
// p90 170. The capture that opened this item measured p10 41 and p90 70 on
// ours — the shadows were already sitting on the reference and the entire top
// of the image was missing. So p90 is the load-bearing floor and p50 is the
// one that stops it being reached by a single blown highlight.
const TONAL_MIN_P90 = 150;
const TONAL_MIN_P50 = 70;
// …and the other side of it, because "make it brighter" is not a light rig.
// A scene lifted uniformly would clear both floors above and read as fog. The
// darks have to STAY dark, and the reference says where: p10 40.
const TONAL_MAX_P10 = 60;
// The range those two imply, asserted directly so a frame cannot satisfy both
// ends on different views and neither on any one of them.
const TONAL_MIN_RANGE = 90;
// Banding in the sky dome. The judge counted "131 distinct values over 360
// rows, longest flat run 11 px, no dither" and called the gradient posterized.
//
// The obvious instrument — count distinct luma levels — was written first and
// is wrong, in a way worth recording: the count is bounded above by how many
// levels the GRADIENT spans, so the horizon-north vantage, whose visible sky
// is a 16-level band, scores 16 however perfectly it is dithered. Measured
// that way our dome read 16-44 against a reference of 232 and the number said
// nothing.
//
// So the wall is on runs, which nothing bounds: the share of horizontally
// adjacent dome pixels whose quantized value DIFFERS. An undithered ramp
// breaks only where it crosses a quantization boundary — a few percent of
// pairs, in flat runs tens of pixels long. A ramp with noise of a level or
// so under the quantizer breaks at roughly half of them — the calibration
// model for this floor; the shipped dither actually delivers 0.5–2.5 levels
// across the dome (scene.js, SKY_DITHER), which only breaks MORE pairs.
const SKY_MIN_BREAK = 0.25;
// …and the longest identical run in a row of sky, directly, as a backstop
// against gross banding the share above could in principle average away. It
// is the secondary of the two: the break fraction is what bites (an
// undithered ramp scores a few percent against a 25% floor), and this is set
// at 2x the measured worst (22 px, at the frame edges where the dome is
// flattest) rather than at the judge's 11, because a hash dither produces
// occasional long runs by chance and a ceiling that trips on luck is a
// flaky gate, not a strict one.
const SKY_MAX_RUN = 48;
// How much of a view has to BE sky before its dome statistics mean anything.
const SKY_MIN_FRACTION = 0.02;
// And how far above the frame's own median the dome has to sit. The sky is
// the only surface in this scene that is not a diffuse reflector, so it is
// where the image's top decile comes from; a dome level with the ground it
// lights is an image with no highlight, which is what the first cut measured
// (sky 142 against a median of 138).
const SKY_MIN_OVER_GROUND = 25;
// The sun. The disc must be brighter than the sky it sits in (levels of luma
// over the dome background well away from it), it must land where the KEY
// LIGHT says it is (the camera aims down `_toSun`, so the disc belongs at the
// principal point), and it must not have grown into a hemisphere-wide wash.
const SUN_MIN_PEAK_OVER_SKY = 30;
const SUN_MAX_OFFSET_PX = 24;
const SUN_MAX_SATURATED = 0.02;
// --- the clipmap gate (DECISIONS.md §open, "shadow clipmap v0") -------------
// Two probes, because the slice makes two separate claims: that the coarse
// levels DARKEN pixels the near level cannot reach, and that those pixels are
// genuinely outside the near level's box rather than just far-ish.
//
// The camera is aimed perpendicular to the sun's bearing, read off the scene
// rather than pinned here. Light-space X is the horizontal axis across the
// sun, and it is the only axis on which the near level's 80 m half-width is
// 80 m of GROUND: along the sun's bearing the same box reaches 80/sin(21°) ≈
// 227 m, so a "past 80 m" claim measured there would be worth nothing.
// Lift the viewpoint and pitch it down, so the far band is most of the frame
// rather than a strip under the horizon (from eye height the same probe
// measured 0.10% — real shadow, at 27 mean Δluma, on almost no pixels). The
// clipmap's centre stays on the player, so the lift changes the sample size
// and nothing else.
const FAR_SHADOW_HEIGHT_M = 80;
const FAR_SHADOW_PROBE_PITCH = -0.42;
// Push the near plane out and narrow the frame, so nothing inside the near
// level's reach is drawn at all. The probe then MEASURES the resulting
// distance (min |light-space x − centre| over all eight frustum corners) and
// hands it back; the assertion below is on the measurement, not on these.
const FAR_SHADOW_NEAR_M = 120;
const FAR_SHADOW_FOV_DEG = 28;
const FAR_SHADOW_MIN_DELTA = 6;
// Floors on the share of that far-only frame the coarse levels darken. The
// pinned spawn measures 0.78% (per yaw 1.3 / 0.3), so these leave ~3x of
// headroom; the failure they guard — a rig whose shadows stop at the near
// box, which is exactly what shipped before this slice — scores 0.00%, and
// that zero is measured rather than assumed (see the control below).
// Small in absolute terms because most of a downward frame at this range is
// lit ground: what is asserted is that the shadow is THERE, and the mean
// Δluma the failure message prints (~29 of 255) says it is not noise.
const FAR_SHADOW_MIN_FRACTION = Number(process.env.BROWSER_SMOKE_FAR_SHADOW_MIN || 0.0025);
const FAR_SHADOW_MIN_FRACTION_PER_YAW = Number(
  process.env.BROWSER_SMOKE_FAR_SHADOW_MIN_YAW || 0.001,
);
// A coarse level may only ever REMOVE light. Any pixel the clipmap makes
// BRIGHTER means the extra levels are lighting the scene instead of
// shadowing it — the failure a zero-intensity level exists to prevent, and
// the one that would otherwise show up as a washed-out frame nobody gates.
const FAR_SHADOW_MAX_LIFTED_FRACTION = 1e-4;
// --- the horizon-caster gate (DECISIONS.md §open, "the horizon casts") ------
// Everything that cast before this slice lived in the near ring: the 5x5x64 m
// chunk box, its scatter, pieces and players. What this slice adds is the far
// mesh, casting everywhere the ring is NOT — so the measurement that proves it
// is not a camera pushed out to some distance, it is the caster taken away.
// The horizon probe's own sweep: four yaws, lifted so the ground past the ring
// is most of the frame rather than a strip under the horizon, and pitched down
// like the other two. There is no near-plane trick here and none is needed —
// the caster it removes is, by construction, only outside the ring.
const HORIZON_PROBE_YAWS = [0, Math.PI / 2, Math.PI, (3 * Math.PI) / 2];
const HORIZON_PROBE_PITCH = -0.42;
const HORIZON_PROBE_HEIGHT_M = 80;
// Floors from the measurement with the usual headroom; the failure they guard
// — a far mesh that receives and never casts, which is what shipped before
// this slice — scores exactly 0, and the probe's control says its zero point
// is a real zero. Measured 0.54% of the sweep at mean Δluma 45-48 (which is
// not a marginal darkening: it is the same order as the near shadows).
const HORIZON_MIN_FRACTION = Number(process.env.BROWSER_SMOKE_HORIZON_MIN || 0.0015);
// Deliberately NOT a floor on every yaw, unlike the near shadow probe. Past
// the ring the only caster is the ground itself — the scatter stops at the
// ring's edge — so a direction casts only where the island has relief steep
// enough to shade at a 21° sun, and which side of the spawn that is, is
// worldgen's fact and not the caster's. Measured 1.57% / 0.57% on two of the
// four yaws and ~0 on the other two. So: how many directions show it, with
// room for one of the two real ones to be a thin frame.
const HORIZON_MIN_DIRECTIONS = 2;
const HORIZON_MIN_FRACTION_PER_YAW = Number(process.env.BROWSER_SMOKE_HORIZON_MIN_YAW || 0.001);
// The island is this wide (TERRAIN.md §4), so a hole half-extent at or past it
// is not a ring footprint — it is the probe's own suppression left behind.
const ISLAND_M = 2048;
// three's FrontSide. The ground must name its shadow side: three's default for
// a FrontSide material is to cast from the BACK face — the right answer for a
// closed solid and the wrong one for a heightfield, which has no back face
// pointed at the sky and is therefore culled out of the depth pass entirely.
const THREE_FRONT_SIDE = 0;
// DESIGN §9's client budget, gated for the first time here. Both counts are
// per rendered frame and INCLUDE the shadow pass, which is the point: a
// second pass over every caster is the obvious way to eat this budget.
const DRAW_CALL_BUDGET = 300;
const TRIANGLE_BUDGET = 1_500_000;
// --- the materials gate (DECISIONS.md §open, "materials v0") ----------------
// Same three-part shape as the lighting gate above, for the same reason: the
// flags say a material was configured, the census says what the shader is
// actually FED, and only a pair of frames says what reached the image.
const SURFACE_PROBE_YAWS = [0, Math.PI / 2, Math.PI, (3 * Math.PI) / 2];
// Steeper than the shadow probe's: this one is about the GROUND's material,
// so the frames it scores should be mostly ground in every direction.
const SURFACE_PROBE_PITCH = -0.7;
// A pixel counts as moved when the field took its luma up or down by more
// than this out of 255 — the same floor the shadow probe uses, and for the
// same reason (SwiftShader's dither noise between two renders sits under it).
const SURFACE_PROBE_MIN_DELTA = 6;
// Floors on the swept pixels the procedural field moves. The pinned spawn
// measures 12.8% (per yaw 2.4 / 6.8 / 8.4 / 33.7), stable to the decimal
// across runs; the failure they guard — a material whose field contributes
// nothing — scores exactly 0.
const SURFACE_MIN_FRACTION = Number(process.env.BROWSER_SMOKE_SURFACE_MIN || 0.04);
const SURFACE_MIN_FRACTION_PER_YAW = Number(process.env.BROWSER_SMOKE_SURFACE_MIN_YAW || 0.01);
// And the half that is hard to fake, because the two above were SHOWN not to
// bite on their own: collapsing the noise scales so the whole ring lands in
// one lattice cell moved 66–96% of the pixels — a uniform darkening — and
// sailed past both floors. Microstructure lightens some pixels and darkens
// others; a constant field, a global tint or an exposure slip cannot. That
// mutation scored +0.00% up on two of the four yaws against a worst real
// yaw of +0.5%.
const SURFACE_MIN_DIRECTIONAL = Number(process.env.BROWSER_SMOKE_SURFACE_MIN_DIR || 0.002);
// The splat weights the shader is fed, over the streamed near ring. Constant
// or one-hot weights render a perfectly convincing single material, so:
// at least this many identities must hold real ground, and this much of the
// ring must be a genuine two-identity blend rather than a hard biome cell.
const SPLAT_MIN_IDENTITIES = 2;
const SPLAT_IDENTITY_SHARE = 0.01;
// Measured at the pinned spawn: spreads 1.00 / 1.00 / 0.00 / 0.16, deepest
// second identity 0.50, 1.5% of the ring blended. Pinning the weights to a
// constant vector takes all four spreads to 0.000.
const SPLAT_MIN_SPREAD = Number(process.env.BROWSER_SMOKE_SPLAT_MIN_SPREAD || 0.15);
const SPLAT_MIN_SECOND = Number(process.env.BROWSER_SMOKE_SPLAT_MIN_SECOND || 0.35);
// Area, not just an extremum: the transition must be a band, not one row of
// vertices. Which biomes a 320 m ring contains is a worldgen fact (the
// moisture channel's features are ~700 m across, so a ring is often one
// biome) — this floor is set from the pinned spawn's measured 1.5%.
const SPLAT_MIN_MIXED = Number(process.env.BROWSER_SMOKE_SPLAT_MIN_MIXED || 0.005);
// --- the grain gate (DECISIONS.md §open, "materials v1") --------------------
// Assertion 15 counts pixels that MOVED, which cannot tell a fourth octave
// from a fourth tint: a wash moves every pixel it touches, and so does an
// exposure slip. What the second pass added is TEXTURE. So the probe here
// measures neighbour-to-neighbour contrast over the pixels the toggle moved,
// in both states — grain is by definition the thing that changes between one
// pixel and the next — and it does it at two views:
//
//   near — grain reaches the frame at arm's length. Pitched steeply down so
//          the ground in frame is the few metres that are grain's whole range.
//   far  — and it is GONE out there. Lifted 60 m and pitched shallow, so the
//          ground in frame is 100–200 m off, well past the cycles-per-pixel
//          fade. An octave that survives that view is an octave that aliases.
const GRAIN_PROBE_MIN_DELTA = 6;
const GRAIN_NEAR_PITCH = -1.05;
const GRAIN_FAR_PITCH = -0.42;
const GRAIN_FAR_LIFT_M = 60;
const GRAIN_VIEWS = [
  { label: "near", yaw: 0, pitch: GRAIN_NEAR_PITCH },
  { label: "far", yaw: 0, pitch: GRAIN_FAR_PITCH, lift: GRAIN_FAR_LIFT_M },
];
// How much of the near frame grain must reach.
const GRAIN_NEAR_MIN_FRACTION = Number(process.env.BROWSER_SMOKE_GRAIN_MIN || 0.05);
// The assertion that says grain and not wash. 1.0 is "the toggle changed the
// pixels without changing the detail between them"; 2.0 is a doubling.
const GRAIN_MIN_CONTRAST_RATIO = Number(process.env.BROWSER_SMOKE_GRAIN_MIN_RATIO || 2.0);
// Signed both ways, per view: a tint can only move the frame one direction.
const GRAIN_MIN_DIRECTIONAL = Number(process.env.BROWSER_SMOKE_GRAIN_MIN_DIR || 0.005);
// The ceiling from 60 m up. Grain that survives out there is grain that
// aliases, which is the failure the cycles-per-pixel fade exists to prevent.
const GRAIN_FAR_MAX_FRACTION = Number(process.env.BROWSER_SMOKE_GRAIN_FAR_MAX || 0.005);

// --- the tint gate (DECISIONS.md §open, "materials v3") ---------------------
// Assertions 15 and 15b share a blind spot, and it is the one the visual judge
// wrote its gap 1 about. Both read LUMA. Every octave this material had before
// the tint multiplied albedo by a scalar, and `k·(r,g,b)` has exactly the
// chromaticity of `(r,g,b)` — so a ground that is one green at forty
// brightnesses and a ground that is forty greens score the same moved
// fraction, the same signed split, and the same neighbour contrast. Both
// gates were green on all six frames a blind reader called "a single
// untextured green sheet".
//
// So this one measures the chromaticity cloud instead: its RMS spread over
// the pixels the toggle moved, in both states, plus how far its centre moved
// and how far the whole frame's mean luma moved. Three numbers, three
// different failures:
//
//   spread up      — the surface gained colour variation. This is the claim.
//   centre still   — it is not a tint. A wash moves the centre and leaves the
//                    spread; that is the failure this octave could most
//                    plausibly be mistaken for.
//   mean luma still— it is not an exposure slip, and the identities' authored
//                    colours are still their MEANS. The deviation is signed
//                    and added for exactly this reason, so the claim is
//                    checkable rather than a comment.
//
// The two views are this octave's own, and NOT grain's, for a reason worth
// stating because grain's pair is right there and was tried first:
//
//   near   grain's own arm's-length view (pitch −1.05 from eye height, so the
//          ground in frame is 1–4 m off). This is where the tile octave lives
//          and where the SPREAD claim is measured.
//   level  a standing look, pitch −0.25, so the frame runs from ~3 m of
//          ground to the fog line — the vantage a player actually spends the
//          game in, and the one that would catch an octave that only exists
//          under the camera's feet.
//
// Both views carry the "variance, not a wash" ceilings, and that pair is what
// deleted a term from this octave rather than being written around it: two
// cuts of the tint carried a coarse bias (macro, then meso) meant to break
// tiling, and each read as a colour cast — six times more centre movement
// than spread on the standing view. See `materials.js`, "what is NOT here".
//
// Grain's 60-m-up "far" view is deliberately absent. It is grain's CEILING —
// an octave that survives out there aliases — and inverting it into a floor
// for the tint measured 0.000% of the frame moved in both states: at 0.2 m/px
// the tile has retired exactly as the sampling law says it must, and what a
// coarse term would leave behind out there is the cast this octave no longer
// has. Asserting on that view would be asserting on the instrument's noise
// floor. Distance is the splat's job (macro drives `gmWob`, so a far hillside
// changes IDENTITY rather than being one identity tinted) and the lighting
// owner's.
//
// The probe's own two thresholds. The luma one is grain's and is here only to
// keep the control render honest; the CHROMA one is what this octave is masked
// on, because a luminance-neutral swing puts nothing in a luma mask. 0.004 of
// chromaticity is what one 8-bit code step is worth on a mid-grey pixel
// (1/255 over a sum of three channels near 128 × 3), so it is the finest
// difference an 8-bit readback can be said to have resolved at all.
const TINT_PROBE_MIN_DELTA = 6;
const TINT_PROBE_MIN_CHROMA = 0.004;
const TINT_NEAR_PITCH = GRAIN_NEAR_PITCH;
const TINT_LEVEL_PITCH = -0.25;
const TINT_VIEWS = [
  { label: "near", yaw: 0, pitch: TINT_NEAR_PITCH },
  { label: "level", yaw: 0, pitch: TINT_LEVEL_PITCH },
];
// How much of each frame the octave must reach, in CHROMATICALLY moved pixels.
// Measured 76.4% near, 26.2% level — the floors sit ~2.5x under both.
const TINT_NEAR_MIN_FRACTION = Number(process.env.BROWSER_SMOKE_TINT_MIN || 0.3);
const TINT_LEVEL_MIN_FRACTION = Number(process.env.BROWSER_SMOKE_TINT_LEVEL_MIN || 0.1);
// Signed both ways on the red-chromaticity axis, per view — the axis all four
// deviations move along, warm at `+dev` and cool at `−dev`. A cast can only go
// one way. Measured warm/cool 27.6/48.8 near and 12.6/13.6 level.
const TINT_MIN_DIRECTIONAL = Number(process.env.BROWSER_SMOKE_TINT_MIN_DIR || 0.05);
// THE assertion. 1.00 is what a scalar octave scores — the chromaticity of a
// pixel is invariant under multiplication, so a brightness pattern cannot move
// this at all except through the tone map's own curvature. Measured ×1.390
// near and ×1.385 level, against an off-state spread that is not flat to begin
// with (shadowed ground is bluer than lit ground under this rig, and the splat
// already varies), so a 39% gain on top of all of that is the octave's alone.
const TINT_MIN_CHROMA_RATIO = Number(process.env.BROWSER_SMOKE_TINT_MIN_CHROMA || 1.2);
// …and the "variance, not a cast" ceiling: how far the chromaticity cloud's
// CENTRE moved, as a share of the cloud's own width in the on state.
//
// The denominator is the width and not the width GAINED, and the difference
// matters enough to write down because the gained form was tried first and
// scored 1.03 against a 0.5 ceiling on ground that is demonstrably not a cast.
// Chromaticity is `(r,g)/(r+g+b)` on tone-mapped 8-bit output — a nonlinear
// coordinate — so the mean of the deviated frame is not the deviation of the
// mean, and a strictly zero-mean, luminance-neutral swing still moves the
// measured centre by a Jensen term. Measuring that term against the small
// difference of two large spreads scores the curve, not the cast.
//
// Against the width it is a real bar, and it was exercised rather than argued:
// compiling `gmTile` to a constant — a genuine cast of this octave's own
// typical magnitude — measured a centre shift of 0.04020 against a spread of
// 0.04162, a share of 0.966, and took the spread ratio DOWN to ×0.840 and the
// cool side to 0.00% at the same time. Three of this assertion's measures fire
// on one mutation. What ships measures 0.29 near and 0.12 level.
const TINT_MAX_CENTRE_SHARE = Number(process.env.BROWSER_SMOKE_TINT_MAX_CENTRE || 0.5);
// The whole frame's mean luma, in luma steps out of 255. Near zero for a
// reason and not by luck: the deviations are luminance-neutral by
// construction, so this octave has no brightness to spend. Measured 77.43 →
// 77.17 near and 67.35 → 67.36 level, and the probe's own luma mask — six
// steps, the one grain lives on — catches 0.000% of either frame.
const TINT_MAX_MEAN_LUMA = Number(process.env.BROWSER_SMOKE_TINT_MAX_LUMA || 1.0);
// Structural: how parallel an identity's chromatic deviation may be to its own
// colour. 1.0 is exactly parallel, which is a brightness multiply wearing a
// texture's name — the state this octave exists to leave. Measured: grass
// +0.152 (the closest), rock −0.047, sand −0.036, litter −0.118. Exercised:
// re-authoring all four as a fixed fraction of their own colour scores 1.0000
// on every one and fails here.
const TINT_MAX_DEV_PARALLEL = Number(process.env.BROWSER_SMOKE_TINT_MAX_PAR || 0.5);

// --- the daylight gate (DECISIONS.md §open, "the daylight register") --------
// Three counted asserts, and none of them is a taste. Each is a difference
// between two renders of one scene (see `scene.daylightProbe`), so each means
// the same thing on this box and on the reference VPS.
//
// The sweep is level-ish on purpose, unlike every other probe in this file:
// the shadow probes pitch down because they are about the ground, and this
// one has to hold the sky AND the ground in one frame or assertion (a) has
// nothing to compare. −0.12 rad puts the horizon a little above centre from
// eye height, which is also the register the capture harness's 01/02/06
// vantages shoot.
const DAYLIGHT_PROBE_YAWS = [0, Math.PI / 2, Math.PI, (3 * Math.PI) / 2];
const DAYLIGHT_PROBE_PITCH = -0.12;
// And the second sweep, lifted, where the air is measurable. The lift is the
// same argument (and a fraction of the height) the far-shadow and horizon
// probes make for theirs; the pitch keeps the horizon inside the frame so the
// far tercile has somewhere to come from.
const DAYLIGHT_AIR_HEIGHT_M = 40;
const DAYLIGHT_AIR_PITCH = -0.18;
// Same 6/255 bar every other probe in this file uses for "a pixel moved":
// comfortably above the rasterizer's own noise between two renders of an
// identical frame.
const DAYLIGHT_MIN_DELTA = 6;
// (a) The sky has to be the top of the value range, not the bottom of it.
// The defect measured by the visual judge was sky 55–114 against sand
// 160–200 — a ground brighter than its own sky, which is not a dark scene
// but an inverted one. Asserted against the frame's own MEDIAN ground pixel
// rather than its mean: a mean can be dragged under by a shadowed half while
// the lit half still out-glares the sky, and the median cannot.
const DAYLIGHT_MIN_SKY_OVER_GROUND = Number(process.env.BROWSER_SMOKE_SKY_OVER_GROUND || 1.15);
// And the sky has to be IN the frame for that ratio to mean anything — a
// vantage that framed only ground would divide by a handful of pixels.
const DAYLIGHT_MIN_SKY_FRACTION = 0.05;
// (b) There has to be air, and it has to lighten. This is the share of the
// swept pixels the haze moved by more than the bar — measured 1.08 / 8.23 /
// 8.19 / 3.47% on the lifted sweep, so the floor sits 2.2x under the worst of
// the four. The failure it guards is what shipped before this slice: a fog
// near plane past everything the frame can see scores exactly 0.00%, which is
// what the eye sweep still reads on the one yaw whose ground is all
// foreground.
const DAYLIGHT_MIN_FOG_FRACTION = Number(process.env.BROWSER_SMOKE_FOG_MIN || 0.005);
// …and the far third of the ground has to be genuinely in it. The probe's
// depth channel is the recovered fog FACTOR, so this is a floor on the mean
// factor of the far tercile: below it the "far" band is near ground with
// rounding noise on it, and the ramp above would be measuring nothing.
const DAYLIGHT_MIN_FAR_FOG = Number(process.env.BROWSER_SMOKE_FAR_FOG_MIN || 0.03);
// Aerial perspective converges on a bright sky, so it can only ADD luma. A
// fog colour under the ground's own value would darken with distance, which
// is exactly what shipped before this slice; that failure scores 0 here.
const DAYLIGHT_MIN_FOG_UP_SHARE = 0.98;
// …and it has to be a RAMP, not a curtain: over the swept frame, the far
// third of the ground reads brighter and less saturated than the near third.
// Ratios rather than differences so the bars do not move with the register.
// Measured x1.118 luma and x0.719 saturation over the four lifted yaws.
const DAYLIGHT_MIN_BAND_LUMA_STEP = 1.05;
const DAYLIGHT_MAX_BAND_SAT_STEP = 0.9;
// (c) The ambient floor: the share of a ground pixel's output luma that
// survives losing the key. Read at the 5th percentile — the darkest ground
// pixels are the most-lit ones, so that is where a floor is actually tested,
// and a p05 rather than a min because the water's specular track is a handful
// of pixels at nearly pure key.
//
// **0.15, and not the judge's 0.30, and this is the one bar in this gate that
// is a REGRESSION wall rather than an achievement.** The rig measures
// **20.8–41.2%** and cannot be raised to clear 0.30: the prop gate's chroma
// ratio ships with 0.02 of headroom over its own floor, every unit of ambient
// lands in that ratio's denominator, and six built-and-measured
// configurations of the fill all take it red — the sky pole through the
// boulder, the bounce pole through the pine (`DECISIONS.md` §open, "the
// daylight register", has the table). Asserting 0.30 anyway would be a gate
// the tree cannot pass; asserting nothing would let the next slice spend the
// floor unnoticed. So this asserts the share cannot FALL, at 1.4x under the
// worst yaw the rig delivers.
//
// The metric is narrower than the judge's sentence, and the difference is
// worth stating rather than blurring: it reads the fill's share on NON-SKY
// pixels, which at these vantages are mostly ground. It is sensitive to both
// poles and measured so — raising the bounce 3.4x while leaving the sky pole
// alone moved the worst yaw 20.8% → 28.2% — but a ground pixel faces up, so
// what it will never see is the case the judge actually measured: a canopy's
// underside at (2, 6, 0). That one needs an object-face probe, and this pass
// did not build one.
const DAYLIGHT_MIN_AMBIENT_FLOOR = Number(process.env.BROWSER_SMOKE_AMBIENT_MIN || 0.15);

// --- the alias gate (DECISIONS.md §open, "the bump's sampling law") ---------
// Where the quad statistic is taken, and against what.
//
// Two vantages, and they are the capture harness's own 01 and 04 (yaw 0 pitch
// 0, yaw 0 pitch −0.8) because that is where the defect was scored. 04 is the
// worse of the pair and the near-ground one; 01 carries the mid distance, so
// between them the mask covers the footprint band grain's fade actually lives
// in. The probe takes them from the player's own position, like 15b's.
const ALIAS_VIEWS = [
  { label: "level", yaw: 0, pitch: 0 },
  { label: "down", yaw: 0, pitch: -0.8 },
];
// A pixel joins the ground mask when the field moved it by more than this,
// which is 15b's own `GRAIN_MIN_DELTA` argument: one 8-bit step separates a
// pixel the material painted from one it did not.
const ALIAS_MIN_DELTA = 1;
// The ceiling on quad-locked energy, as a ratio of across-quad neighbour
// contrast to within-quad. Scale-free, so it says nothing about how much
// detail the ground carries — only about where that detail's edges fall.
//
// Where the number comes from: the probe's `nobump` leg is the floor — the
// identical scene with gmH identically zero, so no screen derivative reaches
// the image at all — and it measures 1.00–1.01 at both vantages. `nograinbump`
// (grain's bump alone removed, both structural octaves' bump left in) measures
// 1.02–1.03. 1.35 sits ~30% above both and 2.3× below the ×3.12/×6.15 the
// SHIPPED material scores, which is the defect this pass measured and did not
// land the fix for — see the §open row. Those two legs are asserted; the ship
// leg is reported and not yet walled, because a ceiling calibrated to let the
// defect through is worse than no ceiling.
const ALIAS_MAX_RATIO = Number(process.env.BROWSER_SMOKE_ALIAS_MAX || 1.35);
// How much of the frame the field must paint for the ratio to mean anything.
// A mask that collapsed would make a flat wash score a clean 1.00 for the
// wrong reason, so this is a floor on the sample and not on the material.
//
// Measured AT THIS GATE'S OWN SPAWN, which is the one that matters: the dev
// shard pins the player to 1024,1024 at y = 12.3 m, so the level vantage is
// mostly sky and paints 21–25% (195–229 k pixels) while the down one paints
// 57%. The capture harness's beach spawn stands at y = 1.2 m and the same two
// vantages paint 83% and 100% there — the reason to state the box the number
// came off, the same way the timed block in this file does.
const ALIAS_MIN_MASK = 0.15;
// The control's ceiling: two renders of one state, differing on at most this
// share of the frame. Not zero — the two live renders on this box have been
// seen to differ on ~11 px of 921,600 — but four orders of magnitude below
// anything that could move the statistic.
const ALIAS_MAX_NOISE = 0.001;

// --- the base-map gate, 15h (ART.md §7, "the CC0 working set") -------------
// The number this whole slice is aimed at, and the only one in this file taken
// off the reference images rather than off our own frames: `ART.md` §3
// measured the near-ground neighbour contrast of `Rust Images/` at **6.3 luma
// per pixel** against **0.26** in ours. It is stated here as a TARGET and not
// as the floor, because a floor set to a number the tree has never reached is
// a gate that fails on merge day and gets widened on the next one — which is
// the same weakening, taken slowly. The floor below is what the maps actually
// deliver on this box; the target is what it is walking toward, and the gap
// between them is printed on every run so it cannot quietly stop closing.
const ART_NEAR_GROUND_TARGET = 6.3;
// Two vantages, and they are `ALIAS_VIEWS`' — the capture harness's own 01 and
// 04 shapes, taken from the player's eye. Reusing them is deliberate: the
// alias gate's numbers were measured at these two framings, this gate's are
// about the same pixels, and a third aim would have made the two sets of
// measurements incomparable for no gain.
const BASE_VIEWS = [
  { label: "level", yaw: 0, pitch: 0 },
  { label: "down", yaw: 0, pitch: -0.8 },
];
// One 8-bit step separates a pixel the maps painted from one they did not —
// `GRAIN_PROBE_MIN_DELTA`'s argument, `ALIAS_MIN_DELTA`'s and `PROP_MIN_DELTA`'s.
const BASE_MIN_DELTA = 1;
// The control's ceiling, `ALIAS_MAX_NOISE`'s value and its argument: two
// renders of one state on this box have been seen to differ on ~11 px of
// 921,600, four orders of magnitude below anything that moves the statistic.
const BASE_MAX_NOISE = 0.001;
// How much of the frame the maps must reach. A photograph differs from a flat
// swatch nearly everywhere it is laid, so unlike the octave gates' fraction
// floors this one is a floor on the INSTRUMENT: it is what stops the contrast
// numbers below from being computed over a cherry-picked handful of pixels.
// Measured AT THIS GATE'S OWN SPAWN, the way `ALIAS_MIN_MASK` states its own:
// the dev shard pins the player to 1024,1024 at y = 12.3 m, and the maps reach
// **9.7%** of the level frame and **58.9%** of the down one. The level number
// is small for a stated reason rather than a worrying one — at pitch 0 most of
// that frame is sky and ground past the base's own footprint retirement — so
// the floor sits ~2x below the worse of the two, which is `PROP_MIN_STRUCTURE`'s
// construction. The failure it guards (the maps not arriving at all) scores 0.
const BASE_MIN_MASK = Number(process.env.BROWSER_SMOKE_BASE_MIN_MASK || 0.05);
// Two-sided, per view. `SURFACE_MIN_DIRECTIONAL`'s argument applied to the
// base: the single most plausible way to move a contrast number without
// adding detail is to lift the ground, and a lift moves every pixel one way.
const BASE_MIN_DIRECTIONAL = Number(process.env.BROWSER_SMOKE_BASE_MIN_DIR || 0.005);
// THE floor, in 8-bit luma per neighbouring pixel, on the SHIPPED frame.
//
// Measured: **5.90** at the level vantage and **8.61** at the near-ground one,
// against **0.41-0.47** from the procedural octaves alone at the same two
// framings — which is the 0.26 `ART.md` §3 recorded, re-measured by a
// different instrument at a different spawn and landing in the same place.
// The near-ground vantage is already past §3's 6.3 reference target; the level
// one is not, and the gap is printed on every run rather than rounded away.
//
// The floor is 4.0: 1.5x below the worse of the two, and 8.5x above what the
// failure it guards — the maps silently not reaching the ground — scores.
const BASE_MIN_CONTRAST = Number(process.env.BROWSER_SMOKE_BASE_MIN_CONTRAST || 4.0);
// …and the lift over the procedural ground, which is the half that says the
// photograph is reaching the image as DETAIL rather than as a colour. A base
// that delivered its mean and nothing else — the exact thing the footprint
// retirement converges to, so the exact thing a fade set too aggressively
// would ship — scores x1. Measured x12.75 and x20.69; the floor is x5.
const BASE_MIN_CONTRAST_RATIO = Number(process.env.BROWSER_SMOKE_BASE_MIN_RATIO || 5.0);

// --- the chroma-artifact gate, 15i (ART.md §7, "a correction, not an
// --- amplifier") ------------------------------------------------------------
// 15h above is a FLOOR on how much the near ground varies from one pixel to the
// next, and the pass that landed it cleared that floor by an order of magnitude
// while shipping per-pixel rainbow speckle across four of six captured frames.
// Both facts are true at once because amplitude cannot tell detail from noise.
// This is the ceiling that can: the high-frequency residual resolved ALONG the
// local mean colour (a surface lighter here, darker there — what 15h counts)
// versus ORTHOGONAL to it (the hue changed between neighbouring pixels).
//
// The TARGET is taken off the reference images — the second number in this file
// that is, after 15h's 6.3 — and it is measured with the probe's own estimator
// rather than a near relative of it. That distinction is not pedantry: the
// first cut of this gate measured the references with a 2x2-box residual while
// the probe used a 4-neighbour-mean one, which put the reference maximum at
// 0.336 instead of 0.193 and would have walled our 0.317 in as a pass. A
// ceiling from a differently-computed statistic is not a ceiling.
//
// Restricted, too, to the thirteen frames that actually have ground in the
// band. `crafting.png`, `inventory.jpeg`, `mapstylized.jpg` and `building.jpeg`
// are UI screenshots and `mapraw.jpg` is a top-down map render; none of them
// can define a statistic about ground, and two of them are the highest readings
// in the unrestricted set. Over the thirteen that qualify:
//
//     median 0.120, range 0.077 (generichighview) - 0.193 (gameplayfoundbase)
//
// and over the six frames the visual judge scored on 2026-08-03, same
// estimator, same band:
//
//     01 0.659  02 0.798  03 0.237  04 0.284  05 0.760  06 0.092
//
// — which is the artifact's own footprint. `06-hud.png` is the only frame with
// no near ground in it and it is the only one inside the reference band; every
// frame that shows ground is over the reference maximum, and the three the
// judge called worst are 3.4-4.1x over it. Our residual ALONG colour was inside
// the reference range the whole time, which is why the fix is a chroma bound
// and not a blur.
const REF_CHROMA_TARGET_MAX = 0.193;
const REF_CHROMA_TARGET_MEDIAN = 0.12;
// …and THE WALL is not the target, for 15h's own stated reason applied to a
// ceiling instead of a floor: *"a floor set to a number the tree has never
// reached is a gate that fails on merge day and gets widened on the next one —
// which is the same weakening, taken slowly."* This pass moved the shipped
// frame from 0.434 to 0.317 at the level vantage and 0.313 to 0.243 at the
// near-ground one. That is a real reduction and it is **not** the references:
// 0.317 is still 1.6x over the reference maximum, and saying so here is the
// point of separating the two numbers. The gap is printed on every run so it
// cannot quietly stop closing.
//
// 0.35 is two-sided. It is 1.10x above the worse of the two shipped readings,
// and 1.24x BELOW the 0.434 that the per-channel gain this pass replaced scores
// at the same vantage — so a regression to the previous behaviour goes red
// rather than merely looking worse.
const CHROMA_MAX_RATIO = Number(process.env.BROWSER_SMOKE_CHROMA_MAX || 0.35);
// The same two vantages, the same one-code-value mask delta and the same
// control ceiling as 15h — this gate is about the same pixels, and a third aim
// would have made the two incomparable for no gain.
const CHROMA_VIEWS = BASE_VIEWS;
const CHROMA_MIN_DELTA = BASE_MIN_DELTA;
const CHROMA_MAX_NOISE = BASE_MAX_NOISE;
// The instrument's own floor: how many neighbourhoods the statistic was taken
// over. Not a quality bar — it is what stops a ratio computed over a handful of
// pixels from standing where one computed over the ground stands. Measured at
// this gate's spawn: the level vantage yields tens of thousands of windows and
// the down one hundreds of thousands.
const CHROMA_MIN_PAIRS = Number(process.env.BROWSER_SMOKE_CHROMA_MIN_PAIRS || 5000);
// …and the half that keeps this from being satisfiable by deleting the base
// maps. A flat swatch has NO chroma residual and would score 0, sailing under
// any ceiling — so the shipped frame must also still show the artifact being
// SUPPRESSED rather than absent: the `stretch` leg (every layer's keep forced
// to 1, which is algebraically the per-channel gain the previous pass shipped)
// must sit above the ship leg by this factor. A build where the two agree has
// either lost the fix or lost the base, and both should be loud.
//
// Measured on the shipped frame: **x1.37 at the level vantage and x1.29 at the
// near-ground one**. The floor is 1.15 — midway between the worse of those two
// and the 1.00 that the failure it guards scores exactly, which is the shape
// `BASE_MIN_MASK` uses ("~2x below the worse of the two") applied to a quantity
// whose failure value is a hard 1 rather than a 0.
//
// The suppression is this modest for a stated reason rather than a worrying
// one, and the `scalar` leg is what says so: the dev shard's spawn is 99.2%
// grass by dominant weight (see the `splat:` line above), and grass's own gain
// span is x1.64, so its keep is 0.61 and its delivered stretch is exactly the
// x1 ceiling. The layers the bound bites hardest on — `litter` at keep 0.26 and
// `rock` at 0.17 — are 0.0% and 0.1% present here. This gate therefore measures
// the fix at its WEAKEST, which is the right direction for a wall to err.
const CHROMA_MIN_SUPPRESSION = Number(process.env.BROWSER_SMOKE_CHROMA_MIN_SUPPRESS || 1.15);

// --- the prop-surface gate (DECISIONS.md §open, "prop surfaces v0") ---------
// Assertions 15 through 15e are all about the GROUND. The visual judge's pass
// 20260802-163821-02 put its ranked gap 1 on everything else: "rock, wood and
// canopy are each one flat colour per facet — literally the rubric's own
// disqualifier", a 4,386-pixel boulder facet at luma sd 0.96 returning the
// identical byte triple at four widely separated sample points, against
// `spawnedrock.jpg`'s rock at sd 26.31. So this gate photographs a PROP.
//
// Two views, each aimed at a real instance the terrain found rather than at a
// bearing — a rock, because it is the class the report measured, and a pine,
// because it is the class a blind reader named in 3 of the 3 frames that show
// one. `off` is a metre offset from the instance's own origin, scaled by its
// instance scale, and `aim` is the height on it the camera looks at, so a big
// pine and a small one frame the same way.
//
// Both look down from above the surrounding ground rather than across it. That
// is the instrument, not the composition: the first cut of this gate put the
// eye level with the trunk 7.5 m out and photographed a hillside, and a view
// with no prop in it scores a perfectly clean 0.00% on a class whose field is
// the strongest in the table.
// Both stand inside 5 m, which is a statement about the material and not about
// composition: the detail octave is the coarse one times `detailMul` and
// therefore retires at a quarter of its distance, so a view that frames the
// prop but stands outside the detail band measures the coarse octave alone and
// scores a neighbour-contrast ratio of ~1.0 by construction. Measured on the
// way here: the same pine at 10 m scored x1.07, at 5 m it is a different
// number about a different octave.
const PROP_VIEWS = [
  { label: "rock", surface: "rock", off: [2.6, 2.6, 1.6], aim: 0.5 },
  { label: "pine", surface: "foliage", off: [1.8, 3.6, 2.8], aim: 2.6 },
];
// How far out to look for one. The near ring is 5x5 64 m chunks, so anything
// inside 150 m of the player is streamed and instanced.
const PROP_SEARCH_M = 150;
// One 8-bit step separates a pixel the field painted from one it did not —
// `GRAIN_MIN_DELTA`'s argument, and `ALIAS_MIN_DELTA`'s.
const PROP_MIN_DELTA = 1;
// How much of the frame the field must reach, per view. A prop at these
// framings is a large object seen close, so this is a floor on the INSTRUMENT
// (did the probe actually find and frame a prop) as much as on the material;
// the failure it guards — a class whose field contributes nothing — scores
// exactly 0.
const PROP_MIN_FRACTION = Number(process.env.BROWSER_SMOKE_PROP_MIN || 0.02);
// Signed both ways, per view, and this is the assertion that separates a
// SURFACE from a wash. Assertion 15 learned it the hard way on the ground: a
// mutation that collapsed the noise scales moved 66–96% of the pixels — a
// uniform darkening — and sailed past every fraction floor there was.
// Microstructure lightens some pixels and darkens others; a tint, an exposure
// slip and a global darkening cannot.
const PROP_MIN_DIRECTIONAL = Number(process.env.BROWSER_SMOKE_PROP_MIN_DIR || 0.002);
// …and the assertion that separates TEXTURE from a colour change, which is
// 15b's argument applied to props: mean neighbour-to-neighbour luma step over
// the moved set, shipped against flat. 1.0 is "the field moved the pixels
// without changing the detail between them", which is exactly what a flat
// facet with a different colour on it would score.
//
// The floor is 1.15 and not higher, and the reason is a limit of THIS measure
// rather than a concession: its denominator is the flat state's own detail —
// the mesh's facet edges, its baked vertex-colour ramp, the shadow map — none
// of which the toggle removes, so the ratio is bounded by
// (baseline + added)/baseline and a prop with structure of its own can never
// score what a smooth heightfield does. Measured: one field, x1.62 on a pale
// faceted boulder and x1.26 on a dark canopy whose baseline has facets and a
// colour ramp already in it. The sharp
// assertion is the next one down, which has no baseline at all.
const PROP_MIN_CONTRAST_RATIO = Number(process.env.BROWSER_SMOKE_PROP_MIN_RATIO || 1.15);
// THE assertion, and the one with no baseline in it: neighbour-to-neighbour
// variation of the field's OWN difference image (ship − flat), as a share of
// that image's magnitude. It asks what the field is MADE of.
//
// A constant offset — a wash, a global tint, an exposure slip, a per-facet
// colour change — has zero neighbour variation in the difference image and
// scores exactly 0. Not approximately: by construction, which is what makes a
// floor here worth setting at all.
//
// Where 0.02 comes from, and it is calibration against a measured pair rather
// than a model: the shipped field scores **0.050** at the boulder view and
// **0.041** at the pine, both AT THIS GATE'S OWN SPAWN (the dev shard pins the
// player to 1024,1024, and the nearest boulder is 17 m out, the nearest pine
// 2.3 m — the reason to state the box a number came off, the same way the
// timed block in this file does). A wash scores 0. 0.02 sits 2x below the
// worse of the two and an infinite distance above what the failure it guards
// can reach, which is `ALIAS_MAX_RATIO`'s shape.
//
// The first cut of this floor was 0.1, derived from a sinusoid model that
// predicted ~0.4 — wrong by 8x, because the difference image's MAGNITUDE is
// dominated by the coarse octave while its neighbour STEP comes almost
// entirely from the detail one, so the ratio is roughly `share * 2*PI / (px per
// cycle)` and not the per-octave figure the model gave. The measurement is
// kept here rather than the model, because the model was checked and failed.
const PROP_MIN_STRUCTURE = Number(process.env.BROWSER_SMOKE_PROP_MIN_STRUCT || 0.02);
// …and the assertion that separates HUE from VALUE, which is 15d's argument
// applied to props: chromaticity spread over the moved set, shipped against
// flat. Every prop term before this pass multiplied albedo by a scalar, and
// `k*(r,g,b)` has the chromaticity of `(r,g,b)`, so a luma-only measure cannot
// see the difference between a boulder with two minerals in it and a boulder
// with one at forty brightnesses.
const PROP_MIN_CHROMA_RATIO = Number(process.env.BROWSER_SMOKE_PROP_MIN_CHROMA || 1.1);
// --- 15g, the pixel half: the two numbers above that are NOT ratios ---------
//
// Every prop assertion above this line divides the field by itself or by what
// it was laid on. `contrastRatio` is (baseline + added)/baseline,
// `diffStructure` is step/magnitude of the difference image, `chromaRatio` is
// spread over spread. All three are scale-free by construction — which is the
// point, for the questions they ask, and a hole for the question they do not.
//
// A field swinging ±0.8 of a level on a surface delivering luma 6 scores
// EXACTLY what the same field swinging ±17 levels on a surface delivering 120
// scores: same ratio, same structure, same chroma spread. Prop surfaces v0
// shipped green through all three (structure 0.050 and 0.041 against a 0.02
// floor) and the visual judge, measuring the merged frames, found "a solid" —
// best-fit-plane residual 1.23/255 over 7,800 px — and named the amplitude
// rather than the absence as the defect. Both readings are correct. The gate
// simply had no number with a unit in it.
//
// So: two absolute floors, in 8-bit luma, on the same mask.
//
// `lumaP50` is the class's delivered median. Its failure is a surface
// delivered into a range an 8-bit framebuffer cannot carry a texture in —
// whatever the cause, authored albedo or the light on it.
//
// Both are plain constants with no `BROWSER_SMOKE_*` env override, unlike the
// five floors above them. That is deliberate and it is the direction this
// should travel: an environment variable that can lower a wall is a way to
// weaken a gate without editing a file anyone reviews. It also lets
// `ci/knob_registry.mjs` pin both to their `DECISIONS.md` §open declarations,
// which it cannot do through a `Number(process.env… || x)` initializer.
const PROP_MIN_VALUE = 24;
// `diffMean` is the mean magnitude of the field's own difference image, in
// those same levels: what the surface is WORTH where it is delivered. Its
// failure is the measured one — a field that is real, structured and
// invisible.
const PROP_MIN_AMP = 2.2;
// Where both come from, calibrated against a measured pair and not modelled,
// which is `PROP_MIN_STRUCTURE`'s own discipline (and its recorded lesson: the
// model it was first derived from was wrong by 8x). At this gate's spawn the
// shipped material measures value p05/p50/p95 and amplitude:
//
//   rock  29/59/123  amp 8.47        pine  12/48/74  amp 4.86
//
// (before prop albedo v1's rescale: rock 27/59/123 amp 8.43, pine 11/46/74
// amp 4.72 — the pine moved because two of its three bands did.)
//
// The floors sit at roughly half the worse of the two — the same margin
// `PROP_MIN_STRUCTURE` takes — and an infinite distance above what the failure
// they guard reaches, which is 1-6 levels of delivered value and under one
// level of amplitude.
//
// And p05, the dark tail — the shaded side of the prop, which prop albedo v1
// deliberately left unwalled because the light rig owned it and no albedo
// could move it. The light rig has now moved it (§open, "lighting v1"), so
// the wall it was waiting for is written here. This is the number the visual
// judge has been measuring by hand every pass: "shadowed surfaces crush to
// zero — `03-canopy-up` cone skirt (3,9,0) at 94.9% under 10". A p05 floor is
// exactly that sentence, gated: the darkest twentieth of a prop's pixels must
// still be somewhere a surface can exist.
const PROP_MIN_P05 = 16;
// The control's ceiling, `ALIAS_MAX_NOISE`'s argument verbatim: two renders of
// one state, differing on at most this share of the frame.
const PROP_MAX_NOISE = 0.001;
// The structural half, asserted off `propFacts()` rather than off pixels.
// How many classes must carry a field at all, and how many DISTINCT structures
// the table must hold — the gap is "you cannot tell our wood from our stone by
// surface", so a table with one row copied seven times would satisfy every
// pixel assertion above and none of the ask.
const PROP_MIN_CLASSES = 6;
const PROP_MIN_DISTINCT = 5;

// --- the projection gate (DECISIONS.md §open, "materials v1", third pass) ---
// 15b proves the surface has grain. It cannot prove the grain is on the
// SURFACE: a world-XZ field stretched 1/u along a slope has exactly the same
// neighbour contrast, the same moved fraction, the same signed split. What is
// wrong with it is directional, so the gate has to be directional too.
//
// The instrument is `projectionProbe`: the shipped program and `flatgrain` (the
// same program carrying materials v1's world-XZ tap) rendered from ONE camera
// in ONE run, each toggled at `uGrain`, each scored on its own difference image
// along both screen axes. Nothing below is a threshold on an absolute number —
// every assertion is one program against the other in the same frame, which is
// the only kind of measurement this box earns the right to make.
//
// The face it is aimed at is FOUND, not written down. A seed change that moved
// the pinned spawn onto a meadow would otherwise aim the probe at level ground
// and every assertion here would pass by default, which is the failure mode the
// repo's trap list calls the worst bug class.
const PROJ_FACE_RADIUS_M = 150;
const PROJ_FACE_BIN_M = 4;
const PROJ_FACE_MIN_VERTS = 8;
// A face has to be a face. `upness` is the bin's mean normal's y (1.0 is
// level); `coherence` is that mean normal's LENGTH, which is 1 only if every
// vertex in the bin agreed and collapses on a ridge line or a crumpled patch.
// Both are floors on the EVIDENCE, not on the fix: a run that cannot find a
// slope near spawn must say so loudly rather than score a meadow.
const PROJ_FACE_MAX_UPNESS = Number(process.env.BROWSER_SMOKE_PROJ_MAX_UPNESS || 0.9);
const PROJ_FACE_MIN_COHERENCE = Number(process.env.BROWSER_SMOKE_PROJ_MIN_COH || 0.9);
// …and it has to be LIT. The grain octave reaches the image as a swing in
// albedo, so on a face turned away from a 21°-elevation key light the swing
// lands under the probe's own luma threshold and there is nothing to measure:
// the unfiltered search picked exactly such a face here and the probe scored
// 0.3% of the frame. 0.35 is the lambert term level ground itself gets from
// this sun (sin 0.36 = 0.352), so it asks a face for no more light than the
// meadow beside it already has.
const PROJ_FACE_MIN_LIT = Number(process.env.BROWSER_SMOKE_PROJ_MIN_LIT || 0.35);
// Where the eye goes: straight out along the face's own NORMAL, looking back
// down it, and the same distance straight down at level ground for the pair's
// control. Two perpendicular views of two differently-tilted patches is the
// whole instrument, and the reason it is built this way is worth stating,
// because two earlier cuts of this gate were built on measures that turned out
// not to mean what they looked like:
//
//   · Aiming down the FALL LINE measured at 74° incidence — 4% of the frame
//     carrying grain, and both screen axes foreshortened by unequal amounts.
//   · Reading the screen-axis split `gradX/gradY` at the face looked like the
//     directional measure a directional defect deserves, and is not one. The
//     flat control settles it: level ground scores 1.11 (both programs agree
//     to 0.001, as they must), and the FACE scores 0.39–0.42 in BOTH programs.
//     A 2.5x screen-axis bias that both programs share is the view's own —
//     terrain curvature across a 107° horizontal frustum — and it swamps the
//     1.456x the projection is worth. It stays in the log as evidence; it is
//     not what is asserted.
//
// What IS asserted survives all of that, because every one of those confounds
// is common to both programs at one camera: the grain's own DETAIL, direction-
// averaged and divided by its amplitude — `(gradX + gradY) / (2·amp)`, an
// inverse characteristic length in pixels — measured at the face and at level
// ground, per program. `detail(flat)/detail(face)` is then how much that
// program's grain coarsens when the surface tilts, and a projection stamped
// from above coarsens by `1/upness` while one laid on the surface does not.
// View distance, fade, splat identity, lighting, curvature and mask selection
// all sit in both programs' numerator and denominator alike.
//
// 1 m out at a 75° vertical fov is a frame ~1.5 m of face tall — well inside a
// 4 m bin — and the distance is set by the FADE, not by framing: a steep face
// wears the rock identity, whose grain is 6 cm, and 2 m out that octave is
// already past its cycles-per-pixel retirement (measured: 0.3% of the frame).
const PROJ_EYE_DIST_M = 1.0;
// Grain must actually reach the face view in BOTH programs, or the anisotropy
// below is a ratio of two zeroes.
const PROJ_MIN_MOVED = Number(process.env.BROWSER_SMOKE_PROJ_MIN_MOVED || 0.02);
// The flat control's own floor, lower because a straight-down view from eye
// height covers more ground per pixel than a 1 m look at a face, so more of it
// sits past the octave's cycles-per-pixel fade (measured: 2.4–2.7%).
const PROJ_MIN_MOVED_FLAT = Number(process.env.BROWSER_SMOKE_PROJ_MIN_FLAT || 0.01);
// THE assertion this slice exists for: how much less the shipped projection's
// grain coarsens across a tilt than the one it replaces. Both programs are
// measured at the same two cameras in the same run, so the number is a ratio of
// two ratios and nothing about this box, this seed or this face survives into
// it. Measured on the 46.6° face this spawn offers: world XZ coarsens ×2.02
// from level ground to the face, triplanar ×1.40 — a gain of ×1.44 against the
// ×1.456 stretch a stamped-from-above projection has to eat in full. The floor
// is 1.15: a program that lost the fix scores 1.00 exactly, and what is left
// over is margin for a seed that offers a gentler face than this one.
const PROJ_MIN_STRETCH_GAIN = Number(process.env.BROWSER_SMOKE_PROJ_MIN_GAIN || 1.15);
// The flat control's own bar. On ground that is not tilted the two projections
// are the same arithmetic and must measure the same grain; they land 6.7% apart
// here because "level" near this spawn is up to 6° of slope, not 0°.
const PROJ_FLAT_MAX_SPREAD = Number(process.env.BROWSER_SMOKE_PROJ_FLAT_SPREAD || 0.15);
// …and it did not buy that by blending the octave into mush, which is what a
// stock triplanar blend does when the deviation is not restored by 1/|w| (the
// abandoned first attempt measured x0.56 the contrast that way — an isotropy
// win bought by deleting the grain). Amplitude is the mean |Δluma| the octave
// contributes over the pixels it moved.
const PROJ_MIN_AMP_RATIO = Number(process.env.BROWSER_SMOKE_PROJ_MIN_AMP || 0.9);
// The confinement ceiling, where the octave is already retired and the two
// programs therefore have nothing to disagree about. Not zero, for the reason
// `COST_IDENTITY_MAX_DELTA` is not zero: separately compiled programs schedule
// the same arithmetic differently and a last-bit difference flips a fragment at
// a smoothstep knee or a silhouette.
const PROJ_RETIRED_MAX_FRACTION = Number(process.env.BROWSER_SMOKE_PROJ_RETIRED_MAX || 0.0002);
// The confinement pair: how far apart the two projections put the frame, over
// the grain's own pixels, in units of what the grain contributes there. Two
// bars, not one ratio, because each names a different failure and a ratio names
// neither — a tap that ignores the normal entirely makes the two programs
// identical, so both sides go to zero and any ratio bar passes 0 >= 0.
//
//   floor on the FACE — the projection has to actually do something where the
//   ground is tilted. A neutered tap scores 0.000 here. Measured 0.534 on this
//   spawn's 46.6° face; the floor is 0.15, and it is a floor on the evidence
//   this seed happens to offer, so a gentler face would lower the measurement
//   while `PROJ_FACE_MAX_UPNESS` keeps it above 25.8° of slope.
//
//   ceiling on LEVEL ground — and next to nothing where it is not. This is the
//   "level ground is unchanged" claim, measured rather than asserted from the
//   algebra: on the flat control the two programs differ by at most 1/255 luma
//   over 14.1% of the frame, which is 0.022 of the octave's own amplitude
//   against the face's 0.534. The ceiling is 0.10 — under a fifth of the face's
//   reading, and five times the measurement, because "level" near this spawn is
//   up to 6° of slope and another seed's meadow may be steeper.
const PROJ_MIN_FACE_EFFECT = Number(process.env.BROWSER_SMOKE_PROJ_MIN_FACE_EFF || 0.15);
const PROJ_MAX_FLAT_EFFECT = Number(process.env.BROWSER_SMOKE_PROJ_MAX_FLAT_EFF || 0.1);

// --- the fragment budget (DECISIONS.md §open, "fragment budget v0") ---------
// DESIGN §9 budgets the frame by DRAW CALLS and TRIANGLES, and the gate has
// asserted both since lighting v0. Neither says anything about what a single
// fragment costs — which is the budget `NOW.md` item 1 ran out of: the grain
// branch measured well and did not merge because the terrain program was
// already too expensive for the browser gate's third tab, and no gate here
// could have caught that coming.
//
// These two are the per-fragment half, and both are COUNTED, so unlike the
// milliseconds the probe prints they mean the same thing on this box and on
// the reference VPS.
//
// Depth fetches per shaded fragment, summed over the clipmap's levels: 18
// today (16 for level 0's PCF, read off three's own chunk, plus one each for
// the two coarse levels). The cap leaves room for a fourth level or a modestly
// wider coarse kernel and still fails the change that matters — a near filter
// that goes back to being the most expensive thing in the shader.
const DEPTH_FETCH_BUDGET = Number(process.env.BROWSER_SMOKE_DEPTH_FETCHES || 24);
// The compiled terrain fragment program, in characters of GLSL with three's
// `#include`s expanded: 88,883 today, of which 73,375 is three's stock
// MeshStandardMaterial as it was handed over and 15,508 (17.4%) is everything
// this repo added to the ground. The cap is ~18% over, the same construction
// as the 96,000 it replaces (81,520 measured, 8,145 ours) — it survives a
// three minor bump, and its slack is about one doubling of this repo's share,
// which is what it is here to catch.
//
// Re-derived rather than widened, and the difference is the point. The base
// maps and materials v5's biplanar wall tap took our share 8,145 -> 33,050 and
// the program to 106,425, which walked through the old cap — correctly, that
// is the gate working. But roughly three-fifths of that growth was COMMENT
// TEXT shipping into the compiled program, so the metric had drifted into
// measuring how much the shader explained itself. `materials.js` now emits
// through a `glsl` tag that blanks full-line comments (they stay in the file;
// the blank line keeps a driver's error line numbers honest), which took the
// program to 88,883 with the code unchanged — verified by this file's own
// probes running green on the same commit. The cap is set against the stripped
// number so the wall keeps its old relative strictness instead of pocketing
// the 17,542 chars the strip freed.
const TERRAIN_FRAGMENT_BUDGET = Number(process.env.BROWSER_SMOKE_FRAG_CHARS || 105000);
// Noise sample sites per shaded ground fragment: 7 today — three field octaves,
// the grain octave's three triplanar taps, and materials v3's one tile tap —
// where materials v1 paid 4 on one world-XZ tap. Each site is four `gmHash`
// evaluations, so this is the arithmetic axis the ground's shading actually
// lives on, and it is the axis `NOW.md` item 1 says to price the projection in
// ("sample sites and program chars — not in ms") after six cost-probe runs read
// five of six the wrong sign. The cap fails a program that went triplanar on
// the whole field, which would be 12+.
//
// The cap is NOT moved by this pass, and that is deliberate: the tile octave
// spends the headroom it was holding rather than widening the budget to fit,
// which is the shape a gate rots into. What is left is one site. The next
// octave that wants one has to justify itself against that, and an octave that
// wants triplanar taps has nowhere to put them.
const NOISE_SAMPLE_BUDGET = Number(process.env.BROWSER_SMOKE_NOISE_SAMPLES || 8);
// Where the cost probe aims and how it sweeps. The bearing is the surface
// probe's steepest yaw at its own pitch — the view with the most ground in
// it, which is the view a fill measurement should be made from. Three scales
// so the fit has a degree of freedom left over; one frame per timed sample and
// the min of four of them, because contention on a shared box only ever adds
// time — the smallest observation is the least contaminated one, and averaging
// frames inside a sample only mixes the clean ones back in with the dirty.
// The probe measures its own resolution alongside these (its `control` run);
// what these numbers buy is not precision, it is knowing how little there is.
const COST_PROBE_YAW = (3 * Math.PI) / 2;
const COST_PROBE_PITCH = -0.7;
const COST_PROBE_SCALES = [1, 0.5, 0.25];
const COST_PROBE_FRAMES = 1;
const COST_PROBE_REPS = 4;
// Two separately compiled programs may schedule identical arithmetic in a
// different order, so the `nofield` and `nograin` variants are required to
// land on their uniform-zeroed image (`uSurface = 0` and `uGrain = 0`
// respectively) within one luma step rather than bit-exactly. Stated before
// the run, not fitted to it.
const COST_IDENTITY_MAX_DELTA = 1;
// How much of a timed frame must be the GPU rather than JS draw submission,
// as a ratio. A check on the instrument, not on this box's speed: the broken
// barrier this probe shipped with scored ~1x and a working one scores ~100x,
// so 2x separates them by a mile from either side and asserts no duration.
const COST_SYNC_MIN_RATIO = 2;
// The micro-octave skip gets a stricter tolerance than that, and it earns it:
// where the fade is nonzero both programs evaluate the SAME expression, and
// where it is zero the octave's contribution is multiplied by exactly 0. That
// is bit-exactness, so the gate asks for it rather than for something near it.
const COST_MICRO_SKIP_MAX_DELTA = 0;

const fail = (msg) => {
  console.error(`GATE FAIL: ${msg}`);
  process.exit(1);
};

// --- dependencies, each a loud failure -------------------------------------
const require = createRequire(path.join(root, "web/package.json"));
let chromium;
try {
  // playwright's entry is CJS: importing it puts the exports under .default,
  // and a bare specifier would resolve against ci/ rather than web/.
  const mod = await import(pathToFileURL(require.resolve("playwright")).href);
  chromium = mod.chromium ?? mod.default?.chromium;
  if (!chromium) throw new Error("playwright exported no chromium");
} catch (e) {
  fail(
    `playwright not installed (web devDependency): ${e.message}\n` +
      "  install it: (cd web && npm install --include=dev)",
  );
}
if (!fs.existsSync(SHARD)) {
  fail(`shard binary missing at ${SHARD}\n  build it: cargo build -p server --bin shard --release`);
}
if (!fs.existsSync(path.join(DIST, "index.html"))) {
  fail(`web bundle missing at ${DIST}\n  build it: (cd web && npx vite build)`);
}
if (!fs.existsSync(path.join(DIST, "client_wasm.wasm"))) {
  fail(`client_wasm.wasm missing from ${DIST} — the bundle is not playable`);
}

let server, browser, tmpDir;
const shards = [];
const cleanup = () => {
  try { browser && browser.close(); } catch {}
  try { server && server.close(); } catch {}
  for (const s of shards) { try { s.kill("SIGTERM"); } catch {} }
  try { tmpDir && fs.rmSync(tmpDir, { recursive: true, force: true }); } catch {}
};
process.on("exit", cleanup);

// --- real shards on real UDP ports ------------------------------------------
// `dev_spawn` set = a dev shard: spawns pinned to one point AND, from wire v7,
// a welcome that says `dev`. Unset = exactly a public shard's config.
tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "gates-smoke-"));
const startShard = (label, port, devSpawn) => {
  const cfgPath = path.join(tmpDir, `${label}.toml`);
  fs.writeFileSync(
    cfgPath,
    `bind = "127.0.0.1:${port}"\nseed = ${SEED}\n` +
      (devSpawn ? `dev_spawn = "${devSpawn}"\n` : ""),
  );
  const log = [];
  const proc = spawn(SHARD, [cfgPath], { cwd: root, env: { ...process.env, RUST_LOG: "warn" } });
  shards.push(proc);
  const ready = new Promise((resolve, reject) => {
    const t = setTimeout(() => reject(new Error(`${label} shard printed no cert hash in 30s`)), 30000);
    const feed = (d) => {
      for (const line of String(d).split("\n")) {
        if (!line) continue;
        log.push(line);
        const m = line.match(/dev cert sha256\s+([0-9a-fA-F:]+)/);
        if (m) { clearTimeout(t); resolve(m[1]); }
      }
    };
    proc.stdout.on("data", feed);
    proc.stderr.on("data", feed);
    proc.on("exit", (c) => reject(new Error(`${label} shard exited (${c}): ${log.join(" | ")}`)));
  });
  return { label, port, log, ready };
};
const devShard = startShard("dev", WIRE_PORT, DEV_SPAWN);
const publicShard = startShard("public", PUBLIC_WIRE_PORT, null);
let certHash = null;
let publicCertHash = null;
try {
  [certHash, publicCertHash] = await Promise.all([devShard.ready, publicShard.ready]);
} catch (e) {
  fail(e.message);
}

// --- serve the built bundle, with the production COOP/COEP headers ----------
const MIME = { ".html": "text/html", ".js": "text/javascript", ".wasm": "application/wasm", ".css": "text/css" };
server = http.createServer((req, res) => {
  const rel = decodeURIComponent(req.url.split("?")[0]);
  const file = path.join(DIST, rel === "/" ? "index.html" : rel);
  if (!file.startsWith(DIST) || !fs.existsSync(file) || fs.statSync(file).isDirectory()) {
    return res.writeHead(404).end("not found");
  }
  res.writeHead(200, {
    "content-type": MIME[path.extname(file)] || "application/octet-stream",
    "cross-origin-opener-policy": "same-origin",
    "cross-origin-embedder-policy": "require-corp",
  });
  fs.createReadStream(file).pipe(res);
});
await new Promise((r) => server.listen(PORT, "127.0.0.1", r));

// --- the browser ------------------------------------------------------------
try {
  browser = await chromium.launch({
    headless: true,
    // The same escape hatch `vantages.mjs` has had, under the same name, for
    // the same reason: a box whose installed Playwright browser build does not
    // match the revision `web/package.json` pins launches nothing, and a wall
    // that cannot run is not a wall (`CLAUDE.md`, on the wasm target). Unset
    // on the reference box and in CI, where the pinned build is present.
    ...(process.env.VANTAGE_CHROME ? { executablePath: process.env.VANTAGE_CHROME } : {}),
    args: [
      // No GPU on the reference box or this one: ANGLE over SwiftShader.
      "--enable-unsafe-swiftshader", "--use-gl=angle", "--use-angle=swiftshader", "--ignore-gpu-blocklist",
      // Two live contexts: neither may be throttled as "background" or the
      // idle one stops pumping inputs and its remote really does freeze.
      "--disable-renderer-backgrounding", "--disable-backgrounding-occluded-windows", "--disable-background-timer-throttling",
    ],
  });
} catch (e) {
  fail(`chromium failed to launch: ${e.message}\n  install it: npx playwright install chromium`);
}

// --- the reference bar, measured before anything else runs -------------------
// `art/RUBRIC.md` scores our frames against `Rust Images/` as an absolute bar,
// and assertion 17 below is the only gate in this file that asserts an
// absolute. Its floors were derived from these numbers, so they are re-read
// here every run rather than remembered: if the reference set is ever changed,
// the floors stop being below it and 17 says so.
//
// Measured HERE, before a single game tab exists, and in a context that is
// closed immediately: this box's join times are monotonic in live tabs (0.4 s
// alone, 34-36 s beside one), so a decode context held open beside two game
// tabs would be spending the thinnest margin in the suite on a JPEG.
let referenceBar;
{
  const ctx = await browser.newContext();
  const p = await ctx.newPage();
  try {
    referenceBar = await measureReference(p);
  } catch (e) {
    fail(`reference bar: ${e.message}`);
  }
  await ctx.close();
}
console.log(
  `  reference bar (${REFERENCE_FRAMES.length} frames of Rust Images/, median): ` +
    `p10 ${referenceBar.p10} · p50 ${referenceBar.p50} · p90 ${referenceBar.p90} · ` +
    `range ${referenceBar.range} · sky ${referenceBar.skyMean.toFixed(1)} ` +
    `in ${referenceBar.skyLevels} levels`,
);

// One context per tab: separate sessions, separate localStorage — the same
// isolation two real players have.
//
// How many of these are ALIVE AT ONCE is load-bearing on this box and is
// tracked, not assumed. Every tab rasterizes in software (SwiftShader, no
// GPU here or on the reference VPS), each renderer runs its own worker
// threads, and the cores are shared with a live game stack (eight here; the
// numbers below were measured on the morr box's four and still bind). The
// join time is monotonic in the count, measured over this gate's own history:
// one live tab joins in 0.4 s, two in 34-36 s, three in 55-61 s. The third
// reading is the one that went over the 60 s window on 2026-08-01 16:26 and
// reddened the wall.
const liveTabs = new Set();
const join = async (label, port = WIRE_PORT, cert = certHash) => {
  const context = await browser.newContext({ viewport: { width: 1280, height: 720 } });
  const page = await context.newPage();
  liveTabs.add(label);
  const errors = [];
  page.on("pageerror", (e) => errors.push(`pageerror: ${e.message}`));
  page.on("console", (m) => { if (m.type() === "error") errors.push(`console.error: ${m.text()}`); });

  await page.goto(`http://127.0.0.1:${PORT}/`, { waitUntil: "load" });
  await page.fill("#url", `https://127.0.0.1:${port}`);
  await page.fill("#cert", cert);
  await page.click("#connect");

  // Assertion 1 — the client reaches the world. Catches bug 1, and any
  // handshake, ring, AOI or delta-encoder break along the way.
  //
  // The poll is shaped like this on purpose. It used to make TWO
  // `page.evaluate` round trips per iteration — the whole `__gatesDebug`
  // object, then `#starterr` — and the first of those grew every time a slice
  // added facts to the debug publisher (the lighting rig, the clipmap's
  // per-level facts, the material system's identities and program stats, the
  // scatter pools). On this box the third live tab's renderer gets a fraction
  // of a core, an evaluate queues behind whatever frame it lands in, and the
  // measured result was **2 polls in 62.6 seconds**: the gate spent its whole
  // 60 s window asking, and asked twice.
  //
  // That is an instrument failure and it reads exactly like a client that
  // never joined. It is what killed the first grain attempt, and the page
  // state printed on failure below is what finally said so — form hidden,
  // `__gatesDebug` present, client in the world, nobody looking.
  //
  // Two repairs, and neither touches what is asserted or how long it is
  // allowed to take:
  //
  //   1. ONE round trip per look, carrying only the fields the join condition
  //      is written against instead of every fact the client publishes. The
  //      refusal check rides along in it rather than paying for its own.
  //   2. The looks do not QUEUE BEHIND EACH OTHER. Serialized, a look costs
  //      its own latency before the next one starts, so a 20 s round trip
  //      buys three looks in a minute no matter how early the client was
  //      ready. With up to JOIN_POLL_INFLIGHT outstanding the answer arrives
  //      when the renderer next has a slice for it, so the join is observed
  //      within one round trip of becoming true instead of within one round
  //      trip of the next poll's turn. JOIN_POLL_MS is a FLOOR on the gap
  //      between launches, not a cadence: the race also wakes on every look
  //      that settles, so a fast tab launches its four immediately and a
  //      starved one paces itself at 250 ms until the window is full.
  //
  // The full object is fetched once, after the wait, for the caller.
  const t0 = Date.now();
  const look = () =>
    page.evaluate(() => {
      const d = globalThis.__gatesDebug;
      return {
        inWorld: d ? d.inWorld : null,
        snapshots: d ? d.snapshots : 0,
        starterr: document.getElementById("starterr")?.textContent || "",
      };
    });
  const inflight = new Set();
  let brief = null;
  let ready = null;
  let polls = 0;
  while (Date.now() - t0 < JOIN_TIMEOUT_MS && !ready) {
    if (inflight.size < JOIN_POLL_INFLIGHT) {
      polls++;
      const p = look()
        .then((r) => {
          brief = r;
          if (r.inWorld && r.snapshots > 0) ready = r;
          return r;
        })
        // A page that went away mid-look is not a signal; the loop's own
        // deadline and the diagnostic below are what report that.
        .catch(() => null)
        .finally(() => inflight.delete(p));
      inflight.add(p);
    }
    await Promise.race([
      ...inflight,
      new Promise((r) => setTimeout(r, JOIN_POLL_MS)),
    ]);
    // #starterr, where boot() puts failures. A refusal is permanent, so
    // seeing it once is enough.
    if (brief && brief.starterr) fail(`${label}: client refused to boot: ${brief.starterr}`);
  }
  const dbg = ready ? await page.evaluate(() => globalThis.__gatesDebug || null) : null;
  if (!dbg || !dbg.inWorld) {
    // Say WHY. "never reached the world" alone is the same message for a
    // handshake that never landed, a shard that died, and a client whose
    // debug publisher threw on its first tick — three very different bugs.
    //
    // And say WHERE, which is the part this cost two passes. `boot()` hides
    // the #start form as its last act before calling `run()`, and `run()`
    // registers the 250 ms publisher — so the form's own display property is
    // a boot-stage marker that has been sitting in the DOM all along, needing
    // no debug hook and no change to the shipped client. Still visible means
    // boot is stuck in wasm load, connect or handshake; already hidden means
    // the client is in the world and its main thread is too busy to publish.
    // The poll count separates both from the third reading — a gate whose own
    // `page.evaluate` starved and never asked (NOW.md item 1: one tab got two
    // polls in sixty seconds).
    const post = await page
      .evaluate(() => ({
        ready: document.readyState,
        formShown: (document.getElementById("start")?.style.display || "") !== "none",
        starterr: document.getElementById("starterr")?.textContent || "",
        hasDebug: typeof globalThis.__gatesDebug,
      }))
      .catch((e) => ({ evaluateFailed: String(e && e.message ? e.message : e) }));
    fail(
      `${label}: never reached the world in ${JOIN_TIMEOUT_MS}ms ` +
        `(__gatesDebug ${
          brief && brief.inWorld !== null
            ? `present, inWorld=${brief.inWorld}, snapshots=${brief.snapshots}`
            : "never published"
        })` +
        `\n    ${polls} looks launched in ${((Date.now() - t0) / 1000).toFixed(1)}s, ` +
        `${inflight.size} still outstanding · ${liveTabs.size} tab(s) live · ` +
        `page state ${JSON.stringify(post)}` +
        (errors.length ? `\n` + errors.slice(0, 8).map((e) => `    ${e}`).join("\n") : "\n    no page errors"),
    );
  }
  // The look count rides along on the PASSING path too, because the instrument
  // starving is a slow failure and a run that squeaked in on its third look is
  // one slice away from a run that never looks at all. The live-tab count rides
  // along for the same reason from the other side: it is what the join time is
  // a function of, so the two belong on one line.
  console.log(
    `  ${label}: in world as player ${dbg.playerId} at [${dbg.own.map((v) => v.toFixed(1))}] ` +
      `(seen ${((Date.now() - t0) / 1000).toFixed(1)}s in, ${polls} looks launched, ` +
      `${liveTabs.size} tab${liveTabs.size === 1 ? "" : "s"} live)`,
  );
  return {
    label,
    page,
    errors,
    playerId: dbg.playerId,
    dbg,
    close: async () => {
      await context.close();
      liveTabs.delete(label);
    },
  };
};

const A = await join("tab A");
const B = await join("tab B");
if (A.playerId === B.playerId) fail(`both tabs joined as player ${A.playerId}`);

// Each page must see the other's remote before anyone moves — dev_spawn puts
// them on the same point, 0 m apart, far inside the 176 m AOI enter.
const remoteOf = (tab, id) =>
  tab.page.evaluate(
    (want) => (globalThis.__gatesDebug?.remotes || []).find((r) => r[0] === want) || null,
    id,
  );
const waitForRemote = async (tab, id) => {
  const t0 = Date.now();
  while (Date.now() - t0 < JOIN_TIMEOUT_MS) {
    const r = await remoteOf(tab, id);
    if (r) return r;
    await tab.page.waitForTimeout(250);
  }
  fail(`${tab.label}: never saw player ${id} in AOI — both spawned at dev_spawn ${DEV_SPAWN}`);
};
const seenA = await waitForRemote(A, B.playerId); // A sees B
const seenB = await waitForRemote(B, A.playerId); // B sees A
console.log(`  mutual AOI: A sees ${B.playerId}, B sees ${A.playerId}`);

// --- vitals: the three bars the shard states at the door -------------------
// Assertion 2a — a fresh spawn's health AND its two survival meters reach the
// DOM, and every number they show is the one in `content/balance.toml`, read
// here rather than typed: the whole chain content → bake → sim → wire v14 →
// wasm → HUD is what this asserts, and a gate that hardcoded 100 would keep
// passing after a balance pass moved it. Observable state, polled — never an
// elapsed-ms bar.
//
// Three rows now, keyed off the fill class rather than row order, so a HUD
// that reordered the stack still has to put the right number in the right
// meter. The clock's spans are tens of minutes, so a fresh spawn's meters are
// still at their ceilings when this runs — which is the point: what is being
// asserted is that the sim GRANTED and ANNOUNCED them at the door, not that
// they drain (the drain is exact integer arithmetic and is gated as such in
// sim-core, where it does not need a browser or a clock).
{
  const balance = fs.readFileSync(path.join(root, "content/balance.toml"), "utf8");
  const num = (re, what) => {
    const v = Number(re.exec(balance)?.[1]);
    if (!Number.isFinite(v) || v <= 0) {
      fail(`content/balance.toml states no ${what} — the vitals assertion cannot run`);
    }
    return v;
  };
  const want = {
    health: num(/^player_hp\s*=\s*(\d+)/m, "player_hp"),
    water: num(/^max_water\s*=\s*(\d+)/m, "max_water"),
    food: num(/^max_food\s*=\s*(\d+)/m, "max_food"),
  };
  const vitals = (tab) =>
    tab.page.evaluate(() => {
      const el = document.getElementById("vitals");
      if (!el || el.style.display !== "block") return { shown: false, rows: {} };
      const rows = {};
      for (const row of el.querySelectorAll(".vrow")) {
        if (row.style.display === "none") continue;
        const fill = row.querySelector(".vfill");
        const num = row.querySelector(".vnum");
        if (!fill || !num) continue;
        const kind = fill.classList.contains("water")
          ? "water"
          : fill.classList.contains("food")
            ? "food"
            : "health";
        rows[kind] = num.textContent.trim();
      }
      return { shown: true, rows };
    });
  for (const tab of [A, B]) {
    let v = await vitals(tab);
    for (let i = 0; i < 40 && Object.keys(v.rows).length < 3; i++) {
      await tab.page.waitForTimeout(250);
      v = await vitals(tab);
    }
    if (!v.shown) {
      fail(`tab ${tab.playerId}: the vitals stack never appeared — no health reached the HUD`);
    }
    // The survival clock made "HUD equals content's starting value" a
    // TIME-DEPENDENT assertion — water drains on the sim's schedule, so the
    // value here depended on how many ticks elapsed before this read, and on
    // 2026-08-03 06:29 the same trunk went red then green on it with no
    // change (the FLAKY-GATE stop this comment is the fix for). Split into
    // the two questions it was conflating, both on observable state:
    //   1. the HUD displays the SIM'S OWN number, exactly (__gatesDebug
    //      .vitals is the client core's authoritative mirror) — that is the
    //      "number the shard plays" half, sharp at any tick;
    //   2. the played number sits inside content's declared bounds (> 0,
    //      <= the declared start) — the "data declares" half at this gate's
    //      altitude. That it STARTS at exactly the declared value is the
    //      sim's own tests' job (survival unit tests + parity/replay), where
    //      the tick is controlled and equality is deterministic.
    const simV = await tab.page.evaluate(() => globalThis.__gatesDebug.vitals);
    if (!simV) fail(`tab ${tab.playerId}: __gatesDebug.vitals missing — the HUD gate has no comparand`);
    const simRows = { health: simV[0], food: simV[2], water: simV[4] };
    for (const kind of ["health", "water", "food"]) {
      if (v.rows[kind] === undefined) {
        fail(
          `tab ${tab.playerId}: the vitals stack has no ${kind} row — ` +
            "the shard never stated that meter at the door",
        );
      }
      if (v.rows[kind] !== String(simRows[kind])) {
        fail(
          `tab ${tab.playerId}: ${kind} HUD reads "${v.rows[kind]}" while the sim holds ` +
            `${simRows[kind]} — the display is not the number the shard plays`,
        );
      }
      if (simRows[kind] <= 0 || simRows[kind] > want[kind]) {
        fail(
          `tab ${tab.playerId}: ${kind} is ${simRows[kind]} against a declared start of ` +
            `${want[kind]} — outside content's bounds at the door`,
        );
      }
    }
  }
  console.log(
    `  vitals: both tabs' HUD matches the sim's own meters exactly, inside ` +
      `content's declared bounds (${want.health} hp · ${want.water} water · ` +
      `${want.food} food at the door; the clock may have drained a step)`,
  );
}

// --- the backpack lane: wired, and reconciling from zero --------------------
// Assertion 2b — the death-backpack lane is present in a real browser and the
// interact key runs its loot path without throwing. Scope, stated plainly:
// nobody dies in this gate (the shipped content arms no weapon a fresh spawn
// holds), so this asserts the WIRING — the client's bag set exists, the
// renderer's mesh map exists, the two agree at zero, and pressing E walks
// `tryLoot` in the real RAF/keydown path. What a bag DOES is asserted where
// it can be: `backpack_wire` drives kill → drop → loot → removal through real
// encoded bytes, and `client_smoke` hand-frames all three subtypes into the
// raw C ABI. A JS throw on that path is what this catches and they cannot.
{
  for (const tab of [A, B]) {
    const before = await tab.page.evaluate(() => {
      const d = globalThis.__gatesDebug;
      return d ? { known: d.bagsKnown, drawn: d.bags } : null;
    });
    if (!before) fail(`tab ${tab.playerId}: __gatesDebug vanished before the backpack check`);
    if (before.known !== 0 || before.drawn !== 0) {
      fail(
        `tab ${tab.playerId}: a fresh shard has ${before.known} bags known and ` +
          `${before.drawn} drawn — nobody has died`,
      );
    }
    await tab.page.keyboard.press("KeyE");
    // Settle on observable state, never a clock: the debug snapshot
    // republishes on its own timer, so poll it until it moves past the
    // press rather than sleeping a guessed number of milliseconds.
    let after = null;
    for (let i = 0; i < 40; i++) {
      after = await tab.page.evaluate(() => {
        const d = globalThis.__gatesDebug;
        return d ? { known: d.bagsKnown, drawn: d.bags, inWorld: d.inWorld } : null;
      });
      if (after && after.inWorld) break;
      await tab.page.waitForTimeout(100);
    }
    if (!after || !after.inWorld) {
      fail(`tab ${tab.playerId}: the loot key took the tab out of the world`);
    }
    if (after.known !== 0 || after.drawn !== 0) {
      fail(
        `tab ${tab.playerId}: pressing loot with nothing in reach conjured ` +
          `${after.known} known / ${after.drawn} drawn bags`,
      );
    }
  }
  console.log("  backpack: the loot key runs and the bag set holds at zero in both tabs");
}

// --- chat, part 1: heard at the spawn --------------------------------------
// Assertion 2 — a line typed into the real composer in one browser reaches
// the other browser's log. Driven entirely through the UI (T, type, Enter)
// and read back off the DOM: no test-only hook, so what passes here is the
// path a player uses. Said HERE, before the walk, because the walk carries
// the two tabs past the 20 m local radius — which part 2 then relies on.
const chatlog = (tab) =>
  tab.page.evaluate(() => document.getElementById("chatlog").textContent || "");

const say = async (tab, text) => {
  await tab.page.keyboard.press("KeyT");
  await tab.page.keyboard.type(text);
  await tab.page.keyboard.press("Enter");
};

// `want` in the log within the window, or null. Polls, because the line
// crosses a real network and a real tick.
const waitForLine = async (tab, want, ms = 5000) => {
  const until = Date.now() + ms;
  for (;;) {
    const log = await chatlog(tab);
    if (log.includes(want)) return log;
    if (Date.now() > until) return null;
    await tab.page.waitForTimeout(250);
  }
};

const ownXZ = async (tab) => {
  const own = await tab.page.evaluate(() => globalThis.__gatesDebug.own);
  return [own[0], own[2]];
};
const apartNow = async () => {
  const [ax, az] = await ownXZ(A);
  const [bx, bz] = await ownXZ(B);
  return Math.hypot(bx - ax, bz - az);
};

const LOCAL_LINE = "stone at the ridge";
await say(A, LOCAL_LINE);
const heardLocalB = await waitForLine(B, LOCAL_LINE);
if (!heardLocalB) {
  fail(`tab B never heard A's local line from ${(await apartNow()).toFixed(1)} m away`);
}
if (!heardLocalB.includes(`#${A.playerId}`)) {
  fail(`tab B heard the line but not from #${A.playerId}: ${heardLocalB.trim()}`);
}
const heardLocalA = await waitForLine(A, LOCAL_LINE);
if (!heardLocalA) fail(`tab A never got its own echo — the delivery receipt is missing`);
console.log(`  chat: A's local line reached B, and A's own echo came back`);

// --- the death screen: built, wired, and closed while you are standing -----
//
// Assertion 17. Structural, and the limit is worth stating rather than
// implying: **no gate on this box drives a browser death.** The two ways a
// body can die are another player's hand, which needs a weapon neither tab
// can gather inside this suite, and the sea — and the sea refuses a drink
// into a full meter (survival.rs), so salt suicide runs at the speed thirst
// drains, which is minutes. What the sim does with a death is owned
// natively (`bag_respawn.rs`, `alloc_zero`, `test_replay`); what the wire
// carries is owned by `test_protocol_golden`; what the client's decoder
// does with those bytes is owned by `client_smoke.mjs`, which hand-frames a
// Death naming its own player id and reads the screen back out of the C
// ABI. The one link none of those three can see is this one: that the
// overlay exists in the shipped page, that it is *closed* while the player
// is alive, and that the action its buttons send encodes through the real
// bridge in a real browser.
const death = await A.page.evaluate(() => {
  const el = document.getElementById("death");
  if (!el) return null;
  const style = getComputedStyle(el);
  return {
    display: style.display,
    // The two buttons and the cause line, by id — a screen that lost one
    // of them is a screen a dead player cannot answer.
    bag: !!document.getElementById("respawnbag"),
    beach: !!document.getElementById("respawnbeach"),
    cause: !!document.getElementById("deathcause"),
    // The client's own view of it, and the wasm encoder behind the buttons.
    open: globalThis.__gatesDebug.deathOpen,
    bagLen: globalThis.__gatesDebug.encodeRespawn(1),
    beachLen: globalThis.__gatesDebug.encodeRespawn(0),
    bagByte: globalThis.__gatesDebug.encodeRespawnByte(1),
    beachByte: globalThis.__gatesDebug.encodeRespawnByte(0),
  };
});
if (!death) fail("tab A: no #death overlay in the page — the death screen never shipped");
if (!(death.bag && death.beach && death.cause)) {
  fail(
    `tab A: the death screen is missing a part (bag=${death.bag} beach=${death.beach} ` +
      `cause=${death.cause}) — a dead player could not answer it`,
  );
}
if (death.display !== "none") {
  fail(`tab A: the death screen is showing at display=${death.display} on a live body`);
}
if (death.open !== false) fail(`tab A: the client thinks a standing body is dead`);
// ACTION(6) in three bits, sub 11 in four, the choice bit at 7 — one byte
// either way, and the two answers must differ in exactly that bit.
if (death.bagLen !== 1 || death.beachLen !== 1) {
  fail(
    `tab A: respawn action is ${death.bagLen}/${death.beachLen} bytes, not 1 — ` +
      `the browser's encoder disagrees with the wire`,
  );
}
const wantBeach = 6 | (11 << 3);
if (death.beachByte !== wantBeach || death.bagByte !== (wantBeach | 0x80)) {
  fail(
    `tab A: respawn bytes are ${death.beachByte}/${death.bagByte}, want ` +
      `${wantBeach}/${wantBeach | 0x80} — the choice bit is not on the wire`,
  );
}
console.log(
  `  death screen: built, closed on a live body, and both answers encode ` +
    `(${death.beachByte} beach / ${death.bagByte} bag)`,
);

// --- lighting: the key light and the shadow map -----------------------------
// Measured HERE, before the walk, on purpose: tab A is still standing on
// `dev_spawn` at a pinned seed, so the frames the probe scores are the same
// frames every pass. After the walk the position depends on how much wall
// clock a shared box gave the input pump, and the floor below would be
// scoring a different place each run.
//
// Assertion 9 — the rig is wired. Cheap and structural, and it exists so that
// assertion 10's failure is diagnosable: "no darkening" means something very
// different when the shadow map was never enabled in the first place.
await A.page.waitForTimeout(SHADOW_SETTLE_MS); // the near ring streams in
const lit = await A.page.evaluate(() => globalThis.__gatesDebug.lighting);
if (!lit) fail(`tab A: __gatesDebug.lighting missing — the scene publishes no lighting state`);
if (lit.shadowMap !== true) fail(`tab A: renderer.shadowMap.enabled is ${lit.shadowMap}`);
if (lit.sunCasts !== true) fail(`tab A: the key light has castShadow=${lit.sunCasts}`);
if (!(lit.mapSize >= SHADOW_MIN_MAP_PX)) {
  fail(`tab A: shadow map is ${lit.mapSize} px — below ${SHADOW_MIN_MAP_PX}, silhouettes cannot resolve`);
}
if (!(lit.radiusM > 0)) fail(`tab A: shadow coverage radius is ${lit.radiusM} m`);
if (!(lit.normalBias > 0)) fail(`tab A: shadow normalBias is ${lit.normalBias} — acne is not being biased away`);
// Tone mapping is the other half of "grade toward a darker edge": 0 is
// THREE.NoToneMapping, i.e. nothing owns the highlight roll-off.
if (!(lit.toneMapping > 0)) fail(`tab A: renderer.toneMapping is NoToneMapping — nothing owns the transfer`);
if (!(lit.exposure > 0)) fail(`tab A: toneMappingExposure is ${lit.exposure}`);
// The fill must not be able to wash the key out; a fill at or above the key
// is the flat-hemisphere state this work exists to end.
if (!(lit.sunIntensity > lit.fillIntensity)) {
  fail(
    `tab A: key ${lit.sunIntensity} is not above fill ${lit.fillIntensity} — ` +
      `a fill that strong flattens every shadow the map draws`,
  );
}

// Assertion 9b — the clipmap is a clipmap. Structural, and it exists for the
// same reason as 9: so 11b's failure is diagnosable. Every check here is on a
// property the reference names — concentric levels, a real guard band, bias
// scaled BY texel width rather than shared across levels, and exactly one
// level carrying the key's energy.
const cm = lit.clipmap;
if (!cm) fail(`tab A: lighting publishes no clipmap state — the levels are not wired`);
if (!(cm.levelCount >= 2)) {
  fail(`tab A: the shadow clipmap has ${cm.levelCount} level(s) — nothing past the near box casts`);
}
if (cm.levels.length !== cm.levelCount) {
  fail(`tab A: clipmap says ${cm.levelCount} levels but published ${cm.levels.length}`);
}
if (cm.activeLevels !== cm.levelCount) {
  fail(
    `tab A: ${cm.activeLevels} of ${cm.levelCount} clipmap levels are contributing — ` +
      `a probe left the scene with levels switched off`,
  );
}
if (cm.levels[0].halfWidthM !== lit.radiusM) {
  fail(`tab A: level 0 is ${cm.levels[0].halfWidthM} m but lighting reports radius ${lit.radiusM} m`);
}
// One bias in texels across every level, which is what "scale normal bias by
// world texel width" means in a number: same texel count, different metres.
const biasTexels = cm.levels.map((L) => L.normalBias / L.texelM);
for (let i = 0; i < cm.levels.length; i++) {
  const L = cm.levels[i];
  if (i > 0 && !(L.halfWidthM > cm.levels[i - 1].halfWidthM)) {
    fail(`tab A: clipmap level ${i} (${L.halfWidthM} m) does not contain level ${i - 1}`);
  }
  if (i > 0 && !(L.texelM > cm.levels[i - 1].texelM)) {
    fail(`tab A: clipmap level ${i} texel ${L.texelM} m is not coarser than level ${i - 1}`);
  }
  // The ortho frustum has to contain the ground its own box covers. A level
  // bounds light-space X and Y; how much light-space DEPTH the ground inside
  // those bounds spans is set by how low the sun is — half·cot(elevation) each
  // way, which at a 21° sun is 2.66 m per metre of half-width. A flat depth
  // (lighting v0 carried 260 m below the centre for every level) is right for
  // one size and clips the far half of every larger one: level 2's 720 m box
  // needs 1913 m. Nothing used to live out there, so nothing showed it.
  const boxDepthM = 2 * L.halfWidthM * (Math.cos(lit.sunElevation) / Math.sin(lit.sunElevation));
  if (!(L.farM >= boxDepthM)) {
    fail(
      `tab A: clipmap level ${i}'s ortho depth is ${L.farM.toFixed(0)} m against the ` +
        `${boxDepthM.toFixed(0)} m its own ${L.halfWidthM} m box spans along the light at a ` +
        `${((lit.sunElevation * 180) / Math.PI).toFixed(0)}° sun — the far half of the level ` +
        `clips out and casts nothing`,
    );
  }
  if (!(L.sampledHalfWidthM < L.halfWidthM)) {
    fail(`tab A: clipmap level ${i} samples its full ${L.halfWidthM} m — no guard band, so a PCF tap can leave the map`);
  }
  if (L.casts !== true) fail(`tab A: clipmap level ${i} does not cast — it has no depth texture`);
  if (!(L.mapPx >= SHADOW_MIN_MAP_PX)) {
    fail(`tab A: clipmap level ${i} is ${L.mapPx} px — below ${SHADOW_MIN_MAP_PX}, silhouettes cannot resolve`);
  }
  if (!L.valid || !(L.renders > 0)) {
    fail(`tab A: clipmap level ${i} has never rendered (valid=${L.valid}, renders=${L.renders})`);
  }
  // Exactly one level lights the scene. Two would double the key.
  const wantsIntensity = i === 0;
  if (wantsIntensity !== L.intensity > 0) {
    fail(
      `tab A: clipmap level ${i} has intensity ${L.intensity} — only level 0 may carry the key, ` +
        `or N levels light the scene N times over`,
    );
  }
  if (Math.abs(biasTexels[i] - biasTexels[0]) > 1e-6) {
    fail(
      `tab A: clipmap level ${i}'s normal bias is ${biasTexels[i].toFixed(3)} texels against level 0's ` +
        `${biasTexels[0].toFixed(3)} — one metre value across ${cm.levels[0].texelM.toFixed(3)} m and ` +
        `${L.texelM.toFixed(3)} m texels is not a coherent bias`,
    );
  }
}
// The coarse levels are CACHED, not redrawn every frame — the whole reason
// the draw budget survives adding them. Counted over the session, not
// asserted from a flag: level 0 is dynamic and redraws every frame, so a
// coarse level that matched it would be uncached.
for (let i = 1; i < cm.levels.length; i++) {
  if (!(cm.levels[i].renders < cm.levels[0].renders)) {
    fail(
      `tab A: clipmap level ${i} has rendered ${cm.levels[i].renders} times against level 0's ` +
        `${cm.levels[0].renders} — the coarse level is not being cached at all`,
    );
  }
}
// Frame time is reported, never asserted: this box shares its eight cores
// with a live game stack and the gate's renderer is a software rasterizer,
// so the number is a same-box regression signal and not a claim about the
// reference VPS. It is printed because the clipmap is the kind of change that
// pays for coverage with fill rate, and a doubled frame time should be
// visible to whoever reads this log rather than only to the flaky assertions
// it would eventually start tripping.
const frameA = await A.page.evaluate(() => globalThis.__gatesDebug.frameMs);
console.log(
  `  clipmap: ${cm.levelCount} levels ` +
    cm.levels
      .map(
        (L) =>
          `[${L.halfWidthM}m/${L.texelM.toFixed(3)}m px @${L.mapPx}, bias ${L.normalBias.toFixed(3)}m, ` +
          `${L.filterTaps} taps, ${L.renders} renders]`,
      )
      .join(" ") +
    `, budget ${cm.updateBudget}/frame, max age ${cm.maxCacheAge}` +
    ` · frame ~${frameA.toFixed(1)} ms (shared box, software GL — a trend, not a claim)`,
);

// The distribution behind that number, same status (reported, never
// asserted): the smoothed frame time is exactly the instrument that hides
// compile stalls, so the log carries the tail too.
const pctA = await A.page.evaluate(() => globalThis.__gatesDebug.framePct);
if (pctA)
  console.log(
    `  frame dist: p50 ${pctA.p50.toFixed(1)} · p95 ${pctA.p95.toFixed(1)} · ` +
      `p99 ${pctA.p99.toFixed(1)} · worst ${pctA.worst.toFixed(1)} ms over the last 240 frames`,
  );

// --- shader prewarm: nothing links after the snapshot ------------------------
// CLAUDE.md trap (Claude-of-Duty postmortem): median frame time hides shader
// compile stalls — elsewhere, 30+ lazy links cost 700 ms+ worst-frames behind
// a 90+ fps benchmark. The client prewarms every program it can wear
// (scene.prewarm(): color programs at boot, the depth program over the first
// in-world frames), and this asserts the result as a COUNT on observable
// state, never a clock. The window: by HERE both tabs have joined, walked,
// seen a remote enter the AOI, and chatted — real play — and every section
// BELOW this line compiles gate instruments on purpose (flatgrain, the cost
// variants), so the assert seam closes exactly here, after play and before
// the first instrument.
for (const [tab, name] of [
  [A, "tab A"],
  [B, "tab B"],
]) {
  const p = await tab.page.evaluate(() => [
    globalThis.__gatesDebug.programs,
    globalThis.__gatesDebug.programsAtInWorld,
    globalThis.__gatesDebug.latePrograms,
    globalThis.__gatesDebug.pinnedPrograms,
    globalThis.__gatesDebug.pinStamp,
    globalThis.__gatesDebug.programLog,
  ]);
  if (!Number.isFinite(p[0]) || !Number.isFinite(p[1]))
    fail(`${name}: __gatesDebug carries no program counts — the prewarm gate cannot run`);
  if (p[1] < 0)
    fail(
      `${name}: programsAtInWorld was never pinned — the client reached this point ` +
        `without three in-world frames, which no walking tab can do`,
    );
  if (p[0] !== p[1])
    fail(
      `${name}: ${p[0] - p[1]} program(s) linked AFTER the in-world snapshot ` +
        `(${p[1]} pinned, ${p[0]} now) — some material's first draw came mid-play; ` +
        `add it to scene.prewarm(). Late cache keys:\n` +
        (p[2] || []).map((k) => `    LATE ${k.slice(0, 300)}`).join("\n") +
        `\n  pinned depth-family keys for comparison:\n` +
        (p[3] || [])
          .map((k) => `    PIN  ${k.slice(0, 240)}`)
          .join("\n") +
        `\n  link log [stamp, programs], pin at stamp ${p[4]}: ${JSON.stringify(p[5])}`,
    );
}
console.log(`  prewarm: 0 program links after the in-world snapshot, both tabs`);

// --- the daylight register (DECISIONS.md §open, "the daylight register") ----
// Assertion 16 — three counted claims about the light this world is seen in,
// each measured as a difference between two renders of the live scene.
//
// Measured HERE, before the walk, for the reason assertion 10 states above
// it: tab A is still standing on `dev_spawn` at a pinned seed, so the frames
// this scores are the same frames every pass — and they are the frames the
// capture harness shoots, which is where the report that asked for this work
// took its own measurements.
//
// The structural half first, so that a numeric failure below is diagnosable:
// "the sky is not above the ground" means something very different when the
// dome is still a vertex ramp or the fog near plane is still outside the ring.
const air = (await A.page.evaluate(() => globalThis.__gatesDebug.lighting)).air;
const skyFacts = (await A.page.evaluate(() => globalThis.__gatesDebug.lighting)).sky;
if (!air) fail(`tab A: the scene publishes no fog — there is no air to grade`);
if (!(air.near < air.ringM)) {
  fail(
    `tab A: the fog near plane is ${air.near} m against a ${air.ringM} m near ring — ` +
      `no pixel a standing player can see reaches the ramp, which is aerial perspective ` +
      `that exists only in the source (this shipped at 180 m against 160 m)`,
  );
}
if (!(air.far > air.near)) fail(`tab A: fog far ${air.far} m is not past near ${air.near} m`);
if (!skyFacts.patched) {
  fail(`tab A: the sky dome carries no fragment program — a 24×16 vertex ramp cannot hold a sun disc, and it bands`);
}
if (skyFacts.vertexColors) {
  fail(`tab A: the sky dome still has vertexColors on — the ramp is being reconstructed by the rasterizer`);
}
if (!(skyFacts.dither > 0)) fail(`tab A: the sky dither is ${skyFacts.dither} — the largest flat region in the frame is undithered`);
if (!(skyFacts.discGain > 0 && skyFacts.discRad[0] > 0 && skyFacts.discRad[1] > skyFacts.discRad[0])) {
  fail(`tab A: the sun disc is [${skyFacts.discRad}] at gain ${skyFacts.discGain} — the sky has no source for the light in it`);
}
// The measured half, swept twice — and the two sweeps are not redundant.
//
// **At the eye** is where the register and the ambient floor are true or not:
// it is the height a player plays at, the height the capture harness shoots
// from, and the height every measurement in the report that asked for this
// work was taken at.
//
// **Lifted** is the only place a question about DISTANCE has enough pixels to
// answer it, and this file has already made that argument twice — the far
// shadow and horizon probes both lift 80 m with the same sentence: from eye
// height the same probe measured almost nothing, not because the effect was
// absent but because the geometry puts it in a band a few pixels tall under
// the horizon. Ours is thinner still. At 1.6 m the ground 100 m out sits
// 0.016 rad below the horizon, which is 1.2% of a 75-degree frame's height;
// at 40 m it sits 0.38 rad down, which is 29% of it.
const daylightHook = await A.page.evaluate(() => typeof globalThis.__gatesDebug.daylightProbe);
if (daylightHook !== "function") {
  fail(`tab A: __gatesDebug.daylightProbe is ${daylightHook} on a dev shard — the daylight gate cannot run`);
}
const daySweep = async (pitch, heightM) =>
  A.page.evaluate(
    ([yaws, p, minDelta, hM]) => globalThis.__gatesDebug.daylightProbe(yaws, p, minDelta, hM),
    [DAYLIGHT_PROBE_YAWS, pitch, DAYLIGHT_MIN_DELTA, heightM],
  );
const dayEye = await daySweep(DAYLIGHT_PROBE_PITCH, 0);
const dayAir = await daySweep(DAYLIGHT_AIR_PITCH, DAYLIGHT_AIR_HEIGHT_M);
if (!dayEye || !dayAir) fail(`tab A: daylightProbe returned null`);
// Printed BEFORE the assertions, which the rest of this file does the other
// way round. Every number here is new, so the first thing a failing pass
// needs is the whole table and not only the row that tripped.
console.log(
  `  daylight: fog ${air.near}-${air.far} m inside a ${air.ringM} m ring, dome patched, ` +
    `disc ${skyFacts.discRad[0]}-${skyFacts.discRad[1]} rad @${skyFacts.discGain}, dither ${skyFacts.dither}`,
);
for (const [label, r] of [
  ["eye", dayEye],
  ["air", dayAir],
]) {
  for (const s of r.samples) {
    console.log(
      `    ${label} +${r.heightM}m yaw ${s.yaw.toFixed(2)}: sky ${s.skyLuma.toFixed(1)} (${(s.skyFraction * 100).toFixed(0)}%) ` +
        `vs ground med ${s.groundMedian} / mean ${s.groundLuma.toFixed(1)} / p90 ${s.groundP90} ` +
        `-> x${(s.skyLuma / Math.max(s.groundMedian, 1)).toFixed(2)} · air ${(s.fogFraction * 100).toFixed(2)}% ` +
        `up ${(s.fogUpShare * 100).toFixed(1)}% lift ${s.fogMeanLift.toFixed(2)} max ${s.fogMaxDelta} · ` +
        `f ${s.bands.map((b) => b.f.toFixed(3)).join("->")} luma ${s.bands.map((b) => b.luma.toFixed(1)).join("->")} ` +
        `lift ${s.bands.map((b) => b.lift.toFixed(2)).join("->")} sat ${s.bands.map((b) => b.sat.toFixed(3)).join("->")} ` +
        `drop ${s.bands.map((b) => b.drop.toFixed(4)).join("->")} (${s.bands.map((b) => b.n).join("/")}) · ` +
        `ambient p05 ${(s.ambientP05 * 100).toFixed(1)}% p50 ${(s.ambientP50 * 100).toFixed(1)}%`,
    );
  }
}
// (a) and (c) — the register and the floor, at the eye.
for (const s of dayEye.samples) {
  const at = `eye yaw ${s.yaw.toFixed(2)}`;
  if (!(s.skyFraction >= DAYLIGHT_MIN_SKY_FRACTION)) {
    fail(
      `tab A: ${at} framed ${(s.skyFraction * 100).toFixed(1)}% sky, under the ` +
        `${(DAYLIGHT_MIN_SKY_FRACTION * 100).toFixed(0)}% this probe needs to compare one against the other`,
    );
  }
  const ratio = s.groundMedian > 0 ? s.skyLuma / s.groundMedian : Infinity;
  if (!(ratio >= DAYLIGHT_MIN_SKY_OVER_GROUND)) {
    fail(
      `tab A: ${at} sky ${s.skyLuma.toFixed(1)} luma against median ground ` +
        `${s.groundMedian} — ratio ${ratio.toFixed(2)}, under ${DAYLIGHT_MIN_SKY_OVER_GROUND}. ` +
        `A ground brighter than its own sky is an inverted register, not a dark one`,
    );
  }
  if (!(s.ambientP05 >= DAYLIGHT_MIN_AMBIENT_FLOOR)) {
    fail(
      `tab A: ${at} the darkest 5% of ground pixels keep ${(s.ambientP05 * 100).toFixed(1)}% of ` +
        `their luma when the key is taken away (median ${(s.ambientP50 * 100).toFixed(1)}%), under ` +
        `${(DAYLIGHT_MIN_AMBIENT_FLOOR * 100).toFixed(0)}% — an unlit face at that share is a black ` +
        `silhouette carrying no albedo, no grain and no material identity`,
    );
  }
}
// (b) — the air, from where there is enough of it to answer.
for (const s of dayAir.samples) {
  const at = `air yaw ${s.yaw.toFixed(2)}`;
  if (!(s.fogFraction >= DAYLIGHT_MIN_FOG_FRACTION)) {
    fail(
      `tab A: ${at} moved ${(s.fogFraction * 100).toFixed(2)}% of its pixels when the fog was pushed ` +
        `past the frame, under ${(DAYLIGHT_MIN_FOG_FRACTION * 100).toFixed(2)}% (mean lift ` +
        `${s.fogMeanLift.toFixed(2)}, max ${s.fogMaxDelta}) — there is no air in this view`,
    );
  }
  if (!(s.fogUpShare >= DAYLIGHT_MIN_FOG_UP_SHARE)) {
    fail(
      `tab A: ${at} the air LIGHTENED only ${(s.fogUpShare * 100).toFixed(1)}% of the pixels it touched, ` +
        `under ${(DAYLIGHT_MIN_FOG_UP_SHARE * 100).toFixed(0)}% — a haze darker than the ground it hazes ` +
        `is distance taking contrast away instead of converging on the sky`,
    );
  }
  const [nearB, midB, farB] = s.bands;
  if (!(nearB.n > 0 && midB.n > 0 && farB.n > 0)) {
    fail(`tab A: ${at} the fog terciles are [${s.bands.map((b) => b.n)}] — one band is empty, so no ramp can be read`);
  }
  if (!(farB.f >= DAYLIGHT_MIN_FAR_FOG)) {
    fail(
      `tab A: ${at} the far third of the ground carries a mean fog factor of ${farB.f.toFixed(3)} ` +
        `(near ${nearB.f.toFixed(3)}, mid ${midB.f.toFixed(3)}), under ${DAYLIGHT_MIN_FAR_FOG} — ` +
        `there is not enough air in the far band for a ramp across it to mean anything`,
    );
  }
  // The mechanism, per yaw, and it is the half the terrain cannot
  // counterfeit. A band's RAW luma is mostly what its ground happens to be
  // made of — measured, this sweep has a yaw whose far third is darker rock
  // and reads x1.015 while the yaw beside it reads x1.25 — but a band's luma
  // LIFT, and its saturation DROP, are the haze and nothing else, because
  // each is that band measured against itself with the air pushed out of the
  // frame. Those two must climb on every step, and they do, by 2-15x.
  for (let k = 1; k < 3; k++) {
    if (!(s.bands[k].lift > s.bands[k - 1].lift)) {
      fail(
        `tab A: ${at} the haze lifts the three bands by ${s.bands.map((b) => b.lift.toFixed(2))} luma — ` +
          `band ${k} is not above band ${k - 1}, so the air is not deepening with distance`,
      );
    }
    if (!(s.bands[k].drop > s.bands[k - 1].drop)) {
      fail(
        `tab A: ${at} the haze cuts the three bands' saturation by ${s.bands.map((b) => b.drop.toFixed(4))} — ` +
          `band ${k} is not above band ${k - 1}, so distance is not washing out with depth`,
      );
    }
  }
}
// …and the report's own criterion, on the frame as rendered, pooled over the
// whole sweep. Pooled and not per yaw for the reason the mechanism block
// above states: one direction's ground is not another's, and the image-level
// ramp is a claim about the world seen from a point, not about a bearing.
// Pixel-weighted, so a yaw with more visible ground counts for more.
const poolBand = (k, key) => {
  let num = 0;
  let den = 0;
  for (const s of dayAir.samples) {
    num += s.bands[k][key] * s.bands[k].n;
    den += s.bands[k].n;
  }
  return den > 0 ? num / den : 0;
};
const nearLuma = poolBand(0, "luma");
const farLuma = poolBand(2, "luma");
const nearSat = poolBand(0, "sat");
const farSat = poolBand(2, "sat");
console.log(
  `    swept: near band ${nearLuma.toFixed(1)} luma / ${nearSat.toFixed(3)} sat → far band ` +
    `${farLuma.toFixed(1)} / ${farSat.toFixed(3)} — x${(farLuma / nearLuma).toFixed(3)} luma, ` +
    `x${(farSat / nearSat).toFixed(3)} sat`,
);
if (!(farLuma >= nearLuma * DAYLIGHT_MIN_BAND_LUMA_STEP)) {
  fail(
    `tab A: swept over 4 yaws the far third of the ground reads ${farLuma.toFixed(1)} luma against ` +
      `the near third's ${nearLuma.toFixed(1)} — x${(farLuma / nearLuma).toFixed(3)}, under ` +
      `x${DAYLIGHT_MIN_BAND_LUMA_STEP}. Distance must lighten`,
  );
}
if (!(farSat <= nearSat * DAYLIGHT_MAX_BAND_SAT_STEP)) {
  fail(
    `tab A: swept over 4 yaws the far third of the ground reads ${farSat.toFixed(3)} saturation against ` +
      `the near third's ${nearSat.toFixed(3)} — x${(farSat / nearSat).toFixed(3)}, over ` +
      `x${DAYLIGHT_MAX_BAND_SAT_STEP}. Distance must wash out`,
  );
}


// Assertion 10 — the shadow map DARKENS PIXELS. A flag says the renderer was
// asked for shadows; only a frame says it got any. The dev-only probe renders
// the live scene twice per yaw — shadow pass on, then off — and counts pixels
// the shadow pass took down. Everything else about this rig (bias, radius,
// caster flags, the light's own position) can be individually wrong in a way
// that leaves every flag above true and the image unchanged.
const probeHook = await A.page.evaluate(() => typeof globalThis.__gatesDebug.shadowProbe);
if (probeHook !== "function") {
  fail(`tab A: __gatesDebug.shadowProbe is ${probeHook} on a dev shard — the shadow gate cannot run`);
}
const probe = await A.page.evaluate(
  ([yaws, pitch, minDelta]) => globalThis.__gatesDebug.shadowProbe(yaws, pitch, minDelta),
  [SHADOW_PROBE_YAWS, SHADOW_PROBE_PITCH, SHADOW_PROBE_MIN_DELTA],
);
const litFraction = probe.darkened / probe.pixels;
if (litFraction < SHADOW_MIN_FRACTION) {
  fail(
    `tab A: the shadow pass darkened ${(litFraction * 100).toFixed(3)}% of ` +
      `${probe.pixels} probed pixels across ${probe.samples.length} yaws — ` +
      `below ${(SHADOW_MIN_FRACTION * 100).toFixed(2)}%. Shadows are enabled but nothing is ` +
      `casting into the frame.\n` +
      probe.samples
        .map(
          (s) =>
            `    yaw ${s.yaw.toFixed(2)}: ${s.darkened} px, mean Δluma ${s.meanDelta.toFixed(1)}, ` +
            `max ${s.maxDelta}, frame luma ${s.litMean.toFixed(1)} unshadowed / ${s.shadowedMean.toFixed(1)} shadowed`,
        )
        .join("\n"),
  );
}
// Per-yaw floor. The aggregate above can be carried by one direction, and it
// was: see SHADOW_MIN_FRACTION_PER_YAW. This catches a direction with no
// shadow at all; attributing what shadow there is to the world is the leg
// below — a job the area floors never had. (The sun did NOT move this pass —
// scene.js, SUN_ELEVATION — so both legs guard the same low-sun frames.)
const thin = probe.samples.filter((s) => s.fraction < SHADOW_MIN_FRACTION_PER_YAW);
if (thin.length) {
  fail(
    `tab A: ${thin.length} of ${probe.samples.length} probed directions have almost no ` +
      `shadow in them (floor ${(SHADOW_MIN_FRACTION_PER_YAW * 100).toFixed(2)}% per yaw). ` +
      `Something near the camera is casting and the world is not.\n` +
      probe.samples
        .map((s) => `    yaw ${s.yaw.toFixed(2)}: ${(s.fraction * 100).toFixed(2)}% (${s.darkened} px)`)
        .join("\n"),
  );
}

// Assertion 10b (lighting v1) — WHOSE shadow is it? The fourth leg: the same
// four frames with `castShadow` off everything that is not another player. The
// difference is what the terrain, the far mesh and the scatter pools drew, and
// it is attribution by construction rather than by aggregate — the mutation
// that calibrated the two floors above by hand on 2026-08-01, taken every run.
if (!(probe.worldCasters >= SHADOW_MIN_WORLD_CASTERS)) {
  fail(
    `tab A: the world-caster mutation found ${probe.worldCasters} casters to suppress, under ` +
      `${SHADOW_MIN_WORLD_CASTERS} — the leg is not removing the world, so the attribution below ` +
      `is measuring nothing. Either nothing in the scene casts, or the remote-exclusion walk ` +
      `swallowed it.`,
  );
}
const worldFraction = probe.worldDarkened / probe.pixels;
const bestWorldYaw = Math.max(...probe.samples.map((s) => s.worldFraction));
if (worldFraction < SHADOW_MIN_WORLD_FRACTION) {
  fail(
    `tab A: only ${(worldFraction * 100).toFixed(3)}% of ${probe.pixels} probed pixels are shadow ` +
      `the WORLD cast (floor ${(SHADOW_MIN_WORLD_FRACTION * 100).toFixed(2)}%), against ` +
      `${(litFraction * 100).toFixed(2)}% shadowed in total — so the shadow in these frames is ` +
      `coming from the other tab's avatar standing on the shared spawn, not from the hills and ` +
      `the pines. The per-yaw area floor above still catches a no-shadow direction; this leg is ` +
      `what ATTRIBUTES the shadow to the world, which the area floors never could.\n` +
      probe.samples
        .map(
          (s) =>
            `    yaw ${s.yaw.toFixed(2)}: ${(s.worldFraction * 100).toFixed(2)}% world of ` +
            `${(s.fraction * 100).toFixed(2)}% total, mean Δluma ${s.worldMeanDelta.toFixed(1)}`,
        )
        .join("\n"),
  );
}
if (bestWorldYaw < SHADOW_MIN_WORLD_BEST_YAW) {
  fail(
    `tab A: the best of ${probe.samples.length} directions attributes ` +
      `${(bestWorldYaw * 100).toFixed(3)}% of its frame to the world's casters (floor ` +
      `${(SHADOW_MIN_WORLD_BEST_YAW * 100).toFixed(2)}%) — the world is casting nowhere, in any ` +
      `direction, and the aggregate above is being carried by noise`,
  );
}
console.log(
  `  whose shadow: ${(worldFraction * 100).toFixed(2)}% of the sweep is the world's ` +
    `(${probe.samples.map((s) => (s.worldFraction * 100).toFixed(1) + "%").join(" ")}) of ` +
    `${(litFraction * 100).toFixed(2)}% total, ${probe.worldCasters} casters suppressed`,
);

// The shadow pass is extra DRAW CALLS, not just an extra texture lookup, and
// the budget below is asserted on a count that includes them. Prove that from
// the same two renders rather than asserting it in a comment — and require
// enough of them that the terrain and the scatter pools must be among them.
const shadowPassCalls = probe.samples.map((s) => s.callsShadowed - s.callsUnshadowed);
if (!shadowPassCalls.every((d) => d >= SHADOW_PASS_MIN_CALLS)) {
  fail(
    `tab A: the shadow pass submitted [${shadowPassCalls}] draw calls, below ` +
      `${SHADOW_PASS_MIN_CALLS} — either it is drawing nothing (so the budget assertion ` +
      `is not counting it either) or the world's meshes are not casters`,
  );
}
console.log(
  `  shadows: ${(litFraction * 100).toFixed(2)}% of ${probe.pixels} probed pixels darkened by the ` +
    `shadow pass (${probe.samples.map((s) => (s.fraction * 100).toFixed(1) + "%").join(" ")}), ` +
    `+${Math.min(...shadowPassCalls)}..${Math.max(...shadowPassCalls)} draw calls`,
);

// Assertion 11b — SHADOWS PAST THE NEAR BOX. The point of the slice, and the
// one thing no flag above can show: lighting v0 passed every assertion in
// this file with a hill at 100 m casting nothing.
//
// The probe renders the same frame with the whole clipmap and with only level
// 0 — which IS lighting v0's reach — and counts what the coarse levels took
// down. Two things make that a claim about DISTANCE. The camera's near plane
// is pushed past the near level so no close geometry is drawn at all; and the
// probe measures the resulting frustum's distance from the near level's
// committed centre along light-space X, so the assertion below is on a number
// read out of the actual camera matrices, not on the two constants that
// produced it.
const farHook = await A.page.evaluate(() => typeof globalThis.__gatesDebug.farShadowProbe);
if (farHook !== "function") {
  fail(`tab A: __gatesDebug.farShadowProbe is ${farHook} on a dev shard — the clipmap gate cannot run`);
}
const farYaws = [lit.sunAzimuth + Math.PI / 2, lit.sunAzimuth - Math.PI / 2];
const far = await A.page.evaluate(
  ([yaws, pitch, minDelta, nearM, fov, heightM]) =>
    globalThis.__gatesDebug.farShadowProbe(yaws, pitch, minDelta, nearM, fov, heightM),
  [
    farYaws,
    FAR_SHADOW_PROBE_PITCH,
    FAR_SHADOW_MIN_DELTA,
    FAR_SHADOW_NEAR_M,
    FAR_SHADOW_FOV_DEG,
    FAR_SHADOW_HEIGHT_M,
  ],
);
// First: is the frame actually outside the near level? If this does not hold
// the darkening below could be ordinary near-level shadow and the whole
// measurement is void, so it is checked before anything is read from it.
if (!(far.minLightXm > far.nearLevelHalfWidthM)) {
  fail(
    `tab A: the far-shadow probe's frustum comes within ${far.minLightXm.toFixed(1)} m of the near ` +
      `level's centre along light-space X, inside its ${far.nearLevelHalfWidthM} m half-width — ` +
      `the frame is not past the near box and nothing it measures is a claim about distance`,
  );
}
// Second: the probe's zero point, measured. Each yaw also renders the
// near-only state TWICE and counts pixels that moved between two identical
// frames. Anything but zero means the counts below are partly the software
// rasterizer talking, and every floor in this block is calibrated against
// noise instead of shadow.
const noisy = far.samples.filter((s) => s.noise > 0);
if (noisy.length) {
  fail(
    `tab A: the far-shadow probe's control renders disagree — ` +
      noisy.map((s) => `yaw ${s.yaw.toFixed(2)}: ${s.noise} px`).join(", ") +
      `. Two renders of the same state moved pixels by > ${FAR_SHADOW_MIN_DELTA}/255, so the ` +
      `darkening measured below cannot be attributed to the clipmap`,
  );
}
const farFraction = far.darkened / far.pixels;
if (farFraction < FAR_SHADOW_MIN_FRACTION) {
  fail(
    `tab A: past the near level's box (every pixel ≥ ${far.minLightXm.toFixed(0)} m out along ` +
      `light-space X) the coarse clipmap levels darkened ${(farFraction * 100).toFixed(3)}% of ` +
      `${far.pixels} pixels — below ${(FAR_SHADOW_MIN_FRACTION * 100).toFixed(2)}%. The levels ` +
      `exist and render, but nothing out there casts into the frame.\n` +
      far.samples
        .map(
          (s) =>
            `    yaw ${s.yaw.toFixed(2)}: ${s.darkened} px, mean Δluma ${s.meanDelta.toFixed(1)}, ` +
            `max ${s.maxDelta}, frustum reach ${s.reachM.toFixed(1)} m`,
        )
        .join("\n"),
  );
}
const farThin = far.samples.filter((s) => s.fraction < FAR_SHADOW_MIN_FRACTION_PER_YAW);
if (farThin.length) {
  fail(
    `tab A: ${farThin.length} of ${far.samples.length} far directions have almost no shadow ` +
      `(floor ${(FAR_SHADOW_MIN_FRACTION_PER_YAW * 100).toFixed(2)}% per yaw). Both are perpendicular ` +
      `to the sun, so one carrying the aggregate means the coverage is one-sided.\n` +
      far.samples
        .map((s) => `    yaw ${s.yaw.toFixed(2)}: ${(s.fraction * 100).toFixed(2)}% (${s.darkened} px)`)
        .join("\n"),
  );
}
const liftedWorst = Math.max(...far.samples.map((s) => s.liftedFraction));
if (liftedWorst > FAR_SHADOW_MAX_LIFTED_FRACTION) {
  fail(
    `tab A: turning the coarse clipmap levels on BRIGHTENED ${(liftedWorst * 100).toFixed(3)}% of a ` +
      `frame — they are contributing light, not shadow. A level past the first must have zero ` +
      `intensity or the key is counted once per level`,
  );
}
console.log(
  `  far shadows: ${(farFraction * 100).toFixed(2)}% of ${far.pixels} pixels darkened by levels ` +
    `past the near box (${far.samples.map((s) => (s.fraction * 100).toFixed(1) + "%").join(" ")}), ` +
    `frame ≥ ${far.minLightXm.toFixed(0)} m out vs a ${far.nearLevelHalfWidthM} m near level, ` +
    `lifted ${(liftedWorst * 100).toFixed(4)}%`,
);

// Assertion 11c — the GROUND casts, and the two LODs of it are kept apart.
// Structural, cheap, and here so 11d's failure is diagnosable: "the horizon
// darkens nothing" means something very different when the ground is still
// being culled out of every depth pass it is submitted to.
const fc = await A.page.evaluate(() => globalThis.__gatesDebug.farCaster);
if (!fc) fail(`tab A: the client publishes no far-caster state — the horizon is not wired`);
if (fc.shadowSide !== THREE_FRONT_SIDE) {
  fail(
    `tab A: the ground's material has shadowSide ${fc.shadowSide}, not FrontSide ` +
      `(${THREE_FRONT_SIDE}). three derives the depth pass from this: for a FrontSide ` +
      `material it flips to BackSide, and a heightfield has no back face turned at the ` +
      `sky — every terrain triangle is culled and hills cast nothing, near or far`,
  );
}
if (!fc.built) fail(`tab A: the far mesh has not been built — nothing can be asserted about the horizon`);
if (fc.casts !== true) fail(`tab A: the far mesh does not cast — the horizon is still lit flat`);
if (fc.customDepth !== true) {
  fail(
    `tab A: the far mesh casts with no custom depth material — nothing punches the near ` +
      `ring out of it, so both LODs of one hillside are in the same map`,
  );
}
if (!(fc.sinkM > 0)) {
  fail(`tab A: the far caster's seam skirt is ${fc.sinkM} m — the two LODs meet with no offset at all`);
}
if (!(fc.triangles > 0)) fail(`tab A: the far caster has ${fc.triangles} triangles`);
// The hole must actually track the ring. Two independent ways of being wrong:
// a hole frozen at the origin (the uniform never written), and a hole that has
// stopped containing the chunks that cast into it.
if (!(fc.nearChunks > 0)) fail(`tab A: no near chunks are loaded — the hole has nothing to track`);
const holeChunksX = (fc.holeHalf[0] * 2) / fc.chunkM;
const holeChunksZ = (fc.holeHalf[1] * 2) / fc.chunkM;
if (!(holeChunksX >= 1 && holeChunksZ >= 1)) {
  fail(
    `tab A: the far caster's hole is ${fc.holeHalf} m against ${fc.nearChunks} loaded near ` +
      `chunks — the near ring's footprint is never written, so the far mesh casts straight ` +
      `through it and every hillside in the ring has two silhouettes in one map`,
  );
}
if (holeChunksX * holeChunksZ < fc.nearChunks) {
  fail(
    `tab A: the hole is ${holeChunksX}x${holeChunksZ} chunks but ${fc.nearChunks} are loaded ` +
      `and casting — a chunk outside the hole is a double caster against the far mesh`,
  );
}
const ownA = await A.page.evaluate(() => globalThis.__gatesDebug.own);
if (
  Math.abs(ownA[0] - fc.holeCenter[0]) > fc.holeHalf[0] ||
  Math.abs(ownA[2] - fc.holeCenter[1]) > fc.holeHalf[1]
) {
  fail(
    `tab A: the player is at [${ownA[0].toFixed(0)}, ${ownA[2].toFixed(0)}] but the hole is ` +
      `centred on [${fc.holeCenter.map((v) => v.toFixed(0))}] ±[${fc.holeHalf}] — the hole is ` +
      `not following the ring`,
  );
}
// And the per-level split that pays for the third level: the two LODs are not
// both submitted to every map.
if (!(fc.farMinLevel >= 1)) {
  fail(`tab A: the far mesh casts from level ${fc.farMinLevel} — level 0's box is inside the hole, so that is a whole map's fill discarded`);
}
if (!(fc.nearMaxLevel < cm.levelCount - 1)) {
  fail(
    `tab A: the near ring casts up to level ${fc.nearMaxLevel} of ${cm.levelCount} — the ` +
      `coarsest level is read only by pixels the ring cannot reach, so that is ${fc.nearChunks} ` +
      `draw calls of caster nothing samples`,
  );
}
console.log(
  `  far caster: ${fc.triangles} tris, shadowSide ${fc.shadowSide} (FrontSide), skirt ${fc.sinkM} m, ` +
    `hole ${holeChunksX}x${holeChunksZ} chunks over ${fc.nearChunks} loaded, ` +
    `near ring in levels 0..${fc.nearMaxLevel}, far mesh in ${fc.farMinLevel}..`,
);

// Assertion 11d — the HORIZON casts, measured by removing it.
//
// 11b's probe proves the coarse levels reach past the near LEVEL's box. It
// cannot prove the horizon casts, and never could: until this slice the near
// ring WAS the caster set, and a frame 97 m out is deep inside it.
//
// This one holds the frame fixed and removes the caster. The far mesh casts
// through a depth material that discards the near ring's footprint; opening
// that hole to swallow the world discards all of it, which is exactly the
// state that shipped before this slice. The difference between the two frames
// is the horizon — and it is a claim about DISTANCE for free, because the hole
// means every pixel it darkens was cast by geometry more than the ring's own
// footprint away. No camera geometry has to be argued.
const horizonHook = await A.page.evaluate(() => typeof globalThis.__gatesDebug.horizonProbe);
if (horizonHook !== "function") {
  fail(`tab A: __gatesDebug.horizonProbe is ${horizonHook} on a dev shard — the horizon gate cannot run`);
}
const horizon = await A.page.evaluate(
  ([yaws, pitch, minDelta, heightM]) =>
    globalThis.__gatesDebug.horizonProbe(yaws, pitch, minDelta, heightM),
  [HORIZON_PROBE_YAWS, HORIZON_PROBE_PITCH, FAR_SHADOW_MIN_DELTA, HORIZON_PROBE_HEIGHT_M],
);
if (!horizon) fail(`tab A: the horizon probe found no far caster to toggle`);
// The probe must have scored the LIVE ring footprint, not a leftover: a hole
// already swallowing the world would make both its frames identical and the
// measurement below vacuously zero in a way the floor would blame on casting.
if (!(horizon.holeHalf[0] > 0) || horizon.holeHalf[0] > ISLAND_M) {
  fail(`tab A: the horizon probe ran with a hole half-extent of ${horizon.holeHalf} m — not the live ring footprint`);
}
const horizonNoisy = horizon.samples.filter((s) => s.noise > 0);
if (horizonNoisy.length) {
  fail(
    `tab A: the horizon probe's control renders disagree — ` +
      horizonNoisy.map((s) => `yaw ${s.yaw.toFixed(2)}: ${s.noise} px`).join(", ") +
      `. Two renders of the same state moved pixels by > ${FAR_SHADOW_MIN_DELTA}/255, so what ` +
      `the toggle below measures cannot be attributed to the far caster`,
  );
}
const horizonFraction = horizon.darkened / horizon.pixels;
if (horizonFraction < HORIZON_MIN_FRACTION) {
  fail(
    `tab A: suppressing the far mesh's shadow changed ${(horizonFraction * 100).toFixed(3)}% of ` +
      `${horizon.pixels} swept pixels — below ${(HORIZON_MIN_FRACTION * 100).toFixed(2)}%. The ` +
      `horizon receives and does not cast, which is exactly the state this slice exists to end.\n` +
      horizon.samples
        .map(
          (s) =>
            `    yaw ${s.yaw.toFixed(2)}: ${s.darkened} px, mean Δluma ${s.meanDelta.toFixed(1)}, max ${s.maxDelta}`,
        )
        .join("\n"),
  );
}
const horizonLive = horizon.samples.filter((s) => s.fraction >= HORIZON_MIN_FRACTION_PER_YAW);
if (horizonLive.length < HORIZON_MIN_DIRECTIONS) {
  fail(
    `tab A: only ${horizonLive.length} of ${horizon.samples.length} directions lose anything when ` +
      `the far caster is suppressed (floor ${(HORIZON_MIN_FRACTION_PER_YAW * 100).toFixed(2)}% per ` +
      `yaw, ${HORIZON_MIN_DIRECTIONS} required) — one direction carrying the whole aggregate is a ` +
      `fluke of where the camera happened to be pointed, not a horizon that casts\n` +
      horizon.samples
        .map((s) => `    yaw ${s.yaw.toFixed(2)}: ${(s.fraction * 100).toFixed(2)}% (${s.darkened} px)`)
        .join("\n"),
  );
}
const horizonLifted = Math.max(...horizon.samples.map((s) => s.liftedFraction));
if (horizonLifted > FAR_SHADOW_MAX_LIFTED_FRACTION) {
  fail(
    `tab A: removing the far caster DARKENED ${(horizonLifted * 100).toFixed(3)}% of a frame — a ` +
      `caster cannot add light, so the hole is inverted and the far mesh is casting inside the ` +
      `near ring instead of outside it`,
  );
}
console.log(
  `  horizon: suppressing the far caster moved ${(horizonFraction * 100).toFixed(2)}% of ` +
    `${horizon.pixels} swept pixels ` +
    `(${horizon.samples.map((s) => (s.fraction * 100).toFixed(1) + "%").join(" ")}), ` +
    `${horizonLive.length}/${horizon.samples.length} directions, mean Δluma ` +
    `${(horizonLive.reduce((a, s) => a + s.meanDelta, 0) / Math.max(1, horizonLive.length)).toFixed(1)}, ` +
    `hole ±${horizon.holeHalf[0]} m, lifted ${(horizonLifted * 100).toFixed(4)}%`,
);

// --- lighting v1: the tonal register, against the reference bar --------------
// Assertion 17. The first absolute in this file: not "the shadow map darkened
// something", not "the field moved pixels" — WHERE the image sits, measured in
// the same statistic `ci/reference_bar.mjs` read off `Rust Images/` before any
// tab existed.
const tonalHook = await A.page.evaluate(() => typeof globalThis.__gatesDebug.tonalProbe);
if (tonalHook !== "function") {
  fail(`tab A: __gatesDebug.tonalProbe is ${tonalHook} on a dev shard — the tonal gate cannot run`);
}
const tonal = await A.page.evaluate(
  (views) => globalThis.__gatesDebug.tonalProbe(views),
  TONAL_VIEWS,
);
if (!tonal || tonal.samples.length !== TONAL_VIEWS.length) {
  fail(
    `tab A: tonalProbe returned ${tonal ? tonal.samples.length : "null"} of ` +
      `${TONAL_VIEWS.length} views — the register was not measured`,
  );
}
console.log(
  `  register: ${tonal.samples.length} vantages, ${tonal.pixels} px · ` +
    `p10 ${tonal.all.p10} · p50 ${tonal.all.p50} · p90 ${tonal.all.p90} ` +
    `(bar ${referenceBar.p10} · ${referenceBar.p50} · ${referenceBar.p90})`,
);
for (const s of tonal.samples) {
  console.log(
    `    ${s.label.padEnd(17)} p10 ${String(s.p10).padStart(3)} · p50 ${String(s.p50).padStart(3)} · ` +
      `p90 ${String(s.p90).padStart(3)} · mean ${s.mean.toFixed(1).padStart(5)} · ` +
      `sky ${(s.skyFraction * 100).toFixed(1)}% at ${s.skyMean.toFixed(1)} in ${s.skyLevels} levels, ` +
      `break ${(s.skyBreak * 100).toFixed(0)}% run ${s.skyLongestRun}`,
  );
}
// A program that fails to LINK is the one renderer failure this whole file
// could not see. three reports it to the console and carries on drawing
// nothing, so the object silently vanishes — and if its absence happens to
// look like something (a sky dome missing behind a clear colour the same
// hue), every measurement above stays plausible. That is exactly what
// happened while lighting v1 was being built: the dome's ShaderMaterial
// redefined two chunks three's own prefix already carries, never linked, and
// the tonal probe cheerfully reported a flat overcast sky for two runs.
//
// So: tab A's console must be empty by here. It has joined, walked, chatted,
// streamed terrain and run five probes' worth of programs by this point, so
// this is not a boot check — it covers every program the client can wear.
if (A.errors.length) {
  fail(
    `tab A: ${A.errors.length} console error(s) by the tonal gate — a shader that fails to link ` +
      `reports here and NOWHERE else, and the object it belonged to just stops drawing:\n` +
      A.errors.slice(0, 4).map((e) => `    ${e.slice(0, 1200)}`).join("\n"),
  );
}

// The floors are BELOW the bar, and that is asserted rather than asserted-once
// and trusted. A floor above the reference would mean a passing frame is
// brighter than the thing it is being compared to, which is a different bug
// wearing this gate's clothes; a floor that drifts above it — because someone
// raised the number to make a pass green — goes red here first.
if (!(referenceBar.p90 >= TONAL_MIN_P90)) {
  fail(
    `the reference set measures p90 ${referenceBar.p90} and this gate's floor is ` +
      `${TONAL_MIN_P90} — the floor is ABOVE the bar it was derived from, so passing it ` +
      `no longer means "as bright as Rust Images/". Lower the floor or say why.`,
  );
}
if (!(referenceBar.p10 <= TONAL_MAX_P10)) {
  fail(
    `the reference set measures p10 ${referenceBar.p10} and this gate's ceiling is ` +
      `${TONAL_MAX_P10} — the ceiling is BELOW the bar, so a frame with reference-correct ` +
      `shadows would fail it`,
  );
}
if (!(tonal.all.p90 >= TONAL_MIN_P90)) {
  fail(
    `tab A: the capture's p90 luma is ${tonal.all.p90} against a floor of ${TONAL_MIN_P90} ` +
      `and a reference bar of ${referenceBar.p90} — the top of the image is missing. ` +
      `Per view: ${tonal.samples.map((s) => `${s.label} ${s.p90}`).join(", ")}`,
  );
}
if (!(tonal.all.p50 >= TONAL_MIN_P50)) {
  fail(
    `tab A: the capture's median luma is ${tonal.all.p50} against a floor of ${TONAL_MIN_P50} ` +
      `and a reference bar of ${referenceBar.p50} — the midtones sit under the bar, so p90 ` +
      `is being carried by highlights rather than by exposure`,
  );
}
if (!(tonal.all.p10 <= TONAL_MAX_P10)) {
  fail(
    `tab A: the capture's p10 luma is ${tonal.all.p10} against a ceiling of ${TONAL_MAX_P10} ` +
      `(reference ${referenceBar.p10}) — the darks came up with everything else, which is a ` +
      `lift and not a light rig`,
  );
}
const tonalRange = tonal.all.p90 - tonal.all.p10;
if (!(tonalRange >= TONAL_MIN_RANGE)) {
  fail(
    `tab A: the capture spans ${tonalRange} luma p10→p90 against a floor of ` +
      `${TONAL_MIN_RANGE} and a reference bar of ${referenceBar.range} — the image is flat`,
  );
}

// The vantages with enough dome in them to score one, used by 17a's
// sky-over-ground check and by 17b below.
const skyViews = tonal.samples.filter((s) => s.skyFraction >= SKY_MIN_FRACTION);
if (skyViews.length < 2) {
  fail(
    `tab A: only ${skyViews.length} of ${tonal.samples.length} vantages see ` +
      `${(SKY_MIN_FRACTION * 100).toFixed(0)}% sky — the dome cannot be scored, which means ` +
      `the probe photographed the wrong thing rather than that the sky is fine`,
  );
}

// 17a — the seam, by construction rather than by photograph. Fog and the sky
// dome's horizon band are the SAME constant; a fully-fogged surface and the
// sky above it therefore arrive at the tone mapper carrying identical linear
// values, and the horizon cannot step. The judge measured that step at 31
// levels. Two numbers that happen to agree would pass a pixel test today and
// drift the day one of them moves, so this asserts the identity instead.
const lit2 = await A.page.evaluate(() => globalThis.__gatesDebug.lighting);
const seamOff = Math.max(...lit2.fogColor.map((v, i) => Math.abs(v - lit2.skyHorizon[i])));
if (!(seamOff === 0)) {
  fail(
    `tab A: fog is linear [${lit2.fogColor.map((v) => v.toFixed(4))}] and the sky's horizon band ` +
      `is [${lit2.skyHorizon.map((v) => v.toFixed(4))}], off by ${seamOff.toExponential(2)} — the ` +
      `seam is two numbers that have to be kept equal by hand, which is how a 31-level step at ` +
      `the horizon happens`,
  );
}
// …and the sky has to be the brightest thing in the frame, or the image has
// no top for p90 to sit on. This was the first cut's actual defect: an
// un-gained dome measured sky 142 against a ground median of 138, an image
// whose whole tonal range was one value wide. Measured on the dome mask, not
// argued from the constants — what matters is what it delivers.
const skyOverGround = Math.min(...skyViews.map((s) => s.skyMean - s.worldP50));
if (!(skyOverGround >= SKY_MIN_OVER_GROUND)) {
  fail(
    `tab A: the sky is only ${skyOverGround.toFixed(1)} luma above the frame's own median on its ` +
      `weakest vantage (floor ${SKY_MIN_OVER_GROUND}) — the dome is no brighter than the ground ` +
      `it lights, so nothing in the frame is a highlight\n` +
      tonal.samples
        .map(
          (s) =>
            `    ${s.label}: sky ${s.skyMean.toFixed(1)} over ${s.worldPixels} px of world at ` +
            `median ${s.worldP50}`,
        )
        .join("\n"),
  );
}
if (lit2.skySunShared !== true) {
  fail(
    `tab A: the sky's sun direction is not the same object as the key light's — the dome ` +
      `can draw a sun the shadows disagree with`,
  );
}
if (!(lit2.fogNear > 0) || !(lit2.fogFar > lit2.fogNear)) {
  fail(`tab A: fog is ${lit2.fogNear}→${lit2.fogFar} m, which is not a range`);
}

// 17b — the sky is a ramp, and a ramp quantized to 8 bits bands. Counted on
// the dome's own mask (a sentinel-clear render, so it is the dome and not
// "the top of the frame"), on every view that has enough sky to measure.
for (const s of skyViews) {
  if (!(s.skyBreak >= SKY_MIN_BREAK)) {
    fail(
      `tab A: ${s.label}'s sky changes value across only ${(s.skyBreak * 100).toFixed(1)}% of its ` +
        `adjacent pixel pairs (floor ${(SKY_MIN_BREAK * 100).toFixed(0)}%), longest identical run ` +
        `${s.skyLongestRun} px over ${s.skyPixels} px of dome — the gradient is posterized, so ` +
        `there is no dither under the quantizer`,
    );
  }
  if (!(s.skyLongestRun <= SKY_MAX_RUN)) {
    fail(
      `tab A: ${s.label}'s sky holds a ${s.skyLongestRun} px run of one identical value (ceiling ` +
        `${SKY_MAX_RUN}) — that is a visible band, and it is the defect the judge measured at 11 px`,
    );
  }
}

// 17c — the sun. Aimed down the key light's own direction vector, so the disc
// belongs at the principal point: this is the assertion that the thing in the
// sky and the thing casting the shadows are one sun.
const sunHook = await A.page.evaluate(() => typeof globalThis.__gatesDebug.sunProbe);
if (sunHook !== "function") {
  fail(`tab A: __gatesDebug.sunProbe is ${sunHook} on a dev shard — the sun gate cannot run`);
}
const sun = await A.page.evaluate(() => globalThis.__gatesDebug.sunProbe());
const sunOverSky = sun.peak - sun.background;
console.log(
  `  sun: peak ${sun.peak} at ${sun.offsetPx.toFixed(1)} px off the aim point, sky background ` +
    `${sun.background.toFixed(1)} (+${sunOverSky.toFixed(1)}), ` +
    `${(sun.saturatedFraction * 100).toFixed(3)}% of the frame at 250+`,
);
if (!(sunOverSky >= SUN_MIN_PEAK_OVER_SKY)) {
  fail(
    `tab A: looking straight at the sun, the brightest pixel is ${sun.peak} against a sky ` +
      `background of ${sun.background.toFixed(1)} — ${sunOverSky.toFixed(1)} levels, under the ` +
      `${SUN_MIN_PEAK_OVER_SKY} floor. There is no sun in the sky, only a gradient`,
  );
}
if (!(sun.offsetPx <= SUN_MAX_OFFSET_PX)) {
  fail(
    `tab A: the brightest sky pixel is ${sun.offsetPx.toFixed(1)} px from the direction the KEY ` +
      `LIGHT points, against a ${SUN_MAX_OFFSET_PX} px tolerance — the dome's sun and the ` +
      `world's sun are in different places`,
  );
}
if (!(sun.saturatedFraction <= SUN_MAX_SATURATED)) {
  fail(
    `tab A: ${(sun.saturatedFraction * 100).toFixed(2)}% of the frame is at 250+ luma looking at ` +
      `the sun, over the ${(SUN_MAX_SATURATED * 100).toFixed(0)}% ceiling — the disc has grown ` +
      `into a wash and the sky around it is blown`,
  );
}

// --- materials: the surface read --------------------------------------------
// Measured here, on the same pinned frame the shadow gate uses, for the same
// reason: after the walk the ground under the camera depends on how much wall
// clock a shared box gave the input pump.
//
// Assertion 13 — the material system is wired, and its channels are real
// channels. Cheap and structural, and it exists so 14's and 15's failures are
// diagnosable: "the field paints nothing" means something very different when
// the ground is still a Lambert material.
const mat = await A.page.evaluate(() => globalThis.__gatesDebug.materials);
if (!mat) fail(`tab A: __gatesDebug.materials missing — the scene publishes no material state`);
if (mat.terrain.type !== "MeshStandardMaterial") {
  fail(`tab A: the ground is a ${mat.terrain.type} — roughness is not a channel it has`);
}
if (mat.terrain.patched !== true) {
  fail(`tab A: the terrain material carries no splat uniforms — onBeforeCompile did not install`);
}
if (mat.identities.length !== 4) {
  fail(`tab A: ${mat.identities.length} terrain identities (${mat.identities}) — TERRAIN.md §4 specifies four sets`);
}
// Roughness and bump per identity, not one number applied to everything: the
// procedural-materials failure list's "roughness is a scalar afterthought".
const distinct = (xs) => new Set(xs.map((v) => v.toFixed(4))).size;
if (distinct(mat.identityRoughness) < 3) {
  fail(`tab A: only ${distinct(mat.identityRoughness)} distinct identity roughness values in [${mat.identityRoughness}] — sand, grass, litter and rock answer light the same way`);
}
if (distinct(mat.identityBump) < 3) {
  fail(`tab A: only ${distinct(mat.identityBump)} distinct identity bump amplitudes in [${mat.identityBump}]`);
}
if (!(mat.breakup > 0)) fail(`tab A: splat break-up amplitude is ${mat.breakup} — biome boundaries stay smooth ramps`);
if (!(mat.specAA > 0)) fail(`tab A: specular-AA gain is ${mat.specAA} — the perturbed normal is unfiltered`);
if (!(mat.fadeMicroM[1] > mat.fadeMicroM[0] && mat.fadeMicroM[0] > 0)) {
  fail(`tab A: micro-octave footprint fade is [${mat.fadeMicroM}] — detail below a pixel is not being faded out`);
}
// Assertion 15a2 — EVERY octave retires while it is still resolvable.
//
// The check above only asks that the micro fade rises, which the shipped
// values did while retiring the octave at 0.65 cycles per pixel — past the
// 0.5 Nyquist limit, so the last stretch of its life was spent aliasing. It
// reached the frame as a two-pixel-period speckle in the grass (measured on
// pass 20260802-050932-01: autocorrelation 0.53 at one pixel, 0.05 at two,
// one hue at two luminances, 10.2 luma/px of neighbour contrast against
// 0.18 on the sand of the same frame), because a height field sampled past
// Nyquist has a per-pixel-random gradient and the bump divides gradients by
// the pixel footprint. Meso was over the line too, at 0.74.
//
// So the law is asserted over the table rather than per octave: a fade that
// is not expressed in cycles per pixel cannot be checked against the
// sampling rate, and every hand-derived metre threshold in this material
// was wrong in the same direction.
if (!Array.isArray(mat.octaves) || mat.octaves.length < 3) {
  fail(`tab A: the material reports ${mat.octaves ? mat.octaves.length : "no"} octaves — the sampling law cannot be checked`);
}
if (!(mat.nyquistCpp > 0 && mat.nyquistCpp <= 0.5)) {
  fail(`tab A: the material's Nyquist limit is ${mat.nyquistCpp} cycles/pixel — above 0.5 it is not Nyquist`);
}
const faded = mat.octaves.filter((o) => o.fadeCpp);
const unfaded = mat.octaves.filter((o) => !o.fadeCpp);
if (!faded.length) fail(`tab A: no octave has a footprint fade at all`);
for (const o of faded) {
  if (!(o.fadeCpp[1] > o.fadeCpp[0] && o.fadeCpp[0] > 0)) {
    fail(`tab A: the ${o.name} octave's fade is [${o.fadeCpp}] cycles/pixel — a fade must rise from above zero`);
  }
  if (!(o.fadeCpp[1] <= mat.nyquistCpp)) {
    fail(
      `tab A: the ${o.name} octave retires at ${o.fadeCpp[1]} cycles/pixel, past the ${mat.nyquistCpp} ` +
        `Nyquist limit — it is still being sampled after it stopped being representable, which reaches ` +
        `the frame as speckle and not as detail`,
    );
  }
  // …and the metres the shader actually compares against must be that same
  // pair, or the cpp above is a decoration over a hand-written distance.
  const want = o.fadeCpp.map((c) => c / o.scale);
  if (o.fadeM.some((m, k) => Math.abs(m - want[k]) > 1e-9)) {
    fail(
      `tab A: the ${o.name} octave fades at [${o.fadeM}] m/px but its ${o.fadeCpp} cycles/pixel over a ` +
        `scale of ${o.scale} /m is [${want}] — the shader is not comparing against the law`,
    );
  }
}
// An octave may go unfaded only if it is too coarse to alias before every
// faded one is already gone: by the time the frame is coarse enough for it,
// there is nothing left for it to alias against.
const coarsestRetire = Math.max(...faded.map((o) => o.fadeCpp[1] / o.scale));
for (const o of unfaded) {
  const aliasAt = mat.nyquistCpp / o.scale;
  if (!(aliasAt > coarsestRetire)) {
    fail(
      `tab A: the unfaded ${o.name} octave aliases at a ${aliasAt.toFixed(2)} m/px footprint, at or before ` +
        `the ${coarsestRetire.toFixed(2)} m/px where the faded octaves retire — it needs a fade of its own`,
    );
  }
}
if (!(mat.bumpMaxSlope > 0 && mat.bumpMaxSlope <= 2)) {
  fail(
    `tab A: the bump's surface gradient is capped at slope ${mat.bumpMaxSlope} — a screen derivative ` +
      `over a screen footprint is unbounded without one (wall 4)`,
  );
}
console.log(
  `  octaves: ${mat.octaves.map((o) => `${o.name} ${(1 / o.scale).toFixed(1)}m` +
    (o.fadeCpp ? ` retires ${o.fadeCpp[1]}cpp/${(o.fadeCpp[1] / o.scale).toFixed(2)}m·px` : " unfaded")).join(" · ")} ` +
    `· Nyquist ${mat.nyquistCpp} · bump cap ${mat.bumpMaxSlope}`,
);

// The tier read (bases.webp): wood, stone and metal must not answer the key
// light identically, and metal must actually be a conductor.
const tierRough = mat.tiers.map((t) => t[1]);
if (mat.tiers.some((t) => t[0] !== "MeshStandardMaterial")) {
  fail(`tab A: build tiers are [${mat.tiers.map((t) => t[0])}] — a tier with no roughness cannot read as its material`);
}
if (distinct(tierRough) !== 3) fail(`tab A: build tiers share roughness [${tierRough}] — wood, stone and metal read alike`);
if (!(mat.tiers[2][2] > 0.5)) fail(`tab A: the metal tier has metalness ${mat.tiers[2][2]} — it is a grey dielectric`);
if (mat.tiers[0][2] !== 0 || mat.tiers[1][2] !== 0) {
  fail(`tab A: wood/stone tiers have metalness [${mat.tiers[0][2]}, ${mat.tiers[1][2]}] — only metal is a conductor`);
}
if (!(mat.water[0] < 0.3)) fail(`tab A: water roughness is ${mat.water[0]} — the sun leaves no track on it`);
// Scatter: every pool authored, colour-baked and per-instance tinted. One
// solid green cone pool (what this slice replaced) shows up as tint 0 or a
// missing instance colour buffer.
if (!Array.isArray(mat.scatter) || mat.scatter.length < 7) {
  fail(`tab A: ${mat.scatter?.length} scatter pools published — sim-core scatters seven archetypes`);
}
for (const p of mat.scatter) {
  if (p.type !== "MeshStandardMaterial") fail(`tab A: scatter archetype ${p.arch} is a ${p.type}`);
  if (!p.vertexColors) fail(`tab A: scatter archetype ${p.arch} has no baked vertex colours — it is one flat colour`);
  if (!p.instanceColor) fail(`tab A: scatter archetype ${p.arch} has no per-instance colour buffer — every instance is identical`);
  if (!(p.tint > 0)) fail(`tab A: scatter archetype ${p.arch} has tint amplitude ${p.tint} — a forest of one green`);
}
if (!mat.scatter.some((p) => p.metalness > 0)) {
  fail(`tab A: no scatter archetype is metallic — the ore nodes read as painted rock`);
}
if (!mat.scatter.some((p) => p.count > 0)) {
  fail(`tab A: every scatter pool is empty — nothing was streamed, so nothing above was measured on real instances`);
}
// …and prop albedo v1: every authored band sits inside the dielectric
// luminance range, in the linear space the fragment multiplies in.
//
// This is the structural half of 15g. It is here rather than beside the pixel
// half because it needs no camera and no instance in frame: it scores all
// SEVEN archetypes, where the rendered probe below can only photograph the two
// the gate is able to reliably find near the pinned spawn. A class whose
// albedo is re-darkened out of band goes red on the whole table.
//
// The floor is what makes it a wall. Below `ALBEDO_LUMA_BAND[0]` the surface
// is delivered into a range where the prop field cannot be carried by an 8-bit
// framebuffer under any light this scene has: measured on the merged frames,
// the pine skirt's authored 0.0453 reached the canopy underside at RGB (2,6,1)
// and the field's ±14% on it is ±0.8 of a level. Both ends of every ramp are
// scored, because the dark end is where a surface stops being one.
const band = mat.props.albedoBand;
if (!Array.isArray(band) || !(band[0] > 0) || !(band[1] > band[0])) {
  fail(`tab A: the client published albedo band ${JSON.stringify(band)} — 15g's structural half has no law to assert`);
}
let albedoParts = 0;
for (const p of mat.scatter) {
  if (!Array.isArray(p.albedo) || p.albedo.length === 0) {
    fail(`tab A: scatter archetype ${p.arch} published no albedo bands — the albedo law cannot be scored on it`);
  }
  for (const a of p.albedo) {
    for (const [end, y] of [["lo", a.lo], ["hi", a.hi]]) {
      albedoParts++;
      if (!(y >= band[0] && y <= band[1])) {
        fail(
          `tab A: scatter archetype ${p.arch} (${p.surface}) part "${a.part}" has a ${end} albedo of ` +
            `${y.toFixed(4)} linear luminance, outside the dielectric band [${band[0]}, ${band[1]}] — ` +
            `${y < band[0]
              ? `darker than charcoal, and the prop field on it is multiplicative, so whatever surface ` +
                `this class was given is worth a fraction of a level wherever it is not in direct sun`
              : `brighter than any natural outdoor material, so it will clip before the tone map sees it`}`,
        );
      }
    }
  }
}
console.log(
  `  prop albedo: ${albedoParts} authored band ends over ${mat.scatter.length} archetypes, all inside ` +
    `[${band[0]}, ${band[1]}] linear luminance · ` +
    mat.scatter
      .map((p) => `${p.surface}#${p.arch} ${p.albedo.map((a) => `${a.part} ${a.lo.toFixed(3)}→${a.hi.toFixed(3)}`).join(", ")}`)
      .join(" · "),
);

// Assertion 13b — wind is animated by the SIM, and it moves the shadow too.
//
// Two claims, neither of which any pixel assertion in this file can reach.
//
// The first is the shadow. A vertex displacement lives in the surface
// material; the shadow pass renders through a different program entirely, and
// three hands every plain caster one shared `MeshDepthMaterial`. Patch only
// the surface and the tree sways out from under a shadow bolted to the ground
// — from the camera's own side that reads as *correct*, because the shadow is
// behind the thing casting it. So this asserts the STRUCTURE: every archetype
// that sways owns the wind-bearing depth material.
//
// The second is the clock, and it is the one worth the most. Wind is the first
// animated thing in this client, so it is the first thing that could make two
// captures of one seed differ — `NOW.md` item 12's whole premise. The client
// takes its wind time from the sim tick, and that is checkable arithmetic
// rather than a sentence: `t` must be the tick in seconds. A wall clock, a
// frame counter or an accumulated dt all fail it within a second of play.
const wind = await A.page.evaluate(() => globalThis.__gatesDebug.wind);
if (!wind) fail(`tab A: __gatesDebug.wind missing — the client publishes no wind state`);
if (!(wind.strength > 0)) fail(`tab A: wind strength is ${wind.strength} — nothing moves`);
const windLen = Math.hypot(wind.dir[0], wind.dir[1]);
if (Math.abs(windLen - 1) > 1e-3) {
  fail(`tab A: wind bearing [${wind.dir}] has length ${windLen.toFixed(4)} — the strength knob is not in metres`);
}
if (!Array.isArray(wind.swaying) || !wind.swaying.includes(1)) {
  fail(`tab A: swaying archetypes are [${wind.swaying}] — archetype 1 is the tree and it is not among them`);
}
if (wind.depthPatched !== true) {
  fail(
    `tab A: a swaying archetype casts through three's shared depth material — its shadow will ` +
      `stand still while it leans, and no capture from the camera's side can see it`,
  );
}
if (!(wind.tick > 0)) fail(`tab A: wind clock is at tick ${wind.tick} — the sim never advanced`);
// The whole determinism claim, as arithmetic. `t` is sim seconds x WIND_SPEED;
// the tolerance is one tick, not a fudge — the snapshot is read between frames.
if (Math.abs(wind.t - (wind.tick / 30) * wind.speed) > (1 / 30) * wind.speed + 1e-6) {
  fail(
    `tab A: wind clock t=${wind.t} against tick ${wind.tick} (${wind.tick / 30}s x speed ${wind.speed}) — ` +
      `the vertex shader is being animated by something other than the sim, so no two captures of ` +
      `one seed can agree`,
  );
}
console.log(
  `  wind: bearing [${wind.dir.map((d) => d.toFixed(3))}] · ${wind.strength} m at the tip · ` +
    `curve ${wind.curve} · archetypes [${wind.swaying}] all casting through the wind depth material · ` +
    `clock t=${wind.t.toFixed(2)}s from tick ${wind.tick} · ${wind.stumps} stump(s), ${wind.felling} falling`,
);

// Assertion 14 — the splat weights are a field, not a constant. The shader
// below can blend four identities perfectly and still paint one, if what it
// is fed is uniform or one-hot; no pixel probe can tell those apart from a
// well-blended world, so count the attribute itself.
const censusHook = await A.page.evaluate(() => typeof globalThis.__gatesDebug.splatCensus);
if (censusHook !== "function") {
  fail(`tab A: __gatesDebug.splatCensus is ${censusHook} on a dev shard — the splat gate cannot run`);
}
const census = await A.page.evaluate(() => globalThis.__gatesDebug.splatCensus());
if (!(census.vertices > 0)) fail(`tab A: splat census saw ${census.vertices} vertices — the near ring never streamed`);
console.log(
  `  splat: ${census.vertices} vertices over ${census.chunks} chunks (${mat.identities.join("/")}) · ` +
    `dominant ${census.dominantFraction.map((f) => (f * 100).toFixed(1) + "%").join("/")} · ` +
    `present ${census.presentFraction.map((f) => (f * 100).toFixed(1) + "%").join("/")} · ` +
    `spread ${census.spread.map((f) => f.toFixed(2)).join("/")} · ` +
    `${(census.mixedFraction * 100).toFixed(1)}% blended, deepest second identity ` +
    `${census.maxSecond.toFixed(2)}`,
);
// Spread, not biome membership: which biomes the pinned spawn's 320 m ring
// happens to contain is a worldgen fact (the moisture channel's feature size
// is ~700 m, so one ring is often one biome). What must be true of the
// ATTRIBUTE anywhere is that it varies — a constant or a stuck weight vector
// reads zero spread on every channel no matter where the ring is.
const varying = census.spread.filter((s) => s >= SPLAT_MIN_SPREAD).length;
if (varying < SPLAT_MIN_IDENTITIES) {
  fail(
    `tab A: only ${varying} of 4 identity weights vary by ≥ ${SPLAT_MIN_SPREAD} over the near ring ` +
      `(spreads ${census.spread.map((s) => s.toFixed(3)).join(" ")}) — the splat attribute is not a ` +
      `field derived from the world, so the shader is blending a constant`,
  );
}
const held = census.presentFraction.filter((f) => f >= SPLAT_IDENTITY_SHARE).length;
if (held < SPLAT_MIN_IDENTITIES) {
  fail(
    `tab A: ${held} of 4 identities contribute to ≥ ${(SPLAT_IDENTITY_SHARE * 100).toFixed(0)}% of the ` +
      `near ring (${census.presentFraction.map((f) => (f * 100).toFixed(1) + "%").join(" ")}) — ` +
      `the ground is one material wearing a four-slot attribute`,
  );
}
// How gradually they meet. Also location-independent: it needs one boundary
// vertex anywhere in the ring, and the check above guarantees one exists.
// A hard threshold hands each vertex wholly to one identity, so the deepest
// second weight it can ever reach is the cliff mask's — not a biome ramp's.
if (census.maxSecond < SPLAT_MIN_SECOND) {
  fail(
    `tab A: the deepest any second identity gets on the near ring is ${census.maxSecond.toFixed(3)} ` +
      `(floor ${SPLAT_MIN_SECOND}) — two identities never genuinely share a vertex, so the biome ` +
      `bands are hard thresholds and the boundary is a seam on the vertex grid`,
  );
}
if (census.mixedFraction < SPLAT_MIN_MIXED) {
  fail(
    `tab A: only ${(census.mixedFraction * 100).toFixed(2)}% of ${census.vertices} near-ring vertices ` +
      `are a real blend of two identities (floor ${(SPLAT_MIN_MIXED * 100).toFixed(2)}%) — the ` +
      `transition is one row of vertices wide, which is a seam with extra steps`,
  );
}

// Assertion 15 — the procedural field REACHES THE FRAME. Everything above can
// be true while the ground is a flat wash: a field scaled into one lattice
// cell, uniforms never bound, a bump term cancelled by its own footprint
// fade. The probe renders the live scene twice per yaw with the field on and
// off and counts the pixels that moved, by direction. Held fixed across the
// pair: the vertex splat weights, the four authored identities, and the
// causal modifiers. Removed: every contribution of the noise field — the
// weight break-up, the mottling, the roughness variation, the bump.
const surfaceHook = await A.page.evaluate(() => typeof globalThis.__gatesDebug.surfaceProbe);
if (surfaceHook !== "function") {
  fail(`tab A: __gatesDebug.surfaceProbe is ${surfaceHook} on a dev shard — the surface gate cannot run`);
}
const surf = await A.page.evaluate(
  ([yaws, pitch, minDelta]) => globalThis.__gatesDebug.surfaceProbe(yaws, pitch, minDelta),
  [SURFACE_PROBE_YAWS, SURFACE_PROBE_PITCH, SURFACE_PROBE_MIN_DELTA],
);
if (!surf) fail(`tab A: surfaceProbe returned null — the scene never took the terrain material's uniforms`);
const surfDetail = () =>
  surf.samples
    .map(
      (s) =>
        `    yaw ${s.yaw.toFixed(2)}: ${(s.fraction * 100).toFixed(2)}% moved ` +
        `(+${(s.upFraction * 100).toFixed(2)}% / −${(s.downFraction * 100).toFixed(2)}%), ` +
        `mean Δluma ${s.meanDelta.toFixed(1)}, max ${s.maxDelta}`,
    )
    .join("\n");
const surfFraction = surf.changed / surf.pixels;
if (surfFraction < SURFACE_MIN_FRACTION) {
  fail(
    `tab A: the procedural surface moved ${(surfFraction * 100).toFixed(3)}% of ${surf.pixels} probed ` +
      `pixels across ${surf.samples.length} yaws — below ${(SURFACE_MIN_FRACTION * 100).toFixed(2)}%. ` +
      `The material is configured and paints nothing.\n${surfDetail()}`,
  );
}
const flatYaws = surf.samples.filter((s) => s.fraction < SURFACE_MIN_FRACTION_PER_YAW);
if (flatYaws.length) {
  fail(
    `tab A: ${flatYaws.length} of ${surf.samples.length} probed directions are flat ground ` +
      `(floor ${(SURFACE_MIN_FRACTION_PER_YAW * 100).toFixed(1)}% per yaw).\n${surfDetail()}`,
  );
}
// Two-sided: a uniform change can only move the frame one way.
const oneSided = surf.samples.filter(
  (s) => s.upFraction < SURFACE_MIN_DIRECTIONAL || s.downFraction < SURFACE_MIN_DIRECTIONAL,
);
if (oneSided.length) {
  fail(
    `tab A: ${oneSided.length} of ${surf.samples.length} directions moved the frame only one way ` +
      `(floor ${(SURFACE_MIN_DIRECTIONAL * 100).toFixed(1)}% up AND down per yaw). Microstructure ` +
      `lightens some pixels and darkens others; a global tint or exposure shift cannot.\n${surfDetail()}`,
  );
}
console.log(
  `  surface: ${(surfFraction * 100).toFixed(2)}% of ${surf.pixels} probed pixels moved by the ` +
    `procedural field, per yaw ` +
    surf.samples
      .map(
        (s) =>
          `${(s.fraction * 100).toFixed(1)}%(+${(s.upFraction * 100).toFixed(1)}/−${(s.downFraction * 100).toFixed(1)})`,
      )
      .join(" "),
);

// Assertion 15b — the surface has GRAIN, and the grain goes away.
//
// 15 proves the field reaches the frame by counting pixels that moved, and
// that is exactly the measure this pass cannot use: every one of grain's
// failure modes moves pixels. A grain octave scaled into one lattice cell is
// a wash. A contrast that survives to 200 m is an aliasing wash. A per-
// identity trio that collapsed to one number is a wash with four names. What
// separates all of them from grain is neighbour-to-neighbour contrast — the
// defining property of grain is that it changes between one pixel and the
// next — so that is what the probe scores, over the pixels the toggle moved
// and in both states, against a same-state control render that must differ
// nowhere.
const grainHook = await A.page.evaluate(() => typeof globalThis.__gatesDebug.grainProbe);
if (grainHook !== "function") {
  fail(`tab A: __gatesDebug.grainProbe is ${grainHook} on a dev shard — the grain gate cannot run`);
}
// The structural half first, so the pixel failures below are diagnosable: a
// grain wavelength that is not finer than the micro octave's cannot be grain,
// and four identical numbers are one grain wearing four names.
if (!(mat.terrain.grain === 1)) {
  fail(`tab A: the terrain ships with uGrain = ${mat.terrain.grain} — the surface has no grain at all`);
}
if (distinct(mat.grainScale) < 3) {
  fail(
    `tab A: only ${distinct(mat.grainScale)} distinct grain wavelengths in [${mat.grainScale}] /m — ` +
      `sand and rock wear the same grain`,
  );
}
if (distinct(mat.grainRidge) < 3) {
  fail(
    `tab A: only ${distinct(mat.grainRidge)} distinct grain shapes in [${mat.grainRidge}] — every ` +
      `identity gets the same stipple`,
  );
}
const coarseGrain = mat.grainScale.filter((s) => s <= mat.microScale * 4);
if (coarseGrain.length) {
  fail(
    `tab A: grain scales [${coarseGrain}] /m are not meaningfully finer than the micro octave's ` +
      `${mat.microScale.toFixed(3)} /m — that is a fourth mottle, not a grain`,
  );
}
// Grain retires on the same law as every other octave now (15a2), and against
// the same limit. The ceiling here was one cycle per pixel, which was twice
// too generous: aliasing starts at HALF a cycle per pixel, not one, and the
// two structural octaves demonstrated what the slack buys by sitting in it.
if (!(mat.grainFadeCpp[1] > mat.grainFadeCpp[0] && mat.grainFadeCpp[1] <= mat.nyquistCpp)) {
  fail(
    `tab A: the grain footprint fade is [${mat.grainFadeCpp}] cycles/pixel — it must rise and it ` +
      `must retire the octave at or before the ${mat.nyquistCpp} Nyquist limit, which is where it ` +
      `starts aliasing`,
  );
}
const grainDetail = (r) =>
  r.samples
    .map(
      (s) =>
        `    ${s.label}: ${(s.movedFraction * 100).toFixed(3)}% of ${s.scored} moved ` +
        `(+${(s.upFraction * 100).toFixed(2)}/−${(s.downFraction * 100).toFixed(2)}), ` +
        `mean Δluma ${s.meanDelta.toFixed(1)}, contrast ${s.contrastOn.toFixed(2)} vs ` +
        `${s.contrastOff.toFixed(2)} (×${s.contrastRatio.toFixed(3)}), control noise ${s.noise}`,
    )
    .join("\n");
const gr = await A.page.evaluate(
  ([views, minDelta]) => globalThis.__gatesDebug.grainProbe("uGrain", views, minDelta),
  [GRAIN_VIEWS, GRAIN_PROBE_MIN_DELTA],
);
if (!gr) {
  fail(`tab A: grainProbe("uGrain") returned null — the scene never took the terrain material's uniforms`);
}
// The probe's own zero point, before anything is read off it: two renders of
// the SAME state must differ nowhere, or the far view's "grain is gone" is
// the rasterizer agreeing with itself by luck.
for (const s of gr.samples) {
  if (s.noise !== 0) {
    fail(
      `tab A: the grain probe's control differs from its own frame on ${s.noise} pixels at view ` +
        `"${s.label}" — two renders of one state are not identical, so every ceiling below is ` +
        `partly the rasterizer.\n${grainDetail(gr)}`,
    );
  }
}
const grainNear = gr.samples.find((s) => s.label === "near");
const grainFar = gr.samples.find((s) => s.label === "far");
if (!grainNear || !grainFar) {
  fail(`tab A: grain probe returned labels [${gr.samples.map((s) => s.label)}] — expected near and far`);
}
if (grainNear.movedFraction < GRAIN_NEAR_MIN_FRACTION) {
  fail(
    `tab A: grain moved ${(grainNear.movedFraction * 100).toFixed(3)}% of ${grainNear.scored} pixels ` +
      `at arm's length — below ${(GRAIN_NEAR_MIN_FRACTION * 100).toFixed(1)}%. The octave is ` +
      `configured and reaches nothing.\n${grainDetail(gr)}`,
  );
}
if (grainNear.upFraction < GRAIN_MIN_DIRECTIONAL || grainNear.downFraction < GRAIN_MIN_DIRECTIONAL) {
  fail(
    `tab A: grain moved the near frame only one way (+${(grainNear.upFraction * 100).toFixed(2)}% / ` +
      `−${(grainNear.downFraction * 100).toFixed(2)}%, floor ${(GRAIN_MIN_DIRECTIONAL * 100).toFixed(1)}% ` +
      `each). A signed octave lightens some pixels and darkens others; a tint cannot.\n${grainDetail(gr)}`,
  );
}
// The assertion this probe exists for. Everything above is also true of a
// fourth mottle; only this separates detail from a wash.
if (grainNear.contrastRatio < GRAIN_MIN_CONTRAST_RATIO) {
  fail(
    `tab A: grain raised neighbour contrast from ${grainNear.contrastOff.toFixed(2)} to ` +
      `${grainNear.contrastOn.toFixed(2)} luma/pixel over the ${grainNear.moved} pixels it moved — a ` +
      `ratio of ${grainNear.contrastRatio.toFixed(3)} against a floor of ${GRAIN_MIN_CONTRAST_RATIO}. It ` +
      `moved the pixels without adding detail between them, which is a wash and not a ` +
      `grain.\n${grainDetail(gr)}`,
  );
}
// And the ceiling: an octave whose whole justification is arm's length must
// not still be paying for itself, or aliasing in, at 100+ m.
if (grainFar.movedFraction > GRAIN_FAR_MAX_FRACTION) {
  fail(
    `tab A: grain still moves ${(grainFar.movedFraction * 100).toFixed(3)}% of the frame from ` +
      `${GRAIN_FAR_LIFT_M} m up (ceiling ${(GRAIN_FAR_MAX_FRACTION * 100).toFixed(1)}%) — a 4 cm ` +
      `octave that reaches the horizon is an aliasing pattern, which is what the cycles-per-pixel ` +
      `fade exists to retire.\n${grainDetail(gr)}`,
  );
}
console.log(
  `  grain: near ${(grainNear.movedFraction * 100).toFixed(2)}% moved ` +
    `(+${(grainNear.upFraction * 100).toFixed(2)}/−${(grainNear.downFraction * 100).toFixed(2)}), contrast ` +
    `${grainNear.contrastOff.toFixed(2)} → ${grainNear.contrastOn.toFixed(2)} luma/px ` +
    `(×${grainNear.contrastRatio.toFixed(2)}) · far ${(grainFar.movedFraction * 100).toFixed(3)}% moved · ` +
    `control noise ${grainNear.noise}/${grainFar.noise}`,
);

// Assertion 15d — the ground has a HUE that varies, not one hue at forty
// brightnesses.
//
// The structural half first, so a pixel failure below is diagnosable.
if (!(mat.terrain.tint === 1)) {
  fail(`tab A: the terrain ships with uTint = ${mat.terrain.tint} — the ground is back to four flat hues`);
}
if (!Array.isArray(mat.tintDev) || mat.tintDev.length !== 4) {
  fail(`tab A: the material publishes ${mat.tintDev?.length} chromatic deviations — one per identity is four`);
}
// Not parallel to its own colour. This is the check that stops the octave
// quietly becoming a fourth mottle: a deviation proportional to `color` is a
// brightness multiply, and every measure 15 and 15b take would still pass.
const cosine = (a, b) => {
  const dot = a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
  const na = Math.hypot(...a);
  const nb = Math.hypot(...b);
  return na > 0 && nb > 0 ? dot / (na * nb) : 1;
};
const parallel = mat.tintDev
  .map((d, i) => ({ name: mat.identities[i], cos: cosine(d, mat.identityColor[i]), dev: d }))
  .filter((r) => r.cos > TINT_MAX_DEV_PARALLEL);
if (parallel.length) {
  fail(
    `tab A: ${parallel.map((r) => `${r.name} (cos ${r.cos.toFixed(4)}, dev [${r.dev}])`).join(", ")} — ` +
      `the chromatic deviation is parallel to the identity's own colour (ceiling ` +
      `${TINT_MAX_DEV_PARALLEL}), so the "tint" is a brightness multiply and the surface is still ` +
      `one hue at a range of luminances, which is the state this octave exists to leave`,
  );
}
// …and the four axes are not one axis. Blue-over-red is the hue coordinate
// that separates them (sand +0.20, grass −0.30, litter −0.17, rock +0.43).
const hueAxis = mat.tintDev.map((d) => (d[0] !== 0 ? d[2] / d[0] : 0));
if (distinct(hueAxis) < 3) {
  fail(
    `tab A: only ${distinct(hueAxis)} distinct hue axes in the four deviations [${hueAxis.map((h) => h.toFixed(3))}] — ` +
      `the identities swing along one colour direction, which is one tint wearing four names`,
  );
}
// The ladder: this octave fills the gap between the micro octave and the
// finest grain, or it is a duplicate of one of them.
if (distinct(mat.tintScale) < 3) {
  fail(`tab A: only ${distinct(mat.tintScale)} distinct tile wavelengths in [${mat.tintScale}] /m — sand and rock tile alike`);
}
const tintMax = Math.max(...mat.tintScale);
const tintMin = Math.min(...mat.tintScale);
if (!(tintMin > mat.microScale)) {
  fail(
    `tab A: the coarsest tile octave is ${tintMin} /m against the micro octave's ` +
      `${mat.microScale.toFixed(3)} /m — it is not finer than the octave above it, so it is a second micro`,
  );
}
if (!(tintMax < Math.min(...mat.grainScale))) {
  fail(
    `tab A: the finest tile octave is ${tintMax} /m against the coarsest grain's ` +
      `${Math.min(...mat.grainScale)} /m — it is not coarser than the octave below it, so it is a second grain`,
  );
}
if (!(mat.tintFadeCpp[1] > mat.tintFadeCpp[0] && mat.tintFadeCpp[1] <= mat.nyquistCpp)) {
  fail(
    `tab A: the tile footprint fade is [${mat.tintFadeCpp}] cycles/pixel — it must rise and retire the ` +
      `octave at or before the ${mat.nyquistCpp} Nyquist limit (15a2's law, which is the whole reason ` +
      `the last pass existed)`,
  );
}
// The gain that makes the authored poles reachable. Without it the field's own
// deviation (0.2262, measured) puts every square metre at a third of its
// deviation and the `dev` column above describes a colour the ground never is.
if (!(mat.tintGain > 2)) {
  fail(
    `tab A: the tile octave's gain is ${mat.tintGain} — at or below 2 it maps the field's NOMINAL ` +
      `[0,1] onto the deviation, and a value-noise field does not visit its nominal range: the ` +
      `authored poles become a place the material never goes`,
  );
}
// Luminance neutrality, the division of labour this octave is built on: three
// scalar octaves and a per-identity grain already move VALUE, at four scales.
// Nothing moved HUE, which is the defect the visual judge named. A deviation
// that leans on brightness is doing the half already done, and — measured this
// pass — it does it by spending assertion 15's directional margin, which at
// this spawn's yaw 0 is 0.5% against a 0.2% floor before anything is added.
const leaning = mat.tintLumaResidual
  .map((r, i) => ({ name: mat.identities[i], r }))
  .filter((x) => Math.abs(x.r) > mat.tintLumaNeutral);
if (leaning.length) {
  fail(
    `tab A: ${leaning.map((x) => `${x.name} ${x.r.toFixed(4)}`).join(", ")} — the chromatic deviation ` +
      `carries luminance (ceiling ${mat.tintLumaNeutral} of its own length, Rec.709 in the linear ` +
      `working space). This octave is the material's HUE channel; value is the three scalar octaves' ` +
      `and the grain's, and a deviation that brightens or darkens is both duplicating them and ` +
      `spending assertion 15's two-sidedness on a frame the bump already darkens.`,
  );
}
const tintDetail = (r) =>
  r.samples
    .map(
      (s) =>
        `    ${s.label}: ${(s.chromaMovedFraction * 100).toFixed(3)}% of ${s.scored} moved ` +
        `chromatically (warm +${(s.chromaUpFraction * 100).toFixed(2)}/cool ` +
        `−${(s.chromaDownFraction * 100).toFixed(2)}), spread ${s.chromaOff.toFixed(5)} → ` +
        `${s.chromaOn.toFixed(5)} (×${s.chromaRatio.toFixed(3)}), centre moved ` +
        `${s.chromaShift.toFixed(5)}, mean luma ${s.lumaOff.toFixed(2)} → ${s.lumaOn.toFixed(2)}, ` +
        `luma-moved ${(s.movedFraction * 100).toFixed(3)}%, control noise ${s.noise}`,
    )
    .join("\n");
const tn = await A.page.evaluate(
  ([views, minDelta, minChroma]) =>
    globalThis.__gatesDebug.grainProbe("uTint", views, minDelta, minChroma),
  [TINT_VIEWS, TINT_PROBE_MIN_DELTA, TINT_PROBE_MIN_CHROMA],
);
if (!tn) fail(`tab A: grainProbe("uTint") returned null — the terrain material has no tint uniform`);
for (const s of tn.samples) {
  if (s.noise !== 0) {
    fail(
      `tab A: the tint probe's control differs from its own frame on ${s.noise} pixels at view ` +
        `"${s.label}" — two renders of one state are not identical, so nothing below is a ` +
        `measurement.\n${tintDetail(tn)}`,
    );
  }
}
const tintNear = tn.samples.find((s) => s.label === "near");
const tintLevel = tn.samples.find((s) => s.label === "level");
if (!tintNear || !tintLevel) {
  fail(`tab A: tint probe returned labels [${tn.samples.map((s) => s.label)}] — expected near and level`);
}
if (tintNear.chromaMovedFraction < TINT_NEAR_MIN_FRACTION) {
  fail(
    `tab A: the tint moved the chromaticity of ${(tintNear.chromaMovedFraction * 100).toFixed(3)}% of ` +
      `${tintNear.scored} pixels at arm's length — below ${(TINT_NEAR_MIN_FRACTION * 100).toFixed(1)}%. ` +
      `The octave is configured and reaches nothing.\n${tintDetail(tn)}`,
  );
}
// …and it is not only a footprint effect: the standing view is the one a
// player spends the game in, and it is where the acceptance's "no surface in
// any vantage" lands.
if (tintLevel.chromaMovedFraction < TINT_LEVEL_MIN_FRACTION) {
  fail(
    `tab A: the tint moved only ${(tintLevel.chromaMovedFraction * 100).toFixed(3)}% of the standing ` +
      `frame (floor ${(TINT_LEVEL_MIN_FRACTION * 100).toFixed(1)}%) — it reaches the ground under the ` +
      `camera and nothing a player is actually looking at.\n${tintDetail(tn)}`,
  );
}
for (const s of [tintNear, tintLevel]) {
  if (
    s.chromaUpFraction < TINT_MIN_DIRECTIONAL ||
    s.chromaDownFraction < TINT_MIN_DIRECTIONAL
  ) {
    fail(
      `tab A: the tint moved the ${s.label} frame only one way on the red-chromaticity axis ` +
        `(warm +${(s.chromaUpFraction * 100).toFixed(2)}% / cool −${(s.chromaDownFraction * 100).toFixed(2)}%, ` +
        `floor ${(TINT_MIN_DIRECTIONAL * 100).toFixed(1)}% each). A signed deviation makes some ground ` +
        `warmer and some cooler; a cast cannot.\n${tintDetail(tn)}`,
    );
  }
}
// The assertion this probe exists for. Everything above is also true of a
// fourth scalar mottle; only this separates a hue from a brightness.
if (tintNear.chromaRatio < TINT_MIN_CHROMA_RATIO) {
  fail(
    `tab A: the tint raised the chromaticity spread from ${tintNear.chromaOff.toFixed(5)} to ` +
      `${tintNear.chromaOn.toFixed(5)} over the ${tintNear.chromaMoved} pixels it moved — a ratio of ` +
      `${tintNear.chromaRatio.toFixed(3)} against a floor of ${TINT_MIN_CHROMA_RATIO}. A scalar octave ` +
      `scores 1.00 here because multiplying an RGB triple leaves its chromaticity alone, so this is ` +
      `the measure that says the ground has a hue that varies rather than one hue at a range of ` +
      `luminances.\n${tintDetail(tn)}`,
  );
}
// …and it bought that as VARIANCE, not as a cast — at BOTH views, which is a
// bar this pass had to delete a term to clear. Two cuts of this octave carried
// a coarse bias meant to break tiling, one off macro and one off meso; each
// read as a cast, because an octave wider than the frame is a constant inside
// it. See `materials.js`, "what is NOT here". A tint that comes back with a
// coarse offset fails right here.
for (const s of [tintNear, tintLevel]) {
  if (!(s.chromaOn > s.chromaOff) || s.chromaShift > s.chromaOn * TINT_MAX_CENTRE_SHARE) {
    fail(
      `tab A: the tint moved the ${s.label} frame's chromaticity CENTRE by ${s.chromaShift.toFixed(5)} ` +
        `against a cloud ${s.chromaOn.toFixed(5)} wide (ceiling ${TINT_MAX_CENTRE_SHARE} of the width) — ` +
        `that is a colour cast over the whole ground and not a per-class albedo. The identities' ` +
        `authored colours are supposed to be their MEANS, which is why the deviation is signed and ` +
        `added rather than lerped to.\n${tintDetail(tn)}`,
    );
  }
  const lumaShift = Math.abs(s.lumaOn - s.lumaOff);
  if (lumaShift > TINT_MAX_MEAN_LUMA) {
    fail(
      `tab A: the tint moved the ${s.label} frame's mean luma by ${lumaShift.toFixed(2)} steps (ceiling ` +
        `${TINT_MAX_MEAN_LUMA}) — a signed deviation added around an unchanged mean cannot brighten or ` +
        `darken the frame, so this is an exposure slip riding in on a texture.\n${tintDetail(tn)}`,
    );
  }
}
console.log(
  `  tint: tiles ${mat.tintScale.map((s) => (1 / s).toFixed(2) + "m").join("/")} · deviations off-colour ` +
    `${mat.tintDev.map((d, i) => cosine(d, mat.identityColor[i]).toFixed(3)).join("/")} · gain ` +
    `${mat.tintGain} · luma residual ${mat.tintLumaResidual.map((r) => r.toFixed(4)).join("/")} · near ` +
    `${(tintNear.chromaMovedFraction * 100).toFixed(2)}% moved ` +
    `(warm +${(tintNear.chromaUpFraction * 100).toFixed(2)}/cool −${(tintNear.chromaDownFraction * 100).toFixed(2)}), spread ` +
    `${tintNear.chromaOff.toFixed(5)} → ${tintNear.chromaOn.toFixed(5)} (×${tintNear.chromaRatio.toFixed(2)}), ` +
    `centre +${tintNear.chromaShift.toFixed(5)}, luma ${tintNear.lumaOff.toFixed(2)} → ` +
    `${tintNear.lumaOn.toFixed(2)} · level ${(tintLevel.chromaMovedFraction * 100).toFixed(2)}% moved ` +
    `(×${tintLevel.chromaRatio.toFixed(2)} chroma, centre +${tintLevel.chromaShift.toFixed(5)} on ` +
    `+${(tintLevel.chromaOn - tintLevel.chromaOff).toFixed(5)} spread) · control noise ` +
    `${tintNear.noise}/${tintLevel.noise}`,
);

// Assertion 15e — the instrument for a defect this material still has.
//
// 15/15b/15d all ask whether something reaches the image. This asks whether
// something reaches it that should not, and it can name the class exactly:
// `dFdx`/`dFdy`/`fwidth` are differences across the rasterizer's 2x2 quad, so
// anything derived from one is constant inside a quad and unrelated to the
// next quad's, while a noise field evaluated per fragment varies inside a quad
// exactly as much as it varies across the boundary. Comparing the two
// separates them with no threshold on brightness, contrast or detail — which
// is why this can be a wall and not a screenshot. See `scene.aliasProbe`.
//
// What it found, and what is asserted. The ground scores x3.12 and x6.15 here
// — the "literal checkerboard" the visual judge named on pass
// 20260802-163821-01 and a blind reader named on four of six frames — and the
// probe bisects it onto the bump's gradient solve, and inside that onto the
// GRAIN octave alone (`DECISIONS.md` §open, "the quad-constant gradient"). The
// fix is a second sampling law and it is NOT in this commit, because removing
// grain's bump takes assertion 15's two-sidedness with it at two of four yaws
// — the field there is a macro-octave cast plus this artefact and nothing
// else, so the wall above has been passing on the defect below.
//
// **The ship leg is a wall now, and the base maps are why.** It is measured at
// x1.00 at both vantages against the x3.12/x6.15 above, and the mechanism is
// not a fix anybody wrote: a `textureGrad` fetch is evaluated per FRAGMENT, so
// the detail it delivers varies inside a quad exactly as much as it varies
// across one, and it is now the dominant term in the near ground's contrast
// (15h: 5.90 and 8.61 luma/px, against 0.42-0.47 from the octaves alone).
// The quad-locked energy did not go away; it was diluted by two orders of
// magnitude of energy that is not quad-locked. That distinction matters and it
// is why the ceiling below is left exactly where it was rather than tightened
// onto the new reading: the defect `DECISIONS.md` §open ("the quad-constant
// gradient") describes is still IN the shader, still reachable the moment
// something retires the base maps out of a frame, and the sampling law that
// bench measured is still the actual fix. What the wall buys is that the
// dilution cannot silently stop.
//   `nobump`      gmH identically zero -> no derivative in the image at all.
//   `nograinbump` grain's bump alone removed -> the structural octaves' bump
//                 must not put quad-locked energy in the frame either.
//   `ship`        what the player sees, held to the same ceiling since 15h.
const aliasHook = await A.page.evaluate(() => typeof globalThis.__gatesDebug.aliasProbe);
if (aliasHook !== "function") {
  fail(`tab A: __gatesDebug.aliasProbe is ${aliasHook} on a dev shard — the alias gate cannot run`);
}
const alias = await A.page.evaluate(
  ([v, d]) => globalThis.__gatesDebug.aliasProbe(v, d),
  [ALIAS_VIEWS, ALIAS_MIN_DELTA],
);
if (!alias || !alias.samples || alias.samples.length !== ALIAS_VIEWS.length) {
  fail(`tab A: the alias probe returned ${JSON.stringify(alias)} — it did not run`);
}
const aliasDetail = (s) =>
  `      ${s.label}: ship ${s.ship.within.toFixed(2)}/${s.ship.across.toFixed(2)} = ` +
  `x${s.ship.ratio.toFixed(2)} · nobump x${s.nobump.ratio.toFixed(2)} · nograinbump ` +
  `x${s.nograinbump.ratio.toFixed(2)} · mask ${(s.maskFraction * 100).toFixed(1)}% · control noise ${s.noise}`;
for (const s of alias.samples) {
  // The instrument first, in the order that makes a failure readable.
  if (!(s.maskFraction >= ALIAS_MIN_MASK)) {
    fail(
      `tab A: the alias probe's ${s.label} ground mask is ${(s.maskFraction * 100).toFixed(1)}% of the frame ` +
        `(floor ${ALIAS_MIN_MASK * 100}%) — the field paints too little of this view for a quad ratio over it ` +
        `to mean anything.\n${aliasDetail(s)}`,
    );
  }
  if (!(s.noise / (alias.width * alias.height) <= ALIAS_MAX_NOISE)) {
    fail(
      `tab A: two renders of ONE state differ on ${s.noise} pixels of the ${s.label} frame — the rasterizer ` +
        `has noise of its own and every ratio below is partly it talking.\n${aliasDetail(s)}`,
    );
  }
  // …and the two walls.
  if (!(s.nobump.ratio <= ALIAS_MAX_RATIO)) {
    fail(
      `tab A: the ${s.label} frame carries x${s.nobump.ratio.toFixed(2)} of quad-locked energy with gmH ` +
        `identically zero (ceiling x${ALIAS_MAX_RATIO}) — something OTHER than the bump is putting a screen ` +
        `derivative in the image, and this probe's whole bisection is wrong.\n${aliasDetail(s)}`,
    );
  }
  if (!(s.nograinbump.ratio <= ALIAS_MAX_RATIO)) {
    fail(
      `tab A: with grain's bump removed the ${s.label} frame still carries x${s.nograinbump.ratio.toFixed(2)} ` +
        `of quad-locked energy (ceiling x${ALIAS_MAX_RATIO}) — a structural octave's bump has started ` +
        `rendering the quad grid too, which is a second instance of the defect grain already has.` +
        `\n${aliasDetail(s)}`,
    );
  }
  // The ship leg — a wall since the base maps landed (see the block comment).
  if (!(s.ship.ratio <= ALIAS_MAX_RATIO)) {
    fail(
      `tab A: the shipped ${s.label} frame carries x${s.ship.ratio.toFixed(2)} of quad-locked energy ` +
        `(ceiling x${ALIAS_MAX_RATIO}, ${s.ship.within.toFixed(2)} luma/px within quads vs ` +
        `${s.ship.across.toFixed(2)} across). Bisection says ` +
        `${s.nograinbump.ratio < s.ship.ratio * 0.6 ? "the GRAIN octave's bump" : "NOT grain's bump"}.` +
        `\n${aliasDetail(s)}`,
    );
  }
  // …and still reported loudly if it ever climbs back toward the ceiling.
  if (s.ship.ratio > ALIAS_MAX_RATIO) {
    console.log(
      `  alias: KNOWN DEFECT, unwalled — ${s.label} scores x${s.ship.ratio.toFixed(2)} against the ` +
        `x${ALIAS_MAX_RATIO} this will be held to (${s.ship.within.toFixed(2)} luma/px within quads, ` +
        `${s.ship.across.toFixed(2)} across). Bisection says ` +
        `${s.nograinbump.ratio < s.ship.ratio * 0.6 ? "the GRAIN octave's bump" : "NOT grain's bump"}.`,
    );
  }
}
console.log(
  `  alias: ` +
    alias.samples
      .map(
        (s) =>
          `${s.label} ship x${s.ship.ratio.toFixed(2)} (within ${s.ship.within.toFixed(2)}, across ` +
          `${s.ship.across.toFixed(2)}) · floor x${s.nobump.ratio.toFixed(2)} · no-grain-bump ` +
          `x${s.nograinbump.ratio.toFixed(2)} · mask ${(s.maskFraction * 100).toFixed(1)}% · noise ${s.noise}`,
      )
      .join(" · ") + ` · walled legs ≤ x${ALIAS_MAX_RATIO}`,
);

// Assertion 15h — the ground samples REAL DETAIL, and it delivers it.
//
// Two halves. The structural one asks whether the base maps arrived, are the
// right shape, and are wired to the identity they belong to; the pixel one
// asks the only question `ART.md` §3 actually poses, in the units it poses it
// in — 8-bit luma per neighbouring pixel, 6.3 in the reference set and 0.26 in
// ours across eight passes of procedural octaves.
//
// It is the first assertion in this file whose sharp number is an ABSOLUTE.
// 15b, 15c, 15d, 15e and 15f are all ratios, and 15g exists because a ratio is
// scale-free: it cannot tell a field swinging ±0.8 of a level from the same
// field swinging ±17. The base maps are aimed squarely at that hole, so the
// gate that holds them has to have a unit in it.
const baseFacts = mat.base;
if (!baseFacts || baseFacts.loaded !== true) {
  fail(
    `tab A: the client published base maps ${JSON.stringify(baseFacts)} — ART.md §7's working set ` +
      `never reached the ground, and every number 15h reports below would be about the octaves alone`,
  );
}
if (baseFacts.layers.join(",") !== mat.identities.join(",")) {
  fail(
    `tab A: base layer order [${baseFacts.layers}] is not identity order [${mat.identities}] — ` +
      `the splat weight and the array layer are the same index, so this delivers sand's photograph ` +
      `under grass's weight and nothing but a picture would say so`,
  );
}
// The tile is the identity's own declared scale, not a number of its own.
if (baseFacts.scale.join(",") !== mat.tintScale.join(",")) {
  fail(
    `tab A: base tile scales [${baseFacts.scale}] /m have drifted from the identities' declared ` +
      `[${mat.tintScale}] /m — NOW.md item 1 says the base is laid at the scales the identities ` +
      `already declare, and a second table is a second thing to keep true`,
  );
}
// Three maps, four layers, and the unit count that actually has a hard limit.
if (baseFacts.units > baseFacts.unitBudget) {
  fail(
    `tab A: the ground binds ${baseFacts.units} texture units against a ${baseFacts.unitBudget} ` +
      `budget — the terrain program already carries five shadow maps`,
  );
}
// Two counts since the biplanar wall tap (materials v5), and asserting both is
// what keeps this a gate: level ground skips the wall block entirely, so it is
// three maps x the layers; a wall pays a second plane and nothing else may
// quietly appear in that doubling.
if (baseFacts.fetchesLevel !== 3 * baseFacts.layers.length) {
  fail(
    `tab A: base level-ground fetch count ${baseFacts.fetchesLevel} is not three maps x ` +
      `${baseFacts.layers.length} layers`,
  );
}
if (baseFacts.fetchesMax !== 2 * baseFacts.fetchesLevel) {
  fail(
    `tab A: base fetch ceiling ${baseFacts.fetchesMax} is not the two biplanar planes over a ` +
      `level-ground ${baseFacts.fetchesLevel} — the wall tap adds one plane, not a third thing`,
  );
}
for (const [k, size] of [
  ["albedo", baseFacts.albedoSize],
  ["normal", baseFacts.normalSize],
  ["rough", baseFacts.roughSize],
]) {
  if (!(size[0] >= 512 && size[1] >= 512)) {
    fail(`tab A: base ${k} map is ${size[0]}x${size[1]} — below the 512 the manifest ships`);
  }
}
// Anisotropy: the near ground at a grazing angle is the case this whole slice
// is aimed at, and an isotropic mip chain over-blurs exactly it back into the
// wash. A capability, not a knob — so the assertion is that it was ASKED for.
// A capability the material ASKS for, capped: the ask is `BASE_ANISOTROPY_MAX`
// and the delivered value is `min(ask, device max)`, so this asserts both that
// the ask reached the texture and that the cap was not quietly exceeded.
if (!(baseFacts.anisotropy >= 2)) {
  fail(
    `tab A: base maps ship at anisotropy ${baseFacts.anisotropy} (device max ` +
      `${baseFacts.anisotropyDeviceMax}) — nothing was applied, so a 0.6–1 m tile seen along the ` +
      `ground is filtered back to a flat colour`,
  );
}
if (baseFacts.anisotropy > Math.min(baseFacts.anisotropyMax, baseFacts.anisotropyDeviceMax)) {
  fail(
    `tab A: base maps ship at anisotropy ${baseFacts.anisotropy} over a cap of ` +
      `${baseFacts.anisotropyMax} and a device max of ${baseFacts.anisotropyDeviceMax} — the cap is ` +
      `what keeps ${baseFacts.fetchesMax} filtered fetches a fragment affordable`,
  );
}
// The hybrid policy as arithmetic. `albedoGain` is `identity colour / measured
// mean of the layer`, so a gain of exactly 1 in all three channels would mean
// the measurement never happened, and a wild gain means a source far off
// `ART.md` §3's band being pulled onto it — which `MANIFEST.md` predicts by
// name for `rock` (cliff_side, "hue ~25° and far more saturated than granite").
// Both ends are checked because both are real failures.
const gainSpan = baseFacts.albedoGain.map((g) => Math.max(...g) / Math.max(Math.min(...g), 1e-6));
for (let i = 0; i < baseFacts.layers.length; i++) {
  const g = baseFacts.albedoGain[i];
  if (!g.every((v) => Number.isFinite(v) && v > 0.05 && v < 40)) {
    fail(
      `tab A: base layer ${baseFacts.layers[i]} has albedo gain [${g.map((v) => v.toFixed(2))}] — ` +
        `off any scale a mean-preserving gain can reach, so either the mean was measured in the ` +
        `wrong space or the layer is not the file the manifest names`,
    );
  }
  if (!(baseFacts.albedoSd[i] > 0.005)) {
    fail(
      `tab A: base layer ${baseFacts.layers[i]} has linear-luma sd ${baseFacts.albedoSd[i].toFixed(5)} ` +
        `over its own texels — that is a swatch, not a photograph, and it is the one thing this ` +
        `slice buys that a noise field could not`,
    );
  }
}
if (mat.terrain.base !== 1) {
  fail(`tab A: uBase ships at ${mat.terrain.base} — a probe put the base maps back wrong, or a merge landed them off`);
}
console.log(
  `  base maps: ${baseFacts.layers.length} layers, ${baseFacts.units} units, ` +
    `≤${baseFacts.fetchesMax} fetches/frag, aniso ${baseFacts.anisotropy} of ${baseFacts.anisotropyDeviceMax} (cap ${baseFacts.anisotropyMax}), ` +
    `albedo ${baseFacts.albedoSize.join("x")} · normal ${baseFacts.normalSize.join("x")} · ` +
    `rough ${baseFacts.roughSize.join("x")} · ` +
    baseFacts.layers
      .map(
        (n, i) =>
          `${n} sd ${baseFacts.albedoSd[i].toFixed(3)} gain x${gainSpan[i].toFixed(2)} span`,
      )
      .join(" · "),
);

// …and the pixel half.
const baseHook = await A.page.evaluate(() => typeof globalThis.__gatesDebug.baseProbe);
if (baseHook !== "function") {
  fail(`tab A: __gatesDebug.baseProbe is ${baseHook} on a dev shard — 15h's pixel half cannot run`);
}
const base = await A.page.evaluate(
  ([views, minDelta]) => globalThis.__gatesDebug.baseProbe(views, minDelta),
  [BASE_VIEWS, BASE_MIN_DELTA],
);
if (!base) fail(`tab A: baseProbe returned null — the scene never took the terrain material's uniforms`);
const baseDetail = () =>
  base.samples
    .map(
      (s) =>
        `    ${s.label}: ship ${s.contrastShip.toFixed(2)} luma/px vs flat ${s.contrastFlat.toFixed(2)} ` +
        `(x${s.contrastRatio.toFixed(2)}), mask ${(s.maskFraction * 100).toFixed(1)}% ` +
        `(+${(s.upFraction * 100).toFixed(1)}/−${(s.downFraction * 100).toFixed(1)}), noise ${s.noise}`,
    )
    .join("\n");
for (const s of base.samples) {
  if (s.noise > base.width * base.height * BASE_MAX_NOISE) {
    fail(
      `tab A: base probe control differs from its own state on ${s.noise} pixels at ${s.label} — ` +
        `two renders of one scene, so nothing measured from this pair is about the material\n${baseDetail()}`,
    );
  }
  if (s.maskFraction < BASE_MIN_MASK) {
    fail(
      `tab A: the base maps reach ${(s.maskFraction * 100).toFixed(2)}% of the ${s.label} frame ` +
        `(floor ${(BASE_MIN_MASK * 100).toFixed(0)}%) — a photograph differs from a flat swatch ` +
        `nearly everywhere it is laid, so this is the maps not arriving at the ground\n${baseDetail()}`,
    );
  }
  // Two-sided, and this is the assertion that separates real detail from a
  // gain. Lifting the ground raises a contrast number without adding a single
  // edge, and it moves every pixel the same way.
  if (s.upFraction < BASE_MIN_DIRECTIONAL || s.downFraction < BASE_MIN_DIRECTIONAL) {
    fail(
      `tab A: at ${s.label} the base maps moved the frame only one way (floor ` +
        `${(BASE_MIN_DIRECTIONAL * 100).toFixed(1)}% up AND down) — a photograph brightens some ` +
        `pixels and darkens others; a gain cannot\n${baseDetail()}`,
    );
  }
  // THE number. Absolute, in ART.md §3's own units.
  if (s.contrastShip < BASE_MIN_CONTRAST) {
    fail(
      `tab A: near-ground neighbour contrast at ${s.label} is ${s.contrastShip.toFixed(2)} luma/px ` +
        `against a floor of ${BASE_MIN_CONTRAST} and ART.md §3's reference target of ` +
        `${ART_NEAR_GROUND_TARGET}. The octaves alone delivered ${s.contrastFlat.toFixed(2)} here.\n${baseDetail()}`,
    );
  }
  if (s.contrastRatio < BASE_MIN_CONTRAST_RATIO) {
    fail(
      `tab A: the base maps lifted ${s.label}'s neighbour contrast only x${s.contrastRatio.toFixed(2)} ` +
        `over the procedural ground (floor x${BASE_MIN_CONTRAST_RATIO}) — the photograph is being ` +
        `sampled but is not reaching the image as detail\n${baseDetail()}`,
    );
  }
}
console.log(
  `  base detail: ` +
    base.samples
      .map(
        (s) =>
          `${s.label} ${s.contrastShip.toFixed(2)} luma/px (was ${s.contrastFlat.toFixed(2)}, ` +
          `x${s.contrastRatio.toFixed(2)}) mask ${(s.maskFraction * 100).toFixed(0)}%`,
      )
      .join(" · ") +
    ` · floor ${BASE_MIN_CONTRAST}, ART.md §3 target ${ART_NEAR_GROUND_TARGET}`,
);

// Assertion 15i — the near ground's variation is DETAIL, not chroma noise.
//
// The structural half first, because it can say why the pixel half failed.
// `chromaKeep` is derived — `min(1, chromaStretchMax / albedoGainSpan)` per
// layer — and the derivation is what is asserted rather than the four numbers
// it currently produces. A keep that stopped tracking its own span would be a
// knob that had quietly become a taste setting, and swapping a source file
// would stop moving it.
const chromaFacts = baseFacts;
if (!Array.isArray(chromaFacts.chromaKeep) || !Array.isArray(chromaFacts.albedoGainSpan)) {
  fail(
    `tab A: baseFacts published no chroma keep / gain span ` +
      `(${JSON.stringify(chromaFacts.chromaKeep)} / ${JSON.stringify(chromaFacts.albedoGainSpan)}) — ` +
      `15i's structural half cannot run`,
  );
}
for (let i = 0; i < chromaFacts.chromaKeep.length; i++) {
  const span = chromaFacts.albedoGainSpan[i];
  // The span the client publishes and the span this file computes off the raw
  // gains are the same quantity by two routes. They agree or one of them is
  // describing a material the other is not.
  if (Math.abs(span - gainSpan[i]) > 1e-6) {
    fail(
      `tab A: layer ${baseFacts.layers[i]}'s published gain span is x${span.toFixed(4)} but its own ` +
        `albedoGain works out to x${gainSpan[i].toFixed(4)} — the fact the chroma keep is derived ` +
        `from is not the gain the shader was handed`,
    );
  }
  const want = Math.min(1, chromaFacts.chromaStretchMax / Math.max(span, 1e-6));
  if (Math.abs(chromaFacts.chromaKeep[i] - want) > 1e-6) {
    fail(
      `tab A: layer ${baseFacts.layers[i]} keeps ${chromaFacts.chromaKeep[i].toFixed(4)} of its chroma ` +
        `but its gain span is x${span.toFixed(2)}, which derives ${want.toFixed(4)} at a stretch ceiling ` +
        `of ${chromaFacts.chromaStretchMax} — the keep has stopped being derived from the source and is ` +
        `now a number somebody chose`,
    );
  }
  // The product IS the policy: a layer's chroma deviation may not be stretched
  // past the ceiling, whatever its span.
  const stretch = span * chromaFacts.chromaKeep[i];
  if (stretch > chromaFacts.chromaStretchMax + 1e-6) {
    fail(
      `tab A: layer ${baseFacts.layers[i]} delivers its chroma at an effective stretch of ` +
        `x${stretch.toFixed(3)} over a ceiling of x${chromaFacts.chromaStretchMax} — the per-channel ` +
        `gain is amplifying the source's colour noise, not correcting its mean`,
    );
  }
}

// …and the pixel half.
const chromaHook = await A.page.evaluate(() => typeof globalThis.__gatesDebug.chromaProbe);
if (chromaHook !== "function") {
  fail(`tab A: __gatesDebug.chromaProbe is ${chromaHook} on a dev shard — 15i's pixel half cannot run`);
}
const chroma = await A.page.evaluate(
  ([views, minDelta]) => globalThis.__gatesDebug.chromaProbe(views, minDelta),
  [CHROMA_VIEWS, CHROMA_MIN_DELTA],
);
if (!chroma) {
  fail(`tab A: chromaProbe returned null — the scene never took the terrain material's uniforms`);
}
// A LIVE re-read, after the probe rather than from the snapshot this file took
// before any probe ran. `chromaProbe` is the second thing in this tree to
// mutate a shipped uniform and put it back — it holds `uBase` at 0 for one leg
// and every layer's keep at 1 for another — and an assertion that a restore
// happened is worth nothing if it is answered from a copy made beforehand.
// (The pre-existing `grain`, `tint` and `base` toggle checks read that older
// snapshot; that is inherited and is recorded in NOW.md, not fixed here.)
const afterProbe = await A.page.evaluate(() => {
  const d = globalThis.__gatesDebug.materials;
  return { base: d.terrain.base, keep: d.base.chromaKeep, derived: d.base.chromaKeep };
});
if (afterProbe.base !== 1) {
  fail(
    `tab A: uBase reads ${afterProbe.base} after the chroma probe ran — the probe did not put the ` +
      `base maps back, and every frame from here on is missing them`,
  );
}
for (let i = 0; i < chromaFacts.chromaKeep.length; i++) {
  if (Math.abs(afterProbe.keep[i] - chromaFacts.chromaKeep[i]) > 1e-6) {
    fail(
      `tab A: layer ${baseFacts.layers[i]}'s chroma keep reads ${afterProbe.keep[i]} after the chroma ` +
        `probe ran, against ${chromaFacts.chromaKeep[i]} before it — the probe left the material in ` +
        `its measurement state`,
    );
  }
}
const chromaDetail = () =>
  chroma.samples
    .map(
      (s) =>
        `    ${s.label}: ship chroma/luma ${s.ratio.toFixed(3)} (along ${s.along.toFixed(4)}, ` +
        `chroma ${s.chroma.toFixed(4)}) vs unbounded ${s.stretchRatio.toFixed(3)} ` +
        `(along ${s.stretchAlong.toFixed(4)}, chroma ${s.stretchChroma.toFixed(4)}) ` +
        `vs luma-only floor ${s.scalarRatio.toFixed(3)} (along ${s.scalarAlong.toFixed(4)}, ` +
        `chroma ${s.scalarChroma.toFixed(4)}), ` +
        `${s.pairs} windows, mask ${(s.maskFraction * 100).toFixed(1)}%, noise ${s.noise}`,
    )
    .join("\n");
for (const s of chroma.samples) {
  if (s.noise > chroma.width * chroma.height * CHROMA_MAX_NOISE) {
    fail(
      `tab A: chroma probe control differs from its own state on ${s.noise} pixels at ${s.label} — ` +
        `two renders of one scene, so nothing measured from this pair is about the material\n${chromaDetail()}`,
    );
  }
  if (s.pairs < CHROMA_MIN_PAIRS) {
    fail(
      `tab A: the chroma statistic at ${s.label} was taken over ${s.pairs} neighbourhoods ` +
        `(floor ${CHROMA_MIN_PAIRS}) — too few for the ratio below to be about the ground rather ` +
        `than about a handful of pixels\n${chromaDetail()}`,
    );
  }
  // THE ceiling. Absolute and dimensionless, with the reference target beside it.
  if (s.ratio > CHROMA_MAX_RATIO) {
    fail(
      `tab A: at ${s.label} the near ground's high-frequency residual is ${s.ratio.toFixed(3)} ` +
        `chroma per unit luma, over a ceiling of ${CHROMA_MAX_RATIO} (Rust Images/'s in-world ` +
        `frames reach ${REF_CHROMA_TARGET_MAX} at worst, median ${REF_CHROMA_TARGET_MEDIAN}). ` +
        `Neighbouring pixels are changing HUE rather than brightness, which is what per-pixel ` +
        `rainbow speckle is; the same frame's along-colour term is ${s.along.toFixed(4)}, so this ` +
        `is not a shortage of detail\n${chromaDetail()}`,
    );
  }
  // …and the half that stops "delete the base maps" from being a way to pass.
  const suppression = s.ratio > 1e-9 ? s.stretchRatio / s.ratio : 0;
  if (suppression < CHROMA_MIN_SUPPRESSION) {
    fail(
      `tab A: at ${s.label} bounding the chroma stretch changed the frame by only ` +
        `x${suppression.toFixed(2)} (floor x${CHROMA_MIN_SUPPRESSION}) — either the bound has stopped ` +
        `doing anything or there is no photograph under it, and a flat swatch passes the ceiling ` +
        `above by having no chroma residual at all\n${chromaDetail()}`,
    );
  }
}
console.log(
  `  base chroma: ` +
    chroma.samples
      .map(
        (s) =>
          `${s.label} ${s.ratio.toFixed(3)} (unbounded ${s.stretchRatio.toFixed(3)}, ` +
          `x${(s.stretchRatio / Math.max(s.ratio, 1e-9)).toFixed(2)} suppressed, ` +
          `luma-only floor ${s.scalarRatio.toFixed(3)})`,
      )
      .join(" · ") +
    ` · wall ${CHROMA_MAX_RATIO}, Rust Images/ in-world target ${REF_CHROMA_TARGET_MAX} max / ` +
    `${REF_CHROMA_TARGET_MEDIAN} median · keep ` +
    chromaFacts.chromaKeep.map((k, i) => `${baseFacts.layers[i]} ${k.toFixed(2)}`).join("/"),
);

// Assertion 15f — the props have a SURFACE, not only a silhouette.
//
// Two halves, and the structural one runs first because it can say WHY the
// pixel half failed: a class that lost its field, a fade that stopped
// retiring, a deviation that started swinging brightness.
const propFacts = mat.props;
if (!propFacts || !Array.isArray(propFacts.classes)) {
  fail(
    `tab A: scene.materials() published no prop field facts (${JSON.stringify(propFacts)}) — the ` +
      `prop-surface gate cannot run`,
  );
}
if (propFacts.toggle !== 1) {
  fail(
    `tab A: the prop field ships at uProp=${propFacts.toggle}. It is a probe input, not a quality ` +
      `setting — a probe that forgot to put it back, or a merge that landed it at 0, is every prop ` +
      `in the world silently losing its surface`,
  );
}
const withField = propFacts.classes.filter((c) => c.field);
if (withField.length < PROP_MIN_CLASSES) {
  fail(
    `tab A: only ${withField.length} of ${propFacts.classes.length} surface classes carry a field ` +
      `(floor ${PROP_MIN_CLASSES}) — ${propFacts.classes.filter((c) => !c.field).map((c) => c.name).join(", ")} ` +
      `have none`,
  );
}
// Distinct STRUCTURE, not distinct amplitude: two classes with the same ridge
// and the same crevice at different contrasts are one material at two
// brightnesses, which is the defect being fixed, one level up.
const structures = new Set(
  withField.map((c) => `${c.scaleMax}|${c.ridge}|${c.crevice}|${c.scale.join(",")}`),
);
if (structures.size < PROP_MIN_DISTINCT) {
  fail(
    `tab A: the ${withField.length} prop fields hold only ${structures.size} distinct structures ` +
      `(floor ${PROP_MIN_DISTINCT}) — the gap this gate exists for is "you cannot tell our wood from ` +
      `our stone by surface", and a table with one row copied cannot`,
  );
}
for (const c of withField) {
  // Every octave retires below Nyquist — the octave table's own law
  // (assertion 15a2), asked of the prop ladder. Both octaves retire on the
  // same band measured in THEIR OWN cycles (the shader scales the footprint by
  // `detailMul` before comparing), so one comparison covers the ladder, and
  // `detailMul > 1` is what makes the second octave a second octave.
  if (!(propFacts.fadeCpp[1] < propFacts.nyquistCpp)) {
    fail(
      `tab A: the prop ladder retires at ${propFacts.fadeCpp[1]} cycles per pixel, at or past the ` +
        `${propFacts.nyquistCpp} Nyquist limit — an octave sampled above it is indistinguishable from a ` +
        `lower-frequency one, which is what aliases (class "${c.name}")`,
    );
  }
  if (!(propFacts.detailMul > 1)) {
    fail(
      `tab A: the prop ladder's detail octave is the coarse one times ${propFacts.detailMul} — at or ` +
        `below 1 it is not a second octave, it is the first one twice`,
    );
  }
  // ONE law for the whole field: the bump retires on the same band as the
  // albedo it rides on, and both are in cycles per pixel. This is materials
  // v2's finding asserted rather than restated — the ground carried two
  // hand-derived metre thresholds and both were wrong in the same direction,
  // past Nyquist, which is what made it alias. A second band here would be a
  // second threshold to get wrong, and the first cut of this pass had one: it
  // retired the detail octave's relief at half the distance its colour
  // survived, and the pine measured x1.13 neighbour contrast against a x1.30
  // floor. What covers the sparkle a bump at full band could cause is spec AA,
  // which is applied to this exact perturbation.
  if (
    propFacts.bumpFadeCpp[0] !== propFacts.fadeCpp[0] ||
    propFacts.bumpFadeCpp[1] !== propFacts.fadeCpp[1]
  ) {
    fail(
      `tab A: the prop bump retires on [${propFacts.bumpFadeCpp}] cycles per pixel and its albedo on ` +
        `[${propFacts.fadeCpp}] — two bands is two hand-derived thresholds to get wrong, which is exactly ` +
        `how the ground came to sample two octaves past Nyquist (materials v2)`,
    );
  }
  if (!(propFacts.specAA > 0)) {
    fail(
      `tab A: the prop field runs its bump to the full fade band with a spec-AA gain of ` +
        `${propFacts.specAA} — the band and the AA term are one decision, and dropping the AA leaves the ` +
        `normal's own variance to sparkle`,
    );
  }
  // Bounded (wall 4), in the convention this file's slope claims are supposed
  // to be in: a sinusoid's peak slope.
  if (!(c.peakSlope > 0.03 && c.peakSlope < 0.25)) {
    fail(
      `tab A: prop class "${c.name}" asks for a peak bump slope of ${c.peakSlope.toFixed(3)}, outside ` +
        `the 0.03–0.25 band every octave in materials.js is authored against — below it a normal does ` +
        `not read, above it the surface becomes a relief map`,
    );
  }
  if (!(c.peakSlope <= propFacts.bumpMaxSlope)) {
    fail(
      `tab A: prop class "${c.name}" asks for ${c.peakSlope.toFixed(3)} against the ` +
        `${propFacts.bumpMaxSlope} cap the shader clamps at — the material is designed past its own bound`,
    );
  }
  // The chromatic deviation carries no brightness. Zero by construction here
  // (the luminance is projected out), so the bar is the ground's and the
  // measurement is the proof, not the comment.
  if (!(Math.abs(c.devLumaResidual) <= propFacts.lumaNeutral)) {
    fail(
      `tab A: prop class "${c.name}"'s deviation leans ${c.devLumaResidual.toFixed(4)} of its own length ` +
        `on brightness (ceiling ${propFacts.lumaNeutral}) — three scalar terms already move VALUE on this ` +
        `material; the deviation's job is the half nothing else does`,
    );
  }
}
console.log(
  `  prop surfaces: ${withField.length}/${propFacts.classes.length} classes, ${structures.size} distinct ` +
    `structures, ${propFacts.noiseSamples} noise sites/fragment · albedo retires ` +
    `[${propFacts.fadeCpp}] cpp, bump [${propFacts.bumpFadeCpp}] · ` +
    withField
      .map((c) => `${c.name} ${c.scaleMax}/m ridge ${c.ridge} slope ${c.peakSlope.toFixed(3)}`)
      .join(" · "),
);

const propHook = await A.page.evaluate(() => typeof globalThis.__gatesDebug.propProbe);
if (propHook !== "function") {
  fail(`tab A: __gatesDebug.propProbe is ${propHook} on a dev shard — the prop-surface gate cannot run`);
}
const props = await A.page.evaluate(
  ([v, d, r]) => globalThis.__gatesDebug.propProbe(v, d, r),
  [PROP_VIEWS, PROP_MIN_DELTA, PROP_SEARCH_M],
);
if (!props || !props.samples) {
  fail(`tab A: the prop probe returned ${JSON.stringify(props)} — it did not run`);
}
// A view that found no instance is a SKIP, and a skip is the worst bug class
// in this file. The probe reports what it found; if a class the gate asked for
// is not in the world, the gate goes red rather than scoring the classes that
// happened to be there.
if (props.samples.length !== PROP_VIEWS.length) {
  fail(
    `tab A: the prop probe framed ${props.samples.length} of ${PROP_VIEWS.length} views — it found ` +
      `${JSON.stringify((props.found || []).map((f) => `${f.surface}@${f.distance.toFixed(0)}m`))} within ` +
      `${PROP_SEARCH_M} m, so a class this gate asserts is not in the streamed world and the assertion ` +
      `below would have passed by measuring nothing`,
  );
}
const propDetail = (s) =>
  `      ${s.label} (${s.surface}): framed at ${(s.viewDistance || 0).toFixed(1)} m; nearest instance ` +
  `${(s.distance || 0).toFixed(1)} m from spawn of ${s.instances} in its pool, eye ` +
  `[${(s.eye || []).map((v) => v.toFixed(1)).join(", ")}]\n` +
  `      ${s.label} (${s.surface}): value ${s.lumaP05}/${s.lumaP50}/${s.lumaP95} (p05/p50/p95), field ` +
  `amplitude ${s.diffMean.toFixed(2)} luma\n` +
  `      ${s.label} (${s.surface}): mask ${(s.maskFraction * 100).toFixed(2)}% · up ` +
  `${(s.upFraction * 100).toFixed(2)}% / down ${(s.downFraction * 100).toFixed(2)}% · contrast ` +
  `${s.contrastFlat.toFixed(2)} -> ${s.contrastShip.toFixed(2)} (x${s.contrastRatio.toFixed(2)}) · structure ` +
  `${s.diffStep.toFixed(2)}/${s.diffMean.toFixed(2)} = ${s.diffStructure.toFixed(3)} · chroma ` +
  `${s.chromaFlat.toFixed(4)} -> ${s.chromaShip.toFixed(4)} (x${s.chromaRatio.toFixed(2)}) · noise ${s.noise}`;
for (const s of props.samples) {
  if (!(s.noise / (props.width * props.height) <= PROP_MAX_NOISE)) {
    fail(
      `tab A: two renders of ONE state differ on ${s.noise} pixels of the ${s.label} frame — the ` +
        `rasterizer has noise of its own and every measure below is partly it talking.\n${propDetail(s)}`,
    );
  }
  if (!(s.maskFraction >= PROP_MIN_FRACTION)) {
    fail(
      `tab A: the prop field reaches ${(s.maskFraction * 100).toFixed(2)}% of the ${s.label} frame ` +
        `(floor ${PROP_MIN_FRACTION * 100}%) — either the ${s.surface} class contributes nothing, or the ` +
        `probe framed no prop.\n${propDetail(s)}`,
    );
  }
  if (!(s.upFraction >= PROP_MIN_DIRECTIONAL && s.downFraction >= PROP_MIN_DIRECTIONAL)) {
    fail(
      `tab A: the ${s.label} frame moved ${(s.upFraction * 100).toFixed(2)}% up and ` +
        `${(s.downFraction * 100).toFixed(2)}% down (floor ${PROP_MIN_DIRECTIONAL * 100}% each) — a field ` +
        `that only darkens is a wash, and a wash is what this class already had.\n${propDetail(s)}`,
    );
  }
  if (!(s.contrastRatio >= PROP_MIN_CONTRAST_RATIO)) {
    fail(
      `tab A: the ${s.label} frame's neighbour contrast went ${s.contrastFlat.toFixed(2)} -> ` +
        `${s.contrastShip.toFixed(2)} luma/px, x${s.contrastRatio.toFixed(2)} (floor ` +
        `x${PROP_MIN_CONTRAST_RATIO}) — the field moved these pixels without changing the detail between ` +
        `them, which is a colour change wearing a texture's name.\n${propDetail(s)}`,
    );
  }
  if (!(s.diffStructure >= PROP_MIN_STRUCTURE)) {
    fail(
      `tab A: the ${s.label} field's own difference image varies ${s.diffStep.toFixed(2)} luma between ` +
        `neighbours against a magnitude of ${s.diffMean.toFixed(2)} — a structure of ` +
        `${s.diffStructure.toFixed(3)} (floor ${PROP_MIN_STRUCTURE}). A wash scores exactly 0 and this is ` +
        `nearly one: whatever the toggle changed, it changed it the same amount everywhere.` +
        `\n${propDetail(s)}`,
    );
  }
  if (!(s.chromaRatio >= PROP_MIN_CHROMA_RATIO)) {
    fail(
      `tab A: the ${s.label} frame's chromaticity spread went ${s.chromaFlat.toFixed(4)} -> ` +
        `${s.chromaShip.toFixed(4)}, x${s.chromaRatio.toFixed(2)} (floor x${PROP_MIN_CHROMA_RATIO}) — the ` +
        `field is moving brightness only, so the class is still one hue at N values.\n${propDetail(s)}`,
    );
  }
  // 15g's pixel half. The two above with a unit in them.
  if (!(s.lumaP50 >= PROP_MIN_VALUE)) {
    fail(
      `tab A: the ${s.label} class is delivered at median luma ${s.lumaP50} of 255 (floor ` +
        `${PROP_MIN_VALUE}), spanning ${s.lumaP05}..${s.lumaP95} — an 8-bit framebuffer cannot carry a ` +
        `texture down there, so every RATIO above can be green while the surface is invisible. Either the ` +
        `authored albedo went under the dielectric band or the light on it did.\n${propDetail(s)}`,
    );
  }
  if (!(s.diffMean >= PROP_MIN_AMP)) {
    fail(
      `tab A: the ${s.label} field is worth ${s.diffMean.toFixed(2)} levels of 255 where it is delivered ` +
        `(floor ${PROP_MIN_AMP}), on a class whose median value is ${s.lumaP50} — the field is real, it is ` +
        `structured (${s.diffStructure.toFixed(3)}), and it is invisible. This is the assertion prop ` +
        `surfaces v0 did not have: its three ratios score a ±0.8-level swing on a base of 6 exactly as ` +
        `they score a ±17-level swing on a base of 120.\n${propDetail(s)}`,
    );
  }
  // …and the dark tail, which prop albedo v1 measured, named and deliberately
  // left unwalled because the light rig owned it. Lighting v1 owns it now.
  if (!(s.lumaP05 >= PROP_MIN_P05)) {
    fail(
      `tab A: the darkest twentieth of the ${s.label} class sits at luma ${s.lumaP05} of 255 (floor ` +
        `${PROP_MIN_P05}), against a field worth ${s.diffMean.toFixed(2)} levels — the shaded side of ` +
        `this prop is below where its own surface can exist, so the class reads as two materials: one ` +
        `textured and one black. This is the visual judge's "94.9% of the skirt under 10/255", gated. ` +
        `It is a LIGHT failure before it is an albedo one: a down-facing face receives only ` +
        `groundColor x fillIntensity x albedo / pi.\n${propDetail(s)}`,
    );
  }
}
console.log(
  `  prop probe: ` +
    props.samples
      .map(
        (s) =>
          `${s.label} framed at ${(s.viewDistance || 0).toFixed(1)} m (nearest instance ` +
          `${(s.distance || 0).toFixed(1)} m from spawn): mask ${(s.maskFraction * 100).toFixed(2)}% · ` +
          `±${(s.upFraction * 100).toFixed(2)}/${(s.downFraction * 100).toFixed(2)}% · contrast ` +
          `x${s.contrastRatio.toFixed(2)} · structure ${s.diffStructure.toFixed(3)} · chroma ` +
          `x${s.chromaRatio.toFixed(2)} · value ${s.lumaP05}/${s.lumaP50}/${s.lumaP95} ` +
          `(amp ${s.diffMean.toFixed(2)}) · noise ${s.noise}`,
      )
      .join(" · "),
);

// Assertion 15c — the grain is laid ON the surface, not stamped through it.
//
// 15b's every measure is blind to this. A grain combed downhill moves the same
// pixels, by the same amount, in both directions, at the same neighbour
// contrast as one lying on the slope — the two frames differ in the SHAPE of
// the detail and in nothing 15b counts. So this scores the shipped program
// against `flatgrain` (materials v1's world-XZ tap, everything else identical)
// from one camera aimed down a real slope's fall line, on the grain's own
// difference image, along both screen axes.
const projHook = await A.page.evaluate(() => [
  typeof globalThis.__gatesDebug.projectionProbe,
  typeof globalThis.__gatesDebug.steepestFace,
]);
if (projHook[0] !== "function" || projHook[1] !== "function") {
  fail(
    `tab A: __gatesDebug.projectionProbe is ${projHook[0]} and steepestFace is ${projHook[1]} on a ` +
      `dev shard — the projection gate cannot run`,
  );
}
// The structural half: the shipped program must SAY it is triplanar and the
// partner must say it is not, or the two frames below are one program measured
// twice and every ratio is 1.000 for free.
if (mat.grainProjection !== "triplanar") {
  fail(
    `tab A: the shipped ground samples its grain on "${mat.grainProjection}" — the octave is still ` +
      `projected from above, so a slope is combed downhill`,
  );
}
if (mat.flatGrainProjection !== "xz") {
  fail(
    `tab A: the projection partner samples its grain on "${mat.flatGrainProjection}", not "xz" — ` +
      `the gate below would be comparing the shipped program against itself`,
  );
}
if (!(mat.grainTaps === 3)) {
  fail(
    `tab A: the shipped grain takes ${mat.grainTaps} noise taps — a triplanar tap is three, one per ` +
      `world plane, and the facts do not match the program`,
  );
}
// Find the slope. Nothing here is hardcoded: a worldgen change that moved the
// pinned spawn onto flat ground fails loudly instead of scoring a meadow, and
// the sun direction the search is filtered by is read off the live rig rather
// than written down, so a lighting change cannot silently aim the probe into
// shadow (which is where the unfiltered search put it).
const sunDir = [
  Math.cos(lit.sunElevation) * Math.sin(lit.sunAzimuth),
  Math.sin(lit.sunElevation),
  Math.cos(lit.sunElevation) * Math.cos(lit.sunAzimuth),
];
const face = await A.page.evaluate(
  ([r, b, m, sun, lit]) => globalThis.__gatesDebug.steepestFace(r, b, m, sun, lit),
  [PROJ_FACE_RADIUS_M, PROJ_FACE_BIN_M, PROJ_FACE_MIN_VERTS, sunDir, PROJ_FACE_MIN_LIT],
);
if (!face || !face.found) {
  fail(
    `tab A: no ground face of ${PROJ_FACE_MIN_VERTS}+ vertices within ${PROJ_FACE_RADIUS_M} m of ` +
      `spawn (${JSON.stringify(face)}) — the near ring never streamed, so the projection has ` +
      `nothing to be measured on`,
  );
}
if (!(face.upness <= PROJ_FACE_MAX_UPNESS)) {
  fail(
    `tab A: the steepest coherent face within ${PROJ_FACE_RADIUS_M} m of spawn is ` +
      `${face.slopeDeg.toFixed(1)}° (upness ${face.upness.toFixed(3)}, ceiling ` +
      `${PROJ_FACE_MAX_UPNESS}) across ${face.candidates} lit candidate bins (${face.unlit} more ` +
      `rejected as facing away from the sun). A world-XZ grain is ` +
      `stretched by 1/upness, so on ground this level there is no comb to measure and this gate ` +
      `would pass by default`,
  );
}
if (!(face.coherence >= PROJ_FACE_MIN_COHERENCE)) {
  fail(
    `tab A: the face at ${face.key} has coherence ${face.coherence.toFixed(3)} over ${face.verts} ` +
      `vertices (floor ${PROJ_FACE_MIN_COHERENCE}) — its normals disagree, so it is a ridge or a ` +
      `crumple and not a face with one fall line`,
  );
}
// The eye: straight out along the face's own normal, looking back down it.
const fn = face.normal;
const PROJ_VIEWS = [
  {
    label: "face",
    eye: [
      face.center[0] + fn[0] * PROJ_EYE_DIST_M,
      face.center[1] + fn[1] * PROJ_EYE_DIST_M,
      face.center[2] + fn[2] * PROJ_EYE_DIST_M,
    ],
    at: face.center,
  },
  // The control the face is measured against: the SAME probe, straight down at
  // ground that is not tilted, from the player's own eye. Everything the pair
  // shares — the octave, the fade, the light, the instrument — cancels between
  // them; what does not is the tilt.
  {
    label: "flat",
    eye: face.eye,
    at: [face.eye[0], face.eye[1] - 1, face.eye[2]],
  },
  {
    label: "retired",
    eye: [face.eye[0], face.eye[1] + GRAIN_FAR_LIFT_M, face.eye[2]],
    at: [
      face.eye[0] + Math.sin(0) * Math.cos(GRAIN_FAR_PITCH),
      face.eye[1] + GRAIN_FAR_LIFT_M + Math.sin(GRAIN_FAR_PITCH),
      face.eye[2] + Math.cos(0) * Math.cos(GRAIN_FAR_PITCH),
    ],
  },
];
const pr = await A.page.evaluate(
  ([views, minDelta]) => globalThis.__gatesDebug.projectionProbe(views, minDelta),
  [PROJ_VIEWS, GRAIN_PROBE_MIN_DELTA],
);
if (!pr) {
  fail(
    `tab A: projectionProbe returned null — the scene never took the terrain's cost hooks, so the ` +
      `projection partner was never built`,
  );
}
const projDetail = (r) =>
  r.samples
    .map(
      (s) =>
        `    ${s.label}/${s.program}: ${(s.movedFraction * 100).toFixed(2)}% moved, amp ` +
        `${s.amp.toFixed(2)} luma, grad ${s.gradX.toFixed(3)}x/${s.gradY.toFixed(3)}y ` +
        `(anisotropy ${s.anisotropy.toFixed(4)}), control noise ${s.noise}` +
        (s.vsFirst
          ? `, vs triplanar ${s.vsFirst.changed} px (max ${s.vsFirst.maxDelta}/255, mean ` +
            `${s.vsFirst.meanAbsMasked.toFixed(2)} over the grain mask)`
          : ""),
    )
    .join("\n");
// The probe's own zero point, before anything is read off it.
for (const s of pr.samples) {
  if (s.noise !== 0) {
    fail(
      `tab A: the projection probe's control differs from its own frame on ${s.noise} pixels at ` +
        `"${s.label}/${s.program}" — two renders of one state are not identical, so every ratio ` +
        `below is partly the rasterizer.\n${projDetail(pr)}`,
    );
  }
}
const projAt = (label, program) =>
  pr.samples.find((s) => s.label === label && s.program === program);
const faceTri = projAt("face", "triplanar");
const faceXZ = projAt("face", "xz");
const flatTri = projAt("flat", "triplanar");
const flatXZ = projAt("flat", "xz");
const retiredXZ = projAt("retired", "xz");
if (!faceTri || !faceXZ || !flatTri || !flatXZ || !retiredXZ) {
  fail(
    `tab A: the projection probe returned [${pr.samples.map((s) => `${s.label}/${s.program}`)}] — ` +
      `expected face, flat and retired in both programs`,
  );
}
if (faceTri.movedFraction < PROJ_MIN_MOVED || faceXZ.movedFraction < PROJ_MIN_MOVED) {
  fail(
    `tab A: grain reaches ${(faceTri.movedFraction * 100).toFixed(2)}% (triplanar) and ` +
      `${(faceXZ.movedFraction * 100).toFixed(2)}% (xz) of the face view, floor ` +
      `${(PROJ_MIN_MOVED * 100).toFixed(1)}% — the eye is too far off the face for the octave to be ` +
      `live there, so every ratio below is taken over two nothings.\n${projDetail(pr)}`,
  );
}
if (flatTri.movedFraction < PROJ_MIN_MOVED_FLAT || flatXZ.movedFraction < PROJ_MIN_MOVED_FLAT) {
  fail(
    `tab A: grain reaches ${(flatTri.movedFraction * 100).toFixed(2)}% (triplanar) and ` +
      `${(flatXZ.movedFraction * 100).toFixed(2)}% (xz) of the flat control view, floor ` +
      `${(PROJ_MIN_MOVED_FLAT * 100).toFixed(1)}% — the control the face is measured against ` +
      `carries no octave.\n${projDetail(pr)}`,
  );
}
// The grain's own DETAIL: direction-averaged so the view's screen-axis bias
// drops out, and divided by the octave's amplitude so a louder grain does not
// read as a finer one. It is an inverse characteristic length in pixels.
const detail = (s) => (s.gradX + s.gradY) / (2 * Math.max(s.amp, 1e-6));
// How much each program's grain coarsens when the ground tilts under it. This
// is the defect, stated as a number: a field stamped from above is stretched by
// 1/upness along the fall line and therefore coarsens; a field laid on the
// surface does not know the surface tilted.
const stretchXZ = detail(flatXZ) / Math.max(detail(faceXZ), 1e-9);
const stretchTri = detail(flatTri) / Math.max(detail(faceTri), 1e-9);
const stretchGain = stretchXZ / Math.max(stretchTri, 1e-9);
// The flat control first: with no tilt there is nothing for the two
// projections to disagree about, and a control that disagrees with itself
// cannot calibrate the face.
const flatSpread = Math.abs(detail(flatTri) / Math.max(detail(flatXZ), 1e-9) - 1);
if (!(flatSpread <= PROJ_FLAT_MAX_SPREAD)) {
  fail(
    `tab A: on the flat control the two projections measure grain detail ` +
      `${detail(flatTri).toFixed(5)} (triplanar) and ${detail(flatXZ).toFixed(5)} (xz) — ` +
      `${(flatSpread * 100).toFixed(1)}% apart against a ${(PROJ_FLAT_MAX_SPREAD * 100).toFixed(0)}% ` +
      `ceiling. Level ground is where the blend weights are (0,1,0) and the two programs are the ` +
      `same arithmetic; a control that disagrees with itself cannot calibrate the ` +
      `face.\n${projDetail(pr)}`,
  );
}
// THE assertion. Every confound the face view carries — its distance, its
// splat identity, its fade, its lighting, the terrain's curvature across a
// 107° frustum, the mask's own selection bias — is in both programs' flat and
// face numbers alike, so it cancels twice over. What is left is the projection.
if (!(stretchGain >= PROJ_MIN_STRETCH_GAIN)) {
  fail(
    `tab A: tilting the ground to ${face.slopeDeg.toFixed(1)}° (upness ${face.upness.toFixed(3)}, a ` +
      `x${(1 / face.upness).toFixed(3)} stretch down the fall line) coarsens the shipped grain by ` +
      `x${stretchTri.toFixed(3)} and the world-XZ grain by x${stretchXZ.toFixed(3)} — a gain of ` +
      `x${stretchGain.toFixed(3)} against a floor of ${PROJ_MIN_STRETCH_GAIN}. The octave is still ` +
      `stamped through the surface from above rather than laid on it.\n${projDetail(pr)}`,
  );
}
// …and it was not bought by deleting the grain.
const ampRatio = faceXZ.amp > 0 ? faceTri.amp / faceXZ.amp : 0;
if (!(ampRatio >= PROJ_MIN_AMP_RATIO)) {
  fail(
    `tab A: the shipped grain contributes ${faceTri.amp.toFixed(2)} luma per moved pixel on the ` +
      `face against the world-XZ program's ${faceXZ.amp.toFixed(2)} — x${ampRatio.toFixed(3)}, ` +
      `floor ${PROJ_MIN_AMP_RATIO}. A triplanar blend without the 1/|w| deviation restore wins the ` +
      `check above by fading the octave out on exactly the faces it was reprojected ` +
      `for.\n${projDetail(pr)}`,
  );
}
// The confinement half. From 60 m up the cycles-per-pixel fade has already
// retired the octave entirely (15b asserts 0.000% of that frame moves), so the
// only instructions the two programs disagree about contribute nothing — and
// the frames must land on each other. Not bit-exactly: two separately compiled
// programs may schedule identical arithmetic in a different order, and a
// last-bit difference at a silhouette or a smoothstep knee flips whole
// fragments. That is the same allowance `COST_IDENTITY_MAX_DELTA` already makes
// for `nofield` and `nograin`, and the ceiling here is tight enough that a
// projection leaking outside the grain block cannot hide under it.
if (retiredXZ.vsFirst.changedFraction > PROJ_RETIRED_MAX_FRACTION) {
  fail(
    `tab A: with grain retired at ${GRAIN_FAR_LIFT_M} m up, the triplanar and world-XZ programs ` +
      `still differ on ${retiredXZ.vsFirst.changed} pixels — ` +
      `${(retiredXZ.vsFirst.changedFraction * 100).toFixed(4)}% of the frame, over the ` +
      `${(PROJ_RETIRED_MAX_FRACTION * 100).toFixed(4)}% ceiling, max ` +
      `${retiredXZ.vsFirst.maxDelta}/255. The projection change is not confined to the grain ` +
      `octave.\n${projDetail(pr)}`,
  );
}
// …and confinement to SLOPES, which is what "level ground is unchanged" means
// once it is measured instead of derived. The algebra says the tap is bit-exact
// where the normal is exactly vertical, and no frame contains such a fragment —
// a heightfield's noise is continuous, so exactly-level is measure-zero. What a
// frame CAN show is the two projections landing within one luma step of each
// other over near-level ground and pulling apart on a face, and that is what
// these two bars hold. Note the size to expect: the difference is bounded by
// the octave itself, so a correctly paired reading cannot exceed ~2 — the pass
// that first shipped this assertion read 4.867 and 2.494 because the probe was
// comparing each frame against another CAMERA's, which is the bug the per-view
// reference array in `scene.js` now prevents.
const faceEffect = faceXZ.vsFirst.meanAbsMasked / Math.max(faceXZ.amp, 1e-6);
const flatEffect = flatXZ.vsFirst.meanAbsMasked / Math.max(flatXZ.amp, 1e-6);
if (!(faceEffect >= PROJ_MIN_FACE_EFFECT)) {
  fail(
    `tab A: the two projections put the frame only ${faceEffect.toFixed(3)} of the octave's own ` +
      `amplitude apart on a ${face.slopeDeg.toFixed(1)}° face (floor ${PROJ_MIN_FACE_EFFECT}, max ` +
      `${faceXZ.vsFirst.maxDelta}/255 over ${faceXZ.vsFirst.changed} px). On a face this steep the ` +
      `triplanar tap reads mostly OTHER planes than XZ, so the two programs cannot agree this ` +
      `closely unless the tap is not reading the surface normal at all.\n${projDetail(pr)}`,
  );
}
if (!(flatEffect <= PROJ_MAX_FLAT_EFFECT)) {
  fail(
    `tab A: the two projections put the frame ${flatEffect.toFixed(3)} of the octave's own ` +
      `amplitude apart on the flat control (ceiling ${PROJ_MAX_FLAT_EFFECT}, max ` +
      `${flatXZ.vsFirst.maxDelta}/255 over ${flatXZ.vsFirst.changed} px), against ` +
      `${faceEffect.toFixed(3)} on the ${face.slopeDeg.toFixed(1)}° face. Where the ground is level ` +
      `the blend weights are (0,1,0) and the triplanar tap IS the world-XZ tap; a projection that ` +
      `moves level ground this far is not blending on the surface normal.\n${projDetail(pr)}`,
  );
}
console.log(
  `  grain projection: triplanar, ${mat.terrain.cost.noiseSamples} noise sites/fragment · face ` +
    `${face.slopeDeg.toFixed(1)}° (upness ${face.upness.toFixed(3)}, lit ${face.lit.toFixed(3)}, ` +
    `coherence ${face.coherence.toFixed(3)}, ${face.verts} verts, ${face.candidates} lit / ` +
    `${face.unlit} unlit of ${face.bins} bins in ${face.radiusM} m; albedo channel only)\n` +
    `${projDetail(pr)}\n` +
    `  grain projection cont: tilting to ${face.slopeDeg.toFixed(1)}° (x` +
    `${(1 / face.upness).toFixed(3)} stretch) coarsens the grain x${stretchXZ.toFixed(3)} on world ` +
    `XZ and x${stretchTri.toFixed(3)} triplanar — gain x${stretchGain.toFixed(3)} over ` +
    `${PROJ_MIN_STRETCH_GAIN}, at x${ampRatio.toFixed(3)} amplitude; flat control agrees to ` +
    `${(flatSpread * 100).toFixed(1)}%; the two projections sit ${faceEffect.toFixed(3)} of the ` +
    `octave apart on the face (floor ${PROJ_MIN_FACE_EFFECT}, max ${faceXZ.vsFirst.maxDelta}/255) ` +
    `and ${flatEffect.toFixed(3)} on level ground (ceiling ${PROJ_MAX_FLAT_EFFECT}, max ` +
    `${flatXZ.vsFirst.maxDelta}/255) — x${(faceEffect / Math.max(flatEffect, 1e-6)).toFixed(1)}; ` +
    `${retiredXZ.vsFirst.changed} px where the octave is retired`,
);

// Assertion 16 — THE FRAGMENT BUDGET. Assertions 9–15 all prove a system
// reaches the image; none of them can say what it costs, because all of them
// work by weighting a term to zero with a uniform, and a uniform removes no
// instruction. That gap is what `NOW.md` item 1 ran into: grain measured well
// and did not merge because the terrain program was already too expensive for
// the third browser tab, and the two named suspects — per-fragment cost and
// program size — could only be argued about.
//
// The counted half first, because it is the half that means the same thing on
// the reference VPS as it does on this box.
const fragCost = mat.terrain.cost;
const fragProgram = mat.terrain.programStats;
if (!fragCost || !fragProgram) {
  fail(
    `tab A: the terrain material reports cost=${JSON.stringify(fragCost)} ` +
      `programStats=${JSON.stringify(fragProgram)} — the program was never measured, so the ` +
      `fragment budget below is asserting nothing`,
  );
}
if (fragCost.variant !== "ship") {
  fail(
    `tab A: the ground is rendering the "${fragCost.variant}" cost variant — those exist for ` +
      `costProbe and must never be what a player sees`,
  );
}
// Level 0's filter cost is read out of three's own installed chunk rather
// than quoted in a constant, so this asserts the two agree: the clipmap's
// reported total must be the sum of the per-level taps it publishes.
const tapSum = cm.levels.reduce((n, L) => n + L.filterTaps, 0);
if (cm.depthFetches !== tapSum) {
  fail(
    `tab A: the clipmap reports ${cm.depthFetches} depth fetches per fragment but its levels ` +
      `sum to ${tapSum} ([${cm.levels.map((L) => L.filterTaps)}]) — the two are derived from ` +
      `different things and one of them is wrong`,
  );
}
if (fragCost.depthFetches !== cm.depthFetches) {
  fail(
    `tab A: the ground's program claims ${fragCost.depthFetches} depth fetches, the clipmap ` +
      `claims ${cm.depthFetches} — the ground is not shadowed by the clipmap the gate measured`,
  );
}
if (cm.depthFetches > DEPTH_FETCH_BUDGET) {
  fail(
    `tab A: every shaded fragment pays ${cm.depthFetches} shadow depth fetches ` +
      `([${cm.levels.map((L) => L.filterTaps)}] per level), over the ${DEPTH_FETCH_BUDGET} budget. ` +
      `This is the per-fragment cost DESIGN §9's draw-call and triangle budgets do not see.`,
  );
}
if (!(fragProgram.resolvedFragmentChars > fragProgram.fragmentChars)) {
  fail(
    `tab A: the terrain program resolves to ${fragProgram.resolvedFragmentChars} chars from a ` +
      `${fragProgram.fragmentChars}-char template — the #include expansion did nothing, so the ` +
      `size below is a template's size and not a program's`,
  );
}
// How much of that program is THIS repo's, measured against three's own
// unpatched template captured before the first replace — not inferred from a
// variant. The first cut of this gate took the `noshadow` variant's size for
// "stock" and called the remainder ours; a variant is the shipped program
// minus one term, and that mislabel understated this repo's share by 2.9x.
if (!(fragCost.stockFragmentChars > 0)) {
  fail(
    `tab A: the terrain material reports a ${fragCost.stockFragmentChars}-char stock program — ` +
      `the unpatched template was never measured, so the split below is inferred and not counted`,
  );
}
if (!(fragCost.stockFragmentChars < fragProgram.resolvedFragmentChars)) {
  fail(
    `tab A: three's unpatched standard material resolves to ${fragCost.stockFragmentChars} chars ` +
      `against the patched ${fragProgram.resolvedFragmentChars} — the ground's patches added no ` +
      `source, so one of the two is not the program it claims to be`,
  );
}
if (fragProgram.resolvedFragmentChars > TERRAIN_FRAGMENT_BUDGET) {
  fail(
    `tab A: the terrain fragment program is ${fragProgram.resolvedFragmentChars} chars of GLSL, ` +
      `over the ${TERRAIN_FRAGMENT_BUDGET} budget. Program size is the other half of what a ` +
      `software rasterizer charges a joining tab (NOW.md item 1).`,
  );
}
// The third counted axis, and the one the triplanar projection actually spends
// in: noise SAMPLE SITES per shaded fragment. `NOW.md` item 1 rules the timed
// question out by name on this box — six runs of the cost probe read five of
// six the wrong sign — and says to price the projection counted instead. This
// is that price, asserted rather than printed.
if (fragCost.noiseSamples > NOISE_SAMPLE_BUDGET) {
  fail(
    `tab A: every shaded ground fragment takes ${fragCost.noiseSamples} noise sample sites ` +
      `(${fragCost.noiseSamples - fragCost.grainTaps - fragCost.tintTaps} field + ` +
      `${fragCost.grainTaps} grain + ${fragCost.tintTaps} tint, ` +
      `projection "${fragCost.grainProjection}"), over the ${NOISE_SAMPLE_BUDGET} budget. Each site ` +
      `is four hash evaluations, and this is the axis a projection change is spent on.`,
  );
}

// The timed half. Every number the probe returns below is measured on a
// shared 4-core box through a software rasterizer, so none of them is a claim
// about reference hardware and none is asserted as a threshold. What IS
// asserted is that each variant differs from the shipped program in the
// image, in the direction it must — because a time difference between two
// programs is worth nothing unless the image difference is known.
const costHook = await A.page.evaluate(() => typeof globalThis.__gatesDebug.costProbe);
if (costHook !== "function") {
  fail(`tab A: __gatesDebug.costProbe is ${costHook} on a dev shard — the fragment-budget gate cannot run`);
}
const cost = await A.page.evaluate(
  ([yaw, pitch, scales, frames, reps]) =>
    globalThis.__gatesDebug.costProbe(yaw, pitch, scales, frames, reps),
  [COST_PROBE_YAW, COST_PROBE_PITCH, COST_PROBE_SCALES, COST_PROBE_FRAMES, COST_PROBE_REPS],
);
if (!cost) fail(`tab A: costProbe returned null — the scene never took the terrain cost variants`);
const byName = Object.fromEntries(cost.variants.map((v) => [v.variant, v]));
for (const want of ["ship", "nofield", "nograin", "near1", "noshadow", "noskip", "control"]) {
  if (!byName[want]) fail(`tab A: costProbe measured no "${want}" variant — it measured [${Object.keys(byName)}]`);
}
// Each variant must have swept every scale, or its fit is a line through one
// point and the slope it reports is arbitrary.
for (const v of cost.variants) {
  if (v.points.length !== COST_PROBE_SCALES.length) {
    fail(`tab A: variant "${v.variant}" swept ${v.points.length} of ${COST_PROBE_SCALES.length} scales`);
  }
  // The timed frames must have DRAWN, and drawn the same scene: a variant
  // changes what the ground's fragments cost, never what is submitted. Equal
  // call counts across variants is what makes the ms difference between two
  // of them a shading difference rather than a geometry one.
  for (const p of v.points) {
    if (!(p.calls > 0)) {
      fail(
        `tab A: variant "${v.variant}" timed ${p.msPerFrame.toFixed(2)} ms/frame at scale ` +
          `${p.scale} over ${p.calls} draw calls — the probe is timing an empty frame`,
      );
    }
  }
  const shipCalls = cost.variants[0].points.map((p) => p.calls).join(",");
  const mine = v.points.map((p) => p.calls).join(",");
  if (mine !== shipCalls) {
    fail(
      `tab A: variant "${v.variant}" submitted [${mine}] draw calls where "${cost.variants[0].variant}" ` +
        `submitted [${shipCalls}] — the variants are not drawing the same scene, so the time ` +
        `between them is not a shading cost`,
    );
  }
  if (!Number.isFinite(v.msPerMpx) || !Number.isFinite(v.fullFrameMs)) {
    fail(
      `tab A: variant "${v.variant}" reports ${v.msPerMpx} ms/Mpx and ${v.fullFrameMs} ms/frame ` +
        `across [${v.points.map((p) => p.msPerFrame.toFixed(1))}] ms at ` +
        `[${v.points.map((p) => p.megapixels.toFixed(3))}] Mpx — the fit produced no number`,
    );
  }
  // The probe must be timing the GPU, not its own draw-call submission. This
  // is a check on the INSTRUMENT and it is deliberately a ratio: it asserts
  // nothing about how fast this box is, only that a timed frame is mostly
  // spent waiting for pixels that exist. It is here because the first cut of
  // this probe used `gl.finish()` as its barrier, which through Chrome's
  // command buffer returns before the frame is rasterized — every variant
  // then "measured" 0.2 ms and the ratio below was ~1.
  if (!(v.fullFrameMs >= COST_SYNC_MIN_RATIO * v.submitMs)) {
    fail(
      `tab A: variant "${v.variant}" timed ${v.fullFrameMs.toFixed(1)} ms per synced frame against ` +
        `${v.submitMs.toFixed(1)} ms of unsynced draw submission — a ratio of ` +
        `${(v.fullFrameMs / Math.max(v.submitMs, 1e-9)).toFixed(1)}x, under ${COST_SYNC_MIN_RATIO}x. ` +
        `The probe is timing JS, not rendering, so every cost it reports is fiction.`,
    );
  }
}
// The probe's zero point, measured rather than argued: the "ship" variant IS
// the live material, so its frame must equal the reference frame the probe
// took before the sweep started. Anything else and every difference counted
// below is partly the rasterizer talking.
if (byName.ship.vsShipped.changed !== 0) {
  fail(
    `tab A: costProbe's "ship" variant differs from its own reference frame in ` +
      `${byName.ship.vsShipped.changed} pixels (max ${byName.ship.vsShipped.maxDelta}/255) — the ` +
      `probe's zero point is not zero, so the attributions below mean nothing`,
  );
}
if (!(byName.ship.vsFlat.changed > 0)) {
  fail(`tab A: costProbe's reference frames at uSurface 1 and 0 are identical — the field is off`);
}
// Attribution 1 — "nofield" removed the field's INSTRUCTIONS and nothing
// else. Every field term is multiplied by uSurface and 0 x finite is exactly
// 0, so a program compiled with the field's constants must render the frame
// the surface probe's toggle already renders. If it does not, the time
// difference between it and the shipped program is some other edit.
const nofield = byName.nofield;
if (nofield.vsFlat.maxDelta > COST_IDENTITY_MAX_DELTA) {
  fail(
    `tab A: the "nofield" variant differs from the shipped program at uSurface=0 by up to ` +
      `${nofield.vsFlat.maxDelta}/255 luma over ${nofield.vsFlat.changed} pixels (tolerance ` +
      `${COST_IDENTITY_MAX_DELTA}). It was supposed to compile the field OUT, not change it, so ` +
      `whatever it costs is not the field's cost.`,
  );
}
if (!(nofield.vsShipped.changed > 0)) {
  fail(
    `tab A: the "nofield" variant renders the shipped frame pixel for pixel — the field it was ` +
      `built without is not reaching the image at all`,
  );
}
// Attribution 1a — "nograin" removed the fourth octave's INSTRUCTIONS and
// nothing else. Same argument as `nofield`, one uniform down: every grain term
// is multiplied by `uGrain`, so a program compiled without the octave must
// render the frame `uGrain = 0` renders. This is what makes the millisecond
// delta printed below grain's cost rather than an unrelated edit's — and it is
// the whole point of the variant. Grain's first attempt rejected itself on a
// ~9% frame delta with no control in the run, and 9% turned out to be inside
// this box's noise.
const nograin = byName.nograin;
if (nograin.grainOn !== false || byName.ship.grainOn !== true) {
  fail(
    `tab A: the "nograin" variant reports grainOn=${nograin.grainOn} and the shipped one ` +
      `${byName.ship.grainOn} — they are not the pair this check needs`,
  );
}
if (nograin.vsFlatGrain.maxDelta > COST_IDENTITY_MAX_DELTA) {
  fail(
    `tab A: the "nograin" variant differs from the shipped program at uGrain=0 by up to ` +
      `${nograin.vsFlatGrain.maxDelta}/255 luma over ${nograin.vsFlatGrain.changed} pixels (tolerance ` +
      `${COST_IDENTITY_MAX_DELTA}). It was supposed to compile the octave OUT, not change it, so ` +
      `whatever it costs is not grain's cost.`,
  );
}
if (!(nograin.vsShipped.changed > 0)) {
  fail(
    `tab A: the "nograin" variant renders the shipped frame pixel for pixel — the grain octave it ` +
      `was built without is not reaching the cost probe's view at all`,
  );
}
if (!(nograin.noiseSamples < byName.ship.noiseSamples)) {
  fail(
    `tab A: "nograin" reports ${nograin.noiseSamples} noise sample sites against the shipped ` +
      `${byName.ship.noiseSamples} — removing the octave removed no sample`,
  );
}
if (!(nograin.resolvedFragmentChars < byName.ship.resolvedFragmentChars)) {
  fail(
    `tab A: the "nograin" program is ${nograin.resolvedFragmentChars} chars against the shipped ` +
      `${byName.ship.resolvedFragmentChars} — removing the grain octave removed no source`,
  );
}
// Attribution 1b — the micro octave is now SKIPPED where its own footprint
// fade has already retired it, and that is only allowed to be an optimization
// if it is not also an image change. `noskip` is the same program with the
// sample taken unconditionally — materials v0's line — so the two must render
// the same frame. This is the assertion that makes "bit-exact by
// construction" a fact rather than a claim.
const noskip = byName.noskip;
if (noskip.microSkipped !== false || byName.ship.microSkipped !== true) {
  fail(
    `tab A: the "noskip" variant reports microSkipped=${noskip.microSkipped} and the shipped one ` +
      `${byName.ship.microSkipped} — they are not the pair this check needs`,
  );
}
if (noskip.vsShipped.maxDelta > COST_MICRO_SKIP_MAX_DELTA) {
  fail(
    `tab A: skipping the micro octave below its own footprint fade moved ` +
      `${noskip.vsShipped.changed} pixels by up to ${noskip.vsShipped.maxDelta}/255 luma against ` +
      `the same program sampling it unconditionally (tolerance ${COST_MICRO_SKIP_MAX_DELTA}). Every ` +
      `use of that octave is multiplied by a fade that is exactly zero where the branch skips, so ` +
      `an image difference means the branch is not where the fade is.`,
  );
}
// Attribution 2 — "noshadow" removed the shadow term, which can only ADD
// light. A single pixel darker than the shipped frame means the variant
// changed something other than the shadow factor.
const noshadow = byName.noshadow;
if (noshadow.vsShipped.down !== 0) {
  fail(
    `tab A: the "noshadow" variant darkened ${noshadow.vsShipped.down} pixels against the shipped ` +
      `frame. Removing a shadow term can only brighten; anything else means the variant is not the ` +
      `same program minus its shadow.`,
  );
}
if (!(noshadow.vsShipped.up > 0)) {
  fail(`tab A: the "noshadow" variant brightened nothing — the ground is not shadowed in this view`);
}
if (noshadow.depthFetches !== 0 || !(byName.near1.depthFetches < byName.ship.depthFetches)) {
  fail(
    `tab A: cost variants report ship=${byName.ship.depthFetches} near1=${byName.near1.depthFetches} ` +
      `noshadow=${noshadow.depthFetches} depth fetches — the variants are not the programs they claim`,
  );
}
// And the source sizes must order the same way, or the "program size" half of
// the measurement is comparing a program against itself.
if (!(noshadow.resolvedFragmentChars < byName.ship.resolvedFragmentChars)) {
  fail(
    `tab A: the "noshadow" program is ${noshadow.resolvedFragmentChars} chars against the shipped ` +
      `${byName.ship.resolvedFragmentChars} — removing the shadow term removed no source`,
  );
}
// The probe's own timing resolution, measured: `control` is the shipped
// program swept a second time at the far end of the probe, so the difference
// between it and `ship` is what this box can tell apart between two sweeps of
// IDENTICAL work. Nothing smaller than that may be called a cost, here or in
// a commit message — which is the whole reason the control exists.
const ship = byName.ship;
const control = byName.control;
const floorMpx = Math.abs(ship.msPerMpx - control.msPerMpx);
const floorFrame = Math.abs(ship.fullFrameMs - control.fullFrameMs);
if (!Number.isFinite(floorMpx) || !Number.isFinite(floorFrame)) {
  fail(
    `tab A: the cost probe's control swept ${control.msPerMpx} ms/Mpx against the shipped ` +
      `${ship.msPerMpx} — with no finite resolution, every difference it reports is unreadable`,
  );
}
if (control.resolvedFragmentChars !== ship.resolvedFragmentChars) {
  fail(
    `tab A: the cost probe's control measured a ${control.resolvedFragmentChars}-char program ` +
      `against the shipped ${ship.resolvedFragmentChars} — the control is supposed to be the same ` +
      `program measured twice, so it is calibrating nothing`,
  );
}
const pct = (a, b) => (b > 0 ? ((a / b) * 100).toFixed(1) : "n/a");
// Every delta is printed against the floor it has to clear, as a multiple of
// it. Deliberately not a verdict: a reader who is shown "1.2x the floor" knows
// what they have, and one shown only "73% of the frame" does not.
const readAs = (delta) =>
  `${delta < 0 ? "+" : "−"}${Math.abs(delta).toFixed(0)} ms ` +
  `(${pct(Math.abs(delta), ship.fullFrameMs)}% of the frame, ` +
  `${(Math.abs(delta) / Math.max(floorFrame, 1e-9)).toFixed(1)}x the floor)` +
  (delta < 0 ? " — WRONG SIGN: less work measured slower" : "");
const repoAdded = ship.resolvedFragmentChars - fragCost.stockFragmentChars;
console.log(
  `  fragment budget (counted): ${cm.depthFetches} shadow depth fetches/fragment ` +
    `[${cm.levels.map((L) => L.filterTaps)}] under ${DEPTH_FETCH_BUDGET} · terrain program ` +
    `${ship.resolvedFragmentChars} chars of GLSL under ${TERRAIN_FRAGMENT_BUDGET}: ` +
    `${fragCost.stockFragmentChars} (${pct(fragCost.stockFragmentChars, ship.resolvedFragmentChars)}%) ` +
    `is three's unpatched standard material, ${repoAdded} ` +
    `(${pct(repoAdded, ship.resolvedFragmentChars)}%) is this repo's — of which the clipmap ` +
    `shadow GLSL is ${ship.resolvedFragmentChars - noshadow.resolvedFragmentChars} and the field's ` +
    `${fragCost.noiseSamples} sample sites ${ship.resolvedFragmentChars - nofield.resolvedFragmentChars} ` +
    `(of which the grain octave ${ship.resolvedFragmentChars - nograin.resolvedFragmentChars}; ` +
    `the field's shared helper and its consumers stay in every variant)` +
    ` · ${ship.noiseSamples} noise sample sites/fragment, ${nograin.noiseSamples} without grain` +
    ` · micro-octave skip is image-identical (${noskip.vsShipped.changed} px differ, max ` +
    `${noskip.vsShipped.maxDelta}/255)`,
);
console.log(
  `  fragment budget (timed — shared 4-core box, software GL, NOT reference hardware): ` +
    `full-scale frame ` +
    cost.variants.map((v) => `${v.variant} ${v.fullFrameMs.toFixed(0)} ms`).join(" · "),
);
console.log(
  `    resolution: two sweeps of the SAME program differ by ${floorFrame.toFixed(0)} ms ` +
    `(${pct(floorFrame, ship.fullFrameMs)}% of ${ship.fullFrameMs.toFixed(0)}) — nothing under ` +
    `that is a measurement. The fitted slopes are worse conditioned still (` +
    cost.variants.map((v) => v.msPerMpx.toFixed(0)).join("/") +
    ` ms/Mpx, floor ${floorMpx.toFixed(0)} = ${pct(floorMpx, ship.msPerMpx)}%), so the frame ` +
    `times above are what the deltas below are taken from.`,
);
console.log(
  `    shadow term ${readAs(ship.fullFrameMs - noshadow.fullFrameMs)}\n` +
    `    level 0's ${cm.levels[0].filterTaps}-fetch PCF ` +
    `${readAs(ship.fullFrameMs - byName.near1.fullFrameMs)}\n` +
    `    noise field ${readAs(ship.fullFrameMs - nofield.fullFrameMs)}\n` +
    `    grain octave ${readAs(ship.fullFrameMs - nograin.fullFrameMs)}` +
    `  <- NOW.md item 1's question, paired with a control at last`,
);
console.log(
  `    instrument: ${(ship.fullFrameMs / Math.max(ship.submitMs, 1e-9)).toFixed(0)}x of a timed ` +
    `frame is GPU rather than JS submission (${ship.submitMs.toFixed(1)} ms unsynced), floor ` +
    `${COST_SYNC_MIN_RATIO}x`,
);
console.log(
  `    program compile, first render through each: ` +
    cost.variants.map((v) => `${v.variant} ${v.firstFrameMs.toFixed(0)} ms`).join(" · ") +
    ` (against a warm frame of ${ship.fullFrameMs.toFixed(0)} ms; "ship" and "control" were ` +
    `already compiled, so they are this measurement's own floor too)`,
);

// Play: A walks forward, B walks backward — opposite headings off the shared
// point. The terrain worker only builds once a player moves, so the window
// where bug 2 fires is AFTER the first snapshot.
await A.page.keyboard.down("KeyW");
await B.page.keyboard.down("KeyS");
await A.page.waitForTimeout(PLAY_MS);
await A.page.keyboard.up("KeyW");
await B.page.keyboard.up("KeyS");

const finalA = await remoteOf(A, B.playerId);
const finalB = await remoteOf(B, A.playerId);
const dbgA = await A.page.evaluate(() => globalThis.__gatesDebug);
const dbgB = await B.page.evaluate(() => globalThis.__gatesDebug);

// Assertion 3 — nothing threw on either page. Catches bug 2, which is
// invisible in a frame.
for (const tab of [A, B]) {
  if (tab.errors.length) {
    fail(
      `${tab.label}: ${tab.errors.length} page error(s) while playing — the client is throwing:\n` +
        tab.errors.slice(0, 8).map((e) => `    ${e}`).join("\n"),
    );
  }
}
// Assertion 4 — the wire actually moved on both sessions.
if (!(dbgA.snapshots > 0)) fail(`tab A: no snapshots received`);
if (!(dbgB.snapshots > 0)) fail(`tab B: no snapshots received`);
// Assertion 5 — CLAUDE.md's trap: an oversize browser datagram silently sends
// nothing. Zero is the only acceptable count.
if (dbgA.oversize !== 0) fail(`tab A: ${dbgA.oversize} oversize datagram(s) — clamp against the live maxDatagramSize`);
if (dbgB.oversize !== 0) fail(`tab B: ${dbgB.oversize} oversize datagram(s) — clamp against the live maxDatagramSize`);
// Assertion 6 — the M0 exit condition: each tab watched the OTHER walk. A
// frozen remote (dead interp, stalled input pump, throttled RAF) fails here.
const planar = (a, b) => Math.hypot(b[1] - a[1], b[3] - a[3]);
const moveA = finalA ? planar(seenA, finalA) : 0; // B's walk, seen from A
const moveB = finalB ? planar(seenB, finalB) : 0; // A's walk, seen from B
if (!finalA) fail(`tab A: player ${B.playerId} left AOI mid-walk`);
if (!finalB) fail(`tab B: player ${A.playerId} left AOI mid-walk`);
if (moveA < MOVE_MIN_M) fail(`tab A watched player ${B.playerId} move ${moveA.toFixed(2)} m — a live remote walks ≥ ${MOVE_MIN_M} m`);
if (moveB < MOVE_MIN_M) fail(`tab B watched player ${A.playerId} move ${moveB.toFixed(2)} m — a live remote walks ≥ ${MOVE_MIN_M} m`);

console.log(`  snapshots A ${dbgA.snapshots} B ${dbgB.snapshots} · oversize 0 · page errors 0`);
console.log(`  mutual movement: A saw B walk ${moveA.toFixed(1)} m, B saw A walk ${moveB.toFixed(1)} m`);

// --- chat, part 2: silent out of earshot ------------------------------------
// Assertion 7 — the 20 m local radius is a real edge, not a constant nobody
// applies. The radius itself is pinned to the quantum natively in
// server/tests/chat_wire.rs; what only a browser can prove is that the keys,
// the encoder, the stream, the fan-out and the log are one chain that also
// knows how to stay quiet.
//
// Walk them apart first. Bounded rounds, and a loud failure if they never
// separate — never a skipped assertion.
let apart = await apartNow();
for (let round = 0; round < 5 && apart < CHAT_APART_M; round++) {
  await A.page.keyboard.down("KeyW");
  await B.page.keyboard.down("KeyS");
  await A.page.waitForTimeout(3000);
  await A.page.keyboard.up("KeyW");
  await B.page.keyboard.up("KeyS");
  await A.page.waitForTimeout(400);
  apart = await apartNow();
}
if (apart < CHAT_APART_M) {
  fail(
    `tabs only walked ${apart.toFixed(1)} m apart in 5 rounds — the local-radius ` +
      `assertion needs ≥ ${CHAT_APART_M} m of separation to mean anything`,
  );
}

const FAR_LOCAL = "cannot hear this";
const GLOBAL_LINE = "wipe is at six";
await say(A, FAR_LOCAL);
await say(A, `/g ${GLOBAL_LINE}`);
const heardGlobalB = await waitForLine(B, GLOBAL_LINE);
if (!heardGlobalB) fail(`tab B never heard A's global line at ${apart.toFixed(1)} m`);
if (heardGlobalB.includes(FAR_LOCAL)) {
  fail(
    `tab B heard a LOCAL line from ${apart.toFixed(1)} m away — the 20 m radius ` +
      `is not being applied`,
  );
}
for (const tab of [A, B]) {
  if (tab.errors.length) {
    fail(
      `${tab.label}: ${tab.errors.length} page error(s) during chat:\n` +
        tab.errors.slice(0, 8).map((e) => `    ${e}`).join("\n"),
    );
  }
}
console.log(`  chat: local silent at ${apart.toFixed(1)} m, global heard through`);

// --- the dev view hook ------------------------------------------------------
// Assertion 8 — on a DEV shard the hook exists and actually aims. Headless
// pointer lock yields no movementX (gates-loop/art/probe-pointerlock.mjs), so
// this is the only path that can point the camera, and a hook that silently
// did nothing would leave the capture harness shooting spawn yaw every pass
// while reporting six distinct vantages.
if (!dbgA.dev) fail(`tab A: dev shard welcomed with dev=false — the client cannot install its dev hooks`);
// typeof INSIDE the page: page.evaluate cannot serialize a function, so
// returning the hook itself would read undefined no matter what is there.
const devHook = await A.page.evaluate(() => typeof globalThis.__gatesDebug.setView);
if (devHook !== "function") {
  fail(`tab A: __gatesDebug.setView is ${devHook} on a dev shard (dev_spawn is set) — art/capture.mjs is blocked`);
}
const aimed = await A.page.evaluate(
  ([y, p]) => globalThis.__gatesDebug.setView(y, p),
  [AIM_YAW, AIM_PITCH],
);
if (aimed !== true) fail(`tab A: setView(${AIM_YAW}, ${AIM_PITCH}) returned ${aimed}`);
// It has to survive into the next published snapshot, not just return true —
// and the baseline the aimed walk is measured from has to be a player who is
// no longer WALKING. No key is held here; anything still moving at walking
// pace is the chat section's walk draining out of the predictor, and it moves
// +Z, which is exactly the axis the assertion below calls a failure. See
// AIM_REST_* above for why this is a speed and not a displacement.
const restProbe = async () => {
  const r = await A.page.evaluate(() => ({
    own: globalThis.__gatesDebug.own,
    snapshots: globalThis.__gatesDebug.snapshots,
    view: globalThis.__gatesDebug.view,
  }));
  // Stamped on arrival: the interval between two FRESH publishes is what the
  // speed is over, and on a starved main thread that is several times the poll
  // period. Dividing by the poll period instead would report a drain rate ~4x
  // too high — an instrument that lies in the direction of failing the gate.
  return { ...r, at: Date.now() };
};
let rest = await restProbe();
let atRest = null;
let clearRun = 0;
let fresh = 0;
let lastSpeed = null;
let lastGapMs = null;
const restDeadline = Date.now() + AIM_REST_DEADLINE_MS;
while (!atRest && fresh < AIM_REST_PUBLISHES && Date.now() < restDeadline) {
  await A.page.waitForTimeout(AIM_REST_POLL_MS);
  const now = await restProbe();
  // A publish the client never refreshed says nothing about whether it moved:
  // two reads of one starved publish would read as "did not move", which is
  // the misreading that has already cost this gate two passes.
  if (!(now.snapshots > rest.snapshots)) continue;
  fresh++;
  lastGapMs = now.at - rest.at;
  lastSpeed = Math.hypot(now.own[0] - rest.own[0], now.own[2] - rest.own[2]) / (lastGapMs / 1000);
  rest = now;
  clearRun = lastSpeed <= AIM_REST_SPEED_MPS ? clearRun + 1 : 0;
  if (clearRun >= AIM_REST_CLEAR_RUNS) atRest = now;
}
if (!atRest) {
  fail(
    `tab A: with no key held the player never stopped walking — ` +
      (lastSpeed === null
        ? `no fresh publish in ${AIM_REST_DEADLINE_MS} ms, so nothing was ever measured ` +
          `(the client has stopped publishing, which is its own failure)`
        : `last ${lastSpeed.toFixed(2)} m/s over ${lastGapMs} ms, ${clearRun} of ` +
          `${AIM_REST_CLEAR_RUNS} consecutive intervals under ${AIM_REST_SPEED_MPS} m/s, ` +
          `${fresh} fresh publish(es) in ${AIM_REST_DEADLINE_MS} ms`) +
      `. The aimed walk below can only measure the aim if the previous walk has drained.`,
  );
}
const view = atRest.view;
if (Math.abs(view[0] - AIM_YAW) > AIM_EPS || Math.abs(view[1] - AIM_PITCH) > AIM_EPS) {
  fail(`tab A: aimed at [${AIM_YAW}, ${AIM_PITCH}] but the camera reads [${view}]`);
}
// And the aim must reach the SIM, not just the camera: yaw pi/2 faces +X, so a
// held W walks east. A hook that moved the render camera alone would pass the
// readback above and still frame a player walking sideways out of shot.
const beforeAim = atRest.own.slice();
await A.page.keyboard.down("KeyW");
await A.page.waitForTimeout(PLAY_MS);
await A.page.keyboard.up("KeyW");
const afterAim = await A.page.evaluate(() => globalThis.__gatesDebug.own);
const dx = afterAim[0] - beforeAim[0];
const dz = afterAim[2] - beforeAim[2];
if (dx < MOVE_MIN_M || Math.abs(dz) > dx) {
  fail(
    `tab A: after setView(yaw pi/2) a held W moved [${dx.toFixed(2)}, ${dz.toFixed(2)}] m — ` +
      `yaw pi/2 faces +X, so the walk must be east-dominant and ≥ ${MOVE_MIN_M} m. ` +
      `The hook is not reaching the input the sim runs on.`,
  );
}
// The residual speed rides along on the PASSING path too: it is the margin the
// AIM_REST_SPEED_MPS bar is set against, and the only way the next slice learns
// that the gap between "residual" and "walking" has closed is by watching it.
console.log(
  `  dev hook: aimed to [${view.map((v) => v.toFixed(2))}], walked +X ${dx.toFixed(1)} m ` +
    `(dz ${dz.toFixed(1)}), from rest after ${fresh} fresh publish(es) — residual ` +
    `${lastSpeed.toFixed(2)} m/s over ${lastGapMs} ms, bar ${AIM_REST_SPEED_MPS}, walk 3`,
);

// Assertion 11 — DESIGN §9's draw budget, which had no gate until the shadow
// pass gave it teeth: a second full pass over every caster is exactly the kind
// of change that eats it. Counted and structural, so it means the same here as
// on the reference box; timing still does not. Read HERE, at the end, rather
// than off the same snapshot as assertion 9: by now both tabs have walked, so
// the near ring has streamed and torn down and the scene is at its fullest.
//
// Asserted on the PEAK across every frame since boot, not on whichever frame
// this line happened to catch. The clipmap made that distinction real: a
// cached coarse level draws on some frames and not others, so a last-frame
// count can miss the expensive one entirely. The budget is what the GPU was
// ever asked to draw.
const budget = await A.page.evaluate(() => globalThis.__gatesDebug.lighting);
if (!(budget.calls > 0)) fail(`tab A: renderer reported ${budget.calls} draw calls — the stats are not being read`);
if (!(budget.peakCalls >= budget.calls)) {
  fail(`tab A: peak ${budget.peakCalls} draw calls is below this frame's ${budget.calls} — the peak is not being tracked`);
}
if (budget.peakCalls >= DRAW_CALL_BUDGET) {
  fail(
    `tab A: peak ${budget.peakCalls} draw calls (main + every shadow level, worst frame; ` +
      `${budget.calls} this frame) — DESIGN §9 budgets < ${DRAW_CALL_BUDGET}`,
  );
}
if (budget.peakTriangles >= TRIANGLE_BUDGET) {
  fail(
    `tab A: peak ${budget.peakTriangles} triangles (main + every shadow level, worst frame; ` +
      `${budget.triangles} this frame) — DESIGN §9 budgets < ${TRIANGLE_BUDGET}`,
  );
}
console.log(
  `  budget: ${budget.calls} draw calls / ${budget.triangles} triangles this frame, ` +
    `peak ${budget.peakCalls} / ${budget.peakTriangles} (all passes)`,
);

// --- hand the box back before the last tab boots ----------------------------
// A and B are DONE: every assertion either tab can make has been made, and the
// dev-gate check below joins a different shard with a different session and
// reads nothing from them. What they still do is rasterize — in software, on
// four cores shared with nineteen other services — and that is what made this
// wall red.
//
// The join time is monotonic in the number of live tabs, measured across this
// gate's own history: 0.4 s alone, 34-36 s beside one, 55-61 s beside two. The
// third reading is not a margin, it is a coin flip against a 60 s window, and
// on 2026-08-01 16:26 it came up tails: `inWorld=true, snapshots=1` at 61.6 s —
// the client HAD joined, one and a half seconds after the gate stopped waiting.
// Slice 21 merged knowing it ("~55 s of a 60 s budget — measured, unexplained")
// and the very next health run went red.
//
// So the fix is to stop asking a third tab to boot while two others hold the
// cores, NOT to widen JOIN_TIMEOUT_MS — which NOW.md item 2 rules out by name,
// and which would have bought one slice of quiet before landing back here.
// Nothing asserted changes and no wait gets longer; the public tab simply gets
// the same empty box tab A gets.
for (const tab of [A, B]) {
  // One last error sweep before the evidence goes away with the context. The
  // aimed-walk section above had no page-error check of its own, so this is a
  // window that was never looked at: a throw in setView, in the input path or
  // in the render loop during those seconds went unreported.
  if (tab.errors.length) {
    fail(
      `${tab.label}: ${tab.errors.length} page error(s) during the dev-hook walk:\n` +
        tab.errors.slice(0, 8).map((e) => `    ${e}`).join("\n"),
    );
  }
  await tab.close();
}
// And pin it, so a later slice cannot quietly put the contention back: the
// public tab's join is only a fair reading of "can a client reach the world"
// if it is the only client rendering.
if (liveTabs.size !== 0) {
  fail(
    `public tab is about to boot beside ${liveTabs.size} live tab(s) (${[...liveTabs].join(", ")}) — ` +
      `that is the configuration that reddened this wall on 2026-08-01`,
  );
}
console.log(`  handed the box back: ${A.label} and ${B.label} closed, 0 tabs live`);

// Assertion 12 — the gate itself. A shard with no dev override is exactly a
// public shard's config, and its client must have no dev surface at all.
const P = await join("public tab", PUBLIC_WIRE_PORT, publicCertHash);
if (P.dbg.dev !== false) fail(`public tab: shard without dev_spawn welcomed with dev=${P.dbg.dev}`);
const publicSetView = await P.page.evaluate(() => typeof globalThis.__gatesDebug.setView);
if (publicSetView !== "undefined") {
  fail(`public tab: __gatesDebug.setView is ${publicSetView} on a shard with no dev override — a dev affordance shipped to a public shard`);
}
for (const hook of [
  "shadowProbe",
  "farShadowProbe",
  "surfaceProbe",
  "splatCensus",
  "horizonProbe",
  // Four renders and four full readbacks per yaw — sixteen frames handed to
  // anyone who asks, on a shard where nobody may ask.
  "daylightProbe",
  "grainProbe",
  // The cost probe is not the only dev affordance that BUILDS something — its
  // variants are five extra terrain programs and the projection probe compiles
  // a sixth. A public tab must have neither the hooks nor the compiles behind
  // them.
  "costProbe",
  "projectionProbe",
  // …and this one is not a probe at all: it walks every near vertex in a 150 m
  // radius, which is a frame's worth of work handed to anyone who asks.
  "steepestFace",
  // Lighting v1's two. `tonalProbe` renders 2N frames and reads the whole
  // drawing buffer back per view; `sunProbe` re-aims the camera. Both are
  // gate instruments and neither belongs on a public shard.
  "tonalProbe",
  "sunProbe",
]) {
  const t = await P.page.evaluate((h) => typeof globalThis.__gatesDebug[h], hook);
  if (t !== "undefined") {
    fail(`public tab: __gatesDebug.${hook} is ${t} on a shard with no dev override — a dev affordance shipped to a public shard`);
  }
}
if (P.errors.length) {
  fail(`public tab: ${P.errors.length} page error(s):\n` + P.errors.slice(0, 8).map((e) => `    ${e}`).join("\n"));
}
console.log(`  dev gate: public shard welcomes dev=false, client installs no setView`);

console.log("browser smoke: all checks passed");
cleanup();
process.exit(0);
