# loop/looks-pass1 — where this stopped, and what is still open

## What landed

`DECISIONS.md` §open, "prop photograph v1". The visual judge's ranked gap 1
(`findings/pass-20260804-153032-01-visual.md`) named the cause in one line and it
was literally true: *"the terrain got a sourced photograph this pass and the props
did not — this is a coverage gap, not a tuning one."* `rock` and `ore` now sample
the granite layer of the array texture the ground already had.

The diagnosis is worth keeping because it contradicts the obvious reading. The
boulder's *authored* albedo is `0x75726d` / `0x8f9399` — sRGB luma ~116 and ~147,
already inside `ART.md` §3's granite band of 127-167 — and it still rendered at
luma 43.5. So the three previous passes that tuned the authored colour were tuning
something that was not wrong. The gap was that nothing sampled a photograph.

## The gate state, stated exactly

**Every code gate is green, including the new one.** `ci/gates.sh` runs them in
order and the log reaches `pine shape: 32 checks passed` with no failure:
knob registry (84 declarations pinned, 338 checks — the four new `PROP_PHOTO_*`
knobs among them), rustfmt, clippy walls, the native suite, the wasm build,
`test_parity_wasm`, the client bridge, bump basis, **prop photo**, the web bundle,
pine shape.

**`browser_smoke` did not complete on this box, in two runs, in two different
ways — and neither is an assertion about this diff.**

| run | how far | what ended it |
|---|---|---|
| 1 | tab A fully green (66/66 looks); tab B seen 110.2 s in | tab B never rendered 3 in-world frames, so `programsAtInWorld` was never pinned. Frame p50 1383 ms. Load average was 13.05. |
| 2 | tab B joined (63.3 s, 24/24 looks); **prewarm green on BOTH tabs**; prop albedo, surface, grain, register, shadows, horizon all printed | chromium closed during `grainProbe("uTint")` (`ci/browser_smoke.mjs:3538`) — `Target page, context or browser has been closed`. Frame p50 833 ms. |

Run 2 clears run 1's failure outright: the prewarm line reads
`0 program links after the in-world snapshot, both tabs`. And run 2's crash site is
the **terrain's** tint octave probe, which does not touch the prop path this branch
changed.

The control: `web/src` was reverted to the merge base, rebuilt, and `browser_smoke`
run alone — **it did not finish in 10 minutes either**. This is `CLAUDE.md`'s
"confirm with a clean tree before believing any diff caused it", and the clean tree
was no faster. Frame times of 0.8-1.5 s on a software rasterizer are the condition
`NOW.md` items 3 and 4 already describe: *nine of eleven gate failures across seven
runs were the harness fighting itself.*

**This is not a claim that the branch is green.** It is not. The renderer tier has
not returned a pass, so nothing here may be reported as "ALL GATES GREEN", and no
gate, assert, tolerance or skip was touched to get closer to one.

## What the next pass should do first

Re-run `./ci/gates.sh` on a quieter box before anything else — both failures are
non-deterministic and the second run cleared the first's. If `browser_smoke` fails
a **third** time in a **third** place, that pattern is itself the finding and it
belongs to `NOW.md` item 4 (tab B should be a bot, not a second browser), which is
not this lane's to fix.

Two numbers to read when it does complete, both new to this branch:

- the prop contrast ratio (`NOW.md` item 2, sitting on its `PROP_MIN_CONTRAST_RATIO`
  floor at x1.15). This change adds mean-preserving detail to `rock`/`ore` gated by
  the same `uProp` toggle the ratio is measured across, so it should move the
  numerator. It does **not** touch `foliage`, which is the pine the floor was
  actually measured on — so the floor may still sit where it sat.
- `prop albedo`, which must still read `rock#2 0.290→0.290`, `ore#3 0.218→0.218`,
  `ore#4 0.418→0.418`, `rock#6 0.169→0.169`. It did in run 2. That is the
  mean-preservation property holding: a photograph that moved those numbers would
  have un-authored the granite value, and `ci/prop_photo.mjs` exists to catch the
  arithmetic that would let it.

## What is deliberately not done

`NOW.md` item 1 carries it: bark for `wood` (the files are on disk and imported by
nothing), needle cards for `foliage` (geometry, not a map), and the frequency split
that would hand everything above the tile frequency to the photograph and leave the
field the coarse patchiness a tiling map cannot supply.
