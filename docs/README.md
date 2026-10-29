# docs/

The design docs. None of them is required reading before you start: `CLAUDE.md`
at the root is the whole process. Grep these when you need an answer.

Code comments cite them by bare name (`NOW.md §0rep`, `DECISIONS.md` 2026-09-23);
that means the file here.

## Working docs

| doc | owns |
|---|---|
| [`NOW.md`](NOW.md) | the backlog: what to pick up, and what waits on the operator |
| [`DECISIONS.md`](DECISIONS.md) | the operator's spoken calls since 2026-09-08, and the open knobs |
| [`ARC.md`](ARC.md) | the wipe arc and its frameworks. Read it before touching works, unlocks, speakers, inscriptions or puzzles |
| [`DESIGN.md`](DESIGN.md) | the design of record: the loop, the verbs, the product |
| [`NETCODE.md`](NETCODE.md) | multiplayer: prediction, reconciliation, the wire |
| [`TERRAIN.md`](TERRAIN.md) | worldgen: the island as a pure function of the seed |
| [`RENDER.md`](RENDER.md) | how the Bevy client draws |
| [`ART.md`](ART.md) | the visual bar. Outranks `RENDER.md` |
| [`CONTENT.md`](CONTENT.md) | the schemas behind `content/*.toml` |
| [`PLAYERS.md`](PLAYERS.md) | the agent player (`jev-bot`): a player's inputs and a player's view, nothing more |
| [`WORLD.md`](WORLD.md) | the fiction. §11 is a secret shared with Before Colony and never said in game |
| [`BUSINESS.md`](BUSINESS.md) | what we sell. Read it only when building the store, the entry price or the bank |
| [`STEAM.md`](STEAM.md) | Gates 2, the Steam edition: no economy, cross-play on plain islands, solo and LAN, the funnel to the store |

## Snapshots and history

| doc | what |
|---|---|
| [`HAVE.md`](HAVE.md) | what a player can do today, drawn off the tree with commands (re-run them; counts drift) |
| [`MENUS.md`](MENUS.md) | an audit of our interaction surface against the reference game (2026-08-04) |
| [`ALPHA.md`](ALPHA.md) | the gap sweep that scoped the first alpha |
| [`DECISIONS-ARCHIVE.md`](DECISIONS-ARCHIVE.md) | the spoken log from 2026-07-31 to 2026-09-05, frozen. Older dated citations land here |

## Elsewhere in the tree

| where | what |
|---|---|
| [`../reference/`](../reference/README.md) | how Rust the game does a system, sourced. Nothing in it ships |
| [`../findings/`](../findings/README.md) | measurements and design notes from before MVP mode. Closed to new notes |
| [`../posts/`](../posts/) | patch notes and devblog posts, one per release |
| `../crates/server/JEV.md` | the `jev-bot` protocol, and how to host an agent-only island |
| `../assets/*/MANIFEST.md`, [`../NOTICE`](../NOTICE) | an asset's source and licence: its folder's `MANIFEST.md` (the icons' `CREDITS.md`) or `NOTICE` |
| [`../ops/`](../ops/README.md) | the public shard's systemd units and the scripts they run |
