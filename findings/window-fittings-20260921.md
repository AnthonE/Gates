# Window glass and shutters, 2026-09-21

Glass and wood shutters complete the window insert choices alongside metal
bars. Glass seals the whole aperture. Shutters seal it when closed and Use
opens them; they cannot take a lock. Their open leaves fold beside the window
and the glass material is transparent. All four inserts share their placement
preview and standing mesh geometry with the simulation's aperture dimensions.

The column index distinguishes panes from bars. The distinction survives
delayed definitions, piece rebuilds, refused prediction, removal and save/load.
It adds two bounded bit masks, with no per-tick allocation or store scan.

The existing address model permits one insert per window. Bars, glass and
shutters are therefore alternatives. Stacking fittings on one socket would
need another address domain; it is not represented by overlapping records.

## Content

Craft inputs, workbench tiers, craft times, stacks and research follow the
Facepunch pages for [glass](https://wiki.facepunch.com/rust/item/wall.window.glass.reinforced)
and [shutters](https://wiki.facepunch.com/rust/item/shutter.wood.a).
The health values, 350 and 200, follow the corresponding RustClash item pages.
DECISIONS.md records the 32-row deploy-table capacity and proposed glass alpha.
Wire v70 widens deploy-row and definition-total fields and regenerates every
fixture, including definitions carrying both new archetypes.

## Validation

Targeted simulation, client-core and protocol suites pass. Tests exercise
both edge axes and ground/upper storeys, shooting through the positions that
are gaps between bars, open/close, lock refusal, independent damage, save/load,
late definitions and prediction rollback. Full gates and a matching graphical
playtest remain required before this branch is finished.
