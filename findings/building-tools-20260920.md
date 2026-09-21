# Building tools, 2026-09-20

The concurrent batch completes the hammer's Rotate verb, target and upgrade
price readout, visible hard/soft wall faces, and held-lock targeting for doors
and boxes. Simulation, wire/server, and client each had one owner; assets,
documentation and final integration had a separate owner.

Rotation reuses the existing placement grace window and claim/reach checks.
It preserves hp, upkeep, placement time and attached objects. Stairs move
between the four existing direction addresses; edge pieces flip facing in
place. The existing remove/place messages carry these changes. Wire v68 adds
one five-byte action; the save format is unchanged.

Integration found that placement/upgrades sent the store's zero damage band,
while full sync derived it from actual hp. That broadcast now uses the same
derivation, so rotation does not visually heal a damaged piece.

A second ordering bug appeared when stairs were rotated and removed in one
tick. The server fabricated a placement record for the now-absent address
with plate zero; inserting it changed the client's entire column height.
Absent final records are now skipped, and the following removals carry the
absence. Raised-base collision is checked after this exact command sequence.

The hammer quotes the received content's next upgrade row. It does not quote
an exact repair price: the client receives damage bands and lacks the repair
percentage. The soft/hard treatment uses the existing FACE_TOP/FACE_BOTTOM
gains on shared mesh variants without changing collision or UV scale.

## Catalogue boundary

A half wall is more than a new shape code. `build::supported` supports a
floor from an edge at `level - 1`; every current level is a whole storey,
and the collision masks identify whole-level edges. A useful half wall needs
its upper socket, partial-height collision, matching support/collapse rules
and targeting, not a short mesh supporting a floating full-storey floor.
The twelve shapes also fill all 48 current piece definition slots.

Window bars and garage doors need placement classes and insert geometry
shared by movement, shots, deploy targeting and rendering. They remain
separate catalogue slices after this completed tools batch.

## Combined playtest

Use matching server and client builds from this branch (wire v68).

Build two storeys using stairs and floor frames. Rotate a placed flight,
climb and descend through the opening, then close a ceiling and check
headroom. Flip a damaged wall and confirm that its visible soft face swaps
without its damage changing. Check target/upgrade prices while moving between
pieces, and place locks on a door and an upstairs box. Rejoin and reload the
saved base; another player should see the same directions and faces.

## Automated validation

`CARGO_BUILD_JOBS=4 CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 ./ci/gates.sh`
completed with **ALL GATES GREEN** on 2026-09-20.

- 2,137 release workspace tests and 1,260 rendered-client test executions
  passed. The existing vendored SDK doc example remains ignored in both runs.
- Native and browser lint/build checks passed, including the browser renderer
  and audio worklet.
- Native release, WebAssembly and native debug digests matched. The gate
  requires eight successful rotation actions, alongside its existing
  movement, combat, respawn, revive and stair-headroom coverage.
- Allocation, replay, save/load, protocol goldens, multiplayer resync,
  event overflow, real aim rays and hammer panel/schedule checks passed.

An earlier renderer link exhausted the disk. Removing stale Cargo executable
and incremental caches allowed the complete rerun above; no source or check
was changed to address that environment failure.

Visual readability of the wall faces and wheel awaits the combined playtest
above; no interactive graphical playtest was run during this batch.
