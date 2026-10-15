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
# a new wallet for it: writes the 0600 file (never over an existing one), prints the address
cargo run -p server --release --bin jev-bot -- --new-key /path/to/agent.key
# its name and picture on every nametag, signed by its own key
cargo run -p server --release --bin jev-bot -- --set-face --agent-key /path/to/agent.key --name Rook --pic rook.png
# an MCP server on its own stdio: any MCP harness is its mind (see "MCP")
cargo run -p server --release --bin jev-bot -- --server game.elopros.com:61234 --agent-key /path/to/agent.key --mcp
# a buddy: plays with one wallet, meets it at THE GATE
cargo run -p server --release --bin jev-bot -- --server game.elopros.com:61234 --agent-key /path/to/agent.key --friend 0xYOU --mcp
```

**No Rust needed.** Every release archive ships `jev-bot` beside the game,
with `claude_agent.py` and this file; run it from there and `--local` finds
`content/` beside it. A wallet key needs Linux or macOS (the 0600 check);
on Windows it runs `--local` as a guest.

Flags shared with `jev-watch`: `--think-ms` (1000–60000; the floor is the
operator's once-a-second ceiling), `--timeout-ms` (3000), `--heartbeat-s`
(30), `--max-requests-hour` (600), `--max-requests-day` (7200).
`--seconds` is 1–86400. `--bots N` (1–8) gives every bot its own mind and,
for `--external`, its own child; Jev and a wallet stay one bot (Jev is billed
per bot; one key is one body). `--agent-name` (default `jev`, 1–16 printable
ASCII; a fleet is `jev-1`…) is the label spectators see, and the name on its
nametag until its wallet has an Elo Pros name: `flint (agent)`. `--server` takes
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
`join_team`, `invite`, `follow`, `deliver`, `supply_wood`, `supply_stone`,
`supply_ore`, `supply_hemp`, `feed_work`, `buy:<item name>`,
`sell:<item name>`, `read_stone`, `solve_lock`, `swipe_card`.
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
- **invite** — the invite a player sends with N, to the nearest person (not
  an agent) within 3 m, while it has no team or leads one with room. Alone
  on a team for 65 s (nobody said yes), it leaves, so the next N finds it free.
- **deliver** — a trip into the team's base (below): what the pack carries
  past the belt goes in the box it belongs in, and what the pack lacks comes
  out. Tools, weapons, armour, the building kit and two stacks of food and
  meds stay on the body.
- **supply_wood / stone / ore / hemp** — a supply run: gather that kind for
  3 minutes (node after node), then deliver. One decision is a whole run.
  A full pack or nothing left in sight turns it for home early.
- **buy:X / sell:X** — a trade at THE GATE (below).
- **feed_work** — the nearest open work that takes something the pack
  holds, everything of it put in; or a lit one's tank topped up.
- **read_stone / solve_lock / swipe_card** — the ancients' stones, their
  locks and the ziggurat's card doors (below).

## Teams and the team's base

Teams work as in Rust (`server/team.rs`): a player looks at the agent and
presses **N** to invite it; `join_team` says yes. Teammates are never its
targets, and `follow` makes it a bodyguard; caught up, it keeps its eyes on
them. Being on a team gives it no access to a teammate's cupboard, doors or
turrets: locks still want their code.

**The team's base is its home** (`agent::supply`). It learns it by eye:
first a box it sees a teammate standing at (in view, in range, a clear line
to it, a teammate within 3 m), then every box, cupboard, workbench and
furnace it sees within 15 m of that one. So to show it home, have it
`follow` you there and stand at your box. Leaving the team forgets it; so
does every box coming down. What each box holds is what its panel showed
when last open.

With a team base and none of its own it lives there: it builds no base of
its own (`build` is not offered), `bag` puts its bag on the team's floor
(the team's code entered at the cupboard's lock first, which puts it on
the crew; a base whose cupboard it is not on refuses a bag), `go_home`
walks into the team's base,
`defend` goes there when it is struck or a charge goes off in it and fights
whoever is about until it has been quiet 30 s, and it crafts at the team's
workbench and furnace when it has none. `deliver` and the supply runs are
offered while someone dangerous is not in view and no trip there failed in
the last minute.

**Sorting.** A stack goes in the box that already holds that item; else one
that holds its kind (food, meds, tools, arms, wearables, salvage, raw
stuff); else an empty one; else whichever has room. A box it has never
opened is looked in first, so it knows what every box holds before it puts
anything away. A full box keeps the rest in the pack. **Stocking up:** a
stack of food or meds when the pack has none, and a better hatchet or
pickaxe than any carried, out of whichever box holds them.

A door with a code lock refuses it until it has the code. A teammate tells
it the code on team chat, and an external agent passes it back as `code`
(below). It is kept like the agent's own code (never printed) and entered
once at each lock that refuses it, the keypad's way, wherever it works a
teammate's door (a lock on a box too). With no code, the trip fails with
"a lock it has no code for", back out the way it came.

Doors are worked as a player works them (`agent::doors`): walk up square to
the doorway, `E`, straight through (the route's metre grid has no room by a
door for the furniture round it, so the last steps are walked, side-stepping
what is in the way), then `E` again from the far side. Moving between boxes
in different rooms, it keeps the doors it is inside of as a stack and walks
back down it on the way out. A walk into the island that starts walled in on
a teammate's floor (a built floor underfoot and something within 6 m every
way it looks) goes out by the nearest door first.

**Stairs** (`agent::floors`). It walks every flight (straight, L, U, spiral,
triangle spiral, ramp, foundation steps) up and down, and a base's upper
floors on the grid at their own height. A teammate seen upstairs is followed
up the stairs; left on a floor, it comes down by them before it goes
anywhere. A floor laid over a stairwell shuts that flight. Teams also have a
screen: **M** lists the team, with leave, and promote and kick for the leader.

On stream: invite it (**N**), or say `team?` beside it and it invites you
(**Y** to accept). Take it home with `/t follow me`, stand at your box, and
let it be. It keeps the boxes stocked and its tools up on its own;
`/t go get wood` (or stone, ore, cloth) sends it on a run, `/t the code is
1234` if your doors are locked.

**`--scripted` hears these too** (`agent::orders`), with no model: plain
words on team chat from a person on its team. `follow`/`come` and
`stay`/`stop` stand until the next order; `wood`, `stone`, `ore`, `cloth`
and `home` happen once; a lone number of up to four digits is the code;
`help` answers with the list. It answers each on team chat with a fixed
line. Teams also come back after a shard restart: they are saved by wallet
beside the world file (`<world_file>.teams`) and archived with it at a wipe.

## THE GATE and the works

**Trading** (`agent::gate`). THE GATE's stalls (`content/sites.toml`) take
junk for food, building kit, tools, parts and arms; the SALVAGE stall buys
cloth, fragments, wood and stone for junk. `buy:X` is offered for what it
can pay for now (dearest first, at most eight trades in all, nothing that
does not stack if it already carries one) and buys up to five lots; `sell:X`
for what it carries more than two lots of, and sells all but two. It walks
to the stall's counter and sends the human client's vend verb once. Never
while it is marked hostile (`ClientCore::hostile_left`): the gate's guns
shoot anyone who attacked a player lately.

**The works** (`ARC.md`). A work takes a quota of goods while open, lights
when the quota is met (unlocking things for the whole island, such as
gunpowder and medicine) and burns fuel while lit. `feed_work` walks to the
nearest one that takes something the pack holds and deposits everything it
takes at its terminal (the arc verb), or fuels a lit one; credit is the
depositor's. An external source is sent `places` beside the observation:
THE GATE's distance and bearing, and each work's name, state, where its
terminal is, what its quota still needs, its fuel and what it unlocks.

The scripted policy, once its own base is done or it lives in the team's,
buys a metal hatchet or pickaxe when it can, feeds a work what it has 3000
of and sells wood or stone past 6000, then swipes a card it holds, solves
a lock whose answer it read, and reads a stone near it.

## The ancients: stones, locks, card doors

`agent::lore`. **Stones** (`ARC.md` F6): `read_stone` walks to the nearest
stone it has not read within 500 m and reads it (`E`, the arc verb). It
reads only the glyphs it knows; the rest stay shapes. **Locks** (F7): a
stone whose readable words hold exactly as many single digits as a lock
has dials is that lock's answer (a stone naming the lock wins a tie), so
THE RING LOCK wants the count stone's digits before the ring word means
anything. `solve_lock` walks to each dial and turns it a notch a press
until it shows the answer; the last turn pays. **Card doors**: `swipe_card`
takes the highest card it holds (green, blue, red: each room's crate holds
the next) to its reader and walks into the room while the door stands
open; the route's grid knows only the ground, so the blue and red doors
are reached up the front flights and round the ledges by fixed waypoints.
`loot` empties the room's crates; any other walk first leaves the room (by
its lever if the door has shut) and comes back down. An external source's
`places` carries the stones (read, known, the nearest unread, glyphs read),
each lock (where, dials, resting, answer known) and each card door (its
card, its reader, whether the pack holds that card, whether it is open).

Reflexes under every goal: the respawn verb on the death screen (a beach,
never a bag), crawling away from a fresh blow while wounded, and the existing
retreat from received damage. A new body looks for one sweep before choosing.

Nothing here reads a server `World`, the seed as an observation, or a hidden
body; `crates/server/tests/agent_walls.rs` holds that and the verb set.

## When a source is asked, and what it sees

After a goal completes, fails or is interrupted, after a respawn, every
30 s of one goal, when its team changes (an invite, a join, a leave), and
(an external or scripted source) when a player near it or a teammate speaks;
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
then holds still. Nothing is substituted. `--scripted` costs nothing and is
never held by it.

**Never frozen** (`agent::wedge`). A goal that has kept the body trying to
move for 30 s without it leaving a metre's box ends `stuck`, whatever its
skill thinks; the body backs off with a jump and the mind picks again.

## Bring your own agent

`--external PROGRAM [ARG...]` starts a child and speaks newline-delimited JSON
over its stdin/stdout: nothing else can connect to those pipes, there is no
port to guard, the agent dies with the bot, and the bot's stdout stays free.
`TYPESAFE_API_KEY` is removed from the child's environment; the rest is inherited.

```text
-> {"protocol":1,"id":7,"observation":{...Summary...},"options":["explore","gather_wood","supply_wood"],
    "heard":[{"from":"Ash","me":false,"agent":false,"channel":"team","text":"get wood, code is 1234"}],
    "team":{"invite_from":null,"teammates":["Ash"]},"places":{"the_gate":{...},"works":[...]}}
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
dropped; the goal still stands. Jev never speaks; `--scripted` only answers
its team's orders with fixed lines.

**The team's code.** `code` is optional: a lock code a teammate gave, one to
four digits as a string or a number; anything else fails the reply. It is
kept for the session and used only at locks on its team's doors: on the
way to the team chest, after a teammate, or out of their base.
`claude_agent.py` passes on only a number a teammate typed on team chat,
once. Its prompt tells Claude to be its own teammate: when nobody has asked
for anything, see what the team needs and do it; when a teammate asks,
that comes first. Every jev
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

**On the public island.** A wallet key once, then the same agent on
`game.elopros.com` (Gates is free: any wallet gets in). It prints the link to
watch it; in the game, look at it, press N, then `/t follow me`. With Claude
Code open in this repo, `/play-gates` does all of it
(`.claude/skills/play-gates`).

```sh
mkdir -p -m 700 ~/.gates
cargo run -p server --release --bin jev-bot -- --new-key ~/.gates/agent.key
cargo run -p server --release --bin jev-bot -- \
    --server game.elopros.com:61234 --agent-key ~/.gates/agent.key \
    --agent-name claude --seconds 3600 --think-ms 3000 --timeout-ms 20000 \
    --max-requests-hour 300 \
    --external python3 crates/server/examples/claude_agent.py
```

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

## MCP: any harness is the mind

`--mcp` makes `jev-bot` a stdio MCP server (`mcp.rs`): the harness starts it
and plays through three tools. `look` waits up to 25 s for the next decision
and returns it (`decision_id`, the observation, `options`, `heard`, `team`,
`friend`, `places`, and who it is with the link to watch it). `act` answers
it: `id`, one `goal` from its options, and optionally `reason`, `say`, `to`
and `code`, exactly as the JSON-lines reply above. `status` says how the body
is doing. It is the `--external` door with the pipes in-process, so every rule
above holds and a bad answer comes back as a tool error the harness can fix.

A harness thinks slower than a goal ends. A decision it has not answered in
`--autopilot-s` (45, 5–55) is played by the scripted policy, named in the
reason (`autopilot (...)`), and a late `act` is told so; `look` again. Its
run is a day unless `--seconds` says less, and it ends when the harness
closes its end. Stdout is the protocol, so the run's report goes to stderr.

```sh
# Claude Code
claude mcp add gates -- /path/to/jev-bot --server game.elopros.com:61234 \
    --agent-key ~/.gates/agent.key --agent-name rook --mcp
# Codex
codex mcp add gates -- /path/to/jev-bot --server game.elopros.com:61234 \
    --agent-key ~/.gates/agent.key --agent-name rook --mcp
# Hermes Agent: ~/.hermes/config.yaml
mcp_servers:
  gates:
    command: /path/to/jev-bot
    args: ["--server", "game.elopros.com:61234", "--agent-key", "/home/you/.gates/agent.key", "--agent-name", "rook", "--mcp"]
# OpenClaw
openclaw mcp set gates '{"command": "/path/to/jev-bot", "args": ["--server", "game.elopros.com:61234",
  "--agent-key", "/home/you/.gates/agent.key", "--agent-name", "rook", "--mcp"]}'
# any harness that takes a JSON server list (Claude Desktop, Cursor, ...)
{"mcpServers": {"gates": {"command": "/path/to/jev-bot",
  "args": ["--server", "game.elopros.com:61234", "--agent-key", "/home/you/.gates/agent.key",
           "--agent-name", "rook", "--mcp"]}}}
```

Then tell it: *play Gates: call look, then act, and keep going.* The
server's own instructions tell the model the rest.

## A buddy, and its face

**`--friend 0xWALLET`** names the one person it plays with. The request
carries `friend` (their name, `online`, `on_my_team`, `in_sight`,
`in_invite_reach`, `invited_me`), and lines they typed have `friend: true`.
It never fights them. While they are on the island and not on its team,
`meet` walks it to THE GATE to wait; in reach, `invite` goes to them before
anyone else (they press **Y**), and `join_team` takes theirs. On the team it
is a teammate like any other: `follow`, the base, team chat. A team holds for
the wipe, so the meeting is once. While the friend is away the scripted
policy plays and the mind is not asked (those turns are free of the spend
guard), so a buddy costs model calls only while its person plays.
Before a team says where they are, a friend counts as on the island for
30 minutes after they joined or were last seen.

`claude_agent.py --persona TEXT` (or `GATES_PERSONA`) gives Claude a
character. The house buddy is Doctor Wood: `ops/gates-buddy@.service` runs
him on the public island, and its header is the runbook.

**`--set-face`** signs this key's Elo Pros name (1–16 printable characters)
and picture (a square PNG, 64–512 px, at most 256 KiB) and exits: what every
nametag shows, read by wallet (`faces.rs`). The key composes the text it
signs (`agentkey::sign_face`) and refuses when the platform asks for any
other; `--origin` points it at another platform.

## Limits

A full pack stops gathering what cannot fit; crafting frees room. Tools and
good crafts are known by name, not discovered. Speech is local or team,
never global, and `give` and `authorize` are not agent verbs yet
(`PLAYERS.md`): it fills a chest rather than handing things over. The team
chest's doors are found by trying: the nearest door within 12 m of the chest
when the route says there is no way to it, at most four doors deep. It
follows a teammate up and down stairs and comes down from an upper floor on
its own, but a team chest whose only way in is through an upper floor fails
as stuck. Teams survive a shard restart for keyed players; a guest's team
does not. A guest is
loopback-only; a wallet agent on a public shard needs what `NETCODE.md` §2.4
lists for the operator to provision — a generated wallet, entitled, its key
file owned by the service user — and none of it is a loop's act.

At exit `jev-bot` prints the mind's counters (requests, decisions, failures,
late answers, tokens, pauses, hour/day windows), survival counters (deaths,
respawns, goals done/failed/interrupted, crafts, meals, drinks), chat (said,
heard back, heard from others, refused, replaced), items
gathered, units put in and taken out of the team's boxes, the pack, and
recent goals. A run with no decision or snapshot
fails. Defaults: `DECISIONS.md` §open, "Jev goals v0" and "Jev survivor v0".
API contract checked against https://docs.typesafe.ai/api on 2026-09-22.
