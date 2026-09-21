# Completing the building catalogue, 2026-09-21

The build wheel has 21 shapes in all four material grades. The seven additions
are stepped foundation, ramp, L stairs, U stairs, square spiral stairs,
triangular spiral stairs and triangular floor frame. The existing straight
flight remains available. Glass/shutters and half/low walls are included in
this completion branch; their findings describe their own behavior.

Every circulation mesh reads the simulation's bounded planar patches. L and U
flights rise one storey with landings; spiral sections rise half a storey and
an opposite-facing section attaches directly above. Removing a lower section
collapses those it carries. A loaded section refuses hammer rotation. Steps
descend from their foundation plane and support only their high edge. Neither
foundation plate limit changes. The ramp reaches an addressable half-storey
landing; DECISIONS.md explicitly records that adaptation from the reference's
quarter-height ramp. Triangular frames retain solid rims and leave a real
opening for movement and projectiles.

The content has 84 piece rows. Prices use the existing reference ratios and
material hp; no per-shape balance is embedded in the simulation. Protocol v72
widens shape codes to five bits and regenerates every fixture. Earlier commits
in this branch record the v70 deploy-table and v71 half-storey transitions.

The source-domain completeness gate includes the new circulation module.
The wheel's cardinal-direction test now uses the nearest wedge centre for a
21-segment ring: ceil(n/4) was only correct for the previous cardinalities.
Clockwise order, the opposite wedge and every existing assertion remain gated.
The measured World-size note moves from 58 to 59 kB; the test is unchanged.
The HUD reports half-storeys as 0.5, 1.5 and so on. R/F turns every flight;
Shift+R/F adjusts foundation-step height without changing the plate bounds.

Focused tests cover walking each flight forward and back after every quarter
turn at ground, half and upper storeys; actual placement, loaded-rotation
refusal, save/load and dependent collapse; step-bearing edges and plate bounds;
frame centre/rim projectile and walk surfaces. The headroom probe includes
all new circulation kinds for zero-allocation and native/Wasm comparison.
Renderer tests compare actual transformed top-face triangles to those walk
surfaces and assert that triangle-frame faces leave the centre open.

The full headless suite passed. A native capture against the matching v72
shard shows the insert gallery and the new flights, including stacked spirals.
The fixture used real placement calls for 27 pieces and five inserts with
unchanged copied content. This is an automated geometry inspection, not a
human playtest of the controls. Full gates remain required before merge; the
first renderer-suite build exhausted local disk during linking, with no
test assertion failure. Disposable build output was cleared before retrying.
