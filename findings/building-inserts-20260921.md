# Building inserts and uneven foundations, 2026-09-21

Metal window bars occupy a window socket; garage doors occupy a wall frame.
Each consumes its crafted item and can be damaged independently of the frame.
Bars retain actual shooting gaps. A garage door uses the existing lock,
open/close, repair, pickup and support-removal paths. Its open mesh fits inside
the frame header, outside the traversable opening. Placement previews and
standing objects use the same insert geometry as projectile collision.

Client mirrors rebuild insert collision after delayed definition tables and
piece resyncs. Projectile impacts distinguish the insert from its supporting
piece, so hitting a closed leaf damages the leaf. Save loading restores the
same collision from existing records. Wire v69 admits the new archetype and
placement codes; it changes no field widths or save layout.

At a height boundary, the wall foot extends down to an adjoining supporting
floor, including upper storeys and touching triangle halves. The renderer
draws the same extension without lowering the head or opening. Aiming below
a solid wall's midpoint continues its nearest end at the same storey; aiming
above stacks it. Doorways retain their insert and top sockets.

## Content sources

Craft inputs, workbench tiers, craft times, stack sizes and research-table
prices follow the Facepunch item pages for
[metal bars](https://wiki.facepunch.com/rust/item/wall.window.bars.metal) and
[garage doors](https://wiki.facepunch.com/rust/item/wall.frame.garagedoor).
The inherited implementation uses 500 and 600 hp respectively, recorded in
DECISIONS.md. The one research price serves both the table and bench tree;
the tree supplies an acquisition route without a loot specimen.

## Validation

The simulation suite covers both edge axes and ground/upper storeys, shooting
gaps, independent destruction, wrong-socket inventory conservation, locks,
support removal, save/load and the wall-foot extension. Client tests cover
real aim rays, late definitions, index rebuilds, refused door prediction and
mesh bounds at raised foundations and upper floors. `./ci/gates.sh` completed
with ALL GATES GREEN on 2026-09-21, including native renderer tests, browser
builds and byte-identical native release/debug/Wasm probes. The local log is
`/tmp/gates-inserts-gates-20260921.log`.

## Graphical playtest

A matching v69 native client joined a local shard with a saved two-storey
fixture (21 pieces and four inserts). Captures `0-design.png` and `8-build.png`
under `/tmp/gates-building-playtest/shots/` show bars seated in both floors'
windows, a closed garage filling its frame, and the upstairs open garage
rolled inside its header. These are automated visual-fit captures; they do
not claim a human has played the lock, firing or hammer interactions.
Client and shard logs contained no game errors. The local fixture uses a
copied content directory.

The interactive review route is:

Use a matching local server and client. Build two storeys with the floor-frame
stairwell, place bars and a garage door upstairs, lock/unlock and open/close it,
then rejoin. Fire through a bar gap and into a bar. Compare a wall spanning a
foundation height boundary before and after its lower neighbour is removed.
Judge the beside/above snap and hard/soft face treatment together with the
hammer rotation checks in building-tools-20260920.md.

The remaining catalogue work is separate: glass and shutters; half and low
walls with real partial-height support; stepped foundations, ramps, additional
stair forms and the triangular floor frame. Half-height support must not be a
short mesh holding a floor at the old full-storey altitude.
