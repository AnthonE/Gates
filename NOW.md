# Gates · NOW.md — what next

The only list that answers "what should the loop pick up." Top item first.
Done items are deleted, not checked — history lives in git and
`DECISIONS.md`. A loop iteration starts here, ends with gates green.

1. **The surface, second pass — grain, and what the budget work found.**
   Materials v0 landed (four authored identities, one shared noise field,
   every channel from the same causes — `DECISIONS.md` §open), so the
   ground has a surface. What it still has no *texture*: the identities
   are analytic, so at arm's length the ground is smooth-shaded mottling
   with no grain. **It was built and measured on
   `loop/m1-surface-grain` (commit `d4232f6`, deliberately unmerged) —
   read that branch and its `BRANCH-NOTES.md` before rebuilding any of
   it.** A fourth octave of the shared field at a per-identity wavelength
   (sand 25/m, grass 8.3/m, litter 11/m, rock 16.7/m) with a per-identity
   ridge fold and contrast, driving albedo, roughness and a third bump
   octave, retired by pixel footprint in cycles-per-pixel so each
   identity dies at its own distance; plus a `grainProbe` that scores
   neighbour-to-neighbour CONTRAST over the pixels a toggle moved — the
   measure that separates grain from a wash, which the existing
   `surfaceProbe` cannot. It read 20.9% of the near frame moved and
   contrast 0.22 → 2.68 luma/px (×12.2), 0.000% at 140 m, zero-noise
   control at both views. Triplanar sampling of the grain works too
   (ridge folded per plane, blend deviation restored by 1/|w|; 1.100 →
   1.078 slope-to-contour on a 47° face at ×1.00 overall contrast, exact
   identity on level ground).

   **The budget work this list asked for is done, and it retired both
   suspects** (`DECISIONS.md` §open "fragment budget v0";
   `ci/browser_smoke.mjs` assertion 16). Grain did not merge because the
   browser gate's third tab missed its 60 s join, and the two named
   suspects were per-fragment cost and program size. Neither survives:

   - **The 36-tap PCF this list pointed at is a 16-tap PCF.** The
     constant was quoted from an older three; the installed r178's
     `PCF_SOFT` is sixteen `texture2DCompare` over a 4×4 footprint, not
     nine bilinear ones. It is now read off three's own installed chunk
     and throws if that branch moves. The whole clipmap costs **18**
     depth fetches a fragment, not 38 — so more than half the headroom
     that was hoped for here was never there to take.
   - **Program size is mostly three's, not ours.** The terrain fragment
     program is **80,563 chars** of GLSL with `#include`s expanded, of
     which **73,375 (91.1%) is stock `MeshStandardMaterial`** — measured
     from the template three hands over, before the first replace — and
     **7,188 (8.9%)** is everything this repo added to the ground: the
     splat graph, four identities, causal modifiers, both bump octaves
     and the clipmap patch. Within our share, the clipmap shadow GLSL is
     2,503 chars and the field's three sample lines are 617. A grain
     octave adds hundreds of characters to an 80 KB program. If program
     size is what a joining tab cannot afford, the lever is three's
     material, not this repo's shader.
   - **Fill: only one number survived five runs — the whole shadow term
     is 2.9–17.1% of the frame** (28 / 176 / 81 / 86 / 150 ms, right sign
     5/5, though under its own run's floor on the noisiest two).
     Everything finer is below what this box can hold still for. Each run
     is read against the probe's own published resolution — two sweeps of
     the *identical* program — and that floor was 0.3%, 2.7%, 3.8%, 10.5%
     and 51.1% of frame, so it is itself a one-sample estimate that
     swings 170×. Level 0's PCF did **not** converge (+18 / −98 / −42 /
     −584 / −567 ms), and twice it refuted itself: `near1` keeps 3
     fetches where `noshadow` has 0, so the PCF's share cannot exceed the
     whole shadow term's in the same run, and it was 7× and 3.8× it. The
     field did not resolve either — wrong sign in 3 of 5. **So the PCF is
     not a lever worth pulling for grain, and this box cannot be the
     instrument that decides anything finer than "the shadow term is
     under a fifth of the frame."**
   - **The consequence for grain, which is the point:** the ~9% frame
     delta the grain branch rejected itself on (665 ms against 609) sits
     inside the band this box's own noise covers, unpaired with any
     control. That rejection was never a measurement. **Re-run it against
     a floor** — `costProbe` takes variants, so a grain variant belongs
     next to `nofield`, and it will be measured against a second sweep of
     the shipped program in the same run — and decide from the ratio.
     If grain's delta lands where the PCF's did, this box cannot answer
     it and the decision has to move to the counted budget or to a
     quieter machine.
   - Free and already taken: the micro octave is skipped where its own
     footprint fade has already retired it, gated image-identical against
     a variant that samples it unconditionally (0 px differ, max 0/255).

   Rails unchanged: constants into `DECISIONS.md` §open, stay inside the
   `DESIGN.md` §9 budget the browser gate asserts (peak 147/300 calls,
   1.05 M/1.5 M tris) and the fragment budget it now asserts too (18/24
   fetches, 80,563/96,000 chars), no fps quoted from this box.
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
