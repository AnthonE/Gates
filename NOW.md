# Gates · NOW.md — what next

The only list that answers "what should the loop pick up." Top item first.
Done items are deleted, not checked — history lives in git and
`DECISIONS.md`. A loop iteration starts here, ends with gates green.

1. **The grain is projected on world XZ, so a slope combs downhill.**
   Grain landed (materials v1, `DECISIONS.md` §open): a fourth octave at a
   per-identity wavelength, ridge-folded and contrasted per identity,
   driving albedo, roughness and bump, retired by pixel footprint in
   cycles-per-pixel against the **world** footprint. The gate measures it
   as CONTRAST rather than moved pixels — 10.01% of the near frame moved,
   0.17 → 1.02 luma/px (×6.12), 0.000% from 60 m up, control noise 0 at
   both views.

   What is left is the projection. A world-XZ field on a face of upness
   `u` is stretched by `1/u` along the slope: at 1.7 m that is a smear
   nobody reads as wrong, at 4 cm it is a hillside combed downhill. The
   fix is to sample **the grain and only the grain** triplanar. It was
   built and measured on `loop/m1-surface-grain` (unmerged, and it
   survives in no tree — rebuilding it means rebuilding it), and the shape
   that worked is: **ridge fold applied per plane BEFORE the blend, and
   the blend's deviation restored by `1/|w|`**. Without both, a 47° face
   measured ×0.56 the contrast of the same face on world XZ. With both:
   slope-to-contour contrast 1.100 → 1.078 on a 47° face at ×1.00 overall
   contrast, and exact identity on level ground (the weights are (0,1,0)
   there). `contrastProbe` takes arbitrary world views, so the gate for it
   is a third view aimed at a measured face plus the existing level-ground
   one as the control.

   **Do not re-run the cost question on this box.** Every run of the gate
   takes it again; the six taken while this slice was built read +14 ms
   (0.3× the floor), −74 (1.7×), −64 (1.3×), −110 (2.2×), −104 (8.2×),
   −62 (0.1×) — five of six the wrong sign, less work measured slower.
   Grain lands exactly where level 0's PCF landed, inside a floor that is
   a one-sample estimate itself and swung 13–603 ms across those same six
   runs. A seventh reading is not a tiebreak.
   The counted budget is the one that answers: 81,520/96,000 chars (the
   octave is 638), 4 noise sample sites/fragment, 18/24 depth fetches.
   Triplanar's ~9% claim from the old branch is inside the same noise, so
   price it counted too — sample sites and program chars — not in ms.

   Also unfinished, and separately gateable: **`gmHash4`**, the four
   lattice corners of a noise sample evaluated in one `vec4` body instead
   of four inlined scalar ones. It is on the old branch, it is image-
   identical by construction, and it has never been gated on its own — the
   pattern to copy is `noskip`: compile a `noh4` variant with materials
   v0's scalar hash and require the same frame.

2. **A tab that boots beside another live tab takes 34 s to reach the
   world. Nobody knows where those seconds go.**
   The third-tab version of this went red on 2026-08-01 16:26 (`inWorld`
   at 61.6 s of a 60 s window) and the recovery pass closed it, but by
   removing the *contention*, not the cost: the gate now closes tab A and
   tab B once their last assertion is made, so the public tab boots on an
   empty box and joins in **0.3 s**, and a structural check refuses to let
   it boot beside a live tab again. `JOIN_TIMEOUT_MS` was not touched.

   What that bought is the reading this item always wanted, at the harness
   level: **join time is monotonic in live tabs — 0.4 s alone, 34–36 s
   beside one, 55–61 s beside two.** The 34 s is the part still standing,
   and it is still the thinnest margin in the suite: tab B needs a live
   tab A (mutual AOI is M0's exit condition), so no amount of harness
   tidying can hand it a quiet box.

   The *client* half is therefore untouched and is the live risk. Grain
   did not cause it — the frame moved 630 → 638 ms, 1.3% — but nothing
   has measured where the seconds go, and every slice that adds a material
   or a program spends more of them. **Do not fix this by widening
   `JOIN_TIMEOUT_MS`.** Measure it first: the tab's own timeline from
   `#connect` to the first publish, split into wasm load, connect,
   handshake, first compile and first chunk. The cost probe already says a
   terrain program costs ~3 s to compile here, and a fresh tab compiles
   more than one.
3. **Nothing casts past 720 m, and nothing out there has a silhouette.**
   The horizon casts now (`DECISIONS.md` §open) but two limits are stated
   rather than solved: the coarsest clipmap level stops at 720 m because
   fog closes at 1000 m, and past the near ring the only caster is the
   8 m ground itself — the scatter stops at the ring's edge, so a forest
   at 400 m casts nothing and the gate measures the horizon on 2 of 4
   yaws for exactly that reason. A scatter LOD (billboard crosses,
   `TERRAIN.md` §4's "trees get two LODs") is the fix and it is a terrain
   job, not a shadow one.
4. **M1 — survival verbs** + bags, hotbar, chat (`ALPHA.md` §1/§6).
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
5. **M2 — combat true**: lag-comp ring + rewound raycasts · ballistic
   projectiles · satchel + damage-by-tier · day/night · netem feel bar.
6. **M3 — economy dark + ops**: OBOL machinery behind the A1 switch ·
   admin lane · backups · status page · error capture · `bench_transport`.
7. **A1 playtest** (operator schedules): 10–20 testers, one wipe cycle —
   then tune content bands from what the anomaly log and the replays say.
8. **M4 — arm A2, then A3** (operator acts): claim rail export · skin
   catalog · the board delivery (repo + playable link + a recorded round
   whose replay hash checks) on `munus-first-sale`.

Standing rule: anything a playtest breaks jumps this queue; anything a
wall catches jumps the playtest.
