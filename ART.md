# Gates · ART.md — the art bible

**Read this before any pass that changes a pixel.** It exists because the loop
spent six consecutive visual passes discovering, one judge report at a time,
rules that could have been written down once. A judge's ranked gap is now
scored against *this file* rather than against an adjective.

Every number here is either **measured** off **the reference set** or
**observed** in it and labelled as such — the style target per `DECISIONS.md`
2026-08-01, *"rip rust for now"*. Nothing is invented. When a number here and a
shipped constant disagree, one of them is wrong and the disagreement is the
finding — same discipline as the knob registry.

**The reference set is not in this repository, and that is deliberate**
(operator, 2026-08-11). It is eighteen screenshots of the reference game:
ten outdoor-daylight frames, thirteen containing ground, four UI screens and
one top-down map render (13 + 4 + 1 = 18; the daylight ten are a subset of the
thirteen). They are Facepunch's images. This repo is public and MIT, so
carrying them in the tree is *redistributing* them — which the IP rail below
does not cover and `NOTICE` explicitly disclaims. They lived in `Rust Images/`
until 2026-08-11 and were removed; nothing about the bar changed with them.

**What survives the removal is the whole of the bar**, because this file
records the *measurement* and never depended on re-reading the pixels: §3's
table, §6's chroma band and every number below are derived statistics, already
computed, and they are what a pass is scored against. A frame you cannot open
is not a bar you cannot meet.

To re-derive rather than trust: put your own copy of the set in a directory
outside this tree, point **`GATES_REFERENCE_DIR`** at it, and run
`ci/native_bar.py`. Without that variable the tool says so loudly and exits
nonzero — it does not quietly score against nothing. Frames are cited below by
filename (`generichighview2.jpg`) because a filename is a *reference to* an
image, not a copy of one.

**The island under the camera is seed 20260731**, and this file did not contain
the word "seed" until 2026-08-14 — which is worth a line, because on that day a
pass concluded the frames were flat *because the island was*, and the operator
was one command from wiping the public shard for it. The measurement was a
quarter of the island (an origin-centred sweep window on a world centred at
1024, 1024). Over the whole square this seed reaches 106.00 m, slope 2.665 and
granite on 10.0% of its land — upper third of 44 — and within 300 m of the
camera's own spawn it paints 8.9%, where the median island paints 0%. **So a
frame with no granite in it is the renderer's to answer for, not the seed's.**
The seed is still an instrument setting like a vantage bearing: changing it
makes frames incomparable across the change, so it is named here rather than
left to whichever `shard.toml` the probe happened to dial
(`sim-core/tests/relief.rs`, `examples/seed_scan`).

The IP rail (`DECISIONS.md`) is unchanged and narrow: no proper nouns, no
traced assets. Statistics of light and colour are nobody's property — and that
is precisely the line the removal draws, because a screenshot is not a
statistic.

---

## 1 · The target, concretely

Outdoor daylight survival, **near-midday**, on a temperate pine-and-granite
island. Read off the reference set:

- **The sun is high.** Shadows in `generichighview2.jpg` run roughly 1.5–2×
  the height of what casts them — a sun in the 30–40° band, not the 21° the
  client shipped through August 2. Shadows are *present and soft*, blue-grey,
  never black.
- **The sky is the brightest thing in the frame, and it has clouds in it.**
  Cumulus with lit tops and grey bases carry a large share of the frame's
  tonal range. A cloudless gradient cannot reach the reference's spread.
- **Air has depth.** Distant hills lighten, desaturate and go blue — the far
  third of a wide frame is a different register from the near third.
- **The ground is not a surface, it is a population.** Grass reads as
  thousands of individual lit blades standing 20–40 cm, not as a textured
  plane. This is the single largest structural difference between our frames
  and the references, and no shader fixes it.
- **Materials separate by VALUE, not only by hue.** Granite sits far above
  grass; wet sand sits between. A frame where everything shares one value
  band reads as fog, not as a place.

## 2 · Hard rules

Each is here because a judge named its violation, or because the reference
set is unanimous. Breaking one is a finding, not a taste dispute.

1. **No surface may be one flat value.** Every material carries albedo
   variation at two scales: a macro break-up (0.5–1 m) and a near-field grain
   (< 5 cm). Measured target below; a uniform hue is the fastest way to read
   as a prototype and the judges have said so in four separate reports.
2. **Nothing sits ON the ground; everything sits IN it.** Boulders, trunks,
   deployables and pieces get a contact term — an AO darkening, a dirt skirt,
   or scatter crowding the base. In `spawnedrock.jpg` the boulder's meeting
   line with the turf is invisible: grass grows up over it. A clean
   intersection edge reads as a decal.
3. **No pure black and no crushed shadow.** No lit surface's shaded face may
   fall below **0.30** of its lit face (the visual judge's counted ask). Pure
   black is unreachable outdoors: the sky fills every upward face and the
   ground bounces into every downward one.
4. **Empty ground is a defect.** Any visible ground patch larger than ~3 m²
   inside 15 m carries scatter — tufts, pebbles, litter, twigs. The reference
   set has no bare ground anywhere inside the near field.
5. **One owner for light.** Sun, sky fill, bounce, exposure, tone map and fog
   move together in one pass, by one owner (`CLAUDE.md`'s coupled-lighting
   law). Nothing else creates a light or sets an exposure.
6. **Silhouette before surface.** If a prop is not identifiable as a black
   shape against the sky, no material work will save it. Pines in the
   references are tall, thin, and ragged-edged; a smooth cone is wrong at any
   texture budget.
7. **Nothing may look procedural.** No visible tiling, no uniform spacing, no
   two identical instances adjacent at the same rotation and scale.

## 3 · The measured bar

Ten outdoor-daylight frames of the reference set, Rec.601 luma over the whole
frame, sky band = top 25% of rows, near band = bottom 35%:

| statistic | reference median | range | ours, 2026-08-03 |
|---|---|---|---|
| whole-frame p10 | **49** | 24–85 | 38 |
| whole-frame p50 | **104** | 54–155 | 128 |
| whole-frame p90 | **189** | 121–230 | 184 |
| sky band mean | **143** | 53–201 | — |
| near band mean | **86** | 59–123 | — |
| near-band saturation | **33%** | 17–47% | — |
| **near-band neighbour contrast** | **6.3 luma** | 2.7–17.2 | **0.26** |

The last row is the one that matters and the one nothing has moved: adjacent
pixels on close ground differ by 6.3 levels in the references and 0.26 in
ours — a **24× gap**, measured by the visual judge on 2026-08-02 and
reproduced here. Rules 1 and 4 are the two mechanisms that close it.

Sampled surfaces (`generichighview2.jpg`, `spawnedrock.jpg` — hue in degrees,
saturation as HSV S):

| surface | luma | hue | sat |
|---|---|---|---|
| sky, zenith band | 184 | 220° | 19% |
| sky, mid | 213 | 212° | 15% |
| cloud, lit | 145–164 | 211° | 47–56% |
| grass, lit | 59–70 | 63–74° | 29–33% |
| grass, shadowed | 38 | 172° (cool) | 22% |
| granite, lit | 127–167 | 35–43° (warm grey) | 10–19% |
| dirt path | 139 | 38° | 15% |
| beach sand | 117 | 42° | 10% |

Two facts worth stating plainly: **grass shadows go COOL** (hue swings from
70° toward 170°), and **granite is warm grey and much brighter than turf** —
a value separation of roughly 2× that our frames have never had.

## 4 · The light rig

One owner, per rule 5. Current shipped values live in `DECISIONS.md` §open
("lighting v1", "the daylight register"); this file states what the
references *ask for*, and the gap between the two is the work.

- **Key**: one directional, warm white, high (30–40° — blocked today on the
  shadow floors and on the ground's structure moving from bump into albedo;
  see `NOW.md`). Shadows soft-edged, blue-grey, clearly present on turf.
- **Sky fill**: hemisphere, sky half cool blue, earth half warm — this is what
  keeps rule 3's 0.30 floor reachable and what makes shadowed grass go cool.
- **Sky**: gradient dome **plus clouds**. Cloudless cannot reach a p90 of 189
  with a sky mean of 143; the references get there with lit cumulus.
- **Fog / aerial perspective**: distance lightens *and* desaturates toward the
  sky's own colour. One value feeds fog and horizon so the seam is exact.
- Exposure and tone map are the rig's, not a material's. If something is
  blown, fix its albedo.

**Occlusion has three scales and the rig only reaches one of them.** Written
here before any of it exists, because the rule that matters is *where each term
applies*, and getting that wrong is how AO becomes a global darkener. Source:
Lagarde & de Rousiers, *Moving Frostbite to PBR* §4.10.3 (fetched 2026-08-04),
cross-read against Filament's material doc.

- **Micro** (creases, cavities — below any map's reach): baked into the albedo,
  and it is the one term that applies to **direct light as well as indirect**.
  The common shorthand "AO only affects indirect" is about the medium/large
  terms and is wrong if applied to this one.
- **Medium** (between a surface's own features — what a fetched `*_ao.jpg`
  carries): **`indirectDiffuse *= ao`, indirect only.** A light rig cannot
  supply this scale; that is the whole reason `assets/textures/*_ao.jpg` now
  exists. It is also the unblock for the ambient floor — raising hemisphere
  fill lands everywhere, including in the prop chroma ratio's denominator,
  while AO removes ambient only where geometry occludes. Raise the fill and put
  the darkness back where it belongs.
- **Large** (between objects — screen-space, contact shadows): also indirect
  only.
- **Never sum or multiply two occlusion terms of the same scale.** Frostbite
  takes `min(bakedAO, ssAO)` to avoid double-darkening; micro is deliberately
  excluded from that min because it is at a different scale and its influence
  should survive. This binds before either term exists here.
- **Specular occlusion is a separate term, not the diffuse one reused.**
  Omitting it "manifests itself as light leaks"; applying diffuse AO to
  specular is visibly wrong at grazing angles. Frostbite's form is
  `computeSpecularOcclusion(NdotV, min(bakedAO, ssAO), roughness)`.

## 5 · Materials

Authored albedo stays inside `ALBEDO_LUMA_BAND = [0.05, 0.55]` linear
(`materials.js`, derived from real outdoor reflectances). Beyond that:

- **Granite**: two minerals, not one — a buff feldspar and a cool biotite —
  plus crack/crevice structure that darkens on the fold's low side. Cracks are
  thin dark lines, not noise. Lichen mottling in patches, never uniform.
- **Bark**: fissures run UP the trunk (per-axis scale), warm brown, value well
  above black — the trunk was 1.9× under the band floor before prop albedo v1.
- **Canopy**: two greens minimum, needle-card silhouette break-up, and the
  underside must still read (rule 3) — it is the face every judge has caught.
- **Turf**: geometry, not texture (rule 1 / §1). Blades catch a rim of sun at
  their tips; the ground plane beneath is nearly invisible in the near field.
- **Wet sand**: a darker, more saturated band at the waterline. **Shipped on
  the native ground 2026-08-08** (`terrain_mesh::wetted`, 45% darker and 35%
  more saturated, floored so no identity leaves the band below) and
  photographed for the first time by a capture spawned on a beach. The
  browser's `WET_RANGE` was never in a vantage; this one was aimed at.
  **Its width is bounded twice and that is the part worth restating here**: a
  band keyed on height alone is sixty metres of stain on a 4% beach and sixty
  centimetres on a steep bank, so it is capped by a horizontal run as well.
  The thing being drawn is how far the water reaches, which is a property of
  the sea and not of the slope.
- **The sea**: cold, dark, desaturated blue-**green** — Atlantic or Baltic,
  never Caribbean. Unusually for this file the source is the reference
  developer's own words rather than a measurement off the reference set: they
  retuned their ocean to "a more Atlantic/Baltic sea colour" so it would look
  less out of place across biomes (`reference/WATER.md` §2). Green is the
  channel that survives depth in coastal water, and a sea whose deep body is
  blue with no green in it is the wrong ocean.
  Three things the bar asks of it that a colour cannot supply: **the specular
  needs structure** (a flat sheet has one highlight and reads as plastic), the
  **waterline must be a band and not a line**, and **shallow water must show
  what is under it** — a sea with a constant alpha is a plate.
- **The waterline**, since it is the thing every reference beach frame is
  really about. Read one and there is no edge in it anywhere: dry sand gives
  way to a wide damp gradient, then to wet reflective sand, then to water so
  thin it is only a sheen, and the white is a **soft streaky wash standing
  offshore** where the waves are breaking — not a rim drawn around the water.
  Four rules follow, and the third is the one everyone gets backwards:
  the damp band is metres of *ground*; thin water is nearly invisible; **foam
  is weakest at the waterline itself**, because foam that peaks there outlines
  the seam instead of hiding it; and the wash's edges are lobes and fingers,
  never a clean contour.

## 6 · Composition and HUD

The references are first-person with a **visible held item and hand** in most
frames, a **bottom-centre hotbar** with item icons, and a **right-side vitals
stack** with numbers and red status chips (`WET 36%`, `STARVING 2`) — small,
unobtrusive, never centred. A frame with no viewmodel and no HUD reads as a
flythrough, which the blind reader has named on every capture so far.

Depth in a wide frame: foreground detail (a prop or the viewmodel), midground
subject (treeline, rocks, a base), background atmosphere (hazed ridges).

**The interface has a face, and it is bold condensed.** This file had nothing
about type until 2026-08-07 and the client had no typeface at all — every
screen drew in Bevy's embedded debug mono, which is the interface equivalent
of §2's flat-value rule and reads as a prototype for the same reason. The
target is **Roboto Condensed**, and unusually for this file the source is not
a measurement off the reference set but a fact in the reference's own public
source: `Facepunch/Rust.Community`'s `CommunityEntity.UI.cs` defaults its UI
text to `RobotoCondensed-Bold.ttf`. Two rules follow, and the second is the
one that carries the look:

1. **Bold is the default weight, regular is the exception.** Labels,
   headings, numbers, buttons, item names and prompts are bold; only prose —
   a description, a hint, a status line, chat — is regular. A reference
   screen reads as chunky because nearly every word on it is bold condensed.
2. **A screen is one face.** Mixing typefaces across two screens a player
   moves between in one keystroke is the type version of §2's "no surface may
   be one flat value" — it is the fastest way to read as three prototypes
   rather than one product. Enforced as a call-site grep, not as a taste
   note: `crates/client/tests/ui.rs` §F.

Not settled, and stated here so a later pass does not read silence as
approval: the **size scale**. Twelve distinct sizes ship across six files,
which is not a hierarchy, and nothing in this repo can photograph a panel to
check a change to it (`DECISIONS.md`, "ui type v0").

## 7 · Assets: real detail is allowed, and preferred

**There is no procedural-only rule in Gates and there never was.** The
all-generated approach was drift, imported by imitation from projects where
"zero binary assets" is a deliberate stunt; it was never a wall, a knob, or a
spoken call. Operator, 2026-08-03: *"if its CC0 im fine to pull in whatever
helps us. then we can replace later."*

The rule that replaces it:

- **Four bases, and the rail is orthogonal to the Facepunch rail** — which is
  proper nouns and traced assets, not the existence of a texture. **CC0** stays
  the default (Poly Haven, ambientCG, Quaternius): no attribution burden, no
  licence file to carry. **CC-BY is accepted** at the price of one `NOTICE`
  entry plus the manifest row (operator, 2026-08-07). **NC and SA are refused**,
  not on open-source grounds but because `BUSINESS.md` prices an entry fee, so
  non-commercial does not survive contact with the product and share-alike does
  not survive a closed depot. **Generated assets are accepted** — Meshy for
  meshes, ElevenLabs for audio (operator, 2026-08-11) — and they are a
  *contract*, not a licence: both vendors restrict commercial use to a paid
  plan, so anything that ships is generated under one. A generated asset's
  prompt is subject to the Facepunch rail exactly as a mesh is: it describes
  the thing, never the source.
- **`assets/textures/MANIFEST.md` records every file's source and licence.** A
  texture with no manifest row does not ship.
- **Hybrid, not replacement.** Real maps supply base albedo / normal /
  roughness — the measured high-frequency detail a noise field cannot encode
  (§3: reference near-ground neighbour contrast 6.3, ours 0.26). Everything
  already built stays as the *variation* layer: splat blending, per-identity
  tint and chroma, per-instance tint, wear, and the triplanar projection that
  solves UV-less props. Tiling is broken up by that layer, not by more octaves.
- **An off-band source is tinted, not tolerated.** Where a sourced albedo sits
  outside §3's measured band, pull it in with the per-identity machinery rather
  than editing the file — the file stays pristine and swappable.
- **…and the tint is a correction, not an amplifier.** The rule above is about
  a source's MEAN. It says nothing about what the same correction does to the
  source's *deviation*, and that omission shipped an artifact: a per-channel
  gain `color / mean` multiplies the whole sample, so a source dragged far
  across channels to hit its mean has its per-channel noise dragged with it.
  `rock` (a sandstone standing in for granite) needed **×13.45 on blue**, whose
  source mean sits near its own JPEG noise floor, and what reached the image at
  a grazing footprint was per-pixel rainbow speckle across four of six captured
  frames — while every amplitude gate went *up*. So: **a sourced map's colour
  deviation may not be stretched by more than ×1 by the correction that places
  its mean.** Bound it per layer against that layer's own gain span; a source
  already in band keeps its colour whole, and one that is not keeps almost none
  of it, because almost none of what it has left is its own.
- **So picking a source is a measurement, not a browse.** The span above is
  computable from a candidate file before it ships, which makes source
  selection the cheapest lever in this document: `rock` went from keep 0.17 to
  **0.97** on a file swap with no code change, chosen by scoring 74 CC0
  candidates on gain span, albedo sd, and directional anisotropy. Score
  candidates with the shipped estimator itself — never a re-implementation that
  might disagree — and record the numbers in `MANIFEST.md` beside the pick.
- **A map is only as good as the surface it is laid on, and a modifier that
  REPLACES an albedo throws the map away.** Two ways the ground can carry a
  photograph and still not show one, both shipped and both fixed in
  materials v4. (1) *Projection.* World XZ is a projection from above; on a
  face of upness `u` it stretches the map `1/u` along the fall line, x2.8 at
  this island's steepest faces and unbounded at vertical. A sourced map goes
  on the SURFACE — the top plane plus the fall-line plane, whose distortions
  are exact complements. **Two things about a multi-plane projection are
  arithmetic, not taste, and getting either wrong costs more than the smear
  it removes.** (a) *The footprint is differentiated before the frame, never
  after.* A per-fragment projection frame makes `dFdx(uv)` pick up the frame's
  own rotation times a WORLD coordinate, which at island scale is a mip
  several levels too coarse — the fix smears worse than the defect. Project
  `dFdx(position)` onto the frame instead. (b) *The blend needs an exponent.*
  Linear weights hand a third of the sample to the worse plane on a steep
  face; `pow(w, 8)` hands it 0.05%. Both rules are Quilez's, from the
  biplanar-mapping article, and both were shipped wrong here first. (2) *Modifiers multiply, never replace.* Wetness and
  cliff darkening scale the albedo, so the photograph survives them; snow
  used `mix(albedo, SNOW_COLOR, 1)`, a constant, and above 80 m that is whole
  hillsides reverting to rule 1's flat value with every amplitude gate still
  green because none of them is measured up there. A modifier that must set a
  colour multiplies the surface's own mean-1 luminance field by it, so the
  authored colour is the delivered mean and the relief's light and shade
  survive. **Any new causal modifier is a multiply or it carries the detail
  through explicitly — there is no third option.**
- **The measurement that separates detail from noise is direction, not
  amplitude.** Resolve the near ground's high-frequency residual along the
  local mean colour (a surface lighter here, darker there — real detail, and
  what §3's 6.3 counts) versus orthogonal to it (the hue changed between
  neighbouring pixels). Measured over the near-ground band of the thirteen
  reference frames that actually contain ground — the four UI screenshots
  and the top-down map render cannot define a statistic about ground — the
  references run **0.077–0.193 chroma per unit luma, median 0.120**. Above that
  band the frame is aliasing, not texture, and no amount of extra blur or
  anisotropy is the fix: ours ran 0.237–0.798 on the five frames showing ground
  while the along-colour term was already inside the reference range. Measure it
  with one estimator, not two — a reference band computed a different way than
  the frame it judges is not a band. **The gate that asserted it
  (`browser_smoke` 15i) was deleted with the browser client**; `ci/native_bar.py`
  computes the same statistic on the native captures and prints it beside the
  reference, but nothing fails on it. Read it; do not assume it is walled.
- **Budget — browser-era, and it is spent.** The working set is 6.0 MB at 1K
  against a **12 MB** ceiling, and that ceiling was a *download*: a first-visit
  cost paid over the network before the browser drew anything. The shipping
  client is a native binary that installs a depot once (`ci/depot.py`) and
  reads from disk after, so the constraint that produced the number is gone —
  `crates/client/src/render/textures.rs` already says so at the load site.
  What remains real is **VRAM and disk**, which are much larger and which
  nothing here has measured. So: 12 MB is no longer a wall, re-sourcing at
  2K/4K is unblocked, and the replacement ceiling is **unset on purpose** —
  it goes to `DECISIONS.md` §open when someone measures it, not into this
  line. KTX2/Basis stays an optimisation, not a prerequisite.
- **Anisotropy is browser-era too.** `BASE_ANISOTROPY_MAX = 4` is registered
  in `DECISIONS.md` §open and its stated reason is explicitly a browser one —
  at 16, *"a second browser tab did not reach the world at all on this box"*,
  a software rasterizer running two tabs. Every real GPU answers 16 and the
  native client is one process. The knob is **not** raised here — it is
  spoken, and a spoken knob moves by being spoken again — but a native
  value is now a legitimate proposal rather than a re-litigation, because
  **the reason on the row no longer describes anything that exists.** The
  browser client that produced it is deleted; nothing ships this ceiling for
  the stated cause.
- **Meshes are the same deal** when the time comes, but procedural vegetation
  may well win on variety — and now does: the native conifer is generated
  (`crates/client/src/render/tree.rs`). The three.js skill pack
  (`.claude/skills/threejs-procedural-vegetation`) is **guidance about
  technique only** — there is no browser build for it to describe, and its
  API is not ours. Reach for it for the physics, never for the calls
  (`CLAUDE.md` §third-party credit).

## 8 · What passes review

A capture passes only if all of these hold. This section is the visual
rubric's checklist and a judge may quote it directly:

- Materials read as distinct substances at a glance — granite ≠ bark ≠ turf ≠
  sand — separated by **value**, not only hue.
- There is visible contact shadowing or crowding where every object meets the
  ground (rule 2).
- There is colour and value variation *within* each surface at both scales
  (rule 1), and near-ground neighbour contrast is on the way to 6.3 (§3).
- No unlit face sits below 0.30 of its lit face (rule 3).
- The ground inside 15 m is populated, not bare (rule 4).
- The sky is the brightest thing in the frame and it has structure in it.
- The far third is lighter, bluer and less saturated than the near third.
- Nothing reads as procedural: no tiling, no uniform spacing, no repeated
  identical instances.
- The frame contains evidence a person is playing: viewmodel, HUD, or both.
