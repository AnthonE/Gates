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
// Same failure from the other side, counted rather than sampled: the seven
// scatter pools are frustumCulled=false, so a rig where the world casts
// submits at least them to the shadow pass. That mutation submitted 2 (the
// avatar's two meshes); the intact rig submits 25.
const SHADOW_PASS_MIN_CALLS = Number(process.env.BROWSER_SMOKE_SHADOW_MIN_CALLS || 8);
const SHADOW_MIN_MAP_PX = 1024;
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
// level of amplitude. p05 is deliberately NOT walled: the dark tail of these
// masks is the shaded side of the prop, and the light rig owns it (§open,
// "prop albedo v1"). Walling a number this pass cannot move would be a gate
// that fails for a reason its owner cannot act on.
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
// `#include`s expanded: 81,520 today, of which 73,375 is three's stock
// MeshStandardMaterial as it was handed over and 8,145 (10.0%) is everything
// this repo added to the ground — the grain octave being 638 of it. The cap is
// ~18% over, which survives a three minor bump and still catches a program
// that doubled.
const TERRAIN_FRAGMENT_BUDGET = Number(process.env.BROWSER_SMOKE_FRAG_CHARS || 96000);
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

// --- vitals: the bar the shard states at the door --------------------------
// Assertion 2a — a fresh spawn's health reaches the DOM, and the number it
// shows is the one in `content/balance.toml`, read here rather than typed:
// the whole chain content → bake → sim → wire v11 → wasm → HUD is what this
// asserts, and a gate that hardcoded 100 would keep passing after a balance
// pass moved it. Observable state, polled — never an elapsed-ms bar.
{
  const balance = fs.readFileSync(path.join(root, "content/balance.toml"), "utf8");
  const wantHp = Number(/^player_hp\s*=\s*(\d+)/m.exec(balance)?.[1]);
  if (!Number.isFinite(wantHp) || wantHp <= 0) {
    fail("content/balance.toml states no player_hp — the vitals assertion cannot run");
  }
  const vitals = (tab) =>
    tab.page.evaluate(() => {
      const el = document.getElementById("vitals");
      return {
        shown: !!el && el.style.display === "block",
        text: el ? el.textContent.trim() : "",
      };
    });
  for (const tab of [A, B]) {
    let v = await vitals(tab);
    for (let i = 0; i < 40 && !v.shown; i++) {
      await tab.page.waitForTimeout(250);
      v = await vitals(tab);
    }
    if (!v.shown) {
      fail(`tab ${tab.playerId}: the vitals stack never appeared — no health reached the HUD`);
    }
    if (v.text !== String(wantHp)) {
      fail(
        `tab ${tab.playerId}: vitals read "${v.text}", content says ${wantHp} — ` +
          "the number the shard plays is not the number the data declares",
      );
    }
  }
  console.log(`  vitals: both tabs read ${wantHp} hp, straight from content/balance.toml`);
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
// was: see SHADOW_MIN_FRACTION_PER_YAW. Shadows in every direction can only
// come from the world casting, not from whatever happens to be next to you.
const thin = probe.samples.filter((s) => s.fraction < SHADOW_MIN_FRACTION_PER_YAW);
if (thin.length) {
  fail(
    `tab A: ${thin.length} of ${probe.samples.length} probed directions have almost no ` +
      `shadow in them (floor ${(SHADOW_MIN_FRACTION_PER_YAW * 100).toFixed(1)}% per yaw). ` +
      `Something near the camera is casting and the world is not.\n` +
      probe.samples
        .map((s) => `    yaw ${s.yaw.toFixed(2)}: ${(s.fraction * 100).toFixed(2)}% (${s.darkened} px)`)
        .join("\n"),
  );
}

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
// So the two legs that are TRUE today are walls, and the ship leg is reported
// and not walled. A ceiling set where the defect fits is worse than none:
//   `nobump`      gmH identically zero -> no derivative in the image at all.
//   `nograinbump` grain's bump alone removed -> the structural octaves' bump
//                 must not put quad-locked energy in the frame either.
// Both must stay under the ceiling the ship leg will be held to once the law
// lands, so the day it lands the wall is already calibrated.
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
  // The ship leg: reported, loudly, and not walled — see the block comment.
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
