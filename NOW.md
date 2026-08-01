# Gates · NOW.md — what next

The only list that answers "what should the loop pick up." Top item first.
Done items are deleted, not checked — history lives in git and
`DECISIONS.md`. A loop iteration starts here, ends with gates green.

1. **Materials and the surface read.** Lighting v0 landed (key + bounded
   texel-snapped shadow map + tone map, `DECISIONS.md` §open), so the world
   now has shape. What it has no *surface*: everything is
   `MeshLambertMaterial` at a flat colour: no roughness, no normal detail,
   no splat blend on the terrain (`TERRAIN.md` §5 already specifies
   height/slope/noise blending in-shader), and the scatter pines are one
   solid green. `threejs-procedural-materials` and
   `threejs-procedural-fields` are in `.claude/skills/` (MIT, Scott Sun —
   credited in `CLAUDE.md`). Same rails: constants into `DECISIONS.md`
   §open, stay inside the `DESIGN.md` §9 budget the browser gate now
   asserts, no fps quoted from this box.
2. **Shadows past 80 m.** The single map is bounded at an 80 m radius by
   design, so a hill or a base further out casts nothing. The clipmap in
   `threejs-shadow-systems` is the fix (concentric levels, committed
   centres, cached coarse updates); the near level already snaps, so this
   is adding levels, not rewriting. Wants the draw-call budget watched —
   the gate reports 58 of 300 today.
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
