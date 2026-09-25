# Interior massifs v0: the island gets mountains, 2026-09-23

The operator's words were *"our world is still weak terrain wise"*. The
measurement behind them, from `examples/terrain_stats` on the shipped seed:
93 % of the land sat under 40 m, most of it on the two shelves `REMAP_LUT`
makes. The highland was noise lumps 60–100 m tall with nothing joining them.
The in-game frames agreed: flat ground, a pale scarp, and a dome on the
skyline.

## What landed

Stage 4d (`terrain::massif_lift`, `TERRAIN.md` §1): two ranges in the
170–560 m annulus, a major and a minor half a turn apart. Each is:
- a wandering four-segment spine under a `t³(4 − 3t)` envelope;
- a spur field that moves the outline, and a crest field for peaks and cols;
- its own 45 m warp over all of it;
- a three-octave, zero-mean **gully filter**, whose C² stripes run down the
  fall line through jittered origins and branch because each octave's fall
  line includes the octaves before it.

The treeline moved from 52 to 68 m with the ranges (`TREELINE_H`).

## Fifteen hillshades before one frame

Every version was drawn with `examples/hillshade` before it was measured.
Each cut is part of the design:

| version | shape | what the hillshade said |
|---|---|---|
| v1 | spine domes | smooth sausages in a three-fold "face" |
| v2 | + ridged carve for spurs | hammered metal (crease peak 4.16) |
| v3 | smoothed | crescent creases; still a face |
| v4 | + gully filter | **mountains**, but a flat top, a halo at the foot, too steep |
| v5 | Wendland crest | a ruled crease along a straight crest |
| v6 | peak clusters | star-bursts radiating off every peak |
| v7 | back to v4, smoothstep profile | caterpillars on straight spines |
| v8 | polyline spines | good major range; minors read as eyes |
| v9 | + 85 m warp | **mesas**: the warp compresses space 4× in places |
| v10 | zero-mean filter | the mesa stayed: it was the warp, not the filter |
| v11 | 45 m warp | the mass is right |
| v12 | `t³(4 − 3t)`, longer minors, `fade` wave | ranges, not ovals — but 57 ‰ cliff, max slope 9.8 |
| v13 | kernel floor, tighter jitter | max slope 5.0 |
| v14 | gentler, crumple steps aside | p50 0.72, 9 % cliff |
| v15 | **two ranges, opposed** | every road suite green |

Two lessons are worth keeping:
- **A domain warp is a compressor as well as a bender.** At 85 m over
  330 m its derivative reaches ~0.77, so ∂warp/∂z touches 0.23. Eighty
  metres of ground then maps onto twenty metres of the spine's cross-section,
  and a crest becomes a plateau. It was diagnosed by probing base and lift
  along a line (`base` flat at 47 m for 80 m), not by looking.
- **Where a feature can go is decided by what else is solved there.** Three
  evenly spaced ranges fail on roads, not on looks: the inland site's side
  roads are straight chords on opposite bearings that may not cross a cliff
  (`solve_side_roads`), so every gap must face another gap. Two opposed
  ranges pass on every seed the road suites run.

## Measured

- **Outside the window, bit-identical.** Heights on a 4 m grid, hashed
  outside the 170–560 m annulus, match the old island on seven seeds. The
  coast, the ring, the haven, the waystations and every spawn keep their
  bits. `tests/massif.rs` holds the lift at exactly zero there.
- **Walkable in the main.** Footprint (lift > 5 m) median slope 0.64–0.71,
  cliff 4.9–10.7 %, peaks 94–141 m on four seeds (`examples/massif_stats`).
  The gate was proven red under three mutants, on the final code as well
  (its header lists them).
- **The economy footprint.** With the treeline at 68 m, summed over four
  seeds, metal nodes go 631 → 649 and sulfur 460 → 484. Per seed the change
  runs −26 % to +56 %. Before the treeline moved it was +27 % to +122 %
  sulfur.
- **Cost, and what was done about it.** The first port cost 2.5× in the
  window and **3.5× on a clutter tile on a range**. That is the one caller on
  the frame thread (`render/clutter.rs`, one tile a frame): the baseline's
  noise rides a `Lattice` memo, and the ranges re-hashed their layout (8
  hashes) and their gully cells (27) on every tap. Three changes, each proven
  on the heights before it was kept:
  - the forward-difference fall line became an **analytic gradient** — the
    spine distance, its smooth-min joints and the radial fade all have
    closed-form derivatives, and the slow fields (spur, crest, warp) are held
    still;
  - the layout and the gully hashes joined the **`Lattice`**. This is
    bit-identical, and `CELL32_KEY` keeps a hash slot from answering for a
    noise quad;
  - a **bounding-disc reach test** skips the warp where no range can reach.
    The test alone is bit-identical. Precomputing each segment's `1/len²`
    moved the last bit, and the golden absorbed it.

  Final, on this box: `height` ~0.95 µs in the window against 0.46, and
  island-wide +20 %. A clutter tile costs 2.7 ms on a range against 1.0 —
  open ground is unchanged. The remainder is the gully filter's own
  arithmetic over 27 cells, and the finest octave is the lever if it ever
  has to go.

## Seen from the ground

These are lavapipe captures at 1280×720, shot on a scratch copy of the
content with animal bites set to zero. Wildlife killed the unarmed probe at
the foot of the major range before its first frame. From the major range's
upper flank the island opens out below — forest, a lake, the sea — which is
the sense of height the terrain did not have. From the minor range's crest
the ground reads as a rocky pine ridge, with the far range on the skyline.

**What still reads wrong:** above the treeline, the rock layer's photograph
(`Gravel004`, 4 m tile) is a field of cobbles at a player's feet. That is a
texture problem on flat granite and predates the ranges — the old summits
had it too. The ranges just put more ground up there. `NOW.md` §0mtn carries
it.
