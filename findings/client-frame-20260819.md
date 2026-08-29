# The client's frame, measured and cut — 2026-08-19

*Client lane. Every number here is release, on this box (4 cores, load ~1–3),
and every one of them has the command that produced it beside it. Re-run
rather than trusting the digits — `CLAUDE.md`'s own rule, and this note has
already been wrong once for exactly the reason §0 gives.*

This is the detail behind `NOW.md` §0pf. The item there is the queue; this is
what the queue cost, what it costs now, and the three things that were true
about it and are not any more.

---

## 0. ⚠ The tree was carrying an uncommitted measurement scaffold, and every
## terrain number taken on it was ~30% high

`crates/sim-core/src/perfcount.rs` existed in the working tree of this branch
— untracked, `git cat-file -e HEAD:` says it is not in the commit — with its
own first line reading *"TEMPORARY measurement scaffold (not for merge)"*. It
put a `static AtomicU64` `fetch_add` inside `terrain::height`, inside
`noise2`, and inside `cell_hash`. One `height` call is 1 + 12 + 48 = **61
contended atomic RMWs on two shared cache lines**.

Measured both ways on the same source: one `water::stream` sweep was 6.10 ms
with the scaffold and 4.25 ms without, on output verified bit-identical —
**1.85 ms of 6.10 was the instrument**. The first baseline in this pass was
taken on the polluted tree and read `clutter_fill` at 2.912 ms; the clean
number is 2.870 ms, which is *close enough to look fine* and is not the same
measurement.

The lesson is not "delete the scaffold" — it was deleted. It is that a
counter cheap enough to leave in is a counter that changes what it counts, and
that the first thing to do before quoting a timing on a shared box is
`git status`.

---

## 1. Where the frame actually went

    cargo run --release -p client --features render --example frame_cost

Every row is an A/B on this box: the "before" column was produced by putting
the plain entry points back in the same build and re-running the same probe,
never by quoting an older number from a different tree.

| what | before | after | |
|---|---|---|---|
| `terrain::clutter_fill`, one 16 m tile | **2.870 ms** | **1.006 ms** | 2.85× |
| `terrain::skirt_fill`, one tile | 0.063 ms | 0.040 ms | 1.58× |
| both, over a 5×5 tile ring | 74.1 ms | 27.7 ms | 2.67× |
| `water::stream`, a walking snap | 3.309 ms | **0.965 ms** | **3.43×** |
| `water::stream`, a teleport (nothing carried) | 3.309 ms | 2.226 ms | 1.49× |
| `heightfield`, near chunk 65² @1 m | 5.874 ms | 5.153 ms | 1.14× |
| `heightfield`, far mesh 257² @8 m | 203.1 ms | 152.7 ms | 1.33×, **and off the frame** |
| `terrain::height`, one call | 399 ns | 399 ns | unchanged **on purpose** |
| `terrain::height` through a `Lattice` | — | 262 ns | 1.51× |
| `terrain::haven(seed)`, once at world load | 5.21 ms | 5.16 ms | unchanged |

Three of these are the frame's and the rest are not, which is the distinction
that decides what any of it is worth:

- **The client fills one clutter tile per frame** (`render/clutter.rs`,
  `CLUTTER_FILLS_PER_FRAME = 1`), so row 1 is a streaming frame's clutter cost:
  2.9 ms → 1.0 ms.
- **`water::stream` runs on the frame that crosses a `SNAP_M` cell**, about
  0.6 times a second at a walk. 3.3 ms → 1.0 ms.
- **`heightfield` is not on the frame at all any more.** Its rows are the
  pool's cost now, which is why the smallest ratio in the table sits beside
  the largest win: the far mesh went from a ~190 ms frame *with the session
  pump inside it* to no frame at all, and a 25% arithmetic saving on top is a
  rounding error next to that.

**`terrain::height`'s own cost is deliberately unmoved.** Every public entry
point still hashes every corner every time; the memo is a second entry point
(`*_memo`) that a caller opts into. That is what makes the change provably
free of consequence for the ~50 call sites in four crates that nobody in this
pass looked at.

**And the memo is worth far less to the mesh than to the population** — 1.14×
against 2.85×, on the same function. `heightfield` already shares its taps
aggressively (3.09 per vertex, none duplicated inside one call), so what is
left for a hash memo to remove is thin; `clutter_fill` was resolving the same
few lattice quads tens of thousands of times. A memo pays where the caller
repeats itself, and the honest way to find out how much is the A/B above and
not the ratio measured on an isolated function.

## 2. The memo: what it is and what it measured

`terrain::Lattice` is a caller-owned, fixed-size, direct-mapped table of
lattice **quads** — the four corner gradient indices for one `noise2` cell,
packed three bits each into a `u16`. One lookup replaces four `cell_hash`
calls (twelve chained `splitmix64` rounds).

It is worth having because a caller almost never samples once: `ground_slope`
is four `height` evaluations 2 m apart, the far mesh is seven taps a vertex,
and a clutter tile resolves 625 cells inside 16 m. The coarsest lattice
`height` reads is 1,200 m across and the finest is 24 m, so **a 16 m tile
touches ~70 distinct quads and draws them tens of thousands of times.**

Sizing, measured on the far mesh's own traversal (257² vertices at 8 m pitch,
seven `ground` taps each):

| slots | hit rate | wall |
|---|---|---|
| 256 | 97.51% | 140 ms |
| 512 | 98.41% | 132 ms |
| **1,024** | **99.25%** | **128 ms** |
| 2,048 | 99.70% | 128 ms |
| 4,096 | 99.79% | 127 ms |

1,024 is the knee and it is 20 KB of slots — small enough to stand on a wasm
build's shadow stack, which is 1 MiB with no guard page and which this repo
has blown before.

**Two things that looked like wins and were not**, recorded so the next pass
does not re-spend them:

- **Restructuring `fbm` so its octaves are visibly independent** (evaluate all
  five `noise2` into locals, then sum in the same order — bit-identical) was
  measured at **1%**. The dependency chain was not the bottleneck.
- **Sharing one `Lattice` across a whole 5×5 tile ring** rather than one per
  `clutter_fill` call measured **27.2 ms against 27.7 ms** — i.e. nothing, and
  slightly worse. A tile already touches ~100% of its own quads; there is
  nothing left for a neighbour to donate. `clutter_fill` therefore builds one
  on its stack and the ring-level entry point exists only because the caller
  might want it.

---

## 3. The two clutter refusals

**The rich stratum refused 87.5% of its cells after resolving the ground they
stand on.** `clutter_rich_cell` drew an acceptance byte, resolved `ground`, a
`moisture`, a four-tap `ground_slope` and a `clump` to compute a rate, and
then compared the byte against it. The rate cannot exceed `RICH_ACCEPT_MAX`
(32 of 256) — `splat_from` normalises its four bytes to 255, so
`grow ≤ 256`, `clump` is clamped to 1, and `256 × 32 / 255` floors to 32 — so
a byte at or above the ceiling was always going to lose. Refusing there is the
same refusal one tap sooner and it is **42% of a tile's whole `height` bill.**

**The acceptance rate and the kind read the same four splat weights**, and
resolved them separately: a second `moisture` and a second four-tap
`ground_slope` on every cell that survived. One `splat_from`, two laws.

`skirt_fill` had the same shape one level out: it built each element in full —
a `ground` tap and a slope fan — and then discarded three quarters of them on
a tile-ownership compare that needs no terrain at all. Asking *where* before
*what* is `skirt_pos` / `skirt_at`.

**None of this is gated by anything that existed.** `clutter_fill` is not in
`state_hash`, not in `test_terrain_golden`, not on the wasm parity diff, and
`tests/clutter.rs`'s determinism check compares the fill against *itself*.
`crates/sim-core/tests/lattice.rs` is the evidence, and §5 is why its first
draft was worthless.

---

## 4. The client, off the main thread

**The far mesh was one ~190 ms frame with the session pump inside it.**
`heightfield` is pure — it reads `sim_core::terrain`, touches no ECS and
allocates only what it returns — and everything it captures is `Copy + Send`
(`Haven` is 64 bytes of f32/u8/bool). It runs on `AsyncComputeTaskPool` now,
along with every near chunk, so the ground costs the main thread one
`meshes.add` and one `spawn` per landing.

Two flags were honest only while the build finished inside the statement that
started it, and both produce no error:

- `far_done` was set **before** the build. It is what `far_ready` reports to
  the loading bar, which is what ends the loading screen. Split into
  `far_started` (guards the spawn) and `far_done` (set when the mesh reaches
  the world).
- `built` was the only test for "is this chunk handled" and it is written when
  the mesh exists — so an async build leaves a window in which a key is in
  neither map and the ring re-queues it every frame.

`crates/client/tests/ground_async.rs` gates both, on a bare `App` with
`MinimalPlugins` (which carries `TaskPoolPlugin`).

**The sea carries its last sweep across a snap.** `NOW.md` §0pf item 2 says
*"the sea's axis is non-uniform, so there is no half-lattice left to share —
off-thread or coarser, not cleverer"*, and that is true of the skirt and false
of the core: `SNAP_M` is 8 m, `STEP_M` is 2 m, and a one-cell step along one
axis slides every core coordinate onto the coordinate four slots away at the
**same `f32` bits** — `ox` is an exact multiple of 8 and a core coordinate an
exact multiple of 2, both far under 2²⁴. Four caches and a rate slide;
the skirt, the four columns that entered the core, and every diagonal
rebuild. Every identity it rests on is CHECKED per index rather than argued,
which is `terrain_mesh::heightfield`'s own `share_x` pattern.
`crates/client/tests/water_carry.rs` walks the grid and compares it against
one built from scratch at the same place, value by value.

---

## 5. ⚠ The first draft of the gate was green under two of its own mutants

`crates/sim-core/tests/lattice.rs` compares `clutter_fill` against a naive
rebuild written out the long way. Its first draft called
`terrain::clutter_rich_cell` for the per-cell law — so **both sides of the
comparison carried the mutant**, and a threshold moved by one (refusing a cell
whose roll is `RICH_ACCEPT_MAX - 1` over ground rich enough to accept it,
which is 36% of land at 1-in-256 rolls) passed every assertion in the file.

A rebuild that shares the function under test is a rebuild of nothing. The fix
is `refused_by_the_law` and `kind_by_the_law`, written from published parts —
which is why `clutter_rich_draw`, `clutter_kind_at`, `kind_from_splat` and
`clutter_richness_at` are now `pub`, and why `clutter_rich_draw` returns a
struct with every slice **named** rather than a tuple of bytes.

**The sea's gate had the same disease in a second form.** `water_carry.rs`
walks the grid and compares it against a freshly built one — right, and
satisfied by never carrying anything. The mutant that derives the index shift
wrong makes `carry_of` refuse every index, so the sweep rebuilds: correct
output, no saving, ten green tests. A safety check that turns a bug into a
fallback makes that bug invisible to every assertion about values. `Sea::
carried` is the fix — one count of vertices, asserted as a floor on a one-cell
snap and as zero on a diagonal — and it is a count rather than a time, so the
no-clock rule is untouched.

The mutant table is in the suite's own header. One row is GREEN and stays
green: the hoisted tile range check cannot be caught, because
`CLUTTER_CELLS_PER_SIDE` is an exact multiple of `CLUTTER_CELLS_PER_TILE` and
every tile the check could wrongly drop is ocean. That is an equivalent mutant
rather than a hole, and `the_tile_grid_divides_the_cell_grid` pins the premise
that makes it one.

---

## 6. What is still open

- **`ground_slope`'s four-tap stencil is now ~80% of what a clutter tile
  spends.** Any change to it changes every splat byte, therefore every clutter
  kind, therefore the drawn world — a design change needing operator sign-off
  and regenerated goldens, not an optimisation. Recorded so the next pass
  knows where the remaining mass is and why it was left.
- **`water::animate` marks the sea's mesh modified every frame**, which deep-
  clones ~677 KiB into the render world and churns its GPU slabs. There is no
  flag that fixes it — a mesh mutated from the main world must keep
  `MAIN_WORLD` usage and must therefore be cloned on every modification. The
  only structural fix is moving the swell into a vertex shader, which
  `render/water.rs` already names as a slice. `water.rs`'s own doc claims "no
  allocation" per frame; that is true of the system body and false of the
  frame.
- **`resolve_field` is 84–87% of `animate`'s CPU and it is 18,516 `sincosf`
  calls.** The obvious `band_weight` hoist measured 0.3% ± 0.5% — a measured
  non-win. Folding `k` into the wave direction is 10% and changes 4,886 of
  7,921 vertices' bits, so it is a design change and `tests/water.rs` is red
  under it by construction. The lever is the shader.
- **No GPU has ever run this client**, so every number in this note is CPU.
  `NOW.md` §0u is the other half and is untouched by this pass.
