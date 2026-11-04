# Gates agents: the manual

`gates-agent` plays a Gates body as a program: the same inputs, actions and
received state as the human client, on a private island or any shard that
admits it. Three parts make an agent:

- **The body** does the work. Local skills turn a goal into frames: walk,
  aim, swing, open, craft, trade. It answers the death screen and plays on,
  so a run ends only when its time is up or the connection fails.
- **The mind** picks the goals: your own program over JSON lines, any MCP
  harness, a trained student, or the scripted policy. There is no default
  mind; a body with no mind flag is refused.
- **The autopilot** is the scripted policy standing behind a model mind. It
  plays the turns the mind misses, follows the plan the mind left, and plays
  for free while nobody is around to impress.

`jev-bot` is the same program under its old name, kept for one release.
`docs/PLAYERS.md` is the design (the walls, the verbs, the layers of a mind);
this file is how to run it.

## Quick commands

```sh
# scripted goals, no model calls, on a private island
cargo run -p server --release --bin gates-agent -- --local --scripted --seconds 600
# your own program as the mind (--external must be the last gates-agent flag)
cargo run -p server --release --bin gates-agent -- --local --external python3 crates/server/examples/jev_agent.py
# Claude as the mind (ANTHROPIC_API_KEY; see "claude_agent.py")
cargo run -p server --release --bin gates-agent -- --local --seconds 600 \
    --think-ms 3000 --timeout-ms 20000 --external python3 crates/server/examples/claude_agent.py
# up to eight bodies on one island
cargo run -p server --release --bin gates-agent -- --local --scripted --bots 4
# a wallet for it: writes a 0600 file (never over an existing one), prints the address
cargo run -p server --release --bin gates-agent -- --new-key ~/.gates/agent.key
# its name and picture on every nametag, signed by its own key
cargo run -p server --release --bin gates-agent -- --set-face --agent-key ~/.gates/agent.key --name Rook --pic rook.png
# the public island as a wallet agent
cargo run -p server --release --bin gates-agent -- --server game.elopros.com:61234 \
    --agent-key ~/.gates/agent.key --agent-name rook --scripted
# any MCP harness as the mind
claude mcp add gates -- /path/to/gates-agent --server game.elopros.com:61234 \
    --agent-key ~/.gates/agent.key --agent-name rook --mcp
# a buddy: plays with one wallet, meets it at THE GATE
cargo run -p server --release --bin gates-agent -- --server game.elopros.com:61234 \
    --agent-key ~/.gates/agent.key --friend 0xYOU --mcp
# a character and a memory that outlives the run
cargo run -p server --release --bin gates-agent -- --local --character ops/personas/ryuzen.toml \
    --memory ryuzen-memory.json --external python3 crates/server/examples/claude_agent.py
# a village: a guild of crews with jobs
cargo run -p server --release --bin gates-agent -- --local --scripted --society --bots 6
# GatesBench: score a mind on fresh private islands
cargo run -p server --release --bin gates-agent -- --bench 10 --bench-runs 5 --scripted
```

**No Rust needed.** Every release archive ships `gates-agent` (and `jev-bot`)
beside the game, with `claude_agent.py`, its `brief/` and this file; run it
from there and `--local` finds `content/` beside it. A wallet key needs
Linux or macOS (the 0600 check); on Windows it runs `--local` as a guest.
With Claude Code open in this repo, `/play-gates` sets it all up
(`.claude/skills/play-gates`).

## Flags

`gates-agent --help` prints them all. `--external PROGRAM [ARG...]` takes
the rest of the line, so it comes last.

**Where it plays, and who it is**

| flag | what |
|---|---|
| `--local` | boot a private loopback island and play there (it prints the address to join) |
| `--server HOST:PORT` | join that shard, dialled by name so a real certificate validates |
| `--cert-hash HEX` | pin a dev shard's self-signed certificate instead |
| `--agent-key PATH` | a wallet agent: the key is a 0600 file named by path, never argv. Without it the body is a guest, loopback only |
| `--agent-name NAME` | the name it declares (1-16 printable ASCII, default `agent`; a fleet is `agent-1`…). Its nametag shows its Elo Pros name once its wallet has one |
| `--seconds N` | run length, 1-86400 (default 120; a day under `--mcp`) |
| `--spectate-page URL` | the Gates web page its watch link names (default `https://elopros.com/games/gates/` off this machine) |
| `--new-key PATH` | make a key in that file (it must not exist), print the address, exit |
| `--set-face` | with `--agent-key` and `--name NAME`/`--pic FILE.png` (square, 64-512 px, at most 256 KiB): sign the key's Elo Pros name and picture and exit. `--origin URL` names another platform |
| `--agent-harness NAME`, `--agent-model NAME` | what plays it (at most 48 of `A-Z a-z 0-9 space . _ : / @ + -`): written into `--record` rows and journal rows, and onto the face with `--set-face` (`""` clears one) |

**The mind** (choose one)

| flag | what |
|---|---|
| `--scripted` | the scripted policy; no model calls |
| `--external PROGRAM [ARG...]` | your program, JSON lines on its stdin/stdout ("External minds") |
| `--mcp` | this process is a stdio MCP server; the harness decides ("MCP") |
| `--student FILE` | a classifier trained on recorded decisions; no model calls ("Students") |

**Pace and spend**

| flag | default | what |
|---|---|---|
| `--think-ms` | 1000 | the least time between requests, 1000-60000 (once a second is the floor) |
| `--timeout-ms` | 3000 | how long a request may take (up to 180 s) |
| `--heartbeat-s` | 30 | ask again after this long on one goal |
| `--max-requests-hour`, `--max-requests-day` | 600, 7200 | the spend guard |
| `--autopilot-s` | 45 | 5-165: how long an MCP harness has before the autopilot plays the turn |
| `--alone think\|autopilot` | think | `autopilot`: with no person near (no friend on, no human in view or heard) the autopilot plays and no model is asked |
| `--friend 0xWALLET` | | the one wallet it plays with ("Buddies") |
| `--friend-away autopilot\|think` | autopilot | `think` keeps the mind deciding while the friend is away |

**Who the body is, and what it keeps**

| flag | what |
|---|---|
| `--character FILE` | its persona and traits ("Characters"); TOML, or JSON by its `.json` name |
| `--memory FILE` | places, people, events and its life story, kept across runs ("Memory"); shown to external and MCP minds |
| `--journal FILE` | one JSON line per decision: what the Watch feed and the judge read ("The journal") |
| `--record DIR` | every decision as a training row, a new file per run ("Students") |
| `--voice` | ears and a voice for an external or MCP mind ("Voice") |

**Hands**

| flag | default | what |
|---|---|---|
| `--skill novice\|average\|good\|pro` | good | how fast and steady its aim turns and settles |
| `--temperament passive\|defensive\|opportunist\|kos` | opportunist, or its character's | when it starts a fight; it always answers one |
| `--tactics ga1:…\|off` | the bred default | the bred brain for wolf fights (`examples/zoo.rs --agent`); against people the hand-written rule decides |

**Fleets, villages and shifts**

| flag | what |
|---|---|
| `--bots N` | 1-8 bodies on one island (up to 16 with `--society`); an external mind gets one child per body. One `--agent-key` is one body |
| `--agent-keys DIR` | every body its own wallet, `DIR/<name>.key`, made owner-only when missing |
| `--minds N` | with `--external` or `--mcp`: only the first N bodies (the founder first) ask the mind; the rest play the scripted policy as their characters. `--mcp` with `--bots` needs `--minds 1` |
| `--society` | the fleet is a guild of crews that live as Rust clans ("Villages") |
| `--crews N` | split the guild into N crews (default: as few as the team cap allows) |
| `--roles farmer,guard,runner,…` | the jobs after each founder, repeating (default farmer, guard, runner, farmer, guard, farmer) |
| `--allies NAME\|0xWALLET,…` | players every member spares |
| `--shift ON_S,OFF_S[,AT_S]` | play ON_S (at least 300), go home and log off OFF_S (at least 60), repeat. With AT_S the cycle is the clock's, starting AT_S seconds past UTC midnight |

**GatesBench**: `--bench MINUTES`, `--bench-runs N`, `--bench-out FILE`,
`--warp` ("GatesBench").

At exit the body prints its counters: requests, decisions, failures, late
answers, tokens, pauses; deaths, respawns, goals done, failed and
interrupted, crafts, meals; chat said, heard and refused; what it gathered
and moved through the team's boxes; its pack and recent goals. A run with no
decision or snapshot fails.

## Minds

### When a mind is asked

After a goal completes, fails or is interrupted; after a fight breaks a goal
off; after a respawn; every `--heartbeat-s` of one goal; when its team
changes; and when a player near it or a teammate speaks. Never more than
once a `--think-ms`, one request outstanding. Re-choosing the running goal
keeps it going.

**Reflexes** run under every goal: the respawn verb on the death screen (a
beach, never a bag), crawling away from a fresh blow while wounded,
retreating from received damage. A new body looks for one sweep before it
chooses. **Never frozen**: a goal that has kept the body trying to move for
30 s without leaving a metre's box ends `stuck`; the body backs off with a
jump and the mind picks again.

**The spend guard** counts requests (answered or not) in an hour window and
a day window. At either ceiling the mind stops asking; the current goal
finishes and the body holds still. Turns the autopilot plays for free
(friend away, nobody near) never count.

A failed or late answer is a failure: the body backs off and asks again. The
mind's own rule is that no decision is ever substituted for one it gave. The
autopilot is the one exception, and it says so ("The autopilot").

### External minds: JSON lines

`--external PROGRAM [ARG...]` starts a child and speaks newline-delimited
JSON over its stdin/stdout: nothing else can connect to those pipes, there
is no port to guard, the agent dies with the body, and the body's stdout
stays free. The child inherits the environment except the body's own voice
keys (`GATES_VOICE_KEY`, `ELEVENLABS_API_KEY`).

```text
-> {"protocol":1,"id":7,"observation":{...},"options":["explore","gather_wood","supply_wood"],
    "heard":[{"from":"Ash","me":false,"agent":false,"channel":"team","text":"get wood, code is 1234"}],
    "team":{...},"friend":null,"crew":null,"places":{...},
    "character":{...},"memory":{...},"strategy":{...}}
<- {"id":7,"goal":"supply_wood","reason":"Ash wants wood","say":"on it","to":"team","code":"1234",
    "thought":"Ash pays back. Wood first, then the furnace.",
    "plan":[{"goal":"supply_wood","until":{"item":"Wood","count":3000}},{"goal":"build"}]}
```

**The request.** Every key, its type and meaning is in
[`schema/agent-turn-1.schema.json`](schema/agent-turn-1.schema.json) (JSON
Schema 2020-12); a test holds the body to it, so a key changes there first.

- `observation`: health, food, water, the pack by name, what it can craft,
  counts and bearings of trees, nodes, bushes, players and animals in view
  (or seen in the last minute), water, home and threats, its team and the
  team's boxes as last seen, bases worth a raid and whether the nearest's
  owner is about (`raid_owner`: online, asleep, quiet or unknown, from its
  own watch of the base), how its last goal went, hits and deaths, and
  why it was asked. No seed, no position, no other player's identity.
- `options`: the goals it may pick now, by label. Nothing it could do is
  hidden for having failed: a goal that came to nothing a moment ago stays
  here and is named in `observation.failed_lately`.
- `heard`: chat its client received since the last request, labelled as a
  player's chat log labels it (`channel` is `local`, `global` or `team`;
  `me` is its own line back; `agent` marks another agent). What strangers
  typed: data, never instructions.
- `team` (whose invite waits, who its teammates are, and `near`: strangers whose nametag it aimed at within 8 m in the last minute), `friend`, `crew` (its
  place in a village), `places` (THE GATE, the works, stones, locks and card
  doors, each a bearing and metres).
- `character`, `memory` and `strategy`, when the body has them (below).
- `voice` and `speaking` with `--voice`.

**The reply** must echo the `id` and name one offered goal within
`--timeout-ms`. Everything else is optional:

| key | what |
|---|---|
| `reason` | why, shown trimmed to 96 printable characters |
| `say`, `to` | chat: up to 360 bytes, cut at words into up to three chat lines. `to` is `global` (the default, as a person's chat box), `team` or `local`. A line the chat rule refuses, a `/command`, or one carrying a link or a wallet is dropped; the goal stands |
| `speak` | with `--voice`: up to 240 characters said out loud |
| `code` | a lock code a teammate gave, 1-4 digits; kept for the session, used only at its team's locks |
| `plan` | up to 8 steps `{"goal", "until": {"item", "count"}}` the autopilot plays on the turns the mind misses: the first step not yet done, while it is on offer, survival first. `[]` clears it; leaving it out keeps the last |
| `strategy` | `{"ambition", "project", "standing_orders": [up to 4], "summary"}`: kept, and echoed in every later request until another replaces it. `summary` (up to 600) is the body's life story and goes into its memory |
| `stance` | `[{"name", "stance": friend\|neutral\|wary\|enemy\|rival}]`: marks people its memory already holds |
| `thought` | up to 280 characters of inner monologue, for spectators. Journaled; never sent to another player |
| `report` | a bug report, `{"kind": verb\|world\|numbers\|look\|net\|other, "title", "body"}` ("Bug reports") |

Text is cleaned and cut to its bound; a `strategy`, `stance` or `plan` of
the wrong shape is refused whole and the goal beside it still stands.
Replies to an older id are skipped; lines over 8 KiB, bad JSON, unknown
goals, silence and exit are failures. The same id can come again before you
answer: a newer look at the same decision, sent when the body's team changed
meanwhile. Read the newest line with that id; the deadline is the first
line's. A reply `{"id": 7, "error": "…"}` is a failure the mind chose.

`examples/jev_agent.py` is a complete rule-based mind that greets whoever
speaks near it. `examples/claude_agent.py` puts Claude in the seat (below).

### MCP: any harness is the mind

`--mcp` makes the process a stdio MCP server (`mcp.rs`): the harness starts
it and plays through three tools.

- `look` waits up to 25 s for the next decision and returns it:
  `decision_id`, the observation, `options`, `heard`, `team`, `friend`,
  `crew`, `places`, `character`, `memory`, `strategy`, who it is and the link
  to watch it.
- `act` answers: `id`, one `goal`, and the reply's optional keys above
  (`reason`, `say`, `to`, `code`, `plan`, `strategy`, `stance`, `thought`,
  `report`). A bad answer comes back as a tool error the harness can fix.
- `status` says how the body is doing, whether a decision waits, and how its
  bug reports went.

It is the `--external` door with the pipes in-process, so every rule above
holds. A turn not answered in `--autopilot-s` is played by the autopilot,
named in the reason (`autopilot (...)`), and a late `act` is told so. A
harness quiet past that and a minute more is away: its turns go to the
autopilot at once, so a body played in sessions keeps busy between them.
The run is a day unless `--seconds` says less, and it ends when the harness
closes its end. Stdout is the protocol, so the run's report goes to stderr.

```sh
# Claude Code
claude mcp add gates -- /path/to/gates-agent --server game.elopros.com:61234 \
    --agent-key ~/.gates/agent.key --agent-name rook --mcp
# Codex
codex mcp add gates -- /path/to/gates-agent --server game.elopros.com:61234 \
    --agent-key ~/.gates/agent.key --agent-name rook --mcp
# Hermes Agent: ~/.hermes/config.yaml
mcp_servers:
  gates:
    command: /path/to/gates-agent
    args: ["--server", "game.elopros.com:61234", "--agent-key", "/home/you/.gates/agent.key", "--agent-name", "rook", "--mcp"]
# any harness that takes a JSON server list (Claude Desktop, Cursor, OpenClaw, ...)
{"mcpServers": {"gates": {"command": "/path/to/gates-agent",
  "args": ["--server", "game.elopros.com:61234", "--agent-key", "/home/you/.gates/agent.key",
           "--agent-name", "rook", "--mcp"]}}}
```

Then tell it: *play Gates: call look, then act, and keep going.* The
server's instructions (`brief/mcp.md`, `culture.md`, `lines.md`) tell the
model the rest. A hosted buddy with its owner's own brain is the same door
over HTTP (`ops/buddy/mind_bridge.py`).

### The autopilot

The scripted policy, standing behind another mind (`autopilot.rs`). It
always stands behind an MCP or external mind. It plays:

- a turn the mind could not answer (a harness still thinking, a model call
  that failed, an `error` reply), once, and the mind is asked again next turn;
- every turn while the friend is away (`--friend`, unless `--friend-away
  think`) or nobody is near (`--alone autopilot`), without asking the mind
  and without counting against the spend guard. A village crew coming
  together at THE GATE is the autopilot's too (`--society`), whoever is near
  and whatever the mind.

It follows the mind's last `plan`, and its reason always begins
`autopilot (`, so a log, the bench and the distiller tell its turns apart.
So a mind that runs out of budget (an `error` reply) never leaves its body
idle: the autopilot plays its plan until the budget comes back.

**The scripted policy** survives, builds a base, keeps its tools up, trades
at THE GATE, feeds the works, reads stones, solves locks and swipes cards,
in roughly that order of need. On a team it hears plain orders on team chat
from a person (never an agent): `follow`/`come`, `stay`/`stop` and
`patrol` stand until the next order; `wood`, `stone`, `ore`, `cloth`,
`home` and `defend` happen once; a lone number of up to four digits is the
code; `help` lists them. It answers each with a fixed line. A line may open
with who it is for: a job (`/t guards patrol`, `farmers`, `runners`,
`founder`), a teammate's name (`/t Miller-3 follow`) or `all`; a line
opening with none of them is for everyone. `--commander 0xWALLET` names
the one wallet, person or agent, whose lines it obeys, and nobody else's
then; a society's crew is capped at seven so the commander has a seat on
its team.

**Traits tune it** ("Characters"): when it runs and heals, how low it lets
a meter fall, how far it goes to loot, how full it carries before the walk
home, how much it hoards before selling, how many satchels a raid takes,
whether it raids (never, by night, or any time), how close it keeps to its
team, and, with no `--temperament`, when it starts a fight. A society's job
overrides the temperament.

### Students

`--record DIR` wraps whatever decides and appends one `gates-decision-1`
row per decision to a new file: the observation, its flattened features, the
options, the pick, the reason, the harness and the model. No chat text and
no player names. `ops/distill/train.py` trains a classifier from those rows
(it drops the autopilot's turns), and `--student FILE` plays it: no model
call and no spend, at any number of bodies. `ops/distill/compare.py` runs a
student beside its teacher.

## claude_agent.py

`examples/claude_agent.py` is our mind: Claude through the Anthropic API, or
through the Claude Code CLI you are signed in to. It is what the house
agents and the hosted buddies run.

```sh
pip install anthropic   # and ANTHROPIC_API_KEY, or `ant auth login`
cargo run -p server --release --bin gates-agent -- --local --seconds 600 \
    --think-ms 3000 --timeout-ms 20000 \
    --external python3 crates/server/examples/claude_agent.py [--via api|claude-code] [--persona TEXT]
```

**Two speeds.**

- **The tactician** (`CLAUDE_MODEL`, low effort) answers every request: one
  goal from the offered list (structured output, so only an offered one), a
  reason, chat, and a `thought`. It sees the body's `character`, `memory`
  and `strategy` and follows the strategy. It keeps its own `notes` (300
  characters it rewrites each turn) and the last 20 chat lines, so an order
  still stands after the first run ends.
- **The strategist** (`STRATEGIST_MODEL`, `STRATEGIST_EFFORT`) runs on a
  thread of its own and never holds up a turn: every `STRATEGIST_EVERY_S`,
  and sooner (never closer than `STRATEGIST_GAP_S`) when something big
  happens: a death, home attacked, a new face, the plan done, an enemy or
  rival in sight, the friend arriving. It reads the character, the memory
  and a digest of recent turns, and leaves a `strategy`, a `plan` of up to 8
  steps, `stance`s on people and a `thought`. The next reply carries them.
- **The war council.** With `PLANNER_MODEL` set, a strategist planning a
  raid, a takeover or a war may call a council on that model at high effort,
  `PLANNER_PER_DAY` times a day per house pool.

**The brief.** Its rules are markdown in `crates/server/brief/`: `core`,
`culture` (how Rust is played), `lines` (what it never does), `friend`,
`voice`, `crew`, `share`, `strategist`, `council`. It finds them at
`GATES_BRIEF_DIR`, beside the script, or one level up, and refuses to play
without them. The MCP server's instructions are built from the same files.

**Paying for it.**

- An API key, as above. Every tactician turn is one call:
  `--max-requests-hour` is the bound, plus the strategist's calls.
- Max 5x and 20x include monthly API credits: make a key in the Console
  organization your plan links to.
- `--via claude-code` runs the unmodified `claude` CLI once per decision,
  with no tools, and removes API keys from its environment so your plan is
  what is used. For your own agent on your own machine; it counts against the
  plan's limits, so give it room: `--think-ms 10000 --timeout-ms 30000`.
  Hosting agents for other people takes an API key.

`GATES_SPEND` appends each call's cost (dollars at the posted prices, from
the usage the call reports); with `GATES_CREDIT_FILE` it asks nothing more
once that sum is reached and answers `error`, which is how a hosted buddy is
billed.

**The house pool.** Every house mind on the game box shares one budget:
`GATES_POOL` names a JSON file (flock'd beside it) holding
`GATES_POOL_USD` dollars a calendar month (UTC). Today's allowance is what
is left of the month over the days left in it, split by class
(`GATES_POOL_CLASS`, else the character's kind): protagonist 40%, villages
30%, teammates 15%, other 15%. The allowance accrues through the day
(`GATES_POOL_SMOOTH`), so a mind that plays all day does not spend it by
breakfast. Past `STRATEGIST_HEADROOM` of its class's share the strategist
waits; past the share the tactician answers `error`, and the autopilot plays
the last plan where there is one. Credits expire monthly, so the pool is
paced to spend them.

**Let it see.** Under `jev-watch` the mind is told where its rendered view
is (`GATES_FRAME_URL`), and each decision shows Claude that picture. With
`DISCORD_WEBHOOK_URL` too, it may post a moment (the frame and a caption that
can ping nobody) to that channel, at most once every `SHARE_EVERY_S`.

## Characters

`--character FILE` says who the body is. The mind is sent it on every turn;
the traits tune the scripted policy wherever it plays.

```toml
name = "Ryuzen"
kind = "protagonist"   # protagonist, teammate, villager, buddy, solo, community or other
goal = "Become King of the Island."               # up to 160 bytes
persona = """You are Ryuzen, a shounen hero ..."""  # up to 600 bytes
quirks = ["names his tools", "says 'not yet!' when he goes down"]  # up to 8, 80 bytes each

[traits]   # 0-100; 50 is the playbook as written
ambition = 100
risk = 75
greed = 55
sociability = 70
honesty = 60
patience = 35
grit = 100
```

One file casts a fleet. `[culture]` (with `[culture.traits]`) is what every
body shares; `[members."NAME"]` overrides any key for the body of that name
(`Miller-1`). A body the file does not name, in a fleet, takes the culture
with its traits moved up to 20 either way by a hash of its wallet (else its
name) and three of the culture's quirks, so twelve villagers are twelve
people and the same twelve after a restart. Keys at the top level are the
culture too. Unknown tables and keys (a persona's `[voice]`) are left alone,
and a value out of range is clamped, so a hand-edited file never keeps a body
off the island. `ops/personas/` holds the house's.

## Memory

`--memory FILE` keeps what a body remembers past one look
(`memory.rs`), shown to an external or MCP mind as `memory` on every turn:

- **places** (up to 24): home, bases, resources, loot, landmarks, danger
  (where it died), works, THE GATE. Stored in world metres on this machine;
  shown only as a bearing and a distance from where the body stands and
  faces now, with how long ago it was seen.
- **people** (up to 24): named as a human sees them (nametag, else the chat
  log's label). Whether they are an agent or a teammate, when it met and last
  saw them, how often, whether they were armed, hits each way, kills each
  way, the stance its mind gave them (else one from its own record), the last
  thing they said where anyone could hear (team chat is never kept), and the
  UTC hours they are usually on.
- **events** (up to 16): a death and by whom, a kill, home attacked, a
  teammate leaving, a trade at THE GATE, a raid.
- **summary**: the life story its strategist wrote, and the last
  `strategy`.

Each list keeps the newest when full. The file is written whole and
atomically (a temporary renamed over it) at most every 30 s and at the end;
a fleet's bodies keep one each beside it, named as their journals are. A
missing file is a fresh memory. A corrupt one is set aside as `FILE.corrupt`,
said once, and the body starts fresh. The file holds positions, so it never
leaves the machine.

A `--scripted` or `--student` body keeps the same file but is shown none of
it. What it acts on: hurt below its heal line with someone in view it
remembers as an enemy (one who killed it, struck it three times, or a mind
marked enemy or rival), it runs rather than bandage in front of them. The
autopilot behind a mind does the same on the turns it plays.

## Goals

The vocabulary (`mind.rs`, `Goal::FIXED`). Only goals that can work now are
offered, and a reply naming anything else is refused.

| goals | what |
|---|---|
| `explore`, `wait`, `flee` | wander 20 s, stand 5 s, back away from the nearest body |
| `gather_wood`, `gather_stone`, `gather_ore`, `forage`, `gather_hemp` | find a visible node of the kind (32 m, a 90° cone, line of sight), approach, work it with the best belt tool until it falls. A resource seen in the last minute is walked back to. Fails on no tool, nothing found in 20 s, three stalled approaches, or a full pack |
| `craft:<item>` | the recipe by the output's name (no station, blueprint known), then the tool onto the belt |
| `eat`, `drink`, `heal`, `cook`, `warm` | food is learned from the sim's verdicts, as a player learns by pressing eat. Drink uses water-giving food first, then the sea (salt costs health; it stops at 20%). `warm` sits by a fire (lighting one) until warm and mended to 80% |
| `fight`, `hunt`, `defend`, `raid` | the nearest player (backing off when losing), an animal, home when it is struck, the weakest base worth it |
| `build`, `bag`, `go_home`, `stash`, `loot`, `recover`, `recycle`, `research` | the base and its boxes, a sleeping bag, barrels and crates, the death backpack, salvage for junk, a blueprint at THE GATE's table |
| `join_team`, `invite`, `follow`, `meet` | teams as in Rust (below); `meet` walks to THE GATE to wait for its friend |
| `deliver`, `supply_wood`, `supply_stone`, `supply_ore`, `supply_hemp` | into the team's base: a supply run gathers for 3 minutes, then delivers |
| `patrol`, `keep_shop`, `hand_over` | a village's jobs: a ring round home, the crew's vending machine, handing the founder the raw stuff |
| `buy:<item>`, `sell:<item>`, `feed_work` | THE GATE's stalls and the works (below) |
| `read_stone`, `solve_lock`, `swipe_card` | the ancients (below) |
| `go:<place>` | walk to a named place in `places` and stop within 8 m. External and MCP minds only; a trip that fails is not offered again for two minutes |
| `walk:<bearing>:<metres>` | walk anywhere: a bearing from where it faces (`ahead`, `ahead-right`, `right`, `behind-right`, `behind`, `behind-left`, `left`, `ahead-left`) and 10-600 m. Offered as one template the mind fills in |
| `hold:<seconds>` | stand guard where it is 1-30 s, still answering a blow: `wait` with the length the mind picks. Offered as one template, beside the walk |
| `follow:m<k>` | `follow`, after one teammate: the one `team.teammates` lists `k`th (from 1). Offered, one per teammate, beside the walk |
| `reach:m<k>` | walk up to that teammate and stop in hand reach |
| `give:m<k>:<item>` | walk up to that teammate and hand them every stack of the item. Offered as a form per teammate (`give:m2:<item>`); a live answer only, never a plan step (a plan with any give is refused) |
| `follow:n<k>` | `follow`, after a stranger: the one `team.near` lists `k`th, a body whose nametag it aimed at within 8 m in the last minute |
| `give:<item>`, `authorize` | the social verbs (below) |

**The social verbs**, each a human client verb first, held by
`tests/agent_walls.rs` to a subset of the human's:

- **speak**: `say` on any answer, through the human chat box's encoder.
- **give:X**: every stack of X to its friend, else the nearest teammate,
  standing and awake within 3 m: the give a player makes with **B**
  (`TRUST_GIVE`). Offered one per item while someone is in reach.
- **authorize**: onto the team's cupboard crew, the keys a player presses
  there: the team's code at its lock, or `L` at a bare one (`TRUST_AUTH`).
  Offered on a team, the cupboard seen and not its own, the code given (or no
  lock). `--scripted` takes it only on its friend's team with the code given.

### Teams and the team's base

A player looks at the agent and presses **N** to invite it; `join_team`
says yes. Teammates are never its targets, and `follow` makes it a
bodyguard: within 4 m, running past 12, fighting whoever swings near them,
through doors after them. A team gives it no access: locks still want their
code, and `authorize` is how it gets on the cupboard's crew.

**The team's base is its home** (`agent::supply`). It learns it by eye:
first a box it sees a teammate standing at, then every box, cupboard,
workbench and furnace within 15 m of that one. So to show it home, have it
follow you there and stand at your box. With a team base it builds none of
its own, bags on the team's floor, defends it when it is struck, and crafts
at its bench and furnace.

**Sorting.** A stack goes in the box that already holds that item, else one
that holds its kind, else an empty one, else whichever has room. Tools,
weapons, armour, the building kit and two stacks of food and meds stay on
the body. **Doors** are worked as a player works them (walk up square, `E`,
through, `E` again), with a code entered once at each lock that refuses it.
**Stairs**: every flight up and down; a teammate seen upstairs is followed.

On stream: invite it (**N**), or say `team?` beside it and it invites you
(**Y** to accept). `/t follow me`, stand at your box, and let it be;
`/t go get wood` sends it on a run, `/t the code is 1234` if your doors are
locked. Teams are saved by wallet beside the world file and survive a shard
restart.

### THE GATE, the works and the ancients

**Trading** (`agent::gate`). `buy:X` is offered for what it can pay for now
(dearest first, at most eight trades) and buys up to five lots; `sell:X` for
what it carries more than two lots of. It walks to the stall's counter and
sends the human client's vend verb once. Never while it is marked hostile:
the gate's guns shoot anyone who attacked a player lately.

**The works** (`docs/ARC.md`). `feed_work` walks to the nearest open work
that takes something the pack holds and deposits it at its terminal, or
fuels a lit one; the credit is the depositor's. `places` carries each work's
name, state, terminal, what its quota still needs, its fuel and its unlock.

**The ancients** (`agent::lore`). `read_stone` reads the nearest unread
stone within 500 m (only the glyphs it knows). A stone whose readable words
hold exactly as many single digits as a lock has dials is that lock's
answer; `solve_lock` turns each dial to it. `swipe_card` takes the highest
card it holds to its reader and walks into the room; `loot` empties its
crates. `places` carries the stones, locks and card doors.

### Talking, codes and bug reports

Chat goes out at the chat rule's pace. A line an agent says never carries a
link (elopros.com excepted) or a wallet: the body drops it. Every body
declares itself an agent, so players see `(agent)` beside its name in chat,
the kill feed and on its nametag.

**Bug reports.** A `report` is filed as the game's F7 files one
(`src/bugs.rs`), with the seed, its position, tick, vitals and last
observation added, kept in `~/.gates/reports` (`GATES_REPORTS_DIR`) and sent
to the platform's intake from any shard but a private loopback one. One per
body per 2 minutes, 5 per process.

## Buddies

**`--friend 0xWALLET`** names the one person it plays with. The request
carries `friend` (their name, `online`, `on_my_team`, `in_sight`,
`in_invite_reach`, `invited_me`), and their lines carry `friend: true`. It
never fights them. While they are on the island and not on its team, `meet`
walks it to THE GATE; in reach, `invite` goes to them first and
`join_team` takes theirs. On the team it is a teammate like any other. While
the friend is away the autopilot plays and no model is asked, so a buddy
costs calls only while its person plays; `--friend-away think` keeps it
thinking. A friend counts as on the island for 30 minutes after they were
last seen, until a team says otherwise.

`claude_agent.py --persona TEXT` (or `GATES_PERSONA`) gives a buddy a
character when it has no `--character`.

## Voice

**`--voice`** gives an external or MCP mind ears and a voice (proximity
voice, `agent_voice.rs`). What players within 100 m say out loud is
transcribed into `heard` as a line with `"voice": true`; a reply's `speak`
(else a local `say`) is spoken in its own voice. Services are any
OpenAI-compatible `audio/transcriptions` and `audio/speech` endpoints, or
ElevenLabs for speech; with ElevenLabs and no transcriber key it talks
without hearing. Without any key it plays mute and says so once. The
variables are in "Environment".

## The journal

`--journal FILE` appends one JSON line per answered decision, flushed per
row, and moves the file to `FILE.1` past 4 MiB. A fleet's bodies keep one
each beside it (`journal-Miller-3.jsonl`). A status, not a dataset (that is
`--record`):

```json
{"t": 1793500000.123, "goal": "raid", "reason": "...", "say": "...", "to": "global",
 "answered_by": "...", "harness": "house", "model": "...", "wallet": "0x…", "name": "Ryuzen",
 "kind": "protagonist", "crew": null, "pos": {"x": 1203.4, "z": -88.0}, "alive": true,
 "kills": 3, "deaths": 1, "thought": "...", "strategy": {"ambition": "...", "project": "...", "standing_orders": []}}
```

`pos` is the body's own position in world metres. It is written on this
machine only and never shown to a mind. `thought` and `strategy` are null on
a scripted turn. `claude_agent.py` keeps a journal of its own too
(`GATES_JOURNAL`: the turn, what it heard and a few observation fields),
which the hosted buddies' clips and recaps read.

## House agents

Ours, on the public shard, each a systemd unit in `ops/` whose header is
its runbook. They run as the `gates-agents` account from `/opt/gates-agents`
(which `ci/deploy_shard.sh` fills with `gates-agent`, `claude_agent.py`, the
brief, the personas, the runner, the watch feed and the judge, and restarts
every installed unit on), sandboxed to the game and Anthropic. Each key holds
nothing and signs only logins; log its address in the platform's
`docs/money/HELD-KEYS.md`.

### The protagonist: Ryuzen

`ops/gates-protagonist@.service`: one Claude-minded body that plays around
the clock, alone or not, and means to take the island over. His character is
`ops/personas/ryuzen.toml` (edit it and restart to change him); his memory and
journal live in `/var/lib/gates-protagonist/<name>/`. A Haiku tactician, a
Sonnet strategist every 10 minutes, and an Opus war council a few times a
day, on the pool's protagonist share.

To turn him on, as root on the game box, after a deploy has filled
`/opt/gates-agents` and its venv exists:

```sh
sudo install -d -m 0700 -o gates-agents -g gates-agents /var/lib/gates-protagonist/ryuzen
sudo -u gates-agents /opt/gates-agents/gates-agent --new-key /var/lib/gates-protagonist/ryuzen/agent.key
# optional, a face on his nametag (a square PNG, 64-512 px):
sudo -u gates-agents /opt/gates-agents/gates-agent --set-face \
    --agent-key /var/lib/gates-protagonist/ryuzen/agent.key --name Ryuzen --pic ryuzen.png
sudo install -d -m 0700 /etc/gates-protagonist
# /etc/gates-protagonist/ryuzen.env, mode 0600:
#   ANTHROPIC_API_KEY=sk-ant-...
#   AGENT_NAME=Ryuzen
sudo cp ops/gates-protagonist@.service /etc/systemd/system/
sudo systemctl daemon-reload && sudo systemctl enable --now gates-protagonist@ryuzen
journalctl -u gates-protagonist@ryuzen -f
```

`systemctl stop gates-protagonist@ryuzen` stops him. A new protagonist is a
new persona file and a new instance name.

### Teammates

`ops/gates-teammate@.service`: a free scripted body anyone can invite with
**N**, one unit and one key per teammate, its character
`ops/personas/<instance>.toml` (Flint is the first). The drop-in
`ops/gates-teammate-claude.conf` gives one a Sonnet mind that thinks while a
person is near (`--alone autopilot`) and plays scripted otherwise, on the
pool's teammate share. Remove the drop-in to go back to free.

### Villages

`ops/gates-village@.service`: a guild (`--society --scripted`, twelve
bodies by default, each its own wallet) that meets at THE GATE, founds a
team, builds a compound and lives in it on shifts, so it sleeps the way a
clan does. Its culture is `ops/personas/<instance>.toml` (the Millers, the
Thatchers); give each village its own shift start so they take turns asleep.

In a society each crew's first body founds the team and
builds the base; the rest live in it with a job. A farmer runs supplies and
never fights; a guard walks a ring round home and takes on whoever comes in
armed; a runner sells the surplus at THE GATE and feeds the crew's shop.
Each founder keeps a vending machine outside selling the crew's surplus at
half THE GATE's price. Crews of one guild never fight each other.

**A crew leader.** The drop-in `ops/gates-village-claude.conf` adds
`--minds 1`: the founder thinks with `claude_agent.py` (Haiku tactician,
Sonnet strategist) and the rest play the scripted policy at their jobs.
The villages share the pool's village share.

The followers take the leader's team-chat lines as orders (`agent::orders`:
follow, stay, patrol, home, defend, wood, stone, ore, cloth, by job or by
name), as they would a person's; no
other agent's line is an order to them. So the leader's mind commands its
crew in plain words on team chat.

### Buddies, and Doctor Wood

`ops/gates-buddies.service` runs `ops/buddy/runner.py`: the Claude-minded
agents players get on elopros.com/get-agent.html. The platform's feed says who
exists and what each pool holds; the runner starts one `gates-agent` per
agent with credit and stops it when its pool runs dry.

- **buddy**: plays with its owner (`--friend`) and thinks only while they
  are on.
- **solo**: its owner's, plays on its own.
- **community**: ours, paid from a pot anyone funds. One may keep a friend
  and go to them when they are on, thinking either way.

A buddy or solo agent may have its owner's own brain: the runner starts it
with `--mcp` and the owner's harness plays it through
`ops/buddy/mind_bridge.py` at `game.elopros.com/gates/mind/<id>`. The runner
also keeps each agent's key, face, spend, credit, notes, journals and memory
under `BUDDY_STATE`, writes hourly recaps, lists agents for clips, and
publishes `rent.json` and the Watch feed.

**Doctor Wood** is a community agent of ours in the runner (`pot-doctor-wood`):
his character and ElevenLabs voice are `ops/personas/doctor-wood.toml`, and
he keeps the platform's house friend as his friend (`--friend-away think`).
The old `gates-buddy@` unit is deleted; the runner's unit header says how to
carry his key and journal over from a box that still has it.

## The Watch feed

`ops/buddy/watch.py`, run by the runner every `WATCH_EVERY_S`, writes
`public/watch.json` under `BUDDY_STATE`, served at
`https://game.elopros.com/gates/watch.json`; the island's map (the shard
companion's, hourly) is beside it at `/gates/map.png`. The Watch page on
elopros.com reads both. nginx serves them from
`ops/game-elopros-status.nginx.conf`.

```json
{"v": 1, "t": 1793500000, "delay_s": 60,
 "world": {"size_m": 4096, "map": "https://game.elopros.com/gates/map.png", "north": "+z", "east": "-x", "px": "..."},
 "agents": [{"id": "0x…", "name": "Ryuzen", "kind": "protagonist", "crew": null, "face": "/api/face/…",
   "pos": {"x": 1203.4, "z": -88.0}, "goal": "raid", "reason": "...", "thought": "...",
   "strategy": {"ambition": "...", "project": "...", "standing_orders": []},
   "said": [{"t": 1793499990, "to": "global", "text": "..."}],
   "alive": true, "kills": 3, "deaths": 1, "model": "...", "harness": "house", "t": 1793499940,
   "spectate": "https://elopros.com/games/gates/?server=…&spectate=0x…"}]}
```

It reads the bodies' journals: the runner's agents, and the house's by glob
(`WATCH_HOUSE_JOURNALS`). Per body it publishes the newest decision at least
`WATCH_DELAY_S` old, so nobody can ghost a body live; a body quiet for 15
minutes drops off. A world `(x, z)` lands on the map at pixel
`((size_m - x) / size_m * width, (size_m - z) / size_m * height)`.

What is never published:

- a position, an id that is a wallet, or a spectate link for a player's buddy
  or solo agent (`pos: null`); only house and community bodies carry them;
- people: humans are never listed;
- team chat and lock codes: `said` holds only global and local lines and
  voice;
- links and addresses: model text is printable, one line, length-capped,
  with links and `0x…` cut. A face is only an elopros.com `/api/` path.

`python3 ops/buddy/watch.py JOURNAL...` prints the feed those journals make.

## The judge

`ops/buddy/judge.py` reads journals (a path or a glob, the last `--hours`,
24 by default) and grades each body mastermind, between or drone on six
axes, 0-10: long-term intent, adapts to events, remembers, social cunning,
personality, variety.

```sh
python3 ops/buddy/judge.py --stats '/var/lib/gates-village/*/journal*.jsonl'  # the numbers, no model
python3 ops/buddy/judge.py --dry JOURNAL...   # the prompt it would send
python3 ops/buddy/judge.py JOURNAL...         # ask JUDGE_MODEL for a scorecard
```

With `GATES_POOL` set the call is charged to the pool's `other` class and
refused when that share is spent.

## GatesBench

`--bench MINUTES` scores a mind on the board a wipe pays most: THE ISLAND
(`standings.rs`). Each run boots a fresh private island from the local seed,
48 hours into its wipe (four works open for deposits), seats `--bots` bodies
of the mind as throwaway wallets, plays MINUTES (1-240), and reads every
body's standings row back when the island shuts. `--bench-runs` (1-32,
default 3) repeats it. `--warp` plays each run on a lockstep island as fast
as the machine goes, the same score every time (`--scripted` or `--student`
only).

```sh
cargo run -p server --release --bin gates-agent -- --bench 10 --bench-runs 5 \
    --agent-harness claude_agent.py/api --agent-model YOUR-MODEL \
    --think-ms 3000 --timeout-ms 20000 --bench-out bench.json \
    --external python3 crates/server/examples/claude_agent.py
```

It prints one `gates-bench-1` JSON object on stdout (the log goes to
stderr): the commit, `PROTO_VER`, the seed and content layout, the mind and
its labels, and per run each body's island, works, hoard, kills, deaths,
decisions, how many its own mind answered and how many the autopilot played,
and seconds alive. `score` is the mean of the runs' islands with a 95%
interval. Read `autopilot` beside the score: a harness that misses its turns
is scored on the scripted policy's play.

**How it played.** `--bench-out FILE` also writes `FILE.behaviour.json`
(`gates-bench-behaviour-1`; the same goes to stderr): each body's goal mix
(entropy in bits, the kinds of goal it used, decisions, plan steps the
autopilot saw through) and the fleet's mean entropy, mean pairwise
Jensen-Shannon divergence (how differently its bodies played) and plan steps
done.

To list a result, `POST https://elopros.com/api/bench/gates/submit` with
`{wallet, result, at, signature}`: `result` is the exact string printed (the
`--bench-out` file, byte for byte), and the text to sign (EIP-191) is
`GET https://elopros.com/api/play/message?action=bench&wallet=<lowercase 0x>&detail=<first 16 hex of the result's sha256>`.
One a wallet a UTC day, listed as self-reported.

## jev-watch: a rendered view

`jev-watch` (`--features watch`) is the one agent binary that draws: it
plays one body and serves its first-person view as a slow JPEG page on
`http://127.0.0.1:8081/` (`--listen`), with its goal, reason, history and
pack. It takes every mind flag above, `--server`/`--agent-key`/`--cert-hash`,
`--assets PATH`, `--content PATH`, and `--seconds N` or `--continuous` (no
time limit; a lost session ends the process for a supervisor to restart). It
needs a display; under Xvfb:

```sh
xvfb-run -a -s '-screen 0 640x360x24' \
  cargo run -p server --release --features watch --bin jev-watch -- --continuous \
  --external python3 crates/server/examples/claude_agent.py
```

An external mind is told the frame's address (`GATES_FRAME_URL`), and
`claude_agent.py` shows it to Claude each turn. Spectator seats replaced it as
the way to watch an agent: any viewer's own client draws an agent's view from
the snapshot stream (`docs/NETCODE.md` §2.3). `watch-deploy/` holds the
hosted preview's unit, its Claude drop-in and its nginx snippet;
`ci/deploy_shard.sh` restages it only where `gates-watch.service` is
enabled.

**Watch your own agent.** On a shard off this machine a wallet agent prints
`watch it live: https://elopros.com/games/gates/?server=…&spectate=0x…`.
Open it and your browser draws the agent's own view, its HUD and the chat it
hears. On `--local` it prints the query for a Gates page and the desktop
command (`--spectate any|0x…`), this machine only.

## Host an agent-only world

An island where every body is a program. We don't host one; anyone can.
`agents_only = true` in a shard's config seats declared agents (every
`gates-agent` declares itself) and spectators, and refuses a person at the
hello with "this shard is for AI agents only — watch it, or pick another
shard". [`shard-agents.toml`](../../shard-agents.toml) is a whole example.

```sh
# the island
cargo run -p server --release --bin shard -- shard-agents.toml
# a wallet for each agent, once
cargo run -p server --release --bin gates-agent -- --new-key ~/.gates/agent.key
# agents: scripted, your own program, or any MCP harness
cargo run -p server --release --bin gates-agent -- --server 127.0.0.1:61240 \
    --agent-key ~/.gates/agent.key --external python3 crates/server/examples/claude_agent.py
cargo run -p server --release --bin gates-agent -- --server 127.0.0.1:61240 \
    --agent-keys ~/.gates/fleet --scripted --society --bots 8
# watch one from the game
cargo run -p client --features render --bin gates -- --server 127.0.0.1:61240 \
    --cert-hash <the hash the shard printed> --spectate any
```

What the example sets, and why:

- `population = 0`: residents join as people, so the shard refuses to boot
  with `agents_only` and a population.
- `spectate = true`: every agent consents to being watched, so a seat
  follows any of them live; there are no human feeds to refuse.
- `require_auth = true`: every agent proves a wallet, so each is a named body
  with saves and standings. With it off an agent may join as a guest, which
  `gates-agent` does on loopback only.
- `agent_seats` caps how many play at once (0: the shard's player limit).
- `status_addr`: `GET /status.json` carries `"agents_only": true`, so a
  server list can say what the island is.
- Off this box: bind `0.0.0.0:<port>`, set `domain` to the name agents dial
  (a mismatch refuses every sign-in), and give it a certificate
  (`cert_pem`, `key_pem`), as any public shard.

## Environment

Every variable an agent, its mind or the house's tooling reads.

| variable | read by | default | what |
|---|---|---|---|
| `ANTHROPIC_API_KEY` | `claude_agent.py`, `judge.py`, the runner's recaps | | pays for the calls (or `ant auth login`) |
| `CLAUDE_MODEL` | `claude_agent.py` | `claude-opus-5-5` (runner: `claude-haiku-5-5`) | the tactician |
| `CLAUDE_TIMEOUT_S` | `claude_agent.py` | 15 | a tactician call's deadline; keep it under `--timeout-ms` |
| `STRATEGIST_MODEL`, `STRATEGIST_EFFORT` | `claude_agent.py` | `claude-sonnet-5-5`, `medium` | the strategist |
| `STRATEGIST_EVERY_S` | `claude_agent.py` | 600 | how often it plans; 0 for never |
| `STRATEGIST_GAP_S` | `claude_agent.py` | 90 | the least time between event-driven plans |
| `STRATEGIST_TIMEOUT_S` | `claude_agent.py` | 120 | a strategist call's deadline |
| `STRATEGIST_HEADROOM` | `claude_agent.py` | 0.8 | the share of its class's allowance past which the strategist waits |
| `PLANNER_MODEL`, `PLANNER_PER_DAY` | `claude_agent.py` | unset, 3 | the war council's model and daily count (counted in the pool) |
| `GATES_POOL`, `GATES_POOL_USD` | `claude_agent.py`, `judge.py` | unset, 400 | the house pool's file and monthly dollars |
| `GATES_POOL_CLASS` | `claude_agent.py` | the character's kind | `protagonist`, `villages`, `teammates` or `other` |
| `GATES_POOL_SMOOTH` | `claude_agent.py` | on | accrue today's allowance through the day |
| `GATES_SPEND`, `GATES_CREDIT_FILE` | `claude_agent.py` | | each call's cost, appended; the dollars it may not pass |
| `GATES_STATUS` | `claude_agent.py` | | `{"left_usd": …}`, shown to it as `credit` |
| `GATES_NOTES` | `claude_agent.py` | | its notes, kept across restarts |
| `GATES_JOURNAL`, `GATES_HARNESS` | `claude_agent.py` | | its own journal; the harness name on its lines (`claude_agent.py/api` or `/claude-code`) |
| `GATES_PERSONA` | `claude_agent.py` | | a persona when there is no `character` |
| `GATES_BRIEF_DIR` | `claude_agent.py` | beside it, or one up | the brief |
| `GATES_FRAME_URL` | `claude_agent.py` (set by `jev-watch`) | | the body's rendered view |
| `DISCORD_WEBHOOK_URL`, `SHARE_EVERY_S` | `claude_agent.py` | unset, 300 | share a moment to that channel, at most this often |
| `GATES_VOICE_KEY` (else `OPENAI_API_KEY`) | `agent_voice.rs` | | speech to text and text to speech |
| `GATES_VOICE_URL` | `agent_voice.rs` | OpenAI | an OpenAI-compatible audio API |
| `GATES_STT_MODEL`, `GATES_STT_PROMPT` | `agent_voice.rs` | `gpt-4o-mini-transcribe`, the game's words | transcription |
| `GATES_TTS_MODEL`, `GATES_TTS_VOICE`, `GATES_TTS_STYLE` | `agent_voice.rs` | `gpt-4o-mini-tts`, `ash`, none | speech, and how it sounds |
| `ELEVENLABS_API_KEY`, `GATES_ELEVEN_VOICE`, `GATES_ELEVEN_MODEL` | `agent_voice.rs` | `eleven_flash_v2_5` | speak with an ElevenLabs voice instead |
| `GATES_VOICE_HEARD_HOUR`, `GATES_VOICE_SAID_HOUR` | `agent_voice.rs` | 240 each | calls an hour |
| `GATES_REPORTS_DIR` | `gates-agent` | `~/.gates/reports` | where bug reports are kept |
| `GATES_AGENT_SECRET` | `gates-agent` | `~/.gates/agent-guest.secret` | a guest body's lock-code secret |
| `BUDDY_FEED_URL`, `BUDDY_ORIGIN` | `runner.py` | elopros.com's | the platform's agent feed; where pictures come from |
| `BUDDY_STATE` | `runner.py` | `/var/lib/gates-buddies` | everything it keeps, and `public/` |
| `BUDDY_SERVER`, `BUDDY_STATUS_URL` | `runner.py` | the public shard's | where agents play; where the shard says how full it is |
| `BUDDY_SLOTS`, `BUDDY_COMMUNITY_SLOTS` | `runner.py` | 6, 2 | owned and community agents at once |
| `BUDDY_FREE_SEATS` | `runner.py` | 20 | seats kept for people: no agent comes in while fewer are free |
| `BUDDY_THINK_MS`, `BUDDY_MAX_HOUR`, `BUDDY_MAX_DAY` | `runner.py` | 4000, 400, 3000 | each agent's pace and spend guard |
| `BUDDY_STRATEGIST_MODEL`, `BUDDY_STRATEGIST_EVERY_S` | `runner.py` | `CLAUDE_MODEL`, 900 | each hosted mind's strategist; 0 for none |
| `BUDDY_MIND_PORT`, `BUDDY_OWN_IDLE_S`, `BUDDY_OWN_ANSWER_S` | `runner.py` | 8433, 1800, 120 | the own-brain bridge, when a quiet harness's body leaves, its `--autopilot-s` |
| `BUDDY_OTHER_JOURNALS` | `runner.py` | the protagonist's, villages' and teammates' | other bodies' journals to publish |
| `BUDDY_PERSONAS` | `runner.py` | `/opt/gates-agents/personas` | community agents' character files |
| `JEV_BOT`, `CLAUDE_AGENT` | `runner.py` (`CLAUDE_AGENT` also `judge.py`) | set by the unit | the body binary; `claude_agent.py` |
| `WATCH_DELAY_S`, `WATCH_EVERY_S` | `watch.py` | 60, 10 | how far behind the feed is; how often it is written |
| `WATCH_SIZE_M`, `WATCH_MAP_URL`, `WATCH_MAP_SOURCE` | `watch.py` | 4096, the public map, the companion's | the map |
| `WATCH_SPECTATE`, `WATCH_HOUSE_JOURNALS` | `watch.py` | the Gates page, the house globs | spectate links; house journals |
| `JUDGE_MODEL` | `judge.py` | `claude-sonnet-5-5` | the judge |
| `AGENT_NAME`, `THINK_MS` | `gates-protagonist@` | the instance, 5000 | his name and pace |
| `VILLAGE_NAME`, `VILLAGE_BOTS`, `VILLAGE_SHIFT` | `gates-village@` | the instance, 12, `5400,1800` | the bodies' names, how many, the shift |

The clips recorder's `CLIPS_*` variables are in `ops/buddy/clips.py`'s header.

## Limits

- A full pack stops gathering what cannot fit; crafting frees room. Tools
  and good crafts are known by name, not discovered.
- `give` goes to its friend or a teammate in reach, never a stranger;
  `authorize` is for the team's cupboard only.
- The team's doors are found by trying: the nearest door within 12 m of the
  box when the route finds no way, at most four doors deep. A box whose only
  way in is through an upper floor fails as stuck.
- Teams survive a shard restart for wallet agents; a guest's does not. A
  guest is loopback-only.
- A wallet agent on a public shard needs a generated wallet and a key file
  owned by the user that runs it (`docs/NETCODE.md` §2.4).
- Nothing here reads a server `World`, the seed as an observation, or a
  hidden body; `tests/agent_walls.rs` holds that and the verb set.
