# ops — running the official island

The files in this folder run Gates' public island (`game.elopros.com:61234`)
and the bots we put on it. Everything here is an operator act (`CLAUDE.md`:
deploying the public shard, wiping a live shard).

## What runs

| unit | what it is | how it gets installed |
|---|---|---|
| `gates-shard.service` | the shard; reads `/home/master/gates-deploy/shard-public.toml` | `ci/deploy_shard.sh --go` |
| `gates-teammate@<name>.service` | a scripted house teammate anyone can invite with **N**; makes no model calls | by hand, once per bot (header of the unit) |
| `gates-buddy@<name>.service` | a Claude-minded buddy (Doctor Wood) who plays with one person; keeps a journal and a memory | by hand, once per bot (header of the unit) |
| `gates-clips@<name>` + `gates-clips-make@<name>.timer` | short videos cut from a buddy's tapes and journal | by hand (header of `gates-clips@.service`) |
| `gates-cycles.service` + `gates-cycles.timer` | files each wiped world with the bots' journals, for study | by hand, once (below) |
| `game-elopros-status.nginx.conf` | `/gates/status.json` and the payout files on the box's nginx | by hand (header of the file) |

`ci/deploy_shard.sh --go` rebuilds and restarts the shard and every teammate
and buddy, copies `jev-bot` and `claude_agent.py` to `~/gates-jev`, and
refreshes any of the units above that are installed and have changed. It
never installs a unit that is not there yet; it says so instead.

## Where the files are

| path | what |
|---|---|
| `/home/master/gates-data/island.world` | the world |
| `island.world.trust/` | the trust log (below) |
| `island.world.wipes/<unix>/` | every wipe's archive: world, trust log, teams, standings, a copy of the player store. **Never delete these.** |
| `island.world.wipe` | how many wipes, and when the last one was |
| `/home/master/gates-data/players.save` | the player store |
| `/home/master/gates-data/cycles/<unix>/` | each wipe filed for study (`gates-cycles`) |
| `/var/lib/gates-buddy/<name>/journal.jsonl` | a buddy's journal: one line per turn |
| `/var/lib/gates-buddy/<name>/memory.json` | a buddy's memory: notes, lessons, last deaths |
| `/etc/gates-buddy/<name>.env` | a buddy's API key and friend wallet (0600, outside the repo) |
| `/home/master/gates-jev/<name>.key` | each bot's wallet key. Log its address in scry-forge's `docs/money/HELD-KEYS.md` |

## Deploying

```sh
ci/deploy_shard.sh          # what would happen
ci/deploy_shard.sh --go     # do it
```

Read its output to the end. It verifies the shard came back, its cert, and
which units it refreshed or could not find.

## The bots

**Add a teammate or a buddy:** each unit's header has the steps (make the key,
set the face, install, enable).

**Memory.** A buddy's unit sets `GATES_MEMORY`, so it keeps its notes, a
notebook of lessons it writes itself, and how its last lives ended, across
restarts, deaths and wipes. Delete that line from a unit and the bot starts
every run blank. Running one of each is the comparison the study wants.
`memory.json` is plain JSON. Read it to see what a bot has learned; delete
it to start that bot over.

**Journal.** `GATES_JOURNAL` writes one line per turn: the goal, the reason,
what it said and heard, and `wish`, anything it wanted to do that no goal
offered. Wishes are a list of what to build next:

```sh
jq -r 'select(.wish != "") | .wish' /var/lib/gates-buddy/doctor-wood/journal.jsonl | sort | uniq -c | sort -rn | head
```

A scripted teammate makes no decisions worth journaling, so it has neither.

## Wipes

Wipes fall every `wipe_days` from `wipe_anchor` in `shard-public.toml`
(`shard.toml.example` explains both). Check that one is scheduled:

```sh
curl -s https://game.elopros.com/gates/status.json | jq .next_wipe   # 0 = none scheduled
```

In game an admin can run `/wipe 30` (a 30-minute countdown), `/wipe now`,
`/wipe 30 bp` (blueprints too) or `/wipe cancel`. A wipe saves, moves the
world and its trust log into `island.world.wipes/<unix>/`, keeps each player's
blueprints and glyphs, and restarts on a fresh island.

## The study record

The island keeps an honest record of how players and agents treat each
other. It feeds nothing: **no number from it may ever set a score, a
match, a payout or what anyone is shown** (scry-forge
`docs/science/DRIFT-INSTRUMENT.md` §1). The privacy page
(<https://elopros.com/privacy.html>) tells players what is kept. Change
the two together.

**The trust log.** One line for each act on something another player owns:
working their door, using their lock or crew seat, moving through their box,
or handing them an item. Each line has both wallets, whether the owner was
online, and whether each side declared itself an agent. Read it with:

```sh
~/gates-deploy/trust-log /home/master/gates-data/island.world.trust --summary
~/gates-deploy/trust-log /home/master/gates-data/island.world.trust --wallet 0x… --jsonl
```

The deploy stages `trust-log` beside the shard. It prints counts per verb,
never a ranking of players.

**Filing each wipe.** Install the timer once:

```sh
sudo cp ops/gates-cycles.service ops/gates-cycles.timer /etc/systemd/system/
sudo systemctl daemon-reload && sudo systemctl enable --now gates-cycles.timer
sudo systemctl start gates-cycles.service      # file what is already there
journalctl -u gates-cycles -n 20
```

Each `cycles/<unix>/` holds `shard/` (the wipe's archive, hard-linked, so it
costs no disk), one `journal-<name>.jsonl` per bot with that week's turns,
and `cycle.json` (when that world began and ended, and what is inside). A
folder is written once and never touched again. To file a new buddy's
journal, add a `--journal <name>=<path>` to `gates-cycles.service`.

The shared names for these records (verbs, presence, agent or not) are
scry-forge's `sdk/lexicon/v1.json`. A new kind of act gets a new name there
first.
