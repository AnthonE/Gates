# Forest scale v0 — the sweep behind the numbers, 2026-09-14

**Every claim here carries the command that produced it** (`findings/README.md`).
The decision is `DECISIONS.md` §open, forest scale v0; this is the table it
was read off.

## The bench

```
cargo run --release -p client --features render --example tree_sweep
```

Each candidate block is fitted to its height exactly as `tree::conifer` fits
it, then measured with the gate's own functions over the six shipped variant
seeds (three per species, `hash2(0x9e37_79b9, variant)`) and twelve more.
Columns: `r_ship` is the crown radius over the species' own shipped seeds,
`r_all` over all eighteen; `trk_shp` / `trk_all` the trunk radius under
0.25 m the same two ways; `tris` bark + needles.

Ceilings: `TREE_MAX_R` 2.900 · `OCCUPANT_R_M[Tree]` 0.2398 (held to 1 mm on
the widest shipped trunk) · `CONIFER_MAX_TRIS` 6000.

## Pass 1 — height alone (limbs scaled with it)

Conifer at 14 m with a 2.2 m limb read 2.92 / 2.86 / 2.96 m on the shipped
seeds — over the ceiling on one. Broadleaf at 11 m with limbs halved
(0.9 / 0.5) read 4.1–4.8 m: the width is not only the limbs.

## Pass 2–4 — what shipped, and the rows beside it

```
candidate           h r_ship  r_all trk_shp trk_all   tris   (ship = the species' own shipped seeds; all = 18 seeds)
pine-shipped     6.60  1.464  1.513  0.1876  0.1876   5900
leaf-shipped     5.40  2.831  3.084  0.2398  0.2463   3806
pine14-e        14.00  2.610  2.643  0.1887  0.1902   5900
pine14-f        14.00  2.727  2.761  0.1880  0.1894   5900
pine14-g        14.00  2.666  2.705  0.1872  0.1886   5900
pine14-h        14.00  2.707  2.762  0.1865  0.1886   5900
leaf11-e        11.00  3.627  3.783  0.2208  0.2344   3806
leaf11-f        11.00  3.812  4.001  0.2183  0.2326   3806
leaf11-g        11.00  3.916  4.125  0.2183  0.2326   3806
leaf11-h        11.00  3.878  4.420  0.2160  0.2309   4006


leaf-shipped     5.40  2.831  3.084  0.2398  0.2463   3806
pine14-e3       14.00  2.664  2.699  0.2390  0.2406   5900
leaf11-n        11.00  2.679  2.892  0.2340  0.2437   3806
leaf11-o        11.00  2.754  3.173  0.2341  0.2437   4006
leaf11-p        11.00  2.798  3.021  0.2312  0.2418   3806
leaf10-k        10.00  2.739  2.870  0.2077  0.2181   3806
leaf10-l        10.00  2.688  3.125  0.2079  0.2181   4006
leaf10-m        10.00  2.637  2.763  0.2102  0.2198   3806

ceilings: TREE_MAX_R 2.900  OCCUPANT_R_M[Tree] 0.2398  CONIFER_MAX_TRIS 6000
```

Chosen: `pine14-e3` (limb 1.8, trunk 0.2540 → variant 0 at 0.2390 m, crown
2.66 m shipped / 2.70 all) and `leaf11-n` (angles 36° / 32°, limbs 0.38 /
0.20, cards 0.60, crown 2.68 shipped / 2.89 all, trunk 0.234). The broadleaf
could not get under the ceiling by shortening limbs at the old angles (half
limbs still read 3.0–3.2, pass 3) — the angles had to close.

⚠ One of the twelve extra conifer seeds reads a 0.2406 m trunk, over the sim's
cylinder by 0.8 mm. The gate holds the three shipped seeds; a fourth seed per
species would need its trunk re-swept.

## What the frames showed

```
cargo build --release -p client --features render --example tree_look
Xvfb :99 -screen 0 1280x720x24 &
VK_DRIVER_FILES=/usr/share/vulkan/icd.d/lvp_icd.json DISPLAY=:99 WGPU_BACKEND=vulkan \
  target/release/examples/tree_look /tmp/shots
```

- At 14 m / 11 m the pines read as tall spires with a bare trunk; the
  broadleaf read as a **yellower conifer**, because both species drew their
  canopy through `needle_image`. `tree::leaf_image` and
  `PropAssets::canopy_material` fixed that; the first leaf card was thinner
  than the sprig (562 opaque texels against 786) and the gate written for it
  said so before the frame did.
- The far hull shaded as a **stack of discs**: `impostor_of` blended each
  band's normals toward that band's own centre on the axis, so every ring
  flipped its tilt. Built flat and blended to a horizontal radial per vertex
  (`blend_canopy_normals`), it shades as one column.
- Cover did not move much: stems are the 8 m grid's. The forest reads
  taller, not denser — `FORESTS.md` §9.6 item 6 is the lever.
