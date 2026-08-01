# Gates · NOW.md — what next

The only list that answers "what should the loop pick up." Top item first.
Done items are deleted, not checked — history lives in git and
`DECISIONS.md`. A loop iteration starts here, ends with gates green.

1. **The surface, second pass — and the fragment budget in the way.**
   Materials v0 landed (four authored identities, one shared noise field,
   every channel from the same causes — `DECISIONS.md` §open), so the
   ground has a surface. What it still has no *texture*: the identities
   are analytic, so at arm's length the ground is smooth-shaded mottling
   with no grain. **It was built and measured on
   `loop/m1-surface-grain` (commit `d4232f6`, deliberately unmerged) —
   read that branch before rebuilding any of it.** A fourth octave of the
   shared field at a per-identity wavelength (sand 25/m, grass 8.3/m,
   litter 11/m, rock 16.7/m) with a per-identity ridge fold and contrast,
   driving albedo, roughness and a third bump octave, retired by pixel
   footprint in cycles-per-pixel so each identity dies at its own
   distance; plus a `grainProbe` that scores neighbour-to-neighbour
   CONTRAST over the pixels a toggle moved — the measure that separates
   grain from a wash, which the existing `surfaceProbe` cannot. It read
   20.9% of the near frame moved and contrast 0.22 → 2.68 luma/px
   (×12.2), 0.000% at 140 m, zero-noise control at both views. Triplanar
   sampling of the grain works too (ridge folded per plane, blend
   deviation restored by 1/|w|; 1.100 → 1.078 slope-to-contour on a 47°
   face at ×1.00 overall contrast, exact identity on level ground).
   **Why it did not merge, which is the real next problem.** The browser
   gate runs three tabs on four shared cores with a software rasterizer.
   `main` clears the public tab's 60 s join with ~no margin; the grain
   build does not, and the triplanar half alone measured ~9% of frame
   time (665 ms against 609 on one tree, versus `main`'s 508–587 band).
   Cutting grain's reach to ~2.5 m brought frame time back inside that
   band and the third tab still missed, so program size is a suspect
   alongside per-fragment cost. **So do the budget work first**: measure
   the terrain program's compile and fill cost directly and find the
   headroom — the 36-tap level-0 PCF (`shadows.js` `LEVEL_FILTER_TAPS`,
   `DECISIONS.md` §open) is the obvious place to look — then land grain
   on top of it. Rails unchanged: constants into `DECISIONS.md` §open,
   stay inside the `DESIGN.md` §9 budget the browser gate asserts (peak
   147/300 calls, 1.05 M/1.5 M tris), no fps quoted from this box.
   One thing measured on the way that is worth not rediscovering: the
   gate's `join()` poll (`page.evaluate` every 250 ms) can spend the
   whole 60 s window on itself when three tabs are live — the third tab
   once got **2 polls in 60 seconds**, one `evaluate` taking 7.1 s. Both
   obvious repairs are **worse**, and were measured: `waitForFunction`
   never gets injected, and a `context.addInitScript` timer that pushes
   through an `exposeBinding` does not run either (it fails the public
   tab on an unmodified `main` client, which the poll passes). Leave the
   poll alone until something explains why a page timer starves there.
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
