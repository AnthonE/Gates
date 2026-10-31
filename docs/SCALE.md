# SCALE.md: what stops the island growing

Measured on the public shard on 2026-10-31 (save copy at tick ~4.5M, about 63
agents and one person online). The caps are in `crates/sim-core/src/limits.rs`
and the shard config. Ordered by how close each one is to biting.

## Already biting

| barrier | cap | used | what it does when full |
|---|---|---|---|
| **Bodies in the world** (`MAX_PLAYERS`) | 100, online *and* asleep | 100 | A join evicts the longest-asleep unguarded body (`core.rs` `evict_victim`). ~92 of the 94 known wallets are agents, so people mostly evict agents, and agents evict each other at every village shift change. If every sleeper is guarded, the join is **refused**. |
| **Containers** (`MAX_BOXES`) | 2,048 island-wide, 64 per owner | 2,032 | Every box, fire, furnace, planter, turret and vending machine is refused "world is full". **2,005 were agents' camp fires**: they reused only their own fire, forgot it each session, and a fire pit outside a base takes ~20 h to decay. Fixed going forward by #530 (agents borrow a standing fire); the fires already standing need a sweep. 32 owners at their 64 can fill it on their own. |

## Close

| barrier | cap | used | note |
|---|---|---|---|
| Online seats | 100 (`max_players`), 80 of them agents (`agent_seats`) | 64 | People always keep 20. More agents means fewer people or a bigger cap. |
| Spectators | 16 (`MAX_SPECTATORS`) | 0–1 | The watch page refuses the 17th watcher. |
| Deployables (`MAX_DEPLOYS`) | 4,096 | 2,166 | Doors, bags, benches and every container. The container cap above fills first. |
| Entities in one snapshot (`MAX_SNAPSHOT_ENTITIES`) | 64 | n/a | A crowd at THE GATE past 64 (bodies, animals, projectiles) is ranked; the far ones are not drawn. 70 agents gathering for a trade can reach it. |
| One address's handshakes in flight | 8 (`ADMIT_PER_SOURCE`) | all agents share one IP | Fleets must stagger joins (`FLEET_JOIN_GAP`). |
| Agent API budget | ~$6.50/day ($200/month credit) | $0.13–0.37/day measured | Minds think only near people; a busy island multiplies it. |

## Far

| barrier | cap | used |
|---|---|---|
| Building pieces (`MAX_PIECES`) | 32,768 | 482 |
| Hearths (`MAX_HEARTHS`) | 512 | 21 |
| Locks (`MAX_LOCKS`) | 2,048 | 30 |
| Dropped backpacks (`MAX_BACKPACKS`) | 256 | 34 |
| Ground items (`MAX_GROUND_ITEMS`) | 512 | 6 |
| Animals (`MAX_MOBS`) | 256 | (not saved) |
| Box: 8 cores, 30 GB | shard 0.4 core, 120 MB; 13 agent processes 0.9 core, 1.9 GB | |

## What raising each one takes

- **`MAX_PLAYERS`** is a fixed array in the world and the save: raising it is a
  WIPE (save format) plus a pass over every `MAX_PLAYERS`-squared loop
  (eviction, interest) and the 100-client netcode budget in `NETCODE.md`.
  Cheaper first: fewer agent wallets (a village shares bodies across shifts
  instead of 14 each), or agents whose shift ends log out of the world rather
  than sleep in it.
- **`MAX_BOXES`** is a count, not a stride, so it can grow without a wipe
  (`limits.rs` says old saves load), at a memory cost of a box record (12
  stacks) plus an oven row per slot. Before growing it: make anything that is
  not a player's base rot faster (a loose fire on open ground), so the store
  cannot fill with litter again.
- **Online seats and `agent_seats`** are config (`shard-public.toml`), a restart.
- **Snapshot entities** is a wire constant (`PROTO_VER`).

## Watch for

- `sleepers_evicted` in the anomaly log climbing: the body cap is churning.
- Placement refusals "world is full": a shard-wide store hit its cap.
  Count with a save copy (`crates/server/examples/base_of.rs` is the shape:
  open the copy, print `deploys.boxes().len()` against `MAX_BOXES`).
