# How the reference game builds a road network

Research, not law. `TERRAIN.md` owns our roads; this owns what theirs are and
what that implies. **Written 2026-09-16 because the operator asked for "more
roads and monuments, however Rust did it" and the honest answer to "didn't we
research that already" was no** — `MONUMENTS.md` researched where a large
authored PLACE goes and mentions roads only in the collision list it produced
(`MONUMENTS.md` §2). Nothing in `reference/` had ever asked how the road
network itself is built.

---

## 0 · Provenance, and the tier of every claim

**Tier 1, fetched whole, 2026-09-16, from this box.** `SOURCES.md` §0's rule
is that reachability is a property of the container and must be *probed*
rather than remembered, and the probe is in the record: a deliberate 404 on
`rust.facepunch.com` returned the host's own 404 rather than a proxy error,
so the host answers here, as it did for `FORESTS.md` (2026-09-09) and
`ROCKS.md` (2026-09-13). Devblogs 50, 63, 180, 188, 189 and 197 and the wiki
pages `procedural_generation_customization`, `map` and `Map_Data` were
fetched and are quoted verbatim below.

**The first pass used one tier-3 source**, Rustafied (2020-02-06), for lane
counts and the size threshold. **Follow-up, 2026-09-16:** the official
February 2020 update and its screenshots were fetched and inspected; §2.1
upgrades the lane counts and corrects the placement-order claim. No road
width in metres has been established. The historical size threshold below
is not a verified present-day rule.

**This is the clean sourcing tier and that is deliberate** — `AUDIO.md`'s
posture. Everything here is a public devblog or a public wiki page: no
decompilation, no extracted assets, no GPL-licensed source. `CLAUDE.md`'s IP
rail forbids copying the reference game's art and nothing here comes close to
it; what is taken is the SHAPE of a generation pipeline, which is an idea
about software and not an expression of theirs.

⚠ **What the sources do NOT contain, said up front so nobody cites this file
for it**: the actual path-finding algorithm. Devblog 180 says the old one was
"a really basic graph algorithm" and that a new one shipped; no devblog and no
wiki page names Dijkstra, A\*, a cost field or a spline fit. §5 describes the
road's stored SHAPE, which is documented, and stops there. Anyone who needs
the search itself is looking at an unopened question.

---

## 1 · The finding, first

**Their road network is a solved, stored path. Ours is a predicate.** That
single difference explains every other one, and it is not a defect on either
side — it is two different answers to "where is the road", each correct for
its own constraints.

`wiki.facepunch.com/rust/Map_Data` documents `PathData` as the thing a map
file stores for a road: a **list of world-space nodes**, a **width**, a
**spline** flag, and indices into the terrain's **splat** and **topology**
layers (§5 quotes it in full). A road is authored once, by a solve, and then
it *exists* as geometry.

`terrain::road_band` (`crates/sim-core/src/terrain.rs:1018`) never asks where
the ring is. It asks *am I on it*, by inverting the shoreline locally in up to
six height taps, and it is a pure function of `(seed, x, z)` with no state.
That is what lets the client mirror the road for free and what keeps it inside
wall 1.

**The consequence for "can we add more roads" is the whole of this document.**
A second road that is ALSO the inversion of something the terrain already
defines is nearly free to us and impossible for them to express. A second road
that goes from *here* to *there* — which is what a side road is — is trivial
for them and needs a solve for us. They are not interchangeable and the
choice is not a matter of taste.

---

## 2 · The network is three tiers, not one

`wiki.facepunch.com/rust/procedural_generation_customization` lists the
generation features a server may switch off, and the road tiers are three
separate booleans, all defaulting true:

> **MainRoads** (default: true)
> **SideRoads** (default: true)
> **Trails** (default: true)

alongside `Rivers`, `Powerlines`, `AboveGroundRails`, `BelowGroundRails` and
`UnderwaterLabs`. So "road" is a family, and a server operator can have the
main network without the capillaries.

The shape of the family, from `wiki.facepunch.com/rust/map`:

> "Around the map there are railroads and roads they create a ring around the
> map branching off to connect to monuments."

**A ring, and branches off it to the places worth going.** That is the whole
topology in one sentence, and it is the sentence this repo needed.

⚠ **Original tier-3 evidence** (Rustafied, 2020-02-06; lane counts now
confirmed by §2.1): the ring road is
> "a 2 lane paved street which goes around the outskirts of each Rust island"

on
> "any procgen map over 3k in size"

(an edit on that page notes the threshold moved from 4k to 3k), and
> "single lane paths branch out from the main road at various points"

to individual monuments. Also from the same page, and worth more to us than
the widths: **"building is no longer allowed on the roads themselves."**

**That historical size threshold is bigger than our whole island.** Ours is
2,048 m (`terrain::ISLAND_SIZE`), below the cited threshold. This is
not an argument against ours — our ring is the spine of the alpha design
(`TERRAIN.md` §5) — but it is a real scale difference and it is why their
branch counts cannot be copied as numbers.

### 2.1 · The 2020 rewrite and the visible surface

**Primary-source correction, 2026-09-16.** Facepunch's
[February 2020 update](https://rust.facepunch.com/news/february-2020)
documents a later rewrite: a two-lane ring with single-lane branches,
slightly curved authored meshes for terrain blending, and supermarket,
gas-station and warehouse placement along the ring. Thus §3's and §9.1's
monuments-first account describes the older system, not every monument tier.

**Observed in its screenshots**, not measured material parameters:

- [Junction](https://files.facepunch.com/andre/Screen%20Shot%202020-01-30%20at%2009.19.39.png):
  grey cracked pavement, faded dashed yellow centre marks, worn white edges,
  and a flared junction with interrupted centre markings.
- [Roadside entrance](https://files.facepunch.com/andre/Screen%20Shot%202020-01-30%20at%2009.31.15.png):
  exposed aggregate and dirt shoulders blend into vegetation; the road bends
  gently past the forecourt. Pavement has broad wear and fine surface grain,
  rather than a flat black fill.

These images establish neither physical widths nor marking spacing. They
remain outside the repository; links are references, not shippable assets.

**Gravel branches remain a Gates proposal.**
[Devblog 189](https://rust.facepunch.com/news/devblog-189)'s road-visual section
describes alternating dirt/asphalt as a future intention, not a shipped
rule. These sources do not establish a universal asphalt-main/gravel-side
material split. `ART.md` still owns our visual bar.

---

## 3 · Monuments are placed first. Roads are routed to them.

This was the 2017 order. §2.1 records the later roadside-tier exception.

Devblog 188:

> "Monuments now simulate a large number of potential placements and pick the
> one that fits the most monuments on the map."

> "Up until now the map generation was only trying to find a spot for the high
> priority monuments and was okay with having less of the other ones around,
> even if there were more ideal options available that could fit much more
> monuments into the same space."

Devblog 189 states the objective it replaced in terms of the player's
experience: some seeds had

> "huge areas of wasteland without anything interesting in sight."

So monument placement is a **global optimization over the set** — how many fit
— rather than a greedy walk down a priority list. Roads come after, and route
to what the optimizer put down.

**Ours is the inverse.** `terrain::haven()` picks the pad by scanning bearings
and inverting the road's own centre-line definition, so the site is chosen ON
the ring by construction (`terrain.rs:1859`), and `pick_minor` places
waystations on the same ring. `MONUMENTS.md` §9.3 already names the exact
moment that stops working:

> "our sites are *chosen on the road* … so §2's road-connection problem is
> solved by placement order rather than by a port table. That holds until a
> site wants to be somewhere the road is not."

That day is the day anyone asks for more roads, because a road between two
points on one ring is not a road — see §7.

---

## 4 · A monument declares where a road may touch it

Devblog 189:

> "Procedural roads now connect to the roads that are part of the monuments.
> This means all monuments can specify in and out points where the roads can
> connect to."

Devblog 188 puts the same thing as the fix:

> "monuments can finally specify the exact points the road network should
> connect to."

And Devblog 180 records it as the intention a year earlier:

> "I want to add road connection points to monuments so the generated road
> network will smoothly lead into the monuments and continue on the other
> side."

**"And continue on the other side" is the load-bearing half.** A port is not a
place the road stops; it is a place the road passes through. A monument with
an in and an out is a bead on the network, not a cul-de-sac — which is what
makes the network circulate rather than branch into dead ends.

---

## 5 · What a road IS, in their map file

`wiki.facepunch.com/rust/Map_Data`, `PathData`:

| field | what it holds |
|---|---|
| `name` | "River, Road or Rail" |
| `width` | the path's physical width |
| `nodes` | "List of the world spaces of the individual nodes" |
| `spline` | curved segments vs straight |
| `innerPadding`, `outerPadding`, `innerFade`, `outerFade` | edge blending |
| `randomScale`, `meshOffset`, `terrainOffset` | visual placement |
| `start`, `end` | endpoint markers |
| `splat`, `topology` | indices into the terrain layers the path paints |

Three things to take from this table and one to leave.

**Take the polyline.** A road is *n* world-space nodes and a width. That is a
representation we can hold: small, fixed-capacity, solvable once, and
queryable afterwards as point-to-segment distance — arithmetic with no height
tap and no trig, which is the cheapest thing in this document.

**Take the splat/topology pair.** A path carries the ground identity it paints
AND the gameplay layer it marks, as two separate indices. We already have both
halves and already keep them agreeing on purpose: `terrain::splat_road`
(the material) and `RoadBand` (the layer), with `ROAD_WEAR_SHOULDER` derived
from the carriageway's own width rather than chosen.

**Take `start`/`end`.** Endpoint markers are what let a network know a stub
from a through-route, which is §4's "continue on the other side".

**Leave the mesh fields.** `randomScale`, `meshOffset`, `terrainOffset` are a
renderer's business in an engine that instantiates road prefabs along a
spline. Ours paints ground.

---

## 6 · What they had to fix, which is a list of the failure modes

Worth more than the feature list, because each is a defect a road network
actually produces and therefore one ours can produce.

**Devblog 50 — roads that wander and cross.**
> "The first version of the procedurally generated roads had a couple of
> issues where they would take pretty crazy routes or cross each other in ways
> that didn't really make much sense."

**Devblog 180 — a network of disconnected fragments.**
> "The road network no longer looks like a loose collection of tiny road
> segments that aren't really connected to each other"

and the fix, which is a rule about legibility rather than about connectivity:
> "it now adds T and Y intersections wherever they make sense in order to
> simplify the road network, just like a road planner would do in real life."

> "you can clearly identify main roads and smaller side roads that connect
> nearby monuments."

**Devblog 180 — not enough redundancy**, stated as the next problem while
shipping the fix to the last one:
> "it should really add more redundant connections, even if they aren't
> required to reach all monuments. This will open up shortcuts and connect
> dead ends back to the main road in order to form more circular shapes."

**Devblog 189 — roads meeting water and terrain badly.**
> "Roads are no longer split in half by rivers, there are no more gaps between
> roads and terrain, and it's no longer possible for roads to be partially
> flooded by water."

**Devblog 63 — the loot rule that makes a road a route.**
> "Barrels only spawn next to roads and monuments"

That last one is the one we already copied, before this file existed: the
coast road's shoulder is a barrel band (`ROAD_OPEN_BARREL_PERMILLE`,
`ROAD_BAY_BARREL_PERMILLE`) and `ci/haven_prize.mjs` holds the destination
above the route. Worth noting that we arrived at it independently and that
theirs is stated as an exclusion — barrels spawn *only* there — where ours is
a rate the beach row also pays.

---

## 7 · What ours measures, against the tree

Measured 2026-09-16 with `cargo run --release -p sim-core --example
second_road` on the shipped seed, at the coast ring's own band width on a 2 m
grid so every row is comparable:

| candidate | area | slope mean | unwalkable | walkable-connected |
|---|---|---|---|---|
| coast ring (shipped) | 5.4 ha | 0.450 | 2.9% | 79% in one piece |
| contour ring @ 25 m | 11.0 ha | 0.480 | 1.6% | 85% |
| 6 radial spokes | 5.3 ha | **0.247** | **1.6%** | 80% |
| 2 site-to-site chords | 1.6 ha | 0.126 | 0.0% | 100% |

And the number about the island rather than any candidate:

> **38% of walkable land is more than 300 m of walking from any road** —
> p50 218 m, p90 570 m, max 782 m.

That is Devblog 189's "huge areas of wasteland without anything interesting in
sight", measured on our own island. The interior has no route.

**Three findings, and two refuted the guess that preceded them.**

**7.1 · A straight road across our interior is not steep.** Radial spokes read
as an obviously bad idea — a line that ignores the terrain — and the
measurement says otherwise: 48% of this island's land lies between 10 and
20 m, so the interior is shelf country and a line across it comes out
**flatter than the shipped coast ring** (0.247 against 0.450 mean slope, 1.6%
against 2.9% unwalkable). Our terrain does not punish a straight road the way
theirs does, because our remap curve manufactures shelves on purpose
(`TERRAIN.md` §1 stage 4).

**7.2 · A road between two points on one ring is not a road.** Chords between
the pad and the waystations are the flattest, most connected thing measured —
0.0% unwalkable, 100% in one piece, no water crossed — and they **save 3% and
5% of the walk**. Three sites on one ring subtend small angles, and a small
chord is very nearly its own arc. This is §3's "that holds until a site wants
to be somewhere the road is not", arriving as a number.

**7.3 · The connectivity that matters is over walkable cells only**, and by
that measure the SHIPPED ring is 79% in one piece. Filling through cliff cells
flatters every candidate to ~100%. The bar for a new road is ~80%, not
perfection.

⚠ **"79% in one piece" is not a measurement, it is a grid** (2026-09-17). The
same islands read 43.5% at a 2 m grid and 13.3% at 4 m under the same
8-neighbour flood, because a 4 m ribbon sampled at its own width is a broken
chain of cells whatever the terrain does — §8 gate 2 already carries this
warning about its own first draft and it applies here too. The second clause
survives and is the useful half: *filling through cliff cells flatters every
candidate*, which is now the confirmed cause (§9.5 item 5). Use the standable
SHARE, which is grid-stable; do not quote a piece count from this section.

Adding spokes to the ring and re-walking the island gives a clean
dose-response:

| spokes | p50 | p90 | land >300 m from a road |
|---|---|---|---|
| 0 (today) | 218 m | 570 m | 38% |
| 4 | 96 m | 254 m | 4% |
| **6** | 78 m | 208 m | **0%** |
| 12 | 46 m | 126 m | 0% |

Six is the knee. ⚠ **But "spokes" is the wrong shape and §3 is why** — theirs
branch to monuments, not to the map's geometric centre. A spoke to the middle
of nowhere is a road that reproduces the defect it was built to fix, one level
in. The reach numbers say how much road the island wants; they do not say
where it should go.

---

## 8 · What the gates should do

Every one is arithmetic over `terrain`, no clock and no pixels.

**Gate 1 — reach.** Assert the p90 walk from land to the nearest road stays
under a stated bound. This is the number that says a road network exists for
the player rather than for the map, it is what §7 measures, and nothing in
this repo asserts it today. Mutant: delete a road tier; it must redden.
✅ **BUILT as a dose-response rather than a bound**, which is the one design
change this list needed: a bound is satisfied by an island that got smaller
and drifts every time worldgen moves. The gate measures the island BOTH ways —
with the side road and with it switched off — and asserts the difference, so
the mutant this line asks for is the control arm and runs every time.
Measured: unserved land falls 38.2% → 28.3%, p90 walk 572 → 451 m.
⚠ **What it cannot see, measured**: truncating the road to a 5 m stub at the
site still moves unserved by 6.9–8.4 points against the full road's 7.8–11.9,
and the two sets overlap, so no floor separates them. Most of the gain is from
a road cell existing in the interior AT ALL. The stub is gate 2's and gate 3's
to catch, and they do.

**Gate 2 — the network is one piece, to the bar the ring already sets.**
Assert the largest walkable component of the whole road set holds ≥ the share
the coast ring alone holds (79% measured). Mutant: route a side road through a
cliff band; the component splits.
⚠ **CORRECTED 2026-09-16 — that wording is wrong and fails on a correct
road.** Adding cells to any component that is not the largest lowers the
largest one's SHARE arithmetically, whatever the road did: on three sweep
seeds the ring's own largest component is 2,595 / 1,326 / 1,647 cells of
3,257 / 3,382 / 3,173, so two of three side roads join a component that is not
the biggest and the share falls while the network strictly improves. **And
79% was itself a sampling artifact** at the wrong grid — the first draft of
the gate read the shipped ring as 7.6% in one piece at 8 m, because a 4 m
ribbon sampled every 8 m along a curve gives diagonal neighbours and a
4-neighbour flood cannot join them. The gate asserts what the sentence meant:
the road is in one component WITH ring, and that component holds **more ring
than road** — you do not walk the length of a road to arrive at less road.
✅ **BUILT** — `tests/side_road.rs::a_side_road_joins_more_ring_than_it_is`,
and it caught the defect below before the road shipped.

**Gate 3 — a side road goes somewhere.** Assert every side road's endpoints
are a ring point and a site port, and that its length is meaningfully shorter
than walking the ring between the same two points. This is §7.2's 3% failure,
gated. Mutant: connect two on-ring sites; the ratio collapses toward 1.
✅ **BUILT, with the second half replaced.** The endpoint half is exact
(`every_inland_site_has_a_road_and_both_ends_are_what_they_claim`: the port is
`WAYSTATION_RADIUS_M` from the site's centre on the bearing the road carries,
to a millimetre, and the ring end is on the carriageway). The ratio half does
not apply to an inland site — it is not ON the ring, so there is no "walking
the ring between the same two points" to compare against — and what stands in
its place is a length floor derived from the tier's own definition
(`ROAD_REACH_M` less the rim) plus gate 1's control arm.

**Gate 4 — no road crosses water or cliff.** Devblog 189's fix, as an
assertion: sample every road centre line and require land above `LAND_MIN_H`
and slope under `CLIFF_SLOPE_RATIO`. Ours already has the shape of this for
the ring (`tests/road.rs::the_road_is_walkable_along_its_length`) and it
should cover every tier.
✅ **BUILT** — `a_side_road_crosses_neither_water_nor_cliff`, walking every
metre where the solve samples every eight, so a solve coarse enough to step
over a 6 m inlet passes its own check and fails the gate. 9,525 samples over
16 seeds: lowest 0.83 m, steepest 0.888 against a 1.192 cliff ratio.

**Gate 5 — the ladder survives.** `ci/haven_prize.mjs` holds the destination
above the route in expected items per site. A second road tier must not
flatten it, and a third site tier must not out-pay the pad. Mutant: give a
side road the ring's own barrel rate.

**Gate 6 — separation, pairwise and by tier.** `MONUMENTS.md` §9.3: the
current floor is one constant asserted by hand against two tiers. Assert a
pairwise rule over ALL sites whatever their tier. Mutant: add a third tier
with no rule; two sites land inside each other.
✅ **BUILT** — `SITE_SEP_M`, `SiteLedger` and `tests/sites.rs`
(`the_shipped_pairs_are_floored_at_the_constant_they_always_were`,
`every_shipped_island_satisfies_its_own_roster`). The mutant run was the
ulp one: moving every entry 600.0 → 600.00006 reddens that file and is
invisible to twenty other tests.

**Gate 7 — an inland site is inland, and inland means the ring does not
reach it.** Not in this doc's first draft, and it is the one that caught a
real defect: §9.2.1's bracket is easy to write as the geometric limit (where
does the footprint stop touching the road) rather than as the service one
(where does the ring stop serving the land), and the two differ by 280 m.
Mutant: restore the geometric bracket; **7 of 16 seeds place at 580 m, 20 m
from the ring's shoulder.**
✅ **BUILT** — `ROAD_REACH_M`, and `tests/sites.rs`
(`an_inland_site_stands_clear_of_the_road_it_is_defined_as_being_off`,
`an_inland_site_is_further_from_the_ring_than_the_ring_reaches`). Two gates
rather than one, because the first stays GREEN under that mutant — a site at
580 m really is clear of the band — which is exactly why a negative
definition needs the positive claim beside it.

⚠ **A warning that applies to all of them**, from `CLAUDE.md`'s `lattice.rs`
entry: run the mutant. A reach or connectivity band wide enough to hold both
the current tree and the defect reads as coverage and is not.

---

## 9 · What it means for us

### 9.1 · The order is the finding, and ours is inverted

Theirs: **monuments, then roads to them.** Ours: **road, then sites on it.**
Neither is wrong in isolation; ours was the cheaper way to get one ring and
three sites onto an island, and `MONUMENTS.md` §9.1 is right that it bought
reproducible placement they do not have.

But it does not extend, and §7.2 is the proof in metres. Every site we have is
on the ring, so every road between two of them is a road beside the ring.
**"More roads" and "more monuments" are therefore not two requests.** The
first is blocked on the second: until a site wants to be somewhere the road is
not, there is nowhere for a new road to go.

### 9.2 · What to build, in the reference's own order

1. **Inland sites.** ✅ **BUILT 2026-09-16** (`INLAND_SITES = 1`,
   `terrain::pick_minor`'s second loop). A placement solve over the ISLAND
   rather than the ring — a polar lattice of `INLAND_CANDIDATES = 32`
   bearings × `INLAND_RADII = 4` radii inside `INLAND_R_MAX` — with the
   pairwise separation rule and the explicit tier field that landed as the
   roster, which is `MONUMENTS.md` §9.3's list. Fills on 16 of 16 seeds.
   **What did NOT land is Devblog 188's distribution objective**: ours is the
   pad's own score (footprint relief plus a height weight), minimized greedily
   against the roster, which is a *local* quality measure where theirs
   optimizes the whole set's count. At one inland site the two are the same
   thing; at five they are not, and §9.3 of `MONUMENTS.md` is still the gap.
   The tier carries no containers — the ladder in `terrain.rs`'s const block
   has one crate of headroom, so arming it is a spoken re-pricing.
2. **Ports.** ✅ **BUILT 2026-09-16** (`SideRoad::port`), and NOT as this
   line proposed. A port off a hash would be a rotation nobody chose; the port
   is the bearing the road's own search settled on, and the point it names is
   on the site's RIM (`WAYSTATION_RADIUS_M`) rather than at its centre — which
   is what a connection point IS (§4), and what keeps a carriageway from
   running over the canopy the site is made of. Only the inland tier has one:
   a ring site needs no port because no side road ends there, and an unused
   field is a claim nothing enforces.
3. **Side roads** ✅ **BUILT 2026-09-16** from the ring to each inland site's
   port, stored as §5's polyline — one segment, solved once inside `haven()`,
   held on `Haven` and queried as point-to-segment distance. No height tap, no
   trig, no state, and the client mirrors it for free exactly as it mirrors
   `Haven` today. The cost §9.3 priced is paid: `road_band` takes a `&Haven`
   and `ring_band` is the ring-only half a solver may ask.
4. **Trails** last, if at all. Their third tier exists at a map scale four
   times ours.

### 9.3 · What our architecture buys, and what it costs

A polyline solved at init and stored is **not** a departure from our walls —
it is what `Haven` already is. `terrain::haven()` runs a bearing scan with a
march and a bisect, costs ~1,000 height taps, runs once at `World::new` and
once per client chunk batch, and hands back a small `Copy` struct that every
later query reads. A road path is the same shape of thing and can ride in the
same struct.

What it costs is the thing to say plainly: **the road stops being a pure
function of `(seed, x, z)` alone and becomes a pure function of `(seed, x, z,
Haven)`** — exactly as `scatter` already did when the pad started vetoing
cells. Every call site that resolves a road today would need the haven
threaded, which `scatter` and `clutter` already carry.

### 9.4 · One thing of ours to keep — and the half of it that was wrong

Their roads wander because their terrain fights them; Devblog 50's first
version took "pretty crazy routes". Ours does not have that problem (§7.1),
and a straight segment between two points is legible, cheap, and gateable in a
way a fitted spline is not. **We should not import the spline.** Straight
segments with a port at each end give us §6's T and Y for free — three
segments meeting at a ring point IS a T — without a search we cannot afford
and cannot gate.

⚠ **Everything above is about GRADE and COST, and it was read as being about
shape.** It is kept as written because it is still true and still the reason
there is no solver here. What it never claimed — and what nobody checked for
three weeks — is how the result LOOKS. The operator booted the game on
2026-09-17 and reported *"just a road going straight across the world. it did
not look good at all"*, and the shipped seed says why in one line: both of the
depot's approaches land on `z = 899.3`, an axis-aligned chord **1,760 m across
a 2,048 m island**. A section arguing that we do not need their search is not
an argument that a ruler is acceptable, and this one was cited as though it
were.

So `SideRoad` is a five-node polyline since side road bend v0
(`DECISIONS.md` 2026-09-17), and **the paragraph above still holds over it**:
there is no spline, no fitted curve and no search. The straight solve picks
the same junction it always did; the three interior nodes take a hashed
lateral capped by `SIDE_ROAD_BEND_M`, each candidate re-validated by the same
`road_corridor_clear` that admitted the chord, and the fallback is the chord —
so the worst case is exactly the road this section designed. §6's T and Y are
unaffected: a leg meeting a ring point is still a leg meeting a ring point.

One thing this correction does **not** fix, because it is not a road problem:
the two approaches leave the yard 180° apart and always will, since
`depot.rs`'s two gates share the compound's local Z axis. The pairing is the
building. What changed is that the road no longer follows the line between
them.

### 9.5 · Ranked, and what is not owed

1. ~~**Inland site placement** (§9.2.1)~~ — ✅ built 2026-09-16, and gates 6
   and 7 with it. Everything below was blocked on it and is not now.
2. ~~**Ports and side roads** (§9.2.2–3)~~ — ✅ built 2026-09-16. Unserved
   land 38.2% → 28.3%, p90 walk 572 → 451 m.
3. ~~**Gates 1–4** (§8) with their mutants~~ — ✅ built with the mechanisms
   (`tests/side_road.rs`, five tests, five mutants run). Two of the four had
   to be restated rather than implemented: see the ⚠ marks in §8.
4. **Redundancy** (§6's Devblog 180 quote) — connect a dead end back to the
   ring so the network forms circles. Their own stated next step, and now ours
   in a specific form: our side road is a dead end at the site, and the ring
   it joins is itself in 4–18 walkable pieces. A second road off the same
   site, refused the first one's junction, is the smallest version of this.
5. ~~**The ring's own fragments**~~ — ✅ **largely BUILT 2026-09-18 as ring
   path v0** (`DECISIONS.md`): the ring became a solved polyline and its
   standable share went 94.2% → 97.9%, longest unbroken run 39.9% → 55.6%.
   §9.3's "ours is a predicate" is retired for the ring; §5's *"take the
   polyline"* is what shipped. What remains is the last ~2%, which needs a
   bench — `NOW.md` §0ring item 2. The diagnosis below stands as written:
   ⚠ **re-measured 2026-09-17 and this row was
   wrong twice, in opposite directions.** Its cause sentence — *"broken where
   it crosses cliffs"* — is **right**, and confirmed: ablate the unwalkable
   cells and the biggest unbroken arc goes 54.4% → **94.8%** mean over eight
   seeds, five of them to 99.8–100%, while ablating radial gaps moves nothing
   (they are 0–2 per seed, p99 gap 0.0 m). Its NUMBERS were instrument
   artifacts and no count of "pieces" in this doc should be trusted: a
   component count over ring cells reads **43.5% at a 2 m grid and 13.3% at
   4 m**, and an angular sweep built on `yaw_dir` samples 256 bearings however
   many steps it claims (the LUT has 256 entries), which read the ring as
   9.3% in one piece with radial gaps as the cause — both numbers were the
   sampling. What is stable to a tenth of a point across both grids is the
   share of the ring a player can stand on: **90.7–100.0%, mean 96.2% over
   twelve seeds**, so 3.8% of the ring is on ground that holds nobody.
   **It is gated now** (`tests/road.rs::the_ring_is_ground_a_player_can_stand_on`)
   and the instrument is `examples/ring_breaks.rs`, which carries both traps.
   The fix is a road BENCH, and the measurement that says so is the gradient
   split: at a cliff cell the ground climbs **3.8–35× more steeply across the
   road than along it** (198 radial against 0 tangential on the shipped seed),
   so the road's length is walkable and the road is simply cut into a slope
   with no shelf under it. What it is blocked on is §9.3 — a bench needs a
   continuous centre line to key off, and a predicate-derived one inherits the
   shoreline's forks (0–470 bearings per seed). `NOW.md` §0ring.
6. **Trails** — a scale we do not have.

**Not owed by this doc:** any number reaching `content/`, and any decision
about what an inland site LOOKS like. The visual register is `WORLD.md`'s and
`WORLD.md` is unadopted design with the operator's own ceiling on it
(`WORLD.md` §9.1: decide the register early, build it late). A greybox
inland site is builder work; a monument is not.

---

## Sources

Fetched whole 2026-09-16 unless marked.

- Devblog 50 — https://rust.facepunch.com/news/devblog-50 (roads taking "crazy routes"; rivers; biomes by latitude and altitude)
- Devblog 63 — https://rust.facepunch.com/news/devblog-63 (new roads; pseudo-erosion; "barrels only spawn next to roads and monuments")
- Devblog 180 — https://rust.facepunch.com/news/devblog-180 (T and Y intersections; main vs side roads; redundancy as the next problem; wanting monument connection points)
- Devblog 188 — https://rust.facepunch.com/news/devblog-188 (monument placement simulated and optimized for count; monuments specify connection points)
- Devblog 189 — https://rust.facepunch.com/news/devblog-189 (roads connect to monument roads via in/out points; rivers, gaps and flooding fixed; "huge areas of wasteland")
- Devblog 197 — https://rust.facepunch.com/news/devblog-197 (procedural spawn rules tuned so rivers, forests, rocks and monuments cohere)
- Wiki, Procedural Generation Customization — https://wiki.facepunch.com/rust/procedural_generation_customization (MainRoads / SideRoads / Trails and the rest of the toggles)
- Wiki, The Map — https://wiki.facepunch.com/rust/map ("a ring around the map branching off to connect to monuments")
- Wiki, Map Data — https://wiki.facepunch.com/rust/Map_Data (`PathData`: nodes, width, spline, splat, topology, start, end)
- ⚠ **Tier 3** — Rustafied, "Ring Road, ho!", 2020-02-06 — https://www.rustafied.com/updates/2020/2/6/ring-road-ho (2 lanes, maps over 3k, single-lane branches, no building on roads)
