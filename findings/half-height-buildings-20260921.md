# Half and low walls, 2026-09-21

A half wall carries a floor at its actual top. Two halves stack to the next
whole storey. Low walls provide cover without carrying a floor. Both work on
straight and diagonal sockets, use the existing material ladder and hammer
verbs, and refuse overlapping edges at offset heights.

The eight-storey height limit stays fixed. Whole-storey addresses retain codes
0..7; codes 8..15 are half a storey above those positions. One shared level_y
function supplies collision, deployables, blast heights, placement and drawing.
The column index widens its masks and distinguishes partial wall heights. The
collapse worklist includes the half and whole top sockets with a bounded
maximum of fourteen candidates. Existing save fields already carry a byte for
the level; content identity continues to protect changed row mappings.

Aiming at a half wall's upper face chooses its half-storey top. The lower face
continues the adjacent wall run at the same height. Floors and deployables keep
the half-storey they stand on. The tallest wall at a corner owns the post;
shorter neighbours must not erase the upper part of a full-height post.

Prices use reference/BUILDING.md's grammar: a half wall costs a full wall, a
low wall half. The proposed low-wall height and the expanded catalogue capacity
are recorded in DECISIONS.md. No foundation plate limit changes.

Targeted support, collapse, save/load, projectile, aiming, content and protocol
tests pass. The headroom parity/allocation probe also traverses partial-height
geometry. Full gates and graphical inspection remain required before merge.
