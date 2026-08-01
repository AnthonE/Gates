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
| 2026-08-01 | **The name is GATES** (*"we are going to start with 'Gates' Rust game clone, make it open source then have agents build it"*; the README header already read `# Gates`). The `ashfall` codename retires; headers updated. ⚠ "gate" also means a CI wall in this repo — in prose, **Gates** capitalized is the game, lowercase **gates** are the walls. | `README.md` · scry `docs/GATES.md` |
| 2026-08-01 | **Open source, agent-built, nightly** (*"make it open source then have agents build it. Nightly builds? … lets work out the agent aide and ace it so any harnass can come help"*). Any harness contributes through `AGENTS.md`; CI runs `./ci/gates.sh` on code PRs; a nightly workflow builds server + wasm artifacts from `main`. Making the repo public is an operator act on GitHub, as is branch protection on `main`. Merge stays a human act. The bar, spoken in the same breath: *"not any ai slop but the best here"* — gates green is the floor, review is the bar. | `AGENTS.md` · `.github/workflows/` |
| 2026-08-01 | **The platform frame is Greenlight-era Steam, not a launchpad** (*"this isnt a launchpad its like steam BEFORE they got rid of steam greenlight"*), and the cascade after Gates is the operator's **MMO, FPS, minecraft clone, and MOBA** — open-source multiplayer titles on the one reserve economy. Platform design of record lives scry-side. | scry `docs/GATES.md` · scry `docs/SENTENCES.md` 2026-08-01 |

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
