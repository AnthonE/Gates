# Forest density v1 — the measurements behind the numbers, 2026-09-14

**Every claim here carries the command that produced it** (`findings/README.md`).
The decision is `DECISIONS.md` §open, forest density v1 (and §Spoken the same
day: *"when can we get forest fr?"*); this is what it was read off.

## 1. The ceiling was the field, not the cell

Every doc priced density as `CELL_SIZE` (one occupant per 8 m cell, 156/ha)
and the Forest sat at 260‰ trees — a quarter of its cells, ~39 stems/ha. The
rail was `tests/scatter.rs::test_no_biome_row_saturates`: a row scaled by a
field peaking at `CLUMP_NORM` = 2.7× its mean may total 370‰ before a grove
cell asks for more than the roll can give. The grid had four times the room.

The way past it is a cap on the field per biome, re-normalized so the row is
still the biome's mean. Which cap:

```
cargo run --release -p sim-core --example clump_cap
```

| cap | mean(min(g, cap)) | norm | cells at/above cap | row rail (‰) |
|---|---|---|---|---|
| 1.00 | 0.7476 | 1.3376 | 43.3 % | 748 |
| 1.10 | 0.7879 | 1.2692 | 37.5 % | 716 |
| 1.25 | 0.8385 | 1.1927 | 30.1 % | 671 |
| 1.50 | 0.9009 | 1.1100 | 20.4 % | 601 |
| 2.00 | 0.9695 | 1.0314 | 8.4 % | 485 |
| 2.70 | 1.0003 | 0.9997 | 1.7 % | 370 |

Four gate seeds, 65,536 cells each. **1.0 — the field's own mean — shipped**:
every above-average cell is a stand at the row's scaled ceiling, every
below-average cell keeps the field's gradient down to the clearing floor
(0.061 of the mean, untouched). Linear compression of the swing toward 1 was
worked on paper first and rejected: at a third of the swing a "clearing"
still carries two thirds of the mean, which is a thinner forest and not an
opening.

Forest row `[640, 12, 0, 0, 20, 28, 0]` = 700‰ × 1.3376 = 936 of 1,000. A
`const` block in `terrain.rs` refuses a row that would saturate.

## 2. What the island reads now

```
cargo test --release -p sim-core --test forest --test scatter -- --nocapture
```

| | before | after (seeds 0 / 1 / 7 / 12345) |
|---|---|---|
| Forest stems/ha | 38.31 / 37.5 / 38.9 / 37.1 | **96.29 / 92.91 / 94.70 / 93.02** |
| Meadow stems/ha | ~11.0 | 12.66 / 12.82 / 11.86 / 13.20 (the edge blend) |
| forest : meadow | 3.2–3.6 | 7.05–7.99 |
| forest bush/ha | ~7.9 | 3.35 / 3.38 / 3.03 / 3.32 (50 → 20‰) |
| 40 m forest window dispersion | 2.90–3.34 | **4.70 / 5.15 / 5.27 / 5.47** |
| empty forest windows | 37–52× null | 0.08–0.20 % (null ~0) |
| live slots | 9,825 / 10,033 / 9,337 / 9,770 | **16,020 / 16,666 / 14,743 / 15,660** |
| shipped seed live slots (`terrain_golden`) | ~9,700 | 15,590 |
| capped field mean × norm | — | 1.0004 / 1.0012 / 0.9979 / 1.0005 |
| cells at the cap | — | 43.3 / 42.9 / 43.3 / 43.6 % |

Canopy cover at the shipped crowns (mean ~2.63 m radius, 21.7 m² a crown):
~20 % mean, ~29 % in a stand — over FAO's 10 %, short of a closed 40 %.
Closing it is the crown now (r²), not the stem count: `NOW.md` §0t item 2.

## 3. The client's ring, and why the budget is a count

```
cargo run --release -p sim-core --example ring_census            # seed 20260731
cargo run --release -p sim-core --example ring_census -- 0       # and 1, 7, 12345
```

| seed | eyes | ring p50 / p90 / max | inside 80 m p50 / p90 / max | inside 95 m (swap + fade) p50 / p90 / max |
|---|---|---|---|---|
| 20260731 | 271 | 344 / **811** / **1,086** | 43 / 198 / 246 | 71 / **278** / **357** |
| 0 | 265 | 463 / 966 / 1,046 | 79 / 206 / 245 | 117 / 286 / 345 |
| 1 | 265 | 469 / 842 / 1,024 | 98 / 194 / 242 | 141 / 265 / 340 |
| 7 | 262 | 339 / 911 / 1,080 | 55 / 200 / 245 | 80 / 275 / 340 |
| 12345 | 274 | 406 / 867 / 1,052 | 71 / 196 / 267 | 101 / 273 / 361 |

Before v1 the ring's p90 was 328 with 82 inside the swap. 357 × 5,900 is
2.1 M before a hull — over `DESIGN.md` §9's 1.5 M for the whole frame — so
a distance no longer bounds anything and `tree::TREE_LOD_CAP` does:

| | at means (5,900 / 105) | at ceilings (6,000 / 112) |
|---|---|---|
| near ring, p90, cap 180 | 1,128,255 | — |
| near ring, densest, cap 180 | 1,157,130 | 1,181,472 |
| outer ring (9×9 − 5×5), p90 / densest | — | 203,392 / 272,384 |
| **whole frame, densest eye** | — | **1,453,856** |

`tests/tree.rs` holds the first column and `tests/outer_ring.rs` the second;
the gates use ceilings, which is why the cap is 180 and not the 200 the
means admit. At `OUTER_RADIUS` 5 the outer ring alone was 350–470 k and the
sum did not fit; it is 4 now (288 m of treeline, from 352).

Where the swap lands (`tests/tree_cap.rs`, `--nocapture`):

```
at the grid's ceiling the swap lands at 44 m (arithmetic 45.6 m)
densest eye: 742 trunks in the ring, 306 inside the tier's 95 m, 172 drawn under the cap's 52 m swap
```

The real streamer at the census's densest eye (676, 292): the cap binds at
52 m, 172 trees draw their pair (the 2 m step drops a ring), every near part
carries the band the cap wrote. At the p90 eye the arithmetic puts it at
~61 m; the cap does not bind at all below ~180 in reach, which is most of
the island (p50 is 71).

## 4. What it cost

- `GOLDEN_TERRAIN_HASH` `0x9033_206F_0ECB_E2A4` → `0x700C_77A5_8F33_97B4`;
  `GOLDEN_FINAL_HASH` `0x565A_4CF4_EC7B_B375` → `0x9B6A_EF8D_69AC_6FF7`;
  `GOLDEN_TRACE_HASH` `0x31F6_CAD0_C685_6B34` → `0xD9A5_FA5D_C6DA_36B9`.
  Worldgen, the cheap kind — no verb moved. A wipe-boundary change.
- `MAX_SLOT_LIVES` 16,384 → 32,768, both tables boxed (`CLAUDE.md`'s wasm
  shadow-stack trap: 32,768 × 16 B is half the stack). `WORLD_SAVE_MAX_BYTES`
  645,326 → 874,702; no format bump.
- Two gates read the forest through something else and were re-measured
  rather than moved: `tests/road.rs`'s shoulder ratio (barrels as a share
  of what stands on the shoulder: 56 / 50 / 64 / 52 % → 46 / 37 / 49 / 39 %,
  the barrel COUNT identical, because a forest road is lined with trees;
  floor 42 → 30 %) and `.cargo/config.toml`'s `World` note (315 → 53 kB,
  the boxed table). Both found by `./ci/gates.sh`, one per run.
- `tests/scatter.rs`'s `test_no_biome_row_saturates` now also sweeps every
  land cell of the four seeds through `scatter_draw_row` (the densest drawn
  row asks for 936 on every seed), and `test_clump_cap_normalizer_holds` is
  new — the cap's `CLUMP_NORM`-shaped gate, with a 25 % floor on the share of
  cells at the cap so a cap that stopped binding fails.

## 5. Frames

```
# shard.toml: dev_spawn = "676,292" (the census's densest eye), then
target/release/shard &
VK_DRIVER_FILES=/usr/share/vulkan/icd.d/lvp_icd.json DISPLAY=:99 WGPU_BACKEND=vulkan \
  target/release/gates 127.0.0.1:4433 --capture <dir>
```

Six vantages under lavapipe, sent to the operator 2026-09-14. What a person
reads off them (the visual gate is the operator's boot, not this):

- **It is a forest.** Every yaw from the spawn is trees to the horizon, with
  a broadleaf among the pines and the understory brush under both. The
  `design` and `sky` frames are canopy over the eye; before v1 the same seed
  read as parkland with sky between every tree.
- **The hull is visible where the cap parks the swap.** In the `east`
  frame two light-green faceted cones stand at the water's edge ~60–80 m
  out — the far hull, at the ~52 m the cap lands the swap at in this stand
  (`tests/tree_cap.rs`'s print). That is `NOW.md` §0t item 1b: the hull's
  look at 50–60 m is the next thing a frame settles, not the count.
- **The ground reads as forest litter**, the grey-brown pebbled channel the
  splat gives a forest, with grass tufts where the grass channel bleeds in.
  Whether it wants more litter (needles, cones) than pebbles is the art
  rubric's, not this slice's.

No GPU has run this.
