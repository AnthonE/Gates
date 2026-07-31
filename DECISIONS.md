# Gates · DECISIONS.md — spoken, and waiting to be spoken

Two lists. **Spoken** is append-only: date, the call, the operator's own
words, a pointer. **Open** is the knob registry — every **(knob)** in the
docs, deduped, with its shipping default. A loop may rely on a default;
only the operator moves a knob from open to spoken. No number gets
invented into code that isn't on this page.

## Spoken

| date | decision | pointer |
|---|---|---|
| 2026-07-30 | **The game is a separate repo, sold through scry's Great Work board by the operator's own agent** (*"ill make it a separate repo… make sure the website and backend are all setup for this"*). Quest `munus-first-sale` posted; claim/submit verbs live on the board. | scry `SENTENCES.md` · `MUNUS.md` |
| 2026-07-30 | **The stack**: Rust server, three.js client, WebTransport/QUIC, no-blocking/no-alloc server discipline (*"backend is in rust, frontend three.js… no allocations in the hotpath… it should be a great game to build upon if we ace the skeleton"*). | `DESIGN.md` · `NETCODE.md` |
| 2026-07-30 | **The coins**: OBOL is the in-world working coin; SCRY and MYRRH price skins, cosmetic only (*"how might we manage to get obol into the mix? Scry and Myrhh can be the premium currency for skins"*). Never-table stands as a wall. | `DESIGN.md` §3 |
| 2026-07-30 | **Built by a loop system** (*"we are going to use a loop system we have to build this out"*). Loop discipline in `CLAUDE.md`; this file + `NOW.md` are the steering. | `CLAUDE.md` |
| 2026-07-31 | **The name is `Gates`** — not `ashfall`, which no one chose (*"idk where the name ashfall came from its 'Gates' or Scry Gate"*; asked which, answered `Gates`). Matches the repo; stays a separate brand orbiting scry per DESIGN §13. Knob closed, removed from §open and DESIGN §14. | `DESIGN.md` §13 |

## Open (defaults ship until spoken)

| knob | default | doc |
|---|---|---|
| shard cap / reference box | 100 / 4-core VPS | DESIGN §14 |
| wipe cadence · BP survival | monthly · BPs one extra cycle | DESIGN §2 |
| island size | 2,048 m | TERRAIN §6 |
| hunger depth | minimal timer-drain | DESIGN §2 |
| day/night length | 45 min | ALPHA §1 |
| bag cooldown · cap | 5 min · 8 | ALPHA §1 |
| nametag range | 8 m, aim-only | ALPHA §1 |
| local chat | on, 20 m | ALPHA §1 |
| claim grace (disconnect standing window) | 10 s | NETCODE §6.3 |
| haven sleeper timeout | 20 min | NETCODE §6.3 |
| node/barrel respawn | 20–45 / 15–30 min jittered | TERRAIN §2 · CONTENT §5 |
| despawn base constant | 5 min × rarity (≈5/20/40/60) | CONTENT §1 |
| balance bands: raid ratio · TTK · farm rate | ≈1.5× starter · 3–6 hits · 300/node | CONTENT §4 |
| revolver: barrel drop or craft-only | rare drop | CONTENT §5 |
| food set (meat cut?) | berries/mushrooms/corn, meat cut | CONTENT §2 |
| bank deposit fee | 2%, burns | DESIGN §3.1 |
| OBOL allotment + claim cadence | unset — scry-side operator act | DESIGN §3.1 |
| skin catalog, prices, SCRY/MYRRH split | unpriced until posted | DESIGN §3.2 |
| skin proceeds fiscus/burn split | 100% fiscus | DESIGN §3.2 |
| queue priority for sale | **never** (flipping is a sentence) | DESIGN §3.3 |
| A1→A2→A3 arming dates | unset — each an operator act | ALPHA §2 |
| WebSocket fallback lane | not in alpha | NETCODE §2 |
| hosting provider · domain | unset | ALPHA §3 |
| playtest channel (Discord/Telegram) | unset | ALPHA §4 |
| coalescing rates · stream-in batch sizes | per NETCODE, bench-tuned | NETCODE §5 |
| grass · tree LOD distances · map-as-item | cosmetic defaults | TERRAIN §4/§5 |
| sim movement constants | walk 3.0 · sprint 5.5 m/s · gravity 20 m/s² · terminal 50 m/s · wade ×0.5 below ground 0.4 m · world border margin 8 m | sim-core `movement.rs` |
| velocity quantization | 1 cm/s (positions per DESIGN §5.5: 3 cm x/z · 1 cm y) | sim-core `movement.rs` |
| worldgen shape params | **all** shape constants in `terrain.rs` (relief 1/600 m gain 2.4 · warp 45 m @ 1/1200 · coast r 960±100 edge 160 · sea floor 12 m · moisture 1/700 · ridge 16 m @ 1/220 from 52 m · biome bands beach <2 m / highland >52 m / forest moist >0.05 · remap LUT v0 · scatter weights v0), pinned by `test_terrain_golden` | sim-core `terrain.rs` |
| command buffer cap | 256 per tick, overflow defers to next tick | sim-core `limits.rs` |
| spawn placeholder (until the spawn-ring slice) | 96 hashed candidates · interior 224–1824 m · height 1.5–45 m · slope < 1.0 · fallback island center | sim-core `world.rs` |
| snapshot v0 wire widths | pos x/z 17 bit · y 14 bit @ −20.48 m bias · vy 14 bit @ ±81.9 m/s (covers terminal 50; NETCODE §3's ±16 predates the spoken terminal) · pos-delta 8/10/8 bit · ids u32 · counts 7 bit, pinned by `test_protocol_golden` | protocol `lib.rs` |
| server ring & buffer caps | input ring 32 datagrams (drop newest — redundancy re-carries) · snapshot ring 4 (skip) · ctrl 8 / graveyard 256 (refuse/retry) · input buffer 16 frames (skip ahead) · pending removals 256 (resync) | sim-core `limits.rs` |
| nudge fill-ins (NETCODE §4 speaks target 1–2 · >6 → consume 2 · empty → resync) | Faster at depth 0 · Ok 1–2 · Slower ≥3 · starve 30 ticks before hard-resync | server `client.rs` |
| net plumbing bounds | handshake timeout 5 s · writer poll 2 ms · sim backlog drop past 8 ticks | server `net.rs` |
