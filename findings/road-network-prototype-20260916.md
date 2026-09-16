# Coastal road routing experiment — 2026-09-16

`crates/sim-core/examples/road_route.rs` is an offline experiment. It changes
no generated world, collision, scatter, site, material, content, or save.
`TERRAIN.md` remains authoritative; this note records measurements.

## Reproduce

```sh
cargo run --release -p sim-core --example road_route
cargo run --release -p sim-core --example road_route -- 20260731 0.125
cargo run --release -p sim-core --example road_route -- 20260731 0.25 --geometry
cargo test -p sim-core --example road_route
cargo clippy -p sim-core --example road_route -- -D warnings
```

The default runs 20260731, 42, and 3735928559. A failed solve, fine audit,
intersection check, or island-centre enclosure exits nonzero; it never silently
substitutes the original broken route. Geometry mode adds CSV rows
`baseline|candidate|anchor,seed,index,x,z` to the report on stdout.

## What it solves

Sixteen ordered coastal anchors start on the existing shoreline-offset trace.
Each can move radially at most the existing `ROAD_INLAND_M` (40 m) to find
terrain that supports the full four-metre carriageway. The haven centre,
two coastal waystation centres, and existing side-road junction are additional
**exact** anchors; none moves. Inland sites and their side roads are untouched.

Between anchors, accept a valid straight segment or search a four-neighbour
8 m graph inside the existing 600–1000 m radial bracket. Every graph edge,
endpoint connector, and subsequent shortcut samples ground across the entire
carriageway at no more than 0.5 m spacing. Squares enclosing the round joints
are checked too. All width samples must remain inside the bracket. BFS uses
fixed neighbour order, marks a node when queued, and visits at most 66,049
nodes per anchor pair; scratch arrays and queue are bounded by that grid.
Visibility simplification has a quadratic bound in the unsimplified path's
node count. This is an offline cost model, **not an acceptable production
init or tick budget established by measurement**.

A separate audit resamples the result at 0.25 m by default, including explicit
centre samples and joint squares. It measures both raw `height`/`slope` and
carved `ground`/`ground_slope`. The result must preserve every exact anchor,
close, enclose the island centre, and have no nonadjacent intersection or
collinear reversal. These are terrain and centreline-topology checks, not
player movement or collision tests.

## What the baseline actually establishes

The baseline trace follows the outermost coastline crossing, offset inland,
at 4,096 interpolated yaw directions. **It is a diagnostic trace corridor,
not an exact reconstruction of the predicate's entire road surface.** On
seeds 42 and 3735928559, a radial may cross multiple shorelines and the trace
can jump between branches. Its normal-width corridor also differs from the
predicate's radial-width band at bends. Thus its overall water/cliff counts,
length, and longest failing-width run cannot establish actual road blockage.
The printed run means that at least one sampled part of each cross-section
fails; it does not mean the full width is impassable.

The separate `center_bad` count admits baseline samples **only when the
shipped `ring_band` says Carriageway**. At 0.25 m audit spacing:

| seed | actual-road centre samples failing land/slope | trace nodes off the shipped road |
|---|---:|---:|
| 20260731 | 836 / 29,643 | 0 |
| 42 | **0 / 27,790** | 104 |
| 3735928559 | 1,547 / 29,231 | 27 |

These counts are identical on raw and carved ground. They establish actual
cliffed carriageway positions on two seeds independently of any raster flood.
They do **not** prove that no narrow walkable route crosses the entire ribbon.
Seed 42 is particularly important: the old 39% raster connectivity number is
not evidence here of a cliffed centreline. No fresh largest-component
percentage is claimed. `second_road` and `side_road` use different grids and
adjacency rules, and their percentages are not interchangeable.

## Candidate measurements

At the default independent 0.25 m audit spacing:

| seed | diagnostic trace length | candidate length | stored points including closure | candidate maximum slope | candidate minimum height |
|---|---:|---:|---:|---:|---:|
| 20260731 | 5,818.4 m | 5,491.5 m | 24 | 1.167 | 2.071 m |
| 42 | 6,204.2 m | 5,670.3 m | 28 | 1.188 | 0.621 m |
| 3735928559 | 5,849.0 m | 5,876.7 m | 35 | 1.190 | 0.608 m |

All three retain all four exact anchors and have zero centreline intersections.
Every candidate width and joint sample passes, on both raw and carved ground.
The cliff threshold remains the existing 1.1917536; no terrain is flattened.
BFS expands 5,559 / 8,601 / 16,997 nodes in total across the anchor pairs.

A further **0.125 m** audit also passes all three, on raw and carved ground:

| seed | passing width samples | passing joint-square samples | passing centre samples |
|---|---:|---:|---:|
| 20260731 | 1,494,946 | 27,744 | 43,969 |
| 42 | 1,543,736 | 32,368 | 45,404 |
| 3735928559 | 1,600,210 | 40,460 | 47,065 |

These are sampled checks, not a continuous mathematical proof. Counts include
shared endpoints and are not independent population observations.

An earlier 1 m solve admitted a segment on 3735928559 that the independent
0.5 m audit rejected: two cliff samples, maximum slope 1.195. Tightening the
experimental solve to 0.5 m changed that route and the subsequent finer audits
pass. This is why an audit finer than the solver is necessary; validating only
the solver's own sample set hid a real counterexample.

## What production integration still owes

1. **Geometry and budget.** Choose a capped stored path and a bounded spatial
   query, account for `Haven` copies and repeated client chunk-batch solves,
   measure startup and hot queries, hash every path in the determinism probe,
   and prove native/wasm parity. Fixed host search order alone proves none of
   those production properties. Broaden the seed sweep and define failure
   policy before choosing caps; three seeds do not establish universal fill.
2. **Actual passage.** The candidate currently goes through unchanged terrain,
   authored sites, and scatter. The live scatter clears the old road, not this
   candidate. Check actual prop volumes, authored canopies, shoulders, swept
   turns and player traversal; exact site-centre anchors alone do not prove
   safe entrances. Existing side roads preserve their junctions but their
   entire widths are not newly certified by this experiment.
3. **Network shape.** Sixteen coarse anchors plus shortest routes produce
   angular chords. A simple centreline enclosing the island is a useful first
   result; it does not enforce turn radius, prevent close parallel road edges,
   constrain local departure from shore, or make a convincing road visually.
   Preserve usable monument ports before smoothing or replacing anchor policy.
4. **World and economy compatibility.** Replacing `ring_band` changes cleared
   cells, barrel positions, bay classification and world hashes. Reprice the
   route/destination comparison with existing gates, audit persistence/wipe
   implications, and update the owning terrain decision together. There is no
   production integration or live deployment in this slice.

The bounded polyline experiment is feasible without terrain grading on the
three sampled worlds. The remaining work is integration, collision, shape,
performance, and broader generation coverage—not a claim that roads are
already fixed in the game.

## Separate production defect found during surface review

`road_band_in` returned the ring's Shoulder before querying the side road.
Where a side Carriageway crosses that shoulder, the union therefore restored
shoulder material, vegetation and eligible scatter slots across the junction.
The contained correction gives either carriageway precedence over either
shoulder. It changes no stored road, site, width, wire layout, or height.

The new sixteen-seed junction regression failed before the correction at
seed 1, `(967.4729, 172.75421)`, with Shoulder instead of Carriageway. It now
covers 223 side-carriageway/ring-shoulder samples and 432 inverse overlaps,
with direct/memo agreement across the whole sampled junction patch. Sixty
relevant existing/new tests pass (`alloc_zero`, `clutter`, `haven`, `road`,
`side_road`, `sites`, `terrain_golden`), plus all-target sim-core clippy.
The pinned terrain golden is unchanged: its sample set misses this narrow
overlap; the new junction regression supplies the missing coverage.

This correction **does** change masks and potentially generated occupants in
that narrow overlap. An unchanged sampled golden is not a claim of identical
world placement or persisted-world compatibility. No deployment accompanies
it. It is independent of adopting the experimental coastal paths above.
