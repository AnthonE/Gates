# Jev local bot prototype

To run the first gathering loop without a model key:

```sh
cargo run -p server --release --bin jev-bot -- --local --scripted --gather-wood
```

With `TYPESAFE_API_KEY` set, omit `--scripted` to let Jev explore. The
collect-wood goal is fixed in this slice: a local controller finds a visible
tree, walks within the ordinary swing reach, selects a working rock/hatchet
from its belt and harvests until the tree is gone. It then searches again.
The final `gathering` line reports actual inventory wood and its increase;
a collect-wood run with no confirmed gain exits unsuccessfully.

The controller reuses `ClientCore` for inventory, content catalog, harvested
slots and built collision. A preallocated event ring delivers the reliable
lane in order; overflow or closure ends the session rather than silently
losing a state update. Search checks one nearby terrain cell per frame, within
32 m and a 90-degree view cone, and checks terrain, scenery and built cover
before acquiring a target. These are the human client's procedural scenery
and received deltas. No authoritative `World` is available to the controller.
Neither the seed nor these target coordinates are sent to Jev.

It turns away from deeper water while searching, abandons a target after
three seconds of simulated time without approach/harvest progress, and takes
a one-second sideways route before searching again. One failed target is
remembered so it does not immediately retry the same obstacle. It stops on
stale snapshots, death/wounding/sleep, a full pack or no working belt tool.
Nearby bodies inhibit harvesting. Item names come from the wire catalog;
item indices, yields, stack sizes and condition ceilings are never assumed.

A received hit interrupts gathering. The bot looks toward the ordinary
damage-direction indicator and sprints backwards. A nearby body that remains
visible renews the retreat, as does another hit; it resumes travelling away
after six seconds without either. At most one body sight ray runs per frame,
using the same range, view cone and cover checks as tree perception. A hidden
attacker's position is never queried. The abandoned tree is remembered so the
bot does not immediately return to it.

This is a rough wood collector: no general path planner, inventory rearranging,
stone gathering, crafting, eating, deliberate combat, respawning or reconnecting yet.
Escape is still unreliable on rough terrain; normal-spawn runs can end in
animal deaths and lose the collected inventory.
Trees behind vegetation volumes may be conservatively hidden. A nearby-body
check reduces accidental swings at other players but cannot prevent a body
from moving into a swing between snapshots.

Without `--gather-wood`, the guest explorer walks, turns and jumps using the
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
exponentially, capped at 64 intervals. No automatic respawn or deliberate combat.

The model is pinned to `jev-1.13.0`. Each exploration request contains the bot's own
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
requests whose responses could not be used. An exploration-only run with no
decisions fails; gathering also requires a confirmed inventory gain.

This guest prototype is restricted to loopback and uses the load client's
local certificate policy. Public deployment needs the normal wallet sign-in,
entitlement and certificate validation, plus richer player-visible senses
before it could be useful as a survival player. No server simulation rules,
wire layouts, balance or public shard configuration change.

API contract checked against https://docs.typesafe.ai/api and
https://docs.typesafe.ai/models on 2026-09-21. Live Jev quality, latency and
billing still require a real key; automated tests use a local HTTP peer and
controlled decision sources.
