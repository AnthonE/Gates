# Gates · NOW.md — what next

The only list that answers "what should the loop pick up." Top item first.
Done items are deleted, not checked — history lives in git and
`DECISIONS.md`. A loop iteration starts here, ends with gates green.

1. **`dev_view` — the camera hook the visual harness needs** (one small
   slice, then straight back to M1). `globalThis.__gatesDebug.setView(yaw,
   pitch)` sets `InputTracker`'s yaw/pitch directly so an automated capture can
   aim. Put it on the **existing 250 ms HUD timer, never the RAF path**
   (`DESIGN.md` L8) and **dev-gate it exactly like `dev_spawn`** — it must not
   exist on a public shard. Register the knob in `DECISIONS.md` §open.
   **Why:** headless Chromium grants pointer lock but yields no `movementX`
   deltas, so a capture can walk but cannot look — measured, not assumed
   (`gates-loop/art/probe-pointerlock.mjs`). Without this the visual judge
   cannot get comparable fixed vantages, and comparability across passes is the
   whole point. Optional extra, not required for v0: `setPos(x, y, z)` for a
   free camera, so a base can be framed from outside without walking to it.
2. **M1 — survival verbs** + bags, hotbar, chat (`ALPHA.md` §1/§6).
   Gather, craft, build, and deployables are sim'd, on the wire, and
   solid (slice 9: doors — a door places closed and seals its doorway,
   the use action toggles it, wire v6 carries the action, the
   announcement and the open bit on every deploy record; E is the
   client's use key and your own door swings on the press, NETCODE §6.1).
   Next: door locks (any hand in reach toggles today) · upgrade-in-place
   · chat · death/backpack/respawn-on-bag (bags place + cap now; the
   anchor lands there).
3. **M2 — combat true**: lag-comp ring + rewound raycasts · ballistic
   projectiles · satchel + damage-by-tier · day/night · netem feel bar.
4. **M3 — economy dark + ops**: OBOL machinery behind the A1 switch ·
   admin lane · backups · status page · error capture · `bench_transport`.
5. **A1 playtest** (operator schedules): 10–20 testers, one wipe cycle —
   then tune content bands from what the anomaly log and the replays say.
6. **M4 — arm A2, then A3** (operator acts): claim rail export · skin
   catalog · the board delivery (repo + playable link + a recorded round
   whose replay hash checks) on `munus-first-sale`.

Standing rule: anything a playtest breaks jumps this queue; anything a
wall catches jumps the playtest.
