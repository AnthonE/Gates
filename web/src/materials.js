// Materials v0 — the surface read (DECISIONS.md §open, "materials v0").
//
// Lighting v0 gave the world shape; it had no *surface*. Everything was
// MeshLambertMaterial at a flat colour: no roughness anywhere, no detail
// normal, and the terrain was a per-vertex biome colour with a per-vertex
// tint jitter — so a hillside was one flat wash and a beach met a meadow on
// a smooth ramp.
//
// This is the splat material TERRAIN.md §4 has always specified ("one splat
// shader, four texture sets, blended in-shader from height, slope, and a
// noise channel recomputed in GLSL — no splatmap textures, no extra
// bandwidth"), built without textures at all: four authored PBR identities
// (sand · grass · forest litter · rock) blended by weights the worker
// derives from the SIM's own (height, moisture, slope), and broken up,
// mottled, roughened and bump-lit by ONE shared noise field evaluated three
// times per pixel.
//
// The graph order is `threejs-procedural-materials`' (MIT, Scott Sun —
// credited in CLAUDE.md; guidance only, no code from the pack ships here):
//
//   stable coordinates (world xz)
//     → structural fields (three octaves of one value-noise field)
//     → material identity weights (the splat attribute, broken up)
//     → causal modifiers (wetness at the waterline, snow on high rock,
//       darkening on cliff faces — each one drives colour AND roughness,
//       never a channel on its own)
//     → filtered microstructure (both bump octaves fade out by pixel
//       footprint, so no normal survives below a pixel)
//     → PBR channels (albedo, roughness, normal)
//
// The skill's failure list is the thing to keep passing: every channel here
// reads the same three noise samples, roughness is a per-identity property
// rather than a scalar afterthought, the high-frequency octave is footprint-
// faded, and the perturbed normal feeds a specular-AA term instead of a post
// pass hiding the shimmer.
//
// `uSurface` is the skill's required channel debug mode and the gate's
// handle: at 1 the field paints, at 0 it does not and the same frame renders
// with flat identities. ci/browser_smoke.mjs renders both and counts the
// pixels that moved — the only proof a material reaches the image.
//
// --- second pass: grain, the octave that only exists at arm's length --------
//
// What v0 shipped had no TEXTURE. Its finest octave is 1.7 m across, so from
// standing height the ground is smooth-shaded mottling and at arm's length it
// is nothing at all — a hillside reads, a footprint's worth of sand does not.
//
// So a fourth octave, at a wavelength the IDENTITY chooses (4 cm of sand,
// 12 cm of grass clump), with a per-identity shape (a soft stipple for sand,
// a ridged crystalline speckle for rock) and a per-identity contrast. One
// sample, not four: the scale, the shape and the contrast are each a dot
// product against the same splat weights, so they morph continuously across a
// biome boundary instead of cross-fading two grains. Like every octave above
// it, it is sampled on world XZ.
//
// `uGrain` is its own probe handle, separate from `uSurface`, because "the
// field paints" and "the surface has grain" fail differently and a gate that
// cannot separate them cannot name what broke. And unlike `uSurface` it has a
// COMPILED partner: `nograin` is the shipped program with the octave's
// instructions gone rather than weighted to zero, so `costProbe` can price the
// octave against its own run's floor. That pairing is the whole reason this
// landed on the second attempt — the first rejected grain on a ~9% frame delta
// that nothing had calibrated, and 9% turned out to be inside this box's noise.
//
// --- third pass: the projection (DECISIONS.md §open, "materials v1") --------
//
// The two passes above sample every octave on world XZ, and at 1.7 m that is
// a smear nobody reads as wrong. At 4 cm it is a hillside combed downhill: a
// world-XZ field on a face of upness `u` is stretched by `1/u` ALONG the
// slope and left alone across it, so a 47° face wears a grain 1.5× longer
// downhill than it is wide. Every other octave is coarse enough to get away
// with it. Grain is not, and grain is the only octave a player ever sees at
// arm's length — which is the whole reason it exists.
//
// So the grain — and ONLY the grain — is sampled triplanar. The shape is not
// stock, and both halves of it are load-bearing:
//
//   1. **The ridge fold is applied per plane, BEFORE the blend.** Folding
//      after the blend folds a value that is already three fields averaged,
//      and the average of three ridged fields is not the ridge of an average:
//      the crystalline speckle rock is built from turns into a soft one, and
//      the octave's whole per-identity `ridge` knob stops meaning anything on
//      any face that is not axis-aligned.
//   2. **The blend's deviation is restored by `1/|w|`.** Blending three
//      INDEPENDENT fields with convex weights `w` shrinks the deviation to
//      `|w|` of one field's — 1.0 on an axis, 0.577 at the 45-45-45 corner —
//      so a stock triplanar blend does not just reproject the grain, it fades
//      it out on exactly the faces it was reprojected for.
//
// Without both, a 47° face measured ×0.56 the contrast of the same face on
// world XZ: the fix would have cost more contrast than the smear it removed.
// With both, measured by the browser gate on the 46.6° face this seed's spawn
// offers, tilting the ground coarsens the world-XZ grain by ×2.02 and this one
// by ×1.40 — ×1.44 of the ×1.456 stretch taken back — at ×1.04 amplitude.
//
// On level ground the weights are `(0,1,0)` EXACTLY — a heightfield's flat
// vertices carry an exactly-vertical normal and the interpolant of three of
// them is exactly vertical — so `|w|` is exactly 1, the x and z taps are
// multiplied by exactly 0, and the result is bit-for-bit the world-XZ tap
// this replaces. The change is confined to slopes by construction.
//
// `flatgrain` is the compiled partner that makes all of that a gate rather
// than a paragraph: the shipped program with materials v1's world-XZ tap in
// place of the triplanar one, nothing else touched. `projectionProbe` renders
// both programs at two cameras — one square-on to a slope, one square-on to
// level ground — and measures how much each program's grain COARSENS between
// them. Everything that is not the projection is in both programs' numerator
// and denominator alike, which is what makes the number quotable on a box that
// cannot be trusted with a millisecond.
//
// The price is counted, per `NOW.md`'s instruction not to re-run the cost
// question in milliseconds on this box: grain's noise sample sites go 1 → 3,
// so the ground's total goes 4 → 6 per fragment, and the program grows by the
// triplanar helper. Both are asserted against a budget by the browser gate.
//
// --- fourth pass: the tint, per class (DECISIONS.md §open, "materials v3") ---
//
// Everything above is ONE COLOUR PER IDENTITY multiplied by a scalar. Read the
// channels: `MOTTLE` multiplies albedo, `ROUGH_VAR` offsets roughness, the
// grain's swing multiplies albedo again. A multiply by a scalar cannot move a
// hue — `k·(r,g,b)` has the same chromaticity as `(r,g,b)` — so every square
// metre of grass in this world was, exactly, one green at a range of
// brightnesses, and so was every square metre of sand, litter and rock. That
// is what the visual judge's blind reader named on five of six frames in its
// own words: "a single untextured green sheet", "flat-shaded", "hue-only". The
// pass before this one found the two arithmetic defects underneath that
// sentence (an aliasing bump, a gradient reconstructed on the triangle) and
// fixed both. This is the sentence itself.
//
// So each identity gets a second axis: a signed chromatic DEVIATION it swings
// along, and a tiling octave in the 0.5–1 m band to swing it with.
//
//   sand    pale bleached quartz  <-> darker, browner mineral
//   grass   sun-dried straw-green <-> lush blue-green
//   litter  needle-brown duff     <-> moss
//   rock    warm pale feldspar    <-> dark neutral mica
//
// Four properties are load-bearing, and each is a thing that could have been
// done the easy way instead:
//
//   1. **The deviation is SIGNED and added, not a second colour lerped to.**
//      `albedo += dev · t` with `t` in [−1, 1] leaves the identity's mean
//      colour exactly where the palette put it — this slice changes the
//      VARIANCE of the ground and not its average, so it cannot be an
//      exposure slip or an art-direction change wearing a texture's name.
//      The gate asserts that directly (mean luma and mean chromaticity hold
//      while the spread rises), which a base→alt lerp could not have offered.
//   2. **`dev` is not parallel to `color`.** A deviation proportional to the
//      identity's own colour is a brightness multiply with extra steps, and
//      would score 1.00 on the chromatic ratio the gate measures. All four
//      sit 16°–45° off their own colour, and the four hue axes are not one
//      axis — both asserted, so "hue-only" cannot come back through a
//      well-meaning re-balance.
//   3. **The tile scale is the IDENTITY's**, like grain's — one octave, one
//      sample, four surfaces, morphing continuously across a biome boundary
//      because the scale is a dot product against the same splat weights.
//      It fills the gap the octave ladder actually had: macro 48 m, meso
//      9.5 m, micro 1.7 m, then nothing until grain at 4–12 cm.
//   4. **Nothing biases it.** The report also asked for "a low-frequency
//      macro-variation octave to break tiling", and this pass built that term
//      off macro, measured it, rebuilt it off meso, measured it again, and
//      deleted it — both times it was a colour cast over the whole ground and
//      not a variation, because an octave wider than the frame is a constant
//      inside it. The premise came with the words: value noise on world XZ
//      does not repeat, so there is no tiling to break. The measurements and
//      the reasoning are kept in full below, under "what is NOT here".
//
// `uTint` is its probe handle, on the pattern `uSurface` and `uGrain` set. It
// gets no compiled partner, and that is a decision rather than an omission:
// `nograin` and `flatgrain` exist to price an octave in milliseconds and to
// compare two projections that no uniform can select between, and neither
// question is asked here — `NOW.md` says not to re-run the cost question on
// this box, and "did the albedo become chromatic" is a property of one
// program's own two states. What the uniform cannot express is folded into
// the FIELD instead: `nofield` takes the tile octave out with the other three,
// because the field is one field.
//
// --- fifth pass: the base's projection (DECISIONS.md §open, "materials v4") --
//
// The third pass above found that a world-XZ field combs downhill, fixed the
// grain octave, and wrote down why. The base maps then landed on world XZ —
// at the finest tile in the ladder, carrying a 1K photograph rather than a
// noise field. Everything the third pass says about grain is true of them and
// worse: a face of upness `u` wears the photograph stretched `1/u` along its
// fall line, which is x2.8 at this seed's steepest faces and unbounded as a
// face goes vertical. It reaches the image as a comb of long streaks with a
// filtering lattice over it — the sampler's anisotropy budget spent on a
// distortion the projection put there — and it is what the operator's frames
// of the cliffs are frames OF.
//
// So the base samples the surface too, biplanar rather than triplanar (two
// fetched maps per wall tap, not three, and the wall skipped whole below 45°).
// `BASE_WALL_ON` carries the whole derivation, including why the ramp's foot
// is a forced value and not a taste.
//
// Two things this pass fixed that are the same bug wearing other clothes, both
// found by reading this file against `TERRAIN.md` rather than by looking:
//
//   · **The base retired on the horizontal footprint.** `TERRAIN.md` states
//     the law for grain — "its fade reads the world footprint, not the
//     horizontal one, because a pixel on a steep face barely moves in XZ while
//     covering metres of surface" — and the octave that landed after it did
//     not follow it. An XZ footprint reports a distant cliff as near ground,
//     so twelve fetches stayed alive out there delivering detail below the
//     pixel. One line, and it is grain's own line.
//   · **Snow REPLACED the albedo.** Every other causal modifier multiplies, so
//     the photograph survives it; `mix(albedo, SNOW_COLOR, 1)` is a constant,
//     and above 80 m that is whole hillsides. See the modifier block.
//
// What this pass did NOT do, so the next one is not misled: the base map's
// RELIEF still dies as a face approaches vertical. `gmBaseSlope` is a world-XZ
// heightfield gradient by construction and the shading normal scales it by
// `gmNy`, so a cliff gets the photograph's colour and almost none of its
// shape. That is a property of the bump's whole formulation — the one this
// file has already spent two passes getting continuous across a triangle edge
// — and not of the projection. It wants its own pass and `ci/bump_basis.mjs`
// re-derived under it, not a constant changed underneath it.

import * as THREE from "three";
import {
  clipmapFetches,
  installClipmapShadows,
  resolvedGlslChars,
} from "./shadows.js";
import { GROUND_LAYERS, groundTextures } from "./textures.js";

// --- the four identities (TERRAIN.md §4's four sets) ------------------------
// Colours are the retired vertex palette's, unchanged, so this slice changes
// the SURFACE and not the art direction: sand was C_BEACH, grass C_MEADOW,
// litter C_FOREST, and rock sits between C_HIGHLAND and C_CLIFF (whose
// midpoint is 0.505/0.495/0.49 — rock is rounded off it). They are
// working-space (linear) triples, which is what the vertex path fed too.
// `bump` is a unitless STRENGTH, not a height: the two bump octaves are
// 5.6× apart in wavelength, so one amplitude in metres cannot serve both
// (0.06 m over a 9.5 m wave is a slope of 0.006 — a surface nobody can see).
// The metres live per octave, below, and this scales them.
//
// `grain` is the second pass's per-identity trio, and the three numbers are
// what make one shared octave read as four different surfaces:
//   scale    cycles per metre of the identity's own grain — 25 /m is a 4 cm
//            sand clump, 8.3 /m a 12 cm tuft of grass.
//   contrast how far it swings albedo, ±, at full strength.
//   ridge    0 is the raw smooth field (a soft stipple), 1 is `1-|2g-1|`
//            (a ridged, crystalline speckle). Rock is nearly all ridge;
//            sand is nearly none.
//
// `tint` is the fourth pass's pair, and it is the only thing in this table
// that can move a HUE. `color` stays the identity's MEAN — the deviation is
// signed, so the palette above is unchanged and what this adds is variance:
//   dev    the chromatic axis the identity swings along, ±, in the same
//          working-space (linear) triple `color` is written in. Two properties
//          it has by construction, both asserted:
//            · **not parallel to `color`** — a parallel deviation is a
//              brightness multiply, which is what every octave above already
//              was. Measured cosines: −0.036 / 0.152 / −0.118 / −0.047.
//            · **luminance-neutral** — `0.2126·r + 0.7152·g + 0.0722·b` is
//              zero to within 0.3% of the deviation's own length, so the
//              swing is pure hue at constant brightness. That is the whole
//              division of labour of this material: three scalar octaves and
//              a grain already do VALUE, and nothing did HUE, which is the
//              defect stated as arithmetic. It is also load-bearing for a
//              gate this pass had to earn — see `TINT_LUMA_NEUTRAL`.
//   scale  cycles per metre of the identity's own tiling — 1.7 /m is a 59 cm
//          sand ripple, 1.0 /m a metre of forest floor. All four land in the
//          0.5–1 m band the visual report asked for, and the whole set sits
//          between the micro octave (0.588 /m) and the finest grain (8.3 /m),
//          which is the hole in the ladder this octave exists to fill.
export const IDENTITIES = [
  {
    name: "sand",
    color: [0.78, 0.71, 0.52],
    roughness: 0.92,
    bump: 0.35,
    grain: { scale: 25.0, contrast: 0.1, ridge: 0.15 },
    // Warm damp/mineral gold (+) to cool bleached grey-quartz (−) — the
    // waterline read `WET_DARKEN` does by value, done by hue, so a dry dune
    // and a tide line differ in colour and not only in brightness.
    // + [0.85, 0.699, 0.42] · − [0.71, 0.721, 0.62].
    tint: { dev: [0.07, -0.011, -0.1], scale: 1.7 },
  },
  {
    name: "grass",
    color: [0.35, 0.49, 0.23],
    roughness: 0.82,
    bump: 0.6,
    grain: { scale: 8.3, contrast: 0.2, ridge: 0.55 },
    // Sun-dried straw-olive (+) to lush blue-green (−) — the largest swing of
    // the four, because grass is the identity that owns 99% of the near ring
    // at this spawn and the one the blind reader called a single green sheet.
    // + [0.47, 0.46, 0.17] · − [0.23, 0.52, 0.29]: two greens a viewer would
    // name differently, at the same brightness.
    tint: { dev: [0.12, -0.03, -0.06], scale: 1.1 },
  },
  {
    name: "litter",
    color: [0.15, 0.33, 0.16],
    roughness: 0.88,
    bump: 0.9,
    grain: { scale: 11.0, contrast: 0.22, ridge: 0.45 },
    // Needle-brown duff (+) to deep moss (−). The same sign pattern as grass
    // on a different axis (b/r −0.90 against grass's −0.50), so the two greens
    // of the forest floor and the meadow do not swing as one colour.
    // + [0.24, 0.311, 0.079] · − [0.06, 0.349, 0.241].
    tint: { dev: [0.09, -0.019, -0.081], scale: 1.0 },
  },
  {
    name: "rock",
    color: [0.5, 0.48, 0.46],
    roughness: 0.7,
    bump: 2.2,
    grain: { scale: 16.7, contrast: 0.14, ridge: 0.8 },
    // Warm buff feldspar (+) to cool blue-grey biotite (−). Granite's VALUE
    // range is wide and this octave deliberately does not spend it: the value
    // is the grain octave's (ridge 0.8, the crystalline speckle) and the
    // crevice darkening the rock rebuild will add. What this adds is the
    // mineral hue. + [0.58, 0.464, 0.38] · − [0.42, 0.496, 0.54].
    tint: { dev: [0.08, -0.016, -0.08], scale: 1.4 },
  },
];

// Field scales, in cycles per metre — one field, three octaves, shared by
// every channel below. Wavelengths ~48 m / ~9.5 m / ~1.7 m.
//
// Re-placing the meso octave was tried this pass and backed out, which is
// worth the note because the reason to try it stands. Every octave retires at
// a footprint (below), so the coarsest one that survives mid distance sets the
// texture a player actually sees, and at 9.5 m that octave completes a third
// of a cycle inside a typical 8 m ground framing — a gradient across the whole
// frame, which is exactly the "single flat hue" the visual judge's acceptance
// names. 4 m completes two cycles and still retires far beyond any footprint
// this world produces.
//
// What stopped it is a coupling, not the arithmetic: the splat break-up wobble
// (`gmWob`, below) is driven by gmMacro and gmMeso, so moving meso moves which
// identity owns a given face — and the grain octave takes its scale, contrast
// and ridge as dot products against those same weights. `browser_smoke` 15c,
// which measures the grain's projection on the 46.6° face this spawn offers,
// went red on the changed mix (gain x0.864 against its x1.15 floor). The
// octave ladder is safe to build on now; re-placing meso is part of the albedo
// work on `NOW.md` item 1 and needs 15c's face re-measured with it, not a
// constant changed underneath it.
const SCALE_MACRO = 1 / 48;
const SCALE_MESO = 1 / 9.5;
const SCALE_MICRO = 1 / 1.7;
// How far the field may push the identity weights around before they are
// sharpened and renormalized. This is what turns a smooth biome ramp into a
// mottled boundary; the four offsets sum to zero so it redistributes rather
// than brightens.
const BLEND_BREAKUP = 0.34;
// Albedo mottling per octave (multiplicative, ±).
const MOTTLE = [0.16, 0.11, 0.07];
// Roughness variation from the micro octave (±).
const ROUGH_VAR = 0.18;
// Bump amplitude per octave, in metres, at identity strength 1. Chosen
// against each octave's own wavelength (9.5 m and 1.7 m) so both land in
// the 0.03–0.25 surface-slope band where a normal actually reads; the
// footprint fades below then retire each one as it stops resolving.
const AMP_MESO_M = 0.55;
const AMP_MICRO_M = 0.09;
// Every octave retires on ONE law, and it is the law the grain octave was
// already written against (`FADE_OCTAVE_CPP` below): cycles per pixel, not
// metres. Metres cannot express "resolved" — a 1.1 m/px footprint is
// comfortable for a 48 m octave and two thirds of a cycle per pixel for a
// 1.7 m one — so a metre threshold has to be re-derived by hand per octave,
// and the two hand-derived ones were both wrong in the same direction.
//
// Measured on pass 20260802-050932-01's `04-ground-down.png`, in the grass:
// the high-pass signal's autocorrelation was 0.53 at one pixel and 0.05 at
// two — a two-pixel period, which is aliasing at exactly Nyquist — and the
// dark and bright pixels were one hue at two luminances, so it was the
// BUMP, not the albedo. The cause is arithmetic: the retired thresholds
// were `meso [2.0, 7.0]` and `micro [0.3, 1.1]` m/px, which against their
// own wavelengths are 0.74 and 0.65 cycles per pixel — both PAST the 0.5
// Nyquist limit, so each octave was still being sampled after it stopped
// being representable. What reaches the image is not the octave but its
// alias, and an aliased height field has a per-pixel-random gradient, which
// `normal_fragment_maps` below divides by the pixel footprint.
//
// Deriving the metres from the octave's own scale instead removes the class:
// `fade_m = cycles_per_pixel / scale`, one line, no per-octave judgement.
const FADE_OCTAVE_CPP = [0.03, 0.09];
// The sampling limit itself — half a cycle per pixel. Not a knob: a signal
// sampled above this is indistinguishable from a lower-frequency one, which
// is what the grass was doing. Every fade above must retire below it, and
// the browser gate asserts exactly that over the whole octave table.
const NYQUIST_CPP = 0.5;
const fadeMetres = (scale) => FADE_OCTAVE_CPP.map((cpp) => cpp / scale);
const FADE_MESO = fadeMetres(SCALE_MESO);
const FADE_MICRO = fadeMetres(SCALE_MICRO);
// Grain's own bump amplitude in metres and roughness swing. 0.008 m against
// the identities' 4–12 cm wavelengths is a surface slope of 0.07 (sand, bump
// 0.35) to 0.29 (rock, bump 2.2) — the same 0.03–0.25 band the two octaves
// above were chosen against, which is the band where a normal reads without
// turning into a relief map.
const AMP_GRAIN_M = 0.008;
const GRAIN_ROUGH = 0.1;
// Grain's footprint fade, in CYCLES PER PIXEL rather than metres, because its
// wavelength is a per-identity number and one metre threshold cannot serve
// four of them; every identity's grain dies on one curve, each at its own
// distance. Retiring at 0.09 under this `length(fwidth())` convention is ~11
// pixels per cycle — well inside Nyquist, so the octave is gone while it is
// still comfortably resolved rather than at the moment it stops being. That is
// deliberate on both counts: an octave that dies early cannot alias, and grain
// is the only octave whose cost scales with how far it reaches.
//
// It is now the SAME array the structural octaves fade on, and that is the
// whole point of this pass: grain was the only octave written against the
// sampling rate, and it was the only octave that never aliased. The law is
// not grain's, it is the field's.
const FADE_GRAIN_CPP = FADE_OCTAVE_CPP;
// The smallest grain scale any identity carries. Grain's own fade needs the
// WORLD footprint and the blended scale, both of which cost real work on
// every fragment in the frame; the horizontal footprint is a lower bound on
// the world one (xz is a subset of xyz), so `gmFw * this >= fade end` proves
// every identity's grain is already retired and the whole block can be
// skipped before any of it is computed. Conservative in the right direction:
// a steep face may enter the block and find the fade zero anyway.
const GRAIN_SCALE_MIN = Math.min(...IDENTITIES.map((i) => i.grain.scale));
// --- the tint octave (fourth pass) -----------------------------------------
// It retires on the same law as everything else — `FADE_OCTAVE_CPP`, in cycles
// per pixel, because its wavelength is a per-identity number and one metre
// threshold cannot serve four of them (grain's argument, verbatim). At 0.09
// cpp the coarsest tile (1.0 /m) retires at a 0.09 m/px footprint and the
// finest (1.7 /m) at 0.053 — roughly 60 m and 36 m out at this frustum, past
// which the macro bias below is what carries the identity's colour variation.
const FADE_TINT_CPP = FADE_OCTAVE_CPP;
// The smallest tile scale any identity carries, for the same early-out
// argument `GRAIN_SCALE_MIN` carries: `gmFw * this >= fade end` proves every
// identity's tile is already retired before the blended scale is computed.
const TINT_SCALE_MIN = Math.min(...IDENTITIES.map((i) => i.tint.scale));
// --- what is NOT here: a coarse bias on the tint ----------------------------
//
// The visual report's ask was "tiled albedo … with a low-frequency
// macro-variation octave to break tiling", and this pass built that term
// twice — once off macro (48 m), once off meso (9.5 m) — and measured it out
// both times. The record is worth keeping, because the next reader will want
// to add it back:
//
//   · **Macro.** 48 m is wider than any frame, so `gmMacro` is ONE NUMBER
//     across everything visible and an additive bias off it is not a
//     variation at all — it is a cast. Assertion 15 said so before this
//     octave's own gate existed: +0.10% of a yaw lightened against −19.80%
//     darkened, on a floor of 0.2% each. The field could only darken.
//   · **Meso.** 9.5 m completes cycles inside a standing frame's mid-ground
//     and the theory was that this would centre it. Measured on the standing
//     view: +0.01% up against −3.64% down, the chromaticity centre moved
//     0.061 while the spread gained 0.009 — six times more cast than
//     variance. Most of a standing frame's ground pixels are past 50 m, where
//     the tile has retired and the bias is all that is left, so the frame
//     inherits whatever one number the coarse octave holds there.
//
// The premise was wrong, and it was inherited from the report's own words
// rather than checked: **there is no tiling here to break.** A tiled albedo
// map repeats, and a repeat needs a macro octave to hide it. This octave is
// value noise on world XZ — it does not repeat, and 160,000 samples of it
// have no period to find. So the term was solving a problem this material
// does not have, and paying for it in the one currency the visual read cannot
// afford: a colour cast over the whole ground.
//
// What varies the ground's colour past the tile's ~55 m retirement is
// therefore the splat weights (the macro octave already drives `gmWob`, which
// moves which IDENTITY owns a face — four different colours, not one colour
// biased) and, when the lighting gap is taken, aerial perspective. Both are
// somebody else's slice, and both are the right owner for a distance effect.
//
// How much of its authored deviation a square metre actually wears.
//
// `dev` above is written as the POLE — "bleached quartz", "lush blue-green" —
// so the field that drives it has to be able to reach the pole, and the
// obvious mapping cannot. `(g - 0.5) * 2` takes the field's NOMINAL [0, 1]
// range onto [−1, 1], but a value-noise field does not spend its time at its
// nominal range: measured over 160,000 samples of this exact `gmNoise`, its
// deviation about the midpoint is 0.2262, so at a gain of 2 the mean |tint| is
// 0.379 and the pole is reached on 0.00% of the ground. The authored numbers
// would then describe a place the material never goes.
//
// 2.5 is the gain at which the mean is 0.468 — about half the pole, which is
// what "the pole" should mean — and 6.2% of the ground saturates against the
// clamp, so the extremes are real ground and not an asymptote. 3.0 saturates
// 15.6%, which starts flattening the tails into patches of constant colour.
const TINT_GAIN = 2.5;
// Luminance, in the linear working space these colours are written in
// (Rec. 709 — the primaries three's tone map and output transform assume).
// It is here so the four deviations' luminance-neutrality is a NUMBER the
// gate reads rather than a claim in a comment.
const LUMA_709 = [0.2126, 0.7152, 0.0722];
const devLuma = (d) => LUMA_709[0] * d[0] + LUMA_709[1] * d[1] + LUMA_709[2] * d[2];
// How far a deviation may lean on brightness, as a share of its own length.
// Measured on the four: 0.0017 / 0.0020 / 0.0025 / 0.0019 — zero to three
// decimal places, because each was solved for it rather than eyeballed.
//
// This is a design law and not a tolerance, and the pass earned it the hard
// way. The first two cuts of this octave swung value AND hue, and the value
// half cost the material a gate it already held: assertion 15 requires the
// procedural field to lighten some pixels and darken others on every probed
// yaw, and at this spawn's yaw 0 the field already darkens 95% of what it
// touches (the bump against a 21° sun, plus the macro octave's ±0.16
// multiply, which is itself a cast at any framing narrower than 48 m). Main
// cleared the 0.2% floor there with 0.5%. A tint that darkened as well took
// it to 0.14% — the octave was not adding a defect, it was SPENDING a margin
// that a pre-existing one had already nearly used up, and no amount of tuning
// its amplitude changes the sign of what it spends.
//
// Neutrality is not a way around that. It is the correct division of labour,
// and reading it off the material makes that obvious: three scalar octaves
// and a per-identity grain already move VALUE, four ways, at four scales.
// Nothing moved HUE. The defect the visual judge named — "one hue at two
// luminances", "a single untextured green sheet" — is that sentence exactly.
// So this octave does the missing half and none of the half already done,
// and the ground ends up varying in both because the material is the sum.
const TINT_LUMA_NEUTRAL = 0.02;
// The tint's roughness swing, on the file's law that a modifier drives colour
// AND roughness or it is a channel on its own. Sign follows the deviation: the
// `+dev` pole is the warm/dry one for all four identities — damp gold sand,
// straw grass, brown duff, buff feldspar — and dry is matte. It is also the
// one place this octave is allowed to touch brightness, and it does so
// causally (through the light response) rather than by moving albedo.
// 0.06 against a ±1 field is ±0.06, two thirds of what the micro octave's
// `ROUGH_VAR` 0.18 contributes over its own ±0.5 field — the same band,
// neither dominating.
const TINT_ROUGH = 0.06;
// Specular-AA gain on the perturbed normal's variance (three already adds
// its own term from the *unperturbed* normal; this covers what we added).
const SPEC_AA = 0.5;
// The cap on the bump's surface gradient, as a slope — the sum of what the
// three octaves that reach gmH (meso, micro and grain) actually ask for, on
// the identity that asks most (rock, bump 2.2); macro drives the splat
// weights only and never the height. In the file's own `amp x bump / wavelength`
// convention (the one the grain block states its 0.07–0.29 range in):
//
//   meso   0.55 x 2.2 / 9.5    = 0.127
//   micro  0.09 x 2.2 / 1.7    = 0.117
//   grain  0.008 x 2.2 / 0.060 = 0.294
//                                -----
//                                0.538  -> 0.55
//
// So every surface this material is DESIGNED to make passes through
// untouched, with the cap holding only what arithmetic cannot justify.
//
// The first cut of this pass put it at 1.0 by summing four octaves at the top
// of the 0.03–0.25 band, and 1.0 is a 45° perturbation. That is not a bound,
// it is a licence: with the gradient reconstruction fixed the bump reaches
// the frame at full strength for the first time (the retired space-mixed
// formula was silently attenuating it, which is what the octave amplitudes
// were unknowingly calibrated against), and at 45° against a sun 21° above
// the horizon most of the ground turns away from the key light. The shadow
// gate caught it immediately and correctly: direct light had nowhere to land,
// so toggling the shadow map moved 0.131% of the frame against a 15% floor.
const BUMP_MAX_SLOPE = 0.55;
// Causal modifiers. Wetness: below the waterline the surface is dark and
// smooth, and it dries out over the first 1.6 m of beach — this is what
// retired the separate sea-floor palette entry (0.68 × sand lands on it).
const WET_RANGE = [-0.4, 1.6];
const WET_DARKEN = 0.68;
const WET_ROUGH = 0.28;
// Snow on high rock, replacing the palette's peak lerp; the band is the
// retired C_PEAK ramp's (52 m → 80 m) and the colour is C_PEAK itself.
const SNOW_RANGE = [52.0, 80.0];
const SNOW_COLOR = [0.72, 0.72, 0.75];
const SNOW_ROUGH = 0.55;
// Cliff faces darken. `upness` is the world normal's y: 0.643 is the sim's
// 50° cliff threshold, so this ramps in over the 39°–53° band the retired
// C_CLIFF blend covered.
const CLIFF_UPNESS = [0.6, 0.78];
const CLIFF_DARKEN = 0.86;

// --- the base maps (ART.md §7) ----------------------------------------------
// The tile the photograph is laid at, in cycles per metre, and it is not a new
// number: it is each identity's OWN declared tile scale — the tint octave's,
// the 0.5–1 m band the visual report asked for and the one place in the ladder
// where a per-identity tiling frequency is already spoken for. `NOW.md` item 1
// says "at the tile scales the identities already declare", and this is the
// declaration it means. Reading it off `IDENTITIES` rather than restating it
// means the two can never disagree.
const BASE_SCALE = IDENTITIES.map((i) => i.tint.scale);
// The most a photographed normal may tilt the shading normal, as a surface
// slope (rise over run). NOT the procedural cap below it — this bounds the
// base map's own contribution BEFORE it is added to gmH's gradient, and
// `BUMP_MAX_SLOPE` then bounds the sum, so the two are nested rather than
// additive. Wall 4: the tap it bounds is `|n.xy| / n.z`, which diverges as a
// tangent normal approaches horizontal.
//
// The value is this file's own existing derivation applied to a new octave
// rather than a fresh judgement: 0.03–0.25 is the band `AMP_MESO_M`,
// `AMP_MICRO_M` and `AMP_GRAIN_M` were each chosen against, stated three times
// above as "where a normal actually reads without turning into a relief map".
// The base map is now the finest thing in that ladder and the one carrying the
// most slope, so it takes the top of the band and not a number of its own.
const BASE_NORMAL_MAX_SLOPE = 0.25;
// Where the wall plane turns on, as the face normal's dominant HORIZONTAL
// component — and it is derived rather than chosen, which is why it is
// `1/sqrt(2)` and not a round number somebody liked.
//
// The base maps ship on world XZ, which is a projection from above: on a face
// of upness `u = n.y` it stretches the photograph by `1/u` along the fall line
// and leaves it alone across it. That is materials v1's finding about the
// grain octave (see the third pass above) applied to the octave that landed
// after it — at the finest tile in the ladder, carrying 1K photographic
// content instead of a noise field, which is why it reads as a comb rather
// than as a smear nobody names. At this seed's steepest faces (`upness 0.357`,
// a 69° slope) the stretch is x2.8, and it is unbounded as the face goes
// vertical.
//
// So the fix is the same fix: sample the surface, not the shadow it casts on
// the ground plane. Two things about the shape, and the second is what this
// constant is.
//
//   1. **Two taps, not three.** Grain's three planes are noise samples; these
//      are anisotropically filtered texture fetches, the most expensive thing
//      in this program. The ground is a heightfield, so the surface has
//      exactly one direction it leans — its fall line — and the vertical
//      plane containing that line is the only wall it ever needs. Its stretch
//      is `1/sin(tilt)` where the top plane's is `1/cos(tilt)`: exact
//      complements, so between the two of them the worst case in the whole
//      world is 45°, where each is x1.41 and they average. Triplanar would
//      buy nothing here and cost a third of the program's bandwidth.
//   2. **The wall turns on at the CROSSOVER, and that is what `1/sqrt(2)`
//      is.** `sin(tilt) = cos(tilt)` at exactly 45°: below it the projection
//      the ground already had is the less distorted of the two, above it the
//      wall is. So the rule is not a tuned threshold, it is "add the second
//      tap only where the one you have stops being the better one" — and the
//      number falls out of the geometry rather than out of a judgement.
//
// It is also, independently, where this file's own accepted residual lands:
// materials v1 measured world-XZ coarsening the grain x2.02 on a 46.6° face
// and shipped a triplanar leaving x1.40 behind. A 45° face is x1.41. The
// crossover and the accepted residual are the same place to within 1%.
//
// What falls out is the level-ground identity: on flat ground `tilt` is
// exactly 0, the wall weight is exactly 0, its block is skipped whole, and
// the top weight is exactly 1 — so on level ground THIS TAP is bit-for-bit
// the single-tap program it replaces. Same property the grain projection
// carries, for the same reason.
//
// The footprint change below it carries no such algebraic identity (a world
// footprint equals a horizontal one only where the surface is exactly flat),
// so the claim for the pass as a whole is measured rather than derived: at
// the pinned spawn's near-ground vantage the two programs differ on **0.0% of
// the frame at a mean delta of 0.00**, and 15h, 15i and 15e read the same
// numbers before and after.
//
// Written as a literal rather than as `Math.SQRT1_2` so the knob registry can
// pin it — `ci/knob_registry.mjs` parses numbers, ratios and arrays, and a
// name it cannot resolve to a value is printed as unpinned. The derivation
// above is what guards it against being "tuned"; the registry is what guards
// the derivation against drifting from what ships.
const BASE_WALL_ON = 0.70711;
// The exponent the two planes' weights are raised to before they are blended,
// and it is Quilez's number at the value his own article states rather than a
// tuning of ours. A triplanar blend is `pow(abs(n), k)`; his biplanar variant
// folds that sharpening into a local-support remap that sends the least
// relevant projection's weight to exactly zero, and for the case where the
// remap does not apply he prescribes the other shape directly: "only use high
// values of k, which will reduce the weight of the least relevant projection
// to zero quickly. For example, with k = 8, we get (1/sqrt(3))^8 = 1.2%."
//
// The remap does not apply here and that is a property of the projection, not
// a shortcut. Its whole job is to remove the discontinuity where two candidate
// planes SWAP identity — Quilez's axis-aligned pair flips wherever two normal
// components are equal, and the remap makes the flip happen at zero weight.
// The fall-line frame rotates through that locus continuously and never swaps,
// so there is no discontinuity to remove and no reason to pay the remap's
// narrowed blend. What is left to buy is the sharpening, and k = 8 is what
// buys it.
//
// Why a linear blend was not good enough, as arithmetic rather than taste: the
// two planes' foreshortenings are cos(tilt) and sin(tilt), so a linear blend
// on this seed's steepest face (69.5 deg) gives the TOP plane 32.3% of the
// sample while that plane is stretched x2.86 along the fall line. A third of
// the photograph arriving smeared is the streak. At k = 8 the same face gives
// it 0.05%, and 60 deg goes 51.6% -> 2.2%.
const BASE_WALL_SHARPNESS = 8.0;
// Texture fetches per fragment at the worst case — all four identities, three
// maps on each of the two planes. The budget's ceiling, not its typical,
// and now doubly so: the exact zero-weight skip means a fragment inside one
// biome takes three, and the wall block is skipped entirely below 45°, which
// is every fragment of level ground. One `sampler2DArray` per map is what
// keeps the UNIT count at three, which is the number that actually has a hard
// limit (16 on the software rasterizer the browser gate runs, and the terrain
// program already binds five shadow maps).
const BASE_FETCHES_MAX = 6 * GROUND_LAYERS.length;
const BASE_FETCHES_LEVEL = 3 * GROUND_LAYERS.length;
const BASE_TEX_UNIT_BUDGET = 16;
// The most a per-channel palette gain may STRETCH the photograph's own colour
// deviation. 1.0 is the identity: the source's chroma variation delivered at
// the scale the camera measured it. Above 1 the palette correction is no
// longer correcting a mean, it is amplifying colour noise; below 1 it is
// discarding measured colour that the references demonstrably carry.
//
// Why this constant has to exist is the artifact the pass before this one
// shipped, and the mechanism is arithmetic rather than taste. The gain it
// shipped was `color / measured mean`, PER CHANNEL, multiplying the whole sample
// — so it scales the photograph's deviation by that same per-channel factor.
// For a source whose gain is near-uniform that is harmless: the deviation
// lands along the authored colour and reads as luminance detail. For an
// off-band source it is not. Measured on the four files that ship:
//
//     sand   gain [3.43 4.09 4.73]  span 1.38
//     grass  gain [1.20 1.97 1.94]  span 1.64
//     litter gain [1.08 3.33 4.12]  span 3.82
//     rock   gain [2.35 5.24 13.45] span 5.72   <- blue mean is 0.034 linear
//
// `rock` is the sandstone `MANIFEST.md` marks as too warm for granite. Its
// blue channel sits near the source's own noise floor, so the ×13.45 that
// corrects its MEAN also amplifies its JPEG chroma noise thirteenfold, and
// what reaches the image at a grazing footprint is per-pixel rainbow speckle.
// The measurement that separates the two, over the near-ground band, is the
// high-frequency residual resolved along the local mean colour versus
// orthogonal to it: the thirteen `Rust Images/` frames that actually contain
// ground run 0.077–0.193 (median 0.120) and the shipped frames ran 0.237–0.798
// on the five showing ground — while our residual ALONG colour, the detail 15h
// asserts, was already inside the reference range. The defect was never
// amplitude, so no amount of extra blur or anisotropy addresses it.
//
// So the ceiling is applied per layer, `min(1, MAX / span)`, and that is the
// same law `ART.md` §7 already states one derivative down: *an off-band source
// is tinted, not tolerated.* A source that already agrees with its palette
// entry keeps its colour variation whole; one that had to be dragged 5.7×
// across channels to hit its mean keeps almost none of it, because almost none
// of what it has left is its own.
const BASE_CHROMA_STRETCH_MAX = 1.0;
// The base maps retire on the SAME law as every octave in this file — cycles
// per pixel, `FADE_OCTAVE_CPP` — and the argument for it here is stronger than
// it is for any of them, which is why it is the same array and not a new one.
//
// An octave retires because a reconstructed field cannot be filtered and an
// unfiltered field past Nyquist reaches the image as its alias. A texture does
// not have that problem: the mip chain filters it correctly at any footprint.
// What a texture has instead is a LIMIT — its own mean — and `gmBaseTap` is
// built so that mean is exactly `IDENTITIES[i].color` at any chroma keep,
// which is exactly what the flat path already delivers. So past the fade the three
// fetches return an answer this shader already has written down, and skipping
// them is not an approximation of the image: `mix(a, b, 0.0)` is `a`.
//
// It is also the cost decision, and it is stated as one rather than buried:
// up to twenty-four fetches with anisotropic filtering — twelve on level
// ground, both planes at a wall — is the most expensive thing in the
// ground program, and the far field — where the answer is already known — is
// most of the screen at any framing that is not pointed at the player's feet.
// Measured on this box, that is the difference between a second browser tab
// reaching the world and not.
const FADE_BASE_CPP = FADE_OCTAVE_CPP;
// The smallest tile any identity carries, for the early-out `GRAIN_SCALE_MIN`
// and `TINT_SCALE_MIN` both carry: the weights are convex and normalized, so
// this is a lower bound on the blended scale, and wherever the guard skips the
// `smoothstep` inside would have returned exactly 1 and the fade exactly 0.
const BASE_SCALE_MIN = Math.min(...BASE_SCALE);
// Layer order is splat order is `IDENTITIES` order. Asserted rather than
// commented, because the failure it prevents — sand's photograph delivered
// under grass's weight — is a silent one that looks like a worldgen bug.
if (GROUND_LAYERS.length !== IDENTITIES.length ||
    GROUND_LAYERS.some((n, i) => n !== IDENTITIES[i].name)) {
  throw new Error(
    `base texture layers [${GROUND_LAYERS}] do not match identities ` +
      `[${IDENTITIES.map((i) => i.name)}] — the splat weight and the layer index are the same number`,
  );
}

// --- the shared field, in GLSL ---------------------------------------------
// Hash-without-sine (Dave Hoskins' hash12): four hashes per noise sample,
// three samples per pixel. No textures, no trig, no dependent texture reads.
const FIELD_GLSL = /* glsl */ `
float gmHash(vec2 p) {
  vec3 p3 = fract(vec3(p.xyx) * 0.1031);
  p3 += dot(p3, p3.yzx + 33.33);
  return fract((p3.x + p3.y) * p3.z);
}
float gmNoise(vec2 p) {
  vec2 i = floor(p);
  vec2 f = fract(p);
  vec2 u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
  float a = gmHash(i);
  float b = gmHash(i + vec2(1.0, 0.0));
  float c = gmHash(i + vec2(0.0, 1.0));
  float d = gmHash(i + vec2(1.0, 1.0));
  return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}
`;

const TERRAIN_VERT_PARS = /* glsl */ `
attribute vec4 splat;
varying vec4 vGmSplat;
varying vec3 vGmPos;
varying vec3 vGmNorm;
`;

const TERRAIN_FRAG_PARS_HEAD = /* glsl */ `
varying vec4 vGmSplat;
varying vec3 vGmPos;
varying vec3 vGmNorm;
uniform float uSurface;
uniform vec3 uIdentColor[4];
uniform vec4 uIdentRough;
uniform vec4 uIdentBump;
uniform vec3 uScales;
uniform vec3 uMottle;
uniform vec4 uFade;
uniform vec2 uOct;
uniform vec3 uSnowColor;
uniform float uGrain;
uniform vec4 uGrainScale;
uniform vec4 uGrainContrast;
uniform vec4 uGrainRidge;
uniform vec2 uGrainFade;
uniform vec2 uGrainAmp;
uniform float uTint;
uniform vec3 uIdentDev[4];
uniform vec4 uTintScale;
uniform vec2 uTintFade;
uniform sampler2DArray uBaseAlb;
uniform sampler2DArray uBaseNrm;
uniform sampler2DArray uBaseRgh;
uniform float uBase;
uniform vec4 uBaseScale;
uniform vec2 uBaseFade;
uniform vec3 uBaseAlbInvMean[4];
uniform vec3 uBaseAlbLumaW[4];
uniform vec4 uBaseChromaKeep;
uniform vec4 uBaseRghGain;
${FIELD_GLSL}
`;

// One identity's base tap: three array fetches at that identity's own tile,
// accumulated by its splat weight.
//
// Three things about the shape:
//
//   · **`texture2DGradEXT`, not `texture2D`.** three maps it to ES 3.00's
//     `textureGrad`, and an explicit-gradient fetch is the one form that is
//     DEFINED inside non-uniform control flow — which is what lets the four
//     call sites sit behind a per-fragment weight test. The gradients are
//     taken once, above the branches, from the same `gmXZ` every octave in
//     this file is a function of.
//   · **The normals are blended before they are divided, not after.** Each
//     tap adds `n.xy` and the slope is taken once from the convex sum, so a
//     single layer with a near-horizontal normal cannot dominate the blend
//     through a divide that has already diverged. z is reconstructed rather
//     than stored (`textures.js` ships RG), which is exact for a unit vector.
//   · **The albedo arrives as a RELATIVE field, and its chroma is delivered
//     at a bounded stretch.** `rel` is the sample over the layer's own
//     measured mean, so it has mean exactly 1 in every channel; `k` is that
//     same field resolved to a scalar through the layer's own luma weights, so
//     it has mean exactly 1 too. Both ends are then multiplied by the authored
//     colour, which makes `IDENTITIES[i].color` the delivered mean **for any
//     value of the keep** — the mix is convex and both of its ends already
//     average to 1, so `ART.md` §7's mean preservation is a property of this
//     shape rather than of its tuning. What the keep chooses is only how much
//     of the photograph's colour DEVIATION survives the per-channel correction
//     that put its mean on the palette (`BASE_CHROMA_STRETCH_MAX`). At keep = 1
//     it is algebraically `sample * (color / mean)` — exactly what the pass
//     before this one shipped — and `chromaProbe` renders that leg to prove it.
//
//     It is non-negative by construction, which the subtractive form of the
//     same correction is not: a convex combination of two non-negative fields
//     needs no clamp, and so introduces no hue shift in the darks from one.
//   · **`k` is accumulated as well as consumed.** The scalar leg leaves the
//     tap as `gmBaseK` — the photograph resolved to a mean-1 luminance field —
//     because one modifier downstream REPLACES the albedo instead of scaling
//     it (snow) and needs the detail back in a form that carries no colour of
//     the layer it came from. One `dot` and one add; no fetch.
const BASE_TAP_GLSL = /* glsl */ `
void gmBaseTap(float lyr, float w, vec3 col, vec3 inv, vec3 lw, float keep, float rg,
               vec2 uv, vec2 dx, vec2 dy,
               inout vec3 alb, inout float lum, inout float rgh) {
  vec3 p = vec3(uv, lyr);
  vec3 rel = texture2DGradEXT(uBaseAlb, p, dx, dy).rgb * inv;
  float k = dot(lw, rel);
  alb += col * mix(vec3(k), rel, keep) * w;
  lum += k * w;
  rgh += texture2DGradEXT(uBaseRgh, p, dx, dy).r * rg * w;
}
`;

// The relief, and why it is a SEPARATE tap from the two channels above.
//
// `gmBaseSlope` below turns this fetch into `(dH/dx, dH/dz)` — a world-XZ
// HEIGHTFIELD gradient, the same quantity `gmSurf` speaks, which is what lets
// the photograph's relief and the procedural octaves' add as one gradient
// instead of two normals fighting over one shading normal. That quantity is
// only defined on the projection it is a gradient OF. A tangent normal
// sampled on a vertical plane describes a surface whose "up" is world ±X or
// ±Z; there is no honest way to read it as a height over XZ, and the wall tap
// therefore does not take one. So the relief stays the top tap's, weighted by
// the top tap's own weight — which sends it to zero exactly as the face goes
// vertical, in step with the `gmNy` factor that already scales it to nothing
// there. One fetch, not two, and the wall tap costs two rather than three.
const BASE_NRM_GLSL = /* glsl */ `
void gmBaseNrm(float lyr, float w, vec2 uv, vec2 dx, vec2 dy, inout vec2 nrm) {
  nrm += (texture2DGradEXT(uBaseNrm, vec3(uv, lyr), dx, dy).rg * 2.0 - 1.0) * w;
}
`;

/**
 * A GLSL template that keeps its prose in THIS file and out of the program.
 *
 * Every full-line `//` comment in the emitted source becomes an empty line —
 * empty rather than deleted, so a driver's `ERROR: 0:412` still names the line
 * this file shows. Nothing else is touched: trailing comments after code stay,
 * because a `//` that is not at the start of a line is one regex away from
 * being a bug and this saves almost nothing.
 *
 * Why it exists, since a size gate that prose can move is the thing to be
 * careful about: `browser_smoke`'s terrain fragment budget counts characters
 * of resolved GLSL as the proxy for what a software rasterizer charges a
 * joining tab to compile. That proxy was written when this repo's share of the
 * program was 8,145 chars and mostly code. The base maps and the biplanar wall
 * tap took the share to ~33,000, of which roughly three-fifths was comment
 * text — so the metric had started measuring how much we had explained
 * ourselves, which is not a cost the driver pays in any way that matters. The
 * budget is re-derived against the stripped program in `browser_smoke`, so the
 * wall keeps its old relative strictness rather than pocketing the difference.
 */
const glsl = (strings, ...values) =>
  strings
    .reduce((acc, s, i) => acc + s + (i < values.length ? values[i] : ""), "")
    .replace(/^[ \t]*\/\/.*$/gm, "");

/**
 * The four call sites, unrolled.
 *
 * The guard is `> 0.0` and it is EXACT, not a threshold: `gmW` is
 * `max(splat + wobble, 0.0)` squared and renormalized, so an identity this
 * metre does not contain arrives as exactly zero, and `0.0 * finite` is
 * exactly `0.0`. Skipping it removes three fetches and changes no bit of the
 * image — the same argument the micro octave's skip carries, applied to the
 * only thing in this program that costs a memory round trip.
 *
 * Unrolled rather than looped because ES 3.00 requires a sampler array index
 * to be a constant expression; the layer here is a `sampler2DArray` LAYER
 * (a float coordinate, dynamic by design) but the four tiles, gains and
 * weights are still four different swizzles, so the loop would have unrolled
 * anyway.
 */
function baseGlsl() {
  let top = "";
  let wall = "";
  for (let i = 0; i < 4; i++) {
    const c = "xyzw"[i];
    top += /* glsl */ `
          if (gmW.${c} > 0.0) {
            gmBaseTap(${i}.0, gmW.${c} * gmTop, uIdentColor[${i}],
                uBaseAlbInvMean[${i}], uBaseAlbLumaW[${i}], uBaseChromaKeep.${c}, uBaseRghGain.${c},
                gmXZ * uBaseScale.${c}, gmBdx * uBaseScale.${c}, gmBdy * uBaseScale.${c},
                gmBaseA, gmBaseK, gmBaseR);
            gmBaseNrm(${i}.0, gmW.${c},
                gmXZ * uBaseScale.${c}, gmBdx * uBaseScale.${c}, gmBdy * uBaseScale.${c},
                gmBaseNT);
          }`;
    wall += /* glsl */ `
            if (gmW.${c} > 0.0) {
              gmBaseTap(${i}.0, gmW.${c} * gmWall, uIdentColor[${i}],
                  uBaseAlbInvMean[${i}], uBaseAlbLumaW[${i}], uBaseChromaKeep.${c}, uBaseRghGain.${c},
                  gmWallUV * uBaseScale.${c}, gmWdx * uBaseScale.${c}, gmWdy * uBaseScale.${c},
                  gmBaseA, gmBaseK, gmBaseR);
              gmBaseNrm(${i}.0, gmW.${c},
                  gmWallUV * uBaseScale.${c}, gmWdx * uBaseScale.${c}, gmWdy * uBaseScale.${c},
                  gmBaseNW);
            }`;
  }
  return /* glsl */ `
        vec3 gmBaseA = vec3(0.0);
        // One accumulator PER PLANE, not one for the tap. A tangent normal is
        // only meaningful in the frame it was sampled in, so the identities
        // blend within a plane (their weights are convex and sum to 1, which is
        // what keeps "blend before divide" true) and the planes combine after,
        // in world space, where they can be added at all.
        vec2 gmBaseNT = vec2(0.0);
        vec2 gmBaseNW = vec2(0.0);
        float gmBaseR = 0.0;
        float gmBaseK = 0.0;
        float gmFadeBase = 0.0;
        // The derivatives, taken ABOVE every guard for the reason every other
        // derivative in this shader is: GLSL leaves them undefined in
        // non-uniform control flow, and these ones reach the image (they are
        // the gradient the fetches filter on, not a footprint that only fades
        // something already at zero). Three components now rather than two,
        // because the wall plane's UV is built from the same world position
        // and the world footprint below is a function of them.
        vec3 gmPdx = dFdx(vGmPos);
        vec3 gmPdy = dFdy(vGmPos);
        vec2 gmBdx = gmPdx.xz;
        vec2 gmBdy = gmPdy.xz;
        // The wall plane and its weight (materials v4). BASE_WALL_ON carries
        // the derivation; this is the construction.
        //
        // The wall is the vertical plane containing the face's own FALL LINE,
        // not one of the two world ones. A heightfield's distortion under the
        // top projection is a pure stretch of 1/cos(tilt) along the fall line,
        // and this plane's is 1/sin(tilt) along the same line — exact
        // complements, equal at 45°, each perfect where the other is worst. It
        // also turns the plane CHOICE into a rotation instead of a swap: the
        // axis-aligned pair flips wherever |n.x| = |n.z|, which is a hard line
        // of changed photograph down every diagonal cliff, and this frame
        // rotates through that locus continuously. One wall tap, always, with
        // nothing to select between.
        //
        // u runs across the slope (horizontal, perpendicular to the fall
        // line), v is world height. Both in metres, so uBaseScale means the
        // same cycles per metre it means on the top plane.
        vec3 gmNb = normalize(vGmNorm);
        vec2 gmFall = gmNb.xz / max(length(gmNb.xz), 1e-4);
        vec2 gmAcross = vec2(-gmFall.y, gmFall.x);
        vec2 gmWallUV = vec2(dot(vGmPos.xz, gmAcross), vGmPos.y);
        // The footprint is the world-position derivative PROJECTED onto the
        // frame, never the derivative OF the finished UV, and the difference
        // is not stylistic — it is a factor of eighty.
        //
        // gmAcross is a per-fragment vector, so dFdx(dot(p.xz, gmAcross))
        // expands by the product rule to
        //     dot(dFdx(p.xz), gmAcross)  +  dot(p.xz, dFdx(gmAcross))
        // and the second term is not a footprint at all, it is the frame
        // turning. It is also multiplied by p.xz, a WORLD coordinate: at this
        // island's scale that is ~1568, so a frame rotation of 1e-4 rad per
        // pixel injects 0.16 m/px of fake footprint against a real one of
        // ~0.002. textureGrad then picks a mip about seven levels too coarse,
        // in bands that follow the terrain's curvature — a flat smear locked
        // to the fall line, which is the artifact this projection was supposed
        // to remove. Quilez states the rule directly for the axis-aligned
        // case: take the gradients of p BEFORE the projection is chosen and
        // pass the components. Holding the frame fixed is the same rule for a
        // frame that rotates instead of switching.
        vec2 gmWdx = vec2(dot(gmPdx.xz, gmAcross), gmPdx.y);
        vec2 gmWdy = vec2(dot(gmPdy.xz, gmAcross), gmPdy.y);
        // The weights, and the exponent on them.
        //
        // The two planes' foreshortenings are sin and cos of the face's tilt,
        // and a LINEAR blend of them is wrong in a way that is easy to state:
        // at this seed's steepest face (69.5 deg) it hands 32.3% of the sample
        // to the top plane, which is stretched x2.86 there. A third of the
        // photograph arriving smeared is not a subtle contribution — it is the
        // streaking itself.
        //
        // The exponent is Quilez's, at the value his article states, and it is
        // sourced rather than invented: a triplanar blend uses pow(|n|, k), his
        // biplanar variant folds the same sharpening into a local-support
        // remap, and where the remap does not apply he says to "only use high
        // values of k, which will reduce the weight of the least relevant
        // projection to zero quickly. For example, with k = 8, we get
        // (1/sqrt(3))^8 = 1.2%, small enough that in practice it will be
        // totally unnoticeable." His remap does not transfer here — it exists
        // to kill a discontinuity where two candidate planes SWAP, and the
        // fall-line frame rotates through that locus instead of swapping — so
        // this is the second of the two shapes he gives, not a third one.
        //
        // At k = 8 the top plane's share goes 32.3% -> 0.05% at 69.5 deg and
        // 51.6% -> 2.2% at 60, while 45 deg is unchanged (the gate below still
        // owns everything gentler than the crossover). The handover lands
        // between 50 and 60 degrees instead of being spread across the whole
        // range where the top plane is the worse of the two.
        float gmTilt = length(gmNb.xz);
        float gmWall = pow(gmTilt, ${BASE_WALL_SHARPNESS.toFixed(1)})
                     * clamp((gmTilt - ${BASE_WALL_ON.toFixed(5)})
                             * ${(1 / (1 - BASE_WALL_ON)).toFixed(5)}, 0.0, 1.0);
        float gmTop = pow(abs(gmNb.y), ${BASE_WALL_SHARPNESS.toFixed(1)});
        float gmBW = max(gmTop + gmWall, 1e-4);
        gmTop /= gmBW;
        gmWall /= gmBW;
        // gmFw is the world footprint since materials v4, so this fade needs
        // no correction of its own: the base maps retired on the horizontal
        // one like everything else, which reported a cliff at 300 m as near
        // ground and kept twelve fetches alive out there delivering a texture
        // below the pixel.
        if (gmFw * ${BASE_SCALE_MIN.toFixed(4)} < uBaseFade.y) {
          float gmBScale = dot(uBaseScale, gmW);
          gmFadeBase = 1.0 - smoothstep(uBaseFade.x, uBaseFade.y, gmFw * gmBScale);
        }
        if (gmFadeBase > 0.0) {${top}
          // Skipped wherever the face is gentler than the ramp's foot, which
          // is most of the world and all of level ground — where gmTop is
          // then exactly 1 and the block above is bit-for-bit the single-tap
          // program this replaces.
          if (gmWall > 0.0) {${wall}
          }
        }
        // Each plane's blended tangent normal becomes a slope in ITS OWN
        // frame, bounded there, and then a world-space VOLUME GRADIENT — a
        // vector, not an xz pair. That is the one representation the two
        // planes can be added in, and it is the same representation the
        // procedural octaves' gradient is promoted to below, so the whole
        // relief of this material is one sum in one space (the surface-
        // gradient framework's central claim, and the reason the wall tap can
        // carry a normal map at all).
        //
        // The bound is applied per plane, before the combine, because
        // BASE_NORMAL_MAX_SLOPE is a statement about what ONE photographed
        // normal may ask for, and |n.xy| / n.z diverges as a tangent normal
        // approaches horizontal (wall 4). The convex plane weights then keep
        // the sum inside the same bound, and BUMP_MAX_SLOPE bounds it again
        // after the octaves join — nested, as before, not additive.
        float gmBaseAmt = uBase * gmFadeBase;
        // The roughness map as a mean-1 field, for the modifiers that set a
        // roughness rather than scaling one. It needs no uniform and no fetch:
        // each layer's tap already carries the gain that puts its mean on the
        // authored value, so dividing the blend by the blended authored value
        // is exactly the photograph over its own mean. The weights are convex
        // and the plane weights sum to 1, so the denominator is the mean of
        // the numerator by construction, at any tilt.
        float gmBaseRel = gmBaseR / max(dot(uIdentRough, gmW), 1e-4);
        vec2 gmSlopeT = -gmBaseNT / max(sqrt(max(1.0 - dot(gmBaseNT, gmBaseNT), 0.0)), 1e-3);
        vec2 gmSlopeW = -gmBaseNW / max(sqrt(max(1.0 - dot(gmBaseNW, gmBaseNW), 0.0)), 1e-3);
        float gmMagT = length(gmSlopeT);
        float gmMagW = length(gmSlopeW);
        gmSlopeT *= min(gmMagT, ${BASE_NORMAL_MAX_SLOPE.toFixed(3)}) / max(gmMagT, 1e-12);
        gmSlopeW *= min(gmMagW, ${BASE_NORMAL_MAX_SLOPE.toFixed(3)}) / max(gmMagW, 1e-12);
        // The top plane's axes are world +x and +z (the frame textures.js'
        // flipY pair was chosen to deliver); the wall plane's are the
        // across-slope horizontal and world +y, which is how gmWallUV was
        // built. Reading the axes off the same vectors that built the UV is
        // what stops the relief and the albedo disagreeing about which way
        // the photograph is facing.
        vec3 gmBaseGrad = gmBaseAmt * (
            gmTop * vec3(gmSlopeT.x, 0.0, gmSlopeT.y)
          + gmWall * (vec3(gmAcross.x, 0.0, gmAcross.y) * gmSlopeW.x
                    + vec3(0.0, 1.0, 0.0) * gmSlopeW.y));`;
}

// Grain's own sample: the shared field, folded toward its ridge by the
// identity's own ridge amount, returned SIGNED (±0.5) so a soft stipple and a
// crystalline speckle sit on the same midpoint and neither biases the albedo
// it multiplies. Emitted only into the variants that carry grain — a helper
// nothing calls is still source the driver compiles, and program size is a
// budget this gate asserts.
const GRAIN_GLSL = /* glsl */ `
float gmGrainTap(vec2 uv, float s, float ridge) {
  float g = gmNoise(uv * s);
  return mix(g, 1.0 - abs(2.0 * g - 1.0), ridge) - 0.5;
}
`;

// The projection (third pass). Three taps of the same tap function, one per
// world plane, blended by the face's own normal — so the grain is laid ON the
// surface instead of stamped through it from above.
//
// `w` is the raw `|n|` normalized to sum 1. No sharpening exponent: that is a
// knob nobody has spoken, and the two corrections below are the ones the
// measurement said were load-bearing.
//
//   · the fold is inside `gmGrainTap`, so it happens PER PLANE, before the
//     blend — a ridged field averaged three ways is not a ridged field.
//   · `/ length(w)` restores the deviation the blend removed. Three
//     independent fields blended with convex weights have `|w|` of one
//     field's spread; dividing by `|w|` puts it back, and `|w|` is exactly 1
//     wherever one plane owns the fragment, which is what makes level ground
//     bit-identical to the world-XZ tap this replaces.
//
// The mean rides along with the deviation (the fold's output is not centred
// on 0.5 for a bell-shaped field, so the tap is not exactly zero-mean), which
// is deliberate: it is the same bias materials v1 shipped, scaled by the same
// factor as the signal it sits on, rather than a second correction nobody has
// measured.
const GRAIN_TRI_GLSL = /* glsl */ `
float gmGrainTri(vec3 p, vec3 nrm, float s, float ridge) {
  vec3 w = abs(nrm);
  w /= max(w.x + w.y + w.z, 1e-4);
  float t = gmGrainTap(p.zy, s, ridge) * w.x
          + gmGrainTap(p.xz, s, ridge) * w.y
          + gmGrainTap(p.xy, s, ridge) * w.z;
  return t / max(length(w), 1e-4);
}
`;

// A helper nothing calls is still source the driver compiles, and program size
// is a budget this gate asserts — so each variant carries exactly the taps it
// uses and no more.
const terrainFragPars = (grain) => {
  // The base tap is in EVERY variant, including `nofield`. That is what keeps
  // `nofield`'s image equal to the `uSurface = 0` one — the property the whole
  // cost sweep's meaning rests on — because the base maps are not part of the
  // field and are not multiplied by `uSurface`: both frames carry them, so the
  // difference between the two is still the field's instructions and nothing
  // else. `uBase` is the handle that removes them, and like `uGrain` and
  // `uTint` it is a probe input, not a quality setting.
  const head = TERRAIN_FRAG_PARS_HEAD + BASE_TAP_GLSL + BASE_NRM_GLSL;
  if (grain === "on") return head + GRAIN_GLSL + GRAIN_TRI_GLSL;
  if (grain === "xz") return head + GRAIN_GLSL;
  return head;
};

// --- the cost variants (NOW.md item 1) --------------------------------------
// A uniform cannot remove an instruction. `uSurface` weights every field term
// to zero and every `gmNoise` call still runs; `uClipLevels` weights every
// shadow level to zero and every depth fetch is still taken (deliberately —
// a comparison sampler behind a per-pixel branch has undefined derivatives).
// So the two probes that prove those systems reach the image cannot say what
// either COSTS, and `NOW.md` item 1 is a question about cost: grain did not
// merge because the terrain program was already too expensive for the browser
// gate's third tab, and nothing in the tree could say which half.
//
// These compile the ground WITHOUT a term instead:
//
//   field: "off"   — the noise field gone, not zeroed. Its image is the same
//                    frame `uSurface = 0` produces (every field term is
//                    multiplied by uSurface, and 0 × finite is exactly 0), so
//                    the probe can CHECK that the variant removed only what
//                    the surface probe's toggle removes, and the time between
//                    them is then attributable to the field's instructions.
//   shadow: "near1"/"off" — shadows.js' variants, same argument.
//   grain: "off"   — the fourth octave gone: its helper, its block, its tap.
//                    Its image is the frame `uGrain = 0` produces (every grain
//                    term is multiplied by uGrain), so the same check the field
//                    gets applies to it, and the time between the two programs
//                    is then the octave's own. This is the variant `NOW.md`
//                    item 1 asked for by name: grain's first attempt rejected
//                    itself on a ~9% frame delta measured against nothing, and
//                    the control run in the same sweep is what turns a delta
//                    into a measurement or into "under the floor".
//
// Nothing the player sees is ever built from one: `makeTerrainMaterial()`
// with no argument is the shipped program, and the gate asserts its variant
// name is "ship".
// A sixth is not about cost at all. `noskip` compiles the micro octave
// UNCONDITIONALLY — the field exactly as materials v0 shipped it, before this
// slice made the sample conditional on its own footprint fade. The skip is
// bit-exact by construction (every use of that octave is multiplied by a fade
// that is exactly zero wherever the branch skips, and 0 × finite is 0), and an
// argument like that is worth precisely as much as the gate behind it. So the
// gate renders both and requires the same frame.
//
// A seventh is not about cost either, and it is not in the sweep. `flatgrain`
// compiles the grain octave on WORLD XZ — materials v1's projection, exactly
// as it shipped — with every other instruction in the program unchanged. It
// exists so `projectionProbe` can render the combed grain and the triplanar
// one from one camera in one run and score the difference, which is the only
// form of that measurement this box can be trusted with. It is built on the
// projection probe's first ask rather than with the cost variants, because a
// seventh compile in the cost sweep would cost the sweep a seventh of its time
// to time a program the sweep is not asking about.
const TERRAIN_VARIANTS = [
  "ship",
  "nofield",
  "nograin",
  "near1",
  "noshadow",
  "noskip",
  "flatgrain",
];
// The six the cost sweep wears. `flatgrain` answers a different question and
// is built by `makeTerrainProjectionVariant()`.
const COST_VARIANTS = ["ship", "nofield", "nograin", "near1", "noshadow", "noskip"];
export const PROJECTION_VARIANT = "flatgrain";

const VARIANT_CONFIG = {
  ship: { field: "full", grain: "on", tint: "on", shadow: "ship" },
  // The field is one field: taking its instructions out takes the fourth and
  // fifth octaves' with them, or `nofield` would be "the field minus three of
  // its five octaves" and its timing would be attributed to the wrong thing.
  // It is also what keeps this variant's IMAGE equal to the `uSurface = 0`
  // one, which is the check that makes its timing mean anything: the tile
  // sample sits behind its own footprint guard and not behind `uSurface`, so
  // a variant that zeroed the field and left the tile compiled would still
  // paint a chromatic swing the uniform-zeroed frame does not have.
  nofield: { field: "off", grain: "off", tint: "off", shadow: "ship" },
  nograin: { field: "full", grain: "off", tint: "on", shadow: "ship" },
  near1: { field: "full", grain: "on", tint: "on", shadow: "near1" },
  noshadow: { field: "full", grain: "on", tint: "on", shadow: "off" },
  noskip: { field: "always", grain: "on", tint: "on", shadow: "ship" },
  flatgrain: { field: "full", grain: "xz", tint: "on", shadow: "ship" },
};

/** What a variant's `grain` setting is called in the facts the gate reads. */
const projectionName = (grain) =>
  grain === "on" ? "triplanar" : grain === "xz" ? "xz" : "none";
/** Noise taps the grain octave costs per fragment under each setting. */
const grainTaps = (grain) => (grain === "on" ? 3 : grain === "xz" ? 1 : 0);
/** …and the tint octave's, which is one tap or none. */
const tintTaps = (tint) => (tint === "on" ? 1 : 0);

/** The three samples of the shared field, or the constants that replace it. */
function fieldGlsl(field) {
  if (field === "always") {
    // materials v0's own line, kept alive as the thing `ship` is checked
    // against. Not reachable from any material a player's frame is built from.
    return /* glsl */ `
        float gmMacro = gmNoise(gmXZ * uScales.x);
        float gmMeso  = gmNoise(gmXZ * uScales.y);
        float gmMicro = gmNoise(gmXZ * uScales.z);`;
  }
  if (field === "off") {
    // 0.5 is the field's own midpoint, so every `(gm* - 0.5)` term folds to
    // exactly zero and this variant lands on the `uSurface = 0` image.
    return /* glsl */ `
        float gmMacro = 0.5;
        float gmMeso  = 0.5;
        float gmMicro = 0.5;`;
  }
  return /* glsl */ `
        float gmMacro = gmNoise(gmXZ * uScales.x);
        float gmMeso  = gmNoise(gmXZ * uScales.y);
        // The micro octave is skipped where its own footprint fade has
        // already retired it. Every use of gmMicro below is multiplied by
        // gmFadeMicro, so at a fade of exactly zero its value cannot reach
        // the image and the skip is bit-exact rather than an approximation.
        // Not a derivative hazard either: every fwidth/dFdx in this shader is
        // taken outside this branch, which is why the footprint is computed
        // above the field rather than below it.
        float gmMicro = 0.0;
        if (gmFadeMicro > 0.0) gmMicro = gmNoise(gmXZ * uScales.z);`;
}

/**
 * The grain octave, or the constants that replace it.
 *
 * Grain's scale, shape and contrast are each a dot product against the same
 * splat weights, so a biome boundary morphs one grain into another rather than
 * cross-fading two of them.
 *
 * The tap itself is three taps in the shipped program — one per world plane,
 * blended by the face's own normal (`GRAIN_TRI_GLSL`) — which is why the guard
 * below matters more after the third pass than it did before it: what it skips
 * is now three noise samples, not one.
 *
 * The whole block sits behind one multiply and one compare, because everything
 * in it — a second fwidth, the blended scale, the fade, the tap — is work the
 * far field would throw away. Its footprint is the WORLD one, not gmXZ's: a
 * pixel on a steep face barely moves in XZ while covering metres of surface,
 * and filtering on the horizontal footprint alone would keep a 4 cm octave
 * live at any distance on exactly the faces it reads worst on.
 *
 * Since materials v4 that is `gmFw` itself and this block no longer takes a
 * footprint of its own. Grain was the only octave in the file written against
 * the sampling rate it is actually sampled at, and the only octave that never
 * aliased; the fix was not to give the others a private copy of grain's
 * measurement but to make the shared one correct, which is what `gmFw` is now.
 * The guard above and the smoothstep inside therefore read the same quantity,
 * so the block is entered exactly when it has something to contribute.
 *
 * `gmGScale >= GRAIN_SCALE_MIN` because the weights are convex and normalized,
 * so wherever the guard skips, the `smoothstep` inside would have returned
 * exactly 1 and the fade exactly 0. `gmH` therefore stays continuous across
 * the boundary and the `dFdx(gmH)` below sees no step.
 *
 * No derivative is taken inside this branch any more — the hazard the old
 * `fwidth(vGmPos)` here carried (GLSL leaves derivatives in non-uniform
 * control flow undefined, and this branch is per-fragment) is simply gone with
 * it. Every derivative in this shader is now taken at the top level.
 */
function grainGlsl(grain) {
  if (grain === "off") {
    // Both terms are the additive identity of what they feed (albedo swing and
    // a signed bump/roughness offset), so this variant lands on the `uGrain=0`
    // image exactly and the gate can check that it did.
    return /* glsl */ `
        float gmGrain = 0.0;
        float gmGrainSwing = 0.0;`;
  }
  // The one line the projection changes. `flatgrain` keeps materials v1's
  // world-XZ tap; the shipped program lays the same tap on the surface.
  const tap =
    grain === "xz"
      ? `gmGrainTap(gmXZ, gmGScale, dot(uGrainRidge, gmW))`
      : `gmGrainTri(vGmPos, vGmNorm, gmGScale, dot(uGrainRidge, gmW))`;
  return /* glsl */ `
        float gmGrain = 0.0;
        float gmGrainSwing = 0.0;
        if (gmFw * ${GRAIN_SCALE_MIN.toFixed(4)} < uGrainFade.y) {
          float gmGScale = dot(uGrainScale, gmW);
          float gmFadeGrain = 1.0 - smoothstep(uGrainFade.x, uGrainFade.y, gmFw * gmGScale);
          if (gmFadeGrain > 0.0) {
            gmGrain = ${tap}
                    * 2.0 * gmFadeGrain * uSurface * uGrain;
            gmGrainSwing = gmGrain * dot(uGrainContrast, gmW);
          }
        }`;
}

/**
 * The tint octave, or the constants that replace it.
 *
 * One noise sample, at the identity's own tile scale, signed to ±1 and biased
 * by the macro octave. What it multiplies is `gmDev` — the identities' four
 * chromatic deviations blended by the same splat weights — so what reaches
 * albedo is a colour that moves in a DIRECTION, which is the one thing the
 * three scalar octaves above this could not do.
 *
 * Three things about the shape are deliberate:
 *
 *   · **The guard is a plain compare and the sample is inside it.** Unlike
 *     grain's block this takes no derivative of its own — `gmFw` is computed
 *     once above the field and `gmNoise` differentiates nothing — so the
 *     branch carries none of grain's non-uniform-control-flow hazard and needs
 *     none of its argument. `TINT_SCALE_MIN` is a lower bound on the blended
 *     scale (the weights are convex and normalized), so wherever the guard
 *     skips, the `smoothstep` inside would have returned exactly 1 and the
 *     fade exactly 0.
 *   · **The fade is applied to the SAMPLE, not to the result.** `gmTile` sits
 *     at exactly 0.5 wherever the octave has retired, including everywhere the
 *     guard skips, so the tint is exactly zero out there and continuous across
 *     the guard's own boundary. Fading the result instead would have been the
 *     same arithmetic here and is not the same arithmetic the moment anything
 *     is added to it — which is how the coarse-bias term below survived its
 *     own fade and reached the far field as a constant.
 *   · **The clamp exists because `TINT_GAIN` makes it reachable.** The
 *     deviations are authored as poles, not as midpoints to be exceeded, and
 *     6.2% of the ground lands on one.
 */
function tintGlsl(tint) {
  if (tint === "off") {
    // Both are the additive identity of what they feed, so this variant lands
    // on the `uTint = 0` image — and, since every other field term is already
    // multiplied by `uSurface`, on the `uSurface = 0` image too.
    return /* glsl */ `
        float gmTint = 0.0;
        vec3 gmDev = vec3(0.0);`;
  }
  return /* glsl */ `
        vec3 gmDev = uIdentDev[0] * gmW.x + uIdentDev[1] * gmW.y
                   + uIdentDev[2] * gmW.z + uIdentDev[3] * gmW.w;
        float gmTScale = dot(uTintScale, gmW);
        float gmTile = 0.5;
        if (gmFw * ${TINT_SCALE_MIN.toFixed(4)} < uTintFade.y) {
          float gmFadeTint = 1.0 - smoothstep(uTintFade.x, uTintFade.y, gmFw * gmTScale);
          if (gmFadeTint > 0.0) {
            gmTile = 0.5 + (gmNoise(gmXZ * gmTScale) - 0.5) * gmFadeTint;
          }
        }
        float gmTint = clamp((gmTile - 0.5) * ${TINT_GAIN.toFixed(3)},
                             -1.0, 1.0) * uSurface * uTint;`;
}

/**
 * The terrain material. One instance serves every chunk and the far mesh,
 * so the uniforms below (including the probe's `uSurface`) are the scene's
 * single handle on the ground's surface.
 *
 * @param {string} [variantName] one of TERRAIN_VARIANTS; "ship" is what the
 *   client renders and the only one built outside `costProbe`.
 */
export function makeTerrainMaterial(variantName = "ship") {
  if (!TERRAIN_VARIANTS.includes(variantName)) {
    throw new Error(`unknown terrain material variant: ${variantName}`);
  }
  const variant = VARIANT_CONFIG[variantName];
  // Loud, not quiet. A ground with no base maps is not a degraded mode worth
  // shipping behind a fallback — it is the material this slice exists to
  // replace, and a client that silently rendered it would be indistinguishable
  // from one where the assets 404'd. `boot()` awaits `loadGroundTextures()`
  // before anything reaches this line.
  const base = groundTextures();
  if (!base) {
    throw new Error(
      "makeTerrainMaterial: base textures not loaded — await loadGroundTextures() before building the ground",
    );
  }
  const material = new THREE.MeshStandardMaterial({
    color: 0xffffff,
    roughness: 1.0,
    metalness: 0.0,
  });
  const uniforms = {
    uSurface: { value: 1 },
    uIdentColor: { value: IDENTITIES.map((i) => new THREE.Vector3(...i.color)) },
    uIdentRough: { value: new THREE.Vector4(...IDENTITIES.map((i) => i.roughness)) },
    uIdentBump: { value: new THREE.Vector4(...IDENTITIES.map((i) => i.bump)) },
    uScales: { value: new THREE.Vector3(SCALE_MACRO, SCALE_MESO, SCALE_MICRO) },
    uMottle: { value: new THREE.Vector3(...MOTTLE) },
    uFade: { value: new THREE.Vector4(FADE_MESO[0], FADE_MESO[1], FADE_MICRO[0], FADE_MICRO[1]) },
    uOct: { value: new THREE.Vector2(AMP_MESO_M, AMP_MICRO_M) },
    uSnowColor: { value: new THREE.Vector3(...SNOW_COLOR) },
    // The second pass's handle, and the four vectors the grain octave reads
    // its identity off. It ships at 1: a probe input, not a quality setting,
    // and the gate reads the live value back to prove it.
    uGrain: { value: 1 },
    uGrainScale: { value: new THREE.Vector4(...IDENTITIES.map((i) => i.grain.scale)) },
    uGrainContrast: { value: new THREE.Vector4(...IDENTITIES.map((i) => i.grain.contrast)) },
    uGrainRidge: { value: new THREE.Vector4(...IDENTITIES.map((i) => i.grain.ridge)) },
    uGrainFade: { value: new THREE.Vector2(...FADE_GRAIN_CPP) },
    uGrainAmp: { value: new THREE.Vector2(AMP_GRAIN_M, GRAIN_ROUGH) },
    // The fourth pass's handle and the three vectors the tint octave reads.
    // Ships at 1 for the same reason `uGrain` does: a probe input, not a
    // quality setting, and the gate reads the live value back to prove it.
    uTint: { value: 1 },
    uIdentDev: { value: IDENTITIES.map((i) => new THREE.Vector3(...i.tint.dev)) },
    uTintScale: { value: new THREE.Vector4(...IDENTITIES.map((i) => i.tint.scale)) },
    uTintFade: { value: new THREE.Vector2(...FADE_TINT_CPP) },
    // The fifth pass, and the first thing in this material that was measured
    // by a camera rather than derived. Ships at 1, same argument as `uGrain`
    // and `uTint`: a probe input, not a quality setting.
    uBase: { value: 1 },
    uBaseAlb: { value: base.albedo },
    uBaseNrm: { value: base.normal },
    uBaseRgh: { value: base.rough },
    uBaseScale: { value: new THREE.Vector4(...BASE_SCALE) },
    uBaseFade: { value: new THREE.Vector2(...FADE_BASE_CPP) },
    // 1 / measured mean, per channel per identity. The sample times this is a
    // field with mean exactly 1, and the authored colour then multiplies it —
    // so the palette entry is the delivered mean and the photograph is the
    // deviation around it, the same arithmetic the gain form carried, split so
    // the deviation's chroma can be bounded separately from its luminance.
    uBaseAlbInvMean: {
      value: IDENTITIES.map(
        (_id, i) =>
          new THREE.Vector3(
            1 / Math.max(base.meanLinear[i][0], 1e-4),
            1 / Math.max(base.meanLinear[i][1], 1e-4),
            1 / Math.max(base.meanLinear[i][2], 1e-4),
          ),
      ),
    },
    // The weights that collapse that relative field to a scalar with mean 1
    // as well: `L * mean / dot(L, mean)`, which sums to 1 by construction. Two
    // consequences, and the second is the reason the weights are per layer
    // rather than a shared luma triple:
    //   · `dot(lw, rel)` has mean exactly 1, so the scalar end of the mix
    //     preserves the palette mean on its own.
    //   · for deviation that is luminance-correlated — which is what a
    //     photograph's DETAIL is — the scalar end delivers the identical luma
    //     contrast the per-channel gain delivered. That is what makes this
    //     change surgical: 15h's absolute luma/px floor measures the term this
    //     leaves alone, and only the chroma that 15i measures moves.
    uBaseAlbLumaW: {
      value: IDENTITIES.map((_id, i) => {
        const m = base.meanLinear[i];
        const w = [0.2126 * m[0], 0.7152 * m[1], 0.0722 * m[2]];
        const s = Math.max(w[0] + w[1] + w[2], 1e-6);
        return new THREE.Vector3(w[0] / s, w[1] / s, w[2] / s);
      }),
    },
    // How much of each photograph's own colour deviation survives, per layer:
    // `min(1, BASE_CHROMA_STRETCH_MAX / span)` where span is that layer's own
    // measured gain span. Derived from the source files rather than chosen —
    // a layer already in its palette's band keeps its colour whole, and the
    // further a source had to be dragged across channels to hit its mean, the
    // less of what is left is its own. `baseChromaKeep()` is where it is
    // computed, so the gate reads the same function the shader is fed from.
    uBaseChromaKeep: { value: new THREE.Vector4(...baseChromaKeep(base)) },
    // …and the same trick on roughness, so `uIdentRough`'s four authored
    // answers-to-light stay each layer's mean and what the map adds is where
    // that layer is polished and where it is matte.
    uBaseRghGain: {
      value: new THREE.Vector4(
        ...IDENTITIES.map((id, i) => id.roughness / Math.max(base.meanRough[i], 1e-4)),
      ),
    },
  };

  material.onBeforeCompile = (shader) => {
    // The baseline, captured BEFORE the first replace: three's own standard
    // material as three handed it over. Measured rather than inferred, and
    // this is the second time round — the first cut of this slice took the
    // `noshadow` variant's size for "stock" and called the remainder ours,
    // which understated what this repo adds to the ground by 2.9x. A variant
    // is the shipped program minus ONE term; it is not the unpatched one.
    material.userData.cost.stockFragmentChars = resolvedGlslChars(shader.fragmentShader);
    Object.assign(shader.uniforms, uniforms);

    shader.vertexShader = shader.vertexShader
      .replace("#include <common>", `#include <common>\n${TERRAIN_VERT_PARS}`)
      .replace(
        "#include <beginnormal_vertex>",
        `#include <beginnormal_vertex>
        vGmNorm = normalize(mat3(modelMatrix) * objectNormal);`,
      )
      .replace(
        "#include <begin_vertex>",
        `#include <begin_vertex>
        vGmSplat = splat;
        vGmPos = (modelMatrix * vec4(transformed, 1.0)).xyz;`,
      );

    // Everything the material decides happens here, in the graph's order,
    // and leaves two locals (gmRough, gmH) for the roughness and normal
    // stages below — three's chunks run in one main(), so they carry.
    shader.fragmentShader = shader.fragmentShader
      .replace("#include <common>", glsl`#include <common>\n${terrainFragPars(variant.grain)}`)
      .replace(
        "#include <color_fragment>",
        glsl`
        vec2 gmXZ = vGmPos.xz;

        // Filtered microstructure, hoisted: both octaves fade by pixel
        // footprint, so detail that can no longer be resolved is gone rather
        // than aliasing — and an octave that is gone need not be SAMPLED.
        // The fades are computed here, above the field, for that second
        // reason (see fieldGlsl).
        //
        // The footprint is the WORLD one and not the horizontal one, and that
        // one word is materials v4's whole finding. TERRAIN.md has stated the
        // law since materials v1 — "its fade reads the world footprint, not
        // the horizontal one, because a pixel on a steep face barely moves in
        // XZ while covering metres of surface" — and it was written about
        // GRAIN, applied to grain, and never propagated to the three octaves
        // grain was derived from, nor to the tint, nor to the base maps. Four
        // consumers of one law and one of them obeying it.
        //
        // What that cost is not subtle and it is not a smear. On a 69 degree
        // face — the steepest this seed builds — the XZ footprint is 1/2.8 of
        // the real one, so every octave here stayed at FULL strength roughly
        // three times past its own retirement. A reconstructed field cannot
        // be filtered (this file's own argument for why it fades at all), so
        // what reached the image past Nyquist was its alias: a radial comb of
        // hard lines with a chevron moire over it, locked to the fall line,
        // over every cliff and steep hillside in the world. Toggling uBase
        // off makes it louder rather than quieter, which is how it was
        // finally attributed — it was never the photograph.
        float gmFw = max(length(fwidth(vGmPos)), 1e-5);
        float gmFadeMeso = 1.0 - smoothstep(uFade.x, uFade.y, gmFw);
        float gmFadeMicro = 1.0 - smoothstep(uFade.z, uFade.w, gmFw);
${fieldGlsl(variant.field)}

        // Identity weights: the sim's own (height, moisture, slope) call,
        // pushed around by the field so the boundary is mottled, then
        // squared (a splat blend, not a dissolve) and renormalized.
        vec4 gmW = max(vGmSplat, 0.0);
        vec4 gmWob = vec4(gmMacro, gmMeso, 1.0 - gmMacro, 1.0 - gmMeso) - 0.5;
        gmW = max(gmW + gmWob * (${BLEND_BREAKUP.toFixed(4)} * uSurface), 0.0);
        gmW *= gmW;
        gmW /= max(dot(gmW, vec4(1.0)), 1e-4);

        vec3 gmAlbedo = uIdentColor[0] * gmW.x + uIdentColor[1] * gmW.y
                      + uIdentColor[2] * gmW.z + uIdentColor[3] * gmW.w;
        float gmRough = dot(uIdentRough, gmW);
        float gmBump = dot(uIdentBump, gmW);
${baseGlsl()}
        // The base lands HERE — after the identity is decided and before
        // anything modifies it — because it IS the identity's surface, not a
        // layer over one. At uBase = 0 both mixes return the authored constant
        // and the ground is bit-for-bit the material that shipped before this
        // slice, which is what makes the toggle an instrument.
        gmAlbedo = mix(gmAlbedo, gmBaseA, gmBaseAmt);
        gmRough = mix(gmRough, gmBaseR, gmBaseAmt);
${tintGlsl(variant.tint)}
        // The tint is part of the IDENTITY, so it lands before the causal
        // modifiers below and not after them: wet sand darkens the sand this
        // metre actually is, and snow covers whatever was under it. It is
        // signed and ADDED, so the identity's mean colour is exactly the
        // palette entry above and what changes is the spread around it.
        gmAlbedo += gmDev * gmTint;

        // Causal modifiers — each drives colour and roughness together.
        float gmWet = 1.0 - smoothstep(${WET_RANGE[0].toFixed(2)}, ${WET_RANGE[1].toFixed(2)}, vGmPos.y);
        float gmSnow = smoothstep(${SNOW_RANGE[0].toFixed(1)}, ${SNOW_RANGE[1].toFixed(1)}, vGmPos.y) * gmW.w;
        float gmCliff = (1.0 - smoothstep(${CLIFF_UPNESS[0].toFixed(3)}, ${CLIFF_UPNESS[1].toFixed(3)}, clamp(vGmNorm.y, 0.0, 1.0))) * gmW.w;
        gmAlbedo *= mix(1.0, ${CLIFF_DARKEN.toFixed(3)}, gmCliff);
        // Snow is the ONE modifier here that replaces the albedo instead of
        // scaling it, and that is why the high ground rendered as a flat sheet
        // while everything below it carried a photograph. gmWet and gmCliff
        // multiply, so whatever variation the base tap delivered survives
        // them; mix(albedo, SNOW_COLOR, 1) is a constant, and above 80 m
        // gmSnow IS 1 over whole hillsides. Every gate the base maps have —
        // 15h's 5.90/8.61 luma per pixel, ART.md rule 1's "no surface may be
        // one flat value" — was measured on ground the snow does not reach, so
        // nothing went red over a fifth of the island reverting to the octaves
        // alone (0.41-0.47 luma/px, an order of magnitude down).
        //
        // The repair is the base tap's own mean-preservation law, reused
        // rather than restated: gmBaseK is the photograph resolved through
        // each layer's luma weights, so it has mean exactly 1, so
        // SNOW_COLOR * gmBaseK has mean exactly SNOW_COLOR. Snow keeps its
        // authored colour to the last bit and gains the surface's luminance
        // detail — which is what snow over rock looks like anyway: it takes
        // the relief's light and shade and none of its colour. At uBase = 0
        // (and past the base's own fade) gmBaseLum is exactly 1 and this line
        // is the constant it replaces, so the toggle stays an instrument.
        //
        // Roughness gets the identical treatment, and it is the same defect
        // one channel over: SNOW_ROUGH and WET_ROUGH also REPLACE. That is why
        // the snow dome kept a uniform plastic sheen even once its albedo
        // carried a photograph, and it is why the beach — the ground every
        // gate in this repo has ever measured — has a constant-roughness strip
        // through it that nothing could see, because no probe compares
        // roughness against anything. gmBaseRel is the roughness map over its
        // own layer's measured mean, so it has mean exactly 1 and a constant
        // times it has mean exactly that constant.
        float gmBaseLum = mix(1.0, gmBaseK, gmBaseAmt);
        float gmBaseRgh = mix(1.0, gmBaseRel, gmBaseAmt);
        gmAlbedo = mix(gmAlbedo, uSnowColor * gmBaseLum, gmSnow);
        gmRough = mix(gmRough, ${SNOW_ROUGH.toFixed(3)} * gmBaseRgh, gmSnow);
        gmAlbedo *= mix(1.0, ${WET_DARKEN.toFixed(3)}, gmWet);
        gmRough = mix(gmRough, ${WET_ROUGH.toFixed(3)} * gmBaseRgh, gmWet);
${grainGlsl(variant.grain)}

        gmAlbedo *= 1.0 + uSurface * (
            (gmMacro - 0.5) * uMottle.x
          + (gmMeso - 0.5) * uMottle.y * gmFadeMeso
          + (gmMicro - 0.5) * uMottle.z * gmFadeMicro)
          + gmGrainSwing;
        gmRough = clamp(
          gmRough + uSurface * (gmMicro - 0.5) * ${ROUGH_VAR.toFixed(3)} * gmFadeMicro
                  + gmGrain * uGrainAmp.y
                  + gmTint * ${TINT_ROUGH.toFixed(3)},
          0.04, 1.0);

        // The bump, as THREE heights rather than one, because the bound below
        // is per octave. Their sum is the sum gmH always was — nothing about
        // the field changed — but each one's gradient is now bounded on its
        // own before they add, which is what BUMP_MAX_SLOPE's own comment has
        // always claimed the budget was ("the sum of what the three octaves
        // ask for") and what one clamp on the total could never enforce.
        float gmHMeso  = gmBump * uSurface * (gmMeso - 0.5) * 2.0 * uOct.x * gmFadeMeso;
        float gmHMicro = gmBump * uSurface * (gmMicro - 0.5) * 2.0 * uOct.y * gmFadeMicro;
        float gmHGrain = gmBump * gmGrain * uGrainAmp.x;

        // Clamped at 1.0 as well as at 0.0, which it was not before the base
        // maps: an albedo is a reflectance and cannot exceed unity, and a
        // photograph's bright grains delivered through a mean-preserving gain
        // reach for it in a way four multiplicative octaves around an authored
        // constant never did. Physical, not a taste — and it bounds the one
        // quantity in this shader whose upper end is now set by a JPEG.
        diffuseColor.rgb *= clamp(gmAlbedo, 0.0, 1.0);
        `,
      )
      .replace(
        "#include <roughnessmap_fragment>",
        `#include <roughnessmap_fragment>\n        roughnessFactor = gmRough;`,
      )
      // The bump, and where its gradient comes from.
      //
      // A procedural height gmH is turned into a normal perturbation. What
      // shipped reconstructed the gradient over dFdx(vGmPos)/dFdy(vGmPos) —
      // vectors spanning the TRIANGLE's flat plane — so the gradient came back
      // tangent to whichever facet the fragment sat on. Adjacent facets of a
      // smooth-shaded heightfield are tilted against each other while the
      // shading normal is not, so the bump jumped at every triangle edge by
      // (facet tilt) x (bump slope): a sawtooth locked to the mesh. Measured
      // on pass 20260802-050932-01's `04-ground-down.png`, scanning the sand
      // at y=500: a ~5-level ramp repeating on a ~58 px period with a hard
      // reset at each edge. The visual judge saw "the terrain triangulation
      // rendered as shading"; a blind reader called it "large low-poly facets
      // readable across the surface". The heightfield was never faceted. Only
      // the light on it was.
      //
      // Rebuilding the basis on the geometric normal is the textbook
      // correction and it is NOT the fix — measured in `ci/bump_basis.mjs` it
      // moves the seam 0.422° → 0.325°, because it still projects onto the
      // same tilted facet. Anything that reads the triangle inherits it.
      //
      // So nothing reads the triangle. gmH is a function of world XZ, so its
      // world gradient follows from the chain rule alone: solve the 2x2 system
      // mapping world-XZ steps to screen steps and invert it. What comes back
      // is dgmH/dx and dgmH/dz in metres per metre — the same two numbers from
      // either side of any edge, because they are a property of the field and
      // not of who asked. Exactly continuous (8.5e-7°), not approximately.
      //
      // Then the two heightfields add: the ground is y = T(x,z) and the bump
      // is gmH(x,z), so the perturbed normal is the normal of their sum, and
      // grad T recovers from the shading normal as -n.xz/n.y — no need to
      // reconstruct T itself.
      //
      // All of that happens in WORLD space, via vGmNorm, and the last line
      // rotates the result back. three's `normal` here is in VIEW space
      // (`normal_fragment_begin` is `normalize(vNormal)`, and vNormal rides the
      // normalMatrix, which is object→view). The retired block took
      // `cross(gmDpdy, normal)` with gmDpdy in world and normal in view — a
      // cross product between two spaces, which has no meaning and rotated the
      // bump as the camera turned. That was the second defect in these lines.
      // mat3(viewMatrix) is a rotation, so it cannot introduce a seam of its
      // own, which is why `ci/bump_basis.mjs` tests the world half and stops.
      .replace(
        "#include <normal_fragment_maps>",
        glsl`
        #include <normal_fragment_maps>
        // Bump: the world-XZ gradient of gmH, solved from screen derivatives,
        // added to the ground's own heightfield. Never reads the triangle.
        {
          vec2 gmDx = dFdx(vGmPos.xz);
          vec2 gmDy = dFdy(vGmPos.xz);
          vec3 gmHx = vec3(dFdx(gmHMeso), dFdx(gmHMicro), dFdx(gmHGrain));
          vec3 gmHy = vec3(dFdy(gmHMeso), dFdy(gmHMicro), dFdy(gmHGrain));
          float gmDet = gmDx.x * gmDy.y - gmDx.y * gmDy.x;
          // No branch: a derivative in non-uniform control flow is undefined,
          // so every dFdx above is taken first and this is a select. It drops
          // the bump where the XZ footprint degenerates — a face seen exactly
          // edge-on — which is where every octave has faded anyway.
          float gmInvDet = abs(gmDet) > 1e-14 ? 1.0 / gmDet : 0.0;
          // One inverse, three solves — the matrix is shared, so the second
          // and third octave cost four multiplies each and no fetch.
          vec3 gmSx = (gmHx * gmDy.y - gmHy * gmDx.y) * gmInvDet;
          vec3 gmSy = (gmHy * gmDx.x - gmHx * gmDy.x) * gmInvDet;
          // Each octave bounded at its OWN share of the budget, and that share
          // is derived rather than chosen: BUMP_MAX_SLOPE is documented as the
          // sum of what the three octaves reaching gmH ask for, so a third of
          // it is what one of them may ask for. Before this, one clamp on the
          // total meant the loudest octave could spend the whole budget and
          // the other two arrived as rounding — measured on rock, whose
          // per-identity bump of 2.2 multiplies every octave's slope after
          // the amplitudes were each chosen inside a 0.03-0.25 band that does
          // not include it, grain's bump alone was 44% of a near cliff's
          // neighbour contrast and the total saturated over 62.9% of the
          // frame. It is also invisible to every gate in this repo: at both
          // vantages the material probes measure, grain's bump contributes
          // x1.00 — it has faded out there — which is why eight passes could
          // not see it.
          vec2 gmSurf = vec2(0.0);
          for (int gmO = 0; gmO < 3; gmO++) {
            vec2 gmS = vec2(gmSx[gmO], gmSy[gmO]);
            float gmL = length(gmS);
            gmSurf += gmS * (min(gmL, ${(BUMP_MAX_SLOPE / 3).toFixed(4)}) / max(gmL, 1e-12));
          }
          // …and the photograph's own relief joins it below, already bounded
          // per plane by BASE_NORMAL_MAX_SLOPE and already zero at uBase = 0.
          // Both relief sources are now VOLUME GRADIENTS in world space —
          // gmH is a function of world XZ, so its gradient has no y term;
          // the photograph's is gmBaseGrad, which carries one because the
          // wall plane's v axis is world +y. They add, because they are the
          // same quantity in the same space, and the sum is the gradient of
          // the sum.
          vec3 gmGrad = vec3(gmSurf.x, 0.0, gmSurf.y) + gmBaseGrad;
          vec3 gmNw = normalize(vGmNorm);
          // The SURFACE gradient: the volume gradient with its normal
          // component removed, which is the only part of it a surface can
          // respond to. This is materials v5's correction and it is not
          // cosmetic (surface-gradient bump framework; the skill pack's
          // "stable coordinates" rule one derivative on).
          //
          // What it replaces treated the height field as if it were laid on
          // the shading plane and scaled the WHOLE perturbation by n.y, which
          // is wrong in both directions at once: a horizontal-plane field on a
          // tilted face is foreshortened ALONG the fall line and not at all
          // ACROSS it, so the retired form over-attenuated the across-slope
          // component and under-attenuated nothing. The projection does it
          // exactly and anisotropically, for free, and on level ground
          // (n = (0,1,0), dot = 0) it is the identity — the same bit-for-bit
          // property every other piece of this pass carries.
          vec3 gmSg = gmGrad - gmNw * dot(gmNw, gmGrad);
          // Bounded (wall 4), and now the bound MEANS something orientation-
          // independent: |surface gradient| is exactly the tangent of the
          // angle the shading normal is tilted through, on any face. The
          // retired bound was on the world-XZ gradient, whose relationship to
          // that angle changed with the face — so one number was a 29 deg
          // perturbation on flat ground and something else on every slope,
          // and it saturated over 68% of a near cliff against 0.6% here.
          float gmSlope = length(gmSg);
          gmSg *= min(gmSlope, ${BUMP_MAX_SLOPE.toFixed(2)}) / max(gmSlope, 1e-12);
          normal = normalize(mat3(viewMatrix) * normalize(gmNw - gmSg));
        }
        // Specular AA on what we just perturbed (procedural-materials
        // reference): variance of the shading normal widens the lobe instead
        // of letting it sparkle.
        {
          vec3 gmNx = dFdx(normal);
          vec3 gmNy = dFdy(normal);
          float gmVar = max(dot(gmNx, gmNx), dot(gmNy, gmNy));
          roughnessFactor = clamp(
            sqrt(roughnessFactor * roughnessFactor + gmVar * ${SPEC_AA.toFixed(3)}),
            0.0, 1.0);
        }
        `,
      );
  };
  // The ground is the biggest shadow RECEIVER in the frame, so it takes the
  // clipmap patch like everything else (shadow clipmap v0). Installed after
  // the splat patch above; it composes with it rather than replacing it.
  installClipmapShadows(material, variant.shadow);
  // …and it is the biggest CASTER, which it was not until this line existed.
  //
  // three derives the shadow pass's material from this one and, for a
  // FrontSide material, flips the side: `shadowSide[FrontSide] = BackSide`.
  // That default is the closed-mesh answer to acne — render the far side of a
  // solid so the near side never self-shadows — and it is exactly wrong for a
  // heightfield, which has one side. Every terrain triangle faces the sky,
  // the sun is in the sky, so every one of them was culled out of the depth
  // pass: hills cast NOTHING, near or far, and the only shadows in the frame
  // came from the closed geometry (scatter, pieces, players). Naming the side
  // explicitly is the fix; the acne the default was avoiding is what the
  // per-level normal bias above is for.
  material.shadowSide = THREE.FrontSide;
  // One program for every chunk: without this three re-runs onBeforeCompile
  // per material-instance key and can compile the same shader repeatedly.
  // The variant is part of the key or the probe's programs would collide with
  // the shipped one and it would time the same shader four times.
  material.customProgramCacheKey = () =>
    `gates-terrain-splat-v3-triplanar-grain-clipmap-${variantName}`;
  material.userData.uniforms = uniforms;
  // What this program costs, counted. `programStats` (the compiled source's
  // size) is filled by installClipmapShadows on first compile — it is the
  // last patch, so it measures what three actually hands the driver.
  material.userData.cost = {
    variant: variantName,
    field: variant.field,
    grain: variant.grain,
    tint: variant.tint,
    shadow: variant.shadow,
    // Depth fetches and noise samples per shaded fragment: the two things
    // `NOW.md` item 1 is trying to buy headroom from, both counted. The count
    // is SITES in the program, not evaluations — two of the four are behind
    // their own footprint fade, so a near fragment pays four and a far one
    // pays two. It is an upper bound, which is what a budget wants.
    depthFetches: clipmapFetches(variant.shadow),
    noiseSamples:
      variant.field === "off" ? 0 : 3 + grainTaps(variant.grain) + tintTaps(variant.tint),
    microSkipped: variant.field === "full",
    grainOn: variant.grain !== "off",
    tintOn: variant.tint !== "off",
    tintTaps: tintTaps(variant.tint),
    // Which projection the grain octave is sampled on, and what it costs in
    // taps. `triplanar` is what ships; `xz` is materials v1's, kept compiled
    // so the projection can be measured against itself rather than argued
    // about.
    grainProjection: projectionName(variant.grain),
    grainTaps: grainTaps(variant.grain),
    // The third counted axis. Texture fetches per shaded fragment, an UPPER
    // bound the way the noise count is one: the exact `gmW > 0.0` skip means a
    // fragment inside one biome pays a quarter of it. Counted and not timed,
    // deliberately — `NOW.md` item 9 records six cost-sweep runs on this box
    // that came back the wrong sign five times, and a seventh reading is not a
    // tiebreak.
    //
    // Two numbers since materials v4, because one of them stopped describing
    // most of the screen. `Level` is what a fragment gentler than 45° pays —
    // the wall block is skipped whole, so it is the pre-v4 count unchanged —
    // and `Max` is what a face steeper than that pays at a four-way splat
    // boundary. Reporting only the ceiling would have said the projection cost
    // 67% more everywhere, and it costs that nowhere a player stands.
    baseTexFetches: BASE_FETCHES_MAX,
    baseTexFetchesLevel: BASE_FETCHES_LEVEL,
    baseProjection: "biplanar",
    baseTexUnits: 3,
  };
  return material;
}

/** Build one of each cost variant. `costProbe` owns the lifetime. */
export function makeTerrainCostVariants() {
  return COST_VARIANTS.map((name) => makeTerrainMaterial(name));
}

/**
 * Build the projection partner. `projectionProbe` owns the lifetime, and it
 * is built on that probe's first ask — never on a public shard, and never in
 * the cost sweep, which is asking a different question.
 */
export function makeTerrainProjectionVariant() {
  return makeTerrainMaterial(PROJECTION_VARIANT);
}

// --- prop surfaces v0: the ladder, extended off the ground -------------------
//
// Everything above this line is the GROUND's material. Every other object in
// the world — the boulder, the trunk, the canopy, the wall, the door — went
// through `surfaceMaterial` below and got a roughness, a metalness and a flat
// colour, and nothing else. The visual judge's pass 20260802-163821-02 named
// exactly that as its ranked gap 1 and measured it: a 4,386-pixel boulder facet
// in `05-held-level.png` at luma sd 0.96, four widely separated sample points
// across two frames all returning the identical byte triple, against
// `spawnedrock.jpg`'s rock interior at sd 26.31 and `gameplayfoundbase.jpeg`'s
// log wall at 18.18 — 27x and 20x our spread on the same objects at the same
// apparent size. Its sentence for the consequence is the one that matters:
// *you cannot tell our wood from our stone by surface, only by silhouette and
// hue*, and "no amount of further terrain work reaches criterion 2 without
// this".
//
// So the field the ground already has is extended to them. Three things are
// deliberately NOT copied from the ground's version:
//
//   1. **Triplanar, not world-XZ.** A prop has no UVs and is not a
//      heightfield; a boulder's overhang and a trunk's flank are exactly the
//      faces a top-down projection smears. Same three-tap blend the grain
//      octave already uses (`gmGrainTri`), same `/length(w)` restoration of
//      the deviation the convex blend removes.
//   2. **The gradient is ANALYTIC, not reconstructed.** The ground solves its
//      bump gradient out of `dFdx`/`dFdy`, and the pass before this one proved
//      what that costs: `dFdx` is a difference ACROSS the rasterizer's 2x2
//      quad, so a reconstructed gradient is constant inside a quad by
//      construction and steps at the boundary — measured at 1.9 luma/px of
//      neighbour contrast within a quad against 21.4 across one, which is the
//      per-pixel dither the blind reader called out in 5 of 6 frames
//      (`DECISIONS.md` §open, "the quad-constant gradient"). Value noise with a
//      quintic fade has an exact derivative and the four corner hashes are
//      already in hand, so this field carries its own gradient out of the same
//      taps. It cannot be quad-locked: there is no derivative in it.
//   3. **Two octaves, and each retires on the sampling law**
//      (`FADE_OCTAVE_CPP`, cycles per pixel — the law wall 1 of the octave
//      table is written against). Bump and albedo retire on the SAME band, and
//      `browser_smoke` 15f fails the build if they ever differ; the argument
//      for halving the bump's band was made, measured and rejected, and it is
//      recorded where the constant is (`PROP_BUMP_FADE_CPP` below).
//
// What separates a rock from a log here is STRUCTURE, not amplitude — the
// point of the gap. `ridge` folds the field toward its own ridge line
// (`1 - |2n-1|`), which turns a blob field into a crack network; `crevice`
// darkens the low side of that fold, so the cracks read as depth rather than
// as paint; and `scale` is a per-axis vec3, so wood's field is stretched along
// world Y and its fissures run UP the trunk the way bark does.
const PROP_FADE_CPP = FADE_OCTAVE_CPP;
// The bump's band is the ALBEDO's band, and that is one law rather than two.
//
// The first cut of this pass halved it, on the argument that a normal is one
// derivative up from the value that generates it. It is a real argument and it
// is not this material's problem: measured on the gate's own pine view at
// 4.9 m, the halved band retired the detail octave's RELIEF to 7% of strength
// while its albedo was still at 90%, and the frame's neighbour contrast came
// back x1.13 against the x1.30 floor — a canopy whose colour varied and whose
// surface did not. Per-pixel detail at arm's length IS the relief; retiring it
// at half the distance the colour survives is retiring the thing this pass
// exists to add.
//
// What handles the sparkle the halved band was insurance against is the same
// thing that handles it on the ground: `SPEC_AA`, which rolls the shading
// normal's own variance into roughness (Toksvig) and is applied to exactly the
// perturbation this field adds. The ground's bump runs the full band under it
// and does not sparkle.
//
// It is also materials v2's own lesson, which unified every ground octave onto
// one cycles-per-pixel law after two hand-derived metre thresholds were both
// wrong in the same direction: "the law is not grain's, it is the field's".
// Two bands is two thresholds to get wrong.
const PROP_BUMP_FADE_CPP = FADE_OCTAVE_CPP;
// The detail octave is the coarse one times this. 4.0 is two octaves of
// separation — far enough that the two do not beat against each other, close
// enough that the detail is still gone at a distance the coarse one survives
// (it retires at exactly a quarter of the coarse octave's footprint).
const PROP_DETAIL_MUL = 4.0;
// How much of the coarse octave's amplitude the detail octave carries.
const PROP_DETAIL_SHARE = 0.6;
// The field's nominal [0,1] maps to a signed swing through this gain, the same
// argument `TINT_GAIN` carries: value noise does not spend its time at its
// nominal range (measured deviation about the midpoint 0.2262), so a gain of 2
// would make the authored contrast a place the material never goes. 2.5 puts
// the mean at about half the authored pole.
const PROP_GAIN = 2.5;
// A chromatic deviation is authored as a direction and then has its luminance
// projected OUT, so it is exactly neutral by construction rather than neutral
// to three decimal places by hand.
//
// Its MAGNITUDE is set against what the deviation actually reaches: it lands as
// `dev * gpV * gpBase`, where `gpBase` is the surface's own luminance (so a
// dark canopy and a pale rock each get a deviation proportionate to what they
// are, rather than the canopy getting a wash). At rock's linear albedo of
// ~0.27 and the field's measured |gpV| of ~0.55, a |dev| of 0.08 moves
// chromaticity by ~3%, and the first cut of this pass shipped exactly that and
// measured it: the prop probe's chroma spread went x1.09 against a x1.10 floor
// — a hue octave that had to be looked for. The authored lengths are ~0.2, the
// band the ground's four identities already sit in (off-colour 0.036–0.152),
// which puts the deviation at ~11% of albedo and the measured spread at x1.4.
//
// Materials v3 earned the neutrality law on the ground

// (`TINT_LUMA_NEUTRAL`): three scalar octaves already move VALUE, nothing moved
// HUE, and a hue octave that also swings value spends the surface probe's
// two-sidedness. Same division of labour here.
const lumaNeutral = (v) => {
  const l = devLuma(v);
  return [v[0] - l, v[1] - l, v[2] - l];
};

// --- prop albedo v1: the band an authored colour has to sit in ---------------
//
// Prop surfaces v0 gave every class a field. It multiplies: the shader's own
// term is `diffuseColor.rgb * (1 + gpV * contrast) * (1 - crevice)`, so what
// the field is WORTH in the delivered image is a share of whatever the surface
// was already delivering. On a face delivering luma 120 a +-14% swing is +-17
// levels and reads as granite; on a face delivering luma 6 the identical field
// is +-0.8 levels and reads as nothing. The visual judge measured the second
// case on the merged frames and named it precisely — "the amplitude is the bug,
// not the absence", best-fit-plane residual 1.23/255 over 7,800 px — while
// every gate this repo had scored the first, because every prop assertion in
// `browser_smoke` is a RATIO and a ratio cannot tell those two apart.
//
// Half of that delivery is the light rig and is not this file's (see the
// §open row, "prop albedo v1", for the arithmetic and who owns the rest). The
// half that IS this file's is the authored albedo, and two of the scatter
// table's parts sat below the band where any light this scene has can carry a
// field at all:
//
//   pine trunk  linear luminance 0.0265 -> 0.0570
//   pine skirt                   0.0453 -> 0.1344
//
// Dry conifer bark measures 8-12% visible reflectance and conifer needles
// 6-15%; 0.0265 is charcoal, and nothing in a forest is charcoal. The floor
// is therefore not a taste, it is the bottom of the dielectric band that real
// outdoor materials occupy, and the ceiling is its top (fresh gypsum sand and
// white paint are the brightest natural things outdoors at ~0.55; snow is out
// of scope until there is a snow biome). Both are stated in `DECISIONS.md`
// §open rather than invented here.
//
// The correction is applied by SCALING the part in linear, both ends of its
// ramp together, so hue and the ramp's shape are preserved exactly — a scalar
// multiply leaves chromaticity where it found it, which is the same arithmetic
// `TINT_LUMA_NEUTRAL` is written against, used in the other direction.
export const ALBEDO_LUMA_BAND = [0.05, 0.55];

const _albedo = new THREE.Color();
/**
 * The linear luminance an authored sRGB hex actually reaches the shader with.
 *
 * `setHex(…, SRGBColorSpace)` is the renderer's own conversion — the same one
 * every material colour and the sky dome get — so this is what the fragment
 * multiplies, not an approximation of it.
 */
export function albedoLuma(hex) {
  _albedo.setHex(hex, THREE.SRGBColorSpace);
  return 0.2126 * _albedo.r + 0.7152 * _albedo.g + 0.0722 * _albedo.b;
}

// --- authored identities for everything that is not the ground -------------
// Per-surface roughness/metalness bundles, so a stone wall, a wooden door and
// a metal sheet answer the same key light differently. Boxes and cones until
// there are models; the RESPONSE is what makes the tier read (bases.webp) —
// and now `field`, which is what makes the SURFACE read.
//
// Every `field` number is authored against one of two bands the file already
// uses. Peak bump slope is `2*PI * amp * scale` (a sinusoid's peak slope, the
// convention the quad-constant-gradient row records the ground's table as
// understating by that same 2*PI), and every class lands inside the 0.03–0.25
// band the ground's octaves were chosen against.
//
// `scale` is in cycles per metre per world axis, so `1/scale` is the feature's
// own size — and the law for picking it is the OBJECT's size, not the ground's.
// The coarse octave's wavelength is about a third of the thing it sits on (a
// 1.0 m crack network on a 3 m canopy, a 0.8 m plate on a 2.5 m boulder, a
// 0.5 m bark fissure around a 0.4 m trunk), and the detail octave is a quarter
// of that. Both halves of that are load-bearing and the first cut of this pass
// got them wrong in the same direction:
//
//   · A field authored at the ground's frequencies — foliage at 5.5 /m, rock
//     at 3.1 /m — retires at `0.09 / scale` metres per pixel, which at this
//     frustum is **7.7 m for the canopy**. Measured, not reasoned: the gate's
//     own pine view sits 10 m out and scored 0.00% of the frame moved, on the
//     class with the strongest field in the table. A prop is looked at from
//     across a clearing; the ground is looked at from underfoot. Same law,
//     different distances, so different frequencies.
//   · An octave one third of its object also *varies inside* the object, which
//     the terrain's own macro octave does not and which cost materials v3 two
//     rebuilt terms: a wavelength wider than the frame is a colour cast, not a
//     variation. Here the frame is the prop.
//
// The retirement distances that fall out, at 75° vFOV / 720 rows: canopy 42 m,
// boulder 34 m, bark 21 m, ore 28 m. That is the band the visual report is
// about — "simplified cards in the mid-ground and dark specks on the far
// ridge" — rather than arm's length.
//
// `field: null` is the deliberate absence — water owns its own look and is the
// one surface here that is not a solid. The shader branches on it, so the
// ocean plane pays none of this.
export const SURFACES = {
  // Bark: fissures across the grain at ~0.38 m, running 4.7x that far up the
  // trunk. The strongest ridge in the table — a bark fissure is a crease.
  wood: {
    roughness: 0.95,
    metalness: 0.0,
    field: {
      scale: [2.0, 0.42, 2.0],
      ridge: 0.85,
      contrast: 0.26,
      crevice: 0.35,
      bump: 0.0143,
      rough: 0.16,
      dev: lumaNeutral([0.145, 0.03, -0.12]),
    },
  },
  // Built stone: coarser and blockier than a boulder, because a wall is
  // courses and a boulder is one lump.
  stone: {
    roughness: 0.88,
    metalness: 0.0,
    field: {
      scale: [1.0, 1.0, 1.0],
      ridge: 0.55,
      contrast: 0.2,
      crevice: 0.28,
      bump: 0.0223,
      rough: 0.12,
      dev: lumaNeutral([0.11, 0.0, -0.13]),
    },
  },
  metal: {
    roughness: 0.42,
    metalness: 0.8,
    field: {
      scale: [1.2, 1.2, 1.2],
      ridge: 0.25,
      contrast: 0.1,
      crevice: 0.12,
      bump: 0.008,
      rough: 0.1,
      dev: lumaNeutral([0.095, 0.0, -0.095]),
    },
  },
  // Canopy: the highest contrast in the table and nearly the finest scale.
  // A cone reads as a cone because its surface is one value; needle clumping
  // is the cheapest thing that stops it, and it is albedo, not geometry.
  foliage: {
    roughness: 0.86,
    metalness: 0.0,
    field: {
      scale: [1.0, 1.0, 1.0],
      ridge: 0.35,
      contrast: 0.34,
      crevice: 0.5,
      bump: 0.0239,
      rough: 0.08,
      dev: lumaNeutral([0.13, 0.06, -0.22]),
    },
  },
  // Granite: a crack network with dark pits. Highest crevice in the table —
  // `spawnedrock.jpg`'s rock is mostly holes.
  rock: {
    roughness: 0.85,
    metalness: 0.0,
    field: {
      scale: [1.25, 1.25, 1.25],
      ridge: 0.7,
      contrast: 0.24,
      crevice: 0.42,
      bump: 0.0229,
      rough: 0.14,
      dev: lumaNeutral([0.12, 0.0, -0.15]),
    },
  },
  // Ore: rock's structure at a finer scale with a metallic fleck — the
  // contrast is high and the ridge is low, so it reads as inclusions rather
  // than as cracks.
  ore: {
    roughness: 0.55,
    metalness: 0.45,
    field: {
      scale: [1.5, 1.5, 1.5],
      ridge: 0.45,
      contrast: 0.3,
      crevice: 0.3,
      bump: 0.0191,
      rough: 0.18,
      dev: lumaNeutral([0.17, -0.02, -0.12]),
    },
  },
  cloth: {
    roughness: 0.78,
    metalness: 0.0,
    field: {
      scale: [2.0, 2.0, 2.0],
      ridge: 0.2,
      contrast: 0.12,
      crevice: 0.1,
      bump: 0.008,
      rough: 0.06,
      dev: lumaNeutral([0.07, 0.0, -0.07]),
    },
  },
  water: { roughness: 0.14, metalness: 0.0, field: null },
};

// The probe's handle on the whole prop field, shared by every material the
// factory below builds — one uniform OBJECT, the way `activeLevels` is shared
// across every clipmap patch, so the toggle moves the whole scene in one write
// and no two props can disagree about whether they have a surface. It ships at
// 1: a probe input, not a quality setting, and the gate reads the live value
// back to prove it.
export const propToggle = { value: 1 };

// Value noise with its exact derivative, from the four corner hashes the value
// already costs. `u` is the quintic fade the ground's `gmNoise` uses and `du`
// is its derivative, so this returns the SAME field — `gpNoiseD(p).x` is
// `gmNoise(p)` to the bit — plus the two partials that come free with it.
//
// This is the whole reason a prop's bump is not the ground's dither: nothing
// here is a screen derivative, so nothing here can be constant across a 2x2
// quad.
const PROP_NOISE_GLSL = /* glsl */ `
vec3 gpNoiseD(vec2 p) {
  vec2 i = floor(p);
  vec2 f = fract(p);
  vec2 u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
  vec2 du = 30.0 * f * f * (f * (f - 2.0) + 1.0);
  float a = gmHash(i);
  float b = gmHash(i + vec2(1.0, 0.0));
  float c = gmHash(i + vec2(0.0, 1.0));
  float d = gmHash(i + vec2(1.0, 1.0));
  float k1 = b - a;
  float k2 = c - a;
  float k3 = a - b - c + d;
  return vec3(a + k1 * u.x + k2 * u.y + k3 * u.x * u.y,
              (k1 + k3 * u.y) * du.x,
              (k2 + k3 * u.x) * du.y);
}
// The ridge fold, carried through to the derivative: r = 1 - |2n-1| has
// dr = -2*sign(2n-1)*dn everywhere except the ridge line itself, which is a
// measure-zero set the rasterizer never lands on exactly.
vec3 gpFold(vec3 n, float ridge) {
  float s = (2.0 * n.x - 1.0) >= 0.0 ? 1.0 : -1.0;
  return vec3(mix(n.x, 1.0 - abs(2.0 * n.x - 1.0), ridge),
              mix(n.y, -2.0 * s * n.y, ridge),
              mix(n.z, -2.0 * s * n.z, ridge));
}
// Three planar taps blended by the face's own normal, returning the signed
// value and its WORLD gradient. Each plane's 2D partials are lifted into the
// two world axes that plane spans, and the whole thing is multiplied by the
// per-axis scale on the way out (chain rule: the taps are taken in q = p * s).
//
// The fold happens per plane, before the blend — a ridged field averaged three
// ways is not a ridged field — and /length(w) restores the deviation the
// convex blend removed, both for the same reasons gmGrainTri states them.
vec4 gpTri(vec3 p, vec3 w, vec3 s, float ridge) {
  vec3 q = p * s;
  vec3 g = vec3(0.0);
  float v = 0.0;
  vec3 t = gpFold(gpNoiseD(q.zy), ridge);
  v += t.x * w.x;
  g += vec3(0.0, t.z, t.y) * w.x;
  t = gpFold(gpNoiseD(q.xz), ridge);
  v += t.x * w.y;
  g += vec3(t.y, 0.0, t.z) * w.y;
  t = gpFold(gpNoiseD(q.xy), ridge);
  v += t.x * w.z;
  g += vec3(t.y, t.z, 0.0) * w.z;
  float inv = 1.0 / max(length(w), 1e-4);
  return vec4((v - 0.5) * inv, g * s * inv);
}
`;

const PROP_VERT_PARS = /* glsl */ `
varying vec3 vGpPos;
varying vec3 vGpNrm;
`;

// The world position and world normal this field is a function of. Both are
// computed here rather than borrowed from three's `worldPosition` (which only
// exists behind a `#if defined(USE_SHADOWMAP) || ...` this file does not own)
// and both apply `instanceMatrix` by hand, because three applies it inside
// `project_vertex` and `defaultnormal_vertex` and neither leaves a world-space
// result behind. Every scatter pool is an InstancedMesh, so a patch that
// forgot this would give all 4,096 trees in a pool the same square metre of
// bark.
const PROP_VERT_GLSL = /* glsl */ `
  vec3 gpNrm = normal;
  vec4 gpWp = vec4(transformed, 1.0);
  #ifdef USE_INSTANCING
    gpNrm = mat3(instanceMatrix) * gpNrm;
    gpWp = instanceMatrix * gpWp;
  #endif
  vGpNrm = normalize(mat3(modelMatrix) * gpNrm);
  vGpPos = (modelMatrix * gpWp).xyz;
`;

const PROP_FRAG_PARS = /* glsl */ `
varying vec3 vGpPos;
varying vec3 vGpNrm;
uniform float uProp;
uniform vec3 uPropScale;
uniform vec4 uPropShape;
uniform vec3 uPropBump;
uniform vec3 uPropDev;
uniform vec4 uPropFade;
${FIELD_GLSL}
${PROP_NOISE_GLSL}
`;

// The field, evaluated once per fragment and left in `gpV` (signed value) and
// `gpGrad` (world gradient) for the roughness and normal stages below — three
// runs its chunks in one main(), so locals carry.
//
// The whole block sits behind a UNIFORM branch, which is coherent across every
// fragment of a draw call: water skips it entirely, and the probe's `uProp = 0`
// state costs what a material without a field costs rather than what a
// material with a zeroed one does. The one derivative in the patch —
// `fwidth(vGpPos)`, the pixel footprint every fade is measured in — is taken
// OUTSIDE it, because a derivative in non-uniform control flow is undefined
// and a uniform branch is only uniform until someone edits the condition.
const PROP_FRAG_GLSL = /* glsl */ `
  float gpV = 0.0;
  vec3 gpGrad = vec3(0.0);
  float gpFw = max(length(fwidth(vGpPos)), 1e-5);
  vec3 gpN = normalize(vGpNrm);
  if (uProp * uPropShape.y > 0.0) {
    vec3 gpW = abs(gpN);
    gpW /= max(gpW.x + gpW.y + gpW.z, 1e-4);
    // Cycles per pixel, off the coarsest axis of the scale: an anisotropic
    // field resolves as badly as its finest direction, so the fade is driven
    // by the largest component and the stretched axis rides along.
    float gpCpp = gpFw * max(max(uPropScale.x, uPropScale.y), uPropScale.z);
    float gpAlbC = 1.0 - smoothstep(uPropFade.x, uPropFade.y, gpCpp);
    float gpAlbD = 1.0 - smoothstep(uPropFade.x, uPropFade.y, gpCpp * ${PROP_DETAIL_MUL.toFixed(1)});
    float gpBmpC = 1.0 - smoothstep(uPropFade.z, uPropFade.w, gpCpp);
    float gpBmpD = 1.0 - smoothstep(uPropFade.z, uPropFade.w, gpCpp * ${PROP_DETAIL_MUL.toFixed(1)});
    vec4 gpC = gpTri(vGpPos, gpW, uPropScale, uPropShape.x);
    vec4 gpD = gpTri(vGpPos, gpW, uPropScale * ${PROP_DETAIL_MUL.toFixed(1)}, uPropShape.x);
    gpV = (gpC.x * gpAlbC + gpD.x * uPropShape.z * gpAlbD) * ${PROP_GAIN.toFixed(2)};
    gpGrad = (gpC.yzw * gpBmpC + gpD.yzw * uPropShape.z * gpBmpD) * uPropBump.x;

    // Albedo. Two terms and they do different jobs: a scalar swing that moves
    // VALUE, and a luminance-neutral deviation that moves HUE — the division
    // materials v3 established on the ground. The crevice is the ridge fold's
    // low side and only ever darkens, which is what makes a crack read as
    // depth; it is scaled by the coarse octave's own fade so a distant prop
    // does not keep a paint-on crack after the geometry that justified it has
    // gone.
    float gpBase = dot(diffuseColor.rgb, vec3(0.2126, 0.7152, 0.0722));
    float gpCrev = clamp(-gpV, 0.0, 1.0) * uPropShape.w * gpAlbC;
    diffuseColor.rgb = max(
      diffuseColor.rgb * ((1.0 + gpV * uPropShape.y) * (1.0 - gpCrev))
        + uPropDev * (gpV * gpBase),
      0.0);
  }
`;

// Bounded (wall 4): a gradient has no upper bound of its own, and the cap is
// the ground's — same convention, same arithmetic, and no prop class in the
// table above asks for a tenth of it, so it holds only what a future edit
// cannot justify. The perturbation is the tangential part of the height
// gradient, which is the surface-gradient form of a bump on an arbitrary
// surface: a heightfield's `-grad h` is the special case where the surface
// normal is +Y.
const PROP_NORMAL_GLSL = /* glsl */ `
  {
    vec3 gpS = gpGrad - dot(gpGrad, gpN) * gpN;
    float gpLen = length(gpS);
    gpS *= min(gpLen, ${BUMP_MAX_SLOPE.toFixed(2)}) / max(gpLen, 1e-12);
    normal = normalize(normal - mat3(viewMatrix) * gpS);
    // Specular AA on what we just perturbed, the same term and the same gain
    // the ground's bump carries: variance of the shading normal widens the
    // lobe instead of letting a boulder sparkle at 40 m.
    vec3 gpNx = dFdx(normal);
    vec3 gpNy = dFdy(normal);
    float gpVar = max(dot(gpNx, gpNx), dot(gpNy, gpNy));
    roughnessFactor = clamp(
      sqrt(roughnessFactor * roughnessFactor + ${SPEC_AA.toFixed(2)} * gpVar),
      0.04, 1.0);
  }
`;

/**
 * A MeshStandardMaterial with one of the authored responses above, wired to
 * the shadow clipmap.
 *
 * Every standard material in the scene goes through here, which is the point:
 * the clipmap replaces three's one-map-per-light shadow term, so a material
 * that skipped this patch would be lit by N lights and shadowed by none of
 * them. `installClipmapShadows` hands each of them the same onBeforeCompile
 * source, so they still share one compiled program.
 */
export function surfaceMaterial(surface, opts = {}) {
  const s = SURFACES[surface];
  if (!s) throw new Error(`unknown surface identity: ${surface}`);
  const material = new THREE.MeshStandardMaterial({
    roughness: s.roughness,
    metalness: s.metalness,
    ...opts,
  });
  // Prop surfaces v0. One patch source for every class — the per-class table
  // arrives as UNIFORMS, never as generated GLSL — because three keys its
  // program cache on `onBeforeCompile.toString()`, so a patch that differed
  // per class would compile one program per class and every one of them would
  // be a mid-play link the prewarm gate is written to catch. Water carries
  // `field: null` and still takes the patch: it is the same source, the same
  // program, and `uPropShape.y = 0` closes the branch at zero cost.
  const f = s.field;
  const uniforms = {
    uProp: propToggle,
    uPropScale: { value: new THREE.Vector3(...(f ? f.scale : [1, 1, 1])) },
    uPropShape: {
      value: new THREE.Vector4(
        f ? f.ridge : 0,
        f ? f.contrast : 0,
        PROP_DETAIL_SHARE,
        f ? f.crevice : 0,
      ),
    },
    uPropBump: { value: new THREE.Vector3(f ? f.bump : 0, f ? f.rough : 0, 0) },
    uPropDev: { value: new THREE.Vector3(...(f ? f.dev : [0, 0, 0])) },
    uPropFade: {
      value: new THREE.Vector4(
        PROP_FADE_CPP[0],
        PROP_FADE_CPP[1],
        PROP_BUMP_FADE_CPP[0],
        PROP_BUMP_FADE_CPP[1],
      ),
    },
  };
  material.userData.propUniforms = uniforms;
  material.userData.propSurface = surface;
  material.onBeforeCompile = (shader) => {
    Object.assign(shader.uniforms, uniforms);
    shader.vertexShader = shader.vertexShader
      .replace("#include <common>", `#include <common>\n${PROP_VERT_PARS}`)
      .replace(
        "#include <project_vertex>",
        `#include <project_vertex>\n${PROP_VERT_GLSL}`,
      );
    shader.fragmentShader = shader.fragmentShader
      .replace("#include <common>", `#include <common>\n${PROP_FRAG_PARS}`)
      // AFTER the include, never instead of it: a prop's vertex colours are
      // its trunk/skirt/crown ramp and its per-instance tint, and the field
      // multiplies what the object already is.
      .replace(
        "#include <color_fragment>",
        `#include <color_fragment>\n${PROP_FRAG_GLSL}`,
      )
      .replace(
        "#include <roughnessmap_fragment>",
        `#include <roughnessmap_fragment>
        roughnessFactor = clamp(roughnessFactor + gpV * uPropBump.y, 0.04, 1.0);`,
      )
      .replace(
        "#include <normal_fragment_maps>",
        `#include <normal_fragment_maps>\n${PROP_NORMAL_GLSL}`,
      );
  };
  return installClipmapShadows(material);
}

/**
 * The prop field's structural facts, for the browser gate to assert. Derived
 * from the table rather than restated beside it, so a class that gains a row
 * or loses a fade cannot pass by agreeing with a copy of itself.
 */
export function propFacts() {
  const classes = Object.entries(SURFACES).map(([name, s]) => {
    const f = s.field;
    if (!f) return { name, field: false };
    const sMax = Math.max(...f.scale);
    return {
      name,
      field: true,
      scale: f.scale,
      // Cycles per metre on the coarsest axis, and the finest feature the
      // material carries — the detail octave's own.
      scaleMax: sMax,
      detailScaleMax: sMax * PROP_DETAIL_MUL,
      ridge: f.ridge,
      contrast: f.contrast,
      crevice: f.crevice,
      bumpM: f.bump,
      roughSwing: f.rough,
      dev: f.dev,
      // A sinusoid's peak slope, which is the convention every slope claim in
      // this file is supposed to be in (the quad-constant-gradient row records
      // the ground's table as 2*PI off it).
      peakSlope: 2 * Math.PI * f.bump * sMax,
      // Zero by construction — the deviation has its luminance projected out.
      // Published so the gate reads a number instead of trusting the sentence.
      devLumaResidual: Math.hypot(...f.dev) > 0 ? devLuma(f.dev) / Math.hypot(...f.dev) : 0,
    };
  });
  return {
    classes,
    fadeCpp: PROP_FADE_CPP,
    bumpFadeCpp: PROP_BUMP_FADE_CPP,
    detailMul: PROP_DETAIL_MUL,
    detailShare: PROP_DETAIL_SHARE,
    gain: PROP_GAIN,
    nyquistCpp: NYQUIST_CPP,
    bumpMaxSlope: BUMP_MAX_SLOPE,
    lumaNeutral: TINT_LUMA_NEUTRAL,
    // The AA term that pays for running the bump to the full fade band. The
    // two are one decision, so the gate asserts them together.
    specAA: SPEC_AA,
    // Taps per shaded prop fragment: two octaves, three planar taps each.
    // Priced in the same currency the ground's budget is (`NOISE_SAMPLE_BUDGET`
    // in the browser gate) so the two are comparable rather than each having
    // its own unit.
    noiseSamples: 6,
    toggle: propToggle.value,
    // Prop albedo v1's law, published for the gate to assert the scatter
    // table against. It lives on the prop facts and not on the scatter facts
    // because it is a statement about the MATERIAL system — the field this
    // module adds is multiplicative, so the band is the condition under which
    // that field is worth anything.
    albedoBand: ALBEDO_LUMA_BAND,
  };
}

/** The material system's structural facts, for the browser gate to assert. */
export function materialFacts() {
  return {
    identities: IDENTITIES.map((i) => i.name),
    identityRoughness: IDENTITIES.map((i) => i.roughness),
    identityBump: IDENTITIES.map((i) => i.bump),
    surfaces: Object.fromEntries(
      Object.entries(SURFACES).map(([k, v]) => [k, [v.roughness, v.metalness]]),
    ),
    props: propFacts(),
    breakup: BLEND_BREAKUP,
    specAA: SPEC_AA,
    fadeMicroM: FADE_MICRO,
    // The octave table, so the gate can assert the sampling law over ALL of
    // them rather than spot-checking the one that happened to have a fact.
    // `scale` is cycles/m, `fadeCpp` is where the octave starts and finishes
    // retiring in cycles per pixel, and `fadeM` is that same pair in the
    // metres-per-pixel the shader actually compares against — derived, so a
    // future hand-written metre threshold shows up here as a cpp past
    // Nyquist instead of hiding as a plausible-looking distance.
    octaves: [
      { name: "macro", scale: SCALE_MACRO, fadeCpp: null, fadeM: null },
      { name: "meso", scale: SCALE_MESO, fadeCpp: FADE_OCTAVE_CPP, fadeM: FADE_MESO },
      { name: "micro", scale: SCALE_MICRO, fadeCpp: FADE_OCTAVE_CPP, fadeM: FADE_MICRO },
    ],
    nyquistCpp: NYQUIST_CPP,
    bumpMaxSlope: BUMP_MAX_SLOPE,
    // The second pass. Scales in cycles/m, so the gate can check they are
    // genuinely finer than the micro octave (0.588 /m) as well as distinct.
    grainScale: IDENTITIES.map((i) => i.grain.scale),
    grainContrast: IDENTITIES.map((i) => i.grain.contrast),
    grainRidge: IDENTITIES.map((i) => i.grain.ridge),
    grainFadeCpp: FADE_GRAIN_CPP,
    grainAmpM: AMP_GRAIN_M,
    grainRough: GRAIN_ROUGH,
    microScale: SCALE_MICRO,
    // The fourth pass. Scales in cycles/m so the gate can place this octave in
    // the ladder — coarser than every grain, finer than the micro octave —
    // rather than take "0.5–1 m" on the comment's word; `tintDev` so it can
    // check the four deviations are neither parallel to their own colours (a
    // brightness multiply wearing a texture's name) nor to each other.
    tintScale: IDENTITIES.map((i) => i.tint.scale),
    tintDev: IDENTITIES.map((i) => i.tint.dev),
    tintFadeCpp: FADE_TINT_CPP,
    tintGain: TINT_GAIN,
    tintRough: TINT_ROUGH,
    // Each deviation's luminance, as a share of its own length. Published
    // rather than asserted here so the gate owns the bar; the ceiling it is
    // checked against is `TINT_LUMA_NEUTRAL`.
    tintLumaResidual: IDENTITIES.map(
      (i) => devLuma(i.tint.dev) / Math.hypot(...i.tint.dev),
    ),
    tintLumaNeutral: TINT_LUMA_NEUTRAL,
    identityColor: IDENTITIES.map((i) => i.color),
    // The third pass. Read off the shipped variant's own config rather than
    // written down beside it, so the fact cannot drift from the program: if
    // `ship` ever goes back to world XZ, this says so and the gate fails.
    grainProjection: projectionName(VARIANT_CONFIG.ship.grain),
    grainTaps: grainTaps(VARIANT_CONFIG.ship.grain),
    // …and what the partner it is measured against is sampled on.
    flatGrainProjection: projectionName(VARIANT_CONFIG[PROJECTION_VARIANT].grain),
    // The fifth pass (ART.md §7). `loaded` is false rather than absent when
    // the maps did not arrive, so the gate asserts a fact instead of failing
    // to find one — the difference between a wall and a wall that passes by
    // matching nothing.
    base: baseFacts(),
  };
}

/**
 * Each layer's per-channel gain span, `max / min` over `color / measured mean`.
 *
 * One number per layer, and it is the diagnosis rather than a summary of it: a
 * span of 1 is a source whose colour already agrees with its palette entry, so
 * correcting its mean is a pure scale and its deviation arrives unstretched. A
 * span of 5.72 — `rock` — is a source being dragged across channels, and the
 * same multiply that fixes its mean multiplies its per-channel NOISE by the
 * same uneven factors.
 */
function baseGainSpan(tex) {
  return IDENTITIES.map((id, i) => {
    const g = id.color.map((c, ch) => c / Math.max(tex.meanLinear[i][ch], 1e-4));
    return Math.max(...g) / Math.max(Math.min(...g), 1e-6);
  });
}

/**
 * How much of each photograph's colour deviation the shader keeps.
 *
 * `min(1, BASE_CHROMA_STRETCH_MAX / span)` — the effective stretch applied to
 * a layer's chroma deviation is its span times its keep, so this holds that
 * product at or below the ceiling for every layer and leaves a layer already
 * under the ceiling entirely alone. Nothing here is per-identity taste: the
 * only input is the span the source files themselves measure, so swapping a
 * better-matched `rock` file raises its keep automatically and swapping a
 * worse one lowers it, with no edit here and none to `content/`.
 */
function baseChromaKeep(tex) {
  return baseGainSpan(tex).map((span) =>
    Math.min(1, BASE_CHROMA_STRETCH_MAX / Math.max(span, 1e-6)),
  );
}

/**
 * What the base maps are and what they deliver, for the gate to assert.
 *
 * `albedoGain` is the whole hybrid policy as four numbers: it is
 * `IDENTITIES[i].color / measured mean`, so a gain far from 1 is a source
 * sitting off `ART.md` §3's band being pulled onto it — `MANIFEST.md` predicts
 * exactly that for `rock`, and this is where the prediction becomes checkable.
 */
export function baseFacts() {
  const tex = groundTextures();
  if (!tex) {
    return { loaded: false, layers: GROUND_LAYERS.slice(), scale: BASE_SCALE };
  }
  return {
    loaded: true,
    ...tex.facts,
    scale: BASE_SCALE,
    // Read off IDENTITIES the same way the uniform is, so a fact and a uniform
    // cannot disagree about what the shader was handed.
    albedoGain: IDENTITIES.map((id, i) =>
      id.color.map((c, ch) => c / Math.max(tex.meanLinear[i][ch], 1e-4)),
    ),
    roughGain: IDENTITIES.map((id, i) => id.roughness / Math.max(tex.meanRough[i], 1e-4)),
    // The gain's SPAN per layer — `max/min` over its three channels — and the
    // chroma keep that falls out of it. The span is the whole diagnosis in one
    // number per layer: 1.0 is a source that already agrees with its palette
    // entry and needs no correction beyond a scale, and `rock`'s 5.72 is a
    // sandstone standing in for granite whose blue mean sits near its own
    // noise floor. Published as a pair so the gate can assert the derivation
    // rather than the result — a keep that stops tracking its span is a knob
    // that has quietly become a taste setting.
    albedoGainSpan: baseGainSpan(tex),
    chromaKeep: baseChromaKeep(tex),
    chromaStretchMax: BASE_CHROMA_STRETCH_MAX,
    normalMaxSlope: BASE_NORMAL_MAX_SLOPE,
    fadeCpp: FADE_BASE_CPP,
    fadeM: FADE_BASE_CPP.map((cpp) => BASE_SCALE.map((s) => cpp / s)),
    fetchesMax: BASE_FETCHES_MAX,
    fetchesLevel: BASE_FETCHES_LEVEL,
    // The projection, and the face tilt at which its second tap turns on —
    // published as the cosine as well, because `BASE_WALL_ON` is a normal
    // component and the thing a reader wants to check it against is a slope.
    projection: "biplanar",
    wallOn: BASE_WALL_ON,
    wallOnDeg: (Math.asin(BASE_WALL_ON) * 180) / Math.PI,
    units: 3,
    unitBudget: BASE_TEX_UNIT_BUDGET,
  };
}
