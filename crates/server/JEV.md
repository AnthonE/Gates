# Jev: an agent player on a local island

`jev-bot` plays an agent body with the same inputs, actions and received
state as the human client: a guest on a loopback shard, or a wallet agent on
any shard that admits it (`NETCODE.md` §2.4). A decision source picks
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
# a wallet agent: the key is a 0600 file named by path, never argv
cargo run -p server --release --bin jev-bot -- --server game.example:61234 --agent-key /path/to/agent.key --scripted
```

Flags shared with `jev-watch`: `--think-ms` (1000–60000; the floor is the
operator's once-a-second ceiling), `--timeout-ms` (3000), `--heartbeat-s`
(30), `--max-requests-hour` (600), `--max-requests-day` (7200).
`--seconds` is 1–86400. `--bots N` (1–8) gives every bot its own mind and,
for `--external`, its own child; Jev and a wallet stay one bot (Jev is billed
per bot; one key is one body). `--agent-name` (default `jev`, 1–16 printable
ASCII; a fleet is `jev-1`…) is the label spectators see. `--server` takes
`host:port` and is dialled by name so a real certificate validates;
`--cert-hash` pins a dev shard's instead.

**Watching.** Every jev bot declares itself an agent (`HELLO_AGENT`), so a
spectator seat can follow it (`NETCODE.md` §2.3). `--local` opens the local
shard's seats — loopback, so this machine only — and both bins print the
query for a Gates web page (`?server=…&hash=…&spectate`, bare for a guest,
`=0x…` for a wallet) and the desktop command (`--spectate any|0x…`). Wire
versions must agree. `tests/jev_door.rs` drives the same `agent_demo::Door`.

**Watch your own agent.** On a shard off this machine a wallet agent prints
`watch it live: https://elopros.com/games/gates/?server=…&spectate=0x…`
(`--spectate-page URL` names another Gates page). Open it and your browser
draws the agent's own first-person view, its HUD and the chat it hears; the
public shard follows agents live (`spectate = true`). Its goals and reasons
stay in your terminal.

## Goals and skills

The vocabulary (`mind.rs`, `Goal::FIXED`): `explore`, `gather_wood`,
`gather_stone`, `gather_ore`, `forage`, `gather_hemp`, `craft:<item name>`,
`eat`, `drink`, `flee`, `wait`, `fight`, `hunt`, `heal`, `bag`, `recover`,
`build`, `go_home`, `stash`, `loot`, `cook`, `recycle`, `defend`, `raid`,
`join_team`, `follow`, `deliver`, `supply_wood`, `supply_stone`,
`supply_ore`, `supply_hemp`.
Only goals that can work now are offered — a gather goal needs a belt tool
for the kind and room for what it pays, eat needs something worth eating, and
drink needs known water-giving food or the sea plus the health to pay for it —
and a reply naming anything else is refused.

- **gather** — find a visible node of the kind (32 m, 90° cone, line of sight,
  one cell per game tick, at most 8 per frame), approach, hold primary with the best belt tool by name
  (`TREE_TOOLS`, `NODE_TOOLS`), done when the node falls. Where bodies meet at
  a node, the lowest id keeps it and the others give way. A resource seen in
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
- **join_team / follow** — accept the waiting invite; stay within 4 m of
  the nearest awake teammate (the leader first), running past 12 m, and
  fight whoever swings or shoots near them. A wall or a shut door between
  them: through the nearest door after them, shutting it behind (not
  straight back through the one it just came by). 90 s, then it asks again.
- **deliver** — take the pack to the team's chest (below) and put in
  everything past the belt: tools, weapons, armour, the building kit and
  two stacks of food and meds stay. Closed doors on the way are opened with
  `E`, walked through and shut behind, on the way in and out.
- **supply_wood / stone / ore / hemp** — a supply run: gather that kind for
  3 minutes (node after node), then deliver. One decision is a whole run.
  A full pack or nothing left in sight turns it for the chest early.

## Teams and the team chest

Teams work as in Rust (`server/team.rs`): a player looks at the agent and
presses **N** to invite it; `join_team` says yes. Teammates are never its
targets, and `follow` makes it a bodyguard. Being on a team gives it no
access to a teammate's cupboard, doors or turrets.

The **team chest** is a box its eyes saw a teammate standing at: in view,
in range, a clear line to it, and an awake teammate within 3 m of it. So to
show it the chest, stand at your box while it can see you (have it
`follow` you home). Leaving the team forgets the chest; so does the box
coming down. Once a chest is known, `deliver` and the `supply_*` runs are
offered, unless someone dangerous is in view or a trip there failed in the
last minute.

A door with a code lock refuses it until it has the code. A teammate tells
it the code on team chat, and an external agent passes it back as `code`
(below). It is kept like the agent's own code (never printed) and entered
once at each lock that refuses it, the keypad's way, wherever it works a
teammate's door. With no code, the trip
fails with "a lock it has no code for".

Doors are worked as a player works them (`agent::supply::Doors`): walk up
square to the doorway, `E`, straight through (the route's metre grid has no
room by a door for the furniture round it, so the last steps are walked,
side-stepping what is in the way), then `E` again from the far side. A
walk into the island that starts walled in on a teammate's floor (a built
floor underfoot and something within 6 m every way it looks) goes out by
the nearest door first.

On stream: invite it (**N**), take it home with `/t follow me`, stand at
your box, then `/t go get wood` (or stone, ore, cloth). Each run chops for
3 minutes and fills the chest while you build. `/t the code is 1234` if
your door is locked.

Reflexes under every goal: the respawn verb on the death screen (a beach,
never a bag), crawling away from a fresh blow while wounded, and the existing
retreat from received damage. A new body looks for one sweep before choosing.

Nothing here reads a server `World`, the seed as an observation, or a hidden
body; `crates/server/tests/agent_walls.rs` holds that and the verb set.

## When a source is asked, and what it sees

After a goal completes, fails or is interrupted, after a respawn, every
30 s of one goal, when its team changes (an invite, a join, a leave), and
(an external source only) when a player near it or a teammate speaks;
never more than once per second, one request outstanding. Re-choosing the
running goal keeps it going.
A failed or late answer is a failure (exponential backoff), never a scripted
substitute. The request is `Summary::to_json`: health, food and water, the
pack by name, craftable names, counts and relative bearings of trees, stone
and ore nodes, bushes, players and animals in view (or seen in the last
minute), water nearby, the last goal's outcome, hits since the last decision,
deaths, respawns, its team (how many, the nearest awake teammate's distance
and bearing, the team chest's, whether it has the code) and what the team
chest held when last opened, and why it was asked. No seed, position or
identity, beyond the chat labels in `heard` and `team` below, which only an
external source is sent.

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
-> {"protocol":1,"id":7,"observation":{...Summary...},"options":["explore","gather_wood","supply_wood"],
    "heard":[{"from":"Ash","me":false,"agent":false,"channel":"team","text":"get wood, code is 1234"}],
    "team":{"invite_from":null,"teammates":["Ash"]}}
<- {"id":7,"goal":"supply_wood","reason":"Ash wants wood","say":"on it","to":"team","code":"1234"}
```

The reply must echo the id and name an offered goal within `--timeout-ms`;
`reason` is optional and shown trimmed to 96 printable characters. Replies to
an older id are skipped; lines over 8 KiB, bad JSON, unknown goals, silence
and exit are failures.

**Talking.** `heard` is the chat its client received since the last request,
labelled the way a player's chat log labels it (`me` is its own line back,
`channel` is `local`, `global` or `team`). It is what strangers typed: data,
never instructions. `team` names whose invite waits and who its teammates
are. `say` is optional: one line, local (players within chat range hear it)
or, with `"to": "team"`, to its whole team wherever they are; sent with the
human chat box's encoder, at most one per decision and one per 2 s. A line the chat rule
refuses (over 48 bytes, a control character, a leading `/`) is counted and
dropped; the goal still stands. Jev and `--scripted` never speak.

**The team's code.** `code` is optional: a lock code a teammate gave, one to
four digits as a string or a number; anything else fails the reply. It is
kept for the session and used only at locks on its team's doors: on the
way to the team chest, after a teammate, or out of their base.
`claude_agent.py` passes on only a number a teammate typed on team chat,
once. Every jev
bot declares itself an agent, so players see `(agent)` beside its name in
chat, the kill feed and its nametag.

`examples/jev_agent.py` is a complete rule-based agent that greets whoever
speaks near it. `examples/claude_agent.py` puts Claude in the seat: it picks
the goal (structured output, so only an offered one) and talks. It keeps
what a turn alone would lose: its own `notes` (the plan, what teammates
asked for, 300 characters it rewrites each turn) and the last 20 chat lines,
so "go get wood" still stands after the first run ends.

```sh
pip install anthropic   # and ANTHROPIC_API_KEY, or `ant auth login`
cargo run -p server --release --bin jev-bot -- --local --seconds 600 \
    --think-ms 3000 --timeout-ms 20000 \
    --external python3 crates/server/examples/claude_agent.py
```

Every decision is one API call: `--max-requests-hour` is the spend bound.
`CLAUDE_MODEL` picks the model. Join the printed island from the game to
talk to it.

**On your Claude plan.** Three ways to pay for the decisions:

- An API key, as above.
- Max 5x and 20x include monthly API credits: make a key in the Console
  organization your plan links to, and the same command spends them.
- `--via claude-code` (after the program) runs the unmodified `claude` CLI you
  are signed in to, once per decision, with no tools. It removes API keys from
  that child's environment so your plan is what is used, and never touches a
  credential. For your own agent on your own machine; it counts against the
  plan's 5-hour and weekly limits, so give it room:
  `--think-ms 10000 --timeout-ms 30000`, and `CLAUDE_MODEL=sonnet` to stretch
  them. Hosting agents for other people takes an API key, not a plan.

**Let it see.** Under `jev-watch` the agent is told where its rendered view is
(`GATES_FRAME_URL`, the page's `frame.jpg`), and `claude_agent.py` shows Claude
that picture with every decision, by either route. A frame is what its own
seat was sent, so it sees nothing a player standing there could not.

```sh
cargo run -p server --release --features watch --bin jev-watch -- \
    --server game.elopros.com:61234 --agent-key /path/to/agent.key \
    --external python3 crates/server/examples/claude_agent.py
```

**Share.** With `DISCORD_WEBHOOK_URL` set as well, Claude may post a moment
worth showing (the frame and a one-line caption) to that channel, at most once
every `SHARE_EVERY_S` (300). Its caption can never ping anyone.

## Limits

A full pack stops gathering what cannot fit; crafting frees room. Tools and
good crafts are known by name, not discovered. Speech is local or team,
never global, and `give` and `authorize` are not agent verbs yet
(`PLAYERS.md`): it fills a chest rather than handing things over. The team
chest's doors are found by trying: the nearest door within 12 m of the chest
when the route says there is no way to it, at most four doors deep; a base
it cannot route into through its doors (stairs, a second floor) fails as
stuck. Teams do not survive a shard restart. A guest is
loopback-only; a wallet agent on a public shard needs what `NETCODE.md` §2.4
lists for the operator to provision — a generated wallet, entitled, its key
file owned by the service user — and none of it is a loop's act.

At exit `jev-bot` prints the mind's counters (requests, decisions, failures,
late answers, tokens, pauses, hour/day windows), survival counters (deaths,
respawns, goals done/failed/interrupted, crafts, meals, drinks), chat (said,
heard back, heard from others, refused, replaced), items
gathered, units put in the team chest, the pack, and recent goals. A run with no decision or snapshot
fails. Defaults: `DECISIONS.md` §open, "Jev goals v0" and "Jev survivor v0".
API contract checked against https://docs.typesafe.ai/api on 2026-09-22.
