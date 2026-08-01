# Gates · NOW.md — what next

The only list that answers "what should the loop pick up." Top item first.
Done items are deleted, not checked — history lives in git and
`DECISIONS.md`. A loop iteration starts here, ends with gates green.

1. **The scatter is still analytic, and so is everything off the ground.**
   Materials v1 landed (grain at arm's length, projected on the surface,
   gated by neighbour contrast — `DECISIONS.md` §open), so the GROUND now
   has texture at every distance. Nothing else does: the pines are a
   merged trunk/skirt/crown with baked vertex ramps and a per-instance
   tint, the rocks are dodecahedra, and none of them carries a surface
   field at all — a boulder at 2 m is a flat-shaded facet. The same
   machinery ports (`threejs-procedural-materials` for a world-space field
   on a closed mesh, `threejs-procedural-vegetation` for leaf cards and
   trunk detail; both MIT, Scott Sun, credited in `CLAUDE.md`). Same
   rails: constants into `DECISIONS.md` §open, stay inside the `DESIGN.md`
   §9 budget the browser gate asserts (peak 147/300 calls, 1.05 M/1.5 M
   tris today — the horizon casting spent most of the headroom), and
   watch the fragment bill: the gate box runs three tabs on a software
   rasterizer, and materials v1 had to buy its grain back out of the micro
   octave to keep them all alive.
2. **Nothing casts past 720 m, and nothing out there has a silhouette.**
   The horizon casts now (`DECISIONS.md` §open) but two limits are stated
   rather than solved: the coarsest clipmap level stops at 720 m because
   fog closes at 1000 m, and past the near ring the only caster is the
   8 m ground itself — the scatter stops at the ring's edge, so a forest
   at 400 m casts nothing and the gate measures the horizon on 2 of 4
   yaws for exactly that reason. A scatter LOD (billboard crosses,
   `TERRAIN.md` §4's "trees get two LODs") is the fix and it is a terrain
   job, not a shadow one.
3. **M1 — survival verbs** + bags, hotbar, chat (`ALPHA.md` §1/§6).
   Gather, craft, build, and deployables are sim'd, on the wire, and
   solid (slice 13: chat — two channels on the reliable lane, local at
   20 m and global, sanitized at both edges and rate-limited per
   connection, deliberately outside the sim so a replay never depends on
   what anyone typed; wire v10 spends the last kind code on it, T opens
   the composer and `/g ` sends global, and the browser gate now types a
   line in one tab and reads it out of the other's DOM).
   Next: **shared access** — one owner id gates a door and a hearth
   today, so two friends cannot share a base; whether that arrives as a
   code lock, a hearth auth list doors inherit, or crews needs the
   operator's word (`DECISIONS.md` §open, lock v0 row) ·
   death/backpack/respawn-on-bag (bags place + cap now; the anchor lands
   there) · piece damage (M2's raid lane: hp exists and decays, nothing
   attacks it yet) · nametags (chat names a speaker by id today, because
   nothing has a name yet).
4. **M2 — combat true**: lag-comp ring + rewound raycasts · ballistic
   projectiles · satchel + damage-by-tier · day/night · netem feel bar.
5. **M3 — economy dark + ops**: OBOL machinery behind the A1 switch ·
   admin lane · backups · status page · error capture · `bench_transport`.
6. **A1 playtest** (operator schedules): 10–20 testers, one wipe cycle —
   then tune content bands from what the anomaly log and the replays say.
7. **M4 — arm A2, then A3** (operator acts): claim rail export · skin
   catalog · the board delivery (repo + playable link + a recorded round
   whose replay hash checks) on `munus-first-sale`.

Standing rule: anything a playtest breaks jumps this queue; anything a
wall catches jumps the playtest.
