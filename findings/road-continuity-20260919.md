# Closing the coast road's terrain gaps

The solved horizontal ring stays in place. A periodic cubic height profile
now grades the existing shoulder and blends back into the island over 12 m.
The profile is solved once from raw terrain and stored beside the radii;
queries examine five nearby segments and read no extra terrain samples.
A smooth union of compact masks avoids creases where segment distances tie.

Monument approach masks preserve the existing site-carve gradient walls.
Where an approach would leave wet carriageway, shallow fill raises the core
to the existing land line. This is a whole-ring bench, with local cut/fill,
not a list of special-case coordinates. Terrain outside the 17 m band is
unchanged. The extra height array grows the measured World size
from 56 to 57 decimal kB; the configured shadow stack remains 4 MiB.

The continuity test walks centre and both carriageway edges every half
metre across 48 seeds, including all segment joints. Every sample must be
above the land line, below the existing cliff limit, and pass a quarter-metre
rise/run check in both directions. A separate all-segment distance scan
checks unchanged bits outside the band. Existing road and monument-gradient
checks retain their limits. The old raw-ground assertion now excludes the
road's explicitly added footprint as well as site footprints; the new
outside-band test covers that boundary independently. Disabling grading
deliberately fails the carriageway test on seed 1; restoring it passes.

Replay final/trace and terrain goldens intentionally move with generated
ground. The native/wasm site probe includes cached control heights and the
ground sampled at nodes. This does not assert obstacle clearance or measure
frame time. A player's lap and visual assessment of headland cuts remain
useful. Deployment onto an existing world requires the operator to decide
world compatibility; this branch performs no wipe or publication.

Manual native view: seed 20260731, (1486.52, 315.02), looking east along
the route. This is the measured largest centre-line cut in that seed
(about 7 m). The carriageway and gravel shoulders form a continuous
surface through the headland. The grade remains visibly steep; this is a
walkability limit, not a vehicle-road design. The capture used High/75%
at 1280×720 with the separate render-scale change.
