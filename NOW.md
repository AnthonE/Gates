# Gates · NOW.md — what next

The only list that answers "what should the loop pick up." Top item first.
Done items are deleted, not checked — history lives in git and
`DECISIONS.md`. A loop iteration starts here, ends with gates green.

1. **The lighting iteration — one owner, the whole coupled set, spoken.**
   *(Operator, 2026-08-02, closing the loop the evidence opened: prop albedo
   v1 reproduced the visual judge's darkest pixel — canopy underside RGB
   (2,6,0) — from the light rig's constants alone, and showed that at 3× the
   authored albedo it is still (5,17,3). The materials lane is blocked on
   light by arithmetic, not taste. CLAUDE.md's coupled-lighting law applies:
   sky, sun, fill, exposure, tonemap, fog and shadow move together in ONE
   pass or not at all.)*

   The one iteration owns, together: the sun's register (the reference bar
   is MIDDAY — `art/RUBRIC.md`'s frames, not the current low-sun murk), a
   sky that reads as a sky lit by that sun, a hemisphere fill that gives
   down-facing surfaces credible ambient (inherit the arithmetic in
   `DECISIONS.md` §open "prop albedo v1" — the (2,6,1) row is the input),
   water specular agreeing with the same sun, exposure/tonemap re-metered
   for the new register, fog that matches the sky at the horizon seam, and
   the shadow rig re-biased if the new sun angle needs it.

   Acceptance, from the panel's own reports: the sun no longer "contradicts
   itself between sky, water, and shadows" (every report so far); a
   down-facing prop face lands in a range where its surface field is
   VISIBLE (then write the p05 wall prop albedo v1 deliberately left
   unwritten); the frames' register sits with the reference set's midday,
   not beside it. Re-measure assertion 15's directional margins and the
   albedo band after — the register change moves both, and the numbers in
   `DECISIONS.md` §open get re-derived, not left stale (the knob-registry
   gate will catch you if you forget).

   Everything else in the materials lane (needle cards, granite structure,
   undergrowth, the coarse-octave re-place) WAITS behind this — it will all
   read differently under the new light, and re-tuning it twice is the
   three-parallel-rounds mistake the trap list already paid for.

2. **Nothing that is not the ground has a surface.** *(Gap pass. From the
   visual judge's ranked gap 1 in
   `findings/pass-20260802-163821-02-visual.md` — "rock, wood and canopy are
   each one flat colour per facet, literally the rubric's own disqualifier",
   and "no amount of further terrain work reaches criterion 2 without this".
   Its gap 2 — the four artifact classes — is the terrain's, and is blocked on
   the coarse-octave slice item 2 already names, so this pass took the half
   that is not.)*

   **Landed this pass** (`DECISIONS.md` §open, "prop surfaces v0"): the field
   the ground has, extended to everything else.

   - **A triplanar two-octave field on every `surfaceMaterial`** — boulder,
     trunk, canopy, wall, door, ore, body. Triplanar because a prop has no UVs
     and is not a heightfield; the same three-tap normal blend and the same
     `/length(w)` deviation restoration `gmGrainTri` already uses.
   - **The gradient is analytic, so this bump cannot be the ground's dither.**
     Value noise with a quintic fade has an exact derivative out of the four
     corner hashes the value already costs. There is no screen derivative
     anywhere in the patch, so nothing in it can be constant across a 2×2 quad
     — the defect the pass before this one measured and could not fix on the
     terrain (`§open`, "the quad-constant gradient").
   - **Structure is what separates a rock from a log**, not amplitude: a ridge
     fold (`1 − |2n−1|`) turns a blob field into a crack network, a crevice
     term darkens the fold's low side so a crack reads as depth, and `scale` is
     a per-axis vec3 so wood's fissures run UP the trunk. Seven classes, seven
     distinct structures, asserted.
   - **The octave frequencies are set by the OBJECT, not by the ground.** The
     first cut used the ground's frequencies and measured 0.00% of the pine
     frame moved at 10 m: at 5.5 /m the canopy's field retires at 7.7 m. The
     coarse octave is now about a third of the object it sits on (canopy 1.0 m,
     boulder 0.8 m, bark 0.5 m across the grain), which retires at 21–42 m —
     the band the report is about.
   - **The pine's silhouette is ragged**, per-vertex and deterministic, pulling
     canopy rings IN only (a canopy that could grow would invalidate the spoken
     4 m beach-spawn clearance from the renderer). 40 → 48 triangles a pine;
     the measured frame peak did not move.
   - **The gate**: `browser_smoke` 15f, structural half plus a two-view probe
     aimed at instances terrain finds. Its sharp assertion is the field's own
     difference image — neighbour variation as a share of magnitude — because
     a wash scores **exactly** 0 there and the ship-vs-flat ratio it replaced
     is bounded by whatever facet detail the mesh already had.

   **Landed the pass after** (`DECISIONS.md` §open, "prop albedo v1"): the
   value the surface is delivered into — and the gate that has a unit in it.

   - **Every prop assertion was a ratio, and a ratio is scale-free.**
     `contrastRatio` is `(baseline+added)/baseline`, `diffStructure` is step
     over magnitude, `chromaRatio` is spread over spread — so a field swinging
     ±0.8 of a level on a surface delivering luma 6 scores *exactly* what the
     same field swinging ±17 levels on a surface delivering 120 scores. That is
     how v0 shipped green through all three while the visual judge, measuring
     the merged frames, found "a solid" (residual 1.23/255 over 7,800 px) and
     named the amplitude rather than the absence. `propProbe` now returns the
     delivered value as a p05/p50/p95 histogram beside `diffMean`, the field's
     own amplitude, both in 8-bit luma, and 15g walls the median and the
     amplitude at 24 and 2.2 against shipped 48/59 and 4.86/8.47.
   - **`ALBEDO_LUMA_BAND = [0.05, 0.55]`**, the linear luminance every authored
     dielectric albedo sits in, asserted over all seven archetypes at both ends
     of every ramp, derived through the renderer's own sRGB conversion rather
     than restated beside the hex. Two of nine bands were under the floor —
     pine trunk ×1.887, pine skirt ×1.106 — and were rescaled in linear, both
     ends together, so hue and ramp shape are exact.
   - **The shaded half is measured, and it is the light rig's, not albedo's.**
     A down-facing prop face receives only `FILL_GROUND × FILL_INTENSITY`, so
     it lands at `groundColor × 1.15 × albedo ÷ π × EXPOSURE` — for the pine
     skirt, **RGB (2,6,1) against the visual judge's measured (2,6,0)** on
     `03-canopy-up`, reproduced from the constants alone. At 3× the authored
     albedo it is still (5,17,3). The §open row is the arithmetic the lighting
     owner inherits; `p05` is deliberately left unwalled until then.

   **What this item still wants:**

   - **Bark and canopy are still one mesh each.** The field gives them a
     surface; it does not give the pine needle cards, a second species, or the
     trunk/bough separation the report asks for, and there is no undergrowth,
     no bushes and no grass instances anywhere on the ground.
   - **The rock and bark rebuild.** Granite's value range is now walled but not
     authored: rock carries the ground identity's HUE, the field's crevice
     darkening and a band-checked albedo, and still no albedo *structure* — no
     two-mineral granite, no vertical bark ridge. The dirt-ring base is still a
     pedestal rather than blended flush.
   - **Five of seven classes have no rendered coverage.** 15g's structural half
     scores all seven; its pixel half photographs `rock` and `foliage`, the two
     the probe can reliably find near the pinned spawn. `wood`, `stone`,
     `metal`, `ore` and `cloth` are asserted structurally and never seen.
   - **A prop-program budget.** The ground's fragment program is walled at
     96,000 chars and 8 noise sites; the prop program is now the second-biggest
     shader in the client and has neither. `propFacts().noiseSamples` publishes
     6; the wall is not written.

3. **The world reads untextured and shows its mesh — and both halves turned
   out to be arithmetic, not missing art.** *(Gap pass. From the visual
   judge's ranked gap 1 in
   `findings/pass-20260802-050932-01-visual.md`, which returned FAIL on all
   ten criteria with a blind reader identifying 0/6 of our frames as real.)*

   **Landed this pass** (`DECISIONS.md` §open, "materials v2"): the two
   defects the frames actually carried, both measured off
   `04-ground-down.png` before anything was changed.

   - The three structural octaves retired *past* Nyquist (meso 0.74, micro
     0.65 cycles per pixel), so each was still being sampled after it
     stopped being representable. Every octave now retires on the one law
     the grain octave was already written against — cycles per pixel, with
     the metres derived — and `browser_smoke` 15a2 asserts it over the
     whole table.
   - The bump reconstructed its gradient on the triangle, so a
     smooth-shaded heightfield rendered its own facets. It is solved in
     world XZ now, mesh-independent by construction, and gated as
     arithmetic in `ci/bump_basis.mjs` rather than as a screenshot.
   - **The knob registry drifted, and the drift is now gated.** The first
     cut of this work proposed `BUMP_MAX_SLOPE = 1.0`; measuring it (a 45°
     perturbation against a 21° key light) sent 0.55 to the shader while
     the `DECISIONS.md` §open row kept saying 1.0 — nine gates green over
     the disagreement, and the merge-gate judge caught it by reading
     (`findings/pass-20260802-050932-02-judge.md`, checks 4 and 9). The row
     now records the shipped value and its derivation, the same stale
     derivation is corrected in the shader comment that also carried it,
     and `ci/knob_registry.mjs` pins every §open knob declaration to the
     constant that actually ships — Rust and JS alike, unresolved and
     ambiguous names failing as loudly as a mismatch.

   **Landed the pass after** (`DECISIONS.md` §open, "materials v3"): the
   ground has a HUE that varies, and a gate that can see one.

   - **Per-class chromatic albedo, tiled at 0.5–1 m.** One noise sample at a
     per-identity tile scale (sand 0.59 m, rock 0.71 m, grass 0.91 m, litter
     1.00 m) driving a signed chromatic deviation per identity, added rather
     than lerped to, so each identity's authored colour stays its exact mean
     and what changed is the variance. Cost 6 → 7 of the 8 budgeted noise
     sample sites, and the budget was not widened to fit.
   - **The deviations are luminance-neutral, and that is a law rather than a
     taste.** Three scalar octaves and a per-identity grain already moved
     VALUE at four scales; nothing moved HUE, which is the defect stated as
     arithmetic (`k·(r,g,b)` has the chromaticity of `(r,g,b)`). Two earlier
     cuts swung both and each one spent assertion 15's directional margin,
     which at this spawn's yaw 0 is 0.5% against a 0.2% floor before anything
     is added.
   - **The report's "macro-variation octave to break tiling" was built twice
     and deleted twice**, off macro and then off meso, because an octave
     wider than the frame is a constant inside it and both read as a colour
     cast. The premise came with the words: value noise on world XZ does not
     repeat, so there was no tiling to break.
   - **A second measurement track**, because the luma probe every gate in
     this file used is structurally blind to a hue-only octave. 15d masks on
     chromaticity and asserts spread up, centre still, mean luma still, and
     warm and cool both present, at two views.

   **Landed this pass** (`DECISIONS.md` §open, "the quad-constant gradient"):
   the dither is measured, bisected and gated as arithmetic — and the fix is
   blocked on the coarse-octave slice below, which is now the top want for a
   second, independent reason.

   - The newest visual report's ranked gap 1 ("either a flat colour wash or a
     per-pixel dither") is **quad-locked**: measured on its own
     `05-held-level.png`, 1.9 luma/px of neighbour contrast inside each 2×2
     quad against 21.4 across quad boundaries. Only a screen derivative can do
     that — `dFdx`/`dFdy` are differences across the quad, so anything built
     from one is constant inside it. The splat wobble the report's ranked fix 1
     blamed reaches albedo per fragment and cannot produce that signal at all.
   - `scene.aliasProbe` (new, four states off existing uniforms) bisects it:
     zeroing gmH takes the ratio 6.15 → 1.01, and zeroing **grain's bump
     alone** does the same. Grain is the only octave whose fade band falls in
     the near field — 33 → 11 px per cycle is 1–4 m from the eye for a 12 cm
     tuft, where meso's equivalent is 165 m out and micro's 30 m.
   - The fix is a second sampling law: a reconstructed gradient is quad-constant,
     so an octave must retire as a BUMP before it retires as a colour. It was
     built and measured — ×1.01 at both vantages with within-quad detail
     unchanged — and **it is not in this commit**, because it reddens assertion
     15: at yaws 0 and 4.71 the surface probe finds 21% and 24% of the frame
     moved and *not one pixel brighter*, since the only thing the field
     brightened there was the mosaic. Three unblocks were built and measured
     and none is enough; the §open row has all six numbers.
   - **A textbook cause was tried, measured and removed**: `vGmPos.xz` is a
     world coordinate in the high hundreds, so a float32 varying reaches the
     fragment quantized to ~1.2e-4 m against a ~2e-3 m pixel — a 6% staircase on
     `dFdx`. Camera-relative coordinates for the Jacobian moved the ratio
     6.16 → 6.15. Not the cause here; the record is in the §open row.
   - **Still open and arithmetic**: the file states bump slope as
     `amp × bump / wavelength`, and a sinusoid's peak slope is 2π times that.
     Every per-octave slope in the comments and in the materials v2 §open row is
     6.3× understated, so `BUMP_MAX_SLOPE` at 0.55 is not the sum it is
     documented as — it is a bound the octaves exceed, and it clips. Re-deriving
     the amplitudes in the right convention is its own slice: it changes how
     much relief the ground has, so it wants the visual judge on it.

   **What this item still wants**, in the order the report ranked it — and the
   first one now blocks the bump law as well as the tint:

   - **Re-place the meso octave — tried, backed out, and now with a second
     reason to want it.** At 9.5 m the coarsest surviving octave completes a
     third of a cycle inside a typical 8 m ground framing; 4 m completes two
     and still retires far past any footprint this world produces. It went
     red on `browser_smoke` 15c, and not on the arithmetic: the splat wobble
     is driven by gmMeso, so moving it moves which identity owns a face, and
     grain reads its scale, contrast and ridge off those same weights. The
     second reason arrived with the tint: **the macro octave's own ±0.16
     albedo multiply is a cast, not a variation**, at every framing narrower
     than 48 m — measured, it is most of why the field darkens 95% of what it
     touches at yaw 0 and why assertion 15's two-sidedness has only 2.5× of
     margin there. A coarse octave that varies inside a frame would fix both.
     Do it as its own slice, with 15c's 46.6° face re-measured alongside it,
     because the coupling is the reason it is not a one-constant change.
     **This pass added the second reason and measured the third.** The bump's
     sampling law cannot land until this does: with the mosaic gone, yaws 0 and
     4.71 of `surfaceProbe` brighten *nothing*, because the field there is a
     macro cast and the artefact. Fading macro on the DUAL of the sampling law
     — cycles per FRAME, so an octave too coarse to vary inside the frame stops
     being a cast — was built and measured at +0.17% on yaw 4.71 against a 0.2%
     floor (from +0.04%) and +0.01% on yaw 0, applied to the albedo multiply
     and to the splat wobble alike. That is the shape of the fix and not enough
     of it; a coarse octave that genuinely varies inside an 8–25 m frame is.
   - **Splat transitions by height/slope/noise, and a wet-sand waterline.**
     `WET_RANGE` exists and paints; the report saw no shoreline in any
     vantage, so either the band is too narrow to read at capture framing
     or no vantage looks at one. Measure before tuning.
   - **The rock and bark rebuild** (report's ranked fix 8): granite albedo
     with granular grain and crevice darkening, the dirt-ring base blended
     flush instead of reading as a pedestal, vertical bark ridges. Note that
     the ground's rock identity now carries granite's HUE range (buff
     feldspar ↔ blue-grey biotite) and deliberately not its value range —
     that half was left for this slice's crevice darkening.
   - **Bark and canopy are still outside all of this.** Materials v0–v3 are
     the ground's splat material; the scatter pools are `surfaceMaterial()`
     bundles with baked vertex colours and a per-instance tint. The report
     asked for bark and canopy albedo in the same breath as grass and sand,
     and nothing in the tint octave reaches them.

   Not this item, and not to be folded into it: the lighting gap
   (`-visual.md` ranked gap 3) is one owner, one iteration, per
   `CLAUDE.md`'s coupled-lighting law — sky, water specular, shadows and
   exposure move together or not at all.

4. **A base can be broken into now, but it cannot be repaired, and a
   raid still ends in a shrug.**
   *(From the merge-gate judge's ranked gap 1 in
   `findings/archive-prestamp/pass-20260802-035930-01-judge.md`, and its
   two predecessors'.)*

   Melee v0, the death backpack, and piece damage all landed
   (`sim-core/combat.rs`, wire v11 → v13): a kill drops what you carried,
   and a swing that finds no node and no player breaks the wall, the
   doorway or the door in front of it. `content/weapons.toml`'s
   `structure` column says how much, `content/balance.toml`'s breach
   bands hold the door as the way in, and the three `DECISIONS.md` §open
   rows hold every bound and every deliberate omission.

   What the lane still wants, in the order it is worth doing:
   - **A repair verb, and the hammer that swings it.** Damage is now
     one-way: a chipped wall stays chipped until it decays away or falls.
     Every base in the genre is a repair loop, and without it the first
     raid a base survives still ends it. Content shape: a repair rate
     per material against the piece's own build cost, banded in
     `balance.toml` next to the breach bands.
   - **Throwables — the satchel has a price and no verb.** The raid
     ratio is computed from a weapon nobody can use; melee is the only
     armed raid tool, which is why the wall floor has to be so high.
     A throw needs an arc the sim can integrate and the client can
     predict, so it is M2's ballistic work, not a slice of its own.
   - **A container UI, and per-slot looting.** The take is all-that-fits
     today, which is honest but blunt: nobody can see what is in a bag
     before opening it, or leave the stone and take the gunpowder. The
     inventory screen in `inventory.jpeg` is the shape; a bag panel beside
     it is the slice. `EventMsg::BagRemoved` already carries *why* a bag
     went, so "someone got there first" has a feed line waiting for it.
   - **Armor and headshots.** `armor.toml` bakes into nothing; aim is
     planar, so there is no head. Both wait on M2's rewound raycasts.
   - **Ground drops for a full inventory.** `gather::inv_add` still loses
     the overflow — now that a ground container exists, that loss has
     somewhere honest to go.

   The wire counters, both moved this pass: the event subtype field is
   **31 of 64** used (v13 widened it 5 → 6 bits, which is why there is
   room again), and the action subtype field is **9 of 16**. The next
   C→S verb — a repair, a throw — is an action subtype, and there are
   seven.

5. **`gmHash4` — four lattice corners in one `vec4` body, never gated.**
   The projection half of this item landed (materials v1 third pass,
   `DECISIONS.md` §open): the grain — and only the grain — is sampled
   triplanar, ridge-folded per plane before the blend and the blend's
   deviation restored by `1/|w|`. Measured on the 46.6° face this spawn
   offers: tilting the ground coarsens the world-XZ grain ×2.017 and the
   shipped one ×1.397, a gain of ×1.444 against a ×1.456 stretch, at
   ×1.044 amplitude. The gate for it (`browser_smoke` 15c) is a within-run
   comparison against a compiled `flatgrain` partner at two square-on
   cameras, and it goes red — ×1.000 — the moment the tap stops reading
   the normal.

   What is left of the old branch is `gmHash4`: the four lattice corners
   of a noise sample evaluated in one `vec4` body instead of four inlined
   scalar ones. It is image-identical by construction, and it has never
   been gated on its own — the pattern to copy is `noskip`: compile a
   `noh4` variant with materials v0's scalar hash and require the same
   frame. It matters more after the projection than before it, because
   grain now takes three noise samples where it took one, so the ground
   pays 6 sample sites per fragment against 4 — and a sample site is four
   hash evaluations.

   **Do not re-run the cost question on this box.** Every run of the gate
   takes it again; the six taken while grain was built read +14 ms
   (0.3× the floor), −74 (1.7×), −64 (1.3×), −110 (2.2×), −104 (8.2×),
   −62 (0.1×) — five of six the wrong sign, less work measured slower.
   Grain lands exactly where level 0's PCF landed, inside a floor that is
   a one-sample estimate itself and swung 13–603 ms across those same six
   runs. A seventh reading is not a tiebreak.
   The counted budget is the one that answers, and it now has three axes,
   all asserted: 81,820/96,000 program chars, 6/8 noise sample sites per
   fragment, 18/24 depth fetches. Price `gmHash4` there too — hash
   evaluations and program chars — not in ms.

6. **A tab that boots beside another live tab takes 34 s to reach the
   world. Nobody knows where those seconds go.**
   The third-tab version of this went red on 2026-08-01 16:26 (`inWorld`
   at 61.6 s of a 60 s window) and the recovery pass closed it, but by
   removing the *contention*, not the cost: the gate now closes tab A and
   tab B once their last assertion is made, so the public tab boots on an
   empty box and joins in **0.3 s**, and a structural check refuses to let
   it boot beside a live tab again. `JOIN_TIMEOUT_MS` was not touched.

   What that bought is the reading this item always wanted, at the harness
   level: **join time is monotonic in live tabs — 0.4 s alone, 34–36 s
   beside one, 55–61 s beside two.** The 34 s is the part still standing,
   and it is still the thinnest margin in the suite: tab B needs a live
   tab A (mutual AOI is M0's exit condition), so no amount of harness
   tidying can hand it a quiet box.

   The *client* half is therefore untouched and is the live risk. Grain
   did not cause it — the frame moved 630 → 638 ms, 1.3% — but nothing
   has measured where the seconds go, and every slice that adds a material
   or a program spends more of them. **Do not fix this by widening
   `JOIN_TIMEOUT_MS`.** Measure it first: the tab's own timeline from
   `#connect` to the first publish, split into wasm load, connect,
   handshake, first compile and first chunk. The cost probe already says a
   terrain program costs ~3 s to compile here, and a fresh tab compiles
   more than one.

7. **Nothing casts past 720 m, and nothing out there has a silhouette.**
   The horizon casts now (`DECISIONS.md` §open) but two limits are stated
   rather than solved: the coarsest clipmap level stops at 720 m because
   fog closes at 1000 m, and past the near ring the only caster is the
   8 m ground itself — the scatter stops at the ring's edge, so a forest
   at 400 m casts nothing and the gate measures the horizon on 2 of 4
   yaws for exactly that reason. A scatter LOD (billboard crosses,
   `TERRAIN.md` §4's "trees get two LODs") is the fix and it is a terrain
   job, not a shadow one.
8. **A capture the same twice is a gate; a capture that drifts is a vibe.**
   Deterministic capture mode (operator, 2026-08-02, `DECISIONS.md` — the
   Claude-of-Duty adoption row): the client animates off the sim tick / an
   injected fixed-step clock in capture mode — today the RAF loop steps off
   `performance.now()` (`main.js`), so boot-time noise shifts every pixel —
   with a fixed seed, the existing `__gatesDebug.setView` shots, and ONE
   fresh page per shot (state leaks between shots on a shared page:
   exposure-like accumulators, particle age). Then a pixel-diff tool
   against blessed per-box goldens, exit nonzero on any moved pixel, used
   two ways: refactor/optimization passes assert zero diff; feature passes
   regenerate goldens in the same commit — `test_protocol_golden`'s
   discipline, wall 6, applied to frames. v1 scope: solo shard, camera-only
   shots, no remotes in frame. v2, if wanted later: render-from-WAL-replay
   (wall 5 already guarantees the state side). The clock conversion is the
   prerequisite, not the diff tool — grep every `performance.now()` and
   RAF-timestamp use in `web/src` and sort each into sim-driven, cosmetic
   (must switch to the engine clock in capture mode), or UI-only (excluded
   from shots). Settle by tick count, never by time.

9. **M1 — survival verbs** + bags, hotbar, chat (`ALPHA.md` §1/§6).
   Gather, craft, build, and deployables are sim'd, on the wire, and
   solid (slice 13: chat — two channels on the reliable lane, local at
   20 m and global, sanitized at both edges and rate-limited per
   connection, deliberately outside the sim so a replay never depends on
   what anyone typed; wire v10 spends the last kind code on it, T opens
   the composer and `/g ` sends global, and the browser gate now types a
   line in one tab and reads it out of the other's DOM).
   Next: **shared access** — one owner id gates a door and a hearth
   today, so two friends cannot share a base; whether that arrives as a
   code lock, a hearth auth list doors inherit, or crews needs the
   operator's word (`DECISIONS.md` §open, lock v0 row) ·
   death/backpack/respawn-on-bag (bags place + cap now; the anchor lands
   there) · piece damage (M2's raid lane: hp exists and decays, nothing
   attacks it yet) · nametags (chat names a speaker by id today, because
   nothing has a name yet).
10. **M2 — combat true**: lag-comp ring + rewound raycasts · ballistic
   projectiles · satchel + damage-by-tier · day/night · netem feel bar.
11. **M3 — economy dark + ops**: OBOL machinery behind the A1 switch ·
   admin lane · backups · status page · error capture · `bench_transport`.
12. **A1 playtest** (operator schedules): 10–20 testers, one wipe cycle —
   then tune content bands from what the anomaly log and the replays say.
13. **M4 — arm A2, then A3** (operator acts): claim rail export · skin
   catalog · the board delivery (repo + playable link + a recorded round
   whose replay hash checks) on `munus-first-sale`.

14. **`cargo test --workspace` overflows a debug thread's stack; only
    `--release` (what CI runs) is green.** Pre-existing, not new: verified
    on `main` at `25f6ec8` before the backpack slice, where
    `snapshot_budget` aborts the same way. The cause is size, not logic —
    `World` is ~416 kB of fixed capacity and `ShardCore::new` builds it on
    the stack, so an unoptimized frame holds two or three copies against a
    2 MB limit. It bites anyone who types the obvious command. The fix is
    the one this slice already used for its own store: box the big
    fixed-capacity members (`Pieces`, `Deploys`, `SlotLives`) at
    construction, the way `ShardCore` already boxes its client array —
    one allocation at boot, none in the tick.

Standing rule: anything a playtest breaks jumps this queue; anything a
wall catches jumps the playtest.
