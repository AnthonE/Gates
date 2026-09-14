# The trees wore two hulls — 2026-09-13

**Every claim here carries the command that produced it** (`findings/README.md`).

## What the operator saw

Two frames of the browser client, 2026-09-13: every tree near the player was
a smooth, faceted, opaque green dome with a real textured trunk under it and
leaf sprigs poking through its skin. The first reading of the frame blamed the
browser's 35 m tree-LOD swap and moved that rung to 55 m (`quality.rs`). A
hull at arm's length is inside any swap distance, so that could not have been
the cause.

## What it was

`props::stream` keeps two rings: a 5×5 near ring (trunk, canopy, hidden stump,
and a hull whose `VisibilityRange` opens past the swap distance) inside an
11×11 outer ring of hull-only trees that carry no range and are never culled.
The outer ring's `retain` asked one question — *is this chunk still inside my
radius* — which every chunk the player walks INTO answers yes. So the near ring
built the real tree and the outer hull stayed on top of it, in every chunk
entered after spawn, since the outer ring landed (2026-08-25).

The capture probe stands where it spawned, where the near ring builds first
and the outer ring skips its keys, so no frame it ever shot could contain
this. The operator had walked to a base.

## The frames that confirmed it

```
cargo build --release -p client --features render --example tree_look
Xvfb :99 -screen 0 1280x720x24 &
VK_DRIVER_FILES=/usr/share/vulkan/icd.d/lvp_icd.json DISPLAY=:99 WGPU_BACKEND=vulkan \
  target/release/examples/tree_look /tmp/shots
```

Nine frames: each variant's near pair alone, its hull alone, and both drawn
together, from 5 m and from 26 m. `both_broadleaf_5m.png` is the operator's
frame — a 105-triangle lathe dome over a 3,800-triangle leaf-card crown, the
sprigs escaping where the hull is a tenth narrower than the tree.
`pair_broadleaf_5m.png` is the tree as designed. Not committed (a bench, not a
gate — `CLAUDE.md`'s visual-gate rule); the command above regenerates them in
about a minute under lavapipe.

## The fix and its gate

`props::stream`: the near build takes the outer chunk down in the same frame
it builds (`ring.outer.remove(&key)`), and the outer retain asks both
questions (`in_outer && !in_near`). `tests/ring_handoff.rs` drives the real
streamer with an eye that moves three chunks and asserts no chunk is in both
rings and no cell carries two hulls, on every frame of the walk.

```
cargo test --release -p client --features render --test ring_handoff
```

Proven red under the old retain (both edits reverted, 2026-09-13): all three
tests fail — the outer ring settles at **111 chunks where 96 belong**, the 15
the near ring had taken over still held, and `no_frame_of_the_walk_shows_both`
fails on the first frame after the first step with a chunk in both rings.

⚠ The gate's first draft read **2** under the same mutant, not 15: its
`settle` returned on ring *counts*, and the old code keeps both counts steady
for the first frames of a walk (the near ring drops one chunk and builds one
per frame; the outer ring's count moves only once its own retain starts). It
had certified a ring holding the right number of the wrong chunks — the
`relief.rs` shape in `CLAUDE.md`. `settle` matches chunk keys against the
eye's position now.

## What it leaves

- **The capture probe should walk one chunk before it shoots.** A frame from
  the spawn chunk certifies the spawn.
- The browser's 55 m rung now stands on its own argument (`quality.rs`), not
  on the frame that moved it.
- The broadleaf hull is a lozenge wider than the tree's own crown at the
  shoulder (`hull_wide.png`); a person should look at the far forest now that
  it is only the far forest.
