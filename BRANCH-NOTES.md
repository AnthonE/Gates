# Branch notes — `loop/scatter-splat-mix`

The slice is complete; this file carries no handoff. (It previously described
`loop/looks-pass1` / "prop photograph v1", which merged long ago — a stale
handoff note is a trap for the next pass, so it is replaced rather than kept.)

**What landed.** `TERRAIN.md` §1 stage 9's last open line — "`biome()` is
still a hard classifier, so a biome boundary is still a step in
*composition*" — is closed. `scatter` no longer selects a biome weight row;
it blends all four by the ground's own splat weights (`terrain::scatter_row`),
because `splat_from`'s channels are sand · grass · forest-litter · rock and
`Biome` is Beach · Meadow · Forest · Highland — the same four identities in
the same order. Stage 10's law for clutter, *the mix IS the splat*, now holds
for the prop population too.

**Measured** (`tests/scatter.rs`, three new checks): worst per-sample jump in
the tree weight across a moisture sweep **4 per-mille against the hard
classifier's 190**; **10.2–11.8%** of land cells sit in a transition band;
live slots moved at most 35 of ~9,800 across the four gate seeds.
`GOLDEN_TERRAIN_HASH` regenerated in the same commit.

**Left for whoever picks up the world lane next** — none of it started here:

- The **pad carve** (NOW.md §4b). Re-scoped this pass: `height` has **18
  production call sites in 3 crates**, not the "~80 in four" the file said.
  Whether a tier should carve at all is open for the operator
  (`DECISIONS.md` §open, waystation canopy v0).
- **One operator question** from this pass, in `DECISIONS.md` §open "scatter
  mix v0": the blend includes `splat_from`'s cliff term, so steep-but-walkable
  ground now draws toward the Highland row (scree/ore on slopes, matching the
  rock painted there). Taken as the "one law, three populations" reading; say
  if props on slopes should ignore the cliff mask instead.
- `combat.rs` has no occupant term and `collide::piece_ground` reads built
  pieces only — both tagged **systems lane** in NOW.md §0a and in
  `terrain.rs`'s own `SHELTER_FLOOR_IX` note.
