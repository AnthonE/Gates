# Gates · NOW.md — what next

The only list that answers "what should the loop pick up." Top item first.
Done items are deleted, not checked — history lives in git and
`DECISIONS.md`. A loop iteration starts here, ends with gates green.

1. **M0 exit check** (`DESIGN.md` §11): the shell is fully built — sim-core,
   protocol, server, wasm client core, web client — and `./ci/gates.sh` is
   green, including the client loop gate (bit-exact prediction, loss
   recovery, interpolation). What remains needs a human with a browser:
   run `cargo run -p server --bin shard` + `./web/dev.sh`, open two tabs,
   paste the shard log's cert hash, confirm two capsules walk around each
   other on the island. Then delete this item. Standing follow-up for a
   later slice: a headless-browser e2e so this check joins CI.
2. **M1 — survival verbs** + bags, hotbar, chat (`ALPHA.md` §1/§6):
   gather/slots · inventory/craft from `content/` · build grid + hearth +
   upkeep/decay · death/backpack/respawn-on-bag.
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
