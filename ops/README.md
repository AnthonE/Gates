# ops/

What runs on the box behind `game.elopros.com`. Deploying any of it is the
operator's; each unit's header says how it is installed and why it is shaped
the way it is. The agents' manual is `crates/server/AGENT-MANUAL.md`.

| file | what |
|---|---|
| `gates-shard.service` | the public shard (`shard-public.toml`) |
| `certbot-deploy-hook.sh` | refreshes the shard's certificate copies on renewal and restarts it |
| `game-elopros-status.nginx.conf` | fronts the shard's status endpoint |
| `gates-buddies.service`, `buddy/` | the Claude-minded buddies players get on elopros.com, the Watch feed (`buddy/watch.py`) and the journal judge (`buddy/judge.py`) |
| `gates-protagonist@.service` | the house protagonist (Ryuzen), playing around the clock |
| `personas/` | the house characters (`gates-agent --character`): the protagonist, the village cultures, the teammates, Doctor Wood |
| `gates-teammate@.service`, `gates-teammate-claude.conf` | the house teammates |
| `gates-village@.service`, `gates-village-claude.conf` | the villages |
| `gates-clips.service` | highlight clips of the buddies (`buddy/clips.py`) |
| `gates-mint.service`/`.timer`, `mint_pending.py` | mints the unlocks the shard owes, every 30 s |
| `gates-free-door.service`/`.timer` | closes the free door on the Gates copy on 1 Dec 2026 |
| `gates-research-pack.service`/`.timer`, `research/` | packs closed wipes as an open dataset, daily |
| `airdrop.py` | who an airdrop goes to |
| `stream/` | the OBS overlays and `Gates` scene collection used to stream the game |
| `distill/` | distils recorded `gates-agent` decisions into a student mind |
