# Gates · ART.md — the art bible

**Read this before any pass that changes a pixel.** It exists because the loop
spent six consecutive visual passes discovering, one judge report at a time,
rules that could have been written down once. A judge's ranked gap is now
scored against *this file* rather than against an adjective.

Every number here is either **measured** off `Rust Images/` (the eighteen
checked-in reference frames — the style target per `DECISIONS.md` 2026-08-01,
*"rip rust for now"*) or **observed** in them and labelled as such. Nothing is
invented. When a number here and a shipped constant disagree, one of them is
wrong and the disagreement is the finding — same discipline as the knob
registry.

The IP rail (`DECISIONS.md`) is unchanged and narrow: no proper nouns, no
traced assets. Statistics of light and colour are nobody's property.

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

Ten outdoor-daylight frames of `Rust Images/`, Rec.601 luma over the whole
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
- **Wet sand**: a darker, more saturated band at the waterline. It exists in
  the shipped shader (`WET_RANGE`) and has never been photographed by a
  vantage — either widen it or aim a capture at it.

## 6 · Composition and HUD

The references are first-person with a **visible held item and hand** in most
frames, a **bottom-centre hotbar** with item icons, and a **right-side vitals
stack** with numbers and red status chips (`WET 36%`, `STARVING 2`) — small,
unobtrusive, never centred. A frame with no viewmodel and no HUD reads as a
flythrough, which the blind reader has named on every capture so far.

Depth in a wide frame: foreground detail (a prop or the viewmodel), midground
subject (treeline, rocks, a base), background atmosphere (hazed ridges).

## 7 · Assets: real detail is allowed, and preferred

**There is no procedural-only rule in Gates and there never was.** The
all-generated approach was drift, imported by imitation from projects where
"zero binary assets" is a deliberate stunt; it was never a wall, a knob, or a
spoken call. Operator, 2026-08-03: *"if its CC0 im fine to pull in whatever
helps us. then we can replace later."*

The rule that replaces it:

- **CC0 / public-domain sources only** (Poly Haven, ambientCG). No attribution
  burden, no license file to carry, and orthogonal to the Facepunch rail —
  which is proper nouns and traced assets, not the existence of a texture.
  Anything else needs the operator.
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
- **Budget**: the working set is 6.0 MB at 1K. Keep total texture payload under
  **12 MB** before compression work; KTX2/Basis is the optimisation once the
  look is settled, not a prerequisite.
- **Meshes are the same deal** when the time comes, but procedural vegetation
  may well win on variety — see `.claude/skills/threejs-procedural-vegetation`.

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
