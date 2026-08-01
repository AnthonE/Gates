# Gates · NOW.md — what next

The only list that answers "what should the loop pick up." Top item first.
Done items are deleted, not checked — history lives in git and
`DECISIONS.md`. A loop iteration starts here, ends with gates green.

1. **The horizon still casts nothing.** The shadow clipmap landed (two
   levels, 80 m and 240 m, committed centres, cached coarse updates —
   `DECISIONS.md` §open), so shadows now reach the edge of what *casts*:
   the 5×5 near ring, ±192 m. Past that the far mesh receives and never
   casts, by lighting v0's call — two disagreeing LODs of one hillside in
   one map is acne — so a mountain at 400 m is still lit flat. Fixing it
   is a terrain job before it is a shadow one (a skirt, or a shadow-only
   proxy at the near↔far seam); then the level table takes one constant.
   Budget headroom: the gate reports peak 101 of 300 calls today.
2. **The surface, second pass.** Materials v0 landed (four authored
   identities, one shared noise field, every channel from the same
   causes — `DECISIONS.md` §open), so the ground has a surface and the
   forest is a forest. What it still has no *texture*: the identities are
   analytic, so at arm's length the ground is smooth-shaded mottling with
   no grain. `threejs-procedural-materials` (triplanar, atlas filtering)
   and `threejs-procedural-vegetation` are in `.claude/skills/` (MIT,
   Scott Sun — credited in `CLAUDE.md`). Same rails: constants into
   `DECISIONS.md` §open, stay inside the `DESIGN.md` §9 budget the browser
   gate asserts (58/300 calls, 492 k/1.5 M tris today), no fps quoted
   from this box.
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
