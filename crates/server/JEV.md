# Jev explorer prototype

One peaceful guest bot on a local shard. It walks, turns and jumps using the
same input datagrams and snapshot decoder as the existing load bots. Jev
chooses a movement once per second; a worker handles HTTP while inputs keep
flowing at the game's cadence. Preallocated single-slot rings carry observations
and decisions; request serialization and error-string destruction stay on the worker. This is the first movement experiment toward
PLAYERS.md, not its wallet-bearing public agent API.

Run from this worktree, with a TypeSafe key in `TYPESAFE_API_KEY`:

```sh
cargo run -p server --release --bin jev-bot -- --local
```

The command boots a temporary loopback shard with shipped content, prints its
address and the exact client command for watching, and runs for 120 seconds.
The bot and viewer share a beach chosen by the game's spawn selector.
There are no saves. Use the matching checkout for the watching client: wire
versions must agree. Ctrl-C closes the bot and its temporary shard.

Without a key, explicitly select the offline movement demonstration:

```sh
cargo run -p server --release --bin jev-bot -- --local --scripted
```

This is labelled SCRIPTED and makes no model calls. It walks and turns right
when a previous walk made no progress. It is never substituted for Jev silently.

To join an existing local guest shard:

```sh
cargo run -p server --release --bin jev-bot -- --server 127.0.0.1:4433 --seconds 300
```

`--think-ms` changes the decision interval; `--timeout-ms` changes both the
HTTP deadline and the maximum age of a snapshot or pending reply. Defaults
are 1000 and 3000 ms. Movement stops at the end of each decision interval
while the next request is pending. Failures stop movement and back off
exponentially, capped at 64 intervals. No automatic respawn or attack.

The model is pinned to `jev-1.13.0`. Each request contains only the bot's own
replicated x/z position, grounded/wounded state, previous action and measured
movement since its previous observation. No world seed, terrain query, other
player's body, chat or credentials enter the observation. Consequently it
cannot see an obstacle ahead of time; it can only react to failed movement.
Turns are quarter/half turns and jumps are single presses. The response is
capped at 16 KiB; unknown actions, model versions and malformed confidence
values are rejected. Confidence is reported, not treated as a correctness
threshold. API errors print a status/reason, never the API key or response body.

At exit the command prints request/decision/error counts, accepted-response
input tokens, moving frames, first/last position in wire quanta and snapshot
statistics. TypeSafe billing remains the authority on paid usage, including
requests whose responses could not be used. A run with no decisions fails.

This guest prototype is restricted to loopback and uses the load client's
local certificate policy. Public deployment needs the normal wallet sign-in,
entitlement and certificate validation, plus richer player-visible senses
before it could be useful as a survival player. No server simulation rules,
wire layouts, balance or public shard configuration change.

API contract checked against https://docs.typesafe.ai/api and
https://docs.typesafe.ai/models on 2026-09-21. Live Jev quality, latency and
billing still require a real key; automated tests use a local HTTP peer and
controlled decision sources.
