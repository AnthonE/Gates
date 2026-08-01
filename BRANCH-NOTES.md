# loop/m1-surface-grain — read this before the code

This branch is **red on purpose** and is not merged. `main` carries only the
record of what it measured (`NOW.md` item 1). It is kept because the next
pass should start from measurements rather than rebuild them.

It is a **mid-experiment snapshot**, and its own in-tree prose describes
earlier states of the experiment. Where they disagree, this file wins.

## What the head actually ships

- `web/src/materials.js` `FADE_GRAIN_CPP = [0.03, 0.09]` — the LAST and
  cheapest configuration tried, grain reaching ~2.5 m. The `DECISIONS.md`
  §open row on this branch says `0.18 → 0.65` and the commit message quotes
  `[0.12, 0.30]`; both are earlier settings, and the comment above the
  constant still argues for 0.3. The code is right, the prose is stale.
- The grain probe at the head therefore measures **near 10.01% moved,
  contrast 0.17 → 1.02 luma/px (×6.12), far 0.000%, control noise 0**.
  The `×12.2 / 20.9%` headline in the commit message and the DECISIONS row
  belongs to `[0.12, 0.30]`, and `25.90%` for the surface probe belongs
  there too. Every number is real; each names a different setting.
- `ci/browser_smoke.mjs` here implements the **`exposeBinding` +
  `addInitScript` push** join, not the `waitForFunction` the DECISIONS row
  describes. `main`'s `NOW.md` records the conclusion: both repairs measure
  WORSE than the poll `main` still uses, and the push variant fails the
  public tab even on an unmodified `main` client. **Do not re-land either.**
  If it is revisited, note the judge's finding that its timeout diagnostic
  runs an unbounded `page.evaluate` against a starved renderer and needs its
  own deadline.

## What is NOT here

**The triplanar projection was backed out before this branch was committed.**
It survives only as prose (the `materials.js` header comment and the
DECISIONS row, which also describes gate assertions "16b" that no code here
contains). Its measurements — slope-to-contour contrast 1.100 → 1.078 on a
47° face at ×1.00 overall contrast, exact identity on level ground, ~9% of
frame time — cannot be re-derived from anything in this tree. Rebuilding it
means rebuilding it, and the shape that worked was: ridge fold applied per
plane BEFORE the blend, and the blend's deviation restored by `1/|w|`;
without both, a 47° face measured ×0.56 the contrast of the same face on
world XZ.

## What is worth salvaging first

Two changes here are **image-identical optimizations** that make `main`'s
terrain program *smaller* and were never gated on their own:

1. `gmHash4` — the four lattice corners evaluated in one vec4 body, lane for
   lane the same arithmetic, taking the field from four inlined hash bodies
   per noise site to one.
2. the micro octave skipped where its own footprint fade is already zero.

The judge's read, and it looks right: those two alone could plausibly go
green and would buy headroom for the grain octave. Gate them in isolation
before adding anything.
