# hitscan v0 — what a bullet costs, measured

*Systems lane, 2026-08-19. Every number here comes from
`cargo run -p sim-core --release --example shot_cost` on this box (8 cores).
Re-run it rather than trusting the digits.*

## 0. Why there is a note at all

`ranged::hitscan` walks a firearm's whole reach **in one tick**. The shipped
revolver is 50 m and the collision spacing is `ARROW_STEP_MM` (170 mm), so one
pull of the trigger is **295 point samples**, each one a `terrain::ground`
evaluation plus an occupant query plus a piece query. An arrow spends at most
`MAX_ARROW_SUBSTEPS` (16) samples per tick and spreads its flight over forty of
them — so a bullet's per-tick cost is **eighteen times an arrow's**, by design,
because a bullet has no flight to spread it over.

That asymmetry is not visible in any wall. Wall 4 asks for a cap and this has
one (`MAX_HITSCAN_SAMPLES`, and one shot per player per tick); a cap is not a
budget, and 100 × 295 samples is a number somebody has to look at.

## 1. What one sample costs

```
crates/sim-core/examples/shot_cost.rs      (the harness)
terrain::ground x 29,500 → 11.0 ms         (≈ 373 ns each)
terrain::height x 29,500 → 10.8 ms
```

**The terrain evaluation is the entire cost of a sample.** `height` is two
2-octave warp fbms, a 5-octave relief fbm and a ridge tap — roughly eleven
noise evaluations — and `ground` adds `site_stamp` on top for ~2 % more. The
occupant and piece queries against an empty `ColIndex` and a barren scatter do
not register beside it.

This is the fact to carry: **a hitscan design's cost is a count of terrain
taps**, and nothing else in it matters until that count changes.

## 2. What a tick costs, at population

100 shooters, all firing on the same tick (the aligned volley — not what play
produces at a 12-tick cadence, but the case a cap has to survive):

| case | mean | worst |
|---|---|---|
| full 50 m walk on every shot (the first draft) | 20.0 ms | 26.4 ms |
| nobody in range, walk bounded at 64 samples | 5.0 ms | 6.5 ms |
| a body 3 m down every barrel | 1.15 ms | 1.41 ms |
| ditto, standing on pristine terrain | 1.13 ms | 1.68 ms |
| 50 shooters, a body at 45 m | 2.40 ms | 3.22 ms |

The tick budget is 33 ms.

## 3. The two things that bought that

Both are in `ranged::hitscan` and both are exact — neither changes who was hit.

1. **Ask the cheap question first.** `nearest_body` is ~100 integer-ish
   compares and no noise; the world walk is 295 terrain taps. So the body is
   solved over the whole segment *first*, and the world is then walked only as
   far as that body — past it, nothing the world could stop changes the answer.
   The single compare `t <= stop_t` afterwards is exactly what passing the real
   `stop_t` into `nearest_body` would have computed, because it picks the
   minimum `t`. This is the **opposite** of `ranged::step`'s order, and
   deliberately: an arrow's segment is 1.3 m and 16 taps, so ordering buys it
   nothing.

2. **Bound the walk a *miss* takes.** With no body in the line there is no
   correctness question left at all — the only reason to walk is `EV_IMPACT`,
   the decal. `MAX_HITSCAN_MARK_SAMPLES = 64` (10.9 m) is the cosmetic bound,
   and it is the row above: 20 ms → 5 ms. What is lost is the mark on a
   hillside past eleven metres, which is the mark nobody is standing close
   enough to read.

   It does **not** bound a shot that hits: where the walk was already owed for
   the hit decision, the mark is free and is emitted wherever the stop fell.

## 4. What is still exposed, and what would fix it

The irreducible worst case is **100 shooters each with a body at maximum
range on one tick** — the truncation has nothing to truncate. Scaling the
50-shooter row above, that is ~5 ms, so it is the same order as the bounded
miss and not a new cliff. Adding a second firearm with a longer reach moves it
linearly, and `MAX_HITSCAN_SAMPLES = 320` (54.4 m) is the boot refusal that
keeps it from moving silently.

If that ever needs to come down, the fix is **not** a coarser step — 170 mm is
pinned by a 0.468 m trunk, and a shot that tunnels through cover is a lie the
data does not admit to. It is a cheaper terrain query along a ray:

* **A conservative march.** `terrain::ground` is a Lipschitz field; if the ray
  is `c` metres above the ground at a sample and the terrain's slope is bounded
  by `S`, the ground cannot reach the ray within `c / (S + |ray slope|)`
  horizontally, so the march can skip. `relief.rs` measures the shipped island
  at max slope 2.665 — **a measurement, not a proof**, and `site_stamp` carves
  authored sites with edges a global bound may not cover. It needs its own gate
  before it can be trusted, which is why it is not in v0.
* **A column cache.** The shots on one tick are spatially clustered; a cached
  coarse heightfield along the ray would amortise across them. It is state, and
  state in the sim is `state_hash`'s problem.

Neither is worth building before a firearm exists that anybody has fired.
