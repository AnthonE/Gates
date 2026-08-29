# CLAUDE.md — Gates

The operating manual for anyone — human or agent loop — working this repo.
Read this first, every iteration. It is deliberately bounded: no dated
state, no counts, no rules for things that don't exist yet. If a claim
here is wrong, fix the claim; history goes in `DECISIONS.md`.

## What this is

A survival game (Rust-the-game tradition): Rust-language authoritative
server, **a native Rust desktop client** (Bevy; operator, 2026-08-05),
WebTransport/QUIC. A separate product that orbits elo — sold through its
Great Work board, coins from its economy — importing none of its code.
**The skeleton is the product**: determinism, netcode, and the hot-path
laws outrank every feature.

**The browser client is deleted** (operator, 2026-08-06: cut, then *"we have
it all backed up on github… we dont need it locally"*). `web/` and its eleven
gates are out of the tree. The native client is the only client.

It is not lost — it is in git history, on GitHub, and readable when a question
about a verb needs it: `git show <commit>:web/src/interact.js`. That matters
more than it sounds, because it WAS the reference implementation of every verb
the native client now carries (the two pick resolvers, `map.js`'s hillshade,
`refusals.js`'s tables), and the doc comments in `crates/` that cite
`web/src/props.js` for a constant still point at something real
(`grep -rn 'props\.js' crates/` — 24 on 2026-08-11, and **the command is the
claim, not the number**: this line said ~36 until it was re-run, which is the
same drift the ⚠ below is about). Read it from history; never restore it to
the tree.

⚠ **Separate the two kinds of dead citation, because only one is harmless.**
"The browser held this constant" is history and stays. "`ci/<gate>.mjs`
refuses a drift between them" is a claim that **something is enforced**, and
eleven deleted gates make many of those false in the present tense — the doc
reads as covered while nothing checks it. Swept 2026-08-09 in the `.md`
files and **2026-08-11 in `crates/`**, which had been the outstanding half:
15 doc comments claimed a deleted gate in the present tense ("still scores",
"holds the two equal", "refuses a drift") and now name it in the past with the
consequence stated, which is usually *nothing enforces this now*. Three gates
cited from `crates/` are still live — `knob_registry.mjs`, `parity.mjs`,
`haven_prize.mjs` — and eleven are gone, so the check when you add a citation
is `ls` the file, not memory. The first mirror anybody actually re-checked had
drifted (`TERRAIN.md` §7.1).

**Bevy draws, it does not decide.** `sim-core` keeps the walls and
`ClientCore` keeps prediction, so gameplay state never enters the ECS, where
it would retire the determinism walls with nothing in CI to notice. This is
the one client rule that still binds.

**There is no visual gate, and this is now a deliberate choice rather than a
debt.** The rule inherited from `MIGRATION.md` — a render path may not land
without its probes — is **retired** (operator, 2026-08-06). Two reasons, and
the first is this repo's own measurement: `vantages.mjs` passed all 36 checks
on a beige smear with no sky, no horizon and no object in it (trap list,
below), so the automated visual gate did not work when we had one. The second
is that the operator boots the game and looks, which is strictly better and
costs nothing to maintain. **Do not build a replacement pixel gate.** What may
be gated about a frame is arithmetic — the mesh fits the volume the sim
blocks, in Rust, the shape of `crates/client/tests/tree.rs`.

**Agents are first-class on both sides.** They build it — `AGENTS.md` is
the door for any harness — and they will play it: the deterministic core
doubles as an RL training environment (a stated goal, `DECISIONS.md`
2026-08-01), which is one more reason the walls never bend. An agent player
pays the same doors and earns the same coins as a human.

## The docs, and which wins

| doc | owns | when in doubt |
|---|---|---|
| `DESIGN.md` | product, pillars, economy, architecture, milestones | the frame |
| `NETCODE.md` | everything multiplayer + transport config | **beats DESIGN §5** |
| `TERRAIN.md` | worldgen, slots, collision, terrain rendering | |
| `CONTENT.md` | every item/recipe/damage/loot number, as data schemas | numbers live here, never in code |
| `ALPHA.md` | the alpha cut, staged economy arming (A1→A2→A3) | |
| `BUSINESS.md` | what we sell: IAP, the entry price, and the one thing that stays out (an advantage over another player) | **product, not engineering** — read it when building the store, never otherwise. Nothing in `crates/` reads it |
| `ART.md` | the art bible: measured targets off the reference set, the hard visual rules, the review checklist | **the visual bar; the art rubric scores against it** |
| `DECISIONS.md` | dated operator calls; **the knob registry** | authoritative on every **(knob)** |
| `MENUS.md` | the interaction surface audit: every screen and verb, ours against the reference, measured off the two Rust mod loaders' hook tables | **owns nothing** — a survey to cut items from, never a queue |
| `RENDER.md` | the **native** client's render path: the Bevy-draws-not-decides boundary, the slice order, the native visual gate, the budgets | owns the path, never the bar — `ART.md` outranks it everywhere |
| `reference/SPAWN.md` | how the reference game places and respawns world objects: four systems, the placement-check chain, the convar layer, and **§9 what it means for us** | **owns nothing** — research, not law. Read it before building placement; `TERRAIN.md` §7/§8 is our answer to it |
| `reference/AUDIO.md` | how the reference game decides what a player hears: the Unity mixer groups/snapshots it built first, the `audio.framebudget 0.3` convar, localized ambience, the 2–5 kHz carve, its four shipped audio bugs, and **§9 what it means for us** | **owns nothing** — research, not law, and a *cleaner* source than `SPAWN.md`: devblogs and the public convar list, nothing decompiled. Our answer is `crates/client/src/sound/` |
| `reference/VOICE.md` | how the reference game does **proximity voice chat**: voice as a `ServerMgr` message rather than an entity RPC, the P2P origin that let players read each other's IP and DDoS them, the forced move onto the server (Devblog 189) and the voice props that move then unlocked, what Steam's voice API actually hands you (a codec and a microphone, **no transport**), the radius as a disclosure mechanic, and the three costs they have published — a talker-side hitch, a fan-out knee around 8–10 concurrent talkers, and moderation forever. **§9 what it means for us** | **owns nothing** — research, not law, `DOORS.md`'s proxy caveat in full (every fetch blocked here; tiers 2–4 are search summaries, and one of them was a decade stale until the in-tree hook table corrected it). Written because two live claims pointed at an unopened question, and it **retires one of them**: voice is not "its own transport" for us (§9.2 — a voice radius sits inside `AOI_ENTER_CM`, so the cull is one compare against a set the AOI scan already built). §9.1 is the half that is unfixable later — a client-side attenuation of a broadcast stream is a wallhack |
| `reference/BUILDING.md` | how the reference game decides **who may build here** *and* **what a shape costs**: the cupboard's authorized list (one pattern it reuses three times), privilege as a *volume emitted by the blocks* rather than a sphere around the cupboard, upkeep as a cost and the activity-keyed decay bug that caused it, the decay ladder, the demolish/rotate grace window, and — **§7b, 2026-08-10** — the 20 × 5 catalogue, twig as the editable draft, the socket separation between an opening and its insert, and the **four ratios** a hundred prices reduce to, with the half-wall's full price as the arbitrage they refuse on purpose. **§9 what it means for us** | **owns nothing** — research, not law, `DOORS.md`'s posture and its proxy caveat too, and §7b is tier 3 transcribed rather than fetched (§8 says so). Written because `DOORS.md` §9 kept pointing at it; §7b because the operator asked for the cost grammar next. **§9's last items are the live half** — the first ten landed 2026-08-08/09, §7b's five 2026-08-10, and **§7c (2026-08-21) is where a piece sits vertically**: the half-wall snap offset, and the three-metre gradient they tried for exactly our problem and reverted |
| `reference/DOORS.md` | how the reference game decides **who is allowed through a door**: the lock is a separate entity and a door with no lock is anyone's, the code lock's remembered list and its guest tier, the two goes it took them to rate-limit a keypad, knocking as a verb, and **§9 what it means for us** | **owns nothing** — research, not law, and `AUDIO.md`'s source posture with one caveat §0 states in full: every page fetch was blocked by this box's proxy, so the devblog and wiki tiers arrived as *search summaries*. The operator adopted it (`DECISIONS.md` 2026-08-08), so §9 is built: our answer is `crates/sim-core/src/lock.rs` |
| `reference/DURABILITY.md` | how the reference game **wears an item out**: condition as per-instance state on the item, worn by a float **keyed on (tool, resource)** so the wrong-tool penalty is *data* rather than a predicate (a metal hatchet pays 0.3 on a tree and 1.0 on flesh), the repair bench as a place you go with a blueprint check on the player, the **20% of *maximum* condition** every repair costs forever, and the two starting items that carry no condition at all so a naked spawn can always bootstrap. **§9 what it means for us** | **owns nothing** — research, not law, and the **strongest provenance in this directory**, which is worth saying because `MONUMENTS.md` holds the other end: §0's numbers are read off `wiki.facepunch.com/rust/item/<slug>` pages **fetched whole on 2026-08-15**, not search summaries, and it corrects `SOURCES.md`'s map twice — those pages *do* carry stat tables, with a per-resource Condition Loss column. Read it before touching `ItemStack`: §9.2 is why condition belongs **on** the stack and what wall 6 charges for putting it there |
| `reference/ARMOR.md` | how the reference game **dresses a player**: wearing is a **container move** — there is no equip RPC, only `CanWearItem(Item, slot)` beside `CanEquipItem` with one `MoveItem` under both — layering expressed as a *conflict* rather than an ordering, protection as a **vector keyed by damage type** (and radiation subtractive on a rate, where damage is proportional), coverage as the mechanic so an uncovered area takes full damage, the heavy set's 40% movement cost charged for *entering the category* and not per piece, and armor as hitpoints that break to **25%** rather than to zero. **§9 what it means for us** | **owns nothing** — research, not law, and `DOORS.md`'s proxy caveat in full (every fetch blocked; tiers 2–3 are search summaries), which is why §1 leads on the in-tree hook table. Written because `content/armor.toml` was priced, validated, hashed and balance-anchored since M1 while `grep -rn armor crates/sim-core` returned one comment — §9.1 is that audit, and **both halves of it are now closed**: reduction landed 2026-08-19 and the equip verb 2026-08-28. §9.2 was the load-bearing half and it is **built** (armor v1, wire v51): `CONT_WEAR = 4` reuses the move verb this repo has paid most dearly to harden, and it cost the `CONT_KIND_BITS` widening (wall 6) that §9.2 priced. §9.3's per-type vector and §9.4's condition are what remain |
| `reference/SAVES.md` | how the reference game remembers a player: **there is no player save file** — the body stays in the world as a sleeper and is saved because it is an entity, the save and the wire on one base class, the stop-the-world stall it never fixed, the wipe split, and **§9 what it means for us** | **owns nothing** — research, not law, and `AUDIO.md`'s clean source posture. The operator adopted its model (`DECISIONS.md` 2026-08-07), so §9 is a plan: read it before touching persistence. Our answer is `crates/server/src/store.rs` + `sim-core/src/persist.rs` |
| `reference/BALANCE.md` | which of our numbers are the reference game's and which are ours: what matched already, what moved on 2026-08-08, and **§4 what deliberately did not move** | **owns one thing, unlike the other `reference/*.md`** — §6's standing instruction, **rewritten 2026-08-10 and now a default rather than a permission**: take theirs, no case needed; a case is needed only to *differ*, and the only admissible one is a **mechanism** difference. Effort, a band, and source uncertainty are explicitly named as costs wearing principle's clothes (§6.2), a split source is broken by §6.3's ladder rather than deferred, and averaging stays forbidden. The bands in `CONTENT.md` §4 still decide whether it may land |
| `reference/RIPLIST.md` | the **queue** for ripping the reference's numbers: what is taken, what is outstanding, what is blocked on research nobody has done, what has no equivalent to take, and the six steps for executing one row | **owns nothing** — a worklist, and `BALANCE.md` §6 plus `CONTENT.md` §4's bands still decide. Read it before touching a balance number. Its §0 carries the **threat frame** (operator, 2026-08-09): their numbers are priced for contested farming and ours are not, so taking a yield without the interruption that balanced it is §4.1's false-familiarity trap one level out |
| `reference/NETWORK.md` | how the reference game does **netcode**, across twelve years of shipping it wrong in public: two networking libraries that each became the bug (a Lidgren stall first investigated as a DDoS, a RakNet fragment-reorder that needed entity checksums to even localize), the freeze that turned out to be serialization garbage rather than transport, the per-frame *time* budget for network processing, interest management sharpened from square grids to circles with per-entity radii to server-side occlusion, and the parallel-jobs arc that ended on one memory-pool lock | **owns nothing** — research, not law, `AUDIO.md`'s clean source posture with `DOORS.md`'s proxy caveat in full (every `rust.facepunch.com` fetch was blocked; tier 1 arrived as search summaries). **Its §9 is the exception to "owns nothing" in one respect**: unlike the other `reference/*.md`, §9 was written *against the tree* with a `file:line` on every claim, and §9.3 is the doc/code delta — including that **all seven of `NETCODE.md` §11's "Added CI gates" are unbuilt** |
| `reference/WATER.md` | how the reference game does water: the order it rebuilt the sea in (**surface → optics → motion → reflections → foam**, waves third), depth-based colour extinction and thickness-based visibility as one idea, shoreline wetness worked from the *land* side, what its own settings screen says water costs, and **§9 what it means for us** | **owns nothing** — research, not law, `AUDIO.md`'s clean source posture (devblogs and a settings screen, nothing decompiled). Our answer is `crates/client/src/render/water.rs` + `crates/client/src/sound/water.rs` |
| `reference/PROJECTILES.md` | how the reference game does **bows and arrows**: there is no `Bow` class (a bow is a one-round-magazine `BaseProjectile`, the same class as every gun), the projectile is **client-simulated** and audited by a thirteen-convar tolerance budget rather than a predicate, ballistics live on the **ammo** (`ItemModProjectile`) so one bow fires four different arrows, the arrow is an item three times over (~15 % break, 10 s lodge), hit detection takes the **most significant** body part not the first intersection, and **§9 what it means for us** | **owns nothing** — research, not law. Read it before touching `crates/sim-core/src/ranged.rs`. §9.1 is why our server-simulated arrow stays server-simulated; §9.3 is the one schema change it argues for (`[weapon.ballistic]` belongs on the ammo row), and it gets harder every arrow we add first |
| `reference/MONUMENTS.md` | how the reference game decides **where a large authored place goes**: placement as a solve rather than a guess (three rewrites in ten years, still moving), the collision list every worldgen system after monuments produced — rivers, cliffs, ice lakes, roads, ring roads, rails — terrain blending as authored per-monument masks rather than a flattened circle, the 2015 client-worldgen checksum mismatch they had to stop kicking for, vertical AOI layers, per-class interest ranges, what one moving monument actually costs, and **§9 what it means for us** | **owns nothing** — research, not law, and **the weakest provenance in this directory: §0 says so in full.** It is a summary of sources nobody here has opened (an operator briefing, 2026-08-10), so no number in it may reach `content/`. Its value is the ORDER, which is checkable against our tree. §9.2 is built (`SiteFootprint` / `site_sweep`); §9.3 is the real gap — our solver is two hand-written tiers |
| `reference/ANIMALS.md` | how survival games do animal mobs: the reference game's baked navmesh (100% CPU at boot) and the fixed think rate and dormancy it settled on, Valheim's ring spawners and two caps, Minecraft's mob cap and 1-in-800 despawn roll, and **§9 what it means for us** | **owns nothing** — research, not law, and `AUDIO.md`'s clean-source posture (devblogs, convar lists, wikis; nothing decompiled). The operator un-cut animals off it (`DECISIONS.md` 2026-08-08), so §9 is a landed design: read it before touching `crates/sim-core/src/mob.rs` |
| `WORLD.md` | the proposed register: **Gates as a threshold dimension** (the name is the fiction — it explains respawn, the wipe cadence and why banked JUNK leaves and carried JUNK does not), an ancient obsidian/lapis/gold civilization, the coast→interior gradient, the monument catalogue, **extraction** as a server-opened window at the haven's bank terminal, world states whose **default is broken** so a wipe cycle is a repair project, and an optional **ward** | **DESIGN — unspoken, none of it built, and a roadmap rather than a v1 spec** (operator, 2026-08-10: *"paths over time"*, and *"i think this is the max deviation"* — it is a ceiling, not a floor). Owns the fiction and nothing in `crates/`. Its §8 is the useful half: five collisions with live gates, two of them real — the visual rubric scores an obsidian world as a defect by construction, and a ward would invalidate `CONTENT.md` §4's TTK anchor *without reddening `test_content`*. §9.1 is the one piece of timing advice: **decide the register early, build it late**, because art made for the wrong register is remade |
| `reference/PLANTS.md` | how games grow a forest: the five-layer forest structure and which two of ours are empty, space colonization vs L-systems (and that we already ship the solver), Deussen's ecosystem sim as the reference for *placement*, octahedral impostors, and **§6 what it means for us** | **owns nothing** — research, not law, `AUDIO.md`'s clean source posture (a SIGGRAPH paper, four MIT repos, and our own dependency's source; `docs.rs` was proxy-blocked so the crate API came off `raw.githubusercontent.com`). Written because "are our trees good" has an arithmetic answer: **one species at three seeds, on a uniform 8 m scatter lattice that `ART.md` rule 7 forbids**. Read it before buying any foliage — §4 is why a mesh generator is the wrong tool — and before touching `render/tree.rs` or `terrain::scatter` |
| `reference/SOURCES.md` | the research **reading list**: which document settles which question, in priority order, with tiers 1–3 marked ANSWERED, tier 4 (the threat/logistics decomposition) half-closed 2026-08-14 — the violence paper is read at primary tier (`RIPLIST.md` §5.6, and it settles threat *shape*, not the magnitude; the session numbers stay open) — and **§3b the systems queue** (logistics by wipe stage, events, progression, clans, industry, moderation, trade) as the standing research worklist | **owns nothing** — a worklist for research the way `RIPLIST.md` is one for numbers. ⚠ Its §0 header is the load-bearing part and has been rewritten **in both directions**: reachability is a property of the container, not of the hosts, so *probe* rather than trusting either the "every Rust domain 403s" claim or the "they are open" one — both were honest measurements, on different boxes, days apart |
| `assets/models/WANTED.md` | the 3D object inventory: 63 meshes and 6 texture sets with sizes read off the code, the glTF/origin/ORM pipeline rules, and what is already covered | **owns nothing** — a sourcing worklist, `RIPLIST.md`'s shape. `MANIFEST.md` records what ships; this records what does not exist yet |
| `assets/textures/CANDIDATES.md` | the texture sourcing queue for the six foliage/bark sets: 84 candidate rows (80 CC0, 4 CC-BY) with licence, fetch mode and the measurement columns still empty, plus `fetch_gates_texture_candidates.py`, the csv/xlsx it reads and `CANDIDATES_CC_BY.md`'s draft notices | **owns nothing** — `WANTED.md`'s shape for pixels rather than meshes, and the measurement still decides (`ART.md` §7's estimator, never the fetcher's). The 1.3 GB it downloads is gitignored; `assets/textures/MANIFEST.md` records what ships |
| `assets/sound/WANTED.md` | the SFX inventory: every cue the client plays with its length, character and delivery spec, the ElevenLabs prompt sheet, the open-source candidates with their licences, and the score's composer brief | **owns nothing** — `WANTED.md`'s shape for audio. The enum (`sound/mod.rs::Cue`) is the authority and the bank stays generated (`sound/synth.rs`) until a file lands with a manifest row; `DECISIONS.md` 2026-08-11 (ElevenLabs, paid plan) and 2026-08-07 (CC0/CC-BY, NC/SA refused) are the rail |
| `assets/models/MANIFEST.md` | what **ships** in `assets/models/`: vendor, mode, prompt, task id and date per mesh, the KTX2/UASTC-at-1024 texture rule and its VRAM reason, and what the client actually loads | **owns the licence rail's audit trail**, which is the one thing here that is not just a note: `DECISIONS.md` 2026-08-07 is CC0 preferred, CC-BY with a `NOTICE` entry, **NC and SA refused** because the game is sold. Recording the provenance per file is what makes that rail auditable after the fact rather than a promise. `WANTED.md` is the inverse — what does not exist yet |
| `PLAYERS.md` | the agent player: the verb set, the observation encoder, and the four walls that keep agent play measurable | **DESIGN — none of it built.** The research half is elo's `SUBSTRATE.md`; this owns only what an agent may do here |
| `BRANCH-NOTES.md` | a **transient** handoff note, written on a branch by the loop's builder when it lands a partial slice (`gates-loop/GOAL.md` §the partial rule) — what landed, what is measured, what remains | **owns nothing and is not a queue.** It describes whatever branch wrote it last, so read the heading before trusting a word of it; the current one says it carries no handoff. Not deleted because the loop is paused, not retired, and its builder recreates this file by protocol |
| `NOW.md` | what next | **the only list that answers that** |

Docs are dated notes, not law. Four things actually bind: the walls below,
the gates in CI, the operator's spoken decisions, and measurements. A doc
that disagrees with a passing gate is wrong — fix the doc.

## The walls (each with its enforcement — a law without a gate is a mood)

Seven, and every one of them has a gate you can run. **Monetization is not
here.** It used to be wall 8 and it had no enforcement, in a list whose own
header says a law without a gate is a mood — so it was product policy wearing
engineering clothes, and it cost context on every pass that never touched
money. It lives in `BUSINESS.md` now: read that when you are working on the
store, and not otherwise.

⚠ **One enforcement named below is still aspirational and is marked so:
there is no soak.** Wall 3 holds on clippy alone, so it is not ungated, but
the list overstated itself and that is the exact failure the header warns
about. Grep before citing a gate. **Wall 4's half is no longer missing —
`crates/sim-core/tests/raid_storm.rs` landed 2026-08-14** and this paragraph
said it did not exist until 2026-08-15; `DESIGN.md` §12 had already been
corrected, so the two docs disagreed and the pessimistic one was the stale
one. That direction of error is the cheaper one and it is still an error:
`ls` the file, do not trust either doc's memory of it.

1. **sim-core is pure.** No I/O, no clock, no threads, no `HashMap`/
   `HashSet` iteration, no libm/trig, floats restricted to
   `+ − × ÷ sqrt min max clamp floor-by-cast`. → clippy disallowed
   types/methods + `test_parity_wasm` (native and wasm bit-identical).
2. **Zero allocation in the tick after warmup.** → counting allocator,
   `test_alloc_zero`.
3. **No locks, no syscalls, no `String`/`format!`/logging in the sim
   thread.** Rings only; integer event codes only. → clippy walls
   (`sim-core/clippy.toml` disallows the lock, clock, I/O and `String` types
   by name, and `ci/gates.sh` runs clippy `-D warnings`). ⚠ **The soak
   tick-jitter assert this line also named does not exist.** `bots.rs` can
   *drive* a 100-bot soak and `NOW.md` §0 asks for one, but nothing asserts
   tick jitter, so the wall holds on clippy alone. (Re-checked 2026-08-11:
   `grep -rn soak crates/` returns wet-surface shading and references to the
   soak that is still wanted — no assert.)
4. **Bounded everything.** Every queue, map, and per-tick work item has a
   cap in `limits.rs` and a stated overflow policy. No `push` on a
   client-driven path without a cap check. → the review wall, plus
   per-site cap tests across ~40 suites (`the_queue_is_bounded_and_says_so`,
   `event_ring_overflow_heals_by_resync`, `the_bag_cap_stays_neutral`,
   `the_autosave_sweep_is_bounded_and_skips_the_unchanged`,
   `the_voice_cap_refuses_rather_than_steals`), **plus `test_raid_storm`
   (`crates/sim-core/tests/raid_storm.rs`, landed 2026-08-14)**, which is
   the one that drives them all at once: 64 synthetic players through
   build/lock/plant/guess/move/loot at the tick's command ceiling. This line
   said that gate did not exist for a day after it did.
5. **Determinism is a gate, not a vibe.** Same build + seed + WAL →
   same state hashes. → `test_replay`, `test_terrain_golden`.
6. **The wire never drifts by accident.** Packet layouts change only with
   a version bump + regenerated goldens in the same commit. →
   `test_protocol_golden`. ⚠ **`PROTO_VER` is one of three version numbers
   and only this wall's** — `crates/protocol/src/version.rs` carries the
   table, and conflating any two of them is what that file exists to prevent.
   The release (`VER`, from `[workspace.package] version`, shown in the
   client's corner and floored per-shard by `min_client`) and the commit
   (`GIT_SHA`) are not this wall's business and do not bump goldens.
7. **Content never touches code.** New items, recipes, balance passes =
   `content/*.toml` only, validated at boot, content hash pinned into the
   WAL header (a replay replays the content it was played under). →
   `test_content`.
## Traps already paid for (learned from research or elo production —
do not rediscover)

- **wtransport must be pinned ≥ commit `0f7609a`** (or a release
  containing it) — 0.7.1 has a two-byte remote panic.
- A browser datagram write over `maxDatagramSize` **silently succeeds and
  sends nothing** — clamp every send against the live value. Browser-only as
  a *silent* failure; the native client speaks wtransport directly and the
  MTU ceiling is still real, so the clamp stays on both paths.
- `send_datagram()` (drop-oldest), never `send_datagram_wait()` — a
  congestion stall must cost freshness, not latency.
- **Quantize both sides** or prediction drifts by rounding: the server
  sims on the values it transmits.
- The client is also a hot path, and **half of this trap was JavaScript's,
  half is not.** Retired with the browser: closures in the RAF loop,
  typed-array parsing, and GC pauses — Rust has no collector, so the pause
  class the browser client had to design around does not exist natively.
  Still true on both: **no per-frame allocations** (a native frame can still
  stall on an allocator under a chunk build), and the general shape — a
  client-side hitch feels identical to a server blip to the player, so the
  client is held to the same discipline as the sim thread even though it is
  not the sim.
- Stream-in AND stream-out are budgeted per frame on the client — the
  teardown spike is the half everyone forgets.
- A suite that skips on a missing dep must say SKIP loudly and exit
  nonzero in CI — a pass it didn't earn is the worst bug class.
- Never start a line of a commit body with `Operator, YYYY-MM-DD:` unless
  the same commit updates `DECISIONS.md`.
- **Median fps hides shader-compile stalls, and natively the symptom is not
  a stall.** A static benchmark can read 90+ fps while lazy WebGL program
  links cost 700 ms+ worst-frames in real play. The MECHANISM survives the
  port — Bevy specializes a pipeline lazily, on the first frame a material is
  actually drawn — but this entry said the native version was *"a bigger
  stall"* and that was wrong, checked 2026-08-20:
  `RenderPlugin::synchronous_pipeline_compilation` defaults to **false**, so
  the pipeline is built on a task pool and a draw whose pipeline is not ready
  is SKIPPED rather than waited for. The native failure is therefore a **pop**
  — a tracer, a bullet mark or another player's body arriving a few frames
  after the thing that caused it — and looking for a hitch is looking for the
  wrong shape.
  ✅ **Mostly closed, two ways.** The loading screen draws the world as it
  streams in (`loading.rs`, and `world_running` includes `Screen::Loading`),
  so terrain, water, clutter, trees and every scatter prop specialize while
  the bar fills; `decal.rs` hand-rolled the same idea for its own material;
  and `render/prewarm.rs` now does it for **every** `StandardMaterial`, off
  `AssetEvent::Added` rather than a list, because a hand-kept list of
  materials-that-need-warming is the drift this file warns about twice
  elsewhere. What is still open is named in that module: **skinned meshes are
  a different pipeline key and nothing warms them**, and a driver may still
  finalize on first real use. The measure remains a COUNT (`PipelineCache::
  pipelines()` is public) and it still has no gate, because that resource
  lives in the render world and needs a GPU. `RENDER.md` §2 carries the
  design; `crates/client/tests/prewarm.rs` gates what reaches the ECS.
- **A clean merge is not a correct merge, and a destructive read is where
  that bites.** `ClientCore`'s own-fact rings (`pop_hit`, `pop_toast`, the
  refusals) hand each fact over exactly once. On 2026-08-06 two lanes each
  added a reader — the HUD's toast/hitmarker surface and the audio mixer —
  touching no common line, so **git merged them without a conflict and the
  result was silently broken**: the earlier-scheduled system drained every
  ring and the game made no sound for a hit, a gather, a craft or a refusal.
  Nothing failed, because each half is correct alone. The shape that cannot
  regress is **one drain at a fixed point in the frame into a resource readers
  borrow immutably** (`render/feed.rs`) — a second reader is then a `Res<_>`
  parameter, which cannot consume anything. The general rule: when two lanes
  are open, a queue with a single-consumer contract needs an owner named in
  code, not in a comment, and the gate for it is a grep for the call site
  (`tests/sound.rs`), because the defect is a call site and not a value.
  ⚠ **That grep's verb list was HAND-KEPT and had drifted** — nine names
  against fourteen destructive rings when it was checked 2026-08-15, with the
  five newest (`pop_knock`, `pop_auth`, `pop_shot`, `pop_research_toast`,
  `pop_research_refusal`) unwatched, so the rule was enforced on the rings
  nobody was about to add a second reader to and silent on the ones they
  were. It is derived from `client-core/src/core.rs` now, `pop_chat` exempt
  by name, and a ring the scrape cannot classify is a loud failure rather
  than a skip. Same shape as the `grep -rn 'props\.js'` line at the top of
  this file: **a hand-kept mirror of another crate's surface goes stale, so
  read the surface — the command is the claim, not the number.**
- **A type shared across the feature line has its exhaustive matches on
  only one side of it.** `ui::interact::Verb` compiles in both builds;
  every `match` that must cover it lives in `render/verbs.rs`, which
  `--features render` gates. So adding a variant is **green on
  `cargo test --workspace`, twice, and red at the Bevy gate** — the one
  that costs five minutes to reach. Found 2026-08-10 adding
  `Verb::Recycler`: the sim, the wire, the content and every non-render
  client test passed while `E` and `C` had no arm for the thing they were
  pointed at. Same shape for any `pub enum` or archetype table read from
  `render::`. When you add a variant, grep the feature-gated modules for
  its type before believing a green workspace run.
- **Two of the same component in one Bevy bundle is a RUNTIME panic, and
  every gate in this repo is blind to it.** `(DeathRoot, ui::screen(bg),
  Node { padding })` is the obvious way to take a shared layout and move one
  field, and it does not merge and does not replace: Bevy 0.18 dies at spawn
  with *"has duplicate components"*, inside a command queue, naming the
  system as `<Enable the debug feature to see the name>`. Written that way
  on 2026-08-16; `cargo build`, `cargo clippy --features render` and
  `./ci/gates.sh` were all green, and the client died the instant a body
  reached the death screen — a screen no headless test spawns and no capture
  probe visits. **Booting the game is what found it**, which is the whole of
  why there is no pixel gate and why a person looking is the visual gate.
  The composable shape is a `fn` returning the `Node`
  (`ui::screen_node()`), spread into a fresh one; a `const Node` does not
  compile, because `Node` holds types with destructors. The general rule:
  **a spawn is not type-checked for duplicates, so a bundle assembled out of
  two helpers is a claim you have to run.**
- **A sweep window that agrees with itself across every case is still not
  validated, and this one nearly wiped a live shard.** On 2026-08-14 a pass
  measured 40 islands and concluded the shipped seed was the flattest of them —
  46.32 m, max slope 0.890, granite on 0.00% of its land — which read as
  airtight: 40 seeds, one method, a clean ranking, a gate written to hold it.
  Every sweep ran `-1024..1024` on both axes. `terrain::continent` centres the
  island on `(ISLAND_SIZE/2, ISLAND_SIZE/2)`, so world coordinates run 0..2048
  and that square's **corner** is the island's centre: it sampled one quadrant,
  632 k m² of a 2.9 M m² island, and not the quadrant the capture camera stands
  in. Whole-island, the same seed is 106.00 m / 2.665 / 10.0% — **upper third of
  44**. The comparison was sound because the bug was uniform, which is exactly
  what made it invisible: consistency across cases proves the *method* is
  constant, never that it is aimed at the right thing. What would have caught it
  is one number checked against a second source — the land-sample count implied
  a quarter-disc all along, and `World::spawn_pos` starts from `c = ISLAND_SIZE
  * 0.5` twelve lines from the sweep. **Cross-check a sweep's window against
  something that already knows where the world is**, before the finding earns a
  gate and a doc paragraph. `sim-core/tests/relief.rs` holds the retraction and
  a gate on the window itself.
- **A "naive rebuild" that calls the function under test is a rebuild of
  nothing, and the gate is green for the wrong reason.** The house pattern for
  an optimization is a second implementation of the law compared field by field
  on `to_bits()` — `client/tests/ground.rs` does it for the mesh. On 2026-08-19
  `sim-core/tests/lattice.rs` did it for `clutter_fill` and its naive side
  called `terrain::clutter_rich_cell` for the per-cell law, so **both sides
  carried the mutant**: moving the new early-out's threshold by one — refusing
  a cell whose roll is `RICH_ACCEPT_MAX - 1` over ground rich enough to accept
  it, which is 36% of land at 1-in-256 rolls, so hundreds of cells in the swept
  block — passed all ten assertions in the file. The fix is to rebuild the law
  from PUBLISHED parts (`clutter_rich_draw`, `clutter_kind_at`,
  `clutter_richness_at` are `pub` for this and for nothing else), and the
  published part returns a **named struct** rather than a tuple of bytes, so
  the gate reads the bit layout instead of re-deriving it — the
  positional-payload trap two entries down, in a test. The general rule: after
  writing a gate for an optimization, **run the mutant**. Ours found two
  worthless assertions out of ten. A test that shares any code path with what
  it is checking is checking that path against itself, and "I compared the fast
  path with the slow path" is only evidence when the slow path is *yours*.
  ⚠ **And gate the optimization's EFFECT, not only its correctness** — the
  same day's second surprise, from `client/tests/water_carry.rs`. The sea now
  carries its last sweep across a `SNAP_M` crossing instead of re-tapping the
  ground; the suite walks the grid and compares it against a freshly built
  one, which is the right correctness test and is **satisfied by never
  carrying anything**. A mutant that derives the index shift wrong makes every
  index fail the guard, so the sweep rebuilds — correct output, no saving, ten
  green tests. A safety check that turns a bug into a fallback makes that bug
  invisible to every assertion about values, and the optimization can then be
  deleted by accident with the gate still green. The fix is one observable
  count (`Sea::carried`) asserted as a floor on a one-cell snap and as zero on
  a diagonal — a count of VERTICES, not a time, so the no-clock rule is
  untouched.
- **A cheap counter is not a free counter, and one was left in the tree.**
  `crates/sim-core/src/perfcount.rs` sat untracked in this branch's working
  tree — its own first line reading *"TEMPORARY measurement scaffold (not for
  merge)"* — putting a shared `static AtomicU64` `fetch_add` inside
  `terrain::height`, `noise2` AND `cell_hash`. One `height` call is 61
  contended atomic RMWs on two cache lines: measured, **1.85 ms of a 6.10 ms
  `water::stream` sweep was the instrument**, ~30% on every terrain timing in
  two crates. The first baseline of the 2026-08-19 perf pass was taken on it
  and read `clutter_fill` at 2.912 ms against a true 2.870 — close enough to
  look right and not the same measurement. **`git status` before quoting a
  timing**, and if a counter has to exist, put it behind a cargo feature so the
  default build has no atomic in the hot path.
- **A shaping curve interpolated with `lerp` is a contour map, and no gate in
  this repo could see it.** `terrain::remap` ran the height field through 17
  LUT knots with `lerp` between them from the first commit to 2026-08-26. A
  piecewise-linear curve is C⁰ and not C¹ — its *slope* steps at every knot,
  here by **8× at knot 7 and 12× at knot 12** — and `render/terrain_mesh.rs`
  takes its normal analytically from that field's gradient, on purpose, so the
  triangulation never shades. A slope step is therefore a normal step, and a
  normal step along a set of constant elevation is a **survey contour drawn on
  the mountain in shading**. Sixteen knots, sixteen rings, nested around every
  hill; the same thing happens at a `clamp`'s two rails and at any linear ramp
  keyed on height (the ridged blend's gate put two more at 52 m and 80 m).
  **Every gate was green and every one of them had to be**: the golden pinned
  the values the curve produced, the replay reproduced them, `test_content` had
  no opinion, and clippy sees a `lerp`. The defect is not a wrong value
  anywhere — it is a *derivative* being discontinuous, and nothing in this tree
  asserted on a derivative. It was found by drawing it
  (`sim-core/examples/hillshade` renders |∇‖∇h‖|; the island came out as a topo
  map) after the operator pointed at a screenshot.
  Two things to carry. **First: a shading defect can live entirely in the sim
  crate.** The renderer was correct; worldgen handed it a creased surface.
  Anything that reaches the frame through an analytic normal — terrain, water,
  any signed distance — is held to C¹, and a LUT, a `clamp`, a `min`/`max`
  ramp and a `t.clamp(0,1)` gate are all C⁰ by default. **Second: the obvious
  gate does not work and was thrown away rather than shipped** — sweeping the
  island and binning curvature by elevation reads 3.58–4.65× the median before
  the fix and 1.54–3.52× after, overlapping, because it cannot tell a crease
  from the cliffs the LUT exists to create. `tests/contour.rs` gates the
  *mechanism* (the curve is C¹ at every knot and at the clamp) instead, which
  is exact, runs in microseconds, and is proven red under the old body.

- **A statistic about an asset is not a number until you say how it was read,
  and the DECODER is part of that.** `assets/textures/MANIFEST.md` and two doc
  tables in `crates/client/src/render/` describe the shipped `.jpg`s — means,
  luma, sd, gain span — and on 2026-08-28 all three were wrong somewhere while
  every constant beside them was right, because `GRAIN_GAIN`, `ROUGH_MEAN` and
  `GRAIN_SHARE` each have a test that re-measures them off the file and the
  prose had nothing. The 2026-08-27 `rock` swap (`Rock023` → `Gravel004`) moved
  exactly the things with a gate pointing at them and left the old texture's
  numbers in **four** places plus a dead `pub ROCK_GAIN` nothing read. Two
  things to carry beyond "gate the prose". **First: the basis is half the
  claim** — one `rock_albedo.jpg` reads an sd of 0.1379 at 1024², 0.1287 at
  512² and 0.1131 at 256², so the file's own candidate table and its prop-bind
  table differ legitimately and neither may be "corrected" into the other.
  **Second: two JPEG decoders of one file disagree by more than the digits
  these tables print** — the first round of corrections here was measured with
  Pillow and `image` 0.25 put five more cells out of range (`bark`'s span
  2.000 → 1.995, `litter`'s 3.586 → 3.559, worst case 0.45%). Neither is wrong;
  the one that matters is **the decoder the game ships**, because Bevy reads
  these files through `image` and the frame is what the number describes. The
  gate is `crates/client/tests/manifest_measured.rs`, its bound is half an ulp
  of whatever precision the text printed (a full ulp let `47.6%` → `47.5%`
  pass — found by running the mutants, six of seven caught and the bound was
  the reason), and it prints the whole table as measured so nobody re-derives
  one by hand again.
- **A judge names the symptom; fix the cause.** Optimizing the judge's
  literal sentence is how a loop circles for three passes — elsewhere,
  "untextured" was really diffuse contrast crushed by an earlier fix for
  "too bright", and the correct change was the opposite of the feedback's
  direction. Diagnose the mechanism before acting on a ranked gap.
- **A pixel statistic cannot see whether the frame is a picture of
  anything, and ours proved it.** On 2026-08-05 `ci/vantages.mjs` passed
  all 36 checks on a beige smear with no sky, no horizon and no object in
  it — scoring the *highest* detail of four vantages (14.28 luma/px). Every
  assertion in that gate is contrast, chroma or luma neutrality, and a
  featureless wash satisfies all three. The same day's captures showed the
  real gap was **content density** — grass, understory, branches, props —
  against which no amount of surface field scores a point.
  **This is why there is no visual gate and why you must not write one**
  (operator, 2026-08-06). The conclusion drawn at the time was "add a
  structural assertion before the statistic"; the conclusion that actually
  held is that the whole approach was spending passes to avoid opening the
  game. A person looking at the frame is the visual gate, it is cheap, and it
  cannot be satisfied by a beige smear. Look at the picture instead of tuning
  the number — not before tuning the number.
- **A byte-golden is blind to what a field means.** Positional payloads
  are where the reference ecosystem actually bled: 49 of Oxide.Rust's
  commits touch a hook's arguments and ~27 correct a payload that had
  already shipped wrong — the right value in the wrong position, four
  hooks corrected more than once. Their patcher pinned an `MSILHash` per
  patched method, the exact analogue of our `test_protocol_golden`, and it
  caught none of them. Ours has the same hole: swap `a` and `b` at an
  `events.push` site and the encoder is untouched (golden green), the
  event queue is not in `state_hash` (replay green), and every field is
  `u32` (clippy green). `reference/FINDINGS.md` §1 had the shape of the
  gate that catches it, and it exists now: `crates/sim-core/tests/
  event_roles.rs` role-checks every event's payload against its
  `/// EV_*: a = … b = …` doc line — 32 of 32 as of 2026-08-09, each
  proven red under its own a/b swap. The trap stays listed because the
  mechanism (three green gates over a wrong payload) is what to remember
  when the NEXT event lands: an event without its role gate re-opens it.
- **The item-move verb is the most bug-prone thing in the reference, and
  it fails as a kick.** Three Oxide fixes in 28 minutes on one 2019 day —
  the third titled as a fix of the fix — all one-line splice-point moves
  on move/stack/loot, all landing as *the server disconnecting the
  client*, because container state diverged and that reads as a forged
  request. The bug is validation ordering against the mutation, never
  arithmetic. Prediction makes it worse for us: the client has already
  drawn the move, so a container refusal must be computed on the same
  values the client predicted with (the quantize-both-sides law, applied
  to containers).
- **A full-world save is a stop-the-world freeze, and the reference game has
  not fixed it in thirteen years.** Every server-host guide says the same two
  things: saving is heavy, and on a high-population shard it "causes a massive
  lag spike that freezes everyone in place". The only mitigation offered
  anywhere by anyone is `server.saveinterval` (default 600 s) — a knob whose
  two ends are *how much a crash costs* and *how often everyone stalls*, with
  no third option. What they did optimise is the other end, load: Devblog 96
  put building stability into the save so a restart could skip recomputing it.
  So the trap is not "saving is slow", it is that **a whole-world walk on the
  sim thread has no good cadence**, and picking one is choosing which player to
  disappoint. Ours is a bounded sweep instead — one player per tick,
  skip-if-unchanged, on a thread that is not the sim (`store.rs`) — and the
  world stores want the same treatment when they land, not a snapshot. Their
  loader also trusts the file outright; ours cannot, because a save is the one
  non-command path into `World`. `reference/SAVES.md` §4 and §9.3 have the
  measurements and the sources.
- **A big fixed array constructed on the stack can blow wasm's shadow
  stack, and the native build will not tell you.** `Box::new(Store::new())`
  materialises the whole struct in a frame before moving it to the heap.
  `World::new` already builds ~100 player records, a 1 024-piece store and a
  1 024-deploy store that way; on 2026-08-08 a 52 KB lock store on top of it
  turned `test_parity_wasm` into `RuntimeError: memory access out of bounds`
  inside `Deploys::new`, with every native test green — **wall 1's own gate
  failing for a reason with nothing to do with determinism**. wasm32's
  shadow stack is 1 MiB and there is no guard page, so the symptom is an
  out-of-bounds read and not a stack-overflow message. The fix is to fill on
  the heap (`vec![..; N].into_boxed_slice().try_into()`), which allocates
  where the old code allocated anyway. The stores that predate this still
  use the stack form; they fit today, and the next one to be added will not.
  **Seen three times in one day** — the lock store found it, the hearth crew
  hit it again, and the third surfaced *inside dlmalloc* rather than in the
  constructor, because the frame that tipped it over was the allocator's own.
  `crate::boxed_array` is the fix and the arrays in `Pieces` and `Deploys`
  all use it now; the piece store alone was 98 KB in a frame.
- **Tonemap, sky, exposure, and fog are one owner.** Split across parallel
  passes they break each other's assumptions faster than they improve
  (measured elsewhere: three parallel rounds worsened visual defects
  60→66; one sequential owner over the coupled set cut them to 26). The
  lighting gap, when attacked, is a single iteration's single ownership.
- **A depot build id has no platform in it, and elo's origin keys by build id
  alone.** `ci/depot.py`'s `build_id()` is `<version>-g<sha>` (plus a
  content-keyed `-dirty` marker); the origin stores
  `$SCRY_DEPOTS_DIR/<slug>/<build>/depot.json` and `published.json` maps
  *platform → build id*. So **two platforms packaged from one commit are one
  directory**, holding one `depot.json`, which carries a single `platform` and
  `launch.exec` — and both platform rows resolve to it. A linux player would be
  handed the Windows depot, silently. `depot.py` even prints the same rsync
  destination for both. It has never fired only because the two builds
  published 2026-08-10 happened to be cut at different commits, which is luck
  wearing a design's clothes. **Do not fix it with a platform suffix on the
  id**: elo's `meter/gamerepo.py::version_of` strips `-g[0-9a-f]+…$` anchored
  at `$`, so `0.2.0-g<sha>-win-x86_64` stops parsing back to `0.2.0` and the
  store row's declared-vs-published gap goes permanently red; a fake-hex sha
  parses and lies about the commit.
  ✅ **FIXED 2026-08-14 — the origin keys by (build, platform) now**
  (`scry-forge` c5eb47c7, `meter/launcher.py`). A depot lives at
  `<slug>/<build>/<platform>/`, `ci/depot.py` bakes `{platform}` into `root`,
  and both platforms of one commit are published routinely — `0.2.0-gbed9e02d6`
  is live for linux and win as two depots with two digests. The workaround
  (adjacent shas) is retired; the entry stays because two things outlive it.
  **First: the old layout is permanent, not migrated.** `root` is inside the
  digest and the digest is sealed in `ScryNotary`, so a published depot's
  location is part of what was notarized and moving one would orphan the number
  on chain — the origin serves both shapes forever, and `publish_depot.py`
  refuses to write a keyed directory beside a legacy one. **Second: the suffix
  is still not the fix**, for the `version_of` reason above, and that is the
  part someone will re-propose.
- **A packager that walks the filesystem ships whatever is lying on the build
  box, and no gate in this repo can see it.** `ci/depot.py` staged assets with
  `shutil.copytree(ROOT/"assets", …)` until 2026-08-14, when a routine
  republish produced a **1.6 GB depot** where the live one is 124 MB — 317
  files against 122. It had swept in `assets/textures/candidates/`, the 1.3 GB
  sourcing queue that 36776f4 deliberately keeps *out* of the tree, 190 raw
  CC0/CC-BY archives. **Every gate was green**: `ci/gates.sh`, `depot.py
  --self-test`'s 51 checks, the document validation. None of them could have
  failed, because an untracked file is invisible to all of them — the defect
  is not a wrong value anywhere, it is a *set* being wider than anyone
  declared. Two things it would have cost: a 13× install for every player, and
  a route around the licence rail (`assets/models/MANIFEST.md`), since the
  candidates are unvetted by construction — `CANDIDATES.md` carries CC-BY rows
  with draft notices. The fix is structural rather than a check: stage from
  `git ls-files` and refuse to fall back, so `.gitignore` stays the single
  author of what is not ours to ship and the packager cannot disagree with it.
  elo's `deploy/publish_scryward.py` chose `git archive` over a walk for this
  exact reason and says it has paid for it seven times; we now have our own.
  **The general shape: a build step that enumerates by walking is only as
  correct as the tidiness of the box it runs on, which is not a property
  anything asserts.**
- **Measuring what a build needs is not meeting it, and the depot shipped a
  Windows game nobody could start.** `ci/depot.py` was written under a Linux
  rule stated in its own docstring — bundle nothing, the machine provides it —
  which is right for libwayland and libasound and wrong for
  **`libstdc++-6.dll`**: mingw's C++ runtime, reached through
  `basis-universal-sys`, and on approximately zero stock Windows machines. The
  measurement was never the gap. `0.2.0-gbed9e02d6`'s published
  `requires.libs` **named `libstdc++-6.dll`**, correctly, sitting among two
  dozen genuine system DLLs where nothing distinguished it — and the staged
  tree held three files. Every check passed because every check was about the
  document: the packager hashed it, `--self-test`'s 51 checks read it, the
  launcher fetched and verified it, and the player got an Application Error box
  (`0xc000007b`) before a frame. Reported 2026-08-16 by the one person who ran
  it. Two things to carry: **a needs-list is not a fix**, and *"the platform
  provides its system libraries"* is a claim about a platform, so it must be
  re-asked per platform rather than inherited. The fix is `runtime_dlls`, and
  its shape is this repo's usual one — **ask, don't type**: `x86_64-w64-mingw32-gcc
  -print-file-name=X` returns a path for a DLL the toolchain owns and echoes
  the bare name back for one Windows owns, so the sort needs no maintained
  list and resolves through the *selected* mingw alternative, which is the
  threading-model trap below arriving at package time. **Transitively**, which
  is the half that bites: `gates.exe` imports only `libstdc++-6.dll`, and
  *that* imports `libgcc_s_seh-1.dll` and `libwinpthread-1.dll`, so a direct
  read of the exe ships a third of the runtime and fails identically.
  The gate that closes it is not another document check — `nightly.yml`'s
  windows leg **runs the staged build under wine** (`gates.exe --help`, ~7 s
  from a cold prefix, no window and no GPU), because every check that existed
  read the depot and the depot was *correct*. Measured both ways: with the
  runtime staged it exits 0, and with the three DLLs moved aside it dies as
  `err:module:import_dll ... not found` / `loader_init ... failed, status
  c0000135`. That is the same failure the player saw with a different code —
  **absent is `c0000135`, present-but-32-bit is `c000007b`** — so a machine
  with no copy at all fails identically and the distinction is only about
  what junk is already on the box.
  ⚠ **The code fixed here is the packager, not the depot on the origin.**
  Republishing is an operator act, so until it happens the live Windows build
  is still the broken one (`NOW.md` §0win). And `0xc000007b` specifically —
  rather than a missing-DLL message — means that box *had* a 32-bit
  `libstdc++-6.dll` on its search path; shipping ours beside the exe fixes
  both cases, because the application directory outranks System32 and PATH.
- **`x86_64-pc-windows-gnu` needs mingw's POSIX threading, and half a switch
  is worse than none.** Ubuntu's default alternative is `13-win32`, whose
  `libgcc.a(gthr-win32.o)` has no `__mingwthr_key_dtor` — the client fails at
  link with every gate green. Pointing only `CARGO_TARGET_..._LINKER` at
  `-posix` moves the error rather than fixing it: the C++ deps
  (`basis_universal_sys`, via `cc` calling `x86_64-w64-mingw32-g++`) are still
  compiled win32 and die on `__gthr_win32_mutex_*`. Both halves must agree —
  `update-alternatives --set` for **gcc and g++** — and `target/
  x86_64-pc-windows-gnu` must be deleted, because the C++ objects are cached
  per threading model. Set to `manual` on this box 2026-08-13.

## The loop discipline

- An iteration = pick from `NOW.md` → branch → build → **all gates green
  locally** → merge. A change that reddens a wall does not merge, ever.
- **Knobs are spoken, never invented.** Every tunable is either in
  `DECISIONS.md` as spoken, or carries its documented default. Inventing
  a number = writing it into `DECISIONS.md` §open, not into code.
- **Operator-only acts** (a loop proposes, never performs): arming A2/A3,
  anything on-chain, publishing the page or the link, deploying to the
  public shard, cert/domain changes, wipes of a live shard, admin bans.
- Parallel loops: one owner per crate per iteration; `protocol` and
  `limits.rs` changes never land from two branches in one merge window.
- When the operator's word conflicts with any doc including this one, the
  word wins; record it in `DECISIONS.md` the same day.
- **A partial slice, landed honestly, is a good iteration** — not a fallback.
  Land the coherent piece with the gates green, say in `NOW.md` what remains
  and what you learned about why, and stop there. The walls above are hard
  because they are gated and objectively checkable; nothing in them asks you
  to finish an item you cannot stand behind.
- **Prose is bounded: a `NOW.md` item ≤ ~25 lines, a commit body ≤ ~20.**
  `NOW.md` is a work queue — the next iteration reads every item before it can
  pick one, so an essay about a single constant is a tax on every pass after
  it. Detail worth keeping goes to `DECISIONS.md` §open (a knob) or a
  `gates-loop/findings/` note (a measurement), and the item points at it in one
  line. Length is not evidence of rigour and no gate or rubric scores it. This
  binds what you write; the items already over it are not a cleanup task, so
  trim one when you edit it anyway and leave the rest.

## Commands (derive, don't quote)

```
cargo test --workspace              # every gate that runs headless
cargo run -p server --bin shard     # the server (reads shard.toml)
cargo run -p server --bin bots -- 100
cargo run --release -p server --bin profile          # where a tick's 33 ms goes
cargo run -p client --features render --bin gates    # the game
cargo clippy -p client --features render --all-targets -- -D warnings
./ci/gates.sh                       # exactly what CI runs — run it before merge
```

**Cutting a release is a tag, and it is an operator act.** Bump
`[workspace.package] version` in the root `Cargo.toml` — the one place a
version is typed, inherited by all six crates — then
`git tag -a v<x.y.z> -m "…" && git push --no-verify origin v<x.y.z>`.
`.github/workflows/release.yml` re-runs the gates on the tagged commit,
refuses a tag that disagrees with the tree, builds Linux/Windows/macOS, and
leaves a **draft** release for a person to read and publish. A shard's
`min_client` floor is raised *after* that release is published, never before.

`--features render` is off by default and everything about the client is
behind it (`crates/client/Cargo.toml` says why). It needs `libwayland-dev`
and `libasound2-dev` on a fresh box — Bevy's default features ask for them
through `winit` and `bevy_audio` and this client uses neither, which is a
trim that is owed (`NOW.md` §0x item 4).

## The loop that builds this repo

⚠ **Not retired, and this line does not know whether it is running.** That is
deliberate now: every previous version of this paragraph asserted a state, and
each was wrong within days — a dated claim about a live process is the shape
`CLAUDE.md` warns about everywhere else. **The check is `loop-status.sh` then
`ls .../STOP`, in that order.** It has run in bursts (21 passes 2026-08-13/14,
11 on 2026-08-15, 8 on 2026-08-28) with long dark stretches between; while it is
dark, `NOW.md` is the steering.

**The failure worth knowing about is 2026-08-29**, because it changed the
harness. A run died at 01:31 with no `STOP` — its first health gate took seven
`rust-lld` SIGBUS crashes, so no test ran and no wall fired, and every one of
them surfaced as `GATE FAIL: native client suites`. The loop could not tell a
build that failed to COMPLETE from a wall that FIRED, so it would have opened a
recovery pass against a green tree; the steward that woke measured silence
against a *previous* run's log and reported 22790s about a four-minute-old
runner; and the sweep meant to file pre-2026-08 reports had been eating the
current ones, so `loop-status.sh` read `0 PASS / 0 FAIL` over 70 real verdicts.
All three are fixed and gated (`watchdog-test.sh`, 23 checks, six mutants);
**why the linker took a SIGBUS is still unknown** and a health run now traces
disk, RAM and swap so the next one is diagnosable —
`gates-loop/findings/note-20260829-the-loop-died-of-the-box-not-the-tree.md`.

The loop wrote most of the commits in this tree. It lives at
`/mnt/hive-data/gates-loop` — **outside this repo, deliberately.** The builder is
told not to touch it and the rubrics are checksummed between passes; if the
harness lived in here, an agent would have write access to the criteria it is
scored against, and a checksum would be the only thing in the way.

| you want | do |
|---|---|
| start it | `tmux new -s gatesloop '/mnt/hive-data/gates-loop/gates-loop.sh'` |
| stop it | `touch /mnt/hive-data/gates-loop/STOP` — finishes the pass, then exits |
| stop it sooner | `touch /mnt/hive-data/gates-loop/YIELD` — the builder lands a coherent partial slice at its next boundary, it is judged as usual, then the runner exits |
| what it is doing | `/mnt/hive-data/gates-loop/loop-status.sh` |
| the frames it captured | `/mnt/hive-data/gates-loop/gallery.py`, then `ssh -L 8899:localhost:8899` |
| why a pass failed | `/mnt/hive-data/gates-loop/findings/pass-<id>-{judge,visual}.md` |
| undo a whole run | `git reset --hard gates-anchor-<stamp>` |
| the frames the visual judge scores against | **outside this repo since 2026-08-11** — `GATES_REFERENCE_DIR`, default `/home/master/gates-reference/rust-images`, held read-only. The runner passes it to all three agents; unset, the judge is told to SKIP rather than score against nothing (`ART.md` §0) |
| `ci/gates.sh` is red on a clean tree | `GATES_FIX_RED=1 /mnt/hive-data/gates-loop/gates-loop.sh` — one pass, wall only |

Two judges score every pass and neither is the builder — and since harness v2
(operator, 2026-08-02) neither is even *spawned* by the builder: the builder
ends its pass on its branch, gates green and unmerged; the runner spawns the
judge holding `judge/RUBRIC.md` (ten procedural checks — the merge gate) and
performs the merge itself on a PASS, then captures and spawns the visual judge
holding `art/RUBRIC.md` (ten visual criteria against the reference set). Both
reports end in a `## Ranked gaps` section, and those gaps — not `NOW.md` — are
where the loop's direction comes from **while it is running**; when it is dark
that instruction is suspended rather than deleted, and the reports are evidence
rather than a queue. `ls -t findings/` is the newest pair — do not trust a date
written here, which is why one is not. **The visual half is the older half**:
capture has been OFF since 2026-08-28, so the newest visual report is weeks
behind the newest judge report, and its frames were shot on seed 20260731 under
`art/capture-native.sh` — the same island the shard ships, which is the only
reason those gaps are about our world at all. Steer from `NOW.md` while it is
dark.

**`git push` is the DEFAULT now** (operator, 2026-08-28: *"pushing needs to be
the default change it"*). It was blocked outright by a `pre-push` hook the
runner installs, with `--no-verify` the only road past it, and that friction
was retired for a measured reason rather than a preference: **it cost a
seven-day divergence.** Two commits sat here from 2026-08-21 while thirty-one
landed on the remote, and because the box also stopped fetching, both sides
independently did the same elopros.com host migration — eight conflicts that
existed only because the work never met. Not-pushing was the default, so work
rotted by omission instead of by decision.

The hook still refuses **one** thing: a force / non-fast-forward push to
`main`, which is the only push that can destroy published history.
`--no-verify` remains the deliberate override for a real rewrite. Everything
else — a branch, a fast-forward of `main` — just goes.

⚠ **So the friction that used to stop you is gone, and the judgement is now
entirely yours.** Fetch before you start (`git fetch && git status` would have
said *"ahead 2, behind 31"* the moment anyone looked), and push a slice when
it lands rather than leaving it to age.

**An agent may push when the operator has plainly asked for it** (operator,
2026-08-17: *"when its clear that i wanna push you can push please"*). This
supersedes the older rule that publishing was operator-only in every case.
The bar is an **explicit instruction in the conversation** — "push it", "get
it up", "ship it" — about a state the operator can see. It is not an
inference from approval of the work: "looks good, merge it" is a merge and
nothing more, and a green gate is never a licence to publish.

Three things do not move:

- **The loop still never pushes.** It proposes; it has no conversation to be
  instructed in. Every autonomous pass ends unpushed.
- **Read the diff first, and say what is in it** — including commits that are
  not yours. A fast-forward publishes everything between the remote and the
  tip, and the operator is agreeing to the sentence you put in front of them,
  not to `git log`.
- **The other operator-only acts are untouched** (§loop discipline): the tag,
  the public-shard deploy, the depot publish, anything on-chain. Pushing a
  branch is not any of those.

**A gate that waits on a clock is not a gate on this box.** Eight cores here —
the rule was learned on the morr box's four at load 4–5, running a cargo
release build and three Chromium tabs against its own shard, and headroom does
not repeal it. On 2026-08-01 three runs of identical code failed on two
different assertions, and the recovery pass found the cause: the third tab was
racing two live renderers. Assert on observable state (`inWorld`, `snapshots >
n`) and never on elapsed milliseconds — the failure that started it reported
`inWorld=true` and timed out anyway. Widening a timeout is not a fix; it is the
same bug with a longer fuse.

**A container that has never run the native client is missing seven things,
and every one of them looks like a defect until you read the message.**
Found 2026-08-06 on a fresh clone, each one costing a rebuild. To BUILD
`--features render`: `libwayland-dev` (bevy's defaults include `wayland` as
well as `x11`), `libasound2-dev` (`bevy_audio` → cpal → `alsa-sys`),
`libudev-dev` (`bevy_gilrs`) — each dies as a `pkg-config` panic 40 lines into
a build script. To RUN a `--capture` probe: `libxkbcommon-x11-0` (winit
panics in `EventLoop::new`), `mesa-vulkan-drivers` + `libvulkan1`, and
`Xvfb`. Plus `rustup target add wasm32-unknown-unknown` for the parity gate
— **which is not a web build**: `sim-core` compiles to a second target so
`test_parity_wasm` can diff its state hashes against native byte for byte,
which is wall 1's enforcement and is worth the same with no browser in
existence. The crate that WAS a web build is gone (operator, 2026-08-08:
*"we use desktop build no more web"*) — `client-wasm` is `client-core`, its
1,635-line C-ABI bridge and the 1,266-line `ci/client_smoke.mjs` that drove
it are deleted, and what that gate actually asserted is
`crates/client-core/tests/wire.rs`.
All of them are the same class — a wall that cannot run is not a wall, so
install them rather than trimming the feature.

Four things that cost time and are not obvious:

- `apt-get install` may 404 on a stale index; `apt-get update` first.
- **`cargo … | tail` reports `tail`'s exit code, not cargo's.** A backgrounded
  gate piped to `tail` reports success while the build is red — `${PIPESTATUS[0]}`.
- The lavapipe ICD is `/usr/share/vulkan/icd.d/**`lvp_icd.json`**, not
  `lvp_icd.x86_64.json`. Point `VK_DRIVER_FILES` at a path that does not exist
  and the loader says only *"Unable to find a GPU"* — the filename is in
  `VK_LOADER_DEBUG=error,warn` and nowhere else. Working invocation:
  `VK_DRIVER_FILES=/usr/share/vulkan/icd.d/lvp_icd.json DISPLAY=:N
  WGPU_BACKEND=vulkan target/release/gates --server … --capture <dir>`.
- **With no sound card, `bevy_audio` logs "No audio device found" and every
  voice is a silent no-op.** That is the correct behaviour and the capture
  probe confirms it: the client must not require an audio device to render.

**Not every red gate is a defect — but check whether the missing capability is
one WE asked for.** `bot_smoke` used to fail all four tests on a container
without IPv6 with `Address family not supported by protocol (os error 97)`, on
a clean tree, and this entry said to believe the box rather than the diff. The
first half was right and the conclusion was one step short: the bind was
**our** code asking for a dual-stack `[::]:0` socket to reach an IPv4 shard.
`bot_endpoint` and `client_endpoint` now bind `InAddrAnyV4` first and keep the
dual-stack bind as the fallback for a v6 shard, reporting both failures if
neither binds — and the gate runs. **Keep the diagnostic habit** —
`git stash -u` before believing a diff caused a red gate — and add the second
question: *is the capability genuinely absent, or are we requesting one we do
not need?* A wall that cannot run is not a wall, and that cuts both ways.
Still environmental on this box, and not the same thing: the `*_wire` server
suites overflow a test thread's 2 MiB stack in a debug build — `RUST_MIN_STACK`
(e.g. `16777216`) runs them green and no tree change is owed. Same class as: `wasm32-unknown-unknown` is not always installed, and the wasm gates
fail with `can't find crate for core` until `rustup target add
wasm32-unknown-unknown` — install it rather than skipping the gate, because a
wall that cannot run is not a wall. (The third of this kind was Playwright's
Chromium not matching the pinned build, fixed with a `VANTAGE_CHROME`
override — **gone with the browser gates**; no gate in this repo starts a
browser now, and none should.) Third, and the one a fresh box
hits first: **`cargo … --features render` needs three `-dev` packages** that a
headless image has no reason to carry — `libwayland-dev`, `libasound2-dev`,
`libudev-dev` — and each fails identically, as a `pkg-config exited with status
code 1` panic from a `*-sys` build script (`wayland-sys`, `alsa-sys`,
`libudev-sys`) with the crate named only in the backtrace. Install them; it is
the box, not the tree. `ci/gates.sh`'s native-client gate now names **all
three** in its own echo line — it listed only the first two when
`libudev-dev` was measured here, and following that line was itself a failure;
it was fixed, so the echo is trustworthy again. Re-confirmed 2026-08-09 on a
fresh container: the gate died at `wayland-sys` and installing the three the
line lists ran it green. Running the client
adds a fourth, and it is a runtime `.so` rather than a build dep, so
`pkg-config` never mentions it: `libxkbcommon-x11-0`, whose absence panics
inside `winit` at `App::run` with every gate already green. **Then ask the
second question for each** — and the answer moved under this entry once
already, which is its own lesson: this paragraph used to say `alsa` was
requested and unused, and since audio v0 (2026-08-06) **`bevy_audio` is
load-bearing** — `render/audio.rs` plays the generated bank through it — so
`alsa` earns its keep. `wayland` (and `x11`) are real — a shipped desktop
client faces both. `libudev` is `bevy_gilrs`, which grep still shows
**unused** (no gamepad code) — that and `vorbis` are the honest trim
targets, while `bevy_gltf`/`bevy_animation` joined the load-bearing set
with the mannequin (2026-08-07). `NOW.md` §0x's trim item tracks what a
grep actually shows rather than what this paragraph last said — re-verify
there before trimming anything.
Neither of these repeals the rule above:
they are missing capabilities, which are diagnosable and permanent, not timing,
which is neither.

## Vendored, and not to be edited here

- `crates/client/src/elo_overlay.rs` is **elo's SDK, byte-for-byte**
  (`sdk/rust/elo_overlay.rs` in **`AnthonE/scry-forge`** — this line said
  `AnthonE/scry` until 2026-08-14, which is a different repo). It is how this game reaches
  a running elo launcher for identity and signatures with no key in the game
  process and no crate added to the tree. `elo::VENDORED_SHA256` pins it and
  a test fails on any local edit — **fix it upstream and re-vendor**, because
  a patch applied here fixes Gates and leaves every other game on the broken
  copy. `crates/client/src/elo.rs` is our wrapper and is ours to change.
  Not third-party: same author, same licence, no notice owed.
  ⚠ **The pin catches a local edit and is blind to upstream moving**, and the
  two are not the same failure. Found 2026-08-09: the copy sat 326 lines
  behind the source with every gate in both repos green — no Windows
  transport (it `use`d `std::os::unix::net` unconditionally, so a Windows
  build of this client could not compile), no `prove`, no `profile`. Nothing
  gates a file in another repo, so the check is a command you run when you
  touch this seam: `sha256sum crates/client/src/elo_overlay.rs` must appear
  in `sdk/SHA256SUMS` upstream. Re-vendoring is `cp` + re-pin + `cargo test
  -p client --lib elo`, and check the CALL SITES, not just the compile —
  `Overlay::title` changed shape under us and only luck kept it uncalled.
  ⚠ **Run that check against `scry-forge` and nothing else — there are two
  repos with an `sdk/` and the other one lies.** `AnthonE/scryward` is the
  public mirror of the open half (launcher, sdk, contracts, docs) and it
  **lags**: on 2026-08-14 it still published `3a81c70…` while the source had
  moved to `3df3d41a…`, so the pin checked against the mirror matched a copy
  two days stale — a **false green**, worse than the drift it was run to find.
  That re-vendor also proved the call-site rule twice over: `play_message`
  changed the bytes a wallet signs (`vow:` → lowercased `wallet:`, upstream
  2026-08-12), which two sides can disagree about while both compile. It is
  re-exported by `elo.rs` and called nowhere, so again nothing broke, and
  again that was luck. **On 2026-08-29 the luck ran out**, and it ran out
  on the one thing a stale copy cannot survive: upstream's platform rename
  (`elo-broker` 575a273b, 2026-08-21 — the same day as ours) moved **the door**,
  all four spellings of it — `SCRY_LAUNCHER_SOCKET` → `ELO_LAUNCHER_SOCKET`,
  `$XDG_RUNTIME_DIR/scry/launcher.sock` → `…/elo/…`, `~/.cache/scry/launcher/`
  → `~/.cache/elo/…`, `\\.\pipe\scry-launcher-<user>` → `elo-launcher-<user>`.
  A game on the old copy then finds no launcher on a machine running one, which
  is the failure the vendored file's own `default_socket` doc calls *"the worst
  shape of bug here"* — the game says "playing anonymously", nothing is red, and
  it lands two hops away as a **login** failure: no launcher → `sign_siwe` is
  `None` → guest → a `require_auth` shard answers `REFUSE_AUTH`, which is what a
  bad signature looks like. **So the drift class to fear is not an API that
  stops compiling — it is a CONSTANT that still compiles and no longer points at
  anything.** Neither the sha pin, nor `cargo test`, nor `ci/gates.sh` can see
  it, and the eight days it survived were paid for in one command nobody ran. The trees are on morr: `/data/apps/scry-forge`
  (`launcher-rs/`, `sdk/`) is the one that is edited and built from.
- The depot the launcher installs is written by `ci/depot.py`, gated by
  `--self-test` in `ci/gates.sh`. It deliberately does **not** compute the
  depot digest — `elo digest` does, and a second implementation of the number
  that gets notarized is elo's invariant 3 with money attached.

## Third-party credit

- `.claude/skills/threejs-*` — the Three.js graphics skill pack, MIT,
  © 2026 Scott Sun (`THREEJS_GRAPHICS_SKILLS_LICENSE`). `threejs-shadow-systems`
  is the source of the client's light-space texel snapping and texel-scaled
  normal bias (`DECISIONS.md` §open, lighting v0). Guidance only — no code
  from the pack ships in this repo. **That is a licence statement, not a
  usage limit: read them.** `threejs-skill-router` routes a graphics task to
  the right ones. **They are now guidance about *technique*, not about this
  codebase**: the renderer they address is three.js and the browser client is
  cut, so a skill's API is no longer ours. What survives the change is what
  was always the valuable half — the shadow pack's texel snapping, the
  exposure pack's log-average metering, the atmosphere pack's LUT structure —
  and every one of those has to be re-expressed against Bevy, exactly as
  `SeedThree`'s TSL wind had to be re-expressed as GLSL. Renderer work starts
  at `RENDER.md`; reach for these for the physics, never for the calls.
- `reference/rust-systems.txt` is ripped from `OxideMod/Oxide.Rust`'s
  `resources/Rust.opj` (MIT, © 2013–2020 Oxide Team and Contributors) —
  facts only: hook names, patched class names, method signatures,
  categories. `CarbonCommunity/Carbon.Hooks.*` (GPL-3.0) is cited as a
  coverage cross-check and deliberately never extracted from, so nothing
  committed here derives from GPL work. `reference/README.md` has the
  provenance and the regeneration command; nothing from either ships.
- `bevy_procedural_tree` (github.com/Affinator/bevy_procedural_tree, MIT OR
  Apache-2.0) — the native client's conifer generator, a **dependency** whose
  code ships. It is `@dgreenheck/ez-tree`'s algorithm (MIT, © Daniel Greenheck)
  ported to Rust and Bevy. The browser client ran the same generator before it
  was deleted, so `props.js`'s swept parameters — in git history — are still
  evidence about this one.
  Only `meshgen::generate_tree_meshes` is called — settings and an `Rng` in,
  two meshes out; the crate's plugin is deliberately unused, because a plugin
  that regenerates entities on change would put tree state in the ECS.
- **game-icons.net** (CC BY 3.0) — the client's item and building icons,
  **shipped**: **64 of the 65** white-on-transparent PNGs in `assets/icons/`,
  rasterised from the project's public SVG archive by `ci/bake_icons.py`.
  (This line read "62" until 2026-08-11 and the tree said 65 — a count in a
  doc drifts every time an icon lands, which is why `CREDITS.md` and
  `STEMS` are the things gated and this is only a pointer.) The 65th,
  `burnt_meat.png`, is **ours** and the CC BY notice does not cover it;
  `CREDITS.md` keeps the two in separate tables so the line is written down
  rather than inferred. CC BY is a *notice* licence — `assets/icons/
  CREDITS.md` names the authors whose work ships — **three since
  2026-08-18**, when `rock` moved off `john-redman/rock` and he stopped
  shipping here; the file is the list, not this line — and `tests/ui.rs` §G
  fails if it stops travelling
  or if the baked set and `ui::icons::STEMS` drift apart. **Nothing is traced from the reference game**: an icon set
  anyone may redistribute with credit is not the IP rail's business, and the
  rail is what forbids copying Rust's own art.
- **Roboto Condensed** (Apache-2.0, © 2011 Google Inc.) — the client's UI
  face, **shipped**: `crates/client/fonts/RobotoCondensed-{Regular,Bold}.ttf`
  are compiled into the binary by `include_bytes!` and
  `fonts/LICENSE-ROBOTO.txt` is the notice that has to travel with them
  (`tests/ui.rs` §F fails if it stops travelling). Not a taste call — the
  reference game names `RobotoCondensed-Bold.ttf` as its own UI default in
  public source (`Facepunch/Rust.Community`, `CommunityEntity.UI.cs`), which
  is a *fact about the target*, in the sense `ART.md` means by measured.
  Nothing was traced and no proper noun ships; a typeface anyone may
  redistribute is not the IP rail's business.
- `SeedThree` (github.com/SkyeShark/SeedThree, MIT) — source of the wind
  design the client ships: one per-vertex `aWind` cantilever weight rooted at
  the trunk base, phase taken from the instance's world position so a gust
  crosses the forest instead of each tree twitching alone, and two sine
  octaves rather than one. Its `wind.js` is TSL/WebGPU node-material source; the
  deleted browser client re-expressed the design as a GLSL injection, and the
  native client has to re-express it again against Bevy — guidance only, no
  code from it ships. Its `impostor.js` (two
  crossed alpha cards baked from front/side ortho cameras in a worker, with the
  backend readback row order probed once against a known image) is the
  reference for `TERRAIN.md` §4's unbuilt billboard LOD; its emit side returns
  a `Group` per tree and would have to become an `InstancedMesh` pool here.
- `Claude-of-Duty` (github.com/mshumer/Claude-of-Duty, MIT, © 2026 mshumer)
  — source of the shader-prewarm trap, the bit-identical-capture discipline
  (fresh page per shot; engine clock, not `performance.now()`), and the
  coupled-lighting single-owner datum, all from its postmortem. Guidance
  only — no code from it ships in this repo.
- `SkyeShark/Eanpa-Sky` and `SkyeShark/SeedThree` (both MIT, © 2026 SkyeShark)
  — read while surveying the node stack. (They were catalogued in
  `MIGRATION.md` §8.3, deleted with the browser client; the licence facts that
  doc carried are restated here in full, because a deleted file cannot hold an
  obligation.) Nothing from either ships **yet** — and unlike the skill packs
  above, MIT means it may. Two routes are open and both require carrying the
  notice **at the donor site**, not a bullet here alone: SeedThree as vendored
  `.glb` output, and Eanpa-Sky's cloud noise, presets and density function
  copied into the renderer. Credit again, in the file, at that point.
  **Neither repo's audio may be touched**: Eanpa-Sky names four xeno-canto
  recordings as CC BY-NC-SA, and SeedThree ships bird recordings whose
  `README.txt` cites xeno-canto but **states no licence per file** —
  unresolved is not the same as permissive, and NC does not survive a sold
  product either way. This matters more now, not less: `sound/synth.rs`
  generates our bank at boot precisely to avoid this class of question.
