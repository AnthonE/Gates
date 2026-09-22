# Jev: an agent player on a local island

`jev-bot` plays one guest body on a loopback shard with the same inputs,
actions and received state as the human client. A decision source picks
**goals**; local skills carry them out. It survives in-game: death is answered
on the death screen and play goes on, so the process ends only when the run
ends or the connection fails.

```sh
# explicit scripted goals, no model calls
cargo run -p server --release --bin jev-bot -- --local --scripted --seconds 600
# Jev (needs TYPESAFE_API_KEY); omit --scripted
cargo run -p server --release --bin jev-bot -- --local --seconds 600
# bring your own agent (must be the last flag)
cargo run -p server --release --bin jev-bot -- --local --external python3 crates/server/examples/jev_agent.py
# up to eight scripted or external agents meeting on one island
cargo run -p server --release --bin jev-bot -- --local --scripted --bots 4
# join an existing loopback guest shard instead of booting one
cargo run -p server --release --bin jev-bot -- --server 127.0.0.1:4433 --scripted
```

Flags shared with `jev-watch`: `--think-ms` (1000–60000; the floor is the
operator's once-a-second ceiling), `--timeout-ms` (3000), `--heartbeat-s`
(30), `--max-requests-hour` (600), `--max-requests-day` (7200).
`--seconds` is 1–86400. `--bots N` (1–8) gives every bot its own mind and,
for `--external`, its own child; Jev stays one bot because it is billed per
bot. `--local` prints the matching client command for watching; wire
versions must agree.

## Goals and skills

The vocabulary (`mind.rs`): `explore`, `gather_wood`, `gather_stone`,
`gather_ore`, `forage`, `craft:<item name>`, `eat`, `drink`, `flee`, `wait`.
Only goals that can work now are offered, and a reply naming anything else is
refused.

- **gather** — find a visible node of the kind (32 m, 90° cone, line of sight,
  one cell per frame), approach, hold primary with the best belt tool by name
  (`TREE_TOOLS`, `NODE_TOOLS`), done when the node falls. A resource seen in
  the last minute is walked back to. Fails on no tool, nothing found in 20 s,
  three stalled approaches, or a full pack (no room for what the node pays).
- **craft** — resolve the recipe by the output's catalog name (no station,
  blueprint known), check inputs, send `Craft`, wait for the completion, then
  `Move` a crafted tool onto the belt.
- **eat / drink** — food is learned from the sim's verdicts, as a player learns
  by pressing eat: refused items are remembered, consumed ones classified by
  the meters that rose. Drink uses known water-giving food first, then the sea
  (salt: it costs health, so it stops at 20% health). Both stop at 80%.
- **explore / wait / flee** — wander 20 s, stand 5 s, back away from the
  nearest visible body.

Reflexes under every goal: the respawn verb on the death screen (a beach,
never a bag), crawling away from a fresh blow while wounded, and the existing
retreat from received damage. A new body looks for one sweep before choosing.

Nothing here reads a server `World`, the seed as an observation, or a hidden
body; `crates/server/tests/agent_walls.rs` holds that and the verb set.

## When a source is asked, and what it sees

After a goal completes, fails or is interrupted, after a respawn, and every
30 s of one goal; never more than once per second, one request outstanding.
A failed or late answer is a failure (exponential backoff), never a scripted
substitute. The request is `Summary::to_json`: health, food and water, the
pack by name, craftable names, counts and relative bearings of trees, stone
and ore nodes, bushes, players and animals in view (or seen in the last
minute), water nearby, the last goal's outcome, hits since the last decision,
deaths, respawns and why it was asked. No seed, position or identity.

**Jev** (`jev-1.13.0`, pinned) is a classifier: it returns a choice,
a confidence and per-option probabilities, and no text. The one-line reason
shown for a Jev decision is composed from those numbers and our own labels.
Responses are capped at 16 KiB; API errors print a status, never the key or
body. Input and output tokens are counted.

**Spend guard.** Requests (sent, answered or not) are counted in an hour window
and a day window from their first request. At either ceiling the mind stops
asking and says `paused: spend cap`; the current goal finishes and the body
then holds still. Nothing is substituted.

## Bring your own agent

`--external PROGRAM [ARG...]` starts a child and speaks newline-delimited JSON
over its stdin/stdout: nothing else can connect to those pipes, there is no
port to guard, the agent dies with the bot, and the bot's stdout stays free.
`TYPESAFE_API_KEY` is removed from the child's environment; the rest is inherited.

```text
-> {"protocol":1,"id":7,"observation":{...Summary...},"options":["explore","gather_wood","craft:Stone Hatchet"]}
<- {"id":7,"goal":"gather_wood","reason":"need wood for a hatchet"}
```

The reply must echo the id and name an offered goal within `--timeout-ms`;
`reason` is optional and shown trimmed to 96 printable characters. Replies to
an older id are skipped; lines over 8 KiB, bad JSON, unknown goals, silence
and exit are failures. `examples/jev_agent.py` is a complete agent.

## Limits

No cooking (it needs a kill, a placed fire and oven moves), no looting its own
death backpack, no building, research, deliberate combat, bags or path
planner. A full pack stops gathering what cannot fit; crafting frees room.
Tools and good crafts are known by name, not discovered. The guest is
loopback-only with the load client's certificate policy; public play needs
the normal wallet sign-in and entitlement.

At exit `jev-bot` prints the mind's counters (requests, decisions, failures,
late answers, tokens, pauses, hour/day windows), survival counters (deaths,
respawns, goals done/failed/interrupted, crafts, meals, drinks), items
gathered, the pack, and recent goals. A run with no decision or snapshot
fails. Defaults: `DECISIONS.md` §open, "Jev goals v0" and "Jev survivor v0".
API contract checked against https://docs.typesafe.ai/api on 2026-09-22.
