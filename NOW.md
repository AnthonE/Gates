# Gates · NOW.md — what next

The only list that answers "what should the loop pick up." Top item first.
Done items are deleted, not checked — history lives in git and
`DECISIONS.md`. A loop iteration starts here, ends with gates green.

1. **A kill pays, but a base still defends nothing.**
   *(From the merge-gate judge's ranked gap 1 in
   `findings/archive-prestamp/pass-20260802-025624-01-judge.md`, and its
   predecessor's, both rounds.)*

   Melee v0 landed (`sim-core/combat.rs`, wire v11), and the death
   backpack landed on top of it (`sim-core/backpack.rs`, wire v12): what
   you carried stays where you fell, the bag despawns on
   `content/balance.toml`'s rarity ladder, and the nearest bag in reach
   opens with E. Both `DECISIONS.md` §open rows hold every bound and every
   deliberate omission.

   What the gap still wants, in the order it is worth doing:
   - **Piece damage** — the raid lane, and now the whole of it. A locked
     door still cannot be breached by force, so the raid bands in
     `CONTENT.md` remain data nothing plays. `weapons.toml` carries no
     melee-vs-structure column and inventing one would move the raid
     ratio `test_content` asserts, so it needs a content column and a
     re-derived anchor, not a code constant.
   - **A container UI, and per-slot looting.** The take is all-that-fits
     today, which is honest but blunt: nobody can see what is in a bag
     before opening it, or leave the stone and take the gunpowder. The
     inventory screen in `inventory.jpeg` is the shape; a bag panel beside
     it is the slice. `EventMsg::BagRemoved` already carries *why* a bag
     went, so "someone got there first" has a feed line waiting for it.
   - **Armor and headshots.** `armor.toml` bakes into nothing; aim is
     planar, so there is no head. Both wait on M2's rewound raycasts.
   - **Ground drops for a full inventory.** `gather::inv_add` still loses
     the overflow — now that a ground container exists, that loss has
     somewhere honest to go.

   Two counters worth watching before the next wire slice: the event
   subtype field is **30 of 32** used, and the action subtype field is
   **9 of 16** (widened 3 → 4 bits by the loot action). The next S→C fact
   past two more costs a `SUB_BITS` bump, which moves every event message.

2. **`gmHash4` — four lattice corners in one `vec4` body, never gated.**
   The projection half of this item landed (materials v1 third pass,
   `DECISIONS.md` §open): the grain — and only the grain — is sampled
   triplanar, ridge-folded per plane before the blend and the blend's
   deviation restored by `1/|w|`. Measured on the 46.6° face this spawn
   offers: tilting the ground coarsens the world-XZ grain ×2.017 and the
   shipped one ×1.397, a gain of ×1.444 against a ×1.456 stretch, at
   ×1.044 amplitude. The gate for it (`browser_smoke` 15c) is a within-run
   comparison against a compiled `flatgrain` partner at two square-on
   cameras, and it goes red — ×1.000 — the moment the tap stops reading
   the normal.

   What is left of the old branch is `gmHash4`: the four lattice corners
   of a noise sample evaluated in one `vec4` body instead of four inlined
   scalar ones. It is image-identical by construction, and it has never
   been gated on its own — the pattern to copy is `noskip`: compile a
   `noh4` variant with materials v0's scalar hash and require the same
   frame. It matters more after the projection than before it, because
   grain now takes three noise samples where it took one, so the ground
   pays 6 sample sites per fragment against 4 — and a sample site is four
   hash evaluations.

   **Do not re-run the cost question on this box.** Every run of the gate
   takes it again; the six taken while grain was built read +14 ms
   (0.3× the floor), −74 (1.7×), −64 (1.3×), −110 (2.2×), −104 (8.2×),
   −62 (0.1×) — five of six the wrong sign, less work measured slower.
   Grain lands exactly where level 0's PCF landed, inside a floor that is
   a one-sample estimate itself and swung 13–603 ms across those same six
   runs. A seventh reading is not a tiebreak.
   The counted budget is the one that answers, and it now has three axes,
   all asserted: 81,820/96,000 program chars, 6/8 noise sample sites per
   fragment, 18/24 depth fetches. Price `gmHash4` there too — hash
   evaluations and program chars — not in ms.

3. **A tab that boots beside another live tab takes 34 s to reach the
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

4. **Nothing casts past 720 m, and nothing out there has a silhouette.**
   The horizon casts now (`DECISIONS.md` §open) but two limits are stated
   rather than solved: the coarsest clipmap level stops at 720 m because
   fog closes at 1000 m, and past the near ring the only caster is the
   8 m ground itself — the scatter stops at the ring's edge, so a forest
   at 400 m casts nothing and the gate measures the horizon on 2 of 4
   yaws for exactly that reason. A scatter LOD (billboard crosses,
   `TERRAIN.md` §4's "trees get two LODs") is the fix and it is a terrain
   job, not a shadow one.
5. **A capture the same twice is a gate; a capture that drifts is a vibe.**
   Deterministic capture mode (operator, 2026-08-02, `DECISIONS.md` — the
   Claude-of-Duty adoption row): the client animates off the sim tick / an
   injected fixed-step clock in capture mode — today the RAF loop steps off
   `performance.now()` (`main.js`), so boot-time noise shifts every pixel —
   with a fixed seed, the existing `__gatesDebug.setView` shots, and ONE
   fresh page per shot (state leaks between shots on a shared page:
   exposure-like accumulators, particle age). Then a pixel-diff tool
   against blessed per-box goldens, exit nonzero on any moved pixel, used
   two ways: refactor/optimization passes assert zero diff; feature passes
   regenerate goldens in the same commit — `test_protocol_golden`'s
   discipline, wall 6, applied to frames. v1 scope: solo shard, camera-only
   shots, no remotes in frame. v2, if wanted later: render-from-WAL-replay
   (wall 5 already guarantees the state side). The clock conversion is the
   prerequisite, not the diff tool — grep every `performance.now()` and
   RAF-timestamp use in `web/src` and sort each into sim-driven, cosmetic
   (must switch to the engine clock in capture mode), or UI-only (excluded
   from shots). Settle by tick count, never by time.

6. **M1 — survival verbs** + bags, hotbar, chat (`ALPHA.md` §1/§6).
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
7. **M2 — combat true**: lag-comp ring + rewound raycasts · ballistic
   projectiles · satchel + damage-by-tier · day/night · netem feel bar.
8. **M3 — economy dark + ops**: OBOL machinery behind the A1 switch ·
   admin lane · backups · status page · error capture · `bench_transport`.
9. **A1 playtest** (operator schedules): 10–20 testers, one wipe cycle —
   then tune content bands from what the anomaly log and the replays say.
10. **M4 — arm A2, then A3** (operator acts): claim rail export · skin
   catalog · the board delivery (repo + playable link + a recorded round
   whose replay hash checks) on `munus-first-sale`.

11. **`cargo test --workspace` overflows a debug thread's stack; only
    `--release` (what CI runs) is green.** Pre-existing, not new: verified
    on `main` at `25f6ec8` before the backpack slice, where
    `snapshot_budget` aborts the same way. The cause is size, not logic —
    `World` is ~416 kB of fixed capacity and `ShardCore::new` builds it on
    the stack, so an unoptimized frame holds two or three copies against a
    2 MB limit. It bites anyone who types the obvious command. The fix is
    the one this slice already used for its own store: box the big
    fixed-capacity members (`Pieces`, `Deploys`, `SlotLives`) at
    construction, the way `ShardCore` already boxes its client array —
    one allocation at boot, none in the tick.

Standing rule: anything a playtest breaks jumps this queue; anything a
wall catches jumps the playtest.
